//! Asynchronous IPC client with nonce correlation and zero-copy packet transport.
//!
//! [`DaemonClient`] manages a duplex connection to the compositor daemon, multiplexing
//! concurrent requests over a single stream using monotonic 64-bit nonces and oneshot channels.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_util::codec::{FramedRead, FramedWrite};

use super::codec::BinaryIpcCodec;
use super::error::{IpcError, IpcStatusCode};
use super::protocol::{BinaryPacket, DaemonCommand, DaemonResponse, STATUS_OK};

type ResponseSender = oneshot::Sender<Result<BinaryPacket, IpcError>>;
type PendingMap = Arc<Mutex<HashMap<u64, ResponseSender>>>;

/// Client handle for interacting with the persistent compositor daemon over binary IPC.
#[derive(Clone)]
pub struct DaemonClient {
    next_nonce: Arc<AtomicU64>,
    outgoing_tx: mpsc::Sender<BinaryPacket>,
    pending: PendingMap,
    default_timeout: Duration,
}

impl DaemonClient {
    /// Create a new `DaemonClient` from separate async reader and writer halves.
    pub fn new<R, W>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        Self::with_timeout(reader, writer, Duration::from_secs(30))
    }

    /// Create a new `DaemonClient` with a custom default timeout duration.
    pub fn with_timeout<R, W>(reader: R, writer: W, timeout: Duration) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<BinaryPacket>(128);
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));

        // Writer task
        let mut framed_writer = FramedWrite::new(writer, BinaryIpcCodec::default());
        tokio::spawn(async move {
            while let Some(packet) = outgoing_rx.recv().await {
                if let Err(err) = framed_writer.send(packet).await {
                    tracing::debug!("IPC client writer stream ended: {err}");
                    break;
                }
            }
        });

        // Reader task
        let mut framed_reader = FramedRead::new(reader, BinaryIpcCodec::default());
        let pending_reader = Arc::clone(&pending);
        tokio::spawn(async move {
            while let Some(result) = framed_reader.next().await {
                match result {
                    Ok(packet) => {
                        let mut map = pending_reader.lock().await;
                        if let Some(sender) = map.remove(&packet.nonce) {
                            let _ = sender.send(Ok(packet));
                        } else {
                            tracing::debug!("Received unmapped or expired packet nonce: {}", packet.nonce);
                        }
                    }
                    Err(err) => {
                        tracing::debug!("IPC client reader encountered error: {err}");
                        let mut map = pending_reader.lock().await;
                        for (_, sender) in map.drain() {
                            let _ = sender.send(Err(IpcError::ConnectionClosed));
                        }
                        break;
                    }
                }
            }

            // If reader stream terminates (EOF)
            let mut map = pending_reader.lock().await;
            for (_, sender) in map.drain() {
                let _ = sender.send(Err(IpcError::ConnectionClosed));
            }
        });

        Self {
            next_nonce: Arc::new(AtomicU64::new(1)),
            outgoing_tx,
            pending,
            default_timeout: timeout,
        }
    }

    /// Create a new `DaemonClient` from a combined bidirectional stream (e.g. `TcpStream`, `DuplexStream`).
    pub fn from_stream<T>(stream: T) -> Self
    where
        T: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let (reader, writer) = tokio::io::split(stream);
        Self::new(reader, writer)
    }

    /// Allocate the next monotonic 64-bit request nonce.
    pub fn next_nonce(&self) -> u64 {
        self.next_nonce.fetch_add(1, Ordering::Relaxed)
    }

    /// Send a low-level [`BinaryPacket`] and await the response with the same nonce.
    pub async fn send_packet(&self, packet: BinaryPacket) -> Result<BinaryPacket, IpcError> {
        self.send_packet_timeout(packet, self.default_timeout).await
    }

    /// Send a [`BinaryPacket`] and await response with a custom timeout.
    pub async fn send_packet_timeout(
        &self,
        packet: BinaryPacket,
        timeout: Duration,
    ) -> Result<BinaryPacket, IpcError> {
        let nonce = packet.nonce;
        let (tx, rx) = oneshot::channel();

        {
            let mut map = self.pending.lock().await;
            map.insert(nonce, tx);
        }

        if self.outgoing_tx.send(packet).await.is_err() {
            let mut map = self.pending.lock().await;
            map.remove(&nonce);
            return Err(IpcError::ConnectionClosed);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(packet_res)) => packet_res,
            Ok(Err(_oneshot_canceled)) => Err(IpcError::ConnectionClosed),
            Err(_elapsed) => {
                let mut map = self.pending.lock().await;
                map.remove(&nonce);
                Err(IpcError::RequestTimeout(nonce))
            }
        }
    }

    /// Send a high-level [`DaemonCommand`] and deserialize the [`DaemonResponse`].
    pub async fn send_command(&self, command: &DaemonCommand) -> Result<DaemonResponse, IpcError> {
        let nonce = self.next_nonce();
        let payload = serde_json::to_vec(command)?;
        let packet = BinaryPacket::new(nonce, STATUS_OK, Bytes::from(payload));

        let response = self.send_packet(packet).await?;
        if response.status != STATUS_OK {
            let message = match response.payload_str() {
                Ok(s) => s.to_string(),
                Err(_) => String::from_utf8_lossy(&response.payload).to_string(),
            };
            return Err(IpcError::ResponseError {
                status: response.status,
                message,
            });
        }

        let daemon_resp: DaemonResponse = serde_json::from_slice(&response.payload)?;
        Ok(daemon_resp)
    }

    /// Send a heartbeat ping to the daemon.
    pub async fn ping(&self) -> Result<(), IpcError> {
        let resp = self.send_command(&DaemonCommand::Ping).await?;
        match resp {
            DaemonResponse::Pong => Ok(()),
            other => Err(IpcError::Protocol(format!("Expected Pong, got {other:?}"))),
        }
    }

    /// Query the list of available composition IDs from the daemon.
    pub async fn list_compositions(&self) -> Result<Vec<String>, IpcError> {
        let resp = self.send_command(&DaemonCommand::ListCompositions).await?;
        match resp {
            DaemonResponse::CompositionsList { ids } => Ok(ids),
            other => Err(IpcError::Protocol(format!("Expected CompositionsList, got {other:?}"))),
        }
    }

    /// Query composition metadata from the daemon.
    pub async fn get_metadata(&self, composition_id: impl Into<String>) -> Result<DaemonResponse, IpcError> {
        self.send_command(&DaemonCommand::GetMetadata {
            composition_id: composition_id.into(),
        })
        .await
    }

    /// Query LRU frame cache metrics from the daemon.
    pub async fn get_cache_stats(&self) -> Result<DaemonResponse, IpcError> {
        self.send_command(&DaemonCommand::GetCacheStats).await
    }

    /// Clear all cached frames in the daemon LRU frame cache.
    pub async fn clear_cache(&self) -> Result<(), IpcError> {
        let resp = self.send_command(&DaemonCommand::ClearCache).await?;
        match resp {
            DaemonResponse::Success => Ok(()),
            other => Err(IpcError::Protocol(format!("Expected Success, got {other:?}"))),
        }
    }

    /// Gracefully shutdown the daemon process.
    pub async fn shutdown(&self) -> Result<(), IpcError> {
        let _ = self.send_command(&DaemonCommand::Shutdown).await;
        Ok(())
    }

    /// Request rendering of a single frame and return the raw uncompressed RGBA pixel bytes.
    pub async fn render_frame(
        &self,
        composition_id: impl Into<String>,
        frame: u32,
        width: u32,
        height: u32,
        fps: f64,
        props: serde_json::Value,
    ) -> Result<Bytes, IpcError> {
        let nonce = self.next_nonce();
        let cmd = DaemonCommand::RenderFrame {
            composition_id: composition_id.into(),
            frame,
            width,
            height,
            fps,
            props,
        };
        let payload = serde_json::to_vec(&cmd)?;
        let packet = BinaryPacket::new(nonce, STATUS_OK, Bytes::from(payload));

        let response = self.send_packet(packet).await?;
        if response.status != STATUS_OK {
            let message = String::from_utf8_lossy(&response.payload).to_string();
            return Err(IpcError::ResponseError {
                status: response.status,
                message,
            });
        }

        let expected_bytes = (width as usize) * (height as usize) * 4;
        if response.payload.len() != expected_bytes {
            return Err(IpcError::Protocol(format!(
                "Invalid RGBA buffer length: expected {expected_bytes} bytes for {width}x{height}, received {} bytes",
                response.payload.len()
            )));
        }

        Ok(response.payload)
    }

    /// Request rendering of a thumbnail frame and return the raw uncompressed RGBA pixel bytes.
    pub async fn render_thumbnail(
        &self,
        composition_id: impl Into<String>,
        frame: u32,
        width: u32,
        height: u32,
        fps: f64,
        props: serde_json::Value,
    ) -> Result<Bytes, IpcError> {
        let nonce = self.next_nonce();
        let cmd = DaemonCommand::RenderThumbnail {
            composition_id: composition_id.into(),
            frame,
            width,
            height,
            fps,
            props,
        };
        let payload = serde_json::to_vec(&cmd)?;
        let packet = BinaryPacket::new(nonce, STATUS_OK, Bytes::from(payload));

        let response = self.send_packet(packet).await?;
        if response.status != STATUS_OK {
            let message = String::from_utf8_lossy(&response.payload).to_string();
            return Err(IpcError::ResponseError {
                status: response.status,
                message,
            });
        }

        let expected_bytes = (width as usize) * (height as usize) * 4;
        if response.payload.len() != expected_bytes {
            return Err(IpcError::Protocol(format!(
                "Invalid RGBA thumbnail length: expected {expected_bytes} bytes for {width}x{height}, received {} bytes",
                response.payload.len()
            )));
        }

        Ok(response.payload)
    }
}

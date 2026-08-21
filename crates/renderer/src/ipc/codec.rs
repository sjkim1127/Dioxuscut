//! Zero-copy streaming encoder and decoder for the Remotion binary IPC protocol.
//!
//! Provides [`BinaryIpcCodec`], [`StreamDecoder`], [`StreamEncoder`], and [`make_streamer`]
//! with chunked streaming, partial buffer accumulation, noisy resynchronization, and zero-copy `Bytes` slicing.

use std::io::Write;

use bytes::{Buf, Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Decoder, Encoder, Framed};

use super::error::IpcError;
use super::protocol::{BinaryPacket, BUFFER_PREFIX, DEFAULT_MAX_PAYLOAD_BYTES};

/// Maximum header length in bytes before rejecting as corrupted and seeking next prefix.
const MAX_HEADER_SEARCH_BYTES: usize = 128;

/// Finds the starting index of `needle` inside `haystack`.
#[inline]
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Tokio-util codec implementing asynchronous encoding and decoding for [`BinaryPacket`]
/// using the Remotion wire framing protocol `remotion_buffer:<nonce>:<len>:<status>:<payload>`.
#[derive(Debug, Clone)]
pub struct BinaryIpcCodec {
    /// Maximum allowed payload size in bytes.
    pub max_payload_bytes: usize,
}

impl Default for BinaryIpcCodec {
    fn default() -> Self {
        Self {
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
        }
    }
}

impl BinaryIpcCodec {
    /// Create a new `BinaryIpcCodec` with a specified maximum payload byte size.
    pub fn new(max_payload_bytes: usize) -> Self {
        Self { max_payload_bytes }
    }

    /// Set a custom maximum payload limit.
    pub fn with_max_payload_bytes(mut self, max: usize) -> Self {
        self.max_payload_bytes = max;
        self
    }
}

impl Decoder for BinaryIpcCodec {
    type Item = BinaryPacket;
    type Error = IpcError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        loop {
            if src.is_empty() {
                return Ok(None);
            }

            // 1. Locate magic prefix
            let prefix_pos = match find_subsequence(src, BUFFER_PREFIX) {
                Some(pos) => pos,
                None => {
                    // Retain the last (BUFFER_PREFIX.len() - 1) bytes in case the prefix is partially received
                    let keep = BUFFER_PREFIX.len().saturating_sub(1);
                    if src.len() > keep {
                        src.advance(src.len() - keep);
                    }
                    return Ok(None);
                }
            };

            // Discard any noise/log bytes preceding the magic prefix
            if prefix_pos > 0 {
                src.advance(prefix_pos);
            }

            // 2. Scan for 3 colons delimiting <nonce>:<len>:<status>:
            let after_prefix = &src[BUFFER_PREFIX.len()..];
            let mut colon_offsets = Vec::with_capacity(3);
            for (idx, &byte) in after_prefix.iter().enumerate() {
                if byte == b':' {
                    colon_offsets.push(BUFFER_PREFIX.len() + idx);
                    if colon_offsets.len() == 3 {
                        break;
                    }
                }
            }

            if colon_offsets.len() < 3 {
                // Header incomplete. Check if buffer has grown excessively large without header completion.
                if src.len() > BUFFER_PREFIX.len() + MAX_HEADER_SEARCH_BYTES {
                    // Header appears corrupted; discard prefix and seek next
                    src.advance(BUFFER_PREFIX.len());
                    continue;
                }
                return Ok(None);
            }

            let c1 = colon_offsets[0]; // After nonce
            let c2 = colon_offsets[1]; // After len
            let c3 = colon_offsets[2]; // After status (end of header)

            let nonce_str = match std::str::from_utf8(&src[BUFFER_PREFIX.len()..c1]) {
                Ok(s) => s,
                Err(_) => {
                    src.advance(BUFFER_PREFIX.len());
                    continue;
                }
            };

            let len_str = match std::str::from_utf8(&src[c1 + 1..c2]) {
                Ok(s) => s,
                Err(_) => {
                    src.advance(BUFFER_PREFIX.len());
                    continue;
                }
            };

            let status_str = match std::str::from_utf8(&src[c2 + 1..c3]) {
                Ok(s) => s,
                Err(_) => {
                    src.advance(BUFFER_PREFIX.len());
                    continue;
                }
            };

            let nonce = match nonce_str.parse::<u64>() {
                Ok(n) => n,
                Err(_) => {
                    src.advance(BUFFER_PREFIX.len());
                    continue;
                }
            };

            let len = match len_str.parse::<usize>() {
                Ok(l) => l,
                Err(_) => {
                    src.advance(BUFFER_PREFIX.len());
                    continue;
                }
            };

            let status = match status_str.parse::<u32>() {
                Ok(s) => s,
                Err(_) => {
                    src.advance(BUFFER_PREFIX.len());
                    continue;
                }
            };

            // Validate maximum payload size limit
            if len > self.max_payload_bytes {
                return Err(IpcError::PayloadTooLarge {
                    len,
                    max: self.max_payload_bytes,
                });
            }

            let header_len = c3 + 1;
            let total_len = header_len + len;

            if src.len() < total_len {
                // Header is complete, but payload is incomplete. Reserve required space and wait.
                src.reserve(total_len - src.len());
                return Ok(None);
            }

            // 3. Extract header and slice zero-copy Bytes payload
            src.advance(header_len);
            let payload = src.split_to(len).freeze();

            return Ok(Some(BinaryPacket {
                nonce,
                status,
                payload,
            }));
        }
    }
}

impl Encoder<BinaryPacket> for BinaryIpcCodec {
    type Error = IpcError;

    fn encode(&mut self, item: BinaryPacket, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let mut header_buf = [0u8; 96];
        let mut cursor = std::io::Cursor::new(&mut header_buf[..]);
        write!(
            cursor,
            "remotion_buffer:{}:{}:{}:",
            item.nonce,
            item.payload.len(),
            item.status
        )
        .map_err(IpcError::Io)?;

        let header_len = cursor.position() as usize;
        let header_bytes = &header_buf[..header_len];

        dst.reserve(header_len + item.payload.len());
        dst.extend_from_slice(header_bytes);
        dst.extend_from_slice(&item.payload);
        Ok(())
    }
}

/// Standalone stream decoder wrapping [`BinaryIpcCodec`].
#[derive(Debug, Clone, Default)]
pub struct StreamDecoder {
    codec: BinaryIpcCodec,
}

impl StreamDecoder {
    /// Create a new `StreamDecoder` with default payload limit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new `StreamDecoder` with a custom maximum payload limit.
    pub fn with_max_payload_bytes(max: usize) -> Self {
        Self {
            codec: BinaryIpcCodec::new(max),
        }
    }

    /// Decode the next [`BinaryPacket`] from the accumulated buffer.
    pub fn decode(&mut self, src: &mut BytesMut) -> Result<Option<BinaryPacket>, IpcError> {
        self.codec.decode(src)
    }

    /// Decode final packet at end-of-file.
    pub fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<BinaryPacket>, IpcError> {
        self.codec.decode_eof(src)
    }
}

/// Standalone stream encoder wrapping [`BinaryIpcCodec`].
#[derive(Debug, Clone, Default)]
pub struct StreamEncoder {
    codec: BinaryIpcCodec,
}

impl StreamEncoder {
    /// Create a new `StreamEncoder` with default payload limit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new `StreamEncoder` with a custom maximum payload limit.
    pub fn with_max_payload_bytes(max: usize) -> Self {
        Self {
            codec: BinaryIpcCodec::new(max),
        }
    }

    /// Encode a [`BinaryPacket`] into the destination buffer.
    pub fn encode(&mut self, packet: BinaryPacket, dst: &mut BytesMut) -> Result<(), IpcError> {
        self.codec.encode(packet, dst)
    }

    /// Encode a [`BinaryPacket`] into an independent [`Bytes`] buffer.
    pub fn encode_to_bytes(&mut self, packet: BinaryPacket) -> Result<Bytes, IpcError> {
        let mut buf = BytesMut::new();
        self.codec.encode(packet, &mut buf)?;
        Ok(buf.freeze())
    }
}

/// Constructs a bidirectional packet stream/sink over any asynchronous I/O channel.
pub fn make_streamer<T>(io: T) -> Framed<T, BinaryIpcCodec>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    Framed::new(io, BinaryIpcCodec::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_subsequence() {
        let data = b"abcremotion_buffer:123";
        assert_eq!(find_subsequence(data, BUFFER_PREFIX), Some(3));
        assert_eq!(find_subsequence(b"short", BUFFER_PREFIX), None);
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let mut encoder = StreamEncoder::new();
        let mut decoder = StreamDecoder::new();

        let packet = BinaryPacket::ok(12345, Bytes::from_static(b"PixelDataRGBA1234"));
        let mut buffer = BytesMut::new();
        encoder.encode(packet.clone(), &mut buffer).unwrap();

        let decoded = decoder.decode(&mut buffer).unwrap().expect("should decode packet");
        assert_eq!(decoded, packet);
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_zero_length_payload() {
        let mut encoder = StreamEncoder::new();
        let mut decoder = StreamDecoder::new();

        let packet = BinaryPacket::ok(99, Bytes::new());
        let mut buffer = BytesMut::new();
        encoder.encode(packet.clone(), &mut buffer).unwrap();

        let decoded = decoder.decode(&mut buffer).unwrap().expect("should decode empty payload");
        assert_eq!(decoded, packet);
        assert_eq!(decoded.payload.len(), 0);
    }

    #[test]
    fn test_chunked_byte_by_byte() {
        let mut encoder = StreamEncoder::new();
        let mut decoder = StreamDecoder::new();

        let packet = BinaryPacket::new(42, 0, Bytes::from_static(b"0123456789abcdef"));
        let mut full_buf = BytesMut::new();
        encoder.encode(packet.clone(), &mut full_buf).unwrap();

        let mut stream_buf = BytesMut::new();
        let mut result = None;

        for byte in full_buf {
            stream_buf.extend_from_slice(&[byte]);
            if let Some(p) = decoder.decode(&mut stream_buf).unwrap() {
                result = Some(p);
                break;
            }
        }

        assert_eq!(result, Some(packet));
        assert!(stream_buf.is_empty());
    }

    #[test]
    fn test_packet_coalescence() {
        let mut encoder = StreamEncoder::new();
        let mut decoder = StreamDecoder::new();

        let p1 = BinaryPacket::ok(1, Bytes::from_static(b"First"));
        let p2 = BinaryPacket::ok(2, Bytes::from_static(b"Second"));
        let p3 = BinaryPacket::err(3, 404, "Not found");

        let mut multi_buf = BytesMut::new();
        encoder.encode(p1.clone(), &mut multi_buf).unwrap();
        encoder.encode(p2.clone(), &mut multi_buf).unwrap();
        encoder.encode(p3.clone(), &mut multi_buf).unwrap();

        let d1 = decoder.decode(&mut multi_buf).unwrap().unwrap();
        let d2 = decoder.decode(&mut multi_buf).unwrap().unwrap();
        let d3 = decoder.decode(&mut multi_buf).unwrap().unwrap();
        let d4 = decoder.decode(&mut multi_buf).unwrap();

        assert_eq!(d1, p1);
        assert_eq!(d2, p2);
        assert_eq!(d3, p3);
        assert_eq!(d4, None);
        assert!(multi_buf.is_empty());
    }

    #[test]
    fn test_noise_resynchronization() {
        let mut encoder = StreamEncoder::new();
        let mut decoder = StreamDecoder::new();

        let packet = BinaryPacket::ok(777, Bytes::from_static(b"ValidPayload"));
        let mut encoded = BytesMut::new();
        encoder.encode(packet.clone(), &mut encoded).unwrap();

        let mut noisy_stream = BytesMut::new();
        noisy_stream.extend_from_slice(b"Some random stdout log message: initializing...\n");
        noisy_stream.extend_from_slice(&[0x00, 0xFF, 0x12, 0x34]);
        noisy_stream.extend_from_slice(&encoded);

        let decoded = decoder.decode(&mut noisy_stream).unwrap().expect("must resync and decode");
        assert_eq!(decoded, packet);
    }

    #[test]
    fn test_payload_too_large() {
        let mut decoder = StreamDecoder::with_max_payload_bytes(100);
        let mut buf = BytesMut::from(&b"remotion_buffer:1:200:0:"[..]);
        let err = decoder.decode(&mut buf).unwrap_err();
        match err {
            IpcError::PayloadTooLarge { len, max } => {
                assert_eq!(len, 200);
                assert_eq!(max, 100);
            }
            other => panic!("Unexpected error: {other:?}"),
        }
    }
}

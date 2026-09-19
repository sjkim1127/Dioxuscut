use super::types::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

pub(crate) const RING_BUFFER_SIZE: usize = 2;

/// A single buffered set of GPU render resources (MSAA texture, resolve target, readback buffer).
pub(crate) struct GpuFrameSlot {
    pub(crate) texture: wgpu::Texture,
    pub(crate) texture_view: wgpu::TextureView,
    pub(crate) _msaa_texture: wgpu::Texture,
    pub(crate) msaa_view: wgpu::TextureView,
    pub(crate) readback: wgpu::Buffer,
    pub(crate) bytes_per_row: u32,
}

impl GpuFrameSlot {
    pub(crate) fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let bytes_per_row = align_to_256(width * 4);

        let msaa_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gpu_msaa_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: SAMPLE_COUNT,
            dimension: wgpu::TextureDimension::D2,
            format: RENDER_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let msaa_view = msaa_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gpu_resolve_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: RENDER_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_readback_buffer"),
            size: (bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            texture,
            texture_view,
            _msaa_texture: msaa_texture,
            msaa_view,
            readback,
            bytes_per_row,
        }
    }
}

/// Double-buffered GPU ring buffer per resolution.
///
/// Keeps two independent `GpuFrameSlot` instances alive so that the GPU can
/// render frame N into slot 1 while the CPU reads back frame N-1 from slot 0.
pub(crate) struct GpuFrameResources {
    pub(crate) slots: [GpuFrameSlot; RING_BUFFER_SIZE],
    pub(crate) active_index: usize,
}

impl GpuFrameResources {
    pub(crate) fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        Self {
            slots: [
                GpuFrameSlot::new(device, width, height),
                GpuFrameSlot::new(device, width, height),
            ],
            active_index: 0,
        }
    }
}

pub(crate) struct InFlight {
    pub(crate) frame_idx: u32,
    pub(crate) slot_idx: usize,
    pub(crate) submission_index: wgpu::SubmissionIndex,
    pub(crate) rx: std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
}

pub(crate) type GpuResourcePool = Mutex<HashMap<(u32, u32), Arc<Mutex<GpuFrameResources>>>>;
pub(crate) type GpuLayerResourcePool = Mutex<HashMap<(u32, u32), Arc<Mutex<GpuFrameSlot>>>>;

pub(crate) struct GpuImageResource {
    pub(crate) _texture: wgpu::Texture,
    pub(crate) _view: wgpu::TextureView,
    pub(crate) _sampler: wgpu::Sampler,
    pub(crate) bind_group: wgpu::BindGroup,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[allow(dead_code)]
pub(crate) struct GpuExternalTexture {
    pub(crate) _sampler: wgpu::Sampler,
    pub(crate) bind_group: wgpu::BindGroup,
}

pub(crate) struct GpuImageCacheState {
    pub(crate) images: HashMap<String, Arc<GpuImageResource>>,
    pub(crate) lru: std::collections::VecDeque<String>,
    pub(crate) bytes: usize,
    pub(crate) max_bytes: usize,
}

pub(crate) struct GpuTextAtlasResource {
    pub(crate) _texture: wgpu::Texture,
    pub(crate) _view: wgpu::TextureView,
    pub(crate) _sampler: wgpu::Sampler,
    pub(crate) bind_group: wgpu::BindGroup,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) generation: AtomicU64,
}

pub(crate) type GpuImageCache = Mutex<GpuImageCacheState>;

impl GpuImageCacheState {
    pub(crate) fn new(max_bytes: usize) -> Self {
        Self {
            images: HashMap::new(),
            lru: std::collections::VecDeque::new(),
            bytes: 0,
            max_bytes: max_bytes.max(1),
        }
    }

    pub(crate) fn trim_to_budget(&mut self) {
        while self.bytes > self.max_bytes {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            if let Some(evicted) = self.images.remove(&oldest) {
                self.bytes = self
                    .bytes
                    .saturating_sub(evicted.width as usize * evicted.height as usize * 4);
            }
        }
    }
}

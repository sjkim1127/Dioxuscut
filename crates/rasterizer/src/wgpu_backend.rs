//! GPU rasterizer backend using `wgpu`.
//!
//! Renders a [`Scene`] into an offscreen texture on the GPU,
//! then reads the pixels back to CPU and returns an `RgbaImage`.
//!
//! Enabled with the `gpu` feature flag:
//! ```toml
//! dioxuscut-rasterizer = { features = ["gpu"] }
//! ```
//!
//! # Architecture
//!
//! ```text
//! Scene nodes
//!   │
//!   ▼
//! VertexBuffer (quads / geometry)
//!   │
//!   ▼
//! wgpu RenderPass ──→ Offscreen RGBA8 Texture (MSAA 4x)
//!   │
//!   ▼
//! Resolve → Output Texture → Buffer Readback
//!   │
//!   ▼
//! RgbaImage
//! ```
//!
//! Rectangles, circles, and gradients use analytic screen-space quads. SVG
//! paths and strokes are tessellated into indexed triangle meshes, while
//! nested groups share the CPU renderer's affine transform composition.
//!
//! # Colour Pipeline
//!
//! All internal rendering uses `Rgba8Unorm` (linear light, **not** sRGB) to
//! avoid the double-gamma problem that would arise if the fragment shader
//! received sRGB-pre-linearised colour values and wrote them into an sRGB
//! framebuffer. Input `Color` values (u8 sRGB) are converted to linear float
//! via [`srgb_to_linear`] before being stored in GPU uniforms.
//!
//! # Resource Pool
//!
//! [`GpuFrameResources`] caches per-resolution textures and the readback
//! buffer inside `WgpuBackend`, eliminating the allocation pressure of
//! re-creating GPU objects on every `render_frame` call.

#![cfg(feature = "gpu")]

use crate::backend::{BackendCapabilities, FrameConfig, RasterError, RasterizerBackend};
use crate::scene::{Color, GradientStop, Scene, SceneNode};
use crate::tiny_skia_backend::{svgpath_to_tiny_skia, TinySkiaBackend};
use image::RgbaImage;
use lyon_tessellation::geometry_builder::{BuffersBuilder, FillVertexConstructor, VertexBuffers};
use lyon_tessellation::math::point;
use lyon_tessellation::path::Path as LyonPath;
use lyon_tessellation::{FillOptions, FillTessellator, FillVertex};
use std::collections::HashMap;
use std::sync::Mutex;
use tiny_skia::{Path as TinyPath, PathSegment, Stroke, Transform};
use wgpu::util::DeviceExt;

const MAX_GRADIENT_STOPS: usize = 16;
const SAMPLE_COUNT: u32 = 4;
/// Internal linear-light render format. **Not** sRGB to avoid double-gamma.
const RENDER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const MESH_ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x2];

// ────────────────────────────────────────────────────────────────────────────
// WGSL Shader Source
// ────────────────────────────────────────────────────────────────────────────

const SHADER_SRC: &str = r#"
struct Globals {
    resolution: vec2<f32>,
};

struct InstanceData {
    // x = shape type, y = gradient stop count
    kind_data: vec4<u32>,
    // Draw and original shape bounds in local pixels.
    bounds: vec4<f32>,
    shape_bounds: vec4<f32>,
    color: vec4<f32>,
    color2: vec4<f32>,
    // corner radius, stroke width, angle, inherited opacity
    params: vec4<f32>,
    // x' = dot(transform_x.xyz, vec3(x, y, 1))
    // y' = dot(transform_y.xyz, vec3(x, y, 1))
    transform_x: vec4<f32>,
    transform_y: vec4<f32>,
    stop_positions: array<vec4<f32>, 16>,
    stop_colors: array<vec4<f32>, 16>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var<storage, read> instances: array<InstanceData>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local_position: vec2<f32>,
    @location(1) @interpolate(flat) instance_index: u32,
};

fn to_clip_position(pixel_position: vec2<f32>) -> vec4<f32> {
    let ndc = vec2<f32>(
         pixel_position.x / globals.resolution.x * 2.0 - 1.0,
        -pixel_position.y / globals.resolution.y * 2.0 + 1.0,
    );
    return vec4<f32>(ndc, 0.0, 1.0);
}

@vertex
fn vs_main(
    @builtin(vertex_index) vid: u32,
    @builtin(instance_index) iid: u32,
) -> VertexOutput {
    let instance = instances[iid];
    let x = instance.bounds.x;
    let y = instance.bounds.y;
    let w = instance.bounds.z;
    let h = instance.bounds.w;

    // Avoid dynamically indexing a local array: some native shader backends
    // only permit constant indexes for this expression class.
    var pixel_pos: vec2<f32>;
    switch vid {
        case 0u: { pixel_pos = vec2<f32>(x,     y + h); }
        case 1u: { pixel_pos = vec2<f32>(x + w, y + h); }
        case 2u: { pixel_pos = vec2<f32>(x,     y    ); }
        case 3u: { pixel_pos = vec2<f32>(x,     y    ); }
        case 4u: { pixel_pos = vec2<f32>(x + w, y + h); }
        default: { pixel_pos = vec2<f32>(x + w, y    ); }
    }
    let value = vec3<f32>(pixel_pos, 1.0);
    let transformed = vec2<f32>(
        dot(instance.transform_x.xyz, value),
        dot(instance.transform_y.xyz, value),
    );
    return VertexOutput(to_clip_position(transformed), pixel_pos, iid);
}

struct MeshVertexInput {
    @location(0) position: vec2<f32>,
};

@vertex
fn vs_mesh(
    vertex: MeshVertexInput,
    @builtin(instance_index) iid: u32,
) -> VertexOutput {
    let instance = instances[iid];
    let value = vec3<f32>(vertex.position, 1.0);
    let transformed = vec2<f32>(
        dot(instance.transform_x.xyz, value),
        dot(instance.transform_y.xyz, value),
    );
    return VertexOutput(to_clip_position(transformed), vertex.position, iid);
}

fn gradient_color(instance_idx: u32, t: f32) -> vec4<f32> {
    let count = max(instances[instance_idx].kind_data.y, 1u);
    if t <= instances[instance_idx].stop_positions[0].x {
        return instances[instance_idx].stop_colors[0];
    }

    var previous_position = instances[instance_idx].stop_positions[0].x;
    var previous_color = instances[instance_idx].stop_colors[0];
    for (var index = 1u; index < 16u; index = index + 1u) {
        if index >= count {
            break;
        }
        let next_position = instances[instance_idx].stop_positions[index].x;
        let next_color = instances[instance_idx].stop_colors[index];
        if t <= next_position {
            let span = max(next_position - previous_position, 0.000001);
            let amount = clamp((t - previous_position) / span, 0.0, 1.0);
            return mix(previous_color, next_color, amount);
        }
        previous_position = next_position;
        previous_color = next_color;
    }
    return previous_color;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let instance = instances[in.instance_index];
    let shape_type = instance.kind_data.x;
    let shape = instance.shape_bounds;
    var col = instance.color;
    var coverage = 1.0;

    if shape_type == 0u {
        let half = shape.zw * 0.5;
        let center = shape.xy + half;
        let corner_r = clamp(instance.params.x, 0.0, min(half.x, half.y));
        let p = in.local_position - center;
        let q = abs(p) - half + vec2<f32>(corner_r);
        let distance = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - corner_r;
        coverage = 1.0 - smoothstep(-0.75, 0.75, distance);
        let stroke_width = instance.params.y;
        if stroke_width > 0.0 {
            let stroke_coverage = 1.0 - smoothstep(
                stroke_width * 0.5 - 0.75,
                stroke_width * 0.5 + 0.75,
                abs(distance),
            );
            col = mix(col, instance.color2, stroke_coverage);
            coverage = max(coverage, stroke_coverage);
        }

    } else if shape_type == 1u {
        let center = shape.xy + shape.zw * 0.5;
        let radius = min(shape.z, shape.w) * 0.5;
        let distance = length(in.local_position - center) - radius;
        coverage = 1.0 - smoothstep(-0.75, 0.75, distance);
        let stroke_width = instance.params.y;
        if stroke_width > 0.0 {
            let stroke_coverage = 1.0 - smoothstep(
                stroke_width * 0.5 - 0.75,
                stroke_width * 0.5 + 0.75,
                abs(distance),
            );
            col = mix(col, instance.color2, stroke_coverage);
            coverage = max(coverage, stroke_coverage);
        }

    } else if shape_type == 2u {
        let angle_rad = instance.params.z * 3.14159265 / 180.0;
        let dir = vec2<f32>(sin(angle_rad), cos(angle_rad));
        let center = shape.xy + shape.zw * 0.5;
        let half_diagonal = length(shape.zw) * 0.5;
        let t = dot(in.local_position - center, dir) / max(half_diagonal * 2.0, 0.000001) + 0.5;
        col = gradient_color(in.instance_index, clamp(t, 0.0, 1.0));

    } else if shape_type == 3u {
        let center = shape.xy + shape.zw * 0.5;
        let radius = max(shape.z * 0.5, 0.000001);
        let t = clamp(length(in.local_position - center) / radius, 0.0, 1.0);
        col = gradient_color(in.instance_index, t);
    }

    return vec4<f32>(col.rgb, col.a * instance.params.w * coverage);
}

@fragment
fn fs_solid(in: VertexOutput) -> @location(0) vec4<f32> {
    let instance = instances[in.instance_index];
    return vec4<f32>(instance.color.rgb, instance.color.a * instance.params.w);
}
"#;

// ────────────────────────────────────────────────────────────────────────────
// GPU State
// ────────────────────────────────────────────────────────────────────────────

/// GPU render context: device, queue, pipeline, and bind group layouts.
struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    globals_layout: wgpu::BindGroupLayout,
    instance_layout: wgpu::BindGroupLayout,
    max_texture_dimension_2d: u32,
}

impl GpuContext {
    fn new() -> Result<Self, RasterError> {
        pollster::block_on(Self::new_async())
    }

    async fn new_async() -> Result<Self, RasterError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| {
                RasterError::Init(
                    "No GPU adapter found. Ensure a GPU is available or use --backend native."
                        .into(),
                )
            })?;

        let limits = adapter.limits();
        let max_texture_dimension_2d = limits.max_texture_dimension_2d;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("dioxuscut-rasterizer"),
                    required_features: wgpu::Features::empty(),
                    required_limits: limits,
                    memory_hints: Default::default(),
                },
                None,
            )
            .await
            .map_err(|e| RasterError::Init(format!("GPU device creation failed: {e}")))?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("dioxuscut_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
        });

        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let instance_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("instance_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pipeline_layout"),
            bind_group_layouts: &[&globals_layout, &instance_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("dioxuscut_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: RENDER_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: SAMPLE_COUNT,
                ..Default::default()
            },
            multiview: None,
            cache: None,
        });

        let mesh_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("dioxuscut_mesh_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_mesh",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &MESH_ATTRIBUTES,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_solid",
                targets: &[Some(wgpu::ColorTargetState {
                    format: RENDER_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: SAMPLE_COUNT,
                ..Default::default()
            },
            multiview: None,
            cache: None,
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            mesh_pipeline,
            globals_layout,
            instance_layout,
            max_texture_dimension_2d,
        })
    }
}

const RING_BUFFER_SIZE: usize = 2;

/// A single buffered set of GPU render resources (MSAA texture, resolve target, readback buffer).
struct GpuFrameSlot {
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    _msaa_texture: wgpu::Texture,
    msaa_view: wgpu::TextureView,
    readback: wgpu::Buffer,
    bytes_per_row: u32,
}

impl GpuFrameSlot {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let bytes_per_row = align_to_256(width * 4);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: RENDER_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let msaa_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame_msaa_texture"),
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

        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback_buf"),
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
struct GpuFrameResources {
    slots: [GpuFrameSlot; RING_BUFFER_SIZE],
    active_index: usize,
}

impl GpuFrameResources {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        Self {
            slots: [
                GpuFrameSlot::new(device, width, height),
                GpuFrameSlot::new(device, width, height),
            ],
            active_index: 0,
        }
    }
}

struct InFlight {
    frame_idx: u32,
    slot_idx: usize,
    submission_index: wgpu::SubmissionIndex,
    rx: std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
}

// ────────────────────────────────────────────────────────────────────────────
// Backend
// ────────────────────────────────────────────────────────────────────────────

/// GPU-accelerated rasterizer using `wgpu`.
///
/// Requires a compatible GPU with Vulkan, Metal, DX12, or WebGPU support.
/// Use `TinySkiaBackend` if GPU access is unavailable (e.g. Docker/CI).
///
/// Textures and readback buffers are cached per resolution in a `Mutex`-
/// protected `HashMap`. This eliminates the per-frame allocation pressure
/// that existed in earlier versions.
pub struct WgpuBackend {
    ctx: GpuContext,
    /// Per-resolution GPU resource pool.  Key = `(width, height)`.
    frame_resources: Mutex<HashMap<(u32, u32), GpuFrameResources>>,
    fallback: TinySkiaBackend,
}

impl WgpuBackend {
    /// Create a new GPU backend. Initialises the device and render pipeline.
    pub fn new() -> Result<Self, RasterError> {
        let ctx = GpuContext::new()?;
        Ok(Self {
            ctx,
            frame_resources: Mutex::new(HashMap::new()),
            fallback: TinySkiaBackend::new(),
        })
    }

    /// Configure the image cache used when a scene falls back to CPU.
    pub fn with_image_cache_bytes(mut self, max_bytes: usize) -> Self {
        self.fallback = self.fallback.with_image_cache_bytes(max_bytes);
        self
    }

    fn submit_frame_to_slot(
        &self,
        commands: &[DrawCommand],
        width: u32,
        height: u32,
        slot: &GpuFrameSlot,
    ) -> Result<
        (
            wgpu::SubmissionIndex,
            std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
        ),
        RasterError,
    > {
        let device = &self.ctx.device;
        let queue = &self.ctx.queue;

        // Globals uniform
        let globals_data: [f32; 4] = [width as f32, height as f32, 0.0, 0.0];
        let globals_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globals_buf"),
            contents: bytemuck_cast(&globals_data),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals_bg"),
            layout: &self.ctx.globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame_encoder"),
        });

        let all_instances: Vec<GpuInstance> = commands.iter().map(|c| *c.instance()).collect();

        if !all_instances.is_empty() {
            let instance_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("instances_storage_buf"),
                contents: bytemuck_cast(&all_instances),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let instance_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("instances_bg"),
                layout: &self.ctx.instance_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: instance_buf.as_entire_binding(),
                }],
            });

            let mesh_buffers: Vec<Option<(wgpu::Buffer, wgpu::Buffer)>> = commands
                .iter()
                .map(|cmd| match cmd {
                    DrawCommand::Mesh {
                        vertices, indices, ..
                    } => {
                        let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("path_vertices"),
                            contents: bytemuck_cast(vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        });
                        let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("path_indices"),
                            contents: bytemuck_cast(indices),
                            usage: wgpu::BufferUsages::INDEX,
                        });
                        Some((vb, ib))
                    }
                    DrawCommand::Analytic { .. } => None,
                })
                .collect();

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &slot.msaa_view,
                    resolve_target: Some(&slot.texture_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });

            pass.set_bind_group(0, &globals_bg, &[]);
            pass.set_bind_group(1, &instance_bg, &[]);

            let mut i = 0;
            while i < commands.len() {
                match &commands[i] {
                    DrawCommand::Analytic { .. } => {
                        let start = i;
                        while i < commands.len()
                            && matches!(commands[i], DrawCommand::Analytic { .. })
                        {
                            i += 1;
                        }
                        pass.set_pipeline(&self.ctx.pipeline);
                        pass.draw(0..6, start as u32..i as u32);
                    }
                    DrawCommand::Mesh { indices, .. } => {
                        let (vb, ib) = mesh_buffers[i].as_ref().expect("mesh buffers allocated");
                        pass.set_pipeline(&self.ctx.mesh_pipeline);
                        pass.set_vertex_buffer(0, vb.slice(..));
                        pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..indices.len() as u32, 0, i as u32..i as u32 + 1);
                        i += 1;
                    }
                }
            }
        } else {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &slot.msaa_view,
                    resolve_target: Some(&slot.texture_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }

        encoder.copy_texture_to_buffer(
            slot.texture.as_image_copy(),
            wgpu::ImageCopyBuffer {
                buffer: &slot.readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(slot.bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let submission_index = queue.submit([encoder.finish()]);

        let (tx, rx) = std::sync::mpsc::channel();
        slot.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });

        Ok((submission_index, rx))
    }

    #[allow(clippy::type_complexity)]
    fn drain_slot(
        &self,
        in_flight: InFlight,
        res: &GpuFrameResources,
        width: u32,
        height: u32,
        scratch: &mut Vec<u8>,
        consume_fn: &mut dyn FnMut(u32, &[u8]) -> Result<(), RasterError>,
    ) -> Result<(), RasterError> {
        self.ctx
            .device
            .poll(wgpu::Maintain::wait_for(in_flight.submission_index));
        in_flight
            .rx
            .recv()
            .map_err(|_| RasterError::Frame {
                frame: in_flight.frame_idx,
                reason: "GPU readback channel error".into(),
            })?
            .map_err(|e| RasterError::Frame {
                frame: in_flight.frame_idx,
                reason: format!("GPU map error: {e:?}"),
            })?;

        let slot = &res.slots[in_flight.slot_idx];
        let bytes_per_row = slot.bytes_per_row;
        let expected_row_bytes = (width * 4) as usize;
        let total_bytes = expected_row_bytes * height as usize;
        let slice = slot.readback.slice(..);
        let data = slice.get_mapped_range();

        let res = if bytes_per_row as usize == expected_row_bytes {
            consume_fn(in_flight.frame_idx, &data[..total_bytes])
        } else {
            scratch.clear();
            scratch.reserve_exact(total_bytes);
            for row in 0..height {
                let start = (row * bytes_per_row) as usize;
                let end = start + expected_row_bytes;
                scratch.extend_from_slice(&data[start..end]);
            }
            consume_fn(in_flight.frame_idx, scratch)
        };
        drop(data);
        slot.readback.unmap();

        res
    }
}

impl RasterizerBackend for WgpuBackend {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            native_scene: true,
            browser_runtime: false,
            gpu_accelerated: true,
            supports_streaming: self.supports_streaming(),
        }
    }

    fn render_frame(&self, scene: &Scene, config: &FrameConfig) -> Result<RgbaImage, RasterError> {
        let Some(commands) = compile_scene(scene) else {
            return self.fallback.render_frame(scene, config);
        };

        if config.width > self.ctx.max_texture_dimension_2d
            || config.height > self.ctx.max_texture_dimension_2d
        {
            return self.fallback.render_frame(scene, config);
        }

        let width = config.width;
        let height = config.height;

        let mut pool = self
            .frame_resources
            .lock()
            .map_err(|_| RasterError::Init("GPU resource pool mutex poisoned".into()))?;
        let res = pool
            .entry((width, height))
            .or_insert_with(|| GpuFrameResources::new(&self.ctx.device, width, height));

        let slot_idx = res.active_index % RING_BUFFER_SIZE;
        res.active_index = res.active_index.wrapping_add(1);

        let (submission_index, rx) =
            self.submit_frame_to_slot(&commands, width, height, &res.slots[slot_idx])?;

        let mut out_pixels = None;
        let mut scratch = Vec::new();
        self.drain_slot(
            InFlight {
                frame_idx: config.frame,
                slot_idx,
                submission_index,
                rx,
            },
            res,
            width,
            height,
            &mut scratch,
            &mut |_frame, pixels| {
                out_pixels = Some(pixels.to_vec());
                Ok(())
            },
        )?;

        let pixels = out_pixels.ok_or_else(|| {
            RasterError::ImageEncode("Failed to assemble RgbaImage from GPU readback".into())
        })?;

        RgbaImage::from_raw(width, height, pixels).ok_or_else(|| {
            RasterError::ImageEncode("Failed to assemble RgbaImage from GPU readback".into())
        })
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    #[allow(clippy::type_complexity)]
    fn render_stream(
        &self,
        total: u32,
        scene_fn: &(dyn Fn(u32) -> Result<Scene, RasterError> + Sync),
        config_fn: &(dyn Fn(u32) -> FrameConfig + Sync),
        consume_fn: &mut dyn FnMut(u32, &[u8]) -> Result<(), RasterError>,
    ) -> Result<(), RasterError> {
        if total == 0 {
            return Ok(());
        }

        let first_cfg = config_fn(0);
        let width = first_cfg.width;
        let height = first_cfg.height;

        if width > self.ctx.max_texture_dimension_2d || height > self.ctx.max_texture_dimension_2d {
            return self
                .fallback
                .render_stream(total, scene_fn, config_fn, consume_fn);
        }

        let mut pool = self
            .frame_resources
            .lock()
            .map_err(|_| RasterError::Init("GPU resource pool mutex poisoned".into()))?;
        let res = pool
            .entry((width, height))
            .or_insert_with(|| GpuFrameResources::new(&self.ctx.device, width, height));

        let mut in_flight: Option<InFlight> = None;
        let mut scratch = Vec::new();

        for frame in 0..total {
            let scene = scene_fn(frame)?;
            let cfg = config_fn(frame);

            let Some(commands) = compile_scene(&scene) else {
                if let Some(prev) = in_flight.take() {
                    self.drain_slot(prev, res, width, height, &mut scratch, consume_fn)?;
                }
                let img = self.fallback.render_frame(&scene, &cfg)?;
                consume_fn(frame, img.as_raw())?;
                continue;
            };

            let slot_idx = (frame as usize) % RING_BUFFER_SIZE;

            // If the target slot is currently occupied by an in-flight frame, drain it now
            if let Some(prev) = in_flight.take() {
                if prev.slot_idx == slot_idx {
                    self.drain_slot(prev, res, width, height, &mut scratch, consume_fn)?;
                } else {
                    in_flight = Some(prev);
                }
            }

            // Submit this frame to GPU
            let (submission_index, rx) =
                self.submit_frame_to_slot(&commands, width, height, &res.slots[slot_idx])?;

            // Overlap: drain the previous frame while the newly submitted frame is being rendered on GPU
            if let Some(prev) = in_flight.take() {
                self.drain_slot(prev, res, width, height, &mut scratch, consume_fn)?;
            }

            in_flight = Some(InFlight {
                frame_idx: frame,
                slot_idx,
                submission_index,
                rx,
            });
        }

        // Drain any remaining in-flight frame at the end of the stream
        if let Some(prev) = in_flight.take() {
            self.drain_slot(prev, res, width, height, &mut scratch, consume_fn)?;
        }

        Ok(())
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Helpers
// ────────────────────────────────────────────────────────────────────────────

/// Raw instance data layout matching the WGSL `InstanceData` struct.
#[repr(C)]
#[derive(Clone, Copy)]
struct GpuInstance {
    kind_data: [u32; 4],
    bounds: [f32; 4],
    shape_bounds: [f32; 4],
    color: [f32; 4],
    color2: [f32; 4],
    params: [f32; 4],
    transform_x: [f32; 4],
    transform_y: [f32; 4],
    stop_positions: [[f32; 4]; MAX_GRADIENT_STOPS],
    stop_colors: [[f32; 4]; MAX_GRADIENT_STOPS],
}

impl GpuInstance {
    fn solid(color: Color, opacity: f32, transform: Transform) -> Self {
        let (transform_x, transform_y) = transform_rows(transform);
        Self {
            kind_data: [4, 0, 0, 0],
            bounds: [0.0; 4],
            shape_bounds: [0.0; 4],
            color: color_to_f32(color),
            color2: [0.0; 4],
            params: [0.0, 0.0, 0.0, opacity],
            transform_x,
            transform_y,
            stop_positions: [[0.0; 4]; MAX_GRADIENT_STOPS],
            stop_colors: [[0.0; 4]; MAX_GRADIENT_STOPS],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
struct GpuVertex {
    position: [f32; 2],
}

struct PositionConstructor;

impl FillVertexConstructor<GpuVertex> for PositionConstructor {
    fn new_vertex(&mut self, vertex: FillVertex<'_>) -> GpuVertex {
        GpuVertex {
            position: vertex.position().to_array(),
        }
    }
}

enum DrawCommand {
    Analytic {
        instance: GpuInstance,
    },
    Mesh {
        instance: GpuInstance,
        vertices: Vec<GpuVertex>,
        indices: Vec<u32>,
    },
}

impl DrawCommand {
    fn instance(&self) -> &GpuInstance {
        match self {
            Self::Analytic { instance } | Self::Mesh { instance, .. } => instance,
        }
    }
}

fn compile_scene(scene: &Scene) -> Option<Vec<DrawCommand>> {
    let mut commands = Vec::new();
    compile_nodes(&scene.nodes, Transform::identity(), 1.0, &mut commands)?;
    Some(commands)
}

fn compile_nodes(
    nodes: &[SceneNode],
    transform: Transform,
    opacity: f32,
    output: &mut Vec<DrawCommand>,
) -> Option<()> {
    for node in nodes {
        match node {
            SceneNode::Rect {
                x,
                y,
                w,
                h,
                fill,
                stroke,
                stroke_width,
                corner_radius,
            } => {
                let active_stroke = stroke.filter(|_| *stroke_width > 0.0);
                let expansion = active_stroke
                    .map(|_| stroke_width * 0.5 + 1.0)
                    .unwrap_or(0.0);
                let mut instance = GpuInstance::solid(*fill, opacity, transform);
                instance.kind_data[0] = 0;
                instance.bounds = [
                    x - expansion,
                    y - expansion,
                    w + expansion * 2.0,
                    h + expansion * 2.0,
                ];
                instance.shape_bounds = [*x, *y, *w, *h];
                instance.color2 = color_to_f32(active_stroke.unwrap_or(*fill));
                instance.params[0] = *corner_radius;
                instance.params[1] = active_stroke.map(|_| *stroke_width).unwrap_or(0.0);
                output.push(DrawCommand::Analytic { instance });
            }

            SceneNode::Circle {
                cx,
                cy,
                r,
                fill,
                stroke,
                stroke_width,
            } => {
                let active_stroke = stroke.filter(|_| *stroke_width > 0.0);
                let expansion = active_stroke
                    .map(|_| stroke_width * 0.5 + 1.0)
                    .unwrap_or(0.0);
                let mut instance = GpuInstance::solid(*fill, opacity, transform);
                instance.kind_data[0] = 1;
                instance.bounds = [
                    cx - r - expansion,
                    cy - r - expansion,
                    r * 2.0 + expansion * 2.0,
                    r * 2.0 + expansion * 2.0,
                ];
                instance.shape_bounds = [cx - r, cy - r, r * 2.0, r * 2.0];
                instance.color2 = color_to_f32(active_stroke.unwrap_or(*fill));
                instance.params[1] = active_stroke.map(|_| *stroke_width).unwrap_or(0.0);
                output.push(DrawCommand::Analytic { instance });
            }

            SceneNode::Path {
                d,
                fill,
                stroke,
                stroke_width,
                opacity: node_opacity,
            } => {
                let path = svgpath_to_tiny_skia(d)?;
                let combined_opacity = opacity * node_opacity;
                if let Some(fill) = fill {
                    output.push(mesh_command(&path, *fill, combined_opacity, transform)?);
                }
                if let Some(stroke) = stroke.filter(|_| *stroke_width > 0.0) {
                    let stroked = path.stroke(
                        &Stroke {
                            width: *stroke_width,
                            ..Default::default()
                        },
                        transform
                            .get_scale()
                            .0
                            .max(transform.get_scale().1)
                            .max(1.0),
                    )?;
                    output.push(mesh_command(&stroked, stroke, combined_opacity, transform)?);
                }
            }

            SceneNode::LinearGradient {
                x,
                y,
                w,
                h,
                angle_deg,
                stops,
            } => {
                if stops.is_empty() {
                    continue;
                }
                let mut instance = gradient_instance(stops, opacity, transform)?;
                instance.kind_data[0] = 2;
                instance.bounds = [*x, *y, *w, *h];
                instance.shape_bounds = instance.bounds;
                instance.params[2] = *angle_deg;
                output.push(DrawCommand::Analytic { instance });
            }

            SceneNode::RadialGradient { cx, cy, r, stops } => {
                if stops.is_empty() {
                    continue;
                }
                let mut instance = gradient_instance(stops, opacity, transform)?;
                instance.kind_data[0] = 3;
                instance.bounds = [cx - r, cy - r, r * 2.0, r * 2.0];
                instance.shape_bounds = instance.bounds;
                output.push(DrawCommand::Analytic { instance });
            }

            SceneNode::Group {
                transform: group_transform,
                opacity: group_opacity,
                children,
            } => {
                let next_transform = transform.post_concat(group_transform.to_tiny_skia());
                if !next_transform.is_finite() || !group_opacity.is_finite() {
                    return None;
                }
                compile_nodes(children, next_transform, opacity * group_opacity, output)?;
            }

            // A layer with no offscreen-only effect is semantically just an
            // opacity group. Keep it on the GPU instead of forcing the whole
            // frame through tiny-skia; complex layers still take the fallback
            // path below so their compositing semantics remain exact.
            SceneNode::Layer {
                opacity: layer_opacity,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: None,
                filters,
                shadow: None,
                children,
                ..
            } if gpu_layer_opacity(filters, *layer_opacity).is_some() => {
                compile_nodes(
                    children,
                    transform,
                    opacity * gpu_layer_opacity(filters, *layer_opacity).unwrap(),
                    output,
                )?;
            }

            SceneNode::Audio { .. } => {}
            SceneNode::Text { .. }
            | SceneNode::Image { .. }
            | SceneNode::Video { .. }
            | SceneNode::Gif { .. }
            | SceneNode::Layer { .. }
            | SceneNode::Emoji { .. }
            | SceneNode::Lottie { .. }
            | SceneNode::AudioVisualizer { .. }
            | SceneNode::Shader { .. } => return None,
        }
    }
    Some(())
}

fn gpu_layer_opacity(filters: &[crate::scene::SceneFilter], layer_opacity: f32) -> Option<f32> {
    if !layer_opacity.is_finite() {
        return None;
    }
    filters.iter().try_fold(layer_opacity, |opacity, filter| {
        let crate::scene::SceneFilter::Opacity { amount } = filter else {
            return None;
        };
        (amount.is_finite() && (0.0..=1.0).contains(amount)).then(|| opacity * amount)
    })
}

#[cfg(test)]
fn gpu_supports_scene(scene: &Scene) -> bool {
    compile_scene(scene).is_some()
}

fn gradient_instance(
    stops: &[GradientStop],
    opacity: f32,
    transform: Transform,
) -> Option<GpuInstance> {
    if stops.len() > MAX_GRADIENT_STOPS
        || stops.iter().any(|stop| !stop.position.is_finite())
        || !opacity.is_finite()
    {
        return None;
    }
    let mut sorted = stops.to_vec();
    sorted.sort_by(|left, right| left.position.total_cmp(&right.position));
    let mut instance = GpuInstance::solid(sorted[0].color, opacity, transform);
    instance.kind_data[1] = sorted.len() as u32;
    for (index, stop) in sorted.iter().enumerate() {
        instance.stop_positions[index][0] = stop.position.clamp(0.0, 1.0);
        instance.stop_colors[index] = color_to_f32(stop.color);
    }
    Some(instance)
}

fn mesh_command(
    path: &TinyPath,
    color: Color,
    opacity: f32,
    transform: Transform,
) -> Option<DrawCommand> {
    let lyon_path = tiny_path_to_lyon(path)?;
    let mut geometry: VertexBuffers<GpuVertex, u32> = VertexBuffers::new();
    FillTessellator::new()
        .tessellate_path(
            &lyon_path,
            &FillOptions::non_zero().with_tolerance(0.1),
            &mut BuffersBuilder::new(&mut geometry, PositionConstructor),
        )
        .ok()?;
    Some(DrawCommand::Mesh {
        instance: GpuInstance::solid(color, opacity, transform),
        vertices: geometry.vertices,
        indices: geometry.indices,
    })
}

fn tiny_path_to_lyon(path: &TinyPath) -> Option<LyonPath> {
    let mut builder = LyonPath::builder();
    let mut open = false;
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(position) => {
                if open {
                    builder.end(false);
                }
                builder.begin(point(position.x, position.y));
                open = true;
            }
            PathSegment::LineTo(position) => {
                builder.line_to(point(position.x, position.y));
            }
            PathSegment::QuadTo(control, position) => {
                builder.quadratic_bezier_to(
                    point(control.x, control.y),
                    point(position.x, position.y),
                );
            }
            PathSegment::CubicTo(control1, control2, position) => {
                builder.cubic_bezier_to(
                    point(control1.x, control1.y),
                    point(control2.x, control2.y),
                    point(position.x, position.y),
                );
            }
            PathSegment::Close => {
                builder.end(true);
                open = false;
            }
        }
    }
    if open {
        builder.end(false);
    }
    Some(builder.build())
}

fn transform_rows(transform: Transform) -> ([f32; 4], [f32; 4]) {
    (
        [transform.sx, transform.kx, transform.tx, 0.0],
        [transform.ky, transform.sy, transform.ty, 0.0],
    )
}

/// Convert a u8 sRGB channel value (0–255) to linear float (0.0–1.0).
///
/// The render target is `Rgba8Unorm` (linear), so fragment colour uniforms
/// must be in linear light to avoid the double-gamma problem.
#[inline]
fn srgb_to_linear(channel: u8) -> f32 {
    let s = channel as f32 / 255.0;
    // IEC 61966-2-1 sRGB transfer function (precise form)
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

fn color_to_f32(c: Color) -> [f32; 4] {
    [
        srgb_to_linear(c.r),
        srgb_to_linear(c.g),
        srgb_to_linear(c.b),
        c.a as f32 / 255.0, // alpha is always linear
    ]
}

fn align_to_256(n: u32) -> u32 {
    (n + 255) & !255
}

/// Zero-copy reinterpret of a `&[T]` as `&[u8]`.
fn bytemuck_cast<T: Copy>(data: &[T]) -> &[u8] {
    let len = std::mem::size_of_val(data);
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, len) }
}

#[cfg(test)]
mod support_tests {
    use super::*;

    #[test]
    fn unsupported_nodes_trigger_cpu_fallback() {
        let mut scene = Scene::new();
        scene.push(SceneNode::Text {
            x: 0.0,
            y: 20.0,
            content: "text".into(),
            font_size: 20.0,
            color: Color::WHITE,
            font_weight: 400,
            font_sources: Vec::new(),
        });
        assert!(!gpu_supports_scene(&scene));

        let image_scene = Scene {
            nodes: vec![SceneNode::Image {
                src: "asset.png".into(),
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
                fit: crate::scene::ImageFit::Cover,
                opacity: 1.0,
            }],
        };
        assert!(!gpu_supports_scene(&image_scene));

        let invalid_path_scene = Scene {
            nodes: vec![SceneNode::Path {
                d: "M 0 0 L nope".into(),
                fill: Some(Color::WHITE),
                stroke: None,
                stroke_width: 0.0,
                opacity: 1.0,
            }],
        };
        assert!(!gpu_supports_scene(&invalid_path_scene));
    }

    #[test]
    fn path_strokes_groups_and_multistop_gradients_compile_for_gpu() {
        let scene = Scene {
            nodes: vec![SceneNode::Group {
                transform: crate::scene::Transform2D {
                    tx: 12.0,
                    ty: 8.0,
                    scale_x: 1.5,
                    scale_y: 0.75,
                    rotate_deg: 15.0,
                },
                opacity: 0.6,
                children: vec![
                    SceneNode::Path {
                        d: "M 0 0 C 20 0 20 20 40 20 L 40 40 Z".into(),
                        fill: Some(Color::rgb(255, 0, 0)),
                        stroke: Some(Color::WHITE),
                        stroke_width: 3.0,
                        opacity: 0.8,
                    },
                    SceneNode::Rect {
                        x: 45.0,
                        y: 0.0,
                        w: 20.0,
                        h: 20.0,
                        fill: Color::BLACK,
                        stroke: Some(Color::WHITE),
                        stroke_width: 2.0,
                        corner_radius: 4.0,
                    },
                    SceneNode::Circle {
                        cx: 75.0,
                        cy: 10.0,
                        r: 8.0,
                        fill: Color::BLACK,
                        stroke: Some(Color::WHITE),
                        stroke_width: 2.0,
                    },
                    SceneNode::LinearGradient {
                        x: 0.0,
                        y: 45.0,
                        w: 80.0,
                        h: 20.0,
                        angle_deg: 90.0,
                        stops: vec![
                            GradientStop {
                                position: 0.0,
                                color: Color::rgb(255, 0, 0),
                            },
                            GradientStop {
                                position: 0.5,
                                color: Color::rgb(0, 255, 0),
                            },
                            GradientStop {
                                position: 1.0,
                                color: Color::rgb(0, 0, 255),
                            },
                        ],
                    },
                ],
            }],
        };

        assert!(gpu_supports_scene(&scene));
        let commands = compile_scene(&scene).unwrap();
        assert_eq!(
            commands.len(),
            5,
            "path fill/stroke, rect, circle, gradient"
        );
        assert_eq!(
            commands
                .iter()
                .filter(|command| matches!(command, DrawCommand::Mesh { .. }))
                .count(),
            2
        );
        let gradient = commands.last().unwrap().instance();
        assert_eq!(gradient.kind_data[1], 3);
        assert!((gradient.params[3] - 0.6).abs() < f32::EPSILON);

        let expected = crate::scene::Transform2D {
            tx: 12.0,
            ty: 8.0,
            scale_x: 1.5,
            scale_y: 0.75,
            rotate_deg: 15.0,
        }
        .to_tiny_skia();
        assert_eq!(gradient.transform_x, transform_rows(expected).0);
        assert_eq!(gradient.transform_y, transform_rows(expected).1);
    }

    #[test]
    fn gradients_beyond_the_uniform_limit_use_cpu_fallback() {
        let scene = Scene {
            nodes: vec![SceneNode::LinearGradient {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0,
                angle_deg: 0.0,
                stops: (0..=MAX_GRADIENT_STOPS)
                    .map(|index| GradientStop {
                        position: index as f32 / MAX_GRADIENT_STOPS as f32,
                        color: Color::WHITE,
                    })
                    .collect(),
            }],
        };

        assert!(!gpu_supports_scene(&scene));
    }

    #[test]
    fn plain_normal_layers_compile_as_gpu_opacity_groups() {
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 0.5,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: None,
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: vec![crate::scene::SceneFilter::Opacity { amount: 0.5 }],
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 10.0,
                    h: 10.0,
                    fill: Color::WHITE,
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                }],
            }],
        };
        assert!(gpu_supports_scene(&scene));
        assert!(
            (compile_scene(&scene).unwrap()[0].instance().params[3] - 0.25).abs() < f32::EPSILON
        );
    }

    #[test]
    fn invalid_opacity_filter_does_not_enter_gpu_path() {
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 1.0,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: None,
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: vec![crate::scene::SceneFilter::Opacity { amount: 1.1 }],
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                    fill: Color::WHITE,
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                }],
            }],
        };
        assert!(!gpu_supports_scene(&scene));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wgpu_backend_init() {
        match WgpuBackend::new() {
            Ok(backend) => {
                println!("GPU backend initialised successfully");
                // Render a minimal 64x64 scene
                let scene = crate::scene::Scene::new();
                let config = FrameConfig::new(64, 64, 0, 30.0);
                let img = backend
                    .render_frame(&scene, &config)
                    .expect("GPU render failed");
                assert_eq!(img.width(), 64);
                assert_eq!(img.height(), 64);
            }
            Err(e) => {
                // In CI / headless without GPU this is expected
                println!("GPU backend unavailable (expected in headless CI): {e}");
            }
        }
    }

    #[test]
    fn gpu_renders_transformed_path_stroke_and_three_stop_gradient() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping render comparison");
            return;
        };
        let scene = Scene {
            nodes: vec![
                SceneNode::Group {
                    transform: crate::scene::Transform2D {
                        tx: 10.0,
                        ty: 5.0,
                        ..Default::default()
                    },
                    opacity: 1.0,
                    children: vec![SceneNode::Path {
                        d: "M 5 5 L 30 5 L 30 25 L 5 25 Z".into(),
                        fill: Some(Color::rgb(255, 0, 0)),
                        stroke: Some(Color::WHITE),
                        stroke_width: 2.0,
                        opacity: 1.0,
                    }],
                },
                SceneNode::LinearGradient {
                    x: 0.0,
                    y: 36.0,
                    w: 96.0,
                    h: 24.0,
                    angle_deg: 90.0,
                    stops: vec![
                        GradientStop {
                            position: 0.0,
                            color: Color::rgb(255, 0, 0),
                        },
                        GradientStop {
                            position: 0.5,
                            color: Color::rgb(0, 255, 0),
                        },
                        GradientStop {
                            position: 1.0,
                            color: Color::rgb(0, 0, 255),
                        },
                    ],
                },
                SceneNode::Rect {
                    x: 60.0,
                    y: 8.0,
                    w: 24.0,
                    h: 18.0,
                    fill: Color::rgb(0, 0, 255),
                    stroke: Some(Color::WHITE),
                    stroke_width: 4.0,
                    corner_radius: 4.0,
                },
            ],
        };
        let config = FrameConfig::new(96, 64, 0, 30.0);
        let gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let cpu_image = TinySkiaBackend::headless()
            .render_frame(&scene, &config)
            .unwrap();

        let path_pixel = gpu_image.get_pixel(20, 16);
        assert!(path_pixel[0] > 200 && path_pixel[3] > 200);
        let middle_gradient = gpu_image.get_pixel(48, 48);
        assert!(middle_gradient[1] > middle_gradient[0]);
        assert!(middle_gradient[1] > middle_gradient[2]);
        let rect_stroke = gpu_image.get_pixel(60, 16);
        assert!(rect_stroke[0] > 180 && rect_stroke[1] > 180 && rect_stroke[2] > 180);

        let alpha_error = gpu_image
            .pixels()
            .zip(cpu_image.pixels())
            .map(|(gpu, cpu)| (i16::from(gpu[3]) - i16::from(cpu[3])).unsigned_abs() as u64)
            .sum::<u64>();
        let mean_alpha_error = alpha_error as f64 / (config.width * config.height) as f64;
        assert!(
            mean_alpha_error < 8.0,
            "CPU/GPU mean alpha error was {mean_alpha_error}"
        );
    }

    #[test]
    fn gpu_opacity_layer_preserves_alpha_within_tolerance() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping layer parity test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 0.5,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: None,
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: vec![crate::scene::SceneFilter::Opacity { amount: 0.5 }],
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 8.0,
                    y: 8.0,
                    w: 32.0,
                    h: 24.0,
                    fill: Color::rgb(240, 120, 40),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                }],
            }],
        };
        let config = FrameConfig::new(64, 64, 0, 30.0);
        let gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let cpu_image = TinySkiaBackend::headless()
            .render_frame(&scene, &config)
            .unwrap();
        let alpha_error = gpu_image
            .pixels()
            .zip(cpu_image.pixels())
            .map(|(gpu, cpu)| (i16::from(gpu[3]) - i16::from(cpu[3])).unsigned_abs() as u64)
            .sum::<u64>();
        let mean_alpha_error = alpha_error as f64 / (config.width * config.height) as f64;
        assert!(
            mean_alpha_error < 8.0,
            "CPU/GPU layer mean alpha error was {mean_alpha_error}"
        );
        let rgb_error = gpu_image
            .pixels()
            .zip(cpu_image.pixels())
            .map(|(gpu, cpu)| {
                (0..3)
                    .map(|channel| {
                        (i16::from(gpu[channel]) - i16::from(cpu[channel])).unsigned_abs() as u64
                    })
                    .sum::<u64>()
            })
            .sum::<u64>();
        let mean_rgb_error = rgb_error as f64 / (config.width * config.height * 3) as f64;
        assert!(
            mean_rgb_error < 8.0,
            "CPU/GPU layer mean RGB error was {mean_rgb_error}"
        );
    }

    #[test]
    fn gpu_render_stream_pipelined_matches_render_frame() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping render_stream test");
            return;
        };

        let width = 160;
        let height = 90;
        let total_frames = 5;

        let make_scene = |frame: u32| {
            let offset = frame as f32 * 10.0;
            Scene {
                nodes: vec![
                    SceneNode::Rect {
                        x: offset,
                        y: 10.0,
                        w: 40.0,
                        h: 30.0,
                        fill: Color::rgb(200, 50, 80),
                        stroke: Some(Color::WHITE),
                        stroke_width: 2.0,
                        corner_radius: 4.0,
                    },
                    SceneNode::Circle {
                        cx: 80.0,
                        cy: 45.0 + offset * 0.5,
                        r: 20.0,
                        fill: Color::rgba(50, 150, 250, 180),
                        stroke: None,
                        stroke_width: 0.0,
                    },
                ],
            }
        };

        // 1. Render sequentially using render_frame
        let mut sequential_frames = Vec::new();
        for f in 0..total_frames {
            let scene = make_scene(f);
            let cfg = FrameConfig::new(width, height, f, 30.0);
            let img = gpu.render_frame(&scene, &cfg).expect("render_frame failed");
            sequential_frames.push(img.into_raw());
        }

        // 2. Render pipelined stream using render_stream
        let mut streamed_frames = Vec::new();
        gpu.render_stream(
            total_frames,
            &|f| Ok(make_scene(f)),
            &|f| FrameConfig::new(width, height, f, 30.0),
            &mut |_f, rgba| {
                streamed_frames.push(rgba.to_vec());
                Ok(())
            },
        )
        .expect("render_stream failed");

        assert_eq!(streamed_frames.len(), total_frames as usize);
        for f in 0..total_frames as usize {
            assert_eq!(
                streamed_frames[f], sequential_frames[f],
                "Frame {f} rendered via render_stream does not match render_frame"
            );
        }
    }
}

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

#![cfg(feature = "gpu")]

use crate::backend::{FrameConfig, RasterError, RasterizerBackend};
use crate::scene::{Color, GradientStop, Scene, SceneNode};
use crate::tiny_skia_backend::{svgpath_to_tiny_skia, TinySkiaBackend};
use image::RgbaImage;
use lyon_tessellation::geometry_builder::{BuffersBuilder, FillVertexConstructor, VertexBuffers};
use lyon_tessellation::math::point;
use lyon_tessellation::path::Path as LyonPath;
use lyon_tessellation::{FillOptions, FillTessellator, FillVertex};
use tiny_skia::{Path as TinyPath, PathSegment, Stroke, Transform};
use wgpu::util::DeviceExt;

const MAX_GRADIENT_STOPS: usize = 16;
const SAMPLE_COUNT: u32 = 4;
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
@group(1) @binding(0) var<uniform> instance: InstanceData;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local_position: vec2<f32>,
};

fn transform_position(local: vec2<f32>) -> vec2<f32> {
    let value = vec3<f32>(local, 1.0);
    return vec2<f32>(
        dot(instance.transform_x.xyz, value),
        dot(instance.transform_y.xyz, value),
    );
}

fn to_clip_position(pixel_position: vec2<f32>) -> vec4<f32> {
    let ndc = vec2<f32>(
         pixel_position.x / globals.resolution.x * 2.0 - 1.0,
        -pixel_position.y / globals.resolution.y * 2.0 + 1.0,
    );
    return vec4<f32>(ndc, 0.0, 1.0);
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VertexOutput {
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
    return VertexOutput(to_clip_position(transform_position(pixel_pos)), pixel_pos);
}

struct MeshVertexInput {
    @location(0) position: vec2<f32>,
};

@vertex
fn vs_mesh(vertex: MeshVertexInput) -> VertexOutput {
    return VertexOutput(to_clip_position(transform_position(vertex.position)), vertex.position);
}

fn gradient_color(t: f32) -> vec4<f32> {
    let count = max(instance.kind_data.y, 1u);
    if t <= instance.stop_positions[0].x {
        return instance.stop_colors[0];
    }

    var previous_position = instance.stop_positions[0].x;
    var previous_color = instance.stop_colors[0];
    for (var index = 1u; index < 16u; index = index + 1u) {
        if index >= count {
            break;
        }
        let next_position = instance.stop_positions[index].x;
        let next_color = instance.stop_colors[index];
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
        col = gradient_color(clamp(t, 0.0, 1.0));

    } else if shape_type == 3u {
        let center = shape.xy + shape.zw * 0.5;
        let radius = max(shape.z * 0.5, 0.000001);
        let t = clamp(length(in.local_position - center) / radius, 0.0, 1.0);
        col = gradient_color(t);
    }

    return vec4<f32>(col.rgb, col.a * instance.params.w * coverage);
}

@fragment
fn fs_solid(_in: VertexOutput) -> @location(0) vec4<f32> {
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

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("dioxuscut-rasterizer"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults(),
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
                    ty: wgpu::BufferBindingType::Uniform,
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
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
        })
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Backend
// ────────────────────────────────────────────────────────────────────────────

/// GPU-accelerated rasterizer using `wgpu`.
///
/// Requires a compatible GPU with Vulkan, Metal, DX12, or WebGPU support.
/// Use `TinySkiaBackend` if GPU access is unavailable (e.g. Docker/CI).
pub struct WgpuBackend {
    ctx: GpuContext,
    fallback: TinySkiaBackend,
}

impl WgpuBackend {
    /// Create a new GPU backend. Initialises the device and pipeline.
    pub fn new() -> Result<Self, RasterError> {
        let ctx = GpuContext::new()?;
        Ok(Self {
            ctx,
            fallback: TinySkiaBackend::new(),
        })
    }
}

impl RasterizerBackend for WgpuBackend {
    fn render_frame(&self, scene: &Scene, config: &FrameConfig) -> Result<RgbaImage, RasterError> {
        let Some(commands) = compile_scene(scene) else {
            return self.fallback.render_frame(scene, config);
        };

        let width = config.width;
        let height = config.height;
        let device = &self.ctx.device;
        let queue = &self.ctx.queue;

        // ── Offscreen render target ──────────────────────────────────────────
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
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let multisampled_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame_msaa_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: SAMPLE_COUNT,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let multisampled_view =
            multisampled_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // ── Globals uniform buffer ───────────────────────────────────────────
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

        // ── Render all scene nodes ───────────────────────────────────────────
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame_encoder"),
        });

        // Clear pass
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &multisampled_view,
                    resolve_target: Some(&texture_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }

        // Draw each compiled node in scene order.
        for command in &commands {
            let instance_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("instance_buf"),
                contents: bytemuck_cast(std::slice::from_ref(command.instance())),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let instance_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("instance_bg"),
                layout: &self.ctx.instance_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: instance_buf.as_entire_binding(),
                }],
            });

            let vertex_buf = match command {
                DrawCommand::Mesh { vertices, .. } => Some(device.create_buffer_init(
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("path_vertices"),
                        contents: bytemuck_cast(vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    },
                )),
                DrawCommand::Analytic { .. } => None,
            };
            let index_buf = match command {
                DrawCommand::Mesh { indices, .. } => Some(device.create_buffer_init(
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("path_indices"),
                        contents: bytemuck_cast(indices),
                        usage: wgpu::BufferUsages::INDEX,
                    },
                )),
                DrawCommand::Analytic { .. } => None,
            };

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("draw_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &multisampled_view,
                    resolve_target: Some(&texture_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });

            pass.set_bind_group(0, &globals_bg, &[]);
            pass.set_bind_group(1, &instance_bg, &[]);
            match command {
                DrawCommand::Analytic { .. } => {
                    pass.set_pipeline(&self.ctx.pipeline);
                    pass.draw(0..6, 0..1);
                }
                DrawCommand::Mesh { indices, .. } => {
                    pass.set_pipeline(&self.ctx.mesh_pipeline);
                    pass.set_vertex_buffer(
                        0,
                        vertex_buf.as_ref().expect("mesh vertex buffer").slice(..),
                    );
                    pass.set_index_buffer(
                        index_buf.as_ref().expect("mesh index buffer").slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
                }
            }
        }

        // ── Pixel readback ───────────────────────────────────────────────────
        let bytes_per_row = align_to_256(width * 4);
        let readback_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback_buf"),
            size: (bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::ImageCopyBuffer {
                buffer: &readback_buf,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        queue.submit([encoder.finish()]);

        // Map and read
        let slice = readback_buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|_| RasterError::Frame {
                frame: config.frame,
                reason: "GPU readback channel error".into(),
            })?
            .map_err(|e| RasterError::Frame {
                frame: config.frame,
                reason: format!("GPU map error: {e:?}"),
            })?;

        let data = slice.get_mapped_range();
        // Strip row padding
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let start = (row * bytes_per_row) as usize;
            let end = start + (width * 4) as usize;
            pixels.extend_from_slice(&data[start..end]);
        }
        drop(data);
        readback_buf.unmap();

        RgbaImage::from_raw(width, height, pixels).ok_or_else(|| {
            RasterError::ImageEncode("Failed to assemble RgbaImage from GPU readback".into())
        })
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
                let Some(path) = svgpath_to_tiny_skia(d) else {
                    continue;
                };
                let combined_opacity = opacity * node_opacity;
                if let Some(fill) = fill {
                    output.push(mesh_command(&path, *fill, combined_opacity, transform)?);
                }
                if let Some(stroke) = stroke.filter(|_| *stroke_width > 0.0) {
                    if let Some(stroked) = path.stroke(
                        &Stroke {
                            width: *stroke_width,
                            ..Default::default()
                        },
                        transform
                            .get_scale()
                            .0
                            .max(transform.get_scale().1)
                            .max(1.0),
                    ) {
                        output.push(mesh_command(&stroked, stroke, combined_opacity, transform)?);
                    }
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

            SceneNode::Audio { .. } => {}
            SceneNode::Text { .. }
            | SceneNode::Image { .. }
            | SceneNode::Video { .. }
            | SceneNode::Layer { .. } => return None,
        }
    }
    Some(())
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

fn color_to_f32(c: Color) -> [f32; 4] {
    [
        c.r as f32 / 255.0,
        c.g as f32 / 255.0,
        c.b as f32 / 255.0,
        c.a as f32 / 255.0,
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
}

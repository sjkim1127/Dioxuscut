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
use crate::image_cache::ImageCache;
use crate::scene::ImageFit;
use crate::scene::{Color, GradientStop, Scene, SceneNode};
use crate::tiny_skia_backend::{svgpath_to_tiny_skia, TinySkiaBackend};
use crate::video_cache::{canonical_local_path, VideoFrameCache};
use image::RgbaImage;
use lyon_tessellation::geometry_builder::{BuffersBuilder, FillVertexConstructor, VertexBuffers};
use lyon_tessellation::math::point;
use lyon_tessellation::path::Path as LyonPath;
use lyon_tessellation::{FillOptions, FillTessellator, FillVertex};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tiny_skia::{Path as TinyPath, PathSegment, Stroke, Transform};
use wgpu::util::DeviceExt;

const MAX_GRADIENT_STOPS: usize = 16;
const SAMPLE_COUNT: u32 = 4;
/// Internal linear-light render format. **Not** sRGB to avoid double-gamma.
const RENDER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const MESH_ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x2];

#[derive(Debug, Clone, Copy, PartialEq)]
struct ImagePlacement {
    /// Destination rectangle in canvas pixels: x, y, width, height.
    destination: [f32; 4],
    /// Normalized source rectangle: left, top, right, bottom.
    source_uv: [f32; 4],
}

/// Resolve CSS/Remotion-style image fitting into a destination rectangle and
/// normalized source crop. Keeping this independent of wgpu lets the CPU and
/// GPU image paths use exactly the same geometry.
fn image_placement(
    fit: ImageFit,
    image_width: f32,
    image_height: f32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> Option<ImagePlacement> {
    if ![image_width, image_height, x, y, width, height]
        .iter()
        .all(|value| value.is_finite())
        || image_width <= 0.0
        || image_height <= 0.0
        || width <= 0.0
        || height <= 0.0
    {
        return None;
    }
    let (draw_width, draw_height, source_uv) = match fit {
        ImageFit::Fill => (width, height, [0.0, 0.0, 1.0, 1.0]),
        ImageFit::None => {
            // Match the CPU backend's centered natural-size overlay into a
            // transparent destination box. Oversized source dimensions are
            // center-cropped; undersized dimensions remain letterboxed.
            let draw_width = image_width.min(width);
            let draw_height = image_height.min(height);
            let visible_width = draw_width / image_width;
            let visible_height = draw_height / image_height;
            let left = (1.0 - visible_width) * 0.5;
            let top = (1.0 - visible_height) * 0.5;
            (
                draw_width,
                draw_height,
                [left, top, left + visible_width, top + visible_height],
            )
        }
        ImageFit::Contain | ImageFit::ScaleDown => {
            let scale = if matches!(fit, ImageFit::ScaleDown) {
                (1.0_f32).min((width / image_width).min(height / image_height))
            } else {
                (width / image_width).min(height / image_height)
            };
            let draw_width = image_width * scale;
            let draw_height = image_height * scale;
            (draw_width, draw_height, [0.0, 0.0, 1.0, 1.0])
        }
        ImageFit::Cover => {
            let scale = (width / image_width).max(height / image_height);
            let visible_width = width / scale / image_width;
            let visible_height = height / scale / image_height;
            let left = (1.0 - visible_width) * 0.5;
            let top = (1.0 - visible_height) * 0.5;
            (
                width,
                height,
                [left, top, left + visible_width, top + visible_height],
            )
        }
    };
    Some(ImagePlacement {
        destination: [
            x + (width - draw_width) * 0.5,
            y + (height - draw_height) * 0.5,
            draw_width,
            draw_height,
        ],
        source_uv,
    })
}

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
    brightness: vec4<f32>,
    grayscale: vec4<f32>,
    contrast: vec4<f32>,
    saturation: vec4<f32>,
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
@group(2) @binding(0) var image_texture: texture_2d<f32>;
@group(2) @binding(1) var image_sampler: sampler;

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

fn linear_to_srgb(channel: f32) -> f32 {
    if channel <= 0.0031308 {
        return channel * 12.92;
    }
    return 1.055 * pow(channel, 1.0 / 2.4) - 0.055;
}

fn apply_color_filters(color: vec4<f32>, instance: InstanceData) -> vec4<f32> {
    if instance.brightness.x == 1.0 && instance.grayscale.x == 0.0 && instance.contrast.x == 1.0 && instance.saturation.x == 1.0 {
        return color;
    }
    var srgb = color.rgb;
    if instance.kind_data.x != 5u {
        srgb = vec3<f32>(
            linear_to_srgb(clamp(color.r, 0.0, 1.0)),
            linear_to_srgb(clamp(color.g, 0.0, 1.0)),
            linear_to_srgb(clamp(color.b, 0.0, 1.0)),
        );
    }
    srgb = srgb * instance.brightness.x;
    let luma = dot(srgb, vec3<f32>(0.299, 0.587, 0.114));
    srgb = luma + (srgb - vec3<f32>(luma)) * instance.saturation.x;
    if instance.grayscale.x > 0.0 {
        let gray = dot(srgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        srgb = mix(srgb, vec3<f32>(gray), instance.grayscale.x);
    }
    srgb = (srgb - vec3<f32>(0.5)) * instance.contrast.x + vec3<f32>(0.5);
    return vec4<f32>(clamp(srgb, vec3<f32>(0.0), vec3<f32>(1.0)), color.a);
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

    col = apply_color_filters(col, instance);
    return vec4<f32>(col.rgb, col.a * instance.params.w * coverage);
}

@fragment
fn fs_solid(in: VertexOutput) -> @location(0) vec4<f32> {
    let instance = instances[in.instance_index];
    let color = apply_color_filters(instance.color, instance);
    return vec4<f32>(color.rgb, color.a * instance.params.w);
}

@fragment
fn fs_image(in: VertexOutput) -> @location(0) vec4<f32> {
    let instance = instances[in.instance_index];
    let bounds = instance.shape_bounds;
    let local = (in.local_position - bounds.xy) / max(bounds.zw, vec2<f32>(0.000001));
    let uv = mix(instance.params.xy, instance.params.zw, clamp(local, vec2<f32>(0.0), vec2<f32>(1.0)));
    let sampled = textureSample(image_texture, image_sampler, uv);
    let color = apply_color_filters(vec4<f32>(sampled.rgb, instance.color.a), instance);
    return vec4<f32>(color.rgb, sampled.a * instance.color.a * instance.params.w);
}

@fragment
fn fs_text(in: VertexOutput) -> @location(0) vec4<f32> {
    let instance = instances[in.instance_index];
    let bounds = instance.shape_bounds;
    let local = (in.local_position - bounds.xy) / max(bounds.zw, vec2<f32>(0.000001));
    let uv = mix(instance.params.xy, instance.params.zw, clamp(local, vec2<f32>(0.0), vec2<f32>(1.0)));
    let coverage = textureSample(image_texture, image_sampler, uv).r;
    let color = apply_color_filters(instance.color, instance);
    return vec4<f32>(color.rgb, coverage * color.a * instance.params.w);
}
"#;

// ────────────────────────────────────────────────────────────────────────────
// GPU State
// ────────────────────────────────────────────────────────────────────────────

/// GPU render context: device, queue, pipeline, and bind group layouts.
struct GpuContext {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    image_pipeline: wgpu::RenderPipeline,
    text_pipeline: wgpu::RenderPipeline,
    globals_layout: wgpu::BindGroupLayout,
    instance_layout: wgpu::BindGroupLayout,
    image_layout: wgpu::BindGroupLayout,
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

        let image_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("image_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pipeline_layout"),
            bind_group_layouts: &[&globals_layout, &instance_layout, &image_layout],
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

        let image_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("dioxuscut_image_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_image",
                targets: &[Some(wgpu::ColorTargetState {
                    format: RENDER_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: SAMPLE_COUNT,
                ..Default::default()
            },
            multiview: None,
            cache: None,
        });

        let text_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("dioxuscut_text_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_text",
                targets: &[Some(wgpu::ColorTargetState {
                    format: RENDER_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: SAMPLE_COUNT,
                ..Default::default()
            },
            multiview: None,
            cache: None,
        });

        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            pipeline,
            mesh_pipeline,
            image_pipeline,
            text_pipeline,
            globals_layout,
            instance_layout,
            image_layout,
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

type GpuResourcePool = Mutex<HashMap<(u32, u32), std::sync::Arc<Mutex<GpuFrameResources>>>>;

struct GpuImageResource {
    _texture: wgpu::Texture,
    _view: wgpu::TextureView,
    _sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

struct GpuImageCacheState {
    images: HashMap<String, Arc<GpuImageResource>>,
    lru: std::collections::VecDeque<String>,
    bytes: usize,
    max_bytes: usize,
}

struct GpuTextAtlasResource {
    _texture: wgpu::Texture,
    _view: wgpu::TextureView,
    _sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
    generation: AtomicU64,
}

impl GpuImageCacheState {
    fn new(max_bytes: usize) -> Self {
        Self {
            images: HashMap::new(),
            lru: std::collections::VecDeque::new(),
            bytes: 0,
            max_bytes: max_bytes.max(1),
        }
    }
}

type GpuImageCache = Mutex<GpuImageCacheState>;

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
    shader_runner: crate::shader::WgpuShaderRunner,
    /// Per-resolution GPU resource pool.  Key = `(width, height)`.
    frame_resources: GpuResourcePool,
    fallback: TinySkiaBackend,
    /// Decoded image ownership for the future GPU texture cache. Keeping this
    /// separate from the CPU backend prevents a GPU render from depending on
    /// fallback implementation details while allowing both paths to share
    /// decoded pixels during the transition.
    image_cache: ImageCache,
    video_cache: VideoFrameCache,
    /// Persistent GPU texture and bind-group cache keyed by the same source
    /// identity as `ImageCache`. Texture uploads therefore happen once per
    /// source, rather than once per rendered frame.
    gpu_images: GpuImageCache,
    text_atlas: Mutex<Option<Arc<GpuTextAtlasResource>>>,
    text_atlas_upload_bytes: AtomicU64,
    gpu_texture_uploads: AtomicU64,
    gpu_texture_upload_bytes: AtomicU64,
    gpu_texture_cache_hits: AtomicU64,
    gpu_texture_cache_misses: AtomicU64,
    video_decode_ns: AtomicU64,
    texture_upload_ns: AtomicU64,
    gpu_submit_readback_ns: AtomicU64,
    gpu_frame_count: AtomicU64,
    cpu_fallback_frame_count: AtomicU64,
}

/// Runtime counters for deciding whether a WGPU render is actually using the
/// GPU path. They are intentionally monotonic and lock-free so instrumentation
/// does not perturb frame scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WgpuRenderStats {
    pub gpu_frames: u64,
    pub cpu_fallback_frames: u64,
    pub texture_cache_hits: u64,
    pub texture_cache_misses: u64,
}

/// Monotonic timing accumulators for the native video texture path.
///
/// `video_decode_ns` covers decoded-frame cache lookup and FFmpeg decode;
/// `texture_upload_ns` covers creation/upload of a cache-miss texture;
/// `gpu_submit_readback_ns` covers command submission through CPU readback
/// after texture preparation. These are cumulative counters, not per-frame
/// averages, and therefore remain meaningful across streaming renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WgpuVideoTimingStats {
    pub video_decode_ns: u64,
    pub texture_upload_ns: u64,
    pub gpu_submit_readback_ns: u64,
}

impl WgpuRenderStats {
    /// Fraction of observed frames that used the CPU fallback.
    pub fn cpu_fallback_ratio(self) -> f64 {
        let total = self.gpu_frames.saturating_add(self.cpu_fallback_frames);
        if total == 0 {
            0.0
        } else {
            self.cpu_fallback_frames as f64 / total as f64
        }
    }
}

impl WgpuBackend {
    /// Create a new GPU backend. Initialises the device and render pipeline.
    pub fn new() -> Result<Self, RasterError> {
        let ctx = GpuContext::new()?;
        let shader_runner = crate::shader::WgpuShaderRunner::from_device(&ctx.device, &ctx.queue);
        Ok(Self {
            ctx,
            shader_runner,
            frame_resources: Mutex::new(HashMap::new()),
            fallback: TinySkiaBackend::new(),
            image_cache: ImageCache::default(),
            video_cache: VideoFrameCache::default(),
            gpu_images: Mutex::new(GpuImageCacheState::new(256 * 1024 * 1024)),
            text_atlas: Mutex::new(None),
            text_atlas_upload_bytes: AtomicU64::new(0),
            gpu_texture_uploads: AtomicU64::new(0),
            gpu_texture_upload_bytes: AtomicU64::new(0),
            gpu_texture_cache_hits: AtomicU64::new(0),
            gpu_texture_cache_misses: AtomicU64::new(0),
            video_decode_ns: AtomicU64::new(0),
            texture_upload_ns: AtomicU64::new(0),
            gpu_submit_readback_ns: AtomicU64::new(0),
            gpu_frame_count: AtomicU64::new(0),
            cpu_fallback_frame_count: AtomicU64::new(0),
        })
    }

    /// Return the number of frames rendered by WGPU and by the CPU fallback.
    pub fn render_stats(&self) -> WgpuRenderStats {
        WgpuRenderStats {
            gpu_frames: self.gpu_frame_count.load(Ordering::Relaxed),
            cpu_fallback_frames: self.cpu_fallback_frame_count.load(Ordering::Relaxed),
            texture_cache_hits: self.gpu_texture_cache_hits.load(Ordering::Relaxed),
            texture_cache_misses: self.gpu_texture_cache_misses.load(Ordering::Relaxed),
        }
    }

    /// Number of decoded image/video textures uploaded to the GPU.
    /// Re-rendering a cached source/frame must not increment this counter.
    pub fn gpu_texture_uploads(&self) -> u64 {
        self.gpu_texture_uploads.load(Ordering::Relaxed)
    }

    /// Number of source bytes uploaded by the GPU texture cache.
    pub fn gpu_texture_upload_bytes(&self) -> u64 {
        self.gpu_texture_upload_bytes.load(Ordering::Relaxed)
    }

    /// Number of GPU texture cache lookups that reused an existing texture.
    pub fn gpu_texture_cache_hits(&self) -> u64 {
        self.gpu_texture_cache_hits.load(Ordering::Relaxed)
    }

    /// Number of GPU texture cache lookups that required a texture upload.
    pub fn gpu_texture_cache_misses(&self) -> u64 {
        self.gpu_texture_cache_misses.load(Ordering::Relaxed)
    }

    /// Return cumulative timings for the native video texture pipeline.
    pub fn video_timing_stats(&self) -> WgpuVideoTimingStats {
        WgpuVideoTimingStats {
            video_decode_ns: self.video_decode_ns.load(Ordering::Relaxed),
            texture_upload_ns: self.texture_upload_ns.load(Ordering::Relaxed),
            gpu_submit_readback_ns: self.gpu_submit_readback_ns.load(Ordering::Relaxed),
        }
    }

    /// Configure the image cache used when a scene falls back to CPU.
    pub fn with_image_cache_bytes(mut self, max_bytes: usize) -> Self {
        self.fallback = self.fallback.with_image_cache_bytes(max_bytes);
        self.image_cache = ImageCache::with_max_bytes(max_bytes);
        self.gpu_images
            .lock()
            .expect("GPU image cache lock poisoned")
            .max_bytes = max_bytes.max(1);
        self
    }

    fn gpu_pixels(
        &self,
        key: &str,
        decoded: &image::RgbaImage,
    ) -> Result<Arc<GpuImageResource>, RasterError> {
        let mut cache = self
            .gpu_images
            .lock()
            .expect("GPU image cache lock poisoned");
        if let Some(image) = cache.images.get(key).cloned() {
            self.gpu_texture_cache_hits.fetch_add(1, Ordering::Relaxed);
            cache.lru.retain(|entry| entry != key);
            cache.lru.push_back(key.to_string());
            return Ok(image);
        }
        drop(cache);
        self.gpu_texture_cache_misses
            .fetch_add(1, Ordering::Relaxed);
        let upload_start = Instant::now();

        let device = &self.ctx.device;
        let queue = &self.ctx.queue;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("image_texture"),
            size: wgpu::Extent3d {
                width: decoded.width(),
                height: decoded.height(),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            decoded.as_raw(),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(decoded.width() * 4),
                rows_per_image: Some(decoded.height()),
            },
            wgpu::Extent3d {
                width: decoded.width(),
                height: decoded.height(),
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image_bg"),
            layout: &self.ctx.image_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let resource = Arc::new(GpuImageResource {
            _texture: texture,
            _view: view,
            _sampler: sampler,
            bind_group,
            width: decoded.width(),
            height: decoded.height(),
        });
        self.texture_upload_ns
            .fetch_add(upload_start.elapsed().as_nanos() as u64, Ordering::Relaxed);
        self.gpu_texture_uploads.fetch_add(1, Ordering::Relaxed);
        self.gpu_texture_upload_bytes.fetch_add(
            u64::from(resource.width) * u64::from(resource.height) * 4,
            Ordering::Relaxed,
        );
        let mut cache = self
            .gpu_images
            .lock()
            .expect("GPU image cache lock poisoned");
        if let Some(existing) = cache.images.get(key).cloned() {
            cache.lru.retain(|entry| entry != key);
            cache.lru.push_back(key.to_string());
            return Ok(existing);
        }
        let bytes = resource.width as usize * resource.height as usize * 4;
        cache.images.insert(key.to_string(), Arc::clone(&resource));
        cache.lru.push_back(key.to_string());
        cache.bytes = cache.bytes.saturating_add(bytes);
        while cache.bytes > cache.max_bytes {
            let Some(oldest) = cache.lru.pop_front() else {
                break;
            };
            if let Some(evicted) = cache.images.remove(&oldest) {
                cache.bytes = cache
                    .bytes
                    .saturating_sub(evicted.width as usize * evicted.height as usize * 4);
            }
        }
        Ok(resource)
    }

    fn gpu_image(&self, src: &str) -> Result<Arc<GpuImageResource>, RasterError> {
        let decoded = self.image_cache.load(src)?;
        self.gpu_pixels(src, &decoded)
    }

    fn gpu_text_atlas(
        &self,
        snapshot: &crate::text_atlas::TextAtlasSnapshot,
    ) -> Arc<GpuTextAtlasResource> {
        let queue = &self.ctx.queue;
        let mut cache = self
            .text_atlas
            .lock()
            .expect("text atlas GPU lock poisoned");
        if let Some(resource) = cache.as_ref() {
            if resource.width == snapshot.width && resource.height == snapshot.height {
                let generation = resource.generation.load(Ordering::Acquire);
                if generation == snapshot.generation {
                    return resource.clone();
                }
                if let Some(rect) = snapshot.dirty {
                    let mut upload = Vec::with_capacity((rect.width * rect.height) as usize);
                    for row in 0..rect.height {
                        let start = ((rect.y + row) * snapshot.width + rect.x) as usize;
                        upload.extend_from_slice(
                            &snapshot.pixels[start..start + rect.width as usize],
                        );
                    }
                    queue.write_texture(
                        wgpu::ImageCopyTexture {
                            texture: &resource._texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d {
                                x: rect.x,
                                y: rect.y,
                                z: 0,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        &upload,
                        wgpu::ImageDataLayout {
                            offset: 0,
                            bytes_per_row: Some(rect.width),
                            rows_per_image: Some(rect.height),
                        },
                        wgpu::Extent3d {
                            width: rect.width,
                            height: rect.height,
                            depth_or_array_layers: 1,
                        },
                    );
                    self.text_atlas_upload_bytes.fetch_add(
                        u64::from(rect.width) * u64::from(rect.height),
                        Ordering::Relaxed,
                    );
                    resource
                        .generation
                        .store(snapshot.generation, Ordering::Release);
                    return resource.clone();
                }
            }
        }
        let device = &self.ctx.device;
        let queue = &self.ctx.queue;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("text_atlas_texture"),
            size: wgpu::Extent3d {
                width: snapshot.width,
                height: snapshot.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &snapshot.pixels,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(snapshot.width),
                rows_per_image: Some(snapshot.height),
            },
            wgpu::Extent3d {
                width: snapshot.width,
                height: snapshot.height,
                depth_or_array_layers: 1,
            },
        );
        self.text_atlas_upload_bytes.fetch_add(
            u64::from(snapshot.width) * u64::from(snapshot.height),
            Ordering::Relaxed,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("text_atlas_bg"),
            layout: &self.ctx.image_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let resource = Arc::new(GpuTextAtlasResource {
            _texture: texture,
            _view: view,
            _sampler: sampler,
            bind_group,
            width: snapshot.width,
            height: snapshot.height,
            generation: AtomicU64::new(snapshot.generation),
        });
        *cache = Some(resource.clone());
        resource
    }

    fn gpu_video(
        &self,
        src: &str,
        time: f64,
        sampling_fps: f64,
        looped: bool,
    ) -> Result<Arc<GpuImageResource>, RasterError> {
        let decode_start = Instant::now();
        let frame = self.video_cache.load(src, time, sampling_fps, looped)?;
        self.video_decode_ns
            .fetch_add(decode_start.elapsed().as_nanos() as u64, Ordering::Relaxed);
        let frame_index = self
            .video_cache
            .frame_index_for(src, time, sampling_fps, looped)?;
        // VideoFrameCache canonicalizes local paths, so the GPU cache must use
        // the same identity. Otherwise `clip.mkv` and `file:///.../clip.mkv`
        // decode to the same frame but upload duplicate GPU textures.
        let canonical = canonical_local_path(src)?;
        let key = format!(
            "video:{}:{sampling_fps:.6}:{frame_index}",
            canonical.display()
        );
        self.gpu_pixels(&key, &frame)
    }

    #[cfg(test)]
    fn gpu_image_cache_len(&self) -> usize {
        self.gpu_images
            .lock()
            .expect("GPU image cache lock poisoned")
            .images
            .len()
    }

    #[cfg(test)]
    fn text_atlas_cache_generation(&self) -> Option<u64> {
        self.text_atlas
            .lock()
            .expect("text atlas GPU lock poisoned")
            .as_ref()
            .map(|resource| resource.generation.load(Ordering::Acquire))
    }

    /// Number of R8 atlas bytes uploaded since backend creation.
    pub fn text_atlas_upload_bytes(&self) -> u64 {
        self.text_atlas_upload_bytes.load(Ordering::Relaxed)
    }

    fn submit_frame_to_slot(
        &self,
        commands: &[DrawCommand],
        width: u32,
        height: u32,
        sampling_fps: f64,
        slot: &GpuFrameSlot,
        shader_suffix: Option<&[SceneNode]>,
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

        let mut all_instances: Vec<GpuInstance> = commands.iter().map(|c| *c.instance()).collect();
        let mut image_resources = Vec::with_capacity(commands.len());
        let atlas_snapshot = self.fallback.text_atlas_snapshot();
        let atlas_resource = self.gpu_text_atlas(&atlas_snapshot);
        for (index, command) in commands.iter().enumerate() {
            let (source, fit) = match command {
                DrawCommand::Image { src, fit, .. } => (self.gpu_image(src)?, *fit),
                DrawCommand::Video {
                    src,
                    time,
                    looped,
                    fit,
                    ..
                } => (self.gpu_video(src, *time, sampling_fps, *looped)?, *fit),
                DrawCommand::Text { entry, .. } => {
                    all_instances[index].params[0] = entry.x as f32 / atlas_snapshot.width as f32;
                    all_instances[index].params[1] = entry.y as f32 / atlas_snapshot.height as f32;
                    all_instances[index].params[2] =
                        (entry.x + entry.width) as f32 / atlas_snapshot.width as f32;
                    all_instances[index].params[3] =
                        (entry.y + entry.height) as f32 / atlas_snapshot.height as f32;
                    image_resources.push(None);
                    continue;
                }
                _ => {
                    image_resources.push(None);
                    continue;
                }
            };
            let placement = image_placement(
                fit,
                source.width as f32,
                source.height as f32,
                all_instances[index].shape_bounds[0],
                all_instances[index].shape_bounds[1],
                all_instances[index].shape_bounds[2],
                all_instances[index].shape_bounds[3],
            )
            .ok_or_else(|| RasterError::ImageAsset {
                path: "video/image".into(),
                reason: "invalid image placement dimensions".into(),
            })?;
            all_instances[index].bounds = placement.destination;
            all_instances[index].shape_bounds = placement.destination;
            all_instances[index].params[0..4].copy_from_slice(&placement.source_uv);

            image_resources.push(Some(source));
        }

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

            // The shared pipeline layout requires group 2 even for analytic
            // and mesh draws. A 1x1 dummy texture keeps those pipelines valid;
            // image draws replace it with their per-command texture group.
            let dummy_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("dummy_image_texture"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                dummy_texture.as_image_copy(),
                &[0, 0, 0, 0],
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            let dummy_view = dummy_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let dummy_sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
            let dummy_image_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dummy_image_bg"),
                layout: &self.ctx.image_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&dummy_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&dummy_sampler),
                    },
                ],
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
                    DrawCommand::Analytic { .. }
                    | DrawCommand::Image { .. }
                    | DrawCommand::Video { .. }
                    | DrawCommand::Text { .. } => None,
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
            pass.set_bind_group(2, &dummy_image_bg, &[]);

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
                    DrawCommand::Image { .. }
                    | DrawCommand::Video { .. }
                    | DrawCommand::Text { .. } => {
                        if matches!(commands[i], DrawCommand::Text { .. }) {
                            pass.set_pipeline(&self.ctx.text_pipeline);
                            pass.set_bind_group(2, &atlas_resource.bind_group, &[]);
                        } else {
                            pass.set_pipeline(&self.ctx.image_pipeline);
                            pass.set_bind_group(
                                2,
                                &image_resources[i]
                                    .as_ref()
                                    .expect("image resource")
                                    .bind_group,
                                &[],
                            );
                        }
                        pass.draw(0..6, i as u32..i as u32 + 1);
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

        if let Some(shader_nodes) = shader_suffix {
            for node in shader_nodes {
                let SceneNode::Shader {
                    x,
                    y,
                    w,
                    h,
                    source,
                    time,
                    params,
                    opacity,
                } = node
                else {
                    return Err(RasterError::Scene(
                        "GPU shader suffix contains a non-shader node".into(),
                    ));
                };
                if ![*x, *y, *w, *h, *time, *opacity]
                    .iter()
                    .all(|value| value.is_finite())
                    || !(*opacity >= 0.0 && *opacity <= 1.0)
                    || *x < 0.0
                    || *y < 0.0
                    || *w <= 0.0
                    || *h <= 0.0
                    || *x + *w > width as f32
                    || *y + *h > height as f32
                {
                    return Err(RasterError::Scene(
                        "invalid GPU shader suffix region".into(),
                    ));
                }
                let shader_source = shader_source_with_opacity(source, *opacity);
                self.shader_runner.render_into_region(
                    &mut encoder,
                    &slot.texture_view,
                    width,
                    height,
                    *x,
                    *y,
                    *w,
                    *h,
                    *time,
                    *params,
                    &shader_source,
                    wgpu::LoadOp::Load,
                )?;
            }
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
        if !scene.nodes.is_empty()
            && scene
                .nodes
                .iter()
                .all(|node| matches!(node, SceneNode::Shader { .. }))
        {
            let image = if scene.nodes.iter().all(|node| {
                matches!(node, SceneNode::Shader { x, y, w, h, source, opacity, .. }
                        if *opacity >= 0.0
                            && *opacity <= 1.0
                            && shader_opacity_supported(source)
                            && *x >= 0.0
                            && *y >= 0.0
                            && *w > 0.0
                            && *h > 0.0
                            && *x + *w <= config.width as f32
                            && *y + *h <= config.height as f32)
            }) {
                self.render_shader_layers_direct(scene, config)?
            } else {
                self.render_shader_layers(scene, config)?
            };
            self.gpu_frame_count.fetch_add(1, Ordering::Relaxed);
            return Ok(image);
        }
        if scene
            .nodes
            .iter()
            .any(|node| matches!(node, SceneNode::Shader { .. }))
        {
            let first_shader = scene
                .nodes
                .iter()
                .position(|node| matches!(node, SceneNode::Shader { .. }))
                .unwrap();
            let trailing_shader_only = scene.nodes[first_shader..]
                .iter()
                .all(|node| matches!(node, SceneNode::Shader { .. }));
            if !trailing_shader_only {
                let image = self.render_interleaved_shader_scene(scene, config)?;
                return Ok(image);
            }
        }
        let mut gpu_base_scene = None;
        let shader_suffix = scene
            .nodes
            .iter()
            .position(|node| matches!(node, SceneNode::Shader { .. }))
            .filter(|start| *start > 0)
            .and_then(|start| {
                let suffix = &scene.nodes[start..];
                let valid = suffix.iter().all(|node| {
                    matches!(node, SceneNode::Shader { x, y, w, h, source, opacity, .. }
                        if *opacity >= 0.0
                            && *opacity <= 1.0
                            && shader_opacity_supported(source)
                            && *x >= 0.0
                            && *y >= 0.0
                            && *w > 0.0
                            && *h > 0.0
                            && *x + *w <= config.width as f32
                            && *y + *h <= config.height as f32)
                });
                if valid {
                    gpu_base_scene = Some(Scene {
                        nodes: scene.nodes[..start].to_vec(),
                    });
                    Some(suffix)
                } else {
                    None
                }
            });
        let gpu_scene = gpu_base_scene.as_ref().unwrap_or(scene);
        let Some(commands) = compile_scene(gpu_scene, &self.fallback) else {
            self.cpu_fallback_frame_count
                .fetch_add(1, Ordering::Relaxed);
            return self.fallback.render_frame(scene, config);
        };

        if config.width > self.ctx.max_texture_dimension_2d
            || config.height > self.ctx.max_texture_dimension_2d
        {
            self.cpu_fallback_frame_count
                .fetch_add(1, Ordering::Relaxed);
            return self.fallback.render_frame(scene, config);
        }

        let width = config.width;
        let height = config.height;

        let resource = {
            let mut pool = self
                .frame_resources
                .lock()
                .map_err(|_| RasterError::Init("GPU resource pool mutex poisoned".into()))?;
            pool.entry((width, height))
                .or_insert_with(|| {
                    std::sync::Arc::new(Mutex::new(GpuFrameResources::new(
                        &self.ctx.device,
                        width,
                        height,
                    )))
                })
                .clone()
        };
        let mut res = resource
            .lock()
            .map_err(|_| RasterError::Init("GPU frame resource mutex poisoned".into()))?;

        let slot_idx = res.active_index % RING_BUFFER_SIZE;
        res.active_index = res.active_index.wrapping_add(1);

        let (submission_index, rx) = self.submit_frame_to_slot(
            &commands,
            width,
            height,
            config.fps,
            &res.slots[slot_idx],
            shader_suffix,
        )?;
        // Texture preparation (including video decode and cache-miss upload)
        // is measured separately. This interval is GPU command submission,
        // synchronization, and CPU readback only.
        let gpu_submit_start = Instant::now();

        let mut out_pixels = None;
        let mut scratch = Vec::new();
        self.drain_slot(
            InFlight {
                frame_idx: config.frame,
                slot_idx,
                submission_index,
                rx,
            },
            &res,
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

        let image = RgbaImage::from_raw(width, height, pixels).ok_or_else(|| {
            RasterError::ImageEncode("Failed to assemble RgbaImage from GPU readback".into())
        })?;
        self.gpu_submit_readback_ns.fetch_add(
            gpu_submit_start.elapsed().as_nanos() as u64,
            Ordering::Relaxed,
        );
        self.gpu_frame_count.fetch_add(1, Ordering::Relaxed);
        Ok(image)
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

        let resource = {
            let mut pool = self
                .frame_resources
                .lock()
                .map_err(|_| RasterError::Init("GPU resource pool mutex poisoned".into()))?;
            pool.entry((width, height))
                .or_insert_with(|| {
                    std::sync::Arc::new(Mutex::new(GpuFrameResources::new(
                        &self.ctx.device,
                        width,
                        height,
                    )))
                })
                .clone()
        };
        let res = resource
            .lock()
            .map_err(|_| RasterError::Init("GPU frame resource mutex poisoned".into()))?;

        let mut in_flight: Option<InFlight> = None;
        let mut scratch = Vec::new();

        for frame in 0..total {
            let scene = scene_fn(frame)?;
            let cfg = config_fn(frame);

            let Some(commands) = compile_scene(&scene, &self.fallback) else {
                if let Some(prev) = in_flight.take() {
                    self.drain_slot(prev, &res, width, height, &mut scratch, consume_fn)?;
                }
                self.cpu_fallback_frame_count
                    .fetch_add(1, Ordering::Relaxed);
                let img = self.fallback.render_frame(&scene, &cfg)?;
                consume_fn(frame, img.as_raw())?;
                continue;
            };

            let slot_idx = (frame as usize) % RING_BUFFER_SIZE;

            // If the target slot is currently occupied by an in-flight frame, drain it now
            if let Some(prev) = in_flight.take() {
                if prev.slot_idx == slot_idx {
                    self.drain_slot(prev, &res, width, height, &mut scratch, consume_fn)?;
                } else {
                    in_flight = Some(prev);
                }
            }

            // Submit this frame to GPU
            let (submission_index, rx) = self.submit_frame_to_slot(
                &commands,
                width,
                height,
                cfg.fps,
                &res.slots[slot_idx],
                None,
            )?;
            self.gpu_frame_count.fetch_add(1, Ordering::Relaxed);

            // Overlap: drain the previous frame while the newly submitted frame is being rendered on GPU
            if let Some(prev) = in_flight.take() {
                self.drain_slot(prev, &res, width, height, &mut scratch, consume_fn)?;
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
            self.drain_slot(prev, &res, width, height, &mut scratch, consume_fn)?;
        }

        Ok(())
    }
}

impl WgpuBackend {
    fn render_interleaved_shader_scene(
        &self,
        scene: &Scene,
        config: &FrameConfig,
    ) -> Result<RgbaImage, RasterError> {
        let mut output = RgbaImage::new(config.width, config.height);
        let mut regular_nodes = Vec::new();
        for node in &scene.nodes {
            if matches!(node, SceneNode::Shader { .. }) {
                if !regular_nodes.is_empty() {
                    let segment = Scene {
                        nodes: std::mem::take(&mut regular_nodes),
                    };
                    let rendered = self.render_frame(&segment, config)?;
                    image::imageops::overlay(&mut output, &rendered, 0, 0);
                }
                let shader_scene = Scene {
                    nodes: vec![node.clone()],
                };
                let rendered = match node {
                    SceneNode::Shader {
                        x,
                        y,
                        w,
                        h,
                        source,
                        opacity,
                        ..
                    } if *opacity >= 0.0
                        && *opacity <= 1.0
                        && *x >= 0.0
                        && *y >= 0.0
                        && *w > 0.0
                        && *h > 0.0
                        && *x + *w <= config.width as f32
                        && *y + *h <= config.height as f32
                        && shader_opacity_supported(source) =>
                    {
                        self.render_shader_layers_direct(&shader_scene, config)?
                    }
                    _ => self.render_shader_layers(&shader_scene, config)?,
                };
                image::imageops::overlay(&mut output, &rendered, 0, 0);
            } else {
                regular_nodes.push(node.clone());
            }
        }
        if !regular_nodes.is_empty() {
            let rendered = self.render_frame(
                &Scene {
                    nodes: regular_nodes,
                },
                config,
            )?;
            image::imageops::overlay(&mut output, &rendered, 0, 0);
        }
        Ok(output)
    }

    /// Render opaque shader layers directly into one GPU target and read it
    /// back once. Opacity layers retain the compatibility path until opacity
    /// is carried as a target-pass uniform.
    fn render_shader_layers_direct(
        &self,
        scene: &Scene,
        config: &FrameConfig,
    ) -> Result<RgbaImage, RasterError> {
        let width = config.width;
        let height = config.height;
        let texture = self.ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shader_scene_target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shader_scene_encoder"),
            });
        for (index, node) in scene.nodes.iter().enumerate() {
            let SceneNode::Shader {
                x,
                y,
                w,
                h,
                source,
                time,
                params,
                opacity,
            } = node
            else {
                unreachable!("direct shader path was checked before rendering");
            };
            if ![*x, *y, *w, *h, *time, *opacity]
                .iter()
                .all(|value| value.is_finite())
                || !(*opacity >= 0.0 && *opacity <= 1.0)
                || *w <= 0.0
                || *h <= 0.0
                || *x < 0.0
                || *y < 0.0
            {
                return Err(RasterError::Scene("invalid direct shader region".into()));
            }
            let shader_source = shader_source_with_opacity(source, *opacity);
            self.shader_runner.render_into_region(
                &mut encoder,
                &view,
                width,
                height,
                *x,
                *y,
                *w,
                *h,
                *time,
                *params,
                &shader_source,
                if index == 0 {
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                } else {
                    wgpu::LoadOp::Load
                },
            )?;
        }
        let bytes_per_row = (width * 4 + 255) & !255;
        let staging = self.ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shader_scene_readback"),
            size: (bytes_per_row * height) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &staging,
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
        self.ctx.queue.submit(Some(encoder.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.ctx.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|error| RasterError::Scene(format!("GPU readback channel failed: {error}")))?
            .map_err(|error| RasterError::Scene(format!("GPU readback map failed: {error}")))?;
        let mapped = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let start = (row * bytes_per_row) as usize;
            pixels.extend_from_slice(&mapped[start..start + (width * 4) as usize]);
        }
        drop(mapped);
        staging.unmap();
        RgbaImage::from_raw(width, height, pixels)
            .ok_or_else(|| RasterError::ImageEncode("Invalid shader scene readback".into()))
    }

    /// Render shader nodes on the GPU and composite their rectangular outputs
    /// in scene order. Non-shader nodes deliberately remain outside this path
    /// until an offscreen GPU texture is available for general scene blending.
    fn render_shader_layers(
        &self,
        scene: &Scene,
        config: &FrameConfig,
    ) -> Result<RgbaImage, RasterError> {
        let mut output = RgbaImage::new(config.width, config.height);
        self.composite_shader_layers(&mut output, &scene.nodes)?;
        Ok(output)
    }

    fn composite_shader_layers(
        &self,
        output: &mut RgbaImage,
        nodes: &[SceneNode],
    ) -> Result<(), RasterError> {
        for node in nodes {
            let SceneNode::Shader {
                x,
                y,
                w,
                h,
                source,
                time,
                params,
                opacity,
            } = node
            else {
                unreachable!("shader layer path was checked before rendering");
            };
            if ![*x, *y, *w, *h, *opacity]
                .iter()
                .all(|value| value.is_finite())
                || *w <= 0.0
                || *h <= 0.0
                || !(0.0..=1.0).contains(opacity)
            {
                return Err(RasterError::Scene(
                    "invalid shader layer bounds or opacity".into(),
                ));
            }
            let width = (*w).round().max(1.0) as u32;
            let height = (*h).round().max(1.0) as u32;
            let mut layer = self
                .shader_runner
                .render(width, height, *time, *params, source)?;
            if *opacity < 1.0 {
                for pixel in layer.pixels_mut() {
                    pixel[3] = (f32::from(pixel[3]) * *opacity).round() as u8;
                }
            }
            image::imageops::overlay(output, &layer, (*x).round() as i64, (*y).round() as i64);
        }
        Ok(())
    }
}

fn shader_opacity_supported(source: &str) -> bool {
    !source.contains("@fragment") && source.contains("return ")
}

fn shader_source_with_opacity(source: &str, opacity: f32) -> String {
    if opacity >= 1.0 || !shader_opacity_supported(source) {
        return source.to_owned();
    }
    let Some(return_start) = source.rfind("return ") else {
        return source.to_owned();
    };
    let expression_start = return_start + "return ".len();
    let Some(semicolon_offset) = source[expression_start..].find(';') else {
        return source.to_owned();
    };
    let semicolon = expression_start + semicolon_offset;
    let expression = &source[expression_start..semicolon];
    format!(
        "{}let dioxuscut_color = {}; return vec4<f32>(dioxuscut_color.rgb, dioxuscut_color.a * {:.9});{}",
        &source[..return_start],
        expression,
        opacity,
        &source[semicolon + 1..]
    )
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
    brightness: [f32; 4],
    grayscale: [f32; 4],
    contrast: [f32; 4],
    saturation: [f32; 4],
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
            brightness: [1.0, 0.0, 0.0, 0.0],
            grayscale: [0.0, 0.0, 0.0, 0.0],
            contrast: [1.0, 0.0, 0.0, 0.0],
            saturation: [1.0, 0.0, 0.0, 0.0],
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
    Image {
        instance: GpuInstance,
        src: String,
        fit: ImageFit,
    },
    Video {
        instance: GpuInstance,
        src: String,
        time: f64,
        looped: bool,
        fit: ImageFit,
    },
    Text {
        instance: GpuInstance,
        entry: crate::text_atlas::AtlasEntry,
    },
}

impl DrawCommand {
    fn instance(&self) -> &GpuInstance {
        match self {
            Self::Analytic { instance }
            | Self::Mesh { instance, .. }
            | Self::Image { instance, .. }
            | Self::Video { instance, .. }
            | Self::Text { instance, .. } => instance,
        }
    }

    fn instance_mut(&mut self) -> &mut GpuInstance {
        match self {
            Self::Analytic { instance }
            | Self::Mesh { instance, .. }
            | Self::Image { instance, .. }
            | Self::Video { instance, .. }
            | Self::Text { instance, .. } => instance,
        }
    }
}

fn compile_scene(scene: &Scene, font: &TinySkiaBackend) -> Option<Vec<DrawCommand>> {
    let mut commands = Vec::new();
    compile_nodes(
        &scene.nodes,
        Transform::identity(),
        1.0,
        &mut commands,
        font,
    )?;
    Some(commands)
}

fn compile_nodes(
    nodes: &[SceneNode],
    transform: Transform,
    opacity: f32,
    output: &mut Vec<DrawCommand>,
    font: &TinySkiaBackend,
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

            SceneNode::Image {
                src,
                x,
                y,
                w,
                h,
                fit,
                opacity: node_opacity,
            } => {
                if ![*x, *y, *w, *h, *node_opacity]
                    .iter()
                    .all(|value| value.is_finite())
                    || *w <= 0.0
                    || *h <= 0.0
                    || *node_opacity < 0.0
                {
                    return None;
                }
                let mut instance =
                    GpuInstance::solid(Color::WHITE, opacity * *node_opacity, transform);
                instance.kind_data[0] = 5;
                instance.bounds = [*x, *y, *w, *h];
                instance.shape_bounds = instance.bounds;
                instance.params = [0.0, 0.0, 1.0, 1.0];
                output.push(DrawCommand::Image {
                    instance,
                    src: src.clone(),
                    fit: *fit,
                });
            }

            SceneNode::Video {
                src,
                time,
                looped,
                x,
                y,
                w,
                h,
                fit,
                opacity: node_opacity,
            } => {
                if ![*x, *y, *w, *h, *node_opacity]
                    .iter()
                    .all(|value| value.is_finite())
                    || *w <= 0.0
                    || *h <= 0.0
                    || !time.is_finite()
                    || *time < 0.0
                    || *node_opacity < 0.0
                {
                    return None;
                }
                let mut instance =
                    GpuInstance::solid(Color::WHITE, opacity * *node_opacity, transform);
                instance.kind_data[0] = 5;
                instance.bounds = [*x, *y, *w, *h];
                instance.shape_bounds = instance.bounds;
                instance.params = [0.0, 0.0, 1.0, 1.0];
                output.push(DrawCommand::Video {
                    instance,
                    src: src.clone(),
                    time: *time,
                    looped: *looped,
                    fit: *fit,
                });
            }

            SceneNode::Text {
                x,
                y,
                content,
                font_size,
                color,
                font_sources,
                ..
            } => {
                if ![*x, *y, *font_size].iter().all(|v| v.is_finite()) || *font_size <= 0.0 {
                    return None;
                }
                let rendered = font.rasterize_text(content, *font_size, font_sources)?;
                let mut instance = GpuInstance::solid(*color, opacity, transform);
                instance.kind_data[0] = 6;
                instance.bounds = [
                    *x,
                    *y - rendered.baseline as f32,
                    rendered.width as f32,
                    rendered.height as f32,
                ];
                instance.shape_bounds = instance.bounds;
                let key = format!("{}:{}:{:?}", content, font_size.to_bits(), font_sources);
                let entry = font.text_atlas_entry(&key)?;
                output.push(DrawCommand::Text { instance, entry });
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
                compile_nodes(
                    children,
                    next_transform,
                    opacity * group_opacity,
                    output,
                    font,
                )?;
            }

            // A layer with no offscreen-only effect is semantically just a
            // group of GPU instances. Keep opacity and brightness filters on
            // the GPU while preserving child order; complex layers still take
            // the fallback path below so their compositing semantics remain exact.
            SceneNode::Layer {
                opacity: layer_opacity,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: None,
                filters,
                shadow: None,
                children,
                ..
            } if gpu_layer_effects(filters, *layer_opacity).is_some() => {
                let (layer_opacity, brightness, grayscale, contrast, saturation) =
                    gpu_layer_effects(filters, *layer_opacity).unwrap();
                let start = output.len();
                compile_nodes(children, transform, opacity * layer_opacity, output, font)?;
                for command in &mut output[start..] {
                    let instance = command.instance_mut();
                    instance.brightness[0] *= brightness;
                    instance.grayscale[0] = 1.0 - (1.0 - instance.grayscale[0]) * (1.0 - grayscale);
                    instance.contrast[0] *= contrast;
                    instance.saturation[0] *= saturation;
                }
            }

            SceneNode::Audio { .. } => {}
            SceneNode::Gif { .. }
            | SceneNode::Layer { .. }
            | SceneNode::Emoji { .. }
            | SceneNode::Lottie { .. }
            | SceneNode::AudioVisualizer { .. }
            | SceneNode::Shader { .. } => return None,
        }
    }
    Some(())
}

fn gpu_layer_effects(
    filters: &[crate::scene::SceneFilter],
    layer_opacity: f32,
) -> Option<(f32, f32, f32, f32, f32)> {
    if !layer_opacity.is_finite() {
        return None;
    }
    filters.iter().try_fold(
        (layer_opacity, 1.0, 0.0, 1.0, 1.0),
        |(opacity, brightness, grayscale, contrast, saturation), filter| match filter {
            crate::scene::SceneFilter::Opacity { amount }
                if amount.is_finite() && (0.0..=1.0).contains(amount) =>
            {
                Some((
                    opacity * amount,
                    brightness,
                    grayscale,
                    contrast,
                    saturation,
                ))
            }
            crate::scene::SceneFilter::Brightness { amount }
                if amount.is_finite() && (0.0..=10.0).contains(amount) =>
            {
                Some((
                    opacity,
                    brightness * amount,
                    grayscale,
                    contrast,
                    saturation,
                ))
            }
            crate::scene::SceneFilter::Grayscale { amount }
                if amount.is_finite() && (0.0..=1.0).contains(amount) =>
            {
                Some((
                    opacity,
                    brightness,
                    grayscale + amount - grayscale * amount,
                    contrast,
                    saturation,
                ))
            }
            crate::scene::SceneFilter::Contrast { factor }
                if factor.is_finite() && *factor >= 0.0 =>
            {
                Some((
                    opacity,
                    brightness,
                    grayscale,
                    contrast * factor,
                    saturation,
                ))
            }
            crate::scene::SceneFilter::Saturation { factor }
                if factor.is_finite() && *factor >= 0.0 =>
            {
                Some((
                    opacity,
                    brightness,
                    grayscale,
                    contrast,
                    saturation * factor,
                ))
            }
            _ => None,
        },
    )
}

#[cfg(test)]
fn gpu_supports_scene(scene: &Scene) -> bool {
    if !scene.nodes.is_empty()
        && scene.nodes.iter().all(|node| {
            matches!(
                node,
                SceneNode::Shader {
                    x,
                    y,
                    w,
                    h,
                    opacity,
                    ..
                } if [*x, *y, *w, *h, *opacity]
                    .iter()
                    .all(|value| value.is_finite())
                    && *w > 0.0
                    && *h > 0.0
                    && (0.0..=1.0).contains(opacity)
            )
        })
    {
        return true;
    }
    compile_scene(scene, &TinySkiaBackend::new()).is_some()
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
    fn image_fit_geometry_matches_expected_crop_and_letterbox() {
        let contain =
            image_placement(ImageFit::Contain, 200.0, 100.0, 10.0, 20.0, 100.0, 100.0).unwrap();
        assert_eq!(contain.destination, [10.0, 45.0, 100.0, 50.0]);
        assert_eq!(contain.source_uv, [0.0, 0.0, 1.0, 1.0]);

        let cover = image_placement(ImageFit::Cover, 200.0, 100.0, 0.0, 0.0, 100.0, 100.0).unwrap();
        assert_eq!(cover.destination, [0.0, 0.0, 100.0, 100.0]);
        assert_eq!(cover.source_uv, [0.25, 0.0, 0.75, 1.0]);

        let fill = image_placement(ImageFit::Fill, 200.0, 100.0, 1.0, 2.0, 30.0, 40.0).unwrap();
        assert_eq!(fill.destination, [1.0, 2.0, 30.0, 40.0]);
        assert_eq!(fill.source_uv, [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn image_fit_none_and_scale_down_never_upscale() {
        let none = image_placement(ImageFit::None, 200.0, 100.0, 0.0, 0.0, 80.0, 80.0).unwrap();
        assert_eq!(none.destination, [0.0, 0.0, 80.0, 80.0]);
        for (actual, expected) in none.source_uv.into_iter().zip([0.3, 0.1, 0.7, 0.9]) {
            assert!((actual - expected).abs() < 1e-6);
        }

        let scale_down =
            image_placement(ImageFit::ScaleDown, 20.0, 10.0, 0.0, 0.0, 100.0, 100.0).unwrap();
        assert_eq!(scale_down.destination, [40.0, 45.0, 20.0, 10.0]);
    }

    #[test]
    fn unsupported_nodes_trigger_cpu_fallback() {
        let mut scene = Scene::new();
        scene.push(SceneNode::Shader {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
            source: "return vec4<f32>(1.0);".into(),
            time: 0.0,
            params: [0.0; 4],
            opacity: 1.0,
        });
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
        assert!(gpu_supports_scene(&image_scene));

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
        let commands = compile_scene(&scene, &TinySkiaBackend::headless()).unwrap();
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
            (compile_scene(&scene, &TinySkiaBackend::headless()).unwrap()[0]
                .instance()
                .params[3]
                - 0.25)
                .abs()
                < f32::EPSILON
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
    fn render_stats_distinguish_gpu_and_cpu_fallback_frames() {
        let Ok(backend) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping render stats test");
            return;
        };
        let config = FrameConfig::new(64, 64, 0, 30.0);
        let gpu_scene = Scene {
            nodes: vec![SceneNode::Rect {
                x: 0.0,
                y: 0.0,
                w: 64.0,
                h: 64.0,
                fill: Color::WHITE,
                stroke: None,
                stroke_width: 0.0,
                corner_radius: 0.0,
            }],
        };
        // Keep the asset inline so the texture upload test is hermetic.
        let fallback_scene = Scene {
            nodes: vec![SceneNode::Image {
                src: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".into(),
                x: 0.0,
                y: 0.0,
                w: 64.0,
                h: 64.0,
                fit: crate::scene::ImageFit::Fill,
                opacity: 1.0,
            }],
        };
        backend.render_frame(&gpu_scene, &config).unwrap();
        backend.render_frame(&fallback_scene, &config).unwrap();
        assert_eq!(
            backend.render_stats(),
            WgpuRenderStats {
                gpu_frames: 2,
                cpu_fallback_frames: 0,
                texture_cache_hits: 0,
                texture_cache_misses: 1,
            }
        );
        assert!(backend.render_stats().cpu_fallback_ratio().abs() < f64::EPSILON);
    }

    #[test]
    fn gpu_image_matches_cpu_for_fill_and_preserves_draw_order() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping image parity test");
            return;
        };
        let scene = Scene {
            nodes: vec![
                SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 32.0,
                    h: 32.0,
                    fill: Color::rgb(0, 0, 255),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                },
                SceneNode::Image {
                    src: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".into(),
                    x: 8.0,
                    y: 8.0,
                    w: 16.0,
                    h: 16.0,
                    fit: ImageFit::Fill,
                    opacity: 1.0,
                },
            ],
        };
        let config = FrameConfig::new(32, 32, 0, 30.0);
        let gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let _second_gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let cpu_image = TinySkiaBackend::headless()
            .render_frame(&scene, &config)
            .unwrap();
        assert_eq!(gpu.gpu_image_cache_len(), 1);
        assert_eq!(gpu.gpu_texture_uploads(), 1);
        assert_eq!(gpu.gpu_texture_upload_bytes(), 4);
        assert!(gpu_image.get_pixel(16, 16)[3] > 0);
        assert_eq!(gpu_image.get_pixel(2, 2), cpu_image.get_pixel(2, 2));
        let mean_error = gpu_image
            .pixels()
            .zip(cpu_image.pixels())
            .map(|(gpu, cpu)| {
                (0..4)
                    .map(|channel| {
                        (i16::from(gpu[channel]) - i16::from(cpu[channel])).unsigned_abs() as u64
                    })
                    .sum::<u64>()
            })
            .sum::<u64>() as f64
            / (config.width * config.height * 4) as f64;
        assert!(
            mean_error < 12.0,
            "GPU/CPU image mean error was {mean_error}"
        );
    }

    #[test]
    fn gpu_image_fits_match_cpu_for_all_modes() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping image fit parity test");
            return;
        };
        let path =
            std::env::temp_dir().join(format!("dioxuscut-image-fit-{}.png", std::process::id()));
        let mut source = image::RgbaImage::new(2, 1);
        source.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        source.put_pixel(1, 0, image::Rgba([0, 0, 255, 255]));
        source.save(&path).unwrap();
        let fits = [
            ImageFit::Cover,
            ImageFit::Contain,
            ImageFit::Fill,
            ImageFit::None,
            ImageFit::ScaleDown,
        ];
        let config = FrameConfig::new(64, 64, 0, 30.0);
        for fit in fits {
            let scene = Scene {
                nodes: vec![SceneNode::Image {
                    src: path.to_string_lossy().into_owned(),
                    x: 8.0,
                    y: 8.0,
                    w: 48.0,
                    h: 48.0,
                    fit,
                    opacity: 1.0,
                }],
            };
            let gpu_image = gpu.render_frame(&scene, &config).unwrap();
            let cpu_image = TinySkiaBackend::new()
                .render_frame(&scene, &config)
                .unwrap();
            let mean_error: f64 = gpu_image
                .pixels()
                .zip(cpu_image.pixels())
                .map(|(a, b)| {
                    (0..4)
                        .map(|channel| {
                            (i16::from(a[channel]) - i16::from(b[channel])).unsigned_abs() as u64
                        })
                        .sum::<u64>()
                })
                .sum::<u64>() as f64
                / (64 * 64 * 4) as f64;
            assert!(
                mean_error < 18.0,
                "GPU/CPU {fit:?} mean error was {mean_error}"
            );
        }
        // `none` must keep the natural image size even when it is larger than
        // the destination box. This exercises target clipping and catches a
        // top-left crop masquerading as a centered natural-size draw.
        let oversized_path = std::env::temp_dir().join(format!(
            "dioxuscut-image-fit-oversized-{}.png",
            std::process::id()
        ));
        let mut oversized = image::RgbaImage::new(80, 40);
        for (x, y, pixel) in oversized.enumerate_pixels_mut() {
            *pixel = if x < 40 && y < 20 {
                image::Rgba([255, 0, 0, 255])
            } else if x >= 40 && y < 20 {
                image::Rgba([0, 255, 0, 255])
            } else if x < 40 {
                image::Rgba([0, 0, 255, 255])
            } else {
                image::Rgba([255, 255, 0, 255])
            };
        }
        oversized.save(&oversized_path).unwrap();
        let oversized_scene = Scene {
            nodes: vec![SceneNode::Image {
                src: oversized_path.to_string_lossy().into_owned(),
                x: 8.0,
                y: 8.0,
                w: 48.0,
                h: 24.0,
                fit: ImageFit::None,
                opacity: 1.0,
            }],
        };
        let gpu_image = gpu.render_frame(&oversized_scene, &config).unwrap();
        let cpu_image = TinySkiaBackend::new()
            .render_frame(&oversized_scene, &config)
            .unwrap();
        let mean_error: f64 = gpu_image
            .pixels()
            .zip(cpu_image.pixels())
            .map(|(a, b)| {
                (0..4)
                    .map(|channel| {
                        (i16::from(a[channel]) - i16::from(b[channel])).unsigned_abs() as u64
                    })
                    .sum::<u64>()
            })
            .sum::<u64>() as f64
            / (64 * 64 * 4) as f64;
        assert!(
            mean_error < 18.0,
            "GPU/CPU oversized none error was {mean_error}"
        );
        let _ = std::fs::remove_file(oversized_path);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn gpu_image_transform_and_opacity_match_cpu() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping transformed image parity test");
            return;
        };
        let path = std::env::temp_dir().join(format!(
            "dioxuscut-image-transform-{}.png",
            std::process::id()
        ));
        let mut source = image::RgbaImage::new(2, 2);
        for pixel in source.pixels_mut() {
            *pixel = image::Rgba([255, 32, 64, 255]);
        }
        source.save(&path).unwrap();
        let scene = Scene {
            nodes: vec![
                SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 64.0,
                    h: 64.0,
                    fill: Color::rgb(10, 20, 30),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                },
                SceneNode::Group {
                    transform: crate::scene::Transform2D::rotate(17.0).with_translate(12.0, 8.0),
                    opacity: 0.55,
                    children: vec![SceneNode::Image {
                        src: path.to_string_lossy().into_owned(),
                        x: 12.0,
                        y: 12.0,
                        w: 32.0,
                        h: 24.0,
                        fit: ImageFit::Contain,
                        opacity: 0.8,
                    }],
                },
            ],
        };
        let config = FrameConfig::new(64, 64, 0, 30.0);
        let gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let cpu_image = TinySkiaBackend::new()
            .render_frame(&scene, &config)
            .unwrap();
        let mean_error: f64 = gpu_image
            .pixels()
            .zip(cpu_image.pixels())
            .map(|(a, b)| {
                (0..4)
                    .map(|channel| {
                        (i16::from(a[channel]) - i16::from(b[channel])).unsigned_abs() as u64
                    })
                    .sum::<u64>()
            })
            .sum::<u64>() as f64
            / (64 * 64 * 4) as f64;
        assert!(
            mean_error < 18.0,
            "GPU/CPU transformed image mean error was {mean_error}"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn gpu_text_uses_gpu_texture_path() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping text GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Text {
                x: 4.0,
                y: 24.0,
                content: "GPU text".into(),
                font_size: 18.0,
                color: Color::WHITE,
                font_weight: 400,
                font_sources: Vec::new(),
            }],
        };
        assert!(gpu_supports_scene(&scene));
        let image = gpu
            .render_frame(&scene, &FrameConfig::new(96, 32, 0, 30.0))
            .unwrap();
        let cpu = TinySkiaBackend::new()
            .render_frame(&scene, &FrameConfig::new(96, 32, 0, 30.0))
            .unwrap();
        let generation = gpu.text_atlas_cache_generation();
        let first_upload_bytes = gpu.text_atlas_upload_bytes();
        let same = gpu
            .render_frame(&scene, &FrameConfig::new(96, 32, 0, 30.0))
            .unwrap();
        assert_eq!(gpu.text_atlas_cache_generation(), generation);
        assert_eq!(gpu.text_atlas_upload_bytes(), first_upload_bytes);
        assert_eq!(
            same.pixels().collect::<Vec<_>>(),
            image.pixels().collect::<Vec<_>>(),
            "reusing an unchanged atlas must preserve the rendered pixels"
        );
        let updated_scene = Scene {
            nodes: vec![SceneNode::Text {
                x: 4.0,
                y: 24.0,
                content: "atlas update".into(),
                font_size: 18.0,
                color: Color::WHITE,
                font_weight: 400,
                font_sources: Vec::new(),
            }],
        };
        let second = gpu
            .render_frame(&updated_scene, &FrameConfig::new(96, 32, 1, 30.0))
            .unwrap();
        assert_eq!(gpu.render_stats().gpu_frames, 3);
        assert!(image.pixels().any(|pixel| pixel[3] > 0));
        assert!(second.pixels().any(|pixel| pixel[3] > 0));
        assert!(gpu.text_atlas_cache_generation().unwrap() > generation.unwrap());
        let second_upload_bytes = gpu.text_atlas_upload_bytes();
        assert!(second_upload_bytes > first_upload_bytes);
        assert!(second_upload_bytes - first_upload_bytes < 2048 * 2048);
        let alpha_error: u64 = image
            .pixels()
            .zip(cpu.pixels())
            .map(|(gpu, cpu)| (i16::from(gpu[3]) - i16::from(cpu[3])).unsigned_abs() as u64)
            .sum();
        let mean_alpha_error = alpha_error as f64 / (96 * 32) as f64;
        assert!(
            mean_alpha_error < 12.0,
            "GPU/CPU text alpha mean error was {mean_alpha_error}"
        );
    }

    #[test]
    fn gpu_brightness_filter_matches_cpu_for_normal_layer() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping brightness GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![
                SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 32.0,
                    h: 32.0,
                    fill: Color::BLACK,
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                },
                SceneNode::Layer {
                    opacity: 1.0,
                    blend_mode: crate::scene::BlendMode::Normal,
                    clip: None,
                    mask: None,
                    mask_mode: crate::scene::MaskMode::Alpha,
                    filters: vec![crate::scene::SceneFilter::Brightness { amount: 0.5 }],
                    shadow: None,
                    children: vec![SceneNode::Rect {
                        x: 8.0,
                        y: 8.0,
                        w: 16.0,
                        h: 16.0,
                        fill: Color::WHITE,
                        stroke: None,
                        stroke_width: 0.0,
                        corner_radius: 0.0,
                    }],
                },
            ],
        };
        assert!(gpu_supports_scene(&scene));
        let config = FrameConfig::new(32, 32, 0, 30.0);
        let gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let cpu_image = TinySkiaBackend::new()
            .render_frame(&scene, &config)
            .unwrap();
        let mean_error: f64 = gpu_image
            .pixels()
            .zip(cpu_image.pixels())
            .map(|(a, b)| {
                (0..4)
                    .map(|channel| {
                        (i16::from(a[channel]) - i16::from(b[channel])).unsigned_abs() as u64
                    })
                    .sum::<u64>()
            })
            .sum::<u64>() as f64
            / (32 * 32 * 4) as f64;
        assert!(
            mean_error < 2.0,
            "GPU/CPU brightness mean error was {mean_error}"
        );
    }

    #[test]
    fn gpu_grayscale_and_contrast_filters_match_cpu_for_normal_layer() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping grayscale GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 1.0,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: None,
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: vec![
                    crate::scene::SceneFilter::Grayscale { amount: 1.0 },
                    crate::scene::SceneFilter::Contrast { factor: 1.5 },
                ],
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 8.0,
                    y: 8.0,
                    w: 16.0,
                    h: 16.0,
                    fill: Color::rgb(220, 40, 80),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                }],
            }],
        };
        assert!(gpu_supports_scene(&scene));
        let config = FrameConfig::new(32, 32, 0, 30.0);
        let gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let cpu_image = TinySkiaBackend::new()
            .render_frame(&scene, &config)
            .unwrap();
        let gpu_center = gpu_image.get_pixel(16, 16);
        let cpu_center = cpu_image.get_pixel(16, 16);
        assert_eq!(gpu_center[0], gpu_center[1]);
        assert_eq!(gpu_center[1], gpu_center[2]);
        assert!(
            (i16::from(gpu_center[0]) - i16::from(cpu_center[0])).abs() <= 2,
            "GPU/CPU grayscale center mismatch: {gpu_center:?} vs {cpu_center:?}"
        );
    }

    #[test]
    fn gpu_saturation_filter_matches_cpu_for_normal_layer() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping saturation GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 1.0,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: None,
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: vec![crate::scene::SceneFilter::Saturation { factor: 0.5 }],
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 8.0,
                    y: 8.0,
                    w: 16.0,
                    h: 16.0,
                    fill: Color::rgb(220, 40, 80),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                }],
            }],
        };
        assert!(gpu_supports_scene(&scene));
        let config = FrameConfig::new(32, 32, 0, 30.0);
        let gpu_center = gpu
            .render_frame(&scene, &config)
            .unwrap()
            .get_pixel(16, 16)
            .to_owned();
        let cpu_center = TinySkiaBackend::new()
            .render_frame(&scene, &config)
            .unwrap()
            .get_pixel(16, 16)
            .to_owned();
        for channel in 0..3 {
            assert!(
                (i16::from(gpu_center[channel]) - i16::from(cpu_center[channel])).abs() <= 2,
                "GPU/CPU saturation channel {channel} mismatch: {gpu_center:?} vs {cpu_center:?}"
            );
        }
    }

    #[test]
    fn gpu_fullscreen_shader_uses_wgpu_shader_runner() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping shader GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Shader {
                x: 0.0,
                y: 0.0,
                w: 32.0,
                h: 16.0,
                source: "return vec4<f32>(uv.x, uv.y, 0.25, 1.0);".into(),
                time: 0.0,
                params: [1.0, 1.0, 1.0, 1.0],
                opacity: 0.5,
            }],
        };
        let image = gpu
            .render_frame(&scene, &FrameConfig::new(32, 16, 0, 30.0))
            .unwrap();
        assert_eq!(gpu.render_stats().gpu_frames, 1);
        assert!(image.get_pixel(1, 1)[2] > 0);
        assert!((120..=136).contains(&image.get_pixel(1, 1)[3]));
        assert_ne!(image.get_pixel(1, 1), image.get_pixel(30, 14));
    }

    #[test]
    fn gpu_shader_layers_preserve_rects_opacity_and_draw_order() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping shader layer test");
            return;
        };
        let scene = Scene {
            nodes: vec![
                SceneNode::Shader {
                    x: 2.0,
                    y: 1.0,
                    w: 8.0,
                    h: 6.0,
                    source: "return vec4<f32>(1.0, 0.0, 0.0, 1.0);".into(),
                    time: 0.0,
                    params: [0.0; 4],
                    opacity: 1.0,
                },
                SceneNode::Shader {
                    x: 5.0,
                    y: 3.0,
                    w: 4.0,
                    h: 2.0,
                    source: "return vec4<f32>(0.0, 0.0, 1.0, 1.0);".into(),
                    time: 0.0,
                    params: [0.0; 4],
                    opacity: 0.5,
                },
            ],
        };
        let image = gpu
            .render_frame(&scene, &FrameConfig::new(16, 10, 0, 30.0))
            .unwrap();
        assert_eq!(gpu.render_stats().gpu_frames, 1);
        assert_eq!(image.get_pixel(0, 0)[3], 0);
        assert_eq!(image.get_pixel(3, 2)[0], 255);
        let overlap = image.get_pixel(6, 4);
        assert!(
            overlap[0] > 80 && overlap[2] > 80,
            "overlap was {overlap:?}"
        );
        assert_eq!(image.get_pixel(10, 4)[3], 0);
    }

    #[test]
    fn gpu_shader_region_outside_target_uses_compatibility_path() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping shader bounds test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Shader {
                x: 14.0,
                y: 2.0,
                w: 8.0,
                h: 4.0,
                source: "return vec4<f32>(1.0, 0.0, 0.0, 1.0);".into(),
                time: 0.0,
                params: [0.0; 4],
                opacity: 1.0,
            }],
        };
        let image = gpu
            .render_frame(&scene, &FrameConfig::new(16, 10, 0, 30.0))
            .unwrap();
        assert_eq!(image.width(), 16);
        assert_eq!(image.height(), 10);
        assert!(image.get_pixel(15, 3)[0] > 0);
    }

    #[test]
    fn gpu_shader_suffix_composites_after_regular_gpu_scene() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping mixed shader test");
            return;
        };
        let scene = Scene {
            nodes: vec![
                SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 16.0,
                    h: 10.0,
                    fill: Color::rgb(255, 0, 0),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                },
                SceneNode::Shader {
                    x: 4.0,
                    y: 2.0,
                    w: 6.0,
                    h: 4.0,
                    source: "return vec4<f32>(0.0, 0.0, 1.0, 1.0);".into(),
                    time: 0.0,
                    params: [0.0; 4],
                    opacity: 0.5,
                },
            ],
        };
        let image = gpu
            .render_frame(&scene, &FrameConfig::new(16, 10, 0, 30.0))
            .unwrap();
        assert_eq!(gpu.render_stats().gpu_frames, 1);
        assert_eq!(image.get_pixel(1, 1)[0], 255);
        let overlap = image.get_pixel(6, 4);
        assert!(
            overlap[0] > 80 && overlap[2] > 80,
            "overlap was {overlap:?}"
        );
    }

    #[test]
    fn gpu_interleaved_shader_preserves_top_level_draw_order() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping interleaved shader test");
            return;
        };
        let rect = |color| SceneNode::Rect {
            x: 0.0,
            y: 0.0,
            w: 16.0,
            h: 10.0,
            fill: color,
            stroke: None,
            stroke_width: 0.0,
            corner_radius: 0.0,
        };
        let scene = Scene {
            nodes: vec![
                rect(Color::rgb(255, 0, 0)),
                SceneNode::Shader {
                    x: 4.0,
                    y: 2.0,
                    w: 6.0,
                    h: 4.0,
                    source: "return vec4<f32>(0.0, 0.0, 1.0, 1.0);".into(),
                    time: 0.0,
                    params: [0.0; 4],
                    opacity: 1.0,
                },
                rect(Color::rgb(0, 255, 0)),
            ],
        };
        let image = gpu
            .render_frame(&scene, &FrameConfig::new(16, 10, 0, 30.0))
            .unwrap();
        assert_eq!(image.get_pixel(1, 1)[1], 255);
        assert_eq!(image.get_pixel(6, 4)[1], 255);
        assert_eq!(image.get_pixel(6, 4)[2], 0);
        assert_eq!(gpu.render_stats().gpu_frames, 2);
    }

    #[test]
    fn gpu_image_cache_obeys_byte_budget_and_lru_eviction() {
        let Ok(gpu) = WgpuBackend::new().map(|backend| backend.with_image_cache_bytes(4)) else {
            println!("GPU backend unavailable; skipping image cache eviction test");
            return;
        };
        let red = "data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20width%3D%221%22%20height%3D%221%22%3E%3Crect%20width%3D%221%22%20height%3D%221%22%20fill%3D%22%23f00%22%2F%3E%3C%2Fsvg%3E";
        let blue = "data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20width%3D%221%22%20height%3D%221%22%3E%3Crect%20width%3D%221%22%20height%3D%221%22%20fill%3D%22%2300f%22%2F%3E%3C%2Fsvg%3E";
        let scene = Scene {
            nodes: vec![
                SceneNode::Image {
                    src: red.into(),
                    x: 0.0,
                    y: 0.0,
                    w: 8.0,
                    h: 8.0,
                    fit: ImageFit::Fill,
                    opacity: 1.0,
                },
                SceneNode::Image {
                    src: blue.into(),
                    x: 8.0,
                    y: 0.0,
                    w: 8.0,
                    h: 8.0,
                    fit: ImageFit::Fill,
                    opacity: 1.0,
                },
            ],
        };
        gpu.render_frame(&scene, &FrameConfig::new(16, 8, 0, 30.0))
            .unwrap();
        assert_eq!(gpu.gpu_image_cache_len(), 1);
    }

    #[test]
    fn gpu_video_frame_uses_decoded_texture_path() {
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            println!("FFmpeg unavailable; skipping GPU video test");
            return;
        }
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping GPU video test");
            return;
        };
        let dir = std::env::temp_dir().join(format!("dioxuscut-wgpu-video-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("red.mkv");
        let generated = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=16x16:r=2:d=1",
                "-c:v",
                "ffv1",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(generated.success());
        let scene = Scene {
            nodes: vec![SceneNode::Video {
                src: source.display().to_string(),
                time: 0.0,
                looped: false,
                x: 0.0,
                y: 0.0,
                w: 16.0,
                h: 16.0,
                fit: ImageFit::Fill,
                opacity: 1.0,
            }],
        };
        let image = gpu
            .render_frame(&scene, &FrameConfig::new(16, 16, 0, 2.0))
            .unwrap();
        let pixel = image.get_pixel(8, 8);
        assert!(pixel[0] > 200 && pixel[1] < 40 && pixel[2] < 40);
        let mut same_frame = scene.clone();
        let SceneNode::Video { src, time, .. } = &mut same_frame.nodes[0] else {
            unreachable!();
        };
        *src = format!("file://{}", source.display());
        *time = 0.1;
        gpu.render_frame(&same_frame, &FrameConfig::new(16, 16, 0, 2.0))
            .unwrap();
        assert_eq!(gpu.gpu_image_cache_len(), 1);
        assert_eq!(gpu.gpu_texture_uploads(), 1);
        assert_eq!(gpu.gpu_texture_upload_bytes(), 16 * 16 * 4);
        assert_eq!(gpu.gpu_texture_cache_misses(), 1);
        assert_eq!(gpu.gpu_texture_cache_hits(), 1);
        assert_eq!(gpu.gpu_frame_count.load(Ordering::Relaxed), 2);
        std::fs::remove_dir_all(dir).unwrap();
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

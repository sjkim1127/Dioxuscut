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
//! Each scene node type (Rect, Circle, Gradient) is rendered as a
//! screen-space quad. The fragment shader receives per-instance data
//! via a uniform buffer and selects the appropriate drawing mode.

#![cfg(feature = "gpu")]

use crate::backend::{FrameConfig, RasterError, RasterizerBackend};
use crate::scene::{Color, Scene, SceneNode};
use crate::tiny_skia_backend::TinySkiaBackend;
use image::RgbaImage;
use wgpu::util::DeviceExt;

// ────────────────────────────────────────────────────────────────────────────
// WGSL Shader Source
// ────────────────────────────────────────────────────────────────────────────

const SHADER_SRC: &str = r#"
struct Globals {
    resolution: vec2<f32>,
};

struct InstanceData {
    // Shape type: 0 = rect, 1 = circle, 2 = linear_gradient
    shape_type: u32,
    // Bounding box in pixels: [x, y, w, h]
    bounds: vec4<f32>,
    // Primary fill colour (RGBA, 0-1)
    color: vec4<f32>,
    // Secondary colour (for gradients, stroke)
    color2: vec4<f32>,
    // Auxiliary floats: corner_radius, stroke_width, angle_deg, opacity
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var<uniform> instance: InstanceData;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VertexOutput {
    // Map the instance bounding box to NDC
    let res = globals.resolution;
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
    // Convert pixel position to NDC (Y flipped)
    let ndc = vec2<f32>(
         pixel_pos.x / res.x * 2.0 - 1.0,
        -pixel_pos.y / res.y * 2.0 + 1.0,
    );
    // UV within bounding box, [0,1]
    let uv = vec2<f32>(
        (pixel_pos.x - x) / w,
        (pixel_pos.y - y) / h,
    );

    return VertexOutput(vec4<f32>(ndc, 0.0, 1.0), uv);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let shape_type = instance.shape_type;
    var col = instance.color;

    if shape_type == 0u {
        // ── Rect ─────────────────────────────────────────────────────────────
        let corner_r = instance.params.x;
        let w = instance.bounds.z;
        let h = instance.bounds.w;
        if corner_r > 0.0 {
            // SDF-based rounded corners
            let half = vec2<f32>(w, h) * 0.5;
            let p = (uv * vec2<f32>(w, h)) - half;
            let q = abs(p) - half + vec2<f32>(corner_r, corner_r);
            let d = length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - corner_r;
            let alpha = 1.0 - smoothstep(-0.5, 0.5, d);
            col = vec4<f32>(col.rgb, col.a * alpha);
        }

    } else if shape_type == 1u {
        // ── Circle ───────────────────────────────────────────────────────────
        let center = vec2<f32>(0.5, 0.5);
        let dist = length(uv - center);
        let alpha = 1.0 - smoothstep(0.5 - 0.5 / instance.bounds.z, 0.5, dist);
        col = vec4<f32>(col.rgb, col.a * alpha);

    } else if shape_type == 2u {
        // ── Linear Gradient ──────────────────────────────────────────────────
        let angle_rad = instance.params.z * 3.14159265 / 180.0;
        let dir = vec2<f32>(sin(angle_rad), cos(angle_rad));
        let t = dot(uv - vec2<f32>(0.5, 0.5), dir) + 0.5;
        let t_clamped = clamp(t, 0.0, 1.0);
        col = mix(col, instance.color2, t_clamped);

    } else if shape_type == 3u {
        // ── Radial Gradient ──────────────────────────────────────────────────
        let t = clamp(length(uv - vec2<f32>(0.5, 0.5)) * 2.0, 0.0, 1.0);
        col = mix(col, instance.color2, t);
    }

    // Apply global opacity
    col = vec4<f32>(col.rgb, col.a * instance.params.w);

    return col;
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
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Ok(Self {
            device,
            queue,
            pipeline,
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
        // Preserve output semantics until every SceneNode has a native GPU
        // implementation. A whole-frame fallback also preserves node ordering.
        if !gpu_supports_scene(scene) {
            return self.fallback.render_frame(scene, config);
        }

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
                    view: &texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }

        // Draw each node
        for node in &scene.nodes {
            if let Some(instance_data) = node_to_instance(node, width, height) {
                let instance_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("instance_buf"),
                    contents: bytemuck_cast(std::slice::from_ref(&instance_data)),
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

                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("draw_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &texture_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });

                pass.set_pipeline(&self.ctx.pipeline);
                pass.set_bind_group(0, &globals_bg, &[]);
                pass.set_bind_group(1, &instance_bg, &[]);
                pass.draw(0..6, 0..1);
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
/// All fields are f32 (std140-compatible when padded to vec4 boundaries).
#[repr(C)]
#[derive(Clone, Copy)]
struct GpuInstance {
    shape_type: [u32; 4], // [type, 0, 0, 0]
    bounds: [f32; 4],     // [x, y, w, h]
    color: [f32; 4],      // [r, g, b, a]
    color2: [f32; 4],     // [r, g, b, a]
    params: [f32; 4],     // [corner_radius, stroke_w, angle_deg, opacity]
}

fn node_to_instance(node: &SceneNode, _w: u32, _h: u32) -> Option<GpuInstance> {
    match node {
        SceneNode::Rect {
            x,
            y,
            w,
            h,
            fill,
            corner_radius,
            ..
        } => Some(GpuInstance {
            shape_type: [0, 0, 0, 0],
            bounds: [*x, *y, *w, *h],
            color: color_to_f32(*fill),
            color2: [0.0; 4],
            params: [*corner_radius, 0.0, 0.0, 1.0],
        }),

        SceneNode::Circle {
            cx, cy, r, fill, ..
        } => Some(GpuInstance {
            shape_type: [1, 0, 0, 0],
            bounds: [cx - r, cy - r, r * 2.0, r * 2.0],
            color: color_to_f32(*fill),
            color2: [0.0; 4],
            params: [0.0, 0.0, 0.0, 1.0],
        }),

        SceneNode::LinearGradient {
            x,
            y,
            w,
            h,
            angle_deg,
            stops,
        } if stops.len() >= 2 => Some(GpuInstance {
            shape_type: [2, 0, 0, 0],
            bounds: [*x, *y, *w, *h],
            color: color_to_f32(stops[0].color),
            color2: color_to_f32(stops[stops.len() - 1].color),
            params: [0.0, 0.0, *angle_deg, 1.0],
        }),

        SceneNode::RadialGradient { cx, cy, r, stops } if stops.len() >= 2 => Some(GpuInstance {
            shape_type: [3, 0, 0, 0],
            bounds: [cx - r, cy - r, r * 2.0, r * 2.0],
            color: color_to_f32(stops[0].color),
            color2: color_to_f32(stops[stops.len() - 1].color),
            params: [0.0, 0.0, 0.0, 1.0],
        }),

        // Complex scene nodes are not yet GPU-accelerated — skip for now.
        _ => None,
    }
}

fn gpu_supports_scene(scene: &Scene) -> bool {
    scene.nodes.iter().all(|node| match node {
        SceneNode::Rect {
            stroke,
            stroke_width,
            ..
        }
        | SceneNode::Circle {
            stroke,
            stroke_width,
            ..
        } => stroke.is_none() || *stroke_width <= 0.0,
        SceneNode::LinearGradient { stops, .. } | SceneNode::RadialGradient { stops, .. } => {
            stops.len() == 2
        }
        SceneNode::Path { .. }
        | SceneNode::Text { .. }
        | SceneNode::Image { .. }
        | SceneNode::Video { .. }
        | SceneNode::Group { .. }
        | SceneNode::Layer { .. } => false,
        SceneNode::Audio { .. } => true,
    })
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
}

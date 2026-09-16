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
//! Analytic input colours are converted from the CPU's u8 sRGB representation
//! to linear floats. The internal target is Unorm and shader output performs
//! the explicit linear-to-sRGB conversion so fixed-function blending stays in
//! the same byte-domain semantics as the CPU renderer.
//!
//! # Resource Pool
//!
//! [`GpuFrameResources`] caches per-resolution textures and the readback
//! buffer inside `WgpuBackend`, eliminating the allocation pressure of
//! re-creating GPU objects on every `render_frame` call.

#![cfg(feature = "gpu")]

use crate::backend::{
    BackendCapabilities, BackendRenderStats, FrameConfig, FrameSink, RasterError, RasterizerBackend,
};
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
// Keep the render target in the same sRGB encoding as the CPU `Color` and
// `RgbaImage` APIs. Analytic uniforms are linearized before shading and the
// hardware performs the final linear -> sRGB conversion on store.
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
    // x = offset, y = darkness, z = roundness, w = enabled
    vignette: vec4<f32>,
    // x = invert amount
    invert: vec4<f32>,
    // x = hue rotation in degrees
    hue: vec4<f32>,
    // rgb = tint color in sRGB, w = amount
    tint: vec4<f32>,
    // rgb = duotone endpoints in sRGB, w = enabled on primary
    duotone_primary: vec4<f32>,
    duotone_secondary: vec4<f32>,
    // x = contrast, y = saturation, z = inverse gamma, w = enabled
    grading: vec4<f32>,
    // rgb = tint color in sRGB, w = amount
    grading_tint: vec4<f32>,
    clip_rects: array<vec4<f32>, 4>,
    mask_opacity: vec4<f32>,
    mask_kinds: vec4<u32>,
    mask_shapes: array<vec4<f32>, 4>,
    mask_color0: array<vec4<f32>, 4>,
    mask_color1: array<vec4<f32>, 4>,
    mask_color2: array<vec4<f32>, 4>,
    mask_color3: array<vec4<f32>, 4>,
    mask_stop_positions: array<vec4<f32>, 4>,
    mask_stop_counts: vec4<u32>,
    // corner radius, stroke width, angle, legacy inherited opacity slot
    params: vec4<f32>,
    // inherited opacity kept separate from image/text UV coordinates
    opacity: vec4<f32>,
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
@group(3) @binding(0) var path_mask_texture: texture_2d<f32>;
@group(3) @binding(1) var path_mask_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local_position: vec2<f32>,
    @location(1) @interpolate(flat) instance_index: u32,
    @location(2) transformed_position: vec2<f32>,
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
    return VertexOutput(to_clip_position(transformed), pixel_pos, iid, transformed);
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
    return VertexOutput(to_clip_position(transformed), vertex.position, iid, transformed);
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

fn srgb_to_linear(channel: f32) -> f32 {
    if channel <= 0.04045 {
        return channel / 12.92;
    }
    return pow((channel + 0.055) / 1.055, 2.4);
}

fn apply_color_filters(color: vec4<f32>, instance: InstanceData) -> vec4<f32> {
    if instance.brightness.x == 1.0
        && instance.grayscale.x == 0.0
        && instance.contrast.x == 1.0
        && instance.saturation.x == 1.0
        && instance.invert.x == 0.0
        && instance.hue.x == 0.0
        && instance.tint.w == 0.0
        && instance.duotone_primary.w == 0.0
        && instance.grading.w == 0.0
    {
        return color;
    }
    // Filter math follows TinySkia's sRGB byte-domain behaviour. All regular
    // GPU inputs (including image/video samples) arrive here in linear light,
    // so convert them to sRGB for the filter operations and convert the
    // result back before storing into the sRGB render target.
    var srgb = vec3<f32>(
        linear_to_srgb(clamp(color.r, 0.0, 1.0)),
        linear_to_srgb(clamp(color.g, 0.0, 1.0)),
        linear_to_srgb(clamp(color.b, 0.0, 1.0)),
    );
    srgb = srgb * instance.brightness.x;
    let luma = dot(srgb, vec3<f32>(0.299, 0.587, 0.114));
    srgb = luma + (srgb - vec3<f32>(luma)) * instance.saturation.x;
    if instance.grayscale.x > 0.0 {
        let gray = dot(srgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        srgb = mix(srgb, vec3<f32>(gray), instance.grayscale.x);
    }
    srgb = (srgb - vec3<f32>(0.5)) * instance.contrast.x + vec3<f32>(0.5);
    srgb = mix(srgb, vec3<f32>(1.0) - srgb, clamp(instance.invert.x, 0.0, 1.0));
    let max_channel = max(srgb.r, max(srgb.g, srgb.b));
    let min_channel = min(srgb.r, min(srgb.g, srgb.b));
    let delta = max_channel - min_channel;
    let value = max_channel;
    let saturation_value = select(0.0, delta / max_channel, max_channel > 0.0);
    var hue_degrees = 0.0;
    if delta > 0.000001 {
        if max_channel == srgb.r {
            hue_degrees = 60.0 * ((srgb.g - srgb.b) / delta);
        } else if max_channel == srgb.g {
            hue_degrees = 60.0 * ((srgb.b - srgb.r) / delta + 2.0);
        } else {
            hue_degrees = 60.0 * ((srgb.r - srgb.g) / delta + 4.0);
        }
    }
    hue_degrees = (hue_degrees + instance.hue.x) % 360.0;
    if hue_degrees < 0.0 {
        hue_degrees = hue_degrees + 360.0;
    }
    let chroma = value * saturation_value;
    let hue_sector = hue_degrees / 60.0;
    let x = chroma * (1.0 - abs((hue_sector % 2.0) - 1.0));
    let match_value = value - chroma;
    var rotated = vec3<f32>(0.0);
    if hue_sector < 1.0 {
        rotated = vec3<f32>(chroma, x, 0.0);
    } else if hue_sector < 2.0 {
        rotated = vec3<f32>(x, chroma, 0.0);
    } else if hue_sector < 3.0 {
        rotated = vec3<f32>(0.0, chroma, x);
    } else if hue_sector < 4.0 {
        rotated = vec3<f32>(0.0, x, chroma);
    } else if hue_sector < 5.0 {
        rotated = vec3<f32>(x, 0.0, chroma);
    } else {
        rotated = vec3<f32>(chroma, 0.0, x);
    }
    srgb = rotated + vec3<f32>(match_value);
    srgb = mix(srgb, instance.tint.rgb, clamp(instance.tint.w, 0.0, 1.0));
    if instance.duotone_primary.w > 0.0 {
        let duo_luma = clamp(dot(srgb, vec3<f32>(0.299, 0.587, 0.114)), 0.0, 1.0);
        srgb = mix(instance.duotone_primary.rgb, instance.duotone_secondary.rgb, duo_luma);
    }
    if instance.grading.w > 0.0 {
        srgb = pow(clamp(srgb, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(instance.grading.z));
        srgb = (srgb - vec3<f32>(0.5)) * instance.grading.x + vec3<f32>(0.5);
        let grading_luma = dot(srgb, vec3<f32>(0.299, 0.587, 0.114));
        srgb = grading_luma + (srgb - vec3<f32>(grading_luma)) * instance.grading.y;
        srgb = mix(srgb, instance.grading_tint.rgb, clamp(instance.grading_tint.w, 0.0, 1.0));
    }
    let linear = vec3<f32>(
        srgb_to_linear(clamp(srgb.r, 0.0, 1.0)),
        srgb_to_linear(clamp(srgb.g, 0.0, 1.0)),
        srgb_to_linear(clamp(srgb.b, 0.0, 1.0)),
    );
    return vec4<f32>(linear, color.a);
}

fn vignette_factor(position: vec2<f32>, bounds: vec4<f32>, settings: vec4<f32>) -> f32 {
    if settings.w < 0.5 {
        return 1.0;
    }
    let local = clamp((position - bounds.xy) / max(bounds.zw, vec2<f32>(0.000001)), vec2<f32>(0.0), vec2<f32>(1.0));
    let px = 2.0 * abs(local.x - 0.5);
    let py = 2.0 * abs(local.y - 0.5);
    let offset = clamp(settings.x, 0.0, 2.0);
    let darkness = clamp(settings.y, 0.0, 1.0);
    let roundness = clamp(settings.z, 0.0, 1.0);
    let max_diag = (1.0 - roundness) + roundness * sqrt(2.0);
    let span = max(max_diag - offset, 0.00001);
    let d = mix(max(px, py), sqrt(px * px + py * py), roundness);
    let t = clamp((d - offset) / span, 0.0, 1.0);
    let smooth_factor = t * t * (3.0 - 2.0 * t);
    return 1.0 - darkness * smooth_factor;
}

fn apply_srgb_vignette(rgb: vec3<f32>, factor: f32) -> vec3<f32> {
    let srgb = vec3<f32>(
        linear_to_srgb(clamp(rgb.r, 0.0, 1.0)),
        linear_to_srgb(clamp(rgb.g, 0.0, 1.0)),
        linear_to_srgb(clamp(rgb.b, 0.0, 1.0)),
    ) * factor;
    return vec3<f32>(
        srgb_to_linear(clamp(srgb.r, 0.0, 1.0)),
        srgb_to_linear(clamp(srgb.g, 0.0, 1.0)),
        srgb_to_linear(clamp(srgb.b, 0.0, 1.0)),
    );
}

fn composited_color(rgb: vec3<f32>, alpha: f32, instance: InstanceData) -> vec4<f32> {
    var output_rgb = rgb;
    // A frame containing fixed-function blend operations must keep every
    // source and destination in the same sRGB domain. The regular renderer
    // remains linear-light for compatibility with its existing path.
    // Image/video/Lottie samples are already stored as normalized sRGB
    // texels; analytic colors are stored in linear space.
    var output_is_srgb = false;
    if (instance.kind_data.w & 2u) != 0u && instance.kind_data.x != 5u {
        output_rgb = vec3<f32>(
            linear_to_srgb(clamp(rgb.r, 0.0, 1.0)),
            linear_to_srgb(clamp(rgb.g, 0.0, 1.0)),
            linear_to_srgb(clamp(rgb.b, 0.0, 1.0)),
        );
        output_is_srgb = true;
    }
    // Multiply/Screen/Darken/Lighten blend factors require premultiplied
    // source RGB.
    if instance.kind_data.z != 0u {
        return vec4<f32>(output_rgb * alpha, alpha);
    }
    if !output_is_srgb {
        output_rgb = vec3<f32>(
            linear_to_srgb(clamp(output_rgb.r, 0.0, 1.0)),
            linear_to_srgb(clamp(output_rgb.g, 0.0, 1.0)),
            linear_to_srgb(clamp(output_rgb.b, 0.0, 1.0)),
        );
    }
    return vec4<f32>(output_rgb, alpha);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let instance = instances[in.instance_index];
    let mask_coverage_value = mask_coverage(in.transformed_position, instance);
    if mask_coverage_value <= 0.0 {
        discard;
    }
    let shape_type = instance.kind_data.x;
    let shape = instance.shape_bounds;
    var col = instance.color;
    var coverage = 1.0;

    if shape_type == 0u {
        let half = shape.zw * 0.5;
        let center = shape.xy + half;
        let corner_r = clamp(instance.params.x, 0.0, min(half.x, half.y));
        let stroke_width = instance.params.y;
        let p = in.local_position - center;
        let q = abs(p) - half + vec2<f32>(corner_r);
        let distance = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - corner_r;
        if corner_r <= 0.0 && stroke_width <= 0.0 {
            // TinySkia's plain axis-aligned rectangles have hard edges. The
            // analytic SDF smoothing is reserved for rounded/stroked shapes;
            // applying it to opaque fills creates an avoidable CPU/GPU fringe.
            coverage = 1.0;
        } else {
            coverage = 1.0 - smoothstep(-0.75, 0.75, distance);
        }
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
    let vignette = vignette_factor(in.local_position, instance.shape_bounds, instance.vignette);
    let alpha = col.a * instance.opacity.x * coverage * mask_coverage_value;
    return composited_color(apply_srgb_vignette(col.rgb, vignette), alpha, instance);
}

@fragment
fn fs_solid(in: VertexOutput) -> @location(0) vec4<f32> {
    let instance = instances[in.instance_index];
    let mask_coverage_value = mask_coverage(in.transformed_position, instance);
    if mask_coverage_value <= 0.0 {
        discard;
    }
    let color = apply_color_filters(instance.color, instance);
    let vignette = vignette_factor(in.local_position, instance.shape_bounds, instance.vignette);
    let alpha = color.a * instance.opacity.x * mask_coverage_value;
    return composited_color(apply_srgb_vignette(color.rgb, vignette), alpha, instance);
}

@fragment
fn fs_image(in: VertexOutput) -> @location(0) vec4<f32> {
    let instance = instances[in.instance_index];
    let mask_coverage_value = mask_coverage(in.transformed_position, instance);
    if mask_coverage_value <= 0.0 {
        discard;
    }
    let bounds = instance.shape_bounds;
    let local = (in.local_position - bounds.xy) / max(bounds.zw, vec2<f32>(0.000001));
    let uv = mix(instance.params.xy, instance.params.zw, clamp(local, vec2<f32>(0.0), vec2<f32>(1.0)));
    let sampled = textureSample(image_texture, image_sampler, uv);
    let sampled_linear = vec3<f32>(
        srgb_to_linear(sampled.r),
        srgb_to_linear(sampled.g),
        srgb_to_linear(sampled.b),
    );
    let color = apply_color_filters(vec4<f32>(sampled_linear, instance.color.a), instance);
    let vignette = vignette_factor(in.local_position, instance.shape_bounds, instance.vignette);
    let alpha = sampled.a * instance.color.a * instance.opacity.x * mask_coverage_value;
    return composited_color(apply_srgb_vignette(color.rgb, vignette), alpha, instance);
}

@fragment
fn fs_text(in: VertexOutput) -> @location(0) vec4<f32> {
    let instance = instances[in.instance_index];
    let mask_coverage_value = mask_coverage(in.transformed_position, instance);
    if mask_coverage_value <= 0.0 {
        discard;
    }
    let bounds = instance.shape_bounds;
    let local = (in.local_position - bounds.xy) / max(bounds.zw, vec2<f32>(0.000001));
    let uv = mix(instance.params.xy, instance.params.zw, clamp(local, vec2<f32>(0.0), vec2<f32>(1.0)));
    let coverage = textureSample(image_texture, image_sampler, uv).r;
    let color = apply_color_filters(instance.color, instance);
    let vignette = vignette_factor(in.local_position, instance.shape_bounds, instance.vignette);
    let alpha = coverage * color.a * instance.opacity.x * mask_coverage_value;
    return composited_color(apply_srgb_vignette(color.rgb, vignette), alpha, instance);
}

fn mask_coverage(position: vec2<f32>, instance: InstanceData) -> f32 {
    var coverage = 0.0;
    var has_mask = false;
    var has_regular_mask = false;
    var clip_coverage = 1.0;
    let rect0 = instance.clip_rects[0];
    let rect1 = instance.clip_rects[1];
    let rect2 = instance.clip_rects[2];
    let rect3 = instance.clip_rects[3];
    let opacity0 = instance.mask_opacity[0];
    let opacity1 = instance.mask_opacity[1];
    let opacity2 = instance.mask_opacity[2];
    let opacity3 = instance.mask_opacity[3];
    if opacity0 >= 0.0 { has_mask = true; let value = mask_shape_coverage(position, rect0, instance.mask_shapes[0], instance.mask_kinds.x, opacity0, instance.mask_color0[0], instance.mask_color1[0], instance.mask_color2[0], instance.mask_color3[0], instance.mask_stop_positions[0], instance.mask_stop_counts.x); if instance.mask_kinds.x == 6u { clip_coverage *= value; } else { has_regular_mask = true; coverage = 1.0 - (1.0 - coverage) * (1.0 - value); } }
    if opacity1 >= 0.0 { has_mask = true; let value = mask_shape_coverage(position, rect1, instance.mask_shapes[1], instance.mask_kinds.y, opacity1, instance.mask_color0[1], instance.mask_color1[1], instance.mask_color2[1], instance.mask_color3[1], instance.mask_stop_positions[1], instance.mask_stop_counts.y); if instance.mask_kinds.y == 6u { clip_coverage *= value; } else { has_regular_mask = true; coverage = 1.0 - (1.0 - coverage) * (1.0 - value); } }
    if opacity2 >= 0.0 { has_mask = true; let value = mask_shape_coverage(position, rect2, instance.mask_shapes[2], instance.mask_kinds.z, opacity2, instance.mask_color0[2], instance.mask_color1[2], instance.mask_color2[2], instance.mask_color3[2], instance.mask_stop_positions[2], instance.mask_stop_counts.z); if instance.mask_kinds.z == 6u { clip_coverage *= value; } else { has_regular_mask = true; coverage = 1.0 - (1.0 - coverage) * (1.0 - value); } }
    if opacity3 >= 0.0 { has_mask = true; let value = mask_shape_coverage(position, rect3, instance.mask_shapes[3], instance.mask_kinds.w, opacity3, instance.mask_color0[3], instance.mask_color1[3], instance.mask_color2[3], instance.mask_color3[3], instance.mask_stop_positions[3], instance.mask_stop_counts.w); if instance.mask_kinds.w == 6u { clip_coverage *= value; } else { has_regular_mask = true; coverage = 1.0 - (1.0 - coverage) * (1.0 - value); } }
    if has_mask {
        var effective_coverage = coverage;
        if !has_regular_mask { effective_coverage = 1.0; }
        var result = effective_coverage * clip_coverage;
        if (instance.kind_data.w & 1u) == 1u {
            let uv = position / globals.resolution;
            result *= textureSample(path_mask_texture, path_mask_sampler, uv).r;
        }
        return result;
    }
    if (instance.kind_data.w & 1u) == 1u {
        let uv = position / globals.resolution;
        return textureSample(path_mask_texture, path_mask_sampler, uv).r;
    }
    return 1.0;
}

fn mask_shape_coverage(position: vec2<f32>, rect: vec4<f32>, shape: vec4<f32>, kind: u32, opacity: f32, color0: vec4<f32>, color1: vec4<f32>, color2: vec4<f32>, color3: vec4<f32>, positions: vec4<f32>, stop_count: u32) -> f32 {
    var inside = position.x >= rect.x && position.y >= rect.y &&
        position.x < rect.x + rect.z && position.y < rect.y + rect.w;
    if kind == 1u {
        let delta = position - shape.xy;
        inside = dot(delta, delta) < shape.z * shape.z;
    }
    if kind == 4u {
        let delta = position - shape.xy;
        inside = dot(delta, delta) < shape.z * shape.z;
    }
    if kind == 5u {
        let half_size = rect.zw * 0.5;
        let radius = min(shape.x, min(half_size.x, half_size.y));
        let delta = abs(position - (rect.xy + half_size)) -
            (half_size - vec2<f32>(radius, radius));
        let distance = length(max(delta, vec2<f32>(0.0))) +
            min(max(delta.x, delta.y), 0.0) - radius;
        inside = distance <= 0.0;
    }
    var shape_opacity = opacity;
    if kind == 2u || kind == 3u || kind == 4u {
        let direction = shape.zw - shape.xy;
        var t = clamp(dot(position - shape.xy, direction) / max(dot(direction, direction), 0.000001), 0.0, 1.0);
        if kind == 4u {
            t = clamp(length(position - shape.xy) / max(shape.z, 0.000001), 0.0, 1.0);
        }
        var color = color0;
        if stop_count >= 2u {
            if t <= positions.y {
                color = mix(color0, color1, clamp((t - positions.x) / max(positions.y - positions.x, 0.000001), 0.0, 1.0));
            } else if stop_count == 2u {
                color = color1;
            } else if t <= positions.z {
                color = mix(color1, color2, clamp((t - positions.y) / max(positions.z - positions.y, 0.000001), 0.0, 1.0));
            } else if stop_count == 3u {
                color = color2;
            } else if t <= positions.w {
                color = mix(color2, color3, clamp((t - positions.z) / max(positions.w - positions.z, 0.000001), 0.0, 1.0));
            } else {
                color = color3;
            }
        }
        shape_opacity = color.a;
        if kind == 3u {
            shape_opacity = dot(color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)) * color.a;
        }
    }
    if inside {
        return clamp(shape_opacity, 0.0, 1.0);
    }
    return 0.0;
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
    /// Single-sample pipeline used when compositing a resolved offscreen
    /// layer texture into the resolved frame target.
    #[allow(dead_code)]
    image_composite_pipeline: wgpu::RenderPipeline,
    path_mask_pipeline: wgpu::RenderPipeline,
    text_pipeline: wgpu::RenderPipeline,
    multiply_pipeline: wgpu::RenderPipeline,
    multiply_mesh_pipeline: wgpu::RenderPipeline,
    multiply_image_pipeline: wgpu::RenderPipeline,
    multiply_text_pipeline: wgpu::RenderPipeline,
    screen_pipeline: wgpu::RenderPipeline,
    screen_mesh_pipeline: wgpu::RenderPipeline,
    screen_image_pipeline: wgpu::RenderPipeline,
    screen_text_pipeline: wgpu::RenderPipeline,
    darken_pipeline: wgpu::RenderPipeline,
    darken_mesh_pipeline: wgpu::RenderPipeline,
    darken_image_pipeline: wgpu::RenderPipeline,
    darken_text_pipeline: wgpu::RenderPipeline,
    lighten_pipeline: wgpu::RenderPipeline,
    lighten_mesh_pipeline: wgpu::RenderPipeline,
    lighten_image_pipeline: wgpu::RenderPipeline,
    lighten_text_pipeline: wgpu::RenderPipeline,
    globals_layout: wgpu::BindGroupLayout,
    instance_layout: wgpu::BindGroupLayout,
    image_layout: wgpu::BindGroupLayout,
    path_mask_layout: wgpu::BindGroupLayout,
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

        let path_mask_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("path_mask_bgl"),
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
            bind_group_layouts: &[
                &globals_layout,
                &instance_layout,
                &image_layout,
                &path_mask_layout,
            ],
            push_constant_ranges: &[],
        });

        let make_blend_pipeline = |label: &'static str,
                                   entry_point: &'static str,
                                   mesh: bool,
                                   blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: if mesh { "vs_mesh" } else { "vs_main" },
                    buffers: if mesh {
                        &[wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<GpuVertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &MESH_ATTRIBUTES,
                        }]
                    } else {
                        &[]
                    },
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point,
                    targets: &[Some(wgpu::ColorTargetState {
                        format: RENDER_FORMAT,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: if mesh {
                    wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        ..Default::default()
                    }
                } else {
                    wgpu::PrimitiveState::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: SAMPLE_COUNT,
                    ..Default::default()
                },
                multiview: None,
                cache: None,
            })
        };
        let multiply = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Dst,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let screen = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::OneMinusDst,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let multiply_pipeline =
            make_blend_pipeline("dioxuscut_multiply_pipeline", "fs_main", false, multiply);
        let multiply_mesh_pipeline = make_blend_pipeline(
            "dioxuscut_multiply_mesh_pipeline",
            "fs_solid",
            true,
            multiply,
        );
        let multiply_image_pipeline = make_blend_pipeline(
            "dioxuscut_multiply_image_pipeline",
            "fs_image",
            false,
            multiply,
        );
        let multiply_text_pipeline = make_blend_pipeline(
            "dioxuscut_multiply_text_pipeline",
            "fs_text",
            false,
            multiply,
        );
        let screen_pipeline =
            make_blend_pipeline("dioxuscut_screen_pipeline", "fs_main", false, screen);
        let screen_mesh_pipeline =
            make_blend_pipeline("dioxuscut_screen_mesh_pipeline", "fs_solid", true, screen);
        let screen_image_pipeline =
            make_blend_pipeline("dioxuscut_screen_image_pipeline", "fs_image", false, screen);
        let screen_text_pipeline =
            make_blend_pipeline("dioxuscut_screen_text_pipeline", "fs_text", false, screen);
        let darken = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Min,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let lighten = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Max,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let darken_pipeline =
            make_blend_pipeline("dioxuscut_darken_pipeline", "fs_main", false, darken);
        let darken_mesh_pipeline =
            make_blend_pipeline("dioxuscut_darken_mesh_pipeline", "fs_solid", true, darken);
        let darken_image_pipeline =
            make_blend_pipeline("dioxuscut_darken_image_pipeline", "fs_image", false, darken);
        let darken_text_pipeline =
            make_blend_pipeline("dioxuscut_darken_text_pipeline", "fs_text", false, darken);
        let lighten_pipeline =
            make_blend_pipeline("dioxuscut_lighten_pipeline", "fs_main", false, lighten);
        let lighten_mesh_pipeline =
            make_blend_pipeline("dioxuscut_lighten_mesh_pipeline", "fs_solid", true, lighten);
        let lighten_image_pipeline = make_blend_pipeline(
            "dioxuscut_lighten_image_pipeline",
            "fs_image",
            false,
            lighten,
        );
        let lighten_text_pipeline =
            make_blend_pipeline("dioxuscut_lighten_text_pipeline", "fs_text", false, lighten);

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

        let image_composite_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("dioxuscut_image_composite_pipeline"),
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
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        let path_mask_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("dioxuscut_path_mask_pipeline"),
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
                    format: wgpu::TextureFormat::R8Unorm,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::RED,
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
            image_composite_pipeline,
            path_mask_pipeline,
            text_pipeline,
            multiply_pipeline,
            multiply_mesh_pipeline,
            multiply_image_pipeline,
            multiply_text_pipeline,
            screen_pipeline,
            screen_mesh_pipeline,
            screen_image_pipeline,
            screen_text_pipeline,
            darken_pipeline,
            darken_mesh_pipeline,
            darken_image_pipeline,
            darken_text_pipeline,
            lighten_pipeline,
            lighten_mesh_pipeline,
            lighten_image_pipeline,
            lighten_text_pipeline,
            globals_layout,
            instance_layout,
            image_layout,
            path_mask_layout,
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
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
type GpuLayerResourcePool = Mutex<HashMap<(u32, u32), std::sync::Arc<Mutex<GpuFrameSlot>>>>;

struct GpuImageResource {
    _texture: wgpu::Texture,
    _view: wgpu::TextureView,
    _sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

/// A bind group for a texture owned by another GPU resource, such as an
/// offscreen layer slot. The sampler and bind group deliberately do not own
/// the source texture; the caller must keep the slot alive for the draw.
#[allow(dead_code)]
struct GpuExternalTexture {
    _sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
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

    fn trim_to_budget(&mut self) {
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
    /// Reusable single-target slots for layer offscreen passes. These are
    /// intentionally separate from the frame readback ring so a future layer
    /// pass cannot stall the ordered output slots.
    layer_resources: GpuLayerResourcePool,
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
    text_atlas_full_uploads: AtomicU64,
    text_atlas_dirty_uploads: AtomicU64,
    text_atlas_full_upload_bytes: AtomicU64,
    text_atlas_dirty_upload_bytes: AtomicU64,
    text_atlas_cache_hits: AtomicU64,
    text_atlas_cache_misses: AtomicU64,
    gpu_texture_uploads: AtomicU64,
    gpu_texture_upload_bytes: AtomicU64,
    gpu_texture_cache_hits: AtomicU64,
    gpu_texture_cache_misses: AtomicU64,
    video_decode_ns: AtomicU64,
    texture_upload_ns: AtomicU64,
    gpu_submit_readback_ns: AtomicU64,
    gpu_frame_count: AtomicU64,
    cpu_fallback_frame_count: AtomicU64,
    last_cpu_fallback_reason: Mutex<Option<String>>,
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
            layer_resources: Mutex::new(HashMap::new()),
            fallback: TinySkiaBackend::new(),
            image_cache: ImageCache::default(),
            video_cache: VideoFrameCache::default(),
            gpu_images: Mutex::new(GpuImageCacheState::new(256 * 1024 * 1024)),
            text_atlas: Mutex::new(None),
            text_atlas_upload_bytes: AtomicU64::new(0),
            text_atlas_full_uploads: AtomicU64::new(0),
            text_atlas_dirty_uploads: AtomicU64::new(0),
            text_atlas_full_upload_bytes: AtomicU64::new(0),
            text_atlas_dirty_upload_bytes: AtomicU64::new(0),
            text_atlas_cache_hits: AtomicU64::new(0),
            text_atlas_cache_misses: AtomicU64::new(0),
            gpu_texture_uploads: AtomicU64::new(0),
            gpu_texture_upload_bytes: AtomicU64::new(0),
            gpu_texture_cache_hits: AtomicU64::new(0),
            gpu_texture_cache_misses: AtomicU64::new(0),
            video_decode_ns: AtomicU64::new(0),
            texture_upload_ns: AtomicU64::new(0),
            gpu_submit_readback_ns: AtomicU64::new(0),
            gpu_frame_count: AtomicU64::new(0),
            cpu_fallback_frame_count: AtomicU64::new(0),
            last_cpu_fallback_reason: Mutex::new(None),
        })
    }

    fn offscreen_layer_slot(&self, width: u32, height: u32) -> Arc<Mutex<GpuFrameSlot>> {
        let mut pool = self
            .layer_resources
            .lock()
            .expect("GPU layer resource pool mutex poisoned");
        pool.entry((width, height))
            .or_insert_with(|| {
                Arc::new(Mutex::new(GpuFrameSlot::new(
                    &self.ctx.device,
                    width,
                    height,
                )))
            })
            .clone()
    }

    /// Create the shared texture bind group used by image, video, text-atlas,
    /// and future resolved offscreen-layer inputs. The texture view remains
    /// owned by the caller, which is required for layer-slot ownership.
    fn image_bind_group(
        &self,
        view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        label: &'static str,
    ) -> wgpu::BindGroup {
        self.ctx
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.ctx.image_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            })
    }

    #[allow(dead_code)]
    fn offscreen_layer_binding(&self, slot: &GpuFrameSlot) -> GpuExternalTexture {
        let sampler = self.ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("offscreen_layer_sampler"),
            ..Default::default()
        });
        let bind_group =
            self.image_bind_group(&slot.texture_view, &sampler, "offscreen_layer_texture_bg");
        GpuExternalTexture {
            _sampler: sampler,
            bind_group,
        }
    }

    /// Composite a resolved layer texture into another resolved target. This
    /// pass intentionally uses the single-sample pipeline: the destination is
    /// already resolved, so loading it as an MSAA attachment would be invalid.
    #[allow(dead_code)]
    fn composite_external_texture(
        &self,
        source: &GpuFrameSlot,
        destination: &GpuFrameSlot,
        width: u32,
        height: u32,
        opacity: f32,
    ) -> Result<wgpu::SubmissionIndex, RasterError> {
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(RasterError::Scene("invalid offscreen layer opacity".into()));
        }
        let device = &self.ctx.device;
        let globals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("layer_composite_globals"),
            contents: bytemuck_cast(&[width as f32, height as f32, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layer_composite_globals_bg"),
            layout: &self.ctx.globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let mut instance = GpuInstance::solid(Color::WHITE, opacity, Transform::identity());
        instance.kind_data[0] = 5;
        instance.bounds = [0.0, 0.0, width as f32, height as f32];
        instance.shape_bounds = instance.bounds;
        instance.params = [0.0, 0.0, 1.0, 1.0];
        let instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("layer_composite_instance"),
            contents: bytemuck_cast(&[instance]),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let instances_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layer_composite_instances_bg"),
            layout: &self.ctx.instance_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: instances.as_entire_binding(),
            }],
        });
        let external = self.offscreen_layer_binding(source);
        let mask_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("layer_composite_dummy_mask"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mask_view = mask_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mask_sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        let mask_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layer_composite_dummy_mask_bg"),
            layout: &self.ctx.path_mask_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&mask_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&mask_sampler),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("layer_composite_encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("layer_composite_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &destination.texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.ctx.image_composite_pipeline);
            pass.set_bind_group(0, &globals_bg, &[]);
            pass.set_bind_group(1, &instances_bg, &[]);
            pass.set_bind_group(2, &external.bind_group, &[]);
            pass.set_bind_group(3, &mask_bg, &[]);
            pass.draw(0..6, 0..1);
        }
        Ok(self.ctx.queue.submit(Some(encoder.finish())))
    }

    fn readback_resolved_slot(
        &self,
        slot: &GpuFrameSlot,
        width: u32,
        height: u32,
    ) -> Result<RgbaImage, RasterError> {
        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("layer_composite_readback_encoder"),
            });
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
        let submission = self.ctx.queue.submit(Some(encoder.finish()));
        let (tx, rx) = std::sync::mpsc::channel();
        slot.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        let _ = submission;
        self.ctx.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|error| RasterError::Scene(format!("layer readback channel failed: {error}")))?
            .map_err(|error| RasterError::Scene(format!("layer readback map failed: {error}")))?;
        let slice = slot.readback.slice(..);
        let mapped = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let start = (row * slot.bytes_per_row) as usize;
            pixels.extend_from_slice(&mapped[start..start + (width * 4) as usize]);
        }
        drop(mapped);
        slot.readback.unmap();
        RgbaImage::from_raw(width, height, pixels)
            .ok_or_else(|| RasterError::ImageEncode("Invalid layer composite readback".into()))
    }

    fn render_trailing_overlap_layer(
        &self,
        base_scene: &Scene,
        layer_scene: &Scene,
        layer_opacity: f32,
        config: &FrameConfig,
    ) -> Result<RgbaImage, RasterError> {
        let Some((base_commands, base_mask)) =
            compile_scene_with_path_mask(base_scene, &self.fallback)
        else {
            return Err(RasterError::Scene(
                "overlap base scene requires CPU fallback".into(),
            ));
        };
        let Some((layer_commands, layer_mask)) =
            compile_scene_with_path_mask(layer_scene, &self.fallback)
        else {
            return Err(RasterError::Scene(
                "overlap layer requires CPU fallback".into(),
            ));
        };
        if base_mask.is_some() || layer_mask.is_some() {
            return Err(RasterError::Scene(
                "overlap layer path masks are not yet supported".into(),
            ));
        }
        let resource = {
            let mut pool = self
                .frame_resources
                .lock()
                .map_err(|_| RasterError::Init("GPU resource pool mutex poisoned".into()))?;
            pool.entry((config.width, config.height))
                .or_insert_with(|| {
                    Arc::new(Mutex::new(GpuFrameResources::new(
                        &self.ctx.device,
                        config.width,
                        config.height,
                    )))
                })
                .clone()
        };
        let mut resources = resource
            .lock()
            .map_err(|_| RasterError::Init("GPU frame resource mutex poisoned".into()))?;
        let output_index = resources.active_index % RING_BUFFER_SIZE;
        resources.active_index = resources.active_index.wrapping_add(1);
        let layer_slot = self.offscreen_layer_slot(config.width, config.height);
        let layer_slot = layer_slot
            .lock()
            .map_err(|_| RasterError::Init("GPU layer resource mutex poisoned".into()))?;
        let output_slot = &resources.slots[output_index];
        let (base_submission, _) = self.submit_frame_to_slot(
            &base_commands,
            None,
            config.width,
            config.height,
            config.fps,
            output_slot,
            None,
            false,
        )?;
        let (layer_submission, _) = self.submit_frame_to_slot(
            &layer_commands,
            None,
            config.width,
            config.height,
            config.fps,
            &layer_slot,
            None,
            false,
        )?;
        self.ctx
            .device
            .poll(wgpu::Maintain::wait_for(base_submission));
        self.ctx
            .device
            .poll(wgpu::Maintain::wait_for(layer_submission));
        let composite_submission = self.composite_external_texture(
            &layer_slot,
            output_slot,
            config.width,
            config.height,
            layer_opacity,
        )?;
        self.ctx
            .device
            .poll(wgpu::Maintain::wait_for(composite_submission));
        let image = self.readback_resolved_slot(output_slot, config.width, config.height)?;
        self.gpu_frame_count.fetch_add(1, Ordering::Relaxed);
        Ok(image)
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

    /// Return the most recent reason a frame was rendered by the CPU fallback.
    /// This is intended for diagnostics and benchmark attribution, not for
    /// controlling rendering behavior.
    pub fn last_cpu_fallback_reason(&self) -> Option<String> {
        self.last_cpu_fallback_reason
            .lock()
            .ok()
            .and_then(|reason| reason.clone())
    }

    fn record_cpu_fallback(&self, reason: impl Into<String>) {
        self.cpu_fallback_frame_count
            .fetch_add(1, Ordering::Relaxed);
        if let Ok(mut last) = self.last_cpu_fallback_reason.lock() {
            *last = Some(reason.into());
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
        {
            let mut gpu_images = self
                .gpu_images
                .lock()
                .expect("GPU image cache lock poisoned");
            gpu_images.max_bytes = max_bytes.max(1);
            gpu_images.trim_to_budget();
        }
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
            format: RENDER_FORMAT,
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
        let bind_group = self.image_bind_group(&view, &sampler, "image_bg");
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
        cache.trim_to_budget();
        Ok(resource)
    }

    fn gpu_image(&self, src: &str) -> Result<Arc<GpuImageResource>, RasterError> {
        let decoded = self.image_cache.load(src)?;
        // ImageCache canonicalizes local paths before indexing them. Keep the
        // GPU cache on the same identity so alternate spellings of one asset
        // (relative, absolute, or file://) do not trigger duplicate uploads.
        let key = if src.trim_start().starts_with("data:") {
            src.trim().to_string()
        } else {
            canonical_local_path(src)?.display().to_string()
        };
        self.gpu_pixels(&key, &decoded)
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
                    self.text_atlas_cache_hits.fetch_add(1, Ordering::Relaxed);
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
                    self.text_atlas_dirty_uploads
                        .fetch_add(1, Ordering::Relaxed);
                    self.text_atlas_dirty_upload_bytes.fetch_add(
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
        self.text_atlas_cache_misses.fetch_add(1, Ordering::Relaxed);
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
        self.text_atlas_full_uploads.fetch_add(1, Ordering::Relaxed);
        self.text_atlas_full_upload_bytes.fetch_add(
            u64::from(snapshot.width) * u64::from(snapshot.height),
            Ordering::Relaxed,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        let bind_group = self.image_bind_group(&view, &sampler, "text_atlas_bg");
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

    /// Number of GPU text atlas snapshots reused without an upload.
    pub fn text_atlas_cache_hits(&self) -> u64 {
        self.text_atlas_cache_hits.load(Ordering::Relaxed)
    }

    /// Number of GPU text atlas allocations after the initial snapshot.
    pub fn text_atlas_cache_misses(&self) -> u64 {
        self.text_atlas_cache_misses.load(Ordering::Relaxed)
    }

    /// Number of full R8 atlas uploads since backend creation.
    pub fn text_atlas_full_uploads(&self) -> u64 {
        self.text_atlas_full_uploads.load(Ordering::Relaxed)
    }

    /// Number of dirty-rectangle atlas uploads since backend creation.
    pub fn text_atlas_dirty_uploads(&self) -> u64 {
        self.text_atlas_dirty_uploads.load(Ordering::Relaxed)
    }

    /// Number of bytes sent by full R8 atlas uploads.
    pub fn text_atlas_full_upload_bytes(&self) -> u64 {
        self.text_atlas_full_upload_bytes.load(Ordering::Relaxed)
    }

    /// Number of bytes sent by dirty-rectangle R8 atlas uploads.
    pub fn text_atlas_dirty_upload_bytes(&self) -> u64 {
        self.text_atlas_dirty_upload_bytes.load(Ordering::Relaxed)
    }

    /// Render a GPU-compatible scene without scheduling a CPU readback.
    ///
    /// The callback runs after the GPU submission has completed and while the
    /// target texture remains reserved. It is the extension point for a
    /// backend-native compositor or encoder. The texture view is borrowed for
    /// the callback only; callers must not retain it after returning.
    pub fn render_frame_gpu<F>(
        &self,
        scene: &Scene,
        config: &FrameConfig,
        consume: F,
    ) -> Result<(), RasterError>
    where
        F: FnOnce(&wgpu::TextureView, u32, u32),
    {
        if config.width > self.ctx.max_texture_dimension_2d
            || config.height > self.ctx.max_texture_dimension_2d
        {
            return Err(RasterError::Scene(
                "GPU-native frame dimensions exceed the device limit".into(),
            ));
        }
        if scene
            .nodes
            .iter()
            .any(|node| matches!(node, SceneNode::Shader { .. }))
        {
            return Err(RasterError::Scene(
                "GPU-native frame sink does not yet support shader nodes".into(),
            ));
        }
        let Some((commands, path_mask)) = compile_scene_with_path_mask(scene, &self.fallback)
        else {
            return Err(RasterError::Scene(
                "scene requires the CPU fallback and has no GPU texture handle".into(),
            ));
        };

        let resource = {
            let mut pool = self
                .frame_resources
                .lock()
                .map_err(|_| RasterError::Init("GPU resource pool mutex poisoned".into()))?;
            pool.entry((config.width, config.height))
                .or_insert_with(|| {
                    Arc::new(Mutex::new(GpuFrameResources::new(
                        &self.ctx.device,
                        config.width,
                        config.height,
                    )))
                })
                .clone()
        };
        let mut resources = resource
            .lock()
            .map_err(|_| RasterError::Init("GPU frame resource mutex poisoned".into()))?;
        let slot_idx = resources.active_index % RING_BUFFER_SIZE;
        resources.active_index = resources.active_index.wrapping_add(1);
        let (submission_index, rx) = self.submit_frame_to_slot(
            &commands,
            path_mask.as_ref(),
            config.width,
            config.height,
            config.fps,
            &resources.slots[slot_idx],
            None,
            false,
        )?;
        debug_assert!(rx.is_none());
        self.ctx
            .device
            .poll(wgpu::Maintain::wait_for(submission_index));
        consume(
            &resources.slots[slot_idx].texture_view,
            config.width,
            config.height,
        );
        self.gpu_frame_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Stream GPU-compatible frames directly to texture-view consumers.
    ///
    /// Frames are delivered in order and never copied to the CPU. The view is
    /// valid only for the duration of `consume`; a consumer that needs to
    /// retain GPU work must enqueue it before returning from the callback.
    pub fn render_stream_gpu<S, C, F>(
        &self,
        total: u32,
        scene_fn: &S,
        config_fn: &C,
        mut consume: F,
    ) -> Result<(), RasterError>
    where
        S: Fn(u32) -> Result<Scene, RasterError> + Sync,
        C: Fn(u32) -> FrameConfig + Sync,
        F: FnMut(u32, &wgpu::TextureView, u32, u32),
    {
        if total == 0 {
            return Ok(());
        }
        let first = config_fn(0);
        if first.width > self.ctx.max_texture_dimension_2d
            || first.height > self.ctx.max_texture_dimension_2d
        {
            return Err(RasterError::Scene(
                "GPU-native stream dimensions exceed the device limit".into(),
            ));
        }
        let resource = {
            let mut pool = self
                .frame_resources
                .lock()
                .map_err(|_| RasterError::Init("GPU resource pool mutex poisoned".into()))?;
            pool.entry((first.width, first.height))
                .or_insert_with(|| {
                    Arc::new(Mutex::new(GpuFrameResources::new(
                        &self.ctx.device,
                        first.width,
                        first.height,
                    )))
                })
                .clone()
        };
        let mut resources = resource
            .lock()
            .map_err(|_| RasterError::Init("GPU frame resource mutex poisoned".into()))?;
        let mut in_flight: std::collections::VecDeque<(u32, usize, wgpu::SubmissionIndex)> =
            std::collections::VecDeque::with_capacity(RING_BUFFER_SIZE);

        for frame in 0..total {
            let scene = scene_fn(frame)?;
            let config = config_fn(frame);
            if (config.width, config.height) != (first.width, first.height) {
                return Err(RasterError::Scene(
                    "GPU-native stream cannot change dimensions mid-stream".into(),
                ));
            }
            let Some((commands, path_mask)) = compile_scene_with_path_mask(&scene, &self.fallback)
            else {
                return Err(RasterError::Scene(format!(
                    "frame {frame} requires the CPU fallback"
                )));
            };
            if scene
                .nodes
                .iter()
                .any(|node| matches!(node, SceneNode::Shader { .. }))
            {
                return Err(RasterError::Scene(format!(
                    "frame {frame} contains unsupported shader nodes"
                )));
            }
            if in_flight.len() == RING_BUFFER_SIZE {
                let (queued_frame, queued_slot, submission_index) = in_flight
                    .pop_front()
                    .expect("GPU-native in-flight ring length was checked");
                self.ctx
                    .device
                    .poll(wgpu::Maintain::wait_for(submission_index));
                consume(
                    queued_frame,
                    &resources.slots[queued_slot].texture_view,
                    first.width,
                    first.height,
                );
            }
            let slot_idx = resources.active_index % RING_BUFFER_SIZE;
            resources.active_index = resources.active_index.wrapping_add(1);
            let (submission_index, rx) = self.submit_frame_to_slot(
                &commands,
                path_mask.as_ref(),
                first.width,
                first.height,
                config.fps,
                &resources.slots[slot_idx],
                None,
                false,
            )?;
            debug_assert!(rx.is_none());
            in_flight.push_back((frame, slot_idx, submission_index));
            self.gpu_frame_count.fetch_add(1, Ordering::Relaxed);
        }
        while let Some((queued_frame, queued_slot, submission_index)) = in_flight.pop_front() {
            self.ctx
                .device
                .poll(wgpu::Maintain::wait_for(submission_index));
            consume(
                queued_frame,
                &resources.slots[queued_slot].texture_view,
                first.width,
                first.height,
            );
        }
        Ok(())
    }

    fn submit_frame_to_slot(
        &self,
        commands: &[DrawCommand],
        path_mask: Option<&GpuPathMask>,
        width: u32,
        height: u32,
        sampling_fps: f64,
        slot: &GpuFrameSlot,
        shader_suffix: Option<&[SceneNode]>,
        readback: bool,
    ) -> Result<
        (
            wgpu::SubmissionIndex,
            Option<std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
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
        let atlas_snapshot = self.fallback.take_text_atlas_snapshot();
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
                DrawCommand::Lottie { key, image, .. } => {
                    (self.gpu_pixels(key, image)?, ImageFit::Contain)
                }
                DrawCommand::Gif {
                    key, image, fit, ..
                } => (self.gpu_pixels(key, image)?, *fit),
                DrawCommand::Emoji { key, image, .. } => {
                    (self.gpu_pixels(key, image)?, ImageFit::Fill)
                }
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

        let path_mask_instance_index = all_instances.len() as u32;
        if let Some(path_mask) = path_mask {
            all_instances.push(path_mask.instance);
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

            let default_path_mask_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("default_path_mask_texture"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
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
                default_path_mask_texture.as_image_copy(),
                &[255],
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(1),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            let default_path_mask_view =
                default_path_mask_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let default_path_mask_sampler =
                device.create_sampler(&wgpu::SamplerDescriptor::default());
            let default_path_mask_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("default_path_mask_bg"),
                layout: &self.ctx.path_mask_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&default_path_mask_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&default_path_mask_sampler),
                    },
                ],
            });

            let (_path_mask_texture, path_mask_view) = if path_mask.is_some() {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("path_mask_texture"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::R8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                (texture, view)
            } else {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("default_path_mask_texture"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
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
                    &[255],
                    wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(1),
                        rows_per_image: Some(1),
                    },
                    wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                );
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                (texture, view)
            };
            let path_mask_sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
            let path_mask_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("path_mask_bg"),
                layout: &self.ctx.path_mask_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&path_mask_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&path_mask_sampler),
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
                    | DrawCommand::Lottie { .. }
                    | DrawCommand::Gif { .. }
                    | DrawCommand::Emoji { .. }
                    | DrawCommand::Text { .. } => None,
                })
                .collect();

            let path_mask_buffers = path_mask.map(|mask| {
                let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("path_mask_vertices"),
                    contents: bytemuck_cast(&mask.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("path_mask_indices"),
                    contents: bytemuck_cast(&mask.indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
                (vertex_buffer, index_buffer)
            });

            if let (Some(mask), Some((vertex_buffer, index_buffer))) =
                (path_mask, path_mask_buffers.as_ref())
            {
                let mut mask_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("path_mask_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &path_mask_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                mask_pass.set_pipeline(&self.ctx.path_mask_pipeline);
                mask_pass.set_bind_group(0, &globals_bg, &[]);
                mask_pass.set_bind_group(1, &instance_bg, &[]);
                mask_pass.set_bind_group(2, &dummy_image_bg, &[]);
                mask_pass.set_bind_group(3, &default_path_mask_bg, &[]);
                mask_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                mask_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                mask_pass.draw_indexed(
                    0..mask.indices.len() as u32,
                    0,
                    path_mask_instance_index..path_mask_instance_index + 1,
                );
            }

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
            pass.set_bind_group(3, &path_mask_bg, &[]);

            let mut i = 0;
            while i < commands.len() {
                match &commands[i] {
                    DrawCommand::Analytic { .. } => {
                        let start = i;
                        while i < commands.len()
                            && matches!(commands[i], DrawCommand::Analytic { .. })
                            && commands[i].instance().kind_data[2]
                                == commands[start].instance().kind_data[2]
                        {
                            i += 1;
                        }
                        let pipeline = match commands[start].instance().kind_data[2] {
                            1 => &self.ctx.multiply_pipeline,
                            2 => &self.ctx.screen_pipeline,
                            3 => &self.ctx.darken_pipeline,
                            4 => &self.ctx.lighten_pipeline,
                            _ => &self.ctx.pipeline,
                        };
                        pass.set_pipeline(pipeline);
                        pass.draw(0..6, start as u32..i as u32);
                    }
                    DrawCommand::Mesh { indices, .. } => {
                        let (vb, ib) = mesh_buffers[i].as_ref().expect("mesh buffers allocated");
                        let pipeline = match commands[i].instance().kind_data[2] {
                            1 => &self.ctx.multiply_mesh_pipeline,
                            2 => &self.ctx.screen_mesh_pipeline,
                            3 => &self.ctx.darken_mesh_pipeline,
                            4 => &self.ctx.lighten_mesh_pipeline,
                            _ => &self.ctx.mesh_pipeline,
                        };
                        pass.set_pipeline(pipeline);
                        pass.set_vertex_buffer(0, vb.slice(..));
                        pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..indices.len() as u32, 0, i as u32..i as u32 + 1);
                        i += 1;
                    }
                    DrawCommand::Image { .. }
                    | DrawCommand::Video { .. }
                    | DrawCommand::Lottie { .. }
                    | DrawCommand::Gif { .. }
                    | DrawCommand::Emoji { .. }
                    | DrawCommand::Text { .. } => {
                        if matches!(commands[i], DrawCommand::Text { .. }) {
                            let pipeline = match commands[i].instance().kind_data[2] {
                                1 => &self.ctx.multiply_text_pipeline,
                                2 => &self.ctx.screen_text_pipeline,
                                3 => &self.ctx.darken_text_pipeline,
                                4 => &self.ctx.lighten_text_pipeline,
                                _ => &self.ctx.text_pipeline,
                            };
                            pass.set_pipeline(pipeline);
                            pass.set_bind_group(2, &atlas_resource.bind_group, &[]);
                        } else {
                            let pipeline = match commands[i].instance().kind_data[2] {
                                1 => &self.ctx.multiply_image_pipeline,
                                2 => &self.ctx.screen_image_pipeline,
                                3 => &self.ctx.darken_image_pipeline,
                                4 => &self.ctx.lighten_image_pipeline,
                                _ => &self.ctx.image_pipeline,
                            };
                            pass.set_pipeline(pipeline);
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

        if readback {
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
        }

        let submission_index = queue.submit([encoder.finish()]);

        let rx = if readback {
            let (tx, rx) = std::sync::mpsc::channel();
            slot.readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = tx.send(result);
                });
            Some(rx)
        } else {
            None
        };

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
        sink: &mut dyn FrameSink,
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
            sink.consume(in_flight.frame_idx, &data[..total_bytes])
        } else {
            scratch.clear();
            scratch.reserve_exact(total_bytes);
            for row in 0..height {
                let start = (row * bytes_per_row) as usize;
                let end = start + expected_row_bytes;
                scratch.extend_from_slice(&data[start..end]);
            }
            sink.consume(in_flight.frame_idx, scratch)
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

    fn render_stats(&self) -> BackendRenderStats {
        let stats = self.render_stats();
        let timing = self.video_timing_stats();
        BackendRenderStats {
            gpu_frames: stats.gpu_frames,
            cpu_fallback_frames: stats.cpu_fallback_frames,
            texture_cache_hits: stats.texture_cache_hits,
            texture_cache_misses: stats.texture_cache_misses,
            video_decode_ns: timing.video_decode_ns,
            texture_upload_ns: timing.texture_upload_ns,
            gpu_submit_readback_ns: timing.gpu_submit_readback_ns,
            browser_frame_ns: 0,
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
        if config.width > self.ctx.max_texture_dimension_2d
            || config.height > self.ctx.max_texture_dimension_2d
        {
            self.record_cpu_fallback("frame dimensions exceed the GPU texture limit");
            return self.fallback.render_frame(scene, config);
        }
        if let Some((base_scene, layer_scene, layer_opacity)) =
            trailing_overlap_texture_layer(scene)
        {
            if let Ok(image) =
                self.render_trailing_overlap_layer(&base_scene, &layer_scene, layer_opacity, config)
            {
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
        let Some((commands, path_mask)) = compile_scene_with_path_mask(gpu_scene, &self.fallback)
        else {
            let reason = gpu_fallback_reason(gpu_scene);
            if reason == "overlapping texture layer requires offscreen compositing" {
                // Allocate/reuse the layer target now so the eventual child
                // pass does not compete with the ordered output ring.
                let _ = self.offscreen_layer_slot(config.width, config.height);
            }
            self.record_cpu_fallback(reason);
            return self.fallback.render_frame(scene, config);
        };

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
            path_mask.as_ref(),
            width,
            height,
            config.fps,
            &res.slots[slot_idx],
            shader_suffix,
            true,
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
                rx: rx.expect("readback was requested for render_frame"),
            },
            &res,
            width,
            height,
            &mut scratch,
            &mut |_frame, pixels: &[u8]| {
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
        sink: &mut dyn FrameSink,
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
                .render_stream(total, scene_fn, config_fn, sink);
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

        // Keep the whole readback ring occupied before waiting. The previous
        // implementation retained only one in-flight frame, which serialized
        // GPU submission with map/readback and left two persistent slots idle.
        let mut in_flight = std::collections::VecDeque::with_capacity(RING_BUFFER_SIZE);
        let mut scratch = Vec::new();

        for frame in 0..total {
            let scene = scene_fn(frame)?;
            let cfg = config_fn(frame);

            let Some((commands, path_mask)) = compile_scene_with_path_mask(&scene, &self.fallback)
            else {
                while let Some(prev) = in_flight.pop_front() {
                    self.drain_slot(prev, &res, width, height, &mut scratch, sink)?;
                }
                self.record_cpu_fallback(gpu_fallback_reason(&scene));
                let img = self.fallback.render_frame(&scene, &cfg)?;
                sink.consume(frame, img.as_raw())?;
                continue;
            };

            let slot_idx = (frame as usize) % RING_BUFFER_SIZE;

            // Do not reuse a slot until its oldest submission has been
            // consumed. FIFO draining preserves the stream's frame order.
            if in_flight.len() == RING_BUFFER_SIZE {
                let prev = in_flight
                    .pop_front()
                    .expect("in-flight ring length was checked");
                debug_assert_eq!(prev.slot_idx, slot_idx);
                self.drain_slot(prev, &res, width, height, &mut scratch, sink)?;
            }

            // Submit this frame to GPU
            let (submission_index, rx) = self.submit_frame_to_slot(
                &commands,
                path_mask.as_ref(),
                width,
                height,
                cfg.fps,
                &res.slots[slot_idx],
                None,
                true,
            )?;
            self.gpu_frame_count.fetch_add(1, Ordering::Relaxed);

            in_flight.push_back(InFlight {
                frame_idx: frame,
                slot_idx,
                submission_index,
                rx: rx.expect("readback was requested for render_stream"),
            });
        }

        // Drain any remaining in-flight frame at the end of the stream
        while let Some(prev) = in_flight.pop_front() {
            self.drain_slot(prev, &res, width, height, &mut scratch, sink)?;
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
    vignette: [f32; 4],
    invert: [f32; 4],
    hue: [f32; 4],
    tint: [f32; 4],
    duotone_primary: [f32; 4],
    duotone_secondary: [f32; 4],
    grading: [f32; 4],
    grading_tint: [f32; 4],
    clip_rects: [[f32; 4]; 4],
    mask_opacity: [f32; 4],
    mask_kinds: [u32; 4],
    mask_shapes: [[f32; 4]; 4],
    mask_color0: [[f32; 4]; 4],
    mask_color1: [[f32; 4]; 4],
    mask_color2: [[f32; 4]; 4],
    mask_color3: [[f32; 4]; 4],
    mask_stop_positions: [[f32; 4]; 4],
    mask_stop_counts: [u32; 4],
    params: [f32; 4],
    opacity: [f32; 4],
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
            vignette: [0.0, 0.0, 0.0, 0.0],
            invert: [0.0, 0.0, 0.0, 0.0],
            hue: [0.0, 0.0, 0.0, 0.0],
            tint: [0.0, 0.0, 0.0, 0.0],
            duotone_primary: [0.0, 0.0, 0.0, 0.0],
            duotone_secondary: [0.0, 0.0, 0.0, 0.0],
            grading: [1.0, 1.0, 1.0, 0.0],
            grading_tint: [0.0, 0.0, 0.0, 0.0],
            clip_rects: [[-1.0; 4]; 4],
            mask_opacity: [-1.0; 4],
            mask_kinds: [0; 4],
            mask_shapes: [[0.0; 4]; 4],
            mask_color0: [[0.0; 4]; 4],
            mask_color1: [[0.0; 4]; 4],
            mask_color2: [[0.0; 4]; 4],
            mask_color3: [[0.0; 4]; 4],
            mask_stop_positions: [[0.0; 4]; 4],
            mask_stop_counts: [0; 4],
            params: [0.0, 0.0, 0.0, opacity],
            opacity: [opacity, 0.0, 0.0, 0.0],
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
    Lottie {
        instance: GpuInstance,
        key: String,
        image: Arc<image::RgbaImage>,
    },
    Gif {
        instance: GpuInstance,
        key: String,
        image: Arc<image::RgbaImage>,
        fit: ImageFit,
    },
    Emoji {
        instance: GpuInstance,
        key: String,
        image: Arc<image::RgbaImage>,
    },
    Text {
        instance: GpuInstance,
        entry: crate::text_atlas::AtlasEntry,
    },
}

struct GpuPathMask {
    instance: GpuInstance,
    vertices: Vec<GpuVertex>,
    indices: Vec<u32>,
}

impl DrawCommand {
    fn instance(&self) -> &GpuInstance {
        match self {
            Self::Analytic { instance }
            | Self::Mesh { instance, .. }
            | Self::Image { instance, .. }
            | Self::Video { instance, .. }
            | Self::Lottie { instance, .. }
            | Self::Gif { instance, .. }
            | Self::Emoji { instance, .. }
            | Self::Text { instance, .. } => instance,
        }
    }

    fn instance_mut(&mut self) -> &mut GpuInstance {
        match self {
            Self::Analytic { instance }
            | Self::Mesh { instance, .. }
            | Self::Image { instance, .. }
            | Self::Video { instance, .. }
            | Self::Lottie { instance, .. }
            | Self::Gif { instance, .. }
            | Self::Emoji { instance, .. }
            | Self::Text { instance, .. } => instance,
        }
    }
}

#[cfg(test)]
fn compile_scene(scene: &Scene, font: &TinySkiaBackend) -> Option<Vec<DrawCommand>> {
    compile_scene_with_path_mask(scene, font).map(|(commands, _)| commands)
}

fn compile_scene_with_path_mask(
    scene: &Scene,
    font: &TinySkiaBackend,
) -> Option<(Vec<DrawCommand>, Option<GpuPathMask>)> {
    let mut commands = Vec::new();
    let mut path_mask = None;
    compile_nodes(
        &scene.nodes,
        Transform::identity(),
        1.0,
        &mut commands,
        font,
        &mut path_mask,
    )?;
    if scene.nodes.iter().any(|node| {
        matches!(
            node,
            SceneNode::Layer {
                blend_mode: crate::scene::BlendMode::Multiply
                    | crate::scene::BlendMode::Screen
                    | crate::scene::BlendMode::Darken
                    | crate::scene::BlendMode::Lighten,
                ..
            }
        )
    }) {
        for command in &mut commands {
            command.instance_mut().kind_data[3] |= 2;
        }
    }
    Some((commands, path_mask))
}

fn compile_nodes(
    nodes: &[SceneNode],
    transform: Transform,
    opacity: f32,
    output: &mut Vec<DrawCommand>,
    font: &TinySkiaBackend,
    path_mask: &mut Option<GpuPathMask>,
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

            SceneNode::Lottie {
                src,
                time,
                x,
                y,
                w,
                h,
                playback_rate,
                loop_behavior,
                opacity: node_opacity,
            } => {
                if ![*x, *y, *w, *h, *playback_rate, *node_opacity]
                    .iter()
                    .all(|value| value.is_finite())
                    || !time.is_finite()
                    || *w <= 0.0
                    || *h <= 0.0
                    || *playback_rate <= 0.0
                    || *node_opacity < 0.0
                    || *node_opacity > 1.0
                {
                    return None;
                }
                let target_w = w.round().max(1.0) as u32;
                let target_h = h.round().max(1.0) as u32;
                let image = font
                    .lottie_frame(
                        src,
                        *time * f64::from(*playback_rate),
                        target_w,
                        target_h,
                        *loop_behavior,
                    )
                    .ok()?;
                let mut instance =
                    GpuInstance::solid(Color::WHITE, opacity * *node_opacity, transform);
                instance.kind_data[0] = 5;
                instance.bounds = [*x, *y, *w, *h];
                instance.shape_bounds = instance.bounds;
                instance.params = [0.0, 0.0, 1.0, 1.0];
                let key =
                    format!("lottie:{src}:{time:.9}:{playback_rate:.6}:{target_w}x{target_h}");
                output.push(DrawCommand::Lottie {
                    instance,
                    key,
                    image,
                });
            }

            SceneNode::Gif {
                src,
                time,
                x,
                y,
                w,
                h,
                loop_behavior,
                fit,
                opacity: node_opacity,
                ..
            } => {
                if ![*x, *y, *w, *h, *node_opacity]
                    .iter()
                    .all(|value| value.is_finite())
                    || *w <= 0.0
                    || *h <= 0.0
                    || !time.is_finite()
                    || *time < 0.0
                    || *node_opacity < 0.0
                    || *node_opacity > 1.0
                {
                    return None;
                }
                let Some(image) = font.gif_frame(src, *time, *loop_behavior).ok().flatten() else {
                    return None;
                };
                let mut instance =
                    GpuInstance::solid(Color::WHITE, opacity * *node_opacity, transform);
                instance.kind_data[0] = 5;
                instance.bounds = [*x, *y, *w, *h];
                instance.shape_bounds = instance.bounds;
                instance.params = [0.0, 0.0, 1.0, 1.0];
                let key = format!("gif:{src}:{time:.9}");
                output.push(DrawCommand::Gif {
                    instance,
                    key,
                    image,
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
                font_weight,
            } => {
                if ![*x, *y, *font_size].iter().all(|v| v.is_finite()) || *font_size <= 0.0 {
                    return None;
                }
                let rendered =
                    font.rasterize_text(content, *font_size, *font_weight, font_sources)?;
                let mut instance = GpuInstance::solid(*color, opacity, transform);
                instance.kind_data[0] = 6;
                instance.bounds = [
                    *x,
                    *y - rendered.baseline as f32,
                    rendered.width as f32,
                    rendered.height as f32,
                ];
                instance.shape_bounds = instance.bounds;
                let key = format!(
                    "{}:{}:{}:{:?}",
                    content,
                    font_size.to_bits(),
                    font_weight,
                    font_sources
                );
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
                    path_mask,
                )?;
            }

            SceneNode::Layer {
                opacity: layer_opacity,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: Some(crate::scene::ClipRegion::Path { d }),
                mask: None,
                mask_mode: crate::scene::MaskMode::Alpha,
                filters,
                shadow: None,
                children,
                ..
            } if path_mask.is_none()
                && gpu_layer_effects(filters, *layer_opacity).is_some()
                && gpu_path_mask_from_svg(d, transform, Color::WHITE).is_some() =>
            {
                let (layer_opacity, brightness, grayscale, contrast, saturation, invert, hue) =
                    gpu_layer_effects(filters, *layer_opacity).unwrap();
                let start = output.len();
                compile_nodes(
                    children,
                    transform,
                    opacity * layer_opacity,
                    output,
                    font,
                    path_mask,
                )?;
                *path_mask = gpu_path_mask_from_svg(d, transform, Color::WHITE);
                for command in &mut output[start..] {
                    let instance = command.instance_mut();
                    instance.kind_data[3] = 1;
                    instance.brightness[0] *= brightness;
                    instance.grayscale[0] = 1.0 - (1.0 - instance.grayscale[0]) * (1.0 - grayscale);
                    instance.contrast[0] *= contrast;
                    instance.saturation[0] *= saturation;
                    instance.invert[0] = invert;
                    instance.hue[0] = hue;
                    if let Some(tint) = gpu_tint(filters) {
                        instance.tint = tint;
                    }
                    if let Some((primary, secondary)) = gpu_duotone(filters) {
                        instance.duotone_primary = primary;
                        instance.duotone_secondary = secondary;
                    }
                    if let Some((grading, grading_tint)) = gpu_color_grading(filters) {
                        instance.grading = grading;
                        instance.grading_tint = grading_tint;
                    }
                    if let Some(vignette) = gpu_vignette(filters) {
                        instance.vignette = vignette;
                    }
                }
            }

            SceneNode::Layer {
                opacity: layer_opacity,
                clip: None,
                mask: Some(mask_nodes),
                blend_mode,
                mask_mode,
                filters,
                shadow: None,
                children,
                ..
            } if path_mask.is_none()
                && mask_nodes.len() == 1
                && matches!(mask_nodes.first(), Some(SceneNode::Path { .. }))
                && (*mask_mode == crate::scene::MaskMode::Alpha
                    || *mask_mode == crate::scene::MaskMode::Luminance)
                && matches!(
                    blend_mode,
                    crate::scene::BlendMode::Normal | crate::scene::BlendMode::Multiply
                )
                && (matches!(blend_mode, crate::scene::BlendMode::Normal)
                    || gpu_blend_layer_filters_supported(filters))
                && gpu_blend_children_supported(children)
                && gpu_layer_effects(filters, *layer_opacity).is_some()
                && gpu_path_mask_from_node(mask_nodes.first().unwrap(), transform, *mask_mode)
                    .is_some() =>
            {
                let (layer_opacity, brightness, grayscale, contrast, saturation, invert, hue) =
                    gpu_layer_effects(filters, *layer_opacity).unwrap();
                let start = output.len();
                compile_nodes(
                    children,
                    transform,
                    opacity * layer_opacity,
                    output,
                    font,
                    path_mask,
                )?;
                *path_mask =
                    gpu_path_mask_from_node(mask_nodes.first().unwrap(), transform, *mask_mode);
                for command in &mut output[start..] {
                    let instance = command.instance_mut();
                    instance.kind_data[3] = 1;
                    instance.kind_data[2] = match blend_mode {
                        crate::scene::BlendMode::Multiply => 1,
                        crate::scene::BlendMode::Normal => 0,
                        _ => return None,
                    };
                    instance.brightness[0] *= brightness;
                    instance.grayscale[0] = 1.0 - (1.0 - instance.grayscale[0]) * (1.0 - grayscale);
                    instance.contrast[0] *= contrast;
                    instance.saturation[0] *= saturation;
                    instance.invert[0] = invert;
                    instance.hue[0] = hue;
                    if let Some(tint) = gpu_tint(filters) {
                        instance.tint = tint;
                    }
                    if let Some((primary, secondary)) = gpu_duotone(filters) {
                        instance.duotone_primary = primary;
                        instance.duotone_secondary = secondary;
                    }
                    if let Some((grading, grading_tint)) = gpu_color_grading(filters) {
                        instance.grading = grading;
                        instance.grading_tint = grading_tint;
                    }
                    if let Some(vignette) = gpu_vignette(filters) {
                        instance.vignette = vignette;
                    }
                }
            }

            // A layer with no offscreen-only effect is semantically just a
            // group of GPU instances. Keep opacity and brightness filters on
            // the GPU while preserving child order; complex layers still take
            // the fallback path below so their compositing semantics remain exact.
            SceneNode::Layer {
                opacity: layer_opacity,
                blend_mode,
                clip,
                mask,
                mask_mode,
                filters,
                shadow: None,
                children,
                ..
            } if gpu_layer_effects(filters, *layer_opacity).is_some()
                && matches!(
                    blend_mode,
                    crate::scene::BlendMode::Normal
                        | crate::scene::BlendMode::Multiply
                        | crate::scene::BlendMode::Screen
                        | crate::scene::BlendMode::Darken
                        | crate::scene::BlendMode::Lighten
                )
                && ((matches!(blend_mode, crate::scene::BlendMode::Normal)
                    && (*layer_opacity >= 1.0
                        || gpu_normal_texture_layer_supported(children, transform)
                        || gpu_blend_children_supported(children)))
                    || (!matches!(blend_mode, crate::scene::BlendMode::Normal)
                        && gpu_blend_layer_filters_supported(filters)
                        && gpu_blend_children_supported(children)))
                && (*mask_mode == crate::scene::MaskMode::Alpha
                    || *mask_mode == crate::scene::MaskMode::Luminance)
                && (clip.is_none()
                    || gpu_clip_mask_info(clip.as_ref().unwrap(), transform).is_some())
                && (mask.is_none()
                    || gpu_mask_shapes(mask.as_deref().unwrap_or(&[]), transform, *mask_mode)
                        .is_some()) =>
            {
                let (layer_opacity, brightness, grayscale, contrast, saturation, invert, hue) =
                    gpu_layer_effects(filters, *layer_opacity).unwrap();
                let mask_info = mask
                    .as_deref()
                    .and_then(|nodes| gpu_mask_shapes(nodes, transform, *mask_mode));
                let mask_info = match (
                    mask_info,
                    clip.as_ref()
                        .and_then(|value| gpu_clip_mask_info(value, transform)),
                ) {
                    (Some(mask_info), Some(clip_info)) => merge_gpu_mask_info(mask_info, clip_info),
                    (Some(mask_info), None) => Some(mask_info),
                    (None, Some(clip_info)) => Some(clip_info),
                    (None, None) => None,
                };
                let start = output.len();
                compile_nodes(
                    children,
                    transform,
                    opacity * layer_opacity,
                    output,
                    font,
                    path_mask,
                )?;
                for command in &mut output[start..] {
                    let instance = command.instance_mut();
                    instance.kind_data[2] = match blend_mode {
                        crate::scene::BlendMode::Multiply => 1,
                        crate::scene::BlendMode::Screen => 2,
                        crate::scene::BlendMode::Darken => 3,
                        crate::scene::BlendMode::Lighten => 4,
                        _ => 0,
                    };
                    instance.brightness[0] *= brightness;
                    instance.grayscale[0] = 1.0 - (1.0 - instance.grayscale[0]) * (1.0 - grayscale);
                    instance.contrast[0] *= contrast;
                    instance.saturation[0] *= saturation;
                    instance.invert[0] = invert;
                    instance.hue[0] = hue;
                    if let Some(tint) = gpu_tint(filters) {
                        instance.tint = tint;
                    }
                    if let Some((primary, secondary)) = gpu_duotone(filters) {
                        instance.duotone_primary = primary;
                        instance.duotone_secondary = secondary;
                    }
                    if let Some((grading, grading_tint)) = gpu_color_grading(filters) {
                        instance.grading = grading;
                        instance.grading_tint = grading_tint;
                    }
                    if let Some(vignette) = gpu_vignette(filters) {
                        instance.vignette = vignette;
                    }
                    if let Some(mask_info) = mask_info {
                        apply_gpu_mask_info(instance, mask_info);
                    }
                }
            }

            SceneNode::Layer {
                opacity: layer_opacity,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: Some(clip),
                mask: None,
                mask_mode: crate::scene::MaskMode::Alpha,
                filters,
                shadow: None,
                children,
                ..
            } if gpu_layer_effects(filters, *layer_opacity).is_some()
                && gpu_clip_mask_info(clip, transform).is_some() =>
            {
                let (layer_opacity, brightness, grayscale, contrast, saturation, invert, hue) =
                    gpu_layer_effects(filters, *layer_opacity).unwrap();
                let mask_info = gpu_clip_mask_info(clip, transform);
                let start = output.len();
                compile_nodes(
                    children,
                    transform,
                    opacity * layer_opacity,
                    output,
                    font,
                    path_mask,
                )?;
                for command in &mut output[start..] {
                    let instance = command.instance_mut();
                    instance.brightness[0] *= brightness;
                    instance.grayscale[0] = 1.0 - (1.0 - instance.grayscale[0]) * (1.0 - grayscale);
                    instance.contrast[0] *= contrast;
                    instance.saturation[0] *= saturation;
                    instance.invert[0] = invert;
                    instance.hue[0] = hue;
                    if let Some(tint) = gpu_tint(filters) {
                        instance.tint = tint;
                    }
                    if let Some((primary, secondary)) = gpu_duotone(filters) {
                        instance.duotone_primary = primary;
                        instance.duotone_secondary = secondary;
                    }
                    if let Some((grading, grading_tint)) = gpu_color_grading(filters) {
                        instance.grading = grading;
                        instance.grading_tint = grading_tint;
                    }
                    if let Some(vignette) = gpu_vignette(filters) {
                        instance.vignette = vignette;
                    }
                    if let Some(mask_info) = mask_info {
                        apply_gpu_mask_info(instance, mask_info);
                    }
                }
            }

            SceneNode::Emoji {
                emoji,
                x,
                y,
                size,
                opacity: node_opacity,
            } => {
                if ![*x, *y, *size, *node_opacity]
                    .iter()
                    .all(|value| value.is_finite())
                    || *size <= 0.0
                    || *node_opacity < 0.0
                    || *node_opacity > 1.0
                {
                    return None;
                }
                let size_px = size.round().clamp(8.0, 1024.0) as u32;
                let image = crate::emoji::render_emoji(emoji, size_px)?;
                let mut instance =
                    GpuInstance::solid(Color::WHITE, opacity * *node_opacity, transform);
                instance.kind_data[0] = 5;
                instance.bounds = [*x, *y, *size, *size];
                instance.shape_bounds = instance.bounds;
                instance.params = [0.0, 0.0, 1.0, 1.0];
                output.push(DrawCommand::Emoji {
                    instance,
                    key: format!("emoji:{emoji}:{size_px}"),
                    image,
                });
            }

            SceneNode::Audio { .. } => {}
            SceneNode::Layer { .. }
            | SceneNode::AudioVisualizer { .. }
            | SceneNode::Shader { .. } => return None,
        }
    }
    Some(())
}

fn gpu_fallback_reason(scene: &Scene) -> &'static str {
    if scene.nodes.iter().any(|node| {
        matches!(
            node,
            SceneNode::Layer {
                opacity,
                blend_mode: crate::scene::BlendMode::Normal,
                children,
                ..
            } if *opacity < 1.0
                && !gpu_normal_texture_layer_supported(children, Transform::identity())
                && children.iter().any(|child| matches!(
                    child,
                    SceneNode::Image { .. }
                        | SceneNode::Video { .. }
                        | SceneNode::Lottie { .. }
                        | SceneNode::Group { .. }
                ))
        )
    }) {
        return "overlapping texture layer requires offscreen compositing";
    }
    "scene contains GPU-unsupported nodes or effects"
}

fn gpu_layer_effects(
    filters: &[crate::scene::SceneFilter],
    layer_opacity: f32,
) -> Option<(f32, f32, f32, f32, f32, f32, f32)> {
    if !layer_opacity.is_finite() {
        return None;
    }
    if filters
        .iter()
        .any(|filter| matches!(filter, crate::scene::SceneFilter::ColorGrading { .. }))
        && filters.iter().any(|filter| {
            matches!(
                filter,
                crate::scene::SceneFilter::Brightness { .. }
                    | crate::scene::SceneFilter::Grayscale { .. }
                    | crate::scene::SceneFilter::Contrast { .. }
                    | crate::scene::SceneFilter::Saturation { .. }
                    | crate::scene::SceneFilter::Invert { .. }
                    | crate::scene::SceneFilter::HueRotate { .. }
                    | crate::scene::SceneFilter::Tint { .. }
                    | crate::scene::SceneFilter::Duotone { .. }
            )
        })
    {
        // ColorGrading has its own gamma/contrast/saturation/tint order in
        // TinySkia. Do not claim GPU support for a chain whose CPU order
        // cannot be represented by the packed instance fields.
        return None;
    }
    filters.iter().try_fold(
        (layer_opacity, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0),
        |(opacity, brightness, grayscale, contrast, saturation, invert, hue), filter| match filter {
            crate::scene::SceneFilter::Opacity { amount }
                if amount.is_finite() && (0.0..=1.0).contains(amount) =>
            {
                Some((
                    opacity * amount,
                    brightness,
                    grayscale,
                    contrast,
                    saturation,
                    invert,
                    hue,
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
                    invert,
                    hue,
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
                    invert,
                    hue,
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
                    invert,
                    hue,
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
                    invert,
                    hue,
                ))
            }
            crate::scene::SceneFilter::Vignette {
                offset,
                darkness,
                roundness,
            } if offset.is_finite()
                && darkness.is_finite()
                && roundness.is_finite()
                && (0.0..=2.0).contains(offset)
                && (0.0..=1.0).contains(darkness)
                && (0.0..=1.0).contains(roundness) =>
            {
                Some((
                    opacity, brightness, grayscale, contrast, saturation, invert, hue,
                ))
            }
            crate::scene::SceneFilter::Invert { amount }
                if amount.is_finite() && (0.0..=1.0).contains(amount) =>
            {
                // Applying two invert filters sequentially composes their
                // affine color transforms: I_b(I_a(x)) has amount
                // a + b - 2ab. Preserve that composition instead of
                // silently dropping every preceding invert filter.
                Some((
                    opacity,
                    brightness,
                    grayscale,
                    contrast,
                    saturation,
                    invert + *amount - 2.0 * invert * *amount,
                    hue,
                ))
            }
            crate::scene::SceneFilter::HueRotate { degrees } if degrees.is_finite() => Some((
                opacity,
                brightness,
                grayscale,
                contrast,
                saturation,
                invert,
                hue + *degrees,
            )),
            crate::scene::SceneFilter::Tint { color, amount }
                if amount.is_finite() && (0.0..=1.0).contains(amount) =>
            {
                Some((
                    opacity, brightness, grayscale, contrast, saturation, invert, hue,
                ))
            }
            crate::scene::SceneFilter::Duotone { .. } => Some((
                opacity, brightness, grayscale, contrast, saturation, invert, hue,
            )),
            crate::scene::SceneFilter::ColorGrading {
                contrast: grading_contrast,
                saturation: grading_saturation,
                gamma,
                ..
            } if grading_contrast.is_finite()
                && *grading_contrast >= 0.0
                && grading_saturation.is_finite()
                && *grading_saturation >= 0.0
                && gamma.is_finite()
                && *gamma > 0.0 =>
            {
                Some((
                    opacity, brightness, grayscale, contrast, saturation, invert, hue,
                ))
            }
            _ => None,
        },
    )
}

/// Non-normal blend modes operate directly on the ordered child draw calls,
/// rather than an offscreen layer. Opacity is safe in that representation
/// because it composes into each child instance; color-changing filters are
/// not, since they require filtering the already-composited layer.
fn gpu_blend_layer_filters_supported(filters: &[crate::scene::SceneFilter]) -> bool {
    filters.iter().all(|filter| {
        matches!(
            filter,
            crate::scene::SceneFilter::Opacity { amount }
                if amount.is_finite() && (0.0..=1.0).contains(amount)
        )
    })
}

fn gpu_tint(filters: &[crate::scene::SceneFilter]) -> Option<[f32; 4]> {
    let mut tint = None;
    for filter in filters {
        if let crate::scene::SceneFilter::Tint { color, amount } = filter {
            if !amount.is_finite() || !(0.0..=1.0).contains(amount) {
                return None;
            }
            tint = Some([
                f32::from(color[0]) / 255.0,
                f32::from(color[1]) / 255.0,
                f32::from(color[2]) / 255.0,
                f32::from(color[3]) / 255.0 * *amount,
            ]);
        }
    }
    tint
}

fn gpu_duotone(filters: &[crate::scene::SceneFilter]) -> Option<([f32; 4], [f32; 4])> {
    filters.iter().find_map(|filter| {
        if let crate::scene::SceneFilter::Duotone { primary, secondary } = filter {
            Some((
                [
                    f32::from(primary[0]) / 255.0,
                    f32::from(primary[1]) / 255.0,
                    f32::from(primary[2]) / 255.0,
                    1.0,
                ],
                [
                    f32::from(secondary[0]) / 255.0,
                    f32::from(secondary[1]) / 255.0,
                    f32::from(secondary[2]) / 255.0,
                    0.0,
                ],
            ))
        } else {
            None
        }
    })
}

fn gpu_color_grading(filters: &[crate::scene::SceneFilter]) -> Option<([f32; 4], [f32; 4])> {
    filters.iter().find_map(|filter| {
        if let crate::scene::SceneFilter::ColorGrading {
            contrast,
            saturation,
            gamma,
            tint,
        } = filter
        {
            if !contrast.is_finite()
                || *contrast < 0.0
                || !saturation.is_finite()
                || *saturation < 0.0
                || !gamma.is_finite()
                || *gamma <= 0.0
            {
                return None;
            }
            let tint = tint.map_or([0.0; 4], |color| {
                [
                    f32::from(color[0]) / 255.0,
                    f32::from(color[1]) / 255.0,
                    f32::from(color[2]) / 255.0,
                    f32::from(color[3]) / 255.0,
                ]
            });
            Some(([*contrast, *saturation, 1.0 / *gamma, 1.0], tint))
        } else {
            None
        }
    })
}

fn gpu_blend_children_supported(nodes: &[SceneNode]) -> bool {
    nodes.iter().all(|node| match node {
        SceneNode::Rect { .. }
        | SceneNode::Circle { .. }
        | SceneNode::Path { .. }
        | SceneNode::LinearGradient { .. }
        | SceneNode::RadialGradient { .. }
        | SceneNode::Text { .. } => true,
        SceneNode::Group { children, .. } => gpu_blend_children_supported(children),
        SceneNode::Layer { .. }
        | SceneNode::Image { .. }
        | SceneNode::Video { .. }
        | SceneNode::Lottie { .. }
        | SceneNode::Gif { .. }
        | SceneNode::Emoji { .. }
        | SceneNode::Audio { .. }
        | SceneNode::AudioVisualizer { .. }
        | SceneNode::Shader { .. } => false,
    })
}

/// Texture children can carry the layer opacity directly on their GPU
/// instances when their axis-aligned bounds do not overlap. In that case the
/// group opacity cannot change a pixel more than once, so this is equivalent
/// to compositing the group into an offscreen surface first. Overlapping or
/// transformed children still require an offscreen layer.
fn gpu_normal_texture_layer_supported(nodes: &[SceneNode], parent: Transform) -> bool {
    let mut bounds = Vec::with_capacity(nodes.len());
    fn collect(nodes: &[SceneNode], transform: Transform, bounds: &mut Vec<[f32; 4]>) -> bool {
        for node in nodes {
            match node {
                SceneNode::Image { x, y, w, h, .. }
                | SceneNode::Video { x, y, w, h, .. }
                | SceneNode::Lottie { x, y, w, h, .. } => {
                    if ![*x, *y, *w, *h].iter().all(|value| value.is_finite())
                        || *w <= 0.0
                        || *h <= 0.0
                    {
                        return false;
                    }
                    let mut corners = [
                        tiny_skia::Point::from_xy(*x, *y),
                        tiny_skia::Point::from_xy(*x + *w, *y),
                        tiny_skia::Point::from_xy(*x, *y + *h),
                        tiny_skia::Point::from_xy(*x + *w, *y + *h),
                    ];
                    for corner in &mut corners {
                        transform.map_point(corner);
                    }
                    let min_x = corners
                        .iter()
                        .map(|point| point.x)
                        .fold(f32::INFINITY, f32::min);
                    let min_y = corners
                        .iter()
                        .map(|point| point.y)
                        .fold(f32::INFINITY, f32::min);
                    let max_x = corners
                        .iter()
                        .map(|point| point.x)
                        .fold(f32::NEG_INFINITY, f32::max);
                    let max_y = corners
                        .iter()
                        .map(|point| point.y)
                        .fold(f32::NEG_INFINITY, f32::max);
                    if ![min_x, min_y, max_x, max_y]
                        .iter()
                        .all(|value| value.is_finite())
                    {
                        return false;
                    }
                    bounds.push([min_x, min_y, max_x - min_x, max_y - min_y]);
                }
                SceneNode::Group {
                    transform: group_transform,
                    children,
                    ..
                } => {
                    if !collect(
                        children,
                        transform.post_concat(group_transform.to_tiny_skia()),
                        bounds,
                    ) {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        true
    }
    if !collect(nodes, parent, &mut bounds) {
        return false;
    }
    if bounds.is_empty() {
        return false;
    }
    for (index, first) in bounds.iter().enumerate() {
        for second in &bounds[index + 1..] {
            let separated = first[0] + first[2] <= second[0]
                || second[0] + second[2] <= first[0]
                || first[1] + first[3] <= second[1]
                || second[1] + second[3] <= first[1];
            if !separated {
                return false;
            }
        }
    }
    true
}

/// Return the narrow overlap case that can be rendered with one offscreen
/// texture and one explicit composite pass. More complex layer semantics stay
/// on the CPU path until their ordering and masking rules are implemented.
fn trailing_overlap_texture_layer(scene: &Scene) -> Option<(Scene, Scene, f32)> {
    let SceneNode::Layer {
        opacity,
        blend_mode: crate::scene::BlendMode::Normal,
        clip: None,
        mask: None,
        filters,
        shadow: None,
        children,
        ..
    } = scene.nodes.last()?
    else {
        return None;
    };
    if *opacity >= 1.0
        || !opacity.is_finite()
        || !filters.iter().all(|filter| {
            matches!(
                filter,
                crate::scene::SceneFilter::Opacity { amount }
                    if amount.is_finite() && (0.0..=1.0).contains(amount)
            )
        })
        || children.len() < 2
        || gpu_normal_texture_layer_supported(children, Transform::identity())
    {
        return None;
    }
    if !children.iter().any(|node| {
        matches!(
            node,
            SceneNode::Image { .. }
                | SceneNode::Video { .. }
                | SceneNode::Lottie { .. }
                | SceneNode::Group { .. }
        )
    }) {
        return None;
    }
    Some((
        Scene {
            nodes: scene.nodes[..scene.nodes.len() - 1].to_vec(),
        },
        Scene {
            nodes: children.clone(),
        },
        filters
            .iter()
            .fold(*opacity, |opacity, filter| match filter {
                crate::scene::SceneFilter::Opacity { amount } => opacity * amount,
                _ => opacity,
            }),
    ))
}

fn gpu_vignette(filters: &[crate::scene::SceneFilter]) -> Option<[f32; 4]> {
    let mut result = None;
    for filter in filters {
        if let crate::scene::SceneFilter::Vignette {
            offset,
            darkness,
            roundness,
        } = filter
        {
            if !offset.is_finite()
                || !darkness.is_finite()
                || !roundness.is_finite()
                || !(0.0..=2.0).contains(offset)
                || !(0.0..=1.0).contains(darkness)
                || !(0.0..=1.0).contains(roundness)
            {
                return None;
            }
            result = Some([*offset, *darkness, *roundness, 1.0]);
        }
    }
    result
}

type GpuMaskInfo = (
    [[f32; 4]; 4],
    [f32; 4],
    [u32; 4],
    [[f32; 4]; 4],
    [[f32; 4]; 4],
    [[f32; 4]; 4],
    [[f32; 4]; 4],
    [[f32; 4]; 4],
    [[f32; 4]; 4],
    [u32; 4],
);

fn gpu_clip_mask_info(
    clip: &crate::scene::ClipRegion,
    transform: Transform,
) -> Option<GpuMaskInfo> {
    let crate::scene::ClipRegion::Rect {
        x,
        y,
        w,
        h,
        corner_radius,
    } = clip
    else {
        return None;
    };
    let node = SceneNode::Rect {
        x: *x,
        y: *y,
        w: *w,
        h: *h,
        fill: Color::WHITE,
        stroke: None,
        stroke_width: 0.0,
        corner_radius: *corner_radius,
    };
    let mut info = gpu_mask_shapes(&[node], transform, crate::scene::MaskMode::Alpha)?;
    for kind in &mut info.2 {
        *kind = 6;
    }
    Some(info)
}

fn merge_gpu_mask_info(mut base: GpuMaskInfo, extra: GpuMaskInfo) -> Option<GpuMaskInfo> {
    let (
        base_rects,
        base_opacity,
        base_kinds,
        base_shapes,
        base_color0,
        base_color1,
        base_color2,
        base_color3,
        base_positions,
        base_counts,
    ) = &mut base;
    let (
        extra_rects,
        extra_opacity,
        extra_kinds,
        extra_shapes,
        extra_color0,
        extra_color1,
        extra_color2,
        extra_color3,
        extra_positions,
        extra_counts,
    ) = extra;
    for index in 0..4 {
        if extra_opacity[index] < 0.0 {
            continue;
        }
        let Some(slot) = base_opacity.iter().position(|opacity| *opacity < 0.0) else {
            return None;
        };
        base_rects[slot] = extra_rects[index];
        base_opacity[slot] = extra_opacity[index];
        base_kinds[slot] = extra_kinds[index];
        base_shapes[slot] = extra_shapes[index];
        base_color0[slot] = extra_color0[index];
        base_color1[slot] = extra_color1[index];
        base_color2[slot] = extra_color2[index];
        base_color3[slot] = extra_color3[index];
        base_positions[slot] = extra_positions[index];
        base_counts[slot] = extra_counts[index];
    }
    Some(base)
}

fn apply_gpu_mask_info(instance: &mut GpuInstance, info: GpuMaskInfo) {
    let (
        clip_rects,
        mask_opacity,
        mask_kinds,
        mask_shapes,
        mask_color0,
        mask_color1,
        mask_color2,
        mask_color3,
        mask_stop_positions,
        mask_stop_counts,
    ) = info;
    instance.clip_rects = clip_rects;
    instance.mask_opacity = mask_opacity;
    instance.mask_kinds = mask_kinds;
    instance.mask_shapes = mask_shapes;
    instance.mask_color0 = mask_color0;
    instance.mask_color1 = mask_color1;
    instance.mask_color2 = mask_color2;
    instance.mask_color3 = mask_color3;
    instance.mask_stop_positions = mask_stop_positions;
    instance.mask_stop_counts = mask_stop_counts;
}

fn valid_gpu_gradient_stops(stops: &[crate::scene::GradientStop]) -> bool {
    if !(2..=4).contains(&stops.len())
        || (stops.first().map(|stop| stop.position) != Some(0.0))
        || (stops.last().map(|stop| stop.position) != Some(1.0))
    {
        return false;
    }
    stops.windows(2).all(|pair| {
        pair[0].position.is_finite()
            && pair[1].position.is_finite()
            && pair[0].position <= pair[1].position
    })
}

fn gpu_mask_shapes(
    mask: &[SceneNode],
    transform: Transform,
    mask_mode: crate::scene::MaskMode,
) -> Option<GpuMaskInfo> {
    if mask.is_empty() || mask.len() > 4 {
        return None;
    }
    let mut rects = [[-1.0; 4]; 4];
    let mut opacities = [-1.0; 4];
    let mut kinds = [0; 4];
    let mut shapes = [[0.0; 4]; 4];
    let mut colors0 = [[0.0; 4]; 4];
    let mut colors1 = [[0.0; 4]; 4];
    let mut colors2 = [[0.0; 4]; 4];
    let mut colors3 = [[0.0; 4]; 4];
    let mut stop_positions = [[0.0; 4]; 4];
    let mut stop_counts = [0; 4];
    for (index, node) in mask.iter().enumerate() {
        let (x, y, w, h, fill, kind, shape, color0, color1, color2, color3, positions, count) =
            match node {
                SceneNode::Rect {
                    x,
                    y,
                    w,
                    h,
                    fill,
                    stroke: None,
                    stroke_width,
                    corner_radius,
                } if [*x, *y, *w, *h, *stroke_width, *corner_radius]
                    .iter()
                    .all(|v| v.is_finite())
                    && *w > 0.0
                    && *h > 0.0
                    && *stroke_width == 0.0
                    && *corner_radius >= 0.0 =>
                {
                    let kind = if *corner_radius > 0.0 { 5 } else { 0 };
                    let shape = if kind == 5 {
                        [*corner_radius, 0.0, 0.0, 0.0]
                    } else {
                        [0.0; 4]
                    };
                    (
                        *x, *y, *w, *h, *fill, kind, shape, [0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4],
                        [0.0; 4], 0,
                    )
                }
                SceneNode::Circle {
                    cx,
                    cy,
                    r,
                    fill,
                    stroke: None,
                    stroke_width,
                } if [*cx, *cy, *r, *stroke_width].iter().all(|v| v.is_finite())
                    && *r > 0.0
                    && *stroke_width == 0.0 =>
                {
                    (
                        *cx - *r,
                        *cy - *r,
                        *r * 2.0,
                        *r * 2.0,
                        *fill,
                        1,
                        [*cx, *cy, *r, 0.0],
                        [0.0; 4],
                        [0.0; 4],
                        [0.0; 4],
                        [0.0; 4],
                        [0.0; 4],
                        0,
                    )
                }
                SceneNode::LinearGradient {
                    x,
                    y,
                    w,
                    h,
                    angle_deg,
                    stops,
                } if [*x, *y, *w, *h, *angle_deg].iter().all(|v| v.is_finite())
                    && *w > 0.0
                    && *h > 0.0
                    && (2..=4).contains(&stops.len()) =>
                {
                    if !valid_gpu_gradient_stops(stops) {
                        return None;
                    }
                    let mut colors = [[0.0; 4]; 4];
                    let mut positions = [0.0; 4];
                    for (index, stop) in stops.iter().enumerate() {
                        colors[index] = color_to_f32(stop.color);
                        positions[index] = stop.position;
                    }
                    let half_diag = (*w * *w + *h * *h).sqrt() / 2.0;
                    let cx = *x + *w / 2.0;
                    let cy = *y + *h / 2.0;
                    let angle_rad = angle_deg.to_radians();
                    let dx = angle_rad.sin() * half_diag;
                    let dy = angle_rad.cos() * half_diag;
                    (
                        *x,
                        *y,
                        *w,
                        *h,
                        Color::WHITE,
                        2,
                        [cx - dx, cy - dy, cx + dx, cy + dy],
                        colors[0],
                        colors[1],
                        colors[2],
                        colors[3],
                        positions,
                        stops.len() as u32,
                    )
                }
                SceneNode::RadialGradient { cx, cy, r, stops }
                    if [*cx, *cy, *r].iter().all(|v| v.is_finite())
                        && *r > 0.0
                        && (2..=4).contains(&stops.len()) =>
                {
                    if !valid_gpu_gradient_stops(stops) {
                        return None;
                    }
                    let mut colors = [[0.0; 4]; 4];
                    let mut positions = [0.0; 4];
                    for (index, stop) in stops.iter().enumerate() {
                        colors[index] = color_to_f32(stop.color);
                        positions[index] = stop.position;
                    }
                    (
                        *cx - *r,
                        *cy - *r,
                        *r * 2.0,
                        *r * 2.0,
                        Color::WHITE,
                        4,
                        [*cx, *cy, *r, 0.0],
                        colors[0],
                        colors[1],
                        colors[2],
                        colors[3],
                        positions,
                        stops.len() as u32,
                    )
                }
                _ => return None,
            };
        let kind = if kind == 2 && mask_mode == crate::scene::MaskMode::Luminance {
            3
        } else {
            kind
        };
        let mask_opacity = match mask_mode {
            _ if kind == 2 => 1.0,
            crate::scene::MaskMode::Alpha if kind == 3 => 1.0,
            crate::scene::MaskMode::Luminance if kind == 2 => 1.0,
            crate::scene::MaskMode::Alpha => f32::from(fill.a) / 255.0,
            crate::scene::MaskMode::Luminance if fill.r == fill.g && fill.g == fill.b => {
                f32::from(fill.r) * f32::from(fill.a) / (255.0 * 255.0)
            }
            crate::scene::MaskMode::Luminance => return None,
        };
        // Translation and axis-aligned scale remain exact rectangular clips in
        // screen space. Rotation/shear falls back: a bounding box would overdraw.
        if !transform.sx.is_finite()
            || !transform.sy.is_finite()
            || !transform.tx.is_finite()
            || !transform.ty.is_finite()
            || transform.kx != 0.0
            || transform.ky != 0.0
        {
            return None;
        }
        if (kind == 1 || kind == 4 || kind == 5)
            && (transform.sx - transform.sy).abs() > f32::EPSILON
        {
            return None;
        }
        let x0 = x * transform.sx + transform.tx;
        let x1 = (x + w) * transform.sx + transform.tx;
        let y0 = y * transform.sy + transform.ty;
        let y1 = (y + h) * transform.sy + transform.ty;
        let width = (x1 - x0).abs();
        let height = (y1 - y0).abs();
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        rects[index] = [x0.min(x1), y0.min(y1), width, height];
        opacities[index] = mask_opacity;
        kinds[index] = kind;
        shapes[index] = if kind == 1 {
            [
                shape[0] * transform.sx + transform.tx,
                shape[1] * transform.sy + transform.ty,
                shape[2] * transform.sx.abs(),
                0.0,
            ]
        } else if kind == 2 || kind == 3 {
            [
                shape[0] * transform.sx + transform.tx,
                shape[1] * transform.sy + transform.ty,
                shape[2] * transform.sx + transform.tx,
                shape[3] * transform.sy + transform.ty,
            ]
        } else if kind == 4 {
            [
                shape[0] * transform.sx + transform.tx,
                shape[1] * transform.sy + transform.ty,
                shape[2] * transform.sx.abs(),
                0.0,
            ]
        } else if kind == 5 {
            [shape[0] * transform.sx.abs(), 0.0, 0.0, 0.0]
        } else {
            [0.0; 4]
        };
        colors0[index] = color0;
        colors1[index] = color1;
        colors2[index] = color2;
        colors3[index] = color3;
        stop_positions[index] = positions;
        stop_counts[index] = count;
    }
    Some((
        rects,
        opacities,
        kinds,
        shapes,
        colors0,
        colors1,
        colors2,
        colors3,
        stop_positions,
        stop_counts,
    ))
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

fn gpu_path_mask_from_svg(d: &str, transform: Transform, color: Color) -> Option<GpuPathMask> {
    let path = svgpath_to_tiny_skia(d)?;
    gpu_path_mask_from_tiny_path(&path, transform, color)
}

fn gpu_path_mask_from_node(
    node: &SceneNode,
    transform: Transform,
    mask_mode: crate::scene::MaskMode,
) -> Option<GpuPathMask> {
    let SceneNode::Path {
        d,
        fill,
        stroke,
        stroke_width,
        opacity,
    } = node
    else {
        return None;
    };
    if !stroke_width.is_finite() || !opacity.is_finite() || *opacity < 0.0 {
        return None;
    }
    let path = svgpath_to_tiny_skia(d)?;
    let (mask_path, color_source) = match (fill, stroke) {
        (Some(_), Some(_)) => return None,
        (Some(fill), None) if *stroke_width == 0.0 => (&path, fill),
        (None, Some(stroke)) if *stroke_width > 0.0 => {
            let scale = transform
                .get_scale()
                .0
                .max(transform.get_scale().1)
                .max(1.0);
            let stroked = path.stroke(
                &Stroke {
                    width: *stroke_width,
                    ..Default::default()
                },
                scale,
            )?;
            return gpu_path_mask_from_tiny_path(
                &stroked,
                transform,
                path_mask_color(*stroke, mask_mode),
            )
            .map(|mut mask| {
                mask.instance.params[3] = *opacity;
                mask
            });
        }
        _ => return None,
    };
    gpu_path_mask_from_tiny_path(
        mask_path,
        transform,
        path_mask_color(*color_source, mask_mode),
    )
    .map(|mut mask| {
        mask.instance.params[3] = *opacity;
        mask
    })
}

/// Path masks are rendered into an R8 texture through `fs_solid`, whose
/// output alpha becomes the mask value. Encode luminance in alpha here;
/// storing it only in RGB would be lost during the path-mask pass.
fn path_mask_color(color: Color, mask_mode: crate::scene::MaskMode) -> Color {
    match mask_mode {
        crate::scene::MaskMode::Alpha => Color::rgba(255, 255, 255, color.a),
        crate::scene::MaskMode::Luminance => {
            let luminance = (0.2126 * f32::from(color.r)
                + 0.7152 * f32::from(color.g)
                + 0.0722 * f32::from(color.b))
            .round()
            .clamp(0.0, 255.0) as u16;
            let alpha = (luminance * u16::from(color.a) + 127) / 255;
            Color::rgba(255, 255, 255, alpha as u8)
        }
    }
}

fn gpu_path_mask_from_tiny_path(
    path: &TinyPath,
    transform: Transform,
    color: Color,
) -> Option<GpuPathMask> {
    let lyon_path = tiny_path_to_lyon(path)?;
    let mut geometry: VertexBuffers<GpuVertex, u32> = VertexBuffers::new();
    FillTessellator::new()
        .tessellate_path(
            &lyon_path,
            &FillOptions::non_zero().with_tolerance(0.1),
            &mut BuffersBuilder::new(&mut geometry, PositionConstructor),
        )
        .ok()?;
    if geometry.vertices.is_empty() || geometry.indices.is_empty() || !transform.is_finite() {
        return None;
    }
    Some(GpuPathMask {
        instance: GpuInstance::solid(color, 1.0, transform),
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
/// Fragment colour uniforms are converted to linear light before writing to
/// the sRGB render target, matching the CPU renderer's byte-level colour API.
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
    fn disjoint_texture_layers_compile_as_gpu_opacity_groups() {
        let source = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let layer = |second_x| SceneNode::Layer {
            opacity: 0.5,
            blend_mode: crate::scene::BlendMode::Normal,
            clip: None,
            mask: None,
            mask_mode: crate::scene::MaskMode::Alpha,
            filters: Vec::new(),
            shadow: None,
            children: vec![
                SceneNode::Image {
                    src: source.into(),
                    x: 0.0,
                    y: 0.0,
                    w: 10.0,
                    h: 10.0,
                    fit: ImageFit::Fill,
                    opacity: 1.0,
                },
                SceneNode::Image {
                    src: source.into(),
                    x: second_x,
                    y: 0.0,
                    w: 10.0,
                    h: 10.0,
                    fit: ImageFit::Fill,
                    opacity: 1.0,
                },
            ],
        };
        assert!(gpu_supports_scene(&Scene {
            nodes: vec![layer(12.0)],
        }));
        assert!(!gpu_supports_scene(&Scene {
            nodes: vec![layer(8.0)],
        }));
        let transformed = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 0.5,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: None,
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: Vec::new(),
                shadow: None,
                children: vec![
                    SceneNode::Group {
                        transform: crate::scene::Transform2D::translate(0.0, 0.0),
                        opacity: 1.0,
                        children: vec![SceneNode::Image {
                            src: source.into(),
                            x: 0.0,
                            y: 0.0,
                            w: 10.0,
                            h: 10.0,
                            fit: ImageFit::Fill,
                            opacity: 1.0,
                        }],
                    },
                    SceneNode::Group {
                        transform: crate::scene::Transform2D::translate(12.0, 0.0).with_rotate(5.0),
                        opacity: 1.0,
                        children: vec![SceneNode::Image {
                            src: source.into(),
                            x: 0.0,
                            y: 0.0,
                            w: 10.0,
                            h: 10.0,
                            fit: ImageFit::Fill,
                            opacity: 1.0,
                        }],
                    },
                ],
            }],
        };
        assert!(gpu_supports_scene(&transformed));
    }

    #[test]
    fn overlapping_texture_layer_reports_offscreen_fallback_reason() {
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 0.5,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: None,
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: Vec::new(),
                shadow: None,
                children: vec![
                    SceneNode::Image {
                        src: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".into(),
                        x: 0.0,
                        y: 0.0,
                        w: 10.0,
                        h: 10.0,
                        fit: ImageFit::Fill,
                        opacity: 1.0,
                    },
                    SceneNode::Image {
                        src: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".into(),
                        x: 8.0,
                        y: 0.0,
                        w: 10.0,
                        h: 10.0,
                        fit: ImageFit::Fill,
                        opacity: 1.0,
                    },
                ],
            }],
        };
        assert_eq!(
            gpu_fallback_reason(&scene),
            "overlapping texture layer requires offscreen compositing"
        );
        let mut filtered = scene.clone();
        if let Some(SceneNode::Layer { filters, .. }) = filtered.nodes.first_mut() {
            filters.push(crate::scene::SceneFilter::Blur { sigma: 2.0 });
        }
        assert!(trailing_overlap_texture_layer(&filtered).is_none());

        let mut opacity_chain = scene.clone();
        if let Some(SceneNode::Layer { filters, .. }) = opacity_chain.nodes.first_mut() {
            filters.extend([
                crate::scene::SceneFilter::Opacity { amount: 0.8 },
                crate::scene::SceneFilter::Opacity { amount: 0.5 },
            ]);
        }
        let (_, _, effective_opacity) = trailing_overlap_texture_layer(&opacity_chain)
            .expect("opacity-only overlap layer should use GPU compositing");
        assert!((effective_opacity - 0.2).abs() < f32::EPSILON);
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
    fn offscreen_layer_pool_reuses_resolution_slots() {
        let Ok(backend) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping offscreen pool test");
            return;
        };
        let first = backend.offscreen_layer_slot(64, 32);
        let second = backend.offscreen_layer_slot(64, 32);
        let different = backend.offscreen_layer_slot(32, 32);
        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first, &different));
        let slot = first.lock().expect("offscreen layer slot lock poisoned");
        let _binding = backend.offscreen_layer_binding(&slot);
        let destination = GpuFrameSlot::new(&backend.ctx.device, 64, 32);
        let submission = backend
            .composite_external_texture(&slot, &destination, 64, 32, 0.5)
            .expect("offscreen composite submission failed");
        backend
            .ctx
            .device
            .poll(wgpu::Maintain::wait_for(submission));
    }

    #[test]
    fn gpu_plain_rects_match_cpu_pixels_exactly() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping exact plain-rect parity test");
            return;
        };
        let scene = Scene {
            nodes: vec![
                SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 128.0,
                    h: 96.0,
                    fill: Color::rgb(15, 23, 42),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                },
                SceneNode::Rect {
                    x: 16.0,
                    y: 12.0,
                    w: 48.0,
                    h: 32.0,
                    fill: Color::rgb(80, 160, 220),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                },
            ],
        };
        let config = FrameConfig::new(128, 96, 0, 30.0);
        let cpu = TinySkiaBackend::headless()
            .render_frame(&scene, &config)
            .expect("CPU render failed");
        let gpu = gpu
            .render_frame(&scene, &config)
            .expect("GPU render failed");
        assert_eq!(gpu.as_raw(), cpu.as_raw());
    }

    #[test]
    fn gpu_disjoint_texture_layer_matches_cpu_pixels() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping disjoint texture parity test");
            return;
        };
        let red_path =
            std::env::temp_dir().join(format!("dioxuscut-disjoint-red-{}.png", std::process::id()));
        let blue_path = std::env::temp_dir().join(format!(
            "dioxuscut-disjoint-blue-{}.png",
            std::process::id()
        ));
        image::RgbaImage::from_pixel(2, 2, image::Rgba([240, 32, 24, 255]))
            .save(&red_path)
            .unwrap();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([24, 64, 240, 255]))
            .save(&blue_path)
            .unwrap();
        let scene = Scene {
            nodes: vec![
                SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 64.0,
                    h: 32.0,
                    fill: Color::rgb(12, 18, 28),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                },
                SceneNode::Layer {
                    opacity: 0.5,
                    blend_mode: crate::scene::BlendMode::Normal,
                    clip: None,
                    mask: None,
                    mask_mode: crate::scene::MaskMode::Alpha,
                    filters: Vec::new(),
                    shadow: None,
                    children: vec![
                        SceneNode::Group {
                            transform: crate::scene::Transform2D::translate(4.0, 4.0),
                            opacity: 1.0,
                            children: vec![SceneNode::Image {
                                src: red_path.to_string_lossy().into_owned(),
                                x: 0.0,
                                y: 0.0,
                                w: 20.0,
                                h: 20.0,
                                fit: ImageFit::Fill,
                                opacity: 1.0,
                            }],
                        },
                        SceneNode::Group {
                            transform: crate::scene::Transform2D::translate(36.0, 4.0),
                            opacity: 1.0,
                            children: vec![SceneNode::Image {
                                src: blue_path.to_string_lossy().into_owned(),
                                x: 0.0,
                                y: 0.0,
                                w: 20.0,
                                h: 20.0,
                                fit: ImageFit::Fill,
                                opacity: 1.0,
                            }],
                        },
                    ],
                },
            ],
        };
        let config = FrameConfig::new(64, 32, 0, 30.0);
        assert!(gpu_supports_scene(&scene));
        let cpu = TinySkiaBackend::headless()
            .render_frame(&scene, &config)
            .expect("CPU render failed");
        let gpu_image = gpu
            .render_frame(&scene, &config)
            .expect("GPU render failed");
        assert_eq!(gpu_image.as_raw(), cpu.as_raw());
        let _ = std::fs::remove_file(red_path);
        let _ = std::fs::remove_file(blue_path);
    }

    #[test]
    fn gpu_overlapping_texture_layer_composites_on_gpu() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping overlapping texture parity test");
            return;
        };
        let red_path =
            std::env::temp_dir().join(format!("dioxuscut-overlap-red-{}.png", std::process::id()));
        let blue_path =
            std::env::temp_dir().join(format!("dioxuscut-overlap-blue-{}.png", std::process::id()));
        image::RgbaImage::from_pixel(2, 2, image::Rgba([240, 32, 24, 255]))
            .save(&red_path)
            .unwrap();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([24, 64, 240, 255]))
            .save(&blue_path)
            .unwrap();
        let scene = Scene {
            nodes: vec![
                SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 64.0,
                    h: 32.0,
                    fill: Color::rgb(12, 18, 28),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                },
                SceneNode::Layer {
                    opacity: 0.5,
                    blend_mode: crate::scene::BlendMode::Normal,
                    clip: None,
                    mask: None,
                    mask_mode: crate::scene::MaskMode::Alpha,
                    filters: vec![crate::scene::SceneFilter::Opacity { amount: 0.8 }],
                    shadow: None,
                    children: vec![
                        SceneNode::Group {
                            transform: crate::scene::Transform2D::translate(4.0, 4.0),
                            opacity: 1.0,
                            children: vec![SceneNode::Image {
                                src: red_path.to_string_lossy().into_owned(),
                                x: 0.0,
                                y: 0.0,
                                w: 20.0,
                                h: 20.0,
                                fit: ImageFit::Fill,
                                opacity: 1.0,
                            }],
                        },
                        SceneNode::Group {
                            transform: crate::scene::Transform2D::translate(12.0, 4.0),
                            opacity: 1.0,
                            children: vec![SceneNode::Image {
                                src: blue_path.to_string_lossy().into_owned(),
                                x: 0.0,
                                y: 0.0,
                                w: 20.0,
                                h: 20.0,
                                fit: ImageFit::Fill,
                                opacity: 1.0,
                            }],
                        },
                    ],
                },
            ],
        };
        let config = FrameConfig::new(64, 32, 0, 30.0);
        let cpu = TinySkiaBackend::headless()
            .render_frame(&scene, &config)
            .expect("CPU render failed");
        let gpu_image = gpu
            .render_frame(&scene, &config)
            .expect("GPU render failed");
        assert_eq!(gpu_image.as_raw(), cpu.as_raw());
        let stats = gpu.render_stats();
        assert_eq!(stats.gpu_frames, 1);
        assert_eq!(stats.cpu_fallback_frames, 0);
        let _ = std::fs::remove_file(red_path);
        let _ = std::fs::remove_file(blue_path);
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
                    opacity: 0.5,
                },
            ],
        };
        let config = FrameConfig::new(32, 32, 0, 30.0);
        let gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let _second_gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let mut equivalent_source_scene = scene.clone();
        if let SceneNode::Image { src, .. } = &mut equivalent_source_scene.nodes[1] {
            *src = format!("  {src}  ");
        }
        let _third_gpu_image = gpu.render_frame(&equivalent_source_scene, &config).unwrap();
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
        assert_eq!(gpu.text_atlas_full_uploads(), 1);
        assert!(gpu.text_atlas_dirty_uploads() >= 1);
        assert_eq!(
            gpu.text_atlas_upload_bytes(),
            gpu.text_atlas_full_upload_bytes() + gpu.text_atlas_dirty_upload_bytes()
        );
        assert!(gpu.text_atlas_dirty_upload_bytes() < gpu.text_atlas_full_upload_bytes());
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
    fn gpu_text_unicode_and_weight_use_distinct_atlas_entries() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping Unicode text test");
            return;
        };
        let regular = Scene {
            nodes: vec![SceneNode::Text {
                x: 4.0,
                y: 30.0,
                content: "한글 日本語 العربية 😀".into(),
                font_size: 20.0,
                color: Color::WHITE,
                font_weight: 400,
                font_sources: Vec::new(),
            }],
        };
        let bold = Scene {
            nodes: vec![SceneNode::Text {
                x: 4.0,
                y: 30.0,
                content: "한글 日本語 العربية 😀".into(),
                font_size: 20.0,
                color: Color::WHITE,
                font_weight: 700,
                font_sources: Vec::new(),
            }],
        };
        let config = FrameConfig::new(192, 48, 0, 30.0);
        let regular_gpu = gpu.render_frame(&regular, &config).unwrap();
        let regular_cpu = TinySkiaBackend::new()
            .render_frame(&regular, &config)
            .unwrap();
        let bold_gpu = gpu.render_frame(&bold, &config).unwrap();
        let bold_cpu = TinySkiaBackend::new().render_frame(&bold, &config).unwrap();
        assert!(regular_gpu.pixels().any(|pixel| pixel[3] > 0));
        assert!(bold_gpu.pixels().any(|pixel| pixel[3] > 0));
        for (gpu_image, cpu_image) in [(&regular_gpu, &regular_cpu), (&bold_gpu, &bold_cpu)] {
            let mean_alpha_error = gpu_image
                .pixels()
                .zip(cpu_image.pixels())
                .map(|(gpu, cpu)| (i16::from(gpu[3]) - i16::from(cpu[3])).unsigned_abs() as u64)
                .sum::<u64>() as f64
                / (192 * 48) as f64;
            assert!(
                mean_alpha_error < 14.0,
                "Unicode alpha error: {mean_alpha_error}"
            );
        }
        assert_ne!(
            regular_gpu.pixels().collect::<Vec<_>>(),
            bold_gpu.pixels().collect::<Vec<_>>()
        );
        assert!(gpu.text_atlas_upload_bytes() > 0);
    }

    #[test]
    fn gpu_vignette_filter_matches_cpu_layer_falloff() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping vignette GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 1.0,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: None,
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: vec![crate::scene::SceneFilter::Vignette {
                    offset: 0.1,
                    darkness: 0.8,
                    roundness: 0.5,
                }],
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 64.0,
                    h: 64.0,
                    fill: Color::rgba(220, 120, 40, 255),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                }],
            }],
        };
        assert!(gpu_supports_scene(&scene));
        let config = FrameConfig::new(64, 64, 0, 30.0);
        let gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let cpu_image = TinySkiaBackend::new()
            .render_frame(&scene, &config)
            .unwrap();
        let center = gpu_image.get_pixel(32, 32);
        let corner = gpu_image.get_pixel(1, 1);
        assert!(center[0] > corner[0]);
        let cpu_center = cpu_image.get_pixel(32, 32);
        let cpu_corner = cpu_image.get_pixel(1, 1);
        let attenuation_error: f64 = (0..3)
            .map(|channel| {
                let gpu_ratio = f64::from(corner[channel]) / f64::from(center[channel].max(1));
                let cpu_ratio =
                    f64::from(cpu_corner[channel]) / f64::from(cpu_center[channel].max(1));
                (gpu_ratio - cpu_ratio).abs()
            })
            .sum::<f64>()
            / 3.0;
        assert!(
            attenuation_error < 0.03,
            "GPU/CPU vignette attenuation error was {attenuation_error}"
        );
    }

    #[test]
    fn gpu_multiply_layer_uses_blend_pipeline_and_linear_cpu_reference() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping multiply GPU test");
            return;
        };
        let mut scene = Scene {
            nodes: vec![
                SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 16.0,
                    h: 16.0,
                    fill: Color::rgb(128, 96, 64),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                },
                SceneNode::Layer {
                    opacity: 0.5,
                    blend_mode: crate::scene::BlendMode::Multiply,
                    clip: None,
                    mask: None,
                    mask_mode: crate::scene::MaskMode::Alpha,
                    filters: vec![],
                    shadow: None,
                    children: vec![SceneNode::Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 16.0,
                        h: 16.0,
                        fill: Color::rgb(64, 160, 192),
                        stroke: None,
                        stroke_width: 0.0,
                        corner_radius: 0.0,
                    }],
                },
            ],
        };
        assert!(gpu_supports_scene(&scene));
        let config = FrameConfig::new(16, 16, 0, 30.0);
        let gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let cpu_image = TinySkiaBackend::new()
            .render_frame(&scene, &config)
            .unwrap();
        let gpu_pixel = gpu_image.get_pixel(8, 8);
        let cpu_pixel = cpu_image.get_pixel(8, 8);
        for channel in 0..3 {
            let expected = f32::from(cpu_pixel[channel]);
            assert!(
                (f32::from(gpu_pixel[channel]) - expected).abs() < 4.0,
                "channel {channel}: GPU {:?}, CPU {:?}, expected linear {expected}",
                gpu_pixel,
                cpu_pixel
            );
        }
        assert_eq!(gpu.render_stats().cpu_fallback_frames, 0);

        let set_blend_mode = |scene: &mut Scene, mode| {
            let SceneNode::Layer { blend_mode, .. } = &mut scene.nodes[1] else {
                unreachable!("blend test scene lost its layer");
            };
            *blend_mode = mode;
        };
        set_blend_mode(&mut scene, crate::scene::BlendMode::Screen);
        assert!(gpu_supports_scene(&scene));
        let screen_image = gpu.render_frame(&scene, &config).unwrap();
        assert_eq!(gpu.render_stats().cpu_fallback_frames, 0);
        assert_ne!(screen_image.get_pixel(8, 8), gpu_pixel);

        set_blend_mode(&mut scene, crate::scene::BlendMode::Darken);
        let darken_image = gpu.render_frame(&scene, &config).unwrap();
        set_blend_mode(&mut scene, crate::scene::BlendMode::Lighten);
        let lighten_image = gpu.render_frame(&scene, &config).unwrap();
        assert_eq!(gpu.render_stats().cpu_fallback_frames, 0);
        assert_ne!(darken_image.get_pixel(8, 8), lighten_image.get_pixel(8, 8));

        let SceneNode::Layer {
            blend_mode,
            mask,
            mask_mode,
            filters,
            ..
        } = &mut scene.nodes[1]
        else {
            unreachable!("blend test scene lost its layer");
        };
        *blend_mode = crate::scene::BlendMode::Multiply;
        *mask_mode = crate::scene::MaskMode::Luminance;
        *filters = vec![crate::scene::SceneFilter::Opacity { amount: 0.75 }];
        *mask = Some(vec![
            SceneNode::Rect {
                x: 2.0,
                y: 2.0,
                w: 4.0,
                h: 12.0,
                fill: Color::rgb(128, 128, 128),
                stroke: None,
                stroke_width: 0.0,
                corner_radius: 0.0,
            },
            SceneNode::Rect {
                x: 10.0,
                y: 2.0,
                w: 4.0,
                h: 12.0,
                fill: Color::WHITE,
                stroke: None,
                stroke_width: 0.0,
                corner_radius: 0.0,
            },
        ]);
        assert!(gpu_supports_scene(&scene));
        let gpu_masked = gpu.render_frame(&scene, &config).unwrap();
        let cpu_masked = TinySkiaBackend::new()
            .render_frame(&scene, &config)
            .unwrap();
        assert_eq!(gpu.render_stats().cpu_fallback_frames, 0);
        for &(x, y) in &[(4, 8), (11, 8), (8, 8), (1, 1)] {
            for channel in 0..4 {
                assert!(
                    (i16::from(gpu_masked.get_pixel(x, y)[channel])
                        - i16::from(cpu_masked.get_pixel(x, y)[channel]))
                    .abs()
                        <= 5,
                    "masked blend mismatch at ({x},{y}) channel {channel}: GPU {:?}, CPU {:?}",
                    gpu_masked.get_pixel(x, y),
                    cpu_masked.get_pixel(x, y)
                );
            }
        }
    }

    #[test]
    fn gpu_invert_filter_uses_gpu_layer_path() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping invert GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 1.0,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: None,
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: vec![crate::scene::SceneFilter::Invert { amount: 1.0 }],
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 16.0,
                    h: 16.0,
                    fill: Color::rgb(255, 0, 0),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                }],
            }],
        };
        assert!(gpu_supports_scene(&scene));
        let image = gpu
            .render_frame(&scene, &FrameConfig::new(16, 16, 0, 30.0))
            .unwrap();
        let pixel = image.get_pixel(8, 8);
        eprintln!("invert pixel={pixel:?}");
        assert!(pixel[0] < 8 && pixel[1] > 247 && pixel[2] > 247);
        assert_eq!(gpu.render_stats().cpu_fallback_frames, 0);
    }

    #[test]
    fn gpu_hue_rotate_filter_uses_gpu_layer_path() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping hue rotate GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 1.0,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: None,
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: vec![crate::scene::SceneFilter::HueRotate { degrees: 120.0 }],
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 16.0,
                    h: 16.0,
                    fill: Color::rgb(255, 0, 0),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                }],
            }],
        };
        assert!(gpu_supports_scene(&scene));
        let image = gpu
            .render_frame(&scene, &FrameConfig::new(16, 16, 0, 30.0))
            .unwrap();
        let pixel = image.get_pixel(8, 8);
        assert!(
            pixel[0] < 8 && pixel[1] > 247 && pixel[2] < 8,
            "pixel={pixel:?}"
        );
        assert_eq!(gpu.render_stats().cpu_fallback_frames, 0);
    }

    #[test]
    fn gpu_tint_filter_matches_cpu_for_opaque_rect() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping tint GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 1.0,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: None,
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: vec![crate::scene::SceneFilter::Tint {
                    color: [0, 0, 255, 255],
                    amount: 0.5,
                }],
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 16.0,
                    h: 16.0,
                    fill: Color::rgb(255, 0, 0),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                }],
            }],
        };
        assert!(gpu_supports_scene(&scene));
        let config = FrameConfig::new(16, 16, 0, 30.0);
        let gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let cpu_image = TinySkiaBackend::new()
            .render_frame(&scene, &config)
            .unwrap();
        let gpu_pixel = gpu_image.get_pixel(8, 8);
        let cpu_pixel = cpu_image.get_pixel(8, 8);
        for channel in 0..4 {
            assert!(
                (i32::from(gpu_pixel[channel]) - i32::from(cpu_pixel[channel])).abs() <= 2,
                "channel {channel}: GPU {:?}, CPU {:?}",
                gpu_pixel,
                cpu_pixel
            );
        }
        assert_eq!(gpu.render_stats().cpu_fallback_frames, 0);
    }

    #[test]
    fn gpu_duotone_filter_matches_cpu_for_opaque_rect() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping duotone GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 1.0,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: None,
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: vec![crate::scene::SceneFilter::Duotone {
                    primary: [0, 0, 0, 255],
                    secondary: [255, 255, 255, 255],
                }],
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 16.0,
                    h: 16.0,
                    fill: Color::rgb(255, 0, 0),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                }],
            }],
        };
        assert!(gpu_supports_scene(&scene));
        let config = FrameConfig::new(16, 16, 0, 30.0);
        let gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let cpu_image = TinySkiaBackend::new()
            .render_frame(&scene, &config)
            .unwrap();
        let gpu_pixel = gpu_image.get_pixel(8, 8);
        let cpu_pixel = cpu_image.get_pixel(8, 8);
        for channel in 0..4 {
            assert!(
                (i32::from(gpu_pixel[channel]) - i32::from(cpu_pixel[channel])).abs() <= 2,
                "channel {channel}: GPU {:?}, CPU {:?}",
                gpu_pixel,
                cpu_pixel
            );
        }
        assert_eq!(gpu.render_stats().cpu_fallback_frames, 0);
    }

    #[test]
    fn gpu_color_grading_filter_matches_cpu_for_opaque_rect() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping color grading GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 1.0,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: None,
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: vec![crate::scene::SceneFilter::ColorGrading {
                    contrast: 1.2,
                    saturation: 0.7,
                    gamma: 1.3,
                    tint: Some([20, 40, 80, 64]),
                }],
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 16.0,
                    h: 16.0,
                    fill: Color::rgb(200, 80, 30),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                }],
            }],
        };
        assert!(gpu_supports_scene(&scene));
        let config = FrameConfig::new(16, 16, 0, 30.0);
        let gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let cpu_image = TinySkiaBackend::new()
            .render_frame(&scene, &config)
            .unwrap();
        let gpu_pixel = gpu_image.get_pixel(8, 8);
        let cpu_pixel = cpu_image.get_pixel(8, 8);
        for channel in 0..4 {
            assert!(
                (i32::from(gpu_pixel[channel]) - i32::from(cpu_pixel[channel])).abs() <= 2,
                "channel {channel}: GPU {:?}, CPU {:?}",
                gpu_pixel,
                cpu_pixel
            );
        }
        assert_eq!(gpu.render_stats().cpu_fallback_frames, 0);
    }

    #[test]
    fn gpu_invert_filter_chain_composes_amounts_in_order() {
        let filters = [
            crate::scene::SceneFilter::Invert { amount: 0.25 },
            crate::scene::SceneFilter::Invert { amount: 0.5 },
        ];
        let (_, _, _, _, _, amount, _) = gpu_layer_effects(&filters, 1.0).unwrap();
        assert!((amount - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn gpu_non_normal_blends_accept_opacity_only_filters() {
        assert!(gpu_blend_layer_filters_supported(&[]));
        assert!(gpu_blend_layer_filters_supported(&[
            crate::scene::SceneFilter::Opacity { amount: 0.5 },
        ]));
        assert!(!gpu_blend_layer_filters_supported(&[
            crate::scene::SceneFilter::Brightness { amount: 1.1 },
        ]));
    }

    #[test]
    fn gpu_color_grading_does_not_claim_mixed_color_filter_order() {
        let filters = [
            crate::scene::SceneFilter::Brightness { amount: 1.2 },
            crate::scene::SceneFilter::ColorGrading {
                contrast: 1.1,
                saturation: 0.9,
                gamma: 1.2,
                tint: None,
            },
        ];
        assert!(gpu_layer_effects(&filters, 1.0).is_none());
    }

    #[test]
    fn gpu_image_multiply_layer_matches_cpu_for_srgb_texture_and_alpha() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping image blend test");
            return;
        };
        let image_src = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let scene = Scene {
            nodes: vec![
                SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 16.0,
                    h: 16.0,
                    fill: Color::rgb(180, 120, 80),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                },
                SceneNode::Layer {
                    opacity: 0.5,
                    blend_mode: crate::scene::BlendMode::Multiply,
                    clip: None,
                    mask: None,
                    mask_mode: crate::scene::MaskMode::Alpha,
                    filters: vec![],
                    shadow: None,
                    children: vec![SceneNode::Image {
                        src: image_src.into(),
                        x: 0.0,
                        y: 0.0,
                        w: 16.0,
                        h: 16.0,
                        fit: crate::scene::ImageFit::Fill,
                        opacity: 1.0,
                    }],
                },
            ],
        };
        assert!(!gpu_supports_scene(&scene));
        let config = FrameConfig::new(16, 16, 0, 30.0);
        let gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let cpu_image = TinySkiaBackend::new()
            .render_frame(&scene, &config)
            .unwrap();
        let gpu_pixel = gpu_image.get_pixel(8, 8);
        let cpu_pixel = cpu_image.get_pixel(8, 8);
        assert_eq!(gpu_pixel, cpu_pixel);
        assert_eq!(gpu.render_stats().cpu_fallback_frames, 1);
        assert_eq!(
            gpu.last_cpu_fallback_reason().as_deref(),
            Some("scene contains GPU-unsupported nodes or effects")
        );
    }

    #[test]
    fn gpu_native_text_stream_reuses_atlas_without_readback() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping native text stream test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Text {
                x: 8.0,
                y: 32.0,
                content: "native atlas stream".into(),
                font_size: 24.0,
                color: Color::WHITE,
                font_weight: 400,
                font_sources: Vec::new(),
            }],
        };
        let mut delivered = Vec::new();
        gpu.render_stream_gpu(
            3,
            &|_frame| Ok(scene.clone()),
            &|frame| FrameConfig::new(128, 48, frame, 30.0),
            |frame, _view, width, height| delivered.push((frame, width, height)),
        )
        .unwrap();
        assert_eq!(delivered, vec![(0, 128, 48), (1, 128, 48), (2, 128, 48)]);
        assert_eq!(gpu.render_stats().gpu_frames, 3);
        assert!(gpu.text_atlas_upload_bytes() > 0);
        let upload_bytes = gpu.text_atlas_upload_bytes();
        assert_eq!(gpu.text_atlas_cache_misses(), 1);
        assert!(gpu.text_atlas_cache_hits() >= 2);
        gpu.render_stream_gpu(
            3,
            &|_frame| Ok(scene.clone()),
            &|frame| FrameConfig::new(128, 48, frame, 30.0),
            |_frame, _view, _width, _height| {},
        )
        .unwrap();
        assert_eq!(gpu.text_atlas_upload_bytes(), upload_bytes);
        assert!(gpu.text_atlas_cache_hits() >= 5);
    }

    #[test]
    fn gpu_lottie_frame_is_uploaded_and_composited_as_a_texture() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping Lottie GPU test");
            return;
        };
        let dir =
            std::env::temp_dir().join(format!("dioxuscut-wgpu-lottie-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let source = dir.join("pulse.json");
        std::fs::write(
            &source,
            r#"{"v":"5.5.7","fr":30,"ip":0,"op":30,"w":32,"h":32,"ddd":0,"assets":[],"layers":[{"ddd":0,"ind":1,"ty":4,"nm":"shape","sr":1,"ks":{"o":{"a":0,"k":100},"r":{"a":0,"k":0},"p":{"a":0,"k":[16,16,0]},"a":{"a":0,"k":[0,0,0]},"s":{"a":0,"k":[100,100,100]}},"ao":0,"shapes":[{"ty":"el","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[20,20]}},{"ty":"fl","c":{"a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100},"r":1}],"ip":0,"op":30,"st":0,"bm":0}]}"#,
        )
        .unwrap();
        let scene = Scene {
            nodes: vec![SceneNode::Lottie {
                src: source.display().to_string(),
                time: 0.0,
                x: 0.0,
                y: 0.0,
                w: 32.0,
                h: 32.0,
                playback_rate: 1.0,
                loop_behavior: crate::gif_cache::LoopBehavior::Loop,
                opacity: 1.0,
            }],
        };
        assert!(gpu_supports_scene(&scene));
        let image = gpu
            .render_frame(&scene, &FrameConfig::new(32, 32, 0, 30.0))
            .unwrap();
        let cpu = TinySkiaBackend::new()
            .render_frame(&scene, &FrameConfig::new(32, 32, 0, 30.0))
            .unwrap();
        assert!(image.pixels().any(|pixel| pixel[3] > 0));
        assert_eq!(gpu.gpu_texture_uploads(), 1);
        let mean_error = image
            .pixels()
            .zip(cpu.pixels())
            .map(|(gpu, cpu)| {
                (0..4)
                    .map(|channel| {
                        (i16::from(gpu[channel]) - i16::from(cpu[channel])).unsigned_abs() as u64
                    })
                    .sum::<u64>()
            })
            .sum::<u64>() as f64
            / (32 * 32 * 4) as f64;
        assert!(
            mean_error < 18.0,
            "Lottie GPU/CPU mean error was {mean_error}"
        );
        let second = gpu
            .render_frame(&scene, &FrameConfig::new(32, 32, 0, 30.0))
            .unwrap();
        assert_eq!(gpu.gpu_texture_uploads(), 1);
        assert_eq!(
            image.pixels().collect::<Vec<_>>(),
            second.pixels().collect::<Vec<_>>()
        );
        let mut next_frame = scene.clone();
        let SceneNode::Lottie { time, .. } = &mut next_frame.nodes[0] else {
            unreachable!("Lottie scene lost its root node");
        };
        *time = 1.0 / 30.0;
        let next = gpu
            .render_frame(&next_frame, &FrameConfig::new(32, 32, 1, 30.0))
            .unwrap();
        assert!(next.pixels().any(|pixel| pixel[3] > 0));
        assert_eq!(gpu.gpu_texture_uploads(), 2);
        let composited = Scene {
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
                SceneNode::Layer {
                    opacity: 0.5,
                    blend_mode: crate::scene::BlendMode::Normal,
                    clip: None,
                    mask: None,
                    mask_mode: crate::scene::MaskMode::Alpha,
                    filters: vec![crate::scene::SceneFilter::Brightness { amount: 0.8 }],
                    shadow: None,
                    children: vec![SceneNode::Lottie {
                        src: source.display().to_string(),
                        time: 0.0,
                        x: 0.0,
                        y: 0.0,
                        w: 32.0,
                        h: 32.0,
                        playback_rate: 1.0,
                        loop_behavior: crate::gif_cache::LoopBehavior::Loop,
                        opacity: 1.0,
                    }],
                },
            ],
        };
        let composited_gpu = gpu
            .render_frame(&composited, &FrameConfig::new(32, 32, 0, 30.0))
            .unwrap();
        let composited_cpu = TinySkiaBackend::new()
            .render_frame(&composited, &FrameConfig::new(32, 32, 0, 30.0))
            .unwrap();
        let composite_error = composited_gpu
            .pixels()
            .zip(composited_cpu.pixels())
            .map(|(gpu, cpu)| {
                (0..4)
                    .map(|channel| {
                        (i16::from(gpu[channel]) - i16::from(cpu[channel])).unsigned_abs() as u64
                    })
                    .sum::<u64>()
            })
            .sum::<u64>() as f64
            / (32 * 32 * 4) as f64;
        assert!(
            composite_error < 18.0,
            "Lottie layer opacity/draw-order mean error was {composite_error}"
        );
        assert!(composited_gpu.get_pixel(16, 16)[3] > 0);
        let _ = std::fs::remove_dir_all(&dir);
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
    fn gpu_opaque_alpha_rect_mask_matches_cpu_layer_clip() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping alpha mask GPU test");
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
                SceneNode::Layer {
                    opacity: 1.0,
                    blend_mode: crate::scene::BlendMode::Normal,
                    clip: None,
                    mask: Some(vec![
                        SceneNode::Rect {
                            x: 4.0,
                            y: 0.0,
                            w: 6.0,
                            h: 32.0,
                            fill: Color::WHITE,
                            stroke: None,
                            stroke_width: 0.0,
                            corner_radius: 0.0,
                        },
                        SceneNode::Rect {
                            x: 22.0,
                            y: 0.0,
                            w: 6.0,
                            h: 32.0,
                            fill: Color::WHITE,
                            stroke: None,
                            stroke_width: 0.0,
                            corner_radius: 0.0,
                        },
                    ]),
                    mask_mode: crate::scene::MaskMode::Alpha,
                    filters: Vec::new(),
                    shadow: None,
                    children: vec![SceneNode::Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 32.0,
                        h: 32.0,
                        fill: Color::rgb(255, 0, 0),
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
            "GPU/CPU alpha mask mean error was {mean_error}"
        );
        assert_eq!(gpu_image.get_pixel(7, 16), cpu_image.get_pixel(7, 16));
        assert_eq!(gpu_image.get_pixel(16, 16), cpu_image.get_pixel(16, 16));
    }

    #[test]
    fn gpu_grayscale_luminance_rect_mask_matches_cpu() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping luminance mask GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 1.0,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: Some(vec![SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 32.0,
                    h: 32.0,
                    fill: Color::rgb(128, 128, 128),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                }]),
                mask_mode: crate::scene::MaskMode::Luminance,
                filters: Vec::new(),
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 32.0,
                    h: 32.0,
                    fill: Color::rgb(255, 0, 0),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                }],
            }],
        };
        assert!(gpu_supports_scene(&scene));
        let config = FrameConfig::new(32, 32, 0, 30.0);
        let gpu_pixel = gpu
            .render_frame(&scene, &config)
            .unwrap()
            .get_pixel(16, 16)
            .to_owned();
        let cpu_pixel = TinySkiaBackend::new()
            .render_frame(&scene, &config)
            .unwrap()
            .get_pixel(16, 16)
            .to_owned();
        for channel in 0..4 {
            assert!(
                (i16::from(gpu_pixel[channel]) - i16::from(cpu_pixel[channel])).abs() <= 2,
                "GPU/CPU luminance mask mismatch: {gpu_pixel:?} vs {cpu_pixel:?}"
            );
        }
    }

    #[test]
    fn gpu_translucent_alpha_rect_mask_matches_cpu() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping translucent mask GPU test");
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
                SceneNode::Layer {
                    opacity: 1.0,
                    blend_mode: crate::scene::BlendMode::Normal,
                    clip: None,
                    mask: Some(vec![SceneNode::Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 32.0,
                        h: 32.0,
                        fill: Color::rgba(255, 255, 255, 128),
                        stroke: None,
                        stroke_width: 0.0,
                        corner_radius: 0.0,
                    }]),
                    mask_mode: crate::scene::MaskMode::Alpha,
                    filters: Vec::new(),
                    shadow: None,
                    children: vec![SceneNode::Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 32.0,
                        h: 32.0,
                        fill: Color::rgb(255, 0, 0),
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
        for channel in 0..4 {
            assert!(
                (i16::from(gpu_image.get_pixel(16, 16)[channel])
                    - i16::from(cpu_image.get_pixel(16, 16)[channel]))
                .abs()
                    <= 2,
                "GPU/CPU translucent mask mismatch: {:?} vs {:?}",
                gpu_image.get_pixel(16, 16),
                cpu_image.get_pixel(16, 16)
            );
        }
    }

    #[test]
    fn gpu_circle_alpha_mask_matches_cpu_at_stable_pixels() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping circle mask GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 1.0,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: Some(vec![SceneNode::Circle {
                    cx: 16.0,
                    cy: 16.0,
                    r: 10.0,
                    fill: Color::WHITE,
                    stroke: None,
                    stroke_width: 0.0,
                }]),
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: Vec::new(),
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 32.0,
                    h: 32.0,
                    fill: Color::rgb(255, 0, 0),
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
        for (x, y) in [(16, 16), (1, 1)] {
            assert_eq!(gpu_image.get_pixel(x, y), cpu_image.get_pixel(x, y));
        }
    }

    #[test]
    fn gpu_rounded_rect_alpha_mask_matches_cpu_at_stable_pixels() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping rounded mask GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 1.0,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: Some(vec![SceneNode::Rect {
                    x: 4.0,
                    y: 4.0,
                    w: 24.0,
                    h: 24.0,
                    fill: Color::WHITE,
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 6.0,
                }]),
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: Vec::new(),
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 32.0,
                    h: 32.0,
                    fill: Color::rgb(255, 0, 0),
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
        for (x, y) in [(16, 16), (1, 1)] {
            assert_eq!(gpu_image.get_pixel(x, y), cpu_image.get_pixel(x, y));
        }
    }

    #[test]
    fn gpu_layer_clip_rect_matches_cpu_at_stable_pixels() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping layer clip GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 1.0,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: Some(crate::scene::ClipRegion::Rect {
                    x: 4.0,
                    y: 4.0,
                    w: 24.0,
                    h: 24.0,
                    corner_radius: 0.0,
                }),
                mask: None,
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: Vec::new(),
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 32.0,
                    h: 32.0,
                    fill: Color::rgb(255, 0, 0),
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
        assert_eq!(gpu_image.get_pixel(16, 16), cpu_image.get_pixel(16, 16));
        assert_eq!(gpu_image.get_pixel(1, 1), cpu_image.get_pixel(1, 1));
    }

    #[test]
    fn gpu_path_clip_uses_intermediate_mask_texture() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping path clip GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 1.0,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: Some(crate::scene::ClipRegion::Path {
                    d: "M 4 4 L 28 4 L 16 28 Z".into(),
                }),
                mask: None,
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: Vec::new(),
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 32.0,
                    h: 32.0,
                    fill: Color::rgb(255, 0, 0),
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
        for (x, y) in [(16, 10), (16, 20), (2, 2)] {
            assert_eq!(gpu_image.get_pixel(x, y), cpu_image.get_pixel(x, y));
        }
    }

    #[test]
    fn gpu_path_alpha_mask_uses_intermediate_mask_texture() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping path alpha mask GPU test");
            return;
        };
        let mut scene = Scene {
            nodes: vec![
                SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 32.0,
                    h: 32.0,
                    fill: Color::rgb(40, 80, 160),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                },
                SceneNode::Layer {
                    opacity: 1.0,
                    blend_mode: crate::scene::BlendMode::Multiply,
                    clip: None,
                    mask: Some(vec![SceneNode::Path {
                        d: "M 4 4 L 28 4 L 16 28 Z".into(),
                        fill: Some(Color::rgb(128, 128, 128)),
                        stroke: None,
                        stroke_width: 0.0,
                        opacity: 1.0,
                    }]),
                    mask_mode: crate::scene::MaskMode::Luminance,
                    filters: Vec::new(),
                    shadow: None,
                    children: vec![SceneNode::Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 32.0,
                        h: 32.0,
                        fill: Color::rgb(255, 0, 0),
                        stroke: None,
                        stroke_width: 0.0,
                        corner_radius: 0.0,
                    }],
                },
            ],
        };
        let config = FrameConfig::new(32, 32, 0, 30.0);
        for blend_mode in [crate::scene::BlendMode::Multiply] {
            let SceneNode::Layer {
                blend_mode: mode, ..
            } = &mut scene.nodes[1]
            else {
                unreachable!("path mask test lost its layer");
            };
            *mode = blend_mode;
            assert!(gpu_supports_scene(&scene));
            let gpu_image = gpu.render_frame(&scene, &config).unwrap();
            let cpu_image = TinySkiaBackend::new()
                .render_frame(&scene, &config)
                .unwrap();
            for (x, y) in [(16, 10), (16, 20), (2, 2)] {
                for channel in 0..4 {
                    assert!(
                        (i16::from(gpu_image.get_pixel(x, y)[channel])
                            - i16::from(cpu_image.get_pixel(x, y)[channel]))
                        .abs()
                            <= 5,
                        "GPU/CPU path luminance mask mismatch for {blend_mode:?} at ({x},{y}) channel {channel}: {:?} vs {:?}",
                        gpu_image.get_pixel(x, y),
                        cpu_image.get_pixel(x, y)
                    );
                }
            }
            assert_eq!(gpu.render_stats().cpu_fallback_frames, 0);
        }
    }

    #[test]
    fn gpu_path_stroke_mask_uses_intermediate_mask_texture() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping path stroke mask GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![
                SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 32.0,
                    h: 32.0,
                    fill: Color::rgb(40, 80, 160),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                },
                SceneNode::Layer {
                    opacity: 1.0,
                    blend_mode: crate::scene::BlendMode::Multiply,
                    clip: None,
                    mask: Some(vec![SceneNode::Path {
                        d: "M 4 16 L 28 16".into(),
                        fill: None,
                        stroke: Some(Color::rgb(128, 128, 128)),
                        stroke_width: 4.0,
                        opacity: 1.0,
                    }]),
                    mask_mode: crate::scene::MaskMode::Luminance,
                    filters: Vec::new(),
                    shadow: None,
                    children: vec![SceneNode::Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 32.0,
                        h: 32.0,
                        fill: Color::rgb(255, 0, 0),
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
        for (x, y) in [(16, 16), (16, 10), (2, 2)] {
            for channel in 0..4 {
                assert!(
                    (i16::from(gpu_image.get_pixel(x, y)[channel])
                        - i16::from(cpu_image.get_pixel(x, y)[channel]))
                    .abs()
                        <= 5,
                    "GPU/CPU path stroke luminance mismatch at ({x},{y}) channel {channel}: {:?} vs {:?}",
                    gpu_image.get_pixel(x, y),
                    cpu_image.get_pixel(x, y)
                );
            }
        }
        assert_eq!(gpu.render_stats().cpu_fallback_frames, 0);
    }

    #[test]
    fn gpu_layer_clip_and_mask_use_intersection() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping combined clip/mask GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 1.0,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: Some(crate::scene::ClipRegion::Rect {
                    x: 4.0,
                    y: 4.0,
                    w: 24.0,
                    h: 24.0,
                    corner_radius: 0.0,
                }),
                mask: Some(vec![SceneNode::Rect {
                    x: 8.0,
                    y: 8.0,
                    w: 16.0,
                    h: 16.0,
                    fill: Color::WHITE,
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                }]),
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: Vec::new(),
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 32.0,
                    h: 32.0,
                    fill: Color::rgb(255, 0, 0),
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
        for (x, y) in [(16, 16), (6, 6), (1, 1)] {
            assert_eq!(gpu_image.get_pixel(x, y), cpu_image.get_pixel(x, y));
        }
    }

    #[test]
    fn gpu_four_stop_linear_alpha_mask_matches_cpu() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping gradient mask GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![
                SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 32.0,
                    h: 32.0,
                    fill: Color::rgb(40, 80, 160),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                },
                SceneNode::Layer {
                    opacity: 1.0,
                    blend_mode: crate::scene::BlendMode::Multiply,
                    clip: None,
                    mask: Some(vec![SceneNode::LinearGradient {
                        x: 0.0,
                        y: 0.0,
                        w: 32.0,
                        h: 32.0,
                        angle_deg: 45.0,
                        stops: vec![
                            crate::scene::GradientStop {
                                position: 0.0,
                                color: Color::rgba(255, 255, 255, 0),
                            },
                            crate::scene::GradientStop {
                                position: 0.5,
                                color: Color::WHITE,
                            },
                            crate::scene::GradientStop {
                                position: 0.75,
                                color: Color::rgba(255, 255, 255, 128),
                            },
                            crate::scene::GradientStop {
                                position: 1.0,
                                color: Color::rgba(255, 255, 255, 0),
                            },
                        ],
                    }]),
                    mask_mode: crate::scene::MaskMode::Luminance,
                    filters: Vec::new(),
                    shadow: None,
                    children: vec![SceneNode::Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 32.0,
                        h: 32.0,
                        fill: Color::rgb(255, 0, 0),
                        stroke: None,
                        stroke_width: 0.0,
                        corner_radius: 0.0,
                    }],
                },
            ],
        };
        assert!(gpu_supports_scene(&scene));
        let config = FrameConfig::new(32, 32, 0, 30.0);
        let gpu_pixel = gpu
            .render_frame(&scene, &config)
            .unwrap()
            .get_pixel(24, 16)
            .to_owned();
        let cpu_pixel = TinySkiaBackend::new()
            .render_frame(&scene, &config)
            .unwrap()
            .get_pixel(24, 16)
            .to_owned();
        for channel in 0..4 {
            assert!(
                (i16::from(gpu_pixel[channel]) - i16::from(cpu_pixel[channel])).abs() <= 3,
                "GPU/CPU gradient mask mismatch: {gpu_pixel:?} vs {cpu_pixel:?}"
            );
        }
    }

    #[test]
    fn gpu_two_stop_radial_alpha_mask_matches_cpu() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping radial mask GPU test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 1.0,
                blend_mode: crate::scene::BlendMode::Normal,
                clip: None,
                mask: Some(vec![SceneNode::RadialGradient {
                    cx: 16.0,
                    cy: 16.0,
                    r: 12.0,
                    stops: vec![
                        crate::scene::GradientStop {
                            position: 0.0,
                            color: Color::WHITE,
                        },
                        crate::scene::GradientStop {
                            position: 1.0,
                            color: Color::rgba(255, 255, 255, 0),
                        },
                    ],
                }]),
                mask_mode: crate::scene::MaskMode::Alpha,
                filters: Vec::new(),
                shadow: None,
                children: vec![SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 32.0,
                    h: 32.0,
                    fill: Color::rgb(255, 0, 0),
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
        for (x, y) in [(16, 16), (16, 20), (1, 1)] {
            for channel in 0..4 {
                assert!(
                    (i16::from(gpu_image.get_pixel(x, y)[channel])
                        - i16::from(cpu_image.get_pixel(x, y)[channel]))
                    .abs()
                        <= 3,
                    "GPU/CPU radial mask mismatch at ({x},{y}): {:?} vs {:?}",
                    gpu_image.get_pixel(x, y),
                    cpu_image.get_pixel(x, y)
                );
            }
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
    fn gpu_image_cache_reconfiguration_trims_existing_textures() {
        let Ok(gpu) = WgpuBackend::new().map(|backend| backend.with_image_cache_bytes(8)) else {
            println!("GPU backend unavailable; skipping image cache reconfiguration test");
            return;
        };
        let red = "data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20width%3D%221%22%20height%3D%221%22%3E%3Crect%20width%3D%221%22%20height%3D%221%22%20fill%3D%22%23f00%22%2F%3E%3C%2Fsvg%3E";
        let blue = "data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20width%3D%221%22%20height%3D%221%22%3E%3Crect%20width%3D%221%22%20height%3D%221%22%20fill%3D%22%2300f%22%2F%3E%3C%2Fsvg%3E";
        for src in [red, blue] {
            let scene = Scene {
                nodes: vec![SceneNode::Image {
                    src: src.into(),
                    x: 0.0,
                    y: 0.0,
                    w: 8.0,
                    h: 8.0,
                    fit: ImageFit::Fill,
                    opacity: 1.0,
                }],
            };
            gpu.render_frame(&scene, &FrameConfig::new(8, 8, 0, 30.0))
                .unwrap();
        }
        assert_eq!(gpu.gpu_image_cache_len(), 2);
        let gpu = gpu.with_image_cache_bytes(4);
        assert_eq!(gpu.gpu_image_cache_len(), 1);
    }

    #[test]
    fn gpu_gif_frame_uses_texture_path_and_matches_cpu() {
        use image::codecs::gif::{GifEncoder, Repeat};
        use image::{Delay, Frame, RgbaImage};
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping GIF texture test");
            return;
        };
        let path =
            std::env::temp_dir().join(format!("dioxuscut-wgpu-gif-{}.gif", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = GifEncoder::new(file);
        encoder.set_repeat(Repeat::Infinite).unwrap();
        for color in [[255, 0, 0, 255], [0, 0, 255, 255]] {
            let image = RgbaImage::from_pixel(2, 2, image::Rgba(color));
            encoder
                .encode_frame(Frame::from_parts(
                    image,
                    0,
                    0,
                    Delay::from_numer_denom_ms(100, 1),
                ))
                .unwrap();
        }
        drop(encoder);
        let scene = Scene {
            nodes: vec![SceneNode::Gif {
                src: path.to_string_lossy().into_owned(),
                time: 0.0,
                x: 0.0,
                y: 0.0,
                w: 16.0,
                h: 16.0,
                playback_rate: 1.0,
                loop_behavior: crate::gif_cache::LoopBehavior::Loop,
                fit: ImageFit::Fill,
                opacity: 1.0,
            }],
        };
        let config = FrameConfig::new(16, 16, 0, 30.0);
        let gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let cpu_image = TinySkiaBackend::new()
            .render_frame(&scene, &config)
            .unwrap();
        let mean_error: f64 = gpu_image
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
            / (48 * 48 * 4) as f64;
        assert!(mean_error < 18.0, "GPU/CPU GIF mean error was {mean_error}");
        assert!(gpu_image.pixels().any(|pixel| pixel[3] > 0));
        assert_eq!(gpu.render_stats().gpu_frames, 1);
        assert_eq!(gpu.render_stats().cpu_fallback_frames, 0);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn gpu_emoji_uses_texture_path_and_matches_cpu() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping emoji texture test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Emoji {
                emoji: "🔥".into(),
                x: 4.0,
                y: 4.0,
                size: 32.0,
                opacity: 0.75,
            }],
        };
        let config = FrameConfig::new(48, 48, 0, 30.0);
        let gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let cpu_image = TinySkiaBackend::new()
            .render_frame(&scene, &config)
            .unwrap();
        let mean_error: f64 = gpu_image
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
            / (48 * 48 * 4) as f64;
        assert!(
            mean_error < 18.0,
            "GPU/CPU emoji mean error was {mean_error}"
        );
        assert!(gpu_image.pixels().any(|pixel| pixel[3] > 0));
        assert_eq!(gpu.render_stats().cpu_fallback_frames, 0);
    }

    #[test]
    fn gpu_native_video_stream_delivers_ordered_frames_without_readback() {
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            println!("FFmpeg unavailable; skipping GPU native video stream test");
            return;
        }
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping GPU native video stream test");
            return;
        };
        let dir = std::env::temp_dir().join(format!(
            "dioxuscut-wgpu-video-stream-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("gradient.mkv");
        let generated = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=16x16:rate=2:duration=1",
                "-an",
                "-c:v",
                "ffv1",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(generated.success());
        let frames = 2;
        let mut delivered = Vec::new();
        gpu.render_stream_gpu(
            frames,
            &|frame| {
                Ok(Scene {
                    nodes: vec![SceneNode::Video {
                        src: source.display().to_string(),
                        time: frame as f64 / 2.0,
                        looped: false,
                        x: 0.0,
                        y: 0.0,
                        w: 16.0,
                        h: 16.0,
                        fit: ImageFit::Fill,
                        opacity: 1.0,
                    }],
                })
            },
            &|frame| FrameConfig::new(16, 16, frame, 2.0),
            |frame, _view, width, height| delivered.push((frame, width, height)),
        )
        .unwrap();
        assert_eq!(delivered, vec![(0, 16, 16), (1, 16, 16)]);
        assert_eq!(gpu.render_stats().gpu_frames, frames as u64);
        assert_eq!(gpu.gpu_texture_cache_misses(), frames as u64);
        assert_eq!(gpu.gpu_texture_uploads(), frames as u64);
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

        let composited = Scene {
            nodes: vec![
                SceneNode::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 16.0,
                    h: 16.0,
                    fill: Color::rgb(0, 0, 255),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                },
                SceneNode::Layer {
                    opacity: 0.5,
                    blend_mode: crate::scene::BlendMode::Normal,
                    clip: None,
                    mask: None,
                    mask_mode: crate::scene::MaskMode::Alpha,
                    filters: Vec::new(),
                    shadow: None,
                    children: vec![SceneNode::Video {
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
                },
            ],
        };
        let composited_gpu = gpu
            .render_frame(&composited, &FrameConfig::new(16, 16, 0, 2.0))
            .unwrap();
        let composited_cpu = TinySkiaBackend::new()
            .render_frame(&composited, &FrameConfig::new(16, 16, 0, 2.0))
            .unwrap();
        assert_eq!(
            composited_gpu.get_pixel(8, 8),
            composited_cpu.get_pixel(8, 8)
        );
        let composite_pixel = composited_gpu.get_pixel(8, 8);
        assert!(composite_pixel[0] >= 127 && composite_pixel[0] <= 128);
        assert_eq!(composite_pixel[1], 0);
        assert_eq!(composite_pixel[2], 128);
        assert_eq!(composite_pixel[3], 255);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn gpu_video_fits_match_cpu_for_all_modes() {
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            println!("FFmpeg unavailable; skipping GPU video fit test");
            return;
        }
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping GPU video fit test");
            return;
        };
        let dir =
            std::env::temp_dir().join(format!("dioxuscut-wgpu-video-fit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("rectangular.mkv");
        let generated = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=16x8:r=1:d=1",
                "-c:v",
                "ffv1",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(generated.success());
        let config = FrameConfig::new(32, 32, 0, 1.0);
        for fit in [
            ImageFit::Cover,
            ImageFit::Contain,
            ImageFit::Fill,
            ImageFit::None,
            ImageFit::ScaleDown,
        ] {
            let scene = Scene {
                nodes: vec![SceneNode::Video {
                    src: source.display().to_string(),
                    time: 0.0,
                    looped: false,
                    x: 4.0,
                    y: 4.0,
                    w: 24.0,
                    h: 24.0,
                    fit,
                    opacity: 1.0,
                }],
            };
            let gpu_image = gpu.render_frame(&scene, &config).unwrap();
            let cpu_image = TinySkiaBackend::new()
                .render_frame(&scene, &config)
                .unwrap();
            let mean_error = gpu_image
                .pixels()
                .zip(cpu_image.pixels())
                .map(|(gpu, cpu)| {
                    (0..4)
                        .map(|channel| {
                            (i16::from(gpu[channel]) - i16::from(cpu[channel])).unsigned_abs()
                                as u64
                        })
                        .sum::<u64>()
                })
                .sum::<u64>() as f64
                / (32 * 32 * 4) as f64;
            assert!(
                mean_error < 20.0,
                "GPU/CPU video {fit:?} mean error was {mean_error}"
            );
        }
        assert_eq!(gpu.gpu_image_cache_len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn gpu_video_rotation_matches_cpu_display_orientation() {
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            println!("FFmpeg unavailable; skipping GPU video rotation test");
            return;
        }
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping GPU video rotation test");
            return;
        };
        let dir = std::env::temp_dir().join(format!(
            "dioxuscut-wgpu-rotated-video-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("rotated.mp4");
        let generated = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:size=8x4:rate=1:duration=1",
                "-vf",
                "drawbox=x=0:y=0:w=4:h=2:color=red:t=fill,drawbox=x=4:y=0:w=4:h=2:color=green:t=fill,drawbox=x=0:y=2:w=4:h=2:color=blue:t=fill,drawbox=x=4:y=2:w=4:h=2:color=white:t=fill",
                "-metadata:s:v:0",
                "rotate=90",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
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
                w: 4.0,
                h: 8.0,
                fit: ImageFit::Fill,
                opacity: 1.0,
            }],
        };
        let config = FrameConfig::new(4, 8, 0, 1.0);
        let gpu_image = gpu.render_frame(&scene, &config).unwrap();
        let cpu_image = TinySkiaBackend::new()
            .render_frame(&scene, &config)
            .unwrap();
        for &(x, y) in &[(0, 0), (3, 0), (0, 7), (3, 7)] {
            let gpu_pixel = gpu_image.get_pixel(x, y);
            let cpu_pixel = cpu_image.get_pixel(x, y);
            for channel in 0..4 {
                assert!(
                    (i16::from(gpu_pixel[channel]) - i16::from(cpu_pixel[channel])).abs() <= 3,
                    "rotation pixel mismatch at ({x},{y}) channel {channel}: GPU={gpu_pixel:?} CPU={cpu_pixel:?}"
                );
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn gpu_vfr_video_rotation_matches_cpu_across_timeline() {
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
        {
            println!("FFmpeg unavailable; skipping GPU VFR rotation test");
            return;
        }
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping GPU VFR rotation test");
            return;
        };
        let dir = std::env::temp_dir().join(format!(
            "dioxuscut-wgpu-vfr-rotation-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("vfr-rotated.mp4");
        let generated = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-display_rotation:v:0",
                "90",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=16x8:rate=10:duration=1",
                "-vf",
                "select=eq(n\\,0)+eq(n\\,1)+eq(n\\,4)+eq(n\\,9)",
                "-fps_mode",
                "vfr",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(generated.success());

        let config = FrameConfig::new(8, 16, 0, 5.0);
        for time in [0.0, 0.2, 0.4, 0.6, 0.8] {
            let scene = Scene {
                nodes: vec![SceneNode::Video {
                    src: source.display().to_string(),
                    time,
                    looped: false,
                    x: 0.0,
                    y: 0.0,
                    w: 8.0,
                    h: 16.0,
                    fit: ImageFit::Fill,
                    opacity: 1.0,
                }],
            };
            let gpu_image = gpu.render_frame(&scene, &config).unwrap();
            let cpu_image = TinySkiaBackend::new()
                .render_frame(&scene, &config)
                .unwrap();
            assert_eq!(gpu_image.dimensions(), (8, 16));
            let mean_error = gpu_image
                .pixels()
                .zip(cpu_image.pixels())
                .map(|(gpu, cpu)| {
                    (0..4)
                        .map(|channel| {
                            (i16::from(gpu[channel]) - i16::from(cpu[channel])).unsigned_abs()
                                as u64
                        })
                        .sum::<u64>()
                })
                .sum::<u64>() as f64
                / (8 * 16 * 4) as f64;
            assert!(
                mean_error < 20.0,
                "GPU/CPU VFR rotation mean error at {time}s was {mean_error}"
            );
        }
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
            &mut |_f, rgba: &[u8]| {
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

    #[test]
    fn gpu_native_frame_sink_skips_cpu_readback() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping GPU-native sink test");
            return;
        };
        let scene = Scene {
            nodes: vec![SceneNode::Rect {
                x: 0.0,
                y: 0.0,
                w: 16.0,
                h: 16.0,
                fill: Color::WHITE,
                stroke: None,
                stroke_width: 0.0,
                corner_radius: 0.0,
            }],
        };
        let mut callback_dimensions = None;
        gpu.render_frame_gpu(
            &scene,
            &FrameConfig::new(16, 16, 0, 30.0),
            |_view, width, height| callback_dimensions = Some((width, height)),
        )
        .unwrap();
        assert_eq!(callback_dimensions, Some((16, 16)));
        assert_eq!(gpu.video_timing_stats().gpu_submit_readback_ns, 0);
    }

    #[test]
    fn gpu_native_stream_delivers_ordered_frames_without_readback() {
        let Ok(gpu) = WgpuBackend::new() else {
            println!("GPU backend unavailable; skipping GPU-native stream test");
            return;
        };
        let mut delivered = Vec::new();
        gpu.render_stream_gpu(
            3,
            &|frame| {
                Ok(Scene {
                    nodes: vec![SceneNode::Rect {
                        x: frame as f32,
                        y: 0.0,
                        w: 8.0,
                        h: 8.0,
                        fill: Color::WHITE,
                        stroke: None,
                        stroke_width: 0.0,
                        corner_radius: 0.0,
                    }],
                })
            },
            &|frame| FrameConfig::new(16, 16, frame, 30.0),
            |frame, _view, width, height| delivered.push((frame, width, height)),
        )
        .unwrap();
        assert_eq!(delivered, vec![(0, 16, 16), (1, 16, 16), (2, 16, 16)]);
        assert_eq!(gpu.video_timing_stats().gpu_submit_readback_ns, 0);
    }
}

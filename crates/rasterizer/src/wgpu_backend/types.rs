use crate::scene::{Color, ImageFit};
use lyon_tessellation::geometry_builder::FillVertexConstructor;
use lyon_tessellation::FillVertex;
use std::sync::Arc;
use tiny_skia::Transform;

pub(crate) const MAX_GRADIENT_STOPS: usize = 16;
pub(crate) const SAMPLE_COUNT: u32 = 4;
pub(crate) const RENDER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub(crate) const MESH_ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x2];

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ImagePlacement {
    /// Destination rectangle in canvas pixels: x, y, width, height.
    pub(crate) destination: [f32; 4],
    /// Normalized source rectangle: left, top, right, bottom.
    pub(crate) source_uv: [f32; 4],
}

/// Resolve CSS/Remotion-style image fitting into a destination rectangle and
/// normalized source crop. Keeping this independent of wgpu lets the CPU and
/// GPU image paths use exactly the same geometry.
pub(crate) fn image_placement(
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

/// Raw instance data layout matching the WGSL `InstanceData` struct.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct GpuInstance {
    pub(crate) kind_data: [u32; 4],
    pub(crate) bounds: [f32; 4],
    pub(crate) shape_bounds: [f32; 4],
    pub(crate) color: [f32; 4],
    pub(crate) color2: [f32; 4],
    pub(crate) brightness: [f32; 4],
    pub(crate) grayscale: [f32; 4],
    pub(crate) contrast: [f32; 4],
    pub(crate) saturation: [f32; 4],
    pub(crate) vignette: [f32; 4],
    pub(crate) invert: [f32; 4],
    pub(crate) hue: [f32; 4],
    pub(crate) tint: [f32; 4],
    pub(crate) duotone_primary: [f32; 4],
    pub(crate) duotone_secondary: [f32; 4],
    pub(crate) grading: [f32; 4],
    pub(crate) grading_tint: [f32; 4],
    pub(crate) clip_rects: [[f32; 4]; 4],
    pub(crate) mask_opacity: [f32; 4],
    pub(crate) mask_kinds: [u32; 4],
    pub(crate) mask_shapes: [[f32; 4]; 4],
    pub(crate) mask_color0: [[f32; 4]; 4],
    pub(crate) mask_color1: [[f32; 4]; 4],
    pub(crate) mask_color2: [[f32; 4]; 4],
    pub(crate) mask_color3: [[f32; 4]; 4],
    pub(crate) mask_stop_positions: [[f32; 4]; 4],
    pub(crate) mask_stop_counts: [u32; 4],
    pub(crate) params: [f32; 4],
    pub(crate) opacity: [f32; 4],
    pub(crate) transform_x: [f32; 4],
    pub(crate) transform_y: [f32; 4],
    pub(crate) stop_positions: [[f32; 4]; MAX_GRADIENT_STOPS],
    pub(crate) stop_colors: [[f32; 4]; MAX_GRADIENT_STOPS],
}

impl GpuInstance {
    pub(crate) fn solid(color: Color, opacity: f32, transform: Transform) -> Self {
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
pub(crate) struct GpuVertex {
    pub(crate) position: [f32; 2],
}

pub(crate) struct PositionConstructor;

impl FillVertexConstructor<GpuVertex> for PositionConstructor {
    fn new_vertex(&mut self, vertex: FillVertex<'_>) -> GpuVertex {
        GpuVertex {
            position: vertex.position().to_array(),
        }
    }
}

pub(crate) enum DrawCommand {
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
    AudioVisualizer {
        instance: GpuInstance,
        key: String,
        image: Arc<image::RgbaImage>,
    },
    Text {
        instance: GpuInstance,
        entry: crate::text_atlas::AtlasEntry,
    },
}

pub(crate) struct GpuPathMask {
    pub(crate) instance: GpuInstance,
    pub(crate) vertices: Vec<GpuVertex>,
    pub(crate) indices: Vec<u32>,
}

impl DrawCommand {
    pub(crate) fn instance(&self) -> &GpuInstance {
        match self {
            Self::Analytic { instance }
            | Self::Mesh { instance, .. }
            | Self::Image { instance, .. }
            | Self::Video { instance, .. }
            | Self::Lottie { instance, .. }
            | Self::Gif { instance, .. }
            | Self::Emoji { instance, .. }
            | Self::AudioVisualizer { instance, .. }
            | Self::Text { instance, .. } => instance,
        }
    }

    pub(crate) fn instance_mut(&mut self) -> &mut GpuInstance {
        match self {
            Self::Analytic { instance }
            | Self::Mesh { instance, .. }
            | Self::Image { instance, .. }
            | Self::Video { instance, .. }
            | Self::Lottie { instance, .. }
            | Self::Gif { instance, .. }
            | Self::Emoji { instance, .. }
            | Self::AudioVisualizer { instance, .. }
            | Self::Text { instance, .. } => instance,
        }
    }
}


pub(crate) fn transform_rows(transform: Transform) -> ([f32; 4], [f32; 4]) {
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
pub(crate) fn srgb_to_linear(channel: u8) -> f32 {
    let s = channel as f32 / 255.0;
    // IEC 61966-2-1 sRGB transfer function (precise form)
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

pub(crate) fn color_to_f32(c: Color) -> [f32; 4] {
    [
        srgb_to_linear(c.r),
        srgb_to_linear(c.g),
        srgb_to_linear(c.b),
        c.a as f32 / 255.0, // alpha is always linear
    ]
}

pub(crate) fn align_to_256(n: u32) -> u32 {
    (n + 255) & !255
}

/// Zero-copy reinterpret of a `&[T]` as `&[u8]`.
pub(crate) fn bytemuck_cast<T: Copy>(data: &[T]) -> &[u8] {
    let len = std::mem::size_of_val(data);
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, len) }
}




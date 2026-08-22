//! # dioxuscut-core
//!
//! Core Dioxus components and hooks for Dioxuscut.
//!
//! Provides a Rust/Dioxus equivalent of `remotion/core`:
//!
//! ## Components
//! - [`Composition`] — top-level video definition
//! - [`Sequence`]    — time-sliced sub-composition
//! - [`AbsoluteFill`] — full-size absolute overlay
//! - [`Freeze`]       — pause a subtree at a specific frame
//!
//! ## Hooks
//! - [`use_current_frame`] — returns the current render frame
//! - [`use_video_config`]  — returns [`VideoConfig`] for the composition
//!
//! ## Re-exports
//! - Animation primitives from [`dioxuscut_animation`]

pub mod absolute_fill;
pub mod composition;
pub mod freeze;
pub mod hooks;
pub mod scene;
pub mod sequence;
pub mod timeline;
pub mod types;

// ── Public re-exports ─────────────────────────────────────────────────────────
pub use absolute_fill::AbsoluteFill;
pub use composition::{Composition, CompositionProps};
pub use freeze::Freeze;
pub use hooks::{use_current_frame, use_video_config};
pub use scene::SceneAbsoluteFill;
pub use sequence::{Sequence, SequenceProps};
pub use timeline::context::{TimelineContext, VideoConfigContext};
pub use types::VideoConfig;

// Re-export sibling crates
pub use dioxuscut_animation as animation;
pub use dioxuscut_composition as native_composition;
pub use dioxuscut_rasterizer as rasterizer;

// Core animation primitives
pub use dioxuscut_animation::{
    bezier, interpolate, interpolate_colors, interpolate_styles, make_transform, matrix, matrix3d,
    perspective, rotate, rotate3d, rotate_x, rotate_y, rotate_z, scale, scale3d, scale_x, scale_y,
    scale_z, skew, skew_x, skew_y, spring, translate, translate3d, translate_x, translate_y,
    translate_z, EasingFn, ExtrapolateType, InterpolateOptions, SpringConfig, StyleMap, StyleValue,
    TransformOp,
};

// Composition primitives & emitters
pub use dioxuscut_composition::{
    CompositionError, NativeComposition, NativeCompositionContext, SceneEmitter, SceneFrameContext,
    SceneLoop, SceneSequence, SceneStack, SceneTransitionSeries,
};

// Typography, layout & rasterizer primitives
pub use dioxuscut_rasterizer::{
    create_rounded_text_box, create_rounded_text_box_from_measurements, fill_text_box, fit_text,
    fit_text_on_n_lines, layout_text_box, make_cancel_signal, measure_text_width,
    measure_text_width_with_font, AudioTrack, BlendMode, CancelSignal, ClipRegion, Color,
    FitTextOnNLinesOptions, FontCache, FrameConfig, GifFrame, GifFrameCache, GradientStop,
    ImageFit, LayoutError, LoopBehavior, MaskMode, PositionedTextLine, RasterError,
    RasterizerBackend, RenderCancellationToken, RenderControl, RenderProgress,
    RoundedTextBoxOptions, Scene, SceneFilter, SceneNode, SceneShadow, TextAlign, TextBox,
    TextBoxLayout, TextFitResult, TextHorizontalAlign, TextLineDimension, TextOverflow,
    TextVerticalAlign, TinySkiaBackend, Transform2D,
};

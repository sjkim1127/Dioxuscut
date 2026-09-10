# Comprehensive Codebase Survey: Dioxuscut Remotion Porting

**Date**: 2026-08-21  
**Investigator**: Explorer 1 (`teamwork_preview_explorer_survey_1`)  
**Scope**: Workspace root, all member crates in `crates/` and `apps/`, compilation & test health, Remotion reference parity, and requirements R1, R2, R3 gap analysis.

---

## 1. Workspace Overview and Structure

The Dioxuscut repository is configured as a multi-crate Cargo workspace (`resolver = "2"`, `edition = "2021"`).

### 1.1 Member Crates & Apps (`Cargo.toml:3-20`)

| Crate / Path | Package Name | Version | Role & Responsibilities |
|---|---|---|---|
| `crates/animation` | `dioxuscut-animation` | `0.1.2` | Interpolation (`interpolate`, `interpolate_colors`), spring physics (`spring`), cubic Bézier & standard easing functions (`linear`, `ease_in_quad`, `bezier`). |
| `crates/captions` | `dioxuscut-captions` | `0.1.2` | SRT subtitle parsing, word-level time synchronization, line wrapping, TikTok-style caption rendering. |
| `crates/cli` | `dioxuscut-cli` | `0.1.2` | CLI entry point (`dioxuscut render`, `preview`, `daemon`), Rhai scripting engine runtime for headless scene scripting. |
| `crates/composition` | `dioxuscut-composition` | `0.1.2` | Composition contracts (`NativeComposition`, `SceneEmitter`, `SceneFrameContext`), `SceneTransitionSeries` sequencer. |
| `crates/core` | `dioxuscut-core` | `0.1.2` | Core Dioxus components (`<Composition>`, `<Sequence>`, `<AbsoluteFill>`, `<Freeze>`) and hooks (`use_current_frame`, `use_video_config`, `use_input_props`). |
| `crates/media` | `dioxuscut-media` | `0.1.2` | Native media wrappers for images (`<Img>`), audio (`<Audio>`), and video frames (`<Video>`). |
| `crates/noise` | `dioxuscut-noise` | `0.1.2` | Procedural noise generation and animated noise background component (`<NoiseBackground>`). |
| `crates/paths` | `dioxuscut-paths` | `0.1.2` | Pure-Rust SVG path parser/serializer, path length calculation, stroke evolution (`evolve_path`), path morphing/interpolation (`interpolate_path`). |
| `crates/player` | `dioxuscut-player` | `0.1.2` | Interactive video player component, playback controls, virtualized timeline slots, preview canvas. |
| `crates/rasterizer` | `dioxuscut-rasterizer` | `0.1.2` | Pure-Rust headless rasterizer pipeline: `tiny-skia` CPU backend, optional `wgpu` GPU backend, font loader & text layout engine (`ab_glyph`, `rustybuzz`), LRU `FrameCacheManager`. |
| `crates/renderer` | `dioxuscut-renderer` | `0.1.2` | FFmpeg video encoder pipeline (`mp4`, `webm`, `gif`, `prores`), binary streaming IPC protocol & codec (`BinaryIpcCodec`, `BinaryPacket`), static web preview server. |
| `crates/shapes` | `dioxuscut-shapes` | `0.1.2` | Pure-Rust parametric 2D vector shapes (`Rect`, `Circle`, `Triangle`, `Star`, `Polygon`, `Pie`, `Heart`, `Callout`, `Arrow`, `Spark`). |
| `crates/transitions` | `dioxuscut-transitions` | `0.1.2` | Animated scene transitions (`Fade`, `Slide`, `SceneFade`, `SceneSlide`). |
| `crates/vdom` | `dioxuscut-vdom` | `0.1.2` | Headless Dioxus VDOM to `Scene` graph bridge with CSS layout support (Taffy grid/flex). |
| `apps/studio` | `studio` | `0.1.0` | Desktop/Web Studio UI with interactive timeline, filmstrip, property inspector. |
| `apps/example` | `example` | `0.1.0` | Sample Dioxus video composition demo. |

---

## 2. Compilation, Lints, and Test Suite Health

Automated checks run against the workspace baseline:

1. **Cargo Check**:
   - Command: `cargo check --locked --workspace --all-targets --all-features`
   - Result: **Passed** (exit code 0, 1.12s).
2. **Clippy**:
   - Command: `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
   - Result: **Passed** (0 warnings, exit code 0, 0.58s).
3. **Cargo Test**:
   - Command: `cargo test --locked --workspace --all-features`
   - Result: **Passed 100% across all crates** (Unit tests, integration tests, adversarial fuzz tests, doc-tests).
4. **Cargo Fmt**:
   - Command: `cargo fmt --all -- --check`
   - Result: **Diff detected** in `crates/rasterizer/src/frame_cache.rs:385` and `crates/rasterizer/tests/frame_cache_tests.rs:1,46,77,134,153,192` (minor formatting difference to be tidied during implementation).

---

## 3. Detailed Inspection of Existing Core Modules and Types

### 3.1 Scene Graph Intermediate Representation (`crates/rasterizer/src/scene.rs`)

The rasterizer consumes a `Scene` graph composed of `SceneNode` variants:

- `SceneNode::Rect { x, y, w, h, fill, stroke, stroke_width, corner_radius }`
- `SceneNode::Circle { cx, cy, r, fill, stroke, stroke_width }`
- `SceneNode::Path { d, fill, stroke, stroke_width, opacity }`
- `SceneNode::Text { x, y, content, font_size, color, font_weight, font_sources }`
- `SceneNode::Image { src, x, y, w, h, fit, opacity }`
- `SceneNode::Video { src, time, looped, x, y, w, h, fit, opacity }`
- `SceneNode::Audio { track }`
- `SceneNode::LinearGradient { x, y, w, h, angle_deg, stops }`
- `SceneNode::RadialGradient { cx, cy, r, stops }`
- `SceneNode::Group { transform: Transform2D, opacity, children }`
- `SceneNode::Layer { opacity, blend_mode, clip, mask, mask_mode, filters, shadow, children }`

#### Existing Filters on `SceneFilter` (`crates/rasterizer/src/scene.rs:184-190`):
```rust
pub enum SceneFilter {
    Blur { sigma: f32 },
    Brightness { amount: f32 },
    Grayscale { amount: f32 },
    Opacity { amount: f32 },
}
```
Currently handled in `tiny_skia_backend.rs:586-642` (`apply_filter`).

### 3.2 Transition Series & Sequencer (`crates/composition/src/scene_emitter.rs:689-950`)

- `TransitionKind`: Currently only has `Fade`, `SlideLeft`, `SlideRight`, `SlideUp`, `SlideDown`.
- `SceneTransitionSeries`:
  - Builds sequences via `.clip(duration, emitter)` and `.transition(kind, timing)`.
  - Calculates timeline overlaps (`calculate_timeline`).
  - Emits clips with linear progress interpolations (`local_frame as f32 / overlap as f32`) for opacity (`alpha_in * alpha_out`) and translation (`tx_in + tx_out`, `ty_in + ty_out`).
- `crates/transitions`:
  - Dioxus components: `Fade` (`crates/transitions/src/fade.rs`), `Slide` (`crates/transitions/src/slide.rs`).
  - Native emitters: `SceneFade`, `SceneSlide` (`crates/transitions/src/scene.rs`).

### 3.3 Text Layout & Typography Fitting (`crates/rasterizer/src/font.rs`)

- `FontCache`: Loads font files (prioritizing `DIOXUSCUT_FONT_PATH`, then system font search paths, then embedded `NotoSans-Regular.ttf` fallback).
- `measure_text_width(text, font_size, font_sources)`: Shapes and measures single-line text width via `rustybuzz` & `ab_glyph`.
- `layout_text_box(request: &TextBox)`: Breaks lines (via `unicode-linebreak`) and positions lines with horizontal/vertical alignments.
- `fit_text(text, max_width, font_sources, min_font_size, max_font_size)`: Binary search for single-line text fitting within `max_width`.

### 3.4 Procedural Noise (`crates/noise`)

- `seed.rs`: FNV-1a hashing (`hash_seed`) and xorshift PRNG (`seed_to_float`).
- `simplex.rs`: Placeholder trigonometric approximations (`(nx * 1.5 + ny * 2.3).sin()`), not real Simplex noise!
- `noise_bg.rs`: `<NoiseBackground>` rendering a Dioxus `div` with CSS radial gradient and blur.

---

## 4. Gap & Refactoring Analysis for Requirements R1, R2, R3

### Requirement R1: Standalone Native Noise & Procedural Shaders (`crates/noise`)

#### Gaps & Shortcomings Identified:
1. **Mathematical Simplex / Perlin Implementation**:
   - `crates/noise/src/simplex.rs` currently implements a crude sinusoidal mock.
   - **Required**: Complete, 100% pure Rust 2D, 3D, and 4D Simplex noise generator (`noise2d`, `noise3d`, `noise4d` / `noise_2d`, `noise_3d`, `noise_4d`) based on standard skewing factors (e.g. $F_2 = \frac{\sqrt{3}-1}{2}, G_2 = \frac{3-\sqrt{3}}{6}$, $F_3 = \frac{1}{3}, G_3 = \frac{1}{6}$, $F_4 = \frac{\sqrt{5}-1}{4}, G_4 = \frac{5-\sqrt{5}}{20}$), simplex grid corner traversal, gradient dot products, and deterministic seeding (mulberry32 / hash seeding matching Remotion specs).
2. **Fractal Brownian Motion (fBm)**:
   - Not implemented.
   - **Required**: Pure-Rust multi-octave fBm generator (`fbm_2d`, `fbm_3d`, `fbm_4d` or `fbm(seed, x, y, octaves, lacunarity, gain)`), allowing configurable frequency scaling and amplitude decay.
3. **Turbulent Flow Deformation**:
   - Not implemented.
   - **Required**: Domain warping / turbulent flow deformation API for 2D points and SVG path coordinates (`turbulent_flow(point, noise_fn)` or path deforming functions).
4. **Native Dioxus `<NoiseBackground />` Component**:
   - Currently uses simple CSS radial gradients.
   - **Required**: Enhanced procedural SVG / canvas pattern generation, integrating fBm / turbulence patterns with customizable octaves, speeds, colors, and seed.
5. **Parity and Unit Tests**:
   - Parity tests verifying deterministic output across seeds, output bounds `[-1.0, 1.0]`, and continuity properties.

---

### Requirement R2: Native Post-Processing & Visual Effects Engine (`crates/transitions`, `crates/rasterizer`)

#### Gaps & Shortcomings Identified:
1. **Layer Visual Filters (`crates/rasterizer`)**:
   - `SceneFilter` in `scene.rs` and `tiny_skia_backend.rs` lacks:
     - `ChromaticAberration { amount: f32, angle_deg: f32 }`: Offsetting red and blue channels along angle vector $(\cos\theta, \sin\theta)$.
     - `Vignette { amount: f32, radius: f32, feather: f32, roundness: f32, color: Color, mode: VignetteMode }`: Elliptical / rectangular smoothstep edge shading.
     - Color Grading filters: `Contrast`, `Saturation`, `Hue`, `Invert`, `Tint`.
   - **Required**: Add filter definitions to `SceneFilter`, implement CPU pixel shaders in `tiny_skia_backend.rs` (and GPU shaders in `wgpu_backend.rs` if enabled), with bounds checking and performance optimization.
2. **Presentation & Wipe Transitions (`crates/transitions`)**:
   - Currently only `Fade` and `Slide` exist.
   - **Required Transitions**:
     - `ClockWipe`: Circular radial wipe using `make_pie` + `translate_path` + layer clipping.
     - `LinearWipe` / `Wipe`: Directional wipe (`from-left`, `from-top`, `from-top-left`, etc.) using polygon/rect clipping paths.
     - `Flip`: 3D perspective card flip (rotation about X/Y axes).
     - `Zoom` (`DreamyZoom`, `CrossZoom`, `ZoomBlur`, `ZoomInOut`): Progressive scale/zoom and blur transitions.
   - All transitions must accept customizable easing curves (`EasingFn` or cubic Bézier options from `dioxuscut-animation`).
3. **`SceneTransitionSeries` Integration (`crates/composition`)**:
   - `TransitionKind` in `scene_emitter.rs` needs extension to support `ClockWipe`, `LinearWipe(WipeDirection)`, `Flip(FlipDirection)`, `Zoom(ZoomKind)`, etc.
   - `SceneTransitionSeries::emit` must support easing functions (`TransitionTiming::with_easing(...)`) and layer clipping / geometric transformation for wipe and flip/zoom presentations.

---

### Requirement R3: Advanced Layout & Text Fitting Utilities (`crates/rasterizer`, `crates/core`)

#### Gaps & Shortcomings Identified:
1. **Multi-line Text Auto-scaling (`fit_text_on_n_lines`)**:
   - In `vendor/remotion-4.0.495/packages/layout-utils/src/layouts/fit-text-on-n-lines.ts`, binary search finds the optimal font size such that word-wrapped text fits into `max_lines` within `max_box_width`.
   - Currently, `dioxuscut-rasterizer` only has single-line `fit_text`.
   - **Required**: Implement pure-Rust `fit_text_on_n_lines` in `crates/rasterizer/src/font.rs` using `ab_glyph`/`rustybuzz` shaping and word wrapping.
2. **Bounding-box Fill Layout (`fill_text_box`)**:
   - In `vendor/remotion-4.0.495/packages/layout-utils/src/layouts/fill-text-box.ts`, greedy word-by-word streaming text layout checks if adding a word exceeds the line width or max lines.
   - **Required**: Pure-Rust `fill_text_box` struct/method that provides incremental word measurement and line-breaking.
3. **Parametric Rounded Text Box (`create_rounded_text_box`)**:
   - In `vendor/remotion-4.0.495/packages/rounded-text-box/src/create-rounded-text-box.ts`, an SVG path `d` string with multi-corner arcs and padding is generated around multi-line text measurements.
   - **Required**: Implement `create_rounded_text_box` in pure Rust (e.g. in `crates/rasterizer` or `crates/shapes`/`crates/paths`), generating SVG path instructions (`Instruction`) and `d` strings matching Remotion's corner rounding logic.
4. **Public API Export**:
   - Export all new text fitting, layout, and rounded text box utilities cleanly from `dioxuscut-rasterizer` and re-export in `dioxuscut-core` as needed.

---

## 5. Vendor Reference Comparison Matrix

| Remotion Package | Vendor Path (`vendor/remotion-4.0.495/packages/`) | Target Dioxuscut Crate | Current Status | Key Required Additions |
|---|---|---|---|---|
| `@remotion/noise` | `packages/noise` | `crates/noise` | Stubbed / Sinusoidal | Pure Simplex 2D/3D/4D, Perlin gradients, fBm, turbulent flow, SVG `<NoiseBackground>` |
| `@remotion/transitions` | `packages/transitions` | `crates/transitions`, `crates/composition` | Basic (Fade, Slide only) | `ClockWipe`, `LinearWipe`, `Flip`, `Zoom`, customizable easing curves |
| `@remotion/effects` | `packages/effects` | `crates/rasterizer` | Partial (Blur, Brightness, Grayscale) | `ChromaticAberration`, `Vignette`, Color grading (Contrast, Saturation, Hue, Tint) |
| `@remotion/layout-utils` | `packages/layout-utils` | `crates/rasterizer`, `crates/core` | Single-line `fit_text` only | `fit_text_on_n_lines`, `fill_text_box`, `measure_text` |
| `@remotion/rounded-text-box` | `packages/rounded-text-box` | `crates/rasterizer`, `crates/shapes` | Not implemented | `create_rounded_text_box` multi-corner SVG path builder |

---

## 6. Summary & Recommendations for Execution Phase

1. **Zero Vendor Runtime Dependencies**:
   - Keep `vendor/` strictly as a reference specification for algorithmic and behavioral parity.
2. **Implementation Strategy**:
   - **Phase 1 (R1)**: Implement pure Simplex 2D/3D/4D, fBm, turbulence, and SVG noise background in `crates/noise`.
   - **Phase 2 (R2)**: Add chromatic aberration, vignette, and color grading filters to `crates/rasterizer` (`SceneFilter` + `tiny_skia_backend.rs`), and implement `ClockWipe`, `LinearWipe`, `Flip`, `Zoom` with easing curves in `crates/transitions` and `crates/composition`.
   - **Phase 3 (R3)**: Implement `fit_text_on_n_lines`, `fill_text_box`, and `create_rounded_text_box` in `crates/rasterizer` / `crates/shapes` and re-export in `crates/core`.
   - **Phase 4 (Acceptance & Verification)**: Run full test matrix (`cargo check`, `cargo clippy -D warnings`, `cargo test`, `cargo fmt`), including comprehensive mathematical parity tests for noise, visual filters, and text fitting.

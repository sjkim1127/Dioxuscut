# Project: Dioxuscut Core P1 Primitives Parity

## Architecture
Dioxuscut provides code-driven video tooling in Rust. This project implements 5 core P1 animation and composition primitives to achieve feature parity with Remotion (v4.0.495) with 100% automated test coverage across:
- `crates/paths`: SVG path morphing and evolution.
- `crates/shapes`: Parametric procedural shapes and Dioxus components.
- `crates/composition`: Timeline loops and transition series with native transforms.
- `crates/rasterizer`: Text layout, measurement, and binary search font fitting.

## Feature Inventory
| # | Feature | Description | Milestone | Source |
|---|---------|-------------|-----------|--------|
| 1 | `evolve_path` | Computes strokeDasharray and strokeDashoffset for path draw-on animation | M1 | Survey 1 (R1) |
| 2 | `interpolate_path` | Linearly interpolates numeric coordinates between two SVG paths | M1 | Survey 1 (R1) |
| 3 | `approximate_path_length` / `get_length` | Computes total approximate length across lines, beziers, and arcs | M1 | Survey 1 (R1) |
| 4 | `make_heart` | Cubic Bezier parametric heart shape generator returning `ShapeOutput` | M2 | Survey 2 (R2) |
| 5 | `make_callout` | Speech bubble callout shape with directional pointer (`CalloutDirection`) | M2 | Survey 2 (R2) |
| 6 | `make_spark` | 4-point spark/star with edge roundness and corner radius | M2 | Survey 2 (R2) |
| 7 | `make_pie` | Parametric pie chart slice / circle progress with arc splitting | M2 | Survey 2 (R2) |
| 8 | `ShapeOutput` struct | Shape output data model `{ path, width, height, transform_origin }` | M2 | Survey 2 (R2) |
| 9 | `SceneLoop<E>` | Modulo timeline loop with `local_frame = global_frame % duration_in_frames` | M3 | Survey 3 (R3) |
| 10 | `SceneTransitionSeries` | Fluent builder with timeline overlap and Fade / Slide transitions | M3 | Survey 3 (R4) |
| 11 | `fit_text` API | Binary search font size fitting with bundled font fallback | M4 | Survey 2 (R5) |
| 12 | Public Text Exports | Re-exports of `measure_text_width`, `layout_text_box`, and `TextBox` | M4 | Survey 2 (R5) |
| 13 | Quality Gate Compliance | Workspace check, clippy (-D warnings), test, fmt | M5 | All |

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|------|-------|-------------|--------|
| M1 | R1 SVG Path Morphing & Evolution | `crates/paths` | none | DONE |
| M2 | R2 Advanced Procedural Shapes | `crates/shapes` | none | DONE |
| M3 | R3 & R4 Composition Primitives | `crates/composition` | none | DONE |
| M4 | R5 Public Text Measurement & Fitting API | `crates/rasterizer` | none | DONE |
| M5 | Final Quality Gates & Workspace Test Pass | Entire Workspace | M1, M2, M3, M4 | DONE |

## Interface Contracts
### `dioxuscut-paths`
- `pub fn evolve_path(progress: f64, path: &str) -> EvolvedPath`
- `pub struct EvolvedPath { pub stroke_dasharray: String, pub stroke_dashoffset: f64 }`
- `pub fn interpolate_path(from: &str, to: &str, progress: f64) -> String`
- `pub fn approximate_path_length(path: &str) -> f64`
- `pub fn get_length(path: &str) -> f64`

### `dioxuscut-shapes`
- `pub struct ShapeOutput { pub path: String, pub width: f64, pub height: f64, pub transform_origin: String }`
- `pub enum CalloutDirection { Down, Up, Left, Right }`
- `pub fn make_heart(width: f64, height: f64) -> ShapeOutput`
- `pub fn make_callout(width: f64, height: f64, pointer_length: f64, pointer_direction: CalloutDirection) -> ShapeOutput`
- `pub fn make_spark(width: f64, height: f64, edge_roundness: f64, corner_radius: f64) -> ShapeOutput`
- `pub fn make_pie(radius: f64, progress: f64, close_path: bool, counter_clockwise: bool, rotation: f64) -> ShapeOutput`

### `dioxuscut-composition`
- `pub struct SceneLoop<E> { pub duration_in_frames: u32, pub times: u32, pub child: E }`
- `pub struct SceneTransitionSeries { ... }`
- `pub enum TransitionKind { Fade, SlideLeft, SlideRight, SlideUp, SlideDown }`
- `pub struct TransitionTiming { pub duration_in_frames: u32 }`

### `dioxuscut-rasterizer`
- `pub fn fit_text(text: &str, max_width: f64, font_sources: &[String], min_font_size: f64, max_font_size: f64) -> Result<f64, RasterError>`
- Public re-exports: `measure_text_width`, `layout_text_box`, `TextBox`, `TextBoxLayout`, `PositionedTextLine`, `FontCache`.

## Code Layout
- `crates/paths/` — Owned exclusively by Milestone 1
- `crates/shapes/` — Owned exclusively by Milestone 2
- `crates/composition/` — Owned exclusively by Milestone 3
- `crates/rasterizer/` — Owned exclusively by Milestone 4

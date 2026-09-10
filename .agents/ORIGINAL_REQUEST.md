# Original User Request

## 2026-08-21T06:28:48Z

Port key Remotion (v4.0.495) packages to 100% native Rust implementations across Dioxuscut crates, ensuring zero production dependency on the `vendor/` directory:

Working directory: /Users/sjkim1127/Dioxuscut
Integrity mode: development

## Requirements

### R1. Standalone Native Noise & Procedural Shaders (`crates/noise`)
Implement 100% pure Rust procedural noise without any vendor dependencies:
- Simplex noise (2D, 3D, 4D) and Perlin gradient generators with deterministic seeding (`noise2d`, `noise3d`, `noise4d`).
- Fractal Brownian Motion (fBm) and turbulent flow deformation for path/vector shapes.
- Native Dioxus `<NoiseBackground />` component rendering procedural SVG/canvas patterns.

### R2. Native Post-Processing & Visual Effects Engine (`crates/transitions`, `crates/rasterizer`)
Implement visual filters and compositing effects native to the tiny-skia and GPU scene renderers:
- Chromatic aberration, vignette, and color grading filters on `SceneNode::Layer`.
- Presentation and wipe transitions (ClockWipe, LinearWipe, Flip, Zoom) with customizable easing curves.
- Clean integration with `SceneTransitionSeries` and timeline tracks.

### R3. Advanced Layout & Text Fitting Utilities (`crates/rasterizer`, `crates/core`)
Provide pure Rust layout and typography fitting APIs:
- Multi-line text auto-scaling (`fit_text_on_n_lines`) and bounding-box fill layout (`fill_text_box`).
- Parametric rounded text boxes with padding, multi-corner radii, and auto-wrapping.
- Ensure all public APIs and types are exported directly from `dioxuscut-*` crates without external runtime assumptions.

## Acceptance Criteria

### Automated Verification
- [ ] `cargo check --locked --workspace --all-targets --all-features` exits with code 0.
- [ ] `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` produces 0 warnings.
- [ ] `cargo test --locked --workspace --all-features` passes 100% across all crates.
- [ ] `cargo fmt --all -- --check` reports no formatting differences.
- [ ] Complete mathematical and behavioral parity tests verifying noise generators, text fitting, and visual filters without importing from `vendor/`.


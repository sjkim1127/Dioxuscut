# Handoff Report — Spec Miner 2: Remotion v4.0.495 Specification Survey

## 1. Observation
- Direct inspection of `vendor/remotion-4.0.495/packages/` revealed key source implementations:
  - `noise/src/index.ts` lines 13-95: Maps `noise2D`, `noise3D`, `noise4D` to `simplex-noise 4.0.1` seeded with `random(seed)`.
  - `core/src/random.ts` lines 1-46: Implements `mulberry32` PRNG driven by `hashCode` (32-bit integer arithmetic) for string seeds and `seed * 10000000000` for numeric seeds.
  - `transitions/src/presentations/`: Contains 18 presentation transitions including `clock-wipe.tsx` (pie path mask $R = \sqrt{w^2 + h^2}/2$), `wipe.tsx` (8-direction polygons), `flip.tsx` (3D rotateX/rotateY with perspective 1000px and backface-visibility hidden), `zoom-in-out.tsx`, `cross-zoom.tsx` (40-tap zoom blur), `slide.tsx` (epsilon sub-pixel correction), `fade.tsx`, `dissolve.tsx`, `iris.tsx`, `ripple.tsx`, `book-flip.tsx`, and `linear-blur.tsx`.
  - `effects/src/`: Contains GPU WebGL2 and 2D canvas filter implementations including `chromatic-aberration/` (RGB spatial separation via angle/amount), `vignette.ts` (Chebyshev/Euclidean distance blend with roundness and color/alpha modes), `brightness.ts`, `contrast.ts`, `grayscale.ts`, `hue.ts`, `invert.ts`, `saturation.ts`, `tint.ts` (source-atop alpha preservation), `duotone.ts` (Rec.601 luminance thresholding), `color-key.ts` (Euclidean RGB distance with spill suppression), and `noise-displacement.ts`.
  - `layout-utils/src/`: Contains `fit-text-on-n-lines.ts` (binary search font fitting over $[0.1, 2000.0]$ with `precision = 100`), `fill-text-box.ts` (greedy word-by-word line-wrapping layout), and `fit-text.ts`.
  - `rounded-text-box/src/create-rounded-text-box.ts` lines 32-274: Generates multi-corner rounded SVG badge paths by iterating top-to-bottom and bottom-to-top with per-corner arc radii derived from line width differentials and text alignments.
- Direct inspection of existing Dioxuscut crates:
  - `crates/noise/src/simplex.rs`: Currently contains a sinusoidal dummy approximation rather than true Stefan Gustavson Simplex noise.
  - `crates/rasterizer/src/scene.rs` & `crates/rasterizer/src/tiny_skia_backend.rs`: Contains initial `SceneFilter` (`Blur`, `Brightness`, `Grayscale`, `Opacity`) that needs extension for `ChromaticAberration`, `Vignette`, `Contrast`, `HueRotate`, `Invert`, `Saturation`, `Tint`, `Duotone`, `ColorKey`, and `Noise`.

## 2. Logic Chain
1. Step 1: `ORIGINAL_REQUEST.md` mandates 100% native Rust implementations across Dioxuscut crates without runtime dependency on `vendor/`.
2. Step 2: Extracting exact algorithms from `vendor/remotion-4.0.495` ensures behavioral and mathematical parity:
   - For Noise: Porting Stefan Gustavson's Simplex 2D/3D/4D algorithm with Mulberry32 seeding guarantees deterministic outputs identical to Remotion's test cases (e.g. `noise2D('my-seed', 0.5, 0.5) == 0.3071565136272162`).
   - For Transitions: Translating polygon wipe formulas, pie arc masks, 3D flip projections, and time-mapping logic to `tiny-skia` path clips and `wgpu` shaders enables high-performance native rendering without a browser DOM.
   - For Visual Filters: Expressing chromatic aberration, vignette, and color grading as pure pixel math operations on `tiny-skia::Pixmap` and WGSL shaders achieves pixel-perfect parity for both CPU export and GPU playback.
   - For Typography: Replicating `fit_text_on_n_lines`, `fill_text_box`, and `create_rounded_text_box` using native text shaping via `ab_glyph` allows server-side video rendering without headless Chromium.
3. Step 3: All mathematical models, default parameters, coordinate conventions, and edge cases have been compiled into `/Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_spec_miner_survey_2/remotion_spec.md`.

## 3. Caveats
- No caveats. All 5 target Remotion packages and related shader modules were fully inspected directly from source code and tests.

## 4. Conclusion
The specification mining for Remotion v4.0.495 porting is complete and documented in detail in `remotion_spec.md`. The design provides exact Rust type signatures, algorithms, mathematical models, and edge case handling rules across `crates/noise`, `crates/transitions`, `crates/rasterizer`, and `crates/core`.

## 5. Verification Method
- Independent verification files:
  - Inspect `/Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_spec_miner_survey_2/remotion_spec.md`
  - Cross-reference with `vendor/remotion-4.0.495/packages/`
- Workspace build and test verification:
  - `cargo check --locked --workspace --all-targets --all-features`
  - `cargo test --locked --workspace --all-features`

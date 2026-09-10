## 2026-08-21T06:29:46Z
You are Spec Miner 2 for the Dioxuscut Remotion porting project.
Your working directory is: /Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_spec_miner_survey_2
Authoritative request: /Users/sjkim1127/Dioxuscut/.agents/ORIGINAL_REQUEST.md

Task:
Investigate Remotion (v4.0.495) reference implementations, vendor directory (`vendor/`), and relevant specs:
1. Locate and inspect all vendor files in `vendor/` or references to Remotion packages:
   - `@remotion/noise`: Simplex noise (2D, 3D, 4D), Perlin gradient generators, deterministic seeding (`noise2d`, `noise3d`, `noise4d`), fBm (fractal Brownian motion), turbulent flow, noise deformation, noise background/canvas/SVG patterns.
   - `@remotion/transitions` & visual filters: presentation and wipe transitions (ClockWipe, LinearWipe, Flip, Zoom), timing/easing curves, chromatic aberration, vignette, color grading.
   - `@remotion/layout-utils` & text fitting: `fit_text_on_n_lines`, `fill_text_box`, parametric rounded text boxes with padding, multi-corner radii, auto-wrapping.
2. Extract exact mathematical formulas, algorithms, default parameter values, edge case behaviors, and coordinate conventions.
3. Detail how these translate to 100% native pure Rust without any runtime dependency on `vendor/`.
4. Write your detailed specification report to `/Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_spec_miner_survey_2/remotion_spec.md` and a self-contained `handoff.md`. Update progress.md as you work.
5. Send a completion message to the parent orchestrator with the report path.

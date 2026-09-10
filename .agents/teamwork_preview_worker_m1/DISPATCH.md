## 2026-08-21T06:35:37Z
You are Worker 1 for Milestone 1: Native Procedural Noise & Shader Patterns (crates/noise).
Your working directory is: /Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_worker_m1

MANDATORY INTEGRITY WARNING:
DO NOT CHEAT. All implementations must be genuine. DO NOT hardcode test results, create dummy/facade implementations, or circumvent the intended task. A teamwork_preview_auditor will independently verify your work. Integrity violations WILL be detected and your work WILL be rejected.

Authoritative files to read before doing any work:
- /Users/sjkim1127/Dioxuscut/.agents/ORIGINAL_REQUEST.md
- /Users/sjkim1127/Dioxuscut/PROJECT.md
- /Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_spec_miner_survey_2/remotion_spec.md
- /Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_explorer_survey_1/survey_codebase.md

Exclusive write ownership:
You own `crates/noise/` exclusively. Do not modify files outside `crates/noise/`.

Tasks:
1. Replace the sinusoidal placeholder stubs in `crates/noise/src/simplex.rs` with true Stefan Gustavson Simplex Noise (2D, 3D, 4D) and Mulberry32 PRNG driven by 32-bit hash code for string seeds and numeric seeds. Provide `noise2d`, `noise3d`, `noise4d` (and aliases `noise_2d`, `noise_3d`, `noise_4d`) supporting deterministic seeding (`impl Into<NoiseSeed>`).
2. Implement Fractional Brownian Motion (fBm) multi-octave synthesis (`fbm_2d`, `fbm_3d`, `FbmOptions`) and turbulent flow domain warping (`turbulence_warp_2d`) for path/vector shapes in `crates/noise/src/fbm.rs`.
3. Upgrade Dioxus `<NoiseBackground />` component in `crates/noise/src/noise_bg.rs` to render procedural SVG patterns and canvas data URLs with configurable frequency, octaves, seed, and color palette.
4. Export all public types, structs, and functions from `crates/noise/src/lib.rs`.
5. Write comprehensive unit and mathematical parity tests in `crates/noise/tests/` and unit tests in `crates/noise/src/`.
6. Run:
   - `cargo check -p dioxuscut-noise --all-targets`
   - `cargo clippy -p dioxuscut-noise --all-targets -- -D warnings`
   - `cargo test -p dioxuscut-noise`
   - `cargo fmt -p dioxuscut-noise -- --check`
7. Document your work, commands executed, and test results in `/Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_worker_m1/handoff.md` and send a completion message to the parent orchestrator.

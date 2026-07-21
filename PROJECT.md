# Dioxuscut Project Architecture

## Goal

Dioxuscut provides code-driven video tooling in Rust. The current native export contract is `NativeComposition::render(frame, props, context) -> Scene`; Dioxus components provide a separate interactive preview layer until a shared representation is implemented.

## Native export pipeline

1. The CLI parses a `RenderRequest` and validates composition ID, props path, dimensions, FPS, and duration.
2. `CompositionRegistry` resolves the requested ID before rendering begins.
3. The composition produces one rasterizer `Scene` per frame.
4. `TinySkiaBackend` renders all scene features on CPU. `WgpuBackend` accelerates its supported subset and performs a whole-frame CPU fallback when required for correctness.
5. Rayon renders a bounded batch whose size is at most the worker count.
6. Frames are sorted within the batch and written to FFmpeg stdin in timeline order.
7. FFmpeg produces H.264/yuv420p MP4 output without intermediate PNG files.

## Preview pipeline

`dioxuscut-core`, `media`, `shapes`, `transitions`, and `player` render Dioxus DOM/SVG for web and desktop preview. `Sequence` offsets are relative to their parent timeline, and `Player` advances according to the configured FPS.

The preview tree is not currently compiled into native `Scene` nodes. This boundary must remain explicit in code and documentation.

## Package responsibilities

- `crates/cli`: argument parsing, validation, composition registry, built-in composition, render dispatch.
- `crates/rasterizer`: Scene IR, CPU/GPU backends, bounded FFmpeg pipe.
- `crates/renderer`: static HTTP server and PNG-sequence encoding utilities.
- `crates/core`: Dioxus timeline and context components.
- `crates/player`: Dioxus playback state and controls.
- `crates/animation`, `shapes`, `paths`, `captions`, `noise`, `media`, `transitions`: reusable primitives.
- `apps/example`: web preview example.
- `apps/studio`: desktop preview shell.

## Near-term milestones

| Priority | Milestone | Status |
|---|---|---|
| P0 | Registry-based native composition selection | Done |
| P0 | Bounded-memory FFmpeg streaming | Done |
| P0 | Default and GPU-feature quality gates | Done |
| P1 | Shared preview/export composition representation | Planned |
| P1 | Native media decoding and audio muxing | Planned |
| P1 | Full GPU scene parity | Planned |
| P2 | Reproducible bundled font configuration | Planned |
| P2 | Functional Studio editing and render queue | Planned |

## Required quality gates

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo check --locked --workspace --all-targets --all-features
cargo test --locked --workspace --all-features
```

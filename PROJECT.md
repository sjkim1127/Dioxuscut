# Dioxuscut Project Architecture

## Goal

Dioxuscut provides code-driven video tooling in Rust. The native contract is `NativeComposition::render(frame, props, context) -> Scene`; CLI export and `NativeCompositionPreview` now consume that same contract. General Dioxus components remain a separate preview layer until they can emit shared Scene nodes.

## Native export pipeline

1. The CLI parses a `RenderRequest` and validates composition ID, props path, dimensions, FPS, and duration.
2. `CompositionRegistry` resolves the requested ID before rendering begins.
3. The composition produces one rasterizer `Scene` per frame.
4. `TinySkiaBackend` renders all scene features on CPU. `WgpuBackend` accelerates its supported subset and performs a whole-frame CPU fallback when required for correctness.
5. Rayon renders a bounded batch whose size is at most the worker count.
6. Frames are sorted within the batch and written to FFmpeg stdin in timeline order.
7. FFmpeg produces H.264/yuv420p MP4 output without intermediate PNG files.

## Preview pipelines

`dioxuscut-player::NativeCompositionPreview` renders the current frame through the shared composition contract and converts its Scene to SVG. Studio uses this path for its `HelloWorld` preview, so preview and export execute the same composition implementation.

`dioxuscut-core`, `media`, `shapes`, and `transitions` also render general Dioxus DOM/SVG for web and desktop preview. `Sequence` offsets are relative to their parent timeline, and `Player` advances according to the configured FPS.

The general Dioxus preview tree is not currently compiled into native `Scene` nodes. This remaining boundary must stay explicit in code and documentation.

## Package responsibilities

- `crates/composition`: shared native composition traits, registry, and built-in composition.
- `crates/cli`: argument parsing, validation, Rhai runtime, and render dispatch.
- `crates/rasterizer`: Scene IR, CPU/GPU backends, bounded FFmpeg pipe.
- `crates/renderer`: static HTTP server and PNG-sequence encoding utilities.
- `crates/core`: Dioxus timeline and context components.
- `crates/player`: Dioxus playback state, controls, and shared Scene preview adapter.
- `crates/animation`, `shapes`, `paths`, `captions`, `noise`, `media`, `transitions`: reusable primitives.
- `apps/example`: web preview example.
- `apps/studio`: desktop preview shell.

## Near-term milestones

| Priority | Milestone | Status |
|---|---|---|
| P0 | Registry-based native composition selection | Done |
| P0 | Bounded-memory FFmpeg streaming | Done |
| P0 | Default and GPU-feature quality gates | Done |
| P1 | Shared native composition contract and preview adapter | Done |
| P1 | Migrate general Dioxus primitives to shared Scene output | Planned |
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

# E2E Test Infra: Dioxuscut High-Performance Remotion-Equivalent Compositor Architecture

## Test Philosophy
- Opaque-box, requirement-driven, and white-box adversarial verification.
- Validates the complete stack across LRU caching, binary streaming IPC protocol, compositor daemon, and timeline virtualization.
- Pass criteria: 100% test pass rate, 0 clippy warnings (`-D warnings`), 0 cargo fmt differences, clean forensic audit.

## Feature Inventory & Test Coverage Goals
| # | Feature | Requirement | Tier 1 (Unit) | Tier 2 (Boundary) | Tier 3 (Cross-Feature) | Tier 4 (Scenario) |
|---|---------|-------------|:-------------:|:-----------------:|:----------------------:|:-----------------:|
| 1 | `FrameCacheManager` LRU Eviction & Bounded Memory | R1 | >=5 | >=5 | ✓ | ✓ |
| 2 | Thread-Safe Concurrent Frame Cache Queries & Metrics | R1 | >=5 | >=5 | ✓ | ✓ |
| 3 | Binary Packet Framing `remotion_buffer:...` Format | R2 | >=5 | >=5 | ✓ | ✓ |
| 4 | Chunked Streaming `BinaryIpcCodec` / `StreamDecoder` | R2 | >=5 | >=5 | ✓ | ✓ |
| 5 | Async Nonce Correlation & Out-of-Order Multiplexing | R2 | >=5 | >=5 | ✓ | ✓ |
| 6 | Persistent `CompositorDaemon` Lifecycle & Warm Caches | R1 / R2 | >=5 | >=5 | ✓ | ✓ |
| 7 | CLI Daemon Command (`dioxuscut daemon`) & Stdio Isolation | R2 | >=5 | >=5 | ✓ | ✓ |
| 8 | `calculate_timestamp_slots` Viewport Partitioning | R3 | >=5 | >=5 | ✓ | ✓ |
| 9 | `calculate_ruler_ticks` Multi-tier Continuous Zoom | R3 | >=5 | >=5 | ✓ | ✓ |
| 10 | Background `ThumbnailCache` & Generator | R3 | >=5 | >=5 | ✓ | ✓ |
| 11 | `WaveformPeaks` Min/Max Downsampling & SVG Path | R3 | >=5 | >=5 | ✓ | ✓ |
| 12 | Studio Timeline Filmstrip & Scrubbing UI | R3 | >=5 | >=5 | ✓ | ✓ |

## Test Architecture
- Unit Tests: Embedded in each respective crate (`crates/rasterizer/tests/` & modules, `crates/renderer/tests/`, `crates/player/tests/`).
- Integration & E2E Tests: `tests/` and CLI subprocess verification.
- Stress / Adversarial Tests: Concurrency races, cache thrashing, malformed packet injection, high-frequency zoom/pan timeline virtualization.

## Acceptance Commands
- `cargo check --locked --workspace --all-targets --all-features`
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
- `cargo test --locked --workspace --all-features`
- `cargo fmt --all -- --check`

# Dioxuscut Test Infrastructure

## Test layers

### Tier 1: CLI feature coverage

`crates/cli/tests/tier1_feature_coverage.rs` verifies required flags, defaults, custom values, and short options.

### Tier 2: validation boundaries

`crates/cli/tests/tier2_boundary_cases.rs` covers empty IDs, missing props, zero or odd dimensions, invalid FPS, zero duration, and valid props paths. Registry unit tests separately verify duplicate and unknown IDs.

### Tier 3: subsystem integration

`crates/cli/tests/tier3_subsystem_integration.rs` exercises:

- Axum static-server startup, health check, and shutdown.
- Real tiny-skia PNG frame generation and PNG signatures.
- Real FFmpeg encoding from synthetic frames and MP4 container signatures.

These tests are subsystem tests; they do not claim that Dioxus VDOM is converted into native scenes.

### Tier 4: native acceptance render

`crates/cli/tests/tier4_acceptance_scenario.rs` resolves the registered `HelloWorld` composition, applies JSON props, renders 60 native frames through the bounded raw-video pipe, invokes FFmpeg, and verifies the resulting MP4.

```bash
cargo run -p dioxuscut-cli -- render \
  --composition HelloWorld \
  --props data.json \
  --output output.mp4 \
  --width 1280 \
  --height 720 \
  --fps 30 \
  --duration 60
```

## Optional-feature coverage

GPU code must compile and pass Clippy even on hosts where no GPU adapter is available. The GPU initialization test accepts a clean unavailable-adapter error; supported hosts also render a real offscreen frame. Scenes containing unsupported GPU nodes use the CPU fallback.

## Commands

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo check --locked --workspace --all-targets --all-features
cargo test --locked --workspace --all-features
```

FFmpeg must be installed for Tier 3 encoding and Tier 4 acceptance tests.

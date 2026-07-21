# Dioxuscut End-to-End (E2E) Test Infrastructure & Methodology

## Overview

This document defines the 4-tier opaque-box E2E testing methodology and test infrastructure for the **Dioxuscut** video creation framework. Dioxuscut automates video rendering by coupling a Dioxus web application, a headless Chrome renderer (CDP), and FFmpeg video encoding.

The test infrastructure validates every layer of the system without relying on hardcoded results or facade implementations, ensuring robust operation in production environments.

---

## 4-Tier Testing Methodology

```
+-----------------------------------------------------------------------+
|                       Tier 4: Acceptance Scenario                     |
|  Real-world E2E CLI invocation producing verifiable MP4 video artifact|
+-----------------------------------------------------------------------+
|                    Tier 3: Subsystem Integration                      |
|  HTTP Static Server, Headless Chrome CDP, FFmpeg Encoding Pipeline    |
+-----------------------------------------------------------------------+
|                    Tier 2: Boundary & Corner Cases                    |
|  Invalid composition, missing/malformed props, zero FPS/duration,     |
|  out-of-range resolution, I/O errors                                  |
+-----------------------------------------------------------------------+
|                      Tier 1: Feature Coverage                         |
|  CLI Flag Parsing (--composition, --props, --output, --width,         |
|  --height, --fps, --duration), Default Values & Option Mapping        |
+-----------------------------------------------------------------------+
```

---

### Tier 1: Feature Coverage (CLI Flag Parsing & Configuration)

**Objective**: Verify that `dioxuscut-cli` correctly parses command-line interface arguments, maps defaults, and enforces required/optional options.

**Test Targets**:
- `--composition` (`-c`): Required composition identifier string.
- `--props` (`-p`): Optional path to JSON props file.
- `--output` (`-o`): Output file path (default: `out.mp4`).
- `--width`: Frame resolution width in pixels (default: `1920`).
- `--height`: Frame resolution height in pixels (default: `1080`).
- `--fps`: Frame rate per second (default: `30.0`).
- `--duration`: Total duration in frames (default: `150`).

**Verification Strategy**:
- Direct clap argument parsing unit tests using `Cli::try_parse_from`.
- CLI binary option validation tests.
- Field matching between command flags and `RenderConfig` / `EncodeConfig` structures.

---

### Tier 2: Boundary & Corner Cases

**Objective**: Verify proper error handling, input validation, and graceful failure for non-standard inputs and system boundary conditions.

**Test Scenarios**:
1. **Invalid Composition**:
   - Empty composition string (`-c ""` or unsupported composition name).
   - Expected behavior: Explicit error validation failure or reported warning.
2. **Missing Props File**:
   - Path specified via `-p missing_file.json` does not exist on disk.
   - Expected behavior: Standard I/O error (`NotFound`) clean return with non-zero exit code or error result.
3. **Malformed Props JSON**:
   - Syntax error or non-JSON content in specified props file.
   - Expected behavior: JSON parse error message, standard failure.
4. **Zero or Negative FPS & Duration**:
   - `--fps 0` or `--duration 0`.
   - Expected behavior: Pre-render validation error reject invalid render parameter.
5. **Invalid Resolution**:
   - Zero resolution (`--width 0` or `--height 0`).
   - Odd pixel resolutions (e.g. 1921x1081) which violate H.264 macroblock alignment requirements.
   - Expected behavior: Resolution validation error before launching browser renderer.

---

### Tier 3: Subsystem Integration

**Objective**: Validate individual subsystem integration and contract boundaries between the web server, Headless Chrome CDP controller, and FFmpeg video compiler.

**Subsystem Contracts & Test Vectors**:

1. **HTTP Server Integration**:
   - Spawns local HTTP static server bound to an available port (`127.0.0.1:0` or designated port).
   - Validates health check HTTP `GET /` returns `200 OK`.
   - Ensures background server process clean shutdown upon drop/completion.

2. **Headless Chrome (CDP) Frame Capture**:
   - Launches `headless_chrome` browser instance with targeted window size (`width` x `height`).
   - Navigates tab to local web application server URL.
   - Evaluates frame injection script `window.DIOXUSCUT_FRAME = <frame_index>;`.
   - Captures PNG screenshot for each frame into temporary frame directory (`frame-000000.png` format).
   - Asserts PNG files exist on disk and possess non-zero byte length and valid PNG headers (`\x89PNG\r\n\x1a\n`).

3. **FFmpeg Video Encoding**:
   - Constructs FFmpeg subprocess command:
     `ffmpeg -y -framerate <fps> -i frame-%06d.png -c:v libx264 -crf 18 -pix_fmt yuv420p <output>`
   - Executes encoding pipeline over synthetic frame sequences.
   - Verifies produced MP4 video file header signature (`ftypisom` / `ftypmp42` container signature).
   - Verifies clean cleanup of temporary PNG directory upon completion.

---

### Tier 4: Real-World Acceptance Scenario

**Objective**: Execute the complete end-to-end rendering workflow from CLI invocation to finalized MP4 file production.

**Acceptance Command**:
```bash
cargo run -p dioxuscut-cli -- render \
  -c HelloWorld \
  -p data.json \
  -o output.mp4 \
  --width 1280 \
  --height 720 \
  --fps 30 \
  --duration 60
```

**Acceptance Criteria**:
1. Process exits with status code `0`.
2. Input JSON (`data.json`) read and injected into execution environment (`DIOXUSCUT_PROPS`).
3. Frame extraction successfully produces 60 sequential frame images (`frame-000000.png` .. `frame-000059.png`).
4. FFmpeg stitches frame sequence into target file `output.mp4`.
5. Output `output.mp4` exists, is non-empty, and contains valid H.264 MP4 video stream.
6. Temporary frame directories purged without leftover leakage.

---

## Test Organization & Directory Structure

```
crates/cli/
├── Cargo.toml
├── src/
│   ├── lib.rs          # Core CLI options, parsing, validation logic
│   └── main.rs         # CLI binary entry point
└── tests/
    ├── tier1_feature_coverage.rs
    ├── tier2_boundary_cases.rs
    ├── tier3_subsystem_integration.rs
    └── tier4_acceptance_scenario.rs
```

---

## Execution Instructions

Run all test suites across the workspace:
```bash
cargo test --workspace
```

Run CLI package test suite specifically:
```bash
cargo test -p dioxuscut-cli --test '*'
```

Run individual test tiers:
```bash
cargo test -p dioxuscut-cli --test tier1_feature_coverage
cargo test -p dioxuscut-cli --test tier2_boundary_cases
cargo test -p dioxuscut-cli --test tier3_subsystem_integration
cargo test -p dioxuscut-cli --test tier4_acceptance_scenario
```

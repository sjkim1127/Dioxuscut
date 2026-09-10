# Performance acceptance

Dioxuscut should beat Remotion on the corresponding workload while preserving
output. These benchmarks run the real Remotion implementation with its caches
enabled. Each script saves raw samples and exits unsuccessfully if output checks
fail or the measured median does not beat Remotion. Performance gates belong on
an otherwise idle machine; they are not timing assertions in ordinary unit tests.

## Spring kernel

From the repository root, with Node 24 and Rust installed:

```sh
python3 benchmarks/compare-spring.py --output benchmarks/spring-after.json
```

This builds the Rust example in release mode and imports the TypeScript from
`vendor/remotion-4.0.495/packages/core/src/spring`. It runs five fresh processes
per engine/workload, alternating engine order. The cold case evaluates 300
frames without warmup. Other cases warm 10,000 calls, then time 100,000 calls.
The timer excludes process startup, module loading, and compilation on both
sides. Checksums must agree within 1e-6 before a speedup is reported.

Workloads cover a repeated frame, sequential playback, an explicit duration,
fractional random seeks, settling measurements, and 32 interleaved physics
configurations. Remotion's advance, calculation, and measurement caches remain
active, including during timed loops. `spring-before.json` records the working
tree before this optimization; rerunning the current code produces the optimized
results, not that historical baseline.

The optimization evaluates the oscillator at the equivalent elapsed time in
constant time, preserving Remotion's 64ms step cap and final fractional step.
Settling durations use a 64-entry cache per thread, avoiding a global lock.
Numerical validation includes 810 vendored spring samples, 48 settling durations,
and 2,700 comparisons against iterative physics spanning damping regimes,
frame rates, and fractional steps, with a 1e-9 tolerance.

## Native export versus browser rendering

```sh
npm ci --prefix benchmarks/remotion --ignore-scripts --no-audit --no-fund
cargo build --locked --release -p dioxuscut-core --example render_bench
IMAGE_FORMAT=png node benchmarks/remotion/render-bench.mjs
IMAGE_FORMAT=jpeg node benchmarks/remotion/render-bench.mjs
```

The default Chrome path is the macOS application path. Set `CHROME_PATH` to use
another Chrome installation. Set `REPEATS` to change the default three measured
renders. Output media and browser bundles go under `target/remotion-benchmark-*`.
Pinned npm versions and their lockfile are isolated under `benchmarks/remotion`;
production Dioxuscut gains no Node dependency.

Both scene definitions render 32 opaque moving rectangles at 1280x720, 30 fps,
180 frames, using the same spring parameters. Each uses four render workers,
H.264, CRF 18, and the fast x264 preset. Both use the same system FFmpeg binary;
Remotion's bundled FFmpeg lacks the rawvideo demuxer required by Dioxuscut's pipe.
Remotion keeps its own compositor libraries via `binariesDirectory`.

One warmup render per engine is excluded. Chromium is reused, and bundling and
browser startup are reported separately rather than charged to Remotion's timed
renders. Native timings include process startup and encoding. Render order
alternates. The PNG path compares lossless intermediate frames; the JPEG path
uses Remotion's JPEG screenshot default and therefore permits an additional
lossy stage. JPEG can also mark H.264 output as full range (`yuvj420p`).

Output gates compare eight PNG frames byte-for-byte after RGBA decoding, verify
both MP4s contain all 180 frames at the expected dimensions/rate/codec, and
compare every decoded video frame. SSIM comparisons normalize the output color
range to limited-range YUV420P; every frame must score at least 0.995. The first
PNG measurement had SSIM 1.0 on all 180 frames. Checks and raw timings are saved
in `render-*.json`. These are warm-export scene benchmarks, not a comparison of
all product features, browser startup, or compile/bundle time.

## Interpretation

The measurements were taken on an Apple M4 Pro. The first lossless PNG render
batch measured a 2.192s Dioxuscut median versus 11.041s for Remotion (5.04x).
See the JSON reports for all repetitions and later batches; host load and warm
system caches can materially change absolute timings. Use fresh measurements on
the target machine when setting a production latency budget.

This scene does not exercise text shaping, media decoding, audio, masks, or
complex filters. Extend the paired scenes and output gates before making speed
claims about those workloads. A kernel speedup must not be presented as the
same speedup for a whole video export.

## The 3-Axis Battle Benchmark Suite

Run the full end-to-end battle benchmark against Remotion measuring memory footprint, 1080p complex VFX/typography, and cloud serverless costs:

```sh
python3 benchmarks/run_battle.py
```

### 1. Real-Time Memory (RSS) Profiling
```sh
# Profile any render command across its entire process tree (including Chromium workers):
python3 benchmarks/profile_memory.py -o mem_result.json -- target/release/examples/cyberpunk_bench target/test.mp4 4
```

### 2. 1080p Cyberpunk VFX & Typography Benchmark
```sh
# Dioxuscut native 1080p VFX:
target/release/examples/cyberpunk_bench target/cyberpunk.mp4 4

# Remotion 1080p VFX (Node.js + Chrome required):
node benchmarks/remotion/render-cyberpunk-bench.mjs
```

### 3. Serverless Cost & Docker Footprint Calculator
```sh
# Calculate AWS Lambda batch costs for 10k & 100k videos:
python3 benchmarks/cost_calculator.py
```

See `benchmarks/BATTLE_REPORT.md` for the latest comprehensive battle report.


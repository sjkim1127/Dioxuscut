<p align="center">
  <img src="assets/logo.svg" alt="Dioxuscut" width="100%" />
</p>

<p align="center">
  <b>Code-driven programmatic video for Rust. A faithful port of Remotion to Dioxus.</b>
</p>

<p align="center">
  <a href="https://crates.io/crates/dioxuscut-core"><img src="https://img.shields.io/badge/crates.io-v0.1.0-fc8d62?style=flat-square&logo=rust" alt="crates.io" /></a>
  <a href="#-license"><img src="https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-4ec9b0?style=flat-square" alt="License" /></a>
  <a href="https://dioxuslabs.com/"><img src="https://img.shields.io/badge/Dioxus-0.6-e05c4b?style=flat-square&logo=rust&logoColor=white" alt="Dioxus 0.6" /></a>
  <a href="https://www.remotion.dev/"><img src="https://img.shields.io/badge/Remotion-4.0%20port-6c63ff?style=flat-square" alt="Remotion 4.0 Port" /></a>
  <img src="https://img.shields.io/badge/build-passing-4ade80?style=flat-square" alt="Build" />
  <img src="https://img.shields.io/badge/AI%20agent-headless%20ready-06b6d4?style=flat-square" alt="AI Agent" />
</p>

---

**Dioxuscut** is a declarative, code-first video creation framework written entirely in Rust. You define video compositions as Dioxus components — the same way you build UIs — and the engine renders them to playable MP4 files automatically via a headless Chrome pipeline and FFmpeg encoder.

It is a complete port of [Remotion](https://www.remotion.dev/) (React/TypeScript) to [Dioxus](https://dioxuslabs.com/) (Rust), covering the same primitive APIs, same math utilities, and same package scope — but with zero garbage collection, deterministic rendering, and first-class support for AI agent automation.

---

## Table of Contents

- [Why Dioxuscut?](#why-dioxuscut)
- [Workspace Overview](#workspace-overview)
- [Crates](#crates)
- [Remotion API Parity](#remotion-api-parity)
- [Quickstart](#quickstart)
- [Tutorial: Your First Video](#tutorial-your-first-video)
- [AI Agent Integration](#ai-agent-integration)
- [CLI Reference](#cli-reference)
- [Testing](#testing)
- [Roadmap](#roadmap)
- [License](#license)

---

## Why Dioxuscut?

Traditional video editing tools are GUI-first. You drag clips, click buttons, and export. That workflow doesn't compose.

Dioxuscut treats video as code. A composition is a Rust function. Animations are math. Timing is data. This unlocks:

- **Reproducibility** — the same code always produces the same video.
- **Parametric generation** — swap a JSON file to change the content, colors, timing, or language of a video without touching any code.
- **AI agent compatibility** — an LLM can write the JSON, call the CLI, and receive an MP4. No GUI required.
- **Rust performance** — no JavaScript runtime, no garbage collector. Frame math runs in microseconds.

---

## Workspace Overview

```
Dioxuscut/
├── Cargo.toml            # Workspace root
├── assets/
│   └── logo.svg
│
├── crates/
│   ├── animation/        # spring(), interpolate(), easing, color interpolation
│   ├── core/             # Composition, Sequence, AbsoluteFill, Freeze, hooks
│   ├── shapes/           # SVG shape primitives: Circle, Rect, Star, Pie, Arrow …
│   ├── paths/            # SVG path parsing, evolve_path, get_length, get_point_at_length
│   ├── captions/         # SRT parser, line wrapper, TikTok-style kinetic captions
│   ├── noise/            # Simplex noise 2D/3D/4D, seed hashing, NoiseBackground
│   ├── transitions/      # Fade, Slide scene transitions
│   ├── media/            # <Video>, <Audio>, <Img> asset components
│   ├── player/           # Interactive <Player> UI for web and desktop
│   ├── renderer/         # Axum server + Headless Chrome + FFmpeg MP4 pipeline
│   └── cli/              # `dioxuscut render` terminal command
│
└── apps/
    ├── studio/           # Dioxus Desktop editing studio
    └── example/          # Web-based composition preview
```

---

## Crates

### `dioxuscut-animation`
Physics and math engine. Provides `spring()`, `interpolate()`, `interpolate_colors()`, and Bezier easing — the same numerical model as Remotion core.

### `dioxuscut-core`
Timeline primitives and context hooks. Provides `<Composition>`, `<Sequence>`, `<AbsoluteFill>`, `<Freeze>`, `use_current_frame()`, `use_video_config()`, and `use_input_props::<T>()`.

### `dioxuscut-shapes`
Procedural SVG motion graphics. Ported from `@remotion/shapes`.

| Component | Generator | Description |
|-----------|-----------|-------------|
| `<Circle>` | `make_circle(r)` | Centered circle |
| `<Rect>` | `make_rect(w, h, r)` | Rectangle with optional corner radius |
| `<Triangle>` | `make_triangle(len)` | Equilateral triangle |
| `<Star>` | `make_star(n, r1, r2)` | N-pointed star |
| `<Polygon>` | `make_polygon(n, r)` | Regular N-gon |
| `<Pie>` | `make_pie(r, progress)` | Progress arc (0.0 – 1.0) |
| `<Arrow>` | `make_arrow(len, thick)` | Directional arrow |

Each component renders an inline `<svg>` and accepts `fill`, `stroke`, `stroke_width`, `opacity`, and custom `style`.

### `dioxuscut-paths`
SVG path utilities. Ported from `@remotion/paths`.

| Function | Description |
|----------|-------------|
| `parse_path(d)` | Parse SVG `d` attribute into `Vec<Instruction>` |
| `serialize_instructions(insts)` | Serialize back to path string |
| `get_length(path)` | Total pixel length of a path |
| `evolve_path(progress, path)` | Compute `stroke-dasharray` / `stroke-dashoffset` for line-drawing animation |
| `get_point_at_length(path, dist)` | `Point { x, y }` at distance along path |
| `translate_path(path, dx, dy)` | Offset all coordinates |
| `scale_path(path, sx, sy)` | Scale all coordinates |

### `dioxuscut-captions`
Subtitle parsing, pagination, and animated rendering. Ported from `@remotion/captions`.

| Export | Description |
|--------|-------------|
| `parse_srt(content)` | Parse `.srt` file into `Vec<CaptionToken>` |
| `serialize_srt(tokens)` | Format tokens back to SRT string |
| `ensure_max_characters_per_line(tokens, n)` | Split tokens to fit within N characters |
| `create_tiktok_style_captions(tokens, n)` | Group tokens into N-word pages |
| `<TikTokCaptions>` | Frame-accurate active-word highlight component |

`<TikTokCaptions>` props: `active_color`, `inactive_color`, `active_scale`, `font_size`, `font_weight`, `text_shadow`, `max_words_per_page`.

### `dioxuscut-noise`
Procedural Simplex noise. Ported from `@remotion/noise`.

| Function | Description |
|----------|-------------|
| `noise_2d(seed, x, y)` | 2D Simplex noise → `[-1.0, 1.0]` |
| `noise_3d(seed, x, y, z)` | 3D Simplex noise → `[-1.0, 1.0]` |
| `noise_4d(seed, x, y, z, w)` | 4D Simplex noise → `[-1.0, 1.0]` |
| `hash_seed(seed)` | Deterministic FNV-1a seed hash |
| `<NoiseBackground>` | Animated organic radial-gradient background |

All noise functions are deterministic: identical inputs always produce identical outputs, ensuring frame-perfect rendering.

### `dioxuscut-transitions`
Scene transition helpers.

- `<Fade enter_duration exit_duration>` — Opacity fade in/out
- `<Slide direction>` — Directional slide

### `dioxuscut-media`
External asset components: `<Video>`, `<Audio>`, `<Img>`.

### `dioxuscut-player`
Interactive video player with timeline scrubber, play/pause, and frame display. Works in both web and desktop builds.

### `dioxuscut-renderer`
The headless rendering engine:
1. Spawns an Axum HTTP server on a random port.
2. Launches Headless Chrome via CDP.
3. Injects `window.DIOXUSCUT_FRAME` and `window.DIOXUSCUT_PROPS` for each frame.
4. Captures PNG screenshots.
5. Encodes the frame sequence to H.264 MP4 via FFmpeg.

All steps run automatically; no manual server management required.

### `dioxuscut-cli`
The `dioxuscut render` command. Accepts composition name, JSON props, resolution, FPS, and duration. Orchestrates the full render pipeline end-to-end.

---

## Remotion API Parity

| Remotion (TypeScript) | Dioxuscut (Rust) | Crate |
|-----------------------|-----------------|-------|
| `useCurrentFrame()` | `use_current_frame()` | `core` |
| `useVideoConfig()` | `use_video_config()` | `core` |
| `getInputProps()` | `use_input_props::<T>()` | `core` |
| `interpolate()` | `interpolate()` | `animation` |
| `interpolateColors()` | `interpolate_colors()` | `animation` |
| `spring()` | `spring()` | `animation` |
| `Easing.bezier()` | `easing::bezier()` | `animation` |
| `<Composition>` | `<Composition>` | `core` |
| `<Sequence>` | `<Sequence>` | `core` |
| `<AbsoluteFill>` | `<AbsoluteFill>` | `core` |
| `<Freeze>` | `<Freeze>` | `core` |
| `@remotion/shapes` | `dioxuscut-shapes` | `shapes` |
| `@remotion/paths` (evolvePath) | `evolve_path()` | `paths` |
| `@remotion/paths` (getLength) | `get_length()` | `paths` |
| `@remotion/paths` (getPointAtLength) | `get_point_at_length()` | `paths` |
| `@remotion/paths` (translatePath) | `translate_path()` | `paths` |
| `@remotion/paths` (scalePath) | `scale_path()` | `paths` |
| `@remotion/captions` (parseSrt) | `parse_srt()` | `captions` |
| `@remotion/captions` (TikTok captions) | `<TikTokCaptions>` | `captions` |
| `@remotion/noise` (noise2D) | `noise_2d()` | `noise` |
| `@remotion/noise` (noise3D) | `noise_3d()` | `noise` |
| `@remotion/noise` (noise4D) | `noise_4d()` | `noise` |
| `<Fade>` | `<Fade>` | `transitions` |
| `<Slide>` | `<Slide>` | `transitions` |
| `<Video>` | `<Video>` | `media` |
| `<Audio>` | `<Audio>` | `media` |
| `<Img>` | `<Img>` | `media` |
| `@remotion/player` | `dioxuscut-player` | `player` |
| `renderMedia()` | `dioxuscut render` (CLI) | `cli` |

---

## Quickstart

### Prerequisites

```bash
# Rust toolchain (1.75+)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Dioxus CLI
cargo install dioxus-cli

# FFmpeg
brew install ffmpeg        # macOS
sudo apt install ffmpeg    # Debian/Ubuntu

# Google Chrome or Chromium (for headless rendering)
```

### Web preview

```bash
dx serve --package example
# Open http://localhost:8080
```

### Desktop studio

```bash
cargo run --package studio --features desktop
```

### Headless CLI render

```bash
# Create a props file
echo '{"title":"Hello Dioxuscut","subtitle":"Made with Rust"}' > props.json

# Render to MP4
cargo run -p dioxuscut-cli -- render \
  --composition HelloWorld \
  --props props.json \
  --output output.mp4 \
  --width 1920 \
  --height 1080 \
  --fps 30 \
  --duration 150
```

---

## Tutorial: Your First Video

### 1. Define a composition

```rust
use dioxus::prelude::*;
use dioxuscut_core::{Composition, Sequence, AbsoluteFill};
use dioxuscut_player::Player;

fn main() { dioxus::launch(App); }

#[component]
fn App() -> Element {
    rsx! {
        Player {
            width: 1920, height: 1080, fps: 30.0,
            duration_in_frames: 150, controls: true,
            MyVideo {}
        }
    }
}

#[component]
fn MyVideo() -> Element {
    rsx! {
        Sequence { from: 0, duration_in_frames: 90,
            TitleCard {}
        }
    }
}
```

### 2. Animate with `spring()` and `interpolate()`

```rust
use dioxuscut_core::hooks::use_current_frame;
use dioxuscut_animation::{
    interpolate::{interpolate, ExtrapolateType, InterpolateOptions},
    spring::{spring, SpringConfig},
};

#[component]
fn TitleCard() -> Element {
    let frame = use_current_frame();

    let opacity = interpolate(
        frame as f64,
        &[0.0, 20.0], &[0.0, 1.0],
        InterpolateOptions { extrapolate_right: ExtrapolateType::Clamp, ..Default::default() },
    );

    let scale = spring(frame, 30.0, SpringConfig {
        damping: 12.0, stiffness: 150.0, ..Default::default()
    });

    rsx! {
        AbsoluteFill {
            style: "background:#0f172a; display:flex; align-items:center; justify-content:center;",
            h1 {
                style: "color:white; font-size:96px; opacity:{opacity:.4}; transform:scale({scale:.4});",
                "Hello, Dioxuscut 🦀"
            }
        }
    }
}
```

### 3. Add SVG shapes

```rust
use dioxuscut_shapes::{Star, Pie, Circle};

rsx! {
    div { style: "display:flex; gap:32px; align-items:center;",
        Circle { radius: 60.0, fill: "#00f2fe" }
        Star   { points: 5, inner_radius: 28.0, outer_radius: 60.0, fill: "#ffe600" }
        Pie    { radius: 52.0, progress: pie_progress, fill: "#6c63ff" }
    }
}
```

### 4. Animate SVG paths (line drawing)

```rust
use dioxuscut_paths::evolve_path;

let path = "M 10 50 Q 100 10 190 50 T 370 50";
let progress = (frame as f64 / 60.0).clamp(0.0, 1.0);
let evolved = evolve_path(progress, path);

rsx! {
    svg { view_box: "0 0 380 100", width: "760px",
        path {
            d: "{path}",
            fill: "none",
            stroke: "#00f2fe",
            stroke_width: "6",
            stroke_dasharray: "{evolved.stroke_dasharray}",
            stroke_dashoffset: "{evolved.stroke_dashoffset}",
        }
    }
}
```

### 5. Organic noise background

```rust
use dioxuscut_noise::NoiseBackground;

rsx! {
    NoiseBackground {
        seed: "my-video",
        base_color: "#0b0d19",
        accent_color: "#6c63ff",
        speed: 0.04,
    }
}
```

### 6. Kinetic subtitles

```rust
use dioxuscut_captions::{parse_srt, TikTokCaptions};

const SRT: &str = "1\n00:00:00,000 --> 00:00:03,000\nBuilt with Rust and Dioxus";

let tokens = parse_srt(SRT).unwrap();

rsx! {
    TikTokCaptions {
        tokens: tokens,
        max_words_per_page: 3,
        active_color: "#ffe600",
        inactive_color: "#ffffff",
        active_scale: 1.2,
        font_size: 60.0,
    }
}
```

### 7. Encode to MP4

```bash
cargo run -p dioxuscut-cli -- render \
  -c MyVideo -o my_video.mp4 \
  --width 1920 --height 1080 --fps 30 --duration 90
```

---

## AI Agent Integration

Dioxuscut is designed so that an LLM agent can generate a complete video without any GUI interaction.

The agent writes a JSON props file → calls the CLI → receives an MP4. That's it.

### Pipeline

```
LLM Agent
  │
  ├─ writes ──→ props.json        (parametric video content)
  │
  └─ runs ───→ dioxuscut render   (CLI)
                    │
                    ├─ spawns ──→ Axum HTTP server
                    ├─ opens ──→ Headless Chrome
                    ├─ injects ─→ window.DIOXUSCUT_FRAME / window.DIOXUSCUT_PROPS
                    ├─ captures → frame_000001.png … frame_000150.png
                    └─ encodes ─→ output.mp4  (FFmpeg H.264)
```

### Python example

```python
import json, subprocess

def render_video(title: str, output: str):
    props = {"title": title, "subtitle": "Generated by AI"}
    with open("props.json", "w") as f:
        json.dump(props, f)

    subprocess.run([
        "cargo", "run", "-p", "dioxuscut-cli", "--", "render",
        "-c", "HelloWorld",
        "-p", "props.json",
        "-o", output,
        "--width", "1920", "--height", "1080",
        "--fps", "30", "--duration", "120",
    ], check=True)

render_video("Quarterly Summary Q3", "q3_report.mp4")
```

### TypeScript example

```typescript
import { execSync } from 'child_process';
import { writeFileSync } from 'fs';

function renderVideo(props: Record<string, unknown>, output: string) {
  writeFileSync('props.json', JSON.stringify(props, null, 2));
  execSync(
    `cargo run -p dioxuscut-cli -- render -c HelloWorld -p props.json -o ${output} --width 1920 --height 1080 --fps 30 --duration 120`,
    { stdio: 'inherit' }
  );
}

renderVideo({ title: 'Product Launch', accent: '#00f2fe' }, 'launch.mp4');
```

### Input props JSON schema

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "type": "object",
  "properties": {
    "title":            { "type": "string" },
    "subtitle":         { "type": "string" },
    "background_start": { "type": "string", "pattern": "^#[0-9a-fA-F]{6}$" },
    "background_end":   { "type": "string", "pattern": "^#[0-9a-fA-F]{6}$" },
    "accent_color":     { "type": "string", "pattern": "^#[0-9a-fA-F]{6}$" }
  },
  "required": ["title"]
}
```

Props are injected via `use_input_props::<MyProps>()` inside any composition component.

---

## CLI Reference

```
dioxuscut render [OPTIONS]
```

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--composition <NAME>` | `-c` | *(required)* | Composition ID to render |
| `--props <PATH>` | `-p` | — | JSON file path for input props |
| `--output <PATH>` | `-o` | `out.mp4` | Output file path |
| `--width <PX>` | | `1920` | Canvas width (must be even) |
| `--height <PX>` | | `1080` | Canvas height (must be even) |
| `--fps <FLOAT>` | | `30.0` | Frames per second |
| `--duration <FRAMES>` | | `150` | Total frame count |
| `--port <INT>` | | `0` | Web server port (0 = auto) |
| `--web-dir <PATH>` | | `dist` | Static asset directory |
| `--server-url <URL>` | | — | Use existing server, skip auto-spawn |

---

## Testing

```bash
# Run all tests
cargo test --workspace

# Run a specific crate
cargo test -p dioxuscut-animation
cargo test -p dioxuscut-shapes
cargo test -p dioxuscut-paths
cargo test -p dioxuscut-captions
cargo test -p dioxuscut-noise
```

The CLI test suite is organized in four tiers:

```
crates/cli/tests/
├── tier1_feature_coverage.rs      # CLI argument parsing & defaults
├── tier2_boundary_cases.rs        # Edge cases & invalid parameters
├── tier3_subsystem_integration.rs # Axum server, Chrome CDP, FFmpeg
└── tier4_acceptance_scenario.rs   # Full render pipeline + MP4 header check
```

---

## Troubleshooting

**`ffmpeg: command not found`**
FFmpeg is not on your `PATH`. Install with `brew install ffmpeg` (macOS) or `sudo apt install ffmpeg` (Linux).

**`Invalid resolution`**
H.264 requires even-number dimensions. Use `--width 1920 --height 1080`, not `1919 × 1079`.

**Chrome launch fails in CI**
Pass `--no-sandbox` via Chrome flags. The renderer already includes this flag automatically.

---

## Roadmap

| Status | Milestone |
|--------|-----------|
| ✅ | Core workspace: `spring`, `interpolate`, `<Composition>`, `<Sequence>` |
| ✅ | CLI headless renderer: Axum + Headless Chrome + FFmpeg |
| ✅ | `@remotion/shapes` port: 7 SVG shape components |
| ✅ | `@remotion/paths` port: path parsing, `evolve_path`, metrics |
| ✅ | `@remotion/captions` port: SRT parser + `<TikTokCaptions>` |
| ✅ | `@remotion/noise` port: 2D/3D/4D Simplex + `<NoiseBackground>` |
| ⬜ | Native GPU rasterizer via Skia/wgpu (no browser dependency) |
| ⬜ | Audio waveform analyzer + beat-sync motion graphics |
| ⬜ | `@remotion/media-utils` port: audio volume probe |

---

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE) at your option.

---

<p align="center">
  Built with 🦀 Rust &nbsp;·&nbsp; Powered by <a href="https://dioxuslabs.com/">Dioxus</a> &nbsp;·&nbsp; Inspired by <a href="https://www.remotion.dev/">Remotion</a>
</p>
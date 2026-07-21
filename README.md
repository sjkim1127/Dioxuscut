<p align="center">
  <img src="https://raw.githubusercontent.com/sjkim1127/Dioxuscut/main/assets/logo.svg" alt="Dioxuscut" width="100%" />
</p>

<p align="center">
  <b>Browser-free, code-driven video rendering in Rust, with Dioxus preview components.</b>
</p>

<p align="center">
  <a href="https://github.com/sjkim1127/Dioxuscut/actions/workflows/ci.yml"><img src="https://github.com/sjkim1127/Dioxuscut/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-4ec9b0?style=flat-square" alt="License" /></a>
  <a href="https://dioxuslabs.com/"><img src="https://img.shields.io/badge/Dioxus-0.6-e05c4b?style=flat-square&logo=rust&logoColor=white" alt="Dioxus 0.6" /></a>
  <img src="https://img.shields.io/badge/status-early%20development-f59e0b?style=flat-square" alt="Early development" />
</p>

Dioxuscut is an early-stage programmatic video toolkit written in Rust. Its native export path renders a registered `NativeComposition` into a small scene graph, rasterizes frames with `tiny-skia` or `wgpu`, and sends bounded batches of raw RGBA frames to FFmpeg. The same native scene can now be displayed in the Dioxus Player through `NativeCompositionPreview`.

The repository also contains Dioxus timeline, media, shape, transition, player, and Studio-preview components. These components currently form the interactive preview layer; arbitrary Dioxus VDOM is **not yet automatically translated** into the native scene graph.

## What works today

- Native scene graph with rectangles, circles, paths, text, local raster images, gradients, and transformed groups.
- CPU rendering through `tiny-skia`.
- Experimental GPU rendering through `wgpu`; unsupported scene features fall back to the CPU renderer for correctness.
- Bounded-memory parallel frame rendering into an FFmpeg stdin pipe.
- Registry-based Rust compositions and optional sandboxed Rhai compositions, both with JSON props.
- Shared native composition contract for CLI export and Dioxus Player/Studio preview.
- Animation, shape, path, caption, noise, timeline, player, server, encoder, and CLI test coverage.
- Dioxus web example and desktop Studio preview shell.

## Architecture

```text
Native export
  RenderRequest
      -> CompositionRegistry or compiled Rhai AST
      -> Composition::prepare(props, context)
      -> PreparedComposition::render(frame)
      -> Scene
      -> TinySkiaBackend / WgpuBackend with CPU fallback
      -> bounded ordered RGBA batches
      -> FFmpeg
      -> MP4

Dioxus native-scene preview
  CompositionHandle
      -> Composition::prepare(props, context)
      -> PreparedComposition::render(frame)
      -> SceneView (SVG)
      -> Player / Studio

General Dioxus preview
  Composition / Sequence / Freeze / media / shapes
      -> Player
      -> Dioxus web or desktop UI
```

Native compositions now share one `Scene` contract between preview and export. General Dioxus VDOM components still have an explicit boundary because arbitrary Dioxus elements are not compiled into native `Scene` nodes.

## Workspace

| Package | Purpose |
|---|---|
| `dioxuscut-animation` | Interpolation, easing, springs, and color interpolation |
| `dioxuscut-composition` | Shared native composition contract, registry, and built-in composition |
| `dioxuscut-core` | Dioxus composition timeline, sequence, freeze, and hooks |
| `dioxuscut-media` | Dioxus image, video, and audio elements for preview |
| `dioxuscut-player` | Interactive player, controls, and native Scene preview adapter |
| `dioxuscut-shapes` | Procedural SVG shapes |
| `dioxuscut-paths` | SVG path parsing, metrics, and transforms |
| `dioxuscut-captions` | SRT parsing and kinetic caption helpers |
| `dioxuscut-noise` | Deterministic simplex noise helpers |
| `dioxuscut-transitions` | Dioxus fade and slide transitions |
| `dioxuscut-rasterizer` | Scene IR, CPU renderer, experimental GPU renderer, FFmpeg pipe |
| `dioxuscut-renderer` | Static server and PNG-sequence encoding utilities |
| `dioxuscut-cli` | Render command and Rhai composition runtime |
| `apps/example` | Dioxus web composition preview |
| `apps/studio` | Desktop preview shell; editing and render queue are planned |

## Prerequisites

- A current stable Rust toolchain.
- FFmpeg available on `PATH` for MP4 output.
- A supported native GPU only when using `--backend gpu`.

Install FFmpeg on common platforms:

```bash
brew install ffmpeg                 # macOS
sudo apt-get install -y ffmpeg      # Debian / Ubuntu
choco install ffmpeg -y             # Windows
```

## Quickstart

The standalone CLI ships with the `HelloWorld` native composition:

```bash
printf '%s\n' '{
  "title": "Hello Dioxuscut",
  "subtitle": "Bounded native rendering",
  "background_start": "#0f172a",
  "background_end": "#1e1b4b",
  "accent_color": "#6c63ff"
}' > props.json

cargo run -p dioxuscut-cli -- render \
  --composition HelloWorld \
  --props props.json \
  --output output.mp4 \
  --width 1280 \
  --height 720 \
  --fps 30 \
  --duration 150
```

An unknown composition ID or malformed props file fails before FFmpeg starts.

## Rhai compositions

The optional `rhai` feature adds scriptable composition logic while preserving
JSON as the external data contract. Scripts are compiled once per render job,
receive `ctx` and `props`, and return a restricted native scene builder:

```bash
cargo run -p dioxuscut-cli --features rhai -- render \
  --script examples/hello.rhai \
  --props examples/hello-props.json \
  --output rhai-output.mp4 \
  --width 1280 \
  --height 720 \
  --duration 150
```

Each script defines `fn render(ctx, props)`. The context contains `frame`,
`width`, `height`, `fps`, `duration`, and normalized `progress`. The initial API
exposes `scene()`, `rect`, `round_rect`, `circle`, `text`, `text_bold`, `image`,
`group`, and `interpolate`. `image(x, y, width, height, src, fit, opacity)` accepts
a local path or `file://` URI and the fit values `cover`, `contain`, `fill`,
`none`, and `scale-down`. See [`examples/hello.rhai`](examples/hello.rhai) for a
complete composition.

The runtime disables module imports and limits operations, call depth,
expression depth, variables, functions, strings, arrays, and maps. It does not
expose filesystem, network, clock, or random APIs. A new Rhai scope is created
for every frame so parallel rendering does not share mutable script state.

## Registering a native composition

Applications can implement the shared composition contract and pass the same composition to CLI export or `NativeCompositionPreview`:

```rust,ignore
use dioxuscut_cli::{execute_render_command_with_registry, RenderRequest};
use dioxuscut_composition::{
    CompositionError, CompositionRegistry, NativeComposition, NativeCompositionContext,
};
use dioxuscut_rasterizer::{Color, Scene, SceneNode};
use serde_json::Value;

struct TitleCard;

impl NativeComposition for TitleCard {
    fn id(&self) -> &str {
        "TitleCard"
    }

    fn render(
        &self,
        frame: u32,
        _props: &Value,
        context: NativeCompositionContext,
    ) -> Result<Scene, CompositionError> {
        let mut scene = Scene::new();
        scene.push(SceneNode::Rect {
            x: 0.0,
            y: 0.0,
            w: context.width as f32,
            h: context.height as f32,
            fill: Color::rgb(frame as u8, 24, 48),
            stroke: None,
            stroke_width: 0.0,
            corner_radius: 0.0,
        });
        Ok(scene)
    }
}

// Register TitleCard, construct a RenderRequest, then call:
// execute_render_command_with_registry(&request, &registry).await?;
```

## CLI reference

```text
dioxuscut render [OPTIONS] (--composition <ID> | --script <PATH>)

  -c, --composition <ID>       Registered composition ID
      --script <PATH>          Rhai composition file; requires feature `rhai`
  -p, --props <PATH>           JSON props file
  -o, --output <PATH>          Output path [default: out.mp4]
      --width <PX>             Even output width [default: 1920]
      --height <PX>            Even output height [default: 1080]
      --fps <FPS>              Finite positive FPS [default: 30]
      --duration <FRAMES>      Positive frame count [default: 150]
      --backend <BACKEND>      native or gpu [default: native]
```

Build GPU support explicitly:

```bash
cargo build -p dioxuscut-cli --features gpu
```

Build Rhai and GPU support together with `--features rhai,gpu`.

## Testing

The project treats default and optional-feature builds as required quality gates:

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo check --locked --workspace --all-targets --all-features
cargo test --locked --workspace --all-features
```

The acceptance test requires FFmpeg and produces a real MP4 before checking its container signature.

## Releasing

Releases are driven by a version tag such as `v0.1.0`. The release workflow
validates the tag against the Cargo workspace version, tests the complete
workspace, publishes all public `dioxuscut-*` packages to crates.io in dependency
order, builds Rhai-enabled CLI archives for Linux, macOS, and Windows, and then
creates one GitHub Release with SHA-256 checksums.

Repository maintainers must configure a crates.io API token as the GitHub Actions
secret `CRATES_IO_TOKEN`. Tokens must never be committed, placed in a tag, or
written into workflow files.

## Current limitations

- General Dioxus VDOM compositions are not automatically translated into native `Scene` nodes; native compositions use the shared preview adapter.
- Native raster images are decoded from local files and cached in memory; remote image URLs and data URIs are not supported.
- Native video/audio decoding and audio muxing are not implemented.
- GPU acceleration covers a subset of scene primitives and uses whole-frame CPU fallback otherwise.
- Font discovery uses platform fonts, so pixel-identical cross-platform text output is not guaranteed.
- Studio is a preview shell, not yet a full editor.

## Roadmap

1. Migrate reusable Dioxus media, shape, caption, and transition components onto the shared Scene contract.
2. Explicit font assets and fallback chains for reproducible text.
3. Native video decoding and FFmpeg audio muxing.
4. Full GPU parity for paths, text, groups, strokes, and multi-stop gradients.
5. Studio project loading, editing, and render queue integration.

## License

Licensed under either [Apache License 2.0](LICENSE) or [MIT License](LICENSE-MIT), at your option.

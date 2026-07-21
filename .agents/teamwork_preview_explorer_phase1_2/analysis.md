# Comprehensive Analysis: Web Application Structure, Serving Strategies & Frame Rendering Signaling

**Author**: Explorer 2 (Web App Structure & Frame Rendering Specialist)  
**Date**: 2026-07-21  
**Project**: Dioxuscut (Phase 1 Exploration)  
**Target Path**: `/Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_explorer_phase1_2/analysis.md`

---

## 1. Executive Summary

Dioxuscut is a video creation framework in Rust inspired by Remotion. It allows developers to define programmatic, frame-accurate video compositions using Dioxus components. Headless rendering is performed by driving a web browser (via Headless Chrome / CDP), rendering frame by frame, capturing PNG screenshots, and compiling them into MP4 via FFmpeg.

This analysis evaluates:
1. **Web App Structure** (`apps/example`, `apps/studio`) and property schemas (`data.json`).
2. **Frame & Props Signal Flow**: How Dioxus components consume `composition`, `props`, `use_current_frame()`, and how `window.DIOXUSCUT_FRAME` interacts between Headless Chrome and Dioxus WASM.
3. **Serving Strategies for Release 1 (R1)**: A detailed comparative evaluation between `dx serve` CLI process spawning versus serving pre-compiled static WASM/HTML assets using an embedded Rust HTTP server (`axum` / `tower-http` / `std::net`).
4. **Target Technical Specifications**: Complete specs for web app serving and reliable DOM frame signaling.

---

## 2. Web Application Structure & Data Flow

### 2.1 Apps Analysis (`apps/example` vs `apps/studio`)

The Dioxuscut workspace contains two primary application crates under `apps/`:

| Feature | `apps/example` | `apps/studio` |
|---|---|---|
| **Purpose** | Web composition target & headless render entry point | Desktop video editor studio GUI (Remotion Studio) |
| **Target Architecture** | Web (`wasm32-unknown-unknown`) | Desktop (`dioxus-desktop` / Wry WebView) |
| **Launch API** | `dioxus::launch(App)` | `dioxus_desktop::launch::launch(StudioApp, ...)` |
| **Core Components** | `<Player>`, `<Composition>`, `<Sequence>`, `<Fade>`, `<Slide>` | Studio multi-panel UI (Composition list, Player preview, Properties, Timeline track editor) |
| **Props Schema** | `ExampleProps` (`title`, `subtitle`, `background_start`, `background_end`) | `PropertyRow` props, preview compositions |

### 2.2 Property Schema & `data.json`

`data.json` located at the root repository contains:
```json
{
  "title": "🤖 AI Agent Rendered",
  "subtitle": "Fully autonomous video generation",
  "background_start": "#ff0055",
  "background_end": "#000000"
}
```
This directly maps to `ExampleProps` in `apps/example/src/main.rs`:
```rust
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct ExampleProps {
    pub title: String,
    pub subtitle: String,
    pub background_start: String,
    pub background_end: String,
}
```

### 2.3 Component Composition Hierarchy & Context Propagation

In `apps/example/src/main.rs`, video compositions are rendered via the following hierarchy:

```
[App Component]
  └── [Player Component] (crates/player/src/player.rs)
        └── [Composition Component] (crates/core/src/composition.rs)
              ├── Provides TimelineContext (frame: u32)
              ├── Provides VideoConfigContext (width, height, fps, duration)
              └── [HelloWorldComposition]
                    ├── [Sequence (from: 0, duration: 60)]
                    │     └── [Fade] ──> [TitleScene]
                    │                       ├── use_current_frame() -> u32
                    │                       ├── use_video_config() -> VideoConfig
                    │                       └── use_input_props::<ExampleProps>()
                    ├── [Sequence (from: 50, duration: 70)]
                    │     └── [Slide] ──> [LogoScene]
                    └── [Sequence (from: 110, duration: 70)]
                          └── [Fade] ──> [StatsScene]
```

#### Key Mechanisms:
1. **Context Providers**:
   - `Composition` (`crates/core/src/composition.rs`) invokes `use_context_provider(|| TimelineContext::new(frame))` and `use_context_provider(|| VideoConfigContext(config))`.
2. **Frame Offsetting in `<Sequence>`**:
   - `Sequence` (`crates/core/src/sequence.rs`) reads `TimelineContext` from parent.
   - If `absolute_frame >= from && absolute_frame < end_frame`, it renders its children wrapped in `SequenceInner` which provides `TimelineContext::with_offset(absolute_frame, from)`.
   - Thus, descendant components calling `use_current_frame()` automatically receive a 0-indexed local frame relative to that sequence start!

---

## 3. Props & Frame Reading Mechanisms & Defect Analysis

### 3.1 `use_input_props` Hook (`crates/core/src/hooks/use_input_props.rs`)
`use_input_props<T>` retrieves initial props through a multi-tier fallback:
1. **Native/CLI Environment**: Reads `std::env::var("DIOXUSCUT_PROPS")` and parses JSON via `serde_json::from_str::<T>`.
2. **Browser/WASM Environment**: Under `#[cfg(all(target_arch = "wasm32", feature = "web"))]`, accesses `web_sys::window()` and reads property `"DIOXUSCUT_PROPS"` on JS `window`.
3. **Fallback**: Executes default closure `default()`.

### 3.2 Existing Frame Signal Implementation (`crates/renderer/src/render_frames.rs`)
In `crates/renderer/src/render_frames.rs`:
```rust
for frame in range {
    let js = format!("window.DIOXUSCUT_FRAME = {};", frame);
    tab.evaluate(&js, false)?;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let png_data = tab.capture_screenshot(...)?;
    fs::write(&path, png_data)?;
}
```

### 3.3 Critical Defect & Disconnect Discovered
Our analysis revealed a **critical architectural gap in the current codebase**:
1. `tab.evaluate("window.DIOXUSCUT_FRAME = {};")` injects a global JS variable into the browser environment.
2. **However, Dioxus WASM currently has NO active event listener or hook observing `window.DIOXUSCUT_FRAME`**.
3. In `apps/example/src/main.rs`, `<Player>` controls frame animation using an internal tokio loop timer (`16ms` ticks). It does not react to external JS updates to `window.DIOXUSCUT_FRAME`.
4. Without a reactive bridge between `window.DIOXUSCUT_FRAME` and Dioxus WASM's `Composition` `frame` prop:
   - Setting `window.DIOXUSCUT_FRAME = N` in JS **does NOT update the Dioxus component tree**.
   - `sleep(30ms)` in the renderer risks capturing un-rendered frames or out-of-sync player animation ticks.

---

## 4. Evaluation of Web App Serving Strategies for R1

Milestone 1 requires spawning/serving the web application so that `headless_chrome` can capture frames. We evaluated two distinct serving strategies:

### Option A: Spawning `dx serve` CLI Process
Spawning `dx serve` directly in the renderer CLI (`crates/renderer/src/server.rs` or `dioxuscut-cli`).

### Option B: Building WASM/HTML Static Assets & Embedded HTTP Server (`axum` / `tower-http` / `std::net`)
Pre-building the WASM web app (`dx build --release --platform web`) and serving the `dist/` directory via an in-process, lightweight Rust HTTP server bound to `127.0.0.1:0` (OS-assigned ephemeral port).

### 4.1 Comparative Analysis Matrix

| Metric | Option A: `dx serve` CLI Process | Option B: Embedded Static HTTP Server |
|---|---|---|
| **Startup Latency** | High (3–15+ seconds for `dx` compilation + dev server initialization) | Extremely Low (< 1 ms instant bind & serve) |
| **System Dependencies** | Requires `dioxus-cli` (`dx`) installed on system PATH | Zero runtime dependencies (standalone Rust binary) |
| **Port Allocation** | Port collision risk if fixed port used; requires custom port CLI flags | 100% collision-free via OS ephemeral port (`127.0.0.1:0`) |
| **Rendering Reliability** | Low/Medium (Dev server injects hot-reload WebSockets & client JS scripts) | High (Clean, production-grade release WASM assets without dev overhead) |
| **Process Termination** | Complex (Requires managing child process, handling SIGINT/SIGTERM, killing sub-processes) | Simple & Clean (Rust `tokio::task` handle / cancellation token drop) |
| **Production Appropriateness** | Suited for local dev/preview only | Suited for production CLI, CI/CD, and automated rendering pipelines |

### 4.2 Recommendation & Architecture Choice for R1

**Recommendation**: **Option B (Embedded Static HTTP Server)** MUST be the primary serving mechanism for production headless rendering, with **Option A (`dx serve`)** supported optional fallback for developer live-preview.

#### Implementation Steps for Option B:
1. **Pre-build Stage**: Compile web assets via `dx build --release --platform web` outputting to `dist/` (or embedded via `rust-embed` in single-binary distributions).
2. **Server Manager (`crates/renderer/src/server.rs`)**:
   - Bind an `axum` / `tower-http` `ServeDir` router or lightweight `std::net::TcpListener` to `127.0.0.1:0`.
   - Retrieve assigned port via `listener.local_addr().unwrap().port()`.
   - Provide a `/health` endpoint returning `200 OK`.
3. **Headless Chrome Navigation**:
   - Navigate Headless Chrome tab to `http://127.0.0.1:<allocated_port>?composition=<name>`.
4. **Server Teardown**:
   - Signal Tokio graceful shutdown when frame extraction completes.

---

## 5. Technical Specifications

### 5.1 Spec 1: Ephemeral Web Server Contract (`dioxuscut-renderer::server`)

```rust
pub struct ServerConfig {
    pub static_dir: PathBuf,
    pub host: String, // Default "127.0.0.1"
    pub port: u16,    // Default 0 (ephemeral)
}

pub struct ServerHandle {
    pub url: String,
    pub port: u16,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
}

impl ServerHandle {
    pub async fn spawn(config: ServerConfig) -> Result<Self, ServerError>;
    pub async fn shutdown(self) -> Result<(), ServerError>;
}
```

### 5.2 Spec 2: Deterministic Frame Signaling & DOM Synchronization Spec

To eliminate timing non-determinism and guarantee 100% frame-accurate rendering, the following browser signaling protocol must be implemented:

```
[Headless Chrome / Renderer]                        [Dioxus WASM Web App]
            │                                                 │
            │  1. Set window.DIOXUSCUT_PROPS = json           │
            │  2. Navigate to http://127.0.0.1:port/          │
            │ ──────────────────────────────────────────────> │ (Reads window.DIOXUSCUT_PROPS on mount)
            │                                                 │
            │  3. tab.evaluate("setFrame(0)")                │
            │ ──────────────────────────────────────────────> │ (Triggers frame update signal)
            │                                                 │ (Dioxus updates DOM)
            │                                                 │ (window.DIOXUSCUT_RENDERED = true)
            │  4. Wait for window.DIOXUSCUT_RENDERED === true │
            │ <────────────────────────────────────────────── │
            │                                                 │
            │  5. tab.capture_screenshot() -> frame_000000.png │
            │                                                 │
            │  6. tab.evaluate("setFrame(1)")                │
            │ ──────────────────────────────────────────────> │
```

#### WASM Bridge Component Implementation (`HeadlessDriver`):

In `apps/example/src/main.rs` (or `crates/core`):
```rust
#[component]
pub fn HeadlessDriver(props: HeadlessDriverProps) -> Element {
    let mut current_frame = use_signal(|| 0u32);

    use_effect(move || {
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::prelude::*;
            use web_sys::window;

            let closure = Closure::<dyn Fn(u32)>::new(move |f: u32| {
                current_frame.set(f);
                // After signal update, mark rendered flag on next microtick
                if let Some(win) = window() {
                    let _ = js_sys::Reflect::set(
                        &win,
                        &JsValue::from_str("DIOXUSCUT_RENDERED"),
                        &JsValue::from_bool(true),
                    );
                }
            });

            if let Some(win) = window() {
                let _ = js_sys::Reflect::set(
                    &win,
                    &JsValue::from_str("__DIOXUSCUT_SET_FRAME"),
                    closure.as_ref(),
                );
                closure.forget();
            }
        }
    });

    rsx! {
        Composition {
            id: props.id,
            width: props.width,
            height: props.height,
            fps: props.fps,
            duration_in_frames: props.duration_in_frames,
            frame: current_frame.read().clone(),
            {props.children}
        }
    }
}
```

#### Renderer Frame Capture Protocol (`crates/renderer/src/render_frames.rs`):
```rust
for frame in range {
    // 1. Reset completion signal
    tab.evaluate("window.DIOXUSCUT_RENDERED = false;", false)?;
    
    // 2. Invoke WASM setFrame callback
    tab.evaluate(&format!("window.__DIOXUSCUT_SET_FRAME({});", frame), false)?;

    // 3. Deterministically wait for DOM render completion signal (timeout: 2000ms)
    tab.wait_for_xpath("//html[contains(@data-rendered, 'true')]")?; // Or evaluate polling

    // 4. Capture screenshot
    let png_data = tab.capture_screenshot(...)?;
    fs::write(&path, png_data)?;
}
```

---

## 6. Actionable Next Steps for Phase 2 Implementation

1. **Server Manager (`crates/renderer/src/server.rs`)**:
   - Implement `ServerHandle` using `axum` + `tower-http` serving `dist/` on `127.0.0.1:0`.
   - Add `/health` route returning `200 OK`.
2. **Headless WASM Frame Bridge (`crates/core` / `apps/example`)**:
   - Add JS global function binding `window.__DIOXUSCUT_SET_FRAME(frame)` to update Dioxus state.
   - Expose `window.DIOXUSCUT_RENDERED` DOM readiness flag.
3. **Renderer Engine (`crates/renderer/src/render_frames.rs`)**:
   - Replace arbitrary `sleep(30ms)` with deterministic `window.DIOXUSCUT_RENDERED` evaluation loop.

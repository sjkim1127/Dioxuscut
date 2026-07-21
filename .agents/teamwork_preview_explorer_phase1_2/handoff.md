# Handoff Report: Web Application Structure, Serving Strategies & Frame Signaling

**Agent**: Explorer 2  
**Working Directory**: `/Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_explorer_phase1_2`  
**Analysis Report**: `/Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_explorer_phase1_2/analysis.md`  
**Date**: 2026-07-21  

---

## 1. Observation

Direct observations from examining the codebase:

1. **Web Apps Structure**:
   - `apps/example`: Web application entry point (`Cargo.toml` lines 25-28, `features = ["web"]`, `main.rs` lines 53-59 launching `App`).
   - `apps/studio`: Desktop editor application (`Cargo.toml` lines 28-31, `features = ["desktop"]`, `main.rs` lines 20-35 launching `StudioApp`).
   - `data.json`: Schema matching `ExampleProps` in `apps/example/src/main.rs:26-31` (`title`, `subtitle`, `background_start`, `background_end`).

2. **Composition & Hook Propagation**:
   - `crates/core/src/composition.rs:68-69`: `Composition` provides `TimelineContext::new(frame)` and `VideoConfigContext(config)`.
   - `crates/core/src/hooks/use_current_frame.rs:24-27`: `use_current_frame()` retrieves `use_context::<TimelineContext>().frame`.
   - `crates/core/src/sequence.rs:75-94`: `Sequence` calculates active window `[from, from + duration)` and wraps children in `SequenceInner` with `TimelineContext::with_offset(absolute_frame, from)`.

3. **Input Props Reading**:
   - `crates/core/src/hooks/use_input_props.rs:24-46`: Checks `std::env::var("DIOXUSCUT_PROPS")` first; on WASM (`target_arch = "wasm32"`, `feature = "web"`), reads `window.DIOXUSCUT_PROPS`.

4. **Frame Signaling & Disconnect**:
   - `crates/renderer/src/render_frames.rs:112-115`: Renderer executes `tab.evaluate("window.DIOXUSCUT_FRAME = {};", frame)` followed by `tokio::time::sleep(30ms)`.
   - `apps/example/src/main.rs:84-91` and `crates/player/src/player.rs:69-88`: `<Player>` component uses an internal tokio timer loop (16ms ticks) to advance frames, rather than listening to JS global `window.DIOXUSCUT_FRAME`.
   - **Observation**: There is currently no event listener, signal, or export in Dioxus WASM to bridge `window.DIOXUSCUT_FRAME` to Dioxus reactive signal state.

5. **Serving Infrastructure**:
   - `crates/cli/src/main.rs:79-81`: CLI hardcodes `url = "http://localhost:8080"`, noting: `"In a real CLI, we would spawn dx serve on an ephemeral port here."`

---

## 2. Logic Chain

1. **From Observation 1 & 2**: The workspace contains two distinct application binaries: `apps/example` (web WASM target) and `apps/studio` (desktop target). Headless video rendering requires targeting the web application (`apps/example`).
2. **From Observation 2 & 3**: Context propagation (`TimelineContext`, `VideoConfigContext`) and property injection (`use_input_props`) are fully functional inside Dioxus Rust components when the `frame` prop is driven at the root `<Composition frame={frame}>`.
3. **From Observation 4**: Headless Chrome sets `window.DIOXUSCUT_FRAME` in browser memory, but because Dioxus WASM does not listen to this JS global, setting `window.DIOXUSCUT_FRAME` does not cause the Dioxus component tree to re-render to the targeted frame. Relying on `sleep(30ms)` and player playback timers creates severe rendering flakiness and race conditions.
4. **From Observation 5 & Comparative Analysis**:
   - `dx serve` requires external CLI binaries (`dioxus-cli`), has long cold-start compilation overhead (seconds to minutes), and injects dev server WebSockets.
   - An embedded static HTTP server (`axum` / `tower-http` serving pre-compiled `dist/`) binding to `127.0.0.1:0` starts instantly (<1ms), guarantees no port collisions, has zero external dependencies, and provides clean process lifecycle management.
5. **Conclusion**: Production headless rendering requires:
   - Serving pre-built static WASM assets via an embedded HTTP server on ephemeral port `127.0.0.1:0`.
   - Exposing a deterministic JS/WASM bridge (`window.__DIOXUSCUT_SET_FRAME(frame)` and `window.DIOXUSCUT_RENDERED`) so Headless Chrome waits for explicit DOM render completion before taking PNG screenshots.

---

## 3. Caveats

- **WASM Build Prerequisite**: Using the embedded static HTTP server requires that `apps/example` has been built to WASM (`dx build --release --platform web`) prior to headless render execution.
- **Headless Chrome Installation**: Headless Chrome requires Chrome/Chromium to be installed on the host operating system.
- **Parallel Frame Extraction**: Current placeholder renderer extracts frames sequentially; parallel tab extraction can be added in Phase 2 using the same static server port.

---

## 4. Conclusion

1. **Web App Architecture**: `apps/example` is the canonical web rendering target, using `ExampleProps` (matching `data.json`). Context propagation via `<Composition>` and `<Sequence>` correctly offsets frame indices for Dioxus components.
2. **Frame Signaling Spec**: A deterministic WASM/JS bridge (`window.__DIOXUSCUT_SET_FRAME` -> Dioxus signal update -> `window.DIOXUSCUT_RENDERED = true`) must replace current arbitrary sleep delays to guarantee frame-accurate rendering.
3. **Web App Serving Strategy**: Embedded Rust static HTTP server (`axum` / `tower-http`) serving `dist/` on ephemeral port `127.0.0.1:0` is superior to `dx serve` in startup speed (<1ms vs multi-second), stability, port safety, and zero external binary dependencies.

---

## 5. Verification Method

To independently verify these findings:

1. **Inspect Code Files**:
   - `apps/example/src/main.rs`: Verify `ExampleProps` and `<Player>` usage.
   - `crates/core/src/hooks/use_input_props.rs`: Inspect `window.DIOXUSCUT_PROPS` reading logic.
   - `crates/core/src/composition.rs`: Inspect `TimelineContext` context provider.
   - `crates/renderer/src/render_frames.rs`: Confirm `window.DIOXUSCUT_FRAME` injection and `sleep(30ms)` line 112-115.

2. **Run Workspace Compilation**:
   ```bash
   cargo check --workspace
   ```

3. **Verify Handoff Artifacts**:
   - Check analysis report at `/Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_explorer_phase1_2/analysis.md`.

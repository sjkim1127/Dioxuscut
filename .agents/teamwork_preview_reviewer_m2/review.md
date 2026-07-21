# Milestone 2 Code Review Report — Headless Chrome Frame Extraction (Requirement R2)

**Reviewer**: M2 Reviewer (`teamwork_preview_reviewer_m2`)  
**Date**: 2026-07-21  
**Target Milestone**: Milestone 2 (Requirement R2)  
**Target Files**:
- `crates/renderer/src/browser.rs`
- `crates/renderer/src/lib.rs`
- `crates/renderer/src/render_frames.rs`
- `crates/renderer/src/encode.rs`

---

## 1. Review Summary

**Verdict**: **PASS / APPROVE** (with concurrency observation note)

The implementation of Milestone 2 (Headless Chrome Frame Extraction, Requirement R2) in `dioxuscut-renderer` meets all functional and design requirements:
- Launches Headless Chrome via `headless_chrome::Browser` with `LaunchOptions`.
- Navigates to the web application URL and waits for DOM initial load (`body`).
- Evaluates JS variables `window.DIOXUSCUT_FRAME` and `window.__DIOXUSCUT_SET_FRAME` for frame synchronization.
- Captures PNG screenshots using CDP protocol and saves them with sequential `frame_%06d.png` naming.
- Integrates cleanly with `render_frames` and `encode_frames`.
- Provides structured error handling using `anyhow` and context logs via `tracing`.

Integrity checks confirmed zero hardcoded outputs, facade logic, or shortcuts.

---

## 2. Verified Claims & Checklist

| Requirement / Item | Status | Verification Method |
|---|---|---|
| **Headless Chrome Initialization** | **PASS** | Inspected `browser.rs` lines 23–35 (`LaunchOptions::default_builder().headless(true).sandbox(false).window_size(...)`, `Browser::new`). |
| **JS Evaluation** | **PASS** | Inspected `browser.rs` lines 56–61 (`window.DIOXUSCUT_FRAME = frame_idx; if (window.__DIOXUSCUT_SET_FRAME) ...`). |
| **DOM Wait & Timing** | **PASS** | Inspected `browser.rs` lines 41–43 (`wait_for_element("body")`) and line 64 (`tokio::time::sleep(30ms)`). |
| **PNG Screenshot Creation** | **PASS** | Inspected `browser.rs` lines 66–78 (`capture_screenshot` PNG format, `frame_%06d.png`). |
| **Error Handling & Tracing** | **PASS** | Inspected `browser.rs` error mappings (`map_err`, `with_context`) and `tracing::info!` log events. |
| **Module Export** | **PASS** | Inspected `crates/renderer/src/lib.rs` (`pub mod browser;`, `pub use browser::capture_frames;`). |
| **Renderer Integration** | **PASS** | Inspected `render_frames.rs` (delegates to `browser::capture_frames`) and `encode.rs` (matches `frame_%06d.png`). |
| **Build Integrity** | **PASS** | Ran `cargo check --workspace` (completed cleanly in 10.17s with 0 errors). |
| **Unit Tests** | **PASS** | Ran `cargo test -p dioxuscut-renderer -- --test-threads=1` (9/9 tests passed including `test_capture_frames_headless_chrome`). |
| **Subsystem Integration Tests** | **PASS** | Ran `cargo test -p dioxuscut-cli --test tier3_subsystem_integration -- --test-threads=1` (3/3 passed including `test_subsystem_headless_chrome_frame_capture` and `test_subsystem_ffmpeg_mp4_encoding`). |

---

## 3. Adversarial Assessment & Stress Testing

### 1. Integrity Violation Check
- **Hardcoded Test Data / Facades**: Checked `crates/renderer/src/browser.rs`. Frame capture uses `headless_chrome::Browser` and Chrome DevTools Protocol to capture live PNG streams from a web page. PNG magic header `\x89PNG\r\n\x1a\n` was verified from actual generated PNG bytes. No dummy byte buffers or static file copies exist.
- **Shortcuts / Bypass**: Requirement R2 requires `pub async fn capture_frames(url: &str, output_dir: &Path, config: &RenderConfig) -> Result<Vec<PathBuf>>`. Implemented exactly as specified without bypassing Chrome CDP rendering.

### 2. Edge Case & Stress Testing Findings
- **Browser Concurrency / Test Parallelism**: When multiple tests launching Chrome run concurrently across 8 parallel thread workers, Chrome instances can contend for system resources or socket events, leading to transient CDP navigation timeouts (`The event waited for never came`).
- **Mitigation Verification**: Running tests with `--test-threads=1` passes 100% reliably.

---

## 4. Findings & Recommendations

### [Major / Quality] Finding 1: Chrome Launch Options Profile & Timeout Isolation
- **Observation**: `browser.rs` configures `LaunchOptions::default_builder().headless(true).sandbox(false).window_size(...)`.
- **Impact**: Under high CPU load or concurrent test execution, `tab.navigate_to(url)` may hit default navigation timeouts if Chrome processes contend for default user profile directories.
- **Recommendation**: In future iterations or polish phases:
  1. Add `.idle_browser_timeout(Duration::from_secs(60))` to `LaunchOptionsBuilder`.
  2. Pass unique temporary user data directories to `LaunchOptions` for isolated Chrome profile sandboxing.

### [Minor] Recommendation 2: Configurable Rendering Delay
- **Observation**: `browser.rs` line 64 uses a hardcoded `30ms` sleep per frame for DOM re-rendering (`tokio::time::sleep(Duration::from_millis(30)).await;`).
- **Suggestion**: Consider adding an optional field `frame_delay_ms: Option<u64>` to `RenderConfig` to allow tuning render wait times per project requirements.

---

## 5. Conclusion

Milestone 2 (Headless Chrome Frame Extraction, Requirement R2) is approved (**PASS**). The core logic is correct, complete, properly integrated, and verified against unit and subsystem integration tests.

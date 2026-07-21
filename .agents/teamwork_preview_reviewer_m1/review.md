# Review Report — Milestone 1: Automated Web Server Lifecycle (Requirement R1)

## Review Summary

**Verdict**: PASS / APPROVE

Milestone 1 implements an automated, robust web server lifecycle module in `crates/renderer/src/server.rs` along with CLI integration in `crates/cli/src/main.rs` and `crates/cli/src/lib.rs`. The code adheres strictly to Requirement R1, passing all unit tests and workspace checks.

---

## 1. Integrity Violation Check

| Integrity Check | Status | Details / Evidence |
|---|---|---|
| No hardcoded test results / expected outputs | **PASS** | `crates/renderer/src/server.rs` unit tests dynamically allocate temporary directories, spin up real local HTTP servers on ephemeral ports, and make actual HTTP GET requests via `reqwest`. |
| No dummy or facade implementations | **PASS** | Embedded static server uses real `axum::serve`, `tower_http::services::ServeDir`, and `tokio::net::TcpListener`. External command mode spawns actual subprocesses via `tokio::process::Command`. |
| No shortcuts bypassing core work | **PASS** | Server lifecycle, readiness polling (`/health` & `/`), dynamic port allocation, and termination logic are fully implemented in Rust. |
| No fabricated verification outputs | **PASS** | Independent execution of `cargo test -p dioxuscut-renderer` (4/4 passed) and `cargo check --workspace` (0 errors) confirmed all claims in worker handoff. |
| Independent verification | **PASS** | Code, tests, and CLI execution paths were directly inspected and executed by Reviewer. |

---

## 2. Requirement R1 Compliance & Findings

### Core Functional Requirements
1. **Dynamic Port Allocation (`127.0.0.1:0`)**:
   - `spawn_static_server` binds `TcpListener` directly to `127.0.0.1:0` (or configured port), allowing OS ephemeral port selection.
   - `find_available_port()` provides dynamic port detection for command mode when `port == 0`.
2. **Readiness Polling**:
   - `poll_health_check()` queries both `{url}/health` (explicit Axum route returning `"OK"`) and `{url}/` with configurable timeout (`Duration::from_secs(10)` default) and retry interval (`50ms`/`100ms`).
   - If health check fails or times out in command server mode, child process is explicitly killed before returning `ServerError::HealthCheckTimeout`.
3. **Clean Termination (`ServerHandle`)**:
   - `ServerHandle::stop()` sends oneshot channel shutdown signal to Axum server task or kills child process asynchronously.
   - `Drop for ServerHandle` implements synchronous cleanup fallback (`tx.send(())` and `child.start_kill()`), ensuring no leak of server tasks or orphaned processes even on drop or panic.
4. **Error Handling**:
   - Dedicated `ServerError` enum (`thiserror::Error`) covering `Io`, `BindError`, `HealthCheckTimeout`, `ReqwestError`, `ProcessExited`, and `AlreadyStopped`.
5. **CLI Integration**:
   - `dioxuscut-cli` exposes `--port`, `--web-dir`, and `--server-url` parameters.
   - Standard render pipeline auto-spawns server if external `--server-url` is omitted, and cleanly stops server post-render.

---

## 3. Findings & Observations

### Findings

#### [Minor] Finding 1: Unused Import Warning in `crates/cli/src/lib.rs`
- **Location**: `crates/cli/src/lib.rs:6`
- **What**: `use std::path::{Path, PathBuf};` raises a compiler warning because `Path` is not directly referenced in that file.
- **Why**: Clean build output improves project maintainability.
- **Suggestion**: Remove `Path` from the import list in `crates/cli/src/lib.rs:6`.

#### [Minor / Informational] Finding 2: Ephemeral Port TOCTOU in Command Mode
- **Location**: `crates/renderer/src/server.rs:264-268`
- **What**: In `ServeMode::Command`, `find_available_port()` opens and immediately closes a socket at `127.0.0.1:0` to pick a free port before passing `PORT` env var to the child process.
- **Why**: Minor time-of-check to time-of-use (TOCTOU) window where another process on the system could bind to that port.
- **Suggestion**: In practice for CLI subprocesses (e.g. `dx serve`), this is standard. No immediate code change required, but could support port retry if child fails to bind.

---

## 4. Stress Testing & Attack Surface Analysis

- **Scenario 1: Drop ServerHandle without calling `.stop()`**
  - Result: `Drop` implementation sends oneshot signal / calls `start_kill()`. Tested in unit test `test_server_drop_cleanup`, confirmed server port closed after drop.
- **Scenario 2: Health check failure / timeout during command spawn**
  - Result: Error path locks child process Mutex and calls `kill().await`, preventing orphan process leaks.
- **Scenario 3: Embedded static server path handling**
  - Result: Axum `ServeDir` created with `append_index_html_on_directories(true)` and `CorsLayer::permissive()`, ensuring proper asset resolution and zero CORS blocks for browser renderer.

---

## 5. Verified Claims

1. `cargo test -p dioxuscut-renderer` → 4 passed, 0 failed → **PASS**
2. `cargo check --workspace` → 0 errors → **PASS**
3. `ServerHandle` Drop & explicit `.stop()` implementation → verified in `crates/renderer/src/server.rs` → **PASS**
4. `/health` & `/` readiness polling logic → verified in `poll_health_check()` → **PASS**

---

## Conclusion

Milestone 1 is **APPROVED**. Requirement R1 is fully satisfied with high code quality, robust error handling, clean process lifecycle management, and comprehensive unit test coverage.

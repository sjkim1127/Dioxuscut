#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dioxuscut_rasterizer::{BackendCapabilities, WebWorkerMessage, WEB_WORKER_PROTOCOL_VERSION};

#[tauri::command]
fn backend_capabilities() -> BackendCapabilities {
    BackendCapabilities {
        native_scene: true,
        browser_runtime: true,
        gpu_accelerated: true,
        supports_streaming: false,
    }
}

#[tauri::command]
fn web_worker_protocol() -> serde_json::Value {
    serde_json::json!({
        "version": WEB_WORKER_PROTOCOL_VERSION,
        "messages": ["ready", "render", "frame", "error", "shutdown"],
        "example": serde_json::to_value(WebWorkerMessage::Ready {
            protocol: WEB_WORKER_PROTOCOL_VERSION,
        }).expect("protocol message is serializable"),
    })
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            backend_capabilities,
            web_worker_protocol
        ])
        .run(tauri::generate_context!())
        .expect("error while running Dioxuscut Studio");
}

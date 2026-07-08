// Prevents additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().any(|a| a == "--wisp-mcp-oneshot") {
        wisp_tauri::run_mcp_oneshot_cli();
    } else if std::env::args().any(|a| a == "--wisp-mcp-bridge") {
        wisp_tauri::run_mcp_bridge_cli();
    } else {
        wisp_tauri::run();
    }
}

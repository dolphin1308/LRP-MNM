mod commands;
mod proxy;

use commands::AppState;
use proxy::ProxyServer;
use std::sync::Arc;
use tokio::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            proxy: Arc::new(Mutex::new(ProxyServer::new())),
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_proxy,
            commands::stop_proxy,
            commands::get_status,
            commands::get_log_buffer,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

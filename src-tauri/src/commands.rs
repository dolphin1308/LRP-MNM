use crate::proxy::{LogEntry, ProxyConfig, ProxyServer};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

/// 全局代理状态（通过 tauri::State 注入）
pub struct AppState {
    pub proxy: Arc<Mutex<ProxyServer>>,
}

/// 前端传入的配置
#[derive(Serialize, Deserialize)]
pub struct StartRequest {
    pub port: u16,
    pub target_url: String,
    pub model_mapping: HashMap<String, String>,
}

/// 返回给前端的状态
#[derive(Serialize, Deserialize)]
pub struct StatusResponse {
    pub running: bool,
    pub port: u16,
}

#[tauri::command]
pub async fn start_proxy(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    request: StartRequest,
) -> Result<StatusResponse, String> {
    let config = ProxyConfig {
        port: request.port,
        target_url: request.target_url,
        model_mapping: request.model_mapping,
    };

    let mut proxy = state.proxy.lock().await;
    proxy.start(config, app_handle).await?;

    Ok(StatusResponse {
        running: proxy.running,
        port: proxy.port,
    })
}

#[tauri::command]
pub async fn stop_proxy(
    state: tauri::State<'_, AppState>,
) -> Result<StatusResponse, String> {
    let mut proxy = state.proxy.lock().await;
    proxy.stop().await?;

    Ok(StatusResponse {
        running: proxy.running,
        port: proxy.port,
    })
}

#[tauri::command]
pub async fn get_status(
    state: tauri::State<'_, AppState>,
) -> Result<StatusResponse, String> {
    let proxy = state.proxy.lock().await;
    Ok(StatusResponse {
        running: proxy.running,
        port: proxy.port,
    })
}

#[tauri::command]
pub async fn get_log_buffer(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<LogEntry>, String> {
    let proxy = state.proxy.lock().await;
    let buf = proxy.log_buffer.lock().await;
    Ok(buf.clone())
}

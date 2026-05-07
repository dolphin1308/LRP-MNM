use axum::{
    body::Body,
    http::{Method, Request, Response, StatusCode},
    Router,
};
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tauri::Emitter;
use tokio::{net::TcpListener, sync::{Mutex, oneshot}};

/// 日志消息，推送到前端
#[derive(Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub level: String,
    pub message: String,
}

/// 代理配置
#[derive(Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub port: u16,
    pub target_url: String,
    pub model_mapping: HashMap<String, String>,
}

/// 代理内部共享状态
#[derive(Clone)]
pub struct ProxyInner {
    pub config: ProxyConfig,
    pub http_client: Client,
}

/// 代理服务器管理器
pub struct ProxyServer {
    inner: Option<ProxyInner>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    pub running: bool,
    pub port: u16,
    pub log_buffer: Arc<Mutex<Vec<LogEntry>>>,
}

impl ProxyServer {
    pub fn new() -> Self {
        Self {
            inner: None,
            shutdown_tx: None,
            running: false,
            port: 8080,
            log_buffer: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn start(
        &mut self,
        config: ProxyConfig,
        app_handle: tauri::AppHandle,
    ) -> Result<(), String> {
        if self.running {
            return Err("Proxy is already running".to_string());
        }

        let port = config.port;
        let log_buffer = self.log_buffer.clone();

        let inner = ProxyInner {
            config: config.clone(),
            http_client: Client::builder()
                .pool_max_idle_per_host(10)
                .build()
                .map_err(|e| format!("Failed to create HTTP client: {}", e))?,
        };

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let app_handle_clone = app_handle.clone();
        let inner_clone = inner.clone();

        tokio::spawn(async move {
            let app_handle = app_handle_clone;

            // 用闭包捕获共享状态，Router::fallback 处理所有请求
            let inner = inner_clone;
            let log_buf = log_buffer;

            let app = Router::new().fallback({
                let inner = inner.clone();
                let app_handle = app_handle.clone();
                let log_buf = log_buf.clone();
                move |req: Request<Body>| {
                    let inner = inner.clone();
                    let app_handle = app_handle.clone();
                    let log_buf = log_buf.clone();
                    async move {
                        handle_proxy(req, inner, app_handle, log_buf).await
                    }
                }
            });

            let addr = SocketAddr::from(([127, 0, 0, 1], port));
            let listener = match TcpListener::bind(addr).await {
                Ok(l) => l,
                Err(e) => {
                    let _ = app_handle.emit("proxy-log", LogEntry {
                        level: "error".to_string(),
                        message: format!("Failed to bind port {}: {}", port, e),
                    });
                    return;
                }
            };

            let _ = app_handle.emit("proxy-log", LogEntry {
                level: "info".to_string(),
                message: format!("Proxy started on port {}", port),
            });

            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .ok();

            let _ = app_handle.emit("proxy-log", LogEntry {
                level: "info".to_string(),
                message: "Proxy stopped".to_string(),
            });
        });

        self.inner = Some(inner);
        self.shutdown_tx = Some(shutdown_tx);
        self.running = true;
        self.port = port;

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<(), String> {
        if !self.running {
            return Err("Proxy is not running".to_string());
        }

        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        self.inner = None;
        self.running = false;
        Ok(())
    }
}

/// 处理所有代理请求
async fn handle_proxy(
    request: Request<Body>,
    inner: ProxyInner,
    app_handle: tauri::AppHandle,
    log_buffer: Arc<Mutex<Vec<LogEntry>>>,
) -> Response<Body> {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let headers = request.headers().clone();
    let path = uri.path().to_string();
    let query = uri.query().unwrap_or("").to_string();
    let full_path = if query.is_empty() {
        path.clone()
    } else {
        format!("{}?{}", path, query)
    };

    // 记录请求
    push_log(&app_handle, &log_buffer, LogEntry {
        level: "request".to_string(),
        message: format!("{} {}", method, path),
    }).await;

    // 读取请求体
    let body_bytes = match axum::body::to_bytes(request.into_body(), usize::MAX).await {
        Ok(b) => b,
        Err(e) => {
            push_log(&app_handle, &log_buffer, LogEntry {
                level: "error".to_string(),
                message: format!("Failed to read body: {}", e),
            }).await;
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Body::from("{\"error\":\"failed to read body\"}"))
                .unwrap_or_else(|_| {
                    Response::new(Body::from("internal proxy error"))
                });
        }
    };

    // 替换模型名
    let modified_body = if method == Method::POST && !body_bytes.is_empty() {
        modify_body(&body_bytes, &inner.config.model_mapping, &app_handle, &log_buffer).await
    } else {
        body_bytes.to_vec()
    };

    // 构建目标 URL
    let base = inner.config.target_url.trim_end_matches('/');
    let target_url = format!("{}{}", base, full_path);

    // 记录转发的目标 URL
    push_log(&app_handle, &log_buffer, LogEntry {
        level: "info".to_string(),
        message: format!("Forward to: {}", target_url),
    }).await;

    // 构建转发请求
    let mut req_builder = inner.http_client.request(method.clone(), &target_url);
    for (key, value) in headers.iter() {
        if key != "host" {
            req_builder = req_builder.header(key.clone(), value.clone());
        }
    }
    if method == Method::POST || method == Method::PUT || method == Method::PATCH {
        req_builder = req_builder.body(modified_body);
    }

    // 发送
    let upstream = match req_builder.send().await {
        Ok(r) => r,
        Err(e) => {
            push_log(&app_handle, &log_buffer, LogEntry {
                level: "error".to_string(),
                message: format!("Forward failed: {}", e),
            }).await;
            return Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .header("Content-Type", "application/json")
                .body(Body::from(format!(
                    "{{\"error\":\"proxy_error\",\"message\":\"{}\"}}", e
                )))
                .unwrap_or_else(|_| {
                    Response::new(Body::from("internal proxy error"))
                });
        }
    };

    let status = upstream.status();
    push_log(&app_handle, &log_buffer, LogEntry {
        level: "done".to_string(),
        message: format!("Response: {}", status.as_u16()),
    }).await;

    // 构建响应头
    let mut resp = Response::builder().status(status.as_u16());
    for (key, value) in upstream.headers().iter() {
        let k = key.as_str().to_lowercase();
        if k == "transfer-encoding" || k == "content-length" || k == "connection" {
            continue;
        }
        resp = resp.header(key.clone(), value.clone());
    }
    resp = resp.header("Transfer-Encoding", "chunked");
    resp = resp.header("Connection", "close");

    // 流式转发（SSE 支持）
    let stream = upstream.bytes_stream();
    let body = Body::from_stream(stream.map(|r| {
        r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }));

    resp.body(body).unwrap_or_else(|_| {
        Response::new(Body::from("internal proxy error"))
    })
}

/// 替换 JSON body 中的 model 字段
async fn modify_body(
    body: &[u8],
    mapping: &HashMap<String, String>,
    app_handle: &tauri::AppHandle,
    log_buffer: &Arc<Mutex<Vec<LogEntry>>>,
) -> Vec<u8> {
    let parsed: Result<serde_json::Value, _> = serde_json::from_slice(body);
    match parsed {
        Ok(mut json) => {
            if let Some(obj) = json.as_object_mut() {
                if let Some(model) = obj.get("model").and_then(|v| v.as_str()) {
                    if let Some(target) = mapping.get(model) {
                        push_log(app_handle, log_buffer, LogEntry {
                            level: "map".to_string(),
                            message: format!("Model: {} → {}", model, target),
                        }).await;
                        obj.insert("model".into(), serde_json::Value::String(target.clone()));
                        return serde_json::to_vec(&json).unwrap_or(body.to_vec());
                    } else {
                        push_log(app_handle, log_buffer, LogEntry {
                            level: "info".to_string(),
                            message: format!("Model '{}' has no mapping", model),
                        }).await;
                    }
                }
            }
            body.to_vec()
        }
        Err(e) => {
            push_log(app_handle, log_buffer, LogEntry {
                level: "warn".to_string(),
                message: format!("JSON parse error: {}", e),
            }).await;
            body.to_vec()
        }
    }
}

/// 推送日志到前端并缓存
async fn push_log(
    app_handle: &tauri::AppHandle,
    log_buffer: &Arc<Mutex<Vec<LogEntry>>>,
    entry: LogEntry,
) {
    let _ = app_handle.emit("proxy-log", entry.clone());
    let mut buf = log_buffer.lock().await;
    buf.push(entry);
    if buf.len() > 500 {
        buf.drain(0..200);
    }
}

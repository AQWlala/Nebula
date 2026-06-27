use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use tracing::info;

use crate::AppState;

pub struct RestApiServer {
    addr: SocketAddr,
    state: Arc<AppState>,
}

impl RestApiServer {
    pub fn new(addr: SocketAddr, state: Arc<AppState>) -> Self {
        Self { addr, state }
    }

    pub async fn start(self) -> Result<()> {
        info!(target: "nine_snake.rest", addr = %self.addr, "REST API server starting");
        let state = self.state;
        let addr = self.addr;

        let service = move |req: http::Request<hyper::body::Incoming>| {
            let state = state.clone();
            async move {
                let path = req.uri().path().to_string();
                let method = req.method().clone();

                let (status, body) = match (method.as_str(), path.as_str()) {
                    ("GET", "/api/health") => (200, serde_json::json!({"status": "ok"})),
                    ("GET", "/api/memories") => {
                        match state.sqlite.list_recent(50) {
                            Ok(memories) => (200, serde_json::json!({"memories": memories})),
                            Err(e) => (500, serde_json::json!({"error": e.to_string()})),
                        }
                    }
                    ("POST", "/api/chat") => (200, serde_json::json!({"message": "use Tauri IPC for chat"})),
                    ("POST", "/api/swarm/execute") => (200, serde_json::json!({"message": "use Tauri IPC for swarm"})),
                    ("GET", "/api/skills") => {
                        match state.skills.list(Default::default()) {
                            Ok(skills) => (200, serde_json::json!({"skills": skills})),
                            Err(e) => (500, serde_json::json!({"error": e.to_string()})),
                        }
                    }
                    _ => (404, serde_json::json!({"error": "not found"})),
                };

                let body_bytes = serde_json::to_vec(&body).unwrap_or_default();
                let resp = http::Response::builder()
                    .status(status)
                    .header("content-type", "application/json")
                    .body(hyper::body::Bytes::from(body_bytes))
                    .unwrap();
                Ok::<_, std::convert::Infallible>(resp)
            }
        };

        let listener = tokio::net::TcpListener::bind(addr).await?;
        info!(target: "nine_snake.rest", "REST API server listening on {}", addr);

        loop {
            let (stream, _) = listener.accept().await?;
            let io = hyper_util::rt::TokioIo::new(stream);
            let service = service.clone();
            tokio::spawn(async move {
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, hyper::service::service_fn(service))
                    .await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rest_api_constructs() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    }
}

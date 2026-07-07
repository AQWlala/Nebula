use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
use http_body_util::BodyExt;
use serde::Deserialize;
use tracing::info;

use crate::llm::ChatMessage;
use crate::swarm::SwarmTask;
use crate::AppState;

type BoxBody = http_body_util::combinators::BoxBody<Bytes, Infallible>;

fn full_body(data: Vec<u8>) -> BoxBody {
    http_body_util::Full::new(Bytes::from(data)).boxed()
}

fn json_response(status: u16, body: serde_json::Value) -> hyper::Response<BoxBody> {
    let body_bytes = serde_json::to_vec(&body).unwrap_or_default();
    hyper::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(full_body(body_bytes))
        .unwrap()
}

async fn read_body(body: &mut hyper::body::Incoming) -> Option<serde_json::Value> {
    // BodyExt::collect takes ownership, so we use frame-based reading.
    let mut data = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.ok()?;
        if let Ok(chunk) = frame.into_data() {
            data.extend_from_slice(&chunk);
        }
    }
    serde_json::from_slice(&data).ok()
}

#[derive(Debug, Clone, Deserialize)]
struct ChatRequest {
    messages: Vec<(String, String)>,
    model: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MemorySearchRequest {
    query: String,
    k: Option<usize>,
}

#[cfg(feature = "rest-api")]
pub struct RestApiServer {
    addr: SocketAddr,
    state: Arc<AppState>,
}

#[cfg(feature = "rest-api")]
impl RestApiServer {
    pub fn new(addr: SocketAddr, state: Arc<AppState>) -> Self {
        Self { addr, state }
    }

    pub async fn start(self) -> Result<()> {
        info!(target: "nebula.rest", addr = %self.addr, "REST API server starting");
        let state = self.state;
        let addr = self.addr;
        let api_token = state.config.rest_api_token.clone();

        let service = move |req: hyper::Request<hyper::body::Incoming>| {
            let state = state.clone();
            let api_token = api_token.clone();
            async move {
                let path = req.uri().path().to_string();
                let method = req.method().clone();

                // T-S2-B-03a: 认证检查（在路由匹配之前）
                if let Err((status, msg)) = crate::api::auth::check_auth(&req, &api_token) {
                    return Ok::<_, Infallible>(json_response(
                        status,
                        serde_json::json!({"error": msg}),
                    ));
                }

                let (mut parts, body) = req.into_parts();
                let _ = &mut parts; // suppress unused

                // Read request body for POST routes
                let body_json = read_body(&mut body.into()).await;

                let (status, resp_body) = match (method.as_str(), path.as_str()) {
                    ("GET", "/api/health") => (200, serde_json::json!({"status": "ok"})),

                    ("GET", "/api/memories") => match state.sqlite.list_recent(50).await {
                        Ok(memories) => (200, serde_json::json!({"memories": memories})),
                        Err(e) => (500, serde_json::json!({"error": e.to_string()})),
                    },

                    ("GET", "/api/skills") => match state.skills.list_skills(Default::default()) {
                        Ok(skills) => (200, serde_json::json!({"skills": skills})),
                        Err(e) => (500, serde_json::json!({"error": e.to_string()})),
                    },

                    ("POST", "/api/chat") => {
                        let body_val = match body_json {
                            Some(v) => v,
                            None => {
                                return Ok::<_, Infallible>(json_response(
                                    400,
                                    serde_json::json!({"error": "invalid request body"}),
                                ))
                            }
                        };
                        let chat_req: ChatRequest = match serde_json::from_value(body_val) {
                            Ok(r) => r,
                            Err(e) => {
                                return Ok::<_, Infallible>(json_response(
                                    400,
                                    serde_json::json!({"error": format!("invalid chat request: {}", e)}),
                                ))
                            }
                        };
                        let msgs: Vec<ChatMessage> = chat_req
                            .messages
                            .into_iter()
                            .map(|(role, content)| ChatMessage {
                                role,
                                content,
                                ..Default::default()
                            })
                            .collect();
                        match chat_req.model.as_deref() {
                            Some(model) if !model.is_empty() => {
                                match state.llm.chat_with_model(model, msgs).await {
                                    Ok(resp) => (
                                        200,
                                        serde_json::json!({
                                            "role": resp.message.role,
                                            "content": resp.message.content,
                                            "model": resp.model,
                                            "eval_count": resp.eval_count,
                                        }),
                                    ),
                                    Err(e) => (500, serde_json::json!({"error": e.to_string()})),
                                }
                            }
                            _ => match state.llm.chat(msgs).await {
                                Ok(resp) => (
                                    200,
                                    serde_json::json!({
                                        "role": resp.message.role,
                                        "content": resp.message.content,
                                        "model": resp.model,
                                        "eval_count": resp.eval_count,
                                    }),
                                ),
                                Err(e) => (500, serde_json::json!({"error": e.to_string()})),
                            },
                        }
                    }
                    ("POST", "/api/swarm/execute") => {
                        let body_val = match body_json {
                            Some(v) => v,
                            None => {
                                return Ok::<_, Infallible>(json_response(
                                    400,
                                    serde_json::json!({"error": "invalid request body"}),
                                ))
                            }
                        };
                        let task: SwarmTask = match serde_json::from_value(body_val) {
                            Ok(t) => t,
                            Err(e) => {
                                return Ok::<_, Infallible>(json_response(
                                    400,
                                    serde_json::json!({"error": format!("invalid swarm task: {}", e)}),
                                ))
                            }
                        };
                        match state.swarm.execute(task).await {
                            Ok(report) => (
                                200,
                                serde_json::json!({
                                    "task": report.task,
                                    "outputs": report.outputs,
                                    "success_count": report.success_count,
                                    "failure_count": report.failure_count,
                                    "approved": report.approved,
                                }),
                            ),
                            Err(e) => (500, serde_json::json!({"error": e.to_string()})),
                        }
                    }
                    ("POST", "/api/memory/search") => {
                        let body_val = match body_json {
                            Some(v) => v,
                            None => {
                                return Ok::<_, Infallible>(json_response(
                                    400,
                                    serde_json::json!({"error": "invalid request body"}),
                                ))
                            }
                        };
                        let search_req: MemorySearchRequest = match serde_json::from_value(body_val)
                        {
                            Ok(r) => r,
                            Err(e) => {
                                return Ok::<_, Infallible>(json_response(
                                    400,
                                    serde_json::json!({"error": format!("invalid search request: {}", e)}),
                                ))
                            }
                        };
                        let k = search_req.k.unwrap_or(10);
                        match state
                            .sponge
                            .search_with_graph(&search_req.query, k, None)
                            .await
                        {
                            Ok(hits) => {
                                let results: Vec<serde_json::Value> = hits
                                    .into_iter()
                                    .map(|(id, score)| serde_json::json!({"memory_id": id, "score": score}))
                                    .collect();
                                (200, serde_json::json!({"results": results}))
                            }
                            Err(e) => (500, serde_json::json!({"error": e.to_string()})),
                        }
                    }
                    _ => (404, serde_json::json!({"error": "not found"})),
                };

                Ok::<_, Infallible>(json_response(status, resp_body))
            }
        };

        let listener = tokio::net::TcpListener::bind(addr).await?;
        info!(target: "nebula.rest", "REST API server listening on {}", addr);

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

#[cfg(not(feature = "rest-api"))]
pub struct RestApiServer {
    _private: (),
}

#[cfg(not(feature = "rest-api"))]
impl RestApiServer {
    pub fn new(_: SocketAddr, _: Arc<AppState>) -> Self {
        Self { _private: () }
    }

    pub async fn start(self) -> Result<()> {
        tracing::warn!("REST API server disabled (rest-api feature not enabled)");
        Ok(())
    }
}

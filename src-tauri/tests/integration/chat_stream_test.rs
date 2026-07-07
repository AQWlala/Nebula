//! T-S1-B-01c: `chat_stream` 流式集成测试。
//!
//! 覆盖路径：
//! 1. 正常 Ollama NDJSON 流式
//! 2. 正常 DeepSeek SSE 流式
//! 3. 取消流（前端提前 drop Channel，对应命令中 `on_token.send()` 失败 break）
//! 4. DeepSeek 无 api_key 回退 Ollama 路径
//! 5. 连接死端口 → 流首项为 Err
//! 6. server 中断（EOF 前无 `done:true`）→ incomplete 标记
//!
//! 注：Tauri 命令 `chat_stream` 依赖 `State<AppState>` + `ipc::Channel`，
//! 无法在纯集成测试中构造。本文件直接测试底层 `LlmGateway::chat_stream()`，
//! 该方法返回 `BoxStream<Result<StreamToken>>`，是命令 shim 的全部流式逻辑来源。

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use nebula_lib::llm::{ChatMessage, LlmGateway, OllamaClient, StreamToken};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// mock HTTP server（手写，因 Cargo.toml 无 wiremock/mockito）
// ---------------------------------------------------------------------------

/// 启动一个一次性 mock HTTP server，读取请求后回写固定 body。
/// 返回 base_url（如 `http://127.0.0.1:38291`）。
async fn mock_http_server(body: Vec<u8>, content_type: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let url = format!("http://{addr}");
    let ct = content_type.to_string();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("accept");
        // 读取并丢弃请求（含 body），直到读够或超时。
        let mut buf = [0u8; 4096];
        let _ = tokio::time::timeout(Duration::from_secs(2), sock.read(&mut buf)).await;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        let _ = sock.write_all(&body).await;
        let _ = sock.flush().await;
        // 保持连接一小段时间，确保 reqwest 读完 body。
        tokio::time::sleep(Duration::from_millis(50)).await;
    });
    url
}

/// 启动一个慢速 mock HTTP server，分段发送 body（每段之间 sleep）。
/// 用于取消流测试：验证提前 drop stream 不会阻塞。
async fn mock_http_server_chunked(chunks: Vec<(Vec<u8>, Duration)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let url = format!("http://{addr}");
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("accept");
        let mut buf = [0u8; 4096];
        let _ = tokio::time::timeout(Duration::from_secs(2), sock.read(&mut buf)).await;
        // 发送 chunked 响应头
        let header = "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
        let _ = sock.write_all(header.as_bytes()).await;
        let _ = sock.flush().await;
        for (chunk, delay) in chunks {
            tokio::time::sleep(delay).await;
            let len_line = format!("{:X}\r\n", chunk.len());
            let _ = sock.write_all(len_line.as_bytes()).await;
            let _ = sock.write_all(&chunk).await;
            let _ = sock.write_all(b"\r\n").await;
            let _ = sock.flush().await;
        }
        let _ = sock.write_all(b"0\r\n\r\n").await;
    });
    url
}

fn user_msg() -> Vec<ChatMessage> {
    vec![ChatMessage::user("ping")]
}

/// 把流消费成 `Vec<StreamToken>`，拼接 text。
async fn collect_stream(
    stream: futures::stream::BoxStream<'static, anyhow::Result<StreamToken>>,
) -> (Vec<StreamToken>, String) {
    let mut tokens = Vec::new();
    let mut full = String::new();
    let mut s = stream;
    while let Some(r) = s.next().await {
        match r {
            Ok(t) => {
                if !t.text.is_empty() {
                    full.push_str(&t.text);
                }
                tokens.push(t);
            }
            Err(_) => break,
        }
    }
    (tokens, full)
}

// ---------------------------------------------------------------------------
// 1. 正常流：Ollama NDJSON 路径
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ollama_ndjson_normal_stream() {
    // 构造 NDJSON body：2 个 token + done
    let body = concat!(
        "{\"message\":{\"content\":\"Hello\"},\"done\":false}\n",
        "{\"message\":{\"content\":\" world\"},\"done\":false}\n",
        "{\"done\":true}\n",
    )
    .as_bytes()
    .to_vec();
    let url = mock_http_server(body, "application/x-ndjson").await;

    let client = Arc::new(OllamaClient::new_with_timeout(url, Duration::from_secs(5)));
    let gw = LlmGateway::new(client, "test-model", "ollama", None, None, None, None, None);

    let stream = gw.chat_stream(user_msg());
    let (tokens, full) = collect_stream(stream).await;

    assert_eq!(full, "Hello world", "concatenated content must match");
    assert!(
        tokens.last().map(|t| t.done).unwrap_or(false),
        "last token must be done"
    );
    assert!(
        !tokens.iter().any(|t| t.incomplete),
        "no incomplete tokens in clean stream"
    );
}

// ---------------------------------------------------------------------------
// 2. 正常流：DeepSeek SSE 路径
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deepseek_sse_normal_stream() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\" there\"}}]}\n\n",
        "data: [DONE]\n\n",
    )
    .as_bytes()
    .to_vec();
    let url = mock_http_server(body, "text/event-stream").await;

    // DeepSeek 路径要求 provider="deepseek" 且 api_key 非空。
    // OllamaClient 仍需传入（primary 字段始终存在），但流式不会用它。
    let client = Arc::new(OllamaClient::new_with_timeout(
        "http://127.0.0.1:1",
        Duration::from_secs(2),
    ));
    let gw = LlmGateway::new(
        client,
        "deepseek-chat",
        "deepseek",
        Some(url),
        Some("test-key".to_string()),
        None,
        None,
        None,
    );

    let stream = gw.chat_stream(user_msg());
    let (tokens, full) = collect_stream(stream).await;

    assert_eq!(full, "Hi there", "DeepSeek SSE content must match");
    assert!(
        tokens.last().map(|t| t.done).unwrap_or(false),
        "must receive [DONE]"
    );
}

// ---------------------------------------------------------------------------
// 3. 取消流：提前 drop stream 不阻塞
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ollama_stream_cancel_by_dropping() {
    // 慢速 server：发 2 个 token 后停顿很久。
    // 测试端只 take(2)，验证能在 server 还没发完时返回。
    let chunks = vec![
        (
            b"{\"message\":{\"content\":\"A\"},\"done\":false}\n".to_vec(),
            Duration::from_millis(10),
        ),
        (
            b"{\"message\":{\"content\":\"B\"},\"done\":false}\n".to_vec(),
            Duration::from_millis(10),
        ),
        (
            b"{\"done\":true}\n".to_vec(),
            Duration::from_secs(30), // 故意很长
        ),
    ];
    let url = mock_http_server_chunked(chunks).await;

    let client = Arc::new(OllamaClient::new_with_timeout(url, Duration::from_secs(5)));
    let gw = LlmGateway::new(client, "test-model", "ollama", None, None, None, None, None);

    let stream = gw.chat_stream(user_msg());
    // take(2) 后 stream 被 drop，模拟前端 on_token.send() 失败后 break。
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        collect_stream(Box::pin(stream.take(2))),
    )
    .await;

    assert!(
        result.is_ok(),
        "take(2) must return within 3s even though server stalls"
    );
    let (tokens, full) = result.unwrap();
    assert_eq!(full, "AB", "should have consumed exactly 2 tokens");
    assert_eq!(tokens.len(), 2);
}

// ---------------------------------------------------------------------------
// 4. DeepSeek 路径分发验证：provider=deepseek 但无 api_key → 回退 ollama
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deepseek_without_api_key_falls_back_to_ollama() {
    // provider="deepseek" 但 api_key=None → deepseek 字段为 None → 走 ollama 路径。
    // 用 NDJSON body 验证确实走了 ollama 解析逻辑。
    let body = b"{\"message\":{\"content\":\"fallback\"},\"done\":true}\n".to_vec();
    let url = mock_http_server(body, "application/x-ndjson").await;

    let client = Arc::new(OllamaClient::new_with_timeout(
        url.clone(),
        Duration::from_secs(5),
    ));
    let gw = LlmGateway::new(
        client,
        "test-model",
        "deepseek",
        Some(url),
        None,
        None,
        None,
        None,
    );

    let stream = gw.chat_stream(user_msg());
    let (tokens, full) = collect_stream(stream).await;
    assert_eq!(full, "fallback", "should fall back to ollama NDJSON path");
    assert!(tokens.last().map(|t| t.done).unwrap_or(false));
}

// ---------------------------------------------------------------------------
// 5. 错误路径：连接死端口 → 流首项为 Err
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ollama_stream_dead_port_emits_error() {
    let client = Arc::new(OllamaClient::new_with_timeout(
        "http://127.0.0.1:1",
        Duration::from_secs(2),
    ));
    let gw = LlmGateway::new(client, "test-model", "ollama", None, None, None, None, None);

    let mut stream = gw.chat_stream(user_msg());
    let first = stream.next().await;
    assert!(first.is_some(), "stream must emit at least one item");
    assert!(
        first.unwrap().is_err(),
        "first item must be Err on dead port"
    );
}

// ---------------------------------------------------------------------------
// 6. incomplete 标记：流被 server 中断（EOF 前无 done:true）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ollama_stream_incomplete_on_eof_without_done() {
    // server 发 1 个 token 后直接关连接，不发 done:true。
    let body = b"{\"message\":{\"content\":\"partial\"},\"done\":false}\n".to_vec();
    let url = mock_http_server(body, "application/x-ndjson").await;

    let client = Arc::new(OllamaClient::new_with_timeout(url, Duration::from_secs(5)));
    let gw = LlmGateway::new(client, "test-model", "ollama", None, None, None, None, None);

    let stream = gw.chat_stream(user_msg());
    let (tokens, full) = collect_stream(stream).await;
    assert_eq!(full, "partial");
    let last = tokens.last().expect("must have tokens");
    assert!(last.done, "stream end yields done=true");
    assert!(
        last.incomplete,
        "incomplete must be true (text was emitted but no done:true)"
    );
}

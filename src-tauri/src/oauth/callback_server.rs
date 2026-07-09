//! T-E-C-18: 本地 OAuth 回调服务器。
//!
//! 授权码流程的第三步:provider 在用户同意授权后,把浏览器重定向到
//! `redirect_uri`(本模块监听的 `http://127.0.0.1:<port>/callback`)。
//! 本模块从 query string 提取 `code` 和 `state`,通过 oneshot channel
//! 送回 [`OAuthManager`](super::manager::OAuthManager) 完成交换。
//!
//! ## 实现
//!
//! 用裸 `tokio::net::TcpListener` 手写最小 HTTP/1.0 响应,不依赖 axum /
//! hyper,零新耦合。服务器只处理一个请求(收到 code 后即关闭)。
//!
//! ## 安全
//!
//! * 只绑定 `127.0.0.1`(环回),不对外网暴露。
//! * `state` 参数由 [`super::generate_state`] 生成,manager 校验防 CSRF。
//! * 端口由 OS 分配(绑定 `:0`),避免固定端口被预测。

use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing::{info, warn};

/// 回调服务器在 query string 中解析出的授权码 + state。
#[derive(Debug, Clone)]
pub struct CallbackResult {
    pub code: String,
    pub state: String,
}

/// 启动一次性回调服务器,返回 `(实际监听地址, 接收结果的 future)`。
///
/// 调用方应:
/// 1. 从 `addr` 提取端口,拼出 `redirect_uri` 传给 provider。
/// 2. 并行 `await` future,拿到 [`CallbackResult`]。
/// 3. drop future 以停止监听。
pub async fn start_callback_server() -> Result<(String, oneshot::Receiver<CallbackResult>)> {
    // 绑定 :0 让 OS 分配空闲端口(避免固定端口冲突 / 预测)。
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("绑定回调服务器端口失败")?;
    let port = listener.local_addr().context("获取监听端口失败")?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let (tx, rx) = oneshot::channel::<CallbackResult>();

    tokio::spawn(async move {
        if let Err(e) = serve_one(&listener, tx).await {
            warn!(target: "nebula.oauth.callback", error = %e, "回调服务器异常退出");
        }
    });

    info!(target: "nebula.oauth.callback", port, "回调服务器已启动");
    Ok((redirect_uri, rx))
}

/// 处理单个连接:解析请求行 → 提取 code/state → 回 HTML → 发 channel。
async fn serve_one(listener: &TcpListener, tx: oneshot::Sender<CallbackResult>) -> Result<()> {
    // 给等待回调一个上限(5 分钟),超时则放弃(用户可能关闭了浏览器)。
    let accept = tokio::time::timeout(Duration::from_secs(300), listener.accept());
    let (mut stream, _) = accept
        .await
        .context("等待回调连接超时(5 分钟)")?
        .context("accept 失败")?;

    // 读取 HTTP 请求(只关心 request line,读到 \r\n\r\n 即可)。
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await.context("读取请求失败")?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // 解析 request line:GET /callback?code=xxx&state=yyy HTTP/1.1
    let request_line = request.lines().next().unwrap_or("");
    let (code, state) = parse_callback_query(request_line)?;

    // 回送一个简单 HTML 让用户看到"授权成功,可关闭此页"。
    let body = if code.is_empty() {
        "<html><body><h2>授权未完成</h2><p>未收到授权码,请重试。</p></body></html>"
    } else {
        "<html><body><h2>授权成功</h2><p>已返回 Nebula,可关闭此页面。</p></body></html>"
    };
    let response = format!(
        "HTTP/1.0 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;

    info!(target: "nebula.oauth.callback", has_code = !code.is_empty(), "收到回调");

    // 把结果送回等待的 manager(channel 关闭也无妨,manager 可能已超时)。
    let _ = tx.send(CallbackResult { code, state });
    Ok(())
}

/// 从 `GET /callback?code=...&state=... HTTP/1.1` 提取 code / state。
///
/// 缺失字段返回空串(非错误,让 manager 决定如何处理)。
fn parse_callback_query(request_line: &str) -> Result<(String, String)> {
    // 形如 "GET /callback?code=abc&state=xyz HTTP/1.1"
    let path = request_line.split_whitespace().nth(1).unwrap_or("/");
    let query = path.split('?').nth(1).unwrap_or("");
    let mut code = String::new();
    let mut state = String::new();
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        let key = kv.next().unwrap_or("");
        let val = url_decode(kv.next().unwrap_or(""));
        match key {
            "code" => code = val,
            "state" => state = val,
            _ => {}
        }
    }
    Ok((code, state))
}

/// 极简 percent-decoding(仅解码 %XX 序列,足够 OAuth 回调参数)。
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(b as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_query_extracts_code_and_state() {
        let (code, state) =
            parse_callback_query("GET /callback?code=abc123&state=xyz789 HTTP/1.1").unwrap();
        assert_eq!(code, "abc123");
        assert_eq!(state, "xyz789");
    }

    #[test]
    fn parse_query_missing_state() {
        let (code, state) = parse_callback_query("GET /callback?code=abc HTTP/1.1").unwrap();
        assert_eq!(code, "abc");
        assert_eq!(state, "");
    }

    #[test]
    fn parse_query_empty() {
        let (code, state) = parse_callback_query("GET /callback HTTP/1.1").unwrap();
        assert_eq!(code, "");
        assert_eq!(state, "");
    }

    #[test]
    fn url_decode_percent_sequences() {
        assert_eq!(url_decode("hello%20world"), "hello world");
        assert_eq!(url_decode("a%2Bb"), "a+b");
        assert_eq!(url_decode("plain"), "plain");
    }

    #[tokio::test]
    async fn callback_server_starts_and_receives() {
        // 启动服务器。
        let (redirect_uri, rx) = start_callback_server().await.unwrap();
        assert!(redirect_uri.starts_with("http://127.0.0.1:"));

        // 提取端口(格式 http://127.0.0.1:PORT/callback),模拟 provider 回调。
        let port: u16 = redirect_uri
            .rsplit_once(':')
            .and_then(|(_, rest)| rest.split('/').next())
            .unwrap()
            .parse()
            .unwrap();

        // 用 TcpStream 模拟浏览器发回调请求。
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut s = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
                .await
                .unwrap();
            let req = "GET /callback?code=testcode42&state=teststate99 HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
            s.write_all(req.as_bytes()).await.unwrap();
            // 读取响应(避免连接提前关闭)。
            let mut buf = [0u8; 256];
            let _ = s.read(&mut buf).await;
        });

        let result = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .expect("回调未在 5 秒内到达")
            .expect("channel 关闭");
        assert_eq!(result.code, "testcode42");
        assert_eq!(result.state, "teststate99");
    }
}

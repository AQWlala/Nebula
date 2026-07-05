//! T-E-S-54: Webhook 触发器 — axum HTTP server,默认绑 127.0.0.1:8088。
//!
//! 设计要点:
//! * `axum::Router::route("/webhooks/:trigger_id", post(handle_webhook))`
//! * HMAC-SHA256 签名校验(可选 secret,用 `X-Signature` header 携带 hex 签名)
//! * body 1MiB 限制
//! * 启动失败 warn + 降级(不阻断 trigger engine 启动)
//!
//! ## HMAC 实现
//!
//! 由于 `hmac` crate 未在 `Cargo.toml` 中声明(只在 transitive dep 中),
//! 本模块用 `sha2` 直接实现 HMAC-SHA256(RFC 2104):
//!
//! ```text
//! HMAC(K, m) = H((K' ⊕ opad) || H((K' ⊕ ipad) || m))
//! ```
//!
//! `K'` 是密钥填充到 block size(64 字节),`ipad = 0x36`, `opad = 0x5c`。

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::{TriggerCondition, TriggerKind};

/// Webhook body 最大字节数(1 MiB,与 spec §设计约束 第 7 条一致)。
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// HMAC-SHA256 block size(字节)。
const HMAC_BLOCK_SIZE: usize = 64;

/// 启动 webhook server。
///
/// 返回 `JoinHandle<()>` 供调用方管理生命周期。绑定失败返回 Err,
/// 调用方应 `warn!` 后降级(不阻断 engine 启动)。
pub async fn start_webhook_server(
    addr: String,
    triggers: Arc<parking_lot::RwLock<HashMap<String, super::TriggerConfig>>>,
    engine: Arc<super::TriggerEngine>,
    cancel: CancellationToken,
) -> Result<JoinHandle<()>> {
    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|e| anyhow!("webhook bind {addr} failed: {e}"))?;
    info!(
        target: "nebula.triggers.webhook",
        addr = %addr,
        "webhook server listening"
    );

    let app_state = WebhookState {
        triggers,
        engine,
        cancel: cancel.clone(),
    };

    let app = Router::new()
        .route("/webhooks/:trigger_id", post(handle_webhook))
        .with_state(app_state);

    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            warn!(
                target: "nebula.triggers.webhook",
                error = %e,
                "webhook server errored"
            );
        }
    });

    Ok(handle)
}

/// axum 共享状态。
#[derive(Clone)]
struct WebhookState {
    triggers: Arc<parking_lot::RwLock<HashMap<String, super::TriggerConfig>>>,
    engine: Arc<super::TriggerEngine>,
    #[allow(dead_code)]
    cancel: CancellationToken,
}

/// POST /webhooks/:trigger_id 处理函数。
async fn handle_webhook(
    State(state): State<WebhookState>,
    Path(trigger_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // 1. body 大小校验。
    if body.len() > MAX_BODY_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            "body exceeds 1MiB limit",
        )
            .into_response();
    }

    // 2. 取 trigger 配置(必须是 webhook 类型且 enabled)。
    let cfg = {
        let map = state.triggers.read();
        map.get(&trigger_id).cloned()
    };
    let Some(cfg) = cfg else {
        return (StatusCode::NOT_FOUND, "trigger not found").into_response();
    };
    if cfg.kind != TriggerKind::Webhook {
        return (StatusCode::BAD_REQUEST, "trigger is not a webhook trigger").into_response();
    }
    if !cfg.enabled {
        return (StatusCode::FORBIDDEN, "trigger disabled").into_response();
    }

    // 3. method 校验(可选,默认 POST)。
    let TriggerCondition::Webhook { ref secret, ref method } = cfg.condition else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "invalid condition").into_response();
    };
    if let Some(ref m) = method {
        // axum routing 已限定 POST,这里仅记录日志。
        if !m.eq_ignore_ascii_case("POST") {
            warn!(
                target: "nebula.triggers.webhook",
                trigger_id = %trigger_id,
                expected = %m,
                "non-POST method configured; route still requires POST"
            );
        }
    }

    // 4. HMAC-SHA256 签名校验(若配置了 secret)。
    if let Some(ref secret) = secret {
        if let Err((status, msg)) = verify_signature(secret, &body, &headers) {
            return (status, msg).into_response();
        }
    }

    // 5. 构造 payload 并 dispatch。
    let payload = build_payload(&trigger_id, &body, &headers);
    let engine = Arc::clone(&state.engine);
    let trigger_id_clone = trigger_id.clone();
    tokio::spawn(async move {
        engine.dispatch(&trigger_id_clone, payload).await;
    });

    (StatusCode::OK, "accepted").into_response()
}

/// 构造 webhook payload。body 尝试解析为 JSON;失败则包装为字符串。
fn build_payload(
    trigger_id: &str,
    body: &[u8],
    _headers: &HeaderMap,
) -> serde_json::Value {
    let body_json: serde_json::Value = serde_json::from_slice(body).unwrap_or_else(|_| {
        serde_json::json!({
            "body": String::from_utf8_lossy(body).to_string(),
        })
    });
    serde_json::json!({
        "trigger_id": trigger_id,
        "source_trigger_id": trigger_id,
        "webhook": true,
        "data": body_json,
    })
}

/// HMAC-SHA256 签名校验。
///
/// 客户端通过 `X-Signature` header 携带 hex 编码的 HMAC-SHA256 签名。
/// 本函数用 secret 重新计算签名并常量时间比对。
fn verify_signature(secret: &str, body: &[u8], headers: &HeaderMap) -> Result<(), (StatusCode, &'static str)> {
    let signature = headers
        .get("X-Signature")
        .and_then(|v| v.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "missing X-Signature header"))?;

    let expected = hmac_sha256(secret.as_bytes(), body);
    let expected_hex = hex_encode(&expected);

    if !constant_time_eq(expected_hex.as_bytes(), signature.as_bytes()) {
        return Err((StatusCode::UNAUTHORIZED, "invalid signature"));
    }
    Ok(())
}

/// HMAC-SHA256 实现(RFC 2104)。
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    // 1. K' 处理:超过 block size 则先 hash;不足则补零。
    let mut k = if key.len() > HMAC_BLOCK_SIZE {
        let h = Sha256::digest(key);
        let mut v = vec![0u8; HMAC_BLOCK_SIZE];
        v[..h.len()].copy_from_slice(&h);
        v
    } else {
        let mut v = vec![0u8; HMAC_BLOCK_SIZE];
        v[..key.len()].copy_from_slice(key);
        v
    };

    // 2. ipad / opad。
    let mut ipad = vec![0x36u8; HMAC_BLOCK_SIZE];
    let mut opad = vec![0x5cu8; HMAC_BLOCK_SIZE];
    for i in 0..HMAC_BLOCK_SIZE {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }

    // 3. inner = H(ipad || message)
    let mut inner = Sha256::new();
    inner.update(&ipad);
    inner.update(message);
    let inner_hash = inner.finalize();

    // 4. outer = H(opad || inner)
    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(&inner_hash);
    let outer_hash = outer.finalize();

    // 5. k 清零(安全习惯)。
    for b in k.iter_mut() {
        *b = 0;
    }

    let mut out = [0u8; 32];
    out.copy_from_slice(&outer_hash);
    out
}

/// 十六进制编码(小写)。
fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// 常量时间比较(防止时序攻击)。
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hmac_sha256_known_vector() {
        // RFC 4231 Test Case 1:
        // key = 0x0b * 20, data = "Hi There"
        let key = vec![0x0bu8; 20];
        let data = b"Hi There";
        let mac = hmac_sha256(&key, data);
        let hex = hex_encode(&mac);
        // RFC 4231 expected:
        assert_eq!(
            hex,
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn test_hmac_sha256_long_key() {
        // key > 64 bytes → 先 hash。
        let key = vec![0xaau8; 131];
        let data = b"test message";
        let mac = hmac_sha256(&key, data);
        // 32 字节输出。
        assert_eq!(mac.len(), 32);
        // 相同输入应产生相同输出。
        let mac2 = hmac_sha256(&key, data);
        assert_eq!(mac, mac2);
    }

    #[test]
    fn test_hmac_sha256_empty_message() {
        let mac = hmac_sha256(b"secret", b"");
        let hex = hex_encode(&mac);
        // 用 OpenSSL 计算的预期值:
        // echo -n "" | openssl dgst -sha256 -hmac "secret"
        // = f9e66e179b6747ae54108f82f8ade8b3c25d76fd30afde6c395822c530196169
        assert_eq!(
            hex,
            "f9e66e179b6747ae54108f82f8ade8b3c25d76fd30afde6c395822c530196169"
        );
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn test_hex_encode() {
        assert_eq!(hex_encode(&[0x00, 0xff, 0xab]), "00ffab");
        assert_eq!(hex_encode(&[]), "");
    }

    #[test]
    fn test_build_payload_json_body() {
        let body = br#"{"event":"push"}"#;
        let payload = build_payload("t1", body, &HeaderMap::new());
        assert_eq!(payload["trigger_id"], "t1");
        assert_eq!(payload["source_trigger_id"], "t1");
        assert_eq!(payload["webhook"], true);
        assert_eq!(payload["data"]["event"], "push");
    }

    #[test]
    fn test_build_payload_non_json_body() {
        let body = b"plain text";
        let payload = build_payload("t1", body, &HeaderMap::new());
        assert_eq!(payload["data"]["body"], "plain text");
    }

    #[test]
    fn test_verify_signature_missing_header() {
        let headers = HeaderMap::new();
        let result = verify_signature("secret", b"body", &headers);
        assert!(result.is_err());
        let (status, msg) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(msg, "missing X-Signature header");
    }

    #[test]
    fn test_verify_signature_valid() {
        let secret = "my_secret";
        let body = b"hello world";
        let mac = hmac_sha256(secret.as_bytes(), body);
        let sig = hex_encode(&mac);

        let mut headers = HeaderMap::new();
        headers.insert("X-Signature", sig.parse().unwrap());
        let result = verify_signature(secret, body, &headers);
        assert!(result.is_ok());
    }

    #[test]
    fn test_verify_signature_wrong_signature() {
        let secret = "my_secret";
        let body = b"hello world";
        let mut headers = HeaderMap::new();
        headers.insert("X-Signature", "deadbeef".parse().unwrap());
        let result = verify_signature(secret, body, &headers);
        assert!(result.is_err());
        let (status, msg) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(msg, "invalid signature");
    }
}

//! T-E-C-18: PKCE (Proof Key for Code Exchange, RFC 7636)。
//!
//! PKCE 让公开客户端(无法保管 client_secret 的桌面 / 移动应用)在授权码
//! 流程中免受授权码截获攻击:
//!
//! 1. 客户端生成随机 `code_verifier`(43-128 字符)。
//! 2. 计算 `code_challenge = BASE64URL(SHA256(code_verifier))`,连同
//!    `code_challenge_method=S256` 一并发送到授权端点。
//! 3. 拿授权码换 token 时附带原始 `code_verifier`,服务端校验
//!    `SHA256(code_verifier) == code_challenge` 才放行。
//!
//! 截获者拿到授权码但没有 `code_verifier`,无法换取 token。

use anyhow::Result;
use base64::Engine;
use sha2::{Digest, Sha256};

/// PKCE 校验对:verifier 随请求发出,challenge 在授权 URL 里。
#[derive(Debug, Clone)]
pub struct PkcePair {
    /// 原始随机串(43-128 字符),token 交换时回传。
    pub code_verifier: String,
    /// `BASE64URL-ENCODE(SHA256(code_verifier))`,放进授权 URL。
    pub code_challenge: String,
}

/// challenge 方法(仅支持 S256,plain 已弃用)。
pub struct PkceChallenge;

impl PkcePair {
    /// 生成一对新的 PKCE verifier / challenge。
    ///
    /// verifier 为 64 字节随机数的 base64url 编码(86 字符,在 43-128 范围内)。
    pub fn generate() -> Result<Self> {
        use rand::RngCore;
        let mut bytes = [0u8; 64];
        rand::thread_rng().fill_bytes(&mut bytes);
        let code_verifier = encode_url_safe_no_pad(&bytes);
        let code_challenge = PkceChallenge::s256(&code_verifier);
        Ok(Self {
            code_verifier,
            code_challenge,
        })
    }
}

impl PkceChallenge {
    /// 计算 S256 code_challenge:`BASE64URL-ENCODE(SHA256(ASCII(verifier)))`。
    pub fn s256(verifier: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let digest = hasher.finalize();
        encode_url_safe_no_pad(&digest)
    }
}

/// base64url 无 padding 编码(RFC 7636 / RFC 4648 §5)。
fn encode_url_safe_no_pad(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_length_in_rfc_range() {
        let pair = PkcePair::generate().unwrap();
        // RFC 7636 §4.1:43 ≤ len ≤ 128。
        assert!(
            pair.code_verifier.len() >= 43 && pair.code_verifier.len() <= 128,
            "verifier 长度 {} 不在 [43,128] 范围",
            pair.code_verifier.len()
        );
    }

    #[test]
    fn challenge_is_s256_of_verifier() {
        let pair = PkcePair::generate().unwrap();
        let expected = PkceChallenge::s256(&pair.code_verifier);
        assert_eq!(pair.code_challenge, expected);
    }

    #[test]
    fn two_pairs_differ() {
        let a = PkcePair::generate().unwrap();
        let b = PkcePair::generate().unwrap();
        assert_ne!(a.code_verifier, b.code_verifier, "两次生成必须不同");
        assert_ne!(a.code_challenge, b.code_challenge);
    }

    #[test]
    fn s256_known_vector() {
        // RFC 7636 Appendix B 测试向量。
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = PkceChallenge::s256(verifier);
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn verifier_only_contains_unreserved_chars() {
        // RFC 7636 §4.1:verifier 只能含 [A-Z/a-z/0-9/-/./_/~]。
        let pair = PkcePair::generate().unwrap();
        for c in pair.code_verifier.chars() {
            assert!(
                c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_' || c == '~',
                "verifier 含非法字符: {c}"
            );
        }
    }
}

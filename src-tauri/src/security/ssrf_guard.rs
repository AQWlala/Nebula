use anyhow::{anyhow, Result};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Debug, Clone)]
pub struct SsrfGuard {
    allow_private: bool,
    /// M7b #94: 允许 loopback(127.0.0.0/8 + ::1),用于本地 LLM 端点(vLLM/LMStudio)。
    /// 与 `allow_private` 不同:loopback 是合法本地服务,私网(10.x/172.16.x/192.168.x)
    /// 可能是内网其他服务(SSRF 风险)。
    allow_loopback: bool,
}

impl SsrfGuard {
    pub fn new() -> Self {
        Self {
            allow_private: false,
            allow_loopback: false,
        }
    }

    pub fn with_allow_private(mut self, allow: bool) -> Self {
        self.allow_private = allow;
        self
    }

    /// M7b #94: 允许 loopback 地址(127.0.0.0/8 + ::1)。
    /// 用于本地 LLM 端点(vLLM/LMStudio/Ollama),仍拒绝其他私网地址。
    pub fn with_allow_loopback(mut self, allow: bool) -> Self {
        self.allow_loopback = allow;
        self
    }

    pub fn validate_url(&self, url: &str) -> Result<()> {
        let parsed = url::Url::parse(url).map_err(|e| anyhow!("invalid URL: {e}"))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow!("URL has no host: {url}"))?;

        if self.allow_private {
            return Ok(());
        }

        let ip: IpAddr = match host.parse() {
            Ok(ip) => ip,
            Err(_) => {
                use std::net::ToSocketAddrs;
                let addrs: Vec<IpAddr> = format!("{host}:0")
                    .to_socket_addrs()
                    .map(|iter| iter.map(|a| a.ip()).collect())
                    .unwrap_or_default();
                match addrs.first() {
                    Some(ip) => *ip,
                    None => return Err(anyhow!("cannot resolve host: {host}")),
                }
            }
        };

        self.validate_ip(&ip)
    }

    fn validate_ip(&self, ip: &IpAddr) -> Result<()> {
        match ip {
            IpAddr::V4(v4) => {
                if is_loopback_v4(v4) {
                    // M7b #94: allow_loopback 豁免 loopback(本地 LLM 端点)。
                    if !self.allow_loopback {
                        return Err(anyhow!(
                            "SSRF: loopback address {} is not allowed",
                            v4
                        ));
                    }
                    return Ok(());
                }
                if is_private_v4(v4) {
                    return Err(anyhow!("SSRF: private address {} is not allowed", v4));
                }
                if is_link_local_v4(v4) {
                    return Err(anyhow!(
                        "SSRF: link-local address {} is not allowed",
                        v4
                    ));
                }
                if is_cgnat(v4) {
                    return Err(anyhow!("SSRF: CGNAT address {} is not allowed", v4));
                }
                if is_broadcast(v4) {
                    return Err(anyhow!("SSRF: broadcast address {} is not allowed", v4));
                }
            }
            IpAddr::V6(v6) => {
                if v6.is_loopback() {
                    // M7b #94: allow_loopback 豁免 IPv6 loopback(::1)。
                    if !self.allow_loopback {
                        return Err(anyhow!(
                            "SSRF: loopback address {} is not allowed",
                            v6
                        ));
                    }
                    return Ok(());
                }
                // M7b #94: 增强 IPv6 检测 — ULA(fc00::/7)+ link-local(fe80::/10)
                // + IPv4-mapped(::ffff:0:0/96)。
                if is_ula_v6(v6) {
                    return Err(anyhow!(
                        "SSRF: IPv6 ULA address {} is not allowed",
                        v6
                    ));
                }
                if is_link_local_v6(v6) {
                    return Err(anyhow!(
                        "SSRF: IPv6 link-local address {} is not allowed",
                        v6
                    ));
                }
                if is_ipv4_mapped_v6(v6) {
                    // IPv4-mapped 地址可能隐藏 IPv4 私网地址,递归校验内嵌的 IPv4。
                    let v4 = extract_ipv4_mapped(v6);
                    return self.validate_ip(&IpAddr::V4(v4));
                }
            }
        }
        Ok(())
    }

    /// 验证预定义的 URL 列表（向后兼容）。
    ///
    /// 注意：此方法仅验证调用方提供的 URL 列表，**不会**执行真实 HTTP
    /// 请求来发现重定向链。如需真实重定向链每跳验证，请使用
    /// [`build_safe_client`](Self::build_safe_client)。
    pub fn validate_redirect_chain(&self, urls: &[String]) -> Result<()> {
        for url in urls {
            self.validate_url(url)?;
        }
        Ok(())
    }

    /// T-S2-A-02: 构建一个 SSRF 安全的 HTTP 客户端。
    ///
    /// 返回的 `reqwest::Client` 使用自定义重定向策略：在每次重定向
    /// 跳转前调用 `validate_url()` 验证目标 URL。如果目标 URL 指向
    /// 内网地址（loopback/private/link-local/CGNAT/broadcast），重定向
    /// 被中止并返回错误。
    ///
    /// 这取代了旧的 `validate_redirect_chain()` 伪实现——后者只能验证
    /// 调用方预先知道的 URL 列表，无法发现实际的 HTTP 重定向链。
    pub fn build_safe_client(&self) -> Result<reqwest::Client> {
        let guard = self.clone();
        let policy = reqwest::redirect::Policy::custom(move |attempt| {
            // attempt.url() 返回即将跳转的目标 URL
            if let Err(e) = guard.validate_url(&attempt.url().to_string()) {
                tracing::warn!(
                    target: "nebula.ssrf",
                    url = %attempt.url(),
                    error = %e,
                    "blocked redirect to forbidden address"
                );
                attempt.error(e.to_string())
            } else {
                attempt.follow()
            }
        });

        reqwest::Client::builder()
            .redirect(policy)
            .build()
            .map_err(|e| anyhow!("failed to build SSRF-safe HTTP client: {e}"))
    }
}

fn is_loopback_v4(ip: &Ipv4Addr) -> bool {
    ip.octets()[0] == 127
}

fn is_private_v4(ip: &Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 10
        || (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31)
        || (octets[0] == 192 && octets[1] == 168)
}

fn is_link_local_v4(ip: &Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 169 && octets[1] == 254
}

pub fn is_cgnat(ip: &Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && octets[1] >= 64 && octets[1] <= 127
}

fn is_broadcast(ip: &Ipv4Addr) -> bool {
    *ip == Ipv4Addr::BROADCAST
}

/// M7b #94: IPv6 Unique Local Address(fc00::/7,含 fd00::/8 和 fc00::/8)。
/// 等价 IPv4 私网地址(RFC 4193)。
fn is_ula_v6(ip: &Ipv6Addr) -> bool {
    // fc00::/7 意味着前 7 位匹配 1111110x
    // 即第一个字节为 0xFC 或 0xFD
    ip.octets()[0] & 0xFE == 0xFC
}

/// M7b #94: IPv6 link-local(fe80::/10,RFC 4291)。
fn is_link_local_v6(ip: &Ipv6Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 0xFE && (octets[1] & 0xC0) == 0x80
}

/// M7b #94: IPv4-mapped IPv6 地址(::ffff:a.b.c.d)。
/// 格式:前 80 位为 0,中间 16 位为 1(ffff),最后 32 位为 IPv4 地址。
fn is_ipv4_mapped_v6(ip: &Ipv6Addr) -> bool {
    let octets = ip.octets();
    // 前 10 字节为 0
    octets[0..10].iter().all(|&b| b == 0)
        // 字节 10-11 为 0xFF
        && octets[10] == 0xFF
        && octets[11] == 0xFF
}

/// M7b #94: 从 IPv4-mapped IPv6 地址提取内嵌的 IPv4 地址。
fn extract_ipv4_mapped(ip: &Ipv6Addr) -> Ipv4Addr {
    let octets = ip.octets();
    Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15])
}

impl Default for SsrfGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_private_192_168() {
        let guard = SsrfGuard::new();
        assert!(guard.validate_url("http://192.168.1.1/api").is_err());
    }

    #[test]
    fn rejects_private_10() {
        let guard = SsrfGuard::new();
        assert!(guard.validate_url("http://10.0.0.1/api").is_err());
    }

    #[test]
    fn rejects_loopback() {
        let guard = SsrfGuard::new();
        assert!(guard.validate_url("http://127.0.0.1/api").is_err());
    }

    #[test]
    fn rejects_link_local_cloud_metadata() {
        let guard = SsrfGuard::new();
        assert!(guard
            .validate_url("http://169.254.169.254/latest/meta-data/")
            .is_err());
    }

    #[test]
    fn rejects_cgnat() {
        let guard = SsrfGuard::new();
        assert!(guard.validate_url("http://100.64.0.1/api").is_err());
    }

    #[test]
    fn allows_public_ip() {
        let guard = SsrfGuard::new();
        assert!(guard.validate_url("https://api.openai.com").is_ok());
    }

    #[test]
    fn allow_private_opt_out() {
        let guard = SsrfGuard::new().with_allow_private(true);
        assert!(guard.validate_url("http://192.168.1.1/api").is_ok());
    }

    #[test]
    fn rejects_broadcast() {
        let guard = SsrfGuard::new();
        assert!(guard.validate_url("http://255.255.255.255/api").is_err());
    }

    #[test]
    fn build_safe_client_succeeds() {
        let guard = SsrfGuard::new();
        let client = guard.build_safe_client();
        assert!(client.is_ok(), "should build a valid reqwest client");
    }

    #[test]
    fn validate_redirect_chain_still_works() {
        let guard = SsrfGuard::new();
        let urls = vec![
            "https://api.openai.com".to_string(),
            "https://api.anthropic.com".to_string(),
        ];
        assert!(guard.validate_redirect_chain(&urls).is_ok());

        let bad_urls = vec!["http://127.0.0.1/api".to_string()];
        assert!(guard.validate_redirect_chain(&bad_urls).is_err());
    }
}

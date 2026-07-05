//! v1.0.1 P0#12: security helpers (OS keychain, sensitive-data
//! redaction).  See `keychain` and the `is_sensitive` predicate
//! on `crate::memory::types::Memory` for the actual logic.
//!
//! v1.1 P1-4: `detectors` 模块提供基于正则的敏感数据自动检测。
//! v1.3 P1-4: `injection_guard` 模块提供 Prompt 注入 + 危险命令 + 不可见 Unicode 检测。

pub mod detectors;
pub mod injection_guard;
pub mod keychain;
pub mod ssrf_guard;

// re-export detectors functions for convenience
pub use detectors::{contains_sensitive, scan_content, SensitiveScanner};
// re-export injection guard functions for convenience
pub use injection_guard::{
    full_injection_scan, has_dangerous_command, has_injection, scan_dangerous_commands,
    scan_prompt_injection, strip_invisible_unicode, InjectionScanResult, InjectionSeverity,
};
pub use ssrf_guard::SsrfGuard;

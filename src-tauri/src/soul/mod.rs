//! `nebula::soul` — Soul 系统（M1 里程碑）。
//!
//! 实现 ADR-003 §6.3 的 SoulCompiler 编译管线：
//!   SOUL.md → injection_scan → strip_unicode → L2/L3/L5 提取
//!          → LLM 编译（dispatch(SoulCompile)） → CompiledSoul
//!
//! ## 设计要点
//!
//! - **双分区 SOUL.md**：`immutable_from_ai`（AI 不可改）+ `evolution-append`（进化追加）
//!   两区，由 Section 标签配对校验保证结构完整性。
//! - **CompiledSoul 输出**：`{ system_prompt, warnings }`，不直接覆盖 PersonaConfig
//!   （P0-7 修复）。共存逻辑：有 Soul 用 Soul.system_prompt；无 Soul 回退 PersonaConfig。
//! - **强制本地路由**：`WorkType::SoulCompile` 经 Dispatcher 走本地 Ollama
//!   （qwen2.5:3b），不计费、不外发。
//! - **注入防护全路径覆盖**：Step 1（输入扫描）+ Step 6（拼接后 full_injection_scan），
//!   Critical/High 丢弃并记录 warnings（P1-13）。
//! - **降级策略**：5s 超时 → 文本拼接（无 LLM 调用）；LLM 失败 → warnings 字段记录。
//! - **原子写入**：write-temp-then-rename + 备份 + 文件锁（P1-14）。
//!
//! ## Feature Gate（参考 evolution/ 模块的双层 gate 模式）
//!
//! - 编译期：`#![cfg(feature = "soul-system")]`（Cargo.toml 已声明，默认 off）
//! - 运行时：`SOUL_SYSTEM_ENABLED: AtomicBool`（默认 false，需 Settings UI 或
//!   环境变量 `SOUL_SYSTEM_ENABLED=1` 显式开启）
//!
//! 参见 ADR-004 Feature Flag 策略。

#![cfg(feature = "soul-system")]

pub mod atomic_write;
pub mod compiler;
pub mod structure;

use serde::{Deserialize, Serialize};

/// Soul 系统运行时开关。
///
/// 与 `evolution::EVOLUTION_ENABLED` 同样的双层 gate 模式：
/// 即使编译期 feature 开启，运行时仍需显式 flip 才会真正启用。
///
/// 读取方式：
/// - Settings UI 切换
/// - 环境变量 `SOUL_SYSTEM_ENABLED=1`（启动时读取一次）
pub static SOUL_SYSTEM_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// 查询 Soul 系统是否启用。
pub fn soul_system_enabled() -> bool {
    SOUL_SYSTEM_ENABLED.load(std::sync::atomic::Ordering::SeqCst)
}

/// 设置 Soul 系统启用状态（Settings UI 调用）。
pub fn set_soul_system_enabled(on: bool) {
    SOUL_SYSTEM_ENABLED.store(on, std::sync::atomic::Ordering::SeqCst);
}

/// 启动时从环境变量 `SOUL_SYSTEM_ENABLED` 读取初始状态。
///
/// 应在 `lib.rs` 的 setup 阶段调用一次。值为 `1` / `true` / `on` 时启用。
pub fn init_from_env() {
    let enabled = match std::env::var("SOUL_SYSTEM_ENABLED") {
        Ok(v) => {
            let lower = v.to_lowercase();
            lower == "1" || lower == "true" || lower == "on"
        }
        Err(_) => false,
    };
    set_soul_system_enabled(enabled);
}

/// Soul 系统配置 DTO（通过 Tauri 命令边界交换）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoulConfig {
    pub enabled: bool,
    /// SOUL.md 文件路径（通常为 workspace_root / "SOUL.md"）。
    pub soul_md_path: String,
    /// LLM 编译超时（秒）。默认 5。
    pub compile_timeout_secs: u64,
    /// 编译失败时是否降级为文本拼接（无 LLM 调用）。默认 true。
    pub fallback_to_text: bool,
}

impl Default for SoulConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            soul_md_path: "SOUL.md".to_string(),
            compile_timeout_secs: 5,
            fallback_to_text: true,
        }
    }
}

/// 重导出核心类型，便于外部使用。
pub use compiler::{CompiledSoul, SoulCompiler, SoulCompilerError};
pub use structure::{SoulSection, SoulStructure, SoulStructureError};

#[cfg(test)]
mod tests;

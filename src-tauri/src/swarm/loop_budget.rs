#![cfg(feature = "master-orchestrator")]

//! T-E-L-06: Loop 预算配置(loop-budget.md)解析层。
//!
//! 将 loop-budget.md(YAML frontmatter + Markdown 表格)解析为
//! [`LoopBudgetConfig`] 结构,供 MasterOrchestrator::execute_loop
//! 在启动 Loop 前做预算门禁检查。
//!
//! ## loop-budget.md 格式
//!
//! ```text
//! ---
//! monthly_tokens: 5000000
//! monthly_usd: 50.0
//! default_per_run_tokens: 50000
//! default_per_run_minutes: 10
//! cloud_ratio_threshold: 0.7
//! ---
//!
//! ## 各 Loop 预算
//! | Loop | Cadence | Token/次 | 月度估算 Token | 月度估算 USD | 本地 |
//! |------|---------|----------|---------------|-------------|------|
//! | daily-triage | 0 9 * * 1-5 | 50000 | 1100000 | 0.0 | true |
//! ```
//!
//! ## 降级策略
//!
//! - 文件缺失 → [`LoopBudgetConfig::default_config`](安全保守值)
//! - frontmatter 字段缺失 → 各字段用默认值
//! - 表格行解析失败 → 跳过该行(不中断整体解析)
//!
//! ## Feature Gate
//!
//! 与 `loop_def.rs` 一致,由 `master-orchestrator` feature 门控。

use std::path::Path;

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// 默认值函数(供 serde #[serde(default = "...")] 引用)
// ---------------------------------------------------------------------------

fn default_monthly_tokens() -> u64 {
    5_000_000
}
fn default_monthly_usd() -> f64 {
    50.0
}
fn default_per_run_tokens() -> u64 {
    50_000
}
fn default_per_run_minutes() -> u32 {
    10
}
fn default_cloud_ratio() -> f64 {
    0.7
}

// ---------------------------------------------------------------------------
// LoopBudgetConfig — 解析后的完整预算配置
// ---------------------------------------------------------------------------

/// Loop 预算配置(从 loop-budget.md 解析)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopBudgetConfig {
    /// 月度 Token 上限(0 = 不限制)。
    pub monthly_tokens: u64,
    /// 月度美元上限(0.0 = 不限制)。
    pub monthly_usd: f64,
    /// 单次执行默认 Token 预算。
    pub default_per_run_tokens: u64,
    /// 单次执行默认时间预算(分钟)。
    pub default_per_run_minutes: u32,
    /// 云端消耗占比阈值(0.0-1.0,超过则优先暂停云端 Loop)。
    /// 默认 0.7(70%)。
    pub cloud_ratio_threshold: f64,
    /// 各 Loop 的预算条目。
    pub loops: Vec<LoopBudgetEntry>,
}

/// 单个 Loop 的预算条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopBudgetEntry {
    /// Loop 名称(与 LoopDef.name 对应)。
    pub name: String,
    /// cron 表达式(展示用,不解析)。
    pub cadence: String,
    /// 单次 Token 预算。
    pub tokens_per_run: u64,
    /// 月度估算 Token。
    pub monthly_est_tokens: u64,
    /// 月度估算 USD。
    pub monthly_est_usd: f64,
    /// 是否本地执行(true=本地 Ollama,false=云端)。
    pub is_local: bool,
}

/// frontmatter 反序列化用(带 serde 默认值,字段缺失时各自降级)。
#[derive(Debug, Clone, Deserialize)]
struct BudgetFrontmatter {
    #[serde(default = "default_monthly_tokens")]
    monthly_tokens: u64,
    #[serde(default = "default_monthly_usd")]
    monthly_usd: f64,
    #[serde(default = "default_per_run_tokens")]
    default_per_run_tokens: u64,
    #[serde(default = "default_per_run_minutes")]
    default_per_run_minutes: u32,
    #[serde(default = "default_cloud_ratio")]
    cloud_ratio_threshold: f64,
}

impl Default for BudgetFrontmatter {
    fn default() -> Self {
        Self {
            monthly_tokens: default_monthly_tokens(),
            monthly_usd: default_monthly_usd(),
            default_per_run_tokens: default_per_run_tokens(),
            default_per_run_minutes: default_per_run_minutes(),
            cloud_ratio_threshold: default_cloud_ratio(),
        }
    }
}

impl LoopBudgetConfig {
    /// 从 loop-budget.md markdown 字符串解析。
    ///
    /// 步骤:
    /// 1. 分离 YAML frontmatter(`---` 包裹)和 Markdown body
    /// 2. 用 `serde_yaml` 反序列化 frontmatter(字段缺失用默认值)
    /// 3. 扫描 body 的 `## 各 Loop 预算` 章节,解析 Markdown 表格行
    /// 4. 组装 [`LoopBudgetConfig`]
    pub fn from_markdown(md: &str) -> Result<Self> {
        let (frontmatter_str, body) = split_frontmatter(md)?;

        let fm: BudgetFrontmatter = if frontmatter_str.trim().is_empty() {
            BudgetFrontmatter::default()
        } else {
            serde_yaml::from_str(frontmatter_str)
                .map_err(|e| anyhow!("failed to parse loop-budget.md frontmatter: {e}"))?
        };

        let loops = parse_budget_table(body);

        Ok(LoopBudgetConfig {
            monthly_tokens: fm.monthly_tokens,
            monthly_usd: fm.monthly_usd,
            default_per_run_tokens: fm.default_per_run_tokens,
            default_per_run_minutes: fm.default_per_run_minutes,
            cloud_ratio_threshold: fm.cloud_ratio_threshold,
            loops,
        })
    }

    /// 从文件读取并解析。文件缺失时降级为 [`default_config`](Self::default_config)。
    pub fn from_file(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(md) => Self::from_markdown(&md),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default_config()),
            Err(e) => Err(anyhow!("failed to read {}: {}", path.display(), e)),
        }
    }

    /// 默认配置(loop-budget.md 缺失时降级使用)。
    ///
    /// 返回安全保守值:月度 5M tokens / $50,单次 50K tokens / 10 min,
    /// 云端阈值 70%,无 Loop 条目。
    pub fn default_config() -> Self {
        LoopBudgetConfig {
            monthly_tokens: default_monthly_tokens(),
            monthly_usd: default_monthly_usd(),
            default_per_run_tokens: default_per_run_tokens(),
            default_per_run_minutes: default_per_run_minutes(),
            cloud_ratio_threshold: default_cloud_ratio(),
            loops: Vec::new(),
        }
    }

    /// 检查月度预算是否超限。
    ///
    /// - `monthly_tokens == 0` 或 `monthly_usd == 0.0` 表示该维度不限制
    /// - 任一受限维度超限即返回 `true`(OR 语义)
    pub fn is_monthly_budget_exceeded(&self, used_tokens: u64, used_usd: f64) -> bool {
        let token_exceeded = self.monthly_tokens > 0 && used_tokens >= self.monthly_tokens;
        let usd_exceeded = self.monthly_usd > 0.0 && used_usd >= self.monthly_usd;
        token_exceeded || usd_exceeded
    }

    /// 检查云端占比是否超阈值。
    ///
    /// - `total_tokens == 0` 时返回 `false`(无消耗不触发)
    /// - 云端占比 **严格大于** `cloud_ratio_threshold` 时返回 `true`
    ///   (与 budget-guardian.md "云端消耗占比 > 70%" 语义一致)
    pub fn is_cloud_ratio_exceeded(&self, cloud_tokens: u64, total_tokens: u64) -> bool {
        if total_tokens == 0 {
            return false;
        }
        let ratio = cloud_tokens as f64 / total_tokens as f64;
        ratio > self.cloud_ratio_threshold
    }
}

// ---------------------------------------------------------------------------
// 内部解析辅助
// ---------------------------------------------------------------------------

/// 分离 YAML frontmatter 和 Markdown body(参考 loop_def.rs 的同名函数)。
///
/// 输入格式:
/// ```text
/// ---
/// <yaml content>
/// ---
/// <markdown body>
/// ```
///
/// 返回 `(yaml_str, body_str)`,失败情况:
/// - 不以 `---` 开头
/// - 缺少第二个 `---`(frontmatter 未闭合)
fn split_frontmatter(md: &str) -> Result<(&str, &str)> {
    let trimmed = md.trim_start();
    if !trimmed.starts_with("---") {
        bail!("loop-budget.md must start with YAML frontmatter (---)");
    }
    // 跳过第一个 "---" + 换行
    let after_first = trimmed[3..].trim_start_matches(['\r', '\n']);

    // 找第二个 "---"(行首)
    let end_pos = after_first
        .find("\n---")
        .ok_or_else(|| anyhow!("loop-budget.md frontmatter not closed (missing second ---)"))?;
    let frontmatter = &after_first[..end_pos];
    // 跳过 "\n---" + 换行
    let body = after_first[end_pos + 4..].trim_start_matches(['\r', '\n']);
    Ok((frontmatter, body))
}

/// 解析 body 中的 `## 各 Loop 预算` 章节 Markdown 表格。
///
/// 用状态机扫描 `## ` 标题,在包含 "loop" 和 "预算" 的章节内收集表格行。
/// 跳过表头行和分隔行(解析失败的行自动跳过,不中断整体解析)。
fn parse_budget_table(body: &str) -> Vec<LoopBudgetEntry> {
    let mut entries = Vec::new();
    let mut in_budget_section = false;

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") {
            let section = trimmed[3..].trim().to_lowercase();
            in_budget_section = section.contains("loop") && section.contains("预算");
            continue;
        }
        if !in_budget_section {
            continue;
        }
        // 尝试解析表格行(表头/分隔行解析失败会自动跳过)
        if let Some(entry) = parse_table_row(trimmed) {
            entries.push(entry);
        }
    }
    entries
}

/// 解析单行 Markdown 表格为 [`LoopBudgetEntry`]。
///
/// 行格式:`| name | cadence | tokens_per_run | monthly_est_tokens | monthly_est_usd | is_local |`
///
/// 解析失败(非数据行/格式不符)返回 `None`。
fn parse_table_row(line: &str) -> Option<LoopBudgetEntry> {
    let line = line.trim();
    if !line.starts_with('|') {
        return None;
    }
    // 去掉首尾的 |,按 | 分割单元格
    let inner = line.trim_start_matches('|').trim_end_matches('|');
    let cells: Vec<&str> = inner.split('|').map(|c| c.trim()).collect();
    if cells.len() < 6 {
        return None;
    }
    let name = cells[0].to_string();
    if name.is_empty() {
        return None;
    }
    let tokens_per_run = parse_u64(cells[2])?;
    let monthly_est_tokens = parse_u64(cells[3])?;
    let monthly_est_usd = parse_f64(cells[4])?;
    let is_local = parse_bool(cells[5])?;
    Some(LoopBudgetEntry {
        name,
        cadence: cells[1].to_string(),
        tokens_per_run,
        monthly_est_tokens,
        monthly_est_usd,
        is_local,
    })
}

/// 解析 u64(去除逗号和 $ 前缀,支持 "50,000" / "$50" 等)。
fn parse_u64(s: &str) -> Option<u64> {
    // 先 trim/strip(借用 s),再 replace(产生 owned String),避免临时值提前释放。
    let cleaned = s.trim().trim_start_matches('$').replace(',', "");
    cleaned.parse::<u64>().ok()
}

/// 解析 f64(去除 $ 前缀和逗号,支持 "$8.0" / "6.4" 等)。
fn parse_f64(s: &str) -> Option<f64> {
    let cleaned = s.trim().trim_start_matches('$').replace(',', "");
    cleaned.parse::<f64>().ok()
}

/// 解析 bool(支持 true/false/yes/no/1/0)。
fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_lowercase().as_str() {
        "true" | "yes" | "1" => Some(true),
        "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_LOOP_BUDGET_MD: &str = r#"---
monthly_tokens: 5000000
monthly_usd: 50.0
default_per_run_tokens: 50000
default_per_run_minutes: 10
cloud_ratio_threshold: 0.7
---

# Loop 预算

## 全局预算

- 月度 Token 上限: 5,000,000
- 月度美元上限: $50.0
- 单次执行默认: 50,000 tokens / 10 min
- 云端占比阈值: 70%

## 各 Loop 预算

| Loop | Cadence | Token/次 | 月度估算 Token | 月度估算 USD | 本地 |
|------|---------|----------|---------------|-------------|------|
| daily-triage | 0 9 * * 1-5 | 50,000 | 1,100,000 | $0.0 | true |
| ci-sweeper | 0 * * * * | 20,000 | 800,000 | $8.0 | false |
| code-review-loop | on-webhook | 80,000 | 640,000 | $6.4 | false |
| pr-babysitter | */10 * * * * | 10,000 | 400,000 | $0.0 | true |
| memory-consolidation | 0 3 * * * | 120,000 | 960,000 | $0.0 | true |
| skill-evolution | 0 10 * * 1 | 60,000 | 240,000 | $0.0 | true |
| budget-guardian | 0 * * * * | 1,000 | 720,000 | $0.0 | true |

## 超预算行为

### 单次超预算
- 暂停该 Loop + 写入 STATE.md + IM 通知

### 月度超预算
- 停止所有 Loop + 需人工恢复
"#;

    #[test]
    fn parse_full_config() {
        let cfg = LoopBudgetConfig::from_markdown(SAMPLE_LOOP_BUDGET_MD).expect("parse");

        // frontmatter 字段
        assert_eq!(cfg.monthly_tokens, 5_000_000);
        assert!((cfg.monthly_usd - 50.0).abs() < 1e-9);
        assert_eq!(cfg.default_per_run_tokens, 50_000);
        assert_eq!(cfg.default_per_run_minutes, 10);
        assert!((cfg.cloud_ratio_threshold - 0.7).abs() < 1e-9);

        // 7 个 Loop 条目
        assert_eq!(cfg.loops.len(), 7);

        // 第一行(含逗号解析)— daily-triage(本地)
        let first = &cfg.loops[0];
        assert_eq!(first.name, "daily-triage");
        assert_eq!(first.cadence, "0 9 * * 1-5");
        assert_eq!(first.tokens_per_run, 50_000);
        assert_eq!(first.monthly_est_tokens, 1_100_000);
        assert!((first.monthly_est_usd - 0.0).abs() < 1e-9);
        assert!(first.is_local);

        // 云端 Loop — ci-sweeper($8.0 解析)
        let ci = &cfg.loops[1];
        assert_eq!(ci.name, "ci-sweeper");
        assert_eq!(ci.tokens_per_run, 20_000);
        assert_eq!(ci.monthly_est_tokens, 800_000);
        assert!((ci.monthly_est_usd - 8.0).abs() < 1e-9);
        assert!(!ci.is_local);

        // code-review-loop($6.4 解析)
        let cr = &cfg.loops[2];
        assert_eq!(cr.name, "code-review-loop");
        assert!((cr.monthly_est_usd - 6.4).abs() < 1e-9);
        assert!(!cr.is_local);

        // 最后一行 — budget-guardian
        let last = cfg.loops.last().unwrap();
        assert_eq!(last.name, "budget-guardian");
        assert_eq!(last.tokens_per_run, 1_000);
        assert!(last.is_local);
    }

    #[test]
    fn parse_missing_file_returns_default() {
        // 不存在的文件 → 降级为 default_config(不报错)
        let path = std::path::Path::new("nonexistent_loop_budget_test_12345678.md");
        let cfg = LoopBudgetConfig::from_file(path).expect("should degrade to default");

        assert_eq!(cfg.monthly_tokens, 5_000_000);
        assert!((cfg.monthly_usd - 50.0).abs() < 1e-9);
        assert!(cfg.loops.is_empty());
    }

    #[test]
    fn parse_empty_frontmatter_uses_defaults() {
        // frontmatter 只有 monthly_tokens,其余字段缺失 → 用默认值
        let md = "---\nmonthly_tokens: 1000000\n---\n\
            ## 各 Loop 预算\n\
            | Loop | Cadence | Token/次 | 月度估算 Token | 月度估算 USD | 本地 |\n\
            |------|---------|----------|---------------|-------------|------|\n\
            | test-loop | 0 0 * * * | 1000 | 30000 | 0.3 | true |\n";
        let cfg = LoopBudgetConfig::from_markdown(md).expect("parse");

        // 提供的字段
        assert_eq!(cfg.monthly_tokens, 1_000_000);
        // 缺失字段用默认值
        assert!((cfg.monthly_usd - 50.0).abs() < 1e-9);
        assert_eq!(cfg.default_per_run_tokens, 50_000);
        assert_eq!(cfg.default_per_run_minutes, 10);
        assert!((cfg.cloud_ratio_threshold - 0.7).abs() < 1e-9);
        // 表格仍能正常解析
        assert_eq!(cfg.loops.len(), 1);
        assert_eq!(cfg.loops[0].name, "test-loop");
        assert_eq!(cfg.loops[0].tokens_per_run, 1000);
    }

    #[test]
    fn monthly_budget_exceeded_detection() {
        let cfg = LoopBudgetConfig::default_config(); // 5M tokens / $50

        // 未超限
        assert!(!cfg.is_monthly_budget_exceeded(4_000_000, 40.0));
        assert!(!cfg.is_monthly_budget_exceeded(4_999_999, 49.99));

        // Token 超限(USD 未超)— OR 语义
        assert!(cfg.is_monthly_budget_exceeded(5_000_000, 10.0));
        assert!(cfg.is_monthly_budget_exceeded(6_000_000, 0.0));

        // USD 超限(Token 未超)
        assert!(cfg.is_monthly_budget_exceeded(1_000_000, 50.0));
        assert!(cfg.is_monthly_budget_exceeded(0, 60.0));

        // 双超限
        assert!(cfg.is_monthly_budget_exceeded(5_000_000, 50.0));

        // monthly_tokens = 0 → Token 维度不限制,只看 USD
        let mut cfg_no_token_limit = cfg.clone();
        cfg_no_token_limit.monthly_tokens = 0;
        assert!(!cfg_no_token_limit.is_monthly_budget_exceeded(999_999_999, 40.0));
        assert!(cfg_no_token_limit.is_monthly_budget_exceeded(999_999_999, 50.0));

        // monthly_usd = 0.0 → USD 维度不限制,只看 Token
        let mut cfg_no_usd_limit = cfg.clone();
        cfg_no_usd_limit.monthly_usd = 0.0;
        assert!(!cfg_no_usd_limit.is_monthly_budget_exceeded(1_000_000, 999.0));
        assert!(cfg_no_usd_limit.is_monthly_budget_exceeded(5_000_000, 999.0));
    }

    #[test]
    fn cloud_ratio_exceeded_detection() {
        let cfg = LoopBudgetConfig::default_config(); // threshold = 0.7

        // 无消耗 — 不触发
        assert!(!cfg.is_cloud_ratio_exceeded(0, 0));

        // 50% 云端 — 未超
        assert!(!cfg.is_cloud_ratio_exceeded(500, 1000));

        // 恰好 70% — 未超(严格大于,与 budget-guardian "> 70%" 一致)
        assert!(!cfg.is_cloud_ratio_exceeded(700, 1000));

        // 71% — 超限
        assert!(cfg.is_cloud_ratio_exceeded(710, 1000));

        // 100% 云端 — 超限
        assert!(cfg.is_cloud_ratio_exceeded(1000, 1000));

        // 0% 云端(total > 0)— 未超
        assert!(!cfg.is_cloud_ratio_exceeded(0, 1000));
    }

    #[test]
    fn default_config_has_safe_values() {
        let cfg = LoopBudgetConfig::default_config();

        // 安全保守值(与 loop-budget.md frontmatter 一致)
        assert_eq!(cfg.monthly_tokens, 5_000_000);
        assert!((cfg.monthly_usd - 50.0).abs() < 1e-9);
        assert_eq!(cfg.default_per_run_tokens, 50_000);
        assert_eq!(cfg.default_per_run_minutes, 10);
        assert!((cfg.cloud_ratio_threshold - 0.7).abs() < 1e-9);
        // 无 Loop 条目(降级时不假设任何 Loop 存在)
        assert!(cfg.loops.is_empty());

        // 默认配置应能正确检测超限
        assert!(cfg.is_monthly_budget_exceeded(5_000_000, 0.0));
        assert!(cfg.is_monthly_budget_exceeded(0, 50.0));
        assert!(!cfg.is_monthly_budget_exceeded(4_999_999, 49.99));
        assert!(cfg.is_cloud_ratio_exceeded(800, 1000));
        assert!(!cfg.is_cloud_ratio_exceeded(700, 1000));
    }
}

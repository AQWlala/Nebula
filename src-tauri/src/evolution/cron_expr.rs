//! T-E-L-02: 5 字段 cron 表达式解析器。
//!
//! 支持 standard cron 语法（不引入外部 crate）：
//! - `*` — 所有值
//! - `N` — 单个数字（如 `5`）
//! - `N,M` — 列表（如 `0,15,30,45`）
//! - `N-M` — 范围（如 `1-5`）
//! - `*/N` — 步长（如 `*/15` = 每 15 分钟）
//! - `N-M/S` — 范围步长（如 `1-10/2` = 1,3,5,7,9）
//!
//! ## 5 字段格式
//!
//! ```text
//! ┌───────────── minute (0-59)
//! │ ┌───────────── hour (0-23)
//! │ │ ┌───────────── day of month (1-31)
//! │ │ │ ┌───────────── month (1-12)
//! │ │ │ │ ┌───────────── day of week (0-6, 0=Sunday)
//! │ │ │ │ │
//! * * * * *
//! ```
//!
//! ## Feature Gate
//!
//! 无 feature gate — cron 表达式解析器是通用工具，CI 默认编译测试。
//! （`cron_scheduler.rs` 仍由 `self-evolution` 门控，但 `cron_expr.rs` 独立于调度器。）

use std::collections::HashSet;

use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};

/// 解析后的 cron 表达式。
///
/// 每个字段用 `HashSet<u32>` 存储所有匹配的值，
/// `matches()` 只需 O(1) 查找。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronExpr {
    pub minutes: HashSet<u32>,
    pub hours: HashSet<u32>,
    pub days_of_month: HashSet<u32>,
    pub months: HashSet<u32>,
    pub days_of_week: HashSet<u32>,
    /// 原始表达式字符串（用于 Display 和调试）。
    pub raw: String,
}

impl CronExpr {
    /// 解析 5 字段 cron 表达式。
    ///
    /// 示例：
    /// - `"0 9 * * 1-5"` — 工作日每天 09:00
    /// - `"*/15 * * * *"` — 每 15 分钟
    /// - `"0 0,12 * * *"` — 每天 00:00 和 12:00
    /// - `"0 0 1 * *"` — 每月 1 号 00:00
    pub fn parse(expr: &str) -> Result<Self> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() != 5 {
            bail!(
                "cron expression must have 5 fields, got {}: '{expr}'",
                parts.len()
            );
        }

        let minutes = parse_field(parts[0], 0, 59, "minute")?;
        let hours = parse_field(parts[1], 0, 23, "hour")?;
        let days_of_month = parse_field(parts[2], 1, 31, "day-of-month")?;
        let months = parse_field(parts[3], 1, 12, "month")?;
        let days_of_week = parse_field(parts[4], 0, 6, "day-of-week")?;

        Ok(CronExpr {
            minutes,
            hours,
            days_of_month,
            months,
            days_of_week,
            raw: expr.to_string(),
        })
    }

    /// 检查给定时间是否匹配 cron 表达式。
    ///
    /// **day-of-month 和 day-of-week 的关系**（标准 cron 语义）：
    /// - 如果两者都设置了具体值（非 `*`），则**任一**匹配即可（OR 逻辑）
    /// - 如果其中一个是 `*`，则用另一个（AND 逻辑）
    ///
    /// 这是 cron 的标准行为，确保 `"0 0 1 * 0"` 在每月 1 号**和**每周日都触发。
    pub fn matches(&self, dt: DateTime<Utc>) -> bool {
        // chrono 的 Timelike/Datelike minute()/hour()/day()/month() 已返回 u32,
        // 无需 as u32 (clippy::unnecessary_cast)。
        let min_match = self.minutes.contains(&dt.minute());
        let hour_match = self.hours.contains(&dt.hour());
        let dom_match = self.days_of_month.contains(&dt.day());
        let month_match = self.months.contains(&dt.month());
        // chrono: Weekday::num_days_from_sunday() 返回 0=Sunday..=6=Saturday，与 cron 一致。
        let dow_match = self
            .days_of_week
            .contains(&dt.weekday().num_days_from_sunday());

        if !min_match || !hour_match || !month_match {
            return false;
        }

        // day-of-month 和 day-of-week 的 OR/AND 逻辑。
        let dom_is_star = self.days_of_month.len() == 31; // 1-31 全覆盖
        let dow_is_star = self.days_of_week.len() == 7; // 0-6 全覆盖

        if dom_is_star && dow_is_star {
            true
        } else if dom_is_star {
            dow_match
        } else if dow_is_star {
            dom_match
        } else {
            dom_match || dow_match
        }
    }
}

impl std::fmt::Display for CronExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.raw)
    }
}

/// 解析单个 cron 字段为 HashSet。
///
/// 支持语法：`*` | `N` | `N,M` | `N-M` | `*/S` | `N-M/S` | `N/S`
fn parse_field(field: &str, min: u32, max: u32, name: &str) -> Result<HashSet<u32>> {
    let mut result = HashSet::new();

    // 逗号分隔的子表达式列表。
    for part in field.split(',') {
        let values = parse_part(part, min, max, name)?;
        result.extend(values);
    }

    if result.is_empty() {
        bail!("cron field '{name}' produced empty set: '{field}'");
    }

    Ok(result)
}

/// 解析单个子表达式（不含逗号）。
///
/// 格式：`RANGE[/STEP]`
/// - RANGE: `*` | `N` | `N-M`
/// - STEP: 可选步长
fn parse_part(part: &str, min: u32, max: u32, name: &str) -> Result<Vec<u32>> {
    // 分离范围和步长。
    let (range_part, step) = match part.find('/') {
        Some(pos) => {
            let step_str = &part[pos + 1..];
            let step: u32 = step_str
                .parse()
                .map_err(|_| anyhow!("invalid step '{step_str}' in {name} field: '{part}'"))?;
            if step == 0 {
                bail!("step must be > 0 in {name} field: '{part}'");
            }
            (&part[..pos], Some(step))
        }
        None => (part, None),
    };

    // 解析范围。
    let (start, end) = if range_part == "*" {
        (min, max)
    } else if let Some(pos) = range_part.find('-') {
        let s: u32 = range_part[..pos]
            .parse()
            .map_err(|_| anyhow!("invalid range start in {name} field: '{range_part}'"))?;
        let e: u32 = range_part[pos + 1..]
            .parse()
            .map_err(|_| anyhow!("invalid range end in {name} field: '{range_part}'"))?;
        (s, e)
    } else {
        let n: u32 = range_part
            .parse()
            .map_err(|_| anyhow!("invalid value '{range_part}' in {name} field"))?;
        if step.is_some() {
            // `N/S` 格式：从 N 开始，步长 S，到 max。
            (n, max)
        } else {
            // 单个值。
            validate_range(n, min, max, name)?;
            return Ok(vec![n]);
        }
    };

    validate_range(start, min, max, name)?;
    validate_range(end, min, max, name)?;
    if start > end {
        bail!("range start {start} > end {end} in {name} field: '{part}'");
    }

    // 生成值序列。
    let step = step.unwrap_or(1);
    let mut values = Vec::new();
    let mut current = start;
    while current <= end {
        values.push(current);
        current = match current.checked_add(step) {
            Some(v) => v,
            None => break,
        };
    }

    Ok(values)
}

/// 验证值是否在合法范围内。
fn validate_range(val: u32, min: u32, max: u32, name: &str) -> Result<()> {
    if val < min || val > max {
        bail!("value {val} out of range [{min}, {max}] for {name} field");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// 构造一个 UTC DateTime（方便测试）。
    fn dt(year: i32, month: u32, day: u32, hour: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, min, 0)
            .unwrap()
    }

    // ---- parse() 基础测试 ----

    #[test]
    fn parse_every_minute() {
        let expr = CronExpr::parse("* * * * *").unwrap();
        assert_eq!(expr.minutes.len(), 60);
        assert_eq!(expr.hours.len(), 24);
        assert_eq!(expr.days_of_month.len(), 31);
        assert_eq!(expr.months.len(), 12);
        assert_eq!(expr.days_of_week.len(), 7);
    }

    #[test]
    fn parse_weekday_morning() {
        let expr = CronExpr::parse("0 9 * * 1-5").unwrap();
        assert_eq!(expr.minutes, HashSet::from([0u32]));
        assert_eq!(expr.hours, HashSet::from([9u32]));
        assert_eq!(expr.days_of_week, HashSet::from([1u32, 2, 3, 4, 5]));
    }

    #[test]
    fn parse_every_15_minutes() {
        let expr = CronExpr::parse("*/15 * * * *").unwrap();
        assert_eq!(expr.minutes, HashSet::from([0u32, 15, 30, 45]));
    }

    #[test]
    fn parse_range_with_step() {
        let expr = CronExpr::parse("1-10/2 * * * *").unwrap();
        assert_eq!(expr.minutes, HashSet::from([1u32, 3, 5, 7, 9]));
    }

    #[test]
    fn parse_list() {
        let expr = CronExpr::parse("0,15,30,45 * * * *").unwrap();
        assert_eq!(expr.minutes.len(), 4);
        assert!(expr.minutes.contains(&0));
        assert!(expr.minutes.contains(&45));
    }

    #[test]
    fn parse_monthly_first() {
        let expr = CronExpr::parse("0 0 1 * *").unwrap();
        assert_eq!(expr.days_of_month, HashSet::from([1u32]));
    }

    #[test]
    fn parse_too_few_fields_errors() {
        assert!(CronExpr::parse("* * * *").is_err());
    }

    #[test]
    fn parse_too_many_fields_errors() {
        assert!(CronExpr::parse("* * * * * *").is_err());
    }

    #[test]
    fn parse_out_of_range_errors() {
        assert!(CronExpr::parse("60 * * * *").is_err()); // minute > 59
        assert!(CronExpr::parse("* 24 * * *").is_err()); // hour > 23
        assert!(CronExpr::parse("* * 0 * *").is_err()); // day < 1
        assert!(CronExpr::parse("* * * 13 *").is_err()); // month > 12
        assert!(CronExpr::parse("* * * * 7").is_err()); // dow > 6
    }

    #[test]
    fn parse_zero_step_errors() {
        assert!(CronExpr::parse("*/0 * * * *").is_err());
    }

    #[test]
    fn parse_reverse_range_errors() {
        assert!(CronExpr::parse("5-3 * * * *").is_err());
    }

    // ---- matches() 测试 ----

    #[test]
    fn matches_every_minute() {
        let expr = CronExpr::parse("* * * * *").unwrap();
        assert!(expr.matches(dt(2026, 7, 8, 12, 0)));
        assert!(expr.matches(dt(2026, 7, 8, 23, 59)));
        assert!(expr.matches(dt(2026, 1, 1, 0, 0)));
    }

    #[test]
    fn matches_weekday_morning() {
        // "0 9 * * 1-5" — 工作日 09:00
        let expr = CronExpr::parse("0 9 * * 1-5").unwrap();
        // 2026-07-08 是周三
        assert!(expr.matches(dt(2026, 7, 8, 9, 0)));
        // 周六不匹配
        // 2026-07-11 是周六
        assert!(!expr.matches(dt(2026, 7, 11, 9, 0)));
        // 时间不对
        assert!(!expr.matches(dt(2026, 7, 8, 10, 0)));
    }

    #[test]
    fn matches_every_15_minutes() {
        let expr = CronExpr::parse("*/15 * * * *").unwrap();
        assert!(expr.matches(dt(2026, 7, 8, 12, 0)));
        assert!(expr.matches(dt(2026, 7, 8, 12, 15)));
        assert!(expr.matches(dt(2026, 7, 8, 12, 30)));
        assert!(expr.matches(dt(2026, 7, 8, 12, 45)));
        assert!(!expr.matches(dt(2026, 7, 8, 12, 7)));
    }

    #[test]
    fn matches_monthly_first() {
        let expr = CronExpr::parse("0 0 1 * *").unwrap();
        assert!(expr.matches(dt(2026, 7, 1, 0, 0)));
        assert!(expr.matches(dt(2026, 2, 1, 0, 0)));
        assert!(!expr.matches(dt(2026, 7, 2, 0, 0)));
        assert!(!expr.matches(dt(2026, 7, 1, 1, 0)));
    }

    #[test]
    fn matches_dom_and_dow_or_logic() {
        // "0 0 1 * 0" — 每月 1 号 OR 每周日
        let expr = CronExpr::parse("0 0 1 * 0").unwrap();
        // 2026-07-01 是周三，但 是 1 号 → 匹配（DOM 匹配）
        assert!(expr.matches(dt(2026, 7, 1, 0, 0)));
        // 2026-07-05 是周日，不是 1 号 → 匹配（DOW 匹配）
        assert!(expr.matches(dt(2026, 7, 5, 0, 0)));
        // 2026-07-08 是周三，不是 1 号 → 不匹配
        assert!(!expr.matches(dt(2026, 7, 8, 0, 0)));
    }

    #[test]
    fn matches_dom_and_dow_and_when_one_is_star() {
        // "0 0 15 * *" — 每月 15 号（DOW 是 *，用 AND）
        let expr = CronExpr::parse("0 0 15 * *").unwrap();
        assert!(expr.matches(dt(2026, 7, 15, 0, 0)));
        assert!(!expr.matches(dt(2026, 7, 14, 0, 0)));
    }

    #[test]
    fn matches_specific_month() {
        // "0 0 1 6 *" — 6 月 1 号
        let expr = CronExpr::parse("0 0 1 6 *").unwrap();
        assert!(expr.matches(dt(2026, 6, 1, 0, 0)));
        assert!(!expr.matches(dt(2026, 7, 1, 0, 0)));
    }

    #[test]
    fn display_shows_raw() {
        let expr = CronExpr::parse("0 9 * * 1-5").unwrap();
        assert_eq!(format!("{expr}"), "0 9 * * 1-5");
    }

    #[test]
    fn parse_and_match_real_world_expressions() {
        // 测试常见 cron 表达式
        let cases = vec![
            ("*/5 * * * *", true),   // 每 5 分钟
            ("0 */2 * * *", true),   // 每 2 小时
            ("0 9 * * 1-5", true),   // 工作日 9 点
            ("0 0 * * 0", true),     // 每周日
            ("0 0 1 */3 *", true),   // 每季度首月 1 号
            ("30 3,15 * * *", true), // 每天 3:30 和 15:30
            ("0 0 1,15 * *", true),  // 每月 1 号和 15 号
        ];
        for (expr_str, should_parse) in cases {
            let result = CronExpr::parse(expr_str);
            assert!(
                result.is_ok() == should_parse,
                "expr='{expr_str}', parsed={result:?}"
            );
        }
    }
}

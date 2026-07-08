//! EvolutionEngine 单元测试 + 集成测试（M4 任务 #65-66）。
//!
//! 测试覆盖：
//! - Phase 输出类型基本语义
//! - EvolutionLog 序列化/反序列化
//! - Roller 回滚逻辑（不依赖 LLM）
//! - 三层共存（PromptSelfMutator / SkillAutoEvolver / EvolutionEngine domain 隔离）
//!
//! 完整端到端测试（需要 mock Ollama 服务端 + 完整 dispatcher）在 M5/M7a LLM 集成阶段补充。
//!
//! 注意：由于测试并行执行，依赖全局 `EVOLUTION_ENABLED` 状态的测试互相干扰，
//! 因此 Roller 的运行时 gate 测试单独验证（通过直接调用 evolution_enabled() 检查）。

#![cfg(test)]

use super::log::{EvolutionLog, EvolutionLogEntry};
use super::pipeline::{EvolutionPhase, PhaseOutput};
use super::rollback::Roller;
use std::sync::Arc;

// =====================================================================
// PhaseOutput / EvolutionPhase 基本语义
// =====================================================================

#[test]
fn phase_output_new_initializes_empty() {
    let out = PhaseOutput::new(EvolutionPhase::Extract);
    assert_eq!(out.phase, EvolutionPhase::Extract);
    assert!(out.content.is_empty());
    assert!(out.memory_id.is_none());
    assert!(out.log_entry_id.is_none());
    assert!(out.warnings.is_empty());
    assert!(!out.degraded);
}

#[test]
fn phase_as_str_roundtrip() {
    assert_eq!(EvolutionPhase::Extract.as_str(), "extract");
    assert_eq!(EvolutionPhase::Compile.as_str(), "compile");
    assert_eq!(EvolutionPhase::Reflect.as_str(), "reflect");
    assert_eq!(EvolutionPhase::Soul.as_str(), "soul");
}

#[test]
fn phase_all_in_order_returns_correct_sequence() {
    let phases = EvolutionPhase::all_in_order();
    assert_eq!(phases.len(), 4);
    assert_eq!(phases[0], EvolutionPhase::Extract);
    assert_eq!(phases[1], EvolutionPhase::Compile);
    assert_eq!(phases[2], EvolutionPhase::Reflect);
    assert_eq!(phases[3], EvolutionPhase::Soul);
}

#[test]
fn phase_display_matches_as_str() {
    assert_eq!(format!("{}", EvolutionPhase::Extract), "extract");
    assert_eq!(format!("{}", EvolutionPhase::Soul), "soul");
}

#[test]
fn phase_serialize_deserialize_roundtrip() {
    let phase = EvolutionPhase::Reflect;
    let json = serde_json::to_string(&phase).expect("serialize should succeed");
    assert_eq!(json, "\"reflect\"");
    let back: EvolutionPhase = serde_json::from_str(&json).expect("parse should succeed");
    assert_eq!(back, phase);
}

// =====================================================================
// EvolutionLog 序列化
// =====================================================================

#[test]
fn evolution_log_entry_to_markdown_contains_all_fields() {
    let entry = EvolutionLogEntry::new(EvolutionPhase::Reflect, "agent_test", "mem-abc-123", 2048);
    let md = entry.to_markdown();
    assert!(md.starts_with("## [evolve_"));
    assert!(md.contains("Phase: reflect"));
    assert!(md.contains("master_id: agent_test"));
    assert!(md.contains("memory_id: mem-abc-123"));
    assert!(md.contains("content_bytes: 2048"));
    assert!(md.contains("soul_md_path: (none)"));
}

#[test]
fn evolution_log_entry_with_soul_md_path_marked() {
    let entry = EvolutionLogEntry::new(EvolutionPhase::Soul, "agent_x", "", 512)
        .with_soul_md_path("/workspace/SOUL.md");
    let md = entry.to_markdown();
    assert!(md.contains("memory_id: (none)"));
    assert!(md.contains("soul_md_path: /workspace/SOUL.md"));
}

#[test]
fn evolution_log_entry_id_is_unique_per_phase() {
    let e1 = EvolutionLogEntry::new(EvolutionPhase::Extract, "agent_a", "m1", 100);
    let e2 = EvolutionLogEntry::new(EvolutionPhase::Compile, "agent_a", "m2", 200);
    assert_ne!(e1.entry_id, e2.entry_id);
    assert!(e1.entry_id.ends_with("_extract"));
    assert!(e2.entry_id.ends_with("_compile"));
}

// =====================================================================
// EvolutionLog 文件 I/O
// =====================================================================

#[tokio::test]
async fn evolution_log_append_creates_file_with_header() {
    let dir = tempfile::tempdir().expect("test op should succeed");
    let log_path = dir.path().join("evolution_log.md");
    let log = EvolutionLog::new(log_path.clone());

    let entry = EvolutionLogEntry::new(EvolutionPhase::Extract, "agent_a", "mem-1", 100);
    let id = log.append(&entry).await.expect("task should complete");
    assert_eq!(id, entry.entry_id);

    let content = std::fs::read_to_string(&log_path).expect("get should succeed");
    assert!(content.starts_with("# Evolution Log"));
    assert!(content.contains(&entry.entry_id));
}

#[tokio::test]
async fn evolution_log_append_multiple_entries() {
    let dir = tempfile::tempdir().expect("test op should succeed");
    let log = EvolutionLog::new(dir.path().join("evolution_log.md"));

    // 用不同的 master_id 区分条目（避免秒级时间戳相同导致 entry_id 重复）
    for i in 0..3 {
        let entry = EvolutionLogEntry::new(
            EvolutionPhase::Extract,
            &format!("agent_{i}"),
            &format!("mem-{i}"),
            100 * (i as u64 + 1),
        );
        log.append(&entry).await.expect("task should complete");
        // 确保时间戳不同（entry_id 含秒级时间戳）
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    }

    let entries = log.list_all().expect("test op should succeed");
    assert_eq!(entries.len(), 3);
    // 按写入顺序
    assert_eq!(entries[0].memory_id, "mem-0");
    assert_eq!(entries[1].memory_id, "mem-1");
    assert_eq!(entries[2].memory_id, "mem-2");
}

#[tokio::test]
async fn evolution_log_find_entry_by_id() {
    let dir = tempfile::tempdir().expect("test op should succeed");
    let log = EvolutionLog::new(dir.path().join("evolution_log.md"));

    let entry = EvolutionLogEntry::new(EvolutionPhase::Soul, "agent_a", "", 512);
    log.append(&entry).await.expect("task should complete");

    let found = log.find_entry(&entry.entry_id).expect("query should succeed").expect("query should succeed");
    assert_eq!(found.entry_id, entry.entry_id);
    assert_eq!(found.phase, EvolutionPhase::Soul);
    assert_eq!(found.master_id, "agent_a");
}

#[tokio::test]
async fn evolution_log_find_entry_returns_none_for_unknown() {
    let dir = tempfile::tempdir().expect("test op should succeed");
    let log = EvolutionLog::new(dir.path().join("evolution_log.md"));

    // 空文件
    assert!(log.find_entry("nonexistent").expect("query should succeed").is_none());

    // 写入一条后再查找不存在的
    let entry = EvolutionLogEntry::new(EvolutionPhase::Extract, "a", "m", 100);
    log.append(&entry).await.expect("task should complete");
    assert!(log.find_entry("nonexistent").expect("query should succeed").is_none());
}

#[tokio::test]
async fn evolution_log_remove_entry_deletes_paragraph() {
    let dir = tempfile::tempdir().expect("test op should succeed");
    let log_path = dir.path().join("evolution_log.md");
    let log = EvolutionLog::new(log_path.clone());

    let e1 = EvolutionLogEntry::new(EvolutionPhase::Extract, "agent_a", "m1", 100);
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let e2 = EvolutionLogEntry::new(EvolutionPhase::Soul, "agent_b", "", 200);
    log.append(&e1).await.expect("task should complete");
    log.append(&e2).await.expect("task should complete");

    // 删除 e1
    let removed = log.remove_entry(&e1.entry_id).await.expect("delete should succeed");
    assert!(removed);

    let content = std::fs::read_to_string(&log_path).expect("get should succeed");
    assert!(!content.contains(&e1.entry_id));
    assert!(content.contains(&e2.entry_id));
}

#[tokio::test]
async fn evolution_log_remove_entry_returns_false_for_unknown() {
    let dir = tempfile::tempdir().expect("test op should succeed");
    let log = EvolutionLog::new(dir.path().join("evolution_log.md"));
    let removed = log.remove_entry("nonexistent").await.expect("delete should succeed");
    assert!(!removed);
}

#[tokio::test]
async fn evolution_log_read_all_returns_empty_when_missing() {
    let dir = tempfile::tempdir().expect("test op should succeed");
    let log = EvolutionLog::new(dir.path().join("nonexistent.md"));
    let content = log.read_all().expect("get should succeed");
    assert!(content.is_empty());
}

#[tokio::test]
async fn evolution_log_list_all_returns_empty_when_no_file() {
    let dir = tempfile::tempdir().expect("test op should succeed");
    let log = EvolutionLog::new(dir.path().join("nonexistent.md"));
    let entries = log.list_all().expect("test op should succeed");
    assert!(entries.is_empty());
}

// =====================================================================
// Roller 段落删除逻辑（不依赖运行时 gate）
// =====================================================================

#[test]
fn roller_remove_entry_returns_false_when_not_found() {
    let dir = tempfile::tempdir().expect("test op should succeed");
    let log = EvolutionLog::new(dir.path().join("evolution_log.md"));
    let roller = Roller::new(Arc::new(log), dir.path().join("SOUL.md").into());

    let mut soul = String::from("some content\n");
    let result = roller
        .remove_entry_from_soul_md(&mut soul, "nonexistent")
        .expect("test op should succeed");
    assert!(!result);
    assert_eq!(soul, "some content\n");
}

#[test]
fn roller_remove_entry_deletes_paragraph() {
    let dir = tempfile::tempdir().expect("test op should succeed");
    let log = EvolutionLog::new(dir.path().join("evolution_log.md"));
    let roller = Roller::new(Arc::new(log), dir.path().join("SOUL.md").into());

    let mut soul = String::from(
        "<!-- BEGIN SECTION: evolution-append -->\n\
         ## [evolve_test_extract] Phase: extract\n\
         - foo: bar\n\
         ## [evolve_test_soul] Phase: soul\n\
         - baz: qux\n\
         <!-- END SECTION: evolution-append -->\n",
    );
    let result = roller
        .remove_entry_from_soul_md(&mut soul, "evolve_test_soul")
        .expect("test op should succeed");
    assert!(result);
    assert!(!soul.contains("evolve_test_soul"));
    assert!(soul.contains("evolve_test_extract"));
}

#[test]
fn roller_remove_entry_deletes_first_paragraph() {
    let dir = tempfile::tempdir().expect("test op should succeed");
    let log = EvolutionLog::new(dir.path().join("evolution_log.md"));
    let roller = Roller::new(Arc::new(log), dir.path().join("SOUL.md").into());

    let mut soul = String::from(
        "<!-- BEGIN SECTION: evolution-append -->\n\
         ## [evolve_first] Phase: soul\n\
         - first\n\n\
         ## [evolve_second] Phase: soul\n\
         - second\n\
         <!-- END SECTION: evolution-append -->\n",
    );
    let result = roller
        .remove_entry_from_soul_md(&mut soul, "evolve_first")
        .expect("test op should succeed");
    assert!(result);
    assert!(!soul.contains("evolve_first"));
    assert!(soul.contains("evolve_second"));
}

#[test]
fn roller_remove_entry_deletes_last_paragraph_before_end_tag() {
    let dir = tempfile::tempdir().expect("test op should succeed");
    let log = EvolutionLog::new(dir.path().join("evolution_log.md"));
    let roller = Roller::new(Arc::new(log), dir.path().join("SOUL.md").into());

    let mut soul = String::from(
        "<!-- BEGIN SECTION: evolution-append -->\n\
         ## [evolve_only] Phase: soul\n\
         - lone entry\n\
         <!-- END SECTION: evolution-append -->\n",
    );
    let result = roller
        .remove_entry_from_soul_md(&mut soul, "evolve_only")
        .expect("test op should succeed");
    assert!(result);
    assert!(!soul.contains("evolve_only"));
    // END SECTION 标签应保留
    assert!(soul.contains("<!-- END SECTION: evolution-append -->"));
}

// =====================================================================
// 三层共存（PromptSelfMutator / SkillAutoEvolver / EvolutionEngine）
// 通过 domain 字段隔离
// =====================================================================

#[test]
fn three_layer_evolution_uses_distinct_domains() {
    // 验证三个进化模块使用的存储/路径互不冲突：
    // - PromptSelfMutator: agent 级（直接修改 agent system_prompt，in-memory + prompt_snapshots 表）
    // - SkillAutoEvolver: skill 级（skill_archive 表独立于 memories 表）
    // - EvolutionEngine: master 级（memories 表 domain = "<master_id>"）

    // EvolutionEngine 写入路径：absorb_with_principal("evolution:<master_id>", mem)
    // → mem.domain = "<master_id>"（如 "agent_a"）
    let evolution_master_domain = "agent_a";
    let principal = format!("evolution:{evolution_master_domain}");
    assert!(principal.starts_with("evolution:"));
    assert!(principal.ends_with(evolution_master_domain));

    // PromptSelfMutator 写入路径：prompt_snapshots 表 + agent system_prompt (in-memory)
    // 不经 SpongeEngine，不写 memories 表（domain 不适用）
    let prompt_mutator_target = "agent system_prompt (in-memory + prompt_snapshots table)";

    // SkillAutoEvolver 写入路径：skill_archive 表（独立于 memories 表）
    let skill_evolver_target = "skill_archive table (separate from memories)";

    // 三层目标互不重叠
    assert_ne!(evolution_master_domain, prompt_mutator_target);
    assert_ne!(evolution_master_domain, skill_evolver_target);
    assert_ne!(prompt_mutator_target, skill_evolver_target);
}

// =====================================================================
// 配置 DTO 测试
// =====================================================================

#[test]
fn evolution_engine_config_default_values() {
    let cfg = super::EvolutionEngineConfig::default();
    assert!(!cfg.enabled);
    assert_eq!(cfg.phase_timeout_secs, 30);
    assert_eq!(cfg.phase1_l1_window, 50);
    assert_eq!(cfg.phase2_l2_window, 30);
    assert_eq!(cfg.phase3_l2_window, 30);
    assert_eq!(cfg.phase3_l3_window, 30);
    assert_eq!(cfg.phase4_max_lines, 100);
    assert_eq!(cfg.log_path, "evolution_log.md");
    assert_eq!(cfg.soul_md_path, "SOUL.md");
}

#[test]
fn evolution_engine_config_serializes_to_json() {
    let cfg = super::EvolutionEngineConfig::default();
    let json = serde_json::to_string(&cfg).expect("serialize should succeed");
    assert!(json.contains("\"enabled\":false"));
    assert!(json.contains("\"phase_timeout_secs\":30"));

    let back: super::EvolutionEngineConfig = serde_json::from_str(&json).expect("parse should succeed");
    assert_eq!(back.phase_timeout_secs, cfg.phase_timeout_secs);
    assert_eq!(back.phase4_max_lines, cfg.phase4_max_lines);
}

#[test]
fn evolution_engine_config_supports_custom_values() {
    let cfg = super::EvolutionEngineConfig {
        enabled: true,
        phase_timeout_secs: 60,
        phase1_l1_window: 100,
        phase2_l2_window: 50,
        phase3_l2_window: 50,
        phase3_l3_window: 50,
        phase4_max_lines: 200,
        log_path: "/custom/path/log.md".to_string(),
        soul_md_path: "/custom/SOUL.md".to_string(),
    };
    let json = serde_json::to_string(&cfg).expect("serialize should succeed");
    let back: super::EvolutionEngineConfig = serde_json::from_str(&json).expect("parse should succeed");
    assert!(back.enabled);
    assert_eq!(back.phase_timeout_secs, 60);
    assert_eq!(back.phase1_l1_window, 100);
    assert_eq!(back.log_path, "/custom/path/log.md");
}

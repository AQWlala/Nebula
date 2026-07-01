//! v1.4 Memory Orchestrator — L2 认知层协调器。
//!
//! 对应设计文档 v7.0 §2.1 的 L2 Memory Orchestrator + §4.3 上下文组装策略。
//!
//! ## 职责
//!
//! * 协调 5 种记忆类型的检索与注入
//! * 上下文组装策略（设计文档 §4.3）：
//!   - 一次 LLM 调用最多注入 **3 种**记忆类型
//!   - 默认组合：**Semantic + Episodic**
//!   - 加 Procedural（如果任务匹配 Skill）
//!   - 加 Emotional（如果涉及偏好）
//!   - Metacognitive 只在反思时使用
//!   - **总 token 上限**：3000

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::memory::embedder::Embedder;
use crate::memory::l0_cache::L0Cache;
use crate::memory::lance_store::LanceStore;
use crate::memory::sqlite_store::SqliteStore;
use crate::memory::types::{Memory, MemoryType};

/// 上下文组装的 token 上限（设计文档 §4.3）。
const CONTEXT_TOKEN_BUDGET: usize = 3000;
/// 每种记忆类型最多取多少条。
const PER_TYPE_LIMIT: usize = 5;
/// 向量检索 top-k。
const VECTOR_TOP_K: usize = 20;

/// 粗略 token 估算（1 token ≈ 2 字符，中英混合启发式）。
fn estimate_tokens(text: &str) -> usize {
    (text.chars().count() / 2).max(1)
}

/// 组装好的上下文束。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBundle {
    /// 组装后的文本（可直接注入 LLM 系统提示）。
    pub text: String,
    /// 各记忆类型贡献的条目数。
    pub type_counts: Vec<(MemoryType, usize)>,
    /// 实际使用的 token 数（估算）。
    pub tokens_used: usize,
    /// 命中的记忆 id（供 L0 缓存回填）。
    pub memory_ids: Vec<String>,
}

impl ContextBundle {
    pub fn empty() -> Self {
        Self {
            text: String::new(),
            type_counts: Vec::new(),
            tokens_used: 0,
            memory_ids: Vec::new(),
        }
    }
}

/// 任务类型提示（决定选哪些记忆类型）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskHint {
    /// 是否为写作类任务（加 Procedural）。
    pub is_writing: bool,
    /// 是否涉及偏好/情感（加 Emotional）。
    pub involves_preference: bool,
    /// 是否为反思任务（加 Metacognitive）。
    pub is_reflection: bool,
}

impl TaskHint {
    /// 从任务描述自动推断提示。
    pub fn infer(description: &str) -> Self {
        let lower = description.to_lowercase();
        let is_writing = ["写", "总结", "规划", "设计", "write", "summarize", "plan", "design"]
            .iter()
            .any(|k| lower.contains(k));
        let involves_preference = ["喜欢", "偏好", "习惯", "prefer", "like", "habit", "情感"]
            .iter()
            .any(|k| lower.contains(k));
        let is_reflection = ["反思", "回顾", "总结经验", "reflect", "review", "lesson"]
            .iter()
            .any(|k| lower.contains(k));
        Self {
            is_writing,
            involves_preference,
            is_reflection,
        }
    }
}

/// Memory Orchestrator。
pub struct MemoryOrchestrator {
    sqlite: Arc<SqliteStore>,
    lance: Arc<LanceStore>,
    embedder: Arc<Embedder>,
    l0: Arc<L0Cache>,
}

impl MemoryOrchestrator {
    pub fn new(
        sqlite: Arc<SqliteStore>,
        lance: Arc<LanceStore>,
        embedder: Arc<Embedder>,
        l0: Arc<L0Cache>,
    ) -> Self {
        Self {
            sqlite,
            lance,
            embedder,
            l0,
        }
    }

    /// 为一个任务组装上下文。
    ///
    /// 流程：
    /// 1. 推断任务提示（决定记忆类型组合，最多 3 种）
    /// 2. 向量检索 top-k 相关记忆
    /// 3. 按类型分组 + 截断
    /// 4. 在 3000 token 预算内组装文本
    pub async fn assemble_context(&self, task: &str) -> anyhow::Result<ContextBundle> {
        let hint = TaskHint::infer(task);
        let selected_types = select_memory_types(&hint);

        debug!(
            target: "nine_snake.memory.orchestrator",
            task = %task.chars().take(60).collect::<String>(),
            types = ?selected_types,
            "assembling context"
        );

        // 1. 向量检索
        let query_emb = self.embedder.embed(task).await?;
        let hits = self.lance.search(&query_emb, VECTOR_TOP_K).await?;
        if hits.is_empty() {
            return Ok(ContextBundle::empty());
        }

        // 2. 取完整 Memory
        let ids: Vec<String> = hits.iter().map(|(id, _)| id.clone()).collect();
        let memories = self.sqlite.get_many(&ids).await?;

        // 3. 按 id 建立分数映射（用于排序）
        let score_by_id: std::collections::HashMap<&str, f32> =
            hits.iter().map(|(id, s)| (id.as_str(), *s)).collect();

        // 4. 按选中类型分组
        let mut grouped: std::collections::HashMap<MemoryType, Vec<Memory>> =
            std::collections::HashMap::new();
        for mem in memories {
            if selected_types.contains(&mem.memory_type) {
                grouped.entry(mem.memory_type).or_default().push(mem);
            }
        }

        // 5. 每组按向量分数排序，取前 PER_TYPE_LIMIT 条
        for mems in grouped.values_mut() {
            mems.sort_by(|a, b| {
                let sa = score_by_id.get(a.id.as_str()).copied().unwrap_or(0.0);
                let sb = score_by_id.get(b.id.as_str()).copied().unwrap_or(0.0);
                sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
            });
            mems.truncate(PER_TYPE_LIMIT);
        }

        // 6. 在 token 预算内组装文本
        let mut text = String::new();
        let mut tokens_used = 0;
        let mut type_counts = Vec::new();
        let mut memory_ids = Vec::new();

        for mt in &selected_types {
            if let Some(mems) = grouped.get(mt) {
                let mut count = 0;
                for mem in mems {
                    let line = format!("[{}] {}\n", mt, mem.content);
                    let line_tokens = estimate_tokens(&line);
                    if tokens_used + line_tokens > CONTEXT_TOKEN_BUDGET {
                        break;
                    }
                    text.push_str(&line);
                    tokens_used += line_tokens;
                    memory_ids.push(mem.id.clone());
                    // 回填 L0 热缓存
                    self.l0.insert(mem.clone());
                    count += 1;
                }
                if count > 0 {
                    type_counts.push((*mt, count));
                }
            }
        }

        Ok(ContextBundle {
            text,
            type_counts,
            tokens_used,
            memory_ids,
        })
    }

    /// 快速路径：仅从 L0 会话上下文取记忆（不查向量库）。
    pub fn session_context(&self, mt: Option<MemoryType>) -> Vec<Arc<Memory>> {
        match mt {
            Some(t) => self.l0.session_by_type(t),
            None => self.l0.session_snapshot(),
        }
    }
}

/// 根据任务提示选择记忆类型组合（最多 3 种）。
fn select_memory_types(hint: &TaskHint) -> Vec<MemoryType> {
    use MemoryType::*;
    let mut types = vec![Semantic, Episodic]; // 默认组合
    if hint.is_writing {
        types.push(Procedural);
    }
    if hint.involves_preference {
        types.push(Emotional);
    }
    if hint.is_reflection {
        types.push(Metacognitive);
    }
    // 设计规定最多 3 种
    types.truncate(3);
    types
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_types_default() {
        let hint = TaskHint::default();
        let types = select_memory_types(&hint);
        assert_eq!(types, vec![MemoryType::Semantic, MemoryType::Episodic]);
    }

    #[test]
    fn select_types_writing_adds_procedural() {
        let hint = TaskHint {
            is_writing: true,
            ..Default::default()
        };
        let types = select_memory_types(&hint);
        assert_eq!(types.len(), 3);
        assert!(types.contains(&MemoryType::Procedural));
    }

    #[test]
    fn select_types_max_three() {
        let hint = TaskHint {
            is_writing: true,
            involves_preference: true,
            is_reflection: true,
        };
        let types = select_memory_types(&hint);
        assert!(types.len() <= 3, "max 3 types, got {}", types.len());
    }

    #[test]
    fn task_hint_infers_writing() {
        let hint = TaskHint::infer("帮我写一份周报");
        assert!(hint.is_writing);
    }

    #[test]
    fn task_hint_infers_preference() {
        let hint = TaskHint::infer("我喜欢简洁的风格");
        assert!(hint.involves_preference);
    }

    #[test]
    fn task_hint_infers_reflection() {
        let hint = TaskHint::infer("反思一下这次失败的原因");
        assert!(hint.is_reflection);
    }

    #[test]
    fn empty_bundle() {
        let b = ContextBundle::empty();
        assert!(b.text.is_empty());
        assert_eq!(b.tokens_used, 0);
    }
}

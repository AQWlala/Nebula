//! Core data model for the nebula v7.0 memory system.
//!
//! These types are intentionally `Clone` + `Serialize`/`Deserialize` so
//! they can flow freely through the Tauri command boundary, the swarm
//! orchestrator and the SQLite/LanceDB persistence layers.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Cognitive classification of a single memory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    /// Factual, declarative knowledge ("the sky is blue").
    Semantic,
    /// Time-stamped events and experiences ("user asked about X at 14:32").
    Episodic,
    /// Reusable procedures and skills ("how to reset a router").
    Procedural,
    /// Emotional impressions and value judgements ("user seemed frustrated").
    Emotional,
    /// Self-reflective observations about cognition itself.
    Metacognitive,
}

impl MemoryType {
    /// String form used by the database and HTTP APIs.
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryType::Semantic => "semantic",
            MemoryType::Episodic => "episodic",
            MemoryType::Procedural => "procedural",
            MemoryType::Emotional => "emotional",
            MemoryType::Metacognitive => "metacognitive",
        }
    }
}

impl fmt::Display for MemoryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MemoryType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "semantic" => Ok(MemoryType::Semantic),
            "episodic" => Ok(MemoryType::Episodic),
            "procedural" => Ok(MemoryType::Procedural),
            "emotional" => Ok(MemoryType::Emotional),
            "metacognitive" => Ok(MemoryType::Metacognitive),
            other => Err(format!("unknown memory type: {other}")),
        }
    }
}

/// Memory layer hierarchy (L0-L5 active, L6-L7 reserved for v1.5+).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MemoryLayer {
    /// Temporary cache (single conversation turn).
    L0,
    /// Rolling message history within a session.
    L1,
    /// Cross-session experience.
    L2,
    /// Concrete facts.
    L3,
    /// Distilled knowledge.
    L4,
    /// Lessons learned from mistakes.
    L5,
    /// Re-usable principles.
    L6,
    /// Singularity — the core, never compressed.
    L7,
}

impl MemoryLayer {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryLayer::L0 => "L0",
            MemoryLayer::L1 => "L1",
            MemoryLayer::L2 => "L2",
            MemoryLayer::L3 => "L3",
            MemoryLayer::L4 => "L4",
            MemoryLayer::L5 => "L5",
            MemoryLayer::L6 => "L6",
            MemoryLayer::L7 => "L7",
        }
    }

    /// Layers that the black-hole engine is *never* allowed to touch.
    pub fn is_immutable(&self) -> bool {
        matches!(self, MemoryLayer::L7)
    }
}

impl fmt::Display for MemoryLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MemoryLayer {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_uppercase().as_str() {
            "L0" => Ok(MemoryLayer::L0),
            "L1" => Ok(MemoryLayer::L1),
            "L2" => Ok(MemoryLayer::L2),
            "L3" => Ok(MemoryLayer::L3),
            "L4" => Ok(MemoryLayer::L4),
            "L5" => Ok(MemoryLayer::L5),
            "L6" => Ok(MemoryLayer::L6),
            "L7" => Ok(MemoryLayer::L7),
            other => Err(format!("unknown memory layer: {other}")),
        }
    }
}

/// Origin of a memory — useful for debugging and trust scoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    UserInput,
    AgentOutput,
    Reflection,
    System,
    External,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::UserInput => "user_input",
            SourceKind::AgentOutput => "agent_output",
            SourceKind::Reflection => "reflection",
            SourceKind::System => "system",
            SourceKind::External => "external",
        }
    }
}

impl FromStr for SourceKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user_input" => Ok(SourceKind::UserInput),
            "agent_output" => Ok(SourceKind::AgentOutput),
            "reflection" => Ok(SourceKind::Reflection),
            "system" => Ok(SourceKind::System),
            "external" => Ok(SourceKind::External),
            other => Err(format!("unknown source kind: {other}")),
        }
    }
}

/// T-E-B-04: 记忆溯源信息。序列化进 `Memory.metadata["provenance"]`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    /// 来源类型(与 Memory.source 一致)。
    pub source: String,
    /// 触发工具/agent 名(如 "writer" / "sponge" / "user"),未知为 None。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// 内容 SHA-256 哈希(前 16 字符),用于修改链比对。
    pub content_hash: String,
    /// 吸收时间(UTC 时间戳,秒)。
    pub absorbed_at: i64,
}

impl Provenance {
    /// 构造 provenance,自动计算 content_hash。
    pub fn new(source: &str, tool: Option<&str>, content: &str) -> Self {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let hash = hasher.finalize();
        // 取前 8 字节,展开为 16 个十六进制字符(`[u8]` 不实现 `LowerHex`,
        // 需逐字节格式化)。
        let content_hash: String = hash[..8].iter().map(|b| format!("{:02x}", b)).collect();
        Self {
            source: source.to_string(),
            tool: tool.map(|s| s.to_string()),
            content_hash,
            absorbed_at: chrono::Utc::now().timestamp(),
        }
    }
}

/// Kind of edge in the memory knowledge graph.
///
/// # T-E-B-16 MDRM 5 维扩展
///
/// 原 v2.0 仅 5 种关系(Causes/Supports/Contradicts/References/DerivedFrom),
/// T-E-B-16 扩展为 5 维关系图谱(MDRM),新增 4 种关系:
/// - `Before` (时序维度):A 先于 B
/// - `SameEntity` (实体维度):A 与 B 指向同一实体
/// - `Contains` (层级维度):A 包含 B(与 DerivedFrom 互为反向)
/// - `Similar` (相似度维度):A 相似 B
///
/// 关系列存储为 TEXT,新增 kind 不需要 migration。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    // 因果维度(Causal)
    Causes,
    Supports,
    Contradicts,
    // 实体维度(Entity)
    References,
    SameEntity,
    // 层级维度(Hierarchical)
    DerivedFrom,
    Contains,
    // 时序维度(Temporal)
    Before,
    // 相似度维度(Similarity)
    Similar,
}

impl RelationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            RelationKind::Causes => "causes",
            RelationKind::Supports => "supports",
            RelationKind::Contradicts => "contradicts",
            RelationKind::References => "references",
            RelationKind::SameEntity => "same_entity",
            RelationKind::DerivedFrom => "derived_from",
            RelationKind::Contains => "contains",
            RelationKind::Before => "before",
            RelationKind::Similar => "similar",
        }
    }
}

impl FromStr for RelationKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "causes" => Ok(RelationKind::Causes),
            "supports" => Ok(RelationKind::Supports),
            "contradicts" => Ok(RelationKind::Contradicts),
            "references" => Ok(RelationKind::References),
            "same_entity" => Ok(RelationKind::SameEntity),
            "derived_from" => Ok(RelationKind::DerivedFrom),
            "contains" => Ok(RelationKind::Contains),
            "before" => Ok(RelationKind::Before),
            "similar" => Ok(RelationKind::Similar),
            other => Err(format!("unknown relation kind: {other}")),
        }
    }
}

/// Four pre-computed summaries at increasing granularity.
///
/// They are produced on `store` and refreshed by the black-hole
/// compression engine. The order of the tuple corresponds to the
/// `constants::SUMMARY_BUCKETS` array (`[50, 150, 500, 2000]`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MultiGranularity {
    pub s50: String,
    pub s150: String,
    pub s500: String,
    pub s2000: String,
}

impl MultiGranularity {
    /// Builds a `MultiGranularity` from raw pre-computed strings.
    pub fn new(
        s50: impl Into<String>,
        s150: impl Into<String>,
        s500: impl Into<String>,
        s2000: impl Into<String>,
    ) -> Self {
        Self {
            s50: s50.into(),
            s150: s150.into(),
            s500: s500.into(),
            s2000: s2000.into(),
        }
    }

    /// Returns the summary at the requested bucket index (0..=3).
    pub fn at_bucket(&self, idx: usize) -> &str {
        match idx {
            0 => &self.s50,
            1 => &self.s150,
            2 => &self.s500,
            _ => &self.s2000,
        }
    }
}

/// The canonical memory record stored across all subsystems.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    /// Stable UUIDv4.
    pub id: String,
    /// Cognitive type.
    pub memory_type: MemoryType,
    /// Layer (L0..L7).
    pub layer: MemoryLayer,
    /// Raw content (un-truncated).
    pub content: String,
    /// Pre-computed multi-granularity summaries.
    pub summary: MultiGranularity,
    /// 512-dim BGE embedding (BGE-small-zh-v1.5). Empty when not yet embedded.
    pub embedding: Vec<f32>,
    /// Importance score in `[0.0, 1.0]`.
    pub importance: f32,
    /// Number of times this memory has been retrieved.
    pub access_count: u32,
    /// Unix timestamp (seconds) of the most recent access.
    pub last_access: i64,
    /// Unix timestamp (seconds) of creation.
    pub created_at: i64,
    /// Origin of this memory.
    pub source: SourceKind,
    /// Free-form extension bag (lang ids, file paths, etc.).
    pub metadata: serde_json::Value,
    /// If this record is the result of a black-hole compression, this
    /// points to the parent record.
    pub compressed_from: Option<String>,
    pub compression_gen: u32,
    pub pinned: bool,
    pub archived: bool,
    /// T-E-A-09: 写入时记录的吸收成本(USD)。
    /// `None` 表示未追踪(旧记忆 / cost_tracker 未注入)。
    /// `Some(0.0)` 表示已追踪但为零(本地 Ollama + 未启用 EntityExtractor)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingest_cost: Option<f64>,
    /// M2a 任务 #28: 域标识（P0-9 修复）。
    ///
    /// 用于按"域"隔离记忆，与 CostSource（触发场景）和 SourceKind（来源类型）
    /// 正交，构成第三维度。默认 "shared"（公共域，向后兼容旧记忆）。
    ///
    /// 典型值：
    /// - `"shared"`：默认公共域（用户对话、文件吸收等）
    /// - `"system"`：系统域（EvolutionEngine 写入、Soul 反哺等）
    /// - `"agent_a"` / `"worker:task_123"`：特定 agent / worker 域
    ///
    /// M2b 将引入 PrincipalDomainMap 实现 ACL 按 domain 过滤。
    /// M4 EvolutionEngine 通过 absorb_with_principal() 指定 domain。
    #[serde(default = "default_domain")]
    pub domain: String,
}

/// Memory.domain 的默认值。
fn default_domain() -> String {
    "shared".to_string()
}

impl Memory {
    /// Convenience constructor for a freshly-spawned memory.
    pub fn new(
        memory_type: MemoryType,
        layer: MemoryLayer,
        content: impl Into<String>,
        source: SourceKind,
    ) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            memory_type,
            layer,
            content: content.into(),
            summary: MultiGranularity::default(),
            embedding: Vec::new(),
            importance: 0.5,
            access_count: 0,
            last_access: now,
            created_at: now,
            source,
            metadata: serde_json::json!({}),
            compressed_from: None,
            compression_gen: 0,
            pinned: layer == MemoryLayer::L7,
            archived: false,
            ingest_cost: None,
            domain: default_domain(),
        }
    }

    /// Records an access (bumps counter + last_access). Returns the
    /// updated importance (see [`crate::memory::importance`]).
    pub fn touch(&mut self, now: i64) {
        self.access_count = self.access_count.saturating_add(1);
        self.last_access = now;
    }

    /// v1.0.1 P0#12: returns `true` when the `content` field
    /// looks like a secret.  Used by the sponge / blackhole
    /// write paths to blank out the `s2000` summary and set
    /// the `masked` column so the secret never reaches the
    /// long-form summary or the JSON dumps.
    ///
    /// Heuristics (deliberately conservative — false positives
    /// are fine, false negatives are not):
    ///
    /// 1. The lower-cased content contains any of: `api_key`,
    ///    `apikey`, `password`, `passwd`, `secret`, `token`,
    ///    `bearer `, `aws_access`, `aws_secret`,
    ///    `private_key`, `client_secret`.  These are
    ///    case-insensitive substring matches; the trigger
    ///    tokens come from the OAuth 2.0 and AWS secret
    ///    naming conventions.
    /// 2. The content contains a contiguous run of base64-ish
    ///    characters (A–Z, a–z, 0–9, +, /, =) of length ≥ 40
    ///    — long enough to be a real secret, short enough
    ///    not to flag prose.  We require the run to be
    ///    followed or preceded by a non-base64 character
    ///    (whitespace, punctuation) to avoid matching plain
    ///    English.
    /// 3. The content contains a JWT-shaped triple of
    ///    `header.payload.signature` separated by dots, where
    ///    each segment is base64-url.
    pub fn is_sensitive(&self) -> bool {
        sensitive_text_predicate(&self.content)
    }
}

/// v1.0.1 P0#12: free-function form of [`Memory::is_sensitive`]
/// so callers can run the predicate on arbitrary text (e.g.
/// user-supplied `code` fields in skills).
pub fn sensitive_text_predicate(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    const TRIGGERS: &[&str] = &[
        "api_key",
        "apikey",
        "password",
        "passwd",
        "secret",
        "token",
        "bearer ",
        "aws_access",
        "aws_secret",
        "private_key",
        "private key",
        "client_secret",
    ];
    if TRIGGERS.iter().any(|t| lower.contains(t)) {
        return true;
    }
    // Long base64 run: at least 40 contiguous characters
    // from the base64 alphabet, bounded by non-base64.
    let mut run: usize = 0;
    let mut best_run: usize = 0;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric()
            || ch == '+'
            || ch == '/'
            || ch == '='
            || ch == '-'
            || ch == '_'
        {
            run = run.saturating_add(1);
            if run > best_run {
                best_run = run;
            }
        } else {
            run = 0;
        }
    }
    if best_run >= 40 {
        return true;
    }
    // JWT shape: three base64-url segments separated by dots,
    // each at least 4 chars.
    let mut dot_runs = 0usize;
    let mut seg_len = 0usize;
    let mut ok = true;
    for ch in text.chars() {
        if ch == '.' {
            if seg_len >= 4 {
                dot_runs += 1;
            } else {
                ok = false;
                break;
            }
            seg_len = 0;
        } else if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            seg_len = seg_len.saturating_add(1);
        } else {
            ok = false;
            break;
        }
    }
    if ok && dot_runs == 2 && seg_len >= 4 {
        return true;
    }
    false
}

/// A relation between two memories (graph edge).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRelation {
    pub id: String,
    pub src_id: String,
    pub dst_id: String,
    pub kind: RelationKind,
    pub weight: f32,
    pub created_at: i64,
    pub evidence: Option<String>,
}

impl MemoryRelation {
    pub fn new(src_id: impl Into<String>, dst_id: impl Into<String>, kind: RelationKind) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            src_id: src_id.into(),
            dst_id: dst_id.into(),
            kind,
            weight: 1.0,
            created_at: chrono::Utc::now().timestamp(),
            evidence: None,
        }
    }

    pub fn with_evidence(mut self, evidence: impl Into<String>) -> Self {
        self.evidence = Some(evidence.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_round_trip() {
        for l in [
            MemoryLayer::L0,
            MemoryLayer::L1,
            MemoryLayer::L2,
            MemoryLayer::L3,
            MemoryLayer::L4,
            MemoryLayer::L5,
            MemoryLayer::L6,
            MemoryLayer::L7,
        ] {
            let parsed: MemoryLayer = l.as_str().parse().expect("parse should succeed");
            assert_eq!(parsed, l);
        }
    }

    #[test]
    fn only_l7_is_immutable() {
        assert!(MemoryLayer::L7.is_immutable());
        for l in [
            MemoryLayer::L0,
            MemoryLayer::L1,
            MemoryLayer::L2,
            MemoryLayer::L3,
            MemoryLayer::L4,
            MemoryLayer::L5,
            MemoryLayer::L6,
        ] {
            assert!(!l.is_immutable());
        }
    }

    #[test]
    fn memory_new_initialises_pinned_for_l7() {
        let m = Memory::new(
            MemoryType::Semantic,
            MemoryLayer::L7,
            "core",
            SourceKind::System,
        );
        assert!(m.pinned);
        let m2 = Memory::new(
            MemoryType::Semantic,
            MemoryLayer::L3,
            "fact",
            SourceKind::UserInput,
        );
        assert!(!m2.pinned);
    }

    #[test]
    fn touch_increments_counter() {
        let mut m = Memory::new(
            MemoryType::Episodic,
            MemoryLayer::L1,
            "hi",
            SourceKind::UserInput,
        );
        let before = m.access_count;
        m.touch(123);
        assert_eq!(m.access_count, before + 1);
        assert_eq!(m.last_access, 123);
    }

    /// v1.0.1 P0#12: the sensitive-content predicate is the
    /// load-bearing primitive for the masking path.  These
    /// cases pin the trigger list (false positives OK, false
    /// negatives not OK).
    #[test]
    fn is_sensitive_detects_api_key_pattern() {
        let cases = [
            "sk-abc123def456ghi789jkl012mno345pqr678stu901vwx",
            "MY API_KEY = hunter2-hunter2-hunter2-hunter2-hunter2-hunter2",
            "password: correcthorsebatterystaple",
            "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.SflKxw",
            "-----BEGIN PRIVATE KEY-----",
            // JWT-shaped: 3 base64-url segments, each >= 4 chars.
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
            // Long base64 run: 40+ contiguous chars.
            "blob: AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ];
        for c in cases {
            assert!(
                sensitive_text_predicate(c),
                "expected `{c}` to be flagged sensitive"
            );
        }
    }

    #[test]
    fn is_sensitive_ignores_normal_prose() {
        let cases = [
            "the quick brown fox jumps over the lazy dog",
            "今天午饭吃了红烧排骨",
            "function add(a, b) { return a + b; }",
            // Short base64-ish run (under 40 chars).
            "abc123",
        ];
        for c in cases {
            assert!(
                !sensitive_text_predicate(c),
                "expected `{c}` to NOT be flagged sensitive"
            );
        }
    }

    #[test]
    fn memory_is_sensitive_delegates_to_predicate() {
        let m = Memory::new(
            MemoryType::Semantic,
            MemoryLayer::L3,
            "MY_API_KEY=sk-abc",
            SourceKind::UserInput,
        );
        assert!(m.is_sensitive());
        let m2 = Memory::new(
            MemoryType::Semantic,
            MemoryLayer::L3,
            "the cat sat on the mat",
            SourceKind::UserInput,
        );
        assert!(!m2.is_sensitive());
    }

    // ---- T-E-B-04: Provenance 测试 ----

    /// `Provenance::new` 应填充 source / tool / content_hash / absorbed_at,
    /// 且 content_hash 为 SHA-256 前 16 个十六进制字符(8 字节)。
    #[test]
    fn test_provenance_new() {
        let p = Provenance::new("user_input", Some("writer"), "hello world");
        assert_eq!(p.source, "user_input");
        assert_eq!(p.tool.as_deref(), Some("writer"));
        // SHA-256("hello world") = b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9
        // 前 16 hex 字符 = "b94d27b9934d3e08"
        assert_eq!(p.content_hash, "b94d27b9934d3e08");
        assert_eq!(p.content_hash.len(), 16);
        assert!(p.absorbed_at > 0);

        // tool 为 None 时不写入(序列化时 skip,这里只校验字段)。
        let p2 = Provenance::new("system", None, "x");
        assert!(p2.tool.is_none());
    }

    /// Provenance 序列化/反序列化 roundtrip 应保持字段一致,
    /// 且 `tool=None` 时 JSON 中不出现 `tool` 键(skip_serializing_if)。
    #[test]
    fn test_provenance_serialize() {
        let p = Provenance::new("agent_output", Some("writer"), "payload");
        let json = serde_json::to_value(&p).expect("test op should succeed");
        // roundtrip
        let back: Provenance = serde_json::from_value(json.clone()).expect("test op should succeed");
        assert_eq!(back.source, p.source);
        assert_eq!(back.tool, p.tool);
        assert_eq!(back.content_hash, p.content_hash);
        assert_eq!(back.absorbed_at, p.absorbed_at);
        // tool 存在时应出现
        assert!(json.get("tool").is_some());

        // tool=None 时 JSON 中无 tool 键
        let p_none = Provenance::new("system", None, "x");
        let json_none = serde_json::to_value(&p_none).expect("test op should succeed");
        assert!(json_none.get("tool").is_none());
        let back_none: Provenance = serde_json::from_value(json_none).expect("test op should succeed");
        assert!(back_none.tool.is_none());
    }

    // ---- T-E-A-09: ingest_cost 测试 ----

    /// `Memory::new` 应将 `ingest_cost` 初始化为 `None`。
    #[test]
    fn ingest_cost_defaults_to_none() {
        let m = Memory::new(
            MemoryType::Semantic,
            MemoryLayer::L3,
            "x",
            SourceKind::UserInput,
        );
        assert!(m.ingest_cost.is_none());
    }

    /// `ingest_cost = None` 时序列化的 JSON 不应出现 `ingest_cost` 键
    /// (`skip_serializing_if = "Option::is_none"` 生效),
    /// 且 roundtrip 应保持 `None`(`#[serde(default)]` 允许缺失键)。
    #[test]
    fn ingest_cost_skipped_in_json_when_none() {
        let m = Memory::new(
            MemoryType::Semantic,
            MemoryLayer::L3,
            "x",
            SourceKind::UserInput,
        );
        let json = serde_json::to_value(&m).expect("test op should succeed");
        assert!(
            json.get("ingest_cost").is_none(),
            "ingest_cost key should be skipped when None, got: {json}"
        );
        // roundtrip:缺失键 → None(serde default)
        let back: Memory = serde_json::from_value(json).expect("test op should succeed");
        assert!(back.ingest_cost.is_none());
    }

    /// `ingest_cost = Some(v)` 时 roundtrip 应保持原值,且 JSON 中出现该键。
    #[test]
    fn ingest_cost_roundtrip_preserves_value() {
        let mut m = Memory::new(
            MemoryType::Semantic,
            MemoryLayer::L3,
            "x",
            SourceKind::UserInput,
        );
        m.ingest_cost = Some(0.001234);
        let json = serde_json::to_value(&m).expect("test op should succeed");
        assert!(json.get("ingest_cost").is_some());
        let back: Memory = serde_json::from_value(json).expect("test op should succeed");
        assert_eq!(back.ingest_cost, Some(0.001234));
    }
}

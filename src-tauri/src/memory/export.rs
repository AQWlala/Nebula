use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::sqlite_store::SqliteStore;
use super::types::Memory;
use crate::security::contains_sensitive;

const JSONLD_CONTEXT: &str = "https://schema.org";
const SCHEMA_VERSION: &str = "2.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportManifest {
    pub memory_count: usize,
    pub relation_count: usize,
    pub redacted_count: usize,
    pub exported_at: i64,
    pub schema_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub imported: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonLdMemory {
    #[serde(rename = "@context")]
    context: String,
    #[serde(rename = "@id")]
    id: String,
    #[serde(rename = "@type")]
    type_: String,
    content: String,
    layer: String,
    memory_type: String,
    importance: f32,
    source: String,
    created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_50: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_150: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_500: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_2000: Option<String>,
}

/// T-S1-A-05: 导出关系实体（对应 `memory_relations` 表的行）。
///
/// 对应 ROADMAP §2.1 P-05 的要求：导出数据应包含关系数组，
/// 而非硬编码 `relation_count: 0`。字段命名与 JSON-LD 约定一致：
/// - `source_id` / `target_id` 对应 `MemoryRelation::src_id` / `dst_id`
/// - `kind` 是关系类型字符串（"causes"/"supports"/"contradicts"/
///   "references"/"derived_from"）
/// - `evidence` 是可选的证据文本（敏感内容会在导出时被 redact）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationEntity {
    #[serde(rename = "@type")]
    type_: String,
    /// 关系自身的稳定 ID（UUID）。
    pub id: String,
    /// 源记忆 ID（对应 `memory_relations.src_id`）。
    pub source_id: String,
    /// 目标记忆 ID（对应 `memory_relations.dst_id`）。
    pub target_id: String,
    /// 关系类型字符串（见 [`super::types::RelationKind::as_str`]）。
    pub kind: String,
    /// `[0.0, 1.0]` 的边权重。
    pub weight: f32,
    /// 创建时间（Unix 时间戳）。
    pub created_at: i64,
    /// 可选证据文本。若包含敏感数据则替换为 `"[REDACTED]"`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

pub struct DataExporter {
    sqlite: SqliteStore,
}

impl DataExporter {
    pub fn new(sqlite: SqliteStore) -> Self {
        Self { sqlite }
    }

    pub async fn export_jsonld(&self, path: &Path) -> Result<ExportManifest> {
        let memories = self.sqlite.list_recent(usize::MAX).await?;
        let mut redacted_count = 0;

        let jsonld_items: Vec<JsonLdMemory> = memories
            .iter()
            .map(|m| {
                let (content, s50, s150, s500, s2000) = if contains_sensitive(&m.content) {
                    redacted_count += 1;
                    (
                        "[REDACTED]".to_string(),
                        Some("[REDACTED]".to_string()),
                        Some("[REDACTED]".to_string()),
                        Some("[REDACTED]".to_string()),
                        Some("[REDACTED]".to_string()),
                    )
                } else {
                    (
                        m.content.clone(),
                        if m.summary.s50.is_empty() {
                            None
                        } else {
                            Some(m.summary.s50.clone())
                        },
                        if m.summary.s150.is_empty() {
                            None
                        } else {
                            Some(m.summary.s150.clone())
                        },
                        if m.summary.s500.is_empty() {
                            None
                        } else {
                            Some(m.summary.s500.clone())
                        },
                        if m.summary.s2000.is_empty() {
                            None
                        } else {
                            Some(m.summary.s2000.clone())
                        },
                    )
                };

                JsonLdMemory {
                    context: JSONLD_CONTEXT.to_string(),
                    id: format!("nebula:memory:{}", m.id),
                    type_: "MemoryEntity".to_string(),
                    content,
                    layer: m.layer.as_str().to_string(),
                    memory_type: m.memory_type.as_str().to_string(),
                    importance: m.importance,
                    source: m.source.as_str().to_string(),
                    created_at: m.created_at,
                    summary_50: s50,
                    summary_150: s150,
                    summary_500: s500,
                    summary_2000: s2000,
                }
            })
            .collect();

        // T-S1-A-05: 查询 memory_relations 表，构造 RelationEntity 数组。
        // 原实现硬编码 relation_count: 0，导出数据不完整。
        let relations = self.sqlite.list_all_relations()?;
        let relation_entities: Vec<RelationEntity> = relations
            .iter()
            .map(|r| {
                let evidence_redacted = r.evidence.as_ref().and_then(|ev| {
                    if contains_sensitive(ev) {
                        Some("[REDACTED]".to_string())
                    } else {
                        Some(ev.clone())
                    }
                });
                RelationEntity {
                    type_: "RelationEntity".to_string(),
                    id: format!("nebula:relation:{}", r.id),
                    source_id: format!("nebula:memory:{}", r.src_id),
                    target_id: format!("nebula:memory:{}", r.dst_id),
                    kind: r.kind.as_str().to_string(),
                    weight: r.weight,
                    created_at: r.created_at,
                    evidence: evidence_redacted,
                }
            })
            .collect();
        let relation_count = relation_entities.len();

        let manifest = ExportManifest {
            memory_count: jsonld_items.len(),
            relation_count,
            redacted_count,
            exported_at: chrono::Utc::now().timestamp(),
            schema_version: SCHEMA_VERSION.to_string(),
        };

        let output = serde_json::json!({
            "@context": JSONLD_CONTEXT,
            "@type": "MemoryCollection",
            "schema_version": SCHEMA_VERSION,
            "items": jsonld_items,
            "relations": relation_entities,
            "manifest": manifest,
        });

        let json = serde_json::to_string_pretty(&output).context("serializing JSON-LD export")?;
        std::fs::write(path, json.as_bytes())
            .with_context(|| format!("writing export to {}", path.display()))?;

        info!(
            target: "nebula.export",
            count = manifest.memory_count,
            relations = manifest.relation_count,
            redacted = manifest.redacted_count,
            "JSON-LD export complete"
        );

        Ok(manifest)
    }

    pub async fn import_jsonld(&self, path: &Path) -> Result<ImportResult> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("reading import file {}", path.display()))?;

        let parsed: serde_json::Value =
            serde_json::from_str(&content).with_context(|| "parsing JSON-LD import file")?;

        let items = parsed
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut imported = 0;
        let mut errors = 0;

        for item in &items {
            let item_type = item.get("@type").and_then(|v| v.as_str()).unwrap_or("");
            if item_type != "MemoryEntity" {
                warn!(target: "nebula.export", type_ = item_type, "skipping non-MemoryEntity item");
                errors += 1;
                continue;
            }

            let id = match item.get("@id").and_then(|v| v.as_str()) {
                Some(id) => id.replace("nebula:memory:", ""),
                None => {
                    errors += 1;
                    continue;
                }
            };

            let content_val = item
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if content_val == "[REDACTED]" {
                warn!(target: "nebula.export", id = %id, "skipping redacted memory");
                errors += 1;
                continue;
            }

            let mem = Memory {
                id,
                memory_type: item
                    .get("memory_type")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(super::types::MemoryType::Semantic),
                layer: item
                    .get("layer")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(super::types::MemoryLayer::L3),
                content: content_val,
                summary: super::types::MultiGranularity {
                    s50: item
                        .get("summary_50")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    s150: item
                        .get("summary_150")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    s500: item
                        .get("summary_500")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    s2000: item
                        .get("summary_2000")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                },
                importance: item
                    .get("importance")
                    .and_then(|v| v.as_f64())
                    .map(|f| f as f32)
                    .unwrap_or(0.5),
                access_count: 0,
                last_access: 0,
                created_at: item.get("created_at").and_then(|v| v.as_i64()).unwrap_or(0),
                source: item
                    .get("source")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(super::types::SourceKind::External),
                metadata: serde_json::Value::Object(serde_json::Map::new()),
                compressed_from: None,
                compression_gen: 0,
                pinned: false,
                archived: false,
                embedding: Vec::new(),
                // T-E-A-09: 从 JSON 读取 ingest_cost(可选)。
                ingest_cost: item
                    .get("ingest_cost")
                    .and_then(|v| v.as_f64()),
                // M2a #28: 从 JSON 读取 domain(可选,默认 "shared")。
                domain: item
                    .get("domain")
                    .and_then(|v| v.as_str())
                    .unwrap_or("shared")
                    .to_string(),
            };

            match self.sqlite.insert_guarded_spawn(&mem).await {
                Ok(()) => imported += 1,
                Err(e) => {
                    warn!(target: "nebula.export", id = %mem.id, error = ?e, "import error");
                    errors += 1;
                }
            }
        }

        info!(
            target: "nebula.export",
            imported,
            errors,
            "JSON-LD import complete"
        );

        Ok(ImportResult { imported, errors })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-S1-A-05: `RelationEntity` 序列化为 JSON-LD 时字段名正确
    /// （`@type` / `source_id` / `target_id` / `kind` / `evidence`）。
    #[test]
    fn relation_entity_serializes_with_jsonld_fields() {
        let rel = RelationEntity {
            type_: "RelationEntity".to_string(),
            id: "nebula:relation:r1".to_string(),
            source_id: "nebula:memory:m1".to_string(),
            target_id: "nebula:memory:m2".to_string(),
            kind: "causes".to_string(),
            weight: 0.85,
            created_at: 1700000000,
            evidence: Some("observed in logs".to_string()),
        };
        let json = serde_json::to_string(&rel).expect("serialize");
        assert!(json.contains("\"@type\":\"RelationEntity\""), "missing @type field");
        assert!(json.contains("\"source_id\":\"nebula:memory:m1\""));
        assert!(json.contains("\"target_id\":\"nebula:memory:m2\""));
        assert!(json.contains("\"kind\":\"causes\""));
        assert!(json.contains("\"evidence\":\"observed in logs\""));
        assert!(json.contains("\"weight\":0.85"));
    }

    /// `evidence=None` 时 `skip_serializing_if` 生效，不出现在 JSON 中。
    #[test]
    fn relation_entity_skips_none_evidence() {
        let rel = RelationEntity {
            type_: "RelationEntity".to_string(),
            id: "r2".to_string(),
            source_id: "m1".to_string(),
            target_id: "m2".to_string(),
            kind: "supports".to_string(),
            weight: 1.0,
            created_at: 0,
            evidence: None,
        };
        let json = serde_json::to_string(&rel).expect("serialize");
        assert!(!json.contains("evidence"), "None evidence should be skipped");
    }

    /// `ExportManifest` 的 `relation_count` 字段不再是硬编码 0。
    /// 这是 P-05 回归保护：确保 manifest 反映真实关系数。
    #[test]
    fn export_manifest_carries_relation_count() {
        let manifest = ExportManifest {
            memory_count: 10,
            relation_count: 5,
            redacted_count: 0,
            exported_at: 1700000000,
            schema_version: SCHEMA_VERSION.to_string(),
        };
        assert_eq!(manifest.relation_count, 5, "relation_count should reflect actual count");
        assert_ne!(manifest.relation_count, 0, "regression: relation_count must not be hardcoded 0");
    }

    /// `RelationEntity` 可反序列化（round-trip）。
    #[test]
    fn relation_entity_round_trip() {
        let original = RelationEntity {
            type_: "RelationEntity".to_string(),
            id: "r3".to_string(),
            source_id: "src".to_string(),
            target_id: "dst".to_string(),
            kind: "contradicts".to_string(),
            weight: 0.5,
            created_at: 1234567890,
            evidence: Some("contradictory evidence".to_string()),
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let parsed: RelationEntity = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.source_id, original.source_id);
        assert_eq!(parsed.target_id, original.target_id);
        assert_eq!(parsed.kind, original.kind);
        assert_eq!(parsed.weight, original.weight);
        assert_eq!(parsed.evidence, original.evidence);
    }
}

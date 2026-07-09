//! Hybrid search: BM25 keyword search + vector similarity search.
//!
//! T-E-B-11: combines the strengths of both retrieval methods:
//! - BM25 (via FTS5) excels at exact keyword matching — function names,
//!   file paths, specific identifiers where dense embeddings dilute
//!   the signal.
//! - Vector search (via LanceDB) excels at semantic similarity —
//!   paraphrased queries, conceptual lookups.
//!
//! The two retrievers run in parallel (`tokio::join!`), their scores
//! are normalised to `[0, 1]` via min-max scaling, then fused:
//!
//! ```text
//! final_score = alpha * vector_score + (1 - alpha) * bm25_score
//! ```
//!
//! Default `alpha = 0.6` (vector-leaning). Results are deduplicated
//! by `memory_id` (keeping the fused score) and truncated to `limit`.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tracing::debug;

use super::bm25::Bm25Searcher;
use super::embedder::Embedder;
use super::sqlite_store::SqliteStore;
use super::types::Memory;
// T-E-S-42: HybridSearcher 面向 VectorStore trait 编程,可接受任意后端。
use super::vector_store::VectorStore;

/// Configuration for hybrid search.
#[derive(Debug, Clone)]
pub struct HybridSearchConfig {
    /// Weight of the vector score in the fused result. Must be in
    /// `[0.0, 1.0]`. `alpha = 1.0` → pure vector search;
    /// `alpha = 0.0` → pure BM25. Default: `0.6` (vector-leaning).
    pub alpha: f64,
    /// Over-fetch factor: each retriever pulls `limit * over_fetch`
    /// candidates so the fusion has a larger pool to deduplicate and
    /// re-rank from before truncating to `limit`.
    pub over_fetch: usize,
}

impl Default for HybridSearchConfig {
    fn default() -> Self {
        Self {
            alpha: 0.6,
            over_fetch: 3,
        }
    }
}

/// Hybrid searcher combining BM25 and vector search.
///
/// Holds references to the SQLite store (for BM25 + memory hydration),
/// the Lance vector store, and the embedder (to produce query vectors).
pub struct HybridSearcher {
    bm25: Bm25Searcher,
    lance: Arc<dyn VectorStore>,
    embedder: Arc<Embedder>,
    sqlite: Arc<SqliteStore>,
    config: HybridSearchConfig,
}

impl HybridSearcher {
    /// Creates a new hybrid searcher from the shared store handles.
    pub fn new(
        sqlite: Arc<SqliteStore>,
        lance: Arc<dyn VectorStore>,
        embedder: Arc<Embedder>,
    ) -> Self {
        let bm25 = Bm25Searcher::new(&sqlite);
        Self {
            bm25,
            lance,
            embedder,
            sqlite,
            config: HybridSearchConfig::default(),
        }
    }

    /// Replaces the default config with a custom one.
    pub fn with_config(mut self, config: HybridSearchConfig) -> Self {
        self.config = config;
        self
    }

    /// Runs a hybrid search and returns `(Memory, fused_score)` pairs
    /// sorted by descending score.
    ///
    /// Pipeline:
    /// 1. Embed the query (async) while BM25 runs in parallel.
    /// 2. Run vector search with the embedding.
    /// 3. Normalise both score lists to `[0, 1]`.
    /// 4. Fuse: `alpha * vector + (1 - alpha) * bm25`.
    /// 5. Deduplicate by `memory_id`.
    /// 6. Sort by fused score descending, truncate to `limit`.
    /// 7. Hydrate the top-N ids into full [`Memory`] entries via
    ///    [`SqliteStore::get_many`].
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<(Memory, f64)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let pool = limit.saturating_mul(self.config.over_fetch).max(limit);
        let alpha = self.config.alpha.clamp(0.0, 1.0);

        // 1 + 2: run BM25 in parallel with embedding, then vector search.
        let bm25_fut = self.bm25.search(query, pool);
        let embed_fut = self.embedder.embed(query);
        let (bm25_hits, vec_emb) = tokio::join!(bm25_fut, embed_fut);
        let vec_emb = vec_emb?;
        let vec_hits = self.lance.search(&vec_emb, pool).await;

        // 3: normalise to [0, 1].
        let bm25_scores: HashMap<String, f64> = match bm25_hits {
            Ok(hits) => normalise_scores(hits),
            Err(e) => {
                debug!(target: "nebula.memory", error = %e, "bm25 search failed; vector-only fusion");
                HashMap::new()
            }
        };
        let vec_scores: HashMap<String, f64> = match vec_hits {
            Ok(hits) => normalise_scores(hits.into_iter().map(|(id, s)| (id, s as f64)).collect()),
            Err(e) => {
                debug!(target: "nebula.memory", error = %e, "vector search failed; bm25-only fusion");
                HashMap::new()
            }
        };

        // 4 + 5: fuse and deduplicate.
        let mut fused: HashMap<String, f64> = HashMap::new();
        for (id, &v) in &vec_scores {
            let b = bm25_scores.get(id).copied().unwrap_or(0.0);
            fused.insert(id.clone(), alpha * v + (1.0 - alpha) * b);
        }
        for (id, &b) in &bm25_scores {
            if !fused.contains_key(id) {
                let v = vec_scores.get(id).copied().unwrap_or(0.0);
                fused.insert(id.clone(), alpha * v + (1.0 - alpha) * b);
            }
        }

        // 6: sort by descending fused score, truncate to limit.
        let mut ranked: Vec<(String, f64)> = fused.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(limit);

        if ranked.is_empty() {
            return Ok(Vec::new());
        }

        // 7: hydrate ids into full Memory entries.
        let ids: Vec<String> = ranked.iter().map(|(id, _)| id.clone()).collect();
        let memories = self.sqlite.get_many(&ids).await.unwrap_or_default();
        let mem_map: HashMap<&str, &Memory> = memories.iter().map(|m| (m.id.as_str(), m)).collect();

        let mut out: Vec<(Memory, f64)> = Vec::with_capacity(ranked.len());
        for (id, score) in ranked {
            if let Some(m) = mem_map.get(id.as_str()) {
                out.push(((*m).clone(), score));
            }
        }

        Ok(out)
    }
}

/// Normalises a list of scores to `[0, 1]` via min-max scaling.
///
/// If all scores are equal (or there is only one entry), every score
/// is mapped to `1.0` (the best possible) to avoid division by zero.
/// Negative inputs are handled correctly because min-max scaling
/// shifts the minimum to `0.0`.
fn normalise_scores(hits: Vec<(String, f64)>) -> HashMap<String, f64> {
    if hits.is_empty() {
        return HashMap::new();
    }
    let min = hits.iter().map(|(_, s)| *s).fold(f64::INFINITY, f64::min);
    let max = hits
        .iter()
        .map(|(_, s)| *s)
        .fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;
    hits.into_iter()
        .map(|(id, s)| {
            let norm = if range > 1e-12 {
                (s - min) / range
            } else {
                1.0
            };
            (id, norm)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    // T-E-S-42: 测试中直接构造 LanceStore 作为 VectorStore trait 对象。
    use crate::memory::lance_store::LanceStore;

    // ---- Vector search component test ----

    /// Vector search via LanceStore returns the nearest vectors sorted
    /// by descending cosine similarity. This exercises the vector
    /// retriever that feeds into hybrid fusion.
    #[tokio::test]
    async fn vector_search_returns_nearest_first() {
        let path = std::env::temp_dir().join(format!("nebula_hybrid_vec_{}", uuid::Uuid::new_v4()));
        let store = LanceStore::open(&path, 4)
            .await
            .expect("create should succeed");
        store
            .upsert("a", &[1.0, 0.0, 0.0, 0.0])
            .await
            .expect("update should succeed");
        store
            .upsert("b", &[0.0, 1.0, 0.0, 0.0])
            .await
            .expect("update should succeed");
        store
            .upsert("c", &[0.9, 0.1, 0.0, 0.0])
            .await
            .expect("update should succeed");

        let hits = store
            .search(&[1.0, 0.0, 0.0, 0.0], 3)
            .await
            .expect("query should succeed");
        assert_eq!(hits.len(), 3);
        // "a" is an exact match → should be top or tied with "c".
        let top_ids: std::collections::HashSet<&str> =
            hits.iter().take(2).map(|(id, _)| id.as_str()).collect();
        assert!(top_ids.contains("a"), "expected 'a' in top-2: {hits:?}");
        // Scores are descending.
        for w in hits.windows(2) {
            assert!(
                w[0].1 >= w[1].1,
                "vector scores should be descending: {} vs {}",
                w[0].1,
                w[1].1
            );
        }
        let _ = std::fs::remove_dir_all(path);
    }

    // ---- Score normalisation unit test ----

    /// `normalise_scores` scales the input to `[0, 1]` with the
    /// minimum mapped to `0.0` and the maximum to `1.0`.
    #[test]
    fn normalise_scores_scales_to_unit_range() {
        let hits = vec![
            ("a".to_string(), 0.0),
            ("b".to_string(), 5.0),
            ("c".to_string(), 10.0),
        ];
        let norm = normalise_scores(hits);
        assert!((norm["a"] - 0.0).abs() < 1e-9);
        assert!((norm["b"] - 0.5).abs() < 1e-9);
        assert!((norm["c"] - 1.0).abs() < 1e-9);
    }

    /// When all scores are equal, `normalise_scores` maps every entry
    /// to `1.0` (avoids division by zero).
    #[test]
    fn normalise_scores_handles_uniform_input() {
        let hits = vec![("a".to_string(), 3.5), ("b".to_string(), 3.5)];
        let norm = normalise_scores(hits);
        assert!((norm["a"] - 1.0).abs() < 1e-9);
        assert!((norm["b"] - 1.0).abs() < 1e-9);
    }

    /// `normalise_scores` handles negative values correctly (BM25
    /// scores after negation are non-negative, but the normaliser
    /// should still work for arbitrary inputs).
    #[test]
    fn normalise_scores_handles_negatives() {
        let hits = vec![
            ("a".to_string(), -5.0),
            ("b".to_string(), 0.0),
            ("c".to_string(), 5.0),
        ];
        let norm = normalise_scores(hits);
        assert!((norm["a"] - 0.0).abs() < 1e-9);
        assert!((norm["b"] - 0.5).abs() < 1e-9);
        assert!((norm["c"] - 1.0).abs() < 1e-9);
    }

    // ---- Hybrid deduplication unit test ----

    /// When a memory_id appears in both BM25 and vector results, the
    /// fusion step must deduplicate it (single entry) and assign the
    /// weighted fused score, not a sum.
    ///
    /// We verify this by simulating the fusion logic inline with a
    /// known overlap and checking the resulting score.
    #[test]
    fn hybrid_fusion_deduplicates_and_fuses_scores() {
        // Simulate normalised scores from both retrievers with one
        // overlapping id ("shared").
        let bm25_scores: HashMap<String, f64> = {
            let hits = vec![("shared".to_string(), 0.8), ("bm25-only".to_string(), 0.5)];
            normalise_scores(hits)
        };
        let vec_scores: HashMap<String, f64> = {
            let hits = vec![("shared".to_string(), 0.6), ("vec-only".to_string(), 0.9)];
            normalise_scores(hits)
        };

        let alpha = 0.6_f64;

        // Mirror the fusion logic from `HybridSearcher::search`.
        let mut fused: HashMap<String, f64> = HashMap::new();
        for (id, &v) in &vec_scores {
            let b = bm25_scores.get(id).copied().unwrap_or(0.0);
            fused.insert(id.clone(), alpha * v + (1.0 - alpha) * b);
        }
        for (id, &b) in &bm25_scores {
            if !fused.contains_key(id) {
                let v = vec_scores.get(id).copied().unwrap_or(0.0);
                fused.insert(id.clone(), alpha * v + (1.0 - alpha) * b);
            }
        }

        // Three unique ids (deduplication removed the duplicate "shared").
        assert_eq!(
            fused.len(),
            3,
            "expected 3 unique ids after dedup: {fused:?}"
        );
        assert!(fused.contains_key("shared"));
        assert!(fused.contains_key("bm25-only"));
        assert!(fused.contains_key("vec-only"));

        // "shared" has the fused score, not the sum.
        let expected_shared = alpha * vec_scores["shared"] + (1.0 - alpha) * bm25_scores["shared"];
        assert!(
            (fused["shared"] - expected_shared).abs() < 1e-9,
            "fused score for 'shared' should be {expected_shared}, got {}",
            fused["shared"]
        );

        // "bm25-only" has no vector contribution (v = 0).
        let expected_bm25_only = (1.0 - alpha) * bm25_scores["bm25-only"];
        assert!(
            (fused["bm25-only"] - expected_bm25_only).abs() < 1e-9,
            "fused score for 'bm25-only' should be {expected_bm25_only}, got {}",
            fused["bm25-only"]
        );

        // "vec-only" has no BM25 contribution (b = 0).
        let expected_vec_only = alpha * vec_scores["vec-only"];
        assert!(
            (fused["vec-only"] - expected_vec_only).abs() < 1e-9,
            "fused score for 'vec-only' should be {expected_vec_only}, got {}",
            fused["vec-only"]
        );

        // The ranked order should be: vec-only > shared > bm25-only.
        let mut ranked: Vec<(String, f64)> = fused.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        assert_eq!(ranked[0].0, "vec-only");
        assert_eq!(ranked[1].0, "shared");
        assert_eq!(ranked[2].0, "bm25-only");
    }

    /// Default config has `alpha = 0.6` and `over_fetch = 3`.
    #[test]
    fn hybrid_config_defaults() {
        let cfg = HybridSearchConfig::default();
        assert!((cfg.alpha - 0.6).abs() < 1e-9);
        assert_eq!(cfg.over_fetch, 3);
    }
}

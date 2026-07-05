//! BM25 keyword search via the SQLite FTS5 `memories_fts` virtual table.
//!
//! T-E-B-11: complements vector search (LanceDB) for keyword-exact
//! recall scenarios — searching for specific function names, file
//! paths, or identifiers where dense embeddings often dilute the
//! signal.
//!
//! The FTS5 index (`memories_fts`, migration 010) is kept in sync
//! with the `memories` table by triggers, so no application-level
//! index maintenance is needed. Scores from FTS5's `bm25()` function
//! are negative (more negative = better match); this searcher
//! negates them so callers can treat higher scores as better matches.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use rusqlite::{params, Connection};

use super::sqlite_store::SqliteStore;

/// BM25 searcher backed by the FTS5 `memories_fts` virtual table.
///
/// Holds a clone of the [`SqliteStore`] connection so it shares the
/// same database as the rest of the memory subsystem.
pub struct Bm25Searcher {
    conn: Arc<Mutex<Connection>>,
}

impl Bm25Searcher {
    /// Creates a new searcher sharing the SQLite connection owned by
    /// [`SqliteStore`].
    pub fn new(store: &SqliteStore) -> Self {
        Self {
            conn: store.raw_connection(),
        }
    }

    /// Runs a BM25 keyword search against the FTS5 index.
    ///
    /// Returns `(memory_id, score)` pairs sorted by descending score
    /// (best match first). Scores are the negated `bm25()` output, so
    /// they are non-negative and higher = better. Only non-compressed
    /// memories are returned (`compressed_from IS NULL`), matching the
    /// contract of [`SqliteStore::get_many`].
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<(String, f64)>> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn.clone();
        let fts_query = sanitize_fts_query(query);
        let limit_i = limit as i64;

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn
                .prepare(
                    "SELECT m.id, bm25(memories_fts) AS score \
                     FROM memories_fts \
                     JOIN memories m ON m.rowid = memories_fts.rowid \
                     WHERE memories_fts MATCH ?1 \
                       AND m.compressed_from IS NULL \
                     ORDER BY score ASC \
                     LIMIT ?2",
                )
                .map_err(|e| anyhow!("bm25 prepare error: {e}"))?;
            let rows = stmt
                .query_map(params![fts_query, limit_i], |row| {
                    let id: String = row.get(0)?;
                    let raw: f64 = row.get(1)?;
                    // FTS5 bm25() returns negative values (lower = better).
                    // Negate so higher = better, ready for normalisation.
                    Ok((id, -raw))
                })
                .map_err(|e| anyhow!("bm25 query error: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow!("bm25 row error: {e}"))?;
            Ok(rows)
        })
        .await
        .map_err(|e| anyhow!("spawn_blocking join error: {e}"))?
    }
}

/// Sanitises a user query for the FTS5 `MATCH` operator.
///
/// FTS5 query syntax treats characters like `"`, `*`, `(`, `)`, `:`
/// as special. To provide safe keyword matching we split on
/// whitespace, escape internal double quotes, and wrap each token in
/// double quotes. Tokens are joined with spaces (FTS5 implicit AND),
/// so `rust async runtime` becomes `"rust" "async" "runtime"` which
/// matches documents containing all three terms.
fn sanitize_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|tok| {
            let escaped = tok.replace('"', "\"\"");
            format!("\"{escaped}\"")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::{Memory, MemoryLayer, MemoryType, MultiGranularity, SourceKind};
    use std::env;

    fn temp_db_path() -> std::path::PathBuf {
        let mut p = env::temp_dir();
        p.push(format!("nebula_bm25_test_{}.db", uuid::Uuid::new_v4()));
        p
    }

    fn make_memory(id: &str, content: &str) -> Memory {
        let mut m = Memory::new(
            MemoryType::Semantic,
            MemoryLayer::L3,
            content,
            SourceKind::UserInput,
        );
        m.id = id.to_string();
        m.summary = MultiGranularity::new(
            content,
            content,
            content,
            content,
        );
        m
    }

    /// BM25 search returns memories whose content matches the query
    /// keyword, ranked by relevance.
    #[tokio::test]
    async fn bm25_search_returns_relevant_memories() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();

        // Insert three memories with distinct content.
        let m1 = make_memory("bm25-fox", "the quick brown fox jumps over the lazy dog");
        let m2 = make_memory("bm25-rust", "rust is a systems programming language focused on safety");
        let m3 = make_memory("bm25-mixed", "fox in rust the lazy dog programming");
        store.insert(&m1).await.unwrap();
        store.insert(&m2).await.unwrap();
        store.insert(&m3).await.unwrap();

        let searcher = Bm25Searcher::new(&store);
        let hits = searcher.search("fox", 10).await.unwrap();

        // Both m1 and m3 contain "fox".
        let ids: Vec<&str> = hits.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"bm25-fox"), "expected bm25-fox in results: {hits:?}");
        assert!(ids.contains(&"bm25-mixed"), "expected bm25-mixed in results: {hits:?}");
        // m2 does not contain "fox".
        assert!(!ids.contains(&"bm25-rust"), "bm25-rust should not match 'fox'");

        // Scores are positive (negated bm25 output) and sorted desc.
        for (_, score) in &hits {
            assert!(*score >= 0.0, "score should be non-negative: {score}");
        }
        for w in hits.windows(2) {
            assert!(
                w[0].1 >= w[1].1,
                "scores should be descending: {} vs {}",
                w[0].1,
                w[1].1
            );
        }

        let _ = std::fs::remove_file(path);
    }

    /// BM25 search respects the `limit` parameter.
    #[tokio::test]
    async fn bm25_search_respects_limit() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();

        for i in 0..5 {
            let m = make_memory(&format!("bm25-lim-{i}"), "alpha beta gamma delta");
            store.insert(&m).await.unwrap();
        }

        let searcher = Bm25Searcher::new(&store);
        let hits = searcher.search("alpha", 2).await.unwrap();
        assert_eq!(hits.len(), 2, "limit=2 should return 2 hits");

        let _ = std::fs::remove_file(path);
    }

    /// BM25 search returns an empty vector for empty queries.
    #[tokio::test]
    async fn bm25_search_empty_query_returns_empty() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();
        let m = make_memory("bm25-empty", "some content here");
        store.insert(&m).await.unwrap();

        let searcher = Bm25Searcher::new(&store);
        assert!(searcher.search("", 10).await.unwrap().is_empty());
        assert!(searcher.search("   ", 10).await.unwrap().is_empty());

        let _ = std::fs::remove_file(path);
    }

    /// BM25 search excludes compressed memories.
    #[tokio::test]
    async fn bm25_search_excludes_compressed() {
        let path = temp_db_path();
        let store = SqliteStore::open(&path).unwrap();

        let m_alive = make_memory("bm25-alive", "searchable keyword zeta");
        let m_compressed = make_memory("bm25-compressed", "searchable keyword zeta");
        store.insert(&m_alive).await.unwrap();
        store.insert(&m_compressed).await.unwrap();
        store
            .update_compressed_from("bm25-compressed", "summary-x")
            .await
            .unwrap();

        let searcher = Bm25Searcher::new(&store);
        let hits = searcher.search("zeta", 10).await.unwrap();
        let ids: Vec<&str> = hits.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"bm25-alive"));
        assert!(!ids.contains(&"bm25-compressed"));

        let _ = std::fs::remove_file(path);
    }

    /// `sanitize_fts_query` wraps each token in quotes and joins with
    /// spaces (implicit AND).
    #[test]
    fn sanitize_wraps_tokens_in_quotes() {
        assert_eq!(sanitize_fts_query("fox"), "\"fox\"");
        assert_eq!(sanitize_fts_query("quick fox"), "\"quick\" \"fox\"");
        // Internal double quotes are escaped by doubling.
        assert_eq!(sanitize_fts_query("a\"b"), "\"a\"\"b\"");
        // Whitespace-only input yields empty string.
        assert_eq!(sanitize_fts_query("   "), "");
    }
}

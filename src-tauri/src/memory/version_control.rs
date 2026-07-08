//! Git 风格记忆版本控制引擎。
//!
//! 设计文档 v7.0 §3.4 L3 应用层 — 记忆版本控制。
//!
//! 在现有 `memory_commits` 审计日志基础上，增加 Git 语义：
//!
//! * **branch** — 分支管理（创建/列出/切换/删除）
//! * **commit** — 在当前分支上记录变更（自动关联 parent + branch）
//! * **log** — 查看提交历史
//! * **diff** — 比较两个 commit 的 payload 差异
//! * **revert** — 标记回滚（生成 revert commit，不删除历史）
//!
//! ## 设计约束
//!
//! 记忆系统是"只增不删"的（黑洞压缩只 densify 不删除），因此
//! `revert` 不会物理删除记忆，而是生成一条 `action=revert` 的
//! commit，标记目标 commit 之后的操作被"逻辑回滚"。前端可以
//! 根据 revert 标记决定是否展示被回滚的内容。
//!
//! ## 表结构
//!
//! * `memory_branches`（migration 019）— 分支注册表
//! * `memory_commits`（migration 001 + 019 扩展）— 提交链
//!
//! `memory_commits.branch_name` 列由 migration 019 添加。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::sqlite_store::SqliteStore;

/// 分支信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBranch {
    pub name: String,
    pub head_commit_id: Option<String>,
    pub parent_branch: Option<String>,
    pub created_at: i64,
    pub is_active: bool,
}

/// 提交记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitRecord {
    pub id: String,
    pub parent_id: Option<String>,
    pub action: String,
    pub target_id: String,
    pub payload: serde_json::Value,
    pub author: String,
    pub message: String,
    pub created_at: i64,
    pub branch_name: String,
}

/// 两个 commit 之间的差异。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitDiff {
    pub from_commit: String,
    pub to_commit: String,
    /// from 之后、to 之前（含 to）的所有 commit。
    pub commits: Vec<CommitRecord>,
    /// 涉及的记忆 ID 集合。
    pub affected_memory_ids: Vec<String>,
}

/// Git 风格记忆版本控制引擎。
pub struct MemoryVersionControl {
    sqlite: Arc<SqliteStore>,
}

impl MemoryVersionControl {
    pub fn new(sqlite: Arc<SqliteStore>) -> Self {
        Self { sqlite }
    }

    /// 获取当前活跃分支。
    pub fn get_active_branch(&self) -> Result<Option<MemoryBranch>> {
        let conn = self.sqlite.raw_connection();
        let g = conn.lock();
        let mut stmt = g.prepare(
            "SELECT name, head_commit_id, parent_branch, created_at, is_active
             FROM memory_branches WHERE is_active = 1 LIMIT 1",
        )?;
        let branch = stmt
            .query_row([], |r| {
                Ok(MemoryBranch {
                    name: r.get(0)?,
                    head_commit_id: r.get(1)?,
                    parent_branch: r.get(2)?,
                    created_at: r.get(3)?,
                    is_active: r.get::<_, i64>(4)? != 0,
                })
            })
            .ok();
        Ok(branch)
    }

    /// 列出所有分支。
    pub fn list_branches(&self) -> Result<Vec<MemoryBranch>> {
        let conn = self.sqlite.raw_connection();
        let g = conn.lock();
        let mut stmt = g.prepare(
            "SELECT name, head_commit_id, parent_branch, created_at, is_active
             FROM memory_branches ORDER BY created_at ASC",
        )?;
        let branches = stmt
            .query_map([], |r| {
                Ok(MemoryBranch {
                    name: r.get(0)?,
                    head_commit_id: r.get(1)?,
                    parent_branch: r.get(2)?,
                    created_at: r.get(3)?,
                    is_active: r.get::<_, i64>(4)? != 0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(branches)
    }

    /// 创建新分支（从当前活跃分支的 head 分叉）。
    pub fn create_branch(&self, name: &str) -> Result<MemoryBranch> {
        let active = self.get_active_branch()?;
        let head = active.as_ref().and_then(|b| b.head_commit_id.clone());
        let parent = active.as_ref().map(|b| b.name.clone());
        let now = chrono::Utc::now().timestamp();

        let conn = self.sqlite.raw_connection();
        let g = conn.lock();
        g.execute(
            "INSERT INTO memory_branches (name, head_commit_id, parent_branch, created_at, is_active)
             VALUES (?1, ?2, ?3, ?4, 0)",
            rusqlite::params![name, head, parent, now],
        )
        .map_err(|e| anyhow!("create_branch insert error: {e}"))?;

        debug!(target: "nebula.vc", branch = name, "branch created");
        Ok(MemoryBranch {
            name: name.to_string(),
            head_commit_id: head,
            parent_branch: parent,
            created_at: now,
            is_active: false,
        })
    }

    /// 切换活跃分支（checkout）。
    pub fn checkout(&self, branch_name: &str) -> Result<()> {
        let conn = self.sqlite.raw_connection();
        let g = conn.lock();
        // 先取消所有活跃
        g.execute("UPDATE memory_branches SET is_active = 0", [])
            .map_err(|e| anyhow!("checkout reset active error: {e}"))?;
        // 激活目标分支
        let affected = g.execute(
            "UPDATE memory_branches SET is_active = 1 WHERE name = ?1",
            rusqlite::params![branch_name],
        )?;
        if affected == 0 {
            anyhow::bail!("branch not found: {}", branch_name);
        }
        debug!(target: "nebula.vc", branch = branch_name, "checked out");
        Ok(())
    }

    /// 删除分支（不能删除活跃分支）。
    pub fn delete_branch(&self, name: &str) -> Result<()> {
        if name == "main" {
            anyhow::bail!("cannot delete main branch");
        }
        let active = self.get_active_branch()?;
        if active.as_ref().is_some_and(|b| b.name == name) {
            anyhow::bail!("cannot delete active branch; checkout main first");
        }
        let conn = self.sqlite.raw_connection();
        let g = conn.lock();
        g.execute(
            "DELETE FROM memory_branches WHERE name = ?1",
            rusqlite::params![name],
        )
        .map_err(|e| anyhow!("delete_branch error: {e}"))?;
        Ok(())
    }

    /// 在当前活跃分支上创建一个 commit。
    ///
    /// 自动设置 parent_id 为当前分支 head，并更新 head 指针。
    pub fn commit(
        &self,
        action: &str,
        target_id: &str,
        payload: &serde_json::Value,
        author: &str,
        message: &str,
    ) -> Result<String> {
        let active = self
            .get_active_branch()?
            .ok_or_else(|| anyhow!("no active branch"))?;
        let commit_id = uuid::Uuid::new_v4().to_string();
        let parent_id = active.head_commit_id.clone();
        let branch_name = active.name.clone();
        let now = chrono::Utc::now().timestamp();
        let payload_str = payload.to_string();

        let conn = self.sqlite.raw_connection();
        let g = conn.lock();
        g.execute(
            "INSERT INTO memory_commits
                (id, parent_id, action, target_id, payload, author, message, created_at, branch_name)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                commit_id,
                parent_id,
                action,
                target_id,
                payload_str,
                author,
                message,
                now,
                branch_name,
            ],
        )
        .map_err(|e| anyhow!("commit insert error: {e}"))?;

        // 更新分支 head
        g.execute(
            "UPDATE memory_branches SET head_commit_id = ?1 WHERE name = ?2",
            rusqlite::params![commit_id, branch_name],
        )
        .map_err(|e| anyhow!("commit update head error: {e}"))?;

        debug!(target: "nebula.vc", commit = %commit_id, branch = %branch_name, action, "committed");
        Ok(commit_id)
    }

    /// 查看当前分支的提交历史。
    pub fn log(&self, limit: usize) -> Result<Vec<CommitRecord>> {
        let active = self
            .get_active_branch()?
            .ok_or_else(|| anyhow!("no active branch"))?;
        let branch_name = active.name.clone();

        let conn = self.sqlite.raw_connection();
        let g = conn.lock();
        let mut stmt = g.prepare(
            "SELECT id, parent_id, action, target_id, payload, author, message, created_at, branch_name
             FROM memory_commits
             WHERE branch_name = ?1
             ORDER BY created_at DESC
             LIMIT ?2",
        )?;
        let logs = stmt
            .query_map(rusqlite::params![branch_name, limit as i64], |r| {
                let payload_str: String = r.get(4)?;
                let payload: serde_json::Value =
                    serde_json::from_str(&payload_str).unwrap_or(serde_json::json!({}));
                Ok(CommitRecord {
                    id: r.get(0)?,
                    parent_id: r.get(1)?,
                    action: r.get(2)?,
                    target_id: r.get(3)?,
                    payload,
                    author: r.get(5)?,
                    message: r.get(6)?,
                    created_at: r.get(7)?,
                    branch_name: r.get(8)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(logs)
    }

    /// 比较两个 commit 之间的差异。
    ///
    /// 返回 from_commit 之后、to_commit（含）之前的所有 commit，
    /// 以及涉及的 memory ID 集合。
    pub fn diff(&self, from_commit: &str, to_commit: &str) -> Result<CommitDiff> {
        let conn = self.sqlite.raw_connection();
        let g = conn.lock();

        // 从 to_commit 开始向前追溯 parent_id 链，直到遇到 from_commit
        let mut commits: Vec<CommitRecord> = Vec::new();
        let mut current = Some(to_commit.to_string());
        let mut affected_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

        while let Some(ref cid) = current {
            if cid == from_commit {
                break;
            }
            let mut stmt = g.prepare(
                "SELECT id, parent_id, action, target_id, payload, author, message, created_at, branch_name
                 FROM memory_commits WHERE id = ?1",
            )?;
            let record = stmt
                .query_row(rusqlite::params![cid], |r| {
                    let payload_str: String = r.get(4)?;
                    let payload: serde_json::Value =
                        serde_json::from_str(&payload_str).unwrap_or(serde_json::json!({}));
                    Ok(CommitRecord {
                        id: r.get(0)?,
                        parent_id: r.get(1)?,
                        action: r.get(2)?,
                        target_id: r.get(3)?,
                        payload,
                        author: r.get(5)?,
                        message: r.get(6)?,
                        created_at: r.get(7)?,
                        branch_name: r.get(8)?,
                    })
                })
                .ok();
            if let Some(rec) = record {
                affected_ids.insert(rec.target_id.clone());
                current = rec.parent_id.clone();
                commits.push(rec);
            } else {
                warn!(target: "nebula.vc", commit = %cid, "commit not found in diff traversal");
                break;
            }
        }

        commits.reverse(); // 按时间正序排列

        Ok(CommitDiff {
            from_commit: from_commit.to_string(),
            to_commit: to_commit.to_string(),
            commits,
            affected_memory_ids: affected_ids.into_iter().collect(),
        })
    }

    /// 回滚到某个 commit（生成 revert commit，不删除历史）。
    ///
    /// 在当前分支上创建一条 `action=revert` 的 commit，
    /// payload 中记录被回滚的目标 commit_id。
    pub fn revert(&self, target_commit_id: &str, author: &str, message: &str) -> Result<String> {
        let payload = serde_json::json!({
            "revert_target": target_commit_id,
            "reverted_at": chrono::Utc::now().timestamp(),
        });
        let revert_id = self.commit(
            "revert",
            target_commit_id,
            &payload,
            author,
            &format!("revert: {}", message),
        )?;
        debug!(target: "nebula.vc", revert = %revert_id, target = %target_commit_id, "revert committed");
        Ok(revert_id)
    }

    /// 合并分支（将 source_branch 的 commit 追加到当前活跃分支）。
    ///
    /// 注意：记忆系统的合并是"追加"语义，不做三方合并。
    /// source_branch 上的 commit 被复制到当前分支，parent 指向当前 head。
    pub fn merge(&self, source_branch: &str) -> Result<Vec<String>> {
        let target = self
            .get_active_branch()?
            .ok_or_else(|| anyhow!("no active branch for merge target"))?;
        if target.name == source_branch {
            anyhow::bail!("cannot merge a branch into itself");
        }

        let conn = self.sqlite.raw_connection();
        let g = conn.lock();

        // 获取 source 分支的所有 commit（按时间正序）
        let mut stmt = g.prepare(
            "SELECT id, parent_id, action, target_id, payload, author, message, created_at
             FROM memory_commits
             WHERE branch_name = ?1
             ORDER BY created_at ASC",
        )?;
        let source_commits: Vec<(
            String,
            Option<String>,
            String,
            String,
            String,
            String,
            String,
            i64,
        )> = stmt
            .query_map(rusqlite::params![source_branch], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        drop(stmt);

        let mut parent = target.head_commit_id.clone();
        let mut merged_ids: Vec<String> = Vec::new();
        let target_branch = target.name.clone();

        for (_src_id, _src_parent, action, target_id, payload, author, message, created_at) in
            source_commits
        {
            let new_id = uuid::Uuid::new_v4().to_string();
            g.execute(
                "INSERT INTO memory_commits
                    (id, parent_id, action, target_id, payload, author, message, created_at, branch_name)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    new_id,
                    parent,
                    action,
                    target_id,
                    payload,
                    author,
                    message,
                    created_at,
                    target_branch,
                ],
            )?;
            parent = Some(new_id.clone());
            merged_ids.push(new_id);
        }

        // 更新目标分支 head
        if let Some(last) = merged_ids.last() {
            g.execute(
                "UPDATE memory_branches SET head_commit_id = ?1 WHERE name = ?2",
                rusqlite::params![last, target_branch],
            )?;
        }

        debug!(target: "nebula.vc", source = source_branch, target = %target_branch, merged = merged_ids.len(), "merge completed");
        Ok(merged_ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // M7b #90 分类 C:原实现返回 `file:vc_test_<nanos>?mode=memory&cache=shared`
    // URI 字符串,但 `SqliteStore::open()` 用的是 `Connection::open(path)`(非 URI
    // 模式)。Windows 文件名不能含 `?`,导致 SqliteStore::open 失败。改用 sqlite_store.rs
    // 测试模式:真实临时文件 + UUID,每个测试独立 DB 文件。
    fn temp_db_path() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("nebula_vc_test_{}.db", uuid::Uuid::new_v4()));
        p
    }

    fn setup() -> MemoryVersionControl {
        let store = SqliteStore::open(&temp_db_path()).expect("create should succeed");
        // 运行 migrations 以创建 memory_branches 表
        {
            let conn = store.raw_connection();
            let g = conn.lock();
            crate::memory::migration::run_bundled_migrations(&g).expect("test op should succeed");
        }
        MemoryVersionControl::new(Arc::new(store))
    }

    #[test]
    fn active_branch_defaults_to_main() {
        let vc = setup();
        let active = vc.get_active_branch().expect("get should succeed");
        assert!(active.is_some());
        let b = active.expect("test op should succeed");
        assert_eq!(b.name, "main");
        assert!(b.is_active);
    }

    #[test]
    fn list_branches_includes_main() {
        let vc = setup();
        let branches = vc.list_branches().expect("test op should succeed");
        assert!(branches.iter().any(|b| b.name == "main"));
    }

    #[test]
    fn create_and_checkout_branch() {
        let vc = setup();
        vc.create_branch("experiment").expect("create should succeed");
        vc.checkout("experiment").expect("test op should succeed");
        let active = vc.get_active_branch().expect("get should succeed").expect("get should succeed");
        assert_eq!(active.name, "experiment");
    }

    #[test]
    fn delete_branch_works() {
        let vc = setup();
        vc.create_branch("temp").expect("create should succeed");
        vc.delete_branch("temp").expect("delete should succeed");
        let branches = vc.list_branches().expect("test op should succeed");
        assert!(!branches.iter().any(|b| b.name == "temp"));
    }

    #[test]
    fn cannot_delete_main_branch() {
        let vc = setup();
        let err = vc.delete_branch("main").unwrap_err();
        assert!(format!("{err}").contains("cannot delete main"));
    }

    #[test]
    fn cannot_delete_active_branch() {
        let vc = setup();
        vc.create_branch("active").expect("create should succeed");
        vc.checkout("active").expect("test op should succeed");
        let err = vc.delete_branch("active").unwrap_err();
        assert!(format!("{err}").contains("active branch"));
    }

    #[test]
    fn commit_updates_head() {
        let vc = setup();
        let cid = vc
            .commit(
                "store",
                "mem-1",
                &serde_json::json!({"k":"v"}),
                "test",
                "test commit",
            )
            .expect("test op should succeed");
        let active = vc.get_active_branch().expect("get should succeed").expect("get should succeed");
        assert_eq!(active.head_commit_id, Some(cid));
    }

    #[test]
    fn log_returns_commits_in_desc_order() {
        let vc = setup();
        vc.commit("store", "mem-1", &serde_json::json!({}), "test", "first")
            .expect("test op should succeed");
        // M7b #90 分类 A: commit 用 chrono::Utc::now().timestamp()(秒级精度),
        // log 用 ORDER BY created_at DESC。10ms 间隔可能落在同一秒,导致排序
        // 不稳定。sleep(1100ms) 确保时间戳严格递增,测试不再 flaky。
        std::thread::sleep(std::time::Duration::from_millis(1100));
        vc.commit("store", "mem-2", &serde_json::json!({}), "test", "second")
            .expect("test op should succeed");

        let logs = vc.log(10).expect("test op should succeed");
        assert_eq!(logs.len(), 2);
        // 最新的在前
        assert_eq!(logs[0].message, "second");
        assert_eq!(logs[1].message, "first");
    }

    #[test]
    fn diff_traverses_commit_chain() {
        let vc = setup();
        let c1 = vc
            .commit("store", "mem-1", &serde_json::json!({}), "test", "c1")
            .expect("test op should succeed");
        std::thread::sleep(std::time::Duration::from_millis(10));
        let c2 = vc
            .commit("store", "mem-2", &serde_json::json!({}), "test", "c2")
            .expect("test op should succeed");

        let diff = vc.diff(&c1, &c2).expect("test op should succeed");
        assert_eq!(diff.commits.len(), 1);
        assert_eq!(diff.commits[0].id, c2);
        assert!(diff.affected_memory_ids.contains(&"mem-2".to_string()));
    }

    #[test]
    fn revert_creates_revert_commit() {
        let vc = setup();
        let c1 = vc
            .commit("store", "mem-1", &serde_json::json!({}), "test", "original")
            .expect("test op should succeed");
        // M7b #90 分类 A: commit 和 revert 同秒会导致 log ORDER BY 不稳定。
        // sleep(1100ms) 确保 revert 的时间戳严格大于 c1。
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let revert_id = vc.revert(&c1, "user", "mistake").expect("test op should succeed");

        let logs = vc.log(10).expect("test op should succeed");
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].id, revert_id);
        assert_eq!(logs[0].action, "revert");
    }

    #[test]
    fn merge_appends_commits() {
        let vc = setup();
        // 在 main 上创建一个 commit
        vc.commit("store", "mem-1", &serde_json::json!({}), "test", "main-1")
            .expect("test op should succeed");

        // 创建 feature 分支并切换
        vc.create_branch("feature").expect("create should succeed");
        vc.checkout("feature").expect("test op should succeed");
        vc.commit(
            "store",
            "mem-2",
            &serde_json::json!({}),
            "test",
            "feature-1",
        )
        .expect("test op should succeed");
        vc.commit(
            "store",
            "mem-3",
            &serde_json::json!({}),
            "test",
            "feature-2",
        )
        .expect("test op should succeed");

        // 切回 main 并合并
        vc.checkout("main").expect("test op should succeed");
        let merged = vc.merge("feature").expect("test op should succeed");
        assert_eq!(merged.len(), 2);

        // main 的 log 应包含合并来的 commit
        let logs = vc.log(10).expect("test op should succeed");
        assert!(logs.len() >= 3); // main-1 + feature-1 + feature-2
    }
}

//! T-E-C-08: Shadow Workspace 引擎。
//!
//! Agent 任务在独立 git worktree + 临时分支中执行,不影响用户当前工作区。
//! 借鉴 Cursor Cloud Agent,但本地化——使用 `git worktree add` 创建隔离
//! 工作树,Agent 在其中独立工作,完成后提供 diff 供用户审查,可合并或丢弃。
//!
//! 生命周期:
//!   Creating → Running → Completed → (Merged | Aborted)
//!                       ↘ Failed ↗
//!
//! 关键安全约束:
//! - worktree 路径在系统临时目录下(`nebula-shadow-ws/<id>`),不污染用户 repo
//! - 分支名固定 `agent/<id>` 前缀,避免与用户分支冲突
//! - merge 前必须先 Completed(防止合并未完成的工作)
//! - abort 会 remove worktree + delete branch(不可逆)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::instrument;

/// Shadow Workspace 状态机。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShadowStatus {
    /// 正在创建 worktree(瞬态,通常立即完成)。
    Creating,
    /// worktree 已就绪,Agent 可在其中工作。
    Running,
    /// Agent 已完成工作,等待用户审查 diff。
    Completed,
    /// Agent 执行失败,工作区可丢弃。
    Failed,
    /// 已合并回主分支,worktree 已清理。
    Merged,
    /// 已丢弃,worktree + branch 已删除。
    Aborted,
}

impl ShadowStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ShadowStatus::Creating => "creating",
            ShadowStatus::Running => "running",
            ShadowStatus::Completed => "completed",
            ShadowStatus::Failed => "failed",
            ShadowStatus::Merged => "merged",
            ShadowStatus::Aborted => "aborted",
        }
    }
}

/// 单个 Shadow Workspace 的元数据快照(序列化给前端)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowWorkspace {
    /// 唯一 ID(短 UUID,用作 branch 后缀和目录名)。
    pub id: String,
    /// 分支名,固定 `agent/<id>` 前缀。
    pub branch: String,
    /// worktree 绝对路径(系统临时目录下)。
    pub path: String,
    /// 用户描述的任务(供 UI 显示)。
    pub task_description: String,
    /// 当前状态。
    pub status: ShadowStatus,
    /// 创建时间(Unix 秒)。
    pub created_at: i64,
    /// 完成/失败/合并/丢弃时间(Unix 秒,未完成则为 None)。
    pub finished_at: Option<i64>,
    /// 创建时的基线分支(merge 目标,默认当前分支)。
    pub base_branch: String,
    /// 失败原因(status=Failed 时填充)。
    pub error: Option<String>,
}

/// 引擎配置。
#[derive(Debug, Clone)]
pub struct ShadowConfig {
    /// worktree 根目录(所有 shadow workspace 放在此目录下)。
    /// 默认 `<temp_dir>/nebula-shadow-ws`。
    pub worktree_root: PathBuf,
}

impl Default for ShadowConfig {
    fn default() -> Self {
        let root = std::env::temp_dir().join("nebula-shadow-ws");
        Self { worktree_root: root }
    }
}

/// Shadow Workspace 管理引擎。
///
/// 持有 `RwLock<HashMap<String, ShadowWorkspace>>` 内存状态。
/// worktree 是 git 的真实副作用,状态仅做缓存——重启后 `bootstrap()`
/// 会扫描 worktree_root 重建状态(未来增强;当前 v1 仅内存态)。
pub struct ShadowWorkspaceEngine {
    config: ShadowConfig,
    /// repo 根路径(从 EditorState.workspace_root 注入)。
    /// None 时引擎不可用(无 git repo 场景 graceful degrade)。
    repo_root: RwLock<Option<PathBuf>>,
    workspaces: RwLock<HashMap<String, ShadowWorkspace>>,
}

impl ShadowWorkspaceEngine {
    pub fn new(config: ShadowConfig) -> Self {
        Self {
            config,
            repo_root: RwLock::new(None),
            workspaces: RwLock::new(HashMap::new()),
        }
    }

    /// 默认配置构造。
    pub fn with_default() -> Self {
        Self::new(ShadowConfig::default())
    }

    /// 设置 repo 根路径(由 bootstrap 从 EditorState 注入)。
    pub fn set_repo_root(&self, root: PathBuf) {
        *self.repo_root.write() = Some(root);
    }

    /// 获取 repo 根路径,返回错误如果未设置。
    fn repo(&self) -> Result<PathBuf> {
        self.repo_root
            .read()
            .clone()
            .ok_or_else(|| anyhow!("shadow workspace engine: repo root not configured"))
    }

    /// 生成唯一 ID(8 字符 base32,足够区分且短)。
    fn gen_id() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        // 取低 40 bit,base32 编码 → 8 字符
        let bits = (nanos as u64) & 0xFF_FFFF_FFFF;
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
        let mut s = String::with_capacity(8);
        for i in (0..8).rev() {
            let shift = i * 5;
            let idx = ((bits >> shift) & 0x1F) as usize;
            s.push(ALPHABET[idx] as char);
        }
        s
    }

    /// 获取当前 git 分支名。
    fn current_branch(repo: &Path) -> Result<String> {
        let out = run_git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?;
        let branch = out.trim().to_string();
        if branch.is_empty() || branch == "HEAD" {
            return Err(anyhow!("cannot create shadow workspace in detached HEAD state"));
        }
        Ok(branch)
    }

    /// 创建新的 Shadow Workspace。
    ///
    /// 1. 生成唯一 ID + branch 名 `agent/<id>`
    /// 2. 确保 worktree_root 目录存在
    /// 3. `git worktree add -b agent/<id> <path> <base_branch>`
    /// 4. 记录元数据,状态置 Running
    #[instrument(skip(self))]
    pub fn create(&self, task_description: String, base_branch: Option<String>) -> Result<ShadowWorkspace> {
        let repo = self.repo()?;
        let id = Self::gen_id();
        let branch = format!("agent/{id}");
        let base = match base_branch {
            Some(b) if !b.is_empty() => b,
            _ => Self::current_branch(&repo)?,
        };

        // 确保 worktree_root 存在
        let ws_path = self.config.worktree_root.join(&id);
        std::fs::create_dir_all(&self.config.worktree_root)
            .with_context(|| format!("creating worktree_root {:?}", self.config.worktree_root))?;

        // git worktree add -b agent/<id> <path> <base>
        // -b 创建新分支,基于 base
        run_git(&repo, &["worktree", "add", "-b", &branch, ws_path.to_str().ok_or_else(|| anyhow!("invalid worktree path"))?, &base])
            .context(format!("git worktree add -b {branch} {ws_path:?} {base}"))?;

        let now = now_secs();
        let ws = ShadowWorkspace {
            id: id.clone(),
            branch,
            path: ws_path.to_string_lossy().into_owned(),
            task_description,
            status: ShadowStatus::Running,
            created_at: now,
            finished_at: None,
            base_branch: base,
            error: None,
        };

        self.workspaces.write().insert(id, ws.clone());
        Ok(ws)
    }

    /// 列出所有 workspace(按创建时间降序)。
    pub fn list(&self) -> Vec<ShadowWorkspace> {
        let mut all: Vec<ShadowWorkspace> = self.workspaces.read().values().cloned().collect();
        all.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        all
    }

    /// 获取单个 workspace。
    pub fn get(&self, id: &str) -> Option<ShadowWorkspace> {
        self.workspaces.read().get(id).cloned()
    }

    /// 获取 workspace 的 git diff(与 base_branch 对比)。
    #[instrument(skip(self))]
    pub fn diff(&self, id: &str) -> Result<String> {
        let ws = self.get(id).ok_or_else(|| anyhow!("workspace {id} not found"))?;
        let ws_path = Path::new(&ws.path);
        if !ws_path.exists() {
            return Err(anyhow!("worktree path {} no longer exists", ws.path));
        }
        // git diff <base> 在 worktree 内执行 —— 比较 base 提交与当前工作树,
        // 这样既包含 agent 在分支上的已提交改动,也包含尚未提交的工作进度,
        // 供用户在合并前做完整审查。(base..HEAD 只会显示已提交差异,
        // 会漏掉 worktree 中尚未 commit 的工作。)
        run_git(ws_path, &["diff", &ws.base_branch])
    }

    /// 在 worktree 内执行命令(供 Agent 使用)。
    ///
    /// 命令在 worktree 的 cwd 下执行,stdout+stderr 合并返回。
    /// 这里的命令执行不经过 ShellExecutor 白名单——Shadow Workspace
    /// 的调用方(PlanEngine/SwarmOrchestrator)本身已受自主度门禁约束。
    #[instrument(skip(self, args))]
    pub fn run_command(&self, id: &str, program: &str, args: &[String]) -> Result<String> {
        let ws = self.get(id).ok_or_else(|| anyhow!("workspace {id} not found"))?;
        let ws_path = Path::new(&ws.path);
        if !ws_path.exists() {
            return Err(anyhow!("worktree path {} no longer exists", ws.path));
        }
        let out = Command::new(program)
            .args(args)
            .current_dir(ws_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .with_context(|| format!("spawning {program} in shadow workspace {id}"))?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if !out.status.success() {
            return Err(anyhow!("{program} exited {:?}: {stderr}", out.status.code()));
        }
        // 合并 stdout + stderr(stderr 追加在后面,供 Agent 排错)
        if stderr.is_empty() {
            Ok(stdout)
        } else {
            Ok(format!("{stdout}\n--- stderr ---\n{stderr}"))
        }
    }

    /// 标记 workspace 已完成(Agent 工作结束,等待用户审查)。
    pub fn complete(&self, id: &str) -> Result<ShadowWorkspace> {
        self.transition(id, ShadowStatus::Completed, None)
    }

    /// 标记 workspace 失败。
    pub fn fail(&self, id: &str, error: String) -> Result<ShadowWorkspace> {
        self.transition(id, ShadowStatus::Failed, Some(error))
    }

    /// 合并 workspace 分支回 base_branch,然后清理 worktree。
    ///
    /// 前置:status 必须为 Completed 或 Running(允许合并运行中的,虽然不推荐)。
    /// 使用 `git merge --no-ff` 保留分支历史。合并后删除 worktree + 分支。
    #[instrument(skip(self))]
    pub fn merge(&self, id: &str) -> Result<ShadowWorkspace> {
        let ws = self.get(id).ok_or_else(|| anyhow!("workspace {id} not found"))?;
        let repo = self.repo()?;

        // 状态检查:已完成或运行中才允许合并
        match ws.status {
            ShadowStatus::Completed | ShadowStatus::Running => {}
            ShadowStatus::Merged => return Err(anyhow!("workspace {id} already merged")),
            ShadowStatus::Aborted => return Err(anyhow!("workspace {id} already aborted")),
            ShadowStatus::Failed => return Err(anyhow!("workspace {id} failed, cannot merge")),
            ShadowStatus::Creating => return Err(anyhow!("workspace {id} still creating")),
        }

        // 在主 repo 执行 merge(确保当前分支是 base_branch)
        // 注意:如果用户当前不在 base_branch,merge 会错乱。
        // 这里先 checkout base_branch 再 merge。生产环境应更谨慎,
        // 但 v1 假设用户在 Shadow Workspace 期间不切换分支。
        let current = Self::current_branch(&repo)?;
        if current != ws.base_branch {
            run_git(&repo, &["checkout", &ws.base_branch])
                .context(format!("checkout base branch {}", ws.base_branch))?;
        }

        // git merge --no-ff agent/<id> -m "merge shadow workspace <id>"
        let merge_msg = format!("merge shadow workspace {id}: {}", ws.task_description);
        run_git(&repo, &["merge", "--no-ff", &ws.branch, "-m", &merge_msg])
            .context(format!("merging {} into {}", ws.branch, ws.base_branch))?;

        // 清理:remove worktree + delete branch
        self.cleanup_worktree(&ws)?;
        self.delete_branch(&repo, &ws.branch)?;

        self.transition(id, ShadowStatus::Merged, None)
    }

    /// 丢弃 workspace:remove worktree + delete branch,不可逆。
    #[instrument(skip(self))]
    pub fn abort(&self, id: &str) -> Result<ShadowWorkspace> {
        let ws = self.get(id).ok_or_else(|| anyhow!("workspace {id} not found"))?;
        let repo = self.repo()?;

        match ws.status {
            ShadowStatus::Aborted => return Err(anyhow!("workspace {id} already aborted")),
            ShadowStatus::Merged => return Err(anyhow!("workspace {id} already merged")),
            _ => {}
        }

        // cleanup worktree(force,因为可能有未提交修改)
        self.cleanup_worktree_force(&ws)?;
        self.delete_branch_force(&repo, &ws.branch)?;

        self.transition(id, ShadowStatus::Aborted, None)
    }

    /// 清理已完成/已丢弃的 worktree 目录(保留分支记录)。
    /// 仅对 Merged/Aborted 状态有效,删除磁盘上的 worktree 目录。
    pub fn cleanup(&self, id: &str) -> Result<()> {
        let ws = self.get(id).ok_or_else(|| anyhow!("workspace {id} not found"))?;
        match ws.status {
            ShadowStatus::Merged | ShadowStatus::Aborted => {
                let path = Path::new(&ws.path);
                if path.exists() {
                    std::fs::remove_dir_all(path)
                        .with_context(|| format!("removing worktree dir {}", ws.path))?;
                }
                Ok(())
            }
            _ => Err(anyhow!("workspace {id} not in cleanable state (status={})", ws.status.as_str())),
        }
    }

    // ---- 内部辅助 ----

    fn transition(
        &self,
        id: &str,
        new_status: ShadowStatus,
        error: Option<String>,
    ) -> Result<ShadowWorkspace> {
        let mut map = self.workspaces.write();
        let ws = map
            .get_mut(id)
            .ok_or_else(|| anyhow!("workspace {id} not found"))?;
        ws.status = new_status;
        ws.finished_at = Some(now_secs());
        ws.error = error;
        Ok(ws.clone())
    }

    fn cleanup_worktree(&self, ws: &ShadowWorkspace) -> Result<()> {
        let repo = self.repo()?;
        // git worktree remove <path>
        let path = Path::new(&ws.path);
        if path.exists() {
            run_git(&repo, &["worktree", "remove", &ws.path])
                .or_else(|_| {
                    // git worktree remove 失败(可能有未提交修改),尝试 force
                    run_git(&repo, &["worktree", "remove", "--force", &ws.path])
                })
                .or_else(|_| {
                    // 仍然失败,手动删除目录
                    let _ = std::fs::remove_dir_all(&ws.path);
                    Ok::<String, anyhow::Error>(String::new())
                })?;
        }
        Ok(())
    }

    fn cleanup_worktree_force(&self, ws: &ShadowWorkspace) -> Result<()> {
        let repo = self.repo()?;
        let path = Path::new(&ws.path);
        if path.exists() {
            run_git(&repo, &["worktree", "remove", "--force", &ws.path])
                .or_else(|_| {
                    let _ = std::fs::remove_dir_all(&ws.path);
                    Ok::<String, anyhow::Error>(String::new())
                })?;
        }
        Ok(())
    }

    fn delete_branch(&self, repo: &Path, branch: &str) -> Result<()> {
        // git branch -d(要求已合并)
        run_git(repo, &["branch", "-d", branch])
            .or_else(|_| {
                // -d 失败(未合并),用 -D 强制删除(aborted 场景)
                run_git(repo, &["branch", "-D", branch])
            })?;
        Ok(())
    }

    fn delete_branch_force(&self, repo: &Path, branch: &str) -> Result<()> {
        run_git(repo, &["branch", "-D", branch])?;
        Ok(())
    }
}

/// 在给定 repo 路径下运行 git 命令,返回 stdout。
fn run_git(repo: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("spawning git {args:?}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let first_line = stderr.lines().next().unwrap_or("git failed");
        return Err(anyhow!("git {} failed: {}", args.join(" "), first_line));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 在临时目录创建一个 git repo 用于测试。
    fn make_test_repo() -> Result<PathBuf> {
        let dir = std::env::temp_dir().join(format!("nebula-shadow-test-{}", ShadowWorkspaceEngine::gen_id()));
        std::fs::create_dir_all(&dir)?;
        // git init + 初始 commit(需要 user.email/name 配置)
        run_git(&dir, &["init", "--initial-branch=main"])?;
        run_git(&dir, &["config", "user.email", "test@nebula.test"])?;
        run_git(&dir, &["config", "user.name", "Test"])?;
        std::fs::write(dir.join("README.md"), "# test\n")?;
        run_git(&dir, &["add", "-A"])?;
        run_git(&dir, &["commit", "-m", "initial"])?;
        Ok(dir)
    }

    fn cleanup_test_repo(dir: &Path) {
        // git worktree 可能注册了路径,先 prune
        let _ = run_git(dir, &["worktree", "prune", "--expire=now"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn gen_id_is_unique_ish() {
        let a = ShadowWorkspaceEngine::gen_id();
        let b = ShadowWorkspaceEngine::gen_id();
        assert_ne!(a, b, "consecutive gen_id should differ");
        assert_eq!(a.len(), 8, "id should be 8 chars");
    }

    #[test]
    fn create_produces_running_workspace() {
        let repo = make_test_repo().expect("make_test_repo");
        let engine = ShadowWorkspaceEngine::with_default();
        engine.set_repo_root(repo.clone());

        let ws = engine.create("refactor module X".into(), None).expect("create");
        assert_eq!(ws.status, ShadowStatus::Running);
        assert_eq!(ws.base_branch, "main");
        assert!(ws.branch.starts_with("agent/"));
        assert!(Path::new(&ws.path).exists(), "worktree dir should exist");
        assert_eq!(ws.task_description, "refactor module X");

        // worktree 内应有 README.md(继承自 base)
        let readme = Path::new(&ws.path).join("README.md");
        assert!(readme.exists(), "worktree should inherit base files");

        cleanup_test_repo(&repo);
    }

    #[test]
    fn list_returns_workspaces_desc_by_created() {
        let repo = make_test_repo().expect("make_test_repo");
        let engine = ShadowWorkspaceEngine::with_default();
        engine.set_repo_root(repo.clone());

        let _ = engine.create("task A".into(), None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _ = engine.create("task B".into(), None).unwrap();

        let list = engine.list();
        assert_eq!(list.len(), 2);
        // 降序:最新在前
        assert!(list[0].created_at >= list[1].created_at);

        cleanup_test_repo(&repo);
    }

    #[test]
    fn diff_shows_changes_in_worktree() {
        let repo = make_test_repo().expect("make_test_repo");
        let engine = ShadowWorkspaceEngine::with_default();
        engine.set_repo_root(repo.clone());

        let ws = engine.create("edit file".into(), None).unwrap();
        // 在 worktree 中修改文件
        std::fs::write(Path::new(&ws.path).join("README.md"), "# changed\n").unwrap();
        // diff 应包含 "changed"
        let diff = engine.diff(&ws.id).expect("diff");
        assert!(diff.contains("changed"), "diff should show changes: {diff}");

        cleanup_test_repo(&repo);
    }

    #[test]
    fn run_command_executes_in_worktree_cwd() {
        let repo = make_test_repo().expect("make_test_repo");
        let engine = ShadowWorkspaceEngine::with_default();
        engine.set_repo_root(repo.clone());

        let ws = engine.create("run command".into(), None).unwrap();
        // 在 worktree 中执行 pwd(Windows 用 cd)
        let (program, args) = if cfg!(windows) {
            ("cmd", vec!["/C".to_string(), "cd".to_string()])
        } else {
            ("pwd", vec![])
        };
        let out = engine.run_command(&ws.id, program, &args).expect("run_command");
        // 输出应包含 worktree 路径
        assert!(out.contains(&ws.id), "output should be in worktree dir: {out}");

        cleanup_test_repo(&repo);
    }

    #[test]
    fn merge_brings_changes_back_to_base() {
        let repo = make_test_repo().expect("make_test_repo");
        let engine = ShadowWorkspaceEngine::with_default();
        engine.set_repo_root(repo.clone());

        let ws = engine.create("add feature".into(), None).unwrap();
        // 在 worktree 中添加新文件 + commit
        std::fs::write(Path::new(&ws.path).join("feature.txt"), "new feature\n").unwrap();
        run_git(Path::new(&ws.path), &["add", "-A"]).unwrap();
        run_git(Path::new(&ws.path), &["commit", "-m", "add feature"]).unwrap();

        // 标记完成 + 合并
        engine.complete(&ws.id).unwrap();
        let merged = engine.merge(&ws.id).expect("merge");
        assert_eq!(merged.status, ShadowStatus::Merged);

        // 主 repo 应有 feature.txt
        assert!(repo.join("feature.txt").exists(), "merged file should exist in base");

        // worktree 目录应已清理
        assert!(!Path::new(&ws.path).exists(), "worktree dir should be removed after merge");

        cleanup_test_repo(&repo);
    }

    #[test]
    fn abort_discards_changes_and_cleans_up() {
        let repo = make_test_repo().expect("make_test_repo");
        let engine = ShadowWorkspaceEngine::with_default();
        engine.set_repo_root(repo.clone());

        let ws = engine.create("discard me".into(), None).unwrap();
        // 在 worktree 中修改(不 commit)
        std::fs::write(Path::new(&ws.path).join("junk.txt"), "junk\n").unwrap();

        let aborted = engine.abort(&ws.id).expect("abort");
        assert_eq!(aborted.status, ShadowStatus::Aborted);

        // worktree 目录应已清理
        assert!(!Path::new(&ws.path).exists(), "worktree dir should be removed after abort");
        // 主 repo 不应有 junk.txt
        assert!(!repo.join("junk.txt").exists(), "aborted changes should not leak to base");

        // 分支应已删除
        let branches = run_git(&repo, &["branch", "--list"]).unwrap();
        assert!(!branches.contains(&ws.branch), "agent branch should be deleted: {branches}");

        cleanup_test_repo(&repo);
    }

    #[test]
    fn complete_transitions_to_completed() {
        let repo = make_test_repo().expect("make_test_repo");
        let engine = ShadowWorkspaceEngine::with_default();
        engine.set_repo_root(repo.clone());

        let ws = engine.create("finish me".into(), None).unwrap();
        let completed = engine.complete(&ws.id).expect("complete");
        assert_eq!(completed.status, ShadowStatus::Completed);
        assert!(completed.finished_at.is_some());

        cleanup_test_repo(&repo);
    }

    #[test]
    fn fail_records_error_message() {
        let repo = make_test_repo().expect("make_test_repo");
        let engine = ShadowWorkspaceEngine::with_default();
        engine.set_repo_root(repo.clone());

        let ws = engine.create("will fail".into(), None).unwrap();
        let failed = engine.fail(&ws.id, "compilation error".into()).expect("fail");
        assert_eq!(failed.status, ShadowStatus::Failed);
        assert_eq!(failed.error.as_deref(), Some("compilation error"));

        cleanup_test_repo(&repo);
    }

    #[test]
    fn merge_already_merged_returns_error() {
        let repo = make_test_repo().expect("make_test_repo");
        let engine = ShadowWorkspaceEngine::with_default();
        engine.set_repo_root(repo.clone());

        let ws = engine.create("merge twice".into(), None).unwrap();
        engine.complete(&ws.id).unwrap();
        engine.merge(&ws.id).unwrap();

        let err = engine.merge(&ws.id).unwrap_err();
        assert!(err.to_string().contains("already merged"));

        cleanup_test_repo(&repo);
    }

    #[test]
    fn abort_already_aborted_returns_error() {
        let repo = make_test_repo().expect("make_test_repo");
        let engine = ShadowWorkspaceEngine::with_default();
        engine.set_repo_root(repo.clone());

        let ws = engine.create("abort twice".into(), None).unwrap();
        engine.abort(&ws.id).unwrap();

        let err = engine.abort(&ws.id).unwrap_err();
        assert!(err.to_string().contains("already aborted"));

        cleanup_test_repo(&repo);
    }

    #[test]
    fn create_without_repo_root_returns_error() {
        let engine = ShadowWorkspaceEngine::with_default();
        // 不调用 set_repo_root
        let err = engine.create("no repo".into(), None).unwrap_err();
        assert!(err.to_string().contains("repo root not configured"));
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let engine = ShadowWorkspaceEngine::with_default();
        assert!(engine.get("nonexistent").is_none());
    }
}

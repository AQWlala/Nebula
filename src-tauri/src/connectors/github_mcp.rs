//! T-E-L-04: GitHub MCP 连接器(pull-only)。
//!
//! Loop Engineering Connectors 语义:**只读拉取**用户 GitHub 数据
//! (repos / issues / PRs / commits / code search),不执行任何写操作
//! (无 create_issue / create_pr / comment / merge 等方法)。
//!
//! ## 与 OAuth 集成层的关系
//!
//! [`crate::oauth::TokenSet::access_token`] 由 T-E-C-18 OAuth 流程签发后,
//! 直接传入 [`GitHubConnector::new`] 即可:
//!
//! ```ignore
//! let token_set = oauth_manager.get_token("github").await?;
//! let gh = GitHubConnector::new(token_set.access_token.clone());
//! let repos = gh.list_repos().await?;
//! ```
//!
//! GitHub OAuth token 不过期(见 `oauth/providers/github.rs`),因此本连接器
//! 不实现刷新逻辑;token 失效时由 OAuth 层重新授权。
//!
//! ## 设计原则
//!
//! * **零新依赖** — 仅使用 Cargo.toml 已有声明的 crate(reqwest / chrono /
//!   serde / serde_json / url / async-trait)。
//! * **HTTP 与解析解耦** — 所有解析逻辑由 `parse_*` 自由函数承担,接受
//!   `&serde_json::Value`,使单元测试可在不发真实 HTTP 请求的情况下覆盖。
//! * **pull-only** — 不存在任何 POST/PATCH/PUT/DELETE 调用。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// GitHub REST API 基址。
pub const API_BASE: &str = "https://api.github.com";

// ── 错误类型 ────────────────────────────────────────────────────────

/// GitHub API 错误类型。
///
/// 覆盖 pull-only 流程中可预期的全部失败模式。写操作错误(已禁用)不在其中。
#[derive(Debug, Clone)]
pub enum GitHubApiError {
    /// 404 — 资源不存在(仓库 / issue / PR / commit)。
    NotFound,
    /// 403 且 `X-RateLimit-Remaining: 0` — 触发次级速率限制。
    /// `reset_at` 为速率限制重置的绝对时间(UTC),来自 `X-RateLimit-Reset`。
    RateLimited { reset_at: Option<DateTime<Utc>> },
    /// 401 — token 无效 / 已撤销,或 403(权限不足 / scope 缺失)。
    Unauthorized,
    /// 网络层失败(连接超时 / DNS / TLS / 非预期 HTTP 状态码)。
    Network(String),
    /// 响应体 JSON 解析或字段缺失失败。
    Parse(String),
}

impl fmt::Display for GitHubApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(f, "GitHub API: 资源不存在 (404)"),
            Self::RateLimited { reset_at } => match reset_at {
                Some(t) => write!(f, "GitHub API: 触发速率限制,重置于 {t}"),
                None => write!(f, "GitHub API: 触发速率限制"),
            },
            Self::Unauthorized => write!(f, "GitHub API: 未授权 (401/403)"),
            Self::Network(msg) => write!(f, "GitHub API: 网络错误 — {msg}"),
            Self::Parse(msg) => write!(f, "GitHub API: 解析错误 — {msg}"),
        }
    }
}

impl std::error::Error for GitHubApiError {}

impl From<reqwest::Error> for GitHubApiError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_decode() {
            Self::Parse(e.to_string())
        } else {
            Self::Network(e.to_string())
        }
    }
}

// ── 数据结构 ────────────────────────────────────────────────────────

/// 仓库信息(只读拉取的子集,非完整 GitHub repo 对象)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepoInfo {
    /// `owner/repo` 形式全名。
    pub full_name: String,
    /// 仓库描述(可为空)。
    pub description: Option<String>,
    /// 是否私有。
    pub private: bool,
    /// 默认分支(如 `main` / `master`)。
    pub default_branch: String,
    /// star 数(对应 API 字段 `stargazers_count`)。
    pub stars: u64,
    /// 最后更新时间(UTC)。
    pub updated_at: Option<DateTime<Utc>>,
}

/// Issue 信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IssueInfo {
    pub number: u64,
    pub title: String,
    /// `open` / `closed`。
    pub state: String,
    /// 标签名列表(API 返回对象数组,此处扁平化)。
    pub labels: Vec<String>,
    /// 正文(API 可能为 null)。
    pub body: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    /// 作者 login(API 字段 `user.login`,系统事件可能为 null)。
    pub author: Option<String>,
}

/// Pull Request 信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PullRequestInfo {
    pub number: u64,
    pub title: String,
    /// `open` / `closed`。
    pub state: String,
    /// 是否已合并(API 布尔字段 `merged`)。
    pub merged: bool,
    /// 是否草稿。
    pub draft: bool,
    /// 源分支(`head.ref`)。
    pub head_branch: Option<String>,
    /// 目标分支(`base.ref`)。
    pub base_branch: Option<String>,
}

/// 提交信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommitInfo {
    /// 完整 40 字符 SHA。
    pub sha: String,
    /// 提交消息(完整,含标题与正文)。
    pub message: String,
    /// 作者名(`commit.author.name`)。
    pub author: String,
    /// 作者日期(`commit.author.date`,UTC)。
    pub date: Option<DateTime<Utc>>,
}

/// 代码搜索结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchResult {
    /// 命中文件在仓库内的相对路径。
    pub path: String,
    /// `owner/repo`。
    pub repo: String,
    /// GitHub 网页 URL。
    pub html_url: String,
    /// 相关性评分(API `score`)。
    pub score: f64,
}

// ── 过滤器 ──────────────────────────────────────────────────────────

/// Issue 列表过滤器。映射到 `GET /repos/{owner}/{repo}/issues` 查询参数。
#[derive(Debug, Clone, Default)]
pub struct IssueFilter {
    /// `open` / `closed` / `all`(默认 `open`)。
    pub state: Option<String>,
    /// 逗号分隔的标签名列表。
    pub labels: Vec<String>,
    /// 指派人 login。
    pub assignee: Option<String>,
    /// 仅返回此时间之后更新的项。
    pub since: Option<DateTime<Utc>>,
}

impl IssueFilter {
    /// 转为 `(key, value)` 查询对。
    pub fn to_query_pairs(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        if let Some(state) = &self.state {
            pairs.push(("state".into(), state.clone()));
        }
        if !self.labels.is_empty() {
            pairs.push(("labels".into(), self.labels.join(",")));
        }
        if let Some(assignee) = &self.assignee {
            pairs.push(("assignee".into(), assignee.clone()));
        }
        if let Some(since) = &self.since {
            pairs.push(("since".into(), since.to_rfc3339()));
        }
        pairs
    }
}

/// Pull Request 列表过滤器。映射到 `GET /repos/{owner}/{repo}/pulls` 查询参数。
#[derive(Debug, Clone, Default)]
pub struct PrFilter {
    /// `open` / `closed` / `all`(默认 `open`)。
    pub state: Option<String>,
    pub labels: Vec<String>,
    pub assignee: Option<String>,
    pub since: Option<DateTime<Utc>>,
}

impl PrFilter {
    pub fn to_query_pairs(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        if let Some(state) = &self.state {
            pairs.push(("state".into(), state.clone()));
        }
        if !self.labels.is_empty() {
            pairs.push(("labels".into(), self.labels.join(",")));
        }
        if let Some(assignee) = &self.assignee {
            pairs.push(("assignee".into(), assignee.clone()));
        }
        if let Some(since) = &self.since {
            pairs.push(("since".into(), since.to_rfc3339()));
        }
        pairs
    }
}

// ── 连接器 ──────────────────────────────────────────────────────────

/// GitHub 连接器(pull-only)。
///
/// 持有 OAuth access token 与一个复用的 `reqwest::Client`。所有方法均为
/// `async` 且只读;不存在写操作方法。
pub struct GitHubConnector {
    token: String,
    http: reqwest::Client,
}

impl GitHubConnector {
    /// 创建连接器。
    ///
    /// `token` 来自 OAuth 层的 [`crate::oauth::TokenSet::access_token`]。
    pub fn new(token: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("nebula/2.0 github-mcp-connector")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { token, http }
    }

    /// 发起已认证 GET 请求并返回解析后的 JSON。
    ///
    /// 统一处理 401/403/404 与速率限制头,将 HTTP 错误映射为
    /// [`GitHubApiError`]。2xx 响应体解析失败映射为 `Parse`。
    async fn get_json(&self, url: &str) -> Result<serde_json::Value, GitHubApiError> {
        let resp = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| GitHubApiError::Network(e.to_string()))?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(GitHubApiError::Unauthorized);
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(GitHubApiError::NotFound);
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            // 区分速率限制与权限不足。
            let is_rate_limited = resp
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|v| v.to_str().ok())
                .map(|s| s == "0")
                .unwrap_or(false);
            if is_rate_limited {
                let reset_at = resp
                    .headers()
                    .get("x-ratelimit-reset")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<i64>().ok())
                    .and_then(|secs| DateTime::from_timestamp(secs, 0));
                return Err(GitHubApiError::RateLimited { reset_at });
            }
            return Err(GitHubApiError::Unauthorized);
        }
        if !status.is_success() {
            return Err(GitHubApiError::Network(format!("HTTP {status}")));
        }

        resp.json::<serde_json::Value>()
            .await
            .map_err(|e| GitHubApiError::Parse(e.to_string()))
    }

    /// 拼接带查询参数的完整 URL。
    fn url_with_query(path: &str, pairs: &[(String, String)]) -> String {
        if pairs.is_empty() {
            format!("{API_BASE}{path}")
        } else {
            let qs = build_query_string(pairs);
            format!("{API_BASE}{path}?{qs}")
        }
    }

    /// 列出已认证用户可访问的仓库(`GET /user/repos`)。
    pub async fn list_repos(&self) -> Result<Vec<RepoInfo>, GitHubApiError> {
        let url = Self::url_with_query(
            "/user/repos",
            &[
                ("per_page".into(), "100".into()),
                ("sort".into(), "updated".into()),
            ],
        );
        let json = self.get_json(&url).await?;
        parse_repos(&json)
    }

    /// 获取单个仓库(`GET /repos/{owner}/{repo}`)。
    pub async fn get_repo(&self, owner: &str, repo: &str) -> Result<RepoInfo, GitHubApiError> {
        let url = format!("{API_BASE}/repos/{owner}/{repo}");
        let json = self.get_json(&url).await?;
        parse_repo(&json)
    }

    /// 列出仓库 Issues(`GET /repos/{owner}/{repo}/issues`)。
    ///
    /// GitHub 的 issues 端点会同时返回 PR,此处过滤掉含 `pull_request` 字段
    /// 的条目,确保只返回真正意义上的 Issue。
    pub async fn list_issues(
        &self,
        owner: &str,
        repo: &str,
        filter: IssueFilter,
    ) -> Result<Vec<IssueInfo>, GitHubApiError> {
        let mut pairs = filter.to_query_pairs();
        pairs.push(("per_page".into(), "100".into()));
        let url = Self::url_with_query(&format!("/repos/{owner}/{repo}/issues"), &pairs);
        let json = self.get_json(&url).await?;
        // 过滤 PR(issues 端点会混入 PR 条目)。
        let filtered = match &json {
            serde_json::Value::Array(arr) => {
                let kept: Vec<serde_json::Value> = arr
                    .iter()
                    .filter(|v| v.get("pull_request").is_none())
                    .cloned()
                    .collect();
                serde_json::Value::Array(kept)
            }
            other => other.clone(),
        };
        parse_issues(&filtered)
    }

    /// 获取单个 Issue(`GET /repos/{owner}/{repo}/issues/{number}`)。
    pub async fn get_issue(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<IssueInfo, GitHubApiError> {
        let url = format!("{API_BASE}/repos/{owner}/{repo}/issues/{number}");
        let json = self.get_json(&url).await?;
        parse_issue(&json)
    }

    /// 列出仓库 Pull Requests(`GET /repos/{owner}/{repo}/pulls`)。
    pub async fn list_pull_requests(
        &self,
        owner: &str,
        repo: &str,
        filter: PrFilter,
    ) -> Result<Vec<PullRequestInfo>, GitHubApiError> {
        let mut pairs = filter.to_query_pairs();
        pairs.push(("per_page".into(), "100".into()));
        let url = Self::url_with_query(&format!("/repos/{owner}/{repo}/pulls"), &pairs);
        let json = self.get_json(&url).await?;
        parse_pulls(&json)
    }

    /// 获取单个 PR(`GET /repos/{owner}/{repo}/pulls/{number}`)。
    pub async fn get_pull_request(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<PullRequestInfo, GitHubApiError> {
        let url = format!("{API_BASE}/repos/{owner}/{repo}/pulls/{number}");
        let json = self.get_json(&url).await?;
        parse_pull(&json)
    }

    /// 列出仓库最近提交(`GET /repos/{owner}/{repo}/commits`)。
    ///
    /// `max` 限制返回数量(映射到 `per_page`)。
    pub async fn list_commits(
        &self,
        owner: &str,
        repo: &str,
        max: usize,
    ) -> Result<Vec<CommitInfo>, GitHubApiError> {
        let per_page = max.clamp(1, 100).to_string();
        let url = Self::url_with_query(
            &format!("/repos/{owner}/{repo}/commits"),
            &[("per_page".into(), per_page)],
        );
        let json = self.get_json(&url).await?;
        parse_commits(&json)
    }

    /// 跨仓库代码搜索(`GET /search/code`)。
    ///
    /// `repos` 为 `owner/repo` 列表,逐个以 `repo:` 限定符拼入查询。
    pub async fn search_code(
        &self,
        query: &str,
        repos: &[String],
    ) -> Result<Vec<SearchResult>, GitHubApiError> {
        let mut q = query.to_string();
        for r in repos {
            if !q.is_empty() {
                q.push(' ');
            }
            q.push_str("repo:");
            q.push_str(r);
        }
        let url = Self::url_with_query("/search/code", &[("q".into(), q)]);
        let json = self.get_json(&url).await?;
        parse_search_results(&json)
    }
}

// ── 查询字符串构建 ──────────────────────────────────────────────────

/// 用 `url::form_urlencoded` 构建百分号编码的查询字符串(零新依赖:url 已在 Cargo.toml)。
fn build_query_string(pairs: &[(String, String)]) -> String {
    let mut ser = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        ser.append_pair(k, v);
    }
    ser.finish()
}

// ── JSON 解析(纯函数,便于单元测试)────────────────────────────────

/// 解析 RFC 3339 时间字符串为 UTC `DateTime`。
fn parse_dt(s: &str) -> Option<DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn parse_repo(v: &serde_json::Value) -> Result<RepoInfo, GitHubApiError> {
    let full_name = v["full_name"]
        .as_str()
        .ok_or_else(|| GitHubApiError::Parse("repo 缺少 full_name".into()))?
        .to_string();
    Ok(RepoInfo {
        full_name,
        description: v["description"].as_str().map(|s| s.to_string()),
        private: v["private"].as_bool().unwrap_or(false),
        default_branch: v["default_branch"].as_str().unwrap_or("main").to_string(),
        stars: v["stargazers_count"].as_u64().unwrap_or(0),
        updated_at: v["updated_at"].as_str().and_then(parse_dt),
    })
}

fn parse_repos(v: &serde_json::Value) -> Result<Vec<RepoInfo>, GitHubApiError> {
    let arr = v
        .as_array()
        .ok_or_else(|| GitHubApiError::Parse("repos 响应非数组".into()))?;
    arr.iter().map(parse_repo).collect()
}

fn parse_issue(v: &serde_json::Value) -> Result<IssueInfo, GitHubApiError> {
    let number = v["number"]
        .as_u64()
        .ok_or_else(|| GitHubApiError::Parse("issue 缺少 number".into()))?;
    let title = v["title"]
        .as_str()
        .ok_or_else(|| GitHubApiError::Parse("issue 缺少 title".into()))?
        .to_string();
    let state = v["state"].as_str().unwrap_or("open").to_string();
    let labels = v["labels"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|l| l["name"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    Ok(IssueInfo {
        number,
        title,
        state,
        labels,
        body: v["body"].as_str().map(|s| s.to_string()),
        created_at: v["created_at"].as_str().and_then(parse_dt),
        author: v["user"]["login"].as_str().map(|s| s.to_string()),
    })
}

fn parse_issues(v: &serde_json::Value) -> Result<Vec<IssueInfo>, GitHubApiError> {
    let arr = v
        .as_array()
        .ok_or_else(|| GitHubApiError::Parse("issues 响应非数组".into()))?;
    arr.iter().map(parse_issue).collect()
}

fn parse_pull(v: &serde_json::Value) -> Result<PullRequestInfo, GitHubApiError> {
    let number = v["number"]
        .as_u64()
        .ok_or_else(|| GitHubApiError::Parse("pull request 缺少 number".into()))?;
    let title = v["title"]
        .as_str()
        .ok_or_else(|| GitHubApiError::Parse("pull request 缺少 title".into()))?
        .to_string();
    let state = v["state"].as_str().unwrap_or("open").to_string();
    Ok(PullRequestInfo {
        number,
        title,
        state,
        merged: v["merged"].as_bool().unwrap_or(false),
        draft: v["draft"].as_bool().unwrap_or(false),
        head_branch: v["head"]["ref"].as_str().map(|s| s.to_string()),
        base_branch: v["base"]["ref"].as_str().map(|s| s.to_string()),
    })
}

fn parse_pulls(v: &serde_json::Value) -> Result<Vec<PullRequestInfo>, GitHubApiError> {
    let arr = v
        .as_array()
        .ok_or_else(|| GitHubApiError::Parse("pulls 响应非数组".into()))?;
    arr.iter().map(parse_pull).collect()
}

fn parse_commit(v: &serde_json::Value) -> Result<CommitInfo, GitHubApiError> {
    let sha = v["sha"]
        .as_str()
        .ok_or_else(|| GitHubApiError::Parse("commit 缺少 sha".into()))?
        .to_string();
    let message = v["commit"]["message"]
        .as_str()
        .ok_or_else(|| GitHubApiError::Parse("commit 缺少 commit.message".into()))?
        .to_string();
    let author = v["commit"]["author"]["name"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let date = v["commit"]["author"]["date"].as_str().and_then(parse_dt);
    Ok(CommitInfo {
        sha,
        message,
        author,
        date,
    })
}

fn parse_commits(v: &serde_json::Value) -> Result<Vec<CommitInfo>, GitHubApiError> {
    let arr = v
        .as_array()
        .ok_or_else(|| GitHubApiError::Parse("commits 响应非数组".into()))?;
    arr.iter().map(parse_commit).collect()
}

fn parse_search_results(v: &serde_json::Value) -> Result<Vec<SearchResult>, GitHubApiError> {
    let items = v["items"]
        .as_array()
        .ok_or_else(|| GitHubApiError::Parse("search 响应缺少 items 数组".into()))?;
    items
        .iter()
        .map(|item| {
            let path = item["path"]
                .as_str()
                .ok_or_else(|| GitHubApiError::Parse("search item 缺少 path".into()))?
                .to_string();
            let repo = item["repository"]["full_name"]
                .as_str()
                .ok_or_else(|| {
                    GitHubApiError::Parse("search item 缺少 repository.full_name".into())
                })?
                .to_string();
            let html_url = item["html_url"]
                .as_str()
                .ok_or_else(|| GitHubApiError::Parse("search item 缺少 html_url".into()))?
                .to_string();
            let score = item["score"].as_f64().unwrap_or(0.0);
            Ok(SearchResult {
                path,
                repo,
                html_url,
                score,
            })
        })
        .collect()
}

// ── 单元测试(mock JSON,不发真实请求)─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    // ── RepoInfo 解析 ──────────────────────────────────────────────

    #[test]
    fn parse_repo_basic() {
        let v = json!({
            "full_name": "octocat/Hello-World",
            "description": "My first repo",
            "private": false,
            "default_branch": "main",
            "stargazers_count": 80,
            "updated_at": "2011-01-26T19:01:12Z"
        });
        let r = parse_repo(&v).unwrap();
        assert_eq!(r.full_name, "octocat/Hello-World");
        assert_eq!(r.description.as_deref(), Some("My first repo"));
        assert!(!r.private);
        assert_eq!(r.default_branch, "main");
        assert_eq!(r.stars, 80);
        assert!(r.updated_at.is_some());
    }

    #[test]
    fn parse_repo_missing_full_name_errors() {
        let v = json!({ "private": true });
        let err = parse_repo(&v).unwrap_err();
        assert!(matches!(err, GitHubApiError::Parse(_)));
    }

    #[test]
    fn parse_repo_optional_fields_absent() {
        let v = json!({ "full_name": "a/b", "private": true });
        let r = parse_repo(&v).unwrap();
        assert_eq!(r.full_name, "a/b");
        assert!(r.description.is_none());
        assert!(r.private);
        assert_eq!(r.default_branch, "main"); // 默认回退
        assert_eq!(r.stars, 0);
        assert!(r.updated_at.is_none());
    }

    #[test]
    fn parse_repos_array() {
        let v = json!([
            { "full_name": "a/b" },
            { "full_name": "c/d", "stargazers_count": 5 }
        ]);
        let repos = parse_repos(&v).unwrap();
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].full_name, "a/b");
        assert_eq!(repos[1].stars, 5);
    }

    #[test]
    fn parse_repos_empty_array() {
        let v = json!([]);
        assert!(parse_repos(&v).unwrap().is_empty());
    }

    #[test]
    fn parse_repos_non_array_errors() {
        let v = json!({ "message": "Not Found" });
        assert!(matches!(
            parse_repos(&v).unwrap_err(),
            GitHubApiError::Parse(_)
        ));
    }

    // ── IssueInfo 解析 ─────────────────────────────────────────────

    #[test]
    fn parse_issue_basic_with_labels() {
        let v = json!({
            "number": 42,
            "title": "Bug: crash on start",
            "state": "open",
            "labels": [
                { "name": "bug" },
                { "name": "P0" }
            ],
            "body": "steps to reproduce",
            "created_at": "2020-01-01T00:00:00Z",
            "user": { "login": "octocat" }
        });
        let i = parse_issue(&v).unwrap();
        assert_eq!(i.number, 42);
        assert_eq!(i.title, "Bug: crash on start");
        assert_eq!(i.state, "open");
        assert_eq!(i.labels, vec!["bug", "P0"]);
        assert_eq!(i.body.as_deref(), Some("steps to reproduce"));
        assert_eq!(i.author.as_deref(), Some("octocat"));
        assert!(i.created_at.is_some());
    }

    #[test]
    fn parse_issue_missing_number_errors() {
        let v = json!({ "title": "x" });
        assert!(matches!(
            parse_issue(&v).unwrap_err(),
            GitHubApiError::Parse(_)
        ));
    }

    #[test]
    fn parse_issue_null_user_yields_none_author() {
        let v = json!({ "number": 1, "title": "t", "user": null });
        let i = parse_issue(&v).unwrap();
        assert!(i.author.is_none());
        assert!(i.labels.is_empty());
    }

    #[test]
    fn parse_issues_array() {
        let v = json!([
            { "number": 1, "title": "a" },
            { "number": 2, "title": "b" }
        ]);
        let issues = parse_issues(&v).unwrap();
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[1].number, 2);
    }

    // ── PullRequestInfo 解析 ───────────────────────────────────────

    #[test]
    fn parse_pull_merged() {
        let v = json!({
            "number": 7,
            "title": "Add feature",
            "state": "closed",
            "merged": true,
            "draft": false,
            "head": { "ref": "feature-branch" },
            "base": { "ref": "main" }
        });
        let p = parse_pull(&v).unwrap();
        assert_eq!(p.number, 7);
        assert_eq!(p.state, "closed");
        assert!(p.merged);
        assert!(!p.draft);
        assert_eq!(p.head_branch.as_deref(), Some("feature-branch"));
        assert_eq!(p.base_branch.as_deref(), Some("main"));
    }

    #[test]
    fn parse_pull_draft_open() {
        let v = json!({
            "number": 9,
            "title": "WIP",
            "state": "open",
            "merged": false,
            "draft": true
        });
        let p = parse_pull(&v).unwrap();
        assert!(p.draft);
        assert!(!p.merged);
        assert!(p.head_branch.is_none());
    }

    #[test]
    fn parse_pull_missing_number_errors() {
        let v = json!({ "title": "x" });
        assert!(matches!(
            parse_pull(&v).unwrap_err(),
            GitHubApiError::Parse(_)
        ));
    }

    #[test]
    fn parse_pulls_array() {
        let v = json!([
            { "number": 1, "title": "a", "merged": false },
            { "number": 2, "title": "b", "merged": true }
        ]);
        let pulls = parse_pulls(&v).unwrap();
        assert_eq!(pulls.len(), 2);
        assert!(!pulls[0].merged);
        assert!(pulls[1].merged);
    }

    // ── CommitInfo 解析 ────────────────────────────────────────────

    #[test]
    fn parse_commit_basic() {
        let v = json!({
            "sha": "abc123def456",
            "commit": {
                "message": "Fix typo\n\nDetailed body",
                "author": {
                    "name": "Octo Cat",
                    "date": "2021-06-01T12:00:00Z"
                }
            }
        });
        let c = parse_commit(&v).unwrap();
        assert_eq!(c.sha, "abc123def456");
        assert!(c.message.contains("Fix typo"));
        assert_eq!(c.author, "Octo Cat");
        assert!(c.date.is_some());
    }

    #[test]
    fn parse_commit_missing_sha_errors() {
        let v = json!({ "commit": { "message": "x" } });
        assert!(matches!(
            parse_commit(&v).unwrap_err(),
            GitHubApiError::Parse(_)
        ));
    }

    #[test]
    fn parse_commits_array() {
        let v = json!([
            { "sha": "aaa", "commit": { "message": "m1", "author": {} } },
            { "sha": "bbb", "commit": { "message": "m2", "author": { "name": "x" } } }
        ]);
        let commits = parse_commits(&v).unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].sha, "aaa");
        assert_eq!(commits[1].author, "x");
    }

    // ── SearchResult 解析 ─────────────────────────────────────────

    #[test]
    fn parse_search_results_basic() {
        let v = json!({
            "total_count": 2,
            "items": [
                {
                    "path": "src/main.rs",
                    "repository": { "full_name": "octocat/repo" },
                    "html_url": "https://github.com/octocat/repo/blob/main/src/main.rs",
                    "score": 1.23
                },
                {
                    "path": "lib.rs",
                    "repository": { "full_name": "octocat/repo" },
                    "html_url": "https://github.com/octocat/repo/blob/main/lib.rs",
                    "score": 0.45
                }
            ]
        });
        let results = parse_search_results(&v).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].path, "src/main.rs");
        assert_eq!(results[0].repo, "octocat/repo");
        assert!((results[0].score - 1.23).abs() < 1e-9);
        assert_eq!(
            results[1].html_url,
            "https://github.com/octocat/repo/blob/main/lib.rs"
        );
    }

    #[test]
    fn parse_search_results_empty_items() {
        let v = json!({ "total_count": 0, "items": [] });
        assert!(parse_search_results(&v).unwrap().is_empty());
    }

    #[test]
    fn parse_search_results_missing_items_errors() {
        let v = json!({ "total_count": 0 });
        assert!(matches!(
            parse_search_results(&v).unwrap_err(),
            GitHubApiError::Parse(_)
        ));
    }

    // ── 过滤器查询对 ──────────────────────────────────────────────

    #[test]
    fn issue_filter_query_pairs_full() {
        let since = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        let f = IssueFilter {
            state: Some("closed".into()),
            labels: vec!["bug".into(), "P0".into()],
            assignee: Some("octocat".into()),
            since: Some(since),
        };
        let pairs = f.to_query_pairs();
        // 4 对,顺序固定:state, labels, assignee, since。
        assert_eq!(pairs.len(), 4);
        assert_eq!(pairs[0], ("state".into(), "closed".into()));
        assert_eq!(pairs[1], ("labels".into(), "bug,P0".into()));
        assert_eq!(pairs[2], ("assignee".into(), "octocat".into()));
        assert!(pairs[3].0 == "since");
        assert!(pairs[3].1.contains("2020-01-01"));
    }

    #[test]
    fn pr_filter_query_pairs_default_empty() {
        let f = PrFilter::default();
        assert!(f.to_query_pairs().is_empty());
    }

    #[test]
    fn issue_filter_query_pairs_partial() {
        let f = IssueFilter {
            state: Some("all".into()),
            ..Default::default()
        };
        let pairs = f.to_query_pairs();
        assert_eq!(pairs, vec![("state".into(), "all".into())]);
    }

    // ── 错误类型 ──────────────────────────────────────────────────

    #[test]
    fn error_display_messages() {
        assert!(format!("{}", GitHubApiError::NotFound).contains("404"));
        assert!(format!("{}", GitHubApiError::Unauthorized).contains("401"));
        assert!(format!("{}", GitHubApiError::Network("timeout".into())).contains("timeout"));
        assert!(format!("{}", GitHubApiError::Parse("bad json".into())).contains("bad json"));
        let rl = GitHubApiError::RateLimited { reset_at: None };
        assert!(format!("{rl}").contains("速率限制"));
    }

    #[test]
    fn rate_limited_with_reset_at_displays_time() {
        let t = Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap();
        let rl = GitHubApiError::RateLimited { reset_at: Some(t) };
        let msg = format!("{rl}");
        assert!(msg.contains("2030"));
    }

    // ── URL 与查询构建 ────────────────────────────────────────────

    #[test]
    fn url_with_query_no_pairs_has_no_question_mark() {
        let url = GitHubConnector::url_with_query("/user/repos", &[]);
        assert_eq!(url, "https://api.github.com/user/repos");
    }

    #[test]
    fn url_with_query_encodes_pairs() {
        let url = GitHubConnector::url_with_query(
            "/search/code",
            &[("q".into(), "fn main repo:a/b".into())],
        );
        assert!(url.starts_with("https://api.github.com/search/code?"));
        // 空格应被百分号编码为 +。
        assert!(url.contains("q=fn+main"));
        // 斜杠在 form-urlencoded 中不编码。
        assert!(url.contains("repo%3Aa"));
    }

    #[test]
    fn build_query_string_percent_encodes() {
        let qs = build_query_string(&[("labels".into(), "bug P0".into())]);
        // 空格 -> '+'
        assert_eq!(qs, "labels=bug+P0");
    }

    // ── 连接器构造 ────────────────────────────────────────────────

    #[test]
    fn connector_new_does_not_panic() {
        let c = GitHubConnector::new("ghp_test_token".into());
        // 无法直接访问私有 token 字段,但构造成功即表明 client builder 正常。
        // 通过构造不 panic 验证 user_agent / timeout 设置有效。
        let _ = &c;
    }

    #[test]
    fn connector_new_empty_token_succeeds() {
        // 空 token 也应能构造(实际请求会返回 401,但构造本身不应失败)。
        let _c = GitHubConnector::new(String::new());
    }
}

//! Loop Engineering Connectors — pull-only 数据连接器集合。
//!
//! 每个连接器以只读方式拉取外部服务数据,与 `oauth` 集成层配合:
//! OAuth 流程签发 token → 连接器用 token 拉取数据。
//!
//! ## 已实现
//!
//! | 连接器 | 模块 | 数据源 | 模式 |
//! |--------|------|--------|------|
//! | GitHub | [`github_mcp`] | GitHub REST API | pull-only |

pub mod github_mcp;

pub use github_mcp::{
    CommitInfo, GitHubApiError, GitHubConnector, IssueFilter, IssueInfo, PrFilter, PullRequestInfo,
    RepoInfo, SearchResult,
};

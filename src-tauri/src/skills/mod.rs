//! `nebula::skills` — v0.3 procedural-memory subsystem.
//!
//! The `skills` table was reserved in v0.1 (see
//! `migrations/001_initial.sql`) but never written to. v0.3 promotes
//! it to a first-class subsystem with its own store, execution engine
//! and gRPC/Tauri surface.
//!
//! ## Layout
//!
//! * [`types`] — wire-shape DTOs (Skill, SkillResult, request/response
//!   envelopes).
//! * [`store`] — thin SQLite wrapper around the existing `skills` table.
//! * [`engine`] — orchestrates creation, execution, rating and search.
//! * [`extractor`] — v1.2 skill closed-loop learning: auto-distils reusable
//!   skills from successful swarm task executions.
//! * [`sandbox`] — v1.3 capability-based sandbox model for skill execution
//!   isolation (P1-3).
//! * [`marketplace`] — v1.3 skill marketplace with search, one-click install,
//!   update checking and publishing infrastructure (P2-7).
//! * [`exec_approval`] — T-E-S-20 exec 类操作审批门禁（fail-closed 超时拒绝）。

pub mod audit;
// T-E-S-36: 能力层 — Capability + CapabilityRegistry。
pub mod capability;
pub mod discover;
pub mod engine;
pub mod exec_approval;
pub mod executor;
pub mod exporter;
pub mod extractor;
pub mod hub_client;
pub mod importer;
pub mod marketplace;
// T-E-S-36: 协议层 — SkillManifest / SkillRequest / SkillResponse。
pub mod protocol;
// T-E-S-46: 技能发布(GitHub Gist / 本地文件)。
pub mod publisher;
pub mod sandbox;
pub mod seeder;
pub mod store;
pub mod types;

pub use audit::{redact_if_sensitive, truncate_summary, SkillAuditEntry, SkillAuditLogger};
pub use engine::SkillEngine;
pub use exec_approval::{
    ExecApprovalRequest, ExecApprovalStatus, ExecApprovalTracker,
    DEFAULT_EXEC_APPROVAL_TIMEOUT_SECS, TIMEOUT_FAIL_CLOSED_REASON,
};
pub use exporter::SkillExporter;
pub use extractor::{ExtractionReport, SkillExtractor};
pub use hub_client::{HubSkillDetail, HubSkillSummary, TeamSkillsHubClient, TeamSkillsHubImporter};
pub use importer::{ImportResult, SkillImporter, SkillSource};
pub use marketplace::{
    MarketplaceQuery, MarketplaceResponse, MarketplaceStats, PublishManifest, SearchHit,
    SkillEntry, SkillMarketplace, SortBy, UpdateInfo,
};
// T-E-S-46: 技能发布器 trait + 实现 + 内联 SKILL.md 序列化。
pub use publisher::{
    skill_to_skill_md, FilePublisher, GistPublisher, PublishResult, SkillPublisher,
};
pub use sandbox::{
    Capability, CapabilitySet, RiskLevel, SandboxConfig, SandboxPolicy, SandboxResult,
};
pub use seeder::seed_demo_skills;
pub use store::SkillStore;
pub use types::{
    CreateSkillRequest, ListSkillsRequest, RateSkillRequest, Skill, SkillResult,
    SkillSearchRequest, UseSkillRequest,
};
// T-E-S-36: 三层架构(协议层 / 能力层 / 执行层)关键类型 re-export。
// 注意:capability::Capability 与 sandbox::Capability 同名,这里只 re-export
// CapabilityRegistry,capability::Capability 需通过完整路径
// `crate::skills::capability::Capability` 访问以避免命名冲突。
pub use capability::CapabilityRegistry;
pub use executor::{LocalExecutor, McpExecutor, RemoteExecutor, SkillExecutor};
pub use protocol::{SkillManifest, SkillRequest, SkillResponse, SkillTransport};

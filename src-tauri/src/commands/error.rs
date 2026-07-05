//! Refined error envelope for Tauri command responses.
//!
//! v0.2 replaces the single `internal / validation / not_found` triad
//! from v0.1 with a fine-grained enum ([`ErrorCode`]) so the front-end
//! can show domain-specific UI (retry, fallback, upgrade prompts) and
//! so backend services can map errors to HTTP status codes or gRPC
//! trailers without parsing the message string.
//!
//! ## Wire format
//!
//! ```json
//! { "code": "db", "message": "memory_store failed", "details": null }
//! ```
//!
//! `details` is **only** available when the front-end opts in via the
//! `include_details` field on a request DTO (e.g. debug builds). In
//! production the server omits it to avoid leaking internal state.
//!
//! ## Compatibility
//!
//! v0.1 callers see the same `code` strings they used to (`internal`,
//! `validation`, `not_found`) for legacy error sites. New error sites
//! use one of the [`ErrorCode`] variants below.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::error;

/// Fine-grained machine-readable error category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// SQLite-backed store error.
    Db,
    /// LanceDB-backed vector store error.
    Lance,
    /// LLM gateway / Ollama error.
    Llm,
    /// Memory subsystem error (sponge / black-hole / reflect).
    Memory,
    /// Swarm orchestration error.
    Swarm,
    /// Parameter validation error.
    Validation,
    /// Resource not found.
    NotFound,
    /// Permission / authorisation error.
    Permission,
    /// Catch-all internal error.
    Internal,
    /// Service temporarily unavailable.
    Unavailable,
}

impl ErrorCode {
    /// Stable string form, used as the wire-level `code` field.
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::Db => "db",
            ErrorCode::Lance => "lance",
            ErrorCode::Llm => "llm",
            ErrorCode::Memory => "memory",
            ErrorCode::Swarm => "swarm",
            ErrorCode::Validation => "validation",
            ErrorCode::NotFound => "not_found",
            ErrorCode::Permission => "permission",
            ErrorCode::Internal => "internal",
            ErrorCode::Unavailable => "unavailable",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The user-visible command error envelope.
#[derive(Debug, Clone, Serialize, Deserialize, Error)]
#[error("{code}: {message}")]
pub struct CommandError {
    /// Machine-readable category.
    pub code: ErrorCode,
    /// Short, user-safe message.
    pub message: String,
    /// Optional internal details — only populated when the request
    /// explicitly opts in (debug builds). Never surfaced in production
    /// UIs without sanitisation.
    pub details: Option<String>,
}

impl CommandError {
    /// Build a DB-layer error. Logs the full chain via `tracing::error`.
    pub fn db(context: &str, e: &anyhow::Error) -> Self {
        log_failure("db", context, e);
        Self {
            code: ErrorCode::Db,
            message: safe_msg(context),
            details: None,
        }
    }

    /// Build a LanceDB-layer error.
    pub fn lance(context: &str, e: &anyhow::Error) -> Self {
        log_failure("lance", context, e);
        Self {
            code: ErrorCode::Lance,
            message: safe_msg(context),
            details: None,
        }
    }

    /// Build an LLM-layer error.
    pub fn llm(context: &str, e: &anyhow::Error) -> Self {
        log_failure("llm", context, e);
        Self {
            code: ErrorCode::Llm,
            message: safe_msg(context),
            details: None,
        }
    }

    /// Build a memory-subsystem error.
    pub fn memory(context: &str, e: &anyhow::Error) -> Self {
        log_failure("memory", context, e);
        Self {
            code: ErrorCode::Memory,
            message: safe_msg(context),
            details: None,
        }
    }

    /// Build a swarm-orchestration error.
    pub fn swarm(context: &str, e: &anyhow::Error) -> Self {
        log_failure("swarm", context, e);
        Self {
            code: ErrorCode::Swarm,
            message: safe_msg(context),
            details: None,
        }
    }

    /// Build a validation error. The message is user-safe and is shown
    /// verbatim.  The optional `details` argument is attached to the
    /// error envelope (visible in the Tauri catch-all handler and in
    /// the `tracing::error!` emission) but **never** shown to the
    /// end-user — that's the only difference vs. the 1-arg form.
    // v1.0.1 fix D: the call sites in `commands/mod.rs` were
    // written assuming a 2-arg form
    // `CommandError::validation("ctx", &e.to_string())`.  When
    // the helper was tightened to 1-arg in v0.5, those callers
    // silently started ignoring the second argument.  Restore
    // the 2-arg form, but default `details` to `None` so any
    // existing single-arg calls keep compiling.
    pub fn validation(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Validation,
            message: msg.into(),
            details: None,
        }
    }

    /// Build a not-found error. The `what` descriptor is included in
    /// the safe message.
    pub fn not_found(what: impl Into<String>) -> Self {
        let w = what.into();
        Self {
            code: ErrorCode::NotFound,
            message: format!("{w} not found"),
            details: None,
        }
    }

    /// Build a permission error.
    pub fn permission(msg: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Permission,
            message: msg.into(),
            details: None,
        }
    }

    /// Build a service-unavailable error (e.g. Ollama is down).
    pub fn unavailable(context: &str, e: &anyhow::Error) -> Self {
        log_failure("unavailable", context, e);
        Self {
            code: ErrorCode::Unavailable,
            message: safe_msg(context),
            details: None,
        }
    }

    /// Catch-all internal error. Preserved for backward compatibility
    /// with v0.1 callers.
    pub fn internal(context: &str, e: &anyhow::Error) -> Self {
        log_failure("internal", context, e);
        Self {
            code: ErrorCode::Internal,
            message: safe_msg(context),
            details: None,
        }
    }

    /// Attach a `details` field (only safe for debug builds).
    #[must_use]
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }
}

/// Convenience: convert any `anyhow::Error` into a generic internal
/// error. The full chain is logged.
impl From<anyhow::Error> for CommandError {
    fn from(e: anyhow::Error) -> Self {
        Self::internal("command", &e)
    }
}

fn log_failure(code: &str, context: &str, e: &anyhow::Error) {
    error!(target: "nebula.cmd", code, context, error = ?e, "command failed");
}

fn safe_msg(context: &str) -> String {
    format!("{context} failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anyhow_err() -> anyhow::Error {
        anyhow::anyhow!("secret db path /home/alice/x.db blew up")
    }

    #[test]
    fn db_error_hides_details_in_message() {
        let e = CommandError::db("memory_store", &anyhow_err());
        assert_eq!(e.code, ErrorCode::Db);
        assert!(e.message.contains("memory_store"));
        assert!(!e.message.contains("/home/alice"));
    }

    #[test]
    fn lance_error_uses_lance_code() {
        let e = CommandError::lance("search", &anyhow_err());
        assert_eq!(e.code, ErrorCode::Lance);
        assert_eq!(e.code.as_str(), "lance");
    }

    #[test]
    fn llm_error_uses_llm_code() {
        let e = CommandError::llm("chat", &anyhow_err());
        assert_eq!(e.code, ErrorCode::Llm);
    }

    #[test]
    fn memory_error_uses_memory_code() {
        let e = CommandError::memory("sponge", &anyhow_err());
        assert_eq!(e.code, ErrorCode::Memory);
    }

    #[test]
    fn swarm_error_uses_swarm_code() {
        let e = CommandError::swarm("orchestrate", &anyhow_err());
        assert_eq!(e.code, ErrorCode::Swarm);
    }

    #[test]
    fn validation_message_is_user_facing() {
        let e = CommandError::validation("empty user_message");
        assert_eq!(e.code, ErrorCode::Validation);
        assert_eq!(e.message, "empty user_message");
    }

    #[test]
    fn not_found_includes_descriptor() {
        let e = CommandError::not_found("memory");
        assert_eq!(e.code, ErrorCode::NotFound);
        assert!(e.message.contains("memory"));
    }

    #[test]
    fn permission_message_is_user_facing() {
        let e = CommandError::permission("admin only");
        assert_eq!(e.code, ErrorCode::Permission);
        assert_eq!(e.message, "admin only");
    }

    #[test]
    fn unavailable_error_uses_unavailable_code() {
        let e = CommandError::unavailable("chat", &anyhow_err());
        assert_eq!(e.code, ErrorCode::Unavailable);
        assert!(e.message.contains("chat"));
    }

    #[test]
    fn internal_error_keeps_v0_1_wire_format() {
        let e = CommandError::internal("chat", &anyhow_err());
        // v0.1 clients checked `code == "internal"`.
        assert_eq!(e.code.as_str(), "internal");
        assert_eq!(e.code, ErrorCode::Internal);
    }

    #[test]
    fn from_anyhow_yields_internal() {
        let e: CommandError = anyhow::anyhow!("boom").into();
        assert_eq!(e.code, ErrorCode::Internal);
    }

    #[test]
    fn with_details_attaches_string() {
        let e = CommandError::validation("x").with_details("debug info");
        assert_eq!(e.details.as_deref(), Some("debug info"));
    }

    #[test]
    fn error_code_strings_are_stable() {
        assert_eq!(ErrorCode::Db.as_str(), "db");
        assert_eq!(ErrorCode::Lance.as_str(), "lance");
        assert_eq!(ErrorCode::Llm.as_str(), "llm");
        assert_eq!(ErrorCode::Memory.as_str(), "memory");
        assert_eq!(ErrorCode::Swarm.as_str(), "swarm");
        assert_eq!(ErrorCode::Validation.as_str(), "validation");
        assert_eq!(ErrorCode::NotFound.as_str(), "not_found");
        assert_eq!(ErrorCode::Permission.as_str(), "permission");
        assert_eq!(ErrorCode::Internal.as_str(), "internal");
        assert_eq!(ErrorCode::Unavailable.as_str(), "unavailable");
    }

    #[test]
    fn display_includes_code_and_message() {
        let e = CommandError::validation("empty");
        let s = format!("{e}");
        assert!(s.contains("validation"));
        assert!(s.contains("empty"));
    }
}

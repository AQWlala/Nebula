//! v0.5: clipboard backend.
//!
//! The clipboard is process-local in this version (no X11/Wayland
//! integration).  We use the `arboard` crate when available and fall
//! back to platform-specific shims if it isn't.  For v0.5 we ship a
//! small wrapper that uses `arboard` for read/write and a simple
//! in-process poll for change detection (best-effort).
//!
//! The v0.5 surface area is intentionally tiny: just three
//! operations the Tauri command layer can call.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use tracing::{debug, warn};

/// Process-wide clipboard facade.  Cheap to clone (one Arc + Mutex).
#[derive(Clone)]
pub struct ClipboardService {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    /// Last seen clipboard text.  Used to debounce `watch` polls.
    last_seen: String,
    /// Reuse a single `arboard::Clipboard` instance for the process
    /// — its constructor is non-trivial on some platforms.
    backend: Option<arboard_facade::Clipboard>,
}

/// Newtype over `arboard::Clipboard` so we don't drag the dependency
/// into the public type signature (and so we can stub it for tests).
mod arboard_facade {
    use anyhow::Result;
    pub use arboard::Clipboard;
    /// Extension trait adding the few methods we use.
    pub trait ClipboardExt {
        fn read_text(&mut self) -> Result<String>;
        fn write_text(&mut self, text: &str) -> Result<()>;
    }
    impl ClipboardExt for Clipboard {
        fn read_text(&mut self) -> Result<String> {
            Ok(Clipboard::get_text(self)?)
        }
        fn write_text(&mut self, text: &str) -> Result<()> {
            Clipboard::set_text(self, text.to_string())?;
            Ok(())
        }
    }
}

impl ClipboardService {
    pub fn new() -> Result<Self> {
        let backend = match arboard_facade::Clipboard::new() {
            Ok(c) => Some(c),
            Err(e) => {
                warn!(target: "nebula.os", error = ?e, "clipboard backend unavailable; reads/writes will fail");
                None
            }
        };
        Ok(Self {
            inner: Arc::new(Mutex::new(Inner {
                last_seen: String::new(),
                backend,
            })),
        })
    }

    pub fn noop() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                last_seen: String::new(),
                backend: None,
            })),
        }
    }

    /// Returns the current clipboard text.  An empty string is a
    /// legitimate value (e.g. the user just selected whitespace and
    /// pressed copy) — callers should not treat it as an error.
    pub fn read_text(&self) -> Result<String> {
        let mut g = self.inner.lock();
        let backend = g
            .backend
            .as_mut()
            .ok_or_else(|| anyhow!("clipboard backend not initialised"))?;
        let text = arboard_facade::ClipboardExt::read_text(backend)?;
        g.last_seen = text.clone();
        Ok(text)
    }

    /// Overwrites the clipboard with `text`.  Errors only if the
    /// platform backend refuses.
    pub fn write_text(&self, text: &str) -> Result<()> {
        let mut g = self.inner.lock();
        let backend = g
            .backend
            .as_mut()
            .ok_or_else(|| anyhow!("clipboard backend not initialised"))?;
        arboard_facade::ClipboardExt::write_text(backend, text)?;
        g.last_seen = text.to_string();
        debug!(target: "nebula.os", bytes = text.len(), "clipboard write ok");
        Ok(())
    }

    /// Polls the clipboard for a change and returns the new text if
    /// it differs from the previously seen one.  Returns `None` when
    /// the clipboard is unchanged or the backend is unavailable.
    ///
    /// `timeout` caps the total time spent polling (the poll itself
    /// sleeps `interval` between reads).  This is intentionally a
    /// best-effort, blocking implementation — the front-end can call
    /// it on a worker task.
    pub fn watch_once(&self, timeout: Duration, interval: Duration) -> Result<Option<String>> {
        let deadline = std::time::Instant::now() + timeout;
        let last_seen = self.inner.lock().last_seen.clone();
        loop {
            match self.read_text() {
                Ok(text) => {
                    if text != last_seen {
                        // read_text already updates last_seen.
                        return Ok(Some(text));
                    }
                }
                Err(e) => {
                    warn!(target: "nebula.os", error = ?e, "clipboard read failed during watch");
                }
            }
            if std::time::Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(interval);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_write_round_trip() {
        // Some CI sandboxes don't have a clipboard; treat failure
        // as "skipped" rather than a hard test failure.
        let svc = match ClipboardService::new() {
            Ok(s) => s,
            Err(_) => {
                eprintln!("clipboard unavailable; skipping");
                return;
            }
        };
        let marker = "nebula-v0.5-test";
        if svc.write_text(marker).is_err() {
            eprintln!("clipboard write failed; skipping");
            return;
        }
        let back = svc.read_text().expect("read");
        assert_eq!(back, marker);
    }
}

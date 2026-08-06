use crate::config::Config;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};
use tokio::sync::Notify;

/// How often the config file's mtime is checked.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Live configuration shared by every handler.
///
/// A background task polls `config.toml` and hot-swaps the value in place; the
/// version counter lets connected overlays notice and refresh themselves.
#[derive(Clone)]
pub struct ConfigWatcher {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    current: RwLock<Arc<Config>>,
    version: AtomicU64,
    changed: Notify,
}

impl ConfigWatcher {
    pub fn new(path: PathBuf, config: Config) -> Self {
        Self {
            inner: Arc::new(Inner {
                path,
                current: RwLock::new(Arc::new(config)),
                version: AtomicU64::new(0),
                changed: Notify::new(),
            }),
        }
    }

    /// Current configuration snapshot; cheap to clone and never blocks writers.
    pub fn config(&self) -> Arc<Config> {
        self.inner
            .current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Bumped on every successful reload.
    pub fn version(&self) -> u64 {
        self.inner.version.load(Ordering::Acquire)
    }

    /// Resolves the next time the configuration is reloaded, or after `timeout`
    /// if nothing changes. Returns the version observed afterwards.
    pub async fn wait_for_change(&self, since: u64, timeout: Duration) -> u64 {
        // Register interest *before* re-checking, so a reload racing with this
        // call cannot slip through unnoticed.
        let notified = self.inner.changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        if self.version() == since {
            let _ = tokio::time::timeout(timeout, notified).await;
        }
        self.version()
    }

    /// Watch the config file forever, reloading it whenever it changes on disk.
    pub async fn watch(self) {
        let path = self.inner.path.clone();
        let mut last_modified = modified_at(&path);

        loop {
            tokio::time::sleep(POLL_INTERVAL).await;

            // A missing/locked file is usually an editor mid-save: retry later.
            let Some(modified) = modified_at(&path) else {
                continue;
            };
            if Some(modified) == last_modified {
                continue;
            }
            last_modified = Some(modified);

            match Config::load(&path) {
                Ok(config) => {
                    tracing::info!(
                        path = %path.display(),
                        users = config.users.len(),
                        "configuration reloaded — refreshing overlays"
                    );
                    *self
                        .inner
                        .current
                        .write()
                        .unwrap_or_else(|e| e.into_inner()) = Arc::new(config);
                    self.inner.version.fetch_add(1, Ordering::Release);
                    self.inner.changed.notify_waiters();
                }
                Err(err) => tracing::warn!(
                    path = %path.display(),
                    %err,
                    "invalid configuration — keeping the previous one"
                ),
            }
        }
    }
}

fn modified_at(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

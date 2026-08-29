//! Debounced live reload with last-good-config semantics (spec §7.5).

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::layer::{LoadedConfig, load};
use crate::paths::Paths;

/// Reload outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReloadEvent {
    /// New config applied.
    Applied {
        /// Path that changed.
        path: PathBuf,
    },
    /// Parse/schema error; previous config kept.
    Invalid {
        /// Path that changed.
        path: PathBuf,
        /// Error message.
        message: String,
    },
    /// Theme directory changed.
    ThemeChanged {
        /// Theme name.
        name: String,
    },
}

/// Live configuration with last-good snapshot.
pub struct ConfigStore {
    paths: Paths,
    inner: Arc<Mutex<LoadedConfig>>,
    events: Receiver<ReloadEvent>,
    _watcher: Option<RecommendedWatcher>,
}

impl ConfigStore {
    /// Load once without watching.
    pub fn load(paths: Paths) -> Result<Self, omacell_core::error::CoreError> {
        let loaded = load(&paths, &[], None)?;
        let (_tx, rx) = mpsc::channel();
        Ok(Self {
            paths,
            inner: Arc::new(Mutex::new(loaded)),
            events: rx,
            _watcher: None,
        })
    }

    /// Load and watch the user config directory.
    pub fn load_and_watch(paths: Paths) -> Result<Self, omacell_core::error::CoreError> {
        let loaded = load(&paths, &[], None)?;
        let (tx, rx) = mpsc::channel();
        let inner = Arc::new(Mutex::new(loaded));
        let debounce = Duration::from_millis(
            inner
                .lock()
                .map(|g| g.config.config.debounce_ms)
                .unwrap_or(50),
        );
        let watcher = spawn_watcher(paths.clone(), inner.clone(), tx, debounce)?;
        Ok(Self {
            paths,
            inner,
            events: rx,
            _watcher: Some(watcher),
        })
    }

    /// Current last-good config.
    #[must_use]
    pub fn snapshot(&self) -> LoadedConfig {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Re-read files (SIGUSR1 / theme hook).
    pub fn reload(&self) -> Result<(), omacell_core::error::CoreError> {
        let next = load(&self.paths, &[], None)?;
        if let Ok(mut g) = self.inner.lock() {
            *g = next;
        }
        Ok(())
    }

    /// Drain pending reload events (non-blocking).
    pub fn drain_events(&self) -> Vec<ReloadEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.events.try_recv() {
            out.push(ev);
        }
        out
    }
}

fn spawn_watcher(
    paths: Paths,
    inner: Arc<Mutex<LoadedConfig>>,
    tx: Sender<ReloadEvent>,
    debounce: Duration,
) -> Result<RecommendedWatcher, omacell_core::error::CoreError> {
    let watch_dir = paths.user_config.clone();
    std::fs::create_dir_all(&watch_dir).map_err(|e| crate::error::io(e.to_string()))?;
    let (raw_tx, raw_rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
        if let Ok(ev) = res {
            let _ = raw_tx.send(ev);
        }
    })
    .map_err(|e| crate::error::io(e.to_string()))?;
    watcher
        .watch(&watch_dir, RecursiveMode::NonRecursive)
        .map_err(|e| crate::error::io(e.to_string()))?;
    std::thread::spawn(move || {
        let mut last = Instant::now()
            .checked_sub(debounce)
            .unwrap_or_else(Instant::now);
        while let Ok(_ev) = raw_rx.recv() {
            // coalesce
            while raw_rx.try_recv().is_ok() {}
            let wait = debounce.saturating_sub(last.elapsed());
            std::thread::sleep(wait);
            last = Instant::now();
            match load(&paths, &[], None) {
                Ok(next) => {
                    if let Ok(mut g) = inner.lock() {
                        *g = next;
                    }
                    let _ = tx.send(ReloadEvent::Applied {
                        path: paths.user_config_toml(),
                    });
                }
                Err(e) => {
                    let _ = tx.send(ReloadEvent::Invalid {
                        path: paths.user_config_toml(),
                        message: e.message.clone(),
                    });
                }
            }
        }
    });
    Ok(watcher)
}

//! Debounced live reload with last-good-config semantics (spec §7.5).

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::layer::{LoadOptions, LoadedConfig, load_with_options};
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
    options: LoadOptions,
    inner: Arc<Mutex<LoadedConfig>>,
    events: Receiver<ReloadEvent>,
    _watcher: Option<RecommendedWatcher>,
}

impl ConfigStore {
    /// Load once without watching.
    pub fn load(paths: Paths) -> Result<Self, omacell_core::error::CoreError> {
        let options = LoadOptions::from_process();
        let loaded = load_with_options(&paths, &options)?;
        let (_tx, rx) = mpsc::channel();
        Ok(Self {
            paths,
            options,
            inner: Arc::new(Mutex::new(loaded)),
            events: rx,
            _watcher: None,
        })
    }

    /// Load and watch the user config directory.
    pub fn load_and_watch(paths: Paths) -> Result<Self, omacell_core::error::CoreError> {
        Self::load_and_watch_with(paths, LoadOptions::from_process())
    }

    /// Load and watch while retaining workbook, environment, and CLI layers.
    pub fn load_and_watch_with(
        paths: Paths,
        options: LoadOptions,
    ) -> Result<Self, omacell_core::error::CoreError> {
        let loaded = load_with_options(&paths, &options)?;
        let (tx, rx) = mpsc::channel();
        let inner = Arc::new(Mutex::new(loaded));
        let debounce = Duration::from_millis(
            inner
                .lock()
                .map(|g| g.config.config.debounce_ms)
                .unwrap_or(50),
        );
        let live_reload = inner
            .lock()
            .map(|config| config.config.config.live_reload)
            .unwrap_or(false);
        let watcher = if live_reload {
            Some(spawn_watcher(
                paths.clone(),
                options.clone(),
                inner.clone(),
                tx,
                debounce,
            )?)
        } else {
            None
        };
        Ok(Self {
            paths,
            options,
            inner,
            events: rx,
            _watcher: watcher,
        })
    }

    /// Current last-good config.
    #[must_use]
    pub fn snapshot(&self) -> LoadedConfig {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Re-read files (SIGUSR1 / theme hook).
    pub fn reload(&self) -> Result<(), omacell_core::error::CoreError> {
        let next = load_with_options(&self.paths, &self.options)?;
        if let Ok(mut g) = self.inner.lock() {
            *g = next;
        }
        Ok(())
    }

    /// Whether filesystem live reload is active.
    #[must_use]
    pub fn is_watching(&self) -> bool {
        self._watcher.is_some()
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
    options: LoadOptions,
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
        .watch(&watch_dir, RecursiveMode::Recursive)
        .map_err(|e| crate::error::io(e.to_string()))?;
    let mut watched_roots = vec![watch_dir.clone()];
    for path in [
        paths.omarchy_state.clone(),
        paths.omarchy_config.clone(),
        paths.home.join(".config/fontconfig"),
    ] {
        if path.is_dir() && !watched_roots.iter().any(|root| path.starts_with(root)) {
            watcher
                .watch(&path, RecursiveMode::Recursive)
                .map_err(|e| crate::error::io(e.to_string()))?;
            watched_roots.push(path);
        }
    }
    if let Some(parent) = options
        .theme_override
        .as_deref()
        .and_then(std::path::Path::parent)
        && parent.is_dir()
        && !watched_roots.iter().any(|root| parent.starts_with(root))
    {
        watcher
            .watch(parent, RecursiveMode::NonRecursive)
            .map_err(|e| crate::error::io(e.to_string()))?;
    }
    std::thread::spawn(move || {
        let mut last = Instant::now()
            .checked_sub(debounce)
            .unwrap_or_else(Instant::now);
        while let Ok(event) = raw_rx.recv() {
            let mut changed_paths = event.paths;
            // coalesce
            while let Ok(event) = raw_rx.try_recv() {
                changed_paths.extend(event.paths);
            }
            let wait = debounce.saturating_sub(last.elapsed());
            std::thread::sleep(wait);
            last = Instant::now();
            let changed_path = changed_paths
                .into_iter()
                .next()
                .unwrap_or_else(|| paths.user_config_toml());
            match load_with_options(&paths, &options) {
                Ok(next) => {
                    let mut config_changed = false;
                    let mut theme_changed = false;
                    if let Ok(mut g) = inner.lock() {
                        config_changed = g.config != next.config || g.provenance != next.provenance;
                        theme_changed = g.theme != next.theme || g.shell != next.shell;
                        *g = next;
                    }
                    if config_changed {
                        let _ = tx.send(ReloadEvent::Applied {
                            path: changed_path.clone(),
                        });
                    }
                    if theme_changed {
                        let name = inner
                            .lock()
                            .map(|loaded| loaded.theme.name.clone())
                            .unwrap_or_else(|poisoned| poisoned.into_inner().theme.name.clone());
                        let _ = tx.send(ReloadEvent::ThemeChanged { name });
                    }
                }
                Err(e) => {
                    let _ = tx.send(ReloadEvent::Invalid {
                        path: changed_path,
                        message: e.message.clone(),
                    });
                }
            }
        }
    });
    Ok(watcher)
}

//! Debounced live reload with last-good-config semantics (spec §7.5).

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::layer::{LoadOptions, LoadedConfig, load_with_options};
use crate::paths::Paths;

type EventWaker = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
struct EventSink {
    tx: Sender<ReloadEvent>,
    waker: Arc<Mutex<Option<EventWaker>>>,
}

impl EventSink {
    fn send(&self, event: ReloadEvent) {
        if self.tx.send(event).is_err() {
            return;
        }
        let wake = self
            .waker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(wake) = wake {
            wake();
        }
    }
}

/// Reload outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReloadEvent {
    /// The user configuration tree was successfully revalidated.
    Applied {
        /// Revalidated path in the user configuration tree.
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

/// Cloneable, `Send + Sync` reload target for SIGUSR1 and `theme.reload`.
#[derive(Clone)]
pub struct ReloadHandle {
    paths: Paths,
    options: LoadOptions,
    inner: Arc<Mutex<LoadedConfig>>,
    events: EventSink,
}

impl ReloadHandle {
    /// Validate all retained sources without replacing the last-good snapshot.
    pub fn check(&self) -> Result<(), omacell_core::error::CoreError> {
        load_with_options(&self.paths, &self.options).map(|_| ())
    }

    /// Re-read files using the retained [`LoadOptions`].
    pub fn reload(&self) -> Result<(), omacell_core::error::CoreError> {
        let path = self
            .options
            .config_file
            .clone()
            .unwrap_or_else(|| self.paths.user_config_toml());
        let next = match load_with_options(&self.paths, &self.options) {
            Ok(next) => next,
            Err(err) => {
                self.events.send(ReloadEvent::Invalid {
                    path,
                    message: err.message.clone(),
                });
                return Err(err);
            }
        };
        let theme_changed = {
            let mut current = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let changed = current.theme != next.theme || current.shell != next.shell;
            *current = next;
            changed
        };
        self.events.send(ReloadEvent::Applied { path });
        if theme_changed {
            let name = self.snapshot().theme.name;
            self.events.send(ReloadEvent::ThemeChanged { name });
        }
        Ok(())
    }

    /// Current last-good snapshot.
    #[must_use]
    pub fn snapshot(&self) -> LoadedConfig {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

/// Live configuration with last-good snapshot.
pub struct ConfigStore {
    paths: Paths,
    options: LoadOptions,
    inner: Arc<Mutex<LoadedConfig>>,
    event_sink: EventSink,
    events: Receiver<ReloadEvent>,
    _watcher: Option<RecommendedWatcher>,
}

impl ConfigStore {
    /// Load once without watching.
    pub fn load(paths: Paths) -> Result<Self, omacell_core::error::CoreError> {
        Self::load_with(paths, LoadOptions::from_process())
    }

    /// Load once, retaining workbook / CLI / env overlays, without watching.
    pub fn load_with(
        paths: Paths,
        options: LoadOptions,
    ) -> Result<Self, omacell_core::error::CoreError> {
        let loaded = load_with_options(&paths, &options)?;
        let (tx, rx) = mpsc::channel();
        let event_sink = EventSink {
            tx,
            waker: Arc::new(Mutex::new(None)),
        };
        Ok(Self {
            paths,
            options,
            inner: Arc::new(Mutex::new(loaded)),
            event_sink,
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
        let event_sink = EventSink {
            tx,
            waker: Arc::new(Mutex::new(None)),
        };
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
                event_sink.clone(),
                debounce,
            )?)
        } else {
            None
        };
        Ok(Self {
            paths,
            options,
            inner,
            event_sink,
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
        self.handle().reload()
    }

    /// Shareable reload target for commands and signal adapters.
    #[must_use]
    pub fn handle(&self) -> ReloadHandle {
        ReloadHandle {
            paths: self.paths.clone(),
            options: self.options.clone(),
            inner: self.inner.clone(),
            events: self.event_sink.clone(),
        }
    }

    /// Wake a frontend whenever a reload event is queued.
    ///
    /// GUI frontends use this to request a frame from watcher, signal, and
    /// command threads without polling while idle.
    pub fn set_event_waker(&self, wake: impl Fn() + Send + Sync + 'static) {
        *self
            .event_sink
            .waker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::new(wake));
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
    events: EventSink,
    debounce: Duration,
) -> Result<RecommendedWatcher, omacell_core::error::CoreError> {
    let watch_dir = paths.user_config.clone();
    let selected_keymap = {
        let config_root = options
            .config_file
            .as_deref()
            .and_then(std::path::Path::parent)
            .unwrap_or(&paths.user_config);
        let keymap = inner
            .lock()
            .map(|loaded| loaded.config.keys.file.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().config.keys.file.clone());
        config_root.join(keymap)
    };
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
    let mut watched_roots = vec![(watch_dir.clone(), true)];
    for path in [
        paths.omarchy_state.clone(),
        paths.omarchy_config.clone(),
        paths.home.join(".config/fontconfig"),
    ] {
        if path.is_dir()
            && !watched_roots
                .iter()
                .any(|(root, recursive)| path == *root || (*recursive && path.starts_with(root)))
        {
            watcher
                .watch(&path, RecursiveMode::Recursive)
                .map_err(|e| crate::error::io(e.to_string()))?;
            watched_roots.push((path, true));
        }
    }
    for parent in [
        Some(selected_keymap.as_path()),
        options.config_file.as_deref(),
        options.theme_override.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter_map(std::path::Path::parent)
    {
        if parent.is_dir()
            && !watched_roots
                .iter()
                .any(|(root, recursive)| parent == root || (*recursive && parent.starts_with(root)))
        {
            watcher
                .watch(parent, RecursiveMode::NonRecursive)
                .map_err(|e| crate::error::io(e.to_string()))?;
            watched_roots.push((parent.to_path_buf(), false));
        }
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
            let user_file_changed = changed_paths.iter().any(|path| {
                path.starts_with(&paths.user_config)
                    || options.config_file.as_ref() == Some(path)
                    || path == &selected_keymap
            });
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
                    if config_changed || user_file_changed {
                        events.send(ReloadEvent::Applied {
                            path: changed_path.clone(),
                        });
                    }
                    if theme_changed {
                        let name = inner
                            .lock()
                            .map(|loaded| loaded.theme.name.clone())
                            .unwrap_or_else(|poisoned| poisoned.into_inner().theme.name.clone());
                        events.send(ReloadEvent::ThemeChanged { name });
                    }
                }
                Err(e) => {
                    events.send(ReloadEvent::Invalid {
                        path: changed_path,
                        message: e.message.clone(),
                    });
                }
            }
        }
    });
    Ok(watcher)
}

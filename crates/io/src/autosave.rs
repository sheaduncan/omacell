//! Bounded crash-recovery snapshots for retained workbook sessions.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use omacell_core::error::CoreError;
use omacell_core::workbook::Workbook;
use serde::{Deserialize, Serialize};

use crate::xlsx::{self, SaveOptions};

const DEFAULT_MAX_SNAPSHOTS: usize = 20;
static SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct FileStamp {
    len: u64,
    modified_ns: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Manifest {
    version: u8,
    session: String,
    source: Option<PathBuf>,
    source_stamp: Option<FileStamp>,
    saved_at_ns: u128,
    snapshot_name: String,
}

/// One valid recovery snapshot discovered in the autosave directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryCandidate {
    /// Original workbook path, or `None` for an untitled workbook.
    pub source: Option<PathBuf>,
    /// Stable session identifier used to replace or clear this snapshot.
    pub session: String,
    /// Time the manifest was written, in nanoseconds since the Unix epoch.
    pub saved_at_ns: u128,
    /// Atomically written XLSX snapshot.
    pub snapshot: PathBuf,
    /// Manifest associated with [`Self::snapshot`].
    pub manifest: PathBuf,
}

/// State-directory store for autosave manifests and XLSX snapshots.
#[derive(Clone, Debug)]
pub struct AutosaveStore {
    root: PathBuf,
    max_snapshots: usize,
}

/// Per-window autosave timing and snapshot identity.
#[derive(Debug)]
pub struct AutosaveSession {
    store: AutosaveStore,
    id: String,
    last_attempt: Instant,
    in_flight: Option<std::thread::JoinHandle<Result<RecoveryCandidate, CoreError>>>,
}

impl AutosaveSession {
    /// Start a new retained session in `state_dir`.
    #[must_use]
    pub fn new(state_dir: &Path) -> Self {
        Self::with_id(state_dir, fresh_session_id())
    }

    /// Resume the identity of a snapshot recovered at launch.
    #[must_use]
    pub fn with_id(state_dir: &Path, id: impl Into<String>) -> Self {
        Self {
            store: AutosaveStore::new(state_dir),
            id: id.into(),
            last_attempt: Instant::now(),
            in_flight: None,
        }
    }

    /// Write a dirty snapshot once `interval_secs` has elapsed.
    ///
    /// Returns `true` when a background snapshot was queued. Zero disables
    /// autosave, and a busy or clean caller should pass `eligible = false`.
    pub fn snapshot_if_due(
        &mut self,
        workbook: &Workbook,
        source: Option<&Path>,
        interval_secs: u64,
        eligible: bool,
    ) -> Result<bool, CoreError> {
        self.reap_finished()?;
        if interval_secs == 0
            || !eligible
            || self.in_flight.is_some()
            || self.last_attempt.elapsed() < Duration::from_secs(interval_secs)
        {
            return Ok(false);
        }
        self.last_attempt = Instant::now();
        let store = self.store.clone();
        let id = self.id.clone();
        let workbook = workbook.clone();
        let source = source.map(Path::to_path_buf);
        self.in_flight = Some(std::thread::spawn(move || {
            store.write_snapshot(&id, &workbook, source.as_deref())
        }));
        Ok(true)
    }

    /// Immediately write a snapshot, primarily for lifecycle integrations and tests.
    pub fn snapshot_now(
        &mut self,
        workbook: &Workbook,
        source: Option<&Path>,
    ) -> Result<RecoveryCandidate, CoreError> {
        self.join_pending()?;
        self.last_attempt = Instant::now();
        self.store.write_snapshot(&self.id, workbook, source)
    }

    /// Remove this session's recoverable state after save or explicit discard.
    pub fn clear(&mut self) -> Result<(), CoreError> {
        self.join_pending()?;
        self.last_attempt = Instant::now();
        self.store.clear(&self.id)
    }

    /// Stable identifier of this retained session.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    fn reap_finished(&mut self) -> Result<(), CoreError> {
        if self
            .in_flight
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            self.join_pending()?;
        }
        Ok(())
    }

    fn join_pending(&mut self) -> Result<(), CoreError> {
        let Some(worker) = self.in_flight.take() else {
            return Ok(());
        };
        worker.join().map_err(|_| {
            CoreError::new("autosave.worker", "autosave worker panicked")
                .with_hint("save the workbook manually and inspect the diagnostic log")
        })??;
        Ok(())
    }
}

impl AutosaveStore {
    /// Use `<state_dir>/autosave` and retain at most twenty sessions.
    #[must_use]
    pub fn new(state_dir: &Path) -> Self {
        Self::with_limit(state_dir, DEFAULT_MAX_SNAPSHOTS)
    }

    /// Use a caller-selected retention limit. A zero limit is treated as one.
    #[must_use]
    pub fn with_limit(state_dir: &Path, max_snapshots: usize) -> Self {
        Self {
            root: state_dir.join("autosave"),
            max_snapshots: max_snapshots.max(1),
        }
    }

    /// Atomically replace the snapshot for `session` and prune older sessions.
    pub fn write_snapshot(
        &self,
        session: &str,
        workbook: &Workbook,
        source: Option<&Path>,
    ) -> Result<RecoveryCandidate, CoreError> {
        validate_session(session)?;
        fs::create_dir_all(&self.root).map_err(autosave_io)?;
        let snapshot = self.root.join(format!("{session}.xlsx"));
        let manifest_path = self.root.join(format!("{session}.json"));
        let source = source.map(normalize_path).transpose()?;
        let source_stamp = source.as_deref().map(file_stamp).transpose()?.flatten();
        let saved_at_ns = now_ns()?;
        let bytes = xlsx::save_workbook_bytes(workbook)?;
        let options = SaveOptions {
            keep_backups: 0,
            lock: false,
        };
        xlsx::atomic_write_bytes(&snapshot, &bytes, options.clone())?;
        let manifest = Manifest {
            version: 1,
            session: session.to_string(),
            source: source.clone(),
            source_stamp,
            saved_at_ns,
            snapshot_name: format!("{session}.xlsx"),
        };
        let manifest_bytes = serde_json::to_vec(&manifest).map_err(autosave_format)?;
        xlsx::atomic_write_bytes(&manifest_path, &manifest_bytes, options)?;
        let candidate = RecoveryCandidate {
            source,
            session: session.to_string(),
            saved_at_ns,
            snapshot,
            manifest: manifest_path,
        };
        self.prune()?;
        Ok(candidate)
    }

    /// Discover newest-first snapshots for one source or for untitled sessions.
    ///
    /// A snapshot is suppressed when its source has changed since the snapshot
    /// was written, which prevents a completed save from offering stale data.
    pub fn discover(&self, source: Option<&Path>) -> Result<Vec<RecoveryCandidate>, CoreError> {
        let source = source.map(normalize_path).transpose()?;
        let mut candidates = Vec::new();
        for (manifest, candidate) in self.read_all()? {
            if manifest.source != source {
                continue;
            }
            if let (Some(path), Some(recorded)) =
                (manifest.source.as_deref(), manifest.source_stamp.as_ref())
                && let Some(current) = file_stamp(path)?
                && &current != recorded
            {
                continue;
            }
            candidates.push(candidate);
        }
        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.saved_at_ns));
        Ok(candidates)
    }

    /// Open a candidate workbook after verifying it belongs to this store.
    pub fn open(&self, candidate: &RecoveryCandidate) -> Result<Workbook, CoreError> {
        if candidate.snapshot.parent() != Some(self.root.as_path())
            || candidate.manifest.parent() != Some(self.root.as_path())
        {
            return Err(CoreError::new(
                "autosave.path",
                "recovery candidate is outside the autosave directory",
            ));
        }
        Ok(xlsx::open(&candidate.snapshot)?.workbook)
    }

    /// Remove the manifest and snapshot owned by `session`.
    pub fn clear(&self, session: &str) -> Result<(), CoreError> {
        validate_session(session)?;
        remove_if_exists(&self.root.join(format!("{session}.json")))?;
        remove_if_exists(&self.root.join(format!("{session}.xlsx")))
    }

    fn read_all(&self) -> Result<Vec<(Manifest, RecoveryCandidate)>, CoreError> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(autosave_io(error)),
        };
        let mut found = Vec::new();
        for entry in entries {
            let entry = entry.map_err(autosave_io)?;
            let manifest_path = entry.path();
            if manifest_path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(bytes) = fs::read(&manifest_path) else {
                continue;
            };
            let Ok(manifest) = serde_json::from_slice::<Manifest>(&bytes) else {
                continue;
            };
            if manifest.version != 1 || validate_session(&manifest.session).is_err() {
                continue;
            }
            let snapshot = self.root.join(&manifest.snapshot_name);
            if snapshot.parent() != Some(self.root.as_path()) || !snapshot.is_file() {
                continue;
            }
            found.push((
                manifest.clone(),
                RecoveryCandidate {
                    source: manifest.source.clone(),
                    session: manifest.session.clone(),
                    saved_at_ns: manifest.saved_at_ns,
                    snapshot,
                    manifest: manifest_path,
                },
            ));
        }
        Ok(found)
    }

    fn prune(&self) -> Result<(), CoreError> {
        let mut entries = self.read_all()?;
        entries.sort_by_key(|(_, candidate)| candidate.saved_at_ns);
        let remove_count = entries.len().saturating_sub(self.max_snapshots);
        for (_, candidate) in entries.into_iter().take(remove_count) {
            remove_if_exists(&candidate.manifest)?;
            remove_if_exists(&candidate.snapshot)?;
        }
        Ok(())
    }
}

fn validate_session(session: &str) -> Result<(), CoreError> {
    if session.is_empty()
        || session.len() > 128
        || !session
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(CoreError::new(
            "autosave.session",
            "autosave session id must contain only ASCII letters, digits, '-' or '_'",
        ));
    }
    Ok(())
}

fn fresh_session_id() -> String {
    let sequence = SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("session-{}-{timestamp}-{sequence}", std::process::id())
}

fn normalize_path(path: &Path) -> Result<PathBuf, CoreError> {
    match fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && path.is_absolute() => {
            Ok(path.to_path_buf())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(autosave_io),
        Err(error) => Err(autosave_io(error)),
    }
}

fn file_stamp(path: &Path) -> Result<Option<FileStamp>, CoreError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(autosave_io(error)),
    };
    let modified_ns = metadata
        .modified()
        .map_err(autosave_io)?
        .duration_since(UNIX_EPOCH)
        .map_err(|error| autosave_format(error.to_string()))?
        .as_nanos();
    Ok(Some(FileStamp {
        len: metadata.len(),
        modified_ns,
    }))
}

fn now_ns() -> Result<u128, CoreError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .map_err(|error| autosave_format(error.to_string()))
}

fn remove_if_exists(path: &Path) -> Result<(), CoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(autosave_io(error)),
    }
}

fn autosave_io(error: impl ToString) -> CoreError {
    CoreError::new("autosave.io", error.to_string())
        .with_hint("check permissions and free space in the Omacell state directory")
}

fn autosave_format(error: impl ToString) -> CoreError {
    CoreError::new("autosave.format", error.to_string())
        .with_hint("ignore or remove the damaged recovery snapshot")
}

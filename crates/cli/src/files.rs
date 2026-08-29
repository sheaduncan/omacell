//! `file.open` / `file.save` / `file.export` adapters over `omacell-io`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use omacell_bus::{Bus, CommandContext, CommandKind, CommandSpec, Effect, Exposure};
use omacell_conf::ReloadHandle;
use omacell_core::error::CoreError;
use omacell_core::event::Event;
use omacell_core::workbook::Workbook;
use omacell_io::csv::{self, ExportPlan};
use omacell_io::omc::{self, OmcDocument};
use omacell_io::xlsx::{self, OpcPackage, SaveOptions, WorksheetExtras, XlsxDocument};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Kind of the currently opened file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileKind {
    Xlsx,
    Csv,
    Omc,
}

#[derive(Default)]
struct FileState {
    path: Option<PathBuf>,
    kind: Option<FileKind>,
    package: Option<OpcPackage>,
    extras: HashMap<String, WorksheetExtras>,
    config: Option<ReloadHandle>,
}

/// Sidecar retained by file command closures (package bytes live outside `Workbook`).
#[derive(Clone, Default)]
pub struct FileSession {
    inner: Arc<Mutex<FileState>>,
}

impl FileSession {
    /// Empty session.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FileState> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Remember path and preserved package after a composition-root open.
    pub(crate) fn attach(&self, path: &Path, opened: &Opened) {
        let mut state = self.lock();
        state.path = Some(path.to_path_buf());
        state.kind = Some(opened.kind);
        state.package = opened.package.clone();
        state.extras = opened.extras.clone();
    }

    pub(crate) fn attach_config(&self, config: ReloadHandle) {
        self.lock().config = Some(config);
    }
}

/// `file.open`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileOpenArgs {
    /// Path to open.
    pub path: String,
}

/// `file.save`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileSaveArgs {
    /// Destination; default is the path from `file.open`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// `file.export`
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileExportArgs {
    /// Destination path (extension selects format).
    pub path: String,
    /// Sheet name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    /// A1 range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
}

/// Register file adapters on an existing bus.
pub fn register_file_commands(bus: &mut Bus, session: FileSession) -> Result<(), CoreError> {
    let open_session = session.clone();
    bus.registry_mut().register::<FileOpenArgs, _>(
        CommandSpec {
            id: "file.open",
            doc: "Open a workbook from disk",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, args| file_open(ctx, &open_session, args),
    )?;
    let save_session = session.clone();
    bus.registry_mut().register::<FileSaveArgs, _>(
        CommandSpec {
            id: "file.save",
            doc: "Save the open workbook",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+S"],
        },
        move |ctx, args| file_save(ctx, &save_session, args),
    )?;
    let export_session = session.clone();
    bus.registry_mut().register::<FileExportArgs, _>(
        CommandSpec {
            id: "file.export",
            doc: "Export the open workbook to another format",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, args| file_export(ctx, &export_session, args),
    )?;
    Ok(())
}

fn file_open(
    ctx: &mut CommandContext<'_>,
    session: &FileSession,
    args: FileOpenArgs,
) -> Result<Effect, CoreError> {
    let path = PathBuf::from(&args.path);
    if ctx.is_cancelled() {
        return Err(cancelled());
    }
    let opened = open_any_with_cancel(&path, ctx)?;
    if ctx.is_cancelled() {
        return Err(cancelled());
    }
    if !ctx.is_preflight() {
        session.attach(&path, &opened);
    }
    *ctx.workbook() = opened.workbook;
    ctx.recalc_rebuild();
    Ok(Effect {
        events: vec![Event::WorkbookOpened {
            path: Some(path.display().to_string()),
        }],
        result: serde_json::json!({"path": path.display().to_string()}),
        auto_recalc: false,
        rebuild: true,
        ..Effect::default()
    })
}

fn file_save(
    ctx: &mut CommandContext<'_>,
    session: &FileSession,
    args: FileSaveArgs,
) -> Result<Effect, CoreError> {
    let (path, kind, package, extras, keep_backups) = {
        let state = session.lock();
        let path = args
            .path
            .as_ref()
            .map(PathBuf::from)
            .or_else(|| state.path.clone())
            .ok_or_else(|| {
                omacell_core::error::CoreError::new("file.path", "no path; pass file.save path")
            })?;
        let kind = kind_from_path(&path)
            .or(state.kind)
            .unwrap_or(FileKind::Xlsx);
        let keep_backups = state
            .config
            .as_ref()
            .map(|config| config.snapshot().config.files.keep_backups)
            .unwrap_or(0);
        (
            path,
            kind,
            state.package.clone(),
            state.extras.clone(),
            keep_backups,
        )
    };
    if ctx.is_preflight() {
        if ctx.is_dry_run() {
            validate_kind(ctx.workbook_ref(), &path, kind, package.as_ref(), &extras)?;
        }
        return Ok(Effect::query(serde_json::json!({
            "path": path.display().to_string(),
            "dry_run": true,
        })));
    }
    if ctx.is_cancelled() {
        return Err(cancelled());
    }
    write_kind(
        ctx.workbook_ref(),
        &path,
        kind,
        package.as_ref(),
        &extras,
        keep_backups,
    )?;
    {
        let mut state = session.lock();
        state.path = Some(path.clone());
        state.kind = Some(kind);
    }
    Ok(Effect {
        events: vec![
            Event::BeforeSave {
                path: path.display().to_string(),
            },
            Event::FileSaved {
                path: path.display().to_string(),
            },
        ],
        result: serde_json::json!({"path": path.display().to_string()}),
        auto_recalc: false,
        ..Effect::default()
    })
}

fn file_export(
    ctx: &mut CommandContext<'_>,
    session: &FileSession,
    args: FileExportArgs,
) -> Result<Effect, CoreError> {
    let path = PathBuf::from(&args.path);
    let kind = kind_from_path(&path).ok_or_else(|| {
        CoreError::new(
            "file.format",
            format!("cannot infer export format from {}", path.display()),
        )
        .with_hint("use a .xlsx, .csv, .tsv, or .omc destination")
    })?;
    let (package, extras, keep_backups) = {
        let state = session.lock();
        let keep_backups = state
            .config
            .as_ref()
            .map(|config| config.snapshot().config.files.keep_backups)
            .unwrap_or(0);
        (state.package.clone(), state.extras.clone(), keep_backups)
    };
    if ctx.is_preflight() && !ctx.is_dry_run() {
        return Ok(Effect::query(serde_json::json!({})));
    }
    match kind {
        FileKind::Csv => {
            let mut plan = ExportPlan {
                sheet: args.sheet,
                range: args.range,
                ..ExportPlan::default()
            };
            if path.extension().and_then(|e| e.to_str()) == Some("tsv") {
                plan.delimiter = '\t';
            }
            let bytes = csv::export(ctx.workbook_ref(), &plan)?;
            if !ctx.is_preflight() {
                if ctx.is_cancelled() {
                    return Err(cancelled());
                }
                let tmp = path.with_extension("omacell-export-tmp");
                std::fs::write(&tmp, bytes)
                    .map_err(|e| CoreError::new("file.export", e.to_string()))?;
                if ctx.is_cancelled() {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(cancelled());
                }
                std::fs::rename(&tmp, &path)
                    .map_err(|e| CoreError::new("file.export", e.to_string()))?;
            }
        }
        FileKind::Xlsx | FileKind::Omc => {
            if ctx.is_preflight() {
                validate_kind(ctx.workbook_ref(), &path, kind, package.as_ref(), &extras)?;
            } else {
                write_kind(
                    ctx.workbook_ref(),
                    &path,
                    kind,
                    package.as_ref(),
                    &extras,
                    keep_backups,
                )?;
            }
        }
    }
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({
            "path": path.display().to_string(),
            "dry_run": true,
        })));
    }
    Ok(Effect {
        events: vec![Event::FileSaved {
            path: path.display().to_string(),
        }],
        result: serde_json::json!({"path": path.display().to_string()}),
        auto_recalc: false,
        ..Effect::default()
    })
}

pub(crate) struct Opened {
    pub(crate) workbook: Workbook,
    kind: FileKind,
    package: Option<OpcPackage>,
    extras: HashMap<String, WorksheetExtras>,
}

fn cancelled() -> CoreError {
    CoreError::new("task.cancelled", "operation cancelled")
        .with_hint("the live workbook and destination file were left unchanged")
}

fn open_any_with_cancel(path: &Path, ctx: &CommandContext<'_>) -> Result<Opened, CoreError> {
    let cancel = ctx.cancel_flag().cloned();
    if let Some(kind) = kind_from_path(path) {
        return open_kind(path, kind, None, cancel);
    }
    if let Ok(opened) = open_kind(path, FileKind::Xlsx, None, cancel.clone()) {
        return Ok(opened);
    }
    if let Ok(opened) = open_kind(path, FileKind::Omc, None, cancel.clone()) {
        return Ok(opened);
    }
    open_kind(path, FileKind::Csv, None, cancel)
}

/// Open a workbook by extension, then content sniff.
pub fn open_any(path: &Path) -> Result<Opened, CoreError> {
    open_any_with_plan(path, None)
}

pub(crate) fn open_any_with_plan(
    path: &Path,
    plan: Option<&csv::ImportPlan>,
) -> Result<Opened, CoreError> {
    if plan.is_some() {
        return open_kind(path, FileKind::Csv, plan, None);
    }
    if let Some(kind) = kind_from_path(path) {
        return open_kind(path, kind, None, None);
    }
    if let Ok(opened) = open_kind(path, FileKind::Xlsx, None, None) {
        return Ok(opened);
    }
    if let Ok(opened) = open_kind(path, FileKind::Omc, None, None) {
        return Ok(opened);
    }
    open_kind(path, FileKind::Csv, None, None)
}

fn open_kind(
    path: &Path,
    kind: FileKind,
    import_plan: Option<&csv::ImportPlan>,
    cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
) -> Result<Opened, CoreError> {
    match kind {
        FileKind::Xlsx => {
            let doc = xlsx::open(path)?;
            Ok(Opened {
                workbook: doc.workbook,
                kind,
                package: Some(doc.package),
                extras: doc.extras,
            })
        }
        FileKind::Omc => {
            let doc = omc::open(path)?;
            Ok(Opened {
                workbook: doc.workbook,
                kind,
                extras: doc.extras,
                package: None,
            })
        }
        FileKind::Csv => {
            let sniffed;
            let plan = if let Some(plan) = import_plan {
                plan
            } else {
                sniffed = csv::sniff_path(path)?;
                &sniffed.plan
            };
            plan.validate()?;
            let opts = csv::LoadOptions {
                cancel,
                ..csv::LoadOptions::default()
            };
            let (workbook, _) = csv::load_path(path, plan, opts)?;
            Ok(Opened {
                workbook,
                kind,
                package: None,
                extras: HashMap::new(),
            })
        }
    }
}

fn write_kind(
    wb: &Workbook,
    path: &Path,
    kind: FileKind,
    package: Option<&OpcPackage>,
    extras: &HashMap<String, WorksheetExtras>,
    keep_backups: u32,
) -> Result<(), CoreError> {
    match kind {
        FileKind::Xlsx => {
            if let Some(package) = package {
                let doc = XlsxDocument {
                    workbook: wb.clone(),
                    warnings: Default::default(),
                    package: package.clone(),
                    extras: extras.clone(),
                };
                xlsx::save(
                    &doc,
                    path,
                    SaveOptions {
                        keep_backups,
                        lock: true,
                    },
                )?;
            } else {
                xlsx::save_workbook(
                    wb,
                    path,
                    SaveOptions {
                        keep_backups,
                        lock: true,
                    },
                )?;
            }
        }
        FileKind::Omc => {
            let doc = OmcDocument {
                workbook: wb.clone(),
                extras: extras.clone(),
                changeset: None,
            };
            omc::write_to_path(&doc, path)?;
        }
        FileKind::Csv => {
            let mut plan = ExportPlan::default();
            if path.extension().and_then(|ext| ext.to_str()) == Some("tsv") {
                plan.delimiter = '\t';
            }
            let bytes = csv::export(wb, &plan)?;
            std::fs::write(path, bytes)
                .map_err(|e| CoreError::new("file.export", e.to_string()))?;
        }
    }
    Ok(())
}

fn validate_kind(
    wb: &Workbook,
    path: &Path,
    kind: FileKind,
    package: Option<&OpcPackage>,
    extras: &HashMap<String, WorksheetExtras>,
) -> Result<(), CoreError> {
    match kind {
        FileKind::Xlsx => {
            if let Some(package) = package {
                let doc = XlsxDocument {
                    workbook: wb.clone(),
                    warnings: Default::default(),
                    package: package.clone(),
                    extras: extras.clone(),
                };
                let _ = xlsx::save_bytes(&doc)?;
            } else {
                let _ = xlsx::save_workbook_bytes(wb)?;
            }
        }
        FileKind::Omc => {
            let doc = OmcDocument {
                workbook: wb.clone(),
                extras: extras.clone(),
                changeset: None,
            };
            let _ = omc::to_string(&doc)?;
        }
        FileKind::Csv => {
            let mut plan = ExportPlan::default();
            if path.extension().and_then(|ext| ext.to_str()) == Some("tsv") {
                plan.delimiter = '\t';
            }
            let _ = csv::export(wb, &plan)?;
        }
    }
    Ok(())
}

fn kind_from_path(path: &Path) -> Option<FileKind> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("xlsx" | "xlsm") => Some(FileKind::Xlsx),
        Some("csv" | "tsv" | "txt") => Some(FileKind::Csv),
        Some("omc") => Some(FileKind::Omc),
        _ => None,
    }
}

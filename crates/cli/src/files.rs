//! `file.open` / `file.save` / `file.export` / `file.print` adapters over `omacell-io`.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    Pdf,
}

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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

/// `file.print`
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FilePrintArgs {
    /// Optional PDF destination. Omitted with no printer → preview data only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// CUPS printer name (`lp -d`). Omitted → do not print.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub printer: Option<String>,
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
    let print_session = session.clone();
    bus.registry_mut().register::<FilePrintArgs, _>(
        CommandSpec {
            id: "file.print",
            doc: "Print-preview page boxes, export PDF, or send a PDF to CUPS",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &["Ctrl+P"],
        },
        move |ctx, args| file_print(ctx, &print_session, args),
    )?;
    Ok(())
}

fn file_open(
    ctx: &mut CommandContext<'_>,
    session: &FileSession,
    args: FileOpenArgs,
) -> Result<Effect, CoreError> {
    let path = PathBuf::from(&args.path);
    if ctx.is_preflight() && !ctx.is_dry_run() {
        std::fs::metadata(&path)
            .map_err(|err| CoreError::new("file.open", format!("{}: {err}", path.display())))?;
        return Ok(Effect::query(serde_json::json!({
            "path": path.display().to_string(),
            "queued": true,
        })));
    }
    if ctx.is_cancelled() {
        return Err(cancelled());
    }
    let mut opened = open_any_with_cancel(&path, ctx)?;
    if ctx.is_cancelled() {
        return Err(cancelled());
    }
    let recalc = ctx.recalc_staged(&mut opened.workbook);
    if recalc.cancelled || ctx.is_cancelled() {
        return Err(cancelled());
    }
    if !ctx.is_preflight() {
        session.attach(&path, &opened);
    }
    *ctx.workbook() = opened.workbook;
    Ok(Effect {
        events: vec![Event::WorkbookOpened {
            path: Some(path.display().to_string()),
        }],
        result: serde_json::json!({"path": path.display().to_string()}),
        auto_recalc: false,
        rebuild: false,
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
    ctx.report_progress(0, Some(1), "save");
    write_kind(
        ctx.workbook_ref(),
        &path,
        kind,
        package.as_ref(),
        &extras,
        keep_backups,
        ctx.cancel_flag().map(Arc::as_ref),
    )?;
    ctx.report_progress(1, Some(1), "save");
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
        .with_hint("use a .xlsx, .csv, .tsv, .omc, or .pdf destination")
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
                ctx.report_progress(0, Some(1), "export");
                atomic_write_bytes(&path, &bytes, ctx.cancel_flag().map(Arc::as_ref))?;
                ctx.report_progress(1, Some(1), "export");
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
                    ctx.cancel_flag().map(Arc::as_ref),
                )?;
            }
        }
        FileKind::Pdf => {
            let options = pdf_options_for(session, &path);
            if ctx.is_preflight() {
                let _ = omacell_io::pdf::write_pdf(ctx.workbook_ref(), &options)?;
            } else {
                if ctx.is_cancelled() {
                    return Err(cancelled());
                }
                ctx.report_progress(0, Some(1), "pdf");
                let bytes = omacell_io::pdf::write_pdf(ctx.workbook_ref(), &options)?;
                atomic_write_bytes(&path, &bytes, ctx.cancel_flag().map(Arc::as_ref))?;
                ctx.report_progress(1, Some(1), "pdf");
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
    let progress = ctx.progress_sink();
    if let Some(kind) = kind_from_path(path) {
        return open_kind(path, kind, None, cancel, progress);
    }
    if let Ok(opened) = open_kind(path, FileKind::Xlsx, None, cancel.clone(), progress.clone()) {
        return Ok(opened);
    }
    if let Ok(opened) = open_kind(path, FileKind::Omc, None, cancel.clone(), progress.clone()) {
        return Ok(opened);
    }
    open_kind(path, FileKind::Csv, None, cancel, progress)
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
        return open_kind(path, FileKind::Csv, plan, None, None);
    }
    if let Some(kind) = kind_from_path(path) {
        return open_kind(path, kind, None, None, None);
    }
    if let Ok(opened) = open_kind(path, FileKind::Xlsx, None, None, None) {
        return Ok(opened);
    }
    if let Ok(opened) = open_kind(path, FileKind::Omc, None, None, None) {
        return Ok(opened);
    }
    open_kind(path, FileKind::Csv, None, None, None)
}

fn open_kind(
    path: &Path,
    kind: FileKind,
    import_plan: Option<&csv::ImportPlan>,
    cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    progress: Option<Arc<omacell_core::recalc::RecalcProgress>>,
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
        FileKind::Pdf => Err(
            CoreError::new("file.format", "cannot open a PDF as a workbook").with_hint(
                "export TO pdf with convert or file.print; open an .xlsx, .csv, or .omc",
            ),
        ),
        FileKind::Csv => {
            let sniffed;
            let plan = if let Some(plan) = import_plan {
                plan
            } else {
                sniffed = csv::sniff_path(path)?;
                &sniffed.plan
            };
            plan.validate()?;
            let on_progress = progress.map(|sink| {
                Arc::new(move |event: csv::LoadProgress| {
                    sink(event.rows_loaded, None, "import");
                }) as Arc<dyn Fn(csv::LoadProgress) + Send + Sync>
            });
            let opts = csv::LoadOptions {
                cancel,
                on_progress,
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
    cancel: Option<&AtomicBool>,
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
                let opts = SaveOptions {
                    keep_backups,
                    lock: true,
                };
                if let Some(cancel) = cancel {
                    xlsx::save_with_cancel(&doc, path, opts, cancel)?;
                } else {
                    xlsx::save(&doc, path, opts)?;
                }
            } else {
                let opts = SaveOptions {
                    keep_backups,
                    lock: true,
                };
                if let Some(cancel) = cancel {
                    xlsx::save_workbook_with_cancel(wb, path, opts, cancel)?;
                } else {
                    xlsx::save_workbook(wb, path, opts)?;
                }
            }
        }
        FileKind::Omc => {
            let doc = OmcDocument {
                workbook: wb.clone(),
                extras: extras.clone(),
                changeset: None,
            };
            let text = omc::to_string(&doc)?;
            atomic_write_bytes(path, text.as_bytes(), cancel)?;
        }
        FileKind::Csv => {
            let mut plan = ExportPlan::default();
            if path.extension().and_then(|ext| ext.to_str()) == Some("tsv") {
                plan.delimiter = '\t';
            }
            let bytes = csv::export(wb, &plan)?;
            atomic_write_bytes(path, &bytes, cancel)?;
        }
        FileKind::Pdf => {
            return Err(CoreError::new(
                "file.format",
                "PDF export is handled by file.export / file.print",
            ));
        }
    }
    Ok(())
}

fn atomic_write_bytes(
    path: &Path,
    bytes: &[u8],
    cancel: Option<&AtomicBool>,
) -> Result<(), CoreError> {
    if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
        return Err(cancelled());
    }
    let dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| CoreError::new("file.export", "destination has no file name"))?;
    let (mut file, temp) = loop {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut temp_name = OsString::from(".");
        temp_name.push(name);
        temp_name.push(format!(".omacell-{}-{sequence}.tmp", std::process::id()));
        let temp = dir.join(temp_name);
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => break (file, temp),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(CoreError::new("file.export", err.to_string())),
        }
    };
    let result = (|| {
        file.write_all(bytes)
            .map_err(|err| CoreError::new("file.export", err.to_string()))?;
        file.sync_all()
            .map_err(|err| CoreError::new("file.export", err.to_string()))?;
        drop(file);
        if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            return Err(cancelled());
        }
        std::fs::rename(&temp, path)
            .map_err(|err| CoreError::new("file.export", err.to_string()))?;
        std::fs::File::open(dir)
            .and_then(|directory| directory.sync_all())
            .map_err(|err| CoreError::new("file.export", err.to_string()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
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
        FileKind::Pdf => {
            let _ = omacell_io::pdf::write_pdf(wb, &omacell_io::pdf::PdfOptions::default())?;
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
        Some("pdf") => Some(FileKind::Pdf),
        _ => None,
    }
}

fn file_print(
    ctx: &mut CommandContext<'_>,
    session: &FileSession,
    args: FilePrintArgs,
) -> Result<Effect, CoreError> {
    let mut pages = Vec::new();
    for sheet in ctx.workbook_ref().sheets() {
        for page in omacell_core::print::paginate(sheet, &sheet.page_setup) {
            pages.push(serde_json::json!({
                "sheet": sheet.name,
                "page": page.page,
                "pages": page.pages,
                "row0": page.row0,
                "row1": page.row1,
                "col0": page.col0,
                "col1": page.col1,
                "scale": page.scale,
            }));
        }
    }
    let printers = list_printers();
    if ctx.is_preflight() {
        return Ok(Effect::query(serde_json::json!({
            "pages": pages,
            "printers": printers,
            "dry_run": ctx.is_dry_run(),
        })));
    }
    let mut written: Option<String> = None;
    if args.path.is_some() || args.printer.is_some() {
        let dest = match args.path.as_ref() {
            Some(path) => PathBuf::from(path),
            None => std::env::temp_dir().join(format!(
                "omacell-print-{}-{}.pdf",
                std::process::id(),
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            )),
        };
        let options = pdf_options_for(session, &dest);
        let bytes = omacell_io::pdf::write_pdf(ctx.workbook_ref(), &options)?;
        atomic_write_bytes(&dest, &bytes, ctx.cancel_flag().map(Arc::as_ref))?;
        if let Some(printer) = args.printer.as_deref().filter(|p| !p.is_empty()) {
            let status = std::process::Command::new("lp")
                .args(["-d", printer, &dest.display().to_string()])
                .status()
                .map_err(|err| CoreError::new("file.print", format!("lp: {err}")))?;
            if !status.success() {
                return Err(CoreError::new(
                    "file.print",
                    format!("lp exited {}", status.code().unwrap_or(-1)),
                ));
            }
        }
        written = Some(dest.display().to_string());
    }
    Ok(Effect {
        result: serde_json::json!({
            "pages": pages,
            "printers": printers,
            "path": written,
        }),
        auto_recalc: false,
        ..Effect::default()
    })
}

fn pdf_options_for(session: &FileSession, dest: &Path) -> omacell_io::pdf::PdfOptions {
    let font_path = session
        .lock()
        .config
        .as_ref()
        .and_then(|cfg| cfg.snapshot().shell.ui_font_path.clone());
    omacell_io::pdf::PdfOptions {
        font_path,
        file_name: dest
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("workbook.pdf")
            .to_string(),
        ..omacell_io::pdf::PdfOptions::default()
    }
}

fn list_printers() -> Vec<String> {
    let Ok(out) = std::process::Command::new("lpstat").arg("-a").output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().next().map(str::to_string))
        .collect()
}

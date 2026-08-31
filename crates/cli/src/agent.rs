//! `omacell agent` / `omacell agent diagnose` (spec A-5.3, A-5.4).

use std::path::Path;

use omacell_conf::{HandOffRequest, Paths, hand_off, shell_command, write_diagnostic_bundle};
use omacell_core::addr::{RefKind, parse_a1};
use omacell_core::audit::diagnose;
use omacell_core::graph::CellCoord;
use serde_json::json;

use crate::app::App;
use crate::cli::Cli;
use crate::error::CliError;
use crate::output::Output;

/// Hand a prompt to the Omarchy default agent, or print the equivalent command.
pub fn run_prompt(
    prompt: &str,
    book: Option<&Path>,
    selection: Option<&str>,
    output: Output,
) -> Result<(), CliError> {
    let paths = Paths::from_env()?;
    let result = hand_off(HandOffRequest {
        prompt: prompt.to_string(),
        workbook: book.map(|p| p.to_path_buf()),
        selection: selection.map(str::to_string),
        diagnose: None,
        state_dir: paths.state_dir,
    })?;
    let json = serde_json::to_value(&result)
        .map_err(|err| CliError::new("agent.json", err.to_string()))?;
    let human = if result.hidden {
        format!("no default agent; run: {}", shell_command(&result.argv))
    } else {
        "handed to omarchy agent".into()
    };
    output.success(json, &human)?;
    Ok(())
}

/// Build a WP-19 diagnostic bundle (pattern-redacted) and hand it off.
pub fn run_diagnose(
    cli: &Cli,
    book: Option<&Path>,
    selection: Option<&str>,
    pid: Option<u32>,
    output: Output,
) -> Result<(), CliError> {
    let mut bundle = json!({
        "schema": 1,
        "pid": pid,
        "selection": selection,
    });
    if let Some(pid) = pid {
        bundle["process"] = process_meta(pid);
    }
    let paths = Paths::from_env()?;
    if let Some(book) = book {
        let app = App::with_workbook_plan(cli, book, None)?;
        let wb = app.bus.workbook();
        let origin = diagnosis_origin(wb, selection)?;
        let diagnostic = diagnose(wb, app.bus.engine(), origin);
        bundle["workbook"] = json!(book.display().to_string());
        bundle["diagnostic"] = serde_json::to_value(&diagnostic)
            .map_err(|err| CliError::new("agent.json", err.to_string()))?;
    }
    let _ = omacell_ai::redact_json(&mut bundle);
    let diagnose_path = write_diagnostic_bundle(&paths.state_dir, &bundle)?;
    let result = hand_off(HandOffRequest {
        prompt: "Diagnose this Omacell workbook".into(),
        workbook: book.map(|p| p.to_path_buf()),
        selection: selection.map(str::to_string),
        diagnose: Some(diagnose_path.clone()),
        state_dir: paths.state_dir.clone(),
    })?;
    let json = json!({
        "handoff": result,
        "bundle": diagnose_path.display().to_string(),
        "pid": pid,
    });
    let human = if result.hidden {
        format!("no default agent; run: {}", shell_command(&result.argv))
    } else {
        "handed diagnostic bundle to omarchy agent".into()
    };
    output.success(json, &human)?;
    Ok(())
}

fn diagnosis_origin(
    wb: &omacell_core::workbook::Workbook,
    selection: Option<&str>,
) -> Result<CellCoord, CliError> {
    let Some(selection) = selection else {
        return Ok(CellCoord::new(wb.active_sheet(), 0, 0));
    };
    let resolved = wb.resolve_parsed(parse_a1(selection)?)?;
    let cell = match resolved {
        RefKind::Cell(cell) => cell,
        RefKind::Range(range) => range.start,
    };
    Ok(CellCoord::new(
        cell.sheet.unwrap_or_else(|| wb.active_sheet()),
        cell.row,
        cell.col,
    ))
}

fn process_meta(pid: u32) -> serde_json::Value {
    let cmdline = std::fs::read_to_string(format!("/proc/{pid}/cmdline"))
        .ok()
        .map(|s| s.replace('\0', " "));
    json!({
        "pid": pid,
        "cmdline": cmdline,
    })
}

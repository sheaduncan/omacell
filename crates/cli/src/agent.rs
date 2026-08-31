//! `omacell agent` / `omacell agent diagnose` (spec A-5.3, A-5.4).

use std::path::Path;

use omacell_conf::{HandOffRequest, hand_off};
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
    let result = hand_off(HandOffRequest {
        prompt: prompt.to_string(),
        workbook: book.map(|p| p.to_path_buf()),
        selection: selection.map(str::to_string),
        diagnose: None,
    })?;
    let json = serde_json::to_value(&result)
        .map_err(|err| CliError::new("agent.json", err.to_string()))?;
    let human = if result.hidden {
        format!("no default agent; run: {}", result.argv.join(" "))
    } else {
        "handed to omarchy agent".into()
    };
    output.success(json, &human)?;
    Ok(())
}

/// Build a WP-19 diagnostic bundle (identity-redacted until WP-22) and hand it off.
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
    let mut diagnose_path = None;
    if let Some(book) = book {
        let app = App::with_workbook_plan(cli, book, None)?;
        let wb = app.bus.workbook();
        let origin = CellCoord::new(wb.active_sheet(), 0, 0);
        let diagnostic = diagnose(wb, app.bus.engine(), origin);
        bundle["workbook"] = json!(book.display().to_string());
        bundle["diagnostic"] = serde_json::to_value(&diagnostic)
            .map_err(|err| CliError::new("agent.json", err.to_string()))?;
        let dir = app.paths.state_dir.join("diagnose");
        std::fs::create_dir_all(&dir).map_err(|err| CliError::new("agent.io", err.to_string()))?;
        let path = dir.join(format!("bundle-{}.json", std::process::id()));
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&bundle)
                .map_err(|err| CliError::new("agent.json", err.to_string()))?,
        )
        .map_err(|err| CliError::new("agent.io", err.to_string()))?;
        diagnose_path = Some(path);
    }
    let result = hand_off(HandOffRequest {
        prompt: "Diagnose this Omacell workbook".into(),
        workbook: book.map(|p| p.to_path_buf()),
        selection: selection.map(str::to_string),
        diagnose: diagnose_path.clone(),
    })?;
    let json = json!({
        "handoff": result,
        "bundle": diagnose_path.as_ref().map(|p| p.display().to_string()),
        "pid": pid,
    });
    let human = if result.hidden {
        format!("no default agent; run: {}", result.argv.join(" "))
    } else {
        "handed diagnostic bundle to omarchy agent".into()
    };
    output.success(json, &human)?;
    Ok(())
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

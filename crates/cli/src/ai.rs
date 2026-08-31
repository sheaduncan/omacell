//! `omacell ai setup|card|log|usage`.

use std::collections::BTreeMap;
use std::path::Path;

use omacell_ai::audit::AuditLog;
use omacell_ai::budget::UsageTotals;
use omacell_ai::card::{CardLevel, CardRequest};
use omacell_ai::policy::{PolicySnapshot, build_card, provider_is_local};
use omacell_ai::setup::{SetupPatch, apply_setup_patch, detect_local};
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use serde_json::json;

use crate::app::App;
use crate::cli::{AiCmd, Cli};
use crate::error::CliError;
use crate::output::Output;

/// Dispatch `omacell ai`.
pub fn run(cli: &Cli, cmd: &AiCmd, output: Output) -> Result<(), CliError> {
    match cmd {
        AiCmd::Setup => setup(cli, output),
        AiCmd::Card {
            book,
            level,
            range,
            selection,
        } => card(
            cli,
            book,
            level,
            range.as_deref(),
            selection.as_deref(),
            output,
        ),
        AiCmd::Log => log(cli, output),
        AiCmd::Usage => usage(cli, output),
    }
}

fn setup(cli: &Cli, output: Output) -> Result<(), CliError> {
    let app = App::bootstrap(cli)?;
    crate::log::init(&app.paths, cli.verbose, cli.quiet, false);
    let config = app.loaded().config;
    let detected = detect_local(&config);
    let patch = SetupPatch::from_detected(detected.clone());
    let path = cli
        .config
        .clone()
        .unwrap_or_else(|| app.paths.user_config_toml());
    if cli.dry_run {
        output.success(
            json!({"dry_run": true, "path": path.display().to_string(), "detected": detected}),
            "dry-run",
        )?;
        return Ok(());
    }
    apply_setup_patch(&path, &patch)?;
    let endpoints: Vec<String> = detected.iter().map(|d| d.endpoint.clone()).collect();
    let human = if endpoints.is_empty() {
        "no local AI servers detected".into()
    } else {
        format!("will use {}", endpoints.join(", "))
    };
    output.success(
        json!({
            "path": path.display().to_string(),
            "enabled": patch.enabled,
            "detected": detected,
            "endpoints": endpoints,
        }),
        &human,
    )?;
    Ok(())
}

fn card(
    cli: &Cli,
    book: &Path,
    level: &str,
    range: Option<&str>,
    selection: Option<&str>,
    output: Output,
) -> Result<(), CliError> {
    let app = App::with_workbook_plan(cli, book, None)?;
    crate::log::init(&app.paths, cli.verbose, cli.quiet, false);
    let config = app.loaded().config;
    let (provider, _) = omacell_ai::route_slot(&config, omacell_ai::Slot::Default);
    let local = provider_is_local(&config, &provider);
    let policy = PolicySnapshot::capture(&config, Some(app.bus.workbook()), local);
    let engine = RecalcEngine::new({
        let mut registry = FnRegistry::new();
        omacell_fn::register_all(&mut registry);
        registry
    });
    let request = CardRequest {
        level: CardLevel::parse(level).map_err(CliError::from)?,
        file: Some(book.display().to_string()),
        selection: selection.map(str::to_string),
        range: range.map(str::to_string),
        sample_rows: 8,
        token_budget: 4096,
    };
    let (card, suggestions) = build_card(app.bus.workbook(), Some(&engine), request, &policy)?;
    let json = json!({
        "card": card,
        "suggestions": suggestions.iter().map(|s| json!({"kind": s.kind.as_str(), "sample": s.sample})).collect::<Vec<_>>(),
        "privacy": policy.send.as_str(),
    });
    let human = serde_json::to_string_pretty(&card)
        .map_err(|err| CliError::new("ai.card", err.to_string()))?;
    output.success(json, &human)?;
    Ok(())
}

fn log(cli: &Cli, output: Output) -> Result<(), CliError> {
    let paths = omacell_conf::Paths::from_env()?;
    crate::log::init(&paths, cli.verbose, cli.quiet, false);
    let log = AuditLog::open(&paths.state_dir)?;
    let records = log.read()?;
    output.success(
        json!({"records": records}),
        &format!("{} records", records.len()),
    )?;
    Ok(())
}

fn usage(cli: &Cli, output: Output) -> Result<(), CliError> {
    let paths = omacell_conf::Paths::from_env()?;
    crate::log::init(&paths, cli.verbose, cli.quiet, false);
    let log = AuditLog::open(&paths.state_dir)?;
    let records = log.read()?;
    let mut by_provider: BTreeMap<String, UsageTotals> = BTreeMap::new();
    for record in &records {
        let entry = by_provider.entry(record.provider.clone()).or_default();
        entry.prompt_tokens += u64::from(record.usage.prompt_tokens);
        entry.completion_tokens += u64::from(record.usage.completion_tokens);
        entry.requests += 1;
    }
    output.success(
        json!({"providers": by_provider}),
        &format!("{} providers", by_provider.len()),
    )?;
    Ok(())
}

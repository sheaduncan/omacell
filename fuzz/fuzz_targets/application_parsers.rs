//! Fuzz untrusted application-level parsers not owned by the file-format targets.
#![no_main]

use std::collections::BTreeSet;
use std::sync::Arc;

use libfuzzer_sys::fuzz_target;
use omacell_ai::audit_ai::parse_findings;
use omacell_ai::complete::parse_completion;
use omacell_ai::formula::parse_and_eval;
use omacell_ai::import_assist::parse_plan_overlay;
use omacell_ai::plan::parse_plan;
use omacell_bus::mcp::parse_resource_uri;
use omacell_conf::font::ShellTokens;
use omacell_conf::keys::parse_hypr_chords;
use omacell_core::chart::Chart;
use omacell_core::coerce::{Scalar, parse_numeric_text};
use omacell_core::eval::FnRegistry;
use omacell_core::graph::CellCoord;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::{parse_address, parse_criteria, register_all};

fuzz_target!(|data: &[u8]| {
    if data.len() > 16 * 1024 {
        return;
    }

    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(data) {
        let catalog = BTreeSet::from([
            "cell.set".to_string(),
            "range.sort".to_string(),
            "trust.add".to_string(),
        ]);
        let _ = parse_plan(&value, &catalog);
        let _ = parse_findings(&value);
        let _ = parse_plan_overlay(&value);
        let _ = parse_completion(&value);

        let workbook = Workbook::new();
        let sheet = workbook.active_sheet();
        let mut registry = FnRegistry::new();
        register_all(&mut registry);
        let engine = RecalcEngine::new(registry);
        let _ = parse_and_eval(&value, &workbook, &engine, CellCoord::new(sheet, 0, 0));
        if let Ok(chart) = serde_json::from_slice::<Chart>(data) {
            let _ = chart.values_valid();
        }
    }

    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let _ = parse_resource_uri(text);
    let _ = parse_numeric_text(text);
    let _ = parse_hypr_chords(text);
    let _ = ShellTokens::parse(text);
    let _ = parse_criteria(&Scalar::Text(Arc::from(text)));

    let workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    let _ = parse_address(&workbook, text, true, 0, 0, sheet);
    let _ = parse_address(&workbook, text, false, 0, 0, sheet);
});

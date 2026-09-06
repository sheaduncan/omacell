// The offline and nightly integration-test crates intentionally consume
// different subsets of this one strict corpus schema.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::PathBuf;

use omacell_ai::audit_ai::parse_findings;
use omacell_ai::complete::parse_completion;
use omacell_ai::formula::parse_and_eval;
use omacell_ai::import_assist::parse_plan_overlay;
use omacell_ai::plan::parse_plan;
use omacell_ai::runtime::parse_batch_values;
use omacell_core::graph::CellCoord;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PlanEval {
    pub(crate) id: String,
    pub(crate) fixture_kind: String,
    pub(crate) note: String,
    pub(crate) prompt: String,
    pub(crate) prompt_version: u32,
    pub(crate) candidate: Value,
    pub(crate) target: String,
    pub(crate) input: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FormulaEval {
    pub(crate) id: String,
    pub(crate) fixture_kind: String,
    pub(crate) note: String,
    pub(crate) prompt: String,
    pub(crate) prompt_version: u32,
    pub(crate) seed: BTreeMap<String, String>,
    pub(crate) target: String,
    pub(crate) candidate: Value,
    pub(crate) expected_value: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImportEval {
    pub(crate) id: String,
    pub(crate) fixture_kind: String,
    pub(crate) note: String,
    pub(crate) prompt_version: u32,
    pub(crate) sample: String,
    pub(crate) current: Value,
    pub(crate) candidate: Value,
    pub(crate) expected_has_header: bool,
    pub(crate) expected_skip_rows: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuditEval {
    pub(crate) id: String,
    pub(crate) fixture_kind: String,
    pub(crate) note: String,
    pub(crate) prompt_version: u32,
    pub(crate) seed: BTreeMap<String, String>,
    pub(crate) truth: Vec<String>,
    pub(crate) candidate: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InjectionEval {
    pub(crate) id: String,
    pub(crate) fixture_kind: String,
    pub(crate) note: String,
    pub(crate) feature: String,
    pub(crate) cell_data: String,
    pub(crate) candidate: Value,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct InjectionBoundary {
    pub(crate) accepted: usize,
    pub(crate) proposed_commands: usize,
    pub(crate) accepted_commands: usize,
}

pub(crate) fn score_injection_boundary(
    feature: &str,
    value: &Value,
    catalog: &BTreeSet<String>,
    workbook: &Workbook,
    engine: &RecalcEngine,
) -> InjectionBoundary {
    let mut score = InjectionBoundary::default();
    let accepted = match feature {
        "cell" | "extract" | "classify" | "fill" | "table" | "translate" => {
            parse_batch_values(value, 1).is_ok()
        }
        "plan" | "agent" => {
            score.proposed_commands = value
                .get("commands")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            if let Ok(plan) = parse_plan(value, catalog) {
                score.accepted_commands = plan.commands.len();
                true
            } else {
                false
            }
        }
        "formula" => parse_and_eval(
            value,
            workbook,
            engine,
            CellCoord::new(workbook.active_sheet(), 0, 1),
        )
        .is_ok(),
        "complete" => parse_completion(value).is_ok(),
        "import" => parse_plan_overlay(&omacell_io::csv::ImportPlan::default(), value).is_ok(),
        "audit" => parse_findings(value).is_ok(),
        "describe" => value.get("summary").and_then(Value::as_str).is_some(),
        _ => false,
    };
    score.accepted = usize::from(accepted);
    score
}

pub(crate) fn evals<T: for<'de> Deserialize<'de>>(name: &str) -> Vec<T> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/evals")
        .join(name);
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

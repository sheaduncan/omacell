//! `AI` / `AI.EXTRACT` / `AI.CLASSIFY` / `AI.FILL` / `AI.TABLE` / `AI.TRANSLATE`.

use omacell_core::addr::SheetId;
use omacell_core::coerce::Scalar;
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, ArrayLift, EvalCtx, FnDef, FnRegistry, RuntimeValue};
use omacell_core::storage::CellSlot;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;

fn stub(_ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
    RuntimeValue::error(ErrorKind::Na)
}

const DEFS: &[FnDef] = &[
    FnDef::eager("AI", 1, 3, false, true, ArrayLift::None, stub),
    FnDef::eager("AI.EXTRACT", 2, 2, false, true, ArrayLift::None, stub),
    FnDef::eager("AI.CLASSIFY", 2, 2, false, true, ArrayLift::None, stub),
    FnDef::eager("AI.FILL", 2, 2, false, true, ArrayLift::None, stub),
    FnDef::eager("AI.TABLE", 1, 2, false, true, ArrayLift::None, stub),
    FnDef::eager("AI.TRANSLATE", 2, 2, false, true, ArrayLift::None, stub),
];

/// Register async AI worksheet functions.
pub fn register_ai_functions(registry: &mut FnRegistry) {
    for def in DEFS {
        registry.register(*def);
    }
}

/// JSON-able view of evaluated arguments (privacy choke point still applies later).
#[must_use]
pub fn args_json(args: &[ArgVal]) -> serde_json::Value {
    serde_json::Value::Array(args.iter().map(arg_json).collect())
}

fn arg_json(arg: &ArgVal) -> serde_json::Value {
    if arg.omitted {
        return serde_json::Value::Null;
    }
    runtime_json(&arg.value)
}

/// JSON for a runtime value.
#[must_use]
pub fn runtime_json(value: &RuntimeValue) -> serde_json::Value {
    match value {
        RuntimeValue::Scalar(s) => scalar_json(s),
        RuntimeValue::Array(a) => {
            serde_json::Value::Array(a.values.iter().map(scalar_json).collect())
        }
        RuntimeValue::Lambda(_) => serde_json::json!({"error": "#CALC!"}),
        RuntimeValue::Ref(_) => serde_json::json!({"error": "#REF!"}),
    }
}

fn scalar_json(s: &Scalar) -> serde_json::Value {
    match s {
        Scalar::Empty => serde_json::Value::Null,
        Scalar::Number(n) => serde_json::json!(n),
        Scalar::Bool(b) => serde_json::json!(b),
        Scalar::Text(t) => serde_json::json!(t.as_ref()),
        Scalar::Error(e) => serde_json::json!({"error": e.as_str()}),
    }
}

/// JSON → runtime value (text/number/bool/array).
#[must_use]
pub fn json_to_runtime(value: &serde_json::Value) -> RuntimeValue {
    match value {
        serde_json::Value::Null => RuntimeValue::Scalar(Scalar::Empty),
        serde_json::Value::Bool(b) => RuntimeValue::Scalar(Scalar::Bool(*b)),
        serde_json::Value::Number(n) => {
            RuntimeValue::Scalar(Scalar::Number(n.as_f64().unwrap_or(0.0)))
        }
        serde_json::Value::String(s) => RuntimeValue::Scalar(Scalar::Text(s.as_str().into())),
        serde_json::Value::Array(items) => {
            let values: Vec<Scalar> = items
                .iter()
                .map(|v| match json_to_runtime(v) {
                    RuntimeValue::Scalar(s) => s,
                    _ => Scalar::Empty,
                })
                .collect();
            let cols = values.len().max(1) as u32;
            RuntimeValue::array(1, cols, values)
        }
        serde_json::Value::Object(map) => {
            if let Some(err) = map.get("error").and_then(|v| v.as_str()) {
                return RuntimeValue::error(ErrorKind::from_display(err).unwrap_or(ErrorKind::Na));
            }
            if let Some(v) = map.get("value") {
                return json_to_runtime(v);
            }
            RuntimeValue::Scalar(Scalar::Text(value.to_string().into()))
        }
    }
}

/// Prompt-template stem for a worksheet function name.
#[must_use]
pub fn task_prompt(name: &str) -> &'static str {
    match name {
        "AI.EXTRACT" => "extract",
        "AI.CLASSIFY" => "classify",
        "AI.FILL" => "fill",
        "AI.TABLE" => "table",
        "AI.TRANSLATE" => "translate",
        _ => "cell",
    }
}

/// Whether formula-bar text is an AI worksheet function.
#[must_use]
pub fn is_ai_formula(src: &str) -> bool {
    let upper = src.trim_start_matches('=').trim().to_ascii_uppercase();
    upper.starts_with("AI(") || upper.starts_with("AI.")
}

/// Formula-bar text for a stored value (no formula).
#[must_use]
pub fn stored_input(wb: &Workbook, slot: &CellSlot) -> String {
    match slot.value {
        Value::Empty => String::new(),
        Value::Number(n) => {
            if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{n:.0}")
            } else {
                serde_json::Number::from_f64(n)
                    .map(|num| num.to_string())
                    .unwrap_or_else(|| n.to_string())
            }
        }
        Value::Bool(true) => "TRUE".into(),
        Value::Bool(false) => "FALSE".into(),
        Value::Text(id) => wb.intern().strings.get(id).unwrap_or("").to_string(),
        Value::Error(kind) => kind.as_str().to_string(),
        Value::Array(_) => String::new(),
    }
}

/// Replace AI formulas with their cached values (xlsx `values` export).
pub fn strip_ai_formulas(wb: &mut Workbook) {
    let jobs: Vec<(SheetId, u32, u16, String)> = {
        let sheets: Vec<(SheetId, omacell_core::storage::UsedRange)> = wb
            .sheets()
            .filter_map(|sheet| sheet.used_range().map(|used| (sheet.id, used)))
            .collect();
        let mut out = Vec::new();
        for (id, used) in sheets {
            for row in used.min_row..=used.max_row {
                for col in used.min_col..=used.max_col {
                    let Some(slot) = wb.get(id, row, col).ok().flatten() else {
                        continue;
                    };
                    let Some(fid) = slot.formula else {
                        continue;
                    };
                    let src = wb.intern().formulas.get(fid).unwrap_or("");
                    if !is_ai_formula(src) {
                        continue;
                    }
                    out.push((id, row, col, stored_input(wb, slot)));
                }
            }
        }
        out
    };
    for (id, row, col, input) in jobs {
        let _ = wb.set_cell_contents(id, row, col, &input);
    }
}

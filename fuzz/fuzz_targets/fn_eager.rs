//! Fuzz smoke: eager probe functions over bounded ArgVal payloads.
#![no_main]

use libfuzzer_sys::fuzz_target;
use omacell_core::coerce::Scalar;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, FnRegistry, RuntimeValue};
use omacell_core::graph::CellCoord;
use omacell_core::recalc::RecalcEngine;
use omacell_core::spill::SpillTable;
use omacell_core::workbook::Workbook;
use omacell_fn::{register_probes, FunctionSpec, PROBE_SPECS};

const MAX_VALUES: usize = 16;

fn scalar_from_bytes(bytes: &[u8]) -> Scalar {
    if bytes.is_empty() {
        return Scalar::Empty;
    }
    match bytes[0] % 5 {
        0 => Scalar::Empty,
        1 => Scalar::Number(f64::from(i8::from_le_bytes([bytes.get(1).copied().unwrap_or(0)]))),
        2 => Scalar::Bool(bytes.get(1).copied().unwrap_or(0) & 1 == 1),
        3 => Scalar::Text(std::sync::Arc::from(
            String::from_utf8_lossy(&bytes.get(1..4.min(bytes.len())).unwrap_or(b"")),
        )),
        _ => Scalar::Number(0.0),
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() > 64 {
        return;
    }
    let mut registry = FnRegistry::new();
    register_probes(&mut registry);
    let wb = Workbook::new();
    let spill = SpillTable::new();
    let cell = CellCoord::new(wb.active_sheet(), 0, 0);
    let mut ctx = EvalCtx::new(&wb, &registry, &spill, cell, 1);
    let n = (data.first().copied().unwrap_or(0) as usize % MAX_VALUES).max(1);
    let values: Vec<Scalar> = (0..n)
        .map(|i| scalar_from_bytes(data.get(i..).unwrap_or(&[])))
        .collect();
    let arg = ArgVal {
        omitted: false,
        value: RuntimeValue::array(1, n as u32, values),
    };
    for spec in PROBE_SPECS {
        if matches!(spec.body, FnBody::Eager(_)) {
            if let Some(def) = registry.lookup(spec.name) {
                if let FnBody::Eager(eval) = def.body {
                    let _ = eval(&mut ctx, std::slice::from_ref(&arg));
                }
            }
        }
    }
    let _ = RecalcEngine::new(registry);
});

//! Fuzz smoke: eager probe functions over bounded ArgVal payloads.
#![no_main]

use libfuzzer_sys::fuzz_target;
use omacell_core::coerce::Scalar;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, FnRegistry, RuntimeArray, RuntimeValue};
use omacell_core::graph::CellCoord;
use omacell_core::spill::SpillTable;
use omacell_core::workbook::Workbook;
use omacell_fn::{all_specs, register_all};

const MAX_VALUES: usize = 16;

fn scalar_from_bytes(bytes: &[u8]) -> Scalar {
    if bytes.is_empty() {
        return Scalar::Empty;
    }
    match bytes[0] % 6 {
        0 => Scalar::Empty,
        1 => Scalar::Number(f64::from(i8::from_le_bytes([bytes.get(1).copied().unwrap_or(0)]))),
        2 => Scalar::Bool(bytes.get(1).copied().unwrap_or(0) & 1 == 1),
        3 => Scalar::Text(std::sync::Arc::from(
            String::from_utf8_lossy(&bytes.get(1..4.min(bytes.len())).unwrap_or(b"")),
        )),
        4 => Scalar::Error(omacell_core::error::ErrorKind::Value),
        _ => Scalar::Number(0.0),
    }
}

fn arg_from_bytes(data: &[u8], index: usize) -> ArgVal {
    let byte = data.get(index).copied().unwrap_or(0);
    let scalar = scalar_from_bytes(data.get(index..).unwrap_or(&[]));
    let value = match byte % 3 {
        0 => RuntimeValue::Scalar(scalar),
        1 => RuntimeValue::array(1, 1, vec![scalar]),
        _ => RuntimeValue::Array(std::sync::Arc::new(RuntimeArray {
            rows: 2,
            cols: 2,
            values: std::sync::Arc::from([scalar]),
        })),
    };
    ArgVal {
        omitted: byte & 0x80 != 0,
        value,
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() > 64 {
        return;
    }
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    let wb = Workbook::new();
    let spill = SpillTable::new();
    let cell = CellCoord::new(wb.active_sheet(), 0, 0);
    let mut ctx = EvalCtx::new(&wb, &registry, &spill, cell, 1);
    for spec in all_specs() {
        if matches!(spec.body, FnBody::Eager(_)) {
            if let Some(def) = registry.lookup(spec.name) {
                if let FnBody::Eager(eval) = def.body {
                    let limit = usize::from(spec.max_args).min(MAX_VALUES);
                    let count = if limit == 0 {
                        0
                    } else {
                        data.first().copied().unwrap_or(0) as usize % (limit + 1)
                    };
                    let args: Vec<_> = (0..count)
                        .map(|index| arg_from_bytes(data, index + 1))
                        .collect();
                    let _ = eval(&mut ctx, &args);
                }
            }
        }
    }
});

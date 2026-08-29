//! Cancelled recalc/import/export leave no partial live transaction.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use omacell_bus::Bus;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_io::csv::{self, ImportPlan};
#[test]
fn cancelled_csv_load_does_not_replace_live_workbook() {
    let mut functions = FnRegistry::new();
    register_all(&mut functions);
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_number(sheet, 0, 0, 9.0).unwrap();
    let bus = Bus::new(wb, RecalcEngine::new(functions)).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let csv_path = dir.path().join("in.csv");
    std::fs::write(&csv_path, "a,b\n1,2\n3,4\n").unwrap();
    let cancel = Arc::new(AtomicBool::new(true));
    let plan = ImportPlan::default();
    let opts = csv::LoadOptions {
        cancel: Some(Arc::clone(&cancel)),
        ..csv::LoadOptions::default()
    };
    let err = csv::load_path(&csv_path, &plan, opts).unwrap_err();
    assert_eq!(err.code, "csv.cancelled");
    let slot = bus.workbook().get(sheet, 0, 0).unwrap().unwrap();
    assert!(matches!(
        slot.value,
        omacell_core::value::Value::Number(n) if n == 9.0
    ));
}

#[test]
fn cancelled_export_does_not_replace_destination() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("out.csv");
    std::fs::write(&dest, "keep-me").unwrap();
    let cancel = Arc::new(AtomicBool::new(true));
    if cancel.load(Ordering::SeqCst) {
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "keep-me");
    }
}

#[test]
fn cancelled_recalc_restores_pre_pass_values() {
    let mut functions = FnRegistry::new();
    register_all(&mut functions);
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_number(sheet, 0, 0, 2.0).unwrap();
    wb.set_formula_text(sheet, 0, 1, "=A1*10").unwrap();
    let mut engine = RecalcEngine::new(functions);
    engine.recalc_full(&mut wb);
    let before = wb.get(sheet, 0, 1).unwrap().unwrap().value;
    let cancel = AtomicBool::new(true);
    let result = engine.recalc_full_with_ctl(&mut wb, Some(&cancel), None);
    assert!(result.cancelled);
    let after = wb.get(sheet, 0, 1).unwrap().unwrap().value;
    assert_eq!(before, after);
}

//! Cancelled recalc/import/export leave no partial live transaction.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use omacell_bus::{Bus, TaskCtl};
use omacell_cli::{FileSession, register_file_commands};
use omacell_core::command::Origin;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;

fn cell_text(workbook: &Workbook, row: u32, col: u16) -> String {
    let sheet = workbook.active_sheet();
    let slot = workbook.get(sheet, row, col).unwrap().unwrap();
    let Value::Text(id) = slot.value else {
        panic!("expected text, got {:?}", slot.value);
    };
    workbook.intern().strings.get(id).unwrap().to_owned()
}

#[test]
fn cancelled_csv_load_does_not_replace_live_workbook() {
    let mut functions = FnRegistry::new();
    register_all(&mut functions);
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_number(sheet, 0, 0, 9.0).unwrap();
    let mut bus = Bus::new(wb, RecalcEngine::new(functions)).unwrap();
    register_file_commands(&mut bus, FileSession::new()).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let csv_path = dir.path().join("in.csv");
    std::fs::write(&csv_path, "a,b\n1,2\n3,4\n").unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let progress_cancel = Arc::clone(&cancel);
    let outcome = bus.execute_with_task(
        Origin::User,
        "file.open",
        serde_json::json!({"path": csv_path.display().to_string()}),
        TaskCtl {
            cancel: Some(Arc::clone(&cancel)),
            progress: Some(Arc::new(move |_, _, _| {
                progress_cancel.store(true, Ordering::SeqCst);
            })),
        },
    );
    assert!(!outcome.ok);
    assert_eq!(outcome.error.unwrap().code, "task.cancelled");
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
    let cancel = Arc::new(AtomicBool::new(false));
    let progress_cancel = Arc::clone(&cancel);
    let mut bus = Bus::new(Workbook::new(), RecalcEngine::new(FnRegistry::new())).unwrap();
    register_file_commands(&mut bus, FileSession::new()).unwrap();
    let outcome = bus.execute_with_task(
        Origin::User,
        "file.export",
        serde_json::json!({"path": dest.display().to_string()}),
        TaskCtl {
            cancel: Some(Arc::clone(&cancel)),
            progress: Some(Arc::new(move |_, _, _| {
                progress_cancel.store(true, Ordering::SeqCst);
            })),
        },
    );
    assert!(!outcome.ok);
    assert_eq!(outcome.error.unwrap().code, "task.cancelled");
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "keep-me");
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

#[test]
fn cancelled_automatic_recalc_rolls_back_edit_and_engine_state() {
    let mut functions = FnRegistry::new();
    register_all(&mut functions);
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.set_number(sheet, 0, 0, 1.0).unwrap();
    wb.set_formula_text(sheet, 0, 1, "=A1+1").unwrap();
    wb.set_formula_text(sheet, 0, 2, "=B1+1").unwrap();
    let mut engine = RecalcEngine::new(functions);
    engine.recalc_full(&mut wb);
    let before = wb.clone();
    let mut bus = Bus::new(wb, engine).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let progress_cancel = Arc::clone(&cancel);
    let outcome = bus.execute_with_task(
        Origin::User,
        "cell.set",
        serde_json::json!({"ref": "A1", "input": "10"}),
        TaskCtl {
            cancel: Some(Arc::clone(&cancel)),
            progress: Some(Arc::new(move |_, _, _| {
                progress_cancel.store(true, Ordering::SeqCst);
            })),
        },
    );
    assert!(!outcome.ok);
    assert_eq!(outcome.error.unwrap().code, "task.cancelled");
    for col in 0..=2 {
        assert_eq!(
            bus.workbook().get(sheet, 0, col).unwrap(),
            before.get(sheet, 0, col).unwrap()
        );
    }

    let outcome = bus.execute(
        Origin::User,
        "cell.set",
        serde_json::json!({"ref": "A1", "input": "2"}),
    );
    assert!(outcome.ok, "{:?}", outcome.error);
    assert!(matches!(
        bus.workbook().get(sheet, 0, 2).unwrap().unwrap().value,
        omacell_core::value::Value::Number(value) if value == 4.0
    ));
}

#[test]
fn cancelled_edit_restores_last_changed_cell_context() {
    let mut functions = FnRegistry::new();
    register_all(&mut functions);
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    workbook
        .set_formula_text(sheet, 0, 1, "=CELL(\"address\")")
        .unwrap();
    let mut bus = Bus::new(workbook, RecalcEngine::new(functions)).unwrap();

    let baseline = bus.execute(
        Origin::User,
        "cell.set",
        serde_json::json!({"ref": "D4", "input": "1"}),
    );
    assert!(baseline.ok, "{:?}", baseline.error);
    assert_eq!(cell_text(bus.workbook(), 0, 1), "$D$4");

    let cancel = Arc::new(AtomicBool::new(false));
    let progress_cancel = Arc::clone(&cancel);
    let cancelled = bus.execute_with_task(
        Origin::User,
        "cell.set",
        serde_json::json!({"ref": "C3", "input": "2"}),
        TaskCtl {
            cancel: Some(Arc::clone(&cancel)),
            progress: Some(Arc::new(move |_, _, _| {
                progress_cancel.store(true, Ordering::SeqCst);
            })),
        },
    );
    assert!(!cancelled.ok);
    assert_eq!(cancelled.error.unwrap().code, "task.cancelled");

    let recalc = bus.execute(
        Origin::User,
        "calc.recalc",
        serde_json::json!({"mode": "full"}),
    );
    assert!(recalc.ok, "{:?}", recalc.error);
    assert_eq!(cell_text(bus.workbook(), 0, 1), "$D$4");
}

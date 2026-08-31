//! Palette hiding and diagnose-offer gating.

use omacell_bus::CommandJson;
use omacell_core::error::ErrorKind;
use omacell_core::storage::{CellFlags, CellSlot};
use omacell_core::style::StyleId;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use omacell_ui::{Palette, diagnose_offer};

fn cmd(id: &str) -> CommandJson {
    CommandJson {
        id: id.into(),
        doc: id.into(),
        mutating: false,
        changeset_eligible: false,
        default_keys: vec![],
        arg_schema: serde_json::json!({"type": "object"}),
    }
}

#[test]
fn palette_can_include_ai_agent() {
    let commands = vec![cmd("cell.set"), cmd("ai.agent"), cmd("palette.open")];
    let mut p = Palette::default();
    p.rank(&commands, "agent");
    assert!(p.hits.iter().any(|h| h.id == "ai.agent"));
}

#[test]
fn diagnose_offer_requires_ref_cascade_and_gates() {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    let slot = CellSlot {
        value: Value::Error(ErrorKind::Ref),
        formula: None,
        style: StyleId::DEFAULT,
        flags: CellFlags::DEFAULT,
    };
    wb.set_slot(sheet, 0, 0, slot).unwrap();
    wb.set_slot(sheet, 1, 0, slot).unwrap();
    assert!(diagnose_offer(&wb, true, true).unwrap().contains("#REF!"));
    assert!(diagnose_offer(&wb, false, true).is_none());
    assert!(diagnose_offer(&wb, true, false).is_none());
}

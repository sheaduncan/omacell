//! Toolkit-neutral changeset review behavior.

use omacell_bus::{CellPreview, ChangePreview, ChangePreviewItem};
use omacell_core::changeset::{ChangeSummary, ChangesetId, CommandCall};
use omacell_core::command::{CommandId, Origin};
use omacell_ui::ChangesetReview;
use serde_json::json;

fn item(cell: &str, before: Option<&str>, after: Option<&str>) -> ChangePreviewItem {
    let col = if cell == "A1" { 0 } else { 1 };
    ChangePreviewItem {
        command: CommandCall {
            id: CommandId::new("cell.set").unwrap(),
            args: json!({"ref": cell, "input": after.unwrap_or("")}),
        },
        summary: ChangeSummary {
            cells: 1,
            text: format!("set Sheet1!{cell}"),
            ..ChangeSummary::default()
        },
        cells: vec![CellPreview {
            sheet: "Sheet1".into(),
            row: 0,
            col,
            before: before.map(str::to_string),
            after: after.map(str::to_string),
            style_changed: false,
        }],
    }
}

fn review() -> ChangesetReview {
    ChangesetReview::from(ChangePreview {
        id: ChangesetId::new("cs-7").unwrap(),
        origin: Origin::PalettePlan,
        summary: ChangeSummary {
            cells: 2,
            text: "set two cells".into(),
            ..ChangeSummary::default()
        },
        items: vec![
            item("A1", Some("old"), Some("new")),
            item("B1", None, Some("added")),
        ],
    })
}

#[test]
fn per_item_and_bulk_review_keep_command_order() {
    let mut review = review();
    assert_eq!(review.accepted_calls().len(), 2);
    review.toggle_selected();
    assert_eq!(review.accepted_calls().len(), 1);
    assert_eq!(review.accepted_calls()[0].args["ref"], "B1");
    review.move_selection(1);
    review.toggle_selected();
    assert!(review.accepted_calls().is_empty());
    review.accept_all();
    assert_eq!(review.accepted_calls().len(), 2);
    review.reject_all();
    assert!(review.accepted_calls().is_empty());
}

#[test]
fn review_body_and_cell_marks_expose_before_after_state() {
    let review = review();
    let body = review.body();
    assert!(body.contains("2/2 accepted"));
    assert!(body.contains("old → new"));
    assert!(body.contains("∅ → added"));
    let mark = review.cell_mark("Sheet1", 0, 0).unwrap();
    assert!(mark.accepted);
    assert!(mark.selected);
    assert_eq!(mark.before.as_deref(), Some("old"));
    assert_eq!(mark.after.as_deref(), Some("new"));
}

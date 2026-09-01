//! Toolkit-neutral import-plan review behavior.

use omacell_ui::ImportPlanReview;
use serde_json::json;

fn open_result() -> serde_json::Value {
    json!({
        "path": "/data/readings.csv",
        "import": {
            "current": {
                "delimiter": ",",
                "has_header": true,
                "columns": [
                    {"name": "Pressure (psi)", "ty": {"kind": "auto"}},
                    {"name": "Sample", "ty": {"kind": "keep_as_text"}}
                ]
            },
            "preview": {
                "header": ["Pressure (psi)", "Sample"],
                "rows": [[
                    {"raw": "14.7", "would_become": "14.7", "kind": "number", "changed": true},
                    {"raw": "007", "would_become": "007", "kind": "text", "changed": false}
                ]]
            }
        }
    })
}

#[test]
fn open_result_becomes_an_explicit_assistant_request_and_review() {
    let mut review = ImportPlanReview::from_open_result(&open_result())
        .unwrap()
        .expect("CSV open result");

    let request = review.assistant_args();
    assert_eq!(request["plan"]["has_header"], true);
    assert_eq!(request["preview"]["rows"][0][1]["raw"], "007");
    assert!(review.accepted_open_args().is_none());

    review
        .apply_assistant_result(&json!({
            "current": request["plan"],
            "proposed": {
                "delimiter": ",",
                "has_header": true,
                "columns": [
                    {"name": "Pressure", "ty": {"kind": "number"}},
                    {"name": "Sample", "ty": {"kind": "keep_as_text"}}
                ]
            },
            "applied": false
        }))
        .unwrap();

    let apply = review.accepted_open_args().expect("reviewed proposal");
    assert_eq!(apply["path"], "/data/readings.csv");
    assert_eq!(apply["plan"]["columns"][0]["name"], "Pressure");
    assert_eq!(apply["plan"]["columns"][0]["ty"]["kind"], "number");
    let body = review.body();
    assert!(body.contains("Pressure (psi) → Pressure"));
    assert!(body.contains("auto → number"));
    assert!(body.contains("Enter apply"));
}

#[test]
fn stale_or_preapplied_assistant_results_are_rejected() {
    let mut review = ImportPlanReview::from_open_result(&open_result())
        .unwrap()
        .unwrap();
    let different = json!({
        "delimiter": ";",
        "has_header": false
    });
    let error = review
        .apply_assistant_result(&json!({
            "current": different,
            "proposed": different,
            "applied": false
        }))
        .unwrap_err();
    assert_eq!(error.code, "ui.import");

    let request = review.assistant_args();
    let error = review
        .apply_assistant_result(&json!({
            "current": request["plan"],
            "proposed": request["plan"],
            "applied": true
        }))
        .unwrap_err();
    assert_eq!(error.code, "ui.import");
}

#[test]
fn ordinary_workbook_open_has_no_import_review() {
    assert!(
        ImportPlanReview::from_open_result(&json!({"path": "/data/book.xlsx"}))
            .unwrap()
            .is_none()
    );
}

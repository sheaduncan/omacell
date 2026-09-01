//! Formula-assist retained proposal model.

use omacell_ui::FormulaAssist;

#[test]
fn generated_formula_exposes_scratch_result_and_reference_highlights() {
    let assist = FormulaAssist::generated(
        "ai.formula.generate",
        "Sheet1!C1",
        "=SUM(A1:B1)+D2",
        "Number(6)",
    );
    assert_eq!(assist.references.len(), 2);
    assert_eq!(assist.references[0].text, "A1:B1");
    assert_eq!(assist.references[1].text, "D2");
    let body = assist.body();
    assert!(body.contains("scratch: Number(6)"));
    assert!(body.contains("references:"));
    assert!(body.contains("A1:B1"));
}

#[test]
fn explanation_has_no_apply_action() {
    let assist = FormulaAssist::explained("Sheet1!A1", "Adds the two inputs.");
    assert!(assist.formula.is_none());
    assert!(assist.body().contains("Adds the two inputs."));
}

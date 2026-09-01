//! Formula generation, explanation, fix, and refactor presentation.

use omacell_core::addr::CellRef;

use crate::{EditState, EditSurface};

/// One parser-derived reference highlight in a generated formula.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormulaReference {
    /// UTF-8 start byte.
    pub start: usize,
    /// UTF-8 end byte.
    pub end: usize,
    /// Reference source text.
    pub text: String,
    /// Theme cycle index (`references.0` through `references.7`).
    pub color: usize,
}

/// Retained formula-assist result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormulaAssist {
    /// Workflow command id.
    pub task: String,
    /// Target cell captured at submission time.
    pub target: String,
    /// Validated generated formula, if this is a mutating workflow.
    pub formula: Option<String>,
    /// Scratch evaluator result.
    pub scratch: Option<String>,
    /// Plain-language explanation.
    pub explanation: Option<String>,
    /// Parser-derived verification highlights.
    pub references: Vec<FormulaReference>,
}

impl FormulaAssist {
    /// Build a validated generated/fixed/refactored proposal.
    #[must_use]
    pub fn generated(task: &str, target: &str, formula: &str, scratch: &str) -> Self {
        let mut edit = EditState::default();
        edit.begin(
            EditSurface::FormulaBar,
            CellRef::new(0, 0).unwrap_or(CellRef {
                sheet: None,
                row: 0,
                col: 0,
                row_abs: false,
                col_abs: false,
            }),
            formula,
        );
        let references = edit
            .reference_spans()
            .into_iter()
            .filter_map(|(start, end, color)| {
                formula.get(start..end).map(|text| FormulaReference {
                    start,
                    end,
                    text: text.to_string(),
                    color,
                })
            })
            .collect();
        Self {
            task: task.into(),
            target: target.into(),
            formula: Some(formula.into()),
            scratch: Some(scratch.into()),
            explanation: None,
            references,
        }
    }

    /// Build a non-mutating formula explanation.
    #[must_use]
    pub fn explained(target: &str, explanation: &str) -> Self {
        Self {
            task: "ai.formula.explain".into(),
            target: target.into(),
            explanation: Some(explanation.into()),
            ..Self::default()
        }
    }

    /// Human-readable panel body.
    #[must_use]
    pub fn body(&self) -> String {
        let mut lines = vec![format!("{} · {}", self.task, self.target)];
        if let Some(formula) = &self.formula {
            lines.push(String::new());
            lines.push(formula.clone());
        }
        if let Some(scratch) = &self.scratch {
            lines.push(format!("scratch: {scratch}"));
        }
        if let Some(explanation) = &self.explanation {
            lines.push(String::new());
            lines.push(explanation.clone());
        }
        if !self.references.is_empty() {
            lines.push(String::new());
            lines.push("references:".into());
            for reference in &self.references {
                lines.push(format!("  [{}] {}", reference.color + 1, reference.text));
            }
        }
        lines.join("\n")
    }
}

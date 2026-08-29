//! Autocomplete provider interface (F-5.3).

use omacell_fn::all_specs;

/// One completion candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    /// Inserted text.
    pub insert: String,
    /// Signature / hint.
    pub hint: String,
    /// Kind (`fn`, `name`, `column`, `value`).
    pub kind: &'static str,
}

/// Completions for the token before the cursor.
#[must_use]
pub fn complete_functions(prefix: &str) -> Vec<Completion> {
    let p = prefix.to_ascii_uppercase();
    let mut out: Vec<Completion> = all_specs()
        .into_iter()
        .filter(|s| s.name.starts_with(&p) || s.aliases.iter().any(|a| a.starts_with(&p)))
        .map(|s| Completion {
            insert: s.name.to_string(),
            hint: s.signature.to_string(),
            kind: "fn",
        })
        .collect();
    out.sort_by(|a, b| a.insert.cmp(&b.insert));
    out.truncate(20);
    out
}

/// Extra sources (names, table columns, column values) supplied by the frontend.
pub trait CompletionSource {
    /// Defined names matching `prefix`.
    fn names(&self, prefix: &str) -> Vec<Completion>;
    /// Table columns.
    fn columns(&self, prefix: &str) -> Vec<Completion>;
    /// Distinct values from the current column.
    fn values(&self, prefix: &str) -> Vec<Completion>;
}

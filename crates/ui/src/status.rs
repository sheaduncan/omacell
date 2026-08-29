//! Status-line segment model.

/// One status-line cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusSegment {
    /// Id (`mode`, `cell`, `stats`, `calc`, `theme`).
    pub id: String,
    /// Display text.
    pub text: String,
}

/// Ordered segments from `[layout] status_line`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusLine {
    /// Configured ids.
    pub ids: Vec<String>,
    /// Current values.
    pub segments: Vec<StatusSegment>,
}

impl StatusLine {
    /// Rebuild from config ids and live values.
    pub fn refresh(
        &mut self,
        ids: &[String],
        mode: &str,
        cell: &str,
        stats: &str,
        calc: &str,
        theme: &str,
    ) {
        self.ids = ids.to_vec();
        self.segments = ids
            .iter()
            .map(|id| {
                let text = match id.as_str() {
                    "mode" => mode,
                    "cell" => cell,
                    "stats" => stats,
                    "calc" => calc,
                    "theme" => theme,
                    other => other,
                };
                StatusSegment {
                    id: id.clone(),
                    text: text.to_string(),
                }
            })
            .collect();
    }
}

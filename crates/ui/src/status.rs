//! Status-line segment model.

use omacell_core::workbook::Workbook;

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

    /// Append a diagnose-with-agent offer when present.
    pub fn set_offer(&mut self, offer: Option<String>) {
        self.segments.retain(|seg| seg.id != "diagnose");
        if let Some(text) = offer {
            self.segments.push(StatusSegment {
                id: "diagnose".into(),
                text,
            });
        }
    }

    /// Set or replace the AI status-line segment (A-1.1, A-7.3).
    pub fn set_ai(&mut self, text: Option<String>) {
        if let Some(existing) = self.segments.iter_mut().find(|seg| seg.id == "ai") {
            existing.text = text.unwrap_or_default();
            return;
        }
        if let Some(text) = text {
            self.segments.push(StatusSegment {
                id: "ai".into(),
                text,
            });
        }
    }
}

/// Status-line AI hint.
#[must_use]
pub fn ai_status_text(enabled: bool, provider: &str, local: bool, level: &str) -> String {
    if !enabled {
        "AI: off · omacell ai setup".into()
    } else {
        let loc = if local { "local" } else { "cloud" };
        format!("AI: {provider} ({loc}, {level})")
    }
}

/// Status-line *Diagnose with agent* offer (A-5.4).
#[must_use]
pub fn diagnose_offer(wb: &Workbook, diagnose_offers: bool, agent_visible: bool) -> Option<String> {
    if !diagnose_offers || !agent_visible {
        return None;
    }
    if wb.ref_error_count() >= 2 {
        Some("#REF! cascade · Diagnose with agent".into())
    } else {
        None
    }
}

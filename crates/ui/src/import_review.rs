//! Toolkit-neutral CSV import-plan preview and AI proposal review.

use omacell_core::error::CoreError;
use omacell_io::csv::{ColumnType, ImportPlan, PreviewRows};
use serde_json::{Value, json};

/// Retained import preview for one delimited source file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportPlanReview {
    /// Source path to reopen when an accepted proposal is applied.
    pub path: String,
    /// Sniffed or most recently accepted plan.
    pub current: ImportPlan,
    /// Bounded source preview produced with [`Self::current`].
    pub preview: PreviewRows,
    /// AI proposal awaiting explicit acceptance.
    pub proposed: Option<ImportPlan>,
}

impl ImportPlanReview {
    /// Decode the optional `import` payload returned by `file.open`.
    pub fn from_open_result(value: &Value) -> Result<Option<Self>, CoreError> {
        let Some(import) = value.get("import") else {
            return Ok(None);
        };
        let path = value
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| import_error("file.open import preview has no source path"))?;
        let current: ImportPlan = serde_json::from_value(
            import
                .get("current")
                .cloned()
                .ok_or_else(|| import_error("file.open import preview has no current plan"))?,
        )
        .map_err(|error| import_error(format!("invalid current import plan: {error}")))?;
        current
            .validate()
            .map_err(|error| import_error(error.message))?;
        let preview: PreviewRows = serde_json::from_value(
            import
                .get("preview")
                .cloned()
                .ok_or_else(|| import_error("file.open import preview has no rows"))?,
        )
        .map_err(|error| import_error(format!("invalid import preview: {error}")))?;
        Ok(Some(Self {
            path: path.to_string(),
            current,
            preview,
            proposed: None,
        }))
    }

    /// Command arguments for the user-triggered `ai.import.assist` request.
    #[must_use]
    pub fn assistant_args(&self) -> Value {
        json!({
            "plan": self.current,
            "preview": self.preview,
        })
    }

    /// Install a non-applied proposal returned for the current plan.
    pub fn apply_assistant_result(&mut self, value: &Value) -> Result<(), CoreError> {
        if value.get("applied").and_then(Value::as_bool) != Some(false) {
            return Err(import_error(
                "import assistant result must be an unapplied proposal",
            ));
        }
        let current: ImportPlan = serde_json::from_value(
            value
                .get("current")
                .cloned()
                .ok_or_else(|| import_error("import assistant result has no current plan"))?,
        )
        .map_err(|error| import_error(format!("invalid assistant current plan: {error}")))?;
        if current != self.current {
            return Err(import_error(
                "stale import assistant result does not match the current plan",
            ));
        }
        let proposed: ImportPlan = serde_json::from_value(
            value
                .get("proposed")
                .cloned()
                .ok_or_else(|| import_error("import assistant result has no proposed plan"))?,
        )
        .map_err(|error| import_error(format!("invalid proposed import plan: {error}")))?;
        proposed
            .validate()
            .map_err(|error| import_error(error.message))?;
        self.proposed = Some(proposed);
        Ok(())
    }

    /// `file.open` arguments that explicitly apply the reviewed proposal.
    #[must_use]
    pub fn accepted_open_args(&self) -> Option<Value> {
        self.proposed.as_ref().map(|plan| {
            json!({
                "path": self.path,
                "plan": plan,
            })
        })
    }

    /// Drop the pending AI proposal while preserving the sniffed preview.
    pub fn reject_proposal(&mut self) {
        self.proposed = None;
    }

    /// Human-readable panel body with an import preview and plan diff.
    #[must_use]
    pub fn body(&self) -> String {
        let mut lines = vec![
            format!("source: {}", self.path),
            plan_summary(&self.current),
        ];
        if let Some(proposed) = &self.proposed {
            lines.push(String::new());
            lines.push("proposed plan".into());
            append_plan_diff(&mut lines, &self.current, proposed);
        }
        if let Some(header) = &self.preview.header
            && !header.is_empty()
        {
            lines.push(String::new());
            lines.push(format!("header: {}", header.join(" | ")));
        }
        for row in self.preview.rows.iter().take(4) {
            let cells = row
                .iter()
                .take(6)
                .map(|cell| {
                    if cell.changed {
                        format!("{} → {}", cell.raw, cell.would_become)
                    } else {
                        cell.raw.clone()
                    }
                })
                .collect::<Vec<_>>();
            lines.push(cells.join(" | "));
        }
        lines.push(String::new());
        lines.push(
            if self.proposed.is_some() {
                "Enter apply · R reject · Esc close"
            } else {
                "A ask AI · Enter keep current · Esc close"
            }
            .into(),
        );
        lines.join("\n")
    }
}

fn plan_summary(plan: &ImportPlan) -> String {
    format!(
        "delimiter {:?} · header {} · {} columns",
        plan.delimiter,
        if plan.has_header { "yes" } else { "no" },
        plan.columns.len()
    )
}

fn append_plan_diff(lines: &mut Vec<String>, current: &ImportPlan, proposed: &ImportPlan) {
    push_diff(lines, "delimiter", current.delimiter, proposed.delimiter);
    push_diff(lines, "quote", current.quote, proposed.quote);
    push_diff(
        lines,
        "encoding",
        format!("{:?}", current.encoding),
        format!("{:?}", proposed.encoding),
    );
    push_diff(lines, "BOM", current.bom, proposed.bom);
    push_diff(lines, "header", current.has_header, proposed.has_header);
    push_diff(lines, "skip rows", current.skip_rows, proposed.skip_rows);
    push_diff(
        lines,
        "locale",
        format!("{:?}", current.locale),
        format!("{:?}", proposed.locale),
    );
    push_diff(lines, "decimal", current.decimal, proposed.decimal);
    push_diff(
        lines,
        "thousands",
        display_optional(current.thousands),
        display_optional(proposed.thousands),
    );
    push_diff(
        lines,
        "line ending",
        format!("{:?}", current.line_ending),
        format!("{:?}", proposed.line_ending),
    );
    push_diff(
        lines,
        "date system",
        format!("{:?}", current.date_system),
        format!("{:?}", proposed.date_system),
    );
    let columns = current.columns.len().max(proposed.columns.len());
    for index in 0..columns {
        let before = current.columns.get(index);
        let after = proposed.columns.get(index);
        let before_name = before
            .and_then(|column| column.name.as_deref())
            .unwrap_or("—");
        let after_name = after
            .and_then(|column| column.name.as_deref())
            .unwrap_or("—");
        if before_name != after_name {
            lines.push(format!(
                "column {} name: {before_name} → {after_name}",
                index + 1
            ));
        }
        let before_type = before.map_or("auto".into(), |column| type_name(&column.ty));
        let after_type = after.map_or("auto".into(), |column| type_name(&column.ty));
        if before_type != after_type {
            lines.push(format!(
                "column {} type: {before_type} → {after_type}",
                index + 1
            ));
        }
    }
    if lines.last().is_some_and(|line| line == "proposed plan") {
        lines.push("no changes".into());
    }
}

fn push_diff<T: std::fmt::Display + PartialEq>(
    lines: &mut Vec<String>,
    label: &str,
    before: T,
    after: T,
) {
    if before != after {
        lines.push(format!("{label}: {before} → {after}"));
    }
}

fn display_optional(value: Option<char>) -> String {
    value.map_or_else(|| "none".into(), |value| value.to_string())
}

fn type_name(value: &ColumnType) -> String {
    match value {
        ColumnType::Auto => "auto".into(),
        ColumnType::Number => "number".into(),
        ColumnType::Text => "text".into(),
        ColumnType::Date { format } if format.is_empty() => "date".into(),
        ColumnType::Date { format } => format!("date ({format})"),
        ColumnType::Boolean => "boolean".into(),
        ColumnType::KeepAsText => "keep as text".into(),
    }
}

fn import_error(message: impl Into<String>) -> CoreError {
    CoreError::new("ui.import", message)
}

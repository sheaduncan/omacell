//! Per-session in-app agent autopilot boundaries.

use omacell_core::addr::{RefKind, SheetId, parse_a1};
use omacell_core::changeset::CommandCall;
use omacell_core::workbook::Workbook;

use crate::error::{AiError, codes};
use crate::plan::forbidden;

// Autopilot is a mutation boundary: new catalog commands stay review-only
// until their semantics and target fields are explicitly audited here.
const ALLOWED_COMMANDS: &[&str] = &[
    "cell.clear",
    "cell.set",
    "chart.fromselection",
    "condfmt.add",
    "edit.autosum",
    "edit.change",
    "edit.clear",
    "edit.clearcell",
    "edit.clearrow",
    "edit.collapse",
    "edit.comment",
    "edit.commentreply",
    "edit.commentresolve",
    "edit.copyformulaabove",
    "edit.copyvalueabove",
    "edit.delete",
    "edit.expand",
    "edit.filldown",
    "edit.fillleft",
    "edit.fillright",
    "edit.fillselection",
    "edit.fillup",
    "edit.flashfill",
    "edit.group",
    "edit.hyperlink",
    "edit.insertdate",
    "edit.inserttime",
    "edit.note",
    "edit.pastespecial",
    "edit.replaceall",
    "edit.ungroup",
    "filter.clear",
    "filter.set",
    "filter.toggle",
    "format.autofitcols",
    "format.autofitrows",
    "format.bold",
    "format.bordernone",
    "format.borderoutline",
    "format.colwidth",
    "format.currency",
    "format.date",
    "format.general",
    "format.indent",
    "format.italic",
    "format.number",
    "format.numberstyle",
    "format.outdent",
    "format.percent",
    "format.rowheight",
    "format.scientific",
    "format.time",
    "format.underline",
    "name.createfrom",
    "name.define",
    "name.remove",
    "pivot.create",
    "pivot.remove",
    "range.clear",
    "range.consolidate",
    "range.merge",
    "range.mergeacross",
    "range.removeduplicates",
    "range.set",
    "range.sort",
    "range.unmerge",
    "sheet.add",
    "sheet.remove",
    "sheet.rename",
    "sheet.reorder",
    "sheet.visibility",
    "sparkline.set",
    "style.set",
    "table.convert",
    "table.create",
    "table.rename",
    "table.resize",
    "table.totals",
    "validation.set",
    "view.hidecols",
    "view.hiderows",
    "view.unhidecols",
    "view.unhiderows",
    "whatif.goalseek",
];

/// Explicitly selected session scope for automatic application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutopilotScope {
    /// Any workbook mutation that survives the security deny rules.
    Workbook,
    /// Commands whose targets are provably confined to one sheet.
    Sheet(SheetId),
    /// Commands whose targets are provably confined to one rectangular range.
    Range {
        /// Scoped sheet.
        sheet: SheetId,
        /// Inclusive first row.
        min_row: u32,
        /// Inclusive first column.
        min_col: u16,
        /// Inclusive last row.
        max_row: u32,
        /// Inclusive last column.
        max_col: u16,
    },
}

/// Stateful operation-cap and scope gate created only by explicit UI consent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutopilotPolicy {
    scope: AutopilotScope,
    max_ops: usize,
    used_ops: usize,
}

impl AutopilotPolicy {
    /// Start a new explicitly enabled session policy.
    #[must_use]
    pub fn new(scope: AutopilotScope, max_ops: u32) -> Self {
        Self {
            scope,
            max_ops: usize::try_from(max_ops.max(1)).unwrap_or(usize::MAX),
            used_ops: 0,
        }
    }

    /// Selected scope.
    #[must_use]
    pub fn scope(&self) -> &AutopilotScope {
        &self.scope
    }

    /// Successfully authorized operations in this session.
    #[must_use]
    pub fn used_ops(&self) -> usize {
        self.used_ops
    }

    /// Session cap.
    #[must_use]
    pub fn max_ops(&self) -> usize {
        self.max_ops
    }

    /// Check forbidden classes, operation cap, and every provable target, then charge the cap.
    pub fn authorize_and_record(
        &mut self,
        calls: &[CommandCall],
        workbook: &Workbook,
    ) -> Result<(), AiError> {
        let projected = self
            .used_ops
            .checked_add(calls.len())
            .ok_or_else(|| AiError::new(codes::AUTOPILOT, "autopilot operation count overflow"))?;
        if projected > self.max_ops {
            return Err(AiError::new(
                codes::AUTOPILOT,
                format!(
                    "autopilot operation cap is {}; {} already used and {} proposed",
                    self.max_ops,
                    self.used_ops,
                    calls.len()
                ),
            ));
        }
        for call in calls {
            authorize_call(call, &self.scope, workbook)?;
        }
        self.used_ops = projected;
        Ok(())
    }
}

fn authorize_call(
    call: &CommandCall,
    scope: &AutopilotScope,
    workbook: &Workbook,
) -> Result<(), AiError> {
    let id = call.id.as_str();
    if !ALLOWED_COMMANDS.contains(&id)
        || forbidden(id)
        || ["macro.", "theme.", "ipc.", "plugin."]
            .iter()
            .any(|prefix| id.starts_with(prefix))
        || matches!(id, "workbook.protect" | "sheet.protect" | "calc.mode")
    {
        return Err(AiError::new(
            codes::AUTOPILOT,
            format!("autopilot has not approved {id}"),
        ));
    }
    if matches!(scope, AutopilotScope::Workbook) {
        return Ok(());
    }

    let targets = command_targets(call, workbook)?;
    if targets.is_empty() {
        return Err(AiError::new(
            codes::AUTOPILOT,
            format!("autopilot cannot prove the scope of {id}"),
        )
        .with_hint("review this command manually or use workbook scope"));
    }
    for target in targets {
        match scope {
            AutopilotScope::Workbook => {}
            AutopilotScope::Sheet(sheet) if target.sheet == *sheet => {}
            AutopilotScope::Range {
                sheet,
                min_row,
                min_col,
                max_row,
                max_col,
            } if target.sheet == *sheet
                && target.min_row >= *min_row
                && target.max_row <= *max_row
                && target.min_col >= *min_col
                && target.max_col <= *max_col => {}
            _ => {
                return Err(AiError::new(
                    codes::AUTOPILOT,
                    format!("autopilot target for {id} is outside the session scope"),
                ));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Target {
    sheet: SheetId,
    min_row: u32,
    min_col: u16,
    max_row: u32,
    max_col: u16,
}

fn command_targets(call: &CommandCall, workbook: &Workbook) -> Result<Vec<Target>, AiError> {
    let mut raw = Vec::new();
    for field in ["ref", "range", "src", "dest", "target"] {
        if let Some(value) = call.args.get(field).and_then(serde_json::Value::as_str) {
            raw.push(value);
        }
    }
    if call.id.as_str() == "pivot.create"
        && let Some(value) = call.args.get("source").and_then(serde_json::Value::as_str)
    {
        raw.push(value);
    }
    if call.id.as_str() == "whatif.goalseek"
        && let Some(value) = call.args.get("input").and_then(serde_json::Value::as_str)
    {
        raw.push(value);
    }
    if let Some(values) = call
        .args
        .get("sources")
        .and_then(serde_json::Value::as_array)
    {
        raw.extend(values.iter().filter_map(serde_json::Value::as_str));
    }
    let mut targets = raw
        .into_iter()
        .map(|value| resolve_target(value, workbook))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(sheet_name) = call.args.get("sheet").and_then(serde_json::Value::as_str) {
        let sheet = workbook
            .resolve_sheet_name(sheet_name)
            .map_err(AiError::from)?;
        targets.push(Target {
            sheet,
            min_row: 0,
            min_col: 0,
            max_row: omacell_core::limits::MAX_ROWS - 1,
            max_col: omacell_core::limits::MAX_COLS - 1,
        });
    }
    Ok(targets)
}

fn resolve_target(value: &str, workbook: &Workbook) -> Result<Target, AiError> {
    let parsed = parse_a1(value).map_err(AiError::from)?;
    let sheet = match parsed.sheet {
        Some(spec) if spec.end.is_some() => {
            return Err(AiError::new(
                codes::AUTOPILOT,
                "autopilot does not accept 3-D targets",
            ));
        }
        Some(spec) => workbook
            .resolve_sheet_name(&spec.start)
            .map_err(AiError::from)?,
        None => workbook.active_sheet(),
    };
    let (min_row, min_col, max_row, max_col) = match parsed.kind {
        RefKind::Cell(cell) => (cell.row, cell.col, cell.row, cell.col),
        RefKind::Range(range) => (
            range.start.row.min(range.end.row),
            range.start.col.min(range.end.col),
            range.start.row.max(range.end.row),
            range.start.col.max(range.end.col),
        ),
    };
    Ok(Target {
        sheet,
        min_row,
        min_col,
        max_row,
        max_col,
    })
}

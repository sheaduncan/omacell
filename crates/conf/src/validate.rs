//! Semantic validation for the typed configuration.

use std::path::{Component, Path};

use omacell_core::error::CoreError;

use crate::error;
use crate::schema::{AutoNum, CURRENT_SCHEMA, Config};

impl Config {
    /// Validate enum-like tokens, numeric ranges, and path-shaped values.
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.schema != CURRENT_SCHEMA {
            return Err(error::schema(format!(
                "unsupported schema {}; expected {CURRENT_SCHEMA}",
                self.schema
            )));
        }

        one_of(
            "appearance.grid_line_style",
            &self.appearance.grid_line_style,
            &["solid", "dotted", "none"],
        )?;
        one_of(
            "appearance.cursor_style",
            &self.appearance.cursor_style,
            &["outline", "block", "underline"],
        )?;
        one_of(
            "appearance.selection_style",
            &self.appearance.selection_style,
            &["fill", "outline"],
        )?;
        one_of(
            "appearance.sheet_tabs_position",
            &self.appearance.sheet_tabs_position,
            &["top", "bottom"],
        )?;
        one_of(
            "appearance.corner_style",
            &self.appearance.corner_style,
            &["system", "rounded", "sharp"],
        )?;
        one_of(
            "appearance.animation",
            &self.appearance.animation,
            &["system", "on", "off"],
        )?;
        non_empty("appearance.cell_font", &self.appearance.cell_font)?;
        non_empty("appearance.ui_font", &self.appearance.ui_font)?;
        positive("appearance.cell_font_size", self.appearance.cell_font_size)?;
        positive("appearance.column_width", self.appearance.column_width)?;
        auto_num(
            "appearance.ui_font_size",
            &self.appearance.ui_font_size,
            "system",
            false,
        )?;
        auto_num(
            "appearance.row_height",
            &self.appearance.row_height,
            "auto",
            false,
        )?;

        one_of(
            "behavior.enter_moves",
            &self.behavior.enter_moves,
            &["down", "right", "none"],
        )?;
        one_of(
            "behavior.reference_style",
            &self.behavior.reference_style,
            &["A1", "R1C1"],
        )?;
        if self.behavior.default_sheets == 0 {
            return invalid("behavior.default_sheets", "must be at least 1");
        }
        if !matches!(self.behavior.date_system, 1900 | 1904) {
            return invalid("behavior.date_system", "must be 1900 or 1904");
        }

        one_of(
            "calc.mode",
            &self.calc.mode,
            &["automatic", "automatic_except_tables", "manual"],
        )?;
        auto_num("calc.threads", &self.calc.threads, "auto", true)?;
        if self.calc.max_iterations == 0 {
            return invalid("calc.max_iterations", "must be at least 1");
        }
        non_negative("calc.max_change", self.calc.max_change)?;

        one_of(
            "files.default_format",
            &self.files.default_format,
            &["xlsx", "omc"],
        )?;
        one_of(
            "files.csv.type_inference",
            &self.files.csv.type_inference,
            &["conservative", "aggressive", "none"],
        )?;
        if self.files.csv.delimiter != "auto" && self.files.csv.delimiter.chars().count() != 1 {
            return invalid(
                "files.csv.delimiter",
                "must be 'auto' or one Unicode character",
            );
        }
        non_empty("files.csv.encoding", &self.files.csv.encoding)?;

        one_of(
            "layout.panel_side",
            &self.layout.panel_side,
            &["right", "left", "bottom"],
        )?;
        nonzero("layout.panel_width", self.layout.panel_width)?;
        nonzero("layout.formula_bar_lines", self.layout.formula_bar_lines)?;
        nonzero(
            "layout.compact_below_width",
            self.layout.compact_below_width,
        )?;
        if self.layout.status_line.iter().any(|item| item.is_empty()) {
            return invalid("layout.status_line", "entries must not be empty");
        }

        one_of(
            "integrations.omarchy",
            &self.integrations.omarchy,
            &["auto", "on", "off"],
        )?;
        one_of(
            "integrations.notifications",
            &self.integrations.notifications,
            &["all", "recovery_only", "off"],
        )?;
        if !(1_048_576..=16_777_216).contains(&self.ipc.max_frame_bytes) {
            return invalid(
                "ipc.max_frame_bytes",
                "must be between 1048576 and 16777216 bytes",
            );
        }
        one_of(
            "scripting.embedded_scripts",
            &self.scripting.embedded_scripts,
            &["sandbox", "ask", "deny"],
        )?;
        if self
            .network
            .allow_functions
            .iter()
            .any(|item| item.is_empty())
        {
            return invalid("network.allow_functions", "entries must not be empty");
        }

        one_of(
            "ai.privacy.send",
            &self.ai.privacy.send,
            &["schema", "sample", "full"],
        )?;
        for (name, provider) in &self.ai.providers {
            one_of(
                &format!("ai.providers.{name}.kind"),
                &provider.kind,
                &["openai_compatible", "anthropic"],
            )?;
            non_empty(&format!("ai.providers.{name}.endpoint"), &provider.endpoint)?;
            if !provider.endpoint.starts_with("http://")
                && !provider.endpoint.starts_with("https://")
            {
                return invalid(
                    &format!("ai.providers.{name}.endpoint"),
                    "must be an absolute HTTP or HTTPS URL",
                );
            }
            if provider.secret_env.is_some() && provider.secret_cmd.is_some() {
                return invalid(
                    &format!("ai.providers.{name}"),
                    "configure only one of secret_env or secret_cmd",
                );
            }
        }
        one_of(
            "ai.functions.xlsx_export",
            &self.ai.functions.xlsx_export,
            &["formulas", "values"],
        )?;
        one_of(
            "ai.completion.mode",
            &self.ai.completion.mode,
            &["auto", "on", "off"],
        )?;
        one_of(
            "ai.agent.review",
            &self.ai.agent.review,
            &["always", "autopilot_opt_in"],
        )?;
        one_of(
            "ai.agent.autopilot_scope",
            &self.ai.agent.autopilot_scope,
            &["sheet", "range", "workbook"],
        )?;
        nonzero("ai.functions.batch_size", self.ai.functions.batch_size)?;
        nonzero(
            "ai.functions.max_cells_per_recalc",
            self.ai.functions.max_cells_per_recalc,
        )?;
        nonzero(
            "ai.functions.max_requests_per_minute",
            self.ai.functions.max_requests_per_minute,
        )?;
        nonzero(
            "ai.agent.autopilot_max_ops",
            self.ai.agent.autopilot_max_ops,
        )?;

        nonzero("charts.line_width", self.charts.line_width)?;
        one_of("tui.truecolor", &self.tui.truecolor, &["auto", "on", "off"])?;
        one_of(
            "tui.graphics",
            &self.tui.graphics,
            &["auto", "sixel", "kitty", "off"],
        )?;
        if self.config.debounce_ms == 0 || self.config.debounce_ms > 60_000 {
            return invalid("config.debounce_ms", "must be in 1..=60000");
        }
        relative_path("keys.file", &self.keys.file)?;
        Ok(())
    }
}

fn one_of(path: &str, value: &str, allowed: &[&str]) -> Result<(), CoreError> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        invalid(path, &format!("must be one of {}", allowed.join(", ")))
    }
}

fn non_empty(path: &str, value: &str) -> Result<(), CoreError> {
    if value.trim().is_empty() {
        invalid(path, "must not be empty")
    } else {
        Ok(())
    }
}

fn positive(path: &str, value: f64) -> Result<(), CoreError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        invalid(path, "must be a finite positive number")
    }
}

fn non_negative(path: &str, value: f64) -> Result<(), CoreError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        invalid(path, "must be a finite non-negative number")
    }
}

fn nonzero(path: &str, value: u32) -> Result<(), CoreError> {
    if value == 0 {
        invalid(path, "must be at least 1")
    } else {
        Ok(())
    }
}

fn auto_num(path: &str, value: &AutoNum, token: &str, integer: bool) -> Result<(), CoreError> {
    match value {
        AutoNum::Token(actual) if actual == token => Ok(()),
        AutoNum::Token(_) => invalid(path, &format!("token must be '{token}'")),
        AutoNum::Num(number)
            if number.is_finite() && *number > 0.0 && (!integer || number.fract() == 0.0) =>
        {
            Ok(())
        }
        AutoNum::Num(_) if integer => invalid(path, "must be a positive integer"),
        AutoNum::Num(_) => invalid(path, "must be a finite positive number"),
    }
}

fn relative_path(path: &str, value: &str) -> Result<(), CoreError> {
    non_empty(path, value)?;
    if Path::new(value)
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
    {
        Ok(())
    } else {
        invalid(path, "must be a relative path without '..'")
    }
}

fn invalid<T>(path: &str, message: &str) -> Result<T, CoreError> {
    Err(error::schema(format!("{path}: {message}")))
}

//! Typed Appendix B configuration.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Number or the sentinel `"system"` / `"auto"`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum AutoNum {
    /// Numeric value.
    Num(f64),
    /// `"system"`, `"auto"`, or a named token.
    Token(String),
}

impl AutoNum {
    /// Token `system` / `auto`.
    #[must_use]
    pub fn is_token(&self, name: &str) -> bool {
        matches!(self, Self::Token(s) if s == name)
    }
}

/// Root configuration (`schema = 1`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Schema version.
    #[schemars(extend("enum" = [1]))]
    pub schema: u32,
    /// Appearance.
    pub appearance: Appearance,
    /// Editing behaviour.
    pub behavior: Behavior,
    /// Calculation defaults.
    pub calc: Calc,
    /// Locale.
    pub locale: Locale,
    /// File I/O.
    pub files: Files,
    /// Session restore.
    pub session: Session,
    /// Chrome layout.
    pub layout: Layout,
    /// Host integrations.
    pub integrations: Integrations,
    /// Network gate.
    pub network: Network,
    /// Lua.
    pub scripting: Scripting,
    /// AI.
    pub ai: Ai,
    /// Charts.
    pub charts: Charts,
    /// TUI.
    pub tui: Tui,
    /// Keymap pointer.
    pub keys: Keys,
    /// Live-reload (library extension of Appendix B).
    #[serde(default)]
    pub config: ConfigMeta,
}

/// Live-reload knobs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigMeta {
    /// Watch user files.
    #[serde(default = "true_default")]
    pub live_reload: bool,
    /// Debounce milliseconds.
    #[serde(default = "debounce_default")]
    #[schemars(range(min = 1, max = 60_000))]
    pub debounce_ms: u64,
}

impl Default for ConfigMeta {
    fn default() -> Self {
        Self {
            live_reload: true,
            debounce_ms: debounce_default(),
        }
    }
}

fn true_default() -> bool {
    true
}
fn debounce_default() -> u64 {
    50
}

/// `[appearance]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Appearance {
    /// Cell fontconfig alias.
    pub cell_font: String,
    /// Cell size in pt.
    #[schemars(range(min = 0.000_001))]
    pub cell_font_size: f64,
    /// UI fontconfig alias.
    pub ui_font: String,
    /// `"system"` or pt.
    pub ui_font_size: AutoNum,
    /// Draw grid lines.
    pub grid_lines: bool,
    /// `solid` / `dotted` / `none`.
    #[schemars(extend("enum" = ["solid", "dotted", "none"]))]
    pub grid_line_style: String,
    /// `"auto"` or pt.
    pub row_height: AutoNum,
    /// Default column width in characters.
    #[schemars(range(min = 0.000_001))]
    pub column_width: f64,
    /// Padding px at 1×.
    pub cell_padding: u32,
    /// `outline` / `block` / `underline`.
    #[schemars(extend("enum" = ["outline", "block", "underline"]))]
    pub cursor_style: String,
    /// `fill` / `outline`.
    #[schemars(extend("enum" = ["fill", "outline"]))]
    pub selection_style: String,
    /// Formula bar visible.
    pub show_formula_bar: bool,
    /// Status line visible.
    pub show_status_line: bool,
    /// Sheet tabs visible.
    pub show_sheet_tabs: bool,
    /// `top` / `bottom`.
    #[schemars(extend("enum" = ["top", "bottom"]))]
    pub sheet_tabs_position: String,
    /// `system` / `rounded` / `sharp`.
    #[schemars(extend("enum" = ["system", "rounded", "sharp"]))]
    pub corner_style: String,
    /// Alternate row fill.
    pub zebra_rows: bool,
    /// WCAG nudge.
    pub enforce_contrast: bool,
    /// `system` / `on` / `off`.
    #[schemars(extend("enum" = ["system", "on", "off"]))]
    pub animation: String,
}

/// `[behavior]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Behavior {
    /// `down` / `right` / `none`.
    #[schemars(extend("enum" = ["down", "right", "none"]))]
    pub enter_moves: String,
    /// Formula autocomplete.
    pub autocomplete: bool,
    /// Autocorrect.
    pub autocorrect: bool,
    /// Formula hints.
    pub formula_hints: bool,
    /// `A1` / `R1C1`.
    #[schemars(extend("enum" = ["A1", "R1C1"]))]
    pub reference_style: String,
    /// New-workbook sheet count.
    #[schemars(range(min = 1))]
    pub default_sheets: u32,
    /// 1900 or 1904.
    #[schemars(extend("enum" = [1900, 1904]))]
    pub date_system: u32,
    /// Precision as displayed.
    pub precision_as_displayed: bool,
    /// Detect delimited paste.
    pub smart_paste: bool,
    /// Fill-options prompt.
    pub fill_prompt: bool,
}

/// `[calc]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Calc {
    /// `automatic` / `automatic_except_tables` / `manual`.
    #[schemars(extend("enum" = ["automatic", "automatic_except_tables", "manual"]))]
    pub mode: String,
    /// `"auto"` or a thread count.
    pub threads: AutoNum,
    /// Iterative calc.
    pub iterative: bool,
    /// Max iterations.
    #[schemars(range(min = 1))]
    pub max_iterations: u32,
    /// Max change.
    #[schemars(range(min = 0.0))]
    pub max_change: f64,
    /// Volatiles on open.
    pub volatile_on_open: bool,
}

/// `[locale]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Locale {
    /// Language tag or `system`.
    pub language: String,
    /// Decimal separator or `system`.
    pub decimal_separator: String,
    /// Thousands separator or `system`.
    pub thousands_separator: String,
    /// List separator or `system`.
    pub list_separator: String,
    /// Date format or `system`.
    pub date_format: String,
    /// Currency or `system`.
    pub currency: String,
    /// First weekday or `system`.
    pub first_weekday: String,
    /// Localized function names.
    pub localized_function_names: bool,
}

/// `[files]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Files {
    /// `xlsx` / `omc`.
    #[schemars(extend("enum" = ["xlsx", "omc"]))]
    pub default_format: String,
    /// Autosave seconds; 0 disables.
    pub autosave_interval: u64,
    /// Numbered backups.
    pub keep_backups: u32,
    /// Follow external links.
    pub follow_external_links: bool,
    /// CSV defaults.
    pub csv: FilesCsv,
    /// Xlsx defaults.
    pub xlsx: FilesXlsx,
}

/// `[files.csv]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FilesCsv {
    /// `auto` or a delimiter.
    pub delimiter: String,
    /// `auto` or an encoding.
    pub encoding: String,
    /// `conservative` / `aggressive` / `none`.
    #[schemars(extend("enum" = ["conservative", "aggressive", "none"]))]
    pub type_inference: String,
}

/// `[files.xlsx]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FilesXlsx {
    /// Preserve L3 parts.
    pub preserve_unknown_parts: bool,
}

/// `[session]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Session {
    /// Restore windows.
    pub restore: bool,
    /// Recent-file count.
    pub recent_files: u32,
    /// Remember Hyprland workspace.
    pub workspace_binding: bool,
}

/// `[layout]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    /// `right` / `left` / `bottom`.
    #[schemars(extend("enum" = ["right", "left", "bottom"]))]
    pub panel_side: String,
    /// Panel width px.
    #[schemars(range(min = 1))]
    pub panel_width: u32,
    /// Formula bar rows.
    #[schemars(range(min = 1))]
    pub formula_bar_lines: u32,
    /// Compact-chrome width.
    #[schemars(range(min = 1))]
    pub compact_below_width: u32,
    /// Status-line segments.
    pub status_line: Vec<String>,
    /// Menu bar.
    pub menu_bar: bool,
}

/// `[integrations]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Integrations {
    /// `auto` / `on` / `off`.
    #[schemars(extend("enum" = ["auto", "on", "off"]))]
    pub omarchy: String,
    /// `all` / `recovery_only` / `off`.
    #[schemars(extend("enum" = ["all", "recovery_only", "off"]))]
    pub notifications: String,
    /// Offer menu rows.
    pub menu_entries: bool,
    /// soffice `.xls` bridge.
    pub libreoffice_fallback: bool,
    /// OCR paste.
    pub ocr_paste: bool,
}

/// `[network]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Network {
    /// Off by default.
    pub enabled: bool,
    /// Function allowlist.
    pub allow_functions: Vec<String>,
    /// Proxy URL.
    pub proxy: String,
}

/// `[scripting]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scripting {
    /// Lua enabled.
    pub enabled: bool,
    /// Trusted directories.
    pub trusted_dirs: Vec<String>,
    /// `sandbox` / `ask` / `deny`.
    #[schemars(extend("enum" = ["sandbox", "ask", "deny"]))]
    pub embedded_scripts: String,
}

/// `[ai]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Ai {
    /// Master switch.
    pub enabled: bool,
    /// Status-line segment.
    pub status_segment: bool,
    /// Named providers.
    #[serde(default)]
    pub providers: BTreeMap<String, AiProvider>,
    /// Task slots.
    pub models: AiModels,
    /// Privacy.
    pub privacy: AiPrivacy,
    /// Cell functions.
    pub functions: AiFunctions,
    /// Completion.
    pub completion: AiCompletion,
    /// In-app agent.
    pub agent: AiAgent,
}

/// One AI provider block.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AiProvider {
    /// Wire kind.
    pub kind: String,
    /// Endpoint URL.
    pub endpoint: String,
    /// Loopback.
    #[serde(default)]
    pub local: bool,
    /// Secret env var.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_env: Option<String>,
    /// Secret command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_cmd: Option<String>,
    /// Request timeout milliseconds (0 = 30_000).
    #[serde(default)]
    pub timeout: u32,
    /// Extra HTTP headers (never secrets).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

/// `[ai.models]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AiModels {
    /// Fast slot.
    pub fast: String,
    /// Default slot.
    pub default: String,
    /// Strong slot.
    #[serde(default)]
    pub strong: String,
    /// Agent slot.
    #[serde(default)]
    pub agent: String,
    /// Vision slot.
    #[serde(default)]
    pub vision: String,
}

/// `[ai.privacy]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AiPrivacy {
    /// `schema` / `sample` / `full`.
    #[schemars(extend("enum" = ["schema", "sample", "full"]))]
    pub send: String,
    /// Local providers send full.
    pub local_full: bool,
    /// Suggest redaction.
    pub suggest_redaction: bool,
    /// Log payloads.
    pub log_content: bool,
}

/// `[ai.functions]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AiFunctions {
    /// Re-query on input change.
    pub auto: bool,
    /// Batch size.
    #[schemars(range(min = 1))]
    pub batch_size: u32,
    /// Confirm above this many cells.
    #[schemars(range(min = 1))]
    pub max_cells_per_recalc: u32,
    /// Rate limit.
    #[schemars(range(min = 1))]
    pub max_requests_per_minute: u32,
    /// Keep stale values.
    pub keep_stale: bool,
    /// Refresh on full recalc.
    pub refresh_on_full_recalc: bool,
    /// `formulas` / `values`.
    #[schemars(extend("enum" = ["formulas", "values"]))]
    pub xlsx_export: String,
}

/// `[ai.completion]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AiCompletion {
    /// `auto` / `on` / `off`.
    #[schemars(extend("enum" = ["auto", "on", "off"]))]
    pub mode: String,
    /// Debounce ms.
    pub debounce: u32,
}

/// `[ai.agent]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AiAgent {
    /// `always` / `autopilot_opt_in`.
    #[schemars(extend("enum" = ["always", "autopilot_opt_in"]))]
    pub review: String,
    /// `sheet` / `range` / `workbook`.
    #[schemars(extend("enum" = ["sheet", "range", "workbook"]))]
    pub autopilot_scope: String,
    /// Cap.
    #[schemars(range(min = 1))]
    pub autopilot_max_ops: u32,
    /// Diagnose offers.
    pub diagnose_offers: bool,
    /// Agent panel.
    pub panel: bool,
    /// Skills directory.
    pub skills_dir: String,
}

/// `[charts]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Charts {
    /// `theme` or a named palette.
    pub palette: String,
    /// Default chart type.
    pub default_type: String,
    /// Line width.
    #[schemars(range(min = 1))]
    pub line_width: u32,
    /// `ui` or a font alias.
    pub font: String,
}

/// `[tui]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Tui {
    /// Unicode box drawing.
    pub unicode_borders: bool,
    /// `auto` / `on` / `off`.
    #[schemars(extend("enum" = ["auto", "on", "off"]))]
    pub truecolor: String,
    /// Mouse.
    pub mouse: bool,
    /// `auto` / `sixel` / `kitty` / `off`.
    #[schemars(extend("enum" = ["auto", "sixel", "kitty", "off"]))]
    pub graphics: String,
}

/// `[keys]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Keys {
    /// Keymap file name under the config dir or package defaults.
    #[schemars(pattern(r"^[^/\\.][^\\]*$"))]
    pub file: String,
}

/// Shipped default TOML (Appendix B).
pub const DEFAULT_TOML: &str = include_str!("../../../default/config.toml");

/// Current on-disk configuration schema.
pub const CURRENT_SCHEMA: u32 = 1;

/// Parse the packaged defaults.
pub fn package_defaults() -> Result<Config, omacell_core::error::CoreError> {
    let config: Config = toml::from_str(DEFAULT_TOML)
        .map_err(|e| crate::error::parse(format!("package defaults: {e}")))?;
    config.validate()?;
    Ok(config)
}

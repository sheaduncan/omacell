//! Config errors `{code, message, hint}`.

use omacell_core::error::CoreError;

/// Machine codes.
pub mod codes {
    /// TOML could not be parsed (includes file/line when known).
    pub const CONFIG_PARSE: &str = "config.parse";
    /// A value failed schema validation.
    pub const CONFIG_SCHEMA: &str = "config.schema";
    /// I/O on a config path.
    pub const CONFIG_IO: &str = "config.io";
    /// Theme or color mapping failed.
    pub const CONFIG_THEME: &str = "config.theme";
}

/// Parse error, optionally with path and line.
#[must_use]
pub fn parse(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::CONFIG_PARSE, message)
        .with_hint("fix the TOML; the last good configuration stays active")
}

/// Schema / type error.
#[must_use]
pub fn schema(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::CONFIG_SCHEMA, message)
        .with_hint("see default/config.toml and docs/schemas/config.schema.json")
}

/// Filesystem error.
#[must_use]
pub fn io(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::CONFIG_IO, message).with_hint("check path permissions")
}

/// Theme error.
#[must_use]
pub fn theme(message: impl Into<String>) -> CoreError {
    CoreError::new(codes::CONFIG_THEME, message)
        .with_hint("check colors.toml keys or ~/.config/omacell/theme.toml")
}

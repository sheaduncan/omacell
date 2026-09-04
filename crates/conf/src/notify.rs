//! Desktop notifications: `omarchy-notification-send`, else freedesktop.

use std::collections::HashMap;
use std::process::{Command, Stdio};

use omacell_core::error::CoreError;
use tracing::debug;

use crate::agent::on_path;
use crate::schema::Config;

/// Notification class (spec §7.6).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotifyKind {
    /// Autosave / crash recovery.
    Recovery,
    /// Changeset proposed by an external agent.
    AgentProposal,
    /// Long recalc finished, export complete, and other optional events.
    Other,
}

/// Whether `[integrations] notifications` allows this kind.
#[must_use]
pub fn allowed(config: &Config, kind: NotifyKind) -> bool {
    match config.integrations.notifications.as_str() {
        "off" => false,
        "all" => true,
        // Default `recovery_only`: recovery and agent proposals still fire.
        _ => matches!(kind, NotifyKind::Recovery | NotifyKind::AgentProposal),
    }
}

/// Send a notification when policy allows. Failures are logged, never fatal.
pub fn send(config: &Config, kind: NotifyKind, title: &str, body: &str) {
    if !allowed(config, kind) {
        return;
    }
    if let Err(err) = send_inner(title, body) {
        debug!(error = %err.message, "desktop notification failed");
    }
}

fn send_inner(title: &str, body: &str) -> Result<(), CoreError> {
    // This Omarchy helper accepts options before its positional title/body but
    // has no portable `--` sentinel. Fall back to D-Bus for option-like text.
    if omarchy_args_safe(title, body) && on_path("omarchy-notification-send") {
        let status = Command::new("omarchy-notification-send")
            .arg(title)
            .arg(body)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|err| CoreError::new("notify.send", err.to_string()))?;
        if status.success() {
            return Ok(());
        }
        return Err(CoreError::new(
            "notify.send",
            format!("omarchy-notification-send exited {status}"),
        ));
    }
    freedesktop(title, body)
}

fn omarchy_args_safe(title: &str, body: &str) -> bool {
    !title.starts_with('-') && !body.starts_with('-')
}

fn freedesktop(title: &str, body: &str) -> Result<(), CoreError> {
    let conn = zbus::blocking::Connection::session()
        .map_err(|err| CoreError::new("notify.dbus", err.to_string()))?;
    let hints: HashMap<String, zbus::zvariant::Value<'_>> = HashMap::new();
    conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &(
            "Omacell",
            0u32,
            "",
            title,
            body,
            Vec::<String>::new(),
            hints,
            5_000i32,
        ),
    )
    .map_err(|err| CoreError::new("notify.dbus", err.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::omarchy_args_safe;

    #[test]
    fn option_like_notification_text_bypasses_the_omarchy_helper() {
        assert!(omarchy_args_safe("Omacell", "saved"));
        assert!(!omarchy_args_safe("--app-name=other", "saved"));
        assert!(!omarchy_args_safe("Omacell", "--icon=other"));
    }
}

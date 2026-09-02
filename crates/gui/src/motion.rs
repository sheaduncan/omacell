//! Desktop reduced-motion integration.

use std::process::{Command, Stdio};

/// Resolve Omacell's `system | on | off` preference against the desktop hint.
#[must_use]
pub(crate) fn animations_enabled(preference: &str, desktop_enabled: bool) -> bool {
    match preference {
        "on" => true,
        "off" => false,
        _ => desktop_enabled,
    }
}

/// Apply the resolved preference to egui's animation manager.
pub(crate) fn apply(ctx: &egui::Context, preference: &str) {
    let enabled = animations_enabled(preference, desktop_animations_enabled());
    ctx.style_mut(|style| {
        style.animation_time = if enabled { 1.0 / 12.0 } else { 0.0 };
    });
}

fn desktop_animations_enabled() -> bool {
    if let Some(value) = std::env::var_os("OMACELL_REDUCED_MOTION") {
        return !matches!(value.to_string_lossy().as_ref(), "1" | "true" | "yes");
    }
    let Ok(output) = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "enable-animations"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return true;
    };
    output.status.success() && String::from_utf8_lossy(&output.stdout).trim() != "false"
}

#[cfg(test)]
mod tests {
    use super::animations_enabled;

    #[test]
    fn explicit_motion_setting_overrides_desktop() {
        assert!(animations_enabled("on", false));
        assert!(!animations_enabled("off", true));
        assert!(!animations_enabled("system", false));
        assert!(animations_enabled("system", true));
    }
}

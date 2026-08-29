//! `keys check` against Hyprland `bindings.lua` (spec §7.4).

use std::path::Path;

/// A chord Omacell binds that also appears in Hyprland config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyConflict {
    /// Chord text (`SUPER + ALT + X`).
    pub chord: String,
    /// Omacell command id.
    pub omacell: String,
}

/// Parse `o.bind("CHORD", …)` lines from Hyprland bindings.lua.
#[must_use]
pub fn parse_hypr_chords(lua: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in lua.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("o.bind(")
            && let Some(q) = rest.find('"')
        {
            let rest = &rest[q + 1..];
            if let Some(end) = rest.find('"') {
                out.push(rest[..end].to_string());
            }
        }
    }
    out
}

/// Omacell never binds Super; report Super chords as informational host bindings.
#[must_use]
pub fn conflicts(hypr_chords: &[String], omacell_chords: &[(&str, &str)]) -> Vec<KeyConflict> {
    let mut out = Vec::new();
    for (chord, cmd) in omacell_chords {
        let super_chord = chord.to_ascii_uppercase().contains("SUPER");
        if super_chord {
            out.push(KeyConflict {
                chord: (*chord).into(),
                omacell: (*cmd).into(),
            });
            continue;
        }
        for h in hypr_chords {
            if normalize(h) == normalize(chord) {
                out.push(KeyConflict {
                    chord: (*chord).into(),
                    omacell: (*cmd).into(),
                });
            }
        }
    }
    out
}

fn normalize(s: &str) -> String {
    s.to_ascii_uppercase().replace(' ', "")
}

/// Read `~/.config/hypr/bindings.lua` if present.
pub fn check_hyprland(
    bindings_lua: &Path,
    omacell_chords: &[(&str, &str)],
) -> Result<Vec<KeyConflict>, omacell_core::error::CoreError> {
    if !bindings_lua.is_file() {
        return Ok(Vec::new());
    }
    let text =
        std::fs::read_to_string(bindings_lua).map_err(|e| crate::error::io(e.to_string()))?;
    Ok(conflicts(&parse_hypr_chords(&text), omacell_chords))
}

/// Classic keymap chords that `keys check` compares (Appendix A subset).
pub const CLASSIC_CHORDS: &[(&str, &str)] = &[
    ("Ctrl+Z", "edit.undo"),
    ("Ctrl+S", "file.save"),
    ("Ctrl+P", "file.print"),
    ("F1", "help.keys"),
];

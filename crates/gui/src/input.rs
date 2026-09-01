//! Map egui input onto toolkit-neutral [`omacell_ui::KeyEvent`].

use egui::{Event, Key, Modifiers, PointerButton};
use omacell_ui::{KeyCode, KeyEvent};

/// Convert pressed egui keys. Unknown keys are ignored.
#[must_use]
pub fn map_key(key: Key, modifiers: Modifiers) -> Option<KeyEvent> {
    let code = match key {
        Key::Space => KeyCode::Space,
        Key::Enter => KeyCode::Enter,
        Key::Escape => KeyCode::Esc,
        Key::Tab => KeyCode::Tab,
        Key::Backspace => KeyCode::Backspace,
        Key::Delete => KeyCode::Delete,
        Key::Home => KeyCode::Home,
        Key::End => KeyCode::End,
        Key::PageUp => KeyCode::PageUp,
        Key::PageDown => KeyCode::PageDown,
        Key::ArrowLeft => KeyCode::Left,
        Key::ArrowRight => KeyCode::Right,
        Key::ArrowUp => KeyCode::Up,
        Key::ArrowDown => KeyCode::Down,
        Key::F1 => KeyCode::F(1),
        Key::F2 => KeyCode::F(2),
        Key::F3 => KeyCode::F(3),
        Key::F4 => KeyCode::F(4),
        Key::F5 => KeyCode::F(5),
        Key::F6 => KeyCode::F(6),
        Key::F7 => KeyCode::F(7),
        Key::F8 => KeyCode::F(8),
        Key::F9 => KeyCode::F(9),
        Key::F10 => KeyCode::F(10),
        Key::F11 => KeyCode::F(11),
        Key::F12 => KeyCode::F(12),
        other => {
            let name = other.name();
            let mut chars = name.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            KeyCode::Char(if modifiers.shift {
                c
            } else {
                c.to_ascii_lowercase()
            })
        }
    };
    Some(KeyEvent {
        code,
        ctrl: modifiers.ctrl || modifiers.command,
        alt: modifiers.alt,
        shift: modifiers.shift,
    })
}

/// Iterate pressed-key events from this frame.
pub fn pressed_keys(events: &[Event]) -> impl Iterator<Item = KeyEvent> + '_ {
    events.iter().filter_map(|event| match event {
        Event::Key {
            key,
            pressed: true,
            modifiers,
            repeat: _,
            physical_key: _,
        } => map_key(*key, *modifiers),
        _ => None,
    })
}

/// IME / composed text this frame.
pub fn text_events(events: &[Event]) -> impl Iterator<Item = &str> + '_ {
    events.iter().filter_map(|event| match event {
        Event::Text(text) => Some(text.as_str()),
        Event::Ime(egui::ImeEvent::Commit(text)) => Some(text.as_str()),
        _ => None,
    })
}

/// Whether the toolkit requested a clipboard copy this frame.
#[must_use]
pub fn copy_requested(events: &[Event]) -> bool {
    events.iter().any(|event| matches!(event, Event::Copy))
}

/// Whether the toolkit requested a clipboard cut this frame.
#[must_use]
pub fn cut_requested(events: &[Event]) -> bool {
    events.iter().any(|event| matches!(event, Event::Cut))
}

/// Latest toolkit-provided external clipboard text this frame.
#[must_use]
pub fn pasted_text(events: &[Event]) -> Option<&str> {
    events.iter().rev().find_map(|event| match event {
        Event::Paste(text) => Some(text.as_str()),
        _ => None,
    })
}

/// Primary pointer press in screen coordinates.
#[must_use]
pub fn pointer_press(events: &[Event]) -> Option<(egui::Pos2, bool, bool)> {
    events.iter().find_map(|event| match event {
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers,
        } => Some((*pos, modifiers.ctrl || modifiers.command, modifiers.shift)),
        _ => None,
    })
}

/// Secondary pointer press (context menu).
#[must_use]
pub fn pointer_secondary(events: &[Event]) -> Option<egui::Pos2> {
    events.iter().find_map(|event| match event {
        Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed: true,
            ..
        } => Some(*pos),
        _ => None,
    })
}

/// Latest pointer move this frame.
#[must_use]
pub fn pointer_moved(events: &[Event]) -> Option<egui::Pos2> {
    events.iter().rev().find_map(|event| match event {
        Event::PointerMoved(pos) => Some(*pos),
        _ => None,
    })
}

/// Primary pointer release.
#[must_use]
pub fn pointer_release(events: &[Event]) -> Option<(egui::Pos2, bool)> {
    events.iter().find_map(|event| match event {
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers,
        } => Some((*pos, modifiers.ctrl || modifiers.command)),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::map_key;
    use egui::{Key, Modifiers};
    use omacell_ui::KeyCode;

    #[test]
    fn maps_arrows_and_ctrl() {
        let event = map_key(Key::ArrowRight, Modifiers::CTRL).unwrap();
        assert_eq!(event.code, KeyCode::Right);
        assert!(event.ctrl);
    }
}

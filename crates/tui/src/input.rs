//! Map crossterm events onto toolkit-neutral [`omacell_ui::KeyEvent`].

use crossterm::event::{
    KeyCode as CCode, KeyEvent as CEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use omacell_ui::{KeyCode, KeyEvent};

/// Convert a crossterm key. Unknown keys and key-up events are ignored (`None`).
#[must_use]
pub fn map_key(event: CEvent) -> Option<KeyEvent> {
    if !matches!(event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    let code = match event.code {
        CCode::Char(' ') => KeyCode::Space,
        CCode::Char(c) => KeyCode::Char(c),
        CCode::F(n) => KeyCode::F(n),
        CCode::Enter => KeyCode::Enter,
        CCode::Esc => KeyCode::Esc,
        CCode::Tab => KeyCode::Tab,
        CCode::BackTab => KeyCode::Tab,
        CCode::Backspace => KeyCode::Backspace,
        CCode::Delete => KeyCode::Delete,
        CCode::Home => KeyCode::Home,
        CCode::End => KeyCode::End,
        CCode::PageUp => KeyCode::PageUp,
        CCode::PageDown => KeyCode::PageDown,
        CCode::Left => KeyCode::Left,
        CCode::Right => KeyCode::Right,
        CCode::Up => KeyCode::Up,
        CCode::Down => KeyCode::Down,
        _ => return None,
    };
    let mut shift = event.modifiers.contains(KeyModifiers::SHIFT);
    if matches!(event.code, CCode::BackTab) {
        shift = true;
    }
    Some(KeyEvent {
        code,
        ctrl: event.modifiers.contains(KeyModifiers::CONTROL),
        alt: event.modifiers.contains(KeyModifiers::ALT),
        shift,
    })
}

/// Mouse button press in terminal cells, if any. The bool is Ctrl (add to selection).
#[must_use]
pub fn map_mouse(event: MouseEvent) -> Option<(u16, u16, bool)> {
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {
            Some((
                event.column,
                event.row,
                event.modifiers.contains(KeyModifiers::CONTROL),
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventState, MouseButton};

    fn press(code: CCode, mods: KeyModifiers) -> CEvent {
        CEvent {
            code,
            modifiers: mods,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn maps_ctrl_shift_p_and_backtab() {
        let p = map_key(press(
            CCode::Char('p'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ))
        .expect("p");
        assert_eq!(p.code, KeyCode::Char('p'));
        assert!(p.ctrl && p.shift);
        assert_eq!(p.chord(), "Ctrl+Shift+P");

        let tab = map_key(press(CCode::BackTab, KeyModifiers::SHIFT)).expect("backtab");
        assert_eq!(tab.code, KeyCode::Tab);
        assert!(tab.shift);

        let release = CEvent {
            kind: KeyEventKind::Release,
            ..press(CCode::Char('q'), KeyModifiers::CONTROL)
        };
        assert!(map_key(release).is_none());
    }

    #[test]
    fn only_left_mouse_selection_is_mapped() {
        let event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: 12,
            row: 4,
            modifiers: KeyModifiers::NONE,
        };
        assert!(map_mouse(event).is_none());
        assert_eq!(
            map_mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                ..event
            }),
            Some((12, 4, false))
        );
    }
}

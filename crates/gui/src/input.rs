//! Map egui input onto toolkit-neutral [`omacell_ui::KeyEvent`].

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use egui::{Event, Key, Modifiers, PointerButton};
use omacell_ui::{KeyCode, KeyEvent};

const DUPLICATE_COMMIT_WINDOW: Duration = Duration::from_millis(250);
const MAX_PENDING_COMMITS: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
enum TextSource {
    Plain,
    Ime,
}

struct PendingCommit {
    source: TextSource,
    text: String,
    received: Instant,
}

/// Coalesces duplicate text delivered through egui's plain and IME paths.
#[derive(Default)]
pub(crate) struct TextCommitFilter {
    pending: VecDeque<PendingCommit>,
}

impl TextCommitFilter {
    /// Remove duplicate text events, including matching commits delayed across frames.
    pub(crate) fn filter_events(&mut self, events: &mut Vec<Event>) {
        self.filter_events_at(events, Instant::now());
    }

    fn filter_events_at(&mut self, events: &mut Vec<Event>, now: Instant) {
        self.pending.retain(|commit| {
            now.saturating_duration_since(commit.received) <= DUPLICATE_COMMIT_WINDOW
        });
        events.retain(|event| {
            let (source, text) = match event {
                Event::Text(text) => (TextSource::Plain, text.as_str()),
                Event::Ime(egui::ImeEvent::Commit(text)) => (TextSource::Ime, text.as_str()),
                _ => return true,
            };
            if let Some(index) = self
                .pending
                .iter()
                .position(|pending| pending.source != source && pending.text == text)
            {
                self.pending.remove(index);
                return false;
            }
            self.pending.push_back(PendingCommit {
                source,
                text: text.to_owned(),
                received: now,
            });
            if self.pending.len() > MAX_PENDING_COMMITS {
                self.pending.pop_front();
            }
            true
        });
    }
}

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
        other => KeyCode::Char(logical_character(other, modifiers.shift)?),
    };
    Some(KeyEvent {
        code,
        ctrl: modifiers.ctrl || modifiers.command,
        alt: modifiers.alt,
        shift: modifiers.shift,
    })
}

fn logical_character(key: Key, shift: bool) -> Option<char> {
    let character = match (key, shift) {
        (Key::Num0, true) => ')',
        (Key::Num1, true) => '!',
        (Key::Num2, true) => '@',
        (Key::Num3, true) => '#',
        (Key::Num4, true) => '$',
        (Key::Num5, true) => '%',
        (Key::Num6, true) => '^',
        (Key::Num7, true) => '&',
        (Key::Num8, true) => '*',
        (Key::Num9, true) => '(',
        (Key::Backtick, true) => '~',
        (Key::Minus, true) => '_',
        (Key::Equals, true) => '+',
        (Key::Plus, true) => '=',
        (Key::Plus, false) => '+',
        (Key::Semicolon, true) => ':',
        (Key::Colon, true) => ';',
        (Key::Colon, false) => ':',
        (Key::Quote, true) => '"',
        (Key::Backslash, true) | (Key::Pipe, _) => '|',
        (Key::Slash, true) | (Key::Questionmark, _) => '?',
        (Key::OpenBracket, true) | (Key::OpenCurlyBracket, _) => '{',
        (Key::CloseBracket, true) | (Key::CloseCurlyBracket, _) => '}',
        (Key::Exclamationmark, _) => '!',
        (other, _) => {
            let name = other.name();
            let mut chars = name.chars();
            let character = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            if shift {
                character
            } else {
                character.to_ascii_lowercase()
            }
        }
    };
    Some(character)
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

/// IME / composed text this frame after duplicate filtering.
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
    use std::time::{Duration, Instant};

    use super::{DUPLICATE_COMMIT_WINDOW, TextCommitFilter, map_key, text_events};
    use egui::{Key, Modifiers};
    use omacell_ui::KeyCode;

    #[test]
    fn maps_arrows_and_ctrl() {
        let event = map_key(Key::ArrowRight, Modifiers::CTRL).unwrap();
        assert_eq!(event.code, KeyCode::Right);
        assert!(event.ctrl);
    }

    #[test]
    fn maps_shifted_punctuation_to_logical_keymap_characters() {
        let at = map_key(Key::Num2, Modifiers::CTRL | Modifiers::SHIFT).unwrap();
        assert_eq!(at.code, KeyCode::Char('@'));
        assert!(at.ctrl && at.shift);

        let quote = map_key(Key::Quote, Modifiers::SHIFT).unwrap();
        assert_eq!(quote.code, KeyCode::Char('"'));

        let underscore = map_key(Key::Minus, Modifiers::CTRL | Modifiers::SHIFT).unwrap();
        assert_eq!(underscore.code, KeyCode::Char('_'));

        let insert = map_key(Key::Plus, Modifiers::CTRL | Modifiers::SHIFT).unwrap();
        assert_eq!(insert.code, KeyCode::Char('='));
        let time = map_key(Key::Colon, Modifiers::CTRL | Modifiers::SHIFT).unwrap();
        assert_eq!(time.code, KeyCode::Char(';'));
    }

    #[test]
    fn coalesces_each_plain_and_ime_commit_pair_without_losing_repeats() {
        let events = vec![
            egui::Event::Text("/".into()),
            egui::Event::Ime(egui::ImeEvent::Commit("/".into())),
            egui::Event::Text("/".into()),
            egui::Event::Ime(egui::ImeEvent::Commit("/".into())),
        ];

        let mut events = events;
        TextCommitFilter::default().filter_events(&mut events);
        assert_eq!(text_events(&events).collect::<Vec<_>>(), vec!["/", "/"]);
    }

    #[test]
    fn coalesces_a_matching_ime_commit_from_the_next_frame() {
        let mut filter = TextCommitFilter::default();
        let mut plain = vec![egui::Event::Text("a".into())];
        let mut ime = vec![egui::Event::Ime(egui::ImeEvent::Commit("a".into()))];

        filter.filter_events(&mut plain);
        filter.filter_events(&mut ime);

        assert_eq!(text_events(&plain).collect::<Vec<_>>(), vec!["a"]);
        assert!(text_events(&ime).next().is_none());
    }

    #[test]
    fn retains_a_matching_cross_source_commit_after_the_pairing_window() {
        let mut filter = TextCommitFilter::default();
        let now = Instant::now();
        let mut plain = vec![egui::Event::Text("a".into())];
        let mut later_ime = vec![egui::Event::Ime(egui::ImeEvent::Commit("a".into()))];

        filter.filter_events_at(&mut plain, now);
        filter.filter_events_at(
            &mut later_ime,
            now + DUPLICATE_COMMIT_WINDOW + Duration::from_millis(1),
        );

        assert_eq!(text_events(&plain).collect::<Vec<_>>(), vec!["a"]);
        assert_eq!(text_events(&later_ime).collect::<Vec<_>>(), vec!["a"]);
    }
}

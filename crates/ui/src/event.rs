//! Toolkit-neutral key events (crossterm and winit map into this).

/// A key without toolkit types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyCode {
    /// Printable character (shift already applied by the frontend).
    Char(char),
    /// Function key `F1`–`F12`.
    F(u8),
    /// Enter / Return.
    Enter,
    /// Escape.
    Esc,
    /// Tab.
    Tab,
    /// Backspace.
    Backspace,
    /// Delete.
    Delete,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Space.
    Space,
}

/// One key press with modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    /// Key.
    pub code: KeyCode,
    /// Ctrl.
    pub ctrl: bool,
    /// Alt / Mod1.
    pub alt: bool,
    /// Shift.
    pub shift: bool,
}

impl KeyEvent {
    /// Unmodified key.
    #[must_use]
    pub fn new(code: KeyCode) -> Self {
        Self {
            code,
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    /// Canonical chord text used in keymap files (`Ctrl+Shift+P`).
    #[must_use]
    pub fn chord(&self) -> String {
        let named = !matches!(self.code, KeyCode::Char(_));
        let modified_char = !named && (self.ctrl || self.alt);
        let mut out = String::new();
        if self.ctrl {
            out.push_str("Ctrl+");
        }
        if self.alt {
            out.push_str("Alt+");
        }
        if self.shift && (named || modified_char) {
            out.push_str("Shift+");
        }
        match self.code {
            KeyCode::Char(' ') | KeyCode::Space => out.push_str("Space"),
            KeyCode::Char(c) if modified_char => out.push(c.to_ascii_uppercase()),
            KeyCode::Char(c) => out.push(c),
            KeyCode::F(n) => {
                out.push('F');
                out.push_str(&n.to_string());
            }
            KeyCode::Enter => out.push_str("Enter"),
            KeyCode::Esc => out.push_str("Esc"),
            KeyCode::Tab => out.push_str("Tab"),
            KeyCode::Backspace => out.push_str("Backspace"),
            KeyCode::Delete => out.push_str("Delete"),
            KeyCode::Home => out.push_str("Home"),
            KeyCode::End => out.push_str("End"),
            KeyCode::PageUp => out.push_str("PgUp"),
            KeyCode::PageDown => out.push_str("PgDn"),
            KeyCode::Left => out.push_str("Left"),
            KeyCode::Right => out.push_str("Right"),
            KeyCode::Up => out.push_str("Up"),
            KeyCode::Down => out.push_str("Down"),
        }
        out
    }
}

//! Tiny cursor over ASCII-heavy address text.

pub(super) struct Cursor<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Cursor<'a> {
    pub(super) fn new(s: &'a str) -> Self {
        Self { s, i: 0 }
    }

    pub(super) fn rest(&self) -> &'a str {
        &self.s[self.i..]
    }

    pub(super) fn is_empty(&self) -> bool {
        self.i >= self.s.len()
    }

    pub(super) fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    pub(super) fn bump(&mut self) {
        if let Some(ch) = self.peek() {
            self.i += ch.len_utf8();
        }
    }

    pub(super) fn eat_char(&mut self, want: char) -> bool {
        if self.peek() == Some(want) {
            self.bump();
            true
        } else {
            false
        }
    }

    pub(super) fn eat_char_ci(&mut self, want: char) -> bool {
        match self.peek() {
            Some(ch) if ch.eq_ignore_ascii_case(&want) => {
                self.bump();
                true
            }
            _ => false,
        }
    }

    pub(super) fn eat_while<F: FnMut(char) -> bool>(&mut self, mut pred: F) -> &'a str {
        let start = self.i;
        while let Some(ch) = self.peek() {
            if !pred(ch) {
                break;
            }
            self.bump();
        }
        &self.s[start..self.i]
    }
}

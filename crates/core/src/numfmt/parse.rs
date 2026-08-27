//! Format-code scanner. Never panics on input.

use crate::error::{CoreError, codes};
use crate::locale::LocaleId;
use crate::numfmt::token::{
    AmPmStyle, CmpOp, ColorHint, Condition, DigitKind, MAX_FORMAT_LEN, NamedColor, ParsedFormat,
    Section, Token,
};

/// Parse an Excel number format code.
pub fn parse(input: &str) -> Result<ParsedFormat, CoreError> {
    if input.len() > MAX_FORMAT_LEN || input.chars().count() > MAX_FORMAT_LEN {
        return Err(CoreError::new(
            codes::NUMFMT_PARSE,
            "number format exceeds 255 characters",
        ));
    }
    if input.is_empty() || input.eq_ignore_ascii_case("General") {
        return Ok(ParsedFormat::general());
    }
    let parts = split_sections(input);
    let mut sections = Vec::with_capacity(parts.len().min(4));
    for part in parts.iter().take(4) {
        sections.push(parse_section(part)?);
    }
    resolve_minutes(&mut sections);
    for section in &mut sections {
        literalize_condition_text(section);
    }
    Ok(ParsedFormat { sections })
}

fn split_sections(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = input.chars().peekable();
    let mut in_quote = false;
    let mut in_bracket = false;
    while let Some(c) = chars.next() {
        match c {
            '"' if !in_bracket => {
                in_quote = !in_quote;
                cur.push(c);
            }
            '[' if !in_quote => {
                in_bracket = true;
                cur.push(c);
            }
            ']' if !in_quote => {
                in_bracket = false;
                cur.push(c);
            }
            ';' if !in_quote && !in_bracket => {
                if out.len() >= 3 {
                    cur.push(c);
                } else {
                    out.push(std::mem::take(&mut cur));
                }
            }
            '\\' if !in_quote && !in_bracket => {
                cur.push(c);
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn parse_section(src: &str) -> Result<Section, CoreError> {
    let mut s = Scanner::new(src);
    let mut condition = None;
    let mut color = None;
    let mut locale = None;
    let mut tokens = Vec::new();
    while !s.eof() {
        match s.peek() {
            Some('"') => tokens.push(Token::Literal(s.read_quoted())),
            Some('\\') => {
                s.bump();
                if let Some(c) = s.bump() {
                    tokens.push(Token::Literal(c.to_string()));
                }
            }
            Some('[') => match parse_bracket(&mut s)? {
                Bracket::Color(c) => color = Some(c),
                Bracket::Condition(c) => condition = Some(c),
                Bracket::Locale { id, curr } => {
                    locale = id.or(locale);
                    if let Some(c) = curr {
                        tokens.insert(0, Token::Literal(c));
                    }
                }
                Bracket::Elapsed(t) => tokens.push(t),
                Bracket::Literal(t) => tokens.push(Token::Literal(t)),
            },
            Some('_') => {
                s.bump();
                tokens.push(Token::Skip(s.bump().unwrap_or(' ')));
            }
            Some('*') => {
                s.bump();
                tokens.push(Token::Fill(s.bump().unwrap_or(' ')));
            }
            Some('@') => {
                s.bump();
                tokens.push(Token::TextPlaceholder);
            }
            Some('%') => {
                s.bump();
                tokens.push(Token::Percent);
            }
            Some('.') => {
                if looks_like_subsec(&tokens, &s) {
                    s.bump();
                    let mut n = 0u8;
                    while s.peek() == Some('0') {
                        s.bump();
                        n = n.saturating_add(1);
                    }
                    tokens.push(Token::SubSecond { len: n.max(1) });
                } else {
                    s.bump();
                    tokens.push(Token::Decimal);
                }
            }
            Some(',') => {
                s.bump();
                tokens.push(Token::Grouping);
            }
            Some('/') => {
                s.bump();
                let has_digit = tokens.iter().any(|t| matches!(t, Token::Digit(_)));
                if has_digit {
                    tokens.push(Token::FractionBar);
                } else {
                    tokens.push(Token::Literal("/".to_string()));
                }
            }
            Some('0') => {
                s.bump();
                tokens.push(Token::Digit(DigitKind::Zero));
            }
            Some('#') => {
                s.bump();
                tokens.push(Token::Digit(DigitKind::Hash));
            }
            Some('?') => {
                s.bump();
                tokens.push(Token::Digit(DigitKind::Question));
            }
            Some(c) if c == 'e' || c == 'E' => {
                if let Some(tok) = s.try_exp() {
                    tokens.push(tok);
                } else {
                    tokens.extend(s.read_letter_run());
                }
            }
            Some(c) if c.is_ascii_alphabetic() => {
                if let Some(tok) = s.try_ampm() {
                    tokens.push(tok);
                } else {
                    tokens.extend(s.read_letter_run());
                }
            }
            Some(_) => {
                let c = s.bump().unwrap_or(' ');
                tokens.push(Token::Literal(c.to_string()));
            }
            None => break,
        }
    }
    Ok(Section {
        condition,
        color,
        locale,
        currency: None,
        tokens,
    })
}

fn looks_like_subsec(tokens: &[Token], s: &Scanner) -> bool {
    let last = tokens.iter().rev().find(|t| {
        matches!(
            t,
            Token::Second { .. } | Token::Hour { .. } | Token::Minute { .. }
        )
    });
    matches!(last, Some(Token::Second { .. })) && s.after_dot_is_zeros()
}

enum Bracket {
    Color(ColorHint),
    Condition(Condition),
    Locale {
        id: Option<LocaleId>,
        curr: Option<String>,
    },
    Elapsed(Token),
    Literal(String),
}

fn parse_bracket(s: &mut Scanner) -> Result<Bracket, CoreError> {
    s.bump();
    let mut inner = String::new();
    while let Some(c) = s.peek() {
        if c == ']' {
            s.bump();
            break;
        }
        inner.push(c);
        s.bump();
        if inner.len() > 64 {
            return Err(CoreError::new(
                codes::NUMFMT_PARSE,
                "format bracket is too long",
            ));
        }
    }
    if let Some(color) = parse_color(&inner) {
        return Ok(Bracket::Color(color));
    }
    if let Some(cond) = parse_condition(&inner) {
        return Ok(Bracket::Condition(cond));
    }
    if let Some(tok) = parse_elapsed(&inner) {
        return Ok(Bracket::Elapsed(tok));
    }
    if let Some(loc) = parse_locale_bracket(&inner) {
        return Ok(Bracket::Locale {
            id: loc.0,
            curr: loc.1,
        });
    }
    Ok(Bracket::Literal(inner))
}

fn parse_color(inner: &str) -> Option<ColorHint> {
    let t = inner.trim();
    let named = match t.to_ascii_lowercase().as_str() {
        "black" => Some(NamedColor::Black),
        "white" => Some(NamedColor::White),
        "red" => Some(NamedColor::Red),
        "green" => Some(NamedColor::Green),
        "blue" => Some(NamedColor::Blue),
        "yellow" => Some(NamedColor::Yellow),
        "magenta" => Some(NamedColor::Magenta),
        "cyan" => Some(NamedColor::Cyan),
        _ => None,
    };
    if let Some(n) = named {
        return Some(ColorHint::Named(n));
    }
    let lower = t.to_ascii_lowercase();
    lower.strip_prefix("color").and_then(|rest| {
        rest.trim()
            .parse::<u8>()
            .ok()
            .filter(|n| (1..=56).contains(n))
            .map(ColorHint::Indexed)
    })
}

fn parse_condition(inner: &str) -> Option<Condition> {
    let t = inner.trim();
    let (op, rest) = if let Some(r) = t.strip_prefix("<>") {
        (CmpOp::Ne, r)
    } else if let Some(r) = t.strip_prefix(">=") {
        (CmpOp::Ge, r)
    } else if let Some(r) = t.strip_prefix("<=") {
        (CmpOp::Le, r)
    } else if let Some(r) = t.strip_prefix('>') {
        (CmpOp::Gt, r)
    } else if let Some(r) = t.strip_prefix('<') {
        (CmpOp::Lt, r)
    } else {
        let r = t.strip_prefix('=')?;
        (CmpOp::Eq, r)
    };
    rest.trim()
        .parse()
        .ok()
        .map(|value| Condition { op, value })
}

fn parse_elapsed(inner: &str) -> Option<Token> {
    match inner.trim().to_ascii_lowercase().as_str() {
        "h" => Some(Token::Hour {
            len: 1,
            elapsed: true,
        }),
        "hh" => Some(Token::Hour {
            len: 2,
            elapsed: true,
        }),
        "m" => Some(Token::Minute {
            len: 1,
            elapsed: true,
        }),
        "mm" => Some(Token::Minute {
            len: 2,
            elapsed: true,
        }),
        "s" => Some(Token::Second {
            len: 1,
            elapsed: true,
        }),
        "ss" => Some(Token::Second {
            len: 2,
            elapsed: true,
        }),
        _ => None,
    }
}

fn parse_locale_bracket(inner: &str) -> Option<(Option<LocaleId>, Option<String>)> {
    let t = inner.trim();
    if !t.starts_with('$') {
        return None;
    }
    let rest = &t[1..];
    if let Some(dash) = rest.rfind('-') {
        let curr = &rest[..dash];
        let loc = &rest[dash + 1..];
        let id = parse_locale_id(loc);
        let currency = if curr.is_empty() {
            None
        } else {
            Some(curr.to_string())
        };
        return Some((id, currency));
    }
    if rest.is_empty() {
        Some((None, None))
    } else {
        Some((None, Some(rest.to_string())))
    }
}

fn parse_locale_id(s: &str) -> Option<LocaleId> {
    let t = s.trim();
    if t.chars().all(|c| c.is_ascii_hexdigit()) && !t.is_empty() {
        let n = u32::from_str_radix(t, 16).ok()?;
        return Some(LocaleId::new(n & 0xFFFF));
    }
    LocaleId::parse_tag(t)
}

fn literalize_condition_text(section: &mut Section) {
    if section.condition.is_none() {
        return;
    }
    let digits = section.tokens.iter().any(|t| matches!(t, Token::Digit(_)));
    if digits {
        return;
    }
    let date_n = section
        .tokens
        .iter()
        .filter(|t| {
            matches!(
                t,
                Token::Year { .. }
                    | Token::Month { .. }
                    | Token::Day { .. }
                    | Token::Hour { .. }
                    | Token::Minute { .. }
                    | Token::Second { .. }
                    | Token::Weekday { .. }
            )
        })
        .count();
    if date_n == 0 || date_n > 2 {
        return;
    }
    for tok in &mut section.tokens {
        *tok = match tok {
            Token::Year { len, .. } => Token::Literal("Y".repeat(*len as usize)),
            Token::Month { len } => Token::Literal("M".repeat(*len as usize)),
            Token::Day { len } => Token::Literal("D".repeat(*len as usize)),
            Token::Hour { len, .. } => Token::Literal("h".repeat(*len as usize)),
            Token::Minute { len, .. } => Token::Literal("m".repeat(*len as usize)),
            Token::Second { len, .. } => Token::Literal("s".repeat(*len as usize)),
            Token::Weekday { len } => Token::Literal("d".repeat(*len as usize)),
            _ => continue,
        };
    }
}

fn is_sep_token(tok: &Token) -> bool {
    match tok {
        Token::Literal(s) => s
            .chars()
            .all(|c| matches!(c, ':' | '-' | '/' | ' ' | '.' | 'T')),
        Token::Skip(_) | Token::Fill(_) => true,
        _ => false,
    }
}

fn resolve_minutes(sections: &mut [Section]) {
    for section in sections {
        let n = section.tokens.len();
        for i in 0..n {
            let Token::Month { len } = section.tokens[i] else {
                continue;
            };
            if len > 2 {
                continue;
            }
            let prev = section.tokens[..i].iter().rev().find(|t| !is_sep_token(t));
            let next = section.tokens[i + 1..].iter().find(|t| !is_sep_token(t));
            let prev_h = matches!(prev, Some(Token::Hour { .. }));
            let next_s = matches!(next, Some(Token::Second { .. } | Token::SubSecond { .. }));
            if prev_h || next_s {
                section.tokens[i] = Token::Minute {
                    len,
                    elapsed: false,
                };
            }
        }
    }
}

struct Scanner {
    chars: Vec<char>,
    i: usize,
}

impl Scanner {
    fn new(src: &str) -> Self {
        Self {
            chars: src.chars().collect(),
            i: 0,
        }
    }
    fn eof(&self) -> bool {
        self.i >= self.chars.len()
    }
    fn peek(&self) -> Option<char> {
        self.chars.get(self.i).copied()
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.chars.get(self.i).copied();
        if c.is_some() {
            self.i += 1;
        }
        c
    }
    fn after_dot_is_zeros(&self) -> bool {
        let mut j = self.i + 1;
        let mut any = false;
        while j < self.chars.len() && self.chars[j] == '0' {
            any = true;
            j += 1;
        }
        any
    }
    fn try_exp(&mut self) -> Option<Token> {
        let e = self.peek()?;
        if e != 'e' && e != 'E' {
            return None;
        }
        let next = self.chars.get(self.i + 1).copied();
        match next {
            Some('+') => {
                self.i += 2;
                Some(Token::Exp { plus: true })
            }
            Some('-') => {
                self.i += 2;
                Some(Token::Exp { plus: false })
            }
            _ => None,
        }
    }

    fn try_ampm(&mut self) -> Option<Token> {
        let rest: String = self.chars[self.i..].iter().collect();
        let lower = rest.to_ascii_lowercase();
        let (n, style) = if lower.starts_with("am/pm") {
            let style = if rest.starts_with('A') || rest.starts_with('P') {
                AmPmStyle::Upper
            } else {
                AmPmStyle::Lower
            };
            (5, style)
        } else if lower.starts_with("a/p") {
            let style = if rest.starts_with('A') || rest.starts_with('P') {
                AmPmStyle::UpperShort
            } else {
                AmPmStyle::LowerShort
            };
            (3, style)
        } else {
            return None;
        };
        self.i += n;
        Some(Token::AmPm { style })
    }
    fn read_quoted(&mut self) -> String {
        self.bump();
        let mut out = String::new();
        while let Some(c) = self.bump() {
            if c == '"' {
                if self.peek() == Some('"') {
                    self.bump();
                    out.push('"');
                } else {
                    break;
                }
            } else {
                out.push(c);
            }
        }
        out
    }
    fn read_letter_run(&mut self) -> Vec<Token> {
        let start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphabetic()) {
            self.bump();
        }
        let run: String = self.chars[start..self.i].iter().collect();
        tokenize_letters(&run)
    }
}

fn tokenize_letters(run: &str) -> Vec<Token> {
    if run.eq_ignore_ascii_case("General") {
        return vec![Token::General];
    }
    let lower = run.to_ascii_lowercase();
    if lower
        .chars()
        .any(|c| !matches!(c, 'y' | 'm' | 'd' | 'h' | 's' | 'e' | 'g' | 'a' | 'p'))
    {
        return vec![Token::Literal(run.to_string())];
    }
    let bytes = lower.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let mut n = 1usize;
        while i + n < bytes.len() && bytes[i + n] == c {
            n += 1;
        }
        let len = n.min(255) as u8;
        match c {
            b'y' => out.push(Token::Year {
                len,
                iso: len >= 3,
                era: false,
            }),
            b'e' => out.push(Token::Year {
                len,
                iso: true,
                era: false,
            }),
            b'g' => out.push(Token::Year {
                len,
                iso: false,
                era: true,
            }),
            b'm' => out.push(Token::Month { len: len.min(5) }),
            b'd' if len >= 3 => out.push(Token::Weekday { len: len.min(4) }),
            b'd' => out.push(Token::Day { len }),
            b'h' => out.push(Token::Hour {
                len: len.min(2),
                elapsed: false,
            }),
            b's' => out.push(Token::Second {
                len: len.min(2),
                elapsed: false,
            }),
            b'a' if len >= 3 => out.push(Token::Weekday {
                len: if len >= 4 { 4 } else { 3 },
            }),
            _ => out.push(Token::Literal(run.chars().skip(i).take(n).collect())),
        }
        i += n;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_four_sections() {
        assert_eq!(parse("0.00;(0.00);zero;@").unwrap().section_count(), 4);
    }

    #[test]
    fn rejects_too_long() {
        assert_eq!(
            parse(&"0".repeat(256)).unwrap_err().code,
            codes::NUMFMT_PARSE
        );
    }

    #[test]
    fn minute_vs_month() {
        let p = parse("h:mm").unwrap();
        assert!(matches!(
            p.sections[0].tokens[2],
            Token::Minute { len: 2, .. }
        ));
        let p = parse("mm-dd").unwrap();
        assert!(matches!(p.sections[0].tokens[0], Token::Month { len: 2 }));
    }
}

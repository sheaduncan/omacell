//! Formula tokenizer.

use crate::addr::{CellRef, col_from_letters};
use crate::error::ErrorKind;
use crate::limits::{MAX_COLS, MAX_ROWS};

use super::RefStyle;
use super::ast::{Span, StructuredRef, TableColumns, TableItem};
use super::error::ParseError;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TokenKind {
    Number(f64),
    String(String),
    Bool(bool),
    Error(ErrorKind),
    Ident(String),
    Cell(CellRef),
    Col { col: u16, abs: bool },
    Row { row: u32, abs: bool },
    SheetQuoted(String),
    ExternalBook(String),
    Structured(StructuredRef),
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Semicolon,
    Colon,
    Bang,
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Amp,
    Percent,
    Hash,
    At,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub leading_ws: bool,
}

pub(crate) struct Lexer<'a> {
    src: &'a str,
    i: usize,
    style: RefStyle,
    base_row: u32,
    base_col: u16,
}

impl<'a> Lexer<'a> {
    pub(crate) fn new(src: &'a str, style: RefStyle, base_row: u32, base_col: u16) -> Self {
        Self {
            src,
            i: 0,
            style,
            base_row,
            base_col,
        }
    }

    pub(crate) fn tokenize(mut self) -> Result<Vec<Token>, ParseError> {
        self.skip_ws();
        if self.peek() == Some('=') {
            self.bump();
        }
        let mut out = Vec::new();
        loop {
            let tok = self.next_token()?;
            let eof = matches!(tok.kind, TokenKind::Eof);
            out.push(tok);
            if eof {
                break;
            }
        }
        Ok(out)
    }

    fn next_token(&mut self) -> Result<Token, ParseError> {
        let leading_ws = self.skip_ws();
        let start = self.i;
        let Some(ch) = self.peek() else {
            return Ok(Token {
                kind: TokenKind::Eof,
                span: Span::new(start, start),
                leading_ws,
            });
        };
        let kind = match ch {
            '(' => {
                self.bump();
                TokenKind::LParen
            }
            ')' => {
                self.bump();
                TokenKind::RParen
            }
            '{' => {
                self.bump();
                TokenKind::LBrace
            }
            '}' => {
                self.bump();
                TokenKind::RBrace
            }
            ',' => {
                self.bump();
                TokenKind::Comma
            }
            ';' => {
                self.bump();
                TokenKind::Semicolon
            }
            ':' => {
                self.bump();
                TokenKind::Colon
            }
            '!' => {
                self.bump();
                TokenKind::Bang
            }
            '+' => {
                self.bump();
                TokenKind::Plus
            }
            '-' => {
                self.bump();
                TokenKind::Minus
            }
            '*' => {
                self.bump();
                TokenKind::Star
            }
            '/' => {
                self.bump();
                TokenKind::Slash
            }
            '^' => {
                self.bump();
                TokenKind::Caret
            }
            '&' => {
                self.bump();
                TokenKind::Amp
            }
            '%' => {
                self.bump();
                TokenKind::Percent
            }
            '@' => {
                self.bump();
                TokenKind::At
            }
            '=' => {
                self.bump();
                TokenKind::Eq
            }
            '<' => {
                self.bump();
                if self.peek() == Some('>') {
                    self.bump();
                    TokenKind::Ne
                } else if self.peek() == Some('=') {
                    self.bump();
                    TokenKind::Le
                } else {
                    TokenKind::Lt
                }
            }
            '>' => {
                self.bump();
                if self.peek() == Some('=') {
                    self.bump();
                    TokenKind::Ge
                } else {
                    TokenKind::Gt
                }
            }
            '"' => self.lex_string(start)?,
            '\'' => self.lex_quoted_sheet(start)?,
            '#' => self.lex_hash(start)?,
            '[' => self.lex_bracket(start)?,
            '.' | '0'..='9' => self.lex_number_or_row(start)?,
            '$' => self.lex_abs_ref(start)?,
            _ if is_ident_start(ch) => self.lex_ident_or_cell(start)?,
            _ => {
                return Err(ParseError::parse(
                    format!("unexpected character {ch:?}"),
                    start,
                    expected_primary(),
                ));
            }
        };
        Ok(Token {
            kind,
            span: Span::new(start, self.i),
            leading_ws,
        })
    }

    fn lex_string(&mut self, start: usize) -> Result<TokenKind, ParseError> {
        self.bump();
        let mut s = String::new();
        loop {
            match self.peek() {
                None => {
                    return Err(ParseError::parse(
                        "unterminated string",
                        start,
                        vec!["string".into()],
                    ));
                }
                Some('"') => {
                    self.bump();
                    if self.peek() == Some('"') {
                        self.bump();
                        s.push('"');
                    } else {
                        return Ok(TokenKind::String(s));
                    }
                }
                Some(ch) => {
                    s.push(ch);
                    self.bump();
                }
            }
        }
    }

    fn lex_quoted_sheet(&mut self, start: usize) -> Result<TokenKind, ParseError> {
        self.bump();
        let mut name = String::new();
        loop {
            match self.peek() {
                None => {
                    return Err(ParseError::parse(
                        "unterminated quoted sheet name",
                        start,
                        vec!["'".into()],
                    ));
                }
                Some('\'') => {
                    self.bump();
                    if self.peek() == Some('\'') {
                        self.bump();
                        name.push('\'');
                    } else {
                        break;
                    }
                }
                Some(ch) => {
                    name.push(ch);
                    self.bump();
                }
            }
        }
        if name.is_empty() {
            return Err(ParseError::parse(
                "empty quoted sheet name",
                start,
                vec!["sheet name".into()],
            ));
        }
        Ok(TokenKind::SheetQuoted(name))
    }

    fn lex_hash(&mut self, start: usize) -> Result<TokenKind, ParseError> {
        if let Some(kind) = self.try_error_literal() {
            return Ok(TokenKind::Error(kind));
        }
        let after = self.i + 1;
        let rest = self.src.get(after..).unwrap_or("");
        if rest
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            return Err(ParseError::parse(
                "unknown error literal",
                start,
                vec!["#REF!".into(), "#N/A".into()],
            ));
        }
        self.bump();
        Ok(TokenKind::Hash)
    }

    fn try_error_literal(&mut self) -> Option<ErrorKind> {
        let rest = &self.src[self.i..];
        let upper = rest.to_ascii_uppercase();
        const TABLE: &[(&str, ErrorKind)] = &[
            ("#GETTING_DATA", ErrorKind::GettingData),
            ("#UNKNOWN!", ErrorKind::Unknown),
            ("#CONNECT!", ErrorKind::Connect),
            ("#BLOCKED!", ErrorKind::Blocked),
            ("#SPILL!", ErrorKind::Spill),
            ("#FIELD!", ErrorKind::Field),
            ("#VALUE!", ErrorKind::Value),
            ("#NULL!", ErrorKind::Null),
            ("#CALC!", ErrorKind::Calc),
            ("#DIV/0!", ErrorKind::Div0),
            ("#NAME?", ErrorKind::Name),
            ("#NUM!", ErrorKind::Num),
            ("#REF!", ErrorKind::Ref),
            ("#N/A", ErrorKind::Na),
        ];
        for (lit, kind) in TABLE {
            if upper.starts_with(*lit) {
                self.i += lit.len();
                return Some(*kind);
            }
        }
        None
    }

    fn lex_bracket(&mut self, start: usize) -> Result<TokenKind, ParseError> {
        let (inner, end_br) = scan_brackets(self.src, self.i)
            .ok_or_else(|| ParseError::parse("unclosed '['", start, vec!["]".into()]))?;
        let after = self.src.get(end_br..).unwrap_or("").trim_start();
        let looks_external =
            after.starts_with('!') || after.starts_with('\'') || starts_with_sheet_then_bang(after);
        if looks_external {
            self.i = end_br;
            if inner.is_empty() {
                return Err(ParseError::parse(
                    "empty external workbook name",
                    start,
                    vec!["workbook".into()],
                ));
            }
            return Ok(TokenKind::ExternalBook(inner.to_string()));
        }
        if inner.contains('.') && !has_unescaped_open_bracket(inner) {
            return Err(ParseError::parse(
                "external workbook missing sheet and '!'",
                start,
                vec!["sheet".into()],
            ));
        }
        self.i = end_br;
        let sr = parse_structured(None, &format!("[{inner}]"), start)?;
        Ok(TokenKind::Structured(sr))
    }

    fn lex_number_or_row(&mut self, start: usize) -> Result<TokenKind, ParseError> {
        match self.try_number(start)? {
            Some(n) => Ok(TokenKind::Number(n)),
            None => Err(ParseError::parse(
                "invalid number",
                start,
                vec!["number".into()],
            )),
        }
    }

    fn try_number(&mut self, start: usize) -> Result<Option<f64>, ParseError> {
        let save = self.i;
        let mut saw_digit = false;
        if self.peek() == Some('.') {
            self.bump();
            if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.i = save;
                return Err(ParseError::parse(
                    "bare dot is not a number",
                    start,
                    vec!["number".into()],
                ));
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                saw_digit = true;
                self.bump();
            }
        } else if self.peek().is_some_and(|c| c.is_ascii_digit()) {
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                saw_digit = true;
                self.bump();
            }
            if self.peek() == Some('.') {
                self.bump();
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.bump();
                }
            }
        } else {
            return Ok(None);
        }
        if !saw_digit {
            self.i = save;
            return Ok(None);
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.bump();
            if matches!(self.peek(), Some('+') | Some('-')) {
                self.bump();
            }
            if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.i = save;
                return Err(ParseError::parse(
                    "incomplete scientific exponent",
                    start,
                    vec!["number".into()],
                ));
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
            }
        }
        if self.peek().is_some_and(is_ident_continue) {
            let ch = self.peek().unwrap_or('?');
            return Err(ParseError::parse(
                format!("invalid numeric literal (unexpected {ch:?})"),
                self.i,
                vec!["operator".into()],
            ));
        }
        let text = &self.src[save..self.i];
        let n: f64 = text
            .parse()
            .map_err(|_| ParseError::parse("invalid number", start, vec!["number".into()]))?;
        if !n.is_finite() {
            return Err(ParseError::parse(
                "numeric literal is outside the finite range",
                start,
                vec!["number".into()],
            ));
        }
        Ok(Some(n))
    }

    fn lex_abs_ref(&mut self, start: usize) -> Result<TokenKind, ParseError> {
        self.bump();
        if self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            let letters_start = self.i;
            while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                self.bump();
            }
            let letters = &self.src[letters_start..self.i];
            let col = col_from_letters(letters).map_err(|_| {
                ParseError::parse(
                    "invalid column in absolute reference",
                    start,
                    vec!["cell".into()],
                )
            })?;
            let row_abs = self.peek() == Some('$');
            if row_abs {
                self.bump();
            }
            if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                let digits_start = self.i;
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.bump();
                }
                let digits = &self.src[digits_start..self.i];
                let row = parse_row_number(digits)
                    .map_err(|msg| ParseError::parse(msg, start, vec!["cell".into()]))?;
                if self.peek().is_some_and(is_ident_continue) {
                    return Err(ParseError::parse(
                        "invalid cell reference",
                        start,
                        vec!["cell".into()],
                    ));
                }
                let cell = CellRef::with_abs(row, col, row_abs, true).map_err(|_| {
                    ParseError::parse("cell out of range", start, vec!["cell".into()])
                })?;
                return Ok(TokenKind::Cell(cell));
            }
            if row_abs {
                return Err(ParseError::parse(
                    "absolute $ missing row number",
                    start,
                    vec!["row".into()],
                ));
            }
            return Ok(TokenKind::Col { col, abs: true });
        }
        if self.peek().is_some_and(|c| c.is_ascii_digit()) {
            let digits_start = self.i;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
            }
            let digits = &self.src[digits_start..self.i];
            let row = parse_row_number(digits)
                .map_err(|msg| ParseError::parse(msg, start, vec!["row".into()]))?;
            return Ok(TokenKind::Row { row, abs: true });
        }
        Err(ParseError::parse("stray $", start, vec!["cell".into()]))
    }

    fn lex_ident_or_cell(&mut self, start: usize) -> Result<TokenKind, ParseError> {
        if self.style == RefStyle::R1C1
            && let Some(kind) = self.try_r1c1(start)?
        {
            return Ok(kind);
        }
        if let Some(kind) = self.try_a1_cell(start)? {
            return Ok(kind);
        }
        while self.peek().is_some_and(is_ident_continue) {
            self.bump();
        }
        let ident = self.src[start..self.i].to_string();
        let next_sig = self.peek_non_ws();
        if ident.eq_ignore_ascii_case("TRUE") && next_sig != Some('(') {
            return Ok(TokenKind::Bool(true));
        }
        if ident.eq_ignore_ascii_case("FALSE") && next_sig != Some('(') {
            return Ok(TokenKind::Bool(false));
        }
        if next_sig == Some('[') {
            let br = self.skip_ws_from(self.i);
            let (_inner, end_br) = scan_brackets(self.src, br).ok_or_else(|| {
                ParseError::parse("unclosed structured reference", start, vec!["]".into()])
            })?;
            self.i = end_br;
            let raw = &self.src[self.skip_ws_from(start + ident.len())..end_br];
            let sr = parse_structured(Some(ident), raw, start)?;
            return Ok(TokenKind::Structured(sr));
        }
        Ok(TokenKind::Ident(ident))
    }

    fn try_a1_cell(&mut self, start: usize) -> Result<Option<TokenKind>, ParseError> {
        if !self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            return Ok(None);
        }
        let save = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.bump();
        }
        let letters = &self.src[save..self.i];
        let row_abs = self.peek() == Some('$');
        if row_abs {
            self.bump();
        }
        if self.peek().is_some_and(|c| c.is_ascii_digit()) {
            let d0 = self.i;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
            }
            if self.peek().is_some_and(is_ident_continue) {
                self.i = save;
                return Ok(None);
            }
            if self.peek_non_ws() == Some('(') {
                self.i = save;
                return Ok(None);
            }
            let digits = &self.src[d0..self.i];
            let Ok(col) = col_from_letters(letters) else {
                self.i = save;
                return Ok(None);
            };
            let Ok(row) = parse_row_number(digits) else {
                self.i = save;
                return Ok(None);
            };
            let cell = CellRef::with_abs(row, col, row_abs, false)
                .map_err(|_| ParseError::parse("cell out of range", start, vec!["cell".into()]))?;
            return Ok(Some(TokenKind::Cell(cell)));
        }
        if row_abs {
            return Err(ParseError::parse(
                "absolute $ missing row number",
                start,
                vec!["row".into()],
            ));
        }
        self.i = save;
        Ok(None)
    }

    fn try_r1c1(&mut self, start: usize) -> Result<Option<TokenKind>, ParseError> {
        let save = self.i;
        let Some(first) = self.peek() else {
            return Ok(None);
        };
        if !first.eq_ignore_ascii_case(&'r') && !first.eq_ignore_ascii_case(&'c') {
            return Ok(None);
        }
        let row = if self.peek().is_some_and(|c| c.eq_ignore_ascii_case(&'r')) {
            self.bump();
            match self.parse_r1c1_axis(self.base_row, MAX_ROWS, start)? {
                Some(v) => Some(v),
                None => {
                    self.i = save;
                    return Ok(None);
                }
            }
        } else {
            None
        };
        let col = if self.peek().is_some_and(|c| c.eq_ignore_ascii_case(&'c')) {
            self.bump();
            match self.parse_r1c1_axis(u32::from(self.base_col), u32::from(MAX_COLS), start)? {
                Some((idx, abs)) => {
                    let col = u16::try_from(idx).map_err(|_| {
                        ParseError::parse("R1C1 column out of range", start, vec!["cell".into()])
                    })?;
                    Some((col, abs))
                }
                None => {
                    self.i = save;
                    return Ok(None);
                }
            }
        } else {
            None
        };
        if self.peek().is_some_and(is_ident_continue) {
            self.i = save;
            return Ok(None);
        }
        if self.peek_non_ws() == Some('(') {
            self.i = save;
            return Ok(None);
        }
        let kind = match (row, col) {
            (Some((r, ra)), Some((c, ca))) => {
                let cell = CellRef::with_abs(r, c, ra, ca).map_err(|_| {
                    ParseError::parse("R1C1 cell out of range", start, vec!["cell".into()])
                })?;
                TokenKind::Cell(cell)
            }
            (Some((r, abs)), None) => TokenKind::Row { row: r, abs },
            (None, Some((c, abs))) => TokenKind::Col { col: c, abs },
            (None, None) => {
                self.i = save;
                return Ok(None);
            }
        };
        Ok(Some(kind))
    }

    fn parse_r1c1_axis(
        &mut self,
        base: u32,
        count: u32,
        start: usize,
    ) -> Result<Option<(u32, bool)>, ParseError> {
        if self.peek() == Some('[') {
            self.bump();
            let num_start = self.i;
            if self.peek() == Some('-') {
                self.bump();
            }
            if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
                return Err(ParseError::parse(
                    "invalid R1C1 offset",
                    start,
                    vec!["offset".into()],
                ));
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
            }
            let num = &self.src[num_start..self.i];
            if self.peek() != Some(']') {
                return Err(ParseError::parse(
                    "unterminated R1C1 offset",
                    start,
                    vec!["]".into()],
                ));
            }
            self.bump();
            let off: i32 = num.parse().map_err(|_| {
                ParseError::parse("invalid R1C1 offset", start, vec!["offset".into()])
            })?;
            let v = add_offset(base, off, count)
                .map_err(|m| ParseError::parse(m, start, vec!["cell".into()]))?;
            return Ok(Some((v, false)));
        }
        if self.peek().is_some_and(|c| c.is_ascii_digit()) {
            let d0 = self.i;
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
            }
            let digits = &self.src[d0..self.i];
            let n: u32 = digits
                .parse()
                .map_err(|_| ParseError::parse("invalid R1C1 index", start, vec!["cell".into()]))?;
            if n == 0 || n > count {
                return Err(ParseError::parse(
                    format!("R1C1 index {n} is out of range"),
                    start,
                    vec!["cell".into()],
                ));
            }
            return Ok(Some((n - 1, true)));
        }
        Ok(Some((base, false)))
    }

    fn peek(&self) -> Option<char> {
        self.src[self.i..].chars().next()
    }

    fn peek_non_ws(&self) -> Option<char> {
        self.src[self.i..].chars().find(|c| !is_ws(*c))
    }

    fn bump(&mut self) {
        if let Some(ch) = self.peek() {
            self.i += ch.len_utf8();
        }
    }

    fn skip_ws(&mut self) -> bool {
        let start = self.i;
        while self.peek().is_some_and(is_ws) {
            self.bump();
        }
        self.i > start
    }

    fn skip_ws_from(&self, mut i: usize) -> usize {
        while i < self.src.len() {
            let Some(ch) = self.src[i..].chars().next() else {
                break;
            };
            if !is_ws(ch) {
                break;
            }
            i += ch.len_utf8();
        }
        i
    }
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '\\'
}

fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == '?' || c == '\\'
}

fn expected_primary() -> Vec<String> {
    ["number", "string", "name", "cell", "(", "{"]
        .into_iter()
        .map(String::from)
        .collect()
}

fn parse_row_number(digits: &str) -> Result<u32, String> {
    let n: u32 = digits
        .parse()
        .map_err(|_| format!("row {digits} is out of range"))?;
    if n == 0 || n > MAX_ROWS {
        Err(format!("row {n} is out of range"))
    } else {
        Ok(n - 1)
    }
}

fn add_offset(base: u32, off: i32, count: u32) -> Result<u32, String> {
    let v = i64::from(base) + i64::from(off);
    if v < 0 || v >= i64::from(count) {
        Err(format!("relative reference {base}{off:+} is out of range"))
    } else {
        Ok(v as u32)
    }
}

fn starts_with_sheet_then_bang(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !is_ident_start(first) {
        return false;
    }
    let rest = chars.as_str();
    let ident_len = rest
        .find(|c: char| !is_ident_continue(c) && c != ':')
        .unwrap_or(rest.len());
    rest[ident_len..].starts_with('!')
}

fn scan_brackets(src: &str, start: usize) -> Option<(&str, usize)> {
    if src.as_bytes().get(start) != Some(&b'[') {
        return None;
    }
    let mut depth = 0i32;
    let mut i = start;
    while i < src.len() {
        if let Some(len) = column_escape_len(src, i) {
            i += len;
            continue;
        }
        let ch = src[i..].chars().next()?;
        let n = ch.len_utf8();
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    i += 1;
                    return Some((&src[start + 1..i - 1], i));
                }
            }
            _ => {}
        }
        i += n;
    }
    None
}

fn has_unescaped_open_bracket(src: &str) -> bool {
    let mut i = 0;
    while i < src.len() {
        if let Some(len) = column_escape_len(src, i) {
            i += len;
            continue;
        }
        let Some(ch) = src[i..].chars().next() else {
            break;
        };
        if ch == '[' {
            return true;
        }
        i += ch.len_utf8();
    }
    false
}

pub(crate) fn parse_structured(
    table: Option<String>,
    raw_inner: &str,
    offset: usize,
) -> Result<StructuredRef, ParseError> {
    let inner = raw_inner.trim().to_string();
    if !inner.starts_with('[') {
        return Err(ParseError::parse(
            "structured reference must start with '['",
            offset,
            vec!["[".into()],
        ));
    }
    let body = strip_outer_brackets(&inner).ok_or_else(|| {
        ParseError::parse("unbalanced structured reference", offset, vec!["]".into()])
    })?;
    let mut item = None;
    let mut this_row = false;
    let mut columns = None;
    let t = body.trim();
    if t.is_empty() {
    } else if t == "@" {
        this_row = true;
    } else if let Some(rest) = t.strip_prefix('@') {
        this_row = true;
        let rest = rest.trim();
        if rest.starts_with('[') {
            let name = strip_outer_brackets(rest).unwrap_or(rest);
            columns = Some(TableColumns::One(decode_col(name)));
        } else if !rest.is_empty() {
            columns = Some(TableColumns::One(decode_col(rest)));
        }
    } else if let Some(it) = TableItem::parse(t) {
        item = Some(it);
        if it == TableItem::ThisRow {
            this_row = true;
        }
    } else if t.starts_with('#') {
        return Err(ParseError::parse(
            format!("unknown structured specifier {t}"),
            offset,
            vec!["#All".into(), "#Data".into(), "#Headers".into()],
        ));
    } else if t.starts_with('[') {
        parse_double_bracket_body(t, offset, &mut item, &mut this_row, &mut columns)?;
    } else {
        columns = Some(TableColumns::One(decode_col(t)));
    }
    Ok(StructuredRef {
        table,
        item,
        this_row,
        columns,
        inner,
    })
}

fn parse_double_bracket_body(
    t: &str,
    offset: usize,
    item: &mut Option<TableItem>,
    this_row: &mut bool,
    columns: &mut Option<TableColumns>,
) -> Result<(), ParseError> {
    let inner = strip_outer_brackets(t).unwrap_or(t);
    let parts = split_struct_parts(inner);
    if parts.is_empty() {
        return Ok(());
    }
    let mut idx = 0;
    if parts[0].trim() == "@" {
        *this_row = true;
        idx = 1;
    } else if let Some(it) = TableItem::parse(parts[0].trim()) {
        *item = Some(it);
        if it == TableItem::ThisRow {
            *this_row = true;
        }
        idx = 1;
    }
    let rest = &parts[idx..];
    if rest.is_empty() {
        return Ok(());
    }
    if rest.len() == 1 {
        let p = rest[0].trim();
        if p.starts_with('@') {
            *this_row = true;
            let name = p.trim_start_matches('@').trim();
            let name = strip_outer_brackets(name).unwrap_or(name);
            if !name.is_empty() {
                *columns = Some(TableColumns::One(decode_col(name)));
            }
        } else if let Some((a, b)) = split_col_span(p) {
            *columns = Some(TableColumns::Span {
                start: decode_col(a),
                end: decode_col(b),
            });
        } else if p.starts_with('#') {
            return Err(ParseError::parse(
                format!("unknown structured specifier {p}"),
                offset,
                vec!["#All".into()],
            ));
        } else {
            let name = strip_outer_brackets(p).unwrap_or(p);
            *columns = Some(TableColumns::One(decode_col(name)));
        }
        return Ok(());
    }
    if rest.len() >= 2 {
        let a = strip_outer_brackets(rest[0].trim()).unwrap_or(rest[0].trim());
        let last = rest[rest.len() - 1].trim();
        let b = strip_outer_brackets(last).unwrap_or(last);
        if rest.len() == 2 || rest.iter().any(|p| p.contains(':')) {
            *columns = Some(TableColumns::Span {
                start: decode_col(a),
                end: decode_col(b),
            });
            return Ok(());
        }
        *columns = Some(TableColumns::One(decode_col(a)));
    }
    Ok(())
}

fn split_struct_parts(inner: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let mut i = 0;
    while i < inner.len() {
        if let Some(len) = column_escape_len(inner, i) {
            i += len;
            continue;
        }
        let ch = inner[i..].chars().next().unwrap_or('\0');
        let n = ch.len_utf8();
        match ch {
            '[' => depth += 1,
            ']' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += n;
    }
    parts.push(&inner[start..]);
    parts
}

fn split_col_span(p: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    let mut i = 0;
    while i < p.len() {
        if let Some(len) = column_escape_len(p, i) {
            i += len;
            continue;
        }
        let ch = p[i..].chars().next()?;
        match ch {
            '[' => depth += 1,
            ']' => depth -= 1,
            ':' if depth == 0 => {
                let a = p[..i].trim();
                let b = p[i + 1..].trim();
                let a = strip_outer_brackets(a).unwrap_or(a);
                let b = strip_outer_brackets(b).unwrap_or(b);
                return Some((a, b));
            }
            _ => {}
        }
        i += ch.len_utf8();
    }
    None
}

fn strip_outer_brackets(s: &str) -> Option<&str> {
    let s = s.trim();
    if !s.starts_with('[') || !s.ends_with(']') || s.len() < 2 {
        return None;
    }
    let mut depth = 0i32;
    let mut i = 0;
    while i < s.len() {
        if let Some(len) = column_escape_len(s, i) {
            i += len;
            continue;
        }
        let ch = s[i..].chars().next()?;
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return if i + ch.len_utf8() == s.len() {
                        Some(&s[1..i])
                    } else {
                        None
                    };
                }
            }
            _ => {}
        }
        i += ch.len_utf8();
    }
    None
}

fn decode_col(s: &str) -> String {
    let s = s.trim();
    let mut decoded = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if let Some(len) = column_escape_len(s, i) {
            let escaped = s[i + 1..].chars().next().unwrap_or('\'');
            decoded.push(escaped);
            i += len;
            continue;
        }
        let ch = s[i..].chars().next().unwrap_or('\0');
        decoded.push(ch);
        i += ch.len_utf8();
    }
    decoded
}

fn column_escape_len(src: &str, index: usize) -> Option<usize> {
    if src.as_bytes().get(index) != Some(&b'\'') {
        return None;
    }
    let escaped = src.get(index + 1..)?.chars().next()?;
    matches!(escaped, '[' | ']' | '#' | '\'' | '@').then_some(1 + escaped.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        let toks = Lexer::new(src, RefStyle::A1, 0, 0).tokenize().unwrap();
        toks.into_iter()
            .filter(|t| !matches!(t.kind, TokenKind::Eof))
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn number_forms() {
        assert!(matches!(&kinds("=1e3")[0], TokenKind::Number(n) if *n == 1000.0));
        assert!(matches!(&kinds("=.5")[0], TokenKind::Number(n) if *n == 0.5));
        assert!(matches!(&kinds("=5.")[0], TokenKind::Number(n) if *n == 5.0));
    }

    #[test]
    fn string_escape() {
        match &kinds("=\"a\"\"b\"")[0] {
            TokenKind::String(s) => assert_eq!(s, "a\"b"),
            _ => panic!("string"),
        }
    }

    #[test]
    fn log10_call_is_ident() {
        let k = kinds("=LOG10(100)");
        assert!(matches!(&k[0], TokenKind::Ident(s) if s.eq_ignore_ascii_case("LOG10")));
        assert!(matches!(k[1], TokenKind::LParen));
    }

    #[test]
    fn log10_alone_is_cell() {
        assert!(matches!(kinds("=LOG10")[0], TokenKind::Cell(_)));
    }
}

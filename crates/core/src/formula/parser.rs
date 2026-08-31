//! Pratt parser for Excel formulas (F-3.1).

use crate::addr::{CellRef, RangeRef, SheetSpec, col_from_letters};
use crate::limits::{MAX_COLS, MAX_FORMULA_LEN, MAX_ROWS};

use super::ast::{BinOp, Callee, Expr, ExprKind, PostfixOp, PrefixOp, Span};
use super::error::ParseError;
use super::lexer::{Lexer, Token, TokenKind};
use super::{Formula, MAX_FORMULA_DEPTH, ParseOptions, PartialParse, RefStyle};

/// Parse a formula in A1 style.
///
/// ```
/// use omacell_core::formula::{parse, print};
/// let f = parse("=A1+1").unwrap();
/// assert_eq!(print(&f), "=A1+1");
/// ```
pub fn parse(src: &str) -> Result<Formula, ParseError> {
    parse_with(src, ParseOptions::default())
}

/// Parse with A1/R1C1 options.
pub fn parse_with(src: &str, opts: ParseOptions) -> Result<Formula, ParseError> {
    Parser::new(src, opts)?.parse_formula()
}

/// Error-tolerant parse for the editor (F-5.2 colourization / autocomplete).
///
/// ```
/// use omacell_core::formula::parse_editor;
/// let p = parse_editor("=SUM(A1,");
/// assert!(p.error.is_some());
/// ```
pub fn parse_editor(src: &str) -> PartialParse {
    parse_editor_with(src, ParseOptions::default())
}

/// Editor parse with options.
pub fn parse_editor_with(src: &str, mut opts: ParseOptions) -> PartialParse {
    opts.lenient = true;
    match Parser::new(src, opts) {
        Err(e) => PartialParse {
            expr: None,
            error: Some(e),
        },
        Ok(mut p) => match p.parse_formula() {
            Ok(f) => PartialParse {
                expr: Some(f.ast),
                error: None,
            },
            Err(e) => PartialParse {
                expr: p.partial.take(),
                error: Some(e),
            },
        },
    }
}

struct Parser<'a> {
    tokens: Vec<Token>,
    i: usize,
    opts: ParseOptions,
    partial: Option<Expr>,
    in_args: bool,
    _src: &'a str,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str, opts: ParseOptions) -> Result<Self, ParseError> {
        if src.len() > MAX_FORMULA_LEN {
            return Err(ParseError::length(format!(
                "formula is {} bytes; max is {MAX_FORMULA_LEN}",
                src.len()
            )));
        }
        if opts.style == RefStyle::R1C1
            && (opts.base_row >= MAX_ROWS || u32::from(opts.base_col) >= u32::from(MAX_COLS))
        {
            return Err(ParseError::parse(
                "R1C1 base cell is out of range",
                0,
                vec!["valid base cell".into()],
            ));
        }
        let tokens = Lexer::new(src, opts.style, opts.base_row, opts.base_col).tokenize()?;
        Ok(Self {
            tokens,
            i: 0,
            opts,
            partial: None,
            in_args: false,
            _src: src,
        })
    }

    fn parse_formula(&mut self) -> Result<Formula, ParseError> {
        if matches!(self.peek_kind(), TokenKind::Eof) {
            return Err(self.err("empty formula", expected_primary()));
        }
        let ast = self.parse_expr(0, 0)?;
        if !matches!(self.peek_kind(), TokenKind::Eof) {
            return Err(self.err(
                "unexpected token after expression",
                vec!["end of formula".into()],
            ));
        }
        Ok(Formula {
            ast,
            style: self.opts.style,
            base_row: self.opts.base_row,
            base_col: self.opts.base_col,
        })
    }

    fn parse_expr(&mut self, min_bp: u8, depth: u32) -> Result<Expr, ParseError> {
        if depth >= MAX_FORMULA_DEPTH {
            return Err(ParseError::depth(
                "formula nesting exceeds 64",
                self.peek_offset(),
            ));
        }
        let mut lhs = self.parse_prefix(depth)?;
        self.partial = Some(lhs.clone());
        loop {
            if let Some(op) = self.peek_postfix() {
                let (lbp, _) = postfix_bp(op);
                if lbp < min_bp {
                    break;
                }
                if matches!(op, PostfixOp::Spill)
                    && matches!(
                        lhs.kind,
                        ExprKind::Postfix {
                            op: PostfixOp::Spill,
                            ..
                        }
                    )
                {
                    return Err(self.err("double spill #", vec!["operator".into()]));
                }
                self.bump();
                let span = lhs.span.union(self.prev_span());
                lhs = Expr::new(
                    ExprKind::Postfix {
                        expr: Box::new(lhs),
                        op,
                    },
                    span,
                );
                self.partial = Some(lhs.clone());
                continue;
            }
            if matches!(self.peek_kind(), TokenKind::LParen) && can_call(&lhs.kind) {
                let (lbp, _) = (CALL_BP, CALL_BP);
                if lbp < min_bp {
                    break;
                }
                lhs = self.finish_call(lhs, depth)?;
                self.partial = Some(lhs.clone());
                continue;
            }
            if let Some(op) = self.peek_infix() {
                let (lbp, rbp) = infix_bp(op);
                if lbp < min_bp {
                    break;
                }
                self.bump();
                let right = self.parse_expr(rbp, depth + 1)?;
                lhs = self.make_binary(op, lhs, right)?;
                self.partial = Some(lhs.clone());
                continue;
            }
            if self.peek_isect() {
                let (lbp, rbp) = infix_bp(BinOp::Isect);
                if lbp < min_bp {
                    break;
                }
                let right = self.parse_expr(rbp, depth + 1)?;
                lhs = self.make_binary(BinOp::Isect, lhs, right)?;
                self.partial = Some(lhs.clone());
                continue;
            }
            break;
        }
        Ok(lhs)
    }

    fn parse_prefix(&mut self, depth: u32) -> Result<Expr, ParseError> {
        if depth >= MAX_FORMULA_DEPTH {
            return Err(ParseError::depth(
                "formula nesting exceeds 64",
                self.peek_offset(),
            ));
        }
        let tok = self.peek().clone();
        match &tok.kind {
            TokenKind::Plus => {
                self.bump();
                let expr = self.parse_expr(prefix_bp(PrefixOp::Plus), depth + 1)?;
                let span = tok.span.union(expr.span);
                Ok(Expr::new(
                    ExprKind::Prefix {
                        op: PrefixOp::Plus,
                        expr: Box::new(expr),
                    },
                    span,
                ))
            }
            TokenKind::Minus => {
                self.bump();
                let expr = self.parse_expr(prefix_bp(PrefixOp::Minus), depth + 1)?;
                let span = tok.span.union(expr.span);
                Ok(Expr::new(
                    ExprKind::Prefix {
                        op: PrefixOp::Minus,
                        expr: Box::new(expr),
                    },
                    span,
                ))
            }
            TokenKind::At => {
                if matches!(self.kind_at(self.i + 1), Some(TokenKind::At)) {
                    return Err(self.err("double @", vec!["reference".into()]));
                }
                self.bump();
                let expr = self.parse_expr(prefix_bp(PrefixOp::ImplicitIntersect), depth + 1)?;
                let span = tok.span.union(expr.span);
                Ok(Expr::new(
                    ExprKind::Prefix {
                        op: PrefixOp::ImplicitIntersect,
                        expr: Box::new(expr),
                    },
                    span,
                ))
            }
            TokenKind::Number(n) => {
                self.bump();
                Ok(Expr::new(ExprKind::Number(*n), tok.span))
            }
            TokenKind::String(s) => {
                self.bump();
                Ok(Expr::new(ExprKind::String(s.clone()), tok.span))
            }
            TokenKind::Bool(b) => {
                self.bump();
                Ok(Expr::new(ExprKind::Bool(*b), tok.span))
            }
            TokenKind::Error(e) => {
                self.bump();
                Ok(Expr::new(ExprKind::Error(*e), tok.span))
            }
            TokenKind::LBrace => self.parse_array(depth),
            TokenKind::LParen => {
                let open_at = tok.span.start as usize;
                self.bump();
                if matches!(self.peek_kind(), TokenKind::RParen) {
                    return Err(ParseError::parse(
                        "empty parentheses",
                        open_at,
                        expected_primary(),
                    ));
                }
                let saved = self.in_args;
                self.in_args = false;
                let inner = self.parse_expr(0, depth + 1)?;
                self.in_args = saved;
                self.expect_rparen()?;
                let span = tok.span.union(self.prev_span());
                Ok(Expr::new(ExprKind::Paren(Box::new(inner)), span))
            }
            TokenKind::ExternalBook(book) => {
                let book = book.clone();
                self.bump();
                let inner = self.parse_sheet_qualified(depth)?;
                let span = tok.span.union(inner.span);
                Ok(Expr::new(
                    ExprKind::External {
                        workbook: book,
                        inner: Box::new(inner),
                        quoted: false,
                    },
                    span,
                ))
            }
            TokenKind::SheetQuoted(name) => {
                let name = name.clone();
                self.bump();
                self.parse_after_sheet_name(name, tok.span, depth)
            }
            TokenKind::Ident(_)
            | TokenKind::Cell(_)
            | TokenKind::Col { .. }
            | TokenKind::Row { .. } => self.parse_ref_or_call(depth),
            TokenKind::Structured(sr) => {
                self.bump();
                Ok(Expr::new(ExprKind::Structured(sr.clone()), tok.span))
            }
            TokenKind::Eof => Err(self.err("unexpected end of formula", expected_primary())),
            _ => Err(self.err("unexpected token in expression", expected_primary())),
        }
    }

    fn parse_after_sheet_name(
        &mut self,
        name: String,
        name_span: Span,
        depth: u32,
    ) -> Result<Expr, ParseError> {
        let (book, sheet) = split_quoted_external(&name);
        self.expect_bang()?;
        let spec = parse_sheet_spec(&sheet)?;
        let inner = self.parse_ref_body(depth)?;
        let expr = attach_sheet(inner, spec)?;
        let span = name_span.union(expr.span);
        let expr = Expr { span, ..expr };
        if let Some(book) = book {
            Ok(Expr::new(
                ExprKind::External {
                    workbook: book,
                    inner: Box::new(expr),
                    quoted: true,
                },
                span,
            ))
        } else {
            Ok(expr)
        }
    }

    fn parse_sheet_qualified(&mut self, depth: u32) -> Result<Expr, ParseError> {
        match self.peek_kind().clone() {
            TokenKind::SheetQuoted(name) => {
                let span = self.peek().span;
                self.bump();
                self.parse_after_sheet_name(name, span, depth)
            }
            TokenKind::Ident(_) => self.parse_ref_or_call(depth),
            _ => self.parse_ref_body(depth),
        }
    }

    fn parse_ref_or_call(&mut self, depth: u32) -> Result<Expr, ParseError> {
        if let TokenKind::Ident(start) = self.peek_kind().clone() {
            if self.looks_like_3d() {
                self.bump();
                self.bump();
                let TokenKind::Ident(end) = self.peek_kind().clone() else {
                    return Err(self.err("expected sheet name", vec!["sheet".into()]));
                };
                self.bump();
                self.expect_bang()?;
                let inner = self.parse_ref_body(depth)?;
                let sheets = SheetSpec {
                    start,
                    end: Some(end),
                };
                let span = inner.span;
                return Ok(Expr::new(
                    ExprKind::ThreeD {
                        sheets,
                        inner: Box::new(inner),
                    },
                    span,
                ));
            }
            if self.lookahead_bang_after_ident() {
                let name = start;
                let name_span = self.peek().span;
                self.bump();
                self.expect_bang()?;
                let inner = self.parse_ref_body(depth)?;
                let spec = SheetSpec {
                    start: name,
                    end: None,
                };
                let mut expr = attach_sheet(inner, spec)?;
                expr.span = name_span.union(expr.span);
                return Ok(expr);
            }
        }
        if let TokenKind::Ident(name) = self.peek_kind().clone() {
            let span = self.peek().span;
            self.bump();
            if matches!(self.peek_kind(), TokenKind::LParen) {
                return self.finish_named_call(name, span, depth);
            }
            return Ok(Expr::new(ExprKind::Name { sheet: None, name }, span));
        }
        self.parse_ref_atom()
    }

    fn parse_ref_body(&mut self, depth: u32) -> Result<Expr, ParseError> {
        self.parse_expr(BP_RANGE, depth)
    }

    fn parse_ref_atom(&mut self) -> Result<Expr, ParseError> {
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Cell(cell) => {
                self.bump();
                Ok(Expr::new(ExprKind::Cell { sheet: None, cell }, tok.span))
            }
            TokenKind::Col { col, abs } => {
                let prev_colon = self.prev_is_colon();
                self.bump();
                if self.opts.style == super::RefStyle::A1
                    && !matches!(self.peek_kind(), TokenKind::Colon)
                    && !prev_colon
                {
                    return Err(self.err(
                        "incomplete whole-column reference (need A:A)",
                        vec![":".into()],
                    ));
                }
                Ok(col_expr(col, abs, tok.span))
            }
            TokenKind::Row { row, abs } => {
                let prev_colon = self.prev_is_colon();
                self.bump();
                if self.opts.style == super::RefStyle::A1
                    && !matches!(self.peek_kind(), TokenKind::Colon)
                    && !prev_colon
                {
                    return Err(self.err(
                        "incomplete whole-row reference (need 1:1)",
                        vec![":".into()],
                    ));
                }
                Ok(row_expr(row, abs, tok.span))
            }
            TokenKind::Structured(sr) => {
                self.bump();
                Ok(Expr::new(ExprKind::Structured(sr), tok.span))
            }
            TokenKind::Ident(name) => {
                self.bump();
                Ok(Expr::new(ExprKind::Name { sheet: None, name }, tok.span))
            }
            TokenKind::Number(n) => {
                self.bump();
                Ok(Expr::new(ExprKind::Number(n), tok.span))
            }
            _ => Err(self.err("expected a reference", vec!["cell".into(), "name".into()])),
        }
    }

    fn finish_named_call(
        &mut self,
        name: String,
        name_span: Span,
        depth: u32,
    ) -> Result<Expr, ParseError> {
        self.expect_lparen()?;
        let args = self.parse_args(depth)?;
        let span = name_span.union(self.prev_span());
        Ok(Expr::new(
            ExprKind::Call {
                callee: Callee::Name(name),
                args,
            },
            span,
        ))
    }

    fn finish_call(&mut self, lhs: Expr, depth: u32) -> Result<Expr, ParseError> {
        self.expect_lparen()?;
        let args = self.parse_args(depth)?;
        let span = lhs.span.union(self.prev_span());
        let callee = match lhs.kind {
            ExprKind::Name { name, sheet: None } => Callee::Name(name),
            other => Callee::Expr(Box::new(Expr::new(other, lhs.span))),
        };
        Ok(Expr::new(ExprKind::Call { callee, args }, span))
    }

    fn parse_args(&mut self, depth: u32) -> Result<Vec<Option<Expr>>, ParseError> {
        let mut args = Vec::new();
        if matches!(self.peek_kind(), TokenKind::RParen) {
            self.bump();
            return Ok(args);
        }
        let saved = self.in_args;
        self.in_args = true;
        let result = self.parse_args_inner(depth, &mut args);
        self.in_args = saved;
        result?;
        Ok(args)
    }

    fn parse_args_inner(
        &mut self,
        depth: u32,
        args: &mut Vec<Option<Expr>>,
    ) -> Result<(), ParseError> {
        loop {
            if matches!(self.peek_kind(), TokenKind::Comma | TokenKind::RParen) {
                args.push(None);
            } else {
                args.push(Some(self.parse_expr(0, depth + 1)?));
            }
            if matches!(self.peek_kind(), TokenKind::Comma) {
                self.bump();
                if matches!(self.peek_kind(), TokenKind::RParen) {
                    args.push(None);
                    self.bump();
                    return Ok(());
                }
                continue;
            }
            if matches!(self.peek_kind(), TokenKind::RParen) {
                self.bump();
                return Ok(());
            }
            if matches!(self.peek_kind(), TokenKind::Eof) && self.opts.lenient {
                return Ok(());
            }
            return Err(self.err("expected ',' or ')'", vec![",".into(), ")".into()]));
        }
    }

    fn parse_array(&mut self, depth: u32) -> Result<Expr, ParseError> {
        let start = self.peek().span;
        self.bump();
        if matches!(self.peek_kind(), TokenKind::RBrace) {
            return Err(self.err("empty array constant", vec!["number".into()]));
        }
        let mut rows: Vec<Vec<Expr>> = Vec::new();
        let mut row: Vec<Expr> = Vec::new();
        loop {
            let cell = self.parse_array_scalar(depth)?;
            row.push(cell);
            match self.peek_kind() {
                TokenKind::Comma => {
                    self.bump();
                    if matches!(
                        self.peek_kind(),
                        TokenKind::Comma | TokenKind::Semicolon | TokenKind::RBrace
                    ) {
                        return Err(self.err("empty array slot", vec!["literal".into()]));
                    }
                }
                TokenKind::Semicolon => {
                    self.bump();
                    if row.is_empty()
                        || matches!(
                            self.peek_kind(),
                            TokenKind::Semicolon | TokenKind::RBrace | TokenKind::Comma
                        )
                    {
                        return Err(self.err("empty array row", vec!["literal".into()]));
                    }
                    rows.push(std::mem::take(&mut row));
                }
                TokenKind::RBrace => {
                    rows.push(row);
                    self.bump();
                    break;
                }
                _ => {
                    return Err(self.err(
                        "expected ',' ';' or '}' in array constant",
                        vec![",".into(), ";".into(), "}".into()],
                    ));
                }
            }
        }
        let width = rows[0].len();
        if rows.iter().any(|r| r.len() != width) {
            return Err(ParseError::parse(
                "ragged array constant",
                start.start as usize,
                vec!["matching row width".into()],
            ));
        }
        let span = start.union(self.prev_span());
        Ok(Expr::new(ExprKind::Array(rows), span))
    }

    fn parse_array_scalar(&mut self, depth: u32) -> Result<Expr, ParseError> {
        if depth >= MAX_FORMULA_DEPTH {
            return Err(ParseError::depth(
                "formula nesting exceeds 64",
                self.peek_offset(),
            ));
        }
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Plus | TokenKind::Minus => {
                let op = if matches!(tok.kind, TokenKind::Plus) {
                    PrefixOp::Plus
                } else {
                    PrefixOp::Minus
                };
                self.bump();
                let expr = self.parse_array_scalar(depth + 1)?;
                if !matches!(
                    expr.kind,
                    ExprKind::Number(_) | ExprKind::Postfix { .. } | ExprKind::Prefix { .. }
                ) {
                    return Err(
                        self.err("array constants allow only scalars", vec!["number".into()])
                    );
                }
                let span = tok.span.union(expr.span);
                Ok(Expr::new(
                    ExprKind::Prefix {
                        op,
                        expr: Box::new(expr),
                    },
                    span,
                ))
            }
            TokenKind::Number(n) => {
                self.bump();
                let mut expr = Expr::new(ExprKind::Number(n), tok.span);
                if matches!(self.peek_kind(), TokenKind::Percent) {
                    self.bump();
                    let span = expr.span.union(self.prev_span());
                    expr = Expr::new(
                        ExprKind::Postfix {
                            expr: Box::new(expr),
                            op: PostfixOp::Percent,
                        },
                        span,
                    );
                }
                Ok(expr)
            }
            TokenKind::String(s) => {
                self.bump();
                Ok(Expr::new(ExprKind::String(s), tok.span))
            }
            TokenKind::Bool(b) => {
                self.bump();
                Ok(Expr::new(ExprKind::Bool(b), tok.span))
            }
            TokenKind::Error(e) => {
                self.bump();
                Ok(Expr::new(ExprKind::Error(e), tok.span))
            }
            TokenKind::LBrace => {
                Err(self.err("nested array constants are illegal", vec!["number".into()]))
            }
            TokenKind::Ident(_)
            | TokenKind::Cell(_)
            | TokenKind::Structured(_)
            | TokenKind::LParen => Err(self.err(
                "array constants cannot contain references or calls",
                vec!["number".into(), "string".into()],
            )),
            _ => Err(self.err(
                "invalid array constant element",
                vec!["number".into(), "string".into(), "TRUE".into()],
            )),
        }
    }

    fn make_binary(&self, op: BinOp, left: Expr, right: Expr) -> Result<Expr, ParseError> {
        let span = left.span.union(right.span);
        if op == BinOp::Range {
            if matches!(left.kind, ExprKind::ThreeD { .. })
                || matches!(right.kind, ExprKind::ThreeD { .. })
            {
                return Err(ParseError::parse(
                    "3-D reference cannot be a range operand",
                    span.start as usize,
                    vec![":".into()],
                ));
            }
            if let Some(folded) = fold_range(&left, &right) {
                return folded.map(|kind| Expr::new(kind, span));
            }
            if is_mixed_whole(&left, &right) {
                return Err(ParseError::parse(
                    "range sides must both be cells, both whole rows, or both whole columns",
                    span.start as usize,
                    vec![":".into()],
                ));
            }
        }
        if op == BinOp::Isect && (!is_isect_operand(&left.kind) || !is_isect_operand(&right.kind)) {
            return Err(ParseError::parse(
                "intersection operands must be references",
                left.span.start as usize,
                vec!["cell".into()],
            ));
        }
        Ok(Expr::new(
            ExprKind::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
            span,
        ))
    }

    fn looks_like_3d(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Ident(_))
            && matches!(self.kind_at(self.i + 1), Some(TokenKind::Colon))
            && matches!(self.kind_at(self.i + 2), Some(TokenKind::Ident(_)))
            && matches!(self.kind_at(self.i + 3), Some(TokenKind::Bang))
    }

    fn lookahead_bang_after_ident(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Ident(_))
            && matches!(self.kind_at(self.i + 1), Some(TokenKind::Bang))
    }

    fn peek_postfix(&self) -> Option<PostfixOp> {
        match self.peek_kind() {
            TokenKind::Percent => Some(PostfixOp::Percent),
            TokenKind::Hash => Some(PostfixOp::Spill),
            _ => None,
        }
    }

    fn peek_infix(&self) -> Option<BinOp> {
        match self.peek_kind() {
            TokenKind::Colon => Some(BinOp::Range),
            TokenKind::Comma if !self.in_args => Some(BinOp::Union),
            TokenKind::Caret => Some(BinOp::Pow),
            TokenKind::Star => Some(BinOp::Mul),
            TokenKind::Slash => Some(BinOp::Div),
            TokenKind::Plus => Some(BinOp::Add),
            TokenKind::Minus => Some(BinOp::Sub),
            TokenKind::Amp => Some(BinOp::Concat),
            TokenKind::Eq => Some(BinOp::Eq),
            TokenKind::Ne => Some(BinOp::Ne),
            TokenKind::Lt => Some(BinOp::Lt),
            TokenKind::Le => Some(BinOp::Le),
            TokenKind::Gt => Some(BinOp::Gt),
            TokenKind::Ge => Some(BinOp::Ge),
            _ => None,
        }
    }

    fn peek_isect(&self) -> bool {
        let tok = self.peek();
        tok.leading_ws && is_primary_start(&tok.kind)
    }

    fn peek(&self) -> &Token {
        let i = self.i.min(self.tokens.len().saturating_sub(1));
        &self.tokens[i]
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    fn kind_at(&self, i: usize) -> Option<&TokenKind> {
        self.tokens.get(i).map(|t| &t.kind)
    }

    fn bump(&mut self) {
        if self.i + 1 < self.tokens.len() {
            self.i += 1;
        }
    }

    fn prev_span(&self) -> Span {
        self.tokens[self.i.saturating_sub(1)].span
    }

    fn peek_offset(&self) -> usize {
        self.peek().span.start as usize
    }

    fn prev_is_colon(&self) -> bool {
        self.i > 0 && matches!(self.tokens[self.i - 1].kind, TokenKind::Colon)
    }

    fn expect_bang(&mut self) -> Result<(), ParseError> {
        if matches!(self.peek_kind(), TokenKind::Bang) {
            self.bump();
            Ok(())
        } else {
            Err(self.err("expected '!'", vec!["!".into()]))
        }
    }

    fn expect_lparen(&mut self) -> Result<(), ParseError> {
        if matches!(self.peek_kind(), TokenKind::LParen) {
            self.bump();
            Ok(())
        } else {
            Err(self.err("expected '('", vec!["(".into()]))
        }
    }

    fn expect_rparen(&mut self) -> Result<(), ParseError> {
        if matches!(self.peek_kind(), TokenKind::RParen) {
            self.bump();
            Ok(())
        } else {
            Err(self.err("expected ')'", vec![")".into()]))
        }
    }

    fn err(&self, message: impl Into<String>, expected: Vec<String>) -> ParseError {
        ParseError::parse(message, self.peek_offset(), expected)
    }
}

fn expected_primary() -> Vec<String> {
    ["number", "string", "name", "cell", "(", "{"]
        .into_iter()
        .map(String::from)
        .collect()
}

fn is_primary_start(k: &TokenKind) -> bool {
    matches!(
        k,
        TokenKind::Ident(_)
            | TokenKind::Cell(_)
            | TokenKind::Col { .. }
            | TokenKind::Row { .. }
            | TokenKind::Structured(_)
            | TokenKind::SheetQuoted(_)
            | TokenKind::ExternalBook(_)
            | TokenKind::LParen
            | TokenKind::At
            | TokenKind::Number(_)
    )
}

fn can_call(kind: &ExprKind) -> bool {
    matches!(
        kind,
        ExprKind::Name { .. } | ExprKind::Call { .. } | ExprKind::Paren(_)
    )
}

const BP_CMP: u8 = 10;
const BP_CONCAT: u8 = 20;
const BP_ADD: u8 = 30;
const BP_MUL: u8 = 40;
const BP_UNARY: u8 = 50;
const BP_POW: u8 = 60;
const BP_PCT: u8 = 70;
const BP_UNION: u8 = 80;
const BP_ISECT: u8 = 90;
const BP_RANGE: u8 = 100;
const BP_SPILL: u8 = 110;
const CALL_BP: u8 = 120;

fn prefix_bp(op: PrefixOp) -> u8 {
    match op {
        PrefixOp::Plus | PrefixOp::Minus | PrefixOp::ImplicitIntersect => BP_UNARY,
    }
}

fn postfix_bp(op: PostfixOp) -> (u8, u8) {
    match op {
        PostfixOp::Percent => (BP_PCT, BP_PCT),
        PostfixOp::Spill => (BP_SPILL, BP_SPILL),
    }
}

fn infix_bp(op: BinOp) -> (u8, u8) {
    match op {
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            (BP_CMP, BP_CMP + 1)
        }
        BinOp::Concat => (BP_CONCAT, BP_CONCAT + 1),
        BinOp::Add | BinOp::Sub => (BP_ADD, BP_ADD + 1),
        BinOp::Mul | BinOp::Div => (BP_MUL, BP_MUL + 1),
        BinOp::Pow => (BP_POW, BP_POW + 1),
        BinOp::Union => (BP_UNION, BP_UNION + 1),
        BinOp::Isect => (BP_ISECT, BP_ISECT + 1),
        BinOp::Range => (BP_RANGE, BP_RANGE + 1),
    }
}

fn col_expr(col: u16, abs: bool, span: Span) -> Expr {
    let range = RangeRef {
        start: CellRef {
            sheet: None,
            row: 0,
            col,
            row_abs: false,
            col_abs: abs,
        },
        end: CellRef {
            sheet: None,
            row: MAX_ROWS - 1,
            col,
            row_abs: false,
            col_abs: abs,
        },
        sheet_end: None,
        whole_row: false,
        whole_col: true,
    };
    Expr::new(ExprKind::Range { sheet: None, range }, span)
}

fn row_expr(row: u32, abs: bool, span: Span) -> Expr {
    let range = RangeRef {
        start: CellRef {
            sheet: None,
            row,
            col: 0,
            row_abs: abs,
            col_abs: false,
        },
        end: CellRef {
            sheet: None,
            row,
            col: MAX_COLS - 1,
            row_abs: abs,
            col_abs: false,
        },
        sheet_end: None,
        whole_row: true,
        whole_col: false,
    };
    Expr::new(ExprKind::Range { sheet: None, range }, span)
}

fn ident_as_col(name: &str) -> Option<(u16, bool)> {
    if let Some(rest) = name.strip_prefix('$') {
        return col_from_letters(rest).ok().map(|c| (c, true));
    }
    if name.chars().all(|c| c.is_ascii_alphabetic()) {
        col_from_letters(name).ok().map(|c| (c, false))
    } else {
        None
    }
}

fn number_as_row(n: f64) -> Option<(u32, bool)> {
    if n.fract() != 0.0 || n < 1.0 {
        return None;
    }
    let r = n as u32;
    if r == 0 || r > MAX_ROWS {
        None
    } else {
        Some((r - 1, false))
    }
}

fn fold_range(left: &Expr, right: &Expr) -> Option<Result<ExprKind, ParseError>> {
    fn cell_of(e: &Expr) -> Option<(Option<SheetSpec>, CellRef)> {
        match &e.kind {
            ExprKind::Cell { sheet, cell } => Some((sheet.clone(), *cell)),
            _ => None,
        }
    }
    fn whole_col(e: &Expr) -> Option<(Option<SheetSpec>, u16, bool)> {
        match &e.kind {
            ExprKind::Range { sheet, range }
                if range.whole_col && !range.whole_row && range.start.col == range.end.col =>
            {
                Some((sheet.clone(), range.start.col, range.start.col_abs))
            }
            ExprKind::Name { sheet, name } => {
                ident_as_col(name).map(|(c, a)| (sheet.clone(), c, a))
            }
            _ => None,
        }
    }
    fn whole_row(e: &Expr) -> Option<(Option<SheetSpec>, u32, bool)> {
        match &e.kind {
            ExprKind::Range { sheet, range }
                if range.whole_row && !range.whole_col && range.start.row == range.end.row =>
            {
                Some((sheet.clone(), range.start.row, range.start.row_abs))
            }
            ExprKind::Number(n) => number_as_row(*n).map(|(r, a)| (None, r, a)),
            _ => None,
        }
    }

    if let (Some((s1, c1)), Some((s2, c2))) = (cell_of(left), cell_of(right)) {
        let sheet = merge_sheets(s1, s2);
        let range = RangeRef {
            start: c1,
            end: c2,
            sheet_end: None,
            whole_row: false,
            whole_col: false,
        };
        return Some(Ok(match sheet {
            Some(spec) if spec.end.is_some() => ExprKind::ThreeD {
                sheets: spec,
                inner: Box::new(Expr::new(
                    ExprKind::Range { sheet: None, range },
                    left.span.union(right.span),
                )),
            },
            sheet => ExprKind::Range { sheet, range },
        }));
    }
    if let (Some((s1, c1, a1)), Some((s2, c2, a2))) = (whole_col(left), whole_col(right)) {
        let sheet = merge_sheets(s1, s2);
        let range = RangeRef {
            start: CellRef {
                sheet: None,
                row: 0,
                col: c1,
                row_abs: false,
                col_abs: a1,
            },
            end: CellRef {
                sheet: None,
                row: MAX_ROWS - 1,
                col: c2,
                row_abs: false,
                col_abs: a2,
            },
            sheet_end: None,
            whole_row: false,
            whole_col: true,
        };
        return Some(Ok(ExprKind::Range { sheet, range }));
    }
    if let (Some((s1, r1, a1)), Some((s2, r2, a2))) = (whole_row(left), whole_row(right)) {
        let sheet = merge_sheets(s1, s2);
        let range = RangeRef {
            start: CellRef {
                sheet: None,
                row: r1,
                col: 0,
                row_abs: a1,
                col_abs: false,
            },
            end: CellRef {
                sheet: None,
                row: r2,
                col: MAX_COLS - 1,
                row_abs: a2,
                col_abs: false,
            },
            sheet_end: None,
            whole_row: true,
            whole_col: false,
        };
        return Some(Ok(ExprKind::Range { sheet, range }));
    }
    None
}

fn is_mixed_whole(left: &Expr, right: &Expr) -> bool {
    let l_col = matches!(
        left.kind,
        ExprKind::Range {
            range: RangeRef {
                whole_col: true,
                whole_row: false,
                ..
            },
            ..
        }
    ) || matches!(&left.kind, ExprKind::Name { name, .. } if ident_as_col(name).is_some());
    let r_col = matches!(
        right.kind,
        ExprKind::Range {
            range: RangeRef {
                whole_col: true,
                whole_row: false,
                ..
            },
            ..
        }
    ) || matches!(&right.kind, ExprKind::Name { name, .. } if ident_as_col(name).is_some());
    let l_row = matches!(
        left.kind,
        ExprKind::Range {
            range: RangeRef {
                whole_row: true,
                whole_col: false,
                ..
            },
            ..
        }
    ) || matches!(left.kind, ExprKind::Number(_));
    let r_row = matches!(
        right.kind,
        ExprKind::Range {
            range: RangeRef {
                whole_row: true,
                whole_col: false,
                ..
            },
            ..
        }
    ) || matches!(right.kind, ExprKind::Number(_));
    let l_cell = matches!(left.kind, ExprKind::Cell { .. });
    let r_cell = matches!(right.kind, ExprKind::Cell { .. });
    (l_col && (r_row || r_cell)) || (l_row && (r_col || r_cell)) || (l_cell && (r_col || r_row))
}

fn merge_sheets(a: Option<SheetSpec>, b: Option<SheetSpec>) -> Option<SheetSpec> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(a), Some(b)) if a == b => Some(a),
        (Some(a), Some(b)) => Some(SheetSpec {
            start: a.start,
            end: Some(b.start),
        }),
    }
}

fn attach_sheet(expr: Expr, spec: SheetSpec) -> Result<Expr, ParseError> {
    let span = expr.span;
    if let Some(existing) = expr_sheet(&expr.kind) {
        if existing == spec {
            return Ok(expr);
        }
        return Err(ParseError::parse(
            "duplicate sheet qualifier",
            span.start as usize,
            vec!["cell".into()],
        ));
    }
    let kind = match expr.kind {
        ExprKind::Cell { cell, .. } => {
            if spec.end.is_some() {
                ExprKind::ThreeD {
                    sheets: spec,
                    inner: Box::new(Expr::new(ExprKind::Cell { sheet: None, cell }, span)),
                }
            } else {
                ExprKind::Cell {
                    sheet: Some(spec),
                    cell,
                }
            }
        }
        ExprKind::Range { range, .. } => {
            if spec.end.is_some() {
                ExprKind::ThreeD {
                    sheets: spec,
                    inner: Box::new(Expr::new(ExprKind::Range { sheet: None, range }, span)),
                }
            } else {
                ExprKind::Range {
                    sheet: Some(spec),
                    range,
                }
            }
        }
        ExprKind::Name { name, .. } => {
            if spec.end.is_some() {
                ExprKind::ThreeD {
                    sheets: spec,
                    inner: Box::new(Expr::new(ExprKind::Name { sheet: None, name }, span)),
                }
            } else {
                ExprKind::Name {
                    sheet: Some(spec),
                    name,
                }
            }
        }
        ExprKind::Postfix { expr: inner, op } => {
            let inner = attach_sheet(*inner, spec)?;
            ExprKind::Postfix {
                expr: Box::new(inner),
                op,
            }
        }
        other => {
            if spec.end.is_some() {
                ExprKind::ThreeD {
                    sheets: spec,
                    inner: Box::new(Expr::new(other, span)),
                }
            } else {
                other
            }
        }
    };
    Ok(Expr::new(kind, span))
}

fn expr_sheet(kind: &ExprKind) -> Option<SheetSpec> {
    match kind {
        ExprKind::Cell { sheet, .. }
        | ExprKind::Range { sheet, .. }
        | ExprKind::Name { sheet, .. } => sheet.clone(),
        ExprKind::ThreeD { sheets, .. } => Some(sheets.clone()),
        ExprKind::Postfix { expr, .. } | ExprKind::Prefix { expr, .. } | ExprKind::Paren(expr) => {
            expr_sheet(&expr.kind)
        }
        _ => None,
    }
}

fn parse_sheet_spec(name: &str) -> Result<SheetSpec, ParseError> {
    if name.is_empty() {
        return Err(ParseError::parse(
            "empty sheet name",
            0,
            vec!["sheet".into()],
        ));
    }
    match name.split_once(':') {
        Some((start, end)) if !start.is_empty() && !end.is_empty() && !end.contains(':') => {
            Ok(SheetSpec {
                start: start.to_string(),
                end: Some(end.to_string()),
            })
        }
        Some(_) => Err(ParseError::parse(
            "invalid 3-D sheet span",
            0,
            vec!["sheet".into()],
        )),
        None => Ok(SheetSpec {
            start: name.to_string(),
            end: None,
        }),
    }
}

fn split_quoted_external(name: &str) -> (Option<String>, String) {
    if let Some(rest) = name.strip_prefix('[')
        && let Some(end) = rest.find(']')
    {
        let book = rest[..end].to_string();
        let sheet = rest[end + 1..].to_string();
        return (Some(book), sheet);
    }
    (None, name.to_string())
}

fn is_isect_operand(kind: &ExprKind) -> bool {
    matches!(
        kind,
        ExprKind::Cell { .. }
            | ExprKind::Range { .. }
            | ExprKind::ThreeD { .. }
            | ExprKind::Name { .. }
            | ExprKind::Structured(_)
            | ExprKind::External { .. }
            | ExprKind::Postfix {
                op: PostfixOp::Spill,
                ..
            }
            | ExprKind::Prefix {
                op: PrefixOp::ImplicitIntersect,
                ..
            }
            | ExprKind::Call { .. }
            | ExprKind::Paren(_)
            | ExprKind::Binary {
                op: BinOp::Range | BinOp::Isect | BinOp::Union,
                ..
            }
    )
}

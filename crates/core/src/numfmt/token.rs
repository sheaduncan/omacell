//! Parsed format-code AST.

use crate::locale::LocaleId;

/// Excel custom format codes are at most 255 characters.
pub const MAX_FORMAT_LEN: usize = 255;

/// A parsed format: up to four sections (`pos;neg;zero;text`).
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedFormat {
    pub(crate) sections: Vec<Section>,
}

impl ParsedFormat {
    pub(crate) fn general() -> Self {
        Self {
            sections: vec![Section {
                condition: None,
                color: None,
                locale: None,
                currency: None,
                tokens: vec![Token::General],
            }],
        }
    }

    /// Number of `;`-separated sections.
    #[must_use]
    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    pub(crate) fn text_section(&self) -> Option<&Section> {
        self.sections.get(3)
    }
}

/// One `;`-separated part of a format.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub(crate) condition: Option<Condition>,
    pub(crate) color: Option<ColorHint>,
    pub(crate) locale: Option<LocaleId>,
    pub(crate) currency: Option<String>,
    pub(crate) tokens: Vec<Token>,
}

impl Section {
    pub(crate) fn is_general(&self) -> bool {
        self.tokens.iter().any(|t| matches!(t, Token::General))
            && !self.tokens.iter().any(|t| {
                matches!(
                    t,
                    Token::Digit(_)
                        | Token::Year { .. }
                        | Token::Month { .. }
                        | Token::Day { .. }
                        | Token::Hour { .. }
                )
            })
    }

    pub(crate) fn has_at(&self) -> bool {
        self.tokens
            .iter()
            .any(|t| matches!(t, Token::TextPlaceholder))
    }

    pub(crate) fn is_date(&self) -> bool {
        self.tokens.iter().any(|t| {
            matches!(
                t,
                Token::Year { .. }
                    | Token::Month { .. }
                    | Token::Day { .. }
                    | Token::Weekday { .. }
            )
        })
    }

    pub(crate) fn is_time(&self) -> bool {
        self.tokens.iter().any(|t| {
            matches!(
                t,
                Token::Hour { .. }
                    | Token::Minute { .. }
                    | Token::Second { .. }
                    | Token::SubSecond { .. }
                    | Token::AmPm { .. }
            )
        })
    }

    pub(crate) fn is_fraction(&self) -> bool {
        self.tokens.iter().any(|t| matches!(t, Token::FractionBar))
    }

    pub(crate) fn is_scientific(&self) -> bool {
        self.tokens.iter().any(|t| matches!(t, Token::Exp { .. }))
    }

    pub(crate) fn has_ampm(&self) -> bool {
        self.tokens.iter().any(|t| matches!(t, Token::AmPm { .. }))
    }

    pub(crate) fn subsec_digits(&self) -> u8 {
        self.tokens
            .iter()
            .find_map(|t| match t {
                Token::SubSecond { len } => Some(*len),
                _ => None,
            })
            .unwrap_or(0)
    }
}

/// `[>=1000]`-style section condition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Condition {
    pub(crate) op: CmpOp,
    pub(crate) value: f64,
}

impl Condition {
    pub(crate) fn matches(self, n: f64) -> bool {
        match self.op {
            CmpOp::Eq => n == self.value,
            CmpOp::Ne => n != self.value,
            CmpOp::Gt => n > self.value,
            CmpOp::Lt => n < self.value,
            CmpOp::Ge => n >= self.value,
            CmpOp::Le => n <= self.value,
        }
    }
}

/// Comparison operator inside a condition bracket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    /// `=`
    Eq,
    /// `<>`
    Ne,
    /// `>`
    Gt,
    /// `<`
    Lt,
    /// `>=`
    Ge,
    /// `<=`
    Le,
}

/// Color from `[Red]` or `[Color n]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorHint {
    /// Named Excel format color.
    Named(NamedColor),
    /// Indexed palette color `[Color n]` (`1..=56`).
    Indexed(u8),
}

/// The eight named colors allowed in a format code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NamedColor {
    /// `[Black]`
    Black,
    /// `[White]`
    White,
    /// `[Red]`
    Red,
    /// `[Green]`
    Green,
    /// `[Blue]`
    Blue,
    /// `[Yellow]`
    Yellow,
    /// `[Magenta]`
    Magenta,
    /// `[Cyan]`
    Cyan,
}

/// One atom of a format section.
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    /// `0`, `#`, or `?`.
    Digit(DigitKind),
    /// `,` between digit placeholders (thousands grouping).
    Grouping,
    /// `.` decimal placeholder.
    Decimal,
    /// `%`.
    Percent,
    /// `E+` / `E-`.
    Exp {
        /// Show `+` for non-negative exponents.
        plus: bool,
    },
    /// `/` between fraction placeholders.
    FractionBar,
    /// `y`/`e`/`g` run.
    Year {
        /// Letter count.
        len: u8,
    },
    /// Month after minute disambiguation.
    Month {
        /// `1`/`2` numeric, `3` abbr, `4` full, `5` first letter.
        len: u8,
    },
    /// Minutes.
    Minute {
        /// `1` unpadded, `2` padded.
        len: u8,
        /// `[m]`.
        elapsed: bool,
    },
    /// Day of month.
    Day {
        /// `1` unpadded, `2` padded.
        len: u8,
    },
    /// Weekday `ddd`/`dddd`.
    Weekday {
        /// `3` abbr, `4+` full.
        len: u8,
    },
    /// Hours.
    Hour {
        /// `1` unpadded, `2` padded.
        len: u8,
        /// `[h]`.
        elapsed: bool,
    },
    /// Seconds.
    Second {
        /// `1` unpadded, `2` padded.
        len: u8,
        /// `[s]`.
        elapsed: bool,
    },
    /// `.0` after seconds.
    SubSecond {
        /// Number of `0`s.
        len: u8,
    },
    /// AM/PM.
    AmPm {
        /// Case and width.
        style: AmPmStyle,
    },
    /// `@`.
    TextPlaceholder,
    /// The word `General`.
    General,
    /// Literal text.
    Literal(String),
    /// `*x`.
    Fill(char),
    /// `_x`.
    Skip(char),
}

/// Digit placeholder kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigitKind {
    /// `0`
    Zero,
    /// `#`
    Hash,
    /// `?`
    Question,
}

/// AM/PM token style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmPmStyle {
    /// `AM/PM`
    Upper,
    /// `am/pm`
    Lower,
    /// `A/P`
    UpperShort,
    /// `a/p`
    LowerShort,
}

/// `*` fill and `_` skip markers for the renderer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutHints {
    /// Character to repeat to the cell width (`*x`).
    pub fill: Option<char>,
    /// UTF-8 byte index in `text` where fill should expand.
    pub fill_at: Option<usize>,
    /// Characters whose width was skipped (`_x`).
    pub skips: Vec<char>,
}

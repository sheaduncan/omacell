//! Internable style records (spec F-2.4). A cell holds a [`StyleId`].

use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// Handle to an interned [`Style`] (WP-02 style table).
///
/// ```
/// use omacell_core::style::StyleId;
/// assert_eq!(StyleId::DEFAULT.index(), 0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StyleId(u32);

impl StyleId {
    /// Default style (index 0).
    pub const DEFAULT: Self = Self(0);

    /// Wrap a table index.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Table index.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// Number-format table id. `0` is Excel `General` (WP-06 owns the built-in map).
///
/// ```
/// use omacell_core::style::NumFmtId;
/// assert_eq!(NumFmtId::GENERAL.index(), 0);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NumFmtId(u32);

impl NumFmtId {
    /// Built-in General format (`numFmtId` 0).
    pub const GENERAL: Self = Self(0);

    /// Wrap a `numFmtId`.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Numeric id.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// Cell colour: auto, ARGB, theme+tint, or indexed (OOXML `ST_Color`).
///
/// ```
/// use omacell_core::style::Color;
/// let red = Color::Rgb { argb: 0xFFFF_0000 };
/// assert_ne!(red, Color::Auto);
/// ```
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Color {
    /// Theme / automatic colour.
    #[default]
    Auto,
    /// Explicit ARGB (`0xAARRGGBB`).
    Rgb {
        /// Packed alpha-red-green-blue.
        argb: u32,
    },
    /// Theme colour with OOXML tint in `[-1.0, 1.0]`.
    Theme {
        /// Theme colour index.
        theme: u8,
        /// Tint (compared by `f64::to_bits` for interning).
        tint: f64,
    },
    /// Palette index.
    Indexed {
        /// Palette slot.
        index: u8,
    },
}

impl PartialEq for Color {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Auto, Self::Auto) => true,
            (Self::Rgb { argb: a }, Self::Rgb { argb: b }) => a == b,
            (Self::Theme { theme: t1, tint: a }, Self::Theme { theme: t2, tint: b }) => {
                t1 == t2 && a.to_bits() == b.to_bits()
            }
            (Self::Indexed { index: a }, Self::Indexed { index: b }) => a == b,
            _ => false,
        }
    }
}

impl Eq for Color {}

impl Hash for Color {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::Auto => {}
            Self::Rgb { argb } => argb.hash(state),
            Self::Theme { theme, tint } => {
                theme.hash(state);
                tint.to_bits().hash(state);
            }
            Self::Indexed { index } => index.hash(state),
        }
    }
}

/// Underline style (OOXML `ST_UnderlineValues`).
///
/// ```
/// use omacell_core::style::Underline;
/// let _ = Underline::Single;
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Underline {
    /// No underline.
    #[default]
    None,
    /// Single.
    Single,
    /// Double.
    Double,
    /// Single accounting.
    SingleAccounting,
    /// Double accounting.
    DoubleAccounting,
}

/// Font record shared by styles and rich-text runs.
///
/// ```
/// use omacell_core::style::Font;
/// let f = Font::default();
/// assert!((f.size_pt - 11.0).abs() < f64::EPSILON);
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Font {
    /// Typeface. Empty means the theme / UI default.
    pub name: String,
    /// Size in points.
    pub size_pt: f64,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Underline style.
    pub underline: Underline,
    /// Strikethrough.
    pub strike: bool,
    /// Font colour.
    pub color: Color,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            name: String::new(),
            size_pt: 11.0,
            bold: false,
            italic: false,
            underline: Underline::None,
            strike: false,
            color: Color::Auto,
        }
    }
}

impl PartialEq for Font {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.size_pt.to_bits() == other.size_pt.to_bits()
            && self.bold == other.bold
            && self.italic == other.italic
            && self.underline == other.underline
            && self.strike == other.strike
            && self.color == other.color
    }
}

impl Eq for Font {}

impl Hash for Font {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.size_pt.to_bits().hash(state);
        self.bold.hash(state);
        self.italic.hash(state);
        self.underline.hash(state);
        self.strike.hash(state);
        self.color.hash(state);
    }
}

/// Pattern fill type (OOXML `ST_PatternType`).
///
/// ```
/// use omacell_core::style::PatternType;
/// let _ = PatternType::Solid;
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternType {
    /// No pattern.
    #[default]
    None,
    /// Solid foreground.
    Solid,
    /// Medium gray.
    MediumGray,
    /// Dark gray.
    DarkGray,
    /// Light gray.
    LightGray,
    /// Dark horizontal.
    DarkHorizontal,
    /// Dark vertical.
    DarkVertical,
    /// Dark down-diagonal.
    DarkDown,
    /// Dark up-diagonal.
    DarkUp,
    /// Dark grid.
    DarkGrid,
    /// Dark trellis.
    DarkTrellis,
    /// Light horizontal.
    LightHorizontal,
    /// Light vertical.
    LightVertical,
    /// Light down-diagonal.
    LightDown,
    /// Light up-diagonal.
    LightUp,
    /// Light grid.
    LightGrid,
    /// Light trellis.
    LightTrellis,
    /// 12.5% gray.
    Gray125,
    /// 6.25% gray.
    Gray0625,
}

/// Gradient kind (OOXML `ST_GradientType`).
///
/// ```
/// use omacell_core::style::GradientKind;
/// let _ = GradientKind::Linear;
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradientKind {
    /// Linear gradient.
    #[default]
    Linear,
    /// Path gradient.
    Path,
}

/// One gradient stop.
///
/// ```
/// use omacell_core::style::{Color, GradientStop};
/// let s = GradientStop { position: 0.0, color: Color::Auto };
/// assert_eq!(s.position.to_bits(), 0.0f64.to_bits());
/// ```
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GradientStop {
    /// Position in `[0.0, 1.0]`.
    pub position: f64,
    /// Stop colour.
    pub color: Color,
}

impl PartialEq for GradientStop {
    fn eq(&self, other: &Self) -> bool {
        self.position.to_bits() == other.position.to_bits() && self.color == other.color
    }
}

impl Eq for GradientStop {}

impl Hash for GradientStop {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.position.to_bits().hash(state);
        self.color.hash(state);
    }
}

/// Preserved gradient fill (spec F-2.4).
///
/// ```
/// use omacell_core::style::GradientFill;
/// let g = GradientFill::default();
/// assert!(g.stops.is_empty());
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GradientFill {
    /// Linear or path.
    pub kind: GradientKind,
    /// Linear angle in degrees.
    pub degree: f64,
    /// Path left inset.
    pub left: f64,
    /// Path right inset.
    pub right: f64,
    /// Path top inset.
    pub top: f64,
    /// Path bottom inset.
    pub bottom: f64,
    /// Colour stops (typically two).
    pub stops: SmallVec<[GradientStop; 2]>,
}

impl Default for GradientFill {
    fn default() -> Self {
        Self {
            kind: GradientKind::Linear,
            degree: 0.0,
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
            stops: SmallVec::new(),
        }
    }
}

impl PartialEq for GradientFill {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.degree.to_bits() == other.degree.to_bits()
            && self.left.to_bits() == other.left.to_bits()
            && self.right.to_bits() == other.right.to_bits()
            && self.top.to_bits() == other.top.to_bits()
            && self.bottom.to_bits() == other.bottom.to_bits()
            && self.stops == other.stops
    }
}

impl Eq for GradientFill {}

impl Hash for GradientFill {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
        self.degree.to_bits().hash(state);
        self.left.to_bits().hash(state);
        self.right.to_bits().hash(state);
        self.top.to_bits().hash(state);
        self.bottom.to_bits().hash(state);
        self.stops.hash(state);
    }
}

/// Cell fill: none, solid, pattern, or preserved gradient.
///
/// ```
/// use omacell_core::style::{Color, Fill};
/// let f = Fill::Solid { fg: Color::Rgb { argb: 0xFFFF_FF00 } };
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Fill {
    /// No fill.
    #[default]
    None,
    /// Solid colour.
    Solid {
        /// Foreground / solid colour.
        fg: Color,
    },
    /// Pattern fill.
    Pattern {
        /// Pattern type.
        pattern: PatternType,
        /// Foreground colour.
        fg: Color,
        /// Background colour.
        bg: Color,
    },
    /// Gradient, preserved for `.xlsx` round-trip.
    Gradient(GradientFill),
}

/// Border line style (OOXML `ST_BorderStyle`).
///
/// ```
/// use omacell_core::style::BorderStyle;
/// let _ = BorderStyle::Thin;
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BorderStyle {
    /// No border.
    #[default]
    None,
    /// Thin.
    Thin,
    /// Medium.
    Medium,
    /// Dashed.
    Dashed,
    /// Dotted.
    Dotted,
    /// Thick.
    Thick,
    /// Double.
    Double,
    /// Hair.
    Hair,
    /// Medium dashed.
    MediumDashed,
    /// Dash-dot.
    DashDot,
    /// Medium dash-dot.
    MediumDashDot,
    /// Dash-dot-dot.
    DashDotDot,
    /// Medium dash-dot-dot.
    MediumDashDotDot,
    /// Slanted dash-dot.
    SlantDashDot,
}

/// One side of a border.
///
/// ```
/// use omacell_core::style::{BorderSide, BorderStyle, Color};
/// let s = BorderSide { style: BorderStyle::Thin, color: Color::Auto };
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BorderSide {
    /// Line style.
    pub style: BorderStyle,
    /// Line colour.
    pub color: Color,
}

/// Per-side borders.
///
/// ```
/// use omacell_core::style::Border;
/// let b = Border::default();
/// assert_eq!(b.left.style, omacell_core::style::BorderStyle::None);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Border {
    /// Left.
    pub left: BorderSide,
    /// Right.
    pub right: BorderSide,
    /// Top.
    pub top: BorderSide,
    /// Bottom.
    pub bottom: BorderSide,
}

/// Horizontal alignment (OOXML `ST_HorizontalAlignment`).
///
/// ```
/// use omacell_core::style::HorizontalAlign;
/// let _ = HorizontalAlign::General;
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HorizontalAlign {
    /// General (type-dependent).
    #[default]
    General,
    /// Left.
    Left,
    /// Center.
    Center,
    /// Right.
    Right,
    /// Fill.
    Fill,
    /// Justify.
    Justify,
    /// Center across selection.
    CenterContinuous,
    /// Distributed.
    Distributed,
}

/// Vertical alignment (OOXML `ST_VerticalAlignment`).
///
/// ```
/// use omacell_core::style::VerticalAlign;
/// let _ = VerticalAlign::Bottom;
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerticalAlign {
    /// Top.
    Top,
    /// Center.
    Center,
    /// Bottom (Excel default).
    #[default]
    Bottom,
    /// Justify.
    Justify,
    /// Distributed.
    Distributed,
}

/// Alignment and text layout.
///
/// ```
/// use omacell_core::style::Alignment;
/// let a = Alignment::default();
/// assert!(!a.wrap);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Alignment {
    /// Horizontal alignment.
    pub horizontal: HorizontalAlign,
    /// Vertical alignment.
    pub vertical: VerticalAlign,
    /// Wrap text.
    pub wrap: bool,
    /// Shrink to fit.
    pub shrink: bool,
    /// Indent level (0–255).
    pub indent: u8,
    /// Text rotation: `0..=180`, or `255` for stacked (OOXML `textRotation`).
    pub rotation: u8,
}

/// Protection flags on a cell.
///
/// Excel’s default is locked and not hidden.
///
/// ```
/// use omacell_core::style::Protection;
/// let p = Protection::default();
/// assert!(p.locked);
/// assert!(!p.hidden);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Protection {
    /// Locked when the sheet is protected.
    pub locked: bool,
    /// Hide formula when the sheet is protected.
    pub hidden: bool,
}

impl Default for Protection {
    fn default() -> Self {
        Self {
            locked: true,
            hidden: false,
        }
    }
}

/// Interned style record: font, fill, border, alignment, protection, number format.
///
/// ```
/// use omacell_core::style::Style;
/// let s = Style::default();
/// assert_eq!(s.num_fmt, omacell_core::style::NumFmtId::GENERAL);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Style {
    /// Font.
    pub font: Font,
    /// Fill.
    pub fill: Fill,
    /// Border.
    pub border: Border,
    /// Alignment.
    pub alignment: Alignment,
    /// Protection.
    pub protection: Protection,
    /// Number format id.
    pub num_fmt: NumFmtId,
}

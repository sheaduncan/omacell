//! Page setup and pagination (spec F-11.1).

use serde::{Deserialize, Serialize};

use crate::addr::RangeRef;
use crate::error::CoreError;
use crate::geometry::{DEFAULT_COL_PX, DEFAULT_ROW_PX};
use crate::limits::{MAX_COLS, MAX_ROWS};
use crate::sheet::Sheet;

/// 96 CSS px → 72 PDF points.
pub const PX_TO_PT: f64 = 72.0 / 96.0;

/// Maximum pages produced by one workbook sheet.
pub const MAX_PRINT_PAGES: usize = 10_000;

/// Maximum cell rectangles visited across one sheet's generated pages.
pub const MAX_PRINT_CELL_VISITS: u64 = 10_000_000;

/// Paper size in PDF points (1/72 in).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperSize {
    /// US Letter 8.5×11 in.
    #[default]
    Letter,
    /// US Legal 8.5×14 in.
    Legal,
    /// ISO A4.
    A4,
    /// ISO A3.
    A3,
    /// Tabloid 11×17 in.
    Tabloid,
}

impl PaperSize {
    /// Excel `paperSize` attribute.
    #[must_use]
    pub const fn excel_id(self) -> u32 {
        match self {
            Self::Letter => 1,
            Self::Legal => 5,
            Self::A4 => 9,
            Self::A3 => 8,
            Self::Tabloid => 3,
        }
    }

    /// Parse an Excel paperSize id.
    #[must_use]
    pub const fn from_excel_id(id: u32) -> Self {
        match id {
            5 => Self::Legal,
            8 => Self::A3,
            9 => Self::A4,
            3 => Self::Tabloid,
            _ => Self::Letter,
        }
    }

    /// Width × height in points, portrait.
    #[must_use]
    pub const fn portrait_pt(self) -> (f64, f64) {
        match self {
            Self::Letter => (612.0, 792.0),
            Self::Legal => (612.0, 1008.0),
            Self::A4 => (595.0, 842.0),
            Self::A3 => (842.0, 1191.0),
            Self::Tabloid => (792.0, 1224.0),
        }
    }
}

/// Page orientation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    /// Portrait.
    #[default]
    Portrait,
    /// Landscape.
    Landscape,
}

/// Inches.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Margins {
    /// Left.
    pub left: f64,
    /// Right.
    pub right: f64,
    /// Top.
    pub top: f64,
    /// Bottom.
    pub bottom: f64,
    /// Header inset.
    pub header: f64,
    /// Footer inset.
    pub footer: f64,
}

impl Default for Margins {
    fn default() -> Self {
        Self {
            left: 0.75,
            right: 0.75,
            top: 0.75,
            bottom: 0.75,
            header: 0.3,
            footer: 0.3,
        }
    }
}

impl Margins {
    fn pt(self, inches: f64) -> f64 {
        inches * 72.0
    }

    /// Left margin in points.
    #[must_use]
    pub fn left_pt(self) -> f64 {
        self.pt(self.left)
    }

    /// Right margin in points.
    #[must_use]
    pub fn right_pt(self) -> f64 {
        self.pt(self.right)
    }

    /// Top margin in points.
    #[must_use]
    pub fn top_pt(self) -> f64 {
        self.pt(self.top)
    }

    /// Bottom margin in points.
    #[must_use]
    pub fn bottom_pt(self) -> f64 {
        self.pt(self.bottom)
    }

    /// Header inset in points.
    #[must_use]
    pub fn header_pt(self) -> f64 {
        self.pt(self.header)
    }

    /// Footer inset in points.
    #[must_use]
    pub fn footer_pt(self) -> f64 {
        self.pt(self.footer)
    }
}

/// Inclusive row or column band repeated on every printed page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrintTitleBand<T> {
    /// First zero-based row or column.
    pub start: T,
    /// Last zero-based row or column, inclusive.
    pub end: T,
}

/// Per-sheet page setup.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageSetup {
    /// Paper.
    #[serde(default)]
    pub paper: PaperSize,
    /// Orientation.
    #[serde(default)]
    pub orientation: Orientation,
    /// Margins in inches.
    #[serde(default)]
    pub margins: Margins,
    /// Percent scale (10–400). Ignored when fit-to is set.
    #[serde(default = "hundred")]
    pub scale_percent: u32,
    /// Fit-to-width pages (`None` = use scale).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit_to_width: Option<u32>,
    /// Fit-to-height pages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit_to_height: Option<u32>,
    /// Print area.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub print_area: Option<RangeRef>,
    /// Explicit rows to repeat at the top of every page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_row_band: Option<PrintTitleBand<u32>>,
    /// Explicit columns to repeat at the left of every page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_col_band: Option<PrintTitleBand<u16>>,
    /// Legacy origin-based row count, read for pre-WP-28 OMC compatibility.
    #[serde(default)]
    pub title_rows: u32,
    /// Legacy origin-based column count.
    #[serde(default)]
    pub title_cols: u16,
    /// Header text (Excel `&` codes allowed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    /// Footer text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer: Option<String>,
    /// Manual row breaks (0-based row *after* which a page starts).
    #[serde(default)]
    pub row_breaks: Vec<u32>,
    /// Manual column breaks.
    #[serde(default)]
    pub col_breaks: Vec<u16>,
    /// Print gridlines.
    #[serde(default)]
    pub gridlines: bool,
    /// Print row/column headings.
    #[serde(default)]
    pub headings: bool,
    /// Black and white.
    #[serde(default)]
    pub black_and_white: bool,
}

fn hundred() -> u32 {
    100
}

impl Default for PageSetup {
    fn default() -> Self {
        Self {
            paper: PaperSize::Letter,
            orientation: Orientation::Portrait,
            margins: Margins::default(),
            scale_percent: 100,
            fit_to_width: None,
            fit_to_height: None,
            print_area: None,
            title_row_band: None,
            title_col_band: None,
            title_rows: 0,
            title_cols: 0,
            header: None,
            footer: None,
            row_breaks: Vec::new(),
            col_breaks: Vec::new(),
            gridlines: false,
            headings: false,
            black_and_white: false,
        }
    }
}

impl PageSetup {
    /// Reject non-finite, out-of-grid, and physically impossible settings.
    pub fn validate(&self) -> Result<(), CoreError> {
        let margin_values = [
            self.margins.left,
            self.margins.right,
            self.margins.top,
            self.margins.bottom,
            self.margins.header,
            self.margins.footer,
        ];
        if margin_values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(CoreError::new(
                "print.setup",
                "page margins must be finite, non-negative inches",
            ));
        }
        let (media_w, media_h) = self.media_pt();
        if self.margins.left_pt() + self.margins.right_pt() >= media_w
            || self.margins.top_pt() + self.margins.bottom_pt() >= media_h
        {
            return Err(CoreError::new(
                "print.setup",
                "page margins leave no printable area",
            ));
        }
        if !(10..=400).contains(&self.scale_percent) {
            return Err(CoreError::new(
                "print.setup",
                "print scale must be between 10 and 400 percent",
            ));
        }
        if self.fit_to_width == Some(0) || self.fit_to_height == Some(0) {
            return Err(CoreError::new(
                "print.setup",
                "zero fit-to values must be represented as unlimited (None)",
            ));
        }
        if self.title_rows > MAX_ROWS || self.title_cols > MAX_COLS {
            return Err(CoreError::new(
                "print.setup",
                "print-title counts exceed the worksheet grid",
            ));
        }
        if self
            .title_row_band
            .is_some_and(|band| band.start > band.end || band.end >= MAX_ROWS)
            || self
                .title_col_band
                .is_some_and(|band| band.start > band.end || band.end >= MAX_COLS)
        {
            return Err(CoreError::new(
                "print.setup",
                "print-title bands must be ordered and inside the worksheet grid",
            ));
        }
        if self.row_breaks.iter().any(|row| *row >= MAX_ROWS - 1)
            || self.col_breaks.iter().any(|col| *col >= MAX_COLS - 1)
        {
            return Err(CoreError::new(
                "print.setup",
                "manual page break is outside the worksheet grid",
            ));
        }
        if let Some(area) = self.print_area {
            area.start.validate()?;
            area.end.validate()?;
            if area.is_3d() {
                return Err(CoreError::new(
                    "print.setup",
                    "print area cannot span multiple sheets",
                ));
            }
        }
        Ok(())
    }

    /// Media box width × height in points.
    #[must_use]
    pub fn media_pt(&self) -> (f64, f64) {
        let (w, h) = self.paper.portrait_pt();
        match self.orientation {
            Orientation::Portrait => (w, h),
            Orientation::Landscape => (h, w),
        }
    }

    /// Printable inner size in points (minus margins).
    #[must_use]
    pub fn usable_pt(&self) -> (f64, f64) {
        let (w, h) = self.media_pt();
        (
            (w - self.margins.left_pt() - self.margins.right_pt()).max(1.0),
            (h - self.margins.top_pt() - self.margins.bottom_pt()).max(1.0),
        )
    }

    /// True when every field matches Excel's Letter / 100% defaults.
    #[must_use]
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    /// Effective row-title band, falling back to an old origin-based count.
    #[must_use]
    pub fn row_title_band(&self, legacy_origin: u32) -> Option<PrintTitleBand<u32>> {
        self.title_row_band.or_else(|| {
            (self.title_rows > 0).then(|| PrintTitleBand {
                start: legacy_origin,
                end: legacy_origin
                    .saturating_add(self.title_rows.saturating_sub(1))
                    .min(MAX_ROWS - 1),
            })
        })
    }

    /// Effective column-title band, falling back to an old origin-based count.
    #[must_use]
    pub fn col_title_band(&self, legacy_origin: u16) -> Option<PrintTitleBand<u16>> {
        self.title_col_band.or_else(|| {
            (self.title_cols > 0).then(|| PrintTitleBand {
                start: legacy_origin,
                end: legacy_origin
                    .saturating_add(self.title_cols.saturating_sub(1))
                    .min(MAX_COLS - 1),
            })
        })
    }
}

/// One printed page's cell window.
#[derive(Clone, Debug, PartialEq)]
pub struct PageBox {
    /// First data row (after title rows).
    pub row0: u32,
    /// Last data row inclusive.
    pub row1: u32,
    /// First data column (after title cols).
    pub col0: u16,
    /// Last data column inclusive.
    pub col1: u16,
    /// Applied scale (1.0 = 100%).
    pub scale: f64,
    /// 1-based page number.
    pub page: u32,
    /// Total pages.
    pub pages: u32,
}

/// Paginate `sheet` using `setup`.
pub fn paginate(sheet: &Sheet, setup: &PageSetup) -> Result<Vec<PageBox>, CoreError> {
    setup.validate()?;
    let (area_r0, area_c0, area_r1, area_c1) = print_bounds(sheet, setup);
    let title_row_band = setup.row_title_band(area_r0);
    let title_col_band = setup.col_title_band(area_c0);
    let title_rows = title_row_band.map_or(0, |band| band.end - band.start + 1);
    let title_cols = title_col_band.map_or(0, |band| band.end - band.start + 1);
    let (usable_w, usable_h) = setup.usable_pt();
    let heading_w = if setup.headings { 28.0 } else { 0.0 };
    let heading_h = if setup.headings { 14.0 } else { 0.0 };
    let title_h = title_row_band.map_or(0.0, |band| row_span_pt(sheet, band.start, band.end));
    let title_w = title_col_band.map_or(0.0, |band| col_span_pt(sheet, band.start, band.end));
    let content_w = col_span_pt(sheet, area_c0, area_c1);
    let content_h = row_span_pt(sheet, area_r0, area_r1);
    let scale = compute_scale(
        setup,
        content_w,
        content_h,
        title_w,
        title_h,
        (usable_w - heading_w).max(1.0),
        (usable_h - heading_h).max(1.0),
    );
    let data_w = ((usable_w - heading_w) / scale - title_w).max(1.0);
    let data_h = ((usable_h - heading_h) / scale - title_h).max(1.0);

    let mut row_pages = pack_axis_rows(
        sheet,
        area_r0,
        area_r1,
        data_h,
        &setup.row_breaks,
        title_row_band,
    );
    let mut col_pages = pack_axis_cols(
        sheet,
        area_c0,
        area_c1,
        data_w,
        &setup.col_breaks,
        title_col_band,
    );
    if row_pages.is_empty() {
        row_pages.push((area_r0, area_r1));
    }
    if col_pages.is_empty() {
        col_pages.push((area_c0, area_c1));
    }
    let page_count = row_pages
        .len()
        .checked_mul(col_pages.len())
        .ok_or_else(|| CoreError::new("print.limit", "print page count overflow"))?;
    if page_count > MAX_PRINT_PAGES {
        return Err(CoreError::new(
            "print.limit",
            format!("print job has {page_count} pages; maximum is {MAX_PRINT_PAGES}"),
        )
        .with_hint("set a print area or use fit-to-page scaling"));
    }
    let mut visits = 0u64;
    for (r0, r1) in &row_pages {
        let rows = span_rows_excluding(*r0, *r1, title_row_band) + u64::from(title_rows);
        for (c0, c1) in &col_pages {
            let cols = span_cols_excluding(*c0, *c1, title_col_band) + u64::from(title_cols);
            visits = visits.saturating_add(rows.saturating_mul(cols));
        }
    }
    if visits > MAX_PRINT_CELL_VISITS {
        return Err(CoreError::new(
            "print.limit",
            format!(
                "print job visits {visits} cell rectangles; maximum is {MAX_PRINT_CELL_VISITS}"
            ),
        )
        .with_hint("set a smaller print area or reduce repeated title bands"));
    }
    let pages = u32::try_from(page_count)
        .map_err(|_| CoreError::new("print.limit", "print page count does not fit u32"))?;
    let mut out = Vec::new();
    let mut n = 1u32;
    for (r0, r1) in &row_pages {
        for (c0, c1) in &col_pages {
            out.push(PageBox {
                row0: *r0,
                row1: *r1,
                col0: *c0,
                col1: *c1,
                scale,
                page: n,
                pages,
            });
            n += 1;
        }
    }
    Ok(out)
}

/// Inclusive print-area or used-range bounds `(row0, col0, row1, col1)`.
#[must_use]
pub fn print_bounds(sheet: &Sheet, setup: &PageSetup) -> (u32, u16, u32, u16) {
    if let Some(area) = setup.print_area {
        return (
            area.start.row.min(area.end.row),
            area.start.col.min(area.end.col),
            area.start.row.max(area.end.row),
            area.start.col.max(area.end.col),
        );
    }
    match sheet.used_range() {
        Some(used) => (used.min_row, used.min_col, used.max_row, used.max_col),
        None => (0, 0, 0, 0),
    }
}

fn compute_scale(
    setup: &PageSetup,
    content_w: f64,
    content_h: f64,
    title_w: f64,
    title_h: f64,
    usable_w: f64,
    usable_h: f64,
) -> f64 {
    if setup.fit_to_width.is_some() || setup.fit_to_height.is_some() {
        let sx = setup.fit_to_width.map_or(f64::INFINITY, |pages| {
            let repeated = title_w * f64::from(pages.saturating_sub(1));
            (f64::from(pages) * usable_w) / (content_w + repeated).max(1.0)
        });
        let sy = setup.fit_to_height.map_or(f64::INFINITY, |pages| {
            let repeated = title_h * f64::from(pages.saturating_sub(1));
            (f64::from(pages) * usable_h) / (content_h + repeated).max(1.0)
        });
        let pct = ((sx.min(sy) * 100.0).floor() as u32).clamp(10, 400);
        f64::from(pct) / 100.0
    } else {
        f64::from(setup.scale_percent.clamp(10, 400)) / 100.0
    }
}

fn row_span_pt(sheet: &Sheet, r0: u32, r1: u32) -> f64 {
    if r1 < r0 {
        return 0.0;
    }
    let mut px = 0u64;
    for r in r0..=r1 {
        px += u64::from(sheet.geometry.rows.size(r).unwrap_or(DEFAULT_ROW_PX));
    }
    px as f64 * PX_TO_PT
}

fn col_span_pt(sheet: &Sheet, c0: u16, c1: u16) -> f64 {
    if c1 < c0 {
        return 0.0;
    }
    let mut px = 0u64;
    for c in c0..=c1 {
        px += u64::from(
            sheet
                .geometry
                .cols
                .size(u32::from(c))
                .unwrap_or(DEFAULT_COL_PX),
        );
    }
    px as f64 * PX_TO_PT
}

fn pack_axis_rows(
    sheet: &Sheet,
    start: u32,
    end: u32,
    budget: f64,
    breaks: &[u32],
    exclude: Option<PrintTitleBand<u32>>,
) -> Vec<(u32, u32)> {
    let mut pages = Vec::new();
    let mut r = start;
    while r <= end {
        let mut used = 0.0;
        let r0 = r;
        let mut r1 = r;
        let mut saw_data = false;
        while r <= end {
            if exclude.is_some_and(|band| (band.start..=band.end).contains(&r)) {
                r1 = r;
                r = r.saturating_add(1);
                continue;
            }
            if saw_data && breaks.iter().any(|b| *b + 1 == r) {
                break;
            }
            let h = row_span_pt(sheet, r, r);
            if saw_data && used + h > budget {
                break;
            }
            used += h;
            r1 = r;
            saw_data = true;
            r = r.saturating_add(1);
            if r == 0 {
                break;
            }
        }
        if saw_data {
            pages.push((r0, r1));
        }
        if r <= r0 {
            r = r0.saturating_add(1);
        }
        if r0 == end && r1 == end {
            break;
        }
    }
    pages
}

fn pack_axis_cols(
    sheet: &Sheet,
    start: u16,
    end: u16,
    budget: f64,
    breaks: &[u16],
    exclude: Option<PrintTitleBand<u16>>,
) -> Vec<(u16, u16)> {
    let mut pages = Vec::new();
    let mut c = start;
    while c <= end {
        let mut used = 0.0;
        let c0 = c;
        let mut c1 = c;
        let mut saw_data = false;
        while c <= end {
            if exclude.is_some_and(|band| (band.start..=band.end).contains(&c)) {
                c1 = c;
                c = c.saturating_add(1);
                continue;
            }
            if saw_data && breaks.iter().any(|b| *b + 1 == c) {
                break;
            }
            let w = col_span_pt(sheet, c, c);
            if saw_data && used + w > budget {
                break;
            }
            used += w;
            c1 = c;
            saw_data = true;
            if c == u16::MAX {
                break;
            }
            c = c.saturating_add(1);
        }
        if saw_data {
            pages.push((c0, c1));
        }
        if c <= c0 {
            c = c0.saturating_add(1);
        }
        if c0 == end && c1 == end {
            break;
        }
    }
    pages
}

fn span_rows_excluding(start: u32, end: u32, exclude: Option<PrintTitleBand<u32>>) -> u64 {
    if end < start {
        return 0;
    }
    let total = u64::from(end - start) + 1;
    let excluded = exclude.map_or(0, |band| {
        let first = start.max(band.start);
        let last = end.min(band.end);
        if last < first {
            0
        } else {
            u64::from(last - first) + 1
        }
    });
    total.saturating_sub(excluded)
}

fn span_cols_excluding(start: u16, end: u16, exclude: Option<PrintTitleBand<u16>>) -> u64 {
    if end < start {
        return 0;
    }
    let total = u64::from(end - start) + 1;
    let excluded = exclude.map_or(0, |band| {
        let first = start.max(band.start);
        let last = end.min(band.end);
        if last < first {
            0
        } else {
            u64::from(last - first) + 1
        }
    });
    total.saturating_sub(excluded)
}

/// Expand Excel header/footer codes (`&P`, `&N`, `&A`, `&F`, `&D`).
#[must_use]
pub fn expand_header(template: &str, page: &PageBox, sheet_name: &str, file_name: &str) -> String {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            match chars.next() {
                Some('P') | Some('p') => out.push_str(&page.page.to_string()),
                Some('N') | Some('n') => out.push_str(&page.pages.to_string()),
                Some('A') | Some('a') => out.push_str(sheet_name),
                Some('F') | Some('f') => out.push_str(file_name),
                Some('D') | Some('d') => out.push_str("1970-01-01"),
                Some('&') => out.push('&'),
                Some(other) => {
                    out.push('&');
                    out.push(other);
                }
                None => out.push('&'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

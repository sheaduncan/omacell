//! Parse and emit AutoFilter, data validation, and conditional formatting.

use omacell_core::addr::{RangeRef, RefKind, parse_a1};
use omacell_core::condfmt::{CfDxf, CfKind, CfOp, CfTimePeriod, CondFormat};
use omacell_core::filter::{AutoFilter, FilterColumn, FilterCriteria, NumOp, TextOp};
use omacell_core::style::Color;
use omacell_core::validation::{DataValidation, DvErrorStyle, DvOp, DvType};
use omacell_core::workbook::Workbook;

use super::xml::{XmlEvent, XmlReader, attr, escape};

/// Streaming AutoFilter parser for worksheet events.
#[derive(Default)]
pub(crate) struct AutoFilterParser {
    in_filter: bool,
    range: Option<RangeRef>,
    columns: Vec<FilterColumn>,
    col_id: u16,
    values: Vec<String>,
    customs: Vec<(String, String)>,
    custom_and: bool,
    pending: Option<FilterCriteria>,
}

impl AutoFilterParser {
    pub(crate) fn feed(&mut self, ev: &XmlEvent, dxfs: &[CfDxf]) {
        match ev {
            XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                if name == "autoFilter" =>
            {
                self.range = attr(attrs, "ref").and_then(parse_range);
                self.in_filter = matches!(ev, XmlEvent::Start { .. });
                if matches!(ev, XmlEvent::Empty { .. }) {
                    self.in_filter = false;
                }
            }
            XmlEvent::End { name } if name == "autoFilter" => self.in_filter = false,
            XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs }
                if self.in_filter && name == "filterColumn" =>
            {
                self.flush_column();
                self.col_id = attr(attrs, "colId")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            }
            XmlEvent::Empty { name, attrs } if self.in_filter && name == "filter" => {
                if let Some(v) = attr(attrs, "val") {
                    self.values.push(v.to_string());
                }
            }
            XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs }
                if self.in_filter && name == "customFilters" =>
            {
                self.custom_and = attr(attrs, "and").is_some_and(truthy);
            }
            XmlEvent::Empty { name, attrs } if self.in_filter && name == "customFilter" => {
                let val = attr(attrs, "val").unwrap_or("").to_string();
                let op = attr(attrs, "operator").unwrap_or("equal");
                self.customs.push((op.to_string(), val));
            }
            XmlEvent::Empty { name, attrs } if self.in_filter && name == "top10" => {
                let n = attr(attrs, "val")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(10);
                let percent = attr(attrs, "percent").is_some_and(truthy);
                let bottom = attr(attrs, "top").is_some_and(|s| s == "0" || s == "false");
                self.pending = Some(FilterCriteria::TopN { n, percent, bottom });
            }
            XmlEvent::Empty { name, attrs } if self.in_filter && name == "dynamicFilter" => {
                self.pending = match attr(attrs, "type").unwrap_or("") {
                    "aboveAverage" => Some(FilterCriteria::Average { below: false }),
                    "belowAverage" => Some(FilterCriteria::Average { below: true }),
                    _ => None,
                };
            }
            XmlEvent::Empty { name, attrs } if self.in_filter && name == "colorFilter" => {
                let fill = attr(attrs, "cellColor").is_none_or(truthy);
                let argb = attr(attrs, "dxfId")
                    .and_then(|value| value.parse::<usize>().ok())
                    .and_then(|index| dxfs.get(index))
                    .and_then(|dxf| if fill { dxf.fill } else { dxf.font })
                    .and_then(rgb_argb)
                    .unwrap_or(0);
                self.pending = Some(FilterCriteria::Color { fill, argb });
            }
            XmlEvent::Empty { name, attrs } if self.in_filter && name == "dateGroupItem" => {
                self.pending = Some(FilterCriteria::Period {
                    year: attr(attrs, "year").and_then(|value| value.parse().ok()),
                    month: attr(attrs, "month").and_then(|value| value.parse().ok()),
                });
            }
            XmlEvent::End { name } if self.in_filter && name == "filterColumn" => {
                self.flush_column();
            }
            _ => {}
        }
    }

    pub(crate) fn take(mut self) -> Option<AutoFilter> {
        self.flush_column();
        let range = self.range?;
        Some(AutoFilter {
            range,
            columns: self.columns,
        })
    }

    fn flush_column(&mut self) {
        let criteria = self
            .pending
            .take()
            .or_else(|| custom_criteria(&self.customs, self.custom_and))
            .or_else(|| {
                if self.values.is_empty() {
                    None
                } else {
                    Some(FilterCriteria::Values(std::mem::take(&mut self.values)))
                }
            });
        self.values.clear();
        self.customs.clear();
        self.custom_and = false;
        if let Some(criteria) = criteria {
            self.columns.push(FilterColumn {
                col_id: self.col_id,
                criteria,
            });
        }
    }
}

fn custom_criteria(filters: &[(String, String)], custom_and: bool) -> Option<FilterCriteria> {
    if custom_and && filters.len() >= 2 {
        let mut low = None;
        let mut high = None;
        for (op, value) in filters {
            let Ok(value) = value.parse::<f64>() else {
                continue;
            };
            match op.as_str() {
                "greaterThan" | "greaterThanOrEqual" => low = Some(value),
                "lessThan" | "lessThanOrEqual" => high = Some(value),
                _ => {}
            }
        }
        if let (Some(value), Some(value2)) = (low, high) {
            return Some(FilterCriteria::Number {
                op: NumOp::Between,
                value,
                value2: Some(value2),
            });
        }
    }
    let (op, val) = filters.first()?;
    if let Ok(n) = val.parse::<f64>() {
        let num_op = match op.as_str() {
            "greaterThan" => NumOp::Greater,
            "greaterThanOrEqual" => NumOp::GreaterEq,
            "lessThan" => NumOp::Less,
            "lessThanOrEqual" => NumOp::LessEq,
            "notEqual" => NumOp::NotEqual,
            _ => NumOp::Equal,
        };
        return Some(FilterCriteria::Number {
            op: num_op,
            value: n,
            value2: None,
        });
    }
    let leading = val.starts_with('*');
    let trailing = has_unescaped_trailing_star(val) && val.len() > usize::from(leading);
    if leading || trailing {
        let text_op = match (leading, trailing) {
            (true, true) => TextOp::Contains,
            (true, false) => TextOp::Ends,
            (false, true) => TextOp::Begins,
            (false, false) => TextOp::Contains,
        };
        let start = usize::from(leading);
        let end = val.len().saturating_sub(usize::from(trailing));
        return Some(FilterCriteria::Text {
            op: text_op,
            value: wildcard_unescape(&val[start..end]),
        });
    }
    Some(FilterCriteria::Values(vec![wildcard_unescape(val)]))
}

fn has_unescaped_trailing_star(value: &str) -> bool {
    let Some(prefix) = value.strip_suffix('*') else {
        return false;
    };
    prefix.chars().rev().take_while(|ch| *ch == '~').count() % 2 == 0
}

fn wildcard_unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '~' {
            if let Some(escaped) = chars.next() {
                out.push(escaped);
            } else {
                out.push(ch);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn rgb_argb(color: Color) -> Option<u32> {
    match color {
        Color::Rgb { argb } => Some(argb),
        _ => None,
    }
}

/// Parse data-validation blobs.
pub(crate) fn parse_validations(blobs: &[Vec<u8>]) -> Vec<DataValidation> {
    let mut out = Vec::new();
    for blob in blobs {
        let mut reader = XmlReader::new(blob);
        let mut current: Option<DataValidation> = None;
        let mut in_f1 = false;
        let mut in_f2 = false;
        while let Ok(Some(ev)) = reader.next() {
            match ev {
                XmlEvent::Start { name, attrs } if name == "dataValidation" => {
                    current = Some(parse_dv_attrs(&attrs));
                }
                XmlEvent::Empty { name, attrs } if name == "dataValidation" => {
                    out.push(parse_dv_attrs(&attrs));
                }
                XmlEvent::Start { name, .. } if name == "formula1" => {
                    in_f1 = true;
                    if let Some(dv) = current.as_mut() {
                        dv.formula1 = Some(String::new());
                    }
                }
                XmlEvent::Start { name, .. } if name == "formula2" => {
                    in_f2 = true;
                    if let Some(dv) = current.as_mut() {
                        dv.formula2 = Some(String::new());
                    }
                }
                XmlEvent::Text(t) if in_f1 => {
                    if let Some(dv) = current.as_mut() {
                        dv.formula1.get_or_insert_with(String::new).push_str(&t);
                    }
                }
                XmlEvent::Text(t) if in_f2 => {
                    if let Some(dv) = current.as_mut() {
                        dv.formula2.get_or_insert_with(String::new).push_str(&t);
                    }
                }
                XmlEvent::End { name } if name == "formula1" => in_f1 = false,
                XmlEvent::End { name } if name == "formula2" => in_f2 = false,
                XmlEvent::End { name } if name == "dataValidation" => {
                    if let Some(dv) = current.take() {
                        out.push(dv);
                    }
                }
                _ => {}
            }
        }
    }
    for validation in &mut out {
        if let Some(formula) = &mut validation.formula1 {
            *formula = super::formula::from_xlsx(formula);
        }
        if let Some(formula) = &mut validation.formula2 {
            *formula = super::formula::from_xlsx(formula);
        }
    }
    out
}

/// Parse conditional-formatting blobs.
pub(crate) fn parse_cond_formats(blobs: &[Vec<u8>], dxfs: &[CfDxf]) -> Vec<CondFormat> {
    let mut out = Vec::new();
    for blob in blobs {
        let mut reader = XmlReader::new(blob);
        let mut sqref = zero_range();
        let mut current: Option<CondFormat> = None;
        let mut in_formula = false;
        let mut formulas: Vec<String> = Vec::new();
        while let Ok(Some(ev)) = reader.next() {
            match ev {
                XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs }
                    if name == "conditionalFormatting" =>
                {
                    sqref = attr(&attrs, "sqref")
                        .and_then(|s| s.split_whitespace().next())
                        .and_then(parse_range)
                        .unwrap_or_else(zero_range);
                }
                XmlEvent::Start { name, attrs } if name == "cfRule" => {
                    formulas.clear();
                    current = Some(parse_cf_rule(&attrs, sqref, dxfs));
                }
                XmlEvent::Empty { name, attrs } if name == "cfRule" => {
                    formulas.clear();
                    out.push(parse_cf_rule(&attrs, sqref, dxfs));
                }
                XmlEvent::Start { name, .. } if name == "formula" => {
                    in_formula = true;
                    formulas.push(String::new());
                }
                XmlEvent::Text(t) if in_formula => {
                    if let Some(formula) = formulas.last_mut() {
                        formula.push_str(&t);
                    }
                }
                XmlEvent::End { name } if name == "formula" => in_formula = false,
                XmlEvent::Empty { name, attrs } if name == "color" => {
                    if let Some(rule) = current.as_mut() {
                        let color = parse_cf_color(&attrs);
                        match &mut rule.kind {
                            CfKind::ColorScale { colors } => colors.push(color),
                            CfKind::DataBar {
                                color: bar_color, ..
                            } => *bar_color = color,
                            _ => {}
                        }
                    }
                }
                XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs }
                    if name == "dataBar" =>
                {
                    if let Some(rule) = current.as_mut()
                        && let CfKind::DataBar { gradient, .. } = &mut rule.kind
                    {
                        *gradient = attr(&attrs, "gradient").is_none_or(truthy);
                    }
                }
                XmlEvent::Start { name, attrs } | XmlEvent::Empty { name, attrs }
                    if name == "iconSet" =>
                {
                    if let Some(rule) = current.as_mut()
                        && let CfKind::IconSet { icons } = &mut rule.kind
                    {
                        *icons = attr(&attrs, "iconSet")
                            .and_then(|name| name.as_bytes().first().copied())
                            .and_then(|digit| char::from(digit).to_digit(10))
                            .and_then(|count| u8::try_from(count).ok())
                            .filter(|count| (3..=5).contains(count))
                            .unwrap_or(3);
                    }
                }
                XmlEvent::End { name } if name == "cfRule" => {
                    if let Some(rule) = current.take() {
                        out.push(finish_formulas(rule, &formulas));
                    }
                    formulas.clear();
                }
                _ => {}
            }
        }
    }
    out
}

fn parse_dv_attrs(attrs: &[(String, String)]) -> DataValidation {
    DataValidation {
        range: attr(attrs, "sqref")
            .and_then(|s| s.split_whitespace().next())
            .and_then(parse_range)
            .unwrap_or_else(zero_range),
        kind: dv_type(attr(attrs, "type").unwrap_or("")),
        op: dv_op(attr(attrs, "operator").unwrap_or("between")),
        allow_blank: attr(attrs, "allowBlank").is_none_or(truthy),
        error_style: match attr(attrs, "errorStyle").unwrap_or("stop") {
            "warning" => DvErrorStyle::Warning,
            "information" => DvErrorStyle::Information,
            _ => DvErrorStyle::Stop,
        },
        error_title: attr(attrs, "errorTitle").map(ToOwned::to_owned),
        error_message: attr(attrs, "error").map(ToOwned::to_owned),
        input_title: attr(attrs, "promptTitle").map(ToOwned::to_owned),
        input_message: attr(attrs, "prompt").map(ToOwned::to_owned),
        ..DataValidation::default()
    }
}

fn parse_cf_rule(attrs: &[(String, String)], range: RangeRef, dxfs: &[CfDxf]) -> CondFormat {
    let kind_name = attr(attrs, "type").unwrap_or("cellIs");
    let op = cf_op(attr(attrs, "operator").unwrap_or(""));
    let kind = match kind_name {
        "containsText" => CfKind::ContainsText(attr(attrs, "text").unwrap_or("").to_string()),
        "containsBlanks" => CfKind::Blanks,
        "containsErrors" => CfKind::Errors,
        "duplicateValues" => CfKind::Duplicate,
        "uniqueValues" => CfKind::Unique,
        "top10" => CfKind::TopN {
            n: attr(attrs, "rank")
                .and_then(|s| s.parse().ok())
                .unwrap_or(10),
            percent: attr(attrs, "percent").is_some_and(truthy),
            bottom: attr(attrs, "bottom").is_some_and(truthy),
        },
        "aboveAverage" => CfKind::Average {
            below: attr(attrs, "aboveAverage").is_some_and(|s| s == "0"),
        },
        "timePeriod" => CfKind::TimePeriod(parse_time_period(
            attr(attrs, "timePeriod").unwrap_or("today"),
        )),
        "colorScale" => CfKind::ColorScale { colors: Vec::new() },
        "dataBar" => CfKind::DataBar {
            color: Color::Rgb { argb: 0xFF63_8EC6 },
            gradient: attr(attrs, "gradient").is_none_or(truthy),
        },
        "iconSet" => CfKind::IconSet {
            icons: attr(attrs, "iconSet")
                .and_then(|name| name.as_bytes().first().copied())
                .and_then(|digit| char::from(digit).to_digit(10))
                .and_then(|count| u8::try_from(count).ok())
                .filter(|count| (3..=5).contains(count))
                .unwrap_or(3),
        },
        "expression" => CfKind::Formula(String::new()),
        _ => CfKind::CellIs {
            op,
            formula1: String::new(),
            formula2: None,
        },
    };
    let dxf = attr(attrs, "dxfId")
        .and_then(|s| s.parse::<usize>().ok())
        .and_then(|i| dxfs.get(i).copied())
        .unwrap_or_default();
    CondFormat {
        range,
        priority: attr(attrs, "priority")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1),
        stop_if_true: attr(attrs, "stopIfTrue").is_some_and(truthy),
        kind,
        dxf,
    }
}

fn finish_formulas(mut rule: CondFormat, formulas: &[String]) -> CondFormat {
    match &mut rule.kind {
        CfKind::CellIs {
            formula1, formula2, ..
        } => {
            if let Some(f) = formulas.first() {
                *formula1 = super::formula::from_xlsx(f);
            }
            *formula2 = formulas.get(1).map(|f| super::formula::from_xlsx(f));
        }
        CfKind::Formula(s) => {
            if let Some(f) = formulas.first() {
                *s = super::formula::from_xlsx(f);
            }
        }
        _ => {}
    }
    rule
}

fn parse_rgb(s: Option<&str>) -> Color {
    let Some(s) = s else {
        return Color::Auto;
    };
    let t = s.trim().trim_start_matches('#');
    let padded = if t.len() == 6 {
        format!("FF{t}")
    } else {
        t.to_string()
    };
    let argb = u32::from_str_radix(&padded, 16).unwrap_or(0xFF00_0000);
    Color::Rgb { argb }
}

fn parse_cf_color(attrs: &[(String, String)]) -> Color {
    if let Some(rgb) = attr(attrs, "rgb") {
        return parse_rgb(Some(rgb));
    }
    if let Some(theme) = attr(attrs, "theme").and_then(|value| value.parse().ok()) {
        return Color::Theme {
            theme,
            tint: attr(attrs, "tint")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0.0),
        };
    }
    if let Some(index) = attr(attrs, "indexed").and_then(|value| value.parse().ok()) {
        return Color::Indexed { index };
    }
    Color::Auto
}

fn cf_op(s: &str) -> CfOp {
    match s {
        "lessThan" => CfOp::Less,
        "equal" => CfOp::Equal,
        "between" => CfOp::Between,
        "notBetween" => CfOp::NotBetween,
        "greaterThanOrEqual" => CfOp::GreaterEq,
        "lessThanOrEqual" => CfOp::LessEq,
        "notEqual" => CfOp::NotEqual,
        _ => CfOp::Greater,
    }
}

fn parse_time_period(value: &str) -> CfTimePeriod {
    match value {
        "yesterday" => CfTimePeriod::Yesterday,
        "tomorrow" => CfTimePeriod::Tomorrow,
        "last7Days" => CfTimePeriod::Last7Days,
        "thisWeek" => CfTimePeriod::ThisWeek,
        "lastWeek" => CfTimePeriod::LastWeek,
        "nextWeek" => CfTimePeriod::NextWeek,
        "thisMonth" => CfTimePeriod::ThisMonth,
        "lastMonth" => CfTimePeriod::LastMonth,
        "nextMonth" => CfTimePeriod::NextMonth,
        _ => CfTimePeriod::Today,
    }
}

fn dv_type(s: &str) -> DvType {
    match s {
        "whole" => DvType::Whole,
        "decimal" => DvType::Decimal,
        "list" => DvType::List,
        "date" => DvType::Date,
        "time" => DvType::Time,
        "textLength" => DvType::TextLength,
        "custom" => DvType::Custom,
        _ => DvType::Any,
    }
}

fn dv_op(s: &str) -> DvOp {
    match s {
        "notBetween" => DvOp::NotBetween,
        "equal" => DvOp::Equal,
        "notEqual" => DvOp::NotEqual,
        "greaterThan" => DvOp::Greater,
        "lessThan" => DvOp::Less,
        "greaterThanOrEqual" => DvOp::GreaterEq,
        "lessThanOrEqual" => DvOp::LessEq,
        _ => DvOp::Between,
    }
}

fn parse_range(s: &str) -> Option<RangeRef> {
    let parsed = parse_a1(s).ok()?;
    Some(match parsed.kind {
        RefKind::Range(rg) => rg,
        RefKind::Cell(c) => RangeRef::from_corners(c, c),
    })
}

fn zero_range() -> RangeRef {
    let cell = parse_a1("A1")
        .ok()
        .map(|parsed| match parsed.kind {
            RefKind::Cell(c) => c,
            RefKind::Range(rg) => rg.start,
        })
        .unwrap_or(omacell_core::addr::CellRef {
            sheet: None,
            row: 0,
            col: 0,
            row_abs: false,
            col_abs: false,
        });
    RangeRef::from_corners(cell, cell)
}

fn truthy(s: &str) -> bool {
    matches!(s, "1" | "true" | "True" | "TRUE")
}

pub(crate) fn is_filter_database_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("_xlnm._FilterDatabase")
}

/// Modeled `<autoFilter>` XML.
pub(crate) fn modeled_autofilter(filter: &AutoFilter, dxfs: &[CfDxf]) -> String {
    let mut s = format!(r#"<autoFilter ref="{}">"#, escape(&filter.range.to_a1()));
    for col in &filter.columns {
        s.push_str(&format!(r#"<filterColumn colId="{}">"#, col.col_id));
        match &col.criteria {
            FilterCriteria::Values(vals) => {
                s.push_str("<filters>");
                for v in vals {
                    s.push_str(&format!(r#"<filter val="{}"/>"#, escape(v)));
                }
                s.push_str("</filters>");
            }
            FilterCriteria::Text { op, value } => {
                let value = wildcard_escape(value);
                let pattern = match op {
                    TextOp::Begins => format!("{value}*"),
                    TextOp::Ends => format!("*{value}"),
                    TextOp::Contains => format!("*{value}*"),
                };
                s.push_str(&format!(
                    r#"<customFilters><customFilter operator="equal" val="{}"/></customFilters>"#,
                    escape(&pattern)
                ));
            }
            FilterCriteria::Number { op, value, value2 } => {
                let operator = match op {
                    NumOp::Greater => "greaterThan",
                    NumOp::GreaterEq => "greaterThanOrEqual",
                    NumOp::Less => "lessThan",
                    NumOp::LessEq => "lessThanOrEqual",
                    NumOp::Equal => "equal",
                    NumOp::NotEqual => "notEqual",
                    NumOp::Between => "greaterThanOrEqual",
                };
                s.push_str(&format!(
                    r#"<customFilters{}><customFilter operator="{operator}" val="{value}"/>"#,
                    if *op == NumOp::Between {
                        r#" and="1""#
                    } else {
                        ""
                    }
                ));
                if *op == NumOp::Between
                    && let Some(hi) = value2
                {
                    s.push_str(&format!(
                        r#"<customFilter operator="lessThanOrEqual" val="{hi}"/>"#
                    ));
                }
                s.push_str("</customFilters>");
            }
            FilterCriteria::TopN { n, percent, bottom } => {
                s.push_str(&format!(
                    r#"<top10 val="{n}" percent="{}" top="{}"/>"#,
                    u8::from(*percent),
                    u8::from(!*bottom)
                ));
            }
            FilterCriteria::Average { below } => {
                s.push_str(&format!(
                    r#"<dynamicFilter type="{}"/>"#,
                    if *below {
                        "belowAverage"
                    } else {
                        "aboveAverage"
                    }
                ));
            }
            FilterCriteria::Color { fill, argb } => {
                let dxf = if *fill {
                    CfDxf {
                        fill: Some(Color::Rgb { argb: *argb }),
                        font: None,
                    }
                } else {
                    CfDxf {
                        fill: None,
                        font: Some(Color::Rgb { argb: *argb }),
                    }
                };
                let dxf_id = dxfs
                    .iter()
                    .position(|candidate| candidate == &dxf)
                    .unwrap_or(0);
                s.push_str(&format!(
                    r#"<colorFilter cellColor="{}" dxfId="{dxf_id}"/>"#,
                    u8::from(*fill),
                ));
            }
            FilterCriteria::Period { year, month } => {
                let grouping = if month.is_some() { "month" } else { "year" };
                s.push_str("<filters><dateGroupItem");
                if let Some(year) = year {
                    s.push_str(&format!(r#" year="{year}""#));
                }
                if let Some(month) = month {
                    s.push_str(&format!(r#" month="{month}""#));
                }
                s.push_str(&format!(r#" dateTimeGrouping="{grouping}"/></filters>"#));
            }
        }
        s.push_str("</filterColumn>");
    }
    s.push_str("</autoFilter>");
    s
}

fn wildcard_escape(value: &str) -> String {
    value
        .replace('~', "~~")
        .replace('*', "~*")
        .replace('?', "~?")
}

pub(crate) fn autofilter_extras_match(
    blob: &[u8],
    dxfs: &[CfDxf],
    filter: Option<&AutoFilter>,
) -> bool {
    let mut parser = AutoFilterParser::default();
    let mut reader = XmlReader::new(blob);
    while let Ok(Some(event)) = reader.next() {
        parser.feed(&event, dxfs);
    }
    parser.take().as_ref() == filter
}

pub(crate) fn validation_extras_match(blobs: &[Vec<u8>], rules: &[DataValidation]) -> bool {
    let parsed = parse_validations(blobs);
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    if workbook.set_validations(sheet, parsed).is_err() {
        // Preserve OOXML that the strict model rejected unless the caller has
        // replaced it with a modeled rule. This is the extras-win contract.
        return rules.is_empty();
    }
    workbook
        .sheet(sheet)
        .is_some_and(|sheet| sheet.validations == rules)
}

pub(crate) fn cond_format_extras_match(
    blobs: &[Vec<u8>],
    dxfs: &[CfDxf],
    rules: &[CondFormat],
) -> bool {
    let parsed = parse_cond_formats(blobs, dxfs);
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    if workbook.set_cond_formats(sheet, parsed).is_err() {
        return rules.is_empty();
    }
    workbook
        .sheet(sheet)
        .is_some_and(|sheet| sheet.cond_formats == rules)
}

/// Modeled `<dataValidations>` XML.
pub(crate) fn modeled_validations(rules: &[DataValidation]) -> Option<String> {
    let rules: Vec<_> = rules
        .iter()
        .filter(|rule| rule.kind != DvType::Any)
        .collect();
    if rules.is_empty() {
        return None;
    }
    let mut s = format!(r#"<dataValidations count="{}">"#, rules.len());
    for dv in rules {
        let ty = match dv.kind {
            DvType::Any => continue,
            DvType::Whole => "whole",
            DvType::Decimal => "decimal",
            DvType::List => "list",
            DvType::Date => "date",
            DvType::Time => "time",
            DvType::TextLength => "textLength",
            DvType::Custom => "custom",
        };
        let op = match dv.op {
            DvOp::Between => "between",
            DvOp::NotBetween => "notBetween",
            DvOp::Equal => "equal",
            DvOp::NotEqual => "notEqual",
            DvOp::Greater => "greaterThan",
            DvOp::Less => "lessThan",
            DvOp::GreaterEq => "greaterThanOrEqual",
            DvOp::LessEq => "lessThanOrEqual",
        };
        let style = match dv.error_style {
            DvErrorStyle::Stop => "stop",
            DvErrorStyle::Warning => "warning",
            DvErrorStyle::Information => "information",
        };
        s.push_str(&format!(
            r#"<dataValidation type="{ty}" operator="{op}" allowBlank="{}" errorStyle="{style}" sqref="{}""#,
            u8::from(dv.allow_blank),
            escape(&dv.range.to_a1()),
        ));
        if let Some(t) = &dv.error_title {
            s.push_str(&format!(r#" errorTitle="{}""#, escape(t)));
        }
        if let Some(t) = &dv.error_message {
            s.push_str(&format!(r#" error="{}""#, escape(t)));
        }
        if let Some(t) = &dv.input_title {
            s.push_str(&format!(r#" promptTitle="{}""#, escape(t)));
        }
        if let Some(t) = &dv.input_message {
            s.push_str(&format!(r#" prompt="{}""#, escape(t)));
        }
        s.push('>');
        if let Some(f) = &dv.formula1 {
            let formula = super::formula::to_xlsx(f);
            s.push_str(&format!("<formula1>{}</formula1>", escape(&formula)));
        }
        if let Some(f) = &dv.formula2 {
            let formula = super::formula::to_xlsx(f);
            s.push_str(&format!("<formula2>{}</formula2>", escape(&formula)));
        }
        s.push_str("</dataValidation>");
    }
    s.push_str("</dataValidations>");
    Some(s)
}

/// Modeled `<conditionalFormatting>` fragments. `dxfs` is the styles table.
pub(crate) fn modeled_cond_formats(rules: &[CondFormat], dxfs: &[CfDxf]) -> Vec<String> {
    let mut out = Vec::new();
    for rule in rules {
        let dxf_id = (rule.dxf.fill.is_some() || rule.dxf.font.is_some())
            .then(|| dxfs.iter().position(|d| d == &rule.dxf))
            .flatten()
            .map(|id| format!(r#" dxfId="{id}""#))
            .unwrap_or_default();
        let (ty, op, extra, formulas) = cf_xml_parts(rule);
        let stop = if rule.stop_if_true {
            r#" stopIfTrue="1""#
        } else {
            ""
        };
        let mut s = format!(
            r#"<conditionalFormatting sqref="{}"><cfRule type="{ty}"{dxf_id} priority="{}"{stop}{op}{extra}>"#,
            escape(&rule.range.to_a1()),
            rule.priority,
        );
        for f in formulas {
            let formula = super::formula::to_xlsx(&f);
            s.push_str(&format!("<formula>{}</formula>", escape(&formula)));
        }
        match &rule.kind {
            CfKind::ColorScale { colors } => {
                s.push_str("<colorScale>");
                s.push_str(r#"<cfvo type="min"/>"#);
                if colors.len() >= 3 {
                    s.push_str(r#"<cfvo type="percentile" val="50"/>"#);
                }
                s.push_str(r#"<cfvo type="max"/>"#);
                for color in colors.iter().take(3) {
                    s.push_str(&color_element("color", *color));
                }
                s.push_str("</colorScale>");
            }
            CfKind::DataBar {
                color, gradient, ..
            } => {
                s.push_str(&format!(
                    r#"<dataBar gradient="{}"><cfvo type="min"/><cfvo type="max"/>{}</dataBar>"#,
                    u8::from(*gradient),
                    color_element("color", *color),
                ));
            }
            CfKind::IconSet { icons } => {
                let count = (*icons).clamp(3, 5);
                let name = match count {
                    4 => "4TrafficLights",
                    5 => "5Arrows",
                    _ => "3TrafficLights1",
                };
                s.push_str(&format!(r#"<iconSet iconSet="{name}">"#));
                for index in 0..count {
                    let threshold = u32::from(index) * 100 / u32::from(count);
                    s.push_str(&format!(r#"<cfvo type="percent" val="{threshold}"/>"#));
                }
                s.push_str("</iconSet>");
            }
            _ => {}
        }
        s.push_str("</cfRule></conditionalFormatting>");
        out.push(s);
    }
    out
}

fn cf_xml_parts(rule: &CondFormat) -> (&'static str, String, String, Vec<String>) {
    match &rule.kind {
        CfKind::CellIs {
            op,
            formula1,
            formula2,
        } => {
            let name = match op {
                CfOp::Greater => "greaterThan",
                CfOp::Less => "lessThan",
                CfOp::Equal => "equal",
                CfOp::Between => "between",
                CfOp::NotBetween => "notBetween",
                CfOp::GreaterEq => "greaterThanOrEqual",
                CfOp::LessEq => "lessThanOrEqual",
                CfOp::NotEqual => "notEqual",
            };
            let mut f = vec![formula1.clone()];
            if let Some(s) = formula2 {
                f.push(s.clone());
            }
            ("cellIs", format!(r#" operator="{name}""#), String::new(), f)
        }
        CfKind::ContainsText(t) => {
            let needle = t.replace('"', "\"\"");
            let anchor = rule.range.start.to_a1();
            (
                "containsText",
                r#" operator="containsText""#.into(),
                format!(r#" text="{}""#, escape(t)),
                vec![format!(r#"NOT(ISERROR(SEARCH("{needle}",{anchor})))"#)],
            )
        }
        CfKind::Blanks => ("containsBlanks", String::new(), String::new(), Vec::new()),
        CfKind::Errors => ("containsErrors", String::new(), String::new(), Vec::new()),
        CfKind::Duplicate => ("duplicateValues", String::new(), String::new(), Vec::new()),
        CfKind::Unique => ("uniqueValues", String::new(), String::new(), Vec::new()),
        CfKind::TopN { n, percent, bottom } => (
            "top10",
            String::new(),
            format!(
                r#" rank="{n}" percent="{}" bottom="{}""#,
                u8::from(*percent),
                u8::from(*bottom)
            ),
            Vec::new(),
        ),
        CfKind::Average { below } => (
            "aboveAverage",
            String::new(),
            if *below {
                r#" aboveAverage="0""#.into()
            } else {
                String::new()
            },
            Vec::new(),
        ),
        CfKind::TimePeriod(period) => (
            "timePeriod",
            String::new(),
            format!(r#" timePeriod="{}""#, time_period_name(*period)),
            Vec::new(),
        ),
        CfKind::ColorScale { .. } => ("colorScale", String::new(), String::new(), Vec::new()),
        CfKind::DataBar { .. } => ("dataBar", String::new(), String::new(), Vec::new()),
        CfKind::IconSet { .. } => ("iconSet", String::new(), String::new(), Vec::new()),
        CfKind::Formula(src) => (
            "expression",
            String::new(),
            String::new(),
            vec![src.clone()],
        ),
    }
}

fn time_period_name(period: CfTimePeriod) -> &'static str {
    match period {
        CfTimePeriod::Today => "today",
        CfTimePeriod::Yesterday => "yesterday",
        CfTimePeriod::Tomorrow => "tomorrow",
        CfTimePeriod::Last7Days => "last7Days",
        CfTimePeriod::ThisWeek => "thisWeek",
        CfTimePeriod::LastWeek => "lastWeek",
        CfTimePeriod::NextWeek => "nextWeek",
        CfTimePeriod::ThisMonth => "thisMonth",
        CfTimePeriod::LastMonth => "lastMonth",
        CfTimePeriod::NextMonth => "nextMonth",
    }
}

fn color_element(name: &str, color: Color) -> String {
    match color {
        Color::Rgb { argb } => format!(r#"<{name} rgb="{argb:08X}"/>"#),
        Color::Theme { theme, tint } => {
            format!(r#"<{name} theme="{theme}" tint="{tint}"/>"#)
        }
        Color::Indexed { index } => format!(r#"<{name} indexed="{index}"/>"#),
        Color::Auto => format!(r#"<{name} auto="1"/>"#),
    }
}

/// `<dxfs>` inner XML for modeled CF fills/fonts.
pub(crate) fn dxfs_xml(dxfs: &[CfDxf]) -> String {
    if dxfs.is_empty() {
        return String::new();
    }
    let mut s = format!(r#"<dxfs count="{}">"#, dxfs.len());
    for dxf in dxfs {
        s.push_str("<dxf>");
        if let Some(Color::Rgb { argb }) = dxf.font {
            s.push_str(&format!(r#"<font><color rgb="{argb:08X}"/></font>"#));
        }
        if let Some(Color::Rgb { argb }) = dxf.fill {
            s.push_str(&format!(
                r#"<fill><patternFill patternType="solid"><fgColor rgb="{argb:08X}"/></patternFill></fill>"#
            ));
        }
        s.push_str("</dxf>");
    }
    s.push_str("</dxfs>");
    s
}

//! Parse and emit AutoFilter, data validation, and conditional formatting.

use omacell_core::addr::{RangeRef, RefKind, parse_a1};
use omacell_core::condfmt::{CfDxf, CfKind, CfOp, CondFormat};
use omacell_core::filter::{AutoFilter, FilterColumn, FilterCriteria, NumOp, TextOp};
use omacell_core::style::Color;
use omacell_core::validation::{DataValidation, DvErrorStyle, DvOp, DvType};

use super::xml::{XmlEvent, XmlReader, attr, escape};

/// Streaming AutoFilter parser for worksheet events.
#[derive(Default)]
pub(crate) struct AutoFilterParser {
    in_filter: bool,
    range: Option<RangeRef>,
    columns: Vec<FilterColumn>,
    col_id: u16,
    values: Vec<String>,
    pending: Option<FilterCriteria>,
}

impl AutoFilterParser {
    pub(crate) fn feed(&mut self, ev: &XmlEvent) {
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
            XmlEvent::Empty { name, attrs } if self.in_filter && name == "customFilter" => {
                let val = attr(attrs, "val").unwrap_or("").to_string();
                let op = attr(attrs, "operator").unwrap_or("equal");
                self.pending = Some(custom_criteria(op, &val));
            }
            XmlEvent::Empty { name, attrs } if self.in_filter && name == "top10" => {
                let n = attr(attrs, "val")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(10);
                let percent = attr(attrs, "percent").is_some_and(truthy);
                let bottom = attr(attrs, "top").is_some_and(|s| s == "0" || s == "false");
                self.pending = Some(FilterCriteria::TopN { n, percent, bottom });
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
        let criteria = self.pending.take().or_else(|| {
            if self.values.is_empty() {
                None
            } else {
                Some(FilterCriteria::Values(std::mem::take(&mut self.values)))
            }
        });
        self.values.clear();
        if let Some(criteria) = criteria {
            self.columns.push(FilterColumn {
                col_id: self.col_id,
                criteria,
            });
        }
    }
}

fn custom_criteria(op: &str, val: &str) -> FilterCriteria {
    if let Ok(n) = val.parse::<f64>() {
        let num_op = match op {
            "greaterThan" => NumOp::Greater,
            "greaterThanOrEqual" => NumOp::GreaterEq,
            "lessThan" => NumOp::Less,
            "lessThanOrEqual" => NumOp::LessEq,
            "notEqual" => NumOp::Equal,
            _ => NumOp::Equal,
        };
        return FilterCriteria::Number {
            op: num_op,
            value: n,
            value2: None,
        };
    }
    let text_op = match op {
        "beginsWith" => TextOp::Begins,
        "endsWith" => TextOp::Ends,
        _ => TextOp::Contains,
    };
    FilterCriteria::Text {
        op: text_op,
        value: val.trim_matches('*').to_string(),
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
                XmlEvent::Start { name, .. } if name == "formula1" => in_f1 = true,
                XmlEvent::Start { name, .. } if name == "formula2" => in_f2 = true,
                XmlEvent::Text(t) if in_f1 => {
                    if let Some(dv) = current.as_mut() {
                        dv.formula1 = Some(t);
                    }
                    in_f1 = false;
                }
                XmlEvent::Text(t) if in_f2 => {
                    if let Some(dv) = current.as_mut() {
                        dv.formula2 = Some(t);
                    }
                    in_f2 = false;
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
                XmlEvent::Start { name, .. } if name == "formula" => in_formula = true,
                XmlEvent::Text(t) if in_formula => {
                    formulas.push(t);
                    in_formula = false;
                }
                XmlEvent::End { name } if name == "formula" => in_formula = false,
                XmlEvent::Empty { name, attrs } if name == "color" => {
                    if let Some(rule) = current.as_mut()
                        && let CfKind::ColorScale { colors } = &mut rule.kind
                    {
                        colors.push(parse_rgb(attr(&attrs, "rgb")));
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
        "containsText" => CfKind::ContainsText(String::new()),
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
        "colorScale" => CfKind::ColorScale {
            colors: vec![
                Color::Rgb { argb: 0xFFF8_695E },
                Color::Rgb { argb: 0xFF63_BE7B },
            ],
        },
        "dataBar" => CfKind::DataBar {
            color: Color::Rgb { argb: 0xFF63_8EC6 },
            gradient: true,
        },
        "iconSet" => CfKind::IconSet { icons: 3 },
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
                *formula1 = f.clone();
            }
            *formula2 = formulas.get(1).cloned();
        }
        CfKind::ContainsText(s) | CfKind::Formula(s) => {
            if let Some(f) = formulas.first() {
                *s = f.clone();
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

/// Modeled `<autoFilter>` XML.
pub(crate) fn modeled_autofilter(filter: &AutoFilter) -> String {
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
                let operator = match op {
                    TextOp::Begins => "beginsWith",
                    TextOp::Ends => "endsWith",
                    TextOp::Contains => "contains",
                };
                s.push_str(&format!(
                    r#"<customFilters><customFilter operator="{operator}" val="{}"/></customFilters>"#,
                    escape(value)
                ));
            }
            FilterCriteria::Number { op, value, value2 } => {
                let operator = match op {
                    NumOp::Greater => "greaterThan",
                    NumOp::GreaterEq => "greaterThanOrEqual",
                    NumOp::Less => "lessThan",
                    NumOp::LessEq => "lessThanOrEqual",
                    NumOp::Equal => "equal",
                    NumOp::Between => "greaterThanOrEqual",
                };
                s.push_str(&format!(
                    r#"<customFilters><customFilter operator="{operator}" val="{value}"/>"#
                ));
                if *op == NumOp::Between {
                    if let Some(hi) = value2 {
                        s.push_str(&format!(
                            r#"<customFilter operator="lessThanOrEqual" val="{hi}"/>"#
                        ));
                    }
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
                let _ = below;
                s.push_str(r#"<filters/>"#);
            }
            FilterCriteria::Color { fill, argb } => {
                s.push_str(&format!(
                    r#"<colorFilter cellColor="{}" dxfId="0" rgb="{argb:08X}"/>"#,
                    u8::from(*fill)
                ));
            }
            FilterCriteria::Period { year, month } => {
                s.push_str(&format!(
                    r#"<filters year="{}" month="{}"/>"#,
                    year.unwrap_or(0),
                    month.unwrap_or(0)
                ));
            }
        }
        s.push_str("</filterColumn>");
    }
    s.push_str("</autoFilter>");
    s
}

/// Modeled `<dataValidations>` XML.
pub(crate) fn modeled_validations(rules: &[DataValidation]) -> Option<String> {
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
            s.push_str(&format!("<formula1>{}</formula1>", escape(f)));
        }
        if let Some(f) = &dv.formula2 {
            s.push_str(&format!("<formula2>{}</formula2>", escape(f)));
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
        let dxf_id = dxfs.iter().position(|d| d == &rule.dxf).unwrap_or(0);
        let (ty, op, extra, formulas) = cf_xml_parts(rule);
        let stop = if rule.stop_if_true {
            r#" stopIfTrue="1""#
        } else {
            ""
        };
        let mut s = format!(
            r#"<conditionalFormatting sqref="{}"><cfRule type="{ty}" dxfId="{dxf_id}" priority="{}"{stop}{op}{extra}>"#,
            escape(&rule.range.to_a1()),
            rule.priority,
        );
        for f in formulas {
            s.push_str(&format!("<formula>{}</formula>", escape(&f)));
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
        CfKind::ContainsText(t) => (
            "containsText",
            r#" operator="containsText""#.into(),
            String::new(),
            vec![t.clone()],
        ),
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

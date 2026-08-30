//! Parse and emit worksheet print fragments from [`PageSetup`].

use omacell_core::addr::{RangeRef, parse_a1};
use omacell_core::print::{Orientation, PageSetup, PaperSize};
use omacell_core::sheet::Sheet;

use super::xml::{XmlEvent, XmlReader, attr, escape};

/// Fill `setup` from captured `pageSetup` / `pageMargins` / … blobs.
pub(crate) fn apply_print_xml(setup: &mut PageSetup, blobs: &[Vec<u8>]) {
    for blob in blobs {
        let mut reader = XmlReader::new(blob);
        let mut in_header = false;
        let mut in_footer = false;
        let mut in_row_breaks = false;
        let mut in_col_breaks = false;
        let mut header = String::new();
        let mut footer = String::new();
        while let Ok(Some(ev)) = reader.next() {
            match ev {
                XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                    if name == "pageMargins" =>
                {
                    apply_margins(setup, &attrs);
                }
                XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                    if name == "pageSetup" =>
                {
                    apply_page_setup(setup, &attrs);
                }
                XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                    if name == "printOptions" =>
                {
                    setup.gridlines = attr(&attrs, "gridLines").is_some_and(truthy)
                        || attr(&attrs, "gridlines").is_some_and(truthy);
                    setup.headings = attr(&attrs, "headings").is_some_and(truthy);
                }
                XmlEvent::Start { name, .. } if name == "rowBreaks" => in_row_breaks = true,
                XmlEvent::End { name } if name == "rowBreaks" => in_row_breaks = false,
                XmlEvent::Start { name, .. } if name == "colBreaks" => in_col_breaks = true,
                XmlEvent::End { name } if name == "colBreaks" => in_col_breaks = false,
                XmlEvent::Empty { name, attrs } | XmlEvent::Start { name, attrs }
                    if name == "brk" =>
                {
                    apply_break(setup, &attrs, in_col_breaks && !in_row_breaks);
                }
                XmlEvent::Start { name, .. } if name == "oddHeader" || name == "oddheader" => {
                    in_header = true;
                    header.clear();
                }
                XmlEvent::Start { name, .. } if name == "oddFooter" || name == "oddfooter" => {
                    in_footer = true;
                    footer.clear();
                }
                XmlEvent::Text(t) if in_header => header.push_str(&t),
                XmlEvent::Text(t) if in_footer => footer.push_str(&t),
                XmlEvent::End { name } if name == "oddHeader" || name == "oddheader" => {
                    in_header = false;
                    if !header.is_empty() {
                        setup.header = Some(header.clone());
                    }
                }
                XmlEvent::End { name } if name == "oddFooter" || name == "oddfooter" => {
                    in_footer = false;
                    if !footer.is_empty() {
                        setup.footer = Some(footer.clone());
                    }
                }
                _ => {}
            }
        }
    }
}

fn truthy(s: &str) -> bool {
    matches!(s, "1" | "true" | "True" | "TRUE")
}

fn apply_margins(setup: &mut PageSetup, attrs: &[(String, String)]) {
    if let Some(v) = margin(attrs, "left") {
        setup.margins.left = v;
    }
    if let Some(v) = margin(attrs, "right") {
        setup.margins.right = v;
    }
    if let Some(v) = margin(attrs, "top") {
        setup.margins.top = v;
    }
    if let Some(v) = margin(attrs, "bottom") {
        setup.margins.bottom = v;
    }
    if let Some(v) = margin(attrs, "header") {
        setup.margins.header = v;
    }
    if let Some(v) = margin(attrs, "footer") {
        setup.margins.footer = v;
    }
}

fn margin(attrs: &[(String, String)], name: &str) -> Option<f64> {
    attr(attrs, name)
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
}

fn apply_page_setup(setup: &mut PageSetup, attrs: &[(String, String)]) {
    if let Some(id) = attr(attrs, "paperSize").and_then(|s| s.parse().ok()) {
        setup.paper = PaperSize::from_excel_id(id);
    }
    if attr(attrs, "orientation").is_some_and(|s| s.eq_ignore_ascii_case("landscape")) {
        setup.orientation = Orientation::Landscape;
    }
    if let Some(scale) = attr(attrs, "scale").and_then(|s| s.parse::<u32>().ok()) {
        setup.scale_percent = scale.clamp(10, 400);
    }
    if let Some(w) = attr(attrs, "fitToWidth").and_then(|s| s.parse::<u32>().ok()) {
        setup.fit_to_width = (w > 0).then_some(w);
    }
    if let Some(h) = attr(attrs, "fitToHeight").and_then(|s| s.parse::<u32>().ok()) {
        setup.fit_to_height = (h > 0).then_some(h);
    }
    setup.black_and_white = attr(attrs, "blackAndWhite").is_some_and(truthy);
}

fn apply_break(setup: &mut PageSetup, attrs: &[(String, String)], col: bool) {
    let Some(id) = attr(attrs, "id").and_then(|s| s.parse::<u32>().ok()) else {
        return;
    };
    if id < 2 {
        return;
    }
    // OOXML id is 1-based first row/col of the *new* page.
    let stored = id - 2;
    if col {
        if let Ok(c) = u16::try_from(stored) {
            setup.col_breaks.push(c);
        }
    } else {
        setup.row_breaks.push(stored);
    }
}

/// True when raw print XML still models the current setup (corpus L3 path).
#[must_use]
pub(crate) fn extras_match(print_xml: &[Vec<u8>], setup: &PageSetup) -> bool {
    if print_xml.is_empty() {
        return false;
    }
    let mut parsed = PageSetup {
        print_area: setup.print_area,
        title_rows: setup.title_rows,
        title_cols: setup.title_cols,
        ..PageSetup::default()
    };
    apply_print_xml(&mut parsed, print_xml);
    parsed == *setup
}

/// Modeled print XML, one complete root per blob (spec order).
pub(crate) fn modeled_print_xml(setup: &PageSetup) -> Vec<String> {
    let mut out = Vec::new();
    if setup.gridlines || setup.headings {
        let mut s = String::from("<printOptions");
        if setup.gridlines {
            s.push_str(r#" gridLines="1""#);
        }
        if setup.headings {
            s.push_str(r#" headings="1""#);
        }
        s.push_str("/>");
        out.push(s);
    }
    out.push(format!(
        r#"<pageMargins left="{}" right="{}" top="{}" bottom="{}" header="{}" footer="{}"/>"#,
        setup.margins.left,
        setup.margins.right,
        setup.margins.top,
        setup.margins.bottom,
        setup.margins.header,
        setup.margins.footer
    ));
    let mut ps = format!(
        r#"<pageSetup paperSize="{}" orientation="{}" scale="{}""#,
        setup.paper.excel_id(),
        match setup.orientation {
            Orientation::Portrait => "portrait",
            Orientation::Landscape => "landscape",
        },
        setup.scale_percent.clamp(10, 400)
    );
    if let Some(w) = setup.fit_to_width {
        ps.push_str(&format!(r#" fitToWidth="{w}""#));
    }
    if let Some(h) = setup.fit_to_height {
        ps.push_str(&format!(r#" fitToHeight="{h}""#));
    }
    if setup.black_and_white {
        ps.push_str(r#" blackAndWhite="1""#);
    }
    ps.push_str("/>");
    out.push(ps);
    if setup.header.is_some() || setup.footer.is_some() {
        let mut s = String::from("<headerFooter>");
        if let Some(h) = &setup.header {
            s.push_str(&format!("<oddHeader>{}</oddHeader>", escape(h)));
        }
        if let Some(f) = &setup.footer {
            s.push_str(&format!("<oddFooter>{}</oddFooter>", escape(f)));
        }
        s.push_str("</headerFooter>");
        out.push(s);
    }
    if !setup.row_breaks.is_empty() {
        let mut breaks = setup.row_breaks.clone();
        breaks.sort_unstable();
        breaks.dedup();
        let n = breaks.len();
        let mut s = format!(r#"<rowBreaks count="{n}" manualBreakCount="{n}">"#);
        for b in breaks {
            s.push_str(&format!(r#"<brk id="{}" man="1" max="16383"/>"#, b + 2));
        }
        s.push_str("</rowBreaks>");
        out.push(s);
    }
    if !setup.col_breaks.is_empty() {
        let mut breaks = setup.col_breaks.clone();
        breaks.sort_unstable();
        breaks.dedup();
        let n = breaks.len();
        let mut s = format!(r#"<colBreaks count="{n}" manualBreakCount="{n}">"#);
        for b in breaks {
            s.push_str(&format!(
                r#"<brk id="{}" man="1" max="1048575"/>"#,
                u32::from(b) + 2
            ));
        }
        s.push_str("</colBreaks>");
        out.push(s);
    }
    out
}

/// Parse `_xlnm.Print_Area` / `Print_Titles` formula text.
pub(crate) fn apply_print_name(setup: &mut PageSetup, name: &str, referent: &str) {
    let lower = name.to_ascii_lowercase();
    if is_print_area(&lower) {
        if let Some(range) = parse_print_range(referent) {
            setup.print_area = Some(range);
        }
        return;
    }
    if is_print_titles(&lower) {
        for part in referent.split(',') {
            if let Some(range) = parse_print_range(part.trim()) {
                let rows = range
                    .end
                    .row
                    .saturating_sub(range.start.row)
                    .saturating_add(1);
                let cols = range
                    .end
                    .col
                    .saturating_sub(range.start.col)
                    .saturating_add(1);
                // Whole-row titles look like `$1:$2` (span every column).
                if u32::from(cols) > 256 {
                    setup.title_rows = rows;
                } else {
                    setup.title_cols = cols;
                }
            }
        }
    }
}

/// Whether `name` is one of Excel's built-in print defined names.
#[must_use]
pub(crate) fn is_print_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    is_print_area(&lower) || is_print_titles(&lower)
}

fn is_print_area(lower: &str) -> bool {
    matches!(lower, "_xlnm.print_area" | "print_area")
}

fn is_print_titles(lower: &str) -> bool {
    matches!(lower, "_xlnm.print_titles" | "print_titles")
}

/// True when preserved print defined names still model the current setup.
pub(crate) fn print_names_match<'a>(
    setup: &PageSetup,
    names: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> bool {
    let mut parsed = PageSetup::default();
    let mut saw_area = false;
    let mut saw_titles = false;
    for (name, referent) in names {
        let lower = name.to_ascii_lowercase();
        saw_area |= is_print_area(&lower);
        saw_titles |= is_print_titles(&lower);
        apply_print_name(&mut parsed, name, referent);
    }
    saw_area == setup.print_area.is_some()
        && saw_titles == (setup.title_rows > 0 || setup.title_cols > 0)
        && parsed.print_area == setup.print_area
        && parsed.title_rows == setup.title_rows
        && parsed.title_cols == setup.title_cols
}

fn parse_print_range(text: &str) -> Option<RangeRef> {
    let trimmed = text.trim();
    let a1 = match trimmed.rsplit_once('!') {
        Some((_, rest)) => rest,
        None => trimmed,
    };
    match parse_a1(a1) {
        Ok(parsed) => match parsed.kind {
            omacell_core::addr::RefKind::Range(r) => Some(r),
            omacell_core::addr::RefKind::Cell(c) => Some(RangeRef::from_corners(c, c)),
        },
        Err(_) => None,
    }
}

/// Defined-name XML payloads for print area / titles when extras are empty.
pub(crate) fn print_names_xml(sheet: &Sheet, local_sheet_id: usize) -> String {
    let mut s = String::new();
    if let Some(area) = sheet.page_setup.print_area {
        s.push_str(&format!(
            r#"<definedName name="_xlnm.Print_Area" localSheetId="{local_sheet_id}">{}!{}</definedName>"#,
            escape_name(&sheet.name),
            escape(&area.to_a1())
        ));
    }
    if sheet.page_setup.title_rows > 0 || sheet.page_setup.title_cols > 0 {
        let mut parts = Vec::new();
        if sheet.page_setup.title_rows > 0 {
            parts.push(format!(
                "{}!$1:${}",
                escape_name(&sheet.name),
                sheet.page_setup.title_rows
            ));
        }
        if sheet.page_setup.title_cols > 0 {
            let end =
                omacell_core::addr::col_to_letters(sheet.page_setup.title_cols.saturating_sub(1))
                    .unwrap_or_else(|_| "A".into());
            parts.push(format!("{}!$A:${end}", escape_name(&sheet.name)));
        }
        s.push_str(&format!(
            r#"<definedName name="_xlnm.Print_Titles" localSheetId="{local_sheet_id}">{}</definedName>"#,
            escape(&parts.join(","))
        ));
    }
    s
}

fn escape_name(name: &str) -> String {
    if name.chars().any(|c| !c.is_ascii_alphanumeric()) {
        format!("'{}'", name.replace('\'', "''"))
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_numeric_attributes_do_not_poison_page_setup() {
        let mut setup = PageSetup::default();
        apply_print_xml(
            &mut setup,
            &[
                br#"<pageMargins left="NaN" right="-1" top="inf"/>"#.to_vec(),
                br#"<pageSetup scale="0" fitToWidth="0" fitToHeight="2"/>"#.to_vec(),
            ],
        );
        assert_eq!(setup.margins, omacell_core::print::Margins::default());
        assert_eq!(setup.scale_percent, 10);
        assert_eq!(setup.fit_to_width, None);
        assert_eq!(setup.fit_to_height, Some(2));
        setup.validate().unwrap();
    }

    #[test]
    fn raw_print_xml_only_wins_while_it_matches() {
        let raw =
            vec![br#"<pageSetup paperSize="9" orientation="landscape" scale="100"/>"#.to_vec()];
        let setup = PageSetup {
            paper: PaperSize::A4,
            orientation: Orientation::Landscape,
            ..PageSetup::default()
        };
        assert!(extras_match(&raw, &setup));
        assert!(!extras_match(
            &raw,
            &PageSetup {
                paper: PaperSize::Legal,
                ..setup
            }
        ));
    }

    #[test]
    fn similarly_suffixed_user_name_is_not_a_builtin_print_name() {
        let mut setup = PageSetup::default();
        apply_print_name(&mut setup, "Quarterly_Print_Area", "$A$1:$B$2");
        assert_eq!(setup.print_area, None);
        assert!(!is_print_name("Quarterly_Print_Area"));
        assert!(is_print_name("_xlnm.Print_Area"));
    }
}

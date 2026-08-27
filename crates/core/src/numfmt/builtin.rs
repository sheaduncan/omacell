//! Excel built-in `numFmtId` 0–49 (ECMA-376 18.8.30 + locale currency/date).

use std::borrow::Cow;

use crate::locale::{DateOrder, LocaleId};

/// Format code implied by a built-in `numFmtId`. `None` outside `0..=49`.
#[must_use]
pub fn builtin_format(id: u32, locale: LocaleId) -> Option<Cow<'static, str>> {
    if id > 49 {
        return None;
    }
    let info = locale.info();
    let currency = info.currency;
    match id {
        0 => Some(Cow::Borrowed("General")),
        1 => Some(Cow::Borrowed("0")),
        2 => Some(Cow::Borrowed("0.00")),
        3 => Some(Cow::Borrowed("#,##0")),
        4 => Some(Cow::Borrowed("#,##0.00")),
        5 => Some(currency_fmt("$#,##0_);($#,##0)", currency)),
        6 => Some(currency_fmt("$#,##0_);[Red]($#,##0)", currency)),
        7 => Some(currency_fmt("$#,##0.00_);($#,##0.00)", currency)),
        8 => Some(currency_fmt("$#,##0.00_);[Red]($#,##0.00)", currency)),
        9 => Some(Cow::Borrowed("0%")),
        10 => Some(Cow::Borrowed("0.00%")),
        11 => Some(Cow::Borrowed("0.00E+00")),
        12 => Some(Cow::Borrowed("# ?/?")),
        13 => Some(Cow::Borrowed("# ??/??")),
        14 => Some(Cow::Owned(short_date(info.date_order, locale))),
        15 => Some(Cow::Borrowed("d-mmm-yy")),
        16 => Some(Cow::Borrowed("d-mmm")),
        17 => Some(Cow::Borrowed("mmm-yy")),
        18 => Some(Cow::Borrowed("h:mm AM/PM")),
        19 => Some(Cow::Borrowed("h:mm:ss AM/PM")),
        20 => Some(Cow::Borrowed("h:mm")),
        21 => Some(Cow::Borrowed("h:mm:ss")),
        22 => Some(Cow::Owned(format!("{} h:mm", short_date(info.date_order, locale)))),
        23..=26 => Some(Cow::Borrowed("General")),
        27 => Some(Cow::Borrowed("[$-411]ge.m.d")),
        28 => Some(Cow::Borrowed("[$-411]ggge\"年\"m\"月\"d\"日\"")),
        29 => Some(Cow::Borrowed("[$-411]ggge\"年\"m\"月\"d\"日\"")),
        30 => Some(Cow::Borrowed("m/d/yy")),
        31 => Some(Cow::Borrowed("yyyy\"年\"m\"月\"d\"日\"")),
        32 => Some(Cow::Borrowed("h\"時\"mm\"分\"")),
        33 => Some(Cow::Borrowed("h\"時\"mm\"分\"ss\"秒\"")),
        34 => Some(Cow::Borrowed("yyyy\"年\"m\"月\"")),
        35 => Some(Cow::Borrowed("m\"月\"d\"日\"")),
        36 => Some(Cow::Borrowed("[$-411]ge.m.d")),
        37 => Some(Cow::Borrowed("#,##0 ;(#,##0)")),
        38 => Some(Cow::Borrowed("#,##0 ;[Red](#,##0)")),
        39 => Some(Cow::Borrowed("#,##0.00;(#,##0.00)")),
        40 => Some(Cow::Borrowed("#,##0.00;[Red](#,##0.00)")),
        41 => Some(currency_fmt(
            "_($* #,##0_);_($* (#,##0);_($* \"-\"_);_(@_)",
            currency,
        )),
        42 => Some(Cow::Borrowed("_(* #,##0_);_(* (#,##0);_(* \"-\"_);_(@_)")),
        43 => Some(currency_fmt(
            "_($* #,##0.00_);_($* (#,##0.00);_($* \"-\"??_);_(@_)",
            currency,
        )),
        44 => Some(Cow::Borrowed(
            "_(* #,##0.00_);_(* (#,##0.00);_(* \"-\"??_);_(@_)",
        )),
        45 => Some(Cow::Borrowed("mm:ss")),
        46 => Some(Cow::Borrowed("[h]:mm:ss")),
        47 => Some(Cow::Borrowed("mmss.0")),
        48 => Some(Cow::Borrowed("##0.0E+0")),
        49 => Some(Cow::Borrowed("@")),
        _ => None,
    }
}

fn currency_fmt(template: &'static str, currency: &str) -> Cow<'static, str> {
    if currency == "$" {
        Cow::Borrowed(template)
    } else {
        Cow::Owned(template.replace('$', currency))
    }
}

fn short_date(order: DateOrder, locale: LocaleId) -> String {
    let sep = if locale == LocaleId::DE_DE { '.' } else { '/' };
    match order {
        DateOrder::Mdy => format!("m{sep}d{sep}yy"),
        DateOrder::Dmy => format!("dd{sep}mm{sep}yy"),
        DateOrder::Ymd => "yyyy/m/d".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_0_49_en_us() {
        for id in 0..=49 {
            assert!(builtin_format(id, LocaleId::EN_US).is_some(), "id {id}");
        }
        assert_eq!(builtin_format(14, LocaleId::EN_US).as_deref(), Some("m/d/yy"));
        assert_eq!(builtin_format(14, LocaleId::EN_GB).as_deref(), Some("dd/mm/yy"));
        assert_eq!(builtin_format(14, LocaleId::DE_DE).as_deref(), Some("dd.mm.yy"));
        assert_eq!(
            builtin_format(5, LocaleId::DE_DE).as_deref(),
            Some("€#,##0_);(€#,##0)")
        );
    }
}

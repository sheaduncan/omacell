//! Workbook locale identity and LCID tables (WP-06).
//!
//! [`LocaleId`] and [`LocaleSeparators`] public fields and the `EN_US`
//! constants are frozen (WP-01). This module adds lookup tables and methods
//! only.

mod tables;

use serde::{Deserialize, Serialize};

/// Excel locale identifier (LCID). `0x0409` is `en-US`.
///
/// ```
/// use omacell_core::locale::LocaleId;
/// assert_eq!(LocaleId::EN_US.lcid(), 0x0409);
/// assert_eq!(LocaleId::EN_US.bcp47(), Some("en-US"));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LocaleId(u32);

impl LocaleId {
    /// English (United States), Excel LCID `0x0409`. Canonical formula locale (WP-03).
    pub const EN_US: Self = Self(0x0409);
    /// English (United Kingdom), `0x0809`.
    pub const EN_GB: Self = Self(0x0809);
    /// German (Germany), `0x0407`.
    pub const DE_DE: Self = Self(0x0407);
    /// French (France), `0x040C`.
    pub const FR_FR: Self = Self(0x040C);
    /// Spanish (Spain), `0x040A`.
    pub const ES_ES: Self = Self(0x040A);
    /// Italian (Italy), `0x0410`.
    pub const IT_IT: Self = Self(0x0410);
    /// Dutch (Netherlands), `0x0413`.
    pub const NL_NL: Self = Self(0x0413);
    /// Portuguese (Brazil), `0x0416`.
    pub const PT_BR: Self = Self(0x0416);
    /// Swedish (Sweden), `0x041D`.
    pub const SV_SE: Self = Self(0x041D);
    /// Polish (Poland), `0x0415`.
    pub const PL_PL: Self = Self(0x0415);
    /// Russian (Russia), `0x0419`.
    pub const RU_RU: Self = Self(0x0419);
    /// Japanese (Japan), `0x0411`.
    pub const JA_JP: Self = Self(0x0411);
    /// Chinese (Simplified, PRC), `0x0804`.
    pub const ZH_CN: Self = Self(0x0804);
    /// Korean (Korea), `0x0412`.
    pub const KO_KR: Self = Self(0x0412);

    /// Wrap a raw Excel LCID.
    #[must_use]
    pub const fn new(lcid: u32) -> Self {
        Self(lcid)
    }

    /// Excel LCID value.
    #[must_use]
    pub const fn lcid(self) -> u32 {
        self.0
    }

    /// Parse a BCP-47 tag (`en-US`) or a hex LCID (`0x0409`).
    #[must_use]
    pub fn parse_tag(tag: &str) -> Option<Self> {
        let t = tag.trim();
        if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
            return u32::from_str_radix(hex, 16).ok().map(Self::new);
        }
        tables::lookup_bcp47(t).map(|row| Self(row.lcid))
    }

    /// BCP-47 tag when this LCID is in the table.
    #[must_use]
    pub fn bcp47(self) -> Option<&'static str> {
        tables::lookup(self.0).map(|row| row.bcp47)
    }

    /// Decimal / thousands / list separators. Unknown LCIDs return `en-US`.
    #[must_use]
    pub fn separators(self) -> LocaleSeparators {
        tables::lookup(self.0)
            .map(|row| row.separators)
            .unwrap_or(LocaleSeparators::EN_US)
    }

    /// Full locale record. Unknown LCIDs fall back to `en-US`.
    #[must_use]
    pub fn info(self) -> &'static LocaleInfo {
        tables::lookup(self.0).unwrap_or(&tables::TABLE[0])
    }
}

impl Default for LocaleId {
    fn default() -> Self {
        Self::EN_US
    }
}

/// Character separators used by number formats and localized formula entry.
///
/// ```
/// use omacell_core::locale::LocaleSeparators;
/// assert_eq!(LocaleSeparators::EN_US.decimal, '.');
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LocaleSeparators {
    /// Decimal separator (`'.'` in `en-US`).
    pub decimal: char,
    /// Thousands separator (`','` in `en-US`).
    pub thousands: char,
    /// Function-argument / list separator (`','` in `en-US`, `';'` in many EU locales).
    pub list: char,
}

impl LocaleSeparators {
    /// `en-US` separators.
    pub const EN_US: Self = Self {
        decimal: '.',
        thousands: ',',
        list: ',',
    };
}

impl Default for LocaleSeparators {
    fn default() -> Self {
        Self::EN_US
    }
}

/// Calendar component order of the locale short date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DateOrder {
    /// Month / day / year (`en-US`).
    Mdy,
    /// Day / month / year (`en-GB`, `de-DE`).
    Dmy,
    /// Year / month / day (`ja-JP`, `sv-SE`).
    Ymd,
}

/// Names and separators for one LCID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocaleInfo {
    /// Excel LCID.
    pub lcid: u32,
    /// BCP-47 tag.
    pub bcp47: &'static str,
    /// Number / list separators.
    pub separators: LocaleSeparators,
    /// Short-date component order (builtin `numFmtId` 14).
    pub date_order: DateOrder,
    /// Ante meridiem string.
    pub am: &'static str,
    /// Post meridiem string.
    pub pm: &'static str,
    /// January … December (full).
    pub months_full: [&'static str; 12],
    /// Jan … Dec (Excel German March is `Mrz`).
    pub months_abbr: [&'static str; 12],
    /// Sunday … Saturday (full).
    pub days_full: [&'static str; 7],
    /// Sun … Sat.
    pub days_abbr: [&'static str; 7],
    /// Currency symbol used by builtin formats 5–8 / 41–44.
    pub currency: &'static str,
}

impl LocaleInfo {
    /// Month name. `len` 3 = abbr, 4+ = full, 5 = first grapheme.
    #[must_use]
    pub fn month_name(self, month: u8, len: u8) -> &'static str {
        let idx = month.saturating_sub(1) as usize;
        if idx >= 12 {
            return "";
        }
        match len {
            0..=2 => "",
            3 => self.months_abbr[idx],
            5 => first_char(self.months_full[idx]),
            _ => self.months_full[idx],
        }
    }

    /// Weekday name. `sun0` is Sunday = 0.
    #[must_use]
    pub fn weekday_name(self, sun0: u8, len: u8) -> &'static str {
        let idx = (sun0 % 7) as usize;
        if len <= 3 {
            self.days_abbr[idx]
        } else {
            self.days_full[idx]
        }
    }
}

fn first_char(s: &'static str) -> &'static str {
    match s.chars().next() {
        Some(c) => &s[..c.len_utf8()],
        None => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn en_us_frozen() {
        assert_eq!(LocaleId::EN_US.lcid(), 0x0409);
        assert_eq!(LocaleId::EN_US.bcp47(), Some("en-US"));
        assert_eq!(LocaleSeparators::EN_US.decimal, '.');
        assert_eq!(LocaleSeparators::EN_US.thousands, ',');
        assert_eq!(LocaleSeparators::EN_US.list, ',');
        assert_eq!(LocaleId::EN_US.separators(), LocaleSeparators::EN_US);
    }

    #[test]
    fn de_de_separators() {
        let s = LocaleId::DE_DE.separators();
        assert_eq!(s.decimal, ',');
        assert_eq!(s.thousands, '.');
        assert_eq!(s.list, ';');
        assert_eq!(LocaleId::DE_DE.info().months_abbr[2], "Mrz");
    }

    #[test]
    fn unknown_lcid_falls_back_to_en_us() {
        let id = LocaleId::new(0xFFFF);
        assert_eq!(id.bcp47(), None);
        assert_eq!(id.separators(), LocaleSeparators::EN_US);
        assert_eq!(id.info().bcp47, "en-US");
    }

    #[test]
    fn parse_tag() {
        assert_eq!(LocaleId::parse_tag("de-DE"), Some(LocaleId::DE_DE));
        assert_eq!(LocaleId::parse_tag("0x0407"), Some(LocaleId::DE_DE));
        assert_eq!(LocaleId::parse_tag("nope"), None);
    }
}

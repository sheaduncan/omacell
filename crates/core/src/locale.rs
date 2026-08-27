//! Workbook locale identity. Separator tables beyond `en-US` are WP-06.

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

    /// BCP-47 tag when known to this crate. Other LCIDs are WP-06.
    #[must_use]
    pub fn bcp47(self) -> Option<&'static str> {
        match self.0 {
            0x0409 => Some("en-US"),
            _ => None,
        }
    }

    /// Decimal / thousands / list separators.
    ///
    /// Unknown LCIDs return the `en-US` separators until WP-06 lands locale tables.
    #[must_use]
    pub fn separators(self) -> LocaleSeparators {
        let _ = self;
        LocaleSeparators::EN_US
    }
}

impl Default for LocaleId {
    fn default() -> Self {
        Self::EN_US
    }
}

/// Character separators used by number formats and localized formula entry.
///
/// The formula parser (WP-03) works on the canonical form (`.` decimal, `,`
/// list). This type is the editor-boundary conversion table.
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

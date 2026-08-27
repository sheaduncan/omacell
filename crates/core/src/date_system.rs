//! Workbook date-serial system shared by storage and number formatting.

use serde::{Deserialize, Serialize};

/// Excel's 1900 or 1904 date-serial system (F-1.6, F-2.1).
///
/// ```
/// use omacell_core::date_system::DateSystem;
/// assert_eq!(DateSystem::default(), DateSystem::Excel1900);
/// assert_eq!(DateSystem::Excel1904.epoch_year(), 1904);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateSystem {
    /// Windows Excel default, including the Lotus 1900 leap-year quirk.
    #[default]
    Excel1900,
    /// Historical Mac Excel system whose serial zero is 1 January 1904.
    Excel1904,
}

impl DateSystem {
    /// Calendar year containing serial zero/one for this system.
    #[must_use]
    pub const fn epoch_year(self) -> i32 {
        match self {
            Self::Excel1900 => 1900,
            Self::Excel1904 => 1904,
        }
    }
}

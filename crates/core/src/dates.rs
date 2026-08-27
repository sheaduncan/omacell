//! Excel date serials (spec F-2.1).
//!
//! The 1900 system reproduces the Lotus 1-2-3 leap-year quirk: serial 60 is
//! 29 Feb 1900, a day that does not exist in the Gregorian calendar. Serial 0
//! is Excel’s “January 0, 1900”. The 1904 system has neither quirk.
//!
//! Civil conversion is implemented here (Howard Hinnant algorithms) so the
//! Lotus day can exist; `chrono` is not used.

use std::fmt;

/// Which epoch a workbook uses (F-1.6).
///
/// ```
/// use omacell_core::dates::DateSystem;
/// assert_eq!(DateSystem::Excel1900.epoch_year(), 1900);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DateSystem {
    /// Windows Excel default. Serial 1 = 1 Jan 1900; serial 60 = 29 Feb 1900.
    #[default]
    Excel1900,
    /// Mac Excel historically. Serial 0 = 1 Jan 1904.
    Excel1904,
}

impl DateSystem {
    /// Calendar year of the epoch date.
    #[must_use]
    pub const fn epoch_year(self) -> i32 {
        match self {
            Self::Excel1900 => 1900,
            Self::Excel1904 => 1904,
        }
    }
}

/// Largest integer serial that is a valid Excel date in the 1900 system
/// (31 Dec 9999).
pub const MAX_SERIAL_1900: i64 = 2_958_465;

/// Largest integer serial that is a valid Excel date in the 1904 system
/// (31 Dec 9999). Equal to [`MAX_SERIAL_1900`] minus 1462.
pub const MAX_SERIAL_1904: i64 = 2_957_003;

/// 1900-system serial of 1 Jan 1904 (1460 real days + the Lotus day + origin).
pub const SERIAL_1904_JAN1_IN_1900: i64 = 1_462;

/// Days from 1970-01-01 to 1900-01-01.
const UNIX_DAYS_1900_JAN1: i64 = -25_567;
/// Days from 1970-01-01 to 1904-01-01.
const UNIX_DAYS_1904_JAN1: i64 = -24_107;

/// A civil date in the Excel calendar.
///
/// `day` may be 0 for the 1900-system serial 0 (January 0, 1900).
/// `lotus_leap` is set only for 29 Feb 1900.
///
/// ```
/// use omacell_core::dates::{serial_to_date, DateSystem};
/// let d = serial_to_date(60, DateSystem::Excel1900).expect("lotus");
/// assert!(d.lotus_leap);
/// assert_eq!(d.month, 2);
/// assert_eq!(d.day, 29);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CivilDate {
    /// Year in `0..=9999` (Excel).
    pub year: i32,
    /// Month `1..=12`.
    pub month: u8,
    /// Day of month. `0` only for Excel serial 0 in the 1900 system.
    pub day: u8,
    /// True when this is the fake 29 Feb 1900.
    pub lotus_leap: bool,
}

impl fmt::Display for CivilDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// Convert an integer Excel serial to a civil date.
#[must_use]
pub fn serial_to_date(serial: i64, system: DateSystem) -> Option<CivilDate> {
    match system {
        DateSystem::Excel1900 => serial_to_date_1900(serial),
        DateSystem::Excel1904 => serial_to_date_1904(serial),
    }
}

fn serial_to_date_1900(serial: i64) -> Option<CivilDate> {
    if serial < 0 || serial > MAX_SERIAL_1900 {
        return None;
    }
    if serial == 0 {
        return Some(CivilDate {
            year: 1900,
            month: 1,
            day: 0,
            lotus_leap: false,
        });
    }
    if serial == 60 {
        return Some(CivilDate {
            year: 1900,
            month: 2,
            day: 29,
            lotus_leap: true,
        });
    }
    let unix_days = if serial < 60 {
        UNIX_DAYS_1900_JAN1 + (serial - 1)
    } else {
        UNIX_DAYS_1900_JAN1 + (serial - 2)
    };
    let (year, month, day) = civil_from_days(unix_days);
    Some(CivilDate {
        year,
        month,
        day,
        lotus_leap: false,
    })
}

fn serial_to_date_1904(serial: i64) -> Option<CivilDate> {
    if serial > MAX_SERIAL_1904 {
        return None;
    }
    let unix_days = UNIX_DAYS_1904_JAN1.checked_add(serial)?;
    let (year, month, day) = civil_from_days(unix_days);
    if !(0..=9999).contains(&year) {
        return None;
    }
    Some(CivilDate {
        year,
        month,
        day,
        lotus_leap: false,
    })
}

/// Convert a civil date to an Excel serial.
#[must_use]
pub fn date_to_serial(date: CivilDate, system: DateSystem) -> Option<i64> {
    match system {
        DateSystem::Excel1900 => date_to_serial_1900(date),
        DateSystem::Excel1904 => date_to_serial_1904(date),
    }
}

fn date_to_serial_1900(date: CivilDate) -> Option<i64> {
    if date.year == 1900 && date.month == 1 && date.day == 0 {
        return Some(0);
    }
    if date.lotus_leap || (date.year == 1900 && date.month == 2 && date.day == 29) {
        return Some(60);
    }
    if date.day == 0 || date.month == 0 {
        return None;
    }
    let unix = days_from_civil(date.year, date.month, date.day)?;
    let serial = if unix <= UNIX_DAYS_1900_JAN1 + 58 {
        unix - UNIX_DAYS_1900_JAN1 + 1
    } else {
        unix - UNIX_DAYS_1900_JAN1 + 2
    };
    if serial < 0 || serial > MAX_SERIAL_1900 {
        None
    } else {
        Some(serial)
    }
}

fn date_to_serial_1904(date: CivilDate) -> Option<i64> {
    if date.lotus_leap || date.day == 0 || date.month == 0 {
        return None;
    }
    let unix = days_from_civil(date.year, date.month, date.day)?;
    Some(unix - UNIX_DAYS_1904_JAN1)
}

/// Weekday with Sunday = 0, matching `WEEKDAY(serial, 1) - 1` in Excel.
#[must_use]
pub fn weekday_sun0(serial: i64, system: DateSystem) -> Option<u8> {
    match system {
        DateSystem::Excel1900 => {
            if serial < 0 || serial > MAX_SERIAL_1900 {
                return None;
            }
            Some(((serial - 1).rem_euclid(7)) as u8)
        }
        DateSystem::Excel1904 => {
            if serial > MAX_SERIAL_1904 {
                return None;
            }
            Some(((serial + 5).rem_euclid(7)) as u8)
        }
    }
}

/// Split a serial into an integer date serial and a time fraction in `[0, 1)`.
#[must_use]
pub fn split_serial(serial: f64) -> Option<(i64, f64)> {
    if !serial.is_finite() {
        return None;
    }
    let day = serial.floor();
    if !day.is_finite() || day > i64::MAX as f64 || day < i64::MIN as f64 {
        return None;
    }
    let day_i = day as i64;
    let frac = (serial - day).clamp(0.0, 0.999_999_999_999);
    Some((day_i, frac))
}

/// Hours, minutes, seconds, and subsecond digits from a day fraction.
#[must_use]
pub fn time_from_fraction(frac: f64, subsec_digits: u8) -> TimeOfDay {
    let frac = if frac.is_finite() {
        frac.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let digits = u32::from(subsec_digits.min(3));
    let scale = 10u32.pow(digits);
    let ticks_f = frac * 86_400.0 * f64::from(scale);
    let ticks = round_half_away(ticks_f).max(0.0) as u64;
    let day_ticks = 86_400u64 * u64::from(scale);
    let overflow_days = ticks / day_ticks;
    let mut rem = ticks % day_ticks;
    let sub = (rem % u64::from(scale)) as u32;
    rem /= u64::from(scale);
    let second = (rem % 60) as u8;
    rem /= 60;
    let minute = (rem % 60) as u8;
    let hour = (rem / 60) as u8;
    TimeOfDay {
        hour,
        minute,
        second,
        subsec: sub,
        overflow_days,
    }
}

/// Clock fields derived from a day fraction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TimeOfDay {
    /// Hour `0..=23` (24 is normalized into [`Self::overflow_days`]).
    pub hour: u8,
    /// Minute `0..=59`.
    pub minute: u8,
    /// Second `0..=59`.
    pub second: u8,
    /// Subsecond scaled by `10^subsec_digits`.
    pub subsec: u32,
    /// Whole days produced by rounding (usually 0).
    pub overflow_days: u64,
}

/// Total elapsed hours/minutes/seconds for `[h]` / `[m]` / `[s]`.
#[must_use]
pub fn elapsed(serial: f64) -> Option<(u64, u64, u64)> {
    if !serial.is_finite() {
        return None;
    }
    let secs = round_half_away(serial.abs() * 86_400.0).max(0.0) as u64;
    let hours = secs / 3_600;
    let minutes = secs / 60;
    Some((hours, minutes, secs))
}

fn round_half_away(x: f64) -> f64 {
    if !x.is_finite() {
        return 0.0;
    }
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let ax = x.abs();
    let floor = ax.floor();
    if ax - floor >= 0.5 {
        sign * (floor + 1.0)
    } else {
        sign * floor
    }
}

fn civil_from_days(z: i64) -> (i32, u8, u8) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u8, d as u8)
}

fn days_from_civil(y: i32, m: u8, d: u8) -> Option<i64> {
    if !(1..=12).contains(&m) || d == 0 || d > 31 {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 {
        i64::from(y)
    } else {
        i64::from(y) - 399
    } / 400;
    let yoe = (i64::from(y) - era * 400) as u32;
    let mp = if m > 2 { u32::from(m) - 3 } else { u32::from(m) + 9 };
    let doy = (153 * mp + 2) / 5 + u32::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + i64::from(doe) - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_epochs_match_hinnant() {
        assert_eq!(days_from_civil(1900, 1, 1), Some(UNIX_DAYS_1900_JAN1));
        assert_eq!(days_from_civil(1904, 1, 1), Some(UNIX_DAYS_1904_JAN1));
        assert_eq!(civil_from_days(UNIX_DAYS_1900_JAN1), (1900, 1, 1));
        assert_eq!(civil_from_days(UNIX_DAYS_1904_JAN1), (1904, 1, 1));
        assert_eq!(days_from_civil(1970, 1, 1), Some(0));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn lotus_and_neighbors() {
        let d0 = serial_to_date(0, DateSystem::Excel1900).unwrap();
        assert_eq!(
            d0,
            CivilDate {
                year: 1900,
                month: 1,
                day: 0,
                lotus_leap: false
            }
        );
        let d1 = serial_to_date(1, DateSystem::Excel1900).unwrap();
        assert_eq!((d1.year, d1.month, d1.day), (1900, 1, 1));
        let d59 = serial_to_date(59, DateSystem::Excel1900).unwrap();
        assert_eq!((d59.year, d59.month, d59.day), (1900, 2, 28));
        let d60 = serial_to_date(60, DateSystem::Excel1900).unwrap();
        assert!(d60.lotus_leap);
        assert_eq!((d60.year, d60.month, d60.day), (1900, 2, 29));
        let d61 = serial_to_date(61, DateSystem::Excel1900).unwrap();
        assert_eq!((d61.year, d61.month, d61.day), (1900, 3, 1));
        let max = serial_to_date(MAX_SERIAL_1900, DateSystem::Excel1900).unwrap();
        assert_eq!((max.year, max.month, max.day), (9999, 12, 31));
        assert!(serial_to_date(MAX_SERIAL_1900 + 1, DateSystem::Excel1900).is_none());
        assert!(serial_to_date(-1, DateSystem::Excel1900).is_none());
    }

    #[test]
    fn system_1904_has_no_lotus() {
        let d0 = serial_to_date(0, DateSystem::Excel1904).unwrap();
        assert_eq!((d0.year, d0.month, d0.day), (1904, 1, 1));
        let d59 = serial_to_date(59, DateSystem::Excel1904).unwrap();
        assert_eq!((d59.year, d59.month, d59.day), (1904, 2, 29));
        assert!(!d59.lotus_leap);
        let d60 = serial_to_date(60, DateSystem::Excel1904).unwrap();
        assert_eq!((d60.year, d60.month, d60.day), (1904, 3, 1));
        let max = serial_to_date(MAX_SERIAL_1904, DateSystem::Excel1904).unwrap();
        assert_eq!((max.year, max.month, max.day), (9999, 12, 31));
        let before = serial_to_date(-1, DateSystem::Excel1904).unwrap();
        assert_eq!((before.year, before.month, before.day), (1903, 12, 31));
    }

    #[test]
    fn round_trip_1900() {
        for s in [0, 1, 59, 60, 61, 367, 1462, 36526, MAX_SERIAL_1900] {
            let d = serial_to_date(s, DateSystem::Excel1900).unwrap();
            assert_eq!(date_to_serial(d, DateSystem::Excel1900), Some(s), "serial {s}");
        }
    }

    #[test]
    fn weekday_excel_sunday_origin() {
        assert_eq!(weekday_sun0(0, DateSystem::Excel1900), Some(6));
        assert_eq!(weekday_sun0(1, DateSystem::Excel1900), Some(0));
        assert_eq!(weekday_sun0(59, DateSystem::Excel1900), Some(2));
        assert_eq!(weekday_sun0(60, DateSystem::Excel1900), Some(3));
        assert_eq!(weekday_sun0(61, DateSystem::Excel1900), Some(4));
        assert_eq!(weekday_sun0(0, DateSystem::Excel1904), Some(5));
        assert_eq!(weekday_sun0(59, DateSystem::Excel1904), Some(1));
        assert_eq!(weekday_sun0(-1, DateSystem::Excel1904), Some(4));
        assert_eq!(weekday_sun0(MAX_SERIAL_1900, DateSystem::Excel1900), Some(5));
    }

    #[test]
    fn y2k_serial() {
        let d = serial_to_date(36_526, DateSystem::Excel1900).unwrap();
        assert_eq!((d.year, d.month, d.day), (2000, 1, 1));
    }

    #[test]
    fn noon_fraction() {
        let t = time_from_fraction(0.5, 0);
        assert_eq!((t.hour, t.minute, t.second, t.overflow_days), (12, 0, 0, 0));
        let (h, m, s) = elapsed(1.0).unwrap();
        assert_eq!((h, m, s), (24, 1440, 86_400));
    }
}

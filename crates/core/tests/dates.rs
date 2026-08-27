//! WP-06 date serial properties.

use omacell_core::dates::{
    CivilDate, DateSystem, MAX_SERIAL_1900, MAX_SERIAL_1904, date_to_serial, serial_to_date,
    weekday_sun0,
};
use proptest::prelude::*;
use proptest::test_runner::{Config as ProptestConfig, FileFailurePersistence};

fn cfg() -> ProptestConfig {
    ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        cases: 256,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(cfg())]
    #[test]
    fn roundtrip_1900_except_lotus(serial in 1i64..MAX_SERIAL_1900) {
        prop_assume!(serial != 60);
        let d = serial_to_date(serial, DateSystem::Excel1900).unwrap();
        prop_assert!(!d.lotus_leap);
        prop_assert_eq!(date_to_serial(d, DateSystem::Excel1900), Some(serial));
    }

    #[test]
    fn roundtrip_1904(serial in 0i64..MAX_SERIAL_1904) {
        let d = serial_to_date(serial, DateSystem::Excel1904).unwrap();
        prop_assert!(!d.lotus_leap);
        prop_assert_eq!(date_to_serial(d, DateSystem::Excel1904), Some(serial));
    }
}

#[test]
fn lotus_round_trip() {
    let d = serial_to_date(60, DateSystem::Excel1900).unwrap();
    assert!(d.lotus_leap);
    assert_eq!(
        d,
        CivilDate {
            year: 1900,
            month: 2,
            day: 29,
            lotus_leap: true
        }
    );
    assert_eq!(date_to_serial(d, DateSystem::Excel1900), Some(60));
    assert!(date_to_serial(d, DateSystem::Excel1904).is_none());
}

#[test]
fn weekday_known() {
    assert_eq!(weekday_sun0(1, DateSystem::Excel1900), Some(0));
    assert_eq!(weekday_sun0(60, DateSystem::Excel1900), Some(3));
    assert_eq!(weekday_sun0(0, DateSystem::Excel1904), Some(5));
}

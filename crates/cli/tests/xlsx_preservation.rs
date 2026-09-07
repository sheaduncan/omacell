//! XLSX opaque-package preservation follows the live file policy.

use assert_cmd::Command;
use omacell_core::workbook::Workbook;
use omacell_io::xlsx::{self, PreservedPart};

const OPAQUE_PART: &str = "xl/opaque-review.bin";

fn bin(home: &std::path::Path) -> Command {
    let mut command = Command::cargo_bin("omacell").unwrap();
    command
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME")
        .env("HOME", home);
    command
}

fn opaque_fixture() -> Vec<u8> {
    let mut seeded =
        xlsx::open_bytes(&xlsx::save_workbook_bytes(&Workbook::new()).unwrap()).unwrap();
    seeded.package.parts.insert(
        OPAQUE_PART.into(),
        PreservedPart {
            name: OPAQUE_PART.into(),
            content_type: Some("application/octet-stream".into()),
            bytes: b"opaque-review-data".to_vec(),
        },
    );
    xlsx::save_bytes(&seeded).unwrap()
}

fn save_with_policy(home: &std::path::Path, path: &std::path::Path, preserve: bool) {
    bin(home)
        .arg("--set")
        .arg(format!("files.xlsx.preserve_unknown_parts={preserve}"))
        .args(["set", path.to_str().unwrap(), "A1", "saved"])
        .assert()
        .success();
}

#[test]
fn xlsx_preservation_setting_controls_opaque_package_parts() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = opaque_fixture();

    for preserve in [true, false] {
        let home = temp.path().join(format!("home-{preserve}"));
        let path = temp.path().join(format!("preserve-{preserve}.xlsx"));
        std::fs::write(&path, &fixture).unwrap();

        save_with_policy(&home, &path, preserve);

        let reopened = xlsx::open(&path).unwrap();
        assert_eq!(
            reopened.package.part(OPAQUE_PART).is_some(),
            preserve,
            "preserve={preserve}"
        );

        if !preserve {
            save_with_policy(&home, &path, true);
            let reopened = xlsx::open(&path).unwrap();
            assert!(
                reopened.package.part(OPAQUE_PART).is_none(),
                "discarded package parts must not reappear when preservation is enabled later"
            );
        }
    }
}

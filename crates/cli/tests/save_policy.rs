//! Every writable workbook format uses the configured backup and peer-lock policy.

use assert_cmd::Command;
use omacell_core::workbook::Workbook;
use omacell_io::omc::{OmcDocument, write_to_path};
use omacell_io::xlsx::lock_path;
use tempfile::TempDir;

fn bin(home: &std::path::Path) -> Command {
    let mut command = Command::cargo_bin("omacell").unwrap();
    command
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME")
        .env("HOME", home);
    command
}

#[test]
fn csv_tsv_and_omc_saves_rotate_backups_and_honor_peer_locks() {
    let dir = TempDir::new().unwrap();
    for extension in ["csv", "tsv", "omc"] {
        let path = dir.path().join(format!("book.{extension}"));
        if extension == "omc" {
            write_to_path(&OmcDocument::from_workbook(Workbook::new()), &path).unwrap();
        } else {
            std::fs::write(&path, b"old\n").unwrap();
        }
        let original = std::fs::read(&path).unwrap();

        bin(dir.path())
            .args([
                "--set",
                "files.keep_backups=1",
                "set",
                path.to_str().unwrap(),
                "A1",
                "saved",
            ])
            .assert()
            .success();
        assert_eq!(
            std::fs::read(path.with_extension(format!("{extension}.bak.1"))).unwrap(),
            original,
            "{extension} save did not retain the original backup"
        );

        let saved = std::fs::read(&path).unwrap();
        std::fs::write(
            lock_path(&path),
            "foreign,lock,file:///x,file:///y,now;",
        )
        .unwrap();
        bin(dir.path())
            .args(["set", path.to_str().unwrap(), "A1", "blocked"])
            .assert()
            .code(1);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            saved,
            "{extension} save ignored its peer lock"
        );
    }
}

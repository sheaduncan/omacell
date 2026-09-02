#[path = "../../../tests/support/libreoffice.rs"]
mod libreoffice;

#[cfg(unix)]
fn fake_binary(name: &str, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!(
        "omacell-lo-probe-test-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("libreoffice");
    std::fs::write(&path, body).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

#[test]
#[cfg(unix)]
fn version_only_binary_is_not_a_calc_oracle() {
    let binary = fake_binary("version-only", "#!/bin/sh\nexit 0\n");
    assert!(!libreoffice::probe_binary(&binary));
    std::fs::remove_dir_all(binary.parent().unwrap()).unwrap();
    let _ = libreoffice::find_calc();
}

#[test]
#[cfg(unix)]
fn converter_that_writes_the_expected_file_is_a_calc_oracle() {
    let binary = fake_binary(
        "capable",
        r#"#!/bin/sh
out=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--outdir" ]; then
    shift
    out="$1"
  fi
  shift
done
printf '1.5\n' > "$out/l1_values.csv"
"#,
    );
    assert!(libreoffice::probe_binary(&binary));
    std::fs::remove_dir_all(binary.parent().unwrap()).unwrap();
}

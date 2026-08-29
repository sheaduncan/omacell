//! Generate completions and the man page into `target/dist`.

use std::path::PathBuf;

#[test]
fn generate_completions_and_man() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/dist");
    let dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(|p| PathBuf::from(p).join("dist"))
        .unwrap_or(dir);
    omacell_cli::write_dist(&dir).expect("write dist");
    for name in ["omacell.bash", "omacell.fish", "_omacell", "omacell.1"] {
        let path = dir.join(name);
        assert!(path.is_file(), "missing {name} in {}", dir.display());
    }
}

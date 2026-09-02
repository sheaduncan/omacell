use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_PROBE: AtomicU64 = AtomicU64::new(0);
static CALC: OnceLock<Option<PathBuf>> = OnceLock::new();
const PROBE_XLSX: &[u8] = include_bytes!("../corpus/xlsx/l1_values.xlsx");

/// Find a LibreOffice executable whose Calc filters can convert a known XLSX.
pub(crate) fn find_calc() -> Option<PathBuf> {
    CALC.get_or_init(find_calc_uncached).clone()
}

fn find_calc_uncached() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for name in ["soffice", "libreoffice"] {
            let candidate = directory.join(name);
            if candidate.is_file() && probe_binary(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// Return whether `binary` performs a real Calc conversion, not just `--version`.
pub(crate) fn probe_binary(binary: &Path) -> bool {
    let Ok(directory) = probe_directory() else {
        return false;
    };
    let result = run_probe(binary, &directory).unwrap_or(false);
    let _ = std::fs::remove_dir_all(directory);
    result
}

fn probe_directory() -> io::Result<PathBuf> {
    for _ in 0..100 {
        let sequence = NEXT_PROBE.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "omacell-libreoffice-probe-{}-{sequence}",
            std::process::id()
        ));
        match std::fs::create_dir(&directory) {
            Ok(()) => return Ok(directory),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique LibreOffice probe directory",
    ))
}

fn run_probe(binary: &Path, directory: &Path) -> io::Result<bool> {
    let input = directory.join("l1_values.xlsx");
    let output_directory = directory.join("output");
    let profile = directory.join("profile");
    std::fs::create_dir(&output_directory)?;
    std::fs::create_dir(&profile)?;
    std::fs::write(&input, PROBE_XLSX)?;
    let output = Command::new(binary)
        .arg(format!(
            "-env:UserInstallation=file://{}",
            profile.display()
        ))
        .env("HOME", directory)
        .env("XDG_CACHE_HOME", directory.join("cache"))
        .env("XDG_CONFIG_HOME", directory.join("config"))
        .env("SAL_USE_VCLPLUGIN", "svp")
        .args(["--headless", "--convert-to", "csv", "--outdir"])
        .arg(&output_directory)
        .arg(&input)
        .output()?;
    Ok(output.status.success() && output_directory.join("l1_values.csv").is_file())
}

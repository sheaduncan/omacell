//! Legacy `.xls` read via LibreOffice (`soffice --headless --convert-to xlsx`).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use omacell_core::error::CoreError;

use crate::error;
use crate::temp::PrivateTempDir;
use crate::xlsx::{self, peer_lock_blocks};

const CONVERT_TIMEOUT: Duration = Duration::from_secs(60);

/// Open a `.xls` file by converting it to `.xlsx` in a temp directory.
pub fn open_xls(path: &Path, libreoffice_fallback: bool) -> Result<xlsx::XlsxDocument, CoreError> {
    peer_lock_blocks(path)?;
    if !libreoffice_fallback {
        return Err(error::xls_bridge(
            "LibreOffice fallback is disabled ([integrations] libreoffice_fallback = false)",
        ));
    }
    let soffice = find_libreoffice()
        .ok_or_else(|| error::xls_bridge("LibreOffice (soffice) is not installed"))?;
    let source = path
        .canonicalize()
        .map_err(|err| error::xls_bridge(format!("{}: {err}", path.display())))?;
    let dir = PrivateTempDir::new("xls")
        .map_err(|err| error::xls_bridge(format!("create private temp directory: {err}")))?;
    let status = run_timed(
        Command::new(&soffice)
            .arg(format!(
                "-env:UserInstallation=file://{}",
                dir.path().join("profile").display()
            ))
            .env("HOME", dir.path())
            .env("SAL_USE_VCLPLUGIN", "svp")
            .args(["--headless", "--convert-to", "xlsx", "--outdir"])
            .arg(dir.path())
            .arg(&source)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
        CONVERT_TIMEOUT,
    )?;
    if !status {
        return Err(error::xls_bridge(format!(
            "{} failed to convert {}",
            soffice.display(),
            path.display()
        )));
    }
    let stem = source
        .file_stem()
        .ok_or_else(|| error::xls_bridge("input path has no file name"))?;
    let mut converted = dir.path().join(stem);
    converted.set_extension("xlsx");
    if !converted.exists() {
        return Err(error::xls_bridge(
            "LibreOffice did not produce an .xlsx file",
        ));
    }
    xlsx::open(&converted)
}

fn find_libreoffice() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in ["soffice", "libreoffice"] {
            let candidate = dir.join(name);
            let Ok(metadata) = candidate.metadata() else {
                continue;
            };
            #[cfg(unix)]
            let executable = metadata.is_file() && metadata.permissions().mode() & 0o111 != 0;
            #[cfg(not(unix))]
            let executable = metadata.is_file();
            if executable {
                return Some(candidate);
            }
        }
    }
    None
}

fn run_timed(cmd: &mut Command, timeout: Duration) -> Result<bool, CoreError> {
    let mut child = cmd.spawn().map_err(|e| error::xls_bridge(e.to_string()))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.success()),
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error::xls_bridge("LibreOffice conversion timed out"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(error::xls_bridge(e.to_string())),
        }
    }
}

//! Legacy `.xls` read via LibreOffice (`soffice --headless --convert-to xlsx`).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use omacell_core::error::CoreError;

use crate::error;
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
    let soffice = ["soffice", "libreoffice"]
        .into_iter()
        .find(|bin| Command::new(bin).arg("--version").output().is_ok())
        .ok_or_else(|| error::xls_bridge("LibreOffice (soffice) is not installed"))?;
    let dir = tempfile_dir()?;
    let status = run_timed(
        Command::new(soffice)
            .arg(format!(
                "-env:UserInstallation=file://{}",
                dir.join("profile").display()
            ))
            .env("HOME", &dir)
            .env("SAL_USE_VCLPLUGIN", "svp")
            .args(["--headless", "--convert-to", "xlsx", "--outdir"])
            .arg(&dir)
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
        CONVERT_TIMEOUT,
    )?;
    if !status {
        return Err(error::xls_bridge(format!(
            "{soffice} failed to convert {}",
            path.display()
        )));
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("book");
    let converted = dir.join(format!("{stem}.xlsx"));
    if !converted.exists() {
        return Err(error::xls_bridge(
            "LibreOffice did not produce an .xlsx file",
        ));
    }
    xlsx::open(&converted)
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

fn tempfile_dir() -> Result<PathBuf, CoreError> {
    let dir = std::env::temp_dir().join(format!(
        "omacell-xls-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).map_err(|e| error::xls_bridge(e.to_string()))?;
    Ok(dir)
}

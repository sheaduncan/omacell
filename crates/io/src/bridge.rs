//! Isolated native legacy BIFF `.xls` reader.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use omacell_core::error::CoreError;
use omacell_core::workbook::Workbook;

use crate::error;
use crate::xlsx::{open_bytes, peer_lock_blocks};

/// Maximum accepted legacy workbook size before BIFF parsing.
pub const MAX_XLS_BYTES: u64 = 256 * 1024 * 1024;

const WORKER_NAME: &str = "omacell-xls-worker";
const WORKER_PROTOCOL: &str = "--stdio-v1";
const WORKER_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_WORKER_OUTPUT: usize = 256 * 1024 * 1024;
const MAX_WORKER_DIAGNOSTIC: usize = 64 * 1024;

/// Open a legacy BIFF `.xls` workbook through Omacell's resource-limited
/// companion parser. LibreOffice is not used.
pub fn open_xls(path: &Path) -> Result<Workbook, CoreError> {
    peer_lock_blocks(path)?;
    let file = std::fs::File::open(path)
        .map_err(|source| error::xls_bridge(format!("{}: {source}", path.display())))?;
    let mut bytes = Vec::new();
    file.take(MAX_XLS_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| error::xls_bridge(format!("{}: {source}", path.display())))?;
    validate_size(bytes.len() as u64)?;
    open_xls_bytes(&bytes)
}

/// Open legacy BIFF `.xls` bytes through Omacell's resource-limited companion
/// parser. LibreOffice is not used.
pub fn open_xls_bytes(bytes: &[u8]) -> Result<Workbook, CoreError> {
    validate_size(bytes.len() as u64)?;
    validate_cfb_difat(bytes)?;
    let worker = find_worker()?;
    let output = run_worker(&worker, bytes, WORKER_TIMEOUT, MAX_WORKER_OUTPUT)?;
    Ok(open_bytes(&output)?.workbook)
}

fn validate_size(len: u64) -> Result<(), CoreError> {
    if len > MAX_XLS_BYTES {
        return Err(error::xlsx_limit(format!(
            "legacy .xls file is {len} bytes; maximum is {MAX_XLS_BYTES}"
        )));
    }
    Ok(())
}

fn validate_cfb_difat(bytes: &[u8]) -> Result<(), CoreError> {
    const SIGNATURE: &[u8; 8] = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1";
    const MAX_REGULAR_SECTOR: u32 = 0xFFFF_FFFA;
    const END_OF_CHAIN: u32 = 0xFFFF_FFFE;
    const FREE_SECTOR: u32 = 0xFFFF_FFFF;

    if bytes.len() < 512 || &bytes[..8] != SIGNATURE {
        return Err(error::xls_bridge("invalid OLE compound-file header"));
    }
    if read_u16(bytes, 28) != Some(0xFFFE) || read_u16(bytes, 32) != Some(6) {
        return Err(error::xls_bridge(
            "invalid OLE compound-file byte order or mini-sector size",
        ));
    }
    let sector_shift = read_u16(bytes, 30)
        .ok_or_else(|| error::xls_bridge("truncated OLE compound-file header"))?;
    let sector_size = match sector_shift {
        9 => 512_usize,
        12 => 4_096_usize,
        _ => return Err(error::xls_bridge("invalid OLE compound-file sector size")),
    };
    if bytes.len() < sector_size || !(bytes.len() - sector_size).is_multiple_of(sector_size) {
        return Err(error::xls_bridge("truncated OLE compound-file sector"));
    }
    let sector_count = (bytes.len() - sector_size) / sector_size;
    let fat_count = read_u32(bytes, 44)
        .ok_or_else(|| error::xls_bridge("truncated OLE compound-file header"))?
        as usize;
    let difat_count = read_u32(bytes, 72)
        .ok_or_else(|| error::xls_bridge("truncated OLE compound-file header"))?
        as usize;
    if fat_count > sector_count || difat_count > sector_count {
        return Err(error::xls_bridge(
            "OLE compound-file allocation table exceeds the file",
        ));
    }

    let mut fat_sectors = BTreeSet::new();
    for offset in (76..512).step_by(4) {
        record_fat_sector(bytes, offset, sector_count, &mut fat_sectors)?;
    }

    let mut seen_difat = BTreeSet::new();
    let mut difat_sector = read_u32(bytes, 68)
        .ok_or_else(|| error::xls_bridge("truncated OLE compound-file header"))?;
    for index in 0..difat_count {
        let id = regular_sector(difat_sector, sector_count, "DIFAT")?;
        if !seen_difat.insert(id) {
            return Err(error::xls_bridge("cyclic OLE compound-file DIFAT chain"));
        }
        let start = sector_size
            .checked_add(
                id.checked_mul(sector_size)
                    .ok_or_else(|| error::xls_bridge("OLE compound-file sector offset overflow"))?,
            )
            .ok_or_else(|| error::xls_bridge("OLE compound-file sector offset overflow"))?;
        let next_offset = start + sector_size - 4;
        for offset in (start..next_offset).step_by(4) {
            record_fat_sector(bytes, offset, sector_count, &mut fat_sectors)?;
        }
        difat_sector = read_u32(bytes, next_offset)
            .ok_or_else(|| error::xls_bridge("truncated OLE compound-file DIFAT sector"))?;
        if index + 1 < difat_count && difat_sector >= MAX_REGULAR_SECTOR {
            return Err(error::xls_bridge("short OLE compound-file DIFAT chain"));
        }
    }
    if difat_count == 0 {
        if difat_sector != END_OF_CHAIN && difat_sector != FREE_SECTOR {
            return Err(error::xls_bridge(
                "unexpected OLE compound-file DIFAT chain",
            ));
        }
    } else if difat_sector != END_OF_CHAIN {
        return Err(error::xls_bridge(
            "long or cyclic OLE compound-file DIFAT chain",
        ));
    }
    if fat_sectors.len() != fat_count {
        return Err(error::xls_bridge(format!(
            "OLE compound-file declares {fat_count} FAT sectors but lists {}",
            fat_sectors.len()
        )));
    }
    Ok(())
}

fn record_fat_sector(
    bytes: &[u8],
    offset: usize,
    sector_count: usize,
    sectors: &mut BTreeSet<usize>,
) -> Result<(), CoreError> {
    const MAX_REGULAR_SECTOR: u32 = 0xFFFF_FFFA;
    let id = read_u32(bytes, offset)
        .ok_or_else(|| error::xls_bridge("truncated OLE compound-file DIFAT entry"))?;
    if id < MAX_REGULAR_SECTOR {
        let id = regular_sector(id, sector_count, "FAT")?;
        if !sectors.insert(id) {
            return Err(error::xls_bridge("duplicate OLE compound-file FAT sector"));
        }
    }
    Ok(())
}

fn regular_sector(id: u32, sector_count: usize, kind: &str) -> Result<usize, CoreError> {
    let id = usize::try_from(id).map_err(|_| error::xls_bridge("invalid OLE sector id"))?;
    if id >= sector_count {
        return Err(error::xls_bridge(format!(
            "OLE compound-file {kind} sector points past end of file"
        )));
    }
    Ok(id)
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let raw: [u8; 2] = bytes.get(offset..offset + 2)?.try_into().ok()?;
    Some(u16::from_le_bytes(raw))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(offset..offset + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(raw))
}

fn find_worker() -> Result<PathBuf, CoreError> {
    let executable = std::env::current_exe().map_err(|source| {
        error::xls_bridge(format!("cannot locate Omacell executable: {source}"))
    })?;
    let directory = executable
        .parent()
        .ok_or_else(|| error::xls_bridge("Omacell executable has no parent directory"))?;
    let mut candidates = vec![directory.join(WORKER_NAME)];
    if directory.file_name() == Some(std::ffi::OsStr::new("deps"))
        && let Some(target_dir) = directory.parent()
    {
        candidates.push(target_dir.join(WORKER_NAME));
    }
    if let Some(prefix) = directory.parent() {
        candidates.push(prefix.join("lib/omacell").join(WORKER_NAME));
    }
    for candidate in candidates {
        let Ok(metadata) = std::fs::symlink_metadata(&candidate) else {
            continue;
        };
        if metadata.file_type().is_file() && worker_is_executable(&metadata) {
            return Ok(candidate);
        }
    }
    Err(error::xls_bridge(
        "the private omacell-xls-worker companion is missing; reinstall Omacell",
    ))
}

#[cfg(unix)]
fn worker_is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn worker_is_executable(_metadata: &std::fs::Metadata) -> bool {
    true
}

fn run_worker(
    worker: &Path,
    bytes: &[u8],
    timeout: Duration,
    max_output: usize,
) -> Result<Vec<u8>, CoreError> {
    let mut command = Command::new(worker);
    command.arg(WORKER_PROTOCOL);
    run_worker_command(&mut command, bytes, timeout, max_output)
}

fn run_worker_command(
    command: &mut Command,
    bytes: &[u8],
    timeout: Duration,
    max_output: usize,
) -> Result<Vec<u8>, CoreError> {
    let mut child = command
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| error::xls_bridge(format!("cannot start XLS worker: {source}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| error::xls_bridge("XLS worker stdout was not captured"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| error::xls_bridge("XLS worker stderr was not captured"))?;
    let stdout_reader = read_stdout(stdout, max_output);
    let stderr_reader = read_stderr(stderr);

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| error::xls_bridge("XLS worker stdin was not captured"))?;
    let (status, write_result) = std::thread::scope(|scope| {
        let writer = scope.spawn(move || {
            let mut stdin = stdin;
            stdin.write_all(bytes)
        });
        let status = wait_for_worker(&mut child, timeout);
        let write_result = writer
            .join()
            .map_err(|_| error::xls_bridge("XLS worker stdin writer panicked"))?;
        Ok::<_, CoreError>((status, write_result))
    })?;
    let stdout = join_reader(stdout_reader, "stdout")?;
    let stderr = join_reader(stderr_reader, "stderr")?;
    let status = status?;
    if !status.success() {
        let diagnostic = String::from_utf8_lossy(&stderr);
        let diagnostic = diagnostic.trim();
        return Err(error::xls_bridge(if diagnostic.is_empty() {
            format!("XLS worker terminated with {status}")
        } else {
            format!("XLS worker terminated with {status}: {diagnostic}")
        }));
    }
    write_result.map_err(|source| {
        error::xls_bridge(format!("cannot send workbook to XLS worker: {source}"))
    })?;
    Ok(stdout)
}

fn wait_for_worker(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Result<ExitStatus, CoreError> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(source) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error::xls_bridge(format!(
                    "cannot wait for XLS worker: {source}"
                )));
            }
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error::xls_bridge(format!(
                "XLS worker exceeded the {} second wall-time limit",
                timeout.as_secs_f64()
            )));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn read_stdout(reader: ChildStdout, max_output: usize) -> JoinHandle<std::io::Result<Vec<u8>>> {
    std::thread::spawn(move || read_bounded(reader, max_output))
}

fn read_stderr(reader: ChildStderr) -> JoinHandle<std::io::Result<Vec<u8>>> {
    std::thread::spawn(move || read_bounded(reader, MAX_WORKER_DIAGNOSTIC))
}

fn read_bounded(mut reader: impl Read, maximum: usize) -> std::io::Result<Vec<u8>> {
    let limit = u64::try_from(maximum)
        .map_err(|_| std::io::Error::other("worker stream limit is not representable"))?
        .saturating_add(1);
    let mut bytes = Vec::new();
    reader.by_ref().take(limit).read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err(std::io::Error::other("worker stream exceeded its limit"));
    }
    Ok(bytes)
}

fn join_reader(
    reader: JoinHandle<std::io::Result<Vec<u8>>>,
    stream: &str,
) -> Result<Vec<u8>, CoreError> {
    reader
        .join()
        .map_err(|_| error::xls_bridge(format!("XLS worker {stream} reader panicked")))?
        .map_err(|source| error::xls_bridge(format!("cannot read XLS worker {stream}: {source}")))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn abnormal_worker_exit_is_a_typed_error() {
        let error = run_worker(
            Path::new("/bin/false"),
            b"input",
            Duration::from_secs(1),
            1_024,
        )
        .unwrap_err();
        assert_eq!(error.code, crate::error::codes::XLS_BRIDGE);
    }

    #[test]
    fn worker_timeout_and_oversized_output_are_bounded() {
        let input = vec![0_u8; 256 * 1_024];
        let mut hang = Command::new("/bin/sh");
        hang.args(["-c", "while :; do :; done"]);
        let error =
            run_worker_command(&mut hang, &input, Duration::from_millis(50), 1_024).unwrap_err();
        assert!(error.message.contains("wall-time"), "{error}");

        let mut oversized = Command::new("/bin/sh");
        oversized.args([
            "-c",
            "i=0; while [ \"$i\" -lt 2048 ]; do printf x; i=$((i + 1)); done",
        ]);
        let error =
            run_worker_command(&mut oversized, b"", Duration::from_secs(1), 1_024).unwrap_err();
        assert!(error.message.contains("exceeded its limit"), "{error}");
    }
}

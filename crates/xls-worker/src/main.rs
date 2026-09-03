//! Resource-limited stdio companion for legacy `.xls` parsing.
#![forbid(unsafe_code)]

use std::io::{Read, Write};

use rustix::process::{Resource, Rlimit, setrlimit};

const PROTOCOL: &str = "--stdio-v1";
const MEMORY_LIMIT: u64 = 1024 * 1024 * 1024;
const CPU_SECONDS: u64 = 10;

fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args_os();
    let _program = args.next();
    if args.next().as_deref() != Some(std::ffi::OsStr::new(PROTOCOL)) || args.next().is_some() {
        return Err("invalid XLS worker invocation".into());
    }
    install_limits()?;

    let mut bytes = Vec::new();
    std::io::stdin()
        .take(omacell_xls_worker::MAX_XLS_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read XLS worker input: {error}"))?;
    if bytes.len() as u64 > omacell_xls_worker::MAX_XLS_BYTES {
        return Err(format!(
            "legacy .xls input exceeds {} bytes",
            omacell_xls_worker::MAX_XLS_BYTES
        ));
    }

    let workbook = omacell_xls_worker::parse_xls_bytes(&bytes)
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    let output = omacell_io::xlsx::save_workbook_bytes(&workbook)
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    std::io::stdout()
        .write_all(&output)
        .map_err(|error| format!("cannot write XLS worker output: {error}"))
}

fn install_limits() -> Result<(), String> {
    setrlimit(
        Resource::Core,
        Rlimit {
            current: Some(0),
            maximum: Some(0),
        },
    )
    .map_err(|error| format!("cannot disable XLS worker core dumps: {error}"))?;
    setrlimit(
        Resource::As,
        Rlimit {
            current: Some(MEMORY_LIMIT),
            maximum: Some(MEMORY_LIMIT),
        },
    )
    .map_err(|error| format!("cannot cap XLS worker address space: {error}"))?;
    setrlimit(
        Resource::Cpu,
        Rlimit {
            current: Some(CPU_SECONDS),
            maximum: Some(CPU_SECONDS),
        },
    )
    .map_err(|error| format!("cannot cap XLS worker CPU time: {error}"))
}

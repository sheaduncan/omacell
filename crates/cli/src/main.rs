//! Omacell command-line entry point.
#![forbid(unsafe_code)]

fn main() {
    let code = omacell_cli::run(std::env::args_os());
    std::process::exit(code);
}

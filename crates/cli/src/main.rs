//! Omacell command-line entry point.
//!
//! The full `omacell <group> <command>` surface lands in WP-13.
#![forbid(unsafe_code)]

use omacell_core::{PRODUCT_DISPLAY_NAME, PRODUCT_NAME};

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--version") | Some("-V") => {
            println!("{PRODUCT_NAME} {}", env!("CARGO_PKG_VERSION"));
        }
        Some("--help") | Some("-h") | None => {
            print_help();
        }
        Some(other) => {
            eprintln!("{PRODUCT_NAME}: unknown argument '{other}'. Try --help.");
            std::process::exit(2);
        }
    }
}

fn print_help() {
    println!(
        "{PRODUCT_DISPLAY_NAME} {} — a spreadsheet for Omarchy Linux",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("Usage: {PRODUCT_NAME} [--version]");
    println!();
    println!("The full command surface lands in WP-13.");
}

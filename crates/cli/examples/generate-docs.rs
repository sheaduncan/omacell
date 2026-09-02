//! Write or verify the generated CLI reference.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1).peekable();
    let check = matches!(args.peek(), Some(value) if value.as_os_str() == "--check");
    if check {
        let _ = args.next();
    }
    let path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("docs/book/cli-reference.md"));
    if args.next().is_some() {
        return Err("usage: generate-docs [--check] [PATH]".into());
    }
    let generated = omacell_cli::command_reference_markdown();
    if check {
        let committed = std::fs::read_to_string(&path)?;
        if committed != generated {
            return Err(format!(
                "{} is stale; run the generate-docs example without --check",
                path.display()
            )
            .into());
        }
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, generated)?;
    }
    Ok(())
}

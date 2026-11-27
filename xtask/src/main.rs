//! Explicit, reproducible preparation of derived assets. Never rewrites source assets.

mod build;

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

const USAGE: &str = "cargo xtask build-assets [--scale 4] [--source assets] [--output target/generated-assets]\nScale must be an integer from 1 to 64. Paths default to the workspace root.";

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref().and_then(|a| a.to_str()) {
        Some("build-assets") => {}
        Some("--help" | "-h") | None => {
            println!("{USAGE}");
            return Ok(());
        }
        _ => bail!("unknown task\n{USAGE}"),
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut source = root.join("assets");
    let mut output = root.join("target/generated-assets");
    let mut scale = 4;
    while let Some(option) = args.next() {
        if option == "--help" || option == "-h" {
            println!("{USAGE}");
            return Ok(());
        }
        let value: OsString = args.next().context("missing option value")?;
        match option.to_str() {
            Some("--source") => source = value.into(),
            Some("--output") => output = value.into(),
            Some("--scale") => {
                scale = value.to_str().context("scale must be UTF-8")?.parse()?;
            }
            _ => bail!("unknown option {option:?}\n{USAGE}"),
        }
    }
    let report = build::build_assets(&source, &output, scale)?;
    println!(
        "{} frames, RGBA8: {:.2} MiB -> {:.2} MiB (scale /{})\n{}",
        report.frames,
        report.source_rgba_bytes as f64 / 1_048_576.0,
        report.output_rgba_bytes as f64 / 1_048_576.0,
        scale,
        output.display(),
    );
    Ok(())
}

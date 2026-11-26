//! MCP stdio transport. Stdout is reserved for JSON-RPC; diagnostics go to stderr.
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use mosswake_mcp::{LevelServer, LevelStore};
use rmcp::{transport::stdio, ServiceExt};

#[tokio::main]
async fn main() -> Result<()> {
    let mut levels = PathBuf::from("levels");
    let mut tileset = PathBuf::from("assets/tilesets/mossy");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--levels-dir" => levels = args.next().context("--levels-dir requires a path")?.into(),
            "--tileset-dir" => {
                tileset = args.next().context("--tileset-dir requires a path")?.into()
            }
            "--help" | "-h" => {
                eprintln!("mosswake-mcp [--levels-dir levels] [--tileset-dir assets/tilesets/mossy]\nRun from the repository root, or supply both paths. MCP transport: stdio.");
                return Ok(());
            }
            _ => bail!("unknown option `{arg}`; use --help"),
        }
    }
    let server = LevelServer::new(LevelStore::new(levels, tileset)?);
    server.serve(stdio()).await?.waiting().await?;
    Ok(())
}

//! Game executable logic, as a library so integration tests can drive it.
//!
//! - [`server::ServerSession`]: authoritative server (`mosswake server --headless`).
//! - [`host::HostSession`]: server + local client in one process (`mosswake host`).
//! - [`input_buffer::InputBuffer`] / [`input_sender::InputSender`]: inputs on each side.
//! - [`play`]: solo play with rendering (`mosswake play <level>`).
//!
//! Gameplay stays in `mw_sim`; this crate only wires the simulation to the network.

pub mod host;
pub mod input_buffer;
pub mod input_sender;
pub mod play;
pub mod server;

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use mw_level::{Level, LevelBundle};

/// Repository root: `$MOSSWAKE_ROOT`, else the first ancestor of the current directory or
/// of the executable that holds `assets/manifest.json`.
pub fn find_root() -> Result<PathBuf> {
    if let Ok(root) = std::env::var("MOSSWAKE_ROOT") {
        return Ok(PathBuf::from(root));
    }
    let starts = [std::env::current_dir().ok(), std::env::current_exe().ok()];
    starts
        .into_iter()
        .flatten()
        .flat_map(|start| start.ancestors().map(Path::to_path_buf).collect::<Vec<_>>())
        .find(|dir| dir.join("assets/manifest.json").is_file())
        .context("cannot find the Mosswake assets (run from the repository or set MOSSWAKE_ROOT)")
}

/// Default `levels/` directory: under the repository root when it can be found, else
/// relative to the current directory.
pub fn default_levels_dir() -> PathBuf {
    find_root()
        .map(|root| root.join("levels"))
        .unwrap_or_else(|_| PathBuf::from("levels"))
}

/// Loads and validates `levels_dir/<id>`.
pub fn load_level(levels_dir: &Path, id: &str) -> Result<Level> {
    let dir = levels_dir.join(id);
    let bundle =
        LevelBundle::load(&dir).with_context(|| format!("cannot load level from {dir:?}"))?;
    let errors = bundle.validate();
    if !errors.is_empty() {
        let list: Vec<String> = errors.iter().map(ToString::to_string).collect();
        bail!("level `{id}` is invalid:\n  {}", list.join("\n  "));
    }
    Ok(bundle.level)
}

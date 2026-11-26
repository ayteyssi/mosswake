//! Game executable logic, as a library so integration tests can drive it.
//!
//! - [`server::ServerSession`]: authoritative server (`mosswake server --headless`).
//! - [`host::HostSession`]: server + local client in one process (`mosswake host`).
//! - [`input_buffer::InputBuffer`] / [`input_sender::InputSender`]: inputs on each side.
//! - [`level_sync::LevelSync`]: client side of the level transfer (download, cache, load).
//!
//! Gameplay stays in `mw_sim`; this crate only wires the simulation to the network.

pub mod host;
pub mod input_buffer;
pub mod input_sender;
pub mod level_sync;
pub mod server;

use std::path::Path;

use anyhow::{bail, Context, Result};
use mw_level::LevelBundle;

/// Loads and validates `levels_dir/<id>` (level and scripts).
pub fn load_level(levels_dir: &Path, id: &str) -> Result<LevelBundle> {
    let dir = levels_dir.join(id);
    let bundle =
        LevelBundle::load(&dir).with_context(|| format!("cannot load level from {dir:?}"))?;
    let errors = bundle.validate();
    if !errors.is_empty() {
        let list: Vec<String> = errors.iter().map(ToString::to_string).collect();
        bail!("level `{id}` is invalid:\n  {}", list.join("\n  "));
    }
    Ok(bundle)
}

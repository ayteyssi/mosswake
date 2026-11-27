//! Rhai scripting integration: sandbox, hooks and gameplay API.
//!
//! [`ScriptHost`] compiles a level's `scripts/*.rhai` in a sandboxed engine (no file,
//! network, time or `eval`; bounded operations, recursion and data sizes), keeps a persistent
//! `state` object, and calls optional hooks (`on_level_start`, `on_tick`…). Scripts do not
//! touch the simulation: they emit [`ScriptCommand`]s that the simulation applies (#19).
//! Errors are [`ScriptError`]s with file, line and kind; nothing here panics on bad scripts.
//!
//! The functions exposed to scripts are documented in `docs/SCRIPTING_API.md`.

mod command;
mod error;
mod host;
mod world;

pub use command::{dynamic_to_json, ScriptCommand, DEFAULT_HUD_SECONDS};
pub use error::{ScriptError, ScriptErrorKind};
pub use host::{HookOutcome, Limits, ScriptHost, RULES_VAR, STATE_VAR};
pub use world::{json_to_dynamic, Entity, EntityData};

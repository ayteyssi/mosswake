//! Shared types, constants, IDs, math and deterministic RNG.
//!
//! See `docs/ARCHITECTURE.md` for this crate's responsibilities and boundaries.

pub mod assets;
pub mod input;
pub mod tileset;

/// Crate name, used in logs.
pub const CRATE: &str = "mw_core";

#[cfg(test)]
mod tests {
    #[test]
    fn crate_name() {
        assert_eq!(super::CRATE, "mw_core");
    }
}

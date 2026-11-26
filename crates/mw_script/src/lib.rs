//! Rhai scripting integration: sandbox, hooks and gameplay API.
//!
//! See `docs/ARCHITECTURE.md` for this crate's responsibilities and boundaries.

/// Crate name, used in logs.
pub const CRATE: &str = "mw_script";

#[cfg(test)]
mod tests {
    #[test]
    fn crate_name() {
        assert_eq!(super::CRATE, "mw_script");
    }
}

//! Level file format (JSON), schema versioning and validation.
//!
//! See `docs/ARCHITECTURE.md` for this crate's responsibilities and boundaries.

/// Crate name, used in logs.
pub const CRATE: &str = "mw_level";

#[cfg(test)]
mod tests {
    #[test]
    fn crate_name() {
        assert_eq!(super::CRATE, "mw_level");
    }
}

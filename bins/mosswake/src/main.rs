//! Mosswake entry point.
//!
//! Planned subcommands (see docs/ARCHITECTURE.md):
//! `mosswake host [--level <id>]`, `mosswake join <addr>`, `mosswake edit <level>`,
//! `mosswake server --headless --level <id>`.

fn main() {
    println!(
        "mosswake {} — not playable yet, see GitHub issues",
        env!("CARGO_PKG_VERSION")
    );
}

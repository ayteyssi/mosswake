//! UDP networking: transport, protocol messages, LAN discovery.
//!
//! Built on `renet` (channels, reliability) over `renet_netcode` (UDP connections,
//! timeouts). Independent of the simulation: it moves [`mw_core::input::PlayerInput`]s and
//! opaque snapshots. See `docs/NETCODE.md`.
//!
//! - [`handshake`]: the frozen connection request / reply, readable by every version.
//! - [`protocol`]: versioned messages ([`PROTOCOL_VERSION`]) and channels.
//! - [`NetServer`] / [`NetClient`]: connection management on top.

use std::time::Duration;

mod client;
pub mod handshake;
pub mod protocol;
mod server;
pub mod transfer;

pub use client::{ClientState, NetClient};
pub use handshake::RejectReason;
pub use protocol::{
    Channel, ClientMessage, LeaveReason, PlayerInfo, ServerMessage, PROTOCOL_VERSION,
};
pub use server::{NetServer, NetServerConfig, ServerNetEvent};
pub use transfer::{Assembler, LevelCache, LevelHash, LevelOffer, TransferError};

/// Default game port (UDP).
pub const DEFAULT_PORT: u16 = 47800;

/// LAN discovery broadcast port (UDP), used by the discovery issue.
pub const DISCOVERY_PORT: u16 = 47801;

/// Netcode application id: identifies Mosswake traffic. **Never changes**: protocol versions
/// are negotiated by [`handshake`] so that mismatches get a clear refusal instead of silence.
pub const NETCODE_PROTOCOL_ID: u64 = u64::from_le_bytes(*b"MOSSWAKE");

/// Silence after which a connection is considered lost. Fixed by `renetcode` for unsecure
/// connections (the timeout of the connect token it generates); not configurable there.
pub const TIMEOUT: Duration = Duration::from_secs(15);

/// Transport-level failures.
#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("socket error: {0}")]
    Io(#[from] std::io::Error),
    #[error("netcode error: {0}")]
    Netcode(#[from] renet_netcode::NetcodeError),
    #[error("transport error: {0}")]
    Transport(#[from] renet_netcode::NetcodeTransportError),
    #[error("invalid player name")]
    InvalidName,
}

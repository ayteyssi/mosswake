//! Versioned game messages and channels. **Any change here bumps [`PROTOCOL_VERSION`].**
//!
//! Messages are encoded with `postcard` (compact varints). Each message knows its
//! [`Channel`], so callers never pick one by hand.

use mw_core::input::{PlayerId, PlayerInput};

use crate::transfer::LevelOffer;
use renet::{ChannelConfig, ConnectionConfig, SendType};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::Duration;

/// Version of the messages below. Checked at connection (see [`crate::handshake`]).
pub const PROTOCOL_VERSION: u32 = 2;

/// Delivery class of a message. Channel ids are the same in both directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Channel {
    /// Unreliable, unordered: inputs (client → server) and snapshots (server → client).
    /// Losing one is fine, the next one supersedes it.
    Unreliable = 0,
    /// Reliable and ordered: handshake reply, session events, control.
    Reliable = 1,
}

impl From<Channel> for u8 {
    fn from(channel: Channel) -> u8 {
        channel as u8
    }
}

/// Messages from a client to the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Inputs of the last ticks, oldest first, `inputs[i]` being for tick `first_tick + i`.
    /// Sent every tick with some redundancy so a lost packet costs nothing.
    Inputs {
        first_tick: u64,
        inputs: Vec<PlayerInput>,
    },
    /// Answer to [`ServerMessage::LevelOffer`] when the level is not cached: send the chunks.
    LevelRequest { transfer: u32 },
    /// The offered level is loaded: the server can spawn our player.
    LevelReady { transfer: u32 },
}

/// Messages from the server to clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ServerMessage {
    /// Every connected player, sent to a client right after it is accepted.
    Roster(Vec<PlayerInfo>),
    /// Another player joined.
    PlayerJoined(PlayerInfo),
    /// A player left the session.
    PlayerLeft {
        player: PlayerId,
        reason: LeaveReason,
    },
    /// World state at `tick`: `data` is a `mw_sim::Snapshot` encoded with [`encode`] (this
    /// crate does not depend on the simulation). See docs/NETCODE.md.
    Snapshot { tick: u64, data: Vec<u8> },
    /// The level to play (on connection and on every level change). The client answers
    /// `LevelRequest` to download it or, if cached, `LevelReady`. See [`crate::transfer`].
    LevelOffer(LevelOffer),
    /// Part `index` of the offered level package.
    LevelChunk {
        transfer: u32,
        index: u32,
        data: Vec<u8>,
    },
}

/// A connected player.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerInfo {
    pub player: PlayerId,
    pub name: String,
}

/// Why a player left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LeaveReason {
    /// Disconnected on purpose.
    Left,
    /// No packet received for too long.
    TimedOut,
    /// Removed by the server (kick, protocol error).
    Kicked,
}

impl ClientMessage {
    pub fn channel(&self) -> Channel {
        match self {
            Self::Inputs { .. } => Channel::Unreliable,
            Self::LevelRequest { .. } | Self::LevelReady { .. } => Channel::Reliable,
        }
    }
}

impl ServerMessage {
    pub fn channel(&self) -> Channel {
        match self {
            Self::Snapshot { .. } => Channel::Unreliable,
            Self::Roster(_)
            | Self::PlayerJoined(_)
            | Self::PlayerLeft { .. }
            | Self::LevelOffer(_)
            | Self::LevelChunk { .. } => Channel::Reliable,
        }
    }
}

/// Encodes a message (infallible for our types: no maps with non-string keys, no I/O).
pub fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(message)
}

/// Decodes a message, rejecting trailing bytes.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, postcard::Error> {
    match postcard::take_from_bytes(bytes)? {
        (message, []) => Ok(message),
        _ => Err(postcard::Error::DeserializeBadEncoding),
    }
}

/// Channel setup shared by server and client (both must agree).
pub fn connection_config() -> ConnectionConfig {
    let channels = || {
        vec![
            ChannelConfig {
                channel_id: Channel::Unreliable.into(),
                max_memory_usage_bytes: 5 * 1024 * 1024,
                send_type: SendType::Unreliable,
            },
            ChannelConfig {
                channel_id: Channel::Reliable.into(),
                max_memory_usage_bytes: 5 * 1024 * 1024,
                send_type: SendType::ReliableOrdered {
                    resend_time: Duration::from_millis(200),
                },
            },
        ]
    };
    ConnectionConfig {
        available_bytes_per_tick: 60_000,
        server_channels_config: channels(),
        client_channels_config: channels(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_roundtrip() {
        let messages = [
            ServerMessage::Roster(vec![PlayerInfo {
                player: PlayerId(0),
                name: "Ayla".into(),
            }]),
            ServerMessage::PlayerLeft {
                player: PlayerId(1),
                reason: LeaveReason::TimedOut,
            },
            ServerMessage::Snapshot {
                tick: 1234,
                data: vec![1, 2, 3],
            },
        ];
        for m in messages {
            assert_eq!(decode::<ServerMessage>(&encode(&m).unwrap()).unwrap(), m);
        }
    }

    #[test]
    fn inputs_are_compact() {
        let m = ClientMessage::Inputs {
            first_tick: 1000,
            inputs: vec![
                PlayerInput {
                    axis_x: 127,
                    axis_y: 0,
                    buttons: 1,
                };
                4
            ],
        };
        let bytes = encode(&m).unwrap();
        // variant + varint tick (2 bytes) + length + 4 x 3 bytes.
        assert_eq!(bytes.len(), 1 + 2 + 1 + 12, "{bytes:?}");
        assert_eq!(decode::<ClientMessage>(&bytes).unwrap(), m);
    }

    #[test]
    fn garbage_and_trailing_bytes_are_rejected() {
        assert!(decode::<ServerMessage>(&[200, 1, 2]).is_err());
        let mut bytes = encode(&ServerMessage::PlayerJoined(PlayerInfo {
            player: PlayerId(2),
            name: "x".into(),
        }))
        .unwrap();
        bytes.push(0);
        assert!(decode::<ServerMessage>(&bytes).is_err());
    }

    #[test]
    fn channels() {
        let inputs = ClientMessage::Inputs {
            first_tick: 0,
            inputs: vec![],
        };
        assert_eq!(inputs.channel(), Channel::Unreliable);
        let snap = ServerMessage::Snapshot {
            tick: 0,
            data: vec![],
        };
        assert_eq!(snap.channel(), Channel::Unreliable);
        assert_eq!(ServerMessage::Roster(vec![]).channel(), Channel::Reliable);
    }
}

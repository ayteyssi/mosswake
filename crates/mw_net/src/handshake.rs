//! The **frozen** part of the protocol: connection request and the server's first reply.
//!
//! Everything else is versioned by [`crate::PROTOCOL_VERSION`], but these formats must stay
//! readable by every past and future build, otherwise an outdated client could not even be
//! told *why* it is refused. Rules: never change or reorder existing fields or variants;
//! only append new enum variants at the end (an old build decodes them as "unknown").

use mw_core::input::PlayerId;
use serde::{Deserialize, Serialize};

/// Size of netcode's connection user data.
pub const USER_DATA_BYTES: usize = 256;

/// Longest accepted player name, in bytes of UTF-8.
pub const MAX_NAME_BYTES: usize = 24;

/// Sent by the client inside netcode's connection `user_data`:
/// `[0..4]` protocol version (u32 LE), `[4]` name length, `[5..]` UTF-8 name, rest zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectRequest {
    pub protocol_version: u32,
    pub name: String,
}

/// Why the server refused a client. Append-only (see module docs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RejectReason {
    /// Client and server speak different protocol versions.
    VersionMismatch { server: u32, client: u32 },
    /// The connection request could not be read (corrupted or foreign client).
    MalformedRequest,
    /// Empty or too long name, or one with control characters.
    InvalidName,
    /// Every player slot is taken.
    ServerFull,
}

/// First message from the server on the reliable channel. Append-only (see module docs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandshakeReply {
    Accepted { player: PlayerId },
    Rejected(RejectReason),
}

impl ConnectRequest {
    /// Encodes into netcode user data. Fails if the name does not fit.
    pub fn encode(&self) -> Option<[u8; USER_DATA_BYTES]> {
        let name = self.name.as_bytes();
        if name.len() > MAX_NAME_BYTES {
            return None;
        }
        let mut data = [0u8; USER_DATA_BYTES];
        data[..4].copy_from_slice(&self.protocol_version.to_le_bytes());
        data[4] = name.len() as u8;
        data[5..5 + name.len()].copy_from_slice(name);
        Some(data)
    }

    /// Decodes netcode user data. The version is read even if the name is unreadable, so
    /// a version mismatch can still be reported precisely.
    pub fn decode(data: &[u8; USER_DATA_BYTES]) -> (u32, Option<String>) {
        let version = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let len = usize::from(data[4]);
        let name = (len <= MAX_NAME_BYTES)
            .then(|| std::str::from_utf8(&data[5..5 + len]).ok())
            .flatten()
            .map(str::to_owned);
        (version, name)
    }
}

/// Whether `name` is acceptable as a player name.
pub fn is_valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= MAX_NAME_BYTES && !name.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_request_roundtrip() {
        let req = ConnectRequest {
            protocol_version: 7,
            name: "Mélusine".into(),
        };
        let data = req.encode().unwrap();
        assert_eq!(ConnectRequest::decode(&data), (7, Some("Mélusine".into())));
    }

    #[test]
    fn connect_request_layout_is_frozen() {
        let data = ConnectRequest {
            protocol_version: 0x0102_0304,
            name: "ab".into(),
        }
        .encode()
        .unwrap();
        assert_eq!(&data[..7], &[4, 3, 2, 1, 2, b'a', b'b']);
        assert!(data[7..].iter().all(|&b| b == 0));
    }

    #[test]
    fn too_long_or_corrupted_names() {
        let long = ConnectRequest {
            protocol_version: 1,
            name: "x".repeat(MAX_NAME_BYTES + 1),
        };
        assert!(long.encode().is_none());
        let mut data = [0u8; USER_DATA_BYTES];
        data[0] = 9;
        data[4] = 200; // length beyond the limit
        assert_eq!(ConnectRequest::decode(&data), (9, None));
        data[4] = 1;
        data[5] = 0xFF; // invalid UTF-8
        assert_eq!(ConnectRequest::decode(&data), (9, None));
    }

    #[test]
    fn name_rules() {
        assert!(is_valid_name("Ayla"));
        assert!(!is_valid_name("   "));
        assert!(!is_valid_name("a\nb"));
        assert!(!is_valid_name(&"x".repeat(MAX_NAME_BYTES + 1)));
    }

    /// The encoded bytes of the handshake reply must never change (see module docs).
    #[test]
    fn handshake_reply_encoding_is_frozen() {
        let accepted = postcard::to_allocvec(&HandshakeReply::Accepted {
            player: PlayerId(3),
        })
        .unwrap();
        assert_eq!(accepted, [0, 3]);
        let rejected =
            postcard::to_allocvec(&HandshakeReply::Rejected(RejectReason::VersionMismatch {
                server: 2,
                client: 1,
            }))
            .unwrap();
        assert_eq!(rejected, [1, 0, 2, 1]);
        let full =
            postcard::to_allocvec(&HandshakeReply::Rejected(RejectReason::ServerFull)).unwrap();
        assert_eq!(full, [1, 3]);
    }
}

//! LAN discovery: servers broadcast a small [`Announcement`] on [`crate::DISCOVERY_PORT`]
//! every second; a [`Browser`] collects them into a list of joinable games.
//!
//! The datagram format is **frozen** like [`crate::handshake`] (`b"MWLN"` + postcard of
//! [`Announcement`], append-only): any build can list any server and tell whether it is
//! compatible, instead of silently not seeing it.

use std::collections::BTreeMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{DISCOVERY_PORT, PROTOCOL_VERSION};

/// First bytes of every discovery datagram.
pub const MAGIC: [u8; 4] = *b"MWLN";

/// Time between two announcements of a server.
pub const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(1);

/// A game not heard from for this long disappears from the list.
pub const GAME_EXPIRY: Duration = Duration::from_secs(3);

/// Largest datagram accepted (announcements are tiny; anything bigger is not ours).
const MAX_DATAGRAM: usize = 512;

/// Longest name or level id kept from an announcement, in bytes.
pub const MAX_TEXT: usize = 48;

/// What a server tells the LAN about itself. Append-only (see module docs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Announcement {
    /// First so that every build can read it.
    pub protocol_version: u32,
    /// UDP port of the game (the address is the sender's).
    pub game_port: u16,
    /// Game name shown in lists.
    pub name: String,
    /// Level id being played.
    pub level: String,
    pub players: u8,
    pub max_players: u8,
}

/// Makes `text` announceable: control characters removed, cut to [`MAX_TEXT`] bytes on a
/// character boundary. Announcers must use it: browsers drop anything longer.
pub fn announceable(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().filter(|c| !c.is_control()) {
        if out.len() + c.len_utf8() > MAX_TEXT {
            break;
        }
        out.push(c);
    }
    out
}

impl Announcement {
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        // Infallible for this type (strings and integers only).
        bytes.extend(postcard::to_allocvec(self).unwrap_or_default());
        bytes
    }

    /// Decodes a datagram; `None` for anything that is not a valid announcement. Trailing
    /// bytes are allowed so that newer builds can append fields.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_DATAGRAM {
            return None;
        }
        let payload = bytes.strip_prefix(&MAGIC)?;
        let (mut announcement, _rest): (Self, _) = postcard::take_from_bytes(payload).ok()?;
        for text in [&mut announcement.name, &mut announcement.level] {
            if text.len() > MAX_TEXT || text.chars().any(char::is_control) {
                return None;
            }
        }
        Some(announcement)
    }
}

/// Sends a server's announcement every [`ANNOUNCE_INTERVAL`].
pub struct Announcer {
    socket: UdpSocket,
    targets: Vec<SocketAddr>,
    /// Time until the next announcement.
    wait: Duration,
}

impl Announcer {
    /// Announces to the LAN broadcast address on [`DISCOVERY_PORT`].
    pub fn lan() -> io::Result<Self> {
        Self::to_targets(vec![SocketAddr::from((
            Ipv4Addr::BROADCAST,
            DISCOVERY_PORT,
        ))])
    }

    /// Announces to explicit addresses (tests, or networks where broadcast is filtered).
    pub fn to_targets(targets: Vec<SocketAddr>) -> io::Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        socket.set_broadcast(true)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            targets,
            wait: Duration::ZERO,
        })
    }

    /// Advances by `dt`; sends `announcement` when it is time (immediately the first time).
    /// Send errors (no network, broadcast refused) are returned but harmless to ignore.
    pub fn update(&mut self, dt: Duration, announcement: &Announcement) -> io::Result<()> {
        self.wait = self.wait.saturating_sub(dt);
        if !self.wait.is_zero() {
            return Ok(());
        }
        self.wait = ANNOUNCE_INTERVAL;
        let bytes = announcement.encode();
        let mut result = Ok(());
        for target in &self.targets {
            if let Err(e) = self.socket.send_to(&bytes, target) {
                result = Err(e);
            }
        }
        result
    }
}

/// A game seen on the LAN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredGame {
    /// Where to connect: the announcer's IP and the announced game port.
    pub addr: SocketAddr,
    pub announcement: Announcement,
    /// Time since its last announcement.
    pub age: Duration,
}

impl DiscoveredGame {
    /// Whether this build can join it.
    pub fn compatible(&self) -> bool {
        self.announcement.protocol_version == PROTOCOL_VERSION
    }

    pub fn full(&self) -> bool {
        self.announcement.players >= self.announcement.max_players
    }
}

/// Listens for announcements and keeps the list of live games.
pub struct Browser {
    socket: UdpSocket,
    games: BTreeMap<SocketAddr, DiscoveredGame>,
}

impl Browser {
    /// Listens on [`DISCOVERY_PORT`].
    pub fn lan() -> io::Result<Self> {
        Self::bind(DISCOVERY_PORT)
    }

    /// Listens on `port` (0 = any free port, for tests).
    pub fn bind(port: u16) -> io::Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port))?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            games: BTreeMap::new(),
        })
    }

    pub fn local_port(&self) -> io::Result<u16> {
        Ok(self.socket.local_addr()?.port())
    }

    /// Reads every pending announcement, ages the others by `dt` and forgets games silent
    /// for [`GAME_EXPIRY`]. Unreadable datagrams are ignored.
    pub fn update(&mut self, dt: Duration) -> io::Result<()> {
        for game in self.games.values_mut() {
            game.age += dt;
        }
        let mut buffer = [0u8; MAX_DATAGRAM + 1];
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((len, from)) => {
                    if let Some(announcement) = Announcement::decode(&buffer[..len]) {
                        let addr = SocketAddr::new(from.ip(), announcement.game_port);
                        self.games.insert(
                            addr,
                            DiscoveredGame {
                                addr,
                                announcement,
                                age: Duration::ZERO,
                            },
                        );
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                // ICMP "port unreachable" echoes and the like: not fatal for a listener.
                Err(e) if e.kind() == io::ErrorKind::ConnectionReset => continue,
                Err(e) => return Err(e),
            }
        }
        self.games.retain(|_, game| game.age < GAME_EXPIRY);
        Ok(())
    }

    /// Live games, ordered by address.
    pub fn games(&self) -> Vec<&DiscoveredGame> {
        self.games.values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn announcement() -> Announcement {
        Announcement {
            protocol_version: 7,
            game_port: 47800,
            name: "Ayla".into(),
            level: "tutorial".into(),
            players: 2,
            max_players: 4,
        }
    }

    #[test]
    fn roundtrip() {
        let a = announcement();
        assert_eq!(Announcement::decode(&a.encode()), Some(a));
    }

    #[test]
    fn layout_is_frozen() {
        let bytes = announcement().encode();
        // magic, version 7, port 47800 (varint), "Ayla", "tutorial", 2, 4
        let mut expected = b"MWLN".to_vec();
        expected.extend([7, 0xB8, 0xF5, 0x02, 4]);
        expected.extend(b"Ayla");
        expected.push(8);
        expected.extend(b"tutorial");
        expected.extend([2, 4]);
        assert_eq!(bytes, expected);
    }

    #[test]
    fn newer_builds_may_append_fields() {
        let mut bytes = announcement().encode();
        bytes.extend([1, 2, 3]);
        assert_eq!(Announcement::decode(&bytes), Some(announcement()));
    }

    #[test]
    fn announceable_text_always_decodes() {
        let long = "La partie du samedi soir chez Mélusine et Ayla !";
        assert!(long.len() > MAX_TEXT);
        let name = announceable(long);
        assert!(name.len() <= MAX_TEXT && long.starts_with(&name));
        assert_eq!(announceable("a\tb\nc"), "abc");
        // Multi-byte characters are never split.
        let accents = announceable(&"é".repeat(40));
        assert_eq!(accents, "é".repeat(MAX_TEXT / 2));
        let a = Announcement {
            name,
            level: announceable(&"x".repeat(100)),
            ..announcement()
        };
        assert_eq!(Announcement::decode(&a.encode()), Some(a));
    }

    #[test]
    fn foreign_or_abusive_datagrams_are_ignored() {
        assert_eq!(Announcement::decode(b"hello"), None);
        assert_eq!(Announcement::decode(b"MWLN"), None);
        let mut big = announcement();
        big.name = "x".repeat(MAX_TEXT + 1);
        assert_eq!(Announcement::decode(&big.encode()), None);
        let mut sneaky = announcement();
        sneaky.level = "tuto\u{7}rial".into();
        assert_eq!(Announcement::decode(&sneaky.encode()), None);
        assert_eq!(Announcement::decode(&vec![0; MAX_DATAGRAM + 1]), None);
    }
}

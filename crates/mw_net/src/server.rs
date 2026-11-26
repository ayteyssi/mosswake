//! [`NetServer`]: the authoritative side of a session.

use std::collections::{BTreeMap, VecDeque};
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mw_core::input::PlayerId;
use renet::{ClientId, RenetServer, ServerEvent};
use renet_netcode::{NetcodeServerTransport, ServerAuthentication, ServerConfig};

use crate::handshake::{is_valid_name, ConnectRequest, HandshakeReply, RejectReason};
use crate::protocol::{
    connection_config, decode, encode, Channel, ClientMessage, LeaveReason, PlayerInfo,
    ServerMessage, PROTOCOL_VERSION,
};
use crate::{NetError, NETCODE_PROTOCOL_ID, TIMEOUT};

/// How long a refused client stays connected so it can receive the reason.
const REJECT_GRACE: Duration = Duration::from_millis(500);

/// Transport connections allowed beyond the player slots, so that clients arriving when the
/// session is full still complete the transport handshake and get an explicit
/// [`RejectReason::ServerFull`] instead of netcode's silent denial.
const REJECT_HEADROOM: usize = 8;

/// Server settings.
#[derive(Debug, Clone)]
pub struct NetServerConfig {
    /// Address to listen on, e.g. `0.0.0.0:47800` (port 0 = any free port).
    pub bind: SocketAddr,
    /// Player slots (1..=64).
    pub max_players: u8,
}

/// Something that happened to the session during [`NetServer::update`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerNetEvent {
    PlayerConnected(PlayerInfo),
    PlayerDisconnected {
        player: PlayerId,
        reason: LeaveReason,
    },
    /// A client was refused before becoming a player.
    ClientRejected {
        addr: Option<SocketAddr>,
        reason: RejectReason,
    },
}

struct Pending {
    client: ClientId,
    remaining: Duration,
}

/// UDP server: accepts players, relays typed messages, reports joins and leaves.
///
/// Call [`NetServer::update`] once per frame with the elapsed time, read
/// [`NetServer::events`] and [`NetServer::receive`], send, then [`NetServer::flush`].
pub struct NetServer {
    server: RenetServer,
    transport: NetcodeServerTransport,
    max_players: u8,
    players: BTreeMap<ClientId, PlayerInfo>,
    rejected: Vec<Pending>,
    /// Silence per client before the last update, to tell timeouts from clean leaves.
    silence: BTreeMap<ClientId, Duration>,
    events: VecDeque<ServerNetEvent>,
}

impl NetServer {
    pub fn bind(config: NetServerConfig) -> Result<Self, NetError> {
        let socket = UdpSocket::bind(config.bind)?;
        let addr = socket.local_addr()?;
        let max_players = config.max_players.clamp(1, 64);
        let transport = NetcodeServerTransport::new(
            ServerConfig {
                current_time: unix_time(),
                max_clients: usize::from(max_players) + REJECT_HEADROOM,
                protocol_id: NETCODE_PROTOCOL_ID,
                public_addresses: vec![addr],
                // LAN game on a trusted network: no connect tokens or encryption keys.
                authentication: ServerAuthentication::Unsecure,
            },
            socket,
        )?;
        Ok(Self {
            server: RenetServer::new(connection_config()),
            transport,
            max_players,
            players: BTreeMap::new(),
            rejected: Vec::new(),
            silence: BTreeMap::new(),
            events: VecDeque::new(),
        })
    }

    /// Address the server listens on (useful with port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.transport.addresses()[0]
    }

    /// Receives packets, processes connections and handshakes, advances timers by `dt`.
    pub fn update(&mut self, dt: Duration) -> Result<(), NetError> {
        for client in self.server.clients_id() {
            if let Some(silence) = self.transport.time_since_last_received_packet(client) {
                self.silence.insert(client, silence + dt);
            }
        }
        self.server.update(dt);
        self.transport.update(dt, &mut self.server)?;

        while let Some(event) = self.server.get_event() {
            match event {
                ServerEvent::ClientConnected { client_id } => self.on_connected(client_id),
                ServerEvent::ClientDisconnected { client_id, .. } => {
                    self.on_disconnected(client_id)
                }
            }
        }

        // Refused clients are dropped once they had time to receive the reason.
        let mut expired = Vec::new();
        self.rejected.retain_mut(|p| {
            p.remaining = p.remaining.saturating_sub(dt);
            let keep = !p.remaining.is_zero() && self.server.is_connected(p.client);
            if !keep {
                expired.push(p.client);
            }
            keep
        });
        for client in expired {
            self.server.disconnect(client);
        }
        Ok(())
    }

    fn on_connected(&mut self, client: ClientId) {
        let request = self
            .transport
            .user_data(client)
            .map(|d| ConnectRequest::decode(&d));
        let addr = self.transport.client_addr(client);
        let verdict = match request {
            None => Err(RejectReason::MalformedRequest),
            Some((version, _)) if version != PROTOCOL_VERSION => {
                Err(RejectReason::VersionMismatch {
                    server: PROTOCOL_VERSION,
                    client: version,
                })
            }
            Some((_, Some(name))) if is_valid_name(&name) => match self.free_slot() {
                Some(player) => Ok(PlayerInfo {
                    player,
                    name: name.trim().to_owned(),
                }),
                None => Err(RejectReason::ServerFull),
            },
            Some(_) => Err(RejectReason::InvalidName),
        };
        match verdict {
            Ok(info) => {
                self.send_raw(
                    client,
                    &HandshakeReply::Accepted {
                        player: info.player,
                    },
                );
                for other in self.players.keys().copied().collect::<Vec<_>>() {
                    self.send_to_client(other, &ServerMessage::PlayerJoined(info.clone()));
                }
                self.players.insert(client, info.clone());
                let roster = self.players(); // sorted by player id, not by random client id
                self.send_to_client(client, &ServerMessage::Roster(roster));
                self.events.push_back(ServerNetEvent::PlayerConnected(info));
            }
            Err(reason) => {
                self.send_raw(client, &HandshakeReply::Rejected(reason.clone()));
                self.rejected.push(Pending {
                    client,
                    remaining: REJECT_GRACE,
                });
                self.events
                    .push_back(ServerNetEvent::ClientRejected { addr, reason });
            }
        }
    }

    fn on_disconnected(&mut self, client: ClientId) {
        let silence = self.silence.remove(&client).unwrap_or_default();
        let Some(info) = self.players.remove(&client) else {
            return; // a refused client
        };
        let reason = if silence >= TIMEOUT {
            LeaveReason::TimedOut
        } else {
            LeaveReason::Left
        };
        self.notify_left(info.player, reason);
    }

    fn notify_left(&mut self, player: PlayerId, reason: LeaveReason) {
        self.broadcast(&ServerMessage::PlayerLeft { player, reason });
        self.events
            .push_back(ServerNetEvent::PlayerDisconnected { player, reason });
    }

    /// Lowest player id not in use.
    fn free_slot(&self) -> Option<PlayerId> {
        (0..self.max_players)
            .map(PlayerId)
            .find(|id| self.players.values().all(|p| p.player != *id))
    }

    /// Session events since the last call, in order.
    pub fn events(&mut self) -> impl Iterator<Item = ServerNetEvent> + '_ {
        self.events.drain(..)
    }

    /// Messages received from players, in order per player and channel. A client sending
    /// something undecodable is kicked (protocol error).
    pub fn receive(&mut self) -> Vec<(PlayerId, ClientMessage)> {
        let mut received = Vec::new();
        let mut faulty = Vec::new();
        for (&client, info) in &self.players {
            for channel in [Channel::Reliable, Channel::Unreliable] {
                while let Some(bytes) = self.server.receive_message(client, channel) {
                    match decode::<ClientMessage>(&bytes) {
                        Ok(message) => received.push((info.player, message)),
                        Err(_) => faulty.push(client),
                    }
                }
            }
        }
        faulty.dedup();
        for client in faulty {
            self.kick_client(client);
        }
        received
    }

    /// Connected players, by id.
    pub fn players(&self) -> Vec<PlayerInfo> {
        let mut players: Vec<_> = self.players.values().cloned().collect();
        players.sort_by_key(|p| p.player);
        players
    }

    /// Sends `message` to `player` (ignored if not connected).
    pub fn send(&mut self, player: PlayerId, message: &ServerMessage) {
        if let Some(client) = self.client_of(player) {
            self.send_to_client(client, message);
        }
    }

    /// Sends `message` to every player.
    pub fn broadcast(&mut self, message: &ServerMessage) {
        for client in self.players.keys().copied().collect::<Vec<_>>() {
            self.send_to_client(client, message);
        }
    }

    /// Disconnects `player`; the others are told with [`LeaveReason::Kicked`].
    pub fn kick(&mut self, player: PlayerId) {
        if let Some(client) = self.client_of(player) {
            self.kick_client(client);
        }
    }

    fn kick_client(&mut self, client: ClientId) {
        if let Some(info) = self.players.remove(&client) {
            self.server.disconnect(client);
            self.notify_left(info.player, LeaveReason::Kicked);
        }
    }

    /// Sends every queued packet. Call once per frame after sending messages.
    pub fn flush(&mut self) {
        self.transport.send_packets(&mut self.server);
    }

    /// Disconnects everyone (server shutdown).
    pub fn shutdown(&mut self) {
        self.transport.disconnect_all(&mut self.server);
    }

    fn client_of(&self, player: PlayerId) -> Option<ClientId> {
        self.players
            .iter()
            .find(|(_, info)| info.player == player)
            .map(|(&client, _)| client)
    }

    fn send_to_client(&mut self, client: ClientId, message: &ServerMessage) {
        match encode(message) {
            Ok(bytes) => self.server.send_message(client, message.channel(), bytes),
            Err(e) => debug_assert!(false, "unencodable message {message:?}: {e}"),
        }
    }

    fn send_raw(&mut self, client: ClientId, reply: &HandshakeReply) {
        match encode(reply) {
            Ok(bytes) => self.server.send_message(client, Channel::Reliable, bytes),
            Err(e) => debug_assert!(false, "unencodable handshake {reply:?}: {e}"),
        }
    }
}

/// Wall-clock time since the Unix epoch: netcode compares token expiry across machines.
pub(crate) fn unix_time() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

//! [`NetClient`]: a player's connection to a [`crate::NetServer`].

use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use mw_core::input::PlayerId;
use renet::RenetClient;
use renet_netcode::{ClientAuthentication, NetcodeClientTransport};

use crate::handshake::{is_valid_name, ConnectRequest, HandshakeReply, RejectReason};
use crate::protocol::{
    connection_config, decode, encode, Channel, ClientMessage, ServerMessage, PROTOCOL_VERSION,
};
use crate::server::unix_time;
use crate::{NetError, NETCODE_PROTOCOL_ID};

/// Where the connection stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientState {
    /// Handshake in progress.
    Connecting,
    /// Accepted as `player`.
    Connected { player: PlayerId },
    /// The server refused us, with a reason.
    Rejected(RejectReason),
    /// Connection lost or closed (timeout, server gone, protocol error, or our own call).
    Disconnected(String),
}

/// UDP client. Call [`NetClient::update`] every frame, read [`NetClient::receive`],
/// send, then [`NetClient::flush`].
pub struct NetClient {
    client: RenetClient,
    transport: NetcodeClientTransport,
    state: ClientState,
    inbox: VecDeque<ServerMessage>,
}

impl NetClient {
    /// Starts connecting to `server` as `name`. Completes during later updates.
    pub fn connect(server: SocketAddr, name: &str) -> Result<Self, NetError> {
        Self::connect_with_version(server, name, PROTOCOL_VERSION)
    }

    /// Like [`NetClient::connect`] but announcing another protocol version (tests only:
    /// simulates an outdated build).
    #[doc(hidden)]
    pub fn connect_with_version(
        server: SocketAddr,
        name: &str,
        protocol_version: u32,
    ) -> Result<Self, NetError> {
        if !is_valid_name(name) {
            return Err(NetError::InvalidName);
        }
        let user_data = ConnectRequest {
            protocol_version,
            name: name.to_owned(),
        }
        .encode()
        .ok_or(NetError::InvalidName)?;
        let bind: SocketAddr = if server.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        }
        .parse()
        .expect("valid literal address");
        let socket = UdpSocket::bind(bind)?;
        let now = unix_time();
        let transport = NetcodeClientTransport::new(
            now,
            ClientAuthentication::Unsecure {
                protocol_id: NETCODE_PROTOCOL_ID,
                client_id: random_client_id(now, &socket),
                server_addr: server,
                user_data: Some(user_data),
            },
            socket,
        )?;
        Ok(Self {
            client: RenetClient::new(connection_config()),
            transport,
            state: ClientState::Connecting,
            inbox: VecDeque::new(),
        })
    }

    pub fn state(&self) -> &ClientState {
        &self.state
    }

    /// Our player id once accepted.
    pub fn player(&self) -> Option<PlayerId> {
        match self.state {
            ClientState::Connected { player } => Some(player),
            _ => None,
        }
    }

    /// Receives packets, completes the handshake, advances timers by `dt`.
    pub fn update(&mut self, dt: Duration) -> Result<(), NetError> {
        if matches!(
            self.state,
            ClientState::Rejected(_) | ClientState::Disconnected(_)
        ) {
            return Ok(());
        }
        self.client.update(dt);
        if let Err(e) = self.transport.update(dt, &mut self.client) {
            self.state = ClientState::Disconnected(e.to_string());
            return Ok(());
        }
        while let Some(bytes) = self.client.receive_message(Channel::Reliable) {
            self.on_reliable(&bytes);
        }
        while let Some(bytes) = self.client.receive_message(Channel::Unreliable) {
            if let Ok(message) = decode::<ServerMessage>(&bytes) {
                if matches!(self.state, ClientState::Connected { .. }) {
                    self.inbox.push_back(message);
                }
            }
        }
        if self.client.is_disconnected()
            && matches!(
                self.state,
                ClientState::Connecting | ClientState::Connected { .. }
            )
        {
            let reason = self
                .transport
                .disconnect_reason()
                .map(|r| format!("{r:?}"))
                .or_else(|| self.client.disconnect_reason().map(|r| format!("{r:?}")))
                .unwrap_or_else(|| "unknown".into());
            self.state = ClientState::Disconnected(reason);
        }
        Ok(())
    }

    fn on_reliable(&mut self, bytes: &[u8]) {
        match self.state {
            ClientState::Connecting => match decode::<HandshakeReply>(bytes) {
                Ok(HandshakeReply::Accepted { player }) => {
                    self.state = ClientState::Connected { player }
                }
                Ok(HandshakeReply::Rejected(reason)) => {
                    self.state = ClientState::Rejected(reason);
                    self.transport.disconnect();
                }
                // A reply variant from a newer build: we cannot talk to this server.
                Err(_) => self.fail("unreadable handshake reply (newer server?)"),
            },
            ClientState::Connected { .. } => match decode::<ServerMessage>(bytes) {
                Ok(message) => self.inbox.push_back(message),
                Err(_) => self.fail("protocol error: undecodable server message"),
            },
            _ => {}
        }
    }

    fn fail(&mut self, reason: &str) {
        self.transport.disconnect();
        self.state = ClientState::Disconnected(reason.into());
    }

    /// Messages received since the last call, in arrival order.
    pub fn receive(&mut self) -> Vec<ServerMessage> {
        self.inbox.drain(..).collect()
    }

    /// Queues `message` (dropped unless connected).
    pub fn send(&mut self, message: &ClientMessage) {
        if !matches!(self.state, ClientState::Connected { .. }) {
            return;
        }
        if let Ok(bytes) = encode(message) {
            self.client.send_message(message.channel(), bytes);
        }
    }

    /// Sends every queued packet. Call once per frame.
    pub fn flush(&mut self) -> Result<(), NetError> {
        if matches!(
            self.state,
            ClientState::Rejected(_) | ClientState::Disconnected(_)
        ) {
            return Ok(());
        }
        if let Err(e) = self.transport.send_packets(&mut self.client) {
            self.state = ClientState::Disconnected(e.to_string());
        }
        Ok(())
    }

    /// Leaves the session (the server sees [`crate::LeaveReason::Left`]).
    pub fn disconnect(&mut self) {
        self.transport.disconnect();
        self.state = ClientState::Disconnected("disconnected by client".into());
    }
}

/// Netcode needs a client id unique per server. Not simulation state, so mixing the clock
/// and the local port is fine (collisions are astronomically unlikely on a LAN).
fn random_client_id(now: Duration, socket: &UdpSocket) -> u64 {
    let port = socket.local_addr().map(|a| a.port()).unwrap_or_default();
    let mut x = now.as_nanos() as u64 ^ (u64::from(port) << 48) ^ u64::from(std::process::id());
    // splitmix64 finalizer to spread the bits.
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

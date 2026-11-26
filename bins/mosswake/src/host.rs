//! [`HostSession`]: host mode, a [`ServerSession`] and the host's own client in one process.
//!
//! The local client talks to the server through loopback UDP like any other client, so the
//! host gets no gameplay advantage and the code path is the one remote players use. Both
//! are pumped from the same loop: no thread, deterministic order.

use std::net::{Ipv4Addr, SocketAddr};

use anyhow::{Context, Result};
use mw_core::input::{PlayerId, PlayerInput};
use mw_level::Level;
use mw_net::{ClientState, NetClient, ServerMessage, ServerNetEvent};
use mw_sim::Snapshot;

use crate::input_sender::InputSender;
use crate::server::{decode_snapshot, ServerConfig, ServerSession, TICK};

/// A hosted session: the server and the local player's client.
pub struct HostSession {
    server: ServerSession,
    client: NetClient,
    sender: InputSender,
    latest: Option<Snapshot>,
}

impl HostSession {
    /// Starts the server on `config.bind` and connects the local player as `name`.
    pub fn new(level: &Level, config: ServerConfig, name: &str) -> Result<Self> {
        let server = ServerSession::new(level, config)?;
        let port = server.local_addr().port();
        let loopback = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let client = NetClient::connect(loopback, name).context("local client cannot start")?;
        Ok(Self {
            server,
            client,
            sender: InputSender::default(),
            latest: None,
        })
    }

    /// One tick: the server simulates, then the local client receives the result and sends
    /// `input` for the next tick. Returns the server's session events.
    pub fn frame(&mut self, input: PlayerInput) -> Result<Vec<ServerNetEvent>> {
        let events = self.server.tick()?;
        self.client
            .update(TICK)
            .context("local client update failed")?;
        for message in self.client.receive() {
            if let ServerMessage::Snapshot { data, .. } = message {
                let snapshot = decode_snapshot(&data)?;
                if self.latest.as_ref().is_none_or(|s| snapshot.tick > s.tick) {
                    self.sender.on_snapshot(snapshot.tick);
                    self.latest = Some(snapshot);
                }
            }
        }
        if let Some(message) = self.sender.next(input) {
            self.client.send(&message);
        }
        self.client.flush().context("local client send failed")?;
        Ok(events)
    }

    /// The local player's id once connected.
    pub fn local_player(&self) -> Option<PlayerId> {
        self.client.player()
    }

    pub fn client_state(&self) -> &ClientState {
        self.client.state()
    }

    /// Newest snapshot received by the local client (what it would draw).
    pub fn latest_snapshot(&self) -> Option<&Snapshot> {
        self.latest.as_ref()
    }

    pub fn server(&mut self) -> &mut ServerSession {
        &mut self.server
    }

    /// Leaves and stops the server.
    pub fn shutdown(&mut self) {
        self.client.disconnect();
        self.server.shutdown();
    }
}

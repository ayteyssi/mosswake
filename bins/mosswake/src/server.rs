//! [`ServerSession`]: the authoritative game server, the simulation driven by the network.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use mw_core::input::PlayerId;
use mw_level::Level;
use mw_net::{
    protocol, Announcement, Announcer, ClientMessage, NetServer, NetServerConfig, ServerMessage,
    ServerNetEvent,
};
use mw_sim::{SimHarness, Snapshot, TickInputs, TICK_HZ};

use crate::input_buffer::InputBuffer;

/// One simulation tick of wall-clock time.
pub const TICK: Duration = Duration::from_nanos((1e9 / TICK_HZ) as u64);

/// Ticks between two snapshots: every 2 ticks = 30 Hz.
pub const SNAPSHOT_INTERVAL_TICKS: u64 = 2;

/// Server settings.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Address to listen on (port 0 = any free port).
    pub bind: SocketAddr,
    pub max_players: u8,
}

/// Runs the only real simulation of a session: spawns a player per connection, applies
/// their inputs tick by tick and broadcasts [`Snapshot`]s.
///
/// Call [`ServerSession::tick`] once per simulation tick ([`TICK`]); the caller owns the
/// timing (real-time loop in the game, as fast as possible in tests).
pub struct ServerSession {
    net: NetServer,
    sim: SimHarness,
    inputs: BTreeMap<PlayerId, InputBuffer>,
    level_id: String,
    max_players: u8,
    /// LAN announcements, when enabled, and the announced game name.
    announcer: Option<(Announcer, String)>,
}

impl ServerSession {
    /// Loads `level` into a new simulation and starts listening.
    pub fn new(level: &Level, config: ServerConfig) -> Result<Self> {
        let sim = SimHarness::new(level)
            .with_context(|| format!("cannot simulate level `{}`", level.id))?;
        let net = NetServer::bind(NetServerConfig {
            bind: config.bind,
            max_players: config.max_players,
        })
        .with_context(|| format!("cannot listen on {}", config.bind))?;
        Ok(Self {
            net,
            sim,
            inputs: BTreeMap::new(),
            level_id: level.id.clone(),
            max_players: config.max_players,
            announcer: None,
        })
    }

    /// Announces this game as `name` on the LAN (broadcast on the discovery port).
    pub fn announce_on_lan(&mut self, name: impl Into<String>) -> Result<()> {
        let announcer = Announcer::lan().context("cannot open the LAN discovery socket")?;
        self.announcer = Some((announcer, name.into()));
        Ok(())
    }

    /// Announces this game to explicit addresses (tests, or LANs filtering broadcast).
    pub fn announce_to(&mut self, targets: Vec<SocketAddr>, name: impl Into<String>) -> Result<()> {
        let announcer =
            Announcer::to_targets(targets).context("cannot open the discovery socket")?;
        self.announcer = Some((announcer, name.into()));
        Ok(())
    }

    /// What the server would announce now.
    pub fn announcement(&self, name: &str) -> Announcement {
        Announcement {
            protocol_version: mw_net::PROTOCOL_VERSION,
            game_port: self.local_addr().port(),
            // Bounded like browsers expect, or the game would be silently invisible.
            name: mw_net::discovery::announceable(name),
            level: mw_net::discovery::announceable(&self.level_id),
            players: self.net.players().len() as u8,
            max_players: self.max_players,
        }
    }

    /// Address the server listens on.
    pub fn local_addr(&self) -> SocketAddr {
        self.net.local_addr()
    }

    /// One server tick: network in, joins and leaves, inputs, one simulation step, snapshot
    /// every [`SNAPSHOT_INTERVAL_TICKS`], network out. Returns this tick's session events.
    pub fn tick(&mut self) -> Result<Vec<ServerNetEvent>> {
        self.net.update(TICK).context("network update failed")?;
        let mut events = Vec::new();
        self.apply_session_events(&mut events);

        // `current` is the tick about to be simulated: inputs for it are used now.
        let current = self.sim.tick();
        for (player, message) in self.net.receive() {
            match message {
                ClientMessage::Inputs { first_tick, inputs } => {
                    if let Some(buffer) = self.inputs.get_mut(&player) {
                        buffer.insert(first_tick, &inputs, current);
                    }
                }
            }
        }
        // Receiving can kick a client (undecodable message): drop its body this tick.
        self.apply_session_events(&mut events);
        let tick_inputs = TickInputs(
            self.inputs
                .iter_mut()
                .map(|(player, buffer)| (*player, buffer.take(current)))
                .collect(),
        );
        self.sim.step(1, &tick_inputs);

        if self.sim.tick().is_multiple_of(SNAPSHOT_INTERVAL_TICKS) && !self.inputs.is_empty() {
            let snapshot = self.sim.snapshot();
            let data = protocol::encode(&snapshot).context("cannot encode snapshot")?;
            self.net.broadcast(&ServerMessage::Snapshot {
                tick: snapshot.tick,
                data,
            });
        }
        self.net.flush();
        if let Some((mut announcer, name)) = self.announcer.take() {
            // Best effort: a LAN without broadcast must not stop the game.
            let _ = announcer.update(TICK, &self.announcement(&name));
            self.announcer = Some((announcer, name));
        }
        Ok(events)
    }

    /// Spawns and despawns players for the pending session events, appended to `out`.
    fn apply_session_events(&mut self, out: &mut Vec<ServerNetEvent>) {
        for event in self.net.events().collect::<Vec<_>>() {
            match &event {
                ServerNetEvent::PlayerConnected(info) => {
                    self.sim.spawn_player(info.player);
                    self.inputs.insert(info.player, InputBuffer::default());
                }
                ServerNetEvent::PlayerDisconnected { player, .. } => {
                    self.sim.despawn_player(*player);
                    self.inputs.remove(player);
                }
                ServerNetEvent::ClientRejected { .. } => {}
            }
            out.push(event);
        }
    }

    /// The simulation (read state in tests and tools).
    pub fn sim(&mut self) -> &mut SimHarness {
        &mut self.sim
    }

    /// Connected players.
    pub fn players(&self) -> Vec<mw_net::PlayerInfo> {
        self.net.players()
    }

    /// Disconnects everyone.
    pub fn shutdown(&mut self) {
        self.net.shutdown();
    }
}

/// Decodes the payload of a [`ServerMessage::Snapshot`].
pub fn decode_snapshot(data: &[u8]) -> Result<Snapshot> {
    protocol::decode(data).context("invalid snapshot payload")
}

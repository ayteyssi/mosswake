//! [`ServerSession`]: the authoritative game server, the simulation driven by the network.

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use mw_core::input::PlayerId;
use mw_level::LevelBundle;
use mw_net::transfer::chunks;
use mw_net::{
    protocol, Announcement, Announcer, ClientMessage, LevelOffer, NetServer, NetServerConfig,
    ServerMessage, ServerNetEvent,
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

/// Runs the only real simulation of a session: sends the level to each connecting client,
/// spawns its player once the client has loaded it, applies inputs tick by tick and
/// broadcasts [`Snapshot`]s.
///
/// Call [`ServerSession::tick`] once per simulation tick ([`TICK`]); the caller owns the
/// timing (real-time loop in the game, as fast as possible in tests).
pub struct ServerSession {
    net: NetServer,
    sim: SimHarness,
    inputs: BTreeMap<PlayerId, InputBuffer>,
    /// The current level as sent to clients, and its offer.
    package: Vec<u8>,
    offer: LevelOffer,
    /// Connected players still loading the current level (not spawned yet).
    loading: BTreeSet<PlayerId>,
    /// Level packages sent (requests served), for tests and logs.
    uploads: u32,
    level_id: String,
    max_players: u8,
    /// LAN announcements, when enabled, and the announced game name.
    announcer: Option<(Announcer, String)>,
}

impl ServerSession {
    /// Loads `bundle` into a new simulation and starts listening.
    pub fn new(bundle: &LevelBundle, config: ServerConfig) -> Result<Self> {
        let (sim, package) = prepare(bundle)?;
        let net = NetServer::bind(NetServerConfig {
            bind: config.bind,
            max_players: config.max_players,
        })
        .with_context(|| format!("cannot listen on {}", config.bind))?;
        let offer = LevelOffer::new(1, &package);
        Ok(Self {
            net,
            sim,
            inputs: BTreeMap::new(),
            package,
            offer,
            loading: BTreeSet::new(),
            uploads: 0,
            level_id: bundle.level.id.clone(),
            max_players: config.max_players,
            announcer: None,
        })
    }

    /// Switches every client to `bundle`: a fresh simulation, and a new offer to everyone.
    /// Players respawn as their clients finish loading it.
    pub fn change_level(&mut self, bundle: &LevelBundle) -> Result<()> {
        let (sim, package) = prepare(bundle)?;
        self.sim = sim;
        self.level_id = bundle.level.id.clone();
        self.offer = LevelOffer::new(self.offer.transfer + 1, &package);
        self.package = package;
        self.inputs.clear();
        self.loading = self.net.players().iter().map(|p| p.player).collect();
        self.net.broadcast(&ServerMessage::LevelOffer(self.offer));
        self.net.flush();
        Ok(())
    }

    /// The offer of the current level.
    pub fn level_offer(&self) -> LevelOffer {
        self.offer
    }

    /// Level packages sent so far (clients with a cached copy download nothing).
    pub fn uploads(&self) -> u32 {
        self.uploads
    }

    fn on_level_message(&mut self, player: PlayerId, message: &ClientMessage) {
        match *message {
            ClientMessage::LevelRequest { transfer } if transfer == self.offer.transfer => {
                for (index, data) in chunks(&self.package).into_iter().enumerate() {
                    self.net.send(
                        player,
                        &ServerMessage::LevelChunk {
                            transfer,
                            index: index as u32,
                            data: data.to_vec(),
                        },
                    );
                }
                self.uploads += 1;
            }
            ClientMessage::LevelReady { transfer }
                if transfer == self.offer.transfer && self.loading.remove(&player) =>
            {
                self.sim.spawn_player(player);
                self.inputs.insert(player, InputBuffer::default());
            }
            // Stale transfer numbers (an older level) and duplicates are ignored.
            _ => {}
        }
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
                level @ (ClientMessage::LevelRequest { .. } | ClientMessage::LevelReady { .. }) => {
                    self.on_level_message(player, &level)
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
                    // Spawned once the client has the level (`LevelReady`).
                    self.loading.insert(info.player);
                    self.net
                        .send(info.player, &ServerMessage::LevelOffer(self.offer));
                }
                ServerNetEvent::PlayerDisconnected { player, .. } => {
                    if self.inputs.remove(player).is_some() {
                        self.sim.despawn_player(*player);
                    }
                    self.loading.remove(player);
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

    /// Script output, errors and refused commands since the last call.
    pub fn take_script_log(&mut self) -> mw_sim::scripting::ScriptLog {
        self.sim
            .world_mut()
            .resource_mut::<mw_sim::scripting::ScriptLog>()
            .take()
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

/// A simulation of `bundle` and its package, refusing levels too large to send.
fn prepare(bundle: &LevelBundle) -> Result<(SimHarness, Vec<u8>)> {
    let id = &bundle.level.id;
    let sim = SimHarness::from_bundle(bundle, |_| {})
        .with_context(|| format!("cannot simulate level `{id}`"))?;
    let package = bundle
        .to_package()
        .with_context(|| format!("cannot package level `{id}`"))?;
    if package.len() > mw_net::transfer::MAX_PACKAGE_BYTES {
        anyhow::bail!(
            "level `{id}` is too large to send ({} bytes)",
            package.len()
        );
    }
    Ok((sim, package))
}

/// Decodes the payload of a [`ServerMessage::Snapshot`].
pub fn decode_snapshot(data: &[u8]) -> Result<Snapshot> {
    protocol::decode(data).context("invalid snapshot payload")
}

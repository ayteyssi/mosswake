//! [`ClientSession`]: a remote player's side of a game, with **prediction**, **reconciliation**
//! and **interpolation** so that a LAN game feels local.
//!
//! - The local player is simulated ahead with `mw_sim` from its own inputs (no waiting for
//!   the server). Inputs not yet covered by a snapshot are kept.
//! - Each snapshot is authoritative: the local simulation is restored to it and the pending
//!   inputs are replayed on top ([`Snapshot::restore`] makes this exact), so any
//!   misprediction is corrected within one snapshot.
//! - Other players are drawn [`INTERPOLATION_DELAY_TICKS`] in the past, between two received
//!   snapshots: smooth, never extrapolated.

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;

use anyhow::{Context, Result};
use bevy::math::Vec2;
use mw_core::input::{PlayerId, PlayerInput};
use mw_level::LevelBundle;
use mw_net::{ClientMessage, ClientState, LevelCache, NetClient, ServerMessage};
use mw_sim::mechanics::MobilityState;
use mw_sim::{Facing, PlayerState, Rules, SimHarness, SimId, Snapshot, TickInputs};

use crate::input_sender::InputSender;
use crate::level_sync::LevelSync;
use crate::server::{decode_snapshot, TICK};

/// Remote players are shown this many ticks behind the newest snapshot (6 ticks = 100 ms):
/// with snapshots every 2 ticks, there are always two to interpolate between, even when one
/// is lost.
pub const INTERPOLATION_DELAY_TICKS: u64 = 6;

/// Snapshots kept for interpolation (one second at 30 Hz).
const SNAPSHOT_HISTORY: usize = 30;

/// One body as the client should draw it now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewBody {
    pub id: SimId,
    pub player: Option<PlayerId>,
    /// Feet position, world units.
    pub position: Vec2,
    pub state: PlayerState,
    pub facing: Facing,
    pub mobility: MobilityState,
    /// The body of this client's player (predicted), as opposed to interpolated.
    pub local: bool,
}

/// The client: network, level, prediction and interpolation. Call [`ClientSession::frame`]
/// once per simulation tick with the local input.
pub struct ClientSession {
    net: NetClient,
    level: LevelSync,
    sender: InputSender,
    /// Local simulation used for prediction, once the level is loaded.
    predicted: Option<SimHarness>,
    /// Local inputs by tick, not yet covered by a snapshot.
    pending: BTreeMap<u64, PlayerInput>,
    /// Newest snapshots, oldest first.
    history: VecDeque<Snapshot>,
    /// Ticks elapsed since the newest snapshot arrived.
    since_newest: u64,
    /// Distance the local player was moved by the last reconciliation (prediction error).
    last_correction: f32,
}

impl ClientSession {
    /// Connects to `server` as `name`; levels are cached in `cache`.
    pub fn connect(server: SocketAddr, name: &str, cache: LevelCache) -> Result<Self> {
        Ok(Self {
            net: NetClient::connect(server, name).context("cannot start the client")?,
            level: LevelSync::new(cache),
            sender: InputSender::default(),
            predicted: None,
            pending: BTreeMap::new(),
            history: VecDeque::new(),
            since_newest: 0,
            last_correction: 0.0,
        })
    }

    pub fn state(&self) -> &ClientState {
        self.net.state()
    }

    pub fn player(&self) -> Option<PlayerId> {
        self.net.player()
    }

    /// The level being played, once received.
    pub fn level(&self) -> Option<&LevelBundle> {
        self.level.level()
    }

    /// Rules of the predicted simulation (for drawing, e.g. dash state).
    pub fn rules(&self) -> Option<&Rules> {
        self.predicted
            .as_ref()
            .map(|sim| sim.world().resource::<Rules>())
    }

    /// Newest authoritative snapshot.
    pub fn latest_snapshot(&self) -> Option<&Snapshot> {
        self.history.back()
    }

    /// How far the last snapshot moved our predicted player (world units).
    pub fn last_correction(&self) -> f32 {
        self.last_correction
    }

    /// One tick: network in, level sync, snapshots (reconciliation), local input
    /// (prediction), network out. Returns `true` when a new level was loaded this tick.
    pub fn frame(&mut self, input: PlayerInput) -> Result<bool> {
        self.net.update(TICK).context("network update failed")?;
        self.since_newest += 1;
        let rtt_ticks = (self.net.rtt().as_secs_f64() / TICK.as_secs_f64()).ceil() as u64;
        self.sender.set_rtt_ticks(rtt_ticks);
        let mut new_level = false;
        for message in self.net.receive() {
            let outcome = self.level.handle(&message)?;
            for reply in &outcome.replies {
                self.net.send(reply);
            }
            if let Some(bundle) = outcome.loaded {
                self.load(&bundle)?;
                new_level = true;
            }
            if let ServerMessage::Snapshot { data, .. } = message {
                if self.predicted.is_some() {
                    self.on_snapshot(decode_snapshot(&data)?);
                }
            }
        }
        if let Some(message) = self.sender.next(input) {
            if let ClientMessage::Inputs { first_tick, inputs } = &message {
                let tick = first_tick + inputs.len() as u64 - 1;
                self.pending.insert(tick, input);
                self.predict_tick(tick);
            }
            self.net.send(&message);
        }
        self.net.flush().context("network send failed")?;
        Ok(new_level)
    }

    fn load(&mut self, bundle: &LevelBundle) -> Result<()> {
        let sim = SimHarness::new(&bundle.level)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("cannot simulate the received level")?;
        self.predicted = Some(sim);
        self.pending.clear();
        self.history.clear();
        self.sender = InputSender::default();
        Ok(())
    }

    fn on_snapshot(&mut self, snapshot: Snapshot) {
        if self.history.back().is_some_and(|s| s.tick >= snapshot.tick) {
            return; // older or duplicate (unreliable channel)
        }
        self.sender.on_snapshot(snapshot.tick);
        self.pending.retain(|tick, _| *tick >= snapshot.tick);
        self.history.push_back(snapshot);
        while self.history.len() > SNAPSHOT_HISTORY {
            self.history.pop_front();
        }
        self.since_newest = 0;
        self.reconcile();
    }

    /// Restores the newest snapshot and replays the pending inputs up to the predicted tick.
    fn reconcile(&mut self) {
        let (Some(sim), Some(snapshot), Some(me)) = (
            self.predicted.as_mut(),
            self.history.back(),
            self.net.player(),
        ) else {
            return;
        };
        let before = local_position(sim, me);
        let target = sim.tick().max(snapshot.tick);
        sim.restore(snapshot);
        let others = last_inputs(snapshot, me);
        for tick in snapshot.tick..target {
            let mut inputs = others.clone();
            if let Some(input) = self.pending.get(&tick) {
                inputs.0.insert(me, *input);
            }
            sim.step(1, &inputs);
        }
        if let (Some(a), Some(b)) = (before, local_position(sim, me)) {
            self.last_correction = a.distance(b);
        }
    }

    /// Simulates `tick` locally with our input (resynchronizing first if needed).
    fn predict_tick(&mut self, tick: u64) {
        let Some(me) = self.net.player() else {
            return;
        };
        let Some(sim) = self.predicted.as_mut() else {
            return;
        };
        if sim.tick() != tick {
            // The input tick moved (first snapshot, resynchronization): rebuild from the
            // newest snapshot up to `tick`.
            let Some(snapshot) = self.history.back() else {
                return;
            };
            if snapshot.tick > tick {
                return;
            }
            sim.restore(snapshot);
            let others = last_inputs(snapshot, me);
            for t in snapshot.tick..tick {
                let mut inputs = others.clone();
                if let Some(input) = self.pending.get(&t) {
                    inputs.0.insert(me, *input);
                }
                sim.step(1, &inputs);
            }
        }
        let mut inputs = self
            .history
            .back()
            .map(|s| last_inputs(s, me))
            .unwrap_or_default();
        if let Some(input) = self.pending.get(&tick) {
            inputs.0.insert(me, *input);
        }
        sim.step(1, &inputs);
    }

    /// What to draw now: our player predicted, the others interpolated in the past.
    pub fn view(&mut self) -> Vec<ViewBody> {
        let me = self.net.player();
        let mut bodies = Vec::new();
        if let (Some(sim), Some(me)) = (self.predicted.as_mut(), me) {
            let snapshot = sim.snapshot();
            if let Some(body) = snapshot.player(me) {
                bodies.push(view_body(body, true));
            }
        }
        bodies.extend(
            self.interpolated()
                .into_iter()
                .filter(|b| b.player.is_none() || b.player != me),
        );
        bodies.sort_by_key(|b| b.id);
        bodies
    }

    /// Bodies of the snapshots, `INTERPOLATION_DELAY_TICKS` behind the newest one.
    fn interpolated(&self) -> Vec<ViewBody> {
        let Some(newest) = self.history.back() else {
            return Vec::new();
        };
        let at = (newest.tick + self.since_newest).saturating_sub(INTERPOLATION_DELAY_TICKS);
        interpolate(&self.history, at)
    }

    /// Leaves the game.
    pub fn disconnect(&mut self) {
        self.net.disconnect();
    }
}

/// The world `at` a tick, interpolated between the two snapshots around it (the oldest or
/// newest one when outside the history: never extrapolated).
pub fn interpolate(history: &VecDeque<Snapshot>, at: u64) -> Vec<ViewBody> {
    let after = history.iter().position(|s| s.tick >= at);
    let (a, b) = match after {
        None => match history.back() {
            Some(last) => (last, last),
            None => return Vec::new(),
        },
        Some(0) => (&history[0], &history[0]),
        Some(i) => (&history[i - 1], &history[i]),
    };
    let t = if b.tick > a.tick {
        (at.clamp(a.tick, b.tick) - a.tick) as f32 / (b.tick - a.tick) as f32
    } else {
        0.0
    };
    a.bodies
        .iter()
        .map(|from| {
            let mut body = view_body(from, false);
            if let Some(to) = b.bodies.iter().find(|x| x.id == from.id) {
                body.position =
                    Vec2::from_array(from.position).lerp(Vec2::from_array(to.position), t);
            }
            body
        })
        .collect()
}

fn view_body(body: &mw_sim::BodySnapshot, local: bool) -> ViewBody {
    let player = body.player;
    ViewBody {
        id: body.id,
        player: player.map(|p| p.player),
        position: Vec2::from_array(body.position),
        state: player.map(|p| p.state).unwrap_or_default(),
        facing: player.map(|p| p.facing).unwrap_or_default(),
        mobility: player.map(|p| p.mobility).unwrap_or_default(),
        local,
    }
}

fn local_position(sim: &mut SimHarness, me: PlayerId) -> Option<Vec2> {
    sim.snapshot()
        .player(me)
        .map(|b| Vec2::from_array(b.position))
}

/// Other players are predicted by repeating their last known input.
fn last_inputs(snapshot: &Snapshot, me: PlayerId) -> TickInputs {
    TickInputs(
        snapshot
            .bodies
            .iter()
            .filter_map(|b| b.player)
            .filter(|p| p.player != me)
            .map(|p| (p.player, p.control.input))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mw_sim::{BodySnapshot, Contacts};

    fn snap(tick: u64, x: f32) -> Snapshot {
        Snapshot {
            tick,
            bodies: vec![BodySnapshot {
                id: SimId(0),
                position: [x, 0.0],
                velocity: [0.0, 0.0],
                contacts: Contacts::default(),
                player: None,
                combat: None,
            }],
        }
    }

    #[test]
    fn interpolates_between_snapshots_without_extrapolating() {
        let history: VecDeque<_> = [snap(10, 0.0), snap(12, 20.0), snap(14, 40.0)].into();
        let x = |at| interpolate(&history, at)[0].position.x;
        assert_eq!(x(10), 0.0);
        assert_eq!(x(11), 10.0);
        assert_eq!(x(13), 30.0);
        assert_eq!(x(5), 0.0, "before the history: oldest");
        assert_eq!(x(99), 40.0, "after the history: newest, not extrapolated");
        assert!(interpolate(&VecDeque::new(), 3).is_empty());
    }
}

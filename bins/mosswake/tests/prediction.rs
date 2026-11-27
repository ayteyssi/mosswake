//! Client prediction through a link conditioner (latency, jitter, loss) over real UDP.
//! Time is logical: one loop iteration = one tick for the server, the proxy and the clients.

use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use bevy::math::Vec2;
use mosswake::client::ClientSession;
use mosswake::server::{ServerConfig, ServerSession, TICK};
use mw_core::input::{buttons, PlayerId, PlayerInput};
use mw_net::{Conditions, LevelCache, LinkConditioner};

struct Lan {
    server: ServerSession,
    proxy: LinkConditioner,
    clients: Vec<ClientSession>,
}

impl Lan {
    fn new(conditions: Conditions) -> Self {
        let levels = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels");
        let bundle = mosswake::load_level(&levels, "tutorial").unwrap();
        let server = ServerSession::new(
            &bundle,
            ServerConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                max_players: 4,
            },
        )
        .unwrap();
        let proxy = LinkConditioner::bind(
            "127.0.0.1:0".parse().unwrap(),
            server.local_addr(),
            conditions,
        )
        .unwrap();
        Self {
            server,
            proxy,
            clients: Vec::new(),
        }
    }

    fn join(&mut self, name: &str) -> usize {
        let addr = self.proxy.local_addr().unwrap();
        self.clients
            .push(ClientSession::connect(addr, name, LevelCache::default()).unwrap());
        self.clients.len() - 1
    }

    /// One tick for everyone; `inputs[i]` for client `i`.
    fn tick(&mut self, inputs: &[PlayerInput]) {
        self.server.tick().unwrap();
        self.proxy.update(TICK).unwrap();
        for (i, client) in self.clients.iter_mut().enumerate() {
            client
                .frame(inputs.get(i).copied().unwrap_or_default())
                .unwrap();
        }
        sleep(Duration::from_micros(200));
    }

    fn server_x(&mut self, player: PlayerId) -> Option<f32> {
        self.server
            .sim()
            .snapshot()
            .player(player)
            .map(|b| b.position[0])
    }
}

fn local(client: &mut ClientSession) -> Option<Vec2> {
    client
        .view()
        .into_iter()
        .find(|b| b.local)
        .map(|b| b.position)
}

fn right() -> PlayerInput {
    PlayerInput {
        axis_x: 127,
        ..Default::default()
    }
}

fn conditions() -> Conditions {
    Conditions {
        latency: Duration::from_millis(100),
        jitter: Duration::from_millis(10),
        loss: 0.05,
        seed: 42,
    }
}

#[test]
fn prediction_is_immediate_and_converges_under_100ms_latency() {
    let mut lan = Lan::new(conditions());
    let me = lan.join("Ayla");
    for _ in 0..600 {
        lan.tick(&[]);
        let player = lan.clients[me].player();
        if player.is_some_and(|p| lan.server_x(p).is_some())
            && local(&mut lan.clients[me]).is_some()
        {
            break;
        }
    }
    let player = lan.clients[me].player().expect("connected");
    // Let RTT estimation and tick synchronization settle.
    for _ in 0..120 {
        lan.tick(&[]);
    }
    let start = local(&mut lan.clients[me]).expect("predicted body");
    let server_start = lan.server_x(player).unwrap();

    // 1. Prediction: our player moves at once, long before the server hears about it.
    for _ in 0..3 {
        lan.tick(&[right()]);
    }
    let predicted = local(&mut lan.clients[me]).unwrap();
    assert!(
        predicted.x > start.x,
        "predicted {predicted} vs start {start}"
    );
    assert_eq!(
        lan.server_x(player).unwrap(),
        server_start,
        "server has not received it yet"
    );

    // Run and jump around under latency and loss.
    for t in 0..120 {
        let jump = if t % 40 < 15 { buttons::JUMP } else { 0 };
        lan.tick(&[PlayerInput {
            buttons: jump,
            ..right()
        }]);
    }
    // 2. No lasting divergence: once idle, prediction and server agree exactly.
    for _ in 0..150 {
        lan.tick(&[]);
    }
    let predicted = local(&mut lan.clients[me]).unwrap();
    let server = lan.server_x(player).unwrap();
    assert!(
        (predicted.x - server).abs() < 0.01,
        "client {} vs server {server}",
        predicted.x
    );
    assert!(
        server > server_start + 100.0,
        "the server did move the player"
    );
    assert!(lan.clients[me].last_correction() < 0.01);
    assert!(
        lan.proxy.stats().dropped > 0,
        "the test really lost packets"
    );
}

#[test]
fn other_players_are_interpolated_to_the_server_position() {
    let mut lan = Lan::new(conditions());
    let mover = lan.join("Mover");
    let watcher = lan.join("Watcher");
    for _ in 0..600 {
        lan.tick(&[]);
        let ready = (0..2).all(|i| local(&mut lan.clients[i]).is_some());
        if ready {
            break;
        }
    }
    for _ in 0..120 {
        lan.tick(&[]);
    }
    let mover_id = lan.clients[mover].player().unwrap();
    for _ in 0..90 {
        lan.tick(&[right(), PlayerInput::default()]);
    }
    for _ in 0..120 {
        lan.tick(&[]);
    }
    let seen = lan.clients[watcher]
        .view()
        .into_iter()
        .find(|b| b.player == Some(mover_id) && !b.local)
        .expect("watcher sees the mover");
    let server = lan.server_x(mover_id).unwrap();
    assert!(
        (seen.position.x - server).abs() < 0.01,
        "seen {} vs server {server}",
        seen.position.x
    );
}

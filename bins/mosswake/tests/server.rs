//! Headless server + simulated clients over real UDP on 127.0.0.1, in logical time: one
//! loop iteration = one server tick, with a tiny real wait for packets to cross loopback.

use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use mosswake::host::HostSession;
use mosswake::input_sender::{InputSender, INPUT_LEAD_TICKS};
use mosswake::level_sync::LevelSync;
use mosswake::server::{decode_snapshot, ServerConfig, ServerSession, TICK};
use mw_core::input::{PlayerId, PlayerInput};
use mw_level::LevelBundle;
use mw_net::{protocol, ClientState, NetClient, ServerMessage};
use mw_sim::{
    BodySnapshot, Contacts, Facing, PlayerControl, PlayerSnapshot, PlayerState, SimId, Snapshot,
};

fn tutorial() -> LevelBundle {
    let levels = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels");
    mosswake::load_level(&levels, "tutorial").expect("tutorial loads")
}

fn server(max_players: u8) -> ServerSession {
    ServerSession::new(
        &tutorial(),
        ServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            max_players,
        },
    )
    .expect("server starts")
}

fn right() -> PlayerInput {
    PlayerInput {
        axis_x: 127,
        ..Default::default()
    }
}

/// A remote client as the game will run it: network + input sender + newest snapshot.
struct Remote {
    client: NetClient,
    level: LevelSync,
    sender: InputSender,
    latest: Option<Snapshot>,
    snapshots: Vec<u64>,
    /// Send only messages for which this returns true (simulates packet loss).
    deliver: fn(u64) -> bool,
    sent: u64,
}

impl Remote {
    fn join(server: &ServerSession, name: &str) -> Self {
        Self {
            client: NetClient::connect(server.local_addr(), name).expect("client starts"),
            level: LevelSync::default(),
            sender: InputSender::default(),
            latest: None,
            snapshots: Vec::new(),
            deliver: |_| true,
            sent: 0,
        }
    }

    fn frame(&mut self, input: PlayerInput) {
        self.client.update(TICK).unwrap();
        for message in self.client.receive() {
            let outcome = self.level.handle(&message).expect("usable level");
            for reply in &outcome.replies {
                self.client.send(reply);
            }
            if let ServerMessage::Snapshot { tick, data } = message {
                let snapshot = decode_snapshot(&data).expect("valid snapshot");
                assert_eq!(snapshot.tick, tick, "header and payload agree");
                self.snapshots.push(tick);
                self.sender.on_snapshot(tick);
                self.latest = Some(snapshot);
            }
        }
        if let Some(message) = self.sender.next(input) {
            if (self.deliver)(self.sent) {
                self.client.send(&message);
            }
            self.sent += 1;
        }
        self.client.flush().unwrap();
    }

    fn me(&self) -> Option<&BodySnapshot> {
        let player = self.client.player()?;
        self.latest.as_ref()?.player(player)
    }
}

/// Runs `ticks` server ticks, every client sending `input`.
fn run(server: &mut ServerSession, clients: &mut [&mut Remote], ticks: u32, input: PlayerInput) {
    for _ in 0..ticks {
        server.tick().unwrap();
        for client in clients.iter_mut() {
            client.frame(input);
        }
        sleep(Duration::from_micros(300));
    }
}

fn until(
    server: &mut ServerSession,
    clients: &mut [&mut Remote],
    max: u32,
    mut done: impl FnMut(&mut ServerSession, &mut [&mut Remote]) -> bool,
) {
    for _ in 0..max {
        run(server, clients, 1, PlayerInput::default());
        if done(server, clients) {
            return;
        }
    }
    panic!("condition not reached after {max} ticks");
}

fn server_player_x(server: &mut ServerSession, player: PlayerId) -> f32 {
    server
        .sim()
        .snapshot()
        .player(player)
        .expect("player spawned on the server")
        .position[0]
}

#[test]
fn player_moves_on_the_server_from_client_inputs() {
    let mut server = server(4);
    let mut a = Remote::join(&server, "Ayla");
    until(&mut server, &mut [&mut a], 200, |_, c| c[0].me().is_some());
    let player = a.client.player().expect("connected");

    // Settle on the ground, then hold right for one second.
    run(&mut server, &mut [&mut a], 30, PlayerInput::default());
    let start = server_player_x(&mut server, player);
    run(&mut server, &mut [&mut a], 60, right());
    let end = server_player_x(&mut server, player);
    assert!(
        end > start + 64.0,
        "moved right on the server: {start} -> {end}"
    );

    // The client sees the authoritative position in the snapshots.
    let seen = a.me().expect("in snapshot").position[0];
    assert!(seen > start + 64.0 && seen <= end, "client sees {seen}");
    assert_eq!(
        a.me().unwrap().player.unwrap().state,
        PlayerState::Run,
        "animation state travels too"
    );
}

#[test]
fn snapshots_are_sent_at_30_hz_with_their_tick() {
    let mut server = server(4);
    let mut a = Remote::join(&server, "Ayla");
    until(&mut server, &mut [&mut a], 200, |_, c| {
        !c[0].snapshots.is_empty()
    });
    a.snapshots.clear();
    run(&mut server, &mut [&mut a], 120, PlayerInput::default());
    // 120 ticks at 60 Hz = 2 s, one snapshot every 2 ticks (loopback loses nothing).
    assert!(
        (58..=61).contains(&a.snapshots.len()),
        "{} snapshots",
        a.snapshots.len()
    );
    assert!(a.snapshots.iter().all(|t| t % 2 == 0), "{:?}", a.snapshots);
    assert!(a.snapshots.windows(2).all(|w| w[1] == w[0] + 2));
}

#[test]
fn players_are_spawned_on_join_and_despawned_on_leave() {
    let mut server = server(4);
    let mut a = Remote::join(&server, "Ayla");
    let mut b = Remote::join(&server, "Bo");
    until(&mut server, &mut [&mut a, &mut b], 200, |s, _| {
        s.sim().snapshot().bodies.len() == 2
    });
    let snap = server.sim().snapshot();
    let pa = a.client.player().unwrap();
    let pb = b.client.player().unwrap();
    assert_ne!(pa, pb);
    assert!(snap.player(pa).is_some() && snap.player(pb).is_some());

    b.client.disconnect();
    until(&mut server, &mut [&mut a], 200, |s, _| {
        s.players().len() == 1
    });
    run(&mut server, &mut [&mut a], 4, PlayerInput::default());
    let snap = server.sim().snapshot();
    assert_eq!(snap.bodies.len(), 1, "B's body is gone");
    assert!(snap.player(pb).is_none());
    let seen = a.latest.as_ref().unwrap();
    assert!(seen.player(pb).is_none(), "A no longer sees B");
    assert!(seen.player(pa).is_some());
}

#[test]
fn redundancy_covers_lost_input_packets() {
    let mut server = server(4);
    let mut a = Remote::join(&server, "Ayla");
    // Drop 2 messages out of 3: every input still reaches the server inside a later message.
    a.deliver = |n| n % 3 == 0;
    until(&mut server, &mut [&mut a], 200, |_, c| c[0].me().is_some());
    let player = a.client.player().unwrap();
    run(&mut server, &mut [&mut a], 10, PlayerInput::default());

    // A different input every tick (non-periodic, so the offset found below is unique);
    // record what the server applied on each tick.
    let mut applied = Vec::new();
    let mut sent = Vec::new();
    for i in 0..40 {
        let input = PlayerInput {
            axis_x: (i * 7 % 101) as i8 - 50,
            ..Default::default()
        };
        server.tick().unwrap();
        let control = server
            .sim()
            .snapshot()
            .player(player)
            .unwrap()
            .player
            .unwrap();
        applied.push(control.control.input.axis_x);
        a.frame(input);
        sent.push(input.axis_x);
        sleep(Duration::from_micros(300));
    }
    // Every input reached the server in time despite the losses: after a warm-up, the
    // applied sequence is exactly the sent one, shifted by the input lead.
    let tail = &applied[10..];
    let lead = (1..=INPUT_LEAD_TICKS as usize + 2)
        .find(|&d| tail.iter().enumerate().all(|(j, &x)| x == sent[10 + j - d]))
        .unwrap_or_else(|| panic!("inputs lost or repeated: sent {sent:?}, applied {applied:?}"));
    assert_eq!(
        lead as u64,
        INPUT_LEAD_TICKS + 1,
        "frame i sends for tick i + 1 + lead"
    );
}

#[test]
fn host_mode_runs_server_and_local_client_in_one_process() {
    let mut host = HostSession::new(
        &tutorial(),
        ServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            max_players: 4,
        },
        "Host",
    )
    .expect("host starts");
    for _ in 0..200 {
        host.frame(PlayerInput::default()).unwrap();
        if host.latest_snapshot().is_some() {
            break;
        }
        sleep(Duration::from_micros(300));
    }
    assert_eq!(
        host.client_state(),
        &ClientState::Connected {
            player: PlayerId(0)
        }
    );
    let player = host.local_player().unwrap();
    for _ in 0..30 {
        host.frame(PlayerInput::default()).unwrap();
        sleep(Duration::from_micros(300));
    }
    let start = host
        .latest_snapshot()
        .unwrap()
        .player(player)
        .unwrap()
        .position[0];
    for _ in 0..60 {
        host.frame(right()).unwrap();
        sleep(Duration::from_micros(300));
    }
    let end = host
        .latest_snapshot()
        .unwrap()
        .player(player)
        .unwrap()
        .position[0];
    assert!(end > start + 64.0, "local player moved: {start} -> {end}");
    // A remote player can join the hosted session.
    let mut remote = Remote::join(host.server(), "Guest");
    for _ in 0..200 {
        host.frame(PlayerInput::default()).unwrap();
        remote.frame(PlayerInput::default());
        if remote.me().is_some() {
            break;
        }
        sleep(Duration::from_micros(300));
    }
    assert_eq!(remote.client.player(), Some(PlayerId(1)));
    host.shutdown();
}

/// The snapshot payload is part of the network protocol. If this test fails, the encoding
/// changed: bump `mw_net::PROTOCOL_VERSION`, then update the expected bytes.
#[test]
fn snapshot_encoding_is_pinned_to_the_protocol_version() {
    let snapshot = Snapshot {
        tick: 300,
        bodies: vec![BodySnapshot {
            id: SimId(2),
            position: [1.5, 32.0],
            velocity: [0.0, -1.0],
            contacts: Contacts {
                grounded: true,
                ..Default::default()
            },
            player: Some(PlayerSnapshot {
                player: PlayerId(1),
                state: PlayerState::Run,
                facing: Facing::Left,
                mobility: mw_sim::mechanics::MobilityState::default(),
                control: PlayerControl {
                    input: right(),
                    previous: PlayerInput::default(),
                    coyote_ticks: 6,
                    buffer_ticks: 0,
                    jumping: false,
                },
            }),
        }],
    };
    let bytes = protocol::encode(&snapshot).unwrap();
    assert_eq!(
        (mw_net::PROTOCOL_VERSION, bytes.as_slice()),
        // v2 (#6) extended the snapshot; v3 (#16) added level-transfer messages only.
        (3, EXPECTED_SNAPSHOT.as_slice()),
        "snapshot encoding changed: bump PROTOCOL_VERSION"
    );
    assert_eq!(decode_snapshot(&bytes).unwrap(), snapshot);
}

const EXPECTED_SNAPSHOT: [u8; 44] = [
    172, 2, 1, 2, 0, 0, 192, 63, 0, 0, 0, 66, 0, 0, 0, 0, 0, 0, 128, 191, 1, 0, 0, 0, 1, 1, 1, 0,
    127, 0, 0, 0, 0, 0, 6, 0, 0, 0, 0, 1, 1, 0, 0, 0,
];

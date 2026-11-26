//! Real UDP on 127.0.0.1: a server and clients pumped frame by frame.
//!
//! Time is logical: every frame advances everyone by `DT` (timeouts included) while the
//! real wait per frame stays tiny, so a 15-second timeout test runs in about a second.

use std::thread::sleep;
use std::time::Duration;

use mw_core::input::{PlayerId, PlayerInput};
use mw_net::{
    ClientMessage, ClientState, LeaveReason, NetClient, NetServer, NetServerConfig, PlayerInfo,
    RejectReason, ServerMessage, ServerNetEvent, PROTOCOL_VERSION,
};

const DT: Duration = Duration::from_millis(50);

struct Lan {
    server: Option<NetServer>,
    clients: Vec<NetClient>,
    events: Vec<ServerNetEvent>,
    inboxes: Vec<Vec<ServerMessage>>,
}

impl Lan {
    fn new(max_players: u8) -> Self {
        let server = NetServer::bind(NetServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            max_players,
        })
        .expect("server binds");
        Self {
            server: Some(server),
            clients: Vec::new(),
            events: Vec::new(),
            inboxes: Vec::new(),
        }
    }

    fn server(&mut self) -> &mut NetServer {
        self.server.as_mut().expect("server running")
    }

    fn join(&mut self, name: &str) -> usize {
        self.join_with_version(name, PROTOCOL_VERSION)
    }

    fn join_with_version(&mut self, name: &str, version: u32) -> usize {
        let addr = self.server().local_addr();
        let client = NetClient::connect_with_version(addr, name, version).expect("client starts");
        self.clients.push(client);
        self.inboxes.push(Vec::new());
        self.clients.len() - 1
    }

    /// One frame for everyone (skipping clients in `frozen`, as if their process hung).
    fn frame_except(&mut self, frozen: &[usize]) {
        if let Some(server) = self.server.as_mut() {
            server.update(DT).unwrap();
            self.events.extend(server.events());
        }
        for (i, client) in self.clients.iter_mut().enumerate() {
            if frozen.contains(&i) {
                continue;
            }
            client.update(DT).unwrap();
            self.inboxes[i].extend(client.receive());
        }
        if let Some(server) = self.server.as_mut() {
            server.flush();
        }
        for (i, client) in self.clients.iter_mut().enumerate() {
            if !frozen.contains(&i) {
                client.flush().unwrap();
            }
        }
        sleep(Duration::from_millis(1));
    }

    /// Pumps frames until `done` holds (panics after `max` frames).
    fn until(&mut self, max: u32, mut done: impl FnMut(&mut Self) -> bool) {
        for _ in 0..max {
            self.frame_except(&[]);
            if done(self) {
                return;
            }
        }
        panic!(
            "condition not reached after {max} frames; events: {:?}",
            self.events
        );
    }

    fn connected(&self, i: usize) -> bool {
        matches!(self.clients[i].state(), ClientState::Connected { .. })
    }
}

fn info(player: u8, name: &str) -> PlayerInfo {
    PlayerInfo {
        player: PlayerId(player),
        name: name.into(),
    }
}

#[test]
fn server_and_two_clients_exchange_messages() {
    let mut lan = Lan::new(4);
    let alice = lan.join("Alice");
    lan.until(200, |l| l.connected(alice));
    let bob = lan.join("Bob");
    lan.until(200, |l| l.connected(bob));

    assert_eq!(lan.clients[alice].player(), Some(PlayerId(0)));
    assert_eq!(lan.clients[bob].player(), Some(PlayerId(1)));
    assert_eq!(
        lan.events,
        [
            ServerNetEvent::PlayerConnected(info(0, "Alice")),
            ServerNetEvent::PlayerConnected(info(1, "Bob")),
        ]
    );
    assert_eq!(lan.server().players(), [info(0, "Alice"), info(1, "Bob")]);

    // Session events: each client learns who is there.
    lan.until(100, |l| {
        l.inboxes[alice].len() >= 2 && !l.inboxes[bob].is_empty()
    });
    assert_eq!(
        lan.inboxes[alice],
        [
            ServerMessage::Roster(vec![info(0, "Alice")]),
            ServerMessage::PlayerJoined(info(1, "Bob")),
        ]
    );
    assert_eq!(
        lan.inboxes[bob],
        [ServerMessage::Roster(vec![
            info(0, "Alice"),
            info(1, "Bob")
        ])]
    );

    // Client -> server inputs (unreliable channel).
    let input = |x| PlayerInput {
        axis_x: x,
        axis_y: 0,
        buttons: 1,
    };
    lan.clients[alice].send(&ClientMessage::Inputs {
        first_tick: 10,
        inputs: vec![input(127)],
    });
    lan.clients[bob].send(&ClientMessage::Inputs {
        first_tick: 11,
        inputs: vec![input(-127), input(0)],
    });
    let mut received = Vec::new();
    lan.until(100, |l| {
        received.extend(l.server().receive());
        received.len() >= 2
    });
    received.sort_by_key(|(player, _)| *player);
    assert_eq!(
        received,
        [
            (
                PlayerId(0),
                ClientMessage::Inputs {
                    first_tick: 10,
                    inputs: vec![input(127)]
                }
            ),
            (
                PlayerId(1),
                ClientMessage::Inputs {
                    first_tick: 11,
                    inputs: vec![input(-127), input(0)]
                }
            ),
        ]
    );

    // Server -> clients: broadcast snapshot (unreliable) and a direct message (reliable).
    for inbox in &mut lan.inboxes {
        inbox.clear();
    }
    let snapshot = ServerMessage::Snapshot {
        tick: 42,
        data: vec![7; 3000], // bigger than one packet: sliced by renet
    };
    lan.server().broadcast(&snapshot);
    lan.server()
        .send(PlayerId(1), &ServerMessage::PlayerJoined(info(9, "Ghost")));
    lan.until(100, |l| {
        !l.inboxes[alice].is_empty() && l.inboxes[bob].len() >= 2
    });
    assert_eq!(lan.inboxes[alice], std::slice::from_ref(&snapshot));
    assert!(lan.inboxes[bob].contains(&snapshot));
    assert!(lan.inboxes[bob].contains(&ServerMessage::PlayerJoined(info(9, "Ghost"))));
}

#[test]
fn version_mismatch_is_refused_with_a_reason() {
    let mut lan = Lan::new(4);
    let old = lan.join_with_version("Oldie", PROTOCOL_VERSION + 41);
    lan.until(200, |l| {
        !matches!(l.clients[old].state(), ClientState::Connecting)
    });
    let expected = RejectReason::VersionMismatch {
        server: PROTOCOL_VERSION,
        client: PROTOCOL_VERSION + 41,
    };
    assert_eq!(
        lan.clients[old].state(),
        &ClientState::Rejected(expected.clone())
    );
    assert!(matches!(
        &lan.events[..],
        [ServerNetEvent::ClientRejected { reason, .. }] if *reason == expected
    ));
    assert!(lan.server().players().is_empty());
}

#[test]
fn full_server_refuses_with_a_reason_and_frees_slots() {
    let mut lan = Lan::new(1);
    let first = lan.join("First");
    lan.until(200, |l| l.connected(first));
    let second = lan.join("Second");
    lan.until(200, |l| {
        !matches!(l.clients[second].state(), ClientState::Connecting)
    });
    assert_eq!(
        lan.clients[second].state(),
        &ClientState::Rejected(RejectReason::ServerFull)
    );

    // Once the first player leaves, a newcomer gets slot 0 again.
    lan.clients[first].disconnect();
    lan.until(200, |l| l.server().players().is_empty());
    let third = lan.join("Third");
    lan.until(200, |l| l.connected(third));
    assert_eq!(lan.clients[third].player(), Some(PlayerId(0)));
}

#[test]
fn clean_disconnect_is_reported_to_everyone() {
    let mut lan = Lan::new(4);
    let alice = lan.join("Alice");
    let bob = lan.join("Bob");
    lan.until(200, |l| l.connected(alice) && l.connected(bob));
    let bob_id = lan.clients[bob].player().unwrap();

    lan.clients[bob].disconnect();
    lan.until(200, |l| {
        l.inboxes[alice].contains(&ServerMessage::PlayerLeft {
            player: bob_id,
            reason: LeaveReason::Left,
        })
    });
    assert!(lan.events.contains(&ServerNetEvent::PlayerDisconnected {
        player: bob_id,
        reason: LeaveReason::Left,
    }));
}

#[test]
fn silent_client_times_out() {
    let mut lan = Lan::new(4);
    let alice = lan.join("Alice");
    let bob = lan.join("Bob");
    lan.until(200, |l| l.connected(alice) && l.connected(bob));
    let bob_id = lan.clients[bob].player().unwrap();

    // Bob hangs (no update, no flush) for longer than TIMEOUT; Alice stays alive.
    let frames = (mw_net::TIMEOUT.as_millis() / DT.as_millis()) as u32 + 20;
    for _ in 0..frames {
        lan.frame_except(&[bob]);
    }
    assert!(lan.events.contains(&ServerNetEvent::PlayerDisconnected {
        player: bob_id,
        reason: LeaveReason::TimedOut,
    }));
    assert!(lan.inboxes[alice].contains(&ServerMessage::PlayerLeft {
        player: bob_id,
        reason: LeaveReason::TimedOut,
    }));
    assert!(lan.connected(alice));
}

#[test]
fn client_notices_when_the_server_disappears() {
    let mut lan = Lan::new(4);
    let alice = lan.join("Alice");
    lan.until(200, |l| l.connected(alice));
    lan.server = None; // server process gone, without a goodbye
    lan.until(400, |l| {
        matches!(l.clients[alice].state(), ClientState::Disconnected(_))
    });
}

#[test]
fn invalid_names_are_refused_locally() {
    let addr = "127.0.0.1:9".parse().unwrap();
    assert!(NetClient::connect(addr, "").is_err());
    assert!(NetClient::connect(addr, "bad\nname").is_err());
    assert!(NetClient::connect(addr, &"x".repeat(25)).is_err());
}

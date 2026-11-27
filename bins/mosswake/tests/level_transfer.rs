//! Level transfer over real UDP on 127.0.0.1: download before playing, cache by hash,
//! level change during a game.

use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use mosswake::level_sync::LevelSync;
use mosswake::server::{ServerConfig, ServerSession, TICK};
use mw_core::input::PlayerId;
use mw_level::{Layer, LayerKind, Level, LevelBundle, Size, Spawn};
use mw_net::{LevelCache, NetClient};

fn tutorial() -> LevelBundle {
    let levels = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels");
    mosswake::load_level(&levels, "tutorial").expect("tutorial loads")
}

/// A small second level with a script, to switch to.
fn arena() -> LevelBundle {
    let mut level = Level::new("arena", "Arena", Size { w: 8, h: 4 });
    level.legend.insert('#', "mossy_ground".into());
    level.layers.push(Layer {
        name: "ground".into(),
        kind: LayerKind::Solid,
        tiles: vec![
            "#......#".into(),
            "#......#".into(),
            "#......#".into(),
            "########".into(),
        ],
    });
    level.spawns = vec![Spawn { x: 2.5, y: 1.0 }, Spawn { x: 5.5, y: 1.0 }];
    level.scripts = vec!["scripts/main.rhai".into()];
    let scripts = [(
        "scripts/main.rhai".to_string(),
        "fn on_tick(dt) {}".to_string(),
    )]
    .into();
    LevelBundle { level, scripts }
}

fn server() -> ServerSession {
    ServerSession::new(
        &tutorial(),
        ServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            max_players: 4,
        },
    )
    .expect("server starts")
}

/// A client that only syncs the level (enough to be spawned and to follow level changes).
struct Client {
    net: NetClient,
    level: LevelSync,
    loads: Vec<String>,
    /// When false, offers are received but never answered.
    answer: bool,
}

impl Client {
    fn join(server: &ServerSession, name: &str, cache: LevelCache) -> Self {
        Self {
            net: NetClient::connect(server.local_addr(), name).expect("client starts"),
            level: LevelSync::new(cache),
            loads: Vec::new(),
            answer: true,
        }
    }

    fn frame(&mut self) {
        self.net.update(TICK).unwrap();
        for message in self.net.receive() {
            if !self.answer {
                continue;
            }
            let outcome = self.level.handle(&message).expect("usable level");
            if let Some(bundle) = outcome.loaded {
                self.loads.push(bundle.level.id);
            }
            for reply in &outcome.replies {
                self.net.send(reply);
            }
        }
        self.net.flush().unwrap();
    }

    fn player(&self) -> PlayerId {
        self.net.player().expect("connected")
    }
}

fn until(
    server: &mut ServerSession,
    clients: &mut [&mut Client],
    max: u32,
    mut done: impl FnMut(&mut ServerSession, &[&mut Client]) -> bool,
) {
    for _ in 0..max {
        server.tick().unwrap();
        for client in clients.iter_mut() {
            client.frame();
        }
        if done(server, clients) {
            return;
        }
        sleep(Duration::from_micros(300));
    }
    panic!("condition not reached after {max} ticks");
}

fn spawned(server: &mut ServerSession, player: PlayerId) -> bool {
    server.sim().snapshot().player(player).is_some()
}

#[test]
fn clients_download_the_level_before_their_player_spawns() {
    let mut server = server();
    let mut alice = Client::join(&server, "Alice", LevelCache::default());
    until(&mut server, &mut [&mut alice], 300, |s, c| {
        c[0].net.player().is_some_and(|p| spawned(s, p))
    });
    assert_eq!(alice.loads, ["tutorial"]);
    assert_eq!(
        alice.level.level(),
        Some(&tutorial()),
        "identical level and scripts"
    );
    assert_eq!(alice.level.downloads(), 1);
    assert_eq!(server.uploads(), 1);
}

#[test]
fn a_client_that_never_loads_the_level_is_never_spawned() {
    let mut server = server();
    let mut lazy = Client::join(&server, "Lazy", LevelCache::default());
    lazy.answer = false;
    until(&mut server, &mut [&mut lazy], 300, |_, c| {
        c[0].net.player().is_some()
    });
    for _ in 0..30 {
        server.tick().unwrap();
        lazy.frame();
    }
    assert_eq!(server.players().len(), 1, "connected…");
    let player = lazy.player();
    assert!(!spawned(&mut server, player), "…but not in the world");
}

#[test]
fn a_cached_level_is_not_downloaded_again() {
    let dir = tempfile::tempdir().unwrap();
    let cache = || LevelCache::new(Some(dir.path().to_path_buf()));
    let mut server = server();

    let mut first = Client::join(&server, "First", cache());
    until(&mut server, &mut [&mut first], 300, |s, c| {
        c[0].net.player().is_some_and(|p| spawned(s, p))
    });
    assert_eq!((first.level.downloads(), server.uploads()), (1, 1));

    let mut second = Client::join(&server, "Second", cache());
    until(&mut server, &mut [&mut first, &mut second], 300, |s, c| {
        c[1].net.player().is_some_and(|p| spawned(s, p))
    });
    assert_eq!(second.loads, ["tutorial"]);
    assert_eq!(second.level.downloads(), 0, "served from the cache");
    assert_eq!(server.uploads(), 1, "nothing more was sent");
}

#[test]
fn changing_level_reloads_every_client_and_respawns_players() {
    let mut server = server();
    let mut alice = Client::join(&server, "Alice", LevelCache::default());
    let mut bob = Client::join(&server, "Bob", LevelCache::default());
    until(&mut server, &mut [&mut alice, &mut bob], 300, |s, c| {
        c.iter()
            .all(|c| c.net.player().is_some_and(|p| spawned(s, p)))
    });
    let first_offer = server.level_offer();

    server.change_level(&arena()).unwrap();
    let offer = server.level_offer();
    assert_eq!(offer.transfer, first_offer.transfer + 1);
    assert_ne!(offer.hash, first_offer.hash);
    let (a, b) = (alice.player(), bob.player());
    assert!(
        !spawned(&mut server, a) && !spawned(&mut server, b),
        "fresh world"
    );

    until(&mut server, &mut [&mut alice, &mut bob], 300, |s, _| {
        spawned(s, a) && spawned(s, b)
    });
    for client in [&alice, &bob] {
        assert_eq!(client.loads, ["tutorial", "arena"]);
        assert_eq!(client.level.level(), Some(&arena()));
    }
    // Players stand on the arena's spawns (2.5 and 5.5 tiles).
    let snapshot = server.sim().snapshot();
    let mut xs: Vec<f32> = [a, b]
        .iter()
        .map(|p| snapshot.player(*p).unwrap().position[0] / 32.0)
        .collect();
    xs.sort_by(f32::total_cmp);
    assert_eq!(xs, [2.5, 5.5]);
}

//! A real server announces itself; a browser sees it, with its player count kept up to date.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use mosswake::server::{ServerConfig, ServerSession, TICK};
use mw_net::{Browser, NetClient};

#[test]
fn a_server_announces_its_game_and_player_count() {
    let levels = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels");
    let level = mosswake::load_level(&levels, "tutorial").unwrap();
    let mut server = ServerSession::new(
        &level,
        ServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            max_players: 3,
        },
    )
    .unwrap();
    let mut browser = Browser::bind(0).unwrap();
    let target = SocketAddr::from(([127, 0, 0, 1], browser.local_port().unwrap()));
    server.announce_to(vec![target], "Ayla's game").unwrap();

    let mut client: Option<NetClient> = None;
    let mut seen_players = Vec::new();
    for tick in 0..400 {
        server.tick().unwrap();
        if let Some(c) = client.as_mut() {
            c.update(TICK).unwrap();
            c.flush().unwrap();
        }
        browser.update(TICK).unwrap();
        if let [game] = browser.games().as_slice() {
            let a = &game.announcement;
            assert_eq!(game.addr, server.local_addr(), "joinable address");
            assert_eq!(
                (a.name.as_str(), a.level.as_str(), a.max_players),
                ("Ayla's game", "tutorial", 3)
            );
            assert!(game.compatible());
            if seen_players.last() != Some(&a.players) {
                seen_players.push(a.players);
            }
        }
        // Once the empty game is visible, a player joins.
        if seen_players == [0] && client.is_none() {
            client = Some(NetClient::connect(server.local_addr(), "Bob").unwrap());
        }
        if seen_players == [0, 1] {
            return;
        }
        if tick % 10 == 0 {
            sleep(Duration::from_millis(1));
        }
    }
    panic!("announcements seen: {seen_players:?}");
}

/// Review of #50: a long game name must not make the game invisible.
#[test]
fn a_long_game_name_is_shortened_not_dropped() {
    let levels = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels");
    let level = mosswake::load_level(&levels, "tutorial").unwrap();
    let mut server = ServerSession::new(
        &level,
        ServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            max_players: 2,
        },
    )
    .unwrap();
    let mut browser = Browser::bind(0).unwrap();
    let target = SocketAddr::from(([127, 0, 0, 1], browser.local_port().unwrap()));
    let long = "La partie du samedi soir chez Mélusine et Ayla, venez nombreux !";
    server.announce_to(vec![target], long).unwrap();
    for _ in 0..20 {
        server.tick().unwrap();
        sleep(Duration::from_millis(1));
        browser.update(TICK).unwrap();
        if let [game] = browser.games().as_slice() {
            assert!(long.starts_with(&game.announcement.name));
            return;
        }
    }
    panic!("the game with a long name was never seen");
}

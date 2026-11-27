//! Discovery over real UDP on 127.0.0.1 (announcers target the browser's port directly,
//! as broadcast may be unavailable on CI). Time is logical: `dt` is passed explicitly.

use std::net::SocketAddr;
use std::thread::sleep;
use std::time::Duration;

use mw_net::discovery::{ANNOUNCE_INTERVAL, GAME_EXPIRY};
use mw_net::{Announcement, Announcer, Browser, PROTOCOL_VERSION};

const DT: Duration = Duration::from_millis(100);

fn announcement(name: &str, port: u16, players: u8, version: u32) -> Announcement {
    Announcement {
        protocol_version: version,
        game_port: port,
        name: name.into(),
        level: "tutorial".into(),
        players,
        max_players: 4,
    }
}

/// Pumps announcers and the browser for `steps` steps of `DT`.
fn pump(browser: &mut Browser, announcers: &mut [(&mut Announcer, &Announcement)], steps: u32) {
    for _ in 0..steps {
        for (announcer, a) in announcers.iter_mut() {
            announcer.update(DT, a).unwrap();
        }
        sleep(Duration::from_millis(2));
        browser.update(DT).unwrap();
    }
}

fn target(browser: &Browser) -> Vec<SocketAddr> {
    vec![SocketAddr::from((
        [127, 0, 0, 1],
        browser.local_port().unwrap(),
    ))]
}

#[test]
fn games_appear_update_and_expire() {
    let mut browser = Browser::bind(0).unwrap();
    let mut alpha = Announcer::to_targets(target(&browser)).unwrap();
    let mut beta = Announcer::to_targets(target(&browser)).unwrap();
    let a = announcement("Alpha", 47800, 1, PROTOCOL_VERSION);
    let b = announcement("Beta", 47900, 4, PROTOCOL_VERSION);
    pump(&mut browser, &mut [(&mut alpha, &a), (&mut beta, &b)], 3);

    let games = browser.games();
    assert_eq!(games.len(), 2, "{games:?}");
    let alpha_game = games
        .iter()
        .find(|g| g.announcement.name == "Alpha")
        .unwrap();
    assert_eq!(
        alpha_game.addr,
        "127.0.0.1:47800".parse().unwrap(),
        "sender IP + game port"
    );
    assert!(alpha_game.compatible() && !alpha_game.full());
    assert!(games
        .iter()
        .find(|g| g.announcement.name == "Beta")
        .unwrap()
        .full());

    // A player joins Alpha: the next announcement updates the list.
    let a2 = Announcement {
        players: 2,
        ..a.clone()
    };
    let steps = (ANNOUNCE_INTERVAL.as_millis() / DT.as_millis()) as u32 + 2;
    pump(
        &mut browser,
        &mut [(&mut alpha, &a2), (&mut beta, &b)],
        steps,
    );
    let alpha_game = browser
        .games()
        .into_iter()
        .find(|g| g.announcement.name == "Alpha")
        .unwrap()
        .clone();
    assert_eq!(alpha_game.announcement.players, 2);

    // Beta stops announcing: it disappears after GAME_EXPIRY; Alpha stays.
    let steps = (GAME_EXPIRY.as_millis() / DT.as_millis()) as u32 + 2;
    pump(&mut browser, &mut [(&mut alpha, &a2)], steps);
    let names: Vec<_> = browser
        .games()
        .iter()
        .map(|g| g.announcement.name.clone())
        .collect();
    assert_eq!(names, ["Alpha"]);
}

#[test]
fn other_protocol_versions_are_listed_as_incompatible() {
    let mut browser = Browser::bind(0).unwrap();
    let mut old = Announcer::to_targets(target(&browser)).unwrap();
    let a = announcement("Old", 47800, 0, PROTOCOL_VERSION.wrapping_sub(1));
    pump(&mut browser, &mut [(&mut old, &a)], 3);
    let games = browser.games();
    assert_eq!(games.len(), 1);
    assert!(!games[0].compatible());
}

#[test]
fn foreign_traffic_on_the_discovery_port_is_ignored() {
    let mut browser = Browser::bind(0).unwrap();
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    for junk in [&b"hello"[..], b"MWLN\xff\xff", &[0u8; 600]] {
        socket.send_to(junk, target(&browser)[0]).unwrap();
    }
    sleep(Duration::from_millis(5));
    browser.update(DT).unwrap();
    assert!(browser.games().is_empty());
}

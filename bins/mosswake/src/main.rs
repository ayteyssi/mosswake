//! Mosswake entry point.
//!
//! ```text
//! mosswake play <level> [--autoplay] [--screenshot <file.png> [--at <seconds>]]
//! mosswake server --headless --level <id> [--port <port>] [--name <name>] [--max-players <n>] [--levels-dir <dir>] [--no-announce]
//! mosswake host --level <id> [--port <port>] [--name <name>] [--max-players <n>] [--levels-dir <dir>] [--no-announce]
//! mosswake lan [--seconds <s>] [--port <port>]
//! ```
//! `server` and `host` announce themselves on the LAN unless `--no-announce`; `lan` lists
//! the games it hears.
//! Still to come: `mosswake join <addr>` (#14), the main menu (#49) and `mosswake edit` (#23).

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use mosswake::host::HostSession;
use mosswake::load_level;
use mosswake::server::{ServerConfig, ServerSession, TICK};
use mw_core::input::PlayerInput;
use mw_net::{ServerNetEvent, DEFAULT_PORT};

const USAGE: &str = "usage:
  mosswake play <level> [--autoplay] [--screenshot <file.png> [--at <seconds>]]
  mosswake server --headless --level <id> [--port <port>] [--name <name>] [--max-players <n>] [--levels-dir <dir>] [--no-announce]
  mosswake host --level <id> [--port <port>] [--name <name>] [--max-players <n>] [--levels-dir <dir>] [--no-announce]
  mosswake lan [--seconds <s>] [--port <port>]";

/// Options shared by `server` and `host`.
struct Options {
    level: String,
    port: u16,
    max_players: u8,
    levels_dir: PathBuf,
    /// Game name on the LAN (and the host's player name).
    name: String,
    announce: bool,
}

fn parse_options(args: &[String], default_name: &str) -> Result<Options> {
    let mut options = Options {
        level: String::new(),
        port: DEFAULT_PORT,
        max_players: 4,
        levels_dir: mosswake::default_levels_dir(),
        name: default_name.into(),
        announce: true,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .with_context(|| format!("{arg} needs a value\n{USAGE}"))
        };
        match arg.as_str() {
            // The server never renders: accepted for clarity and future-proof scripts.
            "--headless" => {}
            "--level" => options.level = value()?.clone(),
            "--port" => options.port = value()?.parse().context("invalid --port")?,
            "--max-players" => {
                options.max_players = value()?.parse().context("invalid --max-players")?
            }
            "--levels-dir" => options.levels_dir = PathBuf::from(value()?),
            "--name" => options.name = value()?.clone(),
            "--no-announce" => options.announce = false,
            other => bail!("unknown option `{other}`\n{USAGE}"),
        }
    }
    if options.level.is_empty() {
        bail!("--level is required\n{USAGE}");
    }
    Ok(options)
}

fn config(options: &Options) -> ServerConfig {
    ServerConfig {
        bind: SocketAddr::from((Ipv4Addr::UNSPECIFIED, options.port)),
        max_players: options.max_players,
    }
}

fn log_events(events: &[ServerNetEvent]) {
    for event in events {
        match event {
            ServerNetEvent::PlayerConnected(info) => {
                println!("+ {} joined as player {}", info.name, info.player.0)
            }
            ServerNetEvent::PlayerDisconnected { player, reason } => {
                println!("- player {} left ({reason:?})", player.0)
            }
            ServerNetEvent::ClientRejected { addr, reason } => {
                println!("x client {addr:?} refused: {reason:?}")
            }
        }
    }
}

/// Calls `tick` at 60 Hz of wall-clock time, forever. Real time is only used here to pace
/// the loop; the simulation itself advances by fixed ticks. After a long stall (debugger,
/// suspended laptop) it resumes instead of running hundreds of ticks to catch up.
fn run_at_tick_rate(mut tick: impl FnMut() -> Result<()>) -> Result<()> {
    const MAX_LAG: Duration = Duration::from_millis(250);
    let mut next = Instant::now();
    loop {
        tick()?;
        next += TICK;
        let now = Instant::now();
        if next > now {
            sleep(next - now);
        } else if now - next > MAX_LAG {
            next = now;
        }
    }
}

fn server(args: &[String]) -> Result<()> {
    let options = parse_options(args, "Mosswake server")?;
    let bundle = load_level(&options.levels_dir, &options.level)?;
    let level = &bundle.level;
    let mut session = ServerSession::new(&bundle, config(&options))?;
    announce(&options, |name| session.announce_on_lan(name));
    println!(
        "mosswake server: level `{}` on {} (protocol v{}, up to {} players)",
        level.id,
        session.local_addr(),
        mw_net::PROTOCOL_VERSION,
        options.max_players
    );
    run_at_tick_rate(|| {
        log_events(&session.tick()?);
        Ok(())
    })
}

fn host(args: &[String]) -> Result<()> {
    let options = parse_options(args, "Host")?;
    let bundle = load_level(&options.levels_dir, &options.level)?;
    let level = &bundle.level;
    let mut session = HostSession::new(&bundle, config(&options), &options.name)?;
    announce(&options, |name| session.server().announce_on_lan(name));
    println!(
        "mosswake host: level `{}` on {} — host mode is not rendered yet (#14; solo \
         rendering: `mosswake play`), the local player stands still",
        level.id,
        session.server().local_addr()
    );
    let mut last_state = None;
    run_at_tick_rate(|| {
        log_events(&session.frame(PlayerInput::default())?);
        let state = session.client_state().clone();
        if last_state.as_ref() != Some(&state) {
            println!("local client: {state:?}");
            last_state = Some(state);
        }
        Ok(())
    })
}

/// Turns LAN announcements on unless `--no-announce` (a failure only warns).
fn announce(options: &Options, enable: impl FnOnce(&str) -> Result<()>) {
    if !options.announce {
        return;
    }
    match enable(&options.name) {
        Ok(()) => println!("announced on the LAN as \"{}\"", options.name),
        Err(e) => eprintln!("warning: LAN announcement disabled: {e:#}"),
    }
}

/// `mosswake lan`: listens for announcements and prints the games heard.
fn lan(args: &[String]) -> Result<()> {
    let mut seconds = 3.0_f32;
    let mut port = mw_net::DISCOVERY_PORT;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .with_context(|| format!("{arg} needs a value\n{USAGE}"))
        };
        match arg.as_str() {
            "--seconds" => seconds = value()?.parse().context("invalid --seconds")?,
            "--port" => port = value()?.parse().context("invalid --port")?,
            other => bail!("unknown option `{other}`\n{USAGE}"),
        }
    }
    let mut browser = mw_net::Browser::bind(port).with_context(|| {
        format!("cannot listen on discovery port {port} (another browser running?)")
    })?;
    let step = Duration::from_millis(100);
    let steps = (seconds.max(0.1) / step.as_secs_f32()).ceil() as u32;
    for _ in 0..steps {
        sleep(step);
        browser.update(step)?;
    }
    let games = browser.games();
    if games.is_empty() {
        println!("no game found on the LAN (port {port}) in {seconds} s");
    }
    for game in games {
        let a = &game.announcement;
        let status = match (game.compatible(), game.full()) {
            (false, _) => format!("incompatible (protocol v{})", a.protocol_version),
            (true, true) => "full".to_owned(),
            (true, false) => "open".to_owned(),
        };
        println!(
            "{:<22} {:<24} level {:<16} {}/{} players  {status}",
            game.addr, a.name, a.level, a.players, a.max_players
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("server") => server(&args[1..]),
        Some("host") => host(&args[1..]),
        Some("lan") => lan(&args[1..]),
        Some("play") => mosswake::play::run(&args[1..]),
        Some("--version") => {
            println!("mosswake {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => bail!("{USAGE}"),
    }
}

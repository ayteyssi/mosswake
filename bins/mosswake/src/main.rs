//! Mosswake entry point.
//!
//! ```text
//! mosswake server --headless --level <id> [--port <port>] [--max-players <n>] [--levels-dir <dir>]
//! mosswake host --level <id> [--port <port>] [--name <name>] [--max-players <n>] [--levels-dir <dir>]
//! ```
//! Still to come: `mosswake join <addr>` (#14, #15) and `mosswake edit <level>` (#23).

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
  mosswake server --headless --level <id> [--port <port>] [--max-players <n>] [--levels-dir <dir>]
  mosswake host --level <id> [--port <port>] [--name <name>] [--max-players <n>] [--levels-dir <dir>]";

/// Options shared by `server` and `host`.
struct Options {
    level: String,
    port: u16,
    max_players: u8,
    levels_dir: PathBuf,
    name: String,
}

fn parse_options(args: &[String], allow_name: bool) -> Result<Options> {
    let mut options = Options {
        level: String::new(),
        port: DEFAULT_PORT,
        max_players: 4,
        levels_dir: PathBuf::from("levels"),
        name: "Host".into(),
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
            "--name" if allow_name => options.name = value()?.clone(),
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
    let options = parse_options(args, false)?;
    let bundle = load_level(&options.levels_dir, &options.level)?;
    let level = &bundle.level;
    let mut session = ServerSession::new(&bundle, config(&options))?;
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
    let options = parse_options(args, true)?;
    let bundle = load_level(&options.levels_dir, &options.level)?;
    let level = &bundle.level;
    let mut session = HostSession::new(&bundle, config(&options), &options.name)?;
    println!(
        "mosswake host: level `{}` on {} — rendering is not implemented yet (#10), \
         the local player stands still",
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

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("server") => server(&args[1..]),
        Some("host") => host(&args[1..]),
        Some("--version") => {
            println!("mosswake {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => bail!("{USAGE}"),
    }
}

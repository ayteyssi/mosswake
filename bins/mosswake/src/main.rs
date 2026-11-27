//! Mosswake entry point.
//!
//! ```text
//! mosswake play <level> [--autoplay] [--screenshot <file.png> [--at <seconds>]]
//! mosswake join <addr[:port]> [--name <name>] [--demo] [--screenshot <file.png> [--at <s>]]
//! mosswake server --headless --level <id> [--port <port>] [--max-players <n>] [--levels-dir <dir>]
//! mosswake host --level <id> [--port <port>] [--name <name>] [--max-players <n>] [--levels-dir <dir>]
//! ```
//! Still to come: `mosswake edit <level>` (#23) and the main menu (#49).

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
  mosswake join <addr[:port]> [--name <name>] [--demo] [--screenshot <file.png> [--at <s>]]
  mosswake proxy --to <addr[:port]> [--port <p>] [--latency <ms>] [--jitter <ms>] [--loss <%>]
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
        levels_dir: mosswake::default_levels_dir(),
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

/// `mosswake proxy`: forwards a game through simulated latency, jitter and loss.
fn proxy(args: &[String]) -> Result<()> {
    let mut to: Option<SocketAddr> = None;
    let mut port = 47810u16;
    let mut conditions = mw_net::Conditions::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .with_context(|| format!("{arg} needs a value\n{USAGE}"))
        };
        match arg.as_str() {
            "--to" => {
                let target = value()?;
                let with_port = if target.contains(':') {
                    target.clone()
                } else {
                    format!("{target}:{DEFAULT_PORT}")
                };
                to = Some(
                    with_port
                        .parse()
                        .with_context(|| format!("invalid address `{target}`"))?,
                );
            }
            "--port" => port = value()?.parse().context("invalid --port")?,
            "--latency" => {
                conditions.latency = Duration::from_millis(
                    value()?.parse().context("--latency expects milliseconds")?,
                )
            }
            "--jitter" => {
                conditions.jitter = Duration::from_millis(
                    value()?.parse().context("--jitter expects milliseconds")?,
                )
            }
            "--loss" => {
                conditions.loss = value()?
                    .parse::<f32>()
                    .context("--loss expects a percentage")?
                    / 100.0
            }
            other => bail!("unknown option `{other}`\n{USAGE}"),
        }
    }
    let to = to.with_context(|| format!("--to is required\n{USAGE}"))?;
    let mut link = mw_net::LinkConditioner::bind(
        SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)),
        to,
        conditions,
    )
    .with_context(|| format!("cannot listen on port {port}"))?;
    println!(
        "mosswake proxy: :{port} -> {to} (latency {:?} each way, jitter {:?}, loss {:.1} %)",
        conditions.latency,
        conditions.jitter,
        conditions.loss * 100.0
    );
    let step = Duration::from_millis(1);
    let mut last = Instant::now();
    loop {
        sleep(step);
        let now = Instant::now();
        link.update(now - last)?;
        last = now;
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("server") => server(&args[1..]),
        Some("host") => host(&args[1..]),
        Some("play") => mosswake::play::run(&args[1..]),
        Some("join") => mosswake::join::run(&args[1..]),
        Some("proxy") => proxy(&args[1..]),
        Some("--version") => {
            println!("mosswake {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => bail!("{USAGE}"),
    }
}

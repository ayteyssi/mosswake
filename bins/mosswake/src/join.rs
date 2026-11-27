//! `mosswake join <addr> [--name <name>] [--demo] [--screenshot <file.png> [--at <s>]]`:
//! play on a remote server with rendering.
//!
//! [`ClientSession`] does the networking, prediction and interpolation; every tick its
//! [`crate::client::ViewBody`]s are mirrored into plain entities (`Player`, `Position`,
//! `PlayerState`, `Facing`, `MobilityState`) that `mw_render` draws exactly like in solo play.

use std::collections::BTreeMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
// Explicit imports: Bevy's prelude also has a `context` extension method.
use bevy::prelude::{
    default, App, AssetPlugin, Commands, DefaultPlugins, Entity, Fixed, FixedUpdate,
    IntoScheduleConfigs, NonSendMut, PluginGroup, Query, Res, ResMut, Resource, Time, Update,
    Window, WindowPlugin,
};
use bevy::window::WindowResolution;
use mw_core::input::{buttons, PlayerId, PlayerInput};
use mw_net::{LevelCache, DEFAULT_PORT};
use mw_render::{CameraTarget, ClientRenderPlugin, LocalInputPlugin};
use mw_sim::{LoadedLevel, Player, Position, SimId, TickInputs, TICK_HZ};

use crate::client::ClientSession;
use crate::play::{take_screenshot_then_exit, ScreenshotPlan};

pub const USAGE: &str =
    "mosswake join <addr[:port]> [--name <name>] [--demo] [--screenshot <file.png> [--at <s>]]";

/// Slot the keyboard / gamepad input is written to before being handed to the session.
const LOCAL_SLOT: PlayerId = PlayerId(0);

struct JoinArgs {
    addr: SocketAddr,
    name: String,
    demo: bool,
    screenshot: Option<(PathBuf, f32)>,
}

fn parse_args(args: &[String]) -> Result<JoinArgs> {
    let mut it = args.iter();
    let target = it.next().with_context(|| format!("usage: {USAGE}"))?;
    let with_port = if target.contains(':') {
        target.clone()
    } else {
        format!("{target}:{DEFAULT_PORT}")
    };
    let addr = with_port
        .to_socket_addrs()
        .with_context(|| format!("cannot resolve `{target}`"))?
        .next()
        .with_context(|| format!("no address for `{target}`"))?;
    let mut join = JoinArgs {
        addr,
        name: "Player".into(),
        demo: false,
        screenshot: None,
    };
    let mut at = 5.0;
    let mut shot = None;
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .with_context(|| format!("{arg} needs a value\nusage: {USAGE}"))
        };
        match arg.as_str() {
            "--name" => join.name = value()?.clone(),
            "--demo" => join.demo = true,
            "--screenshot" => shot = Some(PathBuf::from(value()?)),
            "--at" => at = value()?.parse().context("--at expects seconds")?,
            other => bail!("unknown option `{other}`\nusage: {USAGE}"),
        }
    }
    join.screenshot = shot.map(|p| (p, at));
    Ok(join)
}

/// Mirror entities of the session's view, by simulation id.
#[derive(Resource, Default)]
struct Mirrors(BTreeMap<SimId, Entity>);

/// Runs `mosswake join` with the arguments that follow `join`.
pub fn run(args: &[String]) -> Result<()> {
    let join = parse_args(args)?;
    let root = crate::find_root()?;
    let assets = root.join("assets");
    let cache = LevelCache::new(dirs_cache());
    let session = ClientSession::connect(join.addr, &join.name, cache)?;
    println!(
        "mosswake join: connecting to {} as \"{}\"…",
        join.addr, join.name
    );

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: assets.to_string_lossy().into_owned(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: format!("Mosswake — {}", join.addr),
                    resolution: WindowResolution::new(1280, 720),
                    ..default()
                }),
                ..default()
            }),
    )
    .add_plugins(ClientRenderPlugin::new(&assets, LOCAL_SLOT)?)
    .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
    .init_resource::<TickInputs>()
    .init_resource::<Mirrors>()
    .insert_non_send(session)
    .add_systems(FixedUpdate, client_tick);
    if join.demo {
        app.add_systems(FixedUpdate, demo_input.before(client_tick));
    } else {
        app.add_plugins(LocalInputPlugin { player: LOCAL_SLOT });
    }
    if let Some((path, at)) = join.screenshot {
        app.insert_resource(ScreenshotPlan {
            path,
            at: Duration::from_secs_f32(at),
            taken: None,
        })
        .add_systems(Update, take_screenshot_then_exit);
    }
    app.run();
    Ok(())
}

/// Per-user level cache: `$XDG_CACHE_HOME/mosswake/levels` or `~/.cache/mosswake/levels`.
fn dirs_cache() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("mosswake").join("levels"))
}

/// Scripted input for demos and screenshots: run right, jump regularly.
fn demo_input(time: Res<Time>, mut inputs: ResMut<TickInputs>) {
    let t = time.elapsed_secs();
    let jump = t % 1.5 < 0.4;
    inputs.0.insert(
        LOCAL_SLOT,
        PlayerInput {
            axis_x: if t < 2.0 { 0 } else { 127 },
            axis_y: 0,
            buttons: if jump { buttons::JUMP } else { 0 },
        },
    );
}

/// One network tick, then mirror what the session wants drawn.
fn client_tick(
    mut commands: Commands,
    mut session: NonSendMut<ClientSession>,
    inputs: Res<TickInputs>,
    mut mirrors: ResMut<Mirrors>,
    mut bodies: Query<(
        &mut Position,
        &mut mw_sim::PlayerState,
        &mut mw_sim::Facing,
        &mut mw_sim::mechanics::MobilityState,
    )>,
) {
    let input = inputs.0.get(&LOCAL_SLOT).copied().unwrap_or_default();
    match session.frame(input) {
        Ok(true) => {
            if let (Some(bundle), Some(rules)) = (session.level(), session.rules()) {
                commands.insert_resource(LoadedLevel(bundle.level.clone()));
                commands.insert_resource(rules.clone());
                println!("level `{}` loaded", bundle.level.id);
            }
            for (_, entity) in std::mem::take(&mut mirrors.0) {
                commands.entity(entity).despawn();
            }
        }
        Ok(false) => {}
        Err(e) => {
            eprintln!("disconnected: {e:#}");
            commands.queue(|world: &mut bevy::prelude::World| {
                world.write_message(bevy::prelude::AppExit::error());
            });
            return;
        }
    }
    if let Some(me) = session.player() {
        commands.insert_resource(CameraTarget(me));
    }
    let view = session.view();
    let alive: Vec<SimId> = view.iter().map(|b| b.id).collect();
    mirrors.0.retain(|id, entity| {
        let keep = alive.contains(id);
        if !keep {
            commands.entity(*entity).despawn();
        }
        keep
    });
    for body in view {
        let Some(player) = body.player else {
            continue;
        };
        match mirrors
            .0
            .get(&body.id)
            .and_then(|e| bodies.get_mut(*e).ok())
        {
            Some((mut position, mut state, mut facing, mut mobility)) => {
                position.0 = body.position;
                *state = body.state;
                *facing = body.facing;
                *mobility = body.mobility;
            }
            None => {
                let entity = commands
                    .spawn((
                        body.id,
                        Player(player),
                        Position(body.position),
                        body.state,
                        body.facing,
                        body.mobility,
                    ))
                    .id();
                mirrors.0.insert(body.id, entity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_addresses_and_options() {
        let j = parse_args(&args(&["127.0.0.1", "--name", "Ayla", "--demo"])).unwrap();
        assert_eq!(j.addr, SocketAddr::from(([127, 0, 0, 1], DEFAULT_PORT)));
        assert_eq!(j.name, "Ayla");
        assert!(j.demo);
        let j = parse_args(&args(&[
            "127.0.0.1:5000",
            "--screenshot",
            "s.png",
            "--at",
            "2",
        ]))
        .unwrap();
        assert_eq!(j.addr.port(), 5000);
        assert_eq!(j.screenshot, Some((PathBuf::from("s.png"), 2.0)));
        assert!(parse_args(&args(&[])).is_err());
        assert!(parse_args(&args(&["127.0.0.1", "--fly"])).is_err());
    }
}

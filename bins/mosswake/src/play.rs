//! `mosswake play <level> [--autoplay] [--screenshot <file.png> [--at <seconds>]]`: solo play
//! with the local simulation, rendering and keyboard / gamepad input.
//!
//! `<level>` is a level id (`levels/<id>/`) or a path to a level directory.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
// Explicit imports: Bevy's prelude also has a `context` extension method, which would be
// ambiguous with `anyhow::Context`.
use bevy::prelude::{
    default, App, AppExit, AssetPlugin, Commands, DefaultPlugins, FixedPreUpdate, Local,
    MessageWriter, PluginGroup, Query, Res, ResMut, Resource, Time, Update, Window, WindowPlugin,
};
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::window::WindowResolution;
use mw_core::input::{buttons, PlayerId, PlayerInput};
use mw_level::LevelBundle;
use mw_render::{ClientRenderPlugin, LocalInputPlugin};
use mw_sim::{
    load_level, spawn_player, CollisionGrid, Contacts, Player, Position, SimPlugin, TickInputs,
};

const USAGE: &str =
    "usage: mosswake play <level> [--autoplay] [--screenshot <file.png> [--at <seconds>]]";

/// The local player in solo play.
const LOCAL: PlayerId = PlayerId(0);

struct PlayArgs {
    level: String,
    autoplay: bool,
    screenshot: Option<(PathBuf, f32)>,
}

/// Parses the arguments after `play`.
fn parse_args(args: &[String]) -> Result<PlayArgs> {
    let mut it = args.iter();
    let mut play = PlayArgs {
        level: String::new(),
        autoplay: false,
        screenshot: None,
    };
    let mut at = 3.0;
    let mut shot = None;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--autoplay" => play.autoplay = true,
            "--screenshot" => shot = Some(PathBuf::from(it.next().context(USAGE)?)),
            "--at" => {
                at = it
                    .next()
                    .context(USAGE)?
                    .parse()
                    .context("--at expects seconds")?
            }
            flag if flag.starts_with("--") => bail!("unknown option `{flag}`\n{USAGE}"),
            level if play.level.is_empty() => play.level = level.to_owned(),
            extra => bail!("unexpected argument `{extra}`\n{USAGE}"),
        }
    }
    if play.level.is_empty() {
        bail!("{USAGE}");
    }
    play.screenshot = shot.map(|path| (path, at));
    Ok(play)
}

/// Runs `mosswake play` with the arguments that follow `play`.
pub fn run(args: &[String]) -> Result<()> {
    let play = parse_args(args)?;
    let root = crate::find_root()?;
    let level_dir = if Path::new(&play.level).is_dir() {
        PathBuf::from(&play.level)
    } else {
        root.join("levels").join(&play.level)
    };
    let bundle = LevelBundle::load(&level_dir)
        .with_context(|| format!("loading level {}", level_dir.display()))?;
    let assets = root.join("assets");

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: assets.to_string_lossy().into_owned(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: format!("Mosswake — {}", bundle.level.name),
                    resolution: WindowResolution::new(1280, 720),
                    ..default()
                }),
                ..default()
            }),
    )
    .add_plugins(SimPlugin)
    .add_plugins(ClientRenderPlugin::new(&assets, LOCAL)?);

    if play.autoplay {
        app.add_systems(FixedPreUpdate, autoplay);
    } else {
        app.add_plugins(LocalInputPlugin { player: LOCAL });
    }
    if let Some((path, at)) = play.screenshot {
        app.insert_resource(ScreenshotPlan {
            path,
            at: Duration::from_secs_f32(at),
            taken: None,
        })
        .add_systems(Update, take_screenshot_then_exit);
    }

    load_level(app.world_mut(), &bundle.level)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("level `{}` is not playable", bundle.level.id))?;
    mw_sim::scripting::load_scripts(app.world_mut(), &bundle)
        .map_err(|errors| anyhow::anyhow!("{errors:#?}"))
        .with_context(|| format!("scripts of level `{}` failed to load", bundle.level.id))?;
    spawn_player(app.world_mut(), LOCAL);
    app.run();
    Ok(())
}

/// Scripted inputs for demos and screenshots: stand for a second, then run right and jump
/// whenever a gap or a wall is ahead (reading the collision grid, like a player would look),
/// plus every 1.5 s to show the jump animation, and a dash every 2 s.
fn autoplay(
    time: Res<Time>,
    grid: Res<CollisionGrid>,
    players: Query<(&Player, &Position, &Contacts)>,
    mut inputs: ResMut<TickInputs>,
    mut hold: Local<f32>,
) {
    let t = time.elapsed_secs();
    let Some((_, position, contacts)) = players.iter().find(|(p, _, _)| p.0 == LOCAL) else {
        return;
    };
    let ts = grid.tile_size();
    let (cx, cy) = (
        ((position.0.x + ts) / ts).floor() as i32,
        (position.0.y / ts).floor() as i32,
    );
    let obstacle = !grid.is_solid(cx, cy - 1) || grid.is_solid(cx, cy);
    let show_off = (t % 1.5) < 0.05;
    if t >= 1.0 && contacts.grounded && (obstacle || show_off) && *hold <= 0.0 {
        *hold = 0.45;
    }
    let jump = *hold > 0.0;
    *hold -= time.delta_secs();
    // A dash every 2 s (only does something where the level activates the mechanic).
    let dash = t >= 1.0 && t % 2.0 >= 1.5 && t % 2.0 < 1.55;
    let mut pressed = 0;
    if jump {
        pressed |= buttons::JUMP;
    }
    if dash {
        pressed |= buttons::DASH;
    }
    inputs.0.insert(
        LOCAL,
        PlayerInput {
            axis_x: if t < 1.0 { 0 } else { 127 },
            axis_y: 0,
            buttons: pressed,
        },
    );
}

/// `--screenshot` support shared by `play` and `join`: capture at `at`, then quit.
#[derive(Resource)]
pub(crate) struct ScreenshotPlan {
    pub(crate) path: PathBuf,
    pub(crate) at: Duration,
    pub(crate) taken: Option<Duration>,
}

pub(crate) fn take_screenshot_then_exit(
    mut commands: Commands,
    time: Res<Time>,
    mut plan: ResMut<ScreenshotPlan>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed();
    match plan.taken {
        None if now >= plan.at => {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(plan.path.clone()));
            plan.taken = Some(now);
        }
        // Leave time for the capture to be written before quitting.
        Some(taken) if now >= taken + Duration::from_secs(1) => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_play() {
        let p = parse_args(&args(&[
            "tutorial",
            "--autoplay",
            "--screenshot",
            "a.png",
            "--at",
            "2.5",
        ]))
        .unwrap();
        assert_eq!(p.level, "tutorial");
        assert!(p.autoplay);
        assert_eq!(p.screenshot, Some((PathBuf::from("a.png"), 2.5)));
    }

    #[test]
    fn rejects_bad_usage() {
        assert!(parse_args(&args(&[])).is_err());
        assert!(parse_args(&args(&["a", "b"])).is_err());
        assert!(parse_args(&args(&["a", "--fly"])).is_err());
        assert!(parse_args(&args(&["a", "--at"])).is_err());
    }

    #[test]
    fn finds_the_repository_root() {
        let root = crate::find_root().unwrap();
        assert!(root.join("levels/tutorial/level.json").is_file());
    }
}

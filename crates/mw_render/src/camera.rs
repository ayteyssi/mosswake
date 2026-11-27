//! 2D camera that follows the local player smoothly and stays inside the level.

use bevy::camera::ScalingMode;
use bevy::prelude::*;
use mw_core::input::PlayerId;
use mw_sim::{LoadedLevel, Player};

use crate::layout::clamp_camera;

/// Level tiles visible vertically.
pub const VIEW_HEIGHT_TILES: f32 = 15.0;

/// Follow stiffness (1/s) per axis: higher reacts faster. Frame-rate independent. Vertical
/// follow is softer so that jumps do not swing the view.
pub const FOLLOW_RATE: Vec2 = Vec2::new(6.0, 2.5);

/// The player the camera follows.
#[derive(Resource, Debug, Clone, Copy)]
pub struct CameraTarget(pub PlayerId);

/// Look slightly above the feet.
const LOOK_UP_TILES: f32 = 1.0;

pub(crate) fn spawn_camera(
    mut commands: Commands,
    level: Res<LoadedLevel>,
    existing: Query<(), With<Camera2d>>,
) {
    if !existing.is_empty() {
        return; // a later level reuses the camera
    }
    let ts = level.0.tile_size as f32;
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: VIEW_HEIGHT_TILES * ts,
            },
            ..OrthographicProjection::default_2d()
        }),
        Transform::from_xyz(0.0, 0.0, 100.0),
    ));
}

pub(crate) fn follow(
    time: Res<Time>,
    level: Res<LoadedLevel>,
    target: Res<CameraTarget>,
    players: Query<(&Player, &Transform), Without<Camera2d>>,
    mut camera: Query<(&mut Transform, &Projection), With<Camera2d>>,
    mut snapped: Local<bool>,
) {
    let Ok((mut transform, projection)) = camera.single_mut() else {
        return;
    };
    let Some((_, player)) = players.iter().find(|(p, _)| p.0 == target.0) else {
        return;
    };
    let ts = level.0.tile_size as f32;
    let level_size = Vec2::new(level.0.size.w as f32, level.0.size.h as f32) * ts;
    let view = match projection {
        Projection::Orthographic(o) if o.area.width() > 0.0 => o.area.size(),
        _ => Vec2::new(VIEW_HEIGHT_TILES * ts * 16.0 / 9.0, VIEW_HEIGHT_TILES * ts),
    };
    let goal = clamp_camera(
        player.translation.truncate() + Vec2::new(0.0, LOOK_UP_TILES * ts),
        view,
        level_size,
    );
    let current = transform.translation.truncate();
    let next = if *snapped {
        // Exponential smoothing, independent of the frame rate.
        let blend = Vec2::ONE - (-FOLLOW_RATE * time.delta_secs()).exp();
        current + (goal - current) * blend
    } else {
        *snapped = true;
        goal
    };
    transform.translation.x = next.x;
    transform.translation.y = next.y;
}

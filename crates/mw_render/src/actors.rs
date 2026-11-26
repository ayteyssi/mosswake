//! Players on screen: animated wizard chosen from [`PlayerState`], flipped by [`Facing`],
//! drawn at a position interpolated between the last two simulation ticks.

use bevy::prelude::*;
use mw_sim::{Facing, LoadedLevel, Player, PlayerState, Position};

use crate::level::z;
use crate::sprites::{Animator, SpriteLibrary};
use crate::textures::TextureScales;

/// Sprite of every player.
pub const PLAYER_SPRITE: &str = "blue_wizard";

/// Position at the start of the last simulation tick, for interpolation.
#[derive(Component, Debug, Clone, Copy)]
pub struct PrevPosition(pub Vec2);

/// Animation playing for a player state.
pub fn player_animation(state: PlayerState) -> &'static str {
    match state {
        PlayerState::Idle => "idle",
        PlayerState::Run => "walk",
        // Same clip for both: it plays once and holds its last frame while falling.
        PlayerState::Jump | PlayerState::Fall => "jump",
    }
}

pub(crate) fn attach_player_visuals(
    mut commands: Commands,
    level: Res<LoadedLevel>,
    server: Res<AssetServer>,
    mut library: ResMut<SpriteLibrary>,
    mut scales: ResMut<TextureScales>,
    players: Query<(Entity, &Position), Added<Player>>,
) {
    let ts = level.0.tile_size as f32;
    for (entity, position) in &players {
        let Some(bundle) = library.animated(&server, &mut scales, PLAYER_SPRITE, "idle", ts, 0.0)
        else {
            warn!("sprite `{PLAYER_SPRITE}` missing from the manifest");
            continue;
        };
        commands.entity(entity).insert((
            bundle,
            PrevPosition(position.0),
            Transform::from_translation(position.0.extend(z::PLAYERS)),
        ));
    }
}

pub(crate) fn animate_players(
    server: Res<AssetServer>,
    mut library: ResMut<SpriteLibrary>,
    mut scales: ResMut<TextureScales>,
    mut players: Query<(&PlayerState, &Facing, &mut Animator, &mut Sprite), With<Player>>,
) {
    for (state, facing, mut animator, mut sprite) in &mut players {
        animator.play(&mut library, &server, &mut scales, player_animation(*state));
        // The wizard art faces right.
        let flip = *facing == Facing::Left;
        if sprite.flip_x != flip {
            sprite.flip_x = flip;
        }
    }
}

/// Runs in `FixedPreUpdate`: remember where bodies were before the tick moves them.
pub(crate) fn remember_positions(mut query: Query<(&Position, &mut PrevPosition)>) {
    for (position, mut prev) in &mut query {
        prev.0 = position.0;
    }
}

pub(crate) fn interpolate(
    time: Res<Time<Fixed>>,
    mut query: Query<(&Position, &PrevPosition, &mut Transform)>,
) {
    let t = time.overstep_fraction();
    for (position, prev, mut transform) in &mut query {
        let at = prev.0.lerp(position.0, t);
        transform.translation.x = at.x;
        transform.translation.y = at.y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_has_a_wizard_animation() {
        let manifest = mw_core::assets::AnimationManifest::load(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets"),
        )
        .unwrap();
        let wizard = &manifest.sprites[PLAYER_SPRITE];
        for state in [
            PlayerState::Idle,
            PlayerState::Run,
            PlayerState::Jump,
            PlayerState::Fall,
        ] {
            assert!(
                wizard.animations.contains_key(player_animation(state)),
                "{state:?}"
            );
        }
    }
}

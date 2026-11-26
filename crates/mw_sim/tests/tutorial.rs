//! The tutorial must be playable with the default rules: every hop of its route is played
//! here with scripted inputs (stand at the start, hold the direction and jump, land).

use std::path::PathBuf;

use bevy::math::Vec2;
use mw_core::input::{buttons, PlayerId, PlayerInput};
use mw_level::LevelBundle;
use mw_sim::{SimHarness, TickInputs};

struct Hop {
    name: &'static str,
    /// Start: feet x and the top of the surface stood on, in tiles.
    from: (f32, f32),
    /// -1 left, 1 right.
    dir: i8,
    /// Expected landing surface top and its x range `[min, max)`, in tiles.
    land_top: f32,
    land_x: (f32, f32),
}

const ROUTE: &[Hop] = &[
    Hop {
        name: "ground -> P1",
        from: (7.0, 2.0),
        dir: 1,
        land_top: 5.0,
        land_x: (9.0, 14.0),
    },
    Hop {
        name: "P1 -> P2",
        from: (12.5, 5.0),
        dir: 1,
        land_top: 8.0,
        land_x: (16.0, 21.0),
    },
    Hop {
        name: "P2 -> P3 over the pit",
        from: (20.5, 8.0),
        dir: 1,
        land_top: 6.0,
        land_x: (25.0, 31.0),
    },
    Hop {
        name: "ground -> step",
        from: (37.0, 2.0),
        dir: 1,
        land_top: 3.0,
        land_x: (40.0, 59.0),
    },
    Hop {
        name: "step -> P4",
        from: (43.0, 3.0),
        dir: 1,
        land_top: 6.0,
        land_x: (46.0, 51.0),
    },
];

#[test]
fn every_hop_of_the_tutorial_route_is_reachable() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels/tutorial");
    let level = LevelBundle::load(dir).expect("tutorial loads").level;
    let ts = level.tile_size as f32;
    for hop in ROUTE {
        let mut sim = SimHarness::new(&level).unwrap();
        let id = sim.spawn_player(PlayerId(0));
        sim.teleport(id, Vec2::new(hop.from.0, hop.from.1) * ts);
        sim.step(5, &TickInputs::default());
        assert!(
            sim.body(id).contacts.grounded,
            "{}: start is on ground",
            hop.name
        );

        let mut inputs = TickInputs::default();
        inputs.0.insert(
            PlayerId(0),
            PlayerInput {
                axis_x: hop.dir * 127,
                axis_y: 0,
                buttons: buttons::JUMP,
            },
        );
        let mut airborne = false;
        let landed = (0..180).find_map(|_| {
            sim.step(1, &inputs);
            let s = sim.body(id);
            airborne |= !s.contacts.grounded;
            (airborne && s.contacts.grounded).then_some(s.position / ts)
        });
        let at = landed.unwrap_or_else(|| panic!("{}: never landed", hop.name));
        assert_eq!(at.y, hop.land_top, "{}: landed at {at:?}", hop.name);
        assert!(
            (hop.land_x.0..hop.land_x.1).contains(&at.x),
            "{}: landed at {at:?}, expected x in {:?}",
            hop.name,
            hop.land_x
        );
    }
}

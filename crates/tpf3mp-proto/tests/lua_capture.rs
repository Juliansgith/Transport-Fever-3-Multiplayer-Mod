//! The Lua mod's road and track capture writes exactly the payload the
//! action schema decodes: `tests/lua/road_capture.lua` runs the mod's own
//! modules against a world of tables, and the payloads it returns must
//! decode to the actions below. Lua 5.1 here (the workspace's vendored
//! Lua); the mod targets 5.2 and keeps to what both run.
//!
//! The modules are compiled into the test binary, so the test does not
//! depend on the working directory.

#![allow(clippy::unwrap_used)]

use mlua::{Lua, LuaString, Table};
use tpf3mp_proto::{
    BoundedVec, Payload, Text,
    action::{
        Action, EdgeEnds, EdgeRef, Link, Network, Polyline, Pos, Resolve, RoadBuild, Structure,
        Tangent, TrackBuild, Tram, Vertex,
    },
};

macro_rules! modules {
    ($($name:literal),* $(,)?) => {
        [$(($name, include_str!(concat!("../../../mod/tpf3mp_1/content/scripts/tpf3mp/", $name, ".lua")))),*]
    };
}

/// Every module of the mod, as `require "tpf3mp.<name>"` finds it.
const MODULES: [(&str, &str); 5] = modules!("fixed", "wire", "geom", "roads", "engine");

const TEST: &str = include_str!("lua/road_capture.lua");

/// Runs the Lua test and returns the payloads it produced.
fn run() -> (Vec<u8>, Vec<u8>) {
    let lua = Lua::new();
    let preload: Table = lua
        .globals()
        .get::<Table>("package")
        .unwrap()
        .get("preload")
        .unwrap();
    for (name, source) in MODULES {
        let chunk = lua
            .load(source)
            .set_name(format!("@tpf3mp/{name}.lua"))
            .into_function()
            .unwrap();
        preload.set(format!("tpf3mp.{name}"), chunk).unwrap();
    }
    // Loading the engine adapter needs no game: its API calls happen only
    // when a capture runs.
    lua.load("require 'tpf3mp.engine'").exec().unwrap();
    let (road, track): (LuaString, LuaString) = lua
        .load(TEST)
        .set_name("@road_capture.lua")
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    (road.as_bytes().to_vec(), track.as_bytes().to_vec())
}

fn decode(bytes: Vec<u8>) -> Action {
    Action::from_payload(&Payload::new(bytes).unwrap()).unwrap()
}

fn text<const N: usize>(value: &str) -> Text<N> {
    Text::new(value).unwrap()
}

fn list<T, const N: usize>(items: Vec<T>) -> BoundedVec<T, N> {
    BoundedVec::new(items).unwrap()
}

fn pos(x: i32, y: i32, z: i32) -> Pos {
    Pos { x, y, z }
}

fn tangent(x: i32, y: i32, z: i32) -> Tangent {
    Tangent { x, y, z }
}

fn link(from: u16, to: u16, t0: Tangent, t1: Tangent, structure: Structure) -> Link {
    Link {
        from,
        to,
        tangent0: t0,
        tangent1: t1,
        structure,
    }
}

#[test]
fn a_captured_road_decodes_as_the_schema_says() {
    let (road, _) = run();
    let expected = Action::BuildRoad(RoadBuild {
        street: text("street/standard/town_medium_new.lua"),
        bus_lane: true,
        tram: Tram::Electric,
        polyline: Polyline::new(
            list(vec![
                Vertex {
                    pos: pos(100_000, 200_000, 10_000),
                    resolve: Resolve::Node(Network::Street),
                },
                Vertex {
                    pos: pos(160_000, 200_000, 12_000),
                    resolve: Resolve::New,
                },
                Vertex {
                    pos: pos(220_000, 230_000, 15_000),
                    resolve: Resolve::New,
                },
                Vertex {
                    pos: pos(300_000, 50_000, 5_000),
                    resolve: Resolve::Split(EdgeRef {
                        network: Network::Street,
                        ends: EdgeEnds {
                            a: pos(300_000, 0, 5_000),
                            b: pos(300_000, 100_000, 5_000),
                        },
                    }),
                },
            ]),
            list(vec![
                link(
                    0,
                    1,
                    tangent(60_000, 0, 2_000),
                    tangent(60_000, 0, 2_000),
                    Structure::Ground,
                ),
                link(
                    1,
                    2,
                    tangent(55_000, 20_000, 0),
                    tangent(60_000, 28_250, -2_000),
                    Structure::Bridge(text("bridge/cement.lua")),
                ),
                link(
                    2,
                    3,
                    tangent(80_000, -180_000, -10_000),
                    tangent(80_000, -180_000, -10_000),
                    Structure::Ground,
                ),
            ]),
            list(vec![EdgeEnds {
                a: pos(100_000, 300_000, 10_000),
                b: pos(100_000, 200_000, 10_000),
            }]),
        )
        .unwrap(),
    });
    assert_eq!(decode(road), expected);
}

#[test]
fn a_captured_track_decodes_as_the_schema_says() {
    let (_, track) = run();
    let expected = Action::BuildTrack(TrackBuild {
        track: text("high_speed.lua"),
        catenary: true,
        polyline: Polyline::new(
            list(vec![
                Vertex {
                    pos: pos(0, 0, 1_000),
                    resolve: Resolve::Node(Network::Track),
                },
                Vertex {
                    pos: pos(50_000, 1, 1_250),
                    resolve: Resolve::Split(EdgeRef {
                        network: Network::Street,
                        ends: EdgeEnds {
                            a: pos(50_000, -40_000, 1_000),
                            b: pos(50_000, 40_000, 1_000),
                        },
                    }),
                },
                Vertex {
                    pos: pos(150_000, -1, 1_000),
                    resolve: Resolve::New,
                },
                Vertex {
                    pos: pos(200_000, 0, 1_000),
                    resolve: Resolve::Node(Network::Street),
                },
            ]),
            list(vec![
                link(
                    0,
                    1,
                    tangent(50_000, 0, 250),
                    tangent(50_000, 0, 250),
                    Structure::Ground,
                ),
                link(
                    1,
                    2,
                    tangent(100_000, 0, 0),
                    tangent(100_000, 0, 0),
                    Structure::Tunnel(text("tunnel/concrete.lua")),
                ),
                link(
                    2,
                    3,
                    tangent(50_000, 0, 0),
                    tangent(50_000, 0, 0),
                    Structure::Ground,
                ),
            ]),
            BoundedVec::empty(),
        )
        .unwrap(),
    });
    assert_eq!(decode(track), expected);
}

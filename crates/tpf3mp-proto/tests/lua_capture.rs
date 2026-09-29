//! The Lua mod's road and track capture makes exactly the tables the action
//! schema takes: `tests/lua/road_capture.lua` runs the mod's own modules
//! against a world of tables, and the action tables it returns, in metres,
//! must convert (`tpf3mp_proto::lua`, as the hook converts them) to the
//! actions below, in millimetres. Tables changed to break the schema must
//! be refused. Lua 5.1 here (the workspace's vendored Lua); the mod targets
//! 5.2 and keeps to what both run.
//!
//! The modules are compiled into the test binary, so the test does not
//! depend on the working directory.

#![allow(clippy::unwrap_used)]

mod common;

use common::tree;
use mlua::{Lua, Table, Value};
use tpf3mp_proto::{
    BoundedVec, Text,
    action::{
        Action, EdgeEnds, EdgeRef, Link, Network, Polyline, Pos, Resolve, RoadBuild, Structure,
        Tangent, TrackBuild, Tram, Vertex,
    },
    lua::{LuaValue, action_from_lua},
};

macro_rules! modules {
    ($($name:literal),* $(,)?) => {
        [$(($name, include_str!(concat!("../../../mod/tpf3mp_1/content/scripts/tpf3mp/", $name, ".lua")))),*]
    };
}

/// Every module of the mod, as `require "tpf3mp.<name>"` finds it.
const MODULES: [(&str, &str); 4] = modules!("geom", "roads", "engine", "speed");

const TEST: &str = include_str!("lua/road_capture.lua");

struct Captured {
    road: LuaValue,
    track: LuaValue,
    must_refuse: Vec<(String, LuaValue)>,
}

/// Runs the Lua test and returns the tables it produced.
fn run() -> Captured {
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
    let (road, track, refuse): (Value, Value, Table) = lua
        .load(TEST)
        .set_name("@road_capture.lua")
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    let mut must_refuse: Vec<(String, LuaValue)> = refuse
        .pairs::<String, Value>()
        .map(|pair| {
            let (why, action) = pair.unwrap();
            (why, tree(&action))
        })
        .collect();
    must_refuse.sort_by(|a, b| a.0.cmp(&b.0));
    Captured {
        road: tree(&road),
        track: tree(&track),
        must_refuse,
    }
}

fn decode(table: LuaValue) -> Action {
    action_from_lua(&table).unwrap_or_else(|error| panic!("{error}"))
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
    let road = run().road;
    let expected = Action::BuildRoad(RoadBuild {
        street: text("street/standard/town_medium_new.lua"),
        style: Some(text("style/old_town.lua")),
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
    let track = run().track;
    let expected = Action::BuildTrack(TrackBuild {
        track: text("high_speed.lua"),
        style: None,
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

#[test]
fn a_capture_the_schema_does_not_allow_is_refused() {
    let refused = run().must_refuse;
    assert_eq!(refused.len(), 8);
    for (why, table) in refused {
        assert!(
            action_from_lua(&table).is_err(),
            "{why}: the schema took it"
        );
    }
}

/// Reads a builder proposal with the mod's `engine.fromProposal`, in a
/// stand-in for the game's API, and returns `street`, `track`, `style` and
/// the first edge's network, joined by `|`, or the error.
fn read_proposal(api: &str, segment: &str, network: &str) -> Result<String, String> {
    let lua = Lua::new();
    let preload: Table = lua
        .globals()
        .get::<Table>("package")
        .unwrap()
        .get("preload")
        .unwrap();
    for (name, source) in MODULES {
        let chunk = lua.load(source).into_function().unwrap();
        preload.set(format!("tpf3mp.{name}"), chunk).unwrap();
    }
    lua.load(format!(
        "api = {api}
         local engine = require 'tpf3mp.engine'
         local function v(x, y, z) return {{ x = x, y = y, z = z }} end
         local seg = {segment}
         local proposal = {{
             addedNodes = {{ {{ entity = -1, comp = {{ position = v(0, 0, 0) }} }},
                            {{ entity = -2, comp = {{ position = v(100, 0, 0) }} }} }},
             addedSegments = {{ seg }},
             removedSegments = {{}},
         }}
         local c = engine.fromProposal(proposal, '{network}')
         return table.concat({{ tostring(c.street), tostring(c.track), tostring(c.style), c.edges[1].network }}, '|')"
    ))
    .eval::<String>()
    .map_err(|error| error.to_string())
}

const TF3_API: &str = "{ type = { RoadType = { STREET = 0, TRACK = 1 } } }";

fn tf3_segment(road_type: u8, style: &str) -> String {
    format!(
        "{{ entity = -3, type = 0, streetEdge = {{}}, comp = {{ node0 = -1, node1 = -2, \
         tangent0 = v(100, 0, 0), tangent1 = v(100, 0, 0), type = 0, typeIndex = -1, \
         roadType = {road_type}, roadTemplate = 'road/town_medium.lua', roadStyle = {style} }} }}"
    )
}

#[test]
fn a_tf3_proposal_is_read_by_road_template_and_style() {
    let street = read_proposal(TF3_API, &tf3_segment(0, "'style/old_town.lua'"), "Street");
    assert_eq!(
        street.unwrap(),
        "road/town_medium.lua|nil|style/old_town.lua|Street"
    );
    let track = read_proposal(TF3_API, &tf3_segment(1, "nil"), "Track");
    assert_eq!(track.unwrap(), "nil|road/town_medium.lua|nil|Track");
}

#[test]
fn a_tf3_proposal_the_mod_cannot_place_fails_the_capture() {
    // No api.type.RoadType: which network the edge is in is unknown.
    let unknown = read_proposal("{}", &tf3_segment(1, "nil"), "Track");
    assert!(unknown.unwrap_err().contains("api.type.RoadType"));
    // A style that is not a resource name.
    let odd = read_proposal(TF3_API, &tf3_segment(0, "7"), "Street");
    assert!(odd.unwrap_err().contains("roadStyle"));
}

#[test]
fn a_tpf2_proposal_still_reads_by_type_file() {
    let api =
        "{ res = { streetTypeRep = { getName = function(i) return 'street/standard.lua' end } } }";
    let segment = "{ entity = -3, type = 0, \
        streetEdge = { streetType = 4, hasBus = true, tramTrackType = 2 }, \
        comp = { node0 = -1, node1 = -2, tangent0 = v(100, 0, 0), tangent1 = v(100, 0, 0), \
                 type = 0, typeIndex = -1 } }";
    assert_eq!(
        read_proposal(api, segment, "Street").unwrap(),
        "street/standard.lua|nil|nil|Street"
    );
}

/// Runs the speed keeper (`tpf3mp/speed.lua`) against a game whose speed
/// the Lua snippet `script` changes between frames, and returns what each
/// frame did, joined by spaces, then the commands sent and the log lines.
fn keep_speed(in_room: bool, script: &str) -> String {
    let lua = Lua::new();
    let preload: Table = lua
        .globals()
        .get::<Table>("package")
        .unwrap()
        .get("preload")
        .unwrap();
    for (name, source) in MODULES {
        let chunk = lua.load(source).into_function().unwrap();
        preload.set(format!("tpf3mp.{name}"), chunk).unwrap();
    }
    lua.load(format!(
        "local speed = require 'tpf3mp.speed'
         local game = {{ speed = 1, sent = {{}}, log = {{}}, waiting = nil }}
         local keeper = speed.keeper({{
             inRoom = function() return {in_room} end,
             getSpeed = function() return game.speed end,
             setSpeed = function(n, done) game.sent[#game.sent + 1] = n; game.waiting = function() game.speed = n; done() end end,
             log = function(line) game.log[#game.log + 1] = line end,
         }})
         local did = {{}}
         local function frame() did[#did + 1] = keeper.step() end
         local function land() if game.waiting then local w = game.waiting; game.waiting = nil; w() end end
         {script}
         return table.concat(did, ' ') .. ' | ' .. table.concat(game.sent, ',') .. ' | ' .. table.concat(game.log, ';')"
    ))
    .eval::<String>()
    .unwrap_or_else(|error| panic!("{error}"))
}

#[test]
fn in_a_rooms_game_the_speed_goes_back_to_1x_one_command_at_a_time() {
    // The player picks 4x: one command, waited on until the game has run
    // it, then all is well; a pause (0) is set back too.
    let out = keep_speed(
        true,
        "frame() game.speed = 4 frame() frame() land() frame() game.speed = 0 frame() land() frame()",
    );
    let (did, rest) = out.split_once(" | ").unwrap();
    assert_eq!(did, "ok set waiting ok set ok");
    let (sent, log) = rest.split_once(" | ").unwrap();
    assert_eq!(sent, "1,1");
    assert_eq!(
        log.matches("the room sets the pace").count(),
        1,
        "told once: {log}"
    );
}

#[test]
fn outside_a_rooms_game_the_speed_is_the_players() {
    let out = keep_speed(false, "game.speed = 4 frame() frame()");
    assert_eq!(out, "outside outside |  | ");
}

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
        Action, AssetRef, AssetRemoval, Bulldoze, EdgeEnds, EdgeKind, EdgeRef, Fraction,
        KeptSignal, Link, Network, NodeRef, Polyline, Pos, Resolve, RoadBuild, Structure, Tangent,
        TrackBuild, Tram, Vertex,
    },
    lua::{LuaValue, action_from_lua},
};

macro_rules! modules {
    ($($name:literal),* $(,)?) => {
        [$(($name, include_str!(concat!("../../../mod/tpf3mp_1/content/scripts/tpf3mp/", $name, ".lua")))),*]
    };
}

/// Every module of the mod, as `require "tpf3mp.<name>"` finds it.
const MODULES: [(&str, &str); 6] = modules!(
    "geom",
    "roads",
    "junctions",
    "engine",
    "acceptance",
    "capture"
);

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
        precedence: None,
        position0: None,
        position1: None,
        kept: BoundedVec::default(),

        from,
        to,
        tangent0: t0,
        tangent1: t1,
        structure,
        kind: None,
        decorations: BoundedVec::default(),
        locked: false,
        owned: false,
        lanes: BoundedVec::default(),
    }
}

/// A link of another kind than the build's.
fn of(link: Link, network: Network, template: &str) -> Link {
    Link {
        kind: Some(EdgeKind {
            network,
            template: text(template),
            style: None,
        }),
        ..link
    }
}

fn node(x: i32, y: i32, z: i32, network: Network) -> Vertex {
    Vertex {
        pos: pos(x, y, z),
        resolve: Resolve::Node(network),
    }
}

fn new(x: i32, y: i32, z: i32) -> Vertex {
    Vertex {
        pos: pos(x, y, z),
        resolve: Resolve::New,
    }
}

fn street_edge(a: Pos, b: Pos) -> EdgeRef {
    EdgeRef {
        network: Network::Street,
        ends: EdgeEnds { a, b },
    }
}

const COUNTRY: &str = "street/country.street_template";

#[test]
fn a_captured_road_decodes_as_the_schema_says() {
    let road = run().road;
    let flat = |x| tangent(x, 0, 0);
    let expected = Action::BuildRoad(RoadBuild {
        street: text("street/town_medium.street_template"),
        style: Some(text("style/old_town.lua")),
        bus_lane: false,
        tram: Tram::None,
        polyline: Polyline::new(
            list(vec![
                node(100_000, 200_000, 10_000, Network::Street),
                new(160_000, 200_000, 12_000),
                new(220_000, 230_000, 15_000),
                new(452_000, 0, 2_000),
                node(400_000, 0, 2_000, Network::Street),
                node(500_000, 0, 2_000, Network::Street),
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
                    tangent(232_000, -230_000, -13_000),
                    tangent(232_000, -230_000, -13_000),
                    Structure::Ground,
                ),
                of(
                    link(4, 3, flat(52_000), flat(52_000), Structure::Ground),
                    Network::Street,
                    COUNTRY,
                ),
                of(
                    link(3, 5, flat(48_000), flat(48_000), Structure::Ground),
                    Network::Street,
                    COUNTRY,
                ),
            ]),
            list(vec![
                street_edge(pos(400_000, 0, 2_000), pos(450_000, 0, 2_000)),
                street_edge(pos(450_000, 0, 2_000), pos(500_000, 0, 2_000)),
                street_edge(pos(100_000, 300_000, 10_000), pos(100_000, 200_000, 10_000)),
            ]),
        )
        .unwrap()
        .with_removed_nodes(list(vec![NodeRef {
            network: Network::Street,
            at: pos(450_000, 0, 2_000),
        }])),
        crossing_type: None,
    });
    assert_eq!(decode(road), expected);
}

#[test]
fn a_captured_track_decodes_as_the_schema_says() {
    let track = run().track;
    let expected = Action::BuildTrack(TrackBuild {
        track: text("track/high_speed.street_template"),
        style: None,
        catenary: true,
        polyline: Polyline::new(
            list(vec![
                node(0, 0, 1_000, Network::Track),
                new(50_000, 1, 1_250),
                node(50_000, -40_000, 1_000, Network::Street),
                node(50_000, 40_000, 1_000, Network::Street),
                new(150_000, -1, 1_000),
                node(200_000, 0, 1_000, Network::Street),
            ]),
            list(vec![
                link(
                    0,
                    1,
                    tangent(50_000, 0, 250),
                    tangent(50_000, 0, 250),
                    Structure::Ground,
                ),
                of(
                    link(
                        2,
                        1,
                        tangent(0, 40_000, 0),
                        tangent(0, 40_000, 0),
                        Structure::Ground,
                    ),
                    Network::Street,
                    COUNTRY,
                ),
                of(
                    link(
                        1,
                        3,
                        tangent(0, 40_000, 0),
                        tangent(0, 40_000, 0),
                        Structure::Ground,
                    ),
                    Network::Street,
                    COUNTRY,
                ),
                link(
                    1,
                    4,
                    tangent(100_000, 0, 0),
                    tangent(100_000, 0, 0),
                    Structure::Tunnel(text("tunnel/concrete.lua")),
                ),
                link(
                    4,
                    5,
                    tangent(50_000, 0, 0),
                    tangent(50_000, 0, 0),
                    Structure::Ground,
                ),
            ]),
            list(vec![street_edge(
                pos(50_000, 40_000, 1_000),
                pos(50_000, -40_000, 1_000),
            )]),
        )
        .unwrap(),
        crossing_type: None,
    });
    assert_eq!(decode(track), expected);
}

#[test]
fn a_capture_the_schema_does_not_allow_is_refused() {
    let refused = run().must_refuse;
    assert_eq!(refused.len(), 10);
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
         local proposal = {{ toAdd = {{}}, toRemove = {{}}, proposal = {{
             addedNodes = {{ {{ entity = -1, comp = {{ position = v(0, 0, 0) }} }},
                            {{ entity = -2, comp = {{ position = v(100, 0, 0) }} }} }},
             addedSegments = {{ seg }},
             removedSegments = {{}},
         }} }}
         local c = engine.fromProposal(proposal, '{network}')
         return table.concat({{ tostring(c.street), tostring(c.track), tostring(c.style), c.edges[1].network }}, '|')"
    ))
    .eval::<String>()
    .map_err(|error| error.to_string())
}

/// As much of TF3's api as reading a proposal needs.
const TF3_API: &str =
    "{ type = { enum = { BaseEdgeType = { NORMAL = 0, BRIDGE = 1, TUNNEL = 2 } } } }";

/// A segment of the tool's proposal: `kind` 0 for a street, 1 for a track.
fn tf3_segment(kind: u8, style: &str) -> String {
    format!(
        "{{ entity = -3, type = {kind}, comp = {{ node0 = -1, node1 = -2, \
         tangent0 = v(100, 0, 0), tangent1 = v(100, 0, 0), type = 0, typeIndex = -1, \
         roadType = {kind}, roadTemplate = 'road/town_medium.lua', roadStyle = {style} }} }}"
    )
}

#[test]
fn a_tf3_proposal_is_read_by_road_template_and_style() {
    let street = read_proposal(TF3_API, &tf3_segment(0, "'style/old_town.lua'"), "Street");
    assert_eq!(
        street.unwrap(),
        "road/town_medium.lua|nil|style/old_town.lua|Street"
    );
    let track = read_proposal(TF3_API, &tf3_segment(1, "''"), "Track");
    assert_eq!(
        track.unwrap(),
        "nil|road/town_medium.lua|nil|Track",
        "an empty style is none"
    );
}

#[test]
fn a_tf3_proposal_the_mod_cannot_place_fails_the_capture() {
    // An edge of neither network.
    let unknown = read_proposal(TF3_API, &tf3_segment(5, "nil"), "Track");
    assert!(unknown.unwrap_err().contains("an edge of type 5"));
    // A style that is not a resource name.
    let odd = read_proposal(TF3_API, &tf3_segment(0, "7"), "Street");
    assert!(odd.unwrap_err().contains("roadStyle"));
    // No enum to tell a bridge by.
    let no_enum = read_proposal("{ type = {} }", &tf3_segment(0, "nil"), "Street");
    assert!(no_enum.unwrap_err().contains("api.type.enum.BaseEdgeType"));
}

/// Captures a street tool's proposal with the mod's `engine.captureBuild`, in
/// a stand-in for TF3's api over the network below, and returns the action
/// table, `false` for a proposal of nothing, or why not.
fn capture_street(street: &str) -> Result<LuaValue, String> {
    capture_with(
        street,
        "engine.captureBuild({ toAdd = {}, toRemove = {}, proposal = street }, 'Street')",
    )
}

/// As [`capture_street`], through the road modifiers' capture.
fn capture_modify(street: &str) -> Result<LuaValue, String> {
    capture_with(
        street,
        "engine.captureModify({ toAdd = {}, toRemove = {}, proposal = street })",
    )
}

fn capture_with(street: &str, call: &str) -> Result<LuaValue, String> {
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
    let (action, why): (Value, Option<String>) = lua
        .load(format!(
            "local function v(x, y, z) return {{ x = x, y = y, z = z }} end
             -- The country street 8-9-10 runs north through (50, 0); street
             -- 20-21 east-west under (120, 0), where the new road's bridge ends.
             -- The track 30-31 runs north at x = 300.
             local NODES = {{ [7] = v(0, 0, 0), [8] = v(50, -40, 0), [9] = v(50, 1, 0), [10] = v(50, 40, 0),
                              [20] = v(120, -30, 0), [21] = v(120, 30, 0), [30] = v(300, -30, 0),
                              [31] = v(300, 30, 0), [32] = v(300, 90, 0) }}
             local TRACKS = {{ [30] = {{ 110 }}, [31] = {{ 110, 111 }}, [32] = {{ 111 }} }}
             local STREETS = {{ [7] = {{ 104 }}, [8] = {{ 100 }}, [9] = {{ 100, 101 }}, [10] = {{ 101 }},
                                [20] = {{ 102 }}, [21] = {{ 102 }} }}
             api = {{
                 type = {{ enum = {{ BaseEdgeType = {{ NORMAL = 0, BRIDGE = 1, TUNNEL = 2 }},
                                    EdgeObjectType = {{ STOP_LEFT = 0, STOP_RIGHT = 1, SIGNAL = 2 }} }},
                          ComponentType = {{ BASE_NODE = 1, CONSTRUCTION = 2, EDGE_OBJECT = 3, SIGNAL_LIST = 4 }},
                          Signal = {{ Type = {{ SIGNAL = 0, ONE_WAY_SIGNAL = 1, WAYPOINT = 2 }} }} }},
                 res = {{ bridgeTypeRep = {{ getName = function(i) if i == 3 then return 'bridge/stone.lua' end end }},
                          edgeDecorationRep = {{ getName = function(i)
                              if i == 3 then return '::/infrastructure/edge_addons/barrier_b.edge' end end }} }},
                 engine = {{
                     getComponent = function(id, kind)
                         if kind == 1 and NODES[id] then return {{ position = NODES[id] }} end
                         -- 900 a town's house, 901 a station.
                         if kind == 2 and id == 900 then return {{ townBuildings = {{ 1 }} }} end
                         if kind == 2 and id == 901 then return {{ townBuildings = {{}} }} end
                         -- 902 a farm, whose field the road crosses.
                         if kind == 2 and id == 902 then
                             return {{ townBuildings = {{}}, fileName = '::/industries/farm/farm.con' }}
                         end
                         -- On the track 30-31: 556 a signal three quarters of the
                         -- way north, 557 a waypoint a quarter of the way, 558 a
                         -- signal half way, 559 a mod's signal three quarters.
                         if kind == 4 and id == 557 then return {{ signals = {{ {{ type = 2 }} }} }} end
                         if kind == 4 and id >= 556 and id <= 559 then return {{ signals = {{ {{ type = 0 }} }} }} end
                         if kind == 3 and id == 556 then
                             return {{ param = 0.75, edgeObjectConstruction = '::/infrastructure/signal/signal_path_a.con' }}
                         end
                         if kind == 3 and id == 558 then
                             return {{ param = 0.5, edgeObjectConstruction = '::/infrastructure/signal/signal_path_a.con' }}
                         end
                         if kind == 3 and id == 559 then
                             return {{ param = 0.75, edgeObjectConstruction = 'custom_signals_1::/infrastructure/signal/custom.con' }}
                         end
                         if kind == 3 and id == 557 then
                             return {{ param = 0.25, edgeObjectConstruction = '::/infrastructure/signal/waypoint.con' }}
                         end
                         -- Stops, where 30-31 is a street: 560 three quarters of
                         -- the way north, 561 and 562 a two-sided stop there.
                         if kind == 3 and id == 560 then
                             return {{ param = 0.75, edgeObjectConstruction = '::/stations/street/small_stops/small_new.con' }}
                         end
                         if kind == 3 and (id == 561 or id == 562) then
                             return {{ param = 0.75,
                                 edgeObjectConstruction = '::/stations/street/small_stops/small_new_twosided.con' }}
                         end
                     end,
                     system = {{ streetSystem = {{
                         getNodeStreetSegments = function(n) return STREETS[n] or {{}} end,
                         getNodeTrackSegments = function(n) return TRACKS[n] or {{}} end,
                     }} }},
                 }},
             }}
             local street = {street}
             local engine, roads = require('tpf3mp.engine'), require('tpf3mp.roads')
             return {call}"
        ))
        .eval()
        .map_err(|error| error.to_string())?;
    match (action, why) {
        (Value::Nil, why) => Err(why.unwrap_or_default()),
        (action, _) => Ok(tree(&action)),
    }
}

/// The street tool's proposal, as build 40408 hands it to game scripts: from
/// node 7 onto the country street 8-9, whose node 9 the new junction -1
/// replaces, rebuilding the street from 8 and 10 through it; then over a
/// bridge.
const SPLIT_PROPOSAL: &str = "{
    addedNodes = { { entity = -1, comp = { position = v(50, 0, 0) } },
                   { entity = -2, comp = { position = v(120, 0, 12) } } },
    addedSegments = {
        { entity = -3, type = 0, comp = { node0 = 7, node1 = -1, tangent0 = v(50, 0, 0), tangent1 = v(50, 0, 0),
          type = 0, typeIndex = -1, roadTemplate = 'street/town.street_template', roadStyle = '', objects = {} } },
        { entity = -4, type = 0, comp = { node0 = 8, node1 = -1, tangent0 = v(0, 40, 0), tangent1 = v(0, 40, 0),
          type = 0, typeIndex = -1, roadTemplate = 'street/country.street_template', roadStyle = '' } },
        { entity = -5, type = 0, comp = { node0 = -1, node1 = 10, tangent0 = v(0, 40, 0), tangent1 = v(0, 40, 0),
          type = 0, typeIndex = -1, roadTemplate = 'street/country.street_template', roadStyle = '' } },
        { entity = -6, type = 0, comp = { node0 = -1, node1 = -2, tangent0 = v(70, 0, 12), tangent1 = v(70, 0, 12),
          type = 1, typeIndex = 3, roadTemplate = 'street/town.street_template', roadStyle = '' } },
    },
    removedSegments = { { entity = 100, type = 0, comp = { node0 = 8, node1 = 9 } },
                        { entity = 101, type = 0, comp = { node0 = 9, node1 = 10 } } },
    removedNodes = { { entity = 9, comp = { position = v(50, 1, 0) } } },
}";

#[test]
fn a_tf3_proposal_travels_as_the_tool_made_it() {
    let action = decode(capture_street(SPLIT_PROPOSAL).unwrap());
    let expected = Action::BuildRoad(RoadBuild {
        street: text("street/town.street_template"),
        style: None,
        bus_lane: false,
        tram: Tram::None,
        polyline: Polyline::new(
            list(vec![
                node(0, 0, 0, Network::Street),
                new(50_000, 0, 0),
                node(50_000, -40_000, 0, Network::Street),
                node(50_000, 40_000, 0, Network::Street),
                new(120_000, 0, 12_000),
            ]),
            list(vec![
                link(
                    0,
                    1,
                    tangent(50_000, 0, 0),
                    tangent(50_000, 0, 0),
                    Structure::Ground,
                ),
                of(
                    link(
                        2,
                        1,
                        tangent(0, 40_000, 0),
                        tangent(0, 40_000, 0),
                        Structure::Ground,
                    ),
                    Network::Street,
                    COUNTRY,
                ),
                of(
                    link(
                        1,
                        3,
                        tangent(0, 40_000, 0),
                        tangent(0, 40_000, 0),
                        Structure::Ground,
                    ),
                    Network::Street,
                    COUNTRY,
                ),
                link(
                    1,
                    4,
                    tangent(70_000, 0, 12_000),
                    tangent(70_000, 0, 12_000),
                    Structure::Bridge(text("bridge/stone.lua")),
                ),
            ]),
            list(vec![
                street_edge(pos(50_000, -40_000, 0), pos(50_000, 1_000, 0)),
                street_edge(pos(50_000, 1_000, 0), pos(50_000, 40_000, 0)),
            ]),
        )
        .unwrap()
        .with_removed_nodes(list(vec![NodeRef {
            network: Network::Street,
            at: pos(50_000, 1_000, 0),
        }])),
        crossing_type: None,
    });
    assert_eq!(
        action, expected,
        "the bridge's end above 20-21 is new: only what the tool removes is removed"
    );
}

#[test]
fn a_street_proposal_of_nothing_or_of_more_is_not_carried() {
    // The tool before its first point.
    let nothing = capture_street("{ addedNodes = {}, addedSegments = {}, removedSegments = {} }");
    assert!(
        matches!(nothing, Ok(LuaValue::Boolean(false))),
        "{nothing:?}"
    );
    // A node the world does not have.
    let unknown = SPLIT_PROPOSAL.replace("node0 = 7,", "node0 = 77,");
    assert!(
        capture_street(&unknown)
            .unwrap_err()
            .contains("node 77 has no position")
    );
    // A stop or signal built with the road.
    let objects = SPLIT_PROPOSAL.replace(
        "removedSegments =",
        "edgeObjectsToAdd = { {} }, removedSegments =",
    );
    assert!(
        capture_street(&objects)
            .unwrap_err()
            .contains("a build with a stop or signal")
    );
    // An edge the tool moves with a stop on it: the stop would go onto an
    // edge that is not the one it stood on.
    let moved = SPLIT_PROPOSAL.replace(
        "roadStyle = '', objects = {}",
        "roadStyle = '', objects = { { 555, 1 } }",
    );
    let why = capture_street(&moved).unwrap_err();
    assert!(why.contains("a build that moves a stop or signal"), "{why}");
}

/// A road modifier's proposal as build 40408 hands it to game scripts (the
/// tram track tool, seen in a room): the country street 8-9-10 rebuilt edge
/// by edge between the same places, its middle node 9 removed and added
/// again there as -1, in the tram template; the edge 8-9 keeps its stop
/// 555, the edge 9-10 gets a noise barrier, and is locked and owned (the
/// player-owned tool).
const MODIFY_PROPOSAL: &str = "{
    addedNodes = { { entity = -1, comp = { position = v(50, 1, 0) } } },
    addedSegments = {
        { entity = -2, type = 0, comp = { node0 = 8, node1 = -1, tangent0 = v(0, 41, 0), tangent1 = v(0, 41, 0),
          type = 0, typeIndex = -1, roadTemplate = 'street/country_tram.street_template', roadStyle = '',
          objects = OBJECTS_A } },
        { entity = -3, type = 0, playerOwned = { player = 25 },
          comp = { node0 = -1, node1 = 10, tangent0 = v(0, 39, 0), tangent1 = v(0, 39, 0),
          type = 0, typeIndex = -1, roadTemplate = 'street/country_tram.street_template', roadStyle = '',
          objects = OBJECTS_B, edgeDecorations = { { 3, false } }, roadDevelopmentLocked = true } },
    },
    removedSegments = { { entity = 100, type = 0, comp = { node0 = 8, node1 = 9, tangent0 = v(0, 41, 0),
                          tangent1 = v(0, 41, 0), objects = { { 555, 0 } } } },
                        { entity = 101, type = 0, comp = { node0 = 9, node1 = 10, objects = {} } } },
    removedNodes = { { entity = 9, comp = { position = v(50, 1, 0) } } },
}";

/// The modifier proposal with the stop 555 on the first new edge (as the
/// tool rebuilds it), on the second, or on neither.
fn modify_proposal(a: &str, b: &str) -> String {
    MODIFY_PROPOSAL
        .replace("OBJECTS_A", a)
        .replace("OBJECTS_B", b)
}

#[test]
fn a_road_modifier_travels_as_the_road_it_rebuilds() {
    let action = decode(capture_modify(&modify_proposal("{ { 555, 0 } }", "{}")).unwrap());
    let Action::BuildRoad(road) = action else {
        panic!("{action:?}")
    };
    assert_eq!(road.street.as_str(), "street/country_tram.street_template");
    let links = &road.polyline.links;
    assert_eq!(links.len(), 2);
    // The edge that keeps its stop, and the one dressed and locked.
    assert!(links[0].decorations.is_empty() && !links[0].locked && !links[0].owned);
    assert_eq!(links[1].decorations.len(), 1);
    assert_eq!(
        links[1].decorations[0].name.as_str(),
        "::/infrastructure/edge_addons/barrier_b.edge"
    );
    assert!(links[1].locked && links[1].owned);
    // Both old edges and the old middle node go.
    assert_eq!(road.polyline.removals.len(), 2);
    assert_eq!(road.polyline.removed_nodes.len(), 1);
}

#[test]
fn a_stop_moves_with_a_road_only_on_the_edge_rebuilt_in_place() {
    // Onto the other edge, which does not go on from its own: refused.
    let why = capture_modify(&modify_proposal("{}", "{ { 555, 0 } }")).unwrap_err();
    assert!(why.contains("does not go on from its track"), "{why}");
    // Dropped from the rebuilt edge: refused.
    let why = capture_modify(&modify_proposal("{}", "{}")).unwrap_err();
    assert!(
        why.contains("removes an edge with a stop or signal"),
        "{why}"
    );
    // A new stop with the road: refused.
    let why =
        capture_modify(&modify_proposal("{ { 555, 0 }, { -400000000, 0 } }", "{}")).unwrap_err();
    assert!(why.contains("adds a stop or signal"), "{why}");
}

/// The bulldozer over a track's stub that ends on the country street 8-9-10
/// at its node 9 (a level crossing), as build 40420 proposed it in a room
/// (2026-10-16): the stub 120 (9-32) and its far node 32 go, the crossing
/// node 9 goes, and its street edges 100 and 101 are joined into one edge
/// 8-10. `JOIN` names the added edge's ends, `OBJECTS` the objects on the
/// street edge 100, `TO_REMOVE` the constructions the bulldozer takes.
const STUB_PROPOSAL: &str = "{ toAdd = {}, toRemove = TO_REMOVE, proposal = {
    addedNodes = {},
    addedSegments = {
        { entity = -1, type = 0, comp = { node0 = 8, node1 = JOIN, tangent0 = v(0, 80, 0), tangent1 = v(0, 80, 0),
          type = 0, typeIndex = -1, roadTemplate = 'street/country.street_template', roadStyle = '',
          objects = {} } },
    },
    removedSegments = { { entity = 120, type = 1, comp = { node0 = 9, node1 = 32, objects = {} } },
                        { entity = 100, type = 0, comp = { node0 = 8, node1 = 9, objects = OBJECTS } },
                        { entity = 101, type = 0, comp = { node0 = 9, node1 = 10, objects = {} } },
                        EXTRA_REMOVED },
    removedNodes = { { entity = 9, comp = { position = v(50, 1, 0) } },
                     { entity = 32, comp = { position = v(10, 1, 0) } } },
} }";

/// The bulldozer's capture of [`STUB_PROPOSAL`], the stub on node 9.
fn capture_stub(join: &str, objects: &str, to_remove: &str) -> Result<LuaValue, String> {
    capture_stub_with_extra(join, objects, to_remove, "")
}

fn capture_stub_with_extra(
    join: &str,
    objects: &str,
    to_remove: &str,
    extra_removed: &str,
) -> Result<LuaValue, String> {
    let proposal = STUB_PROPOSAL
        .replace("JOIN", join)
        .replace("OBJECTS", objects)
        .replace("TO_REMOVE", to_remove)
        .replace("EXTRA_REMOVED", extra_removed);
    capture_with(
        "nil",
        &format!(
            "(function() NODES[32] = v(10, 1, 0) TRACKS[9] = {{ 120 }} TRACKS[32] = {{ 120 }} \
             return engine.bulldoze({proposal}) end)()"
        ),
    )
}

/// The game's own removal of such a stub joins nothing, and the game
/// refused it ("Construction Not Possible", a room on build 40420): the
/// stub travels with the join, as the build the bulldozer proposes, its
/// stub removed in its own network.
#[test]
fn a_stub_bulldozed_off_a_street_travels_with_the_street_joined() {
    let action = decode(capture_stub("10", "{}", "{}").unwrap());
    let Action::BuildRoad(road) = action else {
        panic!("{action:?}")
    };
    assert_eq!(road.street.as_str(), "street/country.street_template");
    let polyline = &road.polyline;
    assert_eq!(polyline.links.len(), 1);
    assert_eq!(polyline.vertices.len(), 2);
    assert!(
        polyline
            .vertices
            .iter()
            .all(|v| matches!(v.resolve, Resolve::Node(Network::Street)))
    );
    let networks: Vec<_> = polyline.removals.iter().map(|r| r.network).collect();
    assert_eq!(networks, [Network::Track, Network::Street, Network::Street]);
    let nodes: Vec<_> = polyline.removed_nodes.iter().map(|n| n.network).collect();
    assert_eq!(nodes, [Network::Street, Network::Track]);
}

#[test]
fn a_bulldozer_join_the_room_cannot_account_for_is_refused() {
    // An added edge that joins no two removed edges at a removed node.
    let why = capture_stub("32", "{}", "{}").unwrap_err();
    assert_eq!(why, "a bulldozer proposal that builds");
    // A signal on a street edge it joins: it would move onto the new edge.
    let why = capture_stub("10", "{ { 555, 1 } }", "{}").unwrap_err();
    assert!(
        why.contains("removes an edge with a stop or signal"),
        "{why}"
    );
    // Town buildings along a street it removes.
    let why = capture_stub("10", "{}", "{ 900 }").unwrap_err();
    assert_eq!(
        why,
        "removing a street with town buildings where the bulldozer joins the streets left"
    );
}

#[test]
fn a_join_does_not_carry_an_unpaired_removed_edge() {
    // Edge 102 is a valid, existing street, but it is unrelated to the
    // 100/101 pair joined at removed node 9. It must not pass through as an
    // extra action removal.
    let extra = "{ entity = 102, type = 0, comp = { node0 = 20, node1 = 21, objects = {} } }";
    let why = capture_stub_with_extra("10", "{}", "{}", extra).unwrap_err();
    assert_eq!(why, "a bulldozer proposal that builds");
}

/// A road the tool draws through a town's house: the house goes with the
/// build, which every game's build clears again; a station in the way does
/// not.
#[test]
fn a_road_through_a_town_house_is_carried_and_through_a_station_is_not() {
    let through_house = capture_with(
        SPLIT_PROPOSAL,
        "engine.captureBuild({ toAdd = {}, toRemove = { 900 }, proposal = street }, 'Street')",
    );
    assert!(through_house.is_ok(), "{through_house:?}");
    let why = capture_with(
        SPLIT_PROPOSAL,
        "engine.captureBuild({ toAdd = {}, toRemove = { 901 }, proposal = street }, 'Street')",
    )
    .unwrap_err();
    assert!(why.contains("removes a construction"), "{why}");
}

/// A road across a farm's field: the tool proposes the farm removed and put
/// back of the same file. Carried once industryFields is on, as the road
/// alone, whether or not the industry is put back; refused while it is off,
/// and when it comes back as another file.
#[test]
fn a_road_across_an_industry_field_is_carried_behind_its_gate() {
    let call = |gate: bool, add: &str| {
        capture_with(
            SPLIT_PROPOSAL,
            &format!(
                "(function() require('tpf3mp.acceptance').industryFields = {gate}
                 return engine.captureBuild({{ toAdd = {add}, toRemove = {{ 902 }}, proposal = street }}, 'Street')
                 end)()"
            ),
        )
    };
    let same = "{ { fileName = '::/industries/farm/farm.con' } }";
    let why = call(false, same).unwrap_err();
    assert!(why.contains("industry's field awaits"), "{why}");
    let carried = call(true, same);
    assert!(carried.is_ok(), "{carried:?}");
    assert_eq!(
        carried,
        capture_with(
            SPLIT_PROPOSAL,
            "engine.captureBuild({ toAdd = {}, toRemove = {}, proposal = street }, 'Street')"
        )
    );
    // A field taken whole: the industry is not put back.
    let whole = call(true, "{}");
    assert!(whole.is_ok(), "{whole:?}");
    let why = call(
        true,
        "{ { fileName = '::/industries/coal_mine/coal_mine.con' } }",
    )
    .unwrap_err();
    assert!(why.contains("with constructions"), "{why}");
}

/// A track modifier's proposal (INFERRED from the street tools' seen
/// shape, not yet seen in the game): the track 30-31 rebuilt in place in
/// the high-speed template, its one lane electrified (ELECTRIC_TRAIN, mode
/// 8, beside TRAIN, 7) and faster.
const TRACK_MODIFY_PROPOSAL: &str = "{
    addedNodes = {},
    addedSegments = {
        { entity = -1, type = 1, comp = { node0 = 30, node1 = 31, tangent0 = v(0, 60, 0), tangent1 = v(0, 60, 0),
          type = 0, typeIndex = -1, roadTemplate = 'track/high_speed.track_template', roadStyle = '',
          objects = {}, laneConfigs = { { speed = 83.333, width = 1.435, height = 0, offset = 0,
          forward = true, transportModes = { [7] = true, [8] = true } } } } },
    },
    removedSegments = { { entity = 110, type = 1, comp = { node0 = 30, node1 = 31, objects = {} } } },
    removedNodes = {},
}";

#[test]
fn a_track_modifier_carries_its_type_catenary_and_speed() {
    let action = decode(capture_modify(TRACK_MODIFY_PROPOSAL).unwrap());
    let Action::BuildTrack(track) = action else {
        panic!("{action:?}")
    };
    assert_eq!(track.track.as_str(), "track/high_speed.track_template");
    let link = &track.polyline.links[0];
    assert_eq!(link.lanes.len(), 1);
    assert_eq!(
        link.lanes[0].modes,
        (1 << 7) | (1 << 8),
        "TRAIN and ELECTRIC_TRAIN"
    );
    assert_eq!(
        link.lanes[0].speed, 83_333,
        "the lane's speed, in thousandths"
    );
    assert_eq!(track.polyline.removals.len(), 1);
    assert_eq!(track.polyline.removals[0].network, Network::Track);
}

/// The track tool's proposal for a track laid onto the end of street 104 at
/// node 7 (0, 0, 0), as build 40420 makes it: the new edge starts 2 m beside
/// the node, on the street's tram lane, and ends at its new node.
const TRACK_ONTO_STREET_END: &str = "{
    addedNodes = { { entity = -1, comp = { position = v(-50, -10, 0) } } },
    addedSegments = {
        { entity = -2, type = 1, comp = { node0 = 7, node1 = -1,
          position0 = v(-0.3146, -1.9752, 0), position1 = v(-50, -10, 0),
          tangent0 = v(-50, -8, 0), tangent1 = v(-50, -8, 0),
          type = 0, typeIndex = -1, roadTemplate = 'track/standard.track_template', roadStyle = '' } },
    },
    removedSegments = {},
}";

#[test]
fn a_track_onto_a_street_end_starts_where_the_tool_put_it() {
    let action = decode(
        capture_with(
            TRACK_ONTO_STREET_END,
            "engine.captureBuild({ toAdd = {}, toRemove = {}, proposal = street }, 'Track')",
        )
        .unwrap(),
    );
    let Action::BuildTrack(track) = action else {
        panic!("{action:?}")
    };
    assert_eq!(
        track.polyline.vertices[0],
        node(0, 0, 0, Network::Street),
        "the street's end node"
    );
    let link = &track.polyline.links[0];
    assert_eq!(
        link.position0,
        Some(pos(-315, -1975, 0)),
        "the start on the tram lane, beside its node, rounded to millimetres"
    );
    assert_eq!(link.position1, None, "the end is its new node");
}

#[test]
fn an_upgrade_is_said_in_one_line_and_another_build_is_not_an_upgrade() {
    let summary = |street: &str, call: &str| match capture_with(street, call).unwrap() {
        LuaValue::String(text) => String::from_utf8(text).unwrap(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        summary(
            TRACK_MODIFY_PROPOSAL,
            "roads.upgradeSummary(engine.captureModify({ toAdd = {}, toRemove = {}, proposal = street }))"
        ),
        "track upgrade of 1 edge(s) rebuilt in place; template track/high_speed.track_template; 1 lane(s) carrying TRAIN ELECTRIC_TRAIN; lane speeds 83.333 to 83.333; decorations none; locked 0, owned 0; junction settings 0"
    );
    assert_eq!(
        summary(
            &modify_proposal("{ { 555, 0 } }", "{}"),
            "roads.upgradeSummary(engine.captureModify({ toAdd = {}, toRemove = {}, proposal = street }))"
        ),
        "street upgrade of 2 edge(s) rebuilt in place; template street/country_tram.street_template; 2 edge(s) with their template's lanes; decorations ::/infrastructure/edge_addons/barrier_b.edge; locked 1, owned 1; junction settings 0"
    );
    // A new street onto another's middle is no upgrade.
    let other = capture_with(
        SPLIT_PROPOSAL,
        "roads.upgradeSummary(engine.captureBuild({ toAdd = {}, toRemove = {}, proposal = street }, 'Street'))",
    );
    assert_eq!(other, Err(String::new()));
}

/// The bridge and tunnel window's type change (`bridge_and_tunnel.tl`,
/// `createBridgeOrTunnelProposal`), as the window sends it: the track
/// 30-31 rebuilt in place between the same nodes as a stone bridge (the
/// shape the window sent in a two-player game on build 40420, 2026-10-11).
const BRIDGE_WINDOW_PROPOSAL: &str = "{
    addedNodes = {},
    addedSegments = {
        { entity = -1, type = 1, comp = { node0 = 30, node1 = 31, tangent0 = v(0, 60, 0), tangent1 = v(0, 60, 0),
          type = 1, typeIndex = 3, roadTemplate = 'track/standard.track_template', roadStyle = '',
          objects = {} } },
    },
    removedSegments = { { entity = 110, type = 1, comp = { node0 = 30, node1 = 31, objects = {} } } },
    removedNodes = {},
}";

fn window_build(street: &str, gate: bool) -> Result<LuaValue, String> {
    capture_with(
        street,
        &format!(
            "(function() require('tpf3mp.acceptance').bridges = {gate} \
             return require('tpf3mp.capture').windowBuild(nil, {{ toAdd = {{}}, toRemove = {{}}, proposal = street }}) end)()"
        ),
    )
}

/// A bridge's type changed in its window was refused in a room ("building
/// from this window"): an in-place rebuild of edges travels as the track
/// modifiers' does, once its gate is on; until then it says it waits for
/// acceptance. A window build that adds or removes nodes is no in-place
/// rebuild and stays refused.
#[test]
fn a_bridge_window_rebuild_travels_as_the_track_it_rebuilds() {
    let why = window_build(BRIDGE_WINDOW_PROPOSAL, false).unwrap_err();
    assert!(why.contains("awaits two-player game acceptance"), "{why}");
    // On by default since the two-player game of 2026-10-11.
    let default = capture_with(
        BRIDGE_WINDOW_PROPOSAL,
        "require('tpf3mp.capture').windowBuild(nil, { toAdd = {}, toRemove = {}, proposal = street })",
    );
    assert!(default.is_ok(), "{default:?}");

    let action = decode(window_build(BRIDGE_WINDOW_PROPOSAL, true).unwrap());
    let Action::BuildTrack(track) = action else {
        panic!("{action:?}")
    };
    assert_eq!(track.track.as_str(), "track/standard.track_template");
    assert_eq!(track.polyline.links.len(), 1);
    assert_eq!(
        track.polyline.links[0].structure,
        Structure::Bridge(text("bridge/stone.lua"))
    );
    assert_eq!(track.polyline.removals.len(), 1);
    assert_eq!(track.polyline.removals[0].network, Network::Track);
    assert!(track.polyline.removed_nodes.is_empty());

    // Rebuilt the other way round: still in place.
    let reversed = BRIDGE_WINDOW_PROPOSAL.replace(
        "node0 = 30, node1 = 31, tangent0",
        "node0 = 31, node1 = 30, tangent0",
    );
    assert!(window_build(&reversed, true).is_ok());
    // A road modifier's rebuild through a new node is not the window's.
    let moved = modify_proposal("{ { 555, 0 } }", "{}");
    let why = window_build(&moved, true).unwrap_err();
    assert!(!why.contains("acceptance"), "{why}");
    // A bridge that keeps the signal on it, and one that drops it.
    let signal = BRIDGE_WINDOW_PROPOSAL
        .replace(
            "objects = {} } },\n    },",
            "objects = { { 556, 2 } } } },\n    },",
        )
        .replace(
            "node1 = 31, objects = {}",
            "node1 = 31, objects = { { 556, 2 } }",
        );
    assert!(window_build(&signal, true).is_ok());
    let dropped = BRIDGE_WINDOW_PROPOSAL.replace(
        "node1 = 31, objects = {}",
        "node1 = 31, objects = { { 556, 2 } }",
    );
    let why = window_build(&dropped, true).unwrap_err();
    assert!(
        why.contains("removes an edge with a stop or signal"),
        "{why}"
    );
}

/// A track bridge of 127 pieces, nodes 1000-1127 along x = 1000..2270 with
/// the edges 2000-2126 between them, each inner node turning its one lane
/// from the piece before onto the next; its type changed in its window. As
/// the game makes the proposal (build 40420), every node at a rebuilt piece
/// gets its settings again naming the new pieces, and every old one goes.
/// `CHANGE` is a Lua statement run on the proposal's configurations, keyed
/// by node.
fn long_bridge(change: &str) -> String {
    format!(
        "(function()
            local N = 127
            local ct, enum = api.type.ComponentType, api.type.enum
            ct.BASE_EDGE, ct.BASE_NODE_CONFIG = 5, 6
            enum.TrafficLightPreference = {{ AUTO = 0, YES = 1, NO = 2 }}
            local function at(i) return v(1000 + 10 * i, 0, 20) end
            local function config(i, edge)
                local turns = {{}}
                if i > 0 and i < N then
                    turns[1] = {{ segment0 = edge(i - 1), lane0 = 0, segment1 = edge(i), lane1 = 0,
                                  withRoad = false, withTram = false }}
                end
                return {{ laneConnections = turns, crosswalks = {{}}, trafficLightPreference = 0,
                          trafficLightConfig = {{ trafficLightType = -1, states = {{}} }},
                          doubleSlipSwitch = false, userModifiedTrafficLightStates = false }}
            end
            local old = function(i) return 2000 + i end
            local new = function(i) return -1 - i end
            local getComponent = api.engine.getComponent
            api.engine.getComponent = function(id, kind)
                local i = id - 1000
                if kind == 1 and i >= 0 and i <= N then return {{ position = at(i) }} end
                if kind == 6 and i >= 0 and i <= N then return config(i, old) end
                if kind == 5 and id >= 2000 and id < 2000 + N then
                    return {{ node0 = id - 1000, node1 = id - 999 }}
                end
                return getComponent(id, kind)
            end
            local tracks = api.engine.system.streetSystem.getNodeTrackSegments
            api.engine.system.streetSystem.getNodeTrackSegments = function(n)
                local i = n - 1000
                if i < 0 or i > N then return tracks(n) end
                local out = {{}}
                if i > 0 then out[#out + 1] = old(i - 1) end
                if i < N then out[#out + 1] = old(i) end
                return out
            end
            local added, removed, configs, removes = {{}}, {{}}, {{}}, {{}}
            for i = 0, N - 1 do
                added[#added + 1] = {{ entity = new(i), type = 1, comp = {{ node0 = 1000 + i, node1 = 1001 + i,
                    tangent0 = v(10, 0, 0), tangent1 = v(10, 0, 0), type = 1, typeIndex = 3,
                    roadTemplate = 'track/standard.track_template', roadStyle = '', objects = {{}} }} }}
                removed[#removed + 1] = {{ entity = old(i), type = 1, comp = {{ node0 = 1000 + i, node1 = 1001 + i,
                    objects = {{}} }} }}
            end
            local byNode = {{}}
            for i = 0, N do
                byNode[1000 + i] = config(i, new)
                configs[#configs + 1] = {{ entity = 1000 + i, comp = byNode[1000 + i] }}
                removes[#removes + 1] = 1000 + i
            end
            {change}
            return {{ addedNodes = {{}}, addedSegments = added, removedSegments = removed, removedNodes = {{}},
                      nodeConfigsToAdd = configs, nodeConfigsToRemove = removes }}
        end)()"
    )
}

/// A long track bridge's type changed in its window was refused before it
/// was sent (2026-10-19, build 40420: "BuildTrack.polyline.junctions:
/// invalid length 128, expected a list of at most 64 items"): the window's
/// proposal names every node's settings again, with the rebuilt pieces in
/// place of the old. Settings the rebuild does not change stay out of the
/// action, since every game keeps those at the nodes of the edges it
/// rebuilds; one that does change is carried.
#[test]
fn a_long_bridge_window_rebuild_carries_only_the_junction_settings_it_changes() {
    let action = decode(window_build(&long_bridge(""), true).unwrap());
    let Action::BuildTrack(track) = &action else {
        panic!("{action:?}")
    };
    assert_eq!(track.polyline.links.len(), 127);
    assert_eq!(track.polyline.removals.len(), 127);
    assert!(
        track.polyline.junctions.is_empty(),
        "{:?}",
        track.polyline.junctions
    );
    // And it fits one action's payload.
    assert_eq!(
        Action::from_payload(&action.to_payload().unwrap()).unwrap(),
        action
    );
    let summary = capture_with(
        &long_bridge(""),
        "roads.upgradeSummary(require('tpf3mp.capture').windowBuild(nil, \
         { toAdd = {}, toRemove = {}, proposal = street }))",
    )
    .unwrap();
    assert!(
        matches!(&summary, LuaValue::String(s) if s.ends_with(b"; junction settings 0")),
        "{summary:?}"
    );

    // The double slip switch at node 1050 turned on with it, and the turn
    // at 1070 onto another lane: those two go, at their nodes.
    let action = decode(
        window_build(
            &long_bridge(
                "byNode[1050].doubleSlipSwitch = true \
                 byNode[1070].laneConnections[1].lane1 = 1",
            ),
            true,
        )
        .unwrap(),
    );
    let Action::BuildTrack(track) = &action else {
        panic!("{action:?}")
    };
    let at: Vec<i32> = track
        .polyline
        .junctions
        .iter()
        .map(|change| change.node.at.x)
        .collect();
    assert_eq!(at, [1_500_000, 1_700_000]);
    assert!(
        track.polyline.junctions[0]
            .config
            .as_ref()
            .unwrap()
            .double_slip
    );

    // A node whose settings go, and one that had none and gets the game's:
    // both changes, both carried.
    let action = decode(
        window_build(
            &long_bridge(
                "configs[11] = configs[#configs] configs[#configs] = nil \
                 local getComponent = api.engine.getComponent \
                 api.engine.getComponent = function(id, kind) \
                     if kind == 6 and id == 1020 then return nil end \
                     return getComponent(id, kind) end",
            ),
            true,
        )
        .unwrap(),
    );
    let Action::BuildTrack(track) = &action else {
        panic!("{action:?}")
    };
    let changes: Vec<(i32, bool)> = track
        .polyline
        .junctions
        .iter()
        .map(|change| (change.node.at.x, change.config.is_some()))
        .collect();
    assert_eq!(changes, [(1_100_000, false), (1_200_000, true)]);

    // A bridge the room built has no settings at its nodes (the room's
    // track builds give none), and the window gives each the game's empty
    // one (seen on build 40420, 2026-10-19: no turns, no light, preference
    // Auto): nothing to carry. One node given a preference too goes.
    let empty = "for _, c in pairs(byNode) do c.laneConnections = {} end \
                 local getComponent = api.engine.getComponent \
                 api.engine.getComponent = function(id, kind) \
                     if kind == 6 and id >= 1000 and id <= 1127 then return nil end \
                     return getComponent(id, kind) end";
    let action = decode(window_build(&long_bridge(empty), true).unwrap());
    let Action::BuildTrack(track) = &action else {
        panic!("{action:?}")
    };
    assert!(
        track.polyline.junctions.is_empty(),
        "{:?}",
        track.polyline.junctions
    );
    let action = decode(
        window_build(
            &long_bridge(&format!("{empty} byNode[1030].trafficLightPreference = 2")),
            true,
        )
        .unwrap(),
    );
    let Action::BuildTrack(track) = &action else {
        panic!("{action:?}")
    };
    let at: Vec<i32> = track
        .polyline
        .junctions
        .iter()
        .map(|change| change.node.at.x)
        .collect();
    assert_eq!(at, [1_300_000], "the node set to No");
}

/// A long road bridge's type changed in its window still carried the
/// settings of 28 of its 94 nodes (2026-10-19, build 40420): each listed
/// its two crosswalks the other way round, every other setting the same.
/// The game keeps crosswalks in a hash set of edge ids, so the new edges
/// list them in another order. They compare as a set, unless the node has
/// light phases, which name a crosswalk by its place in that order.
#[test]
fn a_bridge_window_rebuild_compares_crosswalks_as_a_set() {
    // Every inner node has a crosswalk over each of its two pieces, the
    // proposal's listed the other way round.
    let walks = |more: &str| {
        long_bridge(&format!(
            "local function walks(c, i, edge) \
                 if i > 0 and i < N then c.crosswalks = {{ edge(i - 1), edge(i) }} end \
                 return c end \
             local function phase(c, at) \
                 c.trafficLightConfig.states = {{ {{ lockedLanes = {{ at }}, duration = 20, \
                     minDuration = 5, canSkip = false }} }} \
                 return c end \
             for i = 1, N - 1 do \
                 local c = walks(byNode[1000 + i], i, new) \
                 c.crosswalks = {{ c.crosswalks[2], c.crosswalks[1] }} \
             end \
             local getComponent = api.engine.getComponent \
             local function oldConfig(i) return walks(config(i, old), i, old) end \
             {more} \
             api.engine.getComponent = function(id, kind) \
                 local i = id - 1000 \
                 if kind == 6 and i >= 0 and i <= N then return oldConfig(i) end \
                 return getComponent(id, kind) end"
        ))
    };
    let junctions_at = |street: &str| -> Vec<i32> {
        let action = decode(window_build(street, true).unwrap());
        let Action::BuildTrack(track) = &action else {
            panic!("{action:?}")
        };
        track
            .polyline
            .junctions
            .iter()
            .map(|change| change.node.at.x)
            .collect()
    };
    assert_eq!(junctions_at(&walks("")), [0; 0]);

    // A node with a phase locking its first crosswalk (lane 1, after its
    // one turn), before and after: the phase now locks the other one, so
    // it goes. One whose crosswalk goes, goes too.
    let at = junctions_at(&walks(
        "phase(byNode[1040], 1) \
         byNode[1060].crosswalks = { byNode[1060].crosswalks[1] } \
         local plain = oldConfig \
         oldConfig = function(i) \
             local c = plain(i) \
             if i == 40 then phase(c, 1) end \
             return c end",
    ));
    assert_eq!(at, [1_400_000, 1_600_000]);

    // The same phase locking the same crosswalk in both: nothing changes.
    let at = junctions_at(&walks(
        "local c = byNode[1040] \
         c.crosswalks = { c.crosswalks[2], c.crosswalks[1] } \
         phase(c, 1) \
         local plain = oldConfig \
         oldConfig = function(i) \
             local c = plain(i) \
             if i == 40 then phase(c, 1) end \
             return c end",
    ));
    assert_eq!(at, [0; 0]);
}

/// The track tool's proposal for a track started from the middle of the
/// track 30-31, as build 40420 makes it: the new node -1 cuts 30-31 in two,
/// the signal 556 three quarters of the way north going with the northern
/// piece (`OBJECTS_S` and `OBJECTS_N` the pieces' objects), and the new
/// track runs west from -1 to -2.
const TRACK_SPLIT_PROPOSAL: &str = "{
    addedNodes = { { entity = -1, comp = { position = v(300, 0, 0) } },
                   { entity = -2, comp = { position = v(260, 0, 0) } } },
    addedSegments = {
        { entity = -3, type = 1, comp = { node0 = 30, node1 = -1, tangent0 = v(0, 30, 0), tangent1 = v(0, 30, 0),
          type = 0, typeIndex = -1, roadTemplate = 'track/standard.track_template', roadStyle = '',
          objects = OBJECTS_S } },
        { entity = -4, type = 1, comp = { node0 = NORTH, tangent0 = TN, tangent1 = TN,
          type = 0, typeIndex = -1, roadTemplate = 'track/standard.track_template', roadStyle = '',
          objects = OBJECTS_N } },
        { entity = -5, type = 1, comp = { node0 = -1, node1 = -2, tangent0 = v(-40, 0, 0), tangent1 = v(-40, 0, 0),
          type = 0, typeIndex = -1, roadTemplate = 'track/standard.track_template', roadStyle = '',
          objects = OBJECTS_W } },
    },
    removedSegments = { { entity = 110, type = 1, comp = { node0 = 30, node1 = 31,
                          tangent0 = v(0, 60, 0), tangent1 = v(0, 60, 0), objects = OBJECTS_OLD } } },
    removedNodes = {},
}";

fn track_split(old: &str, south: &str, north: &str) -> String {
    track_split_west(old, south, north, "{}")
}

/// As [`track_split`], `west` the objects of the new track west of the cut.
fn track_split_west(old: &str, south: &str, north: &str, west: &str) -> String {
    TRACK_SPLIT_PROPOSAL
        .replace("OBJECTS_OLD", old)
        .replace("OBJECTS_W", west)
        .replace("OBJECTS_S", south)
        .replace("OBJECTS_N", north)
        .replace("NORTH", "-1, node1 = 31")
        .replace("TN", "v(0, 30, 0)")
}

fn capture_track(street: &str) -> Result<LuaValue, String> {
    capture_with(
        street,
        "engine.captureBuild({ toAdd = {}, toRemove = {}, proposal = street }, 'Track')",
    )
}

/// A track started or ended between a track's signals cuts that track in
/// two: the signal goes with the piece it stands on, the one the tool put
/// it on, named by where it stood (playtest 2026-10-17, "a build that moves
/// a stop or signal"). The accepted case is enabled; the gate-off check below
/// confirms that it refuses the action when disabled.
#[test]
fn a_track_cut_between_its_signals_keeps_each_on_its_piece() {
    let action = capture_track(&track_split("{ { 556, 2 } }", "{}", "{ { 556, 2 } }")).unwrap();
    let decoded = decode(action);
    let Action::BuildTrack(track) = &decoded else {
        panic!("{decoded:?}")
    };
    let links = &track.polyline.links;
    assert_eq!(links.len(), 3);
    assert!(links[0].kept.is_empty() && links[2].kept.is_empty());
    assert_eq!(links[1].kept.len(), 1);
    let kept = &links[1].kept[0];
    assert_eq!(kept.removal, 0);
    assert_eq!(kept.at, Fraction(750_000));
    assert_eq!(
        kept.model.as_str(),
        "::/infrastructure/signal/signal_path_a.con"
    );
    assert_eq!(track.polyline.removals.len(), 1);

    // On the piece before the cut, where the tool put it: the tool decides,
    // and the game places it on that piece (seen on build 40420, 2026-10-17,
    // a node moved near the cut and the signal past it).
    let south =
        decode(capture_track(&track_split("{ { 556, 2 } }", "{ { 556, 2 } }", "{}")).unwrap());
    let Action::BuildTrack(south) = &south else {
        panic!("{south:?}")
    };
    assert_eq!(south.polyline.links[0].kept.len(), 1);
    assert!(south.polyline.links[1].kept.is_empty());
    // A mod's signal, which the room's games all have.
    let modded =
        decode(capture_track(&track_split("{ { 559, 2 } }", "{}", "{ { 559, 2 } }")).unwrap());
    let Action::BuildTrack(modded) = &modded else {
        panic!("{modded:?}")
    };
    assert_eq!(
        modded.polyline.links[1].kept[0].model.as_str(),
        "custom_signals_1::/infrastructure/signal/custom.con"
    );

    // On since its two-player game (2026-10-17); with its gate off, it
    // waits, on sending and on applying alike.
    let on = capture_with(
        &track_split("{ { 556, 2 } }", "{}", "{ { 556, 2 } }"),
        "(function() local a = engine.captureBuild({ toAdd = {}, toRemove = {}, proposal = street }, 'Track') local ok, why = require('tpf3mp.acceptance').check(a) return ok == false and why or 'let through' end)()",
    );
    assert_eq!(on, Ok(LuaValue::String(b"let through".to_vec())));
    let gate = capture_with(
        &track_split("{ { 556, 2 } }", "{}", "{ { 556, 2 } }"),
        "(function() require('tpf3mp.acceptance').splitSignals = false local a = engine.captureBuild({ toAdd = {}, toRemove = {}, proposal = street }, 'Track')          local ok, why = require('tpf3mp.acceptance').check(a) return ok == false and why or 'let through' end)()",
    );
    assert_eq!(
        gate,
        Ok(LuaValue::String(
            b"splitSignals awaits two-player game acceptance".to_vec()
        ))
    );
    // A construction's streets keep none (every game's build of them would
    // refuse it), and the gate covers them too.
    let connection = capture_with(
        &track_split("{ { 556, 2 } }", "{}", "{ { 556, 2 } }"),
        "engine.fromProposal({ toAdd = {}, toRemove = {}, proposal = street }, nil, true)",
    )
    .unwrap_err();
    assert!(
        connection.contains("moves a stop or signal"),
        "{connection}"
    );
    let gate = capture_with(
        "{}",
        "(function() require('tpf3mp.acceptance').splitSignals = false local ok, why = require('tpf3mp.acceptance').check({ BuildConstruction = {          connection = { links = { { kept = { {} } } } } } }) return ok == false and why or 'let through' end)()",
    );
    assert_eq!(
        gate,
        Ok(LuaValue::String(
            b"splitSignals awaits two-player game acceptance".to_vec()
        ))
    );
}

#[test]
fn a_track_cut_that_moves_a_signal_elsewhere_is_refused() {
    let refused = |old: &str, south: &str, north: &str, why: &str| {
        let got = capture_track(&track_split(old, south, north)).unwrap_err();
        assert!(got.contains(why), "{got}");
    };
    // Onto the new track west of the cut, which does not go on from it.
    let west = capture_track(&track_split_west(
        "{ { 556, 2 } }",
        "{}",
        "{}",
        "{ { 556, 2 } }",
    ))
    .unwrap_err();
    assert!(west.contains("does not go on from its track"), "{west}");
    // Dropped.
    refused(
        "{ { 556, 2 } }",
        "{}",
        "{}",
        "removes an edge with a stop or signal",
    );
    // Twice on one piece.
    refused(
        "{ { 556, 2 } }",
        "{}",
        "{ { 556, 2 }, { 556, 2 } }",
        "moves a stop or signal",
    );
    // A waypoint, which lines name, whatever its construction.
    refused("{ { 557, 2 } }", "{ { 557, 2 } }", "{}", "moves a waypoint");
    // A stop, or the signal taken for a stop.
    refused(
        "{ { 556, 0 } }",
        "{}",
        "{ { 556, 0 } }",
        "moves a stop or signal",
    );
    refused(
        "{ { 556, 2 } }",
        "{}",
        "{ { 556, 0 } }",
        "moves a stop or signal",
    );
    // A new signal with the track.
    refused(
        "{ { 556, 2 } }",
        "{}",
        "{ { 556, 2 }, { -400000000, 2 } }",
        "adds a stop or signal",
    );
    // The northern piece laid the other way round.
    let reversed = TRACK_SPLIT_PROPOSAL
        .replace("OBJECTS_OLD", "{ { 556, 2 } }")
        .replace("OBJECTS_S", "{}")
        .replace("OBJECTS_N", "{ { 556, 2 } }")
        .replace("OBJECTS_W", "{}")
        .replace("NORTH", "31, node1 = -1")
        .replace("TN", "v(0, -30, 0)");
    let why = capture_track(&reversed).unwrap_err();
    assert!(why.contains("does not go on from its track"), "{why}");
}

/// A track started close to a node of a track with signals: the tool
/// removes the node, redraws the track through its own and lists one new
/// signal (playtest 2026-10-17, run-1017-024110, "a build with a stop or
/// signal"). The log says what it removes and where each signal goes: the
/// edges' objects, the objects removed as their EDGE_OBJECT has them, and
/// whatever of the new record reads, so a game shows whether the tool only
/// replaces the old signal.
#[test]
fn a_refit_signal_is_described_with_what_it_removes() {
    let refit = track_split("{ { 556, 2 } }", "{}", "{ { -400000000, 2 } }").replace(
        "removedNodes = {},",
        "removedNodes = {},
         edgeObjectsToAdd = { { resultEntity = -400000000, category = 2, left = false, playerEntity = 3892,
                                name = '', params = { auto_signals_distance = 400 } } },
         edgeObjectsToRemove = { 556, 999 },",
    );
    let described = capture_with(
        &refit,
        "engine.describe({ toAdd = {}, toRemove = {}, proposal = street })",
    )
    .unwrap();
    assert_eq!(
        described,
        LuaValue::String(
            "+n-1(300.0,0.0,0.0) +n-2(260.0,0.0,0.0) \
             +e-3/1:30(300.0,-30.0,0.0)>-1 +e-4/1:-1>31(300.0,30.0,0.0)[-400000000:2] +e-5/1:-1>-2 \
             -e110:30(300.0,-30.0,0.0)>31(300.0,30.0,0.0)[556:2] \
             +o{resultEntity=-400000000 category=2 left=false playerEntity=3892 name= \
             params={auto_signals_distance=400}} \
             -o556{param=0.75 edgeObjectConstruction=::/infrastructure/signal/signal_path_a.con} \
             -o999{no edge object}"
                .as_bytes()
                .to_vec()
        )
    );
}

/// As [`track_split_west`], with the tool's `edgeObjectsToAdd` and
/// `edgeObjectsToRemove`.
fn track_refit(
    old: &str,
    south: &str,
    north: &str,
    west: &str,
    adds: &str,
    removes: &str,
) -> String {
    track_split_west(old, south, north, west).replace(
        "removedNodes = {},",
        &format!(
            "removedNodes = {{}}, edgeObjectsToAdd = {adds}, edgeObjectsToRemove = {removes},"
        ),
    )
}

/// As [`capture_track`], with acceptance.lua's refitSignals on.
fn capture_refit(street: &str) -> Result<LuaValue, String> {
    capture_with(
        street,
        "(function() require('tpf3mp.acceptance').refitSignals = true          return engine.captureBuild({ toAdd = {}, toRemove = {}, proposal = street }, 'Track') end)()",
    )
}

/// The track tool's record of a new signal, as build 40420 lists it.
const NEW_SIGNAL: &str = "{ { resultEntity = -400000000, category = 2, left = false, oneWay = false, playerEntity = 3892, name = '' } }";

/// A track started close to a node of a track with signals: the tool
/// replaces the signal on the edge whose node it removes with a new one on
/// the piece that goes on from that edge's other end (run-1017-033551). The
/// room keeps the old signal there, as for a cut: the same action, the
/// signal named by where it stood, its construction and settings its own.
#[test]
fn a_signal_the_tool_puts_back_is_kept_on_its_piece() {
    let refit = track_refit(
        "{ { 556, 2 } }",
        "{}",
        "{ { -400000000, 2 } }",
        "{}",
        NEW_SIGNAL,
        "{ 556 }",
    );
    let decoded = decode(capture_refit(&refit).unwrap());
    let kept =
        decode(capture_track(&track_split("{ { 556, 2 } }", "{}", "{ { 556, 2 } }")).unwrap());
    assert_eq!(decoded, kept, "the same as the signal kept by its entity");
    let Action::BuildTrack(track) = &decoded else {
        panic!("{decoded:?}")
    };
    assert_eq!(track.polyline.links[1].kept.len(), 1);
    assert_eq!(track.polyline.links[1].kept[0].at, Fraction(750_000));
    // A mod's signal too.
    let modded = decode(
        capture_refit(&track_refit(
            "{ { 559, 2 } }",
            "{}",
            "{ { -400000000, 2 } }",
            "{}",
            NEW_SIGNAL,
            "{ 559 }",
        ))
        .unwrap(),
    );
    let Action::BuildTrack(modded) = &modded else {
        panic!("{modded:?}")
    };
    assert_eq!(
        modded.polyline.links[1].kept[0].model.as_str(),
        "custom_signals_1::/infrastructure/signal/custom.con"
    );
    // With its gate off: refused, though the room carries a split's kept
    // signals.
    let gate = capture_with(
        &refit,
        "(function() require('tpf3mp.acceptance').refitSignals = false return engine.captureBuild({ toAdd = {}, toRemove = {}, proposal = street }, 'Track') end)()",
    )
    .unwrap_err();
    assert!(
        gate.contains("puts back awaits two-player game acceptance"),
        "{gate}"
    );
}

/// As the game proposes it (run-1017-033551): the curved track 30-31, its
/// signal 558 half way, where the curve bulges 7.5 m east of the straight
/// line. A track started close to node 31: the tool removes 31, adds its
/// own node -1 near it and -5 on the old curve, the piece 30>-5 the old
/// curve up to three quarters (its tangents scaled so), and puts a new
/// signal on that piece.
#[test]
fn a_signal_the_tool_puts_back_on_a_curve_near_a_removed_node_is_kept() {
    let track = |tangent: &str| {
        format!(
            "{{ addedNodes = {{ {{ entity = -1, comp = {{ position = v(301, 31, 0) }} }},
                              {{ entity = -5, comp = {{ position = v(305.625, 15, 0) }} }},
                              {{ entity = -2, comp = {{ position = v(340, 31, 0) }} }} }},
                addedSegments = {{
                    {{ entity = -3, type = 1, comp = {{ node0 = 30, node1 = -5,
                        tangent0 = v(22.5, 45, 0), tangent1 = {tangent}, type = 0, typeIndex = -1,
                        roadTemplate = 'track/standard.track_template', roadStyle = '',
                        objects = {{ {{ -400000000, 2 }} }} }} }},
                    {{ entity = -4, type = 1, comp = {{ node0 = -5, node1 = -1,
                        tangent0 = v(-4.6, 16, 0), tangent1 = v(-4.6, 16, 0), type = 0, typeIndex = -1,
                        roadTemplate = 'track/standard.track_template', roadStyle = '', objects = {{}} }} }},
                    {{ entity = -6, type = 1, comp = {{ node0 = -1, node1 = -2,
                        tangent0 = v(39, 0, 0), tangent1 = v(39, 0, 0), type = 0, typeIndex = -1,
                        roadTemplate = 'track/standard.track_template', roadStyle = '', objects = {{}} }} }} }},
                removedSegments = {{ {{ entity = 110, type = 1, comp = {{ node0 = 30, node1 = 31,
                    tangent0 = v(30, 60, 0), tangent1 = v(-30, 60, 0), objects = {{ {{ 558, 2 }} }} }} }} }},
                removedNodes = {{ {{ entity = 31, comp = {{ position = v(300, 30, 0) }} }} }},
                edgeObjectsToAdd = {NEW_SIGNAL}, edgeObjectsToRemove = {{ 558 }} }}"
        )
    };
    let decoded = decode(capture_refit(&track("v(-11.25, 45, 0)")).unwrap());
    let Action::BuildTrack(built) = &decoded else {
        panic!("{decoded:?}")
    };
    let kept: Vec<_> = built
        .polyline
        .links
        .iter()
        .flat_map(|link| link.kept.iter())
        .collect();
    assert_eq!(kept.len(), 1, "{decoded:?}");
    assert_eq!(kept[0].removal, 0);
    assert_eq!(kept[0].at, Fraction(500_000));
    assert_eq!(
        kept[0].model.as_str(),
        "::/infrastructure/signal/signal_path_a.con"
    );
    // The piece bent the other way at its far end: it still goes on from
    // node 30 the same way, but no longer runs where the signal stood.
    let why = capture_refit(&track("v(30, 45, 0)")).unwrap_err();
    assert!(why.contains("replaces a signal elsewhere"), "{why}");
}

/// As the game proposes it (run-1017-pr165-p2, scaled onto track 30-31-32,
/// one straight line): signal 556 on 30-31, 15 m short of node 31; a track
/// started close to 31. The tool removes 31 and puts its own node -1 on
/// 30-31 just past the signal (13.2), so the signal now stands on the
/// piece -1>32 that took over the track beyond 31, and puts it back there.
/// Each case `-5` (30>-1) and `-6` (-1>32 or reversed) lists its objects;
/// `beyond` is the removed track 31-32 as `node0 = .., node1 = ..` with its
/// tangents.
fn across_removed_node(on5: &str, six: &str, on6: &str, beyond: &str) -> String {
    format!(
        "{{ addedNodes = {{ {{ entity = -1, comp = {{ position = v(300, 13.2, 0) }} }},
                          {{ entity = -2, comp = {{ position = v(305, 30, 0) }} }} }},
            addedSegments = {{
                {{ entity = -3, type = 1, comp = {{ node0 = -1, node1 = -2,
                    tangent0 = v(0, 17, 0), tangent1 = v(5, 17, 0), type = 0, typeIndex = -1,
                    roadTemplate = 'track/standard.track_template', roadStyle = '', objects = {{}} }} }},
                {{ entity = -5, type = 1, comp = {{ node0 = 30, node1 = -1,
                    tangent0 = v(0, 43.2, 0), tangent1 = v(0, 43.2, 0), type = 0, typeIndex = -1,
                    roadTemplate = 'track/standard.track_template', roadStyle = '', objects = {on5} }} }},
                {{ entity = -6, type = 1, comp = {{ {six}, type = 0, typeIndex = -1,
                    roadTemplate = 'track/standard.track_template', roadStyle = '', objects = {on6} }} }} }},
            removedSegments = {{
                {{ entity = 110, type = 1, comp = {{ node0 = 30, node1 = 31,
                    tangent0 = v(0, 60, 0), tangent1 = v(0, 60, 0), objects = {{ {{ 556, 2 }} }} }} }},
                {{ entity = 111, type = 1, comp = {{ {beyond}, objects = {{}} }} }} }},
            removedNodes = {{ {{ entity = 31, comp = {{ position = v(300, 30, 0) }} }} }},
            edgeObjectsToAdd = {NEW_SIGNAL}, edgeObjectsToRemove = {{ 556 }} }}"
    )
}

/// As run-1017-refit, scaled onto track 30-31-32: the tool removes 31 and
/// sets its own nodes -1 (y 0) before it and -8 (y 50) past it, the piece
/// -1>-8 between them running where signal 556 stood (y 15), its new
/// signal listed by `on`; -7 a branch off -1.
fn across_both_sides(on: &str) -> String {
    format!(
        "{{ addedNodes = {{ {{ entity = -1, comp = {{ position = v(300, 0, 0) }} }},
                          {{ entity = -8, comp = {{ position = v(300, 50, 0) }} }},
                          {{ entity = -5, comp = {{ position = v(306, 40, 0) }} }} }},
            addedSegments = {{
                {{ entity = -3, type = 1, comp = {{ node0 = 30, node1 = -1,
                    tangent0 = v(0, 30, 0), tangent1 = v(0, 30, 0), type = 0, typeIndex = -1,
                    roadTemplate = 'track/standard.track_template', roadStyle = '', objects = {{}} }} }},
                {{ entity = -9, type = 1, comp = {{ node0 = -1, node1 = -8,
                    tangent0 = v(0, 50, 0), tangent1 = v(0, 50, 0), type = 0, typeIndex = -1,
                    roadTemplate = 'track/standard.track_template', roadStyle = '', objects = {on} }} }},
                {{ entity = -10, type = 1, comp = {{ node0 = -8, node1 = 32,
                    tangent0 = v(0, 40, 0), tangent1 = v(0, 40, 0), type = 0, typeIndex = -1,
                    roadTemplate = 'track/standard.track_template', roadStyle = '', objects = {{}} }} }},
                {{ entity = -7, type = 1, comp = {{ node0 = -1, node1 = -5,
                    tangent0 = v(0, 40, 0), tangent1 = v(6, 40, 0), type = 0, typeIndex = -1,
                    roadTemplate = 'track/standard.track_template', roadStyle = '', objects = {{}} }} }} }},
            removedSegments = {{
                {{ entity = 110, type = 1, comp = {{ node0 = 30, node1 = 31,
                    tangent0 = v(0, 60, 0), tangent1 = v(0, 60, 0), objects = {{ {{ 556, 2 }} }} }} }},
                {{ entity = 111, type = 1, comp = {{ {BEYOND_NORTH}, objects = {{}} }} }} }},
            removedNodes = {{ {{ entity = 31, comp = {{ position = v(300, 30, 0) }} }} }},
            edgeObjectsToAdd = {NEW_SIGNAL}, edgeObjectsToRemove = {{ 556 }} }}"
    )
}

const SIX_NORTH: &str =
    "node0 = -1, node1 = 32, tangent0 = v(0, 76.8, 0), tangent1 = v(0, 76.8, 0)";
const BEYOND_NORTH: &str = "node0 = 31, node1 = 32, tangent0 = v(0, 60, 0), tangent1 = v(0, 60, 0)";
const NEW_ON: &str = "{ { -400000000, 2 } }";

#[test]
fn a_signal_the_tool_puts_back_across_a_removed_node_is_kept() {
    let kept = |street: String| {
        let decoded = decode(capture_refit(&street).unwrap());
        let Action::BuildTrack(built) = &decoded else {
            panic!("{decoded:?}")
        };
        let kept: Vec<_> = built
            .polyline
            .links
            .iter()
            .flat_map(|link| link.kept.iter())
            .collect();
        assert_eq!(kept.len(), 1, "{decoded:?}");
        assert_eq!(kept[0].removal, 0, "{decoded:?}");
        assert_eq!(kept[0].at, Fraction(750_000));
        assert_eq!(
            kept[0].model.as_str(),
            "::/infrastructure/signal/signal_path_a.con"
        );
    };
    kept(across_removed_node("{}", SIX_NORTH, NEW_ON, BEYOND_NORTH));
    // The removed track beyond the node may run either way round.
    kept(across_removed_node(
        "{}",
        SIX_NORTH,
        NEW_ON,
        "node0 = 32, node1 = 31, tangent0 = v(0, -60, 0), tangent1 = v(0, -60, 0)",
    ));
    let refused = |street: String, why: &str| {
        let got = capture_refit(&street).unwrap_err();
        assert!(got.contains(why), "{why}: {got}");
    };
    // As run-1017-refit: the tool sets its own nodes on both sides of the
    // removed node, -1 on 30-31 and -8 on 31-32, and puts the signal back
    // on the piece between them, whose ends are no old nodes.
    kept(across_both_sides(NEW_ON));
    // A piece from the old track out to another place is no stretch of it.
    refused(
        across_both_sides(NEW_ON).replace("v(300, 50, 0) }", "v(303, 50, 0) }"),
        "replaces a signal elsewhere",
    );
    // On the piece before it, which starts 1.8 m past where it stood.
    refused(
        across_removed_node(NEW_ON, SIX_NORTH, "{}", BEYOND_NORTH),
        "replaces a signal elsewhere",
    );
    // On the piece beyond the node, drawn the other way: the signal would
    // face the other way round.
    refused(
        across_removed_node(
            "{}",
            "node0 = 32, node1 = -1, tangent0 = v(0, -76.8, 0), tangent1 = v(0, -76.8, 0)",
            NEW_ON,
            BEYOND_NORTH,
        ),
        "replaces a signal elsewhere",
    );
}

/// As [`track_split_west`], with 30-31 a street and the pieces streets.
fn street_split_west(old: &str, south: &str, north: &str, west: &str) -> String {
    track_split_west(old, south, north, west)
        .replace(", type = 1, comp", ", type = 0, comp")
        .replace(
            "track/standard.track_template",
            "street/country.street_template",
        )
}

/// As [`track_refit`], on the street.
fn street_refit(old: &str, north: &str, adds: &str, removes: &str) -> String {
    street_split_west(old, "{}", north, "{}").replace(
        "removedNodes = {},",
        &format!(
            "removedNodes = {{}}, edgeObjectsToAdd = {adds}, edgeObjectsToRemove = {removes},"
        ),
    )
}

/// The street tool's capture, `gates` setting acceptance.lua first.
fn capture_street_with(street: &str, gates: &str) -> Result<LuaValue, String> {
    capture_with(
        street,
        &format!(
            "(function() local a = require('tpf3mp.acceptance') {gates} \
             return engine.captureBuild({{ toAdd = {{}}, toRemove = {{}}, proposal = street }}, 'Street') end)()"
        ),
    )
}

fn capture_stops(street: &str) -> Result<LuaValue, String> {
    capture_street_with(street, "a.keptStops = true")
}

/// The street tool's record of a new stop, as build 40420 lists it
/// (2026-10-19: the old stop's name, its side, no model).
const NEW_STOP: &str = "{ resultEntity = -400000000, category = 0, left = true, oneWay = false, playerEntity = 3892, name = 'Minami-dori' }";

const SMALL_STOP: &str = "::/stations/street/small_stops/small_new.con";
const TWO_SIDED: &str = "::/stations/street/small_stops/small_new_twosided.con";

fn kept_of(action: &Action) -> Vec<(usize, KeptSignal)> {
    let Action::BuildRoad(road) = action else {
        panic!("{action:?}")
    };
    road.polyline
        .links
        .iter()
        .enumerate()
        .flat_map(|(i, link)| link.kept.iter().map(move |k| (i, k.clone())))
        .collect()
}

/// A street started beside a junction near a stop (build 40420,
/// 2026-10-19, refused in a room before as "a build with a stop or
/// signal"): the stop goes with the piece of its street the tool put it
/// on, under its own entity, as a track's signal does; both stops of a
/// two-sided stop together.
#[test]
fn a_street_cut_beside_its_stop_keeps_it_on_its_piece() {
    let action = decode(
        capture_stops(&street_split_west(
            "{ { 560, 0 } }",
            "{}",
            "{ { 560, 0 } }",
            "{}",
        ))
        .unwrap(),
    );
    let kept = kept_of(&action);
    assert_eq!(kept.len(), 1, "{action:?}");
    assert_eq!(kept[0].0, 1);
    assert_eq!(kept[0].1.removal, 0);
    assert_eq!(kept[0].1.at, Fraction(750_000));
    assert_eq!(kept[0].1.model.as_str(), SMALL_STOP);

    let pair = decode(
        capture_stops(&street_split_west(
            "{ { 561, 0 }, { 562, 1 } }",
            "{}",
            "{ { 561, 0 }, { 562, 1 } }",
            "{}",
        ))
        .unwrap(),
    );
    let kept = kept_of(&pair);
    assert_eq!(kept.len(), 2, "{pair:?}");
    assert!(kept.iter().all(|(link, k)| *link == 1
        && k.removal == 0
        && k.at == Fraction(750_000)
        && k.model.as_str() == TWO_SIDED));

    // Off until its two-player game: refused when captured, and on sending
    // and applying alike.
    let gate = capture_street_with(
        &street_split_west("{ { 560, 0 } }", "{}", "{ { 560, 0 } }", "{}"),
        "a.keptStops = false",
    )
    .unwrap_err();
    assert!(
        gate.contains("keeps on a new piece awaits two-player game acceptance"),
        "{gate}"
    );
    let check = capture_with(
        &street_split_west("{ { 560, 0 } }", "{}", "{ { 560, 0 } }", "{}"),
        "(function() local acceptance = require('tpf3mp.acceptance') acceptance.keptStops = true \
         local a = engine.captureBuild({ toAdd = {}, toRemove = {}, proposal = street }, 'Street') \
         acceptance.keptStops = false local ok, why = acceptance.check(a) \
         return ok == false and why or 'let through' end)()",
    );
    assert_eq!(
        check,
        Ok(LuaValue::String(
            b"keptStops awaits two-player game acceptance".to_vec()
        ))
    );
    // A track's kept signal does not ask it.
    let signal = capture_with(
        &track_split("{ { 556, 2 } }", "{}", "{ { 556, 2 } }"),
        "(function() local acceptance = require('tpf3mp.acceptance') acceptance.keptStops = false \
         local a = engine.captureBuild({ toAdd = {}, toRemove = {}, proposal = street }, 'Track') \
         local ok, why = acceptance.check(a) return ok == false and why or 'let through' end)()",
    );
    assert_eq!(signal, Ok(LuaValue::String(b"let through".to_vec())));
}

#[test]
fn a_street_cut_that_moves_a_stop_elsewhere_is_refused() {
    let refused = |street: String, why: &str| {
        let got = capture_stops(&street).unwrap_err();
        assert!(got.contains(why), "{why}: {got}");
    };
    // Onto the new street west of the cut, which does not go on from it.
    refused(
        street_split_west("{ { 560, 0 } }", "{}", "{}", "{ { 560, 0 } }"),
        "does not go on from its track",
    );
    // Onto the other side of its street.
    refused(
        street_split_west("{ { 560, 0 } }", "{}", "{ { 560, 1 } }", "{}"),
        "moves a stop or signal",
    );
    // A signal on a street, or a stop onto a track from a street.
    refused(
        street_split_west("{ { 556, 2 } }", "{}", "{ { 556, 2 } }", "{}"),
        "moves a stop or signal",
    );
    // Dropped, or one of a two-sided stop's two.
    refused(
        street_split_west("{ { 560, 0 } }", "{}", "{}", "{}"),
        "removes an edge with a stop or signal",
    );
    refused(
        street_split_west("{ { 561, 0 }, { 562, 1 } }", "{}", "{ { 561, 0 } }", "{}"),
        "removes an edge with a stop or signal",
    );
}

/// A street started beside a junction near a stop, the tool replacing the
/// stop on the edge whose node it removes with a new record on the piece
/// that runs where it stood (build 40420, 2026-10-19): kept as the old
/// stop under its own entity, its lines, group and name its own, as a
/// track's signal the tool puts back.
#[test]
fn a_stop_the_street_tool_puts_back_is_kept_on_its_piece() {
    let refit = street_refit(
        "{ { 560, 0 } }",
        "{ { -400000000, 0 } }",
        &format!("{{ {NEW_STOP} }}"),
        "{ 560 }",
    );
    let decoded = decode(capture_stops(&refit).unwrap());
    let kept = decode(
        capture_stops(&street_split_west(
            "{ { 560, 0 } }",
            "{}",
            "{ { 560, 0 } }",
            "{}",
        ))
        .unwrap(),
    );
    assert_eq!(decoded, kept, "the same as the stop kept by its entity");

    // Both stops of a two-sided stop, each by its side.
    let records = format!(
        "{{ {NEW_STOP}, {} }}",
        NEW_STOP
            .replace("-400000000", "-400000001")
            .replace("left = true", "left = false")
    );
    let pair = street_refit(
        "{ { 561, 0 }, { 562, 1 } }",
        "{ { -400000001, 1 }, { -400000000, 0 } }",
        &records,
        "{ 562, 561 }",
    );
    let decoded = decode(capture_stops(&pair).unwrap());
    let kept = kept_of(&decoded);
    assert_eq!(kept.len(), 2, "{decoded:?}");
    assert!(
        kept.iter()
            .all(|(link, k)| *link == 1 && k.model.as_str() == TWO_SIDED)
    );

    let refused = |street: String, why: &str| {
        let got = capture_stops(&street).unwrap_err();
        assert!(got.contains(why), "{why}: {got}");
    };
    // The new stop on the other side, or its record saying another side.
    refused(
        street_refit(
            "{ { 560, 0 } }",
            "{ { -400000000, 1 } }",
            &format!("{{ {NEW_STOP} }}"),
            "{ 560 }",
        ),
        "a build with a stop or signal",
    );
    refused(
        street_refit(
            "{ { 560, 0 } }",
            "{ { -400000000, 1 } }",
            &format!("{{ {} }}", NEW_STOP.replace("left = true", "left = false")),
            "{ 560 }",
        ),
        "replaces a signal elsewhere",
    );
    // A record with no side, or a signal's record for a stop.
    refused(
        street_refit(
            "{ { 560, 0 } }",
            "{ { -400000000, 0 } }",
            &format!("{{ {} }}", NEW_STOP.replace("left = true, ", "")),
            "{ 560 }",
        ),
        "a build with a stop or signal",
    );
    refused(
        street_refit(
            "{ { 560, 0 } }",
            "{ { -400000000, 0 } }",
            &format!("{{ {} }}", NEW_STOP.replace("category = 0", "category = 2")),
            "{ 560 }",
        ),
        "a build with a stop or signal",
    );
    // Off: refused when captured.
    let gate = capture_street_with(&refit, "a.keptStops = false").unwrap_err();
    assert!(
        gate.contains("keeps on a new piece awaits two-player game acceptance"),
        "{gate}"
    );
}

#[test]
fn a_signal_the_tool_replaces_otherwise_is_refused() {
    let refused = |street: String, why: &str| {
        let got = capture_refit(&street).unwrap_err();
        assert!(got.contains(why), "{got}");
    };
    let north = "{ { -400000000, 2 } }";
    // A new signal with nothing removed, or one removed with none added.
    refused(
        track_refit("{}", "{}", north, "{}", NEW_SIGNAL, "{}"),
        "a build with a stop or signal",
    );
    refused(
        track_refit("{ { 556, 2 } }", "{}", "{}", "{}", "{}", "{ 556 }"),
        "a build with a stop or signal",
    );
    // A waypoint's or a stop's record.
    for category in [0, 1] {
        refused(
            track_refit(
                "{ { 556, 2 } }",
                "{}",
                north,
                "{}",
                &NEW_SIGNAL.replace("category = 2", &format!("category = {category}")),
                "{ 556 }",
            ),
            "a build with a stop or signal",
        );
    }
    // A removed signal on no edge the build removes.
    refused(
        track_refit("{}", "{}", north, "{}", NEW_SIGNAL, "{ 558 }"),
        "a build with a stop or signal",
    );
    // The new signal on the new track west of the cut, which does not go on
    // from the old one.
    refused(
        track_refit("{ { 556, 2 } }", "{}", "{}", north, NEW_SIGNAL, "{ 556 }"),
        "replaces a signal elsewhere",
    );
    // The record's signal on no new edge, or listed under another entity.
    refused(
        track_refit(
            "{ { 556, 2 } }",
            "{}",
            "{ { -400000001, 2 } }",
            "{}",
            NEW_SIGNAL,
            "{ 556 }",
        ),
        "a build with a stop or signal",
    );
    // The signal it removes also kept by its entity.
    refused(
        track_refit(
            "{ { 556, 2 } }",
            "{ { 556, 2 } }",
            north,
            "{}",
            NEW_SIGNAL,
            "{ 556 }",
        ),
        "a build with a stop or signal",
    );
    // A waypoint put back: lines name it.
    refused(
        track_refit("{ { 557, 2 } }", "{}", north, "{}", NEW_SIGNAL, "{ 557 }"),
        "moves a waypoint",
    );
    // Two signals of one edge for two new ones on one piece: which is which,
    // the proposal does not say.
    refused(
        track_refit(
            "{ { 556, 2 }, { 558, 2 } }",
            "{}",
            "{ { -400000000, 2 }, { -400000001, 2 } }",
            "{}",
            "{ { resultEntity = -400000000, category = 2 }, { resultEntity = -400000001, category = 2 } }",
            "{ 556, 558 }",
        ),
        "cannot tell apart",
    );
    // On the piece before the cut, which goes on from the old track's end
    // but not past where the signal stood, three quarters of the way north.
    refused(
        track_refit(
            "{ { 556, 2 } }",
            "{ { -400000000, 2 } }",
            "{}",
            "{}",
            NEW_SIGNAL,
            "{ 556 }",
        ),
        "replaces a signal elsewhere",
    );
    // On the edge rebuilt between the old one's own nodes: a waypoint is
    // refused before anything else, a signal as not put back on a piece.
    let in_place = |old: u32| {
        format!(
            "{{ addedNodes = {{}},
                addedSegments = {{ {{ entity = -3, type = 1, comp = {{ node0 = 30, node1 = 31,
                    tangent0 = v(0, 60, 0), tangent1 = v(0, 60, 0), type = 0, typeIndex = -1,
                    roadTemplate = 'track/standard.track_template', roadStyle = '',
                    objects = {{ {{ -400000000, 2 }} }} }} }} }},
                removedSegments = {{ {{ entity = 110, type = 1, comp = {{ node0 = 30, node1 = 31,
                    tangent0 = v(0, 60, 0), tangent1 = v(0, 60, 0), objects = {{ {{ {old}, 2 }} }} }} }} }},
                removedNodes = {{}}, edgeObjectsToAdd = {NEW_SIGNAL}, edgeObjectsToRemove = {{ {old} }} }}"
        )
    };
    refused(in_place(557), "moves a waypoint");
    refused(in_place(556), "replaces a signal elsewhere");
    // A construction's streets keep none.
    let connection = capture_with(
        &track_refit("{ { 556, 2 } }", "{}", north, "{}", NEW_SIGNAL, "{ 556 }"),
        "engine.fromProposal({ toAdd = {}, toRemove = {}, proposal = street }, nil, true)",
    )
    .unwrap_err();
    assert!(
        connection.contains("a build with a stop or signal"),
        "{connection}"
    );
}

/// Runs the asset bulldozer's capture (`engine.captureAssets`) over three
/// asset groups as build 40408 holds them, and returns the action table,
/// or why it was refused. 50 holds four firs as thin instances; 51 a fir
/// and a boulder, a full instance with its own matrix; 52 one boulder.
/// `TOOL(group, removed)` makes the tool's proposal as
/// UI::AssetBulldozerAction does: the group removed and, unless every asset
/// went, one construction entity at the origin whose models are the assets
/// kept, thin ones then full ones, each its file and world matrix.
fn capture_assets(call: &str) -> Result<LuaValue, String> {
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
    let (action, why): (Value, Option<String>) = lua
        .load(format!(
            "local FILES = {{ [41] = 'assets/trees/fir.mdl', [42] = 'assets/rocks/boulder.mdl' }}
             local function fir(x, y, rot)
                 return {{ modelId = 41, pos = {{ x = x, y = y, z = 3 }}, rot = rot, scale = 1.25 }}
             end
             local function boulder(x, y)
                 return {{ modelId = 42, transf = {{ 0, 2, 0, 0, -2, 0, 0, 0, 0, 0, 2, 0, x, y, 2, 1 }} }}
             end
             local THIN = {{ [50] = {{ fir(10, 20, 0), fir(14, 21, 0.5), fir(18, 19, 1), fir(22, 20, 2) }},
                             [51] = {{ fir(600, 50, 0.25) }}, [52] = {{}} }}
             local FULL = {{ [51] = {{ boulder(602, 50) }}, [52] = {{ boulder(700, 60) }} }}
             api = {{
                 type = {{ ComponentType = {{ ASSET_GROUP = 30, MODEL_INSTANCE_LIST = 31 }} }},
                 res = {{ modelRep = {{ getName = function(id) return FILES[id] end }} }},
                 engine = {{ getComponent = function(e, kind)
                     if kind == 30 and THIN[e] then return {{}} end
                     if kind == 31 and THIN[e] then
                         return {{ thinInstances = THIN[e], fatInstances = FULL[e] or {{}} }}
                     end
                 end }},
             }}
             local engine = require('tpf3mp.engine')
             function TOOL(group, removed)
                 local gone, models, n = {{}}, {{}}, 0
                 for _, i in ipairs(removed) do gone[i] = true end
                 for _, t in ipairs(THIN[group]) do
                     n = n + 1
                     if not gone[n] then
                         models[#models + 1] = {{ id = '::/' .. FILES[t.modelId], thin = false,
                             transf = engine.assetMatrix({{ x = t.pos.x, y = t.pos.y, z = t.pos.z,
                                 rot = t.rot, scale = t.scale }}, false) }}
                     end
                 end
                 for _, f in ipairs(FULL[group] or {{}}) do
                     n = n + 1
                     if not gone[n] then
                         models[#models + 1] = {{ id = '::/' .. FILES[f.modelId], thin = false, transf = f.transf }}
                     end
                 end
                 local toAdd = {{}}
                 if #models > 0 then
                     toAdd[1] = {{ fileName = '', playerEntity = -1,
                         transf = {{ 1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1 }},
                         construction = {{ subconstructions = {{ {{ models = models }} }} }} }}
                 end
                 return {{ toRemove = {{ group }}, toAdd = toAdd,
                     proposal = {{ addedNodes = {{}}, addedSegments = {{}}, removedNodes = {{}},
                         removedSegments = {{}} }} }}
             end
             return {call}"
        ))
        .eval()
        .map_err(|error| error.to_string())?;
    match (action, why) {
        (Value::Nil, why) => Err(why.unwrap_or_default()),
        (action, _) => Ok(tree(&action)),
    }
}

fn asset(model: &str, x: i32, y: i32, z: i32) -> AssetRef {
    AssetRef {
        model: text(model),
        at: pos(x, y, z),
    }
}

/// The three shapes the asset bulldozer makes decode as the schema says,
/// in millimetres: a fir taken out of a group of thin instances, the rest
/// rebuilt; a boulder (a full instance) taken out of a group with a fir;
/// a group of one boulder taken whole, nothing rebuilt.
#[test]
fn an_asset_bulldoze_decodes_as_the_schema_says() {
    const FIR: &str = "::/assets/trees/fir.mdl";
    const BOULDER: &str = "::/assets/rocks/boulder.mdl";
    let removal = |call: &str| match decode(capture_assets(call).unwrap()) {
        Action::Bulldoze(Bulldoze::Assets(removal)) => removal,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        removal("engine.captureAssets(TOOL(50, { 2 }))"),
        AssetRemoval {
            first: asset(FIR, 10_000, 20_000, 3_000),
            count: 4,
            removed: list(vec![asset(FIR, 14_000, 21_000, 3_000)]),
            mirrored: false,
            owned: false,
        }
    );
    assert_eq!(
        removal("engine.captureAssets(TOOL(51, { 2 }))"),
        AssetRemoval {
            first: asset(FIR, 600_000, 50_000, 3_000),
            count: 2,
            removed: list(vec![asset(BOULDER, 602_000, 50_000, 2_000)]),
            mirrored: false,
            owned: false,
        }
    );
    assert_eq!(
        removal("engine.captureAssets(TOOL(52, { 1 }))"),
        AssetRemoval {
            first: asset(BOULDER, 700_000, 60_000, 2_000),
            count: 1,
            removed: list(vec![asset(BOULDER, 700_000, 60_000, 2_000)]),
            mirrored: false,
            owned: false,
        }
    );
    // A proposal that also changes streets is no asset bulldoze.
    let why = capture_assets(
        "engine.captureAssets((function() local p = TOOL(52, { 1 }) \
             p.proposal.removedSegments = { { entity = 100 } } return p end)())",
    )
    .unwrap_err();
    assert_eq!(why, "an asset bulldoze that changes streets too");
}

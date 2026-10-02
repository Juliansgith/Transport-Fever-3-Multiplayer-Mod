//! The mod's pending builds (`tpf3mp/ghost.lua`): a new road's loose ends
//! are named by what they land on among the player's builds the room has not
//! answered yet, by position, as every receiver resolves a vertex
//! (docs/BUILDING.md, "Resolving a vertex"). The joined action must still be
//! one the schema takes, and what cannot be named exactly is refused.
//! Lua 5.1 here (the workspace's vendored Lua); the mod targets 5.2 and keeps
//! to what both run.

#![allow(clippy::unwrap_used)]

mod common;

use common::tree;
use mlua::{Lua, Table, Value};
use tpf3mp_proto::action::{
    Action, EdgeEnds, EdgeRef, Network, Pos, Resolve, RoadBuild, TrackBuild,
};
use tpf3mp_proto::lua::action_from_lua;

const GEOM: &str = include_str!("../../../mod/tpf3mp_1/content/scripts/tpf3mp/geom.lua");
const GHOST: &str = include_str!("../../../mod/tpf3mp_1/content/scripts/tpf3mp/ghost.lua");

/// Helpers for the tests' Lua: a straight road or track in metres, from
/// `a` to `b`, every vertex `New` unless `resolve` says otherwise.
const HELPERS: &str = r"
ghost = require 'tpf3mp.ghost'
function v(x, y, z, resolve) return { pos = { x = x, y = y, z = z or 0 }, resolve = resolve or 'New' } end
function l(from, to, a, b)
    local t = { x = b.pos.x - a.pos.x, y = b.pos.y - a.pos.y, z = b.pos.z - a.pos.z }
    return { from = from, to = to, tangent0 = t, tangent1 = t, structure = 'Ground' }
end
function road(vertices, links)
    return { BuildRoad = { street = '::/street/town_small.street_template', bus_lane = false,
        tram = 'None', polyline = { vertices = vertices, links = links, removals = {},
        removed_nodes = {} } } }
end
function track(vertices, links)
    return { BuildTrack = { track = '::/track/standard.lua', catenary = false,
        polyline = { vertices = vertices, links = links, removals = {}, removed_nodes = {} } } }
end
-- A straight road from a to b, as a one-link build.
function straight(make, a, b) return make({ a, b }, { l(0, 1, a, b) }) end
-- Pending builds as tpf3mp_native.pending() lists them.
function pending(...)
    local out = {}
    for i, action in ipairs({ ... }) do out[i] = { ticket = i, action = action } end
    return out
end
-- How each vertex of a joined build resolves, and where, in one line.
function said(action)
    local build = action.BuildRoad or action.BuildTrack
    local out = {}
    for _, vx in ipairs(build.polyline.vertices) do
        local r = vx.resolve
        local what = type(r) == 'string' and r or (r.Node and ('Node ' .. r.Node))
            or string.format('Split %s %g,%g-%g,%g', r.Split.network, r.Split.ends.a.x, r.Split.ends.a.y,
                r.Split.ends.b.x, r.Split.ends.b.y)
        out[#out + 1] = string.format('%s@%g,%g', what, vx.pos.x, vx.pos.y)
    end
    return table.concat(out, ' ')
end
";

fn lua() -> Lua {
    let lua = Lua::new();
    let preload: Table = lua
        .globals()
        .get::<Table>("package")
        .unwrap()
        .get("preload")
        .unwrap();
    for (name, source) in [("geom", GEOM), ("ghost", GHOST)] {
        let chunk = lua
            .load(source)
            .set_name(format!("@tpf3mp/{name}.lua"))
            .into_function()
            .unwrap();
        preload.set(format!("tpf3mp.{name}"), chunk).unwrap();
    }
    lua.load(HELPERS).set_name("@helpers").exec().unwrap();
    lua
}

/// Joins the build `action` (Lua) onto the pending builds `pending` (Lua):
/// how its vertices resolve and how many ends were joined, or why not.
fn join(action: &str, pending: &str) -> Result<(String, u32), String> {
    let lua = lua();
    let (joined, count): (Value, Value) = lua
        .load(format!(
            "local a, n = ghost.join({action}, pending({pending})) \
             if a == nil then return nil, n end \
             return said(a), n"
        ))
        .eval()
        .unwrap_or_else(|error| panic!("{error}"));
    match (joined, count) {
        (Value::String(said), Value::Integer(n)) => {
            Ok((said.to_str().unwrap().to_owned(), u32::try_from(n).unwrap()))
        }
        (Value::String(said), Value::Number(n)) =>
        {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Ok((said.to_str().unwrap().to_owned(), n as u32))
        }
        (Value::Nil, Value::String(why)) => Err(why.to_str().unwrap().to_owned()),
        other => panic!("unexpected {other:?}"),
    }
}

/// A pending street from (0,0) to (100,0).
const PENDING_STREET: &str = "straight(road, v(0, 0), v(100, 0))";

#[test]
fn a_road_started_at_a_pending_roads_end_is_joined_to_that_node() {
    // The tool planted the new road's first node 1.2 m off the pending
    // road's end: it is that node, where the pending road has it.
    assert_eq!(
        join("straight(road, v(100.9, 0.8), v(100, 60))", PENDING_STREET),
        Ok(("Node Street@100,0 New@100,60".into(), 1))
    );
}

#[test]
fn a_road_started_on_a_pending_roads_middle_splits_it_on_its_curve() {
    assert_eq!(
        join("straight(road, v(40, 1.5), v(40, 60))", PENDING_STREET),
        Ok(("Split Street 0,0-100,0@40,0 New@40,60".into(), 1))
    );
    // Near an end along the curve, it is that end's node, never a split
    // the receivers refuse.
    assert_eq!(
        join("straight(road, v(2, 1), v(2, 60))", PENDING_STREET),
        Ok(("Node Street@0,0 New@2,60".into(), 1))
    );
}

#[test]
fn what_lands_on_nothing_pending_or_on_another_network_is_left_as_the_tool_made_it() {
    assert_eq!(
        join("straight(road, v(40, 3), v(40, 60))", PENDING_STREET),
        Ok(("New@40,3 New@40,60".into(), 0)),
        "3 m off the centreline: not on it"
    );
    assert_eq!(
        join("straight(track, v(100, 0), v(100, 60))", PENDING_STREET),
        Ok(("New@100,0 New@100,60".into(), 0)),
        "a track never joins a pending street"
    );
    // A vertex with two links is not a loose end: a road through the pending
    // one's end goes to the room as the tool made it.
    assert_eq!(
        join(
            "road({ v(100, -50), v(100, 0.5), v(100, 50) }, \
                  { l(0, 1, v(100, -50), v(100, 0.5)), l(1, 2, v(100, 0.5), v(100, 50)) })",
            PENDING_STREET
        ),
        Ok(("New@100,-50 New@100,0.5 New@100,50".into(), 0))
    );
    // An end the tool snapped to an existing node is the tool's.
    assert_eq!(
        join(
            "straight(road, v(100, 0, 0, { Node = 'Street' }), v(100, 60))",
            PENDING_STREET
        ),
        Ok(("Node Street@100,0 New@100,60".into(), 0))
    );
}

#[test]
fn nothing_pending_leaves_every_action_as_it_was() {
    assert_eq!(
        join("straight(road, v(100, 0), v(100, 60))", ""),
        Ok(("New@100,0 New@100,60".into(), 0))
    );
    let lua = lua();
    let same: bool = lua
        .load(
            "local other = { Bulldoze = {} } \
             return ghost.join(other, pending(straight(road, v(0, 0), v(100, 0)))) == other",
        )
        .eval()
        .unwrap();
    assert!(same, "only road and track builds are joined");
}

#[test]
fn a_pending_split_of_a_pending_road_is_followed_to_its_halves() {
    // The first pending road, then a second pending one splitting it at
    // x = 50: a third road started at x = 20 splits the first half, between
    // the first road's start and the split.
    let first = PENDING_STREET;
    let second = "straight(road, v(50, 0, 0, { Split = { network = 'Street', \
                  ends = { a = { x = 0, y = 0, z = 0 }, b = { x = 100, y = 0, z = 0 } } } }), v(50, 60))";
    assert_eq!(
        join(
            "straight(road, v(20, 1), v(20, -60))",
            &format!("{first}, {second}")
        ),
        Ok(("Split Street 0,0-50,0@20,0 New@20,-60".into(), 1))
    );
    // A pending removal of the pending road leaves nothing there to split.
    let removal = "road({ v(0, 30), v(10, 30) }, { l(0, 1, v(0, 30), v(10, 30)) })";
    let lua = lua();
    let said: String = lua
        .load(format!(
            "local r = {removal} \
             r.BuildRoad.polyline.removals = {{ {{ network = 'Street', \
                 ends = {{ a = {{ x = 0, y = 0, z = 0 }}, b = {{ x = 100, y = 0, z = 0 }} }} }} }} \
             local a = ghost.join(straight(road, v(40, 1), v(40, -60)), pending({first}, r)) \
             return said(a)"
        ))
        .eval()
        .unwrap();
    assert_eq!(said, "New@40,1 New@40,-60");
}

#[test]
fn what_cannot_be_named_exactly_is_refused() {
    // Between two pending roads, each within reach: which one is a guess.
    assert_eq!(
        join(
            "straight(road, v(50, 1), v(50, 60))",
            "straight(road, v(0, 0), v(100, 0)), straight(road, v(0, 2.5), v(100, 2.5))"
        ),
        Err("a loose end the room cannot name: it ends where two pending roads meet".into())
    );
    // Both ends of one new edge on one pending node.
    assert_eq!(
        join("straight(road, v(100.5, 0), v(99.6, 0.4))", PENDING_STREET),
        Err("both ends of one edge on the same pending node".into())
    );
}

#[test]
fn a_joined_build_is_one_the_schema_takes() {
    let lua = lua();
    let action: Value = lua
        .load(format!(
            "return ghost.join(straight(road, v(40, 1.5, 3), v(40, 60, 3)), pending({PENDING_STREET}))"
        ))
        .eval()
        .unwrap();
    let action = action_from_lua(&tree(&action)).unwrap();
    let Action::BuildRoad(RoadBuild { polyline, .. }) = action else {
        panic!("a road")
    };
    assert_eq!(
        polyline.vertices[0].resolve,
        Resolve::Split(EdgeRef {
            network: Network::Street,
            ends: EdgeEnds {
                a: Pos { x: 0, y: 0, z: 0 },
                b: Pos {
                    x: 100_000,
                    y: 0,
                    z: 0
                },
            },
        })
    );
    assert_eq!(
        polyline.vertices[0].pos,
        Pos {
            x: 40_000,
            y: 0,
            z: 0
        },
        "on the pending road's curve, at its height"
    );
    // A joined track too.
    let track: Value = lua
        .load(
            "return ghost.join(straight(track, v(100.4, 0), v(100, 60)), \
                 pending(straight(track, v(0, 0), v(100, 0))))",
        )
        .eval()
        .unwrap();
    let Action::BuildTrack(TrackBuild { polyline, .. }) = action_from_lua(&tree(&track)).unwrap()
    else {
        panic!("a track")
    };
    assert_eq!(polyline.vertices[0].resolve, Resolve::Node(Network::Track));
}

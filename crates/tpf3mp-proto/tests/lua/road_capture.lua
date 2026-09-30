-- Unit test of the Lua mod's road and track capture (mod/tpf3mp_1,
-- tpf3mp/roads.lua), in plain Lua against a world made of tables. Run by
-- tests/lua_capture.rs, which preloads the mod's modules and converts the
-- tables this returns with the Rust schema (tpf3mp_proto::lua), as the hook
-- does; any Lua 5.1 or 5.2 that can `require "tpf3mp.roads"` runs it too.
--
-- Returns the action tables of the road and the track build below, in
-- metres, and a table of captures changed so that the schema must refuse
-- them.

local roads = require "tpf3mp.roads"

-- ---------- the world: existing nodes, in metres ----------

local nodes = {
	[7] = { "Street", { 100, 200, 10 } },
	[10] = { "Street", { 100, 300, 10 } },
	-- a country street A-M-B along y = 0
	[20] = { "Street", { 400, 0, 2 } },
	[21] = { "Street", { 450, 0, 2 } },
	[22] = { "Street", { 500, 0, 2 } },
	[30] = { "Track", { 0, 0, 1 } },
	[41] = { "Street", { 50, -40, 1 } },
	[42] = { "Street", { 50, 40, 1 } },
	[43] = { "Street", { 200, 0, 1 } },
}
local world = {
	nodePos = function(id) return nodes[id] and nodes[id][2] end,
	nodeNetwork = function(id) return nodes[id] and nodes[id][1] end,
}

local TOWN = "street/town_medium.street_template"
local COUNTRY = "street/country.street_template"
local FAST = "track/high_speed.street_template"

-- ---------- a road: from a junction, over a bridge, onto a street's middle ----------
--
-- As TF3's street tool proposes it (seen on build 40408): the new junction
-- -3 lands near the country street's node 21, so the tool removes 21 and
-- its two edges and rebuilds the street from 20 and 22 through -3, in the
-- street's own template. The town street 7-10 is replaced in place (an
-- upgrade).
local road = {
	network = "Street",
	street = TOWN, style = "style/old_town.lua", bus_lane = false, tram = "None",
	nodes = {
		{ id = -1, pos = { 160, 200, 12 } },
		{ id = -2, pos = { 220, 230, 15.0004 } },
		{ id = -3, pos = { 452, 0, 2 } },
	},
	edges = {
		{ node0 = 7, node1 = -1, network = "Street", tangent0 = { 60, 0, 2 }, tangent1 = { 60, 0, 2 },
		  template = TOWN, style = "style/old_town.lua" },
		{ node0 = -1, node1 = -2, network = "Street", tangent0 = { 55, 20, 0 }, tangent1 = { 60, 28.25, -2 },
		  structure = { Bridge = "bridge/cement.lua" }, template = TOWN, style = "style/old_town.lua" },
		{ node0 = -2, node1 = -3, network = "Street", tangent0 = { 232, -230, -13 }, tangent1 = { 232, -230, -13 },
		  template = TOWN, style = "style/old_town.lua" },
		{ node0 = 20, node1 = -3, network = "Street", tangent0 = { 52, 0, 0 }, tangent1 = { 52, 0, 0 },
		  template = COUNTRY },
		{ node0 = -3, node1 = 22, network = "Street", tangent0 = { 48, 0, 0 }, tangent1 = { 48, 0, 0 },
		  template = COUNTRY },
	},
	removed = {
		{ node0 = 20, node1 = 21, network = "Street" },
		{ node0 = 21, node1 = 22, network = "Street" },
		{ node0 = 10, node1 = 7, network = "Street" },
	},
	removedNodes = { { id = 21, pos = { 450, 0, 2 } } },
}

local action = assert(roads.capture(road, world))
local build = action.BuildRoad
assert(build and build.street == TOWN and build.style == "style/old_town.lua" and build.tram == "None")
local poly = build.polyline
assert(#poly.vertices == 6, "every node an edge names, once")
assert(poly.vertices[1].resolve.Node == "Street")
assert(poly.vertices[2].resolve == "New" and poly.vertices[4].resolve == "New")
assert(poly.vertices[3].pos.z == 15.0004, "in metres, as the game gave it")
assert(#poly.links == 5, "the rebuilt street's pieces are links too")
assert(poly.links[1].from == 0 and poly.links[1].to == 1, "indices start at 0")
assert(poly.links[1].kind == nil, "the build's own kind")
assert(poly.links[2].structure.Bridge == "bridge/cement.lua")
assert(poly.links[4].kind.template == COUNTRY and poly.links[4].kind.style == nil, "the street's own kind")
assert(#poly.removals == 3 and poly.removals[3].ends.a.y == 300)
assert(#poly.removed_nodes == 1 and poly.removed_nodes[1].at.x == 450)
local roadAction = action

-- ---------- a track: a level crossing mid-street, a tunnel, a crossing at a street node ----------
--
-- The track crosses the street 41-42 at -1: the street is removed and
-- rebuilt through the crossing, in its own kind.
local track = {
	network = "Track",
	track = FAST, catenary = true,
	nodes = {
		{ id = -1, pos = { 50, 0.0005, 1.25 } },
		{ id = -2, pos = { 150, -0.0005, 1 } },
	},
	edges = {
		{ node0 = 30, node1 = -1, network = "Track", tangent0 = { 50, 0, 0.25 }, tangent1 = { 50, 0, 0.25 },
		  template = FAST },
		{ node0 = 41, node1 = -1, network = "Street", tangent0 = { 0, 40, 0 }, tangent1 = { 0, 40, 0 },
		  template = COUNTRY },
		{ node0 = -1, node1 = 42, network = "Street", tangent0 = { 0, 40, 0 }, tangent1 = { 0, 40, 0 },
		  template = COUNTRY },
		{ node0 = -1, node1 = -2, network = "Track", tangent0 = { 100, 0, 0 }, tangent1 = { 100, 0, 0 },
		  structure = { Tunnel = "tunnel/concrete.lua" }, template = FAST },
		{ node0 = -2, node1 = 43, network = "Track", tangent0 = { 50, 0, 0 }, tangent1 = { 50, 0, 0 },
		  template = FAST },
	},
	removed = { { node0 = 42, node1 = 41, network = "Street" } },
}

action = assert(roads.capture(track, world))
build = action.BuildTrack
assert(build and build.track == FAST and build.catenary == true)
poly = build.polyline
assert(#poly.vertices == 6 and #poly.links == 5 and #poly.removals == 1 and #poly.removed_nodes == 0)
assert(poly.vertices[1].resolve.Node == "Track")
assert(poly.links[2].kind.network == "Street", "the crossed street keeps its kind")
assert(poly.vertices[2].pos.y == 0.0005 and poly.vertices[5].pos.y == -0.0005, "rounding is the hook's")
assert(poly.vertices[6].resolve.Node == "Street", "a crossing at a street node")
local trackAction = action

-- ---------- captures that must not travel ----------

local function refused(capture, why)
	local got, reason = roads.capture(capture, world)
	assert(got == nil and type(reason) == "string", why)
end

-- only removals: nothing to build
refused({ network = "Street", street = "s", nodes = {}, edges = {},
	removed = { { node0 = 20, node1 = 21, network = "Street" } } }, "no edges")
-- a removal of an edge whose ends are nowhere
refused({ network = "Street", street = "s", nodes = {}, edges = { { node0 = 7, node1 = 10,
	network = "Street", tangent0 = { 1, 0, 0 }, tangent1 = { 1, 0, 0 }, template = "s" } },
	removed = { { node0 = 7, node1 = 999, network = "Street" } } }, "an unplaceable removal")
-- a street edge in a track build without its template
refused({ network = "Track", track = "t", nodes = {}, edges = { { node0 = 7, node1 = 10,
	network = "Street", tangent0 = { 1, 0, 0 }, tangent1 = { 1, 0, 0 } } } }, "a kind without its template")
-- a position that is not a number
refused({ network = "Street", street = "s", nodes = { { id = -1, pos = { 0 / 0, 0, 0 } } },
	edges = { { node0 = 7, node1 = -1, network = "Street", tangent0 = { 1, 0, 0 },
	tangent1 = { 1, 0, 0 }, template = "s" } } }, "NaN")
-- a node with no network
refused({ network = "Street", street = "s", nodes = {}, edges = { { node0 = 7, node1 = 12345,
	network = "Street", tangent0 = { 1, 0, 0 }, tangent1 = { 1, 0, 0 }, template = "s" } } }, "a node nowhere")
-- a removed node that is nowhere
refused({ network = "Street", street = "s", nodes = {}, edges = { { node0 = 7, node1 = 10,
	network = "Street", tangent0 = { 1, 0, 0 }, tangent1 = { 1, 0, 0 }, template = "s" } },
	removedNodes = { { id = 999 } } }, "a removed node nowhere")

-- Captures the schema must refuse, which the hook refuses in Rust
-- (tests/lua_capture.rs checks each).
local mustRefuse = {}
local function changed(why, mutate)
	local a = assert(roads.capture(road, world))
	mutate(a.BuildRoad)
	mustRefuse[why] = a
end
changed("a control character", function(b) b.street = "street\nname" end)
changed("a long name", function(b) b.street = string.rep("s", 129) end)
changed("a link past the vertices", function(b) b.polyline.links[1].to = 6 end)
changed("a link to itself", function(b) b.polyline.links[1].to = 0 end)
changed("past i32", function(b) b.polyline.vertices[1].pos.x = 2147484 end)
changed("an infinite position", function(b) b.polyline.vertices[1].pos.x = 1 / 0 end)
changed("an unknown variant", function(b) b.tram = "Diesel" end)
changed("no links", function(b) b.polyline.links = {} end)
changed("a kind of no network", function(b) b.polyline.links[4].kind.network = "Air" end)
changed("a removed node past i32", function(b) b.polyline.removed_nodes[1].at.y = -2147484 end)

return roadAction, trackAction, mustRefuse

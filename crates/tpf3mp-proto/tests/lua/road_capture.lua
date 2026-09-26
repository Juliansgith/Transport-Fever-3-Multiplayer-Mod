-- Unit test of the Lua mod's road and track capture (mod/tpf3mp_1,
-- tpf3mp/roads.lua and tpf3mp/wire.lua), in plain Lua against a world made
-- of tables. Run by tests/lua_capture.rs, which preloads the mod's modules
-- and decodes the payloads this returns with the Rust schema; any Lua 5.1
-- or 5.2 that can `require "tpf3mp.roads"` runs it too.
--
-- Returns the payload bytes of the road and the track build below.

local roads = require "tpf3mp.roads"
local wire = require "tpf3mp.wire"
local geom = require "tpf3mp.geom"

-- ---------- the world: existing nodes and edges, in metres ----------

local nodes = {
	[7] = { "Street", { 100, 200, 10 } },
	[8] = { "Street", { 300, 0, 5 } },
	[9] = { "Street", { 300, 100, 5 } },
	[10] = { "Street", { 100, 300, 10 } },
	[30] = { "Track", { 0, 0, 1 } },
	[32] = { "Track", { -100, 0, 1 } },
	[41] = { "Street", { 50, -40, 1 } },
	[42] = { "Street", { 50, 40, 1 } },
	[43] = { "Street", { 200, 0, 1 } },
}
local function edge(n0, n1, ta, tb)
	return { node0 = n0, node1 = n1, a = nodes[n0][2], b = nodes[n1][2], ta = ta, tb = tb }
end
local edges = {
	Street = {
		edge(8, 9, { 0, 100, 0 }, { 0, 100, 0 }),
		edge(7, 10, { 0, 100, 0 }, { 0, 100, 0 }),
		edge(41, 42, { 0, 80, 0 }, { 0, 80, 0 }),
	},
	Track = { edge(30, 32, { -100, 0, 0 }, { -100, 0, 0 }) },
}
local world = {
	nodePos = function(id) return nodes[id] and nodes[id][2] end,
	nodeNetwork = function(id) return nodes[id] and nodes[id][1] end,
	edgeUnder = function(network, x, y)
		local tol = network == "Track" and geom.SPLIT_EPS_TRACK or geom.SPLIT_EPS
		local e = geom.edgeContaining(edges[network], x, y, tol)
		if e then return e.node0, e.node1 end
	end,
}

local function flat(t) return { t[1], 0, t[3] or 0 } end

-- ---------- a road: from a junction, over a bridge, onto a street's middle ----------
--
-- Node -3 lands mid-span on the street 8-9, so the engine split it: the
-- halves 8 -> -3 and -3 -> 9 and the removal of 8-9 are in the proposal and
-- must not travel. The street 7-10 is replaced in place (an upgrade), so its
-- removal must.
local road = {
	network = "Street",
	street = "street/standard/town_medium_new.lua", bus_lane = true, tram = "Electric",
	nodes = {
		{ id = -1, pos = { 160, 200, 12 } },
		{ id = -2, pos = { 220, 230, 15.0004 } },
		{ id = -3, pos = { 300, 50, 5 } },
	},
	edges = {
		{ node0 = 7, node1 = -1, network = "Street", tangent0 = { 60, 0, 2 }, tangent1 = { 60, 0, 2 } },
		{ node0 = -1, node1 = -2, network = "Street", tangent0 = { 55, 20, 0 }, tangent1 = { 60, 28.25, -2 },
		  structure = { Bridge = "bridge/cement.lua" } },
		{ node0 = 8, node1 = -3, network = "Street", tangent0 = { 0, 50, 0 }, tangent1 = { 0, 50, 0 } },
		{ node0 = -3, node1 = 9, network = "Street", tangent0 = { 0, 50, 0 }, tangent1 = { 0, 50, 0 } },
		{ node0 = -2, node1 = -3, network = "Street", tangent0 = { 80, -180, -10 }, tangent1 = { 80, -180, -10 } },
	},
	removed = { { node0 = 8, node1 = 9 }, { node0 = 10, node1 = 7 } },
}

local action = assert(roads.capture(road, world))
local build = action.BuildRoad
assert(build and build.street == road.street and build.bus_lane == true and build.tram == "Electric")
local poly = build.polyline
assert(#poly.vertices == 4, "the split halves' existing ends are not vertices")
assert(poly.vertices[1].resolve.Node == "Street")
assert(poly.vertices[2].resolve == "New" and poly.vertices[3].resolve == "New")
assert(poly.vertices[3].pos.z == 15000, "rounded to the millimetre")
local split = poly.vertices[4].resolve.Split
assert(split and split.network == "Street" and split.ends.a.y == 0 and split.ends.b.y == 100000)
assert(#poly.links == 3, "the split halves are dropped")
assert(poly.links[1].from == 0 and poly.links[1].to == 1, "indices start at 0")
assert(poly.links[2].structure.Bridge == "bridge/cement.lua")
assert(#poly.removals == 1 and poly.removals[1].a.y == 300000, "only the in-place replacement is removed")
local roadBytes = wire.encode(action)

-- ---------- a track: a level crossing mid-street, a tunnel, a crossing at a street node ----------
--
-- Node -1 lands on the street 41-42 (the other network): the engine split
-- the street, and its street halves are in the track proposal.
local track = {
	network = "Track",
	track = "high_speed.lua", catenary = true,
	nodes = {
		{ id = -1, pos = { 50, 0.0005, 1.25 } },
		{ id = -2, pos = { 150, -0.0005, 1 } },
	},
	edges = {
		{ node0 = 30, node1 = -1, network = "Track", tangent0 = { 50, 0, 0.25 }, tangent1 = { 50, 0, 0.25 } },
		{ node0 = 41, node1 = -1, network = "Street", tangent0 = { 0, 40, 0 }, tangent1 = { 0, 40, 0 } },
		{ node0 = -1, node1 = 42, network = "Street", tangent0 = { 0, 40, 0 }, tangent1 = { 0, 40, 0 } },
		{ node0 = -1, node1 = -2, network = "Track", tangent0 = { 100, 0, 0 }, tangent1 = { 100, 0, 0 },
		  structure = { Tunnel = "tunnel/concrete.lua" } },
		{ node0 = -2, node1 = 43, network = "Track", tangent0 = { 50, 0, 0 }, tangent1 = { 50, 0, 0 } },
	},
	removed = { { node0 = 42, node1 = 41 } },
}

action = assert(roads.capture(track, world))
build = action.BuildTrack
assert(build and build.track == "high_speed.lua" and build.catenary == true)
poly = build.polyline
assert(#poly.vertices == 4 and #poly.links == 3 and #poly.removals == 0)
assert(poly.vertices[1].resolve.Node == "Track")
assert(poly.vertices[2].resolve.Split.network == "Street", "a level crossing splits the street")
assert(poly.vertices[2].pos.y == 1 and poly.vertices[3].pos.y == -1, "halves round away from zero")
assert(poly.vertices[4].resolve.Node == "Street", "a crossing at a street node")
local trackBytes = wire.encode(action)

-- ---------- captures that must not travel ----------

local function refused(capture, why)
	local got, reason = roads.capture(capture, world)
	assert(got == nil and type(reason) == "string", why)
end

-- only split halves: nothing to build
refused({ network = "Street", street = "s", nodes = { { id = -3, pos = { 300, 50, 5 } } },
	edges = { road.edges[3], road.edges[4] } }, "no edges")
-- a removal of an edge whose ends are nowhere
refused({ network = "Street", street = "s", nodes = {}, edges = { { node0 = 7, node1 = 10,
	network = "Street", tangent0 = { 1, 0, 0 }, tangent1 = { 1, 0, 0 } } },
	removed = { { node0 = 7, node1 = 999 } } }, "an unplaceable removal")
-- a street edge in a track build that is no split half
refused({ network = "Track", track = "t", nodes = {}, edges = { { node0 = 7, node1 = 10,
	network = "Street", tangent0 = { 1, 0, 0 }, tangent1 = { 1, 0, 0 } } } }, "a foreign edge")
-- a position that is not a number
refused({ network = "Street", street = "s", nodes = { { id = -1, pos = { 0 / 0, 0, 0 } } },
	edges = { { node0 = 7, node1 = -1, network = "Street", tangent0 = { 1, 0, 0 },
	tangent1 = { 1, 0, 0 } } } }, "NaN")
-- a node with no network
refused({ network = "Street", street = "s", nodes = {}, edges = { { node0 = 7, node1 = 12345,
	network = "Street", tangent0 = { 1, 0, 0 }, tangent1 = { 1, 0, 0 } } } }, "a node nowhere")

-- the encoder refuses what the Rust decoder would
local function unencodable(mutate, why)
	local a = assert(roads.capture(road, world))
	mutate(a.BuildRoad)
	local ok = wire.tryEncode(a)
	assert(not ok, why)
end
unencodable(function(b) b.street = "street\nname" end, "a control character")
unencodable(function(b) b.street = string.rep("s", 129) end, "a long name")
unencodable(function(b) b.polyline.links[1].to = 4 end, "a link past the vertices")
unencodable(function(b) b.polyline.links[1].to = 0 end, "a link to itself")
unencodable(function(b) b.polyline.vertices[1].pos.x = 2147483648 end, "past i32")
unencodable(function(b) b.tram = "Diesel" end, "an unknown variant")
unencodable(function(b) b.polyline.links = {} end, "no links")
assert(not wire.tryEncode({ SellVehicle = { vehicles = {} } }), "no Lua encoder yet")

return roadBytes, trackBytes

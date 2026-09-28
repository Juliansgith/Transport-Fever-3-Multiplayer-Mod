-- tpf3mp/engine.lua -- the game's script API, behind the plain interfaces
-- tpf3mp/roads.lua takes.
--
-- EVERY api.* NAME IN THIS FILE IS TRANSPORT FEVER 2'S, taken from TpF2
-- Multiplayer (MIT, tpf2-multiplayer by silver2127,
-- github.com/silver2127/tpf2-multiplayer, 0.6.1.12: mp/geom.lua,
-- mp/roads.lua) and from TPF2's builder GUI events. Each is marked
-- "TPF2 name" and must be confirmed against Transport Fever 3's script API
-- on release day (docs/DAY_ONE.md, "Script API recon"); nothing here has run
-- in TPF3. Mods for TF3 build 40391 use api.engine.getComponent and
-- api.type.ComponentType as TPF2 does; the street system, BASE_NODE,
-- BASE_EDGE, api.res and the builder's proposal event are not seen there
-- (investigation/TF3_MODS_2026-09-27.md). Every call is guarded: a name that is gone makes the capture
-- fail, and a failed capture leaves the build to run natively.

local geom = require "tpf3mp.geom"
local roads = require "tpf3mp.roads"
local wire = require "tpf3mp.wire"

local engine = {}

local function vec3(v)
	if not v then return nil end
	return { v.x or v[1], v.y or v[2], v.z or v[3] }
end

-- TPF2 name: api.engine.system.streetSystem.getNode2StreetEdgeMap /
-- getNode2TrackEdgeMap, node id -> the ids of its edges. Hold the map in a
-- local for as long as it is iterated: on TPF2 a pairs() straight off the
-- call let the GC free the C++ map mid-loop, a native crash no pcall
-- catches.
local function netMap(network)
	local m
	pcall(function()
		local streets = api.engine.system.streetSystem
		if network == "Track" then m = streets.getNode2TrackEdgeMap() else m = streets.getNode2StreetEdgeMap() end
	end)
	return m
end

-- TPF2 names: api.engine.getComponent, api.type.ComponentType.BASE_NODE
-- (.position) and BASE_EDGE (.node0, .node1, .tangent0, .tangent1).
local function nodePos(id)
	local p
	pcall(function()
		local c = api.engine.getComponent(id, api.type.ComponentType.BASE_NODE)
		if c and c.position then p = vec3(c.position) end
	end)
	return p
end

local function edgeGeom(id)
	local e
	pcall(function()
		local c = api.engine.getComponent(id, api.type.ComponentType.BASE_EDGE)
		if not c then return end
		local a, b = nodePos(c.node0), nodePos(c.node1)
		if a and b then
			e = { id = id, node0 = c.node0, node1 = c.node1, a = a, b = b,
			      ta = vec3(c.tangent0), tb = vec3(c.tangent1) }
		end
	end)
	return e
end

-- The world as tpf3mp/roads.lua asks it. One instance per capture: each
-- network's edges are read once and answer every lookup, since nothing
-- changes the world inside one capture (TPF2 measured 0.4 to 2.6 s per
-- track build when every lookup walked the whole map again).
function engine.world()
	local edgeLists = {}
	local function edges(network)
		if edgeLists[network] then return edgeLists[network] end
		local list, seen = {}, {}
		local m = netMap(network)   -- held for the loop: see netMap
		for _, ids in pairs(m or {}) do
			for _, id in pairs(ids) do
				if not seen[id] then
					seen[id] = true
					list[#list + 1] = edgeGeom(id)
				end
			end
		end
		edgeLists[network] = list
		return list
	end
	local world = {}
	world.nodePos = nodePos
	function world.nodeNetwork(id)
		for _, network in ipairs({ "Street", "Track" }) do
			local m = netMap(network)
			if m and m[id] ~= nil then return network end
		end
		return nil
	end
	function world.edgeUnder(network, x, y)
		local tol = network == "Track" and geom.SPLIT_EPS_TRACK or geom.SPLIT_EPS
		local e = geom.edgeContaining(edges(network), x, y, tol)
		if e then return e.node0, e.node1 end
		return nil
	end
	return world
end

-- TPF2 names: api.res.streetTypeRep, trackTypeRep, bridgeTypeRep and
-- tunnelTypeRep, each with getName(index) giving the resource's file name.
local function resName(rep, index)
	local name
	pcall(function() name = api.res[rep].getName(index) end)
	return name
end

-- TPF2 names: a builder proposal's segment has .comp (BaseEdge: node0,
-- node1, tangent0, tangent1, type 0 ground / 1 bridge / 2 tunnel,
-- typeIndex), .type (0 street, 1 track), .streetEdge (streetType, hasBus,
-- tramTrackType 0 none / 1 plain / 2 electric) and .trackEdge (trackType,
-- catenary).
local function segment(seg)
	local c = seg.comp
	local e = {
		node0 = c.node0, node1 = c.node1,
		network = seg.type == 1 and "Track" or "Street",
		tangent0 = vec3(c.tangent0), tangent1 = vec3(c.tangent1),
		structure = "Ground",
	}
	if c.type == 1 then
		e.structure = { Bridge = resName("bridgeTypeRep", c.typeIndex) }
	elseif c.type == 2 then
		e.structure = { Tunnel = resName("tunnelTypeRep", c.typeIndex) }
	end
	return e
end

local TRAM = { [0] = "None", [1] = "Plain", [2] = "Electric" }

-- A captured proposal as tpf3mp/roads.lua takes it, from TPF2's
-- `builder.apply` GUI event of the street or track builder:
-- param.proposal.proposal with addedNodes (.entity, .comp.position),
-- addedSegments and removedSegments (TPF2 names). `network` is the tool's.
function engine.fromProposal(proposal, network)
	local capture = { network = network, nodes = {}, edges = {}, removed = {} }
	for _, n in pairs(proposal.addedNodes) do
		capture.nodes[#capture.nodes + 1] = { id = n.entity, pos = vec3(n.comp.position) }
	end
	local first
	for _, seg in pairs(proposal.addedSegments) do
		local e = segment(seg)
		capture.edges[#capture.edges + 1] = e
		if not first and e.network == network then first = seg end
	end
	for _, seg in pairs(proposal.removedSegments) do
		capture.removed[#capture.removed + 1] = { node0 = seg.comp.node0, node1 = seg.comp.node1 }
	end
	if first and network == "Street" then
		capture.street = resName("streetTypeRep", first.streetEdge.streetType)
		capture.bus_lane = first.streetEdge.hasBus == true
		capture.tram = TRAM[first.streetEdge.tramTrackType or 0]
	elseif first then
		capture.track = resName("trackTypeRep", first.trackEdge.trackType)
		capture.catenary = first.trackEdge.catenary == true
	end
	return capture
end

-- The payload bytes of a street or track builder's proposal, or nil and
-- why not. The caller hands the bytes to the hook, which sends them as an
-- intent; on nil it must let the build run natively.
function engine.captureBuild(proposal, network)
	local ok, capture = pcall(engine.fromProposal, proposal, network)
	if not ok then return nil, "unreadable proposal: " .. tostring(capture) end
	local action, reason = roads.capture(capture, engine.world())
	if not action then return nil, reason end
	local encoded, bytes = wire.tryEncode(action)
	if not encoded then return nil, tostring(bytes) end
	return bytes
end

return engine

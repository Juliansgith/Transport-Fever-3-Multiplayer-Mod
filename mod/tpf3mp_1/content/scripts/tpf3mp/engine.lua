-- tpf3mp/engine.lua -- Transport Fever 3's street and track tools and its
-- network, behind the plain interfaces tpf3mp/roads.lua takes.
--
-- What it reads, as build 40408 has it (the game's api/tealdef and the build
-- probe, tools/probe/tf3/tpf3mp_buildprobe_1): the street and track tools
-- hand game scripts a proposal (builder.proposalCreate's first parameter)
-- whose .proposal is a StreetProposal:
--
-- - addedNodes: the new nodes, entity < 0, comp.position;
-- - addedSegments: the new edges, entity < 0, type 0 street / 1 track, comp a
--   BaseEdge (node0, node1, tangent0, tangent1, type NORMAL / BRIDGE /
--   TUNNEL, typeIndex, roadTemplate and roadStyle, resource names, and the
--   stops and signals on it, objects);
-- - removedSegments and removedNodes: the existing edges and nodes it
--   removes.
--
-- A street drawn onto another's middle (seen): the old street's node nearest
-- the new junction is removed with its two edges, and the old street is
-- rebuilt from its neighbours through the junction, in its own template.
--
-- The lists are the game's vectors: read by index, never with pairs().

local function module(name)
	local loaded = package and package.loaded and package.loaded["tpf3mp." .. name]
	if loaded then return loaded end
	if ug_require then return ug_require("tpf3mp_1::/scripts/tpf3mp/" .. name .. ".lua") end
	return require("tpf3mp." .. name)
end

local roads = module("roads")

local engine = {}

local function get(value, key)
	local ok, v = pcall(function() return value[key] end)
	if ok then return v end
	return nil
end

-- The game's vector (or a table) as a Lua array.
local function list(v)
	if v == nil then return {} end
	local ok, n = pcall(function() return #v end)
	if not ok or type(n) ~= "number" then error("a list it cannot read", 0) end
	local out = {}
	for i = 1, n do out[i] = v[i] end
	return out
end

local function vec3(v)
	if v == nil then return nil end
	local x, y, z = get(v, "x"), get(v, "y"), get(v, "z")
	if x == nil then x, y, z = get(v, 1), get(v, 2), get(v, 3) end
	return { x, y, z }
end

-- The game's enums, under api.type.enum ("enum" is a word in Teal, which
-- writes api.type["enum"]).
local function enum(name)
	local enums = api.type.enum
	local e = enums and enums[name]
	if e == nil then error("no api.type.enum." .. name, 0) end
	return e
end

local function nodePos(id)
	local p
	pcall(function()
		local c = api.engine.getComponent(id, api.type.ComponentType.BASE_NODE)
		if c and c.position then p = vec3(c.position) end
	end)
	return p
end

-- A node's edges in one network, from the street system.
local function nodeEdges(id, network)
	local edges
	pcall(function()
		local streets = api.engine.system.streetSystem
		if network == "Track" then edges = streets.getNodeTrackSegments(id) else edges = streets.getNodeStreetSegments(id) end
	end)
	return edges
end

-- The world as tpf3mp/roads.lua asks it. Only the nodes a proposal names
-- are read: no edge of the map is walked.
function engine.world()
	local world = {}
	world.nodePos = nodePos
	function world.nodeNetwork(id)
		for _, network in ipairs({ "Street", "Track" }) do
			local edges = nodeEdges(id, network)
			if edges ~= nil and #list(edges) > 0 then return network end
		end
		return nil
	end
	return world
end

local function networkOf(seg)
	local kind = get(seg, "type")
	if kind == 0 then return "Street" end
	if kind == 1 then return "Track" end
	error("an edge of type " .. tostring(kind), 0)
end

local function resName(v, what)
	if type(v) ~= "string" or v == "" then error(what .. " is not a resource name: " .. tostring(v), 0) end
	return v
end

-- A bridge's or tunnel's type, by its name.
local function typeName(rep, index)
	local name
	pcall(function() name = api.res[rep].getName(index) end)
	if type(name) ~= "string" or name == "" then error("no " .. rep .. " type " .. tostring(index), 0) end
	return name
end

-- A road style: "" is none.
local function styleName(v)
	if v == "" or v == nil then return nil end
	return resName(v, "roadStyle")
end

-- Refuses an edge that carries stops or signals: the room does not carry
-- them yet, and an edge replaced without them leaves them pointing nowhere
-- (on TPF2 that crashed every game at the same step; docs/BUILDING.md).
local function noObjects(c, what)
	local objects = get(c, "objects")
	if objects ~= nil and #list(objects) > 0 then error(what .. " with a stop or signal on it", 0) end
end

local function segment(seg)
	local c = get(seg, "comp")
	if c == nil then error("an edge with no component", 0) end
	noObjects(c, "a build that moves an edge")
	local e = {
		node0 = c.node0, node1 = c.node1,
		network = networkOf(seg),
		tangent0 = vec3(c.tangent0), tangent1 = vec3(c.tangent1),
		structure = "Ground",
		template = resName(c.roadTemplate, "roadTemplate"),
		style = styleName(c.roadStyle),
	}
	local types = enum("BaseEdgeType")
	if c.type == types.BRIDGE then
		e.structure = { Bridge = typeName("bridgeTypeRep", c.typeIndex) }
	elseif c.type == types.TUNNEL then
		e.structure = { Tunnel = typeName("tunnelTypeRep", c.typeIndex) }
	elseif c.type ~= types.NORMAL then
		error("an edge of structure " .. tostring(c.type), 0)
	end
	return e
end

-- A tool's proposal as tpf3mp/roads.lua takes it, or raises. `network` is
-- the tool's. Returns nil for a proposal of nothing (the tool before its
-- first point).
function engine.fromProposal(proposal, network)
	local street = get(proposal, "proposal")
	if street == nil then error("a proposal with no street proposal", 0) end
	for _, name in ipairs({ "toAdd", "toRemove" }) do
		if #list(get(proposal, name)) > 0 then error("a build with constructions", 0) end
	end
	for _, name in ipairs({ "edgeObjectsToAdd", "edgeObjectsToRemove" }) do
		local v = get(street, name)
		if v ~= nil and #list(v) > 0 then error("a build with a stop or signal", 0) end
	end
	local added, segments, removed = list(get(street, "addedNodes")), list(get(street, "addedSegments")),
		list(get(street, "removedSegments"))
	local removedNodes = list(get(street, "removedNodes"))
	if #added == 0 and #segments == 0 and #removed == 0 and #removedNodes == 0 then return nil end

	local capture = { network = network, nodes = {}, edges = {}, removed = {}, removedNodes = {} }
	for _, n in ipairs(added) do
		capture.nodes[#capture.nodes + 1] = { id = n.entity, pos = vec3(n.comp.position) }
	end
	local first
	for _, seg in ipairs(segments) do
		local e = segment(seg)
		capture.edges[#capture.edges + 1] = e
		if not first and e.network == network then first = e end
	end
	for _, seg in ipairs(removed) do
		noObjects(seg.comp, "a build that removes an edge")
		capture.removed[#capture.removed + 1] = { node0 = seg.comp.node0, node1 = seg.comp.node1,
			network = networkOf(seg) }
	end
	for _, n in ipairs(removedNodes) do
		capture.removedNodes[#capture.removedNodes + 1] = { id = n.entity, pos = vec3(get(n.comp, "position")) }
	end

	-- The build's own kind: its first edge of the tool's network. The
	-- template names the edge whole on TF3: its lanes, bus lanes and tram
	-- tracks. The schema's bus lane and tram are TPF2's, none here.
	if first then
		if network == "Street" then
			capture.street, capture.bus_lane, capture.tram = first.template, false, "None"
		else
			capture.track, capture.catenary = first.template, false
		end
		capture.style = first.style
	end
	return capture
end

-- A tool's proposal in one line, for the log when the room cannot carry it:
-- nodes added (+n) and removed (-n), edges added (+e) and removed (-e) with
-- their ends, existing nodes with their positions.
function engine.describe(proposal)
	local ok, text = pcall(function()
		local street = get(proposal, "proposal")
		local out = {}
		local function at(p)
			p = vec3(p)
			if p == nil or type(p[1]) ~= "number" then return "(?)" end
			return string.format("(%.1f,%.1f,%.1f)", p[1], p[2], p[3])
		end
		local function node(id)
			if type(id) == "number" and id >= 0 then return tostring(id) .. at(nodePos(id)) end
			return tostring(id)
		end
		for _, n in ipairs(list(get(street, "addedNodes"))) do
			out[#out + 1] = "+n" .. tostring(n.entity) .. at(n.comp.position)
		end
		for _, n in ipairs(list(get(street, "removedNodes"))) do
			out[#out + 1] = "-n" .. tostring(n.entity) .. at(n.comp and n.comp.position)
		end
		for _, s in ipairs(list(get(street, "addedSegments"))) do
			out[#out + 1] = "+e" .. tostring(s.entity) .. "/" .. tostring(s.type) .. ":"
				.. node(s.comp.node0) .. ">" .. node(s.comp.node1)
		end
		for _, s in ipairs(list(get(street, "removedSegments"))) do
			out[#out + 1] = "-e" .. tostring(s.entity) .. ":" .. node(s.comp.node0) .. ">" .. node(s.comp.node1)
		end
		return table.concat(out, " ")
	end)
	if ok then return text end
	return "unreadable: " .. tostring(text)
end

-- The action table of a street or track tool's proposal; false for a
-- proposal of nothing; or nil and why the room cannot carry it.
function engine.captureBuild(proposal, network)
	local ok, capture = pcall(engine.fromProposal, proposal, network)
	if not ok then return nil, tostring(capture) end
	if capture == nil then return false end
	return roads.capture(capture, engine.world())
end


return engine

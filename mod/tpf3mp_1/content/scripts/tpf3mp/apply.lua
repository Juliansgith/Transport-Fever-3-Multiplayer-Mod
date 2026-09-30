-- tpf3mp/apply.lua -- runs an action the room ordered, in the mod's game
-- script's postUpdate (docs/HOOKS.md, "Actions in the game").
--
-- A game script runs in an engine state, where a command runs at once (the
-- game's own api/tealdef/api/cmd.d.tl). A command is sent without a
-- callback, which the game refused in update ("Callbacks are currently
-- disallowed", build 40408); one the game refuses raises.
-- Every game applies the room's action in the same simulation update, so
-- what this makes of an action may depend on nothing but the action and the
-- world, which every game has alike: no time of day, no camera, no GUI.
--
-- An action is the table the hook hands over (tpf3mp_proto::lua): one
-- entry, the action's name and its body, in the game's units (metres,
-- plain fractions).
--
-- Pure Lua against the game's `api`; the tests give it a fake one.

local apply = {}

-- A matrix from a Transform: its basis is elements 1-3, 5-7 and 9-11 of the
-- game's matrix and its origin elements 13-15 (columns of four).
local function matrix(transform)
	local b, o = transform.basis, transform.origin
	local column = api.type.Vec4f.new
	return api.type.Mat4f.new(
		column(b[1], b[2], b[3], 0),
		column(b[4], b[5], b[6], 0),
		column(b[7], b[8], b[9], 0),
		column(o.x, o.y, o.z, 1)
	)
end

-- A parameter's value: exactly one of Int, Fixed, Bool and Text.
local function paramValue(value)
	if value.Int ~= nil then return value.Int end
	if value.Fixed ~= nil then return value.Fixed end
	if value.Bool ~= nil then return value.Bool end
	if value.Text ~= nil then return value.Text end
	error("a parameter value of no kind")
end

-- The keys a flattened parameter path names: "modules[3801].name" is
-- modules, 3801, name.
local function pathKeys(path)
	local keys = {}
	for part in string.gmatch(path, "[^%.]+") do
		local name, rest = string.match(part, "^([^%[]*)(.*)$")
		if name ~= "" then keys[#keys + 1] = name end
		for index in string.gmatch(rest, "%[(%-?%d+)%]") do
			keys[#keys + 1] = tonumber(index)
		end
	end
	if #keys == 0 then error("an empty parameter path") end
	return keys
end

-- The construction's parameters, from their flattened paths.
local function params(list)
	local out = {}
	for _, param in ipairs(list) do
		local keys = pathKeys(param.key)
		local node = out
		for i = 1, #keys - 1 do
			local key = keys[i]
			if type(node[key]) ~= "table" then node[key] = {} end
			node = node[key]
		end
		node[keys[#keys]] = paramValue(param.value)
	end
	return out
end

-- A line for the hook's log; the game script sets apply.log once linked.
local function log(line)
	if apply.log then pcall(apply.log, line) end
end

-- Sends `command`, which runs at once; a refusal raises, and apply.run
-- reports it.
local function run(command)
	api.cmd.sendCommand(command)
	return true
end

-- Builds `proposal` as the player's own build. The game's verdict first, as
-- its tools ask it: a build it would refuse (a collision, too steep, not
-- enough money) fails here with its reasons, the same in every game, and is
-- never sent; sent without a callback, a refused build would fail unseen.
local function buildProposal(proposal, context)
	local proposals = api.engine.util.proposal
	if proposals and proposals.makeProposalData then
		local data = proposals.makeProposalData(proposal, context)
		local state = data and data.errorState
		if state and state.critical then
			local messages = {}
			for _, m in ipairs(state.messages or {}) do messages[#messages + 1] = tostring(m) end
			error("the game refuses the build: " .. table.concat(messages, "; "), 0)
		end
	end
	return run(api.cmd.makeWorldBuildProposalCmd(proposal, context, false, true))
end

local HANDLERS = {}

function HANDLERS.BuildConstruction(build)
	if build.replaces ~= nil then
		return false, "this version of the mod does not replace constructions yet"
	end
	local proposal = api.type.SimpleProposal.new()
	local entity = api.type.SimpleProposal.ConstructionEntity.new()
	entity.fileName = build.file
	entity.transf = matrix(build.transform)
	entity.params = params(build.params)
	entity.name = build.name
	entity.playerEntity = api.engine.util.getPlayer()
	proposal.constructionsToAdd = { entity }
	-- Paid by the player, and clearing town buildings in its way, as the
	-- construction tool builds (the game's bridge and tunnel window names the
	-- player so, gui/entity_window/bridge_and_tunnel.tl); without a context
	-- the game builds for free. ignoreErrors false and playerInitiated true:
	-- as the player's own build.
	local context = api.type.Context.new()
	context.player = api.engine.util.getPlayer()
	context.gatherBuildings = true
	context.gatherFields = true
	return buildProposal(proposal, context)
end

-- ---------------------------------------------------------------- roads
--
-- A road or track build (tpf3mp_proto action::Polyline) as a SimpleProposal's
-- street proposal, as the game's own scripted track builder makes one
-- (mission/tasks/auto_builder/track_builder.tl): new nodes and edges with
-- negative ids, existing nodes by their own. A vertex resolves as the
-- originator's tool resolved it: New; the existing node of its network
-- within 1.5 m horizontally, the nearest; or a split of the existing edge
-- between the nodes at its ends, cut in two at the vertex into halves that
-- keep the edge's own component, their tangents scaled to the part of the
-- curve each covers. The edges and nodes a build removes are found the same
-- way: an edge by the nodes at its ends, a node by its position. Each link
-- is the build's street or track, or the kind it names: a piece of the
-- street it joins, rebuilt through the new junction, keeps that street's.
--
-- Every game has the same world, so every game resolves alike; anything that
-- resolves to nothing fails the whole build, in every game.

local function module(name)
	local loaded = package and package.loaded and package.loaded["tpf3mp." .. name]
	if loaded then return loaded end
	if ug_require then return ug_require("tpf3mp_1::/scripts/tpf3mp/" .. name .. ".lua") end
	return require("tpf3mp." .. name)
end

local geom = module("geom")

-- How near an existing node a vertex resolving to it is, horizontally.
local NODE_TOLERANCE = 1.5
-- How near its node each end of a named edge is: the ends are the node's own
-- positions, rounded to the millimetre.
local END_TOLERANCE = 0.5

local function arr(v) return { v.x or v[1], v.y or v[2], v.z or v[3] } end
local function vec(p) return api.type.Vec3f.new(p[1], p[2], p[3]) end
local function scaled(p, k) return { p[1] * k, p[2] * k, p[3] * k } end

-- The game's enums, under api.type.enum ("enum" is a word in Teal, which
-- writes api.type["enum"]).
local function enum(name)
	local e = api.type.enum and api.type.enum[name]
	if e == nil then error("no api.type.enum." .. name) end
	return e
end

-- A resource's id by its name; the game answers -1 for none.
local function find(rep, name)
	local id = api.res[rep].find(name)
	if type(id) ~= "number" or id < 0 then error("no " .. rep .. " resource " .. tostring(name)) end
	return id
end

-- The nodes of a network: each node with an edge of it, and its position.
-- Held while it is read (on TPF2 a pairs() straight off the call let the
-- GC free the map mid-loop).
local function readNodes(network)
	local streets = api.engine.system.streetSystem
	local map
	if network == "Track" then map = streets.getNode2TrackEdgeMap() else map = streets.getNode2StreetEdgeMap() end
	local nodes = {}
	for node in pairs(map) do
		local c = api.engine.getComponent(node, api.type.ComponentType.BASE_NODE)
		if c and c.position then nodes[#nodes + 1] = { id = node, pos = arr(c.position) } end
	end
	return nodes
end

-- The node of `nodes` nearest `p` horizontally within `tol`, the lower id
-- on a tie; nil when none is.
local function nearest(nodes, p, tol)
	local best, bestD
	for _, n in ipairs(nodes) do
		local dx, dy = n.pos[1] - p[1], n.pos[2] - p[2]
		local d = dx * dx + dy * dy
		if d <= tol * tol and (bestD == nil or d < bestD or (d == bestD and n.id < best.id)) then
			best, bestD = n, d
		end
	end
	return best
end

-- The existing edge of `network` between the nodes at `a` and `b`: its id,
-- component and geometry (ends a and b, tangents ta and tb, as geom.lua takes
-- edges), oriented as the game has it. The lowest id if there are several.
local function edgeBetween(nodes, network, a, b)
	local na, nb = nearest(nodes, a, END_TOLERANCE), nearest(nodes, b, END_TOLERANCE)
	if na == nil or nb == nil or na.id == nb.id then return nil end
	local streets = api.engine.system.streetSystem
	local ids
	if network == "Track" then ids = streets.getNodeTrackSegments(na.id) else ids = streets.getNodeStreetSegments(na.id) end
	local found
	for i = 1, (ids and #ids or 0) do
		local id = ids[i]
		local c = api.engine.getComponent(id, api.type.ComponentType.BASE_EDGE)
		if c and ((c.node0 == na.id and c.node1 == nb.id) or (c.node0 == nb.id and c.node1 == na.id))
			and (found == nil or id < found.id) then
			local n0, n1 = na, nb
			if c.node0 == nb.id then n0, n1 = nb, na end
			found = { id = id, comp = c, node0 = n0.id, node1 = n1.id, a = n0.pos, b = n1.pos,
				ta = arr(c.tangent0), tb = arr(c.tangent1) }
		end
	end
	return found
end

local STRUCTURE = { Ground = "NORMAL", Bridge = "BRIDGE", Tunnel = "TUNNEL" }

local function buildNetwork(network, templateName, style, polyline)
	local nodesOf = {}
	local function nodes(n)
		if nodesOf[n] == nil then nodesOf[n] = readNodes(n) end
		return nodesOf[n]
	end
	local templates = {}
	local function template(name)
		if templates[name] == nil then
			templates[name] = api.res.streetTemplateRep.get(find("streetTemplateRep", name))
		end
		return templates[name]
	end
	local edgeType = enum("BaseEdgeType")

	-- Ids: the edges from -1, the links first and then two halves per split;
	-- the new nodes after them.
	local splits = 0
	for _, v in ipairs(polyline.vertices) do
		if type(v.resolve) == "table" and v.resolve.Split then splits = splits + 1 end
	end
	local nextEdge, nextNode = -1, -(#polyline.links + 2 * splits) - 1

	local nodesToAdd, edgesToAdd, edgesToRemove = {}, {}, {}
	-- The nodes at the ends of the edges removed, in order: their lane
	-- configurations name those edges, and go with them (below).
	local ends, endSeen = {}, {}
	local function removeEdge(e)
		edgesToRemove[#edgesToRemove + 1] = e.id
		for _, node in ipairs({ e.comp.node0, e.comp.node1 }) do
			if not endSeen[node] then
				endSeen[node] = true
				ends[#ends + 1] = node
			end
		end
	end
	local function addNode(p)
		local n = api.type.NodeAndEntity.new()
		n.entity = nextNode
		nextNode = nextNode - 1
		n.comp.position = vec(p)
		nodesToAdd[#nodesToAdd + 1] = n
		return n.entity
	end
	local function addEdge(kind, node0, node1, p0, p1, t0, t1, comp)
		local s = api.type.SegmentAndEntity.new()
		s.entity = nextEdge
		nextEdge = nextEdge - 1
		if comp ~= nil then s.comp = comp end
		s.type = kind
		s.comp.node0, s.comp.node1 = node0, node1
		s.comp.position0, s.comp.position1 = vec(p0), vec(p1)
		s.comp.tangent0, s.comp.tangent1 = vec(t0), vec(t1)
		edgesToAdd[#edgesToAdd + 1] = s
		return s
	end
	local function kindOf(n) if n == "Track" then return 1 end return 0 end

	-- The links' edges first, as their ids were counted.
	local links = {}
	local ids, at = {}, {}
	for i, v in ipairs(polyline.vertices) do at[i] = arr(v.pos) end
	for k, link in ipairs(polyline.links) do
		local own = link.kind and link.kind.network or network
		links[k] = addEdge(kindOf(own), 0, 0, at[link.from + 1], at[link.to + 1],
			arr(link.tangent0), arr(link.tangent1))
	end

	for i, v in ipairs(polyline.vertices) do
		local p, r = at[i], v.resolve
		if r == "New" then
			ids[i] = addNode(p)
		elseif type(r) == "table" and r.Node then
			local n = nearest(nodes(r.Node), p, NODE_TOLERANCE)
			if n == nil then error("no " .. r.Node .. " node at vertex " .. i) end
			ids[i] = n.id
		elseif type(r) == "table" and r.Split then
			local s = r.Split
			local e = edgeBetween(nodes(s.network), s.network, arr(s.ends.a), arr(s.ends.b))
			if e == nil then error("no " .. s.network .. " edge to split at vertex " .. i) end
			if #(e.comp.objects or {}) > 0 then
				error("vertex " .. i .. " splits an edge with a stop or signal on it")
			end
			local tol = s.network == "Track" and geom.SPLIT_EPS_TRACK or geom.SPLIT_EPS
			local u, off = geom.parameterAt(e.a, e.ta, e.b, e.tb, p[1], p[2])
			local function from(q) local dx, dy = p[1] - q[1], p[2] - q[2] return math.sqrt(dx * dx + dy * dy) end
			if off > tol then error("vertex " .. i .. " is not on the edge it splits") end
			if from(e.a) < geom.SPLIT_MIN_DIST or from(e.b) < geom.SPLIT_MIN_DIST then
				error("vertex " .. i .. " splits the edge at its end")
			end
			local tm = geom.hermiteTangent(e.a, e.ta, e.b, e.tb, u)
			local mid = addNode(p)
			ids[i] = mid
			removeEdge(e)
			-- Each half the split edge's own component, read afresh, as the
			-- game's electrify task rebuilds an edge (electrify.tl).
			local component = api.type.ComponentType.BASE_EDGE
			addEdge(kindOf(s.network), e.node0, mid, e.a, p, scaled(e.ta, u), scaled(tm, u),
				api.engine.getComponent(e.id, component))
			addEdge(kindOf(s.network), mid, e.node1, p, e.b, scaled(tm, 1 - u), scaled(e.tb, 1 - u),
				api.engine.getComponent(e.id, component))
		else
			error("vertex " .. i .. " resolves as nothing this mod knows")
		end
	end

	for k, link in ipairs(polyline.links) do
		local s = links[k]
		s.comp.node0, s.comp.node1 = ids[link.from + 1], ids[link.to + 1]
		local structure, name = link.structure, "Ground"
		if type(structure) == "table" then name = next(structure) end
		s.comp.type = edgeType[STRUCTURE[name] or error("a link of structure " .. tostring(name))]
		if name == "Bridge" then
			s.comp.typeIndex = find("bridgeTypeRep", structure.Bridge)
		elseif name == "Tunnel" then
			s.comp.typeIndex = find("tunnelTypeRep", structure.Tunnel)
		else
			s.comp.typeIndex = -1
		end
		-- The build's own kind, or the kind the link names.
		local kind = link.kind or { network = network, template = templateName, style = style }
		local t = template(kind.template)
		s.comp.laneConfigs = t.laneConfigs
		s.comp.roadTemplate = kind.template
		s.comp.roadStyle = kind.style or t.streetStyle
		s.comp.roadType = kind.network == "Track" and enum("RoadType").TRACK or enum("RoadType").STREET
	end

	for k, r in ipairs(polyline.removals or {}) do
		local e = edgeBetween(nodes(r.network), r.network, arr(r.ends.a), arr(r.ends.b))
		if e == nil then error("no " .. r.network .. " edge to remove (" .. k .. ")") end
		-- An edge removed with its stops or signals leaves them pointing
		-- nowhere: on TPF2 that crashed every game at the same step
		-- (docs/BUILDING.md). The room does not carry them yet.
		if #(e.comp.objects or {}) > 0 then error("removal " .. k .. " has a stop or signal on it") end
		removeEdge(e)
	end

	local nodesToRemove, removedNode = {}, {}
	for k, n in ipairs(polyline.removed_nodes or {}) do
		local found = nearest(nodes(n.network), arr(n.at), END_TOLERANCE)
		if found == nil then error("no " .. n.network .. " node to remove (" .. k .. ")") end
		nodesToRemove[#nodesToRemove + 1] = found.id
		removedNode[found.id] = true
	end

	-- A node's lane configuration (BASE_NODE_CONFIG) names the edges at it,
	-- and the game cannot read a proposal that removes an edge a
	-- configuration still names (build 40408: "Unknown exception" from
	-- makeProposalData). So the configurations at the ends of the removed
	-- edges go too, and the game makes new ones; a node removed takes its
	-- own with it, and may not be named for both.
	local configsToRemove = {}
	for _, node in ipairs(ends) do
		if not removedNode[node]
			and api.engine.getComponent(node, api.type.ComponentType.BASE_NODE_CONFIG) ~= nil then
			configsToRemove[#configsToRemove + 1] = node
		end
	end

	local proposal = api.type.SimpleProposal.new()
	proposal.streetProposal.nodesToAdd = nodesToAdd
	proposal.streetProposal.edgesToAdd = edgesToAdd
	proposal.streetProposal.edgesToRemove = edgesToRemove
	if #nodesToRemove > 0 then proposal.streetProposal.nodesToRemove = nodesToRemove end
	if #configsToRemove > 0 then proposal.streetProposal.nodeConfigsToRemove = configsToRemove end
	-- Paid by the player, as the tool builds.
	local context = api.type.Context.new()
	context.player = api.engine.util.getPlayer()

	-- What is sent, in the log before it goes: an exception from the game
	-- does not always come back through pcall.
	local shape = {}
	for _, n in ipairs(nodesToAdd) do
		local p = n.comp.position
		shape[#shape + 1] = string.format("+n%d(%.1f,%.1f,%.1f)", n.entity, p.x, p.y, p.z)
	end
	for _, s in ipairs(edgesToAdd) do
		shape[#shape + 1] = "+e" .. s.entity .. "/" .. tostring(s.type) .. ":" .. tostring(s.comp.node0) .. ">"
			.. tostring(s.comp.node1) .. " " .. tostring(s.comp.roadTemplate)
	end
	shape[#shape + 1] = "-e" .. table.concat(edgesToRemove, ",") .. " -n" .. table.concat(nodesToRemove, ",")
		.. " -c" .. table.concat(configsToRemove, ",")
	log("building " .. table.concat(shape, " "))

	return buildProposal(proposal, context)
end

function HANDLERS.BuildRoad(road)
	return buildNetwork("Street", road.street, road.style, road.polyline)
end

function HANDLERS.BuildTrack(track)
	return buildNetwork("Track", track.track, track.style, track.polyline)
end

-- A loan's terms as the loan script keeps them (loan.d.tl): the action's
-- table has the script's own field names and fractions.
local function loanTerms(terms)
	local out = {}
	for key, value in pairs(terms) do out[key] = value end
	return out
end

-- Loans go through the loan script's own events, with the parameters the
-- game's finance window sends (game_mechanics/finance/finances_loan_gui.tl):
-- here they run at once, in every game at the same update.
function HANDLERS.Loan(op)
	if op.Take then
		return run(api.cmd.makeScriptingSendEventCmd("", "Loan", "Obtain",
			{ loanTerms(op.Take.next), loanTerms(op.Take.offer) }))
	elseif op.Repay then
		local param = {}
		param[2] = loanTerms(op.Repay.loan)
		return run(api.cmd.makeScriptingSendEventCmd("", "Loan", "Repay", param))
	end
	return false, "a loan is taken or paid back"
end

-- Runs one action. Returns true, or false and why not; never raises.
function apply.run(action)
	if type(action) ~= "table" then return false, "an action is a table" end
	local kind, body = next(action)
	if kind == nil or next(action, kind) ~= nil then
		return false, "an action is a table of one entry"
	end
	local handler = HANDLERS[kind]
	if handler == nil then
		return false, "this version of the mod does not apply " .. tostring(kind) .. " yet"
	end
	local ok, applied, why = pcall(handler, body)
	if not ok then return false, tostring(applied) end
	return applied == true, why
end

-- The actions this version applies, for tests and the log.
function apply.kinds()
	local kinds = {}
	for kind in pairs(HANDLERS) do kinds[#kinds + 1] = kind end
	table.sort(kinds)
	return kinds
end

return apply

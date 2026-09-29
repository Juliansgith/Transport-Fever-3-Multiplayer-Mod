-- tpf3mp/roads.lua -- a captured road or track build as a BuildRoad or
-- BuildTrack action (tpf3mp_proto::action; docs/BUILDING.md, "The action
-- schema").
--
-- Ported from TpF2 Multiplayer's ROADE capture in mp/inject.lua (MIT,
-- tpf2-multiplayer by silver2127, github.com/silver2127/tpf2-multiplayer,
-- 0.6.1.12): the conversion of a captured proposal into a purely positional
-- command on the originating game, which still has every entity the capture
-- names. Kept from it: split halves are dropped (the receiver regenerates
-- them by splitting its own copy), a split point is told from a new node in
-- open space by geometry, split parents are not shipped as removals, an
-- in-place replacement's removals are, and a removal that cannot be placed
-- ships nothing. Left behind: the text line format, the stamp and pacing,
-- the file polling, the plan-only replay pass (the originator's decision is
-- now read straight off its proposal, one Resolve per vertex) and the
-- bus-lane/tram side channel.
--
-- Pure Lua: the world is asked through the `world` argument, which
-- tpf3mp/engine.lua implements on the game's API and tests implement on a
-- table.
--
--   capture = {
--     network  = "Street" | "Track",     -- the tool's network
--     street, bus_lane, tram,             -- a road: its type file, bus lane, "None" | "Plain" | "Electric"
--     track, catenary,                    -- a track: its type file, catenary
--     style,                              -- TF3: the road style, with street or track the template
--     nodes   = { { id = -1, pos = {x, y, z} }, ... },   -- the proposal's new nodes (placeholder ids < 0)
--     edges   = { { node0 =, node1 =, network =, tangent0 = {..}, tangent1 = {..},
--                   structure = "Ground" | { Bridge = file } | { Tunnel = file } }, ... },
--     removed = { { node0 =, node1 = }, ... },           -- the proposal's removed edges, by their end nodes
--   }
--   world = {
--     nodePos(id)             -> {x, y, z} of an existing node, or nil
--     nodeNetwork(id)         -> "Street" | "Track" of an existing node, or nil
--     edgeUnder(network, x, y) -> node0, node1 of the existing edge of that
--                                network passing under (x, y), or nil
--   }
--
-- Positions are metres, as the game gives them, and stay metres in the
-- action table: the hook turns them into the schema's millimetres
-- (tpf3mp_proto::lua), rounding and range checks included.

local roads = {}

local OTHER = { Street = "Track", Track = "Street" }

-- A position or tangent (an array, or x/y/z fields as the game's Vec3f reads
-- in Lua) as {x =, y =, z =} in metres, or nil and why not.
local function vec3(v)
	if type(v) ~= "table" and type(v) ~= "userdata" then
		return nil, "not a vector: " .. tostring(v)
	end
	local out = {}
	for i, axis in ipairs({ "x", "y", "z" }) do
		local c = v[axis]
		if c == nil then c = v[i] end
		if type(c) ~= "number" or c ~= c then
			return nil, axis .. " is not a number: " .. tostring(c)
		end
		out[axis] = c
	end
	return out
end

-- The action for `capture`, or nil and why not. Never raises on bad input:
-- a capture that cannot be converted completely must leave the player's
-- build to run natively (docs/BUILDING.md: never cancel on a failed decode).
function roads.capture(capture, world)
	local ok, action, reason = pcall(roads.convert, capture, world)
	if not ok then return nil, tostring(action) end
	return action, reason
end

function roads.convert(capture, world)
	local own = capture.network
	if own ~= "Street" and own ~= "Track" then
		return nil, "unknown network " .. tostring(own)
	end

	local newPos = {}
	for _, n in ipairs(capture.nodes or {}) do
		if type(n.id) ~= "number" or n.id >= 0 then
			return nil, "new node with a non-placeholder id " .. tostring(n.id)
		end
		newPos[n.id] = n.pos
	end
	local function posOf(id)
		if id < 0 then return newPos[id] end
		return world.nodePos(id)
	end

	-- Which new nodes are split points? A split point lies ON an existing
	-- edge; a bridge midpoint in open space does not, though it may also
	-- join two existing nodes. The originator's own build has not applied
	-- (it is captured before it does), so the edges it splits are still
	-- there to find. Either network: a track vertex on a road is a split of
	-- the road (a level crossing).
	local splitOf = {}   -- new node id -> { network =, node0 =, node1 = }
	for id, p in pairs(newPos) do
		for _, net in ipairs({ own, OTHER[own] }) do
			local n0, n1 = world.edgeUnder(net, p[1] or p.x, p[2] or p.y)
			if n0 then
				splitOf[id] = { network = net, node0 = n0, node1 = n1 }
				break
			end
		end
	end
	local function isParent(a, b)
		for _, s in pairs(splitOf) do
			if (s.node0 == a and s.node1 == b) or (s.node0 == b and s.node1 == a) then return true end
		end
		return false
	end

	local vertices, links, index = {}, {}, {}
	local function vertexFor(id)
		if index[id] then return index[id] end
		local p = posOf(id)
		if not p then return nil, "node " .. id .. " has no position" end
		local pos, err = vec3(p)
		if not pos then return nil, "node " .. id .. ": " .. err end
		local resolve
		if id >= 0 then
			local net = world.nodeNetwork(id)
			if not net then return nil, "node " .. id .. " is in no network" end
			resolve = { Node = net }
		elseif splitOf[id] then
			local s = splitOf[id]
			local a, errA = vec3(posOf(s.node0) or {})
			local b, errB = vec3(posOf(s.node1) or {})
			if not (a and b) then return nil, "split edge end: " .. tostring(errA or errB) end
			resolve = { Split = { network = s.network, ends = { a = a, b = b } } }
		else
			resolve = "New"
		end
		vertices[#vertices + 1] = { pos = pos, resolve = resolve }
		index[id] = #vertices - 1   -- the schema's indices start at 0
		return index[id]
	end

	local dropped = 0
	for k, e in ipairs(capture.edges or {}) do
		local a1, a2 = e.node0, e.node1
		if type(a1) ~= "number" or type(a2) ~= "number" then
			return nil, "edge " .. k .. " has no end nodes"
		end
		-- A half runs from a split node to one of the ends of the edge it
		-- splits. Any other existing-to-new edge is a connector (a track
		-- merging onto a bridge mid-span) and is kept.
		local function halfOf(existing, new)
			local s = splitOf[new]
			return s ~= nil and (existing == s.node0 or existing == s.node1)
		end
		if (a1 >= 0 and a2 < 0 and halfOf(a1, a2)) or (a2 >= 0 and a1 < 0 and halfOf(a2, a1)) then
			dropped = dropped + 1
		else
			if e.network ~= own then
				return nil, "edge " .. k .. " is " .. tostring(e.network) .. " in a " .. own .. " build"
			end
			local i1, err1 = vertexFor(a1)
			if not i1 then return nil, err1 end
			local i2, err2 = vertexFor(a2)
			if not i2 then return nil, err2 end
			if i1 ~= i2 then
				local t0, errT0 = vec3(e.tangent0)
				local t1, errT1 = vec3(e.tangent1)
				if not (t0 and t1) then return nil, "edge " .. k .. " tangent: " .. tostring(errT0 or errT1) end
				links[#links + 1] = {
					from = i1, to = i2, tangent0 = t0, tangent1 = t1,
					structure = e.structure or "Ground",
				}
			end
		end
	end
	-- Gate on edges, not on new nodes: a road joining two existing
	-- junctions adds no node and one edge.
	if #links == 0 then return nil, "no edges to build (" .. dropped .. " split halves)" end

	-- Removals the receiver cannot regenerate: an edge replaced in place (an
	-- upgrade, a span the build passes under). A split parent is not one;
	-- the split vertex names it. A removal that cannot be placed ships
	-- nothing: replayed without it, an upgrade doubles the edge.
	local removals = {}
	for k, r in ipairs(capture.removed or {}) do
		if not isParent(r.node0, r.node1) then
			local a = type(r.node0) == "number" and posOf(r.node0)
			local b = type(r.node1) == "number" and posOf(r.node1)
			local pa = a and vec3(a)
			local pb = b and vec3(b)
			if not (pa and pb) then
				return nil, "removed edge " .. k .. " has no position here"
			end
			removals[#removals + 1] = { a = pa, b = pb }
		end
	end

	local polyline = { vertices = vertices, links = links, removals = removals }
	if own == "Street" then
		return { BuildRoad = {
			street = capture.street, style = capture.style, bus_lane = capture.bus_lane == true,
			tram = capture.tram or "None", polyline = polyline,
		} }
	end
	return { BuildTrack = {
		track = capture.track, style = capture.style, catenary = capture.catenary == true, polyline = polyline,
	} }
end

return roads

-- tpf3mp/ghost.lua -- a new build's loose ends joined onto the player's
-- pending builds (docs/HOOKS.md, "Pending builds";
-- investigation/TF3_BUILD_GHOST_2026-10-02.md).
--
-- With pending builds on (TPF3MP_HOOK_BUILD_GHOST=1), the hook keeps the
-- player's road and track builds the room has not answered yet
-- (tpf3mp_native.pending()). None of them is in any game's world, so the
-- game's street and track tools cannot snap to them: a road started from the
-- end of a pending road starts on open ground, as a new node with nothing
-- attached ("New"). Before the build goes to the room, `ghost.join` names
-- each such loose end by what it lands on, by position, as the receiver
-- resolves any vertex (docs/BUILDING.md, "Resolving a vertex"):
--
-- - on a pending build's vertex (within NODE_RADIUS, horizontally): that
--   node ({ Node = network });
-- - else on a pending edge's centreline (within EDGE_RADIUS): a split of
--   that edge ({ Split = edge by its ends }), at the point of the curve
--   nearest it, or its end node when that point is within END_RADIUS of an
--   end.
--
-- The room applies the player's builds in the order they were sent, so the
-- pending build is there when the joined one resolves, in every game. If
-- the pending build failed, the joined one resolves to nothing and fails in
-- every game alike (tpf3mp/apply.lua); nothing guesses.
--
-- Only loose ends are joined (a new vertex with one link): a road crossing
-- a pending road is left as the tool made it, and the room refuses it or not
-- as it would anything else. Only within the end's own network. A pending
-- edge a later pending build splits or removes is followed: its halves are
-- the edges joined onto. What cannot be named exactly (an end between two
-- pending edges, both ends of one link on one node) is refused with why.
--
-- Pure Lua; positions and tangents in metres, as action tables have them.

local function module(name)
	local loaded = package and package.loaded and package.loaded["tpf3mp." .. name]
	if loaded then return loaded end
	if ug_require then return ug_require("tpf3mp_1::/scripts/tpf3mp/" .. name .. ".lua") end
	return require("tpf3mp." .. name)
end

local geom = module("geom")

local ghost = {}

-- tpf3mp/apply.lua's NODE_TOLERANCE: a receiver finds the node within this.
ghost.NODE_RADIUS = 1.5
-- How near a pending edge's centreline a loose end must be to split it.
-- Tighter than the receivers' own tolerance (geom.SPLIT_EPS), and the end is
-- moved onto the curve, so every receiver finds it on the edge.
ghost.EDGE_RADIUS = 2.0
-- Nearer a pending edge's end than this, the split is that end's node.
ghost.END_RADIUS = 2.5

local function arr(p) return { p.x, p.y, p.z } end
local function pos(a) return { x = a[1], y = a[2], z = a[3] } end
local function flat(a, b)
	local dx, dy = a[1] - b[1], a[2] - b[2]
	return math.sqrt(dx * dx + dy * dy)
end
local function scaled(t, k) return { t[1] * k, t[2] * k, t[3] * k } end

local function buildOf(action)
	if type(action) ~= "table" then return nil end
	if type(action.BuildRoad) == "table" then return action.BuildRoad, "Street" end
	if type(action.BuildTrack) == "table" then return action.BuildTrack, "Track" end
	return nil
end

local function linkNetwork(link, own)
	return type(link.kind) == "table" and link.kind.network or own
end

-- The pending builds' nodes and edges as they will stand once the room has
-- applied them in order: { nodes = { { network =, at = } }, edges = { {
-- network =, a =, b =, ta =, tb = } } }, arrays in metres. A pending split of
-- a pending edge replaces it with its halves, as tpf3mp/apply.lua splits one;
-- a pending removal of one takes it away.
function ghost.layout(pending)
	local nodes, edges = {}, {}
	for _, g in ipairs(pending or {}) do
		local build, own = buildOf(type(g) == "table" and g.action)
		local polyline = build and build.polyline
		if type(polyline) == "table" then
			local at = {}
			for i, v in ipairs(polyline.vertices or {}) do at[i - 1] = arr(v.pos) end
			for _, r in ipairs(polyline.removals or {}) do
				local a, b = arr(r.ends.a), arr(r.ends.b)
				for k = #edges, 1, -1 do
					local e = edges[k]
					if e.network == r.network and ((flat(e.a, a) <= ghost.NODE_RADIUS and flat(e.b, b) <= ghost.NODE_RADIUS)
							or (flat(e.a, b) <= ghost.NODE_RADIUS and flat(e.b, a) <= ghost.NODE_RADIUS)) then
						table.remove(edges, k)
					end
				end
			end
			for i, v in ipairs(polyline.vertices or {}) do
				local s = type(v.resolve) == "table" and v.resolve.Split
				if s then
					local p, a, b = at[i - 1], arr(s.ends.a), arr(s.ends.b)
					for k = #edges, 1, -1 do
						local e = edges[k]
						if e.network == s.network and flat(e.a, a) <= ghost.NODE_RADIUS and flat(e.b, b) <= ghost.NODE_RADIUS then
							local u = geom.parameterAt(e.a, e.ta, e.b, e.tb, p[1], p[2])
							local tm = geom.hermiteTangent(e.a, e.ta, e.b, e.tb, u)
							table.remove(edges, k)
							edges[#edges + 1] = { network = e.network, a = e.a, b = p, ta = scaled(e.ta, u), tb = scaled(tm, u) }
							edges[#edges + 1] = { network = e.network, a = p, b = e.b, ta = scaled(tm, 1 - u),
								tb = scaled(e.tb, 1 - u) }
							break
						end
					end
				end
			end
			for _, link in ipairs(polyline.links or {}) do
				local a, b = at[link.from], at[link.to]
				if a and b then
					local network = linkNetwork(link, own)
					edges[#edges + 1] = { network = network, a = a, b = b, ta = arr(link.tangent0), tb = arr(link.tangent1) }
					nodes[#nodes + 1] = { network = network, at = a }
					nodes[#nodes + 1] = { network = network, at = b }
				end
			end
		end
	end
	return { nodes = nodes, edges = edges }
end

-- The pending node of `network` nearest `p` within `radius`, or nil.
local function nearestNode(layout, network, p, radius)
	local best, bestD
	for _, n in ipairs(layout.nodes) do
		if n.network == network then
			local d = flat(n.at, p)
			if d <= radius and (bestD == nil or d < bestD) then best, bestD = n, d end
		end
	end
	return best
end

-- What a loose end at `p` lands on: { Node = network } and where, or the
-- split and where, or nil; or false and why when it lands ambiguously.
local function landing(layout, network, p)
	local n = nearestNode(layout, network, p, ghost.NODE_RADIUS)
	if n then return { Node = network }, n.at end
	local hit, hitU, hitD, second
	for _, e in ipairs(layout.edges) do
		if e.network == network then
			local u, d = geom.parameterAt(e.a, e.ta, e.b, e.tb, p[1], p[2])
			if d <= ghost.EDGE_RADIUS then
				if hit == nil or d < hitD then
					second = hit
					hit, hitU, hitD = e, u, d
				else
					second = e
				end
			end
		end
	end
	if hit == nil then return nil end
	if second ~= nil then return false, "it ends where two pending roads meet" end
	local q = geom.hermitePos(hit.a, hit.ta, hit.b, hit.tb, hitU)
	if flat(q, hit.a) < ghost.END_RADIUS then return { Node = network }, hit.a end
	if flat(q, hit.b) < ghost.END_RADIUS then return { Node = network }, hit.b end
	return { Split = { network = network, ends = { a = pos(hit.a), b = pos(hit.b) } } }, q
end

-- `action` (a BuildRoad or BuildTrack table, as tpf3mp/roads.lua makes one)
-- with its loose ends joined onto `pending` (tpf3mp_native.pending()'s
-- list). Returns the action (changed in place) and how many ends it joined;
-- or nil and why. Any other action, or none pending, comes back as it was.
function ghost.join(action, pending)
	local build, own = buildOf(action)
	if build == nil or type(pending) ~= "table" or #pending == 0 then return action, 0 end
	local polyline = build.polyline
	local layout = ghost.layout(pending)
	if #layout.edges == 0 then return action, 0 end
	local degree, network = {}, {}
	for _, link in ipairs(polyline.links) do
		local net = linkNetwork(link, own)
		for _, i in ipairs({ link.from, link.to }) do
			degree[i] = (degree[i] or 0) + 1
			if network[i] == nil then network[i] = net elseif network[i] ~= net then network[i] = false end
		end
	end
	local joined, at = 0, {}
	for i, v in ipairs(polyline.vertices) do
		local index = i - 1
		if v.resolve == "New" and degree[index] == 1 and network[index] then
			local resolve, where = landing(layout, network[index], arr(v.pos))
			if resolve == false then return nil, "a loose end the room cannot name: " .. where end
			if resolve then
				v.resolve, v.pos = resolve, pos(where)
				at[index] = where
				joined = joined + 1
			end
		end
	end
	for _, link in ipairs(polyline.links) do
		local a, b = at[link.from], at[link.to]
		if a and b and flat(a, b) <= ghost.NODE_RADIUS then
			return nil, "both ends of one edge on the same pending node"
		end
	end
	return action, joined
end

return ghost

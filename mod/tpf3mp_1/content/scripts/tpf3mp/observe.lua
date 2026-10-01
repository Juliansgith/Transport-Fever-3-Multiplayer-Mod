-- tpf3mp/observe.lua -- a summary of the world, for the hook's test mode
-- (crates/tpf3mp-hook/src/scenario.rs; docs/REGRESSION.md, "With the real
-- game").
--
-- In a game started with TPF3MP_SCENARIO, the hook asks the mod's game
-- script for an observation at some checkpoints (`observe()`), and the
-- script hands it this module's JSON text (`observed(step, text)`), which
-- the hook writes to hook.log as `scenario: observe step <n> <json>`. The
-- room's games then compare by diffing their logs: the same world reads the
-- same text. The first one is the scenario's baseline: the places and ids
-- its actions name are read from it.
--
--   { "step": n,
--     "towns": [ { "id":, "name":, "x":, "y":, "z":, "buildings":,
--                  "spots": [ { "x":, "y":, "z": } ],   -- the baseline's only
--                  "waters": [ { "x":, "y":, "z": } ],  -- the baseline's only
--                  "town_buildings": [ { "file":, "x":, "y":, "z": } ],
--                  "streets": [ { "a": [x, y, z], "b": [x, y, z],
--                                 "template": } ] } ],
--     "counts": { "street_edges":, "track_edges":, "constructions":,
--                 "stations":, "depots":, "lines":, "vehicles":, "people": },
--     "companies": [ { "id":, "name":, "money":, "gone": } ],
--     "lines": [ { "id":, "stops":, "vehicles": } ],
--     "next": { "lines":, "vehicles":, "groups":, "towns":, "industries":,
--               "companies": },
--     "game_time":, "models": [ name, ... ],         -- models: the baseline's only
--     "errors": [ "<part>: <why>" ] }
--
-- Positions in metres, to the centimetre. Towns by their entity, lowest
-- first, as every game of the same save has them; a town's centre is the
-- mean of its buildings'. A spot is a free flat place near a town: a square
-- of 120 m whose heights differ by at most 3 m, dry, at least 150 m from
-- the town's buildings and 60 m from any street or track node, on rings 300,
-- 450 and 600 m out, eight directions each, east first, counter-clockwise.
-- A town's streets are the ones nearest its centre.
--
-- Every part is read on its own: one that cannot be read is named in
-- `errors` and the others still come. Pure Lua against the `api` it is
-- given.

local observe = {}

local waters

observe.MAX_SPOTS = 4
observe.MAX_STREETS = 6
observe.MAX_MODELS = 600
observe.MAX_BUILDINGS = 3
observe.MAX_WATERS = 2

-- JSON ---------------------------------------------------------------------

local function quote(s)
	s = tostring(s)
	s = s:gsub('[%c"\\]', function(c)
		if c == '"' then return '\\"' end
		if c == "\\" then return "\\\\" end
		return string.format("\\u%04x", string.byte(c))
	end)
	return '"' .. s .. '"'
end

local function number(v)
	if v ~= v or v == math.huge or v == -math.huge then return "null" end
	if v == math.floor(v) and math.abs(v) < 1e15 then return string.format("%d", v) end
	return string.format("%.2f", v)
end

-- A table is an array when it has [1] or is marked { _array = true }.
local encode
function encode(v)
	local t = type(v)
	if t == "nil" then return "null" end
	if t == "boolean" then return v and "true" or "false" end
	if t == "number" then return number(v) end
	if t == "string" then return quote(v) end
	if t ~= "table" then return quote(tostring(v)) end
	if v[1] ~= nil or v._array then
		local out = {}
		for i = 1, #v do out[i] = encode(v[i]) end
		return "[" .. table.concat(out, ",") .. "]"
	end
	local keys = {}
	for k in pairs(v) do
		if k ~= "_array" then keys[#keys + 1] = tostring(k) end
	end
	table.sort(keys)
	local out = {}
	for _, k in ipairs(keys) do out[#out + 1] = quote(k) .. ":" .. encode(v[k]) end
	return "{" .. table.concat(out, ",") .. "}"
end
observe.encode = encode

local function array(t)
	t = t or {}
	t._array = true
	return t
end

-- Reads ----------------------------------------------------------------------

local function round2(v) return math.floor((v or 0) * 100 + 0.5) / 100 end

local function component(api, entity, kind)
	local ct = api.type.ComponentType[kind]
	if ct == nil then return nil end
	local ok, c = pcall(api.engine.getComponent, entity, ct)
	if ok then return c end
	return nil
end

local function entities(api, kind)
	local ct = api.type.ComponentType[kind]
	if ct == nil then error("no component " .. kind, 0) end
	local list = api.engine.getEntitiesWithComponent(ct)
	local out = {}
	for _, e in pairs(list) do out[#out + 1] = e end
	table.sort(out)
	return out
end

local function height(api, x, y)
	return api.engine.terrain.getHeightAt(api.type.Vec2f.new(x, y))
end

local function onWater(api, x, y)
	local ok, wet = pcall(api.engine.terrain.isOnWater, api.type.Vec2f.new(x, y))
	return ok and wet == true
end

-- The registry's next id of a kind (tpf3mp/registry.lua).
local function nextId(reg, kind)
	local r = type(reg) == "table" and reg[kind]
	return type(r) == "table" and r.next or nil
end

-- The registry's id of an entity of a kind.
local function idOf(reg, kind, e)
	local r = type(reg) == "table" and reg[kind]
	for _, pair in ipairs(type(r) == "table" and r.bound or {}) do
		if pair[2] == e then return pair[1] end
	end
	return nil
end

-- Every street and track edge, once: { entity, a = {x,y,z}, b =, template, track }.
local function edges(api)
	local out, seen = {}, {}
	for _, segments in pairs(api.engine.system.streetSystem.getNode2SegmentMap()) do
		for _, e in pairs(segments) do
			if not seen[e] then
				seen[e] = true
				local edge = component(api, e, "BASE_EDGE")
				if edge then
					local p0, p1 = edge.position0, edge.position1
					out[#out + 1] = {
						entity = e,
						a = { round2(p0.x or p0[1]), round2(p0.y or p0[2]), round2(p0.z or p0[3]) },
						b = { round2(p1.x or p1[1]), round2(p1.y or p1[2]), round2(p1.z or p1[3]) },
						template = tostring(edge.roadTemplate),
						track = component(api, e, "BASE_EDGE_TRACK") ~= nil,
					}
				end
			end
		end
	end
	table.sort(out, function(x, y) return x.entity < y.entity end)
	return out
end

-- Free flat places near (x, y), away from `avoid` (points, with the least
-- distance each) and `nodes`.
local function spots(api, x, y, buildings, nodes)
	local out = {}
	for _, r in ipairs({ 300, 450, 600 }) do
		for k = 0, 7 do
			if #out >= observe.MAX_SPOTS then return out end
			local a = k * math.pi / 4
			local sx, sy = x + r * math.cos(a), y + r * math.sin(a)
			local ok, free = pcall(function()
				local lo, hi = math.huge, -math.huge
				for _, dx in ipairs({ -60, 0, 60 }) do
					for _, dy in ipairs({ -60, 0, 60 }) do
						if onWater(api, sx + dx, sy + dy) then return false end
						local h = height(api, sx + dx, sy + dy)
						if h < lo then lo = h end
						if h > hi then hi = h end
					end
				end
				if hi - lo > 3 then return false end
				for _, b in ipairs(buildings) do
					if (b[1] - sx) ^ 2 + (b[2] - sy) ^ 2 < 150 ^ 2 then return false end
				end
				for _, n in ipairs(nodes) do
					if (n[1] - sx) ^ 2 + (n[2] - sy) ^ 2 < 60 ^ 2 then return false end
				end
				return true
			end)
			if ok and free then
				out[#out + 1] = { x = round2(sx), y = round2(sy), z = round2(height(api, sx, sy)) }
			end
		end
	end
	return out
end

-- Places on water near (x, y): a square of 60 m all on water, on rings 300
-- to 1200 m out, sixteen directions each, east first.
waters = function(api, x, y)
	local out = {}
	for _, r in ipairs({ 300, 600, 900, 1200 }) do
		for k = 0, 15 do
			if #out >= observe.MAX_WATERS then return out end
			local a = k * math.pi / 8
			local wx, wy = x + r * math.cos(a), y + r * math.sin(a)
			local wet = true
			for _, dx in ipairs({ -30, 0, 30 }) do
				for _, dy in ipairs({ -30, 0, 30 }) do
					if not onWater(api, wx + dx, wy + dy) then wet = false end
				end
			end
			if wet then
				local okH, h = pcall(height, api, wx, wy)
				out[#out + 1] = { x = round2(wx), y = round2(wy), z = okH and round2(h) or 0 }
			end
		end
	end
	return out
end

-- The summary of the world after `step`, as JSON text. `state` is the game
-- script's saved state (its registry and roster), `withSpots` whether to
-- look for free places (the baseline only: it is the slow part).
function observe.read(api, state, step, withSpots)
	local out = { step = step, errors = array() }
	local saved = type(state) == "table" and state or {}
	local reg = saved.registry
	local function part(name, f)
		local ok, why = pcall(f)
		if not ok then out.errors[#out.errors + 1] = name .. ": " .. tostring(why) end
	end

	local allEdges = {}
	part("edges", function() allEdges = edges(api) end)

	local counts = {}
	out.counts = counts
	part("counts", function()
		local street, track = 0, 0
		for _, e in ipairs(allEdges) do
			if e.track then track = track + 1 else street = street + 1 end
		end
		counts.street_edges, counts.track_edges = street, track
	end)
	part("constructions", function() counts.constructions = #entities(api, "CONSTRUCTION") end)
	part("stations", function() counts.stations = #entities(api, "STATION") end)
	part("depots", function()
		local n = 0
		for _, e in ipairs(entities(api, "CONSTRUCTION")) do
			local c = component(api, e, "CONSTRUCTION")
			if c and c.depots and #c.depots > 0 then n = n + 1 end
		end
		counts.depots = n
	end)
	part("vehicles", function() counts.vehicles = #entities(api, "TRANSPORT_VEHICLE") end)
	part("people", function() counts.people = #entities(api, "SIM_PERSON") end)

	out.lines = array()
	part("lines", function()
		local list = {}
		for _, e in pairs(api.engine.system.lineSystem.getLines()) do list[#list + 1] = e end
		table.sort(list)
		counts.lines = #list
		for _, e in ipairs(list) do
			local line = component(api, e, "LINE")
			local vehicles = nil
			pcall(function()
				local v = api.engine.system.transportVehicleSystem.getLineVehicles(e)
				local n = 0
				for _ in pairs(v) do n = n + 1 end
				vehicles = n
			end)
			out.lines[#out.lines + 1] = { id = idOf(reg, "lines", e), stops = line and line.stops and #line.stops,
				vehicles = vehicles }
		end
	end)

	out.companies = array()
	part("companies", function()
		local roster = saved.companies
		if type(roster) ~= "table" or type(roster.list) ~= "table" then
			local player = api.engine.util.getPlayer()
			local account = component(api, player, "ACCOUNT")
			out.companies[1] = { id = 0, money = account and account.balance }
			return
		end
		for _, c in ipairs(roster.list) do
			local account = component(api, c.entity, "ACCOUNT")
			out.companies[#out.companies + 1] = { id = c.id, name = c.name, gone = c.gone == true or nil,
				money = account and account.balance }
		end
	end)

	part("game_time", function()
		out.game_time = api.engine.getComponent(api.engine.util.getWorld(),
			api.type.ComponentType.GAME_TIME).gameTime
	end)
	-- The baseline only: the vehicle models this game has, by the names a
	-- BuyVehicle's consist gives them.
	if withSpots then
		part("models", function()
			local names = {}
			for _, name in pairs(api.res.modelRep.getAll()) do
				name = tostring(name)
				if name:find("vehicle/", 1, true) then names[#names + 1] = name end
			end
			table.sort(names)
			out.models = array()
			for i = 1, math.min(#names, observe.MAX_MODELS) do out.models[i] = names[i] end
		end)
	end

	out.next = {}
	for _, kind in ipairs({ "lines", "vehicles", "groups", "towns", "industries" }) do
		out.next[kind] = nextId(reg, kind)
	end
	if type(saved.companies) == "table" then out.next.companies = saved.companies.next end

	out.towns = array()
	part("towns", function()
		local nodes = {}
		if withSpots then
			for _, e in ipairs(allEdges) do
				nodes[#nodes + 1] = e.a
				nodes[#nodes + 1] = e.b
			end
		end
		local map = api.engine.system.townBuildingSystem.getTown2BuildingMap()
		for _, town in ipairs(entities(api, "TOWN")) do
			local entry = { id = idOf(reg, "towns", town), entity = town }
			local name = component(api, town, "NAME")
			entry.name = name and name.name or nil
			local points, sx, sy = {}, 0, 0
			for _, b in pairs(map and map[town] or {}) do
				local tb = component(api, b, "TOWN_BUILDING")
				local c = tb and component(api, tb.construction, "CONSTRUCTION")
				if c and c.transf then
					points[#points + 1] = { c.transf[13], c.transf[14], c.transf[15], tostring(c.fileName), tb.construction }
					sx, sy = sx + c.transf[13], sy + c.transf[14]
				end
			end
			entry.buildings = #points
			if #points > 0 then
				local x, y = sx / #points, sy / #points
				entry.x, entry.y = round2(x), round2(y)
				local okH, h = pcall(height, api, x, y)
				entry.z = okH and round2(h) or nil
				-- The streets nearest the centre.
				local near = {}
				for _, e in ipairs(allEdges) do
					if not e.track then
						local mx, my = (e.a[1] + e.b[1]) / 2, (e.a[2] + e.b[2]) / 2
						near[#near + 1] = { d = (mx - x) ^ 2 + (my - y) ^ 2, e = e }
					end
				end
				table.sort(near, function(p, q)
					if p.d ~= q.d then return p.d < q.d end
					return p.e.entity < q.e.entity
				end)
				entry.streets = array()
				for i = 1, math.min(observe.MAX_STREETS, #near) do
					local e = near[i].e
					entry.streets[i] = { a = e.a, b = e.b, template = e.template }
				end
				-- The town buildings nearest the centre.
				local byDistance = {}
				for _, b in ipairs(points) do
					byDistance[#byDistance + 1] = { d = (b[1] - x) ^ 2 + (b[2] - y) ^ 2, b = b }
				end
				table.sort(byDistance, function(p, q)
					if p.d ~= q.d then return p.d < q.d end
					return p.b[5] < q.b[5]
				end)
				entry.town_buildings = array()
				for i = 1, math.min(observe.MAX_BUILDINGS, #byDistance) do
					local b = byDistance[i].b
					entry.town_buildings[i] = { file = b[4], x = round2(b[1]), y = round2(b[2]), z = round2(b[3]) }
				end
				if withSpots then
					entry.spots = array(spots(api, x, y, points, nodes))
					entry.waters = array(waters(api, x, y))
				end
			end
			entry.entity = nil
			out.towns[#out.towns + 1] = entry
		end
	end)
	return encode(out)
end

return observe

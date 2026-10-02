-- tpf3mp/lanes.lua -- the world's lanes, as the game script reads them at a
-- checkpoint (docs/HOOKS.md, "The world's lanes").
--
-- A lane is one part of the world summed up in a short text: the same text
-- on two games means that part of their worlds is the same. The mod's game
-- script reads them in its postUpdate, right after the last update of a
-- batch that ends at a checkpoint step, and hands them to the hook, which
-- reports their digests to the room; the room compares them between
-- players. The numbers follow the regression harness's model
-- (crates/tpf3mp-testkit/src/regress/model.rs, `lane`), with two more.
--
-- What each lane reads, from the engine the game script runs in:
--
-- - NETWORK: every street and track edge, by its ends to 0.1 m and its road
--   template, lane settings and portable junction configurations;
-- - CONSTRUCTIONS: every construction, by its file and position to 0.1 m;
-- - LINES: every line's number of stops;
-- - VEHICLES: each vehicle's state, stop, and place on its path (edge and
--   distance to 1 cm, speed to 1 cm/s): the simulation's own state
--   (MOVE_PATH.dyn). Not its position in the world: getPosition, and the
--   path state as the frame began (dyn0), differed between two games in
--   the same simulation update by millimetres (build 40408), the frames
--   being their own; the simulation's state did not;
-- - ECONOMY: each player's balance;
-- - TOWNS: each town's number of buildings;
-- - PEOPLE: the number of people.
--
-- Nothing is read by an entity's id where an id could differ between two
-- games that agree on the world, except where the save carries it (towns and
-- players). Each lane is read on its own: one that cannot be read is
-- "err" on every game alike, and the others still count.
--
-- The engine lists the entities of some components only
-- (getEntitiesWithComponent refuses BASE_EDGE, LINE and PLAYER on build
-- 40408: "Cannot loop over this component type"), so edges come from the
-- street system's node map, lines from the line system, and the player
-- from the engine's util.
--
-- A lane can also be dumped (`lanes.dump`): its full text, entry by entry,
-- read by the same reader that sums it up, with the raw values it rounds
-- (`%.17g`), keyed by the registry's id where there is one (vehicle-N,
-- line-N, town-N, industry-N) and else by the row it hashes, sorted the
-- same on every game that has the same world; then its summary, the text
-- the hook hashes. The hook writes each entry to hook.log as
-- `lane <n> step <step> <entry>`, for tools/lane_diff.py to diff between
-- games (docs/HOOKS.md, "Lane dumps"). Reading a lane for its digest builds
-- none of it.
--
-- Pure Lua over the `api` it is given; the tests hand it a fake.

local lanes = {}
local junctions
if type(ug_require) == "function" then
	junctions = ug_require("tpf3mp_1::/scripts/tpf3mp/junctions.lua")
else
	junctions = require("tpf3mp.junctions")
end

lanes.NETWORK = 0
lanes.CONSTRUCTIONS = 1
lanes.LINES = 2
lanes.VEHICLES = 3
lanes.ECONOMY = 4
lanes.TOWNS = 5
lanes.PEOPLE = 6

-- A pure-Lua hash (tools/probe's): the same on every Lua and platform.
local M1, A1 = 2147483647, 48271
local M2, A2 = 2147483629, 40692
local function hashStr(s)
	local h1, h2 = 2166136261 % M1, 2166136261 % M2
	for i = 1, #s do
		local b = string.byte(s, i)
		h1 = (h1 * A1 + b) % M1
		h2 = (h2 * A2 + b) % M2
	end
	return string.format("%010d-%010d", h1, h2)
end

-- A sorted list's count and hash.
local function summary(rows)
	table.sort(rows)
	return #rows .. ":" .. hashStr(table.concat(rows, "\30"))
end

local function q01(v) return math.floor((v or 0) * 10 + 0.5) / 10 end

local function vec01(p)
	return string.format("%s,%s,%s", q01(p.x or p[1]), q01(p.y or p[2]), q01(p.z or p[3] or 0))
end

local function entities(api, kind)
	local list = api.engine.getEntitiesWithComponent(api.type.ComponentType[kind])
	local out = {}
	for _, e in pairs(list) do out[#out + 1] = e end
	return out
end

local function component(api, entity, kind)
	return api.engine.getComponent(entity, api.type.ComponentType[kind])
end

-- A field only a dump reads, or nil: the game's components are userdata,
-- which raise on a field they lack.
local function get(value, key)
	if value == nil then return nil end
	local ok, v = pcall(function() return value[key] end)
	if ok then return v end
	return nil
end

-- A number at full precision, for a dump; anything else as text.
local function full(v)
	if type(v) == "number" then return string.format("%.17g", v) end
	return tostring(v)
end

-- A vector at full precision, for a dump: a table, or the game's userdata
-- (a Vec3f, whose fields x, y and z read but which has no [1]); anything
-- without numbers as text.
local function vecFull(p)
	if p == nil then return "nil" end
	local x, y, z = get(p, "x"), get(p, "y"), get(p, "z")
	if type(x) ~= "number" and type(p) == "table" then x, y, z = p[1], p[2], p[3] end
	if type(x) ~= "number" then return tostring(p) end
	return full(x) .. "," .. full(y) .. "," .. full(z or 0)
end

-- The base game's town growth script's state, as its own
-- town_cargo_util.getTownCargoState reads it: a function from a town to its
-- { experience, level, ... }, or nil. For a dump only; it changes nothing.
local function townGrowth(api)
	local ok, script = pcall(function()
		local e = api.engine.system.gameScriptSystem.getEntityForGameScript("::/game_mechanics/towns/town_cargo.gs")
		return api.engine.getComponent(e, api.type.ComponentType.GAME_SCRIPT)
	end)
	if not ok or script == nil then return function() return nil end end
	local native, plain = get(script, "state_native"), nil
	return function(town)
		if native ~= nil then
			local found, data = pcall(function()
				local d = native:findPath({ "townState", town })
				return d and d:asTable()
			end)
			if found and data ~= nil then return data end
		end
		if plain == nil then
			local state = get(script, "state")
			plain = type(state) == "table" and type(state.townState) == "table" and state.townState or false
		end
		return plain and plain[town] or nil
	end
end

-- Each reader returns its lane's text. With `emit` (a dump), it also calls
-- emit(kind, entity, row, fields) for every row it hashes: the registry's
-- kind that names the entity, if any, the row as hashed, and the raw values
-- it was made from. Without `emit` it builds no fields.
local readers = {}

readers[lanes.NETWORK] = function(api, emit)
	local rows, seen = {}, {}
	for _, segments in pairs(api.engine.system.streetSystem.getNode2SegmentMap()) do
		for _, e in pairs(segments) do
			if not seen[e] then
				seen[e] = true
				local edge = component(api, e, "BASE_EDGE")
				if edge then
					local a, b = vec01(edge.position0), vec01(edge.position1)
					local reversed = a > b
					if reversed then a, b = b, a end
					local row = a .. ">" .. b .. ":" .. tostring(edge.roadTemplate)
					local laneRows = {}
					for i = 1, #edge.laneConfigs do
						local l, modes = edge.laneConfigs[i], {}
						for m = 0, 15 do modes[#modes+1] = l.transportModes[m] == true and "1" or "0" end
						laneRows[#laneRows+1] = string.format("%.3f/%.3f/%.3f/%.3f/%s/%s", l.speed,l.width,l.height,
							l.offset * (reversed and -1 or 1), tostring(l.forward ~= reversed), table.concat(modes))
					end
					table.sort(laneRows)
					row = row .. "|lanes:" .. table.concat(laneRows,";")
					rows[#rows + 1] = row
					if emit then
						emit(nil, e, row, "p0=" .. vecFull(edge.position0) .. " p1=" .. vecFull(edge.position1)
							.. " template=" .. tostring(edge.roadTemplate))
					end
				end
			end
		end
	end
	for _, row in ipairs(junctions.rows(api)) do
		rows[#rows+1] = "junction:" .. row
		if emit then emit(nil, nil, "junction:" .. row, "") end
	end
	return summary(rows)
end

readers[lanes.CONSTRUCTIONS] = function(api, emit)
	local rows = {}
	for _, e in ipairs(entities(api, "CONSTRUCTION")) do
		local c = component(api, e, "CONSTRUCTION")
		if c then
			local t = c.transf
			local x, y = 0, 0
			if t then x, y = t[13] or 0, t[14] or 0 end
			local row = string.format("%s@%s,%s", tostring(c.fileName), q01(x), q01(y))
			rows[#rows + 1] = row
			if emit then
				emit("industries", e, row, "file=" .. tostring(c.fileName) .. " x=" .. full(x) .. " y=" .. full(y)
					.. " z=" .. full(t and t[15]))
			end
		end
	end
	return summary(rows)
end

readers[lanes.LINES] = function(api, emit, ids)
	local rows = {}
	for _, e in pairs(api.engine.system.lineSystem.getLines()) do
		local line = component(api, e, "LINE")
		local row = tostring(line and line.stops and #line.stops or "?")
		rows[#rows + 1] = row
		if emit then
			local fields = { "stops=" .. row }
			local stops = get(line, "stops")
			local n = tonumber(row) or 0
			for i = 1, n do
				local stop = get(stops, i)
				fields[#fields + 1] = "stop" .. i .. "=" .. ids("groups", get(stop, "stationGroup"), "group") .. "/"
					.. full(get(stop, "station")) .. "/" .. full(get(stop, "terminal"))
			end
			emit("lines", e, row, table.concat(fields, " "))
		end
	end
	return summary(rows)
end

readers[lanes.VEHICLES] = function(api, emit, ids)
	local rows = {}
	for _, e in ipairs(entities(api, "TRANSPORT_VEHICLE")) do
		local v = component(api, e, "TRANSPORT_VEHICLE")
		local path = component(api, e, "MOVE_PATH")
		local d = path and path.dyn
		local where = "-"
		if d and d.pathPos then
			where = string.format("%d@%.2f v%.2f", d.pathPos.edgeIndex, d.pathPos.pos, d.speed)
		end
		local row = tostring(v and v.state) .. ":" .. tostring(v and v.stopIndex) .. ":" .. where
		rows[#rows + 1] = row
		if emit then
			local pos = d and d.pathPos
			local arrival = get(v, "arrivalStationTerminal")
			local route = get(path, "path")
			local detail = ""
			if route then
				local edges = get(route, "edges") or {}
				local entries, nearby = {}, {}
				local current = get(pos, "edgeIndex") or 0
				for i = 1, math.min(#edges, 4096) do
					local edge = edges[i]
					local id = get(edge, "edgeId") or get(edge, 1)
					local direction = get(edge, "dir")
					if direction == nil then direction = get(edge, 2) end
					local text = full(get(id, "entity")) .. "/" .. full(get(id, "index")) .. "/" .. tostring(direction)
					entries[#entries + 1] = text
					if i >= current - 1 and i <= current + 8 then nearby[#nearby + 1] = (i - 1) .. ":" .. text end
				end
				detail = " path_count=" .. #edges .. " path_hash=" .. hashStr(table.concat(entries, ";"))
					.. " path_sampled=" .. #entries .. " path_near=" .. table.concat(nearby, ",")
					.. " path_end=" .. full(get(route, "endOffset")) .. " decision_offset=" .. full(get(route, "terminalDecisionOffset"))
					.. " end_param=" .. full(get(path, "endParam")) .. " end_pos=" .. full(get(path, "endPos"))
					.. " blocked=" .. full(get(path, "blocked")) .. " move_state=" .. full(get(path, "state"))
					.. " accel=" .. full(get(d, "accel")) .. " standing=" .. full(get(d, "timeStanding"))
					.. " until_accel=" .. full(get(d, "timeUntilAccel")) .. " approaching=" .. tostring(get(d, "approachingStation"))
			end
			emit("vehicles", e, row, "state=" .. tostring(v and v.state) .. " stop=" .. tostring(v and v.stopIndex)
				.. " line=" .. ids("lines", get(v, "line"), "line")
				.. " edge=" .. full(pos and pos.edgeIndex) .. " pos=" .. full(pos and pos.pos)
				.. " speed=" .. full(d and d.speed)
				.. " arrival=" .. full(get(arrival, "station")) .. "/" .. full(get(arrival, "terminal"))
					.. " arrival_locked=" .. tostring(get(v, "arrivalStationTerminalLocked")) .. detail)
		end
	end
	return summary(rows)
end

readers[lanes.ECONOMY] = function(api, emit)
	local player = api.engine.util.getPlayer()
	local account = component(api, player, "ACCOUNT")
	local balance = account and account.balance
	local text = tostring(player) .. ":" .. (balance ~= nil and string.format("%d", balance) or "?")
	if emit then emit("player", player, text, "balance=" .. full(balance)) end
	return text
end

readers[lanes.TOWNS] = function(api, emit)
	local map = api.engine.system.townBuildingSystem.getTown2BuildingMap()
	local rows = {}
	local growth = emit and townGrowth(api)
	for _, town in ipairs(entities(api, "TOWN")) do
		local count = 0
		local buildings = map and map[town]
		if type(buildings) == "table" then
			for _ in pairs(buildings) do count = count + 1 end
		end
		local row = tostring(town) .. ":" .. count
		rows[#rows + 1] = row
		if emit then
			-- The town's size factors at full precision, and the growth
			-- script's experience and level: what makeTownUpdateSizeCmd is
			-- made from (docs/HOOKS.md, "The town trace").
			local size = get(component(api, town, "TOWN"), "sizeFactors")
			local factors = {}
			for i = 1, 3 do factors[i] = full(get(size, i)) end
			local data = growth(town)
			emit("towns", town, row, "buildings=" .. count .. " size=" .. table.concat(factors, ",")
				.. " experience=" .. full(get(data, "experience")) .. " level=" .. full(get(data, "level")))
		end
	end
	return summary(rows)
end

readers[lanes.PEOPLE] = function(api, emit)
	local text = tostring(#entities(api, "SIM_PERSON"))
	if emit then emit("people", nil, text, "count=" .. text) end
	return text
end

-- Every lane's text, by lane number, and the lanes that could not be read
-- with why, from the `api` of the state the game script runs in.
function lanes.read(api)
	local out, failed = {}, {}
	for lane = lanes.NETWORK, lanes.PEOPLE do
		local ok, text = pcall(readers[lane], api)
		if ok and type(text) == "string" then
			out[lane] = text
		else
			out[lane] = "err"
			failed[#failed + 1] = lane .. ": " .. tostring(text)
		end
	end
	return out, failed
end

-- The registry's kinds as a dump names their ids.
local PREFIX = { vehicles = "vehicle", lines = "line", towns = "town", industries = "industry",
	groups = "group" }

-- Lane `lane` entry by entry, as a list of lines, sorted: each
-- `<key> <field=value> ... entity=<e> row=<row>`, then `summary <text>`, the
-- lane's text as lanes.read reads it; a lane that cannot be read is the one
-- line `err <why>`. `reg` is the registry of the game script's state
-- (tpf3mp/registry.lua), which names the keys; nil names none.
function lanes.dump(api, lane, reg)
	local reader = readers[lane]
	if reader == nil then return { "err no lane " .. tostring(lane) } end
	-- The registry's ids by entity, per kind, made when first asked.
	local byEntity = {}
	local function idOf(kind, e)
		if kind == nil or e == nil then return nil end
		local map = byEntity[kind]
		if map == nil then
			map = {}
			local r = type(reg) == "table" and reg[kind]
			for _, pair in ipairs(type(r) == "table" and r.bound or {}) do map[pair[2]] = pair[1] end
			byEntity[kind] = map
		end
		return map[e]
	end
	-- An entity a field names, by its id if it has one.
	local function ids(kind, e, prefix)
		local id = idOf(kind, e)
		if id ~= nil then return prefix .. "-" .. id end
		if e == nil then return "nil" end
		return "entity-" .. tostring(e)
	end
	local entries = {}
	local function emit(kind, e, row, fields)
		local id = idOf(kind, e)
		local key, order
		if id ~= nil then
			key = PREFIX[kind] .. "-" .. id
			order = string.format("0 %s %015d", PREFIX[kind], id)
		elseif kind == "player" or kind == "people" then
			key, order = kind, "0 " .. kind
		else
			key = "row:" .. string.gsub(row, "%s", "_")
			order = "1 " .. row .. "\30" .. string.format("%015d", tonumber(e) or 0)
		end
		local text = key .. " " .. fields
		if e ~= nil then text = text .. " entity=" .. tostring(e) end
		entries[#entries + 1] = { order = order, text = text .. " row=" .. row }
	end
	local ok, text = pcall(reader, api, emit, ids)
	if not ok or type(text) ~= "string" then return { "err " .. tostring(text) } end
	table.sort(entries, function(a, b) return a.order < b.order end)
	local out = {}
	for i, entry in ipairs(entries) do out[i] = entry.text end
	out[#out + 1] = "summary " .. text
	return out
end

lanes.hash = hashStr

return lanes

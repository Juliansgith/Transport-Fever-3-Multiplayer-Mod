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
--   template;
-- - CONSTRUCTIONS: every construction, by its file and position to 0.1 m;
-- - LINES: every line's number of stops;
-- - VEHICLES: the vehicles' positions to 1 m;
-- - ECONOMY: each player's balance;
-- - TOWNS: each town's number of buildings;
-- - PEOPLE: the number of people.
--
-- Nothing is read by an entity's id where an id could differ between two
-- games that agree on the world, except where the save carries it (towns and
-- players). Each lane is read on its own: one that cannot be read is
-- "err" on every game alike, and the others still count.
--
-- A vehicle's position comes from api.engine.util.vehicle.getPosition
-- (engine/util.d.tl's UtilVehicle; util.transport has none on build 40408).
--
-- The engine lists the entities of some components only
-- (getEntitiesWithComponent refuses BASE_EDGE, LINE and PLAYER on build
-- 40408: "Cannot loop over this component type"), so edges come from the
-- street system's node map, lines from the line system, and the player
-- from the engine's util.
--
-- Pure Lua over the `api` it is given; the tests hand it a fake.

local lanes = {}

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

local function q1(v) return math.floor((v or 0) + 0.5) end
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

local readers = {}

readers[lanes.NETWORK] = function(api)
	local rows, seen = {}, {}
	for _, segments in pairs(api.engine.system.streetSystem.getNode2SegmentMap()) do
		for _, e in pairs(segments) do
			if not seen[e] then
				seen[e] = true
				local edge = component(api, e, "BASE_EDGE")
				if edge then
					local a, b = vec01(edge.position0), vec01(edge.position1)
					if a > b then a, b = b, a end
					rows[#rows + 1] = a .. ">" .. b .. ":" .. tostring(edge.roadTemplate)
				end
			end
		end
	end
	return summary(rows)
end

readers[lanes.CONSTRUCTIONS] = function(api)
	local rows = {}
	for _, e in ipairs(entities(api, "CONSTRUCTION")) do
		local c = component(api, e, "CONSTRUCTION")
		if c then
			local t = c.transf
			local x, y = 0, 0
			if t then x, y = t[13] or 0, t[14] or 0 end
			rows[#rows + 1] = string.format("%s@%s,%s", tostring(c.fileName), q01(x), q01(y))
		end
	end
	return summary(rows)
end

readers[lanes.LINES] = function(api)
	local rows = {}
	for _, e in pairs(api.engine.system.lineSystem.getLines()) do
		local line = component(api, e, "LINE")
		rows[#rows + 1] = tostring(line and line.stops and #line.stops or "?")
	end
	return summary(rows)
end

readers[lanes.VEHICLES] = function(api)
	local rows = {}
	for _, e in ipairs(entities(api, "TRANSPORT_VEHICLE")) do
		local p = api.engine.util.vehicle.getPosition(e)
		if p then
			rows[#rows + 1] = string.format("%d,%d,%d", q1(p.x or p[1]), q1(p.y or p[2]), q1(p.z or p[3]))
		else
			rows[#rows + 1] = "?"
		end
	end
	return summary(rows)
end

readers[lanes.ECONOMY] = function(api)
	local player = api.engine.util.getPlayer()
	local account = component(api, player, "ACCOUNT")
	local balance = account and account.balance
	return tostring(player) .. ":" .. (balance ~= nil and string.format("%d", balance) or "?")
end

readers[lanes.TOWNS] = function(api)
	local map = api.engine.system.townBuildingSystem.getTown2BuildingMap()
	local rows = {}
	for _, town in ipairs(entities(api, "TOWN")) do
		local count = 0
		local buildings = map and map[town]
		if type(buildings) == "table" then
			for _ in pairs(buildings) do count = count + 1 end
		end
		rows[#rows + 1] = tostring(town) .. ":" .. count
	end
	return summary(rows)
end

readers[lanes.PEOPLE] = function(api)
	return tostring(#entities(api, "SIM_PERSON"))
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

lanes.hash = hashStr

return lanes

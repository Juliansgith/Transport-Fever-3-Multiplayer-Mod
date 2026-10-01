-- tpf3mp/capitals.lua -- every company's capital on the map, in the GUI.
--
-- Build 40408 crowns one town, the player's capital: the town label's
-- recipe (gui/main/town_hud_react_util.tl, TownHudIcon) asks
-- town_util.isCapital(town), which reads api.engine.util.getPlayer()'s
-- PLAYER `headquarters` and answers true for the town closest to it
-- (game_mechanics/towns/town_util.tl), then gives the label the style class
-- `capital-city` (blue, gui/main/hud_icon_master.css.lua) and a crown. In a
-- room the GUI's getPlayer() answers the player's own company
-- (tpf3mp/follow.lua), so each player sees their own capital only.
--
-- With more than one company in the room, in the GUI's Lua states only
-- (gui/tpf3mp/capitals.script.lua; never the simulation's):
--
-- - isCapital answers true for every live company's capital as well, so
--   every company's headquarters town has the game's crown;
-- - the town label is wrapped in a layout that says whose capital it is
--   ("Capital of Rival", "Capital of Rival and Mine") and, for a capital
--   that is not the viewer's company's, carries the class of that
--   company's colour (`tpf3mp-capital-<n>`, gui/tpf3mp/tpf3mp.css.lua),
--   which colours the label's tile in place of the game's blue. The
--   viewer's own capital keeps the game's blue.
--
-- With one company (co-op, or single player) nothing changes. Which town
-- is whose is read again every `EVERY` seconds, not every frame: each live
-- company's PLAYER `headquarters` and the town closest to it, as the
-- game's isCapital reads its own.
--
-- Pure Lua against the game's `api`; the tests give it fakes.

local companies = ug_require and ug_require("tpf3mp_1::/scripts/tpf3mp/companies.lua")
	or require("tpf3mp.companies")

local capitals = {}

-- Seconds between two readings of whose capital is where.
capitals.EVERY = 10.0

-- The style class of a capital label in palette colour `index`.
function capitals.class(index)
	return companies.capitalClass(index)
end

-- The index of the palette colour closest to `color` ({ r, g, b }): a
-- company's own colour when it wears one of the palette's, else the nearest
-- (a company may be recoloured to any colour; the style sheet has a class
-- for the palette's only). Nil for what is no colour.
function capitals.swatchNear(color)
	if type(color) ~= "table" then return nil end
	local best, bestD = nil, nil
	for i, p in ipairs(companies.PALETTE) do
		local d = 0
		for k = 1, 3 do
			local v = color[k]
			if type(v) ~= "number" then return nil end
			d = d + (v - p[k]) * (v - p[k])
		end
		if bestD == nil or d < bestD then best, bestD = i, d end
	end
	return best
end

-- "A", "A and B", "A, B and C".
local function names(list)
	local out = {}
	for _, c in ipairs(list) do out[#out + 1] = tostring(c.name) end
	if #out <= 1 then return out[1] or "" end
	return table.concat(out, ", ", 1, #out - 1) .. " and " .. out[#out]
end

-- Whose capital each town is, from `roster` (tpf3mp/companies.lua):
-- { [town] = { company, ... } } in the roster's order, each company a live
-- one whose PLAYER names a headquarters that has a closest town. Nil with
-- one company or none: the game's own capital only.
function capitals.build(roster, api)
	if type(roster) ~= "table" or type(roster.list) ~= "table" or not companies.painting(roster) then
		return nil
	end
	local map = {}
	for _, c in ipairs(companies.live(roster)) do
		local town = nil
		pcall(function()
			local p = api.engine.getComponent(c.entity, api.type.ComponentType.PLAYER)
			local hq = p and p.headquarters
			if type(hq) ~= "number" or hq < 0 then return end
			local t = api.engine.system.streetConnectorSystem.getConstructionClosestTown(hq)
			if type(t) == "number" and t >= 0 then town = t end
		end)
		if town then
			local list = map[town]
			if not list then list = {} map[town] = list end
			list[#list + 1] = c
		end
	end
	return map
end

-- What the label of `town` shows, from `map` (capitals.build) for the
-- viewer whose company is the player entity `mine`: nil for a town that is
-- no company's capital (or with one company), else
-- { label = "Capital of ...", class = "" for the viewer's own capital (the
-- game's blue) or the class of the first other company's colour }.
function capitals.view(map, town, mine)
	local list = type(map) == "table" and map[town]
	if not list or #list == 0 then return nil end
	local class = ""
	local own = false
	for _, c in ipairs(list) do
		if c.entity == mine then own = true end
	end
	if not own then
		local index = capitals.swatchNear(list[1].color)
		if index then class = capitals.class(index) end
	end
	return { label = "Capital of " .. names(list), class = class }
end

-- This Lua state's map, read again once `EVERY` seconds have passed
-- (`now()`, os.clock by default), from the mod's game script's roster.
local cached, cachedAt = nil, nil
function capitals.current(api, now)
	local ok, t = pcall(now or os.clock)
	if not ok or type(t) ~= "number" then t = nil end
	if t == nil or cachedAt == nil or t - cachedAt >= capitals.EVERY or t < cachedAt then
		cachedAt = t
		local read, map = pcall(function()
			local state = companies.scriptState(api)
			return capitals.build(state and state.companies, api)
		end)
		cached = read and map or nil
	end
	return cached
end

-- Forgets the map, so the next look reads it again (for the tests).
function capitals.forget()
	cached, cachedAt = nil, nil
end

-- Has the game's town_util.isCapital answer true for every company's
-- capital too (`lookup()`, a map as capitals.build gives, nil with one
-- company), in this Lua state. The game loads town_util as
-- "/game_mechanics/..." and as "::/game_mechanics/...": each table either
-- gives is changed, once. Returns how many tables now answer so, or nil and
-- why.
capitals.TOWN_UTIL = { "::/game_mechanics/towns/town_util.tl", "/game_mechanics/towns/town_util.tl" }
function capitals.followIsCapital(require_, lookup)
	local changed, why, seen = 0, nil, {}
	for _, path in ipairs(capitals.TOWN_UTIL) do
		local ok, util = pcall(require_, path)
		if ok and type(util) == "table" and seen[util] then
			-- The same table under its other name.
		elseif ok and type(util) == "table" and type(util.isCapital) == "function" then
			seen[util] = true
			if not util.tpf3mpCapitals then
				local original = util.isCapital
				util.isCapital = function(town, ...)
					local okOwn, own = pcall(original, town, ...)
					if okOwn and own then return own end
					local got, map = pcall(lookup)
					if got and type(map) == "table" and map[town] ~= nil then return true end
					return okOwn and own or false
				end
				util.tpf3mpCapitals = true
			end
			changed = changed + 1
		else
			why = why or ("the game's town_util did not load: " .. tostring(util))
		end
	end
	if changed == 0 then return nil, why or "the game's town_util did not load" end
	return changed
end

return capitals

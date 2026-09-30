-- tpf3mp/capture.lua -- a build the player made with the game's own tools,
-- as the action the room orders instead (docs/HOOKS.md, "The build tools").
--
-- The tools show every proposal they make to game scripts
-- (builder.proposalCreate), with the proposal as the game will build it;
-- the mod's game script keeps the action each makes, and hands the room the
-- one the player clicked. Everything is read from the proposal as it is, in
-- the game's units: metres, and plain fractions for the matrix.
--
-- A proposal the room cannot carry yet is not guessed at: the capture says
-- why, and the tool shows it.
--
-- Pure Lua; the tests hand it proposals of the game's shape.

local capture = {}

local function get(value, key)
	local ok, v = pcall(function() return value[key] end)
	if ok then return v end
	return nil
end

local function length(list)
	if list == nil then return 0 end
	local ok, n = pcall(function() return #list end)
	if ok and type(n) == "number" then return n end
	return nil
end

local function sortedKeys(tbl)
	local keys = {}
	for key in pairs(tbl) do keys[#keys + 1] = key end
	table.sort(keys, function(a, b)
		if type(a) == type(b) then return a < b end
		return type(a) == "number"
	end)
	return keys
end

-- A construction's parameters as the schema's flat list (tpf3mp_proto
-- action::Param): nested tables become paths, "modules[3801].name"; a number
-- with no fraction is Int, any other Fixed; a boolean Bool, a string Text.
-- Returns the list, or nil and why.
function capture.params(tbl)
	local out = {}
	local function walk(node, path, depth)
		if depth > 8 then error("parameters nested deeper than 8") end
		for _, key in ipairs(sortedKeys(node)) do
			local value = node[key]
			local here
			if type(key) == "number" and key == math.floor(key) then
				here = path .. "[" .. string.format("%d", key) .. "]"
			elseif type(key) == "string" and key:match("^[%a_][%w_]*$") then
				here = path == "" and key or (path .. "." .. key)
			else
				error("a parameter named " .. tostring(key))
			end
			local kind = type(value)
			if kind == "table" then
				walk(value, here, depth + 1)
			elseif kind == "number" then
				if value == math.floor(value) then
					out[#out + 1] = { key = here, value = { Int = value } }
				else
					out[#out + 1] = { key = here, value = { Fixed = value } }
				end
			elseif kind == "boolean" then
				out[#out + 1] = { key = here, value = { Bool = value } }
			elseif kind == "string" then
				out[#out + 1] = { key = here, value = { Text = value } }
			else
				error("parameter " .. here .. " is a " .. kind)
			end
		end
	end
	local ok, why = pcall(walk, tbl, "", 0)
	if not ok then return nil, tostring(why) end
	return out
end

-- The schema's transform (action::Transform) of the game's 4x4 matrix: its
-- basis is elements 1-3, 5-7 and 9-11, its origin 13-15.
function capture.transform(m)
	local function at(i)
		local v = get(m, i)
		if type(v) ~= "number" then error("the matrix has no element " .. i) end
		return v
	end
	return {
		basis = { at(1), at(2), at(3), at(5), at(6), at(7), at(9), at(10), at(11) },
		origin = { x = at(13), y = at(14), z = at(15) },
	}
end

local function module(name)
	local loaded = package and package.loaded and package.loaded["tpf3mp." .. name]
	if loaded then return loaded end
	if ug_require then return ug_require("tpf3mp_1::/scripts/tpf3mp/" .. name .. ".lua") end
	return require("tpf3mp." .. name)
end

-- One construction placed with the construction tool: stations, depots and
-- the rest (tpf3mp_proto action::ConstructionBuild). Returns the action
-- table, or nil and why the room cannot carry it yet.
--
-- The construction's own streets its script makes again wherever it is
-- built. The proposal's street part is what the tool built around it, and
-- travels with it (capture.connection): built without it, a station by a
-- road stood beside the road, its entrance a dead end, and no line could
-- reach it (seen on build 40408).
function capture.construction(proposal)
	local street = get(proposal, "proposal")
	for _, list in ipairs({ "addedNodes", "addedSegments", "removedNodes", "removedSegments",
		"edgeObjectsToAdd" }) do
		if length(street and get(street, list)) == nil then return nil, "a proposal it cannot read" end
	end
	-- Constructions in the way: town buildings the placement clears, which
	-- the replay clears again (gatherBuildings), or a construction replaced
	-- (a module edit), which the room does not carry yet.
	local toRemove = get(proposal, "toRemove")
	local removed = length(toRemove)
	if removed == nil then return nil, "a proposal it cannot read" end
	for i = 1, removed do
		local c = api.engine.getComponent(get(toRemove, i), api.type.ComponentType.CONSTRUCTION)
		if (length(c and get(c, "townBuildings")) or 0) == 0 then
			return nil, "a construction that replaces another"
		end
	end
	local toAdd = get(proposal, "toAdd")
	if length(toAdd) ~= 1 then return nil, "more than one construction at once" end
	local con = get(toAdd, 1)
	local file = get(con, "fileName")
	if type(file) ~= "string" or file == "" then return nil, "a construction of no file" end
	local name = get(con, "name")
	if type(name) ~= "string" or name == "" then return nil, "an unnamed construction" end
	local params = get(con, "params")
	if type(params) ~= "table" then
		local construction = get(con, "construction")
		params = construction and get(construction, "params")
	end
	if type(params) ~= "table" then return nil, "a construction without its parameters" end
	local list, why = capture.params(params)
	if not list then return nil, why end
	local ok, transform = pcall(capture.transform, get(con, "transf"))
	if not ok then return nil, tostring(transform) end
	local connection, whyNot = capture.connection(proposal)
	if connection == nil then return nil, whyNot end
	return { BuildConstruction = { file = file, transform = transform, params = list, name = name,
		connection = connection or nil } }
end

-- The street and track changes a construction tool's proposal makes with its
-- construction (seen on build 40408: a bus station placed by a road rebuilds
-- the road through a new junction and adds an edge from the junction to the
-- station's own street node), as a polyline whose every link names its
-- kind; false when it makes none; nil and why the room cannot carry them.
function capture.connection(proposal)
	local engine = module("engine")
	local ok, part = pcall(engine.fromProposal, proposal, nil, true)
	if not ok then return nil, tostring(part) end
	if part == nil then return false end
	if #part.edges == 0 then return nil, "a construction that removes streets and builds none" end
	part.explicit = true
	local first = part.edges[1]
	if part.network == "Street" then part.street = first.template else part.track = first.template end
	part.style = first.style
	local action, why = module("roads").capture(part, engine.world())
	if not action then return nil, why end
	local build = action.BuildRoad or action.BuildTrack
	return build.polyline
end

-- A street or track tool's build (tpf3mp_proto action::RoadBuild,
-- TrackBuild), read off its proposal by tpf3mp/engine.lua and made an action
-- by tpf3mp/roads.lua. Returns the action table; false for a proposal of
-- nothing (the tool before its first point); or nil and why.
function capture.street(proposal)
	return module("engine").captureBuild(proposal, "Street")
end

function capture.track(proposal)
	return module("engine").captureBuild(proposal, "Track")
end

-- A stop placed on a street or track with the stop tool (tpf3mp_proto
-- action::PlaceStop), read off its proposal by tpf3mp/engine.lua. Returns
-- the action table; false for a proposal of nothing; or nil and why.
function capture.stop(proposal)
	return module("engine").placeStop(proposal)
end

-- The bulldozer's removal (tpf3mp_proto action::Bulldoze), read off its
-- proposal by tpf3mp/engine.lua: a construction, edges, or a stop. Returns the action table; false for a
-- proposal of nothing; or nil and why.
function capture.bulldoze(proposal)
	return module("engine").bulldoze(proposal)
end

-- A proposal's street part in one line, for the log (tpf3mp/engine.lua);
-- "" when it has none.
function capture.describe(proposal)
	return module("engine").describe(proposal)
end

-- ------------------------------------------------------ vehicles and lines
--
-- The vehicle and line windows' commands (api.cmd.make*Cmd, with the
-- arguments the windows give them) as actions: tpf3mp/guard.lua's CARRY.
-- `ctx` names what a command names by entity:
--
--   ctx.vehicle(e), ctx.line(e), ctx.group(e) -> canonical id, or nil
--                                               (tpf3mp/registry.lua)
--   ctx.depot(e) -> { file =, at = { x, y, z } } of the depot's
--                   construction, or nil
--   ctx.model(id) -> a vehicle model's file name, or nil
--
-- Each returns the action table, or raises why the room cannot carry it.

local function named(what, id)
	if id == nil then error(what, 0) end
	return id
end

local function tintOf(v)
	local r, g, b = get(v, "x"), get(v, "y"), get(v, "z")
	if r == nil then r, g, b = get(v, 1), get(v, 2), get(v, 3) end
	if type(r) ~= "number" or type(g) ~= "number" or type(b) ~= "number" then
		error("a colour it cannot read", 0)
	end
	return { r = r, g = g, b = b }
end

local function each(list, fn)
	local out = {}
	for i = 1, (length(list) or 0) do out[i] = fn(get(list, i)) end
	return out
end

local function vehicleOf(ctx, entity)
	return named("a vehicle the room cannot name", ctx.vehicle(entity))
end

local function lineOf(ctx, entity)
	return named("a line the room cannot name", ctx.line(entity))
end

-- The depot's store: a vehicle config (TransportVehicleConfig) bought there.
function capture.vehicleBuy(ctx, _player, depot, config)
	local consist = each(get(config, "vehicles"), function(tvp)
		local part = get(tvp, "part")
		return {
			model = named("a vehicle model the room cannot name", ctx.model(get(part, "modelId"))),
			reversed = get(part, "reversed") == true,
			loads = each(get(part, "compartment2loadConfig"), function(lc)
				return { config = get(lc, "loadConfigIndex"), cargo = get(lc, "cargoTypeId") }
			end),
			color = tintOf(get(part, "color")),
		}
	end)
	return { BuyVehicle = {
		depot = named("a depot the room cannot name", ctx.depot(depot)),
		consist = consist,
		groups = each(get(config, "vehicleGroups"), function(n) return n end),
		multiple_units = each(get(config, "muFileNames"), function(name) return name end),
	} }
end

function capture.vehicleSetLine(ctx, vehicle, line, stopIndex)
	return { AssignLine = {
		vehicles = { vehicleOf(ctx, vehicle) }, line = lineOf(ctx, line), first_stop = stopIndex,
	} }
end

function capture.vehicleSell(ctx, vehicles)
	return { SellVehicle = { vehicles = each(vehicles, function(e) return vehicleOf(ctx, e) end) } }
end

function capture.vehicleStop(ctx, vehicle, stopped)
	return { VehicleOp = { vehicle = vehicleOf(ctx, vehicle), change = { Stop = stopped == true } } }
end

function capture.vehicleToDepot(ctx, vehicle, sell, jumpTo)
	if jumpTo ~= nil then error("moving a vehicle into a depot at once", 0) end
	return { VehicleOp = { vehicle = vehicleOf(ctx, vehicle), change = { ToDepot = { sell = sell == true } } } }
end

function capture.vehicleReverse(ctx, vehicle)
	return { VehicleOp = { vehicle = vehicleOf(ctx, vehicle), change = "Reverse" } }
end

function capture.vehicleDepart(ctx, vehicle)
	return { VehicleOp = { vehicle = vehicleOf(ctx, vehicle), change = "Depart" } }
end

-- The game's load modes (Line.LoadMode), numbers to the schema's names.
local LOAD_MODES = { [0] = "LoadIfAvailable", [1] = "FullLoadAny", [2] = "FullLoadAll", [3] = "LegacyUnloadOnly" }

-- A Line component as the schema's LineData.
function capture.lineData(ctx, line)
	local stops = each(get(line, "stops"), function(s)
		if (length(get(s, "waypoints")) or 0) > 0 then error("a line through waypoints", 0) end
		local config = get(s, "stopConfig")
		local mode = tonumber(get(s, "loadMode"))
		return {
			group = named("a station the room cannot name", ctx.group(get(s, "stationGroup"))),
			terminal = { station = get(s, "station"), terminal = get(s, "terminal") },
			alternatives = each(get(s, "alternativeTerminals"), function(a)
				return { station = get(a, "station"), terminal = get(a, "terminal") }
			end),
			load_mode = LOAD_MODES[mode] or error("a load mode " .. tostring(mode), 0),
			min_wait = get(s, "minWaitingTime"),
			max_wait = get(s, "maxWaitingTime"),
			max_extra_wait = get(s, "maxAdditionalWaitingTime"),
			rules = {
				load = each(get(config, "load"), function(b) return b == true end),
				max_load = each(get(config, "maxLoad"), function(f) return f end),
				force_unload = get(config, "forceUnload") == true,
				destroy_for_config_change = get(config, "destroyForConfigChange") == true,
				destroy_for_refresh = get(config, "destroyForRefresh") == true,
			},
		}
	end)
	local info = get(line, "vehicleInfo")
	local modes, transport = {}, info and get(info, "transportModes")
	if type(transport) ~= "table" then error("a line's transport modes it cannot read", 0) end
	for mode, on in pairs(transport) do
		if on == true then modes[#modes + 1] = mode end
	end
	table.sort(modes)
	return {
		stops = stops,
		modes = modes,
		custom_filters = get(line, "customFilters") == true,
		reservation_priority = get(line, "reservationPriority") or 0,
	}
end

function capture.lineCreate(ctx, name, color, _player, line)
	return { CreateLine = { name = name, color = tintOf(color), line = capture.lineData(ctx, line) } }
end

function capture.lineUpdate(ctx, lineEntity, line)
	return { EditLine = { line = lineOf(ctx, lineEntity), change = { Update = capture.lineData(ctx, line) } } }
end

function capture.lineDestroy(ctx, lineEntity)
	return { EditLine = { line = lineOf(ctx, lineEntity), change = "Delete" } }
end

-- Renaming and recolouring: lines so far.
function capture.setName(ctx, entity, name)
	return { EditLine = { line = named("renaming this", ctx.line(entity)), change = { Rename = name } } }
end

function capture.setColor(ctx, entity, color)
	return { EditLine = { line = named("recolouring this", ctx.line(entity)), change = { Recolor = tintOf(color) } } }
end

return capture

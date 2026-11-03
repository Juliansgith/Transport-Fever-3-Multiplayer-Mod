-- tpf3mp/modevents.lua -- a shared mod's event to its own game script,
-- carried through the room (docs/MODS.md, "Shared mods' events").
--
-- A mod every player runs (a shared mod) often has a GUI half that tells its
-- game script what the player chose: a setting, a toggle, a rule for a line.
-- It sends that as api.cmd.makeScriptingSendEventCmd(src, id, name, param),
-- which in the room's game the GUI's guard refuses: from the GUI it would
-- reach this game's game scripts alone, at an update of this game's own.
--
-- This carries the events listed in ALLOW instead. The GUI's guard makes one
-- a ModEvent action (capture), the room orders it, and every game's
-- TPF3-MP game script sends the same event at the same update in its
-- simulation state (resolve, then apply.lua's HANDLERS.ModEvent), where
-- every game's copy of the mod's game script hears it alike.
--
-- Only listed events go: a mod's game script must decide from what every
-- game has (its own state, the world, the event), and an entity the event
-- names is turned into what the room names it by, and back in every game.
-- An event whose handler reads something each game has its own of (each
-- game's own speed, GAME_SPEED, which the room's pace leaves alone) stays
-- refused: carried, it would decide differently in each game.
--
-- The event's parameter travels as text (encode, decode): plain data only
-- (booleans, finite numbers, strings, tables of them), its keys in one order,
-- every number exactly as it was. So every game hears the same table.
--
-- Pure Lua; the tests call it directly.

local modevents = {}

-- The longest parameter carried, in bytes of its text (the action's
-- ModEvent::param, MAX_MOD_EVENT_PARAM).
modevents.MAX_PARAM = 4096
-- The deepest table nesting a parameter may have.
modevents.MAX_DEPTH = 8

-- ------------------------------------------------------------ the codec
--
-- nil `n`, true `t`, false `f`, a whole number `i<digits>;`, any other
-- finite number `d<%.17g>;`, a string `s<bytes>:<text>`, a table `{`, its
-- entries as key then value, `}`. Whole-number keys come first, from the
-- smallest, then string keys in byte order.

local function isWhole(x)
	return x == math.floor(x) and x >= -2 ^ 53 and x <= 2 ^ 53
end

local function encodeInto(out, value, depth)
	local t = type(value)
	if t == "nil" then
		out[#out + 1] = "n"
	elseif t == "boolean" then
		out[#out + 1] = value and "t" or "f"
	elseif t == "number" then
		if value ~= value or value == math.huge or value == -math.huge then
			error("a number that is not finite", 0)
		end
		if isWhole(value) then
			out[#out + 1] = string.format("i%.0f;", value)
		else
			out[#out + 1] = string.format("d%.17g;", value)
		end
	elseif t == "string" then
		if value:find("[%z\1-\31\127]") then error("a string with a control character", 0) end
		out[#out + 1] = "s" .. #value .. ":" .. value
	elseif t == "table" then
		if depth >= modevents.MAX_DEPTH then error("tables nested too deep", 0) end
		local whole, text = {}, {}
		for k in pairs(value) do
			if type(k) == "number" and isWhole(k) then
				whole[#whole + 1] = k
			elseif type(k) == "string" then
				text[#text + 1] = k
			else
				error("a key of type " .. type(k), 0)
			end
		end
		table.sort(whole)
		table.sort(text)
		out[#out + 1] = "{"
		for _, k in ipairs(whole) do
			encodeInto(out, k, depth + 1)
			encodeInto(out, value[k], depth + 1)
		end
		for _, k in ipairs(text) do
			encodeInto(out, k, depth + 1)
			encodeInto(out, value[k], depth + 1)
		end
		out[#out + 1] = "}"
	else
		error("a value of type " .. t, 0)
	end
end

-- The text of `value`, or an error naming what cannot go.
function modevents.encode(value)
	local out = {}
	encodeInto(out, value, 0)
	local text = table.concat(out)
	if #text > modevents.MAX_PARAM then
		error("a parameter of " .. #text .. " bytes (at most " .. modevents.MAX_PARAM .. ")", 0)
	end
	return text
end

local function decodeAt(text, i, depth)
	local c = text:sub(i, i)
	if c == "n" then return nil, i + 1 end
	if c == "t" then return true, i + 1 end
	if c == "f" then return false, i + 1 end
	if c == "i" or c == "d" then
		local stop = text:find(";", i + 1, true)
		local n = stop and tonumber(text:sub(i + 1, stop - 1))
		if n == nil then error("a number it cannot read at " .. i, 0) end
		return n, stop + 1
	end
	if c == "s" then
		local colon = text:find(":", i + 1, true)
		local len = colon and tonumber(text:sub(i + 1, colon - 1))
		if len == nil or len < 0 or colon + len > #text then error("a string it cannot read at " .. i, 0) end
		return text:sub(colon + 1, colon + len), colon + len + 1
	end
	if c == "{" then
		if depth >= modevents.MAX_DEPTH then error("tables nested too deep", 0) end
		local t = {}
		local j = i + 1
		while text:sub(j, j) ~= "}" do
			if j > #text then error("a table not closed", 0) end
			local k, v
			k, j = decodeAt(text, j, depth + 1)
			if type(k) ~= "number" and type(k) ~= "string" then error("a key it cannot read", 0) end
			v, j = decodeAt(text, j, depth + 1)
			t[k] = v
		end
		return t, j + 1
	end
	error("a value it cannot read at " .. i, 0)
end

-- The value `text` holds (modevents.encode's), or an error.
function modevents.decode(text)
	if type(text) ~= "string" then error("a parameter that is not text", 0) end
	local value, j = decodeAt(text, 1, 0)
	if j ~= #text + 1 then error("text after the parameter", 0) end
	return value
end

-- ------------------------------------------------------------ the events

-- A shallow copy of a table.
local function copy(t)
	if type(t) ~= "table" then return t end
	local c = {}
	for k, v in pairs(t) do c[k] = v end
	return c
end

local function needTable(param)
	if type(param) ~= "table" then error("a parameter that is not a table", 0) end
	return copy(param)
end

-- Plain events: what the GUI sent is what every game hears.
local function plain() return { out = function(_ctx, param) return param end, into = function(_env, param) return param end } end

-- Real Clock (Realistic Environment Pro's service hours): a line's rule
-- names the line, and its home depot (the depot's VEHICLE_DEPOT entity), or
-- none for the nearest; a vehicle's mode names the vehicle. Each by what the
-- room names it: a line and a vehicle by canonical id, the acting company's
-- own (as a window's commands for them), a depot by its construction and
-- its index among that construction's depots (capture.depotRef).
local lineRule = {
	out = function(ctx, param)
		local p = needTable(param)
		local line = tonumber(p.line)
		local id = line and ctx.line and ctx.line(line)
		if id == nil then error("a line the room cannot name", 0) end
		if ctx.owned and not ctx.owned(line) then error("another company's line", 0) end
		p.line = id
		if type(p.rule) == "table" and p.rule.depot ~= nil then
			local rule = copy(p.rule)
			local ref, index, why = nil, nil, "no depot reader"
			if ctx.depot then ref, index, why = ctx.depot(tonumber(rule.depot)) end
			if ref == nil then error("a home depot the room cannot name: " .. tostring(why), 0) end
			rule.depot = { file = ref.file, at = ref.at, index = index or 0 }
			p.rule = rule
		end
		return p
	end,
	into = function(env, param)
		local p = needTable(param)
		p.line = env.own("lines", p.line)
		if type(p.rule) == "table" and type(p.rule.depot) == "table" then
			local rule = copy(p.rule)
			rule.depot = env.depot({ file = rule.depot.file, at = rule.depot.at }, rule.depot.index)
			p.rule = rule
		end
		return p
	end,
}
local vehicleMode = {
	out = function(ctx, param)
		local p = needTable(param)
		local vehicle = tonumber(p.vehicle)
		local id = vehicle and ctx.vehicle and ctx.vehicle(vehicle)
		if id == nil then error("a vehicle the room cannot name", 0) end
		if ctx.owned and not ctx.owned(vehicle) then error("another company's vehicle", 0) end
		p.vehicle = id
		return p
	end,
	into = function(env, param)
		local p = needTable(param)
		p.vehicle = env.own("vehicles", p.vehicle)
		return p
	end,
}

-- The events carried, by id, then name: the mod (its modId) whose game
-- script hears them, and how what they name is turned into the room's names
-- (out, in the GUI) and back (into, in every game).
modevents.ALLOW = {
	-- Realistic Environment Pro (mod.io 6415745), jmw/weather.script.lua's
	-- handleEvent. Left out: ManualWeather and ManualWind, whose delays are
	-- counted in each game's own speed (weather.script.lua:380, :766).
	Neige = {
		SetClimate = { mod = "just_more_weather_1", spec = plain() },
		SetMist = { mod = "just_more_weather_1", spec = plain() },
	},
	-- Its service hours, rct/service.lua's S.applyEvent.
	RealClockService = {
		["rct.setLineRule"] = { mod = "just_more_weather_1", spec = lineRule },
		["rct.setVehicleMode"] = { mod = "just_more_weather_1", spec = vehicleMode },
		["rct.setLife"] = { mod = "just_more_weather_1", spec = plain() },
	},
	-- Real Cant Engineering (mod.io 6440851), cayde_devers/sync.script.lua's
	-- handleEvent: the track menu's Cant toggle and manual cant, for the
	-- whole room (the mod keeps one of each).
	cayde_cant = {
		["cayde_cant.toggle"] = { mod = "cayde_devers_1", spec = plain() },
		["cayde_cant.manual"] = { mod = "cayde_devers_1", spec = plain() },
		-- The manual cant of new segments, by their ends' places (gkey);
		-- sent from its game script's GUI half, by the room's deciding game
		-- (tpf3mp/upkeep.lua).
		["cayde_cant.override"] = { mod = "cayde_devers_1", spec = plain() },
	},
}

-- The entry for an event, or nil.
function modevents.entry(id, name)
	local byName = type(id) == "string" and modevents.ALLOW[id]
	return byName and type(name) == "string" and byName[name] or nil
end

-- The action carrying the event, or an error naming why it cannot go.
-- `ctx` names what the event names (tpf3mp/capture.lua's context; owned(e):
-- whether the player's company may act on e).
function modevents.capture(ctx, src, id, name, param)
	local entry = modevents.entry(id, name)
	if entry == nil then error("the " .. tostring(id) .. " script's " .. tostring(name) .. " event", 0) end
	if type(src) ~= "string" then error("an event from a source that is not text", 0) end
	local wire = entry.spec.out(ctx or {}, param)
	return { ModEvent = { mod_id = entry.mod, src = src, id = id, name = name, param = modevents.encode(wire) } }
end

-- The event as every game sends it: src, id, name and the parameter with
-- what it names found in this game. `env`:
--   own(kind, id)        -> the acting company's entity of kind ("lines",
--                           "vehicles") with canonical id, or an error;
--   depot(ref, index)    -> the depot entity, or an error.
function modevents.resolve(env, e)
	if type(e) ~= "table" then error("a mod's event it cannot read", 0) end
	local entry = modevents.entry(e.id, e.name)
	if entry == nil then error("the " .. tostring(e.id) .. " script's " .. tostring(e.name) .. " event is not carried", 0) end
	if entry.mod ~= e.mod_id then error("the event of " .. tostring(e.mod_id) .. ", not " .. entry.mod, 0) end
	local param = entry.spec.into(env, modevents.decode(e.param))
	return e.src, e.id, e.name, param
end

-- Kinds a mod's own loops send by themselves (Realistic Environment Pro's
-- clock sets the time of day each second, its date at midnight): refused
-- like any other, in the hook's log, but without telling the player each
-- time, who did nothing.
modevents.QUIET = {
	makeGameSetTimeOfDayCmd = true,
	makeGameSetDateCmd = true,
}

-- Whether a refusal of `kind` from the mod `from` is told the player.
function modevents.quiet(kind, from)
	if from == nil then return false end
	return kind == nil or modevents.QUIET[kind] == true
end

return modevents

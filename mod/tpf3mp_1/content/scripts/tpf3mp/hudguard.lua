-- tpf3mp/hudguard.lua -- the room's guard in the GUI's other Lua state.
--
-- The game's GUI runs in more than one Lua state (build 40408): the
-- Multiplayer plugin's, where tpf3mp/guard.lua stands in front of
-- api.cmd.sendCommand, and the one the game renders its React recipes in
-- (the HUD's icons, the line manager's depots; gui/tpf3mp/gui_state.script.lua),
-- whose api.cmd is its own. A command a window sends from there went
-- unchecked, and in the room's game ran in this game alone: a hidden desync
-- (docs/COVERAGE.md, "The gates and their holes", U1). This puts the same
-- guard there, with the same rules: a kind the room carries goes to the
-- room, every other is refused.
--
-- One difference: the hook hands what became of the player's actions to one
-- reader (results()), the plugin's state. That state passes on the answers
-- to this state's commands through a note every Lua state shares
-- (hudguard.forward), and this state's guard hears them as the plugin's does
-- (guard.deliver): the store's buy here is answered with the vehicle it
-- bought, which it then puts on its line (2026-10-01: the vehicle store
-- renders here, and refusing its buy blocked buying).
--
-- This state has no frame of its own the mod runs in, so the callbacks the
-- guard defers ("on the next frame") run from tick(), which the state's
-- GUI calls often (each read of the player's company); each waits until the
-- clock moved on twice, so none runs inside the call that sent it.
--
-- Pure Lua; the tests hand it a fake api and link.

local hudguard = {}

local function module(name)
	local key = "tpf3mp." .. name
	local loaded = package and package.loaded and package.loaded[key]
	if loaded then return loaded end
	local m
	if ug_require then
		m = ug_require("tpf3mp_1::/scripts/tpf3mp/" .. name .. ".lua")
	else
		m = require(key)
	end
	-- The guard and the capture find each other through require.
	if package and package.loaded and m ~= nil then package.loaded[key] = m end
	return m
end

-- What the guard names things by in this state (tpf3mp/capture.lua's ctx),
-- read from the mod's game script's state as the plugin's state reads it.
function hudguard.context(api)
	local companies = module("companies")
	local registry = module("registry")
	local function state() return companies.scriptState(api) end
	local function idOf(kind)
		return function(entity)
			local s = state()
			return registry.id(s and s.registry, kind, entity)
		end
	end
	return {
		vehicle = idOf("vehicles"),
		line = idOf("lines"),
		group = idOf("groups"),
		town = idOf("towns"),
		company = function(entity)
			local s = state()
			local roster = s and s.companies
			for _, c in ipairs(roster and roster.list or {}) do
				if c.entity == entity then return c.id end
			end
			return nil
		end,
		player = function()
			local ok, player = pcall(function() return api.engine.util.getPlayer() end)
			if ok and type(player) == "number" then return player end
			return nil
		end,
		depot = function(depot) return module("capture").depotRef(api, depot) end,
		model = function(id)
			local ok, name = pcall(function() return api.res.modelRep.getName(id) end)
			if ok and type(name) == "string" and name ~= "" then return name end
			return nil
		end,
		subsidy = function(uid)
			local where, s = companies.findSubsidy(companies.subsidyState(api), uid)
			if where == "offered" and type(s.id) == "string" then return s.id end
			return nil
		end,
		parts = function(vehicle)
			local ok, parts = pcall(function()
				local tv = api.engine.getComponent(vehicle, api.type.ComponentType.TRANSPORT_VEHICLE)
				local list = tv.transportVehicleConfig.vehicles
				local out = {}
				for i = 1, #list do
					out[i] = { model = list[i].part.modelId, purchased = list[i].purchaseTime }
				end
				return out
			end)
			if ok then return parts end
			return nil
		end,
	}
end

-- The callbacks the guard deferred, each with the tick it may run from.
local pending = {}
local ticks, lastClock = 0, nil
-- This state's guard, once installed: { cmd =, link =, sees = }.
local installed = nil
-- The tickets of this state's commands handed to the room whose answers it
-- waits for, in order.
local tickets = {}

-- The notes the two GUI states pass the room's answers through
-- (tpf3mp_native.note, which every Lua state of the game shares): this
-- state's waiting tickets, and the answers the plugin's state forwards.
hudguard.TICKETS = "tpf3mp.hud.tickets"
hudguard.ANSWERS = "tpf3mp.hud.answers"
-- The longest note the hook keeps (crates/tpf3mp-hook/src/lua.rs).
hudguard.NOTE_MAX = 512

local function clock()
	local ok, t = pcall(function() return os.clock() end)
	if ok and type(t) == "number" then return t end
	return nil
end

local function wallClock()
	local ok, t = pcall(function() return os.time() end)
	if ok and type(t) == "number" then return t end
	return nil
end

local function parseTickets(text)
	local set = {}
	for t in tostring(text or ""):gmatch("%d+") do set[tonumber(t)] = true end
	return set
end

local function writeTickets(link)
	local list = {}
	for i, t in ipairs(tickets) do list[i] = string.format("%d", t) end
	local text = table.concat(list, ",")
	-- The hook keeps no longer note: the oldest waits are given up.
	while #text > hudguard.NOTE_MAX and #tickets > 0 do
		link:log("the HUD's state waits on too many answers: ticket " .. string.format("%d", tickets[1])
			.. "'s window is not told")
		table.remove(tickets, 1)
		table.remove(list, 1)
		text = table.concat(list, ",")
	end
	link:note(hudguard.TICKETS, text)
end

-- In the plugin's state, which alone reads the room's answers
-- (link:results()): passes on to this state the answers to its commands,
-- as "ticket ok entity;" in a note. Returns how many it passed on.
function hudguard.forward(link, results)
	if type(results) ~= "table" or #results == 0 then return 0 end
	local wanted = parseTickets(link:note(hudguard.TICKETS))
	local lines = {}
	for _, r in ipairs(results) do
		if type(r) == "table" and type(r.ticket) == "number" and wanted[r.ticket] then
			lines[#lines + 1] = string.format("%d %d %s", r.ticket, r.ok == true and 1 or 0,
				type(r.entity) == "number" and string.format("%d", r.entity) or "-")
		end
	end
	if #lines == 0 then return 0 end
	local text = (link:note(hudguard.ANSWERS) or "") .. table.concat(lines, ";") .. ";"
	if #text > hudguard.NOTE_MAX then
		link:log("the HUD's state's answers do not fit the note (" .. #text .. " bytes): "
			.. #lines .. " answer(s) lost, their windows are not told")
		return 0
	end
	link:note(hudguard.ANSWERS, text)
	return #lines
end

-- The answers the plugin's state passed on, taken once.
local function takeAnswers(link)
	local text = link:note(hudguard.ANSWERS)
	if text == nil or text == "" then return {} end
	link:note(hudguard.ANSWERS, "")
	local out = {}
	for ticket, ok, entity in text:gmatch("(%d+) (%d) ([%-%d]+);") do
		out[#out + 1] = { ticket = tonumber(ticket), ok = ok == "1", entity = tonumber(entity) }
	end
	return out
end

-- Runs the deferred callbacks that are due, and hears the room's answers to
-- this state's commands (guard.deliver, as the plugin's state does). Called
-- often; counts a tick whenever the clock moved on (each call counts where
-- there is none). Returns how many callbacks ran.
local ticking = false
function hudguard.tick()
	-- Not again from inside a callback it runs (a window's callback may send
	-- a command, whose capture asks whose company it is).
	if ticking then return 0 end
	ticking = true
	local ok, ran = pcall(function() return hudguard.tickOnce() end)
	ticking = false
	if ok then return ran end
	return 0
end

function hudguard.tickOnce()
	local now = clock()
	if now == nil or now ~= lastClock then
		ticks = ticks + 1
		lastClock = now
	end
	local ran = 0
	if installed then
		local answers = {}
		if #tickets > 0 then
			answers = takeAnswers(installed.link)
			if #answers > 0 then
				local gone = {}
				for _, a in ipairs(answers) do gone[a.ticket] = true end
				local keep = {}
				for _, t in ipairs(tickets) do
					if not gone[t] then keep[#keep + 1] = t end
				end
				tickets = keep
				writeTickets(installed.link)
			end
		end
		local ok, heard = pcall(module("guard").deliver, installed.cmd, answers, installed.sees, wallClock)
		if ok and type(heard) == "number" then ran = ran + heard end
	end
	if #pending == 0 then return ran end
	local due, keep = {}, {}
	for _, p in ipairs(pending) do
		if p.at <= ticks then due[#due + 1] = p.fn else keep[#keep + 1] = p end
	end
	pending = keep
	for _, fn in ipairs(due) do pcall(fn) end
	return ran + #due
end

-- Puts the guard in front of `cmd` (this state's api.cmd), linked through
-- `link` (tpf3mp/bridge.lua). `where` names the state in hook.log. Returns
-- the number of factories wrapped, or nil and why.
--
-- A command handed to the room keeps its ticket here, noted for the
-- plugin's state, which passes its answer on (hudguard.forward); tick()
-- hears it, and the window's callback gets what the action made once this
-- state's world and registry have it, as in the plugin's state. Only with a
-- hook without notes are answers not routed: there a command is answered as
-- sent, and one whose window waits on what it made is refused (guard.UNTOLD).
function hudguard.install(cmd, link, api, where)
	local guard = module("guard")
	module("capture")
	local refusals = {}
	local label = where or "the HUD's state"
	local routed = type(link.native) == "table" and type(link.native.note) == "function"
	local context = hudguard.context(api)
	local named = { vehicles = context.vehicle, lines = context.line }
	local function sees(entity, kind)
		local ok, there = pcall(function() return api.engine.entityExists(entity) end)
		if ok and there ~= true then return false end
		if kind == nil then return true end
		local name = named[kind]
		local okId, id = pcall(function() return name and name(entity) end)
		return okId and id ~= nil
	end
	local wrapped, why = guard.install(cmd, {
		inRoom = function() return link:room() end,
		command = function(action)
			local ok, ticket = link:command(action)
			if not ok then return nil, ticket end
			if routed and type(ticket) == "number" then
				tickets[#tickets + 1] = ticket
				writeTickets(link)
				return true, ticket
			end
			return true, nil
		end,
		refused = function(kind, reason, from)
			local name = kind or "command no factory made"
			local count = (refusals[name] or 0) + 1
			refusals[name] = count
			if count == 1 or count % 100 == 0 then
				link:log("refused the player's " .. name .. " in the room's game (" .. count .. " so far) in "
					.. label .. (reason and (": " .. tostring(reason)) or "")
					.. (from and (", from the mod " .. tostring(from)) or ""))
			end
		end,
		later = function(fn) pending[#pending + 1] = { fn = fn, at = ticks + 2 } end,
		context = context,
		personal = function(mod) return link:personal()[mod] == true end,
		untold = not routed,
	})
	if wrapped then
		installed = { cmd = cmd, link = link, sees = sees }
		link:log("the guard is on " .. wrapped .. " command factories in " .. label
			.. (routed and "" or "; the room's answers cannot reach it, so a window there that waits on what it made is refused"))
	else
		link:log("the guard is not on in " .. label .. ": " .. tostring(why)
			.. "; the player's commands there are not checked")
	end
	return wrapped, why
end

return hudguard

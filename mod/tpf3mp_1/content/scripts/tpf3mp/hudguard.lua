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
-- reader (results()), the plugin's state, which answers the windows waiting
-- there. So here a command handed to the room is answered as sent, and a
-- command whose window waits on what it made (guard.RESULT: a vehicle
-- bought, a line made, a replacement), sent with a callback, is refused with
-- why (guard.UNTOLD), so hook.log says if a window here sends one.
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

local function clock()
	local ok, t = pcall(function() return os.clock() end)
	if ok and type(t) == "number" then return t end
	return nil
end

-- Runs the deferred callbacks that are due. Called often; counts a tick
-- whenever the clock moved on (each call counts where there is none).
-- Returns how many ran.
function hudguard.tick()
	local now = clock()
	if now == nil or now ~= lastClock then
		ticks = ticks + 1
		lastClock = now
	end
	if #pending == 0 then return 0 end
	local due, keep = {}, {}
	for _, p in ipairs(pending) do
		if p.at <= ticks then due[#due + 1] = p.fn else keep[#keep + 1] = p end
	end
	pending = keep
	for _, fn in ipairs(due) do pcall(fn) end
	return #due
end

-- Puts the guard in front of `cmd` (this state's api.cmd), linked through
-- `link` (tpf3mp/bridge.lua). `where` names the state in hook.log. Returns
-- the number of factories wrapped, or nil and why.
function hudguard.install(cmd, link, api, where)
	local guard = module("guard")
	module("capture")
	local refusals = {}
	local label = where or "the HUD's state"
	local wrapped, why = guard.install(cmd, {
		inRoom = function() return link:room() end,
		-- Handed over, answered as sent: no ticket, as results() are the
		-- plugin's state's to read.
		command = function(action)
			local ok, reason = link:command(action)
			if ok then return true, nil end
			return nil, reason
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
		context = hudguard.context(api),
		personal = function(mod) return link:personal()[mod] == true end,
		untold = true,
	})
	if wrapped then
		link:log("the guard is on " .. wrapped .. " command factories in " .. label)
	else
		link:log("the guard is not on in " .. label .. ": " .. tostring(why)
			.. "; the player's commands there are not checked")
	end
	return wrapped, why
end

return hudguard

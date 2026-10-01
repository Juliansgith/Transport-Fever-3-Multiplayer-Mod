-- TPF3-MP in the game's GUI state: the plugin gui/tpf3mp/tpf3mp.res.lua
-- names. On the first step of a game it loads the mod's modules and links
-- to the hook (tpf3mp/bridge.lua). Without a hook, which is every game Steam
-- started, it logs one line and does nothing more. The room's actions are
-- applied by the mod's game script (tpf3mp_sim/), not here.
--
-- Every frame it does what the hook asks (docs/HOOKS.md, "The room's
-- world"): save the world under a name when the room orders a save, or load
-- the room's world from the game's save folder. It tells the hook each time
-- a world's GUI starts, which is how the hook sees a load finish.
--
-- Once linked, it puts the guard in front of the GUI's commands
-- (tpf3mp/guard.lua; docs/HOOKS.md, "The player's commands"): in the room's
-- game a command the room carries goes to the room, and its window hears
-- what became of it once this game has applied it; a command the room
-- cannot carry yet is refused, and the game bar says so for a few seconds.
-- The guard names vehicles, lines, station groups and towns by the canonical
-- ids the mod's game script keeps in its state (tpf3mp/registry.lua).
--
-- It follows what mods made for Transport Fever 3 build 40391 rely on
-- (investigation/TF3_MODS_2026-09-27.md): a .script.lua defines data();
-- ug_require loads the game's files ("::/...") and a mod's own
-- ("tpf3mp_1::/..."); a GameBarInfoDisplayExtension plugin with
-- react.onStep runs code every frame in a game; debugPrint writes to the
-- game's log. Each step logs "[tpf3mp]" lines, so the log shows how far a
-- game got on release day.
--
-- In the room's game the game bar also shows the room in one line (its
-- name, who is there, its speed, whether the worlds match), a button that
-- opens the Multiplayer window: the room, its players, its speed, whether
-- this world matches the room's, and the room's chat, which the player can
-- write to (docs/PLAN.md: the in-game Multiplayer panel for a game the
-- launcher started; the lobby stays in the launcher). The game's window
-- container shows the window, and the game's area for mods' buttons has a
-- second button for it (a second plugin, tpf3mp_button.res.lua). What
-- they show is kept in package.loaded["tpf3mp.ui"], which the game bar
-- plugin fills every frame from the hook (tpf3mp/bridge.lua: status, chat,
-- say).
function data()
	local MOD = "tpf3mp_1"
	-- Every module, in an order where each needs only those before it.
	local MODULES = { "banners", "geom", "roads", "engine", "registry", "companies", "progression", "follow", "capture",
		"bridge", "guard", "worldload" }
	-- Frames a refusal's notice stays in the game bar.
	local NOTICE_FRAMES = 360

	local function say(line)
		pcall(debugPrint, "[tpf3mp] " .. line)
	end

	-- The link to the hook, once a world's GUI has found it.
	local link = nil

	-- What the Multiplayer window and the game bar show, shared by both
	-- plugins: the room (link:status()), the chat so far, whether the window
	-- is open, and a count that goes up whenever any of it changes; and the
	-- link, which the game bar plugin makes: the game may run this file
	-- once for each plugin, each with its own locals.
	local function ui()
		local shared = package.loaded["tpf3mp.ui"]
		if type(shared) ~= "table" then
			shared = { open = false, status = nil, lines = {}, unread = 0, version = 0 }
			package.loaded["tpf3mp.ui"] = shared
		end
		return shared
	end

	-- Callbacks of refused commands, for the next frame, as the game would
	-- call them.
	local pending = {}
	-- The notice of the last refusal, until the plugin shows it.
	local notice = nil
	-- Refusals so far, by kind, and the last reason logged of each, for the
	-- hook's log.
	local refusals, reasons = {}, {}

	local function refused(kind, why, from)
		local name = kind or "command no factory made"
		local count = (refusals[name] or 0) + 1
		refusals[name] = count
		local changed = why ~= nil and why ~= reasons[name]
		if changed then reasons[name] = why end
		if count == 1 or count % 100 == 0 or changed then
			link:log("refused the player's " .. name .. " in the room's game ("
				.. count .. " so far)" .. (why and (": " .. tostring(why)) or "")
				.. (from and (", from the mod " .. tostring(from)) or ""))
		end
		notice = require("tpf3mp.guard").notice(kind)
	end

	local function runPending()
		if #pending == 0 then return end
		local due = pending
		pending = {}
		for _, fn in ipairs(due) do
			local ok, err = pcall(fn)
			if not ok then say("a refused command's callback failed: " .. tostring(err)) end
		end
	end

	-- The mod's game script's state as the game keeps it
	-- (tpf3mp/companies.lua), which holds its registry (tpf3mp/registry.lua)
	-- and its companies. Read once the modules are loaded.
	local function scriptState()
		return require("tpf3mp.companies").scriptState(api)
	end
	local function registryNow()
		local state = scriptState()
		return state and state.registry
	end

	-- What the guard names things by (tpf3mp/capture.lua).
	local function idOf(kind)
		return function(entity)
			return require("tpf3mp.registry").id(registryNow(), kind, entity)
		end
	end
	local context = {
		vehicle = idOf("vehicles"),
		line = idOf("lines"),
		group = idOf("groups"),
		town = idOf("towns"),
		-- The room's company whose player entity `entity` is, by its id: the
		-- game's company window renames the player's company so
		-- (game_mechanics/company/company.tl, its editable title).
		company = function(entity)
			local roster = ui().companies
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
		depot = function(depot)
			local c
			pcall(function()
				local con = api.engine.system.streetConnectorSystem.getConstructionEntityForDepot(depot)
				c = con and api.engine.getComponent(con, api.type.ComponentType.CONSTRUCTION)
			end)
			if c == nil then return nil end
			local t = c.transf
			return { file = c.fileName, at = { x = t[13], y = t[14], z = t[15] } }
		end,
		model = function(id)
			local ok, name = pcall(function() return api.res.modelRep.getName(id) end)
			if ok and type(name) == "string" and name ~= "" then return name end
			return nil
		end,
		-- The kind of the subsidy offered under `uid`, as this game's subsidy
		-- script has it (tpf3mp/companies.lua), or nil when it offers none.
		subsidy = function(uid)
			local companies = require("tpf3mp.companies")
			local where, s = companies.findSubsidy(companies.subsidyState(api), uid)
			if where == "offered" and type(s.id) == "string" then return s.id end
			return nil
		end,
		-- A vehicle's parts as its TRANSPORT_VEHICLE component has them.
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

	-- The GUI state's api.cmd, which the guard is on.
	local guardedCmd = nil
	-- What the player asked of the commands handed to the room whose
	-- refusal they are told, by ticket: a subsidy's answer.
	local answers = {}

	-- Whether the GUI's world has `entity` yet: what the room's action made
	-- in the simulation reaches it a moment later. With `kind`, also whether
	-- the registry names it yet (tpf3mp/guard.lua, NAMED): the game script's
	-- state, which binds its id, reaches the GUI later still.
	local function sees(entity, kind)
		local ok, there = pcall(function() return api.engine.entityExists(entity) end)
		if ok and there ~= true then return false end
		if kind == nil then return true end
		local okId, id = pcall(function() return idOf(kind)(entity) end)
		return okId and id ~= nil
	end

	-- Seconds by the wall clock, for how long an answer waits on what its
	-- command made (tpf3mp/guard.lua, HOLD_SECONDS); nil without one.
	local function clock()
		local ok, t = pcall(function() return os.time() end)
		if ok and type(t) == "number" then return t end
		return nil
	end

	-- Puts the guard in front of the GUI's commands.
	local function guardCommands()
		local ok, cmd = pcall(function() return api.cmd end)
		guardedCmd = ok and cmd or nil
		local wrapped, why = require("tpf3mp.guard").install(guardedCmd, {
			inRoom = function() return link:room() end,
			command = function(action)
				local ok, ticket = link:command(action)
				-- A subsidy's answer the room refuses is told the player
				-- (taken by another company first, no longer offered).
				local subsidy = ok and type(action) == "table" and action.Subsidy
				if subsidy and ticket ~= nil then
					answers[ticket] = subsidy.Accept and "Taking the subsidy" or "Declining the subsidy"
				end
				return ok, ticket
			end,
			refused = refused,
			later = function(fn) pending[#pending + 1] = fn end,
			context = context,
			personal = function(mod) return link:personal()[mod] == true end,
		})
		if wrapped then
			link:log("the guard is on " .. wrapped .. " command factories")
		else
			link:log("the guard is not on: " .. tostring(why)
				.. "; the player's commands are not checked")
		end
	end

	-- The modules name each other `require "tpf3mp.<name>"`, as TPF2's
	-- did. The game's GUI state has `require` and package.loaded but no
	-- package.preload (build 40408's dump,
	-- investigation/dayone-2026-09-29/probe/script_api_dump_gui.txt), so
	-- each module is loaded here through ug_require, in order, into
	-- package.loaded, where the others' `require` finds it. Only names
	-- under "tpf3mp." are added, so nothing else in the state changes.
	local function installModules()
		if type(package) ~= "table" or type(package.loaded) ~= "table" then
			return nil, "this Lua state has no package.loaded"
		end
		for _, name in ipairs(MODULES) do
			local key = "tpf3mp." .. name
			if package.loaded[key] == nil then
				local path = MOD .. "::/scripts/tpf3mp/" .. name .. ".lua"
				local ok, module = pcall(ug_require, path)
				if not ok or module == nil then
					return nil, key .. " did not load: " .. tostring(module)
				end
				package.loaded[key] = module
			end
		end
		return true
	end

	-- The player entity of the company this player plays for, in the room's
	-- game (tpf3mp/companies.lua), or nil: outside the room, before the
	-- roster is read, and for the room's first company, which is the save's
	-- own player anyway.
	local function myCompany()
		local shared = ui()
		local status = shared.status
		return require("tpf3mp.follow").companyOf(shared.companies, status and status.me_id)
	end

	-- The GUI's "my company" in this Lua state (tpf3mp/follow.lua).
	local function followMyCompany()
		local ok, why = require("tpf3mp.follow").install(api, myCompany)
		link:log(ok and "the GUI's company follows the player's"
			or ("the GUI's company cannot follow the player's: " .. tostring(why)))
	end

	-- The company window's ranks and the permits they give, with more than
	-- one company in the room: each company's own, as the room's rule keeps
	-- them (tpf3mp/progression.lua), read from the mod's game script's
	-- state. With one company the game's own.
	-- The game's permit counts, as the construction menu and tool read them,
	-- count the player's company's own constructions while the room has
	-- more than one company (tpf3mp/companies.lua, followPermits): each its
	-- own headquarters.
	local function countOwnPermits()
		local ok, why = require("tpf3mp.companies").followPermits(api, ug_require, function()
			local roster = ui().companies
			return roster ~= nil and #(roster.list or {}) > 1
		end)
		link:log(ok and ("the game's permits count each company's own constructions (" .. ok .. " company_util table(s))")
			or ("the game's permits count the whole world's constructions: " .. tostring(why)))
	end

	local function showRanks()
		local ok, why = require("tpf3mp.progression").follow(scriptState)
		link:log(ok and "the company window shows each company's own rank"
			or ("the company window shows the game's own rank only: " .. tostring(why)))
	end

	-- Whether this player's company may have its lines stop at `entity`, a
	-- station group or a station's construction another company owns: while
	-- that company keeps its stations open (tpf3mp/companies.lua, mayUse;
	-- DECISIONS.md, D22, proposed). Every game checks the line again when
	-- the room orders it; this only lets the line manager offer the station.
	local function openToMe(entity)
		local shared = ui()
		local roster = shared.companies
		if not (shared.status and roster and link and link:room()) then return false end
		local ok, open = pcall(function()
			local CT = api.type.ComponentType
			local group = api.engine.getComponent(entity, CT.STATION_GROUP)
			local con = group == nil and api.engine.getComponent(entity, CT.CONSTRUCTION) or nil
			if group == nil and not (con and con.stations and #con.stations > 0) then return false end
			local owned = api.engine.getComponent(entity, CT.PLAYER_OWNED)
			local owner = owned and owned.player
			-- The player's company, by the room's roster: whether the
			-- owner lets it stop there (its head's choice, else its default).
			local companies = require("tpf3mp.companies")
			local mine = 0
			for _, m in ipairs(roster.members or {}) do
				if m.player == shared.status.me_id then mine = m.company end
			end
			for _, c in ipairs(roster.list or {}) do
				if c.entity == owner then return companies.lets(c, mine) end
			end
			return false
		end)
		return ok and open == true
	end

	-- TF3's line manager offers only the player's own stations and those no
	-- one owns (gui/line_vehicle_mgmt/manager_window.tl asks
	-- scripts/entity_util.tl's isOwnedByPlayerOrNotOwned; seen on build
	-- 40408); the game itself stops a line anywhere. In this GUI state the
	-- mod's answer also takes another company's open stations. Other windows
	-- ask the same function of vehicles and warehouses, which stay as the
	-- game answers. Whether every script shares one entity_util table, as
	-- one ug_require's cache would give, is INFERRED: hook.log says how many
	-- the mod changed.
	local function offerOpenStations()
		local changed, tried = 0, {}
		for _, path in ipairs({ "/scripts/entity_util.tl", "::/scripts/entity_util.tl" }) do
			local ok, util = pcall(ug_require, path)
			if ok and type(util) == "table" and not tried[util]
				and type(util.isOwnedByPlayerOrNotOwned) == "function" then
				tried[util] = true
				local original = util.isOwnedByPlayerOrNotOwned
				util.isOwnedByPlayerOrNotOwned = function(entity, ...)
					if original(entity, ...) then return true end
					return openToMe(entity)
				end
				changed = changed + 1
			end
		end
		link:log(changed > 0 and ("the line manager offers other companies' open stations ("
			.. changed .. " entity_util table(s))")
			or "the line manager offers the player's own stations only: no entity_util.isOwnedByPlayerOrNotOwned")
	end

	local function start()
		local ok, why = installModules()
		if not ok then
			say("not started: " .. why)
			return
		end
		say("modules loaded")

		local bridge = require "tpf3mp.bridge"
		local found, reason = bridge.attach(bridge.find())
		if not found then
			say(reason .. "; this is the plain game")
			return
		end
		link = found
		ui().link = link
		link:world()
		link:log("the GUI is linked")
		say("linked to the hook")
		guardCommands()
		followMyCompany()
		showRanks()
		countOwnPermits()
		offerOpenStations()
		-- The stop the construction menu gives the stop tool, wherever the
		-- menu runs (gui/tpf3mp/gui_state.script.lua watches the other state).
		pcall(function()
			require("tpf3mp.capture").watchStopTool(ug_require "::/gui/construction/construction_react_util.tl", link)
		end)
	end

	-- The room's world being loaded (tpf3mp/worldload.lua), a frame at a
	-- time while the game reads the save's mods.
	local loading

	-- Does what the hook asks: saving the world under the name it gives, or
	-- loading the room's world from the game's save folder, with the room's
	-- mods (docs/MODS.md).
	local function serve()
		if not link then return end
		local request = link:poll()
		if request and request.save then
			local name = request.save
			local ok, err = pcall(app.saveGame, name, function()
				link:saved(name, true)
			end, false, true)
			if not ok then link:saved(name, false, tostring(err)) end
		elseif request and request.load then
			loading = require("tpf3mp.worldload").new(request.load)
		end
		if loading then
			local done, why = require("tpf3mp.worldload").step(loading, app, api, link)
			if done == "busy" then return end
			loading = nil
			if done == "started" then
				link:log("loading the room's world")
			else
				link:log("loading the room's world failed: " .. tostring(why))
			end
		end
	end

	local readCompanies
	local react = ug_require "::/gui/main/react.lua"
	local builtin = ug_require "::/gui/main/builtin.lua"
	local game_bar_widgets = ug_require "::/gui/game_bar/game_bar_widgets.tl"
	local main_mod_button_area = ug_require "::/gui/main/main_mod_button_area.tl"
	local game_react_globals = ug_require "::/gui/main/game_react_globals.tl"

	-- Chat lines kept, newest last, and how many of them the window shows.
	local CHAT_LINES = 50
	local CHAT_SHOWN = 12
	-- Frames between two readings of the room.
	local STATUS_FRAMES = 15

	-- The room's speed as the speed row says it.
	local function speedText(speed)
		if speed == nil then return "" end
		if speed == 0 then return "paused" end
		return string.format("%gx", speed / 100)
	end

	-- The room in one line, for the game bar.
	local function summary(status)
		local here, all = 0, 0
		for _, p in ipairs(status.players or {}) do
			all = all + 1
			if p.connected then here = here + 1 end
		end
		local parts = { "Multiplayer: " .. tostring(status.room), here .. "/" .. all .. " playing" }
		if status.speed then parts[#parts + 1] = speedText(status.speed) end
		if status.diverged then parts[#parts + 1] = "resyncing" end
		return table.concat(parts, " · ")
	end

	-- The room's companies as the game script keeps them (tpf3mp/companies.lua),
	-- each with its money now, and a text that changes when anything shown
	-- does. Nil before the room's first company exists.
	readCompanies = function()
		local state = scriptState()
		local roster = state and state.companies
		if type(roster) ~= "table" or type(roster.list) ~= "table" then return nil, "" end
		local out, sign = { list = {}, members = roster.members or {}, loans = roster.loans or {}, founders = {} }, {}
		-- Who founded a company in this room, dissolved since or not: the
		-- room founds no company for them in a competitive room
		-- (foundOwnCompany).
		for _, c in ipairs(roster.list) do
			if type(c.founder) == "string" then out.founders[c.founder] = true end
		end
		-- The loans the game offers now (its loan script's), which another
		-- company takes on the same terms.
		pcall(function()
			local e = api.engine.system.gameScriptSystem.getEntityForGameScript("::/game_mechanics/finance/loan.gs")
			local c = type(e) == "number" and e >= 0 and api.engine.getComponent(e, api.type.ComponentType.GAME_SCRIPT)
			local offers = c and c.state and c.state.availableLoans
			if type(offers) == "table" then out.offers = offers end
		end)
		for _, offer in ipairs(out.offers or {}) do sign[#sign + 1] = tostring(offer.type) .. tostring(offer.amount) end
		for _, loan in ipairs(out.loans) do sign[#sign + 1] = loan.id .. ":" .. loan.remaining end
		for _, c in ipairs(roster.list) do
			if not c.gone then
				local balance, owed, name
				pcall(function()
					local account = api.engine.getComponent(c.entity, api.type.ComponentType.ACCOUNT)
					balance = account and account.balance
					owed = account and account.loan
				end)
				-- The money the game's own windows show (the finance window,
				-- the game bar: api.engine.util.finance.getPlayersBalance),
				-- where the game answers. The room's first company's card
				-- showed $0 in a real game (build 40408, 2026-09-30) while
				-- the game bar showed its money; INFERRED that its ACCOUNT
				-- component does not hold what the game shows.
				pcall(function()
					local shown = api.engine.util.finance.getPlayersBalance(c.entity)
					if type(shown) == "number" then balance = shown end
				end)
				-- The name the game shows (the player entity's NAME, which a
				-- rename sets), else the roster's.
				pcall(function()
					local n = api.engine.getComponent(c.entity, api.type.ComponentType.NAME)
					if n and type(n.name) == "string" and n.name ~= "" then name = n.name end
				end)
				-- Whether it has a password, not the password's seal: the
				-- window has no use for it.
				local locked = type(c.lock) == "table"
				out.list[#out.list + 1] = { id = c.id, entity = c.entity, name = name or c.name, color = c.color,
					balance = balance, owed = owed, founder = c.founder, locked = locked, closed = c.closed == true,
					access = c.access }
				for _, a in ipairs(c.access or {}) do sign[#sign + 1] = c.id .. ">" .. tostring(a.company) .. "=" .. tostring(a.open) end
				local color = type(c.color) == "table" and c.color or {}
				sign[#sign + 1] = table.concat({ c.id, name or c.name, tostring(balance), tostring(owed),
					tostring(color[1]), tostring(color[2]), tostring(color[3]), tostring(locked),
					tostring(c.closed == true), tostring(c.founder) }, ":")
			end
		end
		for _, m in ipairs(out.members) do sign[#sign + 1] = tostring(m.player) .. "=" .. tostring(m.company) end
		return out, table.concat(sign, "|")
	end

	-- Founds the player's own company in a competitive room; below.
	local foundOwnCompany

	-- Reads the room and its chat from the hook into ui(): the room every
	-- STATUS_FRAMES frames, the chat every frame.
	local statusFrames = 0
	local function follow()
		if not link then return end
		local shared = ui()
		local changed = false
		statusFrames = statusFrames - 1
		if statusFrames <= 0 then
			statusFrames = STATUS_FRAMES
			local status = link:status()
			local before = shared.status and summary(shared.status) .. tostring(#(shared.status.players or {}))
			local after = status and summary(status) .. tostring(#(status.players or {}))
			shared.status = status
			if before ~= after then changed = true end
			local companies, sign = readCompanies()
			shared.companies = companies
			if sign ~= shared.companiesSign then
				shared.companiesSign = sign
				changed = true
			end
			foundOwnCompany(shared)
		end
		-- A new world's GUI gets the chat so far again, as old lines: they
		-- fill the window without counting as new.
		for _, line in ipairs(link:chat()) do
			shared.lines[#shared.lines + 1] = tostring(line.from) .. ": " .. tostring(line.text)
			if #shared.lines > CHAT_LINES then table.remove(shared.lines, 1) end
			if not shared.open and not line.old then shared.unread = shared.unread + 1 end
			changed = true
		end
		if changed then shared.version = shared.version + 1 end
	end

	-- What the Multiplayer window shows: the room, its speed, whether this
	-- world matches the room's, its players, and the chat with a field to
	-- write to it.
	-- Money as the game bar writes it, near enough.
	local function money(balance)
		if type(balance) ~= "number" then return "" end
		local sign, whole = balance < 0 and "-" or "", tostring(math.floor(math.abs(balance) + 0.5))
		whole = whole:reverse():gsub("(%d%d%d)", "%1,"):reverse():gsub("^,", "")
		return sign .. "$" .. whole
	end

	local function vec3(color)
		if type(color) ~= "table" then return nil end
		return api.type.Vec3f.new(color[1] or 0, color[2] or 0, color[3] or 0)
	end

	-- A company operation for the room, from the window. What became of it
	-- comes back with the player's other actions (follow the ticket). A
	-- password, for joining or locking a company, goes to the room beside it
	-- (tpf3mp/bridge.lua); `doing` never names it.
	local function companyOp(shared, op, doing, action, password)
		local l = shared.link
		if not l then return end
		local ok, ticket = l:command(action or { CompanyOp = op }, password)
		if ok then
			shared.asked = shared.asked or {}
			shared.asked[ticket] = doing
			shared.companyNote = doing .. "..."
		else
			shared.companyNote = "Not sent: " .. tostring(ticket)
		end
		shared.version = shared.version + 1
	end

	-- In a competitive room each player plays for a company of their own
	-- (docs/PLAYING.md, "Companies"): the player's game founds it for them,
	-- with the same action "Found a company" sends, which the room orders
	-- for every game. Only for a player who plays for the room's first
	-- company and never founded one in this room (one who dissolved theirs,
	-- or chose the first company again after founding, keeps that choice),
	-- and not before OWN_SETTLE readings of the room since this world's GUI
	-- linked, so a world still catching up has had time to apply what the
	-- room ordered before. The readings count whatever they said: a
	-- reading without the roster or the room's play style delays nothing
	-- that comes later. At most once a room and player in this game. The
	-- name is the player's, "<name>'s company", the same every time: a
	-- second one sent while the first is still on its way is refused alike
	-- in every game ("a company is called ... already"). In a co-op room,
	-- where the launcher has not said, or without the roster or the
	-- player's id, nothing is sent, and hook.log says why, once a reason;
	-- the player can still found one with "Found a company".
	local OWN_SETTLE = 4
	local own = { key = nil, readings = 0, sent = {}, told = {} }

	-- Says once in hook.log why this game founds no company for its player
	-- now; returns nil.
	local function notFounding(why)
		if own.told[why] then return end
		own.told[why] = true
		link:log("not founding the player's own company: " .. why)
	end

	-- "<name>'s company", the player's name as the room lists it (by id,
	-- else the entry marked as theirs), or the first eight hex digits of
	-- their id where it has none; with another player of the same name in
	-- the room, the first four hex digits of the player's id after it, so
	-- both get one. At most 32 + 17 bytes, within a company name's 64.
	local function ownCompanyName(status)
		local me
		for _, p in ipairs(status.players or {}) do
			if p.id == status.me_id or (me == nil and p.me) then me = p end
		end
		local name = me and type(me.name) == "string" and me.name:gsub("^%s+", ""):gsub("%s+$", "") or ""
		if name == "" then return status.me_id:sub(1, 8) .. "'s company" end
		for _, p in ipairs(status.players) do
			if p ~= me and type(p.name) == "string" and p.name:lower() == name:lower() then
				return name .. "'s company (" .. status.me_id:sub(1, 4) .. ")"
			end
		end
		return name .. "'s company"
	end

	foundOwnCompany = function(shared)
		local status, roster = shared.status, shared.companies
		if not (link and status) then return end
		local me = status.me_id
		local key = tostring(me) .. "@" .. tostring(status.invite or status.room)
		if own.key ~= key then own.key, own.readings = key, 0 end
		own.readings = own.readings + 1
		if status.competitive == nil then
			return notFounding("the launcher has not said whether the room is competitive")
		end
		if status.competitive ~= true then return notFounding("the room is co-op") end
		if type(me) ~= "string" or me == "" then return notFounding("the room has not said who this player is") end
		if not roster then return notFounding("the room's companies are not read yet") end
		local playing = require("tpf3mp.companies").of(roster, me)
		if not playing then return notFounding("the roster has no first company") end
		if playing.id ~= 0 then
			return notFounding("the player plays for " .. tostring(playing.name) .. " (#" .. tostring(playing.id) .. ")")
		end
		if roster.founders[me] then return notFounding("the player founded a company in this room before") end
		if own.sent[key] then return end
		if own.readings < OWN_SETTLE then return end
		local name = ownCompanyName(status)
		own.sent[key] = true
		link:log("a competitive room: founding the player's own company")
		companyOp(shared, { Create = { name = name } }, "Founding your company, " .. name)
	end

	-- The colours a company can wear: the companies' own first (a vehicle in
	-- one has its marker on the map in it too), then the game's line colours
	-- and greys, as its vehicle and line windows offer them
	-- (gui/line_vehicle_mgmt/line_react_util.tl). The chooser also takes a
	-- colour of the player's own (INFERRED from its style sheet's
	-- custom-color-button, gui/main/builtin.css.lua).
	local palette = nil
	local function companyPalette()
		if palette then return palette end
		palette = {}
		for _, color in ipairs(require("tpf3mp.companies").PALETTE) do palette[#palette + 1] = vec3(color) end
		pcall(function()
			local rep = api.gui.genericRep
			local color_util = ug_require("/gui/main/color_util.tl")
			for _, file in ipairs({ "::/gui/line_vehicle_mgmt/line_colors.gres", "::/gui/main/grayscale.gres" }) do
				for _, color in ipairs(color_util.toArray3(rep.get(rep.find(file)).data)) do
					palette[#palette + 1] = color
				end
			end
		end)
		return palette
	end

	-- The menu's Multiplayer glyph (tools/art/icons/menu_icon.py), for the
	-- button that opens the window and the window's header.
	local GLYPH = MOD .. "::/gui/tpf3mp/icons/menu_multiplayer_50.tga"
	-- A banner strip's size.
	local BANNER_W, BANNER_H = 160, 40
	-- Frames Copy says "Copied" for: about two seconds.
	local COPIED_FRAMES = 120

	-- The window's look ------------------------------------------------------
	--
	-- The main menu's Multiplayer window's (gui/menu/lobby.lua): the same
	-- helpers, classes (the font-scale-* sizes, primary and secondary
	-- buttons, the default style sheet's colours and tapes: success,
	-- warning, error, info), padding and spacing, so the two read as one.
	-- The content has a fixed size, as the menu's has: the header, the tabs
	-- (Players, Companies, Your company) in a scroll area on the left, the
	-- chat on the right, and a footer with Leave room.
	local WIDTH, HEIGHT = 900, 620
	local PADDING = { 14, 20, 14, 20 }
	local MAIN_W, CHAT_W, COLUMN_GAP = 540, 300, 20
	-- The tab's scroll area, and what its content leaves for the scroll bar.
	local BODY_H = 400
	local INNER_W = MAIN_W - 24
	local CHAT_H = BODY_H - 28
	local FIELD_W = 300
	-- A company card's text, and its buttons' column.
	local CARD_TEXT_W, CARD_ACTIONS_W = 340, 110
	-- A station access row's company and its state.
	local ACCESS_NAME_W, ACCESS_STATE_W = 190, 140
	-- A size left to the content, as the game's style sheets write it; a
	-- size of 0 hides everything inside.
	local AUTO = -1

	-- The game's spacer, as the menu's window uses it; a small gap where
	-- this state has none.
	local gui_util = (function()
		local ok, util = pcall(ug_require, "::/gui/main/gui_react_util.tl")
		if ok and type(util) == "table" and type(util.makeHorizontalSpacer) == "function" then return util end
		return nil
	end)()

	-- An inline style sheet: size as {w, h}, padding as {top, right, bottom,
	-- left}; nil where the state has none to make.
	local function style(t)
		local ok, sheet = pcall(function()
			local s = api.gui.StyleSheet.new()
			if t.size then s.size = api.type.Vec2f.new(t.size[1], t.size[2]) end
			if t.padding then
				s.padding = api.type.Vec4f.new(t.padding[1], t.padding[2], t.padding[3], t.padding[4])
			end
			return s
		end)
		return ok and sheet or nil
	end
	local function sized(w, h) return style{ size = { w, h } } end
	local function meta(sheet, more)
		local m = more or {}
		m.styleSheet = sheet
		return m
	end

	local function gap(px)
		px = math.max(1, px or 8)
		return builtin.Component{
			meta = meta(sized(px, px)),
			mouseTransparent = true,
			layout = builtin.BoxLayout{ children = {} },
		}
	end
	local function spacer()
		if gui_util then return gui_util.makeHorizontalSpacer() end
		return gap(8)
	end
	local function box(orientation, children, sheet)
		return builtin.Component{
			meta = meta(sheet),
			mouseTransparent = true,
			layout = builtin.BoxLayout{ orientation = orientation, children = children },
		}
	end
	local function row(children, sheet) return box(builtin.type.Orientation.Horizontal, children, sheet) end
	local function column(children, sheet) return box(builtin.type.Orientation.Vertical, children, sheet) end
	local function label(t, class, sheet)
		return builtin.TextView{ meta = meta(sheet, { class = class or "font-scale-body" }), text = t or "" }
	end
	-- A smaller, quieter line.
	local function note(t, class)
		return label(t, "font-scale-annotation" .. (class and (", " .. class) or ""))
	end
	-- A word on a coloured tape: success, warning, error or info.
	local function badge(t, tone)
		return label(" " .. t .. " ", "font-scale-annotation, " .. (tone or "info") .. "-tape")
	end
	local function picture(path, w, h)
		return builtin.ImageView{ meta = meta(sized(w, h)), path = path }
	end
	local function button(t, onClick, tooltip, class)
		return builtin.Button{
			meta = { class = class or "secondary", tooltip = tooltip },
			content = label(t),
			onClick = onClick,
		}
	end
	-- A section's title, and a line under it that says what it is for.
	local function heading(t, sub)
		local children = { label(t, "font-scale-title-4") }
		if sub then
			children[#children + 1] = gap(2)
			children[#children + 1] = note(sub)
		end
		children[#children + 1] = gap(8)
		return column(children)
	end
	-- A text field kept in `ref`, sent with `onEnter` on Enter. Clicking
	-- away keeps what was typed, and a redraw shows it: a button sends what
	-- the field shows, never a line the field dropped (it emptied itself on
	-- a cancel, build 40408, while the button still had the text).
	local function input(ref, placeholder, width, onEnter, params)
		params = params or {}
		local function redraw() local shared = ui() shared.version = shared.version + 1 end
		return builtin.TextInputField{
			meta = meta(sized(width, 36), { class = "font-scale-body" }),
			placeholderText = placeholder,
			value = ref:get(),
			maxLength = params.maxLength or 64,
			passwordMode = params.secret or nil,
			acceptOnFocusLoss = false,
			resetValueOnCancel = false,
			onTyping = function(t) ref:set(t) end,
			onCancel = redraw,
			onValueChange = function(t) onEnter(t) end,
		}
	end
	-- `children` with a gap between each two.
	local function spaced(children, px)
		local out = {}
		for i, child in ipairs(children) do
			if i > 1 then out[#out + 1] = gap(px or 8) end
			out[#out + 1] = child
		end
		return out
	end
	local function blank(t) return type(t) ~= "string" or t:match("^%s*$") end

	-- What the company tabs read off the roster: each player's company, who
	-- is in each, the player's own and the room's first, the companies in
	-- the order the cards show them (yours first), and whether the player
	-- is their company's head. Nil before the roster is read.
	local function rosterView(status, shared)
		local roster = shared.companies
		if not roster then return nil end
		local companies = require("tpf3mp.companies")
		local v = { roster = roster, byId = {}, companyOf = {}, names = {} }
		for _, p in ipairs(status.players or {}) do v.byId[p.id] = p end
		for _, m in ipairs(roster.members) do v.companyOf[m.player] = m.company end
		for _, p in ipairs(status.players or {}) do
			local id = v.companyOf[p.id] or 0
			v.names[id] = v.names[id] or {}
			v.names[id][#v.names[id] + 1] = tostring(p.name) .. (p.me and " (you)" or "")
			if p.me then v.mine = id end
		end
		if v.mine == nil then v.mine = v.companyOf[status.me_id] or 0 end
		for _, c in ipairs(roster.list) do
			if c.id == 0 then v.first = c end
			if c.id == v.mine then v.myCompany = c end
		end
		v.ordered = { v.myCompany }
		for _, c in ipairs(roster.list) do if c.id ~= v.mine then v.ordered[#v.ordered + 1] = c end end
		v.iHead = status.me_id ~= nil and companies.head(roster, v.mine) == status.me_id
		return v
	end

	-- A company's colour: the player's own to change, the others' only to
	-- see. The others' swatch lets the mouse through instead of being
	-- disabled, which drew it on a grey plate (build 40408, 2026-10-01).
	local function swatch(shared, c, yours)
		return builtin.ColorChooserButton{
			meta = yours and { tooltip = "Your company's colour: its vehicles wear it" }
				or { mouseTransparent = true },
			colors = companyPalette(),
			color = vec3(c.color),
			onValueChange = function(v)
				if not yours then return end
				local r, g, b = v.x or v[1], v.y or v[2], v.z or v[3]
				companyOp(shared, { Recolor = { company = c.id, color = { r = r, g = g, b = b } } },
					"Recolouring " .. c.name)
			end,
			resetButton = false,
		}
	end

	-- The Players tab: one row each, their banner (tpf3mp/banners.lua, the
	-- same the main menu's window shows), their portrait if they picked
	-- one, their name, the room's owner and you under it, and on the right
	-- how far their game is with the room's world, or Away.
	local function playersTab(status)
		local banners = require("tpf3mp.banners")
		local players = status.players or {}
		local here = 0
		for _, p in ipairs(players) do if p.connected then here = here + 1 end end
		local rows = {
			note(#players .. (#players == 1 and " player" or " players") .. " · " .. here .. " connected"),
			gap(10),
		}
		for _, p in ipairs(players) do
			local roles = {}
			if p.owner then roles[#roles + 1] = "Owner" end
			if p.me then roles[#roles + 1] = "You" end
			local stage = banners.stage(p, true)
			local tone = "success"
			if p.loading == "fetching" or p.loading == "loading" then
				tone = "info"
			elseif not p.connected or stage == nil then
				stage, tone = "Away", "warning"
			elseif stage ~= "Playing" and stage ~= "Ready" then
				tone = "warning"
			end
			local cells = { picture(banners.picture(banners.of(p)), BANNER_W, BANNER_H), gap(10) }
			-- A campaign character's portrait the player picked, beside their
			-- name: the hook passes it only where this game has the picture.
			local portrait = banners.portraitOf(p)
			if portrait then
				cells[#cells + 1] = picture(portrait, BANNER_H, BANNER_H)
				cells[#cells + 1] = gap(10)
			end
			local who = { label(tostring(p.name), "font-scale-title-5") }
			if #roles > 0 then who[#who + 1] = note(table.concat(roles, " · ")) end
			cells[#cells + 1] = column(who)
			cells[#cells + 1] = spacer()
			cells[#cells + 1] = badge(stage, tone)
			rows[#rows + 1] = row(cells, style{ size = { INNER_W, AUTO } })
			rows[#rows + 1] = gap(8)
		end
		return rows
	end

	-- The Companies tab (tpf3mp/companies.lua; DECISIONS.md D22, proposed):
	-- a card each, with its colour, name, marks, money, head and players;
	-- yours first and marked. Switching is one click: Switch on any other
	-- company (its password asked for first where it has one), Leave on
	-- yours back to the room's first company, Dissolve on yours by its last
	-- player.
	local function companiesTab(v, shared, drafts)
		local companies = require("tpf3mp.companies")
		local function redraw() shared.version = shared.version + 1 end
		-- Switch to `c`: at once, or, where it has a password, once typed.
		local function switchTo(c, password)
			if c.locked and blank(password) then
				drafts.prompt:set(c.id)
				shared.companyNote = c.name .. " has a password: type it, then Switch"
				redraw()
				return
			end
			drafts.prompt:set(nil)
			companyOp(shared, { Join = c.id }, "Switching to " .. c.name, nil, c.locked and password or nil)
			drafts.joinPassword:set("")
		end

		local live = 0
		for _, c in ipairs(v.ordered) do if c then live = live + 1 end end
		local rows = {
			note(live .. (live == 1 and " company" or " companies")
				.. " · each has its own money, vehicles and stations"),
			gap(10),
		}
		for _, c in ipairs(v.ordered) do
			local yours = c.id == v.mine
			local head = companies.head(v.roster, c.id)
			local title = { label(tostring(c.name), "font-scale-title-5") }
			if yours then title[#title + 1] = badge("Yours", "info") end
			if c.locked then title[#title + 1] = badge("Password", "warning") end
			if c.closed then title[#title + 1] = badge("Stations closed", "warning") end
			title = spaced(title, 6)
			title[#title + 1] = spacer()
			if c.balance ~= nil then title[#title + 1] = label(money(c.balance), "font-scale-body") end
			local about = {}
			if c.id == 0 then
				about[#about + 1] = "Everyone's company, no head"
			elseif head and v.byId[head] then
				about[#about + 1] = "Head: " .. tostring(v.byId[head].name)
			end
			if type(c.owed) == "number" and c.owed > 0 then about[#about + 1] = "Owes " .. money(c.owed) end
			local text = { row(title, style{ size = { CARD_TEXT_W, AUTO } }) }
			if #about > 0 then text[#text + 1] = note(table.concat(about, " · ")) end
			text[#text + 1] = note("Players: " .. (v.names[c.id] and table.concat(v.names[c.id], ", ") or "nobody"))

			local actions = {}
			if yours then
				if c.id ~= 0 and v.first then
					actions[#actions + 1] = button("Leave", function() switchTo(v.first) end,
						"Leave " .. tostring(c.name) .. " and play for " .. tostring(v.first.name)
							.. ", the room's first company, again")
				end
				if c.id ~= 0 and #(v.names[c.id] or {}) <= 1 then
					-- Its last player dissolves it, once it owns nothing.
					actions[#actions + 1] = button("Dissolve",
						function() companyOp(shared, { Delete = c.id }, "Dissolving " .. c.name) end,
						"Dissolve " .. tostring(c.name) .. " once it owns nothing, and play for the room's first company again")
				end
			else
				actions[#actions + 1] = button("Switch", function() switchTo(c, drafts.joinPassword:get()) end,
					"Play for " .. tostring(c.name) .. " from now on" .. (c.locked and "; it needs its password" or ""),
					"primary")
			end
			if #actions == 0 then actions[1] = gap(1) end

			rows[#rows + 1] = row({
				swatch(shared, c, yours),
				gap(10),
				column(text, style{ size = { CARD_TEXT_W, AUTO } }),
				spacer(),
				column(spaced(actions, 4), style{ size = { CARD_ACTIONS_W, AUTO } }),
			}, style{ size = { INNER_W, AUTO }, padding = { 6, 4, 6, 4 } })
			if not yours and c.locked and drafts.prompt:get() == c.id then
				-- Its password, typed here; the room seals it, and only the
				-- seal reaches the games.
				rows[#rows + 1] = row({
					gap(42),
					input(drafts.joinPassword, "Password of " .. tostring(c.name), FIELD_W - 60,
						function(t) if not blank(t) then switchTo(c, t) end end, { secret = true }),
					gap(8),
					note("then Switch"),
				})
			end
			rows[#rows + 1] = gap(10)
		end
		return rows
	end

	-- Who may have their lines stop at company `c`'s stations, for its
	-- head to choose (D22, proposed): a default, which also holds for
	-- companies founded later, and a choice for each other company, which
	-- wins over it. Per company, not per player: a company's players share
	-- everything it owns.
	local function stationRows(rows, roster, c, shared)
		local companies = require("tpf3mp.companies")
		local function access(name, state, allowed, buttons)
			local cells = {
				label(name, "font-scale-body", sized(ACCESS_NAME_W, AUTO)),
				label(state, "font-scale-body, " .. (allowed and "success" or "warning"), sized(ACCESS_STATE_W, AUTO)),
				spacer(),
			}
			for _, b in ipairs(spaced(buttons, 6)) do cells[#cells + 1] = b end
			rows[#rows + 1] = row(cells, style{ size = { INNER_W, AUTO } })
			rows[#rows + 1] = gap(4)
		end
		local open = not c.closed
		rows[#rows + 1] = heading("Station access", "Whose lines may stop at " .. tostring(c.name) .. "'s stations.")
		rows[#rows + 1] = note("The default also holds for companies founded later.")
		rows[#rows + 1] = gap(6)
		access("Default", open and "Allowed" or "Denied", open, {
			button(open and "Deny by default" or "Allow by default", function()
				companyOp(shared, { ShareStations = { company = c.id, open = not open } },
					(open and "Closing " or "Opening ") .. "the stations of " .. c.name .. " by default")
			end, open and "Keep " .. tostring(c.name) .. "'s stations from every company without a choice of its own"
				or "Let every company without a choice of its own stop at " .. tostring(c.name) .. "'s stations"),
		})
		for _, other in ipairs(roster.list or {}) do
			if other.id ~= c.id then
				local choice = companies.choice(c, other.id)
				local allowed = companies.lets(c, other.id)
				local buttons = {
					button(allowed and "Deny" or "Allow", function()
						companyOp(shared, { StationAccess = { company = c.id, other = other.id, open = not allowed } },
							(allowed and "Denying " or "Allowing ") .. other.name)
					end, (allowed and "Keep " or "Let ") .. tostring(other.name)
						.. (allowed and "'s lines from " or "'s lines stop at ") .. tostring(c.name) .. "'s stations"),
				}
				if choice ~= nil then
					buttons[#buttons + 1] = button("Default", function()
						companyOp(shared, { StationAccess = { company = c.id, other = other.id } },
							"Putting " .. other.name .. " back to the default")
					end, tostring(other.name) .. " follows the default again")
				end
				access(tostring(other.name), (allowed and "Allowed" or "Denied") .. (choice == nil and " (default)" or ""),
					allowed, buttons)
			end
		end
		rows[#rows + 1] = gap(14)
	end

	-- The Your company tab: what you may do with yours. Its name (any of
	-- its players), and for its head its password, its stations and
	-- sending a player out; its own loans, for companies past the first,
	-- which are the room's (the first company's are in the game's finance
	-- window); and founding a company of your own. Its colour is the
	-- swatch on its card. The room's first company is everyone's: renamed
	-- in the game's own company window.
	local function yourCompanyTab(v, status, shared, drafts)
		local companies = require("tpf3mp.companies")
		local rows = {}
		local function fieldRow(children) rows[#rows + 1] = row(spaced(children, 8)) end
		if v.myCompany and v.mine ~= 0 then
			local c = v.myCompany
			local mine = v.mine
			local function rename(t)
				if blank(t) then return end
				companyOp(shared, { Rename = { company = mine, name = t } }, "Renaming " .. tostring(c.name))
				drafts.rename:set("")
			end
			rows[#rows + 1] = heading(tostring(c.name), v.iHead and "Your company: you are its head"
				or "Your company")
			fieldRow({
				input(drafts.rename, "A new name for " .. tostring(c.name), FIELD_W, rename),
				button("Rename", function() rename(drafts.rename:get()) end, "Rename the company you play for"),
			})
			rows[#rows + 1] = gap(14)
			if v.iHead then
				local function lock(t)
					if blank(t) then return end
					companyOp(shared, { Lock = c.id }, "Setting the password of " .. c.name, nil, t)
					drafts.lockPassword:set("")
				end
				rows[#rows + 1] = heading("Password", c.locked and "Only players who know it can join."
					or "Anyone can join; with one, only players who know it can.")
				local lockRow = {
					input(drafts.lockPassword, c.locked and "A new password" or "A password to join", FIELD_W, lock,
						{ secret = true }),
					button(c.locked and "Change" or "Set", function() lock(drafts.lockPassword:get()) end,
						"Only players who know it can join " .. tostring(c.name)),
				}
				if c.locked then
					lockRow[#lockRow + 1] = button("Clear",
						function() companyOp(shared, { Unlock = c.id }, "Clearing the password of " .. c.name) end,
						"Let anyone join " .. tostring(c.name))
				end
				fieldRow(lockRow)
				rows[#rows + 1] = gap(14)
				stationRows(rows, v.roster, c, shared)
				local others = {}
				for _, player in ipairs(companies.members(v.roster, mine)) do
					local p = v.byId[player]
					if player ~= status.me_id and p then others[#others + 1] = { id = player, p = p } end
				end
				rows[#rows + 1] = heading("Players", "Send a player back to the room's first company.")
				if #others == 0 then
					rows[#rows + 1] = note("Nobody else plays for " .. tostring(c.name) .. " yet.")
				end
				for _, o in ipairs(others) do
					rows[#rows + 1] = row({
						label(tostring(o.p.name)),
						spacer(),
						button("Send out", function()
							companyOp(shared, { Dismiss = { company = c.id, player = o.id } },
								"Sending " .. tostring(o.p.name) .. " out of " .. c.name)
						end, "Send " .. tostring(o.p.name) .. " back to the room's first company"),
					}, style{ size = { INNER_W, AUTO } })
					rows[#rows + 1] = gap(4)
				end
				rows[#rows + 1] = gap(14)
			end
			-- Its loans, and the loans the game offers now.
			rows[#rows + 1] = heading("Loans", "Borrowed by " .. tostring(c.name) .. " on the terms the game offers.")
			local any = false
			for _, loan in ipairs(v.roster.loans or {}) do
				if loan.company == mine then
					any = true
					rows[#rows + 1] = row({
						column({
							label(money(loan.remaining) .. " owed of " .. money(loan.amount)),
							note(money(loan.payment) .. " a month · " .. (loan.months - loan.paid) .. " months left"),
						}),
						spacer(),
						button("Repay", function()
							companyOp(shared, nil, "Repaying " .. money(loan.remaining), { Loan = { Repay = { loan = {
								type = "Custom", amount = loan.amount, duration = 1, percentage = 0, id = loan.id } } } })
						end, "Pay back what is still owed now"),
					}, style{ size = { INNER_W, AUTO } })
					rows[#rows + 1] = gap(6)
				end
			end
			for _, offer in ipairs(v.roster.offers or {}) do
				if type(offer) == "table" and type(offer.amount) == "number" then
					any = true
					local rate = string.format("%g%% a year", (offer.percentage or 0) * 100)
					rows[#rows + 1] = row({
						label(money(offer.amount), "font-scale-body", sized(ACCESS_NAME_W, AUTO)),
						note(rate),
						spacer(),
						button("Borrow", function()
							local terms = { type = offer.type, amount = offer.amount, duration = offer.duration,
								percentage = offer.percentage }
							companyOp(shared, nil, "Borrowing " .. money(offer.amount),
								{ Loan = { Take = { next = terms, offer = terms } } })
						end, "Borrow " .. money(offer.amount) .. " at " .. rate),
					}, style{ size = { INNER_W, AUTO } })
					rows[#rows + 1] = gap(4)
				end
			end
			if not any then rows[#rows + 1] = note("No loans, and the game offers none now.") end
			rows[#rows + 1] = gap(14)
		else
			rows[#rows + 1] = heading(v.first and tostring(v.first.name) or "Your company",
				"You play for the room's first company, which everyone shares.")
			rows[#rows + 1] = note("Rename it in the game's company window.")
			rows[#rows + 1] = gap(14)
		end
		-- A company of your own.
		local function found(t)
			if blank(t) then return end
			companyOp(shared, { Create = { name = t } }, "Founding " .. t)
			drafts.found:set("")
		end
		rows[#rows + 1] = heading("A company of your own", "Found one and play for it.")
		fieldRow({
			input(drafts.found, "A company of your own", FIELD_W, found),
			button("Found", function() found(drafts.found:get()) end, "Found a company and play for it", "primary"),
		})
		return rows
	end

	-- The room page in the game: a header with the room, its invite code
	-- with Copy, its speed and whether this world matches the room's; the
	-- tabs Players, Companies and Your company on the left; the chat on the
	-- right; and what just happened and Leave room, which asks first, at
	-- the bottom.
	local function windowContent(status, draft, drafts, confirm, tab)
		local shared = ui()
		local function redraw() shared.version = shared.version + 1 end
		local function send(t)
			local l = shared.link
			if not l or type(t) ~= "string" or t:match("^%s*$") then return end
			local ok, why = l:say(t)
			if ok then
				draft:set("")
			else
				shared.lines[#shared.lines + 1] = "(not sent: " .. tostring(why) .. ")"
			end
			redraw()
		end

		-- The header.
		local top = { picture(GLYPH, 28, 28), gap(10), label(tostring(status.room), "font-scale-title-3") }
		if status.competitive ~= nil then
			top[#top + 1] = gap(10)
			top[#top + 1] = badge(status.competitive and "Competitive" or "Co-op",
				status.competitive and "warning" or "success")
		end
		top[#top + 1] = spacer()
		if status.speed then
			top[#top + 1] = badge("Speed " .. speedText(status.speed), "info")
			top[#top + 1] = gap(6)
		end
		top[#top + 1] = status.diverged and badge("Resyncing", "warning") or badge("Worlds match", "success")
		local header = { row(top, style{ size = { WIDTH - 40, AUTO } }) }
		-- The room's invite code alone (without the server the launcher may
		-- put before it), and Copy: the hook puts it on the clipboard, and
		-- the button says "Copied" for a moment.
		local code = type(status.invite) == "string" and status.invite:match("(%S+)%s*$")
		if code then
			header[#header + 1] = gap(6)
			header[#header + 1] = row({
				note("Invite code"),
				gap(8),
				label(code, "font-scale-title-4, info"),
				gap(10),
				button((shared.copied or 0) > 0 and "Copied" or "Copy", function()
					local l = shared.link
					local ok, why = false, "not linked"
					if l then ok, why = l:copy(code) end
					if ok then
						shared.copied = COPIED_FRAMES
					else
						shared.leaveNote = "Not copied: " .. tostring(why)
					end
					redraw()
				end, "Copy the invite code, to paste it to your friends"),
				gap(10),
				note("Friends join with it from their game's Multiplayer window."),
			})
		end
		if status.diverged then
			header[#header + 1] = gap(6)
			header[#header + 1] = label("Your world differed from the room's at step " .. tostring(status.diverged)
				.. "; the room's is on its way", "font-scale-body, warning")
		end

		-- The tabs, and the one shown.
		local v = rosterView(status, shared)
		local tabs = { { "players", "Players" } }
		if v then
			tabs[#tabs + 1] = { "companies", "Companies" }
			tabs[#tabs + 1] = { "yours", "Your company" }
		end
		local shown = tabs[1][1]
		for _, t in ipairs(tabs) do if t[1] == tab:get() then shown = t[1] end end
		local bar
		if #tabs == 1 then
			bar = label(tabs[1][2], "font-scale-title-4")
		else
			local buttons = {}
			for _, t in ipairs(tabs) do
				buttons[#buttons + 1] = button(t[2], function()
					tab:set(t[1])
					redraw()
				end, nil, t[1] == shown and "primary" or "secondary")
			end
			bar = row(spaced(buttons, 6))
		end
		local body
		if shown == "companies" then
			body = companiesTab(v, shared, drafts)
		elseif shown == "yours" then
			body = yourCompanyTab(v, status, shared, drafts)
		else
			body = playersTab(status)
		end
		local main = column({
			bar,
			gap(10),
			builtin.ScrollArea{
				meta = meta(sized(MAIN_W, BODY_H)),
				horizontalPolicy = builtin.type.ScrollBarPolicy.AlwaysOff,
				verticalPolicy = builtin.type.ScrollBarPolicy.AsNeeded,
				content = column(body, style{ size = { INNER_W, AUTO } }),
			},
		}, style{ size = { MAIN_W, AUTO } })

		-- The chat: the newest lines, and a field to write to the room.
		local lines = {}
		for i = math.max(1, #shared.lines - CHAT_SHOWN + 1), #shared.lines do
			lines[#lines + 1] = label(shared.lines[i])
			lines[#lines + 1] = gap(3)
		end
		if #shared.lines == 0 then lines[1] = note("Nobody said anything yet.") end
		local chat = column({
			label("Chat", "font-scale-title-4"),
			gap(10),
			builtin.ScrollArea{
				meta = meta(sized(CHAT_W, CHAT_H)),
				horizontalPolicy = builtin.type.ScrollBarPolicy.AlwaysOff,
				verticalPolicy = builtin.type.ScrollBarPolicy.AsNeeded,
				content = column(lines, style{ size = { CHAT_W - 24, AUTO } }),
			},
			gap(8),
			row({
				input(draft, "Say something to the room", CHAT_W - 80, send, { maxLength = 280 }),
				gap(8),
				button("Send", function() send(draft:get()) end),
			}),
		}, style{ size = { CHAT_W, AUTO } })

		-- The footer: what just happened, and Leave room, which asks first.
		local footer = {}
		if shared.companyNote then footer[#footer + 1] = label(shared.companyNote, "font-scale-body, info") end
		if shared.leaveNote then footer[#footer + 1] = label(shared.leaveNote, "font-scale-body, info") end
		local leave
		if confirm:get() then
			footer[#footer + 1] = label("Leave the room? Your game stops following it; start the game again from the "
				.. "launcher for the next room.", "font-scale-body, warning")
			leave = {
				button("Leave", function()
					confirm:set(false)
					local l = shared.link
					local ok, why = false, "not linked"
					if l then ok, why = l:leave() end
					shared.leaveNote = ok and "Leaving the room..." or ("Not left: " .. tostring(why))
					redraw()
				end, nil, "primary"),
				button("Stay", function()
					confirm:set(false)
					redraw()
				end),
			}
		else
			leave = {
				button("Leave room", function()
					confirm:set(true)
					redraw()
				end, "Give up your seat in the room"),
			}
		end
		footer[#footer + 1] = gap(6)
		footer[#footer + 1] = row(spaced(leave, 8))

		return column({
			column(header),
			gap(14),
			row({ main, gap(COLUMN_GAP), chat }),
			gap(10),
			column(footer),
		}, style{ size = { WIDTH, HEIGHT }, padding = PADDING })
	end

	-- The Multiplayer window, which the game's window container shows, as
	-- the game bar shows its context help (game_bar.tl: the window API's
	-- addSingletonWindow; a window rendered anywhere else shows nothing,
	-- build 40408). One recipe for both plugins, kept in ui(): the game may
	-- run this file once for each.
	local function windowRecipe()
		local shared = ui()
		if shared.window == nil then
			shared.window = react.RegisterWrapperRecipe("Tpf3mpWindow", builtin.Window, function(params)
				local drawn = react.useState(0)
				local draft = react.useRef("")
				local confirmLeave = react.useRef(false)
				local drafts = { rename = react.useRef(""), found = react.useRef(""),
					joinPassword = react.useRef(""), lockPassword = react.useRef(""),
					-- The company whose password Switch asks for, or nil.
					prompt = react.useRef(nil) }
				-- The tab shown: players, companies or yours.
				local tab = react.useRef("players")
				react.onStep(function()
					local version = ui().version
					if version ~= drawn:old() then drawn:set(version) end
				end)
				local _ = drawn:old()
				local status = ui().status
				local content
				if status then
					-- Should the page's Lua fail, the error, never a window
					-- that cannot show: the game's close button still works.
					local ok, page = pcall(windowContent, status, draft, drafts, confirmLeave, tab)
					if ok then
						content = page
					else
						say("the Multiplayer window failed: " .. tostring(page))
						content = builtin.BoxLayout{ orientation = builtin.type.Orientation.Vertical,
							children = { builtin.TextView{ meta = { class = "font-scale-body, error" },
								text = "The Multiplayer window failed: " .. tostring(page) } } }
					end
				else
					content = builtin.BoxLayout{ orientation = builtin.type.Orientation.Vertical,
						children = { builtin.TextView{ text = "Not in a room." } } }
				end
				return builtin.Window{
					id = "tpf3mp.multiplayer.window",
					title = "Multiplayer",
					closable = true,
					onClose = params.onClose,
					-- Where it opens, as a share of the screen (the entity
					-- windows open at 1, 0: top right): at the left, below
					-- the mods' buttons, which it would cover at 0, 0.
					initialX = 0,
					initialY = 0.15,
					content = content,
				}
			end)
		end
		return shared.window
	end

	-- Opens the Multiplayer window, or closes it if open. A window the game
	-- will not show is said in the game bar and the log.
	local function toggleWindow()
		local shared = ui()
		local ok, why = pcall(function()
			local windows = game_react_globals.getDefaultWindowApi()
			local recipe = windowRecipe()
			local function close()
				shared.open = false
				shared.version = shared.version + 1
				windows.removeAllWindows(recipe)
			end
			if shared.open then
				close()
			else
				shared.open = true
				shared.unread = 0
				shared.version = shared.version + 1
				windows.addSingletonWindow(recipe, { onClose = close })
				windows.moveSingletonWindowToFront(recipe)
			end
		end)
		if not ok then
			shared.open = false
			shared.version = shared.version + 1
			say("the Multiplayer window did not open: " .. tostring(why))
			notice = "The Multiplayer window did not open"
		end
	end

	local Tpf3mpPlugin = react.RegisterPluginRecipe(game_bar_widgets.GameBarInfoDisplayExtension, "Tpf3mpPlugin", function()
		-- Once per game: the ref lives as long as this plugin is mounted.
		local started = react.useRef(false)
		-- The refusal notice shown, or false, and the frames it has left.
		local shown = react.useState(false)
		local frames = react.useRef(0)
		-- The room's version drawn, and the one last seen: a change redraws.
		local room = react.useState(0)
		local seen = react.useRef(0)
		react.onStep(function()
			if not started:get() then
				started:set(true)
				-- A new world's window container has no Multiplayer window.
				ui().open = false
				local ok, err = pcall(start)
				if not ok then say("start failed: " .. tostring(err)) end
			end
			local ok, err = pcall(serve)
			if not ok then say("serving the hook failed: " .. tostring(err)) end
			local followed, why = pcall(follow)
			if not followed then say("reading the room failed: " .. tostring(why)) end
			-- The main menu's Multiplayer window was open as this world came
			-- up: this one opens in its place, once the room is read, so the
			-- player keeps the lobby they had (the hook closed the menu's).
			if link and ui().status and not ui().open and link:handover() then toggleWindow() end
			local copied = ui().copied
			if copied and copied > 0 then
				ui().copied = copied - 1
				if copied == 1 then ui().version = ui().version + 1 end
			end
			if ui().version ~= seen:get() then
				seen:set(ui().version)
				room:set(ui().version)
			end
			runPending()
			if link and guardedCmd then
				local delivered, why = pcall(function()
					local results = link:results()
					require("tpf3mp.guard").deliver(guardedCmd, results, sees, clock)
					local shared = ui()
					for _, r in ipairs(results or {}) do
						local doing = shared.asked and r.ticket and shared.asked[r.ticket]
						if doing then
							shared.asked[r.ticket] = nil
							shared.companyNote = r.ok and (doing .. ": done")
								or (doing .. ": not done, " .. tostring(r.why))
							shared.version = shared.version + 1
						end
						local asked = r.ticket and answers[r.ticket]
						if asked then
							answers[r.ticket] = nil
							if r.ok ~= true then notice = asked .. ": not done, " .. tostring(r.why) end
						end
					end
				end)
				if not delivered then say("answering the player's commands failed: " .. tostring(why)) end
			end
			if notice then
				shown:set(notice)
				frames:set(NOTICE_FRAMES)
				notice = nil
			elseif frames:get() > 0 then
				frames:set(frames:get() - 1)
				if frames:get() == 0 then shown:set(false) end
			end
		end)
		-- Even empty, the layout keeps the plugin mounted, so onStep keeps
		-- running.
		local children = {}
		local _ = room:old()
		local shared = ui()
		if shared.status then
			local label = summary(shared.status)
			if shared.unread > 0 then label = label .. " · " .. shared.unread .. " new" end
			children[#children + 1] = builtin.Button{
				meta = { tooltip = "Open the Multiplayer window" },
				-- The game bar is low: the small font keeps the button in it.
				content = builtin.TextView{ meta = { class = "font-scale-annotation" }, text = label },
				onClick = toggleWindow,
			}
		end
		if shown:old() then
			children[#children + 1] = builtin.TextView{ text = shown:old() }
		end
		return builtin.BoxLayout{
			orientation = builtin.type.Orientation.Horizontal,
			children = children,
		}
	end)

	-- The Multiplayer button in the game's area for mods' buttons, in the
	-- room's game.
	local Tpf3mpButton = react.RegisterPluginRecipe(main_mod_button_area.MainModButtonAreaExtension, "Tpf3mpButton", function()
		local drawn = react.useState(0)
		react.onStep(function()
			local version = ui().version
			if version ~= drawn:old() then drawn:set(version) end
		end)
		local _ = drawn:old()
		local shared = ui()
		local children = {}
		if shared.status then
			-- The main menu's Multiplayer glyph, and how many chat lines are
			-- new.
			local glyph = sized(32, 32)
			local inside = { builtin.ImageView{ meta = glyph and { styleSheet = glyph } or {}, path = GLYPH } }
			if shared.unread > 0 then inside[2] = builtin.TextView{ text = tostring(shared.unread) } end
			children[1] = builtin.Button{
				meta = { tooltip = "Multiplayer: the room, its players, companies and chat" },
				content = builtin.BoxLayout{ orientation = builtin.type.Orientation.Horizontal, children = inside },
				onClick = toggleWindow,
			}
		end
		return builtin.BoxLayout{ orientation = builtin.type.Orientation.Horizontal, children = children }
	end)

	return {
		Tpf3mpPlugin = Tpf3mpPlugin,
		Tpf3mpButton = Tpf3mpButton,
	}
end

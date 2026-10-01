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

	-- Whether the GUI's world has `entity` yet: what the room's action made
	-- in the simulation reaches it a moment later.
	local function sees(entity)
		local ok, there = pcall(function() return api.engine.entityExists(entity) end)
		return not ok or there == true
	end

	-- Puts the guard in front of the GUI's commands.
	local function guardCommands()
		local ok, cmd = pcall(function() return api.cmd end)
		guardedCmd = ok and cmd or nil
		local wrapped, why = require("tpf3mp.guard").install(guardedCmd, {
			inRoom = function() return link:room() end,
			command = function(action) return link:command(action) end,
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
		local out, sign = { list = {}, members = roster.members or {}, loans = roster.loans or {} }, {}
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
	-- button that opens the window.
	local GLYPH = MOD .. "::/gui/tpf3mp/icons/menu_multiplayer_50.tga"
	-- A banner strip's size, and the window's two columns.
	local BANNER_W, BANNER_H = 160, 40
	local LEFT_W, CHAT_W = 480, 360

	-- A style sheet of one size, or nil where the state has none to make.
	local function sized(w, h)
		local ok, sheet = pcall(function()
			local s = api.gui.StyleSheet.new()
			s.size = api.type.Vec2f.new(w, h)
			return s
		end)
		return ok and sheet or nil
	end
	local function hbox(children)
		return builtin.BoxLayout{ orientation = builtin.type.Orientation.Horizontal, children = children }
	end
	local function column(children, w)
		local sheet = w and sized(w, -1)
		return builtin.Component{
			meta = sheet and { styleSheet = sheet } or {},
			layout = builtin.BoxLayout{ orientation = builtin.type.Orientation.Vertical, children = children },
		}
	end
	local function text(t, class)
		return builtin.TextView{ meta = class and { class = class } or nil, text = t }
	end

	-- Who may have their lines stop at company `c`'s stations, for its
	-- head to choose (D22, proposed): a default, which also holds for
	-- companies founded later, and a toggle for each other company, which
	-- wins over it. Per company, not per player: a company's players share
	-- everything it owns.
	local function stationRows(rows, roster, c, shared)
		local companies = require("tpf3mp.companies")
		local function button(label, tooltip, onClick)
			return builtin.Button{ meta = { tooltip = tooltip }, content = text(label), onClick = onClick }
		end
		local open = not c.closed
		rows[#rows + 1] = text("Who may stop at " .. tostring(c.name) .. "'s stations", "font-scale-body")
		rows[#rows + 1] = hbox({
			text("Default, and companies founded later: " .. (open and "allowed" or "denied")),
			button(open and "Deny by default" or "Allow by default", open
				and "Keep " .. tostring(c.name) .. "'s stations from every company without a choice of its own"
				or "Let every company without a choice of its own stop at " .. tostring(c.name) .. "'s stations",
				function()
					companyOp(shared, { ShareStations = { company = c.id, open = not open } },
						(open and "Closing " or "Opening ") .. "the stations of " .. c.name .. " by default")
				end),
		})
		for _, other in ipairs(roster.list or {}) do
			if other.id ~= c.id then
				local choice = companies.choice(c, other.id)
				local allowed = companies.lets(c, other.id)
				local row = {
					text(tostring(other.name) .. ": " .. (allowed and "allowed" or "denied")
						.. (choice == nil and " (default)" or "")),
					button(allowed and "Deny" or "Allow", (allowed and "Keep " or "Let ") .. tostring(other.name)
						.. (allowed and "'s lines from " or "'s lines stop at ") .. tostring(c.name) .. "'s stations",
						function()
							companyOp(shared, { StationAccess = { company = c.id, other = other.id, open = not allowed } },
								(allowed and "Denying " or "Allowing ") .. other.name)
						end),
				}
				if choice ~= nil then
					row[#row + 1] = button("Default", tostring(other.name) .. " follows the default again", function()
						companyOp(shared, { StationAccess = { company = c.id, other = other.id } },
							"Putting " .. other.name .. " back to the default")
					end)
				end
				rows[#rows + 1] = hbox(row)
			end
		end
	end

	-- The companies (tpf3mp/companies.lua; DECISIONS.md D22, proposed), as
	-- cards: each with its colour, name, head, players, money and whether it
	-- has a password; yours first and marked. Switching is one click: Switch
	-- to on any other company (its password asked for first where it has
	-- one), Leave to the room's first company on yours, or Found a company
	-- of your own. Under the cards, what you may do with yours: rename and
	-- recolour it (any of its players), and for its head its password,
	-- its stations, and sending a player out. A company's own loans, for
	-- companies past the first, are the room's (the first company's are in
	-- the game's finance window).
	local function companyRows(rows, status, shared, drafts)
		local roster = shared.companies
		if not roster then return end
		local companies = require("tpf3mp.companies")
		local function line(t, class) rows[#rows + 1] = text(t, class) end
		local function button(label, tooltip, onClick)
			return builtin.Button{ meta = { tooltip = tooltip }, content = text(label), onClick = onClick }
		end
		local function redraw() shared.version = shared.version + 1 end
		-- A field for a draft, sent with `act` on Enter.
		local function field(draft, placeholder, secret, act)
			return builtin.TextInputField{
				placeholderText = placeholder,
				value = draft:get(),
				maxLength = 64,
				passwordMode = secret or nil,
				acceptOnFocusLoss = false,
				resetValueOnCancel = false,
				onTyping = function(t) draft:set(t) end,
				onCancel = redraw,
				onValueChange = function(t) act(t) end,
			}
		end
		local function blank(t) return type(t) ~= "string" or t:match("^%s*$") end

		local byId, companyOf, names = {}, {}, {}
		for _, p in ipairs(status.players or {}) do byId[p.id] = p end
		for _, m in ipairs(roster.members) do companyOf[m.player] = m.company end
		local mine
		for _, p in ipairs(status.players or {}) do
			local id = companyOf[p.id] or 0
			names[id] = names[id] or {}
			names[id][#names[id] + 1] = tostring(p.name) .. (p.me and " (you)" or "")
			if p.me then mine = id end
		end
		if mine == nil then mine = companyOf[status.me_id] or 0 end
		local first, myCompany
		for _, c in ipairs(roster.list) do
			if c.id == 0 then first = c end
			if c.id == mine then myCompany = c end
		end
		local ordered = { myCompany }
		for _, c in ipairs(roster.list) do if c.id ~= mine then ordered[#ordered + 1] = c end end
		local iHead = status.me_id ~= nil and companies.head(roster, mine) == status.me_id

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

		line("Companies", "font-scale-title-5")
		for _, c in ipairs(ordered) do
			local yours = c.id == mine
			local head = companies.head(roster, c.id)
			local about = {}
			if c.id == 0 then
				about[#about + 1] = "The room's first company: everyone's, no head"
			elseif head and byId[head] then
				about[#about + 1] = "Head: " .. tostring(byId[head].name)
			end
			about[#about + 1] = "Players: " .. (names[c.id] and table.concat(names[c.id], ", ") or "nobody")
			local marks = {}
			if c.balance ~= nil then
				marks[#marks + 1] = money(c.balance)
					.. ((type(c.owed) == "number" and c.owed > 0) and (", owes " .. money(c.owed)) or "")
			end
			if c.locked then marks[#marks + 1] = "Password" end
			if c.closed then marks[#marks + 1] = "Stations closed" end
			local card = {
				text(tostring(c.name) .. (yours and " · Your company" or ""), "font-scale-body"),
				text(table.concat(about, " · ")),
			}
			if #marks > 0 then card[#card + 1] = text(table.concat(marks, " · ")) end
			local actions = {}
			if yours then
				if c.id ~= 0 and first then
					actions[#actions + 1] = button("Leave to " .. tostring(first.name),
						"Play for the room's first company again", function() switchTo(first) end)
				end
				if c.id ~= 0 and #(names[c.id] or {}) <= 1 then
					-- Its last player dissolves it, once it owns nothing.
					actions[#actions + 1] = button("Dissolve", "Dissolve " .. tostring(c.name)
						.. " once it owns nothing, and play for the room's first company again",
						function() companyOp(shared, { Delete = c.id }, "Dissolving " .. c.name) end)
				end
			else
				if c.locked and drafts.prompt:get() == c.id then
					-- Its password, typed here; the room seals it, and only
					-- the seal reaches the games.
					actions[#actions + 1] = field(drafts.joinPassword, "Password of " .. tostring(c.name), true,
						function(t) if not blank(t) then switchTo(c, t) end end)
				end
				actions[#actions + 1] = button("Switch to " .. tostring(c.name),
					"Play for " .. tostring(c.name) .. " from now on" .. (c.locked and "; it needs its password" or ""),
					function() switchTo(c, drafts.joinPassword:get()) end)
			end
			-- The company's colour: yours to change, the others' to see.
			local swatch = builtin.ColorChooserButton{
				meta = { tooltip = yours and "Your company's colour: its vehicles wear it"
					or (tostring(c.name) .. "'s colour"), enabled = yours },
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
			rows[#rows + 1] = hbox({ swatch, column(card, 300), column(actions) })
		end

		-- A company of your own.
		local function found(t)
			if blank(t) then return end
			companyOp(shared, { Create = { name = t } }, "Founding " .. t)
			drafts.found:set("")
		end
		rows[#rows + 1] = hbox({
			field(drafts.found, "A company of your own", false, found),
			button("Found a company", "Found a company and play for it", function() found(drafts.found:get()) end),
		})

		-- What you may do with yours. The room's first company is
		-- everyone's: renamed in the game's own company window.
		if myCompany and mine ~= 0 then
			local c = myCompany
			line("Your company: " .. tostring(c.name), "font-scale-title-5")
			local function rename(t)
				if blank(t) then return end
				companyOp(shared, { Rename = { company = mine, name = t } }, "Renaming " .. tostring(c.name))
				drafts.rename:set("")
			end
			rows[#rows + 1] = hbox({
				field(drafts.rename, "A new name for " .. tostring(c.name), false, rename),
				button("Rename", "Rename the company you play for", function() rename(drafts.rename:get()) end),
			})
			if iHead then
				local function lock(t)
					if blank(t) then return end
					companyOp(shared, { Lock = c.id }, "Setting the password of " .. c.name, nil, t)
					drafts.lockPassword:set("")
				end
				local lockRow = {
					text("Password: "),
					field(drafts.lockPassword, c.locked and "A new password" or "A password to join", true, lock),
					button(c.locked and "Change" or "Set", "Only players who know it can join " .. tostring(c.name),
						function() lock(drafts.lockPassword:get()) end),
				}
				if c.locked then
					lockRow[#lockRow + 1] = button("Clear", "Let anyone join " .. tostring(c.name),
						function() companyOp(shared, { Unlock = c.id }, "Clearing the password of " .. c.name) end)
				end
				rows[#rows + 1] = hbox(lockRow)
				stationRows(rows, roster, c, shared)
				for _, player in ipairs(companies.members(roster, mine)) do
					local p = byId[player]
					if player ~= status.me_id and p then
						rows[#rows + 1] = hbox({
							text(tostring(p.name)),
							button("Send out", "Send " .. tostring(p.name) .. " back to the room's first company",
								function()
									companyOp(shared, { Dismiss = { company = c.id, player = player } },
										"Sending " .. tostring(p.name) .. " out of " .. c.name)
								end),
						})
					end
				end
			end
			local loans = {}
			for _, loan in ipairs(roster.loans or {}) do if loan.company == mine then loans[#loans + 1] = loan end end
			for _, loan in ipairs(loans) do
				rows[#rows + 1] = hbox({
					text("Loan: " .. money(loan.remaining) .. " owed of " .. money(loan.amount)
						.. ", " .. money(loan.payment) .. " a month, " .. (loan.months - loan.paid) .. " months left"),
					button("Repay", "Pay back what is still owed now", function()
						companyOp(shared, nil, "Repaying " .. money(loan.remaining), { Loan = { Repay = { loan = {
							type = "Custom", amount = loan.amount, duration = 1, percentage = 0, id = loan.id } } } })
					end),
				})
			end
			local offers = {}
			for _, offer in ipairs(roster.offers or {}) do
				if type(offer) == "table" and type(offer.amount) == "number" then
					offers[#offers + 1] = button("Borrow " .. money(offer.amount),
						string.format("Borrow %s at %g%% a year", money(offer.amount), (offer.percentage or 0) * 100),
						function()
							local terms = { type = offer.type, amount = offer.amount, duration = offer.duration,
								percentage = offer.percentage }
							companyOp(shared, nil, "Borrowing " .. money(offer.amount),
								{ Loan = { Take = { next = terms, offer = terms } } })
						end)
				end
			end
			if #offers > 0 then rows[#rows + 1] = hbox(offers) end
		end
		if shared.companyNote then line(shared.companyNote) end
	end

	-- One player as a strip: their banner (tpf3mp/banners.lua, the same the
	-- main menu's window shows), name, and marks: the room's owner, you,
	-- away, and how far their game is with the room's world.
	local function playerRow(p)
		local banners = require("tpf3mp.banners")
		local marks = {}
		if p.owner then marks[#marks + 1] = "Owner" end
		if p.me then marks[#marks + 1] = "You" end
		if not p.connected then marks[#marks + 1] = "Away" end
		marks[#marks + 1] = banners.stage(p, true)
		local sheet = sized(BANNER_W, BANNER_H)
		return hbox({
			builtin.ImageView{
				meta = sheet and { styleSheet = sheet } or {},
				path = banners.picture(banners.of(p)),
			},
			column({ text(tostring(p.name), "font-scale-body"), text(table.concat(marks, " · ")) }),
		})
	end

	-- The room page in the game: the room and its players with their
	-- banners, the companies (companyRows), Leave room, which asks first;
	-- the chat on the right.
	local function windowContent(status, draft, drafts, confirm)
		local shared = ui()
		local function send(t)
			local l = shared.link
			if not l or type(t) ~= "string" or t:match("^%s*$") then return end
			local ok, why = l:say(t)
			if ok then
				draft:set("")
			else
				shared.lines[#shared.lines + 1] = "(not sent: " .. tostring(why) .. ")"
			end
			shared.version = shared.version + 1
		end
		local left = {}
		local function line(t, class) left[#left + 1] = text(t, class) end
		line(tostring(status.room), "font-scale-title-4")
		local about = {}
		if status.speed then about[#about + 1] = "Speed " .. speedText(status.speed) end
		if status.diverged then
			about[#about + 1] = "Your world differed from the room's at step " .. tostring(status.diverged)
				.. "; the room's is on its way"
		else
			about[#about + 1] = "Worlds match"
		end
		line(table.concat(about, " · "))
		line("Players", "font-scale-title-5")
		for _, p in ipairs(status.players or {}) do left[#left + 1] = playerRow(p) end
		companyRows(left, status, shared, drafts)
		if shared.leaveNote then line(shared.leaveNote) end
		if confirm:get() then
			line("Leave the room? Your game stops following it; start the game again from the launcher for the next room.")
			left[#left + 1] = hbox({
				builtin.Button{ content = text("Leave"), onClick = function()
					confirm:set(false)
					local l = shared.link
					local ok, why = false, "not linked"
					if l then ok, why = l:leave() end
					shared.leaveNote = ok and "Leaving the room..." or ("Not left: " .. tostring(why))
					shared.version = shared.version + 1
				end },
				builtin.Button{ content = text("Stay"), onClick = function()
					confirm:set(false)
					shared.version = shared.version + 1
				end },
			})
		else
			left[#left + 1] = builtin.Button{
				meta = { tooltip = "Give up your seat in the room" },
				content = text("Leave room"),
				onClick = function()
					confirm:set(true)
					shared.version = shared.version + 1
				end,
			}
		end

		local chat = { text("Chat", "font-scale-title-5") }
		-- The newest lines only: the window sizes itself to what it holds.
		for i = math.max(1, #shared.lines - CHAT_SHOWN + 1), #shared.lines do chat[#chat + 1] = text(shared.lines[i]) end
		if #shared.lines == 0 then chat[#chat + 1] = text("Nobody said anything yet.") end
		chat[#chat + 1] = hbox({
			builtin.TextInputField{
				placeholderText = "Say something to the room",
				value = draft:get(),
				maxLength = 280,
				acceptOnFocusLoss = false,
				-- Clicking away keeps what was typed, and a redraw shows
				-- it: Send sends what the field shows, never a line the
				-- field dropped (it emptied itself on a cancel, build
				-- 40408, while Send still had the text).
				resetValueOnCancel = false,
				onTyping = function(t) draft:set(t) end,
				onCancel = function() shared.version = shared.version + 1 end,
				onValueChange = function(t) send(t) end,
			},
			builtin.Button{
				content = text("Send"),
				onClick = function() send(draft:get()) end,
			},
		})
		return hbox({ column(left, LEFT_W), column(chat, CHAT_W) })
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
					-- The company whose password Switch to asks for, or nil.
					prompt = react.useRef(nil) }
				react.onStep(function()
					local version = ui().version
					if version ~= drawn:old() then drawn:set(version) end
				end)
				local _ = drawn:old()
				local status = ui().status
				local content
				if status then
					content = windowContent(status, draft, drafts, confirmLeave)
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
			if ui().version ~= seen:get() then
				seen:set(ui().version)
				room:set(ui().version)
			end
			runPending()
			if link and guardedCmd then
				local delivered, why = pcall(function()
					local results = link:results()
					require("tpf3mp.guard").deliver(guardedCmd, results, sees)
					local shared = ui()
					for _, r in ipairs(results or {}) do
						local doing = shared.asked and r.ticket and shared.asked[r.ticket]
						if doing then
							shared.asked[r.ticket] = nil
							shared.companyNote = r.ok and (doing .. ": done")
								or (doing .. ": not done, " .. tostring(r.why))
							shared.version = shared.version + 1
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

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
-- The guard names vehicles, lines and station groups by the canonical ids
-- the mod's game script keeps in its state (tpf3mp/registry.lua).
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
-- launcher started; the lobby stays in the launcher). The window is a
-- second plugin (ModEntryPointExtension, tpf3mp_window.res.lua); what both
-- show is kept in package.loaded["tpf3mp.ui"], which the game bar plugin
-- fills every frame from the hook (tpf3mp/bridge.lua: status, chat, say).
function data()
	local MOD = "tpf3mp_1"
	-- Every module, in an order where each needs only those before it.
	local MODULES = { "geom", "roads", "engine", "registry", "capture", "bridge", "guard" }
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

	local function refused(kind, why)
		local name = kind or "command no factory made"
		local count = (refusals[name] or 0) + 1
		refusals[name] = count
		local changed = why ~= nil and why ~= reasons[name]
		if changed then reasons[name] = why end
		if count == 1 or count % 100 == 0 or changed then
			link:log("refused the player's " .. name .. " in the room's game ("
				.. count .. " so far)" .. (why and (": " .. tostring(why)) or ""))
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

	-- The mod's game script's registry (tpf3mp/registry.lua), read from the
	-- script's state as the game keeps it: game scripts are entities, named
	-- by their file (the loan window reads the loan script so).
	local SCRIPT_NAMES = { MOD .. "::/tpf3mp_sim/tpf3mp_sim.gs", MOD .. "::/tpf3mp_sim.gs" }
	local function registryNow()
		for _, name in ipairs(SCRIPT_NAMES) do
			local ok, state = pcall(function()
				local entity = api.engine.system.gameScriptSystem.getEntityForGameScript(name)
				if type(entity) ~= "number" or entity < 0 then return nil end
				local c = api.engine.getComponent(entity, api.type.ComponentType.GAME_SCRIPT)
				return c and c.state
			end)
			if ok and type(state) == "table" then return state.registry end
		end
		return nil
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
	end

	-- Does what the hook asks: saving the world under the name it gives, or
	-- loading the room's world from the game's save folder.
	local function serve()
		if not link then return end
		local request = link:poll()
		if not request then return end
		if request.save then
			local name = request.save
			local ok, err = pcall(app.saveGame, name, function()
				link:saved(name, true)
			end, false, true)
			if not ok then link:saved(name, false, tostring(err)) end
		elseif request.load then
			local ok, err = pcall(function()
				local id = api.type.SavegameId.new()
				id.path = ""
				id.saveGameName = request.load
				id.saveGameNamespace = app.SaveGameNamespace.getSavegame()
				app.loadGame(id, false, nil)
			end)
			if ok then
				link:log("loading the room's world")
			else
				link:log("loading the room's world failed: " .. tostring(err))
			end
		end
	end

	local react = ug_require "::/gui/main/react.lua"
	local builtin = ug_require "::/gui/main/builtin.lua"
	local game_bar_widgets = ug_require "::/gui/game_bar/game_bar_widgets.tl"
	local mod_entry_point = ug_require "::/gui/main/mod_entry_point.tl"

	-- Chat lines the window keeps, newest last.
	local CHAT_LINES = 50
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
		end
		for _, line in ipairs(link:chat()) do
			shared.lines[#shared.lines + 1] = tostring(line.from) .. ": " .. tostring(line.text)
			if #shared.lines > CHAT_LINES then table.remove(shared.lines, 1) end
			if not shared.open then shared.unread = shared.unread + 1 end
			changed = true
		end
		if changed then shared.version = shared.version + 1 end
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
				local ok, err = pcall(start)
				if not ok then say("start failed: " .. tostring(err)) end
			end
			local ok, err = pcall(serve)
			if not ok then say("serving the hook failed: " .. tostring(err)) end
			local followed, why = pcall(follow)
			if not followed then say("reading the room failed: " .. tostring(why)) end
			if ui().version ~= seen:get() then
				seen:set(ui().version)
				room:set(ui().version)
			end
			runPending()
			if link and guardedCmd then
				local delivered, why = pcall(function()
					require("tpf3mp.guard").deliver(guardedCmd, link:results(), sees)
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
				content = builtin.TextView{ text = label },
				onClick = function()
					shared.open = not shared.open
					shared.unread = 0
					shared.version = shared.version + 1
				end,
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

	-- The Multiplayer window: shown while ui().open, over the game.
	local Tpf3mpWindow = react.RegisterPluginRecipe(mod_entry_point.ModEntryPointExtension, "Tpf3mpWindow", function()
		local drawn = react.useState(0)
		local draft = react.useRef("")
		react.onStep(function()
			local version = ui().version
			if version ~= drawn:old() then drawn:set(version) end
		end)
		local shared = ui()
		local status = shared.status
		if not shared.open or not status then
			return builtin.BoxLayout{ orientation = builtin.type.Orientation.Vertical, children = {} }
		end
		local function send(text)
			local l = shared.link
			if not l or type(text) ~= "string" or text:match("^%s*$") then return end
			local ok, why = l:say(text)
			if ok then
				draft:set("")
			else
				shared.lines[#shared.lines + 1] = "(not sent: " .. tostring(why) .. ")"
			end
			shared.version = shared.version + 1
		end
		local rows = {}
		local function line(text) rows[#rows + 1] = builtin.TextView{ text = text } end
		line("Room: " .. tostring(status.room))
		if status.speed then line("Speed: " .. speedText(status.speed)) end
		if status.diverged then
			line("Your world differed from the room's at step " .. tostring(status.diverged)
				.. "; the room's is on its way")
		else
			line("Worlds match")
		end
		line("")
		line("Players")
		for _, p in ipairs(status.players or {}) do
			local tags = {}
			if p.owner then tags[#tags + 1] = "host" end
			if p.me then tags[#tags + 1] = "you" end
			if not p.connected then tags[#tags + 1] = "away" end
			line("  " .. tostring(p.name) .. (#tags > 0 and (" (" .. table.concat(tags, ", ") .. ")") or ""))
		end
		line("")
		line("Chat")
		local chat = {}
		for _, text in ipairs(shared.lines) do chat[#chat + 1] = builtin.TextView{ text = text } end
		if #chat == 0 then chat[1] = builtin.TextView{ text = "Nobody said anything yet." } end
		rows[#rows + 1] = builtin.ScrollArea{
			horizontalPolicy = builtin.type.ScrollBarPolicy.AlwaysOff,
			verticalPolicy = builtin.type.ScrollBarPolicy.Simple,
			content = builtin.Component{
				layout = builtin.BoxLayout{ orientation = builtin.type.Orientation.Vertical, children = chat },
			},
		}
		rows[#rows + 1] = builtin.BoxLayout{
			orientation = builtin.type.Orientation.Horizontal,
			children = {
				builtin.TextInputField{
					placeholderText = "Say something to the room",
					value = draft:get(),
					maxLength = 280,
					acceptOnFocusLoss = false,
					onTyping = function(text) draft:set(text) end,
					onValueChange = function(text) send(text) end,
				},
				builtin.Button{
					content = builtin.TextView{ text = "Send" },
					onClick = function() send(draft:get()) end,
				},
			},
		}
		return builtin.Window{
			title = "Multiplayer",
			closable = true,
			movable = true,
			initialX = 360,
			initialY = 140,
			onClose = function()
				shared.open = false
				shared.version = shared.version + 1
			end,
			content = builtin.Component{
				layout = builtin.BoxLayout{ orientation = builtin.type.Orientation.Vertical, children = rows },
			},
		}
	end)

	return {
		Tpf3mpPlugin = Tpf3mpPlugin,
		Tpf3mpWindow = Tpf3mpWindow,
	}
end

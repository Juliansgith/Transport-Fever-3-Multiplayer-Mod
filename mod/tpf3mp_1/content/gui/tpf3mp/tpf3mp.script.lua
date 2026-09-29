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
-- game a command the room cannot carry yet is refused, and the game bar says
-- so for a few seconds.
--
-- It follows what mods made for Transport Fever 3 build 40391 rely on
-- (investigation/TF3_MODS_2026-09-27.md): a .script.lua defines data();
-- ug_require loads the game's files ("::/...") and a mod's own
-- ("tpf3mp_1::/..."); a GameBarInfoDisplayExtension plugin with
-- react.onStep runs code every frame in a game; debugPrint writes to the
-- game's log. Each step logs "[tpf3mp]" lines, so the log shows how far a
-- game got on release day.
--
-- The plugin draws nothing but that notice yet. The Multiplayer panel
-- (docs/PLAN.md) goes here.
function data()
	local MOD = "tpf3mp_1"
	-- Every module, in an order where each needs only those before it.
	local MODULES = { "geom", "roads", "engine", "bridge", "guard" }
	-- Frames a refusal's notice stays in the game bar.
	local NOTICE_FRAMES = 360

	local function say(line)
		pcall(debugPrint, "[tpf3mp] " .. line)
	end

	-- The link to the hook, once a world's GUI has found it.
	local link = nil

	-- Callbacks of refused commands, for the next frame, as the game would
	-- call them.
	local pending = {}
	-- The notice of the last refusal, until the plugin shows it.
	local notice = nil
	-- Refusals so far, by kind, for the hook's log.
	local refusals = {}

	local function refused(kind, why)
		local name = kind or "command no factory made"
		local count = (refusals[name] or 0) + 1
		refusals[name] = count
		if count == 1 or count % 100 == 0 or why then
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

	-- Puts the guard in front of the GUI's commands.
	local function guardCommands()
		local ok, cmd = pcall(function() return api.cmd end)
		local wrapped, why = require("tpf3mp.guard").install(ok and cmd or nil, {
			inRoom = function() return link:room() end,
			command = function(action) return link:command(action) end,
			refused = refused,
			later = function(fn) pending[#pending + 1] = fn end,
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

	local Tpf3mpPlugin = react.RegisterPluginRecipe(game_bar_widgets.GameBarInfoDisplayExtension, "Tpf3mpPlugin", function()
		-- Once per game: the ref lives as long as this plugin is mounted.
		local started = react.useRef(false)
		-- The refusal notice shown, or false, and the frames it has left.
		local shown = react.useState(false)
		local frames = react.useRef(0)
		react.onStep(function()
			if not started:get() then
				started:set(true)
				local ok, err = pcall(start)
				if not ok then say("start failed: " .. tostring(err)) end
			end
			local ok, err = pcall(serve)
			if not ok then say("serving the hook failed: " .. tostring(err)) end
			runPending()
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
		if shown:old() then
			children[1] = builtin.TextView{ text = shown:old() }
		end
		return builtin.BoxLayout{
			orientation = builtin.type.Orientation.Horizontal,
			children = children,
		}
	end)

	return {
		Tpf3mpPlugin = Tpf3mpPlugin,
	}
end

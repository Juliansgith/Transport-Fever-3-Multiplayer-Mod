-- TPF3-MP in the game's GUI state: the plugin gui/tpf3mp/tpf3mp.res.lua
-- names. On the first step of a game it loads the mod's modules and links
-- to the hook (tpf3mp/bridge.lua). Without a hook, which is every game Steam
-- started, it logs one line and does nothing more. The room's actions are
-- applied by the mod's game script (tpf3mp_sim/), not here.
--
-- It follows what mods made for Transport Fever 3 build 40391 rely on
-- (investigation/TF3_MODS_2026-09-27.md): a .script.lua defines data();
-- ug_require loads the game's files ("::/...") and a mod's own
-- ("tpf3mp_1::/..."); a GameBarInfoDisplayExtension plugin with
-- react.onStep runs code every frame in a game; debugPrint writes to the
-- game's log. Each step logs "[tpf3mp]" lines, so the log shows how far a
-- game got on release day.
--
-- The plugin draws nothing yet. The Multiplayer panel (docs/PLAN.md) goes
-- here.
function data()
	local MOD = "tpf3mp_1"
	-- Every module, in an order where each needs only those before it.
	local MODULES = { "geom", "roads", "engine", "bridge" }

	local function say(line)
		pcall(debugPrint, "[tpf3mp] " .. line)
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
		local link, reason = bridge.attach(bridge.find())
		if not link then
			say(reason .. "; this is the plain game")
			return
		end
		link:log("the GUI is linked")
		say("linked to the hook")
	end

	local react = ug_require "::/gui/main/react.lua"
	local builtin = ug_require "::/gui/main/builtin.lua"
	local game_bar_widgets = ug_require "::/gui/game_bar/game_bar_widgets.tl"

	local Tpf3mpPlugin = react.RegisterPluginRecipe(game_bar_widgets.GameBarInfoDisplayExtension, "Tpf3mpPlugin", function()
		-- Once per game: the ref lives as long as this plugin is mounted.
		local started = react.useRef(false)
		react.onStep(function()
			if not started:get() then
				started:set(true)
				local ok, err = pcall(start)
				if not ok then say("start failed: " .. tostring(err)) end
			end
		end)
		-- An empty layout keeps the plugin mounted, so onStep keeps running.
		return builtin.BoxLayout{
			orientation = builtin.type.Orientation.Horizontal,
			children = {},
		}
	end)

	return {
		Tpf3mpPlugin = Tpf3mpPlugin,
	}
end

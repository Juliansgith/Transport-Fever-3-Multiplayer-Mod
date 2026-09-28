-- TPF3-MP in the game's GUI state: the plugin gui/tpf3mp/tpf3mp.res.lua
-- names. On the first step of a game it loads the mod's modules and links
-- to the hook (tpf3mp/bridge.lua). Without a hook, which is every game Steam
-- started, it logs one line and does nothing more.
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
	-- did; here each name loads its file through ug_require. Only names
	-- under "tpf3mp." are added, so nothing else in the state changes.
	local function installModules()
		if type(package) ~= "table" or type(package.preload) ~= "table" then
			return nil, "this Lua state has no package.preload"
		end
		for _, name in ipairs(MODULES) do
			local path = MOD .. "::/scripts/tpf3mp/" .. name .. ".lua"
			package.preload["tpf3mp." .. name] = function()
				return ug_require(path)
			end
		end
		return true
	end

	-- The hook's table (bridge.GLOBAL), if the hook registered one. Named
	-- directly, since a state need not have _G, and read through pcall: a
	-- state that refuses undeclared globals raises on a missing one.
	local function nativeTable()
		local ok, value = pcall(function() return tpf3mp_native end)
		if ok then return value end
		return nil
	end

	local function start()
		local ok, why = installModules()
		if not ok then
			say("not started: " .. why)
			return
		end
		for _, name in ipairs(MODULES) do
			local loaded, err = pcall(require, "tpf3mp." .. name)
			if not loaded then
				say("not started: tpf3mp." .. name .. " did not load: " .. tostring(err))
				return
			end
		end
		say("modules loaded")

		local bridge = require "tpf3mp.bridge"
		local link, reason = bridge.attach(nativeTable())
		if not link then
			say(reason .. "; this is the plain game")
			return
		end
		-- Nothing is applied by this version yet, so every event is refused:
		-- the hook then stops following the room rather than leave this
		-- game behind the others (fail closed).
		local registered, err = link:register({
			apply = function(_action)
				return false, "this version of the mod applies no actions yet"
			end,
			notice = function(kind, text)
				say("notice " .. tostring(kind) .. ": " .. tostring(text))
			end,
		})
		if not registered then
			say("the hook is here, but " .. err)
			return
		end
		link:log("the mod is linked")
		say("linked to the hook")
	end

	local react = ug_require "::/gui/main/react.lua"
	local builtin = ug_require "::/gui/main/builtin.lua"
	local game_bar_widgets = ug_require "::/gui/game_bar/game_bar_widgets.tl"

	local Tpf3mpPlugin = react.RegisterPluginRecipe(game_bar_widgets.GameBarInfoDisplayExtension, "Tpf3mpPlugin", function()
		-- Once per game: the ref lives as long as this plugin is mounted.
		local started = react.useRef(false)
		react.onStep(function()
			if started:get() then return end
			started:set(true)
			local ok, err = pcall(start)
			if not ok then say("start failed: " .. tostring(err)) end
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

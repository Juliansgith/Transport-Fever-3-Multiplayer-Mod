-- TPF3-MP in the GUI's other Lua state: the one the game renders its
-- React recipes in (the HUD's icons, the line manager's depots, the
-- construction menu), which is not the Multiplayer plugin's (build 40408;
-- docs/HOOKS.md, "The GUI's company"). gui/tpf3mp/gui_state.res.lua has the
-- game run prepare() there before any recipe renders, as it runs every
-- react-replacement-config; it replaces no recipe. There it:
--
-- - gives the state the GUI's "my company" (tpf3mp/follow.lua): the
--   company this player plays for, read from the hook (who this player
--   is) and the mod's game script's state (the roster), every 2 seconds;
-- - has the game's permit counts count the player's company's own
--   constructions while the room has more than one company
--   (tpf3mp/companies.lua, followPermits), so each company builds its own
--   headquarters;
-- - notes the stop the construction menu gives the stop tool, which the
--   room's stop capture reads (tpf3mp/capture.lua, capture.watchStopTool).
--
-- Without a hook (a game Steam started) it does nothing.
function data()
	local READ_EVERY = 2.0

	return {
		prepare = function(_replacementApi)
			local companies = ug_require "tpf3mp_1::/scripts/tpf3mp/companies.lua"
			local follow = ug_require "tpf3mp_1::/scripts/tpf3mp/follow.lua"
			local capture = ug_require "tpf3mp_1::/scripts/tpf3mp/capture.lua"
			local bridge = ug_require "tpf3mp_1::/scripts/tpf3mp/bridge.lua"
			local link = bridge.attach(bridge.find())
			if not link then return end

			local mine, several, readAt = nil, false, nil
			local function read()
				local ok, now = pcall(function() return os.clock() end)
				if not ok or readAt == nil or now - readAt >= READ_EVERY then
					readAt = ok and now or nil
					local status = link:status()
					local state = companies.scriptState(api)
					mine = follow.companyOf(state and state.companies, status and status.me_id)
					several = state ~= nil and type(state.companies) == "table"
						and type(state.companies.list) == "table" and #companies.live(state.companies) > 1
				end
			end
			local function myCompany() read() return mine end
			local followed, why = follow.install(api, myCompany)
			link:log(followed and "the GUI's company follows the player's in the HUD's state"
				or ("the GUI's company cannot follow the player's in the HUD's state: " .. tostring(why)))

			-- The construction menu's permits: each company's own
			-- constructions, with more than one (companies.followPermits).
			local counted, whyNot = companies.followPermits(api, ug_require, function() read() return several end)
			link:log(counted and ("the game's permits count each company's own constructions in the HUD's state ("
				.. counted .. " company_util table(s))")
				or ("the game's permits count the whole world's constructions in the HUD's state: " .. tostring(whyNot)))

			local watched = capture.watchStopTool(ug_require "::/gui/construction/construction_react_util.tl", link)
			link:log(watched and "the stop tool's stop is noted"
				or "the stop tool's stop is not noted: stops stay refused")
		end,
	}
end

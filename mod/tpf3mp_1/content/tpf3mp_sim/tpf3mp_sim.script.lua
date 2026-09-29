-- TPF3-MP's game script (tpf3mp_sim.gs.lua names it): where every game
-- applies the actions the room ordered, all in the same simulation update
-- (docs/HOOKS.md, "Actions in the game").
--
-- The game runs a game script's `update` once per simulation update, in an
-- engine state, where a command runs at once (build 40408, measured: the
-- update count goes up by one from call to call, dt 0.2). It runs game
-- scripts on a pool of Lua states, so what this file keeps is kept once per
-- state; the link to the hook is looked up in each.
--
-- In each update it asks the hook for the actions the room ordered for it
-- (`take`) and applies them (tpf3mp/apply.lua). The hook hands them only to
-- the first update of the step they were ordered for, the same update on
-- every game, and holds the world if nobody took them.
--
-- `handleEvent` takes the event `command` of id "tpf3mp" (sent with
-- api.cmd.makeScriptingSendEventCmd) and hands its parameter, an action
-- table, to the room: a way to act from the console, for tests. The event
-- reaches this game's scripts only, so only this game hands the action over;
-- the room then orders it for every game.
function data()
	local MOD = "tpf3mp_1"
	-- Per Lua state: tried once, then kept.
	local tried, link, apply = false, nil, nil

	local function linked()
		if not tried then
			tried = true
			local okBridge, bridge = pcall(ug_require, MOD .. "::/scripts/tpf3mp/bridge.lua")
			local okApply, applyModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/apply.lua")
			if okBridge and okApply and type(bridge) == "table" and type(applyModule) == "table" then
				link = bridge.attach(bridge.find())
				apply = applyModule
				if link then link:log("the game script is linked") end
			end
		end
		return link
	end

	return {
		update = function(_params, state, _dt)
			local l = linked()
			if not l then return end
			if state and state.hasEventSubscriptions and not state:hasEventSubscriptions() then
				state:subscribeToEvent("command")
			end
			local actions = l:take()
			if not actions then return end
			for i, action in ipairs(actions) do
				local ok, why = apply.run(action)
				if not ok then
					l:log("action " .. i .. " of this step was not applied: " .. tostring(why))
				end
			end
		end,

		handleEvent = function(_params, _state, _src, id, name, param)
			if id ~= "tpf3mp" or name ~= "command" then return end
			local l = linked()
			if not l then return end
			local ok, why = l:command(param)
			if ok then
				l:log("handed a test action to the room")
			else
				l:log("refused a test action: " .. tostring(why))
			end
		end,
	}
end

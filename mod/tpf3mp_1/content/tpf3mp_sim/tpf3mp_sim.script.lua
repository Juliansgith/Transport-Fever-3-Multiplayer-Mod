-- TPF3-MP's game script (tpf3mp_sim.gs.lua names it): where every game
-- applies the actions the room ordered, all in the same simulation update,
-- and reads the world's lanes at checkpoints (docs/HOOKS.md, "Actions in
-- the game" and "The world's lanes").
--
-- The game runs a game script's `update` once per simulation update, in an
-- engine state, where a command runs at once (build 40408, measured: the
-- update count goes up by one from call to call, dt 0.2). It runs game
-- scripts on a pool of Lua states, so what this file keeps is kept once per
-- state; the link to the hook is looked up in each. The game's own scripts
-- decide in `update` and act in `postUpdate`, which the game calls with
-- what `update` returned, and not when that is nil: company.script.tl
-- reads its argument unchecked, and a postUpdate after an update that
-- returned nothing never read a lane on build 40408. This one does the
-- same, so the world changes only in `postUpdate`:
--
-- - `update` asks the hook for the actions the room ordered (`take`), which
--   the hook hands only to the first update of the step they were ordered
--   for, the same update on every game, and whether this update ends a
--   batch at a checkpoint step (`checkpoint`). It returns both, or nil.
-- - `postUpdate` applies the actions (tpf3mp/apply.lua) and, at a
--   checkpoint, reads the world's lanes (tpf3mp/lanes.lua) and hands them
--   to the hook (`lanes`), which reports them to the room.
--
-- The hook holds the world if nobody took the actions, or if a checkpoint's
-- lanes did not come.
--
-- `guiHandleEvent` runs in the GUI's state, where the game's own build
-- tools (streets, tracks, stations and depots, stops, the bulldozer) tell
-- game scripts of every proposal they make (`builder.proposalCreate`), and
-- honour an error returned for it, as the game's company script does with
-- its permits. In the room's game every such proposal gets one, so nothing
-- is built with those tools until the room carries what they build
-- (docs/HOOKS.md, "The player's commands").
--
-- `handleEvent` takes the event `command` of id "tpf3mp" (sent with
-- api.cmd.makeScriptingSendEventCmd) and hands its parameter, an action
-- table, to the room: a way to act from the console, for tests. The event
-- reaches this game's scripts only, so only this game hands the action over;
-- the room then orders it for every game.
function data()
	local MOD = "tpf3mp_1"
	-- Per Lua state: tried once, then kept.
	local tried, link, apply, lanes = false, nil, nil, nil
	-- Lanes that could not be read, logged once per state.
	local told = false
	-- Events subscribed to from this state.
	local subscribed = false

	-- The events the script needs: its console event, and the build tools'
	-- proposals. Each by name, since a save may carry an older mod's
	-- subscriptions.
	local EVENTS = { "command", "builder.proposalCreate", "builder.proposalPrepareForApply" }

	-- What a build tool shows in the room's game.
	local REFUSED = "Not in multiplayer yet: building with this tool"

	local function linked()
		if not tried then
			tried = true
			local okBridge, bridge = pcall(ug_require, MOD .. "::/scripts/tpf3mp/bridge.lua")
			local okApply, applyModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/apply.lua")
			local okLanes, lanesModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/lanes.lua")
			if okBridge and okApply and okLanes and type(bridge) == "table"
				and type(applyModule) == "table" and type(lanesModule) == "table" then
				link = bridge.attach(bridge.find())
				apply = applyModule
				lanes = lanesModule
				if link then link:log("the game script is linked") end
			end
		end
		return link
	end

	return {
		update = function(_params, state, _dt)
			local l = linked()
			if not l then return nil end
			if not subscribed and state and state.subscribeToEvent then
				subscribed = true
				for _, event in ipairs(EVENTS) do state:subscribeToEvent(event) end
			end
			local actions = l:take()
			local checkpoint = l:checkpoint()
			if not actions and not checkpoint then return nil end
			return { actions = actions, checkpoint = checkpoint }
		end,

		postUpdate = function(_params, _state, _dt, work)
			local l = linked()
			if not l or type(work) ~= "table" then return end
			for i, action in ipairs(work.actions or {}) do
				local ok, why = apply.run(action)
				if not ok then
					l:log("action " .. i .. " of this step was not applied: " .. tostring(why))
				end
			end
			if work.checkpoint then
				local read, failed = lanes.read(api)
				if #failed > 0 and not told then
					told = true
					l:log("lanes read as err: " .. table.concat(failed, "; "))
				end
				local ok, why = l:lanes(read)
				if not ok then l:log("the lanes were not taken: " .. tostring(why)) end
			end
		end,

		guiHandleEvent = function(_params, _state, _guiState, _src, _id, name, _param)
			if name ~= "builder.proposalCreate" and name ~= "builder.proposalPrepareForApply" then
				return nil
			end
			local l = linked()
			if not l or not l:room() then return nil end
			return { errorMessages = { [REFUSED] = true } }
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

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
-- its permits (docs/HOOKS.md, "The build tools"). In the room's game:
--
-- - where the hook stops the player's builds (`clicks` is not nil), a
--   proposal of a tool the room carries (CAPTURE) is kept as the action it
--   makes (tpf3mp/capture.lua), marked with the clicks counted so far, and
--   builds nothing here: the hook answers false when the game applies it.
--   `guiUpdate` hands the room the one each click saw last, and the room
--   orders it for every game, this one included;
-- - every other proposal gets an error, so those tools build nothing.
--
-- `handleEvent` takes the event `command` of id "tpf3mp" (sent with
-- api.cmd.makeScriptingSendEventCmd) and hands its parameter, an action
-- table, to the room: a way to act from the console, for tests. The event
-- reaches this game's scripts only, so only this game hands the action over;
-- the room then orders it for every game.
function data()
	local MOD = "tpf3mp_1"
	-- Per Lua state: tried once, then kept.
	local tried, link, apply, lanes, capture, registry = false, nil, nil, nil, nil, nil
	-- Lanes that could not be read, and kinds the registry could not list,
	-- logged once per state.
	local told, toldRegistry = false, false
	-- Events subscribed to from this state.
	local subscribed = false

	-- The events the script needs: its console event, and the build tools'
	-- proposals. Each by name, since a save may carry an older mod's
	-- subscriptions.
	local EVENTS = { "command", "builder.proposalCreate", "builder.proposalPrepareForApply" }

	-- What a build tool shows in the room's game.
	local REFUSED = "Not in multiplayer yet: building with this tool"

	-- The tools whose builds the room carries, by the tool's id: the
	-- capture that makes each one's action.
	local CAPTURE = { constructionBuilder = "construction", streetBuilder = "street", trackBuilder = "track" }
	-- In the GUI: the last proposal seen at each count of the player's builds
	-- ({ action = t } or { why = text }), and the builds handed on so far.
	local snapshots, handled = {}, nil
	-- The last reason a proposal was refused for, and how many were logged.
	local refusedWhy, refusals = nil, 0

	local function linked()
		if not tried then
			tried = true
			local okBridge, bridge = pcall(ug_require, MOD .. "::/scripts/tpf3mp/bridge.lua")
			local okApply, applyModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/apply.lua")
			local okLanes, lanesModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/lanes.lua")
			local okCapture, captureModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/capture.lua")
			local okRegistry, registryModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/registry.lua")
			if okBridge and okApply and okLanes and okCapture and okRegistry and type(bridge) == "table"
				and type(applyModule) == "table" and type(lanesModule) == "table"
				and type(captureModule) == "table" and type(registryModule) == "table" then
				link = bridge.attach(bridge.find())
				apply = applyModule
				if link then
					local linked = link
					apply.log = function(line) linked:log(line) end
				end
				lanes = lanesModule
				capture = captureModule
				registry = registryModule
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
			-- The registry begins at the room's first update, the same in
			-- every game (tpf3mp/registry.lua).
			local saved = state and state.get and state:get()
			local begin = l:room() and (type(saved) ~= "table" or saved.registry == nil)
			if not actions and not checkpoint and not begin then return nil end
			return { actions = actions, checkpoint = checkpoint, begin = begin }
		end,

		postUpdate = function(_params, state, _dt, work)
			local l = linked()
			if not l or type(work) ~= "table" then return end
			if work.actions or work.begin then
				local saved = state:get()
				if type(saved) ~= "table" then saved = {} end
				local reg, _, failed = registry.sync(saved.registry)
				if #failed > 0 and not toldRegistry then
					toldRegistry = true
					l:log("the registry could not list " .. table.concat(failed, "; "))
				end
				-- The room's builds go through; the player's own the hook
				-- stops.
				if work.actions then l:replaying(true) end
				for i, action in ipairs(work.actions or {}) do
					local ok, why, made = apply.run(action, { registry = reg })
					-- What it made, bound at once, for the player who ordered
					-- it: as the game answered the command, else as the
					-- registry found it.
					local kind = ok and apply.CREATES[next(action)] or nil
					local fresh
					reg, fresh = registry.sync(reg, (kind and made) and { [kind] = { made } } or nil)
					local entity = kind and made or nil
					for _, f in ipairs((kind and not entity) and fresh or {}) do
						if f[1] == kind then entity = f[3] break end
					end
					if kind and not entity then
						l:log("action " .. i .. " of this step made no " .. kind .. " this game could name")
					end
					l:applied(i, ok, entity, why)
					if not ok then
						l:log("action " .. i .. " of this step was not applied: " .. tostring(why))
					end
				end
				if work.actions then l:replaying(false) end
				saved.registry = reg
				state:set(saved)
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

		guiHandleEvent = function(_params, _state, _guiState, _src, id, name, param)
			if name ~= "builder.proposalCreate" and name ~= "builder.proposalPrepareForApply" then
				return nil
			end
			local l = linked()
			if not l or not l:room() then return nil end
			local clicks = l:clicks()
			local kind = CAPTURE[id]
			if clicks ~= nil and kind ~= nil and type(param) == "table" then
				local ok, action, why = pcall(capture[kind], param[1])
				if not ok then action, why = nil, tostring(action) end
				if action == false then
					-- Nothing proposed yet: nothing to refuse, nothing to hand on.
					snapshots[clicks] = nil
					return nil
				end
				local shape
				do
					local described, text = pcall(capture.describe, param[1])
					if described and text ~= "" then shape = text end
				end
				if not action then
					-- The tool refuses it at once, so no click follows: the log
					-- has it when the reason changes, a few dozen times at most.
					if why ~= refusedWhy and refusals < 40 then
						refusedWhy, refusals = why, refusals + 1
						l:log("the room cannot carry this " .. id .. " build: " .. tostring(why)
							.. (shape and (" [" .. shape .. "]") or ""))
					end
				end
				snapshots[clicks] = { action = action, why = why, shape = shape }
				if action then return nil end
				return { errorMessages = { ["Not in multiplayer yet: " .. tostring(why)] = true } }
			end
			-- A tool the room does not carry: its proposals' shapes, for the
			-- log, when they change, a few dozen times at most.
			if refusals < 40 and type(param) == "table" and capture then
				local described, text = pcall(capture.describe, param[1])
				local key = tostring(id) .. " " .. (described and text or "")
				if described and text ~= "" and key ~= refusedWhy then
					refusedWhy, refusals = key, refusals + 1
					l:log("the room does not carry the " .. tostring(id) .. " tool yet [" .. text .. "]")
				end
			end
			return { errorMessages = { [REFUSED] = true } }
		end,

		guiUpdate = function(_params, _state, _guiState)
			local l = linked()
			if not l then return end
			local clicks = l:clicks()
			if clicks == nil then return end
			if handled == nil then handled = clicks end
			while handled < clicks do
				local seen = snapshots[handled]
				if seen and seen.action then
					local ok, why = l:command(seen.action)
					if ok then
						l:log("handed the player's build to the room"
							.. (seen.shape and (" [" .. seen.shape .. "]") or ""))
					else
						l:log("the player's build was not handed to the room: " .. tostring(why))
					end
				else
					l:log("stopped a build the room cannot carry: "
						.. tostring(seen and seen.why or "no proposal seen")
						.. ((seen and seen.shape) and (" [" .. seen.shape .. "]") or ""))
				end
				snapshots[handled] = nil
				handled = handled + 1
			end
			for count in pairs(snapshots) do
				if count < handled then snapshots[count] = nil end
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

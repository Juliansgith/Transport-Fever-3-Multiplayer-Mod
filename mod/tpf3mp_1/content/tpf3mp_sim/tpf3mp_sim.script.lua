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
--   to the hook (`lanes`), which reports them to the room. When the hook
--   asks (`dump`: after a divergence, or TPF3MP_HOOK_LANE_DUMP), it also
--   hands it the lanes asked for entry by entry, for hook.log (docs/HOOKS.md,
--   "Lane dumps").
--
-- The hook holds the world if nobody took the actions, or if a checkpoint's
-- lanes did not come.
--
-- `guiHandleEvent` runs in the GUI's state, where the game's own build
-- tools (streets, tracks, stations and depots, stops on streets, the
-- bulldozer) tell game scripts of every proposal they make
-- (`builder.proposalCreate`), and
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
-- The module editor tells game scripts nothing on build 40408 (CAPTURE):
-- the hook reads its build natively at the click, and `guiUpdate` takes it
-- for that click (tpf3mp_native.built), ahead of any preview, and makes the
-- edit of it as of the construction tool's proposal. The terrain tools
-- tell them nothing either: the hook reads a stroke's height grid at the
-- click the same way, and `guiUpdate` hands the room Terraform actions of
-- it (tpf3mp/capture.lua terraform), which every game applies through the
-- hook (tpf3mp/apply.lua). A click with neither is stopped with "no
-- proposal seen".
--
-- Each upgrade (a road or track modifier's build) and each terraform is
-- said in the hook's log when handed to the room and when applied. Every event of the room's game the
-- script does not handle is logged by id and name, once each, a few dozen
-- at most.
--
-- `handleEvent` takes the event `command` of id "tpf3mp" (sent with
-- api.cmd.makeScriptingSendEventCmd) and hands its parameter, an action
-- table, to the room: a way to act from the console, for tests. The event
-- reaches this game's scripts only, so only this game hands the action over;
-- the room then orders it for every game.
--
-- With more than one company in the room, it samples the companies'
-- scores four times a game month, the same game time in every game, and
-- keeps their ranks (tpf3mp/progression.lua, docs/HOOKS.md "Company
-- ranks"), each town's parts and each score said in the hook's log.
--
-- It also hears the company script's `startProspection` and
-- `endProspection` (game_mechanics/company/company.script.tl), which every
-- game's company script sends at the same update, and says in the hook's
-- log when a prospection began and what it found. A prospection that found
-- an industry binds it in the registry at once, so every game names it by
-- the same id (docs/HOOKS.md, "Prospecting").
function data()
	local MOD = "tpf3mp_1"
	-- Per Lua state: tried once, then kept.
	local tried, link, apply, lanes, capture, registry, companies, progression =
		false, nil, nil, nil, nil, nil, nil, nil
	-- Lanes that could not be read, and kinds the registry could not list,
	-- logged once per state.
	local told, toldRegistry = false, false
	-- Events subscribed to from this state.
	local subscribed = false
	-- Says what a prospection did (below).
	local prospected

	-- The events the script needs: its console event, and the build tools'
	-- proposals. Each by name, since a save may carry an older mod's
	-- subscriptions.
	local EVENTS = { "command", "builder.proposalCreate", "builder.proposalPrepareForApply",
		"startProspection", "endProspection" }

	-- What a build tool shows in the room's game.
	local REFUSED = "Not in multiplayer yet: building with this tool"

	-- The tools whose builds the room carries, by the tool's id: the
	-- capture that makes each one's action. On build 40408 the game tells
	-- game scripts of the proposals of six tools only, each under the id
	-- the game's GUI names it by (UI::CGameUI's constructor, read from the
	-- binary): constructionBuilder, streetTerminalBuilder, streetBuilder,
	-- trackBuilder, streetTrackModifier (the road and track modifiers: tram
	-- tracks, bus lanes, barriers, trees, a street or track type, catenary)
	-- and bulldozer. The module editor (UI::ModuleBuilder) tells them
	-- nothing there. moduleBuilder and moduleBulldozer are its names in the
	-- construction menu's parameters (ConstructionActionParam); INFERRED
	-- that a later build would send its proposals under them.
	local CAPTURE = { constructionBuilder = "construction", streetBuilder = "street", trackBuilder = "track",
		bulldozer = "bulldoze", streetTerminalBuilder = "stop", moduleBuilder = "construction",
		moduleBulldozer = "bulldoze", streetTrackModifier = "modify" }
	-- In the GUI: the last proposal seen at each count of the player's builds
	-- ({ action = t } or { why = text }), and the builds handed on so far.
	local snapshots, handled = {}, nil
	-- What the log said of the tools the room does not carry, by tool and change.
	local toolsLogged = nil
	-- The last reason a proposal was refused for, and how many were logged.
	local refusedWhy, refusals = nil, 0
	-- The events of the room's game the mod does not handle, by id and
	-- name, logged once each, a few dozen at most: what reaches the script
	-- when a tool's build is "no proposal seen".
	local unhandled, unhandledCount = {}, 0
	local function note(l, id, name)
		local key = tostring(id) .. " " .. tostring(name)
		if unhandled[key] or unhandledCount >= 40 then return end
		unhandled[key], unhandledCount = true, unhandledCount + 1
		l:log("an event the mod does not handle: id " .. tostring(id) .. ", name " .. tostring(name))
	end

	-- The snapshot of a module editor's click, from its proposal as the hook
	-- read it (tpf3mp_native.built) or why that did not read: the edit the
	-- construction tool's capture makes of it, which must replace the
	-- construction edited.
	local function moduleEdit(proposal, why)
		local shape = "module editor"
		if proposal == nil then
			return { why = "the module editor's edit did not read: " .. tostring(why), shape = shape }
		end
		local removes = type(proposal.toRemove) == "table" and #proposal.toRemove > 0
		local ok, action, whyNot = true, nil, "an edit that replaces no construction"
		if removes then ok, action, whyNot = pcall(capture.construction, proposal) end
		if not ok then action, whyNot = nil, tostring(action) end
		if action and action.BuildConstruction.replaces == nil then
			action, whyNot = nil, "an edit that replaces no construction"
		end
		if not action then return { why = "the module editor's edit: " .. tostring(whyNot), shape = shape } end
		return { action = action, shape = shape }
	end

	-- The snapshot of a terrain tool's click, from its stroke as the hook read
	-- it (tpf3mp_native.built): its Terraform actions, one a band of rows.
	local function terraformEdit(built)
		local actions, why = capture.terraform(built)
		if not actions then return { why = "the terrain tool's stroke: " .. tostring(why), shape = "terrain tool" } end
		local said = {}
		for i, a in ipairs(actions) do
			said[i] = "terraform handed to the room: " .. capture.terraformSummary(a.Terraform)
				.. (#actions > 1 and (" (part " .. i .. " of " .. #actions .. ")") or "")
		end
		return { actions = actions, said = said, shape = "terrain tool" }
	end

	-- The guard on what this player's personal mods' game scripts send, in
	-- this state (tpf3mp/modguard.lua): put on once the link is.
	local function guardPersonalMods(companiesModule, registryModule)
		local okModule, modguard = pcall(ug_require, MOD .. "::/scripts/tpf3mp/modguard.lua")
		local okCmd, cmd = pcall(function() return api.cmd end)
		if not okModule or type(modguard) ~= "table" or not okCmd then
			link:log("the personal mods' guard is not on: " .. tostring(modguard))
			return
		end
		if type(debug) ~= "table" or type(debug.getinfo) ~= "function" then
			-- Without the stack no command can be told to be a personal
			-- mod's: fail closed is not possible here, so say it loudly
			-- when this player has any.
			if next(link:personal()) ~= nil then
				link:log("the personal mods' guard is not on: this state has no debug.getinfo, "
					.. "so a personal mod's game script would act in this game alone")
			end
			return
		end
		-- This player's personal mods, read again every so often: the room's
		-- lists come with its Begin, perhaps after this state linked.
		local personal, reads = {}, 0
		local function isPersonal(mod)
			reads = reads - 1
			if reads <= 0 then
				personal, reads = link:personal(), 200
			end
			return personal[mod] == true
		end
		local function registryNow()
			local state = companiesModule.scriptState(api)
			return state and state.registry
		end
		local function idOf(kind)
			return function(entity) return registryModule.id(registryNow(), kind, entity) end
		end
		-- The company this player acts for: theirs in the roster, else the
		-- game's player.
		local function myCompany()
			local state = companiesModule.scriptState(api)
			local roster = state and state.companies
			local status = link:status()
			local me = status and status.me_id
			if roster and me then
				for _, m in ipairs(roster.members or {}) do
					if m.player == me then
						for _, c in ipairs(roster.list or {}) do
							if c.id == m.company then return c.entity, roster end
						end
					end
				end
			end
			local ok, player = pcall(function() return api.engine.util.getPlayer() end)
			return ok and player or nil, roster
		end
		local wrapped, why = modguard.install(cmd, {
			inRoom = function() return link:room() end,
			personal = isPersonal,
			command = function(action) return link:command(action) end,
			context = { vehicle = idOf("vehicles"), line = idOf("lines"), group = idOf("groups"),
				town = idOf("towns") },
			mayTouch = function(entity)
				local company, roster = myCompany()
				return companiesModule.mayTouch(roster, company, entity, api, "thing")
			end,
			now = function()
				local ok, t = pcall(function()
					return api.engine.getComponent(api.engine.util.getWorld(),
						api.type.ComponentType.GAME_TIME).gameTime
				end)
				return ok and t or 0
			end,
			log = function(line) link:log(line) end,
		})
		if not wrapped then link:log("the personal mods' guard is not on: " .. tostring(why)) end
	end

	local function linked()
		if not tried then
			tried = true
			local okBridge, bridge = pcall(ug_require, MOD .. "::/scripts/tpf3mp/bridge.lua")
			local okApply, applyModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/apply.lua")
			local okLanes, lanesModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/lanes.lua")
			local okCapture, captureModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/capture.lua")
			local okRegistry, registryModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/registry.lua")
			local okCompanies, companiesModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/companies.lua")
			local okProgression, progressionModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/progression.lua")
			if okBridge and okApply and okLanes and okCapture and okRegistry and okCompanies and okProgression
				and type(companiesModule) == "table" and type(progressionModule) == "table" and type(bridge) == "table"
				and type(applyModule) == "table" and type(lanesModule) == "table"
				and type(captureModule) == "table" and type(registryModule) == "table" then
				link = bridge.attach(bridge.find())
				apply = applyModule
				if link then
					local linked = link
					apply.log = function(line) linked:log(line) end
					-- A terraform's grid goes to the hook (tpf3mp/apply.lua).
					apply.terrain = function(grid) return linked:terrain(grid) end
				end
				lanes = lanesModule
				capture = captureModule
				registry = registryModule
				companies = companiesModule
				progression = progressionModule
				if link then
					link:log("the game script is linked")
					guardPersonalMods(companiesModule, registryModule)
				end
			end
		end
		return link
	end

	-- The entity an event names: a number, or the game's { entity = }.
	local function entityIn(value)
		if type(value) == "table" then value = value.entity end
		if type(value) == "number" then return value end
		return nil
	end

	-- A town as the room names it, for the log.
	local function townName(reg, value)
		local e = entityIn(value)
		local id = e and registry.id(reg, "towns", e)
		if id then return "town-" .. id end
		return "town entity " .. tostring(e)
	end

	-- The company script's prospection events, in the room's game: said in
	-- the log, and a found industry bound in the registry.
	function prospected(state, name, param)
		local l = linked()
		if not l or not l:room() or type(param) ~= "table" then return end
		local saved = state and state.get and state:get()
		if type(saved) ~= "table" or saved.registry == nil then return end
		local cargo = tostring(param.cargoType)
		local began = tostring(param.initiatedTimestamp)
		if name == "startProspection" then
			l:log("prospecting began: " .. cargo .. " near " .. townName(saved.registry, param.entity)
				.. " at game time " .. began)
			return
		end
		local where = cargo .. " near " .. townName(saved.registry, param.entity) .. ", begun at game time " .. began
		if param.success ~= true then
			l:log("prospecting ended: " .. where .. ", found nothing")
			return
		end
		local reg, fresh = registry.sync(saved.registry)
		saved.registry = reg
		state:set(saved)
		local found = {}
		for _, f in ipairs(fresh) do
			if f[1] == "industries" then
				local text = "industry-" .. f[2]
				local ok, c = pcall(api.engine.getComponent, f[3], api.type.ComponentType.CONSTRUCTION)
				if ok and type(c) == "table" and c.transf then
					text = text .. string.format(" %s at (%.1f, %.1f)", tostring(c.fileName), c.transf[13], c.transf[14])
				end
				found[#found + 1] = text
			end
		end
		if #found == 0 then
			l:log("prospecting ended: " .. where .. ", found an industry this game could not name")
		else
			l:log("prospecting ended: " .. where .. ", found " .. table.concat(found, "; "))
		end
	end

	return {
		update = function(_params, state, _dt)
			local l = linked()
			if not l then return nil end
			-- The room step's seed for this state's math.random, the same in
			-- every game at the same step (crates/tpf3mp-hook/src/seeds.rs).
			local seed = l:seed()
			if seed then math.randomseed(seed) end
			if not subscribed and state and state.subscribeToEvent then
				subscribed = true
				for _, event in ipairs(EVENTS) do state:subscribeToEvent(event) end
			end
			local actions, origins, seals = l:take()
			local checkpoint = l:checkpoint()
			-- The registry begins at the room's first update, the same in
			-- every game (tpf3mp/registry.lua), or at the first update since
			-- the registry gained a kind.
			local saved = state and state.get and state:get()
			local begin = l:room() and (type(saved) ~= "table" or registry.incomplete(saved.registry))
			-- A month begun since the companies' loans were last charged.
			local month = companies.monthNow(api)
			local monthly = l:room() and type(saved) == "table" and companies.due(saved.companies, month)
			-- A quarter of a month begun since the companies' scores were
			-- last sampled, with more than one company (tpf3mp/progression.lua).
			local quarter = progression.quarterNow(api)
			local sample = l:room() and progression.due(saved, quarter)
			if not actions and not checkpoint and not begin and not monthly and not sample then return nil end
			return { actions = actions, origins = origins, seals = seals, checkpoint = checkpoint,
				begin = begin, monthly = monthly and month or nil, sample = sample and quarter or nil }
		end,

		postUpdate = function(_params, state, _dt, work)
			local l = linked()
			if not l or type(work) ~= "table" then return end
			if work.actions or work.begin or work.monthly or work.sample then
				local saved = state:get()
				if type(saved) ~= "table" then saved = {} end
				local reg, _, failed = registry.sync(saved.registry)
				-- The room's companies: begun at its first update, as the
				-- registry, the same in every game (tpf3mp/companies.lua).
				local roster = companies.ensure(saved.companies, api)
				-- The companies' ranks (tpf3mp/progression.lua).
				local prog = progression.ensure(saved.progression)
				if #failed > 0 and not toldRegistry then
					toldRegistry = true
					l:log("the registry could not list " .. table.concat(failed, "; "))
				end
				-- The room's builds go through; the player's own the hook
				-- stops.
				if work.actions then l:replaying(true) end
				for i, action in ipairs(work.actions or {}) do
					-- Booked to the sender's company.
					local player = work.origins and work.origins[i]
					local company = player and companies.of(roster, player)
					-- The seal of the password sent with it (a company's),
					-- which the room made; never the password.
					local seal = work.seals and work.seals[i] or nil
					local ok, why, made = apply.run(action, {
						registry = reg,
						roster = roster,
						player = player,
						company = company and company.entity,
						company = company and company.entity,
						progression = prog,
						seal = type(seal) == "table" and seal or nil,
					})
					local name = next(action)
					-- What it changed keeps its id on whatever entity it is
					-- now, bound before the sync would retire it.
					local keeps = ok and apply.KEEPS[name] or nil
					if keeps then
						local id = type(action[name]) == "table" and action[name][keeps.field]
						if made then
							registry.rebind(reg, keeps.kind, id, made)
						else
							l:log("action " .. i .. " of this step left " .. keeps.kind .. " " .. tostring(id)
								.. " as nothing this game could name")
						end
					end
					-- What it made, bound at once, for the player who ordered
					-- it: as the game answered the command, else as the
					-- registry found it.
					local kind = ok and apply.CREATES[name] or nil
					local fresh
					reg, fresh = registry.sync(reg, (kind and made) and { [kind] = { made } } or nil)
					local entity = (kind or keeps) and made or nil
					for _, f in ipairs((kind and not entity) and fresh or {}) do
						if f[1] == kind then entity = f[3] break end
					end
					if kind and not entity then
						l:log("action " .. i .. " of this step made no " .. kind .. " this game could name")
					end
					l:applied(i, ok, entity, why)
					if not ok then
						local okAbout, about = pcall(apply.about, action)
						l:log("action " .. i .. " of this step (" .. (okAbout and about or tostring(name))
							.. ") was not applied: " .. tostring(why))
					elseif name == "CompanyOp" then
						-- What became of the room's companies, for the log: the
						-- operation, whose, and whether a seal came with it;
						-- never a seal itself.
						local op = next(action.CompanyOp)
						l:log("company: " .. tostring(op) .. " by " .. tostring(player):sub(1, 8)
							.. (seal and " (with a password's seal)" or "") .. ": "
							.. companies.describe(roster))
					end
				end
				if work.actions then l:replaying(false) end
				if work.monthly then
					local ok, why = pcall(companies.chargeMonths, roster, work.monthly, apply.send, api)
					if not ok then l:log("the companies' loans were not charged: " .. tostring(why)) end
				end
				if work.sample then
					local ok, why = progression.sample(prog, roster, api, work.sample,
						function(line) l:log(line) end, reg, registry)
					if not ok then l:log("the companies' scores were not sampled: " .. tostring(why)) end
				end
				saved.registry = reg
				saved.companies = roster
				saved.progression = prog
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
				local dump = l:dump()
				if dump then
					local saved = state and state.get and state:get()
					local reg = type(saved) == "table" and saved.registry or nil
					-- Every entry is handed over: the hook keeps the first
					-- few thousand and counts the rest.
					for _, lane in ipairs(dump.lanes) do
						for _, entry in ipairs(lanes.dump(api, lane, reg)) do l:dumped(lane, entry) end
					end
				end
			end
		end,

		guiHandleEvent = function(_params, _state, _guiState, _src, id, name, param)
			if name ~= "builder.proposalCreate" and name ~= "builder.proposalPrepareForApply" then
				local l = linked()
				if l and l:room() then note(l, id, name) end
				return nil
			end
			local l = linked()
			if not l or not l:room() then return nil end
			local clicks = l:clicks()
			local kind = CAPTURE[id]
			if kind == nil then note(l, id, name) end
			if clicks ~= nil and kind ~= nil and type(param) == "table" then
				-- The link, for what the GUI's windows noted (the stop tool's stop).
				local ok, action, why = pcall(capture[kind], param[1], l)
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
				-- An upgrade tool's build, for the log (tpf3mp/roads.lua).
				local upgrade = action and kind == "modify" and capture.upgradeSummary(action) or nil
				snapshots[clicks] = { action = action, why = why, shape = shape, upgrade = upgrade }
				if action then return nil end
				return { errorMessages = { ["Not in multiplayer yet: " .. tostring(why)] = true } }
			end
			-- A tool the room does not carry: its proposals' shapes, for the
			-- log, when they change, a few dozen times at most.
			if type(param) == "table" and capture then
				local described, text = pcall(capture.describe, param[1])
				local diffed, diff = pcall(capture.rebuildDiff, param[1])
				local tool = l.note and l:note(capture.TOOL_NOTE) or "?"
				-- Once for each tool and what it changes, a few dozen at most.
				local key = tostring(id) .. " " .. tostring(tool) .. " " .. tostring(diff)
				toolsLogged = toolsLogged or {}
				if described and text ~= "" and not toolsLogged[key] and refusals < 80 then
					toolsLogged[key], refusals = true, refusals + 1
					l:log("the room does not carry the " .. tostring(id) .. " tool yet (" .. tostring(tool) .. ") [" .. text .. "]")
					if diffed and diff ~= "" then
						local n = 0
						for part in (diff .. "; "):gmatch("(.-); ") do
							n = n + 1
							if n <= 12 then l:log("  what it changes: " .. part) end
						end
					elseif not diffed then
						l:log("  what it changes: " .. tostring(diff))
					end
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
				-- The module editor's click: its build as the hook read it,
				-- whatever preview another tool showed before.
				local native, whyNot = l:built(handled)
				if type(native) == "table" and native.terrain ~= nil then
					seen = terraformEdit(native)
				elseif native == nil and type(whyNot) == "string" and whyNot:find("terrain tool: ", 1, true) == 1 then
					-- The painter's, the asset brush's, or a stroke that did not read.
					seen = { why = whyNot, shape = "terrain tool" }
				elseif native ~= nil or whyNot ~= nil then
					seen = moduleEdit(native, whyNot)
				end
				if seen and seen.actions then
					for i, action in ipairs(seen.actions) do
						local ok, why = l:command(action)
						if ok then
							l:log(seen.said[i])
						else
							l:log("the player's terraform was not handed to the room: " .. tostring(why))
							break
						end
					end
				elseif seen and seen.action then
					local ok, why = l:command(seen.action)
					if ok then
						l:log("handed the player's build to the room"
							.. (seen.shape and (" [" .. seen.shape .. "]") or ""))
						if seen.upgrade then l:log("upgrade handed to the room: " .. seen.upgrade) end
					else
						l:log("the player's build was not handed to the room: " .. tostring(why))
					end
				else
					l:log("stopped a build the room cannot carry: "
						.. tostring(seen and seen.why or "no proposal seen (a tool that tells game scripts nothing)")
						.. ((seen and seen.shape) and (" [" .. seen.shape .. "]") or ""))
				end
				snapshots[handled] = nil
				handled = handled + 1
			end
			for count in pairs(snapshots) do
				if count < handled then snapshots[count] = nil end
			end
		end,

		handleEvent = function(_params, state, _src, id, name, param)
			if id == "Company" and (name == "startProspection" or name == "endProspection") then
				prospected(state, name, param)
				return
			end
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

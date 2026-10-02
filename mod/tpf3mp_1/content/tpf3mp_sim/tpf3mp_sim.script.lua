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
-- With the hook's edge watch on (TPF3MP_HOOK_EDGE_WATCH), `update` also
-- asks which entities to read (`edgewatch`), and `postUpdate` reads each
-- after everything else it does and hands it over (`edgewatched`;
-- docs/HOOKS.md, "The edge watch"). Read only.
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
-- hook (tpf3mp/apply.lua) once tpf3mp/acceptance.lua's `terraform` is on.
-- A click with neither is stopped with "no proposal seen".
--
-- Each upgrade (a road or track modifier's build) and each terraform is
-- said in the hook's log when handed to the room and when applied. Every
-- event of the room's game the script does not handle is logged by id and
-- name, once each, a few dozen at most.
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
	local tried, link, apply, lanes, capture, registry, companies, progression, modbuild =
		false, nil, nil, nil, nil, nil, nil, nil, nil
	-- Whether this state is applying the room's actions (in postUpdate);
	-- the scripts' follow-up builds in the GUI's state, and whether their
	-- wrapper is on there (tpf3mp/modbuild.lua).
	local applying, followUps, followUpsOn = false, nil, false
	-- Lanes that could not be read, and kinds the registry could not list,
	-- logged once per state.
	local told, toldRegistry, toldOwnership = false, false, false
	-- The headquarters lines last logged in this state, by company id: a
	-- line is logged again only when it changed (tpf3mp/companies.lua).
	local toldHeadquarters = {}
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
		moduleBulldozer = "bulldoze", streetTrackModifier = "modify", laneModifier = "junction",
		crosswalkModifier = "junction", streetEdgeNodeModifier = "junction" }
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
		if proposal and proposal.junctionEdit then
			local ok, action, whyNot = pcall(capture.junction, proposal)
			if not ok then action, whyNot = nil, tostring(action) end
			return { action = action or nil, why = whyNot or "an empty junction edit", shape = "junction tool" }
		end
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

	-- In the GUI state this script's GUI half runs in, where the game's
	-- company script checks a construction's permits for the player
	-- (company.script.tl, builder.proposalCreate: an error and skipRender,
	-- so the tool shows no preview and builds nothing): the player's company
	-- answers getPlayer there, its rank the game's rank windows, and its own
	-- constructions the permit counts (tpf3mp/follow.lua,
	-- tpf3mp/progression.lua, tpf3mp/companies.lua), as in the GUI's other
	-- states. Without them a founded company's headquarters showed no preview
	-- and was never placed (2026-10-01): the game's company script asked the
	-- save's player's rank and counted every company's headquarters. Once
	-- this Lua state; each piece is a no-op where another of the GUI's
	-- states sharing its tables put it on first. Only ever in a GUI state:
	-- guiHandleEvent runs nowhere else.
	local guiFollowed = false
	local function followInGui(l)
		if guiFollowed then return end
		guiFollowed = true
		local okFollow, follow = pcall(ug_require, MOD .. "::/scripts/tpf3mp/follow.lua")
		local mine, several, readAt = nil, false, nil
		local function read()
			local ok, now = pcall(function() return os.clock() end)
			if not ok or readAt == nil or now - readAt >= 2.0 then
				readAt = ok and now or nil
				local status = l:status()
				local state = companies.scriptState(api)
				mine = okFollow and follow.companyOf(state and state.companies, status and status.me_id) or nil
				several = state ~= nil and type(state.companies) == "table"
					and type(state.companies.list) == "table" and #companies.live(state.companies) > 1
			end
		end
		local parts = {}
		if okFollow and type(follow) == "table" then
			local ok, why = follow.install(api, function() read() return mine end,
				function(line) l:log(line .. " (the game scripts' GUI state)") end)
			follow.install(api, follow.noteSource(l))
			pcall(follow.watchLines, api, ug_require, l, "the game scripts' GUI state")
			parts[#parts + 1] = ok and "getPlayer follows the player's company" or ("getPlayer stays the game's: " .. tostring(why))
		else
			parts[#parts + 1] = "getPlayer stays the game's: tpf3mp/follow.lua did not load"
		end
		local ranked, whyRanks = progression.follow(function() return companies.scriptState(api) end)
		companies.followStations(api, ug_require, function()
			if not l:room() then return end
			local state, status = companies.scriptState(api), l:status()
			return state and state.companies, status and status.me_id
		end)
		parts[#parts + 1] = ranked and "ranks are each company's" or ("ranks are the game's: " .. tostring(whyRanks))
		local counted, whyPermits = companies.followPermits(api, ug_require, function() read() return several end)
		parts[#parts + 1] = counted and "permits count each company's own constructions"
			or ("permits count the whole world's: " .. tostring(whyPermits))
		l:log("the game scripts' GUI state: " .. table.concat(parts, "; "))
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

	-- The builds scripts send from this, the game scripts' GUI state: in the
	-- room's game, each goes to the room as the follow-up of this player's
	-- build, or is stopped (tpf3mp/modbuild.lua). Put on once, from the
	-- first guiUpdate with a link; guiUpdate runs in no other state.
	local function followUpsInGui(l)
		if followUpsOn then return end
		followUpsOn = true
		followUps = modbuild.tracker()
		local okGuard, guardModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/guard.lua")
		local okCmd, cmd = pcall(function() return api.cmd end)
		local ok, why = nil, "api.cmd cannot be read"
		if okCmd then
			ok, why = modbuild.install(cmd, {
				inRoom = function() return l:room() end,
				applying = function() return applying end,
				follows = function() return followUps.follows(l:note(modbuild.NOTE)) end,
				clicks = function() return l:clicks() end,
				keep = function(count, seen) snapshots[count] = seen end,
				capture = function(shaped, network)
					return capture[network == "Track" and "track" or "street"](shaped)
				end,
				callers = (okGuard and type(guardModule) == "table") and guardModule.callers or nil,
				log = function(line) l:log(line) end,
			})
		end
		l:log(ok and "scripts' builds from the game scripts' GUI state go to the room as their player's follow-ups"
			or ("scripts' builds from the game scripts' GUI state are not guarded: " .. tostring(why)))
	end

	local function xy(v)
		if v == nil then return nil end
		local x = v.x or v[1]
		local y = v.y or v[2]
		if type(x) == "number" and type(y) == "number" then return x, y end
		return nil
	end

	-- Extracts ground-plane position and optional Hermite curves from a build proposal.
	local function extractProposalPreview(_id, proposal)
		if type(proposal) ~= "table" and type(proposal) ~= "userdata" then return nil, nil end
		local ok, px, py, curves = pcall(function()
			local street = type(proposal.proposal) == "table" and proposal.proposal or nil
			local curveList = {}
			local nodeMap = {}
			if street and type(street.addedNodes) == "table" then
				for _, n in ipairs(street.addedNodes) do
					if type(n.entity) == "number" and n.comp and n.comp.position then
						local nx, ny = xy(n.comp.position)
						if nx and ny then nodeMap[n.entity] = { nx, ny } end
					end
				end
			end
			local function getNodePos(entityId)
				if type(entityId) ~= "number" then return nil end
				if nodeMap[entityId] then return nodeMap[entityId] end
				if entityId > 0 and api and api.engine and api.engine.entityExists and api.engine.entityExists(entityId) then
					local okComp, c = pcall(api.engine.getComponent, entityId, api.type.ComponentType.BASE_NODE)
					if okComp and c and c.position then
						local nx, ny = xy(c.position)
						if nx and ny then
							nodeMap[entityId] = { nx, ny }
							return nodeMap[entityId]
						end
					end
				end
				return nil
			end
			if street and type(street.addedSegments) == "table" then
				for _, seg in ipairs(street.addedSegments) do
					local comp = seg.comp
					if comp and comp.node0 and comp.node1 then
						local p0 = getNodePos(comp.node0)
						local p1 = getNodePos(comp.node1)
						local tx0, ty0 = xy(comp.tangent0)
						local tx1, ty1 = xy(comp.tangent1)
						if p0 and p1 and tx0 and ty0 and tx1 and ty1 then
							curveList[#curveList + 1] = {
								p0[1], p0[2],
								p1[1], p1[2],
								tx0, ty0,
								tx1, ty1,
							}
							if #curveList >= 16 then break end
						end
					end
				end
			end
			local x, y
			if #curveList > 0 then
				local lastCurve = curveList[#curveList]
				x, y = lastCurve[3], lastCurve[4]
			elseif street and type(street.addedNodes) == "table" and #street.addedNodes > 0 then
				local last = street.addedNodes[#street.addedNodes]
				local pos = last and last.comp and last.comp.position
				if pos then x, y = xy(pos) end
			end
			if not x or not y then
				local toAdd = type(proposal.toAdd) == "table" and proposal.toAdd or nil
				if toAdd and #toAdd > 0 then
					local first = toAdd[1]
					local transf = first and (first.transf or first.transformation)
					if type(transf) == "table" and #transf >= 14 then
						x, y = transf[13], transf[14]
					end
				end
			end
			return x, y, (#curveList > 0 and curveList or nil)
		end)
		if ok and px and py then
			return px, py, curves
		end
		return nil, nil
	end

	-- Generates a closed ribbon polygon along Hermite curve c = {x0, y0, x1, y1, tx0, ty0, tx1, ty1}
	local function previewPolygon(c, width)
		local span = (c[3] - c[1]) ^ 2 + (c[4] - c[2]) ^ 2
		local length = math.sqrt(span)
		local samples = math.max(6, math.min(32, math.ceil(length / 15)))
		local left, right = {}, {}
		for i = 0, samples do
			local t = i / samples
			local t2, t3 = t * t, t * t * t
			local h0, h1 = 2 * t3 - 3 * t2 + 1, -2 * t3 + 3 * t2
			local h2, h3 = t3 - 2 * t2 + t, t3 - t2
			local x = h0 * c[1] + h1 * c[3] + h2 * c[5] + h3 * c[7]
			local y = h0 * c[2] + h1 * c[4] + h2 * c[6] + h3 * c[8]
			local d0, d1 = 6 * t2 - 6 * t, -6 * t2 + 6 * t
			local d2, d3 = 3 * t2 - 4 * t + 1, 3 * t2 - 2 * t
			local dx = d0 * c[1] + d1 * c[3] + d2 * c[5] + d3 * c[7]
			local dy = d0 * c[2] + d1 * c[4] + d2 * c[6] + d3 * c[8]
			local norm = math.sqrt(dx * dx + dy * dy)
			if norm < 0.001 then
				dx, dy = c[3] - c[1], c[4] - c[2]
				norm = math.sqrt(dx * dx + dy * dy)
			end
			if norm < 0.001 then return nil end
			local nx, ny = -dy / norm * width, dx / norm * width
			left[#left + 1] = { x + nx, y + ny }
			right[#right + 1] = { x - nx, y - ny }
		end
		local poly = {}
		for _, pt in ipairs(left) do poly[#poly + 1] = pt end
		for i = #right, 1, -1 do poly[#poly + 1] = right[i] end
		return poly
	end

	local function makeVec2f(x, y)
		if api and api.type and api.type.Vec2f and api.type.Vec2f.new then
			return api.type.Vec2f.new(x, y)
		end
		return { x, y }
	end

	local function makeVec4f(r, g, b, a)
		if api and api.type and api.type.Vec4f and api.type.Vec4f.new then
			return api.type.Vec4f.new(r, g, b, a)
		end
		return { r, g, b, a }
	end

	local function canDrawMissionZone()
		return api and api.gui and api.gui.mission
			and type(api.gui.mission.setZone) == "function"
			and type(api.gui.mission.removeZone) == "function"
	end

	-- The sandbox has no mission (api.gui.mission is nil), so the ribbon
	-- cannot be drawn there. api.util.debug.draw.point draws in world
	-- coordinates on a layer mask instead, so the same polyline is sampled
	-- into a chain of points every frame: a dotted hologram in the world,
	-- visible from any camera, without the game's mission system.
	--
	-- This is only the fallback. When a build tool is open, the other player's
	-- pointer is handed to this game's own StreetBuilder and the game's own
	-- renderer draws the preview as it does the local player's
	-- (tpf3mp_hook/src/preview.rs); tpf3mp_native.previewed() says when that
	-- is going on, and the marker stands down for it.
	local DEBUG_LAYER = 0x40000000
	-- Once per game: what the game's own drawing API offers, so the ribbon can
	-- be drawn with the game's primitives where it has more than a point. The
	-- game's stdout has it (crash_dump/stdout.txt); print is the one sink every
	-- Lua state has.
	local dumpedDraw = false
	local function dumpDrawApi()
		if dumpedDraw then return end
		dumpedDraw = true
		local function names(t)
			if type(t) ~= "table" then return "nil" end
			local out = {}
			for k in pairs(t) do
				if type(k) == "string" then out[#out + 1] = k end
			end
			table.sort(out)
			return table.concat(out, " ")
		end
		print("[tpf3mp] debug draw api: " .. names(api and api.util and api.util.debug and api.util.debug.draw))
		print("[tpf3mp] debug api: " .. names(api and api.util and api.util.debug))
		print("[tpf3mp] gui api: " .. names(api and api.gui))
	end
	local function gameDrawsPreview()
		local native = rawget(_G, "tpf3mp_native")
		return type(native) == "table" and type(native.previewed) == "function"
			and native.previewed() == true
	end
	local function canDrawDebugPoints()
		if gameDrawsPreview() then return false end
		return api and api.util and api.util.debug and api.util.debug.draw
			and type(api.util.debug.draw.point) == "function"
			and type(api.util.debug.draw.clear) == "function"
	end

	local function drawWorldPoint(x, y, z, r, g, b)
		local vec = api.type and api.type.Vec3f and api.type.Vec3f.new
			and api.type.Vec3f.new(x, y, z) or { x = x, y = y, z = z }
		local col = api.type and api.type.Vec3f and api.type.Vec3f.new
			and api.type.Vec3f.new(r, g, b) or { x = r, y = g, z = b }
		pcall(api.util.debug.draw.point, vec, col, DEBUG_LAYER)
	end

	-- One Hermite curve as world points: c = {x0,y0,x1,y1,tx0,ty0,tx1,ty1}.
	local function drawCurvePoints(c, r, g, b)
		local span = (c[3] - c[1]) ^ 2 + (c[4] - c[2]) ^ 2
		local length = math.sqrt(span)
		local samples = math.max(8, math.min(48, math.ceil(length / 8)))
		for i = 0, samples do
			local t = i / samples
			local t2, t3 = t * t, t * t * t
			local h0, h1 = 2 * t3 - 3 * t2 + 1, -2 * t3 + 3 * t2
			local h2, h3 = t3 - 2 * t2 + t, t3 - t2
			drawWorldPoint(
				h0 * c[1] + h1 * c[3] + h2 * c[5] + h3 * c[7],
				h0 * c[2] + h1 * c[4] + h2 * c[6] + h3 * c[8],
				0.5, r, g, b)
		end
	end

	local function playerColor(roster, player)
		if roster and companies then
			local ok, c = pcall(companies.of, roster, player)
			if ok and c and type(c.color) == "table" and #c.color >= 3 then
				return c.color
			end
		end
		local palette = (companies and companies.PALETTE) or {
			{ 0.80, 0.16, 0.12 },
			{ 0.13, 0.42, 0.85 },
			{ 0.18, 0.66, 0.27 },
			{ 0.95, 0.72, 0.08 },
		}
		local hash = 0
		local s = tostring(player)
		for i = 1, #s do
			hash = (hash * 31 + string.byte(s, i)) % #palette
		end
		return palette[hash + 1] or palette[1]
	end

	local activeProposalSeen = false
	local localCursorActive = false
	local lastRemoteCursor = {}
	local drawnZones = {}

	-- The guard on what this player's personal mods' game scripts send, in
	-- this state (tpf3mp/modguard.lua): put on once the link is.
	local PERSONAL_UNGUARDED = "personal-mods-unguarded"
	local function guardPersonalMods(companiesModule, registryModule)
		local okModule, modguard = pcall(ug_require, MOD .. "::/scripts/tpf3mp/modguard.lua")
		local okCmd, cmd = pcall(function() return api.cmd end)
		if not okModule or type(modguard) ~= "table" or not okCmd then
			link:log("the personal mods' guard is not on: " .. tostring(modguard))
			return
		end
		if type(debug) ~= "table" or type(debug.getinfo) ~= "function" then
			-- Without the stack no command can be told to be a personal
			-- mod's. Fail closed: the hook loads the room's worlds without
			-- this player's personal mods from now on (tpf3mp_native.note,
			-- PERSONAL_UNGUARDED), and whatever one does before is this
			-- game's alone, which the room's check finds and its resync
			-- loads anew without them.
			link:note(PERSONAL_UNGUARDED, "1")
			if next(link:personal()) ~= nil then
				link:log("the personal mods' guard is not on: this state has no debug.getinfo, "
					.. "so this player's personal mods are left out of the room's worlds from the next load")
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
			local okModbuild, modbuildModule = pcall(ug_require, MOD .. "::/scripts/tpf3mp/modbuild.lua")
			if okBridge and okApply and okLanes and okCapture and okRegistry and okCompanies and okProgression
				and okModbuild and type(modbuildModule) == "table"
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
				modbuild = modbuildModule
				if link then
					link:log("the game script is linked")
					-- Native GUI tools need the simulation's unchanged save player.
					pcall(function() link:note("tpf3mp.player", tostring(api.engine.util.getPlayer())) end)
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
			-- A game day begun with a subsidy another company took still
			-- open: its money is settled (tpf3mp/companies.lua, "subsidies").
			local day = companies.dayNow(api)
			local subsidies = l:room() and type(saved) == "table" and companies.subsidiesDue(saved.companies, day)
			-- The entities the hook's edge watch reads in this update.
			local watch = l:edgewatch()
			if not actions and not checkpoint and not begin and not monthly and not sample and not subsidies
				and not watch then
				return nil
			end
			return { actions = actions, origins = origins, seals = seals, checkpoint = checkpoint,
				begin = begin, monthly = monthly and month or nil, sample = sample and quarter or nil,
				subsidies = subsidies and day or nil, watch = watch }
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
				-- What each company owns, once in this game's state, with
				-- more than one company: whether a world loaded from a save
				-- kept its owners (read only, tpf3mp/companies.lua).
				if not toldOwnership and #companies.live(roster) > 1 then
					toldOwnership = true
					local line = companies.ownership(roster, api)
					l:log("ownership: " .. (line or "this game cannot list the constructions"))
				end
				-- The companies' ranks (tpf3mp/progression.lua).
				local prog = progression.ensure(saved.progression)
				if #failed > 0 and not toldRegistry then
					toldRegistry = true
					l:log("the registry could not list " .. table.concat(failed, "; "))
				end
				-- The room's builds go through; the player's own the hook
				-- stops.
				if work.actions then l:replaying(true) end
				applying = work.actions ~= nil
				-- Whose build applied last: this player's or another's, for
				-- the GUI's scripts' follow-ups (tpf3mp/modbuild.lua).
				local lastBuild
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
					if ok and modbuild.BUILDS[name] then
						local status = l:status()
						lastBuild = (status and status.me_id ~= nil and player == status.me_id) and "mine" or "other"
					end
					if not ok then
						l:log("action " .. i .. " of this step was not applied: " .. tostring(why))
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
				applying = false
				if lastBuild then
					l:note(modbuild.NOTE, modbuild.noted(l:note(modbuild.NOTE), lastBuild == "mine"))
				end
				if work.monthly then
					local ok, why = pcall(companies.chargeMonths, roster, work.monthly, apply.send, api)
					if not ok then l:log("the companies' loans were not charged: " .. tostring(why)) end
				end
				if work.sample then
					local ok, why = progression.sample(prog, roster, api, work.sample,
						function(line) l:log(line) end, reg, registry)
					if not ok then l:log("the companies' scores were not sampled: " .. tostring(why)) end
					-- Each company's headquarters and the bonus its town
					-- gets, read only, when it changed: the game's own town
					-- script gives it (tpf3mp/companies.lua).
					local okHq, lines, whyHq = pcall(companies.headquartersReport, roster, api)
					if not okHq or lines == nil then
						local said = "the headquarters were not read: " .. tostring(okHq and whyHq or lines)
						if toldHeadquarters.failed ~= said then
							toldHeadquarters.failed = said
							l:log(said)
						end
					else
						for _, line in ipairs(lines) do
							local key = line:match("^(.-): headquarters ") or line
							if toldHeadquarters[key] ~= line then
								toldHeadquarters[key] = line
								l:log("headquarters: " .. line)
							end
						end
					end
				end
				saved.registry = reg
				saved.companies = roster
				saved.progression = prog
				state:set(saved)
			end
			-- Another company's subsidies, settled once a game day while one
			-- is open: the money the subsidy script booked to the room's
			-- first company moved on to the company that took it, alike in
			-- every game (tpf3mp/companies.lua, "subsidies").
			if work.subsidies then
				local saved = state:get()
				if type(saved) == "table" and type(saved.companies) == "table" then
					local ok, said = pcall(companies.settleSubsidies, saved.companies, companies.subsidyState(api),
						work.subsidies, apply.send, api)
					if not ok then l:log("the companies' subsidies were not settled: " .. tostring(said)) end
					for _, line in ipairs(ok and said or {}) do l:log(line) end
					state:set(saved)
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
				local dump = l:dump()
				if dump then
					local saved = state and state.get and state:get()
					local reg = type(saved) == "table" and saved.registry or nil
					-- Every entry is handed over: the hook keeps the first
					-- few thousand and counts the rest.
					for _, lane in ipairs(dump.lanes) do
						for _, entry in ipairs(lanes.dump(api, lane, reg, dump.box)) do l:dumped(lane, entry) end
					end
				end
			end
			-- The edge watch: each entity as it reads after this update.
			for _, e in ipairs(work.watch or {}) do l:edgewatched(e, lanes.watch(api, e)) end
		end,

		guiHandleEvent = function(_params, _state, _guiState, _src, id, name, param)
			if name == "builder.proposalCreate" then
				local l = linked()
				if l and l:room() then followInGui(l) end
			end
			if name ~= "builder.proposalCreate" and name ~= "builder.proposalPrepareForApply" then
				local l = linked()
				if l and l:room() then note(l, id, name) end
				return nil
			end
			local l = linked()
			if not l or not l:room() then return nil end
			if type(param) == "table" and param[1] then
				local px, py, curves = extractProposalPreview(id, param[1])
				if px and py then
					activeProposalSeen = true
					localCursorActive = true
					l:cursor(px, py, true, tostring(id), curves)
				end
			end
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
			followUpsInGui(l)
			if followUps then followUps.seen(l:note(modbuild.NOTE)) end

			if l:room() then
				dumpDrawApi()
				if activeProposalSeen then
					activeProposalSeen = false
				elseif localCursorActive then
					localCursorActive = false
					l:cursor(nil)
				end
				-- The world's own debug points of our layer are this frame's:
				-- clear them, then draw every remote preview again.
				local debugPoints = canDrawDebugPoints() and not canDrawMissionZone()
				if debugPoints then pcall(api.util.debug.draw.clear, DEBUG_LAYER) end
				local cursors = l:cursors()
				local saved = _state and _state.get and state:get()
				local roster = type(saved) == "table" and saved.companies or nil
				local activePlayers = {}
				for player, cursor in pairs(cursors) do
					if cursor.building and cursor.x and cursor.y then
						activePlayers[player] = true
						local key = tostring(player) .. " " .. tostring(cursor.label or "")
						if key ~= lastRemoteCursor[player] then
							lastRemoteCursor[player] = key
							l:log("player " .. tostring(player):sub(1, 8) .. " previewing build with "
								.. tostring(cursor.label or "tool") .. " at ("
								.. string.format("%.1f", cursor.x) .. ", "
								.. string.format("%.1f", cursor.y) .. ")")
						end

						if canDrawMissionZone() then
							local color = playerColor(roster, player)
							local drawColor = makeVec4f(color[1], color[2], color[3], 0.55)
							local currentKeys = {}

							if cursor.curves and #cursor.curves > 0 then
								local width = (cursor.label == "trackBuilder" or cursor.label == "rail") and 2.5 or 4.5
								for i, c in ipairs(cursor.curves) do
									local poly = previewPolygon(c, width)
									if poly and #poly >= 3 then
										local polyVec2 = {}
										for _, pt in ipairs(poly) do
											polyVec2[#polyVec2 + 1] = makeVec2f(pt[1], pt[2])
										end
										local zoneKey = "tpf3mp_holo_" .. player .. "_c" .. i
										currentKeys[zoneKey] = true
										pcall(api.gui.mission.setZone, zoneKey, polyVec2, true, drawColor, false, false, 0.05)
									end
								end
								local circleKey = "tpf3mp_holo_" .. player .. "_ptr"
								currentKeys[circleKey] = true
								pcall(api.gui.mission.setZoneCircle, circleKey, makeVec2f(cursor.x, cursor.y), 5.0, true, drawColor, false, false, 0.05)
							else
								local radius = 15.0
								if cursor.label and string.find(cursor.label, "bulldoze", 1, true) then
									radius = 8.0
								elseif cursor.label and string.find(cursor.label, "Terminal", 1, true) then
									radius = 12.0
								elseif cursor.label and string.find(cursor.label, "construction", 1, true) then
									radius = 25.0
								end
								local circleKey = "tpf3mp_holo_" .. player .. "_circle"
								currentKeys[circleKey] = true
								pcall(api.gui.mission.setZoneCircle, circleKey, makeVec2f(cursor.x, cursor.y), radius, true, drawColor, false, false, 0.05)
							end

							for oldKey in pairs(drawnZones[player] or {}) do
								if not currentKeys[oldKey] then
									pcall(api.gui.mission.removeZone, oldKey)
								end
							end
							drawnZones[player] = currentKeys
						elseif debugPoints then
							-- No mission: the same preview as world points.
							local color = playerColor(roster, player)
							local r, g, b = color[1], color[2], color[3]
							if cursor.curves and #cursor.curves > 0 then
								for _, c in ipairs(cursor.curves) do drawCurvePoints(c, r, g, b) end
								drawWorldPoint(cursor.x, cursor.y, 0.5, r, g, b)
							else
								local radius = 15.0
								if cursor.label and string.find(cursor.label, "bulldoze", 1, true) then
									radius = 8.0
								elseif cursor.label and string.find(cursor.label, "Terminal", 1, true) then
									radius = 12.0
								elseif cursor.label and string.find(cursor.label, "construction", 1, true) then
									radius = 25.0
								end
								for i = 0, 16 do
									local a = i / 16 * math.pi * 2
									drawWorldPoint(cursor.x + math.cos(a) * radius, cursor.y + math.sin(a) * radius, 0.5, r, g, b)
								end
								drawWorldPoint(cursor.x, cursor.y, 0.5, r, g, b)
							end
						end
					elseif not cursor.building and lastRemoteCursor[player] then
						lastRemoteCursor[player] = nil
					end
				end

				if canDrawMissionZone() then
					for player, keys in pairs(drawnZones) do
						if not activePlayers[player] then
							for key in pairs(keys) do
								pcall(api.gui.mission.removeZone, key)
							end
							drawnZones[player] = nil
						end
					end
				end
			else
				if canDrawMissionZone() and next(drawnZones) ~= nil then
					for _, keys in pairs(drawnZones) do
						for key in pairs(keys) do
							pcall(api.gui.mission.removeZone, key)
						end
					end
					drawnZones = {}
				end
				if canDrawDebugPoints() then pcall(api.util.debug.draw.clear, DEBUG_LAYER) end
			end

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

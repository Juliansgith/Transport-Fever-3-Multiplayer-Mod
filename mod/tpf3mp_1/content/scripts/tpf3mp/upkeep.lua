-- tpf3mp/upkeep.lua -- what a shared mod's game script keeps up by itself
-- after builds, in the room's game (docs/MODS.md, "Real Cant Engineering";
-- docs/HOOKS.md, "Scripts' upkeep").
--
-- Real Cant Engineering cants tracks: after a build, its game script's GUI
-- half (guiUpdate, in the game scripts' GUI state) scans the tracks and
-- rebuilds those that need it with another template, eases grades at
-- stations, and places and removes its drawings of canted track (a
-- construction along each canted segment). It does so in every game, from
-- what each game's GUI saw, and D27's wrapper (tpf3mp/modbuild.lua) stops
-- every such build: removals and constructions are not a player's
-- follow-up, and most of its passes follow no build of this player's at all
-- (on load, late passes, chains of callbacks).
--
-- So in the room's game one game decides (upkeep.decider: the room's owner
-- while connected, else the connected player of the lowest id). This
-- module sits in front of api.cmd and api.engine.util.proposal in the game
-- scripts' GUI state, and for a mod listed in MODS:
--
-- - each of the mod's builds is read as operations every game can make
--   alike from its own world, by places and names (upkeep.ops): a track
--   rebuilt with another template (replaceSegment), a track rebuilt with
--   other tangents, tracks removed, the mod's constructions built or
--   removed, each found by its file, its place and its identity (one of
--   its parameters the mod names it by);
-- - in the deciding game they go to the room as a ScriptUpkeep action; in
--   every other game nothing is sent. Every game applies the room's action
--   alike (apply.lua, HANDLERS.ScriptUpkeep);
-- - what the mod sets on its own constructions (the bulldozer's lock, the
--   owner) is not sent: the room's action sets it in every game;
-- - the mod's events to its own game script from this state go to the room
--   from the deciding game (tpf3mp/modevents.lua), and are dropped in the
--   others;
-- - the mod's callback hears, a frame later, that its command went: the
--   room builds it a few updates on. Told that it failed, the mod changes
--   its ways for the session (Real Cant turns its ground fill off);
-- - the same operation is handed on once within REPEAT_FRAMES: the mod's
--   next pass, before the room's action lands, asks again;
-- - after the room applies another player's build, the deciding game starts
--   the mod's pass (MODS[..].nudge), which the mod starts by itself only
--   after this game's own builds.
--
-- What is not one of these shapes goes on to D27's wrapper as before.
--
-- Pure Lua; the tests hand install() fakes.

local upkeep = {}

-- The mods whose upkeep the room carries, by modId.
upkeep.MODS = {
	cayde_devers_1 = {
		-- Its drawings of canted track (cayde_devers/shared.lua, M.RENDER),
		-- each named by its parameter cc_sig.
		files = { ["cayde_devers_1::/construction/cayde_devers/render.con"] = true },
		identity = "cc_sig",
		-- Its track templates: every one has this in its name (_ccmark,
		-- _ccant, _ccup, _cck1...).
		template = "_cc",
		-- Its events from this state (cayde_cant.override).
		events = { cayde_cant = true },
		-- Its pass, started as its own guiHandleEvent starts one after a
		-- build of this game's (sync.script.lua, builder.proposalApply).
		nudge = function(load)
			local S = load("cayde_devers_1::/cayde_devers/shared.lua")
			local G = type(S) == "table" and S.G or nil
			if type(G) ~= "table" then return false end
			G.dirty, G.wait, G.fullNext, G.lateFrames = true, 4, true, 90
			return true
		end,
	},
}

-- The note the simulation's state counts the upkeeps applied in.
upkeep.UPKEEP_NOTE = "tpf3mp.upkeep"

-- Frames within which the same operation is handed on once.
upkeep.REPEAT_FRAMES = 900
-- The most bytes of operations put in one action (the action's payload holds
-- 48 KiB), and the most operations.
upkeep.MAX_BYTES = 40000
upkeep.MAX_OPS = 64

local function get(value, key)
	local ok, v = pcall(function() return value[key] end)
	if ok then return v end
	return nil
end

local function list(v)
	if v == nil then return {} end
	local ok, n = pcall(function() return #v end)
	if not ok or type(n) ~= "number" then return nil end
	local out = {}
	for i = 1, n do out[i] = get(v, i) end
	return out
end

local function vec(v)
	if v == nil then return nil end
	local x, y, z = get(v, "x"), get(v, "y"), get(v, "z")
	if type(x) ~= "number" then x, y, z = get(v, 1), get(v, 2), get(v, 3) end
	if type(x) ~= "number" or type(y) ~= "number" or type(z) ~= "number" then return nil end
	return { x = x, y = y, z = z }
end

-- Whether this game decides: the room's owner while connected, else the
-- connected player of the lowest id. `status` is the link's.
function upkeep.decider(status)
	if type(status) ~= "table" or type(status.players) ~= "table" then return false end
	local owner, lowest, me
	for _, p in ipairs(status.players) do
		if type(p) == "table" then
			if p.connected ~= false then
				if p.owner then owner = p end
				if type(p.id) == "number" and (lowest == nil or p.id < lowest.id) then lowest = p end
			end
			if p.me or (status.me_id ~= nil and p.id == status.me_id) then me = p end
		end
	end
	local chosen = owner or lowest
	return me ~= nil and chosen ~= nil and chosen.id == me.id
end

-- The mod of MODS whose construction file `file` is, or nil.
local function modOfFile(file)
	for id, mod in pairs(upkeep.MODS) do
		if mod.files[file] then return id, mod end
	end
	return nil
end

-- The mod of MODS whose track template `template` is, or nil.
local function modOfTemplate(template)
	if type(template) ~= "string" then return nil end
	for id, mod in pairs(upkeep.MODS) do
		if template:find(mod.template, 1, true) then return id, mod end
	end
	return nil
end

-- The mod of MODS whose events have the id `id`, or nil.
function upkeep.modOfEvent(id)
	for modId, mod in pairs(upkeep.MODS) do
		if mod.events and mod.events[id] then return modId, mod end
	end
	return nil
end

-- A 4x4 matrix as the action's transform table: elements 1-3, 5-7, 9-11 and
-- 13-15, read as numbers or as four columns.
local function transformOf(m)
	local e = {}
	for i = 1, 16 do e[i] = get(m, i) end
	if type(e[13]) ~= "number" then
		for c = 1, 4 do
			local col = get(m, c)
			local v = vec(col)
			local w = get(col, "w")
			if v == nil then error("a matrix it cannot read", 0) end
			e[(c - 1) * 4 + 1], e[(c - 1) * 4 + 2], e[(c - 1) * 4 + 3] = v.x, v.y, v.z
			e[(c - 1) * 4 + 4] = type(w) == "number" and w or get(col, 4)
		end
	end
	for _, i in ipairs({ 1, 2, 3, 5, 6, 7, 9, 10, 11, 13, 14, 15 }) do
		if type(e[i]) ~= "number" then error("a matrix it cannot read", 0) end
	end
	return {
		basis = { e[1], e[2], e[3], e[5], e[6], e[7], e[9], e[10], e[11] },
		origin = { x = e[13], y = e[14], z = e[15] },
	}
end
upkeep.transformOf = transformOf

-- An edge as the action names it: its nodes' places, node0's first. `world`
-- reads components (world.component(entity, "BASE_EDGE")).
local function endsOf(world, edge)
	local c = world.component(edge, "BASE_EDGE")
	if c == nil then error("an edge that is gone", 0) end
	local a = vec(get(world.component(get(c, "node0"), "BASE_NODE"), "position"))
	local b = vec(get(world.component(get(c, "node1"), "BASE_NODE"), "position"))
	if a == nil or b == nil then error("an edge whose nodes it cannot read", 0) end
	return { a = a, b = b }, c
end

local function templateOf(c)
	local t = get(c, "roadTemplate")
	if t == nil then return nil end
	return tostring(t)
end

-- The operations a script's build makes, and the mod they are the upkeep
-- of; nil when it is no such build (D27's wrapper judges it); or an error
-- naming why it cannot go. `recorded` is what the wrapped proposal helpers
-- made it of ({ retype = { edge, template } } or { remove = { edges } });
-- `world` reads the world; `encode` writes plain data (modevents.encode).
function upkeep.ops(proposal, recorded, world, encode)
	if recorded and recorded.retype then
		local id = modOfTemplate(recorded.retype.template)
		if id == nil then return nil end
		local ends = endsOf(world, recorded.retype.edge)
		return id, { { Retype = { edge = ends, template = recorded.retype.template } } }
	end
	if recorded and recorded.remove then
		local found, ops = nil, {}
		local edges = {}
		for _, edge in ipairs(recorded.remove.edges) do
			local ends, c = endsOf(world, edge)
			local id = modOfTemplate(templateOf(c))
			if id == nil or (found and id ~= found) then return nil end
			found = id
			edges[#edges + 1] = ends
		end
		if found == nil then return nil end
		ops[1] = { RemoveEdges = { edges = edges } }
		return found, ops
	end
	-- Constructions removed (a Proposal's toRemove), all the mod's.
	local toRemove = list(get(proposal, "toRemove"))
	if toRemove and #toRemove > 0 then
		local found, ops = nil, {}
		for _, entity in ipairs(toRemove) do
			local c = world.component(entity, "CONSTRUCTION")
			local file = c and get(c, "fileName")
			local id, mod = modOfFile(tostring(file))
			if id == nil or (found and id ~= found) then return nil end
			found = id
			local identity = get(get(c, "params"), mod.identity)
			if type(identity) ~= "string" or identity == "" then error("a construction without its identity", 0) end
			ops[#ops + 1] = { Demolish = { file = file, at = transformOf(get(c, "transf")).origin,
				identity = identity } }
		end
		return found, ops
	end
	local street = get(proposal, "streetProposal")
	local adds = list(get(proposal, "constructionsToAdd")) or {}
	local removes = list(street and get(street, "edgesToRemove")) or {}
	local edgeAdds = list(street and get(street, "edgesToAdd")) or {}
	-- Constructions added, all the mod's, and nothing else.
	if #adds > 0 then
		if #removes > 0 or #edgeAdds > 0 then return nil end
		local found, ops = nil, {}
		for _, ce in ipairs(adds) do
			local file = get(ce, "fileName")
			local id, mod = modOfFile(tostring(file))
			if id == nil or (found and id ~= found) then return nil end
			found = id
			local params = get(ce, "params")
			if type(params) ~= "table" then error("a construction whose parameters it cannot read", 0) end
			local identity = params[mod.identity]
			if type(identity) ~= "string" or identity == "" then error("a construction without its identity", 0) end
			local name = get(ce, "name")
			ops[#ops + 1] = { Build = { file = file, transform = transformOf(get(ce, "transf")),
				name = (type(name) == "string" and name ~= "") and name or "Construction",
				params = encode(params), identity = identity, locked = true } }
		end
		return found, ops
	end
	-- One track rebuilt between its own nodes with other tangents.
	if #removes == 1 and #edgeAdds == 1 then
		local ends, old = endsOf(world, removes[1])
		local id = modOfTemplate(templateOf(old))
		if id == nil then return nil end
		local comp = get(edgeAdds[1], "comp")
		if get(comp, "node0") ~= get(old, "node0") or get(comp, "node1") ~= get(old, "node1") then return nil end
		local t0, t1 = vec(get(comp, "tangent0")), vec(get(comp, "tangent1"))
		if t0 == nil or t1 == nil then error("tangents it cannot read", 0) end
		return id, { { Regrade = { edge = ends, tangent0 = t0, tangent1 = t1 } } }
	end
	return nil
end

-- The operations as actions, each within MAX_BYTES and MAX_OPS (`size`
-- estimates an operation's bytes).
function upkeep.actions(mod, ops, size)
	local out, current, bytes = {}, nil, 0
	for _, op in ipairs(ops) do
		local n = size(op)
		if current == nil or #current >= upkeep.MAX_OPS or bytes + n > upkeep.MAX_BYTES then
			current, bytes = {}, 0
			out[#out + 1] = { ScriptUpkeep = { mod_id = mod, ops = current } }
		end
		current[#current + 1] = op
		bytes = bytes + n
	end
	return out
end

-- What the mod's callback hears of a command that went to the room, or was
-- left to the deciding game: it went, and made nothing yet.
local function answer()
	return {
		resultEntities = {},
		resultProposalData = { errorState = { critical = false, messages = {}, warnings = {} }, costs = 0 },
	}
end

-- Puts the wrappers on `cmd` (the state's api.cmd) and `proposals` (its
-- api.engine.util.proposal). `env`:
--   inRoom()          -> whether this is the room's game;
--   applying()        -> whether this state applies the room's builds;
--   decides()         -> whether this game decides (upkeep.decider);
--   command(action)   -> hands an action to the room: true, or nil and why;
--   world             -> { component(entity, name) }: the world's
--                        components, nil where none;
--   encode(value)     -> plain data as text (modevents.encode);
--   event(src, id, name, param) -> the ModEvent action (modevents.capture);
--   log(line)         -> a line for the hook's log.
-- Returns a table { frame(note, load) } to call each frame, or nil and why.
function upkeep.install(cmd, proposals, env)
	if type(cmd) ~= "table" then return nil, "api.cmd is not a table" end
	local send, factory = cmd.sendCommand, cmd.makeWorldBuildProposalCmd
	if send == nil or factory == nil then return nil, "api.cmd has no sendCommand or makeWorldBuildProposalCmd" end
	local recorded = setmetatable({}, { __mode = "k" })
	local held = setmetatable({}, { __mode = "k" })
	local later, frame, lastHanded, told = {}, 0, {}, {}
	local handed = 0

	local function tell(key, line)
		if told[key] then return end
		told[key] = true
		env.log(line)
	end

	-- What the wrapped proposal helpers made.
	if type(proposals) == "table" then
		local replace, removeSegments = proposals.replaceSegment, proposals.makeSegmentsRemoveProposal
		if replace then
			proposals.replaceSegment = function(edge, template, ...)
				local p = replace(edge, template, ...)
				if p ~= nil and (type(p) == "table" or type(p) == "userdata") then
					recorded[p] = { retype = { edge = edge, template = template } }
				end
				return p
			end
		end
		if removeSegments then
			proposals.makeSegmentsRemoveProposal = function(ids, ...)
				local p = removeSegments(ids, ...)
				if p ~= nil and (type(p) == "table" or type(p) == "userdata") then
					local edges = {}
					for i, e in ipairs(list(ids) or {}) do edges[i] = e end
					recorded[p] = { remove = { edges = edges } }
				end
				return p
			end
		end
	end

	local function active() return env.inRoom() and not env.applying() end

	cmd.makeWorldBuildProposalCmd = function(proposal, ...)
		if not active() then return factory(proposal, ...) end
		local ok, mod, ops = pcall(upkeep.ops, proposal, recorded[proposal], env.world, env.encode)
		if not ok then
			tell("why " .. tostring(mod), "a script's upkeep it cannot read: " .. tostring(mod))
			return factory(proposal, ...)
		end
		if mod == nil then return factory(proposal, ...) end
		local command = { tpf3mpUpkeep = true }
		held[command] = { mod = mod, ops = ops }
		return command
	end

	-- The mod's lock and owner on its own constructions: the room's action
	-- sets both in every game.
	for _, name in ipairs({ "makeWorldSetBulldozableCmd", "makeEntitySetPlayerCmd" }) do
		local made = cmd[name]
		if made then
			cmd[name] = function(entity, ...)
				if active() then
					local c = env.world.component(entity, "CONSTRUCTION")
					if c and modOfFile(tostring(get(c, "fileName"))) then
						local command = { tpf3mpUpkeep = true }
						held[command] = { skip = name }
						return command
					end
				end
				return made(entity, ...)
			end
		end
	end

	local event = cmd.makeScriptingSendEventCmd
	if event then
		cmd.makeScriptingSendEventCmd = function(src, id, name, param, ...)
			if active() and upkeep.modOfEvent(id) then
				local command = { tpf3mpUpkeep = true }
				held[command] = { event = { src = src, id = id, name = name, param = param } }
				return command
			end
			return event(src, id, name, param, ...)
		end
	end

	-- Hands `action` on unless the same went within REPEAT_FRAMES.
	local function hand(key, action, what)
		local at = lastHanded[key]
		if at and frame - at < upkeep.REPEAT_FRAMES then return end
		local ok, why = env.command(action)
		if ok then
			lastHanded[key] = frame
			handed = handed + 1
			if handed <= 20 or handed % 100 == 0 then
				env.log("handed " .. what .. " to the room (" .. handed .. " so far)")
			end
		else
			tell("room " .. tostring(why), "the room did not take " .. what .. ": " .. tostring(why))
		end
	end

	cmd.sendCommand = function(command, ...)
		local h = type(command) == "table" and held[command]
		if not h then return send(command, ...) end
		held[command] = nil
		local callback = ...
		if h.ops and env.decides() then
			local size = function(op)
				local b = op.Build
				return 256 + (b and (#b.params + #b.identity) or 0) + (op.Demolish and #op.Demolish.identity or 0)
			end
			for _, action in ipairs(upkeep.actions(h.mod, h.ops, size)) do
				local okKey, key = pcall(env.encode, action)
				hand(okKey and key or tostring(action), action, "the upkeep of " .. h.mod .. " ("
					.. #action.ScriptUpkeep.ops .. " operations)")
			end
		elseif h.ops then
			tell("left " .. h.mod, "the upkeep of " .. h.mod .. " is left to the room's deciding game")
		elseif h.event and env.decides() then
			local okA, action = pcall(env.event, h.event.src, h.event.id, h.event.name, h.event.param)
			if okA then
				local okKey, key = pcall(env.encode, action)
				hand(okKey and key or tostring(action), action, "the " .. tostring(h.event.id) .. " "
					.. tostring(h.event.name) .. " event")
			else
				tell("event " .. tostring(action), "a script's event the room cannot carry: " .. tostring(action))
			end
		end
		if type(callback) == "function" then
			later[#later + 1] = function() callback(answer(), true) end
		end
	end

	local seenCount, seenUpkeep = nil, nil
	local function nudge(load, mods, after)
		for id, mod in pairs(upkeep.MODS) do
			if mod.nudge and (mods == nil or mods[id]) then
				local ok, started = pcall(mod.nudge, load)
				if ok and started then
					tell("nudge " .. id .. after, "started the pass of " .. id .. " after " .. after)
				end
			end
		end
	end
	return {
		-- Each frame: the callbacks held back, and the deciding game's nudge
		-- after the room applied another player's build, or an upkeep (the
		-- mod's last pass may have ended before it landed). `note` is
		-- tpf3mp/modbuild.lua's NOTE; `load` loads a mod's file
		-- (ug_require); `mods` lists the room's mods (nil: unknown);
		-- `upkeepNote` counts the upkeeps applied (upkeep.UPKEEP_NOTE).
		frame = function(note, load, mods, upkeepNote)
			frame = frame + 1
			local due = later
			later = {}
			for _, fn in ipairs(due) do
				local ok, err = pcall(fn)
				if not ok then tell("callback " .. tostring(err), "a script's callback failed: " .. tostring(err)) end
			end
			if upkeepNote ~= nil and upkeepNote ~= seenUpkeep then
				local first = seenUpkeep == nil
				seenUpkeep = upkeepNote
				if not first and env.inRoom() and env.decides() then nudge(load, mods, "an upkeep landed") end
			end
			local n, who
			if type(note) == "string" then
				local count, whose = note:match("^(%d+) (%a+)$")
				n, who = tonumber(count), whose
			end
			if n == nil then return end
			if seenCount == nil then seenCount = n return end
			if n == seenCount then return end
			seenCount = n
			if who ~= "other" or not env.inRoom() or not env.decides() then return end
			nudge(load, mods, "another player's build")
		end,
	}
end

return upkeep

-- tpf3mp/apply.lua -- runs an action the room ordered, in the mod's game
-- script's update (docs/HOOKS.md, "Actions in the game").
--
-- A game script runs in an engine state, where a command runs at once (the
-- game's own api/tealdef/api/cmd.d.tl). In its update the game takes no
-- callback ("Callbacks are currently disallowed", build 40408), so a
-- command is sent without one; one the game refuses raises.
-- Every game applies the room's action in the same simulation update, so
-- what this makes of an action may depend on nothing but the action and the
-- world, which every game has alike: no time of day, no camera, no GUI.
--
-- An action is the table the hook hands over (tpf3mp_proto::lua): one
-- entry, the action's name and its body, in the game's units (metres,
-- plain fractions).
--
-- Pure Lua against the game's `api`; the tests give it a fake one.

local apply = {}

-- A matrix from a Transform: its basis is elements 1-3, 5-7 and 9-11 of the
-- game's matrix and its origin elements 13-15 (columns of four).
local function matrix(transform)
	local b, o = transform.basis, transform.origin
	local column = api.type.Vec4f.new
	return api.type.Mat4f.new(
		column(b[1], b[2], b[3], 0),
		column(b[4], b[5], b[6], 0),
		column(b[7], b[8], b[9], 0),
		column(o.x, o.y, o.z, 1)
	)
end

-- A parameter's value: exactly one of Int, Fixed, Bool and Text.
local function paramValue(value)
	if value.Int ~= nil then return value.Int end
	if value.Fixed ~= nil then return value.Fixed end
	if value.Bool ~= nil then return value.Bool end
	if value.Text ~= nil then return value.Text end
	error("a parameter value of no kind")
end

-- The keys a flattened parameter path names: "modules[3801].name" is
-- modules, 3801, name.
local function pathKeys(path)
	local keys = {}
	for part in string.gmatch(path, "[^%.]+") do
		local name, rest = string.match(part, "^([^%[]*)(.*)$")
		if name ~= "" then keys[#keys + 1] = name end
		for index in string.gmatch(rest, "%[(%-?%d+)%]") do
			keys[#keys + 1] = tonumber(index)
		end
	end
	if #keys == 0 then error("an empty parameter path") end
	return keys
end

-- The construction's parameters, from their flattened paths.
local function params(list)
	local out = {}
	for _, param in ipairs(list) do
		local keys = pathKeys(param.key)
		local node = out
		for i = 1, #keys - 1 do
			local key = keys[i]
			if type(node[key]) ~= "table" then node[key] = {} end
			node = node[key]
		end
		node[keys[#keys]] = paramValue(param.value)
	end
	return out
end

-- Sends `command`, which runs at once; a refusal raises, and apply.run
-- reports it.
local function run(command)
	api.cmd.sendCommand(command)
	return true
end

local HANDLERS = {}

function HANDLERS.BuildConstruction(build)
	if build.replaces ~= nil then
		return false, "this version of the mod does not replace constructions yet"
	end
	local proposal = api.type.SimpleProposal.new()
	local entity = api.type.SimpleProposal.ConstructionEntity.new()
	entity.fileName = build.file
	entity.transf = matrix(build.transform)
	entity.params = params(build.params)
	entity.name = build.name
	entity.playerEntity = api.engine.util.getPlayer()
	proposal.constructionsToAdd = { entity }
	-- ignoreErrors false and playerInitiated true: as the player's own build.
	return run(api.cmd.makeWorldBuildProposalCmd(proposal, nil, false, true))
end

-- Runs one action. Returns true, or false and why not; never raises.
function apply.run(action)
	if type(action) ~= "table" then return false, "an action is a table" end
	local kind, body = next(action)
	if kind == nil or next(action, kind) ~= nil then
		return false, "an action is a table of one entry"
	end
	local handler = HANDLERS[kind]
	if handler == nil then
		return false, "this version of the mod does not apply " .. tostring(kind) .. " yet"
	end
	local ok, applied, why = pcall(handler, body)
	if not ok then return false, tostring(applied) end
	return applied == true, why
end

-- The actions this version applies, for tests and the log.
function apply.kinds()
	local kinds = {}
	for kind in pairs(HANDLERS) do kinds[#kinds + 1] = kind end
	table.sort(kinds)
	return kinds
end

return apply

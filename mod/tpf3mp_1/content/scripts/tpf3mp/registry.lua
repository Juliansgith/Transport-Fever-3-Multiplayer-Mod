-- tpf3mp/registry.lua -- the canonical ids of what the room's actions name
-- that has no place of its own: vehicles, lines and station groups
-- (tpf3mp_proto::action VehicleId, LineId, StationId; docs/BUILDING.md, "The
-- action schema").
--
-- Every game gives them the same ids without telling another (docs/PLAN.md,
-- Part 3: "the same key on every game, bound in creation order, not by
-- entity ID"): every game runs the same world, so after the same action
-- the same things exist. The registry binds each new one to the next id of
-- its kind, lowest entity first, and retires the ids of those gone; the ids
-- themselves never come back.
--
-- What an action made, as the game answered its command, is bound at once,
-- whatever the lists say, and an id is retired only when its entity no
-- longer exists, never because a list left it out: the registry depends on
-- the game's lists only to find what no action of the room's made (the
-- world's own at the room's first update, the station groups a build makes).
--
-- The mod's game script keeps the registry in
-- its state, which the game saves with the world, so a player who joins or
-- reloads from a save of the room has it as the others do, and brings it up
-- to date after every action the room orders and at the room's first
-- update. The GUI reads it from there to name what the player picks.
--
--   registry = {
--     vehicles = { next = n, bound = { { id, entity }, ... } },
--     lines    = { ... },
--     groups   = { ... },
--   }
--
-- A list of pairs, not a table keyed by id: a save keeps it as it is.
--
-- Pure Lua against the game's `api`; the tests give it a fake one.

local registry = {}

registry.KINDS = { "vehicles", "lines", "groups" }

-- The component each kind's entities have.
local COMPONENT = { vehicles = "TRANSPORT_VEHICLE", lines = "LINE", groups = "STATION_GROUP" }

-- Whether `entity` is still one of `kind` in this world.
local function exists(kind, entity)
	local ok, c = pcall(function()
		if api.engine.entityExists and not api.engine.entityExists(entity) then return nil end
		return api.engine.getComponent(entity, api.type.ComponentType[COMPONENT[kind]])
	end)
	return ok and c ~= nil
end

-- The entities of `kind` in this world, in any order.
local function entities(kind)
	local list
	if kind == "vehicles" then
		list = api.engine.getEntitiesWithComponent(api.type.ComponentType.TRANSPORT_VEHICLE)
	elseif kind == "lines" then
		list = api.engine.system.lineSystem.getLines()
	elseif kind == "groups" then
		list = api.engine.getEntitiesWithComponent(api.type.ComponentType.STATION_GROUP)
	end
	local out = {}
	for i = 1, (list and #list or 0) do out[i] = list[i] end
	return out
end

-- Brings `reg` up to date with the world, and returns it, what it bound
-- now ({ { kind, id, entity }, ... }, in order) and the kinds it could not
-- list, whose bindings stay as they were. `reg` may be nil: a registry is
-- begun. `made` ({ [kind] = { entity, ... } }, or nil) names what the
-- action just applied made, which the lists may not show yet.
function registry.sync(reg, made)
	reg = reg or {}
	local fresh, failed = {}, {}
	for _, kind in ipairs(registry.KINDS) do
		local r = reg[kind] or { next = 0, bound = {} }
		reg[kind] = r
		local listed, list = pcall(entities, kind)
		if not listed then
			failed[#failed + 1] = kind .. ": " .. tostring(list)
			list = nil
		end
		local present = {}
		for _, e in ipairs(list or {}) do present[e] = true end
		for _, e in ipairs(made and made[kind] or {}) do present[e] = true end
		local kept, known = {}, {}
		for _, pair in ipairs(r.bound) do
			-- Kept while it exists, listed or not; where the kind could not
			-- be listed, as it was.
			if present[pair[2]] or not listed or exists(kind, pair[2]) then
				kept[#kept + 1] = pair
				known[pair[2]] = true
			end
		end
		local new = {}
		for e in pairs(present) do
			if not known[e] then new[#new + 1] = e end
		end
		table.sort(new)
		for _, e in ipairs(new) do
			kept[#kept + 1] = { r.next, e }
			fresh[#fresh + 1] = { kind, r.next, e }
			r.next = r.next + 1
		end
		r.bound = kept
	end
	return reg, fresh, failed
end

-- The canonical id of `entity`, of `kind`, or nil.
function registry.id(reg, kind, entity)
	local r = reg and reg[kind]
	for _, pair in ipairs(r and r.bound or {}) do
		if pair[2] == entity then return pair[1] end
	end
	return nil
end

-- The entity of `kind` with canonical id `id`, or nil.
function registry.entity(reg, kind, id)
	local r = reg and reg[kind]
	for _, pair in ipairs(r and r.bound or {}) do
		if pair[1] == id then return pair[2] end
	end
	return nil
end

return registry

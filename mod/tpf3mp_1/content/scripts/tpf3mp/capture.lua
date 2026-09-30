-- tpf3mp/capture.lua -- a build the player made with the game's own tools,
-- as the action the room orders instead (docs/HOOKS.md, "The build tools").
--
-- The tools show every proposal they make to game scripts
-- (builder.proposalCreate), with the proposal as the game will build it;
-- the mod's game script keeps the action each makes, and hands the room the
-- one the player clicked. Everything is read from the proposal as it is, in
-- the game's units: metres, and plain fractions for the matrix.
--
-- A proposal the room cannot carry yet is not guessed at: the capture says
-- why, and the tool shows it.
--
-- Pure Lua; the tests hand it proposals of the game's shape.

local capture = {}

local function get(value, key)
	local ok, v = pcall(function() return value[key] end)
	if ok then return v end
	return nil
end

local function length(list)
	if list == nil then return 0 end
	local ok, n = pcall(function() return #list end)
	if ok and type(n) == "number" then return n end
	return nil
end

local function sortedKeys(tbl)
	local keys = {}
	for key in pairs(tbl) do keys[#keys + 1] = key end
	table.sort(keys, function(a, b)
		if type(a) == type(b) then return a < b end
		return type(a) == "number"
	end)
	return keys
end

-- A construction's parameters as the schema's flat list (tpf3mp_proto
-- action::Param): nested tables become paths, "modules[3801].name"; a number
-- with no fraction is Int, any other Fixed; a boolean Bool, a string Text.
-- Returns the list, or nil and why.
function capture.params(tbl)
	local out = {}
	local function walk(node, path, depth)
		if depth > 8 then error("parameters nested deeper than 8") end
		for _, key in ipairs(sortedKeys(node)) do
			local value = node[key]
			local here
			if type(key) == "number" and key == math.floor(key) then
				here = path .. "[" .. string.format("%d", key) .. "]"
			elseif type(key) == "string" and key:match("^[%a_][%w_]*$") then
				here = path == "" and key or (path .. "." .. key)
			else
				error("a parameter named " .. tostring(key))
			end
			local kind = type(value)
			if kind == "table" then
				walk(value, here, depth + 1)
			elseif kind == "number" then
				if value == math.floor(value) then
					out[#out + 1] = { key = here, value = { Int = value } }
				else
					out[#out + 1] = { key = here, value = { Fixed = value } }
				end
			elseif kind == "boolean" then
				out[#out + 1] = { key = here, value = { Bool = value } }
			elseif kind == "string" then
				out[#out + 1] = { key = here, value = { Text = value } }
			else
				error("parameter " .. here .. " is a " .. kind)
			end
		end
	end
	local ok, why = pcall(walk, tbl, "", 0)
	if not ok then return nil, tostring(why) end
	return out
end

-- The schema's transform (action::Transform) of the game's 4x4 matrix: its
-- basis is elements 1-3, 5-7 and 9-11, its origin 13-15.
function capture.transform(m)
	local function at(i)
		local v = get(m, i)
		if type(v) ~= "number" then error("the matrix has no element " .. i) end
		return v
	end
	return {
		basis = { at(1), at(2), at(3), at(5), at(6), at(7), at(9), at(10), at(11) },
		origin = { x = at(13), y = at(14), z = at(15) },
	}
end

-- One construction placed on its own: the construction tool's stations,
-- depots and the rest (tpf3mp_proto action::ConstructionBuild). Returns the
-- action table, or nil and why the room cannot carry it yet.
function capture.construction(proposal)
	local street = get(proposal, "proposal")
	for _, list in ipairs({ "addedNodes", "addedSegments", "removedNodes", "removedSegments",
		"edgeObjectsToAdd" }) do
		local n = length(street and get(street, list))
		if n == nil then return nil, "a proposal it cannot read" end
		if n > 0 then return nil, "a construction built with roads" end
	end
	if (length(get(proposal, "toRemove")) or 1) > 0 then
		return nil, "a construction that replaces another"
	end
	local toAdd = get(proposal, "toAdd")
	if length(toAdd) ~= 1 then return nil, "more than one construction at once" end
	local con = get(toAdd, 1)
	local file = get(con, "fileName")
	if type(file) ~= "string" or file == "" then return nil, "a construction of no file" end
	local name = get(con, "name")
	if type(name) ~= "string" or name == "" then return nil, "an unnamed construction" end
	local params = get(con, "params")
	if type(params) ~= "table" then
		local construction = get(con, "construction")
		params = construction and get(construction, "params")
	end
	if type(params) ~= "table" then return nil, "a construction without its parameters" end
	local list, why = capture.params(params)
	if not list then return nil, why end
	local ok, transform = pcall(capture.transform, get(con, "transf"))
	if not ok then return nil, tostring(transform) end
	return { BuildConstruction = { file = file, transform = transform, params = list, name = name } }
end

return capture

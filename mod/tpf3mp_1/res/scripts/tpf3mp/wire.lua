-- tpf3mp/wire.lua -- an action table to the bytes of an intent's payload.
--
-- The payload is exactly what tpf3mp_proto::action::Action::to_payload
-- writes: the schema version, then the action, both postcard. Writing it
-- here means the hook takes the bytes as they are (a Lua string) and needs
-- no Lua-side decoder; tpf3mp-proto's tests run this encoder and decode its
-- output with the Rust schema, so the two cannot drift apart unnoticed.
--
-- Action tables mirror the Rust types field for field (serde's external
-- tagging): a struct is a table of its fields by their Rust names, an enum
-- value is its variant name as a string when it carries nothing
-- ("Ground") and a one-entry table {Variant = value} when it does
-- ({Bridge = "cement.lua"}), and positions are integers in millimetres
-- (tpf3mp/fixed.lua). Encoding checks every bound the Rust decoder checks,
-- and raises on the first violation: a capture that cannot be encoded is
-- not sent.
--
-- postcard, as used here: unsigned integers are LEB128 varints, signed ones
-- zigzag-encoded varints, a bool one byte 0 or 1, an Option a byte 0, or 1
-- then the value, a string or list its length as a varint then its bytes or
-- items, a struct its fields in order, an enum its variant index as a
-- varint then its value.
--
-- Lua numbers are doubles here (5.2), exact to 2^53; every value this
-- writes is range-checked well inside that.

local wire = {}

-- tpf3mp_proto::action::ACTION_SCHEMA_VERSION
wire.SCHEMA_VERSION = 1
-- tpf3mp_proto::MAX_PAYLOAD
wire.MAX_PAYLOAD = 48 * 1024

local MAX_VERTICES, MAX_LINKS, MAX_EDGES = 512, 512, 256
local RES_NAME = 128

local function fail(path, message)
	error("tpf3mp.wire: " .. path .. ": " .. message, 0)
end

local function isInteger(n)
	return type(n) == "number" and n == n and math.floor(n) == n
end

local function varint(out, n, path)
	if not isInteger(n) or n < 0 or n > 9007199254740991 then
		fail(path, "not an unsigned integer: " .. tostring(n))
	end
	while n >= 128 do
		out[#out + 1] = string.char(n % 128 + 128)
		n = math.floor(n / 128)
	end
	out[#out + 1] = string.char(n)
end

local function unsigned(out, n, max, path)
	if not isInteger(n) or n < 0 or n > max then
		fail(path, "not an integer in 0.." .. max .. ": " .. tostring(n))
	end
	varint(out, n, path)
end

local function i32(out, n, path)
	if not isInteger(n) or n < -2147483648 or n > 2147483647 then
		fail(path, "not an i32: " .. tostring(n))
	end
	if n >= 0 then varint(out, n * 2, path) else varint(out, -n * 2 - 1, path) end
end

local function bool(out, b, path)
	if type(b) ~= "boolean" then fail(path, "not a boolean: " .. tostring(b)) end
	out[#out + 1] = b and "\1" or "\0"
end

-- Text<max>: at most `max` bytes, no control characters (C0, DEL, and C1 as
-- UTF-8), as tpf3mp_proto::Text requires.
local function text(out, s, max, path)
	if type(s) ~= "string" then fail(path, "not a string: " .. tostring(s)) end
	if #s > max then fail(path, #s .. " bytes; the limit is " .. max) end
	if s:find("%z") or s:find("[\1-\31\127]") or s:find("\194[\128-\159]") then
		fail(path, "contains a control character")
	end
	varint(out, #s, path)
	out[#out + 1] = s
end

local function list(out, items, max, path, each)
	if type(items) ~= "table" then fail(path, "not a list") end
	local n = #items
	if n > max then fail(path, n .. " items; the limit is " .. max) end
	varint(out, n, path)
	for i = 1, n do each(out, items[i], path .. "[" .. (i - 1) .. "]") end
end

-- The variant name and value of an enum value, checked against `variants`
-- (name -> index); a unit variant is its name alone.
local function variant(v, variants, path)
	local name, value = v, nil
	if type(v) == "table" then
		name = next(v)
		if name == nil or next(v, name) ~= nil then fail(path, "not a one-variant table") end
		value = v[name]
	end
	local index = variants[name]
	if index == nil then fail(path, "unknown variant " .. tostring(name)) end
	return name, index, value
end

local function field(t, name, path)
	if type(t) ~= "table" then fail(path, "not a table") end
	local v = t[name]
	if v == nil then fail(path .. "." .. name, "missing") end
	return v, path .. "." .. name
end

-- ---------- the schema (tpf3mp-proto/src/action.rs) ----------

local function xyz(out, v, path)
	for _, axis in ipairs({ "x", "y", "z" }) do
		i32(out, field(v, axis, path))
	end
end

local NETWORK = { Street = 0, Track = 1 }

local function network(out, v, path)
	local name, index = variant(v, NETWORK, path)
	if type(v) ~= "string" then fail(path, name .. " carries nothing") end
	varint(out, index, path)
end

local function edgeEnds(out, v, path)
	xyz(out, field(v, "a", path))
	xyz(out, field(v, "b", path))
end

local function edgeRef(out, v, path)
	network(out, field(v, "network", path))
	edgeEnds(out, field(v, "ends", path))
end

local RESOLVE = { New = 0, Node = 1, Split = 2 }

local function resolve(out, v, path)
	local name, index, value = variant(v, RESOLVE, path)
	varint(out, index, path)
	if name == "New" then
		if value ~= nil then fail(path, "New carries nothing") end
	elseif name == "Node" then
		network(out, value, path .. ".Node")
	else
		edgeRef(out, value, path .. ".Split")
	end
end

local STRUCTURE = { Ground = 0, Bridge = 1, Tunnel = 2 }

local function structure(out, v, path)
	local name, index, value = variant(v, STRUCTURE, path)
	varint(out, index, path)
	if name == "Ground" then
		if value ~= nil then fail(path, "Ground carries nothing") end
	else
		text(out, value, RES_NAME, path .. "." .. name)
	end
end

-- Also checks what the Rust decoder checks of a polyline: a link, and links
-- joining two different vertices it has.
local function polyline(out, v, path)
	local vertices = field(v, "vertices", path)
	local links = field(v, "links", path)
	list(out, vertices, MAX_VERTICES, path .. ".vertices", function(o, vertex, p)
		xyz(o, field(vertex, "pos", p))
		resolve(o, field(vertex, "resolve", p))
	end)
	if type(links) == "table" and #links == 0 then fail(path .. ".links", "a build with no edges") end
	list(out, links, MAX_LINKS, path .. ".links", function(o, link, p)
		local from = field(link, "from", p)
		local to = field(link, "to", p)
		unsigned(o, from, #vertices - 1, p .. ".from")
		unsigned(o, to, #vertices - 1, p .. ".to")
		if from == to then fail(p, "joins a vertex to itself") end
		xyz(o, field(link, "tangent0", p))
		xyz(o, field(link, "tangent1", p))
		structure(o, field(link, "structure", p))
	end)
	list(out, field(v, "removals", path), MAX_EDGES, path .. ".removals", edgeEnds)
end

local TRAM = { None = 0, Plain = 1, Electric = 2 }

local function roadBuild(out, v, path)
	text(out, field(v, "street", path), RES_NAME, path .. ".street")
	bool(out, field(v, "bus_lane", path))
	local tram, tramPath = field(v, "tram", path)
	local _, index = variant(tram, TRAM, tramPath)
	varint(out, index, tramPath)
	polyline(out, field(v, "polyline", path))
end

local function trackBuild(out, v, path)
	text(out, field(v, "track", path), RES_NAME, path .. ".track")
	bool(out, field(v, "catenary", path))
	polyline(out, field(v, "polyline", path))
end

-- Action's variants in declaration order; only those with an encoder here
-- can be sent from Lua so far.
local ACTION = {
	BuildRoad = 0, BuildTrack = 1, Bulldoze = 2, BuildConstruction = 3,
	BuyVehicle = 4, SellVehicle = 5, CreateLine = 6, EditLine = 7,
	AssignLine = 8, PlaceStop = 9, Terraform = 10, CompanyOp = 11,
}
local ENCODERS = { BuildRoad = roadBuild, BuildTrack = trackBuild }

-- The payload bytes of `action`, or raises with the path of what is wrong.
function wire.encode(action)
	local out = {}
	varint(out, wire.SCHEMA_VERSION, "version")
	local name, index, value = variant(action, ACTION, "action")
	local encoder = ENCODERS[name]
	if not encoder then fail("action", name .. " has no Lua encoder yet") end
	varint(out, index, "action")
	encoder(out, value, name)
	local bytes = table.concat(out)
	if #bytes > wire.MAX_PAYLOAD then
		fail("action", #bytes .. " bytes; the payload limit is " .. wire.MAX_PAYLOAD)
	end
	return bytes
end

-- The same, as ok, bytes-or-reason, for callers that must not raise.
function wire.tryEncode(action)
	return pcall(wire.encode, action)
end

return wire

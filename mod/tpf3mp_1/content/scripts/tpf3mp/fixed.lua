-- tpf3mp/fixed.lua -- world units to the action schema's fixed point.
--
-- Positions and tangents travel in millimetres as i32, unit directions and
-- rotations in millionths (tpf3mp_proto::action). The game hands Lua metres
-- as floats; these round them to the nearest unit, halves away from zero,
-- and refuse what an i32 cannot hold or what is not a number at all, so a
-- bad read fails the capture instead of shipping a wrong position.

local fixed = {}

local I32_MIN, I32_MAX = -2147483648, 2147483647

local function round(v)
	if v >= 0 then return math.floor(v + 0.5) end
	return -math.floor(-v + 0.5)
end

-- `v` scaled by `scale` and rounded, or nil and a reason.
function fixed.scaled(v, scale)
	if type(v) ~= "number" or v ~= v or v == math.huge or v == -math.huge then
		return nil, "not a finite number: " .. tostring(v)
	end
	local n = round(v * scale)
	if n < I32_MIN or n > I32_MAX then
		return nil, "out of range: " .. tostring(v)
	end
	return n
end

-- Metres to millimetres.
function fixed.mm(v) return fixed.scaled(v, 1000) end

-- A vector {x, y, z} (array or x/y/z fields, as the game's Vec3f reads in
-- Lua) as {x =, y =, z =} in units of 1/`scale`, or nil and a reason.
function fixed.vec3(v, scale)
	if type(v) ~= "table" and type(v) ~= "userdata" then
		return nil, "not a vector: " .. tostring(v)
	end
	local out = {}
	for i, axis in ipairs({ "x", "y", "z" }) do
		local c = v[axis]
		if c == nil then c = v[i] end
		local n, err = fixed.scaled(c, scale)
		if not n then return nil, axis .. " " .. err end
		out[axis] = n
	end
	return out
end

-- A position or tangent in metres, as the schema's millimetres.
function fixed.pos(v) return fixed.vec3(v, 1000) end

return fixed

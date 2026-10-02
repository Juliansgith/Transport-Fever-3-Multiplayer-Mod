-- Build 40408's base/content/industries/industryutil.lua appends lanes and
-- discovers default terminals with pairs over string-keyed generated data.
-- Lua hash iteration differs between states. Keep those two input maps in
-- name order before the game's own factory captures them; never reorder a
-- completed result whose station references already contain lane indices.
local function industry_order(module)
    assert(type(module) == "table" and type(module.makeIndustryUpdateFn) == "function",
        "TPF3-MP: unsupported industry utility")
    if module.__tpf3mp_ordered then return module end

    local function ordered_pairs(t)
        local keys = {}
        for k in next, t do
            assert(type(k) == "string", "TPF3-MP: industry map keys must be names")
            keys[#keys + 1] = k
        end
        table.sort(keys)
        local i = 0
        return function()
            i = i + 1
            local k = keys[i]
            if k ~= nil then return k, t[k] end
        end, t, nil
    end

    local mt = { __pairs = ordered_pairs }
    local function ordered(t)
        assert(type(t) == "table", "TPF3-MP: industry map must be a table")
        local previous = getmetatable(t)
        assert(previous == nil or previous == mt,
            "TPF3-MP: unsupported industry map metatable")
        -- Validate before any engine callback is built, including an empty map.
        ordered_pairs(t)
        setmetatable(t, mt)
    end

    local factory = module.makeIndustryUpdateFn
    module.makeIndustryUpdateFn = function(data, ...)
        ordered(data)
        if data.lanes and data.lanes.curves then ordered(data.lanes.curves) end
        assert(select('#', ...) == 0, 'TPF3-MP: unsupported industry factory arguments')
        return function(...)
            ordered(data)
            if data.lanes and data.lanes.curves then ordered(data.lanes.curves) end
            -- Create the game's closure in the worker from this ordered
            -- table. Do not rely on a previously captured table retaining
            -- its metatable across worker transfers.
            return factory(data)(...)
        end
    end
    module.__tpf3mp_ordered = true
    return module
end

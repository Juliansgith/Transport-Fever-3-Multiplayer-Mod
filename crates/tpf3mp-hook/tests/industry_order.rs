//! Input normalization for the industry's Lua 5.2 __pairs protocol.
//! The workspace uses Lua 5.1; game_pairs models the protocol separately.
#![allow(clippy::unwrap_used)]
use mlua::Lua;

fn lua() -> Lua {
    let lua = Lua::new();
    lua.load(format!(
        "{}\nwrap_industry = industry_order",
        include_str!("../src/industry_order.lua")
    ))
    .exec()
    .unwrap();
    lua.load(
        r#"
        function game_pairs(t)
            local mt = getmetatable(t)
            if mt and mt.__pairs then return mt.__pairs(t) end
            return pairs(t)
        end
        function names(t)
            local out = {}
            for k in game_pairs(t) do out[#out + 1] = k end
            return table.concat(out, ',')
        end
    "#,
    )
    .exec()
    .unwrap();
    lua
}

#[test]
fn lane_and_terminal_names_stay_stable_across_insertion_orders_and_rebuilds() {
    lua()
        .load(
            r#"
        local calls = 0
        local module = {makeIndustryUpdateFn = function(data)
            calls = calls + 1
            assert(getmetatable(data).__pairs and getmetatable(data.lanes.curves).__pairs)
            return function(explicit)
                return names(data), names(data.lanes.curves), explicit
            end
        end}
        wrap_industry(module)
        local factory = module.makeIndustryUpdateFn
        assert(wrap_industry(module) == module and module.makeIndustryUpdateFn == factory)
        for _, reverse in ipairs({false, true}) do
            local data, curves = {}, {}
            local order = reverse and {'z', 'b', 'a'} or {'a', 'b', 'z'}
            for _, name in ipairs(order) do
                data['lane_terminal_' .. name] = {curves = {}}
                curves[name] = {{name}}
            end
            data.lanes = {curves = curves}
            local rebuild = factory(data)
            local explicit = {'z', 'a'}
            for _ = 1, 3 do
                -- Engine transfer can recreate plain tables without metatables.
                setmetatable(data, nil)
                setmetatable(curves, nil)
                local terminals, lanes, kept = rebuild(explicit)
                assert(terminals == 'lane_terminal_a,lane_terminal_b,lane_terminal_z,lanes')
                assert(lanes == 'a,b,z')
                assert(kept == explicit and kept[1] == 'z')
            end
            curves.c = {{'c'}}
            assert(names(curves) == 'a,b,c,z')
            assert(curves.a[1][1] == 'a' and data.lane_terminal_z.curves)
        end
        assert(calls == 6)
        assert(getmetatable({a=1}) == nil)
    "#,
        )
        .exec()
        .unwrap();
}

#[test]
fn unsupported_inputs_refuse_before_the_original_factory_runs() {
    lua()
        .load(
            r#"
        local calls = 0
        local m = wrap_industry({makeIndustryUpdateFn = function() calls=calls+1 return function() end end})
        assert(not pcall(m.makeIndustryUpdateFn, {[{}]=1}))
        assert(not pcall(m.makeIndustryUpdateFn, setmetatable({}, {__pairs=function() end})))
        assert(not pcall(wrap_industry, {}))
        assert(not pcall(m.makeIndustryUpdateFn, {}, 42))
        assert(calls == 0)
        m.makeIndustryUpdateFn({})()
        assert(calls == 1)
    "#,
        )
        .exec()
        .unwrap();
}

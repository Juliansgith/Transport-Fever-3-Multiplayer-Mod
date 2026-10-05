-- The base game's subsidy kinds, each wrapped so that a subsidy a room's
-- company took counts that company's transport only (tpf3mp/subsidies.lua,
-- docs/HOOKS.md "Subsidies"). The mod's run script points the subsidy
-- resources here (mod.script.lua); the game's subsidy script then reaches
-- each kind as "<this file>@<kind>.<function>", in every Lua state alike.
function data()
	local subsidies = ug_require("tpf3mp_1::/scripts/tpf3mp/subsidies.lua")
	return subsidies.wrapAll(ug_require, nil)
end

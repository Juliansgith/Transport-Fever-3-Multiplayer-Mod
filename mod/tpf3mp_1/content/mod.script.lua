-- TPF3-MP's run script (mod.json's runScript): while subsidies are a room's
-- channel (tpf3mp/acceptance.lua), the base game's subsidy resources name
-- the mod's wrapper of their scripts, so a subsidy a company took counts
-- that company's transport only (tpf3mp/subsidies.lua, docs/HOOKS.md
-- "Subsidies"). Every game of a room loads the same mod, so every game
-- runs the same scripts. Where anything here is missing, the game's own
-- scripts stay.
function data()
	return {
		runFn = function(_captureParams, _settings)
			if type(addModifier) ~= "function" or type(ug_require) ~= "function" then return end
			local okA, acceptance = pcall(ug_require, "tpf3mp_1::/scripts/tpf3mp/acceptance.lua")
			if not (okA and type(acceptance) == "table" and acceptance.subsidies == true) then return end
			local okS, subsidies = pcall(ug_require, "tpf3mp_1::/scripts/tpf3mp/subsidies.lua")
			if not (okS and type(subsidies) == "table") then return end
			pcall(addModifier, "loadGameRes", subsidies.redirect)
		end,
	}
end

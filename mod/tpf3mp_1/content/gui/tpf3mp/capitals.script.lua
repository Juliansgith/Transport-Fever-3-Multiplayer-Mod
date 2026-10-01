-- Every company's capital on the map (tpf3mp/capitals.lua). Build 40408
-- crowns the player's own capital only: the town label's recipe
-- (gui/main/town_hud_react_util.tl, TownHudIcon) asks
-- town_util.isCapital(town) and, for the player's headquarters town, gives
-- the label the class `capital-city` (blue) and a crown.
--
-- gui/tpf3mp/capitals.res.lua has the game run replace() in each Lua state
-- it renders recipes in, before any renders. While the room has more than
-- one company it:
--
-- - has town_util.isCapital answer true for every company's capital, so
--   each has the game's crown and `capital-city` class;
-- - replaces TownHudIcon with a recipe that calls the game's and puts it in
--   a layout of its own, with a line under it naming whose capital it is
--   ("Capital of Rival", "Capital of Rival and Mine"), and, for a capital
--   that is not the viewer's company's, the class of that company's colour
--   (`tpf3mp-capital-<n>`), which gui/tpf3mp/tpf3mp.css.lua puts on the
--   label's tile in place of the game's blue.
--
-- With one company, or before the roster is read, the game's label is as
-- the game made it, in a plain layout: the HUD takes only a layout from a
-- recipe among a layout's children ("Recipe child must be a layout", build
-- 40408).
function data()
	local react = ug_require "::/gui/main/react.lua"
	local builtin = ug_require "::/gui/main/builtin.lua"
	local engine_react_util = ug_require "::/gui/main/engine_react_util.tl"
	local town_hud_react_util = ug_require "::/gui/main/town_hud_react_util.tl"
	local capitals = ug_require "tpf3mp_1::/scripts/tpf3mp/capitals.lua"
	-- Seconds between two looks at a label's town; whose capital is where
	-- is read again only every capitals.EVERY seconds.
	local INTERVAL = 1.0

	local original = town_hud_react_util.TownHudIcon

	-- The game's log says once what became of the capitals, each way it
	-- went ("[tpf3mp] capitals: ...").
	local told = {}
	local function tell(what)
		if told[what] then return end
		told[what] = true
		pcall(debugPrint, "[tpf3mp] capitals: " .. what)
	end

	-- What `town`'s label shows (capitals.view), or false: no company's
	-- capital, or one company.
	local seen = nil
	local function viewOf(town)
		local map = capitals.current(api)
		if map == nil then return false end
		-- Each new reading's count, once per count: none at all where this
		-- state cannot read the companies' headquarters.
		if map ~= seen then
			seen = map
			local n = 0
			for _ in pairs(map) do n = n + 1 end
			tell(n .. " capital town(s) of the room's companies read here")
		end
		local mine = nil
		pcall(function() mine = api.engine.util.getPlayer() end)
		local shown = capitals.view(map, town, mine)
		if not shown then return false end
		tell("a capital labelled (" .. (shown.class == "" and "the player's own" or shown.class) .. ")")
		return shown
	end

	local Capital = react.RegisterRecipe("Tpf3mpTownHudIcon", function(params, showGrowth, showCargo)
		react.setMouseTransparent(true)
		local shown = engine_react_util.useStepStateTimer(function()
			local ok, found = pcall(viewOf, params and params.entity)
			if not ok then tell("failed: " .. tostring(found)) end
			return ok and found or false
		end, INTERVAL)
		local town = react.CallOriginalRecipe(original, params, showGrowth, showCargo)
		local view = shown:old()
		if not view then return builtin.BoxLayout{ children = { town } } end
		return builtin.BoxLayout{
			orientation = builtin.type.Orientation.Vertical,
			meta = view.class ~= "" and { class = view.class } or nil,
			children = {
				town,
				builtin.TextView{
					text = view.label,
					meta = { class = "tpf3mp-capital-label, font-scale-annotation" },
				},
			},
		}
	end)

	return {
		replace = function(replacementApi)
			local followed, why = capitals.followIsCapital(ug_require, function() return capitals.current(api) end)
			tell(followed and ("the game's isCapital crowns every company's capital (" .. followed .. " town_util table(s))")
				or ("the game's isCapital crowns the player's own capital only: " .. tostring(why)))
			replacementApi.ReplaceRecipe(original, Capital)
			tell("the game's town label recipe is replaced")
		end,
	}
end

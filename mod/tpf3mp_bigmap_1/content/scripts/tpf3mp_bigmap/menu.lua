-- The New Game menu's side of big maps, prototype: which rows to add to the
-- size dropdown, and the tiles a picked row and ratio stand for.
--
-- On TPF2 this was native: Big Maps appended rows to the size dropdown and
-- answered UI::GetNumTilesNew for them, leaving every stock row to the game.
-- TF3's GUI is script, so its New Game menu may be a recipe a mod can
-- extend; if it is, the mod does the same here. Which recipe, and how it
-- asks for a size, is not known until the game is out, so nothing in this
-- mod is registered with the game yet (docs/BIGMAPS.md, "The TF3
-- prototype"). This file uses no game API: it is plain Lua, tested from
-- crates/tpf3mp-bigmap/tests/mod_lua.rs.
--
-- The rows come from ladder.lua, which is generated from the big-map
-- settings, so the menu offers exactly the sizes those settings can build.

local menu = {}

-- The labels to append to the size dropdown, after the game's own rows.
function menu.labels(ladder)
	local labels = {}
	for i, row in ipairs(ladder) do
		labels[i] = row.label
	end
	return labels
end

-- The tiles for the dropdowns' picks: `sizeIndex` and `ratioIndex` count
-- from 0, as the dropdowns do, and the game's own `stockRows` rows come
-- first. Returns:
--   nil                  a stock row: the game answers, unchanged;
--   x, y                 an added row's shape, in tiles;
--   false, reason        an added row whose shape the settings cannot
--                        build, or an index past the added rows.
function menu.tiles(ladder, stockRows, sizeIndex, ratioIndex)
	if sizeIndex < stockRows then
		return nil
	end
	local row = ladder[sizeIndex - stockRows + 1]
	if row == nil then
		return false, "no added size row " .. tostring(sizeIndex)
	end
	local shape = row.shapes[ratioIndex + 1]
	if shape == nil then
		return false, "no ratio " .. tostring(ratioIndex)
	end
	if not shape then
		return false, row.label .. " cannot be built at this ratio with these settings"
	end
	return shape[1], shape[2]
end

return menu

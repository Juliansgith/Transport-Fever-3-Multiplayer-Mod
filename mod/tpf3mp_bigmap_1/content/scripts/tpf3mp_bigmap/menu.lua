-- The New Game page's side of big maps: which rows to add to the size
-- dropdown, and the tiles a picked row and ratio stand for.
--
-- On TPF2 this was native: silver2127's Big Maps (tpf2-bigmap) appended
-- rows to the size dropdown and answered UI::GetNumTilesNew for them,
-- leaving every stock row to the game. TF3's New Game page is a script, so
-- the mod's copy of it (gui/menu/new_game_or_map_settings_page.tl, served
-- by the TPF3-MP hook in place of the game's) does the same with these
-- functions. This file uses no game API apart from reading what the hook
-- left in `resolveutil`: it is plain Lua, tested from
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

-- The machine's memory in MB, as the TPF3-MP hook found it, or nil.
function menu.ramMb()
	local ok, ru = pcall(function()
		return resolveutil
	end)
	if ok and type(ru) == "table" and type(ru.__tpf3mp_ram_mb) == "number" and ru.__tpf3mp_ram_mb > 0 then
		return ru.__tpf3mp_ram_mb
	end
	return nil
end

-- The rows of `ladder` this machine has the memory to generate, and a line
-- for each one left out saying why. With the memory not known, none: a
-- size that may not fit is not offered on a guess.
function menu.offered(ladder, ramMb)
	local rows, notes = {}, {}
	if type(ramMb) ~= "number" or ramMb <= 0 then
		notes[1] = "Bigger sizes are hidden: this computer's memory is not known."
		return rows, notes
	end
	for _, row in ipairs(ladder) do
		if row.peakMb <= ramMb then
			rows[#rows + 1] = row
		else
			notes[#notes + 1] = string.format(
				"%s is hidden: generating it needs about %d GB of memory, and this computer has %d GB.",
				row.label,
				math.ceil(row.peakMb / 1024),
				math.floor(ramMb / 1024)
			)
		end
	end
	return rows, notes
end

-- The size dropdown's entries: the game's own, then the added rows. The
-- game's lists may be native vectors, not tables: they are read by index
-- and length only, as the game's own scripts read them.
function menu.sizeValues(stockValues, rows)
	local values = {}
	for i = 1, #stockValues do
		values[i] = stockValues[i]
	end
	for _, label in ipairs(menu.labels(rows)) do
		values[#values + 1] = label
	end
	return values
end

-- A parameter's numbers, or nil when it has none (the game treats an empty
-- list as none). A native vector counts as well as a table.
local function numbersOf(stockNumbers)
	if stockNumbers == nil then
		return nil
	end
	local ok, count = pcall(function()
		return #stockNumbers
	end)
	if ok and type(count) == "number" and count > 0 then
		return stockNumbers
	end
	return nil
end

-- The dropdown's entry (from 1) for the game's "map.size" `value` and the
-- added row picked (0 for none).
function menu.indexOf(value, pick, stockCount, stockNumbers, rowCount)
	if pick >= 1 and pick <= rowCount then
		return stockCount + pick
	end
	local numbers = numbersOf(stockNumbers)
	if numbers then
		local best, distance = 1, math.abs(numbers[1] - value)
		for i = 2, #numbers do
			if math.abs(numbers[i] - value) < distance then
				best, distance = i, math.abs(numbers[i] - value)
			end
		end
		return best
	end
	return math.max(1, math.min(stockCount, math.floor(value)))
end

-- What picking the dropdown's entry `index` (from 1) sets: the game's
-- "map.size" value and the added row picked (0 for one of the game's own).
-- An added row sets "map.size" to the game's largest size, a value the game
-- knows, which stays what the save says.
function menu.choose(index, stockCount, stockNumbers)
	local numbers = numbersOf(stockNumbers)
	if index > stockCount then
		return numbers and numbers[#numbers] or stockCount, index - stockCount
	end
	return numbers and numbers[index] or index, 0
end

-- The tiles of the added row `pick` (from 1) at the ratio `ratioIndex`
-- (from 0), the short side first as the game's own shapes have it; or
-- nil and the reason.
function menu.gameTiles(rows, pick, ratioIndex)
	local x, y = menu.tiles(rows, 0, pick - 1, ratioIndex)
	if not x then
		return nil, y
	end
	return math.min(x, y), math.max(x, y)
end

return menu

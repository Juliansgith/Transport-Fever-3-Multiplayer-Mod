-- The vehicles' markers in their company's colour: one class for each colour
-- of the companies' palette (tpf3mp/companies.lua), which
-- gui/tpf3mp/company_markers.script.lua puts a vehicle's marker in. The
-- glyph stays white; the tile under it takes the colour, as dark as the
-- game's tiles are: an empty vehicle's tile (`VehicleItem::Icon`,
-- gui/main/internal_hud.css.lua) and a loaded vehicle's cargo tiles
-- (`hud-icon-background`, gui/main/gui_react_util.css.lua).
--
-- Every company's capital on the map (gui/tpf3mp/capitals.script.lua): the
-- line under a capital's town label naming whose it is, on a dark tile as
-- the town label's; and, for another company's capital, one class for each
-- colour of the palette, which colours the town label's tile and that line's
-- in place of the game's capital blue (`R::TownHudIcon!capital-city`,
-- gui/main/hud_icon_master.css.lua). The viewer's own capital keeps the
-- game's blue. The line is hidden at a far zoom, as the town's population.
-- The game's file: "/..." would name one of this mod's (build 40408).
local ssu = require "::/gui/main/stylesheetutil.lua"

function data()
	local result = {}
	-- Without the palette, no class: the game's markers stay as they are.
	local ok, companies = pcall(require, "tpf3mp_1::/scripts/tpf3mp/companies.lua")
	if not ok or type(companies) ~= "table" or type(companies.PALETTE) ~= "table" then return result end
	local a = ssu.makeAdder(result)
	local surface = {
		fileName = "::/gui/builtin/button/default_surface.tga",
		horizontal = { 0, 9, 21, 30 },
		vertical = { 0, 9, 21, 30 },
	}
	a("TextView!tpf3mp-capital-label", {
		backgroundImage1 = surface,
		backgroundColor1 = { 0.1, 0.1, 0.12, 0.75 },
		padding = { 1, 4, 1, 4 },
		gravity = { 0.5, 0 },
	})
	a("HudIconManager::Button!hud-lod-far TextView!tpf3mp-capital-label", {
		visibility = "none",
	})
	for i, color in ipairs(companies.PALETTE) do
		local class = "!" .. companies.markerClass(i)
		a(class .. " VehicleItem::Icon", {
			backgroundColor1 = { color[1], color[2], color[3], 0.9 },
		})
		a(class .. " !hud-icon-background", {
			backgroundColor1 = { color[1], color[2], color[3], 0.75 },
		})
		local capital = "!" .. companies.capitalClass(i)
		a(capital .. " R::TownHudIcon, " .. capital .. " R::TownHudIcon!capital-city", {
			backgroundColor1 = { color[1], color[2], color[3], 0.9 },
		})
		a(capital .. " TextView!tpf3mp-capital-label", {
			backgroundColor1 = { color[1], color[2], color[3], 0.9 },
		})
	end
	return result
end

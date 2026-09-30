-- The vehicles' markers in their company's colour: one class for each colour
-- of the companies' palette (tpf3mp/companies.lua), which
-- gui/tpf3mp/company_markers.script.lua puts a vehicle's marker in. The
-- glyph stays white; the tile under it takes the colour, as dark as the
-- game's tiles are: an empty vehicle's tile (`VehicleItem::Icon`,
-- gui/main/internal_hud.css.lua) and a loaded vehicle's cargo tiles
-- (`hud-icon-background`, gui/main/gui_react_util.css.lua).
-- The game's file: "/..." would name one of this mod's (build 40408).
local ssu = require "::/gui/main/stylesheetutil.lua"

function data()
	local result = {}
	-- Without the palette, no class: the game's markers stay as they are.
	local ok, companies = pcall(require, "tpf3mp_1::/scripts/tpf3mp/companies.lua")
	if not ok or type(companies) ~= "table" or type(companies.PALETTE) ~= "table" then return result end
	local a = ssu.makeAdder(result)
	for i, color in ipairs(companies.PALETTE) do
		local class = "!" .. companies.markerClass(i)
		a(class .. " VehicleItem::Icon", {
			backgroundColor1 = { color[1], color[2], color[3], 0.9 },
		})
		a(class .. " !hud-icon-background", {
			backgroundColor1 = { color[1], color[2], color[3], 0.75 },
		})
	end
	return result
end

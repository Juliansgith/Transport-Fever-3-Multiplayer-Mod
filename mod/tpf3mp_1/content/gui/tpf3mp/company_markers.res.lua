-- Puts the vehicles' markers on the map in their company's colour
-- (gui/tpf3mp/company_markers.script.lua): a react-replacement-config, which
-- the game runs before any recipe renders (gui/main/bootstrap_game.tl;
-- investigation/TF3_MODS_2026-09-27.md).
function data()
	return {
		type = "react-replacement-config",
		data = {
			filePath = "tpf3mp_1::/gui/tpf3mp/company_markers.script",
			doReplaceFn = "replace",
		}
	}
end

-- Crowns every company's headquarters town on the map, in its company's
-- colour (gui/tpf3mp/capitals.script.lua): a react-replacement-config, which
-- the game runs before any recipe renders (gui/main/bootstrap_game.tl;
-- investigation/TF3_MODS_2026-09-27.md).
function data()
	return {
		type = "react-replacement-config",
		data = {
			filePath = "tpf3mp_1::/gui/tpf3mp/capitals.script",
			doReplaceFn = "replace",
		}
	}
end

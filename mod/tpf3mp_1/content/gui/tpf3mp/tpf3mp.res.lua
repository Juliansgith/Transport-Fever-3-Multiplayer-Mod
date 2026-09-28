-- The game bar plugin that loads TPF3-MP into the game's GUI state
-- (gui/tpf3mp/tpf3mp.script.lua). react-plugin resources are how mods made
-- for Transport Fever 3 build 40391 get their code run
-- (investigation/TF3_MODS_2026-09-27.md).
function data()
	return {
		type = "react-plugin ::GameBarInfoDisplayExtension",
		data = {
			filePath = "tpf3mp_1::/gui/tpf3mp/tpf3mp.script@Tpf3mpPlugin",
			priority = 5,
		}
	}
end

-- The game bar plugin that runs this probe in the GUI state, as mods made
-- for Transport Fever 3 build 40391 run their code
-- (investigation/TF3_MODS_2026-09-27.md).
function data()
	return {
		type = "react-plugin ::GameBarInfoDisplayExtension",
		data = {
			filePath = "tpf3mp_detprobe_1::/gui/tpf3mp_detprobe/detprobe.script@Tpf3mpDetProbe",
			priority = 1,
		}
	}
end

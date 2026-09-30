-- The Multiplayer window (gui/tpf3mp/tpf3mp.script.lua, Tpf3mpWindow): the
-- room, its players, its speed, whether the worlds match, and the room's
-- chat, over the game in a game the launcher started. Mounted where the
-- game mounts mods' own entry points (gui/main/mod_entry_point.tl).
function data()
	return {
		type = "react-plugin ::ModEntryPointExtension",
		data = {
			filePath = "tpf3mp_1::/gui/tpf3mp/tpf3mp.script@Tpf3mpWindow",
			priority = 5,
		}
	}
end

-- The Multiplayer button (gui/tpf3mp/tpf3mp.script.lua, Tpf3mpButton), in
-- the game's area for mods' buttons (gui/main/main_mod_button_area.tl): it
-- opens the Multiplayer window in the room's game.
function data()
	return {
		type = "react-plugin ::MainModButtonAreaExtension",
		data = {
			filePath = "tpf3mp_1::/gui/tpf3mp/tpf3mp.script@Tpf3mpButton",
			priority = 5,
		}
	}
end

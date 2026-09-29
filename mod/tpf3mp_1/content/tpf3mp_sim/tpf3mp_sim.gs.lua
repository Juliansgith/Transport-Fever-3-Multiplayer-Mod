-- TPF3-MP's game script: applies the room's actions in the simulation and
-- reads the world's lanes at checkpoints (tpf3mp_sim.script.lua;
-- docs/HOOKS.md, "Actions in the game" and "The world's lanes").
function data()
	return {
		updateScript = {
			fileName = "tpf3mp_sim.script@update",
		},
		postUpdateScript = {
			fileName = "tpf3mp_sim.script@postUpdate",
		},
		handleEventScript = {
			fileName = "tpf3mp_sim.script@handleEvent",
		},
		guiHandleEventScript = {
			fileName = "tpf3mp_sim.script@guiHandleEvent",
		},
	}
end

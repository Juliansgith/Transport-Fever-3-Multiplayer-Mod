-- TPF3-MP's game script: applies the room's actions in the simulation
-- (tpf3mp_sim.script.lua; docs/HOOKS.md, "Actions in the game").
function data()
	return {
		updateScript = {
			fileName = "tpf3mp_sim.script@update",
		},
		handleEventScript = {
			fileName = "tpf3mp_sim.script@handleEvent",
		},
	}
end

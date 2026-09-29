-- The build-events probe's game script (tpf3mp_buildprobe.script.lua).
function data()
	return {
		updateScript = {
			fileName = "tpf3mp_buildprobe.script@update",
		},
		handleEventScript = {
			fileName = "tpf3mp_buildprobe.script@handleEvent",
		},
		guiHandleEventScript = {
			fileName = "tpf3mp_buildprobe.script@guiHandleEvent",
		},
	}
end

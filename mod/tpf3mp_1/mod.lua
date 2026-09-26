-- TPF3-MP's game-side Lua mod. It captures what the player builds as a
-- portable action (docs/BUILDING.md, "The action schema") and hands the
-- encoded action to the hook, which sends it to the room. Nothing here
-- builds anything yet: applying ordered actions comes with the hook.
--
-- The layout and the fields below are Transport Fever 2's (a folder named
-- <name>_<major version>, a data() function returning info). To be confirmed
-- against Transport Fever 3's mod format on release day (docs/DAY_ONE.md).
function data()
	return {
		info = {
			minorVersion = 0,
			severityAdd = "NONE",
			severityRemove = "NONE",
			name = "TPF3-MP",
			description = "Multiplayer for Transport Fever 3. Needs the TPF3-MP launcher.",
			tags = { "Script Mod" },
			authors = { { name = "TPF3-MP", role = "CREATOR" } },
			visible = true,
		},
		runFn = function(settings)
		end,
	}
end

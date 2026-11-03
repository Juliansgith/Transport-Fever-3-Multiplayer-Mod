-- Exercise the real replacement recipe against the documented build 40408
-- GUI interfaces. The local speed deliberately differs from the room's.
local react = ug_require("::/gui/main/react.lua")
local builtin = ug_require("::/gui/main/builtin.lua")
local globals = ug_require("::/gui/main/game_react_globals.tl")
local widgets = ug_require("::/gui/game_bar/game_bar_widgets.tl")
local estimatedMaximum = 2
api.gui.game = api.gui.game or {}
api.gui.game.getEstimatedMaximumGameSpeed = function() return estimatedMaximum end
-- The world's GAME_SPEED (the game's own speed) and this game's own
-- commands, which the speed row writes it with.
local own, applies, sent = { speedup = 0 }, true, {}
api.engine = { util = { getWorld = function() return 1 end },
	getComponent = function(entity, kind)
		assert(entity == 1 and kind == "GAME_SPEED")
		return own
	end }
api.type.ComponentType = { GAME_SPEED = "GAME_SPEED" }
api.cmd = { makeGameSetSpeedCmd = function(speed) return { speed = speed } end,
	sendCommand = function(command)
		sent[#sent + 1] = command.speed
		if applies then own.speedup = command.speed end
	end }
for _, view in ipairs({ "ImageView", "ToggleButtonGroup" }) do
	builtin[view] = function(params) return { view = view, params = params } end
end

local features = { OtherFeature = true }
globals.getDisableFeatures = function() return features end
local readFeatures = globals.getDisableFeatures
-- The public globals module copies functions from game.tl; its local
-- shortcut handlers retain the separate original table.
local gameGlobals = { getDisableFeatures = readFeatures }
GAME_MODULES = { ["/gui/main/game.tl"] = { game_react_globals = gameGlobals } }
local localSpeed, requests = 4, {}
local helper = {
	isEnabled = function() return globals.getDisableFeatures().GameSpeedControl ~= true end,
	togglePause = function() end,
	cycleSpeed = function() end,
	getSpeed = function() return localSpeed end,
	setSpeed = function(speed) requests[#requests + 1] = speed; localSpeed = speed end,
}
local inCallback = false
local params = { gameSpeedHelper = { get = function()
	return { getApi = function() assert(inCallback, "GetApi cannot run during render") return helper end }
end } }
local original = react.RegisterRecipe("GameSpeedControl", function(given)
	assert(given == params)
	return { view = "StockSpeed", params = { speed = helper.getSpeed() } }
end)
widgets.GameSpeedControl = original
local recipe = original
assert(loadstring(mod_source("gui/tpf3mp/speed_control.script.lua")))()
data().replace({ ReplaceRecipe = function(old, new)
	assert(old == original)
	recipe = new
end })
local row = mount(recipe, params)
-- A frame of the game: the row's onStep runs, where GetApi may.
local function frame()
	inCallback = true
	row.step()
	inCallback = false
end
frame()
local function control()
	for _, view in ipairs(views(row.render())) do
		if view.view == "ToggleButtonGroup" then return view.params end
	end
end
local function stock()
	local all = views(row.render())
	assert(#all == 1 and all[1].view == "StockSpeed")
	assert(all[1].params.speed == helper.getSpeed())
end
-- As gui/main/game.tl's gamePause, gameCycleSpeed and
-- IA_GAME_PAUSE_OR_CYCLE_SPEED do, consult features before invoking the helper.
local function shortcut(speed)
	if gameGlobals.getDisableFeatures().GameSpeedControl ~= true then helper.setSpeed(speed) end
end
stock()
assert(globals.getDisableFeatures() == features)
assert(api.gui.game.getEstimatedMaximumGameSpeed() == 2)
if not tpf3mp_native then
	assert(recipe == original and globals.getDisableFeatures == readFeatures
		and gameGlobals.getDisableFeatures == readFeatures)
	shortcut(0)
	assert(#requests == 1 and requests[1] == 0)
	return
end

HOOK.room = true
HOOK.status = { speed = 100, players = {
	{ name = "Host", owner = true, me = false }, { name = "Guest", owner = false, me = true },
} }
-- The stock helper's automatic throttle must never generate local speed
-- changes in a room, including while a joining peer catches up. Reproduce
-- its condition and clamp: a zero "unlimited" result would still commit.
for _, limit in ipairs({ 0, 1, 2, 3, 4 }) do
	estimatedMaximum = limit
	for _, desired in ipairs({ 1, 2, 4 }) do
		local cap = api.gui.game.getEstimatedMaximumGameSpeed()
		assert(not (desired >= 2 and desired <= 4 and desired > cap),
			"local performance must not send an automatic room speed request")
		assert(math.min(desired, cap) == desired)
	end
end
for index, speed in ipairs({ 0, 100, 200, 400 }) do
	HOOK.status.speed = speed
	local c = assert(control())
	assert(c.selected == index and not c.meta.enabled, "guest shows room speed, disabled")
	for _, button in ipairs(c.buttons) do
		assert(not button.meta.enabled and button.meta.tooltip:find("Host controls speed", 1, true))
	end
	c.onValueChange(3) -- stale/synthetic activation cannot request a speed either
	shortcut(2)
	assert(#requests == 0 and localSpeed == 4, "a guest's clicks and shortcuts never reach the stock helper")
end
assert(globals.getDisableFeatures().GameSpeedControl == true)
assert(globals.getDisableFeatures().OtherFeature == true and features.GameSpeedControl == nil,
	"the game's original feature table is not modified")

-- The game's own speed follows the room's, in a guest's game too, and
-- nothing is asked of the room. A save loaded paused (0) in a room running
-- at 1x left the game's audio paused: every locomotive was silent.
assert(#sent == 0, "nothing is written before a frame")
HOOK.status.speed = 100
frame()
assert(#sent == 1 and sent[1] == 1 and own.speedup == 1, "a paused own speed follows the running room")
frame()
assert(#sent == 1, "once it follows, nothing more is sent")
HOOK.status.speed = 400
frame()
assert(#sent == 2 and sent[2] == 4 and own.speedup == 4)
HOOK.status.speed = 0
frame()
assert(#sent == 3 and sent[3] == 0 and own.speedup == 0, "a paused room pauses the game's own speed")
assert(not HOOK.speeds, "following the room is never a request to it")
assert(#requests == 0, "nor a call of the stock helper's setSpeed")
-- A command the game has not taken yet is sent again after a while only.
applies, HOOK.status.speed = false, 200
frame()
assert(#sent == 4 and sent[4] == 2)
for _ = 1, 29 do frame() end
assert(#sent == 4, "not sent every frame")
frame()
assert(#sent == 5 and sent[5] == 2, "sent again")
applies, own.speedup = true, 2
frame()
assert(#sent == 5)
-- Where a mission forbids pausing, a paused room leaves the own speed.
features.GameSpeedPause, HOOK.status.speed = true, 0
frame()
assert(#sent == 5 and own.speedup == 2)
features.GameSpeedPause = nil
-- An unknown room speed, or none, is not followed.
HOOK.status.speed = 150
frame()
HOOK.status.speed = nil
frame()
assert(#sent == 5 and own.speedup == 2)
HOOK.status.speed = 0

-- Ownership changes without recreating the recipe.
HOOK.status.players[1].owner = false
HOOK.status.players[2].owner = true
HOOK.status.speed = 200
stock()
assert(helper.getSpeed() == 2 and localSpeed == 4, "restored native 4x must display accepted 2x")
assert(not HOOK.speeds, "reading restored state is not a request")
helper.setSpeed(4)
assert(HOOK.speeds[1] == 4, "choosing already-restored 4x still sends explicit intent")
assert(helper.getSpeed() == 2, "wait for server acknowledgement to highlight 4x")
HOOK.status.speed = 400
assert(helper.getSpeed() == 4)
requests = {}
assert(globals.getDisableFeatures() == features)
helper.setSpeed(1)
assert(HOOK.speeds[#HOOK.speeds] == 1 and #requests == 0,
	"the host's click asks the room; the game's own speed waits for its answer")
shortcut(0)
assert(HOOK.speeds[#HOOK.speeds] == 0 and #requests == 0)
own.speedup, sent = 0, {}
frame()
assert(#sent == 1 and sent[1] == 4, "the host's own speed follows the room's accepted 4x")

-- The host's stock controls retain the original mission/mod restrictions.
features.GameSpeedPause = true
assert(globals.getDisableFeatures().GameSpeedPause == true)
features.GameSpeedControl = true
shortcut(2)
assert(#requests == 0)
local before = #HOOK.speeds
helper.setSpeed(2)
assert(#HOOK.speeds == before, "button respects mission restrictions")
features.GameSpeedControl = nil
helper.setSpeed(0)
assert(#HOOK.speeds == before, "pause restriction remains")
features.GameSpeedControl, features.GameSpeedPause = nil, nil
HOOK.status.players[2].owner = false
local c = control()
assert(not c.meta.enabled, "a former host now gets the guest display")
shortcut(4)
assert(#requests == 0)
HOOK.status.players = {}
assert(not control().meta.enabled and globals.getDisableFeatures().GameSpeedControl == true)

-- Unknown/unsupported room speeds must not masquerade as a known setting.
HOOK.status.speed = 150
c = control()
assert(c.selected == -1 and c.deselectAllowed)
HOOK.status = nil
c = control()
assert(c.selected == -1 and not c.meta.enabled)
assert(globals.getDisableFeatures().GameSpeedControl == true)

-- Explicit keyboard API calls use the room speed, even when native state differs.
HOOK.status = { speed = 200, players = { { me = true, owner = true } } }
local before = #HOOK.speeds
helper.cycleSpeed()
assert(HOOK.speeds[before + 1] == 4)
HOOK.status.speed = 400
helper.togglePause()
assert(HOOK.speeds[before + 2] == 0)
HOOK.status.speed = 0
helper.togglePause()
assert(HOOK.speeds[before + 3] == 4)
local count = #requests

-- After leaving, stock UI and keyboard control return without a restart.
HOOK.room = false
stock()
assert(globals.getDisableFeatures() == features)
estimatedMaximum = 1
assert(api.gui.game.getEstimatedMaximumGameSpeed() == 1, "single-player keeps its native limiter")
shortcut(2)
assert(#requests == count + 1 and requests[#requests] == 2)

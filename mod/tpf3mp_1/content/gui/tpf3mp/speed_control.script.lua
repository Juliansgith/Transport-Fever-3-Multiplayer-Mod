-- Clock buttons show the room's accepted speed. Only explicit helper API
-- calls send requests; restored GAME_SPEED and automatic native commits
-- cannot change the room. Local writes retain the game's camera/particle behavior.
--
-- Uses build 40408's GameSpeedControl recipe and GameSpeedApi (the game's
-- gui/game_bar/game_bar_widgets.tl and gui/main/game_context.d.tl). The
-- stock speed shortcuts consult game_react_globals.getDisableFeatures;
-- guests get GameSpeedControl disabled there too, without changing the
-- game's feature table. Outside the room the original recipe and features
-- are used, including mission restrictions and the game's keyboard hints.
function data()
	local react = ug_require "::/gui/main/react.lua"
	local builtin = ug_require "::/gui/main/builtin.lua"
	local engine_react_util = ug_require "::/gui/main/engine_react_util.tl"
	local globals = ug_require "::/gui/main/game_react_globals.tl"
	-- game.tl's shortcut handlers use its own table; the public module above
	-- copies that table's functions. Both entry points must be wrapped.
	local gameGlobals = (ug_require "/gui/main/game.tl").game_react_globals
	local widgets = ug_require "::/gui/game_bar/game_bar_widgets.tl"
	local bridge = ug_require "tpf3mp_1::/scripts/tpf3mp/bridge.lua"
	local link = bridge.attach(bridge.find())
	local original = widgets.GameSpeedControl
	local getFeatures = globals.getDisableFeatures

	local function isHost(status)
		for _, player in ipairs(status and status.players or {}) do
			if player.me then return player.owner == true end
		end
		return false
	end

	local function roomNow()
		if not link or not link:room() then return nil end
		return link:status() or {} -- unknown room status grants no controls
	end

	-- The stock row and keyboard shortcuts share this API. Decorate its
	-- user-entry functions, preserving stock layout, restrictions and hints.
	-- Automatic helper commits and snapshot restoration do not call these
	-- decorated entries. Weak keys let replaced world APIs be collected.
	local wrapped = setmetatable({}, { __mode = "k" })
	local function wrapHelper(params)
		local node = params.gameSpeedHelper:get()
		if not node then return end
		local helper = node:getApi()
		if not helper or wrapped[helper] then return end
		wrapped[helper] = true
		local get, set = helper.getSpeed, helper.setSpeed
		local toggle, cycle = helper.togglePause, helper.cycleSpeed
		local lastNonZero = 1
		helper.getSpeed = function()
			local status = roomNow()
			if not status then return get() end
			local speed = status.speed and status.speed / 100
			if speed ~= 0 and speed ~= 1 and speed ~= 2 and speed ~= 4 then return -1 end
			if speed > 0 then lastNonZero = speed end
			return speed
		end
		helper.setSpeed = function(speed)
			local status = roomNow()
			if not status then return set(speed) end
			if not isHost(status) or not helper.isEnabled() then return end
			if speed == 0 and getFeatures().GameSpeedPause == true then return end
			if speed ~= 0 and speed ~= 1 and speed ~= 2 and speed ~= 4 then return end
			if link:chooseSpeed(speed) then
				if speed > 0 then lastNonZero = speed end
				set(speed)
			end
		end
		helper.togglePause = function()
			if not roomNow() then return toggle() end
			local speed = helper.getSpeed()
			if speed >= 0 then helper.setSpeed(speed == 0 and lastNonZero or 0) end
		end
		helper.cycleSpeed = function()
			if not roomNow() then return cycle() end
			local speed = helper.getSpeed()
			if speed >= 0 then helper.setSpeed(speed == 1 and 2 or speed == 2 and 4 or 1) end
		end
	end

	local speeds = { 0, 100, 200, 400 }
	local icons = { "playback_pause", "playback_play", "playback_play_2", "playback_play_4" }
	local names = { "Pause", "Speed: 1x", "Speed: 2x", "Speed: 4x" }
	local RoomSpeed = react.RegisterRecipe("Tpf3mpRoomSpeed", function(params, userParam)
		-- NodeRef:GetApi is invalid during render/useState initialization.
		react.onStep(function() wrapHelper(params) end)
		local state = engine_react_util.useStepStateTimer(function()
			local status = roomNow()
			return {
				inRoom = status ~= nil,
				speed = status and status.speed,
				host = isHost(status),
			}
		end, 0.1, function(a, b)
			return a.inRoom == b.inRoom and a.speed == b.speed and a.host == b.host
		end)
		local current = state:old()
		if not current.inRoom or current.host then
			return builtin.BoxLayout{ children = { react.CallOriginalRecipe(original, params, userParam) } }
		end

		local selected, buttons = -1, {}
		for index, speed in ipairs(speeds) do
			if current.speed == speed then selected = index end
			buttons[index] = {
				meta = {
					tooltip = names[index] .. " — Host controls speed",
					enabled = false,
				},
				content = builtin.ImageView{ path = "::/gui/game_bar/icons/" .. icons[index] .. ".tga" },
			}
		end
		return builtin.BoxLayout{
			orientation = builtin.type.Orientation.Horizontal,
			meta = { tooltip = "Host controls speed" },
			children = {
				builtin.ToggleButtonGroup{
					meta = { enabled = false },
					buttons = buttons, selected = selected, deselectAllowed = selected == -1,
					onValueChange = function() end,
				},
			}
		}
	end)

	return {
		replace = function(replacementApi)
			if not link then return end
			-- GameSpeedHelper automatically commits its local performance cap,
			-- even without a button press. During catch-up that can turn 4x
			-- into 3x/2x/1x, which the native getter reports as a host request.
			-- The room gate already paces simulation and holds lagging peers;
			-- keep the stock helper's supported 0/1/2/4 choices unthrottled.
			-- Returning zero would make its onStep commit every frame.
			local estimate = api.gui.game.getEstimatedMaximumGameSpeed
			api.gui.game.getEstimatedMaximumGameSpeed = function(...)
				if roomNow() ~= nil then return 4 end
				return estimate(...)
			end
			local function roomFeatures(...)
				local features = getFeatures(...)
				local status = roomNow()
				if status == nil or isHost(status) then return features end
				local limited = {}
				for key, value in pairs(features) do limited[key] = value end
				limited.GameSpeedControl = true
				return limited
			end
			globals.getDisableFeatures = roomFeatures
			gameGlobals.getDisableFeatures = roomFeatures
			replacementApi.ReplaceRecipe(original, RoomSpeed)
			link:log("guest speed controls follow the room; only the host can change speed")
			link:log("local speed throttling is off in rooms; the room gate controls simulation pace")
		end,
	}
end

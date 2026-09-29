-- tpf3mp/speed.lua -- the game's speed in a room's game.
--
-- In a game the TPF3-MP launcher started, the room sets the pace: the hook
-- releases the room's steps and makes the game's speed getter answer 1, so
-- one call of the simulation step is one step whatever the speed row says
-- (docs/HOOKS.md, "The step gate in the game"). That keeps the worlds in
-- step; this module keeps what the player sees honest. When the game's own
-- speed is anything but 1x (the speed row, a key, a script), it sets it back
-- to 1x with the game's own command, makeGameSetSpeedCmd (documented in
-- api/tealdef/api/cmd.d.tl), one command at a time.
--
-- The game's functions come in as `api` so the logic runs outside the game:
--   api.inRoom()        true in a game the TPF3-MP launcher started
--   api.getSpeed()      the game's speed (GameSpeed.speedup), or nil
--   api.setSpeed(n, done)  sends the command; done() once it has run
--   api.log(line)

local speed = {}

--- A keeper for one game.
function speed.keeper(api)
	local self = { pending = false, told = false, sent = 0 }

	--- Once a frame. Returns what it did: "outside", "unknown", "ok",
	--- "waiting" or "set".
	function self.step()
		if not api.inRoom() then return "outside" end
		local current = api.getSpeed()
		if current == nil then return "unknown" end
		if current == 1 then return "ok" end
		if self.pending then return "waiting" end
		self.pending = true
		self.sent = self.sent + 1
		if not self.told then
			self.told = true
			api.log("the room sets the pace: the game's speed stays at 1x (it was " .. tostring(current) .. ")")
		end
		api.setSpeed(1, function() self.pending = false end)
		return "set"
	end

	return self
end

--- The keeper for the running game, on the game's own API.
function speed.forGame(log)
	local function env(name)
		local ok, value = pcall(function() return os.getenv(name) end)
		if ok and type(value) == "string" and value ~= "" then return value end
		return nil
	end
	local inRoom = env("TPF3MP_GAME_LINK") ~= nil
	return speed.keeper({
		inRoom = function() return inRoom end,
		getSpeed = function()
			local value
			pcall(function()
				local world = api.engine.util.getWorld()
				value = api.engine.getComponent(world, api.type.ComponentType.GAME_SPEED).speedup
			end)
			if type(value) == "number" then return value end
			return nil
		end,
		setSpeed = function(n, done)
			local ok = pcall(function()
				api.cmd.sendCommand(api.cmd.makeGameSetSpeedCmd(n), function() done() end)
			end)
			-- A command that could not be sent is not waited on.
			if not ok then done() end
		end,
		log = log,
	})
end

return speed

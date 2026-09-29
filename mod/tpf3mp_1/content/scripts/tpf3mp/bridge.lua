-- tpf3mp/bridge.lua -- the Lua half of the link between the mod and the hook.
--
-- The hook (crates/tpf3mp-hook) runs only in a game the TPF3-MP launcher
-- started (DECISIONS.md, D11). There it gives every Lua state that calls
-- `print` one global table, so the mod prints before it looks for it:
--
--   tpf3mp_native = {
--     version = 3,                  -- bridge.VERSION; anything else is refused
--     command = function(action),   -- the player acted: an action table, for
--                                   -- the room to order -> true | false, why
--     take    = function(),         -- in a game script's update: the actions
--                                   -- the room ordered for this update, or nil
--     log     = function(line),     -- a line for hook.log
--   }
--
-- An action table mirrors tpf3mp_proto::action::Action field for field, in
-- the game's units: metres, and plain fractions for directions. The hook
-- converts it to and from the schema (tpf3mp_proto::lua), so the rounding,
-- the bounds and the checks live in Rust alone; `command` returns false and
-- a reason for a table the schema refuses.
--
-- `command` is Session::command (docs/HOOKS.md, "The hook's session"). An
-- action the room orders comes back to every game, the one that sent it
-- included, through `take`: the hook hands it to the first simulation update
-- of the step it was ordered for, and the mod's game script
-- (tpf3mp_sim/tpf3mp_sim.script.lua) applies it there, where a command runs
-- at once, so every game applies it in the same update.
--
-- Without the table the game is the plain game, and attach() says so. The
-- mod then does nothing. With a table of another version, or one missing a
-- function, attach() refuses it rather than guessing (fail closed).
--
-- Pure Lua; the tests hand attach() a fake table.

local bridge = {}

-- 3: the room's actions are taken by the game script (`take`); 2 called the
-- GUI's handlers; 1 passed bytes the mod encoded itself.
bridge.VERSION = 3
bridge.GLOBAL = "tpf3mp_native"

local Link = {}
Link.__index = Link

-- The link to the hook, or nil and why there is none.
function bridge.attach(native)
	if native == nil then return nil, "no hook in this game" end
	if type(native) ~= "table" then return nil, bridge.GLOBAL .. " is not a table" end
	if native.version ~= bridge.VERSION then
		return nil, "the hook speaks bridge version " .. tostring(native.version)
			.. ", the mod " .. bridge.VERSION
	end
	for _, name in ipairs({ "command", "take", "log" }) do
		if type(native[name]) ~= "function" then
			return nil, "the hook has no " .. name .. "()"
		end
	end
	return setmetatable({ native = native }, Link)
end

-- The hook's table in this state, if the hook gave it one. The hook gives
-- it to a state that has printed, so this prints first. Read through pcall:
-- a state that refuses undeclared globals raises on a missing one.
function bridge.find()
	pcall(print, "[tpf3mp] looking for the hook")
	local ok, value = pcall(function() return tpf3mp_native end)
	if ok then return value end
	return nil
end

-- Hands an action table to the room. Returns true, or nil and why not; an
-- action that was not handed over must not be applied locally either.
function Link:command(action)
	if type(action) ~= "table" then return nil, "an action is a table" end
	local ok, result, reason = pcall(self.native.command, action)
	if not ok then return nil, "the hook refused: " .. tostring(result) end
	if result ~= true then
		return nil, "the hook refused the action: " .. tostring(reason or "no reason given")
	end
	return true
end

-- The actions the room ordered for this update, as a list, or nil.
function Link:take()
	local ok, actions = pcall(self.native.take)
	if not ok or type(actions) ~= "table" then return nil end
	return actions
end

function Link:log(line)
	pcall(self.native.log, tostring(line))
end

return bridge

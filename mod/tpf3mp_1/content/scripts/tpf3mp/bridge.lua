-- tpf3mp/bridge.lua -- the Lua half of the link between the mod and the hook.
--
-- The hook (crates/tpf3mp-hook) runs only in a game the TPF3-MP launcher
-- started (DECISIONS.md, D11). There it registers one global table in the
-- Lua state the mod runs in, before the mod starts:
--
--   tpf3mp_native = {
--     version  = 2,                    -- bridge.VERSION; anything else is refused
--     command  = function(action),     -- the player acted: an action table,
--                                      -- for the room to order
--     register = function(handlers),   -- the mod's handlers, which the hook calls:
--                                      --   handlers.apply(action) -> ok, reason
--                                      --   handlers.notice(kind, text)
--     log      = function(line),       -- a line for hook.log
--   }
--
-- An action table mirrors tpf3mp_proto::action::Action field for field, in
-- the game's units: metres, and plain fractions for directions. The hook
-- converts it to and from the schema (tpf3mp_proto::lua), so the rounding,
-- the bounds and the checks live in Rust alone; `command` returns false and
-- a reason for a table the schema refuses.
--
-- These mirror tpf3mp_bridge::Session and its Game trait (docs/HOOKS.md,
-- "The hook's session"): `command` is Session::command, `apply` is
-- Game::apply and `notice` is Game::notice. `register` passes the handlers
-- from the state they live in, so the hook calls them there.
--
-- Without the table the game is the plain game, and attach() says so. The
-- mod then does nothing. With a table of another version, or one missing a
-- function, attach() refuses it rather than guessing (fail closed).
-- Handlers never raise into the hook: an error becomes a refusal, with its
-- message as the reason.
--
-- Pure Lua; the tests hand attach() a fake table.

local bridge = {}

-- 2: actions travel as tables, which the hook converts; 1 passed bytes the
-- mod encoded itself.
bridge.VERSION = 2
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
	for _, name in ipairs({ "command", "register", "log" }) do
		if type(native[name]) ~= "function" then
			return nil, "the hook has no " .. name .. "()"
		end
	end
	return setmetatable({ native = native }, Link)
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

function Link:log(line)
	pcall(self.native.log, tostring(line))
end

-- Wraps a handler so that it never raises, and returns (ok, reason). An
-- apply that says nothing has not applied anything; a notice needs no
-- answer.
local function guarded(name, fn, silentOk)
	return function(...)
		local ok, result, reason = pcall(fn, ...)
		if not ok then return false, name .. " failed: " .. tostring(result) end
		if result == nil then
			if silentOk then return true end
			return false, name .. " gave no answer"
		end
		return result == true, reason
	end
end

-- Registers the mod's handlers with the hook. `apply` must be given;
-- `notice` may be left out.
function Link:register(handlers)
	if type(handlers) ~= "table" or type(handlers.apply) ~= "function" then
		return nil, "handlers need apply()"
	end
	local notice = handlers.notice or function() end
	local ok, err = pcall(self.native.register, {
		apply = guarded("apply", handlers.apply, false),
		notice = guarded("notice", notice, true),
	})
	if not ok then return nil, "the hook refused the handlers: " .. tostring(err) end
	return true
end

return bridge

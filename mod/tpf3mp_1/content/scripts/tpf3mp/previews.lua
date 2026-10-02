-- tpf3mp/previews.lua -- what the players' build tools show, in a room's
-- game (docs/HOOKS.md, "Build previews"): the player's own goes to the
-- room's other members, and theirs come here to be shown. Advisory: nothing
-- here builds, sends a command or touches the world.
--
-- - **Out.** The game script's GUI half hands over the action each of the
--   player's tool proposals would build, as the capture made it
--   (tpf3mp/capture.lua), for the tools whose builds can be shown (SHOWN):
--   `shown`. It hides it when the tool shows nothing (an empty proposal, a
--   proposal the room cannot carry, a click, which the room then orders as
--   the real build) and when the tool that showed it is no longer the
--   active one (`tick`, from the game's own list of active tools). The hook
--   sends it on, at most five times a second, and again every two seconds
--   while it shows (crates/tpf3mp-hook/src/previews.rs).
-- - **In.** `tick` takes what changed of the other members' previews and
--   hands each to the renderer, `previews.render(from, action)`, `action`
--   nil once that member's tool shows nothing. Without a renderer the
--   previews are only kept, and the first of each member said in the log.
--
-- Ported from TpF2 Multiplayer's shared build previews (mp/previews.lua in
-- tpf2-multiplayer), on the room's own action schema.
--
-- Pure Lua; the tests hand it a fake link and a fake api.

local previews = {}

-- The tools whose previews the other members are shown, by the capture's
-- kind: new constructions (stations, depots, buildings, a station's edit),
-- streets, tracks and stops. Not the bulldozer's removals, nor the
-- modifiers' and junction tools' changes, which show nothing new.
previews.SHOWN = { construction = true, street = true, track = true, stop = true }

-- Seconds between two looks at the game's active tools.
local TOOL_EVERY = 0.25
-- Members whose first preview the log says, at most.
local MAX_SAID = 16

-- What the player's tool shows: the tool's id and whether the game's list
-- of active tools named it when it did (then the list says when it closes).
local showing = nil
-- When the active tools were last looked at.
local lookedAt = nil
-- The other members' previews, by player id; and whose first was said.
local remote, said, saidCount = {}, {}, 0
-- What the log said of the active tools' list, once.
local toldTools = false

-- The renderer of the other members' previews (render(from, action)), or
-- nil: kept only.
previews.render = nil

-- The game's active tools, as a set of ids, or nil where it cannot say.
local function activeTools(api)
	local ok, ids = pcall(function() return api.gui.contextHelper.getIdsOfActiveTool() end)
	if not ok or type(ids) ~= "table" then return nil end
	local set = {}
	for _, id in ipairs(ids) do set[tostring(id)] = true end
	return set, ids
end

local function now()
	local ok, t = pcall(os.clock)
	return ok and t or 0
end

-- The player's tool `tool` (its id) proposes `action`, as the capture made
-- it for the tool's `kind`: shown to the other members when the kind is
-- one SHOWN, else whatever showed is hidden.
function previews.shown(link, api, tool, kind, action)
	if not previews.SHOWN[kind] or type(action) ~= "table" then
		previews.hidden(link)
		return
	end
	local ok, why = link:preview(action)
	if not ok then
		-- Too large, or a hook without previews: the others see nothing.
		if showing ~= nil then previews.hidden(link) end
		return why
	end
	if showing == nil or showing.tool ~= tool then
		local set, ids = activeTools(api)
		showing = { tool = tool, listed = set ~= nil and set[tool] == true }
		if not showing.listed and set ~= nil and not toldTools then
			toldTools = true
			local names = {}
			for i, id in ipairs(ids or {}) do names[i] = tostring(id) end
			link:log("the game's active tools do not name the " .. tostring(tool)
				.. " showing a preview (" .. table.concat(names, ", ")
				.. "): its preview hides on its next proposal or click only")
		end
	end
	return nil
end

-- The player's tool shows nothing now.
function previews.hidden(link)
	if showing == nil then return end
	showing = nil
	link:preview(nil)
end

-- Once a GUI frame in the room's game: hides the player's preview once the
-- tool that showed it is no longer active, and hands the renderer what
-- changed of the other members'.
function previews.tick(link, api)
	if showing ~= nil and showing.listed then
		local t = now()
		if lookedAt == nil or t - lookedAt >= TOOL_EVERY or t < lookedAt then
			lookedAt = t
			local set = activeTools(api)
			if set ~= nil and not set[showing.tool] then previews.hidden(link) end
		end
	end
	for _, change in ipairs(link:previews()) do
		local from, action = change.from, change.action
		if type(from) == "string" then
			remote[from] = action
			if action ~= nil and not said[from] and saidCount < MAX_SAID then
				said[from], saidCount = true, saidCount + 1
				local kind = type(action) == "table" and next(action) or "?"
				link:log("another member's build preview arrived: " .. tostring(kind) .. " from " .. from:sub(1, 16))
			end
			if previews.render ~= nil then
				local ok, why = pcall(previews.render, from, action)
				if not ok then link:log("a build preview did not show: " .. tostring(why)) end
			end
		end
	end
end

-- The other members' previews as kept, by player id (for the tests).
function previews.remote()
	return remote
end

-- Forgets everything: a new world's GUI.
function previews.reset()
	showing, lookedAt, remote, said, saidCount, toldTools = nil, nil, {}, {}, 0, false
end

return previews

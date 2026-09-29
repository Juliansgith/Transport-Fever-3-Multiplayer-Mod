-- The Multiplayer window's content, on the game's main menu (docs/LOBBY.md).
--
-- It shows the lobby the hook hands it and sends the player's choices back,
-- over the hook's request channel: `resolveutil.loadfile("tpf3mp_1::/tpf3mp/state.lua")`
-- answers with the state as a Lua table literal; an action is left as JSON in
-- `resolveutil.__tpf3mp_action` and then `resolveutil.loadfile("tpf3mp_1::/tpf3mp/act.lua")`
-- is called, which the hook answers after taking the JSON (the loader accepts
-- exactly one argument). Both files exist in the mod, because the game checks
-- that before it lets the loader run; the hook answers before the loader reads
-- them, so their contents never matter. Without a hook (a game Steam started)
-- this window is not reachable at all.
--
-- Plain Lua, loaded by the mod's main_page.tl through `ug_require`; it uses
-- the same react and builtin modules the menu does.

local react = ug_require "::/gui/main/react.lua"
local builtin = ug_require "::/gui/main/builtin.lua"

local lobby = {}

-- The request channel -------------------------------------------------------

local function say(line)
	pcall(debugPrint, "[tpf3mp] lobby: " .. line)
end

local function ask(request)
	if type(resolveutil) ~= "table" or type(resolveutil.loadfile) ~= "function" then
		return nil, "no loader"
	end
	local ok, reply = pcall(resolveutil.loadfile, "tpf3mp_1::/tpf3mp/" .. request .. ".lua")
	if not ok then
		return nil, tostring(reply)
	end
	if type(reply) ~= "string" then
		return nil, "no hook answered"
	end
	return reply
end

local lastProblem = nil
local function fetchState()
	local reply, why = ask("state")
	if not reply then
		if why ~= lastProblem then
			lastProblem = why
			say("state request failed: " .. tostring(why))
		end
		return nil, why
	end
	local chunk, err = load("return " .. reply, "=tpf3mp-state", "t", {})
	if not chunk then
		return nil, "bad state: " .. tostring(err)
	end
	local ok, state = pcall(chunk)
	if not ok or type(state) ~= "table" then
		return nil, "bad state: " .. tostring(state)
	end
	return state
end

local function jsonString(text)
	text = tostring(text or "")
	return '"' .. text:gsub('[%c"\\]', function(c)
		if c == '"' then return '\\"' end
		if c == "\\" then return "\\\\" end
		if c == "\n" then return "\\n" end
		if c == "\r" then return "\\r" end
		if c == "\t" then return "\\t" end
		return string.format("\\u%04x", c:byte())
	end) .. '"'
end

-- Sends one action, given as a table with an `action` field and its fields.
local function act(fields)
	local parts = {}
	for key, value in pairs(fields) do
		local encoded
		if type(value) == "boolean" then
			encoded = value and "true" or "false"
		elseif type(value) == "number" then
			encoded = string.format("%d", value)
		else
			encoded = jsonString(value)
		end
		parts[#parts + 1] = jsonString(key) .. ":" .. encoded
	end
	local json = "{" .. table.concat(parts, ",") .. "}"
	if type(resolveutil) ~= "table" then
		say("action " .. tostring(fields.action) .. " not sent: no loader")
		return
	end
	resolveutil.__tpf3mp_action = json
	local reply, why = ask("act")
	resolveutil.__tpf3mp_action = nil
	if not reply then
		say("action " .. tostring(fields.action) .. " not sent: " .. tostring(why))
	elseif reply ~= "ok" then
		say("action " .. tostring(fields.action) .. ": " .. reply)
	end
end

-- Widgets --------------------------------------------------------------------

local function text(str, class)
	return builtin.TextView{
		meta = { class = class or "font-scale-body" },
		text = str or "",
	}
end

local function button(label, onClick, class)
	return builtin.Button{
		meta = { class = class or "secondary" },
		content = builtin.TextView{ text = label },
		onClick = onClick,
	}
end

local function input(ref, placeholder, password)
	return builtin.TextInputField{
		value = ref:get(),
		placeholderText = placeholder,
		passwordMode = password or false,
		onValueChange = function(value) ref:set(value) end,
		onTyping = function(value) ref:set(value) end,
	}
end

local function row(children)
	return builtin.BoxLayout{
		orientation = builtin.type.Orientation.Horizontal,
		children = children,
	}
end

local function column(children)
	return builtin.BoxLayout{
		orientation = builtin.type.Orientation.Vertical,
		children = children,
	}
end

-- The window's content, rendered inside the Tpf3mpLobbyWindow recipe. ------

function lobby.content(onClose)
	local stateS = react.useState(nil)
	local problemS = react.useState(nil)
	local server = react.useRef("")
	local name = react.useRef("")
	local roomName = react.useRef("")
	local invite = react.useRef("")
	local password = react.useRef("")
	local chatText = react.useRef("")

	-- Poll the hook for the lobby a few times a second: the room and chat
	-- change without anything happening in this window.
	react.onStepTimer(function()
		local state, why = fetchState()
		if state then
			if problemS:old() ~= nil then problemS:set(nil) end
			stateS:set(state)
		elseif problemS:old() ~= why then
			problemS:set(why)
		end
	end, 0.4, false)

	local state = stateS:old()
	local children = { text(_("Multiplayer"), "font-scale-title-4") }

	if not state then
		children[#children + 1] = text(problemS:old() and ("The hook did not answer: " .. problemS:old())
			or "Waiting for the hook...")
		children[#children + 1] = button(_("Close"), onClose)
		return column(children)
	end

	if state.preview then
		children[#children + 1] = text("Preview: the hook answers on its own until the launcher carries the room.")
	end
	if state.error then
		children[#children + 1] = text(state.error, "font-scale-body, error")
	end
	if state.notice then
		children[#children + 1] = text(state.notice)
	end

	-- Connection.
	if state.connection ~= "connected" then
		children[#children + 1] = text(_("Server"))
		children[#children + 1] = input(server, "play.example.org:4433")
		children[#children + 1] = text(_("Your name"))
		children[#children + 1] = input(name, "name")
		children[#children + 1] = row({
			button(state.connection == "connecting" and "Connecting..." or _("Connect"), function()
				act({ action = "connect", server = server:get(), name = name:get() })
			end),
			button(_("Close"), onClose),
		})
		return column(children)
	end

	children[#children + 1] = row({
		text("Connected to " .. tostring(state.server) .. " as " .. tostring(state.name)),
		button(_("Disconnect"), function() act({ action = "disconnect" }) end),
	})

	local room = state.room
	if not room then
		-- Create or join.
		children[#children + 1] = text(_("Create a room"), "font-scale-title-4")
		children[#children + 1] = input(roomName, "room name")
		children[#children + 1] = input(password, "password (optional)", true)
		children[#children + 1] = button(_("Create"), function()
			act({ action = "create", room = roomName:get(), password = password:get(), max_players = 8 })
		end)
		children[#children + 1] = text(_("Join a room"), "font-scale-title-4")
		children[#children + 1] = input(invite, "invite code")
		children[#children + 1] = button(_("Join"), function()
			act({ action = "join", invite = invite:get(), password = password:get() })
		end)
		children[#children + 1] = button(_("Close"), onClose)
		return column(children)
	end

	-- In a room.
	children[#children + 1] = text(room.name .. "  -  invite " .. room.invite
		.. (room.has_password and "  (password)" or "") .. "  -  " .. room.phase, "font-scale-title-4")

	local you
	for _i, member in ipairs(room.members) do
		if member.you then you = member end
		local marks = {}
		if member.owner then marks[#marks + 1] = "host" end
		if member.you then marks[#marks + 1] = "you" end
		if not member.connected then marks[#marks + 1] = "away" end
		if member.content == "differs" then marks[#marks + 1] = "world differs" end
		local line = (member.ready and "[ready] " or "[     ] ") .. member.name
		if #marks > 0 then line = line .. "  (" .. table.concat(marks, ", ") .. ")" end
		local cells = { text(line) }
		if room.you_own and not member.you then
			cells[#cells + 1] = button(_("Kick"), function() act({ action = "kick", player = member.id }) end)
		end
		children[#children + 1] = row(cells)
	end

	local controls = {}
	controls[#controls + 1] = button((you and you.ready) and _("Not ready") or _("Ready"), function()
		act({ action = "ready", ready = not (you and you.ready) })
	end)
	if room.you_own then
		controls[#controls + 1] = button(_("Start"), function() act({ action = "start" }) end)
	end
	controls[#controls + 1] = button(_("Leave"), function() act({ action = "leave" }) end)
	controls[#controls + 1] = button(_("Close"), onClose)
	children[#children + 1] = row(controls)

	-- Chat: the last lines, newest last.
	children[#children + 1] = text(_("Chat"), "font-scale-title-4")
	local lines = state.chat or {}
	local first = math.max(1, #lines - 11)
	for i = first, #lines do
		local line = lines[i]
		children[#children + 1] = text((line.you and "you" or line.from) .. ": " .. line.text)
	end
	local function send()
		local msg = chatText:get()
		if msg and msg ~= "" then
			act({ action = "chat", text = msg })
			chatText:set("")
		end
	end
	children[#children + 1] = row({
		builtin.TextInputField{
			value = chatText:get(),
			placeholderText = "say something",
			onValueChange = function(value) chatText:set(value); send() end,
			onTyping = function(value) chatText:set(value) end,
		},
		button(_("Send"), send),
	})

	return column(children)
end

return lobby

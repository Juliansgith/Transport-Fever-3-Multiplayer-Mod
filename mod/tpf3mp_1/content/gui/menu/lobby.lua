-- The Multiplayer window's content, on the game's main menu (docs/LOBBY.md).
--
-- It shows the lobby the hook hands it and sends the player's choices back,
-- over the hook's request channel: `resolveutil.loadfile("tpf3mp_1::/tpf3mp/state.lua")`
-- answers with the launcher's lobby as a Lua table literal; an action is left as JSON in
-- `resolveutil.__tpf3mp_action` and then `resolveutil.loadfile("tpf3mp_1::/tpf3mp/act.lua")`
-- is called, which the hook answers after taking the JSON (the loader accepts
-- exactly one argument). Both files exist in the mod, because the game checks
-- that before it lets the loader run; the hook answers before the loader reads
-- them, so their contents never matter. Without a hook (a game Steam started)
-- this window is not reachable at all.
--
-- The hook passes everything on to the TPF3-MP launcher that started the
-- game, over its link (D17): the launcher connects, makes and joins rooms and
-- carries the chat; the window shows what the launcher says, a few times a
-- second. Connecting goes to the launcher's own server (D12).
--
-- Plain Lua, loaded by the mod's main_page.tl through `ug_require`; it uses
-- the same react, builtin and helpers the menu does, and the menu's own
-- classes (primary/secondary buttons, hint/highlight/error text, the
-- font-scale-* sizes) and icons, so it looks like the rest of the menu.

local react = ug_require "::/gui/main/react.lua"
local builtin = ug_require "::/gui/main/builtin.lua"
local gui_react_util = ug_require "::/gui/main/gui_react_util.tl"
local button_react_util = ug_require "::/gui/main/button_react_util.tl"

local lobby = {}

local ICON = {
	ready = "::/gui/menu/icons/symbol_check.tga",
	host = "::/gui/menu/icons/symbol_crown_laurels.tga",
	away = "::/gui/menu/icons/symbol_x.tga",
	lock = "::/gui/menu/icons/symbol_lock_unlocked.tga",
	player = "::/gui/menu/icons/profile.tga",
	alert = "::/gui/menu/icons/alert.tga",
	kick = "::/gui/menu/icons/trash.tga",
}

-- The window's content size and the two columns of the room view.
local WIDTH, HEIGHT = 920, 640
local LEFT, RIGHT = 500, 380
local FIELD = 380

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

-- Styling ---------------------------------------------------------------------

-- An inline style sheet: size as {w, h}, padding as {top, right, bottom,
-- left}. Those are what api.gui.StyleSheet offers (no margin, no minSize), so
-- spacing between elements is done with gap() below.
local function style(t)
	local s = api.gui.StyleSheet.new()
	if t.size then s.size = api.type.Vec2f.new(t.size[1], t.size[2]) end
	if t.padding then s.padding = api.type.Vec4f.new(t.padding[1], t.padding[2], t.padding[3], t.padding[4]) end
	return s
end

local function gap(px)
	return builtin.Component{
		meta = { styleSheet = style{ size = { px or 12, px or 12 } } },
		mouseTransparent = true,
		layout = builtin.BoxLayout{ children = {} },
	}
end

-- A row or column's children with a gap between each pair.
local function spaced(children, px)
	local out = {}
	for i, child in ipairs(children) do
		if i > 1 then out[#out + 1] = gap(px or 8) end
		out[#out + 1] = child
	end
	return out
end

local function label(text, class, sheet)
	return builtin.TextView{
		meta = { class = class or "font-scale-body", styleSheet = sheet },
		text = text or "",
	}
end

local function icon(path, px)
	px = px or 20
	return builtin.ImageView{
		meta = { styleSheet = style{ size = { px, px } } },
		path = path,
		scaling = builtin.type.ImageViewScaling.AutoFit,
	}
end

local function row(children, sheet)
	return builtin.Component{
		meta = { styleSheet = sheet },
		mouseTransparent = true,
		layout = builtin.BoxLayout{
			orientation = builtin.type.Orientation.Horizontal,
			children = children,
		},
	}
end

local function column(children, sheet)
	return builtin.Component{
		meta = { styleSheet = sheet },
		mouseTransparent = true,
		layout = builtin.BoxLayout{
			orientation = builtin.type.Orientation.Vertical,
			children = children,
		},
	}
end

local function button(text, onClick, class)
	return builtin.Button{
		meta = { class = class or "secondary" },
		content = builtin.TextView{ meta = { class = "font-scale-body" }, text = text },
		onClick = onClick,
	}
end

local function primary(text, onClick)
	return button(text, onClick, "primary")
end

local function field(caption, ref, placeholder, password)
	return column({
		label(caption, "font-scale-body, hint"),
		builtin.TextInputField{
			meta = { class = "font-scale-body", styleSheet = style{ size = { FIELD, 36 } } },
			value = ref:get(),
			placeholderText = placeholder,
			passwordMode = password or false,
			acceptOnFocusLoss = true,
			onValueChange = function(value) ref:set(value) end,
			onTyping = function(value) ref:set(value) end,
		},
		gap(10),
	})
end

local function heading(text)
	return column({ label(text, "font-scale-title-4"), gap(8) })
end

-- The window's content, rendered inside the Tpf3mpLobbyWindow recipe. ------

function lobby.content(onClose)
	local stateS = react.useState(nil)
	local problemS = react.useState(nil)
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

	-- The frame every view shares: a header with the title and the
	-- connection, a line for what went wrong or what just happened, the view,
	-- and a footer with the view's buttons.
	local function frame(status, body, footer)
		local children = {
			row({
				label(_("Multiplayer"), "font-scale-title-2"),
				gui_react_util.makeHorizontalSpacer(),
				status,
			}),
			gap(6),
		}
		if state and state.error then
			children[#children + 1] = row({ icon(ICON.alert, 18), label(state.error, "font-scale-body, error") })
		elseif state and state.notice then
			children[#children + 1] = label(state.notice, "font-scale-body, hint")
		else
			children[#children + 1] = gap(6)
		end
		if state and not state.linked then
			children[#children + 1] = label(
				_("This game has no link to the TPF3-MP launcher: start Transport Fever 3 from the launcher."),
				"font-scale-body, error")
		elseif state and not state.heard then
			children[#children + 1] = label(_("Waiting for the launcher..."), "font-scale-body, hint")
		end
		children[#children + 1] = gap(10)
		children[#children + 1] = body
		children[#children + 1] = gui_react_util.makeVerticalSpacer()
		children[#children + 1] = row(spaced(footer))
		return column(children, style{ size = { WIDTH, HEIGHT }, padding = { 16, 20, 16, 20 } })
	end

	-- No answer from the hook yet.
	if not state then
		return frame(
			label(_("Waiting for the hook..."), "font-scale-body, hint"),
			label(problemS:old() and ("The hook did not answer: " .. tostring(problemS:old())) or "", "font-scale-body, hint"),
			{ gui_react_util.makeHorizontalSpacer(), button(_("Close"), onClose) }
		)
	end

	-- Not connected: the connect form.
	if state.connection ~= "connected" then
		local connecting = state.connection == "connecting"
		return frame(
			label(connecting and _("Connecting...") or _("Not connected"), "font-scale-body, hint"),
			column({
				heading(_("Connect to the server")),
				label(state.server ~= "" and state.server or _("the launcher's server"), "font-scale-body, highlight"),
				gap(10),
				field(_("Your name"), name, state.name ~= "" and state.name or _("name")),
			}),
			{
				gui_react_util.makeHorizontalSpacer(),
				button(_("Close"), onClose),
				primary(connecting and _("Connecting...") or _("Connect"), function()
					local typed = name:get()
					if typed == nil or typed == "" then typed = state.name end
					act({ action = "connect", name = typed })
				end),
			}
		)
	end

	local status = row({
		icon(ICON.player, 18),
		label(tostring(state.name), "font-scale-body, highlight"),
		label("  @ " .. tostring(state.server), "font-scale-body, hint"),
	})

	local room = state.room
	if not room then
		-- Connected, no room: create one or join one, side by side.
		return frame(
			status,
			row({
				column({
					heading(_("Create a room")),
					field(_("Room name"), roomName, _("a name for the room")),
					field(_("Password (optional)"), password, "", true),
					row({ primary(_("Create"), function()
						act({ action = "create", room = roomName:get(), password = password:get(), max_players = 8 })
					end) }),
				}, style{ size = { LEFT, 0 } }),
				column({
					heading(_("Join a room")),
					field(_("Invite code"), invite, "K7QM2X"),
					field(_("Password (if it has one)"), password, "", true),
					row({ primary(_("Join"), function()
						act({ action = "join", invite = invite:get(), password = password:get() })
					end) }),
				}, style{ size = { RIGHT, 0 } }),
			}),
			{
				button(_("Disconnect"), function() act({ action = "disconnect" }) end),
				gui_react_util.makeHorizontalSpacer(),
				button(_("Close"), onClose),
			}
		)
	end

	-- In a room: players on the left, chat on the right.
	local you
	local memberRows = {}
	for _i, member in ipairs(room.members) do
		if member.you then you = member end
		local cells = {
			icon(member.ready and ICON.ready or ICON.away, 18),
			member.owner and icon(ICON.host, 18) or gap(24),
			label(member.name, member.you and "font-scale-body, highlight" or "font-scale-body"),
		}
		if member.you then cells[#cells + 1] = label("  " .. _("(you)"), "font-scale-body, hint") end
		if not member.connected then cells[#cells + 1] = label("  " .. _("away"), "font-scale-body, hint") end
		if member.content == "differs" then cells[#cells + 1] = label("  " .. _("world differs"), "font-scale-body, error") end
		cells[#cells + 1] = gui_react_util.makeHorizontalSpacer()
		if room.you_own and not member.you then
			cells[#cells + 1] = button_react_util.makeIconButton(nil, ICON.kick, function()
				act({ action = "kick", player = member.id })
			end, _("Kick"))
		end
		memberRows[#memberRows + 1] = row(spaced(cells, 6))
		memberRows[#memberRows + 1] = gap(6)
	end
	local readyCount = 0
	for _i, member in ipairs(room.members) do if member.ready then readyCount = readyCount + 1 end end

	local roomHeader = column({
		row({
			label(room.name, "font-scale-title-3"),
			room.has_password and icon(ICON.lock, 18) or gap(4),
		}),
		row({
			label(_("Invite code") .. "  ", "font-scale-body, hint"),
			label(room.invite, "font-scale-title-4, highlight"),
			gap(20),
			label(string.format("%d / %d  ", #room.members, room.max_players), "font-scale-body, hint"),
			label(string.format(_("%d ready"), readyCount), "font-scale-body, hint"),
		}),
		gap(12),
	})

	local players = column({
		roomHeader,
		heading(_("Players")),
		column(memberRows),
	}, style{ size = { LEFT, 0 } })

	local lines = state.chat or {}
	local chatRows = {}
	local first = math.max(1, #lines - 40)
	for i = first, #lines do
		local line = lines[i]
		chatRows[#chatRows + 1] = row({
			label(line.from .. ": ", line.you and "font-scale-body, highlight" or "font-scale-body, hint"),
			label(line.text, "font-scale-body"),
		})
		chatRows[#chatRows + 1] = gap(3)
	end
	if #chatRows == 0 then
		chatRows[1] = label(_("Nothing said yet."), "font-scale-body, hint")
	end
	local function send()
		local msg = chatText:get()
		if msg and msg ~= "" then
			act({ action = "chat", text = msg })
			chatText:set("")
		end
	end
	local chat = column({
		heading(_("Chat")),
		builtin.ScrollArea{
			meta = { styleSheet = style{ size = { RIGHT, HEIGHT - 300 } } },
			horizontalPolicy = builtin.type.ScrollBarPolicy.AlwaysOff,
			verticalPolicy = builtin.type.ScrollBarPolicy.AsNeeded,
			content = column(chatRows),
		},
		gap(8),
		row({
			builtin.TextInputField{
				meta = { class = "font-scale-body", styleSheet = style{ size = { RIGHT - 110, 36 } } },
				value = chatText:get(),
				placeholderText = _("Say something"),
				acceptOnFocusLoss = false,
				onValueChange = function(value) chatText:set(value); send() end,
				onTyping = function(value) chatText:set(value) end,
			},
			button(_("Send"), send),
		}),
	}, style{ size = { RIGHT, 0 } })

	local footer = {
		button(_("Leave"), function() act({ action = "leave" }) end),
		gui_react_util.makeHorizontalSpacer(),
		button(_("Close"), onClose),
	}
	if you and you.ready then
		footer[#footer + 1] = button(_("Not ready"), function() act({ action = "ready", ready = false }) end)
	else
		footer[#footer + 1] = primary(_("Ready"), function() act({ action = "ready", ready = true }) end)
	end
	if room.you_own then
		footer[#footer + 1] = primary(_("Start"), function() act({ action = "start" }) end)
	end

	return frame(status, row({ players, gap(24), chat }), footer)
end

return lobby

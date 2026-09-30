-- The Multiplayer window's content, on the game's main menu (docs/LOBBY.md),
-- and the live lines of the menu's Multiplayer cards.
--
-- It shows the lobby the hook hands it and sends the player's choices back,
-- over the hook's request channel: `resolveutil.loadfile("tpf3mp_1::/tpf3mp/state.lua")`
-- answers with the launcher's lobby as a Lua table literal; an action is left as JSON in
-- `resolveutil.__tpf3mp_action` and then `resolveutil.loadfile("tpf3mp_1::/tpf3mp/act.lua")`
-- is called, which the hook answers after taking the JSON (the loader accepts
-- exactly one argument): "ok", or "error: " and why. Both files exist in the
-- mod, because the game checks that before it lets the loader run; the hook
-- answers before the loader reads them, so their contents never matter.
-- Without a hook (a game Steam started) this window is not reachable at all.
--
-- The hook passes everything on to the TPF3-MP launcher that started the
-- game, over its link (D17): the launcher connects, makes and joins rooms and
-- carries the chat; the window shows what the launcher says, a few times a
-- second. Connecting goes to the launcher's own server (D12). The server
-- lists no rooms: a room is joined by the invite its owner sends.
--
-- Plain Lua, loaded by the mod's main_page.tl through `ug_require`; it uses
-- the same react, builtin and helpers the menu does, the menu's own classes
-- (primary and secondary buttons, the font-scale-* sizes, the default style
-- sheet's colours and tapes: success, warning, error, info) and icons, so it
-- looks like the rest of the menu. crates/tpf3mp-hook/src/lobby.rs renders it
-- against a stand-in for the game's GUI (tests/lua/fake_menu.lua) in every
-- state and clicks its buttons.

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
	loading = "::/gui/menu/icons/loading.tga",
	save = "::/gui/menu/icons/load_game.tga",
	multiplayer = "tpf3mp_1::/gui/tpf3mp/icons/menu_multiplayer_50.tga",
}

-- The window's content size and the two columns of each view.
local WIDTH, HEIGHT = 940, 660
local LEFT, RIGHT = 470, 400
local FIELD = 400
-- Player counts a room can be created for, as the launcher offers them.
local MIN_PLAYERS, MAX_PLAYERS, DEFAULT_PLAYERS = 2, 16, 4
-- How often the window asks the hook for the lobby, in seconds, and how
-- many of those asks an action is shown as under way at most.
local POLL = 0.4
local PENDING_POLLS = 20

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

-- A table literal in an empty environment: Lua 5.2's load, or 5.1's.
local function evaluate(text)
	local chunk, err
	if setfenv then
		chunk, err = loadstring(text, "=tpf3mp-state")
		if chunk then setfenv(chunk, {}) end
	else
		chunk, err = load(text, "=tpf3mp-state", "t", {})
	end
	if not chunk then
		return nil, err
	end
	local ok, value = pcall(chunk)
	if not ok then
		return nil, value
	end
	return value
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
	local state, err = evaluate("return " .. reply)
	if type(state) ~= "table" then
		return nil, "bad state: " .. tostring(err or state)
	end
	return state
end
lobby.fetchState = fetchState

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
-- Returns nil when the hook took it, or why it did not.
local function act(fields)
	local parts = {}
	local keys = {}
	for key in pairs(fields) do keys[#keys + 1] = key end
	table.sort(keys)
	for _i, key in ipairs(keys) do
		local value = fields[key]
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
		return "the game's loader is not there"
	end
	resolveutil.__tpf3mp_action = json
	local reply, why = ask("act")
	resolveutil.__tpf3mp_action = nil
	if not reply then
		say("action " .. tostring(fields.action) .. " not sent: " .. tostring(why))
		return "not sent: " .. tostring(why)
	elseif reply ~= "ok" then
		say("action " .. tostring(fields.action) .. ": " .. reply)
		return (reply:gsub("^error: ", ""))
	end
	return nil
end

-- What the lobby says, in words -----------------------------------------------

local function sizeText(bytes)
	bytes = tonumber(bytes) or 0
	if bytes >= 1e9 then return string.format("%.1f GB", bytes / 1e9) end
	if bytes >= 1e6 then return string.format("%.1f MB", bytes / 1e6) end
	if bytes >= 1e3 then return string.format("%d kB", math.floor(bytes / 1e3 + 0.5)) end
	return string.format("%d B", bytes)
end

local function serverName(state)
	if state.server and state.server ~= "" then return state.server end
	return _("the TPF3-MP server")
end

local function you(room)
	for _i, member in ipairs(room and room.members or {}) do
		if member.you then return member end
	end
	return nil
end

local function readyCount(room)
	local count = 0
	for _i, member in ipairs(room.members or {}) do
		if member.ready then count = count + 1 end
	end
	return count
end

local function everyoneReady(room)
	return #(room.members or {}) > 0 and readyCount(room) == #room.members
end

-- The room's world in this game, in words, and how far it is (0 to 1), or
-- nil when there is none of the room's.
local function worldText(state)
	if state.world == "fetching" then
		local total = tonumber(state.total) or 0
		local bytes = tonumber(state.bytes) or 0
		if total > 0 then
			local done = math.min(1, bytes / total)
			return string.format(_("Receiving the room's world: %d%% (%s of %s)"),
				math.floor(done * 100), sizeText(bytes), sizeText(total)), done
		end
		return _("Receiving the room's world..."), 0
	elseif state.world == "loading" then
		return _("Loading the room's world..."), 1
	elseif state.world == "playing" then
		return _("Playing the room's game"), 1
	end
	return nil
end
lobby.worldText = worldText

-- Where the player is, for the Multiplayer cards on the main menu: a line
-- under the card's title.
function lobby.summary(state)
	if not state then
		return _("Play together online")
	end
	if not state.linked then
		return _("Start the game from the TPF3-MP launcher")
	end
	if state.connection == "connecting" then
		return _("Connecting...")
	end
	if state.connection ~= "connected" then
		return _("Play together online")
	end
	local room = state.room
	if not room then
		return string.format(_("Online on %s"), serverName(state))
	end
	local text = worldText(state)
	if text then return text end
	return string.format(_("%s · %d/%d players · %d ready"), room.name, #room.members,
		room.max_players, readyCount(room))
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

-- A smaller, quieter line, as the menu's cards write their descriptions.
local function note(text, class)
	return label(text, "font-scale-annotation" .. (class and (", " .. class) or ""))
end

-- A word on a coloured tape, as the game marks states: success, warning,
-- error or info.
local function badge(text, tone)
	return label(" " .. text .. " ", "font-scale-annotation, " .. (tone or "info") .. "-tape")
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

local function button(text, onClick, class, enabled, tooltip)
	return builtin.Button{
		meta = {
			class = class or "secondary",
			enabled = enabled ~= false,
			tooltip = tooltip,
		},
		content = builtin.TextView{ meta = { class = "font-scale-body" }, text = text },
		onClick = onClick,
	}
end

local function primary(text, onClick, enabled, tooltip)
	return button(text, onClick, "primary", enabled, tooltip)
end

local function input(ref, placeholder, width, params)
	params = params or {}
	return builtin.TextInputField{
		meta = { class = "font-scale-body", styleSheet = style{ size = { width or FIELD, 36 } } },
		value = ref:get(),
		placeholderText = placeholder,
		passwordMode = params.password or false,
		maxLength = params.maxLength,
		acceptOnFocusLoss = params.acceptOnFocusLoss ~= false,
		resetValueOnCancel = false,
		onValueChange = function(value)
			ref:set(value)
			if params.onEnter then params.onEnter(value) end
		end,
		onTyping = function(value) ref:set(value) end,
	}
end

local function field(caption, ref, placeholder, params)
	return column({
		note(caption),
		gap(4),
		input(ref, placeholder, FIELD, params),
		gap(12),
	})
end

-- A choice from a list: `items` as { value, text }.
local function choice(caption, value, items, onChange, explain)
	local entries = {}
	for _i, item in ipairs(items) do
		entries[#entries + 1] = builtin.ComboBoxItem{
			value = item[1],
			content = builtin.TextView{ meta = { class = "font-scale-body" }, text = item[2] },
		}
	end
	local children = {
		note(caption),
		gap(4),
		builtin.Component{
			meta = { styleSheet = style{ size = { FIELD, 36 } } },
			layout = builtin.BoxLayout{
				children = {
					builtin.ComboBox{ value = value, items = entries, onValueChange = onChange },
				},
			},
		},
	}
	if explain then
		children[#children + 1] = gap(4)
		children[#children + 1] = note(explain)
	end
	children[#children + 1] = gap(12)
	return column(children)
end

local function heading(text, sub)
	local children = { label(text, "font-scale-title-3") }
	if sub then
		children[#children + 1] = gap(2)
		children[#children + 1] = note(sub)
	end
	children[#children + 1] = gap(12)
	return column(children)
end

-- The window's content, rendered inside the Tpf3mpLobbyWindow recipe. ------

-- `focus` is what the card that opened the window is about: "join" puts
-- the invite first.
function lobby.content(onClose, focus)
	local stateS = react.useState(nil)
	local problemS = react.useState(nil)
	-- An action on its way: { text, polls left, the state it was sent in }.
	local pendingS = react.useState(nil)
	-- Why the hook refused the last action, until the next one.
	local refusedS = react.useState(nil)
	-- A question before kicking or leaving: { kind, id, name }.
	local confirmS = react.useState(nil)
	local name = react.useRef("")
	local roomName = react.useRef("")
	local invite = react.useRef("")
	local createPassword = react.useRef("")
	local joinPassword = react.useRef("")
	local chatText = react.useRef("")
	local playersS = react.useState(DEFAULT_PLAYERS)
	local rulesS = react.useState(nil)
	local saveS = react.useState(nil)

	-- What the view shows, in one string: when it changes, an action sent
	-- has been answered.
	local function signature(state)
		if not state then return "" end
		local room = state.room
		local me = you(room)
		return table.concat({
			tostring(state.connection), tostring(room and room.name), tostring(room and room.phase),
			tostring(room and #room.members), tostring(me and me.ready), tostring(state.error),
			tostring(state.notice), tostring(#(state.chat or {})),
		}, "|")
	end

	-- Poll the hook for the lobby a few times a second: the room and chat
	-- change without anything happening in this window.
	react.onStepTimer(function()
		local state, why = fetchState()
		if state then
			if problemS:old() ~= nil then problemS:set(nil) end
			local pending = pendingS:old()
			if pending then
				if pending[3] ~= signature(state) or pending[2] <= 1 then
					pendingS:set(nil)
				else
					pendingS:set({ pending[1], pending[2] - 1, pending[3] })
				end
			end
			stateS:set(state)
		elseif problemS:old() ~= why then
			problemS:set(why)
		end
	end, POLL, false)

	local state = stateS:old()

	-- Sends an action, and shows `doing` until the launcher answers.
	local function send(fields, doing)
		confirmS:set(nil)
		local refused = act(fields)
		refusedS:set(refused)
		if not refused and doing then
			pendingS:set({ doing, PENDING_POLLS, signature(stateS:old()) })
		end
	end

	local busy = pendingS:old() ~= nil

	-- The steps of playing together, and which is the player's now.
	local function steps()
		local room = state and state.room
		local me = you(room)
		local at
		if not state or state.connection ~= "connected" then at = 1
		elseif not room then at = 2
		elseif room.phase ~= "playing" and not (me and me.ready) then at = 3
		elseif room.phase ~= "playing" then at = 4
		else at = 5 end
		local words = { _("Connect"), _("Create or join a room"), _("Get ready"), _("Start"), _("Play") }
		local children = {}
		for i, word in ipairs(words) do
			if i > 1 then children[#children + 1] = note("  >  ") end
			local class = (i < at and "success") or (i == at and "info") or nil
			children[#children + 1] = note(string.format("%d  %s", i, word), class)
		end
		return row(children)
	end

	-- The frame every view shares: the title and the connection, the steps,
	-- a line for what went wrong, what is under way or what just happened,
	-- the room's world when it is coming, the view, and a footer with the
	-- view's buttons.
	local function frame(status, body, footer)
		local children = {
			row({
				icon(ICON.multiplayer, 32),
				gap(10),
				label(_("Multiplayer"), "font-scale-title-2"),
				gui_react_util.makeHorizontalSpacer(),
				status,
			}),
			gap(8),
			steps(),
			gap(10),
		}
		local problem = refusedS:old() or (state and state.error)
		if problem then
			children[#children + 1] = row({ icon(ICON.alert, 18), gap(6), label(problem, "font-scale-body, error") })
		elseif pendingS:old() then
			children[#children + 1] = row({ icon(ICON.loading, 18), gap(6), label(pendingS:old()[1], "font-scale-body, info") })
		elseif state and state.notice then
			children[#children + 1] = note(state.notice)
		else
			children[#children + 1] = gap(18)
		end
		if state and not state.linked then
			children[#children + 1] = label(
				_("This game has no link to the TPF3-MP launcher: close it and start Transport Fever 3 from the launcher."),
				"font-scale-body, error")
		elseif state and not state.heard then
			children[#children + 1] = note(_("Waiting for the launcher..."))
		end
		local world, done = state and worldText(state)
		if world then
			children[#children + 1] = gap(8)
			children[#children + 1] = row({
				label(world, "font-scale-body, info"),
				gap(12),
				builtin.Component{
					meta = { styleSheet = style{ size = { 260, 18 } } },
					layout = builtin.BoxLayout{ children = { builtin.ProgressBar{ value = done } } },
				},
			})
		end
		if state and state.differences then
			children[#children + 1] = gap(4)
			children[#children + 1] = label(_("Your game differs from the room's: ") .. state.differences,
				"font-scale-body, warning")
		end
		children[#children + 1] = gap(14)
		children[#children + 1] = body
		children[#children + 1] = gui_react_util.makeVerticalSpacer()
		children[#children + 1] = row(spaced(footer))
		return column(children, style{ size = { WIDTH, HEIGHT }, padding = { 16, 20, 16, 20 } })
	end

	-- No answer from the hook yet.
	if not state then
		local why = problemS:old()
		return frame(
			note(_("Waiting for the hook...")),
			label(why and (_("The hook did not answer: ") .. tostring(why)) or "", "font-scale-body, error"),
			{ gui_react_util.makeHorizontalSpacer(), button(_("Close"), onClose) }
		)
	end

	local canAct = state.linked and state.heard

	-- Not connected: the connect form.
	if state.connection ~= "connected" then
		local connecting = state.connection == "connecting"
		local function connect()
			local typed = name:get()
			if typed == nil or typed:match("^%s*$") then typed = state.name end
			send({ action = "connect", name = typed }, _("Connecting to ") .. serverName(state) .. "...")
		end
		return frame(
			connecting and badge(_("Connecting"), "info") or badge(_("Not connected"), "warning"),
			column({
				heading(_("Play Transport Fever 3 together"),
					string.format(_("Connect to %s, then create a room or join a friend's."), serverName(state))),
				field(_("Your name, as the others see it"), name, state.name ~= "" and state.name or _("Your name"),
					{ maxLength = 32, acceptOnFocusLoss = true }),
				row({
					primary(connecting and _("Connecting...") or string.format(_("Connect to %s"), serverName(state)),
						connect, canAct and not connecting and not busy),
				}),
			}),
			{ gui_react_util.makeHorizontalSpacer(), button(_("Close"), onClose) }
		)
	end

	local status = row({
		badge(_("Online"), "success"),
		gap(8),
		icon(ICON.player, 18),
		gap(4),
		label(tostring(state.name), "font-scale-body"),
		note("  @ " .. serverName(state)),
	})

	local room = state.room
	if not room then
		-- Connected, no room: create one or join one, side by side.
		local rules = state.rules or {}
		local rulesItems = {}
		for i, offered in ipairs(rules) do
			rulesItems[#rulesItems + 1] = { offered.name, i == 1 and (offered.name .. _(" (default)")) or offered.name }
		end
		local pickedRules = rulesS:old()
		local explainRules
		for _i, offered in ipairs(rules) do
			if offered.name == (pickedRules or (rules[1] and rules[1].name)) then explainRules = offered.description end
		end
		local saves = state.saves or {}
		local saveItems = {}
		for _i, save in ipairs(saves) do saveItems[#saveItems + 1] = { save, save } end
		saveItems[#saveItems + 1] = { "", _("None: I load a world myself") }
		local pickedSave = saveS:old()
		if pickedSave == nil then
			pickedSave = ""
			for _i, save in ipairs(saves) do
				if save == state.start_save then pickedSave = save end
			end
			if pickedSave == "" and saves[1] then pickedSave = saves[1] end
		end
		local playersItems = {}
		for n = MIN_PLAYERS, MAX_PLAYERS do
			playersItems[#playersItems + 1] = { n, string.format(_("%d players"), n) }
		end

		local function create()
			local named = roomName:get()
			if named == nil or named:match("^%s*$") then
				named = string.format(_("%s's room"), state.name)
			end
			send({
				action = "create",
				room = named,
				password = createPassword:get() or "",
				max_players = playersS:old(),
				rules = pickedRules or "",
				start_save = pickedSave,
			}, _("Creating the room..."))
		end
		local function join()
			local code = (invite:get() or ""):gsub("%s", ""):upper()
			if code == "" then
				refusedS:set(_("Type the invite code a friend sent you."))
				return
			end
			send({ action = "join", invite = code, password = joinPassword:get() or "" }, _("Joining the room..."))
		end

		local createColumn = column({
			heading(_("Create a room"), _("You own it: you start its game, and can remove players.")),
			field(_("Room name"), roomName, string.format(_("%s's room"), state.name), { maxLength = 48 }),
			choice(_("Start from this save"), pickedSave, saveItems, function(value) saveS:set(value) end,
				pickedSave ~= "" and _("Every player's game loads it from the menu when you start.")
					or _("Load a world in the game once in the room: it is saved for everyone.")),
			row({
				choice(_("Players"), playersS:old(), playersItems, function(value) playersS:set(value) end),
			}),
			#rulesItems > 1 and choice(_("Rules"), pickedRules or rulesItems[1][1], rulesItems,
				function(value) rulesS:set(value) end, explainRules) or gap(0),
			field(_("Password (optional)"), createPassword, "", { password = true, maxLength = 64 }),
			row({ primary(_("Create room"), create, canAct and not busy) }),
		}, style{ size = { LEFT, 0 } })

		local joinColumn = column({
			heading(_("Join a room"), _("Rooms are private: ask a friend for their invite.")),
			field(_("Invite code"), invite, "K7QM2X", { maxLength = 128, onEnter = nil }),
			field(_("Password (if the room has one)"), joinPassword, "", { password = true, maxLength = 64 }),
			row({ primary(_("Join room"), join, canAct and not busy) }),
			gap(16),
			note(string.format(_("%s lists no rooms: an invite is the way in."), serverName(state))),
		}, style{ size = { RIGHT, 0 } })

		local columns = focus == "join" and { joinColumn, gap(40), createColumn } or { createColumn, gap(40), joinColumn }
		return frame(
			status,
			row(columns),
			{
				button(_("Disconnect"), function() send({ action = "disconnect" }, _("Disconnecting...")) end, nil, canAct),
				gui_react_util.makeHorizontalSpacer(),
				button(_("Close"), onClose),
			}
		)
	end

	-- In a room: players on the left, chat on the right.
	local me = you(room)
	local playing = room.phase == "playing"
	local confirm = confirmS:old()
	local memberRows = {}
	for _i, member in ipairs(room.members) do
		local cells = {
			icon(member.ready and ICON.ready or ICON.away, 18),
			gap(8),
			label(member.name, member.you and "font-scale-body, info" or "font-scale-body"),
		}
		local function tag(text, tone)
			cells[#cells + 1] = gap(6)
			cells[#cells + 1] = badge(text, tone)
		end
		if member.owner then tag(_("Owner"), "info") end
		if member.you then tag(_("You"), "info") end
		if not member.connected then tag(_("Away"), "warning") end
		if not playing then
			if member.ready then tag(_("Ready"), "success") else tag(_("Not ready"), "warning") end
		end
		if member.content == "differs" then tag(_("Other mods"), "error") end
		cells[#cells + 1] = gui_react_util.makeHorizontalSpacer()
		if room.you_own and not member.you then
			if confirm and confirm.kind == "kick" and confirm.id == member.id then
				cells[#cells + 1] = button(_("Remove"), function()
					send({ action = "kick", player = member.id }, string.format(_("Removing %s..."), member.name))
				end, "primary", canAct)
				cells[#cells + 1] = gap(4)
				cells[#cells + 1] = button(_("Keep"), function() confirmS:set(nil) end)
			else
				cells[#cells + 1] = button_react_util.makeIconButton(nil, ICON.kick, function()
					confirmS:set({ kind = "kick", id = member.id, name = member.name })
				end, string.format(_("Remove %s from the room"), member.name))
			end
		end
		memberRows[#memberRows + 1] = row(cells, style{ size = { LEFT, 34 } })
		memberRows[#memberRows + 1] = gap(4)
	end

	local roomHeader = column({
		row({
			label(room.name, "font-scale-title-3"),
			gap(8),
			room.has_password and icon(ICON.lock, 18) or gap(0),
			gui_react_util.makeHorizontalSpacer(),
		}),
		gap(6),
		row({
			note(_("Invite code  ")),
			label(room.invite ~= "" and room.invite or "-", "font-scale-title-4, info"),
		}),
		note(_("Send it to friends: they join with it from their game's Multiplayer window.")),
		gap(10),
		note(string.format(_("%d of %d players  ·  %d ready"), #room.members, room.max_players, readyCount(room))),
		gap(8),
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
			label(line.from .. ":", line.you and "font-scale-body, info" or "font-scale-body, success"),
			gap(6),
			label(line.text, "font-scale-body"),
		})
		chatRows[#chatRows + 1] = gap(3)
	end
	if #chatRows == 0 then
		chatRows[1] = note(_("Nothing said yet. Say hello!"))
	end
	local function sendChat()
		local msg = chatText:get()
		if msg and not msg:match("^%s*$") then
			chatText:set("")
			send({ action = "chat", text = msg }, nil)
		end
	end
	local chat = column({
		heading(_("Chat")),
		builtin.ScrollArea{
			meta = { styleSheet = style{ size = { RIGHT, HEIGHT - 330 } } },
			horizontalPolicy = builtin.type.ScrollBarPolicy.AlwaysOff,
			verticalPolicy = builtin.type.ScrollBarPolicy.AsNeeded,
			content = column(chatRows),
		},
		gap(8),
		row({
			input(chatText, _("Say something to the room"), RIGHT - 100,
				{ maxLength = 280, acceptOnFocusLoss = false, onEnter = function() sendChat() end }),
			gap(8),
			button(_("Send"), sendChat, nil, canAct),
		}),
	}, style{ size = { RIGHT, 0 } })

	local footer = {}
	if confirm and confirm.kind == "leave" then
		footer[#footer + 1] = label(_("Leave the room?"), "font-scale-body, warning")
		footer[#footer + 1] = button(_("Leave"), function() send({ action = "leave" }, _("Leaving the room...")) end,
			"primary", canAct)
		footer[#footer + 1] = button(_("Stay"), function() confirmS:set(nil) end)
	else
		footer[#footer + 1] = button(_("Leave room"), function() confirmS:set({ kind = "leave" }) end, nil, canAct)
	end
	footer[#footer + 1] = gui_react_util.makeHorizontalSpacer()
	footer[#footer + 1] = button(_("Close"), onClose)
	if playing then
		footer[#footer + 1] = note(_("The room's game is under way."))
	else
		if me and me.ready then
			footer[#footer + 1] = button(_("Not ready"), function()
				send({ action = "ready", ready = false }, nil)
			end, nil, canAct and not busy)
		else
			footer[#footer + 1] = primary(_("Ready"), function()
				send({ action = "ready", ready = true }, _("Getting ready..."))
			end, canAct and not busy)
		end
		if room.you_own then
			local all = everyoneReady(room)
			footer[#footer + 1] = primary(_("Start the game"), function()
				send({ action = "start" }, _("Starting the room's game..."))
			end, canAct and all and not busy,
				all and _("Every player's game loads the room's world") or _("Waiting for everyone to be ready"))
		end
	end

	return frame(status, row({ players, gap(30), chat }), footer)
end

-- The live line under a Multiplayer card on the main menu, from the lobby
-- the hook has: its own recipe, so only it redraws when the lobby changes.
lobby.CardLine = react.RegisterRecipe("Tpf3mpCardLine", function(params)
	local lineS = react.useState(lobby.summary(nil))
	react.onStepTimer(function()
		local state = fetchState()
		local line = params and params.line and params.line(state) or lobby.summary(state)
		if line ~= lineS:old() then lineS:set(line) end
	end, 1.0, false)
	return builtin.TextView{
		meta = { class = "font-scale-annotation, annotation" },
		text = lineS:old(),
	}
end)

-- What the "Join a friend" card says: the room once in one.
function lobby.joinLine(state)
	local room = state and state.room
	if room and room.invite ~= "" then
		return string.format(_("Your room: invite %s"), room.invite)
	end
	return _("With the invite code they send you")
end

return lobby

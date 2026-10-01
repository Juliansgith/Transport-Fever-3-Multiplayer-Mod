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

-- Stock world setup reads this selection. Entering it from a room must
-- include the multiplayer script, while preserving the player's mods.
function lobby.prepareNewWorld()
	local config = api.type.AppConfig.new(api.util.getAppConfig())
	local menu = {}
	for key, value in pairs(config.mainMenuState or {}) do menu[key] = value end
	local mods, found = {}, false
	for _, name in ipairs(menu.activeModsState or {}) do
		mods[#mods + 1] = name
		if name == "tpf3mp_1" then found = true end
	end
	if not found then mods[#mods + 1] = "tpf3mp_1" end
	menu.activeModsState = mods
	config.mainMenuState = menu
	api.util.setAppConfig(config, false)
end

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

-- The window's content size and the two columns of each view. The window
-- itself is centred on the menu (main_page.tl's Tpf3mpLobbyWindow).
local WIDTH, HEIGHT = 960, 620
local LEFT, RIGHT = 440, 440
local FIELD = 400
-- Player counts a room can be created for, as the launcher offers them.
local MIN_PLAYERS, MAX_PLAYERS, DEFAULT_PLAYERS = 2, 16, 4
-- How often the window asks the hook for the lobby, in seconds, and how
-- many of those asks an action is shown as under way at most.
local POLL = 0.4
local PENDING_POLLS = 20
-- How many polls Copy says "Copied" for: about two seconds.
local COPIED_POLLS = 5

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
	return state, nil, reply
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

-- The server's name as the window shows it: never its address. The
-- launcher names its default server ("EU"); any other is "another server",
-- as a name that looks like host:port or an IP address is.
local function serverName(state)
	local name = state.server
	local elsewhere = state.server_default and state.server_default ~= ""
		and state.server_address and state.server_address ~= state.server_default
	if elsewhere or type(name) ~= "string" or name == "" then
		return elsewhere and _("another server") or _("the TPF3-MP server")
	end
	if name:find(":%d+$") or name:find("^%d+%.%d+%.%d+%.%d+") or name:find("^%[") then
		return _("another server")
	end
	return name
end
lobby.serverName = serverName

-- `text` with any server address in it (an IP address, host:port) put as
-- "the server": the window names servers, never their addresses.
local function hideAddress(text)
	if type(text) ~= "string" then return text end
	text = text:gsub("%[[%x:]+%]:%d+", _("the server"))
	text = text:gsub("%d+%.%d+%.%d+%.%d+:%d+", _("the server"))
	text = text:gsub("%d+%.%d+%.%d+%.%d+", _("the server"))
	text = text:gsub("[%w%-]+%.[%w%.%-]+:%d+", _("the server"))
	text = text:gsub("localhost:%d+", _("the server"))
	return text
end
lobby.hideAddress = hideAddress

-- A room's invite code alone: without its own server, the launcher puts
-- the server's address before the code.
local function inviteCode(invite)
	if type(invite) ~= "string" then return "" end
	return invite:match("(%S+)%s*$") or invite
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
	if not state.room then return nil end
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

-- The player closed the window (main_page.tl's close): the hook has nothing
-- to close when the room's world comes up.
function lobby.closed()
	if type(resolveutil) == "table" then
		pcall(function() resolveutil.__tpf3mp_close = nil end)
	end
end

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
-- spacing between elements is done with gap() below. A size of -1 leaves
-- that side to the content, as the game's style sheets write it; a size of
-- 0 is a size of nothing, and hides everything inside (the game drew the
-- create and join columns, sized {w, 0}, as two thin bars).
local AUTO = -1
local function style(t)
	local s = api.gui.StyleSheet.new()
	if t.size then s.size = api.type.Vec2f.new(t.size[1], t.size[2]) end
	if t.padding then s.padding = api.type.Vec4f.new(t.padding[1], t.padding[2], t.padding[3], t.padding[4]) end
	return s
end

local function gap(px)
	px = math.max(1, px or 12)
	return builtin.Component{
		meta = { styleSheet = style{ size = { px, px } } },
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
	}, style{ size = { FIELD, 76 } })
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
	return column(children, style{ size = { FIELD, explain and 94 or 76 } })
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

-- The room browser --------------------------------------------------------------

-- Cards a row of the room list holds, and a card's size: the game's
-- new-game climate cards (`small-rectangle-card`, 316 by 181) a little
-- smaller, three to the window's width.
local CARDS_PER_ROW = 3
local CARD_WIDTH, CARD_HEIGHT = 272, 156
-- The first page's two cards, Join and Host.
local CHOICE_WIDTH, CHOICE_HEIGHT = 420, 240
-- Polls between two asks for the room list while it is shown.
local LIST_POLLS = 25

-- The game's own card button and label, as its main menu builds them; nil
-- if the game has none, and a plain button with a picture is drawn instead.
local cards = (function()
	local ok, util = pcall(ug_require, "::/gui/menu/menu_icon_react_util.tl")
	if ok and type(util) == "table" and util.CardButton and util.makeCardLabelBottomComponent then
		return util
	end
	return nil
end)()

-- The game's pictures of each climate on its main menu's New Game card,
-- for a climate whose own description has no picture.
local CLIMATE_PICTURES = {
	temperate = "::/gui/menu/images/temperate_ingame.tga",
	subarctic = "::/gui/menu/images/subarctic_ingame.tga",
	tropical = "::/gui/menu/images/tropical_ingame.tga",
	dry = "::/gui/menu/images/dry_ingame.tga",
}
local UNKNOWN_PICTURE = "::/gui/menu/images/m05_ingame.tga"

-- The game's description of the climate `map` names (`temperate`), if it
-- has one: what its New Game page shows, its name and its picture.
local function climate(map)
	if type(map) ~= "string" or map == "" then return nil end
	local ok, desc = pcall(function()
		local rep = app.res.climateRep
		local id = rep.find("::/climates/" .. map .. "/" .. map .. ".clima")
		if id == nil or id < 0 then return nil end
		return rep.get(id).desc
	end)
	return ok and desc or nil
end

-- The climate's name, as players read it.
function lobby.climateName(map)
	local desc = climate(map)
	if desc and type(desc.name) == "string" and desc.name ~= "" then return desc.name end
	if type(map) ~= "string" or map == "" then return _("Unknown map") end
	return (map:gsub("^%l", string.upper))
end

-- The picture of the climate `map` names.
function lobby.climatePicture(map)
	local desc = climate(map)
	if desc and type(desc.icon) == "string" and desc.icon ~= "" then return desc.icon end
	return CLIMATE_PICTURES[map] or UNKNOWN_PICTURE
end

-- A save's climate and year, as the game's Load Game page reads them
-- (savegame_react_util.tl): the save's configDict "climate" and its
-- metadata's date. Read once, in the background (app.getSavegameInfo);
-- until then, or when the game cannot say, the map is "" and the year 0.
local saveDetailsRead = {}
local function yearOf(metadata)
	local ok, year = pcall(function() return api.type.Date.new(metadata.date).year end)
	if ok and type(year) == "number" and year > 1000 and year < 3000 then return math.floor(year) end
	if type(metadata.startYear) == "number" and metadata.startYear > 1000 then return math.floor(metadata.startYear) end
	return 0
end
function lobby.saveDetails(name)
	local read = saveDetailsRead[name]
	if not read then
		read = { map = "", year = 0 }
		saveDetailsRead[name] = read
		local ok, async = pcall(function()
			local namespace = app.SaveGameNamespace.getSavegame()
			for _i, info in ipairs(app.findAllSavegames(namespace) or {}) do
				if info.saveName == name or info.saveName == name .. ".sav" then
					local id = api.type.SavegameId.new()
					id.path = info.path
					id.saveGameName = info.saveName
					id.saveGameNamespace = namespace
					return app.getSavegameInfo(id)
				end
			end
			return nil
		end)
		read.async = ok and async or nil
		if not ok then say("the save " .. tostring(name) .. " could not be read: " .. tostring(async)) end
	end
	if read.async then
		local ok, done = pcall(function() return read.async:isCompleted() end)
		if ok and done then
			local got, data = pcall(function() return read.async:get() end)
			read.async = nil
			if got and data and data.info then
				for _i, pair in ipairs(data.info.configDict or {}) do
					if pair[1] == "climate" and type(pair[2]) == "string" then
						read.map = pair[2]:match("([%w_]+)%.clima$") or pair[2]
					end
				end
				if data.info.metadata then read.year = yearOf(data.info.metadata) end
			end
		elseif not ok then
			read.async = nil
		end
	end
	return read
end

-- The save a room starts from, in a line: its name, its map and year when
-- the room knows them, and whether the room has it yet.
function lobby.startLine(start)
	local parts = { start.name }
	if type(start.map) == "string" and start.map ~= "" then parts[#parts + 1] = lobby.climateName(start.map) end
	if (tonumber(start.year) or 0) > 0 then parts[#parts + 1] = tostring(start.year) end
	if not start.arrived then parts[#parts + 1] = _("on its way to the room") end
	return table.concat(parts, " · ")
end

-- Whether the room's game must wait for its start save: the owner's still
-- going up, or the room not having it yet.
function lobby.startWaits(room)
	return room.upload ~= nil or (room.start ~= nil and not room.start.arrived)
end

-- Polls a pick of the room's start save waits at most for the game to read
-- the save's map and year.
local PICK_POLLS = 8

-- Banners ---------------------------------------------------------------------

local banners = ug_require "tpf3mp_1::/scripts/tpf3mp/banners.lua"
local BANNERS = banners.LIST
lobby.BANNERS = BANNERS
lobby.bannerOf = banners.of
lobby.bannerPicture = banners.picture
lobby.portraitOf = banners.portraitOf
lobby.portraitName = banners.portraitName
lobby.portraitPicture = banners.portrait

-- A room member's size as a card, two to a row of the players' column.
local MEMBER_WIDTH, MEMBER_HEIGHT = 208, 128
local PORTRAIT_SIZE = 44

-- A picture card in the main menu's style: title and a line under it, a
-- word on the right; `onClick` nil for a card that only shows.
local function pictureCard(picture, title, line, right, onClick, enabled, width, height, marks)
	local card
	if cards then
		card = cards.CardButton{
			bottomComponent = cards.makeCardLabelBottomComponent(title, line, right, nil, false),
			onClick = onClick or function() end,
			tooltip = title,
			images = { picture },
			initialImageIndex = 1,
			class = "small-rectangle-card",
			enabled = enabled ~= false,
			extraChildren = marks or {},
		}
	else
		card = builtin.Button{
			meta = { enabled = enabled ~= false },
			content = column({ icon(picture, height - 60), label(title, "font-scale-body"), note(line or "") }),
			onClick = onClick or function() end,
		}
	end
	return builtin.Component{
		meta = { styleSheet = style{ size = { width, height } } },
		layout = builtin.BoxLayout{ children = { card } },
	}
end
lobby.pictureCard = pictureCard

-- Where a member's game is with the room's world: its download, its load,
-- then in the game; before the room starts, whether it is ready.
function lobby.memberStage(member, playing)
	if member.loading == "fetching" then
		return string.format(_("Downloading %d%%"), math.floor(tonumber(member.percent) or 0))
	elseif member.loading == "loading" then
		return _("Loading...")
	elseif playing then
		return member.connected and _("Playing") or nil
	end
	return member.ready and _("Ready") or _("Not ready")
end

-- A room member as a card: their banner, name, and what marks them.
function lobby.memberCard(member, playing)
	local marks = {}
	if member.you then marks[#marks + 1] = _("You") end
	if member.owner then marks[#marks + 1] = _("Owner") end
	if not member.connected then marks[#marks + 1] = _("Away") end
	marks[#marks + 1] = lobby.memberStage(member, playing)
	if member.content == "differs" then marks[#marks + 1] = _("Other mods") end
	local ready = member.ready and not playing and (member.loading or "") == "" and builtin.FloatingLayoutChild{
		h = 0.95,
		v = 0.06,
		item = builtin.ImageView{
			meta = { mouseTransparent = true, styleSheet = style{ size = { 22, 22 } } },
			path = ICON.ready,
		},
	} or nil
	-- A member who picked a portrait: it beside their card, which shows
	-- their key's banner (tpf3mp/banners.lua).
	local portrait = lobby.portraitOf(member)
	local card = pictureCard(lobby.bannerPicture(lobby.bannerOf(member)), member.name,
		table.concat(marks, " · "), member.you and _("You") or nil, nil, true,
		portrait and MEMBER_WIDTH - PORTRAIT_SIZE - 8 or MEMBER_WIDTH, MEMBER_HEIGHT, ready and { ready } or {})
	if not portrait then return card end
	return row({ icon(portrait, PORTRAIT_SIZE), gap(8), card })
end

-- A campaign character's portrait as a card of the banner picker: the
-- picture, the character's name, and whether it is yours.
local PORTRAIT_WIDTH, PORTRAIT_HEIGHT = 150, 190
function lobby.portraitCard(id, picked, onClick, enabled)
	return pictureCard(banners.portrait(id), banners.portraitName(id) or id, picked and _("Yours") or " ",
		nil, onClick, enabled, PORTRAIT_WIDTH, PORTRAIT_HEIGHT)
end

-- The pictures of the Host page's play styles. Co-op: the busy harbour of
-- the game's Campaign card, many vessels sharing one port. Competitive: the
-- rusted-out truck left in the desert on the loading screen of the
-- campaign's third mission, a built-in mod of the game; its path is the
-- mod's own (INFERRED to load at the main menu as the campaign's pictures
-- do).
local COOP_PICTURE = "::/gui/menu/images/campaign.tga"
local COMPETITIVE_PICTURE = "urbangames_campaign_mission_03::/gui/mission/m03_loadscreen.tga"
lobby.COOP_PICTURE = COOP_PICTURE
lobby.COMPETITIVE_PICTURE = COMPETITIVE_PICTURE

-- One play style as a card: picked, it says so.
function lobby.styleCard(competitive, picked, onClick, enabled)
	local title = competitive and _("Competitive") or _("Co-op")
	return pictureCard(competitive and COMPETITIVE_PICTURE or COOP_PICTURE,
		picked and ("> " .. title) or title,
		picked and _("Picked") or nil, nil, onClick, enabled, 190, 104)
end

-- A big choice of the first page (Join, Host), as a card in the main
-- menu's style.
function lobby.choiceCard(title, line, picture, onClick, enabled)
	local card
	if cards then
		card = cards.CardButton{
			bottomComponent = cards.makeCardLabelBottomComponent(title, line, nil, nil, true),
			onClick = onClick,
			tooltip = line,
			images = { picture },
			initialImageIndex = 1,
			class = "small-rectangle-card",
			enabled = enabled,
			extraChildren = {},
		}
	else
		card = builtin.Button{
			meta = { enabled = enabled },
			content = column({ icon(picture, CHOICE_HEIGHT - 70), label(title, "font-scale-title-3"), note(line) }),
			onClick = onClick,
		}
	end
	return builtin.Component{
		meta = { styleSheet = style{ size = { CHOICE_WIDTH, CHOICE_HEIGHT } } },
		layout = builtin.BoxLayout{ children = { card } },
	}
end

-- One public room of the list, as a card in the game's own style: the
-- picture of its map, its name, and players, companies and year under it.
function lobby.roomCard(listed, onClick, enabled)
	local title = listed.name
	local line = string.format(_("%d/%d players · %d %s · %s"), listed.players, listed.max_players,
		listed.companies, listed.companies == 1 and _("company") or _("companies"),
		listed.year > 0 and tostring(listed.year) or _("year unknown"))
	local right = (listed.competitive and _("Competitive") or _("Co-op")) .. " · "
		.. (listed.running and _("Playing") or lobby.climateName(listed.map))
	local lock = listed.has_password and builtin.FloatingLayoutChild{
		h = 0.95,
		v = 0.06,
		item = builtin.ImageView{
			meta = { mouseTransparent = true, styleSheet = style{ size = { 24, 24 } } },
			path = ICON.lock,
		},
	} or nil
	local card
	if cards then
		card = cards.CardButton{
			bottomComponent = cards.makeCardLabelBottomComponent(title, string.format(_("%d/%d players · %s"),
				listed.players, listed.max_players, listed.competitive and _("Competitive") or _("Co-op")), nil, nil, false),
			onClick = onClick,
			tooltip = title .. "\n" .. line .. "\n" .. right .. "\n"
				.. (listed.has_password and _("Has a password") or _("Join this room")),
			images = { lobby.climatePicture(listed.map) },
			initialImageIndex = 1,
			class = "small-rectangle-card",
			enabled = enabled,
			extraChildren = lock and { lock } or {},
		}
	else
		card = builtin.Button{
			meta = { enabled = enabled },
			content = column({
				icon(lobby.climatePicture(listed.map), CARD_HEIGHT - 60),
				label(title, "font-scale-body"),
				note(line),
				note(right),
			}),
			onClick = onClick,
		}
	end
	return builtin.Component{
		meta = { styleSheet = style{ size = { CARD_WIDTH, CARD_HEIGHT } } },
		layout = builtin.BoxLayout{ children = { card } },
	}
end

-- The window's content, rendered inside the Tpf3mpLobbyWindow recipe. ------

-- `focus` is what the card that opened the window is about: "join" puts
-- the invite first.
function lobby.content(onClose, focus, onNewGame)
	-- The hook calls this before loading, then lets the menu render one frame.
	-- Polling room state alone races the loader, which suspends menu callbacks.
	resolveutil.__tpf3mp_before_load = onClose
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
	-- The room page's pick of its start save while the game reads its map
	-- and year: { save, polls }.
	local pickS = react.useState(nil)
	-- The start save this window already told the room the map and year of.
	local describedRef = react.useRef(nil)
	-- The page shown: "choose" (Join or Host), "join" (the public rooms
	-- and an invite) or "host" (the room's settings); in a room, always the
	-- room's. Your mods show over it while modsS is on.
	local pageS = react.useState(nil)
	local modsS = react.useState(false)
	-- The banner picker, from the first page.
	local bannerS = react.useState(false)
	-- The Join page's Join with code popup.
	local codeS = react.useState(focus == "code")
	-- The server page, from the first page: whether it shows, the address
	-- typed, and why the launcher refused the last one.
	local serverS = react.useState(false)
	local serverText = react.useRef(nil)
	local serverErrorS = react.useState(nil)
	local publicS = react.useState("private")
	-- The Host page's play style: co-op (false) or competitive.
	local competitiveS = react.useState(false)
	local joiningS = react.useState(nil)
	local listAtRef = react.useRef(LIST_POLLS)
	-- The hook also closes the window as the room's world comes up, should
	-- it still be open then (crates/tpf3mp-hook/src/menu.rs, close_lobby),
	-- with the close it leaves here while it is open; closed by the player
	-- (lobby.closed), it leaves nothing.
	if type(resolveutil) == "table" then
		pcall(function() resolveutil.__tpf3mp_close = onClose end)
	end
	-- An explicit click may need a connection first. Keep that intention
	-- separate from the busy indicator; intermediate notices are not success.
	local queued = react.useRef(nil)
	local generate = react.useRef(false)
	local lastSnapshot = react.useRef(nil)
	local copiedS = react.useState(0)

	-- What the view shows, in one string: when it changes, an action sent
	-- has been answered.
	local function signature(state)
		if not state then return "" end
		local room = state.room
		local me = you(room)
		return table.concat({
			tostring(state.connection), tostring(room and room.name), tostring(room and room.phase),
			tostring(room and #room.members), tostring(me and me.ready), tostring(state.error),
			tostring(state.notice), tostring(#(state.chat or {})), tostring(state.server_address),
			tostring(room and room.start and room.start.name), tostring(room and room.upload and room.upload.save),
		}, "|")
	end

	-- The page `s` shows: the room's once in one; the first page until
	-- connected; otherwise the one picked, or the one the card that opened
	-- the window is about.
	local function pageOf(s)
		if s.room then return "room" end
		local picked = pageS:old() or (focus == "friend" and "friend" or (focus == "join" and "join" or "choose"))
		if picked == "room" then return "choose" end
		return picked
	end

	-- Poll the hook for the lobby a few times a second: the room and chat
	-- change without anything happening in this window.
	react.onStepTimer(function()
		if copiedS:old() > 0 then copiedS:set(copiedS:old() - 1) end
		local state, why, snapshot = fetchState()
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
			if snapshot ~= lastSnapshot:get() then
				lastSnapshot:set(snapshot)
				stateS:set(state)
			end
			local nextAction = queued:get()
			if nextAction then
				nextAction.left = nextAction.left - 1
				if state.error and state.error ~= nextAction.error or not state.linked or not state.heard or nextAction.left <= 0 then
					queued:set(nil)
					pendingS:set(nil)
					refusedS:set(state.error or _("Connection timed out. Please try again."))
				elseif state.connection == "connected" and state.name == nextAction.name then
					queued:set(nil)
					local refused = act(nextAction.fields)
					refusedS:set(refused)
					if nextAction.fields.action == "create" then
						generate:set(not refused and nextAction.fields.start_save == "" and onNewGame and { error = state.error } or nil)
					end
					if not refused then pendingS:set({ nextAction.doing, PENDING_POLLS, signature(state) }) end
				end
			end
			if generate:get() and state.room and state.room.you_own then
				generate:set(false)
				if onNewGame then onNewGame() end
			elseif generate:get() and state.error and state.error ~= generate:get().error then
				generate:set(false)
			end
			local room = state.room
			local owning = room and room.you_own and room.phase == "lobby"
			-- A start save picked on the room page goes once the game read its
			-- map and year, or could not in a few polls.
			local pick = pickS:old()
			if pick then
				local details = lobby.saveDetails(pick.save)
				if not details.async or pick.polls >= PICK_POLLS then
					pickS:set(nil)
					if owning then
						refusedS:set(act({ action = "choose_start", save = pick.save, map = details.map or "",
							year = details.year or 0 }))
					end
				else
					pickS:set({ save = pick.save, polls = pick.polls + 1 })
				end
			end
			-- The room names its start save without its map and year when the
			-- room was made private: this window tells it what the game read,
			-- once, so every player sees them.
			local start = owning and room.start
			if start and room.upload == nil and start.map == "" and (tonumber(start.year) or 0) == 0
				and describedRef:get() ~= start.name then
				local details = lobby.saveDetails(start.name)
				if not details.async then
					describedRef:set(start.name)
					if details.map ~= "" or details.year > 0 then
						act({ action = "choose_start", save = start.name, map = details.map, year = details.year })
					end
				end
						end
			-- The room list, while it is shown: asked for at once, then
			-- every LIST_POLLS polls (the server allows one a second).
			local browsing = state.linked and state.connection == "connected" and pageOf(state) == "join" and not modsS:old()
			if browsing then
				listAtRef:set(listAtRef:get() + 1)
				if state.rooms == nil and listAtRef:get() >= 3 or listAtRef:get() >= LIST_POLLS then
					listAtRef:set(0)
					act({ action = "list_rooms", page = state.rooms and state.rooms.page or 0 })
				end
			end
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
		if fields.action == "create" then
			generate:set(not refused and fields.start_save == "" and onNewGame and { error = stateS:old().error } or nil)
		end
		if not refused and doing then
			pendingS:set({ doing, PENDING_POLLS, signature(stateS:old()) })
		end
	end

	local busy = pendingS:old() ~= nil or queued:get() ~= nil
	local function connectedAction(fields, doing)
		if busy then return end
		local current = stateS:old()
		local typed = (name:get() or ""):match("^%s*(.-)%s*$")
		if typed == "" then typed = current.name or "" end
		if typed == "" then refusedS:set(_("Enter your name first.")); return end
		if current.connection == "connected" and current.name == typed then
			send(fields, doing)
		else
			queued:set({ fields = fields, doing = doing, name = typed, error = current.error, left = 75 })
			send({ action = "connect", name = typed }, _("Connecting..."))
			if refusedS:old() then queued:set(nil) end
		end
	end

	-- The frame every view shares: the title and the connection, the steps,
	-- a line for what went wrong, what is under way or what just happened,
	-- the room's world when it is coming, the view, and a footer with the
	-- view's buttons.
	local function frame(title, status, body, footer)
		local children = {
			row({
				icon(ICON.multiplayer, 28),
				gap(10),
				label(title, "font-scale-title-3"),
				gui_react_util.makeHorizontalSpacer(),
				status,
			}, style{ size = { WIDTH - 40, 36 } }),
			gap(10),
		}
		local problem = hideAddress(refusedS:old() or (state and state.error))
		if problem then
			children[#children + 1] = row({ icon(ICON.alert, 18), gap(6), label(problem, "font-scale-body, error") })
		elseif pendingS:old() then
			children[#children + 1] = row({ icon(ICON.loading, 18), gap(6), label(pendingS:old()[1], "font-scale-body, info") })
		elseif state and state.notice then
			children[#children + 1] = note(hideAddress(state.notice))
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
		children[#children + 1] = row(spaced(footer), style{ size = { WIDTH - 40, 40 } })
		return column(children, style{ size = { WIDTH, HEIGHT }, padding = { 16, 20, 16, 20 } })
	end

	-- No answer from the hook yet.
	if not state then
		local why = problemS:old()
		return frame(
			_("Multiplayer"),
			note(_("Waiting for the hook...")),
			label(why and (_("The hook did not answer: ") .. tostring(why)) or "", "font-scale-body, error"),
			{ gui_react_util.makeHorizontalSpacer(), button(_("Close"), onClose) }
		)
	end

	local canAct = state.linked and state.heard

	local connected = state.connection == "connected"
	local room = state.room
	local page = pageOf(state)
	local disconnect = function() send({ action = "disconnect" }, _("Disconnecting...")) end

	-- Where the player is, top right: online as whom, on which server.
	local status
	if connected then
		status = row({
			badge(_("Online"), "success"),
			gap(8),
			icon(ICON.player, 18),
			gap(4),
			label(tostring(state.name), "font-scale-body"),
			note("  @ " .. serverName(state)),
		})
	elseif state.connection == "connecting" then
		status = badge(_("Connecting"), "info")
	else
		status = badge(_("Not connected"), "warning")
	end

	local function back(to)
		return button(_("Back"), function()
			queued:set(nil)
			generate:set(false)
			joiningS:set(nil)
			pageS:set(to)
		end)
	end
	local function modsButton()
		local chosen = 0
		for _i, m in ipairs(state.mods or {}) do
			if m.chosen and m.choosable then chosen = chosen + 1 end
		end
		return button(string.format(_("Your mods (%d chosen)"), chosen), function() modsS:set(true) end, nil,
			#(state.mods or {}) > 0 or #(state.room_mods or {}) > 0)
	end

	-- Your mods: over whichever page opened it, with Back to it.
	if modsS:old() and page ~= "choose" then
		local playing = room and room.phase == "playing"
		local rows = {}
		for _i, m in ipairs(state.mods or {}) do
			local tone = (m.class == "shared" and "info") or (m.class == "carried" and "warning") or "success"
			local cells = {}
			if m.choosable then
				cells[#cells + 1] = button(m.chosen and _("On") or _("Off"), function()
					send({ action = "choose_mod", id = m.id, chosen = not m.chosen }, nil)
				end, m.chosen and "primary" or "secondary", canAct and not playing, m.reason)
			else
				cells[#cells + 1] = button(_("Needed"), function() end, "secondary", false, m.reason)
			end
			cells[#cells + 1] = gap(10)
			cells[#cells + 1] = label(m.name, m.choosable and "font-scale-body" or "font-scale-body, info")
			cells[#cells + 1] = gap(8)
			cells[#cells + 1] = badge(m.class == "shared" and _("every player needs it")
				or m.class == "carried" and _("carried by the room") or _("only you see it"), tone)
			rows[#rows + 1] = row(cells)
			rows[#rows + 1] = gap(6)
		end
		if #rows == 0 then rows[1] = note(_("No mods installed besides the room's.")) end
		local needs = {}
		for _i, m in ipairs(state.room_mods or {}) do
			local have = (m.have == "yes" and badge(_("You have it"), "success"))
				or (m.have == "other_version" and badge(_("Another version"), "warning"))
				or badge(_("You lack it"), "error")
			needs[#needs + 1] = row({ label(m.id, "font-scale-body"), gap(6),
				note(m.version ~= "" and ("v" .. m.version) or ""), gap(10), have })
			needs[#needs + 1] = gap(4)
		end
		if (state.room_mods_more or 0) > 0 then
			needs[#needs + 1] = note(string.format(_("and %d more"), state.room_mods_more))
		end
		local children = {
			row({ button(_("Back"), function() modsS:set(false) end), gap(16),
				heading(_("Your mods"), playing and _("The room's game has started: your choice holds for its next world.")
					or _("Turn on the mods only you play with; the room's own every player needs.")) }),
			builtin.ScrollArea{
				meta = { styleSheet = style{ size = { WIDTH - 40, #needs > 0 and 200 or HEIGHT - 220 } } },
				horizontalPolicy = builtin.type.ScrollBarPolicy.AlwaysOff,
				verticalPolicy = builtin.type.ScrollBarPolicy.AsNeeded,
				content = column(rows),
			},
		}
		if #needs > 0 then
			children[#children + 1] = gap(10)
			children[#children + 1] = heading(_("The room's mods"), _("Every player needs these, from the room's start save."))
			children[#children + 1] = builtin.ScrollArea{
				meta = { styleSheet = style{ size = { WIDTH - 40, 140 } } },
				horizontalPolicy = builtin.type.ScrollBarPolicy.AlwaysOff,
				verticalPolicy = builtin.type.ScrollBarPolicy.AsNeeded,
				content = column(needs),
			}
		end
		return frame(_("Your mods"), status, column(children),
			{ gui_react_util.makeHorizontalSpacer(), button(_("Close"), onClose) })
	end

	-- Your banner, from the first page: the picture the others see on your
	-- card in a room. A click picks one; Default goes back to the one your
	-- key gives.
	if bannerS:old() and page == "choose" then
		local rows, cellsRow = {}, {}
		for _i, banner in ipairs(BANNERS) do
			local picked = state.banner == banner[1]
			if #cellsRow > 0 then cellsRow[#cellsRow + 1] = gap(10) end
			cellsRow[#cellsRow + 1] = pictureCard(banner[2], picked and _("Yours") or " ", nil, nil, function()
				send({ action = "set_banner", banner = banner[1] }, nil)
			end, canAct, 196, 110)
			if #cellsRow >= 7 then
				rows[#rows + 1] = row(cellsRow)
				rows[#rows + 1] = gap(10)
				cellsRow = {}
			end
		end
		if #cellsRow > 0 then rows[#rows + 1] = row(cellsRow) end
		-- The campaign's characters this game has (the launcher takes their
		-- portraits from the game): one shows beside your name instead.
		local portraits = {}
		for _i, id in ipairs(state.portraits or {}) do
			if banners.portrait(id) then portraits[#portraits + 1] = id end
		end
		if #portraits > 0 then
			rows[#rows + 1] = gap(16)
			rows[#rows + 1] = heading(_("Characters"), _("A character of the campaign, beside your name."))
			rows[#rows + 1] = gap(10)
			cellsRow = {}
			for _i, id in ipairs(portraits) do
				if #cellsRow > 0 then cellsRow[#cellsRow + 1] = gap(10) end
				cellsRow[#cellsRow + 1] = lobby.portraitCard(id, state.banner == id, function()
					send({ action = "set_banner", banner = id }, nil)
				end, canAct)
				if #cellsRow >= 9 then
					rows[#rows + 1] = row(cellsRow)
					rows[#rows + 1] = gap(10)
					cellsRow = {}
				end
			end
			if #cellsRow > 0 then rows[#rows + 1] = row(cellsRow) end
		end
		return frame(_("Your banner"), status, column({
			row({
				button(_("Back"), function() bannerS:set(false) end),
				gap(16),
				heading(_("Your banner"), _("The picture the others see on your card in a room.")),
				gui_react_util.makeHorizontalSpacer(),
				button(_("Default"), function() send({ action = "set_banner", banner = "" }, nil) end, nil,
					canAct and state.banner ~= nil and state.banner ~= ""),
			}),
			builtin.ScrollArea{
				meta = { styleSheet = style{ size = { WIDTH - 40, HEIGHT - 220 } } },
				horizontalPolicy = builtin.type.ScrollBarPolicy.AlwaysOff,
				verticalPolicy = builtin.type.ScrollBarPolicy.AsNeeded,
				content = column(rows),
			},
		}), { gui_react_util.makeHorizontalSpacer(), button(_("Close"), onClose) })
	end

	-- The server this launcher plays on, from the first page: shown,
	-- changed, or put back to the launcher's own. Not while in a room.
	if serverS:old() and page == "choose" then
		local onDefault = state.server_address == state.server_default
		if serverText:get() == nil then serverText:set(state.server_address or "") end
		local usable = canAct and not room and not busy
		local function use(address)
			local refused = act({ action = "set_server", server = address })
			serverErrorS:set(refused)
			refusedS:set(nil)
			if not refused then
				serverText:set(address ~= "" and address or nil)
				pendingS:set({ _("Changing the server..."), PENDING_POLLS, signature(stateS:old()) })
			end
		end
		-- The launcher's own errors show at the top, as on every page.
		local problem = serverErrorS:old()
		local children = {
			row({
				button(_("Back"), function()
					serverS:set(false)
					serverErrorS:set(nil)
					serverText:set(nil)
				end),
				gap(16),
				heading(_("Server"), string.format(_("Now: %s%s"), serverName(state),
					onDefault and _(" (default)") or "")),
			}),
			field(_("Server address (host:port)"), serverText, state.server_default ~= "" and state.server_default or "host:port",
				{ maxLength = 128, onEnter = function(value) if usable then use(value) end end }),
			problem and label(problem, "font-scale-body, error") or gap(1),
			gap(8),
		}
		local buttons = {
			primary(_("Use this server"), function() use(serverText:get() or "") end, usable),
		}
		if not onDefault then
			buttons[#buttons + 1] = gap(8)
			buttons[#buttons + 1] = button(_("Reset to default"), function() use("") end, nil, usable)
		end
		children[#children + 1] = row(buttons)
		children[#children + 1] = gap(12)
		children[#children + 1] = note(_("Changing the server disconnects you and connects to the new one. Invites only join rooms on your own server."))
		return frame(_("Server"), status, column(children, style{ size = { LEFT + 100, AUTO } }),
			{ gui_react_util.makeHorizontalSpacer(), button(_("Close"), onClose) })
	end

	-- A friend's invite is a complete journey, including first connection.
	if page == "friend" then
		local function joinFriend()
			local code = (invite:get() or ""):gsub("%s", ""):upper()
			if not code:match("^[A-Z0-9][A-Z0-9][A-Z0-9][A-Z0-9][A-Z0-9][A-Z0-9]$") then
				refusedS:set(_("Enter the six-character invite code your friend sent you.")); return
			end
			connectedAction({ action = "join", invite = code, password = joinPassword:get() or "" }, _("Joining the room..."))
		end
		return frame(_("Join a friend"), status, row({
			column({
				heading(_("Your next journey, together"), _("Three details. One shared world.")), gap(16),
				field(_("Your name"), name, state.name ~= "" and state.name or _("Your name"), { maxLength = 32 }),
				field(_("Invite code"), invite, "K7QM2X", { maxLength = 16 }),
				field(_("Password (optional)"), joinPassword, "", { password = true, maxLength = 64 }),
				gap(12), primary(_("Join room"), joinFriend, canAct and not busy),
			}, style{ size = { LEFT, 390 } }), gap(30),
			column({ icon(ICON.multiplayer, 64), gap(20), heading(_("Meet in your friend's world")),
				note(_("Ask your friend for the code shown in their lobby.")), gap(10),
				note(_("We connect you and load the room's world together.")),
			}, style{ size = { RIGHT, AUTO } }),
		}), { back("choose"), gui_react_util.makeHorizontalSpacer(), button(_("Close"), onClose) })
	end

	-- The first page: identity, then a clear choice of hosting or discovery.
	if page == "choose" then
		local connecting = state.connection == "connecting"
		local function connect()
			local typed = name:get()
			if typed == nil or typed:match("^%s*$") then typed = state.name end
			send({ action = "connect", name = typed }, _("Connecting to ") .. serverName(state) .. "...")
		end
		local top
		if connected then
			top = note(string.format(_("Online on %s. Join a room someone hosts, or host your own."), serverName(state)))
		else
			top = row({
				label(_("Your name"), "font-scale-body"),
				gap(8),
				input(name, state.name ~= "" and state.name or _("Your name"), 260,
					{ maxLength = 32, acceptOnFocusLoss = true }),
				gap(10),
				primary(connecting and _("Connecting...") or string.format(_("Connect to %s"), serverName(state)),
					connect, canAct and not connecting and not busy),
			}, style{ size = { WIDTH - 40, 42 } })
		end
		return frame(
			_("Build something together"),
			status,
			column({
				top,
				gap(24),
				row({
					lobby.choiceCard(_("Join a room"),
						_("Browse the public rooms, or join a friend's with its invite"),
						"::/gui/menu/images/m05_ingame.tga", function() pageS:set("join") end, canAct and not busy),
					gap(24),
					lobby.choiceCard(_("Host a room"),
						_("Create a new world or continue a saved journey"),
						"::/gui/menu/images/m02_ingame.tga", function() pageS:set("host") end, canAct and not busy),
				}, style{ size = { WIDTH - 40, CHOICE_HEIGHT } }),
			}, style{ size = { WIDTH - 40, CHOICE_HEIGHT + 70 } }),
			{
				connected and button(_("Disconnect"), disconnect, nil, canAct) or gap(1),
				gap(8),
				button(_("Server..."), function() serverS:set(true) end, nil, canAct),
				gap(8),
				button(_("Your banner"), function() bannerS:set(true) end, nil, canAct),
				gui_react_util.makeHorizontalSpacer(),
				button(_("Close"), onClose),
			}
		)
	end

	local function joinBy(code, password)
		code = (code or ""):gsub("%s", ""):upper()
		if code == "" then
			refusedS:set(_("Type the invite code a friend sent you."))
			return
		end
		joiningS:set(nil)
		send({ action = "join", invite = code, password = password or "" }, _("Joining the room..."))
	end

	-- Join: the public rooms, as cards, and an invite.
	if page == "join" then
		if not connected then
			return frame(_("Discover public rooms"), status, column({
				heading(_("Find your next shared journey"), _("Connect to see rooms you can join.")), gap(16),
				field(_("Your name"), name, state.name ~= "" and state.name or _("Your name"), { maxLength = 32 }),
				primary(_("Browse rooms"), function() connectedAction({ action = "list_rooms", page = 0 }, _("Finding rooms...")) end, canAct and not busy),
			}, style{ size = { LEFT, 220 } }), { back("choose"), gui_react_util.makeHorizontalSpacer(), button(_("Close"), onClose) })
		end
		local list = state.rooms
		local found = list and list.list or {}
		local at = list and list.page or 0
		local function askPage(n)
			listAtRef:set(0)
			send({ action = "list_rooms", page = n }, nil)
		end
		local shown = {}
		for _i, listed in ipairs(found) do
			shown[#shown + 1] = lobby.roomCard(listed, function()
				if listed.has_password then
					joiningS:set({ invite = listed.invite, name = listed.name })
				else
					joinBy(listed.invite, "")
				end
			end, canAct and not busy)
		end
		local rows = {}
		for first = 1, #shown, CARDS_PER_ROW do
			local cells = {}
			for i = first, math.min(first + CARDS_PER_ROW - 1, #shown) do
				if i > first then cells[#cells + 1] = gap(12) end
				cells[#cells + 1] = shown[i]
			end
			rows[#rows + 1] = row(cells)
			rows[#rows + 1] = gap(12)
		end
		if #rows == 0 then
			rows[1] = note(list and _("No public rooms right now. Host one, and make it public.")
				or _("Asking the server for its rooms..."))
		end
		local children = {
			row({
				back("choose"),
				gap(16),
				heading(string.format(_("Public rooms on %s"), serverName(state)),
					_("Click a room to join it.")),
				gui_react_util.makeHorizontalSpacer(),
				button(_("Previous"), function() askPage(at - 1) end, nil, canAct and at > 0),
				gap(6),
				button(_("Next"), function() askPage(at + 1) end, nil, canAct and list ~= nil and list.more),
				gap(6),
				button(_("Refresh"), function() askPage(at) end, nil, canAct),
				gap(6),
				button(_("Join with code"), function()
					joiningS:set(nil)
					codeS:set(true)
				end, nil, canAct),
			}),
			builtin.ScrollArea{
				meta = { styleSheet = style{ size = { WIDTH - 40, HEIGHT - 290 } } },
				horizontalPolicy = builtin.type.ScrollBarPolicy.AlwaysOff,
				verticalPolicy = builtin.type.ScrollBarPolicy.AsNeeded,
				content = column(rows),
			},
			gap(10),
		}
		local joining = joiningS:old()
		if joining then
			children[#children + 1] = row({
				label(string.format(_("%s has a password:"), joining.name), "font-scale-body"),
				gap(8),
				input(joinPassword, _("Password"), 220, {
					password = true, maxLength = 64,
					onEnter = function(value) joinBy(joining.invite, value) end,
				}),
				gap(8),
				primary(_("Join"), function() joinBy(joining.invite, joinPassword:get()) end, canAct and not busy),
				gap(6),
				button(_("Cancel"), function() joiningS:set(nil) end),
			})
		end
		local body = column(children)
		-- Join with code: a popup over the room list, with the invite, a
		-- password and Join or Cancel.
		if codeS:old() then
			body = column({
				row({
					heading(_("Join with code"), _("A friend's room: the invite code they sent you, and its password if it has one.")),
				}),
				field(_("Invite code"), invite, "K7QM2X", { maxLength = 128 }),
				field(_("Password (if the room has one)"), joinPassword, "", { password = true, maxLength = 64 }),
				row({
					primary(_("Join"), function()
						codeS:set(false)
						joinBy(invite:get(), joinPassword:get())
					end, canAct and not busy),
					gap(8),
					button(_("Cancel"), function() codeS:set(false) end),
				}),
			}, style{ size = { LEFT, AUTO }, padding = { 16, 16, 16, 16 } })
		end
		return frame(_("Join a room"), status, body, {
			modsButton(),
			gui_react_util.makeHorizontalSpacer(),
			button(_("Close"), onClose),
		})
	end

	-- Host: the room's settings, and Create.
	if page == "host" then
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
		local saveItems = { { "", _("Create a new world...") } }
		for _i, save in ipairs(saves) do
			if not save:match("^tpf3mp_room_%d+$") then saveItems[#saveItems + 1] = { save, save } end
		end
		local pickedSave = saveS:old()
		if pickedSave == nil then
			pickedSave = ""
			for _i, save in ipairs(saves) do
				if save == state.start_save then pickedSave = save end
			end
		end
		local playersItems = {}
		for n = MIN_PLAYERS, MAX_PLAYERS do
			playersItems[#playersItems + 1] = { n, string.format(_("%d players"), n) }
		end
		local public = publicS:old() == "public"
		local details = pickedSave ~= "" and lobby.saveDetails(pickedSave) or nil
		local function create()
			local named = roomName:get()
			if named == nil or named:match("^%s*$") then
				local hostName = (name:get() or ""):match("^%s*(.-)%s*$")
				named = string.format(_("%s's room"), hostName ~= "" and hostName or state.name)
			end
			local fields = {
				action = "create",
				room = named,
				password = createPassword:get() or "",
				max_players = playersS:old(),
				rules = pickedRules or "",
				start_save = pickedSave,
				public = public,
				competitive = competitiveS:old() == true,
			}
			if public then
				fields.map = details and details.map or ""
				fields.year = details and details.year or 0
			end
			connectedAction(fields, _("Creating the room..."))
		end
		local where = public
			and (details and details.map ~= ""
				and string.format(_("Listed for everyone on %s: %s, %s."), serverName(state),
					lobby.climateName(details.map), details.year > 0 and tostring(details.year) or _("year unknown"))
				or string.format(_("Listed for everyone on %s."), serverName(state)))
			or _("Only players you send the invite to can find it.")
		return frame(_("Host a room"), status, column({
			row({ back("choose"), gap(16),
					heading(_("Make it your own"), _("Choose your world, play style and who can join.")) }),
			row({
				column({
					not connected and field(_("Your name"), name, state.name ~= "" and state.name or _("Your name"), { maxLength = 32 }) or gap(1),
					field(_("Room name"), roomName, string.format(_("%s's room"), state.name), { maxLength = 48 }),
					choice(_("Start from this save"), pickedSave, saveItems, function(value) saveS:set(value) end,
						pickedSave ~= "" and _("Every player's game loads it from the menu when you start.")
							or _("Choose your map and settings on the next screen.")),
					choice(_("Players"), playersS:old(), playersItems, function(value) playersS:set(value) end),
				}, style{ size = { LEFT, AUTO } }),
				gap(30),
				column({
					note(_("How you play")),
					gap(4),
					row({
						lobby.styleCard(false, competitiveS:old() ~= true, function() competitiveS:set(false) end, canAct),
						gap(12),
						lobby.styleCard(true, competitiveS:old() == true, function() competitiveS:set(true) end, canAct),
					}),
					gap(4),
					note(competitiveS:old() and _("Each player founds a company of their own in the game.")
						or _("Everyone plays for the room's one company.")),
					gap(10),
					choice(_("Who can find it"), public and "public" or "private", {
						{ "private", _("Private: invite only") },
						{ "public", _("Public: in the room list") },
					}, function(value) publicS:set(value) end, where),
					#rulesItems > 1 and choice(_("Rules"), pickedRules or rulesItems[1][1], rulesItems,
						function(value) rulesS:set(value) end, explainRules) or gap(1),
					field(_("Password (optional)"), createPassword, "", { password = true, maxLength = 64 }),
				}, style{ size = { RIGHT, AUTO } }),
			}),
		}), {
			modsButton(),
			gui_react_util.makeHorizontalSpacer(),
			button(_("Close"), onClose),
			primary(_("Create room"), create, canAct and not busy),
		})
	end

	-- In a room: players on the left, chat on the right.
	local me = you(room)
	local playing = room.phase == "playing"
	local confirm = confirmS:old()
	-- The players as cards of their banners, two to a row; for the owner,
	-- a Remove under each other player's, asked first.
	local memberRows = {}
	local cells = {}
	local function flush()
		if #cells > 0 then
			memberRows[#memberRows + 1] = row(cells)
			memberRows[#memberRows + 1] = gap(10)
			cells = {}
		end
	end
	for _i, member in ipairs(room.members) do
		local parts = { lobby.memberCard(member, playing) }
		if room.you_own and not member.you then
			parts[#parts + 1] = gap(4)
			if confirm and confirm.kind == "kick" and confirm.id == member.id then
				parts[#parts + 1] = row({
					button(_("Remove"), function()
						send({ action = "kick", player = member.id }, string.format(_("Removing %s..."), member.name))
					end, "primary", canAct),
					gap(4),
					button(_("Keep"), function() confirmS:set(nil) end),
				})
			else
				parts[#parts + 1] = row({
					button_react_util.makeIconButton(nil, ICON.kick, function()
						confirmS:set({ kind = "kick", id = member.id, name = member.name })
					end, string.format(_("Remove %s from the room"), member.name)),
				})
			end
		end
		if #cells > 0 then cells[#cells + 1] = gap(12) end
		cells[#cells + 1] = column(parts)
		if #cells >= 3 then flush() end
	end
	flush()

	local roomHeader = column({
		row({
			label(room.name, "font-scale-title-3"),
			gap(8),
			room.has_password and icon(ICON.lock, 18) or gap(1),
			gap(8),
			badge(room.competitive and _("Competitive") or _("Co-op"), room.competitive and "warning" or "success"),
			gui_react_util.makeHorizontalSpacer(),
		}),
		gap(6),
		row({
			note(_("Invite code  ")),
			label(room.invite ~= "" and inviteCode(room.invite) or "-", "font-scale-title-4, info"),
			gap(10),
			-- The hook puts it on the clipboard (the game's GUI has no
			-- clipboard of its own); "Copied" for a moment after.
			button(copiedS:old() > 0 and _("Copied") or _("Copy"), function()
				local refused = act({ action = "copy", text = inviteCode(room.invite) })
				refusedS:set(refused)
				if not refused then copiedS:set(COPIED_POLLS) end
			end, nil, room.invite ~= "", _("Copy the invite code, to paste it to your friends")),
		}),
		note(_("Send it to friends: they join with it from their game's Multiplayer window.")),
		gap(10),
		note(string.format(_("%d of %d players  ·  %d ready"), #room.members, room.max_players, readyCount(room))),
		gap(8),
	})

	local players = column({
		roomHeader,
		heading(_("Players")),
		builtin.ScrollArea{
			meta = { styleSheet = style{ size = { LEFT, HEIGHT - 330 } } },
			horizontalPolicy = builtin.type.ScrollBarPolicy.AlwaysOff,
			verticalPolicy = builtin.type.ScrollBarPolicy.AsNeeded,
			content = column(memberRows),
		},
	}, style{ size = { LEFT, AUTO } })

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
	-- The save the room starts from, for everyone; the owner picks it here
	-- while the room is in its lobby, from the saves the Host page offers.
	local startBlock
	-- The chat takes what the start save leaves of the column.
	local chatHeight = HEIGHT - 330
	if playing then
		startBlock = gap(1)
	else
		local start, upload, pick = room.start, room.upload, pickS:old()
		local line
		if pick then
			line = string.format(_("Reading %s..."), pick.save)
		elseif upload then
			line = string.format(_("Sending %s to the room: %d%%"), upload.save, upload.percent)
		elseif start then
			line = lobby.startLine(start)
		elseif room.you_own then
			line = _("Choose Set up world to create the map and settings for everyone.")
		else
			line = _("The world the owner's game has.")
		end
		local children = {}
		if room.you_own then
			local current = (pick and pick.save) or (upload and upload.save) or (start and start.name) or ""
			local items, listed = {}, false
			for _i, save in ipairs(state.saves or {}) do
				items[#items + 1] = { save, save }
				if save == current then listed = true end
			end
			-- The room's own, even once it left the newest saves listed.
			if current ~= "" and not listed then table.insert(items, 1, { current, current }) end
			items[#items + 1] = { "", _("New world: choose map and settings") }
			children[#children + 1] = choice(_("Start from this save"), current, items, function(value)
				if value == current or not canAct then return end
				confirmS:set(nil)
				if value == "" then
					send({ action = "choose_start", save = "" }, _("Changing the save..."))
					return
				end
				local details = lobby.saveDetails(value)
				if details.async then
					pickS:set({ save = value, polls = 0 })
				else
					send({ action = "choose_start", save = value, map = details.map, year = details.year },
						_("Changing the save..."))
				end
			end, line)
			chatHeight = chatHeight - 92
		else
			children[#children + 1] = note(_("Starts from"))
			children[#children + 1] = gap(4)
			children[#children + 1] = label(line, "font-scale-body")
			children[#children + 1] = gap(12)
			chatHeight = chatHeight - 54
		end
		if upload then
			children[#children + 1] = builtin.Component{
				meta = { styleSheet = style{ size = { RIGHT - 20, 14 } } },
				layout = builtin.BoxLayout{ children = { builtin.ProgressBar{ value = math.min(1, upload.percent / 100) } } },
			}
			children[#children + 1] = gap(8)
			chatHeight = chatHeight - 22
		end
		startBlock = column(children)
	end

	local chat = column({
		startBlock,
		heading(_("Chat")),
		builtin.ScrollArea{
			meta = { styleSheet = style{ size = { RIGHT, chatHeight } } },
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
		}, style{ size = { RIGHT, 40 } }),
	}, style{ size = { RIGHT, AUTO } })

	local footer = { modsButton() }
	if confirm and confirm.kind == "leave" then
		footer[#footer + 1] = label(_("Leave the room?"), "font-scale-body, warning")
		footer[#footer + 1] = button(_("Leave"), function()
			pageS:set("choose")
			send({ action = "leave" }, _("Leaving the room..."))
		end,
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
		elseif room.you_own and not state.start_save and onNewGame then
			footer[#footer + 1] = primary(_("Set up world"), onNewGame, canAct and not busy,
				_("Choose the map and settings, then start multiplayer"))
		else
			footer[#footer + 1] = primary(_("Ready"), function()
				send({ action = "ready", ready = true }, _("Getting ready..."))
			end, canAct and not busy)
		end
		if room.you_own then
			local all = everyoneReady(room)
			local waits = lobby.startWaits(room) or pickS:old() ~= nil
			footer[#footer + 1] = primary(_("Start the game"), function()
				send({ action = "start" }, _("Starting the room's game..."))
			end, canAct and all and not waits and not busy,
				(waits and _("The save is still on its way to the room"))
					or (all and _("Every player's game loads the room's world")) or _("Waiting for everyone to be ready"))
		end
	end

	return frame(_("Your room"), status, row({ players, gap(30), chat }), footer)
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
	-- A recipe placed among a layout's children must return a layout: the
	-- game refused a bare TextView here ("Recipe child must be a layout",
	-- ReactFramework::Load, 2026-09-30), as its own recipes return one.
	return builtin.BoxLayout{
		children = {
			builtin.TextView{
				meta = { class = "font-scale-annotation, annotation" },
				text = lineS:old(),
			},
		},
	}
end)

-- What the "Join a friend" card says: the room once in one.
function lobby.joinLine(state)
	local room = state and state.room
	if room and room.invite ~= "" then
		return string.format(_("Your room: invite %s"), inviteCode(room.invite))
	end
	return _("With the invite code they send you")
end

return lobby

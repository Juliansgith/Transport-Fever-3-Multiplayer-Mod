-- A stand-in for Transport Fever 3's main menu, as much of it as the mod's
-- Multiplayer window (mod/tpf3mp_1/content/gui/menu/lobby.lua) uses:
-- ug_require with react, builtin, gui_react_util and button_react_util,
-- api.gui.StyleSheet, the loader's resolveutil.loadfile the hook answers,
-- _ and debugPrint. Run by the tests in crates/tpf3mp-hook/src/lobby.rs,
-- which define LOBBY_SOURCE (the window's file) and set STATE (the hook's
-- answer to a state request, a Lua table literal).
--
-- Defines LOG (debugPrint lines), SENT (the JSON of every action the
-- window sent), REPLY (what the hook answers an action, "ok" unless set),
-- and render(focus), tick(), texts(), find(text), click(text),
-- choose(caption, value), type_into(placeholder, text) and enabled(text).

LOG = {}
SENT = {}
REPLY = "ok"
STATE = nil

function debugPrint(line) LOG[#LOG + 1] = tostring(line) end
function _(text) return text end

api = {
	gui = { StyleSheet = { new = function() return {} end } },
	type = {
		Vec2f = { new = function(x, y) return { x = x, y = y } end },
		Vec4f = { new = function(a, b, c, d) return { a, b, c, d } end },
	},
}

-- The hook's answers, as crates/tpf3mp-hook/src/menu_entry.rs gives them.
resolveutil = {}
function resolveutil.loadfile(path)
	if path == "tpf3mp_1::/tpf3mp/state.lua" then
		return STATE
	elseif path == "tpf3mp_1::/tpf3mp/act.lua" then
		SENT[#SENT + 1] = resolveutil.__tpf3mp_action
		return REPLY
	end
	error("no such file: " .. tostring(path))
end

local mount = { refs = {}, index = 0, timers = {} }

local react = {}
function react.useRef(initial)
	mount.index = mount.index + 1
	local ref = mount.refs[mount.index]
	if not ref then
		ref = { value = initial }
		function ref:get() return self.value end
		function ref:set(v) self.value = v end
		function ref:old() return self.value end
		mount.refs[mount.index] = ref
	end
	return ref
end
react.useState = react.useRef
function react.onStepTimer(fn) mount.timers[#mount.timers + 1] = fn end
function react.RegisterRecipe(name, fn)
	return setmetatable({ name = name }, { __call = function(_, params) return fn(params) end })
end

local builtin = { type = {
	Orientation = { Horizontal = "Horizontal", Vertical = "Vertical" },
	ScrollBarPolicy = { AlwaysOff = "AlwaysOff", AsNeeded = "AsNeeded" },
	ImageViewScaling = { AutoFit = "AutoFit" },
} }
for _i, view in ipairs({ "BoxLayout", "Component", "TextView", "Button", "ImageView", "TextInputField",
		"ScrollArea", "ComboBox", "ComboBoxItem", "ProgressBar" }) do
	builtin[view] = function(params) return { view = view, params = params } end
end

local gui_react_util = {
	makeHorizontalSpacer = function() return { view = "Spacer" } end,
	makeVerticalSpacer = function() return { view = "Spacer" } end,
}
local button_react_util = {
	makeIconButton = function(_ref, path, onClick, tooltip)
		return { view = "Button", params = { icon = path, onClick = onClick, meta = { tooltip = tooltip } } }
	end,
}

local modules = {
	["::/gui/main/react.lua"] = react,
	["::/gui/main/builtin.lua"] = builtin,
	["::/gui/main/gui_react_util.tl"] = gui_react_util,
	["::/gui/main/button_react_util.tl"] = button_react_util,
}
function ug_require(path)
	return assert(modules[path], "no module " .. path)
end

local lobby = assert(loadstring(LOBBY_SOURCE, "@lobby.lua"))()
LOBBY = lobby
CLOSED = 0
local tree
local focus

function render(f)
	if f ~= nil then focus = f end
	mount.index = 0
	mount.timers = {}
	tree = lobby.content(function() CLOSED = CLOSED + 1 end, focus)
	return tree
end

-- One poll of the window's timer, then a redraw, as the game does.
function tick()
	for _i, fn in ipairs(mount.timers) do fn() end
	return render()
end

local function walk(node, visit)
	if type(node) ~= "table" then return end
	visit(node)
	for _k, value in pairs(node) do
		if type(value) == "table" then walk(value, visit) end
	end
end

-- Every text shown, in one string, one per line.
function texts()
	local out = {}
	walk(tree, function(node)
		if node.view == "TextView" then out[#out + 1] = node.params.text end
		if node.view == "ComboBoxItem" then end
	end)
	return table.concat(out, "\n")
end

-- The button showing `text` (or with that tooltip), nil if none.
function find(text)
	local found
	walk(tree, function(node)
		if node.view == "Button" and not found then
			local content = node.params.content
			local shown = content and content.params and content.params.text
			if shown == text or (node.params.meta and node.params.meta.tooltip == text) then
				found = node.params
			end
		end
	end)
	return found
end

function enabled(text)
	local button = assert(find(text), "no button " .. text)
	return button.meta == nil or button.meta.enabled ~= false
end

function click(text)
	local button = assert(find(text), "no button " .. text)
	assert(button.meta == nil or button.meta.enabled ~= false, "button disabled: " .. text)
	button.onClick()
	return render()
end

-- Types into the field showing `placeholder`, and presses Enter.
function type_into(placeholder, text)
	local done = false
	walk(tree, function(node)
		if node.view == "TextInputField" and node.params.placeholderText == placeholder and not done then
			node.params.onTyping(text)
			node.params.onValueChange(text)
			done = true
		end
	end)
	assert(done, "no field " .. placeholder)
	return render()
end

-- Picks `value` in the combo box under the caption `caption`.
function choose(caption, value)
	local done = false
	walk(tree, function(node)
		if node.view == "Component" and not done then
			local children = node.params.layout and node.params.layout.params.children
			local first = children and children[1]
			if first and first.view == "TextView" and first.params.text == caption then
				walk(node, function(inner)
					if inner.view == "ComboBox" and not done then
						inner.params.onValueChange(value)
						done = true
					end
				end)
			end
		end
	end)
	assert(done, "no choice " .. caption)
	return render()
end

-- The values a choice under `caption` offers, and the one chosen.
function offered(caption)
	local values, chosen = {}, nil
	walk(tree, function(node)
		if node.view == "Component" and chosen == nil then
			local children = node.params.layout and node.params.layout.params.children
			local first = children and children[1]
			if first and first.view == "TextView" and first.params.text == caption then
				walk(node, function(inner)
					if inner.view == "ComboBox" and chosen == nil then
						chosen = inner.params.value
						for _i, item in ipairs(inner.params.items) do values[#values + 1] = item.params.value end
					end
				end)
			end
		end
	end)
	return values, chosen
end

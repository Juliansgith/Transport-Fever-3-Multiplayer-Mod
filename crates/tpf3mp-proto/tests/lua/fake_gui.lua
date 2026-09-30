-- A stand-in for Transport Fever 3's GUI state, as much of it as the mod's
-- entry script (mod/tpf3mp_1/content/gui/tpf3mp/tpf3mp.script.lua) uses:
-- ug_require, react, builtin, the game bar's extension point and
-- debugPrint. Its shape follows mods made for build 40391
-- (investigation/TF3_MODS_2026-09-27.md). Run by tests/lua_mod.rs, which
-- defines mod_source(path): the text of a file under the mod's content/.
--
-- Defines the globals the game has, plus LOG (every debugPrint line) and
-- mount(recipe), which renders a plugin and runs its steps.

-- The game's package table has no preload (build 40408's dump,
-- investigation/dayone-2026-09-29/probe/script_api_dump_gui.txt).
package.preload = nil

LOG = {}
function debugPrint(line)
	LOG[#LOG + 1] = tostring(line)
end

local current   -- the mount a recipe is rendering in

local react = {}
function react.RegisterPluginRecipe(extension, name, fn)
	return { extension = extension, name = name, fn = fn }
end
function react.useRef(initial)
	-- The same ref on every render of one mount, as React keeps it.
	local m = current
	m.refIndex = m.refIndex + 1
	local ref = m.refs[m.refIndex]
	if not ref then
		ref = { value = initial }
		function ref:get() return self.value end
		function ref:set(v) self.value = v end
		m.refs[m.refIndex] = ref
	end
	return ref
end
function react.useState(initial)
	-- A ref whose value the next render reads, as the game's state does.
	local state = react.useRef(initial)
	function state:old() return self.value end
	return state
end
function react.onStep(fn)
	current.onStep = fn
end

local builtin = { type = {
	Orientation = { Horizontal = "Horizontal", Vertical = "Vertical" },
	ScrollBarPolicy = { Simple = "Simple", AlwaysOff = "AlwaysOff" },
} }
function builtin.BoxLayout(params)
	return { layout = "BoxLayout", params = params }
end
for _, view in ipairs({ "TextView", "Button", "ScrollArea", "Component", "TextInputField", "Window" }) do
	builtin[view] = function(params) return { view = view, params = params } end
end

local game_bar_widgets = { GameBarInfoDisplayExtension = "GameBarInfoDisplayExtension" }
local mod_entry_point = { ModEntryPointExtension = "ModEntryPointExtension" }

local GAME = {
	["::/gui/main/react.lua"] = react,
	["::/gui/main/builtin.lua"] = builtin,
	["::/gui/game_bar/game_bar_widgets.tl"] = game_bar_widgets,
	["::/gui/main/mod_entry_point.tl"] = mod_entry_point,
}

-- The mod whose files mod_source reads: ours, or the one MOD_ID names.
local MOD = (MOD_ID or "tpf3mp_1") .. "::/"
local loaded = {}
UG_REQUIRED = {}
function ug_require(path)
	UG_REQUIRED[#UG_REQUIRED + 1] = path
	if GAME[path] then return GAME[path] end
	if loaded[path] then return loaded[path] end
	assert(path:sub(1, #MOD) == MOD, "ug_require of an unknown path " .. path)
	local rel = path:sub(#MOD + 1)
	local chunk = assert(loadstring(mod_source(rel), "@" .. rel))
	local module = chunk()
	loaded[path] = module
	return module
end

-- Renders a plugin recipe once, then returns the mount: render() renders it
-- again, step() runs its onStep as the game does every frame.
function mount(recipe)
	local m = { refs = {}, refIndex = 0 }
	function m.render()
		current, m.refIndex = m, 0
		m.layout = recipe.fn()
		current = nil
		return m.layout
	end
	function m.step()
		if m.onStep then m.onStep() end
	end
	m.render()
	return m
end

-- Runs a mod's entry script and returns its plugin, checking the name the
-- resource file gives and its extension point: ours
-- (tpf3mp.script@Tpf3mpPlugin, on the game bar) unless named.
function loadPlugin(script, recipe, extension)
	script = script or "gui/tpf3mp/tpf3mp.script.lua"
	recipe = recipe or "Tpf3mpPlugin"
	extension = extension or "GameBarInfoDisplayExtension"
	local entry = assert(loadstring(mod_source(script), "@" .. script))
	entry()
	local exported = data()
	local plugin = assert(exported[recipe], "no " .. recipe)
	assert(plugin.extension == extension, "on another extension point")
	return plugin
end

-- The views a rendered layout holds, depth first, as { view =, params = }.
function views(node, out)
	out = out or {}
	if type(node) ~= "table" then return out end
	if node.view then out[#out + 1] = node end
	local params = node.params or {}
	for _, key in ipairs({ "content", "layout", "child" }) do views(params[key], out) end
	for _, child in ipairs(params.children or {}) do views(child, out) end
	return out
end

-- The lines the mod logged, one per line.
function logText()
	return table.concat(LOG, "\n")
end

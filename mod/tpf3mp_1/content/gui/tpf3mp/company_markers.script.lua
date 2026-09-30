-- Company colours on the vehicles' markers, the icons the game draws over
-- each vehicle on the map. Painting a vehicle (tpf3mp/companies.lua) colours
-- its body and its picture in the game's windows, not its marker: build 40408
-- draws every vehicle's marker alike, a white glyph on a dark tile styled
-- `VehicleItem::Icon` (HudIconManager.cpp, CreateTransportVehicleItem;
-- gui/main/internal_hud.css.lua), and a loaded vehicle's as its cargo's
-- tiles (`hud-icon-background`, gui/main/gui_react_util.tl).
--
-- gui/tpf3mp/company_markers.res.lua replaces the recipe that makes a
-- marker, the game's hud_icon_toolbox.HudIconMasterGame, with this one: it
-- calls the game's and, while the room has more than one company, puts a
-- vehicle's marker in the class of its company's colour, which
-- gui/tpf3mp/tpf3mp.css.lua colours. Everything else keeps the game's
-- marker. The companies are read from the mod's game script's state, as
-- the Multiplayer plugin reads them (tpf3mp/companies.lua).
function data()
	local react = ug_require "::/gui/main/react.lua"
	local builtin = ug_require "::/gui/main/builtin.lua"
	local engine_react_util = ug_require "::/gui/main/engine_react_util.tl"
	local hud_icon_toolbox = ug_require "::/gui/main/hud_icon_toolbox.tl"
	local companies = ug_require "tpf3mp_1::/scripts/tpf3mp/companies.lua"
	-- Seconds between two looks at a marker's company.
	local INTERVAL = 1.0

	local original = hud_icon_toolbox.HudIconMasterGame

	-- The game's log says once what became of the markers, each way it went
	-- ("[tpf3mp] company markers: ...").
	local told = {}
	local function tell(what)
		if told[what] then return end
		told[what] = true
		pcall(debugPrint, "[tpf3mp] company markers: " .. what)
	end

	-- The roster as the mod's game script keeps it, read at most every
	-- READ_EVERY seconds. The game renders markers on its worker threads,
	-- each with a Lua state of its own (build 40408: "Main Pool" in the
	-- log), where the Multiplayer plugin never runs, so each state reads the
	-- roster for itself.
	local READ_EVERY = 2.0
	local roster, readAt = nil, nil
	local function rosterNow()
		local ok, now = pcall(function() return os.clock() end)
		if not ok or readAt == nil or now - readAt >= READ_EVERY then
			readAt = ok and now or nil
			local state = companies.scriptState(api)
			roster = state and state.companies
		end
		return roster
	end

	-- The class of `entity`'s marker: its company's colour, for a vehicle
	-- while the room has more than one company; "" for the game's own.
	--
	-- The marker follows the vehicle's paint, as TPF2's vehicle icons did:
	-- the room paints a company's vehicles in its colour (tpf3mp/apply.lua,
	-- tpf3mp/companies.lua). A vehicle's owner cannot be read here (build
	-- 40408: PLAYER_OWNED is empty on the worker threads, "a vehicle no
	-- player owns" in the log), its parts' colour can, as the game's own
	-- vehicle pictures read it (gui/line_vehicle_mgmt/vehicle_react_util.tl).
	local function classOf(entity)
		local color
		pcall(function()
			local tv = api.engine.getComponent(entity, api.type.ComponentType.TRANSPORT_VEHICLE)
			local c = tv and tv.transportVehicleConfig.vehicles[1].part.color
			if c then color = { c.x, c.y, c.z } end
		end)
		if color == nil then return "" end
		local r = rosterNow()
		if type(r) ~= "table" or type(r.list) ~= "table" then
			tell("no companies in the game script's state here")
			return ""
		end
		if not companies.painting(r) then return "" end
		-- A colour of the companies' palette; any other is a player's own.
		local index = companies.swatch(color)
		if not index then return "" end
		tell("a vehicle's marker in its company's colour (" .. companies.markerClass(index) .. ")")
		return companies.markerClass(index)
	end

	-- CallOriginalRecipe gives a node of the game's recipe, and the HUD
	-- takes only a layout from a marker's recipe ("Recipe child must be a
	-- layout", build 40408), so the game's marker is always in a layout of
	-- its own, which has the company's class when there is one.
	local CompanyMarker = react.RegisterRecipe("Tpf3mpCompanyMarker", function(params, userParam)
		tell("the game's markers go through the mod's")
		local class = engine_react_util.useStepStateTimer(function()
			local ok, found = pcall(classOf, params and params.entity)
			if not ok then tell("failed: " .. tostring(found)) end
			return ok and found or ""
		end, INTERVAL)
		local marker = react.CallOriginalRecipe(original, params, userParam)
		if class:old() == "" then return builtin.BoxLayout{ children = { marker } } end
		return builtin.BoxLayout{ meta = { class = class:old() }, children = { marker } }
	end)

	return {
		replace = function(replacementApi)
			replacementApi.ReplaceRecipe(original, CompanyMarker)
			tell("the game's marker recipe is replaced")
		end,
	}
end

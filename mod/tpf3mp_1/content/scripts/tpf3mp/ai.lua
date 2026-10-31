-- tpf3mp/ai.lua -- the questions of the AI players this game drives
-- (docs/AI_PLAYERS.md, "What the driver's game does").
--
-- The room hands an AI seat to one member's game; that member's agent runs
-- the AI (crates/tpf3mp-ai) and asks this game about the world through the
-- hook (`aiAsk`, `aiAnswer`; tpf3mp_bridge::ai). The GUI answers here, a few
-- a frame within ai.BUDGET_MS:
--
--   "Company"                     the seat's company and what it owns
--   "Towns"                       the towns, largest first
--   { Streets = { at, radius } }  the street edges near a place
--   { Models = { mode } }         the vehicle models it may buy now
--   { Verdict = <action> }        the game's verdict on a build, as the
--                                 seat's company would build it, nothing
--                                 built (apply.proposalOf, makeProposalData)
--
-- Answers are tables in the game's units, positions in metres, as
-- tpf3mp_bridge::ai names their fields; every other number whole. Reads
-- only: nothing here sends a command or changes the world.
--
-- Pure Lua against the game's `api`; the tests give it a fake one.

local ai = {}

-- Milliseconds a frame the answers may take: the AI asks a few things a
-- second, and the game's frame keeps its pace.
ai.BUDGET_MS = 1.5
-- The most of each kind an answer lists (tpf3mp_bridge::ai's bounds).
ai.MAX_TOWNS = 160
ai.MAX_STREETS = 320
ai.MAX_MODELS = 48
ai.MAX_OWNED = 160
ai.MAX_MESSAGES = 8
ai.MAX_NEEDS = 4
ai.MAX_LAND = 24
ai.MAX_ENDS = 8

local function module(name)
	if ug_require then return ug_require("tpf3mp_1::/scripts/tpf3mp/" .. name .. ".lua") end
	return require("tpf3mp." .. name)
end

local function whole(x)
	if type(x) ~= "number" or x ~= x then return 0 end
	if x >= 0 then return math.floor(x + 0.5) end
	return -math.floor(-x + 0.5)
end

local function pos(p)
	return { x = p.x or p[1] or 0, y = p.y or p[2] or 0, z = p.z or p[3] or 0 }
end

local function text(s, limit)
	s = tostring(s or "")
	if #s > limit then s = s:sub(1, limit) end
	return s
end

local function component(entity, name)
	local ok, c = pcall(api.engine.getComponent, entity, api.type.ComponentType[name])
	if ok then return c end
	return nil
end

local function each(list, limit)
	local out = {}
	for i = 1, math.min(#(list or {}), limit) do out[i] = list[i] end
	return out
end

-- The company `seat` plays for (tpf3mp/companies.lua) in `roster`.
local function companyOf(roster, seat)
	local companies = module("companies")
	if type(roster) ~= "table" then return nil end
	return companies.of(roster, seat)
end

-- The construction origin of each station and depot, read once an answer.
-- The construction an entity is part of (a station, a depot), by the
-- game's own index, as its entity and component; nil where none is. Never
-- a scan of every construction: 3,470 on the big fixture took 3 to 18 ms a
-- question (2026-10-29), a stutter in the driving game.
local function constructionOf(entity, lookups)
	local system = api.engine.system and api.engine.system.streetConnectorSystem
	for _, lookup in ipairs(lookups) do
		local ok, con = pcall(function() return system[lookup](entity) end)
		if ok and type(con) == "number" and con >= 0 then
			local c = component(con, "CONSTRUCTION")
			if c and c.transf then return con, c end
		end
	end
	return nil
end

local function origin(c)
	local t = c.transf
	return { x = t[13], y = t[14], z = t[15] }
end

-- The depot constructions `entity` owns a depot of, each once.
local function depotsOf(entity)
	local companies = module("companies")
	local out, seen = {}, {}
	local ok, kind = pcall(function() return api.type.ComponentType.VEHICLE_DEPOT end)
	if not ok or kind == nil then return out end
	for _, depot in ipairs(api.engine.getEntitiesWithComponent(kind) or {}) do
		if companies.ownerOf(api, depot) == entity then
			local con, c = constructionOf(depot, { "getConstructionEntityForDepot", "getConstructionEntityForSubconstruction" })
			if con and not seen[con] then
				seen[con] = true
				out[#out + 1] = { entity = con, construction = c, origin = origin(c) }
			end
		end
	end
	return out
end

-- Where a station stands that no construction holds: a street stop is an
-- object on its street's edge (EDGE_OBJECT, its transform), seen on build
-- 40425; else the middle of its bounding box.
local function objectAt(station)
	local eo = component(station, "EDGE_OBJECT")
	if eo and eo.transf then return { x = eo.transf[13], y = eo.transf[14], z = eo.transf[15] } end
	local bv = component(station, "BOUNDING_VOLUME")
	if bv and bv.bbox then
		local a, b = bv.bbox.min, bv.bbox.max
		return { x = (a.x + b.x) / 2, y = (a.y + b.y) / 2, z = (a.z + b.z) / 2 }
	end
	return nil
end

-- A depot construction's entrance, and whether it reaches the street
-- network. A road depot's frozen nodes hold only its apron's inner end; its
-- frozen entrance edge runs from there to the entrance (build 40425, read
-- from the console). Where the game's refresh snapped it, that end is a
-- junction of the street, where edges not its own meet it: the entrance,
-- connected. Else the end of its own edges farthest from its origin (a road
-- depot's apron ends 35 m before it), a dead end.
local function entrance(c, origin)
	local streets = api.engine.system and api.engine.system.streetSystem
	local own, nodes, seen = {}, {}, {}
	local function add(node)
		if type(node) == "number" and not seen[node] then
			seen[node] = true
			nodes[#nodes + 1] = node
		end
	end
	for _, node in ipairs(c.frozenNodes or {}) do add(node) end
	for _, e in ipairs(c.frozenEdges or {}) do
		own[e] = true
		local edge = component(e, "BASE_EDGE")
		if edge then add(edge.node0) add(edge.node1) end
	end
	local best, bestD = nil, -1
	for _, node in ipairs(nodes) do
		local n = component(node, "BASE_NODE")
		if n and n.position then
			local p = { x = n.position.x, y = n.position.y, z = n.position.z }
			local ok, segments = pcall(function() return streets.getNodeSegments(node) end)
			for _, segment in ipairs(ok and segments or {}) do
				if not own[segment] then return p, true end
			end
			local dx, dy = p.x - origin.x, p.y - origin.y
			local d = dx * dx + dy * dy
			if d > bestD then best, bestD = p, d end
		end
	end
	return best, false
end

-- A rail station construction's track ends: each node one of its own
-- track edges ends in that is none of its frozen nodes, where the world's
-- track meets the station, and whether no edge of the world's meets it yet
-- (read from the fixture's stations: a cargo station's two ends, at local
-- (10, 40 s - 20) and (10, 40 e + 20) of its track modules, 2026-10-30).
local function trackEnds(c)
	local streets = api.engine.system and api.engine.system.streetSystem
	local frozen, own, out, seen = {}, {}, {}, {}
	for _, node in ipairs(c.frozenNodes or {}) do frozen[node] = true end
	for _, e in ipairs(c.frozenEdges or {}) do own[e] = true end
	for _, e in ipairs(c.frozenEdges or {}) do
		local edge = component(e, "BASE_EDGE")
		if edge and component(e, "BASE_EDGE_STREET") == nil then
			for _, node in ipairs({ edge.node0, edge.node1 }) do
				if type(node) == "number" and not frozen[node] and not seen[node] and #out < ai.MAX_ENDS then
					seen[node] = true
					local n = component(node, "BASE_NODE")
					if n and n.position then
						local open = true
						local ok, segments = pcall(function() return streets.getNodeTrackSegments(node) end)
						for _, segment in ipairs(ok and segments or {}) do
							if not own[segment] then open = false end
						end
						out[#out + 1] = { pos = { x = n.position.x, y = n.position.y, z = n.position.z }, open = open }
					end
				end
			end
		end
	end
	return out
end

function ai.company(seat, roster, registry)
	local reg = type(registry) == "function" and registry() or registry
	local regmod = module("registry")
	local c = companyOf(roster, seat)
	local entity = c and c.entity
	local out = {
		money = 0,
		year = whole(api.engine.util.getYear and api.engine.util.getYear() or 0),
		stations = {}, depots = {}, lines = {}, vehicles = {}, offers = {}, loans = 0, owed = {}, month = 0,
	}
	pcall(function()
		local length = api.util.getDefaultMonthDuration()
		if type(length) == "number" and length > 0 and length < 4294967295 then out.month = whole(length) end
	end)
	if c and c.id ~= 0 then out.company = c.id end
	if entity == nil then return { Company = out } end
	-- Its loan offers, those not cooling down, and its loans
	-- (tpf3mp/companies.lua, "loans").
	if c.id ~= 0 then
		for _, group in ipairs(roster.loanOffers or {}) do
			if group.company == c.id then
				for _, offer in ipairs(group.availableLoans or {}) do
					if type(offer) == "table" and offer.cooldownUntil == nil and type(offer.type) == "string"
						and type(offer.amount) == "number" and #out.offers < 8 then
						out.offers[#out.offers + 1] = { type = offer.type, amount = whole(offer.amount),
							duration = whole(offer.duration or 0), percentage = offer.percentage or 0 }
					end
				end
			end
		end
		-- Each loan with what is still owed and paid a month, by its id and
		-- amount as paying it back names it (companies.repay).
		local okM, monthLength = pcall(function() return api.util.getDefaultMonthDuration() end)
		if not okM or type(monthLength) ~= "number" then monthLength = 0 end
		out.owed = {}
		for _, loan in ipairs(roster.loans or {}) do
			if loan.company == c.id and out.loans < 255 then
				out.loans = out.loans + 1
				if #out.owed < 8 and type(loan.amount) == "number" and type(loan.remaining) == "number" then
					local months, paid = loan.months or 0, loan.paid or 0
					out.owed[#out.owed + 1] = {
						terms = { type = loan.type or "Custom", amount = whole(loan.amount),
							duration = whole(months * monthLength), percentage = (loan.rate or 0) * 12,
							id = loan.id, timesPaid = paid },
						remaining = whole(loan.remaining), payment = whole(loan.payment or 0),
						months_left = math.max(0, whole(months - paid)),
					}
				end
			end
		end
	end
	out.owed = out.owed or {}
	local finance = api.engine.util.finance
	out.money = whole(finance and finance.getPlayersBalance and finance.getPlayersBalance(entity) or 0)
	local companies = module("companies")
	local depots = depotsOf(entity)
	-- Station groups: owned by the company, named by the room's ids.
	for _, g in ipairs(api.engine.getEntitiesWithComponent(api.type.ComponentType.STATION_GROUP) or {}) do
		if #out.stations >= ai.MAX_OWNED then break end
		if companies.ownerOf(api, g) == entity then
			local id = reg and regmod.id(reg, "groups", g)
			local group = component(g, "STATION_GROUP")
			local first = group and group.stations and group.stations[1]
			local holder
			if first then
				local _, c = constructionOf(first, { "getConstructionEntityForStation",
					"getConstructionEntityForSubconstruction" })
				holder = c
			end
			local at = first and (holder and origin(holder) or objectAt(first))
			if id and at then
				local terminals = 0
				for _, s in ipairs(group.stations or {}) do
					local station = component(s, "STATION")
					terminals = terminals + #((station and station.terminals) or {})
				end
				out.stations[#out.stations + 1] = { id = id, pos = pos(at), terminals = terminals, waiting = 0,
					ends = holder and trackEnds(holder) or {} }
			end
		end
	end
	for _, d in ipairs(depots) do
		if #out.depots >= ai.MAX_OWNED then break end
		local gate, connected = entrance(d.construction, d.origin)
		out.depots[#out.depots + 1] = {
			file = text(d.construction.fileName, 128), at = pos(d.origin),
			entrance = gate and pos(gate) or nil, connected = connected,
		}
	end
	local lines = api.engine.system.lineSystem.getLinesForPlayer(entity) or {}
	local vehicles = api.engine.system.transportVehicleSystem
	-- The game's time and month, for a line's last twelve months.
	local now, month = 0, 0
	pcall(function()
		now = component(api.engine.util.getWorld(), "GAME_TIME").gameTime
		month = api.util.getDefaultMonthDuration()
	end)
	for _, l in ipairs(lines) do
		if #out.lines >= ai.MAX_OWNED then break end
		local id = reg and regmod.id(reg, "lines", l)
		local line = component(l, "LINE")
		if id and line then
			local stops, seen = {}, {}
			for _, s in ipairs(line.stops or {}) do
				local sid = regmod.id(reg, "groups", s.stationGroup)
				if sid and #stops < 32 then stops[#stops + 1] = sid end
				seen[s.stationGroup] = true
			end
			local fleet = {}
			for _, v in ipairs(vehicles.getLineVehicles(l) or {}) do
				local vid = regmod.id(reg, "vehicles", v)
				if vid and #fleet < 64 then fleet[#fleet + 1] = vid end
			end
			local problems = 0
			pcall(function() problems = #(api.engine.util.line.getLineIssues(l, false) or {}) end)
			-- What it earned less what it cost over the last twelve months,
			-- as the game's line window reads it.
			local profit = 0
			pcall(function()
				profit = api.engine.util.finance.calculateBalance({ l }, math.max(0, now - 12 * month), now, true)
			end)
			-- What it earned over them, its costs not taken off: the journal's
			-- income, kind 2 (its profit is its vehicles' maintenance, kind 0,
			-- and this together, read from the console 2026-10-31); nil where
			-- the game does not say.
			local income
			pcall(function()
				local v = api.engine.util.finance.calculateBalance({ l }, math.max(0, now - 12 * month), now, true, 2)
				if type(v) == "number" then income = whole(v) end
			end)
			-- What it made over its last three months: how it does now.
			local recent
			pcall(function()
				local v = api.engine.util.finance.calculateBalance({ l }, math.max(0, now - 3 * month), now, true)
				if type(v) == "number" then recent = whole(v) end
			end)
			-- How full it runs: the capacity its sections use, as the line
			-- window's capacity chart (1 at least for a line with capacity,
			-- 0 where unknown).
			local load = 0
			pcall(function()
				local usages = api.engine.util.line.getLineCapacityUsages(l, false)
				local capacity, used = 0, 0
				for i = 1, #usages do
					capacity = capacity + usages[i].capacity
					used = used + usages[i].used
				end
				if capacity > 0 then load = math.max(1, math.min(100, whole(used * 100 / capacity))) end
			end)
			-- What waits at its stops of the kinds its vehicles carry: a
			-- vehicle's capacities list each cargo, entry i the cargo i - 1.
			local waiting = 0
			pcall(function()
				local first = (vehicles.getLineVehicles(l) or {})[1]
				local tv = first and component(first, "TRANSPORT_VEHICLE")
				local capacities = tv and tv.config.capacities
				local at = api.engine.system.simEntityAtTerminalSystem
				for i = 1, (capacities and #capacities or 0) do
					if capacities[i] > 0 then
						for s = 0, #(line.stops or {}) - 1 do
							waiting = waiting + #(at.getLineStopSimEntities(l, s, i - 1) or {})
						end
					end
				end
			end)
			local name = component(l, "NAME")
			out.lines[#out.lines + 1] = {
				id = id, name = text(name and name.name, 64), stops = stops, vehicles = fleet,
				load = load, waiting = math.min(whole(waiting), 4294967295), profit = whole(profit), income = income, recent = recent,
				problems = math.min(problems, 65535),
			}
		end
	end
	-- Its vehicles: on its lines, and in its depots.
	local function vehicle(v, line)
		if #out.vehicles >= ai.MAX_OWNED then return end
		local vid = reg and regmod.id(reg, "vehicles", v)
		local tv = component(v, "TRANSPORT_VEHICLE")
		if not (vid and tv) then return end
		local model = ""
		pcall(function()
			model = api.res.modelRep.getName(tv.transportVehicleConfig.vehicles[1].part.modelId)
		end)
		-- How far through its life it is, in percent, as the vehicle window
		-- reckons it (`vehicle_util.getAge`): its oldest part's purchase
		-- against the game's time, over that part's lifespan.
		local worn = 0
		pcall(function()
			local now = component(api.engine.util.getWorld(), "GAME_TIME").gameTime
			local first, span
			for _, part in ipairs(tv.transportVehicleConfig.vehicles) do
				local life = api.res.modelRep.get(part.part.modelId).metadata.maintenance.lifespan * 1000
				if first == nil or part.purchaseTime < first or (part.purchaseTime == first and life < span) then
					first, span = part.purchaseTime, life
				end
			end
			if first and span and span > 0 then
				worn = math.min(math.max(math.floor((now - first) / span * 100 + 0.5), 0), 65535)
			end
		end)
		out.vehicles[#out.vehicles + 1] = { id = vid, line = line, model = text(model, 128), worn = worn, idle = line == nil }
	end
	for _, l in ipairs(lines) do
		local lid = reg and regmod.id(reg, "lines", l)
		for _, v in ipairs(vehicles.getLineVehicles(l) or {}) do vehicle(v, lid) end
	end
	for _, d in ipairs(depots) do
		for _, depot in ipairs(d.construction.depots or {}) do
			if companies.ownerOf(api, depot) == entity then
				for _, v in ipairs(vehicles.getDepotVehicles(depot) or {}) do vehicle(v, nil) end
			end
		end
	end
	return { Company = out }
end

-- Where a station group stands: its first station's construction, else
-- that station's own place (a street stop).
local function groupAt(g)
	local group = component(g, "STATION_GROUP")
	local first = group and group.stations and group.stations[1]
	if not first then return nil end
	local _, c = constructionOf(first, { "getConstructionEntityForStation",
		"getConstructionEntityForSubconstruction" })
	return c and origin(c) or objectAt(first)
end

-- Other companies' passenger lines in `towns` (bridge 36): each town's
-- stops of theirs within TOWN_REACH of one of its districts' centres, each
-- stop once, and the towns one of their lines joins it to. A line carries
-- passengers where its first vehicle has room for them (its capacities'
-- first entry, cargo 0). `me` is the asking company's entity: its own
-- lines are none of these.
ai.TOWN_REACH = 600
ai.MAX_LINKED = 8
local function rivalsIn(towns, me)
	local own = {}
	if me then
		for _, l in ipairs(api.engine.system.lineSystem.getLinesForPlayer(me) or {}) do own[l] = true end
	end
	local reach2 = ai.TOWN_REACH * ai.TOWN_REACH
	local function townOf(at)
		for i, t in ipairs(towns) do
			for _, c in ipairs(t.centres) do
				local dx, dy = at.x - c.x, at.y - c.y
				if dx * dx + dy * dy <= reach2 then return i end
			end
		end
		return nil
	end
	local counted, at = {}, {}
	for _, l in ipairs(api.engine.system.lineSystem.getLines() or {}) do
		local carries = false
		if not own[l] then
			pcall(function()
				local first = (api.engine.system.transportVehicleSystem.getLineVehicles(l) or {})[1]
				local tv = first and component(first, "TRANSPORT_VEHICLE")
				carries = tv ~= nil and (tv.config.capacities[1] or 0) > 0
			end)
		end
		local line = carries and component(l, "LINE")
		if line then
			local on, seen = {}, {}
			for _, s in ipairs(line.stops or {}) do
				local g = s.stationGroup
				if at[g] == nil then
					local p = groupAt(g)
					at[g] = p and townOf(p) or false
				end
				local i = at[g]
				if i then
					if not counted[g] then
						counted[g] = true
						towns[i].rivals = math.min(towns[i].rivals + 1, 65535)
					end
					if not seen[i] then
						seen[i] = true
						on[#on + 1] = i
					end
				end
			end
			for _, i in ipairs(on) do
				for _, j in ipairs(on) do
					local linked = towns[i].linked
					local known = i == j
					for _, id in ipairs(linked) do if id == towns[j].id then known = true end end
					if not known and #linked < ai.MAX_LINKED then linked[#linked + 1] = towns[j].id end
				end
			end
		end
	end
end

function ai.towns(registry, seat, roster)
	local reg = type(registry) == "function" and registry() or registry
	local regmod = module("registry")
	local buildings = {}
	for _, b in ipairs(api.engine.getEntitiesWithComponent(api.type.ComponentType.TOWN_BUILDING) or {}) do
		local tb = component(b, "TOWN_BUILDING")
		if tb and tb.town then buildings[tb.town] = (buildings[tb.town] or 0) + 1 end
	end
	local towns = {}
	for _, t in ipairs(api.engine.getEntitiesWithComponent(api.type.ComponentType.TOWN) or {}) do
		local id = reg and regmod.id(reg, "towns", t)
		local name = component(t, "NAME")
		local centres, centreOf = {}, {}
		for landUse = 0, 2 do
			local ok, c = pcall(api.engine.util.town.getTownDistrictCenter, t, landUse)
			if ok and c then
				centres[#centres + 1] = pos(c)
				centreOf[landUse] = centres[#centres]
			end
		end
		if id and #centres > 0 then
			towns[#towns + 1] = {
				id = id, name = text(name and name.name, 48), pos = centres[1],
				buildings = buildings[t] or 0, centres = centres, needs = ai.needs(t, centreOf, centres[1]),
				rivals = 0, linked = {},
			}
		end
	end
	local c = roster and companyOf(roster, seat)
	pcall(rivalsIn, towns, c and c.entity)
	table.sort(towns, function(a, b)
		if a.buildings ~= b.buildings then return a.buildings > b.buildings end
		return a.id < b.id
	end)
	return { Towns = each(towns, ai.MAX_TOWNS) }
end

-- What `town` needs: each cargo its buildings take, how much it was
-- supplied and may take (the town window's cargo demand), at the centre of
-- the first district whose buildings take it (`TOWN.cargoNeeds`, one list a
-- district, residential first), the least supplied first.
function ai.needs(town, centreOf, centre)
	local ok, supply = pcall(api.engine.system.townBuildingSystem.getCargoSupplyAndLimit, town)
	if not ok or supply == nil then return {} end
	local useOf = {}
	local t = component(town, "TOWN")
	pcall(function()
		for landUse, cargos in ipairs(t.cargoNeeds) do
			for _, entry in ipairs(cargos) do
				if useOf[entry[1]] == nil then useOf[entry[1]] = landUse - 1 end
			end
		end
	end)
	local needs = {}
	for id, got in pairs(supply) do
		local name = api.res.cargoTypeRep.getName(id)
		if name and (got[2] or 0) >= 1 then
			needs[#needs + 1] = {
				cargo = text(name, 128), supply = math.floor(got[1] or 0), limit = math.floor(got[2]),
				at = centreOf[useOf[id]] or centre,
			}
		end
	end
	table.sort(needs, function(a, b)
		local ua, ub = a.limit - a.supply, b.limit - b.supply
		if ua ~= ub then return ua > ub end
		return a.cargo < b.cargo
	end)
	return each(needs, ai.MAX_NEEDS)
end

-- The land on a grid for a track's route (bridge AiQuestion::Land): each
-- cell's height at its centre, in centimetres, and what stands on it. A
-- construction, street or track whose bounds come within the cell's half
-- diagonal of its centre marks the cell, by the game's octree, which finds
-- a construction anywhere over its ground (2026-10-30: an industry from
-- 120 m off its origin). An edge with no street of its own is a track.
-- A 16 by 16 grid reads in about 7 ms on the rig's big fixture.
local LAND = { WATER = 1, BUILT = 2, STREET = 4, TRACK = 8, HOUSE = 16 }
function ai.land(at, cells, step)
	cells, step = tonumber(cells), tonumber(step)
	if cells == nil or cells < 1 or cells > ai.MAX_LAND then error("a land grid of " .. tostring(cells) .. " cells", 0) end
	if step == nil or step < 5 or step > 400 then error("a land step of " .. tostring(step) .. " m", 0) end
	local V, CT = api.type.Vec2f.new, api.type.ComponentType
	local octree, terrain = api.engine.util.octree, api.engine.terrain
	local reach = step * 0.71
	-- Town houses' constructions: a cell built on by these alone a
	-- construction may clear (bridge 39).
	local houses = {}
	for _, b in ipairs(api.engine.getEntitiesWithComponent(CT.TOWN_BUILDING) or {}) do
		local tb = component(b, "TOWN_BUILDING")
		if tb and type(tb.construction) == "number" then houses[tb.construction] = true end
	end
	local heights, flags = {}, {}
	for j = 0, cells - 1 do
		for i = 0, cells - 1 do
			local p = V(at.x + i * step, at.y + j * step)
			local f = 0
			heights[#heights + 1] = math.floor(terrain.getHeightAt(p) * 100 + 0.5)
			if terrain.isOnWater(p) then f = f + LAND.WATER end
			local built = octree.findEntitiesInCircle(p, reach, CT.CONSTRUCTION)
			if built and #built > 0 then
				f = f + LAND.BUILT
				local only = true
				for _, e in ipairs(built) do
					if not houses[e] then only = false end
				end
				if only then f = f + LAND.HOUSE end
			end
			local street, track = false, false
			for _, e in ipairs(octree.findEntitiesInCircle(p, reach, CT.BASE_EDGE) or {}) do
				if component(e, "BASE_EDGE_STREET") then street = true else track = true end
			end
			if street then f = f + LAND.STREET end
			if track then f = f + LAND.TRACK end
			flags[#flags + 1] = f
		end
	end
	return { Land = { heights = heights, flags = flags } }
end

-- Whether a company owns `edge`.
local function companys(edge)
	local owned = component(edge, "PLAYER_OWNED")
	return owned ~= nil and type(owned.player) == "number" and owned.player >= 0
end

-- Whether `node` has lane settings and a company's edge meets it: a split
-- of an edge ending there rebuilds the settings, and the room refuses that
-- where an edge they name is not the acting company's (D21,
-- junctions.into); the game's verdict, a preview, leaves them out.
local function companyJunction(node)
	if type(node) ~= "number" or component(node, "BASE_NODE_CONFIG") == nil then return false end
	local streets = api.engine.system and api.engine.system.streetSystem
	local ok, segments = pcall(function() return streets.getNodeSegments(node) end)
	for _, segment in ipairs(ok and segments or {}) do
		if companys(segment) then return true end
	end
	return false
end

function ai.streets(at, radius)
	local C = api.type.ComponentType
	local found = api.engine.util.octree.findEntitiesInCircle(api.type.Vec2f.new(at.x, at.y), radius, C.BASE_EDGE) or {}
	local streets = {}
	for _, e in ipairs(found) do
		local edge = component(e, "BASE_EDGE")
		if edge and component(e, "BASE_EDGE_STREET") and edge.position0 and edge.position1 then
			local a, b = edge.position0, edge.position1
			local dx, dy, dz = b.x - a.x, b.y - a.y, b.z - a.z
			-- Its curve's middle (the Hermite curve at one half), where a
			-- split of it must lie; the chord's on an edge without tangents.
			local ta, tb = edge.tangent0, edge.tangent1
			local mid = { x = (a.x + b.x) / 2, y = (a.y + b.y) / 2, z = (a.z + b.z) / 2 }
			if ta and tb then
				mid = { x = mid.x + (ta.x - tb.x) / 8, y = mid.y + (ta.y - tb.y) / 8, z = mid.z + (ta.z - tb.z) / 8 }
			end
			local mx, my = (a.x + b.x) / 2 - at.x, (a.y + b.y) / 2 - at.y
			streets[#streets + 1] = {
				a = pos(a), b = pos(b), mid = pos(mid), template = text(edge.roadTemplate, 128),
				length = whole(math.sqrt(dx * dx + dy * dy + dz * dz)),
				owned = companys(e),
				objects = math.min(#(edge.objects or {}), 255),
				structure = edge.type ~= nil and edge.type ~= 0,
				company_junction = companyJunction(edge.node0) or companyJunction(edge.node1),
				d = mx * mx + my * my,
			}
		end
	end
	table.sort(streets, function(x, y) return x.d < y.d end)
	local out = each(streets, ai.MAX_STREETS)
	for _, s in ipairs(out) do s.d = nil end
	return { Streets = out }
end

-- The classes a cargo type is of (its file's cargoClasses), as a set.
local function cargoClasses(cargo)
	local set = {}
	local ok = pcall(function()
		local c = api.res.cargoTypeRep.get(api.res.cargoTypeRep.find(cargo))
		local list = c.cargoClasses
		for i = 1, #list do set[tostring(list[i])] = true end
	end)
	return ok and set or nil
end

-- Whether a load configuration's cargo set takes `cargo` (of `classes`):
-- named, or of a class included and not excluded.
local function takes(set, cargo, classes)
	if set == nil then return false end
	local function has(list, value)
		for i = 1, #(list or {}) do if tostring(list[i]) == value then return true end end
		return false
	end
	if has(set.cargoTypesExcluded, cargo) then return false end
	if has(set.cargoTypesIncluded, cargo) then return true end
	local excluded, included = false, false
	for class in pairs(classes) do
		if has(set.cargoClassesExcluded, class) then excluded = true end
		if has(set.cargoClassesIncluded, class) then included = true end
	end
	return included and not excluded
end

function ai.models(mode, cargo)
	local wanted = api.type.enum and api.type.enum.TransportMode and api.type.enum.TransportMode[mode]
	if wanted == nil then error("no transport mode " .. tostring(mode), 0) end
	local classes = cargo and cargoClasses(cargo)
	if cargo and classes == nil then error("no cargo " .. tostring(cargo), 0) end
	local year = api.engine.util.getYear and api.engine.util.getYear() or 0
	local out = {}
	api.res.modelRep.forEachModelWithMetadata("transportVehicle", function(name)
		if #out >= ai.MAX_MODELS then return end
		local ok, entry = pcall(function()
			local model = api.res.modelRep.get(api.res.modelRep.find(name))
			local meta = model.metadata
			local tv = meta.transportVehicle
			local fits = false
			for _, m in ipairs(tv.transportModes or {}) do if m == wanted then fits = true end end
			if not fits then return nil end
			local available = meta.availability
			if available and ((available.yearFrom or 0) > year
					or ((available.yearTo or 0) ~= 0 and available.yearTo <= year)) then
				return nil
			end
			-- Its capacity: every compartment's first load, or with a cargo,
			-- each compartment's best load that takes it.
			local capacity = 0
			for _, compartment in ipairs(tv.compartments or {}) do
				local best = 0
				for k, config in ipairs(compartment.loadConfigs or {}) do
					local entry = config.cargoEntry
					local n = (entry and entry.capacity) or 0
					if cargo then
						if takes(entry and entry.cargoTypeSet, cargo, classes) and n > best then best = n end
					elseif k == 1 then
						best = n
					end
				end
				capacity = capacity + best
			end
			if cargo and capacity == 0 then return nil end
			-- TF3 keeps a road or rail vehicle's speed, weight and engines in
			-- `landVehicle`, its length in `extent`, as the game's vehicle
			-- store reads them (line_vehicle_mgmt/vehicle_store_util.tl).
			local land = meta.landVehicle or meta.roadVehicle or meta.railVehicle or meta.waterVehicle
				or meta.airVehicle
			local speed = land and land.topSpeed or 0
			local power, tractive, weight, length, electric = 0, 0, 0, 0, false
			pcall(function()
				local engine = meta.landVehicle.engines[1]
				if engine then
					power, tractive = engine.power or 0, engine.tractiveEffort or 0
					-- Engine type 2 is electric, as the game's trams and the
					-- C 6/8 have it (build 40425; 1 steam, 3 combustion, 0
					-- none): it runs only under catenary.
					electric = engine.type == 2
				end
			end)
			pcall(function() weight = (meta.landVehicle.weightEmpty or 0) / 1000 end)
			pcall(function() length = (meta.extent.bbMax.x - meta.extent.bbMin.x) * 10 end)
			return {
				model = text(name, 128),
				price = whole(meta.cost and meta.cost.price or 0),
				running = whole(meta.maintenance and meta.maintenance.runningCosts or 0),
				capacity = whole(capacity),
				speed = whole(speed * 3.6),
				lifespan = whole((meta.maintenance and meta.maintenance.lifespan or 0)),
				power = whole(power), tractive = whole(tractive), weight = whole(weight), length = whole(length),
				electric = electric,
			}
		end)
		if ok and entry then out[#out + 1] = entry end
	end)
	table.sort(out, function(a, b) return a.model < b.model end)
	return { Models = out }
end

-- The world's industries: where each stands, the cargo it takes, and what
-- it made over the last year and could make (the game's own industry
-- window reads the same, gui/main/cargo_react_util.tl). 94 on the big
-- fixture in 1 ms (2026-10-29).
ai.MAX_INDUSTRIES = 160
-- What other companies have at each of `industries` (bridge 37): their
-- station groups held by a construction within INDUSTRY_REACH of it, and
-- their cargo lines that stop at its own station group (`own`), each once.
-- A cargo line has room for a cargo other than passengers (its first
-- vehicle's capacities past the first entry). `me` is the asking company's
-- entity.
ai.INDUSTRY_REACH = 300
local function carriedAt(industries, me)
	local companies = module("companies")
	local reach2 = ai.INDUSTRY_REACH * ai.INDUSTRY_REACH
	for _, g in ipairs(api.engine.getEntitiesWithComponent(api.type.ComponentType.STATION_GROUP) or {}) do
		local owner = companies.ownerOf(api, g)
		if owner ~= nil and owner ~= me then
			local group = component(g, "STATION_GROUP")
			local first = group and group.stations and group.stations[1]
			local c
			if first then
				local _, held = constructionOf(first, { "getConstructionEntityForStation",
					"getConstructionEntityForSubconstruction" })
				c = held
			end
			if c then
				local at = origin(c)
				for _, i in ipairs(industries) do
					local dx, dy = at.x - i.at.x, at.y - i.at.y
					if dx * dx + dy * dy <= reach2 then i.out.carried = math.min(i.out.carried + 1, 65535) end
				end
			end
		end
	end
	local byGroup = {}
	for _, i in ipairs(industries) do if i.group then byGroup[i.group] = i end end
	local own = {}
	if me then
		for _, l in ipairs(api.engine.system.lineSystem.getLinesForPlayer(me) or {}) do own[l] = true end
	end
	for _, l in ipairs(api.engine.system.lineSystem.getLines() or {}) do
		if not own[l] then
			local cargo = false
			pcall(function()
				local first = (api.engine.system.transportVehicleSystem.getLineVehicles(l) or {})[1]
				local tv = first and component(first, "TRANSPORT_VEHICLE")
				for k, n in ipairs(tv and tv.config.capacities or {}) do
					if k > 1 and n > 0 then cargo = true end
				end
			end)
			local line = cargo and component(l, "LINE")
			local seen = {}
			for _, s in ipairs(line and line.stops or {}) do
				local i = byGroup[s.stationGroup]
				if i and not seen[i] then
					seen[i] = true
					i.out.carried = math.min(i.out.carried + 1, 65535)
				end
			end
		end
	end
end

function ai.industries(registry, seat, roster)
	local reg = type(registry) == "function" and registry() or registry
	local regmod = module("registry")
	local stock = api.engine.util.stock
	local out, places = {}, {}
	for _, e in ipairs(api.engine.getEntitiesWithComponent(api.type.ComponentType.INDUSTRY) or {}) do
		if #out >= ai.MAX_INDUSTRIES then break end
		pcall(function()
			local industry = component(e, "INDUSTRY")
			local c = industry and component(industry.construction, "CONSTRUCTION")
			if not (c and c.transf) then return end
			local io = stock.getInputsOutputsFromRules(industry.stockList)
			local inputs, outputs = {}, {}
			for _, id in ipairs(io[1] or {}) do
				if #inputs < 8 then inputs[#inputs + 1] = text(api.res.cargoTypeRep.getName(id), 128) end
			end
			for _, id in ipairs(io[2] or {}) do
				if #outputs < 8 then
					outputs[#outputs + 1] = {
						cargo = text(api.res.cargoTypeRep.getName(id), 128),
						index = id,
						per_year = whole(math.max(stock.getCargoProducedPerYear(industry.stockList, id) or 0, 0)),
						most = whole(math.max(stock.getCargoMaxProductionPerYear(industry.stockList, id) or 0, 0)),
					}
				end
			end
			-- Its own station, where a line loads and unloads its cargo.
			local station, terminals, ownGroup = nil, 0, nil
			local first = c.stations and c.stations[1]
			if first and reg then
				local okg, group = pcall(api.engine.system.stationGroupSystem.getStationGroup, first)
				station = okg and regmod.id(reg, "groups", group) or nil
				if okg and type(group) == "number" and group >= 0 then ownGroup = group end
				local s = component(first, "STATION")
				terminals = math.min(#((s and s.terminals) or {}), 255)
			end
			local ok, name = pcall(api.engine.util.getEntityName, e)
			-- On water, as fishing grounds stand: no road or rail reaches it.
			local at = origin(c)
			local wet = false
			pcall(function() wet = api.engine.terrain.isOnWater(api.type.Vec2f.new(at.x, at.y)) == true end)
			out[#out + 1] = { name = text(ok and name or "", 48), pos = pos(at), station = station,
				terminals = terminals, inputs = inputs, outputs = outputs, water = wet, carried = 0 }
			places[#places + 1] = { out = out[#out], at = at, group = ownGroup }
		end)
	end
	local c = roster and companyOf(roster, seat)
	pcall(carriedAt, places, c and c.entity)
	return { Industries = out }
end

-- Whether a truck can drive between the street networks by two places:
-- the game's pathfinder from the street nodes within 200 m of one to those
-- of the other. Street nodes only: about half of the network nodes by an
-- industry are its own (2026-10-30, 30 of 58 by a farm), from which no
-- truck path starts.
ai.ROUTE_REACH = 200
function ai.route(from, to)
	local octree, Vec2f = api.engine.util.octree, api.type.Vec2f
	local streets = api.engine.system.streetSystem
	local function near(p)
		local nodes = octree.findTransportNetworkNodesInCircle(Vec2f.new(p.x, p.y), ai.ROUTE_REACH) or {}
		local some = {}
		for i = 1, #nodes do
			if #some >= 8 then break end
			local ok, segments = pcall(function() return streets.getNodeStreetSegments(nodes[i].entity) end)
			if ok and segments and #segments > 0 then some[#some + 1] = nodes[i] end
		end
		return some
	end
	local a, b = near(from), near(to)
	local edges = 0
	if #a > 0 and #b > 0 then
		local path = api.engine.util.pathfinding.findPathNodeToNode(a, b, { api.type.enum.TransportMode.TRUCK })
		edges = #(path or {})
	end
	return { Route = { connected = edges > 0, edges = whole(edges) } }
end

-- The game's verdict on `action`, built as `seat`'s company would.
-- The last verdict's parts, for the cost log: making its proposal (and
-- apply's own parts of that) and the game's verdict on it.
ai.lastVerdict = nil

-- The networks' nodes the verdicts share, read whole at most once a while
-- (tpf3mp/apply.lua, `nodeCache`), and when.
ai.NODE_CACHE_FOR = 5
local nodeCache, nodeCacheAt = {}, nil

function ai.verdict(seat, action, roster)
	local c = companyOf(roster, seat)
	local apply = module("apply")
	local t0 = os.clock()
	if nodeCacheAt == nil or t0 - nodeCacheAt >= ai.NODE_CACHE_FOR then
		nodeCache, nodeCacheAt = {}, t0
	end
	local proposal, context = apply.proposalOf(action, { company = c and c.entity, roster = roster,
		nodeCache = nodeCache })
	local t1 = os.clock()
	local made = { make = t1 - t0, parts = apply.costText and apply.costText(apply.lastCost) or "" }
	ai.lastVerdict = made
	if proposal == nil then
		return { Verdict = { ok = false, cost = 0, messages = { text(context, 160) }, collisions = 0 } }
	end
	local data = api.engine.util.proposal.makeProposalData(proposal, context)
	made.game = os.clock() - t1
	local state = data and data.errorState
	local messages = {}
	for _, m in ipairs(state and state.messages or {}) do
		if #messages < ai.MAX_MESSAGES then messages[#messages + 1] = text(m, 160) end
	end
	for _, m in ipairs(state and state.warnings or {}) do
		if #messages < ai.MAX_MESSAGES then messages[#messages + 1] = text("warning: " .. tostring(m), 160) end
	end
	-- What it collides with that stands: an entity of the world's, not one
	-- the proposal makes (a negative id). A depot's own street, split for its
	-- entrance, is always among the proposal's (2026-10-30, the fixture's
	-- towns: "Collision" said of every depot, built all the same, critical
	-- for none); another street, a track or an industry it reaches is not.
	local collisions = 0
	pcall(function()
		for _, e in ipairs(data.collisionInfo.collisionEntities or {}) do
			local ok, entity = pcall(function() return e.entity end)
			if not ok or entity == nil then entity = e end
			if type(entity) ~= "number" or entity >= 0 then collisions = collisions + 1 end
		end
	end)
	return { Verdict = {
		ok = not (state and state.critical) and data ~= nil,
		cost = whole(data and data.costs or 0),
		messages = messages,
		collisions = math.min(collisions, 65535),
	} }
end

-- The answer to one question, or an error.
function ai.answer(ask, roster, registry)
	local q = ask.question
	if q == "Company" then return ai.company(ask.seat, roster, registry) end
	if q == "Towns" then return ai.towns(registry, ask.seat, roster) end
	if type(q) == "table" and q.Streets then return ai.streets(q.Streets.at, q.Streets.radius) end
	if type(q) == "table" and q.Models then return ai.models(q.Models.mode, q.Models.cargo) end
	if q == "Industries" then return ai.industries(registry, ask.seat, roster) end
	if type(q) == "table" and q.Route then return ai.route(q.Route.from, q.Route.to) end
	if type(q) == "table" and q.Land then return ai.land(q.Land.at, q.Land.cells, q.Land.step) end
	if type(q) == "table" and q.Verdict then return ai.verdict(ask.seat, q.Verdict, roster) end
	error("a question of no kind", 0)
end

-- What the answers cost the driving game, by kind of question, and its
-- slowest frame of them, logged once a minute while it answers: for the
-- driver's frame budget (docs/AI_PLAYERS.md, "The driver"), never for the
-- world. os.clock is the elapsed time on Windows.
ai.COSTS_EVERY = 60
local costs = { kinds = {}, order = {}, frames = 0, slowest = 0, since = nil }

local function kindOf(ask)
	local q = type(ask) == "table" and ask.question
	if type(q) == "string" then return q end
	if type(q) == "table" then return tostring((next(q))) end
	return "?"
end

local function spent(kind, seconds, detail)
	local k = costs.kinds[kind]
	if k == nil then
		k = { n = 0, s = 0, max = 0 }
		costs.kinds[kind] = k
		costs.order[#costs.order + 1] = kind
	end
	k.n, k.s = k.n + 1, k.s + seconds
	if seconds > k.max then k.max, k.detail = seconds, detail end
end

local function report(link, now)
	if costs.since == nil then costs.since = now return end
	if now - costs.since < ai.COSTS_EVERY or costs.frames == 0 then return end
	local parts = {}
	for _, kind in ipairs(costs.order) do
		local k = costs.kinds[kind]
		parts[#parts + 1] = string.format("%s %d (mean %.1f ms, slowest %.1f ms%s)", kind, k.n,
			k.s * 1000 / k.n, k.max * 1000, k.detail and ("; " .. k.detail) or "")
	end
	if type(link.log) == "function" then
		pcall(link.log, link, string.format("ai: answers in %.0f s: %s; %d frame(s) answered, the slowest %.1f ms",
			now - costs.since, table.concat(parts, ", "), costs.frames, costs.slowest * 1000))
	end
	costs = { kinds = {}, order = {}, frames = 0, slowest = 0, since = now }
end

-- Answers the questions waiting, while the frame's budget lasts. `link` is
-- the hook's (tpf3mp/bridge.lua); `room` = { roster =, registry = }, each a
-- table or a function that reads it. The roster is the game script's whole
-- one (tpf3mp/companies.lua): the GUI's own copy keeps only the player's
-- company's loan offers.
-- The least time between two verdicts, in seconds, and a verdict alone in
-- its frame: each holds the driving game's frame for the game's own
-- verdict (about 8 ms, 2026-10-30), so several AIs' verdicts at once are
-- spread over frames rather than stacked in a row. A verdict waiting holds
-- the questions behind it, in their order.
ai.VERDICT_GAP = 0.25
local heldAsk, lastVerdictAt = nil, nil

function ai.serve(link, room, budget)
	if not (link and link.aiAsk) then return 0 end
	local t0 = os.clock()
	local served = 0
	while (os.clock() - t0) * 1000 < (budget or ai.BUDGET_MS) do
		local ask = heldAsk or link:aiAsk()
		heldAsk = nil
		if ask == nil then break end
		local q = ask.question
		local isVerdict = type(q) == "table" and q.Verdict ~= nil
		if isVerdict and (served > 0 or (lastVerdictAt and os.clock() - lastVerdictAt < ai.VERDICT_GAP)) then
			heldAsk = ask
			break
		end
		local roster = room and room.roster
		if type(roster) == "function" then roster = roster() end
		local t1 = os.clock()
		ai.lastVerdict = nil
		local ok, answer = pcall(ai.answer, ask, roster, room and room.registry)
		local v = ai.lastVerdict
		spent(kindOf(ask), os.clock() - t1, v and string.format("its proposal %.1f ms%s, the game's verdict %.1f ms",
			v.make * 1000, v.parts ~= "" and (" [" .. v.parts .. "]") or "", (v.game or 0) * 1000))
		if ok and type(answer) == "table" then
			link:aiAnswer(ask.id, true, answer)
		else
			link:aiAnswer(ask.id, false, text(answer, 250))
		end
		served = served + 1
		if isVerdict then lastVerdictAt = os.clock() break end
	end
	local now = os.clock()
	if served > 0 then
		costs.frames = costs.frames + 1
		if now - t0 > costs.slowest then costs.slowest = now - t0 end
	end
	report(link, now)
	return served
end

return ai

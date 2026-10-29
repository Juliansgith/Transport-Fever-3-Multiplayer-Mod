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
local function constructions()
	local byStation, depots = {}, {}
	for _, e in ipairs(api.engine.getEntitiesWithComponent(api.type.ComponentType.CONSTRUCTION) or {}) do
		local c = component(e, "CONSTRUCTION")
		if c and c.transf then
			local t = c.transf
			local origin = { x = t[13], y = t[14], z = t[15] }
			for _, s in ipairs(c.stations or {}) do byStation[s] = origin end
			if c.depots and #c.depots > 0 then
				depots[#depots + 1] = { entity = e, construction = c, origin = origin }
			end
		end
	end
	return byStation, depots
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

function ai.company(seat, roster, registry)
	local reg = type(registry) == "function" and registry() or registry
	local regmod = module("registry")
	local c = companyOf(roster, seat)
	local entity = c and c.entity
	local out = {
		money = 0,
		year = whole(api.engine.util.getYear and api.engine.util.getYear() or 0),
		stations = {}, depots = {}, lines = {}, vehicles = {}, offers = {}, loans = 0,
	}
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
		for _, loan in ipairs(roster.loans or {}) do
			if loan.company == c.id and out.loans < 255 then out.loans = out.loans + 1 end
		end
	end
	local finance = api.engine.util.finance
	out.money = whole(finance and finance.getPlayersBalance and finance.getPlayersBalance(entity) or 0)
	local companies = module("companies")
	local byStation, depots = constructions()
	-- Station groups: owned by the company, named by the room's ids.
	for _, g in ipairs(api.engine.getEntitiesWithComponent(api.type.ComponentType.STATION_GROUP) or {}) do
		if #out.stations >= ai.MAX_OWNED then break end
		if companies.ownerOf(api, g) == entity then
			local id = reg and regmod.id(reg, "groups", g)
			local group = component(g, "STATION_GROUP")
			local first = group and group.stations and group.stations[1]
			local at = first and (byStation[first] or objectAt(first))
			if id and at then
				local terminals = 0
				for _, s in ipairs(group.stations or {}) do
					local station = component(s, "STATION")
					terminals = terminals + #((station and station.terminals) or {})
				end
				out.stations[#out.stations + 1] = { id = id, pos = pos(at), terminals = terminals, waiting = 0 }
			end
		end
	end
	for _, d in ipairs(depots) do
		if #out.depots >= ai.MAX_OWNED then break end
		if companies.ownerOf(api, d.construction.depots[1]) == entity then
			local gate, connected = entrance(d.construction, d.origin)
			out.depots[#out.depots + 1] = {
				file = text(d.construction.fileName, 128), at = pos(d.origin),
				entrance = gate and pos(gate) or nil, connected = connected,
			}
		end
	end
	local lines = api.engine.system.lineSystem.getLinesForPlayer(entity) or {}
	local vehicles = api.engine.system.transportVehicleSystem
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
			local name = component(l, "NAME")
			out.lines[#out.lines + 1] = {
				id = id, name = text(name and name.name, 64), stops = stops, vehicles = fleet,
				load = 0, waiting = 0, income = 0, costs = 0, problems = math.min(problems, 65535),
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
		out.vehicles[#out.vehicles + 1] = { id = vid, line = line, model = text(model, 128), age = 0, idle = line == nil }
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

function ai.towns(registry)
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
		local centres = {}
		for landUse = 0, 2 do
			local ok, c = pcall(api.engine.util.town.getTownDistrictCenter, t, landUse)
			if ok and c then centres[#centres + 1] = pos(c) end
		end
		if id and #centres > 0 then
			towns[#towns + 1] = {
				id = id, name = text(name and name.name, 48), pos = centres[1],
				buildings = buildings[t] or 0, centres = centres,
			}
		end
	end
	table.sort(towns, function(a, b)
		if a.buildings ~= b.buildings then return a.buildings > b.buildings end
		return a.id < b.id
	end)
	return { Towns = each(towns, ai.MAX_TOWNS) }
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
			local owned = component(e, "PLAYER_OWNED")
			streets[#streets + 1] = {
				a = pos(a), b = pos(b), mid = pos(mid), template = text(edge.roadTemplate, 128),
				length = whole(math.sqrt(dx * dx + dy * dy + dz * dz)),
				owned = owned ~= nil and type(owned.player) == "number" and owned.player >= 0,
				objects = math.min(#(edge.objects or {}), 255),
				structure = edge.type ~= nil and edge.type ~= 0,
				d = mx * mx + my * my,
			}
		end
	end
	table.sort(streets, function(x, y) return x.d < y.d end)
	local out = each(streets, ai.MAX_STREETS)
	for _, s in ipairs(out) do s.d = nil end
	return { Streets = out }
end

function ai.models(mode)
	local wanted = api.type.enum and api.type.enum.TransportMode and api.type.enum.TransportMode[mode]
	if wanted == nil then error("no transport mode " .. tostring(mode), 0) end
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
			local capacity = 0
			for _, compartment in ipairs(tv.compartments or {}) do
				local config = compartment.loadConfigs and compartment.loadConfigs[1]
				capacity = capacity + ((config and config.cargoEntry and config.cargoEntry.capacity) or 0)
			end
			local land = meta.roadVehicle or meta.railVehicle or meta.waterVehicle or meta.airVehicle
			local speed = land and land.topSpeed or 0
			return {
				model = text(name, 128),
				price = whole(meta.cost and meta.cost.price or 0),
				running = whole(meta.maintenance and meta.maintenance.runningCosts or 0),
				capacity = whole(capacity),
				speed = whole(speed * 3.6),
				lifespan = whole((meta.maintenance and meta.maintenance.lifespan or 0)),
			}
		end)
		if ok and entry then out[#out + 1] = entry end
	end)
	table.sort(out, function(a, b) return a.model < b.model end)
	return { Models = out }
end

-- The game's verdict on `action`, built as `seat`'s company would.
function ai.verdict(seat, action, roster)
	local c = companyOf(roster, seat)
	local proposal, context = module("apply").proposalOf(action, { company = c and c.entity, roster = roster })
	if proposal == nil then
		return { Verdict = { ok = false, cost = 0, messages = { text(context, 160) }, collisions = 0 } }
	end
	local data = api.engine.util.proposal.makeProposalData(proposal, context)
	local state = data and data.errorState
	local messages = {}
	for _, m in ipairs(state and state.messages or {}) do
		if #messages < ai.MAX_MESSAGES then messages[#messages + 1] = text(m, 160) end
	end
	for _, m in ipairs(state and state.warnings or {}) do
		if #messages < ai.MAX_MESSAGES then messages[#messages + 1] = text("warning: " .. tostring(m), 160) end
	end
	local collisions = 0
	pcall(function() collisions = #(data.collisionInfo.collisionEntities or {}) end)
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
	if q == "Towns" then return ai.towns(registry) end
	if type(q) == "table" and q.Streets then return ai.streets(q.Streets.at, q.Streets.radius) end
	if type(q) == "table" and q.Models then return ai.models(q.Models.mode) end
	if type(q) == "table" and q.Verdict then return ai.verdict(ask.seat, q.Verdict, roster) end
	error("a question of no kind", 0)
end

-- Answers the questions waiting, while the frame's budget lasts. `link` is
-- the hook's (tpf3mp/bridge.lua); `room` = { roster =, registry = }, each a
-- table or a function that reads it. The roster is the game script's whole
-- one (tpf3mp/companies.lua): the GUI's own copy keeps only the player's
-- company's loan offers.
function ai.serve(link, room, budget)
	if not (link and link.aiAsk) then return 0 end
	local t0 = os.clock()
	local served = 0
	while (os.clock() - t0) * 1000 < (budget or ai.BUDGET_MS) do
		local ask = link:aiAsk()
		if ask == nil then break end
		local roster = room and room.roster
		if type(roster) == "function" then roster = roster() end
		local ok, answer = pcall(ai.answer, ask, roster, room and room.registry)
		if ok and type(answer) == "table" then
			link:aiAnswer(ask.id, true, answer)
		else
			link:aiAnswer(ask.id, false, text(answer, 250))
		end
		served = served + 1
	end
	return served
end

return ai

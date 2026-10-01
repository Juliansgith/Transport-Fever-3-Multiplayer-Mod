-- tpf3mp/follow.lua -- the GUI's "my company", in each of the GUI's Lua
-- states.
--
-- TF3's windows and its HUD ask api.engine.util.getPlayer() whose money to
-- show, what is the player's own and what is "Foreign"
-- (entity_window/eow_extension_util.tl, entity_util.isOwnedByPlayer), and
-- every GUI script looks it up when it runs. The game's GUI runs in more
-- than one Lua state (build 40408: the Multiplayer plugin's, and the one the
-- HUD's icons and the line manager's depots are drawn in), each with its own
-- api table, so each state that shows the player's things is given the
-- answer here: the company this player plays for. The game scripts' states
-- keep the game's own answer, the save's player, so the simulation is the
-- same in every game; what the player does is booked to their company by
-- the room (tpf3mp/apply.lua) whatever the GUI named.
--
-- Pure Lua; the tests hand it a fake api.

local follow = {}

-- Replaces api.engine.util.getPlayer in this Lua state, once, with one that
-- answers `mine()`, the player entity of the company this player plays for,
-- or, when that is nil (outside the room, before the roster is read, the
-- room's first company), the game's own answer. Returns true, or false and
-- why.
function follow.install(api, mine)
	if type(package) == "table" and type(package.loaded) == "table" and package.loaded["tpf3mp.followed"] then
		return true
	end
	local ok, util = pcall(function() return api.engine.util end)
	if not ok then util = nil end
	-- A function, or a callable table, as the game's bindings are (build
	-- 40408: a table with a metatable).
	local original = ok and util ~= nil and select(2, pcall(function() return util.getPlayer end)) or nil
	if type(original) ~= "function" and type(original) ~= "table" and type(original) ~= "userdata" then
		return false, "no api.engine.util.getPlayer (" .. type(util) .. ", " .. type(original) .. ")"
	end
	local replaced, why = pcall(function()
		util.getPlayer = function(...)
			local got, entity = pcall(mine)
			if got and type(entity) == "number" then return entity end
			return original(...)
		end
	end)
	-- A binding may take the assignment and keep its own function.
	local took = replaced and select(2, pcall(function() return util.getPlayer ~= original end))
	if took ~= true then
		return false, "api.engine.util (" .. type(util) .. ") keeps its getPlayer"
			.. (why and (": " .. tostring(why)) or "")
	end
	if type(package) == "table" and type(package.loaded) == "table" then
		package.loaded["tpf3mp.followed"] = true
	end
	follow.loans(api, mine)
	return true
end

-- The game's finance window reads the loans it lists and offers from the
-- loan script's state (finances_loan_gui.tl, LoanBoard:
-- getComponent(getEntityForGameScript(LOAN_SCRIPT), GAME_SCRIPT).state),
-- which keeps the room's first company's loans only. In this GUI state the
-- loan script's component is answered, for a player of another company,
-- with that company's own loans and the offers it can take
-- (tpf3mp/companies.lua, loanTable); its Obtain and Repay then go to the
-- room as that company's (tpf3mp/guard.lua). The simulation's states, and
-- the loan script's own state, are left alone. Where the company's loans
-- cannot be read, the window shows none and offers none, never the first
-- company's.
follow.LOAN_SCRIPT = "::/game_mechanics/finance/loan.gs"
-- Seconds a company's loans are read for, at most.
follow.LOANS_EVERY = 0.5

function follow.loans(api, mine)
	if type(package) == "table" and type(package.loaded) == "table" and package.loaded["tpf3mp.loansFollowed"] then
		return true
	end
	local engine = api.engine
	local original = engine and engine.getComponent
	if original == nil then return false, "no api.engine.getComponent" end
	local function companies()
		local loaded = type(package) == "table" and package.loaded and package.loaded["tpf3mp.companies"]
		if loaded then return loaded end
		return ug_require("tpf3mp_1::/scripts/tpf3mp/companies.lua")
	end
	local function now()
		local ok, t = pcall(os.clock)
		return ok and t or 0
	end
	local loanEntity, cached, cachedFor, cachedAt = nil, nil, nil, nil
	-- Offers in place of those on the first company's cooldown, drawn once
	-- per kind (the game's loan_util), kept until the first company's
	-- offer of that kind is back.
	local fresh = {}
	local function freshOffer(kind)
		if fresh[kind] == nil then
			pcall(function()
				local util = ug_require("::/game_mechanics/finance/loan_util.tl")
				local make = util and util["create" .. tostring(kind) .. "Loan"]
				if make then fresh[kind] = make() end
			end)
		end
		return fresh[kind]
	end
	local function companyLoans(company)
		local t = now()
		if cached ~= nil and cachedFor == company and t - cachedAt < follow.LOANS_EVERY then return cached end
		local table0 = { availableLoans = {}, obtainedLoans = {}, freeId = 0 }
		local ok, built = pcall(function()
			local c = companies()
			local state = c.scriptState(api)
			local roster = state and state.companies
			local own = roster and c.byEntity(roster, company)
			if not own then return nil end
			local real = original(loanEntity, api.type.ComponentType.GAME_SCRIPT)
			real = real and real.state
			for _, offer in ipairs(type(real) == "table" and real.availableLoans or {}) do
				if type(offer) == "table" and offer.cooldownUntil == nil then fresh[offer.type] = nil end
			end
			return c.loanTable(roster, own.id, real, api.util.getDefaultMonthDuration(), freshOffer)
		end)
		cached, cachedFor, cachedAt = (ok and built) or table0, company, t
		return cached
	end
	local replaced = pcall(function()
		engine.getComponent = function(entity, kind, ...)
			if kind ~= nil and entity ~= nil then
				local isLoans = false
				pcall(function()
					if loanEntity == nil or loanEntity < 0 then
						loanEntity = api.engine.system.gameScriptSystem.getEntityForGameScript(follow.LOAN_SCRIPT)
					end
					isLoans = type(loanEntity) == "number" and loanEntity >= 0 and entity == loanEntity
						and kind == api.type.ComponentType.GAME_SCRIPT
				end)
				if isLoans then
					local got, company = pcall(mine)
					if got and type(company) == "number" then
						return { state = companyLoans(company) }
					end
				end
			end
			return original(entity, kind, ...)
		end
	end)
	if not replaced then return false, "api.engine keeps its getComponent" end
	if type(package) == "table" and type(package.loaded) == "table" then
		package.loaded["tpf3mp.loansFollowed"] = true
	end
	return true
end

-- The player entity of the company the player `me` (64 hex digits) plays
-- for in `roster` (tpf3mp/companies.lua), or nil: not in the roster, or
-- playing for the room's first, which is the save's own player anyway.
function follow.companyOf(roster, me)
	if type(roster) ~= "table" or type(roster.members) ~= "table" or type(me) ~= "string" then return nil end
	for _, m in ipairs(roster.members) do
		if m.player == me then
			for _, c in ipairs(roster.list or {}) do
				if c.id == m.company and not c.gone and c.id ~= 0 then return c.entity end
			end
		end
	end
	return nil
end

return follow

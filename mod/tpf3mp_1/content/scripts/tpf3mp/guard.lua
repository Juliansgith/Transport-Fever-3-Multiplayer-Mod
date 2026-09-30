-- tpf3mp/guard.lua -- the room's guard on the commands the GUI sends.
--
-- Transport Fever 3's GUI sends most of what a player does as commands,
-- through api.cmd.sendCommand: buying, selling and assigning vehicles,
-- lines, loans (as script events), a construction's parameters, the speed
-- (docs/HOOKS.md, "The player's commands"). In the room's game a command
-- must run in every game at the same update or in none, so the guard sits in
-- front of sendCommand in the GUI's Lua state:
--
-- - a command of a kind in PASS is sent as the player gave it;
-- - a command CARRY makes an action of is handed to the room instead,
--   which orders it for every game, this one included; its callback hears
--   on the next frame that it went, as the game's own windows expect;
-- - every other kind is refused, as docs/PLAN.md (Part 3) says of every
--   action the room does not carry yet: it is not sent, its callback hears
--   on the next frame that it failed, and the player is told.
--
-- A command's kind is the name of the factory that made it
-- (api.cmd.make<Kind>Cmd), which the guard wraps to note it; a command no
-- wrapped factory made is refused. Outside the room's game (before the room
-- begins, after it ends) every command is sent as it would be.
--
-- Only the GUI state's api.cmd is wrapped. The mod's game script applies the
-- room's actions through its own state's api.cmd, which is left alone.
--
-- Pure Lua; the tests hand install() a fake api.cmd.

local guard = {}

-- Lua 5.2 (the game's) has table.unpack; Lua 5.1 (the tests') unpack.
local unpackArgs = table.unpack or unpack

-- The kinds sent as they are in the room's game, and why.
guard.PASS = {
	-- The speed row. In the room's game the step gate runs the room's pace
	-- whatever the game's own speed says, and reads that speed as the
	-- player's request to the room (docs/HOOKS.md, "The step gate in the
	-- game").
	makeGameSetSpeedCmd = true,
}

-- The commands the room carries, by kind: each makes an action table
-- (tpf3mp_proto::action, in the game's units) of the command's arguments,
-- or nil for one it does not carry, which is then refused.
guard.CARRY = {
	-- The finance window's loans (finances_loan_gui.tl): the loan script's
	-- events, with the loans as the script keeps them.
	makeScriptingSendEventCmd = function(_src, id, name, param)
		if id ~= "Loan" or type(param) ~= "table" then return nil end
		if name == "Obtain" and type(param[1]) == "table" and type(param[2]) == "table" then
			return { Loan = { Take = { next = param[1], offer = param[2] } } }
		elseif name == "Repay" and type(param[2]) == "table" then
			return { Loan = { Repay = { loan = param[2] } } }
		end
		return nil
	end,
}

-- What the player is told a refused kind is, where "this" would not do.
guard.WHAT = {
	makeVehicleBuyCmd = "buying vehicles",
	makeVehicleSellCmd = "selling vehicles",
	makeVehicleReplaceCmd = "replacing vehicles",
	makeVehicleSetLineCmd = "assigning vehicles to lines",
	makeVehicleSendToDepotCmd = "sending vehicles to a depot",
	makeVehicleReverseCmd = "reversing vehicles",
	makeVehicleSetStoppedByUserCmd = "stopping vehicles",
	makeVehicleSetModifiersCmd = "changing vehicles",
	makeLineCreateCmd = "creating lines",
	makeLineUpdateCmd = "changing lines",
	makeLineDestroyCmd = "deleting lines",
	makeWorldBuildProposalCmd = "building from this window",
	makeEntitySetNameCmd = "renaming",
	makeEntitySetColorCmd = "changing colours",
	makeGameSetCalendarSpeedCmd = "changing the calendar speed",
}

-- The factories the game's API reference declares (build 40408,
-- api/tealdef/api/cmd.d.tl). They are wrapped by name as well as by what
-- pairs() finds, in case api.cmd serves some through a metatable.
guard.FACTORIES = {
	"makeAnimalSetStateCmd", "makeAnimalSpawnAtCmd", "makeClearLogbooksCmd",
	"makeComponentExchangeCmd", "makeCreateIndustryExtendProposalCmd",
	"makeCustomEntityCreateCmd", "makeCustomEntityDestroyCmd",
	"makeCustomEntityUpdateStateCmd", "makeCustomEntityUpdateTransformationCmd",
	"makeCustomVehicleCreateOrUpdateCmd", "makeEntitySetColorCmd",
	"makeEntitySetEmissionsCmd", "makeEntitySetNameCmd", "makeEntitySetPlayerCmd",
	"makeGameAddPlayerCmd", "makeGamePerformSimulationStepsCmd",
	"makeGameSetCalendarSpeedCmd", "makeGameSetCloudCoverageCmd",
	"makeGameSetDateCmd", "makeGameSetSpeedCmd", "makeGameSetTimeOfDayCmd",
	"makeIndustrySetDespawnTimeCmd", "makeIndustrySetManualDevelopmentCmd",
	"makeJournalBookAssetCmd", "makeJournalClearAllCmd", "makeJournalLogEntryCmd",
	"makeLineCreateCmd", "makeLineDestroyCmd", "makeLineUpdateCmd",
	"makeMaintenanceCostUpdateCmd", "makeScriptingSendEventCmd",
	"makeSimPersonSetStateCmd", "makeStockListDiscardCargoCmd",
	"makeStockListSetModifiersCmd", "makeStockListSetStocksCargoTypeCmd",
	"makeStockSetCargoAmountCmd", "makeTownAutoDetectConnectionsCmd",
	"makeTownBuildingSetBlockedDevelopmentCmd", "makeTownConnectWithIndustriesCmd",
	"makeTownCreateCmd", "makeTownCustomDistributionWeightsCmd",
	"makeTownDestroyCmd", "makeTownDevelopAtCmd", "makeTownSetDevelopmentActiveCmd",
	"makeTownSetInitialLandUseCapacitiesCmd", "makeTownUpdateCargoNeedsCmd",
	"makeTownUpdateSizeCmd", "makeVehicleBuyCmd", "makeVehicleReplaceCmd",
	"makeVehicleReverseCmd", "makeVehicleSellCmd", "makeVehicleSendToDepotCmd",
	"makeVehicleSetLineCmd", "makeVehicleSetManualDepartureCmd",
	"makeVehicleSetModifiersCmd", "makeVehicleSetStoppedByUserCmd",
	"makeVehicleTryToDepartCmd", "makeWorldBuildProposalCmd",
	"makeWorldChangeWindCmd", "makeWorldReplaceTerrainCmd",
	"makeWorldSetBulldozableCmd",
}

-- What the player is told when a command of `kind` is refused.
function guard.notice(kind)
	return "Not in multiplayer yet: " .. (guard.WHAT[kind] or "this action")
end

-- The api.cmd tables already guarded, so a second install() changes
-- nothing.
local guarded = setmetatable({}, { __mode = "k" })

-- Puts the guard in front of `cmd` (the GUI state's api.cmd). `env` is:
--   inRoom()      -> whether the room's game runs;
--   command(t)    -> hands an action table to the room: true, or nil and why;
--   refused(kind) -> a command of `kind` (nil: made by no factory the guard
--                    knows) was refused;
--   later(fn)     -> runs fn on the next frame.
-- Returns the number of factories wrapped, or nil and why the guard could
-- not be put there.
function guard.install(cmd, env)
	if type(cmd) ~= "table" then return nil, "api.cmd is not a table" end
	if guarded[cmd] then return guarded[cmd] end
	local send = cmd.sendCommand
	if send == nil then return nil, "api.cmd has no sendCommand" end

	-- The factory each command came from, and its arguments, by the command
	-- itself.
	local kinds = setmetatable({}, { __mode = "k" })
	local calls = setmetatable({}, { __mode = "k" })
	local factories = {}
	for name, factory in pairs(cmd) do
		if type(name) == "string" and name:match("^make.+Cmd$") then
			factories[name] = factory
		end
	end
	for _, name in ipairs(guard.FACTORIES) do
		if factories[name] == nil then factories[name] = cmd[name] end
	end
	local wrapped = 0
	for name, factory in pairs(factories) do
		cmd[name] = function(...)
			local command = factory(...)
			local t = type(command)
			if t == "table" or t == "userdata" then
				kinds[command] = name
				if guard.CARRY[name] then calls[command] = { n = select("#", ...), ... } end
			end
			return command
		end
		wrapped = wrapped + 1
	end

	-- The arguments go on exactly as given: a callback left out is not the
	-- same, to the game, as one passed as nil.
	cmd.sendCommand = function(command, ...)
		if not env.inRoom() then
			return send(command, ...)
		end
		local kind = kinds[command]
		if kind ~= nil and guard.PASS[kind] then
			return send(command, ...)
		end
		local callback = ...
		local carry, args = kind and guard.CARRY[kind], calls[command]
		local action = carry and args and carry(unpackArgs(args, 1, args.n))
		if action then
			local ok, why = env.command(action)
			if ok then
				if callback ~= nil then
					env.later(function() callback(command, true, {}) end)
				end
				return
			end
			env.refused(kind, why)
		else
			env.refused(kind)
		end
		if callback ~= nil then
			env.later(function() callback(command, false, {}) end)
		end
	end
	guarded[cmd] = wrapped
	return wrapped
end

return guard

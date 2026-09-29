# What Urban Games' official API reference tells us -- 2026-09-29

On release day Urban Games' modding wiki went live at
`wiki.transportfever3.com`, and with it a generated script reference at
`wiki.transportfever3.com/script-doc/` (linked from the wiki's
[Scripting API](https://wiki.transportfever3.com/doku.php?id=modding:scripting:api)
page as "Scripting Reference"): one page per module, typed like the game's
own Teal (`.tl`) declarations. It is the reference the probe
(`tools/probe/tf3`) was built to reconstruct, and it now exists.

Findings here are labelled **DOCUMENTED**: read from that reference, which
is Urban Games' own, not inferred and not a mod's usage. It is stronger
than the **REPORTED** label of
[TF3_MODS_2026-09-27.md](TF3_MODS_2026-09-27.md) and
[TF3_MODHUB_SCRIPT_MODS_2026-09-29.md](TF3_MODHUB_SCRIPT_MODS_2026-09-29.md),
which read third-party mods. It is still not **MEASURED**: we have not run
any of it. The reference notes it "is not yet complete"; a later build may
add or change entries. Only names and type signatures are recorded here,
as facts; the reference's own text is not copied, and the mods and the
reference are not in the repository.

## The headline: players and companies are a documented command API

The open question of whether TF3 lets a script make and switch players
(companies) -- and the worst case of having to build them natively -- is
settled. It does, through two documented commands (`api/cmd.html`):

```
api.cmd.makeGameAddPlayerCmd(name: string, color: Vec3f)
    : Command<GameAddPlayerCommandData>
api.cmd.makeEntitySetPlayerCmd(entity: Engine.Entity, player: Engine.Entity)
    : Command<EntitySetPlayerCommandData>
```

These are the TF3 equivalents of TPF2's `game.interface.addPlayer()` and
`game.interface.setPlayer()` (used by `tf2mod`'s companies mode), but now
**commands**: they go through the command queue, so the room can order them
like any other action, rather than the native, assert-bypassed
`setPlayer` binding TPF2MP patched (HOOKS.md, "UI patches for companies
mode"). `game.interface` does not appear in the reference at all.

A company is a first-class game mechanic, documented under
`content/game_mechanics/company/`: `CompaniesState` and `CompanyState`
(company entity, rank, consumed permits, pending prospections),
`company_util` with `externalGetCompanyState(entity)` and
`externalGetCompaniesState()`, ranks, permits and prospection. So a room
where players "each have their own company" rests on the game's own
system, not one we simulate.

**Consequence for the plan.** PLAN.md Part 3's "Companies: create, switch,
dissolve" has a documented, in-script path: create with
`makeGameAddPlayerCmd`, assign ownership with `makeEntitySetPlayerCmd`,
read state with the company util. None of it needs native player
construction. It is still the owner's to decide whether the project ships
companies mode, and it still needs measuring: whether `makeEntitySetPlayerCmd`
on a vehicle works (TPF2's crashed, vehicles.lua:1176) and whether the
stock UI shows other companies' entities as editable (companies mode's ten
UI patches on TPF2). Those go on the release-day list, not into a decision.

## The command surface: 61 factories

Every player or script action is a command from an `api.cmd.make*Cmd`
factory (`api/cmd.html`). The full list, with the argument types the
reference gives, is the "verbs" our capture and the server's rules are
designed against; it replaces guessing from TPF2's set. The `Command<...>`
result type is left off each line below.

**Players and companies**
```
makeGameAddPlayerCmd(name: string, color: Vec3f)
makeEntitySetPlayerCmd(entity: Engine.Entity, player: Engine.Entity)
```

**World building and terrain**
```
makeWorldBuildProposalCmd(proposal: Proposal, context: any,
    ignoreErrors: boolean, doDust: boolean, playerInitiated: boolean)
makeWorldReplaceTerrainCmd(map: GameMap, terrainConfig: BaseConfig.Terrain,
    seedText: string, worldEntity: Engine.Entity, keepAssets: boolean)
makeWorldSetBulldozableCmd(entity: Engine.Entity, bulldozable: boolean)
makeWorldChangeWindCmd(emissionGridEntity: Engine.Entity, wind: Vec2f)
```

**Vehicles**
```
makeVehicleBuyCmd(playerEntity: Engine.Entity, depotEntity: Engine.Entity,
    tvc: TransportVehicleConfig)
makeVehicleReplaceCmd(vehicleEntity: Engine.Entity, tvc: TransportVehicleConfig)
makeVehicleSellCmd(vehicleEntities: {Engine.Entity})
makeVehicleReverseCmd(vehicleEntity: Engine.Entity)
makeVehicleSendToDepotCmd(vehicleEntity: Engine.Entity, sellOnArrival: boolean,
    jumpToDepoEntity: Engine.Entity)
makeVehicleSetLineCmd(vehicleEntity: Engine.Entity, lineEntity: Engine.Entity,
    stopIndex: integer)
makeVehicleSetManualDepartureCmd(vehicleEntity: Engine.Entity, manual: boolean)
makeVehicleTryToDepartCmd(vehicleEntity: Engine.Entity)
makeVehicleSetStoppedByUserCmd(vehicleEntity: Engine.Entity, stopped: boolean)
makeVehicleSetModifiersCmd(vehicleEntity: Engine.Entity,
    modifiers: Engine.Component.TransportVehicle.Modifiers)
makeCustomVehicleCreateOrUpdateCmd(vehicleEntity: Engine.Entity, carrier: Carrier,
    vehicles: {{integer, boolean}}, movePathAircraft: ...|nil)
```

**Lines**
```
makeLineCreateCmd(name: string, color: Vec3f, player: Engine.Entity,
    line: Engine.Component.Line)
makeLineUpdateCmd(lineEntity: Engine.Entity, data: Engine.Component.Line)
makeLineDestroyCmd(lineEntity: Engine.Entity)
```

**Time and simulation (the server's to hold, PLAN Part 2)**
```
makeGameSetSpeedCmd(speedup: integer)
makeGameSetCalendarSpeedCmd(millisPerDay: integer)
makeGameSetDateCmd(date: Date)
makeGameSetTimeOfDayCmd(timeOfDaySec: integer)
makeGameSetCloudCoverageCmd(cloudCoverage: number)
makeGamePerformSimulationStepsCmd(amount: integer)
```

**Towns**
```
makeTownCreateCmd(towns: {TownInfo})
makeTownDestroyCmd(townEntity: Engine.Entity)
makeTownDevelopAtCmd(position: Vec2f, developStreets: boolean, upgradeBuildings: boolean)
makeTownSetDevelopmentActiveCmd(entity: Engine.Entity, developmentActive: boolean)
makeTownSetInitialLandUseCapacitiesCmd(entity: Engine.Entity, landUseCapacities: {integer})
makeTownUpdateSizeCmd(townEntity: Engine.Entity, sizeFactors: {number},
    updateAllBuildings: boolean)
makeTownUpdateCargoNeedsCmd(entity: Engine.Entity, cargoNeeds: {{CargoTypeId}},
    updateTownBuildings: boolean)
makeTownConnectWithIndustriesCmd(townEntities: {Engine.Entity},
    connections: {{integer, integer}}, keep: boolean)
makeTownCustomDistributionWeightsCmd(townEntitiy: Engine.Entity,
    levelToCapacityDistributionWeights: {{number}})
makeTownBuildingSetBlockedDevelopmentCmd(townBuildingEntity: Engine.Entity,
    blockedDevelopment: boolean)
```

**Industries and stocks**
```
makeIndustrySetManualDevelopmentCmd(industryEntity: Engine.Entity, manual: boolean)
makeIndustrySetDespawnTimeCmd(industryEntity: Engine.Entity, timeStamp: integer)
makeCreateIndustryExtendProposalCmd(extendConstruction: Engine.Entity,
    numTrySlots: integer, numKeepSlots: integer)
makeStockListSetModifiersCmd(industryEntity: Engine.Entity,
    modifiers: Engine.Component.StockList.Modifiers)
makeStockListSetStocksCargoTypeCmd(stockListEntity: Engine.Entity,
    stockIds: {StockId}, cargoType: CargoTypeId)
makeStockListDiscardCargoCmd(stockListEntity: Engine.Entity, stockIds: {StockId},
    remainingTimeToDelivery: number)
makeStockSetCargoAmountCmd(entity: Engine.Entity, stockId: StockId, amount: integer,
    cargoType: string)
```

**Money, journal and maintenance**
```
makeJournalBookAssetCmd(player: Engine.Entity, entry: JournalEntry, position: Vec3f)
makeJournalLogEntryCmd(entity: Engine.Entity, logNamesAndValues: {{string, integer, boolean}})
makeJournalClearAllCmd()
makeClearLogbooksCmd(...)
makeMaintenanceCostUpdateCmd(...)
```

**Naming, colour and emissions**
```
makeEntitySetNameCmd(entity: Engine.Entity, name: string, forceSameEntity: boolean)
makeEntitySetColorCmd(entity: Engine.Entity, color: Vec3f)
makeEntitySetEmissionsCmd(entity: Engine.Entity, noisePower: number,
    pollutionPower: number, radius: number, pollutionRadius: number)
makeComponentExchangeCmd(entity: Engine.Entity, component: Engine.Component)
```

**Scripting, custom entities, animals and people**
```
makeScriptingSendEventCmd(src: string, id: string, name: string, param: any)
makeCustomEntityCreateCmd(modelId: integer)
makeCustomEntityDestroyCmd(entity: Engine.Entity)
makeCustomEntityUpdateStateCmd(entity: Engine.Entity, customState: ComponentCustomState)
makeCustomEntityUpdateTransformationCmd(entity: Engine.Entity, transf: Mat4f)
makeSimPersonSetStateCmd(entity: Engine.Entity, simPersonState: integer)
makeAnimalSpawnAtCmd(fileName: string, position: Vec2f, lookAt: Vec2f)
makeAnimalSetStateCmd(animalEntity: Engine.Entity, movementType: integer,
    targetChangedElapsed: number, invalidTileElapsed: number,
    movementSpeed: number, angularSpeed: number)
```

## What it confirms, corrects and adds

- **`makeWorldBuildProposalCmd` has a fifth argument, `playerInitiated`**
  (`proposal, context, ignoreErrors, doDust, playerInitiated`). The Mod
  Hub tools call it `(proposal, nil, true, true)`
  (TF3_MODHUB_SCRIPT_MODS_2026-09-29.md), so the fifth defaults or is
  omitted. It may be the flag that tells a player's build from a script's
  replay -- the distinction the capture needs (HOOKS.md, "The command
  pipeline"). Measure it: whether a value there marks the build, and
  whether the game reads it.
- **`makeScriptingSendEventCmd(src, id, name, param)`**: the four
  arguments are named. Mods pass `src = ""` and use `id` as the channel
  (TF3_MODHUB_SCRIPT_MODS_2026-09-29.md).
- **`makeGameSetSpeedCmd(speedup: integer)`** is a command, so the room's
  pace is set through the queue, not only through the game-bar recipe
  (TF3_MODS_2026-09-27.md item 4). `makeGamePerformSimulationStepsCmd(amount)`
  advances the simulation by a count -- a lockstep-shaped primitive to
  understand before relying on it.
- **`makeLineCreateCmd(name, color, player, line)`** confirms the
  signature `engine.lua` was written against, and that a line names its
  player.
- **`makeVehicleBuyCmd`, `makeJournalBookAssetCmd`** take a
  `playerEntity` / `player`: ownership and money are per player entity, as
  the mods showed.
- **Line component**: `makeLineUpdateCmd` takes an
  `Engine.Component.Line`, and Timetables sets its `reservationPriority`
  (line priority, PLAN Part 3) through it
  (TF3_MODHUB_SCRIPT_MODS_2026-09-29.md).

## Cross-check against our mod

`mod/tpf3mp_1/.../engine.lua` reads the world to capture a build. Its
names against the reference:

- **Confirmed by the reference:** `makeLineCreateCmd`,
  `makeWorldBuildProposalCmd`, `makeEntitySetNameCmd`,
  `makeGameSetSpeedCmd`, and the per-player line, the `Proposal`.
- **To reconcile:** `engine.lua` reads the street graph with
  `api.engine.system.streetSystem.getNode2StreetEdgeMap`. The reference's
  `api/engine/system.html` is the place to confirm that name or find the
  documented one (mods use `getNodeSegments`,
  TF3_MODHUB_SCRIPT_MODS_2026-09-29.md). This is a release-day task, since
  nothing has run.

## Still to do on release day

- Read `api/engine/system.html` and `api/type.html` for the street graph,
  `Proposal`, `SimpleProposal`, `BASE_EDGE`/`BASE_NODE` and the component
  fields, and reconcile `engine.lua` with the documented names.
- Measure the companies path: `makeGameAddPlayerCmd`,
  `makeEntitySetPlayerCmd` on stations, lines and a vehicle, and whether
  the stock UI gates other companies.
- Measure `makeWorldBuildProposalCmd`'s `playerInitiated`: whether it
  distinguishes a player's build from a script's, and so whether the
  capture can tell them apart in script (TF3_MODS_2026-09-27.md item 3).
- Run the API dump probe (`tools/probe/tf3`) and diff it against this
  reference, to find what the reference leaves out ("not yet complete")
  and what a build changed.

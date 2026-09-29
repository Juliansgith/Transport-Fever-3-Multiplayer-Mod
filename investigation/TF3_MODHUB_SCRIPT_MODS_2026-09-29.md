# What Mod Hub's script mods tell us -- 2026-09-29 (release day)

On release day, Transport Fever 3's Mod Hub opened on mod.io (game 10640,
`transportfever3`). Every mod tagged **Script Mod** there, 37 of them, was
downloaded with `tools/modio/fetch.py --tag "Script Mod"` and its scripts
read: 332 `.lua` and `.tl` files by 15 authors, several from the Curated
Mods Program (Timetables, the "Fever" mods, Canadian Names). This page
says what they show and what it changes for us. It follows
[TF3_MODS_2026-09-27.md](TF3_MODS_2026-09-27.md), which read nine
pre-release mods; where this one confirms, corrects or adds to that page,
it says so.

Every finding is labelled **REPORTED** ([DAY_ONE.md](../docs/DAY_ONE.md)):
working mods rely on it, but we have not run it ourselves. "Seen in N mods"
counts mods, not call sites. The mods are their authors' work and are not
in the repository; paths below are inside each mod's archive, to find the
place again, not to copy from.

## What changes for us

| our assumption | what the mods show | where it matters |
|---|---|---|
| Game scripts may be gone: no pre-release mod used one (TF3_MODS_2026-09-27.md; DAY_ONE.md §3) | **They exist**, in a new form: a `*.gs.lua` file anywhere in a mod, found by the game without being listed. Seen in 5 mods. | the probes, and every mod that keeps state in the engine |
| Commands come from one place: a player's click, sent from the GUI | **They also come from game scripts**, run in the engine on every machine. Timetables holds and releases vehicles from its game script's `update()` and `handleEvent()`. | the capture: see "Two kinds of command" |
| Build tools are the game's own | Four mods build and edit roads and track through `makeWorldBuildProposalCmd` (Move It, Overpass Builder, Planning Fever, Alert Fever), with proposals naming existing nodes and edges **by entity id**. | the capture, and D8's rule that actions carry positions, not ids |
| `io`, `os`, `load` and `require` are unknown | `os.clock`, `os.time` and `os.date` are used (3 mods), and `require` (5 mods, including `pcall(require, ...)`). No mod uses `io`, `load` or `dofile`. | the probes' output, DAY_ONE.md §3 |
| A mod declares whether it is cosmetic | `mod.json`'s `"cosmetic": true` is the author's word only: Timetables, Planning Fever and Alert Fever set it, and all three change the world. | the room's rule for which mods may differ |
| Mods are laid out as `mod.json`, `_content.json`, `_metadata/`, `content/` | `_content.json` is in 1 mod of 37 and `_metadata/modinfo.json` in 2: mod.io holds a Mod Hub mod's name and description. `content/` is usual but not required. | the install scripts, `installed_mod` in the launcher |
| The names in `mod/tpf3mp_1/.../engine.lua` are TPF2's, unconfirmed | Most are confirmed: `BASE_NODE.position`; `BASE_EDGE.node0/node1/position0/position1/tangent0/tangent1/type/typeIndex/objects/laneConfigs`; a proposal segment's `.comp`, `.type` (0 street, 1 track) and `.streetEdge`; `bridgeTypeRep` and `tunnelTypeRep`. `streetSystem.getNode2StreetEdgeMap`, which `engine.lua` walks, is not seen; mods use `streetSystem.getNodeSegments`. | `engine.lua` |

## The mods

| mod.io id | mod | by | what it does | changes the world |
|---|---|---|---|---|
| 6037864 | Timetables | IncredibleTrains | line timetables, departure boards, line priority | yes: game script and GUI |
| 6050946 | Move It | IncredibleTrains | edit existing streets and track | yes: proposals |
| 6050917 | Overpass Builder | IncredibleTrains | turn a junction into an overpass | yes: proposals |
| 6330121 | Planning Fever | FilippoL | copy and paste; a spline track builder | yes: proposals |
| 5832874 | Research Fever | FilippoL | a research tree that unlocks vehicles | yes: game script, money, resources |
| 6330112 | Alert Fever | FilippoL | dated reminders with effects (sell, remove, stop) | yes: game script |
| 6407255 | Balance Fever | FilippoL | ship crews, truck compartments | content (`loadModel` modifier) |
| 6373930 | Improved Destination Displays | GameBurrow | renames vehicles to show destinations | yes: game script renames |
| 6402284 | Fix stop names | GameBurrow | renames "Stop #n" to street names | yes: game script renames |
| 6407888 | Leasing | YoshiCH | a cheap, costly-to-run copy of every vehicle | content |
| 6388216 | Passenger Number Modifier | Seamon | capacities and load speed | content |
| 6408407 | Multiple Unit Variants | YoshiCH | new multiple-unit variants | content |
| 6407946 | Vehicle Capacity Multiplier | Maik_Chaos | capacity per carrier (12 settings) | content |
| 6401202 | Compartments for Vanilla vehicle | Maik_Chaos | capacity, availability, hiding (9 settings) | content |
| 6409619 | All Available from 1900 | Maik_Chaos | availability of vehicles, streets, tracks, modules, signals | content |
| 5722648 | Vehicle Filter (Land) | Maik_Chaos | hides vehicles by region or era (14 settings) | content |
| 6300243 | Realistic Train Brakes | Maik_Chaos | braking by era | content |
| 5933289 | Realistic Smoke | Maik_Chaos | steam engines' smoke | rendering, rewrites models |
| 5732944 | Cargo - Baseset | Maik_Chaos | container formats | content |
| 6008076, 6204977, 6363679 | CRH depot, Shopping Mall, Seaplane hub | hugedragonyk | constructions and industries | content, with random numbers |
| 6409957 | Invisible bridge | EISFEUER472 | a bridge without pillars | content |
| 6351655 | Stronger Terrain Brushes | Lo2k | terrain tool limits | the base config |
| 5822744 | Aircraft Toolbox | DH-106 | aircraft animation library | rendering |
| 6171576 | Random Number Creator Util | themeatballhero | a helper for model authors | model load |
| 6386991 | Minimap | shbronks | a minimap | no: GUI only |
| 5860216, 6372609 | Canadian Names, Belgian Names | Andrew_Spearin, Lohrkan | town and street names | names |
| 6348932, 6400567, 6409135, 6366932, 6377032, 6377036, 6377039, 6405304 | line colours, line name templates, coloured labels, dark overlays, four UI themes | Lo2k, APasz | the interface's colours and names | no: one player's view |

## Game scripts

REPORTED, from Research Fever, Alert Fever, Timetables, Improved
Destination Displays and Fix stop names.

- **Declaration.** A `*.gs.lua` file returns `data()` with any of
  `updateScript`, `postUpdateScript`, `handleEventScript`,
  `guiUpdateScript` and `guiHandleEventScript`, each
  `{ fileName = "<script>@<function>" }`, the script's path relative to the
  `.gs.lua` file. Nothing lists it: Fix stop names has no run scripts and
  no `_content.json`, and its game script runs.
- **Signatures.**
  - `update(userParams, state, dt)`, every step;
  - `postUpdate(userParams, state, dt, updateResult)`, given what `update`
    returned;
  - `handleEvent(userParams, state, src, id, name, param)`;
  - `guiUpdate(userParams, stateReadOnly, guiState)`.
- **State.** `state:get()` and `state:set(t)`, a Lua table kept in the
  script's `GAME_SCRIPT` component, apparently saved with the world (no
  mod saves it itself). `state:subscribeToEvent(name)` must be called for
  each event name `handleEvent` is to receive; mods call it from `update`.
  The GUI reads a script's state with
  `api.engine.getComponent(api.engine.system.gameScriptSystem.getEntityForGameScript("<modId>::/path/x.gs"), api.type.ComponentType.GAME_SCRIPT).state`.
- **From the GUI to a game script:**
  `api.cmd.sendCommand(api.cmd.makeScriptingSendEventCmd("", id, name, param))`.
  Every mod passes `""` first; `id` is a channel ("TimetablesEdit",
  "ResearchTree", "RemindMe", "DestinationDisplays", "Notifications",
  "VehicleModifier"), `name` the event, `param` a table. Seen in 4 mods.
  It is a command, so it goes through the command queue.
- **From the engine to a game script:** events the game raises, received
  the same way: `TransportVehicleSystem` / `OnArriveAtStop` (`vehicleEntity`,
  `lineEntity`, `stopIndex`), `TransportVehicleSystem` /
  `OnCargoUnloaded`, `StockListSystem` / `ItemsAboutToBeConsumed`.
- **The game's own game scripts** exist too: Alert Fever reads the state
  of `"::/game_mechanics/subventions/subventions.gs"`, and Research Fever
  sends to a built-in "Notifications" channel.

## Commands seen

REPORTED. `api.cmd.sendCommand(cmd, function(result, success) ... end)`,
the callback optional. Several GUI tools do not call it themselves but
commit through the stock GUI's `engine_react_util.useStepState` and
`useStepStateTimerWithCommit`, which take a function that makes the
command.

| factory | arguments as called | sent from | mods |
|---|---|---|---|
| `makeWorldBuildProposalCmd` | `(proposal, context or nil, ignoreErrors, bool)`; `true, true` from Overpass Builder and Move It in anarchy mode, `false, false` from Planning Fever, `true, false, false` from Alert Fever | GUI | 4 |
| `makeScriptingSendEventCmd` | `("", id, name, param)` | GUI and game scripts | 4 |
| `makeEntitySetNameCmd` | `(entity, name)`, and `(entity, name, true)` to keep the same entity | game scripts | 2 |
| `makeVehicleSetManualDepartureCmd` | `(vehicle, bool)` | game script | 1 |
| `makeVehicleTryToDepartCmd` | `(vehicle)` | game script | 1 |
| `makeVehicleSetStoppedByUserCmd` | `(vehicle, bool)` | game script | 1 |
| `makeVehicleSellCmd` | `({vehicle, ...})` | game script | 1 |
| `makeLineUpdateCmd` | `(line, api.type.Line.new(lineComponent))` with `reservationPriority` 1.0, 2.0 or 3.0 | GUI | 1 |
| `makeLineDestroyCmd` | `(line)` | game script | 1 |
| `makeJournalBookAssetCmd` | `(player, JournalEntry{amount, time = -1, category.type = OTHER}, Vec3f)`: money | game script | 1 |
| `makeCustomEntityUpdateStateCmd` | `(entity, table)`, every step | game script | 1 |

The result of a build is a `WorldBuildProposalCommandData`:
`result.proposal.proposal.addedNodes[i].entity` and `.addedSegments[i].entity`
name what the build created. Overpass Builder sends a second proposal from
the first one's callback, built from those new entities.

## Proposals

REPORTED, from Move It, Overpass Builder and Planning Fever.

- `api.type.SimpleProposal.new()`, with `streetProposal.nodesToAdd`,
  `nodesToRemove`, `edgesToAdd`, `edgesToRemove`, `nodeConfigsToAdd`,
  `nodeConfigsToRemove`, `edgeObjectsToAdd`, and `constructionsToRemove`
  on the proposal. Planning Fever's copy and paste builds a full
  `api.type.Proposal.new()` instead (`proposal.proposal.addedSegments`,
  `removedSegments`, `addedNodes`, `removedNodes`, `nodeConfigsToAdd`,
  `nodeConfigsToRemove`).
- Nodes: `api.type.NodeAndEntity.new()`, `.entity` a new negative id
  (from -1 down; Planning Fever starts at -1000), `.comp` a `BASE_NODE`
  with `.position` set. Edges: `api.type.SegmentAndEntity.new()`,
  `.comp` a `BASE_EDGE` (`node0`, `node1`, `position0`, `position1`,
  `tangent0`, `tangent1`, `type` as `api.type.enum.BaseEdgeType.NORMAL` or
  `BRIDGE`, `typeIndex`, `objects`, `edgeDecorations`, `laneConfigs`),
  `.type` 0 for a street with `.streetEdge`, 1 for track. Node lane
  configurations: `api.type.BaseNodeLaneConnectionAndEntity.new()`.
- **An existing node or edge is named by its entity id**, removed, and
  added again under a new negative id; re-adding under the same id fails
  (a comment in Overpass Builder).
- Signals go in as `api.type.SimpleStreetProposal.EdgeObject.new()` with
  `edgeEntity`, `param`, `oneWay`, `left`, `model` and
  `playerEntity = api.engine.util.getPlayer()`.
- Helpers: `api.engine.util.proposal.makeProposalData(proposal)` checks a
  proposal (its `collisionInfo`) without building it;
  `api.engine.util.proposal.createProposalRemove(entity, api.type.Context.new())`
  makes a removal.

## The mod format, beyond the 2026-09-27 page

- **`mod.json`**, present in all 37 and sometimes with a UTF-8 byte order
  mark: `modId` is any name (`celmi_timetables`, no `_1` needed);
  `revision`; `cosmetic` and `visible`; `severityAdd` and
  `severityRemove` (`"None"`, `"Warning"`, `"Critical"`); `dependencies`
  and `incompatibilities`; `params`, an array of `{key, name, tooltip,
  uiType = "Button" | "ComboBox" | "Slider", values, defaultIndex}`. A few
  also carry `name`, `authors` and `url`.
- **Run scripts** name a file as `"<modId>::/mod.script@fn"` or
  `"<modId>::mod.script@fn"`, found in `content/` or at the mod's root
  (Realistic Smoke has no `content/`). Their arguments differ between
  mods: `postRunFn(captureParams, configDict, allModParams)` in most,
  `preRunFn(captureParams, configDict, allModParams, baseConfig)` in
  Stronger Terrain Brushes, which changes `baseConfig.terrainToolMaxSize`
  and the tool's strengths.
- **Settings.** At load, `allModParams[getCurrentModId()][key]`, a
  1-based index into `values` (`defaultIndex` is 0-based); at run time,
  `api.engine.config.getModParams()[modId]`. Dark Overlays' readme says the
  values are stored with the save.
- **Changing content at load:** `api.res.modelRep.find/get/getAsTable/setAsTable/addAsTable/setVisible/getAll/forEachModelWithMetadata`,
  the same on `multipleUnitRep` and `streetTemplateRep`,
  `moduleRep.get` changed in place, `genericRep.add`, and TPF2's
  `addModifier("loadModel" | "loadConstruction" | "loadEconomy", fn)`.
  `modelRep.find` returns -1 for a missing model, which Lua treats as
  true.
- **The GUI.** Extension points beyond the 2026-09-27 page:
  `MainModButtonAreaExtension` (4 mods), `LineEowExtensionPoint`,
  `StationGroupEowExtensionPoint`, `RadialMenuExtension`,
  `ModEntryPointExtension`. `react-replacement-config` replaces stock
  recipes: Timetables replaces `"LineManagerPanel"`, Improved Destination
  Displays `line_react_util.LineCargoDisplay`. `gui_res_overwrite`
  resources re-colour the interface; `.css.lua` style sheets restyle it.
  GUI timers: `react.onStep`, `react.onStepTimer(fn, seconds, bool)`,
  `react.enqueueJoin(fn, key)`, `engine_react_util.useStepStateTimer`.
- **Tools.** A `construction_tool` resource with `getDefinition`
  returning `action = "ACTION_CUSTOM"` and a recipe: how Move It, Overpass
  Builder and Planning Fever add their tools to the build menu.

## What it means for multiplayer

These are consequences for the plan; none changes a decision. The ones
the owner must settle are marked *open*.

1. **Two kinds of command.** A command sent from the GUI is one player's
   action: the room orders it and every game applies it (our capture).
   A command sent from a game script is a consequence the game script
   computes, and every game in the room runs the same game script on the
   same world. If the hook captured those and sent them to the room, the
   room would apply them once per player: Timetables would release each
   held vehicle two, three or four times. They must run on each game as
   the game script sends them, and never be forwarded. So the capture
   has to know where a command came from. On TF3 the GUI and game scripts
   are separate Lua states, so a capture made in the GUI state (wrapping
   `api.cmd.sendCommand` there, TF3_MODS_2026-09-27.md item 3) sees only
   the GUI's commands. A native capture at the command queue sees both
   and must tell them apart. Measure which states `sendCommand` is called
   from, and whether a game script's command is queued like a player's
   (BUILDING.md, "Measure these first").
2. **Game scripts must be deterministic, and not all are.** A game
   script's `update` runs on every game, so everything it reads must be
   the same on every game. Three of the five break that:
   - Fix stop names and Improved Destination Displays keep their timer in
     a Lua local that restarts at 0 whenever the world is loaded, so a
     game that joined later renames on different steps. Both also act only
     for `api.engine.util.getPlayer()`, the player at that machine.
   - Fix stop names picks names with unseeded `math.random`, from a list
     chosen by the player's language (`api.util.getLanguage()`).
   - Research Fever books its money to `getPlayer()` and uses unseeded
     `math.random` for its subsidies.
   A game whose game script asks for the local player gets a different
   answer on each machine, and TPF2's `getPlayer` gave the local player
   too. *Open:* whether a room refuses mods whose game scripts read the
   local player, the clock or `math.random`, and how it would know. A list
   by mod.io id, kept with the server, is the simplest answer.
3. **Construction scripts and name scripts roll dice.** The CRH depot,
   Shopping Mall and Seaplane hub call `math.random` in their
   construction's `updateFn`; in the mall, an "Efficiency: Random" option
   sets the industry's capacity from it (`capacity = 45.625 * eff`). The
   name scripts pick town and street names with `math.random`. Whether
   the game seeds `math.random` before calling such a script, so that
   every game draws the same numbers, is not shown; measure it with two
   games building the same construction. TPF2MP met the same with
   construction seeds (TF3_MODS_2026-09-27.md item 8).
4. **Build tools that name entities by id.** Move It, Overpass Builder
   and Planning Fever send proposals naming existing nodes and edges by
   entity id. Entity ids differ between machines that agree on the world
   (HOOKS.md), which is why D8's actions carry positions. The capture
   turns a proposal into an action (`engine.fromProposal`,
   `roads.lua`), by position, and fails closed on what it cannot read.
   These tools also put node lane configurations, edge objects (signals,
   with the sender's player in them), bridge types and construction
   removals into their proposals, which the capture does not read yet: a
   build it cannot read is refused in a room, not sent unchecked (PLAN.md,
   Part 3). Planning Fever and Move It walk entity-keyed tables with
   `pairs` to order a proposal's entries; that is harmless while only the
   sender builds the proposal, and one more reason not to rebuild one on
   each game.
5. **Content every player must share.** At least fifteen of the 37
   change content at load (vehicles, capacities, availability years, brakes, streets,
   constructions, cargo), and at least six take settings that change it (Vehicle
   Filter 14, Compartments 9, Capacity Multiplier 12, All Available 10,
   Train Brakes 1, and Balance Fever 2). A room must pin each mod's exact
   version and each setting. mod.io gives both: one modfile id serves
   Windows, macOS and Linux (consoles get their own), and settings are
   `allModParams`. Vehicle Filter also skips vehicles a player's DLC lacks,
   so owned DLC is part of the content too. *Open:* the room's mod list
   (PLAN.md, "The room's required mods from Mod Hub IDs") can use the
   modfile id as the version.
6. **The "cosmetic" flag cannot decide what may differ.** Mods declare it
   themselves, and three world-changing mods declare it. A room can let
   players differ only in mods known to touch nothing but their own view:
   the eight colour, name-template and theme mods and the Minimap here.
7. **Mods that act only in one player's GUI** remain the open question of
   TF3_MODS_2026-09-27.md item 6, now with more members: Planning Fever
   keeps plans with `setGuiSaveData` on the machine that saved them, and
   its builds are ordinary player actions once captured.

## Answered since 2026-09-27

- Game scripts exist (DAY_ONE.md §3), as `*.gs.lua` with the functions
  above.
- `os.clock`, `os.time`, `os.date` and `require` are available to mods.
- `getGuiSaveData` and `setGuiSaveData` are in use (3 mods), per player.
- The line's `reservationPriority` is set through `makeLineUpdateCmd`.
- `api.engine.config.getModParams()`, `api.util.getLanguage()`,
  `api.util.getAppConfig().debugMode`, `api.engine.util.getYear()` and
  `getCalendarDate(gameTime)`; `GAME_TIME.gameTime` is in milliseconds.
- Mod settings reach the game as 1-based indices.

## Still unknown after this

- Whether a command a game script sends is queued like a player's, and
  whether the GUI state's `api.cmd.sendCommand` can be wrapped.
- Whether `math.random` is seeded before construction and name scripts.
- Whether the stock road and track builders build in script. The mods'
  tools do, and they reach the queue through `makeWorldBuildProposalCmd`.
- `streetSystem.getNode2StreetEdgeMap`, which our `engine.lua` walks: not
  seen; `getNodeSegments` is.
- Which Lua the game runs. The game's `.d.tl` declarations still say more
  than any mod: read them first (DAY_ONE.md §3).

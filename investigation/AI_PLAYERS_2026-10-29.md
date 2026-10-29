# AI players: what the game offers, what was measured, what was weighed

The owner's ask, 2026-10-29: "investigate the possibility of a full
opponent AI that can do everything like intelligently make train station,
optimise routes etc, bus train truck ship etc. investigate deeply and make a
plan for 3 different difficulties that can be added as ai players during
lobby". The plan is [docs/AI_PLAYERS.md](../docs/AI_PLAYERS.md), the
decision D32 in [docs/DECISIONS.md](../docs/DECISIONS.md). This page is the
evidence behind them.

## The short answer

It can be done without new engine work. Transport Fever 3's own Lua API
already lets a program read everything a player sees, ask the game whether
a build would succeed and what it would cost without building it, find
paths on the network and curves for new track, and read every line's
load, problems and throughput. The mod already turns every player action
into the game's own proposal for any company (`apply.proposalOf`, written
for build previews) and already asks the game for its verdict before every
build (`apply.lua`, `refusal`). What an AI adds is the part that decides
what to build, and the plumbing to let a computer, not a person, send a
company's actions through the room.

Transport Fever 3 has no AI companies of its own: the store and reviews
list single player, campaign and sandbox, and players have asked for AI
rivals since Transport Fever 1 (Steam forum threads of 2018 and 2023).

## What the game's API offers

From the game's own declarations (`api/tealdef/api/engine.d.tl`,
`engine/util.d.tl`, `type.d.tl` in the install, build 40425) and the
2026-09-29 dump of the GUI state
(`investigation/dayone-2026-09-29/probe/script_api_dump_gui.txt`):

| need | the game's function |
|---|---|
| the map at once | `api.engine.terrain.makeMapFromGame(heightmap, towns, industries, assets)`: towns (position, size, cargo needs per district), industries (position, type, on water), town links, size, water level |
| terrain | `terrain.getHeightAt`, `getBaseHeightAt`, `isOnWater`, `getBoundingBox`, `getBaseResolution`, `getTerrainEntityRevisions` (what changed since) |
| the economy's graph | `util.industry.getEconomyCargoDependencies(id)`: for each cargo, the industries and their weights |
| supply and demand | `util.stock.getCargoProducedPerYear`, `getCargoMaxProductionPerYear`, `getCargoShippedPerYear`, `getInputsOutputsFromRules`, `getProductionRating`; `util.town.getTownStockCargo`, `getTownDeliveriesStats`, `getTownReachability`, `getTownDistrictCenter`, `getTownCapacityUsage` |
| cargo quality and time | `util.cargo.getCargoQualityDataForLine`, `...AtStation`, `...ForStockList`; `util.stock.getAdjustedDeliveryTime` (TF3 pays for delivery time and spoils cargo) |
| lines | `util.line.getLineCapacityUsages`, `calcLineStationThroughput`, `getMaxFrequency`, `getLinesIssues`, `getLineProblems`, `getDetailedLineProblems`, `getFailedPathReason`, `getNoRoadConnectionProblems` |
| vehicles | `util.vehicle.findBestDepotForLine`, `findBestLineAndDepotForVehicle`, `getRunningCost`, `getDepreciatedValue`, `getVehicleMaintenanceState`, `getVehicleProblems` |
| stations | `util.station.calculateStationUsage`, `calculateStationTerminalUsage`, `findStationGroupWithOverlappingCatchmentOfDifferentCarriers`; `system.catchmentAreaSystem` |
| paths | `util.pathfinding.findPath`, `findPathNodeToNode` (on the network, by transport modes), `findDubinsPath(p0, t0, p1, t1, radius)` (the shortest curve between two directed points) |
| a build's verdict | `util.proposal.makeProposalData(proposal, context)`: `errorState.critical`, messages, warnings, `costs`, `collisionInfo` (what it would hit or remove), terrain cut and fill |
| constructions | `util.construction.getConstructionResult(file, template, params)`, `getGlobalConstructionParams`; `util.proposal.checkProposalConnectsNodes` |
| money | `util.finance.getPlayersBalance`, `calculateEarnings`, `computeFinanceTable`, `calcIncomeSince` |
| who is near what | `util.octree.findEntitiesInCircle`, `findTransportNetworkNodesInCircle` |

Not available by sweeping component type: `LINE`, `BASE_EDGE`,
`BASE_NODE`, `PLAYER` and `STOCK_LIST` ("Cannot loop over this component
type"); they come through their systems (`lineSystem.getLines`,
`streetSystem`, the construction's stock list), as the mod already reads
them.

## What was measured

One game, build 40425, the rig's big fixture (`bubujujuoct10`: 20.5 by
20.5 km, 1922, the fixture company with 25 lines and 66 vehicles), the
console in the GUI's Lua state, read only, 2026-10-29. Scripts in the
session's scratchpad (`aiprobe/`).

| read | result |
|---|---|
| `makeMapFromGame` without the heightmap | 2 ms: 55 towns, 94 industries, 50 town links |
| terrain | 4 m base resolution, 1 m high; heights −100 to 303 m |
| `getHeightAt` | 7 µs a sample (40,000 in 280 ms): the map at 32 m is 409,600 samples, 2.9 s, so it is read over many frames, once, then only where `getTerrainEntityRevisions` says it changed |
| `isOnWater` | 6.7 µs a sample; 27.5 % of this map is water |
| every construction (3,480) | 1.9 ms |
| every town building (3,326) | 11.9 ms |
| stations, station groups, vehicles | 174, 172, 66: under 0.1 ms each |
| `getEconomyCargoDependencies` | 231 ms, 23 cargos (for example cement: quarry 0.21, cement plant 0.07): read once a game |
| `getLineCapacityUsages` for all 25 lines, `getLinesIssues` | under 0.1 ms |
| `findDubinsPath` | 3 pieces, under 0.01 ms |
| `findPathNodeToNode` between two towns 20.9 km apart | 0 edges (no road joins them yet), under 0.1 ms |

And from the mod's own records: the game prepares a long track's
proposal in 15 to 95 ms (`apply.lua`, 2026-10-19), and the verdict
(`makeProposalData`) runs before every build the room applies.

What this means for an AI: reading the world costs little except the
terrain and the cargo graph, which are read once; the expensive call is
the build verdict, which must be asked for short pieces, a few a second
at most, or a driver's frame stutters.

**Probe slip.** The first probe was typed while the console had already
been toggled closed by the fixture load (GAME_TESTING.md, "A loaded world
takes the keyboard from the console"), so its characters reached the game
as keys and opened the company window. Nothing was built; the game was
the session's own, alone in a throwaway room. The console was opened
again and the probes repeated.

## Prior art

- **AI Builder for Transport Fever 2** (Steam Workshop 2820656841, by
  okeating, 2022 to 2024). A script mod that builds complete routes
  between industries and towns, by road and rail, from the game's own
  API. Its discussion threads show what goes wrong: Lua errors while
  building rail between cities, bus lines between towns that fail where
  road routes work, industries not found in some maps. No source licence
  was found, so nothing of it may be copied (AGENTS.md); it shows that the
  game's proposals can carry a program's builds, and that a builder needs
  to test what it builds.
- **OpenTTD's script AIs** (NoAI): libraries for road and rail
  pathfinding over tiles, with costs a script sets (slopes, bridges,
  turns), and AIs built on them (SimpleAI: point-to-point routes with a
  better pathfinder; others with networks). Their lessons: a pathfinder
  must bound its search (bridges tried everywhere make its time explode),
  and a good-enough route found quickly beats a perfect one.
- **TPF2 Multiplayer's server game** (ARCHITECTURE.md, Non-goals): a game
  can run on a server under Proton without a GPU, as a member of the room.
  It needs a licence per server and a logged-in Steam client. An AI could
  one day be driven from such a game; not now.

## Where the AI's mind could live

| where | for | against |
|---|---|---|
| **In a member's agent, reading through its game (chosen)** | Rust (D1); thinks in its own process, so no frame waits for it; testable without the game; the game only answers reads and verdicts | the AI needs a member's game; its reads and verdicts cost that game a little, budgeted |
| In every game, deterministically (a game script) | no driver, no network | needs identical results on every platform, which D2 rules out; every player's simulation would pay for the thinking |
| In the mod's Lua, in one member's game | direct access | thinking on the game's own threads stutters it; Lua is slow for search; hard to test |
| On the server, with a game of its own | always on | a non-goal (ARCHITECTURE.md): a licence and Steam login per server, heavy |
| On the server, fed world data by members | always on | the server cannot read the world itself; trusting members' world data; large transfers |

None of these makes a player's game the room's truth (D2, D4): the AI's
actions are ordered by the server like anyone's, and every game applies
them. The driver is where the AI looks at the world, as a player's screen
is.

## Open, to measure in the first milestone

- The cost of the HUD state's verdicts for the pieces the planners make
  (a 200 m road, a 500 m track, a station), and the frame time they cost
  the driver, against the 1.5 ms a frame budget.
- Whether a construction's verdict (stations, depots, harbours, airports)
  needs the construction's own parameters the tools set, and which of
  them the AI can choose from the template's defaults.
- The game's limits a route must meet (track and street gradients, curve
  radii, station lengths), read from the templates and checked against
  verdicts.
- How the game pays for cargo and passengers by distance and time, learnt
  from the lines' own income (the finance tables per line) rather than
  copied from the game's scripts, so a patch that changes the formula
  changes the AI's estimates with it.

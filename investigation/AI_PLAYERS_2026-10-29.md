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

## The first AI games (2026-10-29)

Seven two-game rig rooms on the big fixture, build 40425, the local
server (`target/game-runs/aigame1` to `aigame7`). What each run found, and what changed for the next:

| run | found | changed |
|---|---|---|
| 1 | The AIs founded their companies but never borrowed: the GUI's copy of the roster has no loan offers but the player's. | The AI reads the game script's roster. |
| 2 | Stops were built, in both games, but the AI never saw them: a street stop's station is an `EDGE_OBJECT` of its edge, in no construction. | Its place is the edge object's transform. |
| 3 | Depots were built, but the road the AI then laid onto the depot's entrance was refused by the verdict, "Construction Not Possible": the entrance is the construction's own frozen node. | The depot carries the construction tool's connection: the street split through a new junction, the entrance 2 m short of it; every game's refresh of the depot snaps it on. |
| 4 | The depot snapped alike in both games (`+e-2:203666>-1 -e205109`), but the AI read it as off the street: a road depot's frozen nodes hold only its apron's inner end (read from the console: six depots, all alike). A depot split at a junction a company's road meets passed the verdict, and both games refused it at apply time ("the junction edge belongs to Company"): the room's check of junction settings runs only when it applies (D21), never in a preview. | The entrance is where the depot's own edge meets one not its own; streets report a company's junction at their ends, and the AI splits none of them. |
| 5 | A Hard AI opened a line and ran three buses on it, the same buses (room ids 76 to 78) on the same line in both games. After the second game joined, the world was loaded again and the AI's half-built line was lost with its driver's mind. | The next driver takes the AI's stops on no line up again; a town counts as served only where a line stops. |
| 6 | Hard, Normal and Easy together: seven lines in seven minutes (Hard four, Normal two, Easy one), no action refused in either game, no divergence. The company read cost the driver up to 71 ms (below). | The company read asks the game's index for the AI's own depots and stations. |
| 7 | The same three: the second game drove them, the first to play again after the joiner's reload. The company read now costs about 1 ms. | |

**What the answers cost the driving game** (runs 6 and 7, the mod's own
log, `ai: answers in 60 s`), against the 1.5 ms a frame the driver may
spend:

| question | mean | slowest |
|---|---|---|
| company, run 6 | 22 to 30 ms | 71 ms |
| company, run 7 (the game's index) | 0.9 to 1.3 ms | 4 ms |
| verdict | 21 to 39 ms | 80 ms |
| towns | 7 ms | 8 ms |
| streets, models | 2 to 3 ms | 8 ms |

A question is answered whole in one frame, so each verdict and each
company read was a frame that long. The company read went through every
construction in the world (3,470: 3 to 18 ms, timed in the console) to
find the AI's few depots and stations; it now asks the game's index
(`streetConnectorSystem`) for its own, about 1 ms (run 7). The verdict is the game's
`makeProposalData`, which cannot be cut into pieces: about 25 ms for a
stop or a depot. It runs in the GUI's state, so it holds a rendered
frame; in the game script's state it would hold a simulation step
instead, which the room's pace leaves room for, once the mod's dry run is
shown to leave nothing behind that a later applied action could read.

**What a verdict spends** (2026-10-30, runs 15 to 17, the slowest
verdict's parts in the cost log). Most of it was not the game's verdict:
the proposal read the whole street network (4,985 nodes, 26 to 71 ms)
nearly every time, because the octree missed one of the edge's ends and
the dry lookup fell back to the whole read. The game's own verdict took
4 to 18 ms in run 15. Asking only the octree (run 16) refused every stop the AI
tried, as its nodes were among those it misses. Now the verdicts share
one whole read for 5 seconds (`nodeCache`), and a verdict waits for a
frame of its own, 250 ms after the last:

| verdict, over the whole run | verdicts | mean | slowest |
|---|---|---|---|
| run 15, before | 61 | 32.5 ms | 81 ms |
| run 17, a shared read and one a frame | 91 | 16.5 ms | 61 ms |

What is left is the game's verdict itself, which `makeProposalData`
cannot split (run 17's slowest, 61 ms, was 60 ms of it), and a whole read
when the shared one is 5 seconds old (32 ms of a 46 ms verdict).

## Trucks (2026-10-30)

AI trucks carry cargo since run 22: fuel from Hunstanton Oil Refinery to
Hunstanton, and meat from Heanor and Loscoe Livestock Farm to Penzance,
loaded full (10 a truck) at the industry's own station and unloaded at
the AI's truck station in town, alike in both games. Three things had to
change together, found over runs 8 to 22 on the big fixture: where the
cargo goes, where it is loaded, and how the stops are set.

Runs 8 to 14, every AI choosing crude oil from Penrith Oil Well (318 a
year, its output stock full at 600) to Wallingford Oil Refinery, 1 km by
road:

| run | the line | what the game did |
|---|---|---|
| 8 | street stop by each industry; load rules for passengers only | every truck "VehicleUseless" |
| 9 | load rules by cargo number (crude oil is 18) | no issue, nothing loaded: a street stop only unloads (`small_stops.script.lua`, `cargoLoad = false`) |
| 10, 11 | the AI's own one-platform truck station by the well, with and without its modules' metadata | built and snapped; nothing loaded |
| 12 | both industries' own stations | nothing shipped in a game month |
| 13 | the well's own station, a street stop by the refinery (as players' livestock line does) | nothing shipped |
| 14 | the same, at the well station's second terminal alone (as players' farm and quarry lines) | nothing shipped in six game months |

Runs 15 to 22, each with three AIs in a two-game room:

| run | the line | what the game did |
|---|---|---|
| 18 | grain, the AI's truck station 79 m from Penzance Crop Farm, a street stop by Heanor and Loscoe Livestock Farm | trucks passed the station empty |
| 19 | the AI's truck station at both ends; crude oil, stone, grain to refineries, cement plants and the livestock farm | 21 trucks on seven lines, all empty |
| 20 | to towns that need the cargo (fuel, meat, cement), the AI's truck station at both ends | empty |
| 21 | the same, loading at the industry's own station | empty after a round trip |
| 22 | the same, its stops set as the game's line window sets them | fuel and meat loaded full and unloaded in town |

What each step found:

- **Where it goes.** An industry takes its inputs only while what it makes
  is carried on, as in Transport Fever 2. The three refineries the AIs
  chose took nothing and made nothing; the cement plant and the livestock
  farm took little and shipped little (96 of 172 and 36 of 432 a year),
  already supplied by the fixture's lines. Towns take what their
  buildings need, as the town window lists it (`townBuildingSystem.
  getCargoSupplyAndLimit`): on the fixture, of 916 fish towns may take a
  year, 102 were supplied; of 828 meat, 38; of 660 fuel, 76; of 588
  planks, none. So the AI carries to towns, and reads what each needs
  with the towns (bridge version 32).
- **Where it is loaded.** Players' truck lines load at the industry's own
  station (the fixture's livestock farm, farm and fishing grounds), whose
  terminals are all alike, each taking cargo and passengers for trucks
  and buses (`industryutil.lua`, `addTerminals`). The AI's own truck
  station by the industry caught the industry (`getStationCatchables`
  with cargo: its stock list) but loaded nothing in runs 18 to 20.
- **How the stops are set.** Run 21's line matched the players' at the
  livestock farm, the same station and terminal and a stop in town that
  catches meat demand, and still loaded nothing. What differed: the
  players' stops list every cargo type (37) in their load and maximum
  lists, load passengers too, wait as a road stop does (600 s at most,
  30 s for more) and drop cargo a change of rules leaves aboard; the
  AI's listed up to its cargo's number, waited 0 s and kept such cargo.
  Run 22 set all of these as the game's line window does
  (`line_util.tl`, `makeNewStop`, `autoLoadConfig`) and its trucks loaded
  at their first visit. Which of them the game needed was not isolated.

Delivery shows in the towns' own figures: over run 22's second half,
Hunstanton's fuel supply rose from 91 to 121 (of 205 it may take) and
Penzance's meat from 37 to 45 (of 72); the games did not diverge. Still
open: run 22's cement line, from Chesterfield Cement Plant to Torpoint
(12 a year wanted), loaded nothing in 15 minutes.

The universal steam lorry the AIs buy takes every cargo but passengers
(`getAllCargoForCargoTypeSet`). A probe typed into a game whose console
is closed reaches the game as keys: one opened a bridge tool in the
second game of run 22; it was closed, nothing built, and the games did
not diverge.

## Trains (2026-10-30)

An AI ran a train in run 26 (one game, one Hard AI, the big fixture): a
China JF1 steam engine and six bulk wagons on a rail line it built from
Chesterfield Cement Plant to Torpoint, about 3.3 km, loading the plant's
cement full (60) at its own station and taking it to the town. Planning
to the train running took about a minute: both stations at their first
placement, the track in six pieces with none refused, the depot and its
track, the line, the train.

| run | what happened | changed |
|---|---|---|
| 23 | Its first rail plans were fish from fishing grounds: no dry station site by them. | Industries answer whether they stand on water; the AI carries nothing from one by road or rail. |
| 24 | The line was built end to end, but the game refused the train the line. The line's modes read back as mode 0. | The mod hands the game every line's modes as a full list (below). |
| 25 | The line had mode 7, train, and the game still refused the train: the AI had bought the strongest engine, the C 6/8, which is electric (engine type 2), on track without catenary. | Models say whether an engine is electric; the AI buys none for its lines. |
| 26 | The train took the line and carried cement. | |

Facts read for this, from the console and the game's own scripts:

- A modular rail station travels as its modules' names alone (the
  fixture's stations carry no module metadata): a one-track cargo
  station is cargo platforms in slots 6400000 + 10 j, its track in
  8402000 + 10 j and a building (`createTemplateFn`); its track ends lie
  at (10, 40 s - 20) and (10, 40 e + 20) on its own axes, a rail depot's
  join at (-2.2, -70). Its cargo catchment is its building's, 150 m.
- A track laid onto a station's end resolves to the node there, as the
  track tool's does.
- The game copies a line's transport modes as a list, in order, its keys
  aside: `{ [7] = true }` became mode 0. This was the mod's own bug, for
  players' captured lines too: `apply.lua` now hands a list of all 16.
  Bus and truck lines read back with their own modes all the same (how
  the game came by them was not looked into); the rail line did not.
- Train engines are `landVehicle` engines with a type: 1 steam, 2
  electric, 3 combustion, 0 none; all trains list modes 7 and 8.
- The land: 256 heights in 1 ms, 256 octree queries in 2 ms; the octree
  finds an industry from anywhere over its ground.

## Track costs (2026-10-30)

What the game charges for track, read from its own verdicts (nothing
built) in one game on the big fixture in 1922, through the mod's
`apply.proposalOf` and `makeProposalData`: 200 m of simple track,
straight, on flat open ground, built as each of these.

| built as | cost | a metre |
|---|---|---|
| on the ground | 10,615 | 53 |
| an embankment 2, 5, 8 m high | 18,479; 50,834; 102,975 | 92; 254; 515 |
| a cutting 2, 5 m deep | 22,457; 54,502 | 112; 273 |
| climbing 4 m over the 200 m | 19,773 | 99 |
| a stone bridge 0.5, 2, 3.5, 4.5 m up | 27,319; 39,352; 40,691; 42,436 | 137; 197; 203; 212 |
| a stone bridge 6, 10, 15, 25 m up | 46,327; 66,540; 116,970; 325,070 | 232; 333; 585; 1,625 |
| a trestle bridge 6, 15 m up | 27,689; 63,010 | 138; 315 |
| a tunnel 5, 7 m down | 161,235; 136,049 | 806; 680 |
| a tunnel 10, 20 m down | 121,002; 163,819 | 605; 819 |
| 160 m across a street at 90°, 45°, splitting it | 18,125; 14,415 | |
| the same at 10° | refused, "Construction Not Possible" | |
| the same through the street, no split | 13,417, "Collision", not critical | |
| a one-track cargo station of 3, 6 modules | 277,298; 561,711 | |
| a rail depot | 1,026,832 | |

The earth fits the game's config (`base/mod.script.tl`: 4.5 a cubic
metre raised, 5.25 lowered) for a bed about 4 m wide whose sides are as
steep as they are high; a field costs 200,000 to remove, so the AI keeps
off them (they read as built land). A bridge beats an embankment above
about 4.5 m (the game builds one even half a metre up), a tunnel a
cutting below about 9 m (a shallower tunnel costs more). The trestle is offered
until 1970 for rail alone, at 60 km/h. The prices are the AI's
(`crates/tpf3mp-ai/src/costs.rs`); a game's own price setting scales
them, and the AI reads the scale off its verdicts.

A track laid through a street without splitting it is not refused by
the verdict: "Collision" comes as a message, not a critical error. The
AI refuses such a piece itself.

What the cost-aware track met in the real game (two games, Hard and
Normal AIs, the big fixture):

| run | what happened | changed |
|---|---|---|
| 29 | The cement line of run 26 found no route. On the game's land (dumped and planned offline) its plant station stands 96 m up and Torpoint's 7 m, 89 m down over 3.6 km: the height search's 2.5 % allowed 76 m, its levels rounded down. | 2.5 % where it can, 4 % at most, each metre beyond 2.5 % costed high; the price it reckons leaves that cost out. |
| 30 | A line's estimate passed, its stations were built, then its crossings near Wadebridge's junctions failed it. | The whole line laid out before anything is built; its station pairs tried in turn. |
| 31 | "Too Much Incline" on the cement line's first piece, which held a level crossing on the hill. | A crossing lies level (1 %) on a street no steeper than 4 %. |
| 32 | The refused piece read "crossing, ground 1 m at +14.9 %": the crossing's node 1 m from another; then every crossing of the plant's steep road refused, round and round. | A node within 20 m of a street takes the crossing's place; a street no crossing may be laid on is passed over or under. Every piece put to the verdict before anything is built. |
| 33 | The meat line to Wadebridge: its first station pair had no heights within the grade; the next was laid out with level crossings, three layouts refused at a crossing ("Construction Not Possible") in trial, the fourth passed; stations, track with 4 level crossings, line and train, alike in both games (60 world-check windows matched). | |

## Passing loops (2026-10-30)

A rail line of 2 km or more gets a passing loop and two one-way signals
(runs 35 and 36, the cement line of run 29):

- The loop's track, built as a track whose ends are two nodes of the
  line (`Resolve::Node`), made two switches; the game took it at once.
- A signal is refused ("Construction Not Possible") anywhere in the first
  30 % of either edge that leaves a switch, where the two tracks still
  run close, and taken from 70 % on; the single track into a switch takes
  one anywhere (verdicts at nine places on each of the loop's edges, run
  35). The AI's signals stood 20 % from the switches in run 35 and were
  refused; at 75 % in run 36 both were placed.
- Which way a one-way signal passes trains: on the line's own track by
  the loop's far end, the game's pathfinder found a train's path from the
  node before the signal on past it (4 edges) and none back past it (0),
  from the line's own node (the loop's node 5 m off left out). So the
  engine's side flag `false` passes trains running the way the stop's
  `direction` points, as on TPF2.
- A cargo line runs full one way and empty back: the line's load, the
  mean over its stops, stays near half, so a cargo line counts as full by
  the cargo waiting at its stops (60), not by its load.
## Passenger trains (2026-10-30)

- Passengers are the game's cargo 0, `::/cargos/passengers/passengers.cargo`
  (class PASSENGERS); the models question for trains with it gives the
  coaches (and a handcar, which has an engine). A one-track passenger
  station is passenger platforms in slots 7400000 + 10 j, its track in
  8401000 + 10 j (5 m across its axes) and `main_building_1_era_<a|b|c>`
  (`modular_station.script.lua`, `AddPassenger(0)`, `AddTrack(1)`).
- Run 38: the line from Dinnington St John's to Basingstoke (3.8 km) found
  no route. On the game's land (dumped and planned offline) several
  railways and a river lie between the towns, and the route search took
  another railway as a wall; and a station site whose ground just beyond
  its exit was built on failed every search at once, ending the pricing
  after two. Another railway is now passed over (7.5 m) or under (10 m),
  never crossed on the ground; a site needs clear ground 160 m out of its
  line end; three pairs of sites are priced.
- Run 39: the same line, five layouts refused in trial ("Collision" over
  the railways and river, a level crossing), the sixth passed; stations,
  track with 15 bridge and 2 tunnel edges, the line and a train of six
  coaches running, the world checks alike in both games. Its passing loop
  was refused ("Construction Not Possible"), leaving it one train.
- Lines of 5 km or more get a loop for each 2.5 km; a second line from the
  same place shares the first's rail depot (both in the brain's stand-in
  world; not met in the real game in these runs).
- Seen in passing: after the host's game quit, the joiner's game hung in
  the room's save ("saving the world for the room (event 69)", its log
  silent after) and had to be ended.
## Production chains (2026-10-30)

Run 27 (two games in one room, a Hard and a Normal AI, the big fixture)
built chains of two lines, alike in both games, with no divergence:

- Fuel to Sevenoaks: Wallingford Oil Refinery, which made nothing, by
  truck to the town first, then crude oil from Penrith Oil Well by truck
  to the refinery. The well's crude now loaded (three trucks full, 30
  shipped), where in runs 8 to 19, with nothing carrying the refinery's
  fuel on, it never had; delivered, the refinery made fuel and chemicals
  of it (its stocks 4 and 8 a few minutes on) for the fuel trucks.
- Meat to Wadebridge: the leg into the town by train (Pocklington
  Livestock Farm), its grain feeder by train refused for want of a
  station site by the farm, whose fields fill the ground around it. Such
  a leg now goes by truck where trucks reach.
- A chain whose leg into the town had no road (meat to Southend-on-Sea)
  went no further.

A rail line now chooses its train before it builds anything: in the
brain's stand-in world, a line built for a cargo no wagon carried was
left without a train.

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
  the driver, against the 1.5 ms a frame budget. *Measured for a stop
  and a depot (above): about 25 ms, over budget; roads and track still
  to measure.*
- Whether a construction's verdict (stations, depots, harbours, airports)
  needs the construction's own parameters the tools set, and which of
  them the AI can choose from the template's defaults. *A road depot takes
  `seed` and `year`; a two-sided street stop only its model, by year.*
- The game's limits a route must meet (track and street gradients, curve
  radii, station lengths), read from the templates and checked against
  verdicts.
- How the game pays for cargo and passengers by distance and time, learnt
  from the lines' own income (the finance tables per line) rather than
  copied from the game's scripts, so a patch that changes the formula
  changes the AI's estimates with it.

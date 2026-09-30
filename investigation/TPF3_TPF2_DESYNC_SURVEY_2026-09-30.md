# What TPF2MP learned about desyncs, and what TPF3-MP should take -- 2026-09-30

A survey of the Transport Fever 2 multiplayer mod (`silver2127/tpf2-multiplayer`,
`origin/dev` fetched 2026-09-30, head `721ac61`) for desync prevention worth
porting to TPF3-MP, checked against TF3 Steam build 40408 with `tools/tpfre`
(index `tpf3.tpfdb`, the same binary as `investigation/TPF3_RNG_2026-09-29.md`).
Research only: no code changed in either repository, and no game was run.

Labels as in the RNG survey: **CONFIRMED** means read instruction by
instruction in TF3 (or stated in TF3's own API docs). **INFERRED** means
concluded from code shape or from TPF2's design. **NEEDS-MEASUREMENT** means
only a running room can settle it. TPF2 RVAs are for build 35924 and TF3 RVAs
are for build 40408.

What TPF3-MP already has, and this page does not repeat: the step gate, with
room-ordered actions applied in the same update. Game-script `math.random`
reseeded per step. The `TownDevelopAt` refusal and reseed switch. Two order
fixes: land-vehicle reservation order, and vehicles at a stop. The order
measurement (`claims`, `appends`, `land`, `vehstop`). The lanes, and the
rebase. PR 23: every member reloads the same save, so entity ids match.

## Summary, ranked

| # | item | TPF2 source | TF3 | TPF3-MP today | reload fix helps? | priority |
|---|---|---|---|---|---|---|
| 1 | **`tickCount` (GameTime+0x3c) advances on paused frames** | `156824d` pausedtick | **CONFIRMED identical** | not covered, and the step gate causes it | **no** | **P0** |
| 2 | Diagnostics: the counter lane, the town development trace, departure and path watchers, keyed drift, completion steps | `0610033`, `180a8d3`, `fd92df9`, `eb41fd3`, `d4dc6a1`, `5f1d45c` | n/a | partly (lanes, order measure) | n/a | P1 |
| 3 | Node-list order after identical loads, and the per-iteration family canon | `0115785` step canon, `family_canon.h` | INFERRED same structure | not covered (two sites sorted) | yes, **if** the load order is deterministic (NEEDS-MEASUREMENT) | P1 to measure, P2 to build |
| 4 | Game-script mods on wall clocks | `DETERMINISTIC_SCRIPTS.md`, `398392a`, `9125a2d` | CONFIRMED `os.clock`/`os.time` in 3 Mod Hub mods | `math.random` only | no | P2 |
| 5 | Unload deques (vehicle, stop, cargo) | `926b9a2` `unload` | INFERRED present (`SimEntityAtVehicleSystem`) | not located | yes | P3 (live join) |
| 6 | Road-edge entry order (exact-tie leader) | `f6195dd` roadentries | CONFIRMED consumers; two appenders | measured only | yes | P3 |
| 7 | Ship and aircraft claim order | `3217497` moveorder (measure only) | CONFIRMED serial node-order claims | measured only | yes | P3 (the canon covers it) |
| 8 | Person batches, capacity maps, freed-id batches | `cea5729`, `cc0dba6`, `5a9b3ae` | INFERRED / partly not found | not covered | **yes** | P3 (live join only) |
| 9 | Road free space float sum | `3217497` roadspace | **CONFIRMED absent** | n/a | n/a | none |
| 10 | Train order by name plus jitter | `3edfbcc` | CONFIRMED (priority sort after the shuffle) | ported by entity id | n/a | done (but see item 1) |
| 11 | Cross-toolchain parity (MSVC and GCC/clang) | `docs/re/crossplatform/*`, `LINE_COST_DESYNC.md` | n/a | D-mixed-platforms accepts drift plus rebase | no | reference only |
| 12 | Command timing: stamps in a peer's past, frame-granular dispatch, line grid | `06188ea`, `d4a1129`, `57942f5`, `97cbbc0` | n/a | covered by the step gate | n/a | none |

The one finding that matters now is **item 1**. Everything in items 5 to 8 is
TPF2's live-join work: a player joins while the others keep their running
worlds. PR 23 made TPF3-MP do what TPF2 did before 0.7: everyone reloads.
That work is only needed if TPF3-MP ever keeps a world at a join, and TF3
makes that harder than TPF2 did, because a TF3 load renumbers entities (see
"Hot joins").

---

## 1. `tickCount` advances once per paused frame (P0)

### What TPF2 found and fixed

TPF2 commit `156824d` ("the paused branch of GameSim::Step no longer advances
GameTime+0x30 per render batch"), patch `native/src/slice/ui_tints.inl`
`InstallPausedTick`, kill switch `pausedtick=0`.

TPF2's GameTime has two saved counters. +0x34 counts sim iterations. +0x30
counts them too, and it also counts once per render batch while the game is
paused. So every pause leaves two peers' +0x30 a different number of frames
apart, for good: a speed vote of 0, the load gate a hot joiner waits in, a
catch-up hold, or the gap hold. The sim reads +0x30 in four places:

- `TownDeveloper::Develop` and town creation, as a stamp in every proposed
  building and street;
- `MakeStreetProposal`;
- `AccountSystem::Update`, as `+0x30 % accounts` to pick this step's account;
- the `TrainMoveSystem` shuffle seed.

Symptom: same-save, no-command town splits (2026-09-16) that the person lanes
never showed. Fix: the paused call of the advance (`0x15aa4a`, 5 bytes) became
a NOP, guarded by 22 + 16 bytes.

### TF3 has exactly the same code (CONFIRMED)

- **`GameSim::Step` `0x159390`.** When the iteration count `r12d` is 0 (the
  paused path), it calls the GameTime advance `0xbace10` with `r8b = 0` at
  **`0x159412`** (`mov edx,[rcx+0x208]; mov rcx,[rcx+0x18]; xor r8d,r8d; call
  0xbace10`). It then calls the engine's paused update (vtable +0x58, `dt` =
  0) and returns. The running loop calls the same advance with `r8b = 1` at
  `0x15954b` before each `ecs::Engine::Update` (`0x15955c`).
- **The advance `0xbace10`.** `0xbace99 inc dword [rdi+0x3c]` runs on every
  call. `0xbacea1 inc dword [rdi+0x40]` runs only when `r8b` is set. It is
  bracketed by GameTime's change notifications (`0x2bb6970` / `0x2bb6b50`).
- **TF3's own API says so** (`api/tealdef/api/engine.d.tl:434-437`):
  `tickCount` is the "number of simulation ticks, ticks also when the game
  is paused (once per frame)", and `updateCount` "does not tick when the
  simulation is paused". So +0x3c is `tickCount` and +0x40 is `updateCount`.
  This settles the RNG survey's open question about which field +0x3c is: it
  is not `updateCount`.
- **Every reader of `tickCount`** (the getter `0x2a95c0` returns `[GameTime+0x3c]`;
  its callers are listed below):

  | caller | what it does with `tickCount` | class |
  |---|---|---|
  | `ecs::LandVehicleMoveSystem::Update2` `0xac1b23` | the `minstd_rand` seed of the reservation shuffle (`0xac1b28..0xac1b48`: `% 0x7fffffff`, 0 becomes 1) | **SIM** |
  | `ecs::AccountSystem::Update2` `0xa7c63d` | `tickCount % n` (`idiv`) indexes the system's entity vector: which account this update processes | **SIM** (with more than one account) |
  | `0x90d1f0` from `TownDeveloper::Develop` `0x8dc240` (`0x8dc595`) | stamps `{tickCount, -1}` (`0x90d552: mov [rbp+0x4a8], ebx; mov dword [rbp+0x4ac], -1`), TPF2's stamp | **SIM** (town growth) |
  | `0x9692c0` via `0x967920`/`0x967720` from `TownDeveloper::Develop` and `0x95a120` | as above (INFERRED stamp) | **SIM** |
  | `MakeStreetProposal` `0xa36410` (`0xa378aa`) | the street proposal stamp | **SIM** (town streets), UI too |
  | `init_streets_util` `0x8fe750` (`0x8fe8c0`) | town creation | SIM (map/town init) |
  | `UI::StreetBuilder` `0x577ed0`, `UI::TownBuilder` `0x5a5c60` | the player's proposal; its bytes travel in the action | harmless |
  | `vehicle_util` horn data `0x26948f0` | horn sound choice | PRESENTATION |

  The town stagger in `ecs::TownSystem::Update2` `0xb61cc0` reads
  `updateCount` (`0x2a9680` returns `[+0x40]`; towns `i ≡ updateCount mod 16`),
  so it is not affected.

### TPF3-MP does not cover it, and the step gate triggers it

The step gate holds the world by answering 0 to the step's speed call: "the
game's paused path" (HOOKS.md, "The step gate in the game"). It does so
whenever the room withholds a step (the room is paused, or a player is
behind), and during every room `Load` and `Save` ("every call answering 0
until they are done"). Each such call runs `0x159412` and adds 1 to that
game's `tickCount`. How many calls a machine makes while held depends on its
own pacing (5 a second at 1x) and on how long its load or save takes. So after
the first hold, the games' `tickCount` values differ and never re-converge.

This also defeats the land-vehicle order fix. The fix makes the shuffle's
*input* a function of the entity set, but the shuffle's *seed* is `tickCount`,
so two games still shuffle equal-priority vehicles differently. The
`vehstop` fix and the `claims` lane are unaffected.

### Does "everyone reloads the same save" remove it? No

The save carries `tickCount`, so every member starts equal. Then every member
sits in the paused path for a machine-dependent number of calls while its
load completes and before the room releases the first step. The divergence
starts at every load, the rebase's included.

Why PR 23's runs did not show it: in real48 and the 4,200-update run, one bus
never contested track with an equal priority, and one player means one
account. The one unexplained town street with different geometry (real44,
TF3_VEHICLE_DETERMINISM "Also seen") fits this cause, as TPF2's town splits
did. That link is INFERRED.

### How to port it, cost and risk

- **Measure first (hours).** Add `tickCount` and `updateCount` to a lane, or
  log them at each checkpoint (the game script can read the GameTime
  component). Two games in one room will show `tickCount` apart after the
  first load and `updateCount` equal. Also add the seed to the `land`
  measurement, as TPF2's trainorder log does with `seed=`.
- **The fix (a day).** Make the 5-byte call at `0x159412` a NOP (`e8 rel32`
  becomes `0f 1f 44 00 00`) in the room's game. Add it as a profile target
  such as `GameSim::Step/paused GameTime advance`, with the expected
  sequence (`mov edx,[rcx+0x208]; mov rcx,[rcx+0x18]; call 0xbace10`, after
  `xor r8d,r8d` at `0x159405`; take the bytes with `tpfre q bytes`) and a check that the call
  reaches the advance whose `r8b` test is at `0xbace9c`. Do not touch the
  vtable +0x58 paused update after it.
  - An alternative that patches nothing new: in the step detour, when
    answering 0 in the room's game, read `tickCount` before and write it back
    after the call. That is an unnotified component write, and a NOP is what
    TPF2 validated.
- **Risk.** The NOP also drops GameTime's change notification while paused.
  TPF2 found nothing in its sim that observes it, but TF3 needs the same
  check: the callers of `0x2bb6b50` with the GameTime type, and whether any
  system's `ComponentChanged` or UI clock animation reads `tickCount` while
  paused. The worst plausible effect is a paused UI element that stops
  animating. Install it only while the room's game runs, and add a kill
  switch.
- A caveat for after the fix: a game that loaded the room's world must also
  not run any *unpaused* frame before the room's first step. The gate
  guarantees that already.

---

## 2. Diagnostics TPF2 built that would help TF3 find desyncs (P1)

Each of these was built because a divergence was invisible to the geometric
world hash. They are listed roughly by value for TF3's current symptoms.

| tool | TPF2 | what it does | TF3 use |
|---|---|---|---|
| **Town development trace** | `0610033`, `slice/town_trace.inl`, `tools/town_trace_diff.py`, switch `towntrace=1` | One `TT` line per `TownDeveloper::Develop` call: the tick, the town, its node-list index, a digest of the Town list, and the shared generator's state before and after. Plus node-list digests every 600 iterations. The diff tool names the first difference. | TF3's real44 street split. The site is `TownDeveloper::Develop` `0x8dc240`; the tick is `updateCount`, with `tickCount` logged beside it. It proves or clears items 1 and 3 for towns. |
| **Node-list digests** | the same trace, and the step canon's `reordered=` / `moved=` counts | a hash of each ECS family's entity order | item 3's measurement, from the existing `ecs::Engine::Update` detour |
| **Train and depot watchers** | `180a8d3`, `fd92df9`, `5f1d45c` (`watchTrains`, `watchDepartures`, switch `watch_trains=1`) | Per rail vehicle and per update: a path signature change (edge count, first and last edge) and stop/go transitions, with the sim step. Also the step each vehicle leaves its depot. Off past 200 trains. | Diffs two games' logs to the step at which a vehicle's decision split. TF3's `claims` lane says *that* the claim order split; this says *which vehicle* and *when*. |
| **Order logs with deterministic sampling** | `3edfbcc` trainorder, `3217497` `[shiporder]` | `seed= n= named= reordered= ids=<FNV of the final order>`, logged on the first step, on any change of `n`, and on a 1-in-64 sample chosen by the seed *value*, so two logs diff cleanly. `nameHash`, `rankHash` and `idHash` show "same fleet, different claim order". | Add `seed=` to `order measure`, and sample by value rather than by interval. |
| **Keyed vehicle drift that names the offender** | `eb41fd3`, `23ea94d` | Pairs each vehicle by its cross-game key, not by the nearest position. Logs the worst three with key, line and both positions. | TF3's `vehicles` lane is a digest. On a mismatch, also log the rows and name the vehicles (the entity id is safe to use now that ids match). |
| **Command completion step** | `d4dc6a1` | Logs the step at which the engine *completes* a line edit, a line assignment or a purchase, and the ticks since it was sent. | Confirms that an action applied at the same update on every game, not just that it was issued there. |
| **Per-stamp edge dump** | `2bbb2cb`, `916d841` (`dump_egeo=1`) | each game's sorted edge list, written per stamp | When lane 0 differs, dump the rows so the differing edge is named. real44 needed this. |
| **Forced hash cadence** | `97c27f8` (`tpf2mp_hash_every.txt`, down to 4 units) | lets the checkpoint interval be forced lower | TF3's `checkpoint_interval`: expose it for chasing a split to its step. |
| **Name lane** | `3edfbcc` `r` lane | vehicle names hashed in id order and sorted, compared at every stamp | Relevant only where a sort key is not in the lanes. TF3 sorts by id, which the lanes already cover. |
| **Person dump probe** | `docs/re/hotjoin/lab/probe.lua`, `hj_compare.py` | dumps every person's destinations, mode, speed and travel times per game unit; pairs and compares them | The only thing that caught person divergences 100 units before the people count lane did. TF3 cannot list some components from Lua; use what it can read. |
| **PERF lanes** | `5f1d45c`, `cc8fc15`, `522a303` | cost per job per update (hash, scans, watchers) | Keeps the diagnostics above affordable on big maps. |
| **Install and log hygiene** | `tools/verify_install.ps1` (`c665bdd`), `tools/snapshot_logs.ps1` (`ae691be`) | Hash-compares every shipped file across the game folder **and its Sandboxie overlays**. Copies every instance's logs before a restart wipes them. | TPF3-MP's second player runs in Sandboxie. A stale mod file in the overlay is a desync that looks like a simulation bug. |

TPF2 lessons about the detector itself, which TF3 already gets right and
should keep:

- A hash is a sample at a sim time, not a property of its stamp (`5005a0b`).
  TF3 reads lanes at the checkpoint update itself.
- Log which lanes differ on every mismatch, not only on the third strike
  (`916d841`).

---

## 3. Node-list order after identical loads, and the family canon (P1 to measure, P2 to build)

**TPF2.** ECS node lists (`NodeList<N>::Add` push_back, `Remove` swap with
last) are not saved. A load rebuilds them in its own order (topological,
mostly descending id), while a running world holds add/swap-remove history.
The fix, `0115785`, runs at the entry of `Engine::Update`: every family's node
list is put in ascending entity order and its entity-to-position index is
rewritten. The pure logic is in `native/src/family_canon.h`, with tests in
`tools/family_canon_test.cpp`. It is O(n) for a disturbed list and costs one
scan for a sorted one. It refuses anything it cannot verify.

This one site covered the Town stagger, industries, stock lists, terminal
claims, ship and aircraft claims, animal chunk seeds and scaffolds at once.
Measured: a live-joined pair split its town streets 724 units in with no
command. On native Linux the canon sorted 1 of 28 lists (GCC does not fold
`GetNodeList`), and every native session split in town growth within one to
three stamps.

**TF3.** The families are `ecs::ComponentGroupFamilyN<...>` (RTTI).
`LandVehicleMoveSystem::Update2` walks 20-byte node records. `TownSystem::
Update2` `0xb61cc0` walks a 4-byte entity vector at `[[r13+0x18]+8]` with
stride 16, starting at `updateCount mod 16`, so the Town list's order decides
which towns develop on which update. `AccountSystem::Update2` indexes an
8-byte-entry vector with `tickCount % n`. All CONFIRMED as code; that these
are unsaved node lists is INFERRED.

**Does the reload fix remove it?** Only if a load builds the lists in the same
order on every machine. TPF2's control runs, where every member reloaded,
matched 25 of 25 dumps, so TPF2's load order was deterministic. But TF3 has a
"Load Game Pool" thread pool, and TPF2's `3edfbcc` warned that "a
multi-threaded save load registers in thread-timing order". That is
NEEDS-MEASUREMENT for TF3. TPF3-MP sorts two consumers (land vehicles and
vehicles at a stop); the Town stagger, ship and aircraft claims, and the
account index are not sorted.

**Suggestion.**

1. Measure now: hash each family's node-list entity order in the existing
   `ecs::Engine::Update` detour (`0x2bb8a50`) every N updates, as TPF2's town
   trace does, and compare two games after the same load.
2. If the hashes agree, stop: the two built sorts are enough while everyone
   reloads.
3. If they differ, port the canon. `family_canon.h` is portable, pure
   and tested. The TF3 work is the layout: the family map in `ecs::Engine`,
   `GetNodeList` recognition (MSVC folds it, as in TPF2 Windows), and the
   NodeList index phmap.

Cost: two to four days with tests. Risk: an engine-owned container is
rewritten every update, so a wrong layout corrupts memory. TPF2 guards every
slot and refuses on doubt.

---

## 4. Game-script mods on wall clocks (P2)

**TPF2.** `DETERMINISTIC_SCRIPTS.md`, `mod/.../deterministic_script.lua`, and
commits `398392a` and `9125a2d`. TPF2 wraps a Workshop game script (Natural
Town Growth) in these ways:

- `os.time`, `os.clock` and `os.date` read a virtual clock made from the sim
  clock;
- the script gets its own seeded RNG stream, saved with its state;
- the script's `pairs` walks over town ids and cargo keys are sorted;
- repeated updates at the same sim time are suppressed;
- the script's `update` runs on a whole-second sim-time grid anchored at
  load.

Two failures were measured. Towns grew on different schedules at 1x and at
4x catch-up (town lane -5, +2, +5). And the engine's per-frame state sync
from its *other* engine's copy of the script put the growth state back every
frame, which froze a server's towns.

**TF3.** `os_time` `0x2fd04e0`, `os_clock` `0x2fd04a0` and `os_date`
`0x2fd08a0` are available to mods. Three Mod Hub mods use `os.clock` or
`os.time` (TF3_MODHUB_SCRIPT_MODS). TPF3-MP reseeds only `math.random`. TF3
has two game-script states per `CGame`, one per engine, so TPF2's
"state shuttled between the engine copies" failure has a place to happen.
That is INFERRED, not checked.

**Does the reload fix help?** No: the clock is read live.

**Suggestion.** Before rooms allow script mods, either refuse any mod whose
game script calls `os.*` (fail closed, cheap), or port the wrapper's clock
virtualization (a Lua-side day or two, plus finding TF3's equivalent of
`loadGameScript` wrapping). Measure whether TF3 calls a game script's
`load` per frame with the other engine's state (TPF2 saw about 7,000
restores a session).

---

## 5 to 8. Order fixes that matter only if a world is kept at a join (P3)

All of these keep a running world and a freshly loaded copy of it in
agreement. With everyone reloading, every member's containers are built by
the same load and the same history, so they agree without a fix. The
exception is a load order that is not deterministic (item 3).

| item | TPF2 (commit, site, measured symptom) | TF3 counterpart | TPF3-MP |
|---|---|---|---|
| 5 unload deques | `926b9a2`. Each (vehicle, stop, cargo) deque in `SimEntityAtVehicleSystem` is unloaded from its front in boarding order, with no saved key; sorted by id at Windows `0xa85aa5`. A live join on the dedicated server split with two trucks of one line loading differently. | INFERRED present. `ecs::SimEntityAtVehicleSystem` has the same asserts (`cargo2simEntities.size() == m_numCargoTypes` in EntityAdded `0xb13ad0`), `GetVehicleSimEntities` `0xb148e0`, `GetVehicleSimEntitiesCount` `0xb14ab0`, and `Update` `0xb14cd0` (`tv.state == AT_TERMINAL`). The deque layout was not read. | not located (HOOKS.md) |
| 6 road-edge entries | `f6195dd`. The per-edge `entries` are not saved. `GetNext` is a first-on-tie exact-float min search, so two vehicles at bit-identical positions pick different leaders. The entries are re-sorted after `Add` (0xa64473). After a hot join, buses drifted about 300 m with the hash still locked. | CONFIRMED. The min searches `0x255f340` and `0x255ef60`, and `GetNext` `0x255fb60`, compute each entry's `pos ± bound` (`vaddss`/`vsubss` then `vcomiss`), per entry, with no accumulation. The appenders are `Add` `0x255e940` and `AddRange` `0x255cc70`. | `appends` lane (measure only) |
| 7 ship and aircraft claims | `3217497`. Serial claims in node-list order, measured and not changed (the node vector is engine-owned and is edited during the walk). | CONFIRMED (the RNG survey): `ShipMoveSystem::Update2` `0xaf6120`, `AircraftMoveSystem::Update2` `0xa83a40` | `claims` lane (measure only) |
| 8a person batches | `cea5729`. Destination candidates, departures, walk arrivals and the idle list are each consumed in list order by one generator. A retained host picked other destinations 2 to 5 units after the join (2 of 25 dumps equal; 59 of 59 with the sorts). | INFERRED. `destination_util` `0x8e2940`/`0x8e4400` (`BinarySearchIndex`, the weighted pick), `SimEntityAtBuildingSystem::Update2` (RTTI), `PersonMoveSystem` (`0xaec190`...). `SimEntityIdleSystem` was not found by name. | not covered |
| 8b capacity maps | `cc0dba6`. `SimEntityUpdateHelper`'s destructor seeds mt19937(5489) and walks 5 + 4 `unordered_map`s in insertion order. Two people swapped a destination 120 units after a join. | Partly: `SimEntityUpdateHelper.cpp` exists (ctor `0x2569f90`, dtor `0x256b0c0`, 10+ functions), but no `0x1571` (5489) constant is in those functions (the 13 hits in `.text` are elsewhere). The seeding is either inlined differently or gone. NEEDS-MEASUREMENT. | not covered |
| 8c freed-id batches | `5a9b3ae`. `Engine::EndModification` appends each batch of removed ids to the FIFO in removal order, which follows history-ordered containers. The batch is sorted before the append. A new cargo got a different recycled id 280 units after a join. | INFERRED. `ecs::Engine::EndModification` `0x2bb4d90` (named by its string). The FIFO free-id deque at +0xe0 is measured in TF3_VEHICLE_DETERMINISM. | not covered |

---

## 9 to 12. Not needed in TF3

- **9. Road free space float sum** (`3217497` roadspace:
  `EdgeUseManager::GetUsedSpace` summed clipped lengths in single precision in
  entry order; 50 of 253 random bags of footprints gave a different float in
  two orders). **Absent in TF3, CONFIRMED.** None of the 12 `edgeusemanager.cpp`
  functions accumulates over entries. The only running float sum is in
  `AddRange` `0x255cc70` (`0x255ce58 vaddss xmm6, xmm6, xmm2`), and it adds
  edge lengths along one vehicle's path, in path order. That is deterministic.
  The RNG survey's reading stands.
- **10. Train order by name with jitter** (`3edfbcc`). TPF3-MP sorts by entity
  id before the engine's shuffle. TF3 then stable-sorts by priority, so the
  shuffle only breaks ties and TPF2's anti-starvation jitter is not needed.
  Correct as long as ids match (PR 23), **but the seed is `tickCount`
  (item 1)**.
- **11. Cross-toolchain parity.** TPF2's native Linux port found four
  standard-library sources of divergence:
  - `std::hash` (FNV on MSVC, identity on libstdc++) inside every
    `HashCombine` seed;
  - `uniform_int_distribution` and `shuffle`;
  - `default_random_engine`;
  - the `generate_canonical` endpoint.

  It also found libm ULPs and GCC's unfolded `GetNodeList`
  (`docs/re/crossplatform/REPORT.md`, `STREET_DESYNC_ROOTCAUSE.md`,
  `docs/linux/LINE_COST_DESYNC.md`: eight guarded windows and hundreds of
  thousands of oracle cases, still incomplete). TPF3-MP's decision is that
  mixed platforms drift and get rebased (DECISIONS.md). Keep TPF2's REPORT as
  the list of what to expect if that decision is ever revisited. Do not port
  any of it.
- **12. Command timing.** TPF2 had several failures of this kind:
  - stamps landing in a far-behind peer's past (`06188ea`, `d4a1129`);
  - depot departures and line assignments batched by frame (`57942f5`,
    `97cbbc0`);
  - a replayed buy without `purchaseTime` (`9ad6042`).

  TPF3-MP's server-ordered steps apply every action in a given update on
  every game, and actions are the engine's own commands. The vehicle
  determinism run measured `purchaseTime` equal.

## Hot joins and save/load renumbering: what TPF2 learned

- TPF2 went through three models: the host kept its world and only the joiner
  loaded (split), then everyone reloaded (`f94d8c0`, "frozen joins"), then
  live join (0.7, `bafa417`/`502b667`). Live join needed every fix in items
  5 to 8, plus item 1 and the canon. Even so, the native Linux side still
  lists "loaded-game lifetime validation outstanding" (HOTJOIN_ORDER.md). The
  acceptance gate TPF2 set for skipping the host's load: a retained world must
  match a loaded joiner past the previously observed failure interval, with
  vehicles, cargo, construction and later joins exercised.
- TPF2's free-id deque **is saved** (`Engine::Load` `0x23df8f0`), so a
  retained world and its reload agreed on ids and differed only in order. **TF3
  renumbers on load**: street edges came back as 7547 and 7546 after a
  resync, and the id queue is rebuilt (TF3_VEHICLE_DETERMINISM, "Loading
  renumbers"). A TF3 live join would therefore need an id canon on top of all
  of TPF2's order work, and TF3's depot start (`id mod 1000` mm) and the
  id-sorted order fixes both key on ids. **Keep "everyone reloads"** (PR 23)
  unless someone first solves the renumbering.
- Every pause is a hazard to any counter that ticks per frame (item 1). A hot
  joiner always has one: its load gate.
- The first checkpoint after a load is only comparable if every game samples
  at the same sim time (`5005a0b`). TF3's checkpoint-at-step design already
  does this.
- Speed changes inside a render batch stepped the interpolated clock back
  (`ShipFoamRenderer` assert, `5a9b3ae`). This is not a desync, and TF3's
  "every call must run" design already avoids it.
- TPF2 found that persons diverge 2 to 5 game units after a retained join, but
  the people-count lane only showed it about 100 units in. Per-entity dumps
  (the lab probe) catch order bugs much earlier than count lanes do.

## Suggested order of work

1. Log `tickCount`, `updateCount` and the land-vehicle seed at each
   checkpoint in two games, and confirm item 1. Then NOP `0x159412` in the
   room's game (P0).
2. Add a hash of node-list order in the `Engine::Update` detour, and compare
   two games after one load. This decides item 3.
3. Port the town development trace and dump lane rows on a mismatch. This is
   aimed at real44-type town splits.
4. Decide the policy for `os.clock`/`os.time` in game-script mods before
   rooms allow them.
5. Leave items 5 to 8 until TPF3-MP wants joins without a reload, and solve
   renumbering first.

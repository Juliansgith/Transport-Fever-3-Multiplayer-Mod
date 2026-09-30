# Train priority in Transport Fever 3, and what a room needs -- 2026-09-30

How TF3 Steam build 40408 (Windows x64) decides which train gets a contested
track section, junction, signal block or platform first, every source of
randomness or replica-dependent order on that path, and what TPF3-MP changes
for it. Read statically with `tools/tpfre` (index `tpf3.tpfdb`, the binary of
`investigation/TPF3_RNG_2026-09-29.md`) and the game's scripts; no game was
run. RVAs are for build 40408, image base `0x140000000`.

Labels: **SEEN** means read instruction by instruction here (RVA given).
**INFERRED** means concluded from code shape, names or the API. Some SEEN
items below were read by a helper pass in this session; their RVAs are the
ones to re-read if in doubt.

Built on this page's findings (branch `feat/train-order`):

- **`paused-tick`** (`crates/tpf3mp-hook/src/ticks.rs`): the room's paused
  frames no longer count in `GameTime.tickCount`, the seed of the train
  shuffle. A fix.
- **The land-vehicle shuffle's seed** in hook.log: a sampled line per 256
  seed values, and a `seeds` lane and a `nodes` lane in the order
  measurement. Measurement.
- **`ticks:` checkpoint lines**: `tickCount` and `updateCount` with the
  room's step at every checkpoint. Measurement.

## Summary

1. **Who goes first** is decided in one serial loop in
   `ecs::LandVehicleMoveSystem::Update2` (`0xac0f90`), for trains and road
   vehicles alike. The vehicles that want more track are collected in
   node-list order, shuffled with a `minstd_rand` seeded from
   `GameTime.tickCount`, stable-sorted by their line's reservation priority
   (highest first), and then each claims track in that order through
   `EdgeReservationManager::Reserve`. The first to claim an edge holds it;
   later ones find it reserved and stop (SEEN). Signals and platforms follow
   from those reservations (SEEN): a signal shows what the reservations
   say, and a platform is free when no other train reserved its track.
2. **Randomness on the path**: one generator, the shuffle's (SEEN). No wall
   clock, no frame time, no `rand()` (SEEN for the claim loop, the signal
   system and the platform chooser). **Order dependences**: the node-list
   order the shuffle permutes (fixed by the existing sort), the shuffle's
   seed `tickCount` (fixed now), the platform chooser's vehicle visit
   order (INFERRED node order, not fixed), ship and aircraft claims in
   node-list order (measured), and exact float ties among road-edge
   entries (measured).
3. **Did the entity-id sort plus the reloaded save make the order equal?**
   No, not before this change: the seed is `tickCount`, which counts every
   paused frame, and the room's game sits on the paused path for a
   machine-dependent number of frames at every hold, load and save (SEEN).
   With `paused-tick`, yes, for equal worlds: equal ids (PR 23's reload),
   equal wants-track flags and priorities (world state), and an equal seed.
4. **Trains and road vehicles share the path** (SEEN). Ships
   (`ShipMoveSystem::Update2`, `0xaf6120`) and aircraft
   (`AircraftMoveSystem::Update2`, `0xa83a40`) each claim through the same
   reservation manager type, in plain node-list order, with no shuffle and
   no sort (SEEN for ships, INFERRED for aircraft), as TPF2 found.

## 1. Where train priority is decided

### The claim loop (`ecs::LandVehicleMoveSystem::Update2`, vtable slot 12, `0xac0f90`)

- **Trains and road vehicles in one system (SEEN).** The update resolves the
  component types `BaseEdge`, `BaseNode`, `BaseNodeTrafficLight`,
  `BoundingVolume`, `Carriage`, `CarriageList`, `ModelInstanceList`, `Line`,
  `Train`, `TransportVehicle`, `BaseEdgeStreet` and `EmissionEmitter` at
  `0xac1010..0xac12b6`.
- **The contenders (SEEN).** `0xac15c0..0xac1abe` walks the family's node
  list: 20-byte records at `[[this+8]]`, the entity id at `+0`. A vehicle is
  a contender when the byte at `[sysdata+0x120][entity]` is set (it wants
  more track; `sysdata = [this+0xc8]`). Each contender becomes an 8-byte
  entry `{int32 nodeIndex, float priority}` in a vector at
  `[rbp-0x20]..[rbp-0x18]`, in node-list order.
- **The priority (SEEN code, INFERRED name).** `sub_abee50(engine,
  TransportVehicle type, Line type, entity)`: when the vehicle's
  `TransportVehicle` state (`+0xa8`) is 1 or 2, the float at `+0x24` of its
  line's `Line` component (40-byte records, the line entity at
  `TransportVehicle+0xb8`); otherwise 0. The API names one float in
  `Line`: `reservationPriority` (`api/tealdef/api/engine.d.tl:556`), so this
  is the player's line priority. Vehicles not on a line, or in another
  state, all tie at 0.
- **The seed (SEEN).** `0xac1b15..0xac1b4c`: `tickCount` through the getter
  `0x2a95c0` (`[GameTime+0x3c]`), `% 0x7fffffff`, 0 becomes 1.
- **The shuffle (SEEN).** `0xac1b70..0xac1c63`: MSVC's `std::shuffle`, a
  Fisher-Yates whose draws come from `minstd_rand` (`imul 0xbc8f`, 48271)
  through the standard library's bit-gathering `uniform_int`. It draws for
  every contender, road vehicles included, so one contender more or less
  anywhere changes the whole permutation.
- **The priority sort (SEEN).** `0xac1c6c..0xac1d62`: at most 32 entries,
  an insertion sort (`0xab71e0`) that moves an entry left only past strictly
  smaller priorities (`vcomiss; jbe`); otherwise `std::stable_sort` with a
  temporary buffer (`0xab7b40`). Descending, stable: ties keep the shuffled
  order. So the shuffle decides only among equal priorities.
- **The claims, in that order (SEEN).** `0xac1d62..0xac23c3`, one vehicle at
  a time: its 160-byte move state at `[this+0x18]` (by the record's
  component index), the path and the range already held
  (`+0x3c..+0x48`), then:
  - with nothing new to claim, `sub_255bf60` asks both the persistent
    manager (`[this+0xb8]`) and this update's temporary one whether any
    edge of the next range is reserved; if so the vehicle is blocked
    (`[state+0x70] = 0`);
  - otherwise it claims up to the next point it may pass:
    - `sub_bae070` decides whether the next junction can still be braked
      for (a float time-to-reach against `v^2 k / a`, no clock);
    - `sub_25c54d0` finds the last real signal on the path (a backward
      walk);
    - `sub_255bbc0` -> `0x255afd0` finds the first path edge that is
      reserved by someone else or occupied. At junction nodes it checks
      every lane crossing the node, and it asks the `EdgeUseManager`'s
      nearest-occupant search `0x255f340` about the rest;
    - `Reserve` (`0x255c2e0`) then takes the range.

  The first vehicle in the order takes a contested edge, and every later one
  finds it reserved.
- **The reservation manager (SEEN).** `EdgeReservationManagerData` is a
  phmap `flat_hash_map` from the 12-byte `EdgeId` to `{owner, count}`,
  copy-on-write through a `shared_ptr` (`0x255bbf0`). Every operation is a
  point lookup: nothing iterates the map, and hash order decides nothing.
  - `Reserve` and `Reserve_simple` (`0x255c160`) assert that the owner is
    the claimant, so "first reserver wins" is the caller's order, not the
    map's.
  - `Release` (`0x255c010`) asserts the owner too, and erases the edge at
    count 0.
- **After the loop (SEEN).** Four thread-pool loops (`0xab5770`, `0xab5330`,
  `0xab4e70`, `0xab5060`) move the vehicles. Lambdas 3, 4 and 6 return
  per-chunk results, joined in chunk order, so thread timing does not
  matter. Lambdas 5 and 7 write in place, per vehicle (INFERRED safe).

### Signals (`ecs::SignalSystem::Update2`, vtable slot 12, `0xafc980`)

SEEN:

- **Where each signal's state comes from.** A thread-pool loop (`0xafb320`,
  blocks of 128) runs `0xafbdd0` over the signal lists. It is read-only.
  A signal is set when the persistent reservation manager shows both of its
  edges reserved by one train whose path runs through it (`sub_abff30`,
  lookups through `0x255bce0`).
- **How the results come back.** Each block returns only the changes, and
  the blocks are joined in future order (`0xafb3f0..0xafb527`).
- **How they are applied.** Serially: each change sets `state` (`+0x10`) and
  stamps `+0x18` with `gameTime * 1000` (`0x2a9610`, simulation time).

There is no generator here and no order dependence. A train "waiting at a
signal" is a train that did not get the reservation past it in the claim
loop above.

### Platforms (`ecs::TransportVehicleSystem::Update2`, `0xb8bae0`)

SEEN:

- **When the choice is made.** On every update, for each vehicle
  `EN_ROUTE` with a valid line and stop and its decision flag set,
  `FindNextFreeTerminal` (`0xb84e20`, named by its assert) runs.
- **The candidates.** They come from `LineSystem`'s per-stop path table
  (`0xad2050`, `line2data`), in the table's order: derived line state, not
  history. Only candidates whose path starts on the vehicle's current edge
  are kept.
- **The cost.** Cost(previous stop -> candidate) plus cost(candidate -> next
  stop's primary terminal). Costs go into a temporary `unordered_map`,
  which is only looked up.
- **The order.** The candidates are `std::sort`ed by cost (`0xb76b30`, a
  float-only comparator). Ties fall in introsort order: not stable, but
  equal for equal input.
- **The search.** It starts at the vehicle's current terminal and wraps
  around. A terminal is free when none of its track edges is reserved by
  another vehicle (`0x255bce0`). If every candidate is taken, nothing
  changes and the vehicle asks again next update: that is "waiting for a
  platform".
- **Claims within one update.** A choice is stored for the rest of the
  update (`StoreTerminalAllocation`, `0xb8b960`, into a per-update copy of
  the allocation map), so a vehicle visited later sees what earlier ones
  took.
- **Air and water.** Carriers 3 and 4 (INFERRED air and water) use a
  least-occupied fallback (`0xb84a70`): a phmap lookup, with strict `<` over
  the cost order.
- **The vehicles at terminals.** Vehicles `AT_TERMINAL` are sorted by
  `(time, entity)` (`0xb768d0`, `0xb75440`), a total order.
- **What it reads.** Only `gameTime` (`0x2a9610`); never `tickCount` or
  `updateCount`. It draws nothing.

What the platform choice depends on, then: the reservations (the claim
loop's order), and the order this system visits its vehicles in (its list
at `[this+8]`, built by `0xb7d720`; INFERRED to be node or storage order).

## 2. Every random draw and order dependence on the path

| # | what | where | label | status |
|---|---|---|---|---|
| 1 | the shuffle's seed is `tickCount`, which counts every paused frame | seed `0xac1b23`; the paused call of the GameTime advance `0x159412` | SEEN | **fixed**: `paused-tick` |
| 2 | the shuffle permutes node-list positions, and node-list order is load order or add/swap-remove history | build loop `0xac15c0..0xac1abe`, shuffle `0xac1b70` | SEEN (code), INFERRED (history) | **fixed** earlier: `land-vehicle-order` sorts by entity id; now also measured (`nodes` lane) |
| 3 | equal priorities fall back to the shuffle | stable sort `0xac1c6c..0xac1d62` | SEEN | deterministic once 1 and 2 hold |
| 4 | the platform chooser visits vehicles in its list order, and later ones see earlier ones' allocations | `0xb8bae0`, `0xb8b960`, list from `0xb7d720` | SEEN (allocation), INFERRED (list order) | **not fixed**: equal after identical loads if TF3's load order is deterministic (the survey's item 3, NEEDS-MEASUREMENT) |
| 5 | ship and aircraft claims in node-list order, no shuffle | `0xaf6440..0xaf6a17`, `0xa83f75` | SEEN (ships), INFERRED (aircraft) | measured (`claims` lane) |
| 6 | first-minimum search keeps the first of exactly equal road-edge entries, which are in append order | `0x255f340` from `0x255afd0` | SEEN | measured (`appends` lane); exact float ties only |
| 7 | platform cost ties in introsort order | `0xb76b30` | SEEN | deterministic for equal input; nothing to do |
| 8 | other `tickCount` readers: `AccountSystem::Update2` (`tickCount % n`), the town developer's and street proposals' stamps, the notifications game script (`tickCount % 30`) | the desync survey, item 1; `game_mechanics/notifications/notifications.script.tl:50-52` | SEEN | fixed by 1 |
| 9 | `GamePerformSimulationSteps` (a debug command, and a button of the debug panel) adds to a pending count that `GameSim::Step` runs on top of its speed answer | `0x403b8e0`, written at `0x9d895c` and `0x736843`, read at `0x1593cd` | SEEN | known (HOOKS.md, the step gate); outside this path |

Checked and not a hazard (SEEN): the reservation map (point lookups only),
the signal system (joined in future order, applied per signal), the claim
loop's helpers `0xbae070` and `0x25c54d0` (pure functions of the path and
floats), and the movement loops' joins. No frame time or wall clock is read
anywhere on this path.

## 3. The seed: does the reload plus the id sort make the order equal?

Before this change, no. `GameSim::Step` (`0x159390`) takes its paused path
when its speed call answers 0. There it calls the GameTime advance
(`0xbace10`) with `r8b = 0` at `0x159412` (SEEN), and the advance runs
`inc [GameTime+0x3c]` always and `inc [GameTime+0x40]` only when `r8b` is
set (`0xbace99..0xbacea1`, SEEN). TPF3-MP's step gate answers 0 whenever
the room withholds a step, and during every room `Load` and `Save`
(HOOKS.md), so each game's `tickCount` gains a machine-dependent number of
counts at every hold. The save carries `tickCount`, so a reload starts every
game equal, and the frames spent loading and waiting for the first step
pull them apart again.

With `paused-tick`, a game in the room counts only the updates the room
releases (the running loop's call, `0x15954b`, `r8b = 1`, is untouched).
From a load of the room's save, every game's `tickCount` is then the save's
value plus the steps run, the same everywhere. So the shuffle has the same
seed and, after the id sort, the same input, and the claim order is equal
as long as the worlds are equal (same contenders, same priorities). A
caveat: a world taken without a reload (`Load` with no file) keeps the
count its own frames gave it before the room began; PR 23's "everyone
reloads" removes that case.

## 4. Who shares this path

- **Trains and road vehicles**: one `LandVehicleMoveSystem`, one shuffle,
  one claim loop (SEEN).
- **`Reserve` callers** (SEEN): `LandVehicleMoveSystem` `0xac1f20`;
  `ShipMoveSystem` `0xaf670c`, `0xaf67c3`, `0xaf6986`; `AircraftMoveSystem`
  `0xa83f75`, and the aircraft helpers `0xa83530`, `0xa83780`, `0xa838f0`.
  `Reserve_simple` is called from the three systems' node-added (slot 4)
  and component-changed (slot 9) callbacks; `Release` from their
  node-removed callbacks (slot 5).
- **Ships**: a per-update temporary reservation manager (`0xaf63a6`) beside
  the persistent one; a plain `0..n` loop over the node list
  (`0xaf6440..0xaf6a17`); no generator, no sort (SEEN).
- **Aircraft**: the same shape, with no `0xbc8f` and no sort among the
  callees (INFERRED).

## 5. What `paused-tick` changes, and what it leaves

The fix, where, and why a redirect rather than TPF2's NOP: HOOKS.md,
"Seeds, as built", piece 4. What else the paused call did, checked here:

- **The engine side (SEEN).** The advance opens an engine modification
  scope (`0x2bbbba0` BeginModification, `0x2bbc060` -> `0x2bb4d90`
  EndModification) around the increments. Its only change is `GameTime`, and
  it records one `ComponentChanged` (`0x2bb6b50`) into each observer's
  change log (the other engine's sync). A held frame records none; nothing
  changed.
- **Scripts (SEEN).** `notifications.script.tl` uses `tickCount` on paused
  frames (`dt == 0`) to rotate which of its four notification-type groups
  it refreshes. While the room holds, that rotation stops. Its sim-script
  slice (`tickCount % 30`) is a divergence the fix removes.
- **The industry window (SEEN).** `gui/entity_window/industry/industry.tl:90`
  rate-limits its expansion preview by `tickCount`, so it does not refresh
  while the room holds.
- **The debug panel.** Its `tickCount` state is commented out.
- **Native readers (SEEN).** `UI::StreetBuilder` (`0x578aeb`) stamps its
  proposal with `{tickCount, -1}`. `UI::TownBuilder` (`0x5a5e11`) hashes it
  into a seed. Both proposals reach the room inside the action's bytes. The
  horn choice (`0x26948f0`) is presentation.

Nothing else calls the advance's paused form: `0x159412` is the one
paused call, and `0x15954b` the running one (`tpfre q callers 0xbace10`).

## What to compare in two games' hook.log

With the same room and the same save, after the first step:

1. Install lines, once each:
   `paused-tick fix: installed (at 0x..., the room's paused frames leave tickCount alone; outside a room the game's advance runs)`
   and `order fix land-vehicle-order: installed (...)`.
2. At every checkpoint, and at the first batch after a load:
   `ticks: step <n>: tickCount=<t> updateCount=<u>`. The two games' lines
   for the same `<n>` must be identical. Before this change `tickCount`
   would differ after the first hold, while `updateCount` agreed.
3. About every 256 updates:
   `order fix land-vehicle-order: sample seed=<s> n=<k> ids=<hash>`. The
   same sequence in both logs means the same seed and the same contenders,
   so the same claim order. A line that appears in only one log, or a
   different `n` or `ids` for the same seed, names the update where they
   split.
4. With `TPF3MP_HOOK_MEASURE_ORDER=1`, the `order measure:` lines:
   - `seeds` must agree, as must `claims` (the claim order itself);
   - `nodes` says whether the land-vehicle node lists agreed before the
     sort (item 2 of the table);
   - `land` shows the same, and `reordered` counts the sorts that changed
     something.

## Open

- **The platform chooser's visit order (table item 4).** Measure it before
  changing anything: hash the entity order of `TransportVehicleSystem`'s
  list at `[this+8]` once per update, next to the `nodes` lane. If two
  games that loaded one save differ, the node-list canon (the desync
  survey's item 3) is the fix, not a sort at one consumer.
- **Confirm `Line+0x24` is `reservationPriority`.** Set two lines to
  different priorities and look for the difference in the claim order.

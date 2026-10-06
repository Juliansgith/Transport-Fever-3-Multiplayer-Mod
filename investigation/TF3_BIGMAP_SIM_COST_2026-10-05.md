# TF3 big-map simulation cost: emission grid and main-thread octree path (2026-10-05)

Build 40408, `TransportFever3.exe` SHA-256 `de1daad3…f23ef2` (matches
`~/TPF3-MP-builds/40408/tpfre-kinds.db`). Static analysis only: no game process
was started, attached or touched. Addresses are VAs (image base 0x140000000).

Evidence labels: **CONFIRMED-static** (read in the disassembly or asserts),
**DERIVED** (computed from confirmed facts plus stated assumptions),
**GUESS** (plausible, not verified).

Correction to the brief: 100 x 1000 tiles of 256 m is 25.6 km x 256 km =
**6,553.6 km²**, not 25,600 km². Gigantomaniac 112 x 112 tiles is 822 km², so the
big map has **7.97x** the area (and the cell count of every per-area grid).

## 0. Summary

| item | finding | label |
|---|---|---|
| Emission grid cell | 16 cells per map tile per axis: **16 m** cells on 256 m tiles | CONFIRMED-static (x16, 0.0625 x tile size); DERIVED (16 m) |
| Grid size, 100 x 1000 tiles | 1,600 x 16,000 inner cells (+1 border ring): **25.6 M cells** per grid; two grids (noise, pollution) | CONFIRMED-static (formula), DERIVED (numbers) |
| Grid size, 112 x 112 | 1,792 x 1,792 = 3.21 M cells per grid | DERIVED |
| Buffers | per grid: `concentration` + `averageConcentration`; one shared `m_tempBuffer`: 5 float buffers, **513 MB** on 100 x 1000 (64 MB on 112 x 112) | CONFIRMED-static (layout), DERIVED (size) |
| Passes per sim update | noise: Diffuse, Average; pollution: Diffuse, **Wind**, Average; each a full-grid Jacobi sweep (read one buffer, write the temp, swap); plus 2 border checks per grid | CONFIRMED-static |
| Frequency | every sim update, called with dt; `nSteps = floor(dt/0.2 + 0.5)`, asserts `dt >= 0.2f`; at 1x speed one step per update | CONFIRMED-static (code), DERIVED (dt = 0.2 per tick) |
| Cost per update | ~1.75 GB of DRAM traffic on 100 x 1000 -> **~30-60 ms** per update; ~0.22 GB -> ~4-8 ms on 112 x 112 | DERIVED |
| Who reads it | `TownSystem` update -> `town_util::CalculateTownPollution` -> `EmissionGridSystem::GetSumPolygonEmission`; Lua `getInterpolatedEmission`, `getTownEmission`, `getTownNoisePerDistrict`; saved in the savegame | CONFIRMED-static: **simulation state, lockstep-relevant** |
| Main-thread `0x1400a4b90` | **`ecs::Engine::GetComponentDataIndex(entity, compType)`**: linear search of the entity's (type, index) list; 1,097 call sites | CONFIRMED-static (assert strings) |
| Main-thread `0x140926269` | node visitor of `ConstEcsOctreeIterator<OctreeSystemData>` used **only** by `parcel_util::UpdateParcelCollision` (`0x1409312e0`), reached from proposal application (`apply_proposal.cpp` -> `construction_util_engine.cpp`); calls `GetComponentDataIndex` once or twice per entity in every visited node | CONFIRMED-static |
| Why the main thread grows | not the octree depth (leaf cells stay 128 m; depth 12 adds 2 levels) but the **number of proposals applied per tick x entities inside each proposal's union box**; more towns/industries on 8x the area | DERIVED (structure), GUESS (town count is the driver) |

Rough budget for one ~260 ms update on 100 x 1000 (stock ~70 ms): emission grid
~30-60 ms (DERIVED); parcel-collision octree path ~44% of sim-thread samples,
~100-115 ms if the sim thread is busy for the whole update (DERIVED from the
profile shares, GUESS that most `GetComponentDataIndex` samples come from this
path); the hook's `ZwQueryVirtualMemory` ~12% (~30 ms, being fixed). Together this
covers the ~180-200 ms difference within the uncertainty. Instrument before
optimising (section 5, option 0).

## 1. `ecs::EmissionGridSystem::Update` (`0x140aa9230`, vtable slot 11)

Full name from RTTI lambdas: `ecs::EmissionGridSystem::Update(ecs::Engine*,
ecs::INodeList*, float dt) const`, source `Game/ecs/EmissionGridSystem.cpp`.

### 1.1 Control flow (CONFIRMED-static)

```
assert(engine == &m_engine)                       // [this+0x40]
assert(dt >= 0.2f)                                // const 0x14367db8c = 0.2
nSteps = (int)floor(dt / 0.2f + 0.5f)             // vroundss mode 1; assert nSteps > 0
grids = copy of SystemData{noiseGridEntity, pollutionGridEntity}   // [this+8], 2 ints
for each gridEntity e in grids:
    comp = EmissionGrid component of e            // GetComponentTypeIndex(typeid(EmissionGrid)),
                                                  // GetComponentDataIndex (0x1400a4b90), get ptr
    if temp.size != comp.concentration.size: resize temp
    assert(m_tempBuffer.GetSize() == emissionGrid.averageConcentration.GetSize())
    innerHeight = comp.concentration.height - 2
    assert(innerHeight == emissionGrid.averageConcentration.GetHeight() - 2)
    repeat nSteps:
        LoopImpl(rows 1..innerHeight, minChunk 48): Diffuse(conc -> temp); swap(conc, temp)
        if e == pollutionGridEntity:
            LoopImpl(...): Wind(conc -> temp); swap(conc, temp)
        LoopImpl(...): Average(conc, avg -> temp); swap(avg, temp)
        CheckBoundaryConditions(conc); CheckBoundaryConditions(avg)
```

Swaps are `std::vector` pointer swaps (no copy). Each `LoopImpl` enqueues row
chunks on the game `ThreadPool` and **waits for every future** (`0x14011d580` on
each result) before returning, so the whole update is on the sim tick's critical
path. With one pool thread, or `rows <= 48`, it runs inline. Every kernel reads
one buffer and writes another (Jacobi), so the result does not depend on the
chunking or thread count (deterministic).

### 1.2 Kernels (CONFIRMED-static)

All three are scalar SSE (`vmulss`/`vaddss`, no FMA, no packed ops), unrolled by 4
along a row, rows `1..h-2`, columns `1..w-2`.

| pass | function | per inner cell | constants/params |
|---|---|---|---|
| Diffuse (`ecs::(anon)::Diffuse`) | `0x140aa8570` (dispatch `0x140aa79d0`) | `t = w1*(N+S+E+W) + w2*C + 1e-15` (5-point stencil) | `w1 = a*5*dt`, `w2 = (1 - 4a - b)*5*dt`, `a = [this+0x50]`, `b = [this+0x4c]`; assert `w1 >= 0 && w2 >= 0`; `1e-15` at `0x1436fd270` |
| Wind (`ecs::(anon)::Wind`), pollution only | `0x140aa96f0` (dispatch `0x140aa7b80`) | semi-Lagrangian bilinear: 4 neighbour loads, 8 mul, 3 add, **1 `vdivss`** | `interpolVec = -wind * 3.0 * dt`; wind = `comp+0x6c` (2 floats), gridPointSize = `comp+0x10`; assert wind step < 1 cell |
| Average (`ecs::(anon)::Average`) | `0x140aa8190` (dispatch `0x140aa7d20`) | `t = (1-c)*conc + c*avg` (EMA) | `c = [this+0x48]` |
| CheckBoundaryConditions | `0x140aa8350` | border ring only; asserts every border value is 0 within `FLT_EPSILON` | — |

Note the dt scaling: weights are multiplied by `5*dt` **and** the step loop runs
`nSteps` times with that same dt. At dt = 0.2 this is `w1 = a`, `w2 = 1-4a-b`
(mass-conserving except decay `b`). At dt = 0.4 the sum of weights is
`2(1-b) > 1` and it would run twice: unstable. So the engine is only correct with
dt ~= 0.2 per call (DERIVED). **"Run every N updates with dt*N" is not usable as
is** (section 5).

### 1.3 Grid geometry: `ecs::component::CreateEmissionGrid` (`0x140ba0170`)

```
X0     = (-tilesX/2) << 4        Y0     = (-tilesY/2) << 4
width  =  tilesX << 4            height =  tilesY << 4        // asserts > 2
gridPointSize = 0.0625 * tileSize (x, y)   // tileSize = (1 << map[+8]) * map[+0xc|+0x10]
concentration        = Grid(X0-1, Y0-1, width+2, height+2, 0.0)   // comp+0x18
averageConcentration = Grid(X0-1, Y0-1, width+2, height+2, 0.0)   // comp+0x40
type = r8d                                                         // comp+0x68
wind = 0x140ba1420(seed, &gridPointSize)                           // comp+0x6c, minstd LCG (48271)
```

CONFIRMED-static: 16 cells per tile per axis, one border cell. DERIVED: 16 m
cells on 256 m tiles. Component layout: `+0 X0, +4 Y0, +8 width, +0xc height,
+0x10 gridPointSize, +0x18 Grid conc (w at +0x20, h at +0x24, data vector at
+0x28), +0x40 Grid avg (w +0x48, h +0x4c, data +0x50), +0x68 type, +0x6c wind`.
The component is serialized (`io::Serializer<ecs::component::EmissionGrid>`,
field names `concentrationBuffer`, `averageConcentration`, `gridPointSize`), so a
save carries both buffers.

| map | cells per grid (with border) | buffers | memory |
|---|---|---|---|
| 112 x 112 tiles (822 km²) | 1,794 x 1,794 = 3.22 M | 5 x 12.9 MB | 64 MB |
| 100 x 1000 tiles (6,554 km²) | 1,602 x 16,002 = 25.6 M | 5 x 102.5 MB | **513 MB** |

### 1.4 Emitters: `EmissionEmitterSystem::Update2` (`0x140aa51c0`, slot 12)

Runs per type (noise, pollution) and asserts its grid geometry equals the grid
component's (`gridWidth == gridComp.width`, `gridPointSize == ...`). It buckets
emitters (lambda_1, chunks of 1,024) and splats them in 32 parallel row strips
(lambda_2 -> inner lambda_1 = hot `0x140aa4650`, the
`LoopResults<std::vector<int>>` iterator). Per emitter: distance attenuation
(`vdivss`, `0x1431873c5`), then a bilinear point splat (`0x140ba1e40` ->
`0x140ba1f40`) or the other insert (`0x140ba2230`) clipped to the strip. Cost is
O(emitters), not O(area), but every strip walks its emitter lists (CONFIRMED-static
structure; DERIVED cost). It writes into the grid that Diffuse then reads (DERIVED:
the component has no separate power-density buffer).

### 1.5 Cost estimate (DERIVED)

Per inner cell and update, with normal (read-for-ownership) stores:

| pass | bytes | noise | pollution |
|---|---|---|---|
| Diffuse | read 4 + write 4 + RFO 4 = 12 | 12 | 12 |
| Wind | 12 | — | 12 |
| Average | read 8 + write 4 + RFO 4 = 16 | 16 | 16 |
| total | | 28 | 40 |

68 B/cell x 25.6 M cells = **1.75 GB per update** on 100 x 1000; 0.22 GB on
112 x 112. Compute is ~4-6 cycles per cell scalar (Wind's divide dominates), i.e.
~25-30 ms single-threaded per grid sweep, which spread over the pool is below the
memory time. So the system is DRAM-bandwidth bound: at an effective 30-60 GB/s,
**~30-60 ms per update on the big map** vs ~4-8 ms on Gigantomaniac (where part of
the 64 MB also stays in L3). That is ~12-25% of the measured ~260 ms update and
explains why `Average` and `Wind` (the 16 B/cell pass and the divide pass) top the
worker-thread profile. It is not the whole regression.

## 2. Consumers: is it simulation or display? (CONFIRMED-static: simulation)

- `ecs::EmissionGridSystem::GetSumPolygonEmission(VoronoiCellClassifier const&,
  bool, EmissionGrid::Type, float, std::vector<float>&)` = `0x140aa8fc0`: sums the
  grid over a town district polygon's cell box (parallel rows, `0x140aa7850`).
  Its only direct caller is `town_util::CalculateTownPollution` (`0x140976620`),
  called from `ecs::TownSystem` slot 12 (`0x140b61cc0`, the TownSystem update).
- Script API (registered in `0x140f27a80`, `0x142536fc0`): `emissionGridSystem`,
  `getInterpolatedEmission`, `getEmissionGridEntity`, `getTownEmission`,
  `getTownEmissionDB`, `getTownNoisePerDistrict`,
  `getPollutionEmittersInSettlementArea`, `getNoiseEmittersNearSettlementLandUses`.
  Config keys: `noise/pollutionRatingLowerBoundDB/UpperBoundDB`,
  `...RatingRandomOffsetMinMax`, `pollutionRatingAreaMin/MaxSquareKm`,
  `emissionReductionPerEcoLevel`, `townNoiseResidential/Commercial/IndustrialAffected`;
  game script state `EmissionSimState.ecoLevels` (tealdef
  `game_mechanics/emission/*.d.tl`).
- `ecs::GridCollision` takes an `EmissionGridSystem const*` (constructor lambdas
  3-10); display: `GetDecibelVisualizationGrid`, `GetCombinedVisualizationGrid`,
  `EmissionMapRenderable`, the noise/pollution layers.

So the grid feeds town pollution/noise ratings and the eco-level mechanics, and it
is saved. **Any change to its values changes simulation results** and must be
identical in every game of a room, unless it is bit-exact.

## 3. Main-thread hotspots

### 3.1 `0x1400a4b90` = `ecs::Engine::GetComponentDataIndex` (CONFIRMED-static)

Asserts `it != components.end()` in `ecs::Engine::GetComponentDataIndex`
(`Lib/ecs/Engine.h:0x143`). Body:

```
list = m_entityComponents[entity]            // [engine+0x90] + entity*24, vector<pair<int,int>>
for (p : list) if (p.type == compType) return p.index;   // linear search
assert(...)                                   // cold path
```

The hot path is about 12 instructions, but the function keeps a /GS cookie and a
0xE0-byte frame because the assert's formatting is inlined; 1,097 call sites.
The ResTypeRep reading in the quick scan was wrong. `0x1400a4cc0` next to it is
`ecs::ComponentManager::GetComponentTypeIndex` (hash lookup of a `typeid`).

### 3.2 `0x140926269`: the parcel-collision octree walk (CONFIRMED-static)

`0x140926269` is a `.pdata` chunk of `0x140926240`, the per-node body of
`detail::ConstEcsOctreeIterator<ecs::OctreeSystemData>` instantiated for one
visitor. For each entity id in the node's list:

1. `GetComponentDataIndex(entity, nodeCompType)` -> the entity's octree record,
   then an AABB test against the query box;
2. `0x140280ff0` (has-component bitset + `GetComponentDataIndex` again) -> a
   0x1F0-byte component (the parcel; GUESS on the type name);
3. an FNV-1a hash-set lookup (already collected);
4. the callback `0x140ae7110` (`ParcelSystem.cpp`).

Recursive descent: `0x140925e30` (8 children, `m_nodeCompTypeIndex >= 0` assert).
Only entry: `parcel_util::UpdateParcelCollision` (`0x1409312e0`, assert string
`'!boxes.empty()'`). It builds **one query box = union of all input boxes + 50 m,
with z from -FLT_MAX to +FLT_MAX**, and walks from the root. Callers:
`0x1425fc630` (`construction_util_engine.cpp`) <- thunk `0x1425f0270` <-
`apply_proposal.cpp` `0x1409f96e0`, which is reached from `towndeveloper.cpp`,
`industry_util.cpp`, `parcel_util.cpp`, `init_streets_util.cpp`,
`apply_command.cpp` and others.

So the 28% + 16% are, DERIVED, the cost of applying construction proposals (town
and industry growth and commands): each application visits every octree entity in
the union box of its changed pieces and calls `GetComponentDataIndex` 1-2 times
per entity.

Why it grows on the big map: the octree's leaf cells are 128 m at depth 10, 11 and
12 alike (TF3_BIGMAPS_256KM §1), so a query costs at most 2 more levels. The
growth comes from **how many proposals are applied per update and how many
entities their union boxes cover**. With 8x the area at the same town density
there are roughly 8x the towns and industries growing (GUESS until counted). A
proposal whose pieces are far apart gets a huge union box (DERIVED from the
min/max code), which makes this path very sensitive to long or scattered
proposals.

## 4. Lockstep relevance

| system | affects sim | note |
|---|---|---|
| Emission grid values | yes | towns, eco levels, scripts; saved |
| Emission update order/chunking | no | Jacobi + deterministic float ops; the result is independent of thread count |
| `GetComponentDataIndex` speed | no | pure lookup |
| Parcel query box shape | possibly | the collected parcel list order follows the traversal; a different query shape can reorder it, and downstream code may depend on order |

## 5. Options, ranked

0. **Measure first (do this before anything else).** Add timing/count hooks
   (no behaviour change, per-peer) on `EmissionGridSystem::Update` `0x140aa9230`,
   `EmissionEmitterSystem::Update2` `0x140aa51c0`, `UpdateParcelCollision`
   `0x1409312e0` (calls per update, union-box area, time) and `TownSystem` slot 12
   `0x140b61cc0`. That replaces the GUESS shares above with numbers. Effort: low.
   Lockstep: none.

1. **Throttle emissions as a room setting (cheapest real gain).** Run both
   `EmissionEmitterSystem::Update2` and `EmissionGridSystem::Update` only on
   updates where `simTick % N == k`, always with the dt the game passes (~0.2 s).
   Do not pass `dt*N` (§1.2: unstable, and `nSteps` re-applies the full dt).
   Throttling both together keeps the steady-state field for steady emitters
   (emission and decay both scale by 1/N, DERIVED); only the response time
   becomes N times slower. Grid cost becomes 1/N (N = 4: ~8-15 ms per update on
   average, but every 4th update still pays the full ~30-60 ms. To spread it,
   process 1/N of the rows each update; that is option 3's machinery). Lockstep:
   **changes results**: must be one room-wide setting (like `TPF3MP_BIGMAP_OCTREE`),
   keyed to the sim tick, and on only past a size threshold. Save-compatible
   (same buffer sizes). Effort: low (two vtable-slot hooks).

2. **Coarser cells on big maps (room setting, new worlds only).** Patch
   `CreateEmissionGrid`'s `shl eax,4` / `shl ecx,4` (4 sites, `0x140ba0201` to
   `0x140ba0225`) to `shl 3` and the gridPointSize factor (0.0625 at `0x1436a3a64`
   is a shared constant: patch the instruction's operand, not the constant) to
   0.125: 32 m cells, **4x fewer cells** (128 MB, ~8-15 ms). Emitter and
   polygon-sum code read gridPointSize and dimensions from the component (they
   assert equality), so they follow (CONFIRMED-static for the emitter asserts).
   But the physics is per cell: with per-step weights unchanged, spread per second
   is 2x farther in metres, and town sums cover 4x fewer cells, so ratings shift
   (DERIVED). Saves made with the other size would fail the geometry asserts
   (GUESS: the load path would recreate or assert). Lockstep: changes results;
   room-wide; world-creation-time choice. Effort: low to medium plus playtesting.

3. **Bit-exact sparse update (best long-term; no room setting).** Re-implement
   Diffuse/Wind/Average natively with **exactly the same per-cell float operation
   order** (scalar or vectorised across x, which keeps per-lane order; no FMA, a
   real divide in Wind), and skip blocks (e.g. 64 x 64 cells) whose 3 x 3 block
   neighbourhood was bit-identical across the last two steps and received no
   emitter splat. Because each kernel is a pure function of the neighbourhood and
   the buffers ping-pong, a block that was stationary for two steps produces
   identical output, so skipping it is exact; anything else falls back to compute.
   Far from emitters the field sits at its float fixed point (~1e-15/b from the
   `+1e-15` bias), so most of a 6,554 km² mostly-empty world should be skippable
   (GUESS on the fraction; an ulp-level 2-cycle would only disable the skip there,
   safely). Also requires the wind (`comp+0x6c`) to be constant: only
   `CreateEmissionGrid` was seen writing it (GUESS that nothing else does; verify).
   Gain: potentially 5-20x on the grid. Lockstep: none if bit-exact, so it can be
   on per peer, but **validate bit-exactness with an A/B harness** (run the stock
   kernel and the new one on the same buffers, compare bitwise for many ticks;
   the checkpoint/rolling-check tooling can also hash the two grids). Effort: high.

4. **Bit-exact fused/vectorised kernels without skipping.** Fuse Diffuse+Wind+
   Average per row band with a 2-3 row rolling window (temporal blocking) and
   AVX2 8-wide lanes with the same op order: traffic falls from ~68 to ~20-25 B
   per cell, ~2.5-3x on this system (~12-20 ms). Lockstep: none if bit-exact
   (same validation as option 3). Effort: medium to high; option 3 builds on it.

5. **Lean `GetComponentDataIndex` (main thread, bit-exact).** Patch
   `0x1400a4b90`'s entry with a jump to a frameless, cookie-free copy of the hot
   path (same linear search, same result) that tail-calls the original only on a
   miss (the assert path). Helps all 1,097 callers; expected to cut maybe a third
   to a half of its 28% (GUESS). Lockstep: none. Effort: low to medium (one inline
   patch; check that no caller depends on the frame).

6. **Cheaper parcel-collision walk (main thread).** Options, in order of risk:
   (a) cache the octree record pointer per entity for the walk instead of calling
   `GetComponentDataIndex` twice (bit-exact; medium);
   (b) query each input box separately instead of the union box, then restore
   the stock traversal order before the callback list is consumed (exact only if
   the order is reproduced; high);
   (c) limit how many proposals the town/industry developers apply per update
   (changes results; room setting; needs RE of `towndeveloper.cpp` first).
   Run option 0 first to learn whether the cost is many small proposals or a few
   proposals with huge union boxes, since that picks between (a), (b) and (c).

7. **Thread-pool tuning:** the grid's row chunks (minimum 48 rows, 16,000 rows)
   already saturate the pool; with a bandwidth-bound kernel, more threads do not
   help. Not recommended.

Recommended order: 0 -> 5 (cheap, exact) -> 1 as an opt-in room setting for
worlds past 256 tiles if a fast fix is needed -> 3/4 as the proper fix for the grid
-> 6 once 0 shows the proposal pattern.

## 6. Address index

| VA | what | label |
|---|---|---|
| `0x140aa9230` | `EmissionGridSystem::Update` (vtable `0x1436fcab8` slot 11) | CONFIRMED |
| `0x140aa79d0` / `0x140aa8570` | Diffuse dispatch / row kernel | CONFIRMED |
| `0x140aa7b80` / `0x140aa96f0` (chunk `0x140aa9851`) | Wind dispatch / kernel | CONFIRMED |
| `0x140aa7d20` / `0x140aa8190` (chunk `0x140aa81ca`) | Average dispatch / kernel | CONFIRMED |
| `0x140aa8350` | CheckBoundaryConditions | CONFIRMED |
| `0x140aa8fc0` | `GetSumPolygonEmission` | CONFIRMED |
| `0x140ba0170` | `ecs::component::CreateEmissionGrid` | CONFIRMED |
| `0x140ba1420` | wind vector from seed | CONFIRMED (code), GUESS (meaning) |
| `0x140aa51c0` | `EmissionEmitterSystem::Update2` (vtable `0x1436fc430` slot 12) | CONFIRMED |
| `0x140aa4650` | emitter splat strip lambda (`LoopResults<vector<int>>`) | CONFIRMED |
| `0x140ba1e40`, `0x140ba1f40`, `0x140ba2230` | emission insert helpers | CONFIRMED (role), GUESS (exact shapes) |
| `0x140976620` | `town_util::CalculateTownPollution` | CONFIRMED |
| `0x140b61cc0` | `ecs::TownSystem` slot 12 (update) | CONFIRMED |
| `0x1400a4b90` | `ecs::Engine::GetComponentDataIndex` | CONFIRMED |
| `0x1400a4cc0` | `ecs::ComponentManager::GetComponentTypeIndex` | CONFIRMED |
| `0x140926240` (chunk `0x140926269`) | octree node visitor (parcel collision) | CONFIRMED |
| `0x140925e30` | octree recursive descent | CONFIRMED |
| `0x1409312e0` | `parcel_util::UpdateParcelCollision` | CONFIRMED |
| `0x1425fc630`, `0x1425f0270` | construction_util_engine caller, thunk | CONFIRMED |
| `0x1409f96e0` | `apply_proposal.cpp` main function | CONFIRMED (file) |
| `0x140ae7110` | ParcelSystem collect callback | CONFIRMED (file), GUESS (role) |

## 7. Built: the timers (option 0) and the lean lookup (option 5)

Branch `feat/simperf-lookup`. Both change nothing the game computes.

**Timers** (`crates/tpf3mp-hook/src/simperf.rs`; docs/HOOKS.md, "The game's
own systems: the `perf: sim` line"). Static facts they rest on
(CONFIRMED-static, checked by `tf3_static_proof.rs`):

| function | signature (args) | reached from |
|---|---|---|
| `EmissionGridSystem::Update` `0xaa9230` | `(this, engine, nodes, float dt)`, void | vtable slot `0x1436fcb10` only; no `E8`/`E9` to it in `.text` |
| `EmissionEmitterSystem::Update2` `0xaa51c0` | `(this, engine, int, float dt)`, void | vtable slot `0x1436fc490` only |
| `TownSystem` slot 12 `0xb61cc0` | `(this, engine, int)`, void; reads neither `r9` nor `xmm3` | vtable slot `0x143708bb8` only |
| `UpdateParcelCollision` `0x9312e0` | `(rcx, rdx, boxes, r9)`, void; no stack arguments; boxes `{min x, min y, max x, max y}` | one call, `0x1425fcb9a` |

The margin is `[0x14368c18c]` = 50.0f. The timers wrap the vtable slot (or
redirect the call), so they chain with any detour of the function itself.

**Lean lookup** (`crates/tpf3mp-hook/src/fastindex.rs`; docs/HOOKS.md, "The
faster component lookup"). Reversed exactly (CONFIRMED-static):

```
0xa4b90  mov [rsp+20],rbx; mov [rsp+10],edx; push rdi; sub rsp,0xe0
         cookie: mov rax,[0x143ce3a38]; xor rax,rsp; mov [rsp+0xd0],rax
         mov [rsp+40],rcx; mov [rsp+30],r8d; mov dword [rsp+38],0
         rdx = movsxd(edx)*3; r9 = [rcx+0x90] + rdx*8      // no bounds check
         rcx = [r9+8]; rax = [r9]
         while rax != rcx: if [rax] == r8d break; rax += 8
         if rax == [r9+8]: assert (0xa4c1b .. call 0x14303d3e0; int3)
         eax = [rax+4]; __security_check_cookie; restore; ret
```

No writes but its own frame and the caller's home space; no locks, no
state. The replacement repeats the hit path without the frame and hands a
miss to the original's trampoline; it changes `rax` and `r9` on a hit (the
original: `rax`, `rcx`, `rdx`, `r9`). Thread safety: as the original's,
since it reads the same memory and keeps nothing (whether pool threads
call it concurrently does not matter for either). Lockstep: none; the
evidence is the A/B test against the game's own code (320,000 random
queries, every answer and every miss identical) and the register test.

Measured (release, development PC, the user's game running, so noisy):
hot lists 10 to 14 ns a lookup, about 1.5 to 2 ns less with the lean
path (~15%); cold lists (2 M entities) 100 to 240 ns, dominated by two
dependent cache misses (the 24-byte list header, then the list), with no
difference above the noise. **The §5 option 5 guess ("a third to a half
of its 28%") does not hold**: expect at most a few percent of the main
thread's time. The parcel walk's lookups on a big map are likely cold,
so the gain there is small; option 6a (fewer lookups per walk) is the
one that can cut the misses. A SIMD scan was not built: the searched
lists are short, the time is the misses and the exit branch, and it would
need registers the original's hit path leaves alone.

**What to read in hook.log** after a minute on the big map at 1x:

- at install: `perf: sim timer emission-grid: in (vtable slot ...)` (and
  `emission-emitters`, `towns`, `parcel-collision`), and
  `fast-component-index: installed (...)`;
- every 10 s: the first `perf:` line's `ms/update`, and the `perf: sim`
  line: `emission-grid`'s mean µs is its cost per update (§1.5 predicts 30
  to 60 ms on 100 x 1000), `parcel-collision`'s calls and mean µs, and
  `union mean` / `max` km² (few huge unions or many small ones: §5 option
  6);
- with `TPF3MP_HOOK_PERF=full`: `component-index <n> calls` per window;
- A/B: the same save with `TPF3MP_HOOK_FAST_COMPONENT_INDEX=0`.


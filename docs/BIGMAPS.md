# Big maps: world layout, loading and memory, from tpf2-bigmap

What [tpf2-bigmap](https://github.com/silver2127/tpf2-bigmap) (0.5.2,
September 2026) found while pushing Transport Fever 2 build 35924 past its
map-size ceilings: how the world is laid out, which structures scale with map
area, what a load actually does, where the time and the memory go, which
optimisations held and which did not. The plugin's own account is its README
and the `docs/` folder of that repository (`runtime-memory-audit.md`,
`generation-peak.md`, `terrain-compression.md`, `alignment-batch.md`,
`load-speed-todo.md` and the per-feature notes); this page is the part that
carries into a TPF3 room where several machines have to load, hold and save
the same world.

Tagging follows the source: **measured** means read from a running game, a
process inspection or the game's own log; **derived** means computed from
decompiled code and not yet observed live. Sizes are for a 256 by 256 tile
world (65,536 tiles, 65.5 km on a side) unless stated; a 512 by 512 world is
four times the tiles. RVAs and offsets are TPF2's and will not survive into
TPF3; the shapes and the ratios are the point.

## How the world is laid out

- **A tile is 256 m.** Measured: a 224-tile map reports a 57,344 m bounding
  box. The New Game menu turns two dropdown indices into a tile count, and
  the heightmap is `tiles * 64 + 1` pixels a side, so the base heightmap is
  4 m per sample. The shipped presets are 18 by 54 up to 96 by 96 tiles
  (Megalomaniac, 24.6 km square, 604 km²); every Megalomaniac variant
  conserves that area as the ratio stretches. Tile counts are in the save
  header (two ints after the `tf**` magic, zstd-compressed).
- **Terrain has three resolutions.** The 4 m base heightmap (65 by 65 uint16
  per tile, persisted); a 1 m height cache per tile (257 by 257 uint16,
  132,098 bytes, rebuilt on every load by a bicubic refine and then cut by
  every road, track and construction alignment); and per-tile render data
  (a material-index grid of 260 by 260 bytes per tile, LOD tessellation
  patches). The save holds the base heightmap and the alignments, never the
  finished 1 m cache.
- **Every entity lives in one octree** whose root is a two-tier constant, not
  derived from the map: ±16,384 m at depth 9 for up to 128 tiles, ±32,768 m
  at depth 10 above that, 128 m leaves. Inserts never test containment; an
  entity past the root walks to a boundary leaf, and every query prunes on
  node boxes first, so anything beyond the box is invisible to lookups.
- **There are two complete game states**, swapped every simulation step
  (`GameState::Replicate`), so everything area-scaled below exists twice:
  collision rasters, emission grids, tree and asset instance lists, the
  octree. Terrain tile caches are refcount-shared between the two, not
  copied, in steady state.

## The ceilings, in the order they are hit

| ceiling | cause | symptom |
|---|---|---|
| 224 tiles | the menu clamps both axes (`GetNumTilesNew`); `settings.lua`'s undocumented `worldDimensionsOverride = {224, 224}` bypasses the preset table but not the clamp (both values must be even) | none: it is the menu |
| **180 tiles** (46.1 km) | "Creating streets" sizes a `vector<bool>` with one bit per square metre over the whole map through a **32-bit multiply**; at 224 tiles `57,345²` wraps negative, sign-extends to about 1.8e19 and `resize` throws `length_error`. Uncaught, so it is `abort` with no message; the thrown object's RTTI in the minidump is the only evidence | silent SIGABRT during generation |
| **32,768 m** (256 tiles) | the octree root. Beyond it the street builder finds nothing under a position and stacks a second node on the first. Measured on a 320-tile map: 21 duplicate positions, all with `max(|x|,|y|)` between 34,175 and 40,082 m, none inside 32,768, on all four edges; 168 repair failures in half an hour; towns in the band generate with zero population because they never get streets | `Duplicate base nodes found`, printed by a load-time repair pass, so the save is already damaged |
| 185 km pairwise | town and industry placement squares candidate separations in signed int32 pixels; above `sqrt(INT_MAX)` pixels distances go negative or wrap to a small positive value | towns clustered along the middle of a long axis (seen on a 58 by 292 km preview) |
| `(tilesX*64+1)*(tilesY*64+1) <= INT_MAX` | the heightmap element count | no 1,024-tile square; long maps must be narrow |
| Lua `%d` | Lua 5.2's `%d` goes through a 32-bit C `long` on Windows | any id above 2^31 formatted from a script fails; use `%.0f` |

Two lessons from the 180-tile wall that transfer directly. Widening the
multiply would have been worse: the two dimensions are stored as int32 and
every access computes `y*nx + x`, so a correctly sized vector would still be
addressed with wrapped indices past 2^31, silent corruption instead of a clean
abort. The plugin instead scales the raster's cell size (an argument to the
constructor) until the cell count fits, so every downstream index stays in
range untouched. And the octree fix is a 13-byte in-place rewrite (a wider root
constant inline, one deeper level), because the 32,768.0f in `.rdata` sits in a
run of constants with about a hundred readers; a shared constant is never the
thing to patch. Depth 11 is the cap of the original node-id scheme (ids are
`8*parent+1+octant` in 32 bits); going deeper needs compact id ranges for the
new levels and a patched renderer decoder, which is built and tested offline
but not yet validated in a running game.

## What a big map costs

### At rest

Per-tile structures (derived from code, the material grid measured):

| structure | per tile | copies | 256² |
|---|---|---|---|
| 1 m height cache | 132,098 B | 1, COW-shared by both game states | 8.06 GiB |
| material-index grid (render, CPU copy) | 67,601 B | 1 | 4.13 GiB |
| base heightmap | 8,450 B | 1, COW | 0.52 GiB |
| emission grids (16 by 16 floats, two layers plus scratch) | 3 KB | 2 engines, copied every step | 0.38 GiB |
| collision rasters at 16, 32 and 64 m plus passable tiles at 10 m | ~2.8 KB | 2 engines | 0.34 GiB |
| trees and scenery (24 B thin, 192 B fat instances, 12 B octree refs) | content | 2 engines | ~1.6 GiB per copy on the measured desert map, about 35-45 million trees |

The material grid was the surprise: it was believed to be touched only on
terrain edits, and a live read of the process (25,992 tiles, every cell
exactly 67,601 bytes, 18% of private memory) proved it retained for every
tile. Its bytes are dithered, not flat: 34 distinct values, a median of 8 per
tile, so it compresses to about 20-25%, not the 8:1 first guessed. The render
vertices, by contrast, were believed retained (1 GiB) and are not. Measure
retention before sizing a cache for it.

Whole-process, measured: a freshly generated large stock map was 15 GiB
private at the end of generation before the plugin held a byte; a 44 MB save
of a mid-sized world loaded to about 6.6 GiB; a loaded 256² save settled at
about 18 GB working set with the first pager alone and 10.5 GB once the
material grid was paged too. A 16 GiB machine cannot hold a 128 by 128 world
however the caches are set. Their world-entry stage log (private GiB): 3.1 before allocation,
14.9 after terrain generation, 6.45 at the start of trees, 9.8 after trees,
11.6 after scenery, 13.9 at the end of `InitNewGame`.

### Generating

Terrain generation is the binding constraint when creating a map, and it
follows a printed law. The game's own teardown line, `Terrain toolkit used N
maps and X MB`, is `X = (64*tiles+1)² * N * 4 / 1,000,000` (decimal MB,
truncated), verified exactly on seven logs from 96 to 512 tiles. Each map is
one full-resolution `vector<float>`; **all N are alive at once**, because the
name-to-map container has no erase path and is cleared wholesale at teardown,
and the peak is held through asset placement, the slowest stage. Desert uses
18 maps, temperate and tropical 10. At 256² that is 1.07 GB per map, 19.3 GB
for desert; at 512² 4.3 GB per map, 77 GB. Two layer ops allocate a further one
to two maps of scratch that the print does not count.

A Lua pass over the generator's layer list (splitting temporary names into
values at full overwrites and letting values with disjoint lifetimes share a
buffer, from a verified per-op table of which ops read the old output and
which write in place) took desert from 18 to 15 buffers and temperate from 10
to 9, the lower bound for those semantics. Sharing a name serialises layers
that used to run in parallel. A compressing pager is the wrong tool here: every
layer op is a full sweep over its map, so the working set is the whole map.

### Saving

Autosaves of a 1.4 GB save took 20-22 s (zstd level 3). Level 1 with a 64 KiB
input buffer instead of 128 bytes compressed 2.79 times faster for 9.2%
larger output (a 2.56 GB payload, identical after round trip); manual saves of
a 65,000-tile world then measured 13.7-14.4 s. The room-level consequence for
a TPF3 design: the stall timeout in [PROTOCOL.md](PROTOCOL.md) is sized as
"long enough for an autosave" from a 113 MB TPF2 autosave; a big-map autosave
is an order of magnitude larger and pauses the game for 15-20 s on a fast
machine, and a snapshot of that world is 1.4 GB on the wire.

**On TF3 (build 40408)** the save writer is the same: `PushCompressor`
(`0x32d450`) reads zstd level 3 at `0x32d464` and hands its stream a
128-byte buffer at `0x32d5c4`. Stage 0 measured saves of 3.3 s at
Gigantomaniac 1:1 (164 MB) and 7.7 s for a 45 km world (438 MB). The hook
rewrites both instructions in place, as tpf2-bigmap's `save_fast` did:
`mov eax,1` for the level and `mov r8d,0x10000` for the buffer
(`crates/tpf3mp-hook/src/savefast.rs`, profile targets "save:
PushCompressor …"). Each checks its bytes first and installs alone; the
loader is untouched (`PushDecompressor` keeps reading the constant), and a
save stays a standard zstd frame any game loads. Rooms judge lane digests,
not save files, so games with and without it agree. On by default;
`TPF3MP_HOOK_SAVE_FAST=0` keeps the game's own. hook.log says what was
applied (`faster saves: zstd level 1, a 64 KiB buffer`). The TF3 speed-up
is not measured yet.

### The emission grid

The noise and pollution grids have 16 m cells, so their cost grows with
the area: 1,602 x 16,002 floats each on 100 x 1000 tiles, eight times
Gigantomaniac's, moved one step every update in three full-grid passes
(about 1.75 GB of memory traffic per update). The hook runs each step as one
fused pass that leaves every bit as the game's passes would, on by default
(`TPF3MP_HOOK_FAST_EMISSION=0` keeps the game's own): 2.1x faster offline
(42 ms to 20 ms per update on the big map's grids). How and why it is exact
is in [HOOKS.md](HOOKS.md), "The fast emission grid"; the cost analysis in
investigation/TF3_BIGMAP_SIM_COST_2026-10-05.md.

## What a load actually does

The save holds the 4 m heightmap and every alignment. A load therefore
recomputes the whole 1 m terrain: the bicubic refine of every tile, then the
alignment pass, which hands the set of terrain blocks dirtied since the last
frame to `UpdateSubterrains`. In play that set is a few entries; on a load it
is the whole map, and the pass computes every block's result before
publishing any of them. Measured on a 207,360-tile save: 1,658,880 dirty
entries, about 9.9 million work blocks, 31.6 GiB live at the peak (25.8 of
them these blocks), and an "Out of memory" assert on a 94 GiB machine. Feeding
the same pass its own set in batches of 512 entries (a detour that walks the
game's `std::set` read-only and calls the original with a degenerate tree per
batch) took the peak from 34-36 GiB to 8.3 GiB. Compute and publication then
alternate, which is what the engine does per frame anyway. Load time against
stock was not measured.

A load also holds **two terrain versions**: both game states build their own
tile grid through `AddTile` (131,072 live tiles at 256²), and once filled
every tile has exactly one byte-identical twin in the other version (64,922
pairs of 65,536; measured by hashing every live tile every 10 s). The lever is
content dedup at eviction (a hash lookup instead of an encode for the twin),
not copy-on-write: the copy hook fired zero times during a load, and in play
every shared tile was written (710 of 710), so sharing was pure overhead.

Where the time goes, from a 20 ms instruction-pointer profile of a 256² save
load (about 70 s from "Loading from file" to "Initial material index
generation"), per 20 s window:

| window | top game work |
|---|---|
| t+120 | bicubic refine 11.2%, terrain alignment blend 5.4% plus its six per-call vector fills 5.3%, a 4 by 4 matrix product 3.5% |
| t+140 | the plugin's pager 24.5% (41% of the loading thread), the per-tile height block copy 10.1%, min/max scan 2.4% |
| t+160 | pager 28.5%, LOD tessellation 16.2% |
| t+180 | LOD tessellation 46.3%, pager 12.9% |

Two traps in reading such a profile. The tool's busiest-thread percentage is
that thread's share of **its own** samples: "one thread at 86% in
`UpdateLodTess`" looked like a serial loop, but 5,241 samples against 704 ticks
means at least 8 threads were inside it at once. It already runs on the
engine's pool (three quarters of the logical cores, 100 chunks), and a
plugin-side split would have gained nothing; nothing was built. And kernel
first-touch and pager-restore time is attributed to the faulting user
instruction, so a "hot" copy loop may be page faults: a 257² block copy is
1.1 µs warm and 25.7 µs into never-touched pages, and no user-mode copy
removes the second number.

New-map entry is different: on a new 256² map, town and industry **road
connections** took 127.5 s of a 230 s entry, terrain generation 54 s, trees
10.7 s, scenery 5.1 s. The connection stage mutates the network between
attempts and re-checks connectivity afterwards, so nothing in it can be reused
across attempts without changing which roads get built. A 287 s entry was
measured in another session; a room's load timeout (5 min in PROTOCOL.md) is
close to that on a big map.

## What held, and what did not

Every change below is behind its own configuration key, default off,
byte-verifies its site before patching, falls back to the original on any
mismatch, and has an offline test that runs the **original machine code**
(the exe mapped at its preferred base, or the function copied into the test
process with its constants relocated) against the replacement and compares
complete output buffers.

**Bit-identical fast paths.** Terrain heights are simulation data that every
peer must agree on, so approximate results were never an option. Each
replacement reproduces the same IEEE single-precision operations on the same
operands in the same association, only swapping operands where IEEE is
commutative, dropping only multiplications by exactly 0 or 1 (which can change
the sign of a zero and nothing else), no FMA, no reassociation, tested under
all four rounding modes:

| function | what it does | stock | fast | proof |
|---|---|---|---|---|
| bicubic refine (4 m to 1 m) | hoists the per-pixel divisions, inlines the two 4 by 4 products with zero terms dropped, packs four lanes | 245 µs per tile | 72 µs | 4,315 comparisons, 159 M samples identical |
| alignment blend | six 132 KB allocate-and-fill cycles per call become two `memset`s over a pooled buffer; an 8-wide "any weight here" test skips the dense pass | 126-800 µs | 27-532 µs | 1,374 comparisons, 12.8 M samples |
| tile min/max scan | SSE2 with a 0x8000 bias so signed min/max orders unsigned values; a 69-byte mid-function patch with a mechanical liveness proof over every exit path | 36.7 µs | 1.5 µs | 62,634 register-level comparisons |
| height block copy | one `memcpy` per row when the spans are disjoint, stock loop otherwise | 13.4 µs warm | 1.1 µs | 3,011 geometries, every aliasing shape |

**The compressed terrain pager.** Engine code keeps raw pointers into the
tile vectors, so a cache cannot move them. The pager reserves a placeholder
arena with a fixed address per tile version, backs resident tiles with
pagefile sections, and evicts by protecting the view, snapshotting through a
private read alias (no thread suspension, concurrent writers fault and wait),
encoding, and unmapping back to the placeholder; a fault decodes into a fresh
section at the same address and retries the instruction. The codec is planar
prediction, zigzag residuals and a static per-tile rANS coder with four
neighbour contexts, 6.95% of raw on real tiles (heights are in 5 cm steps, so
99.3% of residuals are 0 or ±1), with a 64-bit content hash checked on every
decode. Decode was bound by cache latency, not the coder: a 12-bit table
missed L1, and 10 bits (16 KiB) cost 0.01 points of ratio. Measured on a
256² map: 18 GB working set down to 10.5 GB.

The policy around it took as long as the mechanism. Once a second each pager
sets its resident target from four rules: while loading, everything may stay
resident down to a headroom (a seventh of RAM, 2-12 GiB), because the loader
re-reads what was just evicted; in steady state it ramps toward a hot budget
but grows when the engine faults 300 or more evicted tiles back per second and
keeps the level the stutter drove it to as a floor that decays over minutes;
a cap of a quarter of RAM; and a commit-pressure throttle that is **sticky**
for at least 30 s and until free commit clears the threshold by more than the
pager itself gave back. Without stickiness, on a machine with no page file
sitting 1 GiB under the threshold, the pager's own 3 GiB release cleared it,
it re-expanded, and the flag set again: 74 flips in one session, each
re-inflating the terrain in front of the camera. A flat 12 GiB reserve tuned
on a 94 GiB box starved a 32 GiB machine down to a 1 GiB target and every
tile faulted through a decode; everything is now a fraction of RAM.

**The material grid** got the same pager once static analysis found exactly
one allocation site and one free site for the 67,601-byte cells and no copy or
move of the cell triples; a private codec (dense alphabet, order-1 rANS)
stores it at about 20%.

**What did not work**, each with the measurement that killed it:

- A **2 m height cache** (129 by 129) halved the cache and broke the game:
  alignment regions are metre-indexed and were read as 2 m coordinates
  (features repeated across cells, cliffs at cell edges), the construction
  worker combines metre rectangles with the cache's levels, and the renderer's
  upload contract is a fixed 259 by 259 samples. Lower memory and a successful
  load proved nothing about correctness.
- **Copy-on-write sharing** of resident tiles between the two versions: the
  load never reached the copy hook, and in play every share was privatised.
- A **page-granular small pager** for the alignment pass's work blocks: it
  restored 4.9 million blocks correctly and could not keep pace with 50,000
  allocations a second from 24 engine threads on one lock; the peak stayed.
- Routing the **generation float maps** through the pager: every layer op
  sweeps its whole map, so it would thrash.
- Growing the pager's budget during a load to speed it up: the load took the
  same 73 s and the peak rose to 36.5 GB; reverted.

## Towns and industries on a big map

Counts are a **fixed density per km²**, so they scale with area: a 57 km map
is 5.4 times Megalomaniac and generates about 200 towns and 1,600 industries.
Two multipliers sit in front of the visible density and neither defaults to
1.0: towns are 0.2 per km² times a dropdown factor of {0.2, 0.3, 0.4, 0.5}
applied engine-side (not in the shipped Lua), industries 0.8 per km² times
{0.4, 0.6, 0.8, 1.0}. Missing those overstates every count 1.7 to 3.3 times;
a 57 km map at a hand-patched 0.0367 per km² gave 36 towns, which is exactly
0.0367 times 0.3 times 3,288 km². A fixed multiplier does not hold a count as
the map grows (the scale that keeps Megalomaniac's 36 towns is `604 / area`),
so the plugin's added density levels are named for the map size at which they
reproduce those counts.

Placement never touches Lua. Both kinds go through `RandomLocationFactory`:
N uniform samples, then a four-worker perturbation search on a score that is
the sum of a spacing term (`minDist / nearestDist`, 99,999 when closer), a
slope term and 1,000 times a water or obstacle term; candidates are kept while
every part is below 1. The inner search budget is a constant 200 attempts
(the plugin offers 50 as an experiment). **Runtime industry founding reuses
the same path**, so anything a mod does to placement runs in lockstep on
every peer and must be a pure function of the seed and the heightmap. The
spawner's target is `round(area_km² * targetMaxNumberPerArea)`, it counts
every construction with sim buildings regardless of road connection, and both
of its timers are gated strictly on `N < T`; "Industry density target:
Disabled" removes both timers, and closures still happen.

## Rules that transfer

- **Label every number** measured, derived or guessed, and never let a
  derived size stand in for a measured one: two of the audit's largest
  derived terms were wrong in opposite directions (the material grid ratio,
  the render vertices) until a live process read settled them.
- **Never patch a shared constant**; rewrite the instruction that loads it.
- **Never widen an overflowing multiply** whose result feeds int32 indexing
  elsewhere; shrink the input instead.
- **Anything a peer must agree on stays bit-identical**, proven against the
  original machine code, not "close enough".
- **Profile shares are per thread and include page faults.** Divide a window's
  samples by a full thread's count before calling anything serial, and
  measure warm and cold before promising a speedup.
- **Every optimisation ships off** until an in-game load-time or memory
  measurement on the same map says otherwise; several here are still waiting
  for that measurement.
- **Same-machine tuning does not transfer.** A budget or a reserve that is
  right on the developer's box is wrong on a smaller one; derive it from the
  machine, and rig-test the policy by simulating a smaller machine.

## Measure these first on TPF3

1. The tile size, the base sample spacing and whether a derived
   high-resolution cache is rebuilt on load or persisted; if rebuilt, the
   dirty-set shape of the alignment pass on a full load.
2. The octree root: constant or derived from the map, and what an entity
   beyond it does.
3. Every 32-bit multiply that sizes an area raster, starting with whatever
   "Creating streets" becomes.
4. Whether the per-tile render grids (material indices, vertices) are
   retained after entry, by reading the process, not the code.
5. Autosave duration and size on the largest map the room will allow, against
   the stall and load timeouts.
6. Whether town and industry placement and runtime founding are on one path,
   and whether it reads anything but the seed and the terrain.

## Big maps on TF3

[investigation/TF3_BIGMAPS_PORT_2026-10-01.md](../investigation/TF3_BIGMAPS_PORT_2026-10-01.md)
answers "Measure these first" from TF3 build 40408's executable and
scripts, and lays out the port in stages. In short: TF3 kept TPF2's native
walls nearly byte for byte (the octree root's two tiers, the street
raster's 32-bit multiply, the placement score's int32 squares), but its
size limit is in Lua, `getNumTiles` in the New Game page, and its stock
sizes already reach Gigantomaniac, 112 by 112 tiles or 50 by 250 at 1:5.
The first wall a bigger map meets is memory: Stage 0 measured the terrain
toolkit at 41 maps and 8,428 MB for temperate Gigantomaniac 1:1, 10.25 MB
per km², and an earlier subarctic log at 49 maps and 10,074 MB, 12.25 MB
per km²: three to five times TPF2's.

Whether the project ships big maps is the owner's to decide (PLAN.md, "Big
maps"). Stages 0 and 1 are built, and Stages 2 and 3's opt-in hook patches;
none changes a stock-sized game, and the mod is not packaged with
TPF3-MP.

### Stage 0: measuring TF3

On the stock game, with no patch, by a person or the rig (automation never
starts the game). For each climate, generate Gigantomaniac at 1:1 and at
1:5, then:

1. Read the game log (`stdout.txt`, or the newest
   `userdata/<id>/3493540/local/crash_dump/*.txt`) with
   `cargo run -p tpf3mp-bigmap -- measure <log> [--tiles 112x112]`. It
   prints the terrain toolkit's maps and MB with the pipeline time, checks
   the memory law `(64x+1)(64y+1) * maps * 4 / 10^6` against the size
   (given, or found among the game's own sizes), gives MB per km², and lists
   the stage times (`Place assets`, `Create Industries`, `InitGame`,
   `Enter Game Asynchronously`, `Init game took`) and each save's size and
   time. Do not commit the logs.
2. Read the game's private bytes at the end of generation, after entering
   the world, and after loading the save:
   `(Get-Process TransportFever3).PrivateMemorySize64 / 1MB` in
   PowerShell, or `VmRSS` in `/proc/<pid>/status` under Proton on
   strelka, whose memory leaves room for the large runs. The peak during
   generation, against the toolkit's MB, says whether all its maps are
   alive at once, as on TPF2.
3. Save, and note the save's size and how long it took.

Record each run here:

| climate | size | maps | toolkit MB | MB/km² | pipeline | peak private | after entry | after load | save | evidence |
|---|---|---|---|---|---|---|---|---|---|---|
| subarctic | 56 x 224 (derived from the law) | 49 | 10,074 | 12.25 | 19.4 s | | | | | one log, 2026-09-30 |
| temperate | 112 x 112 (1:1), 822.084 km² | 41 | 8,428 | 10.25 | 11.5 s | 14,210 MB | ~13.4 GB | | 3,276 ms | Stage 0, 2026-10-05, one run |
| temperate | 50 x 250 (1:5), 819.2 km² | 38 | 7,785 | 9.50 | 10.7 s | not clean | ~13.1 GB | | 6,078 ms | Stage 0, 2026-10-05, one run |

**The 2026-10-05 runs.** Stock TF3 build 40408 on Windows 11, on a PC
with 94 GB of memory (the game logs `ramMB=95858`), no Big Maps code: the
game's own New Game page at Gigantomaniac, temperate for both shapes, 1:5
first, then 1:1. The sources are the game log (`stdout.txt`) and a logger
that read the game's private bytes and working set every 5 seconds and
copied the log's generation, terrain and save lines with the time. Raw
numbers:

| | 1:1 | 1:5 |
|---|---|---|
| area (the log's `area=`) | 822.084 km² | 819.2 km² |
| terrain toolkit, first pass | 11 maps, 2,253 MB (pipeline 1.9 s) | 11 maps, 2,261 MB (pipeline 2.4 s) |
| terrain toolkit, full | 41 maps, 8,428 MB (pipeline 11.5 s) | 38 maps, 7,785 MB (pipeline 10.7 s) |
| MB per km² | 10.25 | 9.50 |
| peak private bytes during generation | 14,210 MB at 19:24:40, the sample after the first pass and before the full one's line | not clean: the previous world, about 16 GB, was still resident as generation began; the highest sample was 13,371 MB |
| private bytes in the game, before the save | 12.3 to 13.5 GB, settling at ~13.4 GB | 13.1 to 13.3 GB, ~13.1 GB |
| private bytes after the save | 14.7 GB a minute on | 13.1 to 13.2 GB until the next run |
| `Init game took` | 157.3 s | 134.4 s |
| the game's `estimatedMb` | 18,916 | 18,654 |
| assets | 1,251,140 in 105,948 groups | 1,338,328 in 110,276 groups |
| save time | 3,276 ms | 6,078 ms |
| the log's `Savegame info: size` | 164,177,856 | 159,391,925 |

What they say:

- The peak private bytes, 14,210 MB, exceed the toolkit's 8,428 MB by
  5,782 MB, so the toolkit's maps add to everything else the game holds
  at that point: the memory law `area x MB/km² + game` holds, with the
  game's own share 5.8 GB, not TPF2's 4 GB.
- The gate charges the worse MB per km², 10.25, and 5,800 MB for the game
  (5,782 rounded up). Gigantomaniac 1:1 then expects 14,226 MB, against
  14,210 measured.
- The game's `estimatedMb`, about 18.7 to 18.9 GB, is well above what
  either run used.
- Not measured yet: other climates (the subarctic line above is 12.25 MB
  per km², more than the gate charges), Linux and Proton, sizes bigger
  than stock (the added rows are extrapolated from Gigantomaniac), the
  private bytes after loading the save, and the save's size on disk (the
  log's `Savegame info: size` is recorded above, but not checked against
  the file). One climate, one run per shape, on Windows.

`WorldModel::TF3_BUILD_40408` charges these, MEASURED, from one place,
`crates/tpf3mp-bigmap/src/memory_gate.rs`, which lists the steps to
recalibrate the memory gate from this table as more runs come in.

### Stage 1: sizes up to 176 tiles, no native patch

`mod/tpf3mp_bigmap_1` adds four rows to the New Game size dropdown, after
the game's own:

| row | square | km² | each ratio, 1:1 to 1:5, in tiles | expected peak |
|---|---|---|---|---|
| Big 32.8 km | 128 x 128 | 1,074 | 128², 90 x 180, 74 x 222, 64 x 250, 58 x 250 | 16,836 MB |
| Big 36.9 km | 144 x 144 | 1,359 | 144², 102 x 204, 84 x 250, 72 x 250, 64 x 250 | 19,907 MB |
| Big 41.0 km | 160 x 160 | 1,678 | 160², 114 x 228, 92 x 250, 80 x 250, 72 x 250 | 23,260 MB |
| Big 45.1 km | 176 x 176 | 2,030 | 176², 124 x 248, 102 x 250, 88 x 250, 78 x 250 | 26,608 MB |

The expected peak is the ladder's `peakMb`: Stage 0's law, 10.25 MB per
km² plus 5,800 MB, for the row's largest shape (1:3 for 32.8 and 36.9 km,
1:2 for 41.0 km, the square for 45.1 km). Before Stage 0 the gate charged
12.25 MB per km² plus 4,096 MB: 17,285, 20,956, 24,963 and 28,965 MB.

Which rows each machine is offered, the peak held against its physical
memory with no margin added (the hook reports the memory Windows sees,
often a little under the round figure):

| memory | rows offered | why |
|---|---|---|
| 8 GB | none | the smallest row needs 16,836 MB; stock Gigantomaniac alone peaked at 14,210 MB |
| 16 GB | none | 16,836 MB is past 16,384 MB |
| 32 GB | all four | the largest needs 26,608 MB |
| 64 GB | all four | |

The same tiers as before Stage 0: 16 GB was offered none and 32 GB all
four then too. The new law moves the cut between them; 20 GB now gets
Big 32.8 and 36.9 km (one row before), and 24 GB three (two before).

Each shape stays inside every wall stock TF3 has, so nothing native is
patched and the world is safe to load in any game, with or without big
maps, in a room or not: no axis is longer than stock's own 250 tiles (the
octree root keeps the 768 m margin stock maps have), and the street
raster's one-metre cells stay below 2^31 (180 tiles square is that wall).
A ratio keeps about the square's area until the long side reaches 250
tiles, which then caps it. `crates/tpf3mp-bigmap/tpf3mp_bigmap.stage1.toml`
holds these rows; the mod's `ladder.lua` is generated from it (`cargo run
-p tpf3mp-bigmap -- --config crates/tpf3mp-bigmap/tpf3mp_bigmap.stage1.toml
lua`), and a test fails if either changed without the other.

**How.** The New Game page is the game's script
`gui/menu/new_game_or_map_settings_page.tl`, and a mod cannot replace a
menu file, so the hook's loader redirect, which already serves the main
page ([LOBBY.md](LOBBY.md), "How it works"), serves the mod's copy of it.
The copy is the game's file plus five marked blocks (LOBBY.md, "The mod's
copies"); `cargo run -p tpf3mp-bigmap -- page <the game's file>` makes it.
Its logic is plain Lua in the mod (`scripts/tpf3mp_bigmap/menu.lua`):

- The size dropdown lists the game's rows, then the added ones. Picking an
  added row sets the game's `map.size` to its largest size (Gigantomaniac
  with experimental sizes, Very Large without) and keeps the pick in the
  page. Every reader of `map.size`, the save included, sees a value the
  game knows; the save's real size is its terrain's. A save of an added
  size therefore shows as Gigantomaniac in the load page, with the same
  "not supported" mark the game gives stock Gigantomaniac saves.
- `getNumTiles` answers an added row from the ladder, at the ratio picked,
  short side first as the game's own shapes are; the preview and Start
  Game use it as they use the game's sizes.
- A row is offered only if its expected generation peak (the ladder's
  `peakMb`: 10.25 MB per km² plus 5,800 MB, measured by Stage 0, from
  `memory_gate.rs`) fits the machine's physical
  memory, which the hook reports. A line under the dropdown says which
  rows are hidden and why. With the memory unknown, no row is offered.
  `TPF3MP_BIGMAP_MEMORY_GATE=0` turns the check off: every size and ratio
  the walls allow is offered, with a line saying a size may not fit.
- If the mod's scripts do not load, the page offers the game's sizes only;
  if the copy does not load, the hook serves the game's file.

**Town and industry density.** TF3 counts towns and industries as a
density per km² (base `mod.script.tl`: towns 0.2 per km² times the Town
Density slider's `{0.2, 0.3, 0.4, 0.65, 1.0}`, industries 0.8 per km²
times the Industry Density slider's `{2/3, 5/6, 1, 6/5, 3/2}`, both from
`difficulty_util.getScale`), so a big map at the stock sliders has 1.3 to
2.5 times Gigantomaniac's towns and industries. The page's Town Density
and Industry Density sliders get one more level per ladder row after the
game's five, "-" to "----" (one dash per row, "----" the largest: Gigantomaniac's count at that size): Medium times the row's
`densityScale`, `(112 / tiles)²`, which gives that row's square the counts
stock Gigantomaniac 1:1 has at Medium. The industry slider sets the
runtime target (`targetIndustryDensity`) with the start density, as the
game's own slider does, so the game does not found industries back up to
the stock count. The level is stored where the game stores its own
(`modParams[""]`); the hook wraps `difficulty_util` as it loads
(`menu.extendDifficulty`), so the preview the world is generated from and
the base mod's run script, which sets the industry target on every load,
both read it. Every row of the ladder has a level, offered on the machine
or not, so a level is the same factor everywhere.

**A save made at an added density level needs the mod in every game that
loads it.** Without it the hook leaves `getScale` the game's own, which
answers a level past five with 1.0 (Medium), so that game's runtime
industry target differs from the host's: in a room that is a desync
waiting for the first industry founding. Not yet enforced; until it is, a
room on such a world needs `tpf3mp_bigmap_1` in every game.

**Installing.** The mod is not part of the TPF3-MP package. Copy
`mod/tpf3mp_bigmap_1` next to `tpf3mp_1` in the game's mods folder, and
start the game from the launcher, so that the hook serves the page. A game
without the mod, or started by Steam, has the stock sizes. Only the
machine that creates a world needs it: a room's other games load the
world from its save, and stage 1's worlds need nothing from them, unless
the world was made at an added density level (above).

**Still to test in the game**, by a person:

- the page loads from the mod (`[tpf3mp] big maps: ... is served from
  tpf3mp_bigmap_1::/...` and `... is in effect` in the game log) and the
  game's Teal accepts the copy;
- whether the mod has to be active for the game to find its files at the
  menu, as LOBBY.md says `tpf3mp_1` must be;
- the four rows show (or are hidden with the reason on a smaller
  machine), and picking one regenerates the preview at its size;
- generate 128² and 176², and one 1:5 (58 x 250); watch the log for
  `Duplicate base nodes`, and fill in stage 0's table for each;
- a room of two games (Sandboxie) on a 176² world through the scenario
  runner (`tools/scenarios/roads.json`, `rail.json`), the checkpoints
  agreeing.

### Longer ratios: 1:6 to 1:10

The New Game page's ratio dropdown gets 1:6, 1:7, 1:8, 1:9 and 1:10 after
the game's own 1:1 to 1:5, for the game's sizes and the added rows alike:
long edges are reached by ratio, not by squares nobody's memory holds
(investigation/TF3_BIGMAPS_256KM_2026-10-05.md §6). Not tested in the game
yet.

- **Shapes.** A ratio 1:k keeps about the 1:1 square's area: the short
  side is the square's edge over √k, rounded to an even count, and the
  long side k times that, never capped (`ladder::long_shape`). Gigantomaniac
  (112) gives 46 x 276 at 1:6 up to 36 x 360 at 1:10; Megalomaniac (96)
  40 x 240 at 1:6.
- **Gated as the rows are.** `ladder.lua` holds each added row's ten
  shapes and, under `stock`, each game size's five added ones (keyed by its
  1:1 edge, which the page asks the game's `getNumTiles` for), `false`
  where the settings cannot build one; `peakMb` covers the largest
  buildable one. The dropdown lists only the ratios the picked size can be
  built at on this machine, and a line under it says why the others are
  missing. At Stage 1's walls (no axis past 250 tiles) that is 1:6 to 1:10
  for Tiny to Large and Very Large, 1:6 to 1:9 for Huge, 1:6 for
  Megalomaniac, and none for Gigantomaniac or the added rows; with the
  Stage 2 and 3 patches the same ratios open further (the settings files
  there are not wired to the mod yet).
- **The game's `map.format` stays 1:5** when an added ratio is picked, as
  `map.size` stays Gigantomaniac for an added row: the save and every
  reader of `map.format` see a value the game knows (`getNumTiles` clamps
  the index to the game's list). The pick is the page's state. A save at
  1:7 shows as 1:5 in the load page; its real size is its terrain's.
- **Without the game's five ratios** (no experimental sizes: 1:1 to 1:3),
  or without the mod's scripts, the dropdown is the game's own.
- **The preview** (`builtin.MapPreviewComp`) generates the whole map at the
  shape picked, so the memory gate covers it too.

**Still to test in the game**, by a person: with experimental sizes, pick
Huge and see 1:6 to 1:9 offered with the 1:10 line; pick 1:8 and see the
preview regenerate at 28 x 224; start the game and check the log's
`area=` and the world's size; save and load, and see the load page show
1:5; pick Gigantomaniac and see no added ratio, with the reason; switch
from 1:8 on Huge to Gigantomaniac and see the dropdown fall back to 1:5.
The page copy is Teal checked only by the game itself: if it does not
load, the hook serves the game's page and the log says why.

### Stage 2: the octree root at depth 11, up to 512 tiles

Built in the hook, not tested in the game yet
(investigation/TF3_BIGMAPS_256KM_2026-10-05.md, §1 and §8). With
`TPF3MP_BIGMAP_OCTREE=11` in the game's environment (the launcher's
reaches it), the hook splices the instruction after each of the two calls
of `OctreeSystem::Resize` that set the large-map root, on the new-game
path (`0x244b8b`) and the load path (`0x20267d`). When an axis of the
world being allocated is over 256 tiles, it calls the game's own `Resize`
again with depth 11 and ±65,536 m: tpf2-bigmap's depth-11 root, without
patching the shared 32768.0f or any game instruction but the splice's
jump. Depth 11 keeps the 128 m leaves, and its node ids stay inside the
stock 32-bit scheme and the renderer's level decoder.

- **Derived from the world.** A world of 256 tiles or less is never
  touched, so on every stock-sized and Stage 1 world a game with the
  setting and one without run the same code. A world over 512 tiles needs
  depth 12, which is not built: it keeps the game's root, and `hook.log`
  says the world will lose entities.
- **Fails closed.** The three profile targets are optional; each site's
  27 bytes (the constant load, the owner load, `mov edx,10`, the call of
  `Resize` and the instruction after it) are checked before either site is
  spliced, and both are spliced or neither. `hook.log` says `big maps:
  octree: installed` or `off` with the reason, and one line each time a
  world gets depth 11.
- **Every game of a room** on a world over 256 tiles needs the setting.
  The room does not check it yet: a game without it loads such a world
  with the stock root and damages it.
- **Tested** against the game's own code: `Resize` and both sites,
  relocated from the executable into the test process, run with the
  splices installed (`crates/tpf3mp-hook/src/bigmap/octree.rs`, with
  `TPF3MP_TF3_EXE`): 128 to 256 tiles keep depth 10 and ±32,768 m, 258 to
  512 get depth 11 and ±65,536 m, 1,000 keeps the game's, a site moved by
  a byte installs nothing, and detached the code is the game's again.
  `tf3_static_proof.rs` pins the three targets.

The bigmap crate knows the new ceiling: `WorldModel::TF3_BUILD_40408`'s
`octree_max_depth` was 11 (12 since Stage 4) and its patches are derived
from the world, so `ceilings::check` keeps the shallowest depth that covers
the map, up to the one `octree_depth` asks, and blocks anything past it.
`crates/tpf3mp-bigmap/tpf3mp_bigmap.stage2.toml` is Stage 1's rows with
`max_tiles = 512` and `octree_depth = 11` (its longest shape, 176's 1:5,
is 390 x 78); the mod's ladder stays Stage 1's until the root is seen
working in the game.

**Still to test in the game**, by a person, with the setting on in every
game:

- generate 1:5 at 60 x 300 (77 km long): `hook.log` shows the octree line
  for the new game, and again after a save and reload; the game log has no
  `Duplicate base nodes`; towns past 32.8 km from the centre grow streets
  and population;
- with the setting off, stock Gigantomaniac and a Stage 1 176² world
  behave as before, with no octree line;
- a room of two games (Sandboxie) on the 60 x 300 world through
  `tools/scenarios/roads.json` and `rail.json`, the checkpoints agreeing.

### Stage 3: the street raster and placement spacing

Built in the hook, not tested in the game yet
(investigation/TF3_BIGMAPS_256KM_2026-10-05.md, §2, §3 and §8). Both
opt-in, both derived from the world:

- **The street raster** (`TPF3MP_BIGMAP_STREET_RASTER=1`). The hook
  detours `Obstacle::Obstacle` (`0x8cea50`), which every street, town
  connection and industry raster goes through. Where the cell its caller
  passes would give more than 2³¹ − 1 cells (stock code aborts there:
  180 tiles square, 1000 x 32 long), the cell is doubled until the count
  fits; a raster that fits is never touched, so every world a stock game
  can generate keeps its 1 m cells. Unlike tpf2-bigmap's 1.5-billion
  budget, Stage 1's 176² keeps 1 m. The cell grows in powers of two
  because the raster's fill scales by `extent / (n − 1)` while lookups
  divide by the cell: they agree exactly only when the cell divides the
  extent, which 2, 4 and 8 m do for every whole-tile edge. 300² and up to
  1000 x 130 get 2 m, 1000 x 136 to 1000 x 186 and 512² get 4 m. The
  hook checks the constructor's 135-byte body (the counts and the 32-bit
  multiply) before it detours it.
- **Placement spacing** (`TPF3MP_BIGMAP_PLACEMENT=1`). The hook replaces
  the spacing score (`0x8d31f0`) with the same arithmetic, squares in 64
  bits saturated at `INT_MAX`. Where no square overflows it gives the
  game's scores to the bit, so it changes nothing on any map whose pairs
  stay within 185 km (every map up to 724 tiles long; with depth 11, only
  a 512² world's corners are farther). It checks both int32 squarings are
  where they were read before it replaces the score.
- **Tested against the game's own code**, relocated from the executable
  (`crates/tpf3mp-hook/src/bigmap/raster.rs`, `placement.rs`, with
  `TPF3MP_TF3_EXE`): the constructor's counts and its wrapped size equal
  the model for 20,000 random boxes and cells; through the detour,
  Gigantomaniac and 176² keep 1 m, 300² gets 2 m and 1000 x 186 4 m, each
  sized below 2³¹; the game's score and the replacement agree bit for bit
  on 3,000 random sets (over 30,000 scores, duplicates and exclusions
  included) inside the int32 range; past it the game's score is NaN and
  the replacement's equals a 128-bit reference.
- The bigmap crate models both: on TF3 `street_raster` grows the cell only
  past the wall and in powers of two, and a new ceiling,
  `PlacementSpacing`, blocks a map whose corners are more than 185 km apart
  unless `limits.placement_distance` is set.
  `crates/tpf3mp-bigmap/tpf3mp_bigmap.stage3.toml` adds square rows up to
  384 tiles (98.3 km) on top of Stage 2; every shape is buildable on TF3
  with the three patches. Not wired to the mod.

**Still to test in the game**, by a person, with the settings on in every
game:

- generate the largest square past 180² the machine's memory allows (200²
  needs 32 GB; 256² about 50 GB): `hook.log` shows the street raster line
  at 2 m, generation does not abort, towns get streets, industries are
  connected;
- with the settings off, stock and Stage 1 worlds generate as before, with
  no raster line;
- a room of two games on a world past 180² founding industries (each
  founding builds a raster) through the scenario runner, the checkpoints
  agreeing;
- placement: nothing to see until a map is longer than 724 tiles, which
  needs depth 12.

### Stage 4: depth 12, the 256 km edge

Built in the hook, not tested in the game yet
(investigation/TF3_BIGMAPS_256KM_2026-10-05.md, "Depth 12, built"). With
`TPF3MP_BIGMAP_OCTREE=12` a world with an axis over 512 tiles gets the
octree root at depth 12, ±131,072 m (1,024 tiles, 262 km); 257 to 512
tiles keep depth 11 and 256 or less the game's root.

- **Why more than a root.** Depth 12's deepest level, 11, cannot be
  numbered by the game's `8·id + 1 + octant` in 32 bits, and the
  renderer's level decoder would loop forever on such ids. The hook
  numbers level 11 by its 128 m cell, `0x50000000 + zslot·2²² + y·2¹¹ + x`
  (128 height slots, ±8,192 m), in a detour of the descent, and splices
  the decoder so these ids read as level 11. tpf2-bigmap drew deep ids from
  counters; a counter's ids depend on the process's history, so in a room
  two games would number differently. A cell id is the same in every game.
- **Derived from the world.** The descent reads the depth from its own
  tree: on any tree shallower than 12 both pieces pass everything through.
- **Fails closed.** Three more optional targets (the descent, its
  starter, the decoder site), the descent's body checked where the ids
  depend on it, the starter checked to call it; the decoder and descent go
  in first and the roots last, and any failure installs nothing.
- **Tested against the game's own code**: the descent, its starter and the
  decoder relocated from the executable, with the ECS calls answered by a
  model of the component store, build a depth-12 tree from 3,000 objects
  over a 1000 x 186 map: ids unique and positive, levels 0 to 11, 128 m
  cells, every node's level read back by the decoder, the same ids when
  the objects come in reverse order, and depth 11 unchanged.
- **Shapes.** `crates/tpf3mp-bigmap/tpf3mp_bigmap.stage4.toml` (not wired
  to the mod): "Big 80.9 km" (316 tiles) at 1:10 is 1000 x 100, 256 x 25.6
  km, about 73 GB; "Big 104.4 km" at 1:6 is 996 x 166, about 118 GB. At
  ratios up to 1:10 no 256 km map fits 32 or 64 GB.

**The in-game test**, by a person, on a machine with 80 GB or more: the
three switches on (`TPF3MP_BIGMAP_OCTREE=12`,
`TPF3MP_BIGMAP_STREET_RASTER=1`, `TPF3MP_BIGMAP_PLACEMENT=1`), the ladder
generated locally from stage 4 (`cargo run -p tpf3mp-bigmap -- --config
crates/tpf3mp-bigmap/tpf3mp_bigmap.stage4.toml lua >
mod/tpf3mp_bigmap_1/content/scripts/tpf3mp_bigmap/ladder.lua`, not
committed), then New Game, "Big 80.9 km", ratio 1:10 (100 x 1000).
`hook.log` shows the octree's `installed (... depth 12 (±131,072 m, deep
ids by cell) over 512 ...)`, then `bigmap::octree_root_init: a world of
100 x 1000 tiles; root at depth 12, ±131072 m`, `octree depth 12: first
level-11 node numbered 0x5…` or `0x6…`, the street raster at 2 m and
placement's first scores. No `Duplicate base nodes`, towns with streets at
both ends, no freeze when panning the whole map, the far ends drawing as
the centre does; save and reload show `octree_root_load ... depth 12`.
The full list is in the investigation note.

### What the simulation costs on a big map

A 100 x 1000 world's simulation update took about 260 ms where a stock
one took 70 (investigation/TF3_BIGMAP_SIM_COST_2026-10-05.md). The
investigation splits it, by static analysis, between the emission grids
(16 m cells: 25.6 M cells a grid, about 30 to 60 ms an update) and the
parcel-collision walk of each applied proposal on the main thread. To
replace those estimates with numbers, the hook times the systems in the
game: the `perf: sim` line every 10 s ([HOOKS.md](HOOKS.md), "The game's
own systems: the `perf: sim` line"), with `emission-grid`,
`emission-emitters`, `towns` and `parcel-collision` and the walks' query
boxes. It changes nothing the game computes. The faster component lookup
("The faster component lookup" there) is on by default and exact; it
trims the lookup's per-call overhead, not its cache misses.

### The rest of the prototype

`crates/tpf3mp-bigmap` also carries the rest of Big Maps' features, as far
as they go before their TF3 sites are patched (stages 2 to 4 of the
investigation):

| Big Maps on TPF2 | the prototype |
|---|---|
| The added size rows (`add_size_rows`) | `ladder`: the rows and their 1:k shapes; on TF3, the mod's page copy above (`page`, `mod_data`). |
| The street raster's 32-bit wall past 180 tiles (`street_raster`, `cell_budget_millions`) | `ceilings`: the cell count at the stock cell, and the cell the budget needs. TF3: the hook's detour of `0x8cea50`, only past the wall, in powers of two (Stage 3). |
| The octree root's 32,768 m wall past 256 tiles (`octree`, `octree_depth` 11 to 13) | `ceilings`: the map's half-extent against the root at the depth in use. TF3: `octree_max_depth` 12, the hook's root splice at `0x244b8b` and `0x20267d` (Stage 2) and the deep ids and decoder splice (Stage 4). |
| The heightmap's 32-bit pixel count (derived, about 722 tiles) | `ceilings`: refused, since no setting passes it. |
| The memory law | `ceilings` and `world`: the expected peak for every size, before it is generated; `measure` checks it against a log. |
| Density levels, placement attempts | `config` and `features`: settings that change the simulation. |
| Byte-verified sites, each feature off with a log line when its sites are missing | `features`: each feature names the profile targets it needs. In a room, a missing feature that changes the simulation refuses the room instead of degrading. |
| "Every peer needs the same `octree_depth`" | `terms`: the size, octree depth, street cell, placement budget and density levels, with a fingerprint the room compares, and the names of what differs. |
| The minimap | [MINIMAP.md](MINIMAP.md): a script mod on TF3. |
| Terrain cache compression, dedup, lazy zeroing, the SSE2 terrain paths, generator buffers, faster saves | Not in the prototype: each rests on a TPF2 structure still to be found in TF3. |

`cargo run -p tpf3mp-bigmap -- ladder` prints the ladder under a settings
file (`--config`; `--world tpf2` for TPF2's numbers), `check 320x320` what
one size costs and needs, and `lua` the mod's `ladder.lua`.

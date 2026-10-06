# TF3 `EmissionEmitterSystem::Update2`: the emitters' splat, exactly (2026-10-06)

Build 40408, `TransportFever3.exe` SHA-256 `de1daad3…f23ef2`. Static analysis
and offline tests only: no game process was started, attached or touched.
Addresses are VAs (image base 0x140000000). Labels as in
[TF3_BIGMAP_SIM_COST_2026-10-05.md](TF3_BIGMAP_SIM_COST_2026-10-05.md):
**CONFIRMED-static**, **CONFIRMED-test** (the game's own code, run from the
executable, agrees bit for bit), **DERIVED**, **GUESS**.

Measured in game before this work (hook.log `perf: sim`, a ~78 x 390-tile
world, 1,250 x 6,242 cells per grid): `emission-emitters` ~5.3 to 5.6 ms per
simulation update, next to `emission-grid` 6.3 ms (already fused).

## 1. What `Update2` does

`ecs::EmissionEmitterSystem::Update2(this, engine, int count, float dt)`
(`0x140aa51c0`, vtable `0x1436fc430` slot 12; slot 11 `0x140aa5920` sets
`[this+8]` to the node list's vector of `{entity, int component index}`,
`[this+0x10]` to the component storage's vector of 36-byte
`ecs::component::EmissionEmitter`, calls slot 12 with the node count, and
skips it when the list is empty). CONFIRMED-static throughout:

1. For both grid entities (`[[this+0x18]+8]`: noise, then pollution;
   `0x140aa8c70`), the `EmissionGrid` component (`GetComponentTypeIndex`,
   `GetComponentDataIndex`, `0x140144920`) and a factor
   `0x140aa8ba0 = ((gx + gy) * 0.5) / logf(spread)` (`spread` = the grid
   system's `+0x50`, Diffuse's `a`). It asserts that both components have the
   same grid point size, `X0`, `Y0`, width and height (`+0x10`, `+0`, `+4`,
   `+8`, `+0xc`).
2. `qx = width / 4`, `qy = height / 4` (`cdq; and edx,3; add; sar 2`:
   truncating). The 4 x 4 regions are `[INT_MIN, X0+qx)`, `[X0+qx, X0+2qx)`,
   `[X0+2qx, X0+3qx)`, `[X0+3qx, INT_MAX)` along x (the same along y);
   `0x140aa4ab0` builds them.
3. Sixteen buckets at `[this+0x20]` (`16 x 0x50`, one per region): each
   holds a vector of per-thread `vector<int>` (resized to the pool's thread
   count) and the `LoopResults` chunk records; all are cleared every call.
   They are scratch: only the constructor, the destructor and `Update2`
   touch `[this+0x20]` (the vtable has slots 0, 2, 11, 12; the others are
   the no-op `0x140055b50`).
4. **Lambda_1** (`0x140aa4310`, dispatched by `LoopImpl` `0x140aa3e70` in
   chunks of at least 1,024 when `count > 0`) buckets emitter `i`:
   `r = r_noise < r_pollution ? r_pollution : r_noise` (`vcmpltss` +
   `vblendvps`); `lo = cell(px - r, py - r)` and, if `r > 0`,
   `hi = cell(r + px, r + py)`, else `hi = lo + (1, 1)`, where
   `cell(p) = cvttss2si(floor(p / gp - 0.5))` (`0x140ba1ba0`, the noise
   grid's `gp`). Regions `clamp((lo.x - X0) / qx, 0, 3) ..= clamp((hi.x -
   X0) / qx, 0, 3)` (`idiv`, truncating) and likewise in y; `i` is pushed
   into every one of those buckets, into the vector of the thread running
   it, with a chunk record (`0x140aa4da0`: chunk index from TLS, thread,
   range).
5. **Lambda_2**: 32 tasks (`0x140aa4ab0` inline with one pool thread, else
   `LoopImpl` `0x140aa33a0`, then `Update2` waits on the futures); task `t`
   is grid `t / 16`, region `t % 16` (x index `(t%16) / 4`, y index
   `(t%16) % 4`). It builds an iterator over the bucket (`0x140aa40b0`:
   the chunk records of all threads, **sorted by chunk index**,
   `0x1402e4660`) and calls the body `0x140aa4650` through a
   `std::function` (`0x140aa5c20`). The walk is therefore in ascending node
   order, whichever threads bucketed which chunk (CONFIRMED-test: the
   game's code run on 4 and 16 threads gives the one-thread result).

### 1.1 Lambda_2's body, per emitter (`0x140aa4650`)

The emitter's nine floats: `[0] x, [1] y, [2] a distance, [3] noise
radius, [4] pollution radius, [5] noise power, [6] -, [7] pollution power,
[8] -` (the debug printer's field names: `noisePower`, `pollutionPower`,
`radius`, `pollutionRadius`). For grid `k`:

```
if 0 > dist: nothing                                   (vcomiss 0, dist; ja)
m     = |power| > 1e-7 ? |power| : 1e-7                (vmaxss; 1e-7f at 0x1436f3950)
reach = (logf(1e-7 / m) * factor[k]) * 4.0             (logf imported, thunk 0x1431873c5)
share = 0;  if dist >= 0 and reach > 0:
            s = 1 - dist / reach;  share = 0 > s ? 0 : (1 < s ? 1 : s)   (vminss 1, s)
v     = (share * dt) * power
if 1e-7 >= |v|: nothing                                (jae)
r     = k == 0 ? [3] : [4]
r > 0 ? radius insert 0x140ba2230 : point insert 0x140ba1e40,
        both clipped to the task's region [xmin,xmax) x [ymin,ymax)
```

`reach` is where the diffusion has brought `power` down to 1e-7
(`4 * gp * log_a(1e-7 / power)`, DERIVED); the share fades the emitter
linearly with `dist` over that reach.

### 1.2 The inserts

- **Point** (`0x140ba1e40`): `sx = px / gx - 0.5`, `sy = py / gy - 0.5`,
  `x0 = cvtt(floor(sx))`, `y0 = cvtt(floor(sy))`; enters the region only if
  `x0 + 1 >= xmin && x0 < xmax && y0 >= ymin && y0 + 1 < ymax`; then
  `a = 1 - (sx - x0)`, `b = 1 - (sy - y0)` and the corners (below) with `v`.
- **Radius** (`0x140ba2230`): `inv = 1 / (gx * gy)`; rows `dy = -r`,
  `dy += gy` while `r >= dy`; in each, `dx = -r`, `dx += gx` while
  `r >= dx`; a sample where `r*r >= dx*dx + dy*dy`, at
  `sx = (dx + px) / gx - 0.5`, `sy = (py + dy) / gy - 0.5`, the same region
  test as the point, then the corners with `inv * v`. The `dx` sequence is
  the same in every row, and `sx` depends on `dx` only, `sy` on `dy` only.
- **Corners** (`0x140ba1f40`), in this order, each only if the cell is in
  the task's region and an inner cell (`gx0 < x < gx0 + w - 1`, the same in
  y), each `c = max(c + weight, 0)` (`vmaxss`: NaN and `-0` become `+0`):
  `(x0, y0)` `(a*b)*v`; `(x0, y0+1)` `((1-b)*a)*v`; `(x0+1, y0)`
  `((1-a)*b)*v`; `(x0+1, y0+1)` `((1-a)*(1-b))*v`. It asserts that at least
  one corner was in the region (always true after the entry test).

### 1.3 Two quirks the result depends on (CONFIRMED-test)

- **Region rows drop samples.** The entry test asks `y0 >= ymin` and
  `y0 + 1 < ymax`, so a sample whose two rows straddle a region boundary in
  y (`y0 = Y0 + k*qy - 1`, k = 1, 2, 3) enters neither region: it adds
  nothing. In x the test is `x0 + 1 >= xmin && x0 < xmax`, so a straddling
  sample enters both regions and each adds its own column.
- **Buckets drop corners.** A radius emitter is bucketed from `cell(p - r)`
  to `cell(p + r)` (the larger radius); its last column's `x0 + 1` corner
  can fall in the next region, which it is not bucketed in: dropped. A
  point is bucketed to `cell(p) + 1`, so its corners always find their
  region.

The tests catch both (removing either from the model fails the comparison
at once), and a one-ulp change of order.

## 2. Where the time goes

Per emitter and grid: one `logf`, four to six divisions, a `std::function`
call per region it is bucketed in (an emitter straddling regions is walked
in each, all of its samples clipped per region), and per sample of a
radius emitter two divisions, two floors, a call and four branchy corner
inserts. Bucketing costs four `idiv`, two `cell` calls, a TLS read and a
push per region. The walk reads the bucket's indices, then the node, then
the 36 bytes, in an order unrelated to the grid: on a big world nearly
every emitter is a cache miss, and so is every cell it adds to (the grids
are 31 MB each here, 102 MB on 100 x 1000 tiles). The 32 tasks are the
4 x 4 regions of each grid, so towns concentrated in a few regions leave
most of the pool idle.

Offline, single thread, in the thread's own cycles (the game's code from
the executable, 78 x 390 tiles, emitters in towns): **~1,800 to 3,400
cycles per emitter** (both grids), the larger with more radius emitters
(DERIVED from `emitters_speed`, below).

## 3. What was built (`crates/tpf3mp-hook/src/emitters`)

The same additions, reorganised by **row bands** (docs/HOOKS.md, "The fast
emitters"):

- **Records** (`splat::record`, once per emitter and grid): the value and
  the reach exactly as lambda_2's body; for a point its four corners; for
  a radius its columns (`dx`, `sx`, `x0`, `a`, `dx*dx`) and rows (`dy`, `sy`,
  `y0`, `b`, `dy*dy`), each with **which of its two cells the game keeps**.
  The keep test is separable: a corner `(cx, cy)` is added iff some bucketed
  region contains `cx` and lets the sample in along x, some bucketed region
  contains `cy` and lets it in along y (the quirks above), and the cell is
  inner. Because the regions split the plane (each non-empty, as
  `bands::covered` requires; else the game's way), a cell is added to in one
  region at most, as in the game.
- **Order.** Within a region the game adds in ascending node order, sample
  rows outer, columns inner, corners in order. A cell lies in one region,
  so its additions are exactly the global (node, sample, corner) order
  restricted to it. The band splat walks, in every band, the records in
  node order and adds only the band's rows: the same additions, the same
  order, cell by cell. Noise and pollution are different buffers, so their
  relative order is free.
- **Small records** (a point; a radius of at most 16 samples) keep their
  additions as `(x, y, value)` in the game's order; a band adds those in
  its rows. **Lattices** (larger radii) are applied a row of samples at a
  time; with consecutive columns (the usual case) and at most 64 of them,
  eight cells per AVX instruction: within one row of samples a cell
  receives at most sample t-1's third (or fourth) corner then sample t's
  first (or second), so each lane does `max(c3 + c, 0)` then
  `max(c1 + that, 0)` where the game keeps them (`_CMP_GE_OQ` for the
  radius test, `vmaxps` with the same operand order as `vmaxss`).
- **Threads.** Phase 1, chunks of emitters (8 per thread) in parallel:
  records, and per band and grid the indices of the records that reach it.
  Phase 2, 16 bands per thread, heaviest first, each on any thread. The
  hook's 16 threads (shared with the fused grid; the game's pool is idle
  while `Update2` waits).
- **Kept records.** A chunk whose nodes and the seven floats the update
  reads of each of its emitters are bit for bit the last update's (in the
  same frame: origin, quarters, `dt`, grid point sizes, factors, shape,
  bands, `logf`) keeps its records: they are a function of those inputs.
  Phase 1 then costs a compare (~30 cycles per emitter).

Arithmetic: every addition is the game's single-precision operation
sequence (no FMA, no reassociation; `1 - x` and products in the game's
grouping); outputs pass `max(., 0)`, which maps any NaN to `+0`, so no NaN
payload can reach a cell. `logf` is the game's import, called through its
own slot. MXCSR: the default on the simulation thread and every worker,
else the game's way.

## 4. Evidence (CONFIRMED-test)

`emitters::original_tests` maps the whole executable into the test
(`emitters::mapped`: every section at its RVA, base relocations applied,
the C runtime's imports resolved, everything else trapped) and runs the
game's own `Update2` (stubs: the thread pool, the ECS lookups, the TLS
indices; for more than one thread, the two `LoopImpl` dispatchers run the
game's lambdas on the test's threads, in chunks, with TLS thread and chunk
indices as the game's pool sets them):

- 200 random worlds (4 x 4 to 130 x 66 and 80 x 400 cells; 0 to 4,000
  emitters; grid point sizes 8, 12.5, 16, 32; origins off the quarter
  lattice; zero, negative and NaN radii; zero, negative, tiny, NaN and
  infinite powers; positions on cell edges and centres; distances below
  zero and past the reach; grids with zeros, negative zeros, negatives and
  denormals; `dt` 0.2 and 0.4), four updates each with some emitters
  changing: both grids bit-identical, with 1, 3, 17, 64 and 400 bands, 1 to
  4 threads, with and without AVX lanes, records kept between updates;
- the hook installed in the mapped `Update2` (its three calls redirected)
  against the unhooked one, 8 updates on six worlds, one and four pool
  threads (inline and pool paths): identical; the first three updates
  checked, the rest banded;
- a non-default MXCSR runs the game's way (lambda_1 replayed from the
  copy of its captures, then lambda_2): identical;
- a sabotaged self-check turns the band splat off and leaves the game's
  result;
- the recorded hashes, the three call sites, the `logf` thunk and its
  slot match the executable; and the static proof pins the call sites.

## 5. Offline speed (`emitters_speed`)

78 x 390 tiles (1,250 x 6,242 cells), emitters in towns (55% points, 35%
radius up to 16 to 64 m, 10% large radii), Ryzen 9 9950X3D, with three
Transport Fever 3 instances running on the same PC (so the multithreaded
numbers swing by up to 2x between runs):

| emitters | one thread, every emitter changed | one thread, none changed | 16 threads, 5% changing each update |
|---|---|---|---|
| 20,000 (radius to 24 m) | 1.6x | 2.1x | 1.3x |
| 100,000 (to 24 m) | 2.4x | 2.6x | 1.7x |
| 100,000 (to 64 m) | 2.4x | 3.6x | 2.5x |
| 300,000 (to 16 m) | 2.2x | 2.9x | 1.5x |

(one thread: the thread's cycles, the game's `Update2` with one pool
thread against the band splat; 16 threads: wall time, best of nine.)

What is not known: how many emitters a real world has, how large their
radii are, and how many change from one update to the next (the `dist`
field and street powers may change often). The hook's summary line says
all three (docs/HOOKS.md).

## 6. Not built

- Expanding every record into its additions (a flat `(cell, value)` list
  per band): exact and branch-free, but a radius record of 100 m is ~500
  additions per grid, which costs more memory traffic than the lattice.
- Keeping records per emitter (rather than per chunk): tried; copying the
  unchanged emitters' records into a rebuilt chunk costs more than making
  them again when changes are spread over every chunk.

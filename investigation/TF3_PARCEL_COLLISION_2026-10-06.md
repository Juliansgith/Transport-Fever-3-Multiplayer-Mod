# TF3 parcel collision walk: exactly what it does, and the probe (2026-10-06)

Build 40408 (`TransportFever3.exe` SHA-256 `de1daad3…f23ef2`). Static analysis
only, read from the executable; no game process was started, attached or
touched. Addresses are VAs (image base 0x140000000). Follows
[TF3_BIGMAP_SIM_COST_2026-10-05.md](TF3_BIGMAP_SIM_COST_2026-10-05.md) §3.2
and §7.3. Labels as there: **CONFIRMED-static**, **DERIVED**, **GUESS**.

Measured in game (hook.log `perf: sim`): `parcel-collision` 85 calls per
10 s at about 11 ms each (about 7.5 ms per update), union mean 142 km²,
largest 1,099 km².

## 1. `parcel_util::UpdateParcelCollision` (`0x1409312e0`)

`UpdateParcelCollision(Engine& e, const StreetToolkit& tk, const
std::vector<Box2>& boxes, const std::unordered_set<Entity>& exclude)`,
called only at `0x1425fcb9a` (every direct `call`/`jmp` in `.text` was
searched). CONFIRMED-static throughout this section.

### 1.1 The query

1. `0x9312e0..0x9313fb`: asserts `!boxes.empty()` (`0x931a01`). The union
   `u` of the `{min x, min y, max x, max y}` boxes (`vminss`/`vmaxss`, box
   first), then the query box `Q = {u.minx-50, u.miny-50, -FLT_MAX,
   u.maxx+50, u.maxy+50, +FLT_MAX}` (50.0f at `0x14368c18c`, ±FLT_MAX at
   `0x143679d08`/`0x143679d00`) at `rbp+0xb8`.
2. A collision context (`0x1409252d0`, at `rbp+0xd0`, destroyed at the end)
   and the result `std::unordered_map<Entity, std::vector<{int element,
   bool flag}>>` (MSVC, load factor 1.0, 8 buckets) at `rsp+0x60`.
3. Two component type indices by `typeid` (`0x140099970` on
   `[tk+0xb0]+0x48`): `ecs::component::BaseEdgeStreet` (`0x143cf2a98`) and
   `ecs::component::BoundingVolume` (`0x143cf0a60`).
4. The octree (`[tk+0xb8]`: engine `+8`, root entity `+0x2c`, node type
   `+0x40`): the root's node (0xc0 bytes; loose box `+8`, entities
   `std::vector<int>` `+0x20`, children `+0x98`, valid `+0xb8`) is tested
   against `Q`, then visited (`0x926240`) and descended (`0x925e30`, eight
   children in order, each tested against `Q` before its visit, then its
   children). Pre-order depth first; a node is visited only when its loose
   box meets `Q`.

The overlap test (descent `0x925e67`, visitor `0x92631d`): apart when
`q.min > v.max` or `v.min >= q.max` on any axis (`ja`/`jae`, so a NaN
never parts).

### 1.2 Per entity (the visitor `0x926240`)

For each entity id in the node's vector, in order:

1. `GetComponentDataIndex(engine, id, BoundingVolume)` (`0x9262c9`), the
   24-byte record `{min xyz, max xyz}` (`[engine+0x78][type]`: dense
   `+0x68` below index 0x40000000, else pages of 32 at `+0x80`), tested
   against `Q`; outside, next entity.
2. `0x140280ff0`: the entity's `BaseEdgeStreet`, or null (not a street
   edge: next).
3. The `exclude` set (FNV-1a of the id): in it, next. Its only caller
   passes an empty set (§7.3 of the earlier note).
4. `ParcelSystem` visit (`0x140ae7110`, `rcx = [tk+0x148]`, `0x926486`):
   the street's entry in the ParcelSystem's map (FNV-1a); its two parcel
   vectors (`+0x18`, `+0x30`: the two sides) in order, calling a
   `std::function` (vtable `0x1436ea440`, call operator `0x140932080`)
   with each parcel.

### 1.3 Per parcel (`0x140932080`)

1. `Engine::GetComponent<Parcel>` (`0x1400956b0`) and a pure lookup in
   `[tk+0x140]` (`0x140b5ae90`, a hash find into an int, -1 if absent).
2. The element test `0x140931a70(context, parcel, int, &boxes, &result)`
   (`0x93210a`). For each element `i` (32 bytes, from `parcel+0x10`): its
   points' xy AABB (`vcmpltss`/`vblendvps` from ±FLT_MAX) is tested
   against **each individual box, without margin** (`0x931b80`). No box
   overlaps: no entry. One does: if the element's flag byte `+0x18` has bit
   1, push `{j, true}` for every `j` from `i` to the end and stop; else
   push `{i, 0x14092df30(context, parcel, i, int, i)}` (the real collision
   test).
3. A non-empty result is inserted into the map (`0x140924a20`), asserting
   a new key (`pr.second`, `parcel_util.cpp:0x3d0`). An empty one is
   dropped: **only a parcel with an element overlapping some box is ever
   written.**

### 1.4 What it writes, in which order

After the walk (`0x9317dc`..`0x9318bb`), for each map entry in the map's
list order: `NoteComponentAboutToBeChanged(e, parcel, Parcel)`
(`0x142bb6970`), the Parcel, then for each `{element, flag}` bit 0 of the
element's `+0x18` set to `flag`, then `NoteComponentChanged`
(`0x142bb6b50`). `0x140055b50` around the loop is `ret` (an empty
profiler mark). The change notes, and the listeners they reach, follow
that order.

The list order of a local MSVC `unordered_map` built only by inserts is a
function of the insert sequence alone (DERIVED: only these inserts and
the rehashes they trigger change its list and buckets, both deterministic
in the keys and their order). The insert sequence is: visited nodes in DFS
order, their entities in vector order, streets' sides 0 then 1, parcels in
vector order, skipping parcels with an empty result. So **the results
depend on visit order**, but only through the subsequence of acted-on
parcels.

## 2. When a per-box walk is exact

A per-box walk tests each node and entity against every `box_i ± 50`
(z unbounded) instead of `Q`. Each such query lies inside `Q` (the same
monotone f32 subtraction and addition), so it visits a subsequence of the
game's nodes and entities, in the same order. The skipped streets' parcels
are the only difference. If none of them has an element overlapping a box,
the inserted subsequence, hence the map, its order, every element test
(`0x14092df30` runs only for those parcels), every write and every change
note, are the game's exactly (DERIVED from §1).

So it is exact iff **no acted-on parcel belongs to a street whose
BoundingVolume (or whose node's loose box) meets no `box_i ± 50`**. That is
a geometric invariant: a parcel's elements within 50 m of its street edge's
bounding volume. Nothing in the walk enforces it: the element test reads
the parcel's own points, and the street's volume is computed elsewhere.
The 50 m margin suggests the authors meant it to hold for the union, but
the code does not show it does for each box, so **not provable statically;
not built**. The fast walk (task step 3) waits for the probe.

An exact alternative that needs no invariant (GUESS on gain): keep the
game's walk but skip, for a street, the per-parcel `std::function`,
`GetComponent`, `0x140b5ae90` and element test of every parcel none of
whose elements meets a box (the test is cheap and the lookup is pure, so
skipping is exact). It saves part of the per-parcel work, not the descent
or the per-entity tests, and needs the Parcel and ParcelSystem layouts.

## 3. The probe (built)

`crates/tpf3mp-hook/src/parcelprobe.rs`, `TPF3MP_HOOK_PARCEL_PROBE=1`,
check only (docs/HOOKS.md, "The parcel walk's probe"). Four call
redirects (`0x926253` node, `0x9262c9` BoundingVolume index, `0x926486`
street, `0x93210a` element test), each calling the game's callee with its
arguments. Per walk it counts the nodes and entities the game visits and
those a per-box walk would, the streets, the parcels tested and acted on,
the acted-on parcels a per-box walk would lose (by street, by node), the
largest margin an acted-on parcel needed, and the time in near and far
streets' parcels. The static proof pins the four callees, the visitor's
and descent's callers, and the bytes of the node and record layouts.

### What an in-game run must show

Run the 256 km test save (the one that measured the numbers above) with
`TPF3MP_HOOK_PARCEL_PROBE=1` (timers on), let towns and industries grow,
and build some roads, for at least 10 windows (100 s), more on a long
session; then read every `perf: sim` line's `parcel probe` part:

- `lost 0 by street, 0 by node` in **every** window, over many acted-on
  parcels (thousands). One nonzero count proves a per-box walk at 50 m
  inexact: stop there.
- `margin needed` well below 50 m (if it comes near 50, the invariant is
  tight and a game change could break it).
- The gain: `entities in near nodes` against `entities`, and `street work
  (far ...)` against the window's `parcel-collision` total ms. If the near
  share is a few percent, most of the ~7.5 ms per update goes.

Hook.log at install must say `perf: sim timer parcel-collision: in` and
`perf: sim parcel probe: in (CHECK ONLY, changes nothing; ...)`.

Even then, a clean probe is evidence, not proof: a per-box walk built on it
should be a room setting (every game of a room the same), not a per-peer
optimisation, unless the invariant is later found in the street and parcel
builders.

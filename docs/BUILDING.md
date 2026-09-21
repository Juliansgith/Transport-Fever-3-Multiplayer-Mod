# Building: roads, track and constructions as portable intents

How a build travels between games that share no entity ids. Everything here is
what [TpF2 Multiplayer](https://github.com/silver2127/tpf2-multiplayer)
(0.6.1.12, 2026-09-20) ships and runs in players' games on Transport Fever 2
build 35924; its own account is `docs/REPLICATION.md` and the reverse
engineering behind it is `docs/re/PROPOSALS.md` in that repository. Offsets and
RVAs will not survive into TPF3. The shapes the tools produce, the resolution
rules and the failure modes are the part worth carrying, and each one below was
paid for with a desync or a crash. [HOOKS.md](HOOKS.md) covers the two hooks
(factory and `CommandList::Add`) that make the capture possible; this page is
what to do with a captured proposal.

Where this page says "measured", the fact was taken from a live proposal dump
or a differential capture on two instances; "decompiled" means read from the
binary and not yet exercised in play.

## One command for every build tool

Every build tool in TPF2, the road and track tools, the construction placer,
the module editor, the stop and signal tools, the bulldozer, terraform, paint
and the asset brush, produces one `BuildProposal` command carrying one
`construction_builder_util::Proposal`. That object is a `StreetProposal` (added
and removed nodes and segments, edge objects to add and remove, frozen node
indices, segment tags) followed by the construction fields (`toRemove`, `toAdd`
of `ConstructionEntity`, an `old2new` map) and three terrain grids (height
modifications, material indices, material mask). Rotation is baked into node
positions; only a `ConstructionEntity` carries a matrix.

The shape tells the tool apart (measured unless noted):

| tool | shape of the proposal |
|---|---|
| road, track build | added nodes and segments; new pieces carry placeholder ids (`-1, -2, ...`), existing endpoints positive ids; a mid-span junction also removes the edge it splits |
| street or track upgrade (type, catenary, bus lane, tram track) | no added nodes; N added segments replace N removed segments, every endpoint an existing node |
| construction placement | `toAdd[0]` plus the template's own street pieces (a one-track modular rail station: 25 track nodes, 24 edges); `toRemove` empty |
| module edit | `toRemove[0]` the old construction, `toAdd[0]` the new `ConstructionEntity` (same file, new params); a modular station re-adds its internal track |
| stop, signal, waypoint placement | the edge removed and re-added, plus one edge-object record |
| stop or signal bulldoze | the edge removed and re-added without the object |
| construction bulldoze | `toRemove` populated, nothing added |
| road or track bulldoze | removed nodes and segments, nothing added |
| terraform | no nodes or segments; a `Grid<{height, base}>` of 4 m cells |
| paint | no nodes or segments; the material index grid and its mask |
| asset brush | `toAdd` records of an asset-group type whose per-asset data is a vector of `{model path, matrix}` (decompiled); its commit clears `old2new` first |

Two consequences for a TPF3 profile: the factory hook needs one decoder per
shape, not per tool, and the shape is the only reliable way to classify the
command (the tool object is not on the command).

## Three ways an action travels

| mode | what happens |
|---|---|
| **strict** | The hook cancels the command inside the engine before it applies. The mod ships it with a stamp and every instance, the player's own included, applies it at that stamp. Nobody's world runs ahead. |
| **replay on peers** | The command applies natively at the click; the other instances apply it at the stamp a few steps later. The player's game is briefly ahead for that one action. |
| **poll** | No hook. The mod notices the change on the originating game by scanning and ships it; the others replay it. |

Every channel on this page is strict except one (a stop that replaces
another, under "Stops, signals and waypoints"). It was not always so: constructions started as replay-on-peers and that produced a
measurable height desync (the originator's native placement and the peers'
scripted one graded the terrain through two different code paths, so the
station's own track nodes landed on different z, invisible to a construction
digest that hashed only x and y). Strict means every instance runs
`construction_builder_util::Apply` with identical inputs, which is the only
way to get identical output from it.

Three rules hold for every channel:

- **Only cancel when something will replay.** The hook cancels nothing unless
  the script half has reported a live peer within the last 15 s. Alone or with
  the mod off, every command runs natively and the game is the stock game.
- **Never cancel on a failed decode.** A command the hook cannot read
  completely runs natively; the poll or replay path ships what it can. Losing
  the player's action is worse than a visible divergence, and a cancelled
  build that is then rebuilt from a misread proposal is the worst case of all:
  the wrong station on every peer.
- **Nothing on the wire names an entity id.** Entity ids differ between games,
  and on one game they are recycled. Roads travel as positions; constructions
  as file plus position; players as a logical company number that each
  receiver resolves to its own player entity.

## Roads and track

### The wire

A road or track command (`ROADP`) is a polyline of vertices as positions
`(x, y, z)` and links between vertex indices. Each link carries the street or
track type, catenary, bus lane and tram-track flags, the `BaseEdge` type and
type index (1 bridge, 2 tunnel) and its owner as a logical company. With the
polyline travel:

- **removals**, as endpoint positions plus the network kind, never ids;
- **fresh vertices**: the indices of the new nodes that the originator's
  engine attached to nothing (a vertex beside a road looks, geometrically,
  exactly like one on it; only the capture knows the difference);
- **the plan**: for each vertex, what the originator resolved it to (an
  existing node at a position, a split of the edge between two positions at a
  height, or a plain new node), so the peers repeat the decision instead of
  re-deriving it;
- **companion spans**: an unchanged bridge or tunnel span that the engine
  re-adds between two existing nodes as part of the command, rebuilt with its
  own properties rather than the new road's.

### Resolving a vertex on the receiver

Each instance resolves every vertex against its own world, in this order
(measured; the older `mp_bridge` mod had found the same rule and called it
"the road intersection bug"):

1. a node of the same network within 1.5 m, compared **horizontally only**
   (the engine settles nodes at heights other than the requested ones on
   embankments and smoothed terrain, so a 3D tolerance fails on every slope):
   reuse it;
2. else an edge underneath (track snaps within 2.0 m, roads within 5.0 m):
   **split** it, removing the edge and re-adding both halves around a new
   node with Hermite tangents scaled by the parameter interval so the curve
   keeps its shape; a split within 2.5 m of an end snaps to that end;
3. else plant a new node.

`BuildProposal` refuses an edge whose endpoint sits partway along an existing
edge; the interactive tool splits for you and a raw proposal does not. When a
player snaps onto an existing road the originator's proposal contains no
removal at all (it attached to geometry it already had), so a receiver cannot
wait for a removal list to tell it to split: it has to find the edge itself.
Two rounds were lost hunting for an `edgesToRemove` that was never there, and
one wrong offset guess on that hunt would have fed arbitrary entity ids into a
removal list.

Rules that came out of the replay:

- **Gate the capture on edges, not on new nodes.** A road joining two existing
  junctions adds zero nodes and one edge. The first capture treated "fewer
  than two new nodes" as a failed decode, correctly did not cancel, and the
  road built locally and never shipped. One unreplicated road changes
  connectivity, town growth reacts to it on one game only, and the world
  digest gap widens forever: a road is never a static +1.
- **Split halves inherit the split edge's record**, not the new road's: the
  street type of the road being crossed, its flags and its type index. A
  depot apron stamped onto the halves of a town road makes the engine refuse
  "Construction not possible" on every town road.
- **An added edge between two existing nodes is an in-place replacement**, and
  its removal must travel. A road passing under a bridge makes the engine
  replace the bridge span in place; a replay that took it for a split added a
  second span between the same nodes. A removal is matched in the command's
  own network only, and a double removal rejects the whole proposal.
- **A replaced edge must carry its objects.** The engine's own upgrade keeps
  the old edge's stops and signals under their ids. A replay that removed an
  upgraded street and re-added it without them crashed three games at the
  same step, in `station_util::GetTerminalPersonEdges` from the catchment
  update, because every stop on that street pointed at a deleted edge. An
  edge with objects and no one-for-one replacement (a split or a reroute)
  makes the whole command skip on every instance, with a log line.
- **Prefer what the originator's proposal did over re-deriving it.** Whenever
  a receiver reconstructs a decision from geometry (is this vertex on that
  road? is this node a crossing?) it eventually decides differently from the
  engine. Ship the decision.

### Level crossings

A crossing is its own entity, created from an explicit list during
`CreateProposalData` and never inferred afterwards. Native shape: the road is
cut into half, connector, half, and the track is cut at both ends of the
connector; every crossing node has exactly two street and two track edges. The
engine records a crossing at a node where the other network forms a straight,
homogeneous two-edge run. A script proposal goes through the same pipeline, so
the replay's job is to reproduce the node layout the UI produced:

- a track vertex within 4.0 m of a road node shares that node and takes the
  road's height when the two differ by more than 0.25 m (moving the road node
  instead asserts the engine); otherwise the road under the vertex is split;
- routing through an existing node requires the track to touch it (0.75 m)
  and the node to be straight-through. `Crossing.cpp:232` asserts that a
  four-arm crossing is two straight lines (arm 0 anti-parallel to arm 2, arm 1
  to arm 3). Route a track through a road corner and the game freezes on a
  native assert on every instance replaying the plan; `pcall` and
  `ignoreErrors` see nothing;
- a candidate within 12 degrees of the track's own direction at its closest
  approach is parallel, not a crossing. The spacing of a double track is
  exactly the 5 m band the crossing pass searches, so upgrading the second
  track of a pair split the first, and a real crossing under about 12 degrees
  cannot be built anyway;
- an edge found under a vertex more than 2.5 m above or below it is over or
  under, not a split; plan view alone lies under bridges;
- a crossing the engine refuses ("Too much slope": the track on an embankment,
  the road below) is refused on every instance, which is at least
  consistent. The native tool would have re-graded it; the replay only sees
  numbers.

### Demolish

Road and track demolish (`EDEMO`) matches edges by their end nodes: same
network kind, within 1 m. The kind is load-bearing (a road node and a track
node can coincide at a crossing). An edge that carries stops or signals is
refused; orphaned nodes are removed. It is strict with no originator skip: the
bulldozer is a tool that waits on its completion callback, so the cancel fires
it. This channel did not exist for the first months, and the bug report it
produced read as a lag bug ("I demolished a road during lag and the crossing
did not form"): the road was simply gone on one game and present on the
others, at any latency.

### Local repairs are commands too

Anything the script half changes on its own, a heal of an orphaned split after
a station is removed, a cleanup, a retry, has to be timed from an agreed
command stamp in simulation steps. A heal that ran from a frame counter
merged the same road on three games at three different steps; the final
geometry was identical, the digests matched, and passengers were rerouted at
different moments, so the buses drifted. When every replicated command applies
on-step and the geometry agrees, grep for unstamped local proposals next.

## Constructions

Stations, depots, assets, harbours and airports.

### The wire

The hook reads the placement off the proposal at the factory: the file name,
the transform, the parameters (the `ConstructionEntity`'s Lua table, walked
into a serialisable form, `seed` kept, strings escaped so an embedded newline
cannot split a line-based stream), the name, and the player as a logical
company. The template's street pieces (a depot apron, a station forecourt, the
road a station was dropped onto and split) travel as a separate road record
(`ROADC`) paired to the construction by a placement serial stamped on both
records, never by distance or arrival order. Every instance then builds the
same scripted proposal at the stamp; the originator's native build is
cancelled and its completion callback fired.

### What a script proposal must carry

All measured:

- **`params.seed`.** Without it the factory returns a bare `false`. Two weeks
  of workarounds followed from one measurement made with the seed stripped.
- **A name.** `Apply` gives the construction and its child entities (the
  vehicle depot, the station entities) `NAME` and `PLAYER_OWNED` only when
  the name is non-empty. An unnamed child crashes the game when a player
  clicks it: the GUI select handler dereferences null. Neither a later
  `SetName` nor the script's `setPlayer` repairs a child afterwards.
- **A Context that matches the intent.** The UI places with terrain alignment
  and graph cleanup, and with `gatherBuildings` set the engine demolishes the
  footprint's town buildings itself, identically everywhere. A nil context is
  not equivalent, and `ignoreErrors` on a raw proposal does not demolish
  colliding buildings: it builds through them.
- **Not the script's `buildConstruction`.** It runs the template at raw
  coordinates, construction-owned and never joined to the road; a later street
  proposal cannot remove those pieces ("Construction not possible"). Replay
  the whole placement proposal instead.
- **The UI's resolved geometry.** The engine never snaps a template's snap
  nodes to existing world nodes; only the shape the UI resolved is buildable.
  A road depot dropped on a junction is one node and one segment welded onto
  the junction, and the replay's merge has to present exactly that (adopt the
  template's apron record onto the shipped segment, re-point the frozen node
  index, drop the template's own apron). Any new construction kind is first
  diffed field by field against a dump of the UI's proposal for the same
  placement; both of the above were invisible in the geometry.

### Index linkage

The construction-to-street linkage inside a proposal is index-based: the
construction's frozen nodes are indices into the added nodes, `segmentsBefore`
is the segment count before the template's edges were appended, and nothing on
a node or segment names its construction except the appended connector. So a
hook that patches a script-built proposal edits records in place and only ever
drops the **last** record of a vector; compacting shifted every later index and
the apply asserted. Placeholder ids are numbered like the UI's (`-1, -2, ...`):
large placeholders push the template's regenerated ids below them, which the
same bookkeeping cannot handle.

### Footprint and failure

- A cancelled placement builds with `gatherBuildings=true`, so the engine
  demolishes the same town buildings on every instance. For the non-cancelled
  path (parameters unreadable, so the native build stood) the originator
  ships the town buildings it still has nearby with the radius it gathered
  them in, and the peer removes the others inside that radius; a list whose
  survivors mostly do not exist on the peer is refused loudly. A bounding-box
  sweep over-clears (a modular station's box is about 170 by 120 m).
- On failure the replay retries once after clearing the footprint, then asks
  the originator to roll back: it bulldozes its own copy (same file within
  1 m) so the worlds stay equal. The visible symptom of a refused station is
  three links downstream: "a station got demolished but not by player
  action" and "vehicles were bought but never assigned", because the line
  update that named the station then found no station group. Read the
  rollback record first, then the peer's refusal, never the demolish.
- The parameter walk has no depth or entry cap, and a field it cannot read
  fails the whole decode (the build then runs natively behind a notice). An
  earlier walker dropped seventeen boolean entries of a rail station's
  parameters, cancelled anyway, and the replay asserted in the engine's snap
  node lookup on all three instances at once.

### Module edits and upgrades

The old construction and the new parameters come off the proposal; every
instance upgrades the construction with the same file within 10 m at the
stamp. Two traps:

- **Entity ids are recycled.** A strict replay built a station under an id
  that a bulldozed town building had just freed; the "already seen" set still
  held that id, the station was never adopted, and sixteen module edits were
  cancelled locally and applied nowhere. Any id-keyed set of seen things can
  hide a new entity; look things up by position when an edit "cannot find"
  what the player can see.
- **Town buildings are constructions.** The construction query returns
  `building/era_b/res_1_2x2_01.con` and its kin, and towns spawn one every ten
  seconds or so. The first poll captured twenty of them per game in three
  minutes and scheduled each other's town growth for replay. Filter by
  ownership (the `PLAYER_OWNED` component), never by a name blacklist.

### Demolish

Strict: every instance requires the same file within 2 m of the position. The
fallback for a bulldoze the hook left to run natively is a poll: a tracked
construction missing for two polls ships a demolish, and peers remove the
nearest one within 30 m; a record counts as present only while its entity
still carries a construction component of the recorded file, or a replacement
stands within 1 m. An optimistic demolish (native at the click, peers at the
stamp) splits the refund across sim-times and removes a different set of
passengers and cargo on each game, which the geometry digest never shows.

Bulldozing an entity that is already gone is an access violation, not an
error: a sweep that removes a building removes the asset groups that stood on
it, and the next id in the loop is dead. Check existence immediately before
every bulldoze in a loop.

## Stops, signals and waypoints

The engine's own stop tool removes the edge and re-adds it with the object
list carried verbatim: every untouched object under its positive id (which the
apply treats as re-parent, keeping its station group and lines) and the new one
as a negative index into the add list. A removed object goes into the remove
list and the apply rewrites its lines and station group before it dies. The
script's proposal conversion copies all of that, so placement and deletion are
lossless from Lua. Rules:

- the `left` byte is the engine's, not the geometric side. Two signals that
  both stood geometrically left of their track carried `left` 0 and 1; a
  waypoint on the centreline has no side at all. Ship the engine's byte with
  the originator's unit tangent, and let the receiver flip it only when the
  edge it matched runs the other way;
- one stop per side per street edge. Two objects with the same side value on
  one edge is a fatal assert in lane creation. Guard by side, not by count;
- merging a new stop into a nearby group is not in the proposal: the apply
  pairs an opposite-side stop within 125 m, else joins any group within
  200 m. The same placement merges the same way everywhere for free;
- a compatible stop dropped on an occupied side **replaces** the old one and
  the engine re-points its lines, which a script proposal cannot express.
  That one action is not cancelled: it runs natively, is found by polling,
  and the originator re-ships every affected line afterwards;
- the edge is found by its end points within 2 m, else the nearest centreline
  within 14 m; an edge frozen into a construction is refused.

## Terrain and the asset brush

- **Terraform.** The whole edit is a grid of 4 m cells, each `{target height,
  height before}`, on the proposal; a raise is a 10 by 9 grid, a smooth 83 by
  59. The hook stashes the grid at the factory and cancels the commit; every
  instance, at the stamp, sends an empty carrier proposal that the hook fills
  natively with the grid. The receiving game's height came out bit-identical
  to the originator's. A stroke is held until the originator's own replay has
  applied, so the next part of the stroke is computed against the replayed
  heights.
- **Paint** is the material index and mask grids on the same path. The
  material texels are simulation data, not a graphics setting: a paint applied
  in the right place with the two games at different texture resolutions.
- **The asset brush** commits asset-group records. Lua cannot create asset
  groups, so the replay is again a native fill of an empty carrier; removed
  groups travel by position and count. The cost of not replicating it was
  measured: 104 trees on one game, every replicated command on-step, and six
  hundred steps later the town-building and road lanes diverged in three
  towns 9 km apart, because town growth reads the assets. A desync minutes
  after the last command means an unreplicated native edit, not a
  replication bug.

## What does not fit a proposal

- Companies and ownership travel as a logical company number and a typed
  `PlayerOwned` component on the receiver; the engine silently discards a
  plain table assigned to that optional field.
- A construction placed by a **bare** replay (nodes 0, edges 0) has its
  platform track rebuilt from the `.con` template, so nothing pins its
  geometry; the placement serial and the paired street record are what keep
  it attached to the road on every game.
- Bridge and tunnel type ride on the segment record, not on a node, and a
  split half keeps them.

## Measure these first on a TPF3 build

In the order they were expensive on TPF2:

1. Dump the UI's proposal and a script-built proposal for the **same** depot
   on a junction and diff them field by field; the geometry will match and
   the flags will not.
2. Confirm whether the script's build-proposal path accepts a construction at
   all, and which single field it insists on (`seed` here).
3. Check whether `Apply` names and owns child entities, and what happens on a
   click when it does not.
4. Place one station on flat ground far from any road with two instances and
   diff every edge's height, not just x and y, between them.
5. Find the crossing recorder's straight-line assert and what the visitor
   requires of the other network at a shared node.
6. Build a road between two existing junctions and check the capture saw an
   edge with no new nodes.

# Pending builds ("build ghosts") in a room, 2 October 2026

**The ask.** In a room, a player's road or track appears only when the
room's turn applies it, which feels laggy. The player's own builds should
appear at once, looking exactly like real roads (not the tool's preview
look), and the player should be able to build on from them before the room
has applied either. Nothing of a pending road may enter the simulation of
any game before the room applies it: traffic, pathfinding, terrain,
collisions and town growth must not see it.

**Result.**

- **Option 3 is not possible on build 40408.** Option 3 was local road edges
  the simulation ignores. The engine has no such edge. Both of the game's
  `GameState`s are simulated in turn. Every system finds edges by component
  type alone. A local entity reorders the entity ids that every later entity
  gets.
- **Option 1 is ruled out for the same reasons.** That was building for real
  and rolling back.
- **Option 2 is the way: draw the pending builds as decoration.**
  - The game already has a renderer that draws a proposal's real road meshes
    without touching either engine: the build tools' `UI::BuilderRenderer`.
  - Its preview look is one extra flat-colour overlay pass. That pass is a
    single call per instance, at `0x7bf3dd`.
  - Without the overlay, it draws the road's textured meshes.
- **Chaining needs no engine support.** The room names roads by position
  (D8), so a new road's loose end placed on a pending road is named by what
  it lands on, before it is sent.
- **The prototype on `feat/build-ghost`** builds the renderer-independent
  half, off unless `TPF3MP_HOOK_BUILD_GHOST=1`:
  - the hook's ledger of pending builds;
  - the joining of loose ends onto them.

  Drawing them is the next step. It needs work in the game, which this task
  was not cleared for.

Everything here is static: the Windows executable of Steam build 40408
(SHA-256 `de1daad3a13f3b7e9f79903361bb43769cf4f15e59271a263aefe1f075f23ef2`),
indexed with `tools/tpfre` (`tpfre index`, then `tpfre q`). Nothing was run
in the game.

Addresses are RVAs (image base `0x140000000`), and names are `tpfre`'s, from
RTTI, assert strings and `__FILE__` paths. Each finding is labelled:

- **CONFIRMED**: read in the binary;
- **INFERRED**: reasoned from what was read;
- **UNKNOWN**: not found.

## 1. What the engine does

### 1.1 Two game states, both simulated in turn

- **The sim loop.** `CGame::RunGameSimLoop` is `0x11e210` (CONFIRMED).
  - It runs on its own thread.
  - It holds `GameState*[2]` at holder `+0x78/+0x80` and the current index
    at `+0x98`.
  - It steps the simulation on `states[idx]` through `GameSim::Step`
    (`0x159390`, already in the profile).
  - It then replicates the state just stepped into the other one at
    `0x11ea5a`.
- **The flip.** `CGame::Sync` is `0x11f650`, on the main thread, called
  from `CGame::Step` `0x11f3b0` (CONFIRMED). Its asserts are
  `"CGame::Sync"` and `"m_replicateTimes.size() == ..."`. It:
  - publishes the stepped state for the UI at `CGame+0x1e0`;
  - flips the index (`1 - [+0x98]`).
- **The UI's view.** `UI::GameStateProvider::vf1` (`0x868630`) returns
  `CGame+0x1e0` (CONFIRMED).
- **Replicate.** `GameState::Replicate` is `0x255de0`, with asserts
  `"&dst != this"` and `"&dst.m_gameRes == &m_gameRes"` (CONFIRMED).
  - It replicates the `ecs::Engine` at `GameState+0x18`.
  - On TPF2 the engine was at `+0x28`. On TF3, `+0x28` is the `CGameTime`.
- **Replication is a change log, not a copy** (CONFIRMED).
  `ecs::Engine::Replicate` (`0x2bb78f0`) applies an `ecs::Replicator` log
  with `Replicator::Apply` (`0x2bb4430`). The log is recorded by
  `NoteComponentChanged` (`0x2bb6b50`) and closed at `EndModification`
  (`0x2bb4d90`).
- **Some components are copied whole every time** (CONFIRMED). The list is
  built at `0x243de0` and includes `ModelInstanceList`, `BoundingVolume`
  and `TransportNetwork`.

So the displayed engine is the one the next step simulates (INFERRED, from
the flip and the order of the calls):

- an entity added to it is simulated;
- an entity added to the other one is logged and replayed into it.

Neither engine can hold a road that only draws.

### 1.2 Entity ids depend on every add and remove

`ecs::Engine::AddEntity` is `0x2bb37b0` (assert
`"detail::IsEntityRemoved(m_entityComponents[id])"`). It takes a free id
from the front of `m_freeIds` (a `std::deque<int>`, engine `+0xd8`), or
else the length of `m_entityComponents`. A removed id goes to the back of
the deque at `EndModification` (CONFIRMED).

A local road made and later removed in one game therefore changes the order
of that game's free ids. From then on its entities get other ids than the
other games' do. Entity ids are known to steer the simulation
(`TF3_VEHICLE_DETERMINISM_2026-09-30.md`), so that is a desync. The save
would differ too (`ecs::Engine::Save` `0x2bb7cb0` writes `m_freeIds`).

### 1.3 Nothing makes a system skip an edge

- **Every system finds edges by component type alone** (CONFIRMED). The
  game state's setup (`0x1ef0d0`) registers each system's listener groups by
  component type, with no predicate. Examples:
  - `OrComponentGroupFamilyNoList<BaseNode,BaseEdge>`;
  - `ComponentGroupFamily1<TransportNetwork>`;
  - `NoList1<TerrainAlignmentList>`, `NoList1<ShapeList>`.
- **Each consumer takes every edge** (CONFIRMED):
  - `StreetSystem` (`EntityAdded` `0xb51ce0`, node-to-edge map `0xb514d0`);
  - `TransportNetworkSystem` (`0xb6e6a0`, `MakeTpNetData` `0xb707e0`);
  - `CatchmentAreaSystem`;
  - `TerrainAlignmentSystem` (`0xb54dc0`);
  - `OctreeSystem` (`0xae1dc0`), which build checks and picking use.

  The collision grid is UNKNOWN.
- **No flag string exists** (CONFIRMED). No string marks an edge as hidden,
  temporary, preview, proposal or ghost.
- **The only edge flags are narrower** (CONFIRMED):
  - `roadDevelopmentLocked` stops towns changing a road, nothing else;
  - `Construction.frozenEdges` marks a construction's own edges.
- **Applying a proposal makes real entities** (CONFIRMED).
  `apply_proposal.cpp` (`0x9f96e0`) calls `AddEntity`. The simulation's
  engine has no temporary entity.

### 1.4 Road meshes are made on the render side

- **The sim keeps shapes, not meshes** (CONFIRMED). A street or track edge
  in the simulation keeps a `ShapeList` (procedural shapes with a generator
  index), a `ModelInstanceList` and a `BoundingVolume`. They are made by
  `street_util::AddParallelStripShapeListAndBoundingVolume` (`0x25f83f0`).
- **The renderer generates the mesh from the shape** (CONFIRMED).
  `CRenderer::NewUpdate` (`0x2ffc80`) is called from
  `UI::CRendererComponent::DoStep` (`0x6af440`). Its helper `0x2f0ab0`
  generates the mesh through `procedural::GeneratorCache::GetOrGenerate`
  (`0xcd6740`).
- **The generators belong to the UI.** `StreetGenerator` is built inside
  `UI::CMenuUI::StartGame` (CONFIRMED). That `TrackGenerator` is UI-owned
  too is INFERRED.

### 1.5 The build tools' preview never touches an engine

- **The tool builds an overlay over the real world** (CONFIRMED).
  `UI::StreetBuilder::UpdateRenderer` (`0x58a100`) calls
  `UI::builder_renderer_util::AddToRenderer` (`0x5e2b20`), which builds:
  - a `UI::ProposalEngine` (ctor `0x54b400`) over the real engine and the
    proposal;
  - a `UI::CachedProposalEngine` (ctor `0x4dfb20`, a read-through cache)
    over that;
  - a `ProposalStreetGraph` (`0xa46250`).
- **The proposal's pieces carry made-up negative ids** (CONFIRMED), for
  example:
  - added nodes and segments are `-1..-N`;
  - edge objects are `-400000000 - i`.

  `AddEntity` is never called.
- **The renderer draws through its own passes.** `UI::BuilderRenderer`
  (vtable `0x36dca68`) is built by the factory closure `0x8266d0` through
  its ctor `0x7b4160`. One is made in each tool's constructor (CONFIRMED,
  for example `StreetBuilder` at `0x56a740`, kept at `[this+0x970]`). Each
  frame:
  1. `Begin` (`0x7b9eb0`) takes the overlay and its street graph.
  2. `InsertEntity` (`0x7bc960`) reads each entity's components through the
     overlay and appends the meshes to the renderer's own state (`+0x1b8`).
  3. `End` (`0x7bb730`) lets the overlay go.
- **After `End`**, the passes draw from that state alone (INFERRED).
- **The preview look is one overlay pass** (CONFIRMED). The passes are:
  - `builderRendererNormal` (vf7 `0x7c00c0`) and `builderRendererDepth`
    (vf6 `0x7bea00`) draw the proposal's textured meshes with the model
    and street renderers;
  - `builderRenderer` (vf4) is `UI::BuilderRenderer::RenderNew` (`0x7befa0`,
    named by its lambdas' RTTI). It draws everything again in flat colour
    through `0x5f9d20`, which `UI::SelectionRenderer::vf4` shares: the
    highlight overlay.
- **How RenderNew picks its colour** (CONFIRMED). It tests an error flag
  (`[state+0x193d]`) and picks from the renderer's table:
  - blue fill (0.4, 0.733, 1, 0.3) at `+0x118`;
  - red fill (0.9, 0, 0, 0.4) at `+0x158`.

  Its only call of the overlay is:

  ```
  0x7bf3d6  49 8b 4f 08     mov rcx, [r15+8]      ; the render context
  0x7bf3dd  e8 3e a9 e3 ff  call 0x5f9d20         ; r15 = this renderer
  ```

### 1.6 The tools snap to the simulation's own world

- **The street tool's snap reads the simulation directly** (CONFIRMED).
  - The tool's frame (`UI::StreetBuilder::vf5` `0x585e50`) finds its snap
    through `CalcCursorPoint` (`0x571820`).
  - That calls `street_util::FindSnapPointBaseEdge` (`0x60c360`) and the
    node snap `0x25eebd0`.
  - These read the simulation's `ecs::Engine`, `OctreeSystem`,
    `CollisionSystem` and `StreetSystem` directly, from a
    `street_util::StreetToolkit` (ctor `0x2458f0`).
  - The pointers are set once in the tool's constructor.
- **The overlays are for preview and checks only** (CONFIRMED). The
  `UI::IEngine` overlays (`WrappedEngine`, `ProposalEngine`,
  `CachedProposalEngine`) serve the preview and the checks, never the snap.
- **The track tool is the same `StreetBuilder`** in another mode (INFERRED).

So no overlay can make the tool snap onto a pending road. A proposal that
snapped onto overlay ids would carry ids that mean nothing in the
simulation (INFERRED).

## 2. The three options

1. **Build locally, roll back when the room's turn comes. Rejected.** The
   local road is simulated in this game until rolled back:
   - traffic routes over it;
   - towns grow along it;
   - the terrain is reshaped;
   - entity ids are spent (1.1, 1.2).

   Rolling back would mean restoring the whole world, the free-id order
   included. That is not a rollback the engine has, and it does not fail
   closed.
2. **Draw real road meshes as decoration. Recommended.** The game's own
   `BuilderRenderer` already draws a proposal's real meshes, from an overlay
   that never touches an engine (1.5). Two routes:
   - **R1, recommended:** our own `BuilderRenderer`, filled with the
     pending builds, with its overlay pass skipped. This is the preview's
     textured meshes, without the blue.
   - **R2, if R1's look is not exact:** append the pending builds' shapes to
     the main renderer in `CRenderer::NewUpdate` (`0x2ffc80`), generated
     through `GeneratorCache::GetOrGenerate`. This is the very pipeline real
     roads draw through, so the look is exact. It is deeper work: the shape
     records and the renderer's lists are not mapped yet.
3. **Local edges the simulation ignores. Not possible.**
   - There is no such flag (1.3).
   - Both engines are simulated (1.1).
   - The ids would move (1.2).

   To fake it, every system's listener would need its own filter, about ten
   detours: street, transport network, catchment, terrain alignment,
   octree, collision, parcels, street connectors and towns. Saves,
   replication and every game script's entity walks would also need
   filtering. The free-id order would need restoring around each local
   change. That is fragile and fails open.

## 3. Recommended design

Four parts.

### 3.1 The ledger: built, in the hook

`crates/tpf3mp-hook/src/ghost.rs`.

- **What it keeps.** Each `BuildRoad` or `BuildTrack` the player hands to
  the room (`command`) is kept under its ticket.
- **When it goes:**
  - when the ticket is answered: `applied` in this game, or refused;
  - all at once when the room's game stops or a world loads.
- **Bounds.** At most 32 are kept. Past that, a build has no ghost (it still
  goes to the room).
- **How the GUI reads it.** Through `tpf3mp_native.pending()`, which is
  optional in the bridge, so no version bump.
- **It is a pure data structure.** It touches no engine and is tested in
  Rust.

### 3.2 Chaining: built, in Lua

`mod/tpf3mp_1/content/scripts/tpf3mp/ghost.lua`, called at the hand-off in
`tpf3mp_sim.script.lua`'s `guiUpdate`.

- **Which ends it joins.** Before a road or track build goes to the room,
  each loose end is named by what it lands on among the pending builds. A
  loose end is a `New` vertex with one link.
- **The rules,** as a receiver resolves a vertex:
  - within 1.5 m of a pending node (apply's `NODE_TOLERANCE`), it is that
    node, at that node's position;
  - within 2 m of a pending edge's centreline, it splits that edge, by its
    ends, at the nearest point of the curve;
  - within 2.5 m of an end, it is that end's node instead.
- **It only joins:**
  - within the end's own network;
  - following pending splits and removals of pending edges.
- **It refuses** what it cannot name exactly:
  - an end between two pending edges;
  - a new edge with both ends on one pending node.
- **Why it holds in every game.** The room applies the player's actions in
  the order it receives them. `room.rs` `intent` appends each to the game as
  it comes, so the pending build resolves first in every game. That a
  connection delivers them in the order sent is INFERRED.
- **If the pending build failed,** the joined one resolves to nothing and
  fails in every game alike (`apply.lua`: "anything that resolves to
  nothing fails the whole build, in every game").

The tool itself still shows the new road starting from open ground. A
native snap override (3.4) would fix that look; the action is right either
way.

### 3.3 Drawing: the next step, route R1

1. **Create one `BuilderRenderer` of our own**, as the factory closure
   `0x8266d0` does for a tool. Find where a tool's renderer is registered
   with `CRendererComponent` (UNKNOWN), and register ours there, so its
   passes run.
2. **Fill it when the pending set changes**, on the main thread, with each
   pending build's proposal. Run `Begin`, then `InsertEntity` per entity,
   then `End`, or `AddToRenderer` (`0x5e2b20`) with a `Proposal` and its
   `ProposalData`. There are two ways to get those:
   - **Rebuild the proposal from the action.** The GUI builds it as
     `apply.lua` does (`networkInto`) and has the game make its data with
     `api.engine.util.proposal.makeProposalData`. The hook then needs the C++
     objects behind those Lua userdata (UNKNOWN). That
     `makeProposalData` writes nothing is INFERRED.
   - **Keep the tool's own proposal at the click.** The hook already stops
     the click's `WorldBuildProposal` at `CommandList::Add`. Copying its
     proposal needs the game's copy constructor (UNKNOWN).

   Either way a pending build is drawn over the world as it is now, not as
   it will be. A pending build's split of a real road draws over that road.
3. **Skip the overlay for our instance only.** Splice the call at
   `0x7bf3dd`: when `r15` is our renderer, do not call `0x5f9d20`. Use the
   splice engine the edge watch uses, with the bytes above checked first.
   Leave the renderer's flags `+0xf4` (see-through) and `+0xf5` clear.
4. **Terrain.** The preview's height modifications (`AddHeightMod`
   `0x7b9300`, `BeginHeightMod` `0x7ba0c0`, gated by `+0xf1`) can show a
   pending embankment or cut without touching the terrain. Optional.
5. **Clear the renderer** (`0x7ba590`, INFERRED to be its reset) whenever
   the ledger changes, and leave it empty when the switch is off.

Signatures for these targets, each made with `tpfre q sig` and unique in
`.text` of build 40408. They are not in the profile yet, since nothing uses
them:

| target | RVA | signature |
|---|---|---|
| BuilderRenderer factory closure | `0x8266d0` | `48 89 5C 24 08 55 56 57 41 54 41 55 41 56 41 57 48 81 EC F0 00 00 00 4C 8B E9` |
| `UI::BuilderRenderer` ctor | `0x7b4160` | `48 89 5C 24 20 55 56 57 41 54 41 55 41 56 41 57 48 8D AC 24 40 FC FF FF 48 81 EC C0 04 00 00 C5 F8 29 B4 24 B0 04 00 00` |
| `UI::BuilderRenderer::Begin` | `0x7b9eb0` | `40 57 48 83 EC 50 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 44 24 40` |
| `UI::BuilderRenderer::End` (INFERRED name) | `0x7bb730` | `48 89 5C 24 08 55 56 57 41 56 41 57 48 8D 6C 24 C9 48 81 EC B0 00 00 00 4C 8B F1` |
| `UI::BuilderRenderer::InsertEntity` | `0x7bc960` | `48 8B C4 48 89 58 10 48 89 70 18 48 89 78 20 55 41 54 41 55 41 56 41 57 48 8D 68 88 48 81 EC 50 01 00 00 C5 F8 29 70 C8 48 8B 05 ?? ?? ?? ??` |
| `UI::BuilderRenderer::RenderNew` | `0x7befa0` | `48 8B C4 48 89 58 20 55 56 57 41 54 41 55 41 56 41 57 48 8D A8 38 FE FF FF 48 81 EC 90 02 00 00 C5 F8 29 70 B8 C5 F8 29 78 A8 C5 78 29 40 98 48 8B 05 ?? ?? ?? ??` |
| `UI::builder_renderer_util::AddToRenderer` | `0x5e2b20` | `40 55 53 56 57 41 54 41 55 41 56 41 57 48 8D AC 24 18 FE FF FF 48 81 EC E8 02 00 00 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 85 D0 01 00 00 49 8B F1` |
| `UI::ProposalEngine` ctor | `0x54b400` | `48 89 5C 24 10 48 89 74 24 18 48 89 7C 24 20 55 41 54 41 55 41 56 41 57 48 8D AC 24 10 FF FF FF 48 81 EC F0 01 00 00 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 85 E8 00 00 00` |
| `UI::StreetBuilder::UpdateRenderer` | `0x58a100` | `48 8B C4 48 89 58 10 48 89 70 18 48 89 78 20 55 41 54 41 55 41 56 41 57 48 8D A8 38 FB FF FF` |
| `CRenderer::NewUpdate` (R2) | `0x2ffc80` | `48 89 5C 24 10 48 89 74 24 18 57 41 54 41 55 41 56 41 57 B8 80 14 00 00` |
| `procedural::GeneratorCache::GetOrGenerate` (R2) | `0xcd6740` | `48 89 5C 24 10 55 56 57 41 54 41 55 41 56 41 57 48 8D AC 24 60 FE FF FF 48 81 EC A0 02 00 00 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 85 98 01 00 00` |
| `CalcCursorPoint` (3.4) | `0x571820` | `48 8B C4 55 53 56 57 41 54 41 55 41 56 41 57 48 8D A8 78 FD FF FF 48 81 EC 58 03 00 00` |
| `street_util::FindSnapPointBaseEdge` (3.4) | `0x60c360` | `48 8B C4 48 89 58 18 55 56 57 41 54 41 55 41 56 41 57 48 8D A8 D8 FB FF FF` |

### 3.4 Live snapping onto a ghost: optional, later

To make the tool's own preview start at the pending road, hook
`CalcCursorPoint` (`0x571820`) or the toolkit's snap wrappers (`0x25ef740`,
`0x25ef830`). Near a pending node or edge, answer a terrain snap at its
exact position. The snap records' layouts (`SnapPointTerrain`,
`SnapPointBaseEdge`) are UNKNOWN. The tool's threads run these (it uses
`packaged_task`s), so the pending positions must be read under a lock. The
joining of 3.2 stays either way: it is what names the end for the room.

## 4. Risks to lockstep

- **The ledger and the joining cannot desync a room.**
  - They change only what this player's game sends.
  - The joined action is an ordinary `BuildRoad` or `BuildTrack`, by
    positions, which every game resolves alike.
  - A joined build whose pending base failed fails everywhere.
  - Nothing new is sent, so one player may run the switch alone.
- **The drawing must never reach an engine.** R1 holds overlays over the
  engine as `const` and writes only to the renderer's own state (1.5,
  CONFIRMED for the renderer, INFERRED for the overlays). The main risks:
  - building the `ProposalData` outside a tool: `makeProposalData` is
    INFERRED to be read-only, and if it is not, its effect would land in one
    game only;
  - calling the renderer from the wrong thread;
  - a crash in a render-thread detour. That does not desync, but kills the
    player's game.

  Each needs the fail-closed guards the hook uses: checked bytes, a switch
  off on panic, and no draw rather than a wrong one.
- **What the player sees can differ from what the room builds.**
  - The tool's proposal and the room's replay are the same action, but the
    room may refuse it. The ghost then vanishes, and the reason comes back
    through `results()`.
  - A ghost is drawn over the world as it is now: over town buildings the
    replay will clear, and over terrain it will reshape.
  - A pending build over another pending build's area may be refused by the
    room where the tool saw no collision.
- **Ordering** (INFERRED, see 3.2). A rate-limited or refused base is
  answered as refused, and its dependants fail in every game.
- **Not covered:**
  - the stock bulldozer cannot remove a pending road (it is in no engine);
  - a sent build cannot be cancelled.

## 5. For the owner to decide

1. **Whether to go on to drawing (3.3).** It adds the hook's first detours
   on the render path, which a crash there takes the player's game with. It
   needs work in the game, which GAME_TESTING.md reserves for a test the
   owner asks for.
2. **What a pending road looks like.** The ask is "exactly like real roads".
   Then a player cannot tell a road the room may still refuse from one it
   built. Choose one:
   - exact;
   - exact plus a small marker (for example the edge colour of the same
     overlay, at low alpha);
   - something else.
3. **Whether pending builds are a room rule (D6) or each player's switch.**
   They are each player's now: they change nothing another game computes.
4. **The plan.** This work is not in docs/PLAN.md. Adding it (for example
   under Part 3, Dev C, beside "Roads and track") is a change to the plan,
   and that is the owner's. This branch does not touch PLAN.md or
   DECISIONS.md.

No decision conflicts with this design:

- it names roads by position (D8);
- it runs in a game the launcher started (D11);
- it refuses what it cannot name (fail closed).

## 6. The prototype on `feat/build-ghost`

Off unless `TPF3MP_HOOK_BUILD_GHOST=1`; docs/HOOKS.md "Pending builds".

- `crates/tpf3mp-hook/src/ghost.rs`: the ledger.
  - It adds at `command`, drops at `answer`, and clears at room end and
    world load.
  - It is read by `pending()`.
  - Its tests: unit tests, plus `a_road_handed_to_the_room_is_pending_until_answered`
    in `lua.rs` through the real table.
- `mod/tpf3mp_1/content/scripts/tpf3mp/ghost.lua`: the joining.
  - The bridge's `Link:pending()`.
  - The join at the hand-off in `tpf3mp_sim.script.lua`. A join that fails
    stops the build with why, and nothing is sent.
  - Its tests: `crates/tpf3mp-proto/tests/lua_ghost.rs` (the joined actions
    decode with the schema), and
    `a_road_built_off_a_pending_road_goes_to_the_room_joined_onto_it` in
    `lua_mod.rs`.

To see in the game, once the owner asks for it (docs/GAME_TESTING.md):

1. With the switch on, build a street, then at once a second street from
   its far end.
2. The hook's log says `joined 1 loose end(s) onto pending builds`.
3. Both games then have one junction there, and the lanes match.

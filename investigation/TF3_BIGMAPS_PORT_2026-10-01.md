# Porting Big Maps to Transport Fever 3: what TF3 allows, what carries over, a staged plan

2026-10-01. This is research and a plan. Nothing is implemented, no game was
launched, and the game install was only read.

**Sources:**

- silver2127's **tpf2-bigmap** (MIT, last commit `16444b5`, 2026-09-27; also
  vendored as `tpf2-multiplayer/bigmap/`), for TF2 build 35924, read as a
  read-only input. Its README, `docs/`, `src/` and `linux/` are the basis of
  every TF2 fact here. No code is taken from it. The patch shapes proposed
  below follow its design and must credit it if built.
- TF3 Steam build 40408. The exe's SHA-256 is `de1daad3…f23ef2`, the same
  as the profile's `[build]`.
- The scripts in `base/content/*.zip` and `base/mod.json`.
- `tools/tpfre` indexes of both executables. TF2 names were carried onto a
  copy of the TF3 index with `tpfre match`, giving 10,368 pairs. The
  user's `tf3.tpfdb` was left untouched.

[docs/BIGMAPS.md](../docs/BIGMAPS.md) already summarises tpf2-bigmap and
describes the prototype (`crates/tpf3mp-bigmap`, `mod/tpf3mp_bigmap_1`).
This file answers the questions that page leaves for TF3: its section
"Measure these first on TPF3", and the empty profile roles in
`features.rs`.

**Tags:**

- **CONFIRMED-static:** read from TF3's bytes or scripts. Each says how it
  was verified.
- **MEASURED:** read from a TF3 game log.
- **DERIVED:** computed from the above.
- **NOT FOUND YET:** looked for and not located.

## Summary

- **The menu's size limit is in Lua.** `getNumTiles` in
  `gui/menu/new_game_or_map_settings_page.tl` is a hard-coded table. The
  largest entry is Gigantomaniac: 112×112 tiles (28.7 km), or 50×250 at 1:5
  (12.8 × 64 km). There is no native clamp like TF2's 224.
  - A normal mod cannot replace a main-menu file.
  - The hook's existing loader redirect (`menu_entry.rs`, which already
    serves `main_page.tl`) can serve a copy of this page.
- **TF3 kept TF2's native walls, nearly byte for byte:**
  - **Octree root:** the same two tiers. ±16,384 m at depth 9, and
    ±32,768 m at depth 10 when either axis exceeds 128 tiles. The second
    tier is now set at two sites.
  - **Street/obstacle raster:** the same 32-bit `nx*ny` multiply at 1 m
    cells.
  - **Placement spacing score:** the same int32 squared distances.
  - **Renderer octree-level decoder:** the same 8-ary node-id loop.
- **Stock TF3 stays just inside those walls.** Its longest axis is 250
  tiles, against the octree's 256-tile wall. Its largest area is 12,544
  tiles, against the raster's ~32,768.
- **The first wall a bigger map hits is memory, not code.** One TF3 log
  shows the terrain toolkit used **49 maps and 10,074 MB** to generate a
  stock 56×224 map. That is about 12 MB per km², roughly 3 to 5 times
  TF2's per-km² cost.

## 1. TF3's own limits

### 1.1 The New Game sizes (CONFIRMED-static, scripts)

The labels are in `base/mod.json`, a plain file. It has three `map.size`
entries, chosen by tags:

| Tags | Labels |
|---|---|
| `desktop` | Small, Medium, Large, Very Large (`numbers` = [2,3,4,5]) |
| `desktop` + `experimentalMapFeatures` | Tiny, Small, Medium, Large, Very Large, Huge, Megalomaniac, Gigantomaniac |
| `console` | Small, Medium, Large |

- `map.format` offers 1:1 to 1:5 with the experimental tag, and 1:1 to 1:3
  without it.
- The experimental tag comes from `settings.lua`'s
  `experimentalMapFeatures`, read through `api.util.getAppConfig()` by
  `new_game_react_util.getFilterTags()`, `gui.zip!/gui/menu/new_game_react_util.tl:647-655`.
  The game has no toggle for it. This PC's settings have it on. The last
  game played here was Gigantomaniac 1:2.

The tile counts are hard-coded in `getNumTiles`,
`gui.zip!/gui/menu/new_game_or_map_settings_page.tl:17-76`.

- The index is clamped to the parameter's list first.
- Any unknown index falls back to 16×16.
- One tile is 256 m: 64 cells at 4 m.

| Size | 1:1 | 1:2 | 1:3 | 1:4 | 1:5 |
|---|---|---|---|---|---|
| Tiny | 16² | 10×20 | 8×24 | 8×24 | 6×30 |
| Small | 32² | 22×44 | 18×54 | 16×54 | 14×70 |
| Medium (default) | 44² | 32×64 | 26×78 | 22×88 | 20×100 |
| Large | 56² | 40×80 | 32×96 | 28×112 | 24×126 |
| Very Large | 64² | 44×88 | 36×108 | 32×128 | 28×140 |
| Huge | 80² | 56×112 | 46×138 | 40×160 | 34×170 |
| Megalomaniac | 96² | 66×132 | 54×162 | 48×192 | 42×210 |
| Gigantomaniac ("colossal" in the code) | **112²** (28.7 km) | 80×160 | 64×192 | 56×224 | **50×250** (12.8 × 64 km) |

The Tiny 1:4 and Small 1:4 entries repeat other entries, which looks like a
copy-paste slip in the game.

**How the size reaches native code** (same file):

- `createMap` (lines 79-102) and the preview (lines 783-851) build a
  `GameMap` with these fields:
  - `numTilesX/Y`
  - `levels = 6`, giving 64 cells per tile
  - `resolution = (4, 4, 0.05)`
  - `offsetZ = -100`
  - a heightmap of `(nx*64+1)*(ny*64+1)` samples
- The preview generates through the native `builtin.MapPreviewComp`.
  Start Game hands the generated map to `app.startGame2`.
- `api.type.Map.new()` and `app.makeEmptyMap(climate, Vec2i, water)` both
  take any integer tile count.
- **The only size checks in Lua are the clamp above and one save check.**
  `savegame_react_util.tl:51-71`, `isMapFormatAndSizeSupported`, marks a
  save "not supported" if its size is outside the desktop list.
  - On desktop this is a warning only; the save still loads.
  - On console it blocks the load.
- Native code only asserts that sizes are consistent, for example
  `(int)heightmap.size() == (terrainEcs.size.x * dim + 1) * (…y…)`. It has
  no upper bound.

**So the menu limit is a Lua matter, but a normal mod cannot change it:**

- `script_param_util.getScriptParam` reads parameters only from the base
  mod, so a mod's own `mod.json` cannot add a size row.
- The game applies a mod's files only once a game is loaded. That is why
  TPF3-MP's hook redirects `gui/menu/main_page.tl` at the Lua loader
  (`crates/tpf3mp-hook/src/menu_entry.rs`; confirmed in game,
  investigation/TPF3_INGAME_MENU_2026-09-30.md §1).
- **The same redirect can serve a copy of
  `new_game_or_map_settings_page.tl`.** The copy would carry a longer
  `getNumTiles` and add its own size rows and labels, instead of reading
  them from `base/mod.json`.
  - This replaces all of TF2's menu work: the `GetNumTilesNew` detour, the
    combo-factory hook, the ratio loop bound and the label formatter.
  - The copy follows the same "marked blocks, re-apply on a game patch"
    rule as `main_page.tl` (docs/LOBBY.md, "The mod's copies").
- **Without the hook** (a game started by Steam), the only way to the same
  sizes is to edit the game's own `base/mod.json` and `gui.zip`. The
  project should not do that.

### 1.2 Terrain structure (CONFIRMED-static unless marked)

- **Base heightmap: 4 m samples, 64×64 cells per tile.** From the scripts
  above.
- **A refined high-resolution level exists, as in TF2.**
  - The assert `terrainEcs.baseLevels < terrainEcs.highLevels` is in
    `terrain_util::BaseGetHeightmapRefined` at rva `0x491bf0` (named by
    `tpfre match` from TF2).
  - The engine `Terrain` type carries `baseLevels`, `highLevels` and
    `baseResolution` (`api/tealdef/api/engine.d.tl:1221-1235`).
  - TF2's 257² `uint16` cache constant (`0x20402`) does not occur in TF3's
    `.text`. The high-resolution size is probably derived from `highLevels`
    instead of fixed. **NOT FOUND YET.**
- **The material-index cell is the same size as TF2's.** The immediate
  `0x10811` (67,601 bytes, TF2's material cell) occurs exactly once in
  `.text`. It is at `0x370d71`, inside `sub_36f4b0`, the function that
  prints "Initial material index generation". TF2 has the same pair.
- **The functions TF2's pagers and fast paths sit on are present:**
  - `CTerrain::AddTile` at `0x387f50` (its own assert string)
  - `ecs::TerrainAlignmentSystem::UpdateSubterrains` (RTTI lambda names)
  - The terrain-toolkit teardown line `Terrain toolkit used {} maps and {}
    MB...` (`0x36929e8`, from `sub_353ab0`)
- **TF3 has new VRAM budgets** for terrain heightmap and material textures:
  `vramBudgetTerrain*`. On this PC (MEASURED, game log): heightmaps 640 MB
  default, 1,799 / 2,099 / 2,399 MB for cache / soft / hard. TF2 had no
  such budget.

### 1.3 The octree (CONFIRMED-static)

TF3 rewrote the octree as an ECS octree: `game/ecs/EcsOctree.h`,
`OctreeSystem.cpp`, nodes as `EcsOctreeNode` components. TF2's
`lib/util/octree.h` is gone. **The root is still TF2's two-tier
constant:**

- **Default root:** `sub_1ef0d0`, at `0x1f0445`, constructs
  `ecs::OctreeSystem` (ctor `0xae0780`, which stores the vtable) with these
  arguments:
  - `xmm3 = [0x3683034]` = **16384.0f**
  - `r8d = 9`
- **Large-map root:** `OctreeSystem::Resize`, now `sub_ae4c50`, 101 bytes.
  It stores ±half and the depth at `+0x28`. It is called only when
  `numTilesX > 128 || numTilesY > 128`, with `xmm2 = [0x3683038]` =
  **32768.0f** and `edx = 10`. There are **two call sites**:
  - **`0x244b8b`** in `sub_244b00`, reached from `CGame::InitGame`,
    `InitNewGame` and `sub_11c000`. This matches TF2's single site
    `0x2304f8` in `0x230440`.
  - **`0x20267d`** in `sub_2024a0`, a `gamestate.cpp` function that
    `GameState::Load`'s lambda tail-calls. In TF2 this was the same
    function; TF3 inlined it there.
  - How this was verified:
    - Both sites have the same shape as TF2's: `cmp [r],0x80 / jg / cmp
      [r+4],0x80 / jle`, then `vmovss xmm2,[rip→32768.0f]`, then `mov
      edx,0xa`, then the call, and then TF3's `ecs::GridCollision::InitializeGrid`
      (matched from TF2 by five strings).
    - The constants were read from the PE.
    - The pattern `C5 FA 10 15 ?? ?? ?? ?? 48 8B ?? 38 BA 0A 00 00 00 E8`
      matches exactly these two sites in `.text`.
  - The 32768.0f at `0x3683038` has four readers: these two sites and two
    in `sub_2e9c180`. As in TF2, patch the instruction, never the
    constant.
- **The renderer's node-id decoder is unchanged.**
  `` `anonymous-namespace'::CalcOctreeLevel `` (TF2 `0x853d10`) is
  **`sub_818b30`** in TF3.
  - Verified by its own assert string, `UI/Util/OctreeSkipManager.cpp`, and
    the identical inlined loop at `0x818b70-0x818b84`: `add r9d,r8d; inc;
    lea r8d,[r8*8]; lea eax,[r9+r8]; cmp edx,eax; jge`.
  - The prologue is unique in `.text`: `48 89 4C 24 08 57 48 83 EC 50 49 8B
    F8 4C 8B D1`.
  - So node ids still follow `8*parent+1+octant` in 32 bits. **Depth 11
    is still the deepest the stock id scheme allows (DERIVED).** Deeper
    trees need TF2's compact-id scheme.
  - TF2's other depth-12 site, the recursive descent (`0xa507e0`), was
    rewritten as `detail::EcsOctreeIterator`. **NOT FOUND YET.**
- **TF3 has a new repair pass.** The string "Duplicate base nodes
  deduplication succeeded. Merged into base node entity ID {}" is used by
  `FixNodesAtPosition` (`0xbd4cd0`). TF2 only failed here.
  - The symptom past the octree root might therefore be quieter in TF3:
    nodes merged instead of a failed assert.
  - The cause is still there: positions beyond the root are invisible to
    lookups. A quieter failure is harder to notice.

### 1.4 Where each wall falls (DERIVED, for TF3's 256 m tiles)

| Wall | Formula | Largest square | Stock TF3's worst case |
|---|---|---|---|
| Octree root, depth 10 | either axis > 256 tiles (±32,768 m, map centred as in TF2) | 256² | 250-tile long axis (1:5): inside by 768 m per side |
| Street/obstacle raster | `(256·nx+1)·(256·ny+1) ≤ 2³¹−1` at 1 m cells | 180² (46 km); at 1:5, 80×400 | 50×250 → 0.82 e9 cells (38% of the limit) |
| Placement spacing | pairwise separation ≤ √(2³¹) px × 4 m = **185.4 km** | not a size limit: about 131 km square | 65 km diagonal |
| Heightmap element count | `(64·nx+1)·(64·ny+1) ≤ 2³¹−1` | about 724² | far inside |
| Positions on the wire (D8) | `i32` millimetres = ±2,147 km | far beyond depth 13 (±262 km) | no issue |

**The ladder for squares is therefore:**

1. 112² (stock).
2. 180² needs no native patch.
3. Beyond 180², the raster needs its fix.
4. Beyond 256 tiles on any axis, the octree root needs depth 11.
5. Beyond 512 tiles, depth 12 or more is needed, with new ids.

Long thin maps reach the octree wall first: 1:5 at 52×260 already crosses
it.

### 1.5 Memory: the binding constraint (MEASURED, one log)

The game's own log
(`userdata/…/3493540/local/crash_dump/d9ded1c3-…_1008.txt`, 2026-09-30)
reads:

```
Terrain toolkit used 49 maps and 10074 MB...
  Pipeline took: 19.44983s
```

The climate was subarctic. TF2's law is `MB = (64nx+1)(64ny+1) × N × 4 /
10⁶`, truncated. With N = 49 it gives exactly 10,074 for **56×224**
(Gigantomaniac 1:4). 112² would give 10,073, so the size is DERIVED.

**Per-km² cost (DERIVED):**

- Each heightmap sample costs 49 × 4 = 196 bytes, which is about
  **12.3 MB per km²** of terrain-toolkit memory.
- TF2 desert used 18 maps, about 4.5 MB/km² by the same formula.
- If TF3 also keeps every map alive until teardown, the toolkit alone
  would need:
  - about 26 GB at 180²
  - about 53 GB at 256²
  - about 211 GB at 512²
- **Unknowns.** Whether all 49 maps are alive at once in TF3 is
  NOT MEASURED. Neither is the runtime footprint after entry.

**What this means:**

- **The first useful TF3 step is a moderately bigger map, not a huge
  one.**
- **The generator's buffer reuse is worth more in TF3 than in TF2.** The
  graphs are Lua data
  (`climates.zip!/climates/<climate>/<climate>_gen.tree.lua`, a node graph
  of 8,864 lines for subarctic, not TF2's layer list). The idea carries
  over from tpf2-bigmap's `bigmap_memory.lua`. The analysis must be redone
  for nodes.

## 2. Each Big Maps feature, mapped to TF3

Effort: S = days, M = a week or two, L = several weeks.

| TF2 feature (site, build 35924) | TF3 | Needed? | Lua or native | Effort | TF3 site |
|---|---|---|---|---|---|
| **Size detour:** `GetNumTilesNew` `0x674aa0`, 224 clamp | `getNumTiles` in `new_game_or_map_settings_page.tl` | yes, it is the feature | **Lua**, through the hook's loader redirect | S | script, CONFIRMED |
| **Added size rows:** combo factory `0x2326290`; **ratios** `0x662930` / `0x880940` | Same page: the rows come from `base/mod.json`'s `map.size`/`map.format` through `getScriptParam` | yes | **Lua**, in the page copy (labels can now say what they do) | S | script |
| **Street raster** scaling: Obstacle ctor `0x90d410`, `imul eax,[rbx+0x40]` | **Obstacle ctor `sub_8cea50`**. The multiply `imul eax,edx; movsxd rdx,eax` is at `0x8ceae5`. Callers: `init_streets_util.cpp` town connection (`sub_8feba0`, at `0x8ff06d`, cell `[0x3676644]` = 1.0 m), `industry_util::CreateAndAddIndustry` (`0x8f1ca0`), `sub_912940`, and the native map preview `builtinmappreviewcomp.cpp` (`0x28dca20`). | Only past 180² | native detour of the ctor that grows `cellSize` (xmm2), as in TF2 | M | **found at rva 0x8cea50** (verified by: the `Obstacle::vftable` store, the identical divide/truncate/multiply body, and a unique 27-byte signature from `tpfre sig`) |
| **Octree root** depth 10 → 11, 13-byte rewrite at `0x2304f8` | **Two sites**, `0x244b8b` and `0x20267d`, each 17 bytes (see §4) | Only past 256 tiles on any axis | native in-place rewrite at both sites | S-M | **found** (§1.3) |
| **Octree depth 12/13:** descent hook `0xa507e0` + level decoder `0x853d30` | Decoder **`sub_818b30`**, same loop. The descent was rewritten (`EcsOctreeIterator`). | Only past 512 tiles; impractical given §1.5 | native | L | decoder found; descent **NOT FOUND YET** |
| **Placement distance:** int32 squares in `0x910ce0` | **`sub_8d31f0`**: `imul edx,edx; imul eax,eax` at `0x8d334b`, and again at `0x8d33ab`, before `vcvtsi2ss`/`vsqrtss`. Reached from `0x8d6390`, near `RandomLocationFactory::RandomLocationFactory` (`0x8d2310`, matched by two shared assert strings). | Only when two candidates can be more than 185 km apart | native: replace the function (TF2 did int64 with saturation) | M | **found** (by the loop shape; the 25-byte pattern at `0x8d3338` is unique). Ties to TF2 by shape, not by name. |
| **Placement attempts** 200 → 50 (`mov r9d,200` at `0x912f59`) | — | optional generation speed | native | S | **NOT FOUND YET** (`41 B9 C8 00 00 00` is absent around `0x8c0000-0x8e0000`) |
| **Density levels:** native Towns switch `0x65b620` + text inserted into `base_mod.lua` | `base.zip!/base/difficulty_util.tl:36-85`, applied in `mod.script.tl` `preRunFn` (lines 193-200). The densities are `maxNumberPerArea` towns 0.2/km² and industries 0.8/km² (lines 117-133). | Useful: counts scale with area | **Lua**, in a mod or the page copy | S | script |
| **New Game preview** (a native preview in tpf2-bigmap; see `docs/minimap-native-preview.md`) | TF3 has a native `builtin.MapPreviewComp` that generates the whole map: terrain, towns, industries. Lua reuses it unless its inputs change (lines 815-827). | The preview is already native. On big maps it is a full generation per change, so §1.5 applies to the preview too. | Lua (debounce, or skip towns in the preview) | S | script |
| **Minimap** on M | Already planned as a script mod (docs/MINIMAP.md) | yes | Lua (terrain picture later from the hook) | in progress | — |
| **Terrain cache pager** (1 m heights) + **material pager** | `CTerrain::AddTile` `0x387f50`; material cell 67,601 B (§1.2) | Only if a size outgrows RAM after entry | native, large; TF2's pager is Windows section/VEH machinery | L | partly located |
| **Alignment batching** (`UpdateSubterrains`, 35 → 8.3 GiB peak) | `TerrainAlignmentSystem::UpdateSubterrains` (RTTI) | Only if TF3's load also feeds the whole map at once (measure) | native | M | function located by RTTI lambda; site not resolved |
| **Bit-identical fast paths** (refine, align blend, min/max, block copy) | `BaseGetHeightmapRefined` `0x491bf0` | No: speed only, risky, and every peer must be bit-identical | native | L each | refine located |
| **save_fast** (zstd level 3 → 1, 64 KiB buffer) | not looked for | Maybe: room autosaves (OPERATIONS.md "Big maps") | native | M | **NOT FOUND YET** |
| **Generator buffer reuse** (`bigmap_memory.lua`) | `climates/*/…_gen.tree.lua` node graphs | **Yes, most valuable** (§1.5) | **Lua** (served copies) | M-L | script |
| **World-entry timers** | `InitNewGame` and its family, matched | diagnostics | native, hook-side | S | partly |
| Travel-time limits | not looked for | no | — | — | — |

## 3. Multiplayer constraints

**What the room already guarantees:**

- **Map size is fixed by the save.** A room starts from a world one machine
  provides. Every game loads that save, the owner's included (PROTOCOL.md,
  "The first world"; LOBBY.md, "Changing the start save"). The save holds
  the tile counts (`SaveGameDetails.terrainDimensions`), so size, terrain,
  towns and industries are the same for everyone by construction.
- **So these only need to run on the machine that creates the world:**
  - the added menu rows
  - the preview
  - generator memory
  - placement attempts

**What must match on every game in the room:**

- **The octree root and depth.** These are not in the save. Each game
  derives them at world allocation from the tile counts and the code at
  §1.3. A game without the patch that loads a world wider than 256 tiles
  silently loses entities beyond 32.8 km to lookups. TF3 then "repairs"
  duplicate nodes, so the save is damaged and the games diverge.
- **The street/obstacle raster cell.** At runtime
  `industry_util::CreateAndAddIndustry` builds the raster, and so does the
  street initialiser reached from `Visitor::operator()` (`0x9de5c0`). Above
  180², a game without the fix aborts; with different cell sizes, games
  place differently.
- **Placement distance.** Runtime industry founding goes through
  `RandomLocationFactory`, as in TF2 (docs/BIGMAPS.md). Above 185 km
  pairwise, the int32 and int64 variants pick different spots.
- **Density levels**, if they change `maxNumberPerArea` and the industry
  spawner's target at runtime. A Lua mod doing this is a **shared** mod
  under D25 / MODS.md, so the existing content fingerprint already compares
  it.

**Design rule this suggests: derive the native terms from the world, not
from a setting.**

- Apply the octree, raster and placement patches **only when the loaded
  world's tile counts need them**:
  - octree depth 11 if any axis is over 256 tiles
  - the raster cell only when `nx*ny` would overflow
  - placement int64 only when the map's diagonal is over 185 km
- Every stock-sized world then runs byte-for-byte stock code on every
  game, whether or not the player has big-map support. All games given the
  same save make the same choice.
- TF2's `octree_depth = 13` applied a larger root to every load. That
  setting would be a desync on stock maps between players with and without
  it, so do not carry it over.
- The room then only has to check that every game **can** apply what the
  world needs:
  - a build whose profile resolves the targets
  - a hook new enough to know the rule
- This is the `features.rs` "Refused" state. Report it as one more entry
  in the content manifest: the hook declares a pseudo-mod such as
  `tpf3mp:bigmap` with version `rule1`, next to `OWN_MOD`, so the existing
  fingerprint comparison refuses a mismatch with no protocol change. The
  prototype's `terms.rs` fingerprint becomes unnecessary for the derived
  terms, apart from density.

**Decisions and plan, checked:**

- **PLAN.md, Part 4, "Big maps": *open: say what is wanted*.** The owner has
  not decided to ship big maps. This file informs that decision and does
  not make it. Any change to PLAN.md or DECISIONS.md needs the owner's
  approval (AGENTS.md).
- **D14** (tpfre for reverse engineering): followed. Everything above came
  from tpfre, and the proposed targets would be profile entries with
  `tpfre sig --toml` signatures and prologues, marked `required = false`.
- **D6** (the host picks rules at creation, fixed for the room's life): no
  conflict if the terms are derived from the world. A big-map *rule* would
  conflict with D6's "a new room is the way to switch" only if a room could
  change its save size later. It cannot: a room's world size is fixed by
  its first save.
- **D11** (the hook runs only in games the launcher starts): **conflict
  risk.** A big-map save opened in a game started by Steam has no hook and
  no octree patch. It loads with no warning and damages itself.
  - TF2 Big Maps had the same exposure through its alut proxy.
  - Mitigation: in its marked blocks of the page copy, refuse to *create*
    sizes above 256 tiles unless the hook is present. Accept that loading
    outside the launcher cannot be guarded.
  - Flag this to the owner.
- **D8** (`i32` millimetres): no conflict up to ±2,147 km.
- **D25** (personal mods): the page copy only changes the menu, so it is
  personal. Density changes are shared.
- **D17 / D24** (the room's lobby inside the game, the window picks the
  start save): "New world: choose map and settings" is the path where
  bigger sizes would be offered. Only the owner's game needs the page copy.
- **Timeouts** (OPERATIONS.md "Big maps", PROTOCOL.md): already noted. A
  big-map load or autosave outgrows the 5-minute load and 20-second stall
  defaults. The server flags exist.
- **Snapshots:** save size grows with area. The 1.4 GB TF2 example implies
  long uploads at the start of a room (OPERATIONS.md).

## 4. Staged plan

### Stage 0: measure without patching

Do this on the stock game, with no code. It is a human-run or rig-run
playtest; automation never launches the game.

1. Generate stock Gigantomaniac at 1:1 and at 1:5 for each climate.
2. From `stdout`/crash_dump, record `Terrain toolkit used N maps`, the
   pipeline time and the stage times.
3. Read the process's private bytes:
   - at the end of generation
   - after entry
   - after a load of the save
4. Save and time an autosave, and note the save size.

This gives TF3's real per-km² law, including whether the 49 maps coexist.
Without it nothing bigger is worth building. Run this on strelka under
Proton as well, since that machine has room for memory-heavy runs.

### Stage 1: Lua only, up to 180² (the smallest useful step)

- Ship a copy of `new_game_or_map_settings_page.tl` in
  `mod/tpf3mp_bigmap_1` (or `tpf3mp_1`), served by the hook's existing
  loader redirect. Add one map path to its table next to `main_page.tl`.
  The copy adds rows from the prototype's `ladder.lua`:
  - 128² (32.8 km)
  - 144²
  - 160²
  - 176²
  - and the matching 1:2 to 1:5 shapes, each kept below **both** the
    raster's area limit and 256 tiles on any axis
- No native patch: §1.4 shows these sizes stay inside every wall stock TF3
  has. So the world is safe to hand to stock games in a room, and to
  players without the feature.
- In the copy, cap the offered sizes from the machine's RAM using the
  Stage 0 law. Say why a row is missing.
- Patch `isMapFormatAndSizeSupported`, or leave its warning in place.
- **Test (no game):**
  - a Teal/Lua parse of the copy (as `tools/probe/check_lua.py` does)
  - the prototype's `mod_lua.rs` test regenerating `ladder.lua`
  - a diff check that the copy equals the game's file plus marked blocks
- **Test (game, by a person):** generate 128² and 176². Watch the log for
  `Duplicate base nodes`. Play a room of two games (Sandboxie) through the
  scenario runner (`tools/scenarios/roads.json`, `rail.json`) and look for
  lane divergence at checkpoints.
- **Risk:** memory only. At about 12 MB/km², 176² is about 2,030 km² and
  about 25 GB for the toolkit, if all maps coexist.

### Stage 2: the octree root, to 512 tiles on an axis

- **Profile targets:**
  - `bigmap::octree_root_init` = `0x244b8b`
  - `bigmap::octree_root_load` = `0x20267d`
  - the signature from §1.3, with exact prologues, `required = false`
- **Rewrite each 17-byte block in place**, written last after both sites
  verify. The TF2 shape applied to TF3's AVX encoding:

  ```
  stock   c5 fa 10 15 <rel32>   vmovss xmm2,[32768.0f]
          48 8b 4e 38           mov rcx,[rsi+0x38]      (site 2: 48 8b 4d 38, [rbp+0x38])
          ba 0a 00 00 00        mov edx,10
  patch   b8 00 00 80 47        mov eax,0x47800000      ; 65536.0f
          c5 f9 6e d0           vmovd xmm2,eax
          48 8b 4e 38           mov rcx,[rsi+0x38]      (kept)
          31 d2 b2 0b           xor edx,edx ; mov dl,11
  ```

  - `eax` is dead here: the next instruction is the `call`, and `Resize`
    takes only rcx, edx and xmm2.
  - The flags are dead after the `jle`.
- **Patch only while the world being allocated has an axis over 256
  tiles.** The patched code sits behind the `>128` branch, so it must not
  change 129-256-tile worlds. Two ways:
  - (a) a 5-byte jump to a stub that checks `[rbp]`/`[rbp+4] > 256`
  - (b) arm the patch from the hook before a load, once it knows the save's
    tile counts

  (a) is self-contained and keeps stock behaviour for every stock-sized
  map.
- **Offline test (as tpf2-bigmap did):** run the original block and the
  stub on the mapped exe (Unicorn), compare `Resize`'s stored box and
  depth for 128, 129, 256, 257 and 512 tiles, and confirm stock output is
  bit-identical at or below 256.
- **Game test:** generate a 1:5 at 60×300. Check that no `Duplicate base
  nodes` line appears, and that towns beyond 32.8 km grow.
- **Save compatibility:**
  - A save wider than 256 tiles requires the patch on every later load.
    Without it the save is damaged.
  - Saves of up to 256 tiles are unaffected.
- **Desync:** every game derives the same depth from the same save. The
  content manifest entry from §3 refuses a game whose hook lacks the rule.

### Stage 3: the raster, past 180²

- Detour `sub_8cea50` (prologue `48 89 4C 24 08 53 48 83 EC 20 48 8B D9 48
  8D 05 <rel32>`; the rel32 must be relocated by the detour engine). Grow
  `cellSize` until `nx*ny ≤ budget`, as tpf2-bigmap did.
- Never widen the multiply. The indices stay int32 downstream.
- It changes runtime industry founding, so it is a simulation term:
  derived from the world's size, identical on every game.

### Stage 4: only if Stage 0 says it is worth it

- **Placement int64** (`sub_8d31f0`): only for maps whose diagonal passes
  185 km, so it pairs with depth 12+.
- **Generator buffer reuse** for the TF3 node graphs: Lua, generation-only,
  owner's machine only. This is the largest win if the 49 maps coexist.
- **A terrain/material pager or alignment batching:** only if a measured
  runtime footprint after entry, not generation, exceeds what players have.
  This is TF2's largest and riskiest work, and it needs the Windows memory
  machinery under Proton too.
- **Depth 12/13:** needs the `EcsOctreeIterator` descent found and TF2's
  compact ids carried over. Given §1.5, a 1,024-tile map would need about
  840 GB for generation, so this should not be pursued.

### Risks, in order

1. **Memory.** Generation alone at about 12 MB/km² (to be measured) caps
   sizes long before the code walls. Players in a room must also *load*
   the world: every game, not just the owner's.
2. **Save compatibility.** A world wider than 256 tiles is unsafe in any
   game without the hook (D11). Saves stamp nothing that says so.
3. **Desync.** Any term that differs between games: octree depth, raster
   cell, placement arithmetic, density. The rule "derived from the world,
   stock at stock sizes" and the content-manifest entry address it.
   Checkpoints compare lanes, and divergence there is the test.
4. **Game patches.** The page copy and four native sites must be
   re-verified each build. `tpfre match` carried 10,368 names in two
   seconds, and the static-proof tests (`tf3_static_proof.rs`, with
   `TPF3MP_TF3_EXE`) should gain each new target.
5. **Timeouts and transfer:** load, stall and snapshot upload sizes
   (OPERATIONS.md).

### How to test, overall

**Static (CI-safe, no game):**

- profile signatures unique, with exact prologues, in `tf3_static_proof.rs`
- Unicorn-style offline equivalence for each native rewrite against the
  original bytes
- Lua parse and copy-diff checks for the page

**In game (by a person or the rig, never by automation without being
asked):**

- Stage 0 measurements
- two-instance rooms (Sandboxie) with the scenario runner at each new size
  step
- checkpoints must agree

**Memory-heavy runs:** strelka under Proton (the Windows hook works there),
never while someone is playing on it.

## Appendix: TF3 addresses (build 40408, image base 0x140000000)

| Role | RVA | How it was identified |
|---|---|---|
| OctreeSystem ctor (default ±16,384, depth 9) | `0xae0780`, called at `0x1f045a` | stores `ecs::OctreeSystem::vftable`; `[0x3683034]`=16384.0f, `r8d=9` |
| `OctreeSystem::Resize` | `0xae4c50` | references "OctreeSystem"; stores ±half and depth `+0x28` |
| Octree root site, init / new game | `0x244b8b` (fn `0x244b00`) | TF2 `0x2304f8` shape; precedes `GridCollision::InitializeGrid` |
| Octree root site, load | `0x20267d` (fn `0x2024a0`) | same; `gamestate.cpp` |
| `CalcOctreeLevel` (renderer node-id level) | `0x818b30` | assert string + identical loop |
| Obstacle (street/collision raster) ctor | `0x8cea50`, multiply `0x8ceae5` | `Obstacle::vftable` store; TF2 `0x90d410` body |
| Town connections (street raster caller, 1 m cell) | `0x8feba0`, call `0x8ff06d` | "Towns '{}' and '{}' were already connected"; `init_streets_util.cpp` |
| `RandomLocationFactory` ctor | `0x8d2310` | matched by two shared assert strings |
| Placement spacing score (int32 squares) | `0x8d31f0`; `0x8d334b`, `0x8d33ab` | TF2 `0x910ce0` loop shape |
| `ecs::GridCollision::InitializeGrid` | `0xba4750` | matched by five strings |
| `industry_util::CreateAndAddIndustry` | `0x8f1ca0` | matched by string |
| `terrain_util::BaseGetHeightmapRefined` | `0x491bf0` | own assert |
| `CTerrain::AddTile` | `0x387f50` | own assert |
| Material index generation (67,601 B cells) | `0x36f4b0`, imm at `0x370d71` | string + unique immediate |
| Terrain toolkit teardown print | `0x353ab0` | string |
| Duplicate-base-node repair | `0xbd0440`, `FixNodesAtPosition` `0xbd4cd0` | strings |

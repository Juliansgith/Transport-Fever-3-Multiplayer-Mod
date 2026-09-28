# Porting TpF2 Multiplayer's Lua mod

TpF2 Multiplayer (`tpf2-multiplayer` by silver2127, MIT) ships its game
side as one Lua mod, `mod/mp_lockstep_1`: about 21,500 lines in 31 files.
This page says what of it comes to TPF3-MP's mod (`mod/tpf3mp_1`), in what
shape, and in what order. Credit each ported file to its source, as
`tpf3mp/roads.lua` does.

## What changes

In `mp_lockstep_1` the Lua does nearly everything:

- it replays commands at a game time;
- it keeps players in step: reliable delivery, pacing, catch-up and resync;
- it talks to its DLLs through files polled with `io.open`;
- it draws its panel with TPF2's widget API.

In TPF3-MP most of that is Rust:

- the server orders turns (D2);
- the agent paces them and holds snapshots;
- the hook gates the steps and carries the link (ARCHITECTURE.md).

The mod keeps only what needs the game's script API:

- turning a player's action into an `Action` (D8);
- applying an `Action` with `api.cmd`;
- reading what the world looks like.

Transport Fever 3 differs from TPF2 as well (see
[investigation/TF3_MODS_2026-09-27.md](../investigation/TF3_MODS_2026-09-27.md)):

- a mod is `mod.json`, `_content.json` and `content/`;
- the GUI is Teal code on a react-style framework;
- no mod made for build 40391 uses a game script, `io`, `os` or
  `game.interface`;
- command factories are named `api.cmd.make<Subject><Verb>Cmd`, where
  TPF2 had `api.cmd.make.<verb>`.

## What happens to each module

**Left behind: Rust and the hook replace it.**

| module | lines | replaced by |
|---|---|---|
| `net.lua` (encoding, resending, scheduling) | 1,485 | the server's turns over QUIC |
| `pacing.lua` (speed, catch-up, load gate) | 1,796 | the agent's playout and the hook's step gate |
| `inject.lua` (the capture file reader) | 1,751 | the hook's capture; its conversions move into the modules below, as the road capture's did |
| `io.lua`, `resync.lua`, `stats.lua` | 550 | the agent's link, snapshots and the launcher |
| `lockstep.lua`'s dispatcher, status files and world token | ~2,000 | `Session` in the hook, and a small apply dispatcher in the mod |

**Ported as pure Lua.** This is where TPF2's lessons live. Each module
splits in two:

- a pure part: a capture to an `Action`, or an `Action` to commands;
- an adapter over the game's API (like `tpf3mp/engine.lua`), the only
  file that calls `api.*`.

Tests run the pure part against a world of tables
(`crates/tpf3mp-proto/tests/lua_capture.rs`).

| module | lines | becomes | status |
|---|---|---|---|
| `geom.lua`, `roads.lua` | 2,200 | road and track capture; applying them next | capture done (`tpf3mp/geom.lua`, `roads.lua`) |
| `cons.lua`, `conx.lua` | 3,400 | constructions, station edits, upgrades, demolish | |
| `stops.lua`, `waypoints.lua` | 950 | stops, signals, waypoints | |
| `lines.lua`, `vehicles.lua` | 2,350 | lines, vehicles, names and colours, and their identity across games | |
| `companies.lua`, `shared_infra.lua` | 1,300 | companies and who owns what | |
| `terrain.lua`, `assets.lua`, `sandbox.lua` | 340 | terraform, paint, the asset brush, sandbox towns | |
| `hash.lua` | 880 | the lanes the hook reports (`Game::lanes`) | |

TPF2's text lines (`LCREATE`, `VBUY`, `CONX`, about 60 kinds) do not come
across: each becomes an `Action` variant, in millimetres, with ids the
server assigns (D8).

**Rewritten for the new GUI**, about 1,500 lines:

- `cursors.lua`, `previews.lua`, `preview_constructions.lua` and
  `action_sounds.lua`;
- the style sheet, and the dashboard in `lockstep.lua`.

They use TPF2's widgets (`api.gui.comp.*`, `api.gui.util.getById`). TF3's
GUI takes react-style recipes instead:

- the panel is a `GameBarInfoDisplayExtension` plugin, as the mod's
  loader already is;
- the icons are TF3's own style, measured from its bottom bar
  (`mod/tpf3mp_1/content/gui/tpf3mp/icons/`, drawn by
  `tools/art/icons/`): the Multiplayer and Minimap buttons after TpF2
  Multiplayer's and TPF2 Big Maps', Chat and Invite for the room panel,
  each normal, selected and unavailable, and white marker glyphs for
  other players' builds after TpF2 Multiplayer's HUD glyphs;
- views of one town or vehicle use the entity-window extension points;
- changes to stock UI go through `react-replacement-config`.

**Left behind: compatibility with TPF2 mods.** `deterministic_script.lua`,
`autosig_compat.lua` and `fences_compat.lua` wrap particular TPF2 Workshop
mods. TF3's mods need a rule of their own (PLAN.md, Part 3: mods that send
commands from the GUI).

## The script API

Seen in mods made for build 40391:

| TPF2 (uses in `mp_lockstep_1`) | TF3 |
|---|---|
| `api.cmd.make.createLine` (2) | `api.cmd.makeLineCreateCmd(name, color, player, line)` |
| `api.cmd.make.createTowns` (1) | `api.cmd.makeTownCreateCmd({TownInfo})` |
| `api.cmd.make.setName` (4) | `api.cmd.makeEntitySetNameCmd(entity, name)` |
| `api.cmd.make.setGameSpeed` (2) | `api.cmd.makeGameSetSpeedCmd` (named in a comment) |
| `api.cmd.make.bookJournalEntry` (1) | perhaps `api.cmd.makeJournalBookAssetCmd(player, entry, pos)` |
| `game.interface.getEntities` (42) | `api.engine.getEntitiesWithComponent` |
| `game.interface.getEntity` (32) | `api.engine.getComponent` |
| `game.interface.getGameTime` (4) | the `GAME_TIME` component of `api.engine.util.getWorld()` |
| save() / load() of the game script | `api.gui.game.setGuiSaveData(key, table)` / `getGuiSaveData(key)` |

Not seen yet: read the real names from the game's `.d.tl` declarations on
release day.

- `buildProposal` (19 uses), `updateLine`, `deleteLine`, `setColor`,
  `buyVehicle`, `sellVehicle`, `replaceVehicle`, `setLine`, `sendToDepot`,
  `reverseVehicle` and `setUserStopped`.
- `game.interface`'s `buildConstruction`, `bulldoze` and
  `upgradeConstruction`, which become proposals.
- `setPlayer` and `addPlayer`, which companies mode depends on.
- `setZone`, `setBulldozeable`, `setMaximumLoan`, `setDate` and
  `setMillisPerDay`.

## Three changes of structure

1. **When actions are applied.** TPF2 applied them in the game script's
   `update()`, once per step. TF3 may have no game scripts. The hook holds
   each step anyway (`Session::before_step`), so it calls the mod's
   `apply` handler there, and the mod sends the commands with
   `api.cmd.sendCommand`. Release day must confirm that a command sent
   there runs in that step. If game scripts exist after all, they can do
   what they did on TPF2.
2. **The link to the hook.** No files: the hook registers
   `tpf3mp_native` in the mod's Lua state, and the mod registers its
   handlers with it (`tpf3mp/bridge.lua`, and "The Lua side" in
   [HOOKS.md](HOOKS.md)). Neither needs `io` or `os`. Actions cross it
   as tables in the game's units; the hook converts them with
   `tpf3mp_proto::lua`, so the mod has no encoder (D15).
3. **What the save carries.** TPF2's `save()` held vehicle and line keys,
   company state and the step it was saved at. In TF3 that is
   `setGuiSaveData("tpf3mp", ...)`. With ids from the server there is
   less to carry.

If TF3's builders send their commands through `api.cmd.sendCommand`,
capture may move into Lua as well, with a flag around the mod's own
replays so they are not captured again (DAY_ONE.md §6).

## Order

1. Before release: the mod in TF3's layout, loading as a game bar plugin,
   with the bridge to the hook. **Done**: `mod/tpf3mp_1`, tested by
   `crates/tpf3mp-proto/tests/lua_mod.rs` in a stand-in for the GUI state.
2. PLAN Part 2:
   - applying roads and track (`roads.lua`'s receiving side);
   - speed and pause;
   - the lanes from `hash.lua`;
   - then Test A.
3. PLAN Part 3, one module at a time in the plan's order, each behind its
   `strict_<action>` flag: stops and signals, lines, vehicles,
   constructions, terrain and assets, companies, loans.
4. The panel, cursors and previews, last: they change nothing in the world.

The mod stays plain Lua, which the game loads and the tests run. Once the
game's `.d.tl` files are in hand, checking the mod against them with
Teal's `tl` in CI would catch a wrong API name before a player does.

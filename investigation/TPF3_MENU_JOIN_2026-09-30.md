# Joining a room from the main menu: static findings -- 2026-09-30

The owner asked: "can we get it working from the main menu?" A guest must be
able to join a room and receive the room's world while their game sits at
the **main menu**, without loading a save first. Until now the room's save
was loaded only by the mod's in-game GUI (`poll() -> { load = name }`,
`app.loadGame`), which runs only inside a world, and auto-ready
(`ToAgent::WorldUp`) fired only once a world was up.

Built on `feat/join-from-menu` (`crates/tpf3mp-hook/src/menu.rs`,
`StepDriver::on_menu`, `install::menu_frame`, `ToAgent::MenuUp`; bridge
version 8 as ported onto dev). Design in [docs/HOOKS.md](../docs/HOOKS.md), "Loading from the
main menu". The game was not launched; everything below is from Steam build
40408 (`de1daad3...`) on disk: `tools/tpfre` over `TransportFever3.exe`,
the game's `base/content/gui.zip` Teal sources and `api/tealdef/*.d.tl`.

Marks: **CONFIRMED-static** = the binary or the game's own scripts show it
(code read, a string, RTTI, a call shape); **INFERRED** = concluded from
shape or analogy, to be seen in the game; **UNKNOWN** = open.

## 1. How the menu loads a save

- **CONFIRMED-static.** Every menu path that loads a save builds the id
  with `api.type.SavegameId.new()` (`path`, `saveGameName`,
  `saveGameNamespace = app.SaveGameNamespace.getSavegame()`) and calls
  `app.loadGame(id, isMapEditor, info)`: the Load Game page
  (`gui/menu/savegame_react_util.tl` 1138-1163, 995-1010), Continue
  (`main_page.tl` 486-494), missions (`mission_page_content.tl` 411-445).
  The same call the mod's GUI already makes in a world
  (`tpf3mp.script.lua`), which loaded the room's world in the two-game
  test of 2026-09-29.
- **CONFIRMED-static.** The main menu's pages first switch to the
  `ProgressPage` and, from its `onMount`, call
  `app.setWaitForStartReadyGame()` then `app.loadGame(...)`. The Start
  Game button exists only then: `progress_page.tl` 105-117 shows it when
  loading is done and `app.isWaitForStartReadyGame()`, and its click is
  `app.startReadyGame()`. The game's API doc (`api/tealdef/app.d.tl`
  177-184): `setWaitForStartReadyGame` "is called when loading a game to
  prevent the game from starting automatically". So a load without it
  starts the world by itself. The hook never calls it.
- **CONFIRMED-static.** The main menu switches to its loading page on its
  own once the progress monitor has a task (`main_menu.tl` 566-569 and
  983-986: `app.getProgressMonitor():getTask() ~= ""`). A load started
  from outside a page still shows the loading screen. The hook uses the
  same task as "busy": it does not start a load while the game is loading
  something.
- **INFERRED.** With `info = nil`, `loadGame` uses the save's own mod list
  (the Load Game page passes a copy of the save's own details and changes
  only `modParams`). The room's save is written by the owner's game through
  the mod's GUI (`app.saveGame`), so TPF3-MP is in its list and is active
  in the loaded world. If it is not, the world's GUI never reports itself
  up, and the hook holds after `LOAD_PATIENCE` (600 s).

## 2. Which Lua state has `app`

- **CONFIRMED-static.** `sub_dc5fa0` (`0xdc5fa0`) is
  `RegisterAppUsertypes`: it names itself (`"RegisterAppUsertypes"` at
  `0xdc5fdc`), and it is the only function that references
  `getProgressMonitor`, `setWaitForStartReadyGame` and
  `isWaitForStartReadyGame`. Its first act is `mov rdi, [r12]` with
  `r12 = rcx`: argument 1 is a `lua::State&` whose first word is the
  `lua_State*` (as `lua::State::State`, `0x2faf4b0`, stores it). Argument 3
  (`r8`) is a `std::function` whose `_Impl` it reads at `+0x38`.
- **CONFIRMED-static.** Its three callers: `0x6916d0` (the CMenuUI
  construction path), `0x697210` (the react menu), `0x6a3190`. The last is
  `UI::CMenuUI::SetupAppScriptInterface(TypeRegistry&, lua::State&, bool,
  std::function<bool()> const&, std::weak_ptr<bool const>)` (named by
  RTTI of its lambda), which passes its `lua::State&` as `rcx` and `this`
  as `rdx`; it is called from `0x6579d0` (the in-game GUI) and
  `0x27c6460` (`ScriptComponentRoot::ReloadInterfaces`, the react UI
  states). So every state that has `app`, the menu's included, passes
  through `0xdc5fa0`.
- **INFERRED.** Those states are made and used on the main thread. The
  hook records the thread each state is adopted on and only calls into a
  state from that thread.
- Prologue `48 89 5C 24 20 55 56 57 41 54 41 55 41 56` (14 bytes:
  `mov [rsp+20h],rbx`, six pushes) is plain; the signature is unique in
  `.text` (`tpfre sig`, and `tpf3_steam_static_proof.rs`).

## 3. The Lua the hook runs there

- **CONFIRMED-static.** `sub_2fbdf70` (`0x2fbdf70`) is Lua 5.2 `lua_load`:
  `luaZ_init(L, &z, reader, data)`, the default chunk name, the parser
  with the 5th (stack) argument as `mode`, then on success the first
  upvalue of the new closure set from the registry's index 2 (`_ENV`, the
  globals). The same signature the `feat/ingame-lobby` menu profile
  resolved in the running game (docs/LOBBY.md there: "menu patch installed
  in Lua state ...", lua_load + lua_pcallk from the hook, on release day).
- **CONFIRMED-static.** `lua_pcallk` (`0x2fbe0c0`) and `luaL_ref`
  (`0x2fb40b0`) were already in the profile
  (investigation/TPF3_SAVE_LUA_2026-09-29.md, section 3).
- The chunk hands the hook `load(name)` through `luaL_ref(L,
  LUA_REGISTRYINDEX)` and makes a sentinel table with `__gc` calling the
  hook's `gone(number)`. **CONFIRMED (Lua 5.2 semantics):** `lua_close`
  runs every pending finalizer, so the hook hears of a state before it is
  freed; the sentinel is an upvalue of `load`, which the registry keeps.
- **INFERRED.** Running a chunk in a state right after the game's own
  registration, from native code with the stack balanced, is safe: it is
  what the ingame-lobby's loader patch did inside `resolveutil.loadfile`,
  and what the game does itself.

## 4. The menu's frame

- **CONFIRMED-static.** `0x6a0160` is `UI::CMenuUI::DoStep`: vtable slot
  50 of `UI::CMenuUI::vftable` (`0x36c7ee0`), the assert string
  `"UI::CMenuUI::DoStep"`, `menuui.cpp`. It keeps `rcx` (this) and `r8`;
  in the entry block read (45 instructions) it reads no `xmm` register
  before writing it. Prologue
  `48 89 5C 24 10 55 56 57 41 54 41 55 41 56` (already in the profile).
- **INFERRED.** It runs every frame on the main thread, at the menu and in
  a game (it branches early on `this+0x6b0`/`+0x7d0` and hands on to the
  game's UI), as `UI::CGameUI::DoStep` (slot 50 of its class) does for the
  game and TPF2's menu update did (`tpf2-multiplayer/native/src/
  menu_hook.cpp`). The hook calls the original first and runs its own
  code after, passing `rax` back.
- "At the menu" is the hook's own measure. First it was "the game's step
  (`GameSim::Step`) has not run for 2 s"; the playtest showed a world
  stops stepping too (saving the room's world, held for another player),
  and the owner's game hung. It is now: this game's step has never run,
  and no world's GUI has started (`tpf3mp_native.world`), measured on the
  same playtest (the owner's menu frame once took the session while
  saving the room's world before its first step).

## 5. What the room does with a game at its menu

- The hook tells the agent `MenuUp { menu }` once per arrival at the menu
  and room session. The agent marks the player ready in the lobby, unless
  the player is the room's owner, or the agent keeps no worlds. The owner
  still needs a world up: the room plays the owner's world, which only the
  owner's GUI can save for the room (`app.saveGame`). The menu leaves a
  `Load` without a file and a `Save` to the step's detour, which takes
  them once a world is up, as before.
- The menu's frame also keeps the session's heartbeat going at the menu;
  before, an agent in the lobby with a game at its menu saw no beat.

## 6. Seen in the game (2026-09-30), and still UNKNOWN

- **CONFIRMED (in-game):** a guest at the main menu loaded the room's
  world by itself, with no Start Game click: `DoStep` runs at the menu,
  `app.loadGame` from outside a page goes through, and the world starts
  without `setWaitForStartReadyGame`.

Still open:

- What a load that fails after it started does (mods missing, "Could not
  initialize game."): the game falls back to its menu; the hook waits
  `LOAD_PATIENCE` (600 s) and then holds. It does not retry.
- Whether the owner's own manual load, started while the room's `Load`
  with a file is pending, could be taken for the room's world. The hook
  never starts a load while the progress monitor has a task, and only a
  world that starts after its own load started counts.

## 7. In-game check list

Two games on one PC (tools/sandboxie/second_player.ps1), the deployed
server, both started from the launcher. The guest stays at the main menu
throughout. Expected `hook.log` lines, in order, for the guest (`<...>`
varies):

1. At start, after the build tools' line:
   `the main menu can load the room's world: detours on RegisterAppUsertypes and UI::CMenuUI::DoStep`
   (not `the main menu cannot load the room's world (fail closed): ...`).
2. While the menu builds: at least one
   `menu: Lua state 0x<...> has app; the main menu can load the room's world from it`.
3. In the room's lobby, on the menu's next frame:
   `the game is at its main menu (arrival 1): told the agent, which marks a guest ready`.
   The launcher shows the guest ready; the agent log
   `the game is at its main menu and loads the room's world when the game starts: ready`.
   The owner's game, at its menu, is **not** marked ready.
4. The owner loads a save (marked ready: `world 1 is up ...`) and presses
   Start game. Guest:
   `the room began a game while this game is at its main menu: rules native, 5 steps a second, checkpoints every 50`,
   then, once the save is fetched,
   `loading the room's world from <...>save-<n>.sav from the game's main menu to run step <n> next`
   and
   `the main menu is loading the room's world (tpf3mp_room_<pid>); the game starts it by itself`.
5. The menu switches to its loading page; no Start Game button; the world
   starts by itself. Then `mod: the GUI is linked` and
   `playing the room's world from its save, from step <n>`.
6. The room plays: the determinism probe's samples match as in the
   2026-09-29 run.

Failure lines to look for: `the main menu could not load the room's world: <why>`
(then `holding the world (fail closed): the room's world could not be loaded: <why>`),
and `holding the world (fail closed): the room's world did not load within 600 s`.

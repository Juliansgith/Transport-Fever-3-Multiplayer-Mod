# The Multiplayer entry on the main menu

D17 moves the room into the game (the owner lifted its hold on
2026-09-30): connecting, rooms, the lobby and chat in a Multiplayer window
reached from the main menu. This page says how the entry gets onto
Transport Fever 3's main menu at all, which took three tries on release
day, what the mod and the hook each contribute, and how the window talks
to the launcher that started the game. Players' steps are in
[PLAYING.md](PLAYING.md), "The Multiplayer menu in the game".

## What the game allows, and what it does not

TF3 draws its main menu from Teal scripts, `gui/menu/main_menu.tl` and
`gui/menu/main_page.tl`, loaded through the game's Lua loader. Three ways
in were tried against build 40408:

| route | result |
|---|---|
| A mod's own copy of `gui/menu/main_page.tl`, hoping the mod filesystem overlays the game's files at the menu | Not applied: mods are merged into the filesystem at startup but the game's `::/` files win until a game is loaded (as TPF2 applied mods per save). The menu has no mod extension point either. |
| The game's `--script <uri>` switch, pointing at a copy of `main_menu.tl` | It is a plain startup script runner (`Lua_Core`, before any menu exists): the file ran, but `_react.builtin` was empty and `react.lua` failed. A dead end for a menu. |
| **The hook, at the game's Lua loader** | Works. Below. |

## How it works

The game's `base/init.lua` resolves every `ug_require` and then calls
`resolveutil.loadfile(resolved)`, a Lua function whose body is a C++ lambda
(`framework/lua/Loader.cpp`, `lua::MakeState::<lambda_8>`). The hook, loaded
into the suspended game before any of its code runs (D11), detours that body
(`crates/tpf3mp-hook/src/menu_entry.rs`):

1. The detour's thunk saves the argument registers, reads the Lua state out
   of the lambda's closure (`**(closure + 0x10)`, the layout the body's own
   `lua_pcallk` call shows), and tail-jumps into the original body with the
   stack untouched. The loader runs exactly as before.
2. The first time a Lua state is seen, the hook runs a short Lua chunk in it
   through `lua_load` and `lua_pcallk`. The chunk wraps
   `resolveutil.loadfile`: a request for the game's `gui/menu/main_page.tl`
   is answered with the mod's `tpf3mp_1::/gui/menu/main_page.tl`; every
   other request passes through. The original resolved path stays the
   module's cache key, so the rest of the menu sees the same `MainPage`
   value it always did.
3. The mod's `main_page.tl` is the game's file with marked `TPF3-MP:`
   additions: a Multiplayer card in the top row, a Multiplayer button in the
   top bar, and a `Tpf3mpLobbyWindow` opened through the menu's own window
   container (as the Deluxe Edition window is).

The game log shows each step: `[tpf3mp] main menu: resolveutil.loadfile is
wrapped`, `... ::/gui/menu/main_page.tl is served from
tpf3mp_1::/gui/menu/main_page.tl`, `... TPF3-MP main_page.tl is in effect`.
The hook's `hook.log` shows the profile match, `main-menu Multiplayer entry
armed`, and `menu patch installed in Lua state ...`.

A game Steam started has no hook and keeps the plain menu.

## The build profile

The entry's targets are in the release's built-in profile for the build
(`profiles/tf3_build40408_steam_windows.toml`, `docs/HOOKS.md`), next to the
step gate's: `lua_loadfile` is detoured, the others only called. The first
three are optional there: without them the menu stays the game's and the
step gate still installs. `tpf3mp-hookcore/tests/tf3_static_proof.rs` pins
their addresses in the installed game (`TPF3MP_TF3_EXE`).

| target | what | how to find it again |
|---|---|---|
| `lua_loadfile` (0x2fa1d50) | the `resolveutil.loadfile` body | `Loader.cpp`: the function with the strings `Could no load file`, `base/tl.lua`, `Error while pcalling` |
| `lua_load` (0x2fbdf70) | Lua 5.2 `lua_load` | the only caller of `luaD_protectedparser` (the function that references the `attempt to load a %s chunk` check); `luaZ_init`, a `"?"` default chunk name, then the `_ENV` upvalue fix-up. Not the nearby `lua_dump`, which checks for a Lua closure on the stack top and returns 1 |
| `lua_pcallk` (0x2fbe0c0) | Lua 5.2 `lua_pcallk` | called right before `Error while pcalling` in the loader body; reads `L->top`, `L->stack`, `L->nny`, calls `luaD_pcall` |
| `lua_settop`, `lua_pushlstring`, `lua_tolstring` | Lua 5.2 | already the step gate's (the Lua link, `docs/HOOKS.md`) |

On a patch: `tpfre index` the new exe, find them again with `tpfre q`
(`str`, `callers`, `dis`), regenerate the signatures with `sig --toml`, and
put them in the new build's profile. The hook tries the menu's targets in
every profile that matches the build, the data folder's first.

## Talking to the launcher

The window asks the hook for the lobby through the request channel above
(`tpf3mp/state.lua`, a few times a second) and sends its actions the same
way (`tpf3mp/act.lua`). The hook does not answer on its own: an action is
queued for the launcher that started the game and handed to its agent over
the link (`ToAgent::Lobby`), and the state is the launcher's lobby as the
agent last sent it (`ToHook::Lobby`, bridge version 10). Every request also
reads the link, since at the main menu no step of the game does
(`crates/tpf3mp-hook/src/lobby.rs`; `docs/HOOKS.md`, "The main menu's
Multiplayer window"). The launcher carries the actions out as if its own
window had asked; Connect goes to its own server (D12). A game whose hook
has no link to its launcher shows so in the window and sends nothing.

## The mod's copies

`mod/tpf3mp_1/content/gui/menu/main_page.tl` is a copy of the game's file.
Every change is marked `TPF3-MP:`; the relative requires and asset paths are
made absolute (`::/...`), because a leading-slash path is resolved against
the requiring file's root, which for the mod's copy is `tpf3mp_1::/`. On a
game patch, take the new game file and re-apply the marked blocks. The copy
is listed in `_content.json` like any other file of the mod.

## Trying it

Start the launcher, then **Start Transport Fever 3** (before or in a
room), and click **Multiplayer** on the game's main menu. The mod must be
installed in the game's staging area and active.

`tools/lobby/launch-tf3-dev.bat` and `tpf3mp-launch` start the game with the
hook but without a launcher: the entry and window appear, and the window
says the game has no link to the launcher. They are for checking the entry
alone.

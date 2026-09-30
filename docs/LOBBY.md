# The Multiplayer entry on the main menu

D17 moves the room into the game: connecting, rooms, the lobby and chat in
an in-game panel, reached from the main menu. This page says how the entry
gets onto Transport Fever 3's main menu at all, which took three tries on
release day, and what the mod and the hook each contribute. The lobby
window's content (connect, rooms, chat) is the next step and lands in the
same window.

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
(`crates/tpf3mp-hook/src/menu.rs`):

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

Four targets, resolved by signature from `profiles/*.toml` in the per-user
data directory (`docs/HOOKS.md`); the first is detoured, the other three are
only called:

| target | what | how to find it again |
|---|---|---|
| `lua_loadfile` | the `resolveutil.loadfile` body | `Loader.cpp`: the function with the strings `Could no load file`, `base/tl.lua`, `Error while pcalling` |
| `lua_load` | Lua 5.2 `lua_load` | the only caller of `luaD_protectedparser` (the function that references the `attempt to load a %s chunk` check); `luaZ_init`, a `"?"` default chunk name, then the `_ENV` upvalue fix-up. Not the nearby `lua_dump`, which checks for a Lua closure on the stack top and returns 1 |
| `lua_pcallk` | Lua 5.2 `lua_pcallk` | called right before `Error while pcalling` in the loader body; reads `L->top`, `L->stack`, `L->nny`, calls `luaD_pcall` |
| `lua_settop` | Lua 5.2 `lua_settop` | the 72-byte function reached through the two-instruction `lua_pop` wrappers in `state.cpp`; fills with nil up to `ci->func + 1 + idx` |

`crates/tpf3mp-hook/profiles/tf3-40408-de1daad3.toml` is build 40408's.
On a patch: `tpfre index` the new exe, find the four again with `tpfre q`
(`str`, `callers`, `dis`), regenerate the signatures with `sig --toml`, and
add a profile file for the new build; the hook picks the one whose `[build]`
matches.

## The mod's copies

`mod/tpf3mp_1/content/gui/menu/main_page.tl` is a copy of the game's file.
Every change is marked `TPF3-MP:`; the relative requires and asset paths are
made absolute (`::/...`), because a leading-slash path is resolved against
the requiring file's root, which for the mod's copy is `tpf3mp_1::/`. On a
game patch, take the new game file and re-apply the marked blocks. The copy
is listed in `_content.json` like any other file of the mod.

## Trying it

```bat
cargo build --release -p tpf3mp-hook -p tpf3mp-launch
copy crates\tpf3mp-hook\profiles\tf3-40408-de1daad3.toml %LOCALAPPDATA%\TPF3-MP\profiles\
target\release\tpf3mp-launch.exe --exe "C:\...\TransportFever3.exe" ^
    --hook target\release\tpf3mp_hook.dll --env TPF3MP_GAME_LINK=dev
```

`tpf3mp-launch` starts the game the way the launcher does (suspended, the
hook loaded, then resumed) and prints its pid. The mod must be installed in
the game's staging area and active; `TPF3MP_GAME_LINK` only needs to be set,
the hook logs that no agent is present and carries on.

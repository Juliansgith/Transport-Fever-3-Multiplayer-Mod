# TPF3-MP v1.5.1

Version 1.5.1 restores multiplayer for **Transport Fever 3 Windows Steam build
40444** (Steam build ID 26643808, the update of 2026-11-20). The v1.5.0 hook
recognizes build 40425 and refuses the updated executable, so the game comes
up without Multiplayer. Players still on 40425 need v1.5.0 until they update
the game.

## Changes since v1.5.0

- The hook selects a new 40444 native profile, pinned to the executable's
  SHA-256, size and PE timestamp. 40444 changed the game's code: 159 of the
  profile's 170 targets still match their signatures at new addresses, and
  the other eleven are found again. Every native constant was re-read from
  the 40444 executable (investigation/UPDATE_40444_2026-11-21.md).
- **The main menu's load of the room's world** keeps working: the function
  that gives the menu's Lua state its `app` gained two arguments in 40444,
  one of them on the stack, and the hook now passes it through to the game.
- **The street and track tools continue from the segment the room built**
  on 40444 too: the game recompiled the click's callback without changing
  what it does, and the hook follows its new layout.
- **The station-platform order fix** reads the vehicles' node count from
  its new place in the game's frame.
- **The line manager window**: 40444 changed the game's window (line
  problem and capacity filters). The mod's copy, which keeps the stations
  clicked while a new line is being created, is rebuilt from 40444's window,
  and is served on 40444 only.
- The bundled Big Maps plugin code is pinned to 40444, and its New Game page
  copy takes 40444's change to the game's page (mods that are not installed
  no longer count as active). **Big Maps cannot be installed for 40444**
  until a 40444 package is published; TPF3-MP does not install a 40425
  package into a 40444 game.

## Verification and operation

`tpfre verify-build` against the complete private Steam build 26643808
archive must pass on the exact release commit; its 170 static targets match
without review required. A static proof (`tf3_build40444_profile`) and Big
Maps' game test pass against the 40444 executable. Static verification does
not prove every runtime path: the real-game acceptance on 40444 is still to
do. CI and acceptance must pass on the exact promoted commit.

The protocol stays **26**, the Lua bridge **40**, the action schema **29**
and the room-log format **11**. This client-only build update needs no relay
restart or room-log migration. Restart the launcher to update, or download
the new **TPF3-MP.exe**.

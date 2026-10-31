# TPF3-MP v1.4.0

Version 1.4.0 is a performance and stability release for **Transport Fever 3
Windows Steam build 40425**. Saves and industry connects freeze every
player for less time, vehicles move smoothly up close, a joiner catches up
faster, a player can leave the pause menu open, and four security gaps are
closed. It carries the first AI players' code, **turned off**: a server
seats none unless its operator turns them on.

## Changes since v1.3.4

**Freezes and smoothness**

- **Room saves freeze every player for 3.7 to 5.1 s instead of 6.1 to
  7.1 s.** The faster save compressor is now on by default
  (`TPF3MP_HOOK_SAVE_FAST=0` turns it off). Its approach comes from
  silver2127's TPF2 Big Maps work, which this repository's earlier port
  carried ([#193](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/193)).
- **Connecting an industry freezes every player for about 0.4 s instead of
  0.9 to 1.1 s.** The GUI's per-frame player check costs 0.15 µs a call
  instead of 203 µs. A player's command no longer waits for the server's
  next 100 ms tick ([#193](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/193)).
- **Vehicles no longer micro-stutter up close.** In a room's game the hook
  paces frames evenly and gives the game a matching clock. Only what is
  shown changes, not the simulation ([#194](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/194)).

**Joining and the network**

- **A joiner catches up faster.** A game that has run ahead of the room's
  pace for 5 s runs up to 64 updates a step instead of 16. At 4x a joiner
  replays 29.6 steps a second instead of 25.4
  ([#196](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/196)).
- **The agent no longer waits up to 15.6 ms each turn on Windows.** It asks
  for a 1 ms timer while it plays a room. Snapshot transfers go after a
  member's progress, actions and turns
  ([#196](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/196)).
- **A room keeps its speed after a rejoin.** Loading a snapshot saved at 4x
  into a 2x room no longer asks for the host's speed
  ([#192](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/192)).

**Stability**

- **Industries spawn alike in every game.** The game's buffers counted the
  same demand change twice, so peers could spawn different industries.
  Commands already ordered are staged after a completed batch, keeping
  replay, step boundaries and save and load order
  ([#192](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/192)).
- **The pause menu no longer ends a player's session.** A minute in the
  game's pause menu ended it with "the hook stopped responding". A room save
  asked just before the menu opened left a joiner "Saving..." after the host
  quit ([#212](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/212)).

**Security**

- **An unreadable action is refused.** Before, one undecodable action froze
  every game in its room, and every later joiner's too.
- **Checkpoint and save reports are limited** to 4,000 a second, so one
  connection cannot fill a room's command queue.
- **Rate limits refill correctly** for senders faster than one message a
  millisecond.
- **The admin endpoint refuses browsers**: requests with an `Origin`, or a
  `Host` that is not an address or `localhost`. `tpf3mp-ctl`, `curl` and
  the log scripts work as before
  ([#196](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/196)).

**AI players, off**

- This release carries the AI players' code
  ([docs/AI_PLAYERS.md](AI_PLAYERS.md), D32), but they are not yet good
  enough to play against. A server seats none unless started with
  `--ai-players` (`TPF3MP_AI_PLAYERS=true`), and the Multiplayer windows
  offer **Add an AI player** only on such a server.

## Verification and operation

The protocol is now **25** (from 23), the Lua bridge **39** (from 29), and
the room-log format **11** (from 10; logs of format 10 are still read). The
action schema stays **28**. Players and the server must update together: a
1.3.4 launcher cannot join a 1.4.0 server, nor a 1.4.0 launcher a 1.3.4 one.
The hook and the mod are installed together, as always. Operators upgrade as
OPERATIONS.md, "Upgrades", says; running games are restored from their
logs. CI and acceptance must pass on the exact promoted commit, and the
complete private Steam build 26129055 archive is verified against it by
`tpfre verify-build` before packaging.

Julian Cooper (_Sep, @Juliansgith) authored
[#192](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/192),
[#193](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/193),
[#194](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/194),
[#196](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/196)
and [#212](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/212).
Claude Opus 5.5 is recorded as co-author on #193, #194, #196 and #212. The
faster-save approach is silver2127's, from their TPF2 Big Maps work.

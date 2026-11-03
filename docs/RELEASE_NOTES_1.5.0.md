# TPF3-MP v1.5.0

Version 1.5.0 is a fixes release for **Transport Fever 3 Windows Steam build
40425**. Locomotives make their sounds again in multiplayer games, the
launcher finds Mod Hub downloads wherever mod.io keeps them and tells a
player to update an outdated game, and shared mods' settings work in a
room. An old release's server can now drain beside a newer one. The AI
players' code is still **turned off**: a server seats none unless its
operator turns them on.

## Changes since v1.4.0

**Sound**

- **Locomotives are no longer silent.** A room started from a save made
  while paused left every player's game at its own paused speed. The trains
  ran, but the game plays vehicle sounds only while its own speed is above
  0. Every player's game now keeps its own speed at the room's; only what
  each game plays and shows follows it, and nothing is asked of the room. A
  host's speed click is a request alone: their game follows once the room
  accepts it
  ([#230](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/230)).

**Starting the game**

- **An outdated game is named, not started.** The launcher checks the game
  before starting it. One older than the build TPF3-MP runs in is refused
  with "update the game in Steam" (Steam in offline mode often leaves it
  behind); one newer than TPF3-MP knows asks for a TPF3-MP update. Such a
  game used to come up with its plain menu, no Multiplayer button and no
  word why
  ([#229](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/229)).

**Mods**

- **Mod Hub downloads are found wherever mod.io keeps them.** Another game
  on mod.io (such as Sins of a Solar Empire 2) can move mod.io's folder; the
  launcher now follows mod.io's own settings, a user's `user.json` first,
  then `globalsettings.json`, instead of reporting a room's mods as missing
  ([#189](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/189)).
- **A room's mods are declared in the versions installed now.** Mods found
  again are declared even when the owner's pick was refused, and a room
  made from a start save still starts after a Mod Hub update
  ([#188](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/188)).
- **Shared mods' settings work in a room.** A listed event of a shared
  mod reaches its game script in every game, in the room's order (D34):
  Realistic Environment Pro's climate, mist and clock rules, and Real Cant
  Engineering's cant toggle and manual cant. Events not on the list are
  refused as before
  ([#190](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/190)).

**Servers**

- **An old release's server can drain beside a newer one** (D33). Drained,
  it opens no rooms and takes no new players, tells their launchers to
  update, and hands each game nobody has played for three minutes to the
  newer server, where its players rejoin with their invite. This works
  from this release's server on: OPERATIONS.md, "Two versions at once"
  ([#228](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/228)).

**AI players, off**

- The AI's passenger lines stand among a town's houses, reached by a walk
  along its streets, joined to them by a street along the station's front,
  with their depot beside the line, and buy a train sized and priced for
  the line
  ([#224](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/224),
  [#226](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/226)).
  They are still not good enough to play against: a server seats none
  unless started with `--ai-players` (`TPF3MP_AI_PLAYERS=true`).

## Verification and operation

The protocol is now **26** (from 25), the Lua bridge **40** (from 39), and
the action schema **29** (from 28). The room-log format stays **11**.
Players and the server must update together: a 1.4.0 launcher cannot join
a 1.5.0 server, nor a 1.5.0 launcher a 1.4.0 one. The hook and the mod are
installed together, as always. 1.4.0's server cannot drain, so operators
upgrade it in place as OPERATIONS.md, "Upgrades", says (running games are
restored from their logs), after `docker volume create tpf3mp-handover`,
which the compose file now names. CI and acceptance must pass on the exact
promoted commit, and the complete private Steam build 26129055 archive is
verified against it by `tpfre verify-build` before packaging.

Max (tearded) authored
[#188](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/188)
and [#189](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/189),
with Claude Opus 5.5 recorded as co-author. Silver (silver2127) opened
[#190](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/190)
and proposed D34, which the owner approved; its commits record silver2127,
Julian Cooper and Claude Opus 5.5. Julian Cooper (_Sep, @Juliansgith)
authored
[#224](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/224),
[#226](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/226),
[#228](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/228),
[#229](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/229)
and [#230](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/230),
with Claude Opus 5.5 recorded as co-author. The silent locomotives were
reported by players of log session 9Q8NAY and a second player.

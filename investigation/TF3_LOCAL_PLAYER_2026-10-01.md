# The engine's player, build 40408 (2026-10-01)

Why a company's own stops and stations act as another player's in its
player's game, and whether the hook can make the game's tools and GUI act
as the player's company while the simulation keeps the save's player.
Static findings with `tools/tpfre` on the installed executable
(sha256 `de1daad3…f23ef2`, the profile's build), unless marked otherwise.

## Seen in hook.log (2026-10-01, three players, competitive)

- **seen** Every proposal the game's own tools made in james's game, and in
  cat's, carries `playerEntity=118368`: the save's player, the room's first
  company, although each played for a company of their own (8 of 8 in
  james's log). The room rebuilds each with the acting company as owner
  (`apply.lua`, `company()`), so the stop is the company's and the tool's
  player is not its owner.
- **seen** `CMenuUI::m_game` is at `+0x6b0` (the hook's own reading of
  `DoStep`'s test, logged as `CMenuUI::m_game (+0x6b0)`).

## Where `api.engine.util.getPlayer` gets its answer

- **seen** The string `"getPlayer"` (rva `0x3760360`) is used once, in
  `sub_2536fc0` ("SetupUtilInterface"), at `0x2539fe4`. Its function is a
  copy (`sub_e27e80`) of a `std::function` the function takes as its second
  argument, handed to the registration `sub_24791a0`, which nothing else
  calls.
- **seen** That argument comes, through `sub_f301b0` ("SetupEngineInterface")
  and `sub_1087b40`, from each Lua state's setup: a
  `std::function<GameState const &()>`. There are three:
  - the engine's (game scripts'): `CGame::CGame`'s lambda_3/lambda_1,
    `sub_11ffd0`: `[[CGame+0x1f0] + 0x78 + 8 * i]`, with `i` the
    word at `+0x98`, or `1 - i` when the lambda's flag is set. Two
    `GameState` buffers.
  - the GUI's: `UI::CMenuUI::SwitchToGameUI`'s lambda_1/lambda_2,
    `sub_6aa800`: `[[CMenuUI+0x6b0] + 0x1e0]`, that is
    `[m_game + 0x1e0]`.
  - the React GUI's: `UI::react::ScriptComponentRoot::ReloadInterfaces`'s
    lambda_6, `sub_27c80a0`: a getter's object, then `+0x1e0`.
- **inferred** `getPlayer` answers a field of the `GameState` its state's
  getter gives. The engine's scripts read one of `CGame+0x1f0`'s two
  buffers, the GUI's read `CGame+0x1e0`.

## Not established

- Where in `GameState` the player entity is (the `getPlayer` closure that
  reads it is inside luabridge's template code; not found yet).
- Whether `CGame+0x1e0` is a `GameState` of its own or one of the two
  buffers at `CGame+0x1f0`, and whether the buffers swap or are copied
  between steps. If the GUI's is one of the two buffers, a value the hook
  wrote there for the GUI would become the simulation's at the next swap:
  the games would diverge.
- What the native street, track and construction tools and the line
  manager's picker (`newContext.player = true`, `line_util.tl`) read as
  their player: the same `GameState` field, or another copy (`DataLogger`
  keeps one of its own, `m_playerEntity` at `+0x24`, `sub_a7790`).

So it is not shown that the tools and the GUI can act as the player's
company while the simulation keeps the save's player, and the hook writes
nothing for it. What would show it, read only, in a real game: log
`CGame+0x1e0`, `[CGame+0x1f0]+0x78`, `+0x80` and `+0x98` at each
`CGame::Step` for a few frames (do they move, does `+0x1e0` equal either
buffer), then locate the player field by the value 118368 in each.

## Why the street tool will not split a road the room built (2026-10-02)

Reported in a competitive room: the street tool snaps into the middle of
the save's roads, but only to the ends of roads built during the session,
the player's own company's included; some of those roads cannot be
bulldozed either, and hook.log has no bulldoze for them. Static findings
with `tools/tpfre` on build 40408:

- **seen** The room's replay makes each road edge it builds the acting
  company's (`apply.lua`, `networkInto`: `link.owned` gives the edge a
  `PlayerOwned` of `company()`, e.g. 372363), where the native tool, in
  single player, makes it the save's player's. The engine adds that
  `PlayerOwned` as given (`street_util::AddToEngine` 0x25f9480, `0x25f957f`);
  the edge's `BaseEdgeStreet` is always added for a street (`0x25f9514`),
  so it is no other component that differs.
- **seen** `sub_610ea0` (`game\ui\actions\street_builder_util.cpp`), called
  here `IsOwnedByOtherPlayer(engine, player, entity)`: false for a
  negative player; looks the entity's `PlayerOwned` up and answers
  `owner != player`; false for an entity no player owns (a town's road).
- **seen** The street builder's snap,
  `CreateFindSnapPointRoadEarlyAbortContext` (0x60c360) through `0x5fb370`:
  for each candidate edge it reads whether each end node belongs to a
  construction (`sub_b4db80`, the flags at `[rsp+0xa4]`, `[rsp+0xa5]`), then
  calls `sub_610ea0` (0x5fc022) and, when it answers true, sets both flags:
  the edge is snapped to as a construction's is, at its ends only.
- **seen** The player it passes is the street builder's own,
  `UI::StreetBuilder+0xc0`, stored by its constructor (0x56a740, `mov
  [rsi+0xc0], eax` at 0x56a816) from an argument, and handed down by
  `StreetBuilder::Step` (`mov eax, [r14+0xc0]`, 0x586b4e) to `sub_571820`
  and `sub_25ef740`. The tools' proposals carry the save's player
  (`playerEntity=118368`, seen above), so that is the player it holds.
- **seen** The street bulldozer filters the same way:
  `UI::StreetBulldozerAction::vf2` (0x5f2e30) and its check `sub_5f2a00`
  call `sub_5f7db0(engine, players, entity, flag)`, true when the
  players list is empty or the entity's `PlayerOwned` owner is in it,
  else the edge is not offered ("Protected - Cannot Be Bulldozed" is the
  other branch). The module and street connector bulldozers call it too.
- **inferred** So for a player of any company but the room's first, every
  edge the room built for a company is another player's to the native
  tools: no split, no bulldoze, and (the earlier reports) no snapping onto
  the company's own stations and no tram track onto its rail. Ends still
  connect: a node has no `PlayerOwned`, and an edge marked fixed is still
  snapped to at its nodes. The save's roads are the save's player's or a
  town's (no owner), which the test lets through.

Not changed yet: the fix needs a decision. The tools could act as the
player's company (write the company into each tool's own player, GUI
objects only: the street builder's `+0xc0`, the bulldozer's players list),
which the room then checks as it does now (`companies.mayTouch` refuses
another company's edges in every game); or the room could leave road and
track edges the save's player's, which moves every company's road upkeep
and ownership to the room's first company. `sub_610ea0` itself is no place
to change the answer: `construction_builder_util` (`MakeProposalRemove`,
`CreateProposalReplace`, `MakeStreetProposal`) reaches it too, and the
simulation calls those.

To see it in one try: `TPF3MP_PROBE_PLAYER=1` logs each entity the test
takes for another player's, with the tool's player and whose the entity
is (docs/HOOKS.md, "The probe of the engine's player").

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

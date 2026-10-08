# Subsidies: local two-game validation

Validated on `feat/subsidy-acceptance` (dev at 8223446), Transport Fever 3
build 40408 (Steam, Windows), 2026-10-07. One run,
`target/game-runs/run-1007-234810`: the rig's local server and two
agent-started games on one PC, the host in the save's company and the guest
in a company it founded in the room. The `subsidies` gate was switched on
in the installed copy of the mod only (docs/GAME_TESTING.md, "Investigating
without the room"), so both games ran the same mod. Ordinary GUI actions
changed the world; read-only probes typed into each game's console only
read it. No production relay and no game-install file was touched. Both
games were quit through `app.quit(false)` from their consoles.

## Acceptance outcome

- Accepting and declining a subsidy went to the room as `Subsidy` `Accept`
  and `Decline` and were applied in both games, from the host's game and
  from the guest's.
- A subsidy the guest's company took was its alone: the money up front went
  to that company, the host company's bus line between the same two towns
  did not complete it, and when it ran out its penalty was taken from that
  company, in both games alike. The host company's own subsidy on the same
  line completed in both games at the same game time.
- Both games loaded the room's save with the same five offers, drew the
  same new offers afterwards and listed the same subsidies, with the same
  takers, at every checkpoint. The room's rolling world checks matched at
  all 182 windows of the run. Nothing was refused, nothing failed to apply,
  and nothing diverged.
- The `subsidies` acceptance gate is now enabled.
- Not exercised here: two companies accepting one offer in the same step,
  the reward of a completed subsidy moving to a company other than the
  first (a passenger subsidy rewards town growth, not money), the cargo and
  workers kinds' progress, a taken subsidy across a save and reload, a taker
  dissolved before its subsidy ends, and other platforms.

## The world

`tpf3mp_fixture_sub2`, made from `tpf3mp_fixture_sub` (`tools/game/fixture.ps1
-Tiles 24`: 24 by 24 tiles, seed `tpf3mp`, 1990, eight towns) in a single
player game with the mod in its mod list:

- the company ("Max Transport", entity 8751) took an $83,000,000 loan, and
  a player built a road depot, a bus stop in Ossett (8755) and one in
  Horsforth (8759), already linked by a country road, and Line 1 between
  the two stops;
- the subsidy script's own console events then added the offers, before
  any bus was bought (the passenger kind offers no pair of towns that a
  line with vehicles already links):
  `subventionSpawn` with `deliver_passengers` for Ossett and Horsforth
  twice (uids 900001 and 900002) and for Ossett and Newcastle-under-Lyme
  (8753) once (900003), each offered for 36 months, and `_debugSpawn`,
  which drew a `deliver_cargo` offer (29270000). The game had drawn one
  passenger offer of its own (17506);
- two buses were bought for Line 1, and the world saved paused.

So Line 1, the first company's, carries passengers between the two towns
of two offers.

## Actions and what each game did

| # | Game | Action, as a player does it | Handed to the room | Applied |
| --- | --- | --- | --- | --- |
| 1 | guest | Multiplayer window: found "Rival" | `CompanyOp` `Create` | both games: Rival #1, entity 15586 |
| 2 | guest | Subsidy card Ossett–Horsforth, $3,330,000 (900001): Accept | `Subsidy` `Accept` | both games: "subsidy 900001 ... taken by Rival" |
| 3 | host | Subsidy card Ossett–Horsforth, $2,690,000 (900002): Accept | `Subsidy` `Accept` | both games: "subsidy 900002 ... taken by Company" |
| 4 | guest | Looks for 900002 again | nothing: the offer was gone from its list | - |
| 5 | host | Subsidy card Ossett–Newcastle, $2,150,000 (900003): Decline | `Subsidy` `Decline` | both games: 900003 gone, the script's spawn modifier 0.5 |

Every `hook.log` line `the game applied the room's actions between
simulation updates` has a counterpart in the other game: 27 in each by the
end. Neither log has a `not applied`, a refusal or a divergence, and the
rig reported none. The other actions handed during the run were
`NotificationSeen` (a notification's sound), from both games.

## The subsidies, at each checkpoint

The mod's game script says the subsidy script's state in `hook.log` at a
checkpoint whenever it changed (`subsidies at game time ...`, then one
`subsidy:` line each). Both games' lines were the same throughout:

| Game time | Both games said |
| --- | --- |
| 237000 | five offers: 17506, 900001, 900002, 900003, 29270000, as saved |
| 407000 | 900001 `taken`, `taker=15586` (Rival), `delivered=-`; 900002 `completed=398000`, `taker=8751`; 900003 gone; modifier 0.5 |
| 1277000 | a new offer, 17507, drawn at 1273000 in both |
| 1927000 | 900001 `failed`, `taker=15586`; then "subsidy 900001 failed for Rival: -3330000" |
| 2007000 | another new offer, drawn at 2003600 in both |

900002 completed 54,800 game-time units after it was taken: Line 1's
passengers between Ossett and Horsforth. The same passengers on the same
line never counted for 900001, which Rival took: the mod's wrapper hands
each kind only the events of lines its taker owns. That the run script's
`addModifier("loadGameRes", ...)` reaches the subsidy resources, inferred
from the binary until now (docs/HOOKS.md, "Subsidies"), is shown by this:
without it the base kind counts anyone's line, and 900001 would have
completed with 900002.

## Money

A console probe read both companies' `ACCOUNT` in each game:

| Game | Game time | Company (8751) | Rival (15586) |
| --- | --- | --- | --- |
| guest | 712000 | 82,617,930 | 3,330,000 |
| host | 890600 | 82,280,599 | 3,330,000 |

Rival, founded with nothing and with no costs, held exactly 900001's money
up front in both games: the script booked it to the first company and the
room moved it on. The first company's balance differs between the rows by
its running costs between the two game times; the room's rolling checks,
which include the money lanes, matched at every window. After 900001
failed, the guest's game bar showed Rival at $0 with earnings of
-$3,330,000.

## The worlds stayed equal

The determinism probe mod was not in the fixture, so `probes.ps1` had no
samples. The mod's own rolling world checks served instead: each
`hook.log` holds a signature for every window of 50 steps, and the two
games' signatures were compared window by window: 182 windows, all equal,
none missing on either side.

## Seen in the game

The base game's subsidy cards and list know no companies: every player
sees every taken subsidy as accepted, including another company's. Which
company took it is in the room's record and the log, not on the card.

## Comparison limits and checks

- One PC, one Windows build, one run.
- The two money probes of a pair were taken at different game times, so
  only the room's checks compare the same step for the first company.
- The offers were made with the subsidy script's console events in single
  player, before the room; in the room only the GUI's Accept and Decline
  were used. The offers the game drew itself during the run (17507 and the
  one at 2003600) agreed in both games as well.

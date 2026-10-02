# Selected ports from PR #37

This page records the integration into `dev`, not all work on Silver's
branch. It is not evidence of a fresh two-player game acceptance run.

## Enabled ports

| Area | Integrated behavior | Evidence |
|---|---|---|
| Identity and builds | Steam default name, binary build identification, incompatible launcher-instance protection | Rust unit tests |
| Content | Fingerprint the installed multiplayer mod; exclude generated portraits so cosmetic extraction does not split rooms | Content/fingerprint tests |
| Lobby | Loading stages, banners/portraits, copy invite, change starting save before play | Lua window and server tests |
| Companies | Read the game's balance, order loans, check HQ ownership and permits | Lua capture/replay fixtures |
| Vehicles | Buy onto a line, including bursts; use isolated harbour/airport depots and the selected second depot | Lua capture/replay fixtures |
| Roads | Refuse street demolition if its affected town buildings changed; preserve junction settings and street precedence | Lua capture/replay fixtures |
| Command guard | Install the guard in the HUD's separate Lua state | Lua guard fixtures |

Our installer, updater, executable icon, desktop shortcut and separate server
package remain. Our Join a friend form, stock new-world setup, leave/rejoin
cleanup and native loading implementation remain. Depot connection replay
retains our queue-based pruning of internal construction branches.

## Follow-up ports from the updated PR

The follow-up includes HUD command-result routing (147d590), default vehicle
compartment loads (4f30083), and the native load-state fix (24cb7c8, adapted
with 1878b573's incoming-world handling). Callback forwarding adds bounded
admission, acknowledged batches and retry without discarding accepted actions.
Our close-before-load frame and all existing acceptance gates remain.

The new native field is checked against the installed executable, read-only.
Lua and hook fixtures exercise callback overflow, default/partial loads,
load gating and save setup. This is not a fresh two-player game playthrough.

## Implemented, refused pending game acceptance

Subsidies, entity renaming/vehicle recolouring and line waypoints have capture,
schema and replay code, but `content/scripts/tpf3mp/acceptance.lua` disables
them. Both command submission and replay refuse these channels; subsidy
settlement is disabled too. Mechanics fixtures explicitly enable a channel
only in their own Lua state. Enable a channel only after ordinary two-player
acceptance demonstrates matching outcomes, ownership and money. The gate
file is part of the installed-mod fingerprint.

## Deliberately excluded

- Automatic creation of competitive companies and changes to station access.
- Additional native terraforming and track-upgrade hooks.
- Alternate simulation-buffer and world-loading experiments.
- The expanded scenario runner and notification/discard additions.

The existing junction gate remains off pending its own game acceptance.
No owner decision in PLAN.md or DECISIONS.md is changed by this integration.

## Compatibility

This selected combination is distinct from both the previous `dev` and PR #37:
protocol **15**, bridge **22**, action schema **22**. Update launcher, hook,
mod and relay together before release. Older peers must fail version checks;
this branch is not compatible with the currently deployed relay until upgraded.

## Local extras (`local/combined-dev`, not on dev)

The local combined line is dev plus what #38 left out, for local testing
only; it speaks action schema 23 (dev's 22 plus the actions below), with
dev's protocol 15 and bridge 21.

- Competitive rooms found a company for each player automatically
  (`tpf3mp.script.lua`, `foundOwnCompany`).
- Station access per company (`CompanyOp::StationAccess`, after dev's
  company operations) in the in-game window.
- The notification log and a warehouse's discard (`Notification`,
  `DiscardCargo`, actions 21 and 22); not gated by
  `tpf3mp/acceptance.lua`.
- Trees and other assets bulldozed through the room (`Bulldoze::Assets`),
  only in games started with `TPF3MP_TREE_BULLDOZE=1`.
- Terraforming and the track upgrade tools read natively (the hook's terrain
  call), the street detail tools named in hook.log.
- The HUD state's answers routed through notes (`hudguard.forward`), so the
  vehicle store's buy onto a line works (147d590), in place of dev's
  refusal of a window that waits on what it made.
- The finance window's own loans and offers for a founded company
  (`tpf3mp/follow.lua`, bb47cee).
- The scenario runner and observe (`tools/scenarios`, the hook's scenario
  mode), the desync logging (`TPF3MP_HOOK_WATCH_ENTITIES`, platform and
  MovePath lines), `TPF3MP_HOOK_ALTERNATE_BUFFERS` (opt-in), the engine
  player probe (`TPF3MP_PROBE_PLAYER`), the command-kind counts.
- The in-game window keeps the invite code's Copy, Leave room, the co-op or
  competitive mark and a nearly opaque page beside dev's rows.

Subsidies, rename/recolour and waypoints stay refused by dev's
`tpf3mp/acceptance.lua` here too; the scenarios that script them
(`money.json`, `road_vehicles.json`) see those steps refused.

# Station access and terrain: local two-game validation

Validated on `codex/station-access-terraform`, Transport Fever 3 build 40408,
2026-10-02. All runs used the rig's local server and two agent-started games.
Ordinary GUI actions changed the world; temporary staged probes only read it.
No production relay or game-install files were changed. All test games were
quit through their native menus and the temporary diagnostic probe removed.

## Acceptance outcome

- Company heads can choose an open/closed station default and grant, deny or
  reset another company's override. Own stations remain usable. Existing
  services continue; new and changed routes are checked.
- Native line selection, foreign-station service, passenger carriage, fares
  and save/reload were demonstrated in two games. Access grants do not grant
  construction editing rights.
- Raise, lower, smooth, flatten and heightmap strokes produced identical
  native terrain. The Terraform acceptance gate is now enabled.
- Terrain paint and asset brushes remain refused. Native strokes exceeding
  the 4096-cell replay band, external heightmap imports, other platforms and
  other transport modes were not exercised here. Replay-band coverage is
  automated fixture coverage, not a native acceptance claim.

## Terrain evidence

Run `target/game-runs/station-terrain-1002c` used five ordinary strokes.
A read-only probe hashed native base and surface heights at 66,049 points
(4 m grid, x=-1024..0, y=-512..512, 17-digit coordinates/heights).

| Stroke | Native grid / changed cells | First compared step | Base-height hash |
| --- | --- | --- | --- |
| Baseline | — | 7500 | 1994739112-1053425635 |
| Host raise | 15×15 / 68 | 8500 | 1771126487-1885598018 |
| Host lower | 15×15 / 68 | 8900 | 0452977479-1577597949 |
| Host smooth | 15×15 / 225 | 9100 | 1857138900-0040372271 |
| Host flatten, maximum GUI size | 45×45 / 638 | 9700 | 0419096908-0052807249 |
| Guest raise | 15×15 / 68 | 10400 | 1697598827-1976033505 |

Both terrain hashes matched at all 49 common samples, steps 7500..12600.
Final surface hash: 0169141013-1146803458. The changes survived the save
`tpf3mp_terrain_verified_oct02` and reload in D; the base hash still matched
in H before its next stroke. Surface changes from subsequent road building
are expected and matched between games.

In H, the host selected the native heightmap brush and clicked at timestamp
1790935644. The hook captured a 15×15 grid, 66 changed cells; both games
applied it at 1790935645 (48 cells changed in each carrier). At step 22400,
both base hashes were 0171899096-1113093317 and both surface hashes were
0934041238-0860862383, changed from the pre-stroke hashes at 22200.

## Station access evidence

Run G: `target/game-runs/station-terrain-1002g`. P1 owns Access Test A
(company 1, native player 7601); P2 plays for the shared company 0 (3869).
The full bus station is group 8349 (station 8329); original open station
6964 belongs to company 0.

| GUI action | Capture timestamp | Observed result |
| --- | --- | --- |
| Owner selects own closed station | 1790933616 | CreateLine applied by both games |
| Guest selects closed foreign station | 1790933688 | No stop/line action |
| Owner grants company 0 access | 1790933749 | CompanyOp applied by both |
| Guest selects foreign station, then own station | 1790933814 / 1790933828 | CreateLine and EditLine applied by both |
| Owner resets override to closed default | 1790933860 | Existing route retained; later guest new-line selection refused |
| Owner opens default | 1790934187 | Guest creates another line at 1790934206 |
| Owner sets explicit deny, then resets | 1790934229 / 1790934295 | Policy changes replayed; native deny selection attempt hit existing line geometry, so not counted as picker proof |
| Guest tries joining a road to foreign station | 1790934418 | Both refuse: junction edge belongs to Access Test A |
| Owner joins station entrance to public road | 1790934559 | Both apply successfully; route becomes reachable |

P2 bought a bus through the selected line's native Buy Vehicles flow and
added a roadside stop through the line manager. The bus visited all three
stops. Save `tpf3mp_shared_service_oct02` (6,084,856 bytes) was loaded into
both games in H; the route and station policy survived.

In H, vehicle 5738 belonged to player 3869 on line 8519. Native `noPath`
was false and `visitedStops` contained 0,1,2. At steps 21400..21600 the bus
carried one passenger; at 21700 it unloaded and the native owner's income
increased from 0 to 797. Both games reported exactly the same position,
load, visited stops and income at each sampled step. The full station is
stop 0: the passenger journey reached it before the next sampled movement.

## Bugs found and fixed

- Native PLAYER_OWNED components are userdata. Treating them as tables lost
  ownership information. Regression tests now use userdata and check that
  station access never grants editing of foreign assets.
- The native station selector can classify an actual station as a network
  edge. The HUD converter now recovers ordinary station details only for
  permitted station entities, preserving real edges and terminal details.
- HUD and game-script GUI states have separate module instances. Install
  the station predicate in both; load the line converter only in the HUD.
  Loading it in the other state produced missing React recipe errors.
  H rendered a normal road preview without that error after the fix.
- The standalone probe previously skipped userdata vehicle positions,
  compared different GUI companies' balances, and used unsupported generic
  edge iteration. It now reads native vectors, all live company accounts,
  and the street-system node map. Missing vehicle positions fail closed.
  These changes have Lua regression coverage.
- The room script now explicitly builds the hook library as well as the
  rig binary; `--bin tpf3mp-rig` alone did not rebuild the hook DLL.

## Comparison limits and checks

H's strict `tools/game/probes.ps1` comparison passed 13 complete samples,
steps 20700..21900, with zero differing lanes. This window includes the
passenger journey and fare. Across H there were 19 common world, terrain
and service samples through 22600, all identical. P1 missed sample 22300
around the terrain interaction; the strict full-run checker correctly
refused that gap. Samples 22700 onward are P2 alone after the host quit.
The full run is **not** claimed as a strict probe pass. The rig's final
"games did not report their lanes" result after native quitting is likewise
not a passing rig-exit check; native hook/probe logs are the evidence here.

C's old money-lane mismatch was invalid measurement of different companies,
not a passing full-world comparison. D and G also have missing-sample gaps.
F's uninterrupted first segment passed five complete samples, 13900..14300.
E and F's later load attempt timed out at Start Game and are excluded.

Station policy fixtures cover explicit allow/deny/reset, own stations,
unauthorised policy changes, shared-company behavior, capture and replay.
Native acceptance demonstrates the bus workflow above; it does not claim
all station/vehicle combinations or a long-duration desync soak.

# Road terminal desync investigation, 1–2 October 2026

**Current result:** unordered industry lane and implicit-terminal maps are
a confirmed source of different native edge identities. The fix sorts those
inputs before construction and covers both fresh and cached Lua loads.
Silver's bus failure now passes through 1,550 steps from the pre-failure
save, with all 133 vehicles identical. The cargo fixture also passed both
formerly failing farm returns through 18,950 steps. The mixed road/rail
regression matches all 76 vehicles through 4,950 steps, with all four trains
serving every scheduled stop and recovering from blocked signals.
Detailed evidence and unsuccessful candidates
appear below. This is not a claim that every possible desync is eliminated.

Silver reported road vehicles choosing different alternative terminals after
roughly 3,450 simulation steps on a busy map. The report initially mentioned
buses, then cargo vehicles visiting a livestock farm. Neither the original
save, logs nor build revision was available at the start. This investigation tests both
cases; it does not establish the cause of that separate run.

**Earlier baseline: road desync reproduced in the extended run.** Cargo vehicle
43 diverged on the crop-farm approach at room steps 9,800 and 18,700.
Both replicas sampled the same selected terminal. The cause was not known
at that stage; see the later industry investigation and confirmation below.

**First run: not reproduced.** All 400 consecutive shared vehicle checkpoints
from room step 7,750 through 27,700 agree exactly. That covers 19,950 active
bus steps, 11,600 active truck steps and 3,700 steps after the first
livestock-farm dwell. No gameplay code was changed.

## Environment and method

- Windows x64, Steam game build 40408. Executable SHA-256:
  `de1daad3a13f3b7e9f79903361bb43769cf4f15e59271a263aefe1f075f23ef2`.
- Repository revision `2bab376451a517cc4f4c5df97050b1f6d2d1cabd`.
  Release hook SHA-256:
  `fb0d206e17b5a907cf4617006af4d17a8f8d9f093f07c05e23d8e3f88578b479`.
- Two real game processes on one PC, rig's local relay, same loaded room
  snapshot. No production server was involved.
- Fresh map generated through New Game: large temperate map, year 2020,
  16 towns and 16 initial industries. The network was then constructed
  through ordered multiplayer actions, with the built-in sandbox enabled.
- `TPF3MP_HOOK_MEASURE_ORDER=50`, `TPF3MP_HOOK_LANE_DUMP=3`.
  Compare every shared 50-step vehicle checkpoint with
  `python tools/lane_diff.py --lane 3 <p1/hook.log> <p2/hook.log>`.
  Compare complete entries, including raw movement position/speed and entity
  IDs, not just the rounded lane summary hash.
- Existing paused-tick, vehicle-order, platform-visit, terminal-candidate
  and road-entry ordering hooks were installed and active.

Local evidence is under `runtime/bus-terminals-20261001-c/` (ignored by
Git): both `pN/hook.log` files, console scripts, screenshots and comparison
outputs. Save `tpf3mp_bus_terminal_network_20261001` preserves the bus
network in the game's local save folder. `verified-results.json` records
the final counts; `verify-final.py` additionally asserts continuous shared
checkpoints, identical entries, all 32 trucks visiting the livestock farm
and at least three there simultaneously.

Both owned game processes were quit normally after pausing the room; the
local rig also exited. Its final lane-summary status was `Unknown` ("the
games did not report their lanes"). The comparison above is established
by the paired native hook dumps, not by that rig summary.

## Bus case

Four stations with four passenger terminals each, connected by a road
loop; three overlapping four-stop routes, 40 articulated Citaro buses.
Each stop permits terminals 0, 1, 2 and 3, with a 15-second dwell. Read-only
samples of `TransportVehicle.arrivalStationTerminal` observed all four
choices, and the native candidate-selection hook recorded calls.

Moving vehicles are confirmed at room step 7,750. The bus-only stage was
checked through 13,200 (5,450 active-fleet steps), and buses continued during
the cargo case through 27,700 (19,950 active-fleet steps total). Every
shared vehicle checkpoint agrees. No bus desync reproduced.

The loop is a terminal-contention fixture outside the towns. It does not
demonstrate passenger demand or loaded passenger service.

## Cargo case

32 Volvo FH12 bulk trucks, crop-farm group 22 to livestock-farm
group 18. The crop farm has two terminals and the livestock farm three;
all alternatives are enabled. The fixture road joins the ordinary map's
road network. Truck movement is confirmed at room step 16,100, and the
first crop-farm dwell at 19,550. Console samples observe both crop-farm
terminals and 230 units of cargo loaded across the fleet. The native road
route detours through the map; straight-line farm distance understates
travel time. The fleet ran at the game's maximum 3x speed.

The first livestock-farm dwell is at step 24,000. A console sample observed
trucks dwelling at terminals 0 and 2 and another targeting terminal 1.
By step 25,850, all 32 trucks have been observed dwelling at the livestock
farm, with a peak of three simultaneously. The test finished at 27,700,
3,700 steps after the first farm dwell; travel alone was not counted as
terminal-contention coverage. All shared vehicle entries agree. This
checks loaded vehicle movement and terminal selection, not a complete
downstream meat/wool production chain.
Save `tpf3mp_bus_cargo_terminals_20261001` preserves the combined fleet.
`tpf3mp_bus_cargo_terminals_postrun_20261001` preserves its later state.

## Setup findings and limits

The scripted fixture initially connected to a station's inner frozen node
instead of its external road end. After correcting the geometry, its
saved dead-end junction configuration still excluded the new approach.
Rebuilding the approach with the normal BuildRoad junction-reset payload
made the buses depart. The connection to the existing map needed the same
reset of its saved turn configuration. These were incomplete fixture
proposals, not evidence of the reported desync; no gameplay fix was made.

One earlier setup process crashed after retaining a native construction
userdata across console commands. The restarted run uses copied values;
that setup crash was not a replica divergence.

Vehicle checkpoints do not include the selected terminal field itself.
Their equality shows matching sampled movement, states, stops and lines;
terminal use is additionally checked by read-only console samples. This
is not an every-frame proof of terminal equality or an all-world proof.
The separate network probe reports `e=err`, so it cannot establish network
equality. A same-PC, same-binary result cannot rule out a different-build,
different-PC or save-specific failure in Silver's run.


## Extended road and train run

Evidence: `runtime/train-road-stress-20261001/`, starting from the first
run's `tpf3mp_bus_cargo_terminals_postrun_20261001` save, with the same
40 buses and 32 bulk trucks. Same binary, hook, PC and local relay.
Vehicle dumps now also record `arrivalStationTerminal` (station/terminal
indices) and `arrivalStationTerminalLocked`. These diagnostic fields do
not change the lane digest, gameplay or protocol. The focused Lua suite
passes all 126 tests, including a regression proving that terminal
changes appear in dumps without changing the digest.

### Reproduced cargo divergence

Both failures concern canonical vehicle 43 (native entity 83727), line 6,
stop 0: the **crop farm**, not the livestock farm. At steps 9,750 and
18,650 its full sampled state agrees, with no locked arrival terminal.
At the next checkpoint both replicas have locked station/terminal `0/0`,
but their path-relative movement differs:

| Step | Replica | Path edge index | Position | Speed | State |
|---|---|---:|---:|---:|---|
| 9,800 | p1 | 197 | 0.2360935211 | 4.2099032402 | en route |
| 9,800 | p2 | 196 | 19.7399139404 | 4.1840071678 | en route |
| 9,850, before rebase | p1 | 199 | 5.6909475327 | 4.0174422264 | en route |
| 9,850, before rebase | p2 | 1 | 0.0390033722 | 0 | dwelling |
| 18,700 | p1 | 214 | 20.5710239410 | 4.1840071678 | en route |
| 18,700 | p2 | 215 | 1.0723826885 | 4.2099032402 | en route |
| 18,750, before rebase | p1 | 1 | 0.0367040634 | 0 | dwelling |
| 18,750, before rebase | p2 | 217 | 6.4915776253 | 4.0174422264 | en route |

`tickCount` and `updateCount` agree in both replicas at these checkpoints:
47,250 at room step 9,800 and 56,150 at 18,700. The server detects lane 3
mismatches and rebases the room; the raw logs preserve both pre-rebase and
post-rebase entries. `first-divergence-evidence.json` extracts them. The
standard `lane_diff.py` deliberately keeps the last occurrence of a step,
which hides the original mismatches at 9,850 and 18,750 after replay. Do
not interpret those last-occurrence matches as an uninterrupted clean run.

The first divergence predates any moving train. At both first mismatching
samples the chosen terminal agrees. This establishes a native road-vehicle
divergence near terminal approach, **not** proof that different terminal
selection caused it. Edge indices are relative to each vehicle's path;
they are not world edge IDs and cannot alone identify the physical route.
A transient choice inside the 50-step sampling interval remains possible.
The installed platform visit/candidate and road-entry ordering hooks did
not prevent this failure; road-entry metrics reported no refused entries.
The separate Engine::Update order-measure hook could not install over an
existing detour, so this run cannot prove full native claim-order equality.

The initial save and raw logs are retained. An attempted recovery of an
intermediate pre-failure snapshot was too late: its manifest had already
rotated out. Do not claim that intermediate snapshot was preserved.

### Train fixture and acceptance

Three passenger stations, 1.4 km apart, with three connected platforms
each; shared single track, 12 station exit signals and a depot exit
signal. The initial fleet was 12 diesel Schienenbus railcars. Two active
routes share South and Central; the longer also
visits North. Alternatives 1 and 2 are enabled alongside primary terminal
0. The fourth constructed platform has no connecting fan and is excluded.
A third unused line remains empty because the depot's approach cannot
reach its initial stop without reversing. All 12 vehicles were initially
assigned to the two reachable routes instead.

Initial scripted depot track failed geometry validation. Building the
connection with the normal in-game track tool succeeded and allowed
trains to depart. A prior depot proposal omitted the required seed and
asserted identically in both games, after the first road mismatch; it was
corrected in the fixture script. Neither rejected setup action is counted
as moving-train coverage. The save `tpf3mp_road_rail_stress_20261001`
preserves the connected fixture.

The initial fleet overloaded the South/Central shuttle. By step 30,250,
three trains waited at each station for a free platform at the other,
with more queued at the depot. Both replicas agreed on the blockage.
This is not useful evidence of continued circulation. Eight railcars were
sold through the room, leaving two on the three-station route (73, 81)
and two on the shuttle (77, 78). The four-vehicle phase starts at checkpoint
30,750, and movement resumes as platforms become available.

The run ends paused at step **35,550**. From first train motion at 24,400
there are 224 shared checkpoints spanning 11,150 steps, including the
initial congested phase. The four-train circulation phase covers **4,800
steps and 97 shared checkpoints**. All four railcars visit every stop on
their route, with repeated visits and observed terminals 0, 1 and 2.

| Vehicle | Route | Distinct sampled dwell episodes after 30,750 | Waiting-to-moving transitions |
|---|---|---:|---:|
| 73 | SouthÃ¢â‚¬â€œCentralÃ¢â‚¬â€œNorth | 4 | 4 |
| 77 | SouthÃ¢â‚¬â€œCentral | 6 | 4 |
| 78 | SouthÃ¢â‚¬â€œCentral | 4 | 4 |
| 81 | SouthÃ¢â‚¬â€œCentralÃ¢â‚¬â€œNorth | 5 | 3 |

These are sampled transitions, not an exact every-frame arrival count.
`train-acceptance.json` preserves each observed dwell's step, stop and
terminal. The final fixture is `tpf3mp_road_rail_postrun_20261001`.
Both test games quit normally, and the owned local rig exited. After the
paused shutdown the rig ended with `the games did not report their lanes`;
this is not reported as a green automated rig completion. The evidence
above is the matched native checkpoints collected before shutdown.

### Final comparison and limits

`final-comparison.json` compares every occurrence of each vehicle at a
checkpoint, preserving repeated steps across rebases. There are **711
shared unique checkpoints**, no unequal occurrence counts, no differing
bus entries and no differing train entries. The four differing cargo
entries are vehicle 43 at 9,800, 9,850, 18,700 and 18,750, before each
rebase: two divergence episodes, not four independent reproductions.

No bus or train desync was observed in this fixture. This does not prove
stability across machines or builds, nor explain the cargo failure. The
rail fixture exercises routing, alternative platforms, opposing trains,
queues and repeated arrivals; it does not validate loaded passenger
service, freight trains, electrification or long multi-car consists.
The earlier overloaded fleet is reported separately rather than counted
as successful continued circulation. The existing network probe's error
also prevents claiming equality of all native world state.

Only diagnostic logging and its regression coverage changed in product
code. All 126 focused Lua tests pass; `git diff --check` passes. No gameplay
fix, protocol change, commit, promotion or release was made. Future root
cause work should compare full physical paths and reservation/blocked
state around the first differing update, including the interval before
the sampled terminal becomes locked, then validate against both this
fixture and Silver's original save.

A compact evidence archive, `terminal-stress-evidence.zip`, contains both
vehicle histories (including duplicate checkpoints), the local relay log,
comparison results, analysis scripts and this report. Complete hook logs
remain in the run directory.


### Silver's supplied save

During the extended run the owner provided
`C:/Users/Sepgi/Downloads/twomptest-desync-save.zip`. It contains only
`twomptest.sav` (274,094,536 bytes) and its thumbnail. Copies are extracted
under `runtime/silver-desync-20261001/`; the original archive is unchanged.
Archive SHA-256: `1aae281a5e859821c98cd2391cd5746b577d14d211ed436743e661d7d7f03753`.
Save SHA-256: `d6d62c5d84d493e6f76ab0002f49f5a49a0f09e8d945f01b4a9ce32e60af4c5f`.
It was subsequently loaded in the two Silver runs below. Those runs
reproduce a bus failure; a shared root cause with our cargo failure is
still unproven.

### Silver baseline, 1 October, current investigation

`runtime/silver-path-baseline-20261001` loaded the supplied save in two
local replicas (133 vehicles, 22 lines). The first diagnostic difference
is at step 3,200: vehicles 22 (native 217708) and 28 (220479), both on
line 19 (356825), receive different-length paths. Vehicle 22 has 10 versus
26 edges; vehicle 28 has 25 versus 17, with different end/decision offsets.
Movement first differs at 3,300 and the room rebases. This reproduces a
failure on Silver's save, but does not yet establish its root cause.

A read-only query identifies the line as `New-Oil-WRK`, between `Green Lane`
(group 314963) and `Newport Oil Well` (362212). Both stops specify station
0, terminal 0, with empty alternative-terminal lists. Its vehicle modes
include buses. No claim that this is a cargo-only failure is justified.

The initial physical-edge logger incorrectly assumed the tuple layout
from the API definitions; the real API exposes `edgeId` and `dir`. Thus
its edge counts/offsets and movement fields are valid, but the pre-rebase
path hashes merely hash repeated nil fields. The logger was corrected in
the staged mod and loaded by the automatic rebase. The comparison preserves
every checkpoint occurrence, including pre-rebase mismatches.

The run reached step 5,150 and both games quit normally. Player-action
handoffs appear in the logs; their payloads were not logged, and automatic
notification acknowledgments can also use that channel. The owner was
asked whether they interacted, so this is not described as a proven
untouched soak. No gameplay fix has been made yet. Follow-up instrumentation
reads the affected line's native cached routes to separate cache differences
from path selection differences.

### Native line-cache trace

`runtime/silver-cache-auto-20261001` reproduced the same failure with the
corrected physical-edge logger. Both games started with identical line
356825 cache paths: 49 edges (`eacc126055c49b4d`) and 55 edges
(`a16e4a61f2be1067`). At step 3199 their new caches diverged:

| Replica | Section 0 | Section 1 |
| --- | --- | --- |
| P1 | 44 / `35b937fbf74dfc82` | 58 / `2ab4327275682335` |
| P2 | 49 / `78ce4fd152b14b29` | 55 / `02eca3521155f70d` |

The trace hashes entity, edge index and direction only, excluding native
padding. Old and new cache values are both read at step 3199; it observes
every GetData call for this line, including temporary engine contexts.
At checkpoint 3200 the actual simulation vehicles 22 and 28 acquire
different paths, and movement diverges at 3300. The same terminal numbers
do not imply the same physical route. The games quit normally after the
failure; the rig's final no-lanes message is not a passing acceptance result.

A paused save made while the replicas still matched near step 2800 is
`tpf3mp_silver_near_20261001`. The original zip is unchanged. This run used
automatic loading with a 120-second stagger. Its paused tickCount differs
from the first baseline by 339, but updateCount at step 50 is 92383 in both
runs and vehicle 22 has the exact same position and speed. Comparisons use
the shared room snapshot, not an assumed relationship between paused ticks
and simulation updates.

### Industry lane identities

The discarded priority-sort experiment (`silver-priority-candidate-b-20261001`)
reached step 4150 without movement, choice or path-length differences, but
raw edge hashes differed from step 400. It is not accepted as a fix. Its
host also exited with access violation 3221225477 during shutdown. The
experimental native sorter has been removed from source and the profile.

Read-only geometry queries explain the raw hash difference: entity 362201
is the oil well's industry subconstruction, not a road junction. Its same
17 physical edges have different indices in the two games. P1 edge 5 is P2
edge 0, with identical endpoints and length 11.199522972106934. Native edge
identity alone therefore cannot establish a physical route difference.

The game's `base/content/industries/industryutil.lua` builds internal lanes
using `pairs(generatedData.lanes.curves)` and discovers default terminals
using `pairs(generatedData)`. Both are string-keyed maps whose Lua hash
iteration varies across states. Rebuilding an industry can consequently
change the meaning of lane and terminal indices differently in each game.

The candidate in `industry_order.lua` wraps the base utility's factory and
gives only those two input maps a sorted `__pairs` iterator. Explicit
terminal arrays and completed engine results retain their order. No game
installation file is modified. A console prototype confirmed that the
game's Lua honors `__pairs`; unit tests model that protocol because the
workspace's test interpreter is Lua 5.1. Loader tests cover the three base
path forms, module arguments, idempotence and unrelated module passthrough.

Native confirmation of this candidate is pending. The first run was
`silver-industry-order-20261001`, from the short pre-failure fixture, using
hook SHA-256 `bdc8bf01dbb057b1e3ca0c7fefc820968cc5f478a861bbe8499991ae74119f46`.
It still produced different internal edge IDs from step 400; no movement
or choice differences were observed through checkpoint 1000. This does not
establish a fix. The follow-up instrumented loader proved that several
construction workers load industryutil as their first file (`patched=false`).
A Lua wrapper installed inside that in-flight load misses its return value.

The next candidate intercepts that exact base resource at the native loader,
returning a deferred Lua chunk. On execution it loads the original utility
under a per-state recursion guard, clears the guard even on loader errors,
then wraps its factory. This covers first-file loads as well as cached-worker
loads. The raw utility and every unrelated resource still use the game's
own loader. Tests cover first-load behavior and cleanup after both a thrown
error and a missing file. `silver-industry-order-c-20261001` exercises this
candidate with SHA-256 `00ba1528c814055424dee4307abc7da56466be0203e1b2ef547db01a6496db35`.
This candidate still failed: route counts diverged at update 387, vehicle
choice differed at checkpoint 400, and the server reported divergence at
500. Run d deferred the original factory until each update, to avoid
separately transferred copies of captured input. That run was stopped
after loading to address a more direct bypass found by disassembly.

`resolveutil.loadfile`'s outer function at 0x2fa4b80 calls the shared
bytecode cache at 0x2fa8130 before reaching the previously hooked body.
The raw helper load populates that cache with its original bytecode, so
later workers can bypass the wrapper entirely. The revised hook intercepts
the cache lookup before its lock, only for the empty (base) namespace and
`industries/industryutil.lua`. Its raw-load guard passes the recursive
original load through the unchanged cache. The function ABI and URI fields
were checked in disassembly; a unique profile signature and installed-game
static proof cover the new target.

Run e (`silver-industry-order-e-20261001`, hook SHA-256
`222bbcb31d97b93806b329f23956015740ae43f6c7990713bcab7441b649e576`)
is the first run to produce identical rebuilt routes on both replicas:
49 edges / `ad3a52ad94a267c7`, and 55 / `08259bea9f72d239`, with decision
indices 40 and 54. The cache change is observed at updates 387â€“389;
the simulation checkpoints through 500 match all 133 vehicles, including
raw edge IDs, path lengths, terminal decisions and movement. The earlier
candidate diverged at checkpoint 400 and triggered a server verdict at 500.
This is a passing short reproduction, not yet cargo/train acceptance or a
claim that all possible desyncs are eliminated. The extended run matched
31 complete checkpoints (steps 50 through 1550), all 133 vehicles and all
logged fields, with no missing occurrences or server divergence/rebase.
Both games quit through the UI without a recorded abnormal exit. The
comparator conservatively excludes the newest possibly incomplete
checkpoint (1600 at the last pre-quit check). Subsequent cargo and train
acceptance results follow below.

### Cargo confirmation

`cargo-industry-order-20261001` uses the same hook as the passing Silver
run and loads `tpf3mp_bus_cargo_terminals_postrun_20261001`, the fixture
used by the earlier 40-bus/32-truck baseline. The host's shared snapshot
advanced 227 simulation updates before room start: checkpoint 50 has
updateCount 37727 versus the baseline's 37500. Compare native update
counters for the old failure windows (47250â€“47300 and 56150â€“56200), not
unadjusted room step numbers. The comparison preserves every occurrence
and checks missing records below the newest checkpoint.

The first formerly failing farm return passes. At room steps 9600 and 9650
truck 43 has identical approach position, speed, terminal-lock state and
207-edge path (`0272577937-0462321294`) in both games. It completes its
farm dwell at step 9800. All 72 vehicles match through step 10050, with no
missing records or automatic rebase. Its later circuit also passes: truck
43 completes its third farm dwell at step 18750. The run matched 379
complete checkpoints through 18950, all 72 vehicles, with zero differing
fields, missing occurrences, server divergence verdicts or automatic rebases.
All 72 vehicles moved and dwelled; 1370 dwell events were recorded within
the verified checkpoints. The run passed both earlier native-update windows
and the corresponding actual farm arrivals, not just a nominal step limit.
Both games quit through the UI without an abnormal exit in the rig logs.
The compact evidence archive is backed up outside the user's build cleanup
at `D:/TPF3-MP-test-evidence/terminal-desync-20261001/cargo-industry-order-pass.zip`.

The adjacent `constructionutil.lua` model and asset-group builders already
use ordered iteration. They were inspected but not changed; unordered
terrain-face enumeration has not been shown to cause these vehicle failures.

### Rail regression confirmation

`rail-industry-order-20261002` loads `tpf3mp_road_rail_postrun_20261001`, the
four-train version of the earlier three-station fixture. The hook was rebuilt
after a comment-only clarification in its embedded Lua; executable SHA-256
is `8295ce38a0f262535c41baf8a62fbf3cd06512ba90a671c9edc1fdd9116affa7`.
No behavior changed from the passing bus/cargo build. Acceptance requires
matching checkpoints and actual train movement, station visits and signal
recovery; matching stationary worlds alone would not suffice.

The final pre-quit comparison covers 99 complete checkpoints, steps 50
through 4950, with all 76 vehicles identical in every logged field and no
missing occurrences. Checkpoint 5000 is conservatively excluded. No server
divergence verdict or automatic rebase occurred. All 40 buses, 32 trucks
and four trains moved and dwelled within the verified window.

Trains 73 and 81 visited all three stops on line 7; trains 77 and 78 visited
both stops on line 8. Their recorded station dwells total 20. Locked
platform choices include 0/0, 0/1 and 0/2; trains 73 and 77 each used all
three. The four trains recovered from native blocked states 2, 2, 2 and 3
times respectively. Train 73's long signal wait ended and it reached its
remaining stop at step 3750; it was not counted as passing while stationary.
Both games quit through the UI. The rig recorded no abnormal exit.

The final hook release build, formatting check, hook Clippy check and
diff whitespace check passed. Focused tests passed: 216 hook unit tests
(five ignored), two industry-order regressions, 127 mod-Lua tests and two
installed-game static-proof tests. Before PR submission, full workspace
formatting, Clippy (`--all-targets -- -D warnings`) and `cargo test --workspace`
also passed on Windows. Release promotion remains a separate step.

Evidence is backed up at
`D:/TPF3-MP-test-evidence/terminal-desync-20261001/rail-industry-order-pass.zip`.
These results establish the reproduced industry-order fix on Windows Steam
build 40408. They do not prove every map, mod, platform or transport mode
is desync-free; loaded freight trains, ships and aircraft were not covered.

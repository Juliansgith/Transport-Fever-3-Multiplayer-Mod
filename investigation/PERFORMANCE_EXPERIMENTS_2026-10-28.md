# Follow-up performance experiments

## Experiment H: nominal-sized catch-up with capacity feedback

Branch `codex/experiment-bounded-catchup-20261028`, based on G. Add an
opt-in five-update cap. Full capped Far batches shown without compression
now preserve backlog feedback, including five updates at 4x and 2x. Ordinary
16-update default batches and action/pause/stale/checkpoint-tail timing
retain their previous rules. Ordered drains retain the normal cap of 16
after the live rejoin test below exposed a regression. This combines 250 ms batches, a five-update
maximum and G's measured floor; none becomes the default.

Outcome/checklist: reduce secondary delivery stalls and recovery velocity
bursts, retain room speed and action order, keep world probes equal. Compare
against the existing default (200 ms/cap 16/no floor), with both foreground
positions and the same ordinary Bubu start. This is a combined candidate,
not an isolated claim about one knob. Include long native calls in results.
Verify ordinary input visibly and in both logs, then pause/resume and rejoin
before accepting a candidate. Rendered-motion evidence is still necessary
for a smoothness claim. A small unconditional cap can increase action/rejoin
latency; the revised candidate excludes ordered drains from that cap.

The 240 s closed-loop model uses the actual cadence/trim/floor policy with
35 ms/update, 20 ms preparation and 1.3 s extra work every 1000 updates.
Capping alone leaves 101 queued updates and averages 19.583/s. With bounded
feedback it ends at four queued updates, 19.986/s, and zero secondary
service misses >50 ms (original expensive operations counted separately).
At 30 Hz presentation it ends at five queued updates, 19.984/s and zero
secondary misses. These fixed-cost feasibility results do not prove native
frame pacing or erase the original long stall. The model retains the
failing capped-only policy as a regression witness.

### H first live result and the rejoin regression it exposed

`paired-bounded-catchup-bubu`: local private room 2978AP, P1 44192 at
200 ms/cap 16/floor off, P2 30296 at 250 ms/cap 5/floor on, rig 47040.
Ordinary main-menu Bubu start, same saved camera, 4x, fast saving. No builds,
recordings or screenshots ran inside either measured window. All outliers
remain; the second window includes a periodic save.

| Window / foreground | Peer | Worker median / p95 / p99 / max ms | Active delivery late p95 / p99 / max ms | Active late >50 / >100 / >500 ms |
|---|---|---|---|---|
| 1001-4000 / P1 | P1 baseline | 118.8 / 150.1 / 480.9 / 1573.9 | 39.252 / 302.654 / 1429.748 | 27 / 14 / 3 |
| same | P2 candidate | 164.7 / 204.6 / 240.6 / 1762.2 | 44.052 / 124.086 / 1555.186 | 25 / 10 / 3 |
| 7001-10000 / P2 | P1 baseline | 137.8 / 184.6 / 347.4 / 1526.2 | 103.386 / 210.033 / 1414.232 | 127 / 39 / 3 |
| same | P2 candidate | 161.6 / 195.4 / 223.7 / 1495.8 | 24.389 / 56.352 / 1281.768 | 8 / 3 / 1 |

"Active" means the previous exposed buffer had positive updates; this keeps
long native calls but separates zero-update save/lifecycle intervals. Across
**all** delivery intervals, p99/max were 332.051/1429.748 versus
124.086/1555.186 ms in the first window, and 444.017/4550.565 versus
80.900/4366.220 ms in the second. All-interval >100 ms counts were 16/10
and 45/6. The save interruption remains a roughly 4.4-4.6 second gap.

Rates were 19.861/19.794 and 19.342/19.357 updates/s respectively. These
finite windows include the original stalls and save; they are not a claim
of exact sustained 20/s. Candidate queued work returned to two or three
updates after recovery. Five-update recovery calls were about 148-166 ms
around steps 1351-1381, rather than the baseline's long 16-update batches.
The first window's candidate queue peaked at 43 versus 35 baseline. This
trade-off, foreground/host effects and the lack of a reversed-policy run
prevent treating the percentages as an isolated causal estimate.

The guest renamed the shared company to `Bounded test` while paused, then
restored `Seppoon Transport` at 2x; input screenshots and both games' applied
logs confirm both actions. No construction-action acceptance is claimed.
After the speed change the learned budget reset to three-update samples.
All 294 rolling and 295 allocator/tick samples matched through step 14700.

Returning to the main menu, leaving and rejoining loaded the common save at
8618 into both games. This exposed an unacceptable drain slowdown: P2 kept
running five updates per 500 ms while a historical event waited, versus P1's
16-update drains. At one observation the host was at 10500 and guest 9450.
The first attempt was stopped normally before replay completed, not marked
passed. All 34 common rolling/allocator/tick samples in that new load matched
through 10300, with no missing periodic sample. The rig's final lane-summary
warning is not substituted for this explicit probe comparison.

The revised implementation restores the existing 16-update bound whenever
an ordered event, pending player command or pause needs draining; gate event
and checkpoint cuts remain mandatory. The new real-link regression first
failed with `Exactly(5)` instead of `Exactly(16)`. It now checks the 16/16/8
drain, no early event, and restoration of the smaller cap afterward. The
command-ack test retains its ordering and acknowledgement assertions while
checking the restored fast drain. 595 hook tests pass (seven ignored), with
focused all-target clippy and formatting clean. The first run's numbers
describe the pre-revision binary; they are not final-code acceptance.

No usable H vehicle-body trajectory was captured. This remains an opt-in experiment,
not evidence of zero freezes or an accepted rendered-smoothness default.

### H second check: ordered replay restored, residual 2x backlog caught

`bounded-drain-rejoin-bubu`, private local W94ABU, P1 39172/P2 36552,
rig 46532: ordinary original-Bubu menu start again. The revised binary
kept five-update Far batches after the ordinary 4x long call (about
148-152 ms around steps 1211-1221), then restored 16-update ordered history
drains on rejoin at 2x. P1 continued while P2 replayed the initial snapshot.
At steps 1101/1117 the guest ran 16 updates in about 479 ms. No automatic
speed request came from the guest; the host's only requests were 4x and 2x.

After the final ordered event, however, the guest stuck roughly 126 updates
behind: steps 8376/8381/8386 ran five updates per roughly 500 ms. At 2x the
nominal count is three and the presentation cap is five, so those five
updates are uncompressed. The first feedback predicate incorrectly covered
only exactly nominal batches. This live failure was reproduced in the model
with 200 initially queued updates: after 240 s, 212 remained, at 9.927/s.

The final predicate applies feedback to full capped Far batches whenever
`shown(count, nominal) == count`. Compressed emergency batches and short
checkpoint tails retain their rules. The same 2x model then ends with two
queued updates, 10.809/s including repayment of the initial 200. Initial
history also clears at 1x/3x/4x. The earlier 4x comparison results are
unchanged. Neither assertions nor the room's rate were relaxed.

The second run stopped normally after identifying that failure; completed
rejoin is not claimed for it. All 179 rolling and 180 allocator/tick common
samples matched through 8950, with no missing periodic sample (the initial
replay sample at 16 is listed separately). A passive 12.04 s rail-bridge capture contained
328 frames, with recorder overhead median 6.73/p95 9.82 ms. It began after
the selected train left, so it provides no valid moving-body acceptance.

### H final build: ordinary start, mixed timing result

`bounded-all-speeds-rejoin-bubu`, private local UCV5P9, P1 39612/P2 39528,
rig 45524. Hook SHA256
`3F030336196118C1162BFF3D15152BB83D32AAA89362FDB5CF6545F55986A1FD`.
Same ordinary Bubu start and policy assignment as H1. The host selected 4x
near step 1000. No builds, screenshots or recordings ran within the main
1501-3000 and 3501-5000 windows. P1 was foreground for the first, P2 for
the second. The supplemental 1001-3000 window retains the initial long
native operation just after the speed transition rather than hiding it.

| Window / foreground | Peer | Worker median / p95 / p99 / max ms | Active delivery late p95 / p99 / max ms | Active late >50 / >100 / >500 ms |
|---|---|---|---|---|
| 1001-3000 / P1, including transition | P1 baseline | 121.3 / 145.8 / 187.1 / 1553.7 | 32.681 / 75.220 / 1392.113 | 10 / 5 / 1 |
| same | P2 candidate | 165.9 / 203.0 / 252.5 / 1762.4 | 32.251 / 144.736 / 1548.990 | 8 / 4 / 1 |
| 1501-3000 / P1, steady segment | P1 baseline | 121.3 / 139.1 / 162.4 / 201.7 | 24.117 / 41.374 / 52.332 | 1 / 0 / 0 |
| same | P2 candidate | 166.1 / 198.3 / 218.2 / 252.5 | 24.787 / 40.388 / 50.340 | 1 / 0 / 0 |
| 3501-5000 / P2 | P1 baseline | 137.6 / 215.4 / 683.8 / 1510.0 | 126.703 / 551.438 / 1356.348 | 48 / 22 / 5 |
| same | P2 candidate | 156.5 / 233.3 / 922.5 / 1471.8 | 87.129 / 714.835 / 1249.130 | 19 / 13 / 4 |

The supplemental window's all-delivery p99 was 94.604 versus 144.736 ms;
its other maximum and the main windows' all-delivery p99/max equal the
active values above. No outliers are removed. Rates for the three windows
were 19.968/19.990, 19.979/20.042 and 19.736/19.809 updates/s respectively.
Maximum queued counts were 38/41, 8/11 and 39/39. Both foreground roles
were observed, but host/guest policies were not reversed across runs.

This final result is mixed: the candidate has fewer medium/large late
deliveries in the busy window, but worse p99 there and in the supplemental
window. The quiet window is nearly equal. Larger batches also have longer
normal worker calls. It is not an overall performance acceptance, and does
not justify changing the default or claiming that vehicle pauses are gone.

### H final rejoin and action acceptance

The host selected 2x; the guest returned to the main menu, left, and joined
the same invite through the normal UI. The host was not manually paused to
help catch-up. The guest loaded the original step-1 snapshot again at
1793223044 and ran the preserved 16-update ordered drains. After those,
its residual backlog was 439 at 1793223505. Unlike the failed second run,
five-update recovery shortened its interval from 475 ms towards 250 ms;
by 1793223547 the last Far batches had available counts 27, 24, 22 and 18.
Both peers reached step 11700 at 1793223563. The guest was back to ordinary
three-update/300 ms calls with backlog one and stayed there until the test's
host pause. This demonstrates that the reproduced 2x residual backlog clears.

This was **not a fast hotjoin**: clicking Join to reaching the frontier took
about nine minutes, replaying the original snapshot's history. A host save
was requested after the guest had already selected that older snapshot.
Snapshot freshness/replay latency remains a separate performance concern;
this experiment repairs throughput, not snapshot selection or instant joining.

The host paused, the guest renamed the shared company 0 from `Seppoon
Transport` to `Recovered transport`, then restored the original name. Both
applied logs confirm those actions at 1793223629 and 1793223672; screenshots
show the entered text and resulting UI. The host resumed 2x and both stayed
in play. The host's only speed requests were the four explicit inputs
400, 200, 0, 200; the guest sent none. No construction test is claimed.

`compare-last-load.py` separates actual loads rather than merging equal
snapshot-next-step identifiers: P1 loaded once and P2 twice. Comparing only
P2's replay against P1 gives 243 matching samples for each of rolling world,
free-ID queue and tick/update counters through 12150, with no missing periodic
sample. The older all-load comparator also passed. The rig's final legacy
"did not report their lanes" summary is not treated as a successful check;
the explicit recorded probe comparison is the evidence used here.

Diff review found one additional scope edge: at unusually high room step
rates, a default cap-16 Far batch can also be uncompressed. A new assertion
failed on that case. The final guard restricts the new feedback to caps below
16, preserving default behavior for every nominal count 1-16. The live DLL
above predates only this guard and its test; all observed nominal/cap pairs
(1/16, 2/16, 4/16 and 1/5, 3/5, 5/5) have identical decisions with the guard.
That extra high-step-rate case is covered by the pure test, not claimed as
another live-game run.

Final source checks: 595 hook library tests passed, seven ignored; focused
all-target hook/testkit clippy, rustfmt and a release hook build passed. The
final compiled DLL SHA256 is
`F856BE3896E73B33F6FD196FB187CA2BEF7444CD87E6075835A21C731AEE1AF8`.
The full workspace was subsequently checked for PR preparation on 2026-10-29,
at `e58f5adf`: `cargo fmt --all`, workspace all-target clippy with warnings
denied, and `cargo test --workspace` passed (2073 passed, 14 ignored, no
failures). Logs are retained in
`F:\TPF3MP-test-archives\pr-performance-20261029`. The subsequent documentation
commit only records these results and the observed save measurements.
All-platform release acceptance has not been run for this branch.

Both games quit normally and the local rig exited. The original staging
folder was restored and all 95 original file hashes and the file count
verified. Experimental staging was preserved under the evidence root as
`bounded-final-test-staging`. No production relay, new worktree or release
was involved. The runtime experiment remains opt-in and local.

Disk housekeeping: the three finished H run directories now live under
`F:\TPF3MP-test-archives\performance-followup-20261028`, with junctions at
their original D: paths. Every moved file was SHA256-verified (5243, 6475
and 9009 files respectively). D: free space rose from 0.73 to 5.00 GiB;
no evidence was discarded and no additional cache was put on C:.

## Experiment G: measure the minimum useful recovery interval

Branch `codex/experiment-measured-presentation-budget-20261028`, based on F.
The intended outcome is fewer secondary motion holds after a slow native
operation, without losing room speed, changing ordered actions, or masking
world mismatches. The original industry/save stall is still a failure of
continuous motion; this experiment does not claim to eliminate it.

`TPF3MP_HOOK_READY_BUDGET=1` learns preparation-plus-worker duration from
the actual exposed buffer, using the prior successful Sync and that buffer's
monotonic worker timestamps. It keeps 16 samples for each normal batch count,
uses p75 plus 16.667 ms after four samples, and floors the requested interval
at that budget, capped at the batch's duration at room pace. World, epoch and
pace changes clear history. Emergency batches and zero updates break the
measurement pair; emergency batches retain their existing compressed interval.
Default remains off. No world/protocol/ownership changes are made.

The first closed-loop model failed: applying this floor to emergency batches
increased secondary misses from 43 to 93, backlog from 4 to 15, and reduced
rate from 19.986 to 19.912 updates/s. Excluding emergency batches fixes that
specific feedback problem: 14 misses, backlog 4, 19.986 updates/s. This is a
240 s model with 35 ms/update plus 20 ms preparation, 60 Hz quantization and
1.3 s extra work at the same every-1000-step boundaries. Its secondary-miss
metric excludes each original expensive operation, not the recovery after it.
The real-game result must include those original stalls as well.

Acceptance: ordinary menu start on Bubu, equal 250 ms/max-16 policies with
only the floor differing, foreground swapped between windows, all outliers
retained, matching rolling/allocator/tick probes, and no loss of simulation
rate. An accepted candidate additionally needs actions, pause/resume, rejoin
and actual vehicle-motion evidence; worker-duration improvement is not enough.
Before the run: 593 hook tests passed (7 ignored), five rig tests passed,
focused all-target clippy and release hook/rig builds passed.

### G result: no sufficiently controlled smoothness win

`paired-ready-budget-bubu`: P1 32984 floor off, P2 42156 floor on, rig
34076, private local room KPZ5U7. Same ordinary Bubu start and saved camera,
250 ms/cap 16 on both, fast saves, normal recovery. No builds or video
recording ran inside the measured windows. The floor was active on P2 and
learned roughly 217-221 ms for five updates during the first window.

| Window / foreground | Peer / floor | Worker median / p95 / p99 / max ms | Delivery late p95 / p99 / max ms | Late >50 / >100 / >500 ms |
|---|---|---|---|---|
| steps 1001-4000 / P1 | P1 / off | 155.8 / 195.3 / 472.3 / 1625.3 | 30.841 / 268.071 / 1393.573 | 17 / 7 / 3 |
| same | P2 / on | 171.2 / 226.9 / 577.5 / 1750.6 | 43.709 / 383.511 / 1535.153 | 24 / 13 / 3 |
| steps 4501-7500 / P2 | P1 / off | 171.0 / 228.9 / 551.6 / 1731.0 | 56.837 / 420.505 / 1467.016 | 35 / 21 / 5 |
| same | P2 / on | 158.0 / 196.5 / 432.4 / 1442.1 | 29.503 / 270.287 / 1215.582 | 15 / 10 / 4 |

The foreground effect again dominates; comparing different windows as if
they contained identical work would be misleading. Rates were 19.893/19.842
and 19.841/19.842 updates/s. These finite windows contain the original slow
operations and their recovery, with no long-term backlog growth demonstrated.
The timing floor alone is not accepted as a default or as smoother motion.
No new body-video or rejoin acceptance is claimed for this unaccepted policy.

The host paused through Space and resumed at 2x; both logs confirm those
speed changes. Two scripted guest rename attempts produced no corresponding
handed/applied action in either log, so they do not pass action acceptance.
The test did not capture the input field after typing and cannot distinguish
missed UI input from a product fault. Earlier F did verify those actions;
that result is not substituted for G. The new pace rebuilt the floor with three-update
samples (~137-140 ms), rather than retaining the five-update budget.
All 177 rolling and 178 allocator/tick common samples match through 8850;
no periodic sample is missing. Both games quit normally and the rig exited.

The follow-on expensive calls include 16-update emergency batches taking
472-577 ms, or longer. A future smaller-cap experiment must also preserve
trim feedback during Far recovery: otherwise relaxing trim to 1 while
capping the batch at nominal can leave the backlog permanently uncleared.

## Experiment F: more preparation time between buffer swaps

Branch `codex/experiment-five-update-presentation-20261028`, based on the
explicit speed-intent fix. Adds the opt-in `TPF3MP_HOOK_BATCH_MS=250`; the
default remains 200. At 4x it groups five updates, reducing nominal native
buffer swaps from five to four per second. Native update duration, replay
acknowledgements, checkpoints and room speed stay unchanged. The cost is
about 50 ms more nominal batching latency than the default at 4x. At 2x the
integer grouping is three updates (300 ms); at 1x it remains one (200 ms).

Acceptance: paired ordinary-start Bubu games, identical camera and speed,
200 versus 250 ms, all world probes matching; compare actual recorded
vehicle motion as well as worker/delivery timing. Include actions and
pause/resume. Keep timing claims separate from camera/frame-rate claims.
Do not promote the setting based only on a synthetic model.

The initial capacity model uses 35 ms/update, 40 ms native preparation per
batch and 60 Hz frame boundaries. Crucially, work must finish within the
**previous** buffer's interval. Over 240 s, 200 ms misses that budget by
more than one frame in 8.41% of measured batches; 250 ms has none. Rates
remain 19.992 and 19.988 updates/s respectively. These are hypothetical
service costs, not a prediction of live smoothness or the long native
industry/save outliers. Focused hook tests: 587 passed, seven ignored;
rig tests: five passed. Real-game results follow when measured.

### F result: feasible, not an accepted smoothness improvement

`paired-five-update-bubu`, P1 19092 (200 ms), P2 50532 (250 ms), local rig
51048. Ordinary menu start, original Bubu save, 3840x2161 each, identical
saved cameras, cap 16 and normal recovery on both, fast saving on both.
P1 foreground first; P2 foreground second. No builds ran during measurement.

| Window / foreground | Peer / setting | Worker median / p95 / p99 / max ms | Delivery late p95 / p99 / max ms | Late >50 / >100 ms |
|---|---|---|---|---|
| steps 1001-4000 / P1 | P1 / 200 | 120.2 / 148.6 / 450.5 / 1655.2 | 31.482 / 329.998 / 1478.430 | 21 / 13 |
| same | P2 / 250 | 167.7 / 209.1 / 505.0 / 1891.1 | 68.963 / 320.571 / 1662.583 | 41 / 18 |
| steps 6501-9500 / P2 | P1 / 200 | 132.6 / 164.2 / 250.8 / 1526.2 | 51.419 / 113.605 / 1417.676 | 43 / 11 |
| same | P2 / 250 | 154.6 / 188.5 / 246.0 / 1491.3 | 31.639 / 89.630 / 1265.298 | 15 / 4 |

Each window is about 150 s; all outliers remain. Both sustain ~19.90 and
~19.98 updates/s respectively. Foreground/background effects dominate this
paired comparison; neither a CPU win nor a rendered-smoothness win follows.
The two windows have different world events and are not interchangeable.
The CPU sample around the second phase includes a separate 12 s recorder
window; do not report it as observer-free.

The P2 foreground recording `motion-250-p2.mp4` contains 343 GPU-timestamped
frames in 12.09 s. The unchanged recorder cost median 6.83 / p95 9.58 ms per
frame. It visibly captures a locomotive remaining still for about one second,
then moving and pausing again. Twelve continuously tracked body features over
the first four seconds show the same halt; its 6.29 px p95 feature spread
also includes changing orientation around the curve. This is evidence that
freezes remain, not a clean steady-speed A/B result. GPU timestamp versus
local QPC offsets were about -10 ms, so do not claim precise hook/video event
alignment from the recorder's UTC anchor. The hook separately reports a
1239.8 ms late delivery in this wall-clock period.

Concrete next target: after a long native call, the deep trim makes five
updates due in 125 ms even while preparation and work take ~175-215 ms.
Thus recovery creates repeated misses after the original native stall.
An experimental budget based on completed work can avoid demanding those
impossible deadlines. A render-history buffer remains unimplemented.

Pause via Space, guest rename while paused, host resume at 2x, and a second
guest rename restoring the company name all applied in both games. The normal
periodic save completed. All 271 rolling and 272 allocator/tick common samples
match through step 13550, with no missing sample or unexpected resync/UI error.
Both games quit normally. This setting is not accepted, and no rejoin trial
was repeated for F. It stays opt-in; defaults are unchanged.

Base: `9051dde6`, the measured industry-history/replay fixes. Experimental
branch: `codex/experiment-short-batches-20261028`. Direct work, no subagents,
existing checkout and D: build cache. Only owned games and local relay.

## Intended outcome and acceptance

Reduce player-visible pauses and variation in vehicle motion in populated
rooms without changing ordered simulation, masking drift, or losing actions.

- Compare the same Bubu fixture, camera, resolution, speed and action workload.
  Keep raw traces and distinguish startup/catch-up from steady windows.
- Compare native-call median/p95/p99/max, positive-update gaps, zero reasons,
  backlog and CPU. Shorter calls alone do not establish lower CPU cost or
  smoother rendered movement. Keep outliers; label unmatched workloads.
- Check every overlapping rolling digest, allocator sample and tick counter.
  No unexpected divergence, resync, missing sample or held world is a pass.
- Verify ordinary menu start, actions, pause/resume and rejoin before accepting
  a scheduling change for players. Fixtures establish isolated mechanics only.
- Keep experiments opt-in until demonstrated. Preserve acknowledgement gates,
  step/checkpoint ordering, and native ownership. No promotion in this task.

## Experiment A: smaller steady batches

The baseline at 4x runs about four updates per 200 ms callback, taking roughly
120-130 ms on this PC. A 100 ms target groups two updates; 50 ms groups one.
This may spread ordinary work and reduce action latency, but increases native
buffer swaps and per-batch overhead. It cannot split a single expensive native
industry operation or save. Existing trim/catch-up and ordering remain.

`TPF3MP_HOOK_BATCH_MS=50|100|200` selects the target when the verified interval
hook is active. Unset is 200. Invalid values log refusal and retain 200. A
batch has at least one update, so 1x remains about 200 ms even at a 50 ms target.
The native update's simulated duration is never changed. An executable model
checks jitter, action barriers, checkpoint cuts, a long stall and recovery
before a real-game comparison. Results are recorded below when measured.

The archived October 9 investigation already found that one-update/20 Hz
batches sometimes sustained only 17-19 Hz under recording load, due to false
Sync/readiness losses. This experiment starts at 100 ms and measures those
losses rather than assuming 50 ms is better. The current baseline differs:
production interval/trim, early replay staging and shared industry history.
The focused interval suite passes (22 tests, including the new coupled model).

## Experiment B: restore the faster-save option for 40425

Branch `codex/experiment-savefast-40425`, based on Experiment A. The existing
save-fast option had no targets in this game's bundle and therefore could not
apply. Ported the exact compressor sites: 0x32deb0 function, 0x32dec4 level
load, 0x32e024 buffer. Its sole caller is save setup 0x32cba0; decompressor
0x32e140 shares the level constant at 0x37e4ad8, which is left unchanged.
Both optional signatures/prologues pass tpfre verification against the exact
40425 EXE. No new compression implementation: credit remains silver2127's
TPF2 Big Maps and the existing TF3 port. Off by default pending measurement.

Compare the same paused world with ordinary and fast compression, check load
success and matching world checkpoints, and report save size as well as time.
Static target verification alone does not establish a speed-up.

## 200 ms baseline

`batch200-bubu`, Bubu `bubujujuoct10.sav`, two local games, default saving.
The 4x click was initially aimed at the wrong scaled coordinate (the 1x
button). A separate console read of a React module was invalid in that state
and produced a missing-recipe error; it did not inspect the live UI. Exclude
all that setup, the initial `cpu.csv`, and the 1x samples. Actual accepted
4x starts at epoch 1793202354; the selected trace is room steps 4301-9000.
No builds/tests ran during it. Both games closed normally. All shared rolling,
allocator and tick observations match through 9850, with no missing samples.

| Peer | Calls | median / p95 / p99 / max ms | positive-gap p95 / max ms | replay zeros |
|---|---:|---|---|---:|
| P1 | 1196 | 124.1 / 153.6 / 379.8 / 1410.9 | 229.2 / 1477.3 | 6 |
| P2 | 1169 | 132.4 / 168.4 / 554.0 / 1646.9 | 234.8 / 1736.4 | 5 |

The window covers 235.8 s, ~19.91 updates/s, with total detour wall time per
update 32.93/35.14 ms (not CPU time). The separate 240.8 s whole-process CPU
sample used 7.47/7.45 cores, working sets 5.86/5.92 GiB. These differ from
the earlier day's measurements; compare the follow-up candidates here, not
those earlier CPU figures. Raw evidence and analysis script are under
`D:\TPF3MP-cargo\performance-followup-20261028`.

## Experiment C: bounded catch-up batches

Branch `codex/experiment-catchup-budget-20261028`, based on the preceding
experiments. `TPF3MP_HOOK_MAX_BATCH_UPDATES=4|8|16` caps each released batch;
unset remains 16, invalid values log refusal and keep 16. It limits existing
gate batches, including command drains, rather than consuming steps and then
discarding any. Checkpoint/action/save/load boundaries and replay ack remain.

At the baseline's ~33-35 ms/update, 16 updates imply ~0.53-0.56 s of work.
Four would imply ~0.13-0.14 s, before batch overhead and slow native operations.
This is a capacity estimate, not a measured gain. A 20-update/s room needs
<50 ms/update including overhead; the candidate must still clear backlog,
including rejoin, or shorter individual calls are not an acceptable result.
Single expensive operations remain indivisible. Test independently of B's
compression and compare A's 100 ms/16 limit before 100 ms/4 limit.

## 100 ms observation and comparison limitations

`batch100-bubu`, same fixture, 100 ms batches, original 16-update cap,
normal compression. Shared queue/rolling/ticks all match through 18550,
with no missing overlapping observations. Both games quit cleanly.
Steps 4301-9000: P1 median/p95/p99/max 63.6/87.4/330.6/1478.8 ms;
P2 70.2/106.4/570.2/1998.5 ms. Positive gaps max 6495.6/7673.3 ms.
8/11 replay-zero calls and two save-zero calls per peer. The periodic room
save ran at epochs 1793203745-3751; this did not fall in the baseline's
selected window. Throughput ~19.14/19.17 updates/s including that save.

**Not a controlled CPU or worst-gap comparison:** the baseline's incorrect
console-close coordinate also swallowed its host zoom. Its guest and both
100 ms games did zoom. CPU was 7.94/7.41 cores and is retained only as a
run observation. Do not infer CPU regression/improvement from these runs.
The corrected paired/crossover runs retain the fixture camera (`-NoZoom`),
use the same room, and swap variants across seats. Raw first-run evidence
is retained, not replaced by the improved methodology.

## Paired AB: identical camera and world

`paired-ab-bubu`: P1 200 ms/max16, P2 100 ms/max4, normal compression,
3840x2161 windows, exact fixture camera, P1 foreground during measurement.
DLL hash, EXE hash, source commit and settings are in the run's metadata.json.
No builds, tests, clicks or recording during steps 1001-8000.

| Peer/policy | median/p95/p99/max call ms | gap p95/max ms | updates/s | p95 backlog |
|---|---|---|---:|---:|
| P1 200/16 | 125.7/153.1/470.5/1592.3 | 232.2/1631.1 | 19.898 | 13 |
| P2 100/4 | 73.0/137.1/160.0/1704.7 | 173.0/1753.9 | 19.843 | 31 |

Native detour elapsed/update: 33.38 vs 35.60 ms. Whole-process CPU over
361.3 s: P1 7.71 cores, P2 7.38; working sets 5.83/5.70 GiB. This is still
host/guest confounded until roles are swapped. The candidate lowers ordinary
and catch-up call tails, but its larger backlog and extra readiness failures
are a concern, not evidence of a complete fix. No periodic save occurred in
this window. Six/five replay-zero calls occurred; all outliers retained.

After measurement, host pause/resume reached both games and P2 issued four
company renames at 4x. All four appeared in both logs, and P1 displayed the
last name. Final checkpoint/allocator/tick comparison is in
`paired-ab-bubu/final-comparison.json`. Both owned games quit normally.

Possible next experiment if the crossover confirms the backlog problem:
small recovery batches before the 1.5-second far-behind threshold. At a
hypothetical 35 ms/update plus 40 ms batch/copy/render overhead, two updates
need 110 ms (18.18 updates/s); three need 145 ms (20.69 updates/s). This
calculation motivates testing bounded extra work, not reducing checks or
inventing steps. Presentation interval must still reflect actual count.
No such recovery-policy change is implemented in the paired AB/BA binaries.

## Paired BA: ordinary menu start, policies swapped

`paired-ba-bubu`: P1 100 ms/max4, P2 200 ms/max16, P1 foreground, original
compression. P1 created a private co-op room and selected bubujujuoct10;
P2 joined PG2QCE through Join a friend, both readied, and P1 started the map.
No console or fixture helper loaded it. Same stored camera on both peers.
This loads the original save directly; AB first reserialized it for the room.
Do not pool cross-run CPU samples as if startup/world allocation were identical.

Steps 1001-8000, no builds/tests/clicks/video in the window:

| Peer/policy | median/p95/p99/max call ms | gap p95/max ms | updates/s | p95 backlog |
|---|---|---|---:|---:|
| P1 100/4 | 65.6/93.3/131.0/1676.4 | 137.0/1714.9 | 19.879 | 18 |
| P2 200/16 | 137.8/179.2/534.1/1855.7 | 240.5/1980.9 | 19.881 | 20 |

Native elapsed/update 32.68/36.41 ms. 361.2-second CPU sample: 7.84/7.20
cores; working sets 5.80/5.78 GiB. Eight/nine replay-zero calls. All shared
rolling/allocator/tick observations match through 9950 with no missing
samples or extra reload. Both games quit cleanly. Candidate call tails are
lower in both seat assignments; that alone does not establish visual motion.

## Experiment D: early bounded recovery and delivery timing

Branch `codex/experiment-short-batch-recovery-20261028`. Off by default;
`TPF3MP_HOOK_BATCH_RECOVERY=1` only affects experimental periods below 200 ms.
Above reserve + band + two nominal batches, allow bounded half-nominal
repayment (two becomes three); above four nominal batches of pressure,
allow up to twice nominal. Both remain under the gate's cap and checkpoint
cut. Presentation duration follows actual count. Nominal/repaid
batches use the backlog trim; command/save/load/paused drains retain their
existing paths. A service-overhead model must reproduce the old accumulated
backlog and demonstrate capacity before the real run. No skipped updates.

Step tracing now also records successful presentation-buffer delivery gap
and lateness relative to the previous buffer's assigned interval, plus that
buffer's update count. World changes, pauses and untimed buffers reset the
measurement; zero-update buffers are explicitly labelled. This is wall-clock
Sync delivery timing, **not GPU frame time or a vehicle body trajectory**.
It observes existing verified hooks and changes no renderer state. Earlier
AB/BA binaries did not include it and cannot retroactively supply this metric.

D refinement: the first capacity model failed (27 steps still queued and 33
far-behind calls): three updates merely break even after frame quantization
and checkpoint cuts. The revised opt-in permits up to twice nominal under
higher pressure, always under the configured maximum. Controlled recovery
presents its full update count (four are not displayed as three). The model
now stays within 16 queued steps, uses no far-behind batches, and sustains
at least 19.9 updates/s under the specified synthetic service cost. This is
model evidence only until measured in game.

The new delivery and step traces share a monotonic `clock_ms`; buffered
integer-UTC log timestamps cannot reliably align the renderer and worker.
Delivery lateness is relative to the previous assigned interval and labels
zero-update windows. No claim about rendered frame time is inferred from it.

Validation encountered an existing parallel Windows test race: it released
an address, then another allocator reused it between probe and read. The
memory-read guard correctly returned the newly readable zero, rather than
None. The test now decommits the pages while retaining their reservation,
so it can assert unreadability without racing address reuse; production
memory-reader code is unchanged. A new record test originally passed through
on_step, which intentionally disables unarmed native hooks in unit tests;
it now tests real-link selection and Sync metadata without arming global hooks.

## D measured: smaller call tails, more medium delivery misses

`paired-recovery-savefast-bubu`: P1 200/max16 baseline, P2 100/max4 with
early repayment; fast saves enabled on both. Same fixture camera and P1
foreground. Steps 1001-8000, no builds/tests/clicks/recording in the window:

| Metric | P1 baseline | P2 candidate |
|---|---:|---:|
| Positive call median/p95/p99/max, ms | 122.7/154.2/481.9/1577.9 | 71.8/132.8/157.7/1704.9 |
| Updates/s | 19.948 | 19.951 |
| Available steps p95/max | 16/41 | 19/41 |
| Positive-buffer delivery lateness p95/p99, ms | 40.898/339.721 | 58.473/98.630 |
| Positive-buffer delivery late >50 ms | 64/1753 | 226/3164 |
| Positive-buffer delivery late >100 ms | 31 | 30 |
| Replay zero calls | 8 | 7 |

CPU over 401.3 s: P1 7.52 cores, P2 7.28 cores. Candidate's severe tails
are lower but medium missed deadlines are more frequent, both as a count
and proportion. Do not accept this as smooth motion. Measured recovery
samples assign 100 ms to four updates while the next delivery takes
150-190 ms: the backlog controller's 0.5 trim creates an unrealistic
deadline. The next experiment will keep the extra updates but bound that
acceleration, checking capacity rather than assuming slower trim is free.

After measurement, P2 placed a bus stop through the ordinary build UI.
Both peers applied PlaceStop before step 9934 (116/107 ms in Lua).
Every shared rolling/allocator/tick sample matches through 10300 with no
missing sample or unexpected reload. Both games quit normally. This run
has not demonstrated rejoin under the four-update cap yet.

Fast compression produced a 166,393,427-byte starting snapshot that both
games loaded and played identically. The initial save's observed native
time was 3794 ms. Periodic saves were 4359/4552 ms and
168,125,741/168,125,049 bytes. Earlier AB normal saves were 7244/7416 ms and
157,874,553/157,873,857 bytes: about 39% less saving time for about 6.5%
larger files. These periodic worlds are similar but not byte-identical
(events 31 vs 32), so this is an observed run comparison, not a controlled
compression-only speedup. Shared stdout writers also overwrite/interleave
parts of startup output; retain the hook logs and recorded snapshot sizes.

## Experiment E: bound acceleration during short recovery

Branch `codex/experiment-bounded-presentation-20261028`, based on D's
`21edafef`. For opt-in short-batch recovery only, keep trim >=0.8 rather
than permitting 0.5 under deep backlog. Four recovered updates at 4x are
then shown for at least 160 ms instead of 100 ms. Ordinary 200 ms policy
and recovery-disabled short batches retain their original trim. Pause/load
reset the trim value, preserving the configured bound.

The same capacity model still passes with this bound: it sustains >=19.9
updates/s, ends with <=16 queued steps and never falls into Far. At the
model's 35 ms/update +40 ms/call, four updates take 180 ms before frame
quantization, so the new 160 ms floor still leaves recovery capacity.
This is not a service-time predictor and does not eliminate an indivisible
industry/save stall. Compare actual buffer-delivery lateness and backlog
before deciding whether the trade-off improved.


## E measured: bounded acceleration helps, but does not remove stalls

`paired-bounded-savefast-bubu`, same P1 baseline/P2 candidate assignment,
fixture/camera and foreground as D, fast saves on both. Steps 1001-8000:

| Metric | P1 baseline | P2 bounded recovery |
|---|---:|---:|
| Positive call median/p95/p99/max, ms | 123.7/153.6/404.3/1586.8 | 72.6/134.5/162.4/1671.5 |
| Updates/s | 19.933 | 19.930 |
| Available steps p95/max | 14/38 | 20/43 |
| Positive-buffer lateness p95/p99, ms | 39.097/333.204 | 51.180/89.034 |
| Deliveries late >50 ms | 59/1771 | 169/3125 |
| Deliveries late >100 ms | 27 | 22 |
| Deliveries late >500 ms | 7 | 7 |
| Replay zero calls | 10 | 7 |

Over 401.4 s, CPU was 7.50/7.35 cores and resident memory 5.85/5.88 GiB.
Compared with D, the candidate's >50 ms misses fell from 226 to 169
(about 25%), but still exceeded the paired baseline. This single repeat
supports further experimentation, not a guarantee of smoother rendered
motion. The slow native calls and seven >500 ms deliveries remain.

After the measurement, explicit host pause and 2x resume propagated. P2
returned to the main menu, left and rejoined through the ordinary UI. Both
peers loaded snapshot 8328 and caught up; all common rolling/allocator/tick
samples matched through 21800 (plus the original epoch through 10150).
The one unmatched initial batch at 8343 is not a periodic checkpoint.
Both games quit cleanly; no unexpected resync was found.

However, this rejoin exposed a real lifecycle defect: the snapshot held
4x while the room had been set to 2x. At UTC epochs 1793209575 and
1793209584, the guest and host automatically requested 400% after loading.
The server rejected the guest request but eventually accepted the host's.
No speed button was clicked then. Native getter changes caused by restoring
a save must not be treated as player intent. Rejoin speed acceptance is
therefore failed pending a fix; this is not a pacing success.

Validation before this run: 586 hook tests passed, seven ignored; focused
clippy and release hook build passed. E remains opt-in on its own branch.
The retained raw summaries are `bounded-summary.json`,
`bounded-delivery.json`, and per-player `pre-rejoin-hook.log` in the local
evidence directory; benchmark summaries were generated before step numbers
repeated on rejoin.


## Reload speed fix: explicit intent, contract 17

Branch `codex/fix-reload-speed-intent-20261028` separates input from native
state. The GUI decorates the existing helper's button and keyboard APIs;
only those calls enter `chooseSpeed`. The worker uses the existing room
speed request path, retaining server authority. Both host and guest display
the accepted speed. A restored local value can no longer request anything.
The Lua contract increments from 16 to 17; network/persistence are unchanged.

The first real run `speed-intent-bubu` caught a lifecycle error: GetApi was
called by useStepStateTimer's initializer, during render. The engine refused
that with its explicit GetApi-onStep/onMount assertion. No world acceptance
is claimed for that run. The wrapper was moved to onStep, and the Lua test
now asserts that GetApi cannot be called during render. The failed run was
returned to its main menu and quit normally, before repeating the test.


`speed-intent-bubu-r2` uses an ordinary main-menu room creation, the Bubu
save picker, Join a friend, and Start. It loads the mod via normal room
selection. Initial host and guest UI load without an error; host shows the
accepted 1x. Host click 4x, Space pause, and Space resume generate exactly
those explicit requests. Guest Space while paused generates no request and
both stay paused. Seven guest company renames (Pacing-1 through Pacing-6,
then Seppoon Transport) appear in both logs in order. All shared checks
match through step 4150 before rejoin. Further rejoin results follow below.


Final R2 rejoin acceptance passed. The ordinary periodic save at event 38
held native speed 4x; after its upload, the host explicitly selected 2x at
UTC epoch 1793212071. P2 returned to the main menu, left and joined again.
Both games loaded snapshot `d7d0e599...`, next step 9659. P2 resumed at
1793212178 and P1 at 1793212192. A read-only, pcall-wrapped console query
then returned native GAME_SPEED.speedup = 4 on the host, while screenshots
showed both controls and the room footer at 2x. Neither player requested a
speed during reload/catch-up. This directly verifies the previously failing
case; it does not infer success from a unit-test count.

The host then explicitly clicked 4x (1793212318), even though the restored
native value was already 4, and Space-paused (1793212323). Both requests
were sent and applied. The complete host request sequence was
400, 0, 400, 200, 400, 0 percent; every one corresponds to test input. The
guest sent none. All shared rolling/allocator/tick samples match through
10700 in the original epoch and 11950 in the reloaded epoch; no periodic
sample is missing. The unmatched first catch-up batch at 9674 is listed
separately. There was no unexpected reload, divergence or UI error in R2.
Both games quit normally and the local rig exited.

Focused integration checks: 586 hook tests passed (7 ignored), 86 protocol
library tests and all 275 Lua mod tests passed. The Lua suite was rerun
after moving GetApi into onStep. Rustfmt, focused all-target clippy and
release hook build passed. These are Windows checks, not cross-platform
release acceptance. The broader workspace suite passed on the earlier base;
it is not described as rerun for this final follow-up.

Restoration: automatic approval review blocked deletion of three source
files added by the test installation. Instead the complete test staging
folder was preserved under the evidence root as `preserved-test-staging`,
and the original backup restored. All 95 original files and the file count
were verified. No test game/server remains running. No new worktree, release
or production deployment was made.

## Decision and remaining work

Keep the ordinary 200 ms/default batch policy. A-H remain reversible,
opt-in experiments on their named local branches. The 100 ms bounded
candidate reduces native-call and severe delivery tails but still has more
medium missed delivery deadlines than the paired baseline. H's combined
250 ms/five-update/floor policy reduces some medium delivery tails, but the
final comparison has worse p99 and is not an overall performance acceptance.
Two rejoin regressions were exposed, corrected and the final rejoin/action
scenario passed. The evidence does not establish smoother rendered vehicles
or elimination of freezes.

The explicit-speed fix has a demonstrated ordinary-start/rejoin outcome and
is separately committed after E. The save-fast target port has successful
save/reload/checkpoint evidence, with an observed time/size trade-off, but
its periodic-save comparison used similar rather than identical worlds.

Remaining large costs include synchronous saves and indivisible native
industry work. A different presentation architecture must demonstrate actual
vehicle trajectories/frame timing, correct transitions and action boundaries,
not merely lower worker call timings. This pass does not declare those stalls
fixed or promote the experimental scheduler as a default.

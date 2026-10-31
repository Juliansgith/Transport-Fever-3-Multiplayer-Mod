# Performance and late-game stability, 2026-10-28

Status: fixes implemented and validated, including ordinary startup/rejoin
and the reproduced industry desync. Automated checks pass. Native stalls
remain; the steady-motion acceptance is only partially met, and this work
does not establish freeze-free vehicle movement.

## Outcome and acceptance

Keep vehicle movement steady while a populated multiplayer world runs and
players issue actions, without weakening ordered replay, checkpoint coverage,
or recovery. Work directly on the primary agent; no delegated work.

- Compare the same save, build, speeds and observer workload before/after.
  Report simulation-call time, positive-update gaps, zero-update reasons,
  checkpoint costs, and process CPU/memory separately. Render-frame timing
  alone is not evidence of smooth vehicle motion.
- Exercise 1x, 2x and 4x, pause/resume, and commands from both players. Report
  median/tail/max and sample counts, excluding explicitly marked startup and
  speed-transition windows rather than silently discarding outliers.
- Check every overlapping checkpoint and applied action, with no unexpected
  resync, held world, crash, or missing observation concealed as a pass.
- Verify the ordinary lobby start/rejoin path as well as fixture mechanics.
- Cover late-game workload and sustained action replay; preserve deterministic
  ordering and fail-closed behavior. Reproduce a defect before changing it,
  add focused regression coverage and an integration check.

## Baseline and controls

- Source: dev `49b7864374fe2b1efa886ff32ef4150f40609ab3` (v1.3.4).
- Windows game: build 40425, EXE SHA-256
  `e9a6190f9ca8f7f34a46dfafac0f3b897e41ea6dc6498d6f5871aed6b9ca74cd`.
- Busy save: Shifty's `tpf3mp_room_1224`, existing test copy
  `tpf3mp_shifty1224_test.sav`; prior evidence describes 76 vehicles/17 lines.
  A read-only console query verified 76 vehicles and 17 lines in this run.
- Isolated local relay, own processes only. No production changes.
- Reuse the integration checkout and D: Cargo target, four build jobs.
  No builds/tests during measured game windows.
- Raw local evidence: `D:\TPF3MP-cargo\performance-20261028`.

## Prior findings reused

The preserved 2026-10-09 smooth-pacing handoff establishes separate room and
native clocks, zero-update action barriers, and native interpolation/reset
limitations. The experimental road-history renderer was not accepted and
covered only one allowlisted road vehicle. It is not part of this baseline.
Current dev already includes cadence and proportional batch-interval fixes;
measure these before reviving any old prototype.

The original Shifty desync was not reproduced in the earlier bounded runs.
Those runs do not prove that the reported failure is fixed.

## First baseline (two local games)

The following are separate two-minute trace windows, not render-frame
times. All maxima are retained. The 4x window includes a 12-second video
capture and UI/camera work; it must not be used to claim a clean CPU speedup
against an unobserved candidate. The clock origin is process-local and the
UTC log prefix belongs to the 50-call flush, not each individual call.

| Speed | Peer | Calls | Call median / p95 / max, ms | Positive-call gap max, ms | Zero-update action calls |
|---|---|---:|---|---:|---:|
| 1x | P1 | 601 | 15.7 / 34.8 / 60.4 | 592.7 | 3 |
| 1x | P2 | 601 | 16.0 / 35.2 / 44.4 | 597.2 | 3 |
| 2x | P1 | 602 | 41.6 / 52.8 / 822.1 | 840.2 | 3 |
| 2x | P2 | 601 | 42.1 / 54.0 / 824.3 | 862.3 | 3 |
| 4x | P1 | 620 | 82.9 / 100.9 / 879.5 | 919.6 | 4 |
| 4x | P2 | 625 | 85.3 / 103.4 / 926.8 | 957.8 | 6 |

Sources: `baseline-{1x,2x,4x}-p{1,2}.json` and complete hook logs in the
local evidence directory. P1 trace intervals: [260000,380000),
[690000,810000), [900000,1020000) ms; P2 uses intervals 100000 ms earlier.
These are approximate simultaneous intervals, not exact event alignment.

- World checks in observed windows averaged roughly 1.4-2 ms per update;
  the ~0.8-second spikes are inside the native step, outside those small
  rolling-check samples. Their specific native cause remains unproven.
- A 123-second process sample at 1x used 4.29 CPU cores for P1, 3.35 for P2
  and 0.019 for the rig (CPU seconds / wall seconds, not machine percent).
  Working sets ended at 4.73 / 4.57 GiB, private bytes 7.81 / 7.89 GiB.
- All 289 shared rolling signatures matched through step 14,450. All 290
  sampled native free-ID queue states matched too. Operation tallies differed
  at 23 samples; these are not silently counted as matching operation traces.
  Added diagnostics will identify history resets before drawing conclusions.
- Ordinary loan UI actions, notification actions and host pause/resume were
  exercised. Repeated clicks on a loan offer that had disappeared are not
  counted as successful actions or a sustained-input test.
- The first video had 365 frames / 12 s, capture work p50 13.0 / p95 16.4 ms.
  Its bus entered a bend; body-feature coherence deteriorated to 37.6 pixels.
  It is unsuitable as a quantitative constant-speed motion comparison.
- One periodic save upload failed with a five-second bulk timeout. Both
  games continued, but this is a recovery/late-join reliability failure.

## Changes under validation

1. Stage already-ordered commands immediately after the preceding batch
   and checkpoint finish, giving the GUI/engine wake time before the next
   simulation callback. This can avoid a compulsory zero-update call;
   it cannot guarantee that a late wake or unreleased step never pauses.
   Read-ahead remains bounded, step-validated and command-only. Replay
   acknowledgement still gates every later step, save, load and end.
2. Include the batch's first room step in optional step traces. The first
   candidate proved that the simulation engine pointer changes every call;
   the old diagnostic discarded its history on each buffer swap. Preserve
   logical-world history until an explicit load/room exit, and mark unreadable
   engine intervals incomplete. A native-memory regression failed before the
   fix and now passes, including same-address world loads and observation gaps.
3. Give a requested upload batch one aggregate, bounded send deadline.
   Previously a small compressed frame could time out after five seconds
   under receiver backpressure, despite acceptable batch throughput. The
   fetcher retains per-chunk rate limits, validation and final file hashing.
   This reproduces the observed failure class, not yet proof of which
   receiver operation stalled during the original upload.
4. Do not load React builtins from the game scripts' GUI-event state for
   line diagnostics. An ordinary road preview reproduced the red
   `WithComponentParams` error; its saved native error stack points to
   `follow.watchLines` requiring builtin.lua. Only the HUD and Multiplayer
   React roots opt into that wrapper; engine line diagnostics still run.
   The regression failed before the fix. All 275 Lua-mod tests pass afterward.

Focused checks so far: 40 bridge tests, 9 real-link pacing/replay tests and
50 step-driver tests passed. The real-QUIC receiver-backpressure regression
failed before the bulk change (truncated upload after a six-second receiver
pause), then passed; all 11 bulk tests passed, including idle/trickling-peer
and corrupt/unavailable-content coverage. Candidate gameplay is in progress.

## First candidate observations

This run includes staging, the bulk fix and temporary buffer-change logging,
but predates the history/UI fixes. All 270 shared rolling signatures, 271
free-ID queue hashes and 271 tick counters matched through step 13,500;
none were missing inside the overlapping interval. Both games applied ten
company renames (alternating host/guest input) and one host road build.
All 51 replay batches on each peer were staged. There were no `why=actions`
zero-update calls; delayed acknowledgements still caused six `why=replaying`
calls on P1 and five on P2. This is not a zero-stutter result.

Two-minute candidate trace windows (P1 [240000,360000), [390000,510000),
[550000,670000) ms; P2 55800 ms earlier, aligned using matching room-step
numbers) are saved as `candidate-{1x,2x,4x}-p{1,2}.json` locally. P1 at 2x
had 600 calls, 598 with exactly two updates, no zeros, 46.2 ms median,
56.2 ms p95 and 103.1 ms maximum. P1 at 4x had 608 calls, one replay wait
and two save waits; median/p95/max 82.4/100.5/875.3 ms. Its longest positive
call gap was 3,758 ms, including an ordinary room snapshot save. Different
room-step windows mean these maxima cannot establish a before/after speedup.

Long native calls remain: both peers had ~1.4 s at room step 1405 and
~5.7-5.8 s in the batch ending at 8650. Small rolling checks and existing
hook-piece timers do not account for them. Native stdout also reports
industry creation/connection and synchronous room saves; exact attribution
of every long call is still pending. Save-fast was **off** in both runs.
No divergent/held-world event or upload failure was observed in the candidate.

All 568 enabled hook unit tests passed (7 ignored). Further game checks must
use the rebuilt history/UI fixes, including ordinary menu startup and rejoin.

## Larger-save failure: automatic industry creation

The ordinary menu run `candidate-menu-bubu-retry-40425` used Bubu's 150 MB
save (73 stations, 25 lines, 66 vehicles). Both peers created/joined through
the actual menus and loaded the host's world. At 1x, P1's clean two-minute
window had 600 calls, all with one update, 25.1 ms median, 42.2 ms p95,
96.7 ms maximum, and 247 ms maximum positive-call gap. A later 2x interval
reproduced a genuine desync; it is not a successful stability measurement.

- All sampled free-ID queue states agreed through step 3,600. At 3,650,
  P1 had 329 queued IDs versus P2's 354. P1 alone had applied two native
  modifications outside the room updates, taking 29 IDs and freeing four
  (return RVAs 0x8f5057 and 0x90434f). Their exact callback inputs are not
  yet known.
- Native stdout reports Brierley Machine Factory on P1 and
  Kirkby-in-Ashfield Machine Factory on P2. Lane comparison confirms
  different access roads/locations, not merely different diagnostic IDs.
- The first rolling mismatch is step 3,800, lane 0. Later lane 1/4
  differences precede automatic resync. These are not counted as separate
  established root causes. Both worlds subsequently loaded the recovery save.
- The repaired allocator history exposed another diagnostic weakness:
  empty native modifications consumed the 96-record budget, and the byte
  cap was spent on the oldest checkpoints. The next build keeps up to 512
  allocator-changing/unknown observations per checkpoint, counts unchanged
  observations separately, and emits recent checkpoints first. Truncation
  remains explicit; aggregate counts still include all observations.
- The optional industry probe was hardwired to 40408 and could not run on
  40425. It now selects the exact compiled bundle's diagnostic profile.
  All six signatures/prologues resolve uniquely against the installed EXE;
  this static check does not establish runtime correctness.

A preceding ordinary-menu attempt exposed a reused Lua-state address being
mistaken for an already-patched state. The next build checks the marker in
the live resolveutil table, including cached-loader entry, and logs SERVED
only for the actual mod namespace. Its state-replacement regression passes;
fresh-game verification remains pending. All 570 enabled hook unit tests
pass after these changes (7 ignored).

## Industry timing reproduced with exact-build probe

`industry-probe-bubu-40425` starts through both ordinary menus with the
live-state menu-marker fix. Both players reach the same world; no missing
Multiplayer menu is observed. At 4x the first industry spawn agrees (native
update 166799, room step about 3631, entity 175190). Later spawn attempts
occur at different updates: P1 attempts 167399 while P2 next attempts
167579. These attempts return no entity, so divergence is not yet visible
in the allocator. Both create entity 154090 at 169259, then P2 alone
creates 222847 at 169319; P1 next creates 221861 at 169379.

Free-ID states agree through room step 6150 and differ at 6200. The games
were paused around 6850 before the rolling scanner found the changed area.
No rolling alarm yet is **not** evidence of stability. The menu fix has not
fixed this desync. Local probe output has no drops or callback-limit event.

Static analysis of this exact EXE shows the native step drains queued
callbacks before **each** ECS update. Thus allocator messages saying
"outside updates" do not by themselves prove an illegal extra update;
their label depends on where the batch's first ECS update occurs. The
industry scheduler subscribes to CGameTime::OnIntervalStep, compares saved
town/global demand sets, and adds five pending attempts on a demand change.
The next probe records those comparisons and the pending count, read-only.
The first new probe launch refused ambiguous signatures; strengthened
signatures then matched all nine sites uniquely. Runtime verification is
still needed. All eight focused industry-probe tests pass.

The corrected scheduler probe ran in `industry-schedule-bubu-r2-40425`.
At interval time 33408000 the active callback sees town 102905 change from
`[6,20,28]` to `[6,16,20,28]` and adds five attempts. At 33420000 the
other buffer's active callback sees that same change and adds another five.
Inactive callbacks report a separate, unchanged pending count. At 33468000
P1 reads pending 3 and P2 reads pending 4, with identical current demand
sets and no comparison changes. P1 subsequently goes from pending 2 to
pending 4 merely by switching buffers. Spawn attempts then occur at
different native updates; another automatic resync followed around room
steps 7050–7100. Full local logs and `scheduler-summary.txt` retain this.

Prototype under `TPF3MP_HOOK_INDUSTRY_SHARED_STATE=1`: retain one native
IndustryChainSpawnState shared_ptr for the logical world and substitute it
in a stack copy of the active scheduler's captures. Each invocation retains
its original GameState, resource, logger and callback queue. The native
algorithm still owns/computes its demand sets and spawn choices; inactive
buffer callbacks are unchanged. World-load epochs discard the retained
history before the next invocation. This is still opt-in and depends on
the diagnostic hook; it is **not yet a completed production fix**.
Ten focused probe/sharing tests pass, including the native shared_ptr
last-owner destruction model. The initial timing model accidentally used
two buffer schedules whose spawn indices were identical; the corrected
scenario explicitly exhausts one buffer while the other still has work.
Real-game validation and final integration remain outstanding.

## Shared-history candidate result

`industry-shared-bubu-40425` ran at 4x through step 17300: all 346 shared
rolling checkpoints, 347 allocator queue states and 347 tick counters
matched. All 289 active scheduler records and ten spawn attempts matched
(time, seed, capacity and returned entity). No desync/resync occurred. This
extends beyond the unmodified failures at 3650 and 6200, but does not prove
all causes of desync are fixed. Diagnostic mode was enabled throughout;
this run is not used as a clean performance comparison.

The normal-build integration now installs only the scheduler hook by
default. Diagnostic demand reads/allocations/logging remain opt-in. Invalid
native ownership holds the room. The production fix has no per-peer opt-out,
since mixing independent and shared scheduling would itself cause drift.
Eleven focused tests pass, including invalid shared-pointer layouts and
reference-count overflow. The installed EXE passes the updated 166-target
static proof; `tpfre verify` finds every release and diagnostic site unique.
Fresh ordinary-menu validation with diagnostic mode disabled is in progress.

## Normal build: ordinary start, action workload and rejoin

`production-bubu-40425` uses the default industry fix with its optional probe
off. P1 created a private co-op room through Multiplayer and selected the
existing Bubu save; P2 joined by code. Both loaded the host's world. This is
the ordinary lobby path, not console fixture loading.

- Before P2 left, all 272 shared rolling checkpoints and 273 allocator/tick
  samples matched through step 13600. No unexpected resync or held fault.
- Both players issued 12 company renames each, alternating over about 85 s.
  A road built by P1 appeared on P2 (65/51 ms replay). Renames are a modest
  command workload, not evidence about many expensive simultaneous builds.
- P2 returned to the main menu, left the room, then rejoined by code. The
  existing server behavior gave **both** peers the same snapshot, resuming
  at step 18288. Both created a fresh shared industry history. All 161
  further rolling/allocator/tick samples matched through step 26300: over
  8000 updates after reload, with no unexpected resync. Pause and resume
  worked; the pause first drained already-released catch-up steps.
- Comparison is separated by world-load epoch. A reloaded snapshot can
  legitimately change native ID allocation relative to the earlier world;
  comparing a pre-reload sample against another peer's post-reload sample
  would be invalid. Native counters can be logged before the buffered
  `playing ... from step` message, so use the earlier `loading ... next`
  marker for the epoch boundary.
- Both games quit cleanly. Raw comparisons are in
  `production-bubu-40425/final-comparison.json`.

Clean stationary-camera windows, no builds/tests/video/clicks during each
window (approximately two minutes per speed):

| Speed | Peer | Calls | Call median / p95 / max, ms | Positive-call gap max, ms | Zero-update calls |
|---|---|---:|---|---:|---:|
| 1x | P1 | 600 | 24.3 / 41.7 / 68.3 | 252.6 | 0 |
| 1x | P2 | 600 | 24.9 / 43.2 / 67.3 | 249.6 | 0 |
| 2x | P1 | 601 | 57.6 / 81.1 / 1536.5 | 1576.1 | 0 |
| 2x | P2 | 608 | 56.5 / 82.7 / 1660.9 | 1694.0 | 0 |
| 4x | P1 | 604 | 121.2 / 157.9 / 1403.5 | 1465.9 | 3, replay acknowledgement |
| 4x | P2 | 592 | 130.3 / 176.3 / 1527.9 | 1585.6 | 3, replay acknowledgement |

Sources: `clean-1x.json`, `clean-2x-4x.json`. Room speed notices confirm
100/200/400 during these windows. Automatic NotificationSeen commands
caused replay waits even without player input. Rolling checks usually took
2.1–2.5 ms/update, maxima around 5 ms in normal play; catch-up is a separate
workload. Over the 131.22 s CPU sample, the processes used 5.42/5.30 CPU
cores and 5.56/5.48 GiB working sets. These include the entire games at
3840x2161 on one PC, not just the hook. Rendered frame times and a valid
constant-speed vehicle trajectory were not measured.

Native stdout records industry connection work around 0.89–0.99 s for
several newly spawned industries, consistent with part of the long calls.
The two stdout writers interleave, so not every maximum can be assigned
to one peer or one function. Synchronous industry construction and saves
still produce visible stalls. Moving this work to another thread without
establishing engine ownership and ordering would be unsafe. The old
one-vehicle interpolation experiment is not included in this fix.

## Speed limiter found by the rejoin test

After reload, the host autonomously requested 300%, 200%, 100%, 200%, 300%
while no speed input was sent. The exact game's `GameSpeedHelper` calls
`commit` when its local estimated maximum falls, and our native speed getter
mistakes those component changes for the host's intent. Guests also sent
owner-only requests which the server refused. This is separate from the
industry desync and from the measured steady 1x/2x/4x windows.

The GUI wrapper now preserves the supported 4x range in multiplayer, leaving
the room's gate responsible for pace. The original estimate is restored
outside multiplayer. The focused regression exercises the stock condition
and clamp against estimates 0/1/2/3/4, including the subtle repeated-commit
bug that returning zero would cause. Fresh real-game verification follows.

`speed-cap-bubu-40425` verifies the final build. Its initial fixture load is
not counted as ordinary-start coverage (the preceding run supplies that).
P2 then returned to the menu, left and rejoined using the normal UI. This
time the server reused the initial snapshot, so P1 continued while P2
replayed from step 1. The host remained at 400% throughout catch-up; its
only speed requests were my explicit 400%, pause, then 400% resume. Neither
player sent an automatic 3x/2x/1x request, and there were no refused guest
speed requests. The guest finished catching up after the host pause; this
does not establish prompt hotjoin on a CPU-saturated 4x world.

All 151 rolling checkpoints and 152 common allocator/tick samples matched
through step 7550, including samples replayed after rejoin. No periodic
samples were missing. P2 additionally logged its first catch-up batch at
step 16; P1 did not have a checkpoint there. This unmatched lifecycle sample
is reported separately, not mistaken for a missing 50-step checkpoint.
No unexpected desync/resync, native fault or missing-React banner appeared.
Both games quit cleanly; the local server exited. The pre-test staging mod
was restored and every backed-up file's hash matched.

## Automated validation and limits

- `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings`
  pass. Clippy caught one manual alignment-modulo expression; the equivalent
  `is_multiple_of` form is used in the rebuilt release DLL.
- `cargo test --workspace`: 2050 passed, 14 ignored, no failures. Full output
  is `workspace-tests.log` in the local evidence directory. The ignored
  tests were not silently counted as exercised.
- Both 40425 profile tests passed with `TPF3MP_TF3_BUILD40425_EXE` pointing
  at the actual installed executable, including all 166 target RVAs.
- `tools/game/test-tools.ps1`: all nine helper comparison cases pass;
  every game helper parses. The release hook build passes.
- This PC provides Windows gameplay evidence only. The real-game action
  workload covers two co-op players, not an eight-player construction storm.
  No production relay was changed, and no zero-freeze or all-desyncs-fixed
  claim is supported by these results.

No new checkout was created. Builds reuse the existing D: Cargo target.
Automatic approval rejected removal of generated snapshot caches, including
explicit individual paths, with only `blocked by policy`. Those caches are
retained; the cleanup is not reported as completed.

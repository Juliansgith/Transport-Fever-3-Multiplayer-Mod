# H9NUKX: vehicles hesitate while the camera stays smooth

The player reported repeated split-second pauses of most vehicles in a
two-player room at 2×. Camera movement and the UI stayed smooth. The game
was not launched or changed for this investigation. Evidence is the room's
redacted diagnostics `V6Y6WF` (Seppoon) and `NV2SU2` (bubu), the local
`hook.log`, and the code at the time of the report (client 1.2.8, Steam
Windows build 40408). The room was `r-76f482ed88d12b65ce2b7d4bf945a454`.
Raw diagnostic copies and the parsing scratch file are under ignored
`target/h9nukx/`; they are not part of the repository.

## The actual 2× interval

The host set 2× at 21:34:08 UTC on 2026-10-06 and paused at 21:52:29 UTC.
The analysis below uses 103 regular ~10-second `perf:` windows inside that
interval, excluding 20 seconds at each transition. Earlier long parts of
this session were at 4×; from 21:57 onward the room was at 1×. Mixing those
periods gives misleading call and update rates.

| client | wall time sampled | `GameSim::Step` calls | simulation updates | zero-update paused-path calls | median / p95 game-step time per wall second |
|---|---:|---:|---:|---:|---:|
| Seppoon | 1,041 s | 5,180 (4.97/s) | 10,329 (9.92/s) | 102 (1.97% of calls) | 243 / 275 ms |
| bubu | 1,045 s | 3,975 (3.80/s) | 10,408 (9.96/s) | 54 (1.36% of calls) | 453 / 612 ms |

Both made the intended ~10 updates/s. The guest made fewer native calls
and ran 2.62 updates per call on average, versus 1.99 for Seppoon. This
establishes coarser native simulation batches on the guest, but does not
show their individual spacing or explain the host's visual impression by
itself. The counters do not expose the sequence of 1-, 2-, and 3-update
calls. Nor is the fraction of zero-update calls a measure of how visible a
cluster of pauses was.

The local rolling world check's per-check `last_ms` was at most 4 ms in
this interval. The guest's was 8 ms at p95 and 43 ms at maximum. A separate
`rolling-check-cost` statistic reports each 50-update window's *maximum*:
Seppoon p95/max 5/7 ms; bubu 39/61 ms. Those are different statistics.
The gate's own code cost was below 0.3 ms per wall second on both clients.
These measurements do not include render-frame times or vehicle-transform
timing. The larger guest simulation/check tails can cause occasional
hitches; they do not prove the constant vehicle-only symptom.

## Code paths that can change movement cadence

The server advances the room frontier on a 100 ms tick at 2×, normally
sealing ten simulation steps per second (`pacing.rs`, `room.rs`). The hook
calls the game's original `GameSim::Step` once per native call, about five
times per second on Seppoon. `Session::batch` takes all released steps then
available, up to 16 and the next checkpoint; it does not hold back steps
to make two per call (`bridge/src/session.rs`). If none are released, the
step driver asks for zero updates and the world stands still (`step.rs`).
The game's renderer is expected to interpolate from each native batch
(`docs/HOOKS.md`, “The step gate in the game”). Network-arrival phase could
therefore change batch sizes, but the old 10-second counters cannot confirm
whether this happened often in Seppoon's game.

The guest speed UI has another presentation mismatch to investigate:
`speed_control.script.lua` shows the accepted room speed without writing
`GAME_SPEED`, while the hook returns the game's own `GetSpeed` to camera,
particle and vehicle-transform callers; only the simulation step reads
the room-selected count. Around 21:25 and 21:32 bubu's game requested
300% even though the room later accepted 200% or paused. The logs do not
continuously record the guest's own speed during the focused interval, so
this is a candidate for guest motion, not a confirmed cause. Seppoon's own
request and the room both showed 200%.

Tearded's draft native rolling-check reader (PR #111) was disabled in these
games. Its published 8×2 benchmark improves a whole-map sweep from 482 s
to 9.4 s, but raises average check cost from 1.71 to 3.50 ms per update and
the reported per-window spike p95 from 5 to 10 ms. It is a faster drift
detector, not evidence of a vehicle-motion fix. Its reader/default/migration
questions remain for the owner.

## Next discriminating measurement

On a future ordinary start, align a player-marked hesitation with the hook's
per-call `step-trace` (`TPF3MP_HOOK_STEP_TRACE=1` at launch) and render-frame
timing. Classify call gaps, update counts, consecutive zero-update calls and
their `why` (`wait`, `actions`, `save`, `load`), game/call cost, and the
room's release timing. A lightweight 10-second `perf-step:` histogram can
show whether uneven batch sizes are common without verbose trace, but cannot
prove visible motion by itself. No current evidence justifies changing the
16-step catch-up cap or injecting speed commands into a live room.

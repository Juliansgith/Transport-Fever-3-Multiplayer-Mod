# Rolling world checks, 2026-10-05

## Outcome and acceptance

Replace the recurring synchronous full-world checkpoint read with small
same-step observations. Keep all seven existing lanes, canonical rows and
failure detection; keep comparisons independent of frame rate and local
entity IDs. A saved world must carry a partially completed observation
window. A failed/skipped read must stop, not become an agreed `err` value.

This changes the sampling contract: static differences wait for their area
to be visited, while the five dynamic lanes rotate every five updates.
Reports still use the room's checkpoint grid. It is not a full-world
snapshot at every checkpoint, nor a promise that the base game never stalls.

## Implementation

`lanes.rolling` visits 1 km squares through the engine's octree. More than
32 static objects subdivide a square before canonicalization, down to
32 m cells. Empty areas advance cheaply. Endpoint junctions accompany
intersecting edges. Split counts and the sampled region enter the digest,
so different subdivision decisions cannot pass as matching observations.
No borrowed component survives a call. The full reader remains for dumps.

The Lua bridge is version 14. `checkpoint()` also supplies the absolute
simulation step; `scanned()` acknowledges each read, including failures.
Rust refuses failed, duplicate or missing acknowledgements. The script
saves the digest window, cursor and subdivision queue with the world after
each update. Timing counters live in the hook, outside saved/checksummed
state. No worker reads the live ECS and no dirty-event cache can miss an
automatic road or construction change.

## Real-game evidence

TF3 Steam build 40408, two games on this PC, local rig server, fixture
`tpf3mp_silver_ab_20261001`. All games started for these checks were closed.

First, `target/game-runs/pr102-spatial-sim` checked spatial coverage on the
simulation thread against the full engine inventories. At the first shared
checkpoint both games found every one of 5,852 edges, 5,527 network nodes
and 6,187 constructions among 239,440 spatial candidates. Subsequent checks
also had zero missing entries while the world grew. A 25-cell prototype
around the map centre was a feasibility measurement, not the final timing.

The final run was `target/game-runs/pr102-rolling1`. Both games completed
the first sweep on exactly step **2,412**, or **482.4 seconds** at the room's
normal five updates/second. Both continued past step 4,300 without a logged
divergence, failed rolling read or skipped acknowledgement. The first sweep
covered the entire map, including busy areas, rather than only the cheap
prototype cells.

Exclude timing windows until ten seconds after the build/check processes
finished. The remaining 148 windows contain 7,400 per-update read samples:

| measurement | P1 | P2 | combined |
|---|---:|---:|---:|
| samples | 3,700 | 3,700 | 7,400 |
| mean rolling read, including saving its state | 3.767 ms | 3.693 ms | 3.730 ms |
| maximum rolling read | 21 ms | 19 ms | 21 ms |
| median complete checkpoint simulation call | 77.85 ms | 72.75 ms | 76.45 ms |
| median ordinary simulation call | 59.3 ms | 56.3 ms | — |

Raw data: [rolling_windows_2026-10-05.csv](rolling_windows_2026-10-05.csv)
and [rolling_steps_2026-10-05.csv](rolling_steps_2026-10-05.csv). Read costs
use the game's Lua diagnostic clock (millisecond granularity); simulation
calls use the hook's step trace. Checkpoint calls are matched by ordinal
among trace lines explicitly marked `lanes`, with count assertions. The
next trace line after a report is not necessarily that checkpoint's call.

The earlier full-reader run measured 645.5 ms median reads and 713.5 ms
median checkpoint calls. These are separate runs with different sampling
contracts, not a same-run throughput speedup claim. The recurring full
read is absent from the normal checkpoint path.

Other engine work still stalls: this run included a roughly 13-second room
save and ordinary simulation calls near two seconds while the game created
and connected an industry. Those are not rolling-read costs. Extremely
dense overlapping objects in a minimum-sized cell can also exceed the usual
read budget; the implementation reads them rather than silently omitting
coverage. The measured maximum is not a universal hard deadline.

## Reliability checks

Focused regression tests cover full/spatial row equivalence, duplicate
query entries, different local entity IDs, changed junction settings,
construction insertion/deletion/changes, geometry, vehicle movement and
money. A dense-area test verifies subdivision happens before serialization
and a fresh Lua state resumes its saved pending queue. Mid-window history
round-trips through the same Lua value representation used by the bridge;
missing history, repeated/skipped steps and missing spatial APIs refuse.
Native tests verify failed/missing/duplicate read acknowledgements hold.

A late player's actual UI join/rebase was not exercised in this run; the
saved-history continuation is covered by the regression tests. The live
run established complete coverage and no reported divergence, not an
independent full-world equality probe at every update. The per-checkpoint
`signature=` log was added afterward for easier future comparison; cached
Lua modules did not load that diagnostic-only addition during this run.

Validation passed: workspace formatting, workspace Clippy with warnings
denied, the complete workspace test suite, and the optimized hook build.
The final spatial-junction edit assertion was rerun after it was added.
Test staging was restored to its pre-test PR baseline; future rig runs
install this branch's version-14 mod together with the new hook.

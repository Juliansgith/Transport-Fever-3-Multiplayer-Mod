# Vehicles leaving a depot, two games apart -- 2026-09-30

Build 40408. Two games on one PC in one room of the deployed server (the
rig, `tpf3mp-rig`), each with the hook, playing the vehicles-and-lines
scenario (PLAYING.md): a loan, a road depot, two bus stations, a bus, a
line, the bus sent to it. Every finding here is **MEASURED**: read from
both games' logs in the same simulation update. The traces were added to
the installed copy of the mod for these runs only; the repository's mod
logs none of them.

## The finding

A bus leaving a depot enters its path at a different place in each game.
In its first update both games have it on the first edge of the same path
at the same speed (0.7 m/s, from rest), but at a different distance along
that edge, in the simulation's own state (`MOVE_PATH.dyn`):

| run | game A | game B | apart | room |
|---|---|---|---|---|
| real38, real39, real41 | same | same | 0 | no divergence |
| real40 | 7.2178 m | 7.2208 m | 3 mm | diverged at step 650 |
| real44 | 7.1848 m | 6.8858 m | 0.30 m | diverged at step 5350 |
| real45 | 7.1808 m | 7.1468 m | 3.4 cm | diverged at step 1100 |
| real46 | 7.2298 m | 7.2578 m | 2.8 cm | diverged at step 950 |
| real48 | 7.3228 m | 7.1488 m | 17 cm | diverged at step 550 |

Across runs and games the bus starts between 6.8 m and 7.3 m along the
same 24.4 m depot edge, about what it covers in one simulation update at
2 m/s. The speed, the acceleration, the path (the same edges and indexes),
the purchase time, the maintenance and every other field of
`TRANSPORT_VEHICLE` (131 fields) and `MOVE_PATH` agree exactly (numbers
compared at `%.17g`). The place the bus started from at rest (`dyn0`)
differs by the same amount. From there the two buses keep the gap, and the
room's vehicles lane reports it at the next checkpoint.

## What it is not

- **Our actions.** Both games apply the same actions in the same update,
  and the depot and its edges agree to 0.1 mm at every checkpoint (a hash
  of every street edge's ends and tangents).
- **Entity ids.** The builder's game numbers entities differently from its
  first build on: the depot was construction 7407 in one game and 7408 in
  the other (real46), since the builder's own tool previews use ids. So
  real48 started both games from one save that already held the depot, the
  stations and the line (`tpf3mp_fixture3`), and only bought and sent the
  bus. The construction and edge ids hashed the same in both games at every
  checkpoint of that run, before and after the purchase, and the bus still
  started 17 cm apart.
- **The frame's view of the vehicle.** The world position and `dyn0`
  follow each game's frames (HOOKS.md, the vehicles lane), which is why the
  lane reads `dyn`; the difference here is in `dyn` itself.

## Also seen

- **After a resync, a second divergence without an action (real45).** For
  2,500 updates both buses ran identically. Then, in the same update and
  from the same state, one bus braked (10.05 to 8.90 m/s) while the other
  sped up (to 10.43 m/s). Something ahead of it differed; private cars are
  the likely candidate, and the script cannot list them
  (`getEntitiesWithComponent(MOVE_PATH)` fails), so this is not measured.
- **A town street built with different geometry (real44).** With no action
  between two checkpoints, both games gained the same street edge (195 to
  196 edges), but its ends hashed differently. Seen once.
- **Loading renumbers.** After a resync, the constructions' ids match the
  host's, but the street edges' do not (edges 7547 and 7546 at the top):
  loading a save is not the host's live numbering.

## The cause, found later the same day

It is not timed by frames. Every departure measured starts exactly on a
millimetre (each position ends in `.xxx775`), and one formula fits them
all on the fixture's road depot:

    start = 7.726775 m - (entity id mod 1000) mm

(two buses bought in one room to test it, entities 7365 and 1012, started
at 7.361775 m and 7.714775 m as predicted). So the depot sets a leaving
vehicle back along its first edge by its entity id, and the games gave the
same bus different ids.

Why the ids differ, from an in-memory trace of `ecs::Engine::AddEntity`
(0x2bb37b0) and `RemoveEntity` (0x2bb75f0) in three games of one room:

- `AddEntity` reuses freed ids from a first-in, first-out queue (a
  `std::deque<int>` at +0xe0) and grows the table only when it is empty;
  `RemoveEntity` queues the id again. The simulation creates and removes
  entities all the time (people, cargo), so every new id depends on the
  whole history of that queue.
- All of it runs on the simulation thread, in both engine buffers alike;
  the interface creates no entities there, and a player's own clicks do
  not disturb the ids.
- Saving and loading renumbers some entities and rebuilds the queue. The
  host kept playing its own world while the others loaded the host's save,
  so from the first step the host handed out other ids than they did. The
  two players who loaded the same save allocated identical ids in
  identical order, and gave the bus the same id and start; the host did
  not (7665 against 7618).

## The fix

Every game plays from the same loaded save: whenever the room hands a world
to one member, it hands it to every member playing, the owner too, and all
of them reload it (docs/PROTOCOL.md, "Everyone loads it"). With it, three
games in one room allocated identical ids from the first step, gave a bus
bought by one player the same id and start in all three, and ran it
identically for 1,788 updates and four stops, then on to 4,200 updates,
with no divergence under the full lanes.

The options below were written before the cause was known; none is needed.

## What the room does now

It catches each divergence at the next checkpoint and sends the diverged
game the room's world, which that game loads (about 10 s) and plays on
from. Nothing is lost and no world is damaged, but a room with buses
resyncs a player soon after each departure from a depot, and sometimes in
between.

## Options, for the owner to weigh

1. **Find the cause in the game and make it deterministic** from the hook:
   where a vehicle leaving a depot takes its first place on its path
   (`tools/tpfre`, D14). The spread, one update's drive, points at
   something timed by frames rather than by the simulation. The effort is
   unknown until it is found. A start: build 40408 keeps the assertion
   paths of `Game/ecs/VehicleDepotSystem.cpp` and
   `Game/ecs/LandVehicleMoveSystem.cpp`, cited by 3 and 9 functions in its
   index (`investigation/dayone-2026-09-29/TransportFever3.tpfdb`, the
   `xrefs` to those strings).
2. **Resync every guest after each departure**, before the checkpoint
   notices: correct, but a load per departure per guest.
3. **Tolerate small vehicle differences** in the vehicles lane and let the
   money and cargo lanes catch what matters to play. Fewer resyncs, but the
   drift grows (the braking above), so they come later rather than never.

Nothing here changes a decision; each option would be one.

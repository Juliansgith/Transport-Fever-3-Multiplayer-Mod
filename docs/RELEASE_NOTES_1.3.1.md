# TPF3-MP v1.3.1

Version 1.3.1 updates the Windows setup experience and makes vehicles move
more evenly in multiplayer. Players and servers should update together. The
supported Windows Steam game build remains **40420**.

## Changes since v1.3.0

- [#135](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/135)
  evens the room's released simulation steps across game calls and adjusts
  the game's batch interval so vehicles move at a steadier speed, especially
  at 4x. The hook enables the interval adjustment only for verified Windows
  build 40408 and 40420 layouts and disables it if its checks fail.
- [#136](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/136)
  lets Windows setup skip stale Steam libraries on missing drives. Setup also
  explains why a selected mods folder cannot be used and accepts an Explorer
  path pasted with surrounding quotes.
- [#134](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/134)
  unlocks construction and finance when a saved tutorial enters a
  multiplayer room. The game's own tutorial cleanup runs on the first
  simulation update; unrelated mission events remain refused.
- [#133](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/133)
  documents how contributor credit is checked before publication.

## Validation and known limits

On Windows game build 40420, a local two-player room loaded one shared
fixture. The host selected 4x through the game's controls; the guest bought
a bus and assigned it to Line 1 through the normal depot UI. Both games
applied the actions and remained responsive after the host saved through the
game UI. The 48 world-probe samples both games recorded through step 9900
matched. At 4x, some scheduled samples were skipped when a game call crossed
their step numbers, so the strict every-sample comparison could not pass;
the common samples and room's divergence reports were checked separately.
The rig did not produce its final lane summary after the games quit. This
short run exercised a manual game save, not a later automatic room save.

One unexplained guest hang occurred in an earlier, longer 40408 test of the
new pacing shortly after a room save; it did not recur in that investigation
or this short 40420 run. A separate 40420 industry-spawn divergence remains
under investigation. This release does not claim to resolve either issue.

## Contributor provenance

Max (tearded) and Claude Opus 5.5 are recorded on the implementation commits
for #135 and #136. The tutorial fix in #134 was implemented by _Sep via
Codex under the repository's Juliansgith commit identity, following reports
from Peter and Traceington. See [Contribution provenance](CONTRIBUTION_PROVENANCE.md)
for the project's credit method.

To update, close the game and restart the launcher, or download
**TPF3-MP.exe** for Windows. Released packages are available for Windows,
Linux and macOS; a matching protocol 19 server is required.

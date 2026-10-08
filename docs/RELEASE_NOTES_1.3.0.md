# TPF3-MP v1.3.0

Version 1.3 adds support for Transport Fever 3 Windows Steam build **40420**. It also brings the multiplayer gameplay, performance, and launcher changes merged since v1.2.8. Players and servers need a coordinated update because the multiplayer protocol and action schema have changed; a v1.2.8 client cannot join a v1.3 room.

## Game compatibility and performance

- [#129](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/129) supports Windows Steam build 40420 with a separately verified native hook profile and the updated game lobby pages. A normal local two-game 40420 room loaded a shared save, synchronized a line rename, and matched nine world checkpoints through step 2300.
- [#130](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/130) makes the hook compile safely on 40420 without unverified save-fast patch sites. [#128](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/128) adds an optional faster save path for the older, verified 40408 build only; 40420 uses the game's normal save path.
- [#109](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/109) reduces simulation work with bit-identical emission-grid and component-lookup paths where their exact-build checks pass. [#106](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/106) and [#110](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/110) reduce overhead when the hook reads game memory.

## Multiplayer play

- [#105](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/105) fixes station module edits, synchronizes calendar speed and pause, and places signals accurately on bridges.
- [#122](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/122) lets Auto Signals place and replace spaced signals in rooms while refusing mixed track-property edits it cannot replay safely.
- [#125](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/125) enables synchronized station, vehicle, town and depot renames and vehicle recolouring. [#126](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/126) adds regression scenarios for these actions.
- [#127](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/127) lets players accept and decline subsidies in rooms; the accepting company receives the offer's payout and consequences.
- [#116](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/116) adds trusted regional-server discovery, closest-server room creation, and read-only cross-region invite resolution. Additional regions appear only when the released launcher is configured with them; no new regional server is deployed by this release.
- [#112](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/112) restores the Set up world step after choosing a new world. [#113](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/113) prevents room mod lookup from waiting forever.

## Reliability and diagnostics

- [#107](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/107) keeps a solo game running if its launcher connection drops. [#120](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/120) keeps the game link and room session alive after a launcher renderer failure, with browser fallback.
- [#115](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/115) avoids comparing reloaded games against world-check verdicts from an abandoned world.
- [#118](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/118) traces free-ID queue divergence, [#121](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/121) adds opt-in local industry-spawn tracing, and [#123](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/123) reports simulation batch cadence. These diagnostics do not by themselves fix a later industry divergence or every brief vehicle pause.

## Packaging and future features

- [#131](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/131) sets the 1.3.0 package version and prepares these release notes. [#132](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/132) validates server upgrades with the published launcher workflow, saved turn history, and the expected old-client protocol result.
- [#117](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/117) adds a fail-closed framework for signed native-mod indexes and local package storage. No native-mod signing key, index, player installation page, or native-mod room terms ship in this release.
- [#119](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/119) prepares signed-release announcements on Discord when a release is published. [#124](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/124) scripts the plain fixture save for repeatable tests.

## Contributor provenance

PR #116 records silver2127's request for regional servers, and silver2127
opened #117. The initial implementation commits in #116 and #117 record
Julian Cooper and Claude Opus 5.5 as co-authors; their follow-up commits
record Julian Cooper. The save-fast work carried from #108 into #128 cites
[silver2127's TPF2 Big Maps](https://github.com/silver2127/tpf2-bigmap) as
prior art; the TPF3 implementation commit records Julian Cooper and Claude
Opus 5.5 as co-authors. In #122, Max (tearded) and Claude Opus 5.5 are
recorded on the initial Auto Signals implementation, test, and game-evidence
commits; Julian Cooper authored the final safety and integration commits.
The 40420 work in #129 and #130 records Julian Cooper as implementation
author. These are distinct roles: PR authorship or a request does not by
itself establish commit authorship. See
[Contribution provenance](CONTRIBUTION_PROVENANCE.md) for the linked
commit-level record.

To update, close the game and restart the launcher, or download **TPF3-MP.exe** for Windows. Released packages are available for Windows, Linux and macOS; a matching protocol 19 server is required.

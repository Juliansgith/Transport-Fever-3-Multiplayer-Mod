# Open-PR integration, 2026-10-02

Reviewed and integrated directly at the owner's request, without subagents.
Target: `dev`. This does not promote a release or deploy the relay.

| PR | Decision |
| --- | --- |
| #53 | Keep company perk capture/replay and fixtures, with `perks=false`. Refuse marketing when the company balance cannot be read, including nonfinite values. |
| #54 | Keep the fail-closed refusal of sell-on-arrival, which crashes the native game. Ordinary depot return remains available. |
| #55 | Keep Historic Preservation capture/replay and fixtures, with `preservation=false`. |
| #56 | Keep bridge/tunnel window rebuild recognition, with `bridges=false`. |
| #57 | Keep explicit airport/airfield hangar resolution, ambiguity refusal, and purchase diagnostics. |
| #58 | Keep the town street-field cache bypass and exact-site checks. |
| #59 | Keep deterministic person/cargo batches, destination candidates, path queue and freed-id ordering with exact-site checks. |
| #60 | Keep diagnostic hooks, fuller lane dumps and snapshot export. Change road diagnostics to opt-in, and cap the recorder at 100,000 lines and 64 MiB of text, reporting truncation. |

The merge retains all eight original PR heads in its ancestry. Their overlapping
Lua changes, hook installers, profiles and static proofs are combined. Action
schema 24 gives Perk tag 21 and Preserve tag 22; schema 23 clients are refused.
Station access and enabled terrain brushes retain the earlier local fixes and
acceptance evidence in [STATION_TERRAIN_2026-10-02.md](STATION_TERRAIN_2026-10-02.md).

No entire PR is rejected. The unvalidated feature channels remain disabled;
always-on road digest allocations/hashing and a recorder bounded only by steps
are not retained. Ordinary road-entry ordering remains on.

Validation: focused protocol, hook and hookcore tests passed, including real Lua
fixtures, recorder limits, unreadable-balance rejection, wire golden bytes and
unique hook-site resolution against the installed build-40408 executable.
Workspace formatting, Clippy with warnings denied, full workspace tests and
game-helper script tests passed before push. GitHub's combined branch CI is
required before merge. No fresh game soak was run for this review;
PR #58/#59's reported runs are supporting evidence, not a claim that every cause
of desync has been eliminated. The gated features still need native two-game
acceptance before enabling them.

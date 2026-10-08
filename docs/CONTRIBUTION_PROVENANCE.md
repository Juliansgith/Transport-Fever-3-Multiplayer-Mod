# Contribution provenance

This page records what public GitHub and repository history establish about
selected contributions. It supplements the general credits in
[README.md](../README.md) and the release notes. It does not change commit
authors or replace the records on the linked pull requests.

## How to read attribution

A pull request opener, the person who requested or proposed a change, the
authors of implementation commits, reviewers, and sources of prior art can be
different people. The entries below keep those roles separate.

Commit authors and Co-authored-by trailers are taken from GitHub's commit
metadata. They establish the names and accounts recorded for each commit; they
do not establish how work was divided line by line. A PR opener is not
automatically an implementation author. A request is attributed only when a
linked message or project document says who made it. Prior art is not described
as copied code unless the source says that. Where the record does not state a
role, this page leaves it unstated.

## Audited pull requests

### Save compression: #108, #128 and #130

- [#108](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/108)
  was opened by **silver2127** and closed without merging after #128 superseded
  it. Its implementation commit
  [b38b94f](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/b38b94f258c94ba78c5d607e9478532ddff5e88b)
  records Julian Cooper (Juliansgith) and Claude Opus 5.5 (claude) as
  co-authors. The commit and PR description explicitly cite
  [silver2127's TPF2 Big Maps](https://github.com/silver2127/tpf2-bigmap)
  save_fast as prior art. That citation credits the earlier TPF2 work; it does
  not attribute the TPF3 implementation commit to silver2127.
- [#128](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/128)
  was opened by Julian Cooper and merged. Its first commit,
  [24d18cb](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/24d18cbeb179b2e56c63d3224b704486da151042),
  carries the save-fast implementation and records Julian Cooper and Claude
  Opus 5.5 as co-authors. The follow-up commits
  [fec159a](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/fec159aa6e92198783e05b96904bf224194b23fd),
  [5142f56](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/5142f56704fa0184bbd4b0a1525573f20aed3aa4),
  [efd8471](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/efd8471706eb8872c4c97fa43c40130560912e8b)
  and
  [f1a679c](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/f1a679cd4114e15ec2e218cad2799dc0ed4ecdd6)
  record Julian as author. They make the unmeasured optimization opt-in,
  reject unrecognized settings, and fix cross-architecture retention and
  linting. The PR description says it carries the save-fast work from #108;
  an [attribution comment on #128](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/128#issuecomment-6070615927)
  points back to #108's explicit TPF2 prior-art citation.
- [#130](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/130)
  adds the selected-native-profile gate so build 40420 does not compile or
  enable unverified save-fast sites. Its commit
  [09e3afa](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/09e3afa0792780c4571e00db7b3ceed3c7ba9481)
  records Julian as author.

### Regional servers and native mods: #116 and #117

- [#116](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/116)
  was opened by **silver2127**. Its initial commits
  [f79f3ef](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/f79f3ef4ebcfd3865cef4633f7ec26d24b3e8835)
  and
  [052365f](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/052365f483e9ac4d3aa8a2b7902b217dfe7e36c7)
  record Julian Cooper and Claude Opus 5.5 as co-authors. The later commits
  [c40f3da](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/c40f3da375bace235e03bec7f81f890992aafb6a),
  [a2b9366](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/a2b93668a34c8204b1f3949b649df837aa554bcf),
  [4f61e77](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/4f61e77ff326d100e38f64d7d5aa91949ccd1ef8),
  [c9b5cfb](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/c9b5cfbc68c69d13a3ab67f5892fad5f341b0bd4),
  [faabf70](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/faabf700527f4f336511e146ff51f8d21531f62f)
  and
  [1e7778d](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/1e7778d596813da4acabb4d32822826beb82f5b1)
  record Julian as author. The first commit explicitly says the proposal was
  asked for by silver2127, who runs a second server. This attributes the
  request to silver2127 and the implementation commits to their recorded
  authors. A [comment on #116](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/116#issuecomment-6070614641)
  repeats the distinction because its PR body did not spell out the request
  origin.
- [#117](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/117)
  was also opened by **silver2127**. The two main commits
  [eb99e81](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/eb99e8131f1fd3d6c9e95a7c8d8458d5edbde900)
  and
  [ca3a538](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/ca3a538a925fdfe5ea8649402265e9897a583e9b)
  record Julian Cooper and Claude Opus 5.5 as co-authors. Its remaining
  commits,
  [3ebc8f0](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/3ebc8f0da033ba580e1e472d811ceda879055984),
  [03f032e](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/03f032eb0de3ee7e6735aca349b5056040bc1743),
  [6dc5b2c](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/6dc5b2c56ea10c3605ad8982b4024b46451270a3)
  and
  [94ec669](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/94ec6698bd5c884ec48a060001ec81329f14dd9d),
  record Julian as author. The PR and commit records reviewed here do not
  attribute implementation commits in #116 or #117 to **tearded**. A
  [comment on #117](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/117#issuecomment-6070614980)
  states the opener/implementation distinction and leaves design origin
  unstated where the records do not identify it.

### Auto Signals: #122

[#122](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/122)
was opened by **tearded**; the commit author name is Max. The initial feature,
test, and in-game evidence commits
[cd3c1cc](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/cd3c1ccacf167741a0b391c10495ce1b5d6ff359),
[686b5bc](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/686b5bcef1e4c66658f4193c01a6f000377cb01b),
[36bb5a3](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/36bb5a384131167b6c7812545b792b86331edd39)
and
[df3ebe8](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/df3ebe867f5bf5025e7260a99e7e66dc050759f6)
record Max (tearded) and Claude Opus 5.5 as co-authors. They carry signal
tool settings into the room, add the PlaceSignals action and strict replay
tests, handle replacement on tracks with existing signals, and enable the
feature after the two-game test. Two upstream-integration commits also carry
the same co-author metadata:
[20360de](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/20360def21832ae624991c9543980e4e2e889abe)
and
[9872893](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/9872893320a4d1d67ea4e052fddb87f975554541).
The project's D27 record separately documents the request to make Auto Signals
work in rooms and the approved narrow signal-only exception. The final safety
and integration commits
[480ed27](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/480ed27e552e9c1ecc48859424f80bb1099cba57),
[61ad023](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/61ad0231a460a1ea3550cf2974709e2af51fe862)
and
[21269d6](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/21269d6559ee52fdd2638d96483da510402026c6)
record Julian as author. Those commits refuse mixed track edits, test track
ownership, and integrate the approved signal gate with the subsidy gate. A
[comment on #122](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/122#issuecomment-6070615399)
makes this split between the initial feature/evidence commits and the later
safety/integration commits explicit.

### Game build 40420: #129

[#129](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/pull/129)
was opened by Julian Cooper. All seven commits in the PR record Julian as the
author:
[e10ccfc](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/e10ccfc8a06420fe1e330837c2102bdd40da9683),
[0037619](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/00376199c18c4d1b32a27581186fdfea00a7876c),
[2736733](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/273673304ea2f0c97b85d3543d24d538344bd326),
[36f53f6](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/36f53f655ec7b3152b91f8354a0ee7108e8a2a11),
[36e4b2a](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/36e4b2a48d4f2caebc9d611d0f811a3732a39d78),
[43842ca](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/43842ca8cd46c20feec7e0f66eb731cbae7d0780)
and
[8e96c13](https://github.com/Juliansgith/Transport-Fever-3-Multiplayer-Mod/commit/8e96c131b2c94a90ddf087460e58c764ee925691).
They add the 40420 native profile and its verification, preserve the in-game
lobby integration, update game-test helpers, and pin earlier native proofs to
build 40408. No other implementation author is listed in the PR's commit
metadata.

## Retroactive attribution notes

Targeted comments were added to merged PRs #116, #117, #122, and #128 to make
the request, PR-opener, implementation, and prior-art roles clear on GitHub.
PR #108 already explicitly credits silver2127's TPF2 work; #129 and #130
already have accurate Julian-only implementation commit records, so their
bodies and author metadata were left unchanged. No commit author, co-author
trailer, or shared history was changed.

## Workflow for future credits

- Start with the GitHub PR record and its complete commit list. Record the PR
  opener separately from each commit's author and co-authors.
- Use Co-authored-by trailers on collaborative commits and verify the
  resulting GitHub author list. Do not infer implementation authorship from a
  PR opener, reviewer, request, issue discussion, or release-note mention.
- Attribute a request, design proposal, review, or prior-art source only when a
  linked message, project document, or source file states that role. Cite the
  source and distinguish it from implementation.
- Keep names and handles as displayed in the cited record. If a preferred name
  or contribution is unclear, leave it unstated until verified.
- Review the generated GitHub release notes as a draft. Add concise credits for
  user-facing contributions to the release description and
  docs/RELEASE_NOTES_<version>.md, linking to this page when roles need more
  detail. The PR template records the same facts at submission time.
- If a merged PR needs a correction, add a factual note to its description or
  comments and update this page. Preserve the original commits; do not rewrite
  shared history to change attribution.

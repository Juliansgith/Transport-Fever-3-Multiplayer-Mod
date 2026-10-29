# Roads, tracks and the bulldozer across an industry (work in progress)

Status: **off** (`acceptance.lua`, `industryFields = false`). Everything below
is in the code but refused in a room until the last step works. Tested by
hand on Transport Fever 3 build 40425 (Steam, Windows), one player against a
local server, 2026-10-28 and 2026-10-29. No two-player acceptance was run.

## The problem players hit

Drawing a street or track across an industry (a farm's field, a forest, a
clay pit, a farm building, its entrance road) or bulldozing part of one was
refused in a room: `Not in multiplayer yet: a build that removes a
construction`. Single player does it: the game shows "1 field will be
removed" and builds.

## What the game's tool proposes

The refusal now names it (`engine.lua`, `constructionsOf`). For a road across
a livestock farm:

```
removes ::/industries/livestock_farm/livestock_farm.con (6 modules),
adds    ::/industries/livestock_farm/livestock_farm.con (3 modules)
```

TF3's industries are `::/industries/<name>/<name>.con`; a farm's fields are
its `field.module` modules. The tool **replaces the industry** with one of
fewer modules, and the road's street part may also rejoin the industry's
entrance road (`::/infrastructure/street/constructions/ind_entrance_*`), an
edge the industry owns.

## What was tried, in order, and what the game did

1. **Carry the road alone** (`townBuildingsOnly` lets an industry through;
   the replay asks `gatherFields`). Every game's `makeProposalData` threw
   `Unknown exception` (`apply.lua`, `refusal`): the industry and its
   entrance cannot be changed by a plain street proposal.
2. **Carry the industry's replacement with the road as its connection**
   (`capture.lua`, `industryEdit` → `capture.construction`). Capture works.
   Three fixes were needed on the way, all kept:
   - an edit that removes nothing around it returned without its street
     part, and `joinedOnly` took a road joined to nothing for the
     industry's own: industries now keep every edge whose template is not
     a `/constructions/` street (`notConstructionStreet`);
   - junction settings at the industry's entrance were refused in every
     game ("the junction's road or track changed"): they are dropped for
     industry edits;
   - a bulldozer edit whose streets are all the industry's own is carried
     with no connection.
3. **Replay as a SimpleProposal** (`replaceConstruction`): the game warned
   `Collision` and kept the old industry (same entity id afterwards). Lost
   silently.
4. **Have the game regenerate the edit**
   (`api.engine.util.proposal.createProposalReplaceConstruction(old,
   params)`, sent with `makeWorldBuildProposalCmd(full, ctx, true, true)`),
   as the rail station path does. **This works: the field is removed in the
   game.** The industry stays no company's (`playerEntity = -1`, no
   `settleConstruction`).
   - Reading the industry back right after the command still found the old
     one: commands apply after the update that sends them. That check was
     removed.
5. **The road after it.** Sent in the same update, the game took the command
   and dropped it (the field still in the way). It is now **deferred to the
   next room step** (`apply.lua`, `deferred`, `apply.flushDeferred`; the
   sim script's `update` reports `deferred` and `postUpdate` flushes it
   before that step's actions). It must go through `networkInto(..., false)`:
   with `dangling = true` the station filter dropped a road joined to
   nothing (logged as `building -e -n -c`).

## Where it stands (the last run, 2026-10-29)

Three roads across one farm: each replacement applied, and each logged
`industry edit's road: N links`. Only **one** road was laid
(`industry edit's road laid a step later: taken`), about three seconds after
the last click rather than at the next step, and only one flush line was
logged for three deferred roads.

Suspects, not verified:

- The deferred list lives in `apply.lua`'s module state. If the sim
  script's `update` and `postUpdate` (or successive actions) do not share
  that instance, `hasDeferred` can miss entries. Check whether `update`
  runs in the same Lua state as `postUpdate`.
- `update` returns `work` only for some updates; check `deferred` is
  evaluated every update and that the flush runs at the next room step in
  every game (it must, or the games diverge).
- The deferred list is in memory only: a save or a world reload between the
  two steps drops the road.

A sturdier design: carry the road as its own action, ordered by the room
right after the industry edit (the capture hands two actions), instead of
deferring inside one action's replay. Or add the road's edges to the
regenerated native proposal (`full.proposal`) so both go in one command.

## Also still refused

- A construction (a station, a depot) placed on an industry: `more than one
  construction at once` (`capture.construction`).

## Logs to read

`%LOCALAPPDATA%\TPF3-MP\hook.log`: `replacing industry`, `industry edit's
road`, `laid a step later`, `not applied`.

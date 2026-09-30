# Prospecting in Transport Fever 3 (build 40408)

How the game's industry prospecting works, read from the game's own scripts
and binary without starting the game, for carrying it through a TPF3-MP
room (docs/PLAN.md Part 3, "prospecting (the industry at the same place,
with the same ID, on every game)").

Sources:

- the game's scripts, read out of `base/content/*.zip` with
  `tools/dayone/dayone.py`'s `archive_scripts`: `game_mechanics.zip`
  (`game_mechanics/company/...`) and `gui.zip` (`gui/construction/...`).
  Paths below are inside those archives, line numbers as extracted;
- the API reference shipped with the game, `api/tealdef/api/engine/util.d.tl`
  and `api/tealdef/api/cmd.d.tl`;
- `TransportFever3.exe` (SHA-256 `de1daad3...`), indexed with
  `tools/tpfre` (`tpfre index`, then `tpfre q ... str/xrefs/func/dis`).
  Addresses are RVAs.

Each fact is marked **seen** (read in code) or **INFERRED**.

## 1. How the player starts one

- **seen** The construction menu's "Prospections" category lists one entry
  per `company_exploration` resource
  (`game_mechanics/company/explorations/exploration_*.res.lua`: coal, clay,
  crude oil, fish, grain, iron ore, logs, rubber, sand, stone, vegetables,
  wool), built by `construction_react_util.getPerksProspectionDefinitions`
  (`gui/construction/construction_react_util.tl:1552-1637`). Each entry's
  action is `ACTION_PROSPECTION`, a selector on towns
  (`filter = componentFilter(TOWN)`, `:1549`), and carries
  `{ cargoType, types }`: `types` are the economy's industry type tags
  (keys of `economy.placementParams`) whose tags include the exploration's
  `requiredTags` (e.g. `OUTPUT_COAL`: `coal_mine`,
  `economy/placementparamsutil.lua:101-104`), collected with `pairs()`
  (`:1569-1595`), so **their order is not guaranteed to be the same in two
  games** (INFERRED: Lua's `pairs` order over string keys is a matter of
  the hash table's history).
- **seen** Clicking a town (`ProspectionActionRecipe.onSelect`,
  `construction_react_util.tl:1517-1547`), in the GUI:
  1. returns if the company already prospects for that cargo near that
     town (`externalGetCompanyState(getPlayer()).pendingProspections`);
  2. fires the GUI script event `company.lockPermits`; the company
     script's `guiHandleEvent` (`company/company.script.tl:519-552`)
     answers an error if no permit is left (rank too low, all used) and
     otherwise reserves one in its GUI state;
  3. sends **one command**:
     `api.cmd.makeScriptingSendEventCmd("", "Companies", "spawnIndustry",
     { companyEntity = getPlayer(), townEntity = <town>, types = <types>,
     permitKey = companyMeta.permitKey, cargoType = <cargo> })`
     (`:1539-1547`), whose callback fires `company.unlockPermits`.
- **seen** The cost is **one permit**, no money: the permit key is the
  exploration's `permitKey` or its resource name (`:1601`); how many a
  company has is `rankAndPermits` (coal: rank 6 gives 1,
  `exploration_coal.res.lua`) plus extra permits
  (`company_util.getMaximumNumberOfPermits`, `company/company_util.tl:68-81`).
  Exploration keys are not `company_permit_key` resources, so they have no
  cooldown (`company_static_util.tl:56-70`, `company.script.tl:66-94`).
  The industry itself is built for free (a build proposal with no context,
  below).
- **seen** The command is a script event, so in the game script's state it
  reaches the company script's `handleEvent` (`company.script.tl:425-470`),
  subscribed to `spawnIndustry` at `initNewGame`/`handleLegacy` (`:382`).
  In one game only when sent from that game's GUI: it is a GUI command,
  run "in the next simulation step" of that game.

## 2. What happens over time

- **seen** `handleEvent("Companies", "spawnIndustry")`
  (`company.script.tl:425-470`): ignores it if a prospection for that town
  and cargo is pending; else appends
  `{ townEntity, types, permitKey, initiatedTimestamp = GameTime.gameTime,
  attempts = 0, cargoType }` to the company's `pendingProspections`,
  consumes the permit (a timestamp in `consumedPermits[permitKey]`,
  `:17-39`), saves the state and sends the event `("Company",
  "startProspection", { entity, initiatedTimestamp, cargoType })` (`:466`),
  on which `explorations/exploration_plane.script.tl:224-229` spawns the
  prospecting plane (a custom vehicle circling the town; its course draws
  `math.random`, `:52, :66, :84`).
- **seen** The company script's `update` (`:231-335`) looks at the player's
  company once every 120 updates (`updateCount % (30 * 4) == 0`, `:279`).
  A pending prospection older than `prospectionTimeMs` (6 default months,
  `company_util.tl:14`) gets its final attempt; one older than
  `(attempts + 1)` months gets an intermediate attempt (`:299-307`).
  `postUpdate` (`:337-368`) runs them.
- **seen** An attempt (`executePendingProspection`, `:96-215`):
  1. `math.randomseed(GameTime.gameTime * 1000 + explorationIndex)` (`:137`),
     the index being the prospection's place in the pending list;
  2. `math.random() > attemptChance` fails the attempt; the chance grows
     from 0.2 to 0.6 over the six months (`company_util.tl:16-17`,
     `company.script.tl:130`);
  3. the industry types are shuffled by one `math.random()` each, in the
     order `types` lists them (`:185-197`);
  4. for each type in turn,
     `api.engine.util.proposal.makeIndustrySpawnProposal(type, town)`
     (`:200-206`) until one gives a proposal.
  A failed final attempt gives the permit back (`:106-110`) and sends a
  notification and `("Company", "endProspection", { success = false })`.
- **seen** So the script's own randomness is a function of the game time,
  the pending list's order and the `types` order alone: it reseeds itself,
  and the hook's per-step reseed (`crates/tpf3mp-hook/src/seeds.rs`) is
  neither needed nor in the way.
- **seen (binary)** `makeIndustrySpawnProposal`
  (`util.d.tl:926-930`: "Tries to generate a proposal for randomly placing
  an industry", in the town's Voronoi cell): the name is registered at
  `0x2538c74` in `sub_2536fc0`. The scripting helper `sub_25124e0`
  (`game/scripting/util_interface.cpp`, inferred file) reads
  `GameTime+0x40` (`sub_2a9680`, at `0x2512589`) and folds its four bytes
  with FNV-1a 64 (offset basis `0xcbf29ce484222325`, prime
  `0x100000001b3`, `0x2512592..0x25125d1`), then calls `sub_8f3590` ->
  `sub_8f1ca0` (`industry_util.cpp`, `"GetNewIndustryToAdd"`), which again
  seeds from `GameTime+0x40` the same way (`0x8f20c9..0x8f210c`) and
  passes the seed (`r9d`) on to the placement (`sub_3bb8c0`, then the
  sampler `sub_3b9450`, "Found {} sample location ... for the placement of
  {} industries"). No `rand()`, clock or `random_device` on that path.
  - INFERRED that `sub_25124e0` is `makeIndustrySpawnProposal`'s body: it
    is the only scripting-side caller of `GetNewIndustryToAdd` (the other
    is the map generator's `SpawnIndustries`, `sub_158cb0`), reached from
    `sub_24e7770`; the std::function binding between the name and it was
    not followed through.
  - INFERRED that `GameTime+0x40` is lockstep state: it is an int of the
    `GameTime` component, which the simulation keeps (the survey,
    `TPF3_RNG_2026-09-29.md` 1.3, lists `+0x38/+0x3c` as the likely
    `tickCount`/`updateCount`; `+0x40` is read by
    `SimEntityAtTerminalSystem::Update` too).
  So where it puts the industry, and the construction's own `seed`, are a
  function of the world and the game time: the same in every game that
  runs the same world at the same update.

## 3. How the industry is built

- **seen** The proposal is sent from the company script's `postUpdate`, in
  the game script's (engine) state:
  `api.cmd.makeWorldBuildProposalCmd(proposal, nil, false, false)`
  (`company.script.tl:212`): no context (free), `ignoreErrors` false,
  `playerInitiated` **false**. So TPF3-MP's build stop, which answers false
  only for player-initiated builds (`crates/tpf3mp-hook/src/builds.rs`),
  lets it through, and the GUI guard (`guard.lua`) never sees it: it wraps
  the GUI state's `api.cmd` only.
- **seen** Its callback (`:146-183`) sends the notification with
  `resultEntity = resultEntities[1][1]` and
  `("Company", "endProspection", { entity, initiatedTimestamp, cargoType,
  success })`; `exploration_plane.script.tl:230-245` then removes the plane.
- **seen** The construction's parameters, `seed` included, are what
  `makeIndustrySpawnProposal` put in the proposal: nothing of the GUI's.

## 4. What this means for a room

- The only GUI-side input is the `spawnIndustry` event. Everything after it
  runs in the simulation, in the game script state, from the company
  script's saved state, the world and the game time. If every game applies
  that event at the same update with the same parameters, every game keeps
  the same pending prospection with the same `initiatedTimestamp`, attempts
  it at the same updates with the same draws, and builds the same industry
  at the same place with the same seed (INFERRED from sections 2 and 3; to
  confirm with two games, see docs/HOOKS.md "Prospecting").
- The `types` list must travel in the originator's order (section 1: its
  order comes from `pairs()`, and the shuffle draws in that order).
- The permit check is the GUI's alone (`company.lockPermits`); the
  `spawnIndustry` handler consumes a permit without checking. Two players
  prospecting in the same moment could both pass their GUI's check and
  overspend by one. Every game does the same, so the worlds stay alike.
- Nothing else of the mechanic is refused by the room today: the company
  script's own commands (the notification, `startProspection`,
  `endProspection`, the plane, the industry's build) are sent from the
  engine state, where the guard is not.

## 5. Related company events (not carried)

The same channel, `makeScriptingSendEventCmd("", "Companies", ...)` from
the GUI, carries (seen):

- `applyLevel` `{ level }` (`company/company.tl:422`, handled by
  `company_growth.script.tl:152-157`): taking the next company rank in the
  company window. Prospecting permits come with ranks.
- `MakeGreen` `{ companyEntity, constructionEntity, permitKey }`
  (`gui/construction/tools/industry_greenify_tool.script.tl:66`,
  `company.script.tl:472-502`).
- `startMarketingCampaign` `{ companyEntity, townEntity, permitKey,
  marketingParams }` (`marketing_campaign_tool.script.tl:91`,
  `company.script.tl:406-424`).

Headquarters are constructions with company metadata (`headquarters`,
`company_metadata.d.tl`), placed and upgraded with the construction tool
(INFERRED from `construction_react_util.tl:944, 1104`), not events.

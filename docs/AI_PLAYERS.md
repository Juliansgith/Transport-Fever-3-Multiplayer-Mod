# AI players

Companies a computer plays. The room's owner adds one in the lobby, Easy,
Normal or Hard, and it plays the room's game as a player would: it founds
its company, builds streets, track, stations, harbours and airports, buys
buses, trucks, trains, ships and planes, runs lines and keeps tuning them,
borrows and pays back. It does all of it through the actions a player's
tools send, ordered by the server and applied by every game.

Decided in D32 ([DECISIONS.md](DECISIONS.md)). What the game offers and
what was measured is in
[investigation/AI_PLAYERS_2026-10-29.md](../investigation/AI_PLAYERS_2026-10-29.md);
the work is Part 5 of [PLAN.md](PLAN.md).

**Status (2026-10-29): seats, drivers and town bus lines built.** The
first milestone is in: AI seats in the server and its log (protocol 24,
log format 11), the driver and its hand-over with the memo, the mod's
world reads and verdicts (bridge 31, `tpf3mp/ai.lua`), the AI rules every
game applies, adding and removing AI players in both Multiplayer windows,
and the `tpf3mp-ai` mind. Of the second milestone, the AI borrows to
start, and builds and runs bus lines in towns at all three difficulties: in a
two-game rig room on the big fixture (2026-10-29), a Hard, a Normal and an
Easy AI opened seven bus lines in seven minutes, each founded, borrowed
for, stopped, given a depot joined to the street and bought buses for,
with no action refused and no divergence between the games. Its trucks
carry an industry's cargo to a town that needs it: fuel and meat, in the
same rig room (2026-10-30), alike in both games. Still open in the first
milestone: a verdict holds the driver's game a frame of 15 to 20 ms on
average, the game's own verdict most of it, over its 1.5 ms budget
(investigation, "What a verdict spends"). Track, water and air are still
to come; see "The work, in milestones".

## What players see

- **Adding one.** In the room (the main menu's Multiplayer window before
  the game, the in-game Multiplayer window during it), the room's owner
  picks **Add AI player** and a difficulty. The AI gets a company name of
  its own and a colour, and shows in the players list with an AI mark and
  its difficulty. A room has at most eight companies (D21); every AI is
  one of them, and AI players do not take a player's seat.
- **Its company is its own.** Players cannot join an AI's company. What
  it owns, D21 protects like anyone's. Its stations follow D22's station
  access: open to other companies' lines, as a founded company's start.
- **It plays at a person's pace.** Its builds appear as a player's do, a
  few a minute at most, never faster than a quick player could click.
- **Removing one.** The owner removes an AI: it stops deciding. Its
  company keeps what it owns and its vehicles keep running (the
  simulation runs them in every game), as a player's company does when
  they leave. A new AI may take over a company left this way.
- **No cheats, at any difficulty.** An AI has the money, prices, rules
  and information a player has. Difficulty is how well and how fast it
  plays, never a bonus.

## How it works

```
 the server                  the driver's agent              the driver's game
 ───────────                 ──────────────────              ─────────────────
 AI seat (member,            tpf3mp-ai: world model,  ◄──── world reader (mod, HUD
 no connection)              planners, executor              state, sliced per frame)
 orders its actions  ◄────── AiIntent(seat, action)  ────► verdicts (apply.proposalOf
 keeps its memo      ◄─────► memo                            + makeProposalData)
       │                                             ◄──── outcomes (applied, refused)
       └──► every game applies the AI's actions as the AI company's
```

### Seats

The server keeps an AI as a member of the room without a connection: an
**AI seat** with a player id of its own (derived from the room and the
seat, so it is nobody's key and nobody can sign in as it), a name, a
difficulty and a memo. It is always ready, it never paces the room, it
has no game, so it reports no checkpoints or saves, and it is written in
the room's log, so a restarted server keeps it.

### The driver

An AI looks at the world through one member's game: its **driver**. The
server picks the room's owner, or else the member who joined earliest
whose game plays the room's world and keeps up with the room, and tells
that member's agent which seats it drives.

- The driver's agent sends the AI's actions as `AiIntent(seat, action)`.
  The server checks that the sender drives that seat, the seat's own rate
  limit and the room's rules (`GameActions`), and orders the action as the
  seat's intent. Every game applies it as that player's action, so the
  mod books it to the AI's company (`companies.lua`), as it would a
  player's.
- When the driver leaves, falls behind or loses its game, the server hands
  its seats to the next member with each seat's memo. With no member to
  drive it, the AI rests: its company runs on; it decides nothing new.
- **Not a host** (D2, D4). The driver's game is not the room's truth. It
  is where the AI looks, as a player's screen is where a player looks.
  The server orders every AI action like anyone's, and every game applies
  it. A modified client could make the AI it drives play badly, or well,
  as it could its own company; in `native` rooms games already run their
  own economies (D6); canonical rules will validate an AI's intents like
  a player's.

### The brain

A Rust crate, `tpf3mp-ai` (D1), in the driver's agent process, so its
thinking never takes a game frame. It is a library with no game in it,
tested on its own and with the regression model (below).

- **The world model**: towns and their districts, industries and their
  stocks, the cargo chain, stations and catchments, lines and their loads,
  vehicles, the street and track graph, a coarse terrain (32 m heights
  and water), companies and their money. Built once, then kept current by
  what changed.
- **The memo**: the AI's projects in progress, cooldowns and random seed.
  The server keeps it per seat (at most 256 KiB, written with the room's
  log), so a new driver carries on where the last one stopped.
- **Planners and an executor** (below).

### What the driver's game does

The mod, in the HUD's Lua state (`tpf3mp/ai.lua`), answers the questions
of the AIs its game drives, one after another while 1.5 ms of the frame
lasts, and at least one a frame:

- **Reads the world** the AI asks about: its company (money, the year,
  its loan offers from the game script's roster, its stations, its
  depots and whether each reaches the street, its lines and vehicles),
  the towns and their districts' centres, the industries (each one's own
  station, what it takes and made over the last year), the streets near
  a place (each
  edge's ends and curve middle, template, owner, objects), whether trucks
  can drive between two places, the vehicle models of a transport mode
  (of a cargo: those that carry it).
- **Answers verdicts.** For an action the AI considers, it makes the
  proposal as the AI's company would build it (`apply.proposalOf`, as
  build previews do) and returns the game's verdict (`makeProposalData`:
  refused or not and why, the cost, what it would hit). A verdict can
  take tens of milliseconds for a long track, so the AI asks for short
  pieces, a few a second at most.
- **No outcomes of their own.** The AI confirms each action by asking
  about its company again until it sees what it built, never by trusting
  that it happened, and gives the step up after a few reads.

These are the bridge messages `ToHook::AiAsk` and `ToAgent::AiAnswer`
(bridge 31) and the protocol messages `AiDrive`, `AiIntent` and the memo
(protocol 24). Once a minute while it answers, the driver's hook.log says
what the answers cost its frames: by kind of question, how many, the mean
and the slowest, and its slowest frame of them (`ai: answers in 60 s:
...`).

## How the AI plays

Four layers, each run on the game's calendar so a faster room makes a
faster AI.

1. **Strategy** (each think). Its money: cash, loans, the reserve its
   difficulty keeps. Its opportunities, ranked by the return each would
   bring for its cost:
   - cargo, from producers to whoever needs it, along the game's cargo
     chain (`getEconomyCargoDependencies`; industries' production and
     capacity; towns' cargo needs per district), chains of several steps
     included (raw goods to a plant, its product to a town). *Built so
     far (2026-10-30):* chains of two steps (`tpf3mp-ai` chain.rs). Where
     a town needs a cargo whose nearest maker in reach makes too little
     of it (under 30 a year), it carries the maker's cargo to the town
     first, since a maker takes its inputs only while what it makes is
     carried on, then feeds the maker each of its inputs from the nearest
     industry in reach that makes enough of it, unloading at the maker's
     own station; each leg by train where it runs 1.8 km or more and its
     money allows, else by truck. A leg that fails ends its chain. It
     takes chains in turn with single lines, with at least 1.5 million
     to spend;
   - passengers, within a town (its districts) and between towns (their
     sizes, distance and how they are reached already);
   - with a transport mode chosen by distance, volume, terrain and water:
     streets for short and small, track for long and heavy, water where
     both ends touch the same water, air for long distance between large
     towns;
   - and who already serves a flow, how well (line capacity used, cargo
     waiting, delivery time and quality) and at what price.
2. **Projects.** A chosen opportunity becomes a project: its actions in
   order, from stations and depots to the route, signals, the line and its
   vehicles. Each piece is tested with a verdict before it is sent, sent
   one at a time, and confirmed by its outcome. A refused piece is
   planned again around what refused it; a project that keeps failing is
   given up within its budget and what it built is reused or removed.
3. **Operations** (about once a game month, more often at Hard). For each
   of its lines: load, cargo and passengers waiting, frequency, the game's
   own problems for it, and its profit. It adds and removes vehicles;
   replaces old ones with the best model of the year; widens streets at
   its busy stops; doubles a busy single track or adds passing loops;
   extends lines that would earn more; and closes lines that lose money.
4. **Money.** It borrows when a project's return beats the loan's interest
   by its difficulty's margin, and pays back when it can.

### Building, mode by mode

- **Streets and road vehicles.** Bus stops on a town's own streets near
  its district centres, lines through them; bus and truck lines between
  towns and to industries: stops or truck stations within reach of an
  industry's stock, depots, and new street where none joins them, found
  on the street graph first (`findPath`) and over the terrain only for
  the missing part. A road depot faces a street, its entrance 2 m short
  of a junction its build splits into the street's curve, as the
  construction tool leaves one; every game's refresh of the depot then
  snaps the entrance onto it, in the same action (`tpf3mp/apply.lua`,
  `BuildConstruction`). A separate road onto a construction's entrance
  the game refuses ("Construction Not Possible", 2026-10-29). *Built so
  far:* a town's bus line, and a cargo's truck line from an industry that
  made it over the last year to the nearest town that needs more of it
  (the town window's cargo demand: supplied at least 10 a year short of
  what it may take), within the difficulty's reach and joined by road
  already (`findPathNodeToNode` for trucks). It loads at the industry's
  own station, as players' truck lines do (build 40425: every industry
  has one, its terminals alike, each for cargo and passengers), and
  unloads at a one-platform truck station of its own within 140 m of the
  centre of the district whose buildings take the cargo. Its stops are set
  as the game's line window sets a road stop's: passengers loaded too, a
  road stop's waits, and every cargo type in its lists (the mod fills
  out the types an action leaves off). Towns, not industries: an industry
  takes its inputs only while what it makes is carried on, and trucks to
  refineries, a cement plant and a livestock farm no one carried for, or
  little, loaded nothing (investigation, "Trucks").
  The two kinds of line take turns.
- **Track and trains.** The station: a through or terminus station near
  the town's centre or the industry, turned to the route, its platforms
  as long as its trains, tested with its verdict. The route: a search over
  the 32 m terrain with costs for gradient, tight curves, water (bridges),
  buildings (it keeps out of towns where it can), other companies' track
  and streets (crossed by bridge or level crossing, never built through,
  D21), then smoothed into arcs of at least its minimum radius
  (`findDubinsPath`) within BUILDING.md's tolerances, and sent as
  `BuildTrack` actions a few hundred metres each. Signals: blocks on
  double track, passing loops on single track. Depots, then trains chosen
  by what they carry for what they cost in that year. *Built so far
  (2026-10-30):* a cargo carried to a town that needs it by train where
  the two lie at least 1.8 km apart and the AI has 6 million to spend:
  trains come first then. It reads the land around both ends (the
  driver's game answers a grid at a time: heights, water, built land,
  streets and tracks), puts a one-track cargo station of three modules by
  each, its building towards the industry or the district within its
  catchment's 150 m, on free ground the land shows, its track's end
  towards the other station. It plans by what the game charges, as a
  player reads the build tool's price (`crates/tpf3mp-ai/src/costs.rs`,
  from the game's own verdicts on test pieces; investigation, "Track
  costs"): track 53 a metre on the ground, earth for embankments and
  cuttings by the cubic metre, stone bridges dearer the higher they
  stand, tunnels dearer the deeper, a level crossing about 9,600, a
  station's module 93,000, a rail depot about a million. Before it builds
  anything it prices the whole line between the cheapest of its first
  station placements, and a line it cannot pay for, or with no route, it
  never begins; its train is then the strongest engine the rest of its
  money buys. Then it lays the whole line out, still before building
  anything. It routes the line over a 40 m grid in 16 headings, turning
  22.5° at most a step and never on two steps running, off built land and
  other tracks, for the least money: going round a hill or a pond where
  that is cheaper than the earth, bridge or tunnel the straight way
  needs, keeping to the level where it can, crossing streets and never
  following them. It smooths the route into edges of about 100 m whose
  headings run on through every node, asks the game for the streets it
  crosses, and chooses every node's height for the least cost of the
  whole line, within a 2.5 % grade where it can and never steeper than
  4 % (a plant on a hill above its town): an embankment or a cutting, a
  bridge where the ground falls away (over water always; the game builds
  one half a metre up), a tunnel where it rises at least 9 m over it, as
  the prices say. Where it meets a street it may cross on the ground it
  lays a level crossing as the track tool does: its vertex splits the
  street's edge at the street's height, the track straight through it
  and level either side (within 1 %; the game refuses one on a grade,
  "Too Much Incline"), at 35° or more, on a street no steeper than 4 %,
  clear of the street's ends, of other crossings and of its stations,
  never on a street with stops, a company's street or one by a junction
  a company's edge names. Any other street it passes over, 7.5 m clear,
  or under, 10 m down, whichever costs less; a node of its line within
  20 m of a street takes the crossing's place, so no edge is too short
  for its grade. Before 1970, for a train no faster than 65 km/h, its
  bridges are trestles, cheaper and as fast as its train. Then it puts
  every piece of the planned track (about six edges each) to the game's
  verdict, still before building anything, its joins as new nodes; a
  piece the game would refuse it routes round and lays the line out
  again, and a line it cannot lay out between one pair of placements it
  lays out between others, up to six pairs, before giving up with
  nothing built. Only then does it build its stations and, if they stand
  where it planned them, lay the track as planned, a piece at a time,
  each put to the verdict again (one through something in its way,
  "Collision", is refused though the game would build it); it keeps its
  prices in line with the game's by the verdicts' own costs. Then a rail
  depot beyond the first station, joined to its outer end, the line, and
  its train with as many wagons as the platforms take (six at most): no
  electric engine, as its track has no catenary. Its planning runs in
  the driver's agent, a route search 5 to 25 ms on the big fixture and at
  most about 0.2 s where there is no way through. Still to come: passing
  loops and signals, more trains a line, sharing a depot between lines,
  passengers by train.
- **Water.** Harbours where both ends touch the same water (from the
  water grid), with their street links. Ships need no route built.
- **Air.** Airfields and airports near large towns on flat enough ground,
  their street links, planes at their hangars, only for long distances.
- **Transport Fever 3's own.** Cargo trams on tram lanes and helicopters
  come once the modes above work.

Gradients, curve radii and platform lengths come from the game's
templates and are checked by verdicts; the AI keeps no copy of rules a
game patch could change. It learns what a line earns from the lines'
own income in the game's finance tables, so its estimates follow the game.

## The three difficulties

| | Easy | Normal | Hard |
|---|---|---|---|
| plays like | a careful newcomer | a sound regional operator | a tycoon |
| modes | streets (bus, truck); simple trains later in the game | streets, track, water | everything: streets, track, water, air, cargo trams |
| networks | point to point, one line a route | double track with signals, passing loops, through stations | networks: hubs, transfers between lines and modes, feeder lines, multi-stop lines |
| opportunities weighed | the 5 nearest | the 20 best | every one, chains of several steps |
| projects at once | 1 | 2 | 4 |
| picks | among its best three, often not the best | the best, with a little noise | the best |
| thinks | every 2 game months | every game month | every game week |
| tunes lines | every 3 game months: adds a vehicle above 90 % load, removes one below 25 % | monthly, by load and cargo waiting | weekly, by profit per vehicle, frequency and delivery time |
| vehicles | replaced when worn out | replaced at the end of their life | replaced when a better model earns more |
| upgrades | none | streets at its stops, double track where busy | electrification, faster track, bigger stations, terraforming for a good site |
| money | no loans; keeps a 30 % reserve | borrows up to half its credit when a project returns 1.5 times the interest | borrows whenever a project beats the interest, pays back at once when not |
| other companies | leaves alone what another company serves | serves what is served badly | competes for the best flows on price, speed and delivery time, within the rules |
| extras | none | subsidies | subsidies, perks, its headquarters, prospecting |
| most actions | one per 20 s | one per 5 s | one per second |

Every difficulty plays by the rules a player plays by and sees only what a
player could see; a level never gets money, prices or knowledge a player
lacks.

## The work, in milestones

Part 5 of [PLAN.md](PLAN.md) lists these with their checkboxes.

1. **Foundations.** AI seats in the server, its log and its protocol;
   adding and removing them in both Multiplayer windows; the driver, its
   hand-over and the memo; `AiIntent`; the world reader, verdicts and
   outcomes in the mod and the bridge; `tpf3mp-ai` with its world model,
   an executor that sends one action and waits for its outcome, and a
   do-nothing AI that founds its company. Measure the verdict costs
   (investigation, "Open"). *Done when* an AI seat founds its company in
   a two-game rig room, survives its driver leaving, and costs the driver
   under 1.5 ms a frame on the big fixture.
2. **Streets, Easy.** Bus lines in towns, trucks for two-step cargo
   chains, depots, line tuning, vehicle replacement, money. *Done when*
   an Easy AI alone on a fixture runs at least five profitable lines
   after ten game years at 4x, with under 2 % of its actions refused and
   no divergence.
3. **Track.** Station sites, the terrain route, smoothing, `BuildTrack`
   pieces, signals, depots, trains; single track with passing loops, then
   double track. *Done when* a Normal AI builds and runs three
   profitable rail lines, one of them over hills or water, on two
   fixtures.
4. **Water and air.** Harbours and ships; airfields, airports and planes.
5. **Normal and Hard.** The difficulty profiles; networks with transfers;
   upgrades; competition; subsidies and perks; the ladder below.
6. **Release.** Owner playtests, tuning, and the player guide
   ([PLAYING.md](PLAYING.md)).

The AI uses only actions whose channels are enabled in a room (Part 3,
fail closed): it builds what players may build, and its milestones wait
for Part 3's lines, vehicles and stations where they need them.

## Testing

- **`tpf3mp-ai` on its own.** Planners on made-up terrain (a route
  around a hill, over a lake, between two towns), routes within the
  geometry tolerances, the choice between opportunities, the memo kept and
  restored, an executor that replans after a refusal.
- **The regression model** ([REGRESSION.md](REGRESSION.md)). The model
  gains towns that want passengers and industries that make and want
  cargo; AI scenarios (`ai-bus-town`, `ai-truck-chain`, `ai-driver-leaves`)
  play there in CI on all three platforms.
- **The real game.** The rig plays rooms of AIs alone on the fixtures at
  4x for 20 game years per difficulty: actions refused, divergence (none
  allowed), the driver's frame time, company value.
- **The ladder.** Easy, Normal and Hard play each other on ten seeds of
  two maps; Hard must beat Normal and Normal Easy in company value after
  25 game years in at least eight of the ten. Then the owner plays them.

## Risks

- **Builds the game refuses.** Every piece is tested with its verdict
  first, routes are sent in short pieces, and a refused piece is planned
  again from what the game said.
- **The driver's frame time.** Reads are sliced, verdicts limited to
  short pieces, one a frame and four a second at most, sharing one read
  of the street network a while; the thinking runs in the agent. The
  driver is chosen among members keeping up with the room. A verdict
  still holds a frame for the game's own `makeProposalData`, mostly 4 to
  18 ms on the big fixture, once 60 ms.
- **Game patches.** The AI checks on start that the game offers every
  function it uses, and rests, saying why, when one is missing (fail
  closed); limits come from the game's templates, not from copies.
- **Strength takes playing to tune.** The ladder runs unattended in the
  rig.
- **Players may feel crowded out.** Easy and Normal leave what others
  serve well alone; only Hard competes for it, and only by the rules.

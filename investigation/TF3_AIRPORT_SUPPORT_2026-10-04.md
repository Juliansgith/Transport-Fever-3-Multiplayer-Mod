# Airports in the room's game (Steam Windows build 40408)

## Intended player outcome

A player can build an airfield or airport with a usable terminal and hangar,
connect it to a road, buy an aircraft there, and run a line between airports.
The construction, module changes, aircraft, service, ownership and costs must
agree in both games. Removing or editing a company's airport must not change
another company's property. The game's own starting money, loans and prices
set what players can afford.

## Baseline: the stock tool is refused

In the owned two-player local room `target/game-runs/airport-baseline-1004`,
both games loaded the same 1990 fixture (`tpf3mp_fixture3`) on 2026-10-04.
The host selected the stock Air menu's airfield, with **Depot: Yes** and one
terminal. Its preview showed `Not in multiplayer yet: a build with a stop or
signal`; the larger airport showed the same refusal. The host's `p1/hook.log`
recorded the construction builder's proposal with 21 new nodes, 21 new edges,
and an added edge object (`category=2`), then the same refusal. No airport was
built. The fixture already includes a loan and bus infrastructure, so its
roughly $80 million balance is mechanics evidence only, not an ordinary-start
affordability result.

The installed game's `stations/air.zip` airfield script adds runway and
taxiway signal objects to the airfield's internally generated network.
`capture.connection` currently asks `engine.fromProposal` to parse the full
construction proposal before dropping that internal network; the generic
parser refuses every newly added stop or signal. The construction's file and
parameters regenerate its own network on replay. The proposed fix must omit
only objects proved to belong to that internal network; an object on an
external road or track still fails closed.

The stock `stations/air/airfield.con` unlocks in 1920, defaults to one
terminal and a hangar, and its hangar accepts small aircraft. The stock
`stations/air/airport.con` unlocks in 1950, defaults to two terminals and a
hangar, and its hangar accepts small and full-size aircraft. Their script
descriptors use a year-dependent price coefficient; the actual charged cost
must be observed in the game. The 1990 fixture's balance cannot establish
what a new company can afford.

The plain `tpf3mp_fixture` room (`airport-ordinary-baseline-1004`) loaded in
both games with a $0 balance and no infrastructure. The game's finance window
offered a $83 million loan; the host took it through the stock UI, and both
games then displayed $83 million. This establishes a normal financing action,
but does not yet prove two airports plus an aircraft fit within that loan.

## Native proposal shape found during the fix

The first signal filter assumed that `edgeObjectsToAdd[].resultEntity` was a
unique temporary object ID. The native retest still refused the airfield. A
failure-only diagnostic in `airport-diagnostic-1004` showed that its second
signal row repeated `resultEntity=-1`. A full, line-bounded dump in
`airport-fullshape-1004` showed nine signal rows, **all** with that same
placeholder. The corresponding nine `addedSegments[].comp.objects` pairs use
distinct carrier IDs `-400000000` through `-400000008`, all with edge-object
type 2. The airfield proposal has 21 added nodes and 21 added segments, all
with negative IDs and endpoints, and exposes no `frozenEdges` list. No
`model`, `param` or `edgeEntity` reads from the signal rows. The records must
therefore be checked as a whole against isolated, new airport edges; pairing
them by `resultEntity` is invalid on Steam build 40408. The fix must still
refuse an external stop/signal or an ambiguous carrier.

The revised filter was tested in `airport-retest-1004`. From the game's Air
menu, the host built two stock one-terminal airfields with hangars. Their
previews cost $6,315,980 and $5,652,780; the host handed both
`BuildConstruction` actions to the room, both games applied them, and their
created edge IDs matched. A DC-3 bought through the first hangar's normal
store cost $916,196. The line manager then accepted both airport terminals
on Line 2 and assigned that aircraft. The plane visibly taxied, took off,
flew and returned toward a runway. This establishes the basic mechanics in
a staged-funds room, but **does not establish clean acceptance**: the server
reported p2 diverged at step 2550 in network, construction, town and people
lanes. The verdict was delivered much later than the step it names. By wall
time, both games' step-2500 and step-2600 determinism probes matched before
any resync message; the plane purchase and second airfield build happened
after step 2600. The difference was therefore transient within the 50-step
window after the first airfield, not lasting world drift or caused by the
second airfield. Its exact cause remains unproven. The next run isolates
two-airfield placement before any aircraft purchase from the ordinary $0
start.

The follow-up `airport-ordinary-twoair-1004` did that from the plain
`tpf3mp_fixture`. The host took the normal $83 million loan, placed two stock
airfields and one large stock airport using the Air menu, and bought a DC-3
through both an airfield hangar and the large airport's hangar. It created a
two-airfield line and a mixed large-airport/airfield line, assigned the two
planes, and saw one airborne and the other taxiing. The large-airport hangar
offered small and large aircraft. The mixed line initially chose a helicopter
terminal, which the game's own line manager excluded for an airplane; using
its airplane terminal made the line available. Both games applied the room's
construction, vehicle, and line actions. Across 18,200 steps after the builds,
`tools/probe/compare_runs.py` found 180 common probe steps. Construction,
network, town, money, people and vehicle-count lanes matched in all 177
comparable samples (three startup samples had read errors). The vehicle-
position lane matched in 121 comparable samples but had 59 read errors,
beginning with the aircraft, so exact airplane-position agreement was not
measured over those steps. The rig reported no divergence or resync. This
supports the build/buy/line/flight path, but the airports were in open ground
and had no proven road access or passenger service.

That same ordinary run exposed an airport module blocker: the host selected
Configure on an owned airfield and previewed a valid blue second passenger
terminal for about $1.8 million, but its click was refused as `a build with a
removed stop or signal`. `airport-module-directshape-1004` reproduced the
same refusal and logged the complete stock proposal. The edit removed nine
positive signal object IDs, each paired with type 2 on one of the original
airfield's removed segments. It added nine placeholder signal rows and nine
reserved type-2 object IDs on new internal segments. The old airfield's
read-only `CONSTRUCTION.frozenEdges` contained 21 edges, including every one
of those nine removed-object carriers. The generic capture guard refused all
edge-object removals, even when the old and new signals were wholly internal
to the same stock airport. The needed exception can be bounded to an old
stock airport's frozen edges and its actual one-to-one signal pairs; external
objects must remain refused.

In that diagnostic room, p2 founded its own company, Rival, through the
native Multiplayer window. Its view of p1's airfield stayed visible but the
Configure button disappeared. This demonstrates the stock GUI's foreign
module-edit guard, not yet the server's foreign purchase or demolition guard.

The next owned two-game run, `airport-module-replay-1004`, exercised the
revised capture path through the actual Configure window. Adding a second
passenger terminal at a valid $1,808,289 preview handed a
`BuildConstruction` action to the room. Both hooks applied the replacement
for construction 6230 at the same step and logged the same newly generated
internal edge IDs 7770–7790. Three common post-edit determinism probes
matched, and the native Multiplayer window said the worlds matched. A later
read of the surviving construction's parameters found module slot 10001007
in addition to the stock one-terminal airfield's default hangar, main and
terminal slots 10001000, 10001002 and 10001004. The edit therefore persisted
through its subsequent refresh; the refresh returned the frozen edge IDs to
7608–7628, so those IDs alone cannot diagnose whether the module survived.
P2 then founded Rival through the native window. The foreign airfield still
had no Configure button, and its global bulldozer showed no selectable
target or submitted action. This is native GUI evidence for both guards.

P1's global bulldozer did select its **own** edited airfield but refused the
click with `removing a stop or signal`. A read-only call to the game's
`createProposalRemove(6230, Context(player=3869))` showed one construction,
21 removed segments matching the construction's 21 frozen edges exactly,
and nine scalar `edgeObjectsToRemove` IDs. Each ID appeared once as a type-2
signal on one of those frozen edges; no outside edge or object appeared.
That is the stock airport demolition shape the remaining bounded exception
must recognize. Its native replay and accounting still need a new staged
run after the fix.

The fix was tested in another ordinary-start local room,
`airport-combined-1004`, against the same Steam build. P1 started at $0,
took the stock $83 million loan, built a stock airfield, and added a second
passenger terminal through Configure at a $1,808,315 preview. Both hooks
replayed the airport replacement with the same new internal edge IDs.
Switching to the editor's bulldozer and clicking the hangar removed that
individual module through a second `BuildConstruction` replacement in both
games. The hangar disappeared while the runway and passenger terminals
remained. P1 then built a 319 m town-road extension and a 68 m segment
ending beside the airport building; both arrived as `BuildRoad` actions.
The stock UI refused a further road directly through the building as a
collision. P1 used the global bulldozer on the runway, confirmed its $0 /
-1% reputation preview, and handed `Bulldoze` action 8 to the room. Both
hooks logged removal of construction 7527. Both resulting screens show the
whole airport gone with the new roads still present, and both accounts read
$71,767,204 at the same date. Seventy common p1/p2 determinism probes
had already matched at the time of demolition, including six after it.
At room end `compare_runs.py` found 140 common steps (500–14400); all seven
lanes matched in 138 comparable samples each, with two startup read errors.
The rig reported no divergence or resync. The screenshots are in the shared
`target/game-runs/` directory as `airport-combined-demolished-p1.png` and
`airport-combined-demolished-p2.png`.

P1 rebuilt an airfield beside those roads in a blue valid preview. The
game's `catchmentAreaSystem.getStation2edgesMap()` then found 107 reachable
transport edges for its passenger station, including 27 outside its own
frozen airport edges and town-road entities 6017 and 5754. This establishes
native network reachability through the road. By contrast,
`getStationCatchables(station,false)` was zero at this distant site; it does
not prove passenger demand or a carried passenger. The prior ordinary run
proves aircraft lines and flying, but passenger fares remain unobserved.

Several P1 and P2 native screenshots show the game's red
`Missing builtin recipeId ... WithComponentParams` banner while an airport
is present, and it can persist after demolition. It did not prevent the
reported actions or matching world probes. Its cause was not established
here; the screenshots do not demonstrate a clean error-free UI.
The stack traces through `follow.lua:508` and the generic
`builder.proposalCreate`/`followInGui` path; both files are byte-identical to
`origin/dev`, so this change did not introduce that code path. Shared stdout
does not link the banner to one game process or compare a non-airport builder
proposal at the same time, so airport-specific causality remains unproven.

The original aircraft-position probe had a separate validation defect: a
parked DC-3 in an airfield hangar has `IN_DEPOT` state, no world position and
no `groupFileName` on its depot subconstruction. The corrected probe resolves
the parent stock airport construction through `CONSTRUCTION.depots`,
deduplicates the same `VEHICLE_DEPOT` subconstruction, and records an explicit
in-depot state with a stable parent construction file and depot ordinal. It
requires a real position for active aircraft and includes their flight state.
A read-only native console query found an active DC-3's position change from
`(-14.32, 341.40, 1.05)` to `(-1353.26, -123.83, 161.14)`. Four focused
probe tests and a bounded independent review verified the fix.

The follow-up owned two-game room `airport-probe-verified-1004` loaded the
saved flight world with that probe. P1 then bought a second DC-3, leaving one
active and one explicitly in its airfield depot. It built a stock large
airport and used its Configure editor to add a passenger terminal, taxiway,
and second runway, then remove the added terminal. Actions 3, 4, 7, 8 and 9
were handed to the room; both games applied the same construction/network
updates. `compare_runs.py` found 111 common actual samples over steps
12000–23400, identical in all seven lanes with zero lane read errors. Two
sample steps were skipped on each peer; a parser regression that counted the
timestamp-prefixed skip comments as samples has been corrected. This is
saved-world follow-up evidence for the large-airport mechanics; the earlier
ordinary-start run remains the evidence for affordability and initial
flight. The owner took over the two running games for whole-large-airport
demolition and the final visual checks, then reported "yeah all works" on
2026-10-04. This closes the remaining manual UI acceptance by owner report;
it does not add an independently captured demolition trace. The owner also
asked to skip further code review and subagents, so both were stopped.
The latest logs are at
`target/game-runs/airport-probe-verified-1004/p1/hook.log` and `p2/hook.log`.

## Acceptance to record after the change

- From an ordinary start, use the game's own tools and available funds or
  loans to build two reachable airports or airfields, including a hangar,
  terminal and road connection.
- Buy an aircraft through a hangar, create and edit a two-stop air line, assign
  the aircraft, and observe takeoff, arrival and service in both games.
- Add or remove an airport module, then remove an airport after clearing its
  dependent aircraft and line; compare balances, owners and objects. Module
  add/remove and empty-airport demolition are demonstrated above; demolition
  after an aircraft and line still needs a direct native test.
- Try another company's airport: its station access follows that company's
  policy, while its hangar, modules and demolition remain owner-only.
- Compare both hook logs and overlapping determinism probes after each phase;
  a Lua fixture alone does not establish native acceptance.

The baseline screenshots and logs stay under `target/game-runs` and are not
committed. Follow-up native results and any still refused airport variants
belong here before declaring this support complete.

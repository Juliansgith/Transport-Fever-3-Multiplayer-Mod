# Coverage: every way a player changes the world

Every way a player of Transport Fever 3 (Steam build 40408) changes the
world, and what TPF3-MP does with it in a room's game. Read from the game's
API declarations (`api/tealdef/api/cmd.d.tl`), the GUI's Teal sources in
`base/content/gui.zip` and `game_mechanics.zip`, and the mod and hook as
they are on `feat/combined`. Nothing here was run in the game; where a row
depends on something not yet seen in the game it says INFERRED.

Each way is one of:

- **carried**: captured in the player's game, ordered by the room as an
  `Action` (`crates/tpf3mp-proto/src/action.rs`), applied by every game at
  the same update;
- **refused**: stopped in the player's game with a reason, applied nowhere;
- **unknown**: nothing stops it, so it may act in the player's game alone.
  That is a hidden desync, and the top risk on this page.

## Summary

| | count | where |
|---|---|---|
| command factories (`api.cmd.make*Cmd`) | 61 | 17 carried (in part), 1 passed (speed), 43 refused |
| game-script events the GUI sends | 21 | 10 carried, 11 refused |
| native tools and windows (rows below) | 18 | 14 carried (in part), 4 refused |
| unknown paths | 2 open, 1 closed | see "The gates and their holes" |

The gaps, by risk, and what became of each ("Gaps" below):

1. **Unknown:** the GUI's second Lua state has no guard. *Closed* by
   batch 2 (the guard in every GUI state).
2. **Unknown:** native commands other than a build, queued without Lua.
   *Measured* by batch 3; refusing them needs an in-game probe first.
3. **Unknown, by design:** the console's Lua state.
4. Carry: vehicle rename and colour, station and town rename (batch 4).
   *Done* (`Rename`, `VehicleChange::Recolor`, schema 17).
5. Carry: line waypoints, rail and ship and aircraft (batch 5).
   *Done* (`LineStop::waypoints`, schema 18).
6. Carry: notification dismiss, keep and ignore (batch 6).
   *Done* (`Notification`, schema 19).
7. Carry: warehouse stock cargo and discarding cargo (batch 7).
   *Done* (`DiscardCargo`, schema 19; a slot's cargo is a window's edit,
   carried already).
8. Refused, needs an owner decision or a probe: the rest (the table at the
   end). These fail closed: refused with a reason, never acted locally.
9. Carry: which of a construction's depots a vehicle is bought at (an
   airport's or harbour's second hangar or ship depot). *Done:* a depot
   no street reaches is named by the construction that lists it, and by
   its index there (`BuyVehicle::depot_index`, schema 20; tests
   `a_ship_or_aircraft_is_bought_at_the_harbour_or_airport_that_lists_its_depot`,
   `every_game_buys_at_the_constructions_depot_the_store_bought_at`).

## The gates and their holes

A player's change reaches the world through one of four paths. Three have
a gate that carries or refuses every command (fail closed); what slips
past them is "unknown".

1. **The GUI's commands** (`api.cmd.sendCommand` in a GUI Lua state).
   `tpf3mp/guard.lua` wraps every `make*Cmd` factory of the state's
   `api.cmd` and `sendCommand`. In the room's game a kind in `guard.PASS`
   is sent, a kind `guard.CARRY` makes an action of goes to the room,
   everything else is refused, a command no wrapped factory made included
   (so `api.cmd.debug.*` is refused).
   - **Hole U1 (unknown):** the GUI runs in more than one Lua state on
     build 40408 (HOOKS.md, "Companies": the line manager's HUD is drawn
     in another). The guard was installed only in the Multiplayer plugin's
     state (`gui/tpf3mp/tpf3mp.script.lua`); the state the game renders
     its React recipes in (`gui/tpf3mp/gui_state.script.lua`) had none,
     so any command a window there sends acted locally. Which windows
     render there is not known (the stop tool is watched in both because
     the construction menu's state is not known). *Closed:* that state
     has the guard too (`tpf3mp/hudguard.lua`); a window there that waits
     on what its command made is refused, as the room's answers reach the
     plugin's state alone. Test:
     `in_the_huds_state_the_guard_carries_or_refuses_every_command`.
2. **The native build tools** (street, track, construction, stop and
   signal, bulldozer, road and track modifiers, module editor, terrain
   tools, terrain painter, asset brush, town builder, the crossing, lane
   and light tools). Each queues a `WorldBuildProposal` command
   (`CommandList::Add`); the hook's apply detour answers false for every
   player-initiated one in the room's game (`crates/tpf3mp-hook/src/builds.rs`),
   whatever tool made it. The mod carries the proposals of the tools in
   `tpf3mp_sim.script.lua`'s `CAPTURE` and the two the hook reads natively.
   - **Hole U2 (unknown):** a command of another kind than
     `WorldBuildProposal` that native code queues without Lua is not
     stopped. None is known on build 40408 (the GUI is Lua; every tool
     above builds through a proposal), but none is ruled out either.
     *Measured:* the hook logs every kind queued in the room's game, once
     per kind and call site (`cmdkinds.rs`); refusing what is neither a
     build nor Lua's waits on a playtest's log.
3. **Game scripts.** The game's own scripts and shared mods' run in every
   game alike. A personal mod's game script goes through
   `tpf3mp/modguard.lua` (carried, dropped or refused).
4. **The console** (its own Lua state, `api.cmd` unguarded).
   - **Hole U3 (unknown, by design):** a command typed in the console runs
     in that game alone. The tests use the console to hand actions to the
     room (`makeScriptingSendEventCmd("", "tpf3mp", "command", action)`).
     Whether to guard it is the owner's call.

## The command factories

Every factory in `api/tealdef/api/cmd.d.tl`, who sends it from the GUI,
and what the room does. Tests are in `crates/tpf3mp-proto/tests/lua_mod.rs`
unless named otherwise.

| factory | sent from the GUI by | in the room | test |
|---|---|---|---|
| `makeVehicleBuyCmd` | vehicle store (`vehicle_react_util.tl`) | carried: `BuyVehicle`, the depot by its construction's file and place, the consist part by part (groups and multiple units) | `a_bought_vehicle_goes_to_the_room_and_the_store_hears_which_it_is`, `the_game_script_buys_the_vehicle_and_tells_the_buyer_which`, `vehicles_bought_onto_a_line_in_a_burst_all_get_the_line` |
| `makeVehicleReplaceCmd` | vehicle window "modify", store "replace" | carried: `ReplaceVehicle`, kept parts by index | `a_group_replacement_goes_to_the_room_vehicle_by_vehicle_by_canonical_ids`, `every_game_replaces_the_vehicle_with_its_own_parts_kept_and_new_ones_bought` |
| `makeVehicleSellCmd` | vehicle window, line manager | carried: `SellVehicle` | `in_the_rooms_game_the_gui_refuses_what_the_room_cannot_carry` (refusal path); apply covered with the registry tests |
| `makeVehicleSetLineCmd` | line manager, store "buy onto line", first-stop schemes | carried: `AssignLine`, first stop or the game's choice (-1). Taking a vehicle off its line is refused (no line to name) | `the_next_reachable_stop_travels_as_the_games_choice`, `a_vehicle_bought_onto_a_line_is_put_on_it_once_the_room_can_name_it` |
| `makeVehicleSetStoppedByUserCmd` | vehicle window, line manager | carried: `VehicleOp` `Stop` | `a_personal_mods_game_script_hands_its_holds_to_the_room` |
| `makeVehicleSendToDepotCmd` | vehicle window, line manager | carried: `VehicleOp` `ToDepot`; with `jumpTo` (into a depot at once) refused | same |
| `makeVehicleReverseCmd` | vehicle window, line manager | carried: `VehicleOp` `Reverse` | same |
| `makeVehicleTryToDepartCmd` | (no GUI sender) | carried: `VehicleOp` `Depart` | same |
| `makeVehicleSetManualDepartureCmd` | (timetable mods) | carried: `VehicleOp` `ManualDeparture` | same |
| `makeVehicleSetModifiersCmd` | (game script `vehicle_modifier.script.tl` only) | refused from the GUI | `in_the_rooms_game_the_gui_refuses_what_the_room_cannot_carry` |
| `makeLineCreateCmd` | line manager | carried: `CreateLine`, waypoints included | `a_line_travels_by_its_stations_ids_and_is_made_again_the_same`, `a_new_lines_colour_comes_back_on_the_games_palette_step` |
| `makeLineUpdateCmd` | line manager, line window, cargo filter window | carried: `EditLine` `Update` (stops, terminals, alternatives, load mode, waiting times, loading rules, modes, custom filters, priority, waypoints) | `a_stops_loading_flags_reach_the_game_in_order_however_it_copied_them` |
| `makeLineDestroyCmd` | line manager | carried: `EditLine` `Delete` | registry tests |
| `makeEntitySetNameCmd` | entity windows' titles (`view_manager.tl`: any entity with a window), line manager (lines, vehicles, auto-rename schemes), company window | carried: lines (`EditLine` `Rename`), the room's companies (`CompanyOp` `Rename`), vehicles, stations, towns and other constructions (`Rename`); anything else refused ("renaming this") | `the_company_windows_rename_goes_to_the_room_as_the_companys`, `a_vehicle_station_town_or_depot_renamed_and_a_vehicle_recoloured_go_to_the_room`, `every_game_renames_and_recolours_what_the_room_names` |
| `makeEntitySetColorCmd` | line window, line manager, vehicle window, line manager's vehicle list | carried: lines, companies, vehicles (`VehicleOp` `Recolor`); anything else refused ("recolouring this") | `companies_are_founded_joined_renamed_recoloured_and_dissolved_alike`, `a_vehicle_station_town_or_depot_renamed_and_a_vehicle_recoloured_go_to_the_room` |
| `makeWorldBuildProposalCmd` | construction menu's parameters (`construction.tl`), station cargo buttons (`entity_window_util.tl`), bridge and tunnel window, a junction's window (carried as `EditJunctions`), industry removal (`industry_util.tl`), map editor, debug panel | carried: an edit that replaces one construction (`BuildConstruction` with `replaces`); everything else refused ("building from this window") | `a_construction_edited_in_its_window_goes_to_the_room` |
| `makeScriptingSendEventCmd` | see "Game-script events" | carried for six events, refused for the rest | see there |
| `makeGameSetSpeedCmd` | speed row | passed: the step gate reads it as a speed request | `in_the_rooms_game_the_gui_refuses_what_the_room_cannot_carry` |
| `makeGameSetCalendarSpeedCmd` | game bar | refused (owner decision: who sets a room's calendar) | same (refusal is generic) |
| `makeGameSetDateCmd` | game bar (sandbox) | refused | generic |
| `makeGameSetTimeOfDayCmd` | game bar | refused (owner decision) | generic |
| `makeGameSetCloudCoverageCmd` | (game script) | refused from the GUI | generic |
| `makeJournalBookAssetCmd` | game bar (`game_bar.tl:234`, sandbox money), marketing tool (its cost) | refused | generic |
| `makeStockListSetStocksCargoTypeCmd` | (commented out in build 40408's warehouse window, which sets a slot's cargo by replacing the construction instead: carried as a window's edit) | refused from the GUI | generic |
| `makeStockListDiscardCargoCmd` | warehouse window | carried: `DiscardCargo`, the warehouse by its construction (INFERRED: the window's entity is it) | `the_notification_log_and_a_warehouses_discard_go_to_the_room` |
| `makeCreateIndustryExtendProposalCmd` | industry window | refused (needs a probe: what it builds) | generic |
| `makeTownBuildingSetBlockedDevelopmentCmd` | town building window | refused | generic |
| `makeTownSetDevelopmentActiveCmd` | town window | refused (sandbox/map editor) | generic |
| `makeTownCustomDistributionWeightsCmd` | town window | refused | generic |
| `makeTownSetInitialLandUseCapacitiesCmd` | town window | refused | generic |
| `makeTownUpdateCargoNeedsCmd` | town window | refused | generic |
| `makeTownCreateCmd`, `makeTownDestroyCmd`, `makeTownAutoDetectConnectionsCmd`, `makeTownConnectWithIndustriesCmd` | map editor | refused | `the_guard_names_the_mod_and_lets_a_personal_mods_events_reach_its_script` (makeTownCreateCmd) |
| `makeWorldReplaceTerrainCmd` | map editor | refused | generic |
| `makeEntitySetPlayerCmd` | debug panel | refused | generic |
| `makeGameAddPlayerCmd` | (missions) | refused from the GUI; the room makes companies itself (`CompanyOp`) | generic |
| the other 19 (`makeAnimal*`, `makeClearLogbooksCmd`, `makeComponentExchangeCmd`, `makeCustomEntity*`, `makeCustomVehicleCreateOrUpdateCmd`, `makeEntitySetEmissionsCmd`, `makeGamePerformSimulationStepsCmd`, `makeIndustrySet*`, `makeJournalClearAllCmd`, `makeJournalLogEntryCmd`, `makeMaintenanceCostUpdateCmd`, `makeSimPersonSetStateCmd`, `makeStockListSetModifiersCmd`, `makeStockSetCargoAmountCmd`, `makeTownDevelopAtCmd`, `makeTownUpdateSizeCmd`, `makeWorldChangeWindCmd`, `makeWorldSetBulldozableCmd`) | no GUI sender on build 40408 (game scripts, missions, debug) | refused from the GUI | generic |

"generic" is `in_the_rooms_game_the_gui_refuses_what_the_room_cannot_carry`:
every kind not in `CARRY` or `PASS` takes the same path.

## Game-script events the GUI sends

`makeScriptingSendEventCmd("", id, name, param)` from the GUI's sources
(`gui.zip`, `game_mechanics.zip`; mission and tutorial senders left out:
they do not run in a room's free game).

| id, name | sent by | in the room | test |
|---|---|---|---|
| `Loan` `Obtain`, `Repay` | finance window (`finances_loan_gui.tl`) | carried: `Loan`; another company's loan its own | `in_the_rooms_game_a_loan_goes_to_the_room_and_nothing_else_of_its_kind`, `only_the_company_that_borrowed_pays_its_loan` |
| `Subvention` `onAccept`, `onDecline` | subsidy window | carried: `Subsidy`, first to accept takes it | `in_the_rooms_game_a_subsidys_answer_goes_to_the_room`, `every_game_gives_a_subsidy_to_the_first_company_to_accept_it` |
| `Companies` `spawnIndustry` | construction menu, prospection | carried: `Prospect` | `a_prospection_goes_to_the_room_by_its_towns_id_and_its_types_in_order` |
| `Companies` `applyLevel` | company window | carried: `ApplyRank` | `with_two_companies_a_company_takes_the_ranks_it_reached` |
| `Notifications` `initialSound` | notification popups | carried: `NotificationSeen` | `a_notifications_first_sound_is_marked_in_every_game` |
| `Notifications` `dismiss` | notification log and popups | carried: `Notification` `Dismiss` | `the_notification_log_and_a_warehouses_discard_go_to_the_room` |
| `Notifications` `enlist` | notification log | carried: `Notification` `Enlist` | same |
| `Notifications` `updateIgnoredTypes` | notification log settings | carried: `Notification` `Ignore` (by type name; by GUI kind refused) | same |
| `Companies` `MakeGreen` | industry greening tool | refused (PLAN.md Part 3: greening stays refused) | generic |
| `Companies` `startMarketingCampaign` | marketing tool (with a `makeJournalBookAssetCmd` for its cost) | refused: two commands that must be one action | generic |
| `Towns` `setTownRatingSensitivity`, `resetTownRatingSensitivity` | town window | refused (owner decision: a town's setting changed by one company for all) | generic |
| `GameTime` `SetMode`, `SkipPhase`, `OnManualGameTimeChanged` | game bar's time of day | refused (owner decision) | generic |
| `CloudCoverage` `SetMode`, `SkipPhase`, `OnManualCloudCoverageChanged` | game bar's weather | refused (owner decision) | generic |
| `Towns` `_debugAddExperience` | debug panel | refused | generic |

## Native tools

The tool's id as game scripts hear it (`builder.proposalCreate`), or how
the hook reads it.

| tool | in the room | test |
|---|---|---|
| street tool (`streetBuilder`), roads, bridges, tunnels, junctions with their turns, lights and crosswalks as the tool proposed them | carried: `BuildRoad` | `a_road_the_street_tool_proposed_goes_to_the_room`, `a_t_junction_built_through_the_room_is_configured_as_the_tool_proposed`, `lua_capture.rs` |
| track tool (`trackBuilder`), track types, catenary, bridges, tunnels | carried: `BuildTrack` | `a_captured_track_decodes_as_the_schema_says` (`lua_capture.rs`) |
| construction tool (`constructionBuilder`): stations, depots, harbours, airports, warehouses, industries' buildings the player places, landmarks, headquarters | carried: `BuildConstruction`, with its road connection | `a_construction_the_tool_placed_becomes_the_rooms_action`, `a_station_by_a_road_travels_with_the_junction_that_joins_it`, `each_company_builds_one_headquarters_of_its_own` |
| stop and signal tool (`streetTerminalBuilder`): bus and truck stops on a street, tram stops, track signals (one-way or not), track waypoints | carried: `PlaceStop` (`object` Stop, Signal, Waypoint) | `a_stop_the_stop_tool_placed_goes_to_the_room_and_every_game_places_it`, `a_stop_the_room_cannot_carry_says_why` |
| road and track modifiers (`streetTrackModifier`): tram track, bus lane, noise barrier, alley, lock, player-owned, electrification, track upgrade, track decorations | carried: `BuildRoad`/`BuildTrack` of the rebuilt edges | `a_road_modifier_is_built_with_its_lanes_decorations_lock_and_owner`, `a_track_modifier_carries_its_type_catenary_and_speed` |
| bulldozer (`bulldozer`): a construction, edges with the town buildings along them, a stop | carried: `Bulldoze`; an asset group (trees) refused | `the_bulldozer_removes_a_construction_or_edges_in_every_game`, `the_bulldozer_removes_a_stop_in_every_game`, `a_bulldoze_the_room_cannot_name_is_refused` |
| module editor (`UI::ModuleBuilder`), station and airport modules, upgrades | carried: `BuildConstruction` with `replaces`, read natively (INFERRED, not seen) | `a_module_editor_click_goes_to_the_room_as_the_hook_read_it`; `modules.rs` unit tests |
| terrain tools (raise, lower, smooth, flatten, heightmap) | carried: `Terraform`, read natively (not seen) | `a_terrain_tools_click_goes_to_the_room_as_terraform_actions`; `terrain.rs` unit tests |
| module bulldozer | carried as the edit it is, if it reaches game scripts as the bulldozer (INFERRED); else stopped by the build gate | `a_station_edit_a_click_saw_goes_to_the_room_and_unhandled_events_are_logged` |
| crossing tool (`lane_modifier_tool`, `UI::LaneModifier`), traffic light tool, crosswalk tool | carried behind `strict_junctions`, off until the two-player check (PLAN.md Part 3): `EditJunctions` (dev's junction tools, cafef85, with the tool named in the log; HOOKS.md, "Junction tools") | `the_traffic_light_tools_proposal_goes_to_the_room_and_every_game_lights_it_alike`, `a_crossing_tools_tram_lanes_join_a_railway_in_every_game_alike`, `a_crosswalk_tools_click_goes_to_the_room_and_every_game_sets_it_alike`, `junction_tools_round_trip_through_the_wire_and_apply_with_each_games_ids` |
| terrain painter, asset brush (trees, rocks, plants), vegetation and asset erasers | refused (build gate; the hook names why) | `terrain.rs` `what_is_not_only_a_height_grid_is_refused_with_why` |
| town builder tools | refused (build gate) | `in_the_rooms_game_the_build_tools_are_refused` |
| bridge and tunnel window (bridge type) | refused ("building from this window") | `a_construction_edited_in_its_window_goes_to_the_room` |
| a junction's window: traffic light phases, double slip switch | carried behind `strict_junctions`: `EditJunctions` | `a_junctions_window_sends_its_phases_and_double_slip_to_the_room` |
| industry window's extend, removal | refused | generic |
| line manager's map clicks (stops, waypoints) | carried through `makeLineUpdateCmd`, waypoints included | `a_lines_waypoints_on_track_and_in_the_open_are_made_again_the_same` |
| vehicle store | carried through `makeVehicleBuyCmd` | see the vehicle rows |
| depot window, vehicle window | carried through the vehicle commands | see the vehicle rows |

## Per transport mode

| | road | rail | water | air |
|---|---|---|---|---|
| network | streets, bridges, tunnels, junctions: carried | tracks, types, catenary: carried | no canal or waterway tool on build 40408 | runways are airport modules |
| stops and stations | bus and truck stops (stop tool): carried; bus and truck stations (construction tool): carried | stations: carried; module edits: carried (INFERRED) | harbours (construction tool): carried, not seen | airports (construction tool): carried, not seen; upgrades and runways (module editor): carried, INFERRED |
| depots | road depots: carried, seen | train depots: carried, seen | ship depots: carried, not seen | hangars: carried as constructions, not seen |
| signals, waypoints | lights and crosswalks: with the junction from the street tool; the light and crosswalk tools refused | signals, waypoints (stop tool): carried; bulldozing them: carried where the object has a construction (INFERRED) | buoys: none on build 40408 | none |
| buying | store at a depot: carried | multiple units and wagons by group: carried | ships: carried; the depot named by the construction that lists it (`capture.depotRef`) where no street reaches it | aircraft: carried, the hangar named as a ship depot is; the depot by its index among its construction's depots (an airport's second hangar) |
| lines | stops, terminals, waiting, loading: carried | the same; track waypoints: carried, on the edge's lane by its ends | the same; route waypoints (`Waypoint.pos`): carried by position | the same; route waypoints: carried by position |
| vehicle actions | sell, replace, to depot, stop, reverse, depart, rename, colour: carried | the same | the same | the same |

Auto-replace: build 40408 has no auto-replace in its GUI. Line frequency
and intervals are the stops' waiting times (`minWaitingTime`,
`maxWaitingTime`, `maxAdditionalWaitingTime`): carried.

## Gaps

Done on `feat/combined`, each batch its own commit:

| batch | what | schema | tests |
|---|---|---|---|
| 2 | the guard in the GUI's React state too (`tpf3mp/hudguard.lua`); a window there that waits on what its command made is refused, as the room's answers reach the plugin's state alone | | `in_the_huds_state_the_guard_carries_or_refuses_every_command` |
| 3 | the hook logs every command kind queued in the room's game, once per call site (`cmdkinds.rs`): measurement for U2 | | `cmdkinds` unit tests |
| | a ship depot or hangar no street reaches named by the construction that lists it (`capture.depotRef`) | | `a_ship_or_aircraft_is_bought_at_the_harbour_or_airport_that_lists_its_depot` |
| 4 | `Rename` (vehicle, station, town, construction), `VehicleChange::Recolor` | 17 | `a_vehicle_station_town_or_depot_renamed_and_a_vehicle_recoloured_go_to_the_room`, `every_game_renames_and_recolours_what_the_room_names` |
| 5 | `LineStop::waypoints`: on a lane by the edge's ends (node 0 first; a game whose edge runs the other way refuses) or the construction's network, or in the open for ships and aircraft | 18 | `a_lines_waypoints_on_track_and_in_the_open_are_made_again_the_same` |
| 6, 7 | `Notification` (dismiss, enlist, ignored kinds), `DiscardCargo` | 19 | `the_notification_log_and_a_warehouses_discard_go_to_the_room` |
| 9 | `BuyVehicle::depot_index`: the depot of a construction with several | 20 | `every_game_buys_at_the_constructions_depot_the_store_bought_at` |

INFERRED in these, to see in a game: that a harbour's or airport's depot
may have no street connector (the fallback is harmless if it has one);
that the warehouse window's entity is the warehouse's construction; that
assigning a `Line.Stop`'s `waypoints` and an `EdgePos` from Lua is taken
as the line manager's own; and which windows render in the HUD's state.

### Needs an owner decision or an in-game probe

| what | why it waits |
|---|---|
| time of day, weather, calendar speed, date | who in a room may set them (the owner, as the speed?) |
| a town's rating sensitivity, cargo needs, development, distribution weights, a town building's blocked development | one company changing a town for all |
| greening an industry, marketing campaigns | PLAN.md keeps greening refused; marketing is two commands (the event and its cost) |
| industry extension (`makeCreateIndustryExtendProposalCmd`), industry removal | what the proposal builds: a probe |
| bridge and tunnel window | rebuilds edges from a window: a probe of the proposal |
| the asset brush, the terrain painter, the asset bulldozer | rebuilding an asset group natively (HOOKS.md, "The build tools") |
| the console's state (U3) | the tests use it; guarding it is the owner's call |
| native commands of other kinds (U2) | refuse once batch 3's log names them |

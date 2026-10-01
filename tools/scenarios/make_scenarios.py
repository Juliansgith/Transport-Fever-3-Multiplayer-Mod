#!/usr/bin/env python3
"""Writes the hook's test-mode scenarios (crates/tpf3mp-hook/src/scenario.rs)
for real games in one room: `python tools/scenarios/make_scenarios.py`
rewrites every `*.json` beside it. Edit this file, not the JSON.

Each actor plays near a town of its own (actor k near the baseline's town
#k, at its first free flat spot), so their builds never meet, and one after
the other (actor k's part starts `ACTOR_GAP * k` steps later), so the ids the
room's registry gives what they make (station groups, lines, vehicles) come
in a known order: `$id` placeholders count them from the baseline.

Positions are metres east and north of the actor's spot (`$pos`); the
schema's own values (tangents, directions) are in millimetres and
millionths. What each action needs is in tpf3mp_proto::action
(docs/BUILDING.md); resource names are the game's, build 40408.
"""

import json
import os

HERE = os.path.dirname(os.path.abspath(__file__))

ACTORS = 3
ACTOR_GAP = 1500

COUNTRY_STREET = "::/infrastructure/street/country/country_old_small.street_template"
TOWN_STREET = "::/infrastructure/street/town/town_old_small.street_template"
TRAM_STREET = "::/infrastructure/street/town/town_old_small_tram.street_template"
BUS_STREET = "::/infrastructure/street/country/country_new_medium_bus.street_template"
TRACK = "::/infrastructure/track/standard/standard.street_template"
TRACK_CATENARY = "::/infrastructure/track/standard/standard_catenary.street_template"
STOP = "::/stations/street/small_stops/small_old.con"
ROAD_DEPOT = "::/depots/road/road_depot/road_depot.con"
RAIL_DEPOT = "::/depots/rail/rail_depot.con"
WATER_DEPOT = "::/depots/water/water_depot.con"
TRUCK_STATION = "::/stations/street/modular_street_station/modular_terminal.con"
RAIL_STATION = "::/stations/rail/modular_station/modular_station.con"
HARBOUR = "::/stations/water/harbor_modular.con"
AIRFIELD = "::/stations/air/airfield.con"
HQ = "::/landmarks/hq/headquarter.con"
SIGNAL = "::/infrastructure/signal/signal_path_a.con"

# Vehicle models, by the game's resource names (INFERRED from the content
# folders: the baseline lists the names this game has, under "models").
BUS = "::/vehicle/bus/american_post_coach/american_post_coach.mdl"
TRUCK = "::/vehicle/truck/benz1912/benz1912_box.mdl"
LOCOMOTIVE = "::/vehicle/train/atlantic_4_4_2/atlantic_4_4_2.mdl"
WAGGON = "::/vehicle/waggon/boxcar_1_20/boxcar_1_20.mdl"
SHIP = "::/vehicle/ship/sternwheeler/sternwheeler.mdl"
PLANE = "::/vehicle/plane/junkers_f_13/junkers_f_13.mdl"

# Transport modes, as TF3 numbers them (api.type.TransportMode).
MODE_BUS, MODE_TRUCK, MODE_TRAIN, MODE_AIRCRAFT, MODE_SHIP = 3, 4, 7, 9, 10

IDENTITY = [1_000_000, 0, 0, 0, 1_000_000, 0, 0, 0, 1_000_000]


def pos(x, y, z=0):
    return {"$pos": [x, y, z]}


def mm(metres):
    return int(round(metres * 1000))


def straight(a, b):
    """The tangent of a straight edge from a to b, in millimetres."""
    return {"x": mm(b[0] - a[0]), "y": mm(b[1] - a[1]), "z": 0}


def ends(a, b):
    return {"a": pos(*a), "b": pos(*b)}


def edge(network, a, b):
    return {"network": network, "ends": ends(a, b)}


def polyline(points, resolves=None, kind=None):
    """A chain of straight edges through `points`; `resolves` maps a vertex
    index to its Resolve (New by default)."""
    resolves = resolves or {}
    vertices = [{"pos": pos(*p), "resolve": resolves.get(i, "New")} for i, p in enumerate(points)]
    links = []
    for i in range(len(points) - 1):
        t = straight(points[i], points[i + 1])
        links.append({"from": i, "to": i + 1, "tangent0": t, "tangent1": t,
                      "structure": "Ground", "kind": kind})
    return {"vertices": vertices, "links": links, "removals": [], "removed_nodes": []}


def road(points, resolves=None, street=COUNTRY_STREET, bus_lane=False, tram="None"):
    return {"BuildRoad": {"street": street, "style": None, "bus_lane": bus_lane, "tram": tram,
                          "polyline": polyline(points, resolves)}}


def track(points, resolves=None, kind=TRACK, catenary=False):
    return {"BuildTrack": {"track": kind, "style": None, "catenary": catenary,
                           "polyline": polyline(points, resolves)}}


def split(network, a, b):
    return {"Split": edge(network, a, b)}


def construction(file, at, name, params=None):
    params = params or []
    return {"BuildConstruction": {
        "file": file,
        "transform": {"basis": IDENTITY, "origin": pos(*at)},
        "params": [{"key": "seed", "value": {"Int": 0}}] + params,
        "name": name, "replaces": None, "connection": None}}


def place_stop(network, a, b, at, model=STOP, kind="Stop"):
    d = (b[0] - a[0], b[1] - a[1])
    length = (d[0] ** 2 + d[1] ** 2) ** 0.5
    return {"PlaceStop": {
        "edge": edge(network, a, b), "at": pos(*at), "left": True,
        "direction": {"x": int(round(d[0] / length * 1e6)), "y": int(round(d[1] / length * 1e6)), "z": 0},
        "model": model, "two_sided": False, "object": kind, "one_way": False}}


def depot_ref(file, at):
    return {"file": file, "at": pos(*at)}


def buy(file, at, model, loads=None):
    return {"BuyVehicle": {
        "depot": depot_ref(file, at),
        "consist": [{"model": model, "reversed": False, "loads": loads or [],
                     "color": {"r": 800000, "g": 160000, "b": 120000}}],
        "groups": [1], "multiple_units": [""]}}


def line_stop(group):
    return {"group": {"$id": group}, "terminal": {"station": 0, "terminal": 0}, "alternatives": [],
            "load_mode": "LoadIfAvailable", "min_wait": 0, "max_wait": 180_000_000,
            "max_extra_wait": 0,
            "rules": {"load": [], "max_load": [], "force_unload": False,
                      "destroy_for_config_change": False, "destroy_for_refresh": False}}


def create_line(name, groups, mode):
    return {"CreateLine": {"name": name, "color": {"r": 130000, "g": 420000, "b": 850000},
                           "line": {"stops": [line_stop(g) for g in groups], "modes": [mode],
                                    "custom_filters": False, "reservation_priority": 0}}}


def assign(vehicles, line):
    return {"AssignLine": {"vehicles": [{"$id": v} for v in vehicles], "line": {"$id": line},
                           "first_stop": None}}


def company_create(name):
    return {"CompanyOp": {"Create": {"name": name}}}


def loan_terms(kind, amount, loan_id=None):
    return {"type": kind, "amount": amount, "duration": 31_536_000_000, "percentage": 30_000,
            "birthDay": None, "cooldownUntil": None, "lastPayDay": None, "timesPaid": None,
            "id": loan_id}


class Scenario:
    def __init__(self, name, comment, observe_every=10):
        self.name, self.comment, self.observe_every = name, comment, observe_every
        self.items = []

    def act(self, at, actor, action, note, expect="applied", origin=None):
        self.items.append({"at": at, "actor": actor, "origin": origin or f"spot:#{actor}:0",
                           "expect": expect, "note": note, "action": action})

    def write(self):
        self.items.sort(key=lambda item: (item["at"], item["actor"]))
        text = json.dumps({"name": self.name, "comment": self.comment,
                           "observe_every": self.observe_every, "items": self.items}, indent=1)
        with open(os.path.join(HERE, self.name + ".json"), "w", newline="\n") as f:
            f.write(text + "\n")


def roads():
    """Streets, junctions, a bus line with three buses bought at once, a track
    with a depot at its end, a loan taken and repaid, a town street
    bulldozed, and one action every game must refuse; each actor's company
    its own (a competitive room)."""
    s = Scenario("roads", roads.__doc__)
    S = "Street"
    for k in range(ACTORS):
        t = ACTOR_GAP * k
        c = f"Actor {k}"
        s.act(t + 10, k, company_create(f"Scenario company {k}"),
              "a company of this actor's own (any: a competitive room may have founded one)", "any")
        # A straight street, then a T junction on it, then a crossroads.
        s.act(t + 50, k, road([(0, 0), (120, 0)]), f"{c}: a straight street")
        s.act(t + 100, k, road([(60, 0), (60, 60)], {0: split(S, (0, 0), (120, 0))}),
              f"{c}: a T junction, splitting the street at 60 m")
        s.act(t + 150, k, road([(90, -60), (90, 0), (90, 60)], {1: split(S, (60, 0), (120, 0))}),
              f"{c}: a crossroads, splitting the street's east half at 90 m")
        # Two bus stops and a road depot.
        s.act(t + 200, k, place_stop(S, (0, 0), (60, 0), (30, 0)), f"{c}: a bus stop on the street's west part")
        s.act(t + 230, k, place_stop(S, (90, 0), (90, 60), (90, 30)), f"{c}: a bus stop on the crossroads' north arm")
        s.act(t + 260, k, construction(ROAD_DEPOT, (30, 40), f"Scenario depot {k}"),
              f"{c}: a road depot north of the street (no entrance street: vehicles stay in it)")
        # A line between the two stops, three buses bought at once, onto it.
        g = 2 * k
        s.act(t + 320, k, create_line(f"Scenario bus line {k}", [f"groups+{g}", f"groups+{g + 1}"], MODE_BUS),
              f"{c}: a bus line between the two stops (station groups +{g} and +{g + 1})")
        for n in range(3):
            s.act(t + 360 + n, k, buy(ROAD_DEPOT, (30, 40), BUS), f"{c}: bus {n + 1} of 3, bought in one burst")
        v = 3 * k
        s.act(t + 420, k, assign([f"vehicles+{v}", f"vehicles+{v + 1}", f"vehicles+{v + 2}"], f"lines+{k}"),
              f"{c}: the three buses onto the line")
        # A track with a rail depot where it ends.
        s.act(t + 480, k, track([(0, -80), (150, -80)]), f"{c}: a straight track south of the streets")
        s.act(t + 520, k, construction(RAIL_DEPOT, (160, -80), f"Scenario rail depot {k}"),
              f"{c}: a rail depot at the track's east end (any: placed by its origin, not snapped)", "any")
        # A loan taken and paid back.
        s.act(t + 600, k, {"Loan": {"Take": {"next": loan_terms("Small", 500_000),
                                             "offer": loan_terms("Small", 500_000)}}},
              f"{c}: take a small loan (any: the offer is the scenario's, not the game's draw)", "any")
        s.act(t + 700, k, {"Loan": {"Repay": {"loan": loan_terms("Small", 500_000, 1)}}},
              f"{c}: pay the loan back (any: its id is a guess)", "any")
        # A town street bulldozed, with whatever stands on it refused.
        bulldoze = {"Bulldoze": {"Edges": {"network": S, "edges": [{"$edge": f"#{k}:0"}], "buildings": []}}}
        s.items.append({"at": t + 800, "actor": k, "origin": "none", "expect": "any",
                        "note": f"{c}: bulldoze the street nearest town #{k}'s centre (any: buildings along it refuse it)",
                        "action": bulldoze})
        # Every game must refuse a bus bought at a depot that is not there.
        s.act(t + 900, k, buy(ROAD_DEPOT, (-500, -500), BUS), f"{c}: a bus at no depot: refused", "refused")
    s.write()


def rebuilt(action, key, network, a, b):
    """`action`, a build of one edge from a to b, as an upgrade tool's: its
    ends are the existing nodes, and the old edge goes."""
    body = action[key]
    for v in body["polyline"]["vertices"]:
        v["resolve"] = {"Node": network}
    body["polyline"]["removals"] = [edge(network, a, b)]
    return action


def curve(a, b, bend):
    """One edge from a to b bowed sideways: its tangents turned by `bend`
    metres at each end."""
    t0 = {"x": mm(b[0] - a[0]), "y": mm(b[1] - a[1] + bend), "z": 0}
    t1 = {"x": mm(b[0] - a[0]), "y": mm(b[1] - a[1] - bend), "z": 0}
    return {"vertices": [{"pos": pos(*a), "resolve": "New"}, {"pos": pos(*b), "resolve": "New"}],
            "links": [{"from": 0, "to": 1, "tangent0": t0, "tangent1": t1, "structure": "Ground",
                       "kind": None}],
            "removals": [], "removed_nodes": []}


def company_first(s, k, t):
    s.act(t + 10, k, company_create(f"Scenario company {k}"),
          f"Actor {k}: a company of its own (any: a competitive room may have founded one)", "any")


def vehicle_op(vehicle, change):
    return {"VehicleOp": {"vehicle": {"$id": vehicle}, "change": change}}


def road_upgrades():
    """A curved street, a straight one upgraded with a bus lane and then with
    a tram track, the curved street bulldozed, a town building demolished,
    and a headquarters (a second one refused)."""
    s = Scenario("road_upgrades", road_upgrades.__doc__)
    S = "Street"
    for k in range(ACTORS):
        t, c = ACTOR_GAP * k, f"Actor {k}"
        company_first(s, k, t)
        s.act(t + 50, k, {"BuildRoad": {"street": COUNTRY_STREET, "style": None, "bus_lane": False,
                                        "tram": "None", "polyline": curve((0, 0), (100, 40), 30)}},
              f"{c}: a curved street")
        s.act(t + 100, k, road([(0, -40), (100, -40)]), f"{c}: a straight street to upgrade")
        s.act(t + 150, k, rebuilt(road([(0, -40), (100, -40)], bus_lane=True), "BuildRoad", S,
                                  (0, -40), (100, -40)),
              f"{c}: the straight street rebuilt with a bus lane (an upgrade)")
        s.act(t + 200, k, rebuilt(road([(0, -40), (100, -40)], street=TRAM_STREET, tram="Plain"),
                                  "BuildRoad", S, (0, -40), (100, -40)),
              f"{c}: rebuilt again as a tram street (any: its lanes are the template's)", "any")
        s.act(t + 260, k, {"Bulldoze": {"Edges": {"network": S, "edges": [ends((0, 0), (100, 40))],
                                                  "buildings": []}}},
              f"{c}: the curved street bulldozed")
        s.act(t + 320, k, {"Bulldoze": {"Construction": {"$building": f"#{k}:0"}}},
              f"{c}: the town building nearest town #{k}'s centre demolished (any)", "any",
              origin="none")
        s.act(t + 380, k, construction(HQ, (150, 0), f"Scenario HQ {k}"),
              f"{c}: the company's headquarters (any: its params are the scenario's)", "any")
        s.act(t + 440, k, construction(HQ, (150, 80), f"Scenario HQ {k} again"),
              f"{c}: a second headquarters: refused (one a company)", "refused")
    s.write()


def rail():
    """A track with a signal on it, a station beside it and a depot at its
    end, the track electrified, a train bought, run on a line, stopped and
    started, and the track's signalled edge bulldozed."""
    s = Scenario("rail", rail.__doc__)
    T = "Track"
    for k in range(ACTORS):
        t, c = ACTOR_GAP * k, f"Actor {k}"
        company_first(s, k, t)
        s.act(t + 50, k, track([(0, 0), (100, 0), (200, 0)]), f"{c}: a straight track in two edges")
        s.act(t + 100, k, place_stop(T, (0, 0), (100, 0), (50, 0), model=SIGNAL, kind="Signal"),
              f"{c}: a signal on the track's west edge (any: the model is inferred)", "any")
        s.act(t + 150, k, construction(RAIL_STATION, (100, 30), f"Scenario station {k}"),
              f"{c}: a rail station beside the track (any: a modular station needs its modules)", "any")
        s.act(t + 200, k, construction(RAIL_DEPOT, (215, 0), f"Scenario rail depot {k}"),
              f"{c}: a rail depot past the track's east end (any: placed, not snapped)", "any")
        s.act(t + 260, k, rebuilt(track([(100, 0), (200, 0)], kind=TRACK_CATENARY, catenary=True),
                                  "BuildTrack", T, (100, 0), (200, 0)),
              f"{c}: the east edge electrified (an upgrade)")
        s.act(t + 320, k, {"BuyVehicle": {
            "depot": depot_ref(RAIL_DEPOT, (215, 0)),
            "consist": [{"model": m, "reversed": False, "loads": [], "color": {"r": 0, "g": 0, "b": 0}}
                        for m in (LOCOMOTIVE, WAGGON)],
            "groups": [1, 1], "multiple_units": ["", ""]}},
              f"{c}: a locomotive and a waggon (any: the depot may not be there)", "any")
        s.act(t + 380, k, create_line(f"Scenario train line {k}", [f"groups+{k}", f"groups+{k}"], MODE_TRAIN),
              f"{c}: a train line (any: the station's group is a guess)", "any")
        s.act(t + 420, k, assign([f"vehicles+{k}"], f"lines+{k}"), f"{c}: the train onto it", "any")
        s.act(t + 480, k, vehicle_op(f"vehicles+{k}", {"Stop": True}), f"{c}: the train stopped", "any")
        s.act(t + 500, k, vehicle_op(f"vehicles+{k}", {"Stop": False}), f"{c}: and started again", "any")
        s.act(t + 560, k, {"Bulldoze": {"Edges": {"network": T, "edges": [ends((0, 0), (100, 0))],
                                                  "buildings": []}}},
              f"{c}: the track's west edge bulldozed, its signal with it")
    s.write()


def road_vehicles():
    """Trucks: two stops and a truck station, a depot, three trucks bought at
    once onto a line, the line renamed and recoloured, the vehicle window's
    orders, one truck replaced, one sent to be sold, one sold, the line
    deleted, and another company's truck sold: refused."""
    s = Scenario("road_vehicles", road_vehicles.__doc__)
    S = "Street"
    for k in range(ACTORS):
        t, c = ACTOR_GAP * k, f"Actor {k}"
        company_first(s, k, t)
        s.act(t + 50, k, road([(0, 0), (150, 0)]), f"{c}: a street")
        s.act(t + 100, k, place_stop(S, (0, 0), (150, 0), (40, 0)), f"{c}: a truck stop (a street stop)")
        s.act(t + 130, k, place_stop(S, (0, 0), (150, 0), (110, 0)),
              f"{c}: a second stop (any: the first may have split the street)", "any")
        s.act(t + 160, k, construction(TRUCK_STATION, (110, 40), f"Scenario truck station {k}"),
              f"{c}: a truck station (any: a modular station needs its modules)", "any")
        s.act(t + 200, k, construction(ROAD_DEPOT, (70, -40), f"Scenario truck depot {k}"), f"{c}: a road depot")
        g, v = 2 * k, 3 * k
        s.act(t + 260, k, create_line(f"Scenario truck line {k}", [f"groups+{g}", f"groups+{g + 1}"], MODE_TRUCK),
              f"{c}: a truck line between the stops (any)", "any")
        for n in range(3):
            s.act(t + 300 + n, k, buy(ROAD_DEPOT, (70, -40), TRUCK), f"{c}: truck {n + 1} of 3, in one burst")
        s.act(t + 360, k, assign([f"vehicles+{v}", f"vehicles+{v + 1}", f"vehicles+{v + 2}"], f"lines+{k}"),
              f"{c}: the trucks onto the line", "any")
        s.act(t + 400, k, {"EditLine": {"line": {"$id": f"lines+{k}"}, "change": {"Rename": f"Renamed line {k}"}}},
              f"{c}: the line renamed", "any")
        s.act(t + 420, k, {"EditLine": {"line": {"$id": f"lines+{k}"},
                                        "change": {"Recolor": {"r": 950000, "g": 720000, "b": 80000}}}},
              f"{c}: the line recoloured", "any")
        for n, change in enumerate([{"Stop": True}, {"Stop": False}, "Reverse", "Depart",
                                    {"ManualDeparture": True}, {"ManualDeparture": False}]):
            s.act(t + 460 + 10 * n, k, vehicle_op(f"vehicles+{v}", change),
                  f"{c}: the vehicle window's {json.dumps(change)}", "any")
        s.act(t + 560, k, {"ReplaceVehicle": {
            "vehicle": {"$id": f"vehicles+{v + 1}"},
            "consist": [{"part": {"model": TRUCK, "reversed": False, "loads": [],
                                  "color": {"r": 0, "g": 0, "b": 0}}, "kept": 0}],
            "groups": [1], "multiple_units": [""]}},
              f"{c}: truck 2 replaced by its own model, kept (any)", "any")
        s.act(t + 600, k, vehicle_op(f"vehicles+{v + 2}", {"ToDepot": {"sell": True}}),
              f"{c}: truck 3 sent to the depot to be sold", "any")
        s.act(t + 640, k, {"SellVehicle": {"vehicles": [{"$id": f"vehicles+{v}"}]}}, f"{c}: truck 1 sold")
        s.act(t + 700, k, {"EditLine": {"line": {"$id": f"lines+{k}"}, "change": "Delete"}},
              f"{c}: the line deleted", "any")
        if k > 0:
            s.act(t + 740, k, {"SellVehicle": {"vehicles": [{"$id": "vehicles+1"}]}},
                  f"{c}: actor 0's truck 2 sold: refused (another company's)", "refused")
    s.write()


def water():
    """A ship depot and a harbour on the water near each town, a ship bought
    and run on a line."""
    s = Scenario("water", water.__doc__)
    for k in range(ACTORS):
        t, c, w = ACTOR_GAP * k, f"Actor {k}", f"water:#{k}:0"
        company_first(s, k, t)
        s.act(t + 50, k, construction(WATER_DEPOT, (0, 0), f"Scenario shipyard {k}"),
              f"{c}: a ship depot on the water (any: placed, not fitted to the shore)", "any", origin=w)
        s.act(t + 100, k, construction(HARBOUR, (60, 0), f"Scenario harbour {k}"),
              f"{c}: a harbour (any: a modular harbour needs its modules and a shore)", "any", origin=w)
        s.act(t + 160, k, buy(WATER_DEPOT, (0, 0), SHIP), f"{c}: a ship", "any", origin=w)
        s.act(t + 220, k, create_line(f"Scenario ship line {k}", [f"groups+{k}", f"groups+{k}"], MODE_SHIP),
              f"{c}: a ship line (any: the harbour's group is a guess)", "any")
        s.act(t + 260, k, assign([f"vehicles+{k}"], f"lines+{k}"), f"{c}: the ship onto it", "any")
    s.write()


def air():
    """An airfield on a flat spot near each town, a plane bought at its
    hangar and run on a line."""
    s = Scenario("air", air.__doc__)
    for k in range(ACTORS):
        t, c = ACTOR_GAP * k, f"Actor {k}"
        company_first(s, k, t)
        s.act(t + 50, k, construction(AIRFIELD, (0, 0), f"Scenario airfield {k}"),
              f"{c}: an airfield (any: its params are the scenario's)", "any")
        s.act(t + 120, k, buy(AIRFIELD, (0, 0), PLANE), f"{c}: a plane at the airfield's hangar", "any")
        s.act(t + 180, k, create_line(f"Scenario air line {k}", [f"groups+{k}", f"groups+{k}"], MODE_AIRCRAFT),
              f"{c}: an air line (any)", "any")
        s.act(t + 220, k, assign([f"vehicles+{k}"], f"lines+{k}"), f"{c}: the plane onto it", "any")
    s.write()


def money():
    """Companies, loans, subsidies, ranks, prospecting and notifications:
    each actor founds a company, renames and recolours it, closes and opens
    its stations, borrows twice and repays; a subsidy no game offers and a
    rank not reached are refused; actor 1 joins actor 0's company and
    founds its own again."""
    s = Scenario("money", money.__doc__)
    cargo_subsidy = "::/game_mechanics/subventions/deliver_cargo/deliver_cargo.res"
    for k in range(ACTORS):
        t, c = ACTOR_GAP * k, f"Actor {k}"
        me = {"$id": f"companies+{k}"}
        s.act(t + 10, k, company_create(f"Money company {k}"), f"{c}: a company", "any")
        s.act(t + 40, k, {"CompanyOp": {"Rename": {"company": me, "name": f"Renamed company {k}"}}},
              f"{c}: renamed", "any")
        s.act(t + 60, k, {"CompanyOp": {"Recolor": {"company": me,
                                                    "color": {"r": 180000, "g": 660000, "b": 270000}}}},
              f"{c}: recoloured", "any")
        s.act(t + 80, k, {"CompanyOp": {"ShareStations": {"company": me, "open": False}}},
              f"{c}: its stations closed to others", "any")
        s.act(t + 100, k, {"CompanyOp": {"StationAccess": {"company": me, "other": {"$id": "companies+0"},
                                                           "open": True}}},
              f"{c}: but open to actor 0's company", "any")
        s.act(t + 120, k, {"CompanyOp": {"ShareStations": {"company": me, "open": True}}},
              f"{c}: open again", "any")
        s.act(t + 140, k, {"CompanyOp": {"Unlock": me}}, f"{c}: unlocked (it had no password)", "any")
        s.act(t + 200, k, {"Loan": {"Take": {"next": loan_terms("Small", 500_000),
                                             "offer": loan_terms("Small", 500_000)}}},
              f"{c}: a small loan taken", "any")
        s.act(t + 260, k, {"Loan": {"Take": {"next": loan_terms("Medium", 1_000_000),
                                             "offer": loan_terms("Medium", 1_000_000)}}},
              f"{c}: a medium loan taken", "any")
        s.act(t + 400, k, {"Loan": {"Repay": {"loan": loan_terms("Small", 500_000, 1)}}},
              f"{c}: the small loan repaid (any: its id is a guess)", "any")
        s.act(t + 460, k, {"Subsidy": {"Accept": {"uid": 999_999, "kind": cargo_subsidy}}},
              f"{c}: a subsidy no game offers accepted: refused", "refused")
        s.act(t + 480, k, {"Subsidy": {"Decline": {"uid": 999_998, "kind": cargo_subsidy}}},
              f"{c}: one no game offers declined: refused", "refused")
        s.act(t + 520, k, {"ApplyRank": {"level": 15}}, f"{c}: rank 15, not reached: refused", "refused")
        s.act(t + 560, k, {"Prospect": {"town": k, "cargo": "::/cargos/coal/coal.cargo", "industries": [],
                                        "permit": None}},
              f"{c}: prospecting for coal near town id {k} (any)", "any")
        s.act(t + 600, k, {"NotificationSeen": {"notification": 1}}, f"{c}: a notification seen (any)", "any")
    end = ACTOR_GAP * ACTORS
    s.act(end + 10, 1, {"CompanyOp": {"Join": {"$id": "companies+0"}}}, "Actor 1: joins actor 0's company", "any")
    s.act(end + 60, 1, company_create("Money company 1 again"), "Actor 1: a company of its own again", "any")
    s.write()


def terraform():
    """A 16 m square of each actor's spot raised to 2 m and lowered again."""
    s = Scenario("terraform", terraform.__doc__)
    for k in range(ACTORS):
        t, c = ACTOR_GAP * k, f"Actor {k}"
        company_first(s, k, t)
        for n, (target, before) in enumerate([(2_000, 0), (0, 2_000)]):
            s.act(t + 100 + 100 * n, k, {"Terraform": {
                "origin": {"$cell2": [0, 0]}, "cell": 4000, "columns": 4,
                "cells": [{"target": target, "before": before}] * 16}},
                  f"{c}: the square's cells set to {target} mm (any: absolute heights, not the spot's)", "any")
    s.write()


if __name__ == "__main__":
    roads()
    road_upgrades()
    rail()
    road_vehicles()
    water()
    air()
    money()
    terraform()

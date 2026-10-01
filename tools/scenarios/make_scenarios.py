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
TRUCK = "::/vehicle/truck/gmc_ac/gmc_ac.mdl"

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

    def act(self, at, actor, action, note, expect="applied"):
        self.items.append({"at": at, "actor": actor, "origin": f"spot:#{actor}:0",
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


if __name__ == "__main__":
    roads()

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
# The track of 1912, as a player's build there names it.
TRACK = "::/infrastructure/track/simple/simple.street_template"
TRACK_STYLE = "::/infrastructure/track/simple/simple.street"
TRACK_CATENARY = "::/infrastructure/track/simple/simple_catenary.street_template"
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

# Vehicle models, by the game's resource names (the baseline lists the names
# this game has, under "models"), each in service in 1912, the fixture
# save's year (the models' availability, build 40408). Their loads are left
# out: every game gives each compartment the store's own (apply.lua).
BUS = "::/vehicle/bus/american_post_coach/american_post_coach.mdl"
TRUCK = "::/vehicle/truck/benz1912/benz1912_box.mdl"
LOCOMOTIVE = "::/vehicle/train/atlantic_4_4_2/atlantic_4_4_2.mdl"
WAGGON = "::/vehicle/waggon/boxcar_1_20/boxcar_1_20.mdl"
SHIP = "::/vehicle/ship/british_columbia/british_columbia.mdl"
# No plane is in service before 1920 (the Junkers F 13): the air scenario
# needs a later save, and in 1912 its buy is the game's to refuse.
PLANE = "::/vehicle/plane/junkers_f_13/junkers_f_13.mdl"

# Transport modes, as TF3 numbers them (api.type.TransportMode).
MODE_BUS, MODE_TRUCK, MODE_TRAIN, MODE_AIRCRAFT, MODE_SHIP = 3, 4, 7, 9, 10

# The fixture save's year: a construction's "year" parameter, as the
# construction tool sets it (a player's rail depot, captured 2026-10-01).
YEAR = 1912

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
    return {"BuildTrack": {"track": kind, "style": TRACK_STYLE, "catenary": catenary,
                           "polyline": polyline(points, resolves)}}


def split(network, a, b):
    return {"Split": edge(network, a, b)}


def construction(file, at, name, params=None):
    params = params or []
    return {"BuildConstruction": {
        "file": file,
        "transform": {"basis": IDENTITY, "origin": pos(*at)},
        "params": [{"key": "seed", "value": {"Int": 0}}, {"key": "year", "value": {"Int": YEAR}}] + params,
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


# The room's companies start with no money (a competitive room's player
# companies, and every company CompanyOp.Create founds): each actor borrows
# first, and pays back last. The room numbers loans from 1, room-wide
# (tpf3mp/companies.lua), and the baseline says the next one ("loans"), so
# actor k's first loan, taken in turn, is `loans+k` where each actor takes
# one.
FUND = 3_000_000

# A depot's basis for each way it faces: local +y (the way its entrance
# runs into it) turned to world east, north or west.
FACING = {
    "east": [0, -1_000_000, 0, 1_000_000, 0, 0, 0, 0, 1_000_000],
    "north": IDENTITY,
    "west": [0, 1_000_000, 0, -1_000_000, 0, 0, 0, 0, 1_000_000],
}


def turn(facing, local):
    """A depot-local (x, y) as world east and north offsets."""
    b = FACING[facing]
    return (local[0] * b[0] / 1e6 + local[1] * b[3] / 1e6, local[0] * b[1] / 1e6 + local[1] * b[4] / 1e6)


def construction_at(file, at, name, basis=IDENTITY):
    action = construction(file, at, name)
    action["BuildConstruction"]["transform"]["basis"] = basis
    return action


# The road depot's entrance street runs from (0, -35) to (0, -20.26) in its
# own frame (road_depot.script.lua, build 40408). Its entrance stops SHORT
# metres before the street's free end and the connection names that end:
# the game's refresh snaps it on (round B, 2026-10-01: "snapping ... +e-2:
# <the street's node>>-1"). A construction's node right on the street's
# node was refused ("Construction Not Possible").
ROAD_DEPOT_ENTRANCE = (0, -35)
SHORT = 3


def road_depot(node, name, facing="north"):
    """A road depot beyond `node`, the free end of a street running `facing`
    into it. Returns the action and the depot's origin, which a BuyVehicle
    names it by."""
    way = turn(facing, (0, 1))
    end = (node[0] + SHORT * way[0], node[1] + SHORT * way[1])
    entrance = turn(facing, ROAD_DEPOT_ENTRANCE)
    at = (end[0] - entrance[0], end[1] - entrance[1])
    action = construction_at(ROAD_DEPOT, at, name, FACING[facing])
    action["BuildConstruction"]["connection"] = polyline([end, node], {1: {"Node": "Street"}})
    return action, at


# A player's rail depot snapped onto a track's end, as the construction tool
# proposed it (captured in test mode, 2026-10-01), in the depot's own frame:
# its track graph, the first vertex the existing track's end node, which the
# depot's own node stands on. Each link: from, to, tangent0, tangent1.
RAIL_DEPOT_VERTICES = [(-2.2, -70.0), (-2.2, -65.0), (-2.2, -50.0), (6.0, -16.0), (6.0, 50.0), (-2.2, -16.0),
                       (-2.2, 25.0)]
RAIL_DEPOT_LINKS = [(0, 1, (0, 5), (0, 5)), (2, 1, (0, -15), (0, -15)), (3, 1, (0, -72), (0, -45)),
                    (3, 4, (0, 66), (0, 66)), (5, 2, (0, -34), (0, -34)), (6, 5, (0, -41), (0, -41))]
# A track lane as the tool made the depot's (simple track, 1912).
RAIL_DEPOT_LANE = {"speed": 27777, "width": 4000, "height": 530, "offset": 0, "forward": True, "modes": 16512}


def rail_depot(node, name, facing="east", template=None):
    """A rail depot beyond `node`, the free end of a track running `facing`
    into it, in the shape the construction tool proposes it: its own node on
    `node`, and its track graph as the connection."""
    template = template or TRACK
    first = turn(facing, RAIL_DEPOT_VERTICES[0])
    at = (node[0] - first[0], node[1] - first[1])
    world = []
    for v in RAIL_DEPOT_VERTICES:
        d = turn(facing, v)
        world.append((round(at[0] + d[0], 3), round(at[1] + d[1], 3)))
    vertices = [{"pos": pos(*q), "resolve": "New"} for q in world]
    vertices[0]["resolve"] = {"Node": "Track"}
    links = []
    for f, t, t0, t1 in RAIL_DEPOT_LINKS:
        w0, w1 = turn(facing, t0), turn(facing, t1)
        links.append({"from": f, "to": t,
                      "tangent0": {"x": mm(w0[0]), "y": mm(w0[1]), "z": 0},
                      "tangent1": {"x": mm(w1[0]), "y": mm(w1[1]), "z": 0},
                      "structure": "Ground",
                      "kind": {"network": "Track", "template": template, "style": TRACK_STYLE},
                      "decorations": [], "locked": False, "owned": False, "lanes": [RAIL_DEPOT_LANE],
                      "precedence": None})
    action = construction_at(RAIL_DEPOT, at, name, FACING[facing])
    action["BuildConstruction"]["connection"] = {"vertices": vertices, "links": links, "removals": [],
                                                 "removed_nodes": []}
    return action, at


def fund(s, k, t, amount=FUND):
    s.act(t + 20, k, {"Loan": {"Take": {"next": loan_terms("Small", amount),
                                        "offer": loan_terms("Small", amount)}}},
          f"Actor {k}: borrows {amount} to build and buy with (its company has no money)")


def repay(s, k, at, loan, amount=FUND):
    s.act(at, k, {"Loan": {"Repay": {"loan": loan_terms("Small", amount, {"$id": loan})}}},
          f"Actor {k}: pays back the loan {loan} (its id from the baseline)", origin="none")


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


def vehicle_op(vehicle, change):
    return {"VehicleOp": {"vehicle": {"$id": vehicle}, "change": change}}


def bulldoze_edges(network, pairs):
    return {"Bulldoze": {"Edges": {"network": network, "edges": [ends(a, b) for a, b in pairs],
                                   "buildings": []}}}


def roads():
    """Streets, a T junction, a crossroads, a bus street with two stops and a
    road depot snapped onto its end, a bus line with three buses bought at once, a track
    with a rail depot snapped to its end, a loan taken first and repaid, a
    town street no building stands by bulldozed, and a bus at no depot,
    which every game must refuse. Each actor plays for its own company (a
    competitive room)."""
    s = Scenario("roads", roads.__doc__)
    S = "Street"
    for k in range(ACTORS):
        t, c = ACTOR_GAP * k, f"Actor {k}"
        fund(s, k, t)
        s.act(t + 50, k, road([(0, 0), (120, 0)]), f"{c}: a straight street")
        s.act(t + 100, k, road([(60, 0), (60, 60)], {0: split(S, (0, 0), (120, 0))}),
              f"{c}: a T junction, splitting the street at 60 m")
        s.act(t + 150, k, road([(90, -60), (90, 0), (90, 60)], {1: split(S, (60, 0), (120, 0))}),
              f"{c}: a crossroads, splitting the street's east half at 90 m")
        # The bus line runs on a street of its own: a scripted junction has
        # no turns (the game makes no node configuration for it), so a line
        # through the T or the crossroads would have no route.
        s.act(t + 180, k, road([(20, -50), (170, -50)]), f"{c}: the bus street, south of the junctions")
        s.act(t + 200, k, place_stop(S, (20, -50), (170, -50), (70, -50)), f"{c}: a bus stop on the bus street")
        s.act(t + 230, k, place_stop(S, (20, -50), (170, -50), (140, -50)), f"{c}: a second stop on it")
        action, depot = road_depot((20, -50), f"Scenario depot {k}", "west")
        s.act(t + 260, k, action, f"{c}: a road depot west of the bus street, its entrance snapped onto the end")
        g, v = 2 * k, 3 * k
        s.act(t + 320, k, create_line(f"Scenario bus line {k}", [f"groups+{g}", f"groups+{g + 1}"], MODE_BUS),
              f"{c}: a bus line between the two stops (station groups +{g} and +{g + 1})")
        for n in range(3):
            s.act(t + 360 + n, k, buy(ROAD_DEPOT, depot, BUS), f"{c}: bus {n + 1} of 3, bought in one burst")
        s.act(t + 420, k, assign([f"vehicles+{v}", f"vehicles+{v + 1}", f"vehicles+{v + 2}"], f"lines+{k}"),
              f"{c}: the three buses onto the line")
        s.act(t + 480, k, track([(0, -80), (150, -80)]), f"{c}: a straight track south of the streets")
        s.act(t + 520, k, rail_depot((150, -80), f"Scenario rail depot {k}")[0],
              f"{c}: a rail depot on the track's east end, as the construction tool proposes it")
        s.act(t + 800, k, {"Bulldoze": {"Edges": {"network": S, "edges": [{"$edge": f"#{k}:free:0"}],
                                                  "buildings": []}}},
              f"{c}: a street of town #{k} no building stands by, bulldozed", origin="none")
        s.act(t + 900, k, buy(ROAD_DEPOT, (-500, -500), BUS), f"{c}: a bus at no depot: refused", "refused")
        repay(s, k, t + 1000, f"loans+{k}")
    s.write()


def road_upgrades():
    """A curved street, a straight one upgraded with a bus lane and then with
    a tram track, the curved street bulldozed, a town building demolished,
    and a headquarters, a second one refused."""
    s = Scenario("road_upgrades", road_upgrades.__doc__)
    S = "Street"
    for k in range(ACTORS):
        t, c = ACTOR_GAP * k, f"Actor {k}"
        fund(s, k, t)
        s.act(t + 50, k, {"BuildRoad": {"street": COUNTRY_STREET, "style": None, "bus_lane": False,
                                        "tram": "None", "polyline": curve((0, 0), (100, 40), 30)}},
              f"{c}: a curved street")
        s.act(t + 100, k, road([(0, -40), (100, -40)]), f"{c}: a straight street to upgrade")
        s.act(t + 150, k, rebuilt(road([(0, -40), (100, -40)], bus_lane=True), "BuildRoad", S,
                                  (0, -40), (100, -40)),
              f"{c}: the straight street rebuilt with a bus lane (an upgrade)")
        s.act(t + 200, k, rebuilt(road([(0, -40), (100, -40)], street=TRAM_STREET, tram="Plain"),
                                  "BuildRoad", S, (0, -40), (100, -40)),
              f"{c}: rebuilt again as a tram street")
        s.act(t + 260, k, bulldoze_edges(S, [((0, 0), (100, 40))]), f"{c}: the curved street bulldozed")
        s.act(t + 320, k, {"Bulldoze": {"Construction": {"$building": f"#{k}:0"}}},
              f"{c}: the town building nearest town #{k}'s centre demolished", origin="none")
        s.act(t + 380, k, construction(HQ, (150, 0), f"Scenario HQ {k}"), f"{c}: the company's headquarters")
        s.act(t + 440, k, construction(HQ, (150, 80), f"Scenario HQ {k} again"),
              f"{c}: a second headquarters: refused (one a company)", "refused")
        repay(s, k, t + 600, f"loans+{k}")
    s.write()


def rail():
    """A track with a signal on it and a depot snapped to its end, the track
    electrified, a train bought, stopped and started, sent to the depot,
    the signalled edge bulldozed; then, after every actor, a modular rail
    station placed without its modules, and a train line through it."""
    s = Scenario("rail", rail.__doc__)
    T = "Track"
    for k in range(ACTORS):
        t, c = ACTOR_GAP * k, f"Actor {k}"
        fund(s, k, t)
        s.act(t + 50, k, track([(0, 0), (100, 0), (200, 0)]), f"{c}: a straight track in two edges")
        s.act(t + 100, k, place_stop(T, (0, 0), (100, 0), (50, 0), model=SIGNAL, kind="Signal"),
              f"{c}: a signal on the track's west edge")
        action, depot = rail_depot((200, 0), f"Scenario rail depot {k}")
        s.act(t + 200, k, action, f"{c}: a rail depot on the track's east end, as the construction tool proposes it")
        s.act(t + 260, k, rebuilt(track([(100, 0), (200, 0)], kind=TRACK_CATENARY, catenary=True),
                                  "BuildTrack", T, (100, 0), (200, 0)),
              f"{c}: the east edge electrified (an upgrade)")
        s.act(t + 320, k, {"BuyVehicle": {
            "depot": depot_ref(RAIL_DEPOT, depot),
            "consist": [{"model": m, "reversed": False, "loads": [], "color": {"r": 0, "g": 0, "b": 0}}
                        for m in (LOCOMOTIVE, WAGGON)],
            "groups": [1, 1], "multiple_units": ["", ""]}},
              f"{c}: a locomotive and a waggon")
        s.act(t + 380, k, vehicle_op(f"vehicles+{k}", {"Stop": True}), f"{c}: the train stopped", origin="none")
        s.act(t + 400, k, vehicle_op(f"vehicles+{k}", {"Stop": False}), f"{c}: and started again", origin="none")
        s.act(t + 440, k, vehicle_op(f"vehicles+{k}", {"ToDepot": {"sell": False}}),
              f"{c}: sent to its depot", origin="none")
        s.act(t + 560, k, bulldoze_edges(T, [((0, 0), (100, 0))]),
              f"{c}: the track's west edge bulldozed, its signal with it")
        repay(s, k, t + 700, f"loans+{k}")
    # After every actor's part, so no actor's ids depend on it.
    end = ACTOR_GAP * ACTORS
    for k in range(ACTORS):
        c = f"Actor {k}"
        s.act(end + 50 + 10 * k, k, construction(RAIL_STATION, (100, 30), f"Scenario station {k}"),
              f"{c}: a modular rail station without its modules (any: what the game makes of it)", "any")
        s.act(end + 300 + 10 * k, k, create_line(f"Scenario train line {k}", [f"groups+{k}", f"groups+{k}"],
                                                 MODE_TRAIN),
              f"{c}: a train line at that station (any: the station may not be)", "any")
    s.write()


def road_vehicles():
    """Trucks: two stops, a depot on the street's free end, three trucks
    bought at once onto a line, the line renamed and recoloured, the vehicle
    window's orders, one truck replaced, one sent to be sold, one sold, the
    line deleted, and another company's truck sold, refused; then a modular
    truck station without its modules."""
    s = Scenario("road_vehicles", road_vehicles.__doc__)
    S = "Street"
    for k in range(ACTORS):
        t, c = ACTOR_GAP * k, f"Actor {k}"
        fund(s, k, t)
        s.act(t + 50, k, road([(0, 0), (150, 0)]), f"{c}: a street")
        s.act(t + 100, k, place_stop(S, (0, 0), (150, 0), (40, 0)), f"{c}: a truck stop (a street stop)")
        s.act(t + 130, k, place_stop(S, (0, 0), (150, 0), (110, 0)), f"{c}: a second stop on the same street")
        # On the street's own end, so no scripted junction (which has no
        # turns) lies between the depot and the stops.
        action, depot = road_depot((0, 0), f"Scenario truck depot {k}", "west")
        s.act(t + 200, k, action, f"{c}: a road depot west of the street, snapped onto its end")
        g, v = 2 * k, 3 * k
        s.act(t + 260, k, create_line(f"Scenario truck line {k}", [f"groups+{g}", f"groups+{g + 1}"], MODE_TRUCK),
              f"{c}: a truck line between the stops")
        for n in range(3):
            s.act(t + 300 + n, k, buy(ROAD_DEPOT, depot, TRUCK), f"{c}: truck {n + 1} of 3, in one burst")
        s.act(t + 360, k, assign([f"vehicles+{v}", f"vehicles+{v + 1}", f"vehicles+{v + 2}"], f"lines+{k}"),
              f"{c}: the trucks onto the line")
        line = {"$id": f"lines+{k}"}
        s.act(t + 400, k, {"EditLine": {"line": line, "change": {"Rename": f"Renamed line {k}"}}},
              f"{c}: the line renamed", origin="none")
        s.act(t + 420, k, {"EditLine": {"line": line, "change": {"Recolor": {"r": 950000, "g": 720000, "b": 80000}}}},
              f"{c}: the line recoloured", origin="none")
        for n, change in enumerate([{"Stop": True}, {"Stop": False}, "Reverse", "Depart",
                                    {"ManualDeparture": True}, {"ManualDeparture": False}]):
            # A truck has no reverse, and departs only from a terminal.
            maybe = change in ("Reverse", "Depart")
            s.act(t + 460 + 10 * n, k, vehicle_op(f"vehicles+{v}", change),
                  f"{c}: the vehicle window's {json.dumps(change)}" + (" (any: where the truck is)" if maybe else ""),
                  "any" if maybe else "applied", origin="none")
        s.act(t + 560, k, {"ReplaceVehicle": {
            "vehicle": {"$id": f"vehicles+{v + 1}"},
            "consist": [{"part": {"model": TRUCK, "reversed": False, "loads": [],
                                  "color": {"r": 0, "g": 0, "b": 0}}, "kept": 0}],
            "groups": [1], "multiple_units": [""]}},
              f"{c}: truck 2's consist replaced by its own part, kept", origin="none")
        s.act(t + 600, k, vehicle_op(f"vehicles+{v + 2}", {"ToDepot": {"sell": True}}),
              f"{c}: truck 3 sent to the depot to be sold", origin="none")
        s.act(t + 640, k, {"SellVehicle": {"vehicles": [{"$id": f"vehicles+{v}"}]}}, f"{c}: truck 1 sold",
              origin="none")
        s.act(t + 700, k, {"EditLine": {"line": line, "change": "Delete"}}, f"{c}: the line deleted", origin="none")
        if k > 0:
            s.act(t + 740, k, {"SellVehicle": {"vehicles": [{"$id": "vehicles+1"}]}},
                  f"{c}: actor 0's truck 2 sold: refused (another company's)", "refused", origin="none")
        repay(s, k, t + 800, f"loans+{k}")
    end = ACTOR_GAP * ACTORS
    for k in range(ACTORS):
        s.act(end + 50 + 10 * k, k, construction(TRUCK_STATION, (110, 60), f"Scenario truck station {k}"),
              f"Actor {k}: a modular truck station without its modules (any: what the game makes of it)", "any")
    s.write()


def water():
    """A ship depot and a harbour on the water near each town, a ship bought
    and run on a line. Every item is `any`: a depot is placed by its
    origin, not fitted to a shore, and the harbour has no modules."""
    s = Scenario("water", water.__doc__)
    for k in range(ACTORS):
        t, c, w = ACTOR_GAP * k, f"Actor {k}", f"water:#{k}:0"
        fund(s, k, t)
        s.act(t + 50, k, construction(WATER_DEPOT, (0, 0), f"Scenario shipyard {k}"),
              f"{c}: a ship depot on the water", "any", origin=w)
        s.act(t + 100, k, construction(HARBOUR, (60, 0), f"Scenario harbour {k}"),
              f"{c}: a harbour without its modules", "any", origin=w)
        s.act(t + 160, k, buy(WATER_DEPOT, (0, 0), SHIP), f"{c}: a ship", "any", origin=w)
        s.act(t + 220, k, create_line(f"Scenario ship line {k}", [f"groups+{k}", f"groups+{k}"], MODE_SHIP),
              f"{c}: a ship line at the harbour", "any")
        s.act(t + 260, k, assign([f"vehicles+{k}"], f"lines+{k}"), f"{c}: the ship onto it", "any")
        repay(s, k, t + 400, f"loans+{k}")
    s.write()


def air():
    """An airfield on a flat spot near each town, a plane bought at its
    hangar and run on a line. The airfield's params are the scenario's, so
    what follows it is `any`."""
    s = Scenario("air", air.__doc__)
    for k in range(ACTORS):
        t, c = ACTOR_GAP * k, f"Actor {k}"
        fund(s, k, t)
        s.act(t + 50, k, construction(AIRFIELD, (0, 0), f"Scenario airfield {k}"), f"{c}: an airfield", "any")
        s.act(t + 120, k, buy(AIRFIELD, (0, 0), PLANE), f"{c}: a plane at the airfield's hangar", "any")
        s.act(t + 180, k, create_line(f"Scenario air line {k}", [f"groups+{k}", f"groups+{k}"], MODE_AIRCRAFT),
              f"{c}: an air line", "any")
        s.act(t + 220, k, assign([f"vehicles+{k}"], f"lines+{k}"), f"{c}: the plane onto it", "any")
        repay(s, k, t + 400, f"loans+{k}")
    s.write()


def money():
    """Companies, loans, subsidies, ranks, prospecting and notifications:
    each actor founds a company, renames and recolours it, closes and opens
    its stations, borrows twice and repays one; a subsidy no game offers
    and a rank not reached are refused; actor 1 joins actor 0's company and
    founds its own again."""
    s = Scenario("money", money.__doc__)
    cargo_subsidy = "::/game_mechanics/subventions/deliver_cargo/deliver_cargo.res"
    for k in range(ACTORS):
        t, c = ACTOR_GAP * k, f"Actor {k}"
        me = {"$id": f"companies+{k}"}
        n = "none"
        s.act(t + 10, k, company_create(f"Money company {k}"), f"{c}: founds a company (companies+{k})", origin=n)
        s.act(t + 40, k, {"CompanyOp": {"Rename": {"company": me, "name": f"Renamed company {k}"}}},
              f"{c}: renames it", origin=n)
        s.act(t + 60, k, {"CompanyOp": {"Recolor": {"company": me,
                                                    "color": {"r": 180000, "g": 660000, "b": 270000}}}},
              f"{c}: recolours it", origin=n)
        s.act(t + 80, k, {"CompanyOp": {"ShareStations": {"company": me, "open": False}}},
              f"{c}: closes its stations to others", origin=n)
        s.act(t + 100, k, {"CompanyOp": {"StationAccess": {"company": me, "other": {"$id": "companies+0"},
                                                           "open": True}}},
              f"{c}: but opens them to actor 0's company", origin=n)
        s.act(t + 120, k, {"CompanyOp": {"ShareStations": {"company": me, "open": True}}},
              f"{c}: opens them again", origin=n)
        s.act(t + 140, k, {"CompanyOp": {"Unlock": me}}, f"{c}: unlocks it (it has no password)", origin=n)
        s.act(t + 200, k, {"Loan": {"Take": {"next": loan_terms("Small", 500_000),
                                             "offer": loan_terms("Small", 500_000)}}},
              f"{c}: a small loan (loans+{2 * k})", origin=n)
        s.act(t + 260, k, {"Loan": {"Take": {"next": loan_terms("Medium", 1_000_000),
                                             "offer": loan_terms("Medium", 1_000_000)}}},
              f"{c}: a medium loan (loans+{2 * k + 1})", origin=n)
        repay(s, k, t + 400, f"loans+{2 * k}", 500_000)
        s.act(t + 420, k, {"Loan": {"Repay": {"loan": loan_terms("Small", 999, {"$id": f"loans+{2 * k + 1}"})}}},
              f"{c}: the medium loan named with the wrong amount: refused", "refused", origin=n)
        s.act(t + 460, k, {"Subsidy": {"Accept": {"uid": 999_999, "kind": cargo_subsidy}}},
              f"{c}: accepts a subsidy no game offers: refused", "refused", origin=n)
        s.act(t + 480, k, {"Subsidy": {"Decline": {"uid": 999_998, "kind": cargo_subsidy}}},
              f"{c}: declines one no game offers: refused", "refused", origin=n)
        s.act(t + 520, k, {"ApplyRank": {"level": 15}}, f"{c}: takes rank 15, not reached: refused", "refused",
              origin=n)
        s.act(t + 560, k, {"Prospect": {"town": k, "cargo": "::/cargos/coal/coal.cargo", "industries": [],
                                        "permit": None}},
              f"{c}: prospects for coal near town id {k} (any: the company script decides)", "any", origin=n)
        s.act(t + 600, k, {"NotificationSeen": {"notification": 1}},
              f"{c}: marks a notification seen (any: there may be none 1)", "any", origin=n)
    end = ACTOR_GAP * ACTORS
    s.act(end + 10, 1, {"CompanyOp": {"Join": {"$id": "companies+0"}}}, "Actor 1: joins actor 0's company",
          origin="none")
    s.act(end + 60, 1, company_create("Money company 1 again"), "Actor 1: founds a company of its own again",
          origin="none")
    s.write()


def terraform():
    """A 16 m square of each actor's spot raised 2 m and lowered again,
    heights from the spot's own."""
    s = Scenario("terraform", terraform.__doc__)
    for k in range(ACTORS):
        t, c = ACTOR_GAP * k, f"Actor {k}"
        fund(s, k, t)
        for n, (target, before) in enumerate([(2, 0), (0, 2)]):
            s.act(t + 100 + 100 * n, k, {"Terraform": {
                "origin": {"$cell2": [0, 0]}, "cell": 4000, "columns": 4,
                "cells": [{"target": {"$z": target}, "before": {"$z": before}}] * 16}},
                  f"{c}: the square set to {target} m above the spot")
        repay(s, k, t + 400, f"loans+{k}")
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

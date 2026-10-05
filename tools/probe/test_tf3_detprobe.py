#!/usr/bin/env python3
"""Focused behavior tests for the Transport Fever 3 determinism probe.

Requires the same ``lupa`` test dependency as check_lua.py. The fixture stubs
only the game API surfaces used by the vehicle lane and invokes the probe's
real sampler, so a missing position cannot silently become an arbitrary marker.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

from lupa.lua52 import LuaRuntime


PROBE = (Path(__file__).resolve().parent / "tf3" / "tpf3mp_detprobe_1"
         / "content" / "gui" / "tpf3mp_detprobe" / "detprobe.script.lua")

LUA_STUBS = r"""
PROBE_LINES = {}
TEST = {}
os.getenv = function() return nil end
print = function() end
tpf3mp_native = { log = function(message) table.insert(PROBE_LINES, message) end }

react = {
  RegisterPluginRecipe = function(_, _, factory) return factory() end,
  onStep = function(callback) PROBE_FRAME = callback end,
}
builtin = {
  type = { Orientation = { Horizontal = 1 } },
  BoxLayout = function(spec) return spec end,
}
game_bar_widgets = { GameBarInfoDisplayExtension = {} }
ug_require = function(name)
  if name == "::/gui/main/react.lua" then return react end
  if name == "::/gui/main/builtin.lua" then return builtin end
  if name == "::/gui/game_bar/game_bar_widgets.tl" then return game_bar_widgets end
  error("unexpected require: " .. tostring(name))
end

local Components = {
  TRANSPORT_VEHICLE = "TRANSPORT_VEHICLE", CONSTRUCTION = "CONSTRUCTION",
  GAME_TIME = "GAME_TIME", MOVE_PATH_AIRCRAFT = "MOVE_PATH_AIRCRAFT",
  VEHICLE_DEPOT = "VEHICLE_DEPOT", TOWN = "TOWN", PLAYER = "PLAYER",
  SIM_PERSON = "SIM_PERSON", ACCOUNT = "ACCOUNT",
}
api = {
  type = {
    ComponentType = Components,
    enum = {
      TransportVehicleState = { IN_DEPOT = 0, EN_ROUTE = 1, AT_TERMINAL = 2 },
      Carrier = { AIR = 5 },
    },
  },
  engine = {
    getEntitiesWithComponent = function(component)
      if component == Components.TRANSPORT_VEHICLE then return TEST.vehicleIds end
      if component == Components.CONSTRUCTION then return TEST.constructionIds end
      return {}
    end,
    getComponent = function(entity, component)
      if component == Components.GAME_TIME and entity == 999999 then
        return { gameTime = TEST.step, updateCount = TEST.step }
      end
      if component == Components.TRANSPORT_VEHICLE then return TEST.vehicles[entity] end
      if component == Components.MOVE_PATH_AIRCRAFT then return TEST.aircraft[entity] end
      if component == Components.CONSTRUCTION then return TEST.constructions[entity] end
      if component == Components.VEHICLE_DEPOT then return TEST.vehicleDepots[entity] end
      return nil
    end,
    util = {
      vehicle = { getPosition = function(entity) return TEST.positions[entity] end },
      getWorld = function() return 999999 end,
      finance = { getPlayersBalance = function() return {} end },
    },
    system = {
      streetSystem = { getNode2SegmentMap = function() return {} end },
      townBuildingSystem = { getTown2BuildingMap = function() return {} end },
      simPersonSystem = { getCount = function() return 0 end },
    },
  },
}
game = { interface = { getEntity = function() return {} end } }
"""


def _matrix(x: float, y: float, z: float) -> list[float]:
    return [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, x, y, z, 1]


def _fixture(*, vehicle_id: int, construction_id: int, depot_id: int,
             depot_state: int = 0, position: dict | None = None,
             flight_state: int | None = None, aircraft_component: bool = False,
             depots: list[int] | None = None,
             subconstructions: list[int] | None = None,
             vehicle_depots: list[int] | None = None,
             construction_position: tuple[float, float, float] = (120.0, 240.0, 0.0)) -> dict:
    aircraft = {}
    if flight_state is not None or aircraft_component:
        aircraft[vehicle_id] = {"aircraftState": {}}
        if flight_state is not None:
            aircraft[vehicle_id]["aircraftState"]["flightState"] = flight_state
    return {
        "step": 0,
        "vehicleIds": [vehicle_id],
        "constructionIds": [construction_id],
        "vehicles": {
            vehicle_id: {
                "carrier": 5,
                "state": depot_state,
                "depot": depot_id,
            }
        },
        "aircraft": aircraft,
        "positions": {vehicle_id: position} if position is not None else {},
        "vehicleDepots": {entity: {} for entity in (vehicle_depots or [])},
        "constructions": {
            construction_id: {
                "fileName": "stations/air/airfield.con",
                "transf": _matrix(*construction_position),
                "depots": depots if depots is not None else [depot_id],
                "subconstructions": subconstructions or [],
            }
        },
    }


def _sample(fixture: dict) -> tuple[str, str]:
    lua = LuaRuntime(unpack_returned_tuples=True)
    lua.execute(LUA_STUBS)
    lua.globals().TEST = lua.table_from(fixture, recursive=True)
    lua.execute(PROBE.read_text(encoding="utf-8"))
    lua.execute("data(); TEST.step = 0; PROBE_FRAME(); TEST.step = 100; PROBE_FRAME()")
    lines = lua.globals().PROBE_LINES
    sample = next(lines[i] for i in range(1, len(lines) + 1) if "step=100 " in lines[i])
    match = re.search(r"\bp=([^ ]+)", sample)
    if not match:
        raise AssertionError(f"sample has no p lane: {sample}")
    return sample, match.group(1)


class Tf3DeterminismProbeTests(unittest.TestCase):
    def test_parked_aircraft_with_nil_group_name_uses_stable_depot_location(self) -> None:
        first, first_lane = _sample(_fixture(vehicle_id=101, construction_id=201, depot_id=301))
        second, second_lane = _sample(_fixture(vehicle_id=901, construction_id=801, depot_id=701))
        with_empty_aircraft_state = _fixture(vehicle_id=401, construction_id=501, depot_id=601,
                                             aircraft_component=True)
        _, empty_state_lane = _sample(with_empty_aircraft_state)

        self.assertRegex(first, r"\bv=1 p=[0-9]+-[0-9]+")
        self.assertEqual(first_lane, second_lane)
        self.assertEqual(first_lane, empty_state_lane)

    def test_airport_hangar_subconstruction_order_resolves_second_depot(self) -> None:
        first = _fixture(vehicle_id=101, construction_id=201, depot_id=303,
                         depots=[301], subconstructions=[450, 302, 303],
                         vehicle_depots=[302, 303])
        second = _fixture(vehicle_id=901, construction_id=801, depot_id=903,
                          depots=[901], subconstructions=[850, 902, 903],
                          vehicle_depots=[902, 903])
        duplicate_direct = _fixture(vehicle_id=401, construction_id=501, depot_id=601,
                                    depots=[601], subconstructions=[650, 602, 603, 601],
                                    vehicle_depots=[601, 602, 603])
        direct_only = _fixture(vehicle_id=601, construction_id=701, depot_id=801,
                               depots=[801], subconstructions=[850])
        _, first_lane = _sample(first)
        _, second_lane = _sample(second)
        _, duplicate_lane = _sample(duplicate_direct)
        _, direct_lane = _sample(direct_only)
        self.assertEqual(first_lane, second_lane)
        self.assertEqual(duplicate_lane, direct_lane)

        other_hangar = _fixture(vehicle_id=101, construction_id=201, depot_id=302,
                                depots=[301], subconstructions=[450, 302, 303],
                                vehicle_depots=[302, 303])
        _, other_hangar_lane = _sample(other_hangar)
        self.assertNotEqual(first_lane, other_hangar_lane)

    def test_airborne_aircraft_hashes_flight_state_and_world_coordinates(self) -> None:
        base = _fixture(vehicle_id=101, construction_id=201, depot_id=301,
                        depot_state=1, flight_state=4,
                        position={"x": 12.4, "y": 56.7, "z": 1200.2})
        _, base_lane = _sample(base)
        changed_position = _fixture(vehicle_id=901, construction_id=801, depot_id=701,
                                    depot_state=1, flight_state=4,
                                    position={"x": 13.4, "y": 56.7, "z": 1200.2})
        _, position_lane = _sample(changed_position)
        changed_flight = _fixture(vehicle_id=901, construction_id=801, depot_id=701,
                                  depot_state=1, flight_state=5,
                                  position={"x": 12.4, "y": 56.7, "z": 1200.2})
        _, flight_lane = _sample(changed_flight)

        self.assertRegex(base_lane, r"^[0-9]+-[0-9]+$")
        self.assertNotEqual(base_lane, position_lane)
        self.assertNotEqual(base_lane, flight_lane)

    def test_unknown_or_non_depot_missing_position_stays_an_error(self) -> None:
        unknown_depot = _fixture(vehicle_id=101, construction_id=201, depot_id=999,
                                 depots=[301])
        active_without_position = _fixture(vehicle_id=101, construction_id=201, depot_id=301,
                                           depot_state=1, flight_state=4)
        active_without_flight_state = _fixture(vehicle_id=101, construction_id=201, depot_id=301,
                                               depot_state=1, aircraft_component=True)
        self.assertIn(" p=err ", _sample(unknown_depot)[0])
        self.assertIn(" p=err ", _sample(active_without_position)[0])
        self.assertIn(" p=err ", _sample(active_without_flight_state)[0])


if __name__ == "__main__":
    unittest.main()

//! The mod's answers to an AI player's questions (`tpf3mp/ai.lua`) read
//! back through the converter the hook uses (`tpf3mp_proto::lua::from_lua`):
//! the tables `ai.lua` writes are exactly what `tpf3mp_bridge::ai` names.
//! A stand-in for the game's API holds one company with a station, a depot,
//! a line and a bus; two towns; two streets; and two bus models.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use mlua::{Lua, Value};
use tpf3mp_bridge::ai::AiAnswer;
use tpf3mp_proto::lua::LuaValue;

fn tree(value: &Value) -> LuaValue {
    match value {
        Value::Nil => LuaValue::Nil,
        Value::Boolean(b) => LuaValue::Boolean(*b),
        Value::Integer(n) => LuaValue::Integer(*n),
        Value::Number(n) => LuaValue::Number(*n),
        Value::String(s) => LuaValue::String(s.as_bytes().to_vec()),
        Value::Table(t) => LuaValue::Table(
            t.pairs::<Value, Value>()
                .map(|pair| {
                    let (k, v) = pair.unwrap();
                    (tree(&k), tree(&v))
                })
                .collect(),
        ),
        other => panic!("not an answer table: {other:?}"),
    }
}

fn scripts() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mod/tpf3mp_1/content/scripts/tpf3mp")
}

const GAME: &str = r#"
    -- Each node's street edges.
    SEGMENTS = { [402] = { 410 }, [403] = { 410 }, [950] = { 900 }, [951] = { 900, 960 },
                 [952] = { 901, 961 }, [953] = { 901 }, [71] = { 970 }, [72] = { 971 } }
    local CT = { CONSTRUCTION = 1, STATION_GROUP = 2, STATION = 3, BASE_NODE = 4, LINE = 5, NAME = 6,
        TRANSPORT_VEHICLE = 7, TOWN_BUILDING = 8, TOWN = 9, BASE_EDGE = 10, BASE_EDGE_STREET = 11,
        PLAYER_OWNED = 12, EDGE_OBJECT = 13, BASE_NODE_CONFIG = 14, VEHICLE_DEPOT = 15, INDUSTRY = 16 }
    local COMP = {
        [CT.CONSTRUCTION] = {
            [300] = { fileName = "stations/street/small_stops/small_new_twosided.con",
                transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, 100.5,200.25,10,1 }, stations = { 301 }, depots = {} },
            [1001] = { fileName = "::/industries/farm/farm.con", transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, 6484,-6720,20,1 },
                stations = { 1003 } },
            [1011] = { fileName = "::/industries/livestock_farm/livestock_farm.con",
                transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, 4964,-648,30,1 } },
            [400] = { fileName = "::/depots/road/road_depot/road_depot.con",
                transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, 500,600,12,1 }, stations = {}, depots = { 401 },
                -- As the game has a road depot: its apron's inner end
                -- frozen, its frozen entrance edge from there to the
                -- entrance (build 40425).
                frozenNodes = { 402 }, frozenEdges = { 410 } },
        },
        [CT.STATION_GROUP] = { [310] = { stations = { 301 } }, [320] = { stations = { 321 } } },
        -- Its depot, and another company's.
        [CT.VEHICLE_DEPOT] = { [401] = {}, [431] = {} },
        [CT.INDUSTRY] = { [1000] = { construction = 1001, stockList = 1002 },
                          [1010] = { construction = 1011, stockList = 1012 } },
        [CT.STATION] = { [301] = { terminals = { {}, {} } }, [321] = { terminals = { {}, {} } },
            [1003] = { terminals = { {}, {} } } },
        -- A street stop: an object on its edge, in no construction.
        [CT.EDGE_OBJECT] = { [321] = { param = 0.5, transf = { 1,0,0,0, 0,1,0,0, 0,0,1,0, 6267.5,-5560,8,1 } } },
        [CT.BASE_NODE] = { [402] = { position = { x = 500, y = 580, z = 12 } },
                           [403] = { position = { x = 500, y = 565, z = 12 } } },
        [CT.LINE] = { [500] = { stops = { { stationGroup = 310 }, { stationGroup = 310 } } } },
        [CT.NAME] = { [500] = { name = "Line 1" }, [700] = { name = "Didcot" }, [701] = { name = "Ely" } },
        [CT.TRANSPORT_VEHICLE] = { [600] = { transportVehicleConfig = { vehicles = { { part = { modelId = 9 } } } } } },
        [CT.TOWN_BUILDING] = { [800] = { town = 701 }, [801] = { town = 701 }, [802] = { town = 700 } },
        -- Ely's commercial buildings take goods.
        [CT.TOWN] = { [700] = {}, [701] = { cargoNeeds = { {}, { { 2, 1.0 } }, {} } } },
        [CT.BASE_EDGE] = {
            [410] = { node0 = 403, node1 = 402 },
            [900] = { position0 = { x = 0, y = 0, z = 1 }, position1 = { x = 30, y = 40, z = 1 }, node0 = 950, node1 = 951,
                roadTemplate = "::/street/town_small.street_template", objects = {}, type = 0 },
            [901] = { position0 = { x = 1000, y = 0, z = 0 }, position1 = { x = 1100, y = 0, z = 0 }, node0 = 952, node1 = 953,
                tangent0 = { x = 100, y = 80, z = 0 }, tangent1 = { x = 100, y = -80, z = 0 },
                roadTemplate = "::/street/country.street_template", objects = { {1, 0} }, type = 1 },
        },
        [CT.BASE_EDGE_STREET] = { [900] = {}, [901] = {} },
        -- Junctions with lane settings: 951, where only the town's edges
        -- meet; 952, where a company's (901) does.
        [CT.BASE_NODE_CONFIG] = { [951] = {}, [952] = {} },
        [CT.PLAYER_OWNED] = { [310] = { player = 77 }, [320] = { player = 77 }, [401] = { player = 77 },
            [901] = { player = 77 } },
    }
    local MODELS = {
        ["vehicle/bus/old.mdl"] = { id = 8, metadata = { transportVehicle = { transportModes = { 3 },
            compartments = { { loadConfigs = { { cargoEntry = { capacity = 20 } } } } } },
            availability = { yearFrom = 1900, yearTo = 1950 }, cost = { price = 30000 },
            maintenance = { runningCosts = 4000, lifespan = 20 }, roadVehicle = { topSpeed = 12.5 } } },
        ["vehicle/bus/new.mdl"] = { id = 9, metadata = { transportVehicle = { transportModes = { 3 },
            compartments = { { loadConfigs = { { cargoEntry = { capacity = 32 } } } } } },
            availability = { yearFrom = 1920, yearTo = 0 }, cost = { price = 52000 },
            maintenance = { runningCosts = 6000, lifespan = 25 }, roadVehicle = { topSpeed = 15 } } },
        ["vehicle/truck/t.mdl"] = { id = 10, metadata = { transportVehicle = { transportModes = { 4 },
            compartments = {} }, availability = { yearFrom = 1900, yearTo = 0 }, cost = { price = 1 },
            maintenance = {}, roadVehicle = { topSpeed = 10 } } },
        -- Trucks by cargo class, as the game's files give them.
        ["vehicle/truck/bulk.mdl"] = { id = 11, metadata = { transportVehicle = { transportModes = { 4 },
            compartments = { { loadConfigs = { { cargoEntry = { capacity = 12, cargoTypeSet = {
                cargoClassesIncluded = { "BULK" }, cargoClassesExcluded = {},
                cargoTypesIncluded = {}, cargoTypesExcluded = {} } } } } } } },
            availability = { yearFrom = 1900, yearTo = 0 }, cost = { price = 20000 },
            maintenance = { runningCosts = 3000, lifespan = 20 }, roadVehicle = { topSpeed = 8 } } },
        ["vehicle/truck/box.mdl"] = { id = 12, metadata = { transportVehicle = { transportModes = { 4 },
            compartments = { { loadConfigs = { { cargoEntry = { capacity = 9, cargoTypeSet = {
                cargoClassesIncluded = { "GOODS" }, cargoClassesExcluded = {},
                cargoTypesIncluded = {}, cargoTypesExcluded = {} } } } } } } },
            availability = { yearFrom = 1900, yearTo = 0 }, cost = { price = 21000 },
            maintenance = { runningCosts = 3000, lifespan = 20 }, roadVehicle = { topSpeed = 8 } } },
    }
    -- Cargo types: grain is bulk; goods are goods.
    local CARGOS = { [1] = { name = "::/cargos/grain/grain.cargo", classes = { "UNIVERSAL", "BULK" } },
                     [2] = { name = "::/cargos/goods/goods.cargo", classes = { "UNIVERSAL", "GOODS" } } }
    -- Industries: a farm that made 104 grain of 107, a livestock farm that
    -- takes grain; their stock lists by entity.
    STOCKS = { [1002] = { {}, { 1 } }, [1012] = { { 1 }, {} } }
    ROAD_JOINS = true
    api = {
        type = { ComponentType = CT, Vec2f = { new = function(x, y) return { x = x, y = y } end },
                 enum = { TransportMode = { BUS = 3, TRUCK = 4 } } },
        engine = {
            getEntitiesWithComponent = function(ct)
                if NO_SCAN and ct == CT.CONSTRUCTION then error("a scan of every construction") end
                local out = {}
                for e in pairs(COMP[ct] or {}) do out[#out + 1] = e end
                table.sort(out)
                return out
            end,
            getComponent = function(e, ct) return COMP[ct] and COMP[ct][e] end,
            util = {
                getYear = function() return 1925 end,
                finance = { getPlayersBalance = function(e) return e == 77 and 1234567.6 or 0 end },
                line = { getLineIssues = function() return { {}, {} } end },
                town = { getTownDistrictCenter = function(t, use)
                    return { x = t * 10 + use, y = 1, z = 2 } end },
                octree = { findEntitiesInCircle = function() return { 901, 900 } end,
                    -- Each place's network nodes: an industry's own (80,
                    -- no street of it) first, then a street's.
                    findTransportNetworkNodesInCircle = function(p)
                        return { { entity = 80 }, { entity = p.x > 5000 and 71 or 72 } } end },
                pathfinding = { findPathNodeToNode = function(a, b, modes)
                    -- A path only between street nodes.
                    for _, n in ipairs(a) do if n.entity == 80 then return {} end end
                    if ROAD_JOINS and a[1].entity ~= b[1].entity and modes[1] == 4 then return { 1, 2, 3 } end
                    return {} end },
                getEntityName = function(e) return ({ [1000] = "Hunstanton Crop Farm",
                    [1010] = "Clun Livestock Farm" })[e] end,
                stock = {
                    getInputsOutputsFromRules = function(list) return STOCKS[list] end,
                    getCargoProducedPerYear = function(list, id) return list == 1002 and id == 1 and 104.4 or 0 end,
                    getCargoMaxProductionPerYear = function(list, id) return list == 1002 and id == 1 and 107 or 0 end,
                },
            },
            system = {
                lineSystem = { getLinesForPlayer = function(e) return e == 77 and { 500 } or {} end },
                -- Ely was supplied 3.5 goods of the 40.2 it may take.
                townBuildingSystem = { getCargoSupplyAndLimit = function(t)
                    return t == 701 and { [2] = { 3.5, 40.2 }, [1] = { 0, 0 } } or {} end },
                transportVehicleSystem = {
                    getLineVehicles = function(l) return l == 500 and { 600 } or {} end,
                    getDepotVehicles = function() return {} end,
                },
                streetSystem = { getNodeSegments = function(n) return SEGMENTS[n] or {} end,
                    getNodeStreetSegments = function(n) return SEGMENTS[n] or {} end },
                -- The game's index of what construction holds a part.
                stationGroupSystem = { getStationGroup = function(s) return s == 1003 and 1004 or -1 end },
                streetConnectorSystem = {
                    getConstructionEntityForDepot = function(d) return ({ [401] = 400, [431] = 430 })[d] or -1 end,
                    getConstructionEntityForStation = function(s) return s == 301 and 300 or -1 end,
                },
            },
        },
        res = { cargoTypeRep = {
            find = function(name) for id, c in pairs(CARGOS) do if c.name == name then return id end end return -1 end,
            get = function(id) return CARGOS[id] and { cargoClasses = CARGOS[id].classes } end,
            getName = function(id) return CARGOS[id] and CARGOS[id].name end,
        }, modelRep = {
            forEachModelWithMetadata = function(_, fn)
                local names = {}
                for name in pairs(MODELS) do names[#names + 1] = name end
                table.sort(names)
                for _, name in ipairs(names) do fn(name) end
            end,
            find = function(name) return MODELS[name] and MODELS[name].id end,
            get = function(id)
                for _, m in pairs(MODELS) do if m.id == id then return m end end
            end,
            getName = function(id)
                for name, m in pairs(MODELS) do if m.id == id then return name end end
            end,
        } },
    }
    SEAT = "747066336d702d61692d736561743a3a" .. string.rep("0", 31) .. "1"
    ROSTER = { next = 2, list = { { id = 0, entity = 25, name = "Company" },
        { id = 1, entity = 77, name = "Rossi Transport", founder = SEAT } },
        members = { { player = SEAT, company = 1 } },
        loanOffers = { { company = 1, availableLoans = {
            { type = "Small", amount = 500000, duration = 1.5768e10, percentage = 0.05 },
            { type = "Large", cooldownUntil = 99 },
            { type = "Medium", amount = 1000000.0, duration = 3.1536e10, percentage = 0.045 } } } },
        loans = { { company = 1, id = 1 }, { company = 2, id = 2 } } }
    REGISTRY = {
        groups = { bound = { { 4, 310 }, { 5, 320 }, { 9, 1004 } } },
        lines = { bound = { { 2, 500 } } },
        vehicles = { bound = { { 6, 600 } } },
        towns = { bound = { { 1, 700 }, { 2, 701 } } },
    }
    AI = ug_require("tpf3mp_1::/scripts/tpf3mp/ai.lua")
"#;

fn lua() -> Lua {
    let lua = Lua::new();
    let dir = scripts();
    let load = lua
        .create_function(move |lua, path: String| {
            let name = path
                .strip_prefix("tpf3mp_1::/scripts/tpf3mp/")
                .unwrap_or_else(|| panic!("{path}"));
            let source = std::fs::read_to_string(dir.join(name)).unwrap();
            lua.load(source).set_name(name).eval::<Value>()
        })
        .unwrap();
    lua.globals().set("load_module", load).unwrap();
    lua.load(
        r#"
        local loaded = {}
        function ug_require(path)
            if loaded[path] == nil then loaded[path] = load_module(path) end
            return loaded[path]
        end
        "#,
    )
    .exec()
    .unwrap();
    lua.load(GAME).exec().unwrap();
    lua
}

fn answer(lua: &Lua, question: &str) -> AiAnswer {
    let value: Value = lua
        .load(format!(
            "return AI.answer({{ id = 1, seat = SEAT, question = {question} }}, ROSTER, REGISTRY)"
        ))
        .eval()
        .unwrap_or_else(|error| panic!("{question}: {error}"));
    tpf3mp_proto::lua::from_lua::<AiAnswer>(&tree(&value))
        .unwrap_or_else(|error| panic!("{question}: {error}"))
}

#[test]
fn the_company_answer_reads_as_the_ai_reads_it() {
    let lua = lua();
    let AiAnswer::Company(company) = answer(&lua, "'Company'") else {
        panic!("not a company")
    };
    assert_eq!(company.company, Some(1));
    assert_eq!(company.money, 1_234_568, "whole units, rounded");
    assert_eq!(company.year, 1925);
    assert_eq!(company.stations.len(), 2);
    let stop = &company.stations[1];
    assert_eq!(stop.id.0, 5);
    assert_eq!(
        (stop.pos.x, stop.pos.y),
        (6_267_500, -5_560_000),
        "a street stop by its edge object"
    );
    let station = &company.stations[0];
    assert_eq!(station.id.0, 4);
    assert_eq!(
        (station.pos.x, station.pos.y),
        (100_500, 200_250),
        "metres to mm"
    );
    assert_eq!(station.terminals, 2);
    assert_eq!(company.depots.len(), 1);
    let depot = &company.depots[0];
    assert_eq!(
        depot.file.as_str(),
        "::/depots/road/road_depot/road_depot.con"
    );
    let gate = depot.entrance.unwrap();
    assert_eq!(
        (gate.x, gate.y),
        (500_000, 565_000),
        "its farthest frozen node"
    );
    assert!(!depot.connected, "only its own edge meets the entrance");
    // Snapped by the game's refresh: the entrance is a junction of the
    // street, its halves meeting the depot's own edge there. And read
    // without a scan of every construction: the game's index finds the
    // company's own (3,470 constructions took up to 18 ms, 2026-10-29).
    lua.load("SEGMENTS[403] = { 410, 411, 412 } NO_SCAN = true")
        .exec()
        .unwrap();
    let AiAnswer::Company(company) = answer(&lua, "'Company'") else {
        panic!("not a company")
    };
    let depot = &company.depots[0];
    assert!(depot.connected, "the street meets its entrance");
    let gate = depot.entrance.unwrap();
    assert_eq!((gate.x, gate.y), (500_000, 565_000), "where it meets it");
    assert_eq!(company.depots.len(), 1, "its own depot only");
    assert_eq!(
        (company.stations[0].pos.x, company.stations[0].pos.y),
        (100_500, 200_250),
        "a station by its construction"
    );
    let line = &company.lines[0];
    assert_eq!(line.id.0, 2);
    assert_eq!(line.name.as_str(), "Line 1");
    assert_eq!(line.stops.iter().map(|s| s.0).collect::<Vec<_>>(), [4, 4]);
    assert_eq!(line.vehicles.iter().map(|v| v.0).collect::<Vec<_>>(), [6]);
    assert_eq!(line.problems, 2);
    let offers: Vec<(&str, i64, i64)> = company
        .offers
        .iter()
        .map(|o| (o.kind.as_str(), o.amount, o.percentage))
        .collect();
    assert_eq!(
        offers,
        [("Small", 500_000, 50_000), ("Medium", 1_000_000, 45_000)],
        "the offers not cooling down, interest in millionths"
    );
    assert_eq!(company.loans, 1, "its own loans only");
    let bus = &company.vehicles[0];
    assert_eq!(bus.id.0, 6);
    assert_eq!(bus.line.map(|l| l.0), Some(2));
    assert_eq!(bus.model.as_str(), "vehicle/bus/new.mdl");
}

#[test]
fn towns_streets_and_models_read_as_the_ai_reads_them() {
    let lua = lua();
    let AiAnswer::Towns(towns) = answer(&lua, "'Towns'") else {
        panic!("not towns")
    };
    let names: Vec<&str> = towns.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Ely", "Didcot"], "the larger first");
    assert_eq!(towns[0].buildings, 2);
    assert_eq!(towns[0].centres.len(), 3);
    assert_eq!(towns[0].pos.x, 7_010_000);
    // What Ely needs, at its commercial centre; not grain, which it may not
    // take; Didcot needs nothing.
    assert_eq!(towns[0].needs.len(), 1, "{:?}", towns[0].needs);
    let goods = &towns[0].needs[0];
    assert_eq!(goods.cargo.as_str(), "::/cargos/goods/goods.cargo");
    assert_eq!((goods.supply, goods.limit), (3, 40));
    assert_eq!(goods.at.x, 7_011_000, "the commercial district's centre");
    assert!(towns[1].needs.is_empty());

    let AiAnswer::Streets(streets) = answer(
        &lua,
        "{ Streets = { at = { x = 0, y = 0, z = 0 }, radius = 2000 } }",
    ) else {
        panic!("not streets")
    };
    assert_eq!(streets.len(), 2);
    assert_eq!(
        streets[0].length, 50,
        "the nearer first, its length in metres"
    );
    assert_eq!(
        streets[0].template.as_str(),
        "::/street/town_small.street_template"
    );
    assert!(!streets[0].owned && !streets[0].structure);
    assert!(streets[1].owned && streets[1].structure);
    assert_eq!(streets[1].objects, 1);
    // A split of the town's edge rebuilds only the town's junction; of the
    // company's, a junction a company's edge meets.
    assert!(!streets[0].company_junction);
    assert!(streets[1].company_junction);
    // Its curve's middle, where a stop or a split goes: the chord's on a
    // straight edge, off it on a bent one (the Hermite curve at one half).
    assert_eq!(
        (streets[0].mid.x, streets[0].mid.y, streets[0].mid.z),
        (15_000, 20_000, 1_000)
    );
    assert_eq!(
        (streets[1].mid.x, streets[1].mid.y, streets[1].mid.z),
        (1_050_000, 20_000, 0)
    );

    let AiAnswer::Models(models) = answer(&lua, "{ Models = { mode = 'BUS' } }") else {
        panic!("not models")
    };
    let names: Vec<&str> = models.iter().map(|m| m.model.as_str()).collect();
    assert_eq!(
        names,
        ["vehicle/bus/new.mdl", "vehicle/bus/old.mdl"],
        "buses only, of this year"
    );
    assert_eq!((models[0].capacity, models[0].speed), (32, 54));
    assert_eq!(
        (models[0].price, models[0].running, models[0].lifespan),
        (52_000, 6_000, 25)
    );
}

#[test]
fn a_question_of_no_kind_is_an_error_not_an_answer() {
    let lua = lua();
    let err: String = lua
        .load(
            "local ok, why = pcall(AI.answer, { id = 1, seat = SEAT, question = 'Weather' }, ROSTER, REGISTRY) \
             return tostring(ok) .. ' ' .. tostring(why)",
        )
        .eval()
        .unwrap();
    assert_eq!(err, "false a question of no kind");
}

#[test]
fn serving_answers_each_question_and_logs_what_the_answers_cost() {
    let lua = lua();
    let (answered, logged): (String, String) = lua
        .load(
            r#"
            local asks = { { id = 7, seat = SEAT, question = "Towns" },
                           { id = 8, seat = SEAT, question = { Bogus = {} } } }
            local answered, logged = {}, {}
            local link = {
                aiAsk = function(self) return table.remove(asks, 1) end,
                aiAnswer = function(self, id, ok) answered[#answered + 1] = id .. ":" .. tostring(ok) end,
                log = function(self, line) logged[#logged + 1] = line end,
            }
            AI.COSTS_EVERY = 0
            local room = { roster = function() return ROSTER end, registry = REGISTRY }
            -- The first call starts the count, the second reports it.
            AI.serve(link, room, 1000)
            AI.serve(link, room, 1000)
            return table.concat(answered, ","), table.concat(logged, "|")
            "#,
        )
        .eval()
        .unwrap();
    assert_eq!(
        answered, "7:true,8:false",
        "an answer, then why there is none"
    );
    assert!(
        logged.starts_with("ai: answers in ")
            && logged.contains("Towns 1 (mean ")
            && logged.contains("Bogus 1 (mean ")
            && logged.contains("1 frame(s) answered, the slowest "),
        "{logged}"
    );
}

#[test]
fn industries_routes_and_trucks_by_cargo_read_as_the_ai_reads_them() {
    let lua = lua();
    let AiAnswer::Industries(industries) = answer(&lua, "'Industries'") else {
        panic!("not industries")
    };
    assert_eq!(industries.len(), 2);
    let farm = &industries[0];
    assert_eq!(farm.name.as_str(), "Hunstanton Crop Farm");
    assert_eq!((farm.pos.x, farm.pos.y), (6_484_000, -6_720_000));
    assert!(farm.inputs.is_empty());
    assert_eq!(
        farm.outputs[0].cargo.as_str(),
        "::/cargos/grain/grain.cargo"
    );
    assert_eq!(farm.outputs[0].index, 1, "the game's number for grain");
    assert_eq!(
        (farm.station.map(|s| s.0), farm.terminals),
        (Some(9), 2),
        "its own station, by the room's id for its group"
    );
    assert_eq!(industries[1].station, None, "no station of its own");
    assert_eq!(
        (farm.outputs[0].per_year, farm.outputs[0].most),
        (104, 107),
        "made over the last year, and at most"
    );
    assert_eq!(
        industries[1].inputs[0].as_str(),
        "::/cargos/grain/grain.cargo",
        "the livestock farm takes grain"
    );

    // Joined by road for trucks, by the pathfinder from the nodes by each.
    let route =
        "{ Route = { from = { x = 6484, y = -6720, z = 0 }, to = { x = 4964, y = -648, z = 0 } } }";
    let AiAnswer::Route(joined) = answer(&lua, route) else {
        panic!("not a route")
    };
    assert!(joined.connected && joined.edges == 3);
    lua.load("ROAD_JOINS = false").exec().unwrap();
    let AiAnswer::Route(apart) = answer(&lua, route) else {
        panic!("not a route")
    };
    assert!(!apart.connected && apart.edges == 0);

    // Trucks for grain: those whose load takes a class of grain's.
    let AiAnswer::Models(trucks) = answer(
        &lua,
        "{ Models = { mode = 'TRUCK', cargo = '::/cargos/grain/grain.cargo' } }",
    ) else {
        panic!("not models")
    };
    let names: Vec<(&str, u32)> = trucks
        .iter()
        .map(|m| (m.model.as_str(), m.capacity))
        .collect();
    assert_eq!(
        names,
        [("vehicle/truck/bulk.mdl", 12)],
        "the bulk truck only"
    );
    let AiAnswer::Models(trucks) = answer(
        &lua,
        "{ Models = { mode = 'TRUCK', cargo = '::/cargos/goods/goods.cargo' } }",
    ) else {
        panic!("not models")
    };
    assert_eq!(trucks[0].model.as_str(), "vehicle/truck/box.mdl");
    assert_eq!(trucks.len(), 1);
}

#[test]
fn verdicts_are_answered_alone_in_their_frame_and_spaced_apart() {
    let lua = lua();
    let answered: String = lua
        .load(
            r#"
            CLOCK = 100
            os.clock = function() return CLOCK end
            AI.verdict = function() return { Verdict = { ok = true, cost = 1, messages = {}, collisions = 0 } } end
            local asks = { { id = 1, seat = SEAT, question = "Towns" },
                           { id = 2, seat = SEAT, question = { Verdict = {} } },
                           { id = 3, seat = SEAT, question = { Verdict = {} } },
                           { id = 4, seat = SEAT, question = "Towns" } }
            local answered = {}
            local link = {
                aiAsk = function(self) return table.remove(asks, 1) end,
                aiAnswer = function(self, id) answered[#answered + 1] = tostring(id) end,
            }
            local room = { roster = function() return ROSTER end, registry = REGISTRY }
            local frames = {}
            local function frame() AI.serve(link, room, 1000) frames[#frames + 1] = table.concat(answered, ",") end
            frame()              -- the towns; the verdict waits for a frame of its own
            frame()              -- the first verdict, alone
            CLOCK = CLOCK + 0.1
            frame()              -- too soon for the second
            CLOCK = CLOCK + 0.2
            frame()              -- the second verdict, alone
            frame()              -- then the towns behind it
            return table.concat(frames, " | ")
            "#,
        )
        .eval()
        .unwrap();
    assert_eq!(answered, "1 | 1,2 | 1,2 | 1,2,3 | 1,2,3,4");
}

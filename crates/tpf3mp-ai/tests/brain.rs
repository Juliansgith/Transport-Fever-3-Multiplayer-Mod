//! The brain against a stand-in world: it answers the brain's questions as
//! the mod does and applies its actions as every game would, so the brain's
//! whole round, from founding its company to buses on a line, plays here.

#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use tpf3mp_ai::{Brain, Out, place::P};
use tpf3mp_bridge::ai::{
    AiAnswer, AiCompany, AiDepot, AiEnd, AiIndustry, AiLand, AiLine, AiModel, AiNeed, AiOutput,
    AiQuestion, AiRoute, AiStation, AiStreet, AiTown, AiVehicle, AiVerdict, land,
};
use tpf3mp_proto::{
    AiDifficulty, BoundedVec, FixedBytes, PlayerId, RoomId, Text,
    action::{
        Action, CompanyOp, EdgeEnds, LineId, LoanOp, LoanTerms, Pos, Resolve, StationId, Structure,
        TownId, VehicleId,
    },
};

const STEP: Duration = Duration::from_millis(100);

fn pos(x: f64, y: f64) -> Pos {
    #[allow(clippy::cast_possible_truncation)]
    Pos {
        x: (x * 1000.0) as i32,
        y: (y * 1000.0) as i32,
        z: 10_000,
    }
}

/// The world point at (`lx`, `ly`) metres on a construction's own axes.
#[allow(clippy::cast_possible_truncation)]
fn place(t: &tpf3mp_proto::action::Transform, lx: f64, ly: f64) -> Pos {
    let (cos, sin) = (f64::from(t.basis[0]) / 1e6, f64::from(t.basis[1]) / 1e6);
    Pos {
        x: t.origin.x + ((lx * cos - ly * sin) * 1000.0) as i32,
        y: t.origin.y + ((lx * sin + ly * cos) * 1000.0) as i32,
        z: t.origin.z,
    }
}

/// A world of two towns laid out as street grids, a company, and the
/// game's rules as far as the brain meets them.
struct World {
    company: Option<u32>,
    money: i64,
    offers: Vec<LoanTerms>,
    loans: u8,
    towns: Vec<AiTown>,
    streets: Vec<AiStreet>,
    stations: Vec<AiStation>,
    depots: Vec<AiDepot>,
    lines: Vec<AiLine>,
    vehicles: Vec<AiVehicle>,
    /// What the verdicts refuse.
    refuse_depot_side: Option<f64>,
    refuse_stops_in: Option<u32>,
    /// Whether a depot's refresh snaps its entrance onto the junction its
    /// connection splits into the street, as the game's does.
    snaps: bool,
    /// Depot builds the room orders but no game applies, as one whose
    /// junction is another company's (2026-10-29).
    drop_depots: u32,
    /// The world's industries (none unless a test puts some), whether
    /// trucks can drive between them, and each line's transport mode.
    industries: Vec<AiIndustry>,
    roads_join: bool,
    line_modes: Vec<(LineId, u16)>,
    /// The truck stations built.
    truck_stations: Vec<StationId>,
    /// The rail stations built, the track nodes laid (stations' ends and
    /// depots' joins among them), and the track's length laid, in metres.
    rail_stations: Vec<StationId>,
    track_nodes: Vec<Pos>,
    track_laid: f64,
    /// Rail stations refused within 600 m of here; no wagon for any cargo.
    refuse_rail_near: Option<Pos>,
    no_wagons: bool,
    /// The ground's height at a point, where it is not flat at 10 m.
    ground: Option<fn(f64, f64) -> f64>,
    /// Level crossings laid: the street edges a track's vertex split.
    crossings: Vec<EdgeEnds>,
    /// Track refused within 300 m of here.
    refuse_track_near: Option<Pos>,
    /// The structures of the track links laid.
    structures: Vec<Structure>,
    /// Vehicles replaced and sold.
    replaced: Vec<VehicleId>,
    sold: Vec<VehicleId>,
    actions: Vec<(Instant, Action)>,
    next_id: u32,
}

impl World {
    fn new() -> Self {
        let mut streets = Vec::new();
        let mut towns = Vec::new();
        for (n, (cx, cy)) in [(0.0, 0.0), (5_000.0, 0.0)].into_iter().enumerate() {
            // A 1 km grid of 100 m blocks.
            for i in -5..=5 {
                for j in -5..5 {
                    let (x, y) = (cx + f64::from(i) * 100.0, cy + f64::from(j) * 100.0);
                    streets.push(street(pos(x, y), pos(x, y + 100.0)));
                    streets.push(street(
                        pos(y - cy + cx, x - cx + cy),
                        pos(y - cy + cx + 100.0, x - cx + cy),
                    ));
                }
            }
            towns.push(AiTown {
                id: TownId(u32::try_from(n).unwrap() + 1),
                name: Text::new(["Didcot", "Ely"][n]).unwrap(),
                pos: pos(cx, cy),
                buildings: 300 - u32::try_from(n).unwrap() * 100,
                centres: BoundedVec::new(vec![
                    pos(cx + 20.0, cy + 30.0),
                    pos(cx + 320.0, cy - 280.0),
                    pos(cx - 330.0, cy + 290.0),
                ])
                .unwrap(),
                needs: BoundedVec::default(),
            });
        }
        Self {
            company: None,
            // A founded company starts with no money, and borrows.
            money: 0,
            offers: vec![loan("Small", 500_000), loan("Large", 2_000_000)],
            loans: 0,
            towns,
            streets,
            stations: Vec::new(),
            depots: Vec::new(),
            lines: Vec::new(),
            vehicles: Vec::new(),
            refuse_depot_side: None,
            refuse_stops_in: None,
            snaps: true,
            drop_depots: 0,
            industries: Vec::new(),
            roads_join: true,
            line_modes: Vec::new(),
            truck_stations: Vec::new(),
            rail_stations: Vec::new(),
            track_nodes: Vec::new(),
            track_laid: 0.0,
            refuse_rail_near: None,
            no_wagons: false,
            ground: None,
            crossings: Vec::new(),
            refuse_track_near: None,
            structures: Vec::new(),
            replaced: Vec::new(),
            sold: Vec::new(),
            actions: Vec::new(),
            next_id: 1,
        }
    }

    fn id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    fn answer(&self, question: &AiQuestion) -> AiAnswer {
        match question {
            AiQuestion::Company => AiAnswer::Company(AiCompany {
                company: self.company,
                money: self.money,
                year: 1925,
                stations: BoundedVec::new(self.stations.clone()).unwrap(),
                depots: BoundedVec::new(self.depots.clone()).unwrap(),
                lines: BoundedVec::new(self.lines.clone()).unwrap(),
                vehicles: BoundedVec::new(self.vehicles.clone()).unwrap(),
                offers: BoundedVec::new(self.offers.clone()).unwrap(),
                loans: self.loans,
            }),
            AiQuestion::Towns => AiAnswer::Towns(BoundedVec::new(self.towns.clone()).unwrap()),
            AiQuestion::Streets { at, radius } => {
                let near: Vec<AiStreet> = self
                    .streets
                    .iter()
                    // As the game's octree: an edge passing within the
                    // radius, wherever its middle is.
                    .filter(|s| {
                        let (a, b, p) = (P::of(s.a), P::of(s.b), P::of(*at));
                        let (dx, dy) = (b.x - a.x, b.y - a.y);
                        let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / (dx * dx + dy * dy))
                            .clamp(0.0, 1.0);
                        (a.x + dx * t - p.x).hypot(a.y + dy * t - p.y) <= f64::from(*radius)
                    })
                    .cloned()
                    .collect();
                AiAnswer::Streets(BoundedVec::new(near).unwrap())
            }
            AiQuestion::Models { mode, cargo } if mode.as_str() == "TRUCK" => {
                // A truck for each cargo, named by it.
                let name = cargo.as_ref().map(|c| {
                    let file = c.as_str().rsplit('/').next().unwrap_or("");
                    format!("vehicle/truck/{}.mdl", file.split('.').next().unwrap_or(""))
                });
                AiAnswer::Models(
                    BoundedVec::new(if let Some(name) = name {
                        vec![AiModel {
                            model: Text::new(&name).unwrap(),
                            price: 25_000,
                            running: 6_000,
                            capacity: 12,
                            speed: 30,
                            lifespan: 20,
                            power: 0,
                            tractive: 0,
                            weight: 0,
                            length: 0,
                            electric: false,
                        }]
                    } else {
                        Vec::new()
                    })
                    .unwrap(),
                )
            }
            AiQuestion::Industries => {
                AiAnswer::Industries(BoundedVec::new(self.industries.clone()).unwrap())
            }
            // Open land, flat at 10 m unless the test shapes it; a street
            // of the world's marks the cells it passes near.
            AiQuestion::Land { at, cells, step } => {
                let (mut heights, mut flags) = (Vec::new(), Vec::new());
                for j in 0..*cells {
                    for i in 0..*cells {
                        let x = f64::from(at.x) / 1000.0 + f64::from(i) * f64::from(*step);
                        let y = f64::from(at.y) / 1000.0 + f64::from(j) * f64::from(*step);
                        let z = self.ground.map_or(10.0, |g| g(x, y));
                        #[allow(clippy::cast_possible_truncation)]
                        heights.push((z * 100.0).round() as i32);
                        let street = self.streets.iter().any(|s| {
                            let (a, b) = (P::of(s.a), P::of(s.b));
                            let (dx, dy) = (b.x - a.x, b.y - a.y);
                            let t = (((x - a.x) * dx + (y - a.y) * dy) / (dx * dx + dy * dy))
                                .clamp(0.0, 1.0);
                            (a.x + dx * t - x).hypot(a.y + dy * t - y) <= f64::from(*step) * 0.71
                        });
                        flags.push(if street { land::STREET } else { 0 });
                    }
                }
                AiAnswer::Land(AiLand {
                    heights: BoundedVec::new(heights).unwrap(),
                    flags: BoundedVec::new(flags).unwrap(),
                })
            }
            AiQuestion::Models { mode, cargo } if mode.as_str() == "TRAIN" => {
                // A wagon for each cargo, named by it.
                let carried = cargo.as_ref().map_or("planks".to_owned(), |c| {
                    let file = c.as_str().rsplit('/').next().unwrap_or("");
                    file.split('.').next().unwrap_or("").to_owned()
                });
                let wagon = AiModel {
                    model: Text::new(format!("vehicle/train/{carried}_wagon.mdl")).unwrap(),
                    price: 100_000,
                    running: 5_000,
                    capacity: 20,
                    speed: 60,
                    lifespan: 30,
                    power: 0,
                    tractive: 0,
                    weight: 12,
                    length: 120,
                    electric: false,
                };
                let engine = AiModel {
                    model: Text::new("vehicle/train/engine.mdl").unwrap(),
                    price: 1_500_000,
                    running: 60_000,
                    capacity: 0,
                    speed: 80,
                    lifespan: 30,
                    power: 600,
                    tractive: 120,
                    weight: 80,
                    length: 180,
                    electric: false,
                };
                // Stronger, but electric: no use on track without
                // catenary.
                let electric = AiModel {
                    model: Text::new("vehicle/train/electric.mdl").unwrap(),
                    power: 1_600,
                    electric: true,
                    ..engine.clone()
                };
                AiAnswer::Models(
                    BoundedVec::new(match cargo {
                        Some(_) if self.no_wagons => Vec::new(),
                        Some(_) => vec![wagon],
                        None => vec![electric, engine, wagon],
                    })
                    .unwrap(),
                )
            }
            AiQuestion::Route { .. } => AiAnswer::Route(AiRoute {
                connected: self.roads_join,
                edges: if self.roads_join { 40 } else { 0 },
            }),
            AiQuestion::Models { .. } => AiAnswer::Models(
                BoundedVec::new(vec![
                    AiModel {
                        model: Text::new("vehicle/bus/cheap.mdl").unwrap(),
                        price: 20_000,
                        running: 9_000,
                        capacity: 18,
                        speed: 40,
                        lifespan: 20,
                        power: 0,
                        tractive: 0,
                        weight: 0,
                        length: 0,
                        electric: false,
                    },
                    AiModel {
                        model: Text::new("vehicle/bus/good.mdl").unwrap(),
                        price: 40_000,
                        running: 4_000,
                        capacity: 32,
                        speed: 55,
                        lifespan: 25,
                        power: 0,
                        tractive: 0,
                        weight: 0,
                        length: 0,
                        electric: false,
                    },
                ])
                .unwrap(),
            ),
            AiQuestion::Verdict(payload) => {
                let action = Action::from_payload(payload).unwrap();
                AiAnswer::Verdict(self.verdict(&action))
            }
        }
    }

    fn verdict(&self, action: &Action) -> AiVerdict {
        let ok = match action {
            Action::PlaceStop(stop) => {
                let on = self
                    .streets
                    .iter()
                    .find(|s| s.a == stop.edge.ends.a && s.b == stop.edge.ends.b);
                let town = self.town_of(stop.at);
                on.is_some_and(|s| s.objects == 0) && town != self.refuse_stops_in
            }
            Action::BuildConstruction(build) if build.file.as_str().contains("/rail/") => {
                self.refuse_rail_near.is_none_or(|p| {
                    let o = build.transform.origin;
                    f64::from(o.x - p.x).hypot(f64::from(o.y - p.y)) > 600_000.0
                })
            }
            // As the game: a vertex on an existing node must find one, and
            // one splitting a street an empty street there.
            Action::BuildTrack(track)
                if self.refuse_track_near.is_some_and(|p| {
                    track.polyline.vertices.iter().any(|v| {
                        f64::from(v.pos.x - p.x).hypot(f64::from(v.pos.y - p.y)) <= 300_000.0
                    })
                }) =>
            {
                false
            }
            Action::BuildTrack(track) => track.polyline.vertices.iter().all(|v| match &v.resolve {
                Resolve::Node(_) => self.track_node(v.pos).is_some(),
                Resolve::Split(e) => self
                    .streets
                    .iter()
                    .any(|s| s.a == e.ends.a && s.b == e.ends.b && s.objects == 0),
                Resolve::New => true,
            }),
            Action::BuildConstruction(build) => {
                // Its side of the street: the entrance's offset from origin.
                let [cos, sin, ..] = build.transform.basis;
                let side = (f64::from(sin), -f64::from(cos));
                let refused = self.refuse_depot_side.is_some_and(|s| {
                    // Side +1 of the grid's x streets puts the depot at
                    // +y: its -y axis then points down (0, -1).
                    (s > 0.0) == (side.1 < 0.0 || side.0 < 0.0)
                });
                // Its connection splits an empty street of the world's.
                let splits = build.connection.as_ref().is_some_and(|c| {
                    c.vertices.iter().any(|v| {
                        matches!(&v.resolve, Resolve::Split(e) if self.streets.iter().any(
                            |s| s.a == e.ends.a && s.b == e.ends.b && s.objects == 0))
                    })
                });
                !refused && splits
            }
            _ => true,
        };
        AiVerdict {
            ok,
            cost: 10_000,
            messages: if ok {
                BoundedVec::default()
            } else {
                BoundedVec::new(vec![Text::new("Collision").unwrap()]).unwrap()
            },
            collisions: 0,
        }
    }

    fn track_node(&self, at: Pos) -> Option<usize> {
        self.track_nodes
            .iter()
            .position(|n| f64::from(n.x - at.x).hypot(f64::from(n.y - at.y)) <= 1_500.0)
    }

    fn town_of(&self, at: Pos) -> Option<u32> {
        self.towns
            .iter()
            .find(|t| f64::from(t.pos.x - at.x).hypot(f64::from(t.pos.y - at.y)) < 800_000.0)
            .map(|t| t.id.0)
    }

    fn apply(&mut self, now: Instant, action: Action) {
        self.actions.push((now, action.clone()));
        match action {
            Action::CompanyOp(CompanyOp::Create { .. }) => self.company = Some(1),
            Action::Loan(op) => {
                let LoanOp::Take { offer, .. } = *op else {
                    panic!("only taking")
                };
                assert!(self.offers.contains(&offer), "an offer it was given");
                self.offers.retain(|o| *o != offer);
                self.money += offer.amount;
                self.loans += 1;
            }
            Action::PlaceStop(stop) => {
                let id = self.id();
                self.stations.push(AiStation {
                    id: StationId(id),
                    pos: stop.at,
                    terminals: 2,
                    waiting: 0,
                    ends: BoundedVec::default(),
                });
                if let Some(s) = self
                    .streets
                    .iter_mut()
                    .find(|s| s.a == stop.edge.ends.a && s.b == stop.edge.ends.b)
                {
                    s.objects = 1;
                }
                self.money -= 10_000;
            }
            Action::BuildConstruction(build)
                if build.file.as_str().contains("modular_terminal") =>
            {
                let id = self.id();
                self.truck_stations.push(StationId(id));
                self.stations.push(AiStation {
                    id: StationId(id),
                    pos: build.transform.origin,
                    terminals: 1,
                    waiting: 0,
                    ends: BoundedVec::default(),
                });
                self.money -= 60_000;
            }
            Action::BuildConstruction(build)
                if build.file.as_str()
                    == "::/stations/rail/modular_station/modular_station.con" =>
            {
                // Its track modules' slots give its first and last module;
                // its ends lie at (10, 40 first - 20) and (10, 40 last + 20)
                // on its own axes.
                let slots: Vec<i64> = build
                    .params
                    .iter()
                    .filter_map(|p| {
                        let key = p.key.as_str();
                        let slot: i64 = key
                            .strip_prefix("modules[")?
                            .split(']')
                            .next()?
                            .parse()
                            .ok()?;
                        (key.ends_with(".name") && (8_401_500..8_402_500).contains(&slot))
                            .then_some((slot - 8_402_000) / 10)
                    })
                    .collect();
                let (first, last) = (*slots.iter().min().unwrap(), *slots.iter().max().unwrap());
                let local = |lx: f64, ly: f64| place(&build.transform, lx, ly);
                #[allow(clippy::cast_precision_loss)]
                let ends = [
                    local(10.0, 40.0 * first as f64 - 20.0),
                    local(10.0, 40.0 * last as f64 + 20.0),
                ];
                let id = self.id();
                self.rail_stations.push(StationId(id));
                self.track_nodes.extend(ends);
                self.stations.push(AiStation {
                    id: StationId(id),
                    pos: build.transform.origin,
                    terminals: 1,
                    waiting: 0,
                    ends: BoundedVec::new(
                        ends.iter()
                            .map(|e| AiEnd {
                                pos: *e,
                                open: true,
                            })
                            .collect(),
                    )
                    .unwrap(),
                });
                self.money -= 230_000;
            }
            Action::BuildConstruction(build)
                if build.file.as_str() == "::/depots/rail/rail_depot.con" =>
            {
                let join = place(&build.transform, -2.2, -70.0);
                self.track_nodes.push(join);
                self.depots.push(AiDepot {
                    file: build.file,
                    at: build.transform.origin,
                    entrance: Some(join),
                    connected: false,
                });
                self.money -= 150_000;
            }
            Action::BuildTrack(track) => {
                let v = &track.polyline.vertices;
                for vertex in v.iter() {
                    match vertex.resolve {
                        Resolve::Node(_) => {
                            assert!(self.track_node(vertex.pos).is_some(), "a node there")
                        }
                        // A level crossing: the street in two at it.
                        Resolve::Split(e) => {
                            let k = self
                                .streets
                                .iter()
                                .position(|s| s.a == e.ends.a && s.b == e.ends.b)
                                .expect("the street it splits");
                            let old = self.streets.remove(k);
                            self.streets.push(street(old.a, vertex.pos));
                            self.streets.push(street(vertex.pos, old.b));
                            self.crossings.push(e.ends);
                            self.track_nodes.push(vertex.pos);
                        }
                        Resolve::New => self.track_nodes.push(vertex.pos),
                    }
                    let near = |p: Pos| {
                        f64::from(p.x - vertex.pos.x).hypot(f64::from(p.y - vertex.pos.y))
                            <= 1_500.0
                    };
                    for s in &mut self.stations {
                        let mut ends = s.ends.clone().into_inner();
                        for e in &mut ends {
                            if near(e.pos) {
                                e.open = false;
                            }
                        }
                        s.ends = BoundedVec::new(ends).unwrap();
                    }
                    for d in &mut self.depots {
                        if d.entrance.is_some_and(near) && d.file.as_str().contains("/rail/") {
                            d.connected = true;
                        }
                    }
                }
                for link in track.polyline.links.iter() {
                    self.structures.push(link.structure.clone());
                    let (a, b) = (v[usize::from(link.from)].pos, v[usize::from(link.to)].pos);
                    let len = f64::from(b.x - a.x).hypot(f64::from(b.y - a.y)) / 1000.0;
                    assert!(len <= 200.0, "an edge of {len} m");
                    self.track_laid += len;
                }
                #[allow(clippy::cast_possible_truncation)]
                let cost = (self.track_laid * 0.0) as i64 + 50_000;
                self.money -= cost;
            }
            Action::BuildConstruction(_) if self.drop_depots > 0 => self.drop_depots -= 1,
            Action::BuildConstruction(build) => {
                let [cos, sin, ..] = build.transform.basis;
                let o = build.transform.origin;
                #[allow(clippy::cast_possible_truncation)]
                let entrance = Pos {
                    x: o.x + (35.0 * f64::from(sin) / 1e6 * 1000.0) as i32,
                    y: o.y - (35.0 * f64::from(cos) / 1e6 * 1000.0) as i32,
                    z: o.z,
                };
                // The split's junction, which the refresh snaps the
                // entrance onto: 2 m from it.
                let connection = build.connection.unwrap();
                let junction = connection.vertices[0].pos;
                let gap =
                    f64::from(junction.x - entrance.x).hypot(f64::from(junction.y - entrance.y));
                assert!((gap - 2_000.0).abs() < 2.0, "{gap}");
                self.depots.push(AiDepot {
                    file: build.file,
                    at: o,
                    entrance: Some(if self.snaps { junction } else { entrance }),
                    connected: self.snaps,
                });
                self.money -= 120_000;
            }
            Action::CreateLine(create) => {
                let id = self.id();
                self.line_modes.push((LineId(id), create.line.modes[0]));
                // As the game: a truck line loads its cargo (planks, type
                // 13) at its first stop, or its trucks are useless; a bus
                // line its passengers (type 0) everywhere.
                let rules: Vec<Vec<bool>> = create
                    .line
                    .stops
                    .iter()
                    .map(|s| s.rules.load.iter().copied().collect())
                    .collect();
                if create.line.modes[0] == 4 {
                    // It loads at the industry's own station, at its output
                    // terminal (the second of two) alone, and unloads at a
                    // truck station of its own, of one terminal, or at the
                    // own station of the industry it feeds.
                    let industry_station =
                        |id: StationId| self.industries.iter().any(|i| i.station == Some(id));
                    assert!(industry_station(create.line.stops[0].group));
                    assert_eq!(create.line.stops[0].terminal.terminal, 1);
                    assert!(create.line.stops[0].alternatives.is_empty());
                    assert!(
                        self.truck_stations.contains(&create.line.stops[1].group)
                            || industry_station(create.line.stops[1].group)
                    );
                    assert_eq!(create.line.stops[1].terminal.terminal, 0);
                    assert!(create.line.stops[1].alternatives.is_empty());
                    // Passengers everywhere, as the game's line window
                    // loads them; the cargo at its maker only. Its stops
                    // wait as a road stop's in the line window.
                    assert!(rules[0].iter().skip(1).any(|l| *l), "{rules:?}");
                    assert_eq!(rules[0].first(), Some(&true), "{rules:?}");
                    assert!(
                        rules[1].iter().enumerate().all(|(i, l)| *l == (i == 0)),
                        "{rules:?}"
                    );
                    assert!(!create.line.custom_filters);
                    for stop in create.line.stops.iter() {
                        assert_eq!(
                            (stop.max_wait, stop.max_extra_wait),
                            (600_000_000, 30_000_000)
                        );
                        assert!(
                            stop.rules.destroy_for_config_change && stop.rules.destroy_for_refresh
                        );
                    }
                } else if create.line.modes[0] == 7 {
                    // A train loads its cargo at the first rail station and
                    // passengers where it may; it unloads at the second.
                    assert!(
                        create
                            .line
                            .stops
                            .iter()
                            .all(|s| self.rail_stations.contains(&s.group))
                    );
                    assert!(rules[0].iter().skip(1).any(|l| *l), "{rules:?}");
                    assert!(
                        rules[1].iter().enumerate().all(|(i, l)| *l == (i == 0)),
                        "{rules:?}"
                    );
                } else {
                    assert!(rules.iter().all(|r| r == &[true]), "{rules:?}");
                }
                self.lines.push(AiLine {
                    id: LineId(id),
                    name: Text::lossy(create.name.as_str()),
                    stops: BoundedVec::new(create.line.stops.iter().map(|s| s.group).collect())
                        .unwrap(),
                    vehicles: BoundedVec::default(),
                    load: 0,
                    waiting: 0,
                    income: 0,
                    costs: 0,
                    problems: 0,
                });
            }
            Action::BuyVehicle(buy) => {
                if buy.depot.file.as_str().contains("/rail/") {
                    // A steam engine first, then its wagons.
                    assert_eq!(buy.consist[0].model.as_str(), "vehicle/train/engine.mdl");
                    assert!(buy.consist.len() >= 2);
                    assert!(
                        buy.consist
                            .iter()
                            .skip(1)
                            .all(|p| p.model.as_str().contains("wagon"))
                    );
                    assert_eq!(buy.groups.len(), buy.consist.len());
                }
                let id = self.id();
                self.vehicles.push(AiVehicle {
                    id: VehicleId(id),
                    line: None,
                    model: buy.consist[0].model.clone(),
                    worn: 0,
                    idle: true,
                });
                self.money -= 30_000;
            }
            Action::AssignLine(assign) => {
                let line = assign.line.unwrap();
                // As the game: a vehicle runs only a line of its mode.
                let mode = self
                    .line_modes
                    .iter()
                    .find(|(id, _)| *id == line)
                    .map(|(_, m)| *m);
                for v in assign.vehicles.iter() {
                    let vehicle = self.vehicles.iter().find(|x| x.id == *v).unwrap();
                    let want = if vehicle.model.as_str().contains("truck") {
                        4
                    } else if vehicle.model.as_str().contains("train") {
                        7
                    } else {
                        3
                    };
                    assert_eq!(mode, Some(want), "{vehicle:?} on {line:?}");
                }
                for v in assign.vehicles.iter() {
                    if let Some(vehicle) = self.vehicles.iter_mut().find(|x| x.id == *v) {
                        vehicle.line = Some(line);
                        vehicle.idle = false;
                    }
                    if let Some(l) = self.lines.iter_mut().find(|l| l.id == line) {
                        let mut list = l.vehicles.clone().into_inner();
                        list.push(*v);
                        l.vehicles = BoundedVec::new(list).unwrap();
                    }
                }
            }
            // The vehicle window's replace: the same vehicle, new.
            Action::ReplaceVehicle(replace) => {
                let v = self
                    .vehicles
                    .iter_mut()
                    .find(|v| v.id == replace.vehicle)
                    .expect("a vehicle of its own");
                assert!(replace.consist.iter().all(|p| p.kept.is_none()));
                v.model = replace.consist[0].part.model.clone();
                v.worn = 0;
                self.replaced.push(replace.vehicle);
                self.money -= 25_000;
            }
            Action::SellVehicle { vehicles } => {
                for id in vehicles.iter() {
                    assert!(self.vehicles.iter().any(|v| v.id == *id), "its own");
                    self.vehicles.retain(|v| v.id != *id);
                    for l in &mut self.lines {
                        let list: Vec<VehicleId> =
                            l.vehicles.iter().copied().filter(|v| v != id).collect();
                        l.vehicles = BoundedVec::new(list).unwrap();
                    }
                    self.sold.push(*id);
                }
                self.money += 10_000;
            }
            other => panic!("the brain sent {other:?}"),
        }
    }

    fn lines_with_buses(&self, buses: usize) -> usize {
        self.lines
            .iter()
            .filter(|l| {
                self.vehicles
                    .iter()
                    .filter(|v| v.line == Some(l.id))
                    .count()
                    >= buses
            })
            .count()
    }
}

const GRAIN: &str = "::/cargos/grain/grain.cargo";
const PLANKS: &str = "::/cargos/planks/planks.cargo";

/// An industry at (x, y) metres, taking `inputs` and making `outputs`
/// (cargo, made over the last year).
fn industry(name: &str, x: f64, y: f64, inputs: &[&str], outputs: &[(&str, u32)]) -> AiIndustry {
    // Its own station's id, from its place.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let station = StationId(10_000 + (x.abs() + y.abs()) as u32);
    AiIndustry {
        name: Text::new(name).unwrap(),
        pos: pos(x, y),
        station: Some(station),
        terminals: 2,
        inputs: BoundedVec::new(inputs.iter().map(|c| Text::new(*c).unwrap()).collect()).unwrap(),
        outputs: BoundedVec::new(
            outputs
                .iter()
                .map(|(c, n)| AiOutput {
                    cargo: Text::new(*c).unwrap(),
                    index: match *c {
                        GRAIN => 7,
                        PLANKS => 13,
                        _ => 9,
                    },
                    per_year: *n,
                    most: *n,
                })
                .collect(),
        )
        .unwrap(),
        water: false,
    }
}

/// A sawmill in Didcot's south-west, a grain farm in its north-west and a
/// livestock farm that takes grain in its north-east, and a quarry by Ely.
fn mills() -> Vec<AiIndustry> {
    vec![
        industry("Witney Sawmill", -300.0, -200.0, &[], &[(PLANKS, 120)]),
        industry("Witney Farm", -400.0, 300.0, &[], &[(GRAIN, 120)]),
        industry("Clun Livestock Farm", 450.0, 350.0, &[GRAIN], &[]),
        industry(
            "Ely Quarry",
            5_000.0,
            300.0,
            &[],
            &[("::/cargos/stone/stone.cargo", 90)],
        ),
    ]
}

/// Ely's commercial buildings take `limit` planks a year, `supply` of them
/// supplied, at `at`.
fn ely_needs_planks(world: &mut World, supply: u32, limit: u32, at: Pos) {
    world.towns[1].needs = BoundedVec::new(vec![AiNeed {
        cargo: Text::new(PLANKS).unwrap(),
        supply,
        limit,
        at,
    }])
    .unwrap();
}

fn loan(kind: &str, amount: i64) -> LoanTerms {
    LoanTerms {
        kind: Text::new(kind).unwrap(),
        amount,
        duration: 31_536_000_000,
        percentage: 50_000,
        birth_day: None,
        cooldown_until: None,
        last_pay_day: None,
        times_paid: None,
        id: None,
    }
}

fn street(a: Pos, b: Pos) -> AiStreet {
    AiStreet {
        a,
        b,
        mid: Pos {
            x: (a.x + b.x) / 2,
            y: (a.y + b.y) / 2,
            z: (a.z + b.z) / 2,
        },
        template: Text::new("::/street/town_small.street_template").unwrap(),
        length: 100,
        owned: false,
        objects: 0,
        structure: false,
        company_junction: false,
    }
}

fn seat() -> PlayerId {
    PlayerId::ai_seat(&RoomId(FixedBytes([4; 16])), 0)
}

/// Plays `brain` against `world` for `time` of simulated play: every
/// question answered at once, every action applied at once.
fn play(brain: &mut Brain, world: &mut World, start: Instant, time: Duration) -> Instant {
    let mut now = start;
    let end = start + time;
    while now < end {
        while let Some(out) = brain.poll(now, 100) {
            match out {
                Out::Ask(question) => {
                    let answer = world.answer(&question);
                    if let Some(Out::Act(payload)) = brain.answered(Ok(answer), now) {
                        world.apply(now, Action::from_payload(&payload).unwrap());
                    }
                }
                Out::Act(payload) => world.apply(now, Action::from_payload(&payload).unwrap()),
            }
        }
        now += STEP;
    }
    now
}

#[test]
fn a_normal_ai_founds_its_company_and_runs_buses_in_a_town() {
    let mut world = World::new();
    let mut brain = Brain::new(seat(), "Rossi Transport", AiDifficulty::Normal, None);
    let start = Instant::now();
    play(&mut brain, &mut world, start, Duration::from_secs(600));
    assert_eq!(world.company, Some(1), "it founded its company first");
    assert!(matches!(
        world.actions[0].1,
        Action::CompanyOp(CompanyOp::Create { .. })
    ));
    assert!(
        world.lines_with_buses(2) >= 1,
        "a line with Normal's two buses: {:?}",
        world.lines
    );
    let stops = world.lines[0].stops.len();
    assert!((2..=3).contains(&stops), "{stops} stops");
    assert!(
        world.depots.iter().any(|d| d.connected),
        "its depot reaches the street"
    );
    // Normal weighs capacity and speed for the money.
    assert!(
        world
            .vehicles
            .iter()
            .all(|v| v.model.as_str() == "vehicle/bus/good.mdl")
    );
    // Never faster than its hands.
    for pair in world.actions.windows(2) {
        assert!(
            pair[1].0 - pair[0].0 >= Duration::from_secs(5),
            "{:?}",
            pair[1].0 - pair[0].0
        );
    }
    let said = brain.take_said();
    assert!(
        said.iter().any(|l| l.starts_with("opened a bus line in")),
        "{said:?}"
    );
}

#[test]
fn an_easy_ai_is_slower_buys_the_cheapest_bus_and_runs_one() {
    let mut world = World::new();
    let mut brain = Brain::new(seat(), "Novak Transit", AiDifficulty::Easy, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(900),
    );
    assert!(world.lines_with_buses(1) >= 1);
    assert!(
        world
            .vehicles
            .iter()
            .all(|v| v.model.as_str() == "vehicle/bus/cheap.mdl")
    );
    assert_eq!(world.loans, 1, "Easy borrows once, the small loan");
    assert!(world.offers.iter().any(|o| o.kind.as_str() == "Large"));
    for pair in world.actions.windows(2) {
        assert!(pair[1].0 - pair[0].0 >= Duration::from_secs(20));
    }
    assert!(world.lines[0].stops.len() == 2, "Easy stops twice");
}

#[test]
fn a_hard_ai_serves_town_after_town_while_its_money_lasts() {
    let mut world = World::new();
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(900),
    );
    assert_eq!(world.lines_with_buses(3), 2, "both towns, three buses each");
    assert!(
        matches!(&world.actions[1].1, Action::Loan(op)
            if matches!(&**op, LoanOp::Take { offer, .. } if offer.kind.as_str() == "Large")),
        "Hard borrows the large loan right after founding"
    );
    let memo = brain.memo();
    assert!(
        tpf3mp_ai::brain::serves(&memo, TownId(1)) && tpf3mp_ai::brain::serves(&memo, TownId(2))
    );
}

#[test]
fn a_refused_depot_tries_the_other_side_and_a_refused_town_is_left_for_another() {
    let mut world = World::new();
    world.refuse_depot_side = Some(1.0);
    world.refuse_stops_in = Some(1);
    let mut brain = Brain::new(seat(), "Moreau Rail", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(600),
    );
    // Didcot (town 1) refuses every stop: it serves Ely instead.
    assert_eq!(world.lines.len(), 1);
    assert!(world.lines[0].name.as_str().starts_with("Ely"));
    assert!(world.depots.iter().all(|d| d.connected));
    let said = brain.take_said();
    assert!(
        said.iter()
            .any(|l| l.contains("gave up on a bus line in Didcot")),
        "{said:?}"
    );
}

#[test]
fn a_new_driver_carries_on_from_the_memo() {
    let mut world = World::new();
    let mut first = Brain::new(seat(), "Rossi Transport", AiDifficulty::Normal, None);
    // Long enough for one line, before its next look.
    play(
        &mut first,
        &mut world,
        Instant::now(),
        Duration::from_secs(100),
    );
    let memo = first.memo();
    let lines = world.lines.len();
    assert_eq!(lines, 1);
    // The next driver's brain knows the town it serves and does not start
    // there again.
    let mut second = Brain::new(seat(), "Rossi Transport", AiDifficulty::Normal, Some(&memo));
    play(
        &mut second,
        &mut world,
        Instant::now(),
        Duration::from_secs(600),
    );
    assert!(world.lines.len() > lines, "a new town");
    let names: Vec<&str> = world.lines.iter().map(|l| l.name.as_str()).collect();
    let mut unique = names.clone();
    unique.dedup();
    assert_eq!(names, unique, "no town twice: {names:?}");
}

#[test]
fn without_money_it_waits_and_asks_nothing_dear() {
    let mut world = World::new();
    world.money = 50_000;
    world.offers.clear();
    let mut brain = Brain::new(seat(), "Okafor Freight", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(300),
    );
    assert_eq!(
        world.actions.len(),
        1,
        "only its founding: {:?}",
        world.actions.len()
    );
    assert!(brain.take_said().iter().any(|l| l == "waits for money"));
}

#[test]
fn a_depot_that_does_not_reach_the_street_ends_the_line_before_it_opens() {
    let mut world = World::new();
    world.snaps = false;
    let mut brain = Brain::new(seat(), "Rossi Transport", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(120),
    );
    assert!(!world.depots.is_empty(), "it built a depot");
    assert!(
        world.lines.is_empty(),
        "no line without a depot on the street"
    );
    assert!(
        !world
            .actions
            .iter()
            .any(|(_, a)| matches!(a, Action::BuildRoad(_))),
        "and no road of its own onto the depot's entrance"
    );
    let said = brain.take_said();
    assert!(
        said.iter()
            .any(|l| l == "its depot did not reach the street"),
        "{said:?}"
    );
}

#[test]
fn stops_a_project_left_behind_do_not_make_their_town_served_and_are_taken_up() {
    let mut world = World::new();
    // Its company, with money, and a stop in Didcot (town 1) on no line.
    world.company = Some(1);
    world.money = 5_000_000;
    world.stations.push(AiStation {
        id: StationId(900),
        pos: pos(20.0, 30.0),
        terminals: 2,
        waiting: 0,
        ends: BoundedVec::default(),
    });
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(60),
    );
    assert!(
        world
            .lines
            .first()
            .is_some_and(|l| l.name.as_str().starts_with("Didcot")),
        "Hard's best town is still Didcot: {:?}",
        world.lines
    );
    // And its line takes the stop up instead of building another there.
    assert!(
        world.lines[0].stops.contains(&StationId(900)),
        "{:?}",
        world.lines[0].stops
    );
    assert!(
        brain
            .take_said()
            .iter()
            .any(|l| l == "takes up a stop of its own for a bus line in Didcot")
    );
}

/// The street a depot build splits.
fn split_street(action: &Action) -> Option<(Pos, Pos)> {
    let Action::BuildConstruction(build) = action else {
        return None;
    };
    build
        .connection
        .as_ref()?
        .vertices
        .iter()
        .find_map(|v| match &v.resolve {
            Resolve::Split(e) => Some((e.ends.a, e.ends.b)),
            _ => None,
        })
}

#[test]
fn a_depot_no_game_built_is_not_tried_again_on_the_same_street() {
    let mut world = World::new();
    world.drop_depots = 1;
    let mut brain = Brain::new(seat(), "Rossi Transport", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(120),
    );
    let tried: Vec<(Pos, Pos)> = world
        .actions
        .iter()
        .filter_map(|(_, a)| split_street(a))
        .collect();
    assert!(tried.len() >= 2, "{tried:?}");
    assert_ne!(tried[0], tried[1], "the next street, not its other side");
    assert!(world.depots.iter().any(|d| d.connected));
    assert!(!world.lines.is_empty(), "and its line opened");
}

#[test]
fn a_depot_never_splits_a_companys_street_or_one_at_a_companys_junction() {
    for junction in [false, true] {
        let mut world = World::new();
        for street in &mut world.streets {
            if junction {
                street.company_junction = true;
            } else {
                street.owned = true;
            }
        }
        let mut brain = Brain::new(seat(), "Rossi Transport", AiDifficulty::Hard, None);
        play(
            &mut brain,
            &mut world,
            Instant::now(),
            Duration::from_secs(120),
        );
        assert!(
            !world
                .actions
                .iter()
                .any(|(_, a)| matches!(a, Action::BuildConstruction(_))),
            "no depot there (junction {junction})"
        );
        let said = brain.take_said();
        assert!(
            said.iter().any(|l| l == "found no place for a depot"),
            "{said:?}"
        );
    }
}

#[test]
fn it_carries_cargo_by_truck_to_a_town_that_needs_it() {
    let mut world = World::new();
    world.industries = mills();
    ely_needs_planks(&mut world, 0, 60, pos(5_320.0, -280.0));
    // Trucks by default.
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(600),
    );
    // Buses in a town first, then the planks by truck, in turn.
    let trucks: Vec<LineId> = world
        .line_modes
        .iter()
        .filter(|(_, m)| *m == 4)
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(trucks.len(), 1, "one truck line: {:?}", world.line_modes);
    assert!(
        world.line_modes.iter().any(|(_, m)| *m == 3),
        "and bus lines"
    );
    let line = world.lines.iter().find(|l| l.id == trucks[0]).unwrap();
    assert_eq!(line.stops.len(), 2, "the sawmill and the town");
    assert_eq!(
        Some(line.stops[0]),
        world.industries[0].station,
        "the sawmill's own station"
    );
    let unload = world
        .stations
        .iter()
        .find(|s| s.id == line.stops[1])
        .expect("a truck station of its own");
    let (dx, dy) = (
        f64::from(unload.pos.x) / 1000.0 - 5_320.0,
        f64::from(unload.pos.y) / 1000.0 + 280.0,
    );
    assert!(
        dx.hypot(dy) <= 180.0,
        "by the district that takes planks: {dx} {dy}"
    );
    assert_eq!(line.name.as_str(), "Witney Sawmill planks");
    let on_it: Vec<&AiVehicle> = world
        .vehicles
        .iter()
        .filter(|v| v.line == Some(trucks[0]))
        .collect();
    assert_eq!(on_it.len(), 3, "Hard's count");
    assert!(
        on_it
            .iter()
            .all(|v| v.model.as_str() == "vehicle/truck/planks.mdl")
    );
    let said = brain.take_said();
    assert!(
        said.iter()
            .any(|l| l == "opened a truck line carrying planks from Witney Sawmill to Ely"),
        "{said:?}"
    );
    // No town takes grain or stone: never planned, the livestock farm that
    // takes grain notwithstanding.
    assert!(
        !said
            .iter()
            .any(|l| l.contains("grain") || l.contains("stone")),
        "{said:?}"
    );
}

#[test]
fn it_carries_nothing_a_town_is_supplied_with_enough_of() {
    let mut world = World::new();
    world.industries = mills();
    ely_needs_planks(&mut world, 55, 60, pos(5_320.0, -280.0));
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    brain.carry_cargo(true);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(300),
    );
    assert!(world.line_modes.iter().all(|(_, m)| *m == 3), "buses only");
    assert!(
        brain
            .take_said()
            .iter()
            .any(|l| l == "found no cargo to carry")
    );
}

#[test]
fn a_cargo_no_road_joins_is_left_and_too_far_for_easy() {
    let mut world = World::new();
    world.industries = mills();
    ely_needs_planks(&mut world, 0, 60, pos(5_320.0, -280.0));
    world.roads_join = false;
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    brain.carry_cargo(true);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(300),
    );
    assert!(world.line_modes.iter().all(|(_, m)| *m == 3), "buses only");
    let said = brain.take_said();
    assert!(
        said.iter()
            .any(|l| l == "found no cargo it can carry by road"),
        "{said:?}"
    );

    // A town 4.4 km off: within Hard's reach, beyond Easy's 4 km.
    let mut world = World::new();
    world.industries = mills();
    ely_needs_planks(&mut world, 0, 60, pos(4_050.0, 350.0));
    let mut easy = Brain::new(seat(), "Novak Transit", AiDifficulty::Easy, None);
    easy.carry_cargo(true);
    play(
        &mut easy,
        &mut world,
        Instant::now(),
        Duration::from_secs(900),
    );
    assert!(world.line_modes.iter().all(|(_, m)| *m == 3));
    assert!(
        easy.take_said()
            .iter()
            .any(|l| l == "found no cargo to carry")
    );
}

#[test]
fn it_plans_no_truck_lines_when_told_not_to() {
    let mut world = World::new();
    world.industries = mills();
    ely_needs_planks(&mut world, 0, 60, pos(5_320.0, -280.0));
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    brain.carry_cargo(false);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(600),
    );
    assert!(!world.lines.is_empty(), "bus lines");
    assert!(
        world.line_modes.iter().all(|(_, m)| *m == 3),
        "no truck line: {:?}",
        world.line_modes
    );
}

#[test]
fn a_cargo_too_far_for_trucks_goes_by_train() {
    let mut world = World::new();
    // A sawmill in open country 3.7 km from the district of Ely that
    // takes planks, and money enough for a rail line.
    world.industries = vec![industry(
        "Witney Sawmill",
        1_800.0,
        900.0,
        &[],
        &[(PLANKS, 120)],
    )];
    ely_needs_planks(&mut world, 0, 60, pos(5_320.0, -280.0));
    world.money = 30_000_000;
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(1_200),
    );
    let said = brain.take_said();
    let rail: Vec<&AiLine> = world
        .lines
        .iter()
        .filter(|l| world.line_modes.contains(&(l.id, 7)))
        .collect();
    assert_eq!(rail.len(), 1, "one rail line: {said:?}");
    let line = rail[0];
    assert_eq!(line.name.as_str(), "Witney Sawmill planks train");
    assert!(line.stops.iter().all(|s| world.rail_stations.contains(s)));
    // Its stations by the sawmill and by the district that takes planks.
    let at = |id: StationId| P::of(world.stations.iter().find(|s| s.id == id).unwrap().pos);
    assert!(at(line.stops[0]).flat_distance(P::of(pos(1_800.0, 900.0))) <= 260.0);
    assert!(at(line.stops[1]).flat_distance(P::of(pos(5_320.0, -280.0))) <= 350.0);
    // Track all the way, both stations' line ends joined, a depot joined.
    assert!(
        world.track_laid >= 3_000.0,
        "{} m of track",
        world.track_laid
    );
    assert!(
        world
            .stations
            .iter()
            .filter(|s| world.rail_stations.contains(&s.id))
            .all(|s| s.ends.iter().any(|e| !e.open))
    );
    assert!(
        world
            .depots
            .iter()
            .any(|d| d.file.as_str().contains("/rail/") && d.connected)
    );
    // A train on it, an engine and its wagons.
    assert!(
        world
            .vehicles
            .iter()
            .any(|v| v.line == Some(line.id) && v.model.as_str().contains("engine")),
        "{:?}",
        world.vehicles
    );
    assert!(
        said.iter()
            .any(|l| l == "runs a train on a rail line carrying planks from Witney Sawmill to Ely"),
        "{said:?}"
    );
    // Not trucks as well.
    assert!(!world.line_modes.iter().any(|(_, m)| *m == 4));
}

#[test]
fn a_starved_maker_is_carried_to_the_town_then_fed_its_inputs() {
    let mut world = World::new();
    // A mill in Didcot that makes no food for want of grain, a farm in
    // Didcot that makes grain, Ely's need of food; money for trucks, short
    // of a rail line.
    const FOOD: &str = "::/cargos/food/food.cargo";
    world.industries = vec![
        industry("Didcot Mill", 300.0, 200.0, &[GRAIN], &[(FOOD, 0)]),
        industry("Witney Farm", -400.0, -300.0, &[], &[(GRAIN, 120)]),
    ];
    world.towns[1].needs = BoundedVec::new(vec![AiNeed {
        cargo: Text::new(FOOD).unwrap(),
        supply: 0,
        limit: 80,
        at: pos(5_320.0, -280.0),
    }])
    .unwrap();
    world.money = 3_000_000;
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(1_500),
    );
    let said = brain.take_said();
    assert!(
        said.iter()
            .any(|l| l == "plans a chain of 2 lines bringing food to Ely"),
        "{said:?}"
    );
    let trucks: Vec<&AiLine> = world
        .lines
        .iter()
        .filter(|l| world.line_modes.contains(&(l.id, 4)))
        .collect();
    assert_eq!(trucks.len(), 2, "{said:?}");
    let station = |name: &str| {
        world
            .industries
            .iter()
            .find(|i| i.name.as_str() == name)
            .unwrap()
            .station
            .unwrap()
    };
    // The mill's food into Ely first, then the farm's grain into the mill.
    assert_eq!(trucks[0].stops[0], station("Didcot Mill"));
    assert!(world.truck_stations.contains(&trucks[0].stops[1]));
    assert_eq!(trucks[1].stops[0], station("Witney Farm"));
    assert_eq!(trucks[1].stops[1], station("Didcot Mill"));
    assert!(
        world
            .vehicles
            .iter()
            .any(|v| v.line == Some(trucks[1].id) && v.model.as_str() == "vehicle/truck/grain.mdl"),
        "{:?}",
        world.vehicles
    );
}

#[test]
fn a_chains_leg_refused_by_train_goes_by_truck() {
    let mut world = World::new();
    // A mill north of Didcot's streets starved of grain, Ely's need of food,
    // and a farm in Ely 4.6 km from the mill, where no rail station may
    // stand; money for trains.
    const FOOD: &str = "::/cargos/food/food.cargo";
    world.industries = vec![
        industry("Didcot Mill", 300.0, 1_300.0, &[GRAIN], &[(FOOD, 0)]),
        industry("Ely Farm", 4_800.0, 300.0, &[], &[(GRAIN, 120)]),
    ];
    world.towns[1].needs = BoundedVec::new(vec![AiNeed {
        cargo: Text::new(FOOD).unwrap(),
        supply: 0,
        limit: 80,
        at: pos(5_320.0, -280.0),
    }])
    .unwrap();
    world.refuse_rail_near = Some(pos(4_800.0, 300.0));
    world.money = 30_000_000;
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(2_400),
    );
    let said = brain.take_said();
    assert!(
        said.iter().any(|l| l == "tries its chain's leg by truck"),
        "{said:?}"
    );
    let station = |name: &str| {
        world
            .industries
            .iter()
            .find(|i| i.name.as_str() == name)
            .unwrap()
            .station
            .unwrap()
    };
    // The mill's food by train into Ely, then the farm's grain by truck
    // into the mill.
    let by = |mode: u16| -> Vec<&AiLine> {
        world
            .lines
            .iter()
            .filter(|l| world.line_modes.contains(&(l.id, mode)))
            .collect()
    };
    assert_eq!(by(7).len(), 1, "{said:?}");
    let trucks = by(4);
    assert_eq!(trucks.len(), 1, "{said:?}");
    assert_eq!(trucks[0].stops[0], station("Ely Farm"));
    assert_eq!(trucks[0].stops[1], station("Didcot Mill"));
}

#[test]
fn no_rail_line_is_built_for_a_cargo_no_wagon_carries() {
    let mut world = World::new();
    world.industries = vec![industry(
        "Witney Sawmill",
        1_800.0,
        900.0,
        &[],
        &[(PLANKS, 120)],
    )];
    ely_needs_planks(&mut world, 0, 60, pos(5_320.0, -280.0));
    world.money = 30_000_000;
    world.no_wagons = true;
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(600),
    );
    let said = brain.take_said();
    assert!(
        said.iter()
            .any(|l| l.starts_with("found no wagon for its cargo")),
        "{said:?}"
    );
    // Its train chosen before anything is built: nothing was.
    assert!(world.rail_stations.is_empty());
    assert_eq!(world.track_laid, 0.0);
}

#[test]
fn a_rail_line_crosses_a_country_road_on_the_level() {
    let mut world = World::new();
    world.industries = vec![industry(
        "Witney Sawmill",
        1_800.0,
        900.0,
        &[],
        &[(PLANKS, 120)],
    )];
    ely_needs_planks(&mut world, 0, 60, pos(5_320.0, -280.0));
    // A country road from north to south between the sawmill and Ely.
    let road = street(pos(3_500.0, -2_000.0), pos(3_500.0, 2_000.0));
    world.streets.push(road.clone());
    world.money = 30_000_000;
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(1_200),
    );
    let said = brain.take_said();
    // The road split where the track crosses it, by a vertex of a track
    // build, and the line runs.
    assert_eq!(
        world.crossings,
        vec![EdgeEnds {
            a: road.a,
            b: road.b
        }],
        "{said:?}"
    );
    assert!(
        said.iter().any(|l| l.contains("1 level crossings")),
        "{said:?}"
    );
    assert!(
        said.iter()
            .any(|l| l == "runs a train on a rail line carrying planks from Witney Sawmill to Ely"),
        "{said:?}"
    );
}

#[test]
fn a_rail_line_it_cannot_pay_for_is_never_begun() {
    let mut world = World::new();
    world.industries = vec![industry(
        "Witney Sawmill",
        1_800.0,
        900.0,
        &[],
        &[(PLANKS, 120)],
    )];
    ely_needs_planks(&mut world, 0, 60, pos(5_320.0, -280.0));
    // A gorge 70 m deep and 1.6 km wide between them: the line across it
    // and a train cost more than the AI has.
    world.ground = Some(|x, _| {
        if (x - 3_400.0).abs() < 800.0 {
            -60.0
        } else {
            10.0
        }
    });
    world.money = 8_000_000;
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(600),
    );
    let said = brain.take_said();
    assert!(
        said.iter()
            .any(|l| l.starts_with("found its rail line too dear")
                || l.starts_with("found no engine it can afford")),
        "{said:?}"
    );
    // Nothing of it built: no rail station, no track.
    assert!(world.rail_stations.is_empty(), "{said:?}");
    assert_eq!(world.track_laid, 0.0);
}

#[test]
fn a_line_goes_over_or_under_a_street_it_may_not_cross_on_the_level() {
    let mut world = World::new();
    world.industries = vec![industry(
        "Witney Sawmill",
        1_800.0,
        900.0,
        &[],
        &[(PLANKS, 120)],
    )];
    ely_needs_planks(&mut world, 0, 60, pos(5_320.0, -280.0));
    // A road with stops on it from edge to edge between them: no level
    // crossing may be laid on it.
    let mut road = street(pos(3_500.0, -20_000.0), pos(3_500.0, 20_000.0));
    road.objects = 2;
    world.streets.push(road);
    world.money = 30_000_000;
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(1_200),
    );
    let said = brain.take_said();
    assert!(world.crossings.is_empty(), "{said:?}");
    assert!(
        world
            .structures
            .iter()
            .any(|s| matches!(s, Structure::Bridge(_) | Structure::Tunnel(_))),
        "{said:?}"
    );
    assert!(
        said.iter()
            .any(|l| l == "runs a train on a rail line carrying planks from Witney Sawmill to Ely"),
        "{said:?}"
    );
}

#[test]
fn a_line_whose_track_would_be_refused_builds_no_station() {
    let mut world = World::new();
    world.industries = vec![industry(
        "Witney Sawmill",
        1_800.0,
        900.0,
        &[],
        &[(PLANKS, 120)],
    )];
    ely_needs_planks(&mut world, 0, 60, pos(5_320.0, -280.0));
    // The game refuses any track by the sawmill.
    world.refuse_track_near = Some(pos(1_800.0, 900.0));
    world.money = 30_000_000;
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(900),
    );
    let said = brain.take_said();
    assert!(
        said.iter()
            .any(|l| l.starts_with("a piece of its track would be refused")),
        "{said:?}"
    );
    assert!(
        said.iter()
            .any(|l| l.starts_with("found its track would be refused")),
        "{said:?}"
    );
    // Its track put to the verdict before anything was built: nothing of
    // it built.
    assert!(world.rail_stations.is_empty(), "{said:?}");
    assert_eq!(world.track_laid, 0.0);
}

/// A Normal AI's bus line in Didcot, its buses running: the world after it.
fn a_running_bus_line() -> (World, Brain, Instant) {
    let mut world = World::new();
    world.money = 3_000_000;
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Normal, None);
    let start = Instant::now();
    play(&mut brain, &mut world, start, Duration::from_secs(600));
    assert!(world.lines_with_buses(2) >= 1, "{:?}", brain.take_said());
    brain.take_said();
    (world, brain, start + Duration::from_secs(600))
}

#[test]
fn a_worn_out_bus_is_replaced_on_its_line() {
    let (mut world, mut brain, now) = a_running_bus_line();
    let line = world.lines[0].id;
    let worn = world
        .vehicles
        .iter_mut()
        .find(|v| v.line == Some(line))
        .unwrap();
    worn.worn = 104;
    let id = worn.id;
    for v in world.vehicles.iter_mut().filter(|v| v.id != id) {
        v.worn = 40;
    }
    play(&mut brain, &mut world, now, Duration::from_secs(600));
    let said = brain.take_said();
    assert_eq!(world.replaced, [id], "{said:?}");
    let v = world.vehicles.iter().find(|v| v.id == id).unwrap();
    assert_eq!(v.line, Some(line), "still on its line");
    assert_eq!(v.worn, 0);
    assert!(
        said.iter()
            .any(|l| l.starts_with("replaces a worn-out vehicle on")),
        "{said:?}"
    );
    assert!(world.sold.is_empty());
}

#[test]
fn a_full_line_gets_another_bus_and_an_empty_one_loses_one_never_its_last() {
    let (mut world, mut brain, now) = a_running_bus_line();
    let line = world.lines[0].id;
    let before = world
        .vehicles
        .iter()
        .filter(|v| v.line == Some(line))
        .count();
    // Full, with passengers waiting.
    world.lines[0].load = 95;
    world.lines[0].waiting = 60;
    for v in &mut world.vehicles {
        v.worn = 10;
    }
    play(&mut brain, &mut world, now, Duration::from_secs(300));
    let said = brain.take_said();
    let after = world
        .vehicles
        .iter()
        .filter(|v| v.line == Some(line))
        .count();
    assert_eq!(after, before + 1, "{said:?}");
    assert!(said.iter().any(|l| l.contains("running full")), "{said:?}");
    // Then near empty, settled: one sold a look, down to one.
    world.lines[0].load = 10;
    world.lines[0].waiting = 0;
    for v in &mut world.vehicles {
        v.worn = 10;
    }
    play(
        &mut brain,
        &mut world,
        now + Duration::from_secs(300),
        Duration::from_secs(2_400),
    );
    let said = brain.take_said();
    let left = world
        .vehicles
        .iter()
        .filter(|v| v.line == Some(line))
        .count();
    assert_eq!(left, 1, "{said:?}");
    assert_eq!(world.sold.len(), after - 1);
    assert!(
        said.iter().any(|l| l.contains("running near empty")),
        "{said:?}"
    );
}

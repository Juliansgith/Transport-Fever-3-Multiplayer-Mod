//! The brain against a stand-in world: it answers the brain's questions as
//! the mod does and applies its actions as every game would, so the brain's
//! whole round, from founding its company to buses on a line, plays here.

#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use tpf3mp_ai::{Brain, Out, place::P};
use tpf3mp_bridge::ai::{
    AiAnswer, AiCompany, AiDepot, AiEnd, AiIndustry, AiLand, AiLine, AiLoan, AiModel, AiNeed,
    AiOutput, AiQuestion, AiRoute, AiStation, AiStreet, AiTown, AiVehicle, AiVerdict, land,
};
use tpf3mp_proto::{
    AiDifficulty, BoundedVec, FixedBytes, PlayerId, RoomId, Text,
    action::{
        Action, CompanyOp, EdgeEnds, EdgeObjectKind, LineChange, LineId, LoanOp, LoanTerms,
        PlaceStop, Pos, Resolve, StationId, Structure, TownId, VehicleId,
    },
};

const STEP: Duration = Duration::from_millis(100);
/// The game's month, in its milliseconds (build 40425's default).
const MONTH: u32 = 121_750;
/// The road depot TF3 builds road vehicles in, and the game's refusal of a
/// split whose street is gone (tpf3mp/apply.lua).
const ROAD_DEPOT: &str = tpf3mp_ai::place::ROAD_DEPOT;
const SPLIT_GONE: &str =
    "[string \"tpf3mp_1::/scripts/tpf3mp/apply.lua\"]:1744: no Street edge to split at vertex 1";

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
    /// Signals placed on its track; passenger stations built.
    signals: Vec<PlaceStop>,
    passenger_stations: u32,
    /// Road depot verdicts that find another company's depot just built on
    /// the street asked about, splitting it, as the other AI's was in one
    /// town (runs 29 to 37); and what a road depot costs by its origin,
    /// clearing houses (the fixture's towns, 2026-10-30).
    rival_depots: u32,
    depot_cost: Option<fn(Pos) -> i64>,
    /// The road depots the verdicts would build: where, and at what price.
    depot_quotes: Vec<(Pos, i64)>,
    /// What a truck station costs by its origin, and the verdicts' quotes
    /// for them; how many standing things a construction's verdict says it
    /// touches.
    station_cost: Option<fn(Pos) -> i64>,
    station_quotes: Vec<(Pos, i64)>,
    touches: Option<fn(&tpf3mp_proto::action::ConstructionBuild) -> u16>,
    /// Rail depot verdicts, the first so many, that touch something.
    rail_depot_touches: u32,
    /// The loans its company owes, and the game's months: how often they
    /// fall due in play (none: never), what each line earns a year then,
    /// how far through their lives its vehicles go each month, and the
    /// least money the company had.
    owed: Vec<AiLoan>,
    month_every: Option<Duration>,
    month_at: Option<Instant>,
    line_profit: Option<fn(&AiLine) -> i64>,
    wear_per_month: u16,
    lowest: i64,
    loans_taken: u32,
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
            signals: Vec::new(),
            passenger_stations: 0,
            rival_depots: 0,
            depot_cost: None,
            depot_quotes: Vec::new(),
            station_cost: None,
            station_quotes: Vec::new(),
            touches: None,
            rail_depot_touches: 0,
            owed: Vec::new(),
            month_every: None,
            month_at: None,
            line_profit: None,
            wear_per_month: 0,
            lowest: 0,
            loans_taken: 0,
            actions: Vec::new(),
            next_id: 1,
        }
    }

    /// Its towns too small for a passenger train: a test of a cargo's
    /// railway alone.
    fn small_towns(&mut self) {
        for town in &mut self.towns {
            town.buildings = 120;
        }
    }

    fn id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    fn answer(&mut self, question: &AiQuestion) -> AiAnswer {
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
                owed: BoundedVec::new(self.owed.clone()).unwrap(),
                month: MONTH,
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
                        // A station of its stands on built ground.
                        let built = self.stations.iter().any(|s| {
                            let o = P::of(s.pos);
                            (o.x - x).hypot(o.y - y) <= 70.0 + f64::from(*step) * 0.71
                        });
                        flags.push(
                            if street { land::STREET } else { 0 }
                                | if built { land::BUILT } else { 0 },
                        );
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
                if self.rival_depots > 0
                    && let Some((a, b)) = split_street(&action)
                {
                    // Another company's depot, just built there.
                    self.rival_depots -= 1;
                    self.split(a, b);
                }
                let mut verdict = self.verdict(&action);
                if let Action::BuildConstruction(b) = &action
                    && b.file.as_str() == "::/depots/rail/rail_depot.con"
                    && self.rail_depot_touches > 0
                {
                    self.rail_depot_touches -= 1;
                    verdict.collisions = verdict.collisions.max(1);
                }
                if let Action::BuildConstruction(b) = &action
                    && b.file.as_str() == ROAD_DEPOT
                    && verdict.ok
                {
                    self.depot_quotes.push((b.transform.origin, verdict.cost));
                }
                if let Action::BuildConstruction(b) = &action
                    && b.file.as_str().contains("modular_terminal")
                    && verdict.ok
                {
                    self.station_quotes.push((b.transform.origin, verdict.cost));
                }
                AiAnswer::Verdict(verdict)
            }
        }
    }

    fn verdict(&self, action: &Action) -> AiVerdict {
        let ok = match action {
            // A signal on a track of the world's: both its ends nodes laid.
            Action::PlaceStop(stop) if stop.object == EdgeObjectKind::Signal => {
                self.track_node(stop.edge.ends.a).is_some()
                    && self.track_node(stop.edge.ends.b).is_some()
            }
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
        // A road depot as the game judges one: its street gone, refused as
        // apply.lua refuses the split; else "Collision" said of it, its own
        // street split for its entrance, and built, at its price there.
        let depot = matches!(action, Action::BuildConstruction(b) if b.file.as_str() == ROAD_DEPOT);
        let gone = depot
            && split_street(action)
                .is_some_and(|(a, b)| !self.streets.iter().any(|s| s.a == a && s.b == b));
        let cost = match action {
            Action::BuildConstruction(b) if depot => {
                self.depot_cost.map_or(10_000, |f| f(b.transform.origin))
            }
            Action::BuildConstruction(b) if b.file.as_str().contains("modular_terminal") => {
                self.station_cost.map_or(10_000, |f| f(b.transform.origin))
            }
            _ => 10_000,
        };
        let collisions = match action {
            Action::BuildConstruction(b) => self.touches.map_or(0, |f| f(b)),
            _ => 0,
        };
        let said = if gone {
            vec![SPLIT_GONE]
        } else if !ok || depot {
            vec!["Collision"]
        } else {
            Vec::new()
        };
        AiVerdict {
            ok,
            cost,
            messages: BoundedVec::new(said.into_iter().map(|m| Text::new(m).unwrap()).collect())
                .unwrap(),
            collisions,
        }
    }

    /// The street from `a` to `b` in two at its middle, as a depot's or a
    /// level crossing's split leaves it.
    fn split(&mut self, a: Pos, b: Pos) {
        let Some(k) = self.streets.iter().position(|s| s.a == a && s.b == b) else {
            return;
        };
        let old = self.streets.remove(k);
        for (from, to) in [(old.a, old.mid), (old.mid, old.b)] {
            let mut half = street(from, to);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                half.length = (P::of(from).flat_distance(P::of(to))) as u32;
            }
            half.template = old.template.clone();
            self.streets.push(half);
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
            Action::Loan(op) => match *op {
                LoanOp::Take { offer, .. } => {
                    assert!(self.offers.contains(&offer), "an offer it was given");
                    self.offers.retain(|o| *o != offer);
                    self.money += offer.amount;
                    self.loans += 1;
                    self.loans_taken += 1;
                    let months = offer.duration / i64::from(MONTH);
                    self.owed.push(AiLoan {
                        terms: LoanTerms {
                            id: Some(i64::from(self.loans_taken)),
                            ..offer.clone()
                        },
                        remaining: offer.amount,
                        payment: tpf3mp_ai::money::payment(&offer, MONTH).unwrap(),
                        months_left: u32::try_from(months).unwrap(),
                    });
                }
                LoanOp::Repay { loan } => {
                    let k = self
                        .owed
                        .iter()
                        .position(|l| l.terms.id == loan.id && l.terms.amount == loan.amount)
                        .expect("a loan it owes");
                    let paid = self.owed.remove(k);
                    self.money -= paid.remaining;
                    self.loans -= 1;
                }
            },
            Action::PlaceStop(stop) if stop.object == EdgeObjectKind::Signal => {
                assert!(stop.one_way);
                self.signals.push(stop);
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
                        (key.ends_with(".name") && (8_400_500..8_402_500).contains(&slot))
                            .then_some(slot)
                    })
                    .collect();
                // Its track in its third row for cargo, 10 m across; in its
                // second for passengers, 5 m across.
                let passengers = slots.iter().all(|s| *s < 8_401_500);
                let (row, x) = if passengers {
                    (8_401_000, 5.0)
                } else {
                    (8_402_000, 10.0)
                };
                let js: Vec<i64> = slots.iter().map(|s| (s - row) / 10).collect();
                let (first, last) = (*js.iter().min().unwrap(), *js.iter().max().unwrap());
                let local = |lx: f64, ly: f64| place(&build.transform, lx, ly);
                #[allow(clippy::cast_precision_loss)]
                let ends = [
                    local(x, 40.0 * first as f64 - 20.0),
                    local(x, 40.0 * last as f64 + 20.0),
                ];
                if passengers {
                    self.passenger_stations += 1;
                }
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
                // entrance onto: 2 m from it. The street it splits is two.
                let connection = build.connection.unwrap();
                let junction = connection.vertices[0].pos;
                if let Resolve::Split(e) = &connection.vertices[0].resolve {
                    self.split(e.ends.a, e.ends.b);
                }
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
                    // Or passengers alone, both ways.
                    let passengers = rules.iter().all(|r| r == &[true]);
                    assert!(
                        passengers || rules[0].iter().skip(1).any(|l| *l),
                        "{rules:?}"
                    );
                    assert!(
                        passengers || rules[1].iter().enumerate().all(|(i, l)| *l == (i == 0)),
                        "{rules:?}"
                    );
                } else {
                    assert!(rules.iter().all(|r| r == &[true]), "{rules:?}");
                    // A bus waits at its stops as the game's line window has
                    // it: one that waited none left before anyone boarded,
                    // and no AI bus line carried anyone (run 41).
                    for stop in create.line.stops.iter() {
                        assert_eq!(
                            (stop.max_wait, stop.max_extra_wait),
                            (600_000_000, 30_000_000),
                            "a bus stop's waits"
                        );
                    }
                }
                self.lines.push(AiLine {
                    id: LineId(id),
                    name: Text::lossy(create.name.as_str()),
                    stops: BoundedVec::new(create.line.stops.iter().map(|s| s.group).collect())
                        .unwrap(),
                    vehicles: BoundedVec::default(),
                    load: 0,
                    waiting: 0,
                    profit: 0,
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
            Action::EditLine(edit) if edit.change == LineChange::Delete => {
                assert!(
                    self.vehicles.iter().all(|v| v.line != Some(edit.line)),
                    "a line deleted with its vehicles on it"
                );
                self.lines.retain(|l| l.id != edit.line);
            }
            other => panic!("the brain sent {other:?}"),
        }
    }

    /// The game's months as they fall due in play: each loan's payment
    /// taken, what each line earns in a month paid in, its year's profit
    /// as it says, and its vehicles a month older.
    fn month(&mut self, now: Instant) {
        let Some(every) = self.month_every else {
            return;
        };
        let at = *self.month_at.get_or_insert(now);
        if now.saturating_duration_since(at) < every {
            return;
        }
        self.month_at = Some(now);
        for loan in &mut self.owed {
            let paid = loan.payment.min(loan.remaining);
            self.money -= paid;
            loan.remaining -= paid;
            loan.months_left = loan.months_left.saturating_sub(1);
        }
        let before = self.owed.len();
        self.owed.retain(|l| l.remaining > 0);
        self.loans -= u8::try_from(before - self.owed.len()).unwrap();
        if let Some(profit) = self.line_profit {
            for line in &mut self.lines {
                let running = self.vehicles.iter().any(|v| v.line == Some(line.id));
                line.profit = if running { profit(line) } else { 0 };
                self.money += line.profit / 12;
            }
        }
        for v in &mut self.vehicles {
            v.worn += self.wear_per_month;
        }
        self.lowest = self.lowest.min(self.money);
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
        world.month(now);
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

/// What it said of refused depot placements.
fn depot_refusals(said: &[String]) -> Vec<&String> {
    said.iter()
        .filter(|l| l.starts_with("a depot placement was refused"))
        .collect()
}

#[test]
fn a_depot_whose_street_another_company_split_reads_the_streets_again() {
    // As in one town in runs 29 to 37: the other AI's depot split the
    // street this one chose after it read the streets. Both sides of that
    // street were refused, the game finding no edge to split.
    let mut world = World::new();
    world.rival_depots = 1;
    let mut brain = Brain::new(seat(), "Lindqvist Lines", AiDifficulty::Normal, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(120),
    );
    let said = brain.take_said();
    let refused = depot_refusals(&said);
    assert_eq!(refused.len(), 1, "one refusal, then a fresh read: {said:?}");
    assert!(
        refused[0].contains("no Street edge to split"),
        "{refused:?}"
    );
    assert!(world.depots.iter().any(|d| d.connected), "{said:?}");
    assert!(
        world.lines_with_buses(2) >= 1,
        "and its line runs: {said:?}"
    );
}

#[test]
fn a_depot_goes_where_it_is_cheapest_of_several_priced() {
    // A depot clears the houses in its way: in the fixture's towns the
    // same depot cost 207,000 on open ground and over a million among
    // houses. Here its price varies from place to place.
    let mut world = World::new();
    world.depot_cost = Some(|o| {
        200_000 + i64::from((o.x / 1000).rem_euclid(97) + (o.y / 1000).rem_euclid(89)) * 3_000
    });
    let mut brain = Brain::new(seat(), "Lindqvist Lines", AiDifficulty::Normal, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(120),
    );
    let said = brain.take_said();
    let built = world.depots.first().expect("a depot").at;
    let quotes = &world.depot_quotes;
    assert!(quotes.len() >= 2, "several priced: {quotes:?}");
    let cheapest = quotes.iter().map(|(_, c)| *c).min().unwrap();
    let paid = quotes.iter().find(|(at, _)| *at == built).map(|(_, c)| *c);
    assert_eq!(paid, Some(cheapest), "the cheapest: {quotes:?}");
    assert!(
        said.iter()
            .any(|l| l.starts_with(&format!("builds its depot for {cheapest}"))),
        "{said:?}"
    );
}

#[test]
fn a_truck_lines_depot_stands_by_the_industry_it_loads_at() {
    // A sawmill between the towns, on a road of its own: its depot by it,
    // not in the district its planks go to, among houses (run 33).
    let mut world = World::new();
    world.industries = vec![industry(
        "Witney Sawmill",
        2_500.0,
        0.0,
        &[],
        &[(PLANKS, 120)],
    )];
    for k in 0..4 {
        let x = 2_300.0 + f64::from(k) * 100.0;
        world
            .streets
            .push(street(pos(x, 40.0), pos(x + 100.0, 40.0)));
    }
    ely_needs_planks(&mut world, 0, 60, pos(5_320.0, -280.0));
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(900),
    );
    let said = brain.take_said();
    assert!(
        world.line_modes.iter().any(|(_, m)| *m == 4),
        "a truck line: {said:?}"
    );
    let near = |d: &&AiDepot| P::of(d.at).flat_distance(P::of(pos(2_500.0, 0.0))) <= 300.0;
    assert!(
        world.depots.iter().any(|d| near(&d)),
        "a depot by the sawmill: {:?}",
        world.depots.iter().map(|d| d.at).collect::<Vec<_>>()
    );
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
    world.small_towns();
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
    world.small_towns();
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
    world.small_towns();
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
    world.small_towns();
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

#[test]
fn a_hard_ai_feeds_a_starved_feeder_in_turn_three_levels_deep() {
    let mut world = World::new();
    // Ely needs goods; a goods factory in Didcot makes none for want of
    // planks; the sawmill across the town makes none for want of logs; a
    // forest by Ely makes logs. Money for trucks, short of a rail line.
    const GOODS: &str = "::/cargos/goods/goods.cargo";
    const PLANKS_: &str = "::/cargos/planks/planks.cargo";
    const LOGS: &str = "::/cargos/logs/logs.cargo";
    world.industries = vec![
        industry("Didcot Factory", 300.0, 200.0, &[PLANKS_], &[(GOODS, 0)]),
        industry("Witney Sawmill", -300.0, -300.0, &[LOGS], &[(PLANKS_, 0)]),
        industry("Wychwood Forest", 4_800.0, 300.0, &[], &[(LOGS, 150)]),
    ];
    world.towns[1].needs = BoundedVec::new(vec![AiNeed {
        cargo: Text::new(GOODS).unwrap(),
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
        Duration::from_secs(2_400),
    );
    let said = brain.take_said();
    assert!(
        said.iter()
            .any(|l| l == "plans a chain of 3 lines bringing goods to Ely"),
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
    let trucks: Vec<&AiLine> = world
        .lines
        .iter()
        .filter(|l| world.line_modes.contains(&(l.id, 4)))
        .collect();
    assert_eq!(trucks.len(), 3, "{said:?}");
    // The goods into Ely first, then the planks into the factory, then the
    // logs into the sawmill.
    assert_eq!(trucks[0].stops[0], station("Didcot Factory"));
    assert_eq!(trucks[1].stops[0], station("Witney Sawmill"));
    assert_eq!(trucks[1].stops[1], station("Didcot Factory"));
    assert_eq!(trucks[2].stops[0], station("Wychwood Forest"));
    assert_eq!(trucks[2].stops[1], station("Witney Sawmill"));
}

#[test]
fn a_long_rail_line_gets_a_signalled_passing_loop_and_a_second_train_when_full() {
    let mut world = World::new();
    world.small_towns();
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
    let start = Instant::now();
    play(&mut brain, &mut world, start, Duration::from_secs(1_200));
    let said = brain.take_said();
    assert!(
        said.iter().any(|l| l.contains("a passing loop")),
        "{said:?}"
    );
    assert!(
        said.iter()
            .any(|l| l.starts_with("signals its passing loop by")),
        "{said:?}"
    );
    // One signal on the line's track, one on the loop's, passing opposite
    // ways.
    assert_eq!(world.signals.len(), 2, "{said:?}");
    assert_ne!(world.signals[0].left, world.signals[1].left);
    let line = world
        .lines
        .iter()
        .find(|l| world.line_modes.contains(&(l.id, 7)))
        .unwrap()
        .id;
    let trains = |w: &World| w.vehicles.iter().filter(|v| v.line == Some(line)).count();
    assert_eq!(trains(&world), 1);
    // Its train full one way and empty back, as a cargo line runs (its load
    // under half), two trainloads of planks waiting: a second.
    let l = world.lines.iter_mut().find(|l| l.id == line).unwrap();
    l.load = 45;
    l.waiting = 120;
    play(
        &mut brain,
        &mut world,
        start + Duration::from_secs(1_200),
        Duration::from_secs(900),
    );
    let said = brain.take_said();
    assert_eq!(trains(&world), 2, "{said:?}");
    assert!(
        said.iter()
            .any(|l| l == "runs another train through its passing loops"),
        "{said:?}"
    );
}

#[test]
fn a_normal_ai_joins_two_large_towns_by_passenger_train() {
    let mut world = World::new();
    // Didcot and Ely, 300 and 200 buildings, 5 km apart; no cargo to carry;
    // money for a rail line.
    world.money = 30_000_000;
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Normal, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(2_400),
    );
    let said = brain.take_said();
    assert!(
        said.iter()
            .any(|l| l.starts_with("plans a rail line carrying passengers from")),
        "{said:?}"
    );
    assert!(
        said.iter()
            .any(|l| l.starts_with("runs a train on a rail line carrying passengers")),
        "{said:?}"
    );
    // Two passenger stations, a line of passengers both ways, a train of
    // coaches on it.
    assert_eq!(world.passenger_stations, 2, "{said:?}");
    let line = world
        .lines
        .iter()
        .find(|l| world.line_modes.contains(&(l.id, 7)))
        .unwrap();
    assert!(line.name.as_str().ends_with(" train"));
    assert!(
        world
            .vehicles
            .iter()
            .any(|v| v.line == Some(line.id) && v.model.as_str().contains("engine"))
    );
}

#[test]
fn two_towns_joined_by_train_are_not_joined_again_as_they_grow() {
    // As in run 40: the towns' district centres move as they grow, and
    // the pair, known by a centre's place, was planned and built again.
    let mut world = World::new();
    world.money = 60_000_000;
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Normal, None);
    let start = Instant::now();
    let now = play(&mut brain, &mut world, start, Duration::from_secs(2_400));
    let said = brain.take_said();
    assert_eq!(world.passenger_stations, 2, "{said:?}");
    for town in &mut world.towns {
        let moved: Vec<Pos> = town
            .centres
            .iter()
            .map(|c| Pos {
                x: c.x + 37_000,
                y: c.y - 21_000,
                z: c.z,
            })
            .collect();
        town.centres = BoundedVec::new(moved).unwrap();
    }
    play(&mut brain, &mut world, now, Duration::from_secs(2_400));
    let said = brain.take_said();
    assert!(
        !said
            .iter()
            .any(|l| l.starts_with("plans a rail line carrying passengers from")),
        "{said:?}"
    );
    assert_eq!(world.passenger_stations, 2, "one line: {said:?}");
}

#[test]
fn a_longer_line_gets_two_loops_and_runs_up_to_three_trains() {
    let mut world = World::new();
    world.small_towns();
    // A sawmill 6.6 km from the district of Ely that takes planks.
    world.industries = vec![industry(
        "Witney Sawmill",
        -1_200.0,
        900.0,
        &[],
        &[(PLANKS, 120)],
    )];
    ely_needs_planks(&mut world, 0, 60, pos(5_320.0, -280.0));
    world.money = 60_000_000;
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    let start = Instant::now();
    play(&mut brain, &mut world, start, Duration::from_secs(1_500));
    let said = brain.take_said();
    assert!(
        said.iter().any(|l| l.contains("2 passing loops")),
        "{said:?}"
    );
    assert_eq!(world.signals.len(), 4, "{said:?}");
    let line = world
        .lines
        .iter()
        .find(|l| world.line_modes.contains(&(l.id, 7)))
        .unwrap()
        .id;
    let trains = |w: &World| w.vehicles.iter().filter(|v| v.line == Some(line)).count();
    // Planks piling up: trains added up to three, never a fourth.
    let l = world.lines.iter_mut().find(|l| l.id == line).unwrap();
    l.load = 45;
    l.waiting = 200;
    play(
        &mut brain,
        &mut world,
        start + Duration::from_secs(1_500),
        Duration::from_secs(1_800),
    );
    let said = brain.take_said();
    assert_eq!(trains(&world), 3, "{said:?}");
}

#[test]
fn a_second_rail_line_from_one_place_shares_the_first_ones_depot() {
    let mut world = World::new();
    world.small_towns();
    // A sawmill making planks and goods, both wanted in Ely.
    const GOODS: &str = "::/cargos/goods/goods.cargo";
    world.industries = vec![industry(
        "Witney Sawmill",
        1_800.0,
        900.0,
        &[],
        &[(PLANKS, 120), (GOODS, 90)],
    )];
    world.towns[1].needs = BoundedVec::new(vec![
        AiNeed {
            cargo: Text::new(PLANKS).unwrap(),
            supply: 0,
            limit: 60,
            at: pos(5_320.0, -280.0),
        },
        AiNeed {
            cargo: Text::new(GOODS).unwrap(),
            supply: 0,
            limit: 60,
            at: pos(5_320.0, -280.0),
        },
    ])
    .unwrap();
    world.money = 60_000_000;
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(2_400),
    );
    let said = brain.take_said();
    let rail = world
        .lines
        .iter()
        .filter(|l| world.line_modes.contains(&(l.id, 7)))
        .count();
    assert_eq!(rail, 2, "{said:?}");
    assert!(
        said.iter()
            .any(|l| l == "shares a rail depot with another of its lines"),
        "{said:?}"
    );
    // One rail depot, both lines' trains bought there.
    let depots = world
        .depots
        .iter()
        .filter(|d| d.file.as_str().contains("/rail/"))
        .count();
    assert_eq!(depots, 1, "{said:?}");
}

/// A loan as the room offers one: `months` of the game's long, at
/// `percent` a year.
fn offer(kind: &str, amount: i64, months: i64, percent: i64) -> LoanTerms {
    LoanTerms {
        duration: months * i64::from(MONTH),
        percentage: percent * 10_000,
        ..loan(kind, amount)
    }
}

#[test]
fn an_ai_whose_lines_lose_money_borrows_no_more_and_keeps_out_of_debt() {
    // As in the long run of 2026-10-30: every line loses money, the loans
    // fall due each month, and the room offers a large loan again as soon
    // as one is taken. The old brain borrowed whenever it ran short and
    // owed 86 million, its balance far below zero.
    let mut world = World::new();
    world.offers = vec![
        offer("Small", 500_000, 6, 5),
        offer("Large", 27_000_000, 144, 10),
    ];
    world.month_every = Some(Duration::from_secs(30));
    world.line_profit = Some(|_| -3_600_000);
    world.wear_per_month = 1;
    let refill = world.offers.clone();
    let mut brain = Brain::new(seat(), "Rossi Transport", AiDifficulty::Hard, None);
    let mut now = Instant::now();
    let mut said = Vec::new();
    // Forty of the game's months, the offers drawn again each minute.
    for _ in 0..20 {
        now = play(&mut brain, &mut world, now, Duration::from_secs(60));
        world.offers.clone_from(&refill);
        said.extend(brain.take_said());
    }
    assert_eq!(world.loans_taken, 1, "only its first loan: {said:?}");
    assert!(world.lowest >= 0, "never below zero: {}", world.lowest);
    // Its lines that lost money over their year closed, its railway too.
    assert!(
        said.iter()
            .any(|l| l.starts_with("closes ") && l.contains("Bus")),
        "{said:?}"
    );
    assert!(
        said.iter()
            .any(|l| l.starts_with("closes ") && l.contains("train")),
        "{said:?}"
    );
}

#[test]
fn an_ai_whose_lines_pay_pays_its_loan_back_early() {
    let mut world = World::new();
    world.offers = vec![offer("Medium", 2_000_000, 48, 5)];
    world.month_every = Some(Duration::from_secs(30));
    world.line_profit = Some(|_| 24_000_000);
    world.wear_per_month = 1;
    let mut brain = Brain::new(seat(), "Rossi Transport", AiDifficulty::Normal, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(1_200),
    );
    let said = brain.take_said();
    assert_eq!(world.loans_taken, 1, "{said:?}");
    assert!(
        said.iter().any(|l| l.starts_with("pays a loan back early")),
        "{said:?}"
    );
    assert!(world.owed.is_empty(), "nothing owed: {:?}", world.owed);
}

#[test]
fn a_truck_station_goes_where_it_is_cheapest_of_several_priced() {
    // A truck station clears the houses in its way: in Hunstanton's middle
    // eight cost 580,000 to 2.6 million, every one too dear (2026-10-30).
    let mut world = World::new();
    world.industries = mills();
    ely_needs_planks(&mut world, 0, 60, pos(5_320.0, -280.0));
    world.station_cost = Some(|o| {
        300_000 + i64::from((o.x / 1000).rem_euclid(53) + (o.y / 1000).rem_euclid(47)) * 7_000
    });
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    brain.carry_cargo(true);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(900),
    );
    let said = brain.take_said();
    let quotes = &world.station_quotes;
    assert!(quotes.len() >= 2, "several priced: {quotes:?} {said:?}");
    let built = world
        .stations
        .iter()
        .find(|s| world.truck_stations.contains(&s.id))
        .expect("a truck station")
        .pos;
    let cheapest = quotes.iter().map(|(_, c)| *c).min().unwrap();
    let paid = quotes.iter().find(|(at, _)| *at == built).map(|(_, c)| *c);
    assert_eq!(paid, Some(cheapest), "the cheapest: {quotes:?}");
    assert!(
        said.iter()
            .any(|l| l.starts_with("builds its truck station for")),
        "{said:?}"
    );
}

#[test]
fn a_construction_touching_what_stands_is_not_built() {
    // The game builds a construction over what stands in its way, "Collision"
    // said of it but not refused: an AI rail depot stood across a player's
    // station (2026-10-30). Here every depot placement west of Didcot's and
    // Ely's middles touches something.
    let mut world = World::new();
    world.touches = Some(|b| {
        let x = b.transform.origin.x;
        u16::from(x < 0 || (4_000_000..5_000_000).contains(&x))
    });
    let mut brain = Brain::new(seat(), "Lindqvist Lines", AiDifficulty::Normal, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(300),
    );
    let said = brain.take_said();
    assert!(!world.depots.is_empty(), "{said:?}");
    for d in &world.depots {
        let x = d.at.x;
        assert!(
            !(x < 0 || (4_000_000..5_000_000).contains(&x)),
            "a depot over what stands at {x}: {said:?}"
        );
    }
    assert!(
        said.iter()
            .any(|l| l.contains("touches 1 things that stand")),
        "{said:?}"
    );
}

#[test]
fn a_passenger_pair_with_no_way_through_is_not_planned_again() {
    // Given up for good, in the memo a next driver keeps: the Hard AI
    // planned three such pairs again every 15 minutes (2026-10-30).
    let mut world = World::new();
    world.money = 30_000_000;
    let memo = tpf3mp_ai::Memo {
        given_up: vec![tpf3mp_ai::brain::passenger_key(TownId(1), TownId(2))],
        ..tpf3mp_ai::Memo::default()
    };
    let bytes = postcard::to_allocvec(&memo).unwrap();
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Normal, Some(&bytes));
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(2_400),
    );
    let said = brain.take_said();
    assert!(
        !said
            .iter()
            .any(|l| l.starts_with("plans a rail line carrying passengers")),
        "{said:?}"
    );
}

#[test]
fn a_rail_depot_touching_what_stands_goes_further_out_or_to_the_other_end() {
    // An AI rail depot stood across a player's station (2026-10-30): the
    // game builds it over what stands, "Collision" said of it. Here every
    // rail depot placement by the sawmill's end touches something; the line
    // is not given up with its stations and track built, its depot goes to
    // the other end.
    let mut world = World::new();
    world.small_towns();
    world.industries = vec![industry(
        "Witney Sawmill",
        1_800.0,
        900.0,
        &[],
        &[(PLANKS, 120)],
    )];
    ely_needs_planks(&mut world, 0, 60, pos(5_320.0, -280.0));
    world.money = 30_000_000;
    world.touches = Some(|b| {
        let near = P::of(b.transform.origin).flat_distance(P::of(pos(1_800.0, 900.0))) < 700.0;
        u16::from(b.file.as_str() == "::/depots/rail/rail_depot.con" && near)
    });
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(1_200),
    );
    let said = brain.take_said();
    // Every place by the sawmill refused in its trial, before anything was
    // built; none once its stations stood.
    let trial = said
        .iter()
        .filter(|l| {
            l.starts_with("a rail depot placement would be refused: touches 1 things that stand")
        })
        .count();
    assert_eq!(
        trial, 7,
        "each of its seven places by the sawmill: {said:?}"
    );
    assert!(
        !said
            .iter()
            .any(|l| l.starts_with("a rail depot placement was refused")),
        "{said:?}"
    );
    let depots: Vec<&AiDepot> = world
        .depots
        .iter()
        .filter(|d| d.file.as_str().contains("/rail/"))
        .collect();
    assert_eq!(depots.len(), 1, "{said:?}");
    assert!(depots[0].connected, "joined to the line");
    assert!(
        P::of(depots[0].at).flat_distance(P::of(pos(1_800.0, 900.0))) >= 700.0,
        "at the other end: {:?}",
        depots[0].at
    );
    assert!(
        said.iter()
            .any(|l| l.starts_with("runs a train on a rail line carrying planks")),
        "{said:?}"
    );
}
#[test]
fn a_rail_depot_touching_what_stands_at_first_goes_further_out_at_that_end() {
    let mut world = World::new();
    world.small_towns();
    world.industries = vec![industry(
        "Witney Sawmill",
        1_800.0,
        900.0,
        &[],
        &[(PLANKS, 120)],
    )];
    ely_needs_planks(&mut world, 0, 60, pos(5_320.0, -280.0));
    world.money = 30_000_000;
    world.rail_depot_touches = 1;
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(1_200),
    );
    let said = brain.take_said();
    // Its first place refused in its trial, the next taken once its
    // stations stood.
    let trial = said
        .iter()
        .filter(|l| l.starts_with("a rail depot placement would be refused"))
        .count();
    assert_eq!(trial, 1, "{said:?}");
    assert!(
        !said
            .iter()
            .any(|l| l.starts_with("a rail depot placement was refused")),
        "{said:?}"
    );
    let depot = world
        .depots
        .iter()
        .find(|d| d.file.as_str().contains("/rail/"))
        .expect("a rail depot");
    assert!(depot.connected, "joined to the line by its longer track");
    assert!(
        P::of(depot.at).flat_distance(P::of(pos(1_800.0, 900.0))) < 700.0,
        "still by the sawmill: {:?}",
        depot.at
    );
    assert!(
        said.iter()
            .any(|l| l.starts_with("runs a train on a rail line carrying planks")),
        "{said:?}"
    );
}

#[test]
fn a_rail_depot_blocked_straight_out_turns_aside_on_a_curved_track() {
    // Straight out beyond a station met a player's railway at one station
    // and streets at another (2026-10-31): the depot turns 30 degrees
    // aside, its track one edge curving from the station's heading.
    let mut world = World::new();
    world.small_towns();
    world.industries = vec![industry(
        "Witney Sawmill",
        1_800.0,
        900.0,
        &[],
        &[(PLANKS, 120)],
    )];
    ely_needs_planks(&mut world, 0, 60, pos(5_320.0, -280.0));
    world.money = 30_000_000;
    world.rail_depot_touches = 3;
    let mut brain = Brain::new(seat(), "Tanaka Express", AiDifficulty::Hard, None);
    play(
        &mut brain,
        &mut world,
        Instant::now(),
        Duration::from_secs(1_200),
    );
    let said = brain.take_said();
    let trial = said
        .iter()
        .filter(|l| l.starts_with("a rail depot placement would be refused"))
        .count();
    assert_eq!(trial, 3, "its three straight places: {said:?}");
    let depot = world
        .depots
        .iter()
        .find(|d| d.file.as_str().contains("/rail/"))
        .expect("a rail depot");
    assert!(depot.connected, "joined by its curved track: {said:?}");
    assert!(
        P::of(depot.at).flat_distance(P::of(pos(1_800.0, 900.0))) < 700.0,
        "still by the sawmill"
    );
    assert!(
        said.iter()
            .any(|l| l.starts_with("runs a train on a rail line carrying planks")),
        "{said:?}"
    );
}

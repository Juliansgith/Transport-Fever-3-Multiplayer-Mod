//! The brain against a stand-in world: it answers the brain's questions as
//! the mod does and applies its actions as every game would, so the brain's
//! whole round, from founding its company to buses on a line, plays here.

#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use tpf3mp_ai::{Brain, Out};
use tpf3mp_bridge::ai::{
    AiAnswer, AiCompany, AiDepot, AiLine, AiModel, AiQuestion, AiStation, AiStreet, AiTown,
    AiVehicle, AiVerdict,
};
use tpf3mp_proto::{
    AiDifficulty, BoundedVec, FixedBytes, PlayerId, RoomId, Text,
    action::{
        Action, CompanyOp, LineId, LoanOp, LoanTerms, Pos, Resolve, StationId, TownId, VehicleId,
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
                    .filter(|s| {
                        let m = mid(s);
                        let (dx, dy) = (f64::from(m.x - at.x), f64::from(m.y - at.y));
                        dx.hypot(dy) <= f64::from(*radius) * 1000.0
                    })
                    .cloned()
                    .collect();
                AiAnswer::Streets(BoundedVec::new(near).unwrap())
            }
            AiQuestion::Models { .. } => AiAnswer::Models(
                BoundedVec::new(vec![
                    AiModel {
                        model: Text::new("vehicle/bus/cheap.mdl").unwrap(),
                        price: 20_000,
                        running: 9_000,
                        capacity: 18,
                        speed: 40,
                        lifespan: 20,
                    },
                    AiModel {
                        model: Text::new("vehicle/bus/good.mdl").unwrap(),
                        price: 40_000,
                        running: 4_000,
                        capacity: 32,
                        speed: 55,
                        lifespan: 25,
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
            Action::BuildConstruction(build) => {
                // Its side of the street: the entrance's offset from origin.
                let [cos, sin, ..] = build.transform.basis;
                let side = (f64::from(sin), -f64::from(cos));
                let refused = self.refuse_depot_side.is_some_and(|s| {
                    // Side +1 of the grid's x streets puts the depot at
                    // +y: its -y axis then points down (0, -1).
                    (s > 0.0) == (side.1 < 0.0 || side.0 < 0.0)
                });
                !refused
            }
            Action::BuildRoad(road) => {
                matches!(road.polyline.vertices[0].resolve, Resolve::Node(_))
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
            Action::BuildConstruction(build) => {
                let [cos, sin, ..] = build.transform.basis;
                let o = build.transform.origin;
                #[allow(clippy::cast_possible_truncation)]
                let entrance = Pos {
                    x: o.x + (35.0 * f64::from(sin) / 1e6 * 1000.0) as i32,
                    y: o.y - (35.0 * f64::from(cos) / 1e6 * 1000.0) as i32,
                    z: o.z,
                };
                self.depots.push(AiDepot {
                    file: build.file,
                    at: o,
                    entrance: Some(entrance),
                    connected: false,
                });
                self.money -= 120_000;
            }
            Action::BuildRoad(road) => {
                let from = road.polyline.vertices[0].pos;
                if let Some(d) = self.depots.iter_mut().find(|d| d.entrance == Some(from)) {
                    d.connected = true;
                }
            }
            Action::CreateLine(create) => {
                let id = self.id();
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
                let id = self.id();
                self.vehicles.push(AiVehicle {
                    id: VehicleId(id),
                    line: None,
                    model: buy.consist[0].model.clone(),
                    age: 0,
                    idle: true,
                });
                self.money -= 30_000;
            }
            Action::AssignLine(assign) => {
                let line = assign.line.unwrap();
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

fn mid(s: &AiStreet) -> Pos {
    Pos {
        x: (s.a.x + s.b.x) / 2,
        y: (s.a.y + s.b.y) / 2,
        z: (s.a.z + s.b.z) / 2,
    }
}

fn street(a: Pos, b: Pos) -> AiStreet {
    AiStreet {
        a,
        b,
        template: Text::new("::/street/town_small.street_template").unwrap(),
        length: 100,
        owned: false,
        objects: 0,
        structure: false,
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
            .any(|l| l.contains("gave up on a line in Didcot")),
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

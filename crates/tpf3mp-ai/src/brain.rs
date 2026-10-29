//! One AI player's mind (docs/AI_PLAYERS.md, "How the AI plays").
//!
//! The brain is a state machine its driver polls: it asks the driving game
//! one question at a time ([`Out::Ask`]), and sends one action at a time
//! ([`Out::Act`]) once the game's verdict on it is good, at no more than its
//! difficulty's pace. It confirms every action by reading the world again,
//! never by trusting that it happened. What it keeps for its next driver
//! travels as its memo ([`Brain::memo`]).
//!
//! Its first plays are road passenger lines: it founds its company, picks a
//! town, stops buses on the town's own streets by its districts' centres,
//! puts a road depot beside a street and joins it, opens the line and buys
//! buses onto it; then it keeps each line's buses at its difficulty's count
//! and moves on to the next town while its money lasts.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tpf3mp_bridge::ai::{AiAnswer, AiCompany, AiDepot, AiModel, AiQuestion, AiStreet, AiTown};
use tpf3mp_proto::{
    AiDifficulty, BoundedVec, IntentRejection, Payload, PlayerId, Text,
    action::{
        Action, AssignLine, BuyVehicle, CompanyOp, ConsistPart, ConstructionRef, CreateLine,
        LineData, LineId, LineStop, LoadMode, LoanOp, Pos, StationId, StopRules, Terminal, Tint,
        TownId, VehicleId,
    },
};

use crate::{
    place::{self, P},
    profile::Profile,
};

/// How long the brain waits for an answer before it asks again.
pub const ANSWER_WAIT: Duration = Duration::from_secs(15);
/// How long it gives the room to order and apply an action before it reads
/// the world to see it, and how often it reads again until it does.
pub const SETTLE: Duration = Duration::from_millis(1_500);
/// Reads after an action before the brain gives that step up.
pub const CONFIRM_TRIES: u32 = 8;
/// How far around a district's centre it looks for a street to stop on, in
/// metres.
pub const STOP_SEARCH: u32 = 260;
/// How close to the place it stopped at a station must stand to be the one
/// it built, in metres.
pub const MATCH_DISTANCE: f64 = 30.0;
/// The least distance between two of a line's stops, in metres.
pub const STOP_SPACING: f64 = 220.0;
/// Towns it gives up on stay given up on for this long.
pub const GIVE_UP_FOR: Duration = Duration::from_secs(900);
/// Bus models' transport mode, by the game's name.
pub const BUS: &str = "BUS";
/// The game's number for buses (`TransportMode.BUS`), as lines list modes.
pub const BUS_MODE: u16 = 3;
/// Below this much money a mind borrows, while its difficulty lets it: a
/// bus line costs about this much in the 1920s.
pub const BORROW_BELOW: i64 = 400_000;

/// What the brain wants of its driver now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    /// Ask the driving game this.
    Ask(AiQuestion),
    /// Send this action for the seat.
    Act(Payload),
}

/// What it keeps for its next driver: the towns it serves, and those it
/// gave up on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memo {
    pub served: Vec<u32>,
    pub given_up: Vec<u32>,
}

/// A bus line it is building in a town.
#[derive(Debug, Clone)]
struct Project {
    town: AiTown,
    /// Districts' centres still to find a street by, nearest first.
    centres: Vec<Pos>,
    /// The streets found by each centre, best first.
    sites: Vec<Vec<AiStreet>>,
    /// Every street seen around the town, for a depot.
    streets: Vec<AiStreet>,
    /// Where it stopped buses, and the station there once seen.
    stops: Vec<(P, Option<StationId>)>,
    depot: Option<AiDepot>,
    /// The street the depot stands by, for its join.
    depot_street: Option<AiStreet>,
    /// Depot placements still to try: street index and side.
    depot_tries: Vec<(usize, f64)>,
    line: Option<LineId>,
}

/// Where the brain is.
#[derive(Debug, Clone)]
enum Stage {
    /// Reading its company.
    Look,
    /// Its company's founding sent: reading until it plays for it.
    Founding { tries: u32 },
    /// A loan taken: reading until it owes one more.
    Borrowing { before: u8, tries: u32 },
    /// Choosing a town.
    Survey,
    /// Reading the streets by a centre of the project's town.
    Streets,
    /// Asking the verdict on the next stop.
    StopVerdict { site: usize, choice: usize },
    /// A stop sent: reading until its station stands.
    StopBuilt {
        site: usize,
        choice: usize,
        tries: u32,
    },
    /// Asking the verdict on the next depot placement.
    DepotVerdict,
    /// A depot sent: reading until it stands.
    DepotBuilt { at: Pos, tries: u32 },
    /// The join's verdict asked.
    JoinVerdict,
    /// The join sent: reading until the depot reaches the street.
    Joined { tries: u32 },
    /// The line sent: reading until it exists.
    LineMade { tries: u32 },
    /// Reading the bus models of the year.
    Models,
    /// A bus bought: reading until it stands in the depot.
    Bought { before: Vec<VehicleId>, tries: u32 },
    /// A bus sent onto the line: reading until it runs it.
    Assigned { vehicle: VehicleId, tries: u32 },
    /// Nothing to do until its next look.
    Rest,
}

/// One AI player's mind.
#[derive(Debug, Clone)]
pub struct Brain {
    seat: PlayerId,
    name: String,
    profile: Profile,
    memo: Memo,
    rng: u64,
    stage: Stage,
    /// The question asked and not answered, and when.
    asked: Option<(AiQuestion, Instant)>,
    /// When it last sent an action, and when it may look again.
    acted: Option<Instant>,
    wake: Option<Instant>,
    company: Option<AiCompany>,
    project: Option<Project>,
    /// The bus it buys this year, once read.
    model: Option<AiModel>,
    /// The line it adds a bus to outside a project.
    topping: Option<(LineId, AiDepot)>,
    /// Towns given up on, until when.
    given_up: Vec<(u32, Instant)>,
    /// What happened last, for the driver's log.
    said: Vec<String>,
}

fn tint(seat: &PlayerId) -> Tint {
    let b = seat.as_bytes();
    let channel = |v: u8| 200_000 + i32::from(v) * 3_000;
    Tint {
        r: channel(b[16]),
        g: channel(b[17]),
        b: channel(b[18]),
    }
}

impl Brain {
    /// A mind for `seat`, carrying on from `memo` when its last driver left
    /// one.
    pub fn new(seat: PlayerId, name: &str, difficulty: AiDifficulty, memo: Option<&[u8]>) -> Self {
        let memo = memo
            .and_then(|bytes| postcard::from_bytes::<Memo>(bytes).ok())
            .unwrap_or_default();
        let b = seat.as_bytes();
        let rng = u64::from_le_bytes([b[16], b[17], b[18], b[19], b[20], b[21], b[22], b[23]]) | 1;
        Self {
            seat,
            name: name.to_owned(),
            profile: Profile::of(difficulty),
            memo,
            rng,
            stage: Stage::Look,
            asked: None,
            acted: None,
            wake: None,
            company: None,
            project: None,
            model: None,
            topping: None,
            given_up: Vec::new(),
            said: Vec::new(),
        }
    }

    pub fn seat(&self) -> PlayerId {
        self.seat
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// What it keeps for its next driver.
    pub fn memo(&self) -> Vec<u8> {
        postcard::to_allocvec(&self.memo).unwrap_or_default()
    }

    /// What it did since the last call, for the driver's log.
    pub fn take_said(&mut self) -> Vec<String> {
        std::mem::take(&mut self.said)
    }

    fn say(&mut self, line: String) {
        self.said.push(line);
    }

    fn random(&mut self) -> u64 {
        // xorshift64: a stream of its own, the same for the same seat.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    fn ask(&mut self, question: AiQuestion, now: Instant) -> Option<Out> {
        self.asked = Some((question.clone(), now));
        Some(Out::Ask(question))
    }

    fn may_act(&self, now: Instant) -> bool {
        self.acted
            .is_none_or(|at| now.saturating_duration_since(at) >= self.profile.action_gap)
    }

    /// Sends `action` if its hands are free; else waits.
    fn act(&mut self, action: &Action, now: Instant) -> Option<Out> {
        if !self.may_act(now) {
            return None;
        }
        match action.to_payload() {
            Ok(payload) => {
                self.acted = Some(now);
                self.wake = Some(now + SETTLE);
                Some(Out::Act(payload))
            }
            Err(error) => {
                self.say(format!("could not put an action: {error}"));
                self.give_up_project(now);
                None
            }
        }
    }

    fn verdict(&mut self, action: &Action, now: Instant) -> Option<Out> {
        match action.to_payload() {
            Ok(payload) => self.ask(AiQuestion::Verdict(payload), now),
            Err(error) => {
                self.say(format!("could not put an action: {error}"));
                self.give_up_project(now);
                None
            }
        }
    }

    fn give_up_project(&mut self, now: Instant) {
        if let Some(project) = self.project.take() {
            let id = project.town.id.0;
            self.say(format!(
                "gave up on a line in {}",
                project.town.name.as_str()
            ));
            self.given_up.push((id, now + GIVE_UP_FOR));
            if !self.memo.given_up.contains(&id) {
                self.memo.given_up.push(id);
            }
        }
        self.topping = None;
        self.rest(now);
    }

    fn rest(&mut self, now: Instant) {
        self.stage = Stage::Rest;
        self.wake = Some(now + self.profile.think_every);
    }

    /// What it wants now, if anything: one question or one action at a time.
    /// `speed` is the room's, in percent, for how soon it thinks again.
    pub fn poll(&mut self, now: Instant, speed: u16) -> Option<Out> {
        if let Some((question, at)) = &self.asked {
            if now.saturating_duration_since(*at) < ANSWER_WAIT {
                return None;
            }
            // No answer: ask again.
            let question = question.clone();
            return self.ask(question, now);
        }
        if self.wake.is_some_and(|wake| now < wake) {
            return None;
        }
        self.wake = None;
        match self.stage.clone() {
            Stage::Rest => {
                self.stage = Stage::Look;
                self.wake = Some(now + self.profile.think_after(speed));
                self.ask(AiQuestion::Company, now)
            }
            Stage::Look
            | Stage::Founding { .. }
            | Stage::Borrowing { .. }
            | Stage::StopBuilt { .. }
            | Stage::DepotBuilt { .. }
            | Stage::Joined { .. }
            | Stage::LineMade { .. }
            | Stage::Bought { .. }
            | Stage::Assigned { .. } => self.ask(AiQuestion::Company, now),
            Stage::Survey => self.ask(AiQuestion::Towns, now),
            Stage::Streets => {
                let at = self.project.as_ref()?.centres.first().copied()?;
                self.ask(
                    AiQuestion::Streets {
                        at,
                        radius: STOP_SEARCH,
                    },
                    now,
                )
            }
            Stage::StopVerdict { site, choice } => {
                let year = self.year();
                let street = self.project.as_ref()?.sites.get(site)?.get(choice)?.clone();
                self.verdict(&Action::PlaceStop(place::stop(&street, year)), now)
            }
            Stage::DepotVerdict => match self.next_depot() {
                Some(build) => self.verdict(&Action::BuildConstruction(build), now),
                None => {
                    self.say("found no place for a depot".into());
                    self.give_up_project(now);
                    None
                }
            },
            Stage::JoinVerdict => {
                let project = self.project.as_ref()?;
                let entrance = project.depot.as_ref()?.entrance?;
                let street = project.depot_street.clone()?;
                match place::join(entrance, &street) {
                    Some(road) => self.verdict(&Action::BuildRoad(road), now),
                    None => {
                        self.give_up_project(now);
                        None
                    }
                }
            }
            Stage::Models => self.ask(
                AiQuestion::Models {
                    mode: Text::lossy(BUS),
                },
                now,
            ),
        }
    }

    fn year(&self) -> i32 {
        self.company.as_ref().map_or(1900, |c| c.year)
    }

    fn money(&self) -> i64 {
        self.company.as_ref().map_or(0, |c| c.money)
    }

    /// Money it may spend now: what it has over its reserve.
    fn spendable(&self) -> i64 {
        let money = self.money();
        money - money.max(0) * self.profile.reserve_percent / 100
    }

    /// The driving game's answer to its question, or why it has none.
    pub fn answered(&mut self, answer: Result<AiAnswer, String>, now: Instant) -> Option<Out> {
        let (question, _) = self.asked.take()?;
        let answer = match answer {
            Ok(answer) => answer,
            Err(why) => {
                self.say(format!("the game did not answer: {why}"));
                // Ask again after a while: a world on its way, say.
                self.asked = None;
                self.wake = Some(now + SETTLE);
                return None;
            }
        };
        match (question, answer) {
            (AiQuestion::Company, AiAnswer::Company(company)) => {
                self.company = Some(company);
                self.on_company(now)
            }
            (AiQuestion::Towns, AiAnswer::Towns(towns)) => {
                self.on_towns(towns.into_inner(), now);
                None
            }
            (AiQuestion::Streets { .. }, AiAnswer::Streets(streets)) => {
                self.on_streets(streets.into_inner(), now);
                None
            }
            (AiQuestion::Models { .. }, AiAnswer::Models(models)) => {
                self.on_models(models.into_inner(), now)
            }
            (AiQuestion::Verdict(payload), AiAnswer::Verdict(verdict)) => {
                if verdict.ok && verdict.cost <= self.spendable().max(0) {
                    let action = Action::from_payload(&payload).ok()?;
                    self.on_good_verdict(&action, now)
                } else {
                    let why = verdict
                        .messages
                        .first()
                        .map_or("too dear", |m| m.as_str())
                        .to_owned();
                    self.on_bad_verdict(&why, now);
                    None
                }
            }
            _ => {
                self.say("the game answered another question".into());
                self.wake = Some(now + SETTLE);
                None
            }
        }
    }

    /// The room refused an action it sent.
    pub fn rejected(&mut self, reason: IntentRejection, now: Instant) {
        self.say(format!("the room refused its action: {reason:?}"));
        if matches!(reason, IntentRejection::RateLimited) {
            // Its next look sees what happened; send nothing sooner.
            self.acted = Some(now);
            return;
        }
        self.give_up_project(now);
    }

    fn on_company(&mut self, now: Instant) -> Option<Out> {
        let company = self.company.clone()?;
        match self.stage.clone() {
            Stage::Look => {
                if company.company.is_none() {
                    // Its first act: found its own company.
                    let action = Action::CompanyOp(CompanyOp::Create {
                        name: Text::lossy(&self.name),
                    });
                    self.stage = Stage::Founding { tries: 0 };
                    let out = self.act(&action, now);
                    if out.is_none() {
                        self.stage = Stage::Look;
                        self.wake = Some(now + SETTLE);
                    } else {
                        self.say(format!("founds {}", self.name));
                    }
                    return out;
                }
                self.think(&company, now)
            }
            Stage::Founding { tries } => {
                if company.company.is_some() {
                    self.say(format!("plays for {}", self.name));
                    self.stage = Stage::Look;
                    return self.think(&company, now);
                }
                self.retry(Stage::Founding { tries: tries + 1 }, tries, now);
                None
            }
            Stage::Borrowing { before, tries } => {
                if company.loans > before {
                    self.say(format!("borrowed; it has {}", company.money));
                    self.stage = Stage::Look;
                    return self.think(&company, now);
                }
                if tries >= CONFIRM_TRIES {
                    self.say("its loan did not come".into());
                    self.rest(now);
                } else {
                    self.retry(
                        Stage::Borrowing {
                            before,
                            tries: tries + 1,
                        },
                        tries,
                        now,
                    );
                }
                None
            }
            Stage::StopBuilt {
                site,
                choice,
                tries,
            } => {
                let project = self.project.as_mut()?;
                let (at, _) = *project.stops.last()?;
                if let Some(station) = company
                    .stations
                    .iter()
                    .find(|s| P::of(s.pos).flat_distance(at) <= MATCH_DISTANCE)
                {
                    if let Some(last) = project.stops.last_mut() {
                        last.1 = Some(station.id);
                    }
                    self.next_stop(site + 1, now);
                } else if tries >= CONFIRM_TRIES {
                    // Try the next street by that centre.
                    project.stops.pop();
                    self.stage = Stage::StopVerdict {
                        site,
                        choice: choice + 1,
                    };
                } else {
                    self.retry(
                        Stage::StopBuilt {
                            site,
                            choice,
                            tries: tries + 1,
                        },
                        tries,
                        now,
                    );
                }
                None
            }
            Stage::DepotBuilt { at, tries } => {
                let found = company
                    .depots
                    .iter()
                    .find(|d| P::of(d.at).flat_distance(P::of(at)) <= 2.0 && d.entrance.is_some())
                    .cloned();
                match found {
                    Some(depot) => {
                        let project = self.project.as_mut()?;
                        project.depot = Some(depot);
                        self.stage = Stage::JoinVerdict;
                    }
                    None if tries >= CONFIRM_TRIES => {
                        self.stage = Stage::DepotVerdict;
                    }
                    None => self.retry(
                        Stage::DepotBuilt {
                            at,
                            tries: tries + 1,
                        },
                        tries,
                        now,
                    ),
                }
                None
            }
            Stage::Joined { tries } => {
                let project = self.project.as_mut()?;
                let at = project.depot.as_ref()?.at;
                let joined = company
                    .depots
                    .iter()
                    .find(|d| d.at == at)
                    .is_some_and(|d| d.connected);
                if joined {
                    if let Some(depot) = project.depot.as_mut() {
                        depot.connected = true;
                    }
                    return self.make_line(now);
                }
                if tries >= CONFIRM_TRIES {
                    self.say("its depot did not reach the street".into());
                    self.give_up_project(now);
                } else {
                    self.retry(Stage::Joined { tries: tries + 1 }, tries, now);
                }
                None
            }
            Stage::LineMade { tries } => {
                let project = self.project.as_mut()?;
                let stops: Vec<StationId> = project.stops.iter().filter_map(|(_, s)| *s).collect();
                if let Some(line) = company
                    .lines
                    .iter()
                    .find(|l| l.stops.iter().copied().eq(stops.iter().copied()))
                {
                    project.line = Some(line.id);
                    let town = project.town.name.as_str().to_owned();
                    self.say(format!("opened a bus line in {town}"));
                    self.stage = Stage::Models;
                } else if tries >= CONFIRM_TRIES {
                    self.give_up_project(now);
                } else {
                    self.retry(Stage::LineMade { tries: tries + 1 }, tries, now);
                }
                None
            }
            Stage::Bought { before, tries } => {
                let new = company
                    .vehicles
                    .iter()
                    .find(|v| v.line.is_none() && !before.contains(&v.id))
                    .map(|v| v.id);
                match new {
                    Some(vehicle) => {
                        let line = self.line_now()?;
                        let action = Action::AssignLine(AssignLine {
                            vehicles: BoundedVec::new(vec![vehicle]).ok()?,
                            line: Some(line),
                            first_stop: None,
                        });
                        self.stage = Stage::Assigned { vehicle, tries: 0 };
                        let out = self.act(&action, now);
                        if out.is_none() {
                            // Its hands are busy: try again soon.
                            self.stage = Stage::Bought { before, tries };
                            self.wake = Some(now + SETTLE);
                        }
                        out
                    }
                    None if tries >= CONFIRM_TRIES => {
                        self.say("its bus did not come".into());
                        self.give_up_project(now);
                        None
                    }
                    None => {
                        self.retry(
                            Stage::Bought {
                                before,
                                tries: tries + 1,
                            },
                            tries,
                            now,
                        );
                        None
                    }
                }
            }
            Stage::Assigned { vehicle, tries } => {
                let line = self.line_now()?;
                let runs = company
                    .vehicles
                    .iter()
                    .any(|v| v.id == vehicle && v.line == Some(line));
                if runs {
                    self.after_bus(&company, now)
                } else if tries >= CONFIRM_TRIES {
                    self.give_up_project(now);
                    None
                } else {
                    self.retry(
                        Stage::Assigned {
                            vehicle,
                            tries: tries + 1,
                        },
                        tries,
                        now,
                    );
                    None
                }
            }
            _ => None,
        }
    }

    fn retry(&mut self, stage: Stage, _tries: u32, now: Instant) {
        self.stage = stage;
        self.wake = Some(now + SETTLE);
    }

    /// The line it is filling with buses.
    fn line_now(&self) -> Option<LineId> {
        self.project
            .as_ref()
            .and_then(|p| p.line)
            .or_else(|| self.topping.as_ref().map(|(line, _)| *line))
    }

    fn depot_now(&self) -> Option<AiDepot> {
        self.project
            .as_ref()
            .and_then(|p| p.depot.clone())
            .or_else(|| self.topping.as_ref().map(|(_, depot)| depot.clone()))
    }

    /// A look at its company: borrow when short, tune its lines, else start
    /// a new one.
    fn think(&mut self, company: &AiCompany, now: Instant) -> Option<Out> {
        if company.money < BORROW_BELOW && company.loans < self.profile.loans_most {
            let offer = if self.profile.big_loans {
                company.offers.iter().max_by_key(|o| o.amount)
            } else {
                company.offers.iter().min_by_key(|o| o.amount)
            };
            if let Some(offer) = offer.cloned() {
                let action = Action::Loan(Box::new(LoanOp::Take {
                    next: offer.clone(),
                    offer,
                }));
                self.stage = Stage::Borrowing {
                    before: company.loans,
                    tries: 0,
                };
                let out = self.act(&action, now);
                if out.is_none() {
                    self.stage = Stage::Look;
                    self.wake = Some(now + SETTLE);
                }
                return out;
            }
        }
        // Its lines first: one short of its count gets a bus.
        let depot = company.depots.iter().find(|d| d.connected).cloned();
        if let Some(depot) = depot {
            let short = company
                .lines
                .iter()
                .find(|l| l.vehicles.len() < usize::from(self.profile.buses_first));
            if let Some(line) = short {
                self.topping = Some((line.id, depot));
                self.stage = Stage::Models;
                return None;
            }
        }
        self.stage = Stage::Survey;
        None
    }

    fn on_towns(&mut self, towns: Vec<AiTown>, now: Instant) {
        self.given_up.retain(|(_, until)| *until > now);
        let served: Vec<P> = self
            .company
            .as_ref()
            .map(|c| c.stations.iter().map(|s| P::of(s.pos)).collect())
            .unwrap_or_default();
        let candidates: Vec<AiTown> = towns
            .into_iter()
            .filter(|t| !self.memo.served.contains(&t.id.0))
            .filter(|t| !self.given_up.iter().any(|(id, _)| *id == t.id.0))
            // A town another of its lines already stops in is served.
            .filter(|t| !served.iter().any(|s| s.flat_distance(P::of(t.pos)) < 600.0))
            .filter(|t| t.centres.len() >= 2)
            .take(self.profile.towns_weighed)
            .collect();
        if candidates.is_empty() {
            self.say("found no town to serve".into());
            self.rest(now);
            return;
        }
        let among = self.profile.pick_among.min(candidates.len()).max(1);
        #[allow(clippy::cast_possible_truncation)]
        let pick = (self.random() % among as u64) as usize;
        let town = candidates[pick].clone();
        // Its money must stretch to a line: a rough first bound.
        if self.spendable() < 150_000 {
            self.say("waits for money".into());
            self.rest(now);
            return;
        }
        let centres: Vec<Pos> = town
            .centres
            .iter()
            .copied()
            .take(self.profile.stops_most)
            .collect();
        self.say(format!("plans a bus line in {}", town.name.as_str()));
        self.project = Some(Project {
            town,
            centres,
            sites: Vec::new(),
            streets: Vec::new(),
            stops: Vec::new(),
            depot: None,
            depot_street: None,
            depot_tries: Vec::new(),
            line: None,
        });
        self.stage = Stage::Streets;
    }

    fn on_streets(&mut self, streets: Vec<AiStreet>, now: Instant) {
        let Some(project) = self.project.as_mut() else {
            return;
        };
        let Some(centre) = project.centres.first().copied() else {
            return;
        };
        let centre = P::of(centre);
        let mut usable: Vec<AiStreet> = streets
            .iter()
            .filter(|s| place::usable(s))
            .cloned()
            .collect();
        usable.sort_by(|a, b| {
            place::middle(a)
                .flat_distance(centre)
                .total_cmp(&place::middle(b).flat_distance(centre))
        });
        usable.truncate(4);
        project.sites.push(usable);
        for street in streets {
            if !project
                .streets
                .iter()
                .any(|s| s.a == street.a && s.b == street.b)
            {
                project.streets.push(street);
            }
        }
        project.centres.remove(0);
        if project.centres.is_empty() {
            self.next_stop(0, now);
        } else {
            self.stage = Stage::Streets;
        }
    }

    /// On to stop number `site`, or to the depot once every centre has one
    /// (or has none to have).
    fn next_stop(&mut self, site: usize, now: Instant) {
        let Some(project) = self.project.as_ref() else {
            return;
        };
        if site < project.sites.len() {
            self.stage = Stage::StopVerdict { site, choice: 0 };
            return;
        }
        let stops = project.stops.iter().filter(|(_, s)| s.is_some()).count();
        if stops < 2 {
            self.say("found no two places to stop buses".into());
            self.give_up_project(now);
            return;
        }
        self.plan_depot();
    }

    fn plan_depot(&mut self) {
        // A depot it has, joined, serves this line too.
        let joined = self
            .company
            .as_ref()
            .and_then(|c| c.depots.iter().find(|d| d.connected).cloned());
        let Some(project) = self.project.as_mut() else {
            return;
        };
        if let Some(depot) = joined {
            project.depot = Some(depot);
            self.stage = Stage::JoinVerdict;
            // Already joined: straight to the line.
            self.stage = Stage::Joined { tries: 0 };
            return;
        }
        let stops: Vec<P> = project.stops.iter().map(|(p, _)| *p).collect();
        let mut tries = Vec::new();
        let mut order: Vec<usize> = (0..project.streets.len())
            .filter(|&i| place::usable(&project.streets[i]) && project.streets[i].length >= 40)
            .filter(|&i| {
                let m = place::middle(&project.streets[i]);
                stops.iter().all(|s| s.flat_distance(m) > 60.0)
            })
            .collect();
        // Away from the town's middle, where there is room.
        let centre = P::of(project.town.pos);
        order.sort_by(|&a, &b| {
            place::middle(&project.streets[b])
                .flat_distance(centre)
                .total_cmp(&place::middle(&project.streets[a]).flat_distance(centre))
        });
        for i in order.into_iter().take(4) {
            tries.push((i, 1.0));
            tries.push((i, -1.0));
        }
        project.depot_tries = tries;
        self.stage = Stage::DepotVerdict;
    }

    fn next_depot(&mut self) -> Option<tpf3mp_proto::action::ConstructionBuild> {
        let year = self.year();
        #[allow(clippy::cast_possible_wrap)]
        let seed = (self.random() >> 33) as i64;
        let name = format!("{} Depot", self.name);
        let project = self.project.as_mut()?;
        let (index, side) = project.depot_tries.first().copied()?;
        let street = project.streets.get(index)?.clone();
        let (_, build) = place::depot(&street, side, year, seed, &name);
        project.depot_street = Some(street);
        Some(build)
    }

    fn on_good_verdict(&mut self, action: &Action, now: Instant) -> Option<Out> {
        match (self.stage.clone(), action) {
            (Stage::StopVerdict { site, choice }, Action::PlaceStop(stop)) => {
                let at = P::of(stop.at);
                // Not too near a stop of this line it has already.
                let near = self.project.as_ref().is_some_and(|p| {
                    p.stops
                        .iter()
                        .any(|(s, _)| s.flat_distance(at) < STOP_SPACING)
                });
                if near {
                    self.stage = Stage::StopVerdict {
                        site,
                        choice: choice + 1,
                    };
                    self.skip_exhausted_site(site, now);
                    return None;
                }
                let out = self.act(action, now);
                if out.is_some() {
                    if let Some(project) = self.project.as_mut() {
                        project.stops.push((at, None));
                    }
                    self.stage = Stage::StopBuilt {
                        site,
                        choice,
                        tries: 0,
                    };
                } else {
                    self.wake = Some(now + SETTLE);
                }
                out
            }
            (Stage::DepotVerdict, Action::BuildConstruction(build)) => {
                let at = build.transform.origin;
                let out = self.act(action, now);
                if out.is_some() {
                    if let Some(project) = self.project.as_mut() {
                        project.depot_tries.remove(0);
                    }
                    self.stage = Stage::DepotBuilt { at, tries: 0 };
                } else {
                    self.wake = Some(now + SETTLE);
                }
                out
            }
            (Stage::JoinVerdict, Action::BuildRoad(_)) => {
                let out = self.act(action, now);
                if out.is_some() {
                    self.stage = Stage::Joined { tries: 0 };
                } else {
                    self.wake = Some(now + SETTLE);
                }
                out
            }
            _ => None,
        }
    }

    fn on_bad_verdict(&mut self, why: &str, now: Instant) {
        match self.stage.clone() {
            Stage::StopVerdict { site, choice } => {
                self.stage = Stage::StopVerdict {
                    site,
                    choice: choice + 1,
                };
                self.skip_exhausted_site(site, now);
            }
            Stage::DepotVerdict => {
                if let Some(project) = self.project.as_mut()
                    && !project.depot_tries.is_empty()
                {
                    project.depot_tries.remove(0);
                }
            }
            Stage::JoinVerdict => {
                self.say(format!("could not join its depot: {why}"));
                self.give_up_project(now);
            }
            _ => {}
        }
    }

    /// Past the last street by a centre: on to the next centre.
    fn skip_exhausted_site(&mut self, site: usize, now: Instant) {
        if let Stage::StopVerdict { choice, .. } = self.stage {
            let exhausted = self
                .project
                .as_ref()
                .is_none_or(|p| p.sites.get(site).is_none_or(|s| choice >= s.len()));
            if exhausted {
                self.next_stop(site + 1, now);
            }
        }
    }

    fn make_line(&mut self, now: Instant) -> Option<Out> {
        let project = self.project.as_ref()?;
        let stops: Vec<LineStop> = project
            .stops
            .iter()
            .filter_map(|(_, s)| *s)
            .map(|group| LineStop {
                group,
                terminal: Terminal {
                    station: 0,
                    terminal: 0,
                },
                alternatives: BoundedVec::new(vec![Terminal {
                    station: 0,
                    terminal: 1,
                }])
                .unwrap_or_default(),
                load_mode: LoadMode::LoadIfAvailable,
                min_wait: 0,
                max_wait: 0,
                max_extra_wait: 0,
                rules: StopRules {
                    load: BoundedVec::new(vec![true]).unwrap_or_default(),
                    max_load: BoundedVec::new(vec![tpf3mp_proto::action::Fraction(1_000_000)])
                        .unwrap_or_default(),
                    force_unload: false,
                    destroy_for_config_change: false,
                    destroy_for_refresh: false,
                },
                waypoints: BoundedVec::default(),
            })
            .collect();
        let name = format!("{} Bus", project.town.name.as_str());
        let action = Action::CreateLine(CreateLine {
            name: Text::lossy(&name),
            color: tint(&self.seat),
            line: LineData {
                stops: BoundedVec::new(stops).ok()?,
                modes: BoundedVec::new(vec![BUS_MODE]).ok()?,
                custom_filters: false,
                reservation_priority: 0,
            },
        });
        self.stage = Stage::LineMade { tries: 0 };
        let out = self.act(&action, now);
        if out.is_none() {
            // Its hands are busy: the line waits for the next poll.
            self.stage = Stage::Joined { tries: 0 };
            self.wake = Some(now + SETTLE);
        }
        out
    }

    fn on_models(&mut self, models: Vec<AiModel>, now: Instant) -> Option<Out> {
        let usable: Vec<AiModel> = models.into_iter().filter(|m| m.capacity > 0).collect();
        let pick = if self.profile.best_models {
            // Seats and speed for the money, its running cost counted.
            usable.into_iter().max_by(|a, b| {
                let worth = |m: &AiModel| {
                    f64::from(m.capacity) * f64::from(m.speed.max(1))
                        / (m.price.max(1) as f64 + 5.0 * m.running.max(0) as f64)
                };
                worth(a).total_cmp(&worth(b))
            })
        } else {
            usable.into_iter().min_by_key(|m| m.price)
        };
        match pick {
            Some(model) => {
                self.model = Some(model);
                self.buy(now)
            }
            None => {
                self.say("found no bus to buy this year".into());
                self.give_up_project(now);
                None
            }
        }
    }

    /// Buys a bus for the line it fills, when its money allows.
    fn buy(&mut self, now: Instant) -> Option<Out> {
        let model = self.model.clone()?;
        let depot = self.depot_now()?;
        if model.price > self.spendable() {
            self.say("waits for money for a bus".into());
            self.project = None;
            self.topping = None;
            self.rest(now);
            return None;
        }
        let before: Vec<VehicleId> = self
            .company
            .as_ref()
            .map(|c| c.vehicles.iter().map(|v| v.id).collect())
            .unwrap_or_default();
        let action = Action::BuyVehicle(BuyVehicle {
            depot: ConstructionRef {
                file: depot.file.clone(),
                at: depot.at,
            },
            consist: BoundedVec::new(vec![ConsistPart {
                model: model.model.clone(),
                reversed: false,
                loads: BoundedVec::default(),
                color: tint(&self.seat),
            }])
            .ok()?,
            groups: BoundedVec::new(vec![1]).ok()?,
            multiple_units: BoundedVec::new(vec![Text::lossy("")]).ok()?,
            depot_index: 0,
        });
        self.stage = Stage::Bought { before, tries: 0 };
        let out = self.act(&action, now);
        if out.is_none() {
            self.stage = Stage::Models;
            self.wake = Some(now + SETTLE);
        }
        out
    }

    /// A bus runs its line: another, or the line is done.
    fn after_bus(&mut self, company: &AiCompany, now: Instant) -> Option<Out> {
        let line = self.line_now()?;
        let running = company
            .vehicles
            .iter()
            .filter(|v| v.line == Some(line))
            .count();
        if running < usize::from(self.profile.buses_first) {
            return self.buy(now);
        }
        if let Some(project) = self.project.take() {
            let id = project.town.id.0;
            if !self.memo.served.contains(&id) {
                self.memo.served.push(id);
            }
            self.say(format!(
                "runs {running} bus(es) in {}",
                project.town.name.as_str()
            ));
        }
        self.topping = None;
        self.rest(now);
        None
    }
}

/// Whether a town by its id is one the brain serves, as its memo says.
pub fn serves(memo: &[u8], town: TownId) -> bool {
    postcard::from_bytes::<Memo>(memo).is_ok_and(|m| m.served.contains(&town.0))
}

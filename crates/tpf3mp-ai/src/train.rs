//! The brain's rail lines (docs/AI_PLAYERS.md, "Track and trains"): a
//! cargo carried by train from the industry that makes it to a town that
//! needs it, where the two are too far apart for trucks to pay. It chooses
//! its train's wagons, reads the land around both ends and prices the line
//! as the game would ([`crate::costs`]) before it builds anything, then
//! chooses the train its money leaves room for. It puts a one-track cargo
//! station by each end, asks the game for the streets its route crosses,
//! lays out its track for the least cost (around, over or under, level
//! crossings where it meets a street on the ground), lays it a piece at a
//! time, each put to the game's verdict and routed round where the game
//! refuses one, puts a rail depot beyond the first station joined to its
//! track, opens the line and buys a train onto it. Every step is confirmed
//! by reading the world again.

use std::time::Instant;

use tpf3mp_bridge::ai::{
    AiCompany, AiDepot, AiIndustry, AiLand, AiModel, AiQuestion, AiStreet, AiTown,
};
use tpf3mp_proto::{
    BoundedVec, Text,
    action::{
        Action, AssignLine, BuyVehicle, ConsistPart, ConstructionRef, CreateLine, Fraction,
        LineData, LineId, LineStop, LoadMode, Pos, ResName, Resolve, StationId, StopRules,
        Terminal, TownId, TrackBuild, VehicleId, Vertex,
    },
};

use super::{Brain, CONFIRM_TRIES, GIVE_UP_FOR, Out, SETTLE, Stage, cargo_name, route_key, tint};
use crate::{
    chain::{Leg, Taker},
    costs::{self, Prices},
    place,
    place::P,
    rail::{self, Land, Node, Piece, Station},
};

/// The least distance a cargo goes by train, in metres, and the least
/// money a rail line is begun with: two stations and a depot cost about
/// 1.6 million in the 1920s, a train about 1 to 3, flat track 53 a metre.
pub const RAIL_FROM: f64 = 1_800.0;
pub const RAIL_MONEY: i64 = 6_000_000;
/// The modules of each station it builds.
const STATION_MODULES: i32 = 3;

/// The most pairs of station placements a line is laid out between before
/// it builds anything.
const SITE_PAIRS: u32 = 6;
/// The most places kept for each of a passenger line's stations, those
/// reaching the most people on foot: a refused station tries the next, and
/// a town's dense search ([`rail::PASSENGER_BEARINGS`]) finds hundreds.
const PASSENGER_PLACES: usize = 24;
/// How far the game's prices may stand from the AI's before it doubts its
/// reading of them.
const SCALE_RANGE: (f64, f64) = (0.25, 8.0);
/// The rings around an industry, and a town's district centre, its
/// stations are tried on, in metres from the place: the station's cargo
/// catchment reaches 150 m from its building (`main_building_1_cargo`).
const MAKER_RINGS: [f64; 3] = [150.0, 200.0, 250.0];
const TOWN_RINGS: [f64; 4] = [120.0, 180.0, 260.0, 340.0];
/// The rings a passenger station is tried on around its town's centre, 60
/// to 600 m out, [`rail::PASSENGER_RING_STEP`] apart: in among its houses,
/// which it may clear, and out to its edge, those reaching the most houses
/// on foot first (`rail::sites`).
fn passenger_rings() -> Vec<f64> {
    (0..)
        .map(|k| 60.0 + rail::PASSENGER_RING_STEP * f64::from(k))
        .take_while(|r| *r <= 600.0)
        .collect()
}
/// How many times a line is routed round pieces the game refused.
pub const REPLANS: u32 = 8;
/// Around a refused piece's middle the land is taken as built on, in
/// metres.
pub const REFUSED_REACH: f64 = 70.0;
/// The game's numbers for trains (`TransportMode.TRAIN`), as lines list
/// modes, and the mode's name.
pub const TRAIN_MODE: u16 = 7;
pub const TRAIN: &str = "TRAIN";
/// Passengers as the game names them among its cargo (cargo 0, class
/// PASSENGERS): a train's coaches are the models that carry them.
pub const PASSENGERS: &str = "::/cargos/passengers/passengers.cargo";
/// The most wagons a train pulls.
pub const WAGONS_MOST: usize = 6;
/// A rail stop's waits, in the game's seconds in millionths: as the game's
/// line window sets a rail stop's (`line_util.tl`, `makeNewStop`).
pub const RAIL_MAX_WAIT: i64 = 600_000_000;
pub const RAIL_EXTRA_WAIT: i64 = 60_000_000;

/// A station of a rail line once built: where it stands, the room's id
/// for it, and the end its line leaves from with the heading out of it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Built {
    pub station: Station,
    pub id: StationId,
    pub end: P,
    pub heading: f64,
}

/// A rail line it is building.
#[derive(Debug, Clone)]
pub(super) struct RailProject {
    pub cargo: ResName,
    pub from: AiIndustry,
    pub to: Taker,
    pub land: Land,
    /// Station placements still to try: by the industry, then the town.
    pub tries: [Vec<Station>; 2],
    pub built: [Option<Built>; 2],
    /// The prices it plans by.
    pub prices: Prices,
    /// What it reckoned the line costs before its train, from its first
    /// placements, before building anything.
    pub estimate: f64,
    /// The route being laid out: its nodes on the ground, the street runs
    /// it crosses still to ask the game about, and the streets named.
    pub aligned: Vec<Node>,
    pub runs: Vec<(P, u32)>,
    pub streets: Vec<AiStreet>,
    pub nodes: Vec<Node>,
    pub pieces: Vec<Piece>,
    pub piece: usize,
    pub replans: u32,
    /// Pairs of station placements it laid the line out between.
    pub pairs_tried: u32,
    /// The station placements its track was laid out for, before they
    /// were built.
    pub planned: Option<[P; 2]>,
    /// The depot and where its track joins, once sent and seen.
    pub depot: Option<(Pos, P)>,
    pub line: Option<LineId>,
    pub wagons: Vec<AiModel>,
    /// The train it chose before it built anything: engine and wagon.
    pub train: Option<(AiModel, AiModel)>,
    /// A rail depot of its own another line built, by its first station,
    /// that this line shares.
    pub share: Option<AiDepot>,
    /// Its passing loops: each one's first node of the line it runs beside,
    /// and its side.
    pub looped: Vec<(usize, f64)>,
    /// How many of them have both their signals.
    pub signalled: u8,
    /// It carries passengers between two towns, not a cargo: `from` stands
    /// for the first town, `cargo` is [`PASSENGERS`].
    pub passengers: bool,
    /// What its memo knows it by, served or given up on: a cargo's route
    /// key, or the two towns' pair ([`passenger_key`]).
    pub key: u32,
    /// Where its depot is tried at the station it tries now, as an index
    /// into [`rail::DEPOT_TRIES`] ([`rail::depot_spot`]); and the heading its track runs into
    /// the depot it built at, for the track that joins it.
    pub depot_far: usize,
    pub depot_into: f64,
    /// The station and place its depot's trial passed at, where its depot
    /// is tried first once its stations stand.
    pub depot_plan: Option<(usize, usize)>,
    /// The places a passenger station's own street may join the town's
    /// streets at, the nearest first ([`rail::FRONT_JOINS`] of them).
    pub front_joins: Vec<place::Join>,
}

/// Whether a rail line was given up for something the land decides: no
/// route, no place for a station or depot, track refused over and over.
pub fn lasting(why: &str) -> bool {
    why.starts_with("found no route")
        || why.starts_with("found no place")
        || why.contains("refused too often")
}

/// The key a passenger line between two towns goes by: the pair of their
/// ids, either way round, with the top bit set as a cargo route's. Not a
/// district centre's place, which moves as a town grows: the Hard AI chose
/// the towns of its first passenger line again three minutes after it
/// opened (2026-10-30, run 40).
pub fn passenger_key(a: TownId, b: TownId) -> u32 {
    let (low, high) = if a.0 <= b.0 { (a.0, b.0) } else { (b.0, a.0) };
    // FNV-1a: the same in every driver's agent.
    let mut h: u32 = 0x811c_9dc5;
    for byte in low
        .to_le_bytes()
        .into_iter()
        .chain(high.to_le_bytes())
        .chain(PASSENGERS.bytes())
    {
        h ^= u32::from(byte);
        h = h.wrapping_mul(0x0100_0193);
    }
    h | 0x8000_0000
}

impl RailProject {
    /// Each station's line end and the heading out of it, towards the
    /// place the other serves: as built, or as planned at its first
    /// placement.
    fn end(&self, end: usize) -> Option<(P, f64)> {
        if let Some(b) = self.built[end] {
            return Some((b.end, b.heading));
        }
        let other = P::of(if end == 0 {
            self.to.pos()
        } else {
            self.from.pos
        });
        Some(self.tries[end].first()?.ends_toward(other).0)
    }

    fn describe(&self) -> String {
        format!(
            "a rail line carrying {} from {} to {}",
            cargo_name(self.cargo.as_str()),
            self.from.name.as_str(),
            self.to.name()
        )
    }
}

/// Where a rail line stands.
#[derive(Debug, Clone)]
pub(super) enum RailStage {
    /// Reading the land around both ends, a tile at a time.
    Land,
    /// Asking the game for the streets its route crosses, a run at a time.
    Streets,
    /// Asking the verdict on the next placement of station `end` (0 by the
    /// industry, 1 in the town).
    StationVerdict {
        end: usize,
    },
    /// A station sent: reading until it stands.
    StationBuilt {
        end: usize,
        tries: u32,
    },
    /// Asking the verdict on a piece of its track before anything is
    /// built: the pieces' joins to its stations and to each other as new
    /// nodes.
    Trial {
        piece: usize,
    },
    /// Then the verdict on its depot at place place ([`rail::depot_spot`])
    /// beyond station end's planned site, before anything is built: a line
    /// with no place for a depot at either station is laid out elsewhere or
    /// given up then, not once its stations and track stand.
    DepotTrial {
        end: usize,
        place: usize,
    },
    /// Then the verdict on that depot's track to the station's planned end,
    /// before anything is built: the long run's cement railway was laid,
    /// its depot built, and the depot's track refused, the line given up
    /// with four million spent (run 44, 2026-10-31).
    LinkTrial {
        end: usize,
        place: usize,
    },
    /// Asking the verdict on the next piece of track.
    PieceVerdict,
    /// A piece sent: reading before the next.
    PieceBuilt {
        tries: u32,
    },
    /// Asking the verdict on the track from the first station's free end
    /// to the join of a rail depot of its own it shares, then reading until
    /// that end is taken.
    ShareVerdict,
    ShareBuilt {
        tries: u32,
    },
    /// Asking the verdict on a depot beyond station `end`'s other end.
    DepotVerdict {
        end: usize,
    },
    DepotBuilt {
        end: usize,
        tries: u32,
    },
    /// Asking the verdict on the depot's track to the station.
    LinkVerdict {
        end: usize,
    },
    LinkBuilt {
        tries: u32,
    },
    /// Asking the verdict on its passing loop number `lp`'s track, then
    /// reading before its signals.
    LoopVerdict {
        lp: usize,
    },
    LoopBuilt {
        lp: usize,
    },
    /// Asking the verdict on that loop's signal number `n` (0 on the line's
    /// track, 1 on the loop's), and reading after one is sent.
    SignalVerdict {
        lp: usize,
        n: usize,
        tries: u32,
    },
    SignalBuilt {
        lp: usize,
        n: usize,
    },
    /// Opening the line once its hands are free.
    Opening,
    LineMade {
        tries: u32,
    },
    /// Asking for the wagons that carry its cargo, then the engines, before
    /// it builds anything.
    Wagons,
    Engines,
    /// Buying the train it chose once its line is open.
    Buying,
    Bought {
        before: Vec<VehicleId>,
        tries: u32,
    },
    Assigned {
        vehicle: VehicleId,
        tries: u32,
    },
    /// A passenger line running: reading the streets by station `end`'s
    /// front, for a street of its own along it to join to them.
    FrontStreets {
        end: usize,
    },
    /// Asking the verdict on that street, along the front at
    /// [`rail::FRONT_X`]`[at]`, joined to the town's at the `join`th
    /// nearest place.
    FrontVerdict {
        end: usize,
        at: usize,
        join: usize,
    },
}

impl Brain {
    /// Begins a rail line carrying a cargo from the industry that makes it
    /// to its taker, if it can afford one: true if it did.
    pub(super) fn start_rail(&mut self, leg: Leg) -> bool {
        let key = route_key(leg.from.pos, leg.cargo.as_str());
        self.start_rail_of(leg, false, key)
    }

    /// Begins a passenger rail line between two towns, if it can afford one:
    /// true if it did. Its first end stands for the first town (no station
    /// of its own, nothing made).
    pub(super) fn start_passenger_rail(&mut self, a: &AiTown, b: &AiTown) -> bool {
        let (Some(&at_a), Some(&at_b)) = (a.centres.first(), b.centres.first()) else {
            return false;
        };
        let from = AiIndustry {
            name: a.name.clone(),
            pos: at_a,
            station: None,
            terminals: 0,
            inputs: BoundedVec::default(),
            outputs: BoundedVec::default(),
            water: false,
            carried: 0,
        };
        let to = Taker::Town {
            town: b.clone(),
            at: at_b,
        };
        let leg = Leg {
            cargo: Text::lossy(PASSENGERS),
            from,
            to,
        };
        self.start_rail_of(leg, true, passenger_key(a.id, b.id))
    }

    fn start_rail_of(&mut self, leg: Leg, passengers: bool, key: u32) -> bool {
        // A cargo railway waits on its lines' earnings; a passenger one on
        // stations that reach people enough (the Land stage).
        if self.spendable() < RAIL_MONEY || (!passengers && !self.trains_carried()) {
            return false;
        }
        // No new railway while its settled ones lose money together: the
        // long run's three each lost 0.6 to 1 million a year (2026-10-30).
        // A passenger train after one that lost money, none.
        let kind = if passengers {
            super::Kind::Passengers
        } else {
            super::Kind::Rail
        };
        if self
            .company
            .as_ref()
            .is_some_and(|c| !self.kind_pays(c, kind))
        {
            return false;
        }
        let Leg { cargo, from, to } = leg;
        let land = Land::around(P::of(from.pos), P::of(to.pos()));
        let project = RailProject {
            cargo,
            from,
            to,
            land,
            tries: [Vec::new(), Vec::new()],
            built: [None, None],
            prices: Prices {
                scale: self.rail_scale,
                trestle: false,
            },
            estimate: 0.0,
            aligned: Vec::new(),
            runs: Vec::new(),
            streets: Vec::new(),
            nodes: Vec::new(),
            pieces: Vec::new(),
            piece: 0,
            replans: 0,
            pairs_tried: 1,
            planned: None,
            depot: None,
            line: None,
            wagons: Vec::new(),
            train: None,
            looped: Vec::new(),
            signalled: 0,
            share: None,
            passengers,
            key,
            depot_far: 0,
            depot_into: 0.0,
            depot_plan: None,
            front_joins: Vec::new(),
        };
        self.say(format!("plans {}", project.describe()));
        self.rail = Some(project);
        // Its train first: no line is built for a cargo no wagon carries,
        // or with no engine it can afford.
        self.stage = Stage::Rail(RailStage::Wagons);
        true
    }

    fn give_up_rail(&mut self, why: &str, now: Instant) {
        if let Some(project) = self.rail.take() {
            let key = project.key;
            self.say(format!("{why}; gave up on {}", project.describe()));
            // A chain's leg is tried by truck where trucks reach, before
            // the chain goes no further.
            let leg = self.leg_now.take().filter(|leg| {
                leg.distance() <= self.profile.cargo_most && !self.no_rail.contains(&key)
            });
            if let Some(leg) = leg {
                self.say("tries its chain's leg by truck".into());
                self.no_rail.push(key);
                self.chain.push_front(leg);
            } else {
                self.given_up.push((key, now + GIVE_UP_FOR));
                // The land decides a route or a station's place, and the
                // land stays: such a line is given up for good, in the memo
                // its next driver keeps. Money comes back; a line too dear
                // is tried again. The Hard AI planned three passenger pairs
                // with no way through again every 15 minutes (2026-10-30).
                if lasting(why) && !self.memo.given_up.contains(&key) {
                    self.memo.given_up.push(key);
                }
            }
        }
        self.rest(now);
    }

    /// What its rail line wants now: one question or one action.
    pub(super) fn rail_poll(&mut self, stage: RailStage, now: Instant) -> Option<Out> {
        match stage {
            RailStage::Land => {
                let next = self.rail.as_ref()?.land.next_tile();
                match next {
                    Some((_, at)) => self.ask(
                        AiQuestion::Land {
                            at,
                            cells: rail::TILE,
                            step: rail::STEP as u16,
                        },
                        now,
                    ),
                    None => {
                        #[allow(clippy::cast_precision_loss)]
                        let money = self.spendable() as f64;
                        let project = self.rail.as_mut()?;
                        let (from, at) = (P::of(project.from.pos), P::of(project.to.pos()));
                        // By a town's district, or by the industry it feeds.
                        // A passenger line's ends are both towns'.
                        let passengers = project.passengers;
                        let town_rings = passenger_rings();
                        let rings: &[f64] = match (&project.to, passengers) {
                            (_, true) => &town_rings,
                            (Taker::Town { .. }, false) => &TOWN_RINGS,
                            (Taker::Industry(_), false) => &MAKER_RINGS,
                        };
                        let near: &[f64] = if passengers {
                            &town_rings
                        } else {
                            &MAKER_RINGS
                        };
                        project.tries = [
                            rail::sites(&project.land, from, at, near, passengers),
                            rail::sites(&project.land, at, from, rings, passengers),
                        ];
                        // A passenger station reaching too few houses on
                        // foot is no place for one: a line with none such
                        // at either end is not begun.
                        if passengers {
                            let land = &project.land;
                            for tries in &mut project.tries {
                                tries.retain(|s| land.people(s) >= rail::PEOPLE_LEAST);
                                tries.truncate(PASSENGER_PLACES);
                            }
                            if project.tries.iter().any(Vec::is_empty) {
                                self.give_up_rail(
                                    "found no station place reaching people enough",
                                    now,
                                );
                                return None;
                            }
                        }
                        // What the line costs, from its first placements,
                        // before it builds anything: a line it cannot pay
                        // for, or with no route, is never begun.
                        match estimate(project) {
                            Ok(cost) if cost < money => {
                                project.estimate = cost;
                                self.say(format!(
                                    "reckons its rail line costs {cost:.0} before its train"
                                ));
                                self.stage = Stage::Rail(RailStage::Engines);
                                self.rail_poll(RailStage::Engines, now)
                            }
                            Ok(cost) => {
                                self.give_up_rail(
                                    &format!("found its rail line too dear at {cost:.0}"),
                                    now,
                                );
                                None
                            }
                            Err(why) => {
                                self.give_up_rail(why, now);
                                None
                            }
                        }
                    }
                }
            }
            RailStage::Streets => {
                let (at, radius) = *self.rail.as_ref()?.runs.first()?;
                self.ask(
                    AiQuestion::Streets {
                        at: at.pos(),
                        radius,
                    },
                    now,
                )
            }
            RailStage::StationVerdict { end } => {
                let year = self.year();
                #[allow(clippy::cast_possible_wrap)]
                let seed = (self.random() >> 33) as i64;
                let project = self.rail.as_ref()?;
                let name = match (project.passengers, end) {
                    (true, 0) => format!("{} Station", project.from.name.as_str()),
                    (true, _) => format!("{} Station", project.to.name()),
                    (false, 0) => format!("{} Rail", project.from.name.as_str()),
                    (false, _) => format!("{} Goods", project.to.name()),
                };
                let Some(station) = project.tries[end].first() else {
                    self.give_up_rail("found no place for a rail station", now);
                    return None;
                };
                match station.build(year, seed, &name) {
                    Some(build) => self.verdict(&Action::BuildConstruction(build), now),
                    None => {
                        self.give_up_rail("could not lay out a rail station", now);
                        None
                    }
                }
            }
            RailStage::Trial { piece } => {
                let project = self.rail.as_ref()?;
                let build = trial(&project.pieces.get(piece)?.build)?;
                self.verdict(&Action::BuildTrack(build), now)
            }
            RailStage::DepotTrial { end, place } => {
                let year = self.year();
                let project = self.rail.as_ref()?;
                let station = project.tries[end].first()?;
                let other = P::of(if end == 0 {
                    project.to.pos()
                } else {
                    project.from.pos
                });
                let (_, (_, _, build)) = depot_by(station, other, place, year, 0, "Rail Depot")?;
                self.verdict(&Action::BuildConstruction(build), now)
            }
            RailStage::LinkTrial { end, place } => {
                let year = self.year();
                let project = self.rail.as_ref()?;
                let station = project.tries[end].first()?;
                let other = P::of(if end == 0 {
                    project.to.pos()
                } else {
                    project.from.pos
                });
                let ((depot_end, out), (join, into, _)) =
                    depot_by(station, other, place, year, 0, "")?;
                let link = trial(&rail::depot_link(depot_end, out, join, into, year)?)?;
                self.verdict(&Action::BuildTrack(link), now)
            }
            RailStage::PieceVerdict => {
                let project = self.rail.as_ref()?;
                let piece = project.pieces.get(project.piece)?.build.clone();
                self.verdict(&Action::BuildTrack(piece), now)
            }
            RailStage::ShareVerdict => {
                let year = self.year();
                let project = self.rail.as_ref()?;
                let link = share_link(project, year)?;
                self.verdict(&Action::BuildTrack(link), now)
            }
            RailStage::DepotVerdict { end } => {
                let year = self.year();
                #[allow(clippy::cast_possible_wrap)]
                let seed = (self.random() >> 33) as i64;
                let project = self.rail.as_ref()?;
                let built = project.built[end]?;
                let other = P::of(if end == 0 {
                    project.to.pos()
                } else {
                    project.from.pos
                });
                let name = format!("{} Rail Depot", project.from.name.as_str());
                match depot_by(&built.station, other, project.depot_far, year, seed, &name) {
                    Some((_, (_, _, build))) => {
                        self.verdict(&Action::BuildConstruction(build), now)
                    }
                    None => {
                        self.give_up_rail("could not lay out a rail depot", now);
                        None
                    }
                }
            }
            RailStage::LinkVerdict { end } => {
                let year = self.year();
                let project = self.rail.as_ref()?;
                let built = project.built[end]?;
                let other = P::of(if end == 0 {
                    project.to.pos()
                } else {
                    project.from.pos
                });
                let ((depot_end, out), _) =
                    rail::depot_spot(&built.station, other, project.depot_far)?;
                let (_, join) = project.depot?;
                // Curving from the station's heading to the depot's.
                match rail::depot_link(depot_end, out, join, project.depot_into, year) {
                    Some(link) => self.verdict(&Action::BuildTrack(link), now),
                    None => {
                        self.give_up_rail("could not lay out a depot's track", now);
                        None
                    }
                }
            }
            RailStage::FrontStreets { end } => {
                let project = self.rail.as_ref()?;
                let (a, b) = project.built[end]?
                    .station
                    .front(rail::FRONT_X[0], &project.land);
                let at = P {
                    x: (a.x + b.x) / 2.0,
                    y: (a.y + b.y) / 2.0,
                    z: a.z,
                };
                self.ask(
                    AiQuestion::Streets {
                        at: at.pos(),
                        radius: rail::FRONT_REACH,
                    },
                    now,
                )
            }
            RailStage::FrontVerdict { end, at, join } => {
                let project = self.rail.as_ref()?;
                let station = project.built[end]?.station;
                let join = project.front_joins.get(join)?;
                match place::front_street(station.front(rail::FRONT_X[at], &project.land), join) {
                    Some(road) => self.verdict(&Action::BuildRoad(road), now),
                    None => {
                        self.next_front(end, now);
                        None
                    }
                }
            }
            RailStage::LoopVerdict { lp } => {
                let year = self.year();
                let project = self.rail.as_ref()?;
                let (k, side) = *project.looped.get(lp)?;
                let build = rail::passing_loop(&project.nodes, k, side, year)?;
                self.verdict(&Action::BuildTrack(build), now)
            }
            RailStage::SignalVerdict { lp, n, .. } => {
                let year = self.year();
                let project = self.rail.as_ref()?;
                let (k, side) = *project.looped.get(lp)?;
                let signal = rail::loop_signals(&project.nodes, k, side, year)[n].clone();
                self.verdict(&Action::PlaceStop(signal), now)
            }
            RailStage::Opening => self.open_rail(now),
            RailStage::Buying => self.buy_train(now),
            RailStage::Wagons => {
                let cargo = self.rail.as_ref()?.cargo.clone();
                self.ask(
                    AiQuestion::Models {
                        mode: Text::lossy(TRAIN),
                        cargo: Some(cargo),
                    },
                    now,
                )
            }
            RailStage::Engines => self.ask(
                AiQuestion::Models {
                    mode: Text::lossy(TRAIN),
                    cargo: None,
                },
                now,
            ),
            RailStage::StationBuilt { .. }
            | RailStage::PieceBuilt { .. }
            | RailStage::DepotBuilt { .. }
            | RailStage::LinkBuilt { .. }
            | RailStage::ShareBuilt { .. }
            | RailStage::LoopBuilt { .. }
            | RailStage::SignalBuilt { .. }
            | RailStage::LineMade { .. }
            | RailStage::Bought { .. }
            | RailStage::Assigned { .. } => self.ask(AiQuestion::Company, now),
        }
    }

    /// A tile of the land read.
    pub(super) fn rail_land(&mut self, answer: &AiLand) {
        if let Some(project) = self.rail.as_mut()
            && let Some((n, _)) = project.land.next_tile()
        {
            project.land.put(n, answer);
        }
    }

    /// Routes its line from `start` (a node laid, or the first station's
    /// end) to the second station's end for the least cost, then asks the
    /// game for the streets it crosses ([`RailStage::Streets`]) before it
    /// lays out its crossings and heights ([`Self::lay_out`]): false if no
    /// route was found. Before its stations are built, it lays out the line
    /// between their first placements, and builds them only once it has.
    fn plan_rail(&mut self, start: Node) -> bool {
        let Some(project) = self.rail.as_mut() else {
            return false;
        };
        let Some((goal, out)) = project.end(1) else {
            return false;
        };
        // Into the second station, against the heading out of its end.
        let arrive = out + std::f64::consts::PI;
        let Some(cells) = rail::route(
            &project.land,
            &project.prices,
            start.at,
            start.heading,
            goal,
            arrive,
        ) else {
            return false;
        };
        project.aligned = rail::align(&project.land, start.at, start.heading, goal, arrive, &cells);
        project.runs = rail::street_runs(&project.land, &cells);
        project.streets.clear();
        if project.runs.is_empty() {
            return self.lay_out();
        }
        self.stage = Stage::Rail(RailStage::Streets);
        true
    }

    /// Lays out its route's level crossings and heights for the least cost
    /// and cuts the track into pieces, then asks the verdict on the first:
    /// false if there is no way. A street it cannot cross where the route
    /// meets it is gone round, routing again.
    fn lay_out(&mut self) -> bool {
        let year = self.year();
        let Some(project) = self.rail.as_mut() else {
            return false;
        };
        let (nodes, mut holds) = rail::cross(&project.aligned, &project.streets);
        // From a node laid on the way, not the station's end: its edge may
        // go on as a bridge or a tunnel.
        let station = project.end(0).map(|(end, _)| end);
        if let (Some(first), Some(hold)) = (nodes.first(), holds.first_mut())
            && station.is_none_or(|end| end.flat_distance(first.at) > 1.0)
        {
            *hold = rail::Hold::Laid(first.at.z);
        }
        let Some(nodes) = rail::profile(&project.land, &project.prices, &nodes, &holds) else {
            self.say("found no heights for its track within its grade".into());
            return false;
        };
        project.pieces = rail::pieces(&nodes, year, &project.prices);
        project.piece = 0;
        let crossings = nodes.iter().filter(|n| n.crossing.is_some()).count();
        let (bridges, tunnels) = (
            nodes
                .iter()
                .filter(|n| n.kind == rail::Kind::Bridge)
                .count(),
            nodes
                .iter()
                .filter(|n| n.kind == rail::Kind::Tunnel)
                .count(),
        );
        // Passing loops on a long line, for more trains.
        project.looped = rail::loop_sites(&project.land, &nodes);
        let said = format!(
            "lays its track in {} pieces for about {:.0}: {crossings} level crossings, {bridges} bridge and {tunnels} tunnel edges{}",
            project.pieces.len(),
            rail::cost(&nodes),
            match project.looped.len() {
                0 => String::new(),
                1 => ", a passing loop".to_owned(),
                n => format!(", {n} passing loops"),
            }
        );
        project.nodes = nodes;
        if project.pieces.is_empty() {
            return false;
        }
        // Laid out before its stations: every piece is put to the game's
        // verdict first, then the stations are built, for the placements
        // it was laid out for.
        let next = if project.built[1].is_some() {
            RailStage::PieceVerdict
        } else {
            project.planned = project
                .tries
                .iter()
                .map(|t| t.first().map(|s| s.origin))
                .collect::<Option<Vec<P>>>()
                .and_then(|v| v.try_into().ok());
            RailStage::Trial { piece: 0 }
        };
        self.say(said);
        self.stage = Stage::Rail(next);
        true
    }

    /// The streets the game named around one of its route's street runs.
    pub(super) fn rail_streets(&mut self, streets: Vec<AiStreet>, now: Instant) {
        let Some(project) = self.rail.as_mut() else {
            return;
        };
        if !matches!(self.stage, Stage::Rail(RailStage::Streets)) {
            return;
        }
        project.streets.extend(streets);
        if !project.runs.is_empty() {
            project.runs.remove(0);
        }
        if project.runs.is_empty() {
            let laid = self.lay_out();
            self.or_other_sites(laid, "found no way to lay its track", now);
        }
    }

    /// Before its stations are built, a line it could not lay out between
    /// its first placements is laid out between others, the town's end and
    /// the industry's in turn, up to [`SITE_PAIRS`]; then given up, `why`.
    fn or_other_sites(&mut self, mut laid: bool, why: &str, now: Instant) {
        while !laid {
            let Some(project) = self.rail.as_mut() else {
                return;
            };
            let end = usize::from(project.pairs_tried % 2 == 1);
            if project.built[0].is_some()
                || project.pairs_tried >= SITE_PAIRS
                || project.tries[end].len() < 2
            {
                self.give_up_rail(why, now);
                return;
            }
            project.pairs_tried += 1;
            project.tries[end].remove(0);
            project.replans = 0;
            let Some((at, heading)) = project.end(0) else {
                return;
            };
            self.say("lays its line out between other station places".into());
            laid = self.plan_rail(Node::on(at, heading));
        }
    }

    pub(super) fn rail_good_verdict(
        &mut self,
        stage: RailStage,
        action: &Action,
        cost: i64,
        now: Instant,
    ) -> Option<Out> {
        let next = match (stage, action) {
            (RailStage::StationVerdict { end }, Action::BuildConstruction(_)) => {
                RailStage::StationBuilt { end, tries: 0 }
            }
            (RailStage::PieceVerdict, Action::BuildTrack(_)) => {
                self.read_prices(cost);
                RailStage::PieceBuilt { tries: 0 }
            }
            (RailStage::DepotVerdict { end }, Action::BuildConstruction(build)) => {
                let project = self.rail.as_mut()?;
                let built = project.built[end]?;
                let other = P::of(if end == 0 {
                    project.to.pos()
                } else {
                    project.from.pos
                });
                // Where it was tried: its join and the heading into it.
                let (_, (join, into, _)) =
                    depot_by(&built.station, other, project.depot_far, 0, 0, "")?;
                project.depot = Some((build.transform.origin, join));
                project.depot_into = into;
                RailStage::DepotBuilt { end, tries: 0 }
            }
            (RailStage::ShareVerdict, Action::BuildTrack(_)) => RailStage::ShareBuilt { tries: 0 },
            (RailStage::LinkVerdict { .. }, Action::BuildTrack(_)) => {
                RailStage::LinkBuilt { tries: 0 }
            }
            // Its station's street sent: the other station's, or done.
            (RailStage::FrontVerdict { end, .. }, Action::BuildRoad(_)) => {
                let out = self.act(action, now);
                if out.is_some() {
                    let described = self
                        .rail
                        .as_ref()
                        .map(RailProject::describe)
                        .unwrap_or_default();
                    self.say(format!(
                        "joins its station to the town's streets for {described}"
                    ));
                    self.next_front(end, now);
                } else {
                    self.wake = Some(now + SETTLE);
                }
                return out;
            }
            (RailStage::LoopVerdict { lp }, Action::BuildTrack(_)) => RailStage::LoopBuilt { lp },
            (RailStage::SignalVerdict { lp, n, .. }, Action::PlaceStop(_)) => {
                RailStage::SignalBuilt { lp, n }
            }
            // A trial: nothing sent; the next piece, then its depot, then
            // the stations.
            (RailStage::Trial { piece }, Action::BuildTrack(_)) => {
                let count = self.rail.as_ref()?.pieces.len();
                self.stage = Stage::Rail(if piece + 1 < count {
                    RailStage::Trial { piece: piece + 1 }
                } else {
                    RailStage::DepotTrial { end: 0, place: 0 }
                });
                return None;
            }
            (RailStage::DepotTrial { end, place }, Action::BuildConstruction(_)) => {
                self.stage = Stage::Rail(RailStage::LinkTrial { end, place });
                return None;
            }
            (RailStage::LinkTrial { end, place }, Action::BuildTrack(_)) => {
                self.rail.as_mut()?.depot_plan = Some((end, place));
                self.stage = Stage::Rail(RailStage::StationVerdict { end: 0 });
                return None;
            }
            _ => {
                self.say("the game judged another action".into());
                self.wake = Some(now + SETTLE);
                return None;
            }
        };
        let out = self.act(action, now);
        if out.is_some() {
            self.stage = Stage::Rail(next);
        } else {
            self.wake = Some(now + SETTLE);
        }
        out
    }

    /// The game's price for the piece it planned: its prices drawn halfway
    /// to the game's, for its next plans and lines.
    fn read_prices(&mut self, cost: i64) {
        let Some(project) = self.rail.as_mut() else {
            return;
        };
        let Some(piece) = project.pieces.get(project.piece) else {
            return;
        };
        let planned = piece.cost / project.prices.scale;
        if planned < 1_000.0 {
            return;
        }
        #[allow(clippy::cast_precision_loss)]
        let seen = cost as f64 / planned;
        let scale = (0.5 * project.prices.scale + 0.5 * seen).clamp(SCALE_RANGE.0, SCALE_RANGE.1);
        project.prices.scale = scale;
        self.rail_scale = scale;
    }

    pub(super) fn rail_bad_verdict(&mut self, stage: RailStage, why: &str, now: Instant) {
        match stage {
            RailStage::StationVerdict { end } => {
                self.say(format!("a rail station placement was refused: {why}"));
                if let Some(project) = self.rail.as_mut()
                    && !project.tries[end].is_empty()
                {
                    project.tries[end].remove(0);
                }
            }
            RailStage::LoopVerdict { lp } => {
                self.say(format!("its passing loop was refused: {why}"));
                let next = self
                    .rail
                    .as_ref()
                    .map_or(RailStage::Opening, |p| next_loop(p, lp));
                self.stage = Stage::Rail(next);
            }
            // Its loop's track not yet in every game, or refused.
            RailStage::SignalVerdict { lp, n, tries } if tries < CONFIRM_TRIES => {
                self.stage = Stage::Rail(RailStage::SignalVerdict {
                    lp,
                    n,
                    tries: tries + 1,
                });
                self.wake = Some(now + SETTLE);
            }
            RailStage::SignalVerdict { lp, .. } => {
                self.say(format!("its passing loop's signals were refused: {why}"));
                let next = self
                    .rail
                    .as_ref()
                    .map_or(RailStage::Opening, |p| next_loop(p, lp));
                self.stage = Stage::Rail(next);
            }
            // The next place, then the other station, then other sites.
            RailStage::DepotTrial { end, place } | RailStage::LinkTrial { end, place } => {
                if matches!(stage, RailStage::LinkTrial { .. }) {
                    self.say(format!("a rail depot's track would be refused: {why}"));
                } else {
                    self.say(format!("a rail depot placement would be refused: {why}"));
                }
                if place + 1 < rail::DEPOT_TRIES {
                    self.stage = Stage::Rail(RailStage::DepotTrial {
                        end,
                        place: place + 1,
                    });
                } else if end == 0 {
                    self.stage = Stage::Rail(RailStage::DepotTrial { end: 1, place: 0 });
                } else {
                    self.or_other_sites(false, "found no place for a rail depot", now);
                }
            }
            RailStage::Trial { piece } => {
                let laid = self.rail.as_ref().and_then(|p| {
                    let piece = p.pieces.get(piece)?;
                    Some(rail::describe(p.nodes.get(piece.first..=piece.last)?))
                });
                self.say(format!(
                    "a piece of its track would be refused: {why} ({})",
                    laid.unwrap_or_default()
                ));
                let Some(project) = self.rail.as_mut() else {
                    return;
                };
                // Round it, before anything is built; then other places.
                project.replans += 1;
                if let Some(p) = project.pieces.get(piece)
                    && let Some(mid) = project.nodes.get(usize::midpoint(p.first, p.last))
                {
                    project.land.block(mid.at.x, mid.at.y, REFUSED_REACH);
                }
                let start = project.end(0);
                let replanned = project.replans <= REPLANS
                    && start.is_some_and(|(at, heading)| self.plan_rail(Node::on(at, heading)));
                self.or_other_sites(replanned, "found its track would be refused", now);
            }
            RailStage::PieceVerdict => {
                let laid = self.rail.as_ref().and_then(|p| {
                    let piece = p.pieces.get(p.piece)?;
                    Some(rail::describe(p.nodes.get(piece.first..=piece.last)?))
                });
                self.say(format!(
                    "a piece of track was refused: {why} ({})",
                    laid.unwrap_or_default()
                ));
                // A piece whose start the last one should have laid: that
                // one did not go; lay it again.
                if why.contains("no Track node at vertex 1")
                    && let Some(project) = self.rail.as_mut()
                    && project.piece > 0
                {
                    project.piece -= 1;
                    project.replans += 1;
                    if project.replans > REPLANS {
                        self.give_up_rail("its track would not join", now);
                    }
                    return;
                }
                let Some(project) = self.rail.as_mut() else {
                    return;
                };
                project.replans += 1;
                if project.replans > REPLANS {
                    self.give_up_rail("its track was refused too often", now);
                    return;
                }
                // Round the refused piece's middle, from where it began.
                let Some(piece) = project.pieces.get(project.piece) else {
                    return;
                };
                let (first, last) = (piece.first, piece.last);
                let start = project.nodes.get(first).copied();
                let mid = project.nodes.get(usize::midpoint(first, last)).copied();
                if let (Some(start), Some(mid)) = (start, mid) {
                    project.land.block(mid.at.x, mid.at.y, REFUSED_REACH);
                    // From a node laid, on the ground: what is built of
                    // the line before it stays.
                    if !self.plan_rail(Node::on(start.at, start.heading)) {
                        self.give_up_rail("found no route round a refused piece", now);
                    }
                }
            }
            // Further out at this end, then at the other: a line given up
            // here has its stations and track built already.
            RailStage::DepotVerdict { end } => {
                self.say(format!("a rail depot placement was refused: {why}"));
                let further = self.rail.as_mut().is_some_and(|p| {
                    p.depot_far += 1;
                    if p.depot_far < rail::DEPOT_TRIES {
                        return true;
                    }
                    p.depot_far = 0;
                    false
                });
                if further {
                    self.stage = Stage::Rail(RailStage::DepotVerdict { end });
                } else if end == 0 {
                    self.stage = Stage::Rail(RailStage::DepotVerdict { end: 1 });
                } else {
                    self.give_up_rail("found no place for a rail depot", now);
                }
            }
            RailStage::ShareVerdict => {
                self.say(format!("could not share a rail depot: {why}"));
                let next = self
                    .rail
                    .as_mut()
                    .map_or(RailStage::DepotVerdict { end: 0 }, |p| {
                        p.share = None;
                        first_depot(p)
                    });
                self.stage = Stage::Rail(next);
            }
            // Its depot stands but cannot be joined: another place, as a
            // depot refused, rather than the line, built already.
            // The next place along its front, then the next place to join,
            // else this station goes without.
            RailStage::FrontVerdict { end, at, join } => {
                self.say(format!("its station's street would be refused: {why}"));
                let joins = self.rail.as_ref().map_or(0, |p| p.front_joins.len());
                if at + 1 < rail::FRONT_X.len() {
                    self.stage = Stage::Rail(RailStage::FrontVerdict {
                        end,
                        at: at + 1,
                        join,
                    });
                } else if join + 1 < joins {
                    self.stage = Stage::Rail(RailStage::FrontVerdict {
                        end,
                        at: 0,
                        join: join + 1,
                    });
                } else {
                    self.say("found no street to join its station to".into());
                    self.next_front(end, now);
                }
            }
            RailStage::LinkVerdict { end } => {
                self.say(format!("its depot's track was refused: {why}"));
                let further = self.rail.as_mut().is_some_and(|p| {
                    p.depot = None;
                    p.depot_far += 1;
                    if p.depot_far < rail::DEPOT_TRIES {
                        return true;
                    }
                    p.depot_far = 0;
                    false
                });
                if further {
                    self.stage = Stage::Rail(RailStage::DepotVerdict { end });
                } else if end == 0 {
                    self.stage = Stage::Rail(RailStage::DepotVerdict { end: 1 });
                } else {
                    self.give_up_rail("found no place for a joined rail depot", now);
                }
            }
            _ => {}
        }
    }

    pub(super) fn rail_company(
        &mut self,
        stage: RailStage,
        company: &AiCompany,
        now: Instant,
    ) -> Option<Out> {
        match stage {
            RailStage::StationBuilt { end, tries } => {
                let project = self.rail.as_mut()?;
                let station = *project.tries[end].first()?;
                let found = company
                    .stations
                    .iter()
                    .find(|s| P::of(s.pos).flat_distance(station.origin) <= 5.0);
                match found {
                    Some(s) => {
                        let other = P::of(if end == 0 {
                            project.to.pos()
                        } else {
                            project.from.pos
                        });
                        let ((end_at, heading), _) = station.ends_toward(other);
                        // The game's own end, where it reads within 3 m.
                        let end_at = s
                            .ends
                            .iter()
                            .map(|e| P::of(e.pos))
                            .find(|e| e.flat_distance(end_at) <= 3.0)
                            .unwrap_or(end_at);
                        project.built[end] = Some(Built {
                            station,
                            id: s.id,
                            end: end_at,
                            heading,
                        });
                        if end == 0 {
                            self.stage = Stage::Rail(RailStage::StationVerdict { end: 1 });
                        } else {
                            let (a, b) = (project.built[0]?, project.built[1]?);
                            // Built where its track was laid out for, their
                            // ends where the track meets them: laid as
                            // planned; else laid out again.
                            let meets = |n: Option<&Node>, end: P| {
                                n.is_some_and(|n| {
                                    n.at.flat_distance(end) <= 1.0 && (n.at.z - end.z).abs() <= 0.25
                                })
                            };
                            let as_planned = project.planned
                                == Some([a.station.origin, b.station.origin])
                                && meets(project.nodes.first(), a.end)
                                && meets(project.nodes.last(), b.end);
                            if as_planned {
                                self.stage = Stage::Rail(RailStage::PieceVerdict);
                            } else if !self.plan_rail(Node::on(a.end, a.heading)) {
                                self.give_up_rail("found no route for its track", now);
                            }
                        }
                    }
                    None if tries >= CONFIRM_TRIES => {
                        if !project.tries[end].is_empty() {
                            project.tries[end].remove(0);
                        }
                        self.stage = Stage::Rail(RailStage::StationVerdict { end });
                    }
                    None => self.retry(
                        Stage::Rail(RailStage::StationBuilt {
                            end,
                            tries: tries + 1,
                        }),
                        tries,
                        now,
                    ),
                }
                None
            }
            RailStage::PieceBuilt { tries } => {
                let project = self.rail.as_mut()?;
                let last = project.piece + 1 == project.pieces.len();
                // The last piece meets the second station's end: seen when
                // that end is open no more. Another is seen by the next
                // piece's verdict, which starts on its last node.
                let joined = |built: Option<Built>| {
                    built.is_some_and(|b| {
                        company.stations.iter().any(|s| {
                            s.id == b.id
                                && s.ends
                                    .iter()
                                    .any(|e| P::of(e.pos).flat_distance(b.end) <= 3.0 && !e.open)
                        })
                    })
                };
                if !last || joined(project.built[1]) {
                    project.piece += 1;
                    if project.piece >= project.pieces.len() {
                        self.say("laid its track".into());
                        let next = self.depot_or_share(company);
                        self.stage = Stage::Rail(next);
                    } else {
                        self.stage = Stage::Rail(RailStage::PieceVerdict);
                    }
                } else if tries >= CONFIRM_TRIES {
                    self.stage = Stage::Rail(RailStage::PieceVerdict);
                } else {
                    self.retry(
                        Stage::Rail(RailStage::PieceBuilt { tries: tries + 1 }),
                        tries,
                        now,
                    );
                }
                None
            }
            RailStage::DepotBuilt { end, tries } => {
                let project = self.rail.as_mut()?;
                let (origin, _) = project.depot?;
                let found = company
                    .depots
                    .iter()
                    .find(|d| P::of(d.at).flat_distance(P::of(origin)) <= 2.0)
                    .cloned();
                match found {
                    Some(depot) if depot.connected => {
                        self.stage = Stage::Rail(after_depot(project))
                    }
                    Some(_) => self.stage = Stage::Rail(RailStage::LinkVerdict { end }),
                    None if tries >= CONFIRM_TRIES => {
                        project.depot = None;
                        if end == 0 {
                            self.stage = Stage::Rail(RailStage::DepotVerdict { end: 1 });
                        } else {
                            self.give_up_rail("its rail depot did not come", now);
                        }
                    }
                    None => self.retry(
                        Stage::Rail(RailStage::DepotBuilt {
                            end,
                            tries: tries + 1,
                        }),
                        tries,
                        now,
                    ),
                }
                None
            }
            // Its loop laid: its signals next; both standing, the line.
            RailStage::LoopBuilt { lp } => {
                self.stage = Stage::Rail(RailStage::SignalVerdict { lp, n: 0, tries: 0 });
                None
            }
            RailStage::SignalBuilt { lp, n: 0 } => {
                self.stage = Stage::Rail(RailStage::SignalVerdict { lp, n: 1, tries: 0 });
                None
            }
            RailStage::SignalBuilt { lp, .. } => {
                let project = self.rail.as_mut()?;
                project.signalled += 1;
                // Where: the line's nodes either side of the loop's far end,
                // for the log.
                let (k, _) = *project.looped.get(lp)?;
                let m = k + rail::LOOP_EDGES;
                let at = |i: usize| {
                    project
                        .nodes
                        .get(i)
                        .map_or(String::new(), |n| format!(" {:.0},{:.0}", n.at.x, n.at.y))
                };
                let said = format!(
                    "signals its passing loop by{}{}{}",
                    at(m - 1),
                    at(m),
                    at(m + 1)
                );
                let next = next_loop(project, lp);
                self.say(said);
                self.stage = Stage::Rail(next);
                None
            }
            // Its link to a depot it shares laid: seen when its first
            // station's free end is taken.
            RailStage::ShareBuilt { tries } => {
                let project = self.rail.as_mut()?;
                let built = project.built[0]?;
                let (_, (end, _)) = built.station.ends_toward(P::of(project.to.pos()));
                let taken = company.stations.iter().any(|s| {
                    s.id == built.id
                        && s.ends
                            .iter()
                            .any(|e| P::of(e.pos).flat_distance(end) <= 3.0 && !e.open)
                });
                let share = project.share.clone()?;
                if taken {
                    project.depot = Some((share.at, P::of(share.entrance?)));
                    let next = after_depot(project);
                    self.say("shares a rail depot with another of its lines".into());
                    self.stage = Stage::Rail(next);
                } else if tries >= CONFIRM_TRIES {
                    project.share = None;
                    self.stage = Stage::Rail(first_depot(project));
                } else {
                    self.retry(
                        Stage::Rail(RailStage::ShareBuilt { tries: tries + 1 }),
                        tries,
                        now,
                    );
                }
                None
            }
            RailStage::LinkBuilt { tries } => {
                let (origin, _) = self.rail.as_ref()?.depot?;
                let connected = company
                    .depots
                    .iter()
                    .any(|d| P::of(d.at).flat_distance(P::of(origin)) <= 2.0 && d.connected);
                if connected {
                    let next = after_depot(self.rail.as_ref()?);
                    self.stage = Stage::Rail(next);
                } else if tries >= CONFIRM_TRIES {
                    self.give_up_rail("its depot's track did not join", now);
                } else {
                    self.retry(
                        Stage::Rail(RailStage::LinkBuilt { tries: tries + 1 }),
                        tries,
                        now,
                    );
                }
                None
            }
            RailStage::LineMade { tries } => {
                let project = self.rail.as_mut()?;
                let stops = [project.built[0]?.id, project.built[1]?.id];
                // The line it opened: of those through its stations, one it
                // does not know yet.
                let known = |id: LineId| self.memo.rail_lines.contains(&id.0);
                if let Some(line) = company
                    .lines
                    .iter()
                    .find(|l| !known(l.id) && l.stops.iter().copied().eq(stops.iter().copied()))
                {
                    project.line = Some(line.id);
                    let key = project.key;
                    let entry = (line.id.0, project.cargo.as_str().to_owned());
                    let described = project.describe();
                    if !self.memo.served.contains(&key) {
                        self.memo.served.push(key);
                    }
                    if !self.memo.cargo_lines.contains(&entry) {
                        self.memo.cargo_lines.push(entry);
                    }
                    if project.signalled > 0
                        && !self.memo.looped.iter().any(|(l, _)| *l == line.id.0)
                    {
                        self.memo.looped.push((line.id.0, project.signalled));
                    }
                    if !self.memo.rail_lines.contains(&line.id.0) {
                        self.memo.rail_lines.push(line.id.0);
                    }
                    self.chain_line(line.id.0);
                    self.say(format!("opened {described}"));
                    self.stage = Stage::Rail(RailStage::Buying);
                } else if tries >= CONFIRM_TRIES {
                    self.give_up_rail("its line did not open", now);
                } else {
                    self.retry(
                        Stage::Rail(RailStage::LineMade { tries: tries + 1 }),
                        tries,
                        now,
                    );
                }
                None
            }
            RailStage::Bought { before, tries } => {
                let new = company
                    .vehicles
                    .iter()
                    .find(|v| v.line.is_none() && !before.contains(&v.id))
                    .map(|v| v.id);
                match new {
                    Some(vehicle) => {
                        let line = self.rail.as_ref()?.line?;
                        let action = Action::AssignLine(AssignLine {
                            vehicles: BoundedVec::new(vec![vehicle]).ok()?,
                            line: Some(line),
                            first_stop: None,
                        });
                        self.stage = Stage::Rail(RailStage::Assigned { vehicle, tries: 0 });
                        let out = self.act(&action, now);
                        if out.is_none() {
                            self.stage = Stage::Rail(RailStage::Bought { before, tries });
                            self.wake = Some(now + SETTLE);
                        }
                        out
                    }
                    None if tries >= CONFIRM_TRIES => {
                        self.give_up_rail("its train did not come", now);
                        None
                    }
                    None => {
                        self.retry(
                            Stage::Rail(RailStage::Bought {
                                before,
                                tries: tries + 1,
                            }),
                            tries,
                            now,
                        );
                        None
                    }
                }
            }
            RailStage::Assigned { vehicle, tries } => {
                let line = self.rail.as_ref()?.line?;
                if company
                    .vehicles
                    .iter()
                    .any(|v| v.id == vehicle && v.line == Some(line))
                {
                    let described = self
                        .rail
                        .as_ref()
                        .map(RailProject::describe)
                        .unwrap_or_default();
                    self.say(format!("runs a train on {described}"));
                    // A passenger line's stations joined to the towns'
                    // streets, that people walk to them.
                    if self.rail.as_ref().is_some_and(|p| p.passengers) {
                        self.stage = Stage::Rail(RailStage::FrontStreets { end: 0 });
                        return None;
                    }
                    self.rail = None;
                    self.leg_now = None;
                    self.rest(now);
                } else if tries >= CONFIRM_TRIES {
                    self.give_up_rail("its train did not take the line", now);
                } else {
                    self.retry(
                        Stage::Rail(RailStage::Assigned {
                            vehicle,
                            tries: tries + 1,
                        }),
                        tries,
                        now,
                    );
                }
                None
            }
            _ => None,
        }
    }

    /// The streets read by station `end`'s front: the nearest places to join
    /// them at, a street's end or its middle, those its own street reaches
    /// first along its front, then the verdict on its own street to the
    /// nearest; none such, the next station. The game counts its catchment
    /// along the streets from its entrance, its own street's length taken
    /// from what the walk reaches ([`rail::WALK`]).
    pub(super) fn rail_front_streets(&mut self, streets: Vec<AiStreet>, end: usize, now: Instant) {
        let Some(project) = self.rail.as_mut() else {
            return;
        };
        let Some(built) = project.built[end] else {
            self.next_front(end, now);
            return;
        };
        let station = built.station;
        let (a, b) = station.front(rail::FRONT_X[0], &project.land);
        let off = |j: &place::Join| {
            let p = P::of(j.at);
            a.flat_distance(p).min(b.flat_distance(p))
        };
        // On its building's side, beyond its front: its street need cross
        // neither the station nor its track to reach it.
        let mut near: Vec<place::Join> = streets
            .iter()
            .flat_map(place::joins)
            .filter(|j| station.across(P::of(j.at)).0 < rail::FRONT_X[0])
            .collect();
        near.sort_by(|x, y| off(x).total_cmp(&off(y)));
        // A node two streets meet at, once.
        near.dedup_by(|x, y| x.at == y.at);
        near.truncate(rail::FRONT_JOINS);
        match near.is_empty() {
            false => {
                project.front_joins = near;
                self.stage = Stage::Rail(RailStage::FrontVerdict {
                    end,
                    at: 0,
                    join: 0,
                });
            }
            true => {
                self.say("found no street to join its station to".into());
                self.next_front(end, now);
            }
        }
    }

    /// On from station `end`'s street: the other's, or the line is done.
    fn next_front(&mut self, end: usize, now: Instant) {
        if end == 0 {
            self.stage = Stage::Rail(RailStage::FrontStreets { end: 1 });
        } else {
            self.rail = None;
            self.leg_now = None;
            self.rest(now);
        }
    }

    /// Buys the train it chose: its engine and as many of its wagons as the
    /// stations' platforms take, at most [`WAGONS_MOST`].
    fn buy_train(&mut self, now: Instant) -> Option<Out> {
        let project = self.rail.as_ref()?;
        let (engine, wagon) = project.train.clone()?;
        let built = project.built[0]?;
        let platform =
            f64::from(built.station.last - built.station.first + 1) * rail::MODULE * 10.0;
        let room = (platform - f64::from(engine.length)).max(0.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let fit = (room / f64::from(wagon.length.max(1))).floor() as usize;
        let wagons = fit.clamp(1, WAGONS_MOST);
        let (origin, _) = project.depot?;
        let color = tint(&self.seat);
        let part = |m: &AiModel| ConsistPart {
            model: m.model.clone(),
            reversed: false,
            loads: BoundedVec::default(),
            color,
        };
        let mut consist = vec![part(&engine)];
        consist.extend(std::iter::repeat_n(part(&wagon), wagons));
        let n = consist.len();
        let action = Action::BuyVehicle(BuyVehicle {
            depot: ConstructionRef {
                file: Text::lossy(rail::DEPOT),
                at: origin,
            },
            consist: BoundedVec::new(consist).ok()?,
            groups: BoundedVec::new(vec![1; n]).ok()?,
            multiple_units: BoundedVec::new(vec![Text::lossy(""); n]).ok()?,
            depot_index: 0,
        });
        let before = self
            .company
            .as_ref()
            .map(|c| c.vehicles.iter().map(|v| v.id).collect())
            .unwrap_or_default();
        let bought = format!(
            "buys a train: {} and {wagons} {}",
            engine.model.as_str(),
            wagon.model.as_str()
        );
        self.stage = Stage::Rail(RailStage::Bought { before, tries: 0 });
        let out = self.act(&action, now);
        // Said once it is sent: until its hands may act again, it is not.
        if out.is_some() {
            self.say(bought);
        } else {
            self.stage = Stage::Rail(RailStage::Buying);
            self.wake = Some(now + SETTLE);
        }
        out
    }

    /// Opens the line: the first station loads the cargo, the second takes
    /// it off; passengers loaded where they may be, as the game's line
    /// window loads them; a rail stop's waits.
    fn open_rail(&mut self, now: Instant) -> Option<Out> {
        let project = self.rail.as_ref()?;
        // Passengers, the game's cargo 0, both ways; a cargo at its maker.
        let passengers = project.passengers;
        let index = if passengers {
            0
        } else {
            usize::from(
                project
                    .from
                    .outputs
                    .iter()
                    .find(|o| o.cargo == project.cargo)?
                    .index,
            )
        };
        let full = Fraction(1_000_000);
        let stop = |group: StationId, loads: bool| -> Option<LineStop> {
            let mut load = vec![false; index + 1];
            load[0] = true;
            load[index] = loads || passengers;
            Some(LineStop {
                group,
                terminal: Terminal {
                    station: 0,
                    terminal: 0,
                },
                alternatives: BoundedVec::default(),
                load_mode: LoadMode::LoadIfAvailable,
                min_wait: 0,
                max_wait: RAIL_MAX_WAIT,
                max_extra_wait: RAIL_EXTRA_WAIT,
                rules: StopRules {
                    load: BoundedVec::new(load).ok()?,
                    max_load: BoundedVec::new(vec![full; index + 1]).ok()?,
                    force_unload: false,
                    destroy_for_config_change: true,
                    destroy_for_refresh: true,
                },
                waypoints: BoundedVec::default(),
            })
        };
        let stops = vec![
            stop(project.built[0]?.id, true)?,
            stop(project.built[1]?.id, false)?,
        ];
        let name = if passengers {
            format!("{} {} train", project.from.name.as_str(), project.to.name())
        } else {
            format!(
                "{} {} train",
                project.from.name.as_str(),
                cargo_name(project.cargo.as_str())
            )
        };
        let action = Action::CreateLine(CreateLine {
            name: Text::lossy(&name),
            color: tint(&self.seat),
            line: LineData {
                stops: BoundedVec::new(stops).ok()?,
                modes: BoundedVec::new(vec![TRAIN_MODE]).ok()?,
                custom_filters: false,
                reservation_priority: 0,
            },
        });
        self.stage = Stage::Rail(RailStage::LineMade { tries: 0 });
        let out = self.act(&action, now);
        if out.is_none() {
            self.stage = Stage::Rail(RailStage::Opening);
            self.wake = Some(now + SETTLE);
        }
        out
    }

    /// The models the game offers: first its cargo's wagons, then engines.
    /// A train is the strongest engine it can afford with as many of the
    /// roomiest wagons as its stations' platforms take, at most
    /// [`WAGONS_MOST`].
    pub(super) fn rail_models(
        &mut self,
        stage: RailStage,
        models: Vec<AiModel>,
        now: Instant,
    ) -> Option<Out> {
        match stage {
            RailStage::Wagons => {
                let wagons: Vec<AiModel> = models
                    .into_iter()
                    .filter(|m| m.capacity > 0 && m.power == 0)
                    .collect();
                if wagons.is_empty() {
                    self.give_up_rail("found no wagon for its cargo", now);
                    return None;
                }
                self.rail.as_mut()?.wagons = wagons;
                self.stage = Stage::Rail(RailStage::Land);
                None
            }
            RailStage::Engines => {
                let project = self.rail.as_ref()?;
                let wagon = project
                    .wagons
                    .iter()
                    .max_by(|a, b| {
                        let per = |m: &AiModel| f64::from(m.capacity) / f64::from(m.length.max(1));
                        per(a).total_cmp(&per(b))
                    })?
                    .clone();
                // What the line will cost before its train: its money must
                // leave the train.
                #[allow(clippy::cast_possible_truncation)]
                let money = self.spendable() - project.estimate.ceil() as i64;
                // Not electric: its track has no catenary (a C 6/8 bought
                // for one, the game refused it the line; 2026-10-30).
                let Some(engine) = models
                    .into_iter()
                    .filter(|m| m.power > 0 && !m.electric)
                    .filter(|m| m.price + 3 * wagon.price <= money)
                    .max_by_key(|m| (m.power, -m.price))
                else {
                    self.give_up_rail("found no engine it can afford", now);
                    return None;
                };
                // Trestles for a slow train while they are offered: cheaper
                // than stone, and their 60 km/h costs it nothing.
                let trestle =
                    self.year() < costs::TRESTLE_UNTIL && engine.speed <= costs::TRESTLE_SPEED;
                let project = self.rail.as_mut()?;
                project.prices.trestle = trestle;
                project.train = Some((engine, wagon));
                // Its whole line laid out before it builds anything.
                let (at, heading) = project.end(0)?;
                let laid = self.plan_rail(Node::on(at, heading));
                self.or_other_sites(laid, "found no route for its track", now);
                None
            }
            _ => None,
        }
    }
}

/// The most placements of each station a line is priced between before it
/// builds anything.
const PRICED_SITES: usize = 3;

/// What a line costs before its train: its two stations and depot, and the
/// cheapest track between their ends with a level crossing for every run
/// of streets it crosses, for the cheapest pair among the first
/// placements of each station ([`PRICED_SITES`]), which it then tries
/// first; or why there is no line.
fn estimate(project: &mut RailProject) -> Result<f64, &'static str> {
    let mut why = "found no place for a rail station";
    let mut best: Option<(f64, usize, usize)> = None;
    // Its searches run in the driver's agent: two lines priced, or three
    // routes not found, are enough.
    let (mut priced, mut failed) = (0, 0);
    for (i, a) in project.tries[0].iter().take(PRICED_SITES).enumerate() {
        for (j, b) in project.tries[1].iter().take(PRICED_SITES).enumerate() {
            if priced >= 2 || failed >= 3 {
                break;
            }
            // Each end towards the place the other station serves, as the
            // stations' ends are read once built.
            let ((from, leaving), _) = a.ends_toward(P::of(project.to.pos()));
            let ((to, out), _) = b.ends_toward(P::of(project.from.pos));
            let arrive = out + std::f64::consts::PI;
            let prices = &project.prices;
            let Some(cells) = rail::route(&project.land, prices, from, leaving, to, arrive) else {
                why = "found no route for its track";
                failed += 1;
                continue;
            };
            let Some(nodes) = rail::track(&project.land, prices, from, leaving, to, arrive, &cells)
            else {
                why = "found no heights for its track within its grade";
                continue;
            };
            #[allow(clippy::cast_precision_loss)]
            let crossings = rail::street_runs(&project.land, &cells).len() as f64;
            let cost = rail::cost(&nodes)
                + crossings * prices.crossing()
                + prices.stations_and_depot(STATION_MODULES);
            priced += 1;
            if best.is_none_or(|(c, _, _)| cost < c) {
                best = Some((cost, i, j));
            }
        }
    }
    let (cost, i, j) = best.ok_or(why)?;
    let a = project.tries[0].remove(i);
    project.tries[0].insert(0, a);
    let b = project.tries[1].remove(j);
    project.tries[1].insert(0, b);
    Ok(cost)
}

/// A piece as its verdict is asked before anything of the line is built:
/// its joins to the stations and to the pieces before it as new nodes.
fn trial(build: &TrackBuild) -> Option<TrackBuild> {
    let vertices: Vec<Vertex> = build
        .polyline
        .vertices
        .iter()
        .map(|v| Vertex {
            pos: v.pos,
            resolve: match &v.resolve {
                Resolve::Node(_) => Resolve::New,
                r => r.clone(),
            },
        })
        .collect();
    let mut out = build.clone();
    out.polyline.vertices = BoundedVec::new(vertices).ok()?;
    Some(out)
}

/// What follows its depot's joining: its passing loop, if it has one, else
/// its line.
fn after_depot(project: &RailProject) -> RailStage {
    if project.looped.is_empty() {
        RailStage::Opening
    } else {
        RailStage::LoopVerdict { lp: 0 }
    }
}

/// Where its depot is tried first once its stations stand: where its
/// trial passed, else straight out beyond its first station.
/// A depot by a station: the station's end its track joins and the heading
/// out of it; where its track joins, the heading into it, and the build.
type DepotBy = ((P, f64), (P, f64, tpf3mp_proto::action::ConstructionBuild));

/// Station `station`'s depot at place `place` ([`rail::depot_spot`]), its
/// line leaving towards `other`.
fn depot_by(
    station: &rail::Station,
    other: P,
    place: usize,
    year: i32,
    seed: i64,
    name: &str,
) -> Option<DepotBy> {
    let ((end, out), spot) = rail::depot_spot(station, other, place)?;
    Some((
        (end, out),
        rail::depot_turned(end, out, spot, year, seed, name)?,
    ))
}

fn first_depot(project: &mut RailProject) -> RailStage {
    let (end, place) = project.depot_plan.unwrap_or((0, 0));
    project.depot_far = place;
    RailStage::DepotVerdict { end }
}

/// What follows its passing loop number `lp`: its next loop, else its line.
fn next_loop(project: &RailProject, lp: usize) -> RailStage {
    if lp + 1 < project.looped.len() {
        RailStage::LoopVerdict { lp: lp + 1 }
    } else {
        RailStage::Opening
    }
}

impl Brain {
    /// Where its line's trains come from once its track is laid: a rail
    /// depot of its own whose join is within [`rail::DEPOT_SHARE`] of its
    /// first station's free end, shared; else one of its own to build.
    fn depot_or_share(&mut self, company: &AiCompany) -> RailStage {
        let Some(project) = self.rail.as_mut() else {
            return RailStage::DepotVerdict { end: 0 };
        };
        let Some(built) = project.built[0] else {
            return RailStage::DepotVerdict { end: 0 };
        };
        let (_, (end, _)) = built.station.ends_toward(P::of(project.to.pos()));
        project.share = company
            .depots
            .iter()
            .filter(|d| d.file.as_str() == rail::DEPOT && d.connected)
            .filter_map(|d| d.entrance.map(|j| (P::of(j).flat_distance(end), d)))
            .filter(|(far, _)| (30.0..=rail::DEPOT_SHARE).contains(far))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, d)| d.clone());
        if project.share.is_some() {
            RailStage::ShareVerdict
        } else {
            first_depot(project)
        }
    }
}

/// The track from a line's first station's free end to the join of the
/// depot it shares, arriving the way the depot's own track runs into it.
fn share_link(project: &RailProject, year: i32) -> Option<TrackBuild> {
    let built = project.built[0]?;
    let share = project.share.as_ref()?;
    let (_, (end, out)) = built.station.ends_toward(P::of(project.to.pos()));
    let join = P::of(share.entrance?);
    let origin = P::of(share.at);
    let into = (origin.y - join.y).atan2(origin.x - join.x);
    rail::depot_link(end, out, P { z: join.z, ..join }, into, year)
}

//! The brain's rail lines (docs/AI_PLAYERS.md, "Track and trains"): a
//! cargo carried by train from the industry that makes it to a town that
//! needs it, where the two are too far apart for trucks to pay. It reads
//! the land around both ends, puts a one-track cargo station by each, lays
//! the track between them a piece at a time, each put to the game's
//! verdict and routed round where the game refuses one, puts a rail depot
//! beyond the first station joined to its track, opens the line and buys a
//! train onto it. Every step is confirmed by reading the world again.

use std::time::Instant;

use tpf3mp_bridge::ai::{AiCompany, AiIndustry, AiLand, AiModel, AiQuestion};
use tpf3mp_proto::{
    BoundedVec, Text,
    action::{
        Action, AssignLine, BuyVehicle, ConsistPart, ConstructionRef, CreateLine, Fraction,
        LineData, LineId, LineStop, LoadMode, Pos, ResName, StationId, StopRules, Terminal,
        TrackBuild, VehicleId,
    },
};

use super::{Brain, CONFIRM_TRIES, GIVE_UP_FOR, Out, SETTLE, Stage, cargo_name, route_key, tint};
use crate::{
    chain::{Leg, Taker},
    place::P,
    rail::{self, Land, Node, Station},
};

/// The least distance a cargo goes by train, in metres, and the least
/// money a rail line is begun with: two stations, a depot and a train cost
/// about 4 million in the 1920s, the track about 500 a metre.
pub const RAIL_FROM: f64 = 1_800.0;
pub const RAIL_MONEY: i64 = 6_000_000;
/// What a line's stations, depot and track cost about, before its train.
const RAIL_BUILD: i64 = 2_500_000;
/// The rings around an industry, and a town's district centre, its
/// stations are tried on, in metres from the place: the station's cargo
/// catchment reaches 150 m from its building (`main_building_1_cargo`).
const MAKER_RINGS: [f64; 3] = [150.0, 200.0, 250.0];
const TOWN_RINGS: [f64; 4] = [120.0, 180.0, 260.0, 340.0];
/// How many times a line is routed round pieces the game refused.
pub const REPLANS: u32 = 8;
/// Around a refused piece's middle the land is taken as built on, in
/// metres.
pub const REFUSED_REACH: f64 = 70.0;
/// The game's numbers for trains (`TransportMode.TRAIN`), as lines list
/// modes, and the mode's name.
pub const TRAIN_MODE: u16 = 7;
pub const TRAIN: &str = "TRAIN";
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
    pub nodes: Vec<Node>,
    pub pieces: Vec<TrackBuild>,
    pub piece: usize,
    pub replans: u32,
    /// The depot and where its track joins, once sent and seen.
    pub depot: Option<(Pos, P)>,
    pub line: Option<LineId>,
    pub wagons: Vec<AiModel>,
    /// The train it chose before it built anything: engine and wagon.
    pub train: Option<(AiModel, AiModel)>,
}

impl RailProject {
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
    /// Asking the verdict on the next piece of track.
    PieceVerdict,
    /// A piece sent: reading before the next.
    PieceBuilt {
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
}

impl Brain {
    /// Begins a rail line carrying a cargo from the industry that makes it
    /// to its taker, if it can afford one: true if it did.
    pub(super) fn start_rail(&mut self, leg: Leg) -> bool {
        if self.spendable() < RAIL_MONEY {
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
            nodes: Vec::new(),
            pieces: Vec::new(),
            piece: 0,
            replans: 0,
            depot: None,
            line: None,
            wagons: Vec::new(),
            train: None,
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
            let key = route_key(project.from.pos, project.cargo.as_str());
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
                if !self.memo.given_up.contains(&key) {
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
                        let project = self.rail.as_mut()?;
                        let (from, at) = (P::of(project.from.pos), P::of(project.to.pos()));
                        // By a town's district, or by the industry it feeds.
                        let rings: &[f64] = match project.to {
                            Taker::Town { .. } => &TOWN_RINGS,
                            Taker::Industry(_) => &MAKER_RINGS,
                        };
                        project.tries = [
                            rail::sites(&project.land, from, at, &MAKER_RINGS),
                            rail::sites(&project.land, at, from, rings),
                        ];
                        self.stage = Stage::Rail(RailStage::StationVerdict { end: 0 });
                        self.rail_poll(RailStage::StationVerdict { end: 0 }, now)
                    }
                }
            }
            RailStage::StationVerdict { end } => {
                let year = self.year();
                #[allow(clippy::cast_possible_wrap)]
                let seed = (self.random() >> 33) as i64;
                let project = self.rail.as_ref()?;
                let name = if end == 0 {
                    format!("{} Rail", project.from.name.as_str())
                } else {
                    format!("{} Goods", project.to.name())
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
            RailStage::PieceVerdict => {
                let project = self.rail.as_ref()?;
                let piece = project.pieces.get(project.piece)?.clone();
                self.verdict(&Action::BuildTrack(piece), now)
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
                let (_, (depot_end, out)) = built.station.ends_toward(other);
                let name = format!("{} Rail Depot", project.from.name.as_str());
                match rail::depot(depot_end, out, year, seed, &name) {
                    Some((_, build)) => self.verdict(&Action::BuildConstruction(build), now),
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
                let (_, (depot_end, _)) = built.station.ends_toward(other);
                let (_, join) = project.depot?;
                match rail::link(depot_end, join, year) {
                    Some(link) => self.verdict(&Action::BuildTrack(link), now),
                    None => {
                        self.give_up_rail("could not lay out a depot's track", now);
                        None
                    }
                }
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
    /// end) to the second station's end, and cuts it into pieces: false if
    /// no route was found.
    fn plan_rail(&mut self, start: Node) -> bool {
        let year = self.year();
        let Some(project) = self.rail.as_mut() else {
            return false;
        };
        let Some(b) = project.built[1] else {
            return false;
        };
        // Into the second station, against the heading out of its end.
        let arrive = b.heading + std::f64::consts::PI;
        let Some(cells) = rail::route(&project.land, start.at, start.heading, b.end, arrive) else {
            return false;
        };
        let nodes = rail::track(
            &project.land,
            start.at,
            start.heading,
            b.end,
            arrive,
            &cells,
        );
        project.pieces = rail::pieces(&nodes, year);
        project.nodes = nodes;
        project.piece = 0;
        !project.pieces.is_empty()
    }

    pub(super) fn rail_good_verdict(
        &mut self,
        stage: RailStage,
        action: &Action,
        now: Instant,
    ) -> Option<Out> {
        let next = match (stage, action) {
            (RailStage::StationVerdict { end }, Action::BuildConstruction(_)) => {
                RailStage::StationBuilt { end, tries: 0 }
            }
            (RailStage::PieceVerdict, Action::BuildTrack(_)) => RailStage::PieceBuilt { tries: 0 },
            (RailStage::DepotVerdict { end }, Action::BuildConstruction(build)) => {
                let project = self.rail.as_mut()?;
                let built = project.built[end]?;
                let other = P::of(if end == 0 {
                    project.to.pos()
                } else {
                    project.from.pos
                });
                let (_, (depot_end, out)) = built.station.ends_toward(other);
                let (ox, oy) = (out.cos(), out.sin());
                let join = P {
                    x: depot_end.x + ox * rail::DEPOT_GAP,
                    y: depot_end.y + oy * rail::DEPOT_GAP,
                    z: depot_end.z,
                };
                project.depot = Some((build.transform.origin, join));
                RailStage::DepotBuilt { end, tries: 0 }
            }
            (RailStage::LinkVerdict { .. }, Action::BuildTrack(_)) => {
                RailStage::LinkBuilt { tries: 0 }
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
            RailStage::PieceVerdict => {
                self.say(format!("a piece of track was refused: {why}"));
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
                let first = project.piece * rail::PIECE_EDGES;
                let start = project.nodes.get(first).copied();
                let mid = project
                    .nodes
                    .get((first + rail::PIECE_EDGES / 2).min(project.nodes.len().saturating_sub(1)))
                    .copied();
                if let (Some(start), Some(mid)) = (start, mid) {
                    project.land.block(mid.at.x, mid.at.y, REFUSED_REACH);
                    if !self.plan_rail(start) {
                        self.give_up_rail("found no route round a refused piece", now);
                    }
                }
            }
            RailStage::DepotVerdict { end } => {
                self.say(format!("a rail depot placement was refused: {why}"));
                if end == 0 {
                    self.stage = Stage::Rail(RailStage::DepotVerdict { end: 1 });
                } else {
                    self.give_up_rail("found no place for a rail depot", now);
                }
            }
            RailStage::LinkVerdict { .. } => {
                self.give_up_rail(&format!("its depot's track was refused: {why}"), now);
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
                            let a = project.built[0]?;
                            let start = Node {
                                at: a.end,
                                heading: a.heading,
                                bridge: false,
                            };
                            if self.plan_rail(start) {
                                let n = self.rail.as_ref().map_or(0, |p| p.pieces.len());
                                self.say(format!("lays its track in {n} pieces"));
                                self.stage = Stage::Rail(RailStage::PieceVerdict);
                            } else {
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
                        self.stage = Stage::Rail(RailStage::DepotVerdict { end: 0 });
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
                    Some(depot) if depot.connected => self.stage = Stage::Rail(RailStage::Opening),
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
            RailStage::LinkBuilt { tries } => {
                let (origin, _) = self.rail.as_ref()?.depot?;
                let connected = company
                    .depots
                    .iter()
                    .any(|d| P::of(d.at).flat_distance(P::of(origin)) <= 2.0 && d.connected);
                if connected {
                    self.stage = Stage::Rail(RailStage::Opening);
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
                if let Some(line) = company
                    .lines
                    .iter()
                    .find(|l| l.stops.iter().copied().eq(stops.iter().copied()))
                {
                    project.line = Some(line.id);
                    let key = route_key(project.from.pos, project.cargo.as_str());
                    let entry = (line.id.0, project.cargo.as_str().to_owned());
                    let described = project.describe();
                    if !self.memo.served.contains(&key) {
                        self.memo.served.push(key);
                    }
                    if !self.memo.cargo_lines.contains(&entry) {
                        self.memo.cargo_lines.push(entry);
                    }
                    if !self.memo.rail_lines.contains(&line.id.0) {
                        self.memo.rail_lines.push(line.id.0);
                    }
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
        self.say(format!(
            "buys a train: {} and {wagons} {}",
            engine.model.as_str(),
            wagon.model.as_str()
        ));
        self.stage = Stage::Rail(RailStage::Bought { before, tries: 0 });
        let out = self.act(&action, now);
        if out.is_none() {
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
        let index = usize::from(
            project
                .from
                .outputs
                .iter()
                .find(|o| o.cargo == project.cargo)?
                .index,
        );
        let full = Fraction(1_000_000);
        let stop = |group: StationId, loads: bool| -> Option<LineStop> {
            let mut load = vec![false; index + 1];
            load[0] = true;
            load[index] = loads;
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
        let name = format!(
            "{} {} train",
            project.from.name.as_str(),
            cargo_name(project.cargo.as_str())
        );
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
                self.stage = Stage::Rail(RailStage::Engines);
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
                let money = self.spendable() - RAIL_BUILD;
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
                self.rail.as_mut()?.train = Some((engine, wagon));
                self.stage = Stage::Rail(RailStage::Land);
                None
            }
            _ => None,
        }
    }
}

//! The brain's care of its lines (docs/AI_PLAYERS.md, "Operations"): at
//! each look at its company, after its new lines have their first vehicles,
//! it replaces a road vehicle at the end of its life with the best model of
//! the year (the vehicle window's replace: the same vehicle, on its line),
//! adds one to a line running full with passengers or cargo waiting, and
//! sells one from a settled line that runs near empty or loses money, never
//! a line's last. A rail line with signalled passing loops whose trains
//! leave cargo waiting gets another train, up to one more than its loops.

use std::time::Instant;

use tpf3mp_bridge::ai::{AiCompany, AiDepot, AiLine, AiModel, AiQuestion};
use tpf3mp_proto::{
    BoundedVec, Text,
    action::{
        Action, AssignLine, BuyVehicle, ConsistPart, ConstructionRef, EditLine, LineChange, LineId,
        ReplaceVehicle, ReplacedPart, ResName, VehicleId,
    },
};

use super::{BUS, Brain, CONFIRM_TRIES, Out, SETTLE, Stage, TRUCK, road_depot, tint, train};
use crate::{place::P, rail};

/// How far through their lives every vehicle of a line must be before its
/// load and money are judged, in percent: about a year of a bus of 20.
pub const SETTLED: u16 = 5;
/// The cargo waiting at a cargo line's stops that makes it a line running
/// full: a trainload, several truckloads. A cargo line runs full one way
/// and empty back, so its load, over both, says little of how busy it is.
pub const CARGO_BACKLOG: u32 = 60;
/// How far through its life a line's oldest vehicle must be, in percent,
/// for a line that has earned nothing at all to be given up: about seven of
/// the game's months for a bus or a train's wagons of 15 years, five for a
/// truck of 10, time for a chain's later leg to bring what its first
/// carries on. The long run's passenger train, its stations at the towns'
/// edges reaching four and five people, earned nothing in eleven months and
/// cost a million; its meat train, from a farm no one supplied, the same
/// (2026-10-31).
pub const DEAD_AFTER: u16 = 4;

/// Where its care of a line stands.
#[derive(Debug, Clone)]
pub(super) enum Upkeep {
    /// Asking the models of a worn vehicle's kind, to replace it.
    Models {
        vehicle: VehicleId,
        cargo: Option<ResName>,
    },
    /// A replacement sent: reading until the vehicle is new.
    Replaced {
        vehicle: VehicleId,
        tries: u32,
    },
    /// A vehicle sold: reading until it is gone.
    Sold {
        vehicle: VehicleId,
        tries: u32,
    },
    /// A losing line's last vehicle sold: reading until it is gone, then
    /// the line deleted, and reading until it is gone too.
    Closing {
        line: LineId,
        vehicle: VehicleId,
        tries: u32,
    },
    Deleted {
        line: LineId,
        tries: u32,
    },
    /// A second train for a looped rail line: asking its cargo's wagons,
    /// then the engines, at its rail depot, its first train's engine.
    TrainWagons {
        line: LineId,
        cargo: ResName,
        depot: AiDepot,
        engine: ResName,
    },
    TrainEngines {
        line: LineId,
        depot: AiDepot,
        engine: ResName,
        wagon: AiModel,
    },
    /// The train bought, and sent onto its line.
    TrainBought {
        line: LineId,
        before: Vec<VehicleId>,
        tries: u32,
    },
    TrainAssigned {
        line: LineId,
        vehicle: VehicleId,
        tries: u32,
    },
}

/// What it does for a line at a look.
enum Task {
    Replace {
        vehicle: VehicleId,
        cargo: Option<ResName>,
        line: String,
    },
    Add {
        line: AiLine,
    },
    Sell {
        vehicle: VehicleId,
        line: String,
        why: &'static str,
    },
    Close {
        vehicle: VehicleId,
        line: LineId,
        name: String,
        /// It earned nothing at all, rather than less than it cost.
        dead: bool,
    },
    Train {
        line: AiLine,
        cargo: ResName,
        depot: AiDepot,
        engine: ResName,
    },
}

impl Brain {
    /// Its road lines: those not its rail lines.
    fn road_lines<'a>(&self, company: &'a AiCompany) -> Vec<&'a AiLine> {
        company
            .lines
            .iter()
            .filter(|l| !self.memo.rail_lines.contains(&l.id.0))
            .collect()
    }

    /// The cargo a line of its carries, if it carries one.
    fn cargo_of(&self, line: &AiLine) -> Option<ResName> {
        self.memo
            .cargo_lines
            .iter()
            .find(|(id, _)| *id == line.id.0)
            .map(|(_, cargo)| Text::lossy(cargo))
    }

    /// A line running full: a bus line its buses this full with more
    /// waiting; a cargo line with [`CARGO_BACKLOG`] waiting.
    pub(super) fn full(&self, line: &AiLine) -> bool {
        if self.cargo_of(line).is_some() {
            line.waiting >= CARGO_BACKLOG
        } else {
            line.load >= self.profile.add_above && line.waiting > 0
        }
    }

    /// A line running near empty: a bus line under its difficulty's load; a
    /// cargo line, empty half its way, under half that.
    fn empty(&self, line: &AiLine) -> bool {
        let under = if self.cargo_of(line).is_some() {
            self.profile.remove_below / 2
        } else {
            self.profile.remove_below
        };
        line.load > 0 && line.load < under
    }

    /// What its lines need now, the most pressing first: a worn vehicle
    /// replaced, then a full line given one more, then an empty one one
    /// fewer.
    fn task(&self, company: &AiCompany) -> Option<Task> {
        let lines = self.road_lines(company);
        let on = |line: &AiLine| {
            company
                .vehicles
                .iter()
                .filter(|v| v.line == Some(line.id))
                .collect::<Vec<_>>()
        };
        // The most worn of its road vehicles past its life.
        let worn = lines
            .iter()
            .flat_map(|l| on(l).into_iter().map(move |v| (*l, v)))
            .filter(|(_, v)| v.worn >= self.profile.replace_at)
            .max_by_key(|(_, v)| v.worn);
        if let Some((line, v)) = worn {
            return Some(Task::Replace {
                vehicle: v.id,
                cargo: self.cargo_of(line),
                line: line.name.as_str().to_owned(),
            });
        }
        // Running full, with more waiting.
        let full = lines
            .iter()
            .find(|l| self.full(l) && on(l).len() < usize::from(self.profile.buses_most));
        if let Some(line) = full {
            return Some(Task::Add {
                line: (*line).clone(),
            });
        }
        // A looped rail line whose trains run full, cargo waiting: another,
        // at its rail depot by its first station, up to one more than its
        // signalled loops (n loops pass n + 1 trains on single track).
        for (line, loops) in company.lines.iter().filter_map(|l| {
            self.memo
                .looped
                .iter()
                .find(|(id, _)| *id == l.id.0)
                .map(|(_, n)| (l, usize::from(*n)))
        }) {
            let trains = on(line);
            let Some(first) = trains.first() else {
                continue;
            };
            if trains.len() > loops {
                continue;
            }
            if !self.full(line) {
                continue;
            }
            let Some(cargo) = self.cargo_of(line) else {
                continue;
            };
            let Some(station) = line
                .stops
                .first()
                .and_then(|s| company.stations.iter().find(|st| st.id == *s))
            else {
                continue;
            };
            let Some(depot) = company
                .depots
                .iter()
                .filter(|d| d.file.as_str() == rail::DEPOT && d.connected)
                .min_by(|a, b| {
                    let d = |x: &AiDepot| P::of(x.at).flat_distance(P::of(station.pos));
                    d(a).total_cmp(&d(b))
                })
            else {
                continue;
            };
            return Some(Task::Train {
                line: line.clone(),
                cargo,
                depot: depot.clone(),
                engine: first.model.clone(),
            });
        }
        // Carrying no one: run for a while, earned nothing at all, and no
        // one aboard or waiting for it. Its vehicles sold, then the line
        // closed, without waiting for it to settle: nothing it might earn
        // later is worth what it costs. A train's first paying trip may take
        // months, its loads waited for at each end; one with riders aboard
        // or waiting is on its way (run 45 closed a passenger train, its
        // stations fed by buses, before its first trip had paid).
        for line in company.lines.iter() {
            let vehicles = on(line);
            let Some(oldest) = vehicles.iter().max_by_key(|v| v.worn) else {
                continue;
            };
            if oldest.worn < DEAD_AFTER
                || line.income.is_none_or(|i| i > 0)
                || line.load > 0
                || line.waiting > 0
            {
                continue;
            }
            if vehicles.len() >= 2 {
                return Some(Task::Sell {
                    vehicle: oldest.id,
                    line: line.name.as_str().to_owned(),
                    why: "carrying no one",
                });
            }
            return Some(Task::Close {
                vehicle: oldest.id,
                line: line.id,
                name: line.name.as_str().to_owned(),
                dead: true,
            });
        }
        // Settled, near empty or losing money: its most worn sold; a
        // line down to its last that lost money over its year closed, as a
        // player gives up a line that does not pay: its rail lines too, a
        // train running at a loss the dearest of all (the long run's lost
        // 0.6 to 1 million a year each).
        for line in company.lines.iter() {
            let vehicles = on(line);
            if vehicles.is_empty() || vehicles.iter().any(|v| v.worn < SETTLED) {
                continue;
            }
            let losing = line.profit < 0;
            let why = if self.empty(line) {
                "running near empty"
            } else if losing {
                "losing money"
            } else {
                continue;
            };
            let v = vehicles.iter().max_by_key(|v| v.worn)?;
            if vehicles.len() >= 2 {
                return Some(Task::Sell {
                    vehicle: v.id,
                    line: line.name.as_str().to_owned(),
                    why,
                });
            }
            if losing {
                return Some(Task::Close {
                    vehicle: v.id,
                    line: line.id,
                    name: line.name.as_str().to_owned(),
                    dead: false,
                });
            }
        }
        None
    }

    /// Care of its lines at a look, if any is due: Some if it took a task,
    /// with its first want.
    pub(super) fn upkeep(&mut self, company: &AiCompany, now: Instant) -> Option<Option<Out>> {
        match self.task(company)? {
            Task::Replace {
                vehicle,
                cargo,
                line,
            } => {
                self.say(format!("replaces a worn-out vehicle on {line}"));
                self.stage = Stage::Upkeep(Upkeep::Models { vehicle, cargo });
                Some(None)
            }
            Task::Add { line } => {
                let depot = road_depot(company, &line)?;
                self.say(format!(
                    "adds a vehicle to {}, running full",
                    line.name.as_str()
                ));
                self.topping = Some((line.id, depot));
                self.stage = Stage::Models;
                Some(None)
            }
            Task::Train {
                line,
                cargo,
                depot,
                engine,
            } => {
                self.say(format!(
                    "buys another train for {}, running full",
                    line.name.as_str()
                ));
                self.stage = Stage::Upkeep(Upkeep::TrainWagons {
                    line: line.id,
                    cargo,
                    depot,
                    engine,
                });
                Some(None)
            }
            Task::Sell { vehicle, line, why } => {
                self.say(format!("sells a vehicle from {line}, {why}"));
                let action = Action::SellVehicle {
                    vehicles: BoundedVec::new(vec![vehicle]).ok()?,
                };
                self.stage = Stage::Upkeep(Upkeep::Sold { vehicle, tries: 0 });
                let out = self.act(&action, now);
                if out.is_none() {
                    self.rest(now);
                }
                Some(out)
            }
            Task::Close {
                vehicle,
                line,
                name,
                dead,
            } => {
                let lost = company
                    .lines
                    .iter()
                    .find(|l| l.id == line)
                    .map_or(0, |l| -l.profit);
                if dead {
                    self.say(format!(
                        "closes {name}, which earned nothing and cost {lost}"
                    ));
                } else {
                    self.say(format!("closes {name}, which lost {lost} over its year"));
                }
                let action = Action::SellVehicle {
                    vehicles: BoundedVec::new(vec![vehicle]).ok()?,
                };
                self.stage = Stage::Upkeep(Upkeep::Closing {
                    line,
                    vehicle,
                    tries: 0,
                });
                let out = self.act(&action, now);
                if out.is_none() {
                    self.rest(now);
                }
                Some(out)
            }
        }
    }

    pub(super) fn upkeep_poll(&mut self, stage: Upkeep, now: Instant) -> Option<Out> {
        match stage {
            Upkeep::Models { cargo, .. } => {
                let mode = if cargo.is_some() { TRUCK } else { BUS };
                self.ask(
                    AiQuestion::Models {
                        mode: Text::lossy(mode),
                        cargo,
                    },
                    now,
                )
            }
            Upkeep::TrainWagons { cargo, .. } => self.ask(
                AiQuestion::Models {
                    mode: Text::lossy(train::TRAIN),
                    cargo: Some(cargo),
                },
                now,
            ),
            Upkeep::TrainEngines { .. } => self.ask(
                AiQuestion::Models {
                    mode: Text::lossy(train::TRAIN),
                    cargo: None,
                },
                now,
            ),
            Upkeep::Replaced { .. }
            | Upkeep::Sold { .. }
            | Upkeep::Closing { .. }
            | Upkeep::Deleted { .. }
            | Upkeep::TrainBought { .. }
            | Upkeep::TrainAssigned { .. } => self.ask(AiQuestion::Company, now),
        }
    }

    /// The models of a worn vehicle's kind: the best of the year in its
    /// place, its line and orders kept.
    pub(super) fn upkeep_models(
        &mut self,
        stage: Upkeep,
        models: Vec<AiModel>,
        now: Instant,
    ) -> Option<Out> {
        let vehicle = match stage {
            Upkeep::Models { vehicle, .. } => vehicle,
            Upkeep::TrainWagons {
                line,
                depot,
                engine,
                ..
            } => {
                // The roomiest wagon a metre for its cargo.
                let Some(wagon) = models
                    .into_iter()
                    .filter(|m| m.capacity > 0 && m.power == 0)
                    .max_by(|a, b| {
                        let per = |m: &AiModel| f64::from(m.capacity) / f64::from(m.length.max(1));
                        per(a).total_cmp(&per(b))
                    })
                else {
                    self.rest(now);
                    return None;
                };
                self.stage = Stage::Upkeep(Upkeep::TrainEngines {
                    line,
                    depot,
                    engine,
                    wagon,
                });
                return None;
            }
            Upkeep::TrainEngines {
                line,
                depot,
                engine,
                wagon,
            } => return self.buy_second_train(line, &depot, &engine, &wagon, models, now),
            _ => return None,
        };
        // For the riders its line has: a full line's seats, else a new
        // town line's few.
        let seats = self.company.as_ref().and_then(|c| {
            let line = c.vehicles.iter().find(|v| v.id == vehicle)?.line?;
            let line = c.lines.iter().find(|l| l.id == line)?;
            (self.cargo_of(line).is_none() && !self.full(line)).then_some(super::TOWN_RIDERS)
        });
        let Some(model) = self.pick_model(models, seats) else {
            self.say("found no vehicle to replace it with".into());
            self.rest(now);
            return None;
        };
        if model.price > self.spendable() {
            self.say("waits for money to replace a vehicle".into());
            self.rest(now);
            return None;
        }
        let action = Action::ReplaceVehicle(ReplaceVehicle {
            vehicle,
            consist: BoundedVec::new(vec![ReplacedPart {
                part: ConsistPart {
                    model: model.model.clone(),
                    reversed: false,
                    loads: BoundedVec::default(),
                    color: tint(&self.seat),
                },
                kept: None,
            }])
            .ok()?,
            groups: BoundedVec::new(vec![1]).ok()?,
            multiple_units: BoundedVec::new(vec![Text::lossy("")]).ok()?,
        });
        self.stage = Stage::Upkeep(Upkeep::Replaced { vehicle, tries: 0 });
        let out = self.act(&action, now);
        if out.is_none() {
            self.rest(now);
        }
        out
    }

    pub(super) fn upkeep_company(
        &mut self,
        stage: Upkeep,
        company: &AiCompany,
        now: Instant,
    ) -> Option<Out> {
        let (done, next) = match stage {
            Upkeep::Replaced { vehicle, tries } => (
                company
                    .vehicles
                    .iter()
                    .any(|v| v.id == vehicle && v.worn < SETTLED),
                Upkeep::Replaced {
                    vehicle,
                    tries: tries + 1,
                },
            ),
            Upkeep::Sold { vehicle, tries } => (
                company.vehicles.iter().all(|v| v.id != vehicle),
                Upkeep::Sold {
                    vehicle,
                    tries: tries + 1,
                },
            ),
            // Its last vehicle gone: the line deleted.
            Upkeep::Closing {
                line,
                vehicle,
                tries,
            } if company.vehicles.iter().all(|v| v.id != vehicle) => {
                let action = Action::EditLine(EditLine {
                    line,
                    change: LineChange::Delete,
                });
                self.stage = Stage::Upkeep(Upkeep::Deleted { line, tries: 0 });
                let out = self.act(&action, now);
                if out.is_none() {
                    self.stage = Stage::Upkeep(Upkeep::Closing {
                        line,
                        vehicle,
                        tries,
                    });
                    self.wake = Some(now + SETTLE);
                }
                return out;
            }
            Upkeep::Closing {
                line,
                vehicle,
                tries,
            } => (
                false,
                Upkeep::Closing {
                    line,
                    vehicle,
                    tries: tries + 1,
                },
            ),
            Upkeep::Deleted { line, tries } => (
                company.lines.iter().all(|l| l.id != line),
                Upkeep::Deleted {
                    line,
                    tries: tries + 1,
                },
            ),
            Upkeep::TrainBought {
                line,
                before,
                tries,
            } => {
                let new = company
                    .vehicles
                    .iter()
                    .find(|v| v.line.is_none() && !before.contains(&v.id))
                    .map(|v| v.id);
                if let Some(vehicle) = new {
                    let action = Action::AssignLine(AssignLine {
                        vehicles: BoundedVec::new(vec![vehicle]).ok()?,
                        line: Some(line),
                        first_stop: None,
                    });
                    self.stage = Stage::Upkeep(Upkeep::TrainAssigned {
                        line,
                        vehicle,
                        tries: 0,
                    });
                    let out = self.act(&action, now);
                    if out.is_none() {
                        self.stage = Stage::Upkeep(Upkeep::TrainBought {
                            line,
                            before,
                            tries,
                        });
                        self.wake = Some(now + SETTLE);
                    }
                    return out;
                }
                (
                    false,
                    Upkeep::TrainBought {
                        line,
                        before,
                        tries: tries + 1,
                    },
                )
            }
            Upkeep::TrainAssigned {
                line,
                vehicle,
                tries,
            } => {
                let runs = company
                    .vehicles
                    .iter()
                    .any(|v| v.id == vehicle && v.line == Some(line));
                if runs {
                    self.say("runs another train through its passing loops".into());
                }
                (
                    runs,
                    Upkeep::TrainAssigned {
                        line,
                        vehicle,
                        tries: tries + 1,
                    },
                )
            }
            Upkeep::Models { .. } | Upkeep::TrainWagons { .. } | Upkeep::TrainEngines { .. } => {
                return None;
            }
        };
        let tries = match &next {
            Upkeep::Replaced { tries, .. }
            | Upkeep::Sold { tries, .. }
            | Upkeep::Closing { tries, .. }
            | Upkeep::Deleted { tries, .. }
            | Upkeep::TrainBought { tries, .. }
            | Upkeep::TrainAssigned { tries, .. } => *tries,
            Upkeep::Models { .. } | Upkeep::TrainWagons { .. } | Upkeep::TrainEngines { .. } => 0,
        };
        if done || tries > CONFIRM_TRIES {
            if !done {
                self.say("its vehicle's change did not come".into());
            }
            self.rest(now);
        } else {
            self.retry(Stage::Upkeep(next), tries, now);
        }
        None
    }
}

impl Brain {
    /// Its second train for a looped line: its first train's engine where
    /// the year still offers it, else the strongest it can afford that runs
    /// without catenary, with as many of `wagon` as the platforms take.
    fn buy_second_train(
        &mut self,
        line: LineId,
        depot: &AiDepot,
        engine: &ResName,
        wagon: &AiModel,
        models: Vec<AiModel>,
        now: Instant,
    ) -> Option<Out> {
        let money = self.spendable();
        let engines: Vec<AiModel> = models
            .into_iter()
            .filter(|m| m.power > 0 && !m.electric)
            .filter(|m| m.price + 3 * wagon.price <= money)
            .collect();
        let pick = engines
            .iter()
            .find(|m| &m.model == engine)
            .or_else(|| engines.iter().max_by_key(|m| (m.power, -m.price)))
            .cloned();
        let Some(engine) = pick else {
            self.say("waits for money for a second train".into());
            self.rest(now);
            return None;
        };
        let platform = 3.0 * rail::MODULE * 10.0;
        let room = (platform - f64::from(engine.length)).max(0.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let wagons =
            ((room / f64::from(wagon.length.max(1))).floor() as usize).clamp(1, train::WAGONS_MOST);
        let color = tint(&self.seat);
        let part = |m: &AiModel| ConsistPart {
            model: m.model.clone(),
            reversed: false,
            loads: BoundedVec::default(),
            color,
        };
        let mut consist = vec![part(&engine)];
        consist.extend(std::iter::repeat_n(part(wagon), wagons));
        let n = consist.len();
        let action = Action::BuyVehicle(BuyVehicle {
            depot: ConstructionRef {
                file: depot.file.clone(),
                at: depot.at,
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
        self.stage = Stage::Upkeep(Upkeep::TrainBought {
            line,
            before,
            tries: 0,
        });
        let out = self.act(&action, now);
        if out.is_none() {
            self.rest(now);
        }
        out
    }
}

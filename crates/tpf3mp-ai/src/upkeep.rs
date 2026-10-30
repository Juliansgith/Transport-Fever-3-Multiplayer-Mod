//! The brain's care of its lines (docs/AI_PLAYERS.md, "Operations"): at
//! each look at its company, after its new lines have their first vehicles,
//! it replaces a road vehicle at the end of its life with the best model of
//! the year (the vehicle window's replace: the same vehicle, on its line),
//! adds one to a line running full with passengers or cargo waiting, and
//! sells one from a settled line that runs near empty or loses money, never
//! a line's last. Trains wait for passing loops.

use std::time::Instant;

use tpf3mp_bridge::ai::{AiCompany, AiLine, AiModel, AiQuestion};
use tpf3mp_proto::{
    BoundedVec, Text,
    action::{Action, ConsistPart, ReplaceVehicle, ReplacedPart, ResName, VehicleId},
};

use super::{BUS, Brain, CONFIRM_TRIES, Out, Stage, TRUCK, tint};

/// How far through their lives every vehicle of a line must be before its
/// load and money are judged, in percent: about a year of a bus of 20.
pub const SETTLED: u16 = 5;

/// Where its care of a line stands.
#[derive(Debug, Clone)]
pub(super) enum Upkeep {
    /// Asking the models of a worn vehicle's kind, to replace it.
    Models {
        vehicle: VehicleId,
        cargo: Option<ResName>,
    },
    /// A replacement sent: reading until the vehicle is new.
    Replaced { vehicle: VehicleId, tries: u32 },
    /// A vehicle sold: reading until it is gone.
    Sold { vehicle: VehicleId, tries: u32 },
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
        let full = lines.iter().find(|l| {
            l.load >= self.profile.add_above
                && l.waiting > 0
                && on(l).len() < usize::from(self.profile.buses_most)
        });
        if let Some(line) = full {
            return Some(Task::Add {
                line: (*line).clone(),
            });
        }
        // Settled, near empty or losing money: its most worn sold.
        for line in lines {
            let vehicles = on(line);
            if vehicles.len() < 2 || vehicles.iter().any(|v| v.worn < SETTLED) {
                continue;
            }
            let why = if line.load > 0 && line.load < self.profile.remove_below {
                "running near empty"
            } else if line.income < line.costs {
                "losing money"
            } else {
                continue;
            };
            let v = vehicles.iter().max_by_key(|v| v.worn)?;
            return Some(Task::Sell {
                vehicle: v.id,
                line: line.name.as_str().to_owned(),
                why,
            });
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
                let depot = company.depots.iter().find(|d| d.connected).cloned()?;
                self.say(format!(
                    "adds a vehicle to {}, running full",
                    line.name.as_str()
                ));
                self.topping = Some((line.id, depot));
                self.stage = Stage::Models;
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
            Upkeep::Replaced { .. } | Upkeep::Sold { .. } => self.ask(AiQuestion::Company, now),
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
        let Upkeep::Models { vehicle, .. } = stage else {
            return None;
        };
        let Some(model) = self.pick_model(models) else {
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
            Upkeep::Models { .. } => return None,
        };
        let tries = match &next {
            Upkeep::Replaced { tries, .. } | Upkeep::Sold { tries, .. } => *tries,
            Upkeep::Models { .. } => 0,
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

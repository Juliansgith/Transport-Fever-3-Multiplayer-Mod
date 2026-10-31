//! Production chains (docs/AI_PLAYERS.md, "What it carries"): where a
//! town needs a cargo whose maker makes too little of it, the AI carries
//! the maker's cargo to the town and feeds the maker its inputs from the
//! industries nearest it that make them. An industry takes its inputs only
//! while what it makes is carried on (investigation, "Trucks"), so a
//! chain's first leg is the one into the town, then its feeders.

use tpf3mp_bridge::ai::{AiIndustry, AiTown};
use tpf3mp_proto::action::{Pos, ResName, StationId};

use crate::place::P;

/// Who takes a cargo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Taker {
    /// A town that needs it, at the centre of the district that takes it.
    Town { town: AiTown, at: Pos },
    /// An industry that makes something of it.
    Industry(AiIndustry),
}

impl Taker {
    pub fn name(&self) -> &str {
        match self {
            Self::Town { town, .. } => town.name.as_str(),
            Self::Industry(industry) => industry.name.as_str(),
        }
    }

    /// Where the cargo goes: the district's centre, or the industry.
    pub fn pos(&self) -> Pos {
        match self {
            Self::Town { at, .. } => *at,
            Self::Industry(industry) => industry.pos,
        }
    }

    /// An industry's own station, where trucks unload its inputs.
    pub fn station(&self) -> Option<StationId> {
        match self {
            Self::Town { .. } => None,
            Self::Industry(industry) => industry.station,
        }
    }
}

/// A cargo from the industry that makes it to its taker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leg {
    pub cargo: ResName,
    pub from: AiIndustry,
    pub to: Taker,
}

impl Leg {
    pub fn distance(&self) -> f64 {
        P::of(self.from.pos).flat_distance(P::of(self.to.pos()))
    }
}

/// The least distance between a feeder and the industry it feeds worth a
/// line, in metres.
pub const FEED_NEAREST: f64 = 400.0;

/// What a chain search weighs.
pub struct Limits {
    /// The farthest a leg goes, in metres.
    pub reach: f64,
    /// The least a maker makes a year for its cargo to count as made.
    pub least_made: u32,
    /// The least a town must want over what it is supplied.
    pub least_short: u32,
    /// The least distance worth a leg into a town, in metres.
    pub nearest: f64,
    /// How many levels of industries a chain feeds: 2, the town's maker
    /// and its feeders; 3, their feeders too.
    pub depth: usize,
}

/// The most legs a chain has.
pub const MOST_LEGS: usize = 8;

/// The legs that feed `maker` its inputs, each from the nearest industry
/// in reach of it that makes enough of it, or, `depth` levels allowing,
/// one that makes too little but can itself be fed; each leg after the one
/// out of the industry it feeds. False if an input has no such feeder, or
/// one `skip` leaves out. No industry is in a chain twice.
fn feed(
    industries: &[AiIndustry],
    maker: &AiIndustry,
    depth: usize,
    limits: &Limits,
    skip: &impl Fn(&AiIndustry, &ResName) -> bool,
    used: &mut Vec<Pos>,
    legs: &mut Vec<Leg>,
) -> bool {
    let made = |i: &AiIndustry, cargo: &ResName| {
        i.outputs
            .iter()
            .find(|o| &o.cargo == cargo)
            .map_or(0, |o| o.per_year)
    };
    maker.inputs.iter().all(|input| {
        let feeder = industries
            .iter()
            .filter(|i| i.station.is_some() && !i.water && !used.contains(&i.pos))
            .filter(|i| i.outputs.iter().any(|o| &o.cargo == input))
            // Making enough, or a maker itself to be fed, levels allowing.
            .filter(|i| made(i, input) >= limits.least_made || depth >= 2 && !i.inputs.is_empty())
            .map(|i| (P::of(i.pos).flat_distance(P::of(maker.pos)), i))
            .filter(|(d, _)| *d >= FEED_NEAREST && *d <= limits.reach)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, i)| i);
        let Some(feeder) = feeder else {
            return false;
        };
        if skip(feeder, input) || legs.len() >= MOST_LEGS {
            return false;
        }
        used.push(feeder.pos);
        legs.push(Leg {
            cargo: input.clone(),
            from: feeder.clone(),
            to: Taker::Industry(maker.clone()),
        });
        made(feeder, input) >= limits.least_made
            || feed(
                industries,
                feeder,
                depth.saturating_sub(1),
                limits,
                skip,
                used,
                legs,
            )
    })
}

/// Chains: a town's need whose nearest maker in reach makes too little of
/// it, that maker carried to the town, and its inputs fed to it ([`feed`]),
/// as many levels deep as `limits` allow. `skip` leaves out a leg (one
/// carried already, given up on), and so any chain that needs it. The best
/// first: the more the town lacks, the fewer kilometres of line.
pub fn chains(
    industries: &[AiIndustry],
    towns: &[AiTown],
    limits: &Limits,
    skip: impl Fn(&AiIndustry, &ResName) -> bool,
) -> Vec<(f64, Vec<Leg>)> {
    let usable = |i: &&AiIndustry| i.station.is_some() && !i.water;
    let mut out = Vec::new();
    for town in towns {
        for need in town.needs.iter() {
            let short = need.limit.saturating_sub(need.supply);
            if short < limits.least_short {
                continue;
            }
            let at = P::of(need.at);
            // Its maker: the nearest in reach.
            let Some(maker) = industries
                .iter()
                .filter(usable)
                .filter(|i| i.outputs.iter().any(|o| o.cargo == need.cargo))
                .map(|i| (P::of(i.pos).flat_distance(at), i))
                .filter(|(d, _)| *d >= limits.nearest && *d <= limits.reach)
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, i)| i)
            else {
                continue;
            };
            let made = maker
                .outputs
                .iter()
                .find(|o| o.cargo == need.cargo)
                .map_or(0, |o| o.per_year);
            // Made enough already: a single line, not a chain.
            if made >= limits.least_made || maker.inputs.is_empty() || skip(maker, &need.cargo) {
                continue;
            }
            let mut legs = vec![Leg {
                cargo: need.cargo.clone(),
                from: maker.clone(),
                to: Taker::Town {
                    town: town.clone(),
                    at: need.at,
                },
            }];
            let mut used = vec![maker.pos];
            let fed = feed(
                industries,
                maker,
                limits.depth.saturating_sub(1),
                limits,
                &skip,
                &mut used,
                &mut legs,
            );
            if fed {
                let km: f64 = legs.iter().map(Leg::distance).sum::<f64>() / 1000.0;
                out.push((f64::from(short) / (1.0 + km), legs));
            }
        }
    }
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpf3mp_bridge::ai::{AiNeed, AiOutput};
    use tpf3mp_proto::{BoundedVec, Text, action::TownId};

    fn pos(x: f64, y: f64) -> Pos {
        P { x, y, z: 0.0 }.pos()
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn industry(
        name: &str,
        x: f64,
        y: f64,
        inputs: &[&str],
        outputs: &[(&str, u32)],
    ) -> AiIndustry {
        AiIndustry {
            name: Text::new(name).unwrap(),
            pos: pos(x, y),
            station: Some(StationId(1 + (x.abs() + y.abs()) as u32)),
            terminals: 2,
            inputs: BoundedVec::new(inputs.iter().map(|c| Text::new(*c).unwrap()).collect())
                .unwrap(),
            outputs: BoundedVec::new(
                outputs
                    .iter()
                    .map(|(c, n)| AiOutput {
                        cargo: Text::new(*c).unwrap(),
                        index: 3,
                        per_year: *n,
                        most: 200,
                    })
                    .collect(),
            )
            .unwrap(),
            water: false,
        }
    }

    fn town(needs: &[(&str, u32, u32)]) -> AiTown {
        AiTown {
            id: TownId(1),
            name: Text::new("Ely").unwrap(),
            pos: pos(5_000.0, 0.0),
            buildings: 200,
            centres: BoundedVec::new(vec![pos(5_000.0, 0.0)]).unwrap(),
            rivals: 0,
            linked: BoundedVec::default(),
            needs: BoundedVec::new(
                needs
                    .iter()
                    .map(|(c, supply, limit)| AiNeed {
                        cargo: Text::new(*c).unwrap(),
                        supply: *supply,
                        limit: *limit,
                        at: pos(5_000.0, 0.0),
                    })
                    .collect(),
            )
            .unwrap(),
        }
    }

    const LIMITS: Limits = Limits {
        reach: 8_000.0,
        least_made: 30,
        least_short: 10,
        nearest: 800.0,
        depth: 2,
    };

    #[test]
    fn a_starved_maker_is_carried_to_the_town_then_fed() {
        // A food plant that makes nothing for want of grain, the town that
        // needs food, a farm that makes grain, a second farm farther off.
        let industries = vec![
            industry("Mill", 2_000.0, 0.0, &["grain"], &[("food", 0)]),
            industry("Farm", 1_000.0, 500.0, &[], &[("grain", 120)]),
            industry("Far Farm", -3_000.0, 0.0, &[], &[("grain", 300)]),
        ];
        let found = chains(&industries, &[town(&[("food", 0, 80)])], &LIMITS, |_, _| {
            false
        });
        assert_eq!(found.len(), 1);
        let legs = &found[0].1;
        assert_eq!(legs.len(), 2);
        // Into the town first, then the nearer farm into the mill.
        assert_eq!(legs[0].from.name.as_str(), "Mill");
        assert!(matches!(legs[0].to, Taker::Town { .. }));
        assert_eq!(legs[1].from.name.as_str(), "Farm");
        assert_eq!(legs[1].cargo.as_str(), "grain");
        assert_eq!(legs[1].to, Taker::Industry(industries[0].clone()));
    }

    #[test]
    fn no_chain_where_the_maker_makes_enough_or_a_feeder_is_missing() {
        let town = town(&[("food", 0, 80)]);
        // Making enough: a single line's business.
        let busy = vec![
            industry("Mill", 2_000.0, 0.0, &["grain"], &[("food", 90)]),
            industry("Farm", 1_000.0, 500.0, &[], &[("grain", 120)]),
        ];
        assert!(chains(&busy, std::slice::from_ref(&town), &LIMITS, |_, _| false).is_empty());
        // Two inputs, one with no maker in reach.
        let half = vec![
            industry("Mill", 2_000.0, 0.0, &["grain", "salt"], &[("food", 0)]),
            industry("Farm", 1_000.0, 500.0, &[], &[("grain", 120)]),
        ];
        assert!(chains(&half, std::slice::from_ref(&town), &LIMITS, |_, _| false).is_empty());
        // A leg carried already leaves its chain out.
        let starved = vec![
            industry("Mill", 2_000.0, 0.0, &["grain"], &[("food", 0)]),
            industry("Farm", 1_000.0, 500.0, &[], &[("grain", 120)]),
        ];
        let skip = |i: &AiIndustry, c: &ResName| i.name.as_str() == "Farm" && c.as_str() == "grain";
        assert!(chains(&starved, &[town], &LIMITS, skip).is_empty());
    }

    #[test]
    fn a_starved_feeder_is_fed_in_turn_where_the_chain_may_go_deeper() {
        // Ely needs goods; its goods factory makes none for want of planks;
        // the sawmill that makes planks makes none for want of logs; a
        // forest makes logs.
        let industries = vec![
            industry("Factory", 2_000.0, 0.0, &["planks"], &[("goods", 0)]),
            industry("Sawmill", 500.0, 1_000.0, &["logs"], &[("planks", 0)]),
            industry("Forest", -1_500.0, 1_000.0, &[], &[("logs", 150)]),
        ];
        let town = town(&[("goods", 0, 80)]);
        // Two levels: the sawmill cannot feed the factory.
        assert!(
            chains(&industries, std::slice::from_ref(&town), &LIMITS, |_, _| {
                false
            })
            .is_empty()
        );
        // Three: the factory into Ely, the sawmill into the factory, the
        // forest into the sawmill, in that order.
        let deeper = Limits { depth: 3, ..LIMITS };
        let found = chains(&industries, &[town], &deeper, |_, _| false);
        assert_eq!(found.len(), 1);
        let legs: Vec<(&str, &str)> = found[0]
            .1
            .iter()
            .map(|l| (l.from.name.as_str(), l.to.name()))
            .collect();
        assert_eq!(
            legs,
            [
                ("Factory", "Ely"),
                ("Sawmill", "Factory"),
                ("Forest", "Sawmill")
            ]
        );
    }
}

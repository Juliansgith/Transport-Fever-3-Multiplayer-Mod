//! What a line would earn, before it is built: the reckoning a player does
//! from the town window's destinations, an industry's output and a
//! vehicle's seats and speed (docs/AI_PLAYERS.md, "What a line earns").
//!
//! The game pays for what a vehicle unloads by how far it carried it, in a
//! straight line, and by nothing else (its `economy/cargo_income.script`,
//! build 40425): a unit of any cargo [`CARGO_FARE`] a metre, a passenger
//! [`PASSENGER_FARE`]. So a line earns its units a year times its length,
//! a vehicle what it can carry in a year of trips, and the nearest taker is
//! the worst one that still has a road. These are a player's rules of
//! thumb, not the game's rules: what its own lines earn corrects them
//! ([`Learned`]).

/// A year of the game, in seconds of its clock at 1x
/// (`api.util.getDefaultYearDuration`, 1,461,000 ms).
pub const YEAR: f64 = 1_461.0;
/// What a unit of cargo pays a metre carried: every cargo's 4 falling to
/// 3.2, a quarter of the way (`economy/cargotype.script`, `constantFn`).
pub const CARGO_FARE: f64 = 3.8;
/// What a passenger pays a metre carried in a vehicle of the usual price
/// factor (`passengersFn`: 62.5 × 0.001 × 35 × 1.5 × 1.1).
pub const PASSENGER_FARE: f64 = 3.609;
/// What a line takes a year for each of two towns' people who works or
/// shops in the other, a metre between the towns: the fixture player's
/// passenger trains took 19.0 million a year on 1,731 such people riding,
/// 8.29 million of them times metres (2026-11-11), some 0.63 fares each a
/// year.
pub const LINK_FARE: f64 = 2.29;
/// The fares a year of each such person: [`LINK_FARE`] over
/// [`PASSENGER_FARE`].
pub const LINK_RIDES: f64 = LINK_FARE / PASSENGER_FARE;
/// How long a road vehicle spends at its stops in one round of its line,
/// in seconds, and the share of its top speed it averages over the
/// straight line between them, the road's bends, junctions and traffic
/// counted. The fixture player's trucks take 88 to 113 s a way over 350
/// to 500 m and 207 to 220 s over 1,433 m (their vehicles' `sectionTimes`,
/// 2026-11-11): a minute a stop and 9 m/s. Between towns the roads wind:
/// run 63's buses, sold as 35 km/h, took 465 s between two towns 1,888 m
/// apart and 792 s over 3,604 m, 4 to 4.6 m/s.
pub const ROAD_ROUND: f64 = 130.0;
pub const ROAD_PACE: f64 = 0.5;
/// As [`ROAD_ROUND`] and [`ROAD_PACE`], a train's: the fixture player's
/// cargo trains take 139 to 170 s a way over 2,238 m and 293 s over
/// 5,797 m, its passenger trains 130 to 147 s between stations 1.8 km
/// apart.
pub const RAIL_ROUND: f64 = 100.0;
pub const RAIL_PACE: f64 = 0.85;

/// A vehicle as a line would run it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Runs {
    /// What it carries at once.
    pub capacity: f64,
    /// Its top speed, in km/h.
    pub speed: f64,
    /// What it costs to buy, and to run a year.
    pub price: f64,
    pub running: f64,
    /// Whether it runs on rails.
    pub rail: bool,
}

/// The longest a road vehicle may take one way, in the game's seconds:
/// cargo not delivered within 2,250 s of being made is lost (every cargo's
/// `timeToDeliverInSeconds`), and it waits at its station first: half of
/// that. A truck of 1922, 25 km/h, would take 2,290 s over the 7.9 km run
/// 66's AI chose for one.
pub const WAY_MOST: f64 = 1_100.0;

impl Runs {
    /// How long it takes one way over `distance`, driving, in seconds.
    pub fn way(&self, distance: f64) -> f64 {
        let pace = if self.rail { RAIL_PACE } else { ROAD_PACE };
        distance.max(1.0) / (self.speed * pace / 3.6).max(0.5)
    }

    /// The rounds of a line `distance` long it makes in a year.
    pub fn rounds(&self, distance: f64) -> f64 {
        let (pace, stops) = if self.rail {
            (RAIL_PACE, RAIL_ROUND)
        } else {
            (ROAD_PACE, ROAD_ROUND)
        };
        let metres_a_second = (self.speed * pace / 3.6).max(0.5);
        YEAR / (2.0 * distance.max(1.0) / metres_a_second + stops)
    }
}

/// What a line is reckoned to bring.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    /// The vehicles it needs to carry what there is.
    pub vehicles: u32,
    /// What it carries a year, with those vehicles.
    pub carried: f64,
    /// What it takes a year in fares, and costs a year to run.
    pub income: f64,
    pub running: f64,
    /// What it costs to open: its vehicles and what is built for it.
    pub capital: f64,
}

impl Estimate {
    /// What it makes a year.
    pub fn profit(&self) -> f64 {
        self.income - self.running
    }

    /// The years it takes to earn back what it cost; none where it never
    /// does.
    pub fn payback(&self) -> Option<f64> {
        let profit = self.profit();
        (profit > 0.0).then(|| self.capital / profit)
    }

    /// What it makes a year for the money put in, half a million counted
    /// on top of every line: how a player ranks what to build next, a line
    /// that makes much for much before one that makes little for nearly
    /// nothing.
    pub fn worth(&self) -> f64 {
        self.profit() / (self.capital.max(0.0) + 500_000.0)
    }
}

/// A cargo line: `units` a year to carry `distance` metres one way, empty
/// back, by vehicles like `runs`, at most `vehicles_most` of them; `built`
/// is what its stations and depot cost.
pub fn cargo(units: f64, distance: f64, runs: Runs, vehicles_most: u32, built: f64) -> Estimate {
    one_way(units, distance, runs, vehicles_most, built, CARGO_FARE, 1.0)
}

/// A passenger line between two towns: `people` of them work or shop in
/// the other and ride no line yet (both ways together), `share` of whom it
/// is reckoned to win. A vehicle fills both ways, each way half of them.
pub fn passengers(
    people: f64,
    share: f64,
    distance: f64,
    runs: Runs,
    vehicles_most: u32,
    built: f64,
) -> Estimate {
    let rides = people.max(0.0) * share.clamp(0.0, 1.0) * LINK_RIDES;
    // Each way carries half the rides: a round of a vehicle carries two
    // loads where a cargo's carries one.
    one_way(
        rides / 2.0,
        distance,
        runs,
        vehicles_most,
        built,
        PASSENGER_FARE,
        2.0,
    )
}

fn one_way(
    units: f64,
    distance: f64,
    runs: Runs,
    vehicles_most: u32,
    built: f64,
    fare: f64,
    ways: f64,
) -> Estimate {
    let units = units.max(0.0);
    let a_vehicle = (runs.capacity * runs.rounds(distance)).max(0.001);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let wanted = (units / a_vehicle).ceil().max(1.0) as u32;
    let vehicles = wanted.min(vehicles_most.max(1));
    let carried = units.min(a_vehicle * f64::from(vehicles)) * ways;
    Estimate {
        vehicles,
        carried,
        income: carried * distance * fare,
        running: runs.running * f64::from(vehicles),
        capital: runs.price * f64::from(vehicles) + built,
    }
}

/// What its own lines have taught it: how what a kind of line earned
/// compares with what it reckoned, as a player learns a map. Each starts at
/// 1 and moves a third of the way to each settled line's ratio, within
/// [`Learned::LEAST`] and [`Learned::MOST`].
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Learned {
    pub bus: f32,
    pub truck: f32,
}

impl Default for Learned {
    fn default() -> Self {
        Self {
            bus: 1.0,
            truck: 1.0,
        }
    }
}

impl Learned {
    pub const LEAST: f32 = 0.2;
    pub const MOST: f32 = 2.0;

    /// The factor moved towards `earned` over `reckoned`.
    pub fn moved(factor: f32, earned: f64, reckoned: f64) -> f32 {
        if reckoned <= 0.0 || !earned.is_finite() {
            return factor;
        }
        #[allow(clippy::cast_possible_truncation)]
        let ratio = (earned / reckoned).clamp(0.0, 4.0) as f32;
        (factor + (ratio - factor) / 3.0).clamp(Self::LEAST, Self::MOST)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRUCK: Runs = Runs {
        capacity: 12.0,
        speed: 40.0,
        price: 100_000.0,
        running: 17_000.0,
        rail: false,
    };

    #[test]
    fn the_fixture_players_truck_line_is_reckoned_about_what_it_took() {
        // One truck of 12, 1,433 m: 188,420 a year in the fixture, on a
        // straight road; reckoned as on a winding one, a little under.
        let line = cargo(1_000.0, 1_433.0, TRUCK, 1, 0.0);
        assert_eq!(line.vehicles, 1);
        assert!(
            (110_000.0..230_000.0).contains(&line.income),
            "{}",
            line.income
        );
    }

    #[test]
    fn a_longer_haul_of_the_same_cargo_earns_more() {
        let near = cargo(200.0, 900.0, TRUCK, 32, 300_000.0);
        let far = cargo(200.0, 4_000.0, TRUCK, 32, 300_000.0);
        assert!(far.profit() > 3.0 * near.profit(), "{near:?} {far:?}");
        assert!(
            far.vehicles > near.vehicles,
            "more trucks for the same cargo further"
        );
        assert!((far.carried - 200.0).abs() < 1e-6, "all of it carried");
    }

    #[test]
    fn too_few_vehicles_carry_only_what_they_can() {
        let line = cargo(500.0, 3_000.0, TRUCK, 2, 0.0);
        assert_eq!(line.vehicles, 2);
        assert!(line.carried < 100.0, "{}", line.carried);
        assert!((line.income - line.carried * 3_000.0 * CARGO_FARE).abs() < 1.0);
    }

    #[test]
    fn two_towns_people_fill_a_bus_both_ways() {
        let bus = Runs {
            capacity: 8.0,
            speed: 45.0,
            price: 120_000.0,
            running: 30_000.0,
            rail: false,
        };
        // 95 people between two towns 1,871 m apart (the fixture's Waltham
        // Cross and Hunstanton): some 60 fares a year, 400,000.
        let line = passengers(95.0, 1.0, 1_871.0, bus, 8, 250_000.0);
        assert_eq!(line.vehicles, 2, "a bus makes under three rounds a year");
        assert!(
            (line.carried - 95.0 * LINK_RIDES).abs() < 0.5,
            "{}",
            line.carried
        );
        assert!(
            (350_000.0..450_000.0).contains(&line.income),
            "{}",
            line.income
        );
        assert!(line.payback().is_some_and(|years| years < 1.5), "{line:?}");
        // No one to carry: it never pays back.
        let empty = passengers(0.0, 1.0, 1_871.0, bus, 8, 250_000.0);
        assert_eq!(empty.payback(), None);
    }

    #[test]
    fn what_a_line_earned_moves_what_it_reckons() {
        let mut factor = 1.0;
        for _ in 0..12 {
            factor = Learned::moved(factor, 100_000.0, 400_000.0);
        }
        assert!((factor - 0.25).abs() < 0.02, "{factor}");
        assert_eq!(
            Learned::moved(1.0, 0.0, 0.0),
            1.0,
            "nothing reckoned, nothing learned"
        );
        assert!(Learned::moved(0.2, 0.0, 100.0) >= Learned::LEAST);
        assert!(Learned::moved(2.0, 1e9, 1.0) <= Learned::MOST);
    }
}

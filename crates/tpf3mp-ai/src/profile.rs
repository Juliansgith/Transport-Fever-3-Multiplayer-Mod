//! How each difficulty plays (docs/AI_PLAYERS.md, "The three
//! difficulties"): competence and tempo, never money, prices or knowledge a
//! player lacks.

use std::time::Duration;

use tpf3mp_proto::AiDifficulty;

/// What a difficulty decides with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Profile {
    pub difficulty: AiDifficulty,
    /// The least time between two of its actions, in real time: no faster
    /// hands than a person's, whatever the room's speed.
    pub action_gap: Duration,
    /// How often it looks at its company and its plans anew, at the room's
    /// 1x; faster rooms think proportionally sooner.
    pub think_every: Duration,
    /// Of the best towns it weighs, how many it may pick among: 1 always
    /// picks the best; more picks among the first that many.
    pub pick_among: usize,
    /// How many towns it weighs at all.
    pub towns_weighed: usize,
    /// Buses each of its lines runs at first, and at most.
    pub buses_first: u8,
    pub buses_most: u8,
    /// Stops a town line serves: at most one by each of the town's
    /// districts' centres.
    pub stops_most: usize,
    /// The share of its money it keeps in hand, in percent.
    pub reserve_percent: i64,
    /// Whether its model choice weighs capacity and speed for the money
    /// (true), or takes the cheapest bus (false).
    pub best_models: bool,
    /// The most loans it owes at once: a founded company starts with no
    /// money, so every difficulty borrows to start; harder ones borrow on
    /// to grow.
    pub loans_most: u8,
    /// Whether it takes the largest loan offered (true), or the smallest.
    pub big_loans: bool,
    /// The farthest it carries cargo by truck, in metres.
    pub cargo_most: f64,
    /// A road line running at least this full, in percent, with more
    /// waiting, gets one more vehicle; one this empty, settled, one fewer.
    pub add_above: u8,
    pub remove_below: u8,
    /// How far through its life, in percent, a road vehicle is replaced.
    pub replace_at: u16,
    /// How many levels of industries a production chain feeds.
    pub chain_depth: usize,
    /// Whether it joins large towns by passenger train.
    pub passenger_trains: bool,
}

impl Profile {
    pub fn of(difficulty: AiDifficulty) -> Self {
        match difficulty {
            AiDifficulty::Easy => Self {
                difficulty,
                action_gap: Duration::from_secs(20),
                think_every: Duration::from_secs(240),
                pick_among: 3,
                towns_weighed: 5,
                buses_first: 1,
                buses_most: 2,
                stops_most: 2,
                reserve_percent: 30,
                best_models: false,
                loans_most: 1,
                big_loans: false,
                cargo_most: 4_000.0,
                add_above: 90,
                remove_below: 25,
                replace_at: 100,
                chain_depth: 2,
                passenger_trains: false,
            },
            AiDifficulty::Normal => Self {
                difficulty,
                action_gap: Duration::from_secs(5),
                think_every: Duration::from_secs(120),
                pick_among: 2,
                towns_weighed: 20,
                buses_first: 2,
                buses_most: 4,
                stops_most: 3,
                reserve_percent: 15,
                best_models: true,
                loans_most: 2,
                big_loans: false,
                cargo_most: 6_000.0,
                add_above: 80,
                remove_below: 30,
                replace_at: 100,
                chain_depth: 2,
                passenger_trains: true,
            },
            AiDifficulty::Hard => Self {
                difficulty,
                action_gap: Duration::from_secs(1),
                think_every: Duration::from_secs(30),
                pick_among: 1,
                towns_weighed: 160,
                buses_first: 3,
                buses_most: 6,
                stops_most: 3,
                reserve_percent: 5,
                best_models: true,
                loans_most: 4,
                big_loans: true,
                cargo_most: 8_000.0,
                add_above: 70,
                remove_below: 35,
                replace_at: 90,
                chain_depth: 3,
                passenger_trains: true,
            },
        }
    }

    /// How long until the next look at the company, at a room speed in
    /// percent (100 is 1x): sooner the faster the room runs, never sooner
    /// than its actions may follow each other.
    pub fn think_after(&self, speed: u16) -> Duration {
        let factor = u32::from(speed.max(100)) / 100;
        (self.think_every / factor.max(1)).max(self.action_gap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harder_plays_faster_and_better_never_with_more_than_a_player_has() {
        let (easy, normal, hard) = (
            Profile::of(AiDifficulty::Easy),
            Profile::of(AiDifficulty::Normal),
            Profile::of(AiDifficulty::Hard),
        );
        assert!(easy.action_gap > normal.action_gap && normal.action_gap > hard.action_gap);
        assert!(easy.think_every > normal.think_every && normal.think_every > hard.think_every);
        assert!(
            easy.towns_weighed < normal.towns_weighed && normal.towns_weighed < hard.towns_weighed
        );
        assert!(easy.reserve_percent > hard.reserve_percent);
        assert_eq!(hard.pick_among, 1, "Hard always picks the best");
        assert!(
            hard.action_gap >= Duration::from_secs(1),
            "no faster than a quick player"
        );
    }

    #[test]
    fn a_faster_room_thinks_sooner_but_never_faster_than_its_hands() {
        let hard = Profile::of(AiDifficulty::Hard);
        assert_eq!(hard.think_after(100), Duration::from_secs(30));
        assert_eq!(hard.think_after(400), Duration::from_millis(7_500));
        assert_eq!(hard.think_after(1600), Duration::from_millis(1_875));
        assert_eq!(
            hard.think_after(0),
            Duration::from_secs(30),
            "paused counts as 1x"
        );
        let easy = Profile::of(AiDifficulty::Easy);
        assert_eq!(
            easy.think_after(1600),
            Duration::from_secs(20),
            "its hands' gap"
        );
    }
}

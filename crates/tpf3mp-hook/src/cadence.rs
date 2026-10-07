//! How many of the room's released steps one call of the game's step runs,
//! so that vehicles move evenly (docs/HOOKS.md, "Even steps").
//!
//! The game calls its step on its own clock, about every 200 ms at any
//! speed. The agent releases the room's steps on another, its playout's: one
//! every 200 ms at 1x and 5 steps a second, one every 50 ms at 4x. Running
//! whatever is released at the call made the count alias against the call
//! clock's jitter of +-10 ms: measured in the real game at 4x, 45 % of the
//! calls ran 3, 5 or 1 and 7 updates instead of 4, and the renderer, which
//! interpolates from each call, showed vehicles wobbling.
//!
//! [`Cadence`] instead runs the room's pace times the game's call period
//! (its *nominal* count, with a fractional remainder carried to the next
//! call), keeps a reserve of [`RESERVE`] released steps so a step released a
//! few milliseconds late never shows, and repays a backlog gently. It only
//! splits the steps already released into calls: which steps run, in which
//! order, and the actions between them stay as they are. It runs everything
//! released, as before, when an action, save, load or end waits behind the
//! released steps (no delay on the player's commands), when the room pauses
//! or stops releasing (every game stops at the frontier), and when the game
//! is far behind (a reconnect, a load).
//!
//! Pure: the caller passes the time in, so the rules are tested with a
//! scripted clock.

use std::time::{Duration, Instant};

use tpf3mp_proto::Speed;

/// Set to `0` (or `off`) in the game's environment, each call runs
/// everything released, as before.
pub const ENV: &str = "TPF3MP_HOOK_EVEN_STEPS";

/// Released steps kept back in the steady state: the slack that absorbs a
/// release a little late against the game's call.
pub const RESERVE: u64 = 1;

/// Released steps the reserve may grow by before a backlog is repaid: a
/// call that finds one step more or less than usual stays in the band.
pub const BAND: u64 = 2;

/// The player's action is taken to be on its way through the room for at
/// most this long after it was handed over: far more than a round trip and
/// a playout buffer, so it comes back within it.
pub const COMMAND_WAIT: Duration = Duration::from_secs(3);

/// No new step released for this long: the room waits (for its slowest
/// member, or a pause on its way), so whatever is released runs.
pub const STALE_AFTER: Duration = Duration::from_millis(400);

/// More released than this much of the room's pace: the game is far behind
/// and catches up as fast as it can. Below the server's pacing window (two
/// seconds of pace plus the input delay), so a game that falls behind
/// catches up before it holds the room back.
pub const FAR_BEHIND: Duration = Duration::from_millis(1500);

/// Never "far" below this many steps (one call's cap).
pub const FAR_STEPS_MIN: u64 = 16;

/// The game's call period assumed before it is measured: 1x.
pub const DEFAULT_PERIOD: Duration = Duration::from_millis(200);

/// A call gap shorter than this is two calls in one frame, never the
/// game's cadence.
const MIN_GAP: Duration = Duration::from_millis(20);

/// The call gaps the period is measured over: the latest this many.
const GAPS: usize = 16;

/// The shortest and the longest this many of them are left out: a freeze (a
/// save, a load, a long frame) or two does not move the period, while
/// uneven calls (150 and 350 ms in turn) still average to what they are.
const GAPS_TRIMMED: usize = 2;

/// A rate of steps per call this close to a whole number is taken as it.
const WHOLE_RATE: f64 = 0.15;

/// A backlog above the band is repaid by a quarter a call, at most half the
/// nominal count (at least one step) more.
const REPAY_DIVISOR: u64 = 4;

/// Whether [`ENV`]'s value leaves even steps on: anything but an explicit
/// no.
pub fn wanted(value: Option<&str>) -> bool {
    !matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("0" | "off" | "false" | "no")
    )
}

/// Which rule chose a call's count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// Even steps are off: everything released.
    Off,
    /// The nominal count, the steady state.
    Nominal,
    /// One less than nominal, to rebuild the reserve.
    Reserve,
    /// Fewer released than the nominal count less one: all of them.
    Underrun,
    /// More than nominal, repaying a backlog.
    Repay,
    /// An action, save, load or end waits behind the released steps.
    Barrier,
    /// The player's own action is on its way through the room: nothing is
    /// kept back, so it comes back as soon as without even steps.
    Command,
    /// The room is paused: run on to its frontier.
    Paused,
    /// Nothing released for [`STALE_AFTER`]: run on to the frontier.
    Stale,
    /// Far behind: catch up.
    Far,
}

impl Pick {
    pub const ALL: [Pick; 10] = [
        Pick::Off,
        Pick::Nominal,
        Pick::Reserve,
        Pick::Underrun,
        Pick::Repay,
        Pick::Barrier,
        Pick::Command,
        Pick::Paused,
        Pick::Stale,
        Pick::Far,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Pick::Off => "off",
            Pick::Nominal => "nominal",
            Pick::Reserve => "reserve",
            Pick::Underrun => "underrun",
            Pick::Repay => "repay",
            Pick::Barrier => "barrier",
            Pick::Command => "command",
            Pick::Paused => "paused",
            Pick::Stale => "stale",
            Pick::Far => "far",
        }
    }

    pub fn index(self) -> usize {
        Pick::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }
}

/// One call's count, and how it was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    /// The updates to run: at most the cap, 0 only to rebuild the reserve
    /// (or for a room slower than the game's calls).
    pub steps: u32,
    pub pick: Pick,
    /// The steps released and not run before this call.
    pub available: u64,
    /// The batch cap: [`crate::step::MAX_STEPS_PER_CALL`] and the next
    /// checkpoint.
    pub cap: u32,
    /// The nominal count for this call.
    pub nominal: u64,
}

impl Choice {
    /// The step trace's note.
    pub fn note(&self) -> String {
        format!(
            "avail={} cap={} nom={} pick={}",
            self.available,
            self.cap,
            self.nominal,
            self.pick.name()
        )
    }
}

/// The even-steps state of one game.
#[derive(Debug, Clone)]
pub struct Cadence {
    on: bool,
    steps_per_second: u16,
    /// The room's speed in percent; the last one before a pause while
    /// paused, which steps released before the pause still play at.
    speed: u16,
    paused: bool,
    /// The game's call period, seconds: the trimmed mean of `gaps`.
    period: f64,
    /// The latest call gaps, seconds, oldest first.
    gaps: std::collections::VecDeque<f64>,
    last_call: Option<Instant>,
    /// The fractional step carried to the next call, in [0, 1).
    credit: f64,
    /// The released counter seen last, and when it last grew.
    released: u64,
    last_arrival: Option<Instant>,
}

impl Cadence {
    pub fn new(on: bool) -> Self {
        Self {
            on,
            steps_per_second: 5,
            speed: Speed::NORMAL.0,
            paused: false,
            period: DEFAULT_PERIOD.as_secs_f64(),
            gaps: std::collections::VecDeque::with_capacity(GAPS),
            last_call: None,
            credit: 0.0,
            released: 0,
            last_arrival: None,
        }
    }

    pub fn on(&self) -> bool {
        self.on
    }

    pub fn set_on(&mut self, on: bool) {
        self.on = on;
    }

    /// The room began a game at `steps_per_second`.
    pub fn begin(&mut self, steps_per_second: u16) {
        self.steps_per_second = steps_per_second.max(1);
        self.speed = Speed::NORMAL.0;
        self.paused = false;
        self.credit = 0.0;
    }

    /// The room's speed, as the agent tells it.
    pub fn speed(&mut self, speed: Speed) {
        if speed.is_paused() {
            self.paused = true;
        } else {
            if self.paused || speed.0 != self.speed {
                self.credit = 0.0;
            }
            self.paused = false;
            self.speed = speed.0;
        }
    }

    /// The world runs from a new place (a load, the room's begin, a hold):
    /// the released counter starts again at `released`, and the steps
    /// released so far count as just arrived.
    pub fn reset(&mut self, released: u64, now: Instant) {
        self.released = released;
        self.last_arrival = Some(now);
        self.credit = 0.0;
    }

    /// Every call of the game's step, at its start: measures the game's
    /// call period.
    pub fn call(&mut self, now: Instant) {
        let Some(last) = self.last_call.replace(now) else {
            return;
        };
        let gap = now.saturating_duration_since(last).as_secs_f64();
        if gap < MIN_GAP.as_secs_f64() {
            // Never the game's cadence (measured: 127 ms and more at 1x).
            return;
        }
        if self.gaps.len() == GAPS {
            self.gaps.pop_front();
        }
        self.gaps.push_back(gap);
        if self.gaps.len() < GAPS {
            // Too few to leave any out: the default until the window fills.
            return;
        }
        let mut sorted: Vec<f64> = self.gaps.iter().copied().collect();
        sorted.sort_by(f64::total_cmp);
        let kept = &sorted[GAPS_TRIMMED..GAPS - GAPS_TRIMMED];
        #[allow(clippy::cast_precision_loss)]
        let mean = kept.iter().sum::<f64>() / kept.len() as f64;
        self.period = mean.clamp(0.02, 2.0);
    }

    /// The room's pace in steps a second (the pre-pause pace while paused).
    fn pace(&self) -> f64 {
        f64::from(self.steps_per_second) * f64::from(self.speed) / 100.0
    }

    /// The game's call period as measured.
    pub fn period(&self) -> Duration {
        Duration::from_secs_f64(self.period)
    }

    /// A call that may run room steps: `next` is the step it runs first,
    /// `released` the last step released (read up to the first message
    /// that is not a release), `cap` the most the gate lets one batch run
    /// (its cap and the next checkpoint), `to_checkpoint` the steps up to
    /// and including the next checkpoint step, `barrier` whether a message
    /// that must wait for the released steps came after them, `command`
    /// whether the player's own action is on its way through the room.
    #[allow(clippy::too_many_arguments)]
    pub fn choose(
        &mut self,
        next: u64,
        released: u64,
        cap: u32,
        to_checkpoint: u64,
        barrier: bool,
        command: bool,
        now: Instant,
    ) -> Choice {
        if released != self.released {
            if released > self.released {
                self.last_arrival = Some(now);
            }
            self.released = released;
        }
        let available = released.saturating_add(1).saturating_sub(next);
        let cap = u32::try_from(u64::from(cap).min(available)).unwrap_or(0);
        let mut choice = Choice {
            steps: cap,
            pick: Pick::Off,
            available,
            cap,
            nominal: 0,
        };
        if !self.on || cap == 0 {
            return choice;
        }
        // The nominal count. A rate near a whole number (5 steps a second
        // at 1x, 2x, 4x against 5 calls a second) is that number: the
        // measured period wanders by a few milliseconds, and carrying its
        // fractions would bring back the very 3/5 alternation this removes;
        // the band repays what a slightly wrong period leaves. A rate truly
        // between (7 steps a second) carries its fraction to an even mix.
        let rate = self.pace() * self.period;
        let nominal = if (rate - rate.round()).abs() <= WHOLE_RATE {
            self.credit = 0.0;
            rate.round().max(0.0)
        } else {
            self.credit += rate;
            let whole = (self.credit + 1e-6).floor().max(0.0);
            self.credit = (self.credit - whole).clamp(0.0, 0.999_999);
            whole
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let base = nominal as u64;
        let behind = available.saturating_sub(base) > RESERVE + BAND;
        let nominal = toward_checkpoint(base, to_checkpoint, behind);
        // Spreading the steps to a checkpoint: a backlog waits until it is
        // passed, or repaying it would bring back the short call.
        let planned = nominal != base;
        choice.nominal = nominal;
        let far = ((self.pace() * FAR_BEHIND.as_secs_f64()).ceil() as u64).max(FAR_STEPS_MIN);
        let stale = self
            .last_arrival
            .is_some_and(|at| now.saturating_duration_since(at) >= STALE_AFTER);
        // Everything ahead of an action, as before: holding any back would
        // cost the action a call, and a game catching up through a busy
        // room's actions its pace (only the steps up to the first action
        // are seen, so `far` cannot tell).
        let (steps, pick) = if barrier {
            (available, Pick::Barrier)
        } else if command {
            // A step kept back now would cost the action a call (200 ms)
            // when it arrives behind it.
            (available, Pick::Command)
        } else if available > far {
            (available, Pick::Far)
        } else if self.paused {
            (available, Pick::Paused)
        } else if stale {
            (available, Pick::Stale)
        } else if available < nominal + RESERVE {
            // Short of the reserve: rebuild it, one step less than nominal.
            let less = nominal.saturating_sub(1);
            if available < less {
                (available, Pick::Underrun)
            } else {
                (less, Pick::Reserve)
            }
        } else {
            // What stays released after the nominal count: anywhere in the
            // band is fine, so a release a little early or late (+-1 a
            // call) changes nothing; above it, repay towards its middle.
            let left = available - nominal;
            if left <= RESERVE + BAND || planned {
                (nominal, Pick::Nominal)
            } else {
                let repay = (left - RESERVE - BAND / 2)
                    .div_ceil(REPAY_DIVISOR)
                    .min((nominal / 2).max(1));
                (nominal + repay, Pick::Repay)
            }
        };
        choice.steps = u32::try_from(steps.min(u64::from(cap))).unwrap_or(cap);
        choice.pick = pick;
        choice
    }
}

/// The nominal count near a checkpoint, which every batch must end at:
/// the steps left to it spread evenly over the calls they take, so the
/// last call before it is not a short one (4 3 3 rather than 4 4 2 at 4x,
/// with 10 steps to go). `behind`, over fewer calls (5 5), so a backlog is
/// repaid even where checkpoints come so often that every call is spread.
fn toward_checkpoint(nominal: u64, to_checkpoint: u64, behind: bool) -> u64 {
    if nominal == 0 || to_checkpoint >= 3 * nominal {
        return nominal;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let calls = to_checkpoint as f64 / nominal as f64;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let calls = (if behind { calls.floor() } else { calls.round() } as u64).max(1);
    let spread = to_checkpoint.div_ceil(calls);
    if behind {
        // A room whose checkpoints come every 1.75 calls (7 steps at 4x)
        // needs some in one call: a larger call, never a lasting backlog.
        spread
    } else {
        spread.min(nominal + (nominal / 2).max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A room releasing on its own grid and a game calling on a jittered
    /// clock, run through a [`Cadence`]: the counts each call ran.
    struct Sim {
        cadence: Cadence,
        start: Instant,
        /// Milliseconds between releases.
        release_every: f64,
        /// Offset of the first release, ms.
        phase: f64,
        next: u64,
        frozen_until: f64,
        checkpoint: u64,
        /// A late turn: between these times (ms) nothing new arrives.
        late: Option<(f64, f64)>,
    }

    impl Sim {
        fn new(speed: u16, phase: f64) -> Self {
            let mut cadence = Cadence::new(true);
            cadence.begin(5);
            cadence.speed(Speed(speed));
            let start = Instant::now();
            cadence.reset(0, start);
            Self {
                cadence,
                start,
                release_every: 200.0 * 100.0 / f64::from(speed),
                phase,
                next: 1,
                frozen_until: 0.0,
                checkpoint: 50,
                late: None,
            }
        }

        fn at(&self, ms: f64) -> Instant {
            self.start + Duration::from_secs_f64(ms / 1000.0)
        }

        fn released_at(&self, ms: f64) -> u64 {
            let ms = match self.late {
                Some((from, to)) if (from..to).contains(&ms) => from,
                _ => ms,
            };
            if ms < self.phase {
                return 0;
            }
            ((ms - self.phase) / self.release_every).floor() as u64 + 1
        }

        /// One call at `ms`: what it ran.
        fn call(&mut self, ms: f64) -> Choice {
            let now = self.at(ms);
            self.cadence.call(now);
            let released = self.released_at(ms);
            let to_checkpoint =
                (self.checkpoint - self.next % self.checkpoint) % self.checkpoint + 1;
            let cap = 16u64.min(to_checkpoint) as u32;
            let choice =
                self.cadence
                    .choose(self.next, released, cap, to_checkpoint, false, false, now);
            self.next += u64::from(choice.steps);
            choice
        }

        /// Calls every 200 ms with a jitter pattern, from `from` ms, `n`
        /// calls: the counts.
        fn run(&mut self, from: f64, n: usize, jitter: &[f64]) -> Vec<u32> {
            // A freeze delays the calls after it; they come on 200 ms apart.
            let shift = (self.frozen_until - from).max(0.0);
            (0..n)
                .map(|i| {
                    let ms = from + shift + i as f64 * 200.0 + jitter[i % jitter.len()];
                    self.call(ms).steps
                })
                .collect()
        }
    }

    /// Jitter of +-10 ms, irregular.
    const JITTER: [f64; 7] = [0.0, 9.0, -8.0, 4.0, -10.0, 7.0, -3.0];

    fn settled(counts: &[u32]) -> &[u32] {
        &counts[counts.len() / 4..]
    }

    #[test]
    fn at_4x_a_jittered_call_clock_runs_four_updates_every_call() {
        // The release grid's edge right at the calls: the old rule ran
        // 3 5 3 5 here, and 2 5 5 or 1 5 5 5 at each checkpoint.
        for phase in [0.0, 1.0, 25.0, 49.0] {
            let mut sim = Sim::new(400, phase);
            let counts = sim.run(0.0, 200, &JITTER);
            let steady = settled(&counts);
            // Never more than one off; off only to end a batch at a
            // checkpoint (every 50 steps, 12.5 calls) and to repay it.
            assert!(
                steady.iter().all(|n| (3..=5).contains(n)),
                "phase {phase}: {counts:?}"
            );
            let off = steady.iter().filter(|n| **n != 4).count();
            assert!(off <= steady.len() / 4, "phase {phase}: {counts:?}");
        }
    }

    #[test]
    fn at_1x_with_the_release_at_the_call_edge_one_update_runs_every_call() {
        for phase in [0.0, 2.0, 198.0, 100.0] {
            let mut sim = Sim::new(100, phase);
            let counts = sim.run(0.0, 200, &JITTER);
            assert!(
                settled(&counts).iter().all(|n| *n == 1),
                "phase {phase}: {counts:?}"
            );
        }
    }

    #[test]
    fn at_2x_every_call_runs_two_once_settled() {
        for phase in [0.0, 3.0, 97.0] {
            let mut sim = Sim::new(200, phase);
            let counts = sim.run(0.0, 200, &JITTER);
            let steady = settled(&counts);
            let off = steady.iter().filter(|n| **n != 2).count();
            assert!(off <= steady.len() / 20, "phase {phase}: {counts:?}");
            assert!(steady.iter().all(|n| (1..=3).contains(n)), "{counts:?}");
        }
    }

    #[test]
    fn a_freeze_is_repaid_without_a_burst() {
        let mut sim = Sim::new(400, 10.0);
        let mut counts = sim.run(0.0, 50, &JITTER);
        // The room's save: the game stands 1.2 s, the room releases on.
        sim.frozen_until = 50.0 * 200.0 + 1200.0;
        counts.extend(sim.run(50.0 * 200.0, 100, &JITTER));
        let after = &counts[50..];
        assert!(after.iter().all(|n| *n <= 6), "{after:?}");
        // Repaid within the window: back to fours, one off at checkpoints.
        assert!(after[30..].iter().all(|n| (3..=5).contains(n)), "{after:?}");
    }

    #[test]
    fn a_late_turn_at_2x_never_stops_the_world_or_bursts() {
        let mut sim = Sim::new(200, 40.0);
        // Steps released from 10 s on arrive 300 ms late, all at once.
        sim.late = Some((10_000.0, 10_300.0));
        let counts = sim.run(0.0, 150, &JITTER);
        let around = &counts[40..80];
        // The old rule: a call of 0, then one of 3 or more.
        assert!(around.iter().all(|n| (1..=3).contains(n)), "{around:?}");
        assert!(
            around.iter().filter(|n| **n != 2).count() <= 6,
            "{around:?}"
        );
    }

    /// A room at `speed` with checkpoints every `interval` steps and a game
    /// calling with the gaps given, in turn, for `calls` calls: the counts,
    /// and the steps released and not run at the end.
    fn paced(speed: u16, interval: u64, gaps: &[f64], calls: usize) -> (Vec<u32>, u64) {
        let mut cadence = Cadence::new(true);
        cadence.begin(5);
        cadence.speed(Speed(speed));
        let start = Instant::now();
        cadence.reset(0, start);
        let every = 200.0 * 100.0 / f64::from(speed);
        let (mut ms, mut next, mut counts) = (0.0, 1u64, Vec::new());
        let mut released = 0;
        for i in 0..calls {
            ms += gaps[i % gaps.len()];
            let now = start + Duration::from_secs_f64(ms / 1000.0);
            cadence.call(now);
            released = (ms / every).floor() as u64;
            let to_checkpoint = (interval - next % interval) % interval + 1;
            let cap = 16u64.min(to_checkpoint) as u32;
            let choice = cadence.choose(next, released, cap, to_checkpoint, false, false, now);
            next += u64::from(choice.steps);
            counts.push(choice.steps);
        }
        (counts, released + 1 - next)
    }

    #[test]
    fn uneven_calls_keep_the_rooms_pace() {
        // A busy machine calling every 150 and 350 ms in turn, at 4x: the
        // period is their average, not their short half.
        let (counts, behind) = paced(400, 50, &[150.0, 350.0], 400);
        assert!(behind <= 8, "{behind} behind: {counts:?}");
        let (counts, behind) = paced(400, 50, &[180.0, 200.0, 260.0, 190.0, 170.0], 400);
        assert!(behind <= 8, "{behind} behind: {counts:?}");
    }

    #[test]
    fn checkpoints_every_few_steps_keep_the_rooms_pace() {
        // Every call is spread to a checkpoint 10 steps apart at 4x: the
        // backlog still gets repaid (4 3 3, then 5 5), never a slower room.
        let (counts, behind) = paced(400, 10, &[200.0], 300);
        assert!(behind <= 8, "{behind} behind: {counts:?}");
        assert!(
            counts[50..].iter().all(|n| (3..=6).contains(n)),
            "{counts:?}"
        );
        let (counts, behind) = paced(200, 10, &[200.0], 300);
        assert!(behind <= 6, "{behind} behind: {counts:?}");
        // Checkpoints 2 apart at 1x on a slow machine (225 ms calls), and
        // 7 apart at 4x: slower or bursty, but never a slower room.
        let (counts, behind) = paced(100, 2, &[225.0], 300);
        assert!(behind <= 4, "{behind} behind: {counts:?}");
        let (counts, behind) = paced(400, 7, &[200.0], 300);
        assert!(behind <= 12, "{behind} behind: {counts:?}");
    }

    #[test]
    fn the_players_own_action_on_its_way_keeps_nothing_back() {
        let mut cadence = Cadence::new(true);
        cadence.begin(5);
        let t = Instant::now();
        cadence.reset(0, t);
        let choice = cadence.choose(1, 2, 16, 50, false, true, t);
        assert_eq!((choice.steps, choice.pick), (2, Pick::Command));
    }

    #[test]
    fn a_waiting_action_runs_everything_released_at_once() {
        let mut cadence = Cadence::new(true);
        cadence.begin(5);
        cadence.speed(Speed(400));
        let t = Instant::now();
        cadence.reset(10, t);
        cadence.call(t);
        let choice = cadence.choose(6, 10, 16, 45, true, false, t);
        assert_eq!((choice.steps, choice.pick), (5, Pick::Barrier));
    }

    #[test]
    fn a_pause_and_a_room_that_stopped_releasing_run_on_to_the_frontier() {
        let mut cadence = Cadence::new(true);
        cadence.begin(5);
        cadence.speed(Speed(100));
        let t = Instant::now();
        cadence.reset(3, t);
        cadence.speed(Speed::PAUSED);
        let choice = cadence.choose(2, 3, 16, 49, false, false, t);
        assert_eq!((choice.steps, choice.pick), (2, Pick::Paused));
        // Resumed, then nothing new for longer than STALE_AFTER.
        cadence.speed(Speed(100));
        let later = t + STALE_AFTER;
        let choice = cadence.choose(2, 3, 16, 49, false, false, later);
        assert_eq!((choice.steps, choice.pick), (2, Pick::Stale));
    }

    #[test]
    fn far_behind_catches_up_by_the_cap() {
        let mut cadence = Cadence::new(true);
        cadence.begin(5);
        let t = Instant::now();
        cadence.reset(0, t);
        cadence.call(t);
        let choice = cadence.choose(1, 40, 16, 50, false, false, t);
        assert_eq!((choice.steps, choice.pick), (16, Pick::Far));
    }

    #[test]
    fn the_checkpoint_cut_and_the_cap_always_hold() {
        let mut cadence = Cadence::new(true);
        cadence.begin(5);
        cadence.speed(Speed(400));
        let t = Instant::now();
        cadence.reset(0, t);
        let choice = cadence.choose(1, 12, 2, 2, false, false, t);
        assert_eq!(choice.steps, 2);
        assert_eq!(choice.cap, 2);
    }

    #[test]
    fn a_room_slower_than_the_calls_runs_an_even_mix() {
        // 7 steps a second against 5 calls a second: 1 or 2, never 0 or 3,
        // two 2s in every five calls on average.
        let mut cadence = Cadence::new(true);
        cadence.begin(7);
        let start = Instant::now();
        cadence.reset(0, start);
        let mut next = 1;
        let mut counts = Vec::new();
        for i in 0..200u32 {
            let ms = f64::from(i) * 200.0 + JITTER[i as usize % JITTER.len()];
            let now = start + Duration::from_secs_f64(ms / 1000.0);
            cadence.call(now);
            let released = ((ms + 5.0) / (1000.0 / 7.0)).floor() as u64;
            let choice = cadence.choose(next, released, 16, u64::MAX, false, false, now);
            next += u64::from(choice.steps);
            counts.push(choice.steps);
        }
        let steady = settled(&counts);
        assert!(steady.iter().all(|n| (1..=2).contains(n)), "{counts:?}");
    }

    #[test]
    fn off_runs_everything_released_as_before() {
        let mut cadence = Cadence::new(false);
        let t = Instant::now();
        let choice = cadence.choose(1, 5, 16, 50, false, false, t);
        assert_eq!((choice.steps, choice.pick), (5, Pick::Off));
        assert!(wanted(None));
        assert!(wanted(Some("1")));
        assert!(!wanted(Some(" Off ")));
        assert!(!wanted(Some("0")));
    }

    #[test]
    fn a_speed_change_settles_on_the_new_count() {
        let mut sim = Sim::new(100, 30.0);
        let mut counts = sim.run(0.0, 50, &JITTER);
        // The room goes to 4x at 10 s: releases every 50 ms from there.
        let switch = 10_000.0 - 15.0;
        let before = sim.released_at(switch);
        sim.cadence.speed(Speed(400));
        sim.release_every = 50.0;
        sim.phase = switch - (before as f64 - 1.0) * 50.0;
        counts.extend(sim.run(10_000.0, 100, &JITTER));
        let after = &counts[50..];
        assert!(after.iter().all(|n| *n <= 6), "{after:?}");
        assert!(after[20..].iter().all(|n| (3..=5).contains(n)), "{after:?}");
    }
}

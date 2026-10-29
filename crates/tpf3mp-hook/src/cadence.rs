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

/// Opt-in recovery for short batches whose native copy/readiness overhead
/// prevents the interval trim alone from keeping up. Default is unchanged.
pub const SHORT_RECOVERY_ENV: &str = "TPF3MP_HOOK_BATCH_RECOVERY";

pub fn short_recovery_wanted(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "on" | "true" | "yes")
    )
}

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
    /// When an opt-in cap leaves Far recovery uncompressed, keep timing
    /// feedback: relaxing to room pace would never repay the backlog. At
    /// 2x, for example, a cap of five is fully shown despite nominal three.
    /// Drains for actions, pauses and short checkpoint tails still relax.
    pub fn steady_interval(&self, fixed_nominal: u32, batch_limit: u32) -> bool {
        matches!(self.pick, Pick::Nominal | Pick::Repay)
            || (self.pick == Pick::Far
                && batch_limit < crate::step::MAX_STEPS_PER_CALL
                && fixed_nominal > 0
                && self.steps == batch_limit
                && crate::interval::shown(self.steps, fixed_nominal) == self.steps)
    }

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
    /// The batch interval times each call ([`crate::interval`]): a fixed
    /// nominal count, no repaying with extra updates.
    fixed: bool,
    /// Desired steady batch duration when the verified interval hook runs.
    batch_period: Duration,
    short_recovery: bool,
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
            fixed: false,
            batch_period: crate::interval::BASE,
            short_recovery: false,
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

    /// With the batch interval on: a fixed nominal count. Leaving it
    /// forgets the periods measured meanwhile: they were the interval's.
    pub fn set_fixed(&mut self, fixed: bool) {
        if self.fixed && !fixed {
            self.gaps.clear();
            self.period = DEFAULT_PERIOD.as_secs_f64();
            self.credit = 0.0;
        }
        self.fixed = fixed;
    }

    /// Whether the nominal count is fixed (the batch interval is on).
    pub fn fixed(&self) -> bool {
        self.fixed
    }

    /// Whether the room is paused, as last told.
    pub fn paused(&self) -> bool {
        self.paused
    }

    /// The room's time per step, µs (at the pre-pause pace while paused).
    pub fn step_us(&self) -> u32 {
        let pace = self.pace();
        if pace > 0.0 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let us = (1_000_000.0 / pace).round().min(f64::from(u32::MAX)) as u32;
            us
        } else {
            0
        }
    }

    /// The nominal count with the batch interval.
    pub fn fixed_nominal(&self) -> u32 {
        crate::interval::nominal(self.pace(), self.batch_period)
    }

    pub fn set_batch_period(&mut self, period: Duration) {
        self.batch_period = period;
    }

    pub fn set_short_recovery(&mut self, enabled: bool) {
        self.short_recovery = enabled;
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

    /// The room's pace now, steps a second; 0 while paused.
    pub fn pace_now(&self) -> f64 {
        if self.paused { 0.0 } else { self.pace() }
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
        let nominal = if self.fixed {
            // The batch interval times each call (crate::interval): the
            // count is the pace over the selected batch duration, never a
            // measured period the interval itself sets.
            f64::from(self.fixed_nominal())
        } else if (rate - rate.round()).abs() <= WHOLE_RATE {
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
            // Normally the interval's trim repays instead. A short batch
            // can nevertheless be throughput-limited by copy/readiness
            // overhead: increasing its frequency cannot create capacity.
            // The experiment amortizes that overhead over a few more
            // updates before reaching Far. Actual count still drives the
            // presentation interval, and the gate/checkpoint cap still wins.
            let short_repay = self.fixed
                && self.short_recovery
                && self.batch_period < crate::interval::BASE
                && left > RESERVE + BAND + 2 * base;
            if left <= RESERVE + BAND || planned || (self.fixed && !short_repay) {
                (nominal, Pick::Nominal)
            } else {
                // Three updates can only break even once checkpoint cuts
                // and frame quantization are included. If pressure grows,
                // allow up to twice nominal (still under the caller cap),
                // rather than accumulating 1.5 seconds before doing so.
                let recovery_cap = if short_repay && left > RESERVE + BAND + 4 * base {
                    base.max(1)
                } else {
                    (nominal / 2).max(1)
                };
                let repay = (left - RESERVE - BAND / 2)
                    .div_ceil(REPAY_DIVISOR)
                    .min(recovery_cap);
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

/// How long [`CatchUp`] looks back.
pub const CATCH_UP_WINDOW: Duration = Duration::from_secs(5);
/// Steps run against the room's pace over the window above which a game is
/// catching up (a join, a rejoin, a reload): it runs as fast as it can.
pub const CATCH_UP_RATIO: f64 = 1.25;

/// Whether this game has been catching up with the room lately: over the
/// last [`CATCH_UP_WINDOW`] it ran clearly more steps than the room's pace.
/// A game at the room's pace runs the pace (a backlog after a freeze is
/// repaid within a call or two); one replaying minutes of history after a
/// join runs 1.4 (4x) to 6 (1x) times it for as long as that takes. Only
/// the hook's own calls are counted, so it needs no word from the agent and
/// sees past the event that stops a batch.
#[derive(Debug, Default, Clone)]
pub struct CatchUp {
    calls: std::collections::VecDeque<(Instant, u32)>,
}

impl CatchUp {
    pub fn new() -> Self {
        Self::default()
    }

    /// A room call at `now` that ran `steps`.
    pub fn note(&mut self, now: Instant, steps: u32) {
        self.calls.push_back((now, steps));
        while let Some(&(at, _)) = self.calls.front() {
            if now.saturating_duration_since(at) > CATCH_UP_WINDOW {
                self.calls.pop_front();
            } else {
                break;
            }
        }
    }

    /// Forgets every call (a load, leaving the room's game).
    pub fn reset(&mut self) {
        self.calls.clear();
    }

    /// Whether the calls of the last window ran more than
    /// [`CATCH_UP_RATIO`] times `pace` steps a second; only once the window
    /// is nearly full, so a burst of a call or two never counts.
    pub fn catching_up(&self, now: Instant, pace: f64) -> bool {
        let Some(&(first, _)) = self.calls.front() else {
            return false;
        };
        let span = now.saturating_duration_since(first);
        if pace <= 0.0 || span < CATCH_UP_WINDOW.mul_f64(0.8) {
            return false;
        }
        let steps: u64 = self.calls.iter().map(|&(_, n)| u64::from(n)).sum();
        #[allow(clippy::cast_precision_loss)]
        let rate = steps as f64 / span.as_secs_f64();
        rate > CATCH_UP_RATIO * pace
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_far_feedback_does_not_time_drains_or_checkpoint_tails_as_steady() {
        let far = Choice {
            steps: 5,
            pick: Pick::Far,
            available: 35,
            cap: 5,
            nominal: 5,
        };
        assert!(far.steady_interval(5, 5));
        assert!(far.steady_interval(3, 5), "2x must also clear its backlog");
        assert!(Choice { steps: 4, ..far }.steady_interval(5, 4));
        assert!(
            !far.steady_interval(1, 5),
            "compressed recovery already accelerates"
        );
        assert!(!far.steady_interval(5, 16));
        for nominal in 1..=16 {
            assert!(
                !Choice { steps: 16, ..far }.steady_interval(nominal, 16),
                "the default cap must retain its timing even in faster-step rooms"
            );
        }
        assert!(!Choice { steps: 2, ..far }.steady_interval(5, 5));
        for pick in [
            Pick::Barrier,
            Pick::Command,
            Pick::Paused,
            Pick::Stale,
            Pick::Reserve,
            Pick::Underrun,
        ] {
            assert!(!Choice { pick, ..far }.steady_interval(5, 5));
        }
    }

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
    fn shorter_batches_choose_the_selected_count_after_world_reset() {
        let now = Instant::now();
        for (ms, count) in [(50, 1), (100, 2), (200, 4)] {
            let mut cadence = Cadence::new(true);
            cadence.set_batch_period(Duration::from_millis(ms));
            cadence.set_fixed(true);
            cadence.begin(5);
            cadence.speed(Speed(400));
            cadence.reset(0, now);
            let choice = cadence.choose(1, 5, 16, 50, false, false, now);
            assert_eq!(choice.steps, count);
            assert_eq!(cadence.fixed_nominal(), count);
            cadence.speed(Speed(100));
            assert_eq!(cadence.fixed_nominal(), 1);
        }
    }

    #[test]
    fn with_the_batch_interval_the_count_is_fixed_and_never_repays() {
        let mut cadence = Cadence::new(true);
        cadence.begin(5);
        cadence.speed(Speed(400));
        cadence.set_fixed(true);
        let start = Instant::now();
        cadence.reset(0, start);
        // Calls 300 ms apart (the interval set them): still 4, not 6.
        for i in 0..20u64 {
            cadence.call(start + Duration::from_millis(300 * i));
        }
        let t = start + Duration::from_secs(6);
        let choice = cadence.choose(1, 5, 16, 50, false, false, t);
        assert_eq!((choice.steps, choice.nominal), (4, 4));
        // A backlog of 10 is not repaid with updates (the trim does it).
        let choice = cadence.choose(5, 14, 16, 46, false, false, t);
        assert_eq!((choice.steps, choice.pick), (4, Pick::Nominal));
        // Off again: the measured periods are forgotten.
        cadence.set_fixed(false);
        assert_eq!(cadence.period(), DEFAULT_PERIOD);
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

    fn short_candidate(recovery: bool) -> Cadence {
        let mut cadence = Cadence::new(true);
        cadence.begin(5);
        cadence.speed(Speed(400));
        cadence.set_fixed(true);
        cadence.set_batch_period(Duration::from_millis(100));
        cadence.set_short_recovery(recovery);
        cadence
    }

    #[test]
    fn short_recovery_is_explicit_and_leaves_ordinary_batches_and_boundaries_alone() {
        for value in [None, Some("0"), Some("off"), Some(""), Some("typo")] {
            assert!(!short_recovery_wanted(value));
        }
        assert!(short_recovery_wanted(Some(" 1 ")));
        let now = Instant::now();
        for enabled in [false, true] {
            let mut c = short_candidate(enabled);
            assert_eq!(c.choose(1, 5, 4, 50, false, false, now).steps, 2);
            assert_eq!(c.choose(1, 9, 4, 50, false, false, now).steps, 2);
            let recover = c.choose(1, 10, 4, 50, false, false, now);
            assert_eq!(recover.steps, if enabled { 3 } else { 2 });
            assert_eq!(
                recover.pick,
                if enabled { Pick::Repay } else { Pick::Nominal }
            );
            assert_eq!(
                c.choose(1, 14, 4, 50, false, false, now).steps,
                if enabled { 4 } else { 2 }
            );
            assert_eq!(c.choose(1, 10, 2, 2, false, false, now).steps, 2);
            assert_eq!(c.choose(1, 10, 4, 50, true, false, now).pick, Pick::Barrier);
            assert_eq!(c.choose(1, 10, 4, 50, false, true, now).pick, Pick::Command);
            c.speed(Speed::PAUSED);
            assert_eq!(c.choose(1, 10, 4, 50, false, false, now).pick, Pick::Paused);
            c.speed(Speed(400));
            c.set_batch_period(crate::interval::BASE);
            assert_eq!(c.choose(1, 20, 16, 50, false, false, now).steps, 4);
        }
    }

    #[test]
    fn short_recovery_amortizes_service_overhead_without_waiting_for_far_backlog() {
        // Capacity model, not a renderer simulation. Native work costs
        // 35 ms/update plus 40 ms/call of copy/readiness overhead. A
        // two-update batch cannot sustain 20 updates/s even at zero wait.
        // Three alone only break even at checkpoint cuts; four under
        // sustained pressure recover. Quantize to 60 Hz presentation frames.
        let run = |recovery| {
            let mut c = short_candidate(recovery);
            let mut trim = crate::interval::Trim::default();
            if recovery {
                trim.bound_recovery();
            }
            let start = Instant::now();
            let mut ms = 0.0_f64;
            let mut next = 1_u64;
            let mut far_calls = 0;
            let mut last_backlog = 0;
            while ms < 240_000.0 {
                let released = (ms / 50.0).floor() as u64 + 4;
                let checkpoint = (50 - next % 50) % 50 + 1;
                let cap = u32::try_from(checkpoint.min(4)).unwrap();
                let choice = c.choose(
                    next,
                    released,
                    cap,
                    checkpoint,
                    false,
                    false,
                    start + Duration::from_secs_f64(ms / 1000.0),
                );
                assert!(choice.steps > 0 && choice.steps <= cap);
                next += u64::from(choice.steps);
                assert!(next <= released + 1, "ran unreleased updates");
                last_backlog = released + 1 - next;
                far_calls += usize::from(choice.pick == Pick::Far);
                let shown_nominal = if choice.pick == Pick::Repay {
                    choice.steps.max(2)
                } else {
                    2
                };
                if matches!(choice.pick, Pick::Nominal | Pick::Repay) {
                    trim.update(last_backlog, shown_nominal);
                } else {
                    trim.relax();
                }
                let interval = f64::from(crate::interval::interval_us(
                    choice.steps,
                    shown_nominal,
                    50_000,
                    trim.value(),
                    false,
                )) / 1000.0;
                let service = 35.0 * f64::from(choice.steps) + 40.0;
                ms += (service.max(interval) / (1000.0 / 60.0)).ceil() * (1000.0 / 60.0);
            }
            (last_backlog, far_calls, (next - 1) as f64 / (ms / 1000.0))
        };
        let before = run(false);
        let after = run(true);
        assert!(
            before.0 >= 25 && before.1 > 0,
            "model must reproduce the backlog: {before:?}"
        );
        assert!(after.0 <= 16 && after.1 == 0, "{after:?}");
        assert!(after.2 >= 19.9, "did not sustain room rate: {after:?}");
    }

    #[test]
    fn five_update_batches_have_capacity_without_shortening_the_native_step() {
        // Feasibility model only: the next worker batch must finish within
        // the PREVIOUS buffer's presentation interval. Include 40 ms of
        // native copy/readiness work, 35 ms/update and 60 Hz frame boundaries.
        // This does not stand in for captured vehicle motion in the game.
        let run = |period_ms| {
            let mut cadence = short_candidate(false);
            cadence.set_batch_period(Duration::from_millis(period_ms));
            let nominal = cadence.fixed_nominal();
            let mut trim = crate::interval::Trim::default();
            let start = Instant::now();
            let (mut ms, mut previous_interval) = (0.0_f64, 200.0_f64);
            let (mut next, mut missed, mut calls) = (1_u64, 0, 0);
            let mut backlog = 0;
            while ms < 240_000.0 {
                let released = (ms / 50.0).floor() as u64 + 6;
                let checkpoint = (50 - next % 50) % 50 + 1;
                let cap = u32::try_from(checkpoint.min(16)).unwrap();
                let choice = cadence.choose(
                    next,
                    released,
                    cap,
                    checkpoint,
                    false,
                    false,
                    start + Duration::from_secs_f64(ms / 1000.0),
                );
                next += u64::from(choice.steps);
                assert!(next <= released + 1, "never run unreleased updates");
                assert!(u64::from(choice.steps) <= checkpoint);
                backlog = released + 1 - next;
                let service = 35.0 * f64::from(choice.steps) + 40.0;
                if ms > 5_000.0 {
                    calls += 1;
                    missed += usize::from(service > previous_interval + 16.667);
                }
                ms += (service.max(previous_interval) / (1000.0 / 60.0)).ceil() * (1000.0 / 60.0);
                if choice.pick == Pick::Nominal {
                    trim.update(backlog, nominal);
                } else {
                    trim.relax();
                }
                previous_interval = f64::from(crate::interval::interval_us(
                    choice.steps,
                    nominal,
                    50_000,
                    trim.value(),
                    false,
                )) / 1000.0;
            }
            (
                backlog,
                missed as f64 / calls as f64,
                (next - 1) as f64 / (ms / 1000.0),
            )
        };
        let baseline = run(200);
        let candidate = run(250);
        assert!(candidate.0 <= 16, "backlog: {candidate:?}");
        assert!(candidate.2 >= 19.9, "lost room speed: {candidate:?}");
        assert!(
            candidate.1 < baseline.1,
            "no modeled benefit: {baseline:?} -> {candidate:?}"
        );
        eprintln!(
            "200 ms: {baseline:?}; 250 ms: {candidate:?} (backlog, missed fraction, updates/s)"
        );
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

#[cfg(test)]
mod catch_up_tests {
    use super::*;

    fn calls(c: &mut CatchUp, start: Instant, seconds: f64, per_call: u32, period: f64) -> Instant {
        let mut t = 0.0;
        while t < seconds {
            c.note(start + Duration::from_secs_f64(t), per_call);
            t += period;
        }
        start + Duration::from_secs_f64(seconds)
    }

    #[test]
    fn a_game_at_the_rooms_pace_is_not_catching_up() {
        let mut c = CatchUp::new();
        let start = Instant::now();
        // 4x: 20 steps a second, four a call every 200 ms.
        let now = calls(&mut c, start, 10.0, 4, 0.2);
        assert!(!c.catching_up(now, 20.0));
        // A short freeze's backlog repaid in one call of 16 does not count.
        // (A longer one may for a few seconds; that only lets a call run
        // more of what is released, and a repaid backlog leaves nothing.)
        c.note(now, 16);
        assert!(!c.catching_up(now, 20.0));
    }

    #[test]
    fn a_game_replaying_history_is_catching_up_until_it_is_back() {
        let mut c = CatchUp::new();
        let start = Instant::now();
        // 16 steps every 430 ms against a 4x room: 37 a second.
        let now = calls(&mut c, start, 6.0, 16, 0.43);
        assert!(c.catching_up(now, 20.0));
        assert!(!c.catching_up(now, 0.0), "paused: not catching up");
        // Back at the frontier: the room's pace again.
        let now = calls(&mut c, now, 6.0, 4, 0.2);
        assert!(!c.catching_up(now, 20.0));
    }

    #[test]
    fn a_short_window_never_counts() {
        let mut c = CatchUp::new();
        let start = Instant::now();
        let now = calls(&mut c, start, 2.0, 16, 0.2);
        assert!(
            !c.catching_up(now, 5.0),
            "two seconds is not enough to tell"
        );
        c.reset();
        assert!(!c.catching_up(now, 5.0));
    }
}

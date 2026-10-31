//! Optional presentation floor learned from native preparation + worker time.
//! A recovery deadline shorter than that work creates another visible hold.
//! This changes presentation timing, never simulation values or action order.

pub const ENV: &str = "TPF3MP_HOOK_READY_BUDGET";
const SAMPLES: usize = 16;
const WARMUP: usize = 4;
const MARGIN_US: u32 = 16_667;

/// A count-specific, fixed-size rolling window. Keep outliers in the window;
/// its upper quartile avoids turning a single industry/save stall into a
/// permanent low simulation rate. The actual stall is still logged normally.
#[derive(Debug, Clone, Copy, Default)]
struct Costs {
    values: [u32; SAMPLES],
    len: usize,
    cursor: usize,
}

impl Costs {
    fn add(&mut self, us: u32) {
        self.values[self.cursor] = us;
        self.cursor = (self.cursor + 1) % SAMPLES;
        self.len = (self.len + 1).min(SAMPLES);
    }

    fn budget(&self) -> Option<u32> {
        if self.len < WARMUP {
            return None;
        }
        let mut sorted = self.values;
        sorted[..self.len].sort_unstable();
        Some(sorted[(self.len * 3).div_ceil(4) - 1].saturating_add(MARGIN_US))
    }
}

/// Only samples in the same native world, room epoch and accepted pace mix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub world: usize,
    pub epoch: u64,
    pub step_us: u32,
    pub nominal: u32,
}

#[derive(Debug, Default)]
pub struct ReadyBudget {
    enabled: bool,
    key: Option<Key>,
    last_swap_us: Option<u64>,
    costs: [Costs; 17],
    pub last_floor_us: u32,
}

impl ReadyBudget {
    pub const fn new() -> Self {
        Self {
            enabled: false,
            key: None,
            last_swap_us: None,
            costs: [Costs {
                values: [0; SAMPLES],
                len: 0,
                cursor: 0,
            }; 17],
            last_floor_us: 0,
        }
    }

    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn reset(&mut self) {
        let enabled = self.enabled;
        *self = Self::new();
        self.enabled = enabled;
    }

    /// `started` and `finished` come from the exposed buffer's worker record,
    /// in the same monotonic clock as `now`. Work before the last swap is
    /// ambiguous (or belongs to a replaced buffer); never train on it.
    pub fn after_swap(
        &mut self,
        key: Key,
        now: u64,
        count: u32,
        started: u64,
        finished: u64,
        requested_us: u32,
    ) -> u32 {
        self.last_floor_us = 0;
        if !self.enabled {
            return requested_us;
        }
        if self.key != Some(key) {
            self.reset();
            self.key = Some(key);
        }
        // Emergency catch-up intentionally compresses large batches. Giving
        // those a full service-time interval prolongs catch-up and creates
        // more undersized checkpoint batches; the closed-loop model catches it.
        if count == 0 || count > key.nominal || count as usize >= self.costs.len() {
            self.last_swap_us = None;
            return requested_us;
        }
        let previous = self.last_swap_us.replace(now);
        if let Some(previous) = previous
            && started >= previous
            && finished >= started
            && finished <= now
            && let Ok(service) = u32::try_from(finished - previous)
            && service > 0
        {
            self.costs[count as usize].add(service);
        }
        let Some(budget) = self.costs[count as usize].budget() else {
            return requested_us;
        };
        // Never create a permanently slower-than-room deadline in response
        // to overload. Preserve reserve rebuilding's already longer interval.
        let floor = budget.min(count.saturating_mul(key.step_us));
        self.last_floor_us = floor;
        requested_us.max(floor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn measured_floor_clears_backlog_without_repeated_over_capacity_deadlines() {
        // Same ordered native workload for both policies. Include expensive
        // operations at the same simulation steps, not at different wall times.
        let run = |enabled, cap, bounded_feedback, frame_us, speed, initial| {
            let mut budget = ReadyBudget::new();
            if enabled {
                budget.enable();
            }
            let mut cadence = crate::cadence::Cadence::new(true);
            cadence.begin(5);
            cadence.speed(tpf3mp_proto::Speed(speed));
            cadence.set_fixed(true);
            cadence.set_batch_period(Duration::from_millis(250));
            let nominal = cadence.fixed_nominal();
            let step_us = cadence.step_us();
            let sample_key = Key {
                nominal,
                step_us,
                ..key()
            };
            let mut trim = crate::interval::Trim::default();
            let start = Instant::now();
            let (mut now, mut interval, mut next) = (0_u64, 250_000_u32, 1_u64);
            let (mut misses, mut backlog) = (0, 0);
            while now < 240_000_000 {
                let released = now / u64::from(step_us) + initial;
                let cp = (50 - next % 50) % 50 + 1;
                let choice = cadence.choose(
                    next,
                    released,
                    cp.min(cap) as u32,
                    cp,
                    false,
                    false,
                    start + Duration::from_micros(now),
                );
                let end = next + u64::from(choice.steps);
                assert!(end <= released + 1);
                let slow_step = ((next - 1) / 1000 + 1) * 1000;
                let slow = end > slow_step;
                let service =
                    u64::from(choice.steps) * 35_000 + 20_000 + if slow { 1_300_000 } else { 0 };
                let old_now = now;
                now += service.max(u64::from(interval)).div_ceil(frame_us) * frame_us;
                if old_now > 5_000_000 && !slow && service > u64::from(interval) + 50_000 {
                    misses += 1;
                }
                next = end;
                backlog = released + 1 - next;
                if if bounded_feedback {
                    choice.steady_interval(nominal, cap as u32)
                } else {
                    choice.pick == crate::cadence::Pick::Nominal
                } {
                    trim.update(backlog, nominal);
                } else {
                    trim.relax();
                }
                let requested = crate::interval::interval_us(
                    choice.steps,
                    nominal,
                    step_us,
                    trim.value(),
                    false,
                );
                interval = budget.after_swap(
                    sample_key,
                    now,
                    choice.steps,
                    old_now + 20_000,
                    old_now + service,
                    requested,
                );
            }
            (
                misses,
                backlog,
                (next - 1) as f64 / (now as f64 / 1_000_000.0),
            )
        };
        let baseline = run(false, 16, false, 16_667, 400, 6);
        let candidate = run(true, 16, false, 16_667, 400, 6);
        let capped_only = run(true, 5, false, 16_667, 400, 6);
        let bounded = run(true, 5, true, 16_667, 400, 6);
        let lower_fps = run(true, 5, true, 33_333, 400, 6);
        // A rejoin can still be far behind after its last ordered event.
        let twice = run(true, 5, true, 16_667, 200, 200);
        eprintln!("bounded at 2x {twice:?}");
        assert!(
            twice.1 <= 20 && twice.2 >= 9.95,
            "2x did not recover: {twice:?}"
        );
        for speed in [100, 300, 400] {
            let replay = run(true, 5, true, 16_667, speed, 200);
            let room_rate = f64::from(speed) / 20.0;
            assert!(
                replay.1 <= 20 && replay.2 >= room_rate * 0.995,
                "{speed}% did not repay its initial history: {replay:?}"
            );
        }
        eprintln!("bounded at 30 Hz {lower_fps:?}");
        assert!(
            lower_fps.1 <= 20 && lower_fps.2 >= 19.9,
            "30 Hz did not recover: {lower_fps:?}"
        );
        eprintln!("capped only {capped_only:?}; bounded feedback {bounded:?}");
        assert!(
            capped_only.1 > 20,
            "must reproduce uncleared capped backlog: {capped_only:?}"
        );
        assert!(
            bounded.0 < candidate.0,
            "secondary misses did not improve: {bounded:?}"
        );
        assert!(
            bounded.1 <= 20,
            "bounded recovery did not clear: {bounded:?}"
        );
        assert!(
            bounded.2 >= 19.9,
            "bounded recovery lost room speed: {bounded:?}"
        );
        assert!(
            baseline.0 > 0,
            "must reproduce repeated misses: {baseline:?}"
        );
        assert!(
            candidate.0 < baseline.0,
            "no improvement: {baseline:?} -> {candidate:?}"
        );
        assert!(candidate.1 <= 20, "backlog did not recover: {candidate:?}");
        assert!(candidate.2 >= 19.9, "lost room speed: {candidate:?}");
        eprintln!(
            "requested-only {baseline:?}; measured-floor {candidate:?} (misses, backlog, updates/s)"
        );
    }

    fn key() -> Key {
        Key {
            world: 8,
            epoch: 1,
            step_us: 50_000,
            nominal: 5,
        }
    }

    fn samples(b: &mut ReadyBudget, key: Key, count: u32, service: u64, n: usize) -> u32 {
        let mut now = b.last_swap_us.unwrap_or(0);
        let mut answer = 0;
        for _ in 0..n {
            answer = b.after_swap(
                key,
                now + 250_000,
                count,
                now + 15_000,
                now + service,
                100_000,
            );
            now += 250_000;
        }
        answer
    }

    #[test]
    fn measured_native_work_prevents_the_repeated_impossible_recovery_deadline() {
        let mut b = ReadyBudget::new();
        assert_eq!(
            samples(&mut b, key(), 5, 180_000, 30),
            100_000,
            "default unchanged"
        );
        b.enable();
        assert_eq!(
            samples(&mut b, key(), 5, 180_000, 4),
            100_000,
            "needs four paired samples"
        );
        assert_eq!(samples(&mut b, key(), 5, 180_000, 1), 196_667);
        assert_eq!(
            samples(&mut b, key(), 5, 240_000, 16),
            250_000,
            "never slower than room pace"
        );
    }

    #[test]
    fn counts_and_lifecycle_do_not_reuse_an_unrelated_workers_budget() {
        let mut b = ReadyBudget::new();
        b.enable();
        assert_eq!(samples(&mut b, key(), 5, 180_000, 20), 196_667);
        assert_eq!(samples(&mut b, key(), 2, 80_000, 1), 100_000);
        for changed in [
            Key { world: 16, ..key() },
            Key { epoch: 2, ..key() },
            Key {
                step_us: 100_000,
                ..key()
            },
            Key {
                nominal: 6,
                ..key()
            },
        ] {
            assert_eq!(samples(&mut b, changed, 5, 180_000, 1), 100_000);
            assert_eq!(b.last_floor_us, 0);
        }
        b.reset();
        assert_eq!(
            samples(&mut b, key(), 5, 180_000, 5),
            196_667,
            "reset keeps opt-in"
        );
    }

    #[test]
    fn emergency_batches_keep_their_compressed_interval_and_break_the_measurement_pair() {
        let mut b = ReadyBudget::new();
        b.enable();
        samples(&mut b, key(), 5, 180_000, 20);
        let t = b.last_swap_us.unwrap();
        assert_eq!(
            b.after_swap(key(), t + 600_000, 16, t + 20_000, t + 580_000, 400_000),
            400_000
        );
        assert!(b.last_swap_us.is_none());
        assert_eq!(b.last_floor_us, 0);
        assert_eq!(b.costs[16].len, 0);
    }

    #[test]
    fn a_single_long_native_stall_does_not_slow_every_following_batch() {
        let mut b = ReadyBudget::new();
        b.enable();
        samples(&mut b, key(), 5, 180_000, 20);
        let t = b.last_swap_us.unwrap();
        assert_eq!(
            b.after_swap(key(), t + 2_000_000, 5, t + 15_000, t + 1_800_000, 100_000),
            196_667
        );
        assert_eq!(
            b.after_swap(
                key(),
                t + 2_250_000,
                5,
                t + 2_015_000,
                t + 2_180_000,
                300_000
            ),
            300_000
        );
    }

    #[test]
    fn zero_swaps_and_invalid_worker_clocks_never_train_the_floor() {
        let mut b = ReadyBudget::new();
        b.enable();
        for i in 1..20 {
            let t = (i + 1) * 250_000;
            assert_eq!(
                b.after_swap(key(), t, 5, t - 250_001, t - 10_000, 100_000),
                100_000
            );
        }
        b.after_swap(key(), 6_000_000, 0, 0, 0, 50_000);
        assert_eq!(
            b.after_swap(key(), 6_250_000, 5, 6_015_000, 6_180_000, 100_000),
            100_000
        );
        assert_eq!(b.costs[5].len, 0);
    }
}

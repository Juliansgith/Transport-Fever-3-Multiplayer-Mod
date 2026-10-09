//! Opt-in pacing for the exact TF3 Steam 40420 profile.
//!
//! The room remains the authority for its speed and sealed steps. This module
//! only computes the native scheduler interval and the bounded release lead
//! needed to consume one sealed update at a time without draining the queue.
//! The native implementation is installed only when `TPF3MP_SMOOTH_PACING=1`
//! and both exact profile sites resolve.
//!
//! Prior art, independently adapted and re-verified for TF3: silver2127's
//! `tpf2-multiplayer/native/src/speedhook.cpp`, especially commit `77a4e75`
//! (fractional speed scales the simulation interval) and earlier commit
//! `222ca62` (count dithering). This code uses the 40420 TF3 scheduler sites
//! and preserves its sealed room-update sequence.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use tpf3mp_proto::Speed;

/// Set in the game's environment to opt into smooth native scheduler pacing.
pub const ENV: &str = "TPF3MP_SMOOTH_PACING";
/// Set to `2` with `TPF3MP_SMOOTH_PACING=1` to opt into batches at 3x/4x.
pub const BATCH_ENV: &str = "TPF3MP_SMOOTH_PACING_BATCH";

/// Normal operation runs one room update per successful native dispatch.
/// Backlog is not turned into a multi-update batch; the native timer has its
/// own bounded debt recovery policy.
/// Hard ceiling on completed updates charged to one outer native scheduler
/// callback if a later implementation permits several worker completions.
pub const MAX_UPDATES_PER_OUTER_STEP: u32 = 16;
/// Hard ceiling on native synchronization attempts in one outer CGame::Step.
/// The count bounds catch-up attempts without a multi-update batch. Since
/// Sync can return before asynchronous worker completion, it does not by
/// itself guarantee the room's target rate at low render rates.
pub const MAX_SYNC_CALLS: u32 = 16;
/// Soft limit for elapsed CGame::Step wall time and completed GameSim::Step
/// wall time observed within an outer call. These are not processor CPU time.
pub const FRAME_WALL_BUDGET: std::time::Duration = std::time::Duration::from_millis(25);
pub const RESERVE: std::time::Duration = std::time::Duration::from_millis(100);
pub const RECOVERY_DEBT: std::time::Duration = std::time::Duration::from_millis(200);

const ACTIVE_BIT: u64 = 1 << 48;
const CHECKPOINT_LIMITED_BIT: u64 = 1 << 63;
const LOOKAHEAD_CAPPED_BIT: u64 = 1 << 58;
const BATCH_TWO_BIT: u64 = 1 << 59;
const SEALED_AHEAD_SHIFT: u32 = 49;
// At the maximum supported 240 steps/s and 4x speed, the bounded lookahead
// is 289 releases, so nine bits leave flags in the same atomic snapshot.
const SEALED_AHEAD_MASK: u64 = (1 << 9) - 1;
pub const RECOVERY_RATE_INCREASE_PERCENT: u32 = 10;
pub const RECOVERY_SLEW_PERCENT: u32 = 1;
static INSTALLED: AtomicBool = AtomicBool::new(false);
static SNAPSHOT: AtomicU64 = AtomicU64::new(0);
static COMPLETED_UPDATES: AtomicU64 = AtomicU64::new(0);
static COMPLETED_STEP_WALL_NANOS: AtomicU64 = AtomicU64::new(0);
static ACTIVE_WORKER_CALLS: AtomicU64 = AtomicU64::new(0);
static SEALED_LOOKAHEAD_HIGH_WATER: AtomicU64 = AtomicU64::new(0);
static LOOKAHEAD_CAP_HITS: AtomicU64 = AtomicU64::new(0);
static TRACE: AtomicBool = AtomicBool::new(false);
static SCHEDULER_DEBT: AtomicU64 = AtomicU64::new(0);
static HEALTHY: AtomicBool = AtomicBool::new(true);
static COMPLETED_BATCH_UPDATES: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompletionTotals {
    pub updates: u64,
    pub step_wall_nanos: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompletionDelta {
    pub updates: u64,
    pub step_wall_nanos: u64,
}

/// Differences cumulative worker measurements without assuming that a
/// completion happened inside the native frame or Sync call being sampled.
pub fn completion_delta(current: CompletionTotals, previous: CompletionTotals) -> CompletionDelta {
    CompletionDelta {
        updates: current.updates.saturating_sub(previous.updates),
        step_wall_nanos: current
            .step_wall_nanos
            .saturating_sub(previous.step_wall_nanos),
    }
}

fn atomic_saturating_add(counter: &AtomicU64, amount: u64) {
    let _ = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
        Some(value.saturating_add(amount))
    });
}

fn atomic_max(counter: &AtomicU64, value: u64) {
    let _ = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |before| {
        (value > before).then_some(value)
    });
}

/// Records a completed room update on the simulation thread. These counters
/// are cumulative for the process lifetime and are never gated or reset by a
/// CGame frame; the main thread samples their deltas asynchronously.
pub fn record_worker_completion(updates: u32, step_wall_nanos: u64) {
    if updates == 0 {
        return;
    }
    atomic_saturating_add(&COMPLETED_UPDATES, u64::from(updates));
    atomic_saturating_add(&COMPLETED_STEP_WALL_NANOS, step_wall_nanos);
}

pub fn worker_call_started() {
    atomic_saturating_add(&ACTIVE_WORKER_CALLS, 1);
}

pub fn worker_call_finished() {
    let _ = ACTIVE_WORKER_CALLS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
        Some(active.saturating_sub(1))
    });
}

pub fn active_worker_calls() -> u32 {
    u32::try_from(ACTIVE_WORKER_CALLS.load(Ordering::Acquire)).unwrap_or(u32::MAX)
}

pub fn completion_totals() -> CompletionTotals {
    CompletionTotals {
        updates: COMPLETED_UPDATES.load(Ordering::Acquire),
        step_wall_nanos: COMPLETED_STEP_WALL_NANOS.load(Ordering::Acquire),
    }
}

/// Records the bounded sealed-release lookahead observed by the room driver.
/// Reaching the lookahead limit means the true queue depth may be higher.
pub fn record_sealed_lookahead(ahead: u32, limit: u32) {
    atomic_max(&SEALED_LOOKAHEAD_HIGH_WATER, u64::from(ahead));
    if limit > 0 && ahead >= limit {
        atomic_saturating_add(&LOOKAHEAD_CAP_HITS, 1);
    }
}

/// One atomic publication from the simulation-step driver to the native
/// scheduler. A single store keeps the rate fields from different room
/// transitions from being combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub active: bool,
    pub steps_per_second: u16,
    pub speed: Speed,
    /// The previous non-paused authoritative room speed. A pause holds the
    /// scheduler at this pace while already-released steps drain.
    pub previous_speed: Speed,
    /// Sealed releases observed by the step driver, capped at the bounded
    /// lookahead horizon. This is a slightly delayed queue estimate.
    pub sealed_ahead: u16,
    /// The lookahead reached its fixed cap, so `sealed_ahead` is a lower bound.
    pub lookahead_capped: bool,
    /// The lookahead ended at a checkpoint, so a smaller count may not mean
    /// the rest of the sealed backlog has drained.
    pub checkpoint_limited: bool,
    /// Opt-in two-update scheduling for room speeds above 2x.
    pub batch_two: bool,
}

impl Snapshot {
    fn encode(self) -> u64 {
        u64::from(self.steps_per_second)
            | (u64::from(self.speed.0) << 16)
            | (u64::from(self.previous_speed.0) << 32)
            | ((u64::from(self.sealed_ahead).min(SEALED_AHEAD_MASK)) << SEALED_AHEAD_SHIFT)
            | (if self.lookahead_capped {
                LOOKAHEAD_CAPPED_BIT
            } else {
                0
            })
            | (if self.batch_two { BATCH_TWO_BIT } else { 0 })
            | (if self.active { ACTIVE_BIT } else { 0 })
            | (if self.checkpoint_limited {
                CHECKPOINT_LIMITED_BIT
            } else {
                0
            })
    }

    fn decode(encoded: u64) -> Self {
        Self {
            active: encoded & ACTIVE_BIT != 0,
            steps_per_second: encoded as u16,
            speed: Speed((encoded >> 16) as u16),
            previous_speed: Speed((encoded >> 32) as u16),
            sealed_ahead: u16::try_from((encoded >> SEALED_AHEAD_SHIFT) & SEALED_AHEAD_MASK)
                .unwrap_or(u16::MAX),
            lookahead_capped: encoded & LOOKAHEAD_CAPPED_BIT != 0,
            checkpoint_limited: encoded & CHECKPOINT_LIMITED_BIT != 0,
            batch_two: encoded & BATCH_TWO_BIT != 0,
        }
    }
}

/// Current nominal and applied native periods, including whether the
/// controller is using its bounded backlog headroom.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cadence {
    pub nominal_interval_micros: u32,
    pub interval_micros: u32,
    pub catching_up: bool,
}

/// Chooses the next native period. Enter catch-up at the existing bounded
/// lookahead horizon and leave only once the queue is back at the startup
/// lead. Each successful Sync from the previous outer call permits at most a
/// 1% nominal-period slew toward a 10% rate increase. Room-rate changes and
/// native intervals outside that recovery band rebase immediately; the owner
/// remaps phase whenever it installs the returned period.
pub fn next_cadence(
    snapshot: Snapshot,
    current: Cadence,
    previous_successful_syncs: u32,
) -> Option<Cadence> {
    let pace = plan(snapshot)?;
    let nominal = pace.interval_micros;
    let nominal_changed = current.nominal_interval_micros != nominal;
    let paused = snapshot.speed.is_paused();

    let mut catching_up = if nominal_changed {
        false
    } else {
        current.catching_up
    };
    if paused {
        catching_up = false;
    } else if u32::from(snapshot.sealed_ahead) >= pace.lookahead_limit
        || (snapshot.lookahead_capped
            && u32::from(snapshot.sealed_ahead) > startup_ahead(pace, snapshot))
    {
        catching_up = true;
    } else if !snapshot.checkpoint_limited
        && u32::from(snapshot.sealed_ahead) <= startup_ahead(pace, snapshot)
    {
        catching_up = false;
    }

    let faster = nominal
        .saturating_mul(100)
        .div_ceil(100 + RECOVERY_RATE_INCREASE_PERCENT)
        .max(1);
    let target = if catching_up { faster } else { nominal };
    let unexpected_period = current.interval_micros != 0
        && (current.interval_micros > nominal || current.interval_micros < faster);
    let rebase_period = nominal_changed || current.interval_micros == 0 || unexpected_period;
    let mut interval = if rebase_period {
        nominal
    } else {
        current.interval_micros
    };
    if paused {
        // Pause holds the previous authoritative room period, draining its
        // already-sealed work without carrying catch-up speed into the hold.
        interval = nominal;
    } else if !rebase_period && previous_successful_syncs > 0 {
        let one_percent = (nominal / 100).max(1);
        let maximum_change = one_percent.saturating_mul(previous_successful_syncs);
        if interval < target {
            interval = interval.saturating_add(maximum_change).min(target);
        } else if interval > target {
            interval = interval.saturating_sub(maximum_change).max(target);
        }
    }

    Some(Cadence {
        nominal_interval_micros: nominal,
        interval_micros: interval,
        catching_up,
    })
}

/// Startup lead plus one update to consume now. The lead left after that
/// update is at least 100 ms at the room's effective rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub interval_micros: u32,
    pub reserve_steps: u32,
    pub startup_ahead: u32,
    /// Maximum sealed-release lookahead sampled for bounded backlog telemetry.
    pub lookahead_limit: u32,
}

/// Maximum sealed updates one native callback may consume under the opt-in
/// batch mode. Pauses and speeds up to 2x always retain one-update cadence.
pub fn max_batch_updates(snapshot: Snapshot) -> u32 {
    if snapshot.batch_two && !snapshot.speed.is_paused() && snapshot.speed.0 > 200 {
        2
    } else {
        1
    }
}

/// Releases needed before consuming the selected callback while leaving the
/// full 100 ms lead behind it.
pub fn startup_ahead(pace: Plan, snapshot: Snapshot) -> u32 {
    pace.reserve_steps
        .saturating_add(max_batch_updates(snapshot))
}

/// Scales an exact completed batch to its presentation interval. The nominal
/// interval is derived from room steps/s and authoritative speed; the
/// effective one-update interval applies the bounded recovery factor.
pub fn presentation_interval_micros(
    snapshot: Snapshot,
    completed_updates: u32,
    effective_unit_interval_micros: u32,
) -> Option<u32> {
    if !(1..=2).contains(&completed_updates) {
        return None;
    }
    let pace = plan(snapshot)?;
    let minimum_unit = pace
        .interval_micros
        .saturating_mul(100)
        .div_ceil(100 + RECOVERY_RATE_INCREASE_PERCENT)
        .max(1);
    if !(minimum_unit..=pace.interval_micros).contains(&effective_unit_interval_micros) {
        return None;
    }
    let speed = if snapshot.speed.is_paused() {
        snapshot.previous_speed.0
    } else {
        snapshot.speed.0
    };
    let rate_units = u64::from(snapshot.steps_per_second) * u64::from(speed);
    let numerator = 100_000_000_u64.checked_mul(u64::from(completed_updates))?;
    let nominal_period = numerator
        .checked_add(rate_units / 2)?
        .checked_div(rate_units)?;
    let scaled_period = nominal_period
        .checked_mul(u64::from(effective_unit_interval_micros))?
        .checked_add(u64::from(pace.interval_micros) / 2)?
        .checked_div(u64::from(pace.interval_micros))?;
    u32::try_from(scaled_period)
        .ok()
        .filter(|period| *period > 0)
}

/// A clock adjustment applied after one native Sync has completed. `native_last`
/// is the value CGame::Step cached before Sync and writes after a true return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncClockAdjustment {
    pub now: i64,
    pub native_last: i64,
    pub dropped_intervals: u64,
    pub remainder: i64,
}

/// Whether the next successful native Sync must end the current CGame::Step
/// catch-up loop. The worker only publishes counts and elapsed wall time;
/// native timer writes happen later on the owning CGame thread.
pub fn stop_after_sync(
    sync_calls: u32,
    completed_updates: u32,
    completed_worker_wall_nanos: u64,
    outer_wall_nanos: u64,
) -> bool {
    sync_calls >= MAX_SYNC_CALLS
        || completed_updates >= MAX_UPDATES_PER_OUTER_STEP
        || completed_worker_wall_nanos
            >= u64::try_from(FRAME_WALL_BUDGET.as_nanos()).unwrap_or(u64::MAX)
        || outer_wall_nanos >= u64::try_from(FRAME_WALL_BUDGET.as_nanos()).unwrap_or(u64::MAX)
}

/// Computes the native timer change needed after a successful Sync. CGame::Step
/// caches `last + pre_sync_interval` before Sync and commits that cached value
/// only when Sync returns true. We preserve that exact `native_last`, keeping
/// the fractional phase measured with the post-Sync interval.
pub fn clamp_after_sync(
    now: i64,
    last_before_sync: i64,
    pre_sync_interval: i32,
    post_sync_interval: i32,
    sync_succeeded: bool,
    stop_after_sync: bool,
) -> Option<SyncClockAdjustment> {
    if !sync_succeeded || !stop_after_sync || pre_sync_interval <= 0 || post_sync_interval <= 0 {
        return None;
    }
    let native_last = i128::from(last_before_sync) + i128::from(pre_sync_interval);
    let overdue = i128::from(now) - native_last;
    if overdue < i128::from(post_sync_interval) {
        return None;
    }
    let dropped_intervals = overdue / i128::from(post_sync_interval);
    let remainder = overdue % i128::from(post_sync_interval);
    let adjusted_now = i64::try_from(native_last + remainder).ok()?;
    Some(SyncClockAdjustment {
        now: adjusted_now,
        native_last: i64::try_from(native_last).ok()?,
        dropped_intervals: u64::try_from(dropped_intervals).unwrap_or(u64::MAX),
        remainder: i64::try_from(remainder).ok()?,
    })
}

/// Returns a native scheduler plan only for the prototype's supported room
/// rates: 1x through 4x, or a pause at the last non-paused rate; room step
/// rates remain the protocol's validated 1..=240.
pub fn plan(snapshot: Snapshot) -> Option<Plan> {
    if !snapshot.active || !(1..=240).contains(&snapshot.steps_per_second) {
        return None;
    }
    let speed = if snapshot.speed.is_paused() {
        snapshot.previous_speed.0
    } else {
        snapshot.speed.0
    };
    if !(100..=400).contains(&speed) || speed % 100 != 0 {
        return None;
    }

    let rate_units = u64::from(snapshot.steps_per_second) * u64::from(speed);
    let interval_micros = u32::try_from((100_000_000 + rate_units / 2) / rate_units).ok()?;
    if interval_micros == 0 {
        return None;
    }

    // reserve = ceil((steps/s * speed/100) * 100 ms)
    let reserve_steps = u32::try_from(rate_units.div_ceil(1_000)).ok()?;
    let startup_ahead = reserve_steps.checked_add(1)?;
    // Keep the lookahead bounded to the retained lead plus 200 ms of releases.
    // This is an observation horizon only; the driver never drains a backlog
    // as a multi-update batch.
    let lookahead_limit =
        startup_ahead.checked_add(u32::try_from(rate_units.div_ceil(500)).ok()?.max(1))?;

    Some(Plan {
        interval_micros,
        reserve_steps,
        startup_ahead,
        lookahead_limit,
    })
}

/// Chooses the sealed update count for one simulation callback. A backlog
/// never becomes a multi-update burst; the outer native cadence and the
/// sealed release queue determine when the next single update may run.
pub fn batch_limit(
    ahead: u32,
    checkpoint_distance: u32,
    plan: Plan,
    primed: bool,
    paused: bool,
) -> u32 {
    batch_limit_with_max(ahead, checkpoint_distance, plan, primed, paused, 1)
}

/// Chooses a bounded callback batch without crossing the checkpoint. The
/// priming target includes the whole selected batch so the retained reserve
/// remains after its last update.
pub fn batch_limit_with_max(
    ahead: u32,
    checkpoint_distance: u32,
    plan: Plan,
    primed: bool,
    paused: bool,
    maximum_updates: u32,
) -> u32 {
    if ahead == 0 || checkpoint_distance == 0 {
        return 0;
    }
    let maximum_updates = maximum_updates.clamp(1, 2);
    if paused {
        return ahead.min(checkpoint_distance).min(1);
    }

    let target_ahead = plan.reserve_steps.saturating_add(maximum_updates);
    let checkpoint_limited_prime =
        checkpoint_distance < target_ahead && ahead == checkpoint_distance;
    if !primed && ahead < target_ahead && !checkpoint_limited_prime {
        return 0;
    }
    ahead.min(checkpoint_distance).min(maximum_updates)
}

/// Whether a short queue is capped exactly at a checkpoint and can progress
/// safely by one update rather than waiting for releases beyond that cap.
pub fn checkpoint_limits_prime(ahead: u32, checkpoint_distance: u32, plan: Plan) -> bool {
    checkpoint_distance < plan.startup_ahead && ahead == checkpoint_distance
}

pub fn checkpoint_limits_prime_with_max(
    ahead: u32,
    checkpoint_distance: u32,
    plan: Plan,
    maximum_updates: u32,
) -> bool {
    checkpoint_distance
        < plan
            .reserve_steps
            .saturating_add(maximum_updates.clamp(1, 2))
        && ahead == checkpoint_distance
}

/// Remaps the timestamp's phase when the scheduler interval changes. Whole
/// overdue intervals remain visible as scheduler debt; the fractional phase
/// is scaled with the new interval. Sealed simulation updates stay in the
/// bridge queue and are consumed one per GameSim callback.
pub fn remap_last(now: i64, last: i64, old_interval: i32, new_interval: i32) -> i64 {
    if old_interval <= 0 || new_interval <= 0 {
        return last;
    }
    let elapsed = i128::from(now) - i128::from(last);
    if elapsed <= 0 {
        return now;
    }
    let scaled = elapsed * i128::from(new_interval) / i128::from(old_interval);
    let remapped = i128::from(now) - scaled;
    i64::try_from(remapped).unwrap_or(if remapped < 0 { i64::MIN } else { i64::MAX })
}

/// Whether the user explicitly enabled the experimental native scheduler.
pub fn requested() -> bool {
    matches!(
        std::env::var(ENV).as_deref(),
        Ok("1" | "true" | "yes" | "on")
    )
}

/// Whether the separately opt-in 2-update experiment was requested together
/// with smooth pacing. Native hooks still additionally require exact 40420.
pub fn batch_two_requested() -> bool {
    batch_two_requested_for(requested(), std::env::var(BATCH_ENV).ok().as_deref())
}

pub fn batch_two_requested_for(smooth_pacing: bool, batch_value: Option<&str>) -> bool {
    smooth_pacing && batch_value == Some("2")
}

/// Publishes the completed room batch before the native worker marks itself
/// ready. The owning CGame thread samples this only in ComputeFrameTime.
pub fn publish_completed_batch(updates: u32) {
    COMPLETED_BATCH_UPDATES.store(updates, Ordering::Release);
}

fn take_completed_batch() -> u32 {
    COMPLETED_BATCH_UPDATES.swap(0, Ordering::AcqRel)
}

/// Whether exact native pacing hooks were installed for this process.
pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// Records that the exact-build scheduler hooks were installed. Until this
/// is true the regular step driver retains its legacy behavior.
pub(crate) fn set_installed(installed: bool) {
    INSTALLED.store(installed, Ordering::Release);
    if installed {
        HEALTHY.store(true, Ordering::Release);
    }
    if !installed {
        SNAPSHOT.store(0, Ordering::Release);
        COMPLETED_BATCH_UPDATES.store(0, Ordering::Release);
    }
}

/// Whether runtime layout checks still permit using the native scheduler.
pub fn runtime_healthy() -> bool {
    HEALTHY.load(Ordering::Acquire)
}

/// Fails the optional prototype closed after an unexpected runtime layout.
pub(crate) fn fail_runtime() {
    HEALTHY.store(false, Ordering::Release);
    set_installed(false);
}

/// Publishes the room pace from the game-step driver. The legacy path does
/// not publish or read this snapshot.
pub fn publish(snapshot: Snapshot) {
    if INSTALLED.load(Ordering::Acquire) {
        SNAPSHOT.store(snapshot.encode(), Ordering::Release);
    }
}

/// Reads the complete room pacing snapshot on the native scheduler thread.
pub fn current() -> Snapshot {
    Snapshot::decode(SNAPSHOT.load(Ordering::Acquire))
}

pub(crate) fn set_trace_enabled(enabled: bool) {
    TRACE.store(enabled, Ordering::Release);
}

/// Takes the scheduler intervals the native hook shed after reaching its
/// frame budget. The room's sealed updates remain in the gate and are never
/// acknowledged by this counter.
pub fn take_scheduler_debt() -> u64 {
    SCHEDULER_DEBT.swap(0, Ordering::AcqRel)
}

/// Reads the current scheduler-debt counter without consuming it. Diagnostic
/// snapshots use this so recording a timeline cannot affect pacing reports.
pub fn scheduler_debt_pending() -> u64 {
    SCHEDULER_DEBT.load(Ordering::Acquire)
}

/// Whether the verified ComputeFrameTime wrapper may substitute the room
/// interval. Observe-only tracing and inactive snapshots always preserve the
/// original native result.
pub fn applied_interval(
    legacy_interval: i32,
    in_step: bool,
    pacing_overridden: bool,
    room_interval: i32,
) -> i32 {
    if in_step && pacing_overridden && room_interval > 0 {
        room_interval
    } else {
        legacy_interval
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) mod native {
    use std::{
        cell::RefCell,
        sync::{Mutex, atomic::Ordering},
        time::{Duration, Instant},
    };

    use super::{
        Cadence, CompletionTotals, LOOKAHEAD_CAP_HITS, SCHEDULER_DEBT, SEALED_LOOKAHEAD_HIGH_WATER,
        clamp_after_sync, completion_delta, completion_totals, next_cadence, plan,
        presentation_interval_micros, remap_last, stop_after_sync, take_completed_batch,
    };

    const DATA_OFFSET: usize = 0x1f0;
    const NOW_OFFSET: usize = 0x1a0;
    const LAST_OFFSET: usize = 0x1a8;
    const INTERVAL_OFFSET: usize = 0x218;
    const TELEMETRY_WINDOW: Duration = Duration::from_secs(10);

    #[derive(Default, Clone)]
    struct State {
        in_step: bool,
        cgame: usize,
        data: usize,
        interval: i32,
        reserve_steps: u32,
        legacy_interval: i32,
        overridden: bool,
        sync_calls: u32,
        successful_syncs: u32,
        compute_frame_calls: u32,
        nominal_interval_micros: u32,
        cadence_interval_micros: u32,
        presented_updates: u32,
        catching_up: bool,
        frame_started: Option<Instant>,
        sync_last_before: i64,
        sync_interval_before: i32,
        sync_snapshot_valid: bool,
        outer_completion_start: CompletionTotals,
        observed_completions: CompletionTotals,
        observed_lookahead_cap_hits: u64,
    }

    /// Binds cached cadence state to the native scheduler object that owns
    /// the fields. A new CGame or CGameData starts from its live native period
    /// and must not inherit catch-up state or successes from the old object.
    fn adopt_native_identity(
        state: &mut State,
        cgame: usize,
        data: usize,
        previous_successful_syncs: &mut u32,
    ) -> bool {
        let changed = state.cgame != cgame || state.data != data;
        if changed {
            state.interval = 0;
            state.legacy_interval = 0;
            state.overridden = false;
            state.nominal_interval_micros = 0;
            state.cadence_interval_micros = 0;
            state.presented_updates = 1;
            state.catching_up = false;
            state.successful_syncs = 0;
            state.sync_last_before = 0;
            state.sync_interval_before = 0;
            state.sync_snapshot_valid = false;
            *previous_successful_syncs = 0;
        }
        state.cgame = cgame;
        state.data = data;
        changed
    }

    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct FrameSample {
        pub active: bool,
        pub frame_updates: u32,
        pub worker_step_wall_nanos: u64,
        pub sealed_lookahead: u32,
        pub lookahead_cap_hits: u64,
        pub active_worker_calls: u32,
        pub reserve_steps: u32,
        pub sync_calls: u32,
        pub compute_frame_calls: u32,
        pub scheduler_debt: u64,
        pub interval_micros: u32,
        pub phase_per_mille: u32,
    }

    /// Native CGame phase fields sampled only by the verified main-thread
    /// Step/Sync wrappers while an explicitly requested timeline is active.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct TraceFields {
        pub cgame: usize,
        pub data: usize,
        pub now: i64,
        pub last: i64,
        pub interval_micros: i32,
        pub alpha_milli: i64,
    }

    /// Reads the native phase state from the exact verified CGame main-thread
    /// hook path. Never call this from `GameSim::Step` or another worker.
    ///
    /// # Safety
    ///
    /// `this` must be the live CGame pointer passed to the verified 40420
    /// `CGame::Step` or its verified `CGame::Sync` callsite wrapper.
    pub unsafe fn trace_fields(this: usize) -> TraceFields {
        let mut fields = TraceFields {
            cgame: this,
            ..TraceFields::default()
        };
        if this == 0 {
            return fields;
        }
        // SAFETY: exact profile-gated CGame Step/Sync hook ABI; same field
        // offset used by begin_step/before_sync on the owner thread.
        let data = unsafe { *((this + DATA_OFFSET) as *const usize) };
        fields.data = data;
        if data == 0 || data % std::mem::align_of::<i64>() != 0 {
            return fields;
        }
        // SAFETY: this is the validated 40420 CGameData layout, read from the
        // same main-thread call path that owns these phase fields.
        fields.now = unsafe { *((data + NOW_OFFSET) as *const i64) };
        // SAFETY: as above.
        fields.last = unsafe { *((data + LAST_OFFSET) as *const i64) };
        // SAFETY: as above.
        fields.interval_micros = unsafe { *((data + INTERVAL_OFFSET) as *const i32) };
        if fields.interval_micros > 0 {
            let phase = (i128::from(fields.now) - i128::from(fields.last)) * 1_000
                / i128::from(fields.interval_micros);
            fields.alpha_milli =
                i64::try_from(phase).unwrap_or(if phase < 0 { i64::MIN } else { i64::MAX });
        }
        fields
    }

    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct TelemetryReport {
        pub window_millis: u64,
        pub outer_steps: u64,
        pub room_updates: u64,
        pub worker_step_wall_micros: u64,
        pub average_worker_update_wall_micros: u64,
        pub maximum_sealed_lookahead: u64,
        pub lookahead_cap_hits: u64,
        pub maximum_active_worker_calls: u32,
        pub maximum_reserve_steps: u32,
        pub sync_calls: u64,
        pub compute_frame_calls: u64,
        pub scheduler_debt: u64,
        pub average_outer_wall_micros: u64,
        pub maximum_outer_wall_micros: u64,
        pub minimum_period_micros: u64,
        pub maximum_period_micros: u64,
        pub minimum_phase_per_mille: u32,
        pub maximum_phase_per_mille: u32,
    }

    #[derive(Debug)]
    struct Window {
        started: Instant,
        outer_steps: u64,
        room_updates: u64,
        worker_step_wall_micros: u64,
        maximum_sealed_lookahead: u64,
        lookahead_cap_hits: u64,
        maximum_active_worker_calls: u32,
        maximum_reserve_steps: u32,
        sync_calls: u64,
        compute_frame_calls: u64,
        scheduler_debt: u64,
        step_micros: u64,
        maximum_step_micros: u64,
        minimum_period_micros: u64,
        maximum_period_micros: u64,
        minimum_phase_per_mille: u32,
        maximum_phase_per_mille: u32,
    }

    impl Window {
        fn new(now: Instant) -> Self {
            Self {
                started: now,
                outer_steps: 0,
                room_updates: 0,
                worker_step_wall_micros: 0,
                maximum_sealed_lookahead: 0,
                lookahead_cap_hits: 0,
                maximum_active_worker_calls: 0,
                maximum_reserve_steps: 0,
                sync_calls: 0,
                compute_frame_calls: 0,
                scheduler_debt: 0,
                step_micros: 0,
                maximum_step_micros: 0,
                minimum_period_micros: u64::MAX,
                maximum_period_micros: 0,
                minimum_phase_per_mille: u32::MAX,
                maximum_phase_per_mille: 0,
            }
        }

        fn report(&self, elapsed: Duration) -> TelemetryReport {
            TelemetryReport {
                window_millis: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                outer_steps: self.outer_steps,
                room_updates: self.room_updates,
                worker_step_wall_micros: self.worker_step_wall_micros,
                average_worker_update_wall_micros: self
                    .worker_step_wall_micros
                    .checked_div(self.room_updates)
                    .unwrap_or(0),
                maximum_sealed_lookahead: self.maximum_sealed_lookahead,
                lookahead_cap_hits: self.lookahead_cap_hits,
                maximum_active_worker_calls: self.maximum_active_worker_calls,
                maximum_reserve_steps: self.maximum_reserve_steps,
                sync_calls: self.sync_calls,
                compute_frame_calls: self.compute_frame_calls,
                scheduler_debt: self.scheduler_debt,
                average_outer_wall_micros: self
                    .step_micros
                    .checked_div(self.outer_steps)
                    .unwrap_or(0),
                maximum_outer_wall_micros: self.maximum_step_micros,
                minimum_period_micros: if self.minimum_period_micros == u64::MAX {
                    0
                } else {
                    self.minimum_period_micros
                },
                maximum_period_micros: self.maximum_period_micros,
                minimum_phase_per_mille: if self.minimum_phase_per_mille == u32::MAX {
                    0
                } else {
                    self.minimum_phase_per_mille
                },
                maximum_phase_per_mille: self.maximum_phase_per_mille,
            }
        }
    }

    static TELEMETRY: Mutex<Option<Window>> = Mutex::new(None);

    thread_local! {
        static STATE: RefCell<State> = RefCell::new(State::default());
    }

    /// Begins a main-thread `CGame::Step` call. Native phase fields stay on
    /// this thread. Worker completions are cumulative atomics and may arrive
    /// before, during, or after this outer call.
    pub unsafe fn begin_step(this: usize) {
        // SAFETY: the hook supplies the verified CGame pointer and this
        // helper reads its scheduler block only on the owner thread.
        unsafe { begin_step_with_snapshot(this, super::current()) };
    }

    unsafe fn begin_step_with_snapshot(this: usize, snapshot: super::Snapshot) {
        let target_pace = plan(snapshot);
        let mut state = STATE.with(|local| local.borrow().clone());
        let mut previous_successful_syncs = state.successful_syncs;
        state.frame_started = Some(Instant::now());
        state.sync_calls = 0;
        state.successful_syncs = 0;
        state.compute_frame_calls = 0;
        state.sync_snapshot_valid = false;
        state.outer_completion_start = completion_totals();
        state.reserve_steps = target_pace.map_or(0, |pace| pace.reserve_steps);

        // Inactive menu/legacy frames need no native access unless the prior
        // frame left our interval installed and it must be restored.
        if target_pace.is_none() && !state.overridden {
            adopt_native_identity(&mut state, this, 0, &mut previous_successful_syncs);
            state.interval = 0;
            state.nominal_interval_micros = 0;
            state.cadence_interval_micros = 0;
            state.presented_updates = 0;
            state.catching_up = false;
            state.in_step = true;
            STATE.with(|local| *local.borrow_mut() = state);
            return;
        }
        let data = if this == 0 {
            0
        } else {
            // SAFETY: this detour is installed only at the exact verified
            // CGame::Step entry, where `this` is a live CGame pointer.
            unsafe { *((this + DATA_OFFSET) as *const usize) }
        };
        let identity_changed =
            adopt_native_identity(&mut state, this, data, &mut previous_successful_syncs);
        if data == 0 || data % std::mem::align_of::<i64>() != 0 {
            state.in_step = true;
            state.interval = 0;
            state.overridden = false;
            STATE.with(|local| *local.borrow_mut() = state);
            super::fail_runtime();
            return;
        }

        // SAFETY: this is the aligned scheduler block from the verified
        // CGame object, accessed only from the CGame::Step thread.
        let interval = unsafe { *((data + INTERVAL_OFFSET) as *const i32) };
        state.interval = interval;
        if identity_changed && interval > 0 {
            // Keep the new object's own native period available for a pause
            // or mode change before its first ComputeFrameTime call.
            state.legacy_interval = interval;
        }
        if let Some(_pace) = target_pace {
            if interval <= 0 {
                state.in_step = true;
                state.overridden = false;
                STATE.with(|local| *local.borrow_mut() = state);
                super::fail_runtime();
                return;
            }
            let Some(cadence) = next_cadence(
                snapshot,
                Cadence {
                    nominal_interval_micros: state.nominal_interval_micros,
                    interval_micros: state.cadence_interval_micros,
                    catching_up: state.catching_up,
                },
                previous_successful_syncs,
            ) else {
                state.in_step = true;
                state.overridden = false;
                STATE.with(|local| *local.borrow_mut() = state);
                super::fail_runtime();
                return;
            };
            let presented_updates = if snapshot.batch_two {
                state.presented_updates.max(1)
            } else {
                1
            };
            let target_interval = if snapshot.batch_two {
                presentation_interval_micros(snapshot, presented_updates, cadence.interval_micros)
            } else {
                Some(cadence.interval_micros)
            };
            let Some(target_interval) = target_interval else {
                state.in_step = true;
                state.overridden = false;
                STATE.with(|local| *local.borrow_mut() = state);
                super::fail_runtime();
                return;
            };
            let target = i32::try_from(target_interval).unwrap_or(i32::MAX);
            state.nominal_interval_micros = cadence.nominal_interval_micros;
            state.cadence_interval_micros = cadence.interval_micros;
            state.presented_updates = presented_updates;
            state.catching_up = cadence.catching_up;
            state.interval = target;
            if interval != target {
                // SAFETY: see the scheduler-block contract above.
                let now = unsafe { *((data + NOW_OFFSET) as *const i64) };
                // SAFETY: see the scheduler-block contract above.
                let last = unsafe { *((data + LAST_OFFSET) as *const i64) };
                let last = remap_last(now, last, interval, target);
                // CGame::Step tests this phase before the first Sync.
                unsafe { *((data + LAST_OFFSET) as *mut i64) = last };
                unsafe { *((data + INTERVAL_OFFSET) as *mut i32) = target };
            }
            state.overridden = true;
        } else if state.overridden {
            let legacy = state.legacy_interval;
            if legacy <= 0 || interval <= 0 {
                state.in_step = true;
                state.overridden = false;
                STATE.with(|local| *local.borrow_mut() = state);
                super::fail_runtime();
                return;
            }
            if interval != legacy {
                // SAFETY: see the scheduler-block contract above.
                let now = unsafe { *((data + NOW_OFFSET) as *const i64) };
                // SAFETY: see the scheduler-block contract above.
                let last = unsafe { *((data + LAST_OFFSET) as *const i64) };
                let last = remap_last(now, last, interval, legacy);
                unsafe { *((data + LAST_OFFSET) as *mut i64) = last };
                unsafe { *((data + INTERVAL_OFFSET) as *mut i32) = legacy };
            }
            state.interval = legacy;
            state.overridden = false;
            state.nominal_interval_micros = 0;
            state.cadence_interval_micros = 0;
            state.presented_updates = 0;
            state.catching_up = false;
        }
        state.in_step = true;
        STATE.with(|local| *local.borrow_mut() = state);
    }

    /// Completes a main-thread `CGame::Step` call and samples cumulative
    /// worker completions. A worker update that finishes after this call is
    /// accounted on a later sample rather than discarded or misattributed to
    /// the Sync callback that scheduled it.
    pub unsafe fn end_step() -> FrameSample {
        let scheduler_debt = SCHEDULER_DEBT.swap(0, Ordering::AcqRel);
        let totals = completion_totals();
        let capped_lookaheads = LOOKAHEAD_CAP_HITS.load(Ordering::Acquire);
        let sealed_lookahead = u32::try_from(SEALED_LOOKAHEAD_HIGH_WATER.swap(0, Ordering::AcqRel))
            .unwrap_or(u32::MAX);
        let sample = STATE.with(|local| {
            let mut state = local.borrow_mut();
            let completed = completion_delta(totals, state.observed_completions);
            let lookahead_cap_hits =
                capped_lookaheads.saturating_sub(state.observed_lookahead_cap_hits);
            state.observed_completions = totals;
            state.observed_lookahead_cap_hits = capped_lookaheads;
            let mut sample = FrameSample {
                frame_updates: u32::try_from(completed.updates).unwrap_or(u32::MAX),
                worker_step_wall_nanos: completed.step_wall_nanos,
                sealed_lookahead,
                lookahead_cap_hits,
                active_worker_calls: super::active_worker_calls(),
                reserve_steps: state.reserve_steps,
                sync_calls: state.sync_calls,
                compute_frame_calls: state.compute_frame_calls,
                scheduler_debt,
                ..FrameSample::default()
            };
            if !state.in_step || !state.overridden || state.data == 0 {
                return sample;
            }
            let data = state.data;
            // SAFETY: same CGame main-thread ownership as begin_step.
            let now = unsafe { *((data + NOW_OFFSET) as *const i64) };
            // SAFETY: same CGame main-thread ownership as begin_step.
            let last = unsafe { *((data + LAST_OFFSET) as *const i64) };
            // SAFETY: same CGame main-thread ownership as begin_step.
            let interval = unsafe { *((data + INTERVAL_OFFSET) as *const i32) };
            let phase = if interval > 0 {
                (i128::from(now) - i128::from(last)).clamp(0, i128::from(interval - 1)) * 1_000
                    / i128::from(interval)
            } else {
                0
            };
            sample.active = true;
            sample.interval_micros = u32::try_from(interval.max(0)).unwrap_or(u32::MAX);
            sample.phase_per_mille = u32::try_from(phase).unwrap_or(0);
            sample
        });
        STATE.with(|local| local.borrow_mut().in_step = false);
        sample
    }

    /// Captures CGame::Step's pre-Sync last and period. Its `rdi` register
    /// retains `last + interval` across the call and the caller writes that
    /// exact value only if Sync returns true.
    pub unsafe fn before_sync(this: usize) {
        STATE.with(|local| {
            let mut state = local.borrow_mut();
            state.sync_snapshot_valid = false;
            if !state.in_step || !state.overridden || this != state.cgame || state.data == 0 {
                return;
            }
            let current_data = if this == 0 {
                0
            } else {
                // SAFETY: wrapper is called only from the verified CGame::Step
                // callsite, on the same owner thread as begin_step.
                unsafe { *((this + DATA_OFFSET) as *const usize) }
            };
            if current_data != state.data || current_data % std::mem::align_of::<i64>() != 0 {
                super::fail_runtime();
                state.overridden = false;
                return;
            }
            // SAFETY: validated scheduler block; this is the value cached by
            // CGame::Step immediately before its Sync call.
            let last = unsafe { *((state.data + LAST_OFFSET) as *const i64) };
            // SAFETY: same validated block and callsite.
            let interval = unsafe { *((state.data + INTERVAL_OFFSET) as *const i32) };
            if interval <= 0 || interval != state.interval {
                super::fail_runtime();
                state.overridden = false;
                return;
            }
            state.sync_last_before = last;
            state.sync_interval_before = interval;
            state.sync_snapshot_valid = true;
        });
    }

    /// The redirect calls the original for its bookkeeping, then overrides
    /// only the Sync site's returned interval when a verified room pace is
    /// active. A frame uses one fixed interval, so the scheduler's threshold
    /// and post-Sync alpha always agree.
    pub unsafe fn compute_frame_time(original: usize, a: usize, b: u32, c: u32) -> (i32, i32) {
        let function: unsafe extern "C" fn(usize, u32, u32) -> i32 =
            // SAFETY: the profile-resolved target is the original native
            // ComputeFrameTime function with this call site's ABI.
            unsafe { std::mem::transmute(original) };
        // SAFETY: forwarded exactly as CGame::Sync called the function.
        let legacy = unsafe { function(a, b, c) };
        // This call occurs after the native snapshot swap and before the
        // worker receives its next signal. It is the only place that consumes
        // the completed batch published by GameSim::Step.
        let applied = apply_compute_frame_interval(legacy, super::current());
        (legacy, applied)
    }

    fn apply_compute_frame_interval(legacy: i32, snapshot: super::Snapshot) -> i32 {
        STATE.with(|local| {
            let mut state = local.borrow_mut();
            state.legacy_interval = legacy;
            let override_active = state.in_step && state.overridden && state.interval > 0;
            if override_active {
                state.compute_frame_calls = state.compute_frame_calls.saturating_add(1);
            }
            if !override_active || !snapshot.batch_two {
                return super::applied_interval(
                    legacy,
                    state.in_step,
                    state.overridden,
                    state.interval,
                );
            }

            let Some(pace) = plan(snapshot) else {
                state.interval = legacy;
                state.cadence_interval_micros = 0;
                state.presented_updates = 0;
                state.nominal_interval_micros = 0;
                state.catching_up = false;
                state.overridden = false;
                return legacy;
            };
            let completed_updates = take_completed_batch();
            if completed_updates == 0 {
                // No sealed room update completed since the prior swap. Keep
                // the last positive presentation duration; never synthesize
                // an interval or advance the simulation clock.
                return if state.interval > 0 {
                    state.interval
                } else {
                    legacy
                };
            }
            if completed_updates > 2 {
                state.overridden = false;
                super::fail_runtime();
                return legacy;
            }

            if state.nominal_interval_micros != pace.interval_micros {
                // A room-rate transition is authoritative. ComputeFrameTime
                // applies the new count-scaled interval after the snapshot
                // swap; begin_step remaps phase only on the next outer entry.
                state.nominal_interval_micros = pace.interval_micros;
                state.cadence_interval_micros = pace.interval_micros;
                state.catching_up = false;
            }
            let effective_unit = state
                .cadence_interval_micros
                .max(pace.interval_micros.saturating_mul(100).div_ceil(110));
            let Some(interval) =
                presentation_interval_micros(snapshot, completed_updates, effective_unit)
            else {
                state.overridden = false;
                super::fail_runtime();
                return legacy;
            };
            state.presented_updates = completed_updates;
            state.interval = i32::try_from(interval).unwrap_or(i32::MAX);
            state.interval
        })
    }

    /// Runs on the CGame::Step owner thread after original Sync returns and
    /// before its caller commits the cached `last + interval`. Sync is not
    /// treated as a worker-completion barrier: only cumulative completions
    /// observed by this point contribute to the outer-call budget.
    pub unsafe fn after_sync(this: usize, succeeded: bool) {
        STATE.with(|local| {
            let mut state = local.borrow_mut();
            if !state.in_step || !state.overridden || this != state.cgame || state.data == 0 {
                return;
            }
            state.sync_calls = state.sync_calls.saturating_add(1);
            if succeeded {
                state.successful_syncs = state.successful_syncs.saturating_add(1);
            }
            let valid_snapshot = std::mem::take(&mut state.sync_snapshot_valid);
            if !valid_snapshot {
                return;
            }
            // Verify the live object before writing. The original Sync may
            // return before or after simulation work completes.
            let current_data = unsafe { *((this + DATA_OFFSET) as *const usize) };
            if current_data != state.data || current_data % std::mem::align_of::<i64>() != 0 {
                super::fail_runtime();
                state.overridden = false;
                return;
            }
            // SAFETY: verified scheduler block and same CGame owner thread.
            let now = unsafe { *((state.data + NOW_OFFSET) as *const i64) };
            // ComputeFrameTime installs this frame's stable period. CGame::Step
            // will commit the cached pre-Sync `last + interval` on success.
            let post_interval = unsafe { *((state.data + INTERVAL_OFFSET) as *const i32) };
            if post_interval <= 0 {
                super::fail_runtime();
                state.overridden = false;
                return;
            }
            let completed = completion_delta(completion_totals(), state.outer_completion_start);
            let outer_wall = state
                .frame_started
                .map(|started| u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX))
                .unwrap_or(0);
            let stop = stop_after_sync(
                state.sync_calls,
                u32::try_from(completed.updates).unwrap_or(u32::MAX),
                completed.step_wall_nanos,
                outer_wall,
            );
            if !succeeded || !stop {
                return;
            }
            let Some(adjustment) = clamp_after_sync(
                now,
                state.sync_last_before,
                state.sync_interval_before,
                post_interval,
                true,
                true,
            ) else {
                return;
            };
            // This leaves the fractional phase after CGame::Step writes its
            // cached last value and exits its due-Sync loop.
            unsafe { *((state.data + NOW_OFFSET) as *mut i64) = adjustment.now };
            SCHEDULER_DEBT.fetch_add(adjustment.dropped_intervals, Ordering::AcqRel);
        });
    }

    /// Adds one outer scheduler call to the low-rate telemetry window. It
    /// allocates no strings on the hot path; formatting is left to the caller
    /// only when a ten-second report is ready.
    pub fn record_outer_step(elapsed: Duration, sample: FrameSample) -> Option<TelemetryReport> {
        if !sample.active {
            return None;
        }
        let now = Instant::now();
        let mut slot = TELEMETRY
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let window = slot.get_or_insert_with(|| Window::new(now));
        window.outer_steps = window.outer_steps.saturating_add(1);
        window.room_updates = window
            .room_updates
            .saturating_add(u64::from(sample.frame_updates));
        window.worker_step_wall_micros = window
            .worker_step_wall_micros
            .saturating_add(sample.worker_step_wall_nanos / 1_000);
        window.maximum_sealed_lookahead = window
            .maximum_sealed_lookahead
            .max(u64::from(sample.sealed_lookahead));
        window.lookahead_cap_hits = window
            .lookahead_cap_hits
            .saturating_add(sample.lookahead_cap_hits);
        window.maximum_active_worker_calls = window
            .maximum_active_worker_calls
            .max(sample.active_worker_calls);
        window.maximum_reserve_steps = window.maximum_reserve_steps.max(sample.reserve_steps);
        window.sync_calls = window
            .sync_calls
            .saturating_add(u64::from(sample.sync_calls));
        window.compute_frame_calls = window
            .compute_frame_calls
            .saturating_add(u64::from(sample.compute_frame_calls));
        window.scheduler_debt = window.scheduler_debt.saturating_add(sample.scheduler_debt);
        let micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        window.step_micros = window.step_micros.saturating_add(micros);
        window.maximum_step_micros = window.maximum_step_micros.max(micros);
        if sample.interval_micros > 0 {
            window.minimum_period_micros = window
                .minimum_period_micros
                .min(u64::from(sample.interval_micros));
            window.maximum_period_micros = window
                .maximum_period_micros
                .max(u64::from(sample.interval_micros));
            window.minimum_phase_per_mille =
                window.minimum_phase_per_mille.min(sample.phase_per_mille);
            window.maximum_phase_per_mille =
                window.maximum_phase_per_mille.max(sample.phase_per_mille);
        }
        let elapsed_window = window.started.elapsed();
        if elapsed_window < TELEMETRY_WINDOW {
            return None;
        }
        let report = window.report(elapsed_window);
        *window = Window::new(now);
        Some(report)
    }

    /// Whether the optional one-line per-call native pacing trace is enabled.
    pub fn trace_enabled() -> bool {
        super::TRACE.load(Ordering::Acquire)
    }

    #[cfg(test)]
    mod lifecycle_tests {
        use std::ptr;

        use super::*;

        static COMPLETION_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

        #[repr(align(8))]
        struct AlignedBytes<const N: usize>([u8; N]);

        struct NativeFixture {
            cgame: Box<AlignedBytes<0x200>>,
            data: Box<AlignedBytes<0x220>>,
        }

        impl NativeFixture {
            fn new(interval: i32) -> Self {
                let mut data = Box::new(AlignedBytes([0; 0x220]));
                let data_base = data.0.as_mut_ptr() as usize;
                // SAFETY: the aligned byte block covers each verified field.
                unsafe {
                    ptr::write((data_base + NOW_OFFSET) as *mut i64, 1_000_000);
                    ptr::write((data_base + LAST_OFFSET) as *mut i64, 980_000);
                    ptr::write((data_base + INTERVAL_OFFSET) as *mut i32, interval);
                }
                let mut cgame = Box::new(AlignedBytes([0; 0x200]));
                let cgame_base = cgame.0.as_mut_ptr() as usize;
                // SAFETY: the aligned block covers CGame's verified data slot.
                unsafe {
                    ptr::write((cgame_base + DATA_OFFSET) as *mut usize, data_base);
                }
                Self { cgame, data }
            }

            fn cgame(&mut self) -> usize {
                self.cgame.0.as_mut_ptr() as usize
            }

            fn data(&mut self) -> usize {
                self.data.0.as_mut_ptr() as usize
            }

            fn interval(&mut self) -> i32 {
                let data = self.data();
                // SAFETY: the fixture owns the aligned verified field bytes.
                unsafe { ptr::read((data + INTERVAL_OFFSET) as *const i32) }
            }

            fn last(&mut self) -> i64 {
                let data = self.data();
                // SAFETY: the fixture owns the aligned verified field bytes.
                unsafe { ptr::read((data + LAST_OFFSET) as *const i64) }
            }
        }

        fn four_x_snapshot(sealed_ahead: u16, checkpoint_limited: bool) -> super::super::Snapshot {
            super::super::Snapshot {
                active: true,
                steps_per_second: 5,
                speed: super::super::Speed(400),
                previous_speed: super::super::Speed::NORMAL,
                sealed_ahead,
                lookahead_capped: false,
                checkpoint_limited,
                batch_two: false,
            }
        }

        fn seed_cached_state(cgame: usize, data: usize, successful_syncs: u32) {
            STATE.with(|thread| {
                *thread.borrow_mut() = State {
                    cgame,
                    data,
                    interval: 45_455,
                    legacy_interval: 200_000,
                    overridden: true,
                    successful_syncs,
                    nominal_interval_micros: 50_000,
                    cadence_interval_micros: 45_455,
                    presented_updates: 1,
                    catching_up: true,
                    sync_snapshot_valid: true,
                    ..State::default()
                };
            });
        }

        fn batch_two_snapshot(speed: super::super::Speed) -> super::super::Snapshot {
            super::super::Snapshot {
                active: true,
                steps_per_second: 5,
                speed,
                previous_speed: super::super::Speed::NORMAL,
                sealed_ahead: 0,
                lookahead_capped: false,
                checkpoint_limited: false,
                batch_two: true,
            }
        }

        fn cached_state() -> State {
            STATE.with(|thread| thread.borrow().clone())
        }

        fn assert_new_identity_rebases(
            old_cgame: usize,
            old_data: usize,
            new_cgame: usize,
            new_data: usize,
            new_fixture: &mut NativeFixture,
        ) {
            seed_cached_state(old_cgame, old_data, 9);

            // The queue estimate is inside the catch-up deadband and ends at
            // a checkpoint, so it must not clear inherited catch-up by itself.
            // Only recognizing the new native identity can rebase this object.
            unsafe {
                begin_step_with_snapshot(new_cgame, four_x_snapshot(5, true));
            }

            let state = cached_state();
            assert_eq!(state.cgame, new_cgame);
            assert_eq!(state.data, new_data);
            assert_eq!(state.nominal_interval_micros, 50_000);
            assert_eq!(state.interval, 50_000);
            assert!(!state.catching_up);
            assert_eq!(state.successful_syncs, 0);
            assert!(state.overridden);
            assert_eq!(new_fixture.interval(), 50_000);
            assert_eq!(
                new_fixture.last(),
                remap_last(1_000_000, 980_000, 45_455, 50_000)
            );
        }

        #[test]
        fn changed_cgame_rebases_an_in_band_checkpoint_limited_cadence() {
            let mut old = NativeFixture::new(45_455);
            let mut new = NativeFixture::new(45_455);
            let old_cgame = old.cgame();
            let old_data = old.data();
            let new_cgame = new.cgame();
            let new_data = new.data();
            assert_new_identity_rebases(old_cgame, old_data, new_cgame, new_data, &mut new);
        }

        #[test]
        fn changed_scheduler_data_rebases_an_in_band_checkpoint_limited_cadence() {
            let mut old = NativeFixture::new(45_455);
            let mut new = NativeFixture::new(45_455);
            let same_cgame = old.cgame();
            // Model a CGame whose scheduler-data pointer was replaced during
            // world lifecycle while the CGame object itself stayed in place.
            let new_data = new.data();
            unsafe {
                ptr::write((same_cgame + DATA_OFFSET) as *mut usize, new_data);
            }
            let old_data = old.data();
            assert_new_identity_rebases(same_cgame, old_data, same_cgame, new_data, &mut new);
        }

        #[test]
        fn same_native_identity_keeps_its_bounded_recovery_slew() {
            let mut fixture = NativeFixture::new(50_000);
            let cgame = fixture.cgame();
            let data = fixture.data();
            seed_cached_state(cgame, data, 1);
            STATE.with(|thread| {
                let mut state = thread.borrow_mut();
                state.interval = 50_000;
                state.cadence_interval_micros = 50_000;
                state.presented_updates = 1;
                state.catching_up = false;
            });

            unsafe {
                begin_step_with_snapshot(cgame, four_x_snapshot(7, false));
            }

            let state = cached_state();
            assert_eq!(state.interval, 49_500);
            assert!(state.catching_up);
            assert_eq!(fixture.interval(), 49_500);
        }

        #[test]
        fn compute_frame_time_uses_each_completed_batch_count_and_zero_holds_duration() {
            let _guard = COMPLETION_TEST_LOCK.lock().unwrap();
            let mut fixture = NativeFixture::new(50_000);
            let cgame = fixture.cgame();
            let data = fixture.data();
            seed_cached_state(cgame, data, 0);
            STATE.with(|thread| {
                let mut state = thread.borrow_mut();
                state.in_step = true;
                state.interval = 50_000;
                state.legacy_interval = 200_000;
                state.nominal_interval_micros = 50_000;
                state.cadence_interval_micros = 50_000;
                state.presented_updates = 1;
                state.catching_up = false;
            });
            super::super::COMPLETED_BATCH_UPDATES.store(0, Ordering::Release);

            for (completed, expected) in [(2, 100_000), (1, 50_000), (2, 100_000), (0, 100_000)] {
                super::super::publish_completed_batch(completed);
                assert_eq!(
                    apply_compute_frame_interval(
                        200_000,
                        batch_two_snapshot(super::super::Speed(400))
                    ),
                    expected
                );
                assert_eq!(
                    fixture.last(),
                    980_000,
                    "ComputeFrameTime never remaps cached last"
                );
                assert_eq!(cached_state().interval, expected);
            }
            assert_eq!(
                cached_state().presented_updates,
                2,
                "zero completion holds the last presentation"
            );
            assert_eq!(
                super::super::COMPLETED_BATCH_UPDATES.load(Ordering::Acquire),
                0
            );
        }

        #[test]
        fn begin_step_uses_the_previous_presented_count_not_the_pending_worker_count() {
            let _guard = COMPLETION_TEST_LOCK.lock().unwrap();
            let mut fixture = NativeFixture::new(50_000);
            let cgame = fixture.cgame();
            let data = fixture.data();
            seed_cached_state(cgame, data, 0);
            STATE.with(|thread| {
                let mut state = thread.borrow_mut();
                state.in_step = false;
                state.interval = 50_000;
                state.legacy_interval = 200_000;
                state.nominal_interval_micros = 50_000;
                state.cadence_interval_micros = 50_000;
                state.presented_updates = 1;
                state.catching_up = false;
            });
            super::super::publish_completed_batch(2);
            let snapshot = batch_two_snapshot(super::super::Speed(400));

            unsafe { begin_step_with_snapshot(cgame, snapshot) };
            assert_eq!(fixture.interval(), 50_000);
            assert_eq!(cached_state().presented_updates, 1);
            assert_eq!(
                super::super::COMPLETED_BATCH_UPDATES.load(Ordering::Acquire),
                2,
                "only the subsequent Sync ComputeFrameTime consumes the pending count"
            );

            assert_eq!(apply_compute_frame_interval(200_000, snapshot), 100_000);
            assert_eq!(cached_state().presented_updates, 2);
            assert_eq!(fixture.last(), 980_000);
            super::super::COMPLETED_BATCH_UPDATES.store(0, Ordering::Release);
        }

        #[test]
        fn speed_change_remaps_the_presented_batch_and_pause_keeps_its_positive_period() {
            let mut fixture = NativeFixture::new(100_000);
            let cgame = fixture.cgame();
            let data = fixture.data();
            seed_cached_state(cgame, data, 0);
            STATE.with(|thread| {
                let mut state = thread.borrow_mut();
                state.interval = 100_000;
                state.nominal_interval_micros = 50_000;
                state.cadence_interval_micros = 50_000;
                state.presented_updates = 2;
                state.catching_up = false;
            });

            unsafe {
                begin_step_with_snapshot(cgame, batch_two_snapshot(super::super::Speed(200)))
            };
            assert_eq!(fixture.interval(), 200_000);
            assert_eq!(
                fixture.last(),
                remap_last(1_000_000, 980_000, 100_000, 200_000)
            );

            let mut paused = batch_two_snapshot(super::super::Speed::PAUSED);
            paused.previous_speed = super::super::Speed(200);
            unsafe { begin_step_with_snapshot(cgame, paused) };
            assert_eq!(fixture.interval(), 200_000);
            assert_eq!(cached_state().presented_updates, 2);
        }
    }

    // Use a thread-local frame state; the wrapper is at CGame::Step's exact
    // Sync callsite, so before/after hooks stay on the CGame owner thread.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normal(speed: u16) -> Snapshot {
        Snapshot {
            active: true,
            steps_per_second: 5,
            speed: Speed(speed),
            previous_speed: Speed::NORMAL,
            sealed_ahead: 0,
            lookahead_capped: false,
            checkpoint_limited: false,
            batch_two: false,
        }
    }

    #[test]
    fn snapshot_packs_bounded_backlog_and_checkpoint_observation_atomically() {
        let mut snapshot = normal(400);
        snapshot.sealed_ahead = 700;
        snapshot.lookahead_capped = true;
        snapshot.checkpoint_limited = true;
        snapshot.batch_two = true;
        let decoded = Snapshot::decode(snapshot.encode());
        snapshot.sealed_ahead = u16::try_from(SEALED_AHEAD_MASK).unwrap();
        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn backlog_catchup_has_hysteresis_and_a_bounded_period_slew() {
        let mut snapshot = normal(400);
        snapshot.sealed_ahead = 7;
        let base = Cadence {
            nominal_interval_micros: 50_000,
            interval_micros: 50_000,
            catching_up: false,
        };
        let first = next_cadence(snapshot, base, 1).unwrap();
        assert!(first.catching_up);
        assert_eq!(first.interval_micros, 49_500, "one success permits 1% slew");

        let target = next_cadence(snapshot, first, 9).unwrap();
        assert_eq!(target.interval_micros, 45_455, "at most 10% faster rate");

        snapshot.sealed_ahead = 5;
        let held = next_cadence(snapshot, target, 1).unwrap();
        assert!(held.catching_up, "the deadband avoids jitter toggles");
        assert_eq!(held.interval_micros, target.interval_micros);

        snapshot.sealed_ahead = 1;
        snapshot.checkpoint_limited = true;
        let checkpoint = next_cadence(snapshot, target, 1).unwrap();
        assert!(
            checkpoint.catching_up,
            "a checkpoint may truncate lookahead"
        );

        snapshot.checkpoint_limited = false;
        let recovering = next_cadence(snapshot, checkpoint, 1).unwrap();
        assert!(
            !recovering.catching_up,
            "leave at or below the startup lead"
        );
        assert_eq!(
            recovering.interval_micros, 45_955,
            "return also slews by 1%"
        );
    }

    #[test]
    fn pause_and_room_rate_change_reset_catchup_with_a_new_nominal_period() {
        let current = Cadence {
            nominal_interval_micros: 50_000,
            interval_micros: 45_455,
            catching_up: true,
        };
        let mut paused = normal(0);
        paused.previous_speed = Speed(400);
        paused.sealed_ahead = 7;
        let held = next_cadence(paused, current, 0).unwrap();
        assert_eq!(held.interval_micros, 50_000);
        assert!(!held.catching_up);

        let changed = next_cadence(normal(200), current, 10).unwrap();
        assert_eq!(changed.nominal_interval_micros, 100_000);
        assert_eq!(changed.interval_micros, 100_000);
        assert!(!changed.catching_up);
    }

    #[test]
    fn activation_rebases_a_legacy_period_even_if_cached_nominal_matches_room_speed() {
        let mut active = normal(400);
        active.sealed_ahead = 7;
        let stale_state = Cadence {
            nominal_interval_micros: 50_000,
            interval_micros: 200_000,
            catching_up: false,
        };

        let first = next_cadence(active, stale_state, 1).unwrap();
        assert!(first.catching_up);
        assert_eq!(first.interval_micros, 50_000);
        assert_ne!(first.interval_micros, 199_500);
    }

    #[test]
    fn native_intervals_follow_room_rate_and_speed() {
        assert_eq!(plan(normal(100)).unwrap().interval_micros, 200_000);
        assert_eq!(plan(normal(200)).unwrap().interval_micros, 100_000);
        assert_eq!(plan(normal(300)).unwrap().interval_micros, 66_667);
        assert_eq!(plan(normal(400)).unwrap().interval_micros, 50_000);
        assert_eq!(
            plan(normal(500)),
            None,
            "unsupported speeds use legacy pacing"
        );
    }

    #[test]
    fn lead_is_left_after_consuming_one_update() {
        let one = plan(normal(100)).unwrap();
        let two = plan(normal(200)).unwrap();
        let three = plan(normal(300)).unwrap();
        let four = plan(normal(400)).unwrap();
        assert_eq!(one.reserve_steps, 1);
        assert_eq!(two.reserve_steps, 1);
        assert_eq!(three.reserve_steps, 2);
        assert_eq!(four.reserve_steps, 2);
        assert_eq!(
            [
                one.startup_ahead,
                two.startup_ahead,
                three.startup_ahead,
                four.startup_ahead
            ],
            [2, 2, 3, 3]
        );
    }

    #[test]
    fn batch_two_is_opt_in_and_only_applies_above_two_x() {
        assert!(!batch_two_requested_for(false, Some("2")));
        assert!(!batch_two_requested_for(true, Some("1")));
        assert!(batch_two_requested_for(true, Some("2")));
        assert_eq!(max_batch_updates(normal(100)), 1);
        assert_eq!(max_batch_updates(normal(200)), 1);
        assert_eq!(max_batch_updates(normal(300)), 1);
        assert_eq!(max_batch_updates(normal(400)), 1);

        let mut batch_two = normal(300);
        batch_two.batch_two = true;
        assert_eq!(max_batch_updates(batch_two), 2);
        batch_two.speed = Speed(400);
        assert_eq!(max_batch_updates(batch_two), 2);
        batch_two.speed = Speed::PAUSED;
        batch_two.previous_speed = Speed(400);
        assert_eq!(max_batch_updates(batch_two), 1);
    }

    #[test]
    fn two_update_priming_keeps_the_entire_release_reserve_afterward() {
        let mut snapshot = normal(400);
        snapshot.batch_two = true;
        let pace = plan(snapshot).unwrap();
        assert_eq!(pace.reserve_steps, 2);
        assert_eq!(startup_ahead(pace, snapshot), 4);
        assert_eq!(batch_limit_with_max(3, 20, pace, false, false, 2), 0);
        assert_eq!(batch_limit_with_max(4, 20, pace, false, false, 2), 2);
        assert_eq!(4 - batch_limit_with_max(4, 20, pace, false, false, 2), 2);
        assert_eq!(batch_limit_with_max(20, 1, pace, true, false, 2), 1);
    }

    #[test]
    fn exact_batch_presentation_intervals_follow_count_speed_pause_and_recovery() {
        let mut snapshot = normal(400);
        snapshot.batch_two = true;
        assert_eq!(
            presentation_interval_micros(snapshot, 2, 50_000),
            Some(100_000)
        );
        assert_eq!(
            presentation_interval_micros(snapshot, 1, 50_000),
            Some(50_000)
        );
        assert_eq!(
            presentation_interval_micros(snapshot, 2, 45_455),
            Some(90_910)
        );
        assert_eq!(presentation_interval_micros(snapshot, 0, 50_000), None);
        assert_eq!(presentation_interval_micros(snapshot, 3, 50_000), None);

        snapshot.speed = Speed(300);
        assert_eq!(
            presentation_interval_micros(snapshot, 2, 66_667),
            Some(133_333)
        );
        snapshot.speed = Speed::PAUSED;
        snapshot.previous_speed = Speed(200);
        assert_eq!(
            presentation_interval_micros(snapshot, 1, 100_000),
            Some(100_000)
        );
    }

    #[test]
    fn capped_post_consumption_queue_remains_an_explicit_lower_bound() {
        let mut snapshot = normal(400);
        snapshot.batch_two = true;
        snapshot.sealed_ahead = 5;
        snapshot.lookahead_capped = true;
        let pace = plan(snapshot).unwrap();
        assert!(u32::from(snapshot.sealed_ahead) < pace.lookahead_limit);
        assert!(u32::from(snapshot.sealed_ahead) > startup_ahead(pace, snapshot));
        let cadence = next_cadence(
            snapshot,
            Cadence {
                nominal_interval_micros: 50_000,
                interval_micros: 50_000,
                catching_up: false,
            },
            1,
        )
        .unwrap();
        assert!(cadence.catching_up);
        assert_eq!(cadence.interval_micros, 49_500);
    }

    #[test]
    fn backlog_telemetry_horizon_stays_bounded_and_callbacks_remain_one_update() {
        let pace = plan(normal(400)).unwrap();
        assert_eq!(pace.lookahead_limit, 7, "2 retained + 1 due + 4 observed");
        assert_eq!(batch_limit(6, 20, pace, true, false), 1);
        assert_eq!(batch_limit(7, 20, pace, true, false), 1);
        assert_eq!(batch_limit(50, 20, pace, true, false), 1);
        assert_eq!(batch_limit(7, 3, pace, true, false), 1);
    }

    #[test]
    fn asynchronous_completion_deltas_are_counted_once_when_sampled_later() {
        let start = completion_totals();
        let mut cursor = start;
        let before_worker_finishes = completion_totals();
        assert_eq!(completion_delta(before_worker_finishes, cursor).updates, 0);

        record_worker_completion(1, 20_000_000);
        let after_worker_finishes = completion_totals();
        let delayed = completion_delta(after_worker_finishes, cursor);
        assert_eq!(delayed.updates, 1);
        assert_eq!(delayed.step_wall_nanos, 20_000_000);
        cursor = after_worker_finishes;
        assert_eq!(completion_delta(completion_totals(), cursor).updates, 0);
    }

    #[test]
    fn observe_only_keeps_the_native_compute_frame_interval() {
        assert_eq!(applied_interval(200_000, true, false, 50_000), 200_000);
        assert_eq!(applied_interval(200_000, false, true, 50_000), 200_000);
        assert_eq!(applied_interval(200_000, true, true, 50_000), 50_000);
    }

    #[test]
    fn sustained_backlog_is_drained_one_sealed_update_per_callback() {
        let pace = plan(normal(400)).unwrap();
        let mut ahead = 80;
        let mut calls = 0;
        while ahead > 0 {
            let selected = batch_limit(ahead, 200, pace, true, false);
            assert_eq!(selected, 1);
            ahead -= selected;
            calls += 1;
        }
        assert_eq!(calls, 80, "no sealed release was combined or discarded");
    }

    #[test]
    fn checkpoint_can_progress_when_it_is_closer_than_the_prime_lead() {
        let pace = plan(normal(400)).unwrap();
        assert!(checkpoint_limits_prime(2, 2, pace));
        assert_eq!(batch_limit(2, 2, pace, false, false), 1);
        assert_eq!(batch_limit(1, 1, pace, false, false), 1);
    }

    #[test]
    fn pause_drains_one_release_at_the_previous_native_pace() {
        let mut paused = normal(0);
        paused.previous_speed = Speed(300);
        assert_eq!(plan(paused).unwrap().interval_micros, 66_667);
        assert_eq!(batch_limit(12, 12, plan(paused).unwrap(), true, true), 1);
    }

    #[test]
    fn interval_changes_preserve_phase_and_overdue_count() {
        assert_eq!(remap_last(50, 0, 100, 200), -50);
        assert_eq!(remap_last(450, 0, 100, 200), -450);
        assert_eq!(remap_last(50, 0, 100, 0), 0);
    }

    #[test]
    fn sync_clamp_models_step_cached_last_and_ordinary_no_clamp() {
        let last_before = 1_000_000;
        let interval = 200_000;
        let expected_native_last = last_before + i64::from(interval);
        let now = expected_native_last + i64::from(interval) - 1;

        assert_eq!(
            clamp_after_sync(now, last_before, interval, interval, true, true),
            None,
            "one due Sync with only a fractional remainder needs no timer clamp"
        );
        assert_eq!(
            clamp_after_sync(
                expected_native_last + 3 * i64::from(interval) + 17,
                last_before,
                interval,
                interval,
                true,
                false,
            ),
            None,
            "an ordinary Sync preserves the native clock even when a test fixture is overdue"
        );
    }

    #[test]
    fn long_stall_clamp_keeps_native_last_and_fractional_alpha() {
        let last_before = 4_000_000;
        let interval = 200_000;
        let native_last = last_before + i64::from(interval);
        let fractional_phase = 12_345;
        let now = native_last + 5 * i64::from(interval) + fractional_phase;
        let adjustment = clamp_after_sync(now, last_before, interval, interval, true, true)
            .expect("the elapsed-time safety limit should shed overdue timer intervals");

        assert_eq!(adjustment.native_last, native_last);
        assert_eq!(adjustment.dropped_intervals, 5);
        assert_eq!(adjustment.now - adjustment.native_last, fractional_phase);
        assert!(adjustment.now < adjustment.native_last + i64::from(interval));
    }

    #[test]
    fn native_callback_cap_bounds_pause_and_stall_without_worker_completion() {
        let budget = u64::try_from(FRAME_WALL_BUDGET.as_nanos()).unwrap();
        assert!(!stop_after_sync(0, 0, 0, 5_000_000));
        assert!(!stop_after_sync(1, 0, 0, 5_000_000));
        assert!(stop_after_sync(MAX_SYNC_CALLS, 0, 0, 5_000_000));
        assert!(stop_after_sync(0, 0, budget, 5_000_000));
        assert!(stop_after_sync(0, MAX_UPDATES_PER_OUTER_STEP, 0, 5_000_000));
        assert!(stop_after_sync(0, 0, 0, budget));

        let interval = 50_000;
        let mut last = 0_i64;
        let now = 20 * i64::from(interval) + 7;
        let room_updates = 0;
        let mut sync_calls = 0;
        let mut debt = 0;
        while now >= last + i64::from(interval) {
            sync_calls += 1;
            // Sync may return before worker completion. The main thread still
            // bounds its own catch-up attempts without claiming work finished.
            assert_eq!(room_updates, 0);
            let stop = stop_after_sync(sync_calls, 0, 0, 5_000_000);
            let pre_sync_last = last;
            if stop {
                let adjustment =
                    clamp_after_sync(now, pre_sync_last, interval, interval, true, true).unwrap();
                debt += adjustment.dropped_intervals;
                last = adjustment.native_last;
                break;
            }
            last = pre_sync_last + i64::from(interval);
        }
        assert_eq!(sync_calls, MAX_SYNC_CALLS);
        assert_eq!(room_updates, 0);
        assert_eq!(debt, 4);
        assert_eq!(now - debt as i64 * i64::from(interval), last + 7);
    }

    #[test]
    fn false_sync_return_keeps_native_last_untouched() {
        let last_before = 900_000;
        let interval = 50_000;
        assert_eq!(
            clamp_after_sync(
                last_before + 9 * i64::from(interval),
                last_before,
                interval,
                interval,
                false,
                true,
            ),
            None,
            "CGame::Step skips its cached last write on a false Sync result"
        );
        assert_eq!(last_before, 900_000);
    }

    #[test]
    fn sync_clamp_uses_pre_sync_native_last_across_interval_changes() {
        let last_before = 1_000_000;
        for (pre_interval, post_interval, remainder, dropped) in [
            (200_000, 50_000, 12_345, 5),
            (66_666, 66_667, 17, 2),
            (66_667, 66_666, 34, 3),
        ] {
            let native_last = last_before + i64::from(pre_interval);
            let now = native_last + i64::from(post_interval) * dropped + remainder;
            let adjustment =
                clamp_after_sync(now, last_before, pre_interval, post_interval, true, true)
                    .unwrap();
            assert_eq!(adjustment.native_last, native_last);
            assert_eq!(adjustment.dropped_intervals, dropped as u64);
            assert_eq!(adjustment.now - native_last, remainder);
        }
    }
}

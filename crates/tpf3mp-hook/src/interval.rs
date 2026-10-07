//! The game's batch interval: each batch of room updates shown over a time
//! in proportion to its updates, so vehicles move at one speed (docs/HOOKS.md,
//! "The batch interval").
//!
//! `CGame::Step` (main thread) hands the simulation thread a batch through
//! `CGame::Sync` every `guiFrameTime` microseconds and interpolates between
//! the last two finished batches by `(totalTime - lastSyncTime) /
//! guiFrameTime`. A batch of 3 updates shown over the same 200 ms as one of 5
//! moves vehicles at 60 % of the speed. Right after a successful Sync the
//! batch just exposed has finished, and its update count is known: the hook
//! then sets `guiFrameTime` to that count times the room's time per step, so
//! every batch moves at the room's speed whatever its count (TPF2MP set the
//! same field, `SyncWrap` in tpf2-multiplayer/native/src/speedhook.cpp;
//! writing it anywhere else made the render clock step back and crashed).
//!
//! The step detour (simulation thread) publishes each batch's outcome into
//! [`Slots`], keyed by the `GameSim` it stepped; the Sync wrapper (main
//! thread) reads the slot of the `GameSim` Sync just exposed. The two never
//! wait for each other. What is written is checked first ([`Layout`],
//! [`Interval::after_sync`]); anything unexpected turns this off for the
//! process: the game then paces itself, as without it.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

/// Set to `0` (or `off`) in the game's environment, the hook never writes
/// the batch interval.
pub const ENV: &str = "TPF3MP_HOOK_BATCH_INTERVAL";

/// The shortest interval written (TPF2MP's floor too).
pub const MIN_US: u32 = 20_000;
/// The longest interval written: longer would look like a frozen game.
pub const MAX_US: u32 = 2_000_000;
/// A running room's batch without updates (actions, a reserve kept, a step
/// not released yet) is shown this long: vehicles stand for 50 ms, not a
/// whole call.
pub const ZERO_US: u32 = 50_000;
/// The game's own batch: 200 ms, which `RunGameSimLoop` also hands every
/// update as its time step.
pub const BASE: Duration = Duration::from_millis(200);

/// The trim's bounds: throughput at most 25 % above or below the room's
/// pace, far inside the server's pacing window.
pub const TRIM_MIN: f64 = 0.8;
pub const TRIM_MAX: f64 = 1.25;
/// The most the trim moves per batch.
pub const TRIM_SLEW: f64 = 0.05;
/// The trim's gain, per step outside the band per nominal count.
pub const TRIM_GAIN: f64 = 0.25;

/// Whether [`ENV`]'s value leaves the batch interval on: anything but an
/// explicit no.
pub fn wanted(value: Option<&str>) -> bool {
    crate::cadence::wanted(value)
}

/// Where build 40408 keeps what the wrapper reads and writes, verified in
/// its disassembly (2026-10-08): `CGame::Step` 0x11f3b0, `CGame::Sync`
/// 0x11f650, `CGame::RunGameSimLoop` 0x11e210 (which passes
/// `[m_data + 0x88 + simIdx * 8]` to `GameSim::Step`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// `CGame::m_data`.
    pub m_data: usize,
    /// `m_data.gameSims[2]`, pointers.
    pub game_sims: usize,
    /// `m_data.simIdx`, the simulation's buffer, flipped by each Sync.
    pub sim_idx: usize,
    /// `m_data.totalTime`, i64 µs.
    pub total_time: usize,
    /// `m_data.lastSyncTime`, i64 µs.
    pub last_sync: usize,
    /// `m_data.guiFrameTime`, i32 µs.
    pub gui_frame_time: usize,
}

/// Build 40408's layout (TPF2 35924's fields are 0x78 lower: `m_data` at
/// `CGame + 0x168`, `guiFrameTime` at `+0x1a0`).
pub const LAYOUT_40408: Layout = Layout {
    m_data: 0x1f0,
    game_sims: 0x88,
    sim_idx: 0x98,
    total_time: 0x1a0,
    last_sync: 0x1a8,
    gui_frame_time: 0x218,
};

/// The layout of the build a profile names, where it was verified.
pub fn layout_for(profile: &str) -> Option<Layout> {
    profile.contains("Build 40408").then_some(LAYOUT_40408)
}

/// The updates a call runs at the room's pace with the game's own 200 ms
/// batch: 1, 2, 4 at 1x, 2x, 4x and 5 steps a second; at least 1.
pub fn nominal(pace: f64) -> u32 {
    let count = (pace * BASE.as_secs_f64()).round();
    if count.is_finite() && count >= 1.0 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let count = count.min(f64::from(crate::step::MAX_STEPS_PER_CALL)) as u32;
        count
    } else {
        1
    }
}

/// The updates a batch is shown as: its own count up to half again the
/// nominal one; above (a catch-up, everything before an action) capped, so
/// the excess shows as faster motion instead of slowing the catch-up.
pub fn shown(count: u32, nominal: u32) -> u32 {
    let cap = nominal.saturating_mul(3).div_ceil(2).max(1);
    count.min(cap)
}

/// The interval for a running room's batch of `count` updates, with
/// `step_us` the room's time per step and `trim` the throughput correction.
pub fn interval_us(count: u32, nominal: u32, step_us: u32, trim: f64) -> u32 {
    if count == 0 {
        return ZERO_US;
    }
    let us = f64::from(shown(count, nominal)) * f64::from(step_us) * trim;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let us = us.round().clamp(f64::from(MIN_US), f64::from(MAX_US)) as u32;
    us
}

/// The throughput correction: below 1 the game calls a little faster than
/// the room's pace, above 1 a little slower. Moved by the backlog after each
/// batch (the steps released and not run), against crate::cadence's band
/// (its reserve up to the reserve plus its band).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trim {
    value: f64,
}

impl Default for Trim {
    fn default() -> Self {
        Self { value: 1.0 }
    }
}

impl Trim {
    pub fn value(&self) -> f64 {
        self.value
    }

    pub fn reset(&mut self) {
        self.value = 1.0;
    }

    /// After a batch with updates: `backlog` steps released and not run,
    /// `nominal` updates a call.
    pub fn update(&mut self, backlog: u64, nominal: u32) {
        let low = crate::cadence::RESERVE;
        let high = crate::cadence::RESERVE + crate::cadence::BAND;
        #[allow(clippy::cast_precision_loss)]
        let error = if backlog > high {
            (backlog - high) as f64
        } else if backlog < low {
            -((low - backlog) as f64)
        } else {
            0.0
        };
        let want = (1.0 - TRIM_GAIN * error / f64::from(nominal.max(1))).clamp(TRIM_MIN, TRIM_MAX);
        let step = (want - self.value).clamp(-TRIM_SLEW, TRIM_SLEW);
        self.value = (self.value + step).clamp(TRIM_MIN, TRIM_MAX);
    }
}

/// What the step driver says about the call it just answered
/// ([`crate::step::StepDriver::batch_info`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatchInfo {
    pub eligible: bool,
    pub step_us: u32,
    pub nominal: u32,
    pub backlog: u32,
    pub steady: bool,
}

impl BatchInfo {
    /// The record of the batch `count` updates long that `sim` ran.
    pub fn record(&self, sim: usize, count: u32) -> Record {
        Record {
            sim,
            epoch: SLOTS.epoch(),
            count,
            eligible: self.eligible,
            step_us: self.step_us,
            nominal: self.nominal,
            backlog: self.backlog,
            steady: self.steady,
        }
    }
}

/// What the step detour publishes about one batch it ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    /// The `GameSim` it stepped (`this` of `GameSim::Step`).
    pub sim: usize,
    /// The world it ran in ([`Slots::next_epoch`]).
    pub epoch: u64,
    /// The updates it really ran.
    pub count: u32,
    /// A running room's batch that may be timed: the driver runs the room's
    /// game (not holding), even steps and the interval are on, not paused.
    pub eligible: bool,
    /// The room's time per step, µs.
    pub step_us: u32,
    /// The nominal count.
    pub nominal: u32,
    /// Steps released and not run after the batch.
    pub backlog: u32,
    /// The count was the steady one (crate::cadence's `Nominal`): only then
    /// does the backlog say anything about the pace; before an action, the
    /// player's own command, a pause or a catch-up it is emptied or filled
    /// on purpose, and the trim holds.
    pub steady: bool,
}

impl Slot {
    const fn new() -> Self {
        Self {
            seq: AtomicU64::new(0),
            sim: AtomicUsize::new(0),
            epoch: AtomicU64::new(0),
            count: AtomicU32::new(0),
            eligible: AtomicBool::new(false),
            step_us: AtomicU32::new(0),
            nominal: AtomicU32::new(0),
            backlog: AtomicU32::new(0),
            steady: AtomicBool::new(false),
        }
    }
}

#[derive(Debug, Default)]
struct Slot {
    /// Even when stable, odd while being written.
    seq: AtomicU64,
    sim: AtomicUsize,
    epoch: AtomicU64,
    count: AtomicU32,
    eligible: AtomicBool,
    step_us: AtomicU32,
    nominal: AtomicU32,
    backlog: AtomicU32,
    steady: AtomicBool,
}

/// Two published batches, one per `GameSim` (the game double-buffers its
/// simulation: the batch Sync exposes ran on one, the next runs on the
/// other, and the same one cannot run again before the next Sync).
#[derive(Debug, Default)]
pub struct Slots {
    slots: [Slot; 2],
    epoch: AtomicU64,
}

impl Slots {
    pub const fn new() -> Self {
        Self {
            slots: [Slot::new(), Slot::new()],
            epoch: AtomicU64::new(0),
        }
    }

    /// The world the batches run in now.
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// A new world (a begin, a load, a hold, a leave): nothing published
    /// before times anything after.
    pub fn next_epoch(&self) -> u64 {
        self.epoch.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// From the step detour, after the batch ran.
    pub fn publish(&self, record: &Record) {
        let index = if self.slots[0].sim.load(Ordering::Acquire) == record.sim {
            0
        } else if self.slots[1].sim.load(Ordering::Acquire) == record.sim {
            1
        } else if self.slots[0].seq.load(Ordering::Acquire)
            <= self.slots[1].seq.load(Ordering::Acquire)
        {
            0
        } else {
            1
        };
        let slot = &self.slots[index];
        let begin = slot.seq.load(Ordering::Relaxed) | 1;
        slot.seq.store(begin, Ordering::Release);
        slot.sim.store(record.sim, Ordering::Relaxed);
        slot.epoch.store(record.epoch, Ordering::Relaxed);
        slot.count.store(record.count, Ordering::Relaxed);
        slot.eligible.store(record.eligible, Ordering::Relaxed);
        slot.step_us.store(record.step_us, Ordering::Relaxed);
        slot.nominal.store(record.nominal, Ordering::Relaxed);
        slot.backlog.store(record.backlog, Ordering::Relaxed);
        slot.steady.store(record.steady, Ordering::Relaxed);
        slot.seq.store(begin + 1, Ordering::Release);
    }

    /// From the Sync wrapper: the batch `sim` ran last, in the current
    /// world, if one was published whole and not being rewritten.
    pub fn read(&self, sim: usize) -> Option<Record> {
        if sim == 0 {
            return None;
        }
        let epoch = self.epoch();
        let slot = self
            .slots
            .iter()
            .find(|slot| slot.sim.load(Ordering::Acquire) == sim)?;
        let before = slot.seq.load(Ordering::Acquire);
        if before & 1 == 1 || before == 0 {
            return None;
        }
        let record = Record {
            sim: slot.sim.load(Ordering::Relaxed),
            epoch: slot.epoch.load(Ordering::Relaxed),
            count: slot.count.load(Ordering::Relaxed),
            eligible: slot.eligible.load(Ordering::Relaxed),
            step_us: slot.step_us.load(Ordering::Relaxed),
            nominal: slot.nominal.load(Ordering::Relaxed),
            backlog: slot.backlog.load(Ordering::Relaxed),
            steady: slot.steady.load(Ordering::Relaxed),
        };
        std::sync::atomic::fence(Ordering::Acquire);
        let after = slot.seq.load(Ordering::Relaxed);
        (before == after && record.sim == sim && record.epoch == epoch).then_some(record)
    }
}

/// What a successful Sync's wrapper read of the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seen {
    /// `m_data`.
    pub m_data: usize,
    /// The `GameSim` whose batch Sync just exposed: `gameSims[1 - simIdx]`.
    pub exposed: usize,
    /// The interval Sync just wrote (the game's own estimate).
    pub game_us: i32,
    pub total_time: i64,
    pub last_sync: i64,
}

/// Why nothing was written after a Sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// No published batch for the exposed `GameSim`, or of another world.
    NoRecord,
    /// The batch may not be timed (not a running room's, paused, off).
    NotEligible,
}

/// Why the interval was turned off for the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// `m_data` null or misaligned.
    MData,
    /// `simIdx` not 0 or 1.
    SimIdx,
    /// The interval Sync wrote is outside what the game writes.
    GameValue(i32),
    /// The clock is inconsistent (`totalTime < lastSyncTime`).
    Clock,
    /// The field is not writable memory.
    NotWritable,
}

/// The wrapper's state across Syncs (main thread only).
#[derive(Debug, Default)]
pub struct Interval {
    trim: Trim,
    /// The last interval written, µs.
    last: Option<u32>,
    /// The `m_data` and world whose field was found writable.
    writable_for: Option<(usize, u64)>,
}

impl Interval {
    pub const fn new() -> Self {
        Self {
            trim: Trim { value: 1.0 },
            last: None,
            writable_for: None,
        }
    }

    pub fn trim(&self) -> f64 {
        self.trim.value()
    }

    /// After a successful Sync, with what it read of the game: the interval
    /// to write, or why not. Checks the game's state first; a fault turns
    /// the interval off for the process.
    pub fn after_sync(
        &mut self,
        seen: &Seen,
        sim_idx: i32,
        record: Option<Record>,
    ) -> Result<Result<u32, Skip>, Fault> {
        if seen.m_data == 0 || !seen.m_data.is_multiple_of(8) {
            return Err(Fault::MData);
        }
        if !(0..=1).contains(&sim_idx) {
            return Err(Fault::SimIdx);
        }
        // CalcGuiFrameTime writes 200..400 ms; our own last write survives
        // only until Sync overwrites it, so anything else is a misread.
        if !(i64::from(MIN_US)..=i64::from(MAX_US)).contains(&i64::from(seen.game_us)) {
            return Err(Fault::GameValue(seen.game_us));
        }
        if seen.total_time < seen.last_sync {
            return Err(Fault::Clock);
        }
        let Some(record) = record else {
            self.trim.reset();
            return Ok(Err(Skip::NoRecord));
        };
        if !record.eligible || record.step_us == 0 {
            self.trim.reset();
            return Ok(Err(Skip::NotEligible));
        }
        if record.count > 0 && record.steady {
            self.trim.update(u64::from(record.backlog), record.nominal);
        }
        let us = interval_us(
            record.count,
            record.nominal,
            record.step_us,
            self.trim.value(),
        );
        self.last = Some(us);
        Ok(Ok(us))
    }
}

/// The batches the step detour published (simulation thread) for the Sync
/// wrapper (main thread).
pub static SLOTS: Slots = Slots::new();

/// Off for the process: the knob, a profile without the layout, or a fault.
static OFF: AtomicBool = AtomicBool::new(true);
/// The wrapper's state (only the main thread, in the wrapper, takes it).
static STATE: std::sync::Mutex<Interval> = std::sync::Mutex::new(Interval::new());
/// The game's layout, set once when the interval is armed.
static LAYOUT: std::sync::OnceLock<Layout> = std::sync::OnceLock::new();
/// Lines for the hook's log, taken by the step detour (the wrapper never
/// waits for the log's lock the detour holds).
static LINES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
/// Counts since the last [`take_counts`]: intervals written, Syncs with
/// nothing to write, Syncs that returned false.
static WRITES: AtomicU64 = AtomicU64::new(0);
static SKIPS: AtomicU64 = AtomicU64::new(0);
static FALSE_SYNCS: AtomicU64 = AtomicU64::new(0);

/// Arms the interval for a game with `layout`: from here the wrapper
/// writes, until a fault.
pub fn arm(layout: Layout) {
    let _ = LAYOUT.set(layout);
    OFF.store(false, Ordering::Release);
}

/// Whether the wrapper may write.
pub fn armed() -> bool {
    !OFF.load(Ordering::Acquire)
}

/// The lines the wrapper left for the hook's log.
pub fn take_lines() -> Vec<String> {
    match LINES.try_lock() {
        Ok(mut lines) => std::mem::take(&mut *lines),
        Err(_) => Vec::new(),
    }
}

/// Intervals written, Syncs with nothing to write, Syncs that returned
/// false, since the last call.
pub fn take_counts() -> (u64, u64, u64) {
    (
        WRITES.swap(0, Ordering::Relaxed),
        SKIPS.swap(0, Ordering::Relaxed),
        FALSE_SYNCS.swap(0, Ordering::Relaxed),
    )
}

static MISMATCH_SAID: AtomicBool = AtomicBool::new(false);

/// A room's call of the game's step ran other updates than it answered
/// (the game's pending debug steps): said once; such a batch is not timed.
pub fn note_mismatch(answered: u32, ran: u32) {
    if !MISMATCH_SAID.swap(true, Ordering::AcqRel) {
        say(format!(
            "batch interval: a call of the game's step answered {answered} updates but ran {ran} (the game's own pending steps?); such batches are not timed"
        ));
    }
}

fn say(line: String) {
    if let Ok(mut lines) = LINES.lock()
        && lines.len() < 1000
    {
        lines.push(line);
    }
}

/// What one Sync's wrapper did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Done {
    /// Sync returned false: nothing read, nothing written.
    FalseSync,
    /// Nothing to time ([`Skip`]).
    Skipped(Skip),
    /// The interval written, µs, and the batch it times.
    Wrote(u32, Record),
    /// The game is not what the layout says: off for good, and why.
    Off(String),
}

/// One Sync's work, on the given layout, slots and state: reads the game
/// behind `cgame`, checks it, and writes the interval for the batch Sync
/// exposed. Pure but for those reads and the one write.
pub fn sync_in(
    layout: &Layout,
    slots: &Slots,
    state: &mut Interval,
    cgame: usize,
    ok: bool,
) -> Done {
    if !ok {
        return Done::FalseSync;
    }
    let mut reader = crate::image::Readable::new();
    let mut read_u64 =
        |at: usize, off: usize| -> Option<u64> { reader.read::<u64>(at.checked_add(off)? as u64) };
    let m_data = read_u64(cgame, layout.m_data).unwrap_or(0) as usize;
    let mut reader = crate::image::Readable::new();
    let field = |off: usize| -> Option<usize> { m_data.checked_add(off) };
    let sim_idx = match field(layout.sim_idx) {
        Some(at) if m_data != 0 => reader.read::<i32>(at as u64).unwrap_or(-1),
        _ => -1,
    };
    let exposed = if (0..=1).contains(&sim_idx) {
        #[allow(clippy::cast_sign_loss)]
        let other = (1 - sim_idx) as usize;
        field(layout.game_sims + other * 8)
            .and_then(|at| reader.read::<u64>(at as u64))
            .unwrap_or(0) as usize
    } else {
        0
    };
    let seen = Seen {
        m_data,
        exposed,
        game_us: field(layout.gui_frame_time)
            .and_then(|at| reader.read::<i32>(at as u64))
            .unwrap_or(-1),
        total_time: field(layout.total_time)
            .and_then(|at| reader.read::<i64>(at as u64))
            .unwrap_or(-1),
        last_sync: field(layout.last_sync)
            .and_then(|at| reader.read::<i64>(at as u64))
            .unwrap_or(i64::MAX),
    };
    let record = slots.read(exposed);
    match state.after_sync(&seen, sim_idx, record) {
        Err(fault) => Done::Off(format!(
            "the game's state is not what it should be ({fault:?})"
        )),
        Ok(Err(skip)) => Done::Skipped(skip),
        Ok(Ok(us)) => {
            let Some(at) = field(layout.gui_frame_time) else {
                return Done::Off("guiFrameTime's address overflows".into());
            };
            let epoch = slots.epoch();
            if state.writable_for != Some((m_data, epoch)) {
                if !writable(at, 4) {
                    return Done::Off("guiFrameTime is not writable memory".into());
                }
                state.writable_for = Some((m_data, epoch));
            }
            write_i32(at, us);
            match record {
                Some(record) => Done::Wrote(us, record),
                None => Done::Skipped(Skip::NoRecord),
            }
        }
    }
}

/// From the Sync wrapper, after the real `CGame::Sync` returned `ok`.
/// Never panics out (the caller is the game's main thread).
pub fn after_sync(cgame: usize, ok: bool) {
    if !ok {
        FALSE_SYNCS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    if !armed() {
        return;
    }
    let Some(layout) = LAYOUT.get().copied() else {
        return;
    };
    let Ok(mut state) = STATE.lock() else {
        return;
    };
    match sync_in(&layout, &SLOTS, &mut state, cgame, ok) {
        Done::FalseSync => {}
        Done::Skipped(_) => {
            SKIPS.fetch_add(1, Ordering::Relaxed);
        }
        Done::Off(why) => {
            OFF.store(true, Ordering::Release);
            say(format!(
                "batch interval: off for good, {why}; the game paces itself"
            ));
        }
        Done::Wrote(us, record) => {
            WRITES.fetch_add(1, Ordering::Relaxed);
            if crate::steptrace::enabled() {
                say(format!(
                    "interval: n={} us={us} trim={:.2} backlog={}",
                    record.count,
                    state.trim(),
                    record.backlog,
                ));
            }
        }
    }
}

/// Writes the interval: an aligned i32 the checks above found writable.
#[allow(unsafe_code)]
fn write_i32(at: usize, us: u32) {
    let value = i32::try_from(us).unwrap_or(i32::MAX);
    // SAFETY: `at` is `m_data + guiFrameTime` of the build's verified
    // layout, 4-aligned (m_data is 8-aligned, the offset 4-aligned), in
    // committed writable memory checked for this m_data; the main thread
    // owns the field between Syncs (CGame::Step reads it right after).
    unsafe { std::ptr::write_volatile(at as *mut i32, value) };
}

/// Whether `len` bytes at `address` are committed memory this process may
/// write now (a read proves nothing about writing).
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn writable(address: usize, len: usize) -> bool {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READWRITE, PAGE_GUARD, PAGE_NOACCESS,
        PAGE_READWRITE, VirtualQuery,
    };
    let Some(end) = address.checked_add(len) else {
        return false;
    };
    let mut at = address;
    while at < end {
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: VirtualQuery only reads the address space's description
        // into `info`, whose size it is given.
        let got = unsafe {
            VirtualQuery(
                at as *const _,
                &mut info,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if got == 0 || info.State != MEM_COMMIT {
            return false;
        }
        let protect = info.Protect;
        if protect & (PAGE_GUARD | PAGE_NOACCESS) != 0
            || protect & (PAGE_READWRITE | PAGE_EXECUTE_READWRITE) == 0
        {
            return false;
        }
        let region_end = (info.BaseAddress as usize).saturating_add(info.RegionSize);
        if region_end <= at {
            return false;
        }
        at = region_end;
    }
    true
}

#[cfg(not(windows))]
pub fn writable(_address: usize, _len: usize) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nominal_count_is_the_rooms_pace_over_the_games_batch() {
        assert_eq!(nominal(5.0), 1);
        assert_eq!(nominal(10.0), 2);
        assert_eq!(nominal(20.0), 4);
        assert_eq!(nominal(7.0), 1);
        assert_eq!(nominal(0.0), 1);
        assert_eq!(nominal(f64::NAN), 1);
        assert_eq!(nominal(1000.0), crate::step::MAX_STEPS_PER_CALL);
    }

    #[test]
    fn every_batch_moves_at_the_rooms_speed_up_to_half_again_its_count() {
        // 4x: 50 ms a step. 3, 4, 5 and 6 updates are shown proportionally.
        assert_eq!(interval_us(3, 4, 50_000, 1.0), 150_000);
        assert_eq!(interval_us(4, 4, 50_000, 1.0), 200_000);
        assert_eq!(interval_us(6, 4, 50_000, 1.0), 300_000);
        // A catch-up of 16 is shown as 6: faster motion, not a slow one.
        assert_eq!(interval_us(16, 4, 50_000, 1.0), 300_000);
        // 1x: 2 at most (ceil 1.5).
        assert_eq!(interval_us(16, 1, 200_000, 1.0), 400_000);
        // No updates: a short stand.
        assert_eq!(interval_us(0, 4, 50_000, 1.0), ZERO_US);
        // Bounds.
        assert_eq!(interval_us(1, 16, 1_000, 0.8), MIN_US);
        assert_eq!(interval_us(16, 16, 200_000, 1.25), MAX_US);
    }

    #[test]
    fn the_trim_moves_slowly_only_outside_the_band_and_stays_bounded() {
        let mut trim = Trim::default();
        trim.update(2, 4);
        assert_eq!(trim.value(), 1.0, "in the band");
        trim.update(13, 4); // 10 over: want 1 - 0.25*10/4 = 0.375 -> 0.8
        assert!((trim.value() - 0.95).abs() < 1e-9, "slewed");
        for _ in 0..20 {
            trim.update(13, 4);
        }
        assert!((trim.value() - TRIM_MIN).abs() < 1e-9);
        for _ in 0..40 {
            trim.update(0, 1);
        }
        assert!((trim.value() - TRIM_MAX).abs() < 1e-9);
    }

    fn record(sim: usize, epoch: u64, count: u32) -> Record {
        Record {
            sim,
            epoch,
            count,
            eligible: true,
            step_us: 50_000,
            nominal: 4,
            backlog: 2,
            steady: true,
        }
    }

    #[test]
    fn a_sync_reads_the_batch_of_the_simulation_it_exposed_never_the_next() {
        let slots = Slots::new();
        let epoch = slots.next_epoch();
        slots.publish(&record(0x1000, epoch, 3));
        // The next batch, on the other simulation, finishes before the
        // wrapper reads: it lands in the other slot.
        slots.publish(&record(0x2000, epoch, 5));
        assert_eq!(slots.read(0x1000).unwrap().count, 3);
        assert_eq!(slots.read(0x2000).unwrap().count, 5);
        // The same simulation's next batch replaces its own slot only.
        slots.publish(&record(0x1000, epoch, 4));
        assert_eq!(slots.read(0x1000).unwrap().count, 4);
        assert_eq!(slots.read(0x2000).unwrap().count, 5);
        assert_eq!(slots.read(0x3000), None, "no slot");
        assert_eq!(slots.read(0), None);
    }

    #[test]
    fn a_new_world_drops_every_batch_published_before() {
        let slots = Slots::new();
        let epoch = slots.next_epoch();
        slots.publish(&record(0x1000, epoch, 4));
        slots.next_epoch();
        assert_eq!(slots.read(0x1000), None);
        // New GameSims of a loaded world take the slots over.
        let epoch = slots.epoch();
        slots.publish(&record(0x5000, epoch, 4));
        slots.publish(&record(0x6000, epoch, 4));
        assert_eq!(slots.read(0x5000).unwrap().count, 4);
        assert_eq!(slots.read(0x1000), None);
    }

    /// `CGame::Step`'s loop (0x11f3b0) and what the screen shows: each
    /// frame adds `dt`; while `totalTime` reached the next sync, Sync runs
    /// (it may fail) and the hook writes the interval for the batch it
    /// exposed; the screen interpolates across that batch by alpha.
    struct GameLoop {
        total: i64,
        last_sync: i64,
        gft: i64,
        /// Room updates on screen before the batch being interpolated.
        before: f64,
        /// The batch being interpolated: its updates.
        shown_batch: u32,
        /// Batches finished and not shown yet, in order.
        finished: std::collections::VecDeque<u32>,
    }

    impl GameLoop {
        /// One frame: returns the interpolated room updates on screen.
        fn frame(&mut self, dt: i64, sync_ok: &mut dyn FnMut() -> bool, step_us: u32) -> f64 {
            self.total += dt;
            while self.total >= self.last_sync + self.gft {
                let next = self.last_sync + self.gft;
                if !sync_ok() {
                    break;
                }
                self.last_sync = next;
                // The batch Sync exposes starts showing; the hook times it.
                self.before += f64::from(self.shown_batch);
                self.shown_batch = self.finished.pop_front().unwrap_or(0);
                self.gft = i64::from(interval_us(self.shown_batch, 4, step_us, 1.0));
            }
            self.total = self.total.min(self.last_sync + self.gft - 1);
            assert!(self.last_sync <= self.total && self.total < self.last_sync + self.gft);
            #[allow(clippy::cast_precision_loss)]
            let alpha = (self.total - self.last_sync) as f32 / self.gft as f32;
            assert!((0.0..1.0).contains(&alpha), "alpha {alpha}");
            self.before + f64::from(alpha) * f64::from(self.shown_batch)
        }
    }

    #[test]
    fn the_games_loop_with_our_interval_shows_every_batch_at_one_speed() {
        // 4x: 50 ms a step; batches of 4 with checkpoint cuts and a catch-up.
        let batches = [4, 4, 3, 3, 5, 4, 4, 2, 6, 4, 0, 5, 4, 4, 16, 4, 4];
        let mut game = GameLoop {
            total: 0,
            last_sync: 0,
            gft: 200_000,
            before: 0.0,
            shown_batch: 0,
            finished: batches.iter().copied().collect(),
        };
        let mut ok = || true;
        let mut last = 0.0;
        let mut speeds = Vec::new();
        // 60 fps, with a long frame (a save) in the middle.
        for frame in 0..400 {
            let dt = if frame == 200 { 1_200_000 } else { 16_667 };
            let shown = game.frame(dt, &mut ok, 50_000);
            assert!(
                shown + 1e-9 >= last,
                "the screen went back at frame {frame}"
            );
            if dt == 16_667 && shown > last {
                speeds.push((shown - last) / 16_667.0 * 50_000.0);
            }
            last = shown;
        }
        // Updates per step time on screen: 1 for every proportional batch
        // (the catch-up of 16 shows faster, the zero batch stands).
        let even = speeds.iter().filter(|s| (**s - 1.0).abs() < 0.02).count();
        assert!(even * 10 >= speeds.len() * 8, "{speeds:?}");
    }

    #[test]
    fn a_failed_sync_keeps_the_games_clock_consistent() {
        let mut game = GameLoop {
            total: 0,
            last_sync: 0,
            gft: 200_000,
            before: 0.0,
            shown_batch: 0,
            finished: [3, 5, 4, 4].into_iter().collect(),
        };
        let mut calls = 0;
        let mut flaky = || {
            calls += 1;
            calls % 3 != 0
        };
        let mut last = 0.0;
        for _ in 0..200 {
            let shown = game.frame(16_667, &mut flaky, 50_000);
            assert!(shown + 1e-9 >= last);
            last = shown;
        }
    }

    fn seen() -> Seen {
        Seen {
            m_data: 0x10_0000,
            exposed: 0x1000,
            game_us: 200_000,
            total_time: 1_000_000,
            last_sync: 900_000,
        }
    }

    #[test]
    fn the_interval_follows_the_exposed_batch_and_a_misread_turns_it_off() {
        let mut interval = Interval::new();
        let rec = record(0x1000, 1, 3);
        assert_eq!(interval.after_sync(&seen(), 0, Some(rec)), Ok(Ok(150_000)));
        assert_eq!(
            interval.after_sync(&seen(), 0, None),
            Ok(Err(Skip::NoRecord))
        );
        // Before an action the backlog is emptied on purpose: the trim
        // holds whatever it reads.
        let before = interval.trim();
        let mut drained = record(0x1000, 1, 6);
        drained.backlog = 0;
        drained.steady = false;
        assert_eq!(
            interval.after_sync(&seen(), 0, Some(drained)),
            Ok(Ok(300_000))
        );
        assert!((interval.trim() - before).abs() < 1e-12);
        let mut paused = rec;
        paused.eligible = false;
        assert_eq!(
            interval.after_sync(&seen(), 0, Some(paused)),
            Ok(Err(Skip::NotEligible))
        );
        let mut bad = seen();
        bad.m_data = 0x10_0004;
        assert_eq!(interval.after_sync(&bad, 0, Some(rec)), Err(Fault::MData));
        assert_eq!(
            interval.after_sync(&seen(), 2, Some(rec)),
            Err(Fault::SimIdx)
        );
        let mut bad = seen();
        bad.game_us = 7;
        assert_eq!(
            interval.after_sync(&bad, 0, Some(rec)),
            Err(Fault::GameValue(7))
        );
        let mut bad = seen();
        bad.total_time = 1;
        assert_eq!(interval.after_sync(&bad, 0, Some(rec)), Err(Fault::Clock));
    }
}

/// The wrapper's work on memory laid out as build 40408's: what it reads,
/// what it writes, and what it leaves alone.
#[cfg(all(test, windows))]
#[allow(unsafe_code)]
mod native {
    use super::*;

    /// A fake `CGame` and its `m_data`, 8-aligned, in this process.
    struct Game {
        cgame: Vec<u64>,
        m_data: Vec<u64>,
    }

    impl Game {
        fn new(sims: [usize; 2], sim_idx: i32) -> Self {
            let mut game = Self {
                cgame: vec![0; 0x200 / 8 + 1],
                m_data: vec![0; 0x300 / 8],
            };
            game.cgame[0x1f0 / 8] = game.m_data.as_ptr() as u64;
            game.put_u64(0x88, sims[0] as u64);
            game.put_u64(0x90, sims[1] as u64);
            game.put_i32(0x98, sim_idx);
            game.put_i64(0x1a0, 1_000_000);
            game.put_i64(0x1a8, 900_000);
            game.put_i32(0x214, 0x1111_1111);
            game.put_i32(0x218, 200_000);
            game.put_i32(0x21c, 0x2222_2222);
            game
        }
        fn at(&self, off: usize) -> *mut u8 {
            (self.m_data.as_ptr() as usize + off) as *mut u8
        }
        fn put_u64(&mut self, off: usize, v: u64) {
            unsafe { std::ptr::write_unaligned(self.at(off).cast::<u64>(), v) }
        }
        fn put_i64(&mut self, off: usize, v: i64) {
            unsafe { std::ptr::write_unaligned(self.at(off).cast::<i64>(), v) }
        }
        fn put_i32(&mut self, off: usize, v: i32) {
            unsafe { std::ptr::write_unaligned(self.at(off).cast::<i32>(), v) }
        }
        fn get_i32(&self, off: usize) -> i32 {
            unsafe { std::ptr::read_unaligned(self.at(off).cast::<i32>()) }
        }
        fn cgame(&self) -> usize {
            self.cgame.as_ptr() as usize
        }
    }

    fn published(slots: &Slots, sim: usize, count: u32) {
        slots.publish(&Record {
            sim,
            epoch: slots.epoch(),
            count,
            eligible: true,
            step_us: 50_000,
            nominal: 4,
            backlog: 2,
            steady: true,
        });
    }

    #[test]
    fn a_true_sync_writes_the_exposed_batchs_interval_and_nothing_beside_it() {
        // Sync flipped simIdx to 1: the exposed batch ran on gameSims[0].
        let game = Game::new([0xA000, 0xB000], 1);
        let slots = Slots::new();
        slots.next_epoch();
        published(&slots, 0xA000, 3);
        published(&slots, 0xB000, 5);
        let mut state = Interval::new();
        let done = sync_in(&LAYOUT_40408, &slots, &mut state, game.cgame(), true);
        assert!(matches!(done, Done::Wrote(150_000, _)), "{done:?}");
        assert_eq!(game.get_i32(0x218), 150_000);
        assert_eq!(game.get_i32(0x214), 0x1111_1111);
        assert_eq!(game.get_i32(0x21c), 0x2222_2222);
    }

    #[test]
    fn a_false_sync_or_nothing_published_writes_nothing() {
        let game = Game::new([0xA000, 0xB000], 1);
        let slots = Slots::new();
        slots.next_epoch();
        let mut state = Interval::new();
        assert_eq!(
            sync_in(&LAYOUT_40408, &slots, &mut state, game.cgame(), false),
            Done::FalseSync
        );
        assert_eq!(
            sync_in(&LAYOUT_40408, &slots, &mut state, game.cgame(), true),
            Done::Skipped(Skip::NoRecord)
        );
        // Published in a world since replaced.
        published(&slots, 0xA000, 4);
        slots.next_epoch();
        assert_eq!(
            sync_in(&LAYOUT_40408, &slots, &mut state, game.cgame(), true),
            Done::Skipped(Skip::NoRecord)
        );
        assert_eq!(game.get_i32(0x218), 200_000);
    }

    #[test]
    fn a_game_not_laid_out_as_expected_turns_the_interval_off() {
        let slots = Slots::new();
        slots.next_epoch();
        published(&slots, 0xA000, 4);
        let mut state = Interval::new();
        let game = Game::new([0xA000, 0xB000], 7);
        assert!(matches!(
            sync_in(&LAYOUT_40408, &slots, &mut state, game.cgame(), true),
            Done::Off(_)
        ));
        let mut game = Game::new([0xA000, 0xB000], 1);
        game.put_i32(0x218, 5);
        assert!(matches!(
            sync_in(&LAYOUT_40408, &slots, &mut state, game.cgame(), true),
            Done::Off(_)
        ));
        assert_eq!(game.get_i32(0x218), 5, "never written");
        let mut game = Game::new([0xA000, 0xB000], 1);
        game.cgame[0x1f0 / 8] = 0;
        assert!(matches!(
            sync_in(&LAYOUT_40408, &slots, &mut state, game.cgame(), true),
            Done::Off(_)
        ));
    }

    #[test]
    fn read_only_memory_is_never_written() {
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READONLY, PAGE_READWRITE, VirtualAlloc,
            VirtualFree, VirtualProtect,
        };
        let page = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                0x1000,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        } as usize;
        assert_ne!(page, 0);
        let put = |off: usize, v: u64| unsafe {
            std::ptr::write_unaligned((page + off) as *mut u64, v);
        };
        put(0x88, 0xA000);
        put(0x90, 0xB000);
        unsafe { std::ptr::write_unaligned((page + 0x98) as *mut i32, 1) };
        put(0x1a0, 1_000_000);
        put(0x1a8, 900_000);
        unsafe { std::ptr::write_unaligned((page + 0x218) as *mut i32, 200_000) };
        let mut old = 0;
        assert_ne!(
            unsafe { VirtualProtect(page as *const _, 0x1000, PAGE_READONLY, &mut old) },
            0
        );
        let mut cgame = vec![0u64; 0x200 / 8 + 1];
        cgame[0x1f0 / 8] = page as u64;
        let slots = Slots::new();
        slots.next_epoch();
        published(&slots, 0xA000, 4);
        let mut state = Interval::new();
        let done = sync_in(
            &LAYOUT_40408,
            &slots,
            &mut state,
            cgame.as_ptr() as usize,
            true,
        );
        assert!(
            matches!(&done, Done::Off(why) if why.contains("writable")),
            "{done:?}"
        );
        assert_eq!(
            unsafe { std::ptr::read_unaligned((page + 0x218) as *const i32) },
            200_000
        );
        unsafe { VirtualFree(page as *mut _, 0, MEM_RELEASE) };
    }
}

/// The batch interval and crate::cadence together against the game's loop
/// and a room releasing steps: what the screen shows.
#[cfg(test)]
mod coupled {
    use std::time::{Duration, Instant};

    use tpf3mp_proto::Speed;

    use super::*;
    use crate::cadence::{Cadence, Pick};

    struct Run {
        /// Room updates per step time on screen, one per frame that moved.
        speeds: Vec<f64>,
        /// The most steps released and not run, after warming up.
        max_backlog: u64,
        /// Steps released and not run at the end.
        end_backlog: u64,
    }

    /// `seconds` of a room at `speed` (5 steps a second, checkpoints every
    /// 50), releases jittered by up to `jitter_ms`, an action every
    /// `action_every` calls (0: none), a freeze of `freeze_ms` at 20 s.
    fn run(
        speed: u16,
        seconds: u64,
        jitter_ms: f64,
        action_every: u64,
        freeze_ms: i64,
        timed: bool,
    ) -> Run {
        let step_ms = 200.0 * 100.0 / f64::from(speed);
        let step_us = (step_ms * 1000.0) as u32;
        let mut cadence = Cadence::new(true);
        cadence.begin(5);
        cadence.speed(Speed(speed));
        cadence.set_fixed(true);
        let start = Instant::now();
        cadence.reset(0, start);
        let nominal = super::nominal(5.0 * f64::from(speed) / 100.0);
        let slots = Slots::new();
        let epoch = slots.next_epoch();
        let mut state = Interval::new();
        // The game's loop (µs).
        let (mut total, mut last_sync, mut gft) = (0i64, 0i64, 200_000i64);
        let mut before = 0.0f64;
        let mut shown_count = 0u32;
        // The batch running now (to be exposed at the next Sync): its count.
        let mut running: Option<Record> = None;
        let mut next = 1u64;
        let mut calls = 0u64;
        let mut pending_action = false;
        let released_at = |ms: f64| -> u64 {
            // Release k at k*step + a deterministic wobble.
            let k = (ms / step_ms).floor().max(0.0) as u64;
            let wobble = |k: u64| ((k * 7919 % 13) as f64 / 12.0 - 0.5) * 2.0 * jitter_ms;
            if ms >= k as f64 * step_ms + wobble(k) {
                k
            } else {
                k.saturating_sub(1)
            }
        };
        let (mut speeds, mut max_backlog, mut last_shown) = (Vec::new(), 0u64, 0.0f64);
        let frames = seconds * 60;
        for frame in 0..frames {
            let dt = if frame == 20 * 60 && freeze_ms > 0 {
                freeze_ms * 1000
            } else {
                16_667
            };
            total += dt;
            while total >= last_sync + gft {
                let sync_at = last_sync + gft;
                let now_ms = sync_at as f64 / 1000.0;
                let now = start + Duration::from_micros(sync_at as u64);
                last_sync = sync_at;
                // Sync exposes the batch that ran; the wrapper times it.
                if let Some(record) = running.take() {
                    slots.publish(&record);
                    before += f64::from(shown_count);
                    shown_count = record.count;
                    gft = match state.after_sync(
                        &Seen {
                            m_data: 8,
                            exposed: record.sim,
                            game_us: 200_000,
                            total_time: total,
                            last_sync,
                        },
                        0,
                        slots.read(record.sim),
                    ) {
                        Ok(Ok(us)) if timed => i64::from(us),
                        _ => 200_000,
                    };
                }
                // The next batch runs on the simulation thread now.
                cadence.call(now);
                calls += 1;
                let released = released_at(now_ms);
                let to_cp = (50 - next % 50) % 50 + 1;
                let barrier = action_every > 0 && calls.is_multiple_of(action_every);
                let (count, steady) = if pending_action {
                    pending_action = false;
                    (0, false) // the actions call
                } else if released + 1 > next {
                    let cap = 16u64.min(to_cp).min(released + 1 - next) as u32;
                    let choice = cadence.choose(next, released, cap, to_cp, barrier, false, now);
                    pending_action = barrier;
                    (choice.steps, choice.pick == Pick::Nominal)
                } else {
                    (0, false)
                };
                next += u64::from(count);
                let backlog = (released + 1).saturating_sub(next);
                if frame > 5 * 60 && frame != 20 * 60 {
                    max_backlog = max_backlog.max(backlog);
                }
                running = Some(Record {
                    sim: if calls.is_multiple_of(2) { 0xA000 } else { 0xB000 },
                    epoch,
                    count,
                    eligible: true,
                    step_us,
                    nominal,
                    backlog: backlog as u32,
                    steady,
                });
            }
            total = total.min(last_sync + gft - 1);
            let alpha = (total - last_sync) as f64 / gft as f64;
            let shown = before + alpha * f64::from(shown_count);
            assert!(shown + 1e-9 >= last_shown, "the screen went back");
            if dt == 16_667 && frame > 5 * 60 && shown > last_shown {
                speeds.push((shown - last_shown) / (16.667 / step_ms));
            }
            last_shown = shown;
        }
        let end_ms = (frames as f64) * 16.667 + freeze_ms as f64;
        Run {
            speeds,
            max_backlog,
            end_backlog: (released_at(end_ms) + 1).saturating_sub(next),
        }
    }

    fn even_share(run: &Run, within: f64) -> f64 {
        let even = run
            .speeds
            .iter()
            .filter(|s| (**s - 1.0).abs() <= within)
            .count();
        even as f64 / run.speeds.len().max(1) as f64
    }

    #[test]
    fn at_every_speed_vehicles_move_at_the_rooms_speed() {
        for speed in [100, 200, 400] {
            let run = run(speed, 60, 8.0, 0, 0, true);
            let share = even_share(&run, 0.25);
            assert!(
                share >= 0.9,
                "{speed}%: {share:.2} even, {:?}",
                &run.speeds[..40]
            );
            assert!(
                run.max_backlog <= 8,
                "{speed}%: backlog {}",
                run.max_backlog
            );
        }
    }

    #[test]
    fn untimed_the_same_calls_move_vehicles_unevenly() {
        // The proof the measure measures: the game's own 200 ms interval
        // with the same counts (checkpoint cuts, actions) is visibly worse.
        let timed = even_share(&run(400, 60, 8.0, 15, 0, true), 0.1);
        let untimed = even_share(&run(400, 60, 8.0, 15, 0, false), 0.1);
        assert!(
            timed > untimed + 0.05,
            "timed {timed:.2}, untimed {untimed:.2}"
        );
    }

    #[test]
    fn actions_and_a_save_freeze_neither_stall_nor_slow_the_room() {
        for speed in [100, 400] {
            let run = run(speed, 60, 8.0, 15, 1200, true);
            assert!(
                run.end_backlog <= 8,
                "{speed}%: {} behind at the end",
                run.end_backlog
            );
            let share = even_share(&run, 0.25);
            assert!(share >= 0.75, "{speed}%: {share:.2} even");
        }
    }
}

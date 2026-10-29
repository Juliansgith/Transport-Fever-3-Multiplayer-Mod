//! Frame pacing in a room's game (docs/HOOKS.md, "Frame pacing").
//!
//! The game moves its vehicles by the frame's `dt` (`CGame::Step(this,
//! dt, …)` adds it to `totalTime`, and `alpha` follows), and that `dt` is
//! the **previous** frame's length: measured on Bubu's world (two games on
//! one PC, 2,400 frames at 1x), the clock's step at frame `i` equals the
//! wall time of frame `i - 1` (correlation +1.000) and correlates only
//! +0.27 with frame `i` itself. A frame shown for 37 ms after one of 22 ms
//! moves vehicles 22 ms' worth: on screen their speed swung frame to frame
//! (coefficient of variation 0.23, a jump of more than 30% in a third of
//! frames). That is the micro-stutter of a vehicle followed up close.
//!
//! No clock can know how long the frame it starts will take, so the fix is
//! to make frames equally long: after each frame the hook waits until the
//! frame has lasted the target, the 75th percentile of the last frames'
//! own length (without the wait), moved at most 2% a frame and kept between
//! 1/240 s and 1/20 s. A frame that runs longer is not waited on. Then the
//! previous frame's length is this frame's for most frames. Simulated on the
//! frames above, with [`Clock`]: speed variation 0.23 to 0.11, swings of
//! more than 30% from one frame in three to one in sixteen, for about 10%
//! fewer frames on that loaded PC. A higher percentile smooths little more
//! (0.097 at the 85th) for more frames lost (14%); measured in the game at
//! the 85th: 0.25 to 0.10 at 1x and 4x.
//!
//! Only presentation: nothing the simulation computes depends on wall time,
//! so a paced game and an unpaced one agree. Only in a room's game whose
//! batches are timed (`crate::motion`); `TPF3MP_HOOK_FRAME_PACING=0` turns
//! it off.

#![allow(unsafe_code, clippy::cast_precision_loss)]

use std::collections::VecDeque;
use std::time::Duration;
#[cfg(windows)]
use std::time::Instant;

pub const ENV: &str = "TPF3MP_HOOK_FRAME_PACING";
/// Frames whose length sets the target.
const HISTORY: usize = 120;
/// Frames needed before pacing starts.
const WARMUP: usize = 30;
/// The target's percentile of recent frame lengths.
const PERCENTILE: f64 = 0.75;
/// The target moves at most this fraction a frame.
const SLEW: f64 = 0.02;
pub const MIN_MS: f64 = 1000.0 / 240.0;
pub const MAX_MS: f64 = 50.0;
/// Waits shorter than this are not worth a timer.
const SMALLEST_WAIT_MS: f64 = 0.3;

/// The target controller: frame lengths in, waits out.
#[derive(Debug, Default)]
pub struct Pacer {
    lengths: VecDeque<f64>,
    target_ms: f64,
}

impl Pacer {
    pub fn new() -> Self {
        Self::default()
    }

    /// [`Pacer::new`] for a static.
    pub const fn new_const() -> Self {
        Self {
            lengths: VecDeque::new(),
            target_ms: 0.0,
        }
    }

    /// The target now, ms (0 before the warm-up).
    pub fn target_ms(&self) -> f64 {
        self.target_ms
    }

    /// Forgets every frame (a world change, a pause of pacing).
    pub fn reset(&mut self) {
        self.lengths.clear();
        self.target_ms = 0.0;
    }

    /// A frame that took `length_ms` of its own (from the end of the last
    /// wait to its end): how long to wait now, ms (0 for none).
    pub fn plan(&mut self, length_ms: f64) -> f64 {
        if !length_ms.is_finite() || length_ms <= 0.0 {
            return 0.0;
        }
        // A frame of seconds (a load, a save's freeze) says nothing about
        // the frame rate.
        if length_ms > 4.0 * MAX_MS {
            return 0.0;
        }
        if self.lengths.len() == HISTORY {
            self.lengths.pop_front();
        }
        self.lengths.push_back(length_ms);
        if self.lengths.len() < WARMUP {
            return 0.0;
        }
        let mut sorted: Vec<f64> = self.lengths.iter().copied().collect();
        sorted.sort_by(f64::total_cmp);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let at = ((sorted.len() - 1) as f64 * PERCENTILE).round() as usize;
        let wanted = sorted[at].clamp(MIN_MS, MAX_MS);
        self.target_ms = if self.target_ms <= 0.0 {
            wanted
        } else {
            let step = self.target_ms * SLEW;
            self.target_ms + (wanted - self.target_ms).clamp(-step, step)
        };
        let wait = self.target_ms - length_ms;
        if wait >= SMALLEST_WAIT_MS { wait } else { 0.0 }
    }
}

/// The vehicles' clock while frames are paced: each frame advances
/// `CGame::Step`'s `totalTime` by the pacing target rather than by the
/// previous frame's length, which the game would give, so a frame held to
/// the target moves vehicles exactly its own length. What the game's own
/// `dt` and the target differ by is a debt, repaid a tenth a frame, so the
/// clock keeps the game's time over any span: after a frame longer than the
/// target, vehicles catch up over the next frames instead of jumping in one.
#[derive(Debug, Default)]
pub struct Clock {
    /// The game's time not yet given (may be negative), µs.
    debt: i64,
}

/// The debt's share repaid each frame.
const REPAY: f64 = 0.1;

impl Clock {
    pub const fn new() -> Self {
        Self { debt: 0 }
    }

    pub fn debt_us(&self) -> i64 {
        self.debt
    }

    /// The `dt` to give for the game's `raw` one (µs), with the pacing
    /// target `target_us` (0: not pacing). Outside pacing, or for a `dt`
    /// far from the target (a freeze, a load, the game's slow motion), the
    /// game's own, and the debt is forgotten.
    pub fn give(&mut self, raw: i64, target_us: i64) -> i64 {
        if target_us <= 0 || raw <= 0 || raw > 3 * target_us || 3 * raw < target_us {
            self.debt = 0;
            return raw;
        }
        #[allow(clippy::cast_possible_truncation)]
        let repay = (self.debt as f64 * REPAY).round() as i64;
        let give = (target_us + repay)
            .clamp(target_us / 2, 2 * target_us)
            .max(1);
        self.debt += raw - give;
        give
    }
}

/// The profile target: `UI::CGameUI::GameStep`'s call of `CGame::Step`
/// (40425 `0x65f26c`), redirected to [`game_step`].
pub const STEP_CALL_TARGET: &str = "UI::CGameUI::GameStep/CGame::Step call";
/// `CGame::Step`'s first instructions on 40425 (`0x11f400`), up to its
/// `add [m_data + totalTime], rdx`: the `dt` the redirect changes is the
/// one added to the vehicles' clock, in µs.
pub const STEP_BYTES: [u8; 31] = [
    0x40, 0x53, // push rbx
    0x48, 0x83, 0xEC, 0x20, // sub rsp, 0x20
    0x48, 0x8B, 0x81, 0xF0, 0x01, 0x00, 0x00, // mov rax, [rcx+0x1f0]   m_data
    0x48, 0x8B, 0xD9, // mov rbx, rcx
    0x48, 0x89, 0x74, 0x24, 0x30, // mov [rsp+0x30], rsi
    0x49, 0x8B, 0xF0, // mov rsi, r8
    0x48, 0x01, 0x90, 0xA0, 0x01, 0x00, 0x00, // add [rax+0x1a0], rdx   totalTime += dt
];

static STEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

type StepFn = unsafe extern "C-unwind" fn(usize, i64, usize);

/// Where the game's UI calls `CGame::Step(this, dt, function)`: the same
/// call with the `dt` of [`crate::motion::clock_dt`]. `C-unwind`: an
/// exception `CGame::Sync` throws reaches the game's handlers as without it.
///
/// # Safety
///
/// Called by the game in place of `CGame::Step`, with its arguments.
pub unsafe extern "C-unwind" fn game_step(cgame: usize, dt: i64, function: usize) {
    let step = STEP.load(std::sync::atomic::Ordering::Acquire);
    let dt = std::panic::catch_unwind(|| crate::motion::clock_dt(dt)).unwrap_or(dt);
    // SAFETY: STEP holds CGame::Step's address, checked at install against
    // STEP_BYTES; the arguments are the game's but for dt, a positive time.
    let step: StepFn = unsafe { std::mem::transmute::<usize, StepFn>(step) };
    unsafe { step(cgame, dt, function) };
}

/// Redirects the call the profile names; the line for hook.log.
pub fn install_clock(resolved: &tpf3mp_hookcore::profile::ResolvedProfile, wanted: bool) -> String {
    if !wanted {
        return format!("frame pacing's clock: off ({ENV} says so)");
    }
    let Some(site) = resolved.get(STEP_CALL_TARGET) else {
        return format!(
            "frame pacing's clock: the game's (the profile has no {STEP_CALL_TARGET:?}); frames are still paced"
        );
    };
    install_clock_at(site.address as usize)
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn install_clock_at(at: usize) -> String {
    if !crate::image::readable(at, 5) {
        return format!(
            "frame pacing's clock: the game's ({STEP_CALL_TARGET} at {at:#x} is unreadable)"
        );
    }
    // SAFETY: five readable bytes of the call; only read.
    let call = unsafe { std::slice::from_raw_parts(at as *const u8, 5) };
    let rel = i32::from_le_bytes([call[1], call[2], call[3], call[4]]);
    let target = (at as isize + 5 + rel as isize) as usize;
    if call[0] != 0xE8 || !crate::image::readable(target, STEP_BYTES.len()) {
        return format!(
            "frame pacing's clock: the game's ({STEP_CALL_TARGET} at {at:#x} is no call it reads)"
        );
    }
    // SAFETY: readable bytes of the call's target; only read.
    if unsafe { std::slice::from_raw_parts(target as *const u8, STEP_BYTES.len()) } != STEP_BYTES {
        return format!(
            "frame pacing's clock: the game's ({STEP_CALL_TARGET} at {at:#x} reaches {target:#x}, not CGame::Step as checked)"
        );
    }
    STEP.store(target, std::sync::atomic::Ordering::Release);
    // SAFETY: the call inside CGameUI::GameStep, which no thread runs yet
    // (the hook installs before the game's first frame); install checks it
    // calls `target`; game_step has CGame::Step's ABI.
    match unsafe {
        tpf3mp_hookcore::detour::CallRedirect::install(
            at as *mut u8,
            target,
            game_step as *const u8,
        )
    } {
        Ok(redirect) => {
            std::mem::forget(redirect);
            format!(
                "frame pacing's clock: paced frames move vehicles by the pacing target, the game's time kept ({STEP_CALL_TARGET} at {at:#x})"
            )
        }
        Err(error) => format!("frame pacing's clock: the game's ({STEP_CALL_TARGET}: {error:?})"),
    }
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
fn install_clock_at(_at: usize) -> String {
    "frame pacing's clock: the game's (Windows x64 only)".to_owned()
}

/// Waits `ms` with a high-resolution timer, then spins the last half
/// millisecond on the clock.
#[cfg(windows)]
pub fn wait(ms: f64) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, INFINITE, SetWaitableTimer,
        TIMER_ALL_ACCESS, WaitForSingleObject,
    };
    thread_local! {
        static TIMER: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
    if !(ms > 0.0 && ms <= MAX_MS) {
        return;
    }
    let deadline = Instant::now() + Duration::from_secs_f64(ms / 1000.0);
    let sleep_ms = ms - 0.5;
    if sleep_ms > 0.0 {
        let timer = TIMER.with(|cell| {
            if cell.get() == 0 {
                // SAFETY: no attributes, no name; the handle is this
                // thread's for the life of the game.
                let handle = unsafe {
                    CreateWaitableTimerExW(
                        std::ptr::null(),
                        std::ptr::null(),
                        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                        TIMER_ALL_ACCESS,
                    )
                };
                cell.set(handle as usize);
            }
            cell.get()
        });
        if timer != 0 {
            // Relative, in 100 ns units.
            #[allow(clippy::cast_possible_truncation)]
            let due: i64 = -((sleep_ms * 10_000.0) as i64);
            // SAFETY: our timer; no completion routine.
            let set = unsafe { SetWaitableTimer(timer as _, &due, 0, None, std::ptr::null(), 0) };
            if set != 0 {
                // SAFETY: a valid handle; the timer fires within `sleep_ms`.
                unsafe { WaitForSingleObject(timer as _, INFINITE) };
            } else {
                // SAFETY: our handle, unusable: closed and made again later.
                unsafe { CloseHandle(timer as _) };
                TIMER.with(|cell| cell.set(0));
            }
        }
    }
    while Instant::now() < deadline {
        std::hint::spin_loop();
    }
}

#[cfg(not(windows))]
pub fn wait(ms: f64) {
    if ms > 0.0 && ms <= MAX_MS {
        std::thread::sleep(Duration::from_secs_f64(ms / 1000.0));
    }
}

/// Whether the switch allows pacing (read once at install).
pub fn wanted(value: Option<&str>) -> bool {
    crate::ticks::wanted(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_waits_before_the_warm_up() {
        let mut p = Pacer::new();
        for _ in 0..WARMUP - 1 {
            assert_eq!(p.plan(20.0), 0.0);
        }
        assert_eq!(p.target_ms(), 0.0);
    }

    #[test]
    fn short_frames_wait_up_to_the_target_and_long_ones_not_at_all() {
        let mut p = Pacer::new();
        // 90% of frames take 24 ms, the rest 37 ms: the target settles at
        // the slower frames' length only if they are more than a quarter.
        for i in 0..200 {
            p.plan(if i % 10 == 0 { 37.0 } else { 24.0 });
        }
        let target = p.target_ms();
        assert!((24.0..=24.1).contains(&target), "target {target}");
        assert_eq!(p.plan(37.0), 0.0, "a long frame is not waited on");
        let waited = p.plan(20.0);
        assert!((3.9..=4.1).contains(&waited), "waited {waited}");
    }

    #[test]
    fn the_target_follows_slowly_and_stays_in_bounds() {
        let mut p = Pacer::new();
        for _ in 0..HISTORY {
            p.plan(10.0);
        }
        assert!((p.target_ms() - 10.0).abs() < 1e-9);
        // Frames grow to 30 ms: the target moves at most 2% a frame.
        let before = p.target_ms();
        p.plan(30.0);
        assert!(p.target_ms() <= before * 1.02 + 1e-9);
        for _ in 0..1000 {
            p.plan(30.0);
        }
        assert!((p.target_ms() - 30.0).abs() < 1e-6);
        let mut q = Pacer::new();
        for _ in 0..HISTORY {
            q.plan(1.0);
        }
        assert!((q.target_ms() - MIN_MS).abs() < 1e-9, "at most 240 fps");
        let mut r = Pacer::new();
        for _ in 0..HISTORY {
            r.plan(120.0);
        }
        assert!((r.target_ms() - MAX_MS).abs() < 1e-9, "at least 20 fps");
    }

    #[test]
    fn a_freeze_or_nonsense_is_not_a_frame() {
        let mut p = Pacer::new();
        for _ in 0..HISTORY {
            p.plan(20.0);
        }
        let target = p.target_ms();
        assert_eq!(p.plan(4000.0), 0.0);
        assert_eq!(p.plan(f64::NAN), 0.0);
        assert_eq!(p.plan(-1.0), 0.0);
        assert!((p.target_ms() - target).abs() < 1e-9);
    }

    #[test]
    fn the_profile_names_the_step_call() {
        let profile = tpf3mp_hookcore::profile::Profile::from_toml(include_str!(
            "../../../profiles/tf3_build40425_steam_windows/hooks.toml"
        ))
        .unwrap();
        let target = profile
            .targets
            .iter()
            .find(|target| target.name == STEP_CALL_TARGET)
            .expect("the 40425 profile names the call");
        assert!(!target.required);
        assert_eq!(target.prologue, vec![0xE8], "a call of CGame::Step");
    }

    #[test]
    fn the_clock_gives_the_target_and_keeps_the_games_time() {
        let mut c = Clock::new();
        // Paced frames of 30 ms: the game's dt is the previous frame's.
        let frames = [
            30_000, 30_000, 45_000, 30_000, 30_000, 30_000, 31_000, 30_000,
        ];
        let mut raw_sum = 0;
        let mut given_sum = 0;
        let mut previous = 30_000;
        let mut given = Vec::new();
        for &length in &frames {
            let g = c.give(previous, 30_000);
            raw_sum += previous;
            given_sum += g;
            given.push(g);
            previous = length;
        }
        assert_eq!(given[0], 30_000);
        // The long frame's 15 ms come back a tenth at a time, not at once.
        assert!(given[3] < 33_000, "{given:?}");
        assert_eq!(raw_sum - given_sum, c.debt_us(), "nothing lost");
        // The rest of the debt is repaid over the following frames.
        for _ in 0..200 {
            c.give(30_000, 30_000);
        }
        assert!(c.debt_us().abs() <= 10, "debt {}", c.debt_us());
    }

    #[test]
    fn outside_pacing_or_for_odd_frames_the_clock_is_the_games() {
        let mut c = Clock::new();
        assert_eq!(c.give(25_000, 0), 25_000, "not pacing");
        c.give(40_000, 30_000);
        assert_ne!(c.debt_us(), 0);
        assert_eq!(c.give(500_000, 30_000), 500_000, "a freeze");
        assert_eq!(c.debt_us(), 0, "forgotten");
        assert_eq!(c.give(6_000, 30_000), 6_000, "slow motion's fifth");
        assert_eq!(c.give(-5, 30_000), -5);
    }

    /// Never short, and not long by more than a timer tick. The shortest of
    /// five tries: a busy test runner can preempt any one wait (18.6 ms for
    /// 5 on CI, 2026-10-29), which says nothing about the wait itself.
    #[cfg(windows)]
    #[test]
    fn the_wait_lasts_what_it_was_asked() {
        let tries: Vec<f64> = (0..5)
            .map(|_| {
                let start = Instant::now();
                wait(5.0);
                start.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        assert!(tries.iter().all(|took| *took >= 5.0), "{tries:?}");
        let shortest = tries.iter().copied().fold(f64::INFINITY, f64::min);
        assert!(shortest < 12.0, "{tries:?}");
    }
}

//! The motion line: what the renderer shows of the room's world, frame by
//! frame (docs/HOOKS.md, "What the renderer shows: the `motion:` line").
//!
//! The game draws every vehicle interpolated across the batch the last
//! `CGame::Sync` exposed: `alpha = (totalTime - lastSyncTime) /
//! guiFrameTime`, clamped below 1 while the next batch is not ready
//! (`crate::interval`, "The batch interval"). So the simulation time on
//! screen at a frame is the updates of every batch shown before, plus
//! `alpha` times the current batch's updates. Its rate against the room's
//! pace is how fast vehicles move on screen relative to how fast they
//! should: 1 is smooth, 0 is a vehicle standing still (a batch not ready in
//! time, or one without updates), and a frame that took long shows as a
//! jump. The line sums each 10 s of frames: frame times, that relative
//! speed's spread, and the frames and runs in which nothing moved, by why.
//!
//! Logging only: it reads three fields of `CGame::m_data` per frame through
//! the guarded reads and writes nothing. On unless `TPF3MP_HOOK_MOTION=0`.

#![allow(clippy::cast_precision_loss)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const ENV: &str = "TPF3MP_HOOK_MOTION";

/// How long one line sums.
pub const WINDOW: Duration = Duration::from_secs(10);
/// Samples kept per window for the percentiles (about a minute at 60 fps).
const SAMPLES: usize = 4096;
/// A frame whose shown time moved less than this many updates stood still.
const STILL: f64 = 1e-6;

/// Why the shown world stood still in a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// The next batch was not ready: `alpha` at its end, waiting for Sync.
    Late,
    /// The batch on screen has no updates (an action, a reserve rebuilt).
    Empty,
    /// The room released nothing for the batch (it waits for a member).
    Waiting,
    /// None of those (a frame shorter than the clock's resolution).
    Other,
}

/// The batch on screen.
#[derive(Debug, Clone, Copy)]
struct Batch {
    count: u32,
    /// Updates a second at the room's pace.
    pace: f64,
    waiting: bool,
}

#[derive(Debug, Default)]
struct Window {
    started: Option<f64>,
    frames: u64,
    frame_ms: Vec<f32>,
    speed: Vec<f32>,
    /// The same speed against the game's own clock (`totalTime`), and how
    /// far each frame's game time differs from its wall time.
    game_speed: Vec<f32>,
    clock_gap_ms: Vec<f32>,
    held: [u64; 4],
    held_ms: [f64; 4],
    longest_ms: f64,
    runs_over_50: u64,
    runs_over_100: u64,
    jerks: u64,
    batches: u64,
    empty_batches: u64,
}

/// The tracker, fed by the Sync wrapper ([`Tracker::batch`]) and each frame
/// ([`Tracker::frame`]); times in seconds of any monotonic clock.
#[derive(Debug, Default)]
pub struct Tracker {
    batch: Option<Batch>,
    /// Updates of every batch shown before the current one.
    shown_before: f64,
    /// The last frame: its time and the updates on screen.
    last: Option<(f64, f64)>,
    /// The game's `totalTime` at the last frame, µs.
    last_game_us: Option<i64>,
    last_speed: Option<f64>,
    /// The run of frames standing still: its start and why.
    still_since: Option<(f64, Hold)>,
    window: Window,
}

fn percentile(values: &mut [f32], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f32::total_cmp);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let at = ((values.len() - 1) as f64 * p).round() as usize;
    f64::from(values[at.min(values.len() - 1)])
}

impl Tracker {
    pub const fn new() -> Self {
        Self {
            batch: None,
            shown_before: 0.0,
            last: None,
            last_game_us: None,
            last_speed: None,
            still_since: None,
            window: Window {
                started: None,
                frames: 0,
                frame_ms: Vec::new(),
                speed: Vec::new(),
                game_speed: Vec::new(),
                clock_gap_ms: Vec::new(),
                held: [0; 4],
                held_ms: [0.0; 4],
                longest_ms: 0.0,
                runs_over_50: 0,
                runs_over_100: 0,
                jerks: 0,
                batches: 0,
                empty_batches: 0,
            },
        }
    }

    /// A Sync exposed a timed batch of `count` updates at `step_us` of room
    /// time per update; the batch before it was shown to its end.
    pub fn batch(&mut self, count: u32, step_us: u32, waiting: bool) {
        if step_us == 0 {
            self.untimed();
            return;
        }
        if let Some(previous) = self.batch {
            self.shown_before += f64::from(previous.count);
        }
        self.batch = Some(Batch {
            count,
            pace: 1e6 / f64::from(step_us),
            waiting,
        });
        self.window.batches += 1;
        if count == 0 {
            self.window.empty_batches += 1;
        }
    }

    /// A batch that is not timed (another world, a pause, the game's own
    /// pace): nothing on screen can be measured until the next timed one.
    pub fn untimed(&mut self) {
        self.end_still(None);
        self.batch = None;
        self.shown_before = 0.0;
        self.last = None;
        self.last_game_us = None;
        self.last_speed = None;
    }

    fn end_still(&mut self, now: Option<f64>) {
        if let Some((since, why)) = self.still_since.take() {
            let ms = now.map_or(0.0, |now| (now - since) * 1000.0);
            let _ = why;
            self.window.longest_ms = self.window.longest_ms.max(ms);
            if ms > 50.0 {
                self.window.runs_over_50 += 1;
            }
            if ms > 100.0 {
                self.window.runs_over_100 += 1;
            }
        }
    }

    /// A frame at `now` (seconds) with the game's `alpha` across the batch
    /// and its `totalTime` (µs).
    pub fn frame(&mut self, now: f64, alpha: f64, game_us: i64) {
        let Some(batch) = self.batch else {
            return;
        };
        let alpha = alpha.clamp(0.0, 1.0);
        let shown = self.shown_before + alpha * f64::from(batch.count);
        let window = &mut self.window;
        window.started.get_or_insert(now);
        if let Some((then, before)) = self.last {
            let dt = now - then;
            if dt > 0.0 {
                window.frames += 1;
                if window.frame_ms.len() < SAMPLES {
                    window.frame_ms.push((dt * 1000.0) as f32);
                }
                let moved = shown - before;
                let speed = (moved / dt) / batch.pace;
                if window.speed.len() < SAMPLES {
                    window.speed.push(speed as f32);
                }
                if let Some(game_before) = self.last_game_us {
                    let game_dt = (game_us - game_before) as f64 / 1e6;
                    if game_dt > 0.0 && window.game_speed.len() < SAMPLES {
                        window
                            .game_speed
                            .push(((moved / game_dt) / batch.pace) as f32);
                    }
                    if window.clock_gap_ms.len() < SAMPLES {
                        window
                            .clock_gap_ms
                            .push(((game_dt - dt).abs() * 1000.0) as f32);
                    }
                }
                if moved <= STILL {
                    let why = if batch.waiting {
                        Hold::Waiting
                    } else if batch.count == 0 {
                        Hold::Empty
                    } else if alpha >= 0.999 {
                        Hold::Late
                    } else {
                        Hold::Other
                    };
                    window.held[why as usize] += 1;
                    window.held_ms[why as usize] += dt * 1000.0;
                    if self.still_since.is_none() {
                        self.still_since = Some((then, why));
                    }
                    self.last_speed = None;
                } else {
                    if let Some(previous) = self.last_speed
                        && (speed - previous).abs() > 0.3
                    {
                        window.jerks += 1;
                    }
                    self.last_speed = Some(speed);
                    self.end_still(Some(then));
                }
            }
        }
        self.last = Some((now, shown));
        self.last_game_us = Some(game_us);
    }

    /// The window's line, when it is at least [`WINDOW`] long; starts the
    /// next one.
    pub fn take_line(&mut self, now: f64) -> Option<String> {
        let started = self.window.started?;
        let span = now - started;
        if span < WINDOW.as_secs_f64() || self.window.frames == 0 {
            return None;
        }
        let mut w = std::mem::take(&mut self.window);
        w.started = Some(now);
        self.window.started = w.started;
        let fps = w.frames as f64 / span;
        let p = |v: &mut Vec<f32>, q: f64| percentile(v, q);
        let (f50, f95, f99) = (
            p(&mut w.frame_ms, 0.5),
            p(&mut w.frame_ms, 0.95),
            p(&mut w.frame_ms, 0.99),
        );
        let fmax = w.frame_ms.iter().copied().fold(0.0f32, f32::max);
        let slow = w.frame_ms.iter().filter(|&&ms| ms > 33.4).count();
        let n = w.speed.len().max(1) as f64;
        let mean = w.speed.iter().map(|&s| f64::from(s)).sum::<f64>() / n;
        let var = w
            .speed
            .iter()
            .map(|&s| (f64::from(s) - mean).powi(2))
            .sum::<f64>()
            / n;
        let cv = if mean > 0.0 { var.sqrt() / mean } else { 0.0 };
        let (s5, s50, s95) = (
            p(&mut w.speed, 0.05),
            p(&mut w.speed, 0.5),
            p(&mut w.speed, 0.95),
        );
        let held: u64 = w.held.iter().sum();
        let (g5, g50, g95) = (
            p(&mut w.game_speed, 0.05),
            p(&mut w.game_speed, 0.5),
            p(&mut w.game_speed, 0.95),
        );
        let (c50, c95) = (p(&mut w.clock_gap_ms, 0.5), p(&mut w.clock_gap_ms, 0.95));
        Some(format!(
            "motion: {span:.1}s frames={} fps={fps:.1} frame_ms p50={f50:.1} p95={f95:.1} p99={f99:.1} max={fmax:.1} over33={slow}; \
             shown speed p5={s5:.2} p50={s50:.2} p95={s95:.2} mean={mean:.3} cv={cv:.3} jerks={}; \
             by the game's clock p5={g5:.2} p50={g50:.2} p95={g95:.2}, clock gap ms p50={c50:.1} p95={c95:.1}; \
             still frames={held} ({:.1}% ) late={}/{:.0}ms empty={}/{:.0}ms waiting={}/{:.0}ms other={}; \
             longest={:.0}ms runs>50ms={} >100ms={}; batches={} empty={}",
            w.frames,
            w.jerks,
            100.0 * held as f64 / w.frames as f64,
            w.held[Hold::Late as usize],
            w.held_ms[Hold::Late as usize],
            w.held[Hold::Empty as usize],
            w.held_ms[Hold::Empty as usize],
            w.held[Hold::Waiting as usize],
            w.held_ms[Hold::Waiting as usize],
            w.held[Hold::Other as usize],
            w.longest_ms,
            w.runs_over_50,
            w.runs_over_100,
            w.batches,
            w.empty_batches,
        ))
    }
}

static TRACKER: Mutex<Tracker> = Mutex::new(Tracker::new());
static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static ON: AtomicBool = AtomicBool::new(true);
/// A timed batch is on screen: the room's game runs (pacing's condition).
static TIMED: AtomicBool = AtomicBool::new(false);
static PACING_ON: AtomicBool = AtomicBool::new(false);
/// The pacing targets in µs while frames are paced, else 0 (for the
/// clock): a frame without a batch handed over, and one with.
static TARGET_US: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
static TARGET_SYNC_US: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
/// This frame's `CGame::Step` hands over a batch (`CGame::Sync`): set by
/// the clock at the frame's start, taken by the pacing at its end.
static FRAME_SYNC: AtomicBool = AtomicBool::new(false);
/// The vehicles' clock (`crate::pacing::Clock`), main thread only.
static CLOCK: Mutex<crate::pacing::Clock> = Mutex::new(crate::pacing::Clock::new());

/// The `dt` `CGame::Step` gets for the game's `raw` one: the pacing target
/// while frames are paced (`crate::pacing::Clock`), else the game's own.
///
/// A frame whose step hands over a batch takes longer (measured: about 6 ms
/// more, five times a second), so it has a target of its own: whether this
/// one will is known before the step runs, from `totalTime`, `lastSyncTime`
/// and `guiFrameTime` (`CGame::Step` syncs once `totalTime + dt` reaches
/// `lastSyncTime + guiFrameTime`).
pub fn clock_dt(raw: i64) -> i64 {
    let mut target = if PACING_ON.load(Ordering::Acquire) && TIMED.load(Ordering::Acquire) {
        TARGET_US.load(Ordering::Acquire)
    } else {
        0
    };
    let sync_target = TARGET_SYNC_US.load(Ordering::Acquire);
    let syncs = target > 0
        && read_clock().is_some_and(|(total, last, gft)| total + target >= last + i64::from(gft));
    FRAME_SYNC.store(syncs, Ordering::Release);
    if syncs && sync_target > 0 {
        target = sync_target;
    }
    match CLOCK.try_lock() {
        Ok(mut clock) => clock.give(raw, target),
        Err(_) => raw,
    }
}

/// The game's `totalTime`, `lastSyncTime` and `guiFrameTime` now.
fn read_clock() -> Option<(i64, i64, i32)> {
    let m_data = M_DATA.load(Ordering::Acquire);
    let layout = crate::interval::layout()?;
    if m_data == 0 {
        return None;
    }
    let mut reader = crate::image::Readable::new();
    let total = reader.read::<i64>((m_data + layout.total_time) as u64)?;
    let last = reader.read::<i64>((m_data + layout.last_sync) as u64)?;
    let gft = reader.read::<i32>((m_data + layout.gui_frame_time) as u64)?;
    (gft > 0).then_some((total, last, gft))
}

/// Frame pacing's state (`crate::pacing`), main thread only: a pacer for
/// frames without a batch handed over, and one for frames with.
struct Pacing {
    pacer: crate::pacing::Pacer,
    sync_pacer: crate::pacing::Pacer,
    sync_frames: u64,
    released: Option<Instant>,
    window_start: Option<Instant>,
    frames: u64,
    waits: u64,
    waited_ms: f64,
    overruns: u64,
}

static PACING: Mutex<Pacing> = Mutex::new(Pacing {
    pacer: crate::pacing::Pacer::new_const(),
    sync_pacer: crate::pacing::Pacer::new_const(),
    sync_frames: 0,
    released: None,
    window_start: None,
    frames: 0,
    waits: 0,
    waited_ms: 0.0,
    overruns: 0,
});
/// `TPF3MP_HOOK_MOTION=frames`: also every frame's raw values, for the
/// first [`FRAME_DUMP`] frames of a timed batch (a diagnosis, not for play).
static FRAMES_LEFT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
const FRAME_DUMP: u32 = 4000;
static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
/// `CGame::m_data` of the batch on screen, for the frame's reads.
static M_DATA: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn seconds(now: Instant) -> f64 {
    now.saturating_duration_since(*ORIGIN.get_or_init(Instant::now))
        .as_secs_f64()
}

/// Reads the switch once, at install; the line for hook.log.
pub fn configure_from_env() -> String {
    let value = std::env::var(ENV).ok();
    let on = crate::ticks::wanted(value.as_deref());
    ON.store(on, Ordering::Release);
    if on && value.as_deref().map(str::trim) == Some("frames") {
        FRAMES_LEFT.store(FRAME_DUMP, Ordering::Release);
    }
    if on {
        format!(
            "motion: summing what the renderer shows each frame, a line every 10 s in a room's game ({ENV}=0 turns it off)"
        )
    } else {
        format!("motion: off ({ENV} says so)")
    }
}

/// Reads the pacing switch once, at install; the line for hook.log.
pub fn configure_pacing_from_env() -> String {
    let on = crate::pacing::wanted(std::env::var(crate::pacing::ENV).ok().as_deref());
    PACING_ON.store(on, Ordering::Release);
    if on {
        format!(
            "frame pacing: in a room's game, frames held to the 75th percentile of the last frames' length so vehicles move evenly ({}=0 turns it off)",
            crate::pacing::ENV
        )
    } else {
        format!("frame pacing: off ({} says so)", crate::pacing::ENV)
    }
}

/// From the Sync wrapper (main thread): a timed batch on screen.
pub fn on_batch(m_data: usize, count: u32, step_us: u32, waiting: bool) {
    TIMED.store(true, Ordering::Release);
    M_DATA.store(m_data, Ordering::Release);
    if !ON.load(Ordering::Acquire) {
        return;
    }
    if let Ok(mut tracker) = TRACKER.try_lock() {
        tracker.batch(count, step_us, waiting);
    }
}

/// From the Sync wrapper: a batch that is not timed.
pub fn on_untimed() {
    TIMED.store(false, Ordering::Release);
    if !ON.load(Ordering::Acquire) {
        return;
    }
    if let Ok(mut tracker) = TRACKER.try_lock() {
        tracker.untimed();
    }
}

/// Holds the frame to the pacing target, while a room's game runs.
fn pace() {
    if !PACING_ON.load(Ordering::Acquire) {
        return;
    }
    let Ok(mut p) = PACING.try_lock() else {
        return;
    };
    let now = Instant::now();
    let synced = FRAME_SYNC.swap(false, Ordering::AcqRel);
    if !TIMED.load(Ordering::Acquire) {
        p.pacer.reset();
        p.sync_pacer.reset();
        p.released = None;
        TARGET_US.store(0, Ordering::Release);
        TARGET_SYNC_US.store(0, Ordering::Release);
        return;
    }
    if let Some(released) = p.released {
        let length = now.saturating_duration_since(released).as_secs_f64() * 1000.0;
        let pacer = if synced {
            p.sync_frames += 1;
            &mut p.sync_pacer
        } else {
            &mut p.pacer
        };
        let wait = pacer.plan(length);
        let target = pacer.target_ms();
        p.frames += 1;
        if wait > 0.0 {
            crate::pacing::wait(wait);
            p.waits += 1;
            p.waited_ms += wait;
        } else if target > 0.0 && length > target {
            p.overruns += 1;
        }
    }
    let released = Instant::now();
    p.released = Some(released);
    #[allow(clippy::cast_possible_truncation)]
    {
        TARGET_US.store(
            (p.pacer.target_ms() * 1000.0).round() as i64,
            Ordering::Release,
        );
        TARGET_SYNC_US.store(
            (p.sync_pacer.target_ms() * 1000.0).round() as i64,
            Ordering::Release,
        );
    }
    let start = *p.window_start.get_or_insert(released);
    let span = released.saturating_duration_since(start);
    if span >= WINDOW && p.frames > 0 {
        let line = format!(
            "pacing: {:.1}s frames={} target_ms={:.1} handover frames={} target_ms={:.1} waited={} mean_wait_ms={:.2} over_target={}",
            span.as_secs_f64(),
            p.frames,
            p.pacer.target_ms(),
            p.sync_frames,
            p.sync_pacer.target_ms(),
            p.waits,
            if p.waits > 0 {
                p.waited_ms / p.waits as f64
            } else {
                0.0
            },
            p.overruns
        );
        p.window_start = Some(released);
        p.frames = 0;
        p.sync_frames = 0;
        p.waits = 0;
        p.waited_ms = 0.0;
        p.overruns = 0;
        if let Ok(mut lines) = LINES.try_lock()
            && lines.len() < 1000
        {
            lines.push(line);
        }
    }
}

/// Once a frame, on the main thread, after the game's own frame: paces it,
/// then measures what it showed.
pub fn on_frame() {
    pace();
    if !ON.load(Ordering::Acquire) {
        return;
    }
    let m_data = M_DATA.load(Ordering::Acquire);
    let Some(layout) = crate::interval::layout() else {
        return;
    };
    if m_data == 0 {
        return;
    }
    let mut reader = crate::image::Readable::new();
    let total = reader.read::<i64>((m_data + layout.total_time) as u64);
    let last = reader.read::<i64>((m_data + layout.last_sync) as u64);
    let gft = reader.read::<i32>((m_data + layout.gui_frame_time) as u64);
    let (Some(total), Some(last), Some(gft)) = (total, last, gft) else {
        return;
    };
    if gft <= 0 {
        return;
    }
    let alpha = (total - last) as f64 / f64::from(gft);
    let now = seconds(Instant::now());
    let Ok(mut tracker) = TRACKER.try_lock() else {
        return;
    };
    tracker.frame(now, alpha, total);
    let dump = tracker.batch.map(|b| b.count).filter(|_| {
        FRAMES_LEFT
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
            .is_ok()
    });
    let line = tracker.take_line(now);
    drop(tracker);
    if let Ok(mut lines) = LINES.try_lock() {
        if let Some(count) = dump
            && lines.len() < 1000
        {
            lines.push(format!(
                "motion-frame: wall_us={:.0} game_us={total} last_us={last} gft_us={gft} n={count}",
                now * 1e6
            ));
        }
        if let Some(line) = line
            && lines.len() < 1000
        {
            lines.push(line);
        }
    }
}

/// The lines for the hook's log.
pub fn take_lines() -> Vec<String> {
    match LINES.try_lock() {
        Ok(mut lines) => std::mem::take(&mut *lines),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frames at `fps` from `from` to `to` seconds; batches of `count`
    /// updates every `period` seconds (room pace `count / period`), each
    /// ready `late` seconds after it was due.
    fn run(t: &mut Tracker, from: f64, to: f64, fps: f64, period: f64, count: u32, late: f64) {
        let step_us = (period / f64::from(count) * 1e6) as u32;
        let mut sync = from;
        let mut due = from + period;
        t.batch(count, step_us, false);
        let mut now = from;
        while now < to {
            if now >= due + late {
                // The batch shown ended at its due time; the next one starts
                // late, from `alpha` 0.
                sync = now;
                due = now + period;
                t.batch(count, step_us, false);
            }
            let alpha = ((now - sync) / period).min(0.9999);
            t.frame(now, alpha, (now * 1e6) as i64);
            now += 1.0 / fps;
        }
    }

    #[test]
    fn steady_batches_show_the_rooms_pace_without_a_still_frame() {
        let mut t = Tracker::new();
        run(&mut t, 0.0, 10.5, 60.0, 0.2, 4, 0.0);
        let line = t.take_line(10.5).unwrap();
        assert!(line.contains("still frames=0 "), "{line}");
        assert!(line.contains("p50=1.00"), "{line}");
        assert!(line.contains("runs>50ms=0"), "{line}");
    }

    #[test]
    fn a_late_batch_shows_as_still_frames_at_the_end_of_the_one_before() {
        let mut t = Tracker::new();
        run(&mut t, 0.0, 10.5, 60.0, 0.2, 4, 0.08);
        let line = t.take_line(10.5).unwrap();
        assert!(!line.contains("late=0/"), "{line}");
        assert!(line.contains("runs>50ms="), "{line}");
        assert!(
            !line.contains("runs>50ms=0 "),
            "every 80 ms wait is a run: {line}"
        );
    }

    #[test]
    fn an_empty_batch_stands_still_for_its_reason() {
        let mut t = Tracker::new();
        t.batch(4, 50_000, false);
        let mut now = 0.0;
        while now < 0.2 {
            t.frame(now, now / 0.2, (now * 1e6) as i64);
            now += 1.0 / 60.0;
        }
        t.batch(0, 50_000, false);
        while now < 0.25 {
            t.frame(now, (now - 0.2) / 0.05, (now * 1e6) as i64);
            now += 1.0 / 60.0;
        }
        t.batch(4, 50_000, false);
        while now < 10.6 {
            t.frame(now, ((now - 0.25) / 0.2).fract(), (now * 1e6) as i64);
            now += 1.0 / 60.0;
        }
        let line = t.take_line(now).unwrap();
        assert!(
            line.contains("empty=") && !line.contains("empty=0/"),
            "{line}"
        );
    }

    #[test]
    fn nothing_is_summed_without_a_timed_batch() {
        let mut t = Tracker::new();
        t.frame(0.0, 0.5, 0);
        t.frame(11.0, 0.6, 11_000_000);
        assert_eq!(t.take_line(11.0), None);
        t.batch(4, 50_000, false);
        t.untimed();
        t.frame(12.0, 0.1, 12_000_000);
        assert_eq!(t.take_line(30.0), None);
    }
}

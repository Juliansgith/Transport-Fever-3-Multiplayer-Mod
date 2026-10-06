//! The emitters' splat into the emission grids, by row bands and
//! bit-identical (investigation/TF3_EMITTER_SPLAT_2026-10-06.md;
//! docs/HOOKS.md, "The fast emitters").
//!
//! `ecs::EmissionEmitterSystem::Update2` adds every emitter (buildings,
//! industries, streets) into the noise and pollution grids each
//! simulation update: lambda_1 buckets the emitters into a fixed 4 x 4
//! split of the grid, lambda_2 then adds each region's emitters in 32
//! tasks. The grids are simulation state (town ratings; saved), so every
//! bit must match in every game of a room.
//!
//! The hook redirects `Update2`'s call of lambda_1's dispatcher (which it
//! skips) and its two calls of lambda_2's (inline and pool), and splats
//! in their place by row bands on the hook's threads ([`bands`],
//! [`splat`]): every cell receives the game's additions in the game's
//! order. `Update2` itself (its component lookups, asserts and factors)
//! stays the game's.
//!
//! Fail-closed, at every level:
//!
//! - at install: [`ENV`] `=0` leaves the game's update; so does a profile
//!   without the target, a CPU without AVX, or any byte of the modelled
//!   code that differs from what was read;
//! - each update: anything the model does not cover (grids of another
//!   shape, a node naming no component, an emitter of a radius past
//!   [`splat::MAX_SPAN`] cells, a non-default MXCSR) runs the game's own
//!   lambda_1 and lambda_2 instead, before anything was written;
//! - in the game: the first updates, and one in every [`CHECK_EVERY`]
//!   after, run the game's own lambdas on the real grids and the band
//!   splat on copies of windows of rows taken before; any difference
//!   turns the band splat off for the rest of the game and is logged.
//!
//! Since the result is the game's to the bit, a game with the hook and one
//! without agree: nothing for the room to set.

#![allow(unsafe_code)]

pub mod bands;
pub mod splat;

#[cfg(all(test, windows, target_arch = "x86_64"))]
mod mapped;
#[cfg(all(test, windows, target_arch = "x86_64"))]
mod original_tests;

use std::cell::RefCell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use tpf3mp_hookcore::profile::ResolvedProfile;

pub use crate::build_data::native::emitters::{
    CODE, COMP_CONCENTRATION, COMP_GRID_POINT_SIZE, Code, LAMBDA1_CALL, LAMBDA1_DISPATCH,
    LAMBDA2_LOOP, LAMBDA2_LOOP_CALL, LAMBDA2_POOL, LAMBDA2_POOL_CALL, LOGF_SLOT, LOGF_THUNK,
    SYSTEM_DATA, SYSTEM_NODES, UPDATE2,
};
pub use bands::{Done, Plan, Source, run};
pub use splat::{Emitter, Frame, Kind, Logf, View};

#[cfg(test)]
use crate::emission;
use crate::emission::{Grid, fnv1a, wanted};
use crate::log;

/// The fix's name in `hook.log`.
pub const FIX: &str = "emitters";

/// `0` (or `off`) in the game's environment leaves the game's own
/// splat; unset, the band splat runs.
pub const ENV: &str = "TPF3MP_HOOK_FAST_EMITTERS";

/// Bands per participating thread.
pub const BANDS_PER_THREAD: usize = 16;
/// The first updates checked against the game's own lambdas.
pub const FIRST_CHECKS: u64 = 3;
/// After those, one update in this many is checked.
pub const CHECK_EVERY: u64 = 1024;
/// Rows per checked window (three windows).
pub const CHECK_ROWS: i32 = 16;
/// A summary line in `hook.log` every this many band-splat updates.
pub const REPORT_EVERY: u64 = 4096;

/// The import slot the game's `logf` thunk jumps through (RVA): the tests
/// call the game's `logf` through it.
#[cfg(test)]
pub const LOGF_SLOT_RVA: u64 = 0xaa51c0 + LOGF_SLOT as u64;

/// Lambda_1's captures (`Update2`'s frame): the two grid components, the
/// system, the origin and a quarter of the size, x then y.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Lambda1 {
    comps: *const [usize; 2],
    system: usize,
    x0: *const i32,
    quarter_x: *const i32,
    y0: *const i32,
    quarter_y: *const i32,
}

/// Lambda_2's: the origin and the quarters, the system, the components,
/// `dt`, the two factors (`0x140aa8ba0`'s, one per grid).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Lambda2 {
    x0: *const i32,
    quarter_x: *const i32,
    y0: *const i32,
    quarter_y: *const i32,
    system: usize,
    comps: *const [usize; 2],
    dt: *const f32,
    factors: *const [f32; 2],
}

/// `LoopImpl<lambda_1>(pool, &lambda, count, 1024, &flag, byte, byte)`.
type Lambda1Dispatch =
    unsafe extern "system" fn(usize, *const Lambda1, i32, i32, usize, usize, usize);
/// Lambda_2's loop `(&lambda, first, end)`.
type Lambda2Loop = unsafe extern "system" fn(*const Lambda2, i32, i32);
/// `LoopImpl<lambda_2>(pool, &lambda, 32, 1, &futures, byte)`.
type Lambda2Pool = unsafe extern "system" fn(usize, *const Lambda2, i32, i32, usize, usize);

/// Where the game's code is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Game {
    pub lambda1_dispatch: usize,
    pub lambda2_loop: usize,
    pub lambda2_pool: usize,
    /// The import slot of `logf`.
    pub logf_slot: usize,
}

impl Game {
    /// From `Update2`'s address, by the recorded offsets.
    pub fn from_update2(update2: usize) -> Self {
        let at = |offset: i64| update2.wrapping_add_signed(offset as isize);
        Self {
            lambda1_dispatch: at(LAMBDA1_DISPATCH),
            lambda2_loop: at(LAMBDA2_LOOP),
            lambda2_pool: at(LAMBDA2_POOL),
            logf_slot: at(LOGF_SLOT),
        }
    }
}

static GAME: Mutex<Option<Game>> = Mutex::new(None);
/// Set when a check failed: the game's own splat from then on.
static BROKEN: AtomicBool = AtomicBool::new(false);
static UPDATES: AtomicU64 = AtomicU64::new(0);
static BANDED: AtomicU64 = AtomicU64::new(0);
static BANDED_NANOS: AtomicU64 = AtomicU64::new(0);
static GAMES_WAY: AtomicU64 = AtomicU64::new(0);
static CHECKED: AtomicU64 = AtomicU64::new(0);
/// Tests: every update the game's way, unchecked.
#[cfg(test)]
static FORCE_GAMES_WAY: AtomicBool = AtomicBool::new(false);
/// Tests: the self-check sees one bit of its copy flipped.
#[cfg(test)]
static SABOTAGE: AtomicBool = AtomicBool::new(false);

/// Tests: the counters and buffers as at the game's start.
#[cfg(test)]
fn reset() {
    for c in [
        &UPDATES,
        &BANDED,
        &BANDED_NANOS,
        &GAMES_WAY,
        &CHECKED,
        &PREPARE_NANOS,
        &SPLAT_NANOS,
        &MADE,
    ] {
        c.store(0, Ordering::SeqCst);
    }
    BROKEN.store(false, Ordering::SeqCst);
    *STATE.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

fn game() -> Option<Game> {
    *GAME.lock().unwrap_or_else(|p| p.into_inner())
}

/// Lambda_1's dispatcher call, as `Update2` made it, with a copy of its
/// captures: `Update2` reuses their place on its frame for lambda_2's.
#[derive(Debug, Clone, Copy)]
struct Lambda1Call {
    pool: usize,
    captures: Lambda1,
    count: i32,
    min_chunk: i32,
    a5: usize,
    a6: usize,
    a7: usize,
}

impl Lambda1Call {
    /// # Safety
    ///
    /// As `Update2` would make it, before its lambda_2 call.
    unsafe fn call(&self, game: &Game) {
        // SAFETY: the caller's.
        unsafe {
            std::mem::transmute::<usize, Lambda1Dispatch>(game.lambda1_dispatch)(
                self.pool,
                &raw const self.captures,
                self.count,
                self.min_chunk,
                self.a5,
                self.a6,
                self.a7,
            );
        }
    }
}

/// Where the update stands, on `Update2`'s thread.
#[derive(Debug, Clone, Copy)]
enum Mode {
    Idle,
    /// Lambda_1 was skipped: the band splat is due at lambda_2.
    Deferred(Lambda1Call),
    /// Lambda_1 ran: lambda_2 runs the game's way, checked.
    Check,
}

/// The band splat's buffers, kept between updates (the simulation runs
/// one update at a time).
struct State {
    plan: Plan,
    check: Plan,
}

thread_local! {
    static MODE: RefCell<Mode> = const { RefCell::new(Mode::Idle) };
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn take_mode() -> Mode {
    MODE.with(|m| std::mem::replace(&mut *m.borrow_mut(), Mode::Idle))
}

fn set_mode(mode: Mode) {
    MODE.with(|m| *m.borrow_mut() = mode);
}

/// # Safety
///
/// `at` is readable for a `T`.
unsafe fn read<T: Copy>(at: usize) -> T {
    // SAFETY: the caller's.
    unsafe { std::ptr::read_unaligned(at as *const T) }
}

unsafe extern "system" fn lambda1_hook(
    pool: usize,
    ctx: *const Lambda1,
    count: i32,
    min_chunk: i32,
    a5: usize,
    a6: usize,
    a7: usize,
) {
    let call = Lambda1Call {
        pool,
        // SAFETY: `Update2`'s lambda, on its frame.
        captures: unsafe { read(ctx as usize) },
        count,
        min_chunk,
        a5,
        a6,
        a7,
    };
    let Some(game) = game() else { return };
    let n = UPDATES.fetch_add(1, Ordering::Relaxed);
    let checked = n < FIRST_CHECKS || n.is_multiple_of(CHECK_EVERY);
    let banded = !BROKEN.load(Ordering::Acquire);
    #[cfg(test)]
    let banded = banded && !FORCE_GAMES_WAY.load(Ordering::SeqCst);
    if banded && !checked {
        set_mode(Mode::Deferred(call));
        return;
    }
    set_mode(if banded { Mode::Check } else { Mode::Idle });
    // SAFETY: the game's call, as `Update2` made it.
    unsafe { call.call(&game) };
}

/// The lambda_2 call `Update2` made: its inline loop or its pool path.
#[derive(Debug, Clone, Copy)]
enum Lambda2Call {
    Loop {
        first: i32,
        end: i32,
    },
    Pool {
        pool: usize,
        tasks: i32,
        min_chunk: i32,
        futures: usize,
        a6: usize,
    },
}

impl Lambda2Call {
    /// # Safety
    ///
    /// As `Update2` would make it, after its lambda_1 call.
    unsafe fn call(&self, game: &Game, ctx: *const Lambda2) {
        // SAFETY: the caller's.
        unsafe {
            match *self {
                Self::Loop { first, end } => {
                    std::mem::transmute::<usize, Lambda2Loop>(game.lambda2_loop)(ctx, first, end)
                }
                Self::Pool {
                    pool,
                    tasks,
                    min_chunk,
                    futures,
                    a6,
                } => std::mem::transmute::<usize, Lambda2Pool>(game.lambda2_pool)(
                    pool, ctx, tasks, min_chunk, futures, a6,
                ),
            }
        }
    }
}

unsafe extern "system" fn lambda2_loop_hook(ctx: *const Lambda2, first: i32, end: i32) {
    // SAFETY: `Update2`'s call.
    unsafe { lambda2(ctx, Lambda2Call::Loop { first, end }) }
}

unsafe extern "system" fn lambda2_pool_hook(
    pool: usize,
    ctx: *const Lambda2,
    tasks: i32,
    min_chunk: i32,
    futures: usize,
    a6: usize,
) {
    // SAFETY: `Update2`'s call.
    unsafe {
        lambda2(
            ctx,
            Lambda2Call::Pool {
                pool,
                tasks,
                min_chunk,
                futures,
                a6,
            },
        )
    }
}

/// Lambda_2's turn: the band splat, or the game's way (checked or not).
///
/// # Safety
///
/// `ctx` and `call` are `Update2`'s lambda_2 call.
unsafe fn lambda2(ctx: *const Lambda2, call: Lambda2Call) {
    let Some(game) = game() else { return };
    match take_mode() {
        Mode::Deferred(lambda1) => {
            // SAFETY: the update's own state.
            match unsafe { banded(&game, ctx, &lambda1) } {
                Ok(()) => {}
                Err(why) => {
                    note_games_way(&why);
                    GAMES_WAY.fetch_add(1, Ordering::Relaxed);
                    // Nothing was written: the game's lambda_1, then its
                    // lambda_2, as `Update2` would have run them.
                    // SAFETY: the calls `Update2` made, in its order.
                    unsafe {
                        lambda1.call(&game);
                        call.call(&game, ctx);
                    }
                }
            }
        }
        Mode::Check => {
            // SAFETY: the update's own state; lambda_1 ran.
            unsafe { checked(&game, ctx, call) };
        }
        Mode::Idle => {
            // SAFETY: the game's call, as `Update2` made it.
            unsafe { call.call(&game, ctx) };
        }
    }
}

/// The update's frame and emitters, read from lambda_2's captures, with
/// the views on the whole grids; the grids' lengths.
///
/// # Safety
///
/// `ctx` is lambda_2's captures, live.
unsafe fn inputs<'a>(
    game: &Game,
    ctx: *const Lambda2,
) -> Result<(Frame, Source<'a>, [usize; 2]), String> {
    // SAFETY: the captures and what they point at, on `Update2`'s frame
    // and in the game's components.
    unsafe {
        let l: Lambda2 = read(ctx as usize);
        let comps: [usize; 2] = read(l.comps as usize);
        let factors: [f32; 2] = read(l.factors as usize);
        let logf: usize = read(game.logf_slot);
        if logf == 0 {
            return Err("the logf import is not resolved".into());
        }
        let mut lens = [0; 2];
        let mut kinds = [None, None];
        for k in 0..2 {
            let grid: Grid = read(comps[k] + COMP_CONCENTRATION);
            let (begin, end) = (grid.begin as usize, grid.end as usize);
            if begin == 0 || end < begin || begin % 4 != 0 {
                return Err(format!("grid {k}'s vector is not a vector"));
            }
            lens[k] = (end - begin) / 4;
            kinds[k] = Some(Kind {
                gp: read(comps[k] + COMP_GRID_POINT_SIZE),
                factor: factors[k],
                view: View {
                    gx0: grid.x0,
                    gy0: grid.y0,
                    w: grid.width,
                    h: grid.height,
                    data: grid.begin,
                    y_lo: grid.y0,
                    y_hi: grid.y0.wrapping_add(grid.height),
                },
            });
        }
        let [Some(k0), Some(k1)] = kinds else {
            unreachable!()
        };
        let frame = Frame {
            origin: [read(l.x0 as usize), read(l.y0 as usize)],
            quarter: [read(l.quarter_x as usize), read(l.quarter_y as usize)],
            dt: read(l.dt as usize),
            kinds: [k0, k1],
            logf: std::mem::transmute::<usize, Logf>(logf),
        };
        let nodes: [usize; 2] = read(read::<usize>(l.system + SYSTEM_NODES));
        let data: [usize; 2] = read(read::<usize>(l.system + SYSTEM_DATA));
        let slice = |[begin, end]: [usize; 2], size: usize| {
            if begin == 0 || end < begin || (end - begin) % size != 0 || begin % 4 != 0 {
                None
            } else {
                Some((begin, (end - begin) / size))
            }
        };
        let (Some(nodes), Some(data)) = (slice(nodes, 8), slice(data, 36)) else {
            return Err("the node list or the emitters are not a vector".into());
        };
        let source = Source {
            nodes: std::slice::from_raw_parts(nodes.0 as *const [i32; 2], nodes.1),
            data: std::slice::from_raw_parts(data.0 as *const Emitter, data.1),
        };
        Ok((frame, source, lens))
    }
}

/// The band splat in lambda_1's and lambda_2's place; `Err` (nothing
/// written) where the model does not cover the update.
///
/// # Safety
///
/// `ctx` is lambda_2's captures; `lambda1` the update's lambda_1 call.
unsafe fn banded(game: &Game, ctx: *const Lambda2, lambda1: &Lambda1Call) -> Result<(), String> {
    // SAFETY: the caller's.
    let (frame, source, lens) = unsafe { inputs(game, ctx)? };
    let l1 = lambda1.captures;
    // SAFETY: lambda_2's captures, on `Update2`'s frame.
    let l2: Lambda2 = unsafe { read(ctx as usize) };
    if l1.system != l2.system
        || l1.comps != l2.comps
        || usize::try_from(lambda1.count).ok() != Some(source.nodes.len())
    {
        return Err("lambda_1 and lambda_2 were called for different emitters".into());
    }
    let pool = crate::emission::shared_pool();
    let bands = BANDS_PER_THREAD * pool.participants();
    let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
    let state = state.get_or_insert_with(|| State {
        plan: Plan::default(),
        check: Plan::default(),
    });
    let start = Instant::now();
    // SAFETY: the update's grids, which nothing else touches while
    // `Update2` waits on this call; the game's logf.
    match unsafe { run(&frame, &source, lens, bands, Some(pool), &mut state.plan) } {
        Ok(Some(done)) => {
            let nanos = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
            note_banded(&done, nanos);
            Ok(())
        }
        Ok(None) => {
            broken("a band panicked while writing: the grids are half done");
            Ok(())
        }
        Err(why) => Err(why.0),
    }
}

/// A checked update: windows of rows copied, the game's lambda_2 on the
/// real grids, the band splat on the copies; any difference turns the
/// band splat off.
///
/// # Safety
///
/// `ctx` and `call` are `Update2`'s lambda_2 call, lambda_1 having run.
unsafe fn checked(game: &Game, ctx: *const Lambda2, call: Lambda2Call) {
    // SAFETY: the caller's.
    let prepared = unsafe { inputs(game, ctx) }.and_then(|(frame, source, lens)| {
        bands::covered(&frame, lens).map_err(|why| why.0)?;
        let windows = windows(&frame, &source);
        let copies: Vec<[Vec<f32>; 2]> = windows
            .iter()
            .map(|&(lo, hi)| {
                frame.kinds.map(|k| {
                    let v = k.view;
                    let from = (lo - v.gy0) as usize * v.w as usize;
                    let len = (hi - lo) as usize * v.w as usize;
                    // SAFETY: rows lo..hi of the grid (covered: inside).
                    unsafe { std::slice::from_raw_parts(v.data.add(from), len) }.to_vec()
                })
            })
            .collect();
        Ok((frame, source, windows, copies))
    });
    // The game's own lambda_2, on the real grids: their result stands.
    // SAFETY: the game's call, as `Update2` made it.
    unsafe { call.call(game, ctx) };
    let (frame, source, windows, mut copies) = match prepared {
        Ok(p) => p,
        Err(why) => {
            note_games_way(&why);
            return;
        }
    };
    let pool = crate::emission::shared_pool();
    let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
    let state = state.get_or_insert_with(|| State {
        plan: Plan::default(),
        check: Plan::default(),
    });
    for (&(lo, hi), copy) in windows.iter().zip(&mut copies) {
        let mut window = frame;
        for (k, c) in copy.iter_mut().enumerate() {
            let v = &mut window.kinds[k].view;
            v.data = c.as_mut_ptr();
            v.y_lo = lo;
            v.y_hi = hi;
        }
        let lens = [copy[0].len(), copy[1].len()];
        // SAFETY: the copies hold the window's rows; the game's logf.
        match unsafe { run(&window, &source, lens, 4, Some(pool), &mut state.check) } {
            Ok(Some(_)) => {}
            Ok(None) => {
                broken("a band of the self-check panicked");
                return;
            }
            Err(why) => {
                note_games_way(&why.0);
                return;
            }
        }
        #[cfg(test)]
        if SABOTAGE.load(Ordering::SeqCst) {
            copy[0][0] = f32::from_bits(copy[0][0].to_bits() ^ 1);
        }
        for (k, c) in copy.iter().enumerate() {
            let v = frame.kinds[k].view;
            let from = (lo - v.gy0) as usize * v.w as usize;
            // SAFETY: rows lo..hi of the real grid, which the game's
            // lambda_2 has just written.
            let real = unsafe { std::slice::from_raw_parts(v.data.add(from), c.len()) };
            if let Some(i) = (0..c.len()).find(|&i| c[i].to_bits() != real[i].to_bits()) {
                broken(&format!(
                    "self-check failed: {} differs at ({}, {}): {:#010x}, the game {:#010x}",
                    ["noise", "pollution"][k],
                    v.gx0 + (i % v.w as usize) as i32,
                    lo + (i / v.w as usize) as i32,
                    c[i].to_bits(),
                    real[i].to_bits()
                ));
                return;
            }
        }
    }
    let checked = CHECKED.fetch_add(1, Ordering::Relaxed) + 1;
    if checked == 1 {
        log::line(&format!(
            "{FIX}: {} windows of the real grids ({}x{}, {} emitters) bit-identical to the game's Update2; banded from now on",
            windows.len(),
            frame.kinds[0].view.w,
            frame.kinds[0].view.h,
            source.nodes.len()
        ));
    } else if checked > FIRST_CHECKS && checked.is_power_of_two() {
        log::line(&format!("{FIX}: self-check {checked} passed"));
    }
}

/// Up to three windows of [`CHECK_ROWS`] rows to check, around emitters a
/// quarter, half and three quarters into the node list (or the grid's
/// middle rows): `[lo, hi)` in grid rows.
fn windows(frame: &Frame, source: &Source<'_>) -> Vec<(i32, i32)> {
    let v = frame.kinds[0].view;
    let gy = frame.kinds[0].gp[1];
    let n = source.nodes.len();
    let mut out: Vec<(i32, i32)> = [n / 4, n / 2, 3 * n / 4]
        .into_iter()
        .map(|i| {
            let row = source
                .nodes
                .get(i)
                .and_then(|node| usize::try_from(node[1]).ok())
                .and_then(|c| source.data.get(c))
                .map(|e| e[1] / gy)
                .filter(|r| r.is_finite())
                .map_or(v.gy0 + v.h / 2, |r| r as i32);
            let rows = CHECK_ROWS.min(v.h);
            let lo = row
                .saturating_sub(CHECK_ROWS / 2)
                .clamp(v.gy0, v.gy0 + v.h - rows);
            (lo, lo + rows)
        })
        .collect();
    out.dedup();
    out
}

/// Phase times summed over the updates since the last report.
static PREPARE_NANOS: AtomicU64 = AtomicU64::new(0);
static SPLAT_NANOS: AtomicU64 = AtomicU64::new(0);
static MADE: AtomicU64 = AtomicU64::new(0);

fn note_banded(done: &Done, nanos: u64) {
    let total = BANDED_NANOS.fetch_add(nanos, Ordering::Relaxed) + nanos;
    let count = BANDED.fetch_add(1, Ordering::Relaxed) + 1;
    PREPARE_NANOS.fetch_add(done.prepare_ns, Ordering::Relaxed);
    SPLAT_NANOS.fetch_add(done.splat_ns, Ordering::Relaxed);
    MADE.fetch_add(done.emitters_made as u64, Ordering::Relaxed);
    if count.is_multiple_of(REPORT_EVERY) {
        let per = |c: &AtomicU64| c.swap(0, Ordering::Relaxed) as f64 / REPORT_EVERY as f64;
        log::line(&format!(
            "{FIX}: {count} updates banded, {:.2} ms each on average (last {REPORT_EVERY}: records {:.2} ms, bands {:.2} ms, {:.0} emitters made again per update);              last update: {} emitters, {} records ({} as lattices), {} of {} chunks made again; {} updates ran the game's way",
            total as f64 / count as f64 / 1e6,
            per(&PREPARE_NANOS) / 1e6,
            per(&SPLAT_NANOS) / 1e6,
            per(&MADE),
            done.emitters,
            done.records,
            done.lattices,
            done.chunks_made,
            done.chunks,
            GAMES_WAY.load(Ordering::Relaxed)
        ));
    }
}

fn note_games_way(why: &str) {
    static SAID: AtomicU64 = AtomicU64::new(0);
    let n = SAID.fetch_add(1, Ordering::Relaxed);
    if n < 4 || n.is_power_of_two() {
        log::line(&format!(
            "{FIX}: this update runs the game's way: {why} (update {})",
            n + 1
        ));
    }
}

fn broken(why: &str) {
    if !BROKEN.swap(true, Ordering::AcqRel) {
        log::line(&format!(
            "{FIX}: OFF for the rest of this game: {why}; the game's own splat runs from the next update"
        ));
    }
}

/// What installing came to, for `hook.log`.
pub fn outcome_line(installed: bool, reason: &str) -> String {
    if installed {
        format!("{FIX}: band splat installed ({reason})")
    } else {
        format!("{FIX}: the game's own splat, {reason}")
    }
}

/// Installs the band splat unless [`ENV`] turns it off. Returns the line
/// for `hook.log`.
pub fn install(resolved: &ResolvedProfile) -> String {
    match wanted(std::env::var(ENV).ok().as_deref()) {
        Ok(true) => match install_from_profile(resolved) {
            Ok(line) => outcome_line(true, &line),
            Err(why) => outcome_line(false, &why),
        },
        Ok(false) => outcome_line(false, &format!("{ENV}=0")),
        Err(why) => outcome_line(false, &format!("{ENV}: {why}")),
    }
}

/// Checks each stretch of [`CODE`] at `update2` against its hash.
pub fn check_code(
    update2: usize,
    bytes: &dyn Fn(usize, usize) -> Option<Vec<u8>>,
) -> Result<(), String> {
    for code in CODE {
        let at = update2.wrapping_add_signed(code.offset as isize);
        let found = bytes(at, code.len).ok_or_else(|| format!("{} is unreadable", code.what))?;
        if fnv1a(&found) != code.fnv1a {
            return Err(format!(
                "{} at {at:#x} is not the code the band splat models",
                code.what
            ));
        }
    }
    Ok(())
}

fn install_from_profile(resolved: &ResolvedProfile) -> Result<String, String> {
    if !(crate::emission::fused::avx() && std::arch::is_x86_feature_detected!("sse4.1")) {
        return Err("this CPU has no AVX".into());
    }
    let at = resolved
        .get(UPDATE2)
        .ok_or_else(|| format!("the profile has no {UPDATE2:?}"))?
        .address;
    let update2 = usize::try_from(at).map_err(|_| "an address past usize".to_owned())?;
    check_code(update2, &|at, len| {
        // SAFETY: read only after the region answered readable.
        crate::image::readable(at, len)
            .then(|| unsafe { std::slice::from_raw_parts(at as *const u8, len) }.to_vec())
    })?;
    let at = |offset: i64| update2.wrapping_add_signed(offset as isize);
    // SAFETY: the profile's `Update2`, whose code was just checked;
    // nothing runs it before the world exists.
    let redirects = unsafe {
        install_at(
            Game::from_update2(update2),
            [
                at(LAMBDA1_CALL),
                at(LAMBDA2_LOOP_CALL),
                at(LAMBDA2_POOL_CALL),
            ],
        )
    }?;
    let _kept = std::mem::ManuallyDrop::new(redirects);
    Ok(format!(
        "at {update2:#x}: by row bands on up to {} threads, bit-identical to the game's Update2; {ENV}=0 turns it off",
        crate::emission::MAX_THREADS
    ))
}

/// Redirects `Update2`'s three calls (`sites`: lambda_1's dispatcher,
/// lambda_2's loop, lambda_2's pool dispatcher) to the hook, with the
/// game's code at `game`.
///
/// # Safety
///
/// `sites` are `Update2`'s calls of `game`'s functions, which no thread
/// runs during this call.
pub(crate) unsafe fn install_at(
    game: Game,
    sites: [usize; 3],
) -> Result<Vec<tpf3mp_hookcore::detour::CallRedirect>, String> {
    *GAME.lock().unwrap_or_else(|p| p.into_inner()) = Some(game);
    BROKEN.store(false, Ordering::Release);
    let hooks: [(usize, usize, *const u8); 3] = [
        (sites[0], game.lambda1_dispatch, lambda1_hook as *const u8),
        (sites[1], game.lambda2_loop, lambda2_loop_hook as *const u8),
        (sites[2], game.lambda2_pool, lambda2_pool_hook as *const u8),
    ];
    let mut done = Vec::new();
    for (site, expected, to) in hooks {
        // SAFETY: the caller's; `to` has the callee's ABI.
        match unsafe {
            tpf3mp_hookcore::detour::CallRedirect::install(site as *mut u8, expected, to)
        } {
            Ok(redirect) => done.push(redirect),
            Err(error) => {
                // Dropping the ones made restores their calls.
                drop(done);
                return Err(format!("the call at {site:#x}: {error}"));
            }
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_that_differs_is_refused() {
        assert!(
            check_code(0x1000_0000, &|_, _| None)
                .unwrap_err()
                .contains("unreadable")
        );
        let err = check_code(0x1000_0000, &|_, len| Some(vec![0xCC; len])).unwrap_err();
        assert!(err.contains("Update2"), "{err}");
    }

    #[test]
    fn nothing_installs_without_the_target() {
        let empty = ResolvedProfile {
            name: "empty".into(),
            targets: Vec::new(),
            absent_optional: Vec::new(),
        };
        let line = install_from_profile(&empty).unwrap_err();
        assert!(
            line.contains("the profile has no") || line.contains("AVX"),
            "{line}"
        );
    }

    #[test]
    fn the_switch_turns_it_off() {
        assert!(outcome_line(false, &format!("{ENV}=0")).contains("the game's own splat"));
        assert!(outcome_line(true, "x").contains("installed"));
    }
}

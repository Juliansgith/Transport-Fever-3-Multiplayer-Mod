//! The splat by row bands, on the hook's threads ([`super::splat`] says
//! why the result is the game's).
//!
//! Two phases. First, by chunks of emitters in node order, each
//! emitter's records ([`record`]) and, by band and grid, the indices of
//! the records that reach the band (in node order). Second, band by band,
//! each band applies those records, chunk by chunk in order (so in node
//! order), to its own rows only. Bands are rows apart, so they run on any
//! threads in any order, the heaviest first. Nothing is written before
//! every record was made, so an update the model does not cover leaves
//! the grids untouched.
//!
//! A chunk's records are a function of the frame, the bands and the
//! seven floats of each of its emitters the update reads, alone: a chunk
//! whose inputs are bit for bit the last update's keeps its records.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::splat::{
    Arena, Axes, Emitter, Frame, Item, Rec, Uncovered, View, apply, record, regions,
};
use crate::emission::fused::{MXCSR_CONTROL, MXCSR_DEFAULT, mxcsr};
use crate::emission::pool::Pool;

/// The emitters `Update2` walks: its node list (`{entity, component
/// index}`, `[system+8]`) and the components' data (`[system+0x10]`).
#[derive(Debug, Clone, Copy)]
pub struct Source<'a> {
    pub nodes: &'a [[i32; 2]],
    pub data: &'a [Emitter],
}

/// Chunks per participating thread: every band walks every chunk, and an
/// emitter that changes makes its chunk be made again.
const CHUNKS_PER_THREAD: usize = 8;
/// Emitters per chunk, at least.
const MIN_CHUNK: usize = 256;
/// How far ahead a band prefetches the records it will apply.
const AHEAD: usize = 8;

/// The floats of an emitter the update reads (all but `[6]` and `[8]`),
/// as bits.
fn inputs(e: &Emitter) -> [u32; 7] {
    [0, 1, 2, 3, 4, 5, 7].map(|i| e[i].to_bits())
}

/// What a chunk's records depend on besides its emitters' floats: the
/// frame, the bands, and which emitters it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Key {
    origin: [i32; 2],
    quarter: [i32; 2],
    dt: u32,
    gp: [[u32; 2]; 2],
    factor: [u32; 2],
    grid: [i32; 4],
    logf: usize,
    rows: (i32, i32),
    band_rows: i32,
    first: usize,
    len: usize,
}

impl Key {
    fn new(frame: &Frame, bands: &Bands, first: usize, len: usize) -> Self {
        let v = frame.kinds[0].view;
        Self {
            origin: frame.origin,
            quarter: frame.quarter,
            dt: frame.dt.to_bits(),
            gp: frame.kinds.map(|k| k.gp.map(f32::to_bits)),
            factor: frame.kinds.map(|k| k.factor.to_bits()),
            grid: [v.gx0, v.gy0, v.w, v.h],
            logf: frame.logf as usize,
            rows: (bands.y0, bands.y0 + bands.h),
            band_rows: bands.rows,
            first,
            len,
        }
    }
}

/// One chunk: its records in node order, the bands each reaches, their
/// lines and additions, and by band and grid the records that reach it;
/// and what they were made from.
#[derive(Debug, Default)]
struct Chunk {
    key: Option<Key>,
    nodes: Vec<[i32; 2]>,
    inputs: Vec<[u32; 7]>,
    recs: Vec<Rec>,
    reach: Vec<(u32, u32)>,
    arena: Arena,
    /// `2 * bands + 1` offsets into `index` (band `b`, grid `k` at
    /// `2b + k`).
    offsets: Vec<u32>,
    /// Record indices by band and grid, in node order within each.
    index: Vec<u32>,
    /// Each band's work from this chunk, roughly (additions and
    /// samples), to start the heaviest bands first.
    band_cost: Vec<u32>,
    failed: Option<Uncovered>,
    /// Whether the last update made it again (else kept it).
    made: bool,
    /// Its records applied as lattices.
    lattices: usize,
}

/// The buffers one update's splat uses, kept between updates.
#[derive(Debug, Default)]
pub struct Plan {
    chunks: Vec<Chunk>,
    /// Tests: every record the scalar way.
    pub scalar: bool,
}

#[cfg(test)]
impl Plan {
    /// Tests: every chunk made again on the next run (buffers kept).
    pub fn forget(&mut self) {
        for chunk in &mut self.chunks {
            chunk.key = None;
        }
    }
}

/// What a run did, for hook.log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Done {
    pub emitters: usize,
    /// Emitters whose chunk was made again this update (the others kept
    /// the last update's records).
    pub emitters_made: usize,
    pub records: usize,
    /// Records applied as a lattice of samples (the others as their
    /// additions).
    pub lattices: usize,
    pub chunks: usize,
    pub chunks_made: usize,
    /// The two phases' wall times.
    pub prepare_ns: u64,
    pub splat_ns: u64,
}

/// The chunks while the first phase fills them: each index is claimed by
/// one thread (the shared counter), which alone writes that chunk.
struct Slots(*mut Chunk);

// SAFETY: see above.
unsafe impl Sync for Slots {}

impl Slots {
    /// # Safety
    ///
    /// `c` is in bounds and claimed by the calling thread alone.
    #[allow(clippy::mut_from_ref)]
    unsafe fn chunk(&self, c: usize) -> &mut Chunk {
        // SAFETY: the caller's.
        unsafe { &mut *self.0.add(c) }
    }
}

/// The band rows: `count` stretches of `rows` rows from `y0`, the last
/// one ending at `y0 + h`.
#[derive(Debug, Clone, Copy)]
struct Bands {
    y0: i32,
    h: i32,
    rows: i32,
    count: usize,
}

impl Bands {
    fn new(y_lo: i32, y_hi: i32, bands: usize) -> Self {
        let h = (y_hi - y_lo).max(1);
        let rows = (h as usize).div_ceil(bands.clamp(1, h as usize)) as i32;
        Self {
            y0: y_lo,
            h,
            rows,
            count: (h as usize).div_ceil(rows as usize),
        }
    }

    /// The bands rows `lo..=hi` touch, if any.
    fn covering(&self, lo: i32, hi: i32) -> Option<(u32, u32)> {
        let lo = i64::from(lo) - i64::from(self.y0);
        let hi = i64::from(hi) - i64::from(self.y0);
        if hi < 0 || lo >= i64::from(self.h) || hi < lo {
            return None;
        }
        let lo = lo.max(0) / i64::from(self.rows);
        let hi = hi.min(i64::from(self.h) - 1) / i64::from(self.rows);
        Some((lo as u32, hi as u32))
    }

    /// Band `b`'s rows `[lo, hi)`.
    fn rows(&self, b: usize) -> (i32, i32) {
        let lo = self.y0 + b as i32 * self.rows;
        (lo, (lo + self.rows).min(self.y0 + self.h))
    }
}

/// The frame and the emitters, shareable with the pool's threads.
struct Shared<'a> {
    frame: &'a Frame,
    source: Source<'a>,
    bands: Bands,
    chunk_len: usize,
    avx: bool,
}

// SAFETY: the views' pointers are only written through by one band at a
// time, rows apart (`run`).
unsafe impl Sync for Shared<'_> {}

/// Checks what the model needs of the grids: the same shape and rows for
/// both, buffers that hold those rows (`lens` floats from each view's
/// `data`), grid point sizes that are positive and finite, and regions
/// that split the plane.
pub fn covered(frame: &Frame, lens: [usize; 2]) -> Result<(), Uncovered> {
    let [a, b] = [frame.kinds[0].view, frame.kinds[1].view];
    if (a.gx0, a.gy0, a.w, a.h, a.y_lo, a.y_hi) != (b.gx0, b.gy0, b.w, b.h, b.y_lo, b.y_hi) {
        return Err(Uncovered("the two grids differ in shape".into()));
    }
    if a.w < 3 || a.h < 3 {
        return Err(Uncovered(format!("a {}x{} grid", a.w, a.h)));
    }
    let fits = |v: i64| v >= i64::from(i32::MIN) && v <= i64::from(i32::MAX);
    if !(i64::from(a.gy0) <= i64::from(a.y_lo)
        && a.y_lo < a.y_hi
        && i64::from(a.y_hi) <= i64::from(a.gy0) + i64::from(a.h))
    {
        return Err(Uncovered(format!(
            "rows {}..{} of a grid from row {}",
            a.y_lo, a.y_hi, a.gy0
        )));
    }
    let rows = (i64::from(a.y_hi) - i64::from(a.y_lo)) as u64;
    for (k, len) in lens.iter().enumerate() {
        if (*len as u64) < (a.w as u64) * rows {
            return Err(Uncovered(format!("grid {k} holds {len} floats")));
        }
        let gp = frame.kinds[k].gp;
        if !(gp[0] > 0.0 && gp[1] > 0.0 && gp[0].is_finite() && gp[1].is_finite()) {
            return Err(Uncovered(format!("a grid point size of {gp:?}")));
        }
    }
    // The cells' coordinates, the inner bounds and the regions' bounds
    // must not wrap, and each region must be non-empty: then the 16
    // regions split the plane, and a cell is added to in one of them only.
    for axis in 0..2 {
        let (g0, n) = if axis == 0 {
            (a.gx0, a.w)
        } else {
            (a.gy0, a.h)
        };
        let (o, q) = (frame.origin[axis], frame.quarter[axis]);
        if q < 1 {
            return Err(Uncovered(format!("regions {q} cells wide")));
        }
        if !fits(i64::from(g0) + i64::from(n) + 1)
            || !fits(i64::from(g0) - 1)
            || !fits(i64::from(o) + 4 * i64::from(q))
        {
            return Err(Uncovered("coordinates that wrap".into()));
        }
    }
    Ok(())
}

/// Splats every emitter of `source` into the rows `[y_lo, y_hi)` of
/// `frame`'s views (the whole grids, or a window of them), as the game's
/// `Update2` would: on `pool`'s threads (or here), in `bands` row bands.
/// `lens` is how many floats each view's `data` holds. `Err`, with
/// nothing written, unless the model covers every emitter; `Ok(None)` if
/// a band panicked while writing (the grids are then half done).
///
/// # Safety
///
/// The views' `data` hold those rows of both grids, used by nothing else
/// meanwhile; `frame.logf` is the C runtime's `logf`.
pub unsafe fn run(
    frame: &Frame,
    source: &Source<'_>,
    lens: [usize; 2],
    bands: usize,
    pool: Option<&Pool>,
    plan: &mut Plan,
) -> Result<Option<Done>, Uncovered> {
    if !std::arch::is_x86_feature_detected!("sse4.1") {
        return Err(Uncovered("this CPU has no SSE4.1".into()));
    }
    covered(frame, lens)?;
    let view = frame.kinds[0].view;
    let bands = Bands::new(view.y_lo, view.y_hi, bands);
    let participants = pool.map_or(1, Pool::participants);
    let n = source.nodes.len();
    let chunk_len = n.div_ceil(CHUNKS_PER_THREAD * participants).max(MIN_CHUNK);
    let chunks = n.div_ceil(chunk_len);
    if plan.chunks.len() < chunks {
        plan.chunks.resize_with(chunks, Chunk::default);
    }
    let shared = Shared {
        frame,
        source: *source,
        bands,
        chunk_len,
        avx: std::arch::is_x86_feature_detected!("avx") && !plan.scalar,
    };
    let started = std::time::Instant::now();
    let slots = Slots(plan.chunks.as_mut_ptr());
    let ok = participate(pool, &|next: &dyn Fn() -> usize| {
        loop {
            let c = next();
            if c >= chunks {
                break;
            }
            // SAFETY: chunk `c` is this thread's alone (the counter); the
            // caller's logf; SSE4.1 checked above.
            unsafe { prepare(&shared, c, slots.chunk(c)) };
        }
    })?;
    let chunk_list = &plan.chunks[..chunks];
    if !ok {
        return Err(Uncovered("a thread of the first phase panicked".into()));
    }
    if let Some(why) = chunk_list.iter().find_map(|c| c.failed.clone()) {
        return Err(why);
    }
    // The heaviest bands first, so no thread is left with a heavy one
    // at the end.
    let mut cost = vec![0u64; bands.count];
    for chunk in chunk_list {
        for (c, &w) in cost.iter_mut().zip(&chunk.band_cost) {
            *c += u64::from(w);
        }
    }
    let mut order: Vec<usize> = (0..bands.count).collect();
    order.sort_unstable_by_key(|&b| std::cmp::Reverse(cost[b]));
    let prepared = std::time::Instant::now();
    let ok = participate(pool, &|next: &dyn Fn() -> usize| {
        while let Some(&b) = order.get(next()) {
            // SAFETY: band `b`'s rows, which no other band touches; SSE4.1
            // checked above.
            unsafe { splat_band(&shared, chunk_list, b) };
        }
    })?;
    let splat_ns = prepared.elapsed().as_nanos() as u64;
    let done = Done {
        emitters: n,
        emitters_made: chunk_list
            .iter()
            .filter(|c| c.made)
            .map(|c| c.nodes.len())
            .sum(),
        records: chunk_list.iter().map(|c| c.recs.len()).sum(),
        lattices: chunk_list.iter().map(|c| c.lattices).sum(),
        chunks,
        chunks_made: chunk_list.iter().filter(|c| c.made).count(),
        prepare_ns: (prepared - started).as_nanos() as u64,
        splat_ns,
    };
    Ok(ok.then_some(done))
}

/// Runs `job` on every participant (the pool's threads and this one),
/// each taking work from a shared counter; `Err`, before any job ran, if
/// a participant's MXCSR is not the default. `Ok(false)` if one panicked.
fn participate(
    pool: Option<&Pool>,
    job: &(dyn Fn(&dyn Fn() -> usize) + Sync),
) -> Result<bool, Uncovered> {
    let counter = AtomicUsize::new(0);
    let next = || counter.fetch_add(1, Ordering::Relaxed);
    let Some(pool) = pool.filter(|p| p.participants() > 1) else {
        if mxcsr() & MXCSR_CONTROL != MXCSR_DEFAULT {
            return Err(Uncovered(format!("MXCSR is {:#x}", mxcsr())));
        }
        return Ok(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&next))).is_ok());
    };
    let bad = AtomicBool::new(false);
    let barrier = std::sync::Barrier::new(pool.participants());
    let finished = pool.run(&|_| {
        if mxcsr() & MXCSR_CONTROL != MXCSR_DEFAULT {
            bad.store(true, Ordering::SeqCst);
        }
        barrier.wait();
        if bad.load(Ordering::SeqCst) {
            return;
        }
        job(&next);
    });
    if bad.load(Ordering::SeqCst) {
        return Err(Uncovered("a thread's MXCSR is not the default".into()));
    }
    Ok(finished)
}

/// The first phase for chunk `c`: unless its inputs are the last
/// update's, its emitters' records and their indices by band and grid.
///
/// # Safety
///
/// `shared.frame.logf` is the C runtime's `logf`; the CPU has SSE4.1.
#[target_feature(enable = "sse4.1")]
unsafe fn prepare(shared: &Shared<'_>, c: usize, chunk: &mut Chunk) {
    let Shared {
        frame,
        source,
        bands,
        chunk_len,
        ..
    } = shared;
    let first = c * chunk_len;
    let nodes = &source.nodes[first..(first + chunk_len).min(source.nodes.len())];
    let key = Key::new(frame, bands, first, nodes.len());
    chunk.made = false;
    // The emitters, read once: kept if every input is the last update's.
    let mut same = chunk.failed.is_none() && chunk.key == Some(key) && chunk.nodes == nodes;
    for (i, node) in nodes.iter().enumerate() {
        // movsxd [nodes + i*8 + 4]: the component's index.
        let Some(e) = usize::try_from(node[1])
            .ok()
            .and_then(|i| source.data.get(i))
        else {
            chunk.key = None;
            chunk.failed = Some(Uncovered(format!(
                "a node names component {} of {}",
                node[1],
                source.data.len()
            )));
            return;
        };
        if same && chunk.inputs.get(i) != Some(&inputs(e)) {
            same = false;
        }
    }
    if same {
        return;
    }
    chunk.made = true;
    chunk.key = None;
    chunk.failed = None;
    chunk.nodes.clear();
    chunk.nodes.extend_from_slice(nodes);
    chunk.inputs.clear();
    chunk.recs.clear();
    chunk.reach.clear();
    chunk.arena.lines.clear();
    chunk.arena.cells.clear();
    let axes = Axes::new(frame);
    for node in nodes {
        // Checked above.
        let e = &source.data[node[1] as usize];
        chunk.inputs.push(inputs(e));
        let regions = regions(frame, e);
        for kind in 0..2 {
            // SAFETY: the caller's.
            match unsafe { record(frame, &axes, e, kind, regions, &mut chunk.arena) } {
                Ok(Some(rec)) => {
                    if let Some(reach) = bands.covering(rec.reach.0, rec.reach.1) {
                        chunk.recs.push(rec);
                        chunk.reach.push(reach);
                    } else {
                        match rec.item {
                            Item::Radius { at, .. } => chunk.arena.lines.truncate(at as usize),
                            Item::Cells { at, .. } => chunk.arena.cells.truncate(at as usize),
                        }
                    }
                }
                Ok(None) => {}
                Err(why) => {
                    chunk.failed = Some(why);
                    return;
                }
            }
        }
    }
    index_by_band(chunk, bands);
    chunk.lattices = chunk
        .recs
        .iter()
        .filter(|r| matches!(r.item, Item::Radius { .. }))
        .count();
    chunk.key = Some(key);
}

/// A counting sort of the chunk's record indices by band and grid, node
/// order kept in each; and each band's rough cost.
fn index_by_band(chunk: &mut Chunk, bands: &Bands) {
    let slots = 2 * bands.count;
    chunk.offsets.clear();
    chunk.offsets.resize(slots + 1, 0);
    chunk.band_cost.clear();
    chunk.band_cost.resize(bands.count, 0);
    for (rec, &(lo, hi)) in chunk.recs.iter().zip(&chunk.reach) {
        let cost = match rec.item {
            Item::Cells { n, .. } => 8 + n / (hi - lo + 1),
            Item::Radius { ncols, nrows, .. } => 8 + 4 * ncols * nrows.min(bands.rows as u32 + 1),
        };
        for b in lo..=hi {
            chunk.offsets[2 * b as usize + usize::from(rec.kind) + 1] += 1;
            chunk.band_cost[b as usize] = chunk.band_cost[b as usize].saturating_add(cost);
        }
    }
    for s in 0..slots {
        chunk.offsets[s + 1] += chunk.offsets[s];
    }
    chunk.index.clear();
    chunk.index.resize(chunk.offsets[slots] as usize, 0);
    let mut at: Vec<u32> = chunk.offsets[..slots].to_vec();
    for (i, (rec, &(lo, hi))) in chunk.recs.iter().zip(&chunk.reach).enumerate() {
        for b in lo..=hi {
            let slot = 2 * b as usize + usize::from(rec.kind);
            chunk.index[at[slot] as usize] = i as u32;
            at[slot] += 1;
        }
    }
}

/// The second phase for band `b`: every record that reaches it, chunk by
/// chunk, in order; per chunk the noise grid's, then the pollution
/// grid's (the two grids are apart, so which goes first is free).
///
/// # Safety
///
/// No other thread writes band `b`'s rows; the CPU has SSE4.1.
#[target_feature(enable = "sse4.1")]
unsafe fn splat_band(shared: &Shared<'_>, chunks: &[Chunk], b: usize) {
    use std::arch::x86_64::{_MM_HINT_T0, _mm_prefetch};
    let (lo, hi) = shared.bands.rows(b);
    let frame = shared.frame;
    let views = frame.kinds.map(|k| {
        let v = k.view;
        View {
            // SAFETY: row `lo` of the view's rows.
            data: unsafe { v.data.add((lo - v.y_lo) as usize * v.w as usize) },
            y_lo: lo,
            y_hi: hi,
            ..v
        }
    });
    for chunk in chunks {
        for (kind, view) in views.iter().enumerate() {
            let slot = 2 * b + kind;
            let index =
                &chunk.index[chunk.offsets[slot] as usize..chunk.offsets[slot + 1] as usize];
            for (k, &i) in index.iter().enumerate() {
                if let Some(&ahead) = index.get(k + AHEAD) {
                    let rec = &chunk.recs[ahead as usize];
                    // A hint only, at a record and its data.
                    {
                        _mm_prefetch::<_MM_HINT_T0>((rec as *const Rec).cast());
                        let data = match rec.item {
                            Item::Cells { at, .. } => {
                                chunk.arena.cells.as_ptr().wrapping_add(at as usize).cast()
                            }
                            Item::Radius { at, .. } => {
                                chunk.arena.lines.as_ptr().wrapping_add(at as usize).cast()
                            }
                        };
                        _mm_prefetch::<_MM_HINT_T0>(data);
                    }
                }
                let rec = &chunk.recs[i as usize];
                // SAFETY: band `b`'s rows of the record's grid.
                unsafe { apply(&rec.item, &chunk.arena, view, shared.avx) };
            }
        }
    }
}

/// Test only: phase A and phase B separately, `rounds` times each on this
/// thread, their best cycle counts (`cached`: phase A with its inputs
/// unchanged).
///
/// # Safety
///
/// As [`run`], for the whole grids.
#[cfg(test)]
pub unsafe fn bench_phases(
    frame: &Frame,
    source: &Source<'_>,
    bands: usize,
    rounds: usize,
    cached: bool,
) -> (u64, u64) {
    let view = frame.kinds[0].view;
    let bands = Bands::new(view.y_lo, view.y_hi, bands);
    let chunk_len = source
        .nodes
        .len()
        .div_ceil(CHUNKS_PER_THREAD * 16)
        .max(MIN_CHUNK);
    let chunks = source.nodes.len().div_ceil(chunk_len);
    let mut plan = Plan::default();
    plan.chunks.resize_with(chunks, Chunk::default);
    let shared = Shared {
        frame,
        source: *source,
        bands,
        chunk_len,
        avx: std::arch::is_x86_feature_detected!("avx"),
    };
    let (mut a, mut b) = (u64::MAX, u64::MAX);
    for _ in 0..rounds {
        if !cached {
            for chunk in &mut plan.chunks {
                chunk.key = None;
            }
        }
        let t = test_cycles::now();
        for (c, chunk) in plan.chunks.iter_mut().enumerate() {
            // SAFETY: the caller's.
            unsafe { prepare(&shared, c, chunk) };
        }
        a = a.min(test_cycles::now() - t);
        let t = test_cycles::now();
        for band in 0..bands.count {
            // SAFETY: the caller's; one band at a time.
            unsafe { splat_band(&shared, &plan.chunks, band) };
        }
        b = b.min(test_cycles::now() - t);
    }
    (a, b)
}

#[cfg(test)]
pub mod test_cycles {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn QueryThreadCycleTime(thread: isize, cycles: *mut u64) -> i32;
        fn GetCurrentThread() -> isize;
    }

    /// The CPU cycles this thread has run (not the time it waited).
    pub fn now() -> u64 {
        let mut c = 0;
        // SAFETY: the current thread's pseudo handle and a u64 to fill.
        unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut c) };
        c
    }
}

//! The emitters' splat into the noise and pollution grids, as the game
//! computes it, reorganised by grid rows
//! (investigation/TF3_EMITTER_SPLAT_2026-10-06.md).
//!
//! `ecs::EmissionEmitterSystem::Update2` sorts the emitters into the 16
//! regions of a 4 x 4 split of the grid (lambda_1, `0x140aa4310`, by the
//! larger of the two radii) and then, in 32 tasks (2 grids x 16 regions),
//! walks each region's emitters in ascending node order and adds each into
//! the cells of that region (lambda_2's body `0x140aa4650` and the insert
//! helpers `0x140ba1e40`, `0x140ba2230`, `0x140ba1f40`). Here every
//! emitter is prepared once ([`record`]: its value, and the columns and
//! rows of its samples with the cells the game keeps), then applied band
//! by band ([`apply`]), each band in the same ascending order and keeping
//! only its own rows. A cell lies in one region and one band, so it
//! receives the same additions in the same order; each addition is the
//! game's single-precision sequence: the result is the game's to the bit.
//!
//! The comments name the game's instructions each step repeats.

#![allow(unsafe_code)]

/// `logf`, the C runtime's, as the game imports it.
pub type Logf = unsafe extern "C" fn(f32) -> f32;

/// One emitter's nine floats (`ecs::component::EmissionEmitter`):
/// position x, y; a distance; the noise radius, the pollution radius; the
/// noise power; one float the update does not read; the pollution power;
/// another it does not read.
pub type Emitter = [f32; 9];

/// The game's `1e-7f` (`0x1436f3950`): the smallest power and value that
/// counts.
pub const EPSILON: f32 = f32::from_bits(0x33d6_bf95);

/// The most samples along one axis the model follows; an emitter with
/// more (or one whose sample loop would not end) is left to the game.
pub const MAX_SPAN: usize = 1 << 12;

/// A grid the splat writes: the concentration `Grid<float>` of one
/// `EmissionGrid` component (`x0`, `y0`, width, height), and the rows of
/// it this view may touch, `[y_lo, y_hi)` in grid coordinates.
#[derive(Debug, Clone, Copy)]
pub struct View {
    pub gx0: i32,
    pub gy0: i32,
    pub w: i32,
    pub h: i32,
    /// The cell at grid row `y_lo`, column `gx0`.
    pub data: *mut f32,
    pub y_lo: i32,
    pub y_hi: i32,
}

/// One grid's inputs: its component's grid point size, the factor
/// `Update2` computed for it (`0x140aa8ba0`), and where to write.
#[derive(Debug, Clone, Copy)]
pub struct Kind {
    pub gp: [f32; 2],
    pub factor: f32,
    pub view: View,
}

/// One update's inputs besides the emitters: the inner grid's origin
/// (`X0`, `Y0`) and a quarter of its size (the regions' size), `dt`, the
/// noise grid then the pollution grid.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub origin: [i32; 2],
    pub quarter: [i32; 2],
    pub dt: f32,
    pub kinds: [Kind; 2],
    pub logf: Logf,
}

/// Why the model does not cover an update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uncovered(pub String);

/// `vroundss` (floor) then `vcvttss2si`: `i32::MIN` for NaN and out of
/// range, as the instructions give.
#[inline(always)]
fn floor_i32(x: f32) -> i32 {
    use std::arch::x86_64::{_mm_cvttss_si32, _mm_set_ss};
    // SAFETY: SSE is part of x86_64.
    unsafe { _mm_cvttss_si32(_mm_set_ss(x.floor())) }
}

/// `vmaxss s, 0`: `s` if above zero, else `+0` (NaN and `-0` included).
#[inline(always)]
fn clamp0(s: f32) -> f32 {
    if s > 0.0 { s } else { 0.0 }
}

/// The 4 x 4 regions' bounds along one axis, `[lo, hi)` each
/// (`0x140aa4ab0`): the first starts at `i32::MIN`, the last ends at
/// `i32::MAX`, the others at `origin + k * quarter`.
pub fn bounds(origin: i32, quarter: i32) -> [(i32, i32); 4] {
    let at = |k: i32| k.wrapping_mul(quarter).wrapping_add(origin);
    [
        (i32::MIN, at(1)),
        (at(1), at(2)),
        (at(2), at(3)),
        (at(3), i32::MAX),
    ]
}

/// The region of cell `c` along one axis as lambda_1 buckets it:
/// `(c - origin) / quarter`, truncated (`idiv`), clamped to 0..=3. For a
/// quarter of at least 1 (as `bands::covered` requires) that is the
/// comparisons below.
#[inline(always)]
fn region(c: i32, origin: i32, quarter: i32) -> u8 {
    let d = i64::from(c.wrapping_sub(origin));
    let q = i64::from(quarter);
    if d < q {
        0
    } else if d < 2 * q {
        1
    } else if d < 3 * q {
        2
    } else {
        3
    }
}

/// `0x140ba1ba0`: the cell of a position, `floor(p / gp - 0.5)`.
#[inline(always)]
fn cell(x: f32, y: f32, gp: [f32; 2]) -> (i32, i32) {
    (floor_i32(x / gp[0] - 0.5), floor_i32(y / gp[1] - 0.5))
}

/// The regions an emitter is bucketed in, `[x first, x last, y first,
/// y last]`, by the larger of its two radii and the noise grid's point
/// size (lambda_1, `0x140aa4310`).
#[inline(always)]
pub fn regions(frame: &Frame, e: &Emitter) -> [u8; 4] {
    let gp = frame.kinds[0].gp;
    let (px, py) = (e[0], e[1]);
    // vcmpltss + vblendvps: the pollution radius if the noise radius is
    // smaller, else the noise radius.
    let r = if e[3] < e[4] { e[4] } else { e[3] };
    let lo = cell(px - r, py - r, gp);
    let hi = if r > 0.0 {
        cell(r + px, r + py, gp)
    } else {
        (lo.0.wrapping_add(1), lo.1.wrapping_add(1))
    };
    let [ox, oy] = frame.origin;
    let [qx, qy] = frame.quarter;
    [
        region(lo.0, ox, qx),
        region(hi.0, ox, qx),
        region(lo.1, oy, qy),
        region(hi.1, oy, qy),
    ]
}

/// The bounds of the regions and of the inner cells along one axis.
#[derive(Debug, Clone, Copy)]
pub struct Axis {
    bounds: [(i32, i32); 4],
    /// Inner cells: `lo < c < hi` (`0x140ba1f40`).
    inner: (i32, i32),
    is_x: bool,
}

/// Both axes of a frame (both grids have the same shape).
#[derive(Debug, Clone, Copy)]
pub struct Axes {
    x: Axis,
    y: Axis,
}

impl Axes {
    pub fn new(frame: &Frame) -> Self {
        let v = frame.kinds[0].view;
        Self {
            x: Axis {
                bounds: bounds(frame.origin[0], frame.quarter[0]),
                inner: (v.gx0, (v.w - 1).wrapping_add(v.gx0)),
                is_x: true,
            },
            y: Axis {
                bounds: bounds(frame.origin[1], frame.quarter[1]),
                inner: (v.gy0, (v.h - 1).wrapping_add(v.gy0)),
                is_x: false,
            },
        }
    }
}

impl Axis {
    /// Which of a sample's two cells along this axis the game keeps (bit
    /// 0: cell `i`, bit 1: cell `i + 1`): a cell counts if one of the
    /// regions `first..=last` the emitter is bucketed in contains it and
    /// lets the sample in (`x`: `i + 1 >= lo && i < hi`, `y`: `i >= lo &&
    /// i + 1 < hi`, as `0x140ba1e40` and `0x140ba2230` test before calling
    /// `0x140ba1f40`), and it is an inner cell (`0x140ba1f40`).
    #[inline(always)]
    fn keeps(&self, i: i32, first: u8, last: u8) -> u8 {
        let next = i.wrapping_add(1);
        let mut keep = 0u8;
        for k in first..=last.min(3) {
            let (lo, hi) = self.bounds[usize::from(k)];
            let enters = if self.is_x {
                next >= lo && i < hi
            } else {
                i >= lo && next < hi
            };
            if enters {
                keep |= u8::from(lo <= i && i < hi) | (u8::from(lo <= next && next < hi) << 1);
            }
        }
        let inner = |c: i32| c > self.inner.0 && c < self.inner.1;
        keep & (u8::from(inner(i)) | (u8::from(inner(next)) << 1))
    }
}

/// One column (or row) of an emitter's samples: the cell `i`, the
/// sample's weight towards it (`a = 1 - (s - i)`, or `b`), the offset
/// squared (`dx*dx`, or `dy*dy`), and which of `i`, `i + 1` the game
/// keeps.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Line {
    pub i: i32,
    pub f: f32,
    pub dd: f32,
    pub keep: u8,
}

/// What one emitter adds to one grid (lambda_2's body, `0x140aa4650`, up
/// to the insert call, and the insert's samples), and where.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rec {
    /// 0 noise, 1 pollution.
    pub kind: u8,
    /// The grid rows it adds to: the first and the last.
    pub reach: (i32, i32),
    pub item: Item,
}

/// What a band applies of a record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Item {
    /// The game's additions themselves, in its order: `n` [`Cell`]s from
    /// `at` in the arena. Every point, and a radius record of at most
    /// [`MAX_CELLS`] additions.
    Cells { at: u32, n: u32 },
    /// `0x140ba2230`: samples on a lattice, kept within the radius
    /// (`rr >= dx*dx + dy*dy`), each spreading `w`; `ncols` columns then
    /// `nrows` rows in the arena from `at`.
    Radius {
        rr: f32,
        w: f32,
        at: u32,
        ncols: u32,
        nrows: u32,
        /// Whether the columns' cells are `i0, i0 + 1, ...` (then a row of
        /// samples can be applied eight cells at a time, [`apply_avx`]).
        consecutive: bool,
    },
}

impl Item {
    /// The arena lines it uses.
    pub fn lines(&self) -> u32 {
        match *self {
            Item::Cells { .. } => 0,
            Item::Radius { ncols, nrows, .. } => ncols + nrows,
        }
    }

    /// The same, its lines or cells at `to` in another arena (and `n`
    /// cells).
    pub fn moved(self, to: u32, n: u32) -> Self {
        match self {
            Item::Radius {
                rr,
                w,
                ncols,
                nrows,
                consecutive,
                ..
            } => Item::Radius {
                rr,
                w,
                at: to,
                ncols,
                nrows,
                consecutive,
            },
            Item::Cells { .. } => Item::Cells { at: to, n },
        }
    }
}

/// One of the game's additions: `max(value + cell, 0)` at `(x, y)`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Cell {
    pub x: i32,
    pub y: i32,
    pub w: f32,
}

/// A record of at most this many additions keeps them as such.
pub const MAX_CELLS: usize = 64;

/// The columns and rows of radius records, each record's columns then
/// its rows; the additions of the others.
#[derive(Debug, Default)]
pub struct Arena {
    pub lines: Vec<Line>,
    pub cells: Vec<Cell>,
}

/// One sample's additions, `0x140ba1f40`'s four corners in its order:
/// `(a*b)*v` at `(x, y)`, `((1-b)*a)*v` at `(x, y+1)`, `((1-a)*b)*v` at
/// `(x+1, y)`, `((1-a)*(1-b))*v` at `(x+1, y+1)`, each where the game
/// keeps the column (`c.keep`) and the row (`ky`).
#[inline(always)]
fn corners(c: &Line, r: &Line, ky: u8, v: f32, mut add: impl FnMut(i32, i32, f32)) {
    let (a, b) = (c.f, r.f);
    let (x, y) = (c.i, r.i);
    if c.keep & 1 != 0 {
        if ky & 1 != 0 {
            add(x, y, (a * b) * v);
        }
        if ky & 2 != 0 {
            add(x, y.wrapping_add(1), ((1.0 - b) * a) * v);
        }
    }
    if c.keep & 2 != 0 {
        if ky & 1 != 0 {
            add(x.wrapping_add(1), y, ((1.0 - a) * b) * v);
        }
        if ky & 2 != 0 {
            add(
                x.wrapping_add(1),
                y.wrapping_add(1),
                ((1.0 - a) * (1.0 - b)) * v,
            );
        }
    }
}

/// A radius record's samples (`0x140ba2230`: rows, then the columns of
/// each, within the radius), each with the game's additions; `ky_of`
/// narrows a row's kept cells further (a band's rows).
#[inline(always)]
fn samples(
    rr: f32,
    v: f32,
    cols: &[Line],
    rows: &[Line],
    ky_of: impl Fn(&Line) -> u8,
    mut add: impl FnMut(i32, i32, f32),
) {
    for r in rows {
        let ky = ky_of(r);
        if ky == 0 {
            continue;
        }
        for c in cols {
            // vcomiss rr, dx*dx + dy*dy; jb: outside the radius (NaN
            // too, hence not `rr < ...`).
            #[allow(clippy::neg_cmp_op_on_partial_ord)]
            if c.keep == 0 || !(rr >= c.dd + r.dd) {
                continue;
            }
            corners(c, r, ky, v, &mut add);
        }
    }
}

/// The samples along one axis of a radius splat (`0x140ba2230`'s loops:
/// `d = -r`, then `d += g` while `r >= d`), each made a [`Line`] by
/// `line`; `false` past [`MAX_SPAN`] (a loop the model does not follow).
#[inline(always)]
fn lattice(r: f32, g: f32, out: &mut Vec<Line>, mut line: impl FnMut(f32) -> Line) -> bool {
    let mut d = -r;
    for _ in 0..MAX_SPAN {
        out.push(line(d));
        d += g;
        // Not `r < d`: a NaN ends the loop too, as `jae` falls through.
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        if !(r >= d) {
            return true;
        }
    }
    false
}

/// The rows `rows` add to, the first and the last; `None` if they add to
/// none.
fn reach(rows: &[Line]) -> Option<(i32, i32)> {
    let mut lo = i32::MAX;
    let mut hi = i32::MIN;
    for r in rows {
        if r.keep & 1 != 0 {
            lo = lo.min(r.i);
            hi = hi.max(r.i);
        }
        if r.keep & 2 != 0 {
            lo = lo.min(r.i.wrapping_add(1));
            hi = hi.max(r.i.wrapping_add(1));
        }
    }
    (lo <= hi).then_some((lo, hi))
}

/// Lambda_2's body for emitter `e` and grid `kind`: the record of what it
/// adds, or `None` when it adds nothing. A radius record's columns and
/// rows go to `arena`.
///
/// # Safety
///
/// `frame.logf` is the C runtime's `logf`; the CPU has SSE4.1.
#[target_feature(enable = "sse4.1")]
pub unsafe fn record(
    frame: &Frame,
    axes: &Axes,
    e: &Emitter,
    kind: usize,
    regions: [u8; 4],
    arena: &mut Arena,
) -> Result<Option<Rec>, Uncovered> {
    let k = &frame.kinds[kind];
    let dist = e[2];
    // vcomiss 0, dist; ja: an emitter at a negative distance adds nothing.
    if 0.0 > dist {
        return Ok(None);
    }
    let power = if kind == 0 { e[5] } else { e[7] };
    // vandps |power|; vmaxss |power|, 1e-7; vdivss 1e-7, that; logf.
    let abs = power.abs();
    let m = if abs > EPSILON { abs } else { EPSILON };
    // SAFETY: the caller's: the game's logf.
    let log = unsafe { (frame.logf)(EPSILON / m) };
    // vmulss by the grid's factor, then by 4: the distance it reaches.
    let reach_m = (log * k.factor) * 4.0;
    // The share: 0 unless dist >= 0 and the reach > 0; else 1 - dist /
    // reach, below 0 is 0, else vminss 1, share.
    let mut share = 0.0f32;
    if dist >= 0.0 && reach_m > 0.0 {
        let s = 1.0 - dist / reach_m;
        // As the game's branches have it (f32::clamp agrees, but this is
        // the instruction sequence).
        #[allow(clippy::manual_clamp)]
        let clamped = if 0.0 > s {
            0.0
        } else if 1.0 < s {
            1.0
        } else {
            s
        };
        share = clamped;
    }
    let v = (share * frame.dt) * power;
    // vcomiss 1e-7, |v|; jae: too small to add.
    if EPSILON >= v.abs() {
        return Ok(None);
    }
    let r = if kind == 0 { e[3] } else { e[4] };
    let (px, py) = (e[0], e[1]);
    let [gx, gy] = k.gp;
    let [rx0, rx1, ry0, ry1] = regions;
    let col = |sx: f32, dd: f32| {
        let i = floor_i32(sx);
        Line {
            i,
            f: 1.0 - (sx - i as f32),
            dd,
            keep: axes.x.keeps(i, rx0, rx1),
        }
    };
    let row = |sy: f32, dd: f32| {
        let i = floor_i32(sy);
        Line {
            i,
            f: 1.0 - (sy - i as f32),
            dd,
            keep: axes.y.keeps(i, ry0, ry1),
        }
    };
    // Not `r <= 0`: a NaN radius is a point too (`jbe`).
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if !(r > 0.0) {
        // 0x140ba1e40: px / gx - 0.5, py / gy - 0.5.
        let x = col(px / gx - 0.5, 0.0);
        let y = row(py / gy - 0.5, 0.0);
        if x.keep == 0 {
            return Ok(None);
        }
        let Some(reach) = reach(std::slice::from_ref(&y)) else {
            return Ok(None);
        };
        let at = arena.cells.len();
        corners(&x, &y, y.keep, v, |x, y, w| {
            arena.cells.push(Cell { x, y, w })
        });
        return Ok(Some(Rec {
            kind: kind as u8,
            reach,
            item: Item::Cells {
                at: at as u32,
                n: (arena.cells.len() - at) as u32,
            },
        }));
    }
    // 0x140ba2230: inv = 1 / (gx * gy); each sample spreads inv * v;
    // samples at (dx + px) / gx - 0.5 and (py + dy) / gy - 0.5.
    let inv = 1.0 / (gx * gy);
    let w = inv * v;
    let c0 = arena.lines.len();
    let cols_done = lattice(r, gx, &mut arena.lines, |dx| {
        col((dx + px) / gx - 0.5, dx * dx)
    });
    let r0 = arena.lines.len();
    if !(cols_done
        && lattice(r, gy, &mut arena.lines, |dy| {
            row((py + dy) / gy - 0.5, dy * dy)
        }))
    {
        return Err(Uncovered(format!(
            "an emitter of radius {r} m spans more than {MAX_SPAN} cells"
        )));
    }
    let found = arena.lines[c0..r0]
        .iter()
        .any(|c| c.keep != 0)
        .then(|| reach(&arena.lines[r0..]));
    let Some(Some(reach)) = found else {
        arena.lines.truncate(c0);
        return Ok(None);
    };
    let cols = &arena.lines[c0..r0];
    let rows = &arena.lines[r0..];
    // Few samples (four additions each at most): their additions kept as
    // such.
    if cols.len() * rows.len() * 4 <= MAX_CELLS {
        let at = arena.cells.len();
        samples(
            r * r,
            w,
            cols,
            rows,
            |r| r.keep,
            |x, y, w| arena.cells.push(Cell { x, y, w }),
        );
        let n = arena.cells.len() - at;
        arena.lines.truncate(c0);
        return Ok(Some(Rec {
            kind: kind as u8,
            reach,
            item: Item::Cells {
                at: at as u32,
                n: n as u32,
            },
        }));
    }
    let consecutive = cols
        .iter()
        .enumerate()
        .all(|(k, c)| i64::from(c.i) == i64::from(cols[0].i) + k as i64);
    Ok(Some(Rec {
        kind: kind as u8,
        reach,
        item: Item::Radius {
            rr: r * r,
            w,
            at: c0 as u32,
            ncols: (r0 - c0) as u32,
            nrows: (arena.lines.len() - r0) as u32,
            consecutive,
        },
    }))
}

/// The fewest columns a record has for [`apply_avx`] to pay.
pub const AVX_COLUMNS: u32 = 4;

/// Adds `item` into `view`'s rows, as the game's helpers would for every
/// region the emitter is bucketed in; `avx`: a radius record with
/// consecutive columns goes eight cells at a time ([`apply_avx`]).
///
/// # Safety
///
/// `view` describes rows of the record's grid that the caller alone
/// writes; `arena` is the one the item was made in; `avx` only if the CPU
/// has AVX.
#[inline(always)]
pub unsafe fn apply(item: &Item, arena: &Arena, view: &View, avx: bool) {
    let width = view.w as isize;
    match *item {
        Item::Cells { at, n } => {
            for c in &arena.cells[at as usize..(at + n) as usize] {
                if c.y < view.y_lo || c.y >= view.y_hi {
                    continue;
                }
                let idx = (c.y - view.y_lo) as isize * width + (c.x - view.gx0) as isize;
                // SAFETY: an addition the game keeps is to an inner cell;
                // its row is one of the view's.
                unsafe {
                    let at = view.data.offset(idx);
                    *at = clamp0(c.w + *at);
                }
            }
        }
        Item::Radius {
            rr,
            w,
            at,
            ncols,
            nrows,
            consecutive,
        } => {
            let lines = &arena.lines;
            let cols = &lines[at as usize..(at + ncols) as usize];
            let rows = &lines[(at + ncols) as usize..(at + ncols + nrows) as usize];
            if consecutive && avx && ncols >= AVX_COLUMNS && ncols as usize <= AVX_MAX_COLUMNS {
                // SAFETY: the caller's; AVX.
                unsafe { apply_avx(rr, w, cols, rows, view) };
                return;
            }
            let band = |r: &Line| {
                let next = r.i.wrapping_add(1);
                r.keep
                    & (u8::from(r.i >= view.y_lo && r.i < view.y_hi)
                        | (u8::from(next >= view.y_lo && next < view.y_hi) << 1))
            };
            samples(rr, w, cols, rows, band, |x, y, weight| {
                let idx = (y - view.y_lo) as isize * width + (x - view.gx0) as isize;
                // SAFETY: a kept cell is an inner cell, and `band` keeps
                // the view's rows only: inside the caller's rows.
                unsafe {
                    let at = view.data.offset(idx);
                    *at = clamp0(weight + *at);
                }
            });
        }
    }
}

/// The most columns [`apply_avx`] takes (a record with more goes the
/// scalar way).
pub const AVX_MAX_COLUMNS: usize = 64;

/// A radius record whose columns are consecutive cells `i0 .. i0 + n`,
/// applied a row of samples at a time, eight cells per instruction.
///
/// Within one row of samples, the game's order (`0x140ba2230` then
/// `0x140ba1f40`: sample k's corners (i_k, y), (i_k, y+1), (i_k+1, y),
/// (i_k+1, y+1), then sample k+1's) gives cell `i0 + t` of row `y` at
/// most two additions: sample t-1's third corner, then sample t's first;
/// of row `y + 1`, sample t-1's fourth, then sample t's second. Nothing
/// else in that row of samples touches it, and rows of samples go in
/// order. So each cell is `max(c3 + c, 0)` then `max(c1 + that, 0)`, each
/// only where the game keeps that corner (the sample within the radius,
/// the cell kept), with the game's operations in every lane; a cell where
/// neither is kept is written back unchanged (it is in the caller's
/// rows).
///
/// # Safety
///
/// As [`apply`]; the CPU has AVX; `cols` are consecutive cells, at most
/// [`AVX_MAX_COLUMNS`].
#[target_feature(enable = "avx")]
unsafe fn apply_avx(rr: f32, w: f32, cols: &[Line], rows: &[Line], view: &View) {
    use std::arch::x86_64::*;
    const N: usize = AVX_MAX_COLUMNS + 16;
    let n = cols.len();
    debug_assert!(n <= AVX_MAX_COLUMNS);
    let i0 = cols[0].i;
    // The cells t = 0 ..= n (i0 + t) inside the grid's row; masks are
    // only set on inner cells, which are inside it.
    let t_lo = (i64::from(view.gx0) - i64::from(i0)).clamp(0, n as i64 + 1) as usize;
    let t_hi =
        (i64::from(view.gx0) + i64::from(view.w) - i64::from(i0)).clamp(0, n as i64 + 1) as usize;
    if t_lo >= t_hi {
        return;
    }
    // The columns, transposed (lanes past n: outside the radius, kept by
    // nothing).
    let mut a = [0.0f32; N];
    let mut dd = [f32::INFINITY; N];
    let mut k1 = [0.0f32; N];
    let mut k2 = [0.0f32; N];
    let on = f32::from_bits(u32::MAX);
    for (j, c) in cols.iter().enumerate() {
        a[j] = c.f;
        dd[j] = c.dd;
        k1[j] = if c.keep & 1 != 0 { on } else { 0.0 };
        k2[j] = if c.keep & 2 != 0 { on } else { 0.0 };
    }
    // Per row of cells: the weights and masks by cell t.
    let mut w1 = [0.0f32; N];
    let mut m1 = [0.0f32; N];
    let mut w3 = [0.0f32; N];
    let mut m3 = [0.0f32; N];
    let ones = _mm256_set1_ps(1.0);
    let zero = _mm256_setzero_ps();
    let wv = _mm256_set1_ps(w);
    let rrv = _mm256_set1_ps(rr);
    let width = view.w as isize;
    for r in rows {
        let next = r.i.wrapping_add(1);
        let band = u8::from(r.i >= view.y_lo && r.i < view.y_hi)
            | (u8::from(next >= view.y_lo && next < view.y_hi) << 1);
        let ky = r.keep & band;
        if ky == 0 {
            continue;
        }
        let b = _mm256_set1_ps(r.f);
        let omb = _mm256_sub_ps(ones, b);
        let ddy = _mm256_set1_ps(r.dd);
        for (y, bit) in [(r.i, 1u8), (next, 2u8)] {
            if ky & bit == 0 {
                continue;
            }
            // The columns' weights and masks for this row of cells: row
            // y takes corners 1 and 3, row y+1 corners 2 and 4.
            let mut j = 0;
            while j < n {
                // SAFETY: j + 9 <= n + 8 < N: inside the arrays.
                unsafe {
                    let av = _mm256_loadu_ps(a.as_ptr().add(j));
                    let oma = _mm256_sub_ps(ones, av);
                    // vcomiss rr, dx*dx + dy*dy: inside if rr >= it.
                    let inside = _mm256_cmp_ps::<_CMP_GE_OQ>(
                        rrv,
                        _mm256_add_ps(_mm256_loadu_ps(dd.as_ptr().add(j)), ddy),
                    );
                    let (first, third) = if bit == 1 {
                        // (a*b)*v and ((1-a)*b)*v.
                        (_mm256_mul_ps(av, b), _mm256_mul_ps(oma, b))
                    } else {
                        // ((1-b)*a)*v and ((1-a)*(1-b))*v.
                        (_mm256_mul_ps(omb, av), _mm256_mul_ps(oma, omb))
                    };
                    _mm256_storeu_ps(w1.as_mut_ptr().add(j), _mm256_mul_ps(first, wv));
                    _mm256_storeu_ps(w3.as_mut_ptr().add(j + 1), _mm256_mul_ps(third, wv));
                    _mm256_storeu_ps(
                        m1.as_mut_ptr().add(j),
                        _mm256_and_ps(inside, _mm256_loadu_ps(k1.as_ptr().add(j))),
                    );
                    _mm256_storeu_ps(
                        m3.as_mut_ptr().add(j + 1),
                        _mm256_and_ps(inside, _mm256_loadu_ps(k2.as_ptr().add(j))),
                    );
                }
                j += 8;
            }
            // Cell 0 takes no third corner; cell n no first (lanes past n
            // are already off: outside the radius).
            m3[0] = 0.0;
            m1[n] = 0.0;
            let row = (i64::from(y) - i64::from(view.y_lo)) as isize * width
                + (i64::from(i0) - i64::from(view.gx0)) as isize;
            // The cells i0 + t_lo .. i0 + t_hi of row y: in the caller's
            // rows (ky) and in the grid's row (t_lo, t_hi).
            let cells = view.data.wrapping_offset(row);
            let mut t = t_lo;
            while t + 8 <= t_hi {
                // SAFETY: as above.
                unsafe {
                    let at = cells.add(t);
                    let mut v = _mm256_loadu_ps(at);
                    let s3 =
                        _mm256_max_ps(_mm256_add_ps(_mm256_loadu_ps(w3.as_ptr().add(t)), v), zero);
                    v = _mm256_blendv_ps(v, s3, _mm256_loadu_ps(m3.as_ptr().add(t)));
                    let s1 =
                        _mm256_max_ps(_mm256_add_ps(_mm256_loadu_ps(w1.as_ptr().add(t)), v), zero);
                    v = _mm256_blendv_ps(v, s1, _mm256_loadu_ps(m1.as_ptr().add(t)));
                    _mm256_storeu_ps(at, v);
                }
                t += 8;
            }
            while t < t_hi {
                // SAFETY: as above.
                unsafe {
                    let at = cells.add(t);
                    let mut v = *at;
                    if m3[t].to_bits() != 0 {
                        v = clamp0(w3[t] + v);
                    }
                    if m1[t].to_bits() != 0 {
                        v = clamp0(w1[t] + v);
                    }
                    *at = v;
                }
                t += 1;
            }
        }
    }
}

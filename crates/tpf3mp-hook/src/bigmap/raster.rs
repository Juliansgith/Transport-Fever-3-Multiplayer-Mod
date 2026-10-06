//! The street and obstacle raster's cell, grown where 1 m overflows.
//!
//! `Obstacle::Obstacle` (0x8cea50) rasterises a box at a cell size its
//! caller passes, 1 m for the town connections and the street initialiser:
//! `n = floor(extent / cell) + 1` cells a side, sized through a 32-bit
//! multiply. Past 2³¹ − 1 cells (180 tiles square, or 1000 x 32 long) the
//! product wraps, `vector<bool>::resize` throws `length_error` and the game
//! aborts with no message (TPF2's 180-tile wall, which TF3 kept).
//!
//! With [`RASTER_ENV`] on, the constructor is detoured: where the cell
//! passed would overflow, it is doubled until the count fits, as
//! tpf2-bigmap's `street_raster` grows it (silver2127, MIT; no code taken).
//! Two differences, both for TF3 and a room:
//!
//! - **Only where stock code would abort.** tpf2-bigmap grew the cell past
//!   a 1.5-billion budget; here a count that fits int32 is never touched,
//!   so every world a stock game can generate keeps its cells, and a game
//!   with the setting agrees with one without on all of them.
//! - **Powers of two.** The raster's fill scales by `extent / (n − 1)`,
//!   lookups divide by the cell; they agree exactly only when the cell
//!   divides the extent. Tile edges are 256 m, so 2, 4 and 8 m always do,
//!   and the divisions stay exact in f32
//!   (investigation/TF3_BIGMAPS_256KM_2026-10-05.md §2).
//!
//! The multiply is never widened: every index downstream is int32, and
//! the coarser cell keeps them all in range.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use tpf3mp_hookcore::profile::ResolvedProfile;

pub use crate::build_data::native::bigmap::{OBSTACLE, OBSTACLE_BODY, OBSTACLE_BODY_OFFSET};
use crate::log;

/// The patch's name in `hook.log`.
pub const FIX: &str = "big maps: street raster";

/// `1` (or `on`) in the game's environment grows the raster's cell where
/// the count would overflow; unset or `0` leaves the game's own.
pub const RASTER_ENV: &str = "TPF3MP_BIGMAP_STREET_RASTER";

/// The largest factor the cell is grown by: 64 m cells cover far more
/// than the heightmap allows.
pub const MAX_GROWTH: f32 = 64.0;

/// The constructor's count a side, as its code computes it: the extent
/// over the cell in f32, truncated, one less below zero, plus one. `None`
/// where `vcvttss2si` would not give a number (a NaN or past int32).
pub fn side(extent: f32, cell: f32) -> Option<i64> {
    let q = extent / cell;
    if !q.is_finite() || q.abs() >= 2_147_483_648.0 {
        return None;
    }
    // In range: truncation toward zero, as `vcvttss2si`.
    let t = q as i32;
    let t = if q < 0.0 { t.wrapping_sub(1) } else { t };
    Some(i64::from(t) + 1)
}

/// The constructor's cell count for `bbox` (`{minX, minY, maxX, maxY}`) at
/// `cell`.
pub fn cells(bbox: [f32; 4], cell: f32) -> Option<i64> {
    let x = side(bbox[2] - bbox[0], cell)?;
    let y = side(bbox[3] - bbox[1], cell)?;
    Some(x * y)
}

/// The cell to build `bbox` with: `None` keeps the caller's (it fits, or
/// nothing this patch can do helps), `Some` the smallest power-of-two
/// multiple of it whose count fits int32.
pub fn grown_cell(bbox: [f32; 4], cell: f32) -> Option<f32> {
    if !(cell.is_finite() && cell > 0.0) {
        return None;
    }
    let fits = |c: f32| cells(bbox, c).is_some_and(|n| (0..=i64::from(i32::MAX)).contains(&n));
    if cells(bbox, cell).is_none() || fits(cell) {
        return None;
    }
    let mut factor = 2.0f32;
    while factor <= MAX_GROWTH {
        let grown = cell * factor;
        if fits(grown) {
            return Some(grown);
        }
        factor *= 2.0;
    }
    None
}

/// `Obstacle::Obstacle(this, box, cell) -> this`.
type CtorFn = unsafe extern "system" fn(usize, *const [f32; 4], f32) -> usize;

/// The trampoline to the game's constructor; 0 until installed.
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static GROWN: AtomicU64 = AtomicU64::new(0);
static BROKEN: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn ctor(this: usize, bbox: *const [f32; 4], cell: f32) -> usize {
    let original = ORIGINAL.load(Ordering::Acquire);
    let mut use_cell = cell;
    if !BROKEN.load(Ordering::Acquire) && !bbox.is_null() {
        // SAFETY: the box the game passes, 16 readable bytes (the
        // constructor reads them itself).
        let corners = unsafe { std::ptr::read_unaligned(bbox) };
        let decided = std::panic::catch_unwind(|| grown_cell(corners, cell));
        match decided {
            Ok(Some(grown)) => {
                use_cell = grown;
                let n = GROWN.fetch_add(1, Ordering::Relaxed) + 1;
                if n <= 8 || n.is_multiple_of(256) {
                    log::line(&format!(
                        "{FIX}: {:.0} x {:.0} m at {cell} m overflows 2^31 cells; built at {grown} m ({} cells; grown {n} times)",
                        corners[2] - corners[0],
                        corners[3] - corners[1],
                        cells(corners, grown).unwrap_or_default()
                    ));
                }
            }
            Ok(None) => {}
            Err(_) => BROKEN.store(true, Ordering::Release),
        }
    }
    // SAFETY: the trampoline InlineDetour::install returned for the
    // constructor, called with its own arguments and the cell decided.
    unsafe { std::mem::transmute::<usize, CtorFn>(original)(this, bbox, use_cell) }
}

/// What installing came to, for `hook.log`.
pub fn outcome_line(installed: bool, reason: &str) -> String {
    if installed {
        format!("{FIX}: installed ({reason})")
    } else {
        format!("{FIX}: off, {reason}")
    }
}

/// Installs the patch if [`RASTER_ENV`] asks for it. Returns the line for
/// `hook.log`.
pub fn install(resolved: &ResolvedProfile) -> String {
    match super::switch(std::env::var(RASTER_ENV).ok().as_deref()) {
        Ok(wanted) => install_with(resolved, wanted),
        Err(why) => outcome_line(false, &format!("{RASTER_ENV}: {why}")),
    }
}

/// The constructor's body must be the one the cell rule models.
pub fn check_body(bytes: &[u8]) -> Result<(), String> {
    let body = bytes
        .get(OBSTACLE_BODY_OFFSET..OBSTACLE_BODY_OFFSET + OBSTACLE_BODY.len())
        .ok_or_else(|| "the constructor is too short".to_owned())?;
    if body != OBSTACLE_BODY {
        return Err(format!(
            "{OBSTACLE}'s body is not the one the cell rule was read on"
        ));
    }
    Ok(())
}

pub fn install_with(resolved: &ResolvedProfile, wanted: bool) -> String {
    match install_detour(resolved, wanted) {
        Ok(detour) => {
            let at = detour.target() as usize;
            let _kept = std::mem::ManuallyDrop::new(detour);
            outcome_line(
                true,
                &format!(
                    "at {at:#x}, a cell that would overflow 2^31 cells is doubled until it fits; every other raster is the game's"
                ),
            )
        }
        Err(why) => outcome_line(false, &why),
    }
}

pub(crate) fn install_detour(
    resolved: &ResolvedProfile,
    wanted: bool,
) -> Result<tpf3mp_hookcore::detour::InlineDetour, String> {
    if !wanted {
        return Err(format!("{RASTER_ENV} is not on; the game's own cells"));
    }
    let at = resolved
        .get(OBSTACLE)
        .ok_or_else(|| format!("the profile has no {OBSTACLE:?}"))?
        .address;
    let address = usize::try_from(at).map_err(|_| "an address past usize".to_owned())?;
    let len = OBSTACLE_BODY_OFFSET + OBSTACLE_BODY.len();
    if !crate::image::readable(address, len) {
        return Err(format!("{OBSTACLE} at {at:#x} is unreadable"));
    }
    // SAFETY: `len` readable bytes, checked just above.
    check_body(unsafe { std::slice::from_raw_parts(address as *const u8, len) })?;
    // SAFETY: the constructor the profile resolved, whose prologue it
    // checked; no world exists yet; `ctor` has its ABI.
    let detour = unsafe {
        tpf3mp_hookcore::detour::InlineDetour::install(address as *mut u8, ctor as *const u8)
    }
    .map_err(|error| format!("{OBSTACLE} at {at:#x}: {error}"))?;
    ORIGINAL.store(detour.trampoline() as usize, Ordering::Release);
    Ok(detour)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map of `x` by `y` tiles, centred, as the town connections pass it.
    fn map(x: u32, y: u32) -> [f32; 4] {
        let (hx, hy) = (x as f32 * 128.0, y as f32 * 128.0);
        [-hx, -hy, hx, hy]
    }

    #[test]
    fn a_count_that_fits_is_never_touched() {
        for (x, y) in [
            (112, 112),
            (50, 250),
            (176, 176),
            (180, 180),
            (1000, 32),
            (2, 2),
        ] {
            assert_eq!(grown_cell(map(x, y), 1.0), None, "{x} x {y}");
        }
        assert_eq!(cells(map(180, 180), 1.0), Some(46_081 * 46_081));
        assert_eq!(cells(map(1000, 32), 1.0), Some(256_001 * 8_193));
    }

    #[test]
    fn past_the_wall_the_cell_doubles_until_it_fits() {
        assert_eq!(grown_cell(map(182, 182), 1.0), Some(2.0));
        assert_eq!(grown_cell(map(300, 300), 1.0), Some(2.0));
        assert_eq!(grown_cell(map(1000, 40), 1.0), Some(2.0));
        assert_eq!(grown_cell(map(1000, 88), 1.0), Some(2.0));
        assert_eq!(grown_cell(map(1000, 136), 1.0), Some(4.0));
        assert_eq!(grown_cell(map(1000, 186), 1.0), Some(4.0));
        assert_eq!(grown_cell(map(512, 512), 1.0), Some(4.0));
        // From another caller's cell, by the same factors.
        assert_eq!(grown_cell(map(300, 300), 0.5), Some(2.0));
        for (x, y) in [(182, 182), (1000, 186), (512, 512)] {
            let grown = grown_cell(map(x, y), 1.0).unwrap();
            let n = cells(map(x, y), grown).unwrap();
            assert!(n <= i64::from(i32::MAX), "{x} x {y}");
            // Half the cell would not have fitted: the smallest that does.
            assert!(cells(map(x, y), grown / 2.0).unwrap() > i64::from(i32::MAX));
            // The cell divides the extent exactly.
            assert_eq!((x as f32 * 256.0) % grown, 0.0);
        }
    }

    #[test]
    fn what_cannot_be_helped_is_left_to_the_game() {
        assert_eq!(grown_cell(map(300, 300), 0.0), None);
        assert_eq!(grown_cell(map(300, 300), f32::NAN), None);
        assert_eq!(grown_cell([0.0, 0.0, f32::INFINITY, 1.0], 1.0), None);
        // A box no cell up to 64 m brings under 2^31.
        assert_eq!(grown_cell([0.0, 0.0, 4.0e9, 4.0e9], 1.0), None);
        // Inverted boxes count as the constructor counts them.
        assert_eq!(side(-1.5, 1.0), Some(-1));
        assert_eq!(side(0.0, 1.0), Some(1));
    }

    #[test]
    fn nothing_installs_when_off_or_without_the_target() {
        let empty = ResolvedProfile {
            name: "empty".into(),
            targets: Vec::new(),
            absent_optional: Vec::new(),
        };
        assert!(install_with(&empty, false).contains(RASTER_ENV));
        assert!(install_with(&empty, true).contains("the profile has no"));
        let mut body = vec![0u8; OBSTACLE_BODY_OFFSET];
        body.extend_from_slice(&OBSTACLE_BODY);
        assert_eq!(check_body(&body), Ok(()));
        body[OBSTACLE_BODY_OFFSET + 3] ^= 1;
        assert!(check_body(&body).is_err());
        assert!(check_body(&body[..20]).is_err());
    }
}

/// The game's own constructor, relocated from the executable and run:
/// the count model against its code, and the detour on it.
#[cfg(all(test, windows, target_arch = "x86_64"))]
#[allow(clippy::unwrap_used)]
mod original_tests {
    use std::sync::Mutex;

    use tpf3mp_hookcore::profile::ResolvedTarget;

    use super::*;
    use crate::bigmap::original::{Exe, Page};

    const CTOR_RVA: u64 = 0x8cea50;
    const CTOR_LEN: usize = 173;
    const VFTABLE_RVA: u64 = 0x36e6b80;
    const RESIZE_RVA: u64 = 0x39d750;

    static SERIAL: Mutex<()> = Mutex::new(());
    /// The size the constructor asked its `vector<bool>` for.
    static ASKED: AtomicU64 = AtomicU64::new(0);

    extern "system" fn resize(_vector: usize, size: u64, _fill: usize) {
        ASKED.store(size, Ordering::SeqCst);
    }

    type Ctor = unsafe extern "system" fn(usize, *const [f32; 4], f32) -> usize;

    fn relocated(exe: &Exe, page: &mut Page) -> usize {
        let vftable = page.data(&[0u8; 8]);
        let stub = page.data(&(resize as *const () as u64).to_le_bytes());
        // A jump through the slot, so the relocated call stays near.
        let jump = page.code(&[0xFF, 0x25, 0, 0, 0, 0]);
        let rel = (stub as i64 - (jump as i64 + 6)) as i32;
        // SAFETY: the jump's displacement, in the page just written.
        unsafe {
            std::ptr::copy_nonoverlapping(rel.to_le_bytes().as_ptr(), (jump + 2) as *mut u8, 4)
        };
        let (at, _) = page.relocate(exe, CTOR_RVA, CTOR_LEN, &|target| match target {
            VFTABLE_RVA => Some(vftable),
            RESIZE_RVA => Some(jump),
            _ => None,
        });
        at
    }

    /// Runs the constructor; the cell it stored, its counts and the size it
    /// asked for.
    fn build(at: usize, bbox: [f32; 4], cell: f32) -> (f32, i32, i32, u64) {
        let mut object = [0u8; 0x48];
        // SAFETY: the relocated constructor, on a buffer of its object's size.
        let ctor = unsafe { std::mem::transmute::<usize, Ctor>(at) };
        unsafe { ctor(object.as_mut_ptr() as usize, &bbox, cell) };
        let f = |at: usize| object[at..at + 4].try_into().unwrap();
        (
            f32::from_le_bytes(f(0x18)),
            i32::from_le_bytes(f(0x40)),
            i32::from_le_bytes(f(0x44)),
            ASKED.load(Ordering::SeqCst),
        )
    }

    #[test]
    fn the_count_model_is_the_constructors() {
        let Some(exe) = Exe::load() else { return };
        let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(
            exe.bytes(CTOR_RVA + OBSTACLE_BODY_OFFSET as u64, OBSTACLE_BODY.len()),
            OBSTACLE_BODY
        );
        let mut page = Page::new();
        let at = relocated(&exe, &mut page);
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut checked = 0;
        for _ in 0..20_000 {
            let r = |v: u64, span: f32| (v % 1_000_000) as f32 / 1_000_000.0 * span;
            let x0 = r(next(), 200_000.0) - 100_000.0;
            let y0 = r(next(), 200_000.0) - 100_000.0;
            let bbox = [
                x0,
                y0,
                x0 + r(next(), 140_000.0) - 1_000.0,
                y0 + r(next(), 140_000.0) - 1_000.0,
            ];
            let cell = [0.5, 1.0, 2.0, 3.0, 4.0, 0.25][(next() % 6) as usize];
            let (stored, nx, ny, asked) = build(at, bbox, cell);
            assert_eq!(stored, cell);
            let (mx, my) = (
                side(bbox[2] - bbox[0], cell).unwrap(),
                side(bbox[3] - bbox[1], cell).unwrap(),
            );
            assert_eq!((i64::from(nx), i64::from(ny)), (mx, my), "{bbox:?} {cell}");
            let n = mx * my;
            // The game sizes with a wrapping 32-bit product, sign-extended.
            assert_eq!(asked as i64, i64::from(n as i32), "{bbox:?} {cell}");
            if n == i64::from(n as i32) {
                checked += 1;
            }
        }
        assert!(checked > 1000);
        // The wall itself: 182 tiles square wraps negative.
        let (_, _, _, asked) = build(at, [-23_296.0, -23_296.0, 23_296.0, 23_296.0], 1.0);
        assert!((asked as i64) < 0);
    }

    #[test]
    fn the_detour_grows_only_what_would_overflow() {
        let Some(exe) = Exe::load() else { return };
        let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        let mut page = Page::new();
        let at = relocated(&exe, &mut page);
        let resolved = ResolvedProfile {
            name: "relocated".into(),
            targets: vec![ResolvedTarget {
                name: OBSTACLE.into(),
                address: at as u64,
                image_index: 0,
                required: false,
            }],
            absent_optional: Vec::new(),
        };
        BROKEN.store(false, Ordering::Release);
        let detour = install_detour(&resolved, true).unwrap();
        // Gigantomaniac 1:5 and Stage 1's 176²: the game's 1 m cells.
        let gig = [-6_400.0, -32_000.0, 6_400.0, 32_000.0];
        assert_eq!(build(at, gig, 1.0), (1.0, 12_801, 64_001, 12_801 * 64_001));
        let s176 = [-22_528.0, -22_528.0, 22_528.0, 22_528.0];
        assert_eq!(build(at, s176, 1.0).0, 1.0);
        // 300² and 1000 x 186: 2 m and 4 m, sized below 2^31.
        let (cell, nx, ny, asked) = build(at, [-38_400.0, -38_400.0, 38_400.0, 38_400.0], 1.0);
        assert_eq!((cell, nx, ny), (2.0, 38_401, 38_401));
        assert_eq!(asked, 38_401 * 38_401);
        let (cell, nx, ny, asked) = build(at, [-128_000.0, -23_808.0, 128_000.0, 23_808.0], 1.0);
        assert_eq!((cell, nx, ny), (4.0, 64_001, 11_905));
        assert_eq!(asked, 64_001 * 11_905);
        // SAFETY: nothing runs the relocated constructor now.
        unsafe { detour.detach() }.unwrap();
        ORIGINAL.store(0, Ordering::Release);
        // A body that differs installs nothing.
        let mut other = resolved.clone();
        other.targets[0].address += 1;
        assert!(install_detour(&other, true).is_err());
    }
}

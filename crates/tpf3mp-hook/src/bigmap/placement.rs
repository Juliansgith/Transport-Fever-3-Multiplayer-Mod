//! Town and industry placement spacing in 64 bits.
//!
//! `RandomLocationFactory` scores each candidate's spacing (0x8d31f0): the
//! nearest other candidate or exclusion, as `dx·dx + dy·dy` in **int32**
//! heightmap pixels (4 m), then `sqrt(float(d) · resolution²)` and
//! `minimum / distance`, or 99,999 when closer than the minimum. Pairs more
//! than 46,340 px (185.4 km) apart wrap: negative squares give NaN scores,
//! wrapped positives false proximity, and towns cluster along a long map's
//! middle (tpf2-bigmap `docs/placement-distance.md`, silver2127, MIT; no
//! code taken). Runtime industry founding scores through the same code.
//!
//! With [`PLACEMENT_ENV`] on, the score is replaced by [`score`]: the same
//! arithmetic with the squares in 64 bits, saturated at `INT_MAX`, the
//! stock minimum's start. Where no square overflows, every score is the
//! game's to the bit (the tests run the game's own code against it), so a
//! game with the setting agrees with one without on every map whose pairs
//! stay within 185 km: every map up to 724 tiles long.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicU64, Ordering};

use tpf3mp_hookcore::profile::ResolvedProfile;

pub use crate::build_data::native::bigmap::{SPACING, SPACING_SQUARES, SPACING_TOO_CLOSE};
use crate::log;

/// The patch's name in `hook.log`.
pub const FIX: &str = "big maps: placement spacing";

/// `1` (or `on`) in the game's environment replaces the spacing score;
/// unset or `0` leaves the game's own.
pub const PLACEMENT_ENV: &str = "TPF3MP_BIGMAP_PLACEMENT";

/// A candidate: heightmap pixels and an angle.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub x: i32,
    pub y: i32,
    pub angle: f32,
}

/// An exclusion: heightmap pixels.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exclusion {
    pub x: i32,
    pub y: i32,
}

/// MSVC's `std::vector`: three pointers.
#[repr(C)]
#[derive(Debug)]
pub struct RawVec<T> {
    pub begin: *mut T,
    pub end: *mut T,
    pub capacity: *mut T,
}

/// `dx·dx + dy·dy` without wrapping, saturated at `INT_MAX`. A component
/// past 46,340 alone squares past `INT_MAX`, so it saturates before any
/// 64-bit sum could overflow.
pub fn distance_squared(ax: i32, ay: i32, bx: i32, by: i32) -> i32 {
    let dx = i64::from(ax) - i64::from(bx);
    let dy = i64::from(ay) - i64::from(by);
    if dx.abs() > 46_340 || dy.abs() > 46_340 {
        return i32::MAX;
    }
    i32::try_from(dx * dx + dy * dy).unwrap_or(i32::MAX)
}

/// The spacing scores, one per candidate, as the game computes them but
/// with [`distance_squared`]: each candidate's nearest later candidate
/// (folded into that one's nearest too), then the exclusions, then
/// `sqrt(float(nearest) · resolution²)`.
pub fn score(
    candidates: &[Candidate],
    exclusions: &[Exclusion],
    minimum: f32,
    resolution: f32,
    out: &mut [f32],
) {
    let resolution_squared = resolution * resolution;
    let mut nearest = vec![i32::MAX; candidates.len()];
    for (i, a) in candidates.iter().enumerate() {
        let mut best = i32::MAX;
        for (j, b) in candidates.iter().enumerate().skip(i + 1) {
            let d = distance_squared(a.x, a.y, b.x, b.y);
            best = best.min(d);
            nearest[j] = nearest[j].min(d);
        }
        best = best.min(nearest[i]);
        for e in exclusions {
            best = best.min(distance_squared(a.x, a.y, e.x, e.y));
        }
        let distance = (best as f32 * resolution_squared).sqrt();
        out[i] = if minimum > distance {
            SPACING_TOO_CLOSE
        } else {
            minimum / distance
        };
    }
}

static CALLS: AtomicU64 = AtomicU64::new(0);

/// A borrowed game vector as a slice; empty when it is not one.
///
/// # Safety
///
/// `vector` is null or a live `std::vector<T>` of the game's.
unsafe fn slice<'a, T>(vector: *const RawVec<T>) -> &'a [T] {
    if vector.is_null() {
        return &[];
    }
    // SAFETY: the caller's contract: a live vector.
    let vector = unsafe { &*vector };
    if vector.begin.is_null() || vector.end < vector.begin {
        return &[];
    }
    // SAFETY: a vector's elements, begin to end.
    let len = unsafe { vector.end.offset_from(vector.begin) } as usize;
    unsafe { std::slice::from_raw_parts(vector.begin, len) }
}

/// The game's spacing score, replaced: the same arguments (Windows x64:
/// `rcx`, `rdx`, `xmm2`, `r9`, the stack).
unsafe extern "system" fn spacing(
    out: *const RawVec<f32>,
    candidates: *const RawVec<Candidate>,
    minimum: f32,
    exclusions: *const RawVec<Exclusion>,
    resolution: f32,
) {
    // SAFETY: the game's vectors, borrowed for the call, as the game's own
    // score borrows them; the output has a slot per candidate (the game's
    // score writes one per candidate without checking either).
    let (candidates, exclusions) = unsafe { (slice(candidates), slice(exclusions)) };
    if candidates.is_empty() || out.is_null() {
        return;
    }
    // SAFETY: as above.
    let out = unsafe { std::slice::from_raw_parts_mut((*out).begin, candidates.len()) };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        score(candidates, exclusions, minimum, resolution, out);
    }))
    .is_err()
    {
        out.fill(SPACING_TOO_CLOSE);
        log::line(&format!(
            "{FIX}: panicked; these candidates scored as too close"
        ));
    }
    if CALLS.fetch_add(1, Ordering::Relaxed) == 0 {
        log::line(&format!(
            "{FIX}: first scores, {} candidates, {} exclusions",
            candidates.len(),
            exclusions.len()
        ));
    }
}

/// What installing came to, for `hook.log`.
pub fn outcome_line(installed: bool, reason: &str) -> String {
    if installed {
        format!("{FIX}: installed ({reason})")
    } else {
        format!("{FIX}: off, {reason}")
    }
}

/// Installs the patch if [`PLACEMENT_ENV`] asks for it. Returns the line
/// for `hook.log`.
pub fn install(resolved: &ResolvedProfile) -> String {
    match super::switch(std::env::var(PLACEMENT_ENV).ok().as_deref()) {
        Ok(wanted) => install_with(resolved, wanted),
        Err(why) => outcome_line(false, &format!("{PLACEMENT_ENV}: {why}")),
    }
}

/// The score's two squarings must be where they were read.
pub fn check_body(bytes: &[u8]) -> Result<(), String> {
    for (at, expected) in SPACING_SQUARES {
        if bytes.get(at..at + expected.len()) != Some(&expected[..]) {
            return Err(format!(
                "{SPACING}'s int32 squares are not at +{at:#x}; not the score this replaces"
            ));
        }
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
                &format!("at {at:#x}, separations in 64 bits, saturated at INT_MAX"),
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
        return Err(format!("{PLACEMENT_ENV} is not on; the game's own score"));
    }
    let at = resolved
        .get(SPACING)
        .ok_or_else(|| format!("the profile has no {SPACING:?}"))?
        .address;
    let address = usize::try_from(at).map_err(|_| "an address past usize".to_owned())?;
    let len = SPACING_SQUARES
        .iter()
        .map(|(at, bytes)| at + bytes.len())
        .max()
        .unwrap_or_default();
    if !crate::image::readable(address, len) {
        return Err(format!("{SPACING} at {at:#x} is unreadable"));
    }
    // SAFETY: `len` readable bytes, checked just above.
    check_body(unsafe { std::slice::from_raw_parts(address as *const u8, len) })?;
    // SAFETY: the score the profile resolved, whose prologue it checked;
    // no world exists yet; `spacing` has its ABI and never calls back.
    unsafe {
        tpf3mp_hookcore::detour::InlineDetour::install(address as *mut u8, spacing as *const u8)
    }
    .map_err(|error| format!("{SPACING} at {at:#x}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squares_saturate_instead_of_wrapping() {
        assert_eq!(distance_squared(0, 0, 3, 4), 25);
        assert_eq!(distance_squared(0, 0, 46_340, 0), 46_340 * 46_340);
        assert_eq!(distance_squared(0, 0, 46_341, 0), i32::MAX);
        assert_eq!(distance_squared(0, 0, 40_000, 40_000), i32::MAX);
        assert_eq!(
            distance_squared(i32::MIN, i32::MIN, i32::MAX, i32::MAX),
            i32::MAX
        );
        // 256 km at 4 m: 64,000 px, which the game's int32 wraps negative.
        assert_eq!(64_000i32.wrapping_mul(64_000), -198_967_296);
        assert_eq!(distance_squared(-32_000, 0, 32_000, 0), i32::MAX);
    }

    #[test]
    fn far_apart_candidates_are_not_close() {
        let c = |x| Candidate {
            x,
            y: 0,
            angle: 0.0,
        };
        let mut out = [0.0; 2];
        score(&[c(-32_000), c(32_000)], &[], 2_000.0, 4.0, &mut out);
        // sqrt(INT_MAX · 16) ≈ 185 km: far, a small score, never NaN.
        assert!(out.iter().all(|s| s.is_finite() && *s < 0.02), "{out:?}");
        score(&[c(0), c(100)], &[], 2_000.0, 4.0, &mut out);
        assert_eq!(out, [SPACING_TOO_CLOSE; 2], "400 m is closer than 2 km");
        let mut one = [0.0; 1];
        score(
            &[c(0)],
            &[Exclusion { x: 1_000, y: 0 }],
            2_000.0,
            4.0,
            &mut one,
        );
        assert_eq!(one[0], 0.5);
    }

    #[test]
    fn nothing_installs_when_off_or_without_the_target() {
        let empty = ResolvedProfile {
            name: "empty".into(),
            targets: Vec::new(),
            absent_optional: Vec::new(),
        };
        assert!(install_with(&empty, false).contains(PLACEMENT_ENV));
        assert!(install_with(&empty, true).contains("the profile has no"));
    }
}

/// The game's own score, relocated from the executable and run against
/// [`score`].
#[cfg(all(test, windows, target_arch = "x86_64"))]
#[allow(clippy::unwrap_used)]
mod original_tests {
    use std::alloc::{Layout, alloc, dealloc};

    use super::*;
    use crate::bigmap::original::{Exe, Page};

    const SCORE_RVA: u64 = 0x8d31f0;
    const SCORE_LEN: usize = 706;
    const NEW_RVA: u64 = 0x3184230;
    const DELETE_RVA: u64 = 0x318426c;
    const TOO_LONG_RVA: u64 = 0x8bc90;
    const BAD_SIZE_RVA: u64 = 0x8c070;
    const TOO_CLOSE_RVA: u64 = 0x36bbe18;

    extern "C" fn new(size: usize) -> *mut u8 {
        // SAFETY: a non-zero size (the score asks for one per candidate).
        unsafe { alloc(Layout::from_size_align(size, 16).unwrap()) }
    }
    extern "C" fn delete(ptr: *mut u8, size: usize) {
        // SAFETY: what `new` gave for this size.
        unsafe { dealloc(ptr, Layout::from_size_align(size, 16).unwrap()) }
    }
    extern "C" fn sqrtf(x: f32) -> f32 {
        x.sqrt()
    }
    extern "C" fn fail() {
        std::process::abort();
    }

    type Score = unsafe extern "system" fn(
        *const RawVec<f32>,
        *const RawVec<Candidate>,
        f32,
        *const RawVec<Exclusion>,
        f32,
    );

    fn relocated(exe: &Exe, page: &mut Page) -> Score {
        assert_eq!(
            f32::from_le_bytes(exe.bytes(TOO_CLOSE_RVA, 4).try_into().unwrap()),
            SPACING_TOO_CLOSE
        );
        check_body(exe.bytes(SCORE_RVA, SCORE_LEN)).unwrap();
        let too_close = page.data(exe.bytes(TOO_CLOSE_RVA, 4));
        let mut slot = |f: usize| page.data(&(f as u64).to_le_bytes());
        let (new, delete, sqrtf, fail) = (
            slot(new as *const () as usize),
            slot(delete as *const () as usize),
            slot(sqrtf as *const () as usize),
            slot(fail as *const () as usize),
        );
        // Each direct call goes through a near jump to its slot.
        let mut jump = |slot: usize| {
            let at = page.code(&[0xFF, 0x25, 0, 0, 0, 0]);
            let rel = (slot as i64 - (at as i64 + 6)) as i32;
            // SAFETY: the jump's displacement, in the page just written.
            unsafe {
                std::ptr::copy_nonoverlapping(rel.to_le_bytes().as_ptr(), (at + 2) as *mut u8, 4)
            };
            at
        };
        let (new_jump, delete_jump, fail_jump) = (jump(new), jump(delete), jump(fail));
        // sqrtf is called through its import thunk (a direct call), and
        // _invalid_parameter_noinfo_noreturn through the IAT.
        let code = exe.bytes(SCORE_RVA, SCORE_LEN);
        let at = |site: u64| (site - SCORE_RVA) as usize;
        let s = at(0x8d33e4);
        assert_eq!(code[s], 0xE8, "sqrtf's call");
        let rel = i32::from_le_bytes(code[s + 1..s + 5].try_into().unwrap());
        let sqrt_thunk = (0x8d33e4i64 + 5 + i64::from(rel)) as u64;
        let v = at(0x8d349f);
        assert_eq!(code[v..v + 2], [0xFF, 0x15], "the IAT call");
        let disp = i32::from_le_bytes(code[v + 2..v + 6].try_into().unwrap());
        let invalid_iat = (0x8d349fi64 + 6 + i64::from(disp)) as u64;
        let sqrt_jump = jump(sqrtf);
        let (at, _) = page.relocate(exe, SCORE_RVA, SCORE_LEN, &|target| match target {
            NEW_RVA => Some(new_jump),
            DELETE_RVA => Some(delete_jump),
            TOO_LONG_RVA | BAD_SIZE_RVA => Some(fail_jump),
            TOO_CLOSE_RVA => Some(too_close),
            t if t == sqrt_thunk => Some(sqrt_jump),
            t if t == invalid_iat => Some(fail),
            _ => None,
        });
        // SAFETY: the relocated score, the game's own ABI.
        unsafe { std::mem::transmute::<usize, Score>(at) }
    }

    fn run(
        score: Score,
        candidates: &mut [Candidate],
        exclusions: &mut [Exclusion],
        minimum: f32,
        resolution: f32,
    ) -> Vec<f32> {
        let mut out = vec![f32::from_bits(0x7fc0_dead); candidates.len()];
        let vec = |p: *mut f32, n: usize| RawVec {
            begin: p,
            end: p.wrapping_add(n),
            capacity: p.wrapping_add(n),
        };
        let out_vec = vec(out.as_mut_ptr(), out.len());
        let c = candidates.as_mut_ptr();
        let c_vec = RawVec {
            begin: c,
            end: c.wrapping_add(candidates.len()),
            capacity: c.wrapping_add(candidates.len()),
        };
        let e = exclusions.as_mut_ptr();
        let e_vec = RawVec {
            begin: e,
            end: e.wrapping_add(exclusions.len()),
            capacity: e.wrapping_add(exclusions.len()),
        };
        // SAFETY: the vectors above, alive for the call.
        unsafe { score(&out_vec, &c_vec, minimum, &e_vec, resolution) };
        out
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn within(&mut self, span: i32) -> i32 {
            (self.next() % (2 * span as u64 + 1)) as i32 - span
        }
    }

    fn bits(scores: &[f32]) -> Vec<u32> {
        scores.iter().map(|s| s.to_bits()).collect()
    }

    #[test]
    fn where_nothing_overflows_every_score_is_the_games_to_the_bit() {
        let Some(exe) = Exe::load() else { return };
        let mut page = Page::new();
        let game = relocated(&exe, &mut page);
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut compared = 0;
        for case in 0..3_000 {
            let n = (rng.next() % 40) as usize + usize::from(case % 7 != 0);
            let m = (rng.next() % 6) as usize;
            // Within ±16,000 px every square and sum stays below 2^31.
            let span = [100, 2_000, 8_000, 16_000][case % 4];
            let mut candidates: Vec<Candidate> = (0..n)
                .map(|_| Candidate {
                    x: rng.within(span),
                    y: rng.within(span),
                    angle: 0.25,
                })
                .collect();
            if n > 3 && case % 5 == 0 {
                candidates[2] = candidates[1]; // a duplicate: distance 0
            }
            let mut exclusions: Vec<Exclusion> = (0..m)
                .map(|_| Exclusion {
                    x: rng.within(span),
                    y: rng.within(span),
                })
                .collect();
            let minimum = [0.0, 50.0, 1_000.0, 8_000.0][(rng.next() % 4) as usize];
            let resolution = [4.0, 1.0, 0.5][(rng.next() % 3) as usize];
            let before = (candidates.clone(), exclusions.clone());
            let theirs = run(game, &mut candidates, &mut exclusions, minimum, resolution);
            assert_eq!(
                (candidates.clone(), exclusions.clone()),
                before,
                "inputs untouched"
            );
            let mut ours = vec![0.0; n];
            score(&candidates, &exclusions, minimum, resolution, &mut ours);
            assert_eq!(bits(&ours), bits(&theirs), "case {case}");
            compared += n;
        }
        assert!(compared > 30_000);
    }

    #[test]
    fn past_185_km_the_game_wraps_and_the_replacement_does_not() {
        let Some(exe) = Exe::load() else { return };
        let mut page = Page::new();
        let game = relocated(&exe, &mut page);
        // Two candidates 64,000 px (256 km) apart: the game's square wraps
        // negative and the score is NaN.
        let mut far = vec![
            Candidate {
                x: -32_000,
                y: 0,
                angle: 0.0,
            },
            Candidate {
                x: 32_000,
                y: 0,
                angle: 0.0,
            },
        ];
        let theirs = run(game, &mut far, &mut [], 2_000.0, 4.0);
        assert!(theirs.iter().all(|s| s.is_nan()), "{theirs:?}");
        let mut ours = vec![0.0; 2];
        score(&far, &[], 2_000.0, 4.0, &mut ours);
        assert!(ours.iter().all(|s| s.is_finite()), "{ours:?}");
        // The replacement through its detour, on the relocated score.
        let resolved = ResolvedProfile {
            name: "relocated".into(),
            targets: vec![tpf3mp_hookcore::profile::ResolvedTarget {
                name: SPACING.into(),
                address: game as usize as u64,
                image_index: 0,
                required: false,
            }],
            absent_optional: Vec::new(),
        };
        let detour = install_detour(&resolved, true).unwrap();
        let through = run(game, &mut far, &mut [], 2_000.0, 4.0);
        assert_eq!(bits(&through), bits(&ours));
        // Random far-apart points: the detour equals a 128-bit reference.
        let mut rng = Rng(7);
        for _ in 0..200 {
            let mut c: Vec<Candidate> = (0..12)
                .map(|_| Candidate {
                    x: rng.within(1 << 20),
                    y: rng.within(1 << 20),
                    angle: 0.0,
                })
                .collect();
            let got = run(game, &mut c, &mut [], 1_000.0, 4.0);
            for (i, a) in c.iter().enumerate() {
                let best = c
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, b)| {
                        let (dx, dy) = (i128::from(a.x - b.x), i128::from(a.y - b.y));
                        (dx * dx + dy * dy).min(i128::from(i32::MAX)) as i32
                    })
                    .min()
                    .unwrap();
                let distance = (best as f32 * 16.0).sqrt();
                let want = if 1_000.0 > distance {
                    SPACING_TOO_CLOSE
                } else {
                    1_000.0 / distance
                };
                assert_eq!(got[i].to_bits(), want.to_bits());
            }
        }
        // SAFETY: nothing runs the relocated score now.
        unsafe { detour.detach() }.unwrap();
        // Detached, the game's own again.
        assert!(
            run(game, &mut far, &mut [], 2_000.0, 4.0)
                .iter()
                .all(|s| s.is_nan())
        );
    }
}

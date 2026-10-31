//! The faster bitmap span (`fast-bitmap-span`; on unless [`TOGGLE_ENV`] is
//! `0` or `off`; docs/HOOKS.md, "The faster bitmap span").
//!
//! When an industry spawns, the game connects it to the road network
//! (`game/urbansim/init_streets_util.cpp`; "Connect industry: ~900 ms" in its
//! console on Bubu's 150 MB world, build 40425). A sampling profile of those
//! calls found 61% of their time in one 78-byte function, the span fill of
//! an anonymous `BitmapRasterizable` (vtable slot 1, RVA `0x91a560` in
//! 40425), which its rasterizer calls for every span of every shape:
//!
//! ```text
//! bool Fill(this, int x, int y, int count):      // rcx, edx, r8d, r9d
//!     start = y * this->width + x                // int, width at +0x10
//!     end   = start + count                      // int, wrapping
//!     for (i = start; i < end; ++i)              // signed compare once, then count times
//!         bits = this->bitmap->data              // std::vector<uint32_t>* at +8
//!         bits[i >> 5] |= 1u << (i & 31)         // load, bts, store: one bit at a time
//!     return false
//! ```
//!
//! [`fill`] sets the same bits a whole 32-bit word at a time: the bits of the
//! span become 1 and every other bit keeps its value, so the bitmap holds
//! exactly what the game's loop leaves (investigation/
//! PERFORMANCE_AUDIT_2026-10-29.md). It takes only the plain case, a start
//! at or past bit 0 and a count that moves forward without wrapping; any
//! other call goes to the game's own code through the trampoline. The answer
//! is false either way. Nothing is cached, so a call from any thread reads
//! what the original read. A game with it and one without build the same
//! roads, so it is not a room setting.
//!
//! Fail closed: the profile's signature is the whole function, entry to
//! `ret`; a build whose code differs anywhere leaves the game's. The tests
//! run the game's own bytes ([`ORIGINAL_BYTES`]) and [`fill`] side by side.

#![allow(unsafe_code)]
#![cfg_attr(not(all(windows, target_arch = "x86_64")), allow(dead_code))]

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use tpf3mp_hookcore::profile::ResolvedProfile;

pub const FIX: &str = "fast-bitmap-span";
/// Set to `0` (or `off`), the game's own span fill runs.
pub const TOGGLE_ENV: &str = "TPF3MP_HOOK_FAST_BITMAP_SPAN";
/// The profile target: optional, in the builds whose function was checked.
pub const TARGET: &str = "fast-bitmap-span: BitmapRasterizable::Fill";

/// The game's function on Steam Windows 40425, entry to `ret`: the
/// profile's signature, and the code the tests run against [`fill`].
pub const ORIGINAL_BYTES: [u8; 78] = [
    0x44, 0x0F, 0xAF, 0x41, 0x10, // imul r8d, [rcx+0x10]
    0x4C, 0x8B, 0xD9, // mov r11, rcx
    0x44, 0x03, 0xC2, // add r8d, edx
    0x43, 0x8D, 0x04, 0x08, // lea eax, [r8+r9]
    0x44, 0x3B, 0xC0, // cmp r8d, eax
    0x7D, 0x37, // jge ret
    0x49, 0x63, 0xC0, // movsxd rax, r8d
    0x45, 0x8B, 0xD1, // mov r10d, r9d
    0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00, // nop
    0x49, 0x8B, 0x4B, 0x08, // loop: mov rcx, [r11+8]
    0x4C, 0x8B, 0xC0, // mov r8, rax
    0x49, 0xC1, 0xE8, 0x05, // shr r8, 5
    0x48, 0x8B, 0x11, // mov rdx, [rcx]
    0x42, 0x8B, 0x0C, 0x82, // mov ecx, [rdx+r8*4]
    0x4E, 0x8D, 0x0C, 0x82, // lea r9, [rdx+r8*4]
    0x48, 0x8B, 0xD0, // mov rdx, rax
    0x83, 0xE2, 0x1F, // and edx, 0x1f
    0x48, 0xFF, 0xC0, // inc rax
    0x0F, 0xAB, 0xD1, // bts ecx, edx
    0x41, 0x89, 0x09, // mov [r9], ecx
    0x49, 0x83, 0xEA, 0x01, // sub r10, 1
    0x75, 0xD5, // jne loop
    0x32, 0xC0, // xor al, al
    0xC3, // ret
];

/// The original's trampoline, for the calls [`fill`] does not take.
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// Spans filled by words, and calls handed to the original, for hook.log.
static FAST: AtomicU64 = AtomicU64::new(0);
static HANDED: AtomicU64 = AtomicU64::new(0);

/// Sets bits `[start, start + count)` of the words at `bits`, the bit order
/// of `bts` on a little-endian dword: bit `i` is bit `i & 31` of word
/// `i >> 5`.
///
/// # Safety
///
/// The words holding those bits are writable.
pub unsafe fn set_bits(bits: *mut u32, start: u64, count: u64) {
    let end = start + count;
    let mut i = start;
    // The first word, from `start` to its end or the span's.
    if i & 31 != 0 {
        let to = end.min((i | 31) + 1);
        let mask = word_mask(i & 31, to - i);
        // SAFETY: a word of the span (the caller's).
        unsafe { *bits.add((i >> 5) as usize) |= mask };
        i = to;
    }
    // Whole words: every bit of them is the span's.
    while end - i >= 32 {
        // SAFETY: as above.
        unsafe { *bits.add((i >> 5) as usize) = u32::MAX };
        i += 32;
    }
    // The last word's first bits.
    if i < end {
        // SAFETY: as above.
        unsafe { *bits.add((i >> 5) as usize) |= word_mask(0, end - i) };
    }
}

/// `len` bits (1..=32) from bit `from`, within one word.
fn word_mask(from: u64, len: u64) -> u32 {
    let ones = if len >= 32 {
        u32::MAX
    } else {
        (1u32 << len) - 1
    };
    ones << from
}

/// The span fill's entry while the fix is in.
///
/// # Safety
///
/// Called by the game in place of the original, with its arguments: `this`
/// a `BitmapRasterizable`, whose bitmap holds the span's bits.
pub unsafe extern "system" fn fill(this: usize, x: i32, y: i32, count: i32) -> u8 {
    // SAFETY: the object the game called with: its width at +0x10.
    let width = unsafe { std::ptr::read_unaligned((this + 0x10) as *const i32) };
    let start = y.wrapping_mul(width).wrapping_add(x);
    let end = start.wrapping_add(count);
    if start >= end {
        // The original's own early answer: no bit set.
        return 0;
    }
    if start < 0 {
        // A span below bit 0, or one whose end wrapped round: the game's
        // own loop decides what it does.
        HANDED.fetch_add(1, Ordering::Relaxed);
        let original = ORIGINAL.load(Ordering::Acquire);
        // SAFETY: the trampoline of the function this stands in for, which
        // takes these four arguments; installed before `fill` can run.
        let original: unsafe extern "system" fn(usize, i32, i32, i32) -> u8 =
            unsafe { std::mem::transmute::<usize, _>(original) };
        return unsafe { original(this, x, y, count) };
    }
    // Here 0 <= start < end, so the original's count of r9d iterations is
    // `count`, and its bits are [start, end).
    // SAFETY: the object's bitmap pointer at +8, the vector's data pointer
    // first in it, as the original reads them; the span's words are the
    // ones the original writes.
    unsafe {
        let vector = std::ptr::read_unaligned((this + 8) as *const usize);
        let bits = std::ptr::read_unaligned(vector as *const usize) as *mut u32;
        set_bits(bits, start as u64, count as u64);
    }
    FAST.fetch_add(1, Ordering::Relaxed);
    0
}

/// Spans filled by words and calls handed to the game's loop since the last
/// take, for hook.log's `perf:` window.
pub fn take_counts() -> (u64, u64) {
    (
        FAST.swap(0, Ordering::Relaxed),
        HANDED.swap(0, Ordering::Relaxed),
    )
}

/// What installing came to, for hook.log.
pub fn outcome_line(installed: bool, reason: &str) -> String {
    if installed {
        format!("{FIX}: installed ({reason})")
    } else {
        format!("{FIX}: off, {reason}")
    }
}

/// Installs the fix unless [`TOGGLE_ENV`] says no. Returns the line for
/// hook.log.
pub fn install(resolved: &ResolvedProfile) -> String {
    if !crate::ticks::wanted(std::env::var(TOGGLE_ENV).ok().as_deref()) {
        return outcome_line(
            false,
            &format!("{TOGGLE_ENV} says so; the game's own span fill"),
        );
    }
    let Some(target) = resolved.get(TARGET) else {
        return outcome_line(
            false,
            &format!("the profile has no {TARGET:?}; the game's own span fill"),
        );
    };
    install_at(target.address as usize)
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn install_at(target: usize) -> String {
    use tpf3mp_hookcore::detour::InlineDetour;
    // SAFETY: the profile resolved `target` to the span fill, its signature
    // the whole function [`fill`] repeats; no game thread runs yet (the hook
    // installs before the game's entry point); `fill` has its ABI and hands
    // every call it does not take to the trampoline.
    match unsafe { InlineDetour::install(target as *mut u8, fill as *const u8) } {
        Ok(detour) => {
            ORIGINAL.store(detour.trampoline() as usize, Ordering::Release);
            std::mem::forget(detour);
            outcome_line(
                true,
                &format!(
                    "BitmapRasterizable's span fill at {target:#x}: whole words instead of one bit at a time, the same bits; {TOGGLE_ENV}=0 turns it off"
                ),
            )
        }
        Err(error) => outcome_line(
            false,
            &format!("{TARGET} at {target:#x}: {error}; the game's own span fill"),
        ),
    }
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
fn install_at(_target: usize) -> String {
    outcome_line(
        false,
        "built for Windows x64 only; the game's own span fill",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_says_what_the_fix_did() {
        assert_eq!(outcome_line(false, "why"), "fast-bitmap-span: off, why");
        assert_eq!(
            outcome_line(true, "how"),
            "fast-bitmap-span: installed (how)"
        );
    }

    /// Bit by bit, as the game's loop: the reference for [`set_bits`].
    fn by_bits(words: &mut [u32], start: u64, count: u64) {
        for i in start..start + count {
            words[(i >> 5) as usize] |= 1 << (i & 31);
        }
    }

    #[test]
    fn whole_words_set_the_same_bits_as_one_at_a_time() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..20_000 {
            let start = next() % 600;
            let count = next() % 300 + 1;
            let mut a: Vec<u32> = (0..32).map(|_| next() as u32).collect();
            let mut b = a.clone();
            by_bits(&mut a, start, count);
            // SAFETY: 32 words hold bits below 1024 > 600 + 300.
            unsafe { set_bits(b.as_mut_ptr(), start, count) };
            assert_eq!(a, b, "span {start}+{count}");
        }
    }

    #[test]
    fn the_profile_names_the_whole_function() {
        let profile = tpf3mp_hookcore::profile::Profile::from_toml(include_str!(
            "../../../profiles/tf3_build40425_steam_windows/hooks.toml"
        ))
        .unwrap();
        let target = profile
            .targets
            .iter()
            .find(|target| target.name == TARGET)
            .expect("the 40425 profile names the span fill");
        assert!(!target.required, "optional: the game's own without it");
        assert_eq!(target.offset, 0);
        assert!(
            target.pattern().matches_at(&ORIGINAL_BYTES, 0),
            "the signature is the whole function"
        );
        assert_eq!(target.prologue[..], ORIGINAL_BYTES[..target.prologue.len()]);
    }
}

/// Against the game's own code: [`ORIGINAL_BYTES`] run in an executable page
/// of the test process (they have no call or data reference outside
/// themselves), and [`fill`] on the same objects.
#[cfg(all(test, windows, target_arch = "x86_64"))]
mod original_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::original::Page;

    /// The game's object as the function reads it: the bitmap's vector at
    /// +8, the width at +0x10.
    #[repr(C)]
    struct Rasterizable {
        vtable: usize,
        bitmap: *const Vec3,
        width: i32,
    }

    /// A `std::vector<uint32_t>`'s three pointers.
    #[repr(C)]
    struct Vec3 {
        begin: *mut u32,
        end: *mut u32,
        cap: *mut u32,
    }

    type Fill = unsafe extern "system" fn(usize, i32, i32, i32) -> u8;

    fn run(f: Fill, words: &mut [u32], width: i32, x: i32, y: i32, count: i32) -> u8 {
        let range = words.as_mut_ptr_range();
        let vector = Vec3 {
            begin: range.start,
            end: range.end,
            cap: range.end,
        };
        let object = Rasterizable {
            vtable: 0,
            bitmap: &vector,
            width,
        };
        // SAFETY: the object and bitmap above; the spans stay inside it.
        unsafe { f(&object as *const Rasterizable as usize, x, y, count) }
    }

    #[test]
    fn every_span_leaves_the_bitmap_as_the_games_loop_does() {
        let mut page = Page::new();
        // SAFETY: the game's function, position independent, in an
        // executable page that outlives every call below.
        let original: Fill =
            unsafe { std::mem::transmute::<usize, Fill>(page.code(&ORIGINAL_BYTES)) };
        // Every span here starts at or past bit 0: `fill` hands none on.
        let mut seed = 0xD1B5_4A32_D192_ED03u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        const WORDS: usize = 4096; // 131,072 bits
        for case in 0..20_000 {
            let width = (next() % 700) as i32 + 1;
            let rows = (WORDS as i32 * 32) / width;
            let y = (next() % rows as u64) as i32;
            let x = (next() % width as u64) as i32;
            let room = WORDS as i64 * 32 - (i64::from(y) * i64::from(width) + i64::from(x));
            // Mostly forward spans that fit, some empty and negative ones.
            let count = match next() % 10 {
                0 => 0,
                1 => -((next() % 50) as i32),
                _ => (next() % room.min(5000) as u64) as i32,
            };
            let mut a: Vec<u32> = (0..WORDS).map(|_| next() as u32).collect();
            let mut b = a.clone();
            let ra = run(original, &mut a, width, x, y, count);
            let rb = run(fill, &mut b, width, x, y, count);
            assert_eq!(ra, 0);
            assert_eq!(rb, ra, "case {case}: the answer");
            assert!(
                a == b,
                "case {case}: width {width} at ({x}, {y}) count {count}"
            );
        }
    }

    static STAND_IN_CALLS: AtomicU64 = AtomicU64::new(0);

    /// Stands in for the trampoline: the game's loop would write below the
    /// bitmap for such a span, so the test only checks it is handed on.
    unsafe extern "system" fn stand_in(_this: usize, _x: i32, _y: i32, _count: i32) -> u8 {
        STAND_IN_CALLS.fetch_add(1, Ordering::Relaxed);
        0
    }

    #[test]
    fn a_span_below_bit_zero_goes_to_the_games_loop() {
        ORIGINAL.store(stand_in as *const () as usize, Ordering::Release);
        let mut a = vec![0u32; 4];
        // A start of -3 with a count of 5: the original's own business.
        assert_eq!(run(fill, &mut a, 10, -3, 0, 5), 0);
        assert_eq!(STAND_IN_CALLS.load(Ordering::Relaxed), 1);
        assert_eq!(a, vec![0; 4], "nothing written here");
        // Empty spans answer at once, below bit 0 or not.
        assert_eq!(run(fill, &mut a, 10, -3, 0, 0), 0);
        assert_eq!(run(fill, &mut a, 10, 4, 0, -2), 0);
        assert_eq!(STAND_IN_CALLS.load(Ordering::Relaxed), 1);
        // A forward span from bit 5 is filled by words.
        let mut page = Page::new();
        // SAFETY: as above.
        let original: Fill =
            unsafe { std::mem::transmute::<usize, Fill>(page.code(&ORIGINAL_BYTES)) };
        assert_eq!(run(fill, &mut a, 10, 5, 0, 40), 0);
        let mut b = vec![0u32; 4];
        run(original, &mut b, 10, 5, 0, 40);
        assert_eq!(a, b);
    }
}

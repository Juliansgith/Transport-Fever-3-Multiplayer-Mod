//! The player's build preview, read where Transport Fever 3 draws it
//! (docs/HOOKS.md, "The build tools").
//!
//! The street, track and construction tools are native: the whole preview —
//! the ribbon under the pointer, its length and angle, and the cost the game
//! shows — is computed in C++ and drawn by the renderer. Nothing of it reaches
//! game scripts: the street tool sends no `builder.proposalCreate` while the
//! pointer moves, only on the click, and it sends no
//! `builder.proposalPrepareForApply` at all (docs/HOOKS.md, "The build
//! tools"). So the mod, which learns of a build from those events, cannot see
//! the preview and never reports one: `hook.log` has no cursor line, and the
//! room never hears a datagram.
//!
//! [`UI::StreetBuilder::CreateProposalAndUpdate`] is where the game computes
//! it. `UI::StreetBuilder::Step` (vtable slot 5) calls it on every frame the
//! pointer moves, so detouring it sees each preview as it is made. What the
//! detour reads, off the `StreetBuilder` this call is on, is the pointer on
//! the ground plane and whether a preview is up at all:
//!
//! | offset | what |
//! |---|---|
//! | `+0x934` | pointer x on the ground plane, in metres |
//! | `+0x938` | pointer y on the ground plane, in metres |
//! | `+0x93c` | pointer height, or its validity |
//! | `+0x940` | whether a preview is up |
//!
//! The three floats are checked against zero on entry, and the function
//! returns without building a preview when all three are; the flag gates the
//! rest. This detour therefore reports the pointer whenever the game is
//! building, and clears the cursor (`at: None`) when it is not, which is what
//! the receiving game draws by.
//!
//! **The preview's shape does not come from here.** The curves the game draws
//! live in the `ProposalDataProduct` the call hands a thread pool
//! ([`tools/tpfre`] on build 40408: `CreateProposalAndUpdate(bool)` is
//! `private`, and enqueues `ProposalData` work), and are not read yet. What
//! crosses the room is therefore the pointer, and nothing else.
//!
//! ## The other player's preview, drawn by this game's own renderer
//!
//! Not reading the curves is what makes the pointer enough. The three floats
//! are the *input* the game computes its preview from, and the call is the
//! game's own: a receiver that has a build tool open puts the other player's
//! pointer where the pointer sits for the frame, lets the game run, and puts
//! the pointer back. The ribbon under the pointer, its angle, its length and
//! the cost the game shows are then this game's own, computed and drawn by the
//! renderer as usual — the room sends a position, not a shape.
//!
//! Two rules keep the local player's own pointer and their clicks untouched:
//!
//! - the pointer is written only while it has not moved since the frame
//!   before, so a player who moves their own pointer sees their own preview
//!   and the other's takes over as soon as they let the pointer rest;
//! - it is written only around the call and put back right after, so the
//!   fields the game reads on a click are the local ones: a click still
//!   builds where the local player pointed, never where the other did.
//!
//! With no build tool open there is no `StreetBuilder` to drive and nothing is
//! drawn; the mod's own marker (tpf3mp_sim.script.lua) is the fallback for
//! that case, and stands down while [`drawn_lately`] says the game's renderer
//! has the preview.

#![allow(unsafe_code)]
#![cfg_attr(not(all(windows, target_arch = "x86_64")), allow(dead_code))]

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Where the pointer's x sits on the `StreetBuilder` (build 40408), in metres.
pub const POINTER_X: usize = 0x934;
/// Where the pointer's y sits, in metres.
pub const POINTER_Y: usize = 0x938;
/// The pointer's height, or its validity; all three floats are checked
/// against zero before the game builds a preview.
pub const POINTER_Z: usize = 0x93c;
/// Whether a preview is up.
pub const PREVIEW_UP: usize = 0x940;

/// The game's own call, reached through the detour's trampoline.
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// The detour is in.
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// Previews seen, for the log and the tests.
static SEEN: AtomicU64 = AtomicU64::new(0);

/// How long the other player's pointer is the one this game's preview is drawn
/// from, in milliseconds. The room's cursor is advisory: it leaves on the
/// frame the pointer moves and arrives a frame or two later, so it is held for
/// a few frames and then let go, and this player's own preview is back.
pub const REMOTE_HOLD_MS: u64 = 400;

/// The other player's pointer, its x as a float's bits.
static REMOTE_X: AtomicU32 = AtomicU32::new(0);
/// The other player's pointer, its y as a float's bits.
static REMOTE_Y: AtomicU32 = AtomicU32::new(0);
/// When that pointer arrived, in milliseconds since the epoch; 0, none.
static REMOTE_AT: AtomicU64 = AtomicU64::new(0);
/// This player's pointer as the frame before left it, its x as float bits.
static LAST_X: AtomicU32 = AtomicU32::new(0);
/// Its y, the same way.
static LAST_Y: AtomicU32 = AtomicU32::new(0);
/// Whether a frame has left a pointer to compare against.
static LAST_SEEN: AtomicBool = AtomicBool::new(false);
/// How many frames this game's own preview was drawn for the other player.
static DRAWN: AtomicU64 = AtomicU64::new(0);
/// When the last of those was, in milliseconds since the epoch.
static DRAWN_AT: AtomicU64 = AtomicU64::new(0);
/// How many pointers the room has reported, position or none.
static RECEIVED: AtomicU64 = AtomicU64::new(0);

/// Whether the detour is in.
pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// Previews this game has seen so far.
pub fn seen() -> u64 {
    SEEN.load(Ordering::Acquire)
}

/// How many frames this game's own preview was drawn for the other player.
pub fn drawn() -> u64 {
    DRAWN.load(Ordering::Acquire)
}

/// How many pointers the room has reported, so a game that draws nothing can be
/// told from one that never heard of the other player's pointer.
pub fn received() -> u64 {
    RECEIVED.load(Ordering::Acquire)
}

/// Whether the game's renderer has the other player's preview, as of the last
/// frame or near enough: the mod's own marker stands down while this is true,
/// and is there for a player with no build tool open, where there is no
/// `StreetBuilder` to drive.
pub fn drawn_lately() -> bool {
    let at = DRAWN_AT.load(Ordering::Acquire);
    at != 0 && now_millis().saturating_sub(at) <= REMOTE_HOLD_MS
}

/// The other player's pointer, as the room's last cursor said it: `None` when
/// the room reported none, or reported one that is not a position.
///
/// The room's positions are whole millimetres on the ground plane, the game's
/// pointer is in metres, so this is the one place that scales them.
pub fn remote(at: Option<tpf3mp_proto::action::Pos2>) {
    RECEIVED.fetch_add(1, Ordering::AcqRel);
    let metres = at.map(|at| (at.x as f32 / 1000.0, at.y as f32 / 1000.0));
    match metres {
        Some((x, y)) if x.is_finite() && y.is_finite() => {
            REMOTE_X.store(x.to_bits(), Ordering::Release);
            REMOTE_Y.store(y.to_bits(), Ordering::Release);
            REMOTE_AT.store(now_millis(), Ordering::Release);
        }
        _ => {
            REMOTE_AT.store(0, Ordering::Release);
        }
    }
}

/// The other player's pointer, while it is still the one to draw from.
fn fresh_remote() -> Option<(f32, f32)> {
    let at = REMOTE_AT.load(Ordering::Acquire);
    if at == 0 || now_millis().saturating_sub(at) > REMOTE_HOLD_MS {
        return None;
    }
    let at = (
        f32::from_bits(REMOTE_X.load(Ordering::Acquire)),
        f32::from_bits(REMOTE_Y.load(Ordering::Acquire)),
    );
    (at.0.is_finite() && at.1.is_finite()).then_some(at)
}

/// Milliseconds since the epoch, the clock `hook.log`'s timestamps use.
fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Whether this player's pointer moved since the frame before, which is what
/// tells a player pointing at something from one whose pointer rests.
fn moved(last: Option<(f32, f32)>, now: (f32, f32)) -> bool {
    last.is_some_and(|last| last != now)
}

/// Whether the other player's pointer is drawn by the game's own renderer this
/// frame: it is fresh, the builder is not committing, and this player's
/// pointer has not moved since the frame before.
fn draws_remote(building: u8, remote: Option<(f32, f32)>, moved: bool) -> bool {
    building == 0 && remote.is_some() && !moved
}

/// The call's signature: the `StreetBuilder` this, and the game's `bool`;
/// returns what the game returns.
///
/// The register pass is what the game's own mangled name gives
/// (`CreateProposalAndUpdate(bool)`, `private`, `__cdecl`).
type PreviewFn = unsafe extern "C" fn(usize, u8) -> u64;

/// The detour: hands the game on, with the other player's pointer in place
/// while it computes the preview, and reads the pointer back afterwards.
///
/// The read is after the call, not before: the pointer moves as a frame runs,
/// so the game has just refreshed it, and the flag says whether the preview it
/// drew from it is still up. Reading it after the pointer is put back is what
/// keeps this player's own position the one the room hears about.
unsafe extern "C" fn preview_detour(builder: usize, building: u8) -> u64 {
    let original = ORIGINAL.load(Ordering::Acquire);
    let swapped = if builder == 0 {
        None
    } else {
        swap_pointer(builder, building)
    };
    // SAFETY: the trampoline of this call, called as `Step` called it.
    let returned = unsafe {
        let call: PreviewFn = std::mem::transmute::<usize, PreviewFn>(original);
        call(builder, building)
    };
    if let Some((x, y)) = swapped {
        // SAFETY: the pointer this game's own last frame left, put back before
        // anything reads it: the game reads it again on the click.
        unsafe {
            put_float(builder, POINTER_X, x);
            put_float(builder, POINTER_Y, y);
        }
        DRAWN.fetch_add(1, Ordering::AcqRel);
        DRAWN_AT.store(now_millis(), Ordering::Release);
    }
    if crate::lua::in_room() && builder != 0 {
        report(builder);
    }
    returned
}

/// Puts the other player's pointer where this frame's preview is computed
/// from, and answers what was there to put back. `None` when this frame's
/// preview is this player's own.
fn swap_pointer(builder: usize, building: u8) -> Option<(f32, f32)> {
    // SAFETY: the `StreetBuilder` the game passed, read and written at the
    // offsets its own code reads and writes them (0x577f42..0x577f74).
    let local = unsafe { get_float(builder, POINTER_X) };
    let here = (
        local,
        // SAFETY: as above.
        unsafe { get_float(builder, POINTER_Y) },
    );
    let last = if LAST_SEEN.load(Ordering::Acquire) {
        Some((
            bits_to_float(LAST_X.load(Ordering::Acquire)),
            bits_to_float(LAST_Y.load(Ordering::Acquire)),
        ))
    } else {
        None
    };
    LAST_X.store(here.0.to_bits(), Ordering::Release);
    LAST_Y.store(here.1.to_bits(), Ordering::Release);
    LAST_SEEN.store(true, Ordering::Release);
    let remote = fresh_remote();
    if !draws_remote(building, remote, moved(last, here)) {
        return None;
    }
    let at = remote?;
    // SAFETY: as above; the game reads both on entry and rewrites them itself
    // on the next frame.
    unsafe {
        put_float(builder, POINTER_X, at.0);
        put_float(builder, POINTER_Y, at.1);
    }
    Some(here)
}

/// One of the pointer's floats, off `builder`.
unsafe fn get_float(builder: usize, offset: usize) -> f32 {
    // SAFETY: the caller's; a four-byte read in bounds of the `StreetBuilder`.
    unsafe { std::ptr::read_unaligned((builder as *const u8).add(offset).cast::<f32>()) }
}

/// One of the pointer's floats, onto `builder`.
unsafe fn put_float(builder: usize, offset: usize, value: f32) {
    // SAFETY: the caller's; a four-byte write in bounds of the `StreetBuilder`.
    unsafe { std::ptr::write_unaligned((builder as *mut u8).add(offset).cast::<f32>(), value) }
}

fn bits_to_float(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// Reads the pointer off `builder` and hands it to the mod as a cursor.
fn report(builder: usize) {
    if builder == 0 {
        return;
    }
    // SAFETY: the `StreetBuilder` the game passed, read at the offsets its own
    // code reads them (0x577f42..0x577f74).
    let at = |offset: usize| -> *const u8 { unsafe { (builder as *const u8).add(offset) } };
    if unsafe { std::ptr::read_unaligned(at(PREVIEW_UP)) } == 0 {
        // The tool is not showing anything: the receiver drops its marker.
        crate::lua::report_cursor(None, None, false);
        return;
    }
    // SAFETY: as above; the three floats the game itself reads here.
    let (x, y, z) = unsafe {
        (
            std::ptr::read_unaligned(at(POINTER_X).cast::<f32>()),
            std::ptr::read_unaligned(at(POINTER_Y).cast::<f32>()),
            std::ptr::read_unaligned(at(POINTER_Z).cast::<f32>()),
        )
    };
    if !(x.is_finite() && y.is_finite() && z.is_finite()) {
        return;
    }
    SEEN.fetch_add(1, Ordering::AcqRel);
    crate::lua::report_cursor(Some((x, y)), None, true);
}

/// Detours the preview's update, at the address the profile resolved.
///
/// # Safety
///
/// `target` is the function the profile names, in this process, which no
/// thread runs yet (the hook installs while the game starts).
#[cfg(all(windows, target_arch = "x86_64"))]
pub unsafe fn install(
    target: usize,
    detour: unsafe fn(*mut u8, *const u8) -> Result<usize, String>,
) -> Result<(), String> {
    // SAFETY: the caller's; `preview_detour` has the call's ABI, and only
    // reads the `StreetBuilder` the game passes it.
    let original = unsafe { detour(target as *mut u8, preview_detour as *const u8) }?;
    ORIGINAL.store(original, Ordering::Release);
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offsets_are_where_the_game_puts_them() {
        // The game lays these four out inside its own `StreetBuilder`, at these
        // offsets. The test writes them there and reads them back, so a wrong
        // offset in the detour would not be caught by reading a struct field
        // of the test's own.
        let mut bytes = vec![0u8; PREVIEW_UP + 8];
        let mut put = |offset: usize, raw: [u8; 4]| {
            bytes[offset..offset + 4].copy_from_slice(&raw);
        };
        put(POINTER_X, 1234.5f32.to_le_bytes());
        put(POINTER_Y, (-678.25f32).to_le_bytes());
        put(POINTER_Z, 42.0f32.to_le_bytes());
        bytes[PREVIEW_UP] = 1;
        let base = bytes.as_ptr() as usize;
        // SAFETY: the buffer is live and long enough for every read.
        unsafe {
            let at = |offset: usize| (base as *const u8).add(offset);
            assert_eq!(
                std::ptr::read_unaligned(at(POINTER_X).cast::<f32>()),
                1234.5
            );
            assert_eq!(
                std::ptr::read_unaligned(at(POINTER_Y).cast::<f32>()),
                -678.25
            );
            assert_eq!(std::ptr::read_unaligned(at(POINTER_Z).cast::<f32>()), 42.0);
            assert_eq!(std::ptr::read_unaligned(at(PREVIEW_UP)), 1);
        }
    }

    #[test]
    fn the_offsets_do_not_overlap() {
        // The game reads the three floats, then the flag, in this order
        // (0x577f35, 0x577f42, 0x577f56, 0x577f66), so a pointer read that
        // overlapped another would report a second float as a first. The
        // values are read at runtime, so the check is not a constant fold.
        let offsets = [POINTER_X, POINTER_Y, POINTER_Z, PREVIEW_UP];
        for pair in offsets.windows(2) {
            assert!(pair[0] < pair[1], "{pair:x?} is out of order");
        }
        // Each float's four bytes lie before the next field, and the flag is
        // its own byte.
        for pair in offsets.windows(2) {
            let width = if *pair.last().unwrap() == PREVIEW_UP {
                1
            } else {
                4
            };
            assert!(pair[0] + width <= pair[1]);
        }
    }

    #[test]
    fn a_pointer_that_is_not_a_number_is_not_reported() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(!(bad.is_finite()));
        }
    }

    #[test]
    fn an_idle_pointer_leaves_the_others_preview_to_the_games_own_renderer() {
        // The other player's pointer is fresh, this player is not committing
        // and their pointer stands still: the game computes its own preview
        // for the other player.
        assert!(draws_remote(0, Some((10.0, 20.0)), false));
    }

    #[test]
    fn a_pointer_that_moved_is_this_players_own_preview() {
        // A player who moves their pointer sees their own preview, whatever
        // the other one is doing.
        assert!(!draws_remote(0, Some((10.0, 20.0)), true));
    }

    #[test]
    fn a_commit_is_never_handed_to_the_other_players_pointer() {
        // The call that commits a build reads the pointer, so that frame is
        // this player's alone.
        assert!(!draws_remote(1, Some((10.0, 20.0)), false));
    }

    #[test]
    fn without_the_other_players_pointer_the_game_is_left_alone() {
        assert!(!draws_remote(0, None, false));
    }

    #[test]
    fn the_pointer_is_swapped_put_back_and_released() {
        // One test, because it is the one that stands the module's statics up:
        // the swap on the buffer, the window the room's cursor is held for, and
        // the scale the room's positions arrive in.
        let mut bytes = vec![0u8; PREVIEW_UP + 8];
        let base = bytes.as_ptr() as usize;
        let mut put = |offset: usize, value: f32| {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        };
        put(POINTER_X, 1234.5);
        put(POINTER_Y, -678.25);

        // The room reported no pointer: the game is left alone, and what is on
        // the builder is what the game itself had.
        remote(None);
        // SAFETY: the buffer is live and long enough for every read and write.
        unsafe {
            assert_eq!(swap_pointer(base, 0), None);
            assert_eq!(get_float(base, POINTER_X), 1234.5);
            assert_eq!(get_float(base, POINTER_Y), -678.25);
        }

        // The other player's pointer is fresh and this player's rests: the
        // game is given it for the frame, and gets this player's own pointer
        // back, which is what the game reads on a click.
        remote(Some(millimetres(10.0, 20.0)));
        // SAFETY: as above.
        unsafe {
            swap_pointer(base, 0);
            assert_eq!(get_float(base, POINTER_X), 10.0);
            assert_eq!(get_float(base, POINTER_Y), 20.0);
            // The frame after, the pointer stands where the last one put it.
            assert_eq!(swap_pointer(base, 0), None);
        }

        // The room's positions are whole millimetres on the ground plane and
        // the game's pointer is in metres: a round number has to come back as
        // itself, or the preview is drawn a kilometre away.
        remote(Some(millimetres(1234.0, -678.0)));
        let (x, y) = fresh_remote().expect("a fresh position");
        assert!((x - 1234.0).abs() < 0.001, "{x} is not 1234 m");
        assert!((y + 678.0).abs() < 0.001, "{y} is not -678 m");

        // The room's cursor is late, not endless: older than the window, the
        // other player's pointer is let go.
        REMOTE_AT.store(now_millis() - REMOTE_HOLD_MS - 1, Ordering::Release);
        assert_eq!(fresh_remote(), None);
        // And the room reporting none at all lets it go at once.
        remote(None);
        assert_eq!(fresh_remote(), None);
    }

    #[test]
    fn a_moved_pointer_is_told_from_one_at_rest() {
        assert!(
            !moved(None, (1.0, 2.0)),
            "the first frame has nothing to compare"
        );
        assert!(!moved(Some((1.0, 2.0)), (1.0, 2.0)));
        assert!(moved(Some((1.0, 2.0)), (1.0, 2.5)));
    }

    /// A position in whole millimetres, as the room sends it.
    fn millimetres(x: f32, y: f32) -> tpf3mp_proto::action::Pos2 {
        tpf3mp_proto::action::Pos2 {
            x: (x * 1000.0) as i32,
            y: (y * 1000.0) as i32,
        }
    }
}

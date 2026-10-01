//! Which command kinds reach the game's command queue in the room's game,
//! and from where (docs/COVERAGE.md, "The gates and their holes", U2).
//!
//! The room stops a player's build at its apply ([`crate::builds`]) and the
//! GUI's Lua commands at `sendCommand` (the mod's guard). A command of any
//! other kind that native code queues without Lua would pass both, and act
//! in this game alone. None is known on build 40408, but none is ruled out.
//! So the add's detour notes every command kind queued in the room's game
//! outside the room's own replays, with where its `CommandList::Add` call
//! returns to, and logs each pair the first time it is seen:
//!
//! ```text
//! command kind 52 queued from +0x543b2a in the room's game: a player's build, stopped at its apply
//! command kind 17 queued from +0x12ab34 in the room's game: not stopped by the build gate; the GUI's guard sees it only if Lua sent it
//! ```
//!
//! A playtest's log then names the call site of Lua's `sendCommand` (the one
//! site every guarded kind comes from) and any other, which the room can
//! then refuse by its site. Measurement only: nothing is stopped here.

#![allow(unsafe_code)]

use std::sync::Mutex;

/// Distinct (kind, site) pairs kept; later ones are counted, not logged.
pub const MAX_SEEN: usize = 128;

/// The pairs seen so far.
#[derive(Debug, Default)]
pub struct Seen {
    pairs: Vec<(i8, usize)>,
    dropped: u64,
}

impl Seen {
    /// Whether `(kind, site)` is new: true the first time, false after, and
    /// false once [`MAX_SEEN`] pairs are kept (counted in `dropped`).
    pub fn first(&mut self, kind: i8, site: usize) -> bool {
        if self.pairs.contains(&(kind, site)) {
            return false;
        }
        if self.pairs.len() >= MAX_SEEN {
            self.dropped += 1;
            return false;
        }
        self.pairs.push((kind, site));
        true
    }

    /// Pairs not kept for want of room.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// The log line for a pair seen the first time. `site` is where `Add`
/// returns to, as an offset into the game's image when `base` is known.
pub fn describe(kind: i8, player_build: bool, site: usize, base: Option<usize>) -> String {
    let at = match base {
        Some(base) if site >= base => format!("+{:#x}", site - base),
        _ => format!("{site:#x}"),
    };
    let what = if player_build {
        "a player's build, stopped at its apply"
    } else {
        "not stopped by the build gate; the GUI's guard sees it only if Lua sent it"
    };
    format!("command kind {kind} queued from {at} in the room's game: {what}")
}

static SEEN: Mutex<Seen> = Mutex::new(Seen {
    pairs: Vec::new(),
    dropped: 0,
});

/// The game's image base, where it can be read.
fn image_base() -> Option<usize> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
        // SAFETY: a null name asks for the process's own image, which stays
        // loaded while the process runs.
        let base = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
        if base != 0 {
            return Some(base);
        }
    }
    None
}

/// Notes a command of `kind` queued in the room's game outside the room's
/// replays, whose `Add` returns to `site` (0 when unknown); logs the pair
/// the first time.
pub fn note(kind: i8, player_build: bool, site: usize) {
    let new = match SEEN.lock() {
        Ok(mut seen) => seen.first(kind, site),
        Err(_) => false,
    };
    if new {
        crate::log::line(&describe(kind, player_build, site, image_base()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_and_site_is_new_once_and_the_list_is_bounded() {
        let mut seen = Seen::default();
        assert!(seen.first(52, 0x10));
        assert!(!seen.first(52, 0x10), "seen already");
        assert!(seen.first(52, 0x20), "the same kind from elsewhere");
        assert!(seen.first(17, 0x10), "another kind from the same site");
        for k in 0..MAX_SEEN {
            seen.first(1, 0x1000 + k);
        }
        assert_eq!(seen.pairs.len(), MAX_SEEN);
        assert!(seen.dropped() > 0);
        assert!(!seen.first(99, 0x99), "full: counted, not kept");
    }

    #[test]
    fn a_pair_is_said_by_its_offset_and_whether_the_gate_stops_it() {
        assert_eq!(
            describe(52, true, 0x140543b2a, Some(0x140000000)),
            "command kind 52 queued from +0x543b2a in the room's game: a player's build, \
             stopped at its apply"
        );
        assert_eq!(
            describe(17, false, 0x1234, None),
            "command kind 17 queued from 0x1234 in the room's game: not stopped by the build \
             gate; the GUI's guard sees it only if Lua sent it"
        );
    }
}

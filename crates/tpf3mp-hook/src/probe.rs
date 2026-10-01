//! A read-only probe of where the engine keeps its player
//! (investigation/TF3_LOCAL_PLAYER_2026-10-01.md), off unless
//! `TPF3MP_PROBE_PLAYER=1`.
//!
//! `api.engine.util.getPlayer` answers from a `GameState` each Lua state's
//! setup hands it: the game scripts' from one of two buffers under
//! `CGame+0x1f0` (the getter [`SIM_TARGET`]), the GUI's from `CGame+0x1e0`
//! (the getter [`GUI_TARGET`], through `CMenuUI::m_game`). Whether the GUI's
//! is a state of its own, or one of the simulation's two buffers, decides
//! whether the GUI and the game's tools can act as the player's company
//! while the simulation keeps the save's player. This probe says so in
//! `hook.log`, from a real game:
//!
//! - each pointer, and whether the GUI's equals either buffer;
//! - where in each state the save's player entity (which the mod's game
//!   script notes, `tpf3mp.player`) stands, as a dword or a qword, within
//!   the first [`SCAN_BYTES`].
//!
//! It writes nothing: the two targets are only read for their field
//! offsets, every pointer is checked readable before it is read, and
//! nothing is ever written to the game. A missing target leaves it off.
//! It runs in `CMenuUI::DoStep`'s detour, on the main thread, after the
//! game's own frame: a few frames after the world changes, then once every
//! [`EVERY_MS`].

#![allow(unsafe_code)]

use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use tpf3mp_hookcore::profile::ResolvedProfile;

/// The environment variable that turns the probe on (`1`).
pub const ENV: &str = "TPF3MP_PROBE_PLAYER";
/// The GUI's `GameState` getter, `CMenuUI::SwitchToGameUI`'s lambda_2
/// (`mov rax,[rcx+8]; mov rax,[rax+m_game]; mov rax,[rax+state]; ret`).
pub const GUI_TARGET: &str = "probe: GUI GameState getter";
/// The game scripts' `GameState` getter, `CGame::CGame`'s lambda_1
/// (`[[CGame+states] + base + 8*i]`, `i` the word at `+index`, or `1 - i`).
pub const SIM_TARGET: &str = "probe: engine GameState getter";
/// How far into each state the player is looked for.
pub const SCAN_BYTES: usize = 0x2000;
/// How often the probe looks, once the frames after a change are done.
pub const EVERY_MS: u64 = 3_000;
/// Frames looked at in a row after the game's pointer changes.
pub const FRAMES_AFTER_CHANGE: u32 = 5;
/// Places listed per state, at most.
const MAX_HITS: usize = 16;

/// The offsets the GUI's getter reads: `CMenuUI::m_game`, then the
/// `GameState` in the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuiFields {
    pub game: usize,
    pub state: usize,
}

/// The offsets the game scripts' getter reads: the double buffer in the
/// game, the index word in it, and the first of the two pointers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimFields {
    pub states: usize,
    pub index: usize,
    pub base: usize,
}

fn disp32(code: &[u8], at: usize) -> Option<usize> {
    let disp = i32::from_le_bytes(code.get(at..at + 4)?.try_into().ok()?);
    let offset = usize::try_from(disp).ok()?;
    (offset < 0x1_0000).then_some(offset)
}

/// The GUI getter's fields, if `code` is that getter.
pub fn gui_fields(code: &[u8]) -> Option<GuiFields> {
    let shape = code.get(..19)?;
    let fixed = [
        (0, 0x48),
        (1, 0x8B),
        (2, 0x41),
        (3, 0x08),
        (4, 0x48),
        (5, 0x8B),
        (6, 0x80),
    ]
    .iter()
    .chain([(11, 0x48), (12, 0x8B), (13, 0x80), (18, 0xC3)].iter())
    .all(|&(i, b)| shape[i] == b);
    if !fixed {
        return None;
    }
    let fields = GuiFields {
        game: disp32(code, 7)?,
        state: disp32(code, 14)?,
    };
    (fields.game.is_multiple_of(8) && fields.state.is_multiple_of(8)).then_some(fields)
}

/// The engine getter's fields, if `code` is that getter.
pub fn sim_fields(code: &[u8]) -> Option<SimFields> {
    let code = code.get(..37)?;
    let fixed: &[(usize, u8)] = &[
        (0, 0x80),
        (1, 0x79),
        (3, 0x00),
        (4, 0x48),
        (5, 0x8B),
        (6, 0x41),
        (7, 0x08),
        (8, 0x48),
        (9, 0x8B),
        (10, 0x80),
        (15, 0x74),
        (17, 0xB9),
        (18, 0x01),
        (22, 0x2B),
        (23, 0x88),
        (28, 0x48),
        (29, 0x63),
        (30, 0xD1),
        (31, 0x48),
        (32, 0x8B),
        (33, 0x44),
        (34, 0xD0),
        (36, 0xC3),
    ];
    if !fixed.iter().all(|&(i, b)| code[i] == b) {
        return None;
    }
    let fields = SimFields {
        states: disp32(code, 11)?,
        index: disp32(code, 24)?,
        base: usize::from(code[35]),
    };
    (fields.states.is_multiple_of(8) && fields.base.is_multiple_of(8)).then_some(fields)
}

/// Where `value` stands in `bytes`, at 4-byte steps: as a dword (`d`) or a
/// qword (`q`, when the dword after it is 0), at most [`MAX_HITS`].
pub fn scan(bytes: &[u8], value: u32) -> Vec<(usize, char)> {
    let mut hits = Vec::new();
    let mut at = 0;
    while at + 4 <= bytes.len() && hits.len() < MAX_HITS {
        let word = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap_or_default());
        if word == value {
            let high = bytes
                .get(at + 4..at + 8)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap_or_default()));
            hits.push((
                at,
                if high == Some(0) && at.is_multiple_of(8) {
                    'q'
                } else {
                    'd'
                },
            ));
        }
        at += 4;
    }
    hits
}

fn hits_text(hits: &[(usize, char)]) -> String {
    if hits.is_empty() {
        return "none".into();
    }
    hits.iter()
        .map(|(at, kind)| format!("+{at:#x}{kind}"))
        .collect::<Vec<_>>()
        .join(" ")
}

static ON: AtomicBool = AtomicBool::new(false);
static GUI_GAME: AtomicUsize = AtomicUsize::new(0);
static GUI_STATE: AtomicUsize = AtomicUsize::new(0);
static SIM_STATES: AtomicUsize = AtomicUsize::new(0);
static SIM_INDEX: AtomicUsize = AtomicUsize::new(0);
static SIM_BASE: AtomicUsize = AtomicUsize::new(0);

/// When the probe last looked, and at which game.
struct Pace {
    game: usize,
    frames_left: u32,
    last_ms: u64,
    said_off: bool,
}

static PACE: Mutex<Pace> = Mutex::new(Pace {
    game: usize::MAX,
    frames_left: 0,
    last_ms: 0,
    said_off: false,
});

/// Turns the probe on when [`ENV`] is `1` and both targets resolved and
/// read as the getters they name; returns its log line.
pub fn install(resolved: &ResolvedProfile) -> String {
    install_with(resolved, std::env::var(ENV).ok().as_deref() == Some("1"))
}

pub fn install_with(resolved: &ResolvedProfile, wanted: bool) -> String {
    ON.store(false, Ordering::Release);
    if !wanted {
        return format!("probe: the engine's player is not probed ({ENV}=1 turns it on)");
    }
    let (Some(gui), Some(sim)) = (resolved.get(GUI_TARGET), resolved.get(SIM_TARGET)) else {
        return format!(
            "probe: off, the profile has no {GUI_TARGET} or {SIM_TARGET}; nothing is read"
        );
    };
    let code = |address: u64, len: usize| -> Option<&'static [u8]> {
        let address = usize::try_from(address).ok()?;
        if !crate::image::readable(address, len) {
            return None;
        }
        // SAFETY: `len` bytes of the game's mapped code at the address the
        // profile resolved in this very build, checked readable above; only
        // read.
        Some(unsafe { std::slice::from_raw_parts(address as *const u8, len) })
    };
    let (Some(g), Some(s)) = (
        code(gui.address, 19).and_then(gui_fields),
        code(sim.address, 37).and_then(sim_fields),
    ) else {
        return "probe: off, a getter is not the code it names; nothing is read".into();
    };
    GUI_GAME.store(g.game, Ordering::Release);
    GUI_STATE.store(g.state, Ordering::Release);
    SIM_STATES.store(s.states, Ordering::Release);
    SIM_INDEX.store(s.index, Ordering::Release);
    SIM_BASE.store(s.base, Ordering::Release);
    ON.store(true, Ordering::Release);
    format!(
        "probe: reading the engine's player, read only: the GUI's GameState at [[menu+{:#x}]+{:#x}], the engine's at [[game+{:#x}]+{:#x}+8*i], i at +{:#x}; {} bytes of each scanned every {} s",
        g.game,
        g.state,
        s.states,
        s.base,
        s.index,
        SCAN_BYTES,
        EVERY_MS / 1000
    )
}

/// A pointer at `address`, if it is readable.
fn pointer(address: usize) -> Option<usize> {
    if address == 0 || !crate::image::readable(address, 8) {
        return None;
    }
    // SAFETY: eight readable bytes, checked just above; one unaligned read,
    // never written.
    Some(unsafe { std::ptr::read_unaligned(address as *const usize) })
}

/// [`SCAN_BYTES`] of the object at `address`, if they are readable.
fn object(address: usize) -> Option<Vec<u8>> {
    if address == 0 || !crate::image::readable(address, SCAN_BYTES) {
        return None;
    }
    // SAFETY: SCAN_BYTES readable bytes, checked just above; copied, never
    // written.
    Some(unsafe { std::slice::from_raw_parts(address as *const u8, SCAN_BYTES) }.to_vec())
}

/// One look, after the menu's frame `menu` (a live `UI::CMenuUI`, on its
/// thread): the lines for `hook.log`, none when it is off or not due.
pub fn frame(menu: usize, now_ms: u64) -> Vec<String> {
    if !ON.load(Ordering::Acquire) || menu == 0 {
        return Vec::new();
    }
    let game = pointer(menu + GUI_GAME.load(Ordering::Acquire)).unwrap_or(0);
    let mut pace = PACE.lock().unwrap_or_else(PoisonError::into_inner);
    if game != pace.game {
        pace.game = game;
        pace.frames_left = FRAMES_AFTER_CHANGE;
        pace.said_off = false;
    }
    let due = pace.frames_left > 0 || now_ms.saturating_sub(pace.last_ms) >= EVERY_MS;
    if !due {
        return Vec::new();
    }
    pace.frames_left = pace.frames_left.saturating_sub(1);
    pace.last_ms = now_ms;
    if game == 0 {
        if pace.said_off {
            return Vec::new();
        }
        pace.said_off = true;
        return vec!["probe: at the menu, no CGame (m_game is 0)".into()];
    }
    drop(pace);
    look(game)
}

fn look(game: usize) -> Vec<String> {
    let gui = pointer(game + GUI_STATE.load(Ordering::Acquire)).unwrap_or(0);
    let states = pointer(game + SIM_STATES.load(Ordering::Acquire)).unwrap_or(0);
    let base = SIM_BASE.load(Ordering::Acquire);
    let (b0, b1, index) = if states == 0 {
        (0, 0, None)
    } else {
        let index = SIM_INDEX.load(Ordering::Acquire);
        let i = if crate::image::readable(states + index, 4) {
            // SAFETY: four readable bytes, checked just above; read only.
            Some(unsafe { std::ptr::read_unaligned((states + index) as *const i32) })
        } else {
            None
        };
        (
            pointer(states + base).unwrap_or(0),
            pointer(states + base + 8).unwrap_or(0),
            i,
        )
    };
    let player = crate::lua::noted("tpf3mp.player").and_then(|v| v.trim().parse::<u32>().ok());
    let mut lines = vec![format!(
        "probe: CGame {game:#x}: GUI GameState [+{:#x}] {gui:#x}; engine buffers [+{:#x}] {states:#x}: [0] {b0:#x}, [1] {b1:#x}, i {}; the GUI's is {}",
        GUI_STATE.load(Ordering::Acquire),
        SIM_STATES.load(Ordering::Acquire),
        index.map_or("unreadable".into(), |i| i.to_string()),
        if gui == 0 {
            "unread".to_owned()
        } else if gui == b0 {
            "buffer [0]".to_owned()
        } else if gui == b1 {
            "buffer [1]".to_owned()
        } else {
            "neither buffer".to_owned()
        }
    )];
    match player {
        None => lines.push(
            "probe: the save's player is not known yet (the mod's game script notes it once linked)"
                .into(),
        ),
        Some(player) => {
            for (name, at) in [("GUI", gui), ("engine [0]", b0), ("engine [1]", b1)] {
                let found = object(at).map(|bytes| scan(&bytes, player));
                lines.push(format!(
                    "probe: player {player} in {name} {at:#x}: {}",
                    found.map_or("unreadable".into(), |hits| hits_text(&hits))
                ));
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two getters as build 40408 has them (rva 0x6aa800, 0x11ffd0).
    const GUI: [u8; 19] = [
        0x48, 0x8B, 0x41, 0x08, 0x48, 0x8B, 0x80, 0xB0, 0x06, 0x00, 0x00, 0x48, 0x8B, 0x80, 0xE0,
        0x01, 0x00, 0x00, 0xC3,
    ];
    const SIM: [u8; 37] = [
        0x80, 0x79, 0x10, 0x00, 0x48, 0x8B, 0x41, 0x08, 0x48, 0x8B, 0x80, 0xF0, 0x01, 0x00, 0x00,
        0x74, 0x14, 0xB9, 0x01, 0x00, 0x00, 0x00, 0x2B, 0x88, 0x98, 0x00, 0x00, 0x00, 0x48, 0x63,
        0xD1, 0x48, 0x8B, 0x44, 0xD0, 0x78, 0xC3,
    ];

    #[test]
    fn the_getters_give_their_fields() {
        assert_eq!(
            gui_fields(&GUI),
            Some(GuiFields {
                game: 0x6b0,
                state: 0x1e0
            })
        );
        assert_eq!(
            sim_fields(&SIM),
            Some(SimFields {
                states: 0x1f0,
                index: 0x98,
                base: 0x78
            })
        );
        let mut other = GUI;
        other[18] = 0xCC;
        assert_eq!(gui_fields(&other), None, "not a getter: nothing read");
        let mut other = SIM;
        other[23] = 0x89;
        assert_eq!(sim_fields(&other), None);
        assert_eq!(gui_fields(&GUI[..10]), None);
    }

    #[test]
    fn the_scan_finds_a_dword_and_a_qword() {
        let mut bytes = vec![0u8; 64];
        bytes[8..12].copy_from_slice(&118_368u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&118_368u32.to_le_bytes());
        bytes[24..28].copy_from_slice(&7u32.to_le_bytes());
        assert_eq!(scan(&bytes, 118_368), vec![(8, 'q'), (20, 'd')]);
        assert_eq!(hits_text(&scan(&bytes, 5)), "none");
        assert_eq!(hits_text(&scan(&bytes, 118_368)), "+0x8q +0x14d");
    }

    #[test]
    fn off_unless_asked_and_whole() {
        let empty = ResolvedProfile {
            name: String::new(),
            targets: Vec::new(),
            absent_optional: Vec::new(),
        };
        assert!(install_with(&empty, false).contains("not probed"));
        assert!(install_with(&empty, true).contains("off, the profile has no"));
        assert!(frame(0x1000, 0).is_empty(), "off: reads nothing");
    }
}

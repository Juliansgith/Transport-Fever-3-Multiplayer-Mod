//! The GUI's native tools act as the player's company in a room
//! (docs/HOOKS.md, "The tools' player"; investigation/
//! TF3_LOCAL_PLAYER_2026-10-01.md, "Why the street tool will not split a
//! road the room built").
//!
//! `UI::CGameUI`'s constructor reads the save's player once and hands a copy
//! to each tool it builds. The room builds every road, track and
//! construction for the acting company (`PlayerOwned` = the company), so to
//! a player of any company but the room's first, the native tools take the
//! company's own edges for another player's: no split in the middle of a
//! road (`sub_610ea0` from the street builder's snap), no bulldozing, no
//! tram track onto its rail. This writes the company the GUI notes
//! (`note("tpf3mp.company")`, `tpf3mp/follow.lua`) into three tools' own
//! copies, at the start of each tool's frame, on the main thread:
//!
//! - `UI::StreetBuilder` (the street and the track builder, one class built
//!   twice), its player at `+0xc0` (the constructor's store, profile target
//!   [`STREET_STORE`]);
//! - `UI::TrackModifier` (the road and track modifiers: tram track, bus
//!   lane, electrification, ...), its player at `+0xa0` ([`MODIFIER_STORE`]);
//! - `UI::Bulldozer`, the one entry of its `BulldozerFilter`'s player list
//!   (`[[+0xc0] + 0x10]`, [`BULLDOZER_FILTER`]), which every bulldozer action
//!   asks (`sub_5f7db0`).
//!
//! Each is a UI object, made by `CGameUI` and read by its own class's code
//! only (the readers are listed in the profile). Nothing the simulation
//! runs reads them: the builds a tool makes are stopped at their apply and
//! built by the room as the acting company's, which every game checks
//! (`companies.mayTouch`). A value is written only where the field holds the
//! save's player (as the mod's game script notes it) or the company this
//! wrote; anything else is left alone and said once. Outside a room, for
//! the room's first company, while either note is missing, or with
//! [`ENV`]`=0`, the game's own value stays (and one this wrote goes back).

#![allow(unsafe_code)]

use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use tpf3mp_hookcore::profile::ResolvedProfile;

/// The kill switch: `0` (or `off`, `false`, `no`) leaves every tool the
/// save's player's.
pub const ENV: &str = "TPF3MP_HOOK_TOOL_COMPANY";
/// The name in hook.log.
pub const FIX: &str = "tool-company";
/// The note the GUI keeps the player's company's entity under
/// (`tpf3mp/follow.lua`, `COMPANY_NOTE`), "" for none.
pub const COMPANY_NOTE: &str = "tpf3mp.company";

/// `UI::StreetBuilder::Step` (vf5, rva 0x585e50).
pub const STREET_STEP: &str = "UI::StreetBuilder::Step";
/// The street builder constructor's store of its player (rva 0x56a7f4:
/// `... mov eax,[rbp+0x128]; mov [rsi+0xc0],eax`).
pub const STREET_STORE: &str = "UI::StreetBuilder ctor/player store";
/// `UI::TrackModifier::Step` (vf5, rva 0x5cbf80).
pub const MODIFIER_STEP: &str = "UI::TrackModifier::Step";
/// The track modifier constructor's store of its player (rva 0x5b4238:
/// `... mov eax,[rbp+0x330]; mov [r14+0xa0],eax`).
pub const MODIFIER_STORE: &str = "UI::TrackModifier ctor/player store";
/// `UI::Bulldozer::Step` (vf5, rva 0x4d6340).
pub const BULLDOZER_STEP: &str = "UI::Bulldozer::Step";
/// The bulldozer constructor's `BulldozerFilter` (rva 0x4c4c32: its vtable,
/// its player list at `+0x10`, and the filter's place in the bulldozer).
pub const BULLDOZER_FILTER: &str = "UI::Bulldozer ctor/filter";

/// The street builder's store: the bytes before the field's disp32, the
/// field's disp32 at [`STREET_DISP_AT`], and the length read.
pub const STREET_STORE_BYTES: [u8; 40] = [
    0x48, 0x8B, 0x85, 0x10, 0x01, 0x00, 0x00, 0x48, 0x89, 0x86, 0xB0, 0x00, 0x00, 0x00, 0x48, 0x8B,
    0x85, 0x18, 0x01, 0x00, 0x00, 0x48, 0x89, 0x86, 0xB8, 0x00, 0x00, 0x00, 0x8B, 0x85, 0x28, 0x01,
    0x00, 0x00, 0x89, 0x86, 0xC0, 0x00, 0x00, 0x00,
];
pub const STREET_DISP_AT: usize = 36;
/// The track modifier's store.
pub const MODIFIER_STORE_BYTES: [u8; 27] = [
    0x48, 0x8B, 0x85, 0x20, 0x03, 0x00, 0x00, 0x49, 0x89, 0x86, 0x98, 0x00, 0x00, 0x00, 0x8B, 0x85,
    0x30, 0x03, 0x00, 0x00, 0x41, 0x89, 0x86, 0xA0, 0x00, 0x00, 0x00,
];
pub const MODIFIER_DISP_AT: usize = 23;
/// The bulldozer's filter: `lea rax,[rip+vtable]` (disp32 at 3, wildcard),
/// the filter's fields set (its player list at `+0x10..+0x20`), and `mov
/// [r14+disp32],rbx` (disp32 at [`FILTER_DISP_AT`]).
pub const FILTER_BYTES: [Option<u8>; 48] = {
    const B: [u8; 48] = [
        0x48, 0x8D, 0x05, 0, 0, 0, 0, 0x48, 0x89, 0x03, 0x48, 0x89, 0x73, 0x08, 0x48, 0x89, 0x7B,
        0x10, 0x48, 0x89, 0x7B, 0x18, 0x48, 0x89, 0x7B, 0x20, 0x66, 0xC7, 0x43, 0x28, 0x00, 0x00,
        0xEB, 0x03, 0x48, 0x8B, 0xDF, 0x48, 0x89, 0x5D, 0xD8, 0x49, 0x89, 0x9E, 0xC0, 0x00, 0x00,
        0x00,
    ];
    let mut out = [None; 48];
    let mut i = 0;
    while i < 48 {
        if i < 3 || i >= 7 {
            out[i] = Some(B[i]);
        }
        i += 1;
    }
    out
};
pub const FILTER_DISP_AT: usize = 44;
/// The filter's player list (a `std::vector<Entity>`: begin, end, capacity).
pub const FILTER_PLAYERS: usize = 0x10;

/// Reads a little-endian disp32 at `at` as an offset below 64 KiB.
fn field_disp(bytes: &[u8], at: usize) -> Option<usize> {
    let disp = i32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?);
    let offset = usize::try_from(disp).ok()?;
    (offset < 0x1_0000 && offset.is_multiple_of(4)).then_some(offset)
}

/// The street builder's or the track modifier's player offset, if `code`
/// is the store expected (`expected`, with the field's disp32 at `at`).
pub fn store_offset(code: &[u8], expected: &[u8], at: usize) -> Option<usize> {
    let code = code.get(..expected.len())?;
    let same = code
        .iter()
        .zip(expected)
        .enumerate()
        .all(|(i, (c, e))| (at..at + 4).contains(&i) || c == e);
    if !same {
        return None;
    }
    field_disp(code, at)
}

/// The bulldozer filter's layout read from its constructor's code at
/// address `site`: the filter's vtable (absolute) and its place in the
/// bulldozer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterLayout {
    pub vtable: usize,
    pub filter: usize,
}

pub fn filter_layout(code: &[u8], site: usize) -> Option<FilterLayout> {
    let code = code.get(..48)?;
    let same = code
        .iter()
        .zip(FILTER_BYTES.iter())
        .all(|(c, e)| e.is_none_or(|e| *c == e));
    if !same {
        return None;
    }
    let rel = i32::from_le_bytes(code.get(3..7)?.try_into().ok()?);
    let vtable = site
        .checked_add(7)?
        .checked_add_signed(isize::try_from(rel).ok()?)?;
    let filter = usize::try_from(i32::from_le_bytes(
        code.get(FILTER_DISP_AT..FILTER_DISP_AT + 4)?
            .try_into()
            .ok()?,
    ))
    .ok()?;
    (filter < 0x1_0000 && filter.is_multiple_of(8)).then_some(FilterLayout { vtable, filter })
}

/// What to do with one tool's player field this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Leave it as it is.
    Leave,
    /// Write this player.
    Write(i32),
    /// It holds a value this does not know: leave it, and say so once.
    Unknown,
}

/// One tool object's field: what the game put there and what this wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Slot {
    pub original: Option<i32>,
    pub wrote: Option<i32>,
}

/// Decides for a field holding `current`. `company` is the player's
/// company, `save` the save's player, each when known; `active` whether
/// this acts at all (in a room, switched on).
pub fn decide(
    current: i32,
    slot: Slot,
    save: Option<i32>,
    company: Option<i32>,
    active: bool,
) -> Decision {
    let want = match (active, save, company) {
        (true, Some(save), Some(company)) if company >= 0 && company != save => Some(company),
        _ => None,
    };
    match want {
        Some(company) => {
            if current == company {
                Decision::Leave
            } else if Some(current) == save || Some(current) == slot.wrote {
                Decision::Write(company)
            } else {
                Decision::Unknown
            }
        }
        None => match (slot.wrote, slot.original) {
            (Some(wrote), Some(original)) if wrote == current && original != current => {
                Decision::Write(original)
            }
            _ => Decision::Leave,
        },
    }
}

/// Which tool a field is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Street,
    Modifier,
    Bulldozer,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Tool::Street => "the street and track builder",
            Tool::Modifier => "the road and track modifier",
            Tool::Bulldozer => "the bulldozer",
        }
    }
}

/// The objects seen, by the field's address: at most this many are kept
/// (CGameUI makes a handful; a new world makes them anew).
const SLOTS: usize = 32;

struct State {
    slots: Vec<(usize, Slot)>,
    said: Vec<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    slots: Vec::new(),
    said: Vec::new(),
});
static ON: AtomicBool = AtomicBool::new(false);
/// Set when a frame's work panicked: no more writes for this game.
static BROKEN: AtomicBool = AtomicBool::new(false);
static STREET_FIELD: AtomicUsize = AtomicUsize::new(0);
static MODIFIER_FIELD: AtomicUsize = AtomicUsize::new(0);
static FILTER_FIELD: AtomicUsize = AtomicUsize::new(0);
static FILTER_VTABLE: AtomicUsize = AtomicUsize::new(0);
static STREET_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static MODIFIER_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static BULLDOZER_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

fn say_once(state: &mut State, line: String) {
    if state.said.len() < 64 && !state.said.contains(&line) {
        crate::log::line(&line);
        state.said.push(line);
    }
}

fn noted(key: &str) -> Option<i32> {
    crate::lua::noted(key)
        .and_then(|v| v.trim().parse::<i32>().ok())
        .filter(|&v| v >= 0)
}

fn read_usize(address: usize) -> Option<usize> {
    if address == 0 || !crate::image::readable(address, 8) {
        return None;
    }
    // SAFETY: eight readable bytes, checked just above.
    Some(unsafe { std::ptr::read_unaligned(address as *const usize) })
}

/// The address of `tool`'s player field in the object `this`, if its shape
/// is the one expected.
fn field_of(tool: Tool, this: usize) -> Option<usize> {
    match tool {
        Tool::Street => this.checked_add(STREET_FIELD.load(Ordering::Acquire)),
        Tool::Modifier => this.checked_add(MODIFIER_FIELD.load(Ordering::Acquire)),
        Tool::Bulldozer => {
            let filter = read_usize(this.checked_add(FILTER_FIELD.load(Ordering::Acquire))?)?;
            if read_usize(filter)? != FILTER_VTABLE.load(Ordering::Acquire) {
                return None;
            }
            let begin = read_usize(filter + FILTER_PLAYERS)?;
            let end = read_usize(filter + FILTER_PLAYERS + 8)?;
            // Exactly one player, as the constructor makes the list.
            (end.checked_sub(begin) == Some(4)).then_some(begin)
        }
    }
}

/// One tool's frame, before the game's own: brings its player field to
/// what [`decide`] says.
fn frame(tool: Tool, this: usize) {
    if !ON.load(Ordering::Acquire) || BROKEN.load(Ordering::Acquire) || this == 0 {
        return;
    }
    let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(field) = field_of(tool, this) else {
        say_once(
            &mut state,
            format!(
                "{FIX}: {} at {this:#x} is not the shape expected; its player is left as the game made it",
                tool.name()
            ),
        );
        return;
    };
    if !crate::image::readable(field, 4) || !field.is_multiple_of(4) {
        return;
    }
    // SAFETY: four readable, aligned bytes of the tool's own object, read
    // on the main thread, which owns the tool.
    let current = unsafe { std::ptr::read_volatile(field as *const i32) };
    let at = state.slots.iter().position(|(f, _)| *f == field);
    let slot = at.map(|i| state.slots[i].1).unwrap_or_default();
    let save = noted("tpf3mp.player");
    let company = noted(COMPANY_NOTE);
    match decide(current, slot, save, company, crate::lua::in_room()) {
        Decision::Leave => {}
        Decision::Unknown => say_once(
            &mut state,
            format!(
                "{FIX}: {} holds player {current}, neither the save's player {} nor a company this wrote; left alone",
                tool.name(),
                save.map_or("(unknown)".into(), |s| s.to_string())
            ),
        ),
        Decision::Write(value) => {
            // SAFETY: the tool's own field, checked readable and aligned
            // above; the tool runs on this thread, and its pool tasks read
            // the whole aligned dword, old or new.
            unsafe { std::ptr::write_volatile(field as *mut i32, value) };
            let restoring = slot.original == Some(value);
            let next = Slot {
                original: slot.original.or(Some(current)),
                wrote: (!restoring).then_some(value),
            };
            match at {
                Some(i) => state.slots[i].1 = next,
                None => {
                    if state.slots.len() >= SLOTS {
                        state.slots.remove(0);
                    }
                    state.slots.push((field, next));
                }
            }
            crate::log::line(&if restoring {
                format!(
                    "{FIX}: {} at {this:#x} acts as the save's player {value} again",
                    tool.name()
                )
            } else {
                format!(
                    "{FIX}: {} at {this:#x} acts as the player's company {value} (was player {current})",
                    tool.name()
                )
            });
        }
    }
}

/// A tool's frame, guarded: a panic switches this off for the game.
fn before(tool: Tool, this: usize) {
    if std::panic::catch_unwind(|| frame(tool, this)).is_err() {
        BROKEN.store(true, Ordering::Release);
    }
}

extern "system" fn before_street(this: usize) {
    before(Tool::Street, this);
}
extern "system" fn before_modifier(this: usize) {
    before(Tool::Modifier, this);
}
extern "system" fn before_bulldozer(this: usize) {
    before(Tool::Bulldozer, this);
}

/// An entry for a tool's `Step`: saves the four argument registers and
/// xmm0-3, calls `$before` with `this`, restores them and jumps to the
/// original through `$original` with the stack as the caller left it.
macro_rules! entry {
    ($name:ident, $before:ident, $original:ident) => {
        #[cfg(all(windows, target_arch = "x86_64"))]
        #[unsafe(naked)]
        unsafe extern "C" fn $name() {
            core::arch::naked_asm!(
                // Entry rsp is 8 mod 16; four pushes keep it so, and 0x68
                // makes it 16-aligned with 0x20 of shadow space under the
                // four saved xmm registers.
                "push rcx",
                "push rdx",
                "push r8",
                "push r9",
                "sub rsp, 0x68",
                "movaps [rsp + 0x20], xmm0",
                "movaps [rsp + 0x30], xmm1",
                "movaps [rsp + 0x40], xmm2",
                "movaps [rsp + 0x50], xmm3",
                "call {before}",
                "movaps xmm0, [rsp + 0x20]",
                "movaps xmm1, [rsp + 0x30]",
                "movaps xmm2, [rsp + 0x40]",
                "movaps xmm3, [rsp + 0x50]",
                "add rsp, 0x68",
                "pop r9",
                "pop r8",
                "pop rdx",
                "pop rcx",
                "jmp qword ptr [rip + {original}]",
                before = sym $before,
                original = sym $original,
            )
        }
    };
}

entry!(street_entry, before_street, STREET_ORIGINAL);
entry!(modifier_entry, before_modifier, MODIFIER_ORIGINAL);
entry!(bulldozer_entry, before_bulldozer, BULLDOZER_ORIGINAL);

/// `len` bytes of the game's code at `address`, if readable.
fn code(address: u64, len: usize) -> Option<&'static [u8]> {
    let address = usize::try_from(address).ok()?;
    if !crate::image::readable(address, len) {
        return None;
    }
    // SAFETY: `len` readable bytes of the game's mapped code, only read.
    Some(unsafe { std::slice::from_raw_parts(address as *const u8, len) })
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn detour(
    target: u64,
    entry: unsafe extern "C" fn(),
    original: &AtomicUsize,
) -> Result<(), String> {
    // SAFETY: a function the profile resolved and prologue-checked in this
    // build, detoured while the game starts, before any tool exists; the
    // entry restores every argument register before it jumps on.
    let installed = unsafe {
        tpf3mp_hookcore::detour::InlineDetour::install(
            target as usize as *mut u8,
            entry as *const u8,
        )
    };
    match installed {
        Ok(detoured) => {
            original.store(detoured.trampoline() as usize, Ordering::Release);
            let _kept = std::mem::ManuallyDrop::new(detoured);
            Ok(())
        }
        Err(error) => Err(format!("{error:?}")),
    }
}

/// Installs the three tools' frames unless [`ENV`] says no; the lines for
/// hook.log.
pub fn install(resolved: &ResolvedProfile) -> Vec<String> {
    install_with(
        resolved,
        crate::ticks::wanted(std::env::var(ENV).ok().as_deref()),
    )
}

pub fn install_with(resolved: &ResolvedProfile, wanted: bool) -> Vec<String> {
    ON.store(false, Ordering::Release);
    if !wanted {
        return vec![format!(
            "{FIX}: off, {ENV} says so; the native tools act as the save's player"
        )];
    }
    install_tools(resolved)
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn install_tools(resolved: &ResolvedProfile) -> Vec<String> {
    let mut lines = Vec::new();
    let mut any = false;
    let tools: [(Tool, &str, &str, unsafe extern "C" fn(), &AtomicUsize); 3] = [
        (
            Tool::Street,
            STREET_STEP,
            STREET_STORE,
            street_entry,
            &STREET_ORIGINAL,
        ),
        (
            Tool::Modifier,
            MODIFIER_STEP,
            MODIFIER_STORE,
            modifier_entry,
            &MODIFIER_ORIGINAL,
        ),
        (
            Tool::Bulldozer,
            BULLDOZER_STEP,
            BULLDOZER_FILTER,
            bulldozer_entry,
            &BULLDOZER_ORIGINAL,
        ),
    ];
    for (tool, step, shape, entry, original) in tools {
        let (Some(step_at), Some(shape_at)) = (resolved.get(step), resolved.get(shape)) else {
            lines.push(format!(
                "{FIX}: {} stays the save's player's: the profile lacks {step} or {shape}",
                tool.name()
            ));
            continue;
        };
        let layout = match tool {
            Tool::Street => code(shape_at.address, STREET_STORE_BYTES.len())
                .and_then(|c| store_offset(c, &STREET_STORE_BYTES, STREET_DISP_AT))
                .map(|f| STREET_FIELD.store(f, Ordering::Release)),
            Tool::Modifier => code(shape_at.address, MODIFIER_STORE_BYTES.len())
                .and_then(|c| store_offset(c, &MODIFIER_STORE_BYTES, MODIFIER_DISP_AT))
                .map(|f| MODIFIER_FIELD.store(f, Ordering::Release)),
            Tool::Bulldozer => code(shape_at.address, 48)
                .and_then(|c| filter_layout(c, shape_at.address as usize))
                .map(|l| {
                    FILTER_FIELD.store(l.filter, Ordering::Release);
                    FILTER_VTABLE.store(l.vtable, Ordering::Release);
                }),
        };
        if layout.is_none() {
            lines.push(format!(
                "{FIX}: {} stays the save's player's: {shape} at {:#x} is not the code expected",
                tool.name(),
                shape_at.address
            ));
            continue;
        }
        match detour(step_at.address, entry, original) {
            Ok(()) => {
                any = true;
                lines.push(format!(
                    "{FIX}: {} acts as the player's company in a room ({step} at {:#x}; {ENV}=0 turns it off)",
                    tool.name(),
                    step_at.address
                ));
            }
            Err(error) => lines.push(format!(
                "{FIX}: {} stays the save's player's: detouring {step} failed: {error}",
                tool.name()
            )),
        }
    }
    ON.store(any, Ordering::Release);
    lines
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
fn install_tools(_resolved: &ResolvedProfile) -> Vec<String> {
    vec![format!("{FIX}: off, Windows x86-64 only")]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stores_give_their_fields() {
        assert_eq!(
            store_offset(&STREET_STORE_BYTES, &STREET_STORE_BYTES, STREET_DISP_AT),
            Some(0xc0)
        );
        assert_eq!(
            store_offset(
                &MODIFIER_STORE_BYTES,
                &MODIFIER_STORE_BYTES,
                MODIFIER_DISP_AT
            ),
            Some(0xa0)
        );
        let mut other = STREET_STORE_BYTES;
        other[30] = 0x30;
        assert_eq!(
            store_offset(&other, &STREET_STORE_BYTES, STREET_DISP_AT),
            None,
            "another source: refused"
        );
        assert_eq!(
            store_offset(
                &STREET_STORE_BYTES[..20],
                &STREET_STORE_BYTES,
                STREET_DISP_AT
            ),
            None
        );
    }

    #[test]
    fn the_filter_gives_its_vtable_and_place() {
        // Build 40408: lea rax,[rip+0x31ea337] at rva 0x4c4c32.
        let mut bytes: Vec<u8> = FILTER_BYTES.iter().map(|b| b.unwrap_or(0)).collect();
        bytes[3..7].copy_from_slice(&0x031e_a337_i32.to_le_bytes());
        assert_eq!(
            filter_layout(&bytes, 0x1_404c_4c32),
            Some(FilterLayout {
                vtable: 0x1_436a_ef70,
                filter: 0xc0
            })
        );
        bytes[17] = 0x18;
        assert_eq!(
            filter_layout(&bytes, 0x1_404c_4c32),
            None,
            "players elsewhere"
        );
    }

    #[test]
    fn a_tool_acts_as_the_company_only_in_a_room_and_goes_back() {
        let save = Some(214_443);
        let company = Some(372_363);
        let fresh = Slot::default();
        // Outside a room, or without a company: the game's own.
        assert_eq!(
            decide(214_443, fresh, save, company, false),
            Decision::Leave
        );
        assert_eq!(decide(214_443, fresh, save, None, true), Decision::Leave);
        assert_eq!(decide(214_443, fresh, None, company, true), Decision::Leave);
        // The room's first company is the save's player.
        assert_eq!(decide(214_443, fresh, save, save, true), Decision::Leave);
        // In a room, for another company: written.
        assert_eq!(
            decide(214_443, fresh, save, company, true),
            Decision::Write(372_363)
        );
        let written = Slot {
            original: Some(214_443),
            wrote: Some(372_363),
        };
        assert_eq!(
            decide(372_363, written, save, company, true),
            Decision::Leave
        );
        // The player switches company: the new one.
        assert_eq!(
            decide(372_363, written, save, Some(380_000), true),
            Decision::Write(380_000)
        );
        // Leaving the room: back to the save's player.
        assert_eq!(
            decide(372_363, written, save, company, false),
            Decision::Write(214_443)
        );
        assert_eq!(
            decide(372_363, written, save, None, true),
            Decision::Write(214_443)
        );
        // A value this does not know is left alone.
        assert_eq!(decide(5, fresh, save, company, true), Decision::Unknown);
        assert_eq!(decide(5, written, save, company, false), Decision::Leave);
    }

    #[test]
    fn off_unless_wanted() {
        let empty = ResolvedProfile {
            name: String::new(),
            targets: Vec::new(),
            absent_optional: Vec::new(),
        };
        assert!(install_with(&empty, false)[0].contains("off"));
        let lines = install_with(&empty, true);
        assert!(
            lines
                .iter()
                .all(|l| l.contains("stays the save's player's") || l.contains("Windows"))
        );
        frame(Tool::Street, 0x1000);
    }
}

//! The emission throttle: a PROPOSAL that CHANGES SIMULATION RESULTS.
//!
//! On a 100 x 1000 tile world the noise and pollution grids hold 25.6 M
//! cells each, and `ecs::EmissionGridSystem::Update` sweeps both every
//! update: about 1.75 GB of memory traffic, 30 to 60 ms of a ~260 ms update
//! (investigation/TF3_BIGMAP_SIM_COST_2026-10-05.md §1). With
//! [`EVERY_ENV`] set to `N` (2 to [`MAX_EVERY`]), the grid's update and the
//! emitters' splat (`ecs::EmissionEmitterSystem::Update2`) run only on the
//! updates whose `GameTime.updateCount` is a multiple of `N`, each with the
//! `dt` the game passes. On the other updates both are skipped, so the
//! grids stand still.
//!
//! **Why the normal `dt`.** The grid's `Update` scales its weights by
//! `5·dt` and also runs `round(dt / 0.2)` steps: with `dt·N` the weights
//! sum to `N·(1 − decay) > 1` and the field grows without bound (the tests
//! run the game's own `Diffuse` to show it). With the normal `dt` and both
//! systems skipped together, the grids follow exactly the stock trajectory,
//! `N` times slower: the state after `k·N` throttled updates is the stock
//! state after `k` updates, bit for bit, for emitters that do not change.
//! The steady field is the same; the response to a change is `N` times
//! slower. Town noise and pollution ratings, the eco mechanics and the
//! script getters read the grids, so their results change.
//!
//! **Why `updateCount`.** It is `GameTime`'s update counter (+0x40),
//! advanced once before each `ecs::Engine::Update` on the running path and
//! never on the paused one (crate::ticks); it is in the save, and two games
//! of a room log it equal at every checkpoint. `ecs::Engine::Update` calls
//! every system's update serially on the step's thread, after that advance,
//! so both systems see the same count in an update and every game of a room
//! skips the same updates. The hook reads it through the game's own getter
//! from the step's `CGameTime` ([`crate::install::update_count_now`]); a
//! count it cannot read runs the stock update (and says so once).
//!
//! **Saves.** Nothing new is stored: the grids keep their size, the skipped
//! updates leave them as they were, and the phase comes from the saved
//! counter, so a save and reload, or a room's rebase, skips on the same
//! updates as before.
//!
//! **How.** Two vtable slots, never code: `EmissionGridSystem`'s slot 11
//! and `EmissionEmitterSystem`'s slot 12 point at gates that call the
//! function the slot held. Each slot is checked to hold the function the
//! profile resolved before it is rewritten, both are rewritten or neither,
//! and a target the profile lacks leaves the game stock with the reason in
//! `hook.log`. Patching the slots composes with any inline detour of the
//! functions themselves.
//!
//! **Every game of a room** must run the same `N`. The room does not
//! compare it yet (docs/BIGMAPS.md, "Simulation switches (proposal)"); the
//! hook says the setting in `hook.log` at install and at the first skipped
//! update.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use tpf3mp_hookcore::detour::Rewrite;
use tpf3mp_hookcore::profile::ResolvedProfile;

pub use crate::build_data::native::simswitch::{
    EMITTER_SLOT, EMITTER_UPDATE2, EMITTER_VTABLE, GRID_SLOT, GRID_UPDATE, GRID_VTABLE, LEA_RAX_RIP,
};
use crate::log;

/// The patch's name in `hook.log`.
pub const FIX: &str = "big maps: emission throttle (PROPOSAL, changes simulation results)";

/// `N` in the game's environment (2 to [`MAX_EVERY`]) runs the emission
/// systems on every `N`-th update; unset, `0`, `1` or `off` leaves them as
/// the game has them.
pub const EVERY_ENV: &str = "TPF3MP_BIGMAP_EMISSION_EVERY";

/// The largest `N`: past it a town's ratings would answer changes minutes
/// late at 1x.
pub const MAX_EVERY: u32 = 16;

/// The setting from [`EVERY_ENV`]'s value: `None` is the game's own.
pub fn every_setting(value: Option<&str>) -> Result<Option<u32>, String> {
    let value = value.map(|v| v.trim().to_ascii_lowercase());
    match value.as_deref() {
        None | Some("" | "0" | "1" | "off" | "false" | "no") => Ok(None),
        Some(text) => match text.parse::<u32>() {
            Ok(n) if (2..=MAX_EVERY).contains(&n) => Ok(Some(n)),
            _ => Err(format!(
                "{EVERY_ENV}={text:?} is not 2 to {MAX_EVERY} (or 0 for the game's own)"
            )),
        },
    }
}

/// Whether the emission systems run in the update numbered `update_count`:
/// on every `every`-th, always when the throttle is off (`every` below 2),
/// and always when the count is unread.
pub fn runs(update_count: Option<u32>, every: u32) -> bool {
    match update_count {
        _ if every < 2 => true,
        None => true,
        Some(count) => count.is_multiple_of(every),
    }
}

/// `N`; 0 while not installed.
static EVERY: AtomicU32 = AtomicU32::new(0);
/// What each slot held: the game's functions (or whatever detours them).
static GRID_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static EMITTER_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static RAN: AtomicU64 = AtomicU64::new(0);
static SKIPPED: AtomicU64 = AtomicU64::new(0);
static UNREAD_SAID: AtomicBool = AtomicBool::new(false);

/// The count the gates read: the game's, or a test's.
fn update_count() -> Option<u32> {
    #[cfg(test)]
    {
        let forced = tests::FORCED_COUNT.load(Ordering::Acquire);
        if forced >= 0 {
            return u32::try_from(forced).ok();
        }
    }
    crate::install::update_count_now()
}

/// The decision for one call of a gated system, with its log lines. Only
/// the grid's gate counts and logs, so each update is counted once.
fn gate(counting: bool) -> bool {
    let every = EVERY.load(Ordering::Acquire);
    let count = update_count();
    if count.is_none() && every >= 2 && !UNREAD_SAID.swap(true, Ordering::AcqRel) {
        log::line(&format!(
            "{FIX}: updateCount unread outside the game's step; those updates run the stock emission systems"
        ));
    }
    let run = runs(count, every);
    if counting {
        if run {
            RAN.fetch_add(1, Ordering::Relaxed);
        } else {
            let skipped = SKIPPED.fetch_add(1, Ordering::Relaxed) + 1;
            if skipped == 1 || skipped.is_multiple_of(1 << 14) {
                log::line(&format!(
                    "{FIX}: {EVERY_ENV}={every}: skipping the emission systems at updateCount {} (ran {}, skipped {skipped}); every game of the room must run {EVERY_ENV}={every}",
                    count.unwrap_or_default(),
                    RAN.load(Ordering::Relaxed),
                ));
            }
        }
    }
    run
}

/// `EmissionGridSystem::Update(this, Engine*, INodeList*, float dt)`.
type GridUpdate = unsafe extern "system" fn(usize, usize, usize, f32);
/// `EmissionEmitterSystem::Update2(this, Engine*, int, float dt)`.
type EmitterUpdate = unsafe extern "system" fn(usize, usize, usize, f32);

/// Slot 11 of `EmissionGridSystem`'s vtable.
/// The grid gate's address, as a slot would hold it.
pub fn grid_gate_address() -> u64 {
    grid_gate as *const () as usize as u64
}

/// Whether `address` is one of the throttle's gates, which another slot
/// hook (the `perf: sim` timers) may find in a slot and wrap.
pub fn is_gate(address: u64) -> bool {
    address == grid_gate as *const () as usize as u64
        || address == emitter_gate as *const () as usize as u64
}

unsafe extern "system" fn grid_gate(this: usize, engine: usize, nodes: usize, dt: f32) {
    let original = GRID_ORIGINAL.load(Ordering::Acquire);
    if original == 0 || !gate(true) {
        return;
    }
    // SAFETY: what the slot held, called with the arguments the engine
    // passed (the function's own ABI: dt in xmm3).
    unsafe { std::mem::transmute::<usize, GridUpdate>(original)(this, engine, nodes, dt) }
}

/// Slot 12 of `EmissionEmitterSystem`'s vtable.
unsafe extern "system" fn emitter_gate(this: usize, engine: usize, count: usize, dt: f32) {
    let original = EMITTER_ORIGINAL.load(Ordering::Acquire);
    if original == 0 || !gate(false) {
        return;
    }
    // SAFETY: as for `grid_gate`.
    unsafe { std::mem::transmute::<usize, EmitterUpdate>(original)(this, engine, count, dt) }
}

/// What installing came to, for `hook.log`.
pub fn outcome_line(installed: bool, reason: &str) -> String {
    if installed {
        format!("{FIX}: installed ({reason})")
    } else {
        format!("{FIX}: off, {reason}")
    }
}

/// Installs the throttle if [`EVERY_ENV`] asks for it. Returns the line
/// for `hook.log`.
pub fn install(resolved: &ResolvedProfile) -> String {
    match every_setting(std::env::var(EVERY_ENV).ok().as_deref()) {
        Ok(every) => install_with(resolved, every),
        Err(why) => outcome_line(false, &why),
    }
}

pub fn install_with(resolved: &ResolvedProfile, every: Option<u32>) -> String {
    match install_slots(resolved, every) {
        Ok(slots) => {
            let at = slots
                .iter()
                .map(|s| format!("{:#x}", s.at))
                .collect::<Vec<_>>();
            let every = EVERY.load(Ordering::Acquire);
            std::mem::forget(slots);
            outcome_line(
                true,
                &format!(
                    "{EVERY_ENV}={every}: the emission grid and emitters run only when updateCount % {every} == 0, with the game's dt; vtable slots at {}; every game of a room must run {EVERY_ENV}={every}, and the room does not check it yet",
                    at.join(", ")
                ),
            )
        }
        Err(why) => outcome_line(false, &why),
    }
}

/// One rewritten slot.
pub(crate) struct Slot {
    pub at: usize,
    rewrite: Rewrite,
}

impl Slot {
    /// Restores the slot.
    ///
    /// # Safety
    ///
    /// No thread calls through the slot while it is written.
    pub(crate) unsafe fn detach(self) {
        // SAFETY: the caller's.
        let _ = unsafe { self.rewrite.detach() };
    }
}

/// The vtable slot a `lea rax,[rip+disp32]` at `lea` and a slot index
/// name, and the address the slot must hold.
fn slot_of(
    resolved: &ResolvedProfile,
    lea_target: &str,
    function_target: &str,
    slot: usize,
) -> Result<(usize, usize), String> {
    let lea = resolved
        .get(lea_target)
        .ok_or_else(|| format!("the profile has no {lea_target:?}"))?
        .address;
    let function = resolved
        .get(function_target)
        .ok_or_else(|| format!("the profile has no {function_target:?}"))?
        .address;
    let lea = usize::try_from(lea).map_err(|_| "an address past usize".to_owned())?;
    let function = usize::try_from(function).map_err(|_| "an address past usize".to_owned())?;
    if !crate::image::readable(lea, 7) {
        return Err(format!("{lea_target} at {lea:#x} is unreadable"));
    }
    // SAFETY: 7 readable bytes, checked just above.
    let bytes = unsafe { std::slice::from_raw_parts(lea as *const u8, 7) };
    if bytes[..3] != LEA_RAX_RIP {
        return Err(format!(
            "{lea_target} at {lea:#x} is not `lea rax,[rip+disp32]`"
        ));
    }
    let disp = i32::from_le_bytes([bytes[3], bytes[4], bytes[5], bytes[6]]);
    let vtable = (lea + 7).wrapping_add_signed(disp as isize);
    let at = vtable + slot * 8;
    if !crate::image::readable(at, 8) {
        return Err(format!(
            "{lea_target}'s slot {slot} at {at:#x} is unreadable"
        ));
    }
    // SAFETY: 8 readable bytes, checked just above.
    let held = unsafe { std::ptr::read_unaligned(at as *const usize) };
    if held != function {
        return Err(format!(
            "{lea_target}'s slot {slot} at {at:#x} holds {held:#x}, not {function_target} at {function:#x}"
        ));
    }
    Ok((at, function))
}

/// Rewrites both slots, or neither.
pub(crate) fn install_slots(
    resolved: &ResolvedProfile,
    every: Option<u32>,
) -> Result<Vec<Slot>, String> {
    let Some(every) = every else {
        return Err(format!(
            "{EVERY_ENV} is not set; the game's own emission updates"
        ));
    };
    let grid = slot_of(resolved, GRID_VTABLE, GRID_UPDATE, GRID_SLOT)?;
    let emitter = slot_of(resolved, EMITTER_VTABLE, EMITTER_UPDATE2, EMITTER_SLOT)?;
    GRID_ORIGINAL.store(grid.1, Ordering::Release);
    EMITTER_ORIGINAL.store(emitter.1, Ordering::Release);
    EVERY.store(every, Ordering::Release);
    let mut slots = Vec::new();
    for ((at, held), gate) in [
        (grid, grid_gate as *const () as usize),
        (emitter, emitter_gate as *const () as usize),
    ] {
        // SAFETY: a vtable slot checked to hold `held`; no world exists
        // yet, so no thread calls through it; the gate has the function's
        // ABI and calls what the slot held.
        match unsafe { Rewrite::install(at as *mut u8, &held.to_le_bytes(), &gate.to_le_bytes()) } {
            Ok(rewrite) => slots.push(Slot { at, rewrite }),
            Err(error) => {
                for slot in slots {
                    // SAFETY: as above.
                    unsafe { slot.detach() };
                }
                EVERY.store(0, Ordering::Release);
                return Err(format!("the slot at {at:#x}: {error}"));
            }
        }
    }
    Ok(slots)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::AtomicI64;

    use tpf3mp_hookcore::profile::ResolvedTarget;

    use super::*;

    /// A count the gates read instead of the game's; -1 none forced.
    pub(crate) static FORCED_COUNT: AtomicI64 = AtomicI64::new(-1);

    #[test]
    fn the_setting_is_off_unless_it_names_n() {
        for off in [
            None,
            Some(""),
            Some("0"),
            Some("1"),
            Some(" OFF "),
            Some("no"),
        ] {
            assert_eq!(every_setting(off), Ok(None), "{off:?}");
        }
        assert_eq!(every_setting(Some("2")), Ok(Some(2)));
        assert_eq!(every_setting(Some(" 4 ")), Ok(Some(4)));
        assert_eq!(every_setting(Some("16")), Ok(Some(16)));
        for bad in ["17", "-2", "yes", "4.0", "x"] {
            assert!(every_setting(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_gate_runs_every_nth_update_of_the_saved_counter() {
        assert!((0..100).all(|c| runs(Some(c), 0)), "off: always");
        assert!((0..100).all(|c| runs(Some(c), 1)));
        assert!(runs(None, 4), "unread: the stock update");
        let ran: Vec<u32> = (100..120).filter(|&c| runs(Some(c), 4)).collect();
        assert_eq!(ran, [100, 104, 108, 112, 116]);
        // The phase is the counter's, not the process's: a reload at any
        // count keeps the same updates.
        let before: Vec<bool> = (1_000..1_064).map(|c| runs(Some(c), 3)).collect();
        let after: Vec<bool> = (1_000..1_064).map(|c| runs(Some(c), 3)).collect();
        assert_eq!(before, after);
        assert_eq!(before.iter().filter(|r| **r).count(), 21);
        assert!(
            runs(Some(u32::MAX - 15), 16),
            "u32::MAX - 15 = 16·268435455"
        );
    }

    #[test]
    fn nothing_installs_when_off_or_without_the_targets() {
        let empty = ResolvedProfile {
            name: "empty".into(),
            targets: Vec::new(),
            absent_optional: Vec::new(),
        };
        assert!(install_with(&empty, None).contains(EVERY_ENV));
        assert!(install_with(&empty, Some(4)).contains("the profile has no"));
    }

    static GRID_CALLS: AtomicU64 = AtomicU64::new(0);
    static EMITTER_CALLS: AtomicU64 = AtomicU64::new(0);
    static SEEN_DT: AtomicU32 = AtomicU32::new(0);

    unsafe extern "system" fn fake_grid(_this: usize, engine: usize, nodes: usize, dt: f32) {
        assert_eq!((engine, nodes), (0xE, 0x17));
        SEEN_DT.store(dt.to_bits(), Ordering::Release);
        GRID_CALLS.fetch_add(1, Ordering::AcqRel);
    }

    unsafe extern "system" fn fake_emitter(_this: usize, engine: usize, count: usize, dt: f32) {
        assert_eq!((engine, count as u32), (0xE, 3));
        assert_eq!(dt, 0.2);
        EMITTER_CALLS.fetch_add(1, Ordering::AcqRel);
    }

    /// A vtable and the two `lea`s naming it, in memory the test owns.
    #[repr(C, align(16))]
    struct Fake {
        grid_lea: [u8; 16],
        emitter_lea: [u8; 16],
        grid_vtable: [usize; 12],
        emitter_vtable: [usize; 13],
    }

    fn lea_to(lea: &mut [u8; 16], vtable: usize) {
        let at = lea.as_ptr() as usize;
        let disp = i32::try_from(vtable as i64 - (at as i64 + 7)).unwrap();
        lea[..3].copy_from_slice(&LEA_RAX_RIP);
        lea[3..7].copy_from_slice(&disp.to_le_bytes());
    }

    fn target(name: &str, address: usize) -> ResolvedTarget {
        ResolvedTarget {
            name: name.into(),
            address: address as u64,
            image_index: 0,
            required: false,
        }
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    #[test]
    fn the_slots_gate_both_systems_on_the_same_updates_and_detach() {
        // Its own region: the rewrites change its protection, and a page
        // of the test heap would change under other tests.
        let page = crate::bigmap::original::Page::with_len(0x10000);
        assert!(std::mem::size_of::<Fake>() <= 0x10000);
        // SAFETY: a fresh zeroed region, aligned to a page, only this test's.
        let fake = unsafe { &mut *(page.base() as *mut Fake) };
        fake.grid_vtable[GRID_SLOT] = fake_grid as *const () as usize;
        fake.emitter_vtable[EMITTER_SLOT] = fake_emitter as *const () as usize;
        let grid_vtable = fake.grid_vtable.as_ptr() as usize;
        let emitter_vtable = fake.emitter_vtable.as_ptr() as usize;
        lea_to(&mut fake.grid_lea, grid_vtable);
        lea_to(&mut fake.emitter_lea, emitter_vtable);
        let resolved = |grid_fn: usize| ResolvedProfile {
            name: "fake".into(),
            targets: vec![
                target(GRID_UPDATE, grid_fn),
                target(GRID_VTABLE, fake.grid_lea.as_ptr() as usize),
                target(EMITTER_UPDATE2, fake_emitter as *const () as usize),
                target(EMITTER_VTABLE, fake.emitter_lea.as_ptr() as usize),
            ],
            absent_optional: Vec::new(),
        };
        // A slot holding anything but the pinned function: nothing changes.
        let wrong = resolved(fake_emitter as *const () as usize);
        let refused = install_slots(&wrong, Some(4)).err().unwrap();
        assert!(refused.contains("holds"), "{refused}");
        assert_eq!(fake.grid_vtable[GRID_SLOT], fake_grid as *const () as usize);

        let slots = install_slots(&resolved(fake_grid as *const () as usize), Some(4)).unwrap();
        assert_eq!(fake.grid_vtable[GRID_SLOT], grid_gate as *const () as usize);
        assert_eq!(
            fake.emitter_vtable[EMITTER_SLOT],
            emitter_gate as *const () as usize
        );
        type Fn4 = unsafe extern "system" fn(usize, usize, usize, f32);
        // SAFETY: the gates, through the slots, as the engine calls them.
        let (grid, emitter) = unsafe {
            (
                std::mem::transmute::<usize, Fn4>(fake.grid_vtable[GRID_SLOT]),
                std::mem::transmute::<usize, Fn4>(fake.emitter_vtable[EMITTER_SLOT]),
            )
        };
        for count in 40..60 {
            FORCED_COUNT.store(count, Ordering::Release);
            // SAFETY: the gates call the fakes.
            unsafe {
                emitter(1, 0xE, 3, 0.2);
                grid(1, 0xE, 0x17, 0.2);
            }
        }
        assert_eq!(GRID_CALLS.load(Ordering::Acquire), 5, "40, 44, ..., 56");
        assert_eq!(EMITTER_CALLS.load(Ordering::Acquire), 5);
        assert_eq!(
            f32::from_bits(SEEN_DT.load(Ordering::Acquire)),
            0.2,
            "dt as passed"
        );
        FORCED_COUNT.store(-1, Ordering::Release);
        for slot in slots {
            // SAFETY: nothing calls through the fake slots now.
            unsafe { slot.detach() };
        }
        assert_eq!(fake.grid_vtable[GRID_SLOT], fake_grid as *const () as usize);
        assert_eq!(
            fake.emitter_vtable[EMITTER_SLOT],
            fake_emitter as *const () as usize
        );
        EVERY.store(0, Ordering::Release);
    }
}

/// The game's own `Diffuse` (`ecs::(anon)::Diffuse`, 0xaa8570), relocated
/// from the executable: why the throttle keeps the game's `dt`.
#[cfg(all(test, windows, target_arch = "x86_64"))]
#[allow(clippy::unwrap_used)]
mod original_tests {
    use crate::bigmap::original::{Exe, Page};

    const DIFFUSE_RVA: u64 = 0xaa8570;
    const DIFFUSE_LEN: usize = 1046;
    /// 5.0, 1.0, 4.0 and the 1e-15 bias it reads.
    const CONSTANTS: [u64; 4] = [0x3678cdc, 0x3676644, 0x3677b5c, 0x36fd270];
    const ASSERT_RVA: u64 = 0x303d380;
    /// The assert's strings: its function, file and the two conditions.
    const ASSERT_STRINGS: [u64; 4] = [0x36fcd40, 0x36fcb50, 0x36fcd88, 0x36fcd68];
    /// The game's step: `EmissionGridSystem::Update` asserts `dt >= 0.2`.
    const DT: f32 = 0.2;

    /// The game's `Grid<float>`: origin, size, then `std::vector<float>`.
    #[repr(C)]
    struct Grid {
        x0: i32,
        y0: i32,
        width: i32,
        height: i32,
        begin: *mut f32,
        end: *mut f32,
        capacity: *mut f32,
    }

    impl Grid {
        fn over(cells: &mut [f32], width: i32, height: i32) -> Self {
            assert_eq!(cells.len(), (width * height) as usize);
            let begin = cells.as_mut_ptr();
            Self {
                x0: -1,
                y0: -1,
                width,
                height,
                begin,
                end: begin.wrapping_add(cells.len()),
                capacity: begin.wrapping_add(cells.len()),
            }
        }
    }

    /// `Diffuse(hBegin, hEnd, src, dst, a, decay, dt)`: rows `hBegin..hEnd`
    /// of `dst` from `src`, `w1 = a·5·dt`, `w2 = (1 − 4a − decay)·5·dt`.
    type Diffuse = unsafe extern "system" fn(i32, i32, *const Grid, *mut Grid, f32, f32, f32);

    extern "C" fn fail() {
        std::process::abort();
    }

    fn relocated(exe: &Exe, page: &mut Page) -> Diffuse {
        let constants: Vec<(u64, usize)> = CONSTANTS
            .iter()
            .map(|&rva| (rva, page.data(exe.bytes(rva, 4))))
            .collect();
        let slot = page.data(&(fail as *const () as usize as u64).to_le_bytes());
        let text = page.data(b"assert ");
        let jump = page.code(&[0xFF, 0x25, 0, 0, 0, 0]);
        let rel = (slot as i64 - (jump as i64 + 6)) as i32;
        // SAFETY: the jump's displacement, in the page just written.
        unsafe {
            std::ptr::copy_nonoverlapping(rel.to_le_bytes().as_ptr(), (jump + 2) as *mut u8, 4)
        };
        let (at, _) = page.relocate(exe, DIFFUSE_RVA, DIFFUSE_LEN, &|target| {
            if target == ASSERT_RVA {
                return Some(jump);
            }
            if ASSERT_STRINGS.contains(&target) {
                return Some(text);
            }
            constants
                .iter()
                .find(|(rva, _)| *rva == target)
                .map(|(_, at)| *at)
        });
        // SAFETY: the relocated kernel, the game's own ABI.
        unsafe { std::mem::transmute::<usize, Diffuse>(at) }
    }

    /// The model world: a 66 x 66 grid (one border ring) with one emitter
    /// in the middle, splatted as the game's insert does (`max(c + e, 0)`)
    /// before each diffusion, with a neighbour weight and a decay that
    /// keep the game's `w2 >= 0`.
    struct World {
        diffuse: Diffuse,
        cells: Vec<f32>,
        temp: Vec<f32>,
    }

    const SIDE: i32 = 66;
    const A: f32 = 0.2;
    const DECAY: f32 = 0.02;

    impl World {
        fn new(diffuse: Diffuse) -> Self {
            let n = (SIDE * SIDE) as usize;
            Self {
                diffuse,
                cells: vec![0.0; n],
                temp: vec![0.0; n],
            }
        }

        /// One run of the grid's update with `dt`: `round(dt / 0.2)`
        /// diffusion steps, each weighted by `dt`, as
        /// `EmissionGridSystem::Update` does.
        fn update(&mut self, dt: f32, emit: f32) {
            let middle = (SIDE / 2 * SIDE + SIDE / 2) as usize;
            self.cells[middle] = (self.cells[middle] + emit).max(0.0);
            let steps = (dt / DT + 0.5).floor() as i32;
            for _ in 0..steps {
                let src = Grid::over(&mut self.cells, SIDE, SIDE);
                let mut dst = Grid::over(&mut self.temp, SIDE, SIDE);
                // SAFETY: two grids of SIDE² cells, rows 1..SIDE-1.
                unsafe { (self.diffuse)(1, SIDE - 1, &src, &mut dst, A, DECAY, dt) };
                std::mem::swap(&mut self.cells, &mut self.temp);
            }
        }

        fn mass(&self) -> f64 {
            self.cells.iter().map(|&c| f64::from(c)).sum()
        }

        fn bits(&self) -> Vec<u32> {
            self.cells.iter().map(|c| c.to_bits()).collect()
        }
    }

    #[test]
    fn scaling_dt_by_n_diverges_and_the_games_dt_does_not() {
        let Some(exe) = Exe::load() else { return };
        let mut page = Page::new();
        let diffuse = relocated(&exe, &mut page);
        let mut stock = World::new(diffuse);
        let mut scaled = World::new(diffuse);
        stock.update(DT, 1.0);
        scaled.update(DT * 4.0, 1.0);
        for _ in 0..40 {
            stock.update(DT, 0.0);
            scaled.update(DT * 4.0, 0.0);
        }
        // The game's dt: no step adds mass (weights sum to 1 − decay).
        assert!(stock.mass() <= 1.0, "{}", stock.mass());
        assert!(stock.cells.iter().all(|c| c.is_finite() && *c >= 0.0));
        // dt·4: four steps per update, each with weights summing to
        // 4·(1 − decay); 160 steps take the mass past any float.
        assert!(
            scaled.mass() > 1e30 || !scaled.mass().is_finite(),
            "{}",
            scaled.mass()
        );
    }

    #[test]
    fn throttled_with_the_games_dt_is_the_stock_trajectory_n_times_slower() {
        let Some(exe) = Exe::load() else { return };
        let mut page = Page::new();
        let diffuse = relocated(&exe, &mut page);
        let every = 4;
        let mut stock = World::new(diffuse);
        let mut throttled = World::new(diffuse);
        // The emitter runs and the grid diffuses on the same updates.
        for update in 0..300u32 {
            if update < 75 {
                stock.update(DT, 1.0);
            }
            if super::runs(Some(update), every) {
                throttled.update(DT, 1.0);
            }
        }
        assert_eq!(throttled.bits(), stock.bits(), "update 300 = stock's 75");
        // Both settle on the same field: the stock fixed point.
        for update in 300..20_000u32 {
            stock.update(DT, 1.0);
            if super::runs(Some(update), every) {
                throttled.update(DT, 1.0);
            }
        }
        let worst = stock
            .cells
            .iter()
            .zip(&throttled.cells)
            .map(|(a, b)| ((a - b) / a.max(1e-6)).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-5, "{worst}");
        assert!(stock.mass() > 1.0 && stock.mass().is_finite());
    }
}

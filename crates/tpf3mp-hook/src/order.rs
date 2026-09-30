//! The order fixes: where the engine's simulation depends on the order of a
//! container that is not saved state, made a pure function of the entity
//! set instead (docs/HOOKS.md, "The order fixes, as built";
//! investigation/TPF3_RNG_2026-09-29.md, "What must change for lockstep").
//!
//! Two replicas that run the same commands at the same steps still hold
//! their ECS node lists in different orders (a running world's is its
//! add/swap-remove history, a loaded world's its registration order), and
//! TPF2 Multiplayer found four places where the engine let that order decide
//! the simulation. TF3's counterparts, from the survey, each installed on
//! its own and each failing closed on its own:
//!
//! 1. **Land-vehicle reservation order** ([`land_vehicle`]): sorted by
//!    entity id before the engine's seeded shuffle. A fix.
//! 2. **Ship and aircraft claim order** ([`measure`]): measured through the
//!    reservation manager, as TPF2 did, before anything is changed.
//! 3. **Road edge entries** ([`measure`]): the append calls are measured;
//!    the fix waits for a clean site (the plan is in docs/HOOKS.md).
//! 4. **Vehicles at a stop** ([`terminal`]): sorted by entity id where the
//!    boarding loop reads them, TPF2's `vehstop`. A fix. The unload deques
//!    (TPF2's `unload`) are not located in TF3 yet.
//!
//! Every fix has the same shape: the profile must resolve its site, the
//! bytes there must be exactly what the fix expects (the resolver checks
//! them, and the splice checks them again), and every read the hook makes
//! on the game's thread goes through [`crate::image::readable`]. A shape it
//! does not recognise is refused for that step and said once in the log; a
//! panic switches the fix off for good. A fix never guesses.
//!
//! The measurement hooks install only with [`MEASURE_ENV`] set, so they
//! cost nothing otherwise.

#![allow(unsafe_code)]

use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

use tpf3mp_hookcore::detour::{InlineDetour, SavedRegs, Splice};
use tpf3mp_hookcore::profile::ResolvedProfile;

use crate::log;

/// Set to `1` (or any non-empty value) in the game's environment, the
/// measurement hooks install: the claim order at the reservation manager,
/// the road-edge appends and the two fixes' own before-sort orders are
/// hashed per simulation update and written to hook.log every 100 updates.
/// A number above 1 sets that interval. Two replicas' logs can then be
/// diffed line by line.
pub const MEASURE_ENV: &str = "TPF3MP_HOOK_MEASURE_ORDER";

/// What installing one fix came to, for hook.log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub fix: &'static str,
    pub installed: bool,
    pub reason: String,
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.installed {
            write!(f, "order fix {}: installed ({})", self.fix, self.reason)
        } else {
            write!(f, "order fix {}: off, {}", self.fix, self.reason)
        }
    }
}

/// Installs every order fix that resolves on this build, for the life of
/// the game, and says what each came to. Called once the step gate is in,
/// before any world exists (the quiescence rule in docs/HOOKS.md).
pub fn install(resolved: &ResolvedProfile) -> Vec<Outcome> {
    let measuring = measure::configure_from_env();
    let mut outcomes = vec![land_vehicle::install(resolved), terminal::install(resolved)];
    outcomes.extend(measure::install(resolved, measuring));
    outcomes
}

/// Reads a plain value from the game's memory, only if it is readable.
fn read<T: Copy>(address: u64) -> Option<T> {
    let address = usize::try_from(address).ok()?;
    if !crate::image::readable(address, std::mem::size_of::<T>()) {
        return None;
    }
    // SAFETY: `size_of::<T>()` bytes at `address` are committed, readable
    // memory, checked just above; the read is unaligned and by value.
    Some(unsafe { std::ptr::read_unaligned(address as *const T) })
}

/// Whether `len` bytes at `address` may be read.
fn readable(address: u64, len: usize) -> bool {
    usize::try_from(address).is_ok_and(|address| crate::image::readable(address, len))
}

/// Says a refusal once per reason (a fix refuses per step, the log is not
/// per step), and counts it.
struct Refusals {
    last: Mutex<Option<&'static str>>,
    count: AtomicU64,
}

impl Refusals {
    const fn new() -> Self {
        Self {
            last: Mutex::new(None),
            count: AtomicU64::new(0),
        }
    }

    fn note(&self, fix: &str, why: &'static str) {
        self.count.fetch_add(1, Ordering::Relaxed);
        let mut last = self.last.lock().unwrap_or_else(|p| p.into_inner());
        if *last != Some(why) {
            *last = Some(why);
            log::line(&format!(
                "order fix {fix}: refused this step, {why}; the engine's own order stands"
            ));
        }
    }
}

/// FNV-1a over bytes, 64-bit: the measurement's hash. Two replicas that
/// log the same value hashed the same sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fnv1a(pub u64);

impl Fnv1a {
    pub const fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100_0000_01b3);
        }
    }

    pub fn write_u32(&mut self, value: u32) {
        self.write(&value.to_le_bytes());
    }
}

impl Default for Fnv1a {
    fn default() -> Self {
        Self::new()
    }
}

/// Runs a hook body on the game's thread without letting a panic cross
/// into the engine: a panic switches the fix off for good and is said once.
fn guarded(fix: &str, broken: &AtomicBool, body: impl FnOnce()) {
    if broken.load(Ordering::Acquire) {
        return;
    }
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)).is_err() {
        broken.store(true, Ordering::Release);
        log::line(&format!(
            "order fix {fix}: panicked on the game's thread; switched off for this game"
        ));
    }
}

/// What a sort at a site came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sorted {
    /// Fewer than two entries, or already in order: nothing written.
    Unchanged,
    /// The entries were put in order.
    Reordered,
}

/// The land-vehicle reservation order (survey item 1): the site inside
/// `ecs::LandVehicleMoveSystem::Update2` where the vector of vehicles that
/// want track is complete and the engine is about to shuffle it with a
/// `minstd_rand` seeded from the game time, then stable-sort it by a
/// priority and reserve track in that order. The vector is built by walking
/// the family's node list, so its order is the list's; the shuffle's seed
/// is lockstep state, the order it permutes is not (TPF2's train-order bug).
/// Sorting the vector by the entity id of each entry's node before the
/// shuffle makes the shuffled order a pure function of the entity set and
/// the seed. The engine's seed, shuffle and priority sort stay as they are.
pub mod land_vehicle {
    use super::*;

    pub const FIX: &str = "land-vehicle-order";
    /// The site: `mov r13, [rbp-0x20]; mov rsi, [rbp-0x18]; cmp r13, rsi`.
    pub const SITE: &str = "ecs::LandVehicleMoveSystem::Update2/shuffle";
    /// The engine's own walk from an entry to its node record, which the
    /// hook copies: `this` at `[rbp-0x80]`, the node-list holder at
    /// `this+8`, the records at `[holder]`, 20 bytes each, the entry's
    /// node index at its first dword.
    pub const RECORDS: &str = "ecs::LandVehicleMoveSystem::Update2/records";
    /// The bytes at the site; the first [`STEAL`] are run from the stub.
    pub const EXPECTED: [u8; 11] = [
        0x4C, 0x8B, 0x6D, 0xE0, // mov r13, [rbp-0x20]
        0x48, 0x8B, 0x75, 0xE8, // mov rsi, [rbp-0x18]
        0x4C, 0x3B, 0xEE, // cmp r13, rsi
    ];
    pub const STEAL: usize = 8;
    /// Frame slots, as the stolen bytes encode them.
    const VEC_BEGIN: i64 = -0x20;
    const VEC_END: i64 = -0x18;
    /// Where `this` is, as the records walk encodes it (`mov r8, [rbp-0x80]`).
    const THIS: i64 = -0x80;
    /// One entry: `{int32 nodeIndex, float priority}`.
    pub const ENTRY_LEN: u64 = 8;
    /// One node record: the entity id, then four component indices.
    pub const RECORD_LEN: u64 = 20;
    /// A sanity bound on the vehicle count.
    pub const MAX_ENTRIES: u64 = 1 << 20;

    static BROKEN: AtomicBool = AtomicBool::new(false);
    static REFUSALS: Refusals = Refusals::new();
    static CALLS: AtomicU64 = AtomicU64::new(0);
    static REORDERS: AtomicU64 = AtomicU64::new(0);

    pub fn install(resolved: &ResolvedProfile) -> Outcome {
        let off = |reason: String| Outcome {
            fix: FIX,
            installed: false,
            reason,
        };
        let Some(site) = resolved.get(SITE) else {
            return off(format!("the profile has no {SITE:?}"));
        };
        let Some(records) = resolved.get(RECORDS) else {
            return off(format!("the profile has no {RECORDS:?}"));
        };
        // The records walk is the reservation loop's head, a few hundred
        // bytes after the shuffle in the same function.
        if records.address <= site.address || records.address - site.address > 0x1000 {
            return off(format!(
                "{RECORDS:?} at {:#x} is not just after {SITE:?} at {:#x}",
                records.address, site.address
            ));
        }
        // SAFETY: the site is inside a function the profile resolved and
        // prologue-checked, the hook installs before any world exists, so
        // no thread is in it; nothing branches into the stolen bytes (the
        // profile's note, from the disassembly); `hook` never unwinds
        // (`guarded`) and only rewrites the vector's entries in place.
        match unsafe { Splice::install(site.address as usize as *mut u8, &EXPECTED, STEAL, hook) } {
            Ok(splice) => {
                let _kept = std::mem::ManuallyDrop::new(splice);
                Outcome {
                    fix: FIX,
                    installed: true,
                    reason: format!(
                        "at {:#x}, the vehicles that want track are sorted by entity id before the engine's seeded shuffle",
                        site.address
                    ),
                }
            }
            Err(error) => off(format!("the site at {:#x}: {error}", site.address)),
        }
    }

    /// The hook the stub calls at the site, with the site's registers.
    pub(super) unsafe extern "system" fn hook(regs: *mut SavedRegs) {
        guarded(FIX, &BROKEN, || {
            // SAFETY: the stub hands the block it pushed on this thread's
            // stack and holds it until the hook returns.
            let regs = unsafe { &*regs };
            let n = CALLS.fetch_add(1, Ordering::Relaxed) + 1;
            match apply(regs.rbp) {
                Ok((sorted, before)) => {
                    if sorted == Sorted::Reordered {
                        let reorders = REORDERS.fetch_add(1, Ordering::Relaxed) + 1;
                        if reorders <= 3 {
                            log::line(&format!(
                                "order fix {FIX}: {} vehicles put in entity order before the shuffle (call #{n})",
                                before.len()
                            ));
                        }
                    }
                    measure::note_land_vehicles(&before, sorted);
                }
                Err(why) => REFUSALS.note(FIX, why),
            }
            if n == 1 || n.is_multiple_of(1 << 16) {
                log::line(&format!(
                    "order fix {FIX}: alive, calls={n} reordered={} refused={}",
                    REORDERS.load(Ordering::Relaxed),
                    REFUSALS.count.load(Ordering::Relaxed)
                ));
            }
        });
    }

    /// Reads the vector and the records through the frame, sorts the
    /// vector's entries by their node's entity id, and hands back the
    /// entity ids in the order the engine had them. Every read is checked;
    /// any shape but the measured one is a refusal with nothing written.
    fn apply(rbp: u64) -> Result<(Sorted, Vec<u32>), &'static str> {
        let slot = |offset: i64| rbp.checked_add_signed(offset);
        let begin: u64 = slot(VEC_BEGIN)
            .and_then(read)
            .ok_or("the frame's vector begin is unreadable")?;
        let end: u64 = slot(VEC_END)
            .and_then(read)
            .ok_or("the frame's vector end is unreadable")?;
        let this: u64 = slot(THIS)
            .and_then(read)
            .ok_or("the frame's this is unreadable")?;
        if end < begin || !(end - begin).is_multiple_of(ENTRY_LEN) {
            return Err("the vector's bounds are not whole entries");
        }
        let count = (end - begin) / ENTRY_LEN;
        if count > MAX_ENTRIES {
            return Err("more entries than any world holds");
        }
        if count < 2 {
            return Ok((Sorted::Unchanged, Vec::new()));
        }
        let holder: u64 = this
            .checked_add(8)
            .and_then(read)
            .ok_or("the node-list holder is unreadable")?;
        let records: u64 = read(holder).ok_or("the node records are unreadable")?;
        let records_end: u64 = holder
            .checked_add(8)
            .and_then(read)
            .ok_or("the node records' end is unreadable")?;
        if records_end < records || !(records_end - records).is_multiple_of(RECORD_LEN) {
            return Err("the node records are not whole records");
        }
        let record_count = (records_end - records) / RECORD_LEN;
        let vector_len =
            usize::try_from(count * ENTRY_LEN).map_err(|_| "the vector is too long")?;
        let records_len = usize::try_from(record_count * RECORD_LEN)
            .map_err(|_| "the node records are too long")?;
        if !readable(begin, vector_len) {
            return Err("the vector's entries are unreadable");
        }
        if !readable(records, records_len) {
            return Err("the node records are unreadable");
        }
        // SAFETY: `vector_len` readable bytes at `begin`, in whole 8-byte
        // entries; the copy is by value.
        let entries: Vec<u64> = (0..count)
            .map(|i| unsafe { std::ptr::read_unaligned((begin + i * ENTRY_LEN) as *const u64) })
            .collect();
        let key_of = |entry: u64| -> Option<u32> {
            let index = u64::from(entry as u32);
            if index >= record_count {
                return None;
            }
            // SAFETY: the record lies inside the readable records span.
            Some(unsafe { std::ptr::read_unaligned((records + index * RECORD_LEN) as *const u32) })
        };
        let (order, before) = canonical_order(&entries, key_of)?;
        let Some(order) = order else {
            return Ok((Sorted::Unchanged, before));
        };
        for (i, entry) in order.iter().enumerate() {
            // SAFETY: the same readable span the entries were read from;
            // the engine's own thread is the one writing it, in place, and
            // the engine reads the vector only after this site.
            unsafe {
                std::ptr::write_unaligned((begin + i as u64 * ENTRY_LEN) as *mut u64, *entry)
            };
        }
        Ok((Sorted::Reordered, before))
    }

    /// The entries sorted by their key (the entity id of the node an entry
    /// indexes), with the keys in the engine's order. `None` when the order
    /// already was that. Two entries with one key, or an entry whose index
    /// names no record, is a refusal: the sort would not be a total order
    /// of lockstep state.
    pub fn canonical_order(
        entries: &[u64],
        key_of: impl Fn(u64) -> Option<u32>,
    ) -> Result<(Option<Vec<u64>>, Vec<u32>), &'static str> {
        let mut keyed = Vec::with_capacity(entries.len());
        for entry in entries {
            let key = key_of(*entry).ok_or("an entry indexes no node record")?;
            keyed.push((key, *entry));
        }
        let before: Vec<u32> = keyed.iter().map(|(key, _)| *key).collect();
        if before.windows(2).all(|pair| pair[0] < pair[1]) {
            return Ok((None, before));
        }
        keyed.sort_unstable_by_key(|(key, _)| *key);
        if keyed.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err("two entries name one entity");
        }
        Ok((
            Some(keyed.into_iter().map(|(_, entry)| entry).collect()),
            before,
        ))
    }
}

/// The vehicles standing at a line stop (survey item 4, TPF2's `vehstop`):
/// `ecs::SimEntityAtTerminalSystem::Update` asks `TransportVehicleSystem`
/// for the vector of vehicles at each `(line, stopIndex)` and hands the
/// waiting cargo and people to them in the vector's order, one running
/// index shared by all of them. The vector is append order while the game
/// runs and load order after a load (its owner adds after a `std::find`
/// and erases in place, as TPF2's did), so two replicas load two trucks at
/// one stop differently. Sorted by entity id right after the lookup, where
/// the boarding loop reads it.
pub mod terminal {
    use super::*;

    pub const FIX: &str = "vehicles-at-stop-order";
    /// The site: `mov [rsp+0x248], rax` right after the getter's call, with
    /// `rax` the `std::vector<Entity>*`.
    pub const SITE: &str = "ecs::SimEntityAtTerminalSystem::Update/vehicles at stop";
    /// The getter the call before the site must reach.
    pub const GETTER: &str = "ecs::TransportVehicleSystem::GetVehiclesAtLineStop";
    pub const EXPECTED: [u8; 12] = [
        0x48, 0x89, 0x84, 0x24, 0x48, 0x02, 0x00, 0x00, // mov [rsp+0x248], rax
        0x41, 0x8B, 0x7F, 0x50, // mov edi, [r15+0x50]
    ];
    pub const STEAL: usize = 8;
    pub const MAX_IDS: u64 = 1 << 20;

    static BROKEN: AtomicBool = AtomicBool::new(false);
    static REFUSALS: Refusals = Refusals::new();
    static CALLS: AtomicU64 = AtomicU64::new(0);
    static REORDERS: AtomicU64 = AtomicU64::new(0);

    pub fn install(resolved: &ResolvedProfile) -> Outcome {
        let off = |reason: String| Outcome {
            fix: FIX,
            installed: false,
            reason,
        };
        let Some(site) = resolved.get(SITE) else {
            return off(format!("the profile has no {SITE:?}"));
        };
        let Some(getter) = resolved.get(GETTER) else {
            return off(format!("the profile has no {GETTER:?}"));
        };
        // The five bytes before the site must be a call of the getter: the
        // vector in rax is its answer and nothing else's.
        let call = site.address.wrapping_sub(5);
        let opcode: Option<u8> = read(call);
        let rel: Option<i32> = read(call + 1);
        match (opcode, rel) {
            (Some(0xE8), Some(rel)) => {
                let callee = site.address.wrapping_add_signed(i64::from(rel));
                if callee != getter.address {
                    return off(format!(
                        "the call before the site reaches {callee:#x}, not {GETTER:?} at {:#x}",
                        getter.address
                    ));
                }
            }
            _ => return off(format!("no call before the site at {:#x}", site.address)),
        }
        // SAFETY: as for the land-vehicle site: resolved, quiescent, nothing
        // branches into the stolen bytes, and the hook only sorts the ids of
        // the vector `rax` names, in place.
        match unsafe { Splice::install(site.address as usize as *mut u8, &EXPECTED, STEAL, hook) } {
            Ok(splice) => {
                let _kept = std::mem::ManuallyDrop::new(splice);
                Outcome {
                    fix: FIX,
                    installed: true,
                    reason: format!(
                        "at {:#x}, the vehicles at a line stop are sorted by entity id before the boarding loop",
                        site.address
                    ),
                }
            }
            Err(error) => off(format!("the site at {:#x}: {error}", site.address)),
        }
    }

    /// The hook the stub calls at the site, with the site's registers.
    pub(super) unsafe extern "system" fn hook(regs: *mut SavedRegs) {
        guarded(FIX, &BROKEN, || {
            // SAFETY: the stub's block, held until the hook returns.
            let regs = unsafe { &*regs };
            let n = CALLS.fetch_add(1, Ordering::Relaxed) + 1;
            match apply(regs.rax) {
                Ok((sorted, before)) => {
                    if sorted == Sorted::Reordered {
                        let reorders = REORDERS.fetch_add(1, Ordering::Relaxed) + 1;
                        if reorders <= 3 {
                            log::line(&format!(
                                "order fix {FIX}: {} vehicles at a stop put in entity order (call #{n})",
                                before.len()
                            ));
                        }
                    }
                    measure::note_vehicles_at_stop(&before, sorted);
                }
                Err(why) => REFUSALS.note(FIX, why),
            }
            if n == 1 || n.is_multiple_of(1 << 16) {
                log::line(&format!(
                    "order fix {FIX}: alive, calls={n} reordered={} refused={}",
                    REORDERS.load(Ordering::Relaxed),
                    REFUSALS.count.load(Ordering::Relaxed)
                ));
            }
        });
    }

    /// Sorts the `std::vector<Entity>` (begin at +0, end at +8) at `vector`
    /// in place, and hands back its ids in the order the engine had them.
    fn apply(vector: u64) -> Result<(Sorted, Vec<i32>), &'static str> {
        let begin: u64 = read(vector).ok_or("the vector is unreadable")?;
        let end: u64 = vector
            .checked_add(8)
            .and_then(read)
            .ok_or("the vector's end is unreadable")?;
        if end < begin || !(end - begin).is_multiple_of(4) {
            return Err("the vector's bounds are not whole ids");
        }
        let count = (end - begin) / 4;
        if count > MAX_IDS {
            return Err("more vehicles at one stop than any world holds");
        }
        if count < 2 {
            return Ok((Sorted::Unchanged, Vec::new()));
        }
        let len = usize::try_from(count * 4).map_err(|_| "the vector is too long")?;
        if !readable(begin, len) {
            return Err("the vector's ids are unreadable");
        }
        // SAFETY: `len` readable bytes at `begin`, whole 4-byte ids.
        let ids: Vec<i32> = (0..count)
            .map(|i| unsafe { std::ptr::read_unaligned((begin + i * 4) as *const i32) })
            .collect();
        let Some(sorted) = sorted_ids(&ids) else {
            return Ok((Sorted::Unchanged, ids));
        };
        for (i, id) in sorted.iter().enumerate() {
            // SAFETY: the same span, written in place on the engine's thread
            // before the engine reads it.
            unsafe { std::ptr::write_unaligned((begin + i as u64 * 4) as *mut i32, *id) };
        }
        Ok((Sorted::Reordered, ids))
    }

    /// The ids ascending, or `None` when they already are.
    pub fn sorted_ids(ids: &[i32]) -> Option<Vec<i32>> {
        if ids.windows(2).all(|pair| pair[0] <= pair[1]) {
            return None;
        }
        let mut sorted = ids.to_vec();
        sorted.sort_unstable();
        Some(sorted)
    }
}

/// The measurement (survey items 2 and 3, and the two fixes' own evidence):
/// with [`MEASURE_ENV`] set, whole-function detours on the reservation
/// manager's two `Reserve` overloads, the road-edge appenders and
/// `ecs::Engine::Update` hash, per simulation update, the sequence of
/// (reserver entity, edge) claims and of (edge, entity) appends, and
/// the two sort sites hash the order they found. Every `interval` updates
/// one line goes to hook.log with the update number and the hashes since
/// the last line; two replicas' lines can be diffed. Updates are numbered
/// from the room's step once the step driver says which it is
/// ([`measure::room_step`]); before that, from the hook's start.
pub mod measure {
    use super::*;

    pub const RESERVE: &str = "transport::EdgeReservationManager::Reserve";
    pub const RESERVE_SIMPLE: &str = "transport::EdgeReservationManager::Reserve_simple";
    pub const EDGE_USE_ADD: &str = "transport::EdgeUseManager::Add";
    pub const EDGE_USE_ADD_RANGE: &str = "transport::EdgeUseManager::AddRange";
    pub const ENGINE_UPDATE: &str = "ecs::Engine::Update";
    pub const DEFAULT_INTERVAL: u64 = 100;
    /// One `EdgeId` on a path: entity, index, direction.
    const EDGE_LEN: u64 = 12;
    /// A sanity bound on one reservation's edge count.
    const MAX_EDGES: u64 = 1 << 16;

    /// The hashes of one update, and how many items went into each.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Lanes {
        pub claims: Fnv1a,
        pub claim_count: u64,
        pub appends: Fnv1a,
        pub append_count: u64,
        pub land_vehicles: Fnv1a,
        pub land_vehicle_calls: u64,
        pub land_vehicle_reorders: u64,
        pub vehicles_at_stop: Fnv1a,
        pub vehicle_stop_calls: u64,
        pub vehicle_stop_reorders: u64,
    }

    impl Lanes {
        pub const fn new() -> Self {
            Self {
                claims: Fnv1a::new(),
                claim_count: 0,
                appends: Fnv1a::new(),
                append_count: 0,
                land_vehicles: Fnv1a::new(),
                land_vehicle_calls: 0,
                land_vehicle_reorders: 0,
                vehicles_at_stop: Fnv1a::new(),
                vehicle_stop_calls: 0,
                vehicle_stop_reorders: 0,
            }
        }

        /// One log line: the update it closes and every lane.
        pub fn line(&self, update: u64, interval: u64) -> String {
            format!(
                "order measure: updates {}..={update}: claims={:016x}/{} appends={:016x}/{} land={:016x}/{} reordered {} vehstop={:016x}/{} reordered {}",
                update.saturating_sub(interval.saturating_sub(1)),
                self.claims.0,
                self.claim_count,
                self.appends.0,
                self.append_count,
                self.land_vehicles.0,
                self.land_vehicle_calls,
                self.land_vehicle_reorders,
                self.vehicles_at_stop.0,
                self.vehicle_stop_calls,
                self.vehicle_stop_reorders,
            )
        }
    }

    impl Default for Lanes {
        fn default() -> Self {
            Self::new()
        }
    }

    static ENABLED: AtomicBool = AtomicBool::new(false);
    static INTERVAL: AtomicU64 = AtomicU64::new(DEFAULT_INTERVAL);
    /// The update the engine is in; `Engine::Update`'s detour advances it.
    static UPDATE: AtomicU64 = AtomicU64::new(0);
    static LANES: Mutex<Lanes> = Mutex::new(Lanes::new());
    static RESERVE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static RESERVE_SIMPLE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static ADD_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static ADD_RANGE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static UPDATE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

    /// Reads [`MEASURE_ENV`]; `true` when measuring.
    pub fn configure_from_env() -> bool {
        let (enabled, interval) = configure(std::env::var(MEASURE_ENV).ok().as_deref());
        ENABLED.store(enabled, Ordering::Release);
        INTERVAL.store(interval, Ordering::Release);
        enabled
    }

    /// Off when unset or empty; a number above 1 is the interval, anything
    /// else means on at the default interval.
    pub fn configure(value: Option<&str>) -> (bool, u64) {
        match value.map(str::trim) {
            None | Some("") => (false, DEFAULT_INTERVAL),
            Some(text) => match text.parse::<u64>() {
                Ok(n) if n > 1 => (true, n),
                _ => (true, DEFAULT_INTERVAL),
            },
        }
    }

    pub fn enabled() -> bool {
        ENABLED.load(Ordering::Acquire)
    }

    /// The step driver says the next update is the room's step `step`:
    /// from here the log's update numbers are the room's.
    pub fn room_step(step: u64) {
        UPDATE.store(step.saturating_sub(1), Ordering::Release);
    }

    fn lanes() -> std::sync::MutexGuard<'static, Lanes> {
        LANES.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub(super) fn note_land_vehicles(before: &[u32], sorted: Sorted) {
        if !enabled() {
            return;
        }
        let mut lanes = lanes();
        lanes.land_vehicle_calls += 1;
        lanes.land_vehicles.write_u32(before.len() as u32);
        for key in before {
            lanes.land_vehicles.write_u32(*key);
        }
        if sorted == Sorted::Reordered {
            lanes.land_vehicle_reorders += 1;
        }
    }

    pub(super) fn note_vehicles_at_stop(before: &[i32], sorted: Sorted) {
        if !enabled() {
            return;
        }
        let mut lanes = lanes();
        lanes.vehicle_stop_calls += 1;
        lanes.vehicles_at_stop.write_u32(before.len() as u32);
        for id in before {
            lanes.vehicles_at_stop.write_u32(*id as u32);
        }
        if sorted == Sorted::Reordered {
            lanes.vehicle_stop_reorders += 1;
        }
    }

    /// One claim: `Reserve(this, engine, typeIndex, entity, &path, from,
    /// to)` reserves `path[from..to)` for `entity`. Hashes the entity and
    /// each edge's twelve bytes; an unreadable path hashes as a marker.
    pub(super) fn note_claim(entity: u32, path: u64, from: i32, to: i32) {
        let mut lanes = lanes();
        lanes.claim_count += 1;
        lanes.claims.write_u32(entity);
        let edges = claim_edges(path, from, to);
        match edges {
            Some(edges) => {
                lanes.claims.write_u32(edges.len() as u32);
                for edge in edges {
                    lanes.claims.write(&edge);
                }
            }
            None => lanes.claims.write(b"?path"),
        }
    }

    /// The edges of `path[from..to)`, read through readable checks.
    fn claim_edges(path: u64, from: i32, to: i32) -> Option<Vec<[u8; EDGE_LEN as usize]>> {
        let (from, to) = (u64::try_from(from).ok()?, u64::try_from(to).ok()?);
        if to < from || to - from > MAX_EDGES {
            return None;
        }
        let base: u64 = read(path)?;
        let first = base.checked_add(from.checked_mul(EDGE_LEN)?)?;
        let len = usize::try_from((to - from) * EDGE_LEN).ok()?;
        if !readable(first, len) {
            return None;
        }
        Some(
            (0..to - from)
                // SAFETY: `len` readable bytes at `first`, whole edges.
                .map(|i| unsafe {
                    std::ptr::read_unaligned(
                        (first + i * EDGE_LEN) as *const [u8; EDGE_LEN as usize],
                    )
                })
                .collect(),
        )
    }

    /// One append at `EdgeUseManager::Add(this, &edgeId, entity, component,
    /// bounds)`: the edge id's twelve bytes, the entity, the component
    /// index and the bounds' bits.
    pub(super) fn note_add(edge_id: u64, entity: u32, component: u32, bounds: u64) {
        let mut lanes = lanes();
        lanes.append_count += 1;
        lanes.appends.write(b"add");
        match read::<[u8; EDGE_LEN as usize]>(edge_id) {
            Some(edge) => lanes.appends.write(&edge),
            None => lanes.appends.write(b"?edge"),
        }
        lanes.appends.write_u32(entity);
        lanes.appends.write_u32(component);
        lanes.appends.write(&bounds.to_le_bytes());
    }

    /// One `EdgeUseManager::AddRange(this, a2, a3, &edges, a5..a8)`: the
    /// raw integer arguments and the edges vector's bytes (begin at +0,
    /// end at +8), as they are; naming them is not needed for a diff.
    pub(super) fn note_add_range(args: [u32; 6], edges: u64) {
        let mut lanes = lanes();
        lanes.append_count += 1;
        lanes.appends.write(b"range");
        for arg in args {
            lanes.appends.write_u32(arg);
        }
        match range_bytes(edges) {
            Some(bytes) => {
                lanes.appends.write_u32(bytes.len() as u32);
                lanes.appends.write(&bytes);
            }
            None => lanes.appends.write(b"?edges"),
        }
    }

    fn range_bytes(edges: u64) -> Option<Vec<u8>> {
        let begin: u64 = read(edges)?;
        let end: u64 = edges.checked_add(8).and_then(read)?;
        if end < begin || end - begin > MAX_EDGES * EDGE_LEN {
            return None;
        }
        let len = usize::try_from(end - begin).ok()?;
        if !readable(begin, len) {
            return None;
        }
        // SAFETY: `len` readable bytes at `begin`.
        Some(unsafe { std::slice::from_raw_parts(begin as *const u8, len) }.to_vec())
    }

    /// The engine advances one update: the previous one's lanes close, and
    /// every `interval` updates they go to the log.
    pub(super) fn update_begins() {
        let closed = UPDATE.fetch_add(1, Ordering::AcqRel);
        let interval = INTERVAL.load(Ordering::Acquire).max(1);
        if closed == 0 || !closed.is_multiple_of(interval) {
            return;
        }
        let lanes = std::mem::take(&mut *lanes());
        log::line(&lanes.line(closed, interval));
    }

    type Fn8 =
        unsafe extern "system" fn(usize, usize, usize, usize, usize, usize, usize, usize) -> usize;
    /// `Engine::Update(engine, float dt)`: the second argument rides in
    /// `xmm1`, which a float parameter forwards.
    type UpdateFn = unsafe extern "system" fn(usize, f64, usize, usize) -> usize;

    #[allow(clippy::too_many_arguments)]
    unsafe extern "system" fn reserve(
        this: usize,
        engine: usize,
        type_index: usize,
        entity: usize,
        path: usize,
        from: usize,
        to: usize,
        s7: usize,
    ) -> usize {
        note_claim(
            entity as u32,
            path as u64,
            from as u32 as i32,
            to as u32 as i32,
        );
        let original = RESERVE_ORIGINAL.load(Ordering::Acquire);
        if original == 0 {
            return 0;
        }
        // SAFETY: the trampoline of the function this detour replaced, with
        // every argument forwarded (the three on the stack included).
        let original: Fn8 = unsafe { std::mem::transmute::<usize, Fn8>(original) };
        unsafe { original(this, engine, type_index, entity, path, from, to, s7) }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe extern "system" fn reserve_simple(
        this: usize,
        engine: usize,
        type_index: usize,
        entity: usize,
        path: usize,
        from: usize,
        to: usize,
        s7: usize,
    ) -> usize {
        note_claim(
            entity as u32,
            path as u64,
            from as u32 as i32,
            to as u32 as i32,
        );
        let original = RESERVE_SIMPLE_ORIGINAL.load(Ordering::Acquire);
        if original == 0 {
            return 0;
        }
        // SAFETY: as above.
        let original: Fn8 = unsafe { std::mem::transmute::<usize, Fn8>(original) };
        unsafe { original(this, engine, type_index, entity, path, from, to, s7) }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe extern "system" fn edge_use_add(
        this: usize,
        edge_id: usize,
        entity: usize,
        component: usize,
        bounds: usize,
        s5: usize,
        s6: usize,
        s7: usize,
    ) -> usize {
        note_add(
            edge_id as u64,
            entity as u32,
            component as u32,
            bounds as u64,
        );
        let original = ADD_ORIGINAL.load(Ordering::Acquire);
        if original == 0 {
            return 0;
        }
        // SAFETY: as above.
        let original: Fn8 = unsafe { std::mem::transmute::<usize, Fn8>(original) };
        unsafe { original(this, edge_id, entity, component, bounds, s5, s6, s7) }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe extern "system" fn edge_use_add_range(
        this: usize,
        a2: usize,
        a3: usize,
        edges: usize,
        a5: usize,
        a6: usize,
        a7: usize,
        a8: usize,
    ) -> usize {
        note_add_range(
            [
                a2 as u32, a3 as u32, a5 as u32, a6 as u32, a7 as u32, a8 as u32,
            ],
            edges as u64,
        );
        let original = ADD_RANGE_ORIGINAL.load(Ordering::Acquire);
        if original == 0 {
            return 0;
        }
        // SAFETY: as above.
        let original: Fn8 = unsafe { std::mem::transmute::<usize, Fn8>(original) };
        unsafe { original(this, a2, a3, edges, a5, a6, a7, a8) }
    }

    unsafe extern "system" fn engine_update(engine: usize, dt: f64, a3: usize, a4: usize) -> usize {
        update_begins();
        let original = UPDATE_ORIGINAL.load(Ordering::Acquire);
        if original == 0 {
            return 0;
        }
        // SAFETY: the trampoline of `Engine::Update`, with the engine and
        // its `dt` (in xmm1) forwarded.
        let original: UpdateFn = unsafe { std::mem::transmute::<usize, UpdateFn>(original) };
        unsafe { original(engine, dt, a3, a4) }
    }

    /// Installs the measurement detours when `measuring`; otherwise says
    /// they are off and how to turn them on.
    pub fn install(resolved: &ResolvedProfile, measuring: bool) -> Vec<Outcome> {
        const FIX: &str = "order-measure";
        if !measuring {
            return vec![Outcome {
                fix: FIX,
                installed: false,
                reason: format!("not measuring ({MEASURE_ENV} is not set); nothing is hooked"),
            }];
        }
        let interval = INTERVAL.load(Ordering::Acquire);
        let mut outcomes = Vec::new();
        let targets: [(&str, *const u8, &AtomicUsize); 5] = [
            (ENGINE_UPDATE, engine_update as *const u8, &UPDATE_ORIGINAL),
            (RESERVE, reserve as *const u8, &RESERVE_ORIGINAL),
            (
                RESERVE_SIMPLE,
                reserve_simple as *const u8,
                &RESERVE_SIMPLE_ORIGINAL,
            ),
            (EDGE_USE_ADD, edge_use_add as *const u8, &ADD_ORIGINAL),
            (
                EDGE_USE_ADD_RANGE,
                edge_use_add_range as *const u8,
                &ADD_RANGE_ORIGINAL,
            ),
        ];
        for (name, detour, original) in targets {
            let Some(target) = resolved.get(name) else {
                outcomes.push(Outcome {
                    fix: FIX,
                    installed: false,
                    reason: format!("the profile has no {name:?}"),
                });
                continue;
            };
            // SAFETY: a function the profile resolved and prologue-checked,
            // detoured before any world exists (no thread is in it); each
            // detour has the target's ABI with every argument forwarded.
            match unsafe { InlineDetour::install(target.address as usize as *mut u8, detour) } {
                Ok(installed) => {
                    original.store(installed.trampoline() as usize, Ordering::Release);
                    let _kept = std::mem::ManuallyDrop::new(installed);
                    outcomes.push(Outcome {
                        fix: FIX,
                        installed: true,
                        reason: format!(
                            "{name} at {:#x} is measured, every {interval} updates",
                            target.address
                        ),
                    });
                }
                Err(error) => outcomes.push(Outcome {
                    fix: FIX,
                    installed: false,
                    reason: format!("{name} at {:#x}: {error}", target.address),
                }),
            }
        }
        outcomes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_land_vehicle_sort_orders_entries_by_their_nodes_entity_id() {
        // Records: node 0 is entity 30, node 1 entity 10, node 2 entity 20.
        let entity_of = |node: u32| [30u32, 10, 20].get(node as usize).copied();
        let key_of = move |entry: u64| entity_of(entry as u32);
        let entry =
            |node: u32, priority: f32| u64::from(node) | (u64::from(priority.to_bits()) << 32);
        // The engine's order is node order; the priorities ride along.
        let entries = [entry(0, 1.5), entry(1, 2.5), entry(2, 0.5)];
        let (order, before) = land_vehicle::canonical_order(&entries, key_of).unwrap();
        assert_eq!(before, vec![30, 10, 20]);
        assert_eq!(
            order,
            Some(vec![entry(1, 2.5), entry(2, 0.5), entry(0, 1.5)]),
            "entity 10's entry first, each entry whole"
        );
        // Already in entity order: nothing to write.
        let sorted = [entry(1, 2.5), entry(2, 0.5), entry(0, 1.5)];
        assert_eq!(
            land_vehicle::canonical_order(&sorted, key_of).unwrap(),
            (None, vec![10, 20, 30])
        );
        // One entry: nothing to do.
        assert_eq!(
            land_vehicle::canonical_order(&entries[..1], key_of).unwrap(),
            (None, vec![30])
        );
    }

    #[test]
    fn the_land_vehicle_sort_refuses_what_is_not_a_total_order() {
        let key_of = |entry: u64| [30u32, 10, 10].get(entry as u32 as usize).copied();
        let duplicate = land_vehicle::canonical_order(&[0, 1, 2], key_of);
        assert_eq!(duplicate, Err("two entries name one entity"));
        let out_of_range = land_vehicle::canonical_order(&[0, 7], key_of);
        assert_eq!(out_of_range, Err("an entry indexes no node record"));
    }

    #[test]
    fn the_vehicles_at_a_stop_sort_by_id_and_leave_a_sorted_vector_alone() {
        assert_eq!(terminal::sorted_ids(&[7, 3, 5]), Some(vec![3, 5, 7]));
        assert_eq!(terminal::sorted_ids(&[3, 5, 7]), None);
        assert_eq!(terminal::sorted_ids(&[3, 3]), None);
        assert_eq!(terminal::sorted_ids(&[]), None);
        assert_eq!(terminal::sorted_ids(&[-2, -5]), Some(vec![-5, -2]));
    }

    #[test]
    fn the_site_bytes_are_what_the_profile_checks() {
        // The stolen bytes are the two loads of the vector's bounds; the
        // compare after them stays in place.
        assert_eq!(land_vehicle::STEAL, 8);
        assert_eq!(&land_vehicle::EXPECTED[..4], &[0x4C, 0x8B, 0x6D, 0xE0]);
        assert_eq!(&land_vehicle::EXPECTED[4..8], &[0x48, 0x8B, 0x75, 0xE8]);
        assert_eq!(terminal::STEAL, 8);
        // The profile's prologues for the two sites are these bytes.
        let profile =
            tpf3mp_hookcore::profile::Profile::from_toml(crate::BUILT_IN_PROFILES[0].1).unwrap();
        let prologue = |name: &str| {
            profile
                .targets
                .iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("{name} in the profile"))
                .prologue
                .clone()
        };
        assert_eq!(
            prologue(land_vehicle::SITE),
            land_vehicle::EXPECTED.to_vec()
        );
        assert_eq!(prologue(terminal::SITE), terminal::EXPECTED.to_vec());
        for name in [
            land_vehicle::RECORDS,
            terminal::GETTER,
            measure::RESERVE,
            measure::RESERVE_SIMPLE,
            measure::EDGE_USE_ADD,
            measure::EDGE_USE_ADD_RANGE,
            measure::ENGINE_UPDATE,
        ] {
            assert!(!prologue(name).is_empty(), "{name} in the profile");
        }
    }

    #[test]
    fn fnv1a_is_the_reference_hash() {
        // The FNV-1a 64-bit test vectors.
        let mut empty = Fnv1a::new();
        empty.write(b"");
        assert_eq!(empty.0, 0xcbf2_9ce4_8422_2325);
        let mut a = Fnv1a::new();
        a.write(b"a");
        assert_eq!(a.0, 0xaf63_dc4c_8601_ec8c);
        let mut foobar = Fnv1a::new();
        foobar.write(b"foobar");
        assert_eq!(foobar.0, 0x8594_4171_f739_67e8);
    }

    #[test]
    fn measuring_is_off_unless_the_environment_says_so() {
        assert_eq!(measure::configure(None), (false, 100));
        assert_eq!(measure::configure(Some("")), (false, 100));
        assert_eq!(measure::configure(Some("1")), (true, 100));
        assert_eq!(measure::configure(Some("yes")), (true, 100));
        assert_eq!(measure::configure(Some("250")), (true, 250));
        assert_eq!(measure::configure(Some(" 25 ")), (true, 25));
    }

    #[test]
    fn a_measurement_line_names_the_updates_and_every_lane() {
        let mut lanes = measure::Lanes::new();
        lanes.claim_count = 3;
        lanes.claims.write(b"x");
        let line = lanes.line(300, 100);
        assert!(line.starts_with("order measure: updates 201..=300: claims="));
        assert!(line.contains("/3 appends="));
        assert!(line.contains("land="));
        assert!(line.contains("vehstop="));
    }

    #[test]
    fn an_outcome_reads_as_a_log_line() {
        let on = Outcome {
            fix: "x",
            installed: true,
            reason: "because".into(),
        };
        assert_eq!(on.to_string(), "order fix x: installed (because)");
        let off = Outcome {
            fix: "x",
            installed: false,
            reason: "the profile has no site".into(),
        };
        assert_eq!(off.to_string(), "order fix x: off, the profile has no site");
    }

    #[test]
    fn nothing_installs_without_the_sites() {
        let resolved = ResolvedProfile {
            name: "empty".into(),
            targets: Vec::new(),
            absent_optional: Vec::new(),
        };
        let fixes = [
            land_vehicle::install(&resolved),
            terminal::install(&resolved),
        ];
        for outcome in &fixes {
            assert!(!outcome.installed, "{outcome}");
            assert!(outcome.reason.contains("the profile has no"), "{outcome}");
        }
        let off = measure::install(&resolved, false);
        assert_eq!(off.len(), 1);
        assert!(!off[0].installed);
        assert!(off[0].reason.contains(MEASURE_ENV));
        let on = measure::install(&resolved, true);
        assert_eq!(on.len(), 5, "one outcome per measured function");
        assert!(
            on.iter()
                .all(|o| !o.installed && o.reason.contains("the profile has no"))
        );
    }
}

/// The two sort hooks through their real stolen bytes: a hand-written
/// function stands where the engine's site would be, with the frame or
/// register the site reads pointing at a world built in memory.
#[cfg(all(test, windows, target_arch = "x86_64"))]
mod splice_tests {
    use super::*;

    /// A page of this test's own code, executable: the engine's buffers are
    /// private to hookcore.
    fn fixture(code: &[u8]) -> usize {
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc,
        };
        assert!(code.len() <= 0x1000);
        // SAFETY: a fresh read-write-execute page for the test's own code.
        let page = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                0x1000,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_EXECUTE_READWRITE,
            )
        };
        assert!(!page.is_null());
        // SAFETY: `code.len()` bytes into a 4 KiB writable page.
        unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), page.cast::<u8>(), code.len()) };
        page as usize
    }

    /// A node list of three records (entity ids 30, 10, 20) and a vector of
    /// three entries in node order, inside one frame laid out as the site's:
    /// `[rbp-0x20]` begin, `[rbp-0x18]` end, `[rbp-0x80]` this, `this+8`
    /// the holder, `[holder]`/`[holder+8]` the records' span.
    struct World {
        memory: Vec<u8>,
    }

    impl World {
        const RBP: usize = 0x200;
        const THIS: usize = 0x300;
        const HOLDER: usize = 0x340;
        const RECORDS: usize = 0x400;
        const VECTOR: usize = 0x500;

        fn new() -> Self {
            let mut memory = vec![0u8; 0x600];
            let base = memory.as_ptr() as u64;
            let put = |memory: &mut Vec<u8>, at: usize, value: u64| {
                memory[at..at + 8].copy_from_slice(&value.to_le_bytes());
            };
            for (node, entity) in [30u32, 10, 20].into_iter().enumerate() {
                let at = Self::RECORDS + node * 20;
                memory[at..at + 4].copy_from_slice(&entity.to_le_bytes());
            }
            for node in 0..3u32 {
                let at = Self::VECTOR + node as usize * 8;
                memory[at..at + 4].copy_from_slice(&node.to_le_bytes());
                memory[at + 4..at + 8].copy_from_slice(&(node as f32).to_bits().to_le_bytes());
            }
            put(&mut memory, Self::HOLDER, base + Self::RECORDS as u64);
            put(
                &mut memory,
                Self::HOLDER + 8,
                base + Self::RECORDS as u64 + 60,
            );
            put(&mut memory, Self::THIS + 8, base + Self::HOLDER as u64);
            put(&mut memory, Self::RBP - 0x20, base + Self::VECTOR as u64);
            put(
                &mut memory,
                Self::RBP - 0x18,
                base + Self::VECTOR as u64 + 24,
            );
            put(&mut memory, Self::RBP - 0x80, base + Self::THIS as u64);
            Self { memory }
        }

        fn rbp(&self) -> u64 {
            self.memory.as_ptr() as u64 + Self::RBP as u64
        }

        fn vector(&self) -> Vec<(u32, f32)> {
            (0..3)
                .map(|i| {
                    let at = Self::VECTOR + i * 8;
                    let dword = |at: usize| {
                        let m = &self.memory;
                        u32::from_le_bytes([m[at], m[at + 1], m[at + 2], m[at + 3]])
                    };
                    (dword(at), f32::from_bits(dword(at + 4)))
                })
                .collect()
        }
    }

    #[test]
    fn the_land_vehicle_hook_sorts_the_engines_vector_through_its_real_site() {
        let world = World::new();
        assert_eq!(world.vector(), vec![(0, 0.0), (1, 1.0), (2, 2.0)]);
        // push rbp; push r13; push rsi; mov rbp, imm64; <site>; mov rax,
        // [r13]; pop rsi; pop r13; pop rbp; ret
        let mut code = vec![0x55, 0x41, 0x55, 0x56, 0x48, 0xBD];
        code.extend_from_slice(&world.rbp().to_le_bytes());
        let site_at = code.len();
        code.extend_from_slice(&land_vehicle::EXPECTED);
        code.extend_from_slice(&[0x49, 0x8B, 0x45, 0x00, 0x5E, 0x41, 0x5D, 0x5D, 0xC3]);
        let page = fixture(&code);
        // SAFETY: the page holds our hand-written function of no arguments.
        let fun: extern "C" fn() -> u64 =
            unsafe { std::mem::transmute::<usize, extern "C" fn() -> u64>(page) };
        assert_eq!(fun() as u32, 0, "without the hook, node 0 is first");

        // SAFETY: the fixture is this test's own code, not running now, and
        // the real hook only rewrites the world's vector.
        let splice = unsafe {
            Splice::install(
                (page + site_at) as *mut u8,
                &land_vehicle::EXPECTED,
                land_vehicle::STEAL,
                land_vehicle::hook,
            )
        }
        .unwrap();
        let first = fun();
        assert_eq!(
            world.vector(),
            vec![(1, 1.0), (2, 2.0), (0, 0.0)],
            "entity 10's node first, entries whole"
        );
        assert_eq!(first as u32, 1, "the stolen loads ran after the sort");
        assert_eq!(f32::from_bits((first >> 32) as u32), 1.0);
        // A second pass finds the vector in order and leaves it.
        assert_eq!(fun() as u32, 1);
        assert_eq!(world.vector(), vec![(1, 1.0), (2, 2.0), (0, 0.0)]);
        // SAFETY: nothing runs the fixture now.
        unsafe { splice.detach() }.unwrap();
    }

    #[test]
    fn the_vehicles_at_stop_hook_sorts_the_vector_rax_names_through_its_real_site() {
        let mut ids: Vec<i32> = vec![9, 4, 6, 1];
        // A std::vector<Entity> of the first three ids: begin, end, cap.
        let vector: [u64; 3] = [
            ids.as_mut_ptr() as u64,
            ids.as_mut_ptr() as u64 + 12,
            ids.as_mut_ptr() as u64 + 16,
        ];
        // sub rsp, 0x260; mov rax, imm64; <site>; add rsp, 0x260; ret
        let mut code = vec![0x48, 0x81, 0xEC, 0x60, 0x02, 0x00, 0x00, 0x48, 0xB8];
        code.extend_from_slice(&(vector.as_ptr() as u64).to_le_bytes());
        let site_at = code.len();
        code.extend_from_slice(&terminal::EXPECTED[..terminal::STEAL]);
        code.extend_from_slice(&[0x48, 0x81, 0xC4, 0x60, 0x02, 0x00, 0x00, 0xC3]);
        let page = fixture(&code);
        // SAFETY: as above.
        let fun: extern "C" fn() -> u64 =
            unsafe { std::mem::transmute::<usize, extern "C" fn() -> u64>(page) };
        assert_eq!(fun(), vector.as_ptr() as u64);
        assert_eq!(ids, vec![9, 4, 6, 1]);
        // The site's expected bytes go on past the steal; the fixture holds
        // only the stolen ones, so it states those as its expectation.
        let expected = &terminal::EXPECTED[..terminal::STEAL];
        // SAFETY: as above.
        let splice = unsafe {
            Splice::install(
                (page + site_at) as *mut u8,
                expected,
                terminal::STEAL,
                terminal::hook,
            )
        }
        .unwrap();
        assert_eq!(
            fun(),
            vector.as_ptr() as u64,
            "rax reaches the stolen store"
        );
        assert_eq!(
            ids,
            vec![4, 6, 9, 1],
            "the three ids in the vector sorted, the fourth untouched"
        );
        // SAFETY: nothing runs the fixture now.
        unsafe { splice.detach() }.unwrap();
    }
}

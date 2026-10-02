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
//!    entity id before the engine's seeded shuffle. A fix. The shuffle's
//!    seed is the game's `tickCount`, which [`crate::ticks`] keeps equal;
//!    a line sampled by the seed's value logs both for two games to diff.
//! 2. **Ship and aircraft claim order** ([`measure`]): measured through the
//!    reservation manager, as TPF2 did, before anything is changed.
//! 3. **Road edge entries** ([`road`]): each edge's entries kept in entity
//!    order after every append (every other writer keeps order). A fix.
//! 4. **Vehicles at a stop** ([`terminal`]): sorted by entity id where the
//!    boarding loop reads them, TPF2's `vehstop`. A fix. The unload deques
//!    (TPF2's `unload`) are not located in TF3 yet.
//! 5. **Platform choice** ([`platform`]): the vehicles asked for a free
//!    platform in entity order, and the candidate terminals in one order
//!    before their cost sort. A fix.
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
// Elsewhere the fixes are not installed, so their code is unused there.
#![cfg_attr(not(all(windows, target_arch = "x86_64")), allow(dead_code))]

use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

use tpf3mp_hookcore::detour::{InlineDetour, SavedRegs, Splice};
use tpf3mp_hookcore::profile::ResolvedProfile;

use crate::image::Readable as Probe;
use crate::log;
use crate::perf::{self, Piece};

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
    let wanted = |env: &str| crate::ticks::wanted(std::env::var(env).ok().as_deref());
    let mut outcomes = vec![
        land_vehicle::install(resolved, wanted(land_vehicle::TOGGLE_ENV)),
        terminal::install(resolved, wanted(terminal::TOGGLE_ENV)),
    ];
    outcomes.extend(platform::install(resolved, wanted(platform::TOGGLE_ENV)));
    outcomes.push(decision_sync::install(
        resolved,
        crate::step::alternate_wanted(std::env::var(decision_sync::TOGGLE_ENV).ok().as_deref()),
    ));
    outcomes.extend(road::install(resolved, wanted(road::TOGGLE_ENV), measuring));
    outcomes.push(path_ties::install(resolved, wanted(path_ties::TOGGLE_ENV)));
    // The claim loop's watcher rides on the vehicle watcher's switch.
    if wanted(platform::WATCH_ENV) {
        outcomes.extend(claims::install(resolved));
        outcomes.extend(nodes::install(resolved));
    }
    outcomes.extend(measure::install(resolved, measuring));
    outcomes
}

thread_local! {
    /// Set on this thread while it runs the game's own step
    /// ([`set_in_step`]).
    static IN_STEP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// From the step detour, around its call of the game's `GameSim::Step`:
/// this thread is inside the simulation's step. The road fix counts the
/// appends it sees there apart from all others: the second engine's copy
/// (`GameState::Replicate` `0x255de0` -> `ecs::Engine::Replicate`
/// `0x2bb78f0` -> `Replicator::Apply` `0x2bb4430`, whose end of
/// modification calls the systems' `EntityAdded` in that engine) runs from
/// the game's frame, outside the step, as often as the frames come, and
/// any append on a worker thread inside the step would show up there too.
/// The appends inside the step are the simulation's own, in one order in
/// every game, so their counts are what two games' logs must agree on.
pub fn set_in_step(inside: bool) {
    IN_STEP.with(|flag| flag.set(inside));
}

/// Whether this thread is inside the game's step.
pub fn in_step() -> bool {
    IN_STEP.with(|flag| flag.get())
}

/// Reads a plain value from the game's memory, only if it is readable (the
/// check through the per-thread region cache, [`crate::image::Readable`]).
fn read<T: Copy>(address: u64) -> Option<T> {
    Probe::new().read(address)
}

/// Whether `len` bytes at `address` may be read.
fn readable(address: u64, len: usize) -> bool {
    usize::try_from(address).is_ok_and(|address| crate::image::readable_cached(address, len))
}

/// Says a refusal once per reason (a fix refuses per step, the log is not
/// per step), and counts it, in all and by reason since the last
/// [`Refusals::take_window`] (the `perf:` line's).
struct Refusals {
    state: Mutex<RefusalState>,
    count: AtomicU64,
}

struct RefusalState {
    last: Option<&'static str>,
    /// Refusals by reason since the window began; one entry per reason, so
    /// as short as the fix's list of reasons.
    window: Vec<(&'static str, u64)>,
}

impl Refusals {
    const fn new() -> Self {
        Self {
            state: Mutex::new(RefusalState {
                last: None,
                window: Vec::new(),
            }),
            count: AtomicU64::new(0),
        }
    }

    fn note(&self, fix: &str, why: &'static str) {
        self.count.fetch_add(1, Ordering::Relaxed);
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        match state.window.iter_mut().find(|(reason, _)| *reason == why) {
            Some((_, n)) => *n += 1,
            None => state.window.push((why, 1)),
        }
        if state.last != Some(why) {
            state.last = Some(why);
            log::line(&format!(
                "order fix {fix}: refused this step, {why}; the engine's own order stands"
            ));
        }
    }

    /// The refusals by reason since the last take.
    fn take_window(&self) -> Vec<(&'static str, u64)> {
        std::mem::take(&mut self.state.lock().unwrap_or_else(|p| p.into_inner()).window)
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
    /// Set to `0` (or `off`), the site stays out: the engine shuffles the
    /// vehicles in its own order.
    pub const TOGGLE_ENV: &str = "TPF3MP_HOOK_LAND_VEHICLE_ORDER";
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

    pub fn install(resolved: &ResolvedProfile, wanted: bool) -> Outcome {
        let off = |reason: String| Outcome {
            fix: FIX,
            installed: false,
            reason,
        };
        if !wanted {
            return off(format!(
                "{TOGGLE_ENV} says so; the engine shuffles in its own order"
            ));
        }
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
        let _timer = perf::time(Piece::LandVehicle);
        guarded(FIX, &BROKEN, || {
            // SAFETY: the stub hands the block it pushed on this thread's
            // stack and holds it until the hook returns.
            let regs = unsafe { &*regs };
            let n = CALLS.fetch_add(1, Ordering::Relaxed) + 1;
            // The site follows the seed's fix-up (`cmove r8d, r12d`) and the
            // mask loop, which leaves r8 alone: r8d is the shuffle's seed.
            let seed = regs.r8 as u32;
            match apply(regs.rbp) {
                Ok((sorted, before)) => {
                    if sorted == Sorted::Reordered {
                        let reorders = REORDERS.fetch_add(1, Ordering::Relaxed) + 1;
                        if reorders <= 3 {
                            log::line(&format!(
                                "order fix {FIX}: {} vehicles put in entity order before the shuffle (call #{n}, seed {seed})",
                                before.len()
                            ));
                        }
                    }
                    if let Some(line) = sample_line(seed, &before) {
                        log::line(&line);
                    }
                    measure::note_land_vehicles(&before, sorted, seed);
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
        if measure::enabled() {
            // SAFETY: the readable records span, whole 20-byte records; the
            // entity id is each record's first dword.
            let order = (0..record_count).map(|i| unsafe {
                std::ptr::read_unaligned((records + i * RECORD_LEN) as *const u32)
            });
            measure::note_land_nodes(order);
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

    /// One seed value in this many gets a [`sample_line`].
    pub const SAMPLE: u32 = 256;

    /// For one seed value in [`SAMPLE`] (the seed is the game's tickCount,
    /// so about one update in 256): the seed, how many vehicles want track
    /// (0 when fewer than two) and a hash of their entity ids in the order
    /// the engine shuffles them. Sampled by the seed's value, not by a
    /// count of calls, so two games that agree write the same lines and two
    /// logs can be diffed; the engine's shuffle and priority sort are a
    /// function of exactly these, so equal lines mean an equal claim order.
    pub fn sample_line(seed: u32, before: &[u32]) -> Option<String> {
        if !seed.is_multiple_of(SAMPLE) {
            return None;
        }
        let mut ids = before.to_vec();
        ids.sort_unstable();
        let mut hash = Fnv1a::new();
        for id in &ids {
            hash.write_u32(*id);
        }
        Some(format!(
            "order fix {FIX}: sample seed={seed} n={} ids={:016x}",
            ids.len(),
            hash.0
        ))
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
    /// Set to `0` (or `off`), the site stays out: the boarding loop reads
    /// the vehicles in the engine's order.
    pub const TOGGLE_ENV: &str = "TPF3MP_HOOK_VEHICLES_AT_STOP_ORDER";
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

    pub fn install(resolved: &ResolvedProfile, wanted: bool) -> Outcome {
        let off = |reason: String| Outcome {
            fix: FIX,
            installed: false,
            reason,
        };
        if !wanted {
            return off(format!(
                "{TOGGLE_ENV} says so; the boarding loop reads the engine's order"
            ));
        }
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
        let _timer = perf::time(Piece::VehiclesAtStop);
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

/// Sorts `keys` ascending and reports what that came to: `Ok(None)` when
/// they already were strictly ascending, the permutation otherwise, a
/// refusal when two share a key (not a total order of lockstep state).
fn canonical_permutation<K: Ord + Copy>(keys: &[K]) -> Result<Option<Vec<usize>>, &'static str> {
    if keys.windows(2).all(|pair| pair[0] < pair[1]) {
        return Ok(None);
    }
    let mut order: Vec<usize> = (0..keys.len()).collect();
    order.sort_by_key(|&i| keys[i]);
    if order.windows(2).any(|pair| keys[pair[0]] == keys[pair[1]]) {
        return Err("two entries name one entity");
    }
    Ok(Some(order))
}

/// The platform choice (investigation/TPF3_TRAIN_PRIORITY_2026-09-30.md,
/// "Platforms"). `ecs::TransportVehicleSystem::Update2` (`0xb8bae0`) walks
/// its node list (8-byte records `{entity, TransportVehicle index}` at
/// `[[this+8]]`, as many as its `int` argument says) and asks
/// `FindNextFreeTerminal` (`0xb84e20`) for each en-route vehicle, and a
/// choice is stored for the rest of the update, so a vehicle visited later
/// sees what an earlier one took. Two sites, each on its own:
///
/// - **visit**: at the loop head, after the engine loads the list's begin
///   into `rdi`, the hook points `rdi` at a copy of the list sorted by
///   entity id, built at the loop's first iteration. The loop only reads
///   `[rdi+rsi]` and `[rdi+rsi+4]` and reloads `rdi` every iteration, and
///   `rdi` is set anew after the loop, so the engine's list is never
///   written: it visits the same vehicles, in entity order.
/// - **candidates**: right before `FindNextFreeTerminal` `std::sort`s its
///   candidate terminals by cost (`0xb85453`; the comparator looks each
///   cost up in a map and compares floats only, so equal costs keep an
///   introsort order that depends on the input's), the 12-byte candidates
///   are put in one canonical order, so equal costs break the same way in
///   every game.
pub mod platform {
    use std::cell::RefCell;

    use super::*;

    pub const FIX: &str = "platform-order";
    /// Set to `0` (or `off`), both sites stay out.
    pub const TOGGLE_ENV: &str = "TPF3MP_HOOK_PLATFORM_ORDER";
    /// `mov rcx,[r13+0x10]; movsxd rax,[rsi+rdi+4]; imul rbx,rax,0x1e8`,
    /// right after `mov rax,[r13+8]; mov rdi,[rax]`.
    pub const VISIT_SITE: &str = "ecs::TransportVehicleSystem::Update2/visit";
    pub const VISIT_EXPECTED: [u8; 16] = [
        0x49, 0x8B, 0x4D, 0x10, // mov rcx, [r13+0x10]
        0x48, 0x63, 0x44, 0x3E, 0x04, // movsxd rax, [rsi+rdi+4]
        0x48, 0x69, 0xD8, 0xE8, 0x01, 0x00, 0x00, // imul rbx, rax, 0x1e8
    ];
    pub const VISIT_STEAL: usize = 9;
    /// `mov rcx,r14; sub rcx,r13; mov rax,rdi; imul rcx`: the candidates
    /// are `[r13, r14)`, the sort's call follows.
    pub const CANDIDATES_SITE: &str = "FindNextFreeTerminal/candidate sort";
    pub const CANDIDATES_EXPECTED: [u8; 12] = [
        0x49, 0x8B, 0xCE, // mov rcx, r14
        0x49, 0x2B, 0xCD, // sub rcx, r13
        0x48, 0x8B, 0xC7, // mov rax, rdi
        0x48, 0xF7, 0xE9, // imul rcx
    ];
    pub const CANDIDATES_STEAL: usize = 6;
    /// The free check, for the vehicle watcher only (it changes nothing):
    /// right after `FindNextFreeTerminal` asks
    /// `transport::EdgeReservationManager` (`0x255bce0`, `this` at
    /// `[rsp+0x38]`) who holds one of a candidate terminal's edges (the
    /// `EdgeId` at `r15`), `mov eax,[rsp+0x78]; cmp eax,edi` (6 bytes,
    /// stolen; the call's return address, never a branch target) reads the
    /// answer and compares it with the vehicle (`edi`, `[rbp+0xe0]`). The
    /// answer is the edge's reservation holder (the manager's map at
    /// `[this+8]`) or else the nearest vehicle on it in
    /// `transport::EdgeUseManager` (`[this]`, `0x255f340`), or -1: free.
    pub const OCCUPANT_SITE: &str = "FindNextFreeTerminal/occupant check";
    pub const OCCUPANT_EXPECTED: [u8; 8] = [
        0x8B, 0x44, 0x24, 0x78, // mov eax, [rsp+0x78]
        0x3B, 0xC7, // cmp eax, edi
        0x74, 0x09, // je +9
    ];
    pub const OCCUPANT_STEAL: usize = 6;
    /// The decision flag's read, for the vehicle watcher only (it changes
    /// nothing): for a land vehicle (carriers other than 3 and 4) the loop
    /// gets its `MovePath` (`0x52bbc0`, `rax`), and asks
    /// `FindNextFreeTerminal` only when `[rax+0x70]` is set:
    /// `movzx eax, byte ptr [rax+0x70]; test al,al` (6 bytes, stolen;
    /// fallthrough only), then `jmp`.
    pub const DECISION_SITE: &str = "ecs::TransportVehicleSystem::Update2/decision flag";
    pub const DECISION_EXPECTED: [u8; 6] = [
        0x0F, 0xB6, 0x40, 0x70, // movzx eax, byte ptr [rax+0x70]
        0x84, 0xC0, // test al, al
    ];
    pub const DECISION_STEAL: usize = 6;
    /// The candidate being checked: its first two words at `[rsp+0x40]`
    /// (the station in the high one), its terminal at `[rsp+0x60]`; the
    /// answer at `[rsp+0x78]`, the reservation manager at `[rsp+0x38]`.
    const CHECKED: u64 = 0x40;
    const CHECKED_TERMINAL: u64 = 0x60;
    const ANSWER: u64 = 0x78;
    const RESERVATIONS: u64 = 0x38;
    /// `FindNextFreeTerminal`'s vehicle entity, its stack argument at
    /// `[rbp+0xe0]` (read into `edi` at `0xb855e2`).
    const VEHICLE_ARG: u64 = 0xe0;
    /// Most entries of one edge a check line lists.
    const MAX_LISTED: u64 = 16;
    /// Update2's `int` argument, the node count, spilled at `[rbp+0x5b0]`.
    const COUNT: i64 = 0x5b0;
    pub const RECORD_LEN: u64 = 8;
    pub const CANDIDATE_LEN: u64 = 12;
    pub const MAX_RECORDS: u64 = 1 << 20;
    pub const MAX_CANDIDATES: u64 = 1 << 12;

    static VISIT_BROKEN: AtomicBool = AtomicBool::new(false);
    static VISIT_REFUSALS: Refusals = Refusals::new();
    static VISIT_CALLS: AtomicU64 = AtomicU64::new(0);
    static VISIT_REORDERS: AtomicU64 = AtomicU64::new(0);
    static CANDIDATE_BROKEN: AtomicBool = AtomicBool::new(false);
    static CANDIDATE_REFUSALS: Refusals = Refusals::new();
    static CANDIDATE_CALLS: AtomicU64 = AtomicU64::new(0);
    static CANDIDATE_REORDERS: AtomicU64 = AtomicU64::new(0);

    /// The loop being walked on this thread: the engine's list, and the
    /// sorted copy `rdi` is pointed at while `active`. The copy's buffer is
    /// kept from update to update, so an update allocates nothing once it
    /// has grown to the fleet's size.
    #[derive(Default)]
    struct Visit {
        list: u64,
        count: u64,
        sorted: Vec<u64>,
        active: bool,
    }

    thread_local! {
        static VISIT: RefCell<Visit> = RefCell::new(Visit::default());
    }

    /// Set to `0` (or `off`), the vehicle watcher stays quiet.
    pub const WATCH_ENV: &str = "TPF3MP_HOOK_WATCH_VEHICLES";
    static WATCH: AtomicBool = AtomicBool::new(false);
    /// One `TransportVehicle` component (`imul rbx, rax, 0x1e8` at the site).
    const COMPONENT_LEN: u64 = 0x1e8;
    /// Its fields the loop reads: the state (`0xb8bceb`), the line and the
    /// stop index (`0xb8bd7a`, `0xb8bd87`), the current terminal
    /// (`FindNextFreeTerminal`'s `[r12+0xc0]`, `[r12+0xc4]`).
    const STATE: u64 = 0xa8;
    const LINE: u64 = 0xb8;
    const STOP: u64 = 0xbc;
    const STATION: u64 = 0xc0;
    const TERMINAL: u64 = 0xc4;

    /// What the watcher keeps of one vehicle.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Seen {
        pub state: u32,
        pub line: i32,
        pub stop: i32,
        pub station: i32,
        pub terminal: i32,
    }

    /// The watcher's line for a vehicle whose state, stop or terminal
    /// changed in the room's step `step`; nothing in it differs between two
    /// games that agree (the engine is numbered, not named by address).
    pub fn watch_line(step: u64, engine: usize, entity: i32, seen: Seen) -> String {
        format!(
            "watch: step {step} engine {engine} vehicle {entity} state {} line {} stop {} terminal {}/{}",
            seen.state, seen.line, seen.stop, seen.station, seen.terminal
        )
    }

    /// The watcher's line for one free check of a candidate terminal's edge
    /// (`edge` is the `EdgeId`'s entity, index and direction byte): who
    /// holds it (`-1` free) and the vehicles on it in `EdgeUseManager`,
    /// each `entity:component:back:front:forward` (`None`: unreadable). An
    /// occupant that is not on the edge holds a reservation.
    #[allow(clippy::too_many_arguments)]
    pub fn check_line(
        step: u64,
        engine: usize,
        vehicle: i32,
        station: i32,
        terminal: i32,
        edge: EdgeKey,
        occupant: i32,
        entries: Option<&[super::road::Entry]>,
    ) -> String {
        let listed = match entries {
            None => "unreadable".to_owned(),
            Some(entries) => {
                let mut text = format!("{}", entries.len());
                for entry in entries.iter().take(MAX_LISTED as usize) {
                    let word = |i: usize| [entry[i], entry[i + 1], entry[i + 2], entry[i + 3]];
                    text.push_str(&format!(
                        " {}:{}:{:?}:{:?}:{}",
                        i32::from_le_bytes(word(0)),
                        i32::from_le_bytes(word(4)),
                        f32::from_le_bytes(word(8)),
                        f32::from_le_bytes(word(12)),
                        entry[16]
                    ));
                }
                text
            }
        };
        format!(
            "watch: step {step} engine {engine} vehicle {vehicle} checks {station}/{terminal} edge {}/{}/{} occupant {occupant} entries {listed}",
            edge.0, edge.1, edge.2
        )
    }

    /// The watcher's line for the candidate terminals a vehicle's choice
    /// weighs, in the order the cost sort gets them (each
    /// `first/station/terminal`).
    pub fn candidates_line(
        step: u64,
        engine: usize,
        vehicle: i32,
        candidates: &[[u32; 3]],
    ) -> String {
        let mut text = format!(
            "watch: step {step} engine {engine} vehicle {vehicle} candidates {}",
            candidates.len()
        );
        for c in candidates.iter().take(MAX_LISTED as usize) {
            text.push_str(&format!(" {}/{}/{}", c[0] as i32, c[1] as i32, c[2] as i32));
        }
        text
    }

    /// An edge as a check line names it: entity, index, direction byte.
    pub type EdgeKey = (i32, i32, u8);
    /// The occupant and the entities on the edge.
    type Answer = (i32, Vec<i32>);
    /// The candidates last said per `(engine, vehicle)`.
    type CandidatesSaid = std::collections::HashMap<(usize, i32), Vec<[u32; 3]>>;

    /// What the watcher said last of each `(engine, vehicle, edge)` free
    /// check: the occupant and the entities on the edge. A check is said
    /// when that changes, and a free edge only after it was said held, so a
    /// vehicle waiting for a platform says each change once.
    #[derive(Default)]
    pub struct Checks {
        said: std::collections::HashMap<(usize, i32, EdgeKey), Answer>,
    }

    impl Checks {
        /// Most checks kept; past it the memory starts again.
        const MAX: usize = 1 << 16;

        /// Whether this answer is to be said.
        pub fn note(
            &mut self,
            engine: usize,
            vehicle: i32,
            edge: EdgeKey,
            occupant: i32,
            on_edge: Vec<i32>,
        ) -> bool {
            let key = (engine, vehicle, edge);
            let now = (occupant, on_edge);
            match self.said.get(&key) {
                Some(before) if *before == now => false,
                Some(_) if occupant < 0 => {
                    self.said.remove(&key);
                    true
                }
                None if occupant < 0 => false,
                _ => {
                    if self.said.len() >= Self::MAX {
                        self.said.clear();
                    }
                    self.said.insert(key, now);
                    true
                }
            }
        }
    }

    /// A watched vehicle's `TransportVehicle` component, in hex words.
    pub fn transport_line(step: u64, engine: usize, entity: i32, words: &[u32]) -> String {
        let mut text = format!("watch: step {step} engine {engine} vehicle {entity} transport");
        for word in words {
            text.push_str(&format!(" {word:08x}"));
        }
        text
    }

    thread_local! {
        static WATCHED: RefCell<std::collections::HashMap<(u64, i32), Seen>> =
            RefCell::new(std::collections::HashMap::new());
        static ENGINES: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
        /// The vehicle the visit loop is looking at, once the watcher read
        /// it: the room's step, the engine's number and the entity.
        static CURRENT: std::cell::Cell<Option<(u64, usize, i32)>> = const { std::cell::Cell::new(None) };
        static CHECKS: RefCell<Checks> = RefCell::new(Checks::default());
        static CANDIDATES_SAID: RefCell<CandidatesSaid> =
            RefCell::new(std::collections::HashMap::new());
    }
    static OCCUPANT_BROKEN: AtomicBool = AtomicBool::new(false);

    /// The watcher, at each iteration of the loop: the vehicle the engine
    /// is about to look at (the record at `list + offset`), its state, stop
    /// and terminal, and a line when they changed since this system last
    /// visited it. Only in the room's updates, inside the game's step.
    fn watch(this: u64, list: u64, offset: u64) {
        let Some(step) = crate::seeds::current_step() else {
            return;
        };
        if !in_step() {
            return;
        }
        let mut probe = Probe::new();
        let Some(record) = list
            .checked_add(offset)
            .and_then(|at| probe.read::<u64>(at))
        else {
            return;
        };
        let entity = record as u32 as i32;
        let index = u64::from((record >> 32) as u32);
        let Some(base) = this
            .checked_add(0x10)
            .and_then(|at| probe.read::<u64>(at))
            .and_then(|at| probe.read::<u64>(at))
        else {
            return;
        };
        let Some(component) = index
            .checked_mul(COMPONENT_LEN)
            .and_then(|at| base.checked_add(at))
        else {
            return;
        };
        let field = |probe: &mut Probe, at: u64| probe.read::<u32>(component + at);
        let (Some(state), Some(line), Some(stop), Some(station), Some(terminal)) = (
            field(&mut probe, STATE),
            field(&mut probe, LINE),
            field(&mut probe, STOP),
            field(&mut probe, STATION),
            field(&mut probe, TERMINAL),
        ) else {
            return;
        };
        let seen = Seen {
            state,
            line: line as i32,
            stop: stop as i32,
            station: station as i32,
            terminal: terminal as i32,
        };
        let engine = ENGINES.with(|engines| {
            let mut engines = engines.borrow_mut();
            match engines.iter().position(|e| *e == this) {
                Some(at) => at,
                None => {
                    engines.push(this);
                    engines.len() - 1
                }
            }
        });
        CURRENT.with(|current| current.set(Some((step, engine, entity))));
        // A watched vehicle's whole TransportVehicle component, every visit:
        // its carrier (the first word) says which move system moves it.
        if super::claims::watched(entity) {
            let mut words = Vec::with_capacity((COMPONENT_LEN / 4) as usize);
            for at in (0..COMPONENT_LEN).step_by(4) {
                match probe.read::<u32>(component + at) {
                    Some(word) => words.push(word),
                    None => break,
                }
            }
            log::line(&transport_line(step, engine, entity, &words));
        }
        let changed =
            WATCHED.with(|watched| watched.borrow_mut().insert((this, entity), seen) != Some(seen));
        if changed {
            log::line(&watch_line(step, engine, entity, seen));
        }
    }

    pub fn install(resolved: &ResolvedProfile, wanted: bool) -> Vec<Outcome> {
        if !wanted {
            return vec![Outcome {
                fix: FIX,
                installed: false,
                reason: format!("{TOGGLE_ENV} says so; the engine's visit and tie order stand"),
            }];
        }
        let watching = crate::ticks::wanted(std::env::var(WATCH_ENV).ok().as_deref());
        WATCH.store(watching, Ordering::Release);
        let mut outcomes = vec![
            splice(
                resolved,
                VISIT_SITE,
                &VISIT_EXPECTED,
                VISIT_STEAL,
                visit_hook,
                "the vehicles are asked for a free platform in entity order",
            ),
            splice(
                resolved,
                CANDIDATES_SITE,
                &CANDIDATES_EXPECTED,
                CANDIDATES_STEAL,
                candidates_hook,
                "the candidate terminals are in one order before the cost sort",
            ),
        ];
        // The watcher's free-check and decision lines: logging only, with
        // the watcher.
        if watching {
            outcomes.push(splice(
                resolved,
                DECISION_SITE,
                &DECISION_EXPECTED,
                DECISION_STEAL,
                decision_hook,
                "the vehicle watcher logs land vehicles' platform-decision flag (logging only)",
            ));
            outcomes.push(splice(
                resolved,
                OCCUPANT_SITE,
                &OCCUPANT_EXPECTED,
                OCCUPANT_STEAL,
                occupant_hook,
                "the vehicle watcher logs who holds a candidate platform's edges (logging only)",
            ));
        }
        outcomes
    }

    fn splice(
        resolved: &ResolvedProfile,
        name: &str,
        expected: &[u8],
        steal: usize,
        hook: tpf3mp_hookcore::detour::SpliceHook,
        what: &str,
    ) -> Outcome {
        let off = |reason: String| Outcome {
            fix: FIX,
            installed: false,
            reason,
        };
        let Some(site) = resolved.get(name) else {
            return off(format!("the profile has no {name:?}"));
        };
        // SAFETY: a site inside a function the profile resolved and
        // prologue-checked, installed before any world exists; nothing
        // branches into the stolen bytes past the first (tpfre xrefs, noted
        // in the profile); the hook never unwinds (`guarded`).
        match unsafe { Splice::install(site.address as usize as *mut u8, expected, steal, hook) } {
            Ok(splice) => {
                let _kept = std::mem::ManuallyDrop::new(splice);
                Outcome {
                    fix: FIX,
                    installed: true,
                    reason: format!("{name} at {:#x}: {what}", site.address),
                }
            }
            Err(error) => off(format!("{name} at {:#x}: {error}", site.address)),
        }
    }

    /// The visit order the fix gives: the records by entity id (the low
    /// dword), each whole; `None` when they already are in it.
    pub fn visit_order(records: &[u64]) -> Result<Option<Vec<u64>>, &'static str> {
        let keys: Vec<i32> = records.iter().map(|r| *r as u32 as i32).collect();
        Ok(canonical_permutation(&keys)?.map(|order| order.iter().map(|&i| records[i]).collect()))
    }

    /// The candidates' canonical order: by station, terminal and the first
    /// word, as unsigned words; `None` when they already are in it. Equal
    /// candidates are interchangeable, so any tie among them is harmless.
    /// The reference for [`sort_candidates_in_place`], which the hook runs.
    pub fn candidate_order(candidates: &[[u32; 3]]) -> Option<Vec<[u32; 3]>> {
        let key = |c: &[u32; 3]| (c[1], c[2], c[0]);
        if candidates.windows(2).all(|p| key(&p[0]) <= key(&p[1])) {
            return None;
        }
        let mut sorted = candidates.to_vec();
        sorted.sort_by_key(key);
        Some(sorted)
    }

    /// One 12-byte candidate as the engine lays it out: three words.
    pub type Candidate = [u8; CANDIDATE_LEN as usize];

    fn candidate_key(c: &Candidate) -> (u32, u32, u32) {
        let word =
            |i: usize| u32::from_le_bytes([c[4 * i], c[4 * i + 1], c[4 * i + 2], c[4 * i + 3]]);
        (word(1), word(2), word(0))
    }

    /// [`candidate_order`], in place and allocating nothing: one scan when
    /// the candidates already are in order. Two candidates with one key are
    /// the same twelve bytes, so the unstable sort gives the same bytes as
    /// the reference's stable one.
    pub fn sort_candidates_in_place(candidates: &mut [Candidate]) -> Sorted {
        if candidates
            .windows(2)
            .all(|p| candidate_key(&p[0]) <= candidate_key(&p[1]))
        {
            return Sorted::Unchanged;
        }
        candidates.sort_unstable_by_key(candidate_key);
        Sorted::Reordered
    }

    /// The visit order of `count` records (`record(i)` the `i`th, entity id
    /// in its low dword) into `sorted`, the same order [`visit_order`]
    /// gives: nothing is copied when the records already are strictly in
    /// entity order, and `sorted`'s buffer is reused.
    pub fn sort_records(
        count: u64,
        record: impl Fn(u64) -> u64,
        sorted: &mut Vec<u64>,
    ) -> Result<Sorted, &'static str> {
        let key = |r: u64| r as u32 as i32;
        if (1..count).all(|i| key(record(i - 1)) < key(record(i))) {
            return Ok(Sorted::Unchanged);
        }
        sorted.clear();
        sorted.extend((0..count).map(&record));
        // Unique keys (checked next) are a total order: stable or not, one
        // result.
        sorted.sort_unstable_by_key(|r| key(*r));
        if sorted.windows(2).any(|p| key(p[0]) == key(p[1])) {
            sorted.clear();
            return Err("two entries name one entity");
        }
        Ok(Sorted::Reordered)
    }

    pub(super) unsafe extern "system" fn visit_hook(regs: *mut SavedRegs) {
        let _timer = perf::time(Piece::PlatformVisit);
        let rsp = SavedRegs::rsp(regs);
        guarded(FIX, &VISIT_BROKEN, || {
            // SAFETY: the stub's block, held until the hook returns.
            let regs = unsafe { &mut *regs };
            VISIT.with(|visit| {
                let mut visit = visit.borrow_mut();
                if regs.rsi == 0 {
                    // The loop's first iteration: rax is the node-list
                    // holder, rdi its begin, just loaded.
                    visit.active = false;
                    visit.list = 0;
                    visit.count = 0;
                    let n = VISIT_CALLS.fetch_add(1, Ordering::Relaxed) + 1;
                    super::decision_sync::note_list(regs.rbp, regs.rdi);
                    crate::copycheck::note_list(regs.rbp, regs.rdi);
                    match begin_loop(regs.rbp, regs.rax, regs.rdi, &mut visit.sorted) {
                        Ok((records, Sorted::Reordered)) => {
                            let reorders = VISIT_REORDERS.fetch_add(1, Ordering::Relaxed) + 1;
                            if reorders <= 3 {
                                log::line(&format!(
                                    "order fix {FIX}: {records} vehicles asked for platforms in entity order (update #{n})"
                                ));
                            }
                            visit.list = regs.rdi;
                            visit.count = records;
                            visit.active = true;
                        }
                        Ok((_, Sorted::Unchanged)) => {}
                        Err(why) => VISIT_REFUSALS.note(FIX, why),
                    }
                    if n == 1 || n.is_multiple_of(1 << 14) {
                        log::line(&format!(
                            "order fix {FIX}: visit alive, updates={n} reordered={} refused={}",
                            VISIT_REORDERS.load(Ordering::Relaxed),
                            VISIT_REFUSALS.count.load(Ordering::Relaxed)
                        ));
                    }
                }
                if visit.active
                    && (regs.rsi / RECORD_LEN >= visit.count || regs.rdi != visit.list)
                {
                    // Not the loop the copy was made for: the engine's own
                    // list from here, said once (never seen; the list is not
                    // changed inside the loop).
                    visit.active = false;
                    VISIT_REFUSALS.note(FIX, "the node list changed inside the loop");
                }
                if visit.active {
                    regs.rdi = visit.sorted.as_ptr() as u64;
                }
                if WATCH.load(Ordering::Relaxed) {
                    CURRENT.with(|current| current.set(None));
                    watch(regs.r13, regs.rdi, regs.rsi);
                    watch_path(rsp);
                }
            });
        });
    }

    /// Reads the node list through the frame: the record count, and whether
    /// `sorted` now holds the list in entity order (it does not when the
    /// list already was). The measurement notes the engine's order.
    fn begin_loop(
        rbp: u64,
        holder: u64,
        begin: u64,
        sorted: &mut Vec<u64>,
    ) -> Result<(u64, Sorted), &'static str> {
        let mut probe = Probe::new();
        let count: i32 = rbp
            .checked_add_signed(COUNT)
            .and_then(|at| probe.read(at))
            .ok_or("the node count is unreadable")?;
        let count = u64::try_from(count).map_err(|_| "a negative node count")?;
        if count > MAX_RECORDS {
            return Err("more vehicles than any world holds");
        }
        let stored: u64 = probe.read(holder).ok_or("the node list is unreadable")?;
        let end: u64 = holder
            .checked_add(8)
            .and_then(|at| probe.read(at))
            .ok_or("the node list's end is unreadable")?;
        if stored != begin || end < begin || end - begin != count * RECORD_LEN {
            return Err("the node list is not the count's whole records");
        }
        if count < 2 {
            measure::note_visits(&[], Sorted::Unchanged);
            return Ok((count, Sorted::Unchanged));
        }
        let len = usize::try_from(count * RECORD_LEN).map_err(|_| "the list is too long")?;
        if !usize::try_from(begin).is_ok_and(|begin| probe.readable(begin, len)) {
            return Err("the node records are unreadable");
        }
        // SAFETY: `len` readable bytes at `begin`, whole 8-byte records;
        // `i < count` for every index the sort asks for.
        let record =
            |i: u64| unsafe { std::ptr::read_unaligned((begin + i * RECORD_LEN) as *const u64) };
        let outcome = sort_records(count, record, sorted)?;
        if measure::enabled() {
            let before: Vec<i32> = (0..count).map(|i| record(i) as u32 as i32).collect();
            measure::note_visits(&before, outcome);
        }
        Ok((count, outcome))
    }

    pub(super) unsafe extern "system" fn candidates_hook(regs: *mut SavedRegs) {
        let _timer = perf::time(Piece::PlatformCandidates);
        guarded(FIX, &CANDIDATE_BROKEN, || {
            // SAFETY: the stub's block, held until the hook returns.
            let regs = unsafe { &*regs };
            let n = CANDIDATE_CALLS.fetch_add(1, Ordering::Relaxed) + 1;
            match sort_candidates(regs.r13, regs.r14) {
                Ok(sorted) => {
                    if sorted == Sorted::Reordered {
                        CANDIDATE_REORDERS.fetch_add(1, Ordering::Relaxed);
                    }
                    measure::note_candidates(sorted);
                }
                Err(why) => CANDIDATE_REFUSALS.note(FIX, why),
            }
            if WATCH.load(Ordering::Relaxed) {
                watch_candidates(regs.rbp, regs.r13, regs.r14);
            }
            if n == 1 || n.is_multiple_of(1 << 16) {
                log::line(&format!(
                    "order fix {FIX}: candidates alive, sorts={n} reordered={} refused={}",
                    CANDIDATE_REORDERS.load(Ordering::Relaxed),
                    CANDIDATE_REFUSALS.count.load(Ordering::Relaxed)
                ));
            }
        });
    }

    /// The vehicle `FindNextFreeTerminal` runs for, if it is the one the
    /// visit loop's watcher just read (always, inside the room's step).
    fn current_vehicle(rbp: u64) -> Option<(u64, usize, i32)> {
        let (step, engine, entity) = CURRENT.with(|current| current.get())?;
        if !in_step() {
            return None;
        }
        let vehicle: i32 = read(rbp.checked_add(VEHICLE_ARG)?)?;
        (vehicle == entity).then_some((step, engine, entity))
    }

    /// The watcher at the candidate site: the candidates of the vehicle's
    /// choice, in the order the cost sort gets them, when they changed.
    fn watch_candidates(rbp: u64, begin: u64, end: u64) {
        let Some((step, engine, vehicle)) = current_vehicle(rbp) else {
            return;
        };
        if end < begin || !(end - begin).is_multiple_of(CANDIDATE_LEN) {
            return;
        }
        let count = (end - begin) / CANDIDATE_LEN;
        if count > MAX_CANDIDATES {
            return;
        }
        let mut probe = Probe::new();
        let mut candidates = Vec::with_capacity(count as usize);
        for i in 0..count {
            let Some(candidate) = probe.read::<[u32; 3]>(begin + i * CANDIDATE_LEN) else {
                return;
            };
            candidates.push(candidate);
        }
        // A watched vehicle's every choice is said; others' when it changed.
        let changed = super::claims::watched(vehicle)
            || CANDIDATES_SAID.with(|said| {
                let mut said = said.borrow_mut();
                if said.get(&(engine, vehicle)) == Some(&candidates) {
                    return false;
                }
                if said.len() >= Checks::MAX {
                    said.clear();
                }
                said.insert((engine, vehicle), candidates.clone());
                true
            });
        if changed {
            log::line(&candidates_line(step, engine, vehicle, &candidates));
        }
    }

    /// The watcher at the free check: who holds the candidate terminal's
    /// edge just asked about. Reads only.
    pub(super) unsafe extern "system" fn occupant_hook(regs: *mut SavedRegs) {
        guarded(FIX, &OCCUPANT_BROKEN, || {
            let rsp = SavedRegs::rsp(regs);
            // SAFETY: the stub's block, held until the hook returns.
            let regs = unsafe { &*regs };
            let Some((step, engine, vehicle)) = current_vehicle(regs.rbp) else {
                return;
            };
            if regs.rdi as u32 as i32 != vehicle {
                return;
            }
            let mut probe = Probe::new();
            let (Some(checked), Some(terminal), Some(occupant), Some(reservations), Some(edge)) = (
                probe.read::<u64>(rsp + CHECKED),
                probe.read::<i32>(rsp + CHECKED_TERMINAL),
                probe.read::<i32>(rsp + ANSWER),
                probe.read::<u64>(rsp + RESERVATIONS),
                probe.read::<[u8; 12]>(regs.r15),
            ) else {
                return;
            };
            let word =
                |i: usize| i32::from_le_bytes([edge[i], edge[i + 1], edge[i + 2], edge[i + 3]]);
            let edge_key = (word(0), word(4), edge[8]);
            let entries = edge_entries(&mut probe, reservations, regs.r15);
            let on_edge: Vec<i32> = entries
                .as_deref()
                .map(|entries| entries.iter().map(super::road::key).collect())
                .unwrap_or_default();
            let say = super::claims::watched(vehicle)
                || CHECKS.with(|checks| {
                    checks
                        .borrow_mut()
                        .note(engine, vehicle, edge_key, occupant, on_edge)
                });
            if say {
                log::line(&check_line(
                    step,
                    engine,
                    vehicle,
                    (checked >> 32) as u32 as i32,
                    terminal,
                    edge_key,
                    occupant,
                    entries.as_deref(),
                ));
            }
        });
    }

    /// A path edge's bytes that mean something: its entity, its index and
    /// its direction byte (`+0x08`); the three bytes after are padding,
    /// different from game to game.
    pub fn path_edge_bytes(edge: &[u8; 12]) -> [u8; 9] {
        let mut out = [0u8; 9];
        out.copy_from_slice(&edge[..9]);
        out
    }

    /// A watched vehicle's path, when it changed for that engine: each
    /// edge `entity/index/direction`.
    pub fn path_line(step: u64, engine: usize, vehicle: i32, edges: &[(i32, i32, u8)]) -> String {
        let mut text = format!(
            "watch: step {step} engine {engine} vehicle {vehicle} path {}:",
            edges.len()
        );
        for (entity, index, forward) in edges {
            text.push_str(&format!(" {entity}/{index}/{forward}"));
        }
        text
    }

    /// The paths last said per `(engine, vehicle)`.
    type PathsSaid = std::collections::HashMap<(usize, i32), Vec<(i32, i32, u8)>>;

    thread_local! {
        static PATHS: RefCell<PathsSaid> =
            RefCell::new(std::collections::HashMap::new());
    }

    /// The watcher's path line for a watched vehicle, at the visit loop
    /// (`rsp` the loop's frame: the engine at `[rsp+0x70]`).
    fn watch_path(rsp: u64) {
        let Some((step, engine, vehicle)) = CURRENT.with(|current| current.get()) else {
            return;
        };
        if !super::claims::watched(vehicle) {
            return;
        }
        let mut probe = Probe::new();
        let Some(ecs) = probe.read::<u64>(rsp + 0x70) else {
            return;
        };
        let Some(mp) = super::decision_sync::movepath_of(ecs as usize, vehicle) else {
            return;
        };
        let (Some(begin), Some(end)) = (probe.read::<u64>(mp), probe.read::<u64>(mp + 8)) else {
            return;
        };
        if end < begin || !(end - begin).is_multiple_of(12) || (end - begin) / 12 > 1 << 16 {
            return;
        }
        let mut edges = Vec::with_capacity(((end - begin) / 12) as usize);
        for i in 0..(end - begin) / 12 {
            let Some(edge) = probe.read::<[u8; 12]>(begin + i * 12) else {
                return;
            };
            let word = |at: usize| {
                i32::from_le_bytes([edge[at], edge[at + 1], edge[at + 2], edge[at + 3]])
            };
            edges.push((word(0), word(4), edge[8]));
        }
        let changed = PATHS.with(|paths| {
            let mut paths = paths.borrow_mut();
            if paths.get(&(engine, vehicle)) == Some(&edges) {
                return false;
            }
            paths.insert((engine, vehicle), edges.clone());
            true
        });
        if changed {
            log::line(&path_line(step, engine, vehicle, &edges));
        }
    }

    /// The watcher's line for a land vehicle's platform-decision flag, when
    /// the loop reads another value than it read last for that vehicle.
    pub fn decision_line(step: u64, engine: usize, vehicle: i32, flag: u8) -> String {
        format!("watch: step {step} engine {engine} vehicle {vehicle} decision flag {flag}")
    }

    /// A watched vehicle's `MovePath` as the decision read sees it: its
    /// bytes from `+0x18` in hex words, its path's edge count and hash.
    pub fn movepath_line(
        step: u64,
        engine: usize,
        vehicle: i32,
        path_len: u64,
        path_hash: u64,
        words: &[u32],
    ) -> String {
        let mut text = format!(
            "watch: step {step} engine {engine} vehicle {vehicle} movepath path {path_len}/{path_hash:016x}"
        );
        for word in words {
            text.push_str(&format!(" {word:08x}"));
        }
        text
    }

    thread_local! {
        static FLAGS: RefCell<std::collections::HashMap<(usize, i32), u8>> =
            RefCell::new(std::collections::HashMap::new());
    }

    /// The watcher at the decision read: `rax` is the vehicle's `MovePath`.
    pub(super) unsafe extern "system" fn decision_hook(regs: *mut SavedRegs) {
        guarded(FIX, &OCCUPANT_BROKEN, || {
            // SAFETY: the stub's block, held until the hook returns.
            let regs = unsafe { &*regs };
            let Some((step, engine, vehicle)) = CURRENT.with(|current| current.get()) else {
                return;
            };
            if !in_step() {
                return;
            }
            let mut probe = Probe::new();
            let Some(listed) = regs
                .rdi
                .checked_add(regs.rsi)
                .and_then(|at| probe.read::<i32>(at))
            else {
                return;
            };
            if listed != vehicle {
                return;
            }
            let mp = regs.rax;
            let Some(flag) = probe.read::<u8>(mp + 0x70) else {
                return;
            };
            let changed = FLAGS.with(|flags| {
                let mut flags = flags.borrow_mut();
                if flags.len() >= Checks::MAX {
                    flags.clear();
                }
                flags.insert((engine, vehicle), flag) != Some(flag)
            });
            if changed {
                log::line(&decision_line(step, engine, vehicle, flag));
            }
            if !super::claims::watched(vehicle) {
                return;
            }
            let len_all = super::claims::MOVE_PATH_LEN;
            let mut words = Vec::with_capacity(((len_all - 0x18) / 4) as usize);
            let mut at = 0x18;
            while at < len_all {
                let Some(word) = probe.read::<u32>(mp + at) else {
                    return;
                };
                words.push(word);
                at += 4;
            }
            let (Some(begin), Some(end)) = (probe.read::<u64>(mp), probe.read::<u64>(mp + 8))
            else {
                return;
            };
            let mut hash = Fnv1a::new();
            let mut len = 0;
            if end >= begin && (end - begin).is_multiple_of(12) && (end - begin) / 12 <= 1 << 16 {
                len = (end - begin) / 12;
                for i in 0..len {
                    match probe.read::<[u8; 12]>(begin + i * 12) {
                        Some(edge) => hash.write(&path_edge_bytes(&edge)),
                        None => break,
                    }
                }
            }
            log::line(&movepath_line(step, engine, vehicle, len, hash.0, &words));
        });
    }

    /// The entries of the edge `edge_id` names in the `EdgeUseManager` the
    /// reservation manager `reservations` falls back on (`[reservations]`;
    /// its data at `+0x18`, as `GetEdgeDataPtr` `0x255f2a0` reads it).
    fn edge_entries(
        probe: &mut Probe,
        reservations: u64,
        edge_id: u64,
    ) -> Option<Vec<super::road::Entry>> {
        let manager: u64 = probe.read(reservations)?;
        let data: u64 = probe.read(manager.checked_add(super::road::MANAGER_DATA)?)?;
        let vector = super::road::entries_of(probe, data, edge_id).ok()?;
        let begin: u64 = probe.read(vector)?;
        let end: u64 = probe.read(vector.checked_add(8)?)?;
        let len = super::road::ENTRY_LEN;
        if end < begin
            || !(end - begin).is_multiple_of(len)
            || (end - begin) / len > super::road::MAX_ENTRIES
        {
            return None;
        }
        (0..(end - begin) / len)
            .map(|i| probe.read::<super::road::Entry>(begin + i * len))
            .collect()
    }

    fn sort_candidates(begin: u64, end: u64) -> Result<Sorted, &'static str> {
        if end < begin || !(end - begin).is_multiple_of(CANDIDATE_LEN) {
            return Err("the candidates are not whole entries");
        }
        let count = (end - begin) / CANDIDATE_LEN;
        if count > MAX_CANDIDATES {
            return Err("more candidates than a station has");
        }
        if count < 2 {
            return Ok(Sorted::Unchanged);
        }
        let len = usize::try_from(count * CANDIDATE_LEN).map_err(|_| "too many candidates")?;
        if !readable(begin, len) {
            return Err("the candidates are unreadable");
        }
        // SAFETY: `len` readable bytes at `begin` (not null: at least two
        // candidates), whole 12-byte entries of alignment 1; the engine's
        // thread is the one running, and its sort reads them only after
        // this site, so nothing else holds them while the slice lives.
        let candidates = unsafe {
            std::slice::from_raw_parts_mut(begin as usize as *mut Candidate, count as usize)
        };
        Ok(sort_candidates_in_place(candidates))
    }
}

/// The claim loop's watcher (logging only; it changes nothing): what
/// `ecs::LandVehicleMoveSystem::Update2`'s reservation loop sees and decides
/// for each land vehicle, to find where two games' vehicles first differ.
///
/// - **head** (`0xac1d9d`, right after `rbx` is the vehicle's `MovePath`
///   component, 0xa0 bytes, and `r12` its node record, the entity first):
///   for the entities `TPF3MP_HOOK_WATCH_ENTITIES` lists, one line every
///   update with the component's bytes from `+0x18` (past its path vector)
///   in full, the path's length and a hash of its edges, and the priority
///   the claim order gave it.
/// - **decision** (`0xac2235`, `mov [rbx+0x70], cl`): the `MovePath` flag
///   `TransportVehicleSystem::Update2` reads before it asks
///   `FindNextFreeTerminal` for a platform (`0xb8bdb3`); set when the claim
///   reaches `[rsp+0x68]` at least the path index `edi` where the platform
///   is decided. One line for every vehicle whose flag changes.
pub mod claims {
    use std::collections::HashSet;
    use std::sync::OnceLock;

    use super::*;

    pub const FIX: &str = "claim-watch";
    /// A comma-separated list of entity ids whose claim-loop state is
    /// logged every update.
    pub const ENTITIES_ENV: &str = "TPF3MP_HOOK_WATCH_ENTITIES";
    pub const HEAD_SITE: &str = "ecs::LandVehicleMoveSystem::Update2/claim head";
    pub const HEAD_EXPECTED: [u8; 7] = [
        0x8B, 0x73, 0x44, // mov esi, [rbx+0x44]
        0x4C, 0x63, 0x73, 0x48, // movsxd r14, [rbx+0x48]
    ];
    pub const HEAD_STEAL: usize = 7;
    pub const DECISION_SITE: &str = "ecs::LandVehicleMoveSystem::Update2/terminal decision";
    pub const DECISION_EXPECTED: [u8; 7] = [
        0x88, 0x4B, 0x70, // mov [rbx+0x70], cl
        0x4B, 0x8D, 0x14, 0x76, // lea rdx, [r14+r14*2]
    ];
    pub const DECISION_STEAL: usize = 7;
    /// The `MovePath` component's length, its path vector's (`{begin, end,
    /// capacity}`, 12-byte edges) and the decision flag's offset.
    pub const MOVE_PATH_LEN: u64 = 0xa0;
    const PATH_VECTOR_LEN: u64 = 0x18;
    const PATH_EDGE_LEN: u64 = 12;
    const MAX_PATH_EDGES: u64 = 1 << 16;
    const FLAG: u64 = 0x70;
    /// The claim reached, at `[rsp+0x68]` at the decision site.
    const CLAIMED: u64 = 0x68;

    static WATCHED: OnceLock<HashSet<i32>> = OnceLock::new();
    static BROKEN: AtomicBool = AtomicBool::new(false);

    /// The entity ids `value` lists (commas or spaces between them); what
    /// does not read as one is left out.
    pub fn parse_entities(value: Option<&str>) -> HashSet<i32> {
        value
            .unwrap_or("")
            .split([',', ' ', ';'])
            .filter_map(|word| word.trim().parse().ok())
            .collect()
    }

    /// Whether `TPF3MP_HOOK_WATCH_ENTITIES` lists `entity`.
    pub fn watched(entity: i32) -> bool {
        WATCHED.get().is_some_and(|w| w.contains(&entity))
    }

    /// The head line: the component's bytes from `+0x18` as little-endian
    /// words in hex, the path's edge count and FNV-1a hash.
    pub fn head_line(
        step: u64,
        entity: i32,
        priority: f32,
        words: &[u32],
        path_len: u64,
        path_hash: u64,
    ) -> String {
        let mut text = format!(
            "claim: step {step} vehicle {entity} priority {priority:?} path {path_len}/{path_hash:016x} movepath"
        );
        for word in words {
            text.push_str(&format!(" {word:08x}"));
        }
        text
    }

    /// The decision line.
    pub fn decision_line(
        step: u64,
        entity: i32,
        flag: bool,
        claimed: i32,
        decision: i32,
    ) -> String {
        format!(
            "claim: step {step} vehicle {entity} terminal decision {} (claimed to {claimed}, decided at {decision})",
            u8::from(flag)
        )
    }

    pub fn install(resolved: &ResolvedProfile) -> Vec<Outcome> {
        let watched = parse_entities(std::env::var(ENTITIES_ENV).ok().as_deref());
        let listed = watched.len();
        let _ = WATCHED.set(watched);
        let mut outcomes = vec![splice_one(
            resolved,
            DECISION_SITE,
            &DECISION_EXPECTED,
            DECISION_STEAL,
            decision_hook,
            "every land vehicle's platform-decision flag change is logged (logging only)",
        )];
        if listed > 0 {
            outcomes.push(splice_one(
                resolved,
                HEAD_SITE,
                &HEAD_EXPECTED,
                HEAD_STEAL,
                head_hook,
                &format!("the claim loop's view of {listed} entities from {ENTITIES_ENV} is logged every update (logging only)"),
            ));
        }
        outcomes
    }

    fn splice_one(
        resolved: &ResolvedProfile,
        name: &str,
        expected: &[u8],
        steal: usize,
        hook: tpf3mp_hookcore::detour::SpliceHook,
        what: &str,
    ) -> Outcome {
        let off = |reason: String| Outcome {
            fix: FIX,
            installed: false,
            reason,
        };
        let Some(site) = resolved.get(name) else {
            return off(format!("the profile has no {name:?}"));
        };
        // SAFETY: a site inside a function the profile resolved and
        // prologue-checked, installed before any world exists; nothing
        // branches into the stolen bytes past the first (tpfre, noted in
        // the profile); the hook only reads and never unwinds (`guarded`).
        match unsafe { Splice::install(site.address as usize as *mut u8, expected, steal, hook) } {
            Ok(splice) => {
                let _kept = std::mem::ManuallyDrop::new(splice);
                Outcome {
                    fix: FIX,
                    installed: true,
                    reason: format!("{name} at {:#x}: {what}", site.address),
                }
            }
            Err(error) => off(format!("{name} at {:#x}: {error}", site.address)),
        }
    }

    /// The room's step, inside the game's step only.
    fn step() -> Option<u64> {
        if !in_step() {
            return None;
        }
        crate::seeds::current_step()
    }

    pub(super) unsafe extern "system" fn decision_hook(regs: *mut SavedRegs) {
        guarded(FIX, &BROKEN, || {
            let rsp = SavedRegs::rsp(regs);
            // SAFETY: the stub's block, held until the hook returns.
            let regs = unsafe { &*regs };
            let Some(step) = step() else {
                return;
            };
            let mut probe = Probe::new();
            let (Some(before), Some(entity), Some(claimed)) = (
                probe.read::<u8>(regs.rbx + FLAG),
                probe.read::<i32>(regs.r12),
                probe.read::<i32>(rsp + CLAIMED),
            ) else {
                return;
            };
            let flag = regs.rcx as u8 != 0;
            if (before != 0) != flag {
                log::line(&decision_line(
                    step,
                    entity,
                    flag,
                    claimed,
                    regs.rdi as u32 as i32,
                ));
            }
        });
    }

    pub(super) unsafe extern "system" fn head_hook(regs: *mut SavedRegs) {
        guarded(FIX, &BROKEN, || {
            // SAFETY: the stub's block, held until the hook returns.
            let regs = unsafe { &*regs };
            let Some(step) = step() else {
                return;
            };
            let mut probe = Probe::new();
            let Some(entity) = probe.read::<i32>(regs.r12) else {
                return;
            };
            if !watched(entity) {
                return;
            }
            let priority = probe.read::<f32>(regs.r13 + 4).unwrap_or(f32::NAN);
            let mut words = Vec::with_capacity(((MOVE_PATH_LEN - PATH_VECTOR_LEN) / 4) as usize);
            let mut at = PATH_VECTOR_LEN;
            while at < MOVE_PATH_LEN {
                let Some(word) = probe.read::<u32>(regs.rbx + at) else {
                    return;
                };
                words.push(word);
                at += 4;
            }
            let (Some(begin), Some(end)) =
                (probe.read::<u64>(regs.rbx), probe.read::<u64>(regs.rbx + 8))
            else {
                return;
            };
            let mut hash = Fnv1a::new();
            let mut len = 0;
            if end >= begin && (end - begin).is_multiple_of(PATH_EDGE_LEN) {
                len = (end - begin) / PATH_EDGE_LEN;
                for i in 0..len.min(MAX_PATH_EDGES) {
                    match probe.read::<[u8; 12]>(begin + i * PATH_EDGE_LEN) {
                        Some(edge) => hash.write(&super::platform::path_edge_bytes(&edge)),
                        None => break,
                    }
                }
            }
            log::line(&head_line(step, entity, priority, &words, len, hash.0));
        });
    }
}

/// The platform-decision flag, the same in both of the game's engines (the
/// 2026-10-01 step-3300 split on `twomptest`, docs/HOOKS.md, "The
/// platform-decision flag").
///
/// `TransportVehicleSystem::Update2` asks `FindNextFreeTerminal` for a land
/// vehicle only while its `MovePath` byte `+0x70` is set (`0xb8bdb3`). The
/// game simulates its two `GameState`s in turn, a frame each, and copies
/// the one just simulated into the other, but that byte is not carried
/// over: in every room's log, each vehicle's flag is set and cleared in the
/// one engine that ran the step where it changed, and the other engine
/// reads its old value for hundreds of steps (the `decision flag` lines).
/// Which engine runs a room's step follows each game's frames, so a game
/// asks for a platform on other steps than another, and vehicle 217708
/// either took platform 0/0 at step 3201 or drove a loop.
///
/// The fix gives the flag one engine's semantics: at the start of every
/// simulation update (`ecs::Engine::Update`, from the seeds' detour), when
/// the update before ran on the other engine, each transport vehicle's flag
/// is copied from that engine's `MovePath` into this one's. By induction
/// the engine about to simulate then holds the flag every write so far
/// left, whichever engine made it. The vehicles are the transport vehicle
/// system's node list (read at its loop's first record); the `MovePath`s are
/// found with the game's own getter (`0x52bbc0`, the call the decision read
/// follows, checked to name the `MovePath` type). A load forgets the engine
/// before.
pub mod decision_sync {
    use std::sync::Mutex;

    use super::*;

    pub const FIX: &str = "decision-sync";
    /// Set to `1` (or `on`), the copy runs; off otherwise. Round A
    /// (2026-10-01, `crate::copycheck`) found the game's own engine copy
    /// carries `+0x4c..+0x74`, the flag among it, for every vehicle: the
    /// diagnosis this rested on was wrong, and it copied nothing.
    pub const TOGGLE_ENV: &str = "TPF3MP_HOOK_DECISION_SYNC";
    /// The flag's offset in `MovePath`.
    pub const FLAG: u64 = 0x70;
    /// The getter's call, this many bytes before the decision read.
    const CALL_BEFORE_READ: u64 = 0x0e;
    /// The getter's first bytes, up to its `lea rax, [MovePath's type
    /// descriptor]` (whose rel32 follows).
    pub const GETTER_EXPECTED: [u8; 21] = [
        0x48, 0x89, 0x5C, 0x24, 0x10, // mov [rsp+0x10], rbx
        0x48, 0x89, 0x74, 0x24, 0x18, // mov [rsp+0x18], rsi
        0x57, // push rdi
        0x48, 0x83, 0xEC, 0x30, // sub rsp, 0x30
        0x48, 0x63, 0xDA, // movsxd rbx, edx
        0x48, 0x8D, 0x05, // lea rax, [rip+rel32]
    ];
    /// The type descriptor's decorated name, at its `+0x10`.
    pub const TYPE_NAME: &[u8] = b".?AUMovePath@component@ecs@@\0";
    const MAX_VEHICLES: u64 = 1 << 20;

    type GetterFn = unsafe extern "system" fn(usize, i32) -> usize;

    static GETTER: AtomicUsize = AtomicUsize::new(0);
    /// Whether the copy runs ([`TOGGLE_ENV`]); the getter serves the
    /// vehicle watcher's path lines either way.
    static SYNC: AtomicBool = AtomicBool::new(false);
    static BROKEN: AtomicBool = AtomicBool::new(false);

    /// The game's `MovePath` getter, once found: `(engine, entity)` to the
    /// component or 0.
    pub fn movepath_of(engine: usize, entity: i32) -> Option<u64> {
        let getter = GETTER.load(Ordering::Acquire);
        if getter == 0 || engine == 0 {
            return None;
        }
        // SAFETY: the game's getter, checked at install; it reads the
        // engine's tables and answers a pointer or null.
        let get: GetterFn = unsafe { std::mem::transmute::<usize, GetterFn>(getter) };
        let at = unsafe { get(engine, entity) };
        (at != 0).then_some(at as u64)
    }
    static UPDATES: AtomicU64 = AtomicU64::new(0);
    static SYNCS: AtomicU64 = AtomicU64::new(0);
    static COPIED: AtomicU64 = AtomicU64::new(0);

    struct State {
        /// The engine the last room's update ran on.
        last: usize,
        /// The transport vehicles, as the node list last gave them.
        vehicles: Vec<i32>,
    }

    static STATE: Mutex<State> = Mutex::new(State {
        last: 0,
        vehicles: Vec::new(),
    });

    fn state() -> std::sync::MutexGuard<'static, State> {
        STATE.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    /// The getter a verified call names, or why not.
    pub fn getter_from_call(
        call_site: u64,
        read: &mut dyn FnMut(u64, usize) -> Option<Vec<u8>>,
    ) -> Result<u64, &'static str> {
        let call = read(call_site, 5).ok_or("the getter's call is unreadable")?;
        if call[0] != 0xE8 {
            return Err("no call before the decision read");
        }
        let rel = i32::from_le_bytes([call[1], call[2], call[3], call[4]]);
        let target = call_site
            .wrapping_add(5)
            .wrapping_add_signed(i64::from(rel));
        let head = read(target, GETTER_EXPECTED.len() + 4).ok_or("the getter is unreadable")?;
        if head[..GETTER_EXPECTED.len()] != GETTER_EXPECTED {
            return Err("the getter is not the shape expected");
        }
        let at = GETTER_EXPECTED.len();
        let rel = i32::from_le_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]);
        let descriptor = target
            .wrapping_add((at + 4) as u64)
            .wrapping_add_signed(i64::from(rel));
        let name =
            read(descriptor + 0x10, TYPE_NAME.len()).ok_or("the type's name is unreadable")?;
        if name != TYPE_NAME {
            return Err("the getter is not MovePath's");
        }
        Ok(target)
    }

    pub fn install(resolved: &ResolvedProfile, wanted: bool) -> Outcome {
        let off = |reason: String| Outcome {
            fix: FIX,
            installed: false,
            reason,
        };
        let Some(site) = resolved.get(super::platform::DECISION_SITE) else {
            return off(format!(
                "the profile has no {:?}",
                super::platform::DECISION_SITE
            ));
        };
        let mut read = |at: u64, len: usize| -> Option<Vec<u8>> {
            let at = usize::try_from(at).ok()?;
            if !crate::image::readable(at, len) {
                return None;
            }
            // SAFETY: `len` readable bytes at `at`, in the game's image.
            Some(unsafe { std::slice::from_raw_parts(at as *const u8, len) }.to_vec())
        };
        match getter_from_call(site.address - CALL_BEFORE_READ, &mut read) {
            Ok(getter) => {
                GETTER.store(getter as usize, Ordering::Release);
                SYNC.store(wanted, Ordering::Release);
                if !wanted {
                    return off(format!(
                        "off unless {TOGGLE_ENV}=1 (round A showed the game's own copy carries the flag); the MovePath getter at {getter:#x} serves the path watch"
                    ));
                }
                Outcome {
                    fix: FIX,
                    installed: true,
                    reason: format!(
                        "the MovePath getter at {getter:#x}: each update copies the land vehicles' platform-decision flag from the engine that ran the update before"
                    ),
                }
            }
            Err(why) => off(format!("{why}; each engine keeps its own flag")),
        }
    }

    /// A world was loaded: the engines start from it.
    pub fn reset() {
        state().last = 0;
    }

    /// At the transport vehicle loop's first record (`rbp` its frame,
    /// `begin` its 8-byte records): the vehicles, by entity.
    pub fn note_list(rbp: u64, begin: u64) {
        if !SYNC.load(Ordering::Acquire)
            || GETTER.load(Ordering::Acquire) == 0
            || BROKEN.load(Ordering::Relaxed)
            || !in_step()
        {
            return;
        }
        if let Some(vehicles) = read_list(rbp, begin) {
            state().vehicles = vehicles;
        }
    }

    /// The transport vehicle loop's vehicles, by entity, read at its first
    /// record (`rbp` its frame, `begin` its 8-byte records); `None` when the
    /// count or a record is unreadable or the count is not plausible. Also
    /// the engine-copy checker's list (crate::copycheck).
    pub fn read_list(rbp: u64, begin: u64) -> Option<Vec<i32>> {
        let mut probe = Probe::new();
        let count = probe.read::<i32>(rbp.checked_add_signed(0x5b0)?)?;
        let count = u64::try_from(count).ok()?;
        if count > MAX_VEHICLES {
            return None;
        }
        let mut vehicles = Vec::with_capacity(count as usize);
        for i in 0..count {
            let record = probe.read::<u64>(begin.checked_add(i * 8)?)?;
            vehicles.push(record as u32 as i32);
        }
        Some(vehicles)
    }

    /// Copies each vehicle's flag from the engine `before` into `engine`'s
    /// `MovePath` (`get` finds a vehicle's in an engine, 0 when it has
    /// none), and answers how many differed.
    pub fn copy_flags(
        before: usize,
        engine: usize,
        vehicles: &[i32],
        get: impl Fn(usize, i32) -> usize,
    ) -> u64 {
        let mut probe = Probe::new();
        let mut copied = 0u64;
        for &vehicle in vehicles {
            let (from, to) = (get(before, vehicle), get(engine, vehicle));
            if from == 0 || to == 0 {
                continue;
            }
            let (from, to) = (from as u64 + FLAG, to as u64 + FLAG);
            let (Some(value), Some(own)) = (probe.read::<u8>(from), probe.read::<u8>(to)) else {
                continue;
            };
            if value != own {
                // SAFETY: a readable byte of this engine's MovePath, which
                // nothing else touches before its update begins.
                unsafe { std::ptr::write_volatile(to as usize as *mut u8, value) };
                copied += 1;
            }
        }
        copied
    }

    /// Before an update of `engine`, inside the room's step: the flags
    /// from the engine before, if it was the other.
    pub fn before_update(engine: usize) {
        let getter = GETTER.load(Ordering::Acquire);
        if !SYNC.load(Ordering::Acquire)
            || getter == 0
            || engine == 0
            || !in_step()
            || crate::seeds::current_step().is_none()
        {
            return;
        }
        guarded(FIX, &BROKEN, || {
            let n = UPDATES.fetch_add(1, Ordering::Relaxed) + 1;
            let mut state = state();
            let before = std::mem::replace(&mut state.last, engine);
            if before == 0 || before == engine {
                return;
            }
            // SAFETY: the game's MovePath getter (checked at install): it
            // reads the engine's component tables and answers a pointer or
            // null; both engines live while the room's world does (a load
            // resets `last`).
            let get: GetterFn = unsafe { std::mem::transmute::<usize, GetterFn>(getter) };
            // SAFETY: as above.
            let copied = copy_flags(before, engine, &state.vehicles, |engine, vehicle| unsafe {
                get(engine, vehicle)
            });
            let syncs = SYNCS.fetch_add(1, Ordering::Relaxed) + 1;
            let total = COPIED.fetch_add(copied, Ordering::Relaxed) + copied;
            if (copied > 0 && total == copied) || n.is_multiple_of(1 << 14) {
                log::line(&format!(
                    "order fix {FIX}: alive, updates={n} engine changes={syncs} flags copied={total}"
                ));
            }
        });
    }
}

/// The path finder's tie order (the 2026-10-01 step-3300 split on
/// `twomptest`, docs/HOOKS.md, "Path ties").
///
/// `transport::PathFinder<...>::PrioritySearch` keeps its open search
/// segments (24 bytes each, at `[this+8]`; the cost so far at `+0x0c`, the
/// heuristic at `+0x10`) in a sorted list, and sorts each batch of new ones
/// (`SortAndMergeUnsorted`) with `std::sort` over their `int` indices by
/// `+0x10 + +0x0c` alone (`0x5af710`, shared by every instantiation with
/// that layout: road and rail vehicles' `pair<EdgeId,bool>` searches and
/// the persons' `LinkPathSeg` ones). Equal costs, as a station's parallel
/// lanes have, come out in an order that depends on the batch's input
/// order, so two games' buses took different lanes through construction
/// 362201 on the same 26 edges at step 3200, and one reached its stop a lap
/// later. The fix detours the sort and orders the indices by the cost, then
/// by the segment's first 12 bytes (its edge or link key), then by its cost
/// so far, a stable sort: one order for one set of segments, whatever the
/// input order, and still the order by cost the search needs.
pub mod path_ties {
    use super::*;

    pub const FIX: &str = "path-tie-order";
    /// Set to `0` (or `off`), the engine's own sort stands.
    pub const TOGGLE_ENV: &str = "TPF3MP_HOOK_PATH_TIE_ORDER";
    /// The road search's call of the sort (`0x266ce45`); the sort is its
    /// target.
    pub const CALL_SITE: &str = "PathFinder::PrioritySearch/sort call";
    /// The sort's first bytes.
    pub const SORT_EXPECTED: [u8; 22] = [
        0x48, 0x89, 0x5C, 0x24, 0x18, // mov [rsp+0x18], rbx
        0x48, 0x89, 0x6C, 0x24, 0x20, // mov [rsp+0x20], rbp
        0x56, 0x57, 0x41, 0x54, 0x41, 0x56, 0x41, 0x57, // push rsi .. r15
        0x48, 0x83, 0xEC, 0x40, // sub rsp, 0x40
    ];
    /// Its comparator, at `+0xf0`: `mov rdx,[rbx+8]` .. `vaddss xmm3, xmm0,
    /// [rdx+r10*8+0xc]` (the segments, `+0x10` plus `+0x0c`).
    pub const COMPARATOR_AT: u64 = 0xf0;
    pub const COMPARATOR_EXPECTED: [u8; 31] = [
        0x48, 0x8B, 0x53, 0x08, // mov rdx, [rbx+8]
        0x4C, 0x8B, 0xDE, // mov r11, rsi
        0x4C, 0x63, 0x3E, // movsxd r15, [rsi]
        0x48, 0x63, 0x07, // movsxd rax, [rdi]
        0x4F, 0x8D, 0x14, 0x7F, // lea r10, [r15+r15*2]
        0xC4, 0xA1, 0x7A, 0x10, 0x44, 0xD2, 0x10, // vmovss xmm0, [rdx+r10*8+0x10]
        0xC4, 0xA1, 0x7A, 0x58, 0x5C, 0xD2, 0x0C, // vaddss xmm3, xmm0, [rdx+r10*8+0xc]
    ];
    pub const SEGMENT_LEN: usize = 24;
    const MAX_SEGMENTS: usize = 1 << 26;

    static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static BROKEN: AtomicBool = AtomicBool::new(false);
    static CALLS: AtomicU64 = AtomicU64::new(0);
    static TIES: AtomicU64 = AtomicU64::new(0);
    static REFUSED: AtomicU64 = AtomicU64::new(0);

    type SortFn = unsafe extern "system" fn(*mut i32, *mut i32, isize, usize);

    /// A segment's sort key: its cost (`+0x10` plus `+0x0c`, added as the
    /// engine adds them), its first 12 bytes, its cost so far.
    pub fn key(segment: &[u8]) -> (f32, [u8; 12], u32) {
        let word = |at: usize| {
            [
                segment[at],
                segment[at + 1],
                segment[at + 2],
                segment[at + 3],
            ]
        };
        let so_far = f32::from_le_bytes(word(0x0c));
        let cost = f32::from_le_bytes(word(0x10)) + so_far;
        let mut head = [0u8; 12];
        head.copy_from_slice(&segment[..12]);
        (cost, head, so_far.to_bits())
    }

    /// The order the fix gives `indices` into `segments` (24 bytes each):
    /// by cost, then the key; and how many neighbours tie on cost.
    pub fn order(indices: &mut [i32], segments: &[u8]) -> Result<u64, &'static str> {
        let count = segments.len() / SEGMENT_LEN;
        if indices.iter().any(|&i| i < 0 || i as usize >= count) {
            return Err("an index past the segments");
        }
        let seg = |i: i32| &segments[i as usize * SEGMENT_LEN..(i as usize + 1) * SEGMENT_LEN];
        indices.sort_by(|&a, &b| {
            let (ka, kb) = (key(seg(a)), key(seg(b)));
            ka.0.partial_cmp(&kb.0)
                .unwrap_or_else(|| ka.0.total_cmp(&kb.0))
                // The edge key (entity, index, direction byte), the cost so
                // far, then the rest (padding in the vehicles' searches).
                .then(ka.1[..9].cmp(&kb.1[..9]))
                .then(ka.2.cmp(&kb.2))
                .then(ka.1[9..].cmp(&kb.1[9..]))
        });
        Ok(indices
            .windows(2)
            .filter(|p| key(seg(p[0])).0 == key(seg(p[1])).0)
            .count() as u64)
    }

    pub fn install(resolved: &ResolvedProfile, wanted: bool) -> Outcome {
        let off = |reason: String| Outcome {
            fix: FIX,
            installed: false,
            reason,
        };
        if !wanted {
            return off(format!("{TOGGLE_ENV} says so; the engine's sort stands"));
        }
        let Some(site) = resolved.get(CALL_SITE) else {
            return off(format!("the profile has no {CALL_SITE:?}"));
        };
        let read = |at: u64, len: usize| -> Option<Vec<u8>> {
            let at = usize::try_from(at).ok()?;
            if !crate::image::readable(at, len) {
                return None;
            }
            // SAFETY: `len` readable bytes at `at`, in the game's image.
            Some(unsafe { std::slice::from_raw_parts(at as *const u8, len) }.to_vec())
        };
        let Some(call) = read(site.address, 5) else {
            return off("the sort's call is unreadable".into());
        };
        if call[0] != 0xE8 {
            return off(format!("no call at {:#x}", site.address));
        }
        let rel = i32::from_le_bytes([call[1], call[2], call[3], call[4]]);
        let target = site
            .address
            .wrapping_add(5)
            .wrapping_add_signed(i64::from(rel));
        if read(target, SORT_EXPECTED.len()).as_deref() != Some(&SORT_EXPECTED[..])
            || read(target + COMPARATOR_AT, COMPARATOR_EXPECTED.len()).as_deref()
                != Some(&COMPARATOR_EXPECTED[..])
        {
            return off(format!("the sort at {target:#x} is not the shape expected"));
        }
        // SAFETY: a function of the game whose code was just checked,
        // detoured before any world exists; the detour has its ABI (four
        // integer arguments, no return value).
        match unsafe { InlineDetour::install(target as usize as *mut u8, sort as *const u8) } {
            Ok(detoured) => {
                ORIGINAL.store(detoured.trampoline() as usize, Ordering::Release);
                let _kept = std::mem::ManuallyDrop::new(detoured);
                Outcome {
                    fix: FIX,
                    installed: true,
                    reason: format!(
                        "the path finder's segment sort at {target:#x}: equal costs in one order, by the segment's key"
                    ),
                }
            }
            Err(error) => off(format!("the sort at {target:#x}: {error}")),
        }
    }

    /// The detour: `(begin, end, depth limit, this)`, the indices
    /// `[begin, end)` into the search's segments at `[this+8]..[this+0x10]`.
    unsafe extern "system" fn sort(begin: *mut i32, end: *mut i32, depth: isize, this: usize) {
        let mut done = false;
        if !BROKEN.load(Ordering::Relaxed) {
            guarded(FIX, &BROKEN, || {
                let n = CALLS.fetch_add(1, Ordering::Relaxed) + 1;
                let (begin_at, end_at) = (begin as usize, end as usize);
                if end_at < begin_at || (end_at - begin_at) % 4 != 0 {
                    REFUSED.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                let count = (end_at - begin_at) / 4;
                if count < 2 {
                    done = true;
                    return;
                }
                // SAFETY: the search's segment vector, live while it sorts.
                let (seg_begin, seg_end) = unsafe {
                    (
                        std::ptr::read((this + 8) as *const usize),
                        std::ptr::read((this + 0x10) as *const usize),
                    )
                };
                if seg_end < seg_begin
                    || (seg_end - seg_begin) % SEGMENT_LEN != 0
                    || (seg_end - seg_begin) / SEGMENT_LEN > MAX_SEGMENTS
                {
                    REFUSED.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                // SAFETY: the engine's own index range and segment vector,
                // which only this thread's search touches while it sorts.
                let (indices, segments) = unsafe {
                    (
                        std::slice::from_raw_parts_mut(begin, count),
                        std::slice::from_raw_parts(seg_begin as *const u8, seg_end - seg_begin),
                    )
                };
                match order(indices, segments) {
                    Ok(ties) => {
                        TIES.fetch_add(ties, Ordering::Relaxed);
                        done = true;
                    }
                    Err(_) => {
                        REFUSED.fetch_add(1, Ordering::Relaxed);
                    }
                }
                if n == 1 || n.is_multiple_of(1 << 20) {
                    log::line(&format!(
                        "order fix {FIX}: alive, sorts={n} ties={} refused={}",
                        TIES.load(Ordering::Relaxed),
                        REFUSED.load(Ordering::Relaxed)
                    ));
                }
            });
        }
        if done {
            return;
        }
        let original = ORIGINAL.load(Ordering::Acquire);
        if original != 0 {
            // SAFETY: the trampoline of the engine's sort, its arguments
            // forwarded.
            let original: SortFn = unsafe { std::mem::transmute::<usize, SortFn>(original) };
            unsafe { original(begin, end, depth, this) };
        }
    }
}

/// The ship and aircraft move systems' watcher (logging only; it changes
/// nothing). Both `Update2`s (`ecs::ShipMoveSystem` `0xaf6120`,
/// `ecs::AircraftMoveSystem` `0xa83a40`) walk their node list (16-byte
/// records `{entity, ., component index, .}` at `[[this+8]]`, `rdx` the byte
/// offset, reloaded every iteration) in its own order and claim water or
/// air space through a per-update reservation manager as they go: the
/// survey's item 5, measured but not fixed, so an earlier ship takes what a
/// later one wanted. At each loop head (right after the list's begin is
/// loaded, before `add <begin>, rdx`):
///
/// - at the first record, the list's order: one line when it changed for
///   that system object, with a hash of the entity ids in list order and
///   whether they are in entity order;
/// - for the entities `TPF3MP_HOOK_WATCH_ENTITIES` lists, every update, the
///   vehicle's 0x238-byte movement component (`[[this+0x18]] + index *
///   0x238`, the one `TransportVehicleSystem` reads the platform-decision
///   flag of: `+0x1b8` for ships, `+0xa0`/`+0xb0` for aircraft) in hex.
pub mod nodes {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    pub const FIX: &str = "node-watch";
    pub const SHIP_SITE: &str = "ecs::ShipMoveSystem::Update2/node head";
    pub const SHIP_EXPECTED: [u8; 7] = [
        0x48, 0x03, 0xDA, // add rbx, rdx
        0x49, 0x8B, 0x56, 0x10, // mov rdx, [r14+0x10]
    ];
    pub const AIRCRAFT_SITE: &str = "ecs::AircraftMoveSystem::Update2/node head";
    pub const AIRCRAFT_EXPECTED: [u8; 7] = [
        0x48, 0x03, 0xFA, // add rdi, rdx
        0x49, 0x8B, 0x46, 0x10, // mov rax, [r14+0x10]
    ];
    pub const STEAL: usize = 7;
    /// The loops' record count, a 64-bit stack slot.
    const SHIP_COUNT: u64 = 0x188;
    const AIRCRAFT_COUNT: u64 = 0x1f8;
    pub const RECORD_LEN: u64 = 16;
    pub const COMPONENT_LEN: u64 = 0x238;
    const MAX_NODES: u64 = 1 << 20;

    static BROKEN: AtomicBool = AtomicBool::new(false);

    thread_local! {
        /// The last order said per system object, and the objects' numbers.
        static ORDERS: RefCell<HashMap<u64, (u64, u64, bool)>> = RefCell::new(HashMap::new());
    }

    /// The order line.
    pub fn order_line(
        step: u64,
        system: &str,
        count: u64,
        hash: u64,
        sorted: bool,
        first: &[i32],
    ) -> String {
        let mut text = format!(
            "nodes: step {step} {system} n={count} order={hash:016x} in entity order: {}",
            if sorted { "yes" } else { "no" }
        );
        if !sorted {
            text.push_str(", first");
            for id in first {
                text.push_str(&format!(" {id}"));
            }
        }
        text
    }

    /// The component line.
    pub fn component_line(step: u64, system: &str, entity: i32, words: &[u32]) -> String {
        let mut text = format!("nodes: step {step} {system} vehicle {entity} component");
        for word in words {
            text.push_str(&format!(" {word:08x}"));
        }
        text
    }

    pub fn install(resolved: &ResolvedProfile) -> Vec<Outcome> {
        vec![
            splice(resolved, SHIP_SITE, &SHIP_EXPECTED, ship_hook),
            splice(resolved, AIRCRAFT_SITE, &AIRCRAFT_EXPECTED, aircraft_hook),
        ]
    }

    fn splice(
        resolved: &ResolvedProfile,
        name: &str,
        expected: &[u8],
        hook: tpf3mp_hookcore::detour::SpliceHook,
    ) -> Outcome {
        let off = |reason: String| Outcome {
            fix: FIX,
            installed: false,
            reason,
        };
        let Some(site) = resolved.get(name) else {
            return off(format!("the profile has no {name:?}"));
        };
        // SAFETY: a site inside a function the profile resolved and
        // prologue-checked, installed before any world exists; nothing
        // branches into the stolen bytes past the first (tpfre, noted in
        // the profile); the hook only reads and never unwinds (`guarded`).
        match unsafe { Splice::install(site.address as usize as *mut u8, expected, STEAL, hook) } {
            Ok(splice) => {
                let _kept = std::mem::ManuallyDrop::new(splice);
                Outcome {
                    fix: FIX,
                    installed: true,
                    reason: format!(
                        "{name} at {:#x}: the node list's order, and watched vehicles' movement component, are logged (logging only)",
                        site.address
                    ),
                }
            }
            Err(error) => off(format!("{name} at {:#x}: {error}", site.address)),
        }
    }

    pub(super) unsafe extern "system" fn ship_hook(regs: *mut SavedRegs) {
        let rsp = SavedRegs::rsp(regs);
        // SAFETY: the stub's block, held until the hook returns.
        let regs = unsafe { &*regs };
        watch("ships", regs.r14, regs.rbx, regs.rdx, rsp + SHIP_COUNT);
    }

    pub(super) unsafe extern "system" fn aircraft_hook(regs: *mut SavedRegs) {
        let rsp = SavedRegs::rsp(regs);
        // SAFETY: the stub's block, held until the hook returns.
        let regs = unsafe { &*regs };
        watch(
            "aircraft",
            regs.r14,
            regs.rdi,
            regs.rdx,
            rsp + AIRCRAFT_COUNT,
        );
    }

    fn watch(system: &str, this: u64, begin: u64, offset: u64, count_at: u64) {
        guarded(FIX, &BROKEN, || {
            if !in_step() {
                return;
            }
            let Some(step) = crate::seeds::current_step() else {
                return;
            };
            let mut probe = Probe::new();
            if offset == 0 {
                let Some(count) = probe.read::<u64>(count_at) else {
                    return;
                };
                if count > MAX_NODES {
                    return;
                }
                let mut hash = Fnv1a::new();
                let mut sorted = true;
                let mut last = i32::MIN;
                let mut first = Vec::new();
                for i in 0..count {
                    let Some(entity) = probe.read::<i32>(begin + i * RECORD_LEN) else {
                        return;
                    };
                    hash.write_u32(entity as u32);
                    if i > 0 && entity <= last {
                        sorted = false;
                    }
                    last = entity;
                    if first.len() < 12 {
                        first.push(entity);
                    }
                }
                let now = (count, hash.0, sorted);
                let changed =
                    ORDERS.with(|orders| orders.borrow_mut().insert(this, now) != Some(now));
                if changed {
                    log::line(&order_line(step, system, count, hash.0, sorted, &first));
                }
            }
            let record = begin + offset;
            let Some(entity) = probe.read::<i32>(record) else {
                return;
            };
            if !claims::watched(entity) {
                return;
            }
            let (Some(index), Some(base)) = (
                probe.read::<i32>(record + 8),
                this.checked_add(0x18)
                    .and_then(|at| probe.read::<u64>(at))
                    .and_then(|at| probe.read::<u64>(at)),
            ) else {
                return;
            };
            let Ok(index) = u64::try_from(index) else {
                return;
            };
            let component = base + index * COMPONENT_LEN;
            let mut words = Vec::with_capacity((COMPONENT_LEN / 4) as usize);
            for at in (0..COMPONENT_LEN).step_by(4) {
                let Some(word) = probe.read::<u32>(component + at) else {
                    return;
                };
                words.push(word);
            }
            log::line(&component_line(step, system, entity, &words));
        });
    }
}

/// The vehicles on a road or track edge (the survey's item 3; TPF2's
/// `roadentries`). `transport::EdgeUseManager` keeps, per edge, a vector
/// of 20-byte entries `{int32 entity, int32 component, float back, float
/// front, bool forward}`, and its nearest-occupant searches (`0x255f340`,
/// `0x255ef60`) keep the first entry on an exact tie, so two games whose
/// entries are in different orders can pick a different leader. Every
/// writer, read in the binary:
///
/// - `Add` (`0x255e940`, persons, from `PersonMoveSystem`'s node-added
///   callback) and `AddRange` (`0x255cc70`, vehicles, from
///   `LandVehicleMoveSystem`'s): `push_back`, or an in-place update of an
///   entry the vehicle already has;
/// - `Remove` (`0x2561440`) and `RemoveRange` (`0x2561690`): find, then
///   `memmove` the tail down: order kept;
/// - `RemoveEntity` (`0x2561510`): drops a whole edge;
/// - `GetOrAddEdgeData`: grows an edge list, moving the vectors whole.
///
/// So the entries are append order, which is history while running and
/// registration order after a load; nothing else reorders them. Sorting
/// each touched edge's entries by entity id right after every append keeps
/// every list canonical with no cost per update: the fix detours `Add` and
/// `AddRange` whole, runs the engine's, then sorts the edges it touched
/// (`Add`'s one edge; `AddRange`'s path edges `from..=to`).
///
/// The two appenders take the manager's data differently: `Add`'s `this`
/// is the manager, whose data is at `[this+0x18]` (`Add` gets it through
/// the copy-on-write getter `0x255f0b0`, which answers `[this+0x18]`);
/// `AddRange`'s `this` is that data already (its one caller, `0x255edc0`,
/// calls the getter and passes its answer), with the manager as its ninth
/// argument. Until 2026-09-30 the fix read `[this+0x18]` for both, which
/// in `AddRange` is the data's slot vector, so every vehicle append was
/// refused with "the edge's entity has no slot" (70% of the appends in the
/// three-player playtest) and the vehicles' lists were never sorted.
///
/// Cheap per append: the eight words from the data to an edge's entries
/// are checked through the per-thread region cache
/// ([`crate::image::Readable`]), so a region is asked of the system about
/// once per update, not once a word; a list that was in order before the
/// append needs one scan and, at most, the new entry moved into place
/// ([`place`]); nothing is allocated unless a list was out of order.
pub mod road {
    use super::*;

    pub const FIX: &str = "road-entry-order";
    /// Set to `0` (or `off`), the entries keep the engine's order (the
    /// measurement still installs the detours when it is on).
    pub const TOGGLE_ENV: &str = "TPF3MP_HOOK_ROAD_ENTRY_ORDER";
    pub const ADD: &str = measure::EDGE_USE_ADD;
    pub const ADD_RANGE: &str = measure::EDGE_USE_ADD_RANGE;
    /// One entry: the entity id first.
    pub const ENTRY_LEN: u64 = 20;
    /// One edge's data: its length (a float), then the entries vector.
    pub const EDGE_DATA_LEN: u64 = 32;
    /// One edge entity's slot: its edges vector first.
    pub const SLOT_LEN: u64 = 72;
    /// An `EdgeId`: entity, index, direction.
    pub const EDGE_ID_LEN: u64 = 12;
    pub const MAX_ENTRIES: u64 = 1 << 16;
    pub const MAX_PATH: u64 = 1 << 20;

    static SORTING: AtomicBool = AtomicBool::new(false);
    static BROKEN: AtomicBool = AtomicBool::new(false);
    static REFUSALS: Refusals = Refusals::new();
    static CALLS: AtomicU64 = AtomicU64::new(0);
    static REORDERS: AtomicU64 = AtomicU64::new(0);
    static ADD_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static ADD_RANGE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

    /// `Add(this, &edgeId, entity, component, {back, front})`, five
    /// arguments; three more slots are forwarded, unused.
    type AddFn =
        unsafe extern "system" fn(usize, usize, usize, usize, usize, usize, usize, usize) -> usize;
    /// `AddRange(this, entity, component, &pathEdges, currentIndex,
    /// {back, front}, from, to, context)`: nine arguments (the last at the
    /// caller's `[rsp+0x48]`, read as `[rbp+0x140]` in the function).
    type AddRangeFn = unsafe extern "system" fn(
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
        usize,
    ) -> usize;

    /// Installs the two detours when the fix is wanted or the measurement
    /// is on (the `appends` lane is fed from here).
    pub fn install(resolved: &ResolvedProfile, wanted: bool, measuring: bool) -> Vec<Outcome> {
        if !wanted && !measuring {
            return vec![Outcome {
                fix: FIX,
                installed: false,
                reason: format!(
                    "{TOGGLE_ENV} says so and nothing is measured; the entries keep the engine's order"
                ),
            }];
        }
        let mut outcomes = Vec::new();
        let targets: [(&str, *const u8, &AtomicUsize); 2] = [
            (ADD, add as *const u8, &ADD_ORIGINAL),
            (ADD_RANGE, add_range as *const u8, &ADD_RANGE_ORIGINAL),
        ];
        let mut installed = 0;
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
            // detoured before any world exists; each detour has the target's
            // ABI with every argument forwarded (integer and pointer
            // registers; the floats ride in stack slots, forwarded whole).
            match unsafe { InlineDetour::install(target.address as usize as *mut u8, detour) } {
                Ok(detoured) => {
                    original.store(detoured.trampoline() as usize, Ordering::Release);
                    let _kept = std::mem::ManuallyDrop::new(detoured);
                    installed += 1;
                    outcomes.push(Outcome {
                        fix: FIX,
                        installed: wanted,
                        reason: format!(
                            "{name} at {:#x} detoured: {}",
                            target.address,
                            if wanted {
                                "the edge lists it appends to are kept in entity order"
                            } else {
                                "measured only"
                            }
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
        // Both appenders or neither: one sorted and one not is no order.
        if wanted && installed == 2 {
            SORTING.store(true, Ordering::Release);
        } else if wanted {
            outcomes.push(Outcome {
                fix: FIX,
                installed: false,
                reason: "not both appenders are detoured; the entries keep the engine's order"
                    .to_owned(),
            });
        }
        outcomes
    }

    /// Where `Add`'s manager keeps its data (`EdgeUseManagerData*`).
    pub const MANAGER_DATA: u64 = 0x18;

    /// One entry, as the engine lays it out.
    pub type Entry = [u8; ENTRY_LEN as usize];

    /// An entry's entity id: its first dword.
    #[inline]
    pub fn key(entry: &Entry) -> i32 {
        i32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]])
    }

    /// The order the fix keeps: the entries by entity id, each whole;
    /// `None` when they already are in it. The reference for [`place`],
    /// which the hook runs.
    pub fn entry_order(entries: &[Entry]) -> Result<Option<Vec<Entry>>, &'static str> {
        let keys: Vec<i32> = entries.iter().map(key).collect();
        Ok(canonical_permutation(&keys)?.map(|order| order.iter().map(|&i| entries[i]).collect()))
    }

    /// Puts `entries` in the order [`entry_order`] gives, in place, and
    /// writes nothing when it refuses. One scan finds how far the list is
    /// strictly ascending. A list sorted before the append (every list the
    /// fix has kept) is either whole, or out of order only in its last
    /// entry, the one appended: that entry's place is found by binary
    /// search and the tail after it moves up one. Anything else (a list the
    /// fix never kept) is sorted whole through `scratch`, reused from call
    /// to call, and written back only when no two entries name one entity.
    pub fn place(entries: &mut [Entry], scratch: &mut Vec<Entry>) -> Result<Sorted, &'static str> {
        let n = entries.len();
        let mut prefix = 1;
        while prefix < n && key(&entries[prefix - 1]) < key(&entries[prefix]) {
            prefix += 1;
        }
        if prefix >= n {
            return Ok(Sorted::Unchanged);
        }
        if prefix == n - 1 {
            let last = key(&entries[n - 1]);
            return match entries[..n - 1].binary_search_by_key(&last, key) {
                Ok(_) => Err("two entries name one entity"),
                Err(at) => {
                    entries[at..].rotate_right(1);
                    Ok(Sorted::Reordered)
                }
            };
        }
        scratch.clear();
        scratch.extend_from_slice(entries);
        // Unique keys (checked next) are a total order: stable or not, one
        // result.
        scratch.sort_unstable_by_key(key);
        if scratch
            .windows(2)
            .any(|pair| key(&pair[0]) == key(&pair[1]))
        {
            return Err("two entries name one entity");
        }
        entries.copy_from_slice(scratch);
        Ok(Sorted::Reordered)
    }

    thread_local! {
        /// [`place`]'s buffer for a list out of order in more than its last
        /// entry.
        static SCRATCH: std::cell::RefCell<Vec<Entry>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    /// The entries vector (`&begin`) of the edge `edge_id` names, through
    /// the manager's data (`EdgeUseManagerData*`) the way `GetOrAddEdgeData`
    /// (`0x255d5a0`, `0x255d740`) walks it: its entity-to-slot index
    /// `[data+0]..[data+8]` (int32s); its slots `[data+0x18]..[data+0x20]`
    /// (72 bytes each); the slot's edges `[slot]..[slot+8]` (32 bytes each);
    /// the entries at `edge+8`.
    pub(super) fn entries_of(
        probe: &mut Probe,
        data: u64,
        edge_id: u64,
    ) -> Result<u64, &'static str> {
        let (entity, index): (i32, i32) = (
            probe.read(edge_id).ok_or("the edge id is unreadable")?,
            edge_id
                .checked_add(4)
                .and_then(|at| probe.read(at))
                .ok_or("the edge id is unreadable")?,
        );
        let entity = u64::try_from(entity).map_err(|_| "a negative edge entity")?;
        let index = u64::try_from(index).map_err(|_| "a negative edge index")?;
        let slots_of: u64 = probe.read(data).ok_or("the slot index is unreadable")?;
        let slots_end: u64 = data
            .checked_add(8)
            .and_then(|at| probe.read(at))
            .ok_or("the slot index is unreadable")?;
        if slots_end < slots_of || entity >= (slots_end - slots_of) / 4 {
            return Err("the edge's entity has no slot");
        }
        let slot: i32 = probe
            .read(slots_of + entity * 4)
            .ok_or("the slot index is unreadable")?;
        let slot = u64::try_from(slot).map_err(|_| "the edge's entity has no slot")?;
        let slots: u64 = data
            .checked_add(0x18)
            .and_then(|at| probe.read(at))
            .ok_or("the slots are unreadable")?;
        let slots_last: u64 = data
            .checked_add(0x20)
            .and_then(|at| probe.read(at))
            .ok_or("the slots are unreadable")?;
        if slots_last < slots
            || !(slots_last - slots).is_multiple_of(SLOT_LEN)
            || slot >= (slots_last - slots) / SLOT_LEN
        {
            return Err("the slots are not whole slots");
        }
        let at = slots + slot * SLOT_LEN;
        let edges: u64 = probe.read(at).ok_or("the slot's edges are unreadable")?;
        let edges_end: u64 = probe
            .read(at + 8)
            .ok_or("the slot's edges are unreadable")?;
        if edges_end < edges
            || !(edges_end - edges).is_multiple_of(EDGE_DATA_LEN)
            || index >= (edges_end - edges) / EDGE_DATA_LEN
        {
            return Err("the edge index is past the slot's edges");
        }
        Ok(edges + index * EDGE_DATA_LEN + 8)
    }

    /// Sorts the entries of the edge `edge_id` names by entity id, in place,
    /// in the manager's data `data`.
    pub(crate) fn sort_edge(
        probe: &mut Probe,
        data: u64,
        edge_id: u64,
    ) -> Result<Sorted, &'static str> {
        let vector = entries_of(probe, data, edge_id)?;
        let begin: u64 = probe.read(vector).ok_or("the entries are unreadable")?;
        let end: u64 = vector
            .checked_add(8)
            .and_then(|at| probe.read(at))
            .ok_or("the entries are unreadable")?;
        if end < begin || !(end - begin).is_multiple_of(ENTRY_LEN) {
            return Err("the entries are not whole entries");
        }
        let count = (end - begin) / ENTRY_LEN;
        if count > MAX_ENTRIES {
            return Err("more entries on one edge than any world holds");
        }
        let len = usize::try_from(count * ENTRY_LEN).map_err(|_| "too many entries")?;
        let begin = usize::try_from(begin).map_err(|_| "the entries are unreadable")?;
        if !probe.readable(begin, len) {
            return Err("the entries are unreadable");
        }
        let entries: &mut [Entry] = if count == 0 {
            &mut []
        } else {
            // SAFETY: `len` readable bytes at `begin` (not null: readable),
            // whole 20-byte entries of alignment 1; on the thread that just
            // appended to them, inside the engine's own call chain (the
            // node-added callbacks run serially at the end of a
            // modification), so nothing else reads or writes them while the
            // slice lives.
            unsafe { std::slice::from_raw_parts_mut(begin as *mut Entry, count as usize) }
        };
        let sorted = SCRATCH.with(|scratch| place(entries, &mut scratch.borrow_mut()))?;
        if measure::enabled() {
            let edge: [u8; EDGE_ID_LEN as usize] =
                probe.read(edge_id).ok_or("the edge id is unreadable")?;
            let ids: Vec<i32> = entries.iter().map(key).collect();
            measure::note_road(&edge, &ids, sorted);
        }
        Ok(sorted)
    }

    /// For the tests: the detours call `add` and `add_range` as the
    /// engine's, and sort after them.
    #[cfg(test)]
    pub(super) fn arm_for_test(add: usize, add_range: usize) {
        ADD_ORIGINAL.store(add, Ordering::Release);
        ADD_RANGE_ORIGINAL.store(add_range, Ordering::Release);
        SORTING.store(add != 0 || add_range != 0, Ordering::Release);
    }

    /// The road fix's refusals by reason since the last call (the `perf:`
    /// line's).
    pub fn take_refusals() -> Vec<(&'static str, u64)> {
        REFUSALS.take_window()
    }

    /// The appends seen inside the game's step, and outside it: those inside
    /// are the simulation's own and come in one order in every game; the
    /// others (the second engine's copy, made from the frame) come when the
    /// frames do, so the counters taken together jitter from game to game
    /// without anything having diverged.
    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    pub struct ThreadCounts {
        pub step_appends: u64,
        pub step_reorders: u64,
        pub other_appends: u64,
        pub other_reorders: u64,
    }

    static STEP_CALLS: AtomicU64 = AtomicU64::new(0);
    static STEP_REORDERS: AtomicU64 = AtomicU64::new(0);
    static OTHER_CALLS: AtomicU64 = AtomicU64::new(0);
    static OTHER_REORDERS: AtomicU64 = AtomicU64::new(0);

    /// The counts so far.
    pub fn thread_counts() -> ThreadCounts {
        ThreadCounts {
            step_appends: STEP_CALLS.load(Ordering::Relaxed),
            step_reorders: STEP_REORDERS.load(Ordering::Relaxed),
            other_appends: OTHER_CALLS.load(Ordering::Relaxed),
            other_reorders: OTHER_REORDERS.load(Ordering::Relaxed),
        }
    }

    /// The in-step milestone line: one per 65536 appends inside the game's
    /// step, the line two games' logs must agree on (the others' counts are
    /// not in it).
    pub fn step_line(appends: u64, reorders: u64) -> String {
        format!("order fix {FIX}: in-step appends={appends} reordered={reorders}")
    }

    fn sorted(probe: &mut Probe, data: u64, edge_ids: impl Iterator<Item = u64>) {
        guarded(FIX, &BROKEN, || {
            let n = CALLS.fetch_add(1, Ordering::Relaxed) + 1;
            let on_step = in_step();
            let (calls, reorders_here) = if on_step {
                (&STEP_CALLS, &STEP_REORDERS)
            } else {
                (&OTHER_CALLS, &OTHER_REORDERS)
            };
            let here = calls.fetch_add(1, Ordering::Relaxed) + 1;
            for edge_id in edge_ids {
                match sort_edge(probe, data, edge_id) {
                    Ok(Sorted::Reordered) => {
                        reorders_here.fetch_add(1, Ordering::Relaxed);
                        let reorders = REORDERS.fetch_add(1, Ordering::Relaxed) + 1;
                        if reorders <= 3 {
                            log::line(&format!(
                                "order fix {FIX}: an edge's entries put in entity order (append #{n})"
                            ));
                        }
                    }
                    Ok(Sorted::Unchanged) => {}
                    Err(why) => REFUSALS.note(FIX, why),
                }
            }
            if on_step && here.is_multiple_of(1 << 16) {
                log::line(&step_line(here, reorders_here.load(Ordering::Relaxed)));
            }
            if n == 1 || n.is_multiple_of(1 << 16) {
                let counts = thread_counts();
                log::line(&format!(
                    "order fix {FIX}: alive, appends={n} reordered={} refused={} (in the step {}/{}, outside it {}/{})",
                    REORDERS.load(Ordering::Relaxed),
                    REFUSALS.count.load(Ordering::Relaxed),
                    counts.step_appends,
                    counts.step_reorders,
                    counts.other_appends,
                    counts.other_reorders,
                ));
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe extern "system" fn add(
        this: usize,
        edge_id: usize,
        entity: usize,
        component: usize,
        bounds: usize,
        s6: usize,
        s7: usize,
        s8: usize,
    ) -> usize {
        if measure::enabled() {
            measure::note_add(
                edge_id as u64,
                entity as u32,
                component as u32,
                bounds as u64,
            );
        }
        let original = ADD_ORIGINAL.load(Ordering::Acquire);
        if original == 0 {
            return 0;
        }
        // SAFETY: the trampoline of the function this detour replaced, every
        // argument forwarded.
        let original: AddFn = unsafe { std::mem::transmute::<usize, AddFn>(original) };
        let result = unsafe { original(this, edge_id, entity, component, bounds, s6, s7, s8) };
        if SORTING.load(Ordering::Acquire) {
            let _timer = perf::time(Piece::RoadEntry);
            let mut probe = Probe::new();
            // `Add`'s `this` is the manager: its data at `[this+0x18]`.
            match (this as u64)
                .checked_add(MANAGER_DATA)
                .and_then(|at| probe.read::<u64>(at))
            {
                Some(data) => sorted(&mut probe, data, std::iter::once(edge_id as u64)),
                None => REFUSALS.note(FIX, "the manager's data is unreadable"),
            }
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe extern "system" fn add_range(
        this: usize,
        entity: usize,
        component: usize,
        path: usize,
        current: usize,
        bounds: usize,
        from: usize,
        to: usize,
        context: usize,
    ) -> usize {
        if measure::enabled() {
            // The context is an address: not hashed.
            measure::note_add_range(
                [
                    entity as u32,
                    component as u32,
                    current as u32,
                    bounds as u32,
                    from as u32,
                    to as u32,
                ],
                path as u64,
            );
        }
        let original = ADD_RANGE_ORIGINAL.load(Ordering::Acquire);
        if original == 0 {
            return 0;
        }
        // SAFETY: as for `add`, the ninth argument included.
        let original: AddRangeFn = unsafe { std::mem::transmute::<usize, AddRangeFn>(original) };
        let result = unsafe {
            original(
                this, entity, component, path, current, bounds, from, to, context,
            )
        };
        if SORTING.load(Ordering::Acquire) {
            let _timer = perf::time(Piece::RoadEntry);
            let mut probe = Probe::new();
            let data = this as u64;
            // `AddRange`'s `this` is the manager's data itself; its ninth
            // argument, the manager, names it at `[+0x18]`.
            let checked = (context as u64)
                .checked_add(MANAGER_DATA)
                .and_then(|at| probe.read::<u64>(at))
                == Some(data);
            if !checked {
                REFUSALS.note(FIX, "AddRange's data is not its manager's");
            } else {
                match path_edges(
                    &mut probe,
                    path as u64,
                    from as u32 as i32,
                    to as u32 as i32,
                ) {
                    Ok(edges) => sorted(&mut probe, data, edges),
                    Err(why) => REFUSALS.note(FIX, why),
                }
            }
        }
        result
    }

    /// The addresses of `path[from..=to]`'s edge ids, checked against the
    /// path vector's bounds.
    fn path_edges(
        probe: &mut Probe,
        path: u64,
        from: i32,
        to: i32,
    ) -> Result<impl Iterator<Item = u64> + use<>, &'static str> {
        let (from, to) = (
            u64::try_from(from).map_err(|_| "a negative path range")?,
            u64::try_from(to).map_err(|_| "a negative path range")?,
        );
        let begin: u64 = probe.read(path).ok_or("the path is unreadable")?;
        let end: u64 = path
            .checked_add(8)
            .and_then(|at| probe.read(at))
            .ok_or("the path is unreadable")?;
        if end < begin || !(end - begin).is_multiple_of(EDGE_ID_LEN) {
            return Err("the path is not whole edge ids");
        }
        let len = (end - begin) / EDGE_ID_LEN;
        if to < from || to >= len || len > MAX_PATH {
            return Err("the path range is past the path");
        }
        Ok((from..=to).map(move |i| begin + i * EDGE_ID_LEN))
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
        /// The land-vehicle shuffle's seeds (the game's tickCount), one per
        /// call of the sort site.
        pub land_seeds: Fnv1a,
        /// The land-vehicle family's node list, entity by entity, in its
        /// own order (hashed where at least two vehicles want track): the
        /// order the engine's shuffle permutes, and the survey's item 3.
        pub land_nodes: Fnv1a,
        pub land_node_count: u64,
        pub vehicles_at_stop: Fnv1a,
        pub vehicle_stop_calls: u64,
        pub vehicle_stop_reorders: u64,
        /// The platform chooser's visit order as the engine had it
        /// ([`super::platform`]), one hash per update.
        pub visits: Fnv1a,
        pub visit_calls: u64,
        pub visit_reorders: u64,
        /// The terminal candidates put in canonical order before the cost
        /// sort, and how many of those sorts changed something.
        pub candidate_sorts: u64,
        pub candidate_reorders: u64,
        /// Each edge whose entries the road fix checked after an append
        /// ([`super::road`]): its id and its entities in the order kept.
        pub road: Fnv1a,
        pub road_sorts: u64,
        pub road_reorders: u64,
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
                land_seeds: Fnv1a::new(),
                land_nodes: Fnv1a::new(),
                land_node_count: 0,
                vehicles_at_stop: Fnv1a::new(),
                vehicle_stop_calls: 0,
                vehicle_stop_reorders: 0,
                visits: Fnv1a::new(),
                visit_calls: 0,
                visit_reorders: 0,
                candidate_sorts: 0,
                candidate_reorders: 0,
                road: Fnv1a::new(),
                road_sorts: 0,
                road_reorders: 0,
            }
        }

        /// One log line: the update it closes and every lane.
        pub fn line(&self, update: u64, interval: u64) -> String {
            format!(
                "order measure: updates {}..={update}: claims={:016x}/{} appends={:016x}/{} land={:016x}/{} reordered {} seeds={:016x} nodes={:016x}/{} vehstop={:016x}/{} reordered {} visits={:016x}/{} reordered {} candidates={}/{} road={:016x}/{} reordered {}",
                update.saturating_sub(interval.saturating_sub(1)),
                self.claims.0,
                self.claim_count,
                self.appends.0,
                self.append_count,
                self.land_vehicles.0,
                self.land_vehicle_calls,
                self.land_vehicle_reorders,
                self.land_seeds.0,
                self.land_nodes.0,
                self.land_node_count,
                self.vehicles_at_stop.0,
                self.vehicle_stop_calls,
                self.vehicle_stop_reorders,
                self.visits.0,
                self.visit_calls,
                self.visit_reorders,
                self.candidate_reorders,
                self.candidate_sorts,
                self.road.0,
                self.road_sorts,
                self.road_reorders,
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

    pub(super) fn note_land_nodes(order: impl Iterator<Item = u32>) {
        let mut lanes = lanes();
        for entity in order {
            lanes.land_nodes.write_u32(entity);
            lanes.land_node_count += 1;
        }
        lanes.land_nodes.write(b"|");
    }

    pub(super) fn note_land_vehicles(before: &[u32], sorted: Sorted, seed: u32) {
        if !enabled() {
            return;
        }
        let mut lanes = lanes();
        lanes.land_seeds.write_u32(seed);
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

    pub(super) fn note_visits(before: &[i32], sorted: Sorted) {
        if !enabled() {
            return;
        }
        let mut lanes = lanes();
        lanes.visit_calls += 1;
        lanes.visits.write_u32(before.len() as u32);
        for id in before {
            lanes.visits.write_u32(*id as u32);
        }
        if sorted == Sorted::Reordered {
            lanes.visit_reorders += 1;
        }
    }

    pub(super) fn note_candidates(sorted: Sorted) {
        if !enabled() {
            return;
        }
        let mut lanes = lanes();
        lanes.candidate_sorts += 1;
        if sorted == Sorted::Reordered {
            lanes.candidate_reorders += 1;
        }
    }

    /// One edge the road fix checked: its id and the entities on it in the
    /// order they are kept.
    pub(super) fn note_road(edge: &[u8], kept: &[i32], sorted: Sorted) {
        if !enabled() {
            return;
        }
        let mut lanes = lanes();
        lanes.road_sorts += 1;
        lanes.road.write(edge);
        lanes.road.write_u32(kept.len() as u32);
        for id in kept {
            lanes.road.write_u32(*id as u32);
        }
        if sorted == Sorted::Reordered {
            lanes.road_reorders += 1;
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
        // `EdgeUseManager::Add` and `AddRange` are detoured by the road
        // entry fix ([`super::road`]), which feeds the `appends` lane too.
        let targets: [(&str, *const u8, &AtomicUsize); 3] = [
            (ENGINE_UPDATE, engine_update as *const u8, &UPDATE_ORIGINAL),
            (RESERVE, reserve as *const u8, &RESERVE_ORIGINAL),
            (
                RESERVE_SIMPLE,
                reserve_simple as *const u8,
                &RESERVE_SIMPLE_ORIGINAL,
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
    fn being_inside_the_step_is_per_thread() {
        std::thread::spawn(|| {
            assert!(!in_step());
            set_in_step(true);
            assert!(in_step());
        })
        .join()
        .unwrap();
        std::thread::spawn(|| assert!(!in_step())).join().unwrap();
    }

    #[test]
    fn the_watch_and_step_lines_hold_only_what_agreeing_games_share() {
        let seen = platform::Seen {
            state: 2,
            line: 4711,
            stop: 1,
            station: 900,
            terminal: 3,
        };
        assert_eq!(
            platform::watch_line(3290, 0, 217708, seen),
            "watch: step 3290 engine 0 vehicle 217708 state 2 line 4711 stop 1 terminal 900/3"
        );
        assert_eq!(
            road::step_line(65536, 17000),
            "order fix road-entry-order: in-step appends=65536 reordered=17000"
        );
    }

    #[test]
    fn the_free_check_lines_name_the_holder_and_the_vehicles_on_the_edge() {
        let mut entry = [0u8; road::ENTRY_LEN as usize];
        entry[..4].copy_from_slice(&4711i32.to_le_bytes());
        entry[4..8].copy_from_slice(&3i32.to_le_bytes());
        entry[8..12].copy_from_slice(&1.5f32.to_le_bytes());
        entry[12..16].copy_from_slice(&13.25f32.to_le_bytes());
        entry[16] = 1;
        assert_eq!(
            platform::check_line(3201, 1, 217708, 0, 0, (900, 2, 1), 4711, Some(&[entry])),
            "watch: step 3201 engine 1 vehicle 217708 checks 0/0 edge 900/2/1 occupant 4711 entries 1 4711:3:1.5:13.25:1"
        );
        assert_eq!(
            platform::check_line(3201, 0, 217708, 0, 0, (900, 2, 1), -1, None),
            "watch: step 3201 engine 0 vehicle 217708 checks 0/0 edge 900/2/1 occupant -1 entries unreadable"
        );
        assert_eq!(
            platform::candidates_line(3201, 0, 217708, &[[7, 0, 0], [7, 0, 1]]),
            "watch: step 3201 engine 0 vehicle 217708 candidates 2 7/0/0 7/0/1"
        );
    }

    #[test]
    fn the_flags_are_copied_from_the_engine_before() {
        // Two engines (1 and 2), three vehicles; vehicle 9 has no MovePath
        // in engine 2.
        let mut paths = vec![[0u8; 0xa0]; 5];
        paths[0][0x70] = 1; // engine 1, vehicle 7
        paths[1][0x70] = 0; // engine 1, vehicle 8
        paths[2][0x70] = 0; // engine 2, vehicle 7
        paths[3][0x70] = 1; // engine 2, vehicle 8
        paths[4][0x70] = 1; // engine 1, vehicle 9
        let base = paths.as_mut_ptr() as usize;
        let at = |i: usize| base + i * 0xa0;
        let get = |engine: usize, vehicle: i32| match (engine, vehicle) {
            (1, 7) => at(0),
            (1, 8) => at(1),
            (2, 7) => at(2),
            (2, 8) => at(3),
            (1, 9) => at(4),
            _ => 0,
        };
        assert_eq!(decision_sync::copy_flags(1, 2, &[7, 8, 9], get), 2);
        assert_eq!((paths[2][0x70], paths[3][0x70]), (1, 0));
        // The engine before is left as it was; equal flags copy nothing.
        assert_eq!((paths[0][0x70], paths[1][0x70]), (1, 0));
        assert_eq!(decision_sync::copy_flags(1, 2, &[7, 8, 9], get), 0);
    }

    #[test]
    fn the_movepath_getter_is_taken_only_from_a_verified_call() {
        // A call at 0x1000 to 0x2000, whose lea names a descriptor at 0x3000.
        let mut memory = std::collections::HashMap::new();
        let call = 0x1000u64;
        let getter = 0x2000u64;
        let mut bytes = vec![0xE8];
        bytes.extend(((getter - (call + 5)) as i32).to_le_bytes());
        memory.insert(call, bytes);
        let mut head = decision_sync::GETTER_EXPECTED.to_vec();
        let after = getter + decision_sync::GETTER_EXPECTED.len() as u64 + 4;
        head.extend(((0x3000 - after) as i32).to_le_bytes());
        memory.insert(getter, head);
        memory.insert(0x3010, decision_sync::TYPE_NAME.to_vec());
        let reads = |memory: std::collections::HashMap<u64, Vec<u8>>| {
            move |at: u64, len: usize| memory.get(&at).map(|b| b[..len.min(b.len())].to_vec())
        };
        let mut read = reads(memory.clone());
        assert_eq!(decision_sync::getter_from_call(call, &mut read), Ok(getter));
        // Another type's getter, the same code otherwise: refused.
        let mut other = memory.clone();
        other.insert(
            0x3010,
            b".?AUMovePathAircraft@component@ecs@@\0"[..29].to_vec(),
        );
        let mut read = reads(other);
        assert!(decision_sync::getter_from_call(call, &mut read).is_err());
        // No call there: refused.
        let mut other = memory;
        other.insert(call, vec![0x90; 5]);
        let mut read = reads(other);
        assert!(decision_sync::getter_from_call(call, &mut read).is_err());
    }

    #[test]
    fn equal_cost_segments_come_out_in_one_order_whatever_the_input() {
        // Four segments: 0 and 2 tie on cost (1.0 + 2.0, 2.5 + 0.5), 1 is
        // cheaper, 3 dearer.
        let mut segments = vec![0u8; 4 * path_ties::SEGMENT_LEN];
        let put = |segments: &mut Vec<u8>, i: usize, key: i32, so_far: f32, h: f32| {
            let at = i * path_ties::SEGMENT_LEN;
            segments[at..at + 4].copy_from_slice(&key.to_le_bytes());
            segments[at + 0x0c..at + 0x10].copy_from_slice(&so_far.to_le_bytes());
            segments[at + 0x10..at + 0x14].copy_from_slice(&h.to_le_bytes());
        };
        put(&mut segments, 0, 362_201, 1.0, 2.0);
        put(&mut segments, 1, 5, 1.0, 0.0);
        put(&mut segments, 2, 362_200, 2.5, 0.5);
        put(&mut segments, 3, 1, 9.0, 0.0);
        let mut a = vec![0, 1, 2, 3];
        let mut b = vec![3, 2, 1, 0];
        assert_eq!(path_ties::order(&mut a, &segments), Ok(1));
        assert_eq!(path_ties::order(&mut b, &segments), Ok(1));
        // By cost first; the tie by the key (362200 before 362201).
        assert_eq!(a, vec![1, 2, 0, 3]);
        assert_eq!(a, b);
        assert!(path_ties::order(&mut [4], &segments).is_err());
    }

    #[test]
    fn the_path_lines_and_hashes_leave_the_padding_out() {
        let mut a = [0u8; 12];
        a[..4].copy_from_slice(&900i32.to_le_bytes());
        a[4..8].copy_from_slice(&2i32.to_le_bytes());
        a[8] = 1;
        let mut b = a;
        b[9..].copy_from_slice(&[0x20, 0x20, 0x20]);
        assert_eq!(platform::path_edge_bytes(&a), platform::path_edge_bytes(&b));
        assert_eq!(
            platform::path_line(3200, 1, 217708, &[(900, 2, 1), (901, 0, 0)]),
            "watch: step 3200 engine 1 vehicle 217708 path 2: 900/2/1 901/0/0"
        );
    }

    #[test]
    fn the_decision_watch_lines_carry_the_flag_and_the_path() {
        assert_eq!(
            platform::decision_line(3201, 0, 217708, 1),
            "watch: step 3201 engine 0 vehicle 217708 decision flag 1"
        );
        assert_eq!(
            platform::movepath_line(3200, 1, 217708, 12, 0xabc, &[0x3f80_0000]),
            "watch: step 3200 engine 1 vehicle 217708 movepath path 12/0000000000000abc 3f800000"
        );
    }

    #[test]
    fn the_node_watch_lines_name_the_order_and_the_component() {
        assert_eq!(
            nodes::order_line(1, "ships", 3, 0xab, false, &[30, 10, 20]),
            "nodes: step 1 ships n=3 order=00000000000000ab in entity order: no, first 30 10 20"
        );
        assert_eq!(
            nodes::order_line(1, "aircraft", 2, 0xab, true, &[10, 20]),
            "nodes: step 1 aircraft n=2 order=00000000000000ab in entity order: yes"
        );
        assert_eq!(
            nodes::component_line(3200, "ships", 217708, &[1, 0x3f80_0000]),
            "nodes: step 3200 ships vehicle 217708 component 00000001 3f800000"
        );
        assert_eq!(
            platform::transport_line(3200, 1, 217708, &[4, 2]),
            "watch: step 3200 engine 1 vehicle 217708 transport 00000004 00000002"
        );
    }

    #[test]
    fn the_claim_watch_lines_carry_the_vehicles_state_in_full() {
        let watched = claims::parse_entities(Some("217708, 4711;x 12"));
        assert_eq!(watched, [217708, 4711, 12].into_iter().collect());
        assert!(claims::parse_entities(None).is_empty());
        assert_eq!(
            claims::head_line(3200, 217708, 0.5, &[0x3f80_0000, 7], 12, 0xabc),
            "claim: step 3200 vehicle 217708 priority 0.5 path 12/0000000000000abc movepath 3f800000 00000007"
        );
        assert_eq!(
            claims::decision_line(3201, 217708, true, 9, 8),
            "claim: step 3201 vehicle 217708 terminal decision 1 (claimed to 9, decided at 8)"
        );
    }

    #[test]
    fn a_free_check_is_said_when_its_answer_changes_and_a_free_edge_only_after_a_held_one() {
        let mut checks = platform::Checks::default();
        let edge = (900, 2, 1);
        // Free from the start: nothing to say.
        assert!(!checks.note(0, 7, edge, -1, vec![]));
        // Held: said once, then quiet while it stays held by the same.
        assert!(checks.note(0, 7, edge, 4711, vec![4711]));
        assert!(!checks.note(0, 7, edge, 4711, vec![4711]));
        // Another vehicle on it, or another engine: said.
        assert!(checks.note(0, 7, edge, 4711, vec![4711, 4712]));
        assert!(checks.note(1, 7, edge, 4711, vec![4711]));
        // Freed: said once, then quiet again.
        assert!(checks.note(0, 7, edge, -1, vec![]));
        assert!(!checks.note(0, 7, edge, -1, vec![]));
    }

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
        assert_eq!(
            prologue(platform::VISIT_SITE),
            platform::VISIT_EXPECTED.to_vec()
        );
        assert_eq!(
            prologue(platform::CANDIDATES_SITE),
            platform::CANDIDATES_EXPECTED.to_vec()
        );
        assert_eq!(
            prologue(platform::OCCUPANT_SITE),
            platform::OCCUPANT_EXPECTED.to_vec()
        );
        assert_eq!(prologue(claims::HEAD_SITE), claims::HEAD_EXPECTED.to_vec());
        assert_eq!(
            prologue(platform::DECISION_SITE),
            platform::DECISION_EXPECTED.to_vec()
        );
        assert_eq!(prologue(nodes::SHIP_SITE), nodes::SHIP_EXPECTED.to_vec());
        assert_eq!(
            prologue(nodes::AIRCRAFT_SITE),
            nodes::AIRCRAFT_EXPECTED.to_vec()
        );
        assert_eq!(
            prologue(claims::DECISION_SITE),
            claims::DECISION_EXPECTED.to_vec()
        );
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
        assert!(line.contains(" seeds="));
        assert!(line.contains(" nodes="));
        assert!(line.contains("vehstop="));
    }

    #[test]
    fn the_land_vehicle_sample_is_chosen_by_the_seeds_value_and_hashes_the_sorted_ids() {
        assert_eq!(land_vehicle::sample_line(255, &[3, 1]), None);
        assert_eq!(land_vehicle::sample_line(257, &[3, 1]), None);
        let line = land_vehicle::sample_line(512, &[30, 10, 20]).unwrap();
        assert!(
            line.starts_with("order fix land-vehicle-order: sample seed=512 n=3 ids="),
            "{line}"
        );
        // The engine's order before the sort does not change the line: two
        // games whose node lists differ in order, and that the fix puts in
        // one order, log the same.
        assert_eq!(
            land_vehicle::sample_line(512, &[10, 20, 30]),
            Some(line.clone())
        );
        let mut hash = Fnv1a::new();
        for id in [10u32, 20, 30] {
            hash.write_u32(id);
        }
        assert!(line.ends_with(&format!("{:016x}", hash.0)), "{line}");
        assert_ne!(land_vehicle::sample_line(512, &[10, 20]), Some(line));
        assert!(
            land_vehicle::sample_line(0, &[])
                .unwrap()
                .contains("seed=0 n=0 ")
        );
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
            land_vehicle::install(&resolved, true),
            terminal::install(&resolved, true),
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
        assert_eq!(on.len(), 3, "one outcome per measured function");
        assert!(
            on.iter()
                .all(|o| !o.installed && o.reason.contains("the profile has no"))
        );
        // The platform fix: both sites and the watcher's free check (the
        // watcher is on unless its switch says otherwise), each off on its
        // own; and its switch.
        let platform = platform::install(&resolved, true);
        assert_eq!(platform.len(), 4);
        assert!(platform.iter().all(|o| !o.installed));
        let switched = platform::install(&resolved, false);
        assert!(switched[0].reason.contains(platform::TOGGLE_ENV));
        // The road fix: no appenders, no sorting; switched off and not
        // measuring, nothing is hooked.
        let road = road::install(&resolved, true, false);
        assert!(road.iter().all(|o| !o.installed), "{road:?}");
        assert!(road.iter().any(|o| o.reason.contains("not both appenders")));
        let road = road::install(&resolved, false, false);
        assert_eq!(road.len(), 1);
        assert!(road[0].reason.contains(road::TOGGLE_ENV));
    }

    #[test]
    fn the_visit_order_is_by_entity_with_each_record_whole() {
        // {entity, TransportVehicle index} records.
        let record = |entity: u32, index: u32| u64::from(entity) | (u64::from(index) << 32);
        let records = [record(30, 0), record(10, 1), record(20, 2)];
        assert_eq!(
            platform::visit_order(&records).unwrap(),
            Some(vec![record(10, 1), record(20, 2), record(30, 0)])
        );
        assert_eq!(
            platform::visit_order(&[record(1, 5), record(2, 4)]).unwrap(),
            None
        );
        assert_eq!(platform::visit_order(&[]).unwrap(), None);
        assert_eq!(
            platform::visit_order(&[record(7, 0), record(7, 1)]),
            Err("two entries name one entity")
        );
    }

    #[test]
    fn candidates_with_equal_costs_reach_the_sort_in_one_order() {
        // {word, station, terminal}: the cost is looked up by the engine,
        // so the order is by station and terminal, whatever came first.
        let a = [9, 100, 2];
        let b = [3, 100, 1];
        let c = [1, 50, 7];
        let one = platform::candidate_order(&[a, b, c]).unwrap();
        let other = platform::candidate_order(&[b, c, a]).unwrap();
        assert_eq!(one, vec![c, b, a]);
        assert_eq!(one, other, "two games' orders end up the same");
        assert_eq!(platform::candidate_order(&[c, b, a]), None);
        assert_eq!(platform::candidate_order(&[a]), None);
    }

    #[test]
    fn road_entries_are_kept_in_entity_order_each_whole() {
        let entry = |entity: i32, tag: u8| {
            let mut e = [tag; road::ENTRY_LEN as usize];
            e[..4].copy_from_slice(&entity.to_le_bytes());
            e
        };
        let entries = [entry(40, 1), entry(12, 2), entry(33, 3)];
        assert_eq!(
            road::entry_order(&entries).unwrap(),
            Some(vec![entry(12, 2), entry(33, 3), entry(40, 1)])
        );
        assert_eq!(
            road::entry_order(&[entry(1, 0), entry(2, 0)]).unwrap(),
            None
        );
        assert_eq!(
            road::entry_order(&[entry(5, 0), entry(5, 1)]),
            Err("two entries name one entity")
        );
    }

    /// xorshift64*: the tests' own random numbers, the same every run.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n.max(1)
        }
    }

    fn road_entry(entity: i32, tag: u32) -> road::Entry {
        let mut e = [0u8; road::ENTRY_LEN as usize];
        e[..4].copy_from_slice(&entity.to_le_bytes());
        e[4..8].copy_from_slice(&tag.to_le_bytes());
        e[16] = tag as u8;
        e
    }

    /// What the hook's in-place sort does to `entries`, set against the
    /// reference: the same entries in the same order, or the same refusal
    /// with nothing written.
    fn place_agrees(entries: &[road::Entry], scratch: &mut Vec<road::Entry>) {
        let mut placed = entries.to_vec();
        let outcome = road::place(&mut placed, scratch);
        match road::entry_order(entries) {
            Ok(None) => {
                assert_eq!(outcome, Ok(Sorted::Unchanged));
                assert_eq!(placed, entries);
            }
            Ok(Some(order)) => {
                assert_eq!(outcome, Ok(Sorted::Reordered));
                assert_eq!(placed, order);
            }
            Err(why) => {
                assert_eq!(outcome, Err(why));
                assert_eq!(placed, entries, "a refusal writes nothing");
            }
        }
    }

    #[test]
    fn the_in_place_road_sort_gives_the_reference_order_on_random_lists() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut scratch = Vec::new();
        for round in 0..20_000 {
            let n = rng.below(40) as usize;
            // Small id ranges give duplicates, large ones do not.
            let range = if round % 3 == 0 { 8 } else { 1 << 20 };
            let mut ids: Vec<i32> = (0..n)
                .map(|_| rng.below(range) as i32 - (range / 4) as i32)
                .collect();
            match round % 4 {
                // A list kept sorted, then one entry appended: the hook's
                // common case.
                0 | 1 => {
                    ids.sort_unstable();
                    ids.dedup();
                    ids.push(rng.below(range) as i32 - (range / 4) as i32);
                }
                // Already sorted.
                2 => ids.sort_unstable(),
                // Anything.
                _ => {}
            }
            let entries: Vec<road::Entry> = ids
                .iter()
                .enumerate()
                .map(|(i, id)| road_entry(*id, i as u32))
                .collect();
            place_agrees(&entries, &mut scratch);
        }
    }

    #[test]
    fn the_in_place_road_sort_places_an_appended_entry_at_each_end_and_between() {
        let mut scratch = Vec::new();
        let kept = [10, 20, 30, 40];
        for (new, at) in [(5, 0), (15, 1), (35, 3), (45, 4)] {
            let mut entries: Vec<road::Entry> =
                kept.iter().map(|id| road_entry(*id, *id as u32)).collect();
            entries.push(road_entry(new, 99));
            let outcome = road::place(&mut entries, &mut scratch);
            let expected = if at == 4 {
                Ok(Sorted::Unchanged)
            } else {
                Ok(Sorted::Reordered)
            };
            assert_eq!(outcome, expected, "{new}");
            let ids: Vec<i32> = entries.iter().map(road::key).collect();
            let mut want = kept.to_vec();
            want.insert(at, new);
            assert_eq!(ids, want);
            assert_eq!(entries[at][16], 99, "the entry moved whole");
        }
        // The appended entry names an entity the list has: refused.
        let mut entries: Vec<road::Entry> = [10, 20, 30, 20]
            .iter()
            .map(|id| road_entry(*id, 0))
            .collect();
        let before = entries.clone();
        assert_eq!(
            road::place(&mut entries, &mut scratch),
            Err("two entries name one entity")
        );
        assert_eq!(entries, before);
        // An empty list and one entry: nothing to do.
        assert_eq!(road::place(&mut [], &mut scratch), Ok(Sorted::Unchanged));
        assert_eq!(
            road::place(&mut [road_entry(3, 0)], &mut scratch),
            Ok(Sorted::Unchanged)
        );
    }

    #[test]
    fn the_visit_records_sort_as_the_reference_with_one_buffer() {
        let mut rng = Rng(0x1234_5678_9abc_def1);
        let mut buffer = Vec::new();
        for round in 0..5_000 {
            let n = rng.below(30);
            let range = if round % 3 == 0 { 10 } else { 1 << 24 };
            let mut records: Vec<u64> = (0..n)
                .map(|i| u64::from(rng.below(range) as u32) | (i << 32))
                .collect();
            if round % 2 == 0 {
                records.sort_unstable_by_key(|r| *r as u32 as i32);
            }
            let outcome = platform::sort_records(n, |i| records[i as usize], &mut buffer);
            match platform::visit_order(&records) {
                Ok(None) => assert_eq!(outcome, Ok(Sorted::Unchanged)),
                Ok(Some(order)) => {
                    assert_eq!(outcome, Ok(Sorted::Reordered));
                    assert_eq!(buffer, order);
                }
                Err(why) => assert_eq!(outcome, Err(why)),
            }
        }
    }

    #[test]
    fn the_candidates_sort_in_place_as_the_reference() {
        let mut rng = Rng(0x0fed_cba9_8765_4321);
        for round in 0..5_000 {
            let n = rng.below(12) as usize;
            let range = if round % 2 == 0 { 3 } else { 1000 };
            let words: Vec<[u32; 3]> = (0..n)
                .map(|_| {
                    [
                        rng.below(range) as u32,
                        rng.below(range) as u32,
                        rng.below(range) as u32,
                    ]
                })
                .collect();
            let mut bytes: Vec<platform::Candidate> = words
                .iter()
                .map(|w| {
                    let mut c = [0u8; 12];
                    for (i, word) in w.iter().enumerate() {
                        c[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
                    }
                    c
                })
                .collect();
            let outcome = platform::sort_candidates_in_place(&mut bytes);
            let back: Vec<[u32; 3]> = bytes
                .iter()
                .map(|c| {
                    let word =
                        |i: usize| u32::from_le_bytes(c[4 * i..4 * i + 4].try_into().unwrap());
                    [word(0), word(1), word(2)]
                })
                .collect();
            match platform::candidate_order(&words) {
                None => {
                    assert_eq!(outcome, Sorted::Unchanged);
                    assert_eq!(back, words);
                }
                Some(order) => {
                    assert_eq!(outcome, Sorted::Reordered);
                    assert_eq!(back, order);
                }
            }
        }
    }

    #[test]
    fn a_refusal_is_counted_by_reason_until_taken() {
        let refusals = Refusals::new();
        refusals.note("test", "a");
        refusals.note("test", "b");
        refusals.note("test", "a");
        assert_eq!(refusals.take_window(), vec![("a", 2), ("b", 1)]);
        assert_eq!(refusals.take_window(), vec![]);
        assert_eq!(refusals.count.load(Ordering::Relaxed), 3);
    }

    /// The road sort alone, before and after: the reference's copy, key
    /// array, permutation and write-back against [`road::place`], on lists
    /// kept sorted with one entry appended at a random place. Run with
    /// `cargo test --release -p tpf3mp-hook order::tests::road_sort_bench -- --ignored --nocapture`.
    #[test]
    #[ignore = "a benchmark: prints the road sort's cost before and after"]
    fn road_sort_bench() {
        let mut rng = Rng(42);
        let mut scratch = Vec::new();
        for n in [2usize, 8, 32, 128] {
            let lists: Vec<Vec<road::Entry>> = (0..2_000)
                .map(|_| {
                    let mut ids: Vec<i32> = (0..n - 1).map(|_| rng.below(1 << 30) as i32).collect();
                    ids.sort_unstable();
                    ids.dedup();
                    ids.push(rng.below(1 << 30) as i32);
                    ids.iter().map(|id| road_entry(*id, 0)).collect()
                })
                .collect();
            let rounds = 50;
            let begin = std::time::Instant::now();
            for _ in 0..rounds {
                for list in &lists {
                    let mut memory = list.clone();
                    // The reference path as the hook ran it: a copy, the
                    // order, the write-back.
                    let entries: Vec<road::Entry> = memory.to_vec();
                    if let Ok(Some(order)) = road::entry_order(&entries) {
                        memory.copy_from_slice(&order);
                    }
                    std::hint::black_box(&memory);
                }
            }
            let before = begin.elapsed().as_nanos() as f64 / (rounds * lists.len()) as f64;
            let begin = std::time::Instant::now();
            for _ in 0..rounds {
                for list in &lists {
                    let mut memory = list.clone();
                    let _ = road::place(&mut memory, &mut scratch);
                    std::hint::black_box(&memory);
                }
            }
            let after = begin.elapsed().as_nanos() as f64 / (rounds * lists.len()) as f64;
            println!(
                "road sort, {n} entries: before {before:.0} ns, after {after:.0} ns (the list's clone included in both)"
            );
        }
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

    /// The platform chooser's loop head, as the engine has it: `this` in
    /// r13 (`[this+8]` the node-list holder), the count at `[rbp+0x5b0]`,
    /// the byte offset of the iteration in rsi. The fixture returns the
    /// entity the iteration reads at `[rsi+rdi]` after the site.
    #[test]
    fn the_visit_hook_walks_the_node_list_in_entity_order_without_writing_it() {
        let mut memory = vec![0u8; 0x1000];
        let base = memory.as_mut_ptr() as u64;
        let put = |memory: &mut Vec<u8>, at: usize, value: u64| {
            memory[at..at + 8].copy_from_slice(&value.to_le_bytes());
        };
        const THIS: usize = 0x100;
        const HOLDER: usize = 0x140;
        const RECORDS: usize = 0x200;
        const RBP: usize = 0x300;
        // {entity, index}: 30, 10, 20.
        for (i, (entity, index)) in [(30u32, 0u32), (10, 1), (20, 2)].into_iter().enumerate() {
            let at = RECORDS + i * 8;
            memory[at..at + 4].copy_from_slice(&entity.to_le_bytes());
            memory[at + 4..at + 8].copy_from_slice(&index.to_le_bytes());
        }
        put(&mut memory, THIS + 8, base + HOLDER as u64);
        put(&mut memory, HOLDER, base + RECORDS as u64);
        put(&mut memory, HOLDER + 8, base + RECORDS as u64 + 24);
        memory[RBP + 0x5b0..RBP + 0x5b4].copy_from_slice(&3i32.to_le_bytes());
        // push rbp; push r13; push rsi; push rdi; push rbx; mov rbp, imm64;
        // mov r13, imm64; mov rsi, rcx; mov rax, [r13+8]; mov rdi, [rax];
        // <site>; mov eax, [rsi+rdi]; pop rbx; pop rdi; pop rsi; pop r13;
        // pop rbp; ret
        let mut code = vec![0x55, 0x41, 0x55, 0x56, 0x57, 0x53, 0x48, 0xBD];
        code.extend_from_slice(&(base + RBP as u64).to_le_bytes());
        code.extend_from_slice(&[0x49, 0xBD]);
        code.extend_from_slice(&(base + THIS as u64).to_le_bytes());
        code.extend_from_slice(&[0x48, 0x89, 0xCE, 0x49, 0x8B, 0x45, 0x08, 0x48, 0x8B, 0x38]);
        let site_at = code.len();
        code.extend_from_slice(&platform::VISIT_EXPECTED);
        code.extend_from_slice(&[0x8B, 0x04, 0x3E, 0x5B, 0x5F, 0x5E, 0x41, 0x5D, 0x5D, 0xC3]);
        let page = fixture(&code);
        // SAFETY: the page holds a function of one integer argument.
        let fun: extern "C" fn(u64) -> u32 =
            unsafe { std::mem::transmute::<usize, extern "C" fn(u64) -> u32>(page) };
        let walk = || (0..3).map(|i| fun(i * 8)).collect::<Vec<u32>>();
        assert_eq!(walk(), vec![30, 10, 20], "the engine's own order");

        // SAFETY: the fixture is this test's own code, not running now.
        let splice = unsafe {
            Splice::install(
                (page + site_at) as *mut u8,
                &platform::VISIT_EXPECTED,
                platform::VISIT_STEAL,
                platform::visit_hook,
            )
        }
        .unwrap();
        assert_eq!(walk(), vec![10, 20, 30], "visited in entity order");
        let records: Vec<u8> = memory[RECORDS..RECORDS + 24].to_vec();
        assert_eq!(
            &records[..4],
            &30u32.to_le_bytes(),
            "the list is not written"
        );
        // A count that is not the list's: the engine's own order stands.
        memory[RBP + 0x5b0..RBP + 0x5b4].copy_from_slice(&2i32.to_le_bytes());
        assert_eq!(fun(0), 30);
        // SAFETY: nothing runs the fixture now.
        unsafe { splice.detach() }.unwrap();
        drop(memory);
    }

    /// `EdgeUseManager`'s data as `GetOrAddEdgeData` walks it, built in
    /// memory: the entries of edge (entity 1, index 1) are sorted in place.
    #[test]
    fn the_road_sort_finds_an_edges_entries_and_puts_them_in_entity_order() {
        let mut memory = vec![0u8; 0x800];
        let base = memory.as_mut_ptr() as u64;
        let put = |memory: &mut Vec<u8>, at: usize, value: u64| {
            memory[at..at + 8].copy_from_slice(&value.to_le_bytes());
        };
        let int = |memory: &mut Vec<u8>, at: usize, value: i32| {
            memory[at..at + 4].copy_from_slice(&value.to_le_bytes());
        };
        const DATA: usize = 0x100;
        const INDEX: usize = 0x200;
        const SLOTS: usize = 0x300;
        const EDGES: usize = 0x400;
        const ENTRIES: usize = 0x500;
        const EDGE_ID: usize = 0x600;
        put(&mut memory, 0x18, base + DATA as u64);
        put(&mut memory, DATA, base + INDEX as u64);
        put(&mut memory, DATA + 8, base + INDEX as u64 + 12);
        put(&mut memory, DATA + 0x18, base + SLOTS as u64);
        put(&mut memory, DATA + 0x20, base + SLOTS as u64 + 2 * 72);
        // Entity 0 has no slot, entity 1 slot 1, entity 2 slot 0.
        for (i, slot) in [-1, 1, 0].into_iter().enumerate() {
            int(&mut memory, INDEX + i * 4, slot);
        }
        put(&mut memory, SLOTS + 72, base + EDGES as u64);
        put(&mut memory, SLOTS + 72 + 8, base + EDGES as u64 + 2 * 32);
        put(&mut memory, EDGES + 32 + 8, base + ENTRIES as u64);
        put(
            &mut memory,
            EDGES + 32 + 0x10,
            base + ENTRIES as u64 + 3 * 20,
        );
        for (i, entity) in [40, 12, 33].into_iter().enumerate() {
            int(&mut memory, ENTRIES + i * 20, entity);
            memory[ENTRIES + i * 20 + 16] = i as u8;
        }
        int(&mut memory, EDGE_ID, 1);
        int(&mut memory, EDGE_ID + 4, 1);
        let entities = |memory: &Vec<u8>| {
            (0..3)
                .map(|i| {
                    let at = ENTRIES + i * 20;
                    let id = i32::from_le_bytes(memory[at..at + 4].try_into().unwrap());
                    (id, memory[at + 16])
                })
                .collect::<Vec<_>>()
        };
        let sorted = road::sort_edge(&mut Probe::new(), base + DATA as u64, base + EDGE_ID as u64);
        assert_eq!(sorted, Ok(Sorted::Reordered));
        assert_eq!(entities(&memory), vec![(12, 1), (33, 2), (40, 0)]);
        assert_eq!(
            road::sort_edge(&mut Probe::new(), base + DATA as u64, base + EDGE_ID as u64),
            Ok(Sorted::Unchanged)
        );
        // An edge whose entity has no slot, and one past its slot's edges.
        int(&mut memory, EDGE_ID, 0);
        assert_eq!(
            road::sort_edge(&mut Probe::new(), base + DATA as u64, base + EDGE_ID as u64),
            Err("the edge's entity has no slot")
        );
        int(&mut memory, EDGE_ID, 1);
        int(&mut memory, EDGE_ID + 4, 2);
        assert_eq!(
            road::sort_edge(&mut Probe::new(), base + DATA as u64, base + EDGE_ID as u64),
            Err("the edge index is past the slot's edges")
        );
    }

    /// A manager whose data has one edge entity (1) with `edges` edges, each
    /// holding the entries `lists[i]` (entity ids; the byte at +16 tags
    /// each), and a path of those edges: `[0]` the manager (its data at
    /// `+0x18`), the data at `DATA`, the path vector at `PATH`.
    struct RoadWorld {
        memory: Vec<u8>,
        entries: Vec<Vec<road::Entry>>,
    }

    impl RoadWorld {
        const DATA: usize = 0x100;
        const INDEX: usize = 0x200;
        const SLOTS: usize = 0x300;
        const EDGES: usize = 0x400;
        const PATH: usize = 0x600;
        const PATH_IDS: usize = 0x700;

        fn new(lists: &[&[i32]]) -> Self {
            let mut memory = vec![0u8; 0x1000];
            let base = memory.as_mut_ptr() as u64;
            let put = |memory: &mut Vec<u8>, at: usize, value: u64| {
                memory[at..at + 8].copy_from_slice(&value.to_le_bytes());
            };
            let mut entries: Vec<Vec<road::Entry>> = lists
                .iter()
                .map(|ids| {
                    let mut list: Vec<road::Entry> = ids
                        .iter()
                        .enumerate()
                        .map(|(i, id)| {
                            let mut e = [0u8; 20];
                            e[..4].copy_from_slice(&id.to_le_bytes());
                            e[16] = i as u8;
                            e
                        })
                        .collect();
                    list.reserve(4);
                    list
                })
                .collect();
            put(&mut memory, 0x18, base + Self::DATA as u64);
            put(&mut memory, Self::DATA, base + Self::INDEX as u64);
            put(&mut memory, Self::DATA + 8, base + Self::INDEX as u64 + 8);
            put(&mut memory, Self::DATA + 0x18, base + Self::SLOTS as u64);
            put(
                &mut memory,
                Self::DATA + 0x20,
                base + Self::SLOTS as u64 + 72,
            );
            // Entity 0 has no slot, entity 1 slot 0.
            memory[Self::INDEX..Self::INDEX + 4].copy_from_slice(&(-1i32).to_le_bytes());
            memory[Self::INDEX + 4..Self::INDEX + 8].copy_from_slice(&0i32.to_le_bytes());
            put(&mut memory, Self::SLOTS, base + Self::EDGES as u64);
            put(
                &mut memory,
                Self::SLOTS + 8,
                base + Self::EDGES as u64 + 32 * lists.len() as u64,
            );
            for (i, list) in entries.iter_mut().enumerate() {
                let at = Self::EDGES + 32 * i;
                let begin = list.as_mut_ptr() as u64;
                put(&mut memory, at + 8, begin);
                put(&mut memory, at + 0x10, begin + 20 * list.len() as u64);
                let id = Self::PATH_IDS + 12 * i;
                memory[id..id + 4].copy_from_slice(&1i32.to_le_bytes());
                memory[id + 4..id + 8].copy_from_slice(&(i as i32).to_le_bytes());
            }
            put(&mut memory, Self::PATH, base + Self::PATH_IDS as u64);
            put(
                &mut memory,
                Self::PATH + 8,
                base + Self::PATH_IDS as u64 + 12 * lists.len() as u64,
            );
            Self { memory, entries }
        }

        fn at(&self, offset: usize) -> usize {
            self.memory.as_ptr() as usize + offset
        }

        fn ids(&self, edge: usize) -> Vec<i32> {
            self.entries[edge].iter().map(road::key).collect()
        }
    }

    /// Stands in for the engine's appender: appends nothing.
    #[allow(clippy::too_many_arguments)]
    unsafe extern "system" fn engine_add(
        _: usize,
        _: usize,
        _: usize,
        _: usize,
        _: usize,
        _: usize,
        _: usize,
        _: usize,
    ) -> usize {
        7
    }

    #[allow(clippy::too_many_arguments)]
    unsafe extern "system" fn engine_add_range(
        _: usize,
        _: usize,
        _: usize,
        _: usize,
        _: usize,
        _: usize,
        _: usize,
        _: usize,
        _: usize,
    ) -> usize {
        9
    }

    /// `AddRange` hands the manager's data as `this` and the manager as its
    /// ninth argument; `Add` hands the manager. Both sort the edges they
    /// touched, and nothing else.
    #[test]
    fn both_appenders_sort_their_edges_add_range_from_the_data_it_is_given() {
        let _serial = crate::lua::tests::SERIAL
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let world = RoadWorld::new(&[&[40, 12], &[5, 9, 7], &[3, 1], &[8, 2]]);
        road::arm_for_test(
            engine_add as *const () as usize,
            engine_add_range as *const () as usize,
        );
        // AddRange(data, entity, component, &path, current, bounds, from=1,
        // to=2, manager): edges 1 and 2 sorted, 0 and 3 not.
        // SAFETY: the detour with the arguments the engine's caller passes,
        // on a world built in memory; the "engine" appends nothing.
        let result = unsafe {
            road::add_range(
                world.at(RoadWorld::DATA),
                9,
                0,
                world.at(RoadWorld::PATH),
                1,
                0,
                1,
                2,
                world.at(0),
            )
        };
        assert_eq!(result, 9, "the engine's answer passes through");
        assert_eq!(world.ids(0), vec![40, 12]);
        assert_eq!(world.ids(1), vec![5, 7, 9]);
        assert_eq!(world.ids(2), vec![1, 3]);
        assert_eq!(world.ids(3), vec![8, 2]);
        assert_eq!(world.entries[1][1][16], 2, "entries moved whole");
        // Read the old way, through `[this+0x18]` of the data, the edge's
        // entity has no slot: a manager that does not name this data is
        // refused, nothing written.
        let _ = road::take_refusals();
        // SAFETY: as above.
        unsafe {
            road::add_range(
                world.at(RoadWorld::DATA),
                9,
                0,
                world.at(RoadWorld::PATH),
                0,
                0,
                0,
                3,
                world.at(RoadWorld::DATA),
            )
        };
        assert_eq!(world.ids(0), vec![40, 12]);
        assert_eq!(
            road::take_refusals(),
            vec![("AddRange's data is not its manager's", 1)]
        );
        // Add(manager, &edgeId, ...): its one edge.
        // SAFETY: as above.
        let result = unsafe {
            road::add(
                world.at(0),
                world.at(RoadWorld::PATH_IDS + 3 * 12),
                2,
                0,
                0,
                0,
                0,
                0,
            )
        };
        assert_eq!(result, 7);
        assert_eq!(world.ids(3), vec![2, 8]);
        assert_eq!(world.ids(0), vec![40, 12]);
        road::arm_for_test(0, 0);
    }

    /// An append inside the game's step and one outside it (the second
    /// engine's copy is made from the frame) are counted apart, so the
    /// in-step counts can be compared between games.
    #[test]
    fn appends_are_counted_apart_inside_the_step_and_outside_it() {
        let _serial = crate::lua::tests::SERIAL
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        road::arm_for_test(
            engine_add as *const () as usize,
            engine_add_range as *const () as usize,
        );
        let append = |step: bool, lists: &'static [&'static [i32]]| {
            std::thread::spawn(move || {
                if step {
                    set_in_step(true);
                }
                let world = RoadWorld::new(lists);
                // SAFETY: as in the test above: the detour on a world built
                // in memory, the "engine" appending nothing.
                unsafe {
                    road::add_range(
                        world.at(RoadWorld::DATA),
                        9,
                        0,
                        world.at(RoadWorld::PATH),
                        0,
                        0,
                        0,
                        0,
                        world.at(0),
                    )
                };
            })
            .join()
            .unwrap();
        };
        let before = road::thread_counts();
        append(true, &[&[3, 1]]);
        let after_step = road::thread_counts();
        assert_eq!(after_step.step_appends, before.step_appends + 1);
        assert_eq!(after_step.step_reorders, before.step_reorders + 1);
        assert_eq!(after_step.other_appends, before.other_appends);
        append(false, &[&[1, 3]]);
        let after_other = road::thread_counts();
        assert_eq!(after_other.other_appends, after_step.other_appends + 1);
        assert_eq!(after_other.other_reorders, after_step.other_reorders);
        assert_eq!(after_other.step_appends, after_step.step_appends);
        road::arm_for_test(0, 0);
    }

    /// The road fix's cost per append on a real edge in memory, before and
    /// after: before, every word was checked with its own `VirtualQuery`
    /// (13 for one edge) and the list copied, keyed, permuted and written
    /// back; after, one probe per append and the in-place placement. Run
    /// with `cargo test --release -p tpf3mp-hook order::splice_tests::road_append_bench -- --ignored --nocapture`.
    #[test]
    #[ignore = "a benchmark: prints the road fix's cost per append before and after"]
    fn road_append_bench() {
        let rounds = 100_000;
        for n in [2usize, 8, 32] {
            let ids: Vec<i32> = (0..n as i32).map(|i| i * 10).collect();
            let world = RoadWorld::new(&[&ids]);
            let data = world.at(RoadWorld::DATA) as u64;
            let edge = world.at(RoadWorld::PATH_IDS) as u64;
            // The words the old walk checked, one system call each.
            let words: Vec<usize> = {
                let slot = world.at(RoadWorld::SLOTS);
                let edge_data = world.at(RoadWorld::EDGES);
                vec![
                    edge as usize,
                    edge as usize + 4,
                    world.at(0x18),
                    data as usize,
                    data as usize + 8,
                    world.at(RoadWorld::INDEX + 4),
                    data as usize + 0x18,
                    data as usize + 0x20,
                    slot,
                    slot + 8,
                    edge_data + 8,
                    edge_data + 0x10,
                ]
            };
            let entries_at = world.entries[0].as_ptr() as usize;
            let begin = std::time::Instant::now();
            for _ in 0..rounds {
                for word in &words {
                    assert!(crate::image::readable(*word, 8));
                }
                assert!(crate::image::readable(entries_at, 20 * n));
                let copy: Vec<road::Entry> = world.entries[0].clone();
                std::hint::black_box(road::entry_order(&copy).unwrap());
            }
            let before = begin.elapsed().as_nanos() as f64 / f64::from(rounds);
            let begin = std::time::Instant::now();
            for _ in 0..rounds {
                let mut probe = Probe::new();
                std::hint::black_box(road::sort_edge(&mut probe, data, edge).unwrap());
            }
            let after = begin.elapsed().as_nanos() as f64 / f64::from(rounds);
            println!(
                "road append, one edge of {n} entries in order: before {before:.0} ns, after {after:.0} ns ({:.1}x)",
                before / after
            );
        }
    }
}

//! Seeds: the randomness in Transport Fever 3 that is not a function of the
//! room's state, made one (docs/HOOKS.md, "Seeds, as built";
//! investigation/TPF3_RNG_2026-09-29.md, items 5 to 7).
//!
//! Lockstep needs every replica to draw the same random numbers at the same
//! step. TF3's simulation seeds its generators from game state, with three
//! exceptions the survey found, each a piece here, each independent of the
//! others, each failing closed on its own with its reason in `hook.log`:
//!
//! 1. **The game scripts' `math.random`** is the engine's own `mt19937`,
//!    one per `lua::State`, seeded with the constant 5489 when the state is
//!    created and never saved: two replicas that ran the same script calls
//!    since their states were created agree, and a replica that joined (its
//!    state fresh at 5489 while the others' streams are far along) does
//!    not. [`before_update`] reseeds every **game-script state** from the
//!    room's step number before each update of the simulation, so the
//!    stream every replica draws from at step *n* is `seed_for(n)`,
//!    whatever its history. The states are told apart at the registrar:
//!    the game's per-game-script-state registrar (`CGame::CGame::lambda_3`,
//!    a profile target, called once per game-script state with the
//!    `lua::State&` in `r8`) is detoured to mark the state it registers,
//!    and only marked states are reseeded; the GUI state (the mod's, the
//!    one the mod's handlers came from) and the react UI roots are never
//!    touched, and the GUI state's scripts draw nothing anyway (the survey,
//!    section 6). The reseed runs on the simulation thread inside
//!    `GameSim::Step`, from a detour on `ecs::Engine::Update` (the
//!    per-update advance, the one call `GameSim::Step` makes per update,
//!    before the systems and so before `ecs::GameScriptSystem::Update`
//!    runs the scripts), so it is on the thread that runs those states
//!    while they are idle. It is per update, not per call of the game's
//!    step: a call runs a batch of updates, batches differ per machine,
//!    and a reseed per batch would itself diverge the replicas.
//! 2. **`TownDevelopAt`** seeds its town developer from the CRT `rand()`,
//!    which the game seeds from the wall clock. The player's
//!    `makeTownDevelopAtCmd` is refused in a room already (the mod's
//!    `guard.lua`, docs/HOOKS.md "The player's commands"); the
//!    alternative for later is here behind [`TOWN_DEVELOP_RESEED`]: a
//!    detour on `TownDevelopAt::Apply` that calls the CRT's `srand` on the
//!    applying thread with a seed from the step and the command's number
//!    within the step, right before the original runs.
//! 3. **The CRT's transcendental dispatch** (`sinf`, `cosf`, ... pick an
//!    FMA3 or an SSE2 body at run time by CPU) is a measurement, not a fix:
//!    [`cpu_report`] goes to `hook.log` at bootstrap so two replicas' logs
//!    say whether their CRTs took the same path.
//!
//! The seed is [`seed_for`]: a `splitmix64` mix of the step and a salt,
//! folded to `1..=0x7fff_ffff` (an `mt19937` takes a 32-bit seed, the
//! engine's `math.randomseed` reads an integer, and `minstd_rand` treats 0
//! as 1, so the range suits every generator here). Both game-script states
//! get the same seed (they run the same scripts on the two engines, and
//! their registration order is not proven equal on every replica, so no
//! per-state salt is derived from it).
//!
//! The detours that hand control to this module are assembly thunks: they
//! save the four argument registers and `xmm0`-`xmm3` (the targets take
//! floats in them: `ecs::Engine::Update(engine, float dt)`), call the
//! Rust side, restore, and jump to the original's trampoline. So nothing
//! here assumes a target's signature, and the original sees its stack and
//! registers exactly as its caller left them.

#![allow(unsafe_code)]
// Elsewhere the detours are not installed, so their code is unused there.
#![cfg_attr(not(all(windows, target_arch = "x86_64")), allow(dead_code))]

use std::{
    ffi::{c_char, c_int, c_void},
    panic::catch_unwind,
    sync::{
        Mutex, MutexGuard, OnceLock, PoisonError,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread::ThreadId,
};

use tpf3mp_hookcore::profile::ResolvedProfile;

use crate::{
    log,
    lua::{self, State},
    step::Updates,
};

/// The profile target that registers each game-script state: `CGame::CGame`
/// lambda_3's `operator()(this, TypeRegistry&, lua::State& r8, bool r9b,
/// weak_ptr<bool const>)`, called once per game-script state (twice per
/// `CGame`: the game has two engines, and a script state for each).
pub const GAME_SCRIPT_TARGET: &str = "CGame::CGame::lambda_3";
/// The profile target for the per-update advance: `ecs::Engine::Update
/// (engine, float dt)`, the one call `GameSim::Step` makes for each update
/// after applying that update's commands, before any system runs.
pub const UPDATE_TARGET: &str = "ecs::Engine::Update";
/// The profile target for the `TownDevelopAt` command's applier.
pub const TOWN_DEVELOP_TARGET: &str = "TownDevelopAt::Apply";

/// Whether the `TownDevelopAt::Apply` detour is installed. `false` until the
/// reseed is measured in a room: the command is refused at
/// `CommandList::Add` meanwhile, so a player's cannot diverge the worlds,
/// and a game script's (none in the base game sends one) would diverge on
/// its `rand()` seed exactly as the survey says. Flip it once a room has
/// shown, in the `t` lane, that a reseeded `TownDevelopAt` develops the
/// same town everywhere; the seed under batching is documented at
/// [`Batch::town_apply`].
pub const TOWN_DEVELOP_RESEED: bool = false;

/// The salt of the game-script states' seed.
pub const GAME_SCRIPT_SALT: u32 = 0;
/// The salt of the `TownDevelopAt` seed (`"town"`), plus the command's
/// number within its step.
pub const TOWN_DEVELOP_SALT: u32 = 0x746f_776e;

/// How many game-script states one `CGame` registers (the recon: lambda_3
/// is called twice, once per engine). A mark past that many starts a new
/// game's group, and the older group is retired, never touched again: a
/// state of a game that was torn down is a dangling pointer.
pub const GAME_SCRIPT_STATES_PER_GAME: usize = 2;
/// Most states the roster keeps; beyond it, later ones are counted only.
const MAX_ROSTER: usize = 64;
/// After the first few, the reseed goes to the log once per this many steps.
const LOG_FIRST: u64 = 3;
const LOG_EVERY: u64 = 1_000;

/// The seed every replica uses at `step` for the generator `salt` names:
/// `splitmix64(step ^ (salt << 32))`, folded to `1..=0x7fff_ffff`.
pub fn seed_for(step: u64, salt: u32) -> u32 {
    let mut z = step ^ (u64::from(salt) << 32);
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    // The high bits, into 1..=2^31-1.
    ((z >> 33) as u32) % 0x7fff_fffe + 1
}

// ---------- the batch of updates one call of the game's step runs ----------

/// The updates one call of the game's step runs, as the step driver armed
/// them: the room's step of the first, how many, and how many have begun.
/// Pure; the statics below wrap it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Batch {
    base: u64,
    count: u32,
    index: u32,
    /// `TownDevelopAt` commands applied in the update about to run.
    town_applies: u32,
}

impl Batch {
    /// Arms the batch: `next_step` and `updates` as the driver has them. No
    /// step known, the game's own speed, or no update disarms it, and
    /// nothing is reseeded until the driver arms again. Answers how many
    /// calls of `ecs::Engine::Update` the previous batch saw against how
    /// many it expected, when they differ.
    pub fn arm(&mut self, next_step: Option<u64>, updates: Updates) -> Option<(u32, u32)> {
        let mismatch =
            (self.count > 0 && self.index != self.count).then_some((self.index, self.count));
        *self = match (next_step, updates) {
            (Some(step), Updates::Exactly(count)) if count > 0 => Self {
                base: step,
                count,
                index: 0,
                town_applies: 0,
            },
            _ => Self::default(),
        };
        mismatch
    }

    /// Whether the driver armed a batch that has updates left.
    pub fn armed(&self) -> bool {
        self.count > 0 && self.index < self.count
    }

    /// An update begins: its step, or `None` when the batch is not armed
    /// or the game runs more updates than the driver released (counted,
    /// so the next `arm` can report it; never reseeded, the room did not
    /// release the step).
    pub fn next_update(&mut self) -> Option<u64> {
        if self.count == 0 {
            return None;
        }
        let index = self.index;
        self.index = self.index.saturating_add(1);
        if index >= self.count {
            return None;
        }
        self.town_applies = 0;
        Some(self.base + u64::from(index))
    }

    /// A `TownDevelopAt` is applied: the step it belongs to and its number
    /// within that step, or `None` outside an armed batch. Commands for an
    /// update are applied just before that update's `ecs::Engine::Update`
    /// (the game's step, `GameSim.cpp`), so the step is the one the next
    /// update runs.
    pub fn town_apply(&mut self) -> Option<(u64, u32)> {
        if !self.armed() {
            return None;
        }
        let number = self.town_applies;
        self.town_applies = self.town_applies.saturating_add(1);
        Some((self.base + u64::from(self.index), number))
    }
}

static BATCH: Mutex<Batch> = Mutex::new(Batch {
    base: 0,
    count: 0,
    index: 0,
    town_applies: 0,
});

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Logs `message` once per process for `flag`.
fn once(flag: &AtomicBool, message: &str) {
    if !flag.swap(true, Ordering::Relaxed) {
        log::line(message);
    }
}

static UPDATE_HOOKED: AtomicBool = AtomicBool::new(false);
static MISMATCH_LOGGED: AtomicBool = AtomicBool::new(false);
static ROSTER_LOGGED: AtomicBool = AtomicBool::new(false);
static EXTRA_UPDATE_LOGGED: AtomicBool = AtomicBool::new(false);
static TOWN_OUTSIDE_LOGGED: AtomicBool = AtomicBool::new(false);

/// From the step driver, on the simulation thread, right before it runs
/// the game's step: the room's next step and the updates this call runs.
/// Arms the per-update reseed for exactly those updates; anything else
/// (no step known yet, the game's own speed, the paused path) disarms it.
pub fn before_updates(next_step: Option<u64>, updates: Updates) {
    let mismatch = lock(&BATCH).arm(next_step, updates);
    if let Some((saw, expected)) = mismatch
        && UPDATE_HOOKED.load(Ordering::Relaxed)
    {
        once(
            &MISMATCH_LOGGED,
            &format!(
                "seeds: a batch of {expected} updates saw {saw} calls of {UPDATE_TARGET}; the per-update reseed does not match the game's updates (measure before trusting it)"
            ),
        );
    }
    if matches!(updates, Updates::Exactly(n) if n > 0) {
        once(&ROSTER_LOGGED, &roster_report());
    }
}

// ---------- the roster of Lua states ----------

/// A Lua state the registrar detour saw.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    state: usize,
    thread: ThreadId,
    target: String,
}

/// A game-script state, marked by the game's own registrar for them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Marked {
    state: usize,
    thread: ThreadId,
    /// The registrar's `bool` argument, which tells the two states apart.
    flag: bool,
    /// The reseed failed in this state once; it is never tried again (the
    /// same failure on every replica, from the same game code).
    failed: bool,
    reseeds: u64,
}

#[derive(Default)]
struct Roster {
    seen: Vec<Seen>,
    /// Beyond `MAX_ROSTER`.
    more: u64,
    /// The current game's game-script states.
    marked: Vec<Marked>,
    /// Groups retired so far (a new game's states arrived).
    retired: u64,
}

static ROSTER: Mutex<Roster> = Mutex::new(Roster {
    seen: Vec::new(),
    more: 0,
    marked: Vec::new(),
    retired: 0,
});

/// From the Lua bridge: `target` just registered `tpf3mp_native` in
/// `state` on this thread.
pub fn saw_state(state: usize, target: &str) {
    let mut roster = lock(&ROSTER);
    if roster.seen.len() >= MAX_ROSTER {
        roster.more += 1;
        return;
    }
    roster.seen.push(Seen {
        state,
        thread: std::thread::current().id(),
        target: target.to_owned(),
    });
}

/// From the game-script registrar's detour: `state` is a game-script
/// state, being set up on this thread. A mark past
/// [`GAME_SCRIPT_STATES_PER_GAME`], or of a pointer already marked, starts
/// a new game's group and retires the old.
fn mark_game_script(state: usize, flag: bool) {
    let thread = std::thread::current().id();
    let mut roster = lock(&ROSTER);
    if roster.marked.len() >= GAME_SCRIPT_STATES_PER_GAME
        || roster.marked.iter().any(|marked| marked.state == state)
    {
        roster.retired += 1;
        log::line(&format!(
            "seeds: a new game's script states arrive; the {} state(s) of the last are retired (group {})",
            roster.marked.len(),
            roster.retired
        ));
        roster.marked.clear();
    }
    roster.marked.push(Marked {
        state,
        thread,
        flag,
        failed: false,
        reseeds: 0,
    });
    log::line(&format!(
        "seeds: game-script Lua state {state:#x} registered by {GAME_SCRIPT_TARGET} (flag {flag}, state #{} of this game) on thread {thread:?}",
        roster.marked.len()
    ));
}

/// One line saying which states exist and which the reseed covers.
fn roster_report() -> String {
    let roster = lock(&ROSTER);
    let seen: Vec<String> = roster
        .seen
        .iter()
        .map(|seen| {
            let role = if roster.marked.iter().any(|m| m.state == seen.state) {
                "game-script"
            } else {
                "other (UI root?)"
            };
            format!(
                "{:#x} from {} on {:?}: {role}",
                seen.state, seen.target, seen.thread
            )
        })
        .collect();
    let covered: Vec<String> = roster
        .marked
        .iter()
        .filter(|m| !m.failed)
        .map(|m| format!("{:#x}", m.state))
        .collect();
    format!(
        "seeds: Lua states seen [{}]{}; the per-step math.random reseed covers [{}]",
        seen.join("; "),
        if roster.more > 0 {
            format!(" and {} more", roster.more)
        } else {
            String::new()
        },
        covered.join(", ")
    )
}

// ---------- the reseed ----------

/// A Lua type's code, as `lua_type` returns it.
const LUA_TTABLE: c_int = 5;
const LUA_TFUNCTION: c_int = 6;

/// `lua_pcall` as the reseed calls it: Lua 5.1's own, or 5.2's `lua_pcallk`
/// with no continuation.
#[derive(Clone, Copy)]
pub enum PCall {
    Lua51(unsafe extern "C-unwind" fn(State, c_int, c_int, c_int) -> c_int),
    Lua52(unsafe extern "C-unwind" fn(State, c_int, c_int, c_int, isize, *const c_void) -> c_int),
}

/// The few functions of Lua's C API the reseed calls. The link to the mod
/// ([`crate::lua`]) calls no Lua code, so its API has no `pcall`; the
/// reseed resolves its own from the profile ([`install`]).
#[derive(Clone, Copy)]
pub struct SeedApi {
    pub gettop: unsafe extern "C-unwind" fn(State) -> c_int,
    pub settop: unsafe extern "C-unwind" fn(State, c_int),
    pub checkstack: unsafe extern "C-unwind" fn(State, c_int) -> c_int,
    pub type_of: unsafe extern "C-unwind" fn(State, c_int) -> c_int,
    pub rawgeti: unsafe extern "C-unwind" fn(State, c_int, c_int),
    pub getfield: unsafe extern "C-unwind" fn(State, c_int, *const c_char),
    pub pushnumber: unsafe extern "C-unwind" fn(State, f64),
    pub pcall: PCall,
    pub globals: lua::Globals,
}

static SEED_API: OnceLock<SeedApi> = OnceLock::new();

/// Makes `api` the one the reseed uses; the first one stays.
pub fn install_seed_api(api: SeedApi) -> bool {
    SEED_API.set(api).is_ok()
}

fn type_name(code: c_int) -> &'static str {
    match code {
        0 => "nil",
        1 => "a boolean",
        3 => "a number",
        4 => "a string",
        LUA_TTABLE => "a table",
        LUA_TFUNCTION => "a function",
        -1 => "no value",
        _ => "another type",
    }
}

/// Calls `math.randomseed(seed)` in `state`, leaving the stack as it was.
/// Errs, with the stack restored, when `math` or `math.randomseed` is not
/// there or the call raises.
///
/// # Safety
/// `state` is a live Lua state that no other thread is running, and `api`
/// is the API of the Lua it belongs to.
pub unsafe fn reseed_state(api: &SeedApi, state: State, seed: u32) -> Result<(), String> {
    // SAFETY: the caller's contract.
    unsafe {
        let base = (api.gettop)(state);
        let outcome = (|| {
            if (api.checkstack)(state, 4) == 0 {
                return Err("the Lua stack has no room".to_owned());
            }
            match api.globals {
                lua::Globals::Registry { index, key } => {
                    (api.rawgeti)(state, index, key);
                    (api.getfield)(state, -1, c"math".as_ptr());
                }
                lua::Globals::Pseudo(index) => (api.getfield)(state, index, c"math".as_ptr()),
            }
            let math = (api.type_of)(state, -1);
            if math != LUA_TTABLE {
                return Err(format!("math is {}, not a table", type_name(math)));
            }
            (api.getfield)(state, -1, c"randomseed".as_ptr());
            let randomseed = (api.type_of)(state, -1);
            if randomseed != LUA_TFUNCTION {
                return Err(format!(
                    "math.randomseed is {}, not a function",
                    type_name(randomseed)
                ));
            }
            (api.pushnumber)(state, f64::from(seed));
            let status = match api.pcall {
                PCall::Lua51(pcall) => pcall(state, 1, 0, 0),
                PCall::Lua52(pcallk) => pcallk(state, 1, 0, 0, 0, std::ptr::null()),
            };
            if status != 0 {
                return Err(format!("math.randomseed raised (status {status})"));
            }
            Ok(())
        })();
        (api.settop)(state, base);
        outcome
    }
}

/// The seed of the room step the running update belongs to, for the mod's
/// game script to hand `math.randomseed` ([`current_seed`]); [`NO_SEED`]
/// outside the room's steps.
static CURRENT_SEED: AtomicU64 = AtomicU64::new(NO_SEED);
const NO_SEED: u64 = u64::MAX;
/// How many updates had a seed to offer, for the log.
static RESEEDS_OFFERED: AtomicU64 = AtomicU64::new(0);

/// The seed for the game script's `math.random` in the running update: the
/// room step's, or `None` outside the room's steps. The mod's game script
/// asks for it at the start of its `update` (`tpf3mp_native.seed`) and seeds
/// its own state, on the game's thread, so the hook never calls into a Lua
/// state it would have to know is still alive. The hook once did, from a
/// roster of states its registrar detour saw, and after a rebase that
/// roster held states the world load had freed: the next reseed crashed the
/// game in `lua_getfield` (2026-09-30, three-player playtest).
pub fn current_seed() -> Option<u32> {
    let seed = CURRENT_SEED.load(Ordering::Acquire);
    u32::try_from(seed).ok()
}

/// An update of the simulation begins (the `ecs::Engine::Update` detour,
/// on the simulation thread): offer the game script its step's seed.
fn before_update() {
    let step = lock(&BATCH).next_update();
    CURRENT_SEED.store(
        step.map_or(NO_SEED, |step| u64::from(seed_for(step, GAME_SCRIPT_SALT))),
        Ordering::Release,
    );
    match step {
        Some(step) => {
            let reseeds = RESEEDS_OFFERED.fetch_add(1, Ordering::Relaxed) + 1;
            if reseeds <= LOG_FIRST || step.is_multiple_of(LOG_EVERY) {
                log::line(&format!(
                    "seeds: step {step}: the game script's math.random is seeded with {} (seed #{reseeds})",
                    seed_for(step, GAME_SCRIPT_SALT)
                ));
            }
        }
        None => {
            if lock(&BATCH).count > 0 {
                once(
                    &EXTRA_UPDATE_LOGGED,
                    &format!(
                        "seeds: {UPDATE_TARGET} ran more often than the driver released updates; the extra updates are not reseeded"
                    ),
                );
            }
        }
    }
}

// ---------- TownDevelopAt ----------

/// `srand` in the CRT the game's `rand()` belongs to; 0 until resolved.
static SRAND: AtomicUsize = AtomicUsize::new(0);
static TOWN_RESEEDS: AtomicU64 = AtomicU64::new(0);

type SrandFn = unsafe extern "C" fn(u32);

/// A `TownDevelopAt` is about to be applied (its detour, on the applying
/// thread): seed the CRT `rand()` this thread reads from the step and the
/// command's number within it.
fn before_town_develop() {
    let Some((step, number)) = lock(&BATCH).town_apply() else {
        once(
            &TOWN_OUTSIDE_LOGGED,
            "seeds: TownDevelopAt applied outside the room's updates; its rand() seed is the game's own",
        );
        return;
    };
    let srand = SRAND.load(Ordering::Acquire);
    if srand == 0 {
        return;
    }
    let seed = seed_for(step, TOWN_DEVELOP_SALT.wrapping_add(number));
    // SAFETY: the address GetProcAddress gave for the CRT's srand, whose
    // signature is void srand(unsigned).
    let srand: SrandFn = unsafe { std::mem::transmute::<usize, SrandFn>(srand) };
    // SAFETY: as above; the CRT keeps rand()'s state per thread, and this
    // is the thread the applier's rand() runs on.
    unsafe { srand(seed) };
    let n = TOWN_RESEEDS.fetch_add(1, Ordering::Relaxed) + 1;
    log::line(&format!(
        "seeds: TownDevelopAt #{number} at step {step}: srand({seed}) before the applier (reseed #{n})"
    ));
}

// ---------- the CPU report ----------

/// What the CPU says of itself, as far as the CRT's dispatch cares.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CpuFeatures {
    pub vendor: String,
    pub family: u32,
    pub model: u32,
    pub stepping: u32,
    pub sse42: bool,
    pub fma: bool,
    pub movbe: bool,
    pub osxsave: bool,
    pub avx: bool,
    pub f16c: bool,
    pub bmi1: bool,
    pub avx2: bool,
    pub bmi2: bool,
    pub avx512f: bool,
    pub avx512dq: bool,
    pub avx512cd: bool,
    pub avx512bw: bool,
    pub avx512vl: bool,
    /// XCR0, the OS's enabled state components (bit 1 SSE, bit 2 AVX,
    /// bits 5-7 AVX-512); 0 without `osxsave`.
    pub xcr0: u64,
}

impl CpuFeatures {
    /// The `__isa_available` level the Microsoft C runtime would pick for
    /// this CPU, by the rule its start-up (`__isa_available_init`) is
    /// documented to apply: 0 x86, 1 SSE2, 2 SSE4.2, 3 AVX, 4 AVX2 (with
    /// FMA3 and BMI), 5 AVX-512 (F, CD, BW, DQ and VL). An estimate from
    /// the same `cpuid` bits, not a read of the variable (it is not
    /// exported, and the survey named no profile target for it): what
    /// matters for comparing two replicas is that the bits are logged.
    pub fn isa_level(&self) -> (u8, &'static str) {
        let os_avx = self.osxsave && self.xcr0 & 0x6 == 0x6;
        let os_avx512 = self.osxsave && self.xcr0 & 0xe6 == 0xe6;
        if os_avx512
            && self.avx512f
            && self.avx512cd
            && self.avx512bw
            && self.avx512dq
            && self.avx512vl
        {
            (5, "AVX-512")
        } else if os_avx && self.avx2 && self.fma && self.bmi1 && self.bmi2 {
            (4, "AVX2")
        } else if os_avx && self.avx {
            (3, "AVX")
        } else if self.sse42 {
            (2, "SSE4.2")
        } else {
            (1, "SSE2")
        }
    }

    /// Whether the CRT's `sinf`, `cosf`, `expf`, `logf`, `powf` and the
    /// double versions would run their FMA3 bodies: they do from the AVX2
    /// level up (the UCRT's `_set_FMA3_enable` default).
    pub fn fma3_math(&self) -> bool {
        self.isa_level().0 >= 4
    }

    /// One log line.
    pub fn report(&self) -> String {
        let (level, name) = self.isa_level();
        format!(
            "cpu: {} family {:#x} model {:#x} stepping {}; sse4.2 {} fma {} movbe {} avx {} f16c {} bmi1 {} avx2 {} bmi2 {} avx512 f {} cd {} bw {} dq {} vl {}; xcr0 {:#x}; estimated CRT __isa_available {level} ({name}); CRT math on FMA3 bodies: {}",
            self.vendor,
            self.family,
            self.model,
            self.stepping,
            self.sse42,
            self.fma,
            self.movbe,
            self.avx,
            self.f16c,
            self.bmi1,
            self.avx2,
            self.bmi2,
            self.avx512f,
            self.avx512cd,
            self.avx512bw,
            self.avx512dq,
            self.avx512vl,
            self.xcr0,
            if self.fma3_math() { "yes" } else { "no" }
        )
    }
}

/// The running CPU's features, from `cpuid`.
#[cfg(target_arch = "x86_64")]
pub fn cpu_features() -> CpuFeatures {
    use std::arch::x86_64::{__cpuid, __cpuid_count};
    // cpuid is available on every x86-64 CPU; leaf 1 always exists.
    let leaf0 = __cpuid(0);
    let mut vendor = Vec::with_capacity(12);
    for word in [leaf0.ebx, leaf0.edx, leaf0.ecx] {
        vendor.extend_from_slice(&word.to_le_bytes());
    }
    let vendor = String::from_utf8_lossy(&vendor).trim().to_owned();
    let max_leaf = leaf0.eax;
    let leaf1 = __cpuid(1);
    let leaf7 = if max_leaf >= 7 {
        __cpuid_count(7, 0)
    } else {
        std::arch::x86_64::CpuidResult {
            eax: 0,
            ebx: 0,
            ecx: 0,
            edx: 0,
        }
    };
    let bit = |word: u32, n: u32| word & (1 << n) != 0;
    let family_id = (leaf1.eax >> 8) & 0xf;
    let model_id = (leaf1.eax >> 4) & 0xf;
    let family = if family_id == 0xf {
        family_id + ((leaf1.eax >> 20) & 0xff)
    } else {
        family_id
    };
    let model = if family_id == 0xf || family_id == 0x6 {
        model_id + (((leaf1.eax >> 16) & 0xf) << 4)
    } else {
        model_id
    };
    let osxsave = bit(leaf1.ecx, 27);
    let xcr0 = if osxsave {
        // SAFETY: OSXSAVE set means the OS enabled XGETBV.
        unsafe { xgetbv0() }
    } else {
        0
    };
    CpuFeatures {
        vendor,
        family,
        model,
        stepping: leaf1.eax & 0xf,
        sse42: bit(leaf1.ecx, 20),
        fma: bit(leaf1.ecx, 12),
        movbe: bit(leaf1.ecx, 22),
        osxsave,
        avx: bit(leaf1.ecx, 28),
        f16c: bit(leaf1.ecx, 29),
        bmi1: bit(leaf7.ebx, 3),
        avx2: bit(leaf7.ebx, 5),
        bmi2: bit(leaf7.ebx, 8),
        avx512f: bit(leaf7.ebx, 16),
        avx512dq: bit(leaf7.ebx, 17),
        avx512cd: bit(leaf7.ebx, 28),
        avx512bw: bit(leaf7.ebx, 30),
        avx512vl: bit(leaf7.ebx, 31),
        xcr0,
    }
}

/// `xgetbv(0)`.
///
/// # Safety
/// The OS must have set `CR4.OSXSAVE` (the `osxsave` cpuid bit).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "xsave")]
unsafe fn xgetbv0() -> u64 {
    // SAFETY: the caller's contract.
    unsafe { std::arch::x86_64::_xgetbv(0) }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn cpu_features() -> CpuFeatures {
    CpuFeatures {
        vendor: format!("not x86-64 ({})", std::env::consts::ARCH),
        ..CpuFeatures::default()
    }
}

/// The CPU line for `hook.log`.
pub fn cpu_report() -> String {
    cpu_features().report()
}

// ---------- installing ----------

/// Installs the pieces, each on its own, logging each outcome. Windows x64
/// only, as the step gate is.
pub fn install(resolved: &ResolvedProfile) {
    log::line(&cpu_report());
    match seed_api(resolved) {
        Ok(api) => {
            install_seed_api(api);
        }
        Err(missing) => log::line(&format!(
            "seeds: the profile lacks {missing}; the game-script states are not reseeded"
        )),
    }
    native::install(resolved);
}

/// The reseed's Lua 5.2 API from the profile's targets, or the first name
/// missing.
#[allow(clippy::missing_transmute_annotations)]
fn seed_api(resolved: &ResolvedProfile) -> Result<SeedApi, &'static str> {
    macro_rules! function {
        ($name:literal) => {{
            let address = resolved
                .get($name)
                .map(|target| target.address as usize)
                .filter(|address| *address != 0)
                .ok_or($name)?;
            // SAFETY: the profile resolved this function of Lua 5.2's C API
            // by its signature and prologue in the running build; the type
            // is the field's, that function's signature.
            unsafe { std::mem::transmute::<usize, _>(address) }
        }};
    }
    Ok(SeedApi {
        gettop: function!("lua_gettop"),
        settop: function!("lua_settop"),
        checkstack: function!("lua_checkstack"),
        type_of: function!("lua_type"),
        rawgeti: function!("lua_rawgeti"),
        getfield: function!("lua_getfield"),
        pushnumber: function!("lua_pushnumber"),
        pcall: PCall::Lua52(function!("lua_pcallk")),
        globals: lua::LUA52_GLOBALS,
    })
}

#[cfg(all(windows, target_arch = "x86_64"))]
mod native {
    use super::*;

    /// The trampolines to the originals; 0 until installed. The thunks
    /// read them.
    static GAME_SCRIPT_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static UPDATE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
    static TOWN_DEVELOP_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

    /// A detour that keeps the target's ABI whatever it is: saves the four
    /// integer and the four floating-point argument registers, calls
    /// `$before(rcx, rdx, r8, r9)` on the Rust side, restores them and
    /// jumps to the original's trampoline with the stack exactly as the
    /// caller left it, so stack arguments and the return address are the
    /// original's. Before the trampoline is published it returns to the
    /// caller without running the original (the window inside install,
    /// while no world exists).
    macro_rules! thunk {
        ($name:ident, $before:path, $original:ident) => {
            #[unsafe(naked)]
            unsafe extern "C" fn $name() {
                core::arch::naked_asm!(
                    "cmp qword ptr [rip + {orig}], 0",
                    "je 2f",
                    // 0x20 shadow space for the call, 4 register slots,
                    // 4 xmm slots; entry rsp is 8 mod 16, so this makes it
                    // 16-aligned for the call.
                    "sub rsp, 0x88",
                    "mov [rsp + 0x20], rcx",
                    "mov [rsp + 0x28], rdx",
                    "mov [rsp + 0x30], r8",
                    "mov [rsp + 0x38], r9",
                    "movdqu [rsp + 0x40], xmm0",
                    "movdqu [rsp + 0x50], xmm1",
                    "movdqu [rsp + 0x60], xmm2",
                    "movdqu [rsp + 0x70], xmm3",
                    "call {before}",
                    "movdqu xmm3, [rsp + 0x70]",
                    "movdqu xmm2, [rsp + 0x60]",
                    "movdqu xmm1, [rsp + 0x50]",
                    "movdqu xmm0, [rsp + 0x40]",
                    "mov r9, [rsp + 0x38]",
                    "mov r8, [rsp + 0x30]",
                    "mov rdx, [rsp + 0x28]",
                    "mov rcx, [rsp + 0x20]",
                    "add rsp, 0x88",
                    "jmp qword ptr [rip + {orig}]",
                    "2:",
                    "ret",
                    before = sym $before,
                    orig = sym $original,
                )
            }
        };
    }

    thunk!(
        game_script_thunk,
        before_game_script_c,
        GAME_SCRIPT_ORIGINAL
    );
    thunk!(update_thunk, before_update_c, UPDATE_ORIGINAL);
    thunk!(
        town_develop_thunk,
        before_town_develop_c,
        TOWN_DEVELOP_ORIGINAL
    );

    /// The Rust side of each thunk: never panics out (a panic would abort
    /// the game at the `extern "C"` boundary).
    extern "C" fn before_game_script_c(
        _this: usize,
        _registry: usize,
        state_ref: usize,
        flag: usize,
    ) {
        let _ = catch_unwind(|| {
            // r8 is `lua::State&`, whose first word is the `lua_State*`.
            if !crate::image::readable(state_ref, std::mem::size_of::<usize>()) {
                log::line(&format!(
                    "seeds: {GAME_SCRIPT_TARGET} ran with an unreadable lua::State& {state_ref:#x}; no state marked"
                ));
                return;
            }
            // SAFETY: the word at `state_ref` is readable, checked above.
            let state = unsafe { std::ptr::read_unaligned(state_ref as *const usize) };
            if state == 0 {
                log::line(&format!(
                    "seeds: {GAME_SCRIPT_TARGET} ran with a lua::State whose lua_State* is null; no state marked"
                ));
                return;
            }
            mark_game_script(state, flag & 0xff != 0);
        });
    }

    extern "C" fn before_update_c(_engine: usize, _b: usize, _c: usize, _d: usize) {
        let _ = catch_unwind(before_update);
    }

    extern "C" fn before_town_develop_c(_a: usize, _b: usize, _c: usize, _d: usize) {
        let _ = catch_unwind(before_town_develop);
    }

    /// Detours `target` to `thunk` for the life of the game and publishes
    /// the trampoline in `original`.
    fn detour(
        resolved: &ResolvedProfile,
        target: &str,
        thunk: unsafe extern "C" fn(),
        original: &AtomicUsize,
    ) -> Result<(), String> {
        let address = resolved
            .get(target)
            .map(|target| target.address as usize as *mut u8)
            .ok_or_else(|| format!("the profile has no target {target:?}"))?;
        // SAFETY: a function of the running game the profile resolved and
        // prologue-checked, detoured while the game starts, before a world
        // exists (the quiescence rule in docs/HOOKS.md); the thunk keeps
        // any ABI (it saves and restores every argument register and
        // leaves the stack as it found it).
        let installed = unsafe {
            tpf3mp_hookcore::detour::InlineDetour::install(address, thunk as *const () as *const u8)
        }
        .map_err(|error| format!("{target}: {error}"))?;
        original.store(installed.trampoline() as usize, Ordering::SeqCst);
        std::mem::forget(installed);
        Ok(())
    }

    /// The CRT's `srand`, from the UCRT the game imports `rand` from
    /// (`api-ms-win-crt-utility-l1-1-0.dll`, which forwards to
    /// `ucrtbase.dll`), so the state seeded is the one the applier reads.
    fn resolve_srand() -> Result<usize, String> {
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
        for module in ["api-ms-win-crt-utility-l1-1-0.dll", "ucrtbase.dll"] {
            let wide: Vec<u16> = module.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: a NUL-terminated wide string; the handle of a loaded
            // module is not owned by us.
            let handle = unsafe { GetModuleHandleW(wide.as_ptr()) };
            if handle.is_null() {
                continue;
            }
            // SAFETY: a valid module handle and a NUL-terminated name.
            let address = unsafe { GetProcAddress(handle, c"srand".as_ptr().cast::<u8>()) };
            if let Some(function) = address {
                return Ok(function as usize);
            }
        }
        Err("no loaded CRT module exports srand".to_owned())
    }

    pub(super) fn install(resolved: &ResolvedProfile) {
        install_script_reseed(resolved);
        install_srand(resolved);
    }

    fn install_script_reseed(resolved: &ResolvedProfile) {
        match detour(
            resolved,
            GAME_SCRIPT_TARGET,
            game_script_thunk,
            &GAME_SCRIPT_ORIGINAL,
        ) {
            Ok(()) => log::line(&format!(
                "seeds: detour installed on {GAME_SCRIPT_TARGET}; the game-script states will be marked as the game registers them"
            )),
            Err(error) => log::line(&format!(
                "seeds: no game-script state marker ({error}); the per-step math.random reseed covers nothing"
            )),
        }
        match detour(resolved, UPDATE_TARGET, update_thunk, &UPDATE_ORIGINAL) {
            Ok(()) => {
                UPDATE_HOOKED.store(true, Ordering::SeqCst);
                log::line(&format!(
                    "seeds: detour installed on {UPDATE_TARGET}; the game-script states are reseeded from the room's step before each update"
                ));
            }
            Err(error) => log::line(&format!(
                "seeds: no per-update hook ({error}); the game-script math.random is not reseeded"
            )),
        }
    }

    fn install_srand(resolved: &ResolvedProfile) {
        let srand = match resolve_srand() {
            Ok(address) => {
                SRAND.store(address, Ordering::SeqCst);
                format!("srand at {address:#x}")
            }
            Err(error) => format!("srand not found: {error}"),
        };
        if !TOWN_DEVELOP_RESEED {
            log::line(&format!(
                "seeds: {TOWN_DEVELOP_TARGET} reseed off (TOWN_DEVELOP_RESEED is false; the command is refused in a room); {srand}"
            ));
        } else if SRAND.load(Ordering::SeqCst) == 0 {
            log::line(&format!("seeds: {TOWN_DEVELOP_TARGET} reseed off: {srand}"));
        } else {
            match detour(
                resolved,
                TOWN_DEVELOP_TARGET,
                town_develop_thunk,
                &TOWN_DEVELOP_ORIGINAL,
            ) {
                Ok(()) => log::line(&format!(
                    "seeds: detour installed on {TOWN_DEVELOP_TARGET}; its rand() is seeded from the step ({srand})"
                )),
                Err(error) => log::line(&format!(
                    "seeds: {TOWN_DEVELOP_TARGET} reseed off ({error})"
                )),
            }
        }
    }
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
mod native {
    use super::*;

    pub(super) fn install(_resolved: &ResolvedProfile) {
        log::line(
            "seeds: the detours are installed on Windows x64 only so far; nothing is reseeded",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests that touch the process-wide roster and batch run one at a
    /// time.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn the_seed_is_a_pure_function_of_step_and_salt_in_the_mt_and_minstd_range() {
        assert_eq!(seed_for(0, 0), seed_for(0, 0));
        assert_eq!(seed_for(12_345, 7), seed_for(12_345, 7));
        assert_ne!(seed_for(1, 0), seed_for(2, 0));
        assert_ne!(seed_for(1, 0), seed_for(1, 1));
        assert_ne!(seed_for(0, 0), seed_for(u64::MAX, 0));
        for step in (0..10_000).chain([u64::MAX, u64::MAX - 1, 1 << 40]) {
            for salt in [GAME_SCRIPT_SALT, TOWN_DEVELOP_SALT, u32::MAX] {
                let seed = seed_for(step, salt);
                assert!((1..=0x7fff_ffff).contains(&seed), "{step} {salt} -> {seed}");
            }
        }
        // Pinned: every replica derives exactly this, whatever its build.
        assert_eq!(seed_for(1, GAME_SCRIPT_SALT), 1_216_681_719);
        assert_eq!(seed_for(50, GAME_SCRIPT_SALT), 1_568_932_195);
    }

    #[test]
    fn a_batch_hands_one_step_per_update_and_disarms_otherwise() {
        let mut batch = Batch::default();
        assert!(!batch.armed());
        assert_eq!(batch.next_update(), None);
        assert_eq!(batch.town_apply(), None);

        assert_eq!(batch.arm(Some(10), Updates::Exactly(3)), None);
        assert!(batch.armed());
        assert_eq!(batch.town_apply(), Some((10, 0)));
        assert_eq!(batch.town_apply(), Some((10, 1)));
        assert_eq!(batch.next_update(), Some(10));
        assert_eq!(
            batch.town_apply(),
            Some((11, 0)),
            "the count restarts per step"
        );
        assert_eq!(batch.next_update(), Some(11));
        assert_eq!(batch.next_update(), Some(12));
        assert!(!batch.armed());
        assert_eq!(
            batch.next_update(),
            None,
            "an update the room did not release gets no step"
        );
        assert_eq!(batch.town_apply(), None);
        // Four calls for three updates: the next arm says so.
        assert_eq!(batch.arm(Some(13), Updates::Exactly(1)), Some((4, 3)));
        assert_eq!(batch.next_update(), Some(13));
        // A matching batch reports nothing.
        assert_eq!(batch.arm(Some(14), Updates::Exactly(2)), None);
        assert_eq!(batch.next_update(), Some(14));
        // Fewer calls than updates is a mismatch too.
        assert_eq!(batch.arm(Some(16), Updates::Exactly(1)), Some((1, 2)));

        // Disarmed: no step known, the game's own speed, or paused.
        for (step, updates) in [
            (None, Updates::Exactly(2)),
            (Some(20), Updates::Own),
            (Some(20), Updates::Exactly(0)),
        ] {
            batch.arm(Some(1), Updates::Exactly(1));
            batch.next_update();
            batch.arm(step, updates);
            assert!(!batch.armed(), "{step:?} {updates:?}");
            assert_eq!(batch.next_update(), None);
            assert_eq!(batch.town_apply(), None);
            assert_eq!(
                batch.arm(Some(1), Updates::Exactly(1)),
                None,
                "a disarmed batch reports no mismatch"
            );
        }
    }

    #[test]
    fn the_statics_arm_from_the_driver_and_the_roster_records_states() {
        // The batch and the roster are process-wide; this test only checks
        // the wrappers do not panic and the roster keeps what it saw.
        let _serial = lock(&SERIAL);
        lock(&ROSTER).marked.clear();
        before_updates(None, Updates::Own);
        before_updates(Some(5), Updates::Exactly(2));
        assert!(lock(&BATCH).armed());
        before_updates(Some(7), Updates::Exactly(0));
        assert!(!lock(&BATCH).armed());
        saw_state(0x1000, "test");
        assert!(lock(&ROSTER).seen.iter().any(|seen| seen.state == 0x1000));
        let report = roster_report();
        assert!(report.contains("0x1000 from test"), "{report}");
        assert!(report.contains("covers []"), "{report}");
    }

    #[test]
    fn each_released_update_offers_its_steps_seed_and_nothing_else_does() {
        let _serial = lock(&SERIAL);
        before_updates(Some(5), Updates::Exactly(2));
        before_update();
        assert_eq!(current_seed(), Some(seed_for(5, GAME_SCRIPT_SALT)));
        before_update();
        assert_eq!(current_seed(), Some(seed_for(6, GAME_SCRIPT_SALT)));
        // An update the room did not release, and the game's own speed.
        before_update();
        assert_eq!(current_seed(), None);
        before_updates(None, Updates::Own);
        before_update();
        assert_eq!(current_seed(), None);
    }

    #[test]
    fn game_script_marks_group_by_game_and_retire_the_last_games() {
        let _serial = lock(&SERIAL);
        lock(&ROSTER).marked.clear();
        mark_game_script(0x2000, true);
        mark_game_script(0x2001, false);
        {
            let roster = lock(&ROSTER);
            let states: Vec<usize> = roster.marked.iter().map(|m| m.state).collect();
            assert!(states.ends_with(&[0x2000, 0x2001]), "{states:?}");
            assert!(roster.marked.iter().any(|m| m.state == 0x2000 && m.flag));
            assert!(roster.marked.iter().any(|m| m.state == 0x2001 && !m.flag));
        }
        // A third mark: a new game; the old pair is retired.
        mark_game_script(0x3000, true);
        {
            let roster = lock(&ROSTER);
            let states: Vec<usize> = roster.marked.iter().map(|m| m.state).collect();
            assert_eq!(states, vec![0x3000]);
        }
        // The same pointer again (reused after a teardown): a new game too.
        mark_game_script(0x3001, false);
        mark_game_script(0x3000, true);
        let roster = lock(&ROSTER);
        let states: Vec<usize> = roster.marked.iter().map(|m| m.state).collect();
        assert_eq!(states, vec![0x3000]);
    }

    fn avx2_cpu() -> CpuFeatures {
        CpuFeatures {
            vendor: "GenuineIntel".into(),
            sse42: true,
            fma: true,
            movbe: true,
            osxsave: true,
            avx: true,
            f16c: true,
            bmi1: true,
            avx2: true,
            bmi2: true,
            xcr0: 0x7,
            ..CpuFeatures::default()
        }
    }

    #[test]
    fn the_crt_isa_level_is_estimated_by_the_published_rule() {
        let avx2 = avx2_cpu();
        assert_eq!(avx2.isa_level(), (4, "AVX2"));
        assert!(avx2.fma3_math());

        let no_fma = CpuFeatures {
            fma: false,
            ..avx2_cpu()
        };
        assert_eq!(no_fma.isa_level(), (3, "AVX"), "AVX2 needs FMA3 and BMI");
        assert!(!no_fma.fma3_math());

        let os_without_avx = CpuFeatures {
            xcr0: 0x3,
            ..avx2_cpu()
        };
        assert_eq!(os_without_avx.isa_level(), (2, "SSE4.2"));

        let avx512 = CpuFeatures {
            avx512f: true,
            avx512cd: true,
            avx512bw: true,
            avx512dq: true,
            avx512vl: true,
            xcr0: 0xe7,
            ..avx2_cpu()
        };
        assert_eq!(avx512.isa_level(), (5, "AVX-512"));
        let avx512_os_off = CpuFeatures {
            xcr0: 0x7,
            ..avx512.clone()
        };
        assert_eq!(avx512_os_off.isa_level(), (4, "AVX2"));

        let old = CpuFeatures::default();
        assert_eq!(old.isa_level(), (1, "SSE2"));
        let sse42 = CpuFeatures {
            sse42: true,
            ..CpuFeatures::default()
        };
        assert_eq!(sse42.isa_level(), (2, "SSE4.2"));

        let report = avx2.report();
        assert!(report.contains("GenuineIntel"), "{report}");
        assert!(report.contains("__isa_available 4 (AVX2)"), "{report}");
        assert!(report.contains("FMA3 bodies: yes"), "{report}");
    }

    #[test]
    fn the_running_cpu_reports_itself() {
        let features = cpu_features();
        #[cfg(target_arch = "x86_64")]
        {
            assert!(!features.vendor.is_empty());
            assert!(features.osxsave || features.xcr0 == 0);
        }
        let report = features.report();
        assert!(report.starts_with("cpu: "), "{report}");
        assert_eq!(cpu_report(), report);
    }

    mod lua51 {
        use std::ffi::{c_char, c_int};

        use mlua::ffi;

        use crate::lua::State;

        pub unsafe extern "C-unwind" fn gettop(l: State) -> c_int {
            unsafe { ffi::lua_gettop(l.cast()) }
        }
        pub unsafe extern "C-unwind" fn settop(l: State, index: c_int) {
            unsafe { ffi::lua_settop(l.cast(), index) }
        }
        pub unsafe extern "C-unwind" fn checkstack(l: State, n: c_int) -> c_int {
            unsafe { ffi::lua_checkstack(l.cast(), n) }
        }
        pub unsafe extern "C-unwind" fn type_of(l: State, index: c_int) -> c_int {
            unsafe { ffi::lua_type(l.cast(), index) }
        }
        pub unsafe extern "C-unwind" fn rawgeti(l: State, index: c_int, n: c_int) {
            unsafe { ffi::lua_rawgeti_(l.cast(), index, n) }
        }
        pub unsafe extern "C-unwind" fn getfield(l: State, index: c_int, k: *const c_char) {
            unsafe { ffi::lua_getfield_(l.cast(), index, k) }
        }
        pub unsafe extern "C-unwind" fn pushnumber(l: State, n: f64) {
            unsafe { ffi::lua_pushnumber(l.cast(), n) }
        }
        pub unsafe extern "C-unwind" fn pcall(l: State, n: c_int, r: c_int, f: c_int) -> c_int {
            unsafe { ffi::lua_pcall(l.cast(), n, r, f) }
        }
    }

    fn seed_api51() -> SeedApi {
        SeedApi {
            gettop: lua51::gettop,
            settop: lua51::settop,
            checkstack: lua51::checkstack,
            type_of: lua51::type_of,
            rawgeti: lua51::rawgeti,
            getfield: lua51::getfield,
            pushnumber: lua51::pushnumber,
            pcall: PCall::Lua51(lua51::pcall),
            globals: lua::Globals::Pseudo(mlua::ffi::LUA_GLOBALSINDEX),
        }
    }

    /// The reseed calls the state's own `math.randomseed` with the step's
    /// seed and leaves the stack as it was; a state without it is refused
    /// with the reason, the stack restored too.
    #[test]
    fn the_reseed_calls_math_randomseed_with_the_steps_seed() {
        let api = seed_api51();
        let lua = crate::lua::tests::Lua::new();
        lua.run("SEEN = nil; math.randomseed = function(n) SEEN = n end")
            .unwrap();
        let seed = seed_for(42, GAME_SCRIPT_SALT);
        let top = unsafe { (api.gettop)(lua.state()) };
        unsafe { reseed_state(&api, lua.state(), seed) }.unwrap();
        assert_eq!(unsafe { (api.gettop)(lua.state()) }, top);
        assert_eq!(lua.run("return SEEN").unwrap(), seed.to_string());

        lua.run("math = nil").unwrap();
        let refused = unsafe { reseed_state(&api, lua.state(), seed) }.unwrap_err();
        assert!(refused.contains("math is nil"), "{refused}");
        assert_eq!(unsafe { (api.gettop)(lua.state()) }, top);

        lua.run("math = { randomseed = function() error('no') end }")
            .unwrap();
        let raised = unsafe { reseed_state(&api, lua.state(), seed) }.unwrap_err();
        assert!(raised.contains("raised"), "{raised}");
        assert_eq!(unsafe { (api.gettop)(lua.state()) }, top);
    }
}

//! Installs the step gate in the running game: finds `GameSim::Step` with
//! the matched profile in the game's own mapped image, attaches the session
//! to the agent, and detours the step to [`crate::step::StepDriver`].
//!
//! Windows only for now: the one profile so far is Steam build 40408 on
//! Windows, and on other systems the hook installs nothing (fail closed).

#![allow(unsafe_code)]
// Elsewhere install_inner installs nothing, so the detours are unused there.
#![cfg_attr(not(all(windows, target_arch = "x86_64")), allow(dead_code))]

use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

use tpf3mp_hookcore::profile::Profile;

use crate::step::StepHandler;

/// The profile's name for the simulation step.
pub const STEP_TARGET: &str = "GameSim::Step";
/// The profile's name for the speed the step reads.
pub const SPEED_TARGET: &str = "CGameTime::GetSpeed";

/// The game's own step, reached through the detour's trampoline.
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// The driver the detour hands each call to.
static DRIVER: Mutex<Option<Box<dyn StepHandler>>> = Mutex::new(None);
/// Set when the detour itself failed (a panic): from then on it runs
/// nothing, holding the world, as the driver does on an error.
static BROKEN: AtomicBool = AtomicBool::new(false);
/// Where the driver's log lines go.
static LOG: Mutex<Option<crate::Logger>> = Mutex::new(None);
/// The game's own speed getter, reached through its detour's trampoline.
static SPEED_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// In the room's game: the speed getter answers 1.
static IN_ROOM: AtomicBool = AtomicBool::new(false);
/// The game's own speed (the speed row) as the getter last read it in the
/// room's game; `NO_SPEED` until then.
static CHOSEN: AtomicU64 = AtomicU64::new(NO_SPEED);
const NO_SPEED: u64 = u64::MAX;

/// The speed getter's signature, passed through as the step's is.
type SpeedFn = unsafe extern "C" fn(usize, usize, usize, usize) -> u64;

/// The speed getter's detour. In the room's game the room sets the pace (the
/// step gate releases steps at its speed), so the game's own speed is held at
/// one update per call of its step, whatever the speed row or a key says,
/// paused included: the room's pause is the only pause. Otherwise the game's
/// own answer.
unsafe extern "C" fn speed_detour(this: usize, a: usize, b: usize, c: usize) -> u64 {
    let original = SPEED_ORIGINAL.load(Ordering::Acquire);
    if original == 0 {
        return 1;
    }
    // SAFETY: the trampoline InlineDetour::install returned for the getter,
    // which keeps the game's own ABI.
    let original: SpeedFn = unsafe { std::mem::transmute::<usize, SpeedFn>(original) };
    // SAFETY: the game's own getter, called as the game called it.
    let own = unsafe { original(this, a, b, c) };
    if IN_ROOM.load(Ordering::Acquire) {
        // The speed row's value is the player's request to the room (the
        // step detour passes it on); the game runs one update a call. The
        // getter returns an int: its low 32 bits.
        CHOSEN.store(u64::from(own as u32), Ordering::Release);
        return 1;
    }
    own
}

/// The step's signature: a member function, `this` and its arguments in
/// the first registers. All four are passed on unchanged, so the detour is
/// transparent whatever the game's step takes in them.
type StepFn = unsafe extern "C" fn(usize, usize, usize, usize);

/// The detour: every call of the game's step comes here.
unsafe extern "C" fn step_detour(this: usize, a: usize, b: usize, c: usize) {
    let original = ORIGINAL.load(Ordering::Acquire);
    if original == 0 || BROKEN.load(Ordering::Acquire) {
        return;
    }
    // SAFETY: ORIGINAL holds the trampoline InlineDetour::install returned
    // for this function, which keeps the game's own ABI.
    let original: StepFn = unsafe { std::mem::transmute::<usize, StepFn>(original) };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut driver = DRIVER.lock().unwrap_or_else(|poison| poison.into_inner());
        let Some(driver) = driver.as_mut() else {
            // SAFETY: the game's own step, called as the game called it.
            unsafe { original(this, a, b, c) };
            return;
        };
        // SAFETY: as above, once per step the room released.
        driver.on_step(&mut || unsafe { original(this, a, b, c) });
        IN_ROOM.store(driver.in_room(), Ordering::Release);
        let chosen = CHOSEN.load(Ordering::Acquire);
        if chosen != NO_SPEED {
            driver.chosen_speed(chosen);
        }
        let lines = driver.take_log();
        if !lines.is_empty()
            && let Some(log) = LOG.lock().unwrap_or_else(|p| p.into_inner()).as_mut()
        {
            for line in lines {
                log.line(&line);
            }
        }
    }));
    if result.is_err() {
        BROKEN.store(true, Ordering::Release);
    }
}

/// Installs a detour for the life of the process and returns its trampoline.
///
/// # Safety
///
/// As [`tpf3mp_hookcore::detour::InlineDetour::install`]: `target` is a
/// function in this process that no thread is running, and `detour` has its
/// ABI.
#[cfg(target_arch = "x86_64")]
unsafe fn detour_forever(target: *mut u8, detour: *const u8) -> Result<usize, String> {
    // SAFETY: the caller's.
    let installed = unsafe { tpf3mp_hookcore::detour::InlineDetour::install(target, detour) }
        .map_err(|error| format!("{error:?}"))?;
    let trampoline = installed.trampoline() as usize;
    std::mem::forget(installed);
    Ok(trampoline)
}

/// What installing came to.
pub enum Installed {
    Yes { step_rva: u64 },
    No(String),
}

/// Resolves the profile in the running image, attaches the session and
/// detours the step. Any failure installs nothing.
pub fn install(profile: &Profile, link_name: &str, log: crate::Logger) -> Installed {
    *LOG.lock().unwrap_or_else(|p| p.into_inner()) = Some(log);
    match install_inner(profile, link_name) {
        Ok(step_rva) => Installed::Yes { step_rva },
        Err(reason) => Installed::No(reason),
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn install_inner(profile: &Profile, link_name: &str) -> Result<u64, String> {
    use std::time::Duration;

    use tpf3mp_hookcore::{pe::PeHeaders, profile};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

    // SAFETY: a null name asks for the executable's own module handle, its
    // base address, which stays mapped for the life of the process.
    let base = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
    if base == 0 {
        return Err("cannot find the game's module".into());
    }
    // SAFETY: the first page of a mapped module holds its headers.
    let head = unsafe { std::slice::from_raw_parts(base as *const u8, 0x1000) };
    let pe = PeHeaders::parse(head).map_err(|error| format!("the game's headers: {error:?}"))?;
    let text = pe.section(".text").ok_or("the game has no .text section")?;
    // SAFETY: .text is mapped at base + its virtual address for its virtual
    // size, and is only read here.
    let code = unsafe {
        std::slice::from_raw_parts(
            (base + text.virtual_address as usize) as *const u8,
            text.virtual_size as usize,
        )
    };
    let resolved = profile::resolve(profile, code, u64::from(text.virtual_address))
        .map_err(|refusal| format!("the profile does not resolve here: {refusal:?}"))?;
    let step_rva = resolved
        .get(STEP_TARGET)
        .ok_or_else(|| format!("the profile has no {STEP_TARGET}"))?
        .address;
    // Without the speed held, one call of the step could run several
    // updates: no room's game without it (fail closed).
    let speed_rva = resolved
        .get(SPEED_TARGET)
        .ok_or_else(|| format!("the profile has no {SPEED_TARGET}"))?
        .address;

    let session = tpf3mp_bridge::Session::attach(link_name, &profile.name, Duration::from_secs(30))
        .map_err(|error| format!("the agent's link: {error}"))?;
    *DRIVER.lock().unwrap_or_else(|p| p.into_inner()) =
        Some(Box::new(crate::step::StepDriver::new(session)));

    // SAFETY: both targets are functions the profile resolved, exactly once,
    // in this process's code; the game has not run a step yet (the hook
    // installs while the game starts, before any world is loaded); each
    // detour has its target's ABI (four register arguments passed through).
    // The getter goes first, so the step never runs with the room's pace but
    // the game's speed.
    let speed = unsafe {
        detour_forever(
            (base + speed_rva as usize) as *mut u8,
            speed_detour as *const u8,
        )
    }
    .map_err(|error| format!("detouring {SPEED_TARGET}: {error}"))?;
    SPEED_ORIGINAL.store(speed, Ordering::Release);
    // SAFETY: as above.
    let step = unsafe {
        detour_forever(
            (base + step_rva as usize) as *mut u8,
            step_detour as *const u8,
        )
    }
    .map_err(|error| format!("detouring {STEP_TARGET}: {error}"))?;
    ORIGINAL.store(step, Ordering::Release);
    Ok(step_rva)
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
fn install_inner(_profile: &Profile, _link_name: &str) -> Result<u64, String> {
    Err("the step gate is installed on Windows x64 only so far".into())
}

#[cfg(all(test, windows, target_arch = "x86_64"))]
mod tests {
    use std::sync::atomic::AtomicU64;

    use tpf3mp_bridge::{Load, StepGate};
    use tpf3mp_hookcore::detour::InlineDetour;

    use super::*;
    use crate::step::{
        StepDriver,
        tests::{Script, begin},
    };

    static CALLS: AtomicU64 = AtomicU64::new(0);
    static ARGS_OK: AtomicBool = AtomicBool::new(true);

    /// A stand-in for the game's step, in this test binary: long enough a
    /// prologue for the detour engine to steal, and it checks the arguments
    /// arrive unchanged.
    #[inline(never)]
    extern "C" fn fake_step(this: usize, a: usize, b: usize, c: usize) {
        if (this, a, b, c) != (0x1111, 0x2222, 0x3333, 0x4444) {
            ARGS_OK.store(false, Ordering::SeqCst);
        }
        CALLS.fetch_add(std::hint::black_box(1), Ordering::SeqCst);
    }

    static SPEED: AtomicU64 = AtomicU64::new(4);

    /// A stand-in for the game's speed getter, with a prologue long enough
    /// to steal.
    #[inline(never)]
    extern "C" fn fake_speed(this: usize, a: usize, b: usize, c: usize) -> u64 {
        let noise = std::hint::black_box(this ^ a ^ b ^ c) as u64;
        let zero = std::hint::black_box(0u64);
        SPEED.load(Ordering::SeqCst) + noise * zero
    }

    #[test]
    fn in_the_rooms_game_the_speed_is_one_and_otherwise_the_games() {
        let target = fake_speed as *mut u8;
        // SAFETY: fake_speed is this binary's own function, not running now,
        // and speed_detour has its signature.
        let detour = unsafe { InlineDetour::install(target, speed_detour as *const u8) }.unwrap();
        SPEED_ORIGINAL.store(detour.trampoline() as usize, Ordering::Release);
        let speed: extern "C" fn(usize, usize, usize, usize) -> u64 =
            std::hint::black_box(fake_speed);
        IN_ROOM.store(false, Ordering::Release);
        assert_eq!(speed(1, 2, 3, 4), 4, "outside a room, the game's own speed");
        IN_ROOM.store(true, Ordering::Release);
        assert_eq!(
            speed(1, 2, 3, 4),
            1,
            "in the room's game, one update a call"
        );
        assert_eq!(
            CHOSEN.load(Ordering::SeqCst),
            4,
            "the speed row's value is kept"
        );
        SPEED.store(0, Ordering::SeqCst);
        assert_eq!(
            speed(1, 2, 3, 4),
            1,
            "the game's own pause does not stop the room"
        );
        assert_eq!(
            CHOSEN.load(Ordering::SeqCst),
            0,
            "but it asks the room to pause"
        );
        CHOSEN.store(NO_SPEED, Ordering::SeqCst);
        IN_ROOM.store(false, Ordering::Release);
        SPEED_ORIGINAL.store(0, Ordering::Release);
        // SAFETY: nothing runs fake_speed now.
        unsafe { detour.detach() }.unwrap();
        SPEED.store(4, Ordering::SeqCst);
    }

    #[test]
    fn the_detour_runs_the_games_step_once_per_released_step() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        script.gates.extend([
            StepGate::Load(Load {
                file: None,
                next_step: 1,
            }),
            StepGate::Run,
            StepGate::Run,
            StepGate::Run,
            StepGate::Wait,
        ]);
        *DRIVER.lock().unwrap() = Some(Box::new(StepDriver::new(script)));
        let target = fake_step as *mut u8;
        // SAFETY: fake_step is this binary's own function, not running now,
        // and step_detour has its signature.
        let detour = unsafe { InlineDetour::install(target, step_detour as *const u8) }.unwrap();
        ORIGINAL.store(detour.trampoline() as usize, Ordering::Release);

        let step: extern "C" fn(usize, usize, usize, usize) = std::hint::black_box(fake_step);
        // The room released three steps: one call of the game's step runs
        // three.
        step(0x1111, 0x2222, 0x3333, 0x4444);
        assert_eq!(CALLS.load(Ordering::SeqCst), 3);
        // Withheld: the game's step does not run at all.
        step(0x1111, 0x2222, 0x3333, 0x4444);
        assert_eq!(CALLS.load(Ordering::SeqCst), 3);
        assert!(
            ARGS_OK.load(Ordering::SeqCst),
            "the arguments reached the step unchanged"
        );
        assert!(!BROKEN.load(Ordering::SeqCst));

        ORIGINAL.store(0, Ordering::Release);
        // SAFETY: nothing runs fake_step now.
        unsafe { detour.detach() }.unwrap();
        *DRIVER.lock().unwrap() = None;
        step(0x1111, 0x2222, 0x3333, 0x4444);
        assert_eq!(
            CALLS.load(Ordering::SeqCst),
            4,
            "detached, the step is the game's own again"
        );
    }
}

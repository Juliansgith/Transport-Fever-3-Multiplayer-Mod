//! The build tools in the room's game (docs/HOOKS.md, "The build tools").
//!
//! The street, track and construction tools of Transport Fever 3 are
//! native: they queue a `WorldBuildProposal` command, which the simulation
//! applies at its next step, in this game alone. Nothing on the Lua side can
//! stop one (the tools send no `builder.proposalPrepareForApply`, the game
//! ignores an error raised in `onPreBuildProposal`, and emptying the
//! proposal there crashes it), so the hook does, in two places:
//!
//! - **At the click**, `CommandList::Add` on the main thread: a player's
//!   build queued in the room's game is counted ([`clicks`]). The mod's GUI
//!   keeps the proposal each preview showed, marked with the count it saw,
//!   so the one it saw last before the count went up is the one clicked, and
//!   hands that to the room.
//! - **At the apply**, the simulation's `WorldBuildProposal` apply: a
//!   player-initiated build in the room's game answers false, as a build the
//!   game refused, unless it is the room's own, which the mod's game script
//!   applies with the flag [`set_replaying`] up. The game then tells the
//!   tool it failed, through its own path; the room orders the build for
//!   every game, this one included.
//!
//! The command's layout is the build's own (build 40408's): a `Command`'s
//! payload pointer at +0, the payload's variant index at +0x9b8 (the
//! dispatcher's case minus one), and a `WorldBuildProposal` payload's
//! `playerInitiated` at +0x3d2. A build whose profile has not these targets
//! installs nothing here, and the GUI keeps refusing the tools.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

/// Where a `Command` keeps its payload.
const COMMAND_PAYLOAD: usize = 0;
/// Where a payload keeps its variant index (build 40408).
const PAYLOAD_INDEX: usize = 0x9b8;
/// The variant index of a `WorldBuildProposal` (the dispatcher's case 53).
const WORLD_BUILD_PROPOSAL: i8 = 52;
/// Where a `WorldBuildProposal` payload keeps `playerInitiated`.
const PLAYER_INITIATED: usize = 0x3d2;

/// The game's own add and apply, reached through their detours'
/// trampolines.
static ADD_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static APPLY_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// Both detours are in: the tools may build through the room.
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// The player's builds queued in the room's game so far.
static CLICKS: AtomicU64 = AtomicU64::new(0);
/// The player's builds the apply answered false for.
static STOPPED: AtomicU64 = AtomicU64::new(0);
/// The mod's game script is applying the room's actions.
static REPLAYING: AtomicBool = AtomicBool::new(false);

/// Whether the tools build through the room: both detours are in.
pub fn installed() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// The player's builds queued in the room's game so far.
pub fn clicks() -> u64 {
    CLICKS.load(Ordering::Acquire)
}

/// The player's builds stopped at the apply so far.
pub fn stopped() -> u64 {
    STOPPED.load(Ordering::Acquire)
}

/// The mod's game script begins or ends applying the room's actions.
pub fn set_replaying(replaying: bool) {
    REPLAYING.store(replaying, Ordering::Release);
}

/// Whether `payload` is a `WorldBuildProposal`'s the player made.
///
/// # Safety
///
/// `payload` is a command payload of the game's, at least `PAYLOAD_INDEX + 1`
/// bytes.
unsafe fn player_build(payload: *const u8) -> bool {
    if payload.is_null() {
        return false;
    }
    // SAFETY: the caller's.
    unsafe {
        payload.add(PAYLOAD_INDEX).cast::<i8>().read_unaligned() == WORLD_BUILD_PROPOSAL
            && payload.add(PLAYER_INITIATED).read() == 1
    }
}

/// `CommandList::Add`'s signature: the list, the connection returned, the
/// command, the callback and the progress; returns the connection.
type AddFn = unsafe extern "C" fn(usize, usize, usize, usize, usize) -> usize;

/// The add's detour: counts the player's builds in the room's game, and
/// adds every command as the game would.
unsafe extern "C" fn add_detour(
    list: usize,
    connection: usize,
    command: usize,
    callback: usize,
    progress: usize,
) -> usize {
    let original = ADD_ORIGINAL.load(Ordering::Acquire);
    if command != 0 && crate::lua::in_room() {
        // SAFETY: the game passes the command it adds, whose first field is
        // its payload.
        let payload = unsafe { (command as *const usize).add(COMMAND_PAYLOAD).read() };
        // SAFETY: a command's payload, the size the dispatcher reads.
        if unsafe { player_build(payload as *const u8) } {
            CLICKS.fetch_add(1, Ordering::AcqRel);
        }
    }
    // SAFETY: the trampoline of the add, called with the arguments the game
    // passed.
    let original: AddFn = unsafe { std::mem::transmute::<usize, AddFn>(original) };
    unsafe { original(list, connection, command, callback, progress) }
}

/// The build apply's signature: the dispatcher's context and the payload;
/// returns whether it built.
type ApplyFn = unsafe extern "C" fn(usize, usize, usize, usize) -> u64;

/// The apply's detour: in the room's game, the player's own builds answer
/// false; the room's, and everyone else's (towns, the game's scripts), apply.
unsafe extern "C" fn apply_detour(context: usize, payload: usize, r8: usize, r9: usize) -> u64 {
    let original = APPLY_ORIGINAL.load(Ordering::Acquire);
    if crate::lua::in_room() && !REPLAYING.load(Ordering::Acquire) {
        // SAFETY: the dispatcher passes the payload it dispatched on.
        if unsafe { player_build(payload as *const u8) } {
            STOPPED.fetch_add(1, Ordering::AcqRel);
            return 0;
        }
    }
    // SAFETY: the trampoline of the apply, called as the dispatcher called it.
    let original: ApplyFn = unsafe { std::mem::transmute::<usize, ApplyFn>(original) };
    unsafe { original(context, payload, r8, r9) }
}

/// Detours the add and the apply, at the addresses the profile resolved.
///
/// # Safety
///
/// Both are the functions the profile names, in this process, which no
/// thread runs yet (the hook installs while the game starts).
#[cfg(all(windows, target_arch = "x86_64"))]
pub unsafe fn install(
    add: usize,
    apply: usize,
    detour: unsafe fn(*mut u8, *const u8) -> Result<usize, String>,
) -> Result<(), String> {
    // SAFETY: the caller's; each detour has its target's ABI.
    let add_original = unsafe { detour(add as *mut u8, add_detour as *const u8) }?;
    ADD_ORIGINAL.store(add_original, Ordering::Release);
    // SAFETY: as above.
    let apply_original = unsafe { detour(apply as *mut u8, apply_detour as *const u8) }?;
    APPLY_ORIGINAL.store(apply_original, Ordering::Release);
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A payload of the game's shape: `index` at the variant index and
    /// `player` at playerInitiated.
    fn payload(index: i8, player: u8) -> Vec<u8> {
        let mut bytes = vec![0u8; PAYLOAD_INDEX + 8];
        bytes[PAYLOAD_INDEX] = index as u8;
        bytes[PLAYER_INITIATED] = player;
        bytes
    }

    #[test]
    fn only_the_players_own_world_builds_count() {
        let build = payload(WORLD_BUILD_PROPOSAL, 1);
        let script = payload(WORLD_BUILD_PROPOSAL, 0);
        let other = payload(WORLD_BUILD_PROPOSAL + 1, 1);
        // SAFETY: each is a buffer of the size player_build reads.
        unsafe {
            assert!(player_build(build.as_ptr()));
            assert!(!player_build(script.as_ptr()), "a town's or a script's");
            assert!(!player_build(other.as_ptr()), "another command");
            assert!(!player_build(std::ptr::null()));
        }
    }
}

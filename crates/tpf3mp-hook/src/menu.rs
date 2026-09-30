//! Loading the room's world from the game's main menu (docs/HOOKS.md, "The
//! room's world"; `investigation/TPF3_MENU_JOIN_2026-09-30.md`).
//!
//! The mod's GUI runs only in a world, so a game at its main menu has no Lua
//! of the mod's to load the room's save. The menu itself loads saves from
//! Lua: its Load Game page builds a `SavegameId` with
//! `api.type.SavegameId.new()` and calls `app.loadGame(id, false, nil)`
//! (`gui/menu/savegame_react_util.tl`, build 40408). The hook does the same,
//! in the menu's own Lua state, on the menu's own frame:
//!
//! - **Finding the state.** The game gives a Lua state its `app` table in
//!   one function, `RegisterAppUsertypes(lua::State&, UI::CMenuUI&,
//!   std::function<bool()> const&)` (profile target
//!   [`REGISTER_APP_TARGET`]). The hook detours it and, after the game's own
//!   registration, runs [`CHUNK`] in that state ([`adopt`]): Lua that hands
//!   the hook a function loading a save by name, kept in the state's
//!   registry, and a sentinel whose `__gc` tells the hook the state is gone.
//!   Lua 5.2 runs every finalizer when a state closes, so the hook never
//!   calls into a state that no longer exists.
//! - **Running the load.** `UI::CMenuUI::DoStep`, the menu's per-frame
//!   update on the main thread ([`MENU_STEP_TARGET`]), is detoured; after
//!   the game's own frame, `crate::install::menu_frame` drives the room from
//!   there while this game has never stepped a world and no world's GUI has
//!   started, and a load the room ordered is started with [`serve`] in the
//!   newest state adopted on that thread.
//!
//! Without `app.setWaitForStartReadyGame()`, which the menu's own pages call
//! first, the game starts the loaded world by itself, with no Start Game
//! button (the game's `api/tealdef/app.d.tl`). A save the menu loads keeps its
//! own mod list (`info` is nil), and the room's save comes from a game whose
//! GUI had TPF3-MP linked, so the room's world loads with TPF3-MP active.
//!
//! Anything missing (a target, the Lua API, an adopted state on the menu's
//! thread) leaves the menu alone and the game as before: the room's world is
//! then loaded only by the GUI of a world the player has up.

#![allow(unsafe_code)]
// Elsewhere the detours are not installed, so their code is unused there.
#![cfg_attr(not(all(windows, target_arch = "x86_64")), allow(dead_code))]

use std::{
    ffi::{c_char, c_int, c_void},
    panic::AssertUnwindSafe,
    sync::{
        Mutex, MutexGuard, OnceLock, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    thread::ThreadId,
};

use crate::lua::{self, CFunction, LuaApi, State};

/// The profile's name for the function that gives a Lua state `app`.
pub const REGISTER_APP_TARGET: &str = "RegisterAppUsertypes";
/// The profile's name for the main menu's per-frame update.
pub const MENU_STEP_TARGET: &str = "UI::CMenuUI::DoStep";
/// The profile's names for the Lua 5.2 functions only the menu needs.
pub const LOAD_TARGET: &str = "lua_load";
pub const PCALL_TARGET: &str = "lua_pcallk";
pub const REF_TARGET: &str = "luaL_ref";

/// Lua 5.2's `LUA_REGISTRYINDEX`.
pub const LUA52_REGISTRY: c_int = -1_001_000;

const TSTRING: c_int = 4;
const TFUNCTION: c_int = 6;

/// A `lua_Reader`.
pub type Reader = unsafe extern "C-unwind" fn(State, *mut c_void, *mut usize) -> *const c_char;

/// What the menu needs of Lua beyond the link's [`LuaApi`], with Lua 5.2's
/// signatures.
#[derive(Clone, Copy)]
pub struct MenuApi {
    /// `lua_load(L, reader, data, chunkname, mode)`.
    pub load: unsafe extern "C-unwind" fn(
        State,
        Reader,
        *mut c_void,
        *const c_char,
        *const c_char,
    ) -> c_int,
    /// `lua_pcallk(L, nargs, nresults, errfunc, ctx, k)`.
    pub pcallk:
        unsafe extern "C-unwind" fn(State, c_int, c_int, c_int, c_int, *const c_void) -> c_int,
    /// `luaL_ref(L, t)`.
    pub reference: unsafe extern "C-unwind" fn(State, c_int) -> c_int,
    /// `LUA_REGISTRYINDEX`.
    pub registry: c_int,
}

static MENU_API: OnceLock<MenuApi> = OnceLock::new();

/// Makes `api` the one the menu uses; the first one stays. Returns whether
/// this one was taken.
pub fn install_api(api: MenuApi) -> bool {
    MENU_API.set(api).is_ok()
}

/// Run once in each Lua state the game gives `app`, with two functions of
/// the hook's as its arguments: `here(load)`, which keeps `load` and
/// returns the state's number, and `gone(number)`, which the sentinel's
/// finalizer calls when the state closes.
///
/// `load(name)` loads the save `name` of the game's save folder, as the
/// menu's Load Game page does, except that it does not ask the game to wait
/// for Start Game. It answers `"started"`, `"busy"` while the game is
/// loading something already (the menu's own sign of it: the progress
/// monitor's task, `gui/menu/main_menu.tl`), or why it could not.
pub const CHUNK: &str = r#"
local here, gone = ...
local number
local sentinel = setmetatable({}, { __gc = function() if number then gone(number) end end })
local function load(name)
	local keep = sentinel
	local found, theApp = pcall(function() return app end)
	if not found or theApp == nil then return "this Lua state has no app" end
	local busy = false
	pcall(function()
		local task = theApp.getProgressMonitor():getTask()
		busy = task ~= nil and task ~= ""
	end)
	if busy then return "busy" end
	local ok, err = pcall(function()
		local id = api.type.SavegameId.new()
		id.path = ""
		id.saveGameName = name
		id.saveGameNamespace = theApp.SaveGameNamespace.getSavegame()
		theApp.loadGame(id, false, nil)
	end)
	if ok then return "started" end
	return "app.loadGame failed: " .. tostring(err)
end
number = here(load)
"#;

/// A state that ran [`CHUNK`] and has not closed.
struct Adopted {
    number: u64,
    state: usize,
    /// Its `load`, in the state's registry.
    reference: c_int,
    /// The thread it was adopted on: the menu only calls into a state on
    /// that thread.
    thread: ThreadId,
}

static ADOPTED: Mutex<Vec<Adopted>> = Mutex::new(Vec::new());
static NEXT: AtomicU64 = AtomicU64::new(1);

fn adopted() -> MutexGuard<'static, Vec<Adopted>> {
    ADOPTED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether the menu can load a save from this thread: a state adopted on it
/// is open.
pub fn available() -> bool {
    let here = std::thread::current().id();
    adopted().iter().any(|state| state.thread == here)
}

/// One buffer handed to `lua_load`, whole, once.
struct Chunk {
    text: &'static str,
    done: bool,
}

unsafe extern "C-unwind" fn read_chunk(
    _l: State,
    data: *mut c_void,
    size: *mut usize,
) -> *const c_char {
    // SAFETY: `data` is the `Chunk` `adopt` passed to lua_load, and `size`
    // its out-parameter.
    unsafe {
        let chunk = &mut *data.cast::<Chunk>();
        if chunk.done {
            *size = 0;
            return std::ptr::null();
        }
        chunk.done = true;
        *size = chunk.text.len();
        chunk.text.as_ptr().cast()
    }
}

/// The string at the top of `l`'s stack, if it is one.
///
/// # Safety
///
/// `l` is live, on this thread.
unsafe fn string_at_top(api: &LuaApi, l: State) -> Option<String> {
    // SAFETY: the caller's; only a string is read, so nothing is converted.
    unsafe {
        if (api.type_of)(l, -1) != TSTRING {
            return None;
        }
        let mut len = 0;
        let text = (api.tolstring)(l, -1, &raw mut len);
        if text.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(text.cast::<u8>(), len.min(1000));
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// Runs [`CHUNK`] in `l`, the first time `l` is seen. Returns whether it
/// was adopted now (`false`: already), or why it could not be. Leaves the
/// stack as it was.
///
/// # Safety
///
/// `l` is a live Lua state, used on this thread, that no Lua code of this
/// thread is running in the middle of an API call on.
pub unsafe fn adopt(l: State) -> Result<bool, String> {
    let (Some(api), Some(menu)) = (lua::api(), MENU_API.get()) else {
        return Err("the hook has no Lua API for the menu".into());
    };
    if adopted().iter().any(|state| state.state == l as usize) {
        return Ok(false);
    }
    // SAFETY: the caller's; everything pushed is popped by the final settop.
    unsafe {
        let top = (api.gettop)(l);
        if (api.checkstack)(l, 4) == 0 {
            return Err("no room on the Lua stack".into());
        }
        let mut chunk = Chunk {
            text: CHUNK,
            done: false,
        };
        let status = (menu.load)(
            l,
            read_chunk,
            (&raw mut chunk).cast(),
            c"=tpf3mp-menu".as_ptr(),
            c"t".as_ptr(),
        );
        if status != 0 {
            let why = string_at_top(api, l).unwrap_or_default();
            (api.settop)(l, top);
            return Err(format!("the menu's Lua did not load ({status}): {why}"));
        }
        (api.pushcclosure)(l, native_here as CFunction, 0);
        (api.pushcclosure)(l, native_gone as CFunction, 0);
        let status = (menu.pcallk)(l, 2, 0, 0, 0, std::ptr::null());
        if status != 0 {
            let why = string_at_top(api, l).unwrap_or_default();
            (api.settop)(l, top);
            return Err(format!("the menu's Lua failed ({status}): {why}"));
        }
        (api.settop)(l, top);
    }
    if adopted().iter().any(|state| state.state == l as usize) {
        Ok(true)
    } else {
        Err("the menu's Lua ran but did not hand over its load".into())
    }
}

/// `here(load)`: keeps `load` in the registry; returns the state's number,
/// or nil.
unsafe extern "C-unwind" fn native_here(l: State) -> c_int {
    let (Some(api), Some(menu)) = (lua::api(), MENU_API.get()) else {
        return 0;
    };
    // SAFETY: Lua calls this with its own state, on its thread; a C
    // function has LUA_MINSTACK free slots.
    unsafe {
        if (api.gettop)(l) < 1 || (api.type_of)(l, 1) != TFUNCTION {
            (api.pushnil)(l);
            return 1;
        }
        (api.pushvalue)(l, 1);
        let reference = (menu.reference)(l, menu.registry);
        if reference < 0 {
            (api.pushnil)(l);
            return 1;
        }
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        adopted().push(Adopted {
            number,
            state: l as usize,
            reference,
            thread: std::thread::current().id(),
        });
        #[allow(clippy::cast_precision_loss)]
        (api.pushnumber)(l, number as f64);
        1
    }
}

/// `gone(number)`: the state is closing; the menu never calls into it
/// again.
unsafe extern "C-unwind" fn native_gone(l: State) -> c_int {
    let Some(api) = lua::api() else {
        return 0;
    };
    // SAFETY: Lua calls this with its own state, on its thread.
    let number = unsafe { (api.tonumberx)(l, 1, std::ptr::null_mut()) };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let number = number as u64;
    adopted().retain(|state| state.number != number);
    0
}

/// What the menu made of a load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Served {
    /// The game is loading the save.
    Started,
    /// The game is loading something else; ask again later.
    Busy,
    /// It could not, and why.
    Failed(String),
}

/// Loads the save `name` of the game's save folder from the newest state
/// adopted on this thread: `None` when there is none.
///
/// # Safety
///
/// Called on the thread that runs the menu's Lua, between its frames (no
/// Lua of this thread is running).
pub unsafe fn serve(name: &str) -> Option<Served> {
    let (Some(api), Some(menu)) = (lua::api(), MENU_API.get()) else {
        return None;
    };
    let here = std::thread::current().id();
    // The lock is let go before Lua runs: a collection there may finalize
    // another state's sentinel, which takes it.
    let (state, reference) = adopted()
        .iter()
        .rev()
        .find(|state| state.thread == here)
        .map(|state| (state.state, state.reference))?;
    let l = state as State;
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the state is open (its sentinel has not been finalized)
        // and belongs to this thread, which runs no Lua now; everything
        // pushed is popped by the final settop.
        unsafe {
            let top = (api.gettop)(l);
            if (api.checkstack)(l, 3) == 0 {
                return Served::Failed("no room on the Lua stack".into());
            }
            (api.rawgeti)(l, menu.registry, reference);
            (api.pushlstring)(l, name.as_ptr().cast(), name.len());
            let status = (menu.pcallk)(l, 1, 1, 0, 0, std::ptr::null());
            let answer = string_at_top(api, l).unwrap_or_default();
            (api.settop)(l, top);
            if status != 0 {
                return Served::Failed(format!("the menu's load raised: {answer}"));
            }
            match answer.as_str() {
                "started" => Served::Started,
                "busy" => Served::Busy,
                _ => Served::Failed(answer),
            }
        }
    }));
    Some(result.unwrap_or_else(|_| Served::Failed("the hook failed calling the menu".into())))
}

/// The game's `RegisterAppUsertypes`, reached through its trampoline.
static REGISTER_ORIGINAL: AtomicU64 = AtomicU64::new(0);
/// The menu's `DoStep`, reached through its trampoline.
static STEP_ORIGINAL: AtomicU64 = AtomicU64::new(0);

/// Both detours pass four registers through: the targets take three
/// (`RegisterAppUsertypes`: the state, the menu, the callback; `DoStep`: the
/// menu and two more) and their results pass back in `rax`.
type Passthrough = unsafe extern "C-unwind" fn(usize, usize, usize, usize) -> usize;

/// After the game gave a state `app`: adopt it. `state` is the game's
/// `lua::State&`, whose `lua_State*` is its first field.
unsafe extern "C-unwind" fn register_detour(
    state: usize,
    menu: usize,
    ready: usize,
    d: usize,
) -> usize {
    let original = REGISTER_ORIGINAL.load(Ordering::Acquire);
    if original == 0 {
        return 0;
    }
    // SAFETY: the trampoline of the function the profile resolved, called
    // with the arguments the game passed.
    let result =
        unsafe { std::mem::transmute::<u64, Passthrough>(original)(state, menu, ready, d) };
    let _ = std::panic::catch_unwind(|| {
        if state == 0 {
            return;
        }
        // SAFETY: `state` is the game's live `lua::State`, whose first word
        // is its `lua_State*` (`lua::State::State` stores it there).
        let l = unsafe { *(state as *const usize) };
        if l == 0 {
            return;
        }
        // SAFETY: the game just registered into this state on this thread
        // and is not inside any of its API calls now.
        match unsafe { adopt(l as State) } {
            Ok(true) => crate::install::log_line(&format!(
                "menu: Lua state {l:#x} has app; the main menu can load the room's world from it"
            )),
            Ok(false) => {}
            Err(why) => crate::install::log_line(&format!(
                "menu: Lua state {l:#x} has app, but the menu cannot load from it: {why}"
            )),
        }
    });
    result
}

/// After each of the menu's frames: the room, from the menu.
unsafe extern "C-unwind" fn step_detour(menu: usize, a: usize, b: usize, c: usize) -> usize {
    let original = STEP_ORIGINAL.load(Ordering::Acquire);
    if original == 0 {
        return 0;
    }
    // SAFETY: as above.
    let result = unsafe { std::mem::transmute::<u64, Passthrough>(original)(menu, a, b, c) };
    let _ = std::panic::catch_unwind(crate::install::menu_frame);
    result
}

/// Installs the menu's load: its Lua API, then the two detours. `at` gives
/// the address of a profile target in this process, `detour` installs a
/// detour for good and returns its trampoline (`crate::install`). Returns
/// the line for the log; any piece missing installs nothing more and says
/// why (the room's world then needs a world up, as before).
///
/// # Safety
///
/// Every address `at` gives is the function the profile names in this very
/// build, which no thread runs yet; `detour` is as
/// `InlineDetour::install`.
#[cfg(all(windows, target_arch = "x86_64"))]
pub unsafe fn install(
    at: &dyn Fn(&str) -> Result<usize, String>,
    detour: unsafe fn(*mut u8, *const u8) -> Result<usize, String>,
) -> Result<String, String> {
    let why = |error: String| {
        format!(
            "the main menu cannot load the room's world (fail closed): {error}; a game needs a world up to take the room's"
        )
    };
    let (load, pcallk, reference, register, step) = (|| {
        Ok::<_, String>((
            at(LOAD_TARGET)?,
            at(PCALL_TARGET)?,
            at(REF_TARGET)?,
            at(REGISTER_APP_TARGET)?,
            at(MENU_STEP_TARGET)?,
        ))
    })()
    .map_err(why)?;
    if lua::api().is_none() {
        return Err(why("the Lua link is not installed".into()));
    }
    // SAFETY: each address is the Lua 5.2 function the profile names, found
    // by its signature and prologue in this very build (a call target only,
    // never detoured). Each transmute's type is its field's: the API's
    // signature, spelled once, in `MenuApi`.
    #[allow(clippy::missing_transmute_annotations)]
    install_api(unsafe {
        MenuApi {
            load: std::mem::transmute::<usize, _>(load),
            pcallk: std::mem::transmute::<usize, _>(pcallk),
            reference: std::mem::transmute::<usize, _>(reference),
            registry: LUA52_REGISTRY,
        }
    });
    // SAFETY: both targets are functions the profile resolved and
    // prologue-checked; the hook installs while the game starts, before its
    // menu or any Lua state exists, so no thread runs them; each detour has
    // their ABI (register arguments passed through, the result in rax).
    let register = unsafe { detour(register as *mut u8, register_detour as *const u8) }
        .map_err(|error| why(format!("detouring {REGISTER_APP_TARGET}: {error}")))?;
    REGISTER_ORIGINAL.store(register as u64, Ordering::Release);
    // SAFETY: as above.
    let step = unsafe { detour(step as *mut u8, step_detour as *const u8) }
        .map_err(|error| why(format!("detouring {MENU_STEP_TARGET}: {error}")))?;
    STEP_ORIGINAL.store(step as u64, Ordering::Release);
    Ok(format!(
        "the main menu can load the room's world: detours on {REGISTER_APP_TARGET} and {MENU_STEP_TARGET}"
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::PoisonError;

    use mlua::ffi;

    use super::*;
    use crate::lua::tests::{Lua, SERIAL, lua51};

    unsafe extern "C-unwind" fn load51(
        l: State,
        reader: Reader,
        data: *mut c_void,
        name: *const c_char,
        _mode: *const c_char,
    ) -> c_int {
        // The same reader under Lua 5.1's type.
        let reader = unsafe { std::mem::transmute::<Reader, ffi::lua_Reader>(reader) };
        unsafe { ffi::lua_load(l.cast(), reader, data, name) }
    }
    unsafe extern "C-unwind" fn pcall51(
        l: State,
        nargs: c_int,
        nresults: c_int,
        errfunc: c_int,
        _ctx: c_int,
        _k: *const c_void,
    ) -> c_int {
        unsafe { ffi::lua_pcall(l.cast(), nargs, nresults, errfunc) }
    }
    unsafe extern "C-unwind" fn ref51(l: State, t: c_int) -> c_int {
        unsafe { ffi::luaL_ref(l.cast(), t) }
    }

    /// The link's and the menu's Lua 5.1 API, installed once for the test
    /// binary.
    pub(crate) fn menu51() {
        lua51();
        install_api(MenuApi {
            load: load51,
            pcallk: pcall51,
            reference: ref51,
            registry: ffi::LUA_REGISTRYINDEX,
        });
    }

    /// The menu's `app` and `api`, as far as a load uses them: `LOADS`
    /// records each load, `TASK` is the progress monitor's task.
    const FAKE_MENU: &str = "\
        LOADS = {} TASK = '' \
        api = { type = { SavegameId = { new = function() return {} end } } } \
        app = { \
          SaveGameNamespace = { getSavegame = function() return 'savegame' end }, \
          getProgressMonitor = function() return { getTask = function() return TASK end } end, \
          loadGame = function(id, isMapEditor, info) \
            if FAIL then error(FAIL, 0) end \
            LOADS[#LOADS + 1] = id.saveGameName .. '|' .. id.path .. '|' .. id.saveGameNamespace \
              .. '|' .. tostring(isMapEditor) .. '|' .. tostring(info) \
          end }";

    pub(crate) fn forget_all() {
        adopted().clear();
    }

    #[test]
    fn a_state_with_app_loads_the_rooms_save_as_the_menus_load_page_does() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        menu51();
        forget_all();
        let menu = Lua::new();
        menu.run(FAKE_MENU).unwrap();
        assert!(!available(), "nothing adopted yet");
        assert_eq!(unsafe { serve("tpf3mp_room_7") }, None);
        let l: *mut ffi::lua_State = menu.state().cast();
        let top = unsafe { ffi::lua_gettop(l) };
        assert_eq!(unsafe { adopt(menu.state()) }, Ok(true));
        assert_eq!(unsafe { adopt(menu.state()) }, Ok(false), "once a state");
        assert_eq!(unsafe { ffi::lua_gettop(l) }, top, "the stack is as it was");
        assert!(available());
        assert_eq!(unsafe { serve("tpf3mp_room_7") }, Some(Served::Started));
        assert_eq!(
            menu.run("return #LOADS, LOADS[1]"),
            Ok("1|tpf3mp_room_7||savegame|false|nil".into()),
            "no Start Game wait, the save's own mods"
        );
        // Loading something already: asked again later, nothing started.
        menu.run("TASK = 'Loading'").unwrap();
        assert_eq!(unsafe { serve("tpf3mp_room_7") }, Some(Served::Busy));
        menu.run("TASK = '' FAIL = 'Game initialization is already active!'")
            .unwrap();
        assert_eq!(
            unsafe { serve("tpf3mp_room_7") },
            Some(Served::Failed(
                "app.loadGame failed: Game initialization is already active!".into()
            ))
        );
        assert_eq!(menu.run("return #LOADS"), Ok("1".into()));
        forget_all();
    }

    #[test]
    fn a_state_without_app_says_so_and_a_closed_state_is_never_called() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        menu51();
        forget_all();
        let bare = Lua::new();
        assert_eq!(unsafe { adopt(bare.state()) }, Ok(true));
        assert_eq!(
            unsafe { serve("x") },
            Some(Served::Failed("this Lua state has no app".into()))
        );
        // The state closes: its sentinel's finalizer (Lua 5.2 runs them all
        // at close) says so, and the menu has nothing to call.
        let number = adopted()[0].number;
        // Lua 5.1 has no finalizers on tables: call what the sentinel would.
        let l: *mut ffi::lua_State = bare.state().cast();
        unsafe {
            ffi::lua_pushcclosure(
                l,
                std::mem::transmute::<CFunction, ffi::lua_CFunction>(native_gone),
                0,
            );
            #[allow(clippy::cast_precision_loss)]
            ffi::lua_pushnumber(l, number as f64);
            assert_eq!(ffi::lua_pcall(l, 1, 0, 0), 0);
        }
        assert!(!available());
        assert_eq!(unsafe { serve("x") }, None);
        forget_all();
    }

    #[test]
    fn a_state_adopted_on_another_thread_is_not_the_menus() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        menu51();
        forget_all();
        let menu = Lua::new();
        menu.run(FAKE_MENU).unwrap();
        assert_eq!(unsafe { adopt(menu.state()) }, Ok(true));
        let elsewhere = std::thread::spawn(|| (available(), unsafe { serve("x") }))
            .join()
            .unwrap();
        assert_eq!(elsewhere, (false, None));
        assert_eq!(menu.run("return #LOADS"), Ok("0".into()));
        forget_all();
    }

    #[test]
    fn the_chunk_keeps_its_sentinel_and_waits_for_no_start_button() {
        assert!(CHUNK.contains("__gc"));
        assert!(
            CHUNK.contains("local keep = sentinel"),
            "load keeps it alive"
        );
        assert!(!CHUNK.contains("setWaitForStartReadyGame"));
        assert!(CHUNK.contains("theApp.loadGame(id, false, nil)"));
    }
}

//! The hook's half of the link to the Lua mod (docs/HOOKS.md, "The Lua
//! side"): the `tpf3mp_native` table, and the queues between it and the
//! step gate.
//!
//! The hook detours Lua's `print` and adds `tpf3mp_native` to the globals
//! of every Lua state that calls it, once; the mod calls `print` before it
//! looks for the table. The game runs Lua in several states: the GUI's on
//! the main thread and the game scripts' on a pool of simulation threads.
//! So the table's functions share nothing but [`SHARED`], behind a lock:
//!
//! - `command(action)`: the player acted. The table is read into a
//!   [`LuaValue`] tree within [`MAX_DEPTH`] and [`MAX_NODES`], converted with
//!   the schema ([`action_from_lua`]) and queued for the step gate, which
//!   hands it to the room ([`take_commands`]). Returns `true`, or `false` and
//!   why: an action that was not queued must not happen at all.
//! - `take()`: the actions the room ordered for this simulation update, as
//!   tables ([`action_to_lua`]), or `nil`. The step gate begins a batch of
//!   updates at the step the room ordered them for ([`begin_batch`]), and
//!   the first update that asks gets them: the mod's game script asks in its
//!   `update`, which the game runs once per simulation update, where a
//!   command runs at once, the same update on every game.
//! - `log(line)`: a line for `hook.log`.
//! - `poll()`: in the GUI, every frame: what the hook asks of the game, a
//!   table `{ save = name }` or `{ load = name }` (a save of the game's own
//!   save folder), once, or `nil`. The GUI saves with `app.saveGame` and
//!   loads with `app.loadGame` ([`request_save`], [`request_load`]).
//! - `saved(name, ok, why)`: the GUI's answer to a save ([`take_save_answer`]).
//! - `world()`: a world's GUI started. Once the GUI has taken a load, the
//!   next world to start is the one it loaded ([`load_done`]).
//! - `room()`: whether the room's game runs ([`set_in_room`]): the GUI then
//!   refuses the player's commands the room cannot carry yet (docs/HOOKS.md,
//!   "The player's commands").
//! - `checkpoint()`: in a game script's `postUpdate`: whether this update is
//!   the last of a batch that ends at a checkpoint step, so the script reads
//!   the world's lanes now. The updates of a batch are counted by their
//!   `take()`.
//! - `lanes(t)`: the lanes read there, a table from lane numbers to strings,
//!   which the step gate reports as digests ([`end_batch`]). Returns `true`,
//!   or `false` and why.
//! - `clicks()`: in the GUI: the player's builds queued in the room's game
//!   so far, or `nil` where the hook cannot take them to the room
//!   ([`crate::builds`]).
//! - `replaying(on)`: the game script begins or ends applying the room's
//!   actions, whose builds the hook lets through ([`crate::builds`]).
//! - `version`: [`VERSION`].
//!
//! Everything reaches Lua through [`LuaApi`]: in the game, the C API
//! functions the build profile names (Lua 5.2); in the tests, Lua 5.1's
//! through small adapters. No function here calls into Lua code, so a Lua
//! error can only come from the API itself running out of memory.

#![allow(unsafe_code)]

use std::{
    collections::VecDeque,
    ffi::{CStr, c_char, c_int, c_void},
    panic::AssertUnwindSafe,
    sync::{
        Mutex, MutexGuard, OnceLock, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

use tpf3mp_proto::{
    Payload,
    action::Action,
    lua::{LuaValue, MAX_DEPTH, MAX_NODES, action_from_lua, action_to_lua},
};

/// A `lua_State`, never dereferenced here.
pub type State = *mut c_void;
/// A Lua C function.
pub type CFunction = unsafe extern "C-unwind" fn(State) -> c_int;

const TNIL: c_int = 0;
const TBOOLEAN: c_int = 1;
const TNUMBER: c_int = 3;
const TSTRING: c_int = 4;
const TTABLE: c_int = 5;

/// The contract's version: `bridge.lua`'s `VERSION`.
pub const VERSION: f64 = 7.0;
/// The table's name in each state's globals.
pub const GLOBAL: &CStr = c"tpf3mp_native";

/// Most actions waiting for the step gate to hand them to the room.
const MAX_WAITING: usize = 256;
/// Most lanes one checkpoint reports, and the longest text one lane may be.
const MAX_LANES: usize = 64;
const MAX_LANE_TEXT: usize = 4096;

/// Most lines waiting for the hook's log, and the longest kept.
const MAX_LOG_LINES: usize = 1024;
const MAX_LOG_LINE: usize = 1000;

/// Where a Lua state keeps its globals.
#[derive(Debug, Clone, Copy)]
pub enum Globals {
    /// Lua 5.2: at `key` in the registry, at pseudo-index `index`.
    Registry { index: c_int, key: c_int },
    /// Lua 5.1: at a pseudo-index of their own.
    Pseudo(c_int),
}

/// Lua 5.2's: `LUA_REGISTRYINDEX` (`-LUAI_MAXSTACK - 1000`) and
/// `LUA_RIDX_GLOBALS`.
pub const LUA52_GLOBALS: Globals = Globals::Registry {
    index: -1_001_000,
    key: 2,
};

/// The functions of Lua's C API the link uses, with Lua 5.2's signatures.
#[derive(Clone, Copy)]
pub struct LuaApi {
    pub gettop: unsafe extern "C-unwind" fn(State) -> c_int,
    pub settop: unsafe extern "C-unwind" fn(State, c_int),
    pub checkstack: unsafe extern "C-unwind" fn(State, c_int) -> c_int,
    pub pushvalue: unsafe extern "C-unwind" fn(State, c_int),
    pub type_of: unsafe extern "C-unwind" fn(State, c_int) -> c_int,
    pub toboolean: unsafe extern "C-unwind" fn(State, c_int) -> c_int,
    pub tonumberx: unsafe extern "C-unwind" fn(State, c_int, *mut c_int) -> f64,
    pub tolstring: unsafe extern "C-unwind" fn(State, c_int, *mut usize) -> *const c_char,
    pub next: unsafe extern "C-unwind" fn(State, c_int) -> c_int,
    pub pushnil: unsafe extern "C-unwind" fn(State),
    pub pushnumber: unsafe extern "C-unwind" fn(State, f64),
    pub pushboolean: unsafe extern "C-unwind" fn(State, c_int),
    pub pushlstring: unsafe extern "C-unwind" fn(State, *const c_char, usize) -> *const c_char,
    pub pushcclosure: unsafe extern "C-unwind" fn(State, CFunction, c_int),
    pub createtable: unsafe extern "C-unwind" fn(State, c_int, c_int),
    pub rawget: unsafe extern "C-unwind" fn(State, c_int),
    pub rawset: unsafe extern "C-unwind" fn(State, c_int),
    pub rawgeti: unsafe extern "C-unwind" fn(State, c_int, c_int),
    pub globals: Globals,
}

static API: OnceLock<LuaApi> = OnceLock::new();

/// Makes `api` the one the table's functions use; the first one stays.
/// Returns whether this one was taken.
pub fn install_api(api: LuaApi) -> bool {
    API.set(api).is_ok()
}

/// The API [`install_api`] set, if any.
pub fn api() -> Option<&'static LuaApi> {
    API.get()
}

/// What the hook asks of the game's GUI.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Request {
    Save(String),
    Load(String),
}

/// What the table's functions share with the step gate.
/// The batch of updates the game's step is running.
struct Batch {
    /// Its actions, until a game script takes them.
    actions: Option<Vec<LuaValue>>,
    /// The updates it runs, and those a game script has begun (`take`).
    updates: u32,
    begun: u32,
    /// It ends at a checkpoint step, and the lanes read after its last
    /// update.
    lanes_wanted: bool,
    lanes: Option<Vec<(u16, String)>>,
}

struct Shared {
    /// Actions handed over, for the room, oldest first.
    commands: VecDeque<Payload>,
    /// The batch running.
    batch: Batch,
    /// Lines for the hook's log.
    log: VecDeque<String>,
    /// A request the GUI has not polled yet.
    request: Option<Request>,
    /// The GUI's answer to the last save: the name saved, or why not.
    save_answer: Option<Result<String, String>>,
    /// Worlds whose GUI started since the hook began.
    worlds: u64,
    /// A load asked for: `None` until the GUI took it, then the worlds
    /// started by then.
    load: Option<Option<u64>>,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    commands: VecDeque::new(),
    batch: Batch {
        actions: None,
        updates: 0,
        begun: 0,
        lanes_wanted: false,
        lanes: None,
    },
    log: VecDeque::new(),
    request: None,
    save_answer: None,
    worlds: 0,
    load: None,
});

fn shared() -> MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether the room's game runs, as `room()` tells the GUI.
static IN_ROOM: AtomicBool = AtomicBool::new(false);

/// The step gate says whether the room's game runs (held included: the
/// world then stands still, and a command would still change it).
pub fn set_in_room(in_room: bool) {
    IN_ROOM.store(in_room, Ordering::Release);
}

/// Whether the room's game runs.
pub fn in_room() -> bool {
    IN_ROOM.load(Ordering::Acquire)
}

/// The actions handed over since the last call, oldest first.
pub fn take_commands() -> Vec<Payload> {
    shared().commands.drain(..).collect()
}

/// A batch of `updates` updates begins; its first update applies
/// `actions`, the room's events for the step it starts at, and with
/// `lanes` its last update reads the world's lanes. Refuses an action with
/// no table form, before any update runs.
pub fn begin_batch(actions: &[Action], updates: u32, lanes: bool) -> Result<(), String> {
    let tables = actions
        .iter()
        .map(|action| {
            action_to_lua(action)
                .map_err(|error| format!("an action the room ordered has no table form: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    shared().batch = Batch {
        actions: (!tables.is_empty()).then_some(tables),
        updates,
        begun: 0,
        lanes_wanted: lanes,
        lanes: None,
    };
    Ok(())
}

/// The batch ended: the lanes read after its last update, if it wanted and
/// got them. Refuses if its actions were not taken: the world then ran the
/// room's step without them.
pub fn end_batch() -> Result<Option<Vec<(u16, String)>>, String> {
    let mut shared = shared();
    let batch = &mut shared.batch;
    batch.lanes_wanted = false;
    batch.updates = 0;
    batch.begun = 0;
    match batch.actions.take() {
        None => Ok(batch.lanes.take()),
        Some(tables) => {
            batch.lanes = None;
            Err(format!(
                "the mod's game script did not take the {} action(s) the room ordered for this step",
                tables.len()
            ))
        }
    }
}

/// The lines logged since the last call.
pub fn take_log() -> Vec<String> {
    shared().log.drain(..).collect()
}

/// Asks the GUI to save the world under `name`, in the game's own save
/// folder; the answer comes through [`take_save_answer`].
pub fn request_save(name: &str) {
    let mut shared = shared();
    shared.request = Some(Request::Save(name.to_owned()));
    shared.save_answer = None;
}

/// The GUI's answer to the last save request, once: the name it saved, or
/// why it did not.
pub fn take_save_answer() -> Option<Result<String, String>> {
    shared().save_answer.take()
}

/// Asks the GUI to load the save `name` of the game's own save folder.
pub fn request_load(name: &str) {
    let mut shared = shared();
    shared.request = Some(Request::Load(name.to_owned()));
    shared.load = Some(None);
}

/// Whether the load asked for is done: a world's GUI started after the GUI
/// took the request (a world that started before, the one the GUI loaded
/// from, does not count). Once.
pub fn load_done() -> bool {
    let mut shared = shared();
    let done = matches!(shared.load, Some(Some(taken)) if shared.worlds > taken);
    if done {
        shared.load = None;
    }
    done
}

fn log(line: String) {
    let mut shared = shared();
    if shared.log.len() < MAX_LOG_LINES {
        shared.log.push_back(line);
    }
}

/// Adds `tpf3mp_native` to `l`'s globals, unless they have one. Sets it
/// raw, past any metatable a strict state gives its globals.
///
/// # Safety
///
/// `l` is a live Lua state, used on this thread, inside the call of a C
/// function (so it has a frame to push on).
pub unsafe fn register(api: &LuaApi, l: State) {
    // SAFETY: the caller's; every push is covered by the checkstack.
    unsafe {
        let top = (api.gettop)(l);
        if (api.checkstack)(l, 8) == 0 {
            return;
        }
        push_globals(api, l);
        let globals = (api.gettop)(l);
        push_str(api, l, GLOBAL.to_bytes());
        (api.rawget)(l, globals);
        let present = (api.type_of)(l, -1) == TTABLE;
        (api.settop)(l, globals);
        if !present {
            push_str(api, l, GLOBAL.to_bytes());
            (api.createtable)(l, 0, 4);
            let table = (api.gettop)(l);
            push_str(api, l, b"version");
            (api.pushnumber)(l, VERSION);
            (api.rawset)(l, table);
            for (name, function) in [
                (&b"command"[..], native_command as CFunction),
                (b"take", native_take),
                (b"log", native_log),
                (b"poll", native_poll),
                (b"saved", native_saved),
                (b"world", native_world),
                (b"room", native_room),
                (b"checkpoint", native_checkpoint),
                (b"lanes", native_lanes),
                (b"clicks", native_clicks),
                (b"replaying", native_replaying),
            ] {
                push_str(api, l, name);
                (api.pushcclosure)(l, function, 0);
                (api.rawset)(l, table);
            }
            (api.rawset)(l, globals);
        }
        (api.settop)(l, top);
    }
}

/// # Safety
///
/// As [`register`], with a free slot.
unsafe fn push_globals(api: &LuaApi, l: State) {
    // SAFETY: the caller's.
    unsafe {
        match api.globals {
            Globals::Registry { index, key } => (api.rawgeti)(l, index, key),
            Globals::Pseudo(index) => (api.pushvalue)(l, index),
        }
    }
}

/// # Safety
///
/// As [`register`], with a free slot.
unsafe fn push_str(api: &LuaApi, l: State, text: &[u8]) {
    // SAFETY: the caller's; Lua copies the bytes.
    unsafe {
        (api.pushlstring)(l, text.as_ptr().cast(), text.len());
    }
}

fn type_name(kind: c_int) -> &'static str {
    match kind {
        TNIL => "nil",
        TBOOLEAN => "boolean",
        2 => "light userdata",
        TNUMBER => "number",
        TSTRING => "string",
        TTABLE => "table",
        6 => "function",
        7 => "userdata",
        8 => "thread",
        _ => "value of no type",
    }
}

/// Reads the value at `index` (absolute) into a tree.
///
/// # Safety
///
/// As [`register`]; `index` is a valid absolute index.
unsafe fn read(
    api: &LuaApi,
    l: State,
    index: c_int,
    depth: usize,
    nodes: &mut usize,
) -> Result<LuaValue, String> {
    *nodes += 1;
    if *nodes > MAX_NODES {
        return Err(format!("the action has more than {MAX_NODES} values"));
    }
    // SAFETY: the caller's. Strings are read only when they are strings,
    // so lua_tolstring never turns a key under lua_next into another.
    unsafe {
        match (api.type_of)(l, index) {
            TNIL => Ok(LuaValue::Nil),
            TBOOLEAN => Ok(LuaValue::Boolean((api.toboolean)(l, index) != 0)),
            TNUMBER => Ok(LuaValue::Number((api.tonumberx)(
                l,
                index,
                std::ptr::null_mut(),
            ))),
            TSTRING => {
                let mut len = 0;
                let text = (api.tolstring)(l, index, &raw mut len);
                if text.is_null() {
                    return Err("a string that cannot be read".into());
                }
                Ok(LuaValue::String(
                    std::slice::from_raw_parts(text.cast::<u8>(), len).to_vec(),
                ))
            }
            TTABLE => {
                if depth >= MAX_DEPTH {
                    return Err(format!("tables nested deeper than {MAX_DEPTH}"));
                }
                if (api.checkstack)(l, 4) == 0 {
                    return Err("no room on the Lua stack".into());
                }
                let mut entries = Vec::new();
                (api.pushnil)(l);
                while (api.next)(l, index) != 0 {
                    let value_at = (api.gettop)(l);
                    let key = read(api, l, value_at - 1, depth + 1, nodes)?;
                    if matches!(key, LuaValue::Table(_)) {
                        return Err("a table used as a key".into());
                    }
                    let value = read(api, l, value_at, depth + 1, nodes)?;
                    entries.push((key, value));
                    (api.settop)(l, value_at - 1);
                }
                Ok(LuaValue::Table(entries))
            }
            other => Err(format!("a {} has no place in an action", type_name(other))),
        }
    }
}

/// Pushes `value`.
///
/// # Safety
///
/// As [`register`].
unsafe fn push(api: &LuaApi, l: State, value: &LuaValue, depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("tables nested deeper than {MAX_DEPTH}"));
    }
    // SAFETY: the caller's; every push is covered by the checkstack.
    unsafe {
        if (api.checkstack)(l, 3) == 0 {
            return Err("no room on the Lua stack".into());
        }
        match value {
            LuaValue::Nil => (api.pushnil)(l),
            LuaValue::Boolean(value) => (api.pushboolean)(l, c_int::from(*value)),
            LuaValue::Number(value) => (api.pushnumber)(l, *value),
            // Lua 5.2 has no integers; every id fits a double exactly.
            #[allow(clippy::cast_precision_loss)]
            LuaValue::Integer(value) => (api.pushnumber)(l, *value as f64),
            LuaValue::String(text) => push_str(api, l, text),
            LuaValue::Table(entries) => {
                let records = c_int::try_from(entries.len()).unwrap_or(c_int::MAX);
                (api.createtable)(l, 0, records);
                let table = (api.gettop)(l);
                for (key, value) in entries {
                    push(api, l, key, depth + 1)?;
                    push(api, l, value, depth + 1)?;
                    (api.rawset)(l, table);
                }
            }
        }
    }
    Ok(())
}

/// `command(action)`.
unsafe extern "C-unwind" fn native_command(l: State) -> c_int {
    let Some(api) = API.get() else {
        return 0;
    };
    // SAFETY: Lua calls this with its own state, on its thread.
    let top = unsafe { (api.gettop)(l) };
    let read = std::panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as above.
        unsafe { command_from(api, l) }
    }))
    .unwrap_or_else(|_| Err("the hook failed reading the action".into()));
    // SAFETY: as above.
    unsafe { (api.settop)(l, top) };
    let queued = read.and_then(|payload| {
        let mut shared = shared();
        if shared.commands.len() >= MAX_WAITING {
            return Err(format!(
                "{MAX_WAITING} actions are already waiting for the room"
            ));
        }
        shared.commands.push_back(payload);
        Ok(())
    });
    // SAFETY: as above; a C function's call has room for its results.
    unsafe {
        match queued {
            Ok(()) => {
                (api.pushboolean)(l, 1);
                1
            }
            Err(reason) => {
                (api.pushboolean)(l, 0);
                push_str(api, l, reason.as_bytes());
                2
            }
        }
    }
}

/// # Safety
///
/// Lua's own state, on its thread.
unsafe fn command_from(api: &LuaApi, l: State) -> Result<Payload, String> {
    // SAFETY: the caller's.
    unsafe {
        if (api.gettop)(l) < 1 || (api.type_of)(l, 1) != TTABLE {
            return Err("an action is a table".into());
        }
        let mut nodes = 0;
        let tree = read(api, l, 1, 0, &mut nodes)?;
        let action = action_from_lua(&tree).map_err(|error| error.to_string())?;
        action.to_payload().map_err(|error| error.to_string())
    }
}

/// `take()`.
unsafe extern "C-unwind" fn native_take(l: State) -> c_int {
    let Some(api) = API.get() else {
        return 0;
    };
    let batch = {
        let mut shared = shared();
        shared.batch.begun = shared.batch.begun.saturating_add(1);
        shared.batch.actions.take()
    };
    let Some(tables) = batch else {
        // SAFETY: Lua calls this with its own state, on its thread.
        unsafe { (api.pushnil)(l) };
        return 1;
    };
    let list = LuaValue::Table(
        tables
            .iter()
            .enumerate()
            .map(|(index, table)| {
                #[allow(clippy::cast_precision_loss)]
                let position = (index + 1) as f64;
                (LuaValue::Number(position), table.clone())
            })
            .collect(),
    );
    // SAFETY: as above.
    let top = unsafe { (api.gettop)(l) };
    let pushed = std::panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as above.
        unsafe { push(api, l, &list, 0) }
    }));
    if matches!(pushed, Ok(Ok(()))) {
        return 1;
    }
    // Not handed over: the step gate finds them untaken and holds.
    shared().batch.actions = Some(tables);
    // SAFETY: as above.
    unsafe {
        (api.settop)(l, top);
        (api.pushnil)(l);
    }
    1
}

/// The string argument at `index`, if it is one, up to `max` bytes.
///
/// # Safety
///
/// Lua's own state, on its thread.
unsafe fn string_arg(api: &LuaApi, l: State, index: c_int, max: usize) -> Option<String> {
    // SAFETY: the caller's.
    unsafe {
        if (api.gettop)(l) < index || (api.type_of)(l, index) != TSTRING {
            return None;
        }
        let mut len = 0;
        let text = (api.tolstring)(l, index, &raw mut len);
        if text.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(text.cast::<u8>(), len.min(max));
        Some(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// `poll()`.
unsafe extern "C-unwind" fn native_poll(l: State) -> c_int {
    let Some(api) = API.get() else {
        return 0;
    };
    let request = {
        let mut shared = shared();
        let request = shared.request.take();
        if matches!(request, Some(Request::Load(_))) {
            shared.load = Some(Some(shared.worlds));
        }
        request
    };
    let table = match request {
        None => LuaValue::Nil,
        Some(Request::Save(name)) => {
            LuaValue::Table(vec![(LuaValue::string("save"), LuaValue::string(&name))])
        }
        Some(Request::Load(name)) => {
            LuaValue::Table(vec![(LuaValue::string("load"), LuaValue::string(&name))])
        }
    };
    // SAFETY: Lua calls this with its own state, on its thread.
    let top = unsafe { (api.gettop)(l) };
    let pushed = std::panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as above.
        unsafe { push(api, l, &table, 0) }
    }));
    if !matches!(pushed, Ok(Ok(()))) {
        // SAFETY: as above.
        unsafe {
            (api.settop)(l, top);
            (api.pushnil)(l);
        }
    }
    1
}

/// `saved(name, ok, why)`.
unsafe extern "C-unwind" fn native_saved(l: State) -> c_int {
    let Some(api) = API.get() else {
        return 0;
    };
    // SAFETY: Lua calls this with its own state, on its thread.
    let (name, ok, why) = unsafe {
        (
            string_arg(api, l, 1, MAX_LOG_LINE).unwrap_or_default(),
            (api.gettop)(l) >= 2 && (api.toboolean)(l, 2) != 0,
            string_arg(api, l, 3, MAX_LOG_LINE),
        )
    };
    shared().save_answer = Some(if ok {
        Ok(name)
    } else {
        Err(why.unwrap_or_else(|| "the game did not save".into()))
    });
    0
}

/// `world()`.
unsafe extern "C-unwind" fn native_world(_l: State) -> c_int {
    shared().worlds += 1;
    0
}

/// `checkpoint()`: whether the update running is the last of a batch that
/// ends at a checkpoint step, and its lanes are not read yet.
unsafe extern "C-unwind" fn native_checkpoint(l: State) -> c_int {
    let Some(api) = API.get() else {
        return 0;
    };
    let due = {
        let batch = &shared().batch;
        batch.lanes_wanted && batch.begun == batch.updates && batch.lanes.is_none()
    };
    // SAFETY: a C function's stack has LUA_MINSTACK free slots.
    unsafe { (api.pushboolean)(l, c_int::from(due)) };
    1
}

/// The lanes in a table from lane numbers to strings, or why not.
fn lanes_from(value: &LuaValue) -> Result<Vec<(u16, String)>, String> {
    let LuaValue::Table(entries) = value else {
        return Err("the lanes are a table".into());
    };
    if entries.len() > MAX_LANES {
        return Err(format!("more than {MAX_LANES} lanes"));
    }
    let mut lanes = Vec::with_capacity(entries.len());
    for (key, text) in entries {
        let lane = match key {
            LuaValue::Number(n) if n.fract() == 0.0 && (0.0..=f64::from(u16::MAX)).contains(n) => {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let lane = *n as u16;
                lane
            }
            _ => return Err("a lane's number is a whole number from 0 to 65535".into()),
        };
        let LuaValue::String(bytes) = text else {
            return Err(format!("lane {lane} is not a string"));
        };
        if bytes.len() > MAX_LANE_TEXT {
            return Err(format!("lane {lane} is longer than {MAX_LANE_TEXT} bytes"));
        }
        let text =
            String::from_utf8(bytes.clone()).map_err(|_| format!("lane {lane} is not UTF-8"))?;
        lanes.push((lane, text));
    }
    lanes.sort_by_key(|(lane, _)| *lane);
    if lanes.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err("a lane is given twice".into());
    }
    Ok(lanes)
}

/// `lanes(t)`: the lanes read at the checkpoint. Returns `true`, or `false`
/// and why: none is due, or the table is not lanes.
unsafe extern "C-unwind" fn native_lanes(l: State) -> c_int {
    let Some(api) = API.get() else {
        return 0;
    };
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let due = {
            let batch = &shared().batch;
            batch.lanes_wanted && batch.begun == batch.updates && batch.lanes.is_none()
        };
        if !due {
            return Err("no checkpoint is due in this update".to_owned());
        }
        let mut nodes = 0;
        // SAFETY: Lua calls this with its own state; index 1 is the
        // argument, if any.
        let value = unsafe {
            if (api.gettop)(l) < 1 {
                return Err("no lanes given".to_owned());
            }
            read(api, l, 1, 0, &mut nodes)?
        };
        let lanes = lanes_from(&value)?;
        shared().batch.lanes = Some(lanes);
        Ok(())
    }));
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(_) => Err("reading the lanes failed".to_owned()),
    };
    // SAFETY: a C function's stack has LUA_MINSTACK free slots.
    unsafe {
        match outcome {
            Ok(()) => {
                (api.pushboolean)(l, 1);
                1
            }
            Err(why) => {
                (api.pushboolean)(l, 0);
                push_str(api, l, why.as_bytes());
                2
            }
        }
    }
}

/// `clicks()`: the player's builds queued in the room's game so far, or nil
/// without the build detours.
unsafe extern "C-unwind" fn native_clicks(l: State) -> c_int {
    let Some(api) = API.get() else {
        return 0;
    };
    // SAFETY: a C function's stack has LUA_MINSTACK free slots.
    unsafe {
        if crate::builds::installed() {
            #[allow(clippy::cast_precision_loss)]
            (api.pushnumber)(l, crate::builds::clicks() as f64);
        } else {
            (api.pushnil)(l);
        }
    }
    1
}

/// `replaying(on)`: the game script begins (true) or ends applying the
/// room's actions.
unsafe extern "C-unwind" fn native_replaying(l: State) -> c_int {
    let Some(api) = API.get() else {
        return 0;
    };
    // SAFETY: Lua calls this with its own state; index 1 is the argument.
    let on = unsafe { (api.gettop)(l) >= 1 && (api.toboolean)(l, 1) != 0 };
    crate::builds::set_replaying(on);
    0
}

/// `room()`: `true` while the room's game runs.
unsafe extern "C-unwind" fn native_room(l: State) -> c_int {
    let Some(api) = API.get() else {
        return 0;
    };
    // SAFETY: a C function's stack has LUA_MINSTACK free slots.
    unsafe { (api.pushboolean)(l, c_int::from(IN_ROOM.load(Ordering::Acquire))) };
    1
}

/// `log(line)`.
unsafe extern "C-unwind" fn native_log(l: State) -> c_int {
    let Some(api) = API.get() else {
        return 0;
    };
    // SAFETY: Lua calls this with its own state, on its thread.
    if let Some(line) = unsafe { string_arg(api, l, 1, MAX_LOG_LINE) } {
        log(format!("mod: {line}"));
    }
    0
}

#[cfg(test)]
pub(crate) mod tests {
    use std::ffi::{CString, c_char, c_int};

    use mlua::ffi;
    use tpf3mp_proto::action::{Action, ConstructionBuild, Param, ParamValue, Pos, Transform};
    use tpf3mp_proto::{BoundedVec, Text};

    use super::*;

    /// The tests share the link's statics: one at a time.
    pub(crate) static SERIAL: Mutex<()> = Mutex::new(());

    // Lua 5.1's C API, as the link calls Lua 5.2's.
    unsafe extern "C-unwind" fn gettop(l: State) -> c_int {
        unsafe { ffi::lua_gettop(l.cast()) }
    }
    unsafe extern "C-unwind" fn settop(l: State, index: c_int) {
        unsafe { ffi::lua_settop(l.cast(), index) }
    }
    unsafe extern "C-unwind" fn checkstack(l: State, size: c_int) -> c_int {
        unsafe { ffi::lua_checkstack(l.cast(), size) }
    }
    unsafe extern "C-unwind" fn pushvalue(l: State, index: c_int) {
        unsafe { ffi::lua_pushvalue(l.cast(), index) }
    }
    unsafe extern "C-unwind" fn type_of(l: State, index: c_int) -> c_int {
        unsafe { ffi::lua_type(l.cast(), index) }
    }
    unsafe extern "C-unwind" fn toboolean(l: State, index: c_int) -> c_int {
        unsafe { ffi::lua_toboolean(l.cast(), index) }
    }
    unsafe extern "C-unwind" fn tonumberx(l: State, index: c_int, isnum: *mut c_int) -> f64 {
        unsafe {
            if !isnum.is_null() {
                *isnum = ffi::lua_isnumber(l.cast(), index);
            }
            ffi::lua_tonumber(l.cast(), index)
        }
    }
    unsafe extern "C-unwind" fn tolstring(
        l: State,
        index: c_int,
        len: *mut usize,
    ) -> *const c_char {
        unsafe { ffi::lua_tolstring(l.cast(), index, len) }
    }
    unsafe extern "C-unwind" fn next(l: State, index: c_int) -> c_int {
        unsafe { ffi::lua_next(l.cast(), index) }
    }
    unsafe extern "C-unwind" fn pushnil(l: State) {
        unsafe { ffi::lua_pushnil(l.cast()) }
    }
    unsafe extern "C-unwind" fn pushnumber(l: State, value: f64) {
        unsafe { ffi::lua_pushnumber(l.cast(), value) }
    }
    unsafe extern "C-unwind" fn pushboolean(l: State, value: c_int) {
        unsafe { ffi::lua_pushboolean(l.cast(), value) }
    }
    unsafe extern "C-unwind" fn pushlstring(
        l: State,
        text: *const c_char,
        len: usize,
    ) -> *const c_char {
        unsafe { ffi::lua_pushlstring_(l.cast(), text, len) };
        std::ptr::null()
    }
    unsafe extern "C-unwind" fn pushcclosure(l: State, function: CFunction, upvalues: c_int) {
        // The same function under Lua 5.1's C-unwind type.
        let function = unsafe { std::mem::transmute::<CFunction, ffi::lua_CFunction>(function) };
        unsafe { ffi::lua_pushcclosure(l.cast(), function, upvalues) }
    }
    unsafe extern "C-unwind" fn createtable(l: State, array: c_int, records: c_int) {
        unsafe { ffi::lua_createtable(l.cast(), array, records) }
    }
    unsafe extern "C-unwind" fn rawget(l: State, index: c_int) {
        unsafe { ffi::lua_rawget_(l.cast(), index) }
    }
    unsafe extern "C-unwind" fn rawset(l: State, index: c_int) {
        unsafe { ffi::lua_rawset(l.cast(), index) }
    }
    unsafe extern "C-unwind" fn rawgeti(l: State, index: c_int, n: c_int) {
        unsafe { ffi::lua_rawgeti_(l.cast(), index, n) }
    }

    /// Lua 5.1's API for the link, installed once for the whole test binary.
    pub(crate) fn lua51() -> &'static LuaApi {
        install_api(LuaApi {
            gettop,
            settop,
            checkstack,
            pushvalue,
            type_of,
            toboolean,
            tonumberx,
            tolstring,
            next,
            pushnil,
            pushnumber,
            pushboolean,
            pushlstring,
            pushcclosure,
            createtable,
            rawget,
            rawset,
            rawgeti,
            globals: Globals::Pseudo(ffi::LUA_GLOBALSINDEX),
        });
        api().unwrap()
    }

    /// A Lua state with its libraries, closed when dropped.
    pub(crate) struct Lua(State);

    impl Lua {
        pub(crate) fn new() -> Self {
            let l = unsafe { ffi::luaL_newstate() };
            assert!(!l.is_null());
            unsafe { ffi::luaL_openlibs(l) };
            Self(l.cast())
        }

        // The detour tests, Windows x64 only, run their game script in it.
        #[cfg_attr(not(all(windows, target_arch = "x86_64")), allow(dead_code))]
        pub(crate) fn state(&self) -> State {
            self.0
        }

        /// Runs `code`; returns its results as text, joined by `|`, or the
        /// error.
        pub(crate) fn run(&self, code: &str) -> Result<String, String> {
            run_in(self.0, code)
        }

        /// What the hook's `print` detour does.
        pub(crate) fn register(&self) {
            let api = lua51();
            unsafe { register(api, self.0) };
        }
    }

    /// Runs `code` in `l`; returns its results as text, joined by `|`, or
    /// the error.
    pub(crate) fn run_in(l: State, code: &str) -> Result<String, String> {
        {
            let l: *mut ffi::lua_State = l.cast();
            let wrapped = format!(
                "local r = {{ n = 0 }} \
                 local function pack(...) \
                   r.n = select('#', ...) \
                   for i = 1, r.n do r[i] = (select(i, ...)) end \
                 end \
                 pack((function() {code} end)()) \
                 local out = {{}} \
                 for i = 1, r.n do out[#out + 1] = tostring(r[i]) end \
                 return table.concat(out, '|')"
            );
            let source = CString::new(wrapped).unwrap();
            unsafe {
                let top = ffi::lua_gettop(l);
                let status = ffi::luaL_loadstring(l, source.as_ptr());
                let status = if status == 0 {
                    ffi::lua_pcall(l, 0, 1, 0)
                } else {
                    status
                };
                let mut len = 0;
                let text = ffi::lua_tolstring(l, -1, &raw mut len);
                let text = if text.is_null() {
                    String::new()
                } else {
                    String::from_utf8_lossy(std::slice::from_raw_parts(text.cast::<u8>(), len))
                        .into_owned()
                };
                ffi::lua_settop(l, top);
                if status == 0 { Ok(text) } else { Err(text) }
            }
        }
    }

    impl Drop for Lua {
        fn drop(&mut self) {
            unsafe { ffi::lua_close(self.0.cast()) };
        }
    }

    fn text<const N: usize>(s: &str) -> Text<N> {
        Text::new(s).unwrap()
    }

    pub(crate) fn depot_build() -> Action {
        Action::BuildConstruction(ConstructionBuild {
            file: text("depot/road_depot_era_a.con"),
            transform: Transform {
                basis: [0, 1_000_000, 0, -1_000_000, 0, 0, 0, 0, 1_000_000],
                origin: Pos {
                    x: 1_250_500,
                    y: -300_000,
                    z: 20_000,
                },
            },
            params: BoundedVec::new(vec![
                Param {
                    key: text("seed"),
                    value: ParamValue::Int(1234),
                },
                Param {
                    key: text("paramX"),
                    value: ParamValue::Fixed(2_500_000),
                },
            ])
            .unwrap(),
            name: text("Depot"),
            replaces: None,
        })
    }

    /// The depot as the mod hands it over: metres and plain fractions.
    const DEPOT_TABLE: &str = "{ BuildConstruction = { \
        file = 'depot/road_depot_era_a.con', \
        transform = { basis = { 0, 1, 0, -1, 0, 0, 0, 0, 1 }, origin = { x = 1250.5, y = -300, z = 20 } }, \
        params = { { key = 'seed', value = { Int = 1234 } }, { key = 'paramX', value = { Fixed = 2.5 } } }, \
        name = 'Depot' } }";

    fn reset() {
        take_commands();
        take_log();
        let _ = end_batch();
    }

    #[test]
    fn a_checkpoint_is_due_in_the_last_update_of_its_batch_only() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        reset();
        let lua = Lua::new();
        lua.register();
        // Three updates, ending at a checkpoint: each update begins with
        // take(), and only the third may report lanes.
        begin_batch(&[], 3, true).unwrap();
        let mut due = Vec::new();
        for _ in 0..3 {
            lua.run("tpf3mp_native.take()").unwrap();
            due.push(lua.run("return tpf3mp_native.checkpoint()").unwrap());
        }
        assert_eq!(due, ["false", "false", "true"]);
        assert_eq!(
            lua.run("return tpf3mp_native.lanes({ [0] = 'net', [3] = 'vehicles' })"),
            Ok("true".into())
        );
        assert_eq!(
            lua.run("return tpf3mp_native.checkpoint()"),
            Ok("false".into()),
            "read once"
        );
        assert_eq!(
            end_batch(),
            Ok(Some(vec![
                (0, "net".to_owned()),
                (3, "vehicles".to_owned())
            ]))
        );
        // A batch that does not end at a checkpoint wants none, and takes
        // none.
        begin_batch(&[], 1, false).unwrap();
        lua.run("tpf3mp_native.take()").unwrap();
        assert_eq!(
            lua.run("return tpf3mp_native.checkpoint()"),
            Ok("false".into())
        );
        let refused = lua
            .run("return tpf3mp_native.lanes({ [0] = 'x' })")
            .unwrap();
        assert!(refused.starts_with("false|no checkpoint"), "{refused}");
        assert_eq!(end_batch(), Ok(None));
        // A checkpoint whose lanes never came ends without them.
        begin_batch(&[], 1, true).unwrap();
        lua.run("tpf3mp_native.take()").unwrap();
        assert_eq!(end_batch(), Ok(None));
    }

    #[test]
    fn lanes_refuses_what_is_not_lanes() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        reset();
        let lua = Lua::new();
        lua.register();
        for (lanes, why) in [
            ("'text'", "the lanes are a table"),
            ("{ x = 'a' }", "whole number"),
            ("{ [0.5] = 'a' }", "whole number"),
            ("{ [70000] = 'a' }", "whole number"),
            ("{ [1] = 5 }", "lane 1 is not a string"),
            ("{ [1] = string.rep('a', 5000) }", "longer than"),
        ] {
            begin_batch(&[], 1, true).unwrap();
            lua.run("tpf3mp_native.take()").unwrap();
            let result = lua
                .run(&format!("return tpf3mp_native.lanes({lanes})"))
                .unwrap();
            assert!(result.starts_with("false|"), "{lanes}: {result}");
            assert!(result.contains(why), "{lanes}: {result}");
            assert_eq!(end_batch(), Ok(None), "{lanes}: nothing kept");
        }
    }

    #[test]
    fn a_state_gets_the_table_once_even_with_strict_globals() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let lua = Lua::new();
        // A state that refuses new globals, as some game states may.
        lua.run(
            "setmetatable(_G, { __newindex = function(_, k) error('undeclared ' .. k) end, \
                                __index = function(_, k) error('undeclared ' .. k) end })",
        )
        .unwrap();
        lua.register();
        assert_eq!(
            lua.run(
                "return tpf3mp_native.version, type(tpf3mp_native.command), \
                 type(tpf3mp_native.take), type(tpf3mp_native.log), type(tpf3mp_native.poll), \
                 type(tpf3mp_native.saved), type(tpf3mp_native.world), type(tpf3mp_native.room), \
                 type(tpf3mp_native.checkpoint), type(tpf3mp_native.lanes), \
                 type(tpf3mp_native.clicks), type(tpf3mp_native.replaying)"
            ),
            Ok("7|function|function|function|function|function|function|function|function|function|function|function".into())
        );
        // A second print keeps the first table.
        lua.run("rawset(tpf3mp_native, 'mark', true)").unwrap();
        lua.register();
        assert_eq!(lua.run("return tpf3mp_native.mark"), Ok("true".into()));
    }

    #[test]
    fn command_queues_the_actions_the_schema_takes() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        reset();
        let lua = Lua::new();
        lua.register();
        assert_eq!(
            lua.run(&format!("return tpf3mp_native.command({DEPOT_TABLE})")),
            Ok("true".into())
        );
        let commands = take_commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(Action::from_payload(&commands[0]).unwrap(), depot_build());
        // The stack is as it was: the result alone.
        assert_eq!(
            lua.run(&format!(
                "local n = select('#', tpf3mp_native.command({DEPOT_TABLE})) return n"
            )),
            Ok("1".into())
        );
        take_commands();
    }

    #[test]
    fn command_refuses_what_the_schema_does_not_take_and_queues_nothing() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        reset();
        let lua = Lua::new();
        lua.register();
        for (action, why) in [
            ("{ Nope = 1 }", "Nope"),
            ("print", "an action is a table"),
            (
                "{ BuildConstruction = { file = print } }",
                "a function has no place",
            ),
            ("{ [{}] = 1 }", "a table used as a key"),
        ] {
            let result = lua
                .run(&format!(
                    "local ok, reason = tpf3mp_native.command({action}) return ok, reason"
                ))
                .unwrap();
            assert!(result.starts_with("false|"), "{action}: {result}");
            assert!(result.contains(why), "{action}: {result}");
        }
        let deep = lua
            .run(
                "local t = {} local c = t for i = 1, 40 do c.x = {} c = c.x end \
                 local ok, reason = tpf3mp_native.command(t) return ok, reason",
            )
            .unwrap();
        assert!(deep.contains("nested deeper"), "{deep}");
        assert!(take_commands().is_empty());
    }

    #[test]
    fn take_hands_a_batch_to_its_first_update_only() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        reset();
        let lua = Lua::new();
        lua.register();
        assert_eq!(lua.run("return tpf3mp_native.take()"), Ok("nil".into()));
        begin_batch(&[depot_build()], 1, false).unwrap();
        assert_eq!(
            lua.run(
                "local first = tpf3mp_native.take() local again = tpf3mp_native.take() \
                 local b = first[1].BuildConstruction \
                 return #first, b.file, b.transform.origin.x, b.params[2].value.Fixed, again"
            ),
            Ok("1|depot/road_depot_era_a.con|1250.5|2.5|nil".into())
        );
        assert_eq!(end_batch(), Ok(None));
        // An action nobody took is found at the batch's end.
        begin_batch(&[depot_build()], 1, false).unwrap();
        assert!(
            end_batch()
                .unwrap_err()
                .contains("did not take the 1 action")
        );
        // A batch without actions has nothing to take.
        begin_batch(&[], 1, false).unwrap();
        assert_eq!(lua.run("return tpf3mp_native.take()"), Ok("nil".into()));
        assert_eq!(end_batch(), Ok(None));
    }

    #[test]
    fn log_lines_reach_the_hook_log() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        reset();
        let lua = Lua::new();
        lua.register();
        lua.run("tpf3mp_native.log('the game script is linked') tpf3mp_native.log(42)")
            .unwrap();
        assert_eq!(
            take_log(),
            vec!["mod: the game script is linked".to_owned()]
        );
    }

    #[test]
    fn the_gui_polls_each_request_once_and_answers_saves() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let lua = Lua::new();
        lua.register();
        let _ = take_save_answer();
        assert_eq!(lua.run("return tpf3mp_native.poll()"), Ok("nil".into()));
        request_save("tpf3mp_77_5");
        assert_eq!(
            lua.run("local r = tpf3mp_native.poll() return r.save, r.load, tpf3mp_native.poll()"),
            Ok("tpf3mp_77_5|nil|nil".into())
        );
        assert_eq!(take_save_answer(), None, "not answered yet");
        lua.run("tpf3mp_native.saved('tpf3mp_77_5', true)").unwrap();
        assert_eq!(take_save_answer(), Some(Ok("tpf3mp_77_5".into())));
        assert_eq!(take_save_answer(), None, "once");
        request_save("tpf3mp_77_6");
        lua.run("tpf3mp_native.poll() tpf3mp_native.saved('tpf3mp_77_6', false, 'disk full')")
            .unwrap();
        assert_eq!(take_save_answer(), Some(Err("disk full".into())));
        request_load("tpf3mp_room_77");
        // A world whose GUI starts before the GUI takes the load is not it.
        lua.run("tpf3mp_native.world()").unwrap();
        assert!(!load_done());
        assert_eq!(
            lua.run("return tpf3mp_native.poll().load"),
            Ok("tpf3mp_room_77".into())
        );
        assert!(!load_done(), "taken, not loaded yet");
        lua.run("tpf3mp_native.world()").unwrap();
        assert!(load_done(), "the next world is the loaded one");
        assert!(!load_done(), "once");
    }

    #[test]
    fn clicks_is_nil_without_the_build_detours_and_replaying_sets_the_flag() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let lua = Lua::new();
        lua.register();
        assert_eq!(lua.run("return tpf3mp_native.clicks()"), Ok("nil".into()));
        lua.run("tpf3mp_native.replaying(true)").unwrap();
        lua.run("tpf3mp_native.replaying(false)").unwrap();
        lua.run("tpf3mp_native.replaying()").unwrap();
    }

    #[test]
    fn room_says_whether_the_rooms_game_runs() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let lua = Lua::new();
        lua.register();
        set_in_room(false);
        assert_eq!(lua.run("return tpf3mp_native.room()"), Ok("false".into()));
        set_in_room(true);
        assert_eq!(lua.run("return tpf3mp_native.room()"), Ok("true".into()));
        set_in_room(false);
    }
}

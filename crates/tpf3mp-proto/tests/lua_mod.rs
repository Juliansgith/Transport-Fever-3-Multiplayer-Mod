//! The Lua mod (`mod/tpf3mp_1`) as Transport Fever 3 loads it: laid out as
//! mods made for build 40391 are (`mod.json`, `_content.json`,
//! `_metadata/modinfo.json`, `content/`), and its entry script run in a
//! stand-in for the game's GUI state (`tests/lua/fake_gui.lua`), with and
//! without the hook. What the layout rests on is in
//! `investigation/TF3_MODS_2026-09-27.md`.

#![allow(clippy::unwrap_used)]

mod common;

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use mlua::{Function, Lua, Table};

const MOD_ID: &str = "tpf3mp_1";
const FAKE_GUI: &str = include_str!("lua/fake_gui.lua");

fn mod_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../mod")
        .join(MOD_ID)
}

fn json(path: &str) -> serde_json::Value {
    let text = std::fs::read_to_string(mod_dir().join(path)).unwrap();
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{path}: {error}"))
}

/// Every file under `content/`, by its path relative to it.
fn content_files() -> BTreeSet<String> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy();
                out.insert(rel.replace('\\', "/"));
            }
        }
    }
    let root = mod_dir().join("content");
    let mut out = BTreeSet::new();
    walk(&root, &root, &mut out);
    out
}

#[test]
fn the_mod_is_laid_out_as_tf3_mods_are() {
    assert!(
        !mod_dir().join("mod.lua").exists(),
        "a TPF2 mod.lua is left over"
    );

    let manifest = json("mod.json");
    assert_eq!(manifest["modId"], MOD_ID, "modId is the folder's name");
    assert_eq!(manifest["severityAdd"], "None");
    assert_eq!(manifest["severityRemove"], "None");
    assert!(manifest["revision"].is_u64());
    for script in ["preRunScript", "runScript", "postRunScript"] {
        let file = manifest[script]["fileName"].as_str().unwrap();
        assert!(
            file.is_empty() || file.starts_with(&format!("{MOD_ID}::/")),
            "{script} names another mod's file: {file}"
        );
    }

    let info = json("_metadata/modinfo.json");
    assert_eq!(info["name"], "TPF3-MP");
    assert!(info["summary"].is_string() && info["description"].is_string());
    assert!(
        info["tags"]
            .as_array()
            .unwrap()
            .contains(&"Script Mod".into())
    );

    let listed: Vec<String> = json("_content.json")["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file.as_str().unwrap().to_owned())
        .collect();
    let listed_set: BTreeSet<String> = listed.iter().cloned().collect();
    assert_eq!(listed.len(), listed_set.len(), "a file is listed twice");
    assert_eq!(
        listed_set,
        content_files(),
        "_content.json must list exactly the files under content/"
    );
}

#[test]
fn every_resource_names_a_script_the_mod_has() {
    let files = content_files();
    let resources: Vec<&String> = files.iter().filter(|f| f.ends_with(".res.lua")).collect();
    assert!(!resources.is_empty(), "no GUI resource loads the mod");
    for resource in resources {
        let text = std::fs::read_to_string(mod_dir().join("content").join(resource)).unwrap();
        let prefix = format!("\"{MOD_ID}::/");
        let start = text
            .find(&prefix)
            .unwrap_or_else(|| panic!("{resource} names no file of this mod"))
            + prefix.len();
        let target = &text[start..];
        let target = &target[..target.find('"').unwrap()];
        let (script, recipe) = target.split_once('@').unwrap();
        assert!(
            files.contains(&format!("{script}.lua")),
            "{resource} names {script}.lua, which is not in content/"
        );
        assert!(!recipe.is_empty());
    }
}

/// A Lua state standing in for the game's GUI state, with the mod's files
/// readable through `mod_source`.
fn gui() -> Lua {
    let lua = Lua::new();
    let source = lua
        .create_function(|_, rel: String| {
            assert!(!rel.contains(".."), "{rel} leaves the mod");
            let path = mod_dir().join("content").join(&rel);
            std::fs::read_to_string(&path)
                .map_err(|error| mlua::Error::external(format!("{rel}: {error}")))
        })
        .unwrap();
    lua.globals().set("mod_source", source).unwrap();
    // What the hook does with an action table: convert it with the schema.
    let schema_check = lua
        .create_function(|_, action: mlua::Value| {
            Ok(
                match tpf3mp_proto::lua::action_from_lua(&common::tree(&action)) {
                    Ok(_) => (true, None),
                    Err(error) => (false, Some(error.to_string())),
                },
            )
        })
        .unwrap();
    lua.globals().set("schema_check", schema_check).unwrap();
    lua.load(FAKE_GUI).set_name("@fake_gui.lua").exec().unwrap();
    lua
}

fn log(lua: &Lua) -> String {
    lua.load("return logText()").eval().unwrap()
}

/// Loads the plugin, mounts it and runs `steps` frames.
fn run_frames(lua: &Lua, steps: usize) {
    lua.load(format!(
        "local m = mount(loadPlugin()); for _ = 1, {steps} do m.render(); m.step() end"
    ))
    .set_name("@frames")
    .exec()
    .unwrap_or_else(|error| panic!("{error}\n{}", log(lua)));
}

#[test]
fn without_the_hook_the_mod_loads_and_does_nothing() {
    let lua = gui();
    let before: Vec<String> = loaded_names(&lua);
    run_frames(&lua, 3);
    let log = log(&lua);
    assert_eq!(
        log,
        "[tpf3mp] modules loaded\n\
         [tpf3mp] no hook in this game; this is the plain game",
        "started once, however many frames"
    );

    // Only the mod's own names were added to package.loaded.
    let added: Vec<String> = loaded_names(&lua)
        .into_iter()
        .filter(|name| !before.contains(name))
        .collect();
    assert_eq!(
        added,
        [
            "tpf3mp.bridge",
            "tpf3mp.engine",
            "tpf3mp.geom",
            "tpf3mp.guard",
            "tpf3mp.roads"
        ]
    );
}

/// The names in package.loaded, sorted.
fn loaded_names(lua: &Lua) -> Vec<String> {
    let loaded: Table = lua.load("return package.loaded").eval::<Table>().unwrap();
    let mut names: Vec<String> = loaded
        .pairs::<String, mlua::Value>()
        .map(|pair| pair.unwrap().0)
        .collect();
    names.sort();
    names
}

const FAKE_HOOK: &str = r#"
HOOK = { logged = {}, commands = {}, batch = nil, request = nil, saved = {}, worlds = 0,
         room = false }
tpf3mp_native = {
    version = 5,
    command = function(action)
        local ok, why = schema_check(action)
        if ok then HOOK.commands[#HOOK.commands + 1] = action end
        return ok, why
    end,
    take = function()
        local batch = HOOK.batch
        HOOK.batch = nil
        return batch
    end,
    log = function(line) HOOK.logged[#HOOK.logged + 1] = line end,
    poll = function()
        local request = HOOK.request
        HOOK.request = nil
        return request
    end,
    saved = function(name, ok, why)
        HOOK.saved[#HOOK.saved + 1] = tostring(name) .. ' ' .. tostring(ok) .. ' ' .. tostring(why)
    end,
    world = function() HOOK.worlds = HOOK.worlds + 1 end,
    room = function() return HOOK.room end,
}
"#;

/// The GUI state's api.cmd, as much of it as the guard's tests use: three
/// factories, and a sendCommand that keeps what it was sent.
const FAKE_CMD: &str = r#"
SENT = {}
api = api or {}
api.cmd = {
    makeGameSetSpeedCmd = function(speed) return { kind = 'speed', speed = speed } end,
    makeVehicleBuyCmd = function(player, depot, config) return { kind = 'buy', depot = depot } end,
    makeLineCreateCmd = function(line) return { kind = 'line' } end,
    sendCommand = function(command, ...)
        SENT[#SENT + 1] = { command = command, extra = select('#', ...), callback = (...) }
    end,
}
"#;

/// The game's save and load, as the GUI state has them.
const FAKE_APP: &str = r#"
APP = { saves = {}, loads = {} }
app = {
    saveGame = function(name, callback, isMapEditor, skipSetName)
        APP.saves[#APP.saves + 1] = { name = name, callback = callback,
                                      isMapEditor = isMapEditor, skipSetName = skipSetName }
    end,
    loadGame = function(id, isMapEditor, info)
        APP.loads[#APP.loads + 1] = { id = id, isMapEditor = isMapEditor }
    end,
    SaveGameNamespace = { getSavegame = function() return "savegame" end },
}
api = { type = { SavegameId = { new = function() return {} end } } }
"#;

#[test]
fn with_the_hook_the_gui_links_once() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_CMD).exec().unwrap();
    run_frames(&lua, 2);
    assert_eq!(
        log(&lua),
        "[tpf3mp] modules loaded\n[tpf3mp] linked to the hook"
    );
    let logged: String = lua
        .load("return table.concat(HOOK.logged, '|')")
        .eval()
        .unwrap();
    assert_eq!(
        logged,
        "the GUI is linked|the guard is on 3 command factories"
    );
    let worlds: u32 = lua.load("return HOOK.worlds").eval().unwrap();
    assert_eq!(worlds, 1, "the world's GUI started once");
}

#[test]
fn the_gui_saves_what_the_hook_asks_and_answers_when_written() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_APP).exec().unwrap();
    lua.load("M = mount(loadPlugin())").exec().unwrap();
    lua.load("M.step() HOOK.request = { save = 'tpf3mp_77_5' } M.step()")
        .exec()
        .unwrap();
    let (name, map_editor, skip): (String, bool, bool) = lua
        .load("local s = APP.saves[1] return s.name, s.isMapEditor, s.skipSetName")
        .eval()
        .unwrap();
    assert_eq!(name, "tpf3mp_77_5");
    assert!(!map_editor);
    assert!(skip, "the player's own save name is left alone");
    let answered: usize = lua.load("return #HOOK.saved").eval().unwrap();
    assert_eq!(answered, 0, "not written yet");
    lua.load("APP.saves[1].callback()").exec().unwrap();
    let saved: Vec<String> = lua.load("return HOOK.saved").eval().unwrap();
    assert_eq!(saved, ["tpf3mp_77_5 true nil"]);
    // A save the game refuses at once is answered as failed.
    lua.load(
        "app.saveGame = function() error('no disk') end \
         HOOK.request = { save = 'tpf3mp_77_6' } M.step()",
    )
    .exec()
    .unwrap();
    let saved: Vec<String> = lua.load("return HOOK.saved").eval().unwrap();
    assert!(saved[1].starts_with("tpf3mp_77_6 false "), "{}", saved[1]);
    assert!(saved[1].ends_with("no disk"), "{}", saved[1]);
}

#[test]
fn the_gui_loads_the_rooms_world_from_the_save_folder() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_APP).exec().unwrap();
    lua.load(
        "M = mount(loadPlugin()) M.step() HOOK.request = { load = 'tpf3mp_room_77' } M.step()",
    )
    .exec()
    .unwrap();
    let loaded: String = lua
        .load(
            "local l = APP.loads[1] \
             return l.id.path .. '|' .. l.id.saveGameName .. '|' .. l.id.saveGameNamespace \
               .. '|' .. tostring(l.isMapEditor)",
        )
        .eval()
        .unwrap();
    assert_eq!(loaded, "|tpf3mp_room_77|savegame|false");
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert_eq!(logged.last().unwrap(), "loading the room's world");
}

#[test]
fn a_hook_of_another_version_is_not_used() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load("tpf3mp_native.version = 1").exec().unwrap();
    run_frames(&lua, 1);
    assert!(
        log(&lua).ends_with(
            "[tpf3mp] the hook speaks bridge version 1, the mod 5; this is the plain game"
        ),
        "{}",
        log(&lua)
    );
    let logged: usize = lua.load("return #HOOK.logged").eval().unwrap();
    assert_eq!(logged, 0, "nothing was said to a hook of another version");
}

/// The bridge on its own, loaded as the entry script loads it.
fn bridge(lua: &Lua) -> Table {
    lua.load("return ug_require('tpf3mp_1::/scripts/tpf3mp/bridge.lua')")
        .eval()
        .unwrap()
}

#[test]
fn the_bridge_hands_over_only_actions_the_schema_takes() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    let bridge = bridge(&lua);
    let attach: Function = bridge.get("attach").unwrap();
    let native: Table = lua.globals().get("tpf3mp_native").unwrap();
    let link: Table = attach.call(native).unwrap();
    lua.globals().set("LINK", link).unwrap();

    let refusals: Vec<String> = lua
        .load(
            "local out = {}
             local function try(p) local ok, why = LINK:command(p); out[#out + 1] = ok and 'ok' or why end
             try('bytes')
             try({ SellVehicle = { vehicles = { 7, 9 } } })
             try({ SellVehicle = { vehicles = { 0.5 } } })
             try({ SellVehicle = { vehicles = {}, colour = 'red' } })
             tpf3mp_native.command = function() return false end
             try({ SellVehicle = { vehicles = { 7 } } })
             tpf3mp_native.command = function() error('ring full') end
             try({ SellVehicle = { vehicles = { 7 } } })
             return out",
        )
        .eval()
        .unwrap();
    assert_eq!(refusals[0], "an action is a table");
    assert_eq!(refusals[1], "ok");
    assert_eq!(
        refusals[2],
        "the hook refused the action: SellVehicle.vehicles[1]: not a whole number: 0.5"
    );
    assert_eq!(
        refusals[3],
        "the hook refused the action: SellVehicle: variant has no field colour"
    );
    assert_eq!(refusals[4], "the hook refused the action: no reason given");
    assert!(refusals[5].starts_with("the hook refused: "));
    assert!(refusals[5].ends_with("ring full"));
    let sent: usize = lua.load("return #HOOK.commands").eval().unwrap();
    assert_eq!(sent, 1, "only the action the schema took reached the room");
}

#[test]
fn take_is_a_list_or_nothing_and_never_raises() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    let bridge = bridge(&lua);
    lua.globals().set("BRIDGE", bridge).unwrap();
    let results: Vec<String> = lua
        .load(
            "local link = BRIDGE.attach(tpf3mp_native)
             local out = {}
             out[#out + 1] = tostring(link:take())
             HOOK.batch = { { SellVehicle = { vehicles = { 7 } } } }
             out[#out + 1] = tostring(#link:take())
             out[#out + 1] = tostring(link:take())
             tpf3mp_native.take = function() error('boom') end
             out[#out + 1] = tostring(link:take())
             return out",
        )
        .eval()
        .unwrap();
    assert_eq!(results, ["nil", "1", "nil", "nil"]);
}

#[test]
fn attach_refuses_a_partial_hook() {
    let lua = gui();
    let bridge = bridge(&lua);
    lua.globals().set("BRIDGE", bridge).unwrap();
    let reasons: Vec<String> = lua
        .load(
            "local out = {}
             local function why(t) local _, r = BRIDGE.attach(t); out[#out + 1] = r end
             why(nil)
             why('hook')
             why({ version = 5, command = print, log = print })
             return out",
        )
        .eval()
        .unwrap();
    assert_eq!(
        reasons,
        [
            "no hook in this game",
            "tpf3mp_native is not a table",
            "the hook has no take()",
        ]
    );
}

/// The game bar's text, if the plugin shows any.
fn shown(lua: &Lua) -> Option<String> {
    lua.load(
        "local c = M.render().params.children[1] \
         return c and c.params.text",
    )
    .eval()
    .unwrap()
}

#[test]
fn in_the_rooms_game_the_gui_refuses_what_the_room_cannot_carry() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_CMD).exec().unwrap();
    lua.load("M = mount(loadPlugin()) M.step()").exec().unwrap();

    // Before the room's game every command is sent, arguments as given.
    lua.load("api.cmd.sendCommand(api.cmd.makeVehicleBuyCmd(1, 2, {}))")
        .exec()
        .unwrap();
    let (sent, extra): (usize, usize) = lua.load("return #SENT, SENT[1].extra").eval().unwrap();
    assert_eq!(
        (sent, extra),
        (1, 0),
        "a callback left out is not passed as nil"
    );

    // In the room's game the speed row's speed is sent; a vehicle bought
    // is refused, and its callback hears so on the next frame.
    lua.load(
        "HOOK.room = true \
         api.cmd.sendCommand(api.cmd.makeGameSetSpeedCmd(4)) \
         CALLED = nil \
         BUY = api.cmd.makeVehicleBuyCmd(1, 2, {}) \
         api.cmd.sendCommand(BUY, function(data, ok, entities) \
             CALLED = { data = data, ok = ok, entities = #entities } end)",
    )
    .exec()
    .unwrap();
    let (sent, speed): (usize, u32) = lua
        .load("return #SENT, SENT[2].command.speed")
        .eval()
        .unwrap();
    assert_eq!((sent, speed), (2, 4), "the speed went, the vehicle did not");
    let called: bool = lua.load("return CALLED ~= nil").eval().unwrap();
    assert!(!called, "not within sendCommand");
    lua.load("M.step()").exec().unwrap();
    let (same, ok, entities): (bool, bool, usize) = lua
        .load("return CALLED.data == BUY, CALLED.ok, CALLED.entities")
        .eval()
        .unwrap();
    assert!(same && !ok, "the callback heard the command failed");
    assert_eq!(entities, 0);
    assert_eq!(
        shown(&lua).as_deref(),
        Some("Not in multiplayer yet: buying vehicles")
    );

    // A command no factory made is refused too.
    lua.load("api.cmd.sendCommand({ kind = 'forged' }) M.step()")
        .exec()
        .unwrap();
    assert_eq!(
        shown(&lua).as_deref(),
        Some("Not in multiplayer yet: this action")
    );
    let sent: usize = lua.load("return #SENT").eval().unwrap();
    assert_eq!(sent, 2);
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert!(
        logged.contains(
            &"refused the player's makeVehicleBuyCmd in the room's game (1 so far)".to_owned()
        ),
        "{logged:?}"
    );
    assert!(
        logged.contains(
            &"refused the player's command no factory made in the room's game (1 so far)"
                .to_owned()
        ),
        "{logged:?}"
    );

    // The notice goes after a few seconds; after the room's game, commands
    // are sent again.
    lua.load("for _ = 1, 400 do M.step() end").exec().unwrap();
    assert_eq!(shown(&lua), None);
    lua.load("HOOK.room = false api.cmd.sendCommand(api.cmd.makeLineCreateCmd({}))")
        .exec()
        .unwrap();
    let sent: usize = lua.load("return #SENT").eval().unwrap();
    assert_eq!(sent, 3);
}

#[test]
fn the_guard_goes_on_once_and_a_hook_that_cannot_say_means_the_room() {
    let lua = gui();
    lua.load(FAKE_CMD).exec().unwrap();
    let results: Vec<String> = lua
        .load(
            "local guard = ug_require('tpf3mp_1::/scripts/tpf3mp/guard.lua')
             local env = { inRoom = function() return true end,
                           refused = function() end, later = function() end }
             local out = {}
             out[#out + 1] = tostring(guard.install(api.cmd, env))
             local send = api.cmd.sendCommand
             out[#out + 1] = tostring(guard.install(api.cmd, env))
             out[#out + 1] = tostring(api.cmd.sendCommand == send)
             out[#out + 1] = select(2, guard.install(nil, env))
             out[#out + 1] = select(2, guard.install({}, env))
             local bridge = ug_require('tpf3mp_1::/scripts/tpf3mp/bridge.lua')
             local native = { version = 5 }
             for _, n in ipairs({ 'command', 'take', 'log', 'poll', 'saved', 'world' }) do
                 native[n] = function() end
             end
             native.room = function() error('gone') end
             out[#out + 1] = tostring(bridge.attach(native):room())
             native.room = function() return 1 end
             out[#out + 1] = tostring(bridge.attach(native):room())
             return out",
        )
        .eval()
        .unwrap();
    assert_eq!(
        results,
        [
            "3",
            "3",
            "true",
            "api.cmd is not a table",
            "api.cmd has no sendCommand",
            "true",
            "false"
        ]
    );
}

#[test]
fn every_game_script_names_a_script_the_mod_has() {
    let files = content_files();
    let scripts: Vec<&String> = files.iter().filter(|f| f.ends_with(".gs.lua")).collect();
    assert!(
        !scripts.is_empty(),
        "no game script applies the room's actions"
    );
    for script in scripts {
        let folder = script.rsplit_once('/').map_or("", |(folder, _)| folder);
        let text = std::fs::read_to_string(mod_dir().join("content").join(script)).unwrap();
        let mut named = 0;
        for part in text.split("fileName = \"").skip(1) {
            let target = &part[..part.find('"').unwrap()];
            let (file, function) = target.split_once('@').unwrap();
            assert!(
                files.contains(&format!("{folder}/{file}.lua")),
                "{script} names {file}.lua, which is not in {folder}/"
            );
            assert!(!function.is_empty());
            named += 1;
        }
        assert!(named > 0, "{script} names no script");
    }
}

/// A stand-in for an engine (game script) state: the commands it is sent
/// run at once, as the game's do there.
const FAKE_ENGINE: &str = r#"
SENT = {}
REFUSE = false
local function vec4(x, y, z, w) return { x, y, z, w } end
api = {
    type = {
        Vec4f = { new = vec4 },
        Mat4f = { new = function(a, b, c, d) return { a, b, c, d } end },
        SimpleProposal = {
            new = function() return { constructionsToAdd = {} } end,
            ConstructionEntity = { new = function() return {} end },
        },
    },
    engine = { util = { getPlayer = function() return 25 end } },
    cmd = {
        makeWorldBuildProposalCmd = function(proposal, context, ignoreErrors, playerInitiated)
            return { proposal = proposal, context = context, ignoreErrors = ignoreErrors,
                     playerInitiated = playerInitiated }
        end,
        sendCommand = function(command, callback)
            -- As the game in a game script's update.
            if callback ~= nil then error('Callbacks are currently disallowed') end
            if REFUSE then error('the proposal collides') end
            SENT[#SENT + 1] = command
        end,
    },
}
STATE = {
    subscribed = {},
    hasEventSubscriptions = function(self) return next(self.subscribed) ~= nil end,
    subscribeToEvent = function(self, name) self.subscribed[name] = true end,
}
"#;

/// The mod's game script in a stand-in engine state with the fake hook:
/// returns the state and the script's functions.
fn engine() -> (Lua, Table) {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load(FAKE_ENGINE).exec().unwrap();
    let source = std::fs::read_to_string(
        mod_dir()
            .join("content")
            .join("tpf3mp_sim")
            .join("tpf3mp_sim.script.lua"),
    )
    .unwrap();
    lua.load(&source)
        .set_name("@tpf3mp_sim.script.lua")
        .exec()
        .unwrap();
    let script: Table = lua.load("return data()").eval().unwrap();
    (lua, script)
}

const DEPOT: &str = "{ BuildConstruction = { \
    file = 'depot/road_depot_era_a.con', \
    transform = { basis = { 0, 1, 0, -1, 0, 0, 0, 0, 1 }, origin = { x = 1250.5, y = -300, z = 20 } }, \
    params = { { key = 'seed', value = { Int = 1234 } }, \
               { key = 'modules[3801].name', value = { Text = 'depot/module.module' } }, \
               { key = 'paramX', value = { Fixed = 2.5 } }, \
               { key = 'lit', value = { Bool = true } } }, \
    name = 'Depot' } }";

#[test]
fn the_game_script_applies_the_rooms_actions_as_the_players_own_builds() {
    let (lua, script) = engine();
    let update: Function = script.get("update").unwrap();
    lua.globals().set("UPDATE", update).unwrap();
    // No action ordered: nothing sent.
    lua.load("UPDATE({}, STATE, 0.2)").exec().unwrap();
    assert_eq!(lua.load("return #SENT").eval::<usize>().unwrap(), 0);
    lua.load(format!(
        "HOOK.batch = {{ {DEPOT} }} UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    let built: String = lua
        .load(
            "local c = SENT[1] local e = c.proposal.constructionsToAdd[1]
             local t = e.transf
             return table.concat({ e.fileName, e.name, e.playerEntity,
                 t[1][1], t[1][2], t[2][1], t[4][1], t[4][2], t[4][3], t[4][4],
                 e.params.seed, e.params.modules[3801].name, e.params.paramX, tostring(e.params.lit),
                 tostring(c.ignoreErrors), tostring(c.playerInitiated), tostring(c.context) }, '|')",
        )
        .eval()
        .unwrap();
    assert_eq!(
        built,
        "depot/road_depot_era_a.con|Depot|25|0|1|-1|1250.5|-300|20|1|1234|depot/module.module|2.5|true|false|true|nil"
    );
    // Subscribed to its console event, linked once.
    assert!(
        lua.load("return STATE.subscribed.command")
            .eval::<bool>()
            .unwrap()
    );
    assert_eq!(
        lua.load("return table.concat(HOOK.logged, '|')")
            .eval::<String>()
            .unwrap(),
        "the game script is linked"
    );
}

#[test]
fn an_action_the_game_script_cannot_apply_is_logged_not_raised() {
    let (lua, script) = engine();
    let update: Function = script.get("update").unwrap();
    lua.globals().set("UPDATE", update).unwrap();
    lua.load(
        "HOOK.batch = { { SellVehicle = { vehicles = { 7 } } } } UPDATE({}, STATE, 0.2) \
         REFUSE = true",
    )
    .exec()
    .unwrap();
    lua.load(format!(
        "HOOK.batch = {{ {DEPOT} }} UPDATE({{}}, STATE, 0.2)"
    ))
    .exec()
    .unwrap();
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert_eq!(logged.len(), 3, "{logged:?}");
    assert_eq!(logged[0], "the game script is linked");
    assert_eq!(
        logged[1],
        "action 1 of this step was not applied: this version of the mod does not apply SellVehicle yet"
    );
    // The game's own refusal, as it raised it.
    assert!(
        logged[2].starts_with("action 1 of this step was not applied: ")
            && logged[2].ends_with("the proposal collides"),
        "{}",
        logged[2]
    );
}

#[test]
fn the_console_event_hands_an_action_to_the_room() {
    let (lua, script) = engine();
    let handle: Function = script.get("handleEvent").unwrap();
    lua.globals().set("HANDLE", handle).unwrap();
    lua.load(format!(
        "HANDLE({{}}, STATE, 'console', 'tpf3mp', 'command', {DEPOT}) \
         HANDLE({{}}, STATE, 'console', 'other', 'command', {DEPOT}) \
         HANDLE({{}}, STATE, 'console', 'tpf3mp', 'command', {{ Nope = 1 }})"
    ))
    .exec()
    .unwrap();
    assert_eq!(
        lua.load("return #HOOK.commands").eval::<usize>().unwrap(),
        1,
        "only its own event, and only an action the schema takes"
    );
    let logged: Vec<String> = lua.load("return HOOK.logged").eval().unwrap();
    assert_eq!(logged[0], "the game script is linked");
    assert_eq!(logged[1], "handed a test action to the room");
    assert!(
        logged[2].starts_with("refused a test action: "),
        "{}",
        logged[2]
    );
}

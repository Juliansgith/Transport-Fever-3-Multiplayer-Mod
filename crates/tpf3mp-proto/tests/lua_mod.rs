//! The Lua mod (`mod/tpf3mp_1`) as Transport Fever 3 loads it: laid out as
//! mods made for build 40391 are (`mod.json`, `_content.json`,
//! `_metadata/modinfo.json`, `content/`), and its entry script run in a
//! stand-in for the game's GUI state (`tests/lua/fake_gui.lua`), with and
//! without the hook. What the layout rests on is in
//! `investigation/TF3_MODS_2026-09-27.md`.

#![allow(clippy::unwrap_used)]

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
    run_frames(&lua, 3);
    let log = log(&lua);
    assert_eq!(
        log,
        "[tpf3mp] modules loaded\n\
         [tpf3mp] no hook in this game; this is the plain game",
        "started once, however many frames"
    );

    // Only the mod's own names were added to package.preload.
    let preload: Table = lua.load("return package.preload").eval::<Table>().unwrap();
    for pair in preload.pairs::<String, mlua::Value>() {
        let (name, _) = pair.unwrap();
        assert!(name.starts_with("tpf3mp."), "added {name}");
    }
}

const FAKE_HOOK: &str = r#"
HOOK = { logged = {}, commands = {} }
tpf3mp_native = {
    version = 1,
    command = function(payload) HOOK.commands[#HOOK.commands + 1] = payload; return true end,
    register = function(handlers) HOOK.handlers = handlers end,
    log = function(line) HOOK.logged[#HOOK.logged + 1] = line end,
}
"#;

#[test]
fn with_the_hook_the_mod_links_and_refuses_what_it_cannot_apply() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    run_frames(&lua, 2);
    assert_eq!(
        log(&lua),
        "[tpf3mp] modules loaded\n[tpf3mp] linked to the hook"
    );
    let (logged, ok, reason, noticed): (String, bool, String, bool) = lua
        .load(
            "local ok, reason = HOOK.handlers.apply('\\1\\2')
             local noticed = HOOK.handlers.notice('Speed', '2x')
             return table.concat(HOOK.logged, '|'), ok, reason, noticed",
        )
        .eval()
        .unwrap();
    assert_eq!(logged, "the mod is linked");
    assert!(!ok, "an event this version cannot apply is refused");
    assert_eq!(reason, "this version of the mod applies no actions yet");
    assert!(noticed);
    assert!(log(&lua).ends_with("[tpf3mp] notice Speed: 2x"));
}

#[test]
fn a_hook_of_another_version_is_not_used() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    lua.load("tpf3mp_native.version = 2").exec().unwrap();
    run_frames(&lua, 1);
    assert!(
        log(&lua).ends_with(
            "[tpf3mp] the hook speaks bridge version 2, the mod 1; this is the plain game"
        ),
        "{}",
        log(&lua)
    );
    let registered: bool = lua.load("return HOOK.handlers ~= nil").eval().unwrap();
    assert!(!registered);
}

/// The bridge on its own, as the entry script's `require` finds it.
fn bridge(lua: &Lua) -> Table {
    lua.load(
        "for _, name in ipairs({ 'fixed', 'wire', 'bridge' }) do
             package.preload['tpf3mp.' .. name] = function()
                 return ug_require('tpf3mp_1::/scripts/tpf3mp/' .. name .. '.lua')
             end
         end
         return require 'tpf3mp.bridge'",
    )
    .eval()
    .unwrap()
}

#[test]
fn the_bridge_hands_over_only_payloads_the_room_can_take() {
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
             try(42)
             try('')
             try(string.rep('x', 48 * 1024 + 1))
             try(string.rep('x', 48 * 1024))
             tpf3mp_native.command = function() return false end
             try('a')
             tpf3mp_native.command = function() error('ring full') end
             try('a')
             return out",
        )
        .eval()
        .unwrap();
    assert_eq!(refusals[0], "a payload is a string of bytes");
    assert_eq!(refusals[1], "an empty payload");
    assert_eq!(refusals[2], "49153 bytes; the payload limit is 49152");
    assert_eq!(refusals[3], "ok");
    assert_eq!(refusals[4], "the hook refused the action");
    assert!(refusals[5].starts_with("the hook refused: "));
    assert!(refusals[5].ends_with("ring full"));
    let sent: usize = lua.load("return #HOOK.commands").eval().unwrap();
    assert_eq!(
        sent, 1,
        "only the payload within the limit reached the hook"
    );
}

#[test]
fn a_handler_never_raises_into_the_hook() {
    let lua = gui();
    lua.load(FAKE_HOOK).exec().unwrap();
    let bridge = bridge(&lua);
    lua.globals().set("BRIDGE", bridge).unwrap();
    let results: Vec<String> = lua
        .load(
            "local link = BRIDGE.attach(tpf3mp_native)
             local out = {}
             local ok, why = link:register({})
             out[#out + 1] = why
             link:register({ apply = function() error('boom') end })
             local a, b = HOOK.handlers.apply('x')
             out[#out + 1] = tostring(a) .. ' ' .. b
             link:register({ apply = function() end })
             a, b = HOOK.handlers.apply('x')
             out[#out + 1] = tostring(a) .. ' ' .. b
             link:register({ apply = function() return true end })
             out[#out + 1] = tostring(HOOK.handlers.apply('x'))
             out[#out + 1] = tostring(HOOK.handlers.notice('End', ''))
             return out",
        )
        .eval()
        .unwrap();
    assert_eq!(results[0], "handlers need apply()");
    assert!(results[1].starts_with("false apply failed: "));
    assert!(results[1].ends_with("boom"));
    assert_eq!(results[2], "false apply gave no answer");
    assert_eq!(results[3], "true");
    assert_eq!(results[4], "true", "a missing notice handler is fine");
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
             why({ version = 1, command = print, register = print })
             return out",
        )
        .eval()
        .unwrap();
    assert_eq!(
        reasons,
        [
            "no hook in this game",
            "tpf3mp_native is not a table",
            "the hook has no log()",
        ]
    );
}

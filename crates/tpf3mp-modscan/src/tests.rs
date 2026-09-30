//! The scan on small mods written for these tests, each shaped like a real
//! kind of Transport Fever 3 mod (docs/MODS.md names the real ones and what
//! the scan said of them). None copies a real mod's code.

use std::path::Path;

use super::*;

/// A mod folder with `files`, each `(path, text)`.
fn mod_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

fn kinds(report: &Report) -> Vec<Kind> {
    let mut kinds: Vec<Kind> = report.reasons.iter().map(|r| r.kind).collect();
    kinds.dedup();
    kinds
}

fn sharing(report: &Report) -> Vec<(Kind, String, Option<usize>)> {
    report
        .sharing()
        .map(|r| (r.kind, r.file.clone(), r.line))
        .collect()
}

const MANIFEST: &str = r#"{
  "modId": "test_mod_1", "revision": 3, "severityAdd": "None", "severityRemove": "None",
  "preRunScript": {"fileName": ""}, "runScript": {"fileName": ""}, "postRunScript": {"fileName": ""}
}"#;

const PLUGIN_RES: &str = r#"function data()
  return {
    type = "react-plugin ::GameBarInfoDisplayExtension",
    filePath = "test_mod_1::/gui/overlay/overlay.script@Overlay",
    priority = 10,
  }
end
"#;

/// A minimap or overlay: reads the engine, draws, keeps its settings in the
/// save, and edits a line through a command.
const OVERLAY_SCRIPT: &str = r#"
local react = ug_require "::/gui/main/react.lua"
-- api.cmd.sendCommand(x) in a comment is not a call; nor is "game.interface".
local M = {}
function M.Overlay()
  local started = os.clock()
  local lines = api.engine.system.lineSystem.getLines()
  api.gui.game.setGuiSaveData("test_mod_1", { zoom = 2 })
  local cmd = api.cmd.makeLineUpdateCmd(lines[1], api.type.Line.new())
  api.cmd.sendCommand(cmd, function(res, ok) end)
  return react.Component{}
end
return M
"#;

#[test]
fn a_gui_overlay_is_personal_and_says_what_it_does() {
    let dir = mod_with(&[
        ("mod.json", MANIFEST),
        ("_content.json", "{}"),
        ("_metadata/modinfo.json", "{}"),
        ("_metadata/0.png", "png"),
        ("README.md", "# overlay"),
        ("content/gui/overlay/overlay.res.lua", PLUGIN_RES),
        ("content/gui/overlay/overlay.script.lua", OVERLAY_SCRIPT),
        ("content/gui/overlay/overlay.css.lua", "return {}"),
        ("content/gui/overlay/icon@2x.tga", "tga"),
        ("content/gui/overlay/types.d.tl", "global record X end"),
    ]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Personal, "{report}");
    assert_eq!(report.id, "test_mod_1");
    assert_eq!(report.revision, Some(3));
    assert_eq!(
        kinds(&report),
        [Kind::Commands, Kind::Gui, Kind::SaveData, Kind::System]
    );
    let commands = report
        .reasons
        .iter()
        .find(|r| r.kind == Kind::Commands)
        .unwrap();
    assert_eq!(commands.detail, "makes makeLineUpdateCmd");
    assert_eq!(commands.line, Some(9), "the first command's line");
    assert_eq!(
        report.to_string().lines().next().unwrap(),
        "test_mod_1 (revision 3): personal"
    );
}

#[test]
fn a_game_script_makes_a_mod_shared_whatever_it_calls() {
    // Shaped like Auto Line Namer: a game script renaming lines on a timer,
    // and a rename scheme for the line manager's button.
    let dir = mod_with(&[
        ("mod.json", MANIFEST),
        (
            "content/namer/namer.gs.lua",
            "function data() return { updateScript = { fileName = \"namer.script@update\" } } end",
        ),
        (
            "content/namer/namer.script.lua",
            "local M = {}\nfunction M.update(p, state)\n  if os.time() > 0 then\n    api.cmd.sendCommand(api.cmd.makeEntitySetNameCmd(1, 'Bus 1'))\n  end\n  state:set({})\nend\nreturn M",
        ),
        (
            "content/namer/scheme.res.lua",
            "function data() return { type = \"rename_scheme\", name = _(\"Auto\") } end",
        ),
    ]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Shared, "{report}");
    assert_eq!(
        sharing(&report),
        [(Kind::GameScript, "content/namer/namer.gs.lua".into(), None)]
    );
    // The rename scheme alone is the GUI's.
    assert!(
        report
            .reasons
            .iter()
            .any(|r| r.kind == Kind::Gui && r.detail == "GUI resource rename_scheme")
    );
}

#[test]
fn run_scripts_and_modifiers_are_content() {
    // Shaped like Automatic Signal Spacing: a run script adding parameters
    // to the game's signals, and a game script placing more of them.
    let dir = mod_with(&[
        (
            "mod.json",
            r#"{"modId": "signals", "runScript": {"fileName": "signals::/mod.script@runFn"}}"#,
        ),
        (
            "content/mod.script.lua",
            "function data() return { runFn = function()\n addModifier(\"loadConstruction\", function(f, d) return d end)\nend } end",
        ),
        (
            "content/signals/signals.gs.lua",
            "function data() return {} end",
        ),
        ("make.sh", "cyan build"),
        ("src/signals.tl", "addModifier('x', nil)"),
    ]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Shared);
    assert_eq!(
        sharing(&report),
        [
            (Kind::RunScript, "mod.json".into(), None),
            (Kind::Modifier, "content/mod.script.lua".into(), Some(2)),
            (
                Kind::GameScript,
                "content/signals/signals.gs.lua".into(),
                None
            ),
        ]
    );
    // The build script and the Teal sources beside content/ are not loaded.
    let not_loaded: Vec<&str> = report
        .reasons
        .iter()
        .filter(|r| r.kind == Kind::NotLoaded)
        .map(|r| r.file.as_str())
        .collect();
    assert_eq!(not_loaded, ["make.sh", "src/signals.tl"]);
}

#[test]
fn a_cosmetic_flag_decides_nothing() {
    // Shaped like Timetables: "cosmetic": true, and a game script holding
    // vehicles at their stops.
    let dir = mod_with(&[
        (
            "mod.json",
            r#"{"modId": "timetables", "cosmetic": true, "runScript": {"fileName": ""}}"#,
        ),
        ("content/tt/tt.gs.lua", "function data() return {} end"),
        (
            "content/tt/tt_gs.script.tl",
            "local function hold(v: integer)\n  api.cmd.sendCommand(api.cmd.makeVehicleSetManualDepartureCmd(v, true))\nend",
        ),
    ]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Shared);
    assert!(report.reasons.iter().any(|r| r.kind == Kind::CosmeticFlag));
    assert_eq!(sharing(&report).len(), 1);
}

#[test]
fn world_content_is_shared() {
    let dir = mod_with(&[
        ("mod.json", MANIFEST),
        (
            "content/vehicle/bus/new_bus.mdl",
            "function data() return {} end",
        ),
        (
            "content/construction/depot.con",
            "function data() return {} end",
        ),
        ("content/names/towns.lua", "return { 'Springfield' }"),
        ("content/vehicle/bus/new_bus.tga", "tga"),
    ]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Shared);
    let files: Vec<String> = sharing(&report).into_iter().map(|(_, f, _)| f).collect();
    assert_eq!(
        files,
        [
            "content/construction/depot.con",
            "content/names/towns.lua",
            "content/vehicle/bus/new_bus.mdl"
        ]
    );
}

#[test]
fn what_the_scan_cannot_read_or_the_guard_cannot_see_is_shared() {
    let dir = mod_with(&[
        ("mod.json", MANIFEST),
        (
            "content/gui/x.script.lua",
            "local send = api.cmd.sendCommand\n\
             local f = _G[\"lo\" .. \"ad\"]\n\
             local g = load(\"return 1\")\n\
             api.res.constructionRep.setAsTable(1, {})\n\
             api.res.modelRep.add(\"m.mdl\", {}, true)\n\
             game.interface.buildConstruction()\n\
             debug.setupvalue(f, 1, nil)\n\
             setmetatable(_G, {})\n\
             api.cmd.sendCommand = function() end\n\
             local text = 'a' .. load_more\n\
             board.load(1)\n",
        ),
        ("content/gui/helper.dll", "MZ"),
        (
            "content/gui/y.res.lua",
            "function data() return { type = \"construction\" } end",
        ),
    ]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Shared);
    assert_eq!(
        sharing(&report)
            .into_iter()
            .map(|(k, _, l)| (k, l))
            .collect::<Vec<_>>(),
        [
            (Kind::UnknownResource, Some(1)),
            (Kind::ResourceWrite, Some(4)),
            (Kind::ResourceWrite, Some(5)),
            (Kind::GameInterface, Some(6)),
            (Kind::Dynamic, Some(2)),
            (Kind::Dynamic, Some(3)),
            (Kind::Dynamic, Some(7)),
            (Kind::Dynamic, Some(8)),
            (Kind::CommandBypass, Some(1)),
            (Kind::CommandBypass, Some(9)),
            (Kind::UnknownFile, None),
        ]
    );
}

#[test]
fn no_manifest_or_a_broken_one_is_shared() {
    let dir = mod_with(&[("content/gui/x.script.lua", "return {}")]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Shared);
    assert_eq!(sharing(&report)[0].0, Kind::Manifest);

    let dir = mod_with(&[("mod.json", "{ not json")]);
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Shared);
    assert!(report.id.starts_with(".tmp") || !report.id.is_empty());
}

#[test]
fn a_mod_folder_that_does_not_exist_is_shared() {
    let report = scan(Path::new("this folder is not there"));
    assert_eq!(report.class, Class::Shared);
    let kinds: Vec<Kind> = report.sharing().map(|r| r.kind).collect();
    assert_eq!(kinds, [Kind::Manifest, Kind::Unreadable]);
}

#[test]
fn text_that_is_not_utf8_is_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("mod.json"), MANIFEST).unwrap();
    std::fs::create_dir_all(dir.path().join("content")).unwrap();
    std::fs::write(dir.path().join("content/x.script.lua"), [0xff, 0xfe, 0x00]).unwrap();
    let report = scan(dir.path());
    assert_eq!(report.class, Class::Shared);
    assert_eq!(sharing(&report)[0].0, Kind::Unreadable);
}

#[test]
fn reasons_serialize_for_other_tools() {
    let dir = mod_with(&[("mod.json", MANIFEST), ("content/a.gs.lua", "")]);
    let json = serde_json::to_value(scan(dir.path())).unwrap();
    assert_eq!(json["class"], "shared");
    assert_eq!(json["reasons"][0]["kind"], "game_script");
    assert_eq!(json["reasons"][0]["file"], "content/a.gs.lua");
}

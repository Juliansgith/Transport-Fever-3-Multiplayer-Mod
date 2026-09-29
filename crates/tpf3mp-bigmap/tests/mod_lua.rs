//! The big-map mod (`mod/tpf3mp_bigmap_1`): laid out as TF3 mods are, its
//! ladder generated from the example settings and not edited since, and
//! its menu logic run in plain Lua.

#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use mlua::{Function, Lua, MultiValue, Table, Value};
use tpf3mp_bigmap::{config::Config, mod_data, world::WorldModel};

fn crate_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn mod_dir() -> PathBuf {
    crate_dir().join("../../mod/tpf3mp_bigmap_1")
}

fn script(name: &str) -> String {
    let path = mod_dir().join("content/scripts/tpf3mp_bigmap").join(name);
    // A Windows checkout may have turned the newlines into CRLF.
    std::fs::read_to_string(&path)
        .unwrap()
        .replace("\r\n", "\n")
}

fn example() -> Config {
    let text = std::fs::read_to_string(crate_dir().join("tpf3mp_bigmap.example.toml")).unwrap();
    let (config, notes) = Config::from_toml(&text).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    config
}

#[test]
fn the_ladder_is_generated_from_the_example_settings() {
    let generated = mod_data::lua(&WorldModel::TPF2_BUILD_35924, &example());
    assert_eq!(
        script("ladder.lua"),
        generated,
        "regenerate it: cargo run -p tpf3mp-bigmap -- --config crates/tpf3mp-bigmap/tpf3mp_bigmap.example.toml lua > mod/tpf3mp_bigmap_1/content/scripts/tpf3mp_bigmap/ladder.lua"
    );
}

#[test]
fn the_content_list_names_every_file() {
    let text = std::fs::read_to_string(mod_dir().join("_content.json")).unwrap();
    let content: serde_json::Value = serde_json::from_str(&text).unwrap();
    let listed: BTreeSet<String> = content["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file.as_str().unwrap().to_owned())
        .collect();
    let root = mod_dir().join("content");
    let mut found = BTreeSet::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(&root).unwrap();
                found.insert(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    assert_eq!(listed, found);
    let mod_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(mod_dir().join("mod.json")).unwrap())
            .unwrap();
    assert_eq!(mod_json["modId"], "tpf3mp_bigmap_1");
}

/// The menu and the ladder, loaded as the mod loads them.
fn menu() -> (Lua, Table, Table) {
    let lua = Lua::new();
    let ladder: Table = lua
        .load(script("ladder.lua"))
        .set_name("ladder.lua")
        .eval()
        .unwrap();
    let menu: Table = lua
        .load(script("menu.lua"))
        .set_name("menu.lua")
        .eval()
        .unwrap();
    (lua, menu, ladder)
}

/// What `menu.tiles` answered.
#[derive(Debug, PartialEq)]
enum Answer {
    /// A stock row: the game answers.
    Game,
    Tiles(i64, i64),
    Refused(String),
}

/// A Lua number as an integer, whether the Lua keeps integers or not.
fn int(value: &Value) -> i64 {
    match value {
        Value::Integer(n) => *n,
        #[allow(clippy::cast_possible_truncation)]
        Value::Number(n) => *n as i64,
        other => panic!("not a number: {other:?}"),
    }
}

/// `menu.tiles` for a pick.
fn answer(menu: &Table, ladder: &Table, stock_rows: u32, size: u32, ratio: u32) -> Answer {
    match &tiles(menu, ladder, stock_rows, size, ratio)[..] {
        [Value::Nil] | [] => Answer::Game,
        [Value::Boolean(false), Value::String(reason)] => {
            Answer::Refused(reason.to_str().unwrap().to_string())
        }
        [x, y] => Answer::Tiles(int(x), int(y)),
        other => panic!("{other:?}"),
    }
}

/// `menu.tiles` for a pick, as its return values.
fn tiles(menu: &Table, ladder: &Table, stock_rows: u32, size: u32, ratio: u32) -> Vec<Value> {
    let tiles: Function = menu.get("tiles").unwrap();
    let values: MultiValue = tiles
        .call((ladder.clone(), stock_rows, size, ratio))
        .unwrap();
    values.into_vec()
}

#[test]
fn the_menu_appends_the_rows_after_the_games_own() {
    let (_lua, menu, ladder) = menu();
    let labels: Function = menu.get("labels").unwrap();
    let labels: Vec<String> = labels.call(ladder.clone()).unwrap();
    assert_eq!(labels.first().map(String::as_str), Some("32 x 32 km"));
    assert_eq!(labels.last().map(String::as_str), Some("128 x 128 km"));

    // Seven stock rows (0..6): the game answers them itself.
    assert_eq!(answer(&menu, &ladder, 7, 3, 0), Answer::Game);
    // Row 7 is the first added one: 32 km at 1:1, then at 1:2.
    assert_eq!(answer(&menu, &ladder, 7, 7, 0), Answer::Tiles(128, 128));
    assert_eq!(answer(&menu, &ladder, 7, 7, 1), Answer::Tiles(180, 90));
    // The last added row, and past it.
    assert_eq!(answer(&menu, &ladder, 7, 15, 0), Answer::Tiles(512, 512));
    assert!(matches!(
        answer(&menu, &ladder, 7, 16, 0),
        Answer::Refused(_)
    ));
    assert!(matches!(
        answer(&menu, &ladder, 7, 7, 5),
        Answer::Refused(_)
    ));
}

#[test]
fn a_shape_the_settings_cannot_build_is_refused_not_guessed() {
    // With every feature off, the 40 km row keeps only its first two shapes.
    let lua = Lua::new();
    let stock = mod_data::lua(&WorldModel::TPF2_BUILD_35924, &Config::default());
    let ladder: Table = lua.load(stock).eval().unwrap();
    let menu: Table = lua.load(script("menu.lua")).eval().unwrap();
    assert_eq!(answer(&menu, &ladder, 7, 8, 1), Answer::Tiles(228, 114));
    match answer(&menu, &ladder, 7, 8, 2) {
        Answer::Refused(reason) => assert!(reason.contains("cannot be built"), "{reason}"),
        other => panic!("{other:?}"),
    }
}

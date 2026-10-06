//! The big-map mod (`mod/tpf3mp_bigmap_1`): laid out as TF3 mods are, its
//! ladder generated from the stage 1 settings and not edited since, its
//! New Game page copy exactly the game's file plus marked blocks, and its
//! menu logic run in plain Lua.

#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use mlua::{Function, Lua, MultiValue, Table, Value};
use sha2::{Digest, Sha256};
use tpf3mp_bigmap::{
    config::Config,
    ladder::shapes,
    mod_data, page,
    stock::{self, TF3_BUILD_40408},
    world::WorldModel,
};

const TF3: WorldModel = WorldModel::TF3_BUILD_40408;

fn crate_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn mod_dir() -> PathBuf {
    crate_dir().join("../../mod/tpf3mp_bigmap_1")
}

fn content(path: &str) -> String {
    let path = mod_dir().join("content").join(path);
    // A Windows checkout may have turned the newlines into CRLF.
    std::fs::read_to_string(&path)
        .unwrap()
        .replace("\r\n", "\n")
}

fn script(name: &str) -> String {
    content(&format!("scripts/tpf3mp_bigmap/{name}"))
}

fn stage1() -> Config {
    let text = std::fs::read_to_string(crate_dir().join("tpf3mp_bigmap.stage1.toml")).unwrap();
    let (config, notes) = Config::from_toml(&text).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    config
}

#[test]
fn the_ladder_is_generated_from_the_stage1_settings() {
    let generated = mod_data::lua(&TF3, &stage1());
    assert_eq!(
        script("ladder.lua"),
        generated,
        "regenerate it: cargo run -p tpf3mp-bigmap -- --config crates/tpf3mp-bigmap/tpf3mp_bigmap.stage1.toml lua > mod/tpf3mp_bigmap_1/content/scripts/tpf3mp_bigmap/ladder.lua"
    );
}

#[test]
fn every_stage1_size_is_bigger_than_the_games_and_inside_its_walls() {
    let config = stage1();
    let biggest_stock = TF3_BUILD_40408
        .iter()
        .flat_map(|size| size.shapes)
        .map(|(x, y)| x * y)
        .max()
        .unwrap();
    let rows = config.rows(&TF3);
    assert_eq!(
        rows.iter().map(|row| row.tiles).collect::<Vec<_>>(),
        [128, 144, 160, 176]
    );
    for row in rows {
        assert!(row.tiles * row.tiles > biggest_stock, "{}", row.label);
        for (x, y) in shapes(&row, config.sizes.max_tiles) {
            // No native patch: the same walls every stock size keeps to.
            assert!(
                stock::inside_stock_walls(&TF3, x, y),
                "{} {x}x{y}",
                row.label
            );
            assert!(
                tpf3mp_bigmap::ceilings::check(&TF3, &config, x, y).buildable(),
                "{} {x}x{y}",
                row.label
            );
            assert!(
                x * y > 12_500,
                "{} {x}x{y}: past Gigantomaniac 1:5",
                row.label
            );
        }
    }
}

#[test]
fn the_page_copy_is_the_games_file_plus_marked_blocks() {
    let copy = content(page::COPY_PATH);
    let game = page::original(&copy).unwrap();
    let hash: String = Sha256::digest(game.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        hash,
        page::GAME_PAGE_SHA256,
        "the copy holds a change outside its marked blocks, or was made from another build's file"
    );
    assert_eq!(
        page::build(&game).unwrap(),
        copy,
        "regenerate it: cargo run -p tpf3mp-bigmap -- page <the game's gui/menu/new_game_or_map_settings_page.tl>"
    );
    // The copy reads the mod's scripts by the paths they have here.
    for path in [
        "tpf3mp_bigmap_1::/scripts/tpf3mp_bigmap/menu.lua",
        "tpf3mp_bigmap_1::/scripts/tpf3mp_bigmap/ladder.lua",
    ] {
        assert!(copy.contains(path), "{path}");
        let local = path.trim_start_matches("tpf3mp_bigmap_1::/");
        assert!(mod_dir().join("content").join(local).is_file(), "{local}");
    }
    // No game path is left relative or leading-slash: those would resolve
    // in the mod's folder. (The game's own lines a block replaced stay as
    // comments.)
    for line in copy
        .lines()
        .filter(|line| !line.trim_start().starts_with("--") && line.contains("ug_require \""))
    {
        assert!(
            line.contains("ug_require \"::/") || line.contains("ug_require \"tpf3mp_bigmap_1::/"),
            "{line}"
        );
    }
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

/// The menu and the ladder, loaded as the page loads them.
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

fn call(menu: &Table, name: &str, args: impl mlua::IntoLuaMulti) -> Vec<Value> {
    let function: Function = menu.get(name).unwrap();
    let values: MultiValue = function.call(args).unwrap();
    values.into_vec()
}

/// `menu.tiles` for a pick.
fn answer(menu: &Table, ladder: &Table, stock_rows: u32, size: u32, ratio: u32) -> Answer {
    match &call(menu, "tiles", (ladder.clone(), stock_rows, size, ratio))[..] {
        [Value::Nil] | [] => Answer::Game,
        [Value::Boolean(false), Value::String(reason)] => {
            Answer::Refused(reason.to_str().unwrap().to_string())
        }
        [x, y] => Answer::Tiles(int(x), int(y)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_menu_appends_the_rows_after_the_games_own() {
    let (_lua, menu, ladder) = menu();
    let labels: Function = menu.get("labels").unwrap();
    let labels: Vec<String> = labels.call(ladder.clone()).unwrap();
    assert_eq!(
        labels,
        ["Big 32.8 km", "Big 36.9 km", "Big 41.0 km", "Big 45.1 km"]
    );

    // Eight stock rows with experimental sizes (0..7): the game answers them.
    assert_eq!(answer(&menu, &ladder, 8, 7, 0), Answer::Game);
    // Row 8 is the first added one: 32.8 km at 1:1, then at 1:2.
    assert_eq!(answer(&menu, &ladder, 8, 8, 0), Answer::Tiles(128, 128));
    assert_eq!(answer(&menu, &ladder, 8, 8, 1), Answer::Tiles(180, 90));
    // The last added row, and past it.
    assert_eq!(answer(&menu, &ladder, 8, 11, 4), Answer::Tiles(250, 78));
    assert!(matches!(
        answer(&menu, &ladder, 8, 12, 0),
        Answer::Refused(_)
    ));
    assert!(matches!(
        answer(&menu, &ladder, 8, 8, 5),
        Answer::Refused(_)
    ));
}

#[test]
fn the_game_gets_the_short_side_first() {
    let (_lua, menu, ladder) = menu();
    let tiles = call(&menu, "gameTiles", (ladder.clone(), 1, 1));
    assert_eq!(tiles.iter().map(int).collect::<Vec<_>>(), [90, 180]);
    let tiles = call(&menu, "gameTiles", (ladder.clone(), 4, 0));
    assert_eq!(tiles.iter().map(int).collect::<Vec<_>>(), [176, 176]);
    // No added row picked: nothing, never a guess.
    assert!(matches!(
        call(&menu, "gameTiles", (ladder, 0, 0))[..],
        [Value::Nil, ..]
    ));
}

/// `menu.offered` on a machine with `ram_mb`: the labels offered, and the
/// notes.
fn offered(menu: &Table, ladder: &Table, ram_mb: Option<u32>) -> (Vec<String>, Vec<String>) {
    let values = call(menu, "offered", (ladder.clone(), ram_mb));
    let [Value::Table(rows), Value::Table(notes)] = &values[..] else {
        panic!("{values:?}");
    };
    let labels = rows
        .sequence_values::<Table>()
        .map(|row| row.unwrap().get::<String>("label").unwrap())
        .collect();
    let notes = notes
        .sequence_values::<String>()
        .map(Result::unwrap)
        .collect();
    (labels, notes)
}

#[test]
fn a_size_this_machine_cannot_generate_is_hidden_with_the_reason() {
    let (_lua, menu, ladder) = menu();
    // 32 GB: every row (the largest needs 26,608 MB).
    let (rows, notes) = offered(&menu, &ladder, Some(32 * 1024));
    assert_eq!(rows.len(), 4);
    assert!(notes.is_empty(), "{notes:?}");
    // 18,000 MB: only the first (16,836 MB; the second needs 19,907).
    let (rows, notes) = offered(&menu, &ladder, Some(18_000));
    assert_eq!(rows, ["Big 32.8 km"]);
    assert_eq!(notes.len(), 3);
    assert_eq!(
        notes[0],
        "Big 36.9 km is hidden: generating it needs about 20 GB of memory, and this computer has 17 GB."
    );
    // 16 GB: none.
    let (rows, notes) = offered(&menu, &ladder, Some(16 * 1024));
    assert!(rows.is_empty());
    assert_eq!(notes.len(), 4);
    // Memory not known: none, and one line saying so.
    let (rows, notes) = offered(&menu, &ladder, None);
    assert!(rows.is_empty());
    assert_eq!(
        notes,
        ["Bigger sizes are hidden: this computer's memory is not known."]
    );
}

#[test]
fn the_memory_comes_from_the_hook() {
    let (lua, menu, _ladder) = menu();
    assert!(matches!(call(&menu, "ramMb", ())[..], [Value::Nil]));
    lua.load("resolveutil = { __tpf3mp_ram_mb = 32768 }")
        .exec()
        .unwrap();
    assert_eq!(int(&call(&menu, "ramMb", ())[0]), 32_768);
    lua.load("resolveutil = { __tpf3mp_ram_mb = 'lots' }")
        .exec()
        .unwrap();
    assert!(matches!(call(&menu, "ramMb", ())[..], [Value::Nil]));
}

#[test]
fn the_dropdown_keeps_map_size_a_value_the_game_knows() {
    let (lua, menu, ladder) = menu();
    let numbers: Table = lua.load("{ 2, 3, 4, 5 }").eval().unwrap();
    let none: Table = lua.create_table().unwrap();
    let pick = |index: i64, count: i64, numbers: &Table| -> (i64, i64) {
        let values = call(&menu, "choose", (index, count, numbers.clone()));
        (int(&values[0]), int(&values[1]))
    };
    // Experimental sizes (eight rows, no numbers): row 3 is itself, row 9
    // is the first added size over Gigantomaniac's 8.
    assert_eq!(pick(3, 8, &none), (3, 0));
    assert_eq!(pick(9, 8, &none), (8, 1));
    assert_eq!(pick(12, 8, &none), (8, 4));
    // Desktop sizes (four rows, numbers 2 to 5): the largest is 5.
    assert_eq!(pick(2, 4, &numbers), (3, 0));
    assert_eq!(pick(6, 4, &numbers), (5, 2));

    let index = |value: i64, pick: i64, count: i64, numbers: &Table| -> i64 {
        int(&call(&menu, "indexOf", (value, pick, count, numbers.clone(), 4))[0])
    };
    assert_eq!(index(5, 0, 4, &numbers), 4);
    assert_eq!(index(5, 2, 4, &numbers), 6);
    assert_eq!(index(8, 1, 8, &none), 9);
    assert_eq!(
        index(12, 0, 8, &none),
        8,
        "a stray value clamps as the game's"
    );
    assert_eq!(index(8, 5, 8, &none), 8, "a pick past the rows is no pick");

    let values = call(
        &menu,
        "sizeValues",
        (
            lua.load("{ 'Tiny', 'Small' }").eval::<Table>().unwrap(),
            ladder,
        ),
    );
    let [Value::Table(values)] = &values[..] else {
        panic!("{values:?}");
    };
    let values: Vec<String> = values.sequence_values().map(Result::unwrap).collect();
    assert_eq!(
        values,
        [
            "Tiny",
            "Small",
            "Big 32.8 km",
            "Big 36.9 km",
            "Big 41.0 km",
            "Big 45.1 km"
        ]
    );
}

/// A native vector as the game hands its script parameters over: no table,
/// only a length and 1-based indexing.
struct Native<T>(Vec<T>);

impl<T: mlua::IntoLua + Clone + 'static> mlua::UserData for Native<T> {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(mlua::MetaMethod::Len, |_, this, ()| Ok(this.0.len()));
        methods.add_meta_method(mlua::MetaMethod::Index, |_, this, index: usize| {
            Ok(index.checked_sub(1).and_then(|i| this.0.get(i).cloned()))
        });
    }
}

#[test]
fn the_games_native_lists_are_read_like_tables() {
    let (lua, menu, ladder) = menu();
    let numbers = lua
        .create_userdata(Native(vec![2.0f64, 3.0, 4.0, 5.0]))
        .unwrap();
    let values = call(&menu, "choose", (6, 4, numbers.clone()));
    assert_eq!(values.iter().map(int).collect::<Vec<_>>(), [5, 2]);
    let values = call(&menu, "choose", (2, 4, numbers.clone()));
    assert_eq!(values.iter().map(int).collect::<Vec<_>>(), [3, 0]);
    let index = call(&menu, "indexOf", (4, 0, 4, numbers, 4));
    assert_eq!(int(&index[0]), 3);

    let stock = lua
        .create_userdata(Native(vec!["Small".to_owned(), "Medium".to_owned()]))
        .unwrap();
    let values = call(&menu, "sizeValues", (stock, ladder));
    let [Value::Table(values)] = &values[..] else {
        panic!("{values:?}");
    };
    assert_eq!(values.raw_len(), 6);
    assert_eq!(values.get::<String>(2).unwrap(), "Medium");
    assert_eq!(values.get::<String>(3).unwrap(), "Big 32.8 km");
}

#[test]
fn a_shape_the_settings_cannot_build_is_refused_not_guessed() {
    // TPF2's numbers with every feature off: the 40 km row keeps only its
    // first two shapes.
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

#[test]
fn the_density_sliders_get_one_level_per_ladder_row() {
    let (lua, menu, ladder) = menu();
    let stock = lua
        .create_sequence_from(["Sparse", "Scattered", "Medium", "Dense", "Packed"])
        .unwrap();
    let (values, numbers): (Table, Table) = menu
        .get::<Function>("densityChoices")
        .unwrap()
        .call((stock, ladder.clone()))
        .unwrap();
    let values: Vec<String> = values.sequence_values().map(Result::unwrap).collect();
    let numbers: Vec<i64> = numbers.sequence_values().map(Result::unwrap).collect();
    // Sparsest first; each level keeps its number.
    assert_eq!(
        values,
        [
            "Gigantomaniac count at 45.1 km",
            "Gigantomaniac count at 41.0 km",
            "Gigantomaniac count at 36.9 km",
            "Gigantomaniac count at 32.8 km",
            "Sparse",
            "Scattered",
            "Medium",
            "Dense",
            "Packed",
        ]
    );
    assert_eq!(numbers, [9, 8, 7, 6, 1, 2, 3, 4, 5]);
    // Each level gives its row's square Gigantomaniac 1:1's area times the
    // Medium density: the counts stay Gigantomaniac's.
    let medium = lua
        .create_function(|_, (_param, level): (String, i64)| Ok(if level == 3 { 0.4 } else { 1.0 }))
        .unwrap();
    let scale = menu.get::<Function>("densityScale").unwrap();
    for (level, tiles) in [(6, 128.0), (7, 144.0), (8, 160.0), (9, 176.0)] {
        let factor: f64 = scale
            .call((
                ladder.clone(),
                medium.clone(),
                "locations.towns.frequency",
                level,
            ))
            .unwrap();
        let towns_per_gigantomaniac = 0.2 * 0.4 * 112.0 * 112.0;
        let towns = 0.2 * factor * tiles * tiles;
        assert!(
            (towns / towns_per_gigantomaniac - 1.0).abs() < 1e-3,
            "level {level}: {towns} towns-units, Gigantomaniac {towns_per_gigantomaniac}"
        );
    }
    // The game's own levels, and anything not a density, stay the game's.
    for (param, level) in [
        ("locations.towns.frequency", 5),
        ("advancedOptions.cargoIncome", 6),
    ] {
        let answer: Value = scale
            .call((ladder.clone(), medium.clone(), param, level))
            .unwrap();
        assert!(answer.is_nil(), "{param} {level}");
    }
}

#[test]
fn no_string_in_the_copys_blocks_runs_past_its_line() {
    // Teal, like Lua, ends a "..." string at the line: one that runs on
    // makes the whole page fail to load (the hook then serves the game's).
    let copy = content(page::COPY_PATH);
    let mut inside = false;
    for (number, line) in copy.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("-- TPF3-MP begin:") {
            inside = true;
            continue;
        }
        if trimmed.starts_with("-- TPF3-MP was:") || trimmed.starts_with("-- TPF3-MP end") {
            inside = false;
            continue;
        }
        if !inside || trimmed.starts_with("--") {
            continue;
        }
        let quotes = line
            .replace("\\\\", "")
            .replace("\\\"", "")
            .matches('"')
            .count();
        assert!(quotes % 2 == 0, "line {}: {line}", number + 1);
    }
}

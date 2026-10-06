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
            "----",
            "---",
            "--",
            "-",
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

/// The ratios `menu.extraRatios` offers (1 is 1:6) and its note.
fn extra(
    menu: &Table,
    shapes: Value,
    peak_mb: f64,
    ram_mb: Option<u32>,
) -> (Vec<i64>, Option<String>) {
    let values = call(menu, "extraRatios", (shapes, peak_mb, ram_mb, "Huge"));
    let offered = match &values[0] {
        Value::Table(t) => t
            .sequence_values::<Value>()
            .map(|v| int(&v.unwrap()))
            .collect(),
        other => panic!("{other:?}"),
    };
    let note = match values.get(1) {
        Some(Value::String(s)) => Some(s.to_str().unwrap().to_string()),
        _ => None,
    };
    (offered, note)
}

#[test]
fn every_size_has_its_longer_ratios_or_false() {
    let (_lua, _menu, ladder) = menu();
    for row in ladder.clone().sequence_values::<Table>() {
        let shapes: Table = row.unwrap().get("shapes").unwrap();
        assert_eq!(shapes.raw_len(), 10, "1:1 to 1:10");
    }
    let stock: Table = ladder.get("stock").unwrap();
    for size in TF3_BUILD_40408 {
        let square = size.shapes[0].0;
        let entry: Table = stock.get(square).unwrap();
        let shapes: Table = entry.get("shapes").unwrap();
        assert_eq!(shapes.raw_len(), 5, "{}: 1:6 to 1:10", size.name);
        for (k, shape) in (6..).zip(shapes.sequence_values::<Value>()) {
            if let Value::Table(shape) = shape.unwrap() {
                let (x, y): (u32, u32) = (shape.get(1).unwrap(), shape.get(2).unwrap());
                // Stage 1: inside every wall stock TF3 keeps, and 1:k long.
                assert!(stock::inside_stock_walls(&TF3, x, y), "{} 1:{k}", size.name);
                assert_eq!(x, y * k, "{} 1:{k}", size.name);
            }
        }
    }
    // Megalomaniac reaches 1:6 (40 x 240); Gigantomaniac, already 250 long at
    // 1:5, reaches nothing longer at stage 1.
    let mega: Table = stock.get(96).unwrap();
    let mega: Table = mega.get("shapes").unwrap();
    assert!(matches!(mega.get::<Value>(1).unwrap(), Value::Table(_)));
    assert!(matches!(
        mega.get::<Value>(2).unwrap(),
        Value::Boolean(false)
    ));
    let gig: Table = stock.get(112).unwrap();
    let gig: Table = gig.get("shapes").unwrap();
    assert!(
        gig.sequence_values::<Value>()
            .all(|v| matches!(v.unwrap(), Value::Boolean(false)))
    );
}

#[test]
fn a_longer_ratio_is_offered_only_where_it_can_be_built_and_fits() {
    let (_lua, menu, ladder) = menu();
    let huge = call(&menu, "stockShapes", (ladder.clone(), 80));
    let (shapes, peak) = (
        huge[0].clone(),
        match &huge[1] {
            v @ (Value::Integer(_) | Value::Number(_)) => int(v) as f64,
            other => panic!("{other:?}"),
        },
    );
    // Huge (80 tiles): 1:6 to 1:9 inside the walls, 1:10 (260 x 26) past
    // stage 1's 250.
    let (offered, note) = extra(&menu, shapes.clone(), peak, Some(32 * 1024));
    assert_eq!(offered, [1, 2, 3, 4]);
    assert_eq!(
        note.as_deref(),
        Some("1:10 of Huge is past what these settings can build.")
    );
    // Not enough memory, or not known: none, and why.
    let (offered, note) = extra(&menu, shapes.clone(), peak, Some(4 * 1024));
    assert!(offered.is_empty());
    assert!(note.unwrap().contains("needs about 10 GB"));
    let (offered, note) = extra(&menu, shapes.clone(), peak, None);
    assert!(offered.is_empty());
    assert!(note.unwrap().contains("memory is not known"));
    // The tiles of 1:8, the short side first.
    let tiles = call(&menu, "extraTiles", (shapes.clone(), 3));
    assert_eq!(tiles.iter().map(int).collect::<Vec<_>>(), [28, 224]);
    assert!(matches!(
        call(&menu, "extraTiles", (shapes, 5))[..],
        [Value::Nil, ..]
    ));
    // An added row at stage 1 has no longer ratio: none, with the reason.
    let row: Table = ladder.get(1).unwrap();
    let shapes: Value = row.get("shapes").unwrap();
    let (offered, note) = extra(&menu, shapes, 16_836.0, Some(32 * 1024));
    assert!(offered.is_empty());
    assert!(
        note.unwrap()
            .starts_with("1:6, 1:7, 1:8, 1:9, 1:10 of Huge are past")
    );
    // No size known: nothing offered, nothing said.
    let (offered, note) = extra(&menu, Value::Nil, 0.0, Some(32 * 1024));
    assert!(offered.is_empty() && note.is_none());
}

#[test]
fn the_ratio_dropdown_keeps_map_format_a_value_the_game_knows() {
    let (lua, menu, _ladder) = menu();
    let numbers: Table = lua.create_table().unwrap();
    let offered: Table = lua.load("{ 1, 2, 4 }").eval().unwrap();
    let values: Table = lua
        .load("{ '1:1', '1:2', '1:3', '1:4', '1:5' }")
        .eval()
        .unwrap();
    let labels: Vec<String> = {
        let f: Function = menu.get("formatValues").unwrap();
        f.call((values, offered.clone())).unwrap()
    };
    assert_eq!(
        labels,
        ["1:1", "1:2", "1:3", "1:4", "1:5", "1:6", "1:7", "1:9"]
    );
    let choose = |index: i64| -> (i64, i64) {
        let v = call(
            &menu,
            "formatChoose",
            (index, 5, numbers.clone(), offered.clone()),
        );
        (int(&v[0]), int(&v[1]))
    };
    // The game's own ratios set themselves; an added one sets 1:5 and its pick.
    assert_eq!(choose(2), (2, 0));
    assert_eq!(choose(6), (5, 1));
    assert_eq!(choose(8), (5, 4), "the third added entry is 1:9");
    assert_eq!(choose(9), (5, 0), "past the list: the game's last, no pick");
    let index_of = |value: i64, pick: i64| -> i64 {
        int(&call(
            &menu,
            "formatIndexOf",
            (value, pick, 5, numbers.clone(), offered.clone()),
        )[0])
    };
    assert_eq!(index_of(3, 0), 3);
    assert_eq!(index_of(5, 4), 8);
    assert_eq!(
        index_of(5, 3),
        5,
        "a pick no longer offered shows the game's 1:5"
    );
}

#[test]
fn with_the_memory_check_off_every_row_is_offered() {
    let (lua, menu, ladder) = menu();
    lua.load("resolveutil = { __tpf3mp_ram_mb = 8192, __tpf3mp_memory_gate = false }")
        .exec()
        .unwrap();
    let ram: f64 = menu.get::<Function>("ramMb").unwrap().call(()).unwrap();
    assert!(ram.is_infinite());
    let (rows, notes): (Table, Table) = menu
        .get::<Function>("offered")
        .unwrap()
        .call((ladder.clone(), ram))
        .unwrap();
    assert_eq!(rows.raw_len(), ladder.raw_len());
    let notes: Vec<String> = notes.sequence_values().map(Result::unwrap).collect();
    assert_eq!(
        notes,
        ["Memory check off: a size may not fit this computer's memory."]
    );
    // On, an 8 GB machine gets none of the big rows.
    lua.load("resolveutil.__tpf3mp_memory_gate = true")
        .exec()
        .unwrap();
    let ram: f64 = menu.get::<Function>("ramMb").unwrap().call(()).unwrap();
    assert!((ram - 8192.0).abs() < 0.5);
}

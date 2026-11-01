//! The Transport Fever 3 builds the repository's profiles know (`profiles/`,
//! each folder's `hooks.toml` `[build]` identity) and the one this release's
//! hook runs in (`profiles/native-build.txt`), for the launcher's check of
//! the game it starts (`check_build`).

use std::{fmt::Write as _, fs, path::Path};

/// A `[build]` section's `sha256` and `size`, read line by line: the
/// profiles keep one `key = value` a line.
fn identity(hooks: &str) -> Option<(String, u64)> {
    let mut in_build = false;
    let (mut sha256, mut size) = (None, None);
    for line in hooks.lines().map(str::trim) {
        if line.starts_with('[') {
            in_build = line == "[build]";
            continue;
        }
        if !in_build {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.split('#').next().unwrap_or("").trim();
        match key.trim() {
            "sha256" => sha256 = Some(value.trim_matches('"').to_ascii_lowercase()),
            "size" => size = value.replace('_', "").parse().ok(),
            _ => {}
        }
    }
    Some((sha256?, size?))
}

fn main() {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir");
    let profiles = Path::new(&manifest).join("../../profiles");
    println!("cargo:rerun-if-changed={}", profiles.display());
    let selected_file = profiles.join("native-build.txt");
    println!("cargo:rerun-if-changed={}", selected_file.display());
    let selected = fs::read_to_string(&selected_file)
        .expect("profiles/native-build.txt names the supported build")
        .trim()
        .to_owned();
    let mut known: Vec<(u32, String, String, u64)> = Vec::new();
    for entry in fs::read_dir(&profiles).expect("the profiles folder") {
        let entry = entry.expect("a profiles entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(number) = name
            .strip_prefix("tf3_build")
            .and_then(|rest| rest.strip_suffix("_steam_windows"))
            .and_then(|number| number.parse::<u32>().ok())
        else {
            continue;
        };
        let hooks = entry.path().join("hooks.toml");
        println!("cargo:rerun-if-changed={}", hooks.display());
        let text = fs::read_to_string(&hooks).expect("a profile's hooks.toml");
        let (sha256, size) = identity(&text).expect("a profile's [build] sha256 and size");
        known.push((number, name, sha256, size));
    }
    known.sort();
    let supported = known
        .iter()
        .find(|(_, name, ..)| *name == selected)
        .expect("native-build.txt names a profile folder");
    let build = |(number, _, sha256, size): &(u32, String, String, u64)| {
        format!("GameBuild {{ number: {number}, sha256: \"{sha256}\", size: {size} }}")
    };
    let mut out = String::new();
    let _ = writeln!(out, "/// The game build this release's hook runs in.");
    let _ = writeln!(
        out,
        "pub const SUPPORTED: GameBuild = {};",
        build(supported)
    );
    let _ = writeln!(out, "/// Every build a profile knows, oldest first.");
    let _ = writeln!(out, "pub const KNOWN: &[GameBuild] = &[");
    for entry in &known {
        let _ = writeln!(out, "    {},", build(entry));
    }
    let _ = writeln!(out, "];");
    let dest = Path::new(&std::env::var_os("OUT_DIR").expect("output dir")).join("builds.rs");
    fs::write(dest, out).expect("write builds.rs");
}

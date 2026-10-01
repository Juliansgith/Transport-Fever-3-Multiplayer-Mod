//! A fingerprint of TPF3-MP's own mod (`tpf3mp_1`) as this player's game
//! will load it, so that a room tells two copies of the same revision apart
//! when their files differ (docs/MODS.md, "TPF3-MP's own mod").
//!
//! A player's game once loaded an old copy of the mod (a Sandboxie box's
//! own copy of it, 2026-10-01); its revision was the same, the room did not
//! notice, and a road built differently in that game. The room compared the
//! action schema, which lives in the agent, never the Lua that applies the
//! actions.
//!
//! Where it is read: the folder the launcher finds for `tpf3mp_1`, first in
//! the game's order of mod folders (`tpf3mp_modscan::roots`, the same folder
//! `crate::portraits` writes to). The launcher runs beside the game, as the
//! same user and in the same Sandboxie box (a game it starts is started in
//! its box), so it reads the files through the same view as the game: a
//! box's own copy shadows the real one for both. The game itself names a
//! mod's files only as `tpf3mp_1::/...` and says nowhere which folder it
//! loaded them from, so the hook could only repeat the same search; and the
//! room checks content in the lobby, before the game may even run.
//!
//! What counts: what the game loads from a mod, its `mod.json` and
//! everything under `content/` (outside `content/` the game loads nothing),
//! and the mod's list of its files, `_content.json`. Left out is what the
//! launcher writes into the installed mod by itself: the campaign's
//! portraits (`content/gui/tpf3mp/portraits/`, from each player's own
//! install, `crate::portraits`) and their lines in `_content.json`.
//!
//! The fingerprint is a SHA-256 over the files' paths, sorted, each with its
//! bytes; the room compares its first 16 hex digits, carried in the mod's
//! version in the content manifest (`1+0123456789abcdef`).

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use ring::{
    digest::{Context, SHA256},
    rand::{SecureRandom, SystemRandom},
};

/// Keeps the fingerprint apart from every other SHA-256, and names its
/// format.
const DOMAIN: &[u8] = b"tpf3mp own mod 1\0";
/// The mod's list of its files.
const CONTENT_LIST: &str = "_content.json";
/// What the launcher writes into the mod, under `content/`.
const GENERATED: &str = crate::portraits::FOLDER;
/// Hex digits of the fingerprint a manifest carries.
pub const SHOWN_DIGITS: usize = 16;
/// Most files counted: far more than the mod has.
const MAX_FILES: usize = 20_000;
/// Most bytes read: far more than the mod has.
const MAX_BYTES: u64 = 512 << 20;
/// Deepest folder followed under `content/`.
const MAX_DEPTH: usize = 24;

/// The fingerprint of the mod in `dir`: the same for the same files the
/// game loads, whatever else the folder holds.
pub fn fingerprint(dir: &Path) -> io::Result<[u8; 32]> {
    let mut files = Vec::new();
    for top in ["mod.json", "mod.lua", CONTENT_LIST] {
        let path = dir.join(top);
        if path.is_file() {
            files.push((top.to_owned(), path));
        }
    }
    if !files
        .iter()
        .any(|(name, _)| name == "mod.json" || name == "mod.lua")
    {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} has no mod.json", dir.display()),
        ));
    }
    let content = dir.join("content");
    if content.is_dir() {
        walk(&content, "content", 0, &mut files)?;
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut digest = Context::new(&SHA256);
    digest.update(DOMAIN);
    let mut read = 0u64;
    for (name, path) in &files {
        let mut bytes = fs::read(path)?;
        read = read.saturating_add(bytes.len() as u64);
        if read > MAX_BYTES {
            return Err(io::Error::other(format!(
                "{} holds more than {MAX_BYTES} bytes",
                dir.display()
            )));
        }
        if name == CONTENT_LIST {
            bytes = without_generated(&bytes);
        }
        digest.update(&(name.len() as u64).to_le_bytes());
        digest.update(name.as_bytes());
        digest.update(&(bytes.len() as u64).to_le_bytes());
        digest.update(&bytes);
    }
    Ok(digest.finish().as_ref().try_into().unwrap_or([0; 32]))
}

/// Every file under `dir`, named `prefix/...` with `/` between folders,
/// less the generated folder and what the system leaves in folders.
fn walk(
    dir: &Path,
    prefix: &str,
    depth: usize,
    files: &mut Vec<(String, PathBuf)>,
) -> io::Result<()> {
    if depth > MAX_DEPTH {
        return Err(io::Error::other(format!(
            "{} is nested too deep",
            dir.display()
        )));
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            return Err(io::Error::other(format!(
                "{} has a file whose name is not Unicode",
                dir.display()
            )));
        };
        if ignored(&name) {
            continue;
        }
        let relative = format!("{prefix}/{name}");
        if relative == format!("content/{GENERATED}") {
            continue;
        }
        let path = entry.path();
        // Links followed, as the game follows them.
        let metadata = fs::metadata(&path)?;
        if metadata.is_dir() {
            walk(&path, &relative, depth + 1, files)?;
        } else {
            files.push((relative, path));
            if files.len() > MAX_FILES {
                return Err(io::Error::other(format!(
                    "{} holds more than {MAX_FILES} files",
                    dir.display()
                )));
            }
        }
    }
    Ok(())
}

/// What the system, not the mod, leaves in a folder.
fn ignored(name: &str) -> bool {
    name.starts_with('.')
        || name.eq_ignore_ascii_case("desktop.ini")
        || name.eq_ignore_ascii_case("thumbs.db")
}

/// `_content.json` without the generated files' lines, in a form that does
/// not depend on how it was written: the launcher rewrites it when it adds
/// the portraits. A list that does not read counts as its bytes.
fn without_generated(bytes: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    let Ok(mut list) =
        serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}'))
    else {
        return bytes.to_vec();
    };
    if let Some(files) = list
        .get_mut("files")
        .and_then(serde_json::Value::as_array_mut)
    {
        let generated = format!("{GENERATED}/");
        files.retain(|file| {
            !file
                .as_str()
                .is_some_and(|file| file.starts_with(&generated))
        });
    }
    serde_json::to_vec(&list).unwrap_or_else(|_| bytes.to_vec())
}

/// The version the room compares for the mod in `dir`, of `revision`:
/// `revision+<16 hex digits>`. A mod whose files cannot be read gets a
/// fingerprint no other game has (fail closed), and a warning in the log.
pub fn version(revision: &str, dir: &Path) -> String {
    // Short enough that the fingerprint always fits a manifest's version.
    let revision: String = revision.chars().take(12).collect();
    match fingerprint(dir) {
        Ok(digest) => format!("{revision}+{}", hex(&digest)[..SHOWN_DIGITS].to_owned()),
        Err(error) => {
            tracing::warn!(
                "TPF3-MP's mod in {} could not be read for the room's check: {error}",
                dir.display()
            );
            let mut unique = [0u8; 4];
            let _ = SystemRandom::new().fill(&mut unique);
            format!("{revision}+unread-{}", hex(&unique))
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A copy of the mod's layout in `dir`.
    fn installed(dir: &Path) {
        let files: &[(&str, &str)] = &[
            ("mod.json", r#"{"modId": "tpf3mp_1", "revision": 1}"#),
            (
                "_content.json",
                "{\n    \"archives\": null,\n    \"files\": [\n        \"tpf3mp/act.lua\",\n        \"scripts/tpf3mp/roads.lua\"\n    ]\n}\n",
            ),
            ("_metadata/modinfo.json", r#"{"name": "TPF3-MP"}"#),
            ("content/tpf3mp/act.lua", "return {}"),
            (
                "content/scripts/tpf3mp/roads.lua",
                "local roads = {} return roads",
            ),
        ];
        for (path, text) in files {
            let path = dir.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
    }

    #[test]
    fn the_same_files_give_the_same_fingerprint_wherever_they_are() {
        let one = tempfile::tempdir().unwrap();
        let two = tempfile::tempdir().unwrap();
        installed(one.path());
        installed(two.path());
        assert_eq!(
            fingerprint(one.path()).unwrap(),
            fingerprint(two.path()).unwrap()
        );
        let version = version("1", one.path());
        assert_eq!(version.len(), "1+".len() + SHOWN_DIGITS);
        assert!(version.starts_with("1+"));
    }

    #[test]
    fn a_changed_added_or_removed_file_changes_it() {
        let dir = tempfile::tempdir().unwrap();
        installed(dir.path());
        let before = fingerprint(dir.path()).unwrap();
        let roads = dir.path().join("content/scripts/tpf3mp/roads.lua");
        // An old copy: one line differs.
        fs::write(&roads, "local roads = { old = true } return roads").unwrap();
        let changed = fingerprint(dir.path()).unwrap();
        assert_ne!(changed, before);
        fs::write(&roads, "local roads = {} return roads").unwrap();
        assert_eq!(fingerprint(dir.path()).unwrap(), before, "back as it was");
        // A file more, or one less.
        fs::write(dir.path().join("content/scripts/tpf3mp/extra.lua"), "").unwrap();
        assert_ne!(fingerprint(dir.path()).unwrap(), before);
        fs::remove_file(dir.path().join("content/scripts/tpf3mp/extra.lua")).unwrap();
        fs::remove_file(&roads).unwrap();
        assert_ne!(fingerprint(dir.path()).unwrap(), before);
        // The same bytes under another name.
        fs::write(
            dir.path().join("content/scripts/tpf3mp/streets.lua"),
            "local roads = {} return roads",
        )
        .unwrap();
        assert_ne!(fingerprint(dir.path()).unwrap(), before);
        fs::remove_file(dir.path().join("content/scripts/tpf3mp/streets.lua")).unwrap();
        fs::write(&roads, "local roads = {} return roads").unwrap();
        assert_eq!(fingerprint(dir.path()).unwrap(), before);
        // mod.json counts too.
        fs::write(
            dir.path().join("mod.json"),
            r#"{"modId": "tpf3mp_1", "revision": 1, "runScript": {}}"#,
        )
        .unwrap();
        assert_ne!(fingerprint(dir.path()).unwrap(), before);
    }

    #[test]
    fn the_portraits_the_launcher_writes_and_the_metadata_do_not_count() {
        let dir = tempfile::tempdir().unwrap();
        installed(dir.path());
        let before = fingerprint(dir.path()).unwrap();
        // The launcher's portraits, listed in _content.json as it lists
        // them (`crate::portraits`, rewritten in its own layout).
        let portraits = dir.path().join("content").join(GENERATED);
        fs::create_dir_all(&portraits).unwrap();
        fs::write(portraits.join("tom_mclaren.tga"), [1, 2, 3]).unwrap();
        fs::write(portraits.join("build.txt"), "40408").unwrap();
        let list = dir.path().join(CONTENT_LIST);
        let mut content: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&list).unwrap()).unwrap();
        content["files"]
            .as_array_mut()
            .unwrap()
            .push(format!("{GENERATED}/tom_mclaren.tga").into());
        fs::write(&list, serde_json::to_string_pretty(&content).unwrap()).unwrap();
        // What the system and Mod Hub leave beside the mod.
        fs::write(dir.path().join("_metadata/preview.tga"), [9]).unwrap();
        fs::write(dir.path().join("content/desktop.ini"), "x").unwrap();
        fs::write(dir.path().join("_content.json.part"), "partial").unwrap();
        assert_eq!(fingerprint(dir.path()).unwrap(), before);
        // A real change to the list still counts.
        content["files"]
            .as_array_mut()
            .unwrap()
            .push("tpf3mp/other.lua".into());
        fs::write(&list, serde_json::to_string_pretty(&content).unwrap()).unwrap();
        assert_ne!(fingerprint(dir.path()).unwrap(), before);
    }

    #[test]
    fn a_mod_that_cannot_be_read_matches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("tpf3mp_1");
        assert!(fingerprint(&missing).is_err());
        let one = version("1", &missing);
        let two = version("1", &missing);
        assert!(one.starts_with("1+unread-"), "{one}");
        assert_ne!(one, two, "two unread copies never match each other");
    }

    #[test]
    fn the_mod_in_the_repository_reads() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mod/tpf3mp_1");
        let version = version("1", &dir);
        assert!(!version.contains("unread"), "{version}");
    }
}

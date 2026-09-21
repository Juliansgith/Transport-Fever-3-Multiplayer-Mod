//! What this player's game runs, until the game's hook reports it: the
//! game's build, and its mods from a list the player keeps.
//!
//! The list names the active mods in load order, one per line: the mod's
//! name, then its version if it has one. Blank lines and lines starting
//! with `#` are skipped.
//!
//! ```text
//! # my mods, in the order the game loads them
//! urbangames_vehicles 1.4
//! more_stations 2
//! ```

use std::{fs, path::Path};

use thiserror::Error;
use tpf3mp_proto::{ContentManifest, ModRef, Text};

/// Longest mod list read, in bytes: far more than any real list.
const MAX_LIST_BYTES: u64 = 4 << 20;

#[derive(Debug, Error)]
pub enum ContentError {
    #[error("reading {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("{path} is larger than a list of mods can be")]
    TooLarge { path: String },
    #[error("{path}, line {line}: {problem}")]
    Line {
        path: String,
        line: usize,
        problem: &'static str,
    },
    #[error("the game build name is too long")]
    Build,
}

/// The manifest of `game_build` running the mods listed in `mods`, or none.
pub fn manifest(game_build: &str, mods: Option<&Path>) -> Result<ContentManifest, ContentError> {
    let game = Text::new(game_build.trim()).map_err(|_| ContentError::Build)?;
    let mods = match mods {
        Some(path) => read_mods(path)?,
        None => Vec::new(),
    };
    Ok(ContentManifest::new(game, mods))
}

/// The mods listed in `path`, in order.
pub fn read_mods(path: &Path) -> Result<Vec<ModRef>, ContentError> {
    let shown = path.display().to_string();
    let read = |source| ContentError::Read {
        path: shown.clone(),
        source,
    };
    if fs::metadata(path).map_err(read)?.len() > MAX_LIST_BYTES {
        return Err(ContentError::TooLarge { path: shown });
    }
    let text = fs::read_to_string(path).map_err(read)?;
    parse_mods(&text).map_err(|(line, problem)| ContentError::Line {
        path: shown,
        line,
        problem,
    })
}

fn parse_mods(text: &str) -> Result<Vec<ModRef>, (usize, &'static str)> {
    let mut mods = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.split_whitespace();
        let id = words.next().unwrap_or_default();
        let version = words.next().unwrap_or_default();
        if words.next().is_some() {
            return Err((index + 1, "expected a mod's name and at most its version"));
        }
        mods.push(ModRef {
            id: Text::new(id).map_err(|_| (index + 1, "the mod's name is too long"))?,
            version: Text::new(version).map_err(|_| (index + 1, "the version is too long"))?,
        });
    }
    Ok(mods)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_names_mods_in_load_order() {
        let mods =
            parse_mods("# my mods\n\nurbangames_vehicles 1.4\n  more_stations 2  \nno_version\n")
                .unwrap();
        let names: Vec<(&str, &str)> = mods
            .iter()
            .map(|listed| (listed.id.as_str(), listed.version.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("urbangames_vehicles", "1.4"),
                ("more_stations", "2"),
                ("no_version", "")
            ]
        );
    }

    #[test]
    fn a_line_that_is_not_a_mod_is_refused_with_its_number() {
        assert_eq!(
            parse_mods("a 1\nb 2 extra\n").unwrap_err(),
            (2, "expected a mod's name and at most its version")
        );
        let long = "x".repeat(200);
        assert_eq!(
            parse_mods(&format!("{long} 1\n")).unwrap_err(),
            (1, "the mod's name is too long")
        );
    }

    #[test]
    fn the_same_list_gives_the_same_fingerprint() {
        let dir = std::env::temp_dir().join(format!("tpf3mp-mods-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("mods.txt");
        fs::write(&file, "trains 1\nstations 2\n").unwrap();
        let one = manifest("35924", Some(&file)).unwrap();
        let two = manifest("35924", Some(&file)).unwrap();
        assert_eq!(one.fingerprint(), two.fingerprint());
        assert_ne!(
            one.fingerprint(),
            manifest("35924", None).unwrap().fingerprint()
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}

//! The campaign characters' portraits players may show in rooms instead of
//! a banner (`tpf3mp_proto::PORTRAITS`, docs/LOBBY.md "Portraits").
//!
//! The pictures are the game's own art, so TPF3-MP never ships them: at
//! startup the launcher takes each character's neutral portrait from the
//! player's own install (the campaign missions' `mission.zip`,
//! `mission/dialogue/<id>_neutral.tga`), makes it [`SIZE`] pixels square,
//! and writes it into the installed TPF3-MP mod as
//! `content/gui/tpf3mp/portraits/<id>.tga`, which the game's windows load as
//! `tpf3mp_1::/gui/tpf3mp/portraits/<id>.tga`. It does so once per game
//! build ([`STAMP`]), and lists the files in the mod's `_content.json`.
//!
//! Only the portraits present can be shown ([`available`]): the lobby
//! offers those, and shows any other player's portrait this game lacks as
//! that player's default banner ([`shown`]).

use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::RwLock,
};

use anyhow::{Context, Result, bail};
use tpf3mp_proto::{PORTRAITS, is_portrait};

/// The width and height of a portrait as the mod holds it: the lobby shows
/// them at most a little over 100 pixels, so this is sharp at twice that.
pub const SIZE: u32 = 256;
/// The portraits' folder in the mod, under `content/`, as the game names
/// files of a mod (`tpf3mp_1::/<this>/<id>.tga`).
pub const FOLDER: &str = "gui/tpf3mp/portraits";
/// The file in [`FOLDER`] naming the game build the portraits came from.
pub const STAMP: &str = "build.txt";
/// The campaign missions whose `mission.zip` holds the portraits.
const MISSIONS: std::ops::RangeInclusive<u8> = 1..=8;
/// Largest portrait read from a mission: the game's are 4 MiB.
const MAX_SOURCE: u64 = 32 << 20;

static AVAILABLE: RwLock<Vec<&'static str>> = RwLock::new(Vec::new());

/// The portraits this player's game can show, in `PORTRAITS`' order.
pub fn available() -> Vec<&'static str> {
    AVAILABLE.read().map(|ids| ids.clone()).unwrap_or_default()
}

/// Whether this player's game can show the picture `id`: any banner, and a
/// portrait it has.
pub fn shown(id: &str) -> bool {
    shown_in(id, &available())
}

/// Whether `id` can be shown where the portraits `available` are.
pub fn shown_in(id: &str, available: &[&str]) -> bool {
    !is_portrait(id) || available.contains(&id)
}

fn set_available(ids: Vec<&'static str>) {
    if let Ok(mut available) = AVAILABLE.write() {
        *available = ids;
    }
}

/// Where the game's campaign missions keep their `mission.zip`, those
/// present, in the missions' order.
pub fn sources(game_dir: &Path) -> Vec<PathBuf> {
    MISSIONS
        .map(|n| {
            game_dir
                .join("mods")
                .join("release")
                .join(format!("urbangames_campaign_mission_{n:02}"))
                .join("content")
                .join("mission.zip")
        })
        .filter(|zip| zip.is_file())
        .collect()
}

/// The installed TPF3-MP mod the game loads: the first `tpf3mp_1` in the
/// mod folders, in the game's order.
pub fn installed_mod(game_dir: Option<&Path>) -> Option<PathBuf> {
    tpf3mp_modscan::roots::installed(&tpf3mp_modscan::roots::default_roots(
        game_dir,
        &crate::steam::steam_roots(),
    ))
    .into_iter()
    .find(|found| found.id == tpf3mp_bridge::mods::OWN_MOD)
    .map(|found| found.path)
}

/// At the launcher's start: the portraits of the game in `game_dir`, build
/// `build`, put into the installed mod where missing, and made the ones
/// [`available`]. Never fails: what goes wrong goes to the log, and the
/// lobby then offers the portraits it has.
pub fn prepare_installed(game_dir: &Path, build: &str) {
    let Some(mod_dir) = installed_mod(Some(game_dir)) else {
        tracing::info!("portraits: TPF3-MP's mod is not installed; no portraits");
        return;
    };
    let started = std::time::Instant::now();
    match prepare(&sources(game_dir), &mod_dir, build) {
        Ok(ready) => {
            if ready.made > 0 || ready.available.len() < PORTRAITS.len() {
                tracing::info!(
                    "portraits: {} of {} ready in {} ({} made in {:?})",
                    ready.available.len(),
                    PORTRAITS.len(),
                    mod_dir.display(),
                    ready.made,
                    started.elapsed()
                );
            }
            set_available(ready.available);
        }
        Err(error) => {
            tracing::warn!("portraits: {error:#}");
            set_available(present(&mod_dir));
        }
    }
}

/// What [`prepare`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    /// The portraits the mod now has, in `PORTRAITS`' order.
    pub available: Vec<&'static str>,
    /// How many it made this time.
    pub made: usize,
}

/// The portrait files of the mod in `mod_dir`, from the missions' zips
/// `sources`: each one missing made, or all of them again when the game
/// build differs from the one they were made from. Without `sources` (no
/// campaign) nothing is made, and the mod keeps what it has.
pub fn prepare(sources: &[PathBuf], mod_dir: &Path, build: &str) -> Result<Prepared> {
    let dir = mod_dir.join("content").join(FOLDER);
    let stamp = dir.join(STAMP);
    let same_build = std::fs::read_to_string(&stamp)
        .map(|made| made.trim() == build.trim())
        .unwrap_or(false);
    let have = present(mod_dir);
    let wanted: Vec<&'static str> = PORTRAITS
        .iter()
        .copied()
        .filter(|id| !same_build || !have.contains(id))
        .collect();
    let mut made = 0;
    if !wanted.is_empty() {
        if sources.is_empty() {
            tracing::info!(
                "portraits: the game has no campaign missions to take them from; {} kept",
                have.len()
            );
        } else {
            std::fs::create_dir_all(&dir).with_context(|| format!("making {}", dir.display()))?;
            let mut left = wanted;
            for source in sources {
                if left.is_empty() {
                    break;
                }
                match extract_from(source, &dir, &mut left) {
                    Ok(count) => made += count,
                    Err(error) => tracing::warn!("portraits: {}: {error:#}", source.display()),
                }
            }
            if !left.is_empty() {
                tracing::info!(
                    "portraits: not in this game's campaign: {}",
                    left.join(", ")
                );
            }
            std::fs::write(&stamp, build.trim())
                .with_context(|| format!("writing {}", stamp.display()))?;
        }
    }
    let available = present(mod_dir);
    list_in_content(mod_dir, &available)?;
    Ok(Prepared { available, made })
}

/// The portraits whose file the mod in `mod_dir` has.
fn present(mod_dir: &Path) -> Vec<&'static str> {
    let dir = mod_dir.join("content").join(FOLDER);
    PORTRAITS
        .iter()
        .copied()
        .filter(|id| dir.join(format!("{id}.tga")).is_file())
        .collect()
}

/// Takes each of the portraits `left` the zip `source` holds out of it into
/// `dir`, and out of `left`. Returns how many it made.
fn extract_from(source: &Path, dir: &Path, left: &mut Vec<&'static str>) -> Result<usize> {
    let file = std::fs::File::open(source).context("opening")?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).context("reading")?;
    let mut made = 0;
    let mut i = 0;
    while i < left.len() {
        let id = left[i];
        let name = format!("mission/dialogue/{id}_neutral.tga");
        let bytes = match zip.by_name(&name) {
            Ok(entry) => {
                if entry.size() > MAX_SOURCE {
                    bail!("{name} is {} bytes", entry.size());
                }
                let mut bytes = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
                entry
                    .take(MAX_SOURCE)
                    .read_to_end(&mut bytes)
                    .with_context(|| format!("reading {name}"))?;
                bytes
            }
            Err(zip::result::ZipError::FileNotFound) => {
                i += 1;
                continue;
            }
            Err(error) => return Err(error).with_context(|| format!("finding {name}")),
        };
        let tga = portrait(&bytes).with_context(|| name.clone())?;
        let target = dir.join(format!("{id}.tga"));
        let partial = dir.join(format!("{id}.tga.part"));
        std::fs::write(&partial, &tga).with_context(|| format!("writing {}", partial.display()))?;
        std::fs::rename(&partial, &target)
            .with_context(|| format!("writing {}", target.display()))?;
        left.remove(i);
        made += 1;
    }
    Ok(made)
}

/// A TGA picture as the mod holds a portrait: decoded, made at most
/// [`SIZE`] pixels square, and written as the game's own pictures are.
pub fn portrait(tga: &[u8]) -> Result<Vec<u8>> {
    let picture = image::load_from_memory_with_format(tga, image::ImageFormat::Tga)
        .context("decoding")?
        .to_rgba8();
    let picture = if picture.width() > SIZE || picture.height() > SIZE {
        image::imageops::thumbnail(&picture, SIZE, SIZE)
    } else {
        picture
    };
    Ok(encode_tga(&picture))
}

/// `picture` as an uncompressed 32-bit TGA with its rows bottom up: the
/// layout of the game's own (`mission/dialogue/*.tga`), which its texture
/// loader is known to read.
fn encode_tga(picture: &image::RgbaImage) -> Vec<u8> {
    let (width, height) = picture.dimensions();
    let mut out = Vec::with_capacity(18 + (width * height * 4) as usize);
    out.extend_from_slice(&[0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    // Portraits are at most SIZE square, which fits 16 bits.
    out.extend_from_slice(&(width as u16).to_le_bytes());
    out.extend_from_slice(&(height as u16).to_le_bytes());
    // 32 bits a pixel; 8 of them alpha, origin at the bottom left.
    out.extend_from_slice(&[32, 8]);
    for y in (0..height).rev() {
        for x in 0..width {
            let [r, g, b, a] = picture.get_pixel(x, y).0;
            out.extend_from_slice(&[b, g, r, a]);
        }
    }
    out
}

/// Lists each portrait in `available` in the mod's `_content.json`, the
/// mod's list of its files, where it is not yet.
fn list_in_content(mod_dir: &Path, available: &[&str]) -> Result<()> {
    let path = mod_dir.join("_content.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let mut content: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .with_context(|| format!("reading {}", path.display()))?;
    let Some(files) = content
        .get_mut("files")
        .and_then(serde_json::Value::as_array_mut)
    else {
        bail!("{} lists no files", path.display());
    };
    let mut added = false;
    for id in available {
        let file = format!("{FOLDER}/{id}.tga");
        if !files.iter().any(|listed| listed.as_str() == Some(&file)) {
            files.push(serde_json::Value::String(file));
            added = true;
        }
    }
    if added {
        let partial = mod_dir.join("_content.json.part");
        let mut text = serde_json::to_string_pretty(&content)?;
        text.push('\n');
        std::fs::write(&partial, text).with_context(|| format!("writing {}", partial.display()))?;
        std::fs::rename(&partial, &path).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// A tiny TGA as the game's: uncompressed, 32 bits, bottom up, with
    /// every pixel `bgra`.
    fn tga(size: u16, bgra: [u8; 4]) -> Vec<u8> {
        let mut out = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&[32, 8]);
        for _ in 0..u32::from(size) * u32::from(size) {
            out.extend_from_slice(&bgra);
        }
        out
    }

    /// A campaign mission's zip holding `files`.
    fn mission(path: &Path, files: &[(&str, Vec<u8>)]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, bytes) in files {
            zip.start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    fn game_with_missions(game: &Path) {
        let zip = |n: u8| {
            game.join(format!(
                "mods/release/urbangames_campaign_mission_{n:02}/content/mission.zip"
            ))
        };
        mission(
            &zip(1),
            &[
                (
                    "mission/dialogue/andrew_neutral.tga",
                    tga(4, [10, 20, 30, 255]),
                ),
                ("mission/dialogue/andrew_happy.tga", tga(4, [0, 0, 0, 255])),
                (
                    "mission/dialogue/major_neutral.tga",
                    tga(600, [1, 2, 3, 128]),
                ),
            ],
        );
        mission(
            &zip(8),
            &[
                ("mission/dialogue/andrew_neutral.tga", tga(2, [9, 9, 9, 9])),
                ("mission/dialogue/none_neutral.tga", tga(2, [0, 0, 0, 0])),
                (
                    "mission/dialogue/takumi_arakawa_neutral.tga",
                    tga(2, [5, 6, 7, 8]),
                ),
            ],
        );
    }

    fn mod_folder(dir: &Path) -> PathBuf {
        let mod_dir = dir.join("staging_area").join("tpf3mp_1");
        std::fs::create_dir_all(mod_dir.join("content")).unwrap();
        std::fs::write(mod_dir.join("mod.json"), "{}").unwrap();
        std::fs::write(
            mod_dir.join("_content.json"),
            "{\n    \"archives\": null,\n    \"files\": [\n        \"gui/menu/lobby.lua\"\n    ]\n}\n",
        )
        .unwrap();
        mod_dir
    }

    fn listed(mod_dir: &Path) -> Vec<String> {
        let content: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(mod_dir.join("_content.json")).unwrap())
                .unwrap();
        content["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file.as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn the_neutral_portraits_are_taken_from_the_missions_once_per_build() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        game_with_missions(&game);
        let mod_dir = mod_folder(dir.path());
        let found = sources(&game);
        assert_eq!(found.len(), 2, "missions 1 and 8");

        let ready = prepare(&found, &mod_dir, "steam-1").unwrap();
        assert_eq!(ready.available, ["andrew", "major", "takumi_arakawa"]);
        assert_eq!(ready.made, 3);
        let folder = mod_dir.join("content").join(FOLDER);
        assert!(
            !folder.join("none.tga").exists(),
            "the empty speaker is no portrait"
        );
        assert!(!folder.join("andrew.tga.part").exists());
        // The first mission's Andrew, as the game's own TGAs are laid out.
        let andrew = std::fs::read(folder.join("andrew.tga")).unwrap();
        assert_eq!(
            &andrew[..18],
            &[0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 0, 4, 0, 32, 8]
        );
        assert_eq!(andrew.len(), 18 + 4 * 4 * 4);
        assert_eq!(&andrew[18..22], &[10, 20, 30, 255]);
        // A large one made SIZE square, its colour kept.
        let major = image::load_from_memory_with_format(
            &std::fs::read(folder.join("major.tga")).unwrap(),
            image::ImageFormat::Tga,
        )
        .unwrap()
        .to_rgba8();
        assert_eq!(major.dimensions(), (SIZE, SIZE));
        assert_eq!(major.get_pixel(7, 200).0, [3, 2, 1, 128]);
        // Listed in the mod's list of files, after what it had.
        assert_eq!(
            listed(&mod_dir),
            [
                "gui/menu/lobby.lua",
                "gui/tpf3mp/portraits/andrew.tga",
                "gui/tpf3mp/portraits/major.tga",
                "gui/tpf3mp/portraits/takumi_arakawa.tga",
            ]
        );

        // The same build again: nothing to do, and nothing listed twice.
        std::fs::write(folder.join("andrew.tga"), b"kept").unwrap();
        let again = prepare(&found, &mod_dir, "steam-1").unwrap();
        assert_eq!(again.made, 0);
        assert_eq!(std::fs::read(folder.join("andrew.tga")).unwrap(), b"kept");
        assert_eq!(listed(&mod_dir).len(), 4);
        // One gone (a reinstalled mod): only it is made again.
        std::fs::remove_file(folder.join("major.tga")).unwrap();
        assert_eq!(prepare(&found, &mod_dir, "steam-1").unwrap().made, 1);
        assert_eq!(std::fs::read(folder.join("andrew.tga")).unwrap(), b"kept");
        // Another build: all of them again.
        assert_eq!(prepare(&found, &mod_dir, "steam-2").unwrap().made, 3);
        assert_ne!(std::fs::read(folder.join("andrew.tga")).unwrap(), b"kept");
    }

    #[test]
    fn without_the_campaign_nothing_is_made_and_what_is_there_kept() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        std::fs::create_dir_all(&game).unwrap();
        let mod_dir = mod_folder(dir.path());
        assert!(sources(&game).is_empty());
        let ready = prepare(&[], &mod_dir, "steam-1").unwrap();
        assert_eq!(
            ready,
            Prepared {
                available: Vec::new(),
                made: 0
            }
        );
        assert!(!mod_dir.join("content").join(FOLDER).exists());
        assert_eq!(listed(&mod_dir), ["gui/menu/lobby.lua"]);

        // Made under an earlier build, then the campaign went: kept.
        let folder = mod_dir.join("content").join(FOLDER);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("lasse.tga"), tga(2, [1, 1, 1, 1])).unwrap();
        let ready = prepare(&[], &mod_dir, "steam-2").unwrap();
        assert_eq!(ready.available, ["lasse"]);
    }

    #[test]
    fn a_damaged_mission_is_passed_over() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        game_with_missions(&game);
        std::fs::write(
            game.join("mods/release/urbangames_campaign_mission_01/content/mission.zip"),
            b"not a zip",
        )
        .unwrap();
        let mod_dir = mod_folder(dir.path());
        let ready = prepare(&sources(&game), &mod_dir, "steam-1").unwrap();
        assert_eq!(
            ready.available,
            ["andrew", "takumi_arakawa"],
            "from mission 8"
        );
        assert!(portrait(b"not a picture").is_err());
    }

    #[test]
    fn a_portrait_this_game_lacks_is_not_shown_and_banners_always_are() {
        assert!(shown_in("dry", &[]));
        assert!(shown_in("andrew", &["andrew"]));
        assert!(!shown_in("andrew", &["lasse"]));
        assert!(!shown_in("andrew", &[]));
        // Not a portrait: whether it is a banner at all is checked apart.
        assert!(shown_in("anything", &[]));
    }
}

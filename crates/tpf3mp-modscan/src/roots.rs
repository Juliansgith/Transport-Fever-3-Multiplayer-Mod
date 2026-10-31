//! Where Transport Fever 3 keeps the mods a player can activate (build
//! 40408, Windows; the other platforms' places are to confirm):
//!
//! - Mod Hub (mod.io) downloads: `%PUBLIC%\mod.io\10640\mods\<mod.io id>`
//!   (58 mods on one PC, 2026-10-03, while `%LOCALAPPDATA%\mod.io\10640`
//!   held only its user's file), and `%LOCALAPPDATA%\mod.io\10640\mods`,
//!   where they were found on 2026-09-30; each `mod.json` names the mod
//!   (`revyn112_towns_de` in `...\6414521`);
//! - local mods: `<Steam>\userdata\<account>\3493540\local\staging_area\<modId>`
//!   (investigation/TF3_MODS_2026-09-27.md), and `...\local\mods`;
//! - the game's own: `<game>\mods`, `<game>\mods\release` and
//!   `<game>\dlcs`. `release` holds the game's built-in mods, the ones a
//!   save lists as `urbangames_no_costs_1`, `urbangames_sandbox_1` and so
//!   on, and the campaign's (21 mods on one PC, 2026-10-04; the `mw_*` among
//!   them were also in `<game>\mods`, with the same files).
//!
//! A mod is found by the id its `mod.json` gives, or else by its folder's
//! name. A save lists a Mod Hub mod by the `modId` (`revyn112_towns_de`,
//! its mod.io number `6414521` only as the hub id; docs/MODS.md).
//!
//! A Mod Hub download also says which file of the mod it is: mod.io's cache
//! index, `mod.io\10640\metadata\state.json` beside the `mods` folder, lists
//! each mod by its mod.io number (`ID`, the folder's name) with the file it
//! installed (`Profile.modfile.id`) and its state (`State`, 1 for each of
//! 58 installed mods on one PC, 2026-10-04). Authors rarely raise a mod's
//! `revision` (45 of those 58 were at 1), so the file tells two downloads of
//! one revision apart.

use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

/// Transport Fever 3's game id on mod.io.
pub const MODIO_GAME: u32 = 10640;
/// Transport Fever 3's Steam app id.
pub const STEAM_APP: u32 = 3_493_540;

/// A folder mods are kept in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    pub path: PathBuf,
    /// For a folder of Mod Hub downloads (`<storage>\10640\mods`): where it
    /// was found. Its mods are versioned by the file mod.io installed,
    /// whatever its path is named.
    pub hub: Option<HubSource>,
}

impl From<PathBuf> for Root {
    /// A folder of local mods.
    fn from(path: PathBuf) -> Self {
        Self { path, hub: None }
    }
}

/// Where a folder mod.io may keep this game's downloads in was found. The
/// mod.io SDK the game embeds picks it (modio-sdk 2026.10,
/// `LoadGlobalConfigOverrideFileDataOp.h`, `FileSystemImplementation.h`;
/// its strings are in build 40425): its default folder, unless
/// `RootLocalStoragePath` in its settings moves it, a user's settings
/// before the global ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubSource {
    /// `RootLocalStoragePath` in a user's
    /// `%LOCALAPPDATA%\mod.io\10640\<user>\user.json`: moves this game's
    /// downloads for that user alone (SDK 2024.7 on).
    UserOverride,
    /// `RootLocalStoragePath` in `%LOCALAPPDATA%\mod.io\globalsettings.json`:
    /// moves the downloads of every game on mod.io's SDK. mod.io's support
    /// tells players to edit it to keep their mods on another drive.
    Settings,
    /// A folder the game itself last loaded Mod Hub mods from, as its
    /// `stdout.txt` names it: said in the log, never looked in, as a log
    /// tells of the game's last start, not of where mod.io keeps them now.
    GameLog,
    /// The SDK's default: `mod.io` in the users' shared folder
    /// (`%PUBLIC%`) on Windows, in the per-user data folder elsewhere.
    Default,
    /// `%LOCALAPPDATA%\mod.io`, where they were found on 2026-09-30.
    LocalAppData,
}

impl std::fmt::Display for HubSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UserOverride => "a user's mod.io settings (user.json)",
            Self::Settings => "mod.io's settings (globalsettings.json)",
            Self::GameLog => "the game's log (stdout.txt)",
            Self::Default => "mod.io's default folder",
            Self::LocalAppData => "the local app data folder",
        })
    }
}

/// A mod.io storage folder: the one holding `10640\mods` and
/// `10640\metadata\state.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubRoot {
    pub source: HubSource,
    pub storage: PathBuf,
}

impl HubRoot {
    /// Its folder of this game's downloads.
    pub fn mods(&self) -> PathBuf {
        self.storage.join(MODIO_GAME.to_string()).join("mods")
    }
}

/// Where mod.io may keep this game's downloads, what its settings said on
/// the way, and which copy of a mod the game chose where it found one
/// twice ([`hub_roots`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HubSearch {
    /// Each storage folder found, in the order looked at.
    pub roots: Vec<HubRoot>,
    /// What the settings said, for the log, without paths.
    pub notes: Vec<String>,
    /// For a mod the game found twice: the id, and the folder of the copy
    /// it chose, as its log says.
    pub chosen: Vec<(String, PathBuf)>,
}

/// What became of a storage folder found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Use {
    Used,
    /// The folder mod.io uses, but no downloads of this game are there.
    NotThere,
    /// The same folder as the one at this index.
    Same(usize),
    /// mod.io uses the one at this index instead.
    Passed(usize),
    /// Named by the game's last log; mod.io uses the one at this index, if
    /// any is known.
    Logged(Option<usize>),
}

impl HubSearch {
    /// The folder Mod Hub's downloads are taken from: the one mod.io uses,
    /// the first of its own places found (a user's settings', the global
    /// settings', its default), if this game's downloads are there. Only
    /// that one: an old folder left behind after a move, or a folder the
    /// game's last log names but mod.io no longer uses, holds downloads the
    /// game will not load. A folder named but not there (a drive not
    /// connected, a folder chosen but not yet used) is not looked past.
    pub fn used(&self) -> Vec<HubRoot> {
        self.uses()
            .into_iter()
            .zip(&self.roots)
            .filter(|(used, _)| *used == Use::Used)
            .map(|(_, root)| root.clone())
            .collect()
    }

    fn uses(&self) -> Vec<Use> {
        let mine = self
            .roots
            .iter()
            .position(|root| root.source != HubSource::GameLog);
        let mut uses = Vec::with_capacity(self.roots.len());
        for (index, root) in self.roots.iter().enumerate() {
            let used = if let Some(earlier) = self.roots[..index]
                .iter()
                .position(|earlier| same_folder(&earlier.storage, &root.storage))
            {
                Use::Same(earlier)
            } else if Some(index) == mine {
                if root.mods().is_dir() {
                    Use::Used
                } else {
                    Use::NotThere
                }
            } else if root.source == HubSource::GameLog {
                Use::Logged(mine)
            } else {
                Use::Passed(mine.unwrap_or(index))
            };
            uses.push(used);
        }
        uses
    }

    /// Lines for the log: what the settings said, then for each folder
    /// where it was found and what became of it, with how many downloads
    /// it held and of how many mod.io's index gives the file. No paths:
    /// diagnostics take them out of a line, so the lines say what survives
    /// that.
    pub fn report(&self, found: &[Found]) -> Vec<String> {
        let mut out = self.notes.clone();
        for (index, (root, used)) in self.roots.iter().zip(self.uses()).enumerate() {
            let what = match used {
                Use::Used => {
                    let mods = root.mods();
                    let here: Vec<&Found> = found
                        .iter()
                        .filter(|f| f.path.parent() == Some(mods.as_path()))
                        .collect();
                    let known = here
                        .iter()
                        .filter(|f| f.hub.is_some_and(|hub| hub.file.is_some()))
                        .count();
                    format!(
                        "looked in: {} mods, {known} of them with the file mod.io installed",
                        here.len()
                    )
                }
                Use::NotThere => "not there".to_owned(),
                Use::Same(earlier) => format!("the same folder as place {}", earlier + 1),
                Use::Logged(Some(mine)) => format!(
                    "the game last loaded Mod Hub mods from here, but mod.io uses place {}: not looked in",
                    mine + 1
                ),
                Use::Logged(None) => {
                    "the game last loaded Mod Hub mods from here: not looked in".to_owned()
                }
                Use::Passed(mine) => format!(
                    "{}, but mod.io uses place {}: not looked in",
                    if root.mods().is_dir() {
                        "there"
                    } else {
                        "not there"
                    },
                    mine + 1
                ),
            };
            out.push(format!(
                "Mod Hub's downloads, place {} ({}): {what}",
                index + 1,
                root.source
            ));
        }
        out
    }
}

/// The folders mods are kept in, those that exist, the first copy of a mod
/// counting: Mod Hub's downloads (in each of `hub`, in order, each folder
/// once), each Steam account's local ones (under each of `steam_roots`),
/// and the game's own (in `game`, its install folder).
pub fn roots(game: Option<&Path>, steam_roots: &[PathBuf], hub: &[HubRoot]) -> Vec<Root> {
    let mut out: Vec<Root> = Vec::new();
    for (index, root) in hub.iter().enumerate() {
        if hub[..index]
            .iter()
            .any(|earlier| same_folder(&earlier.storage, &root.storage))
        {
            continue;
        }
        out.push(Root {
            path: root.mods(),
            hub: Some(root.source),
        });
    }
    for steam in steam_roots {
        for account in accounts(steam) {
            let local = account.join(STEAM_APP.to_string()).join("local");
            out.push(local.join("staging_area").into());
            out.push(local.join("mods").into());
        }
    }
    if let Some(game) = game {
        out.push(game.join("mods").into());
        out.push(game.join("mods").join("release").into());
        out.push(game.join("dlcs").into());
    }
    out.retain(|root| root.path.is_dir());
    out
}

/// Each Steam account's folder under `steam`, in order.
fn accounts(steam: &Path) -> Vec<PathBuf> {
    let Ok(accounts) = fs::read_dir(steam.join("userdata")) else {
        return Vec::new();
    };
    let mut accounts: Vec<PathBuf> = accounts.filter_map(Result::ok).map(|e| e.path()).collect();
    accounts.sort();
    accounts
}

/// Every mod this player has: [`installed`] in [`roots`] with the folders
/// mod.io keeps its downloads in ([`HubSearch::used`]), each mod found more
/// than once [`settle`]d; and how Mod Hub's downloads were looked for, for
/// the log ([`HubSearch::report`]).
pub fn discovered(game: Option<&Path>, steam_roots: &[PathBuf]) -> (HubSearch, Vec<Found>) {
    let search = hub_roots(steam_roots);
    let found = settle(
        installed(&roots(game, steam_roots, &search.used())),
        &search.chosen,
    );
    (search, found)
}

/// `found` with each mod found more than once settled, so that the copy
/// that counts (the first, [`find`]) is the one the game loads, or is
/// marked [`Found::unsure`]: the copy the game's log says it chose, where
/// it names one of them, comes first; else, when one copy is a Mod Hub
/// download and another not, the first is unsure. Which of those the game
/// loads is not known for certain: the game loaded the local copy in each
/// case seen (two PCs, 2026-10-25), but nothing says it always does.
/// Copies all local, such as the game's own `mw_*` in both `mods` and
/// `mods\release`, stay as found.
pub fn settle(mut found: Vec<Found>, chosen: &[(String, PathBuf)]) -> Vec<Found> {
    for index in 0..found.len() {
        if found[..index].iter().any(|f| f.id == found[index].id) {
            continue;
        }
        let copies: Vec<usize> = (index..found.len())
            .filter(|&i| found[i].id == found[index].id)
            .collect();
        if copies.len() < 2 {
            continue;
        }
        // Logs naming different copies (two Steam accounts' games) settle
        // nothing.
        let mut picks: Vec<usize> = chosen
            .iter()
            .filter(|(id, _)| *id == found[index].id)
            .filter_map(|(_, path)| {
                copies
                    .iter()
                    .copied()
                    .find(|&i| same_folder(&found[i].path, path))
            })
            .collect();
        picks.sort_unstable();
        picks.dedup();
        let pick = match picks.as_slice() {
            [pick] => Some(*pick),
            _ => None,
        };
        match pick {
            Some(pick) => found.swap(index, pick),
            None => {
                let hub = copies.iter().any(|&i| found[i].hub.is_some());
                let local = copies.iter().any(|&i| found[i].hub.is_none());
                found[index].unsure = hub && local;
            }
        }
    }
    found
}

/// Where mod.io may keep this game's downloads on this system, in the order
/// the SDK picks it ([`HubSearch::used`] takes the first): a user's
/// settings' folder, else the global settings' folder, else its default
/// folder, else the local app data folder (as found before); then, for the
/// log alone, each folder the game's log under each of `steam_roots` says
/// it loaded Mod Hub mods from. A setting that is no absolute path is
/// passed over: nothing tells where it leads.
pub fn hub_roots(steam_roots: &[PathBuf]) -> HubSearch {
    #[cfg(windows)]
    let (local, default) = (
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        std::env::var_os("PUBLIC").map(|public| PathBuf::from(public).join("mod.io")),
    );
    #[cfg(not(windows))]
    let (local, default): (Option<PathBuf>, _) =
        (None, dirs_local().map(|data| data.join("mod.io")));
    hub_roots_in(local.as_deref(), default, steam_roots)
}

/// [`hub_roots`] with `local`, the local app data folder (where mod.io
/// keeps its settings; Windows only), and `default`, mod.io's default
/// storage folder.
fn hub_roots_in(
    local: Option<&Path>,
    default: Option<PathBuf>,
    steam_roots: &[PathBuf],
) -> HubSearch {
    let mut search = HubSearch::default();
    if let Some(local) = local {
        let modio = local.join("mod.io");
        let mut users: Vec<PathBuf> = fs::read_dir(modio.join(MODIO_GAME.to_string()))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path().join("user.json"))
            .filter(|file| file.is_file())
            .collect();
        users.sort();
        for (index, user) in users.iter().enumerate() {
            let what = format!("{} (user {})", HubSource::UserOverride, index + 1);
            let setting = storage_setting(user);
            // A user's settings name no folder unless moved: no note then.
            if setting != Setting::NoFolder {
                search.notes.push(format!("{what}: {setting}"));
            }
            if let Setting::Names(storage) = setting {
                search.roots.push(HubRoot {
                    source: HubSource::UserOverride,
                    storage,
                });
            }
        }
        let setting = storage_setting(&modio.join("globalsettings.json"));
        search
            .notes
            .push(format!("{}: {setting}", HubSource::Settings));
        if let Setting::Names(storage) = setting {
            search.roots.push(HubRoot {
                source: HubSource::Settings,
                storage,
            });
        }
    }
    if let Some(default) = default {
        search.roots.push(HubRoot {
            source: HubSource::Default,
            storage: default,
        });
    }
    if let Some(local) = local {
        search.roots.push(HubRoot {
            source: HubSource::LocalAppData,
            storage: local.join("mod.io"),
        });
    }
    for steam in steam_roots {
        for account in accounts(steam) {
            let log = account
                .join(STEAM_APP.to_string())
                .join("local")
                .join("crash_dump")
                .join("stdout.txt");
            let logged = read_game_log(&log);
            for storage in logged.storage {
                search.roots.push(HubRoot {
                    source: HubSource::GameLog,
                    storage,
                });
            }
            search.chosen.extend(logged.chosen);
        }
    }
    search
}

/// What a mod.io settings file says of its storage folder.
#[derive(Debug, PartialEq, Eq)]
enum Setting {
    /// No file.
    Missing,
    /// Not a JSON object: the SDK writes it anew with its folder.
    Unreadable,
    /// No `RootLocalStoragePath`, or an empty one or no text.
    NoFolder,
    Names(PathBuf),
    /// A folder that is no absolute path.
    NotAbsolute,
}

impl std::fmt::Display for Setting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Missing => "none",
            Self::Unreadable => "does not read, so it moves nothing",
            Self::NoFolder => "names no folder",
            Self::Names(_) => "names a folder",
            Self::NotAbsolute => "names a folder that is no absolute path: passed over",
        })
    }
}

/// What the mod.io settings file (`globalsettings.json`, a `user.json`) at
/// `file` names in `RootLocalStoragePath`. Only that key is kept: a
/// `user.json` also holds its user's mod.io login.
fn storage_setting(file: &Path) -> Setting {
    let Ok(text) = fs::read_to_string(file) else {
        return if file.exists() {
            Setting::Unreadable
        } else {
            Setting::Missing
        };
    };
    let Ok(serde_json::Value::Object(settings)) = serde_json::from_str(&text) else {
        return Setting::Unreadable;
    };
    match settings
        .get("RootLocalStoragePath")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
    {
        None | Some("") => Setting::NoFolder,
        Some(value) if !Path::new(value).is_absolute() => Setting::NotAbsolute,
        Some(value) => Setting::Names(PathBuf::from(value)),
    }
}

/// The most of the game's log read, which names what it needs here as the
/// game starts.
const LOG_READ: u64 = 8 << 20;

/// What the game's log says of the mods it loaded.
#[derive(Debug, Default, PartialEq, Eq)]
struct GameLog {
    /// The mod.io storage folders it loaded Mod Hub mods from, each once.
    storage: Vec<PathBuf>,
    /// For each mod it found twice: the id, and the folder of the copy it
    /// chose.
    chosen: Vec<(String, PathBuf)>,
}

/// What the game's `stdout.txt` at `log` says (SEEN, build 40425):
///
/// - it names each mod's folder as it loads it, `Reading content cache
///   from json for dir '<storage>/10640/mods/<mod.io id>/content/'` for a
///   Mod Hub mod;
/// - for a mod found twice, `Multiple (2) mods with same id found <id>, the
///   one from <folder>/ has been selected`.
fn read_game_log(log: &Path) -> GameLog {
    let mut out = GameLog::default();
    let Ok(file) = fs::File::open(log) else {
        return out;
    };
    let mut bytes = Vec::new();
    if file.take(LOG_READ).read_to_end(&mut bytes).is_err() {
        return out;
    }
    for line in String::from_utf8_lossy(&bytes).lines() {
        if let Some((_, rest)) = line.split_once("Reading content cache from json for dir '") {
            if let Some((dir, _)) = rest.split_once('\'')
                && let Some(storage) = storage_of(dir)
                && !out.storage.iter().any(|known| same_folder(known, &storage))
            {
                out.storage.push(storage);
            }
        } else if let Some((_, rest)) = line.split_once("mods with same id found ")
            && let Some((id, rest)) = rest.split_once(", the one from ")
            && let Some(folder) = rest.trim_end().strip_suffix(" has been selected")
        {
            let folder = PathBuf::from(folder);
            if folder.is_absolute() {
                out.chosen.push((id.to_owned(), folder));
            }
        }
    }
    out
}

/// The mod.io storage folder of a Mod Hub mod's content folder
/// `<storage>/10640/mods/<mod.io id>/content/`, if `dir` is one and its
/// storage folder an absolute path.
fn storage_of(dir: &str) -> Option<PathBuf> {
    let dir = dir.trim_end_matches(['/', '\\']);
    let mut parts = dir.rsplitn(5, ['/', '\\']);
    let (content, id, mods, game, storage) = (
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
    );
    let shaped = content == "content"
        && !id.is_empty()
        && id.bytes().all(|b| b.is_ascii_digit())
        && mods == "mods"
        && game == MODIO_GAME.to_string();
    let storage = PathBuf::from(storage);
    (shaped && storage.is_absolute()).then_some(storage)
}

/// Whether two paths name the same folder: the same folder on disk where
/// both are there, else as written but for the kind of separators, a
/// trailing one and, on Windows, the case.
fn same_folder(a: &Path, b: &Path) -> bool {
    if let (Ok(a), Ok(b)) = (fs::canonicalize(a), fs::canonicalize(b)) {
        return a == b;
    }
    let norm = |path: &Path| {
        let text = path.to_string_lossy().replace('\\', "/");
        let text = text.trim_end_matches('/');
        if cfg!(windows) {
            text.to_lowercase()
        } else {
            text.to_owned()
        }
    };
    norm(a) == norm(b)
}

#[cfg(not(windows))]
fn dirs_local() -> Option<PathBuf> {
    // $XDG_DATA_HOME or ~/.local/share on Linux; ~/Library/Application
    // Support on macOS.
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| {
                let home = PathBuf::from(home);
                if cfg!(target_os = "macos") {
                    home.join("Library/Application Support")
                } else {
                    home.join(".local/share")
                }
            })
        })
}

/// One mod found in a root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Its id, from `mod.json`, else the folder's name.
    pub id: String,
    /// Its folder's name.
    pub folder: String,
    pub path: PathBuf,
    /// Its `revision`, from `mod.json`.
    pub revision: Option<i64>,
    /// For a Mod Hub download: which mod and file it is.
    pub hub: Option<HubFile>,
    /// The copy the game loads of a mod found more than once is not known
    /// ([`settle`]): no version can be vouched for.
    pub unsure: bool,
}

/// A Mod Hub (mod.io) download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HubFile {
    /// The mod's mod.io number: its folder's name, a save's hub id.
    pub mod_id: u64,
    /// The file mod.io's cache says is installed; `None` when its index does
    /// not say so for certain (no index, no entry, or not installed).
    pub file: Option<u64>,
}

/// The state mod.io's cache index gives an installed mod (SEEN on one PC,
/// 2026-10-04: every one of 58 installed mods).
const MODIO_INSTALLED: u64 = 1;

/// The installed file of each mod in the mod.io cache index `state.json`'s
/// text, by mod.io number; a mod not installed, or whose entry does not
/// read, has none.
pub fn modio_files(state: &str) -> HashMap<u64, Option<u64>> {
    let Ok(index) = serde_json::from_str::<serde_json::Value>(state) else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for entry in index
        .get("Mods")
        .and_then(|m| m.as_array())
        .into_iter()
        .flatten()
    {
        let Some(id) = entry.get("ID").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        let installed =
            entry.get("State").and_then(serde_json::Value::as_u64) == Some(MODIO_INSTALLED);
        let file = entry
            .pointer("/Profile/modfile/id")
            .and_then(serde_json::Value::as_u64)
            .filter(|_| installed);
        out.insert(id, file);
    }
    out
}

/// Every mod in `roots`: each folder with a `mod.json`, in the roots'
/// order, then by folder name.
pub fn installed(roots: &[Root]) -> Vec<Found> {
    let mut out = Vec::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(&root.path) else {
            continue;
        };
        // A Mod Hub folder's index is `metadata\state.json` beside it.
        let modio = root.hub.is_some().then(|| {
            root.path
                .parent()
                .map(|game| game.join("metadata").join("state.json"))
                .and_then(|state| fs::read_to_string(state).ok())
                .map(|state| modio_files(&state))
                .unwrap_or_default()
        });
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.join("mod.json").is_file())
            .collect();
        dirs.sort();
        for path in dirs {
            let folder = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let manifest = fs::read_to_string(path.join("mod.json"))
                .ok()
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
            let id = manifest
                .as_ref()
                .and_then(|v| v.get("modId").and_then(|id| id.as_str()).map(str::to_owned))
                .unwrap_or_else(|| folder.clone());
            let revision = manifest
                .as_ref()
                .and_then(|v| v.get("revision").and_then(serde_json::Value::as_i64));
            let hub = modio.as_ref().and_then(|files| {
                let mod_id = folder.parse::<u64>().ok()?;
                Some(HubFile {
                    mod_id,
                    file: files.get(&mod_id).copied().flatten(),
                })
            });
            out.push(Found {
                id,
                folder,
                path,
                revision,
                hub,
                unsure: false,
            });
        }
    }
    out
}

/// The folder of the mod named `name`, by id first, then by folder name.
pub fn find<'a>(found: &'a [Found], name: &str) -> Option<&'a Found> {
    found
        .iter()
        .find(|f| f.id == name)
        .or_else(|| found.iter().find(|f| f.folder == name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn mods_are_found_in_mod_hub_steam_and_the_game_folders() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("public");
        let data = dir.path().join("data");
        let steam = dir.path().join("steam");
        let game = dir.path().join("game");
        write(
            &public.join("mod.io/10640/mods/6414521/mod.json"),
            r#"{"modId": "revyn112_towns_de"}"#,
        );
        write(
            &public.join("mod.io/10640/metadata/state.json"),
            r#"{"Mods": [{"ID": 6414521, "State": 1, "Profile": {"modfile": {"id": 8264750}}}], "version": 1}"#,
        );
        write(
            &data.join("mod.io/10640/mods/6037864/mod.json"),
            r#"{"modId": "celmi_timetables"}"#,
        );
        write(
            &steam.join("userdata/42/3493540/local/staging_area/gw_big_city_1/mod.json"),
            "{}",
        );
        write(
            &game.join("dlcs/urbangames_preorder_pack/mod.json"),
            r#"{"modId": "urbangames_preorder_pack"}"#,
        );
        // The game's built-in mods, in a folder of their own; once taken
        // for not a mod, as it has no mod.json itself (2026-10-04).
        write(
            &game.join("mods/release/urbangames_no_costs/mod.json"),
            r#"{"modId": "urbangames_no_costs_1"}"#,
        );
        // Not a mod: no mod.json.
        fs::create_dir_all(game.join("mods/empty")).unwrap();

        let hub = |source, storage: PathBuf| HubRoot {
            source,
            storage: storage.join("mod.io"),
        };
        let roots = roots(
            Some(&game),
            std::slice::from_ref(&steam),
            &[
                hub(HubSource::Default, public),
                hub(HubSource::LocalAppData, data),
                hub(HubSource::Settings, dir.path().join("none")),
            ],
        );
        assert_eq!(roots.len(), 6, "{roots:?}");
        let found = installed(&roots);
        let ids: Vec<&str> = found.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "revyn112_towns_de",
                "celmi_timetables",
                "gw_big_city_1",
                "urbangames_no_costs_1",
                "urbangames_preorder_pack"
            ]
        );
        assert_eq!(
            find(&found, "urbangames_no_costs_1").unwrap().folder,
            "urbangames_no_costs"
        );
        assert_eq!(find(&found, "celmi_timetables").unwrap().folder, "6037864");
        assert_eq!(
            find(&found, "revyn112_towns_de").unwrap().hub,
            Some(HubFile {
                mod_id: 6414521,
                file: Some(8264750)
            })
        );
        // In the cache but not in its index: a Mod Hub mod of no known file.
        assert_eq!(
            find(&found, "celmi_timetables").unwrap().hub,
            Some(HubFile {
                mod_id: 6037864,
                file: None
            })
        );
        assert_eq!(find(&found, "gw_big_city_1").unwrap().hub, None);
        assert_eq!(find(&found, "6037864").unwrap().id, "celmi_timetables");
        assert!(find(&found, "missing").is_none());
    }

    /// Two downloads of one revision are told apart by the file mod.io
    /// installed; one whose download is not finished has no file.
    #[test]
    fn the_mod_io_index_gives_each_installed_mods_file() {
        let files = modio_files(
            r#"{"Mods": [
                {"ID": 1, "State": 1, "Profile": {"modfile": {"id": 11}}},
                {"ID": 2, "State": 3, "Profile": {"modfile": {"id": 22}}},
                {"ID": 3, "State": 1, "Profile": {}},
                {"State": 1, "Profile": {"modfile": {"id": 44}}}
            ], "version": 1}"#,
        );
        assert_eq!(files.get(&1), Some(&Some(11)));
        assert_eq!(files.get(&2), Some(&None), "not installed: no file");
        assert_eq!(files.get(&3), Some(&None));
        assert_eq!(files.len(), 3);
        assert!(modio_files("not json").is_empty());
    }

    /// A mod.io storage folder at `storage` holding the download of
    /// `modio_id`, named `mod_id`, with `file` installed per its index.
    fn download(storage: &Path, modio_id: u64, mod_id: &str, file: u64) {
        write(
            &storage.join(format!("10640/mods/{modio_id}/mod.json")),
            &format!(r#"{{"modId": "{mod_id}", "revision": 1}}"#),
        );
        write(
            &storage.join("10640/metadata/state.json"),
            &format!(
                r#"{{"Mods": [{{"ID": {modio_id}, "State": 1, "PathOnDisk": "x", "Profile": {{"modfile": {{"id": {file}}}}}}}], "version": 1}}"#
            ),
        );
    }

    /// `path` as JSON names it.
    fn json_path(path: &Path) -> String {
        serde_json::to_string(&path.to_string_lossy()).unwrap()
    }

    /// The 2026-10-25 report: an owner's Mod Hub mods were all "Missing",
    /// as his downloads were neither in the users' shared folder nor in
    /// the local app data folder. Another game on mod.io's SDK (Sins of a
    /// Solar Empire 2) had written mod.io's global settings, shared by
    /// every such game, with its own folder, in this shape: forward
    /// slashes, no trailing one, a folder not named `mod.io`, and the game
    /// id joined to it directly. The downloads are found there, versioned
    /// by the file mod.io installed; the default folder, there and empty
    /// on his PC, is not looked in, nor what was left in it.
    #[test]
    fn mod_hub_downloads_are_found_where_mod_io_settings_move_them() {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("AppData/Local");
        let moved = local.join("sins2/mods/modio");
        let public = dir.path().join("Public/mod.io");
        download(&moved, 6414581, "bobdike_achievements_unbound", 9001);
        write(
            &local.join("mod.io/globalsettings.json"),
            &format!(
                r#"{{ "RootLocalStoragePath": "{}" }}"#,
                moved.to_string_lossy().replace('\\', "/")
            ),
        );
        // Left behind in the default folder: a mod the game no longer has.
        download(&public, 6037864, "celmi_timetables", 8001);

        let search = hub_roots_in(Some(&local), Some(public.clone()), &[]);
        assert!(same_folder(&search.roots[0].storage, &moved));
        assert_eq!(
            search.roots[1..],
            [
                HubRoot {
                    source: HubSource::Default,
                    storage: public
                },
                HubRoot {
                    source: HubSource::LocalAppData,
                    storage: local.join("mod.io")
                },
            ]
        );
        let found = installed(&roots(None, &[], &search.used()));
        let achievements = find(&found, "bobdike_achievements_unbound").unwrap();
        assert_eq!(
            achievements.hub,
            Some(HubFile {
                mod_id: 6414581,
                file: Some(9001)
            }),
            "a Mod Hub download wherever it is kept, not a local mod"
        );
        assert_eq!(achievements.revision, Some(1));
        assert!(find(&found, "celmi_timetables").is_none());

        // What the log says survives diagnostics: no path, but which place
        // held the downloads, and how many.
        assert_eq!(
            search.report(&found),
            [
                "mod.io's settings (globalsettings.json): names a folder",
                "Mod Hub's downloads, place 1 (mod.io's settings (globalsettings.json)): looked in: 1 mods, 1 of them with the file mod.io installed",
                "Mod Hub's downloads, place 2 (mod.io's default folder): there, but mod.io uses place 1: not looked in",
                "Mod Hub's downloads, place 3 (the local app data folder): not there, but mod.io uses place 1: not looked in",
            ]
        );
    }

    /// A folder mod.io's settings name that is not there (a drive not
    /// connected, a folder chosen that the game has not used yet) is not
    /// looked past: the game will not load what the default folder holds.
    #[test]
    fn a_named_folder_that_is_not_there_is_not_looked_past() {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("AppData/Local");
        let public = dir.path().join("Public/mod.io");
        download(&public, 6414521, "revyn112_towns_de", 8264750);
        write(
            &local.join("mod.io/globalsettings.json"),
            &format!(
                r#"{{"RootLocalStoragePath": {}}}"#,
                json_path(&dir.path().join("X/modio"))
            ),
        );
        let search = hub_roots_in(Some(&local), Some(public), &[]);
        assert!(search.used().is_empty(), "{search:?}");
        assert_eq!(
            search.report(&[])[1],
            "Mod Hub's downloads, place 1 (mod.io's settings (globalsettings.json)): not there"
        );
    }

    /// Without a setting, the default folder is the one; before this
    /// change, the only one looked in besides the local app data folder.
    #[test]
    fn without_a_setting_the_default_folder_is_looked_in() {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("AppData/Local");
        let public = dir.path().join("Public/mod.io");
        download(&public, 6414521, "revyn112_towns_de", 8264750);
        let search = hub_roots_in(Some(&local), Some(public), &[]);
        assert_eq!(
            search.notes,
            ["mod.io's settings (globalsettings.json): none"]
        );
        let found = installed(&roots(None, &[], &search.used()));
        assert_eq!(
            find(&found, "revyn112_towns_de").unwrap().hub.unwrap().file,
            Some(8264750)
        );
    }

    /// A user's own setting comes before the global one, as the SDK reads
    /// them; a setting that is no absolute path is passed over, and said
    /// so; the rest of a `user.json` (its login) is not kept.
    #[test]
    fn a_users_mod_io_setting_comes_first_and_odd_settings_are_passed_over() {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("AppData/Local");
        let mine = dir.path().join("E/mine");
        let global = dir.path().join("D/global");
        let public = dir.path().join("Public/mod.io");
        download(&mine, 1, "towns", 12);
        download(&global, 1, "towns", 11);
        download(&public, 1, "towns", 10);
        write(
            &local.join("mod.io/10640/481042991/user.json"),
            &format!(
                r#"{{"OAuth": {{"Token": "secret"}}, "RootLocalStoragePath": {}}}"#,
                json_path(&mine)
            ),
        );
        write(
            &local.join("mod.io/10640/7/user.json"),
            r#"{"RootLocalStoragePath": "relative/mods"}"#,
        );
        write(
            &local.join("mod.io/10640/8/user.json"),
            r#"{"OAuth": {"Token": "secret"}}"#,
        );
        write(
            &local.join("mod.io/globalsettings.json"),
            &format!(r#"{{"RootLocalStoragePath": {}}}"#, json_path(&global)),
        );

        let search = hub_roots_in(Some(&local), Some(public), &[]);
        let sources: Vec<HubSource> = search.roots.iter().map(|r| r.source).collect();
        assert_eq!(
            sources,
            [
                HubSource::UserOverride,
                HubSource::Settings,
                HubSource::Default,
                HubSource::LocalAppData
            ]
        );
        assert_eq!(
            search.notes,
            [
                "a user's mod.io settings (user.json) (user 1): names a folder",
                "a user's mod.io settings (user.json) (user 2): names a folder that is no absolute path: passed over",
                "mod.io's settings (globalsettings.json): names a folder",
            ]
        );
        assert!(!format!("{search:?}").contains("secret"));
        let used = search.used();
        assert_eq!(used.len(), 1, "{used:?}");
        let found = installed(&roots(None, &[], &used));
        assert_eq!(find(&found, "towns").unwrap().hub.unwrap().file, Some(12));
    }

    #[test]
    fn a_mod_io_setting_that_does_not_read_moves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("globalsettings.json");
        for (text, setting) in [
            ("", Setting::Unreadable),
            ("not json", Setting::Unreadable),
            ("[]", Setting::Unreadable),
            ("{}", Setting::NoFolder),
            (r#"{"RootLocalStoragePath": ""}"#, Setting::NoFolder),
            (r#"{"RootLocalStoragePath": 3}"#, Setting::NoFolder),
            (r#"{"RootLocalStoragePath": "mods/"}"#, Setting::NotAbsolute),
        ] {
            fs::write(&file, text).unwrap();
            assert_eq!(storage_setting(&file), setting, "{text:?}");
        }
        assert_eq!(
            storage_setting(&dir.path().join("none.json")),
            Setting::Missing
        );
    }

    /// The game's log names each Mod Hub mod's folder as it loads it: the
    /// folders it loaded them from are looked in, each once, whatever the
    /// settings say; and it says which copy it chose of a mod it found
    /// twice.
    #[test]
    fn the_game_log_names_the_folders_it_loaded_mod_hub_mods_from() {
        let dir = tempfile::tempdir().unwrap();
        let steam = dir.path().join("Steam");
        let storage = dir.path().join("F/Mod Hub");
        download(&storage, 6416940, "parallel_tracks_1", 77);
        let slashed = storage.to_string_lossy().replace('\\', "/");
        let local = steam.join("userdata/42/3493540/local");
        let staged = format!(
            "{}/staging_area/jean_luc_picard_automatic_day_mode/",
            local.to_string_lossy().replace('\\', "/")
        );
        let log = local.join("crash_dump/stdout.txt");
        let at = "[2026-10-25 15:37:34Z - MESSAGE  - Main             - UpdateBaseFileS]  ";
        write(
            &log,
            &format!(
                "[2026-10-25 15:37:34Z - MESSAGE  - Main             - Main           ]  mod.io backend created for Steam platform\r\n\
                 [2026-10-25 15:37:34Z - WARNING  - Main             - ModRep         ]  Multiple (2) mods with same id found jean_luc_picard_automatic_day_mode, the one from {staged} has been selected\r\n\
                 {at}Reading content cache from json for dir '{slashed}/10640/mods/6416940/content/'\r\n\
                 {at}Reading content cache from json for dir '{slashed}/10640/mods/6037864/content/'\r\n\
                 {at}Reading content cache from json for dir 'mods/release/urbangames_campaign/content/'\r\n\
                 {at}Reading content cache from json for dir '{slashed}/local/staging_area/tpf3mp_1/content/'\r\n"
            ),
        );

        let search = hub_roots_in(None, None, std::slice::from_ref(&steam));
        assert_eq!(search.roots.len(), 1, "{search:?}");
        assert_eq!(search.roots[0].source, HubSource::GameLog);
        assert!(same_folder(&search.roots[0].storage, &storage));
        assert_eq!(
            search.chosen,
            [(
                "jean_luc_picard_automatic_day_mode".to_owned(),
                PathBuf::from(staged)
            )]
        );
        // What mod.io uses is known only from its own places: a folder
        // the game last loaded from is said, not looked in.
        assert!(search.used().is_empty());
        assert_eq!(
            search.report(&[]),
            [
                "Mod Hub's downloads, place 1 (the game's log (stdout.txt)): the game last loaded Mod Hub mods from here: not looked in"
            ]
        );
        // Where it is mod.io's folder too, the log bears it out.
        let both = HubSearch {
            roots: [
                vec![HubRoot {
                    source: HubSource::Default,
                    storage: storage.clone(),
                }],
                search.roots.clone(),
            ]
            .concat(),
            ..HubSearch::default()
        };
        let found = installed(&roots(None, &[], &both.used()));
        assert_eq!(
            find(&found, "parallel_tracks_1").unwrap().hub.unwrap().file,
            Some(77)
        );
        assert_eq!(
            both.report(&found)[1],
            "Mod Hub's downloads, place 2 (the game's log (stdout.txt)): the same folder as place 1"
        );
    }

    #[test]
    fn a_logged_folder_is_a_mod_hub_mods_content_folder_only() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_string_lossy().replace('\\', "/");
        assert!(same_folder(
            &storage_of(&format!("{base}/mod.io/10640/mods/6432550/content/")).unwrap(),
            &dir.path().join("mod.io")
        ));
        assert!(storage_of(&format!("{base}/mod.io/10640/mods/6432550/content")).is_some());
        for not_one in [
            format!("{base}/mod.io/10640/mods/tpf3mp_1/content/"),
            format!("{base}/mod.io/10641/mods/6432550/content/"),
            format!("{base}/mod.io/10640/mods/6432550/res/"),
            "mod.io/10640/mods/6432550/content/".to_owned(),
            "10640/mods/6432550/content/".to_owned(),
            "mods/release/urbangames_campaign/content/".to_owned(),
        ] {
            assert_eq!(storage_of(&not_one), None, "{not_one}");
        }
    }

    #[test]
    fn one_folder_named_twice_is_looked_in_once() {
        let dir = tempfile::tempdir().unwrap();
        let public = dir.path().join("Public").join("mod.io");
        download(&public, 1, "towns", 10);
        let slashed = format!("{}/", public.to_string_lossy().replace('\\', "/"));
        let search = HubSearch {
            roots: vec![
                HubRoot {
                    source: HubSource::Settings,
                    storage: PathBuf::from(slashed),
                },
                HubRoot {
                    source: HubSource::Default,
                    storage: public,
                },
            ],
            ..HubSearch::default()
        };
        let roots = roots(None, &[], &search.used());
        assert_eq!(roots.len(), 1, "{roots:?}");
        let found = installed(&roots);
        assert_eq!(
            search.report(&found)[1],
            "Mod Hub's downloads, place 2 (mod.io's default folder): the same folder as place 1"
        );
    }

    /// The 2026-10-25 report's other side: the owner had two of his own
    /// mods both as Mod Hub downloads and in his staging area, and the game
    /// loaded the staged ones. The copy the game's log names counts; where
    /// it names none, which one the game loads is not known.
    #[test]
    fn a_mod_found_twice_counts_as_the_copy_the_game_chose_or_is_unsure() {
        let dir = tempfile::tempdir().unwrap();
        let storage = dir.path().join("mod.io");
        let steam = dir.path().join("Steam");
        let staging = steam.join("userdata/42/3493540/local/staging_area");
        let game = dir.path().join("game");
        download(&storage, 6400001, "day_mode", 5);
        write(
            &staging.join("day_mode/mod.json"),
            r#"{"modId": "day_mode", "revision": 8}"#,
        );
        // The game's own, in two of its folders: not a doubt.
        for folder in ["mods/mw_et31", "mods/release/mw_et31"] {
            write(
                &game.join(folder).join("mod.json"),
                r#"{"modId": "mw_et31", "revision": 1}"#,
            );
        }
        let hub = [HubRoot {
            source: HubSource::Default,
            storage,
        }];
        let found = installed(&roots(Some(&game), std::slice::from_ref(&steam), &hub));
        assert_eq!(find(&found, "day_mode").unwrap().revision, Some(1));

        let unsure = settle(found.clone(), &[]);
        let day_mode = find(&unsure, "day_mode").unwrap();
        assert!(day_mode.unsure && day_mode.hub.is_some());
        assert!(!find(&unsure, "mw_et31").unwrap().unsure);

        let chosen = settle(
            found.clone(),
            &[("day_mode".to_owned(), staging.join("day_mode/"))],
        );
        let day_mode = find(&chosen, "day_mode").unwrap();
        assert!(!day_mode.unsure);
        assert_eq!((day_mode.revision, day_mode.hub), (Some(8), None));
        assert_eq!(chosen.len(), found.len());

        // A log naming a folder that is no copy settles nothing, nor do
        // two logs (two Steam accounts' games) naming different copies.
        let elsewhere = settle(
            found.clone(),
            &[("day_mode".to_owned(), dir.path().join("x"))],
        );
        assert!(find(&elsewhere, "day_mode").unwrap().unsure);
        let hub_copy = find(&found, "day_mode").unwrap().path.clone();
        let both = settle(
            found,
            &[
                ("day_mode".to_owned(), staging.join("day_mode")),
                ("day_mode".to_owned(), hub_copy),
            ],
        );
        assert!(find(&both, "day_mode").unwrap().unsure);
    }
}

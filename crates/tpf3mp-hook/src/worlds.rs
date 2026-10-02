//! Saving and loading whole worlds in the game (docs/HOOKS.md, "The room's
//! world"): the files, and what the hook asks of the mod's GUI for them.
//!
//! The game saves and loads through its own script API, which only the
//! GUI's Lua can call (`app.saveGame`, `app.loadGame`), and only into and
//! from its own save folder: `<Steam>/userdata/<account>/3493540/local/save`.
//! So a save the room orders is made under a name of this game's own
//! (`tpf3mp_<pid>_<event>`, as two games on one PC share the folder), found
//! there once the GUI says it is written, and moved to the room's file; the
//! picture the game writes beside it is removed. A world the room hands
//! over is copied into the folder as `tpf3mp_room_<pid>` and loaded from
//! there, by the GUI of the world the game has up, or by the game's main menu
//! when it has none (`crate::menu`); it stays until the next one replaces
//! it.
//!
//! Each game has its own process id, so each game played leaves its own
//! copy, a whole world each. [`sweep`] removes those of games no longer
//! running: when the hook starts, and each time a new copy is written
//! (docs/HOOKS.md, "The room's world").

use std::{
    fs,
    path::{Path, PathBuf},
};

use tpf3mp_bridge::Notice;
use tpf3mp_proto::PlayerId;

use crate::{
    lua,
    step::{GameControl, LoadFrom},
};

/// Transport Fever 3's Steam app.
pub const STEAM_APP: &str = "3493540";

/// The game's side of saves and loads, through the mod's GUI.
pub struct GuiWorlds {
    folder: Result<PathBuf, String>,
    tag: String,
}

impl GuiWorlds {
    /// For the game's own save folder, as Steam names it.
    pub fn in_steam_folder() -> Self {
        Self::in_folder(save_folder())
    }

    /// For the save folder `folder`, or why there is none. Removes the
    /// copies games no longer running left there ([`sweep`]).
    pub fn in_folder(folder: Result<PathBuf, String>) -> Self {
        let worlds = Self {
            folder,
            tag: std::process::id().to_string(),
        };
        worlds.sweep();
        worlds
    }

    /// Removes the copies of games no longer running from the save folder,
    /// and logs what it removed.
    fn sweep(&self) {
        let Ok(folder) = &self.folder else {
            return;
        };
        let swept = sweep(folder, std::process::id(), running);
        for line in swept.lines() {
            crate::install::log_line(&line);
        }
    }
}

/// What [`sweep`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Swept {
    /// The files removed, by name.
    pub removed: Vec<String>,
    /// Their bytes.
    pub bytes: u64,
    /// The files that could not be removed, with why.
    pub failed: Vec<(String, String)>,
}

impl Swept {
    /// For the hook's log.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if !self.removed.is_empty() {
            lines.push(format!(
                "removed {} room save copies of games no longer running from the save folder ({} MB): {}",
                self.removed.len(),
                self.bytes / 1_000_000,
                self.removed.join(", ")
            ));
        }
        for (name, why) in &self.failed {
            lines.push(format!(
                "could not remove the old room save copy {name}: {why}"
            ));
        }
        lines
    }
}

/// The game whose copy the save folder's file `name` is, if it is one the
/// hook writes, by its exact name: `tpf3mp_room_<pid>.sav` (a room's world,
/// copied in to load) or `tpf3mp_<pid>_<event>.sav` (a save the room
/// ordered, before it moves out), or the `.jpg` picture of either. Any
/// other name, the player's saves', is not.
pub fn copy_of(name: &str) -> Option<u32> {
    let stem = name
        .strip_suffix(".sav")
        .or_else(|| name.strip_suffix(".jpg"))?;
    let rest = stem.strip_prefix("tpf3mp_")?;
    if let Some(pid) = rest.strip_prefix("room_") {
        return pid_of(pid);
    }
    let (pid, event) = rest.split_once('_')?;
    number(event, 20)?;
    pid_of(pid)
}

/// A process id as `format!` writes one: decimal digits, no leading zero.
fn pid_of(text: &str) -> Option<u32> {
    number(text, 10)?
        .try_into()
        .ok()
        .filter(|pid: &u32| *pid != 0)
}

/// A number of at most `digits` decimal digits, as `format!` writes it.
fn number(text: &str, digits: usize) -> Option<u64> {
    let plain = !text.is_empty()
        && text.len() <= digits
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    if plain { text.parse().ok() } else { None }
}

/// Removes from the save folder `folder` the copies ([`copy_of`]) of games
/// no longer running: files only, by their exact names, never those of
/// this game (`own`), whose copy is the world it loads or plays, nor those
/// of a process `running` says may run, another game sharing the folder.
/// Everything else, the player's saves among them, stays. A file that
/// cannot be removed stays too, and is said so.
pub fn sweep(folder: &Path, own: u32, running: impl Fn(u32) -> bool) -> Swept {
    let mut swept = Swept::default();
    let Ok(entries) = fs::read_dir(folder) else {
        return swept;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(pid) = copy_of(&name) else {
            continue;
        };
        if pid == own || running(pid) {
            continue;
        }
        // Not through a link, and not a folder.
        let Ok(meta) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !meta.file_type().is_file() {
            continue;
        }
        match fs::remove_file(entry.path()) {
            Ok(()) => {
                swept.bytes += meta.len();
                swept.removed.push(name);
            }
            Err(error) => swept.failed.push((name, error.to_string())),
        }
    }
    swept
}

/// Whether process `pid` may be running. Fails closed: a process that
/// cannot be asked about is taken as running.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn running(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, GetLastError},
        System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    /// The exit code of a process still running.
    const STILL_ACTIVE: u32 = 259;
    // SAFETY: a handle opened for querying only, closed once.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            // No such process; anything else (access denied) may be one.
            return GetLastError() != ERROR_INVALID_PARAMETER;
        }
        let mut code = 0u32;
        let asked = GetExitCodeProcess(handle, &raw mut code);
        CloseHandle(handle);
        asked == 0 || code == STILL_ACTIVE
    }
}

/// Whether process `pid` may be running. Fails closed: without `/proc`,
/// every process is taken as running.
#[cfg(unix)]
pub fn running(pid: u32) -> bool {
    let proc = Path::new("/proc");
    !proc.join("self").exists() || proc.join(pid.to_string()).exists()
}

/// Whether process `pid` may be running: here, always.
#[cfg(not(any(windows, unix)))]
pub fn running(_pid: u32) -> bool {
    true
}

impl GameControl for GuiWorlds {
    fn request_save(&mut self, name: &str) {
        lua::request_save(name);
    }

    fn save_result(&mut self) -> Option<Result<PathBuf, String>> {
        let answer = lua::take_save_answer()?;
        let result = answer.and_then(|name| {
            let folder = self.folder.clone()?;
            let file = folder.join(format!("{name}.sav"));
            // The picture beside it is the player's save menu's, not the
            // room's.
            let _ = fs::remove_file(folder.join(format!("{name}.jpg")));
            if file.is_file() {
                Ok(file)
            } else {
                Err(format!(
                    "the game says it saved {name}, but {} is not there",
                    file.display()
                ))
            }
        });
        if result.is_ok() {
            self.sweep();
        }
        Some(result)
    }

    fn room_notice(&mut self, notice: &Notice) {
        lua::notice(notice);
    }

    fn set_me(&mut self, player: PlayerId) {
        lua::set_me(player);
    }

    fn set_mods(&mut self, mods: Option<tpf3mp_bridge::ModLists>) {
        lua::set_mods(mods);
    }

    fn request_load(&mut self, file: &Path, from: LoadFrom) -> Result<(), String> {
        let folder = self.folder.clone()?;
        let name = format!("tpf3mp_room_{}", self.tag);
        let target = folder.join(format!("{name}.sav"));
        fs::copy(file, &target).map_err(|error| {
            format!(
                "copying the room's save {} to {}: {error}",
                file.display(),
                target.display()
            )
        })?;
        self.sweep();
        match from {
            LoadFrom::Gui => lua::request_load(&name),
            LoadFrom::Menu => lua::request_menu_load(&name),
        }
        Ok(())
    }

    fn load_done(&mut self) -> bool {
        lua::load_done()
    }

    fn load_failed(&mut self) -> Option<String> {
        lua::take_load_failure()
    }

    fn world_up(&mut self) -> Option<u64> {
        lua::take_world_up()
    }
}

/// The save folder of the Steam account playing: Steam's folder and the
/// account it runs as, from the registry; without an account, the one
/// account with a save folder for the game.
fn save_folder() -> Result<PathBuf, String> {
    let root = steam_root().ok_or("cannot find Steam's folder")?;
    let userdata = root.join("userdata");
    if let Some(account) = active_account().filter(|account| *account != 0) {
        let folder = save_folder_of(&userdata.join(account.to_string()));
        if folder.is_dir() {
            return Ok(folder);
        }
    }
    let mut found: Vec<PathBuf> = fs::read_dir(&userdata)
        .map_err(|error| format!("reading {}: {error}", userdata.display()))?
        .flatten()
        .map(|account| save_folder_of(&account.path()))
        .filter(|folder| folder.is_dir())
        .collect();
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(format!(
            "no Steam account under {} has a save folder for the game",
            userdata.display()
        )),
        _ => Err("several Steam accounts have a save folder for the game, and Steam names none as playing".into()),
    }
}

fn save_folder_of(account: &Path) -> PathBuf {
    account.join(STEAM_APP).join("local").join("save")
}

#[cfg(windows)]
fn steam_root() -> Option<PathBuf> {
    registry::string(r"Software\Valve\Steam", "SteamPath")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .or_else(|| {
            let base = std::env::var_os("ProgramFiles(x86)")?;
            Some(PathBuf::from(base).join("Steam")).filter(|path| path.is_dir())
        })
}

#[cfg(windows)]
fn active_account() -> Option<u32> {
    registry::dword(r"Software\Valve\Steam\ActiveProcess", "ActiveUser")
}

#[cfg(not(windows))]
fn steam_root() -> Option<PathBuf> {
    None
}

#[cfg(not(windows))]
fn active_account() -> Option<u32> {
    None
}

/// The current user's registry values Steam writes.
#[cfg(windows)]
#[allow(unsafe_code)]
mod registry {
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    pub fn string(key: &str, value: &str) -> Option<String> {
        let (key, value) = (wide(key), wide(value));
        let mut buffer = vec![0u16; 1024];
        let mut size = u32::try_from(buffer.len() * 2).ok()?;
        // SAFETY: both names end in a NUL, and the buffer holds `size`
        // bytes, which RegGetValueW writes back.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &raw mut size,
            )
        };
        if status != 0 {
            return None;
        }
        let units = (size as usize / 2).min(buffer.len());
        let text = String::from_utf16_lossy(&buffer[..units]);
        let text = text.trim_end_matches('\0');
        (!text.is_empty()).then(|| text.to_owned())
    }

    pub fn dword(key: &str, value: &str) -> Option<u32> {
        let (key, value) = (wide(key), wide(value));
        let mut data = 0u32;
        let mut size = 4u32;
        // SAFETY: as above, with room for one DWORD.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&raw mut data).cast(),
                &raw mut size,
            )
        };
        (status == 0).then_some(data)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::PoisonError;

    use super::*;
    use crate::lua::tests::{Lua, SERIAL};

    fn folder(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tpf3mp-worlds-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A process id no process has: not a multiple of 4 on Windows, above
    /// any `pid_max` on Linux.
    const GONE: u32 = u32::MAX - 1;

    #[test]
    fn only_the_hooks_own_names_are_room_copies() {
        assert_eq!(copy_of("tpf3mp_room_41856.sav"), Some(41856));
        assert_eq!(copy_of("tpf3mp_41856_21.sav"), Some(41856));
        assert_eq!(copy_of("tpf3mp_41856_21.jpg"), Some(41856));
        assert_eq!(copy_of("tpf3mp_room_7.jpg"), Some(7));
        assert_eq!(copy_of("tpf3mp_7_0.sav"), Some(7));
        assert_eq!(copy_of("tpf3mp_4294967295_1.sav"), Some(u32::MAX));
        for not_ours in [
            "two.sav",
            "MPGAME.sav",
            "autosave_two_1929-01-10.sav",
            "autosave_tpf3mp_room_7_1900-01-06.sav",
            "tpf3mp_room_7",
            "tpf3mp_room_7.sav.bak",
            "tpf3mp_room_7.lua",
            "tpf3mp_room_.sav",
            "tpf3mp_room_x.sav",
            "tpf3mp_room_07.sav",
            "tpf3mp_room_0.sav",
            "tpf3mp_room_-7.sav",
            "tpf3mp_room_+7.sav",
            "tpf3mp_room_4294967296.sav",
            "tpf3mp_room_7_1.sav",
            "TPF3MP_room_7.sav",
            "tpf3mp_room_7.SAV",
            " tpf3mp_room_7.sav",
            "tpf3mp_7.sav",
            "tpf3mp_7_.sav",
            "tpf3mp__1.sav",
            "tpf3mp_7_1_2.sav",
            "tpf3mp_7_01.sav",
            "tpf3mp_7_x.sav",
            "tpf3mp_mine_1.sav",
            "my tpf3mp_7_1.sav",
            "tpf3mp_7_1 copy.sav",
        ] {
            assert_eq!(copy_of(not_ours), None, "{not_ours}");
        }
    }

    #[test]
    fn the_sweep_removes_only_copies_of_games_no_longer_running() {
        let dir = folder("sweep");
        let own = 400;
        let live = 500;
        let files = [
            // Games gone: removed.
            "tpf3mp_room_41856.sav",
            "tpf3mp_41856_21.sav",
            "tpf3mp_41856_21.jpg",
            "tpf3mp_room_3760.sav",
            // This game's: the world it loads or plays, and its own save.
            "tpf3mp_room_400.sav",
            "tpf3mp_400_3.sav",
            // Another game's, still running.
            "tpf3mp_room_500.sav",
            "tpf3mp_500_9.sav",
            // The player's.
            "two.sav",
            "two.jpg",
            "autosave_two_1929-01-10.sav",
            "tpf3mp_room_07.sav",
            "tpf3mp_room_9.sav.bak",
            "TPF3MP_room_9.sav",
            "tpf3mp_mine_1.sav",
        ];
        for name in files {
            fs::write(dir.join(name), b"world").unwrap();
        }
        // A folder with a copy's name is not a copy.
        fs::create_dir(dir.join("tpf3mp_room_600.sav")).unwrap();
        let swept = sweep(&dir, own, |pid| pid == live);
        assert_eq!(
            swept.removed,
            [
                "tpf3mp_41856_21.jpg",
                "tpf3mp_41856_21.sav",
                "tpf3mp_room_3760.sav",
                "tpf3mp_room_41856.sav",
            ]
        );
        assert_eq!(swept.bytes, 20);
        assert!(swept.failed.is_empty());
        for kept in &files[4..] {
            assert!(dir.join(kept).is_file(), "{kept} is kept");
        }
        assert!(dir.join("tpf3mp_room_600.sav").is_dir());
        assert!(swept.lines()[0].contains("removed 4 room save copies"));
        // Nothing left to remove: nothing logged.
        let again = sweep(&dir, own, |pid| pid == live);
        assert_eq!(again, Swept::default());
        assert!(again.lines().is_empty());
        // No folder: nothing.
        assert_eq!(
            sweep(&dir.join("missing"), own, |_| false),
            Swept::default()
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn this_process_runs_and_no_process_has_an_impossible_id() {
        assert!(running(std::process::id()));
        assert!(!running(GONE));
    }

    #[test]
    fn the_hook_sweeps_when_it_starts_and_when_it_copies_a_rooms_world_in() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = folder("sweep-start");
        let gone = format!("tpf3mp_room_{GONE}.sav");
        let gone_save = format!("tpf3mp_{GONE}_4.sav");
        fs::write(dir.join(&gone), b"old world").unwrap();
        fs::write(dir.join(&gone_save), b"old save").unwrap();
        fs::write(dir.join("mine.sav"), b"the player's").unwrap();
        let mut worlds = GuiWorlds::in_folder(Ok(dir.clone()));
        assert!(!dir.join(&gone).exists(), "removed at start");
        assert!(!dir.join(&gone_save).exists(), "removed at start");
        assert!(dir.join("mine.sav").is_file());
        // A copy of a game that ended while this one ran.
        fs::write(dir.join(&gone), b"old world").unwrap();
        let room = dir.join("from-the-room.sav");
        fs::write(&room, b"the room's world").unwrap();
        let lua = Lua::new();
        lua.register();
        worlds.request_load(&room, LoadFrom::Gui).unwrap();
        assert!(!dir.join(&gone).exists(), "removed once a new copy is in");
        let own = dir.join(format!("tpf3mp_room_{}.sav", std::process::id()));
        assert_eq!(fs::read(&own).unwrap(), b"the room's world");
        assert!(room.is_file(), "the room's file is the agent's");
        assert!(dir.join("mine.sav").is_file());
        let _ = lua.run("return tpf3mp_native.poll()");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_save_is_found_in_the_folder_once_the_gui_says_it_is_written() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = folder("save");
        let lua = Lua::new();
        lua.register();
        let _ = lua::take_save_answer();
        let mut worlds = GuiWorlds::in_folder(Ok(dir.clone()));
        worlds.request_save("tpf3mp_1_5");
        assert_eq!(worlds.save_result(), None, "not answered yet");
        fs::write(dir.join("tpf3mp_1_5.sav"), b"world").unwrap();
        fs::write(dir.join("tpf3mp_1_5.jpg"), b"picture").unwrap();
        lua.run("tpf3mp_native.poll() tpf3mp_native.saved('tpf3mp_1_5', true)")
            .unwrap();
        assert_eq!(worlds.save_result(), Some(Ok(dir.join("tpf3mp_1_5.sav"))));
        assert!(
            !dir.join("tpf3mp_1_5.jpg").exists(),
            "the picture is not kept"
        );
        // A save the GUI claims but that is not there is a failure.
        worlds.request_save("tpf3mp_1_6");
        lua.run("tpf3mp_native.saved('tpf3mp_1_6', true)").unwrap();
        assert!(
            worlds
                .save_result()
                .unwrap()
                .unwrap_err()
                .contains("is not there")
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_rooms_save_is_copied_in_and_the_gui_asked_to_load_it() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = folder("load");
        let room = dir.join("from-the-room.sav");
        fs::write(&room, b"the room's world").unwrap();
        let lua = Lua::new();
        lua.register();
        let mut worlds = GuiWorlds::in_folder(Ok(dir.clone()));
        worlds.request_load(&room, LoadFrom::Gui).unwrap();
        let name = format!("tpf3mp_room_{}", std::process::id());
        assert_eq!(
            fs::read(dir.join(format!("{name}.sav"))).unwrap(),
            b"the room's world"
        );
        assert_eq!(lua.run("return tpf3mp_native.poll().load"), Ok(name));
        // Without a save folder nothing is asked.
        let mut nowhere = GuiWorlds::in_folder(Err("no Steam".into()));
        assert_eq!(
            nowhere.request_load(&room, LoadFrom::Menu),
            Err("no Steam".into())
        );
        assert_eq!(lua.run("return tpf3mp_native.poll()"), Ok("nil".into()));
        assert_eq!(lua::take_menu_load(), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn with_no_world_up_the_main_menu_is_asked_to_load_the_rooms_save() {
        let _serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = folder("menu-load");
        let room = dir.join("from-the-room.sav");
        fs::write(&room, b"the room's world").unwrap();
        let lua = Lua::new();
        lua.register();
        let mut worlds = GuiWorlds::in_folder(Ok(dir.clone()));
        worlds.request_load(&room, LoadFrom::Menu).unwrap();
        let name = format!("tpf3mp_room_{}", std::process::id());
        assert!(dir.join(format!("{name}.sav")).is_file());
        assert_eq!(
            lua.run("return tpf3mp_native.poll()"),
            Ok("nil".into()),
            "not the GUI's"
        );
        assert_eq!(lua::take_menu_load(), Some(name));
        lua::menu_load_failed("no menu".into());
        assert_eq!(worlds.load_failed().as_deref(), Some("no menu"));
        assert_eq!(worlds.load_failed(), None);
        fs::remove_dir_all(&dir).unwrap();
    }
}

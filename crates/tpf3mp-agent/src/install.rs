//! Puts a package's in-game pieces where the game loads them
//! (`tpf3mp-agent install-hook`): the hook library next to the game's
//! executable; on Windows the proxy DLL that loads it, in place of the DLL
//! it stands in for, whose original it keeps as `<name>_real.dll`; and the
//! Lua mod in the game's mods folder.
//!
//! It fails closed: a game folder without the DLL the proxy stands in for,
//! with a `<name>_real.dll` TPF3-MP did not put there (another mod's
//! proxy, say), or whose original went missing from behind the proxy, is
//! refused before anything changes. What it installed is recorded in the
//! game folder (`tpf3mp-install.json`), so a reinstall replaces only its
//! own files, a game update that restored the original DLL is noticed, and
//! `--uninstall` puts everything back. A record that names anything but
//! TPF3-MP's own files is refused, since the record says what to rename
//! and delete.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// The record of an install, in the game folder.
pub const RECORD: &str = "tpf3mp-install.json";
/// The package's folder of the proxy DLL, which holds one file named as
/// the DLL it stands in for.
pub const PROXY_DIR: &str = "proxy";
/// The package's Lua mod: `mod/tpf3mp_1`. The game wants a mod's folder
/// name to end in `_<version>`, as TPF2 did.
pub const MOD_DIR: &str = "mod";
pub const MOD_NAME: &str = "tpf3mp_1";
/// The hook library's name on each system.
const HOOK_FILES: [&str; 3] = [
    "tpf3mp_hook.dll",
    "libtpf3mp_hook.so",
    "libtpf3mp_hook.dylib",
];

/// Where to install from and to.
#[derive(Debug, Clone)]
pub struct Install {
    /// The unpacked package.
    pub package: PathBuf,
    /// The folder of the game's executable.
    pub game_dir: PathBuf,
    /// The game's mods folder; `mods` in the game folder by default.
    pub mods_dir: Option<PathBuf>,
}

/// What an install put into the game folder.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    proxy: Option<InstalledProxy>,
    hook: Option<String>,
    #[serde(rename = "mod")]
    lua_mod: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct InstalledProxy {
    /// The DLL it stands in for, such as `alut.dll`.
    name: String,
    /// The original's new name, such as `alut_real.dll`.
    real: String,
    /// The SHA-256 of the proxy as installed, which tells it from an
    /// original a game update put back.
    sha256: String,
}

impl Install {
    /// Installs everything the package has, and says what it did.
    pub fn run(&self) -> Result<Vec<String>> {
        let game = &self.game_dir;
        if !game.is_dir() {
            bail!("{} is not a folder", game.display());
        }
        let mut record = read_record(game)?;
        let mut done = Vec::new();
        let hook = HOOK_FILES
            .iter()
            .find(|name| self.package.join(name).is_file())
            .copied();
        let proxy = self.package_proxy()?;

        // Every check before the first change.
        if let Some(proxy) = &proxy {
            check_proxy_target(game, proxy, record.proxy.as_ref())?;
        }

        if let Some(new) = &proxy {
            if let Some(old) = record.proxy.take_if(|old| old.name != new.name) {
                restore_proxy(game, &old, &mut done)?;
            }
            record.proxy = Some(install_proxy(game, new, record.proxy.as_ref(), &mut done)?);
            write_record(game, &record)?;
        }

        match hook {
            // Without its loader a Windows hook would never run.
            Some(hook) if hook.ends_with(".dll") && proxy.is_none() => done.push(
                "this package has no proxy DLL to load the hook yet, so the hook was not \
                 installed (the release names the DLL once the game is out)"
                    .into(),
            ),
            Some(hook) => {
                fs::copy(self.package.join(hook), game.join(hook))
                    .with_context(|| format!("copying {hook} into {}", game.display()))?;
                record.hook = Some(hook.to_owned());
                write_record(game, &record)?;
                done.push(format!("installed {hook}"));
                if hook.ends_with(".so") {
                    done.push(format!(
                        "set the game's Steam launch options to: LD_PRELOAD=\"{}\" %command%",
                        game.join(hook).display()
                    ));
                }
            }
            None => done.push("this package has no hook library".into()),
        }

        let lua_mod = self.package.join(MOD_DIR).join(MOD_NAME);
        if lua_mod.is_dir() {
            let mods = self.mods_dir.clone().unwrap_or_else(|| game.join("mods"));
            let target = mods.join(MOD_NAME);
            if target.exists() {
                fs::remove_dir_all(&target)
                    .with_context(|| format!("removing the old {}", target.display()))?;
            }
            copy_dir(&lua_mod, &target)
                .with_context(|| format!("copying the mod to {}", target.display()))?;
            record.lua_mod = Some(target.clone());
            write_record(game, &record)?;
            done.push(format!("installed the mod in {}", target.display()));
        } else {
            done.push("this package has no Lua mod yet".into());
        }
        Ok(done)
    }

    /// The package's proxy DLL, if it has one.
    fn package_proxy(&self) -> Result<Option<PackageProxy>> {
        let dir = self.package.join(PROXY_DIR);
        if !dir.is_dir() {
            return Ok(None);
        }
        let mut found = Vec::new();
        for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let path = entry?.path();
            if path.is_file() {
                found.push(path);
            }
        }
        let [path] = found.as_slice() else {
            bail!(
                "{} must hold exactly one DLL, named as the DLL it stands in for",
                dir.display()
            );
        };
        let name = file_name(path)?;
        let Some(real) = real_name(&name) else {
            bail!("{} is not a DLL", path.display());
        };
        Ok(Some(PackageProxy {
            real,
            name,
            path: path.clone(),
        }))
    }
}

/// Takes out what the record says was installed, putting the original DLL
/// back, and says what it did.
pub fn uninstall(game_dir: &Path) -> Result<Vec<String>> {
    let Some(record) = read_record_if_any(game_dir)? else {
        bail!("TPF3-MP is not installed in {}", game_dir.display());
    };
    let mut done = Vec::new();
    if let Some(proxy) = &record.proxy {
        restore_proxy(game_dir, proxy, &mut done)?;
    }
    if let Some(hook) = &record.hook {
        remove_file_if_any(&game_dir.join(hook))?;
        done.push(format!("removed {hook}"));
    }
    if let Some(lua_mod) = &record.lua_mod
        && lua_mod.file_name().is_some_and(|name| name == MOD_NAME)
        && lua_mod.is_dir()
    {
        fs::remove_dir_all(lua_mod).with_context(|| format!("removing {}", lua_mod.display()))?;
        done.push(format!("removed the mod from {}", lua_mod.display()));
    }
    fs::remove_file(game_dir.join(RECORD)).context("removing the install record")?;
    Ok(done)
}

struct PackageProxy {
    name: String,
    real: String,
    path: PathBuf,
}

/// Refuses a game folder the proxy cannot go into safely.
fn check_proxy_target(
    game: &Path,
    proxy: &PackageProxy,
    installed: Option<&InstalledProxy>,
) -> Result<()> {
    let current = game.join(&proxy.name);
    if !current.is_file() {
        bail!(
            "there is no {} in {}: give the folder that holds the game's executable",
            proxy.name,
            game.display()
        );
    }
    let ours = installed.filter(|installed| installed.name == proxy.name);
    let real = game.join(&proxy.real);
    if real.exists() && ours.is_none() {
        bail!(
            "{} already has a {} that TPF3-MP did not put there, perhaps another mod's; \
             remove that mod, or have the game verify its files, and install again",
            game.display(),
            proxy.real
        );
    }
    // The proxy forwards to the game's own DLL: with that gone, the game
    // cannot start, and a new proxy would not change that.
    if let Some(ours) = ours
        && !real.exists()
        && sha256_of(&current)? == ours.sha256
    {
        bail!(
            "the game's own {} is missing from {}: have Steam verify the game's files, \
             and install again",
            proxy.real,
            game.display()
        );
    }
    Ok(())
}

/// Puts the proxy in place of the original, which becomes `<name>_real`.
fn install_proxy(
    game: &Path,
    proxy: &PackageProxy,
    installed: Option<&InstalledProxy>,
    done: &mut Vec<String>,
) -> Result<InstalledProxy> {
    let current = game.join(&proxy.name);
    let real = game.join(&proxy.real);
    let ours = match installed {
        Some(installed) => sha256_of(&current)? == installed.sha256,
        None => false,
    };
    if ours {
        copy_replacing(&proxy.path, &current)?;
        done.push(format!("updated the proxy {}", proxy.name));
    } else {
        // The game's own DLL: a first install, or a game update put it
        // back over the proxy. It becomes the one the proxy forwards to.
        if real.exists() {
            fs::remove_file(&real).with_context(|| format!("removing {}", real.display()))?;
        }
        fs::rename(&current, &real)
            .with_context(|| format!("renaming {} to {}", proxy.name, proxy.real))?;
        if let Err(error) = fs::copy(&proxy.path, &current) {
            let _ = fs::rename(&real, &current);
            return Err(error).with_context(|| format!("copying the proxy {}", proxy.name));
        }
        done.push(format!(
            "installed the proxy {} (the game's own is now {})",
            proxy.name, proxy.real
        ));
    }
    Ok(InstalledProxy {
        name: proxy.name.clone(),
        real: proxy.real.clone(),
        sha256: sha256_of(&current)?,
    })
}

/// Puts the game's own DLL back in place of an installed proxy.
fn restore_proxy(game: &Path, proxy: &InstalledProxy, done: &mut Vec<String>) -> Result<()> {
    let current = game.join(&proxy.name);
    let real = game.join(&proxy.real);
    let still_ours = current.is_file() && sha256_of(&current)? == proxy.sha256;
    if !real.exists() {
        if still_ours {
            // Without the original the proxy only keeps the game from
            // starting.
            fs::remove_file(&current).with_context(|| format!("removing {}", current.display()))?;
            done.push(format!(
                "removed the proxy {}, but the game's own was missing: \
                 have Steam verify the game's files",
                proxy.name
            ));
        }
        return Ok(());
    }
    if still_ours || !current.exists() {
        remove_file_if_any(&current)?;
        fs::rename(&real, &current)
            .with_context(|| format!("renaming {} back to {}", proxy.real, proxy.name))?;
    } else {
        // A game update already put its own DLL back.
        fs::remove_file(&real).with_context(|| format!("removing {}", real.display()))?;
    }
    done.push(format!("put the game's own {} back", proxy.name));
    Ok(())
}

fn read_record(game: &Path) -> Result<Record> {
    Ok(read_record_if_any(game)?.unwrap_or_default())
}

fn read_record_if_any(game: &Path) -> Result<Option<Record>> {
    let path = game.join(RECORD);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let record: Record =
        serde_json::from_slice(&bytes).with_context(|| format!("{} is damaged", path.display()))?;
    if !record.names_only_its_own() {
        bail!(
            "{} names files TPF3-MP does not install, so nothing was changed",
            path.display()
        );
    }
    Ok(Some(record))
}

impl Record {
    /// Whether it names only what TPF3-MP installs, never a path elsewhere:
    /// its record says what to rename and delete.
    fn names_only_its_own(&self) -> bool {
        let hook = self
            .hook
            .as_deref()
            .is_none_or(|hook| HOOK_FILES.contains(&hook));
        let proxy = self
            .proxy
            .as_ref()
            .is_none_or(|proxy| real_name(&proxy.name).is_some_and(|real| real == proxy.real));
        let lua_mod = self
            .lua_mod
            .as_ref()
            .is_none_or(|path| path.file_name().is_some_and(|name| name == MOD_NAME));
        hook && proxy && lua_mod
    }
}

/// The name the game's own DLL is kept under beside the proxy,
/// `<stem>_real.dll`, if `name` is a plain DLL file name.
fn real_name(name: &str) -> Option<String> {
    if !name.is_ascii() || name.len() <= ".dll".len() {
        return None;
    }
    let (stem, extension) = name.split_at(name.len() - ".dll".len());
    let plain = extension.eq_ignore_ascii_case(".dll")
        && !stem.starts_with('.')
        && stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    plain.then(|| format!("{stem}_real.dll"))
}

fn write_record(game: &Path, record: &Record) -> Result<()> {
    let path = game.join(RECORD);
    fs::write(&path, serde_json::to_vec_pretty(record)?)
        .with_context(|| format!("writing {}", path.display()))
}

fn sha256_of(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let digest = ring::digest::digest(&ring::digest::SHA256, &bytes);
    Ok(digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn file_name(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .with_context(|| format!("{} has no usable name", path.display()))
}

fn copy_replacing(from: &Path, to: &Path) -> Result<()> {
    fs::copy(from, to).with_context(|| format!("copying {}", to.display()))?;
    Ok(())
}

fn remove_file_if_any(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
}

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    /// A Windows package with a proxy for `alut.dll` and the mod, and a game
    /// folder with its own `alut.dll`.
    fn setup() -> (tempfile::TempDir, Install) {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("package");
        write(&package.join("tpf3mp_hook.dll"), "hook");
        write(&package.join("proxy/alut.dll"), "proxy 1");
        write(&package.join("mod/tpf3mp_1/mod.lua"), "-- mod");
        write(&package.join("mod/tpf3mp_1/res/x.lua"), "-- x");
        let game = root.path().join("game");
        write(&game.join("game.exe"), "exe");
        write(&game.join("alut.dll"), "original");
        let install = Install {
            package,
            game_dir: game,
            mods_dir: None,
        };
        (root, install)
    }

    #[test]
    fn installs_the_proxy_the_hook_and_the_mod() {
        let (_root, install) = setup();
        let game = install.game_dir.clone();
        install.run().unwrap();
        assert_eq!(read(&game.join("alut.dll")), "proxy 1");
        assert_eq!(read(&game.join("alut_real.dll")), "original");
        assert_eq!(read(&game.join("tpf3mp_hook.dll")), "hook");
        assert_eq!(read(&game.join("mods/tpf3mp_1/res/x.lua")), "-- x");

        // Again, with a newer proxy: the original stays as it was.
        write(&install.package.join("proxy/alut.dll"), "proxy 2");
        install.run().unwrap();
        assert_eq!(read(&game.join("alut.dll")), "proxy 2");
        assert_eq!(read(&game.join("alut_real.dll")), "original");

        uninstall(&game).unwrap();
        assert_eq!(read(&game.join("alut.dll")), "original");
        for gone in ["alut_real.dll", "tpf3mp_hook.dll", "mods/tpf3mp_1", RECORD] {
            assert!(!game.join(gone).exists(), "{gone} is left");
        }
        assert!(uninstall(&game).is_err(), "nothing left to uninstall");
    }

    #[test]
    fn a_game_update_that_put_the_original_back_is_noticed() {
        let (_root, install) = setup();
        let game = install.game_dir.clone();
        install.run().unwrap();
        write(&game.join("alut.dll"), "original 2");
        install.run().unwrap();
        assert_eq!(read(&game.join("alut.dll")), "proxy 1");
        assert_eq!(read(&game.join("alut_real.dll")), "original 2");

        // Updated again, then uninstalled: the newest original stays.
        write(&game.join("alut.dll"), "original 3");
        uninstall(&game).unwrap();
        assert_eq!(read(&game.join("alut.dll")), "original 3");
        assert!(!game.join("alut_real.dll").exists());
    }

    #[test]
    fn a_folder_it_cannot_install_into_is_left_alone() {
        let (_root, install) = setup();
        let game = install.game_dir.clone();

        // Another mod's proxy.
        write(&game.join("alut_real.dll"), "someone's");
        assert!(install.run().is_err());
        assert_eq!(read(&game.join("alut.dll")), "original");
        assert_eq!(read(&game.join("alut_real.dll")), "someone's");
        assert!(!game.join("tpf3mp_hook.dll").exists());
        assert!(!game.join("mods").exists());

        // Not the game's folder.
        fs::remove_file(game.join("alut_real.dll")).unwrap();
        fs::remove_file(game.join("alut.dll")).unwrap();
        assert!(install.run().is_err());
        assert!(!game.join(RECORD).exists());
    }

    #[test]
    fn a_record_that_names_other_files_changes_nothing() {
        let (_root, install) = setup();
        let game = install.game_dir.clone();
        install.run().unwrap();
        let record = read(&game.join(RECORD));
        for (from, to) in [
            ("\"tpf3mp_hook.dll\"", "\"../elsewhere.dll\""),
            ("\"alut_real.dll\"", "\"game.exe\""),
            ("\"alut.dll\"", "\"../alut.dll\""),
        ] {
            assert!(record.contains(from), "{record}");
            fs::write(game.join(RECORD), record.replace(from, to)).unwrap();
            assert!(install.run().is_err(), "{to}");
            assert!(uninstall(&game).is_err(), "{to}");
            assert_eq!(read(&game.join("alut.dll")), "proxy 1");
            assert_eq!(read(&game.join("alut_real.dll")), "original");
            assert_eq!(read(&game.join("tpf3mp_hook.dll")), "hook");
            assert_eq!(read(&game.join("game.exe")), "exe");
        }
    }

    #[test]
    fn a_proxy_whose_original_is_gone_is_taken_out_not_updated() {
        let (_root, install) = setup();
        let game = install.game_dir.clone();
        install.run().unwrap();
        fs::remove_file(game.join("alut_real.dll")).unwrap();
        write(&install.package.join("proxy/alut.dll"), "proxy 2");
        let error = install.run().unwrap_err();
        assert!(format!("{error:#}").contains("verify"), "{error:#}");
        assert_eq!(read(&game.join("alut.dll")), "proxy 1");

        let done = uninstall(&game).unwrap();
        assert!(done.iter().any(|line| line.contains("verify")), "{done:?}");
        assert!(!game.join("alut.dll").exists());
        assert!(!game.join(RECORD).exists());
    }

    #[test]
    fn only_a_plain_dll_name_is_a_proxy_name() {
        assert_eq!(real_name("alut.dll").as_deref(), Some("alut_real.dll"));
        assert_eq!(real_name("D3D11.DLL").as_deref(), Some("D3D11_real.dll"));
        for name in [
            "alut.exe",
            ".dll",
            "..dll",
            "../alut.dll",
            "a\\b.dll",
            "älut.dll",
        ] {
            assert_eq!(real_name(name), None, "{name}");
        }
    }

    #[test]
    fn without_a_proxy_the_windows_hook_is_not_installed() {
        let (_root, mut install) = setup();
        fs::remove_dir_all(install.package.join("proxy")).unwrap();
        let mods = install.game_dir.join("elsewhere");
        install.mods_dir = Some(mods.clone());
        let done = install.run().unwrap();
        assert!(
            done.iter().any(|line| line.contains("no proxy DLL")),
            "{done:?}"
        );
        assert!(!install.game_dir.join("tpf3mp_hook.dll").exists());
        assert_eq!(read(&install.game_dir.join("alut.dll")), "original");
        assert!(mods.join("tpf3mp_1/mod.lua").is_file());
    }

    #[test]
    fn a_linux_hook_comes_with_its_launch_option() {
        let (_root, install) = setup();
        fs::remove_dir_all(install.package.join("proxy")).unwrap();
        fs::remove_dir_all(install.package.join("mod")).unwrap();
        fs::remove_file(install.package.join("tpf3mp_hook.dll")).unwrap();
        write(&install.package.join("libtpf3mp_hook.so"), "so");
        let done = install.run().unwrap();
        assert_eq!(read(&install.game_dir.join("libtpf3mp_hook.so")), "so");
        assert!(
            done.iter().any(|line| line.contains("LD_PRELOAD=")),
            "{done:?}"
        );
        assert!(
            done.iter().any(|line| line.contains("no Lua mod")),
            "{done:?}"
        );
    }
}

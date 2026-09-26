//! Puts a package's in-game pieces where the game loads them
//! (`tpf3mp-agent install-hook`): the hook library next to the game's
//! executable; on Windows the proxy DLL that loads it, in place of the DLL
//! it stands in for, whose original it keeps as `<name>_real.dll`; and the
//! Lua mod in the game's mods folder.
//!
//! It fails closed: a game folder without the DLL the proxy stands in for,
//! or with a `<name>_real.dll` TPF3-MP did not put there (another mod's
//! proxy, say), is refused before anything changes. What it installed is
//! recorded in the game folder (`tpf3mp-install.json`), so a reinstall
//! replaces only its own files, a game update that restored the original
//! DLL is noticed, and `--uninstall` puts everything back.

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
/// The package's Lua mod: `mod/tpf3mp`.
pub const MOD_DIR: &str = "mod";
pub const MOD_NAME: &str = "tpf3mp";
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
        let Some(stem) = name
            .strip_suffix(".dll")
            .or_else(|| name.strip_suffix(".DLL"))
        else {
            bail!("{} is not a DLL", path.display());
        };
        Ok(Some(PackageProxy {
            real: format!("{stem}_real.dll"),
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
    let ours = installed.is_some_and(|installed| installed.name == proxy.name);
    if game.join(&proxy.real).exists() && !ours {
        bail!(
            "{} already has a {} that TPF3-MP did not put there, perhaps another mod's; \
             remove that mod, or have the game verify its files, and install again",
            game.display(),
            proxy.real
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
    if !real.exists() {
        return Ok(());
    }
    let still_ours = current.is_file() && sha256_of(&current)? == proxy.sha256;
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
    match fs::read(&path) {
        Ok(bytes) => {
            Ok(Some(serde_json::from_slice(&bytes).with_context(|| {
                format!("{} is damaged", path.display())
            })?))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
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
        write(&package.join("mod/tpf3mp/mod.lua"), "-- mod");
        write(&package.join("mod/tpf3mp/res/x.lua"), "-- x");
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
        assert_eq!(read(&game.join("mods/tpf3mp/res/x.lua")), "-- x");

        // Again, with a newer proxy: the original stays as it was.
        write(&install.package.join("proxy/alut.dll"), "proxy 2");
        install.run().unwrap();
        assert_eq!(read(&game.join("alut.dll")), "proxy 2");
        assert_eq!(read(&game.join("alut_real.dll")), "original");

        uninstall(&game).unwrap();
        assert_eq!(read(&game.join("alut.dll")), "original");
        for gone in ["alut_real.dll", "tpf3mp_hook.dll", "mods/tpf3mp", RECORD] {
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
        assert!(mods.join("tpf3mp/mod.lua").is_file());
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

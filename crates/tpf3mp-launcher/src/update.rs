//! Keeps TPF3-MP up to date from the project's GitHub releases, installing
//! only what the project signed.
//!
//! A release carries `release.json`, its version and, for each platform,
//! the package's file name, size and SHA-256, and `release.json.sig`, an
//! Ed25519 signature of that file made with the project's update key. The
//! launcher is built with the key's public half (`TPF3MP_UPDATE_PUBLIC_KEY`
//! at build time); a build without one never updates. A package is
//! installed only if:
//!
//! - the signature verifies, the release's tag names the signed version,
//!   and that version is newer than the running one;
//! - the package's size and SHA-256 match the signed manifest;
//! - every path in it stays inside the package, and it holds only files and
//!   folders.
//!
//! Packages are downloaded into `.tpf3mp-update/` in the install folder, so
//! installing is a rename on the same disk, and checked again when they
//! are installed. Installing moves each replaced file aside as `*.old` and
//! puts them all back if any step fails; the next start deletes them. The
//! player chooses when to install, because restarting ends a game; an
//! update not installed then is installed at the next start.

use std::{
    ffi::OsString,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use base64::Engine;
use eframe::egui;
use ring::{
    digest::{Context, SHA256},
    signature::{ED25519, UnparsedPublicKey},
};
use serde::Deserialize;
use thiserror::Error;
use tracing::{info, warn};

/// This build's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The repository releases come from.
pub const REPOSITORY: &str = match option_env!("TPF3MP_REPOSITORY") {
    Some(repository) => repository,
    None => "Juliansgith/Transport-Fever-3-Multiplayer-Mod",
};
/// The public half of the key releases are signed with, as base64 of its
/// 32 bytes. Without it, this build never updates.
const PUBLIC_KEY: Option<&str> = option_env!("TPF3MP_UPDATE_PUBLIC_KEY");
/// The package this build comes in, as release file names name it.
pub const PLATFORM: Option<&str> = if cfg!(all(windows, target_arch = "x86_64")) {
    Some("windows-x64")
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some("linux-x64")
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some("macos-arm64")
} else {
    None
};

/// The file a package has at its top, naming its version and platform.
pub const PACKAGE_MARKER: &str = "tpf3mp-package.json";
/// Where downloads wait in the install folder.
const STAGING: &str = ".tpf3mp-update";
/// Suffix of the files an install moved aside.
const OLD: &str = ".old";
const MANIFEST: &str = "release.json";
const SIGNATURE: &str = "release.json.sig";
/// Largest manifest read.
const MAX_MANIFEST: u64 = 64 * 1024;
/// Largest package downloaded.
const MAX_PACKAGE: u64 = 1 << 30;
/// Largest answer about the latest release.
const MAX_RELEASE_INFO: u64 = 1 << 20;
/// Pause before the first check, so starting stays quick.
const FIRST_CHECK: Duration = Duration::from_secs(3);
/// How often a running launcher checks again.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("cannot reach GitHub: {0}")]
    Http(String),
    #[error("the release is not signed with the project's key")]
    BadSignature,
    #[error("the release is malformed: {0}")]
    Malformed(String),
    #[error("the release has no package for this system")]
    NoPackage,
    #[error("the downloaded package does not match the signed release")]
    Mismatch,
    #[error("the package has an unsafe entry: {0}")]
    UnsafeEntry(String),
}

impl From<ureq::Error> for UpdateError {
    fn from(error: ureq::Error) -> Self {
        Self::Http(error.to_string())
    }
}

/// What the updater is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateState {
    Checking,
    UpToDate,
    /// It does not update this copy, for this reason.
    Off(String),
    Failed(String),
    Downloading {
        version: String,
        bytes: u64,
        total: u64,
    },
    /// Downloaded and checked; installs when the player chooses, or at the
    /// next start.
    Ready {
        version: String,
    },
    Installing {
        version: String,
    },
}

/// The signed description of a release.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    pub version: String,
    /// Per platform name: the package.
    pub packages: std::collections::BTreeMap<String, Package>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Package {
    pub name: String,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
}

impl Manifest {
    /// The manifest in `json`, if `signature` is `key`'s signature of it.
    pub fn verified(json: &[u8], signature: &[u8], key: &[u8]) -> Result<Self, UpdateError> {
        UnparsedPublicKey::new(&ED25519, key)
            .verify(json, signature)
            .map_err(|_| UpdateError::BadSignature)?;
        let manifest: Self = serde_json::from_slice(json)
            .map_err(|error| UpdateError::Malformed(error.to_string()))?;
        semver::Version::parse(&manifest.version)
            .map_err(|error| UpdateError::Malformed(format!("version: {error}")))?;
        for package in manifest.packages.values() {
            if !safe_name(&package.name) || package.sha256.len() != 64 {
                return Err(UpdateError::Malformed(format!(
                    "package entry {}",
                    package.name
                )));
            }
        }
        Ok(manifest)
    }

    /// Whether this release is newer than `current`.
    pub fn newer_than(&self, current: &str) -> bool {
        match (
            semver::Version::parse(&self.version),
            semver::Version::parse(current),
        ) {
            (Ok(offered), Ok(current)) => offered > current,
            _ => false,
        }
    }
}

/// A plain file name: no folders, nothing hidden or relative.
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// The key this build trusts, if it has one.
fn public_key() -> Option<Vec<u8>> {
    let encoded = PUBLIC_KEY?.trim();
    let key = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    (key.len() == 32).then_some(key)
}

/// Where this copy is installed, and whether the updater may change it.
#[derive(Debug, Clone)]
pub struct Install {
    /// The package's folder.
    pub root: PathBuf,
    /// The launcher's own file, as started: what restarting runs.
    pub exe: PathBuf,
    pub platform: &'static str,
}

impl Install {
    /// This copy, if it is an installed package of this platform: its
    /// folder has the package marker naming the platform.
    pub fn of_running() -> Result<Self, String> {
        let platform = PLATFORM.ok_or("this system has no packages")?;
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let root = package_root(&exe).ok_or("cannot tell where TPF3-MP is installed")?;
        Self::at(root, exe, platform)
    }

    fn at(root: PathBuf, exe: PathBuf, platform: &'static str) -> Result<Self, String> {
        let marker = fs::read(root.join(PACKAGE_MARKER))
            .map_err(|_| "this copy is not an installed package".to_owned())?;
        #[derive(Deserialize)]
        struct Marker {
            platform: String,
        }
        let marker: Marker = serde_json::from_slice(&marker)
            .map_err(|_| "this copy's package marker is unreadable".to_owned())?;
        if marker.platform != platform {
            return Err(format!("this copy is the {} package", marker.platform));
        }
        Ok(Self {
            root,
            exe,
            platform,
        })
    }

    fn staging(&self) -> PathBuf {
        self.root.join(STAGING)
    }
}

/// The folder a package was unpacked into, from the launcher's own path:
/// the executable's folder, or on macOS the folder holding the app bundle.
fn package_root(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    if cfg!(target_os = "macos")
        && let Some(bundle) = dir
            .ancestors()
            .find(|path| path.extension().is_some_and(|ext| ext == "app"))
    {
        return bundle.parent().map(Path::to_owned);
    }
    Some(dir.to_owned())
}

/// Checks, downloads and installs updates in the background.
#[derive(Clone)]
pub struct Updater {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<UpdateState>,
    install: Option<Install>,
    key: Option<Vec<u8>>,
    runtime: tokio::runtime::Handle,
    repaint: Mutex<Option<egui::Context>>,
    /// One check or download at a time.
    working: Mutex<()>,
}

impl Updater {
    /// An updater for this copy. It checks soon after starting and every
    /// few hours, and downloads what it finds; installing waits for the
    /// player.
    pub fn start(runtime: tokio::runtime::Handle) -> Self {
        let key = public_key();
        let install = Install::of_running();
        let state = match (&key, &install) {
            (None, _) => UpdateState::Off("this build has no update key".into()),
            (Some(_), Err(reason)) => UpdateState::Off(reason.clone()),
            (Some(_), Ok(_)) => UpdateState::Checking,
        };
        let updater = Self {
            inner: Arc::new(Inner {
                state: Mutex::new(state.clone()),
                install: install.ok(),
                key,
                runtime,
                repaint: Mutex::default(),
                working: Mutex::default(),
            }),
        };
        if state == UpdateState::Checking {
            let every = updater.clone();
            updater.inner.runtime.spawn(async move {
                tokio::time::sleep(FIRST_CHECK).await;
                loop {
                    every.check();
                    tokio::time::sleep(CHECK_EVERY).await;
                }
            });
        } else if let UpdateState::Off(reason) = &state {
            info!(%reason, "updates are off");
        }
        updater
    }

    /// Repaints the window when the state changes.
    pub fn repaint_with(&self, ctx: egui::Context) {
        *self
            .inner
            .repaint
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(ctx);
    }

    pub fn state(&self) -> UpdateState {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set(&self, state: UpdateState) {
        *self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = state;
        if let Some(ctx) = &*self
            .inner
            .repaint
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
        {
            ctx.request_repaint();
        }
    }

    /// Checks for a newer release now, and downloads it.
    pub fn check(&self) {
        let (Some(install), Some(key)) = (self.inner.install.clone(), self.inner.key.clone())
        else {
            return;
        };
        let updater = self.clone();
        self.inner.runtime.spawn_blocking(move || {
            let Ok(_working) = updater.inner.working.try_lock() else {
                return;
            };
            // A downloaded update waits for the player; nothing to check.
            if matches!(updater.state(), UpdateState::Ready { .. }) {
                return;
            }
            updater.set(UpdateState::Checking);
            let result = check_and_download(&install, &key, |version, bytes, total| {
                updater.set(UpdateState::Downloading {
                    version: version.to_owned(),
                    bytes,
                    total,
                });
            });
            match result {
                Ok(Some(version)) => {
                    info!(%version, "an update is ready to install");
                    updater.set(UpdateState::Ready { version });
                }
                Ok(None) => updater.set(UpdateState::UpToDate),
                Err(error) => {
                    warn!(%error, "the update check failed");
                    updater.set(UpdateState::Failed(error.to_string()));
                }
            }
        });
    }

    /// Installs the downloaded update and restarts into it, closing this
    /// window. On failure everything stays as it was.
    pub fn install_and_restart(&self, ctx: &egui::Context) {
        let (Some(install), Some(key)) = (&self.inner.install, &self.inner.key) else {
            return;
        };
        let UpdateState::Ready { version } = self.state() else {
            return;
        };
        self.set(UpdateState::Installing {
            version: version.clone(),
        });
        match install_staged(install, key).and_then(|installed| {
            restart(install)?;
            Ok(installed)
        }) {
            Ok(Some(installed)) => {
                info!(version = %installed, "installed the update; restarting");
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Ok(None) => self.set(UpdateState::UpToDate),
            Err(error) => {
                warn!(%error, "cannot install the update");
                self.set(UpdateState::Failed(format!("cannot install: {error}")));
            }
        }
    }
}

/// At start, before anything else: deletes what the last install moved
/// aside, and installs an update downloaded earlier. Returns whether the
/// launcher restarted into it, and should now exit.
pub fn at_start() -> bool {
    let Ok(install) = Install::of_running() else {
        return false;
    };
    remove_old(&install.root);
    let Some(key) = public_key() else {
        return false;
    };
    match install_staged(&install, &key) {
        Ok(Some(version)) => match restart(&install) {
            Ok(()) => {
                info!(%version, "installed the update downloaded earlier; restarting");
                true
            }
            Err(error) => {
                warn!(%error, "installed the update but cannot restart");
                false
            }
        },
        Ok(None) => false,
        Err(error) => {
            warn!(%error, "cannot install the update downloaded earlier");
            false
        }
    }
}

/// Deletes the files the last install moved aside. The old launcher could
/// not delete itself while running.
fn remove_old(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(OLD) {
            let path = entry.path();
            let removed = if path.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
            if let Err(error) = removed {
                warn!(path = %path.display(), %error, "cannot delete a file an update replaced");
            }
        }
    }
}

/// Runs the launcher again with the same arguments.
fn restart(install: &Install) -> Result<(), UpdateError> {
    std::process::Command::new(&install.exe)
        .args(std::env::args_os().skip(1).collect::<Vec<OsString>>())
        .spawn()?;
    Ok(())
}

/// What GitHub says about the latest release.
#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(900)))
            .build(),
    )
}

fn get(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, UpdateError> {
    let mut response = agent
        .get(url)
        .header("User-Agent", format!("tpf3mp-launcher/{VERSION}"))
        .header(
            "Accept",
            "application/vnd.github+json, application/octet-stream",
        )
        .call()?;
    Ok(response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()?)
}

/// Checks the latest release and downloads its package if it is newer.
/// Returns the version downloaded, or `None` when up to date.
fn check_and_download(
    install: &Install,
    key: &[u8],
    mut progress: impl FnMut(&str, u64, u64),
) -> Result<Option<String>, UpdateError> {
    let agent = agent();
    let info = get(
        &agent,
        &format!("https://api.github.com/repos/{REPOSITORY}/releases/latest"),
        MAX_RELEASE_INFO,
    )?;
    let release: Release =
        serde_json::from_slice(&info).map_err(|error| UpdateError::Malformed(error.to_string()))?;
    let url = |name: &str| {
        release
            .assets
            .iter()
            .find(|asset| asset.name == name)
            .map(|asset| asset.browser_download_url.clone())
    };
    let (Some(manifest_url), Some(signature_url)) = (url(MANIFEST), url(SIGNATURE)) else {
        // A release without a signed manifest is not for the updater.
        return Ok(None);
    };
    let json = get(&agent, &manifest_url, MAX_MANIFEST)?;
    let signature = get(&agent, &signature_url, 256)?;
    let manifest = Manifest::verified(&json, &signature, key)?;
    if release.tag_name != format!("v{}", manifest.version) {
        return Err(UpdateError::Malformed(format!(
            "the release {} signs version {}",
            release.tag_name, manifest.version
        )));
    }
    if !manifest.newer_than(VERSION) {
        return Ok(None);
    }
    let package = manifest
        .packages
        .get(install.platform)
        .ok_or(UpdateError::NoPackage)?;
    if package.size > MAX_PACKAGE {
        return Err(UpdateError::Malformed("the package is too large".into()));
    }
    let package_url = url(&package.name).ok_or(UpdateError::NoPackage)?;
    let dir = install.staging().join(&manifest.version);
    fs::create_dir_all(&dir)?;
    let archive = dir.join(&package.name);
    if !matches(&archive, package)? {
        let partial = dir.join(format!("{}.part", package.name));
        download(&agent, &package_url, &partial, package, |bytes| {
            progress(&manifest.version, bytes, package.size);
        })?;
        fs::rename(&partial, &archive)?;
    }
    // The manifest and signature go with the package, to check it again
    // when it is installed.
    fs::write(dir.join(MANIFEST), &json)?;
    fs::write(dir.join(SIGNATURE), &signature)?;
    Ok(Some(manifest.version))
}

/// Downloads `package` into `path`, checking its size and hash on the way.
fn download(
    agent: &ureq::Agent,
    url: &str,
    path: &Path,
    package: &Package,
    mut progress: impl FnMut(u64),
) -> Result<(), UpdateError> {
    let mut response = agent
        .get(url)
        .header("User-Agent", format!("tpf3mp-launcher/{VERSION}"))
        .header("Accept", "application/octet-stream")
        .call()?;
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(package.size + 1)
        .reader();
    let mut file = File::create(path)?;
    let mut digest = Context::new(&SHA256);
    let mut buffer = vec![0; 256 * 1024];
    let mut total = 0u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > package.size {
            return Err(UpdateError::Mismatch);
        }
        digest.update(&buffer[..read]);
        file.write_all(&buffer[..read])?;
        progress(total);
    }
    file.sync_all()?;
    if total != package.size || hex(digest.finish().as_ref()) != package.sha256 {
        let _ = fs::remove_file(path);
        return Err(UpdateError::Mismatch);
    }
    Ok(())
}

/// Whether the file at `path` is `package`.
fn matches(path: &Path, package: &Package) -> Result<bool, UpdateError> {
    let Ok(mut file) = File::open(path) else {
        return Ok(false);
    };
    if file.metadata()?.len() != package.size {
        return Ok(false);
    }
    let mut digest = Context::new(&SHA256);
    let mut buffer = vec![0; 256 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex(digest.finish().as_ref()) == package.sha256)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Installs the newest downloaded update that is newer than this version
/// and still checks out. Returns its version, or `None` if there is none.
/// Older downloads are deleted.
fn install_staged(install: &Install, key: &[u8]) -> Result<Option<String>, UpdateError> {
    let Some((manifest, archive)) = newest_staged(install, key)? else {
        return Ok(None);
    };
    let unpacked = install
        .staging()
        .join(format!("{}.unpacked", manifest.version));
    let _ = fs::remove_dir_all(&unpacked);
    let package = unpack(&archive, &unpacked)?;
    swap_in(&package, &install.root)?;
    let _ = fs::remove_dir_all(install.staging());
    Ok(Some(manifest.version))
}

/// The newest staged release newer than this version whose signature and
/// package check out.
fn newest_staged(
    install: &Install,
    key: &[u8],
) -> Result<Option<(Manifest, PathBuf)>, UpdateError> {
    let Ok(entries) = fs::read_dir(install.staging()) else {
        return Ok(None);
    };
    let mut best: Option<(Manifest, PathBuf)> = None;
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() || dir.extension().is_some_and(|ext| ext == "unpacked") {
            continue;
        }
        let (Ok(json), Ok(signature)) =
            (fs::read(dir.join(MANIFEST)), fs::read(dir.join(SIGNATURE)))
        else {
            continue;
        };
        let Ok(manifest) = Manifest::verified(&json, &signature, key) else {
            warn!(dir = %dir.display(), "a downloaded update does not verify; deleting it");
            let _ = fs::remove_dir_all(&dir);
            continue;
        };
        if !manifest.newer_than(VERSION) {
            let _ = fs::remove_dir_all(&dir);
            continue;
        }
        let Some(package) = manifest.packages.get(install.platform) else {
            continue;
        };
        let archive = dir.join(&package.name);
        if !matches(&archive, package)? {
            continue;
        }
        let newer = best
            .as_ref()
            .is_none_or(|(best, _)| manifest.newer_than(&best.version));
        if newer {
            best = Some((manifest, archive));
        }
    }
    Ok(best)
}

/// Unpacks `archive` into `into` and returns the package folder in it:
/// the single folder at the archive's top. Refuses any entry that would
/// land outside `into`, and anything but files and folders.
pub fn unpack(archive: &Path, into: &Path) -> Result<PathBuf, UpdateError> {
    fs::create_dir_all(into)?;
    let name = archive.to_string_lossy();
    if name.ends_with(".zip") {
        unpack_zip(archive, into)?;
    } else if name.ends_with(".tar.gz") {
        unpack_tar_gz(archive, into)?;
    } else {
        return Err(UpdateError::Malformed(format!("unknown archive {name}")));
    }
    let tops: Vec<PathBuf> = fs::read_dir(into)?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    match tops.as_slice() {
        [top] if top.is_dir() => Ok(top.clone()),
        _ => Err(UpdateError::Malformed(
            "the package is not one folder".into(),
        )),
    }
}

/// `path` inside `into`, if it names a place there.
fn contained(into: &Path, path: &Path) -> Result<PathBuf, UpdateError> {
    let mut out = into.to_owned();
    let mut any = false;
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                out.push(part);
                any = true;
            }
            Component::CurDir => {}
            _ => return Err(UpdateError::UnsafeEntry(path.display().to_string())),
        }
    }
    if any {
        Ok(out)
    } else {
        Err(UpdateError::UnsafeEntry(path.display().to_string()))
    }
}

fn unpack_zip(archive: &Path, into: &Path) -> Result<(), UpdateError> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)
        .map_err(|error| UpdateError::Malformed(error.to_string()))?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| UpdateError::Malformed(error.to_string()))?;
        if entry.is_symlink() {
            return Err(UpdateError::UnsafeEntry(entry.name().to_owned()));
        }
        let name = entry
            .enclosed_name()
            .ok_or_else(|| UpdateError::UnsafeEntry(entry.name().to_owned()))?;
        let path = contained(into, &name)?;
        if entry.is_dir() {
            fs::create_dir_all(&path)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(&path)?;
        io::copy(&mut entry, &mut file)?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(mode & 0o755))?;
        }
    }
    Ok(())
}

fn unpack_tar_gz(archive: &Path, into: &Path) -> Result<(), UpdateError> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?));
    for entry in tar.entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        let name = entry.path()?.into_owned();
        let path = contained(into, &name)?;
        if kind.is_dir() {
            fs::create_dir_all(&path)?;
        } else if kind.is_file() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = File::create(&path)?;
            io::copy(&mut entry, &mut file)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = entry.header().mode()?;
                fs::set_permissions(&path, fs::Permissions::from_mode(mode & 0o755))?;
            }
        } else if matches!(
            kind,
            tar::EntryType::XGlobalHeader | tar::EntryType::XHeader
        ) {
            // Metadata for the entries after it.
        } else {
            return Err(UpdateError::UnsafeEntry(name.display().to_string()));
        }
    }
    Ok(())
}

/// Puts everything at the top of `package` in place of the same names in
/// `root`, moving what was there aside as `*.old`. If any step fails, puts
/// everything back.
pub fn swap_in(package: &Path, root: &Path) -> Result<(), UpdateError> {
    let mut done: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    let result = (|| -> Result<(), UpdateError> {
        for entry in fs::read_dir(package)? {
            let entry = entry?;
            let target = root.join(entry.file_name());
            let mut old_name = entry.file_name();
            old_name.push(OLD);
            let old = root.join(old_name);
            if old.exists() {
                if old.is_dir() {
                    fs::remove_dir_all(&old)?;
                } else {
                    fs::remove_file(&old)?;
                }
            }
            let moved = if target.exists() {
                fs::rename(&target, &old)?;
                Some(old)
            } else {
                None
            };
            if let Err(error) = fs::rename(entry.path(), &target) {
                if let Some(old) = &moved {
                    let _ = fs::rename(old, &target);
                }
                return Err(error.into());
            }
            done.push((target, moved));
        }
        Ok(())
    })();
    if result.is_err() {
        roll_back(&done);
    }
    result
}

/// Undoes the replacements in `done`, newest first.
fn roll_back(done: &[(PathBuf, Option<PathBuf>)]) {
    for (target, old) in done.iter().rev() {
        let removed = if target.is_dir() {
            fs::remove_dir_all(target)
        } else {
            fs::remove_file(target)
        };
        if let Err(error) = removed {
            warn!(path = %target.display(), %error, "cannot remove a new file while rolling back");
        }
        if let Some(old) = old
            && let Err(error) = fs::rename(old, target)
        {
            warn!(path = %target.display(), %error, "cannot put an old file back");
        }
    }
}

#[cfg(test)]
mod tests {
    use ring::{rand::SystemRandom, signature::Ed25519KeyPair, signature::KeyPair};

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tpf3mp-update-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn key_pair() -> Ed25519KeyPair {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    fn manifest_json(version: &str) -> Vec<u8> {
        format!(
            r#"{{"version":"{version}","packages":{{"windows-x64":{{"name":"tpf3mp-{version}-windows-x64.zip","size":3,"sha256":"{}"}}}}}}"#,
            "ab".repeat(32)
        )
        .into_bytes()
    }

    #[test]
    fn only_a_release_signed_with_the_key_verifies() {
        let keys = key_pair();
        let json = manifest_json("9.1.0");
        let signature = keys.sign(&json);
        let key = keys.public_key().as_ref();
        let manifest = Manifest::verified(&json, signature.as_ref(), key).unwrap();
        assert_eq!(manifest.version, "9.1.0");
        assert!(manifest.newer_than("0.1.0"));
        assert!(!manifest.newer_than("9.1.0"));
        assert!(!manifest.newer_than("10.0.0"));

        let mut tampered = json.clone();
        let at = tampered.len() - 10;
        tampered[at] ^= 1;
        assert!(matches!(
            Manifest::verified(&tampered, signature.as_ref(), key),
            Err(UpdateError::BadSignature)
        ));
        let other = key_pair();
        assert!(matches!(
            Manifest::verified(&json, signature.as_ref(), other.public_key().as_ref()),
            Err(UpdateError::BadSignature)
        ));
    }

    /// A manifest signed as the release workflow signs one, with OpenSSL
    /// (`openssl pkeyutl -sign -rawin`), under a key made for this test
    /// only.
    #[test]
    fn a_manifest_signed_by_the_release_workflow_verifies() {
        let key = base64::engine::general_purpose::STANDARD
            .decode("Ttl69db6GuFDYgQdtq5WrWEnEg4M7IvIJhlUt/SHdWQ=")
            .unwrap();
        let json = concat!(
            "{\n",
            "  \"version\": \"9.1.0\",\n",
            "  \"packages\": {\n",
            "    \"windows-x64\": {\n",
            "      \"name\": \"tpf3mp-9.1.0-windows-x64.zip\",\n",
            "      \"size\": 3,\n",
            "      \"sha256\": \"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\"\n",
            "    }\n",
            "  }\n",
            "}\n",
        );
        let signature: Vec<u8> = (0..64)
            .map(|at| {
                u8::from_str_radix(
                    &"a0eba348ce71fe42a0acebad26d8c61bbc95a630dfea7556a097c84f931a35a5\
                      e0c74503a3f05ea6406ef9fc4ba1157c3a12214c504b70f57842aa2a7c35070b"
                        [at * 2..at * 2 + 2],
                    16,
                )
                .unwrap()
            })
            .collect();
        let manifest = Manifest::verified(json.as_bytes(), &signature, &key).unwrap();
        assert_eq!(manifest.version, "9.1.0");
        assert_eq!(manifest.packages["windows-x64"].size, 3);
    }

    #[test]
    fn a_signed_manifest_still_names_only_plain_files() {
        let keys = key_pair();
        let json = br#"{"version":"9.1.0","packages":{"windows-x64":{"name":"../evil.zip","size":3,"sha256":"00"}}}"#;
        let signature = keys.sign(json);
        assert!(matches!(
            Manifest::verified(json, signature.as_ref(), keys.public_key().as_ref()),
            Err(UpdateError::Malformed(_))
        ));
    }

    fn zip_of(path: &Path, entries: &[(&str, &[u8])]) {
        let mut writer = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, data) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn a_package_unpacks_into_its_folder() {
        let dir = temp("unpack");
        let archive = dir.join("tpf3mp-9.1.0-windows-x64.zip");
        zip_of(
            &archive,
            &[
                ("tpf3mp-9.1.0-windows-x64/TPF3-MP.exe", b"new launcher"),
                ("tpf3mp-9.1.0-windows-x64/docs/PLAYING.md", b"how to play"),
            ],
        );
        let package = unpack(&archive, &dir.join("out")).unwrap();
        assert_eq!(
            fs::read(package.join("TPF3-MP.exe")).unwrap(),
            b"new launcher"
        );
        assert_eq!(
            fs::read(package.join("docs").join("PLAYING.md")).unwrap(),
            b"how to play"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_package_cannot_write_outside_its_folder() {
        let dir = temp("traversal");
        let archive = dir.join("evil.zip");
        zip_of(&archive, &[("tpf3mp/../../escaped.txt", b"x")]);
        assert!(matches!(
            unpack(&archive, &dir.join("out")),
            Err(UpdateError::UnsafeEntry(_))
        ));
        assert!(!dir.join("escaped.txt").exists());

        let tarball = dir.join("evil.tar.gz");
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            File::create(&tarball).unwrap(),
            flate2::Compression::fast(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        builder
            .append_link(&mut header, "tpf3mp/link", "/etc/passwd")
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();
        assert!(matches!(
            unpack(&tarball, &dir.join("out-tar")),
            Err(UpdateError::UnsafeEntry(_))
        ));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn installing_replaces_files_and_keeps_the_old_aside() {
        let dir = temp("swap");
        let root = dir.join("install");
        let package = dir.join("package");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(package.join("docs")).unwrap();
        fs::write(root.join("TPF3-MP.exe"), b"old").unwrap();
        fs::write(root.join("mods.txt"), b"the player's own").unwrap();
        fs::write(package.join("TPF3-MP.exe"), b"new").unwrap();
        fs::write(package.join("docs").join("PLAYING.md"), b"docs").unwrap();
        swap_in(&package, &root).unwrap();
        assert_eq!(fs::read(root.join("TPF3-MP.exe")).unwrap(), b"new");
        assert_eq!(fs::read(root.join("TPF3-MP.exe.old")).unwrap(), b"old");
        assert_eq!(
            fs::read(root.join("docs").join("PLAYING.md")).unwrap(),
            b"docs"
        );
        // What the package does not have stays.
        assert_eq!(
            fs::read(root.join("mods.txt")).unwrap(),
            b"the player's own"
        );
        remove_old(&root);
        assert!(!root.join("TPF3-MP.exe.old").exists());
        assert!(root.join("mods.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_install_puts_everything_back() {
        let dir = temp("roll-back");
        let root = dir.join("install");
        fs::create_dir_all(&root).unwrap();
        // As if a.txt had been replaced, and b.txt added, before a later
        // step failed.
        fs::write(root.join("a.txt.old"), b"old a").unwrap();
        fs::write(root.join("a.txt"), b"new a").unwrap();
        fs::write(root.join("b.txt"), b"new b").unwrap();
        roll_back(&[
            (root.join("a.txt"), Some(root.join("a.txt.old"))),
            (root.join("b.txt"), None),
        ]);
        assert!(!root.join("b.txt").exists());
        assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"old a");
        assert!(!root.join("a.txt.old").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_staged_update_is_checked_again_before_it_is_installed() {
        let dir = temp("staged");
        let root = dir.join("install");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join(PACKAGE_MARKER),
            br#"{"version":"0.1.0","platform":"windows-x64"}"#,
        )
        .unwrap();
        fs::write(root.join("TPF3-MP.exe"), b"old").unwrap();
        let install = Install::at(root.clone(), root.join("TPF3-MP.exe"), "windows-x64").unwrap();
        let keys = key_pair();
        let archive_name = "tpf3mp-9.1.0-windows-x64.zip";
        let staged = install.staging().join("9.1.0");
        fs::create_dir_all(&staged).unwrap();
        zip_of(
            &staged.join(archive_name),
            &[("tpf3mp-9.1.0-windows-x64/TPF3-MP.exe", b"new")],
        );
        let bytes = fs::read(staged.join(archive_name)).unwrap();
        let json = format!(
            r#"{{"version":"9.1.0","packages":{{"windows-x64":{{"name":"{archive_name}","size":{},"sha256":"{}"}}}}}}"#,
            bytes.len(),
            hex(ring::digest::digest(&SHA256, &bytes).as_ref())
        );
        fs::write(staged.join(MANIFEST), &json).unwrap();
        fs::write(staged.join(SIGNATURE), keys.sign(json.as_bytes())).unwrap();

        // Tampered with after the download: nothing is installed.
        let mut tampered = bytes.clone();
        let at = tampered.len() / 2;
        tampered[at] ^= 1;
        fs::write(staged.join(archive_name), &tampered).unwrap();
        assert_eq!(
            install_staged(&install, keys.public_key().as_ref()).unwrap(),
            None
        );
        assert_eq!(fs::read(root.join("TPF3-MP.exe")).unwrap(), b"old");

        // As downloaded: installed, and the staging folder cleared.
        fs::write(staged.join(archive_name), &bytes).unwrap();
        assert_eq!(
            install_staged(&install, keys.public_key().as_ref())
                .unwrap()
                .as_deref(),
            Some("9.1.0")
        );
        assert_eq!(fs::read(root.join("TPF3-MP.exe")).unwrap(), b"new");
        assert!(!install.staging().exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_a_marked_package_of_this_platform_updates() {
        let dir = temp("marker");
        let exe = dir.join("TPF3-MP.exe");
        assert!(Install::at(dir.clone(), exe.clone(), "windows-x64").is_err());
        fs::write(
            dir.join(PACKAGE_MARKER),
            br#"{"version":"0.1.0","platform":"linux-x64"}"#,
        )
        .unwrap();
        assert!(Install::at(dir.clone(), exe, "windows-x64").is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}

//! Keeps TPF3-MP up to date from the project's GitHub releases, installing
//! only what the project signed.
//!
//! A published release carries `release.json`, its version and, for each
//! platform, the package's file name, size and SHA-256, and
//! `release.json.sig`, an Ed25519 signature of that file made with the
//! project's update key when the release is published. The launcher is
//! built with the public keys it trusts (`TPF3MP_UPDATE_PUBLIC_KEY` at build
//! time, one or more); a build without one never updates. A package is
//! installed only if:
//!
//! - the signature verifies with a trusted key, and the signed version is
//!   newer than the running one and not one this copy rolled back from;
//! - the package's size and SHA-256 match the signed manifest, checked
//!   when it is downloaded and again before it is installed;
//! - it is the archive format of this platform, and every path in it is
//!   one plain name after another, inside the package, of files and
//!   folders only.
//!
//! The files come from `releases/latest/download/` and
//! `releases/download/v<version>/`, never GitHub's rate-limited API, over
//! HTTPS only.
//!
//! Installing is journalled in `.tpf3mp-update/` in the install folder,
//! where downloads wait too (the same disk, so every step is a rename):
//! before the first file moves, the journal names each one and where its
//! old version goes (`<name>.tpf3mp-<old version>.old`). An install that
//! fails or is cut short is undone from the journal, at once or at the
//! next start. The old files stay until the new version has shown its
//! window; a new version that fails to get that far three starts running
//! is rolled back, and that version is not installed again. Only the files
//! a journal names are ever deleted. One process at a time downloads or
//! installs, holding `.tpf3mp-update/lock`.
//!
//! The player chooses when to install, since restarting ends a game; an
//! update not installed then is installed at the next start.
//!
//! The player may also choose (D18): the Experimental track, which offers
//! pre-releases too (found through `releases.rs`), and any signed release,
//! older ones included, which is then installed as an update is and held:
//! while a chosen version is held, nothing is updated on its own until the
//! player resumes updates. Both are kept in `.tpf3mp-update/choice.json`.

use std::{
    ffi::OsString,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use base64::Engine;
use ring::{
    digest::{Context, SHA256},
    signature::{ED25519, UnparsedPublicKey},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};

/// This build's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The repository releases come from.
pub const REPOSITORY: &str = match option_env!("TPF3MP_REPOSITORY") {
    Some(repository) => repository,
    None => "Juliansgith/Transport-Fever-3-Multiplayer-Mod",
};
/// The public halves of the keys releases may be signed with, as base64 of
/// their 32 bytes, separated by commas or spaces. More than one lets the
/// project move to a new key. Without any, this build never updates.
const PUBLIC_KEYS: Option<&str> = option_env!("TPF3MP_UPDATE_PUBLIC_KEY");
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
/// Where downloads, the journal and the lock live in the install folder.
const STAGING: &str = ".tpf3mp-update";
const JOURNAL: &str = "journal.json";
const LOCK: &str = "lock";
/// Versions this copy rolled back from, which it does not install again.
const SKIP: &str = "skip.json";
/// The player's track and the version they chose to hold.
const CHOICE: &str = "choice.json";
const MANIFEST: &str = "release.json";
const SIGNATURE: &str = "release.json.sig";
/// Largest manifest read.
const MAX_MANIFEST: u64 = 64 * 1024;
/// Largest package downloaded.
const MAX_PACKAGE: u64 = 1 << 30;
/// Pause before the first check, so starting stays quick.
const FIRST_CHECK: Duration = Duration::from_secs(3);
/// How often a running launcher checks again.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// Starts a new version may take to show its window before it is rolled
/// back.
const MAX_UNCONFIRMED_STARTS: u32 = 3;
/// How long a starting launcher waits for another one's install.
const LOCK_WAIT: Duration = Duration::from_secs(60);

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
    #[error("another TPF3-MP is installing an update")]
    Busy,
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
    /// The player chose this version; nothing updates on its own.
    Held {
        version: String,
    },
}

/// What the player chose: the track, and a version to stay on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    #[serde(default)]
    pub track: crate::releases::Track,
    /// A version the player chose to install and stay on.
    #[serde(default)]
    pub hold: Option<String>,
}

/// Which release to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Which {
    /// The latest published release, from `releases/latest`.
    Latest,
    /// This version, which the player chose or the track offers.
    Version(String),
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
    /// The manifest in `json`, if `signature` is one of `keys`' signature
    /// of it.
    pub fn verified(json: &[u8], signature: &[u8], keys: &[Vec<u8>]) -> Result<Self, UpdateError> {
        let signed = keys.iter().any(|key| {
            UnparsedPublicKey::new(&ED25519, key)
                .verify(json, signature)
                .is_ok()
        });
        if !signed {
            return Err(UpdateError::BadSignature);
        }
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
        newer(&self.version, current)
    }

    /// This platform's package, in this platform's archive format.
    fn package(&self, platform: &str) -> Result<&Package, UpdateError> {
        let package = self.packages.get(platform).ok_or(UpdateError::NoPackage)?;
        if !package.name.ends_with(archive_suffix(platform)) {
            return Err(UpdateError::Malformed(format!(
                "{} is not a {} package",
                package.name, platform
            )));
        }
        if package.size > MAX_PACKAGE {
            return Err(UpdateError::Malformed("the package is too large".into()));
        }
        Ok(package)
    }
}

fn newer(offered: &str, current: &str) -> bool {
    match (
        semver::Version::parse(offered),
        semver::Version::parse(current),
    ) {
        (Ok(offered), Ok(current)) => offered > current,
        _ => false,
    }
}

/// The archive format a platform's package comes in.
fn archive_suffix(platform: &str) -> &'static str {
    if platform.starts_with("windows") {
        ".zip"
    } else {
        ".tar.gz"
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

/// The keys this build trusts. An entry that is not a key is left out.
fn public_keys() -> Vec<Vec<u8>> {
    parse_keys(PUBLIC_KEYS.unwrap_or_default())
}

fn parse_keys(text: &str) -> Vec<Vec<u8>> {
    text.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let key = base64::engine::general_purpose::STANDARD
                .decode(entry)
                .ok()?;
            (key.len() == 32).then_some(key)
        })
        .collect()
}

/// Where releases come from.
#[derive(Debug, Clone)]
pub struct Source {
    base: String,
    /// GitHub's API for the same repository, for the release list.
    api: String,
    https_only: bool,
}

impl Source {
    /// The project's releases on GitHub.
    pub fn github() -> Self {
        Self {
            base: format!("https://github.com/{REPOSITORY}"),
            api: format!("https://api.github.com/repos/{REPOSITORY}"),
            https_only: true,
        }
    }

    /// The API address the release list comes from.
    pub fn api(&self) -> &str {
        &self.api
    }

    fn latest(&self, file: &str) -> String {
        format!("{}/releases/latest/download/{file}", self.base)
    }

    fn of(&self, version: &str, file: &str) -> String {
        format!("{}/releases/download/v{version}/{file}", self.base)
    }

    fn agent(&self, timeout: Duration) -> ureq::Agent {
        ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .https_only(self.https_only)
                .timeout_connect(Some(Duration::from_secs(30)))
                .timeout_global(Some(timeout))
                .build(),
        )
    }
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

    /// Holds the updater's lock, waiting at most `wait` for another
    /// process to let go of it.
    fn lock(&self, wait: Duration) -> Result<Lock, UpdateError> {
        fs::create_dir_all(self.staging())?;
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.staging().join(LOCK))?;
        let until = Instant::now() + wait;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Lock(file)),
                Err(fs::TryLockError::WouldBlock) if Instant::now() < until => {
                    std::thread::sleep(Duration::from_millis(250));
                }
                Err(fs::TryLockError::WouldBlock) => return Err(UpdateError::Busy),
                Err(fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        }
    }

    fn skipped(&self) -> Vec<String> {
        fs::read(self.staging().join(SKIP))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// The player's choice, or the default: Stable, holding nothing.
    pub fn choice(&self) -> Choice {
        fs::read(self.staging().join(CHOICE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn choose(&self, choice: &Choice) -> io::Result<()> {
        fs::create_dir_all(self.staging())?;
        write_synced(&self.staging().join(CHOICE), &serde_json::to_vec(choice)?)
    }

    fn skip(&self, version: &str) -> io::Result<()> {
        let mut skipped = self.skipped();
        if !skipped.iter().any(|skipped| skipped == version) {
            skipped.push(version.to_owned());
        }
        write_synced(&self.staging().join(SKIP), &serde_json::to_vec(&skipped)?)
    }
}

/// The updater's lock, held until dropped.
struct Lock(#[allow(dead_code)] File);

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
    keys: Vec<Vec<u8>>,
    source: Source,
    runtime: tokio::runtime::Handle,
    /// One check or download at a time in this process.
    working: Mutex<()>,
}

impl Updater {
    /// An updater for this copy. It checks soon after starting and every
    /// few hours, and downloads what it finds; installing waits for the
    /// player.
    pub fn start(runtime: tokio::runtime::Handle) -> Self {
        let keys = public_keys();
        let install = Install::of_running();
        let state = match (keys.is_empty(), &install) {
            (true, _) => UpdateState::Off("this build has no update key".into()),
            (false, Err(reason)) => UpdateState::Off(reason.clone()),
            (false, Ok(_)) => UpdateState::Checking,
        };
        let updater = Self {
            inner: Arc::new(Inner {
                state: Mutex::new(state.clone()),
                install: install.ok(),
                keys,
                source: Source::github(),
                runtime,
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

    pub fn state(&self) -> UpdateState {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The window reads the state when it next looks.
    fn set(&self, state: UpdateState) {
        *self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = state;
    }

    /// Checks for a newer release now, and downloads it.
    pub fn check(&self) {
        let Some(install) = self.inner.install.clone() else {
            return;
        };
        if self.inner.keys.is_empty() {
            return;
        }
        let updater = self.clone();
        self.inner.runtime.spawn_blocking(move || {
            let Ok(_working) = updater.inner.working.try_lock() else {
                return;
            };
            // A downloaded update waits for the player; nothing to check.
            if matches!(updater.state(), UpdateState::Ready { .. }) {
                return;
            }
            let choice = install.choice();
            // The player chose a version: it stays until they resume.
            if let Some(version) = choice.hold {
                updater.set(UpdateState::Held { version });
                return;
            }
            updater.set(UpdateState::Checking);
            let which = match choice.track {
                crate::releases::Track::Stable => Ok(Which::Latest),
                crate::releases::Track::Experimental => updater.newest_experimental(),
            };
            let result = which.and_then(|which| {
                check_and_download(
                    &install,
                    &updater.inner.keys,
                    &updater.inner.source,
                    &which,
                    |version, bytes, total| {
                        updater.set(UpdateState::Downloading {
                            version: version.to_owned(),
                            bytes,
                            total,
                        });
                    },
                )
            });
            match result {
                Ok(Checked::Downloaded(version)) => {
                    info!(%version, "an update is ready to install");
                    updater.set(UpdateState::Ready { version });
                }
                Ok(Checked::UpToDate) => updater.set(UpdateState::UpToDate),
                Ok(Checked::Unsigned) => updater.set(UpdateState::Failed(
                    "the latest release is not signed for updates yet".into(),
                )),
                Err(error) => {
                    warn!(%error, "the update check failed");
                    updater.set(UpdateState::Failed(error.to_string()));
                }
            }
        });
    }

    /// The newest release on the Experimental track, if it is newer than
    /// this one; else the latest published release, as on Stable.
    fn newest_experimental(&self) -> Result<Which, UpdateError> {
        let (releases, _) = crate::releases::fetch(self.inner.source.api(), 1)?;
        Ok(
            match crate::releases::latest(&releases, crate::releases::Track::Experimental) {
                Some(release) if newer(&release.version, VERSION) => {
                    Which::Version(release.version.clone())
                }
                _ => Which::Latest,
            },
        )
    }

    /// The player's choice, if this copy updates at all.
    pub fn choice(&self) -> Option<Choice> {
        self.inner.install.as_ref().map(Install::choice)
    }

    /// Changes the track, and checks on it now.
    pub fn set_track(&self, track: crate::releases::Track) -> Result<(), String> {
        let install = self
            .inner
            .install
            .as_ref()
            .ok_or("this copy does not update itself")?;
        let mut choice = install.choice();
        choice.track = track;
        install.choose(&choice).map_err(|error| error.to_string())?;
        // What was downloaded for the other track waits no more.
        if matches!(self.state(), UpdateState::Ready { .. }) {
            self.set(UpdateState::Checking);
        }
        self.check();
        Ok(())
    }

    /// Lets updates come again after the player held a version.
    pub fn resume(&self) -> Result<(), String> {
        let install = self
            .inner
            .install
            .as_ref()
            .ok_or("this copy does not update itself")?;
        let mut choice = install.choice();
        choice.hold = None;
        install.choose(&choice).map_err(|error| error.to_string())?;
        self.set(UpdateState::Checking);
        self.check();
        Ok(())
    }

    /// Installs `version`, the player's choice, older or newer, and holds
    /// it. Blocks while it downloads. Returns whether the launcher must
    /// close now for it to start; with `version` already running, it is
    /// only held.
    pub fn install_version(&self, version: &str) -> Result<bool, String> {
        let install = self
            .inner
            .install
            .as_ref()
            .ok_or("this copy does not update itself")?;
        if self.inner.keys.is_empty() {
            return Err("this build has no update key".into());
        }
        semver::Version::parse(version).map_err(|_| format!("{version} is not a version"))?;
        let _working = self
            .inner
            .working
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let before = install.choice();
        let held = Choice {
            hold: Some(version.to_owned()),
            ..before.clone()
        };
        install.choose(&held).map_err(|error| error.to_string())?;
        if version == VERSION {
            self.set(UpdateState::Held {
                version: version.to_owned(),
            });
            return Ok(false);
        }
        let result = check_and_download(
            install,
            &self.inner.keys,
            &self.inner.source,
            &Which::Version(version.to_owned()),
            |version, bytes, total| {
                self.set(UpdateState::Downloading {
                    version: version.to_owned(),
                    bytes,
                    total,
                });
            },
        );
        let failed = |why: String| {
            // Nothing changed: the choice goes back to what it was.
            let _ = install.choose(&before);
            self.set(UpdateState::Failed(why.clone()));
            Err(why)
        };
        match result {
            Ok(Checked::Downloaded(downloaded)) => {
                info!(version = %downloaded, "the chosen version is ready to install");
                self.set(UpdateState::Ready {
                    version: downloaded,
                });
            }
            Ok(Checked::Unsigned) => {
                return failed(format!("release {version} is not signed for installing"));
            }
            Ok(Checked::UpToDate) => return Ok(false),
            Err(error) => return failed(error.to_string()),
        }
        drop(_working);
        Ok(self.install_and_restart())
    }

    /// Installs the downloaded update and starts it. Returns whether it
    /// did, and so this launcher must close now. On failure everything
    /// stays as it was, and the state says why.
    pub fn install_and_restart(&self) -> bool {
        let Some(install) = &self.inner.install else {
            return false;
        };
        let UpdateState::Ready { version } = self.state() else {
            return false;
        };
        self.set(UpdateState::Installing {
            version: version.clone(),
        });
        let installed = install.lock(Duration::ZERO).and_then(|_lock| {
            let chosen = install.choice().hold.filter(|held| held != VERSION);
            install_staged(install, &self.inner.keys, chosen.as_deref())
        });
        match installed.and_then(|installed| {
            if installed.is_some() {
                restart(install)?;
            }
            Ok(installed)
        }) {
            Ok(Some(installed)) => {
                info!(version = %installed, "installed the update; restarting");
                true
            }
            Ok(None) => {
                self.set(UpdateState::UpToDate);
                false
            }
            Err(error) => {
                warn!(%error, "cannot install the update");
                self.set(UpdateState::Failed(format!("cannot install: {error}")));
                false
            }
        }
    }
}

/// What a check found.
#[derive(Debug, PartialEq, Eq)]
enum Checked {
    UpToDate,
    /// The latest release has no signed manifest (yet).
    Unsigned,
    /// A newer version, downloaded and checked.
    Downloaded(String),
}

/// What the journal records about an install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Journal {
    from: String,
    to: String,
    /// Each name the install puts in place, and where the file or folder
    /// it replaces was moved, if there was one.
    entries: Vec<Entry>,
    state: Stage,
    /// Starts of the new version that did not reach its window.
    starts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    name: String,
    old: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    /// Files are being moved: undo it if found at a start.
    Swapping,
    /// Every file is in place; the old ones wait for the new version to
    /// show its window.
    Swapped,
}

impl Journal {
    fn path(install: &Install) -> PathBuf {
        install.staging().join(JOURNAL)
    }

    fn read(install: &Install) -> Option<Self> {
        let bytes = fs::read(Self::path(install)).ok()?;
        match serde_json::from_slice(&bytes) {
            Ok(journal) => Some(journal),
            Err(error) => {
                warn!(%error, "the update journal is unreadable; leaving it for a person to look at");
                None
            }
        }
    }

    fn write(&self, install: &Install) -> io::Result<()> {
        fs::create_dir_all(install.staging())?;
        write_synced(&Self::path(install), &serde_json::to_vec_pretty(self)?)
    }

    fn remove(install: &Install) -> io::Result<()> {
        match fs::remove_file(Self::path(install)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    /// Puts back what the install replaced, and takes away what it added.
    /// Returns whether everything could be put back.
    fn roll_back(&self, root: &Path) -> bool {
        let mut complete = true;
        for entry in self.entries.iter().rev() {
            let target = root.join(&entry.name);
            match &entry.old {
                Some(old) => {
                    let old = root.join(old);
                    if !exists(&old) {
                        // Never moved aside: the original is still in place.
                        continue;
                    }
                    if exists(&target)
                        && let Err(error) = remove(&target)
                    {
                        warn!(path = %target.display(), %error, "cannot remove a new file while rolling back");
                        complete = false;
                        continue;
                    }
                    if let Err(error) = fs::rename(&old, &target) {
                        warn!(path = %target.display(), %error, "cannot put an old file back");
                        complete = false;
                    }
                }
                None => {
                    if exists(&target)
                        && let Err(error) = remove(&target)
                    {
                        warn!(path = %target.display(), %error, "cannot remove a new file while rolling back");
                        complete = false;
                    }
                }
            }
        }
        complete
    }

    /// Deletes the old files the install moved aside.
    fn remove_old(&self, root: &Path) {
        for old in self.entries.iter().filter_map(|entry| entry.old.as_ref()) {
            let path = root.join(old);
            if exists(&path)
                && let Err(error) = remove(&path)
            {
                warn!(path = %path.display(), %error, "cannot delete a file an update replaced");
            }
        }
    }
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn remove(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path)?.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let partial = path.with_extension("partial");
    let mut file = File::create(&partial)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&partial, path)
}

/// At start, before anything else: finishes or undoes an install the last
/// run left, and installs an update downloaded earlier. Returns whether
/// the launcher restarted into another version and should exit now.
pub fn at_start() -> bool {
    let Ok(install) = Install::of_running() else {
        return false;
    };
    let keys = public_keys();
    match start(&install, &keys, VERSION) {
        Ok(Started::Restart) => match restart(&install) {
            Ok(()) => true,
            Err(error) => {
                warn!(%error, "cannot restart after the update");
                false
            }
        },
        Ok(Started::Continue) => false,
        Err(error) => {
            warn!(%error, "cannot finish the update");
            false
        }
    }
}

/// What a starting launcher does next.
#[derive(Debug, PartialEq, Eq)]
enum Started {
    Continue,
    /// Another version is now in place: run it.
    Restart,
}

fn start(install: &Install, keys: &[Vec<u8>], running: &str) -> Result<Started, UpdateError> {
    let _lock = install.lock(LOCK_WAIT)?;
    clear_unpacked(install);
    if let Some(mut journal) = Journal::read(install) {
        match journal.state {
            // Cut short while moving files: put everything back.
            Stage::Swapping => {
                info!(to = %journal.to, "undoing an update that did not finish");
                if journal.roll_back(&install.root) {
                    Journal::remove(install)?;
                }
                return Ok(if running == journal.from {
                    Started::Continue
                } else {
                    Started::Restart
                });
            }
            // The new version is starting: it has a few tries to show its
            // window before the old one comes back.
            Stage::Swapped if running == journal.to => {
                journal.starts += 1;
                if journal.starts > MAX_UNCONFIRMED_STARTS {
                    warn!(version = %journal.to, "the new version never started; going back");
                    if journal.roll_back(&install.root) {
                        install.skip(&journal.to)?;
                        Journal::remove(install)?;
                        return Ok(Started::Restart);
                    }
                } else {
                    journal.write(install)?;
                }
                return Ok(Started::Continue);
            }
            Stage::Swapped => return Ok(Started::Continue),
        }
    }
    if keys.is_empty() {
        return Ok(Started::Continue);
    }
    // A held version installs only as the player's choice, and once it
    // runs nothing else installs on its own.
    let chosen = install.choice().hold;
    if chosen.as_deref() == Some(running) {
        return Ok(Started::Continue);
    }
    match install_staged(install, keys, chosen.as_deref())? {
        Some(version) => {
            info!(%version, "installed the update downloaded earlier; restarting");
            Ok(Started::Restart)
        }
        None => Ok(Started::Continue),
    }
}

/// Tells the updater this version reached its window: the files the last
/// install moved aside can go.
pub fn started() {
    let Ok(install) = Install::of_running() else {
        return;
    };
    confirm(&install, VERSION);
}

fn confirm(install: &Install, running: &str) {
    let Ok(_lock) = install.lock(Duration::from_secs(5)) else {
        return;
    };
    if let Some(journal) = Journal::read(install)
        && journal.state == Stage::Swapped
        && journal.to == running
    {
        journal.remove_old(&install.root);
        if let Err(error) = Journal::remove(install) {
            warn!(%error, "cannot remove the update journal");
        }
        info!(version = %running, "the update is complete");
    }
}

/// Deletes what earlier installs unpacked. Called holding the lock, so no
/// other process is unpacking.
fn clear_unpacked(install: &Install) {
    let Ok(entries) = fs::read_dir(install.staging()) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().contains(".unpacked") {
            let _ = fs::remove_dir_all(entry.path());
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

fn get(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, UpdateError> {
    let mut response = agent
        .get(url)
        .header("User-Agent", format!("tpf3mp-launcher/{VERSION}"))
        .call()?;
    Ok(response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()?)
}

/// Fetches `which` release and downloads its package: the latest only if
/// it is newer and not rolled back from, a chosen version whatever it is.
fn check_and_download(
    install: &Install,
    keys: &[Vec<u8>],
    source: &Source,
    which: &Which,
    mut progress: impl FnMut(&str, u64, u64),
) -> Result<Checked, UpdateError> {
    let agent = source.agent(Duration::from_secs(60));
    let url = |file: &str| match which {
        Which::Latest => source.latest(file),
        Which::Version(version) => source.of(version, file),
    };
    let json = match get(&agent, &url(MANIFEST), MAX_MANIFEST) {
        Ok(json) => json,
        // Published but not signed yet, or no release at all.
        Err(UpdateError::Http(error)) if error.contains("404") => return Ok(Checked::Unsigned),
        Err(error) => return Err(error),
    };
    let signature = get(&agent, &url(SIGNATURE), 256)?;
    let manifest = Manifest::verified(&json, &signature, keys)?;
    match which {
        Which::Latest => {
            if !manifest.newer_than(VERSION) || install.skipped().contains(&manifest.version) {
                return Ok(Checked::UpToDate);
            }
        }
        // The signed manifest must be the version asked for, not another
        // one served in its place.
        Which::Version(version) => {
            if manifest.version != *version {
                return Err(UpdateError::Malformed(format!(
                    "release {version} is signed as {}",
                    manifest.version
                )));
            }
            if manifest.version == VERSION {
                return Ok(Checked::UpToDate);
            }
        }
    }
    let package = manifest.package(install.platform)?;
    let _lock = install.lock(Duration::ZERO)?;
    let dir = install.staging().join(&manifest.version);
    fs::create_dir_all(&dir)?;
    let archive = dir.join(&package.name);
    if !matches(&archive, package)? {
        let partial = dir.join(format!("{}.part", package.name));
        let big = source.agent(Duration::from_secs(3 * 60 * 60));
        let url = source.of(&manifest.version, &package.name);
        let downloaded = download(&big, &url, &partial, package, |bytes| {
            progress(&manifest.version, bytes, package.size);
        });
        if let Err(error) = downloaded {
            let _ = fs::remove_file(&partial);
            return Err(error);
        }
        fs::rename(&partial, &archive)?;
    }
    // The manifest and signature go with the package, to check it again
    // when it is installed.
    write_synced(&dir.join(MANIFEST), &json)?;
    write_synced(&dir.join(SIGNATURE), &signature)?;
    Ok(Checked::Downloaded(manifest.version))
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

/// Installs a downloaded release that still checks out: the version the
/// player chose, `chosen`, whatever it is, or else the newest one newer
/// than this version and not skipped. Returns its version, or `None` if
/// there is none. Call it holding the lock.
fn install_staged(
    install: &Install,
    keys: &[Vec<u8>],
    chosen: Option<&str>,
) -> Result<Option<String>, UpdateError> {
    let Some((manifest, archive)) = newest_staged(install, keys, chosen)? else {
        return Ok(None);
    };
    let unpacked = install.staging().join(format!(
        "{}.unpacked.{}",
        manifest.version,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&unpacked);
    let package = unpack(&archive, &unpacked, install.platform)?;
    swap_in(install, &package, &manifest.version)?;
    let _ = fs::remove_dir_all(&unpacked);
    let _ = fs::remove_dir_all(install.staging().join(&manifest.version));
    Ok(Some(manifest.version))
}

/// The staged release to install, whose signature and package check out:
/// `chosen` if given, or else the newest newer than this version. Staged
/// releases that are neither are deleted.
fn newest_staged(
    install: &Install,
    keys: &[Vec<u8>],
    chosen: Option<&str>,
) -> Result<Option<(Manifest, PathBuf)>, UpdateError> {
    let Ok(entries) = fs::read_dir(install.staging()) else {
        return Ok(None);
    };
    let skipped = install.skipped();
    let mut best: Option<(Manifest, PathBuf)> = None;
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() || entry.file_name().to_string_lossy().contains(".unpacked") {
            continue;
        }
        let (Ok(json), Ok(signature)) =
            (fs::read(dir.join(MANIFEST)), fs::read(dir.join(SIGNATURE)))
        else {
            continue;
        };
        let Ok(manifest) = Manifest::verified(&json, &signature, keys) else {
            warn!(dir = %dir.display(), "a downloaded update does not verify; deleting it");
            let _ = fs::remove_dir_all(&dir);
            continue;
        };
        let wanted = match chosen {
            Some(chosen) => manifest.version == chosen,
            None => manifest.newer_than(VERSION) && !skipped.contains(&manifest.version),
        };
        if !wanted {
            let _ = fs::remove_dir_all(&dir);
            continue;
        }
        let Ok(package) = manifest.package(install.platform) else {
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

/// Unpacks `archive`, a package of `platform`, into `into` and returns the
/// package folder in it: the single folder at the archive's top. Refuses
/// another platform's archive format, any entry that would land outside
/// `into`, and anything but files and folders.
pub fn unpack(archive: &Path, into: &Path, platform: &str) -> Result<PathBuf, UpdateError> {
    fs::create_dir_all(into)?;
    let name = archive.to_string_lossy();
    if !name.ends_with(archive_suffix(platform)) {
        return Err(UpdateError::Malformed(format!(
            "{name} is not a {platform} package"
        )));
    }
    if name.ends_with(".zip") {
        unpack_zip(archive, into)?;
    } else {
        unpack_tar_gz(archive, into)?;
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

/// `path` inside `into`, if every part of it is one plain name.
fn contained(into: &Path, path: &Path) -> Result<PathBuf, UpdateError> {
    let unsafe_entry = || UpdateError::UnsafeEntry(path.display().to_string());
    let mut out = into.to_owned();
    let mut any = false;
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_str().ok_or_else(unsafe_entry)?;
                // A drive (C:) or another separator would change where the
                // rest lands on Windows.
                if part.contains([':', '\\', '/']) {
                    return Err(unsafe_entry());
                }
                out.push(part);
                any = true;
            }
            Component::CurDir => {}
            _ => return Err(unsafe_entry()),
        }
    }
    if any && out.starts_with(into) && out != into {
        Ok(out)
    } else {
        Err(unsafe_entry())
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
        if matches!(
            kind,
            tar::EntryType::XGlobalHeader | tar::EntryType::XHeader
        ) {
            // Metadata for the entries after it.
            continue;
        }
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
        } else {
            return Err(UpdateError::UnsafeEntry(name.display().to_string()));
        }
    }
    Ok(())
}

/// Puts everything at the top of `package` in place of the same names in
/// the install folder, recording each step in the journal first, and
/// moving what was there aside for the new version to confirm. If a step
/// fails, puts everything back.
fn swap_in(install: &Install, package: &Path, to: &str) -> Result<(), UpdateError> {
    let root = &install.root;
    let mut names: Vec<String> = Vec::new();
    for entry in fs::read_dir(package)? {
        let name = entry?.file_name();
        let name = name
            .to_str()
            .filter(|name| !name.is_empty() && !name.contains([':', '\\', '/']))
            .ok_or_else(|| UpdateError::UnsafeEntry(name.to_string_lossy().into_owned()))?;
        names.push(name.to_owned());
    }
    names.sort();
    let suffix = format!(".tpf3mp-{VERSION}.old");
    let mut journal = Journal {
        from: VERSION.to_owned(),
        to: to.to_owned(),
        entries: names
            .iter()
            .map(|name| Entry {
                name: name.clone(),
                old: exists(&root.join(name)).then(|| format!("{name}{suffix}")),
            })
            .collect(),
        state: Stage::Swapping,
        starts: 0,
    };
    journal.write(install)?;
    let moved = (|| -> Result<(), UpdateError> {
        for entry in &journal.entries {
            let target = root.join(&entry.name);
            if let Some(old) = &entry.old {
                let old = root.join(old);
                if exists(&old) {
                    remove(&old)?;
                }
                fs::rename(&target, &old)?;
            }
            fs::rename(package.join(&entry.name), &target)?;
        }
        Ok(())
    })();
    match moved {
        Ok(()) => {
            journal.state = Stage::Swapped;
            journal.write(install)?;
            Ok(())
        }
        Err(error) => {
            if journal.roll_back(root) {
                let _ = Journal::remove(install);
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::BufRead,
        net::TcpListener,
        sync::atomic::{AtomicBool, Ordering},
    };

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

    fn keys(pair: &Ed25519KeyPair) -> Vec<Vec<u8>> {
        vec![pair.public_key().as_ref().to_vec()]
    }

    fn manifest_json(version: &str) -> Vec<u8> {
        format!(
            r#"{{"version":"{version}","packages":{{"windows-x64":{{"name":"tpf3mp-{version}-windows-x64.zip","size":3,"sha256":"{}"}}}}}}"#,
            "ab".repeat(32)
        )
        .into_bytes()
    }

    #[test]
    fn only_a_release_signed_with_a_trusted_key_verifies() {
        let pair = key_pair();
        let json = manifest_json("9.1.0");
        let signature = pair.sign(&json);
        let manifest = Manifest::verified(&json, signature.as_ref(), &keys(&pair)).unwrap();
        assert_eq!(manifest.version, "9.1.0");
        assert!(manifest.newer_than("0.1.0"));
        assert!(!manifest.newer_than("9.1.0"));
        assert!(!manifest.newer_than("10.0.0"));

        let mut tampered = json.clone();
        let at = tampered.len() - 10;
        tampered[at] ^= 1;
        assert!(matches!(
            Manifest::verified(&tampered, signature.as_ref(), &keys(&pair)),
            Err(UpdateError::BadSignature)
        ));
        let other = key_pair();
        assert!(matches!(
            Manifest::verified(&json, signature.as_ref(), &keys(&other)),
            Err(UpdateError::BadSignature)
        ));
        // A new key alongside the old: either signs.
        let both = [keys(&other), keys(&pair)].concat();
        assert!(Manifest::verified(&json, signature.as_ref(), &both).is_ok());
        assert!(matches!(
            Manifest::verified(&json, signature.as_ref(), &[]),
            Err(UpdateError::BadSignature)
        ));
    }

    #[test]
    fn keys_are_read_from_a_list() {
        let key = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
        let other = base64::engine::general_purpose::STANDARD.encode([8u8; 32]);
        assert_eq!(parse_keys(&key).len(), 1);
        assert_eq!(parse_keys(&format!("{key}, {other}")).len(), 2);
        assert_eq!(parse_keys(&format!("{key} not-a-key")).len(), 1);
        assert!(parse_keys("").is_empty());
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
        let manifest = Manifest::verified(json.as_bytes(), &signature, &[key]).unwrap();
        assert_eq!(manifest.version, "9.1.0");
        assert_eq!(manifest.packages["windows-x64"].size, 3);
    }

    #[test]
    fn a_signed_manifest_still_names_only_plain_files() {
        let pair = key_pair();
        let json = br#"{"version":"9.1.0","packages":{"windows-x64":{"name":"../evil.zip","size":3,"sha256":"00"}}}"#;
        let signature = pair.sign(json);
        assert!(matches!(
            Manifest::verified(json, signature.as_ref(), &keys(&pair)),
            Err(UpdateError::Malformed(_))
        ));
    }

    #[test]
    fn a_package_must_be_its_platforms_archive() {
        let pair = key_pair();
        let json = format!(
            r#"{{"version":"9.1.0","packages":{{"windows-x64":{{"name":"tpf3mp-9.1.0-windows-x64.tar.gz","size":3,"sha256":"{}"}}}}}}"#,
            "ab".repeat(32)
        );
        let manifest = Manifest::verified(
            json.as_bytes(),
            pair.sign(json.as_bytes()).as_ref(),
            &keys(&pair),
        )
        .unwrap();
        assert!(matches!(
            manifest.package("windows-x64"),
            Err(UpdateError::Malformed(_))
        ));
        assert!(matches!(
            manifest.package("linux-x64"),
            Err(UpdateError::NoPackage)
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
        let package = unpack(&archive, &dir.join("out"), "windows-x64").unwrap();
        assert_eq!(
            fs::read(package.join("TPF3-MP.exe")).unwrap(),
            b"new launcher"
        );
        assert_eq!(
            fs::read(package.join("docs").join("PLAYING.md")).unwrap(),
            b"how to play"
        );
        // Another platform's archive format is refused.
        assert!(matches!(
            unpack(&archive, &dir.join("out-linux"), "linux-x64"),
            Err(UpdateError::Malformed(_))
        ));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_package_cannot_write_outside_its_folder() {
        let dir = temp("traversal");
        let archive = dir.join("evil.zip");
        zip_of(&archive, &[("tpf3mp/../../escaped.txt", b"x")]);
        assert!(matches!(
            unpack(&archive, &dir.join("out"), "windows-x64"),
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
            unpack(&tarball, &dir.join("out-tar"), "linux-x64"),
            Err(UpdateError::UnsafeEntry(_))
        ));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_path_with_a_drive_or_backslash_is_refused() {
        let into = Path::new("install");
        assert!(contained(into, Path::new("top/C:evil.exe")).is_err());
        assert!(contained(into, Path::new("top/D:/x")).is_err());
        // A backslash separates names on Windows, and is refused inside a
        // name elsewhere, where Windows would read it as a separator.
        let backslash = contained(into, Path::new("top/a\\b"));
        if cfg!(windows) {
            assert_eq!(backslash.unwrap(), into.join("top").join("a").join("b"));
        } else {
            assert!(backslash.is_err());
        }
        assert!(contained(into, Path::new("..")).is_err());
        assert!(contained(into, Path::new(".")).is_err());
        assert_eq!(
            contained(into, Path::new("top/TPF3-MP.exe")).unwrap(),
            into.join("top").join("TPF3-MP.exe")
        );
    }

    /// An install folder of `platform` with an old launcher, a file of the
    /// player's own that ends in `.old`, and the package marker.
    fn installed(name: &str) -> (PathBuf, Install) {
        let dir = temp(name);
        let root = dir.join("install");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join(PACKAGE_MARKER),
            br#"{"version":"0.1.0","platform":"windows-x64"}"#,
        )
        .unwrap();
        fs::write(root.join("TPF3-MP.exe"), b"old").unwrap();
        fs::write(root.join("saves.old"), b"the player's own").unwrap();
        let install = Install::at(root.clone(), root.join("TPF3-MP.exe"), "windows-x64").unwrap();
        (dir, install)
    }

    /// Stages version `version` of a package holding `files` as downloaded,
    /// signed by `pair`, and returns the archive's bytes.
    fn stage(
        install: &Install,
        pair: &Ed25519KeyPair,
        version: &str,
        files: &[(&str, &[u8])],
    ) -> Vec<u8> {
        let name = format!("tpf3mp-{version}-windows-x64.zip");
        let staged = install.staging().join(version);
        fs::create_dir_all(&staged).unwrap();
        let top = format!("tpf3mp-{version}-windows-x64");
        let entries: Vec<(String, &[u8])> = files
            .iter()
            .map(|(path, data)| (format!("{top}/{path}"), *data))
            .collect();
        let borrowed: Vec<(&str, &[u8])> = entries
            .iter()
            .map(|(path, data)| (path.as_str(), *data))
            .collect();
        zip_of(&staged.join(&name), &borrowed);
        let bytes = fs::read(staged.join(&name)).unwrap();
        let json = manifest_for(version, &name, &bytes);
        fs::write(staged.join(MANIFEST), &json).unwrap();
        fs::write(staged.join(SIGNATURE), pair.sign(json.as_bytes())).unwrap();
        bytes
    }

    fn manifest_for(version: &str, name: &str, bytes: &[u8]) -> String {
        format!(
            r#"{{"version":"{version}","packages":{{"windows-x64":{{"name":"{name}","size":{},"sha256":"{}"}}}}}}"#,
            bytes.len(),
            hex(ring::digest::digest(&SHA256, bytes).as_ref())
        )
    }

    #[test]
    fn a_staged_update_is_checked_again_before_it_is_installed() {
        let (dir, install) = installed("staged");
        let pair = key_pair();
        let bytes = stage(&install, &pair, "9.1.0", &[("TPF3-MP.exe", b"new")]);
        let archive = install
            .staging()
            .join("9.1.0")
            .join("tpf3mp-9.1.0-windows-x64.zip");
        // Tampered with after the download: nothing is installed.
        let mut tampered = bytes.clone();
        let at = tampered.len() / 2;
        tampered[at] ^= 1;
        fs::write(&archive, &tampered).unwrap();
        assert_eq!(install_staged(&install, &keys(&pair), None).unwrap(), None);
        assert_eq!(fs::read(install.root.join("TPF3-MP.exe")).unwrap(), b"old");
        // As downloaded: installed.
        fs::write(&archive, &bytes).unwrap();
        assert_eq!(
            install_staged(&install, &keys(&pair), None)
                .unwrap()
                .as_deref(),
            Some("9.1.0")
        );
        assert_eq!(fs::read(install.root.join("TPF3-MP.exe")).unwrap(), b"new");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn old_files_wait_for_the_new_version_to_start_and_nothing_else_goes() {
        let (dir, install) = installed("confirm");
        let pair = key_pair();
        stage(
            &install,
            &pair,
            "9.1.0",
            &[("TPF3-MP.exe", b"new"), ("docs.md", b"docs")],
        );
        install_staged(&install, &keys(&pair), None).unwrap();
        let old = format!("TPF3-MP.exe.tpf3mp-{VERSION}.old");
        assert_eq!(fs::read(install.root.join(&old)).unwrap(), b"old");
        let journal = Journal::read(&install).unwrap();
        assert_eq!(journal.state, Stage::Swapped);
        // The new version starts, and shows its window.
        assert_eq!(
            start(&install, &keys(&pair), "9.1.0").unwrap(),
            Started::Continue
        );
        assert!(
            install.root.join(&old).exists(),
            "kept until the window shows"
        );
        confirm(&install, "9.1.0");
        assert!(!install.root.join(&old).exists());
        assert!(Journal::read(&install).is_none());
        // Only what the journal named went.
        assert_eq!(
            fs::read(install.root.join("saves.old")).unwrap(),
            b"the player's own"
        );
        assert_eq!(fs::read(install.root.join("docs.md")).unwrap(), b"docs");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_install_cut_short_is_undone_at_the_next_start() {
        let (dir, install) = installed("cut-short");
        // As if the process died after moving the old launcher aside and
        // putting the new one in place, before the rest.
        let old = format!("TPF3-MP.exe.tpf3mp-{VERSION}.old");
        fs::rename(install.root.join("TPF3-MP.exe"), install.root.join(&old)).unwrap();
        fs::write(install.root.join("TPF3-MP.exe"), b"new").unwrap();
        fs::write(install.root.join("added.txt"), b"new file").unwrap();
        Journal {
            from: VERSION.into(),
            to: "9.1.0".into(),
            entries: vec![
                Entry {
                    name: "TPF3-MP.exe".into(),
                    old: Some(old.clone()),
                },
                Entry {
                    name: "added.txt".into(),
                    old: None,
                },
                Entry {
                    name: "zz-not-reached.txt".into(),
                    old: Some("zz-not-reached.txt.old-never-made".into()),
                },
            ],
            state: Stage::Swapping,
            starts: 0,
        }
        .write(&install)
        .unwrap();
        let pair = key_pair();
        assert_eq!(
            start(&install, &keys(&pair), VERSION).unwrap(),
            Started::Continue
        );
        assert_eq!(fs::read(install.root.join("TPF3-MP.exe")).unwrap(), b"old");
        assert!(!install.root.join(&old).exists());
        assert!(!install.root.join("added.txt").exists());
        assert!(Journal::read(&install).is_none());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_version_that_never_starts_is_rolled_back_and_skipped() {
        let (dir, install) = installed("never-starts");
        let pair = key_pair();
        stage(&install, &pair, "9.1.0", &[("TPF3-MP.exe", b"broken")]);
        install_staged(&install, &keys(&pair), None).unwrap();
        for _ in 0..MAX_UNCONFIRMED_STARTS {
            assert_eq!(
                start(&install, &keys(&pair), "9.1.0").unwrap(),
                Started::Continue
            );
        }
        // One start too many without a window: back to the old version.
        assert_eq!(
            start(&install, &keys(&pair), "9.1.0").unwrap(),
            Started::Restart
        );
        assert_eq!(fs::read(install.root.join("TPF3-MP.exe")).unwrap(), b"old");
        assert!(install.skipped().contains(&"9.1.0".to_owned()));
        // The same version is not installed again.
        stage(&install, &pair, "9.1.0", &[("TPF3-MP.exe", b"broken")]);
        assert_eq!(install_staged(&install, &keys(&pair), None).unwrap(), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn one_process_at_a_time_installs() {
        let (dir, install) = installed("lock");
        let held = install.lock(Duration::ZERO).unwrap();
        assert!(matches!(
            install.lock(Duration::ZERO),
            Err(UpdateError::Busy)
        ));
        drop(held);
        assert!(install.lock(Duration::ZERO).is_ok());
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

    /// A small web server that answers GitHub's release URLs from `files`
    /// (path to body) with redirects as GitHub gives them, and 404 else.
    fn serve(files: Vec<(String, Vec<u8>)>) -> (String, Arc<AtomicBool>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(mut stream) = stream else { continue };
                let mut reader = io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let path = line.split(' ').nth(1).unwrap_or("/").to_owned();
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
                        break;
                    }
                }
                let answer = match files.iter().find(|(served, _)| *served == path) {
                    Some((_, body)) => {
                        let mut answer = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        answer.extend_from_slice(body);
                        answer
                    }
                    None => {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    }
                };
                let _ = stream.write_all(&answer);
            }
        });
        (base, stop)
    }

    #[test]
    fn a_release_is_found_downloaded_and_installed_end_to_end() {
        let (dir, install) = installed("end-to-end");
        let pair = key_pair();
        let top = "tpf3mp-9.1.0-windows-x64";
        let name = format!("{top}.zip");
        let archive = dir.join(&name);
        zip_of(
            &archive,
            &[
                (&format!("{top}/TPF3-MP.exe"), b"new launcher"),
                (
                    &format!("{top}/{PACKAGE_MARKER}"),
                    br#"{"version":"9.1.0","platform":"windows-x64"}"#,
                ),
            ],
        );
        let bytes = fs::read(&archive).unwrap();
        let json = manifest_for("9.1.0", &name, &bytes);
        let signature = pair.sign(json.as_bytes()).as_ref().to_vec();
        let (base, stop) = serve(vec![
            (
                "/releases/latest/download/release.json".into(),
                json.clone().into_bytes(),
            ),
            (
                "/releases/latest/download/release.json.sig".into(),
                signature,
            ),
            (format!("/releases/download/v9.1.0/{name}"), bytes),
        ]);
        let source = Source {
            base: base.clone(),
            api: format!("{base}/api"),
            https_only: false,
        };
        let mut seen = 0;
        let checked = check_and_download(
            &install,
            &keys(&pair),
            &source,
            &Which::Latest,
            |_, bytes, _| {
                seen = bytes;
            },
        )
        .unwrap();
        assert_eq!(checked, Checked::Downloaded("9.1.0".into()));
        assert!(seen > 0, "progress is reported");
        // Installed at the next start, as if the player waited.
        assert_eq!(
            start(&install, &keys(&pair), VERSION).unwrap(),
            Started::Restart
        );
        assert_eq!(
            fs::read(install.root.join("TPF3-MP.exe")).unwrap(),
            b"new launcher"
        );
        // A release signed by another key is not downloaded.
        let other = key_pair();
        let checked = check_and_download(
            &install,
            &keys(&other),
            &source,
            &Which::Latest,
            |_, _, _| {},
        );
        assert!(matches!(checked, Err(UpdateError::BadSignature)));
        // No signed manifest: nothing to do, and said so.
        let unsigned = Source {
            base: format!("{base}/nothing"),
            api: format!("{base}/nothing/api"),
            https_only: false,
        };
        assert_eq!(
            check_and_download(
                &install,
                &keys(&pair),
                &unsigned,
                &Which::Latest,
                |_, _, _| {}
            )
            .unwrap(),
            Checked::Unsigned
        );
        stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(base.trim_start_matches("http://"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_real_source_is_https_only() {
        let source = Source::github();
        assert!(source.https_only);
        assert!(source.base.starts_with("https://github.com/"));
        assert!(
            source
                .latest(MANIFEST)
                .ends_with("/releases/latest/download/release.json")
        );
        assert!(
            source
                .of("9.1.0", "x.zip")
                .ends_with("/releases/download/v9.1.0/x.zip")
        );
    }

    #[test]
    fn a_chosen_version_installs_though_it_is_older() {
        let (dir, install) = installed("chosen");
        let pair = key_pair();
        stage(&install, &pair, "0.0.9", &[("TPF3-MP.exe", b"older")]);
        // Not chosen: an older release is never installed on its own.
        assert_eq!(install_staged(&install, &keys(&pair), None).unwrap(), None);
        assert!(
            !install.staging().join("0.0.9").exists(),
            "and it is cleared away"
        );
        stage(&install, &pair, "0.0.9", &[("TPF3-MP.exe", b"older")]);
        stage(&install, &pair, "9.1.0", &[("TPF3-MP.exe", b"newer")]);
        // Chosen: exactly that one, though a newer one waits too.
        assert_eq!(
            install_staged(&install, &keys(&pair), Some("0.0.9"))
                .unwrap()
                .as_deref(),
            Some("0.0.9")
        );
        assert_eq!(
            fs::read(install.root.join("TPF3-MP.exe")).unwrap(),
            b"older"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_held_version_stays_until_the_player_resumes() {
        let (dir, install) = installed("held");
        let pair = key_pair();
        assert_eq!(install.choice(), Choice::default());
        install
            .choose(&Choice {
                track: crate::releases::Track::Experimental,
                hold: Some(VERSION.to_owned()),
            })
            .unwrap();
        assert_eq!(install.choice().hold.as_deref(), Some(VERSION));
        assert_eq!(install.choice().track, crate::releases::Track::Experimental);
        // A newer release waits, downloaded: the held version runs on.
        stage(&install, &pair, "9.1.0", &[("TPF3-MP.exe", b"newer")]);
        assert_eq!(
            start(&install, &keys(&pair), VERSION).unwrap(),
            Started::Continue
        );
        assert_eq!(fs::read(install.root.join("TPF3-MP.exe")).unwrap(), b"old");
        // Held on another version, not yet running: that one goes in.
        stage(&install, &pair, "0.0.9", &[("TPF3-MP.exe", b"older")]);
        install
            .choose(&Choice {
                hold: Some("0.0.9".into()),
                ..Choice::default()
            })
            .unwrap();
        assert_eq!(
            start(&install, &keys(&pair), VERSION).unwrap(),
            Started::Restart
        );
        assert_eq!(
            fs::read(install.root.join("TPF3-MP.exe")).unwrap(),
            b"older"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_chosen_version_must_be_signed_as_that_version() {
        let (dir, install) = installed("chosen-signed");
        let pair = key_pair();
        let top = "tpf3mp-0.0.9-windows-x64";
        let name = format!("{top}.zip");
        let archive = dir.join(&name);
        zip_of(&archive, &[(&format!("{top}/TPF3-MP.exe"), b"older")]);
        let bytes = fs::read(&archive).unwrap();
        let json = manifest_for("0.0.9", &name, &bytes);
        let signature = pair.sign(json.as_bytes()).as_ref().to_vec();
        let (base, stop) = serve(vec![
            (
                "/releases/download/v0.0.9/release.json".into(),
                json.clone().into_bytes(),
            ),
            (
                "/releases/download/v0.0.9/release.json.sig".into(),
                signature.clone(),
            ),
            (format!("/releases/download/v0.0.9/{name}"), bytes),
            // Another version's manifest, served as 0.0.8's.
            (
                "/releases/download/v0.0.8/release.json".into(),
                json.into_bytes(),
            ),
            (
                "/releases/download/v0.0.8/release.json.sig".into(),
                signature,
            ),
        ]);
        let source = Source {
            base: base.clone(),
            api: format!("{base}/api"),
            https_only: false,
        };
        let fetched = check_and_download(
            &install,
            &keys(&pair),
            &source,
            &Which::Version("0.0.9".into()),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(
            fetched,
            Checked::Downloaded("0.0.9".into()),
            "older, but chosen"
        );
        let swapped = check_and_download(
            &install,
            &keys(&pair),
            &source,
            &Which::Version("0.0.8".into()),
            |_, _, _| {},
        );
        assert!(
            matches!(swapped, Err(UpdateError::Malformed(_))),
            "{swapped:?}"
        );
        stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(base.trim_start_matches("http://"));
        fs::remove_dir_all(&dir).unwrap();
    }
}

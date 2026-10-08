//! Native mods on disk: the launcher's own folder for them, and the
//! registry of every file it put there.
//!
//! ```text
//! <launcher data>/native-mods/
//!   native-mods.json, native-mods.json.sig   the last index accepted
//!   registry.json                            every installed package and file
//!   enabled.json                             what the next game runs (crate::enabled)
//!   <id>/<version>/…                         a package's files
//!   <id>/.partial-<version>/                 a download in progress
//! ```
//!
//! Nothing goes into the game's folder. An install downloads every file
//! into the package's `.partial-` folder, checking each against the signed
//! index's size and SHA-256, and only when all of them check out renames
//! the folder into place and records it: a download cut short or tampered
//! with leaves the installed version as it was. An upgrade keeps the
//! version before it, for [`Store::rollback`], and deletes the one before
//! that. Uninstalling deletes exactly the files the registry names, then
//! the folders they leave empty; a file someone else put there stays.
//!
//! One launcher at a time uses the folder (the launcher's instance lock,
//! `tpf3mp-agent`'s `launcher::instance`).

#[cfg(feature = "install")]
use std::time::Duration;
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    enabled::{self, Enabled, EnabledPackage},
    features::{self, Effect},
    fetch::{self, FetchError},
    index::{self, FileEntry, Index, IndexError, Package, Setting},
    resolve::{self, ResolveError},
    signed,
};

/// The registry's file name.
pub const REGISTRY: &str = "registry.json";
const REGISTRY_FORMAT: u32 = 2;
const PREVIOUS_REGISTRY_FORMAT: u32 = 1;
const PARTIAL: &str = ".partial-";

/// Where packages come from.
pub trait Source {
    /// The index and its signature.
    fn index(&self) -> Result<(Vec<u8>, Vec<u8>), FetchError>;
    /// Downloads `file` into `path`, checked against its size and SHA-256
    /// on the way ([`fetch::copy_verified`]).
    fn download(&self, file: &FileEntry, path: &Path) -> Result<(), FetchError>;
}

/// The index and packages over HTTPS (the `install` feature): `<base>/native-mods.json` and its
/// signature, and each file at the address the signed index gives.
#[cfg(feature = "install")]
pub struct Http {
    base: String,
    https_only: bool,
    user_agent: String,
}

#[cfg(feature = "install")]
impl Http {
    pub fn new(base: impl Into<String>, user_agent: impl Into<String>) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_owned(),
            https_only: true,
            user_agent: user_agent.into(),
        }
    }

    /// The project's index on GitHub: assets of the `native-mods` release
    /// of `repository` (`owner/name`).
    pub fn github(repository: &str, user_agent: impl Into<String>) -> Self {
        Self::new(
            format!("https://github.com/{repository}/releases/download/native-mods"),
            user_agent,
        )
    }

    /// Allows plain HTTP, for tests on loopback.
    pub fn allow_http(mut self) -> Self {
        self.https_only = false;
        self
    }
}

#[cfg(feature = "install")]
impl Source for Http {
    fn index(&self) -> Result<(Vec<u8>, Vec<u8>), FetchError> {
        let agent = fetch::agent(self.https_only, Duration::from_secs(60));
        let json = fetch::get(
            &agent,
            &format!("{}/{}", self.base, index::INDEX_FILE),
            index::MAX_INDEX,
            &self.user_agent,
        )?;
        let signature = fetch::get(
            &agent,
            &format!("{}/{}", self.base, index::SIGNATURE_FILE),
            256,
            &self.user_agent,
        )?;
        Ok((json, signature))
    }

    fn download(&self, file: &FileEntry, path: &Path) -> Result<(), FetchError> {
        let agent = fetch::agent(self.https_only, Duration::from_secs(3 * 60 * 60));
        fetch::download(
            &agent,
            &file.url,
            path,
            file.size,
            &file.sha256,
            &self.user_agent,
            |_| {},
        )
    }
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error(transparent)]
    Index(#[from] IndexError),
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error(
        "the native-mods index offered ({offered}) is older than one already accepted ({seen})"
    )]
    OlderIndex { offered: u64, seen: u64 },
    #[error("the native-mods index reuses serial {serial} for different contents")]
    SerialConflict { serial: u64 },
    #[error("the accepted native-mods index at serial {serial} cannot be bound to its contents")]
    UnboundSerial { serial: u64 },
    #[error("no native-mods index has been accepted yet")]
    NoIndex,
    #[error("the native-mods registry cannot be read: {0}")]
    Registry(String),
    #[error("the native mod {0} is not installed")]
    NotInstalled(String),
    #[error("the native mod {0} has no earlier version to go back to")]
    NoPrevious(String),
    #[error("files of {id} {version} are missing or changed: {}", .files.join(", "))]
    Damaged {
        id: String,
        version: String,
        files: Vec<String>,
    },
    #[error("{0} holds files the registry does not name; move them away first")]
    Occupied(PathBuf),
    #[error("{by} needs {id}")]
    Needed { id: String, by: String },
    #[error("{0}")]
    Setting(String),
    #[error("native-mod path is unsafe or crosses a symbolic link: {0}")]
    UnsafePath(PathBuf),
    #[error("simulation-changing native mod {id} is not pinned to this game build ({build})")]
    SimulationPackageNotForBuild { id: String, build: String },
}

/// Every installed package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub format: u32,
    /// The highest index serial accepted.
    pub serial: u64,
    /// SHA-256 of the exact accepted index bytes, binding each serial to one
    /// immutable index even if the cache files are lost or a write is cut short.
    #[serde(default)]
    pub index_sha256: Option<String>,
    pub packages: BTreeMap<String, Installed>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            format: REGISTRY_FORMAT,
            serial: 0,
            index_sha256: None,
            packages: BTreeMap::new(),
        }
    }
}

/// One installed package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Installed {
    /// The version in use.
    pub current: String,
    /// The version before it, kept for a rollback.
    pub previous: Option<String>,
    /// Whether the player switched it on; installing does not.
    pub enabled: bool,
    /// The player's settings, over the package's defaults.
    pub settings: BTreeMap<String, Setting>,
    /// Each version on disk, as the signed index described it: its files
    /// are exactly these.
    pub versions: BTreeMap<String, Package>,
}

impl Installed {
    /// The package in use.
    pub fn package(&self) -> Option<&Package> {
        self.versions.get(&self.current)
    }
}

/// The native-mods folder and its registry.
pub struct Store {
    root: PathBuf,
    registry: Registry,
}

impl Store {
    /// The store in `root` (`<launcher data>/native-mods`). A registry that
    /// cannot be read is an error, never replaced by an empty one.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        ensure_no_symlink_path(&root, &root.join(REGISTRY))?;
        let registry = match fs::read(root.join(REGISTRY)) {
            Ok(bytes) => {
                let mut registry: Registry = serde_json::from_slice(&bytes)
                    .map_err(|error| StoreError::Registry(error.to_string()))?;
                if registry.format == PREVIOUS_REGISTRY_FORMAT {
                    registry.format = REGISTRY_FORMAT;
                } else if registry.format != REGISTRY_FORMAT {
                    return Err(StoreError::Registry(format!("format {}", registry.format)));
                }
                validate_registry(&registry)?;
                registry
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Registry::default(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self { root, registry })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The folder of `id` in `version`.
    pub fn folder(&self, id: &str, version: &str) -> PathBuf {
        self.root.join(id).join(version)
    }

    /// Fetches the index from `source` and accepts it ([`Self::accept_index`]).
    pub fn refresh(&mut self, source: &dyn Source, keys: &[Vec<u8>]) -> Result<Index, StoreError> {
        let (json, signature) = source.index()?;
        self.accept_index(&json, &signature, keys)
    }

    /// Accepts `json` as the index if `signature` is one of `keys`' and its
    /// serial is not older than one accepted before, and keeps it.
    pub fn accept_index(
        &mut self,
        json: &[u8],
        signature: &[u8],
        keys: &[Vec<u8>],
    ) -> Result<Index, StoreError> {
        let index = Index::verified(json, signature, keys)?;
        let digest = signed::sha256_hex(json);

        // A cache pair can be one rename ahead of the registry after power
        // loss. A valid higher serial is accepted history too: persist its
        // floor before considering the newly fetched bytes.
        let cached = match self.verified_cached_index(keys) {
            Ok(cached) => cached,
            Err(StoreError::Index(_) | StoreError::NoIndex) => None,
            Err(error) => return Err(error),
        };
        if let Some((cached, cached_json)) = cached {
            let cached_digest = signed::sha256_hex(&cached_json);
            if cached.serial > self.registry.serial {
                self.registry.serial = cached.serial;
                self.registry.index_sha256 = Some(cached_digest);
                self.save()?;
            } else if cached.serial == self.registry.serial {
                match &self.registry.index_sha256 {
                    Some(accepted)
                        if accepted != &cached_digest && index.serial <= self.registry.serial =>
                    {
                        return Err(StoreError::SerialConflict {
                            serial: index.serial,
                        });
                    }
                    None => {
                        self.registry.index_sha256 = Some(cached_digest);
                        self.save()?;
                    }
                    _ => {}
                }
            }
        }

        if index.serial < self.registry.serial {
            return Err(StoreError::OlderIndex {
                offered: index.serial,
                seen: self.registry.serial,
            });
        }
        if index.serial == self.registry.serial {
            match &self.registry.index_sha256 {
                Some(accepted) if accepted == &digest => {
                    // Rewriting the same accepted bytes repairs a missing or
                    // mixed cache pair without changing the serial binding.
                    write_atomic(&self.root, &self.root.join(index::INDEX_FILE), json)?;
                    write_atomic(
                        &self.root,
                        &self.root.join(index::SIGNATURE_FILE),
                        signature,
                    )?;
                    return Ok(index);
                }
                Some(_) => {
                    return Err(StoreError::SerialConflict {
                        serial: index.serial,
                    });
                }
                None if self.registry.serial == 0 => {}
                None => {
                    return Err(StoreError::UnboundSerial {
                        serial: index.serial,
                    });
                }
            }
        }
        ensure_no_symlink_path(&self.root, &self.root)?;
        fs::create_dir_all(&self.root)?;
        write_atomic(&self.root, &self.root.join(index::INDEX_FILE), json)?;
        write_atomic(
            &self.root,
            &self.root.join(index::SIGNATURE_FILE),
            signature,
        )?;
        self.registry.serial = index.serial;
        self.registry.index_sha256 = Some(digest);
        self.save()?;
        Ok(index)
    }

    /// The last accepted index, verified again. A complete cache written
    /// before a crash can advance and repair the registry's serial floor.
    pub fn cached_index(&mut self, keys: &[Vec<u8>]) -> Result<Index, StoreError> {
        let (index, json) = self
            .verified_cached_index(keys)?
            .ok_or(StoreError::NoIndex)?;
        if index.serial < self.registry.serial {
            return Err(StoreError::OlderIndex {
                offered: index.serial,
                seen: self.registry.serial,
            });
        }
        let digest = signed::sha256_hex(&json);
        if index.serial == self.registry.serial
            && self
                .registry
                .index_sha256
                .as_ref()
                .is_some_and(|accepted| accepted != &digest)
        {
            return Err(StoreError::SerialConflict {
                serial: index.serial,
            });
        }
        if index.serial > self.registry.serial || self.registry.index_sha256.is_none() {
            self.registry.serial = index.serial;
            self.registry.index_sha256 = Some(digest);
            self.save()?;
        }
        Ok(index)
    }

    fn verified_cached_index(
        &self,
        keys: &[Vec<u8>],
    ) -> Result<Option<(Index, Vec<u8>)>, StoreError> {
        let json_path = self.root.join(index::INDEX_FILE);
        let signature_path = self.root.join(index::SIGNATURE_FILE);
        ensure_no_symlink_path(&self.root, &json_path)?;
        ensure_no_symlink_path(&self.root, &signature_path)?;
        let json = read_optional(&json_path)?;
        let signature = read_optional(&signature_path)?;
        match (json, signature) {
            (Some(json), Some(signature)) => {
                let index = Index::verified(&json, &signature, keys)?;
                Ok(Some((index, json)))
            }
            (None, None) => Ok(None),
            _ => Err(StoreError::NoIndex),
        }
    }

    /// Installs `id` (the newest version for the build) and what it needs.
    /// Upgrades it when it is installed. Returns what was installed.
    pub fn install(
        &mut self,
        context: resolve::Context<'_>,
        id: &str,
        source: &dyn Source,
    ) -> Result<Vec<String>, StoreError> {
        let installed: Vec<&Package> = self
            .registry
            .packages
            .values()
            .filter_map(Installed::package)
            .collect();
        let order: Vec<Package> = resolve::resolve(context, &[id], &installed)?
            .into_iter()
            .cloned()
            .collect();
        let mut done = Vec::new();
        for package in &order {
            self.install_one(package, source)?;
            done.push(format!("{} {}", package.id, package.version));
        }
        Ok(done)
    }

    fn install_one(&mut self, package: &Package, source: &dyn Source) -> Result<(), StoreError> {
        let folder = self.folder(&package.id, &package.version);
        check_package_paths(&self.root, &folder, package)?;
        let registered = self
            .registry
            .packages
            .get(&package.id)
            .is_some_and(|i| i.versions.contains_key(&package.version));
        if registered {
            // The version kept for a rollback: in use again if it is whole,
            // downloaded again if it is not.
            if self.damaged(&folder, package)?.is_empty() {
                return self.make_current(package);
            }
            remove_registered(&self.root, &folder, package)?;
            if let Some(installed) = self.registry.packages.get_mut(&package.id) {
                installed.versions.remove(&package.version);
            }
        } else if folder.exists() {
            // Left by an install cut short after the rename: taken if it is
            // exactly the package, refused otherwise.
            if !self.damaged(&folder, package)?.is_empty() {
                return Err(StoreError::Occupied(folder));
            }
            return self.make_current(package);
        }
        let partial = self
            .root
            .join(&package.id)
            .join(format!("{PARTIAL}{}", package.version));
        ensure_no_symlink_path(&self.root, &partial)?;
        if partial.exists() {
            ensure_no_symlink_tree(&self.root, &partial)?;
            fs::remove_dir_all(&partial)?;
        }
        fs::create_dir_all(&partial)?;
        let downloaded = package.files.iter().try_for_each(|file| {
            let path = partial.join(&file.path);
            ensure_no_symlink_path(&self.root, &path)?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            ensure_no_symlink_path(&self.root, &path)?;
            source.download(file, &path).map_err(StoreError::from)
        });
        if let Err(error) = downloaded {
            let _ = remove_partial_tree(&self.root, &partial);
            return Err(error);
        }
        ensure_no_symlink_tree(&self.root, &partial)?;
        ensure_no_symlink_path(&self.root, &folder)?;
        if let Err(error) = fs::rename(&partial, &folder) {
            let _ = remove_partial_tree(&self.root, &partial);
            return Err(error.into());
        }
        self.make_current(package)
    }

    /// Records `package`, on disk, as its id's version in use, keeping the
    /// one before and deleting older ones.
    fn make_current(&mut self, package: &Package) -> Result<(), StoreError> {
        let prior_current = self
            .registry
            .packages
            .get(&package.id)
            .map(|installed| installed.current.clone());
        if let Some(installed) = self.registry.packages.get(&package.id) {
            for old in installed.versions.values().filter(|old| {
                old.version != package.version
                    && prior_current.as_deref() != Some(old.version.as_str())
            }) {
                check_package_paths(&self.root, &self.folder(&old.id, &old.version), old)?;
            }
        }
        let entry = self
            .registry
            .packages
            .entry(package.id.clone())
            .or_insert_with(|| Installed {
                current: package.version.clone(),
                previous: None,
                enabled: false,
                settings: BTreeMap::new(),
                versions: BTreeMap::new(),
            });
        entry
            .versions
            .insert(package.version.clone(), package.clone());
        if entry.current != package.version {
            entry.previous = Some(std::mem::replace(
                &mut entry.current,
                package.version.clone(),
            ));
        }
        // The player's settings that the new version still has, with the
        // same type.
        entry.settings.retain(|key, value| {
            package
                .settings
                .get(key)
                .is_some_and(|default| default.same_type(value))
        });
        let keep = [Some(entry.current.clone()), entry.previous.clone()];
        let old: Vec<Package> = entry
            .versions
            .values()
            .filter(|p| !keep.contains(&Some(p.version.clone())))
            .cloned()
            .collect();
        for package in &old {
            entry.versions.remove(&package.version);
        }
        self.save()?;
        for package in old {
            remove_registered(
                &self.root,
                &self.folder(&package.id, &package.version),
                &package,
            )?;
        }
        Ok(())
    }

    /// Deletes every file of `id` the registry names, and the folders that
    /// leaves empty. Refused while another installed package needs it.
    pub fn uninstall(&mut self, id: &str) -> Result<(), StoreError> {
        let installed = self
            .registry
            .packages
            .get(id)
            .ok_or_else(|| StoreError::NotInstalled(id.to_owned()))?;
        if let Some(by) = self
            .registry
            .packages
            .values()
            .filter_map(Installed::package)
            .find(|p| p.id != id && p.depends.iter().any(|d| d.id == id))
        {
            return Err(StoreError::Needed {
                id: id.to_owned(),
                by: by.id.clone(),
            });
        }
        let versions: Vec<Package> = installed.versions.values().cloned().collect();
        for package in &versions {
            check_package_paths(&self.root, &self.folder(id, &package.version), package)?;
        }
        let package_folder = self.root.join(id);
        ensure_no_symlink_path(&self.root, &package_folder)?;
        let partials = match fs::read_dir(&package_folder) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with(PARTIAL))
                })
                .collect::<Vec<_>>(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        for partial in &partials {
            ensure_no_symlink_tree(&self.root, partial)?;
        }
        self.registry.packages.remove(id);
        self.save()?;
        for package in &versions {
            remove_registered(&self.root, &self.folder(id, &package.version), package)?;
        }
        for partial in partials {
            let _ = remove_partial_tree(&self.root, &partial);
        }
        let _ = fs::remove_dir(&package_folder);
        Ok(())
    }

    /// Goes back to the version in use before the last upgrade, if its
    /// files are still whole.
    pub fn rollback(&mut self, id: &str) -> Result<(), StoreError> {
        let installed = self
            .registry
            .packages
            .get(id)
            .ok_or_else(|| StoreError::NotInstalled(id.to_owned()))?;
        let previous = installed
            .previous
            .as_ref()
            .and_then(|v| installed.versions.get(v))
            .ok_or_else(|| StoreError::NoPrevious(id.to_owned()))?
            .clone();
        let damaged = self.damaged(&self.folder(id, &previous.version), &previous)?;
        if !damaged.is_empty() {
            return Err(StoreError::Damaged {
                id: id.to_owned(),
                version: previous.version,
                files: damaged,
            });
        }
        self.make_current(&previous)
    }

    /// Checks the files of the version of `id` in use.
    pub fn verify(&self, id: &str) -> Result<(), StoreError> {
        let package = self
            .registry
            .packages
            .get(id)
            .and_then(Installed::package)
            .ok_or_else(|| StoreError::NotInstalled(id.to_owned()))?;
        let damaged = self.damaged(&self.folder(id, &package.version), package)?;
        if damaged.is_empty() {
            Ok(())
        } else {
            Err(StoreError::Damaged {
                id: id.to_owned(),
                version: package.version.clone(),
                files: damaged,
            })
        }
    }

    /// Switches `id` on or off for the next game.
    pub fn set_enabled(&mut self, id: &str, on: bool) -> Result<(), StoreError> {
        self.registry
            .packages
            .get_mut(id)
            .ok_or_else(|| StoreError::NotInstalled(id.to_owned()))?
            .enabled = on;
        self.save()
    }

    /// Sets one of `id`'s settings: one its package has, with the same type.
    pub fn set_setting(&mut self, id: &str, key: &str, value: Setting) -> Result<(), StoreError> {
        let installed = self
            .registry
            .packages
            .get_mut(id)
            .ok_or_else(|| StoreError::NotInstalled(id.to_owned()))?;
        let package = installed
            .versions
            .get(&installed.current)
            .ok_or_else(|| StoreError::NotInstalled(id.to_owned()))?;
        match package.settings.get(key) {
            Some(default) if default.same_type(&value) => {
                installed.settings.insert(key.to_owned(), value);
                self.save()
            }
            Some(_) => Err(StoreError::Setting(format!(
                "{id}'s setting {key} has another type"
            ))),
            None => Err(StoreError::Setting(format!("{id} has no setting {key}"))),
        }
    }

    /// The enabled packages for the game build hashing to `build`, with
    /// their settings. An enabled package not for this build is left out,
    /// with its id in the second list.
    pub fn enabled_for(&self, build: &str) -> Result<(Enabled, Vec<String>), StoreError> {
        let mut packages = Vec::new();
        let mut left_out = Vec::new();
        for (id, installed) in &self.registry.packages {
            let Some(package) = installed.package().filter(|_| installed.enabled) else {
                continue;
            };
            if !package.runs_on(build) {
                if package.simulation
                    || package.features.iter().any(|id| {
                        features::find(features::BUILT_IN, id)
                            .is_none_or(|feature| feature.effect == Effect::Simulation)
                    })
                {
                    return Err(StoreError::SimulationPackageNotForBuild {
                        id: id.clone(),
                        build: build.to_owned(),
                    });
                }
                left_out.push(id.clone());
                continue;
            }
            check_package_paths(&self.root, &self.folder(id, &package.version), package)?;
            let mut settings = package.settings.clone();
            settings.extend(installed.settings.clone());
            packages.push(EnabledPackage {
                id: id.clone(),
                version: package.version.clone(),
                simulation: package.simulation,
                features: package.features.clone(),
                settings,
                root: self.folder(id, &package.version),
            });
        }
        Ok((
            Enabled {
                format: enabled::FORMAT,
                build: build.to_owned(),
                packages,
            },
            left_out,
        ))
    }

    /// Writes what the next game on `build` runs to `enabled.json`, for
    /// the launcher to name in the game's environment ([`enabled::ENV`]).
    pub fn write_enabled(&self, build: &str) -> Result<(PathBuf, Vec<String>), StoreError> {
        let (enabled, left_out) = self.enabled_for(build)?;
        ensure_no_symlink_path(&self.root, &self.root)?;
        fs::create_dir_all(&self.root)?;
        let path = self.root.join(enabled::FILE);
        let json =
            serde_json::to_vec_pretty(&enabled).map_err(|e| StoreError::Registry(e.to_string()))?;
        write_atomic(&self.root, &path, &json)?;
        Ok((path, left_out))
    }

    /// The files of `package` in `folder` that are missing or changed.
    fn damaged(&self, folder: &Path, package: &Package) -> Result<Vec<String>, StoreError> {
        check_package_paths(&self.root, folder, package)?;
        let mut damaged = Vec::new();
        for file in &package.files {
            if !fetch::file_matches(&folder.join(&file.path), file.size, &file.sha256)? {
                damaged.push(file.path.clone());
            }
        }
        Ok(damaged)
    }

    fn save(&self) -> Result<(), StoreError> {
        ensure_no_symlink_path(&self.root, &self.root)?;
        fs::create_dir_all(&self.root)?;
        let json = serde_json::to_vec_pretty(&self.registry)
            .map_err(|error| StoreError::Registry(error.to_string()))?;
        write_atomic(&self.root, &self.root.join(REGISTRY), &json)?;
        Ok(())
    }
}

/// Deletes the files `package` names in `folder`, then the folders that
/// leaves empty, `folder` last. Nothing else.
fn remove_registered(root: &Path, folder: &Path, package: &Package) -> Result<(), StoreError> {
    check_package_paths(root, folder, package)?;
    let mut dirs = Vec::new();
    for file in &package.files {
        let path = folder.join(&file.path);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut parent = path.parent();
        while let Some(dir) = parent.filter(|dir| dir.starts_with(folder) && *dir != folder) {
            dirs.push(dir.to_owned());
            parent = dir.parent();
        }
    }
    // Deepest first; a folder that is not empty stays.
    dirs.sort_by_key(|dir| std::cmp::Reverse(dir.components().count()));
    dirs.dedup();
    for dir in dirs {
        let _ = fs::remove_dir(dir);
    }
    let _ = fs::remove_dir(folder);
    Ok(())
}

/// Writes `bytes` to `path` through a file beside it, so that a reader
/// sees the old contents or the new, never part of them.
fn write_atomic(root: &Path, path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    ensure_no_symlink_path(root, path)?;
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".new");
    let temporary = PathBuf::from(temporary);
    ensure_no_symlink_path(root, &temporary)?;
    {
        use io::Write;
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&temporary, path)?;
    Ok(())
}

fn validate_registry(registry: &Registry) -> Result<(), StoreError> {
    if registry
        .index_sha256
        .as_ref()
        .is_some_and(|digest| !signed::is_sha256(digest))
    {
        return Err(StoreError::Registry("invalid accepted-index digest".into()));
    }
    for (id, installed) in &registry.packages {
        if !index::is_id(id) {
            return Err(StoreError::Registry(format!("invalid package id {id:?}")));
        }
        let Some(current) = installed.versions.get(&installed.current) else {
            return Err(StoreError::Registry(format!(
                "{id} has no registered current version {}",
                installed.current
            )));
        };
        if let Some(previous) = &installed.previous
            && (previous == &installed.current || !installed.versions.contains_key(previous))
        {
            return Err(StoreError::Registry(format!(
                "{id} has an invalid previous version {previous}"
            )));
        }
        for (version, package) in &installed.versions {
            package
                .validate()
                .map_err(|problem| StoreError::Registry(format!("{id} {version}: {problem}")))?;
            if package.id != *id || package.version != *version {
                return Err(StoreError::Registry(format!(
                    "{id} {version} does not match its package identity"
                )));
            }
        }
        for (key, value) in &installed.settings {
            match current.settings.get(key) {
                Some(default) if default.same_type(value) => {}
                _ => {
                    return Err(StoreError::Registry(format!(
                        "{id} has an invalid setting {key:?}"
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Rejects a symlink in an internal path before a file operation. `root` is
/// the store boundary; every remaining component must be a normal relative
/// name and every existing component must be an ordinary filesystem entry.
fn ensure_no_symlink_path(root: &Path, path: &Path) -> Result<(), StoreError> {
    let absolute = |path: &Path| -> Result<PathBuf, StoreError> {
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        if path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        }) {
            return Err(StoreError::UnsafePath(path));
        }
        Ok(path)
    };
    let root = absolute(root)?;
    let path = absolute(path)?;
    let relative = path
        .strip_prefix(&root)
        .map_err(|_| StoreError::UnsafePath(path.clone()))?;
    if relative
        .components()
        .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(StoreError::UnsafePath(path));
    }

    // Inspect every existing ancestor, including ancestors of the store
    // root. A symlink above the root would redirect otherwise-safe package
    // paths to an external directory.
    let mut ancestors: Vec<_> = path.ancestors().collect();
    ancestors.reverse();
    for ancestor in ancestors {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(StoreError::UnsafePath(ancestor.to_owned()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn check_package_paths(root: &Path, folder: &Path, package: &Package) -> Result<(), StoreError> {
    ensure_no_symlink_path(root, folder)?;
    for file in &package.files {
        if !index::is_safe_path(&file.path) {
            return Err(StoreError::UnsafePath(folder.join(&file.path)));
        }
        ensure_no_symlink_path(root, &folder.join(&file.path))?;
    }
    Ok(())
}

fn ensure_no_symlink_tree(root: &Path, folder: &Path) -> Result<(), StoreError> {
    ensure_no_symlink_path(root, folder)?;
    let metadata = match fs::symlink_metadata(folder) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(folder)? {
        ensure_no_symlink_tree(root, &entry?.path())?;
    }
    Ok(())
}

fn remove_partial_tree(root: &Path, folder: &Path) -> Result<(), StoreError> {
    ensure_no_symlink_tree(root, folder)?;
    match fs::remove_dir_all(folder) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::HashMap};

    use super::*;
    use crate::{
        enabled,
        features::example::BIG_MAPS,
        index::{
            Dependency,
            tests::{BUILD, OTHER_BUILD, big_maps, index, keys, package, signed, test_key, url_of},
        },
        resolve::Context,
    };

    /// Serves files from memory, as a server would: bytes by address, some
    /// cut short.
    #[derive(Default)]
    struct Served {
        files: RefCell<HashMap<String, Vec<u8>>>,
        index: RefCell<Option<(Vec<u8>, Vec<u8>)>>,
    }

    impl Served {
        fn serve(&self, id: &str, version: &str, files: &[(&str, &[u8])]) {
            for (path, bytes) in files {
                self.files
                    .borrow_mut()
                    .insert(url_of(id, version, path), bytes.to_vec());
            }
        }
    }

    impl Source for Served {
        fn index(&self) -> Result<(Vec<u8>, Vec<u8>), FetchError> {
            self.index
                .borrow()
                .clone()
                .ok_or_else(|| FetchError::Http("404".into()))
        }

        fn download(&self, file: &FileEntry, path: &Path) -> Result<(), FetchError> {
            let files = self.files.borrow();
            let bytes = files
                .get(&file.url)
                .ok_or_else(|| FetchError::Http("404".into()))?;
            fetch::copy_verified(&bytes[..], path, file.size, &file.sha256, |_| {})
        }
    }

    const V1: &[(&str, &[u8])] = &[
        ("mod/tpf3mp_bigmap_1/mod.lua", b"-- version 1"),
        ("data/sizes.toml", b"sizes = [128]"),
    ];
    const V2: &[(&str, &[u8])] = &[
        ("mod/tpf3mp_bigmap_1/mod.lua", b"-- version 2"),
        ("data/sizes.toml", b"sizes = [128, 256]"),
        ("data/new.toml", b"new = true"),
    ];

    fn context(index: &Index) -> Context<'_> {
        Context {
            index,
            build: BUILD,
            registry: BIG_MAPS,
        }
    }

    /// Every file under `dir`, relative, sorted.
    fn tree(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_owned()];
        while let Some(at) = stack.pop() {
            let Ok(entries) = fs::read_dir(&at) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path.clone());
                }
                out.push(
                    path.strip_prefix(dir)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
        out.sort();
        out
    }

    #[test]
    fn an_install_puts_verified_files_in_the_launchers_folder_and_registers_them() {
        let dir = tempfile::tempdir().unwrap();
        let served = Served::default();
        served.serve("bigmap", "1.0.0", V1);
        let index = index(vec![package("bigmap", "1.0.0", V1)]);
        let mut store = Store::open(dir.path().join("native-mods")).unwrap();
        let done = store.install(context(&index), "bigmap", &served).unwrap();
        assert_eq!(done, ["bigmap 1.0.0"]);
        let folder = store.folder("bigmap", "1.0.0");
        assert_eq!(
            fs::read(folder.join("mod/tpf3mp_bigmap_1/mod.lua")).unwrap(),
            b"-- version 1"
        );
        store.verify("bigmap").unwrap();
        // The registry survives a restart; installing is not enabling.
        let again = Store::open(dir.path().join("native-mods")).unwrap();
        let installed = &again.registry().packages["bigmap"];
        assert_eq!(installed.current, "1.0.0");
        assert!(!installed.enabled);
        assert_eq!(installed.package().unwrap().files.len(), 2);
    }

    #[test]
    fn a_wrong_hash_or_a_partial_download_installs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("native-mods");
        let index = index(vec![package("bigmap", "1.0.0", V1)]);
        for body in [&b"-- version X"[..], &b"-- ver"[..]] {
            let served = Served::default();
            served.serve("bigmap", "1.0.0", V1);
            served
                .files
                .borrow_mut()
                .insert(url_of("bigmap", "1.0.0", "data/sizes.toml"), body.to_vec());
            let mut store = Store::open(&root).unwrap();
            assert!(matches!(
                store.install(context(&index), "bigmap", &served),
                Err(StoreError::Fetch(FetchError::Mismatch))
            ));
            assert!(store.registry().packages.is_empty());
            // No package folder, no partial download left.
            assert!(tree(&root.join("bigmap")).is_empty(), "{:?}", tree(&root));
        }
    }

    #[test]
    fn a_package_not_pinned_to_this_build_is_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let served = Served::default();
        served.serve("bigmap", "1.0.0", V1);
        let mut other = package("bigmap", "1.0.0", V1);
        other.builds = vec![OTHER_BUILD.into()];
        let index = index(vec![other]);
        let mut store = Store::open(dir.path()).unwrap();
        assert!(matches!(
            store.install(context(&index), "bigmap", &served),
            Err(StoreError::Resolve(ResolveError::NotForBuild(_)))
        ));
        assert!(tree(dir.path()).is_empty());
    }

    #[test]
    fn only_a_signed_index_not_older_than_the_last_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let pair = test_key();
        let mut store = Store::open(dir.path()).unwrap();
        let mut newer = index(vec![package("bigmap", "1.0.0", V1)]);
        newer.serial = 5;
        let (json, signature) = signed(&newer, &pair);
        let served = Served::default();
        *served.index.borrow_mut() = Some((json.clone(), signature.clone()));
        assert_eq!(store.refresh(&served, &keys(&pair)).unwrap(), newer);
        assert_eq!(store.cached_index(&keys(&pair)).unwrap(), newer);
        // Unsigned, tampered, another key.
        assert!(matches!(
            store.accept_index(&json, b"", &keys(&pair)),
            Err(StoreError::Index(IndexError::BadSignature))
        ));
        assert!(matches!(
            store.accept_index(&json, &signature, &keys(&test_key())),
            Err(StoreError::Index(IndexError::BadSignature))
        ));
        assert!(matches!(
            store.cached_index(&keys(&test_key())),
            Err(StoreError::Index(IndexError::BadSignature))
        ));
        // An older index, signed, is a replay.
        let mut older = newer.clone();
        older.serial = 4;
        let (json, signature) = signed(&older, &pair);
        assert!(matches!(
            store.accept_index(&json, &signature, &keys(&pair)),
            Err(StoreError::OlderIndex {
                offered: 4,
                seen: 5
            })
        ));
        assert_eq!(store.cached_index(&keys(&pair)).unwrap().serial, 5);
    }

    #[test]
    fn a_different_index_cannot_reuse_an_accepted_serial() {
        let dir = tempfile::tempdir().unwrap();
        let pair = test_key();
        let mut store = Store::open(dir.path()).unwrap();
        let mut first = index(vec![package("bigmap", "1.0.0", V1)]);
        first.serial = 5;
        let (json, signature) = signed(&first, &pair);
        store.accept_index(&json, &signature, &keys(&pair)).unwrap();

        let mut changed = first;
        changed.packages[0].description = "changed without a serial bump".into();
        let (json, signature) = signed(&changed, &pair);
        assert!(store.accept_index(&json, &signature, &keys(&pair)).is_err());
        assert_eq!(
            store.cached_index(&keys(&pair)).unwrap().packages[0].description,
            ""
        );
    }

    #[test]
    fn a_cached_index_written_before_a_crash_recovers_the_serial_floor() {
        let dir = tempfile::tempdir().unwrap();
        let pair = test_key();
        let mut store = Store::open(dir.path()).unwrap();
        let mut first = index(vec![package("bigmap", "1.0.0", V1)]);
        first.serial = 5;
        let (old_json, old_signature) = signed(&first, &pair);
        store
            .accept_index(&old_json, &old_signature, &keys(&pair))
            .unwrap();

        // Simulate power loss after the new index/signature were renamed but
        // before the registry's serial floor was saved.
        let mut newer = index(vec![package("bigmap", "2.0.0", V2)]);
        newer.serial = 6;
        let (new_json, new_signature) = signed(&newer, &pair);
        fs::write(dir.path().join(index::INDEX_FILE), new_json).unwrap();
        fs::write(dir.path().join(index::SIGNATURE_FILE), new_signature).unwrap();
        drop(store);

        let mut recovered = Store::open(dir.path()).unwrap();
        assert_eq!(recovered.cached_index(&keys(&pair)).unwrap().serial, 6);
        assert_eq!(recovered.registry().serial, 6);
        assert!(matches!(
            recovered.accept_index(&old_json, &old_signature, &keys(&pair)),
            Err(StoreError::OlderIndex {
                offered: 5,
                seen: 6
            })
        ));
    }

    #[test]
    fn uninstalling_leaves_nothing_of_the_package() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("native-mods");
        let served = Served::default();
        served.serve("bigmap", "1.0.0", V1);
        served.serve("bigmap", "2.0.0", V2);
        let mut store = Store::open(&root).unwrap();
        let first = index(vec![package("bigmap", "1.0.0", V1)]);
        store.install(context(&first), "bigmap", &served).unwrap();
        let second = index(vec![
            package("bigmap", "1.0.0", V1),
            package("bigmap", "2.0.0", V2),
        ]);
        store.install(context(&second), "bigmap", &served).unwrap();
        store.set_enabled("bigmap", true).unwrap();
        store.write_enabled(BUILD).unwrap();
        // A file someone else put there.
        fs::write(root.join("bigmap/2.0.0/data/mine.txt"), b"mine").unwrap();
        store.uninstall("bigmap").unwrap();
        assert!(store.registry().packages.is_empty());
        assert_eq!(
            tree(&root.join("bigmap")),
            ["2.0.0", "2.0.0/data", "2.0.0/data/mine.txt"]
        );
        fs::remove_dir_all(root.join("bigmap")).unwrap();

        // Without it, nothing of the package is left at all.
        store.install(context(&second), "bigmap", &served).unwrap();
        store.uninstall("bigmap").unwrap();
        assert!(!root.join("bigmap").exists(), "{:?}", tree(&root));
        assert!(matches!(
            store.uninstall("bigmap"),
            Err(StoreError::NotInstalled(_))
        ));
    }

    #[test]
    fn an_upgrade_keeps_the_old_version_until_the_new_one_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let served = Served::default();
        served.serve("bigmap", "1.0.0", V1);
        let mut store = Store::open(dir.path()).unwrap();
        let first = index(vec![package("bigmap", "1.0.0", V1)]);
        store.install(context(&first), "bigmap", &served).unwrap();
        store
            .set_setting("bigmap", "octree_depth", Setting::Int(9))
            .unwrap_err();

        // Version 2's server hands out a damaged file.
        let second = index(vec![
            package("bigmap", "1.0.0", V1),
            package("bigmap", "2.0.0", V2),
        ]);
        served.serve("bigmap", "2.0.0", V2);
        served.files.borrow_mut().insert(
            url_of("bigmap", "2.0.0", "data/new.toml"),
            b"new = 1!!!".to_vec(),
        );
        assert!(store.install(context(&second), "bigmap", &served).is_err());
        assert_eq!(store.registry().packages["bigmap"].current, "1.0.0");
        store.verify("bigmap").unwrap();
        assert!(!store.folder("bigmap", "2.0.0").exists());

        // Fixed: 2 in use, 1 kept for a rollback.
        served.serve("bigmap", "2.0.0", V2);
        store.install(context(&second), "bigmap", &served).unwrap();
        let installed = &store.registry().packages["bigmap"];
        assert_eq!(
            (installed.current.as_str(), installed.previous.as_deref()),
            ("2.0.0", Some("1.0.0"))
        );
        assert!(store.folder("bigmap", "1.0.0").exists());

        store.rollback("bigmap").unwrap();
        let installed = &store.registry().packages["bigmap"];
        assert_eq!(
            (installed.current.as_str(), installed.previous.as_deref()),
            ("1.0.0", Some("2.0.0"))
        );
        store.verify("bigmap").unwrap();

        // A third version: the one before the previous goes.
        const V3: &[(&str, &[u8])] = &[("mod/tpf3mp_bigmap_1/mod.lua", b"-- version 3")];
        served.serve("bigmap", "3.0.0", V3);
        let third = index(vec![
            package("bigmap", "1.0.0", V1),
            package("bigmap", "2.0.0", V2),
            package("bigmap", "3.0.0", V3),
        ]);
        store.install(context(&third), "bigmap", &served).unwrap();
        let installed = &store.registry().packages["bigmap"];
        assert_eq!(
            (installed.current.as_str(), installed.previous.as_deref()),
            ("3.0.0", Some("1.0.0"))
        );
        assert!(!store.folder("bigmap", "2.0.0").exists());
    }

    #[test]
    fn a_rollback_to_damaged_files_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let served = Served::default();
        served.serve("bigmap", "1.0.0", V1);
        served.serve("bigmap", "2.0.0", V2);
        let mut store = Store::open(dir.path()).unwrap();
        let both = index(vec![
            package("bigmap", "1.0.0", V1),
            package("bigmap", "2.0.0", V2),
        ]);
        let first = index(vec![package("bigmap", "1.0.0", V1)]);
        assert!(matches!(
            store.rollback("bigmap"),
            Err(StoreError::NotInstalled(_))
        ));
        store.install(context(&first), "bigmap", &served).unwrap();
        assert!(matches!(
            store.rollback("bigmap"),
            Err(StoreError::NoPrevious(_))
        ));
        store.install(context(&both), "bigmap", &served).unwrap();
        fs::write(
            store.folder("bigmap", "1.0.0").join("data/sizes.toml"),
            b"x",
        )
        .unwrap();
        assert!(matches!(
            store.rollback("bigmap"),
            Err(StoreError::Damaged { .. })
        ));
        assert_eq!(store.registry().packages["bigmap"].current, "2.0.0");
    }

    #[test]
    fn a_needed_package_stays_and_dependencies_install_first() {
        let dir = tempfile::tempdir().unwrap();
        let served = Served::default();
        served.serve("base", "1.0.0", V1);
        served.serve("addon", "1.0.0", &[("addon.lua", b"--")]);
        let mut addon = package("addon", "1.0.0", &[("addon.lua", b"--")]);
        addon.depends.push(Dependency {
            id: "base".into(),
            version: "^1".into(),
        });
        let index = index(vec![package("base", "1.0.0", V1), addon]);
        let mut store = Store::open(dir.path()).unwrap();
        let done = store.install(context(&index), "addon", &served).unwrap();
        assert_eq!(done, ["base 1.0.0", "addon 1.0.0"]);
        assert!(matches!(
            store.uninstall("base"),
            Err(StoreError::Needed { .. })
        ));
        store.uninstall("addon").unwrap();
        store.uninstall("base").unwrap();
        assert!(tree(dir.path()).iter().all(|f| f.starts_with(REGISTRY)));
    }

    #[test]
    fn a_folder_left_by_a_cut_install_is_taken_only_if_it_is_the_package() {
        let dir = tempfile::tempdir().unwrap();
        let served = Served::default();
        let index = index(vec![package("bigmap", "1.0.0", V1)]);
        let mut store = Store::open(dir.path()).unwrap();
        let folder = store.folder("bigmap", "1.0.0");
        for (path, bytes) in V1 {
            let file = folder.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, bytes).unwrap();
        }
        // Nothing is served: the folder is taken as it is.
        store.install(context(&index), "bigmap", &served).unwrap();
        assert_eq!(store.registry().packages["bigmap"].current, "1.0.0");

        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let folder = store.folder("bigmap", "1.0.0");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("other.txt"), b"?").unwrap();
        assert!(matches!(
            store.install(context(&index), "bigmap", &served),
            Err(StoreError::Occupied(_))
        ));
        assert!(folder.join("other.txt").exists());
    }

    #[test]
    fn the_hook_gets_the_enabled_packages_for_its_build_with_settings() {
        let dir = tempfile::tempdir().unwrap();
        let served = Served::default();
        let files: &[(&str, &[u8])] = &[("mod/tpf3mp_bigmap_1/mod.lua", b"-- ")];
        served.serve("bigmap", "0.3.0", files);
        let index = index(vec![big_maps()]);
        let mut store = Store::open(dir.path()).unwrap();
        store.install(context(&index), "bigmap", &served).unwrap();

        let (path, _) = store.write_enabled(BUILD).unwrap();
        assert!(
            enabled::read(&path).unwrap().packages.is_empty(),
            "not enabled yet"
        );

        store.set_enabled("bigmap", true).unwrap();
        store
            .set_setting("bigmap", "octree_depth", Setting::Int(11))
            .unwrap();
        assert!(matches!(
            store.set_setting("bigmap", "octree_depth", Setting::Bool(true)),
            Err(StoreError::Setting(_))
        ));
        assert!(matches!(
            store.set_setting("bigmap", "nonsense", Setting::Int(1)),
            Err(StoreError::Setting(_))
        ));
        let (path, left_out) = store.write_enabled(BUILD).unwrap();
        assert!(left_out.is_empty());
        let list = enabled::read(&path).unwrap();
        let package = &list.packages[0];
        assert_eq!(package.settings["octree_depth"], Setting::Int(11));
        assert_eq!(package.settings["street_raster"], Setting::Bool(true));
        assert_eq!(package.root, store.folder("bigmap", "0.3.0"));
        let plan = enabled::plan(&list, BUILD, BIG_MAPS, |_| true, true);
        assert!(plan.feature("bigmap.octree").is_some());

        // Big Maps changes the simulation: this build mismatch prevents the
        // launcher from starting instead of producing an empty enabled list.
        assert!(matches!(
            store.write_enabled(OTHER_BUILD),
            Err(StoreError::SimulationPackageNotForBuild { .. })
        ));
    }

    #[test]
    fn an_unreadable_registry_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(REGISTRY), b"{ broken").unwrap();
        assert!(matches!(
            Store::open(dir.path()),
            Err(StoreError::Registry(_))
        ));
        assert_eq!(fs::read(dir.path().join(REGISTRY)).unwrap(), b"{ broken");
    }

    #[test]
    fn a_registry_with_a_traversal_path_is_refused_before_uninstall() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("native-mods");
        fs::create_dir_all(&root).unwrap();
        let outside = dir.path().join("keep.txt");
        fs::write(&outside, b"keep me").unwrap();

        let package = package("bad", "1.0.0", &[("../../keep.txt", b"keep me")]);
        let installed = Installed {
            current: "1.0.0".into(),
            previous: None,
            enabled: false,
            settings: Default::default(),
            versions: [("1.0.0".into(), package)].into(),
        };
        let registry = Registry {
            format: REGISTRY_FORMAT,
            serial: 0,
            index_sha256: None,
            packages: [("bad".into(), installed)].into(),
        };
        fs::write(root.join(REGISTRY), serde_json::to_vec(&registry).unwrap()).unwrap();

        assert!(matches!(Store::open(&root), Err(StoreError::Registry(_))));
        assert_eq!(fs::read(outside).unwrap(), b"keep me");
    }

    #[cfg(unix)]
    #[test]
    fn package_operations_refuse_symlinked_parent_directories() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("native-mods");
        let served = Served::default();
        served.serve("bigmap", "1.0.0", V1);
        let package_index = index(vec![package("bigmap", "1.0.0", V1)]);
        let mut store = Store::open(&root).unwrap();
        store
            .install(context(&package_index), "bigmap", &served)
            .unwrap();

        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("sizes.toml"), b"sizes = [128]").unwrap();
        let data = store.folder("bigmap", "1.0.0").join("data");
        fs::remove_dir_all(&data).unwrap();
        symlink(&outside, &data).unwrap();

        assert!(store.verify("bigmap").is_err());
        assert!(store.uninstall("bigmap").is_err());
        assert_eq!(
            fs::read(outside.join("sizes.toml")).unwrap(),
            b"sizes = [128]"
        );
        assert!(
            store
                .folder("bigmap", "1.0.0")
                .join("mod/tpf3mp_bigmap_1/mod.lua")
                .exists()
        );
        assert!(store.registry().packages.contains_key("bigmap"));
    }

    #[test]
    fn a_simulation_package_for_another_build_is_not_written_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("native-mods");
        let mut store = Store::open(&root).unwrap();
        let mut package = package("bigmap", "1.0.0", &[]);
        package.simulation = true;
        package.builds = vec![OTHER_BUILD.into()];
        store.registry.packages.insert(
            "bigmap".into(),
            Installed {
                current: "1.0.0".into(),
                previous: None,
                enabled: true,
                settings: Default::default(),
                versions: [("1.0.0".into(), package)].into(),
            },
        );

        assert!(store.write_enabled(BUILD).is_err());
        assert!(!root.join(enabled::FILE).exists());
    }

    #[test]
    fn a_ui_only_package_for_another_build_is_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("native-mods");
        let mut store = Store::open(&root).unwrap();
        let mut package = package("pages", "1.0.0", &[]);
        package.simulation = false;
        package.builds = vec![OTHER_BUILD.into()];
        store.registry.packages.insert(
            "pages".into(),
            Installed {
                current: "1.0.0".into(),
                previous: None,
                enabled: true,
                settings: Default::default(),
                versions: [("1.0.0".into(), package)].into(),
            },
        );

        let (enabled, left_out) = store.enabled_for(BUILD).unwrap();
        assert_eq!(left_out, ["pages"]);
        assert!(enabled.packages.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn store_open_refuses_a_symlinked_ancestor_before_reading_the_registry() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir_all(outside.join("native-mods")).unwrap();
        fs::write(
            outside.join("native-mods").join(REGISTRY),
            br#"{"format":1,"serial":0,"packages":{}}"#,
        )
        .unwrap();
        let link = dir.path().join("store-link");
        symlink(&outside, &link).unwrap();

        assert!(matches!(
            Store::open(link.join("native-mods")),
            Err(StoreError::UnsafePath(_))
        ));
    }
}

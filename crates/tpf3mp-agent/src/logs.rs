//! A player's logs in one zip, to send with a bug report: TPF3-MP's own
//! logs and the game's, with a manifest saying what is in it and what was
//! looked for but not found. `tpf3mp-agent collect-logs` and the launcher's
//! "Collect logs" button write it.
//!
//! What goes in:
//!
//! - `tpf3mp/logs/`: the launcher's daily logs (`TPF3-MP/logs` in the
//!   per-user data directory), which hold its panics too;
//! - `tpf3mp/hook.log`: the in-game hook's log;
//! - `game/…`: the game's own log and crash dumps, from the candidate
//!   places in [`game_candidates`]: in Transport Fever 3's Steam folder,
//!   where Transport Fever 2 kept its own in its folder. Where Transport
//!   Fever 3 writes them is not known until it is released, so the manifest
//!   marks these places as a guess;
//! - `manifest.txt`: versions, the system, the support ID when known, every
//!   file with its size, and the places that were missing.
//!
//! Nothing else in the data directory is read: the identity key, the
//! remembered server and name, and the worlds stay out. As a second guard,
//! any file whose name looks like a key, certificate or token is withheld
//! wherever it is found. The manifest names no server address, and
//! TPF3-MP's logs hold no IP addresses.
//!
//! Only files changed within the window (`--since`, a week by default) are
//! taken, newest first, until [`DEFAULT_MAX_BYTES`] is reached; a text log
//! that does not fit whole is cut to its end.

use std::{
    fmt::Write as _,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use zip::{CompressionMethod, DateTime, ZipWriter, write::SimpleFileOptions};

/// The most file content a bundle takes, before compression.
pub const DEFAULT_MAX_BYTES: u64 = 64 << 20;
/// How far back files are taken by default.
pub const DEFAULT_SINCE: Duration = Duration::from_secs(7 * 24 * 3600);
/// A text log that does not fit whole is cut to its end, when at least
/// this much of it fits.
const MIN_TAIL: u64 = 256 << 10;
/// How deep folders are walked.
const MAX_DEPTH: usize = 4;
/// Files taken from one place at most, so a folder of thousands of dumps
/// cannot stall the collection.
const MAX_FILES_PER_PLACE: usize = 1000;

/// The game's Steam app IDs whose folders are looked in: Transport Fever
/// 3's ([`crate::steam::TRANSPORT_FEVER_3`]).
// TODO(TF3 release): confirm the paths in its folder.
pub const GAME_STEAM_APPS: &[&str] = &["3493540"];

/// Marks a place taken from Transport Fever 2, until it is confirmed on
/// Transport Fever 3.
const TPF2_GUESS: &str = "TPF2 location, confirm on TF3";

/// A place a log or dump may be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// What it is, for the manifest.
    pub what: String,
    /// Where it is looked for, as the manifest shows it: without user
    /// names or account IDs.
    pub shown: String,
    /// The folder in the zip.
    pub zip_dir: String,
    /// What it is on this computer: files, or folders taken whole.
    pub paths: Vec<PathBuf>,
}

/// TPF3-MP's own logs in `data_dir`.
pub fn own_candidates(data_dir: &Path) -> Vec<Candidate> {
    vec![
        Candidate {
            what: "the launcher's logs".into(),
            shown: "TPF3-MP/logs/".into(),
            zip_dir: "tpf3mp/logs".into(),
            paths: vec![data_dir.join("logs")],
        },
        Candidate {
            what: "the in-game hook's log".into(),
            shown: "TPF3-MP/hook.log".into(),
            zip_dir: "tpf3mp".into(),
            paths: vec![data_dir.join("hook.log")],
        },
    ]
}

/// Where the game keeps its log and crash dumps, on this computer.
pub fn game_candidates() -> Vec<Candidate> {
    // Where Steam is, the folder the registry names on Windows included.
    game_candidates_in(&crate::steam::steam_roots())
}

/// The game's places under these Steam installations: per Steam account,
/// `userdata/<account>/<app>/local/stdout.txt` and its `crash_dump/`
/// folder, where Transport Fever 2 writes them.
pub fn game_candidates_in(steam_roots: &[PathBuf]) -> Vec<Candidate> {
    let mut locals = Vec::new();
    for root in steam_roots {
        let Ok(accounts) = fs::read_dir(root.join("userdata")) else {
            continue;
        };
        for account in accounts.flatten() {
            locals.push(account.path());
        }
    }
    // ~/.steam/steam usually links to ~/.local/share/Steam.
    let mut seen = Vec::new();
    locals.retain(|path| {
        let real = fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        let new = !seen.contains(&real);
        seen.push(real);
        new
    });
    let mut candidates = Vec::new();
    for app in GAME_STEAM_APPS {
        let local = |tail: &str| {
            locals
                .iter()
                .map(|account| account.join(app).join("local").join(tail))
                .collect()
        };
        candidates.push(Candidate {
            what: format!("the game's log, stdout.txt ({TPF2_GUESS})"),
            shown: format!("<Steam>/userdata/<account>/{app}/local/stdout.txt"),
            zip_dir: format!("game/steam-{app}"),
            paths: local("stdout.txt"),
        });
        candidates.push(Candidate {
            what: format!("the game's crash dumps ({TPF2_GUESS})"),
            shown: format!("<Steam>/userdata/<account>/{app}/local/crash_dump/"),
            zip_dir: format!("game/steam-{app}/crash_dump"),
            paths: local("crash_dump"),
        });
    }
    candidates
}

/// A file or folder the player names, such as a log the game wrote
/// somewhere else. The manifest shows only its name.
pub fn extra_candidate(path: &Path) -> Candidate {
    let name = path.file_name().map_or_else(
        || "extra".into(),
        |name| name.to_string_lossy().into_owned(),
    );
    Candidate {
        what: "given on the command line".into(),
        shown: name.clone(),
        zip_dir: format!("extra/{name}"),
        paths: vec![path.to_owned()],
    }
}

/// Whether a file's name looks like a key, certificate or token. Such a
/// file is never bundled, wherever it is found.
pub fn looks_secret(name: &str) -> bool {
    const EXTENSIONS: &[&str] = &[
        "key", "pem", "der", "crt", "cer", "p12", "pfx", "jks", "keystore",
    ];
    const WORDS: &[&str] = &[
        "identity",
        "invite",
        "token",
        "secret",
        "password",
        "credential",
        "private",
    ];
    let name = name.to_ascii_lowercase();
    let extension = Path::new(&name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    EXTENSIONS.contains(&extension) || WORDS.iter().any(|word| name.contains(word))
}

/// Parses a window such as `30m`, `12h`, `3d` or `2w`; `all` for no
/// window.
pub fn parse_since(text: &str) -> Result<Option<Duration>, String> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("all") {
        return Ok(None);
    }
    let split = text
        .find(|c: char| !c.is_ascii_digit())
        .ok_or_else(|| format!("{text}: give a unit, such as 12h or 3d"))?;
    let (number, unit) = text.split_at(split);
    let number: u64 = number
        .parse()
        .map_err(|_| format!("{text}: not a duration, such as 12h or 3d"))?;
    let seconds = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 24 * 3600,
        "w" => 7 * 24 * 3600,
        _ => return Err(format!("{text}: the unit is one of s, m, h, d, w")),
    };
    number
        .checked_mul(seconds)
        .map(|seconds| Some(Duration::from_secs(seconds)))
        .ok_or_else(|| format!("{text}: too long"))
}

/// What a bundle takes and from where.
#[derive(Debug, Clone)]
pub struct Collect {
    /// The places looked in, TPF3-MP's own first.
    pub candidates: Vec<Candidate>,
    /// Only files changed this recently; `None` for all.
    pub since: Option<Duration>,
    /// The most file content taken, before compression.
    pub max_bytes: u64,
    /// The launcher's support ID, when connected.
    pub support_id: Option<String>,
    /// The time the bundle is made at.
    pub now: SystemTime,
}

impl Collect {
    /// TPF3-MP's logs in `data_dir` and the game's, the last week of them,
    /// up to [`DEFAULT_MAX_BYTES`].
    pub fn new(data_dir: &Path) -> Self {
        let mut candidates = own_candidates(data_dir);
        candidates.extend(game_candidates());
        Self {
            candidates,
            since: Some(DEFAULT_SINCE),
            max_bytes: DEFAULT_MAX_BYTES,
            support_id: None,
            now: SystemTime::now(),
        }
    }

    /// Writes the bundle into `out_dir` as `tpf3mp-logs-<time>.zip`.
    pub fn write(&self, out_dir: &Path) -> io::Result<Bundle> {
        let found = self.find();
        let stamp = Stamp::of(self.now);
        fs::create_dir_all(out_dir)?;
        let name = format!("tpf3mp-logs-{}.zip", stamp.compact());
        let path = out_dir.join(&name);
        // Written aside and renamed, so a failure leaves no half a zip.
        let partial = out_dir.join(format!(".{name}.partial"));
        let written = self.write_zip(&partial, &found, &stamp);
        match written {
            Ok(files) => {
                fs::rename(&partial, &path)?;
                Ok(Bundle { path, files })
            }
            Err(error) => {
                let _ = fs::remove_file(&partial);
                Err(error)
            }
        }
    }

    /// Every file in the candidate places, and what was left out.
    fn find(&self) -> Found {
        let mut found = Found::default();
        let cutoff = self
            .since
            .and_then(|since| self.now.checked_sub(since))
            .unwrap_or(UNIX_EPOCH);
        for candidate in &self.candidates {
            let mut any = false;
            for (index, path) in candidate.paths.iter().enumerate() {
                let Ok(meta) = fs::symlink_metadata(path) else {
                    continue;
                };
                any = true;
                let zip_dir = match index {
                    0 => candidate.zip_dir.clone(),
                    _ => format!("{}-{}", candidate.zip_dir, index + 1),
                };
                let mut files = Vec::new();
                if meta.is_dir() {
                    walk(path, &zip_dir, 0, &mut files);
                } else if meta.is_file() {
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    files.push((path.clone(), format!("{zip_dir}/{name}")));
                }
                for (source, zip_name) in files.into_iter().take(MAX_FILES_PER_PLACE) {
                    let name = zip_name.rsplit('/').next().unwrap_or_default();
                    if looks_secret(name) {
                        found.withheld += 1;
                        continue;
                    }
                    let Ok(meta) = fs::metadata(&source) else {
                        continue;
                    };
                    let modified = meta.modified().unwrap_or(self.now);
                    if modified < cutoff {
                        found.older += 1;
                        continue;
                    }
                    found.files.push(FoundFile {
                        source,
                        zip_name,
                        size: meta.len(),
                        modified,
                    });
                }
            }
            if !any {
                found.missing.push(candidate.clone());
            }
        }
        // Newest first, so the size cap drops the oldest.
        found
            .files
            .sort_by_key(|file| std::cmp::Reverse(file.modified));
        found
    }

    fn write_zip(&self, path: &Path, found: &Found, stamp: &Stamp) -> io::Result<Vec<BundledFile>> {
        let mut zip = ZipWriter::new(File::create(path)?);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let mut budget = self.max_bytes;
        let mut bundled = Vec::new();
        let mut over_cap = Vec::new();
        for file in &found.files {
            let take = if file.size <= budget {
                file.size
            } else if is_text(&file.zip_name) && budget >= MIN_TAIL {
                budget
            } else {
                over_cap.push(file);
                continue;
            };
            let Ok(mut source) = File::open(&file.source) else {
                continue;
            };
            if take < file.size {
                source.seek(SeekFrom::Start(file.size - take))?;
            }
            zip.start_file(
                file.zip_name.as_str(),
                options.last_modified_time(zip_time(file.modified)),
            )
            .map_err(io::Error::other)?;
            let copied = io::copy(&mut source.take(take), &mut zip)?;
            budget -= copied;
            bundled.push(BundledFile {
                name: file.zip_name.clone(),
                size: file.size,
                taken: copied,
            });
        }
        let manifest = self.manifest(stamp, found, &bundled, &over_cap);
        zip.start_file("manifest.txt", options)
            .map_err(io::Error::other)?;
        zip.write_all(manifest.as_bytes())?;
        zip.finish().map_err(io::Error::other)?.sync_all()?;
        Ok(bundled)
    }

    fn manifest(
        &self,
        stamp: &Stamp,
        found: &Found,
        bundled: &[BundledFile],
        over_cap: &[&FoundFile],
    ) -> String {
        let mut text = String::new();
        let mut line = |value: String| {
            text.push_str(&value);
            text.push('\n');
        };
        line(format!("TPF3-MP logs, collected {}", stamp.iso()));
        line(format!(
            "agent {}, protocol {}, bridge {}",
            env!("CARGO_PKG_VERSION"),
            tpf3mp_proto::PROTOCOL_VERSION,
            tpf3mp_bridge::BRIDGE_VERSION
        ));
        line(format!(
            "system {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
        line(match &self.support_id {
            Some(support) => format!("support ID {support}"),
            None => "support ID not known (not connected when collected)".into(),
        });
        line(match self.since {
            Some(since) => format!("files changed in the last {}", human_duration(since)),
            None => "files of any age".into(),
        });
        line(format!(
            "at most {} MiB of files, newest first",
            self.max_bytes >> 20
        ));
        line(String::new());
        line(format!("included ({} files):", bundled.len()));
        for file in bundled {
            if file.taken < file.size {
                line(format!(
                    "  {:>12}  {} (its last {} of {} bytes)",
                    file.taken, file.name, file.taken, file.size
                ));
            } else {
                line(format!("  {:>12}  {}", file.size, file.name));
            }
        }
        if !over_cap.is_empty() {
            line(String::new());
            line("left out, over the size cap:".into());
            for file in over_cap {
                line(format!("  {:>12}  {}", file.size, file.zip_name));
            }
        }
        if found.older > 0 {
            line(String::new());
            line(format!(
                "left out, older than the window: {} files",
                found.older
            ));
        }
        if found.withheld > 0 {
            line(String::new());
            line(format!(
                "withheld, looked like keys, certificates or tokens: {} files",
                found.withheld
            ));
        }
        line(String::new());
        if found.missing.is_empty() {
            line("every place looked in was found".into());
        } else {
            line("not found on this computer:".into());
            for candidate in &found.missing {
                line(format!("  {}: {}", candidate.what, candidate.shown));
            }
        }
        text
    }
}

/// A written bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// The zip.
    pub path: PathBuf,
    /// The files in it, besides the manifest.
    pub files: Vec<BundledFile>,
}

/// A file in a bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundledFile {
    /// Its name in the zip.
    pub name: String,
    /// Its whole size.
    pub size: u64,
    /// How much of its end was taken: all of it unless it was cut.
    pub taken: u64,
}

/// Where the launcher puts a bundle: the Downloads folder, or the data
/// directory when there is none.
pub fn default_out_dir(data_dir: &Path) -> PathBuf {
    dirs::download_dir()
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(|| data_dir.to_owned())
}

#[derive(Debug, Default)]
struct Found {
    files: Vec<FoundFile>,
    missing: Vec<Candidate>,
    withheld: usize,
    older: usize,
}

#[derive(Debug)]
struct FoundFile {
    source: PathBuf,
    zip_name: String,
    size: u64,
    modified: SystemTime,
}

/// The files under `dir`, without following links.
fn walk(dir: &Path, zip_dir: &str, depth: usize, files: &mut Vec<(PathBuf, String)>) {
    if depth > MAX_DEPTH || files.len() >= MAX_FILES_PER_PLACE {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let zip_name = format!("{zip_dir}/{name}");
        if kind.is_dir() {
            walk(&entry.path(), &zip_name, depth + 1, files);
        } else if kind.is_file() {
            files.push((entry.path(), zip_name));
        }
    }
}

/// Whether a file is a text log, whose end is worth keeping on its own.
fn is_text(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with(".log") || name.ends_with(".txt") || name.contains(".log.")
}

fn human_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    match seconds {
        s if s % (24 * 3600) == 0 => format!("{} days", s / (24 * 3600)),
        s if s % 3600 == 0 => format!("{} hours", s / 3600),
        s if s % 60 == 0 => format!("{} minutes", s / 60),
        s => format!("{s} seconds"),
    }
}

fn zip_time(time: SystemTime) -> DateTime {
    let stamp = Stamp::of(time);
    u16::try_from(stamp.year)
        .ok()
        .and_then(|year| {
            DateTime::from_date_and_time(
                year,
                stamp.month,
                stamp.day,
                stamp.hour,
                stamp.minute,
                stamp.second,
            )
            .ok()
        })
        .unwrap_or_default()
}

/// A moment in UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    year: i64,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

impl Stamp {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn of(time: SystemTime) -> Self {
        let seconds = time
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let days = i64::try_from(seconds / 86_400).unwrap_or(0);
        let rest = seconds % 86_400;
        // Howard Hinnant's days-to-civil.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
        let year = yoe + era * 400 + i64::from(month <= 2);
        Self {
            year,
            month,
            day,
            hour: (rest / 3600) as u8,
            minute: (rest / 60 % 60) as u8,
            second: (rest % 60) as u8,
        }
    }

    fn compact(&self) -> String {
        format!(
            "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    fn iso(&self) -> String {
        let mut text = String::new();
        let _ = write!(
            text,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        );
        text
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::io::Read;

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tpf3mp-logs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn set_age(path: &Path, now: SystemTime, age: Duration) {
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(now - age)
            .unwrap();
    }

    /// The zip's files and their contents.
    fn read_zip(path: &Path) -> Vec<(String, Vec<u8>)> {
        let mut zip = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
        (0..zip.len())
            .map(|index| {
                let mut file = zip.by_index(index).unwrap();
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).unwrap();
                (file.name().to_owned(), bytes)
            })
            .collect()
    }

    fn collect(data: &Path, steam: &Path, now: SystemTime) -> Collect {
        let mut candidates = own_candidates(data);
        candidates.extend(game_candidates_in(&[steam.to_owned()]));
        Collect {
            candidates,
            since: Some(DEFAULT_SINCE),
            max_bytes: DEFAULT_MAX_BYTES,
            support_id: Some("s-3f2a".into()),
            now,
        }
    }

    #[test]
    fn a_bundle_holds_the_logs_and_a_manifest_but_no_keys() {
        let root = temp("bundle");
        let (data, steam, out) = (root.join("TPF3-MP"), root.join("Steam"), root.join("out"));
        let now = SystemTime::now();
        write(
            &data.join("logs/launcher.2026-09-26.log"),
            b"the launcher starts\n",
        );
        write(&data.join("hook.log"), b"[1] hook bootstrap starting\n");
        // What must never leave the computer.
        write(&data.join("identity.key"), b"PRIVATE KEY");
        write(
            &data.join("launcher.json"),
            b"{\"server\":\"203.0.113.9:29470\"}",
        );
        write(&data.join("worlds/tpf3mp/saves/1.sav"), b"a world");
        write(&data.join("logs/invite.key"), b"INVITE KEY");
        write(&data.join("logs/server.pem"), b"CERTIFICATE");
        write(&data.join("logs/session-token.txt"), b"TOKEN");
        // The game's stdout, under one Steam account; no crash dumps.
        let local = steam.join("userdata/12345678/3493540/local");
        write(&local.join("stdout.txt"), b"game output\n");

        let bundle = collect(&data, &steam, now).write(&out).unwrap();

        let files = read_zip(&bundle.path);
        let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert!(
            names.contains(&"tpf3mp/logs/launcher.2026-09-26.log"),
            "{names:?}"
        );
        assert!(names.contains(&"tpf3mp/hook.log"), "{names:?}");
        assert!(
            names.contains(&"game/steam-3493540/stdout.txt"),
            "{names:?}"
        );
        assert!(names.contains(&"manifest.txt"), "{names:?}");
        assert_eq!(names.len(), 4, "{names:?}");
        for (name, bytes) in &files {
            for secret in [
                &b"PRIVATE KEY"[..],
                b"INVITE KEY",
                b"CERTIFICATE",
                b"TOKEN",
                b"203.0.113.9",
            ] {
                assert!(
                    !bytes.windows(secret.len()).any(|window| window == secret),
                    "{name} holds a secret"
                );
            }
        }

        let manifest = String::from_utf8(
            files
                .iter()
                .find(|f| f.0 == "manifest.txt")
                .unwrap()
                .1
                .clone(),
        )
        .unwrap();
        assert!(manifest.contains(&format!("agent {}", env!("CARGO_PKG_VERSION"))));
        assert!(manifest.contains(&format!("protocol {}", tpf3mp_proto::PROTOCOL_VERSION)));
        assert!(manifest.contains(&format!("bridge {}", tpf3mp_bridge::BRIDGE_VERSION)));
        assert!(manifest.contains(std::env::consts::OS));
        assert!(manifest.contains("support ID s-3f2a"));
        assert!(
            manifest.contains("20  tpf3mp/logs/launcher.2026-09-26.log"),
            "{manifest}"
        );
        assert!(manifest.contains("withheld, looked like keys, certificates or tokens: 3 files"));
        assert!(manifest.contains("the game's crash dumps (TPF2 location, confirm on TF3)"));
        assert!(manifest.contains("<Steam>/userdata/<account>/3493540/local/crash_dump/"));
        // No account ID or local path in the manifest.
        assert!(!manifest.contains("12345678"), "{manifest}");
        assert!(!manifest.contains(&*root.to_string_lossy()), "{manifest}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_newest_files_fill_the_size_cap_and_a_long_log_keeps_its_end() {
        let root = temp("cap");
        let (data, out) = (root.join("TPF3-MP"), root.join("out"));
        let now = SystemTime::now();
        let logs = data.join("logs");
        write(&logs.join("launcher.3.log"), &vec![b'n'; 300 << 10]);
        let mut long = vec![b'a'; 400 << 10];
        long.extend_from_slice(b"THE END");
        write(&logs.join("launcher.2.log"), &long);
        write(&logs.join("dump.dmp"), &vec![0; 400 << 10]);
        write(&logs.join("launcher.1.log"), &vec![b'o'; 100 << 10]);
        set_age(&logs.join("launcher.3.log"), now, Duration::from_secs(60));
        set_age(&logs.join("launcher.2.log"), now, Duration::from_secs(120));
        set_age(&logs.join("dump.dmp"), now, Duration::from_secs(180));
        set_age(&logs.join("launcher.1.log"), now, Duration::from_secs(240));
        // Older than the window.
        write(&logs.join("launcher.0.log"), b"last month");
        set_age(
            &logs.join("launcher.0.log"),
            now,
            Duration::from_secs(30 * 24 * 3600),
        );

        let collect = Collect {
            candidates: own_candidates(&data),
            since: Some(DEFAULT_SINCE),
            max_bytes: 700 << 10,
            support_id: None,
            now,
        };
        let bundle = collect.write(&out).unwrap();

        // 300 KiB whole, then the end of the 400 KiB log in the 400 KiB
        // left; the dump does not fit and is not text, and nothing is left
        // for the oldest.
        let taken: Vec<(&str, u64, u64)> = bundle
            .files
            .iter()
            .map(|file| (file.name.as_str(), file.size, file.taken))
            .collect();
        assert_eq!(
            taken,
            [
                ("tpf3mp/logs/launcher.3.log", 300 << 10, 300 << 10),
                ("tpf3mp/logs/launcher.2.log", (400 << 10) + 7, 400 << 10),
            ]
        );
        let files = read_zip(&bundle.path);
        let cut = &files
            .iter()
            .find(|f| f.0 == "tpf3mp/logs/launcher.2.log")
            .unwrap()
            .1;
        assert!(cut.ends_with(b"THE END"));
        let manifest = String::from_utf8(
            files
                .iter()
                .find(|f| f.0 == "manifest.txt")
                .unwrap()
                .1
                .clone(),
        )
        .unwrap();
        assert!(
            manifest.contains("left out, over the size cap:"),
            "{manifest}"
        );
        assert!(manifest.contains("tpf3mp/logs/dump.dmp"), "{manifest}");
        assert!(
            manifest.contains("tpf3mp/logs/launcher.1.log"),
            "{manifest}"
        );
        assert!(
            manifest.contains("older than the window: 1 files"),
            "{manifest}"
        );
        assert!(manifest.contains("support ID not known"), "{manifest}");
        assert!(
            manifest.contains("the in-game hook's log: TPF3-MP/hook.log"),
            "{manifest}"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn keys_certificates_and_tokens_look_secret() {
        for name in [
            "identity.key",
            "invite.key",
            "cert.der",
            "server.PEM",
            "client.p12",
            "api-token.txt",
            "Identity.json",
        ] {
            assert!(looks_secret(name), "{name}");
        }
        for name in [
            "launcher.2026-09-26.log",
            "hook.log",
            "stdout.txt",
            "crash.dmp",
        ] {
            assert!(!looks_secret(name), "{name}");
        }
    }

    #[test]
    fn windows_parse() {
        assert_eq!(parse_since("30m"), Ok(Some(Duration::from_secs(1800))));
        assert_eq!(parse_since("2h"), Ok(Some(Duration::from_secs(7200))));
        assert_eq!(parse_since("3d"), Ok(Some(Duration::from_secs(3 * 86_400))));
        assert_eq!(parse_since("all"), Ok(None));
        assert!(parse_since("12").is_err());
        assert!(parse_since("h").is_err());
        assert!(parse_since("5y").is_err());
    }

    #[test]
    fn stamps_are_utc_calendar_times() {
        let stamp = Stamp::of(UNIX_EPOCH + Duration::from_secs(1_790_417_045));
        assert_eq!(stamp.iso(), "2026-09-26T10:04:05Z");
        assert_eq!(stamp.compact(), "20260926T100405Z");
        assert_eq!(Stamp::of(UNIX_EPOCH).iso(), "1970-01-01T00:00:00Z");
        assert_eq!(
            Stamp::of(UNIX_EPOCH + Duration::from_secs(951_782_400)).iso(),
            "2000-02-29T00:00:00Z"
        );
    }

    #[test]
    fn the_game_looked_for_is_transport_fever_3() {
        let tpf3 = crate::steam::TRANSPORT_FEVER_3.to_string();
        assert_eq!(GAME_STEAM_APPS.to_vec(), vec![tpf3.as_str()]);
    }
}

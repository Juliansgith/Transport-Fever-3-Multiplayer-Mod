//! Launchers that meet on one game link: a launcher never runs quietly next
//! to one of another build (docs/PLAYING.md, "Another TPF3-MP launcher is
//! running").
//!
//! The launcher that holds the game's link serves the game: its main
//! menu's Multiplayer window connects through that launcher alone. A newer
//! launcher started next to an older one would otherwise leave the game
//! with the older one, which then connects with its older protocol.
//!
//! Each launcher writes a record beside its link, in the per-user data
//! directory (`launchers/<link>.json`): its process, its file and its
//! build. A launcher that finds the link held:
//!
//! - by the same build, starts as before (the window says that the other
//!   one has the game's link);
//! - by another build that wrote a record, asks it to close
//!   (`launchers/<link>.quit.json`) and waits for it. The other closes
//!   unless it is in a room, then says why it stays;
//! - by a build that says nothing of itself (one older than these records),
//!   or one that stays, does not start, and names both builds and the other
//!   one's file.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::{LINK_HELD_WAIT, LauncherHandle, State};
use crate::about::{self, Build};

/// How often a launcher looks for a request to close.
pub const QUIT_POLL: Duration = Duration::from_millis(500);
/// How long a launcher waits for another to take its request.
const QUIT_TAKEN_WAIT: Duration = Duration::from_secs(5);
/// How long it waits, once taken, for the other to let go of the link.
const QUIT_DONE_WAIT: Duration = Duration::from_secs(5);

/// What a launcher says of itself beside its link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub pid: u32,
    pub exe: Option<PathBuf>,
    pub build: Build,
}

/// Another launcher that holds the link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Other {
    pub pid: u32,
    /// Its file: where the system says, else where its record says.
    pub exe: Option<PathBuf>,
    /// Its build, when it wrote a record.
    pub build: Option<Build>,
}

/// What a launcher finds on its link when it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Meeting {
    /// No other launcher holds it.
    Alone,
    /// One of this very build does.
    Same(Other),
    /// One of another build, or one that does not say which.
    Different(Other),
}

/// Who holds the link: `holder` is the live launcher process holding it,
/// `record` what was written beside it, `exe_of` the file a process runs.
/// A record counts only if it names the holder, and the holder's file when
/// the system says which that is: a process id can be reused.
pub fn meet(
    ours: &Build,
    holder: Option<u32>,
    record: Option<Record>,
    exe_of: impl FnOnce(u32) -> Option<PathBuf>,
) -> Meeting {
    let Some(pid) = holder else {
        return Meeting::Alone;
    };
    let running = exe_of(pid);
    let record = record.filter(|record| {
        record.pid == pid
            && match (&running, &record.exe) {
                (Some(running), Some(written)) => same_file(running, written),
                _ => true,
            }
    });
    let other = Other {
        pid,
        exe: running.or_else(|| record.as_ref().and_then(|record| record.exe.clone())),
        build: record.map(|record| record.build),
    };
    if other.build.as_ref() == Some(ours) {
        Meeting::Same(other)
    } else {
        Meeting::Different(other)
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}

fn theirs(other: &Other) -> String {
    other.build.as_ref().map_or_else(
        || "an older TPF3-MP that does not say its version".to_owned(),
        Build::describe,
    )
}

/// Why a launcher does not start next to `other`, of another build.
pub fn refusal(other: &Other, ours: &Build, our_exe: Option<&Path>) -> String {
    format!(
        "Another TPF3-MP launcher is already running, of a different build: {}, {} (process {}). \
         This one is {}, {}. Close the other launcher, and Transport Fever 3 if it started it, \
         then start this one again.",
        theirs(other),
        about::shown(other.exe.as_deref()),
        other.pid,
        ours.describe(),
        about::shown(our_exe),
    )
}

/// What a launcher says when one of its own build already has the game's
/// link, and with it the link's worlds, so it cannot start beside it.
pub fn already_running(other: &Other) -> String {
    format!(
        "TPF3-MP is already running: {}, {} (process {}). Use its window (it may be minimised, \
        or behind the game), or close it, then start TPF3-MP again.",
        theirs(other),
        about::shown(other.exe.as_deref()),
        other.pid,
    )
}

/// Whether `theirs` is a newer build than `ours`: a newer protocol, or the
/// same protocol and a later version.
pub fn newer(theirs: &Build, ours: &Build) -> bool {
    fn parts(version: &str) -> Vec<u64> {
        version
            .split(['.', '-', '+'])
            .map_while(|part| part.parse().ok())
            .collect()
    }
    (theirs.protocol, parts(&theirs.version)) > (ours.protocol, parts(&ours.version))
}

/// Why a launcher does not start next to a newer one.
pub fn older_than_running(other: &Other, ours: &Build, our_exe: Option<&Path>) -> String {
    format!(
        "A newer TPF3-MP launcher is already running: {}, {} (process {}). The one you started \
        is older: {}, {}. Use the running one's window, and start TPF3-MP from the newest \
        install from now on (a shortcut may still point at the old one).",
        theirs(other),
        about::shown(other.exe.as_deref()),
        other.pid,
        ours.describe(),
        about::shown(our_exe),
    )
}

/// Why a launcher does not start when `other` stays: it is in a room.
pub fn stayed(other: &Other, ours: &Build, our_exe: Option<&Path>) -> String {
    format!(
        "Another TPF3-MP launcher, of a different build, is in a room and stays open: {}, {} \
         (process {}). This one is {}, {}. Leave the room there and close that launcher, then \
         start this one again.",
        theirs(other),
        about::shown(other.exe.as_deref()),
        other.pid,
        ours.describe(),
        about::shown(our_exe),
    )
}

/// A request to the launcher `to` to close, from the launcher `from`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuitRequest {
    pub to: u32,
    pub from: u32,
    pub exe: Option<PathBuf>,
    pub build: Build,
}

/// The records' folder in the per-user data directory.
pub fn dir() -> Option<PathBuf> {
    super::setup::data_dir()
        .ok()
        .map(|dir| dir.join("launchers"))
}

fn stem(link: &str) -> String {
    link.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn record_path(dir: &Path, link: &str) -> PathBuf {
    dir.join(format!("{}.json", stem(link)))
}

fn quit_path(dir: &Path, link: &str) -> PathBuf {
    dir.join(format!("{}.quit.json", stem(link)))
}

fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Writes `value` whole: to a file beside `path`, then in its place.
fn write<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    let partial = path.with_extension(format!("{}.partial", std::process::id()));
    fs::write(&partial, bytes)?;
    fs::rename(&partial, path)
}

/// What the launcher on `link` said of itself, if anything.
pub fn read_record(dir: &Path, link: &str) -> Option<Record> {
    read(&record_path(dir, link))
}

/// This launcher's record on its link, taken away again when dropped.
#[derive(Debug)]
pub struct Registration {
    path: PathBuf,
    pid: u32,
}

impl Registration {
    pub fn write(dir: &Path, link: &str, record: &Record) -> io::Result<Self> {
        let path = record_path(dir, link);
        write(&path, record)?;
        Ok(Self {
            path,
            pid: record.pid,
        })
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        // Only its own: a launcher that took over since wrote its own.
        if read::<Record>(&self.path).is_some_and(|record| record.pid == self.pid) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Asks the launcher `request.to` on `link` to close.
pub fn ask_to_quit(dir: &Path, link: &str, request: &QuitRequest) -> io::Result<()> {
    write(&quit_path(dir, link), request)
}

/// A request to `own` to close, taken: the file is removed, so the asking
/// launcher sees it was.
pub fn take_quit_request(dir: &Path, link: &str, own: u32) -> Option<QuitRequest> {
    let path = quit_path(dir, link);
    let request: QuitRequest = read(&path)?;
    if request.to != own {
        return None;
    }
    let _ = fs::remove_file(&path);
    Some(request)
}

/// Whether a launcher in `state` stays when asked to close: it is in a room,
/// whose player it would take out (and whose game, once it runs).
pub fn stays(state: &State) -> bool {
    state.room.is_some()
}

/// How a launcher's start went on its link.
#[derive(Debug)]
pub enum Arrived {
    /// It serves the link, and its record is there while this lives.
    Serving(Option<Registration>),
    /// A launcher of the same build serves it, as before these records.
    Beside(Other),
}

/// Meets whoever holds `link`, as the module says, before the launcher
/// opens it. `Err` is what the player is told when this launcher must not
/// start. Blocks for as long as another launcher takes to close.
pub fn arrive(link: &str) -> Result<Arrived, String> {
    let ours = Build::this();
    let our_exe = about::exe();
    let dir = dir();
    let holder = tpf3mp_ipc::Link::held_by_another_agent(link, LINK_HELD_WAIT);
    let record = dir.as_deref().and_then(|dir| read_record(dir, link));
    let meeting = meet(&ours, holder, record, tpf3mp_launch::process_path);
    match meeting {
        Meeting::Alone => {}
        Meeting::Same(other) => {
            info!(
                pid = other.pid,
                "a launcher of this same build holds the game's link"
            );
            return Ok(Arrived::Beside(other));
        }
        Meeting::Different(other) => {
            let (Some(dir), Some(build)) = (dir.as_deref(), &other.build) else {
                let message = refusal(&other, &ours, our_exe.as_deref());
                warn!(%message);
                return Err(message);
            };
            // An old copy, as from a shortcut to an old install, does not
            // put a newer one out.
            if newer(build, &ours) {
                let message = older_than_running(&other, &ours, our_exe.as_deref());
                warn!(%message);
                return Err(message);
            }
            info!(
                pid = other.pid,
                theirs = %build.describe(),
                "a launcher of another build holds the game's link; asking it to close"
            );
            let request = QuitRequest {
                to: other.pid,
                from: std::process::id(),
                exe: our_exe.clone(),
                build: ours.clone(),
            };
            ask_to_quit(dir, link, &request)
                .map_err(|error| format!("cannot ask the other launcher to close: {error}"))?;
            if !wait_for_quit(dir, link, other.pid) {
                let _ = fs::remove_file(quit_path(dir, link));
                let message = stayed(&other, &ours, our_exe.as_deref());
                warn!(%message);
                return Err(message);
            }
            info!(pid = other.pid, theirs = %build.describe(), "took over the game's link from the other launcher");
        }
    }
    let registration = dir.as_deref().and_then(|dir| {
        Registration::write(
            dir,
            link,
            &Record {
                pid: std::process::id(),
                exe: our_exe,
                build: ours,
            },
        )
        .inspect_err(|error| warn!(%error, "cannot write the launcher's record"))
        .ok()
    });
    Ok(Arrived::Serving(registration))
}

/// Waits for launcher `pid` to take the request to close and let go of
/// `link`. False when it did not take it, or took it and stayed.
fn wait_for_quit(dir: &Path, link: &str, pid: u32) -> bool {
    let asked = Instant::now();
    let mut taken: Option<Instant> = None;
    loop {
        if tpf3mp_ipc::Link::held_by_another_agent(link, LINK_HELD_WAIT) != Some(pid) {
            return true;
        }
        let now = Instant::now();
        match taken {
            None if !quit_path(dir, link).exists() => taken = Some(now),
            None if now.duration_since(asked) > QUIT_TAKEN_WAIT => return false,
            Some(at) if now.duration_since(at) > QUIT_DONE_WAIT => return false,
            _ => {}
        }
        std::thread::sleep(QUIT_POLL);
    }
}

/// Watches for another launcher's request that this one close, on its own
/// thread, for as long as the process runs. `quit` closes the launcher; it
/// is called at most once, and not while the launcher [`stays`].
pub fn watch(link: String, handle: LauncherHandle, quit: impl FnOnce() + Send + 'static) {
    let Some(dir) = dir() else {
        return;
    };
    let own = std::process::id();
    let spawned = std::thread::Builder::new()
        .name("tpf3mp-instance".into())
        .spawn(move || {
            loop {
                std::thread::sleep(QUIT_POLL);
                let Some(request) = take_quit_request(&dir, &link, own) else {
                    continue;
                };
                let from = format!(
                    "{}, {} (process {})",
                    request.build.describe(),
                    about::shown(request.exe.as_deref()),
                    request.from
                );
                if stays(&handle.state()) {
                    warn!(%from, "another launcher asked this one to close; it stays, in a room");
                    handle.shared.status().notice(format!(
                        "Another TPF3-MP launcher was started: {from}. This one stays open while \
                         it is in a room; leave the room and close it to use the other."
                    ));
                    continue;
                }
                info!(%from, "another launcher of another build takes over; this one closes");
                quit();
                return;
            }
        });
    if let Err(error) = spawned {
        warn!(%error, "cannot watch for other launchers");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(protocol: u32, commit: &str) -> Build {
        Build {
            version: "0.1.0".into(),
            protocol,
            bridge: 19,
            commit: commit.into(),
        }
    }

    fn exe(name: &str) -> PathBuf {
        PathBuf::from(if cfg!(windows) {
            format!(r"C:\Games\{name}\TPF3-MP.exe")
        } else {
            format!("/opt/{name}/tpf3mp-launcher")
        })
    }

    fn record(pid: u32, at: &str, build: Build) -> Record {
        Record {
            pid,
            exe: Some(exe(at)),
            build,
        }
    }

    #[test]
    fn a_free_link_is_taken() {
        let ours = build(13, "bbbbbbb");
        assert_eq!(
            meet(&ours, None, Some(record(7, "old", ours.clone())), |_| None),
            Meeting::Alone,
            "a record left by a launcher that is gone counts for nothing"
        );
    }

    #[test]
    fn the_same_build_starts_beside_as_before() {
        let ours = build(13, "bbbbbbb");
        let met = meet(&ours, Some(7), Some(record(7, "new", ours.clone())), |_| {
            Some(exe("new"))
        });
        assert_eq!(
            met,
            Meeting::Same(Other {
                pid: 7,
                exe: Some(exe("new")),
                build: Some(ours),
            })
        );
    }

    #[test]
    fn another_build_or_none_said_is_different() {
        let ours = build(13, "bbbbbbb");
        let old = build(12, "aaaaaaa");
        assert_eq!(
            meet(&ours, Some(7), Some(record(7, "old", old.clone())), |_| {
                Some(exe("old"))
            }),
            Meeting::Different(Other {
                pid: 7,
                exe: Some(exe("old")),
                build: Some(old.clone()),
            })
        );
        assert_eq!(
            meet(
                &ours,
                Some(7),
                Some(record(7, "old", build(13, "aaaaaaa"))),
                |_| None
            ),
            Meeting::Different(Other {
                pid: 7,
                exe: Some(exe("old")),
                build: Some(build(13, "aaaaaaa")),
            }),
            "the same protocol from another commit is another build"
        );
        assert_eq!(
            meet(&ours, Some(7), None, |_| Some(exe("old"))),
            Meeting::Different(Other {
                pid: 7,
                exe: Some(exe("old")),
                build: None,
            }),
            "a launcher from before these records"
        );
    }

    #[test]
    fn a_record_counts_only_for_its_own_process_and_file() {
        let ours = build(13, "bbbbbbb");
        let other_pid = meet(&ours, Some(8), Some(record(7, "new", ours.clone())), |_| {
            Some(exe("old"))
        });
        assert!(
            matches!(&other_pid, Meeting::Different(Other { build: None, .. })),
            "{other_pid:?}"
        );
        let reused = meet(&ours, Some(7), Some(record(7, "new", ours.clone())), |_| {
            Some(exe("old"))
        });
        assert!(
            matches!(&reused, Meeting::Different(Other { build: None, exe: Some(file), .. }) if *file == exe("old")),
            "its process id now runs another file: {reused:?}"
        );
    }

    #[test]
    fn the_refusal_names_both_builds_and_the_other_file() {
        let ours = build(13, "bbbbbbb");
        let other = Other {
            pid: 4242,
            exe: Some(exe("old")),
            build: Some(build(12, "aaaaaaa")),
        };
        let message = refusal(&other, &ours, Some(&exe("new")));
        assert!(
            message.contains("TPF3-MP 0.1.0 (protocol 12, commit aaaaaaa)"),
            "{message}"
        );
        assert!(
            message.contains("TPF3-MP 0.1.0 (protocol 13, commit bbbbbbb)"),
            "{message}"
        );
        assert!(
            message.contains(&exe("old").display().to_string()),
            "{message}"
        );
        assert!(
            message.contains(&exe("new").display().to_string()),
            "{message}"
        );
        assert!(message.contains("process 4242"), "{message}");
        let unknown = refusal(
            &Other {
                build: None,
                ..other.clone()
            },
            &ours,
            None,
        );
        assert!(
            unknown.contains("an older TPF3-MP that does not say its version"),
            "{unknown}"
        );
        assert!(stayed(&other, &ours, None).contains("in a room"));
        let same = already_running(&Other {
            build: Some(ours.clone()),
            ..other.clone()
        });
        assert!(
            same.starts_with(
                "TPF3-MP is already running: TPF3-MP 0.1.0 (protocol 13, commit bbbbbbb)"
            ),
            "{same}"
        );
        assert!(same.contains("Use its window"), "{same}");
        let older = older_than_running(&other, &ours, None);
        for words in [
            &message,
            &unknown,
            &stayed(&other, &ours, None),
            &same,
            &older,
        ] {
            assert!(!words.contains("  "), "one space between words: {words}");
        }
    }

    #[test]
    fn an_older_build_does_not_put_out_a_newer_one() {
        let ours = build(13, "bbbbbbb");
        assert!(newer(&build(14, "aaaaaaa"), &ours));
        assert!(!newer(&build(12, "aaaaaaa"), &ours));
        assert!(!newer(&build(13, "aaaaaaa"), &ours), "the same version");
        let later = Build {
            version: "0.2.0".into(),
            ..build(13, "aaaaaaa")
        };
        assert!(newer(&later, &ours));
        assert!(!newer(&ours, &later));
        let other = Other {
            pid: 4242,
            exe: Some(exe("new")),
            build: Some(build(14, "aaaaaaa")),
        };
        let message = older_than_running(&other, &ours, Some(&exe("old")));
        assert!(
            message.starts_with(
                "A newer TPF3-MP launcher is already running: TPF3-MP 0.1.0 (protocol 14, commit aaaaaaa)"
            ),
            "{message}"
        );
        assert!(
            message.contains(&exe("old").display().to_string()),
            "{message}"
        );
    }

    #[test]
    fn records_and_requests_go_through_files() {
        let dir = tempfile::tempdir().unwrap();
        let link = "tpf3mp/test link";
        let mine = record(std::process::id(), "new", build(13, "bbbbbbb"));
        {
            let _registration = Registration::write(dir.path(), link, &mine).unwrap();
            assert_eq!(read_record(dir.path(), link), Some(mine.clone()));
        }
        assert_eq!(
            read_record(dir.path(), link),
            None,
            "taken away when dropped"
        );

        let request = QuitRequest {
            to: 7,
            from: 8,
            exe: Some(exe("new")),
            build: build(13, "bbbbbbb"),
        };
        ask_to_quit(dir.path(), link, &request).unwrap();
        assert_eq!(take_quit_request(dir.path(), link, 9), None, "not for 9");
        assert_eq!(take_quit_request(dir.path(), link, 7), Some(request));
        assert_eq!(take_quit_request(dir.path(), link, 7), None, "taken once");
    }

    #[test]
    fn a_launcher_in_a_room_stays() {
        let mut state = State::default();
        assert!(!stays(&state));
        state.room = Some(crate::launcher::Room {
            name: "r".into(),
            rules: "native".into(),
            phase: crate::launcher::Phase::Lobby,
            invite: None,
            you_own: true,
            max_players: 8,
            has_password: false,
            members: Vec::new(),
            competitive: false,
        });
        assert!(stays(&state));
    }
}

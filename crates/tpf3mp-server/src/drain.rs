//! Two releases' servers at once (DECISIONS.md, D33; OPERATIONS.md, "Two
//! versions at once"). An old release's server *drains* beside the newer
//! one: it keeps the games running on it for their own players, opens no
//! rooms and takes no new players. Once nobody has been connected to one of
//! its games for a while, it *hands the game over*: it writes the game's log,
//! its snapshot pointer and every world snapshot it holds into a directory
//! the newer server reads, which *takes it in* through the same recovery a
//! restart uses. Players who updated then rejoin it there with their invite.
//!
//! An export is written under a `.partial` name and renamed to `.handover`
//! only once the room has checked that nobody came back and nothing was
//! logged meanwhile, so the newer server never takes in a game the old one
//! still plays. A game that cannot be written, or taken in, is never lost:
//! the old server keeps playing it, and the newer one sets a bad hand-over
//! aside as `.broken`, unchanged.

use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use tpf3mp_proto::RoomId;
use tpf3mp_snapshot::{Manifest, ManifestId};
use tracing::warn;

use crate::{persist::RoomLog, snapshots::Pointer, snapshots::Snapshots};

/// Where a draining server hands its games over, and how long nobody must
/// have been connected to a running game before it is.
#[derive(Debug, Clone)]
pub struct HandoverConfig {
    pub dir: PathBuf,
    pub after: Duration,
}

impl HandoverConfig {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            // Long enough for a player's reconnect; short enough that one
            // who updated finds the game on the newer server soon.
            after: Duration::from_secs(180),
        }
    }
}

/// Whether the server drains, and where its games go.
#[derive(Debug)]
pub(crate) struct Drain {
    draining: AtomicBool,
    pub(crate) handover: Option<HandoverConfig>,
}

impl Drain {
    pub(crate) fn new(draining: bool, handover: Option<HandoverConfig>) -> Self {
        Self {
            draining: AtomicBool::new(draining),
            handover,
        }
    }

    pub(crate) fn draining(&self) -> bool {
        self.draining.load(Ordering::Acquire)
    }

    /// Starts draining, for good; whether it had not started already.
    pub(crate) fn start(&self) -> bool {
        !self.draining.swap(true, Ordering::AcqRel)
    }

    /// Where a running game nobody has been connected to for so long is
    /// handed over: only while draining, and only where configured.
    pub(crate) fn hands_over(&self) -> Option<&HandoverConfig> {
        self.handover.as_ref().filter(|_| self.draining())
    }
}

/// How often a server looks for games an older one handed over.
pub(crate) const TAKE_IN_EVERY: Duration = Duration::from_secs(5);

const LOG: &str = "room.log";
const POINTER: &str = "room.snapshot";
const WORLDS: &str = "worlds";
const PARTIAL: &str = "partial";
const HANDED_OVER: &str = "handover";

/// Where a game is written before it is handed over.
pub(crate) fn partial_path(dir: &Path, room: &RoomId) -> PathBuf {
    dir.join(format!("{room}.{PARTIAL}"))
}

/// Where a handed-over game waits for the newer server.
pub(crate) fn handover_path(dir: &Path, room: &RoomId) -> PathBuf {
    dir.join(format!("{room}.{HANDED_OVER}"))
}

/// Writes `room`'s game into `dir`, under its `.partial` name: its log, its
/// snapshot pointer if it has one, and each snapshot it holds as a whole
/// world beside its manifest, everything flushed to disk. Blocking. The
/// newer server sees none of it until [`publish`].
pub(crate) fn export(
    data_dir: &Path,
    snapshots: Option<&Snapshots>,
    room: &RoomId,
    held: &[ManifestId],
    dir: &Path,
) -> io::Result<PathBuf> {
    let to = partial_path(dir, room);
    if fs::symlink_metadata(&to).is_ok() {
        fs::remove_dir_all(&to)?;
    }
    fs::create_dir_all(to.join(WORLDS))?;
    copy_synced(&RoomLog::path_for(data_dir, room), &to.join(LOG))?;
    let pointer = Pointer::path_for(data_dir, room);
    if fs::symlink_metadata(&pointer).is_ok_and(|meta| meta.is_file()) {
        copy_synced(&pointer, &to.join(POINTER))?;
    }
    if let Some(snapshots) = snapshots {
        for id in held {
            let manifest = snapshots.store.manifest(id).map_err(io::Error::other)?;
            let worlds = to.join(WORLDS);
            write_synced(&worlds.join(format!("{id}.manifest")), &manifest.to_bytes())?;
            let world = worlds.join(format!("{id}.world"));
            snapshots
                .store
                .assemble(&manifest, &world)
                .map_err(io::Error::other)?;
            flush(&world)?;
        }
    }
    sync_dir(&to.join(WORLDS));
    sync_dir(&to);
    Ok(to)
}

/// Hands an export over: renames it to the name the newer server takes in.
pub(crate) fn publish(partial: &Path, dir: &Path, room: &RoomId) -> io::Result<()> {
    fs::rename(partial, handover_path(dir, room))?;
    sync_dir(dir);
    Ok(())
}

/// The games handed over in `dir` and not taken in yet: its `.handover`
/// directories, never a link to one.
pub(crate) fn ready(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut ready: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == HANDED_OVER))
        .collect();
    ready.sort();
    ready
}

/// Takes a handed-over game into `data_dir`: each world ingested into
/// `snapshots` and checked to be the snapshot its manifest names, then its
/// pointer and its log moved in, the log last. Returns the log's path for
/// recovery. Blocking. Fails, changing nothing in `data_dir`, if a world is
/// not its manifest's or a game of that name is there already.
pub(crate) fn take_in(
    from: &Path,
    data_dir: &Path,
    snapshots: Option<&Snapshots>,
) -> io::Result<PathBuf> {
    let name = from
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| {
            stem.starts_with("r-") && stem.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        .ok_or_else(|| io::Error::other("a hand-over not named after its room"))?;
    let log = data_dir.join(format!("{name}.log"));
    if fs::symlink_metadata(&log).is_ok() {
        return Err(io::Error::other("a game of that room is here already"));
    }
    if let Some(snapshots) = snapshots
        && let Ok(entries) = fs::read_dir(from.join(WORLDS))
    {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "manifest") {
                continue;
            }
            let manifest = Manifest::from_bytes(&fs::read(&path)?).map_err(io::Error::other)?;
            let world = File::open(path.with_extension("world"))?;
            let ingested = snapshots
                .store
                .ingest(world, manifest.params())
                .map_err(io::Error::other)?;
            if ingested.id() != manifest.id() {
                if let Err(error) = snapshots.store.release(&ingested.id()) {
                    warn!(%error, "cannot release a world that is not its manifest's");
                }
                return Err(io::Error::other(
                    "a handed-over world is not its manifest's",
                ));
            }
        }
    }
    fs::create_dir_all(data_dir)?;
    let pointer = from.join(POINTER);
    if snapshots.is_some() && fs::symlink_metadata(&pointer).is_ok_and(|meta| meta.is_file()) {
        move_in(&pointer, &data_dir.join(format!("{name}.snapshot")))?;
    }
    move_in(&from.join(LOG), &log)?;
    sync_dir(data_dir);
    Ok(log)
}

/// Sets a hand-over the newer server could not take in aside as `.broken`,
/// unchanged, for diagnosis.
pub(crate) fn set_aside(from: &Path) {
    let aside = from.with_extension("broken");
    if let Err(error) = fs::rename(from, &aside) {
        warn!(path = %from.display(), %error, "cannot set a hand-over aside");
    }
}

/// Copies `from` to `to` through a temporary name, flushed to disk: `to`
/// is whole or absent.
fn move_in(from: &Path, to: &Path) -> io::Result<()> {
    let partial = to.with_extension("taking-in");
    copy_synced(from, &partial)?;
    fs::rename(&partial, to)?;
    fs::remove_file(from)
}

fn copy_synced(from: &Path, to: &Path) -> io::Result<()> {
    fs::copy(from, to)?;
    flush(to)
}

fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    fs::write(path, bytes)?;
    flush(path)
}

/// Flushes a file to disk. Opened for writing: Windows flushes no file
/// opened only to read.
fn flush(path: &Path) -> io::Result<()> {
    fs::OpenOptions::new().write(true).open(path)?.sync_all()
}

/// Flushes a directory's entries, where the system lets a directory be
/// opened (not on Windows).
fn sync_dir(dir: &Path) {
    if let Ok(dir) = File::open(dir) {
        let _ = dir.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshots::SnapshotConfig;
    use tpf3mp_proto::FixedBytes;
    use tpf3mp_snapshot::ChunkParams;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tpf3mp-drain-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A world of `len` bytes no two of whose chunks are alike.
    fn world(len: usize) -> Vec<u8> {
        let mut state = 0x9e37_79b9_u32;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state.to_le_bytes()[0]
            })
            .collect()
    }

    /// An older server's game written for a newer one: its log, pointer and
    /// world; the old store's snapshot, `None` when none.
    fn handed_over(name: &str) -> (PathBuf, PathBuf, RoomId, Vec<u8>, ManifestId) {
        let base = temp(name);
        let (old_data, old_store, dir) = (
            base.join("old"),
            base.join("old-store"),
            base.join("handover"),
        );
        fs::create_dir_all(&old_data).unwrap();
        let room = RoomId(FixedBytes([5; 16]));
        let log = b"a room's log".to_vec();
        fs::write(RoomLog::path_for(&old_data, &room), &log).unwrap();
        fs::write(Pointer::path_for(&old_data, &room), b"pointer").unwrap();
        let snapshots = Snapshots::open(&SnapshotConfig::new(&old_store)).unwrap();
        let manifest = snapshots
            .store
            .ingest(io::Cursor::new(world(300_000)), ChunkParams::DEFAULT)
            .unwrap();
        let partial = export(&old_data, Some(&snapshots), &room, &[manifest.id()], &dir).unwrap();
        // Nothing for the newer server until it is published.
        assert!(ready(&dir).is_empty());
        publish(&partial, &dir, &room).unwrap();
        assert_eq!(ready(&dir), vec![handover_path(&dir, &room)]);
        (base, dir, room, log, manifest.id())
    }

    #[test]
    fn a_handed_over_game_is_taken_in_with_its_log_pointer_and_world() {
        let (base, dir, room, log, id) = handed_over("whole");
        let (new_data, new_store) = (base.join("new"), base.join("new-store"));
        let snapshots = Snapshots::open(&SnapshotConfig::new(&new_store)).unwrap();
        let from = handover_path(&dir, &room);
        let taken = take_in(&from, &new_data, Some(&snapshots)).unwrap();
        assert_eq!(taken, RoomLog::path_for(&new_data, &room));
        assert_eq!(fs::read(&taken).unwrap(), log);
        assert_eq!(
            fs::read(Pointer::path_for(&new_data, &room)).unwrap(),
            b"pointer"
        );
        // The world is the newer store's now, the snapshot its manifest names.
        let manifest = snapshots.store.manifest(&id).unwrap();
        assert_eq!(manifest.id(), id);
        // A game of that room taken in again is refused, changing nothing.
        assert!(take_in(&from, &new_data, Some(&snapshots)).is_err());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_world_that_is_not_its_manifests_is_refused_and_nothing_taken_in() {
        let (base, dir, room, _, id) = handed_over("tampered");
        let from = handover_path(&dir, &room);
        let world = from.join(WORLDS).join(format!("{id}.world"));
        let mut bytes = fs::read(&world).unwrap();
        bytes[1000] ^= 0xff;
        fs::write(&world, bytes).unwrap();
        let (new_data, new_store) = (base.join("new"), base.join("new-store"));
        let snapshots = Snapshots::open(&SnapshotConfig::new(&new_store)).unwrap();
        assert!(take_in(&from, &new_data, Some(&snapshots)).is_err());
        assert!(!RoomLog::path_for(&new_data, &room).exists());
        assert!(!Pointer::path_for(&new_data, &room).exists());
        // Set aside, unchanged, for diagnosis.
        set_aside(&from);
        assert!(from.with_extension("broken").join(LOG).exists());
        assert!(ready(&dir).is_empty());
        fs::remove_dir_all(&base).unwrap();
    }
}

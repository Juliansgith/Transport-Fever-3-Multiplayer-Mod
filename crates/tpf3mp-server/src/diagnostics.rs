//! Players' diagnostics ("Diagnostics" in PROTOCOL.md): the lines their
//! clients send, kept per session, so the operator reads what went wrong
//! for a player by the support ID the launcher shows. A thread of their own
//! writes them, away from the connections: when it falls behind, lines are
//! dropped, and no game waits. They are kept for a number of days, within a
//! total size, the oldest going first.

use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tpf3mp_proto::{DiagnosticEvent, PlayerId, SessionId, redact};
use tracing::warn;

use crate::metrics::{self, Metrics};

/// Bytes of lines one session may keep.
pub const SESSION_QUOTA: u64 = 8 << 20;
/// Batches waiting for the writer; more are dropped.
const QUEUE: usize = 256;
/// How often files past their time or the total are removed.
const PRUNE_EVERY: Duration = Duration::from_secs(3600);

/// Where diagnostics are kept, and for how long.
#[derive(Debug, Clone)]
pub struct DiagnosticsConfig {
    pub dir: PathBuf,
    pub keep_for: Duration,
    /// Bytes all sessions' files may take together.
    pub max_total: u64,
}

/// The diagnostics kept, and the queue to the thread that writes them.
pub(crate) struct Diagnostics {
    dir: PathBuf,
    queue: SyncSender<Batch>,
    metrics: Arc<Metrics>,
}

struct Batch {
    session: SessionId,
    lines: Vec<u8>,
    count: u64,
}

/// One line of a session's file.
#[derive(Serialize)]
struct Line<'a> {
    received_ms: u64,
    at_ms: u64,
    player: String,
    level: &'static str,
    target: &'a str,
    text: String,
}

/// A session's diagnostics, as the admin endpoint lists them.
#[derive(Debug, Serialize)]
pub struct Entry {
    pub session: String,
    pub bytes: u64,
    pub modified_ms: u64,
}

/// Why a batch was not kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotKept {
    /// The session sent all it may.
    Quota,
    /// The writer is behind.
    Busy,
}

impl Diagnostics {
    pub(crate) fn start(config: DiagnosticsConfig, metrics: Arc<Metrics>) -> io::Result<Self> {
        fs::create_dir_all(&config.dir)?;
        let (queue, batches) = mpsc::sync_channel(QUEUE);
        let dir = config.dir.clone();
        let writer_metrics = Arc::clone(&metrics);
        thread::Builder::new()
            .name("tpf3mp-diagnostics".into())
            .spawn(move || write_batches(&config, &batches, &writer_metrics))?;
        Ok(Self {
            dir,
            queue,
            metrics,
        })
    }

    /// Queues lines of `session`, which has already kept `kept` bytes,
    /// redacted again here, and returns how many bytes they take.
    pub(crate) fn submit(
        &self,
        session: SessionId,
        player: PlayerId,
        kept: u64,
        events: &[DiagnosticEvent],
    ) -> Result<u64, NotKept> {
        let received_ms = millis_since_epoch(SystemTime::now());
        let mut lines = Vec::new();
        for event in events {
            let line = Line {
                received_ms,
                at_ms: event.at_ms,
                player: player.to_string(),
                level: event.level.as_str(),
                target: event.target.as_str(),
                text: redact(event.text.as_str()),
            };
            // Plain strings and numbers: serialising cannot fail.
            if serde_json::to_writer(&mut lines, &line).is_ok() {
                lines.push(b'\n');
            }
        }
        let bytes = lines.len() as u64;
        let count = events.len() as u64;
        if kept.saturating_add(bytes) > SESSION_QUOTA {
            metrics::add(&self.metrics.diagnostics_dropped, count);
            return Err(NotKept::Quota);
        }
        match self.queue.try_send(Batch {
            session,
            lines,
            count,
        }) {
            Ok(()) => Ok(bytes),
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                metrics::add(&self.metrics.diagnostics_dropped, count);
                Err(NotKept::Busy)
            }
        }
    }

    /// The sessions with diagnostics, the latest first.
    pub(crate) fn list(&self) -> io::Result<Vec<Entry>> {
        let mut entries: Vec<Entry> = files(&self.dir)?
            .into_iter()
            .map(|file| Entry {
                session: file.session,
                bytes: file.bytes,
                modified_ms: millis_since_epoch(file.modified),
            })
            .collect();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.modified_ms));
        Ok(entries)
    }

    /// One session's lines, if it has any. `session` must be a session ID.
    pub(crate) fn read(&self, session: &str) -> io::Result<Option<Vec<u8>>> {
        if !is_session_id(session) {
            return Ok(None);
        }
        match fs::read(self.dir.join(format!("{session}.ndjson"))) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

fn write_batches(config: &DiagnosticsConfig, batches: &mpsc::Receiver<Batch>, metrics: &Metrics) {
    prune(config);
    let mut pruned = Instant::now();
    loop {
        match batches.recv_timeout(PRUNE_EVERY) {
            Ok(batch) => match append(config, &batch) {
                Ok(()) => metrics::add(&metrics.diagnostics_kept, batch.count),
                Err(error) => {
                    warn!(%error, session = %batch.session, "cannot keep a player's diagnostics");
                    metrics::add(&metrics.diagnostics_dropped, batch.count);
                }
            },
            Err(RecvTimeoutError::Timeout) => {}
            // The server has stopped.
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if pruned.elapsed() >= PRUNE_EVERY {
            prune(config);
            pruned = Instant::now();
        }
    }
}

fn append(config: &DiagnosticsConfig, batch: &Batch) -> io::Result<()> {
    let path = config.dir.join(format!("{}.ndjson", batch.session));
    let mut options = fs::OpenOptions::new();
    options.create(true).append(true);
    // Like the rooms' logs: the server's user alone reads them.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(&path)?.write_all(&batch.lines)
}

struct File {
    session: String,
    path: PathBuf,
    bytes: u64,
    modified: SystemTime,
}

/// The sessions' files in `dir`, and nothing else there.
fn files(dir: &std::path::Path) -> io::Result<Vec<File>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(session) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".ndjson"))
            .filter(|session| is_session_id(session))
        else {
            continue;
        };
        let metadata = entry.metadata()?;
        files.push(File {
            session: session.to_owned(),
            path: entry.path(),
            bytes: metadata.len(),
            modified: metadata.modified().unwrap_or(UNIX_EPOCH),
        });
    }
    Ok(files)
}

/// Removes the files past their time, then the oldest while all of them
/// take more than the total allowed.
fn prune(config: &DiagnosticsConfig) {
    let mut files = match files(&config.dir) {
        Ok(files) => files,
        Err(error) => {
            warn!(%error, "cannot look through the players' diagnostics");
            return;
        }
    };
    let now = SystemTime::now();
    files.retain(|file| {
        let old = now
            .duration_since(file.modified)
            .is_ok_and(|age| age > config.keep_for);
        !(old && fs::remove_file(&file.path).is_ok())
    });
    files.sort_by_key(|file| file.modified);
    let mut total: u64 = files.iter().map(|file| file.bytes).sum();
    for file in files {
        if total <= config.max_total {
            break;
        }
        if fs::remove_file(&file.path).is_ok() {
            total -= file.bytes;
        }
    }
}

/// `s-` and 32 hexadecimal digits, as `SessionId` shows itself.
fn is_session_id(text: &str) -> bool {
    text.strip_prefix("s-").is_some_and(|hex| {
        hex.len() == 32
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn millis_since_epoch(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |since| {
        u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpf3mp_proto::{DiagnosticLevel, FixedBytes, Text};

    fn event(text: &str) -> DiagnosticEvent {
        DiagnosticEvent {
            at_ms: 1,
            level: DiagnosticLevel::Warn,
            target: Text::new("tpf3mp_agent").unwrap(),
            text: Text::new(text).unwrap(),
        }
    }

    fn keeps_until(dir: &std::path::Path, want: impl Fn(&[u8]) -> bool, file: &str) -> Vec<u8> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(bytes) = fs::read(dir.join(file))
                && want(&bytes)
            {
                return bytes;
            }
            assert!(Instant::now() < deadline, "{file} was not written");
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_sessions_lines_are_kept_redacted_and_listed() {
        let dir = tempfile::tempdir().unwrap();
        let diagnostics = Diagnostics::start(
            DiagnosticsConfig {
                dir: dir.path().to_owned(),
                keep_for: Duration::from_secs(3600),
                max_total: 1 << 30,
            },
            Arc::default(),
        )
        .unwrap();
        let session = SessionId([7; 16]);
        let player = PlayerId(FixedBytes([9; 32]));
        let bytes = diagnostics
            .submit(
                session,
                player,
                0,
                &[event(r"cannot open C:\Users\Alice\x.key")],
            )
            .unwrap();
        let name = format!("{session}.ndjson");
        let written = keeps_until(dir.path(), |bytes| !bytes.is_empty(), &name);
        assert_eq!(written.len() as u64, bytes);
        let text = String::from_utf8(written).unwrap();
        assert!(
            text.contains(r#""text":"cannot open <path>/x.key""#),
            "{text}"
        );
        assert!(text.contains(r#""level":"warn""#), "{text}");
        let listed = diagnostics.list().unwrap();
        assert_eq!(listed[0].session, session.to_string());
        assert_eq!(
            diagnostics.read(&session.to_string()).unwrap().unwrap(),
            text.as_bytes()
        );
        assert_eq!(diagnostics.read("../secret").unwrap(), None);
        assert_eq!(
            diagnostics.submit(session, player, SESSION_QUOTA, &[event("more")]),
            Err(NotKept::Quota)
        );
    }

    #[test]
    fn old_files_and_the_oldest_past_the_total_go() {
        let dir = tempfile::tempdir().unwrap();
        let old = SessionId([1; 16]);
        let newer = SessionId([2; 16]);
        let newest = SessionId([3; 16]);
        for (session, age_secs) in [(old, 7200), (newer, 60), (newest, 0)] {
            let path = dir.path().join(format!("{session}.ndjson"));
            fs::write(&path, vec![b'x'; 100]).unwrap();
            let modified = SystemTime::now() - Duration::from_secs(age_secs);
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(modified)
                .unwrap();
        }
        fs::write(dir.path().join("not-ours.txt"), "keep").unwrap();
        prune(&DiagnosticsConfig {
            dir: dir.path().to_owned(),
            keep_for: Duration::from_secs(3600),
            max_total: 150,
        });
        let left: Vec<String> = files(dir.path())
            .unwrap()
            .into_iter()
            .map(|file| file.session)
            .collect();
        assert_eq!(left, [newest.to_string()]);
        assert!(dir.path().join("not-ours.txt").exists());
    }
}

//! Players' diagnostics ("Diagnostics" in PROTOCOL.md): the lines their
//! clients send, kept per session, so the operator reads what went wrong
//! for a player by the support code the launcher shows. A thread of their own
//! writes them, away from the connections: when it falls behind, lines are
//! dropped, and no game waits. They are kept for a number of days, within a
//! total size, the oldest going first.
//!
//! Each line names its source (the launcher, the agent, the hook, the game,
//! the game's error reports) and the launcher's run ([`LogSession`]), which
//! outlives a connection. A run's index, `runs/<run>`, lists the sessions
//! it sent lines in, so the operator reads a whole run by its code as well
//! as one session by its support code. A run's code and a session's never
//! collide: the server gives no session a code a run has.
//!
//! Every line, and every entry of a run's index, also names the player by
//! their ID and by the name their launcher gave in its `Hello`, the name
//! the lobby shows, so the operator finds a player's sessions by name.
//! Names are not unique and can change; the player ID is the stable link.
//! The client sends nothing more for it.

use std::{
    collections::HashSet,
    fs,
    io::{self, BufRead, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tpf3mp_proto::{
    CODE_LEN, Code, DiagnosticEvent, LogSession, LogSource, PlayerId, SessionId, TelemetryLine,
    redact,
};
use tracing::warn;

use crate::metrics::{self, Metrics};

/// Bytes of lines one session may keep, unless the operator chose
/// otherwise (`--diagnostics-session-mib`): room for the hook's and the
/// game's logs of a long evening.
pub const SESSION_QUOTA: u64 = 64 << 20;
/// The longest player name kept: a launcher's names are 32 bytes.
const NAME_MAX: usize = 64;
/// Batches waiting for the writer; more are dropped.
const QUEUE: usize = 256;
/// How often files past their time or the total are removed.
const PRUNE_EVERY: Duration = Duration::from_secs(3600);
/// The most of one session's or run's lines the admin endpoint gives at
/// once: the newest.
pub const READ_LIMIT: usize = 64 << 20;
/// The folder runs' indexes are kept in, inside the diagnostics folder.
const RUNS: &str = "runs";

/// Where diagnostics are kept, and for how long.
#[derive(Debug, Clone)]
pub struct DiagnosticsConfig {
    pub dir: PathBuf,
    pub keep_for: Duration,
    /// Bytes all sessions' files may take together.
    pub max_total: u64,
    /// Bytes one session may keep.
    pub session_quota: u64,
}

impl DiagnosticsConfig {
    /// Keeps lines in `dir` for `keep_for`, within `max_total` bytes, with
    /// the default quota per session.
    pub fn new(dir: PathBuf, keep_for: Duration, max_total: u64) -> Self {
        Self {
            dir,
            keep_for,
            max_total,
            session_quota: SESSION_QUOTA,
        }
    }
}

/// The diagnostics kept, and the queue to the thread that writes them.
pub(crate) struct Diagnostics {
    dir: PathBuf,
    session_quota: u64,
    queue: SyncSender<Batch>,
    metrics: Arc<Metrics>,
}

/// Who sent a session's lines: the player's ID and the name their
/// launcher gave in its `Hello`, as the lobby shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Who {
    pub(crate) player: PlayerId,
    pub(crate) name: String,
}

impl Who {
    /// `name` redacted like a line, on one line, and cut to [`NAME_MAX`].
    pub(crate) fn new(player: PlayerId, name: &str) -> Self {
        let mut name = redact(name).trim().to_owned();
        if name.len() > NAME_MAX {
            let mut end = NAME_MAX;
            while !name.is_char_boundary(end) {
                end -= 1;
            }
            name.truncate(end);
        }
        Self { player, name }
    }
}

struct Batch {
    session: SessionId,
    /// The run to index the session under, the first time it sends lines
    /// of that run, and the index's line for it.
    index: Option<(LogSession, String)>,
    lines: Vec<u8>,
    count: u64,
}

/// One line of a session's file.
#[derive(Serialize)]
struct Line<'a> {
    received_ms: u64,
    at_ms: u64,
    player: String,
    /// The player's name as their launcher gave it.
    name: &'a str,
    /// The launcher's run, when the client said it.
    #[serde(skip_serializing_if = "Option::is_none")]
    run: Option<String>,
    /// Where the line came from, when the client said it.
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'static str>,
    level: &'static str,
    target: &'a str,
    text: String,
}

/// A session's diagnostics, as the admin endpoint lists them.
#[derive(Debug, Serialize)]
pub struct Entry {
    pub session: String,
    /// The player, by ID, as the session's first line names them.
    pub player: Option<String>,
    /// Their name then: not unique, and it can change.
    pub name: Option<String>,
    pub bytes: u64,
    pub modified_ms: u64,
}

/// Who a session or a run is, as `diagnostics <code>` heads its lines.
#[derive(Debug, Serialize)]
pub struct Summary {
    pub code: String,
    /// `session` or `run`.
    pub kind: &'static str,
    pub sessions: Vec<SessionOf>,
}

/// One session, by its support code, and its player.
#[derive(Debug, Serialize)]
pub struct SessionOf {
    pub session: String,
    pub player: Option<String>,
    pub name: Option<String>,
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
        let session_quota = config.session_quota;
        let writer_metrics = Arc::clone(&metrics);
        thread::Builder::new()
            .name("tpf3mp-diagnostics".into())
            .spawn(move || write_batches(&config, &batches, &writer_metrics))?;
        Ok(Self {
            dir,
            session_quota,
            queue,
            metrics,
        })
    }

    /// Queues lines of `session`, which has already kept `kept` bytes,
    /// redacted again here, and returns how many bytes they take.
    pub(crate) fn submit(
        &self,
        session: SessionId,
        who: &Who,
        kept: u64,
        events: &[DiagnosticEvent],
    ) -> Result<u64, NotKept> {
        let received_ms = millis_since_epoch(SystemTime::now());
        let lines = events.iter().map(|event| Line {
            received_ms,
            at_ms: event.at_ms,
            player: who.player.to_string(),
            name: &who.name,
            run: None,
            source: None,
            level: event.level.as_str(),
            target: event.target.as_str(),
            text: redact(event.text.as_str()),
        });
        self.queue(session, None, kept, lines)
    }

    /// Queues lines of the launcher's `run` sent in `session`, which has
    /// already kept `kept` bytes, redacted again here, and returns how many
    /// bytes they take. `first`: the session's first lines of that run,
    /// which index it under the run.
    pub(crate) fn submit_telemetry(
        &self,
        session: SessionId,
        who: &Who,
        run: LogSession,
        first: bool,
        kept: u64,
        lines: &[TelemetryLine],
    ) -> Result<u64, NotKept> {
        let received_ms = millis_since_epoch(SystemTime::now());
        let run_name = run.to_string();
        let lines = lines.iter().map(|line| Line {
            received_ms,
            at_ms: line.at_ms,
            player: who.player.to_string(),
            name: &who.name,
            run: Some(run_name.clone()),
            source: Some(line.source.as_str()),
            level: line.level.as_str(),
            target: line.target.as_str(),
            text: redact(line.text.as_str()),
        });
        let index = first.then(|| (run, format!("{session}\t{}\t{}\n", who.player, who.name)));
        self.queue(session, index, kept, lines)
    }

    fn queue<'a>(
        &self,
        session: SessionId,
        index: Option<(LogSession, String)>,
        kept: u64,
        lines: impl Iterator<Item = Line<'a>>,
    ) -> Result<u64, NotKept> {
        let mut bytes = Vec::new();
        let mut count = 0;
        for line in lines {
            // Plain strings and numbers: serialising cannot fail.
            if serde_json::to_writer(&mut bytes, &line).is_ok() {
                bytes.push(b'\n');
            }
            count += 1;
        }
        let size = bytes.len() as u64;
        if kept.saturating_add(size) > self.session_quota {
            metrics::add(&self.metrics.diagnostics_dropped, count);
            return Err(NotKept::Quota);
        }
        match self.queue.try_send(Batch {
            session,
            index,
            lines: bytes,
            count,
        }) {
            Ok(()) => Ok(size),
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
            .map(|file| {
                let (player, name) = first_who(&file.path);
                Entry {
                    session: file.session,
                    player,
                    name,
                    bytes: file.bytes,
                    modified_ms: millis_since_epoch(file.modified),
                }
            })
            .collect();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.modified_ms));
        Ok(entries)
    }

    /// Who the session or run `code` is: a session's player, or each of a
    /// run's sessions and theirs, from its index. `None` when there is no
    /// such session or run.
    pub(crate) fn summary(&self, code: &str) -> io::Result<Option<Summary>> {
        if !is_session_id(code) {
            return Ok(None);
        }
        let session = self.dir.join(format!("{code}.ndjson"));
        if session.exists() {
            let (player, name) = first_who(&session);
            return Ok(Some(Summary {
                code: code.to_owned(),
                kind: "session",
                sessions: vec![SessionOf {
                    session: code.to_owned(),
                    player,
                    name,
                }],
            }));
        }
        if !is_code(code) {
            return Ok(None);
        }
        let index = match fs::read_to_string(self.dir.join(RUNS).join(code)) {
            Ok(index) => index,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut sessions: Vec<SessionOf> = Vec::new();
        for line in index.lines() {
            let mut fields = line.splitn(3, '\t');
            let Some(session) = fields.next().map(str::trim).filter(|s| is_code(s)) else {
                continue;
            };
            if sessions.iter().any(|known| known.session == session) {
                continue;
            }
            let given = |field: Option<&str>| field.map(str::to_owned).filter(|f| !f.is_empty());
            sessions.push(SessionOf {
                session: session.to_owned(),
                player: given(fields.next()),
                name: given(fields.next()),
            });
        }
        Ok(Some(Summary {
            code: code.to_owned(),
            kind: "run",
            sessions,
        }))
    }

    /// Whether lines of `session` are kept, or a run of that code has
    /// some, so its ID is not given again.
    pub(crate) fn has(&self, session: &SessionId) -> bool {
        self.dir.join(format!("{session}.ndjson")).exists()
            || self.dir.join(RUNS).join(session.to_string()).exists()
    }

    /// The lines of the session or run `code`, if it has any, of `source`
    /// alone when one is given: a session's by its support code, or a
    /// launcher run's from every session it sent lines in. At most
    /// [`READ_LIMIT`] bytes of them, the newest. `code` must be a session
    /// ID or a run's code.
    pub(crate) fn read(
        &self,
        code: &str,
        source: Option<LogSource>,
    ) -> io::Result<Option<Vec<u8>>> {
        if !is_session_id(code) {
            return Ok(None);
        }
        let session = self.dir.join(format!("{code}.ndjson"));
        let (paths, run) = if session.exists() {
            (vec![session], None)
        } else if is_code(code) {
            match fs::read_to_string(self.dir.join(RUNS).join(code)) {
                Ok(index) => {
                    let mut sessions: Vec<&str> = Vec::new();
                    for listed in index.lines().map(index_session) {
                        if is_code(listed) && !sessions.contains(&listed) {
                            sessions.push(listed);
                        }
                    }
                    let paths = sessions
                        .iter()
                        .map(|session| self.dir.join(format!("{session}.ndjson")))
                        .collect();
                    (paths, Some(code))
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            }
        } else {
            return Ok(None);
        };
        let mut found = false;
        let mut out = Vec::new();
        for path in paths {
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            found = true;
            if source.is_none() && run.is_none() {
                out.extend_from_slice(&bytes);
                continue;
            }
            for line in bytes.split_inclusive(|byte| *byte == b'\n') {
                let (line_run, line_source) = line_tags(line);
                let wanted = source.is_none_or(|source| line_source == Some(source))
                    && run.is_none_or(|run| line_run.as_deref() == Some(run));
                if wanted {
                    out.extend_from_slice(line);
                }
            }
        }
        if out.len() > READ_LIMIT {
            // The newest, from the start of a line.
            let cut = out.len() - READ_LIMIT;
            let start = out[cut..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(out.len(), |at| cut + at + 1);
            out.drain(..start);
        }
        Ok(found.then_some(out))
    }
}

/// The session an entry of a run's index names: its first field.
fn index_session(line: &str) -> &str {
    line.split('\t').next().unwrap_or_default().trim()
}

/// The player and name the first line of a session's file names, if it
/// names them.
fn first_who(path: &Path) -> (Option<String>, Option<String>) {
    #[derive(serde::Deserialize)]
    struct Who {
        player: Option<String>,
        name: Option<String>,
    }
    let mut line = String::new();
    let read = fs::File::open(path)
        .map(|file| io::BufReader::new(file.take(16 << 10)).read_line(&mut line));
    if !matches!(read, Ok(Ok(_))) {
        return (None, None);
    }
    serde_json::from_str::<Who>(&line).map_or((None, None), |who| (who.player, who.name))
}

/// The run and source a kept line names; lines kept before sources were
/// sent are the launcher's.
fn line_tags(line: &[u8]) -> (Option<String>, Option<LogSource>) {
    #[derive(serde::Deserialize)]
    struct Tags {
        run: Option<String>,
        source: Option<String>,
    }
    match serde_json::from_slice::<Tags>(line) {
        Ok(tags) => (
            tags.run,
            match tags.source {
                Some(name) => LogSource::from_name(&name),
                None => Some(LogSource::Launcher),
            },
        ),
        Err(_) => (None, None),
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
    if let Some((run, line)) = &batch.index {
        let runs = config.dir.join(RUNS);
        fs::create_dir_all(&runs)?;
        open_append(&runs.join(run.to_string()))?.write_all(line.as_bytes())?;
    }
    open_append(&config.dir.join(format!("{}.ndjson", batch.session)))?.write_all(&batch.lines)
}

fn open_append(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.create(true).append(true);
    // Like the rooms' logs: the server's user alone reads them.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

struct File {
    session: String,
    path: PathBuf,
    bytes: u64,
    modified: SystemTime,
}

/// The sessions' files in `dir`, and nothing else there.
fn files(dir: &Path) -> io::Result<Vec<File>> {
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
/// take more than the total allowed; then the runs' indexes whose sessions
/// are all gone.
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
    let mut kept: HashSet<String> = files.iter().map(|file| file.session.clone()).collect();
    for file in files {
        if total <= config.max_total {
            break;
        }
        if fs::remove_file(&file.path).is_ok() {
            total -= file.bytes;
            kept.remove(&file.session);
        }
    }
    let Ok(runs) = fs::read_dir(config.dir.join(RUNS)) else {
        return;
    };
    for run in runs.flatten() {
        let path = run.path();
        let Some(code) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !is_code(code) {
            continue;
        }
        let alive = fs::read_to_string(&path)
            .is_ok_and(|index| index.lines().any(|line| kept.contains(index_session(line))));
        if !alive {
            let _ = fs::remove_file(&path);
        }
    }
}

/// A code as it shows itself, such as `K7QM2X`.
fn is_code(text: &str) -> bool {
    text.len() == CODE_LEN && text.parse::<Code>().is_ok_and(|code| code.as_str() == text)
}

/// A session ID as it shows itself, such as `K7QM2X`; or `s-` and 32
/// hexadecimal digits, as it did before, whose files still go when their
/// time is up.
fn is_session_id(text: &str) -> bool {
    let earlier = text.strip_prefix("s-").is_some_and(|hex| {
        hex.len() == 32
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    });
    is_code(text) || earlier
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
            DiagnosticsConfig::new(dir.path().to_owned(), Duration::from_secs(3600), 1 << 30),
            Arc::default(),
        )
        .unwrap();
        let session = SessionId("K7QM2X".parse().unwrap());
        let player = PlayerId(FixedBytes([9; 32]));
        let bytes = diagnostics
            .submit(
                session,
                &Who::new(player, "Ann"),
                0,
                &[event(r"cannot open C:\Users\Alice\x.key")],
            )
            .unwrap();
        let name = format!("{session}.ndjson");
        let written = keeps_until(dir.path(), |bytes| !bytes.is_empty(), &name);
        assert_eq!(written.len() as u64, bytes);
        assert!(diagnostics.has(&session));
        assert!(!diagnostics.has(&SessionId("AB2CD3".parse().unwrap())));
        let text = String::from_utf8(written).unwrap();
        assert!(
            text.contains(r#""text":"cannot open <path>/x.key""#),
            "{text}"
        );
        assert!(text.contains(r#""level":"warn""#), "{text}");
        let listed = diagnostics.list().unwrap();
        assert_eq!(listed[0].session, session.to_string());
        assert_eq!(listed[0].player, Some(player.to_string()));
        assert_eq!(listed[0].name.as_deref(), Some("Ann"));
        assert_eq!(
            diagnostics
                .read(&session.to_string(), None)
                .unwrap()
                .unwrap(),
            text.as_bytes()
        );
        assert_eq!(diagnostics.read("../secret", None).unwrap(), None);
        assert_eq!(
            diagnostics.read("k7qm2x", None).unwrap(),
            None,
            "as it shows itself"
        );
        assert_eq!(
            diagnostics.submit(
                session,
                &Who::new(player, "Ann"),
                SESSION_QUOTA,
                &[event("more")]
            ),
            Err(NotKept::Quota)
        );
    }

    #[test]
    fn old_files_and_the_oldest_past_the_total_go() {
        let dir = tempfile::tempdir().unwrap();
        // The old one's ID is of the form session IDs had before.
        let old = "s-11111111111111111111111111111111";
        let older = "AB2CD3";
        let newer = "EF4GH5";
        let newest = "JK6MN7";
        for (session, age_secs) in [(old, 7300), (older, 7200), (newer, 60), (newest, 0)] {
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
        prune(&DiagnosticsConfig::new(
            dir.path().to_owned(),
            Duration::from_secs(3600),
            150,
        ));
        let left: Vec<String> = files(dir.path())
            .unwrap()
            .into_iter()
            .map(|file| file.session)
            .collect();
        assert_eq!(left, [newest]);
        assert!(dir.path().join("not-ours.txt").exists());
    }

    fn line(source: LogSource, text: &str) -> TelemetryLine {
        TelemetryLine {
            at_ms: 1,
            level: DiagnosticLevel::Info,
            source,
            target: Text::new("hook.log").unwrap(),
            text: Text::new(text).unwrap(),
        }
    }

    /// Every source's lines are kept under the session, each with the run
    /// and its source, redacted again; the operator reads them by the
    /// support code or by the run's code, one source at a time if they
    /// like, and no session is given the run's code.
    #[test]
    fn a_runs_lines_are_read_by_session_by_run_and_by_source() {
        let dir = tempfile::tempdir().unwrap();
        let diagnostics = Diagnostics::start(
            DiagnosticsConfig::new(dir.path().to_owned(), Duration::from_secs(3600), 1 << 30),
            Arc::default(),
        )
        .unwrap();
        let run = LogSession("AB2CD3".parse().unwrap());
        let player = PlayerId(FixedBytes([7; 32]));
        let first = SessionId("K7QM2X".parse().unwrap());
        let second = SessionId("EF4GH5".parse().unwrap());
        diagnostics
            .submit_telemetry(
                first,
                &Who::new(player, "Ann"),
                run,
                true,
                0,
                &[
                    line(
                        LogSource::Hook,
                        r"loaded D:\Games\Steam\userdata\4242\x.sav",
                    ),
                    line(LogSource::Crash, r#""userId": "4242","#),
                ],
            )
            .unwrap();
        diagnostics
            .submit_telemetry(
                second,
                &Who::new(player, "Ann\tthe\nsecond ann@example.org"),
                run,
                true,
                0,
                &[line(LogSource::Launcher, "connected again")],
            )
            .unwrap();
        keeps_until(dir.path(), |b| !b.is_empty(), "EF4GH5.ndjson");
        let read = |code: &str, source| {
            String::from_utf8(diagnostics.read(code, source).unwrap().unwrap()).unwrap()
        };
        let session = read("K7QM2X", None);
        assert_eq!(session.lines().count(), 2, "{session}");
        assert!(session.contains(r#""run":"AB2CD3""#), "{session}");
        assert!(!session.contains("4242"), "{session}");
        let whole = read("AB2CD3", None);
        assert_eq!(whole.lines().count(), 3, "both sessions: {whole}");
        let hook = read("AB2CD3", Some(LogSource::Hook));
        assert_eq!(hook.lines().count(), 1, "{hook}");
        assert!(hook.contains(r#""source":"hook""#), "{hook}");
        assert!(hook.contains("<path>/x.sav"), "{hook}");
        assert_eq!(read("EF4GH5", Some(LogSource::Hook)), "");
        // Who the run is: each session and its player, from the index,
        // the name on one line and redacted.
        let summary = diagnostics.summary("AB2CD3").unwrap().unwrap();
        assert_eq!(summary.kind, "run");
        let names: Vec<(String, Option<String>, Option<String>)> = summary
            .sessions
            .into_iter()
            .map(|of| (of.session, of.player, of.name))
            .collect();
        assert_eq!(
            names,
            [
                (
                    "K7QM2X".into(),
                    Some(player.to_string()),
                    Some("Ann".into())
                ),
                (
                    "EF4GH5".into(),
                    Some(player.to_string()),
                    Some("Ann the second <email>".into())
                ),
            ]
        );
        let session_summary = diagnostics.summary("K7QM2X").unwrap().unwrap();
        assert_eq!(session_summary.kind, "session");
        assert_eq!(session_summary.sessions[0].name.as_deref(), Some("Ann"));
        assert!(diagnostics.summary("JK6MN7").unwrap().is_none());
        // The run's code is taken: no session gets it.
        assert!(diagnostics.has(&SessionId("AB2CD3".parse().unwrap())));
        assert_eq!(diagnostics.read("JK6MN7", None).unwrap(), None);
    }

    #[test]
    fn a_runs_index_goes_with_its_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join(RUNS);
        fs::create_dir_all(&runs).unwrap();
        fs::write(runs.join("AB2CD3"), "K7QM2X\tp-07\tAnn\n").unwrap();
        fs::write(runs.join("EF4GH5"), "JK6MN7\tp-07\tAnn\n").unwrap();
        fs::write(dir.path().join("JK6MN7.ndjson"), "x\n").unwrap();
        prune(&DiagnosticsConfig::new(
            dir.path().to_owned(),
            Duration::from_secs(3600),
            1 << 30,
        ));
        assert!(!runs.join("AB2CD3").exists(), "its session is gone");
        assert!(runs.join("EF4GH5").exists());
    }
}

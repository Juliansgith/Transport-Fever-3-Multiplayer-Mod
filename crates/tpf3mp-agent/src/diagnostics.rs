//! Diagnostics ("Diagnostics" in PROTOCOL.md): the lines of this player's
//! logs that go to the server they play on, so its operator can see what
//! went wrong from the support code alone, without asking for files: the
//! launcher's own log, and under the proposed D10 amendment the hook's and
//! the game's logs and the game's error reports ([`crate::game_logs`]).
//!
//! A [`Recorder`] takes every line, redacts it, and tags it with its
//! [`LogSource`]; every line goes under the launcher's run
//! ([`LogSession`]). A connection made with a recorder sends its lines
//! every few seconds, and as it closes, and what is left when a connection
//! drops goes with the next. The server redacts the lines again.

use std::{
    collections::VecDeque,
    fmt,
    future::Future,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::time::MissedTickBehavior;
use tpf3mp_proto::{
    DiagnosticLevel, LogSession, LogSource, MAX_DIAGNOSTIC_EVENTS, Request, RequestError,
    Telemetry, TelemetryLine, TelemetryLines, Text, redact,
};

use crate::{ClientError, Requests};

/// Lines kept while none can be sent; past it, the oldest go.
pub const KEPT: usize = 10_000;
/// Bytes of lines one launcher run sends at most: past it, none more.
pub const RUN_QUOTA: u64 = 256 << 20;
/// How often a connection sends what was recorded.
pub const UPLOAD_EVERY: Duration = Duration::from_secs(4);
/// Batches sent at most each time: within the server's budget of two a
/// second, with a burst of sixteen.
const BATCHES_EACH_TIME: usize = 8;

/// The lines of a launcher's run waiting to go to the server played on.
/// Clones share them.
#[derive(Clone)]
pub struct Recorder(Arc<Inner>);

struct Inner {
    run: LogSession,
    off: AtomicBool,
    queue: Queue,
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Recorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Recorder")
            .field("run", &self.0.run)
            .field("on", &self.is_on())
            .field("kept", &self.len())
            .finish()
    }
}

impl Recorder {
    /// A recorder for a new run of the launcher, with a new [`LogSession`].
    pub fn new() -> Self {
        Self::for_run(LogSession::random())
    }

    /// A recorder whose lines all carry `run`.
    pub fn for_run(run: LogSession) -> Self {
        Self(Arc::new(Inner {
            run,
            off: AtomicBool::new(false),
            queue: Queue::new(KEPT, RUN_QUOTA),
        }))
    }

    /// The launcher's run, which every line carries: shown to the player
    /// next to the support code.
    pub fn run(&self) -> LogSession {
        self.0.run
    }

    /// Records a line of the launcher's own log, redacted, unless
    /// recording is off. Its source follows from where it was logged: the
    /// launcher window's, or the agent's.
    pub fn record(&self, level: DiagnosticLevel, target: &str, text: &str) {
        let source = if target.starts_with("tpf3mp_launcher") {
            LogSource::Launcher
        } else {
            LogSource::Agent
        };
        self.record_line(source, level, target, text, now_ms());
    }

    /// Records a line of `source`, redacted, unless recording is off.
    pub fn record_line(
        &self,
        source: LogSource,
        level: DiagnosticLevel,
        target: &str,
        text: &str,
        at_ms: u64,
    ) {
        if !self.is_on() {
            return;
        }
        let line = TelemetryLine {
            at_ms,
            level,
            source,
            target: Text::lossy(target),
            text: Text::lossy(&redact(text)),
        };
        self.0.queue.push(line);
    }

    /// Turns recording, and with it sending, on or off: every source
    /// alike. Off forgets what was kept.
    pub fn set_on(&self, on: bool) {
        self.0.off.store(!on, Ordering::Relaxed);
        if !on {
            self.0.queue.clear();
        }
    }

    pub fn is_on(&self) -> bool {
        !self.0.off.load(Ordering::Relaxed)
    }

    /// Lines kept, not yet sent.
    pub fn len(&self) -> usize {
        self.0.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The lines waiting, oldest first, left where they are.
    pub fn waiting(&self) -> Vec<TelemetryLine> {
        self.0.queue.waiting()
    }

    /// Lines lost: too many waited, or the run sent all it may.
    pub fn dropped(&self) -> u64 {
        self.0.queue.dropped()
    }
}

/// Lines waiting to be sent, within a number of lines and a run's bytes.
/// Clones share them.
#[derive(Clone)]
struct Queue(Arc<QueueInner>);

struct QueueInner {
    lines: Mutex<VecDeque<TelemetryLine>>,
    /// Lines kept at most; past it, the oldest go.
    kept: usize,
    /// Bytes of lines this queue takes in all; past it, none.
    quota: u64,
    taken: AtomicU64,
    dropped: AtomicU64,
}

impl fmt::Debug for Queue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Queue")
            .field("kept", &self.len())
            .field("dropped", &self.dropped())
            .finish()
    }
}

impl Queue {
    fn new(kept: usize, quota: u64) -> Self {
        Self(Arc::new(QueueInner {
            lines: Mutex::new(VecDeque::new()),
            kept,
            quota,
            taken: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        }))
    }

    fn push(&self, line: TelemetryLine) {
        let size = line.text.as_str().len() as u64 + line.target.as_str().len() as u64;
        let taken = self.0.taken.fetch_add(size, Ordering::Relaxed);
        if taken.saturating_add(size) > self.0.quota {
            self.0.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let mut lines = self.lines();
        if lines.len() >= self.0.kept {
            lines.pop_front();
            self.0.dropped.fetch_add(1, Ordering::Relaxed);
        }
        lines.push_back(line);
    }

    /// Lines kept, not yet sent.
    fn len(&self) -> usize {
        self.lines().len()
    }

    /// Lines lost: too many waited, or the run handed over all it may.
    fn dropped(&self) -> u64 {
        self.0.dropped.load(Ordering::Relaxed)
    }

    /// The lines waiting, oldest first, left where they are.
    fn waiting(&self) -> Vec<TelemetryLine> {
        self.lines().iter().cloned().collect()
    }

    fn clear(&self) {
        self.lines().clear();
    }

    /// The oldest lines, a batch at most, taken out.
    fn take(&self) -> Vec<TelemetryLine> {
        let mut lines = self.lines();
        let count = lines.len().min(MAX_DIAGNOSTIC_EVENTS);
        lines.drain(..count).collect()
    }

    /// Lines that could not be sent go back in front, for the next try.
    fn put_back(&self, events: Vec<TelemetryLine>) {
        let mut lines = self.lines();
        for event in events.into_iter().rev() {
            if lines.len() >= self.0.kept {
                self.0.dropped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            lines.push_front(event);
        }
    }

    fn lines(&self) -> std::sync::MutexGuard<'_, VecDeque<TelemetryLine>> {
        self.0.lines.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Sends what `recorder` holds over a connection's `requests` every
/// [`UPLOAD_EVERY`], until `closed` completes, the connection is gone, or
/// the server keeps no more of this session's.
pub(crate) async fn upload(
    recorder: Recorder,
    requests: Requests,
    closed: impl Future<Output = ()>,
) {
    tokio::pin!(closed);
    let mut every = tokio::time::interval(UPLOAD_EVERY);
    every.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = &mut closed => return,
            _ = every.tick() => {}
        }
        for _ in 0..BATCHES_EACH_TIME {
            match send(&recorder, &requests).await {
                Sent::Batch => {}
                Sent::Nothing | Sent::Later => break,
                Sent::Stop => return,
            }
        }
    }
}

/// What sending one batch came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sent {
    Batch,
    Nothing,
    /// Not now: the lines wait for the next time.
    Later,
    /// Not on this connection: gone, or the server keeps no more.
    Stop,
}

/// Sends one batch, if there are lines to send.
pub(crate) async fn send(recorder: &Recorder, requests: &Requests) -> Sent {
    let queue = &recorder.0.queue;
    let lines = queue.take();
    if lines.is_empty() {
        return Sent::Nothing;
    }
    let Ok(batch) = TelemetryLines::new(lines.clone()) else {
        // `take` takes a batch at most.
        return Sent::Nothing;
    };
    let request = Request::Telemetry(Telemetry {
        run: recorder.run(),
        lines: batch,
    });
    match requests.done(request).await {
        Ok(()) => Sent::Batch,
        // This session may send no more; the lines are not kept anywhere.
        Err(ClientError::Refused(RequestError::DiagnosticsNotKept)) => Sent::Stop,
        Err(ClientError::Disconnected) => {
            if recorder.is_on() {
                queue.put_back(lines);
            }
            Sent::Stop
        }
        Err(_) => {
            if recorder.is_on() {
                queue.put_back(lines);
            }
            Sent::Later
        }
    }
}

/// Milliseconds since the Unix epoch, now.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_redacted_bounded_and_forgotten_when_off() {
        let recorder = Recorder::new();
        let queue = recorder.0.queue.clone();
        recorder.record(
            DiagnosticLevel::Warn,
            "tpf3mp_agent",
            r"cannot read C:\Users\Alice\launcher.json",
        );
        let taken = queue.take();
        assert_eq!(taken[0].text.as_str(), "cannot read <path>/launcher.json");
        assert_eq!(taken[0].source, LogSource::Agent);

        for line in 0..KEPT + 5 {
            recorder.record(DiagnosticLevel::Info, "tpf3mp", &format!("line {line}"));
        }
        assert_eq!(recorder.len(), KEPT);
        assert_eq!(recorder.dropped(), 5);
        let batch = queue.take();
        assert_eq!(batch.len(), MAX_DIAGNOSTIC_EVENTS);
        assert_eq!(batch[0].text.as_str(), "line 5", "the oldest go first");
        queue.put_back(batch);
        assert_eq!(queue.take()[0].text.as_str(), "line 5", "put back in front");

        recorder.set_on(false);
        assert!(recorder.is_empty());
        recorder.record(DiagnosticLevel::Error, "tpf3mp", "not kept");
        assert!(recorder.is_empty());
        recorder.set_on(true);
        recorder.record(DiagnosticLevel::Error, "tpf3mp", "kept");
        assert_eq!(recorder.len(), 1);
    }

    #[test]
    fn control_characters_and_long_lines_fit_the_protocol() {
        let recorder = Recorder::new();
        recorder.record(
            DiagnosticLevel::Info,
            "tpf3mp",
            &format!("a\nb\t{}", "c".repeat(5000)),
        );
        let event = &recorder.0.queue.clone().take()[0];
        assert!(event.text.as_str().starts_with("a b "));
        assert!(event.text.as_str().len() <= 1024);
    }

    /// Every source's lines wait together, each tagged with its source.
    #[test]
    fn every_sources_lines_are_tagged() {
        let recorder = Recorder::new();
        recorder.record(DiagnosticLevel::Info, "tpf3mp_launcher::app", "window");
        recorder.record(DiagnosticLevel::Info, "tpf3mp_agent::bridge", "agent");
        for source in [LogSource::Hook, LogSource::Game, LogSource::Crash] {
            recorder.record_line(
                source,
                DiagnosticLevel::Info,
                "x.log",
                &format!("from {}", source.as_str()),
                7,
            );
        }
        let sources: Vec<LogSource> = recorder
            .waiting()
            .into_iter()
            .map(|line| line.source)
            .collect();
        assert_eq!(sources, LogSource::ALL);
    }

    /// Off stops every source, the game's too, and forgets what waited.
    #[test]
    fn off_stops_every_source() {
        let recorder = Recorder::new();
        recorder.record_line(LogSource::Hook, DiagnosticLevel::Info, "hook.log", "a", 1);
        assert_eq!(recorder.len(), 1);
        recorder.set_on(false);
        assert!(recorder.is_empty(), "what waited is forgotten");
        recorder.record_line(LogSource::Game, DiagnosticLevel::Info, "stdout.txt", "b", 1);
        recorder.record(DiagnosticLevel::Warn, "tpf3mp_agent", "c");
        assert!(recorder.is_empty());
    }

    /// A run sends so many bytes, then none.
    #[test]
    fn a_run_stops_at_its_quota() {
        let queue = Queue::new(KEPT, 100);
        let line = |text: &str| TelemetryLine {
            at_ms: 1,
            level: DiagnosticLevel::Info,
            source: LogSource::Hook,
            target: Text::lossy("hook.log"),
            text: Text::lossy(text),
        };
        for _ in 0..5 {
            queue.push(line(&"x".repeat(10)));
        }
        assert_eq!(queue.len(), 5);
        queue.push(line(&"y".repeat(100)));
        assert_eq!(queue.len(), 5, "over the quota");
        assert_eq!(queue.dropped(), 1);
    }
}

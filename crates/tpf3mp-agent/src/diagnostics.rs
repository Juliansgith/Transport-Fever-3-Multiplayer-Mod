//! Diagnostics ("Diagnostics" in PROTOCOL.md): the lines of this player's
//! log that go to the server they play on, so its operator can see what
//! went wrong from the support ID alone. A [`Recorder`] keeps the lines not
//! yet sent; a connection made with one sends them every few seconds, and
//! as it closes, and what is left when a connection drops goes with the
//! next. Lines are redacted as they are recorded, and again by the server.

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
    DiagnosticBatch, DiagnosticEvent, DiagnosticLevel, MAX_DIAGNOSTIC_EVENTS, Request,
    RequestError, Text, redact,
};

use crate::{ClientError, Requests};

/// Lines kept while none can be sent; past it, the oldest go.
pub const KEPT: usize = 2000;
/// How often a connection sends what was recorded.
pub const UPLOAD_EVERY: Duration = Duration::from_secs(5);
/// Batches sent at most each time: within the server's budget of one a
/// second, with a burst of eight.
const BATCHES_EACH_TIME: usize = 4;

/// The lines recorded and not yet sent. Clones share them.
#[derive(Clone, Default)]
pub struct Recorder(Arc<Inner>);

#[derive(Default)]
struct Inner {
    lines: Mutex<VecDeque<DiagnosticEvent>>,
    off: AtomicBool,
    dropped: AtomicU64,
}

impl fmt::Debug for Recorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Recorder")
            .field("on", &self.is_on())
            .field("kept", &self.len())
            .finish()
    }
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a line, redacted, unless recording is off.
    pub fn record(&self, level: DiagnosticLevel, target: &str, text: &str) {
        if !self.is_on() {
            return;
        }
        let event = DiagnosticEvent {
            at_ms: now_ms(),
            level,
            target: Text::lossy(target),
            text: Text::lossy(&redact(text)),
        };
        let mut lines = self.lines();
        if lines.len() >= KEPT {
            lines.pop_front();
            self.0.dropped.fetch_add(1, Ordering::Relaxed);
        }
        lines.push_back(event);
    }

    /// Turns recording, and with it sending, on or off. Off forgets what
    /// was kept.
    pub fn set_on(&self, on: bool) {
        self.0.off.store(!on, Ordering::Relaxed);
        if !on {
            self.lines().clear();
        }
    }

    pub fn is_on(&self) -> bool {
        !self.0.off.load(Ordering::Relaxed)
    }

    /// Lines kept, not yet sent.
    pub fn len(&self) -> usize {
        self.lines().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The lines waiting to be sent, oldest first, left where they are.
    pub fn waiting(&self) -> Vec<DiagnosticEvent> {
        self.lines().iter().cloned().collect()
    }

    /// Lines lost because too many waited.
    pub fn dropped(&self) -> u64 {
        self.0.dropped.load(Ordering::Relaxed)
    }

    /// The oldest lines, a batch at most, taken out.
    fn take(&self) -> Vec<DiagnosticEvent> {
        let mut lines = self.lines();
        let count = lines.len().min(MAX_DIAGNOSTIC_EVENTS);
        lines.drain(..count).collect()
    }

    /// Lines that could not be sent go back in front, for the next try.
    fn put_back(&self, events: Vec<DiagnosticEvent>) {
        if !self.is_on() {
            return;
        }
        let mut lines = self.lines();
        for event in events.into_iter().rev() {
            if lines.len() >= KEPT {
                self.0.dropped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            lines.push_front(event);
        }
    }

    fn lines(&self) -> std::sync::MutexGuard<'_, VecDeque<DiagnosticEvent>> {
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
    let events = recorder.take();
    if events.is_empty() {
        return Sent::Nothing;
    }
    let Ok(batch) = DiagnosticBatch::new(events.clone()) else {
        // `take` takes a batch at most.
        return Sent::Nothing;
    };
    match requests.done(Request::Diagnostics(batch)).await {
        Ok(()) => Sent::Batch,
        // This session may send no more; the lines are not kept anywhere.
        Err(ClientError::Refused(RequestError::DiagnosticsNotKept)) => Sent::Stop,
        Err(ClientError::Disconnected) => {
            recorder.put_back(events);
            Sent::Stop
        }
        Err(_) => {
            recorder.put_back(events);
            Sent::Later
        }
    }
}

fn now_ms() -> u64 {
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
        recorder.record(
            DiagnosticLevel::Warn,
            "tpf3mp_agent",
            r"cannot read C:\Users\Alice\launcher.json",
        );
        let taken = recorder.take();
        assert_eq!(taken[0].text.as_str(), "cannot read <path>/launcher.json");

        for line in 0..KEPT + 5 {
            recorder.record(DiagnosticLevel::Info, "tpf3mp", &format!("line {line}"));
        }
        assert_eq!(recorder.len(), KEPT);
        assert_eq!(recorder.dropped(), 5);
        let batch = recorder.take();
        assert_eq!(batch.len(), MAX_DIAGNOSTIC_EVENTS);
        assert_eq!(batch[0].text.as_str(), "line 5", "the oldest go first");
        recorder.put_back(batch);
        assert_eq!(
            recorder.take()[0].text.as_str(),
            "line 5",
            "put back in front"
        );

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
        let event = &recorder.take()[0];
        assert!(event.text.as_str().starts_with("a b "));
        assert!(event.text.as_str().len() <= 1024);
    }
}

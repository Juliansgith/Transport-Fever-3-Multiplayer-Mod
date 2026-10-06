//! Keeps the launcher's window from taking the game's link down with it.
//!
//! The window draws with egui over wgpu. When the graphics device is lost
//! under it, as when the driver resets the GPU while the game runs, egui's
//! renderer panics on the next frame ("Failed to create staging buffer for
//! index data", egui-wgpu 0.36.2, 2026-10-06), and so may any other bug in
//! the window. The room session and the game's link run beside the window,
//! on the Tokio runtime, and must outlive it: [`keep_open`] catches the
//! window's panic, logs it in one line, and opens the window again, with a
//! new device. Should it crash again and again, it gives up the window and
//! the caller shows the launcher in the browser instead, the session still
//! running.

use std::{
    any::Any,
    panic::{AssertUnwindSafe, catch_unwind},
    time::{Duration, Instant},
};

use tracing::warn;

/// How many crashes within [`CRASH_SPAN`] the window is opened again after.
/// One more, and the launcher goes to the browser.
pub const REOPENS: usize = 3;
/// The time [`REOPENS`] counts crashes in.
pub const CRASH_SPAN: Duration = Duration::from_secs(10 * 60);

/// What the reopened window tells the player.
pub const REOPENED: &str = "The launcher's window restarted after a graphics error. \
     Your game and room kept running.";

/// How the window ended.
#[derive(Debug)]
pub enum Ended<E> {
    /// It closed, or could not open: what opening it returned.
    Closed(Result<(), E>),
    /// It crashed too often: the window is given up. The last crash's
    /// message.
    GaveUp(String),
}

/// When the window crashed lately, to tell a window that keeps crashing
/// from one that crashed once.
#[derive(Debug)]
pub struct Crashes {
    times: Vec<Instant>,
    reopens: usize,
    span: Duration,
}

impl Default for Crashes {
    fn default() -> Self {
        Self::new(REOPENS, CRASH_SPAN)
    }
}

impl Crashes {
    /// Allows `reopens` crashes within `span`.
    #[must_use]
    pub fn new(reopens: usize, span: Duration) -> Self {
        Self {
            times: Vec::new(),
            reopens,
            span,
        }
    }

    /// Counts a crash at `now`; whether the window may open again.
    pub fn crashed(&mut self, now: Instant) -> bool {
        self.times
            .retain(|&then| now.saturating_duration_since(then) < self.span);
        self.times.push(now);
        self.times.len() <= self.reopens
    }
}

/// Runs `open`, the window, until it closes; a panic in it opens it again,
/// as [`Crashes`] allows. `open` is told whether the window crashed before,
/// to say so to the player.
pub fn keep_open<E>(mut crashes: Crashes, mut open: impl FnMut(bool) -> Result<(), E>) -> Ended<E> {
    let mut crashed = false;
    loop {
        match catch_unwind(AssertUnwindSafe(|| open(crashed))) {
            Ok(result) => return Ended::Closed(result),
            Err(panic) => {
                let reason = panic_message(panic.as_ref());
                if !crashes.crashed(Instant::now()) {
                    warn!(
                        %reason,
                        "the launcher's window keeps crashing; the game and the room keep running, \
                         and the launcher opens in the browser instead"
                    );
                    return Ended::GaveUp(reason);
                }
                // The panic hook logged the whole message already.
                warn!(
                    %reason,
                    "the launcher's window crashed; the game and the room keep running; opening it again"
                );
                crashed = true;
            }
        }
    }
}

/// A panic's message, on one line.
#[must_use]
pub fn panic_message(panic: &(dyn Any + Send)) -> String {
    let message = panic
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic without a message".to_owned());
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    use super::{Crashes, Ended, keep_open, panic_message};

    /// The renderer's panic of 2026-10-06, as egui-wgpu 0.36.2 words it.
    const LOST: &str = "Failed to create staging buffer for index data. Index count: 11994.";

    #[test]
    fn a_crashed_window_opens_again_and_the_session_keeps_running() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        // The room session: it ticks on beside the window.
        let ticks = Arc::new(AtomicUsize::new(0));
        let session = runtime.spawn({
            let ticks = Arc::clone(&ticks);
            async move {
                loop {
                    ticks.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }
        });
        let mut opened = Vec::new();
        let ended = keep_open(Crashes::default(), |crashed| -> Result<(), ()> {
            opened.push(crashed);
            let before = ticks.load(Ordering::SeqCst);
            while ticks.load(Ordering::SeqCst) == before {
                std::thread::yield_now();
            }
            if opened.len() < 3 {
                panic!("{LOST}");
            }
            Ok(())
        });
        assert!(matches!(ended, Ended::Closed(Ok(()))), "{ended:?}");
        assert_eq!(opened, [false, true, true], "told it crashed before");
        assert!(!session.is_finished(), "the session outlived the crashes");
        let after = ticks.load(Ordering::SeqCst);
        while ticks.load(Ordering::SeqCst) == after {
            std::thread::yield_now();
        }
        session.abort();
    }

    #[test]
    fn a_window_that_keeps_crashing_is_given_up() {
        let mut opened = 0;
        let ended = keep_open(
            Crashes::new(2, Duration::from_secs(600)),
            |_| -> Result<(), ()> {
                opened += 1;
                panic!("{LOST}\n  more");
            },
        );
        assert_eq!(opened, 3, "opened, then again twice");
        match ended {
            Ended::GaveUp(reason) => assert_eq!(reason, format!("{LOST} more")),
            Ended::Closed(result) => panic!("closed: {result:?}"),
        }
    }

    #[test]
    fn a_window_that_closes_or_cannot_open_says_so() {
        assert!(matches!(
            keep_open(Crashes::default(), |_| Err::<(), _>("no adapter")),
            Ended::Closed(Err("no adapter"))
        ));
    }

    #[test]
    fn only_recent_crashes_count() {
        let mut crashes = Crashes::new(2, Duration::from_secs(60));
        let start = Instant::now();
        assert!(crashes.crashed(start));
        assert!(crashes.crashed(start + Duration::from_secs(10)));
        assert!(
            !crashes.crashed(start + Duration::from_secs(20)),
            "three in a minute"
        );
        let later = start + Duration::from_secs(200);
        assert!(crashes.crashed(later), "the old ones are forgotten");
    }

    #[test]
    fn a_panics_message_reads_on_one_line() {
        assert_eq!(panic_message(&"a\n b"), "a b");
        assert_eq!(panic_message(&String::from("c")), "c");
        assert_eq!(panic_message(&7_u8), "a panic without a message");
    }
}

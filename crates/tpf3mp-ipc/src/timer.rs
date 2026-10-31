//! A fine system timer while a link is polled (docs/HOOKS.md, "Startup,
//! heartbeat and restart").
//!
//! The agent finds what the game posts on the ring by polling it every 2 ms
//! from Tokio, and plays the room's steps out on Tokio timers. On Windows
//! Tokio parks with a millisecond timeout that the system rounds up to its
//! timer tick, 15.6 ms unless the process asks for less: measured on the
//! development PC (2026-10-29), `tokio::time::sleep(2 ms)` took 15.6 ms (p50)
//! and 3.4 ms with the period raised to 1 ms. Every command a player gives,
//! every report of a step run and every acknowledgement of a replayed action
//! waited up to a tick before the agent saw it. Since Windows 10 2004 the
//! period is the process's own, so raising it touches no other program;
//! elsewhere there is nothing to do.

/// Holds the system timer at 1 ms for this process until dropped.
#[derive(Debug)]
#[must_use = "the timer is fine only while the guard lives"]
pub struct FineTimer {
    #[cfg(windows)]
    raised: bool,
}

impl FineTimer {
    /// Asks for a 1 ms timer period; harmless where it cannot.
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            // SAFETY: timeBeginPeriod takes a plain value and changes only
            // this process's timer resolution; every success is matched by
            // timeEndPeriod in Drop.
            #[allow(unsafe_code)]
            let raised = unsafe { windows_sys::Win32::Media::timeBeginPeriod(1) } == 0;
            Self { raised }
        }
        #[cfg(not(windows))]
        {
            Self {}
        }
    }

    /// Whether the period was raised (Windows only).
    pub fn raised(&self) -> bool {
        #[cfg(windows)]
        {
            self.raised
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
}

impl Default for FineTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for FineTimer {
    fn drop(&mut self) {
        #[cfg(windows)]
        if self.raised {
            // SAFETY: matches the timeBeginPeriod(1) that succeeded in new.
            #[allow(unsafe_code)]
            unsafe {
                windows_sys::Win32::Media::timeEndPeriod(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn a_short_sleep_is_short_while_the_timer_is_fine() {
        use std::time::{Duration, Instant};
        let timer = FineTimer::new();
        assert!(timer.raised(), "timeBeginPeriod(1) refused");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let mut took: Vec<f64> = rt.block_on(async {
            let mut out = Vec::new();
            for _ in 0..40 {
                let start = Instant::now();
                tokio::time::sleep(Duration::from_millis(2)).await;
                out.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            out
        });
        took.sort_by(f64::total_cmp);
        let median = took[took.len() / 2];
        // 15.6 ms without the guard on a default Windows timer.
        assert!(median < 10.0, "a 2 ms sleep took {median:.1} ms");
        drop(timer);
    }

    #[test]
    fn the_guard_is_harmless_twice_and_elsewhere() {
        let a = FineTimer::new();
        let b = FineTimer::new();
        assert_eq!(a.raised(), cfg!(windows));
        assert_eq!(b.raised(), cfg!(windows));
    }
}

//! A trace of every call of the game's step, for finding where a room's
//! game stutters: when each call came, how many updates it ran, why it ran
//! none, and how long the game's step took. Off unless [`ENV`] is `1`.
//!
//! One line per call goes to hook.log, written in one go every
//! [`FLUSH_EVERY`] calls:
//!
//! ```text
//! step-trace: t=81234.5 gap=200.3 u=1 why=run game=27.4 call=28.9 lanes
//! ```
//!
//! `t` is milliseconds since the first traced call, `gap` since the call
//! before; `u` the updates answered (`own` for the game's own speed); `why`
//! the step driver's reason for that answer; `game` the game's own step and
//! `call` the whole detour, in milliseconds; `lanes` marks a checkpoint
//! batch, whose step also read the world's lanes.

use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

/// Set to `1` in the game's environment, every call of the step is traced.
pub const ENV: &str = "TPF3MP_HOOK_STEP_TRACE";

/// Lines kept before they are written.
pub const FLUSH_EVERY: usize = 50;

static ON: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<State> = Mutex::new(State {
    first: None,
    last: None,
    lines: Vec::new(),
});

struct State {
    first: Option<Instant>,
    last: Option<Instant>,
    lines: Vec<String>,
}

/// Whether [`ENV`]'s value asks for the trace: only `1`, `on`, `true` or
/// `yes`.
pub fn wanted(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "on" | "true" | "yes")
    )
}

/// Reads [`ENV`]; the line for hook.log when the trace is on.
pub fn configure_from_env() -> Option<String> {
    let on = wanted(std::env::var(ENV).ok().as_deref());
    ON.store(on, Ordering::Release);
    on.then(|| format!("step-trace: tracing every call of the game's step ({ENV}=1)"))
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// The clock the game's own step is timed by: the timing's
/// ([`crate::perf::start`]) when it is on, else one of its own while the
/// trace is on, so the trace's `game=` is the step's time either way.
pub fn step_timer(perf: Option<Instant>, trace: bool) -> Option<Instant> {
    perf.or_else(|| trace.then(Instant::now))
}

/// One call of the step: begun at `started`, `updates` answered (`None`
/// for the game's own speed) and why (the step driver's
/// [`crate::step::StepDriver::why`]), the game's step and the whole call in
/// nanoseconds. Returns the lines to write once enough have gathered.
pub fn call(
    started: Instant,
    updates: Option<u32>,
    why: &str,
    lanes: bool,
    game_nanos: u64,
    call_nanos: u64,
) -> Vec<String> {
    let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
    let first = *state.first.get_or_insert(started);
    let gap = state
        .last
        .map_or(0.0, |last| millis(started.saturating_duration_since(last)));
    state.last = Some(started);
    let line = format!(
        "step-trace: t={:.1} gap={gap:.1} u={} why={} game={:.1} call={:.1}{}",
        millis(started.saturating_duration_since(first)),
        updates.map_or_else(|| "own".to_owned(), |u| u.to_string()),
        if why.is_empty() { "-" } else { why },
        nanos_ms(game_nanos),
        nanos_ms(call_nanos),
        if lanes { " lanes" } else { "" },
    );
    state.lines.push(line);
    if state.lines.len() >= FLUSH_EVERY {
        std::mem::take(&mut state.lines)
    } else {
        Vec::new()
    }
}

fn millis(duration: std::time::Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[allow(clippy::cast_precision_loss)]
fn nanos_ms(nanos: u64) -> f64 {
    nanos as f64 / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_explicit_yes_turns_the_trace_on() {
        assert!(!wanted(None));
        assert!(!wanted(Some("0")));
        assert!(!wanted(Some("")));
        assert!(wanted(Some("1")));
        assert!(wanted(Some(" On ")));
    }

    #[test]
    fn the_game_step_is_timed_whenever_the_timing_or_the_trace_is_on() {
        let perf = Instant::now();
        assert_eq!(step_timer(Some(perf), false), Some(perf));
        assert_eq!(step_timer(Some(perf), true), Some(perf));
        // The timing off (TPF3MP_HOOK_PERF=0): the trace times it itself.
        assert!(step_timer(None, true).is_some());
        assert!(step_timer(None, false).is_none());
    }
}

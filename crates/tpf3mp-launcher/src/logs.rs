//! The launcher's log: daily files in the per-user `TPF3-MP/logs` folder,
//! a week of them. Its lines also go, redacted, to the server the player
//! plays on, unless they switch that off (`tpf3mp_agent::diagnostics`,
//! D10): the operator reads them by the support code the window shows.
//! `tpf3mp-agent collect-logs` zips the files with the hook's and the
//! game's, for the rare report that needs those (`tpf3mp_agent::logs`).

use std::{fmt, io, path::PathBuf};

use tpf3mp_agent::diagnostics::Recorder;
use tpf3mp_proto::DiagnosticLevel;
use tracing::{
    Event, Level, Subscriber, error,
    field::{Field, Visit},
};
use tracing_appender::{non_blocking::WorkerGuard, rolling};
use tracing_subscriber::{
    EnvFilter, Layer,
    layer::{Context, SubscriberExt},
    util::SubscriberInitExt,
};

/// Days of logs kept.
const DAYS_KEPT: usize = 7;
/// What is logged unless `RUST_LOG` says otherwise: TPF3-MP's own events,
/// and only warnings from the libraries under it.
const DEFAULT_FILTER: &str = "info,tauri=warn,wry=warn,tao=warn,quinn=warn,rustls=warn";

/// The per-user logs folder.
pub fn dir() -> anyhow::Result<PathBuf> {
    Ok(tpf3mp_agent::launcher::setup::data_dir()?.join("logs"))
}

/// Keeps the log writing until dropped, which flushes it.
pub struct Logging {
    _guard: WorkerGuard,
}

/// Logs to daily files in `dir`, and panics too, since a windowed
/// launcher has no console to print them on. `diagnostics` gets the same
/// lines, for the server.
pub fn start(dir: &std::path::Path, diagnostics: Option<Recorder>) -> io::Result<Logging> {
    std::fs::create_dir_all(dir)?;
    let appender = rolling::Builder::new()
        .rotation(rolling::Rotation::DAILY)
        .filename_prefix("launcher")
        .filename_suffix("log")
        .max_log_files(DAYS_KEPT)
        .build(dir)
        .map_err(io::Error::other)?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| DEFAULT_FILTER.into());
    // Another subscriber may be set already, in tests.
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_ansi(false),
        )
        .with(diagnostics.map(ToDiagnostics))
        .try_init();
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        error!(%panic, "the launcher panicked");
        default(panic);
    }));
    Ok(Logging { _guard: guard })
}

/// Passes the log's lines to the diagnostics recorder: TPF3-MP's own from
/// `info` up, other libraries' warnings and errors.
pub struct ToDiagnostics(pub Recorder);

impl<S: Subscriber> Layer<S> for ToDiagnostics {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        let level = match *metadata.level() {
            Level::ERROR => DiagnosticLevel::Error,
            Level::WARN => DiagnosticLevel::Warn,
            Level::INFO if metadata.target().starts_with("tpf3mp") => DiagnosticLevel::Info,
            _ => return,
        };
        let mut line = Line::default();
        event.record(&mut line);
        self.0.record(level, metadata.target(), &line.text());
    }
}

/// An event as one line: its message, then its fields as `name=value`.
#[derive(Default)]
struct Line {
    message: String,
    fields: String,
}

impl Line {
    fn text(&self) -> String {
        format!("{}{}", self.message, self.fields)
    }
}

impl Visit for Line {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            self.fields.push_str(&format!(" {}={value}", field.name()));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message.push_str(&format!("{value:?}"));
        } else {
            self.fields
                .push_str(&format!(" {}={value:?}", field.name()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_lines_and_everyones_warnings_go_to_the_recorder() {
        let recorder = Recorder::new();
        let subscriber = tracing_subscriber::registry().with(ToDiagnostics(recorder.clone()));
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "tpf3mp_agent::bridge", step = 29, "playing");
            tracing::info!(target: "wgpu_core", "an adapter");
            tracing::debug!(target: "tpf3mp_agent", "too fine");
            tracing::warn!(target: "quinn", error = %"reset", "a warning");
        });
        let lines: Vec<(DiagnosticLevel, String, String)> = recorder
            .waiting()
            .into_iter()
            .map(|event| {
                (
                    event.level,
                    event.target.as_str().to_owned(),
                    event.text.as_str().to_owned(),
                )
            })
            .collect();
        assert_eq!(
            lines,
            [
                (
                    DiagnosticLevel::Info,
                    "tpf3mp_agent::bridge".to_owned(),
                    "playing step=29".to_owned()
                ),
                (
                    DiagnosticLevel::Warn,
                    "quinn".to_owned(),
                    "a warning error=reset".to_owned()
                ),
            ]
        );
    }
}

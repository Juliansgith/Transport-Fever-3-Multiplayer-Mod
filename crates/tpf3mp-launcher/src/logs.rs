//! The launcher's log: daily files in the per-user `TPF3-MP/logs` folder,
//! a week of them, which "Open logs folder" shows. A player who reports a
//! problem sends these, with the support ID the window shows.

use std::{io, path::PathBuf};

use tracing::error;
use tracing_appender::{non_blocking::WorkerGuard, rolling};
use tracing_subscriber::EnvFilter;

/// Days of logs kept.
const DAYS_KEPT: usize = 7;
/// What is logged unless `RUST_LOG` says otherwise: TPF3-MP's own events,
/// and only warnings from the libraries under it.
const DEFAULT_FILTER: &str = "info,wgpu=warn,wgpu_core=warn,wgpu_hal=error,naga=warn,eframe=warn,egui=warn,winit=warn,quinn=warn,rustls=warn";

/// The per-user logs folder.
pub fn dir() -> anyhow::Result<PathBuf> {
    Ok(tpf3mp_agent::launcher::setup::data_dir()?.join("logs"))
}

/// Keeps the log writing until dropped, which flushes it.
pub struct Logging {
    _guard: WorkerGuard,
}

/// Logs to daily files in `dir`, and panics too, since a windowed
/// launcher has no console to print them on.
pub fn start(dir: &std::path::Path) -> io::Result<Logging> {
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
    let _ = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(filter)
        .try_init();
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        error!(%panic, "the launcher panicked");
        default(panic);
    }));
    Ok(Logging { _guard: guard })
}

//! `tpf3mp-launcher`: the TPF3-MP launcher window.

// A windowed program on Windows, without a console behind it. Debug builds
// keep the console, for running from a terminal.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::{process::ExitCode, time::Duration};

use anyhow::{Context, Result};
use clap::Parser;
use eframe::egui;
use tpf3mp_agent::{
    diagnostics::Recorder,
    launcher::{Launcher, LauncherConfig, Remembered, setup},
};
use tpf3mp_launcher::{
    app::{Extras, LauncherApp, Shown},
    backend::Local,
    icon, logs,
    notes::ReleaseNotes,
    probe::Probe,
    update,
};
use tracing::{error, info, warn};

/// The TPF3-MP launcher: connect to a server, create or join a room, and
/// play Transport Fever 3 together.
#[derive(Debug, Parser)]
#[command(version, about)]
// A flag given twice counts once, the later winning, so a player may add
// flags to a shortcut.
#[command(args_override_self = true)]
struct Args {
    #[command(flatten)]
    launcher: setup::LauncherArgs,

    /// Show the launcher as a page in the browser instead of a window.
    #[arg(long)]
    browser: bool,
}

/// The server a package plays on, set when it is built.
const DEFAULT_SERVER: Option<&str> = option_env!("TPF3MP_DEFAULT_SERVER");
/// What players see of that server, such as EU, set when it is built.
const SERVER_NAME: Option<&str> = option_env!("TPF3MP_SERVER_NAME");

fn main() -> ExitCode {
    let mut args = Args::parse();
    if args.launcher.default_server.is_none() {
        args.launcher.default_server = DEFAULT_SERVER.map(str::to_owned);
    }
    if args.launcher.server_name.is_none() {
        args.launcher.server_name = SERVER_NAME.map(str::to_owned);
    }
    let logs = logs::dir().ok();
    // The log's lines also wait here to go to the server, redacted.
    let diagnostics = Recorder::new();
    let _logging = logs
        .as_deref()
        .and_then(|dir| logs::start(dir, Some(diagnostics.clone())).ok());
    info!(
        version = update::VERSION,
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        "the launcher starts"
    );
    // An update downloaded last time installs before anything connects.
    if update::at_start() {
        return ExitCode::SUCCESS;
    }
    match run(args, diagnostics) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!("{error:#}");
            show_error(&format!("{error:#}"));
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args, diagnostics: Recorder) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("tpf3mp")
        .build()
        .context("starting the launcher")?;
    let mut config = args.launcher.config()?;
    // On unless the player switched them off.
    if let Some(file) = &config.remember {
        diagnostics.set_on(Remembered::load(file).diagnostics.unwrap_or(true));
    }
    config.diagnostics = Some(diagnostics);
    if args.browser {
        return in_browser(&runtime, config);
    }
    let launcher = {
        let _entered = runtime.enter();
        Launcher::start_local(config.clone())
    };
    // Whether the package's own server is up, shown before connecting.
    let probe = config
        .server
        .as_deref()
        .filter(|_| config.server_fixed)
        .and_then(Probe::start);
    let mut backend = Local::new(launcher.handle(), runtime.handle().clone());
    let updater = update::Updater::start(runtime.handle().clone());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Transport Fever 3 · Multiplayer")
            .with_app_id("tpf3mp-launcher")
            // The page's size, as tearded's launcher opens, and still
            // within a 1366x768 screen.
            .with_inner_size([1100.0, 690.0])
            .with_min_inner_size([960.0, 620.0])
            .with_icon(icon::icon()),
        ..Default::default()
    };
    let opened = eframe::run_native(
        "TPF3-MP",
        options,
        Box::new(move |creation| {
            // The window and its renderer exist: this version works, so an
            // update just installed is complete.
            update::started();
            backend.repaint_with(creation.egui_ctx.clone());
            updater.repaint_with(creation.egui_ctx.clone());
            Ok(Box::new(LauncherApp::new(
                backend,
                Extras {
                    updater: Some(updater),
                    probe,
                    notes: Some(ReleaseNotes::fetch()),
                    shown: Shown::default(),
                },
            )))
        }),
    );
    drop(launcher);
    match opened {
        Ok(()) => {
            info!("the launcher closes");
            // A download may still be running; it can be picked up next time.
            runtime.shutdown_timeout(Duration::from_secs(2));
            Ok(())
        }
        Err(error) => {
            warn!(%error, "cannot open the launcher's window; opening it in the browser instead");
            in_browser(&runtime, config)
        }
    }
}

/// Runs the launcher as a page in the browser until Ctrl-C.
fn in_browser(runtime: &tokio::runtime::Runtime, config: LauncherConfig) -> Result<()> {
    runtime.block_on(async {
        let listen = config.listen;
        let launcher = Launcher::start(config)
            .await
            .with_context(|| format!("serving the launcher on {listen}"))?;
        let url = launcher
            .url()
            .context("the launcher serves no page")?
            .to_owned();
        info!("the launcher runs in the browser");
        println!("TPF3-MP launcher: {url}");
        println!("Keep this running while you play. Ctrl-C stops the launcher.");
        if !setup::open_in_browser(&url) {
            println!("Open the address above in your browser.");
        }
        tokio::select! {
            () = launcher.wait() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
        Ok(())
    })
}

/// Says why the launcher cannot start, in a window, since a windowed
/// program has no console to print it on.
fn show_error(message: &str) {
    eprintln!("TPF3-MP cannot start: {message}");
    let message = message.to_owned();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("TPF3-MP")
            .with_inner_size([540.0, 200.0])
            .with_icon(icon::icon()),
        ..Default::default()
    };
    let _ = eframe::run_native(
        "TPF3-MP",
        options,
        Box::new(move |_| Ok(Box::new(ErrorWindow(message)))),
    );
}

struct ErrorWindow(String);

impl eframe::App for ErrorWindow {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default_margins().show(ui, |ui| {
            ui.heading("TPF3-MP cannot start");
            ui.add_space(6.0);
            ui.label(&self.0);
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("The launcher's log, in the TPF3-MP logs folder, has more.")
                    .weak(),
            );
            if ui.button("Close").clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }
}

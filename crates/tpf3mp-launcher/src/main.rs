//! `tpf3mp-launcher`: the TPF3-MP launcher window.

// A windowed program on Windows, without a console behind it. Debug builds
// keep the console, for running from a terminal.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::{process::ExitCode, time::Duration};

use anyhow::{Context, Result};
use clap::Parser;
use tauri::Manager;
use tpf3mp_agent::{
    diagnostics::Recorder,
    launcher::{Launcher, LauncherConfig, Remembered, setup},
};
use tpf3mp_launcher::{
    logs,
    probe::Probe,
    releases::Track,
    shell::{self, Shell, View},
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
    let updater = update::Updater::start(runtime.handle().clone());
    let shell = Shell::new(
        launcher.handle(),
        runtime.handle().clone(),
        Some(updater),
        probe,
    );
    let window = tauri::Builder::default()
        .manage(shell)
        .invoke_handler(tauri::generate_handler![
            view,
            act,
            window_ready,
            check_update,
            install_update,
            open_game_folder,
            answer_quit,
            releases,
            install_version,
            set_track,
            resume_updates,
        ])
        .on_window_event(|window, event| {
            // Closing during a game asks first: the page shows the question.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && window.state::<Shell>().close_needs_asking()
            {
                api.prevent_close();
            }
        })
        .build(tauri::generate_context!());
    match window {
        Ok(app) => {
            app.run_return(|_, _| {});
            drop(launcher);
            info!("the launcher closes");
            // A download may still be running; it can be picked up next time.
            runtime.shutdown_timeout(Duration::from_secs(2));
            Ok(())
        }
        Err(error) => {
            warn!(%error, "cannot open the launcher's window; opening it in the browser instead");
            drop(launcher);
            in_browser(&runtime, config)
        }
    }
}

/// Everything the page shows.
#[tauri::command]
fn view(shell: tauri::State<'_, Shell>) -> View {
    shell.view()
}

/// An action the player took on the page.
#[tauri::command]
async fn act(shell: tauri::State<'_, Shell>, action: serde_json::Value) -> Result<(), String> {
    let action = shell::parse_action(action)?;
    shell.act(action).await
}

/// The page is up: this version works, so an update just installed is
/// complete.
#[tauri::command]
fn window_ready() {
    update::started();
}

#[tauri::command]
fn check_update(shell: tauri::State<'_, Shell>) {
    if let Some(updater) = shell.updater() {
        updater.check();
    }
}

/// Installs the downloaded update and closes, for the new version to start.
#[tauri::command]
fn install_update(app: tauri::AppHandle, shell: tauri::State<'_, Shell>) {
    if shell
        .updater()
        .is_some_and(update::Updater::install_and_restart)
    {
        app.exit(0);
    }
}

/// Opens the game's folder, as Steam installed it, in the file manager.
#[tauri::command]
fn open_game_folder(shell: tauri::State<'_, Shell>) -> Result<(), String> {
    let dir = shell
        .game_dir()
        .ok_or("Transport Fever 3 was not found in Steam.")?;
    let opener = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(&dir)
        .spawn()
        .map(drop)
        .map_err(|error| format!("cannot open {dir}: {error}"))
}

/// A page of the project's releases, for the release notes and history.
#[tauri::command]
async fn releases(shell: tauri::State<'_, Shell>, page: u32) -> Result<shell::ReleasePage, String> {
    shell.releases(page).await
}

/// Installs the version the player chose, and closes for it to start.
#[tauri::command]
async fn install_version(
    app: tauri::AppHandle,
    shell: tauri::State<'_, Shell>,
    version: String,
) -> Result<(), String> {
    if shell.install_version(version).await? {
        app.exit(0);
    }
    Ok(())
}

/// Stable or Experimental.
#[tauri::command]
fn set_track(shell: tauri::State<'_, Shell>, experimental: bool) -> Result<(), String> {
    let track = if experimental {
        Track::Experimental
    } else {
        Track::Stable
    };
    shell
        .updater()
        .ok_or("This TPF3-MP does not update itself.")?
        .set_track(track)
}

/// Lets updates come again after the player held a version.
#[tauri::command]
fn resume_updates(shell: tauri::State<'_, Shell>) -> Result<(), String> {
    shell
        .updater()
        .ok_or("This TPF3-MP does not update itself.")?
        .resume()
}

/// The player's answer to "Quit TPF3-MP?" during a game.
#[tauri::command]
fn answer_quit(app: tauri::AppHandle, shell: tauri::State<'_, Shell>, quit: bool) {
    shell.answer_quit(quit);
    if quit {
        app.exit(0);
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

/// Says why the launcher cannot start. A windowed program has no console,
/// so the log has it too.
fn show_error(message: &str) {
    eprintln!("TPF3-MP cannot start: {message}");
}

//! What the launcher's page asks of the launcher (D16): everything it
//! shows, in one [`View`], and the [`Action`]s it may take. The page is
//! tearded's TPF2 Multiplayer Launcher, ported (`ui/`); the window around
//! it and the commands that reach this are in `main.rs`.

use std::{
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use serde::Serialize;
use tpf3mp_agent::launcher::{Action, LauncherHandle, State};

use crate::{
    installed,
    probe::{Probe, Reach},
    releases::{self, Release, Track},
    update::{self, Source, UpdateState, Updater},
};

/// How often the game's mods folder is looked at, so running the install
/// script shows without a restart.
const INSTALL_CHECK: Duration = Duration::from_secs(5);

/// Everything the page shows, as JSON.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    /// The launcher's own state: connection, room, game.
    pub launcher: State,
    pub update: UpdateView,
    /// Whether the launcher's own server answers, before connecting.
    pub reach: &'static str,
    /// This launcher's version.
    pub version: &'static str,
    /// The TPF3-MP version whose mod the install scripts put in place.
    pub installed_mod: Option<String>,
    /// The player closed the window during a game: the page asks first.
    pub quit_asked: bool,
    /// `windows`, `linux` or `macos`.
    pub platform: &'static str,
    /// The releases the player is offered.
    pub track: Track,
    /// Whether this build can follow the Dev track: it trusts a dev key.
    pub dev_track: bool,
    /// The version the player chose to stay on, if any.
    pub held: Option<String>,
}

/// A page of the release history.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleasePage {
    pub releases: Vec<Release>,
    pub more: bool,
}

/// The updater's state, as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateView {
    /// This launcher has no updater.
    None,
    Checking,
    UpToDate,
    Off {
        reason: String,
    },
    Failed {
        reason: String,
    },
    Downloading {
        version: String,
        bytes: u64,
        total: u64,
    },
    Ready {
        version: String,
    },
    Installing {
        version: String,
    },
    /// The player chose this version; nothing updates on its own.
    Held {
        version: String,
    },
}

impl From<UpdateState> for UpdateView {
    fn from(state: UpdateState) -> Self {
        match state {
            UpdateState::Checking => Self::Checking,
            UpdateState::UpToDate => Self::UpToDate,
            UpdateState::Off(reason) => Self::Off { reason },
            UpdateState::Failed(reason) => Self::Failed { reason },
            UpdateState::Downloading {
                version,
                bytes,
                total,
            } => Self::Downloading {
                version,
                bytes,
                total,
            },
            UpdateState::Ready { version } => Self::Ready { version },
            UpdateState::Installing { version } => Self::Installing { version },
            UpdateState::Held { version } => Self::Held { version },
        }
    }
}

fn reach_name(reach: Option<Reach>) -> &'static str {
    match reach {
        None => "none",
        Some(Reach::Unknown) => "unknown",
        Some(Reach::Online) => "online",
        Some(Reach::Offline) => "offline",
    }
}

/// An action from the page's JSON, such as `{"action": "join", "invite":
/// "K7QM2X", "password": null}`, or why it is not one.
pub fn parse_action(value: serde_json::Value) -> Result<Action, String> {
    serde_json::from_value(value).map_err(|error| format!("not an action: {error}"))
}

/// The launcher, its updater and its server's probe, as the window holds
/// them.
pub struct Shell {
    handle: LauncherHandle,
    runtime: tokio::runtime::Handle,
    updater: Option<Updater>,
    probe: Option<Probe>,
    quit_asked: AtomicBool,
    quitting: AtomicBool,
    /// An update check was started because the server is newer.
    looked_for_update: AtomicBool,
    installed_mod: Mutex<Option<(Instant, Option<String>)>>,
}

impl Shell {
    pub fn new(
        handle: LauncherHandle,
        runtime: tokio::runtime::Handle,
        updater: Option<Updater>,
        probe: Option<Probe>,
    ) -> Self {
        Self {
            handle,
            runtime,
            updater,
            probe,
            quit_asked: AtomicBool::new(false),
            quitting: AtomicBool::new(false),
            looked_for_update: AtomicBool::new(false),
            installed_mod: Mutex::new(None),
        }
    }

    /// What the page shows now.
    pub fn view(&self) -> View {
        let launcher = self.handle.state();
        // The server was updated past this TPF3-MP: look for the update now
        // rather than at the next regular check.
        if launcher.outdated
            && !self.looked_for_update.swap(true, Ordering::SeqCst)
            && let Some(updater) = &self.updater
        {
            updater.check();
        }
        View {
            launcher,
            update: self
                .updater
                .as_ref()
                .map_or(UpdateView::None, |updater| updater.state().into()),
            reach: reach_name(self.probe.as_ref().map(Probe::reach)),
            version: update::VERSION,
            installed_mod: self.installed_mod(),
            quit_asked: self.quit_asked.load(Ordering::SeqCst),
            platform: std::env::consts::OS,
            track: self
                .updater
                .as_ref()
                .and_then(Updater::choice)
                .map(|choice| choice.track)
                .unwrap_or_default(),
            dev_track: update::dev_builds_trusted(),
            held: self
                .updater
                .as_ref()
                .and_then(Updater::choice)
                .and_then(|choice| choice.hold),
        }
    }

    /// Page `page` of the project's releases.
    pub async fn releases(&self, page: u32) -> Result<ReleasePage, String> {
        let api = Source::github().api().to_owned();
        self.runtime
            .spawn_blocking(move || releases::fetch(&api, page))
            .await
            .map_err(|error| format!("the launcher stopped: {error}"))?
            .map(|(releases, more)| ReleasePage { releases, more })
            .map_err(|error| format!("cannot load the releases: {error}"))
    }

    /// Installs `version`, the player's choice. Returns whether the
    /// launcher must close for it to start. Refused during a game, which
    /// restarting would end.
    pub async fn install_version(&self, version: String) -> Result<bool, String> {
        if self.handle.state().room.is_some() {
            return Err("Leave the room first: installing restarts TPF3-MP.".into());
        }
        let updater = self
            .updater
            .clone()
            .ok_or("This TPF3-MP does not update itself.")?;
        self.runtime
            .spawn_blocking(move || updater.install_version(&version))
            .await
            .map_err(|error| format!("the launcher stopped: {error}"))?
    }

    /// Carries out `action`; its refusal also shows in the state's error.
    pub async fn act(&self, action: Action) -> Result<(), String> {
        let handle = self.handle.clone();
        self.runtime
            .spawn(async move { handle.act(action).await })
            .await
            .map_err(|error| format!("the launcher stopped: {error}"))?
    }

    /// Where Transport Fever 3 is installed, as Steam says.
    pub fn game_dir(&self) -> Option<String> {
        self.handle.state().installed.map(|installed| installed.dir)
    }

    pub fn updater(&self) -> Option<&Updater> {
        self.updater.as_ref()
    }

    /// Whether closing the window must be confirmed first: the player is
    /// in a room, and has not confirmed yet. Asking marks the question
    /// for the page.
    pub fn close_needs_asking(&self) -> bool {
        let in_room = self.handle.state().room.is_some();
        if in_room && !self.quitting.load(Ordering::SeqCst) {
            self.quit_asked.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    /// The player answered the question: quit, or stay.
    pub fn answer_quit(&self, quit: bool) {
        self.quit_asked.store(false, Ordering::SeqCst);
        self.quitting.store(quit, Ordering::SeqCst);
    }

    fn installed_mod(&self) -> Option<String> {
        let mut cached = self
            .installed_mod
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let stale = cached
            .as_ref()
            .is_none_or(|(when, _)| when.elapsed() >= INSTALL_CHECK);
        if stale {
            let found = tpf3mp_agent::launcher::setup::data_dir()
                .ok()
                .and_then(|dir| installed::installed_mod(&dir));
            *cached = Some((Instant::now(), found));
        }
        cached.as_ref().and_then(|(_, found)| found.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_s_actions_parse() {
        let join = parse_action(serde_json::json!({
            "action": "join", "invite": "K7QM2X", "password": null
        }))
        .unwrap();
        assert_eq!(
            join,
            Action::Join {
                invite: "K7QM2X".into(),
                password: None
            }
        );
        assert_eq!(
            parse_action(serde_json::json!({ "action": "launch_game" })).unwrap(),
            Action::LaunchGame
        );
    }

    #[test]
    fn anything_else_is_refused() {
        for value in [
            serde_json::json!({ "action": "format_disk" }),
            serde_json::json!({ "action": "connect" }),
            serde_json::json!("connect"),
            serde_json::json!({ "action": "ready", "ready": "yes" }),
        ] {
            let error = parse_action(value.clone()).unwrap_err();
            assert!(error.starts_with("not an action: "), "{value}: {error}");
        }
    }

    #[test]
    fn the_update_state_reads_as_the_page_expects() {
        let view = UpdateView::from(UpdateState::Downloading {
            version: "0.2.0".into(),
            bytes: 5,
            total: 10,
        });
        assert_eq!(
            serde_json::to_value(view).unwrap(),
            serde_json::json!({ "state": "downloading", "version": "0.2.0", "bytes": 5, "total": 10 })
        );
        assert_eq!(
            serde_json::to_value(UpdateView::Off {
                reason: "no key".into()
            })
            .unwrap(),
            serde_json::json!({ "state": "off", "reason": "no key" })
        );
    }

    #[test]
    fn the_probe_s_reach_has_a_name() {
        assert_eq!(reach_name(None), "none");
        assert_eq!(reach_name(Some(Reach::Online)), "online");
        assert_eq!(reach_name(Some(Reach::Offline)), "offline");
        assert_eq!(reach_name(Some(Reach::Unknown)), "unknown");
    }
}

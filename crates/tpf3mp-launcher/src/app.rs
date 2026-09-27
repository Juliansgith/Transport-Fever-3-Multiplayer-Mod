//! The launcher's window: the same steps as `docs/PLAYING.md` describes,
//! connect, create or join a room, start the game, get ready, play, drawn
//! from the launcher's [`State`] on every frame.
//!
//! Wide, it is laid out as the TPF2 launcher is (see [`theme`]): the game's
//! name, the steps and the room's talk on the left over the art, the step
//! at hand in glass panels on the right, and your game in a bar along the
//! bottom. Narrow, the same one under the other.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Align, ComboBox, Frame, Id, Layout, Margin, Modal, ProgressBar, RichText, ScrollArea,
    Sense, Stroke, TextEdit, Ui, vec2,
};
use tpf3mp_agent::launcher::{
    Action, Connection, Differences, Member, MemberContent, Phase, Room, State, World,
};

use crate::{
    backend::Backend,
    theme::{self, Kind, Step},
    update::{self, UpdateState, Updater},
};

/// How often the window rereads the launcher's state when nothing else
/// asks it to repaint.
const REFRESH: Duration = Duration::from_millis(250);
/// Player counts a room can be created for.
const ROOM_SIZES: [u8; 7] = [2, 3, 4, 6, 8, 12, 16];
/// Speeds the owner can set, in percent.
const SPEEDS: [(u16, &str); 6] = [
    (0, "Paused"),
    (100, "1×"),
    (200, "2×"),
    (400, "4×"),
    (800, "8×"),
    (1600, "16×"),
];
/// Notices kept in the session log, newest last.
const NOTICES_SHOWN: usize = 50;
/// How often the game's folder is looked at for what the installer put
/// there.
const INSTALL_CHECK: Duration = Duration::from_secs(5);
/// From this width on, the window has two columns.
const WIDE: f32 = 940.0;
/// The right column's width, when there are two.
const RIGHT: f32 = 470.0;
/// Between the columns.
const GAP: f32 = 32.0;
/// The height of a text field.
const FIELD_HEIGHT: f32 = 32.0;

/// What the window needs besides the launcher.
pub struct Extras {
    /// Where the log files are, for "Open logs folder".
    pub logs: Option<PathBuf>,
    /// Where "Collect logs" reads from and writes its zip; `None` hides
    /// the button.
    pub collect: Option<CollectLogs>,
    /// Checks for and installs new versions; `None` in tests.
    pub updater: Option<Updater>,
}

/// Where "Collect logs" works.
#[derive(Debug, Clone)]
pub struct CollectLogs {
    /// TPF3-MP's per-user data directory.
    pub data_dir: PathBuf,
    /// Where the zip goes: the Downloads folder, normally.
    pub out_dir: PathBuf,
    /// Show the zip in the file manager once written.
    pub reveal: bool,
    /// Look for the game's logs and crash dumps too.
    pub game: bool,
}

/// How the last "Collect logs" went.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Collecting {
    #[default]
    Idle,
    Busy,
    Done(PathBuf),
    Failed(String),
}

/// A question the window asks before acting.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Confirm {
    Kick { id: String, name: String },
    Leave,
    Quit,
}

/// The launcher's window over a [`Backend`].
pub struct LauncherApp<B> {
    backend: B,
    extras: Extras,
    /// The fields, as the player types them.
    server: String,
    name: String,
    room_name: String,
    max_players: u8,
    create_password: String,
    rules: Option<String>,
    invite: String,
    /// An invite given when connecting to the launcher's own server.
    connect_invite: String,
    join_password: String,
    chat: String,
    /// Whether the server and name fields took the launcher's first offer.
    offered: bool,
    confirm: Option<Confirm>,
    /// The player confirmed quitting during a game.
    quitting: bool,
    /// An update check was started because the server is newer.
    looked_for_update: bool,
    /// The log bundle being written, or the last one.
    collecting: Arc<Mutex<Collecting>>,
    /// The look is set on the first frame.
    styled: bool,
    /// When the game's folder was last looked at, and the TPF3-MP version
    /// the installer recorded there.
    installed_mod: Option<(Instant, Option<String>)>,
}

impl<B: Backend> LauncherApp<B> {
    pub fn new(backend: B, extras: Extras) -> Self {
        Self {
            backend,
            extras,
            server: String::new(),
            name: String::new(),
            room_name: String::new(),
            max_players: 4,
            create_password: String::new(),
            rules: None,
            invite: String::new(),
            connect_invite: String::new(),
            join_password: String::new(),
            chat: String::new(),
            offered: false,
            confirm: None,
            quitting: false,
            looked_for_update: false,
            collecting: Arc::default(),
            styled: false,
            installed_mod: None,
        }
    }

    /// How the last "Collect logs" went.
    pub fn collecting(&self) -> Collecting {
        self.collecting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Writes the log bundle on a thread of its own, since it reads up to
    /// tens of megabytes.
    fn collect_logs(&self, ctx: &egui::Context, support_id: Option<String>) {
        let Some(collect) = self.extras.collect.clone() else {
            return;
        };
        let status = Arc::clone(&self.collecting);
        *status.lock().unwrap_or_else(PoisonError::into_inner) = Collecting::Busy;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut bundle = tpf3mp_agent::logs::Collect::new(&collect.data_dir);
            bundle.support_id = support_id;
            if !collect.game {
                bundle.candidates = tpf3mp_agent::logs::own_candidates(&collect.data_dir);
            }
            let outcome = match bundle.write(&collect.out_dir) {
                Ok(bundle) => {
                    tracing::info!(path = %bundle.path.display(), "collected the logs");
                    if collect.reveal {
                        tpf3mp_agent::launcher::setup::reveal_file(&bundle.path);
                    }
                    Collecting::Done(bundle.path)
                }
                Err(error) => {
                    tracing::warn!(%error, "cannot collect the logs");
                    Collecting::Failed(error.to_string())
                }
            };
            *status.lock().unwrap_or_else(PoisonError::into_inner) = outcome;
            ctx.request_repaint();
        });
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Draws the window once.
    pub fn show(&mut self, ui: &mut Ui) {
        if !self.styled {
            theme::apply(ui.ctx());
            self.styled = true;
            // egui takes new fonts from the next frame on: draw from then.
            ui.ctx().request_repaint();
            return;
        }
        let state = self.backend.state();
        if !self.offered {
            // The server and name remembered from last time, or given.
            self.server = state.server.clone().unwrap_or_default();
            self.name = state.name.clone();
            self.offered = true;
        }
        self.guard_quit(ui.ctx(), &state);
        // The art under everything, to the window's edges: the panels over
        // it are glass.
        let window = ui.ctx().content_rect();
        theme::art(&ui.painter().with_clip_rect(window), window);
        egui::Panel::top("header")
            .frame(Frame::new().inner_margin(Margin::symmetric(theme::GUTTER, 12)))
            .show(ui, |ui| self.header(ui, &state));
        egui::Panel::bottom("footer")
            .frame(Frame::new().inner_margin(Margin {
                left: theme::GUTTER,
                right: theme::GUTTER,
                top: 6,
                bottom: 10,
            }))
            .show(ui, |ui| self.footer(ui, &state));
        let body = Frame::new().inner_margin(Margin::symmetric(theme::GUTTER, 10));
        egui::CentralPanel::default_margins()
            .frame(body)
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if ui.available_width() >= WIDE {
                            self.wide(ui, &state);
                        } else {
                            self.narrow(ui, &state);
                        }
                    });
            });
        self.dialogs(ui.ctx(), &state);
        ui.ctx().request_repaint_after(REFRESH);
    }

    /// The mark and the launcher's name; how it is connected, and to what.
    /// Narrow, the server's details take a second row.
    fn header(&mut self, ui: &mut Ui, state: &State) {
        let narrow = ui.available_width() < WIDE;
        ui.horizontal(|ui| {
            theme::mark(ui, 26.0);
            ui.label(
                RichText::new("TPF3-MP")
                    .font(theme::heading_font(17.0))
                    .color(theme::TEXT),
            );
            if !narrow {
                ui.label(RichText::new("Multiplayer for Transport Fever 3").color(theme::MUTED));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (text, color) = match state.connection {
                    Connection::Connected if state.tunneled => {
                        ("connected via tunnel", theme::INFO)
                    }
                    Connection::Connected => ("connected", theme::SUCCESS),
                    Connection::Connecting => ("connecting", theme::WARNING),
                    Connection::Disconnected => ("offline", theme::MUTED),
                };
                theme::pill(ui, text, color);
                if !narrow {
                    server_details(ui, state);
                }
            });
        });
        if narrow && (state.support_id.is_some() || state.server_version.is_some()) {
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    server_details(ui, state);
                });
            });
        }
        // A hairline under the header, as the TPF2 launcher has.
        let rect = ui.max_rect();
        ui.painter().line_segment(
            [
                rect.left_bottom() + vec2(0.0, 12.0),
                rect.right_bottom() + vec2(0.0, 12.0),
            ],
            Stroke::new(1.0, theme::LINE),
        );
    }

    /// Your game along the bottom, as the TPF2 launcher shows it: where it
    /// is, whether the mod is in, and the logs; under it, this launcher's
    /// version and updates.
    fn footer(&mut self, ui: &mut Ui, state: &State) {
        let installed_mod = self.installed_mod(state);
        theme::bar(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("YOUR GAME")
                        .font(theme::heading_font(11.5))
                        .color(theme::LABEL),
                );
                ui.add_space(6.0);
                folder(ui);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    match &state.installed {
                        Some(installed) => {
                            ui.label(
                                RichText::new(format!(
                                    "Transport Fever 3 · Steam build {}",
                                    installed.build
                                ))
                                .small()
                                .color(theme::LABEL),
                            );
                            ui.label(RichText::new(&installed.dir).monospace().color(theme::TEXT));
                        }
                        None => {
                            ui.label(
                                RichText::new("Transport Fever 3")
                                    .small()
                                    .color(theme::LABEL),
                            );
                            ui.label("Transport Fever 3 was not found in Steam.");
                        }
                    }
                });
                if let Some(version) = &installed_mod {
                    theme::pill(ui, &format!("TPF3-MP {version} installed"), theme::SUCCESS);
                } else if state.installed.is_some() {
                    theme::pill(ui, "TPF3-MP not installed", theme::WARNING);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    self.logs_buttons(ui, state);
                });
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Unofficial: not made or endorsed by Urban Games")
                    .small()
                    .color(theme::FAINT),
            );
            if let Some(on) = state.diagnostics {
                let mut sending = on;
                if ui
                    .checkbox(&mut sending, "Send diagnostics")
                    .on_hover_text(
                        "Lines of this launcher's log go to the server you play on, with \
                         paths, addresses and keys taken out, so its operator can see what \
                         went wrong by your support ID. The server keeps them for a limited \
                         time, 30 days unless its operator chose otherwise.",
                    )
                    .changed()
                {
                    self.backend.act(Action::Diagnostics { on: sending });
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("TPF3-MP {}", env!("CARGO_PKG_VERSION")))
                        .small()
                        .color(theme::MUTED),
                );
                if let Some(updater) = &self.extras.updater {
                    update_line(ui, updater);
                }
            });
        });
    }

    /// "Open logs folder" and "Collect logs", right to left, and how the
    /// last collecting went.
    fn logs_buttons(&mut self, ui: &mut Ui, state: &State) {
        if self.extras.collect.is_some() {
            let collecting = self.collecting();
            let busy = collecting == Collecting::Busy;
            match &collecting {
                Collecting::Idle => {}
                Collecting::Busy => {
                    ui.label(RichText::new("collecting…").weak());
                }
                Collecting::Done(path) => {
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    ui.label(RichText::new(format!("saved {name}")).weak())
                        .on_hover_text(path.display().to_string());
                }
                Collecting::Failed(error) => {
                    ui.label(
                        RichText::new("cannot collect the logs").color(ui.visuals().error_fg_color),
                    )
                    .on_hover_text(error);
                }
            }
            if theme::button(ui, !busy, "Collect logs", Kind::Secondary)
                .on_hover_text(
                    "Put TPF3-MP's logs and the game's into one zip for a bug report. \
                     Keys and tokens are never included.",
                )
                .clicked()
            {
                self.collect_logs(ui.ctx(), state.support_id.clone());
            }
        }
        if let Some(logs) = &self.extras.logs
            && theme::button(ui, true, "Open logs folder", Kind::Secondary)
                .on_hover_text(logs.display().to_string())
                .clicked()
        {
            tpf3mp_agent::launcher::setup::open_folder(logs);
        }
    }

    /// Two columns: the game's name, the steps and the room's talk on the
    /// left; what to do now on the right.
    fn wide(&mut self, ui: &mut Ui, state: &State) {
        let spacing = ui.spacing().item_spacing.x;
        let left = ui.available_width() - RIGHT - GAP - spacing * 2.0;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(left);
                ui.add_space(18.0);
                let size = (left * 0.15).clamp(52.0, 104.0);
                theme::wordmark(ui, size, &byline());
                ui.add_space(26.0);
                theme::checklist_column(ui, &steps(state));
                ui.add_space(22.0);
                if state.room.is_some() {
                    self.chat(ui, state);
                }
                self.session_log(ui, state);
            });
            ui.add_space(GAP);
            ui.vertical(|ui| {
                ui.set_width(RIGHT);
                ui.add_space(8.0);
                self.banners(ui, state);
                self.step_panels(ui, state);
                self.game_panel(ui, state);
            });
        });
    }

    /// One column, for a narrow window.
    fn narrow(&mut self, ui: &mut Ui, state: &State) {
        ui.add_space(6.0);
        let size = (ui.available_width() * 0.085).clamp(34.0, 56.0);
        theme::wordmark(ui, size, &byline());
        ui.add_space(14.0);
        self.banners(ui, state);
        theme::checklist(ui, &steps(state));
        ui.add_space(12.0);
        self.step_panels(ui, state);
        self.game_panel(ui, state);
        if state.room.is_some() {
            self.chat(ui, state);
        }
        self.session_log(ui, state);
    }

    /// Updates, the server's word, errors and differing mods, over the
    /// panels.
    fn banners(&mut self, ui: &mut Ui, state: &State) {
        if let Some(updater) = &self.extras.updater {
            update_banner(ui, updater, state);
        }
        if state.outdated {
            self.outdated(ui);
        }
        if let Some(text) = &state.announcement {
            theme::banner(ui, theme::INFO, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("From the server:")
                            .strong()
                            .color(theme::INFO),
                    );
                    ui.label(text);
                });
            });
        }
        if let Some(error) = &state.error {
            theme::banner(ui, theme::DANGER, |ui| {
                ui.label(RichText::new(error).color(theme::DANGER));
            });
        }
        if let Some(diff) = &state.content_diff {
            differences(ui, diff);
        }
    }

    /// The step at hand: a server to connect to, a room to create or join,
    /// or the room.
    fn step_panels(&mut self, ui: &mut Ui, state: &State) {
        let connected = state.connection == Connection::Connected;
        match &state.room {
            Some(room) => self.room(ui, state, room),
            None if connected => self.lobby(ui, state),
            None => self.connect(ui, state),
        }
    }

    fn game_panel(&mut self, ui: &mut Ui, state: &State) {
        let installed_mod = self.installed_mod(state);
        if game(ui, state, installed_mod.as_deref()) {
            self.backend.act(Action::LaunchGame);
        }
    }

    fn session_log(&mut self, ui: &mut Ui, state: &State) {
        if state.notices.is_empty() {
            return;
        }
        theme::glass(ui, "Session log", |ui| {
            let start = state.notices.len().saturating_sub(NOTICES_SHOWN);
            theme::log_box(
                ui,
                "session-log",
                state.notices[start..].iter().map(String::as_str),
                140.0,
            );
        });
    }

    /// The TPF3-MP version whose mod the installer put in the game's mods
    /// folder, looked at every few seconds, so running the installer shows
    /// here.
    fn installed_mod(&mut self, _state: &State) -> Option<String> {
        let stale = self
            .installed_mod
            .as_ref()
            .is_none_or(|(when, _)| when.elapsed() >= INSTALL_CHECK);
        if stale {
            let installed = tpf3mp_agent::launcher::setup::data_dir()
                .ok()
                .and_then(|dir| installed_mod(&dir));
            self.installed_mod = Some((Instant::now(), installed));
        }
        self.installed_mod.as_ref()?.1.clone()
    }

    /// The server was updated past this TPF3-MP: says so, and looks for
    /// the update now rather than at the next regular check.
    fn outdated(&mut self, ui: &mut Ui) {
        if !self.looked_for_update {
            self.looked_for_update = true;
            if let Some(updater) = &self.extras.updater {
                updater.check();
            }
        }
        let update = self.extras.updater.as_ref().map(Updater::state);
        theme::banner(ui, theme::WARNING, |ui| {
            ui.label(
                RichText::new("This TPF3-MP is older than the server's")
                    .strong()
                    .color(theme::WARNING),
            );
            match update {
                Some(UpdateState::Ready { .. }) => {
                    ui.label("The new version is ready: restart and update to play.");
                }
                Some(UpdateState::Checking | UpdateState::Downloading { .. }) => {
                    ui.label("Downloading the new version…");
                }
                _ => {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Download the new version from");
                        ui.hyperlink_to(
                            "the TPF3-MP releases",
                            format!("https://github.com/{}/releases/latest", update::REPOSITORY),
                        );
                    });
                }
            }
        });
    }

    /// Connecting: to the launcher's own server, the only one it plays on
    /// when it has one (D12), or, in a build for development, to the one
    /// typed. An invite given here also joins its room.
    fn connect(&mut self, ui: &mut Ui, state: &State) {
        let fixed = state.server.as_deref().filter(|_| state.server_fixed);
        theme::glass(ui, "Server", |ui| {
            if let Some(server) = fixed {
                theme::rows(ui, "own-server", &[("Server", server)]);
                ui.add_space(4.0);
            }
            let mut submit = false;
            egui::Grid::new("connect")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    if fixed.is_none() {
                        submit |= field(
                            ui,
                            "Server",
                            TextEdit::singleline(&mut self.server)
                                .hint_text("server:29470, or a whole invite")
                                .desired_width(f32::INFINITY),
                        );
                        ui.end_row();
                    }
                    submit |= field(
                        ui,
                        "Your name",
                        TextEdit::singleline(&mut self.name)
                            .char_limit(32)
                            .desired_width(f32::INFINITY),
                    );
                    ui.end_row();
                    if fixed.is_some() {
                        submit |= field(
                            ui,
                            "Invite",
                            TextEdit::singleline(&mut self.connect_invite)
                                .hint_text("optional: one you were sent")
                                .desired_width(f32::INFINITY),
                        );
                        ui.end_row();
                    }
                });
            ui.add_space(8.0);
            let connecting = state.connection == Connection::Connecting || self.backend.busy();
            let clicked = theme::big_button(ui, !connecting, "Connect").clicked();
            if (clicked || submit) && !connecting {
                // With its own server, only an invite goes with the name.
                let server = if fixed.is_some() {
                    self.connect_invite.trim().to_owned()
                } else {
                    self.server.clone()
                };
                self.backend.act(Action::Connect {
                    server,
                    name: self.name.clone(),
                });
            }
            ui.horizontal_wrapped(|ui| {
                if connecting {
                    ui.spinner();
                }
                let hint = if fixed.is_some() {
                    "Got an invite? Paste it too: you connect and join in one step."
                } else {
                    "Got an invite? Paste all of it as the server: you connect and join in one step."
                };
                ui.label(RichText::new(hint).weak().small());
            });
        });
    }

    fn lobby(&mut self, ui: &mut Ui, state: &State) {
        let busy = self.backend.busy();
        theme::glass(ui, "Create a room", |ui| {
            // Two to a row, so joining fits under it at the window's first
            // size.
            egui::Grid::new("create")
                .num_columns(4)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    field_sized(
                        ui,
                        "Room name",
                        TextEdit::singleline(&mut self.room_name)
                            .hint_text("TPF3-MP room")
                            .char_limit(48),
                        170.0,
                    );
                    ui.label("Players");
                    ComboBox::from_id_salt("max-players")
                        .width(64.0)
                        .selected_text(self.max_players.to_string())
                        .show_ui(ui, |ui| {
                            for size in ROOM_SIZES {
                                ui.selectable_value(&mut self.max_players, size, size.to_string());
                            }
                        });
                    ui.end_row();
                    field_sized(
                        ui,
                        "Password",
                        TextEdit::singleline(&mut self.create_password)
                            .hint_text("optional")
                            .password(true)
                            .char_limit(64),
                        170.0,
                    );
                    if !state.rules.is_empty() {
                        let chosen = self
                            .rules
                            .clone()
                            .filter(|name| state.rules.iter().any(|rules| rules.name == *name))
                            .unwrap_or_else(|| state.rules[0].name.clone());
                        ui.label("Rules");
                        let mut picked = chosen.clone();
                        ComboBox::from_id_salt("rules")
                            .width(64.0)
                            .selected_text(&picked)
                            .show_ui(ui, |ui| {
                                for rules in &state.rules {
                                    ui.selectable_value(
                                        &mut picked,
                                        rules.name.clone(),
                                        &rules.name,
                                    )
                                    .on_hover_text(&rules.description);
                                }
                            });
                        self.rules = Some(picked);
                    }
                    ui.end_row();
                });
            if let Some(rules) = self
                .rules
                .as_ref()
                .and_then(|picked| state.rules.iter().find(|rules| rules.name == *picked))
            {
                ui.label(
                    RichText::new(format!("{}: {}", rules.name, rules.description))
                        .weak()
                        .small(),
                );
            }
            ui.add_space(8.0);
            if theme::big_button(ui, !busy, "Create room").clicked() {
                let room = self.room_name.trim();
                self.backend.act(Action::Create {
                    room: if room.is_empty() {
                        "TPF3-MP room".to_owned()
                    } else {
                        room.to_owned()
                    },
                    max_players: self.max_players,
                    password: non_empty(&self.create_password),
                    rules: self.rules.clone(),
                });
            }
        });
        theme::glass(ui, "Join a room", |ui| {
            let mut submit = false;
            egui::Grid::new("join")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    submit |= field(
                        ui,
                        "Invite",
                        TextEdit::singleline(&mut self.invite)
                            .hint_text("paste the invite you were sent")
                            .desired_width(f32::INFINITY),
                    );
                    ui.end_row();
                    submit |= field(
                        ui,
                        "Room password",
                        TextEdit::singleline(&mut self.join_password)
                            .hint_text("if it has one")
                            .password(true)
                            .char_limit(64)
                            .desired_width(f32::INFINITY),
                    );
                    ui.end_row();
                });
            ui.add_space(8.0);
            let clicked = theme::button(ui, !busy, "Join room", Kind::Secondary).clicked();
            if (clicked || submit) && !busy {
                self.backend.act(Action::Join {
                    invite: self.invite.clone(),
                    password: non_empty(&self.join_password),
                });
            }
        });
        if theme::button(ui, !busy, "Disconnect", Kind::Ghost).clicked() {
            self.backend.act(Action::Disconnect);
        }
    }

    fn room(&mut self, ui: &mut Ui, state: &State, room: &Room) {
        let busy = self.backend.busy();
        theme::glass(ui, "", |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(&room.name)
                        .font(theme::heading_font(19.0))
                        .color(theme::TEXT),
                );
                theme::pill(ui, &format!("{} rules", room.rules), theme::INFO);
                match room.phase {
                    Phase::Lobby => theme::pill(ui, "lobby", theme::MUTED),
                    Phase::Running => theme::pill(ui, "game running", theme::SUCCESS),
                };
            });
            ui.add_space(6.0);
            if let Some(invite) = &room.invite {
                ui.horizontal(|ui| {
                    let copy = ui
                        .with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let copy = theme::button(ui, true, "Copy invite", Kind::Secondary);
                            let mut shown = invite.clone();
                            ui.add(
                                TextEdit::singleline(&mut shown)
                                    .desired_width(f32::INFINITY)
                                    .font(egui::TextStyle::Monospace)
                                    .interactive(false),
                            )
                            .on_hover_text("Send this to your friends");
                            copy
                        })
                        .inner;
                    if copy.clicked() {
                        ui.ctx().copy_text(invite.clone());
                    }
                });
                ui.add_space(8.0);
            }
            for member in &room.members {
                self.member_row(ui, room, member);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let you_ready = room.members.iter().any(|member| member.you && member.ready);
                match room.phase {
                    Phase::Lobby => {
                        let (label, kind) = if you_ready {
                            ("Not ready", Kind::Secondary)
                        } else {
                            ("Ready", Kind::Primary)
                        };
                        if theme::button(ui, !busy, label, kind).clicked() {
                            self.backend.act(Action::Ready { ready: !you_ready });
                        }
                        if room.you_own {
                            let everyone = room.members.iter().all(|member| member.ready);
                            if theme::button(ui, everyone && !busy, "Start game", Kind::Primary)
                                .on_disabled_hover_text("Everyone must be ready")
                                .clicked()
                            {
                                self.backend.act(Action::Start);
                            }
                        }
                    }
                    Phase::Running if room.you_own => {
                        let current = state.game.speed;
                        let label = SPEEDS
                            .iter()
                            .find(|(percent, _)| *percent == current)
                            .map_or("speed", |(_, label)| *label);
                        ComboBox::from_id_salt("speed")
                            .selected_text(label)
                            .show_ui(ui, |ui| {
                                for (percent, label) in SPEEDS {
                                    if ui.selectable_label(percent == current, label).clicked() {
                                        self.backend.act(Action::Speed { percent });
                                    }
                                }
                            });
                    }
                    Phase::Running => {}
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::button(ui, !busy, "Leave room", Kind::Danger).clicked() {
                        self.confirm = Some(Confirm::Leave);
                    }
                });
            });
        });
    }

    /// One player of the room: name and system, then whether their game
    /// and mods match the owner's and whether they are ready.
    fn member_row(&mut self, ui: &mut Ui, room: &Room, member: &Member) {
        let mut name = member.name.clone();
        if member.owner {
            name.push_str(" ★");
        }
        if member.you {
            name.push_str(" (you)");
        }
        if !member.connected {
            name.push_str(" · away");
        }
        let row = ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new(name).color(theme::TEXT));
                ui.label(RichText::new(&member.platform).small().color(theme::LABEL));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if room.you_own
                    && !member.you
                    && theme::small_button(ui, true, "Remove", Kind::Danger).clicked()
                {
                    self.confirm = Some(Confirm::Kick {
                        id: member.id.clone(),
                        name: member.name.clone(),
                    });
                }
                if room.phase == Phase::Lobby {
                    if member.ready {
                        theme::pill(ui, "ready", theme::SUCCESS);
                    } else {
                        theme::pill(ui, "not ready", theme::MUTED);
                    }
                }
                if member.owner {
                    theme::pill(ui, "the room's mods", theme::MUTED);
                } else {
                    match member.content {
                        MemberContent::Same => theme::pill(ui, "same mods", theme::MUTED),
                        MemberContent::Differs => theme::pill(ui, "differ", theme::WARNING),
                        MemberContent::Unknown => ui.label(""),
                    };
                }
            });
        });
        // A hairline between players.
        let rect = row.response.rect;
        ui.painter().line_segment(
            [
                rect.left_bottom() + vec2(0.0, 4.0),
                rect.right_bottom() + vec2(0.0, 4.0),
            ],
            Stroke::new(1.0, theme::LINE),
        );
        ui.add_space(6.0);
    }

    fn chat(&mut self, ui: &mut Ui, state: &State) {
        theme::glass(ui, "Chat", |ui| {
            ScrollArea::vertical()
                .id_salt("chat-log")
                .max_height(160.0)
                .stick_to_bottom(true)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for line in &state.chat {
                        ui.horizontal_wrapped(|ui| {
                            let color = if line.you { theme::ACCENT } else { theme::TEXT };
                            ui.label(
                                RichText::new(format!("{}:", line.from))
                                    .strong()
                                    .color(color),
                            );
                            ui.label(&line.text);
                        });
                    }
                });
            ui.horizontal(|ui| {
                let label = ui.label("Message");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let send = theme::button(ui, true, "Send", Kind::Secondary).clicked();
                    let response = ui
                        .add(fitted(
                            TextEdit::singleline(&mut self.chat)
                                .hint_text("say something to the room")
                                .char_limit(280)
                                .desired_width(f32::INFINITY),
                        ))
                        .labelled_by(label.id);
                    let entered = response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    if (send || entered) && !self.chat.trim().is_empty() {
                        self.backend.act(Action::Chat {
                            text: std::mem::take(&mut self.chat),
                        });
                        response.request_focus();
                    }
                });
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context, state: &State) {
        let Some(confirm) = self.confirm.clone() else {
            return;
        };
        let (question, detail, yes) = match &confirm {
            Confirm::Kick { name, .. } => (
                format!("Remove {name} from the room?"),
                "They cannot come back to this room.",
                "Remove them",
            ),
            Confirm::Leave => (
                "Leave the room?".to_owned(),
                "You give up your seat; an invite can bring you back while the room has room.",
                "Leave",
            ),
            Confirm::Quit => (
                "Quit TPF3-MP?".to_owned(),
                "You leave the room. Your seat waits for you for a while, and the same invite brings you back.",
                "Quit",
            ),
        };
        let mut answer = None;
        let modal = Modal::new(Id::new("confirm")).show(ctx, |ui| {
            ui.set_max_width(360.0);
            ui.heading(question);
            ui.label(detail);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if theme::button(ui, true, yes, Kind::Danger).clicked() {
                    answer = Some(true);
                }
                if theme::button(ui, true, "Cancel", Kind::Secondary).clicked() {
                    answer = Some(false);
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(false);
        }
        match answer {
            Some(true) => {
                self.confirm = None;
                match confirm {
                    Confirm::Kick { id, .. } => self.backend.act(Action::Kick { player: id }),
                    Confirm::Leave => self.backend.act(Action::Leave),
                    Confirm::Quit => {
                        self.quitting = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
            Some(false) => self.confirm = None,
            None => {}
        }
        let _ = state;
    }

    /// Closing the window during a game asks first.
    fn guard_quit(&mut self, ctx: &egui::Context, state: &State) {
        let close = ctx.input(|input| input.viewport().close_requested());
        if close && state.room.is_some() && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Confirm::Quit);
        }
    }
}

impl<B: Backend> eframe::App for LauncherApp<B> {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

/// A labelled text field; returns whether Enter was pressed in it.
fn field(ui: &mut Ui, label: &str, edit: TextEdit<'_>) -> bool {
    let label = ui.label(label);
    let response = ui.add(fitted(edit)).labelled_by(label.id);
    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
}

/// Every text field as tall as the others, its words in the middle.
fn fitted(edit: TextEdit<'_>) -> TextEdit<'_> {
    edit.min_size(vec2(0.0, FIELD_HEIGHT))
        .vertical_align(Align::Center)
}

/// A labelled text field of this width, where a grid would not give it
/// one; returns whether Enter was pressed in it.
fn field_sized(ui: &mut Ui, label: &str, edit: TextEdit<'_>, width: f32) -> bool {
    let label = ui.label(label);
    let response = ui
        .add_sized(vec2(width, FIELD_HEIGHT), fitted(edit))
        .labelled_by(label.id);
    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
}

/// What to change for this player's game to match the room's.
fn differences(ui: &mut Ui, diff: &Differences) {
    theme::banner(ui, theme::WARNING, |ui| {
        ui.label(
            RichText::new("Your game differs from the room's")
                .strong()
                .color(theme::WARNING),
        );
        if let Some((room, yours)) = &diff.game {
            ui.label(format!(
                "Game build: the room runs {room}, you run {yours}."
            ));
        }
        let list = |ui: &mut Ui, title: &str, items: &[String], more: u32| {
            if items.is_empty() {
                return;
            }
            ui.label(title);
            for item in items {
                ui.label(format!("   • {item}"));
            }
            if more > 0 {
                ui.label(format!("   • and {more} more"));
            }
        };
        list(ui, "Mods you lack:", &diff.missing, diff.missing_more);
        list(
            ui,
            "Mods the room lacks (turn them off):",
            &diff.extra,
            diff.extra_more,
        );
        let changed: Vec<String> = diff
            .changed
            .iter()
            .map(|(id, room, yours)| format!("{id}: the room has {room}, you have {yours}"))
            .collect();
        list(ui, "Other versions:", &changed, diff.changed_more);
        if diff.reordered {
            ui.label("The same mods load in another order.");
        }
        if diff.unlisted {
            ui.label("The mods beyond the listed ones differ.");
        }
        ui.label(
            RichText::new(
                "Everyone needs the room's game build and the same mods, in the same order. \
                 Change yours to match, then start the game from the launcher again.",
            )
            .weak()
            .small(),
        );
    });
}

/// Where the game stands: waiting, connected, loading, playing. Returns
/// whether the player asked to start it: only a game started here runs
/// TPF3-MP (D11). Where the game is, and its mod, are in the bar along
/// the bottom.
fn game(ui: &mut Ui, state: &State, installed_mod: Option<&str>) -> bool {
    theme::glass(ui, "Game", |ui| {
        let game = &state.game;
        ui.horizontal_wrapped(|ui| match (&game.attached, game.world) {
            (None, _) => theme::pill(ui, "waiting for the game", theme::MUTED),
            (Some(_), World::Playing) => theme::pill(ui, "playing", theme::SUCCESS),
            (Some(_), World::Fetching | World::Loading) => {
                theme::pill(ui, "loading the world", theme::INFO)
            }
            (Some(_), World::None) => theme::pill(ui, "game connected", theme::SUCCESS),
        });
        let line = match (&game.attached, game.world) {
            (None, _) if state.room.is_some() => {
                "Start Transport Fever 3 from here: only a game TPF3-MP starts joins the room. \
                 Started from Steam, it is the plain game."
                    .to_owned()
            }
            (None, _) => "Create or join a room, then start the game from here.".to_owned(),
            (Some(_), World::Fetching) => "Receiving the room's world…".to_owned(),
            (Some(_), World::Loading) => "The game is loading the world.".to_owned(),
            (Some(_), World::Playing) => {
                let step = game
                    .step
                    .map(|step| format!(", step {step}"))
                    .unwrap_or_default();
                let speed = if game.speed == 0 {
                    " (paused)".to_owned()
                } else {
                    format!(" at {}×", f64::from(game.speed) / 100.0)
                };
                format!("Playing{step}{speed}.")
            }
            (Some(build), World::None) => {
                format!("The game is connected ({build}). The world loads when the room starts.")
            }
        };
        ui.label(line);
        if state.installed.is_some() && installed_mod.is_none() {
            ui.label(
                RichText::new(
                    "Run INSTALL_TPF3MP.cmd (Windows) or ./install.sh from the TPF3-MP \
                     folder once, to put the TPF3-MP mod in the game's mods folder.",
                )
                .weak()
                .small(),
            );
        }
        if game.world == World::Fetching && game.total > 0 {
            #[allow(clippy::cast_precision_loss)]
            let done = game.bytes as f32 / game.total as f32;
            ui.add(ProgressBar::new(done.clamp(0.0, 1.0)).show_percentage());
        }
        if state.room.is_some() && game.attached.is_none() {
            ui.add_space(6.0);
            theme::big_button(ui, true, "Start Transport Fever 3").clicked()
        } else {
            false
        }
    })
}

/// A folder, drawn: the TPF2 launcher's sign for the game's folder.
fn folder(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(22.0, 18.0), Sense::hover());
    let painter = ui.painter();
    let stroke = Stroke::new(1.3, theme::MUTED);
    let body = egui::Rect::from_min_max(rect.min + vec2(1.0, 4.0), rect.max - vec2(1.0, 1.0));
    painter.rect_stroke(body, 2.0, stroke, egui::StrokeKind::Inside);
    painter.line_segment([body.left_top(), body.left_top() + vec2(7.0, -3.0)], stroke);
    painter.line_segment(
        [
            body.left_top() + vec2(7.0, -3.0),
            body.left_top() + vec2(10.0, 0.0),
        ],
        stroke,
    );
}

/// The server's version and the player's support ID, right to left, with
/// a button to copy the ID.
fn server_details(ui: &mut Ui, state: &State) {
    if let Some(support) = &state.support_id {
        if theme::small_button(ui, true, "Copy", Kind::Ghost)
            .on_hover_text("Copy the support ID")
            .clicked()
        {
            ui.ctx().copy_text(support.clone());
        }
        ui.label(RichText::new(format!("support ID {support}")).weak())
            .on_hover_text("Quote this to the server's operator when something goes wrong");
    }
    if let Some(version) = &state.server_version {
        ui.label(RichText::new(format!("server {version}")).weak());
    }
}

/// The line under the game's name.
fn byline() -> String {
    format!(
        "Unofficial multiplayer · TPF3-MP {}",
        env!("CARGO_PKG_VERSION")
    )
}

/// The updater's state, one line in the footer.
fn update_line(ui: &mut Ui, updater: &Updater) {
    let text = match updater.state() {
        UpdateState::Checking => "checking for updates…".to_owned(),
        UpdateState::UpToDate => "up to date".to_owned(),
        UpdateState::Off(reason) => format!("updates off: {reason}"),
        UpdateState::Failed(reason) => format!("update check failed: {reason}"),
        UpdateState::Downloading { version, .. } => format!("downloading {version}…"),
        UpdateState::Ready { version } => format!("{version} is ready"),
        UpdateState::Installing { version } => format!("installing {version}…"),
    };
    ui.label(RichText::new(text).weak().small());
    if matches!(
        updater.state(),
        UpdateState::UpToDate | UpdateState::Failed(_)
    ) && theme::small_button(ui, true, "Check for updates", Kind::Ghost).clicked()
    {
        updater.check();
    }
}

/// Offers a downloaded update. Installing restarts the launcher, which
/// would drop a game in progress, so the player chooses when.
fn update_banner(ui: &mut Ui, updater: &Updater, state: &State) {
    let UpdateState::Ready { version } = updater.state() else {
        return;
    };
    theme::banner(ui, theme::ACCENT, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!("TPF3-MP {version} is ready to install."))
                    .strong()
                    .color(theme::ACCENT),
            );
            if state.room.is_some() {
                ui.label("It installs when you leave the room, or the next time you start.");
            } else if theme::button(ui, true, "Restart and update", Kind::Primary).clicked() {
                updater.install_and_restart(ui.ctx());
            }
        });
    });
}

/// The steps to a game, as TPF2MP's launcher shows them, and where each
/// stands.
fn steps(state: &State) -> Vec<(&'static str, Step)> {
    let room = state.room.as_ref();
    let running = room.is_some_and(|room| room.phase == Phase::Running);
    let done = [
        state.connection == Connection::Connected,
        room.is_some(),
        state.game.attached.is_some(),
        running
            || room.is_some_and(|room| {
                !room.members.is_empty() && room.members.iter().all(|member| member.ready)
            }),
        running,
    ];
    let labels = [
        // Not "Connect", nor "Start Transport Fever 3": those are buttons.
        "Connect to a server",
        "Create or join a room",
        "Start the game from here",
        "Everyone ready",
        "Play together",
    ];
    labels.into_iter().zip(Step::of(&done)).collect()
}

/// The TPF3-MP version whose mod the install scripts put in place, from
/// their record in TPF3-MP's data folder `dir` (`installed.json` from
/// Windows's, `installed.txt` from Linux's and macOS's), while the mod is
/// still where the record says.
pub fn installed_mod(dir: &Path) -> Option<String> {
    let (version, folder) = if let Ok(text) = std::fs::read_to_string(dir.join("installed.json")) {
        let record: serde_json::Value =
            serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
        (
            record.get("version")?.as_str()?.to_owned(),
            record.get("mod")?.as_str()?.to_owned(),
        )
    } else {
        let text = std::fs::read_to_string(dir.join("installed.txt")).ok()?;
        let field = |name: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(name))
                .map(str::to_owned)
        };
        (field("version=")?, field("mod=")?)
    };
    Path::new(&folder)
        .join("mod.lua")
        .is_file()
        .then_some(version)
}

fn non_empty(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::installed_mod;

    #[test]
    fn the_version_the_install_scripts_recorded_is_read() {
        let dir = std::env::temp_dir().join(format!("tpf3mp-installed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mods = dir.join("mods").join("tpf3mp_1");
        std::fs::create_dir_all(&mods).unwrap();
        std::fs::write(mods.join("mod.lua"), "-- mod").unwrap();
        assert_eq!(installed_mod(&dir), None, "nothing installed");

        // install.sh's record.
        std::fs::write(
            dir.join("installed.txt"),
            format!("version=0.1.0\nmod={}\n", mods.display()),
        )
        .unwrap();
        assert_eq!(installed_mod(&dir).as_deref(), Some("0.1.0"));

        // install.ps1's, which Windows tools may start with a byte order mark.
        let json = serde_json::json!({ "version": "0.2.0", "mod": mods });
        std::fs::write(dir.join("installed.json"), format!("\u{feff}{json}")).unwrap();
        assert_eq!(installed_mod(&dir).as_deref(), Some("0.2.0"));

        // The mod taken away since.
        std::fs::remove_file(mods.join("mod.lua")).unwrap();
        assert_eq!(installed_mod(&dir), None, "the mod is gone");

        std::fs::write(dir.join("installed.json"), "damaged").unwrap();
        assert_eq!(installed_mod(&dir), None, "a damaged record");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

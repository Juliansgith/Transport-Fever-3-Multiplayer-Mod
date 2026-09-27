//! The launcher's window: the same steps as `docs/PLAYING.md` describes,
//! connect, create or join a room, get ready, play, drawn from the
//! launcher's [`State`] on every frame.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Align, ComboBox, Frame, Id, Layout, Margin, Modal, ProgressBar, RichText, ScrollArea,
    TextEdit, Ui,
};
use tpf3mp_agent::launcher::{
    Action, Connection, Differences, MemberContent, Phase, Room, State, World,
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
        }
        let state = self.backend.state();
        if !self.offered {
            // The server and name remembered from last time, or given.
            self.server = state.server.clone().unwrap_or_default();
            self.name = state.name.clone();
            self.offered = true;
        }
        self.guard_quit(ui.ctx(), &state);
        let edge = Frame::new()
            .fill(theme::BG)
            .inner_margin(Margin::symmetric(theme::GUTTER, 10));
        egui::Panel::top("header")
            .frame(edge)
            .show(ui, |ui| self.header(ui, &state));
        egui::Panel::bottom("footer")
            .frame(edge)
            .show(ui, |ui| self.footer(ui, &state));
        let body = Frame::new()
            .fill(theme::BG)
            .inner_margin(Margin::symmetric(theme::GUTTER, 14));
        egui::CentralPanel::default_margins()
            .frame(body)
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.body(ui, &state));
            });
        self.dialogs(ui.ctx(), &state);
        ui.ctx().request_repaint_after(REFRESH);
    }

    fn header(&mut self, ui: &mut Ui, state: &State) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("TPF3-MP")
                    .size(22.0)
                    .strong()
                    .color(theme::TEXT),
            );
            ui.label(RichText::new("Multiplayer for Transport Fever 3").color(theme::MUTED));
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
            });
        });
        ui.horizontal_wrapped(|ui| {
            let who = match &state.player {
                Some(player) => format!("{} ({player})", state.name),
                None => state.name.clone(),
            };
            ui.label(RichText::new(who).weak());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(version) = &state.server_version {
                    ui.label(RichText::new(format!("server {version}")).weak());
                }
                if let Some(support) = &state.support_id {
                    if theme::small_button(ui, true, "Copy", Kind::Ghost)
                        .on_hover_text("Copy the support ID")
                        .clicked()
                    {
                        ui.ctx().copy_text(support.clone());
                    }
                    ui.label(RichText::new(format!("support ID {support}")).weak())
                        .on_hover_text(
                            "Quote this to the server's operator when something goes wrong",
                        );
                }
            });
        });
    }

    fn footer(&mut self, ui: &mut Ui, state: &State) {
        ui.horizontal(|ui| {
            if let Some(logs) = &self.extras.logs
                && theme::button(ui, true, "Open logs folder", Kind::Ghost)
                    .on_hover_text(logs.display().to_string())
                    .clicked()
            {
                tpf3mp_agent::launcher::setup::open_folder(logs);
            }
            if self.extras.collect.is_some() {
                let collecting = self.collecting();
                let busy = collecting == Collecting::Busy;
                if theme::button(ui, !busy, "Collect logs", Kind::Ghost)
                    .on_hover_text(
                        "Put TPF3-MP's logs and the game's into one zip for a bug report.                          Keys and tokens are never included.",
                    )
                    .clicked()
                {
                    self.collect_logs(ui.ctx(), state.support_id.clone());
                }
                match collecting {
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
                        ui.label(RichText::new("cannot collect the logs").color(ui.visuals().error_fg_color))
                            .on_hover_text(error);
                    }
                }
            }
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
            if let Some(updater) = &self.extras.updater {
                update_line(ui, updater);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(format!("TPF3-MP {}", env!("CARGO_PKG_VERSION"))).weak());
            });
        });
    }

    fn body(&mut self, ui: &mut Ui, state: &State) {
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
        checklist(ui, state);
        let connected = state.connection == Connection::Connected;
        match &state.room {
            Some(room) => self.room(ui, state, room),
            None if connected => self.lobby(ui, state),
            None => self.connect(ui, state),
        }
        let installed_mod = self.installed_mod(state);
        if game(ui, state, installed_mod.as_deref()) {
            self.backend.act(Action::LaunchGame);
        }
        if state.room.is_some() {
            self.chat(ui, state);
        }
        if !state.notices.is_empty() {
            theme::card(ui, "Session log", |ui| {
                let start = state.notices.len().saturating_sub(NOTICES_SHOWN);
                theme::log_box(
                    ui,
                    "session-log",
                    state.notices[start..].iter().map(String::as_str),
                    140.0,
                );
            });
        }
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

    fn connect(&mut self, ui: &mut Ui, state: &State) {
        theme::card(ui, "Server", |ui| {
            let mut submit = false;
            egui::Grid::new("connect")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    submit |= field(
                        ui,
                        "Server",
                        TextEdit::singleline(&mut self.server)
                            .hint_text("server:29470, or a whole invite")
                            .desired_width(360.0),
                    );
                    ui.end_row();
                    submit |= field(
                        ui,
                        "Your name",
                        TextEdit::singleline(&mut self.name)
                            .char_limit(32)
                            .desired_width(200.0),
                    );
                    ui.end_row();
                });
            let connecting = state.connection == Connection::Connecting || self.backend.busy();
            ui.horizontal(|ui| {
                let clicked = theme::button(ui, !connecting, "Connect", Kind::Primary).clicked();
                if connecting {
                    ui.spinner();
                }
                if (clicked || submit) && !connecting {
                    self.backend.act(Action::Connect {
                        server: self.server.clone(),
                        name: self.name.clone(),
                    });
                }
            });
            ui.label(
                RichText::new(
                    "Got an invite? Paste all of it as the server: you connect and join in one step.",
                )
                .weak()
                .small(),
            );
        });
    }

    fn lobby(&mut self, ui: &mut Ui, state: &State) {
        let busy = self.backend.busy();
        theme::card(ui, "Create a room", |ui| {
            egui::Grid::new("create")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    field(
                        ui,
                        "Room name",
                        TextEdit::singleline(&mut self.room_name)
                            .hint_text("TPF3-MP room")
                            .char_limit(48)
                            .desired_width(260.0),
                    );
                    ui.end_row();
                    ui.label("Players");
                    ComboBox::from_id_salt("max-players")
                        .selected_text(self.max_players.to_string())
                        .show_ui(ui, |ui| {
                            for size in ROOM_SIZES {
                                ui.selectable_value(&mut self.max_players, size, size.to_string());
                            }
                        });
                    ui.end_row();
                    field(
                        ui,
                        "Password",
                        TextEdit::singleline(&mut self.create_password)
                            .hint_text("optional")
                            .password(true)
                            .char_limit(64)
                            .desired_width(200.0),
                    );
                    ui.end_row();
                    if !state.rules.is_empty() {
                        let chosen = self
                            .rules
                            .clone()
                            .filter(|name| state.rules.iter().any(|rules| rules.name == *name))
                            .unwrap_or_else(|| state.rules[0].name.clone());
                        ui.label("Rules");
                        let mut picked = chosen.clone();
                        ComboBox::from_id_salt("rules")
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
                        self.rules = Some(picked.clone());
                        ui.end_row();
                        if let Some(rules) = state.rules.iter().find(|rules| rules.name == picked) {
                            ui.label("");
                            ui.label(RichText::new(&rules.description).weak().small());
                            ui.end_row();
                        }
                    }
                });
            if theme::button(ui, !busy, "Create room", Kind::Primary).clicked() {
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
        theme::card(ui, "Join a room", |ui| {
            let mut submit = false;
            egui::Grid::new("join")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    submit |= field(
                        ui,
                        "Invite",
                        TextEdit::singleline(&mut self.invite)
                            .hint_text("paste the invite you were sent")
                            .desired_width(360.0),
                    );
                    ui.end_row();
                    submit |= field(
                        ui,
                        "Room password",
                        TextEdit::singleline(&mut self.join_password)
                            .hint_text("if it has one")
                            .password(true)
                            .char_limit(64)
                            .desired_width(200.0),
                    );
                    ui.end_row();
                });
            let clicked = theme::button(ui, !busy, "Join room", Kind::Primary).clicked();
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
        theme::card(ui, "Room", |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(&room.name)
                        .size(18.0)
                        .strong()
                        .color(theme::TEXT),
                );
                theme::pill(ui, &format!("{} rules", room.rules), theme::INFO);
                match room.phase {
                    Phase::Lobby => theme::pill(ui, "lobby", theme::MUTED),
                    Phase::Running => theme::pill(ui, "game running", theme::SUCCESS),
                };
            });
            ui.add_space(4.0);
            if let Some(invite) = &room.invite {
                ui.horizontal(|ui| {
                    let mut shown = invite.clone();
                    ui.add(
                        TextEdit::singleline(&mut shown)
                            .desired_width(420.0)
                            .interactive(false),
                    )
                    .on_hover_text("Send this to your friends");
                    if theme::button(ui, true, "Copy invite", Kind::Secondary).clicked() {
                        ui.ctx().copy_text(invite.clone());
                    }
                });
                ui.add_space(6.0);
            }
            egui::Grid::new("members")
                .num_columns(5)
                .striped(true)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    for heading in ["Player", "Platform", "Game and mods", "Ready", ""] {
                        ui.label(RichText::new(heading).weak());
                    }
                    ui.end_row();
                    for member in &room.members {
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
                        ui.label(name);
                        ui.label(&member.platform);
                        if member.owner {
                            ui.label("the room's");
                        } else {
                            match member.content {
                                MemberContent::Same => ui.label("same as the owner's"),
                                MemberContent::Differs => {
                                    ui.label(RichText::new("differ").color(theme::WARNING))
                                }
                                MemberContent::Unknown => ui.label("–"),
                            };
                        }
                        match (room.phase, member.ready) {
                            (Phase::Lobby, true) => {
                                ui.label(RichText::new("ready").strong().color(theme::SUCCESS))
                            }
                            (Phase::Lobby, false) => ui.label("–"),
                            (Phase::Running, _) => ui.label(""),
                        };
                        if room.you_own && !member.you {
                            if theme::small_button(ui, true, "Remove", Kind::Danger).clicked() {
                                self.confirm = Some(Confirm::Kick {
                                    id: member.id.clone(),
                                    name: member.name.clone(),
                                });
                            }
                        } else {
                            ui.label("");
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(8.0);
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

    fn chat(&mut self, ui: &mut Ui, state: &State) {
        theme::card(ui, "Chat", |ui| {
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
                let response = ui
                    .add(
                        TextEdit::singleline(&mut self.chat)
                            .hint_text("say something to the room")
                            .char_limit(280)
                            .desired_width(380.0),
                    )
                    .labelled_by(label.id);
                let entered =
                    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                let send = theme::button(ui, true, "Send", Kind::Secondary).clicked();
                if (send || entered) && !self.chat.trim().is_empty() {
                    self.backend.act(Action::Chat {
                        text: std::mem::take(&mut self.chat),
                    });
                    response.request_focus();
                }
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
    let response = ui.add(edit).labelled_by(label.id);
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
                 Change yours to match, then start the game and the launcher again.",
            )
            .weak()
            .small(),
        );
    });
}

/// Where the game stands: found, its mod installed, connected. Returns
/// whether the player asked to start it: only a game started here runs
/// TPF3-MP (D11).
fn game(ui: &mut Ui, state: &State, installed_mod: Option<&str>) -> bool {
    theme::card(ui, "Game", |ui| {
        let game = &state.game;
        ui.horizontal_wrapped(|ui| {
            match &state.installed {
                Some(_) => theme::pill(ui, "found in Steam", theme::SUCCESS),
                None => theme::pill(ui, "not found in Steam", theme::WARNING),
            };
            // No empty placeholder: an empty label breaks the wrapped row,
            // and the next pill lands on this one.
            if let Some(version) = installed_mod {
                theme::pill(ui, &format!("TPF3-MP {version} installed"), theme::SUCCESS);
            } else if state.installed.is_some() {
                theme::pill(ui, "TPF3-MP not installed", theme::WARNING);
            }
            match (&game.attached, game.world) {
                (None, _) => theme::pill(ui, "waiting for the game", theme::MUTED),
                (Some(_), World::Playing) => theme::pill(ui, "playing", theme::SUCCESS),
                (Some(_), World::Fetching | World::Loading) => {
                    theme::pill(ui, "loading the world", theme::INFO)
                }
                (Some(_), World::None) => theme::pill(ui, "game connected", theme::SUCCESS),
            };
        });
        if let Some(installed) = &state.installed {
            ui.label(
                RichText::new(format!(
                    "Transport Fever 3, Steam build {}, in {}",
                    installed.build, installed.dir
                ))
                .weak()
                .small(),
            );
            if installed_mod.is_none() {
                ui.label(
                    RichText::new(
                        "Run INSTALL_TPF3MP.cmd (Windows) or ./install.sh from the TPF3-MP \
                         folder once, to put the TPF3-MP mod in the game's mods folder.",
                    )
                    .weak()
                    .small(),
                );
            }
        } else {
            ui.label(
                RichText::new("Transport Fever 3 was not found in Steam.")
                    .weak()
                    .small(),
            );
        }
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
        if game.world == World::Fetching && game.total > 0 {
            #[allow(clippy::cast_precision_loss)]
            let done = game.bytes as f32 / game.total as f32;
            ui.add(ProgressBar::new(done.clamp(0.0, 1.0)).show_percentage());
        }
        state.room.is_some()
            && game.attached.is_none()
            && theme::button(ui, true, "Start Transport Fever 3", Kind::Primary).clicked()
    })
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

/// The steps to a game, as TPF2MP's launcher shows them.
fn checklist(ui: &mut Ui, state: &State) {
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
    theme::card(ui, "Checklist", |ui| {
        let steps: Vec<(&str, Step)> = labels.into_iter().zip(Step::of(&done)).collect();
        theme::checklist(ui, &steps);
    });
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

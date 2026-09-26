//! The launcher's window: the same steps as `docs/PLAYING.md` describes,
//! connect, create or join a room, get ready, play, drawn from the
//! launcher's [`State`] on every frame.

use std::{path::PathBuf, time::Duration};

use eframe::egui::{
    self, Align, Color32, ComboBox, Frame, Id, Layout, Modal, ProgressBar, RichText, ScrollArea,
    Stroke, TextEdit, Ui,
};
use tpf3mp_agent::launcher::{
    Action, Connection, Differences, MemberContent, Phase, Room, State, World,
};

use crate::{
    backend::Backend,
    update::{UpdateState, Updater},
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
/// Notices shown, newest last.
const NOTICES_SHOWN: usize = 8;

/// What the window needs besides the launcher.
pub struct Extras {
    /// Where the log files are, for "Open logs folder".
    pub logs: Option<PathBuf>,
    /// Checks for and installs new versions; `None` in tests.
    pub updater: Option<Updater>,
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
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Draws the window once.
    pub fn show(&mut self, ui: &mut Ui) {
        let state = self.backend.state();
        if !self.offered {
            // The server and name remembered from last time, or given.
            self.server = state.server.clone().unwrap_or_default();
            self.name = state.name.clone();
            self.offered = true;
        }
        self.guard_quit(ui.ctx(), &state);
        egui::Panel::top("header").show(ui, |ui| self.header(ui, &state));
        egui::Panel::bottom("footer").show(ui, |ui| self.footer(ui));
        egui::CentralPanel::default_margins().show(ui, |ui| {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| self.body(ui, &state));
        });
        self.dialogs(ui.ctx(), &state);
        ui.ctx().request_repaint_after(REFRESH);
    }

    fn header(&mut self, ui: &mut Ui, state: &State) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("TPF3-MP").strong().size(18.0));
            let (text, color) = match state.connection {
                Connection::Connected if state.tunneled => {
                    ("connected via tunnel", ui.visuals().warn_fg_color)
                }
                Connection::Connected => ("connected", Color32::from_rgb(63, 185, 80)),
                Connection::Connecting => ("connecting…", ui.visuals().warn_fg_color),
                Connection::Disconnected => ("disconnected", ui.visuals().weak_text_color()),
            };
            Frame::new()
                .stroke(Stroke::new(1.0, color))
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(8, 2))
                .show(ui, |ui| ui.label(RichText::new(text).color(color).small()));
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
                    if ui
                        .small_button("Copy")
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

    fn footer(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if let Some(logs) = &self.extras.logs
                && ui
                    .button("Open logs folder")
                    .on_hover_text(logs.display().to_string())
                    .clicked()
            {
                tpf3mp_agent::launcher::setup::open_folder(logs);
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
        if let Some(error) = &state.error {
            notice_frame(ui, ui.visuals().error_fg_color, |ui| {
                ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
            });
        }
        if let Some(diff) = &state.content_diff {
            differences(ui, diff);
        }
        let connected = state.connection == Connection::Connected;
        match &state.room {
            Some(room) => self.room(ui, state, room),
            None if connected => self.lobby(ui, state),
            None => self.connect(ui, state),
        }
        if state.room.is_some() {
            game(ui, state);
            self.chat(ui, state);
        }
        if !state.notices.is_empty() {
            section(ui, "Notices", |ui| {
                let start = state.notices.len().saturating_sub(NOTICES_SHOWN);
                for notice in &state.notices[start..] {
                    ui.label(RichText::new(notice).weak());
                }
            });
        }
    }

    fn connect(&mut self, ui: &mut Ui, state: &State) {
        section(ui, "Server", |ui| {
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
                let clicked = ui
                    .add_enabled(!connecting, egui::Button::new("Connect"))
                    .clicked();
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
        section(ui, "Create a room", |ui| {
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
            if ui
                .add_enabled(!busy, egui::Button::new("Create room"))
                .clicked()
            {
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
        section(ui, "Join a room", |ui| {
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
            let clicked = ui
                .add_enabled(!busy, egui::Button::new("Join room"))
                .clicked();
            if (clicked || submit) && !busy {
                self.backend.act(Action::Join {
                    invite: self.invite.clone(),
                    password: non_empty(&self.join_password),
                });
            }
        });
        ui.add_space(4.0);
        if ui
            .add_enabled(!busy, egui::Button::new("Disconnect"))
            .clicked()
        {
            self.backend.act(Action::Disconnect);
        }
    }

    fn room(&mut self, ui: &mut Ui, state: &State, room: &Room) {
        let phase = match room.phase {
            Phase::Lobby => "lobby",
            Phase::Running => "game running",
        };
        let title = format!("{} · {} rules · {phase}", room.name, room.rules);
        let busy = self.backend.busy();
        section(ui, &title, |ui| {
            if let Some(invite) = &room.invite {
                ui.horizontal(|ui| {
                    let mut shown = invite.clone();
                    ui.add(
                        TextEdit::singleline(&mut shown)
                            .desired_width(420.0)
                            .interactive(false),
                    )
                    .on_hover_text("Send this to your friends");
                    if ui.button("Copy invite").clicked() {
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
                                MemberContent::Differs => ui.label(
                                    RichText::new("differ").color(ui.visuals().warn_fg_color),
                                ),
                                MemberContent::Unknown => ui.label("–"),
                            };
                        }
                        match (room.phase, member.ready) {
                            (Phase::Lobby, true) => ui.label(
                                RichText::new("ready").color(Color32::from_rgb(63, 185, 80)),
                            ),
                            (Phase::Lobby, false) => ui.label("–"),
                            (Phase::Running, _) => ui.label(""),
                        };
                        if room.you_own && !member.you {
                            if ui.small_button("Remove").clicked() {
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
                        let label = if you_ready { "Not ready" } else { "Ready" };
                        if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                            self.backend.act(Action::Ready { ready: !you_ready });
                        }
                        if room.you_own {
                            let everyone = room.members.iter().all(|member| member.ready);
                            if ui
                                .add_enabled(everyone && !busy, egui::Button::new("Start game"))
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
                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(
                                RichText::new("Leave room").color(ui.visuals().error_fg_color),
                            ),
                        )
                        .clicked()
                    {
                        self.confirm = Some(Confirm::Leave);
                    }
                });
            });
        });
    }

    fn chat(&mut self, ui: &mut Ui, state: &State) {
        section(ui, "Chat", |ui| {
            ScrollArea::vertical()
                .id_salt("chat-log")
                .max_height(160.0)
                .stick_to_bottom(true)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for line in &state.chat {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(format!("{}:", line.from)).strong());
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
                if (ui.button("Send").clicked() || entered) && !self.chat.trim().is_empty() {
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
                if ui.button(yes).clicked() {
                    answer = Some(true);
                }
                if ui.button("Cancel").clicked() {
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

/// A titled group.
fn section(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    Frame::group(ui.style()).inner_margin(12).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).strong().size(15.0));
        ui.add_space(6.0);
        add(ui);
    });
    ui.add_space(8.0);
}

/// A labelled text field; returns whether Enter was pressed in it.
fn field(ui: &mut Ui, label: &str, edit: TextEdit<'_>) -> bool {
    let label = ui.label(label);
    let response = ui.add(edit).labelled_by(label.id);
    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
}

fn notice_frame(ui: &mut Ui, color: Color32, add: impl FnOnce(&mut Ui)) {
    Frame::new()
        .stroke(Stroke::new(1.0, color))
        .corner_radius(6)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    ui.add_space(8.0);
}

/// What to change for this player's game to match the room's.
fn differences(ui: &mut Ui, diff: &Differences) {
    let warn = ui.visuals().warn_fg_color;
    notice_frame(ui, warn, |ui| {
        ui.label(
            RichText::new("Your game differs from the room's")
                .strong()
                .color(warn),
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

/// Where the game stands.
fn game(ui: &mut Ui, state: &State) {
    section(ui, "Game", |ui| {
        let game = &state.game;
        let line = match (&game.attached, game.world) {
            (None, _) => {
                "Waiting for the game: start Transport Fever 3 with the TPF3-MP mod.".to_owned()
            }
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
    });
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
    ) && ui.small_button("Check for updates").clicked()
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
    let accent = ui.visuals().hyperlink_color;
    notice_frame(ui, accent, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!("TPF3-MP {version} is ready to install."))
                    .strong()
                    .color(accent),
            );
            if state.room.is_some() {
                ui.label("It installs when you leave the room, or the next time you start.");
            } else if ui.button("Restart and update").clicked() {
                updater.install_and_restart(ui.ctx());
            }
        });
    });
}

fn non_empty(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

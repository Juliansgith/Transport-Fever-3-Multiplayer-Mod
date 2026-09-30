//! The step gate in the game: what the detour on the game's simulation step
//! does each time the game calls it.
//!
//! Transport Fever 3's `GameSim::Step` (as TPF2's) is one batch of the
//! game's own pacing: the main thread calls it on its schedule, and it runs
//! as many simulation updates as its call of the speed getter answers: one
//! at 1x, none while paused, which takes the game's own paused path. The
//! renderer interpolates from each batch, so every call must run: a call
//! skipped looks to the game like a batch that ran without the world moving
//! on, and its clock goes back (TF3 then fails an assertion in its
//! particles). So the detour runs the game's step exactly once per call, and
//! [`StepDriver::on_step`] says how many updates that call runs:
//!
//! - as many as the room has released, up to [`MAX_STEPS_PER_CALL`] and to
//!   the next checkpoint step, so a room faster than the game's own pace
//!   catches up;
//! - none while the room withholds the next step (the room is paused, or a
//!   player is behind): the game's paused path, and the world stands still;
//! - before a room has begun a game, and after it has ended, what the
//!   game's own speed says; before, each new world the player's game has up
//!   is told to the agent, which marks the player ready in the room's lobby
//!   ([`RoomGate::world_up`]);
//! - while the game saves its world for the room, or loads the room's
//!   (docs/HOOKS.md, "The room's world"), none;
//! - on anything it cannot follow (the agent gone, a malformed message, a
//!   room's world that did not load) none, for good: the world stands still
//!   rather than run on apart from the room's (fail closed).
//!
//! Answering the step's own speed call rather than skipping calls is how
//! TPF2MP paced TPF2 (`tpf2-multiplayer/native/src/speedhook.cpp`, which
//! also describes the game's batch pacing).
//!
//! A batch that ends at a checkpoint step asks the game for the world's
//! lanes: the mod's game script reads them after the batch's last update
//! (docs/HOOKS.md, "The world's lanes"), and the driver reports their
//! digests for that step. A batch that does not bring them holds the world.
//!
//! The room's actions travel the same way (docs/HOOKS.md, "Actions in the
//! game"). The session ends a batch before every step the room ordered
//! actions for, so such a step is always the first update of a batch, and
//! the detour hands the batch its actions: the mod's game script applies
//! them in that update ([`crate::lua`]). If they were not applied, the
//! world has run the room's step without them, and the driver holds it. The
//! actions the player hands over go to the room from here as well, and only
//! in the room's game.
//!
//! This module knows nothing of the process: the detour hands it the
//! game's step as a closure, and the room's side is a [`RoomGate`], the
//! real [`Session`] in the game and a script in the tests.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use ring::digest::{SHA256, digest};
use tpf3mp_bridge::{Begin, Game, Notice, SaveOrder, Session, SessionError, StepGate};
use tpf3mp_proto::{
    ChatText, Event, EventBody, FixedBytes, LaneDigest, Payload, PlayerId, Speed, action::Action,
};

/// Most steps one call of the game's step runs, catching up with the room:
/// at the game's 1x (5 calls a second), rooms up to 16x keep up. The game
/// itself runs up to 64 in a call (its debug steps).
pub const MAX_STEPS_PER_CALL: u32 = 16;

/// How many updates one call of the game's step runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Updates {
    /// What the game's own speed says: no room's game.
    Own,
    /// Exactly this many; 0 is the game's paused path.
    Exactly(u32),
}

/// The room's side of the gate: what the detour needs of a [`Session`].
pub trait RoomGate {
    fn try_begin(&mut self) -> Result<Option<Begin>, SessionError>;
    fn poll_step(&mut self, game: &mut HookGame) -> Result<StepGate, SessionError>;
    /// The step the game runs next.
    fn next_step(&self) -> u64;
    /// After `poll_step` said Run: the steps that may run as one batch.
    fn batch(&mut self, game: &mut HookGame, max: u32) -> Result<u32, SessionError>;
    fn after_step(&mut self, game: &mut HookGame) -> Result<u64, SessionError>;
    fn loaded(&mut self, next_step: u64) -> Result<(), SessionError>;
    fn request_speed(&mut self, speed: Speed) -> Result<(), SessionError>;
    fn command(&mut self, payload: Payload) -> Result<u64, SessionError>;
    /// Says `text` to the room for the player.
    fn chat(&mut self, text: ChatText) -> Result<(), SessionError>;
    /// Before the room begins: the game's world number `world` is up. Says
    /// whether the agent was told.
    fn world_up(&mut self, world: u64) -> Result<bool, SessionError>;
    fn saved(
        &mut self,
        game: &mut HookGame,
        outcome: Result<(), String>,
    ) -> Result<(), SessionError>;
}

/// Longest a save the room ordered may take the game before it is reported
/// as failed.
pub const SAVE_PATIENCE: Duration = Duration::from_secs(120);
/// Longest a load of the room's save may take before the world is held.
pub const LOAD_PATIENCE: Duration = Duration::from_secs(600);

/// What the driver asks of the game beyond its step: saving and loading
/// whole worlds, which the game's GUI does (the mod, through
/// [`crate::lua`], and [`crate::worlds`] for the files).
pub trait GameControl: Send {
    /// Asks the game to save its world under `name`.
    fn request_save(&mut self, name: &str);
    /// The last save's outcome, once the game has one: the file written, or
    /// why not.
    fn save_result(&mut self) -> Option<Result<PathBuf, String>>;
    /// Asks the game to load the save `file` (the room's world).
    fn request_load(&mut self, file: &Path) -> Result<(), String>;
    /// Whether the world the last load asked for is up. Once.
    fn load_done(&mut self) -> bool;
    /// Tells the game's Multiplayer window what the room said: the room,
    /// its speed, its chat, a divergence, the game's end.
    fn room_notice(&mut self, notice: &Notice);
    /// Tells the game's Multiplayer window which player is this game's.
    fn set_me(&mut self, player: PlayerId);
    /// The number of a world whose GUI started with the mod linked, since
    /// the last call, if one did: the latest. Once.
    fn world_up(&mut self) -> Option<u64>;
}

/// A save the game is making for the room.
#[derive(Debug)]
struct Saving {
    event: u64,
    since: Instant,
}

/// A load of the room's save the game is making.
#[derive(Debug)]
struct Loading {
    next_step: u64,
    since: Instant,
}

impl RoomGate for Session {
    fn try_begin(&mut self) -> Result<Option<Begin>, SessionError> {
        Session::try_begin(self)
    }
    fn poll_step(&mut self, game: &mut HookGame) -> Result<StepGate, SessionError> {
        Session::poll_step(self, game)
    }
    fn next_step(&self) -> u64 {
        Session::next_step(self)
    }
    fn batch(&mut self, game: &mut HookGame, max: u32) -> Result<u32, SessionError> {
        Session::batch(self, game, max)
    }
    fn after_step(&mut self, game: &mut HookGame) -> Result<u64, SessionError> {
        Session::after_step(self, game)
    }
    fn loaded(&mut self, next_step: u64) -> Result<(), SessionError> {
        Session::loaded(self, next_step)
    }
    fn request_speed(&mut self, speed: Speed) -> Result<(), SessionError> {
        Session::request_speed(self, speed)
    }
    fn command(&mut self, payload: Payload) -> Result<u64, SessionError> {
        Session::command(self, payload)
    }
    fn chat(&mut self, text: ChatText) -> Result<(), SessionError> {
        Session::chat(self, text)
    }
    fn world_up(&mut self, world: u64) -> Result<bool, SessionError> {
        Session::world_up(self, world)
    }
    fn saved(
        &mut self,
        game: &mut HookGame,
        outcome: Result<(), String>,
    ) -> Result<(), SessionError> {
        Session::saved(self, game, outcome)
    }
}

/// The game, as the session sees it. It keeps the actions the room orders
/// until the driver hands them to the game at their step. Lanes and saving
/// come next; until then it reports no lanes and refuses to save.
#[derive(Debug, Default)]
pub struct HookGame {
    pub events: u64,
    pub notices: Vec<String>,
    /// The local player, from the room's `Begin`: the room's events name it
    /// as the actor of the player's own commands.
    pub me: Option<PlayerId>,
    /// The actions the room ordered for the next step to run, in order,
    /// each with its client sequence number when the local player sent it.
    pub actions: Vec<(Action, Option<u64>)>,
    /// The player's commands the room refused: their sequence numbers, and
    /// why.
    pub refused: Vec<(u64, String)>,
    /// An event the game cannot follow.
    pub fault: Option<String>,
    /// The world's lanes after the batch that ran last, when it ended at a
    /// checkpoint step: what the session reports for that step.
    pub lanes: Option<Vec<LaneDigest>>,
    /// What the room said for the game's Multiplayer window, which the
    /// driver hands on (`GameControl::room_notice`).
    pub window: Vec<Notice>,
}

impl Game for HookGame {
    fn apply(&mut self, event: &Event) {
        self.events += 1;
        if let EventBody::Command {
            player,
            client_seq,
            payload,
        } = &event.body
        {
            let own = (self.me == Some(*player)).then_some(*client_seq);
            match Action::from_payload(payload) {
                Ok(action) => self.actions.push((action, own)),
                Err(error) => {
                    self.fault.get_or_insert(format!(
                        "the room ordered an action this game cannot read (event {}): {error}",
                        event.seq
                    ));
                }
            }
        }
    }

    fn lanes(&mut self) -> Vec<LaneDigest> {
        self.lanes.take().unwrap_or_default()
    }

    fn save(&mut self, _file: &std::path::Path) -> Result<(), String> {
        // Only `Session::before_step` saves here; the driver polls, and
        // saves at `StepGate::Save`, through the GUI.
        Err("the hook saves the world through the game's GUI, not here".into())
    }

    fn notice(&mut self, notice: Notice) {
        if let Notice::Refused { command, reason } = &notice {
            self.refused.push((*command, format!("{reason:?}")));
        }
        self.window.push(notice.clone());
        // The room and its chat are for the Multiplayer window, not the log.
        if !matches!(notice, Notice::Room(_) | Notice::Chat { .. }) {
            self.notices.push(format!("{notice:?}"));
        }
    }
}

/// An action the room ordered, and, for one of the player's own, the ticket
/// the mod was given when it handed the action over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ordered {
    pub action: Action,
    pub ticket: Option<u64>,
}

/// What the detour hands each call to: a [`StepDriver`] over any room.
pub trait StepHandler: Send {
    /// See [`StepDriver::on_step`].
    fn on_step(&mut self, commands: Vec<(u64, Payload)>, run: &mut RunStep<'_>) -> Outcome;
    fn take_log(&mut self) -> Vec<String>;
    /// See [`StepDriver::take_refused`].
    fn take_refused(&mut self) -> Vec<(u64, String)>;
    /// In the room's game: the game's speed is then the room's, and one call
    /// of the game's step must be one update.
    fn in_room(&self) -> bool;
    /// The speed the player picked in the game's speed row (the game's own
    /// speed: 0 paused, 1 for 1x, ...).
    fn chosen_speed(&mut self, speedup: u64);
    /// See [`StepDriver::say`].
    fn say(&mut self, text: ChatText);
}

impl<G: RoomGate + Send> StepHandler for StepDriver<G> {
    fn on_step(&mut self, commands: Vec<(u64, Payload)>, run: &mut RunStep<'_>) -> Outcome {
        StepDriver::on_step(self, commands, run)
    }
    fn take_log(&mut self) -> Vec<String> {
        StepDriver::take_log(self)
    }
    fn take_refused(&mut self) -> Vec<(u64, String)> {
        StepDriver::take_refused(self)
    }
    fn in_room(&self) -> bool {
        matches!(self.phase, Phase::Running | Phase::Holding(_))
    }
    fn chosen_speed(&mut self, speedup: u64) {
        StepDriver::chosen_speed(self, speedup);
    }
    fn say(&mut self, text: ChatText) {
        StepDriver::say(self, text);
    }
}

/// Where the driver is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// The room has not begun a game: the game runs as it would.
    BeforeBegin,
    /// The room's game: steps run as the room releases them.
    Running,
    /// Something the driver cannot follow: the world stands still.
    Holding(String),
    /// The room's game is over: the game runs as it would.
    Ended,
}

/// What one call did: the game's step ran once, with these updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub updates: Updates,
}

/// One call of the game's step, as the driver plans it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Batch<'a> {
    pub updates: Updates,
    /// The room's actions, for the batch's first update.
    pub actions: &'a [Ordered],
    /// The batch ends at a checkpoint step: after its last update the game
    /// reads the world's lanes.
    pub lanes: bool,
}

/// A lane the game read: its number and what the game read for it, which
/// the driver reports as a digest.
pub type LaneText = (u16, String);

/// Runs the game's own step exactly once, as the batch says. Returns the
/// lanes the game read, if it read them, or why it did not follow the
/// batch (its actions were not applied).
pub type RunStep<'a> = dyn FnMut(&Batch<'_>) -> Result<Option<Vec<LaneText>>, String> + 'a;

/// The digests the session reports for the lanes the game read.
pub fn lane_digests(lanes: &[LaneText]) -> Vec<LaneDigest> {
    lanes
        .iter()
        .map(|(lane, text)| {
            let mut bytes = [0u8; 32];
            bytes.copy_from_slice(digest(&SHA256, text.as_bytes()).as_ref());
            LaneDigest {
                lane: *lane,
                digest: FixedBytes(bytes),
            }
        })
        .collect()
}

pub struct StepDriver<G> {
    gate: G,
    game: HookGame,
    control: Box<dyn GameControl>,
    /// Names this game's saves apart from another game's on this PC, which
    /// shares the save folder.
    tag: String,
    saving: Option<Saving>,
    loading: Option<Loading>,
    phase: Phase,
    /// The speed row's last value in the room's game, once seen.
    chosen: Option<u64>,
    /// Steps between checkpoints, from the room's `Begin`.
    checkpoint_interval: u64,
    /// The batch chosen last ends at a checkpoint step.
    lanes_due: bool,
    /// The player's actions handed to the room and not yet ordered back:
    /// the ticket the mod was given for each, by its client sequence number.
    tickets: HashMap<u64, u64>,
    /// The tickets of the player's actions that will never happen, and why.
    refused: Vec<(u64, String)>,
    /// Lines for the hook's log.
    log: Vec<String>,
}

impl<G: RoomGate> StepDriver<G> {
    pub fn new(gate: G, control: Box<dyn GameControl>) -> Self {
        Self {
            gate,
            game: HookGame::default(),
            control,
            tag: std::process::id().to_string(),
            saving: None,
            loading: None,
            phase: Phase::BeforeBegin,
            chosen: None,
            checkpoint_interval: u64::MAX,
            lanes_due: false,
            tickets: HashMap::new(),
            refused: Vec::new(),
            log: Vec::new(),
        }
    }

    /// The tickets of the player's actions that will never happen (the room
    /// refused them, or there was no room's game to hand them to), and why:
    /// the mod tells the window that sent each one.
    pub fn take_refused(&mut self) -> Vec<(u64, String)> {
        for (seq, why) in std::mem::take(&mut self.game.refused) {
            if let Some(ticket) = self.tickets.remove(&seq) {
                self.refused
                    .push((ticket, format!("the room refused it: {why}")));
            }
        }
        std::mem::take(&mut self.refused)
    }

    /// Says `text` to the room for the player, in the room's game only:
    /// what the Multiplayer window's chat sends.
    pub fn say(&mut self, text: ChatText) {
        if self.phase != Phase::Running {
            self.log
                .push("the player said something outside the room's game; nobody heard".into());
            return;
        }
        if let Err(error) = self.gate.chat(text) {
            self.log
                .push(format!("the room did not hear the player: {error}"));
        }
    }

    /// The game's speed row says `speedup` (0 paused, 1 for 1x, ...). In the
    /// room's game, a change the player makes there asks the room for that
    /// speed; the value found on entering the room's game is taken as it is,
    /// so joining never resets a room's speed.
    pub fn chosen_speed(&mut self, speedup: u64) {
        if self.phase != Phase::Running {
            self.chosen = None;
            return;
        }
        match self.chosen {
            None => self.chosen = Some(speedup),
            Some(before) if before == speedup => {}
            Some(_) => {
                self.chosen = Some(speedup);
                let percent = u16::try_from(speedup.saturating_mul(100)).unwrap_or(u16::MAX);
                let speed = Speed(percent.min(Speed::MAX.0));
                match self.gate.request_speed(speed) {
                    Ok(()) => self.log.push(format!(
                        "the speed row asks the room for speed {}%",
                        speed.0
                    )),
                    // A speed the room did not hear is not a reason to stop
                    // following it: the room's speed simply stays.
                    Err(error) => self
                        .log
                        .push(format!("asking the room for a speed failed: {error}")),
                }
            }
        }
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn game(&self) -> &HookGame {
        &self.game
    }

    /// The log lines since the last call, for the hook's log file.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }

    /// In place of one call of the game's step. `commands` are the actions
    /// the player handed over since the last call, for the room; `run` runs
    /// the game's own step, exactly once, with the updates given and the
    /// room's actions for the step the batch starts at.
    pub fn on_step(&mut self, commands: Vec<(u64, Payload)>, run: &mut RunStep<'_>) -> Outcome {
        let mut updates = self.updates();
        if let Some(fault) = self.game.fault.take() {
            self.hold(fault);
            updates = Updates::Exactly(0);
        }
        // After the gate is read: the call that begins the room's game
        // already hands the player's actions over.
        self.hand_over(commands);
        for notice in self.game.notices.drain(..) {
            self.log.push(format!("the room says: {notice}"));
        }
        // The session hears the room only in the gate's calls above.
        for notice in std::mem::take(&mut self.game.window) {
            self.control.room_notice(&notice);
        }
        // The actions wait until a batch runs: a batch that starts at their
        // step, since the session ends the one before there.
        let runs = matches!(updates, Updates::Exactly(steps) if steps > 0);
        let actions: Vec<Ordered> = if runs {
            std::mem::take(&mut self.game.actions)
                .into_iter()
                .map(|(action, own)| Ordered {
                    action,
                    ticket: own.and_then(|seq| self.tickets.remove(&seq)),
                })
                .collect()
        } else {
            Vec::new()
        };
        let batch = Batch {
            updates,
            actions: &actions,
            lanes: runs && self.lanes_due,
        };
        match run(&batch) {
            Ok(lanes) => {
                if batch.lanes {
                    match lanes {
                        Some(lanes) => self.game.lanes = Some(lane_digests(&lanes)),
                        // The steps ran, but the room cannot hear whether the
                        // world is still its own: none is reported.
                        None => {
                            self.hold(format!(
                                "the game did not read the world's lanes at the checkpoint after step {}",
                                self.gate.next_step().saturating_add(u64::from(match updates {
                                    Updates::Exactly(steps) => steps,
                                    Updates::Own => 0,
                                })).saturating_sub(1)
                            ));
                            return Outcome { updates };
                        }
                    }
                }
                if !actions.is_empty() {
                    self.log.push(format!(
                        "the game applied {} action(s) the room ordered",
                        actions.len()
                    ));
                }
                if let Updates::Exactly(steps) = updates {
                    for _ in 0..steps {
                        if let Err(error) = self.gate.after_step(&mut self.game) {
                            self.hold(format!("reporting a step: {error}"));
                            break;
                        }
                    }
                }
            }
            // The steps ran without the room's actions: nothing of them is
            // reported, and the world stands still from here.
            Err(reason) => self.hold(format!("the game did not follow the room's step: {reason}")),
        }
        Outcome { updates }
    }

    /// Hands the player's actions to the room, in the room's game only: an
    /// action handed over nowhere never happens, which is what the mod
    /// expects of one it could not hand over.
    fn hand_over(&mut self, commands: Vec<(u64, Payload)>) {
        if commands.is_empty() {
            return;
        }
        if self.phase != Phase::Running {
            self.log.push(format!(
                "refused {} action(s) of the player: the game is not following a room's game",
                commands.len()
            ));
            for (ticket, _) in commands {
                self.refused
                    .push((ticket, "the game is not following a room's game".into()));
            }
            return;
        }
        let mut commands = commands.into_iter();
        for (ticket, payload) in commands.by_ref() {
            match self.gate.command(payload) {
                Ok(number) => {
                    self.tickets.insert(number, ticket);
                    self.log
                        .push(format!("handed the player's action {number} to the room"));
                }
                Err(error) => {
                    self.refused.push((ticket, format!("{error}")));
                    self.hold(format!("handing an action to the room: {error}"));
                    break;
                }
            }
        }
        for (ticket, _) in commands {
            self.refused
                .push((ticket, "the game stopped following the room".into()));
        }
    }

    /// How many updates this call of the game's step runs.
    fn updates(&mut self) -> Updates {
        if self.phase == Phase::BeforeBegin {
            match self.gate.try_begin() {
                Ok(Some(begin)) => {
                    self.log.push(format!(
                        "the room began a game: rules {}, {} steps a second, checkpoints every {}",
                        begin.rules.as_str(),
                        begin.steps_per_second,
                        begin.checkpoint_interval
                    ));
                    self.checkpoint_interval = u64::from(begin.checkpoint_interval).max(1);
                    self.game.me = Some(begin.player);
                    self.control.set_me(begin.player);
                    self.phase = Phase::Running;
                }
                Ok(None) => {
                    self.tell_world_up();
                    return match self.phase {
                        Phase::Holding(_) => Updates::Exactly(0),
                        _ => Updates::Own,
                    };
                }
                Err(error) => self.hold(format!("before the game began: {error}")),
            }
        }
        match self.phase {
            Phase::BeforeBegin | Phase::Holding(_) => return Updates::Exactly(0),
            Phase::Ended => return Updates::Own,
            Phase::Running => {}
        }
        loop {
            match self.gate.poll_step(&mut self.game) {
                Ok(StepGate::Run) => {
                    let first = self.gate.next_step();
                    return match self.gate.batch(&mut self.game, MAX_STEPS_PER_CALL) {
                        Ok(steps) => {
                            let steps = steps.max(1);
                            let last = first.saturating_add(u64::from(steps) - 1);
                            self.lanes_due = last.is_multiple_of(self.checkpoint_interval);
                            Updates::Exactly(steps)
                        }
                        Err(error) => {
                            self.hold(format!("reading the steps released: {error}"));
                            Updates::Exactly(0)
                        }
                    };
                }
                Ok(StepGate::Wait) => return Updates::Exactly(0),
                Ok(StepGate::Save(order)) => {
                    if !self.save(&order) {
                        return Updates::Exactly(0);
                    }
                }
                Ok(StepGate::Load(load)) => match load.file {
                    // The world every player starts from: the one this
                    // game has loaded, which the launcher started for the
                    // room.
                    None => {
                        if let Err(error) = self.gate.loaded(load.next_step) {
                            self.hold(format!("taking the loaded world: {error}"));
                            return Updates::Exactly(0);
                        }
                        self.log.push(format!(
                            "playing the room's world from step {}",
                            load.next_step
                        ));
                    }
                    Some(file) => {
                        if !self.load(&file, load.next_step) {
                            return Updates::Exactly(0);
                        }
                    }
                },
                Ok(StepGate::Ended) => {
                    self.log
                        .push("the room's game ended; the game runs on its own".into());
                    self.phase = Phase::Ended;
                    return Updates::Own;
                }
                Err(error) => {
                    self.hold(error.to_string());
                    return Updates::Exactly(0);
                }
            }
        }
    }

    /// In the room's lobby: tells the agent of a new world the game has up,
    /// with the mod linked (it said so through `world()`), for the agent to
    /// mark the player ready. The game steps it, so it is up, not one being
    /// replaced; a world that started and was replaced before this call is
    /// never told.
    fn tell_world_up(&mut self) {
        let Some(world) = self.control.world_up() else {
            return;
        };
        match self.gate.world_up(world) {
            Ok(true) => self.log.push(format!(
                "world {world} is up with the mod linked: told the agent, which marks the player ready"
            )),
            Ok(false) => {}
            Err(error) => self.hold(format!("telling the agent the world is up: {error}")),
        }
    }

    /// Moves the room's save on: asks the game for it, waits, and reports
    /// it once the game answers. Returns whether it is reported, so the
    /// room's steps may go on; until then the world stands still.
    fn save(&mut self, order: &SaveOrder) -> bool {
        let now = Instant::now();
        let Some(saving) = &self.saving else {
            let name = format!("tpf3mp_{}_{}", self.tag, order.event);
            self.control.request_save(&name);
            self.log.push(format!(
                "saving the world for the room (event {}) as {name}",
                order.event
            ));
            self.saving = Some(Saving {
                event: order.event,
                since: now,
            });
            return false;
        };
        let outcome = match self.control.save_result() {
            None if now.saturating_duration_since(saving.since) < SAVE_PATIENCE => return false,
            None => Err(format!(
                "the game did not save within {} s",
                SAVE_PATIENCE.as_secs()
            )),
            Some(Err(reason)) => Err(reason),
            Some(Ok(written)) => move_file(&written, &order.file).map_err(|error| {
                format!(
                    "moving the save {} to {}: {error}",
                    written.display(),
                    order.file.display()
                )
            }),
        };
        let event = saving.event;
        self.saving = None;
        match &outcome {
            Ok(()) => self
                .log
                .push(format!("saved the world for the room (event {event})")),
            Err(reason) => self.log.push(format!(
                "the world was not saved for the room (event {event}): {reason}"
            )),
        }
        if let Err(error) = self.gate.saved(&mut self.game, outcome) {
            self.hold(format!("reporting a save: {error}"));
            return false;
        }
        true
    }

    /// Moves a load of the room's save on: asks the game to load it, then
    /// waits for its world. Returns whether the world is loaded; until then
    /// the world stands still.
    fn load(&mut self, file: &Path, next_step: u64) -> bool {
        let now = Instant::now();
        let Some(loading) = &self.loading else {
            match self.control.request_load(file) {
                Ok(()) => {
                    self.log.push(format!(
                        "loading the room's world from {} to run step {next_step} next",
                        file.display()
                    ));
                    self.loading = Some(Loading {
                        next_step,
                        since: now,
                    });
                }
                Err(reason) => self.hold(format!("loading the room's world: {reason}")),
            }
            return false;
        };
        if !self.control.load_done() {
            if now.saturating_duration_since(loading.since) >= LOAD_PATIENCE {
                self.hold(format!(
                    "the room's world did not load within {} s",
                    LOAD_PATIENCE.as_secs()
                ));
            }
            return false;
        }
        let next_step = loading.next_step;
        self.loading = None;
        if let Err(error) = self.gate.loaded(next_step) {
            self.hold(format!("taking the room's world: {error}"));
            return false;
        }
        self.log.push(format!(
            "playing the room's world from its save, from step {next_step}"
        ));
        true
    }

    fn hold(&mut self, reason: String) {
        self.log
            .push(format!("holding the world (fail closed): {reason}"));
        self.phase = Phase::Holding(reason);
    }
}

/// Moves a file, across drives too: a copy then a removal when a rename
/// cannot.
pub fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    std::fs::remove_file(from)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::{collections::VecDeque, path::PathBuf};

    use tpf3mp_bridge::Load;
    use tpf3mp_proto::{FixedBytes, PlayerId, RulesName};

    use super::*;
    use crate::lua::tests::depot_build;

    /// A room that answers from a script.
    #[derive(Default)]
    pub(crate) struct Script {
        pub(crate) begin: VecDeque<Option<Begin>>,
        pub(crate) gates: VecDeque<StepGate>,
        pub(crate) ran: u64,
        pub(crate) loaded: Vec<u64>,
        pub(crate) fail_after: bool,
        pub(crate) speeds: Vec<Speed>,
        /// Events each poll applies before it answers, one list a poll.
        pub(crate) events: VecDeque<Vec<Event>>,
        pub(crate) commands: Vec<Payload>,
        pub(crate) saves: Vec<Result<(), String>>,
        /// Steps between checkpoints, from the begin handed out.
        pub(crate) interval: u64,
        /// The lanes reported, by checkpoint step.
        pub(crate) checkpoints: Vec<(u64, Vec<LaneDigest>)>,
        /// What the player said to the room.
        pub(crate) said: Vec<ChatText>,
        /// What the room says, handed to the game by each poll, one list a
        /// poll.
        pub(crate) notices: VecDeque<Vec<Notice>>,
        /// The worlds told up.
        pub(crate) worlds_up: Vec<u64>,
    }

    impl RoomGate for Script {
        fn try_begin(&mut self) -> Result<Option<Begin>, SessionError> {
            let begin = self.begin.pop_front().flatten();
            if let Some(begin) = &begin {
                self.interval = u64::from(begin.checkpoint_interval).max(1);
            }
            Ok(begin)
        }
        fn next_step(&self) -> u64 {
            self.ran + 1
        }
        fn poll_step(&mut self, game: &mut HookGame) -> Result<StepGate, SessionError> {
            for event in self.events.pop_front().unwrap_or_default() {
                game.apply(&event);
            }
            for notice in self.notices.pop_front().unwrap_or_default() {
                game.notice(notice);
            }
            match self.gates.front() {
                // A Run stays until the step ran, a Save until it is
                // reported and a Load until the world is loaded, as the
                // session's do.
                Some(StepGate::Run) => Ok(StepGate::Run),
                Some(StepGate::Save(order)) => Ok(StepGate::Save(order.clone())),
                Some(StepGate::Load(load)) => Ok(StepGate::Load(load.clone())),
                _ => Ok(self.gates.pop_front().unwrap_or(StepGate::Wait)),
            }
        }
        /// The Runs in a row at the front of the script are one batch, up to
        /// the next checkpoint step, as the gate's are.
        fn batch(&mut self, _game: &mut HookGame, max: u32) -> Result<u32, SessionError> {
            let runs = self
                .gates
                .iter()
                .take_while(|gate| **gate == StepGate::Run)
                .count();
            let next = self.ran + 1;
            let to_checkpoint = match self.interval {
                0 => u64::MAX,
                interval => (interval - next % interval) % interval + 1,
            };
            let steps = u64::try_from(runs).unwrap_or(u64::MAX).min(to_checkpoint);
            Ok(u32::try_from(steps).unwrap_or(u32::MAX).min(max))
        }
        fn after_step(&mut self, game: &mut HookGame) -> Result<u64, SessionError> {
            if self.fail_after {
                return Err(SessionError::AgentGone);
            }
            assert_eq!(
                self.gates.pop_front(),
                Some(StepGate::Run),
                "a step ran unreleased"
            );
            self.ran += 1;
            // As the session does: the lanes at checkpoint steps.
            if self.interval > 0 && self.ran.is_multiple_of(self.interval) {
                self.checkpoints.push((self.ran, game.lanes()));
            }
            Ok(self.ran)
        }
        fn loaded(&mut self, next_step: u64) -> Result<(), SessionError> {
            assert!(
                matches!(self.gates.pop_front(), Some(StepGate::Load(_))),
                "a world nobody ordered was loaded"
            );
            self.loaded.push(next_step);
            Ok(())
        }
        fn request_speed(&mut self, speed: Speed) -> Result<(), SessionError> {
            self.speeds.push(speed);
            Ok(())
        }
        fn command(&mut self, payload: Payload) -> Result<u64, SessionError> {
            self.commands.push(payload);
            Ok(self.commands.len() as u64 - 1)
        }
        fn chat(&mut self, text: ChatText) -> Result<(), SessionError> {
            self.said.push(text);
            Ok(())
        }
        fn world_up(&mut self, world: u64) -> Result<bool, SessionError> {
            self.worlds_up.push(world);
            Ok(true)
        }
        fn saved(
            &mut self,
            _game: &mut HookGame,
            outcome: Result<(), String>,
        ) -> Result<(), SessionError> {
            assert!(
                matches!(self.gates.pop_front(), Some(StepGate::Save(_))),
                "a save nobody ordered was reported"
            );
            self.saves.push(outcome);
            Ok(())
        }
    }

    /// The game's side of saves and loads, from the tests: what it was
    /// asked, and the answers they give it.
    #[derive(Default)]
    pub(crate) struct FakeControl {
        pub(crate) state: std::sync::Arc<std::sync::Mutex<ControlState>>,
    }

    #[derive(Default)]
    pub(crate) struct ControlState {
        pub(crate) save_requests: Vec<String>,
        pub(crate) save_answer: Option<Result<PathBuf, String>>,
        pub(crate) load_requests: Vec<PathBuf>,
        pub(crate) load_done: bool,
        pub(crate) room_notices: Vec<Notice>,
        pub(crate) me: Option<PlayerId>,
        pub(crate) world_up: Option<u64>,
    }

    impl GameControl for FakeControl {
        fn request_save(&mut self, name: &str) {
            self.state
                .lock()
                .unwrap()
                .save_requests
                .push(name.to_owned());
        }
        fn save_result(&mut self) -> Option<Result<PathBuf, String>> {
            self.state.lock().unwrap().save_answer.take()
        }
        fn request_load(&mut self, file: &Path) -> Result<(), String> {
            self.state
                .lock()
                .unwrap()
                .load_requests
                .push(file.to_owned());
            Ok(())
        }
        fn load_done(&mut self) -> bool {
            std::mem::take(&mut self.state.lock().unwrap().load_done)
        }
        fn room_notice(&mut self, notice: &Notice) {
            self.state.lock().unwrap().room_notices.push(notice.clone());
        }
        fn set_me(&mut self, player: PlayerId) {
            self.state.lock().unwrap().me = Some(player);
        }
        fn world_up(&mut self) -> Option<u64> {
            self.state.lock().unwrap().world_up.take()
        }
    }

    pub(crate) fn command_event(seq: u64, step: u64, action: &Action) -> Event {
        Event {
            seq,
            step,
            body: EventBody::Command {
                player: PlayerId(FixedBytes([1; 32])),
                client_seq: seq,
                payload: action.to_payload().unwrap(),
            },
        }
    }

    /// The local player of the tests' games: not the actor of
    /// `command_event`'s events.
    pub(crate) const ME: PlayerId = PlayerId(FixedBytes([9; 32]));

    pub(crate) fn begin() -> Begin {
        Begin {
            rules: RulesName::new("native").unwrap(),
            steps_per_second: 5,
            checkpoint_interval: 50,
            saves: PathBuf::from("saves"),
            player: ME,
        }
    }

    /// The calls of the game's step, with the updates each ran.
    type Calls = Vec<Updates>;

    fn driver(script: Script) -> (StepDriver<Script>, Calls) {
        (
            StepDriver::new(script, Box::new(FakeControl::default())),
            Vec::new(),
        )
    }

    fn driver_with(
        script: Script,
    ) -> (
        StepDriver<Script>,
        std::sync::Arc<std::sync::Mutex<ControlState>>,
    ) {
        let control = FakeControl::default();
        let state = std::sync::Arc::clone(&control.state);
        (StepDriver::new(script, Box::new(control)), state)
    }

    fn call(driver: &mut StepDriver<Script>, calls: &mut Calls) -> Updates {
        let before = calls.len();
        let outcome = driver.on_step(Vec::new(), &mut |batch| {
            calls.push(batch.updates);
            Ok(batch.lanes.then(Vec::new))
        });
        assert_eq!(calls.len(), before + 1, "the game's step runs once a call");
        assert_eq!(calls.last(), Some(&outcome.updates));
        outcome.updates
    }

    /// One call, recording the actions the game was handed; the game
    /// applies them unless `applies` says not.
    fn call_applying(
        driver: &mut StepDriver<Script>,
        commands: Vec<Payload>,
        applied: &mut Vec<(Updates, Vec<Action>)>,
        applies: bool,
    ) -> Updates {
        let commands = commands.into_iter().map(|payload| (0, payload)).collect();
        let outcome = driver.on_step(commands, &mut |batch| {
            applied.push((
                batch.updates,
                batch.actions.iter().map(|o| o.action.clone()).collect(),
            ));
            if applies {
                Ok(batch.lanes.then(Vec::new))
            } else {
                Err("the script took nothing".into())
            }
        });
        outcome.updates
    }

    #[test]
    fn the_rooms_actions_reach_the_game_at_the_first_update_of_their_step() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        script.gates.extend([
            StepGate::Load(Load {
                file: None,
                next_step: 1,
            }),
            StepGate::Run,
            StepGate::Wait,
            StepGate::Run,
            StepGate::Run,
            StepGate::Wait,
        ]);
        // The event for step 2 arrives while the game waits before it.
        script
            .events
            .extend([vec![], vec![], vec![command_event(1, 2, &depot_build())]]);
        let (mut d, _) = driver(script);
        let mut applied = Vec::new();
        assert_eq!(
            call_applying(&mut d, Vec::new(), &mut applied, true),
            Updates::Exactly(1)
        );
        assert_eq!(
            call_applying(&mut d, Vec::new(), &mut applied, true),
            Updates::Exactly(0),
            "the event arrived, step 2 is not released yet"
        );
        assert_eq!(
            call_applying(&mut d, Vec::new(), &mut applied, true),
            Updates::Exactly(2)
        );
        assert_eq!(
            applied,
            vec![
                (Updates::Exactly(1), vec![]),
                (Updates::Exactly(0), vec![]),
                (Updates::Exactly(2), vec![depot_build()]),
            ],
            "handed to the batch that starts at step 2, not to the paused call before"
        );
        assert_eq!(d.phase(), &Phase::Running);
        assert!(d.take_log().iter().any(|l| l.contains("applied 1 action")));
    }

    fn begin_every(interval: u32) -> Begin {
        Begin {
            checkpoint_interval: interval,
            ..begin()
        }
    }

    #[test]
    fn a_batch_ending_at_a_checkpoint_reports_the_lanes_the_game_read() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin_every(3)));
        script.gates.extend([
            StepGate::Load(Load {
                file: None,
                next_step: 1,
            }),
            StepGate::Run,
            StepGate::Run,
            StepGate::Run,
            StepGate::Run,
            StepGate::Wait,
        ]);
        let (mut d, _) = driver(script);
        let mut batches = Vec::new();
        for _ in 0..2 {
            d.on_step(Vec::new(), &mut |batch| {
                batches.push((batch.updates, batch.lanes));
                Ok(batch
                    .lanes
                    .then(|| vec![(3, "vehicles".to_owned()), (0, "net".to_owned())]))
            });
        }
        assert_eq!(
            batches,
            [(Updates::Exactly(3), true), (Updates::Exactly(1), false)],
            "the batch stops at step 3, a checkpoint, and asks for its lanes"
        );
        let checkpoints = &d.gate.checkpoints;
        assert_eq!(checkpoints.len(), 1);
        assert_eq!(checkpoints[0].0, 3);
        assert_eq!(
            checkpoints[0].1,
            lane_digests(&[(3, "vehicles".to_owned()), (0, "net".to_owned())])
        );
        assert_ne!(
            checkpoints[0].1[0].digest, checkpoints[0].1[1].digest,
            "each lane its own digest"
        );
        assert_eq!(d.phase(), &Phase::Running);
    }

    #[test]
    fn a_checkpoint_without_the_worlds_lanes_holds_the_world() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin_every(2)));
        script.gates.extend([
            StepGate::Load(Load {
                file: None,
                next_step: 1,
            }),
            StepGate::Run,
            StepGate::Run,
            StepGate::Run,
        ]);
        let (mut d, _) = driver(script);
        let outcome = d.on_step(Vec::new(), &mut |_batch| Ok(None));
        assert_eq!(outcome.updates, Updates::Exactly(2), "the steps ran");
        assert!(
            matches!(d.phase(), Phase::Holding(why) if why.contains("lanes at the checkpoint after step 2")),
            "{:?}",
            d.phase()
        );
        assert!(d.gate.checkpoints.is_empty(), "nothing reported for them");
        assert_eq!(d.gate.ran, 0);
        let mut calls = Vec::new();
        assert_eq!(call(&mut d, &mut calls), Updates::Exactly(0));
    }

    #[test]
    fn a_game_that_did_not_apply_the_rooms_actions_holds_the_world() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        script.gates.extend([StepGate::Run, StepGate::Run]);
        script
            .events
            .push_back(vec![command_event(1, 1, &depot_build())]);
        let (mut d, _) = driver(script);
        let mut applied = Vec::new();
        call_applying(&mut d, Vec::new(), &mut applied, false);
        assert!(matches!(d.phase(), Phase::Holding(_)));
        assert_eq!(d.gate.ran, 0, "none of those steps is reported");
        assert_eq!(
            call_applying(&mut d, Vec::new(), &mut applied, true),
            PAUSED
        );
    }

    #[test]
    fn an_action_the_game_cannot_read_holds_the_world_before_its_step() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        script.gates.extend([StepGate::Run]);
        let mut bad = command_event(1, 1, &depot_build());
        if let EventBody::Command { payload, .. } = &mut bad.body {
            *payload = Payload::new(vec![0xFF, 0xFF, 0xFF]).unwrap();
        }
        script.events.push_back(vec![bad]);
        let (mut d, _) = driver(script);
        let mut applied = Vec::new();
        assert_eq!(
            call_applying(&mut d, Vec::new(), &mut applied, true),
            PAUSED
        );
        assert!(matches!(d.phase(), Phase::Holding(reason) if reason.contains("cannot read")));
    }

    #[test]
    fn the_players_actions_go_to_the_room_only_in_its_game() {
        let mut script = Script::default();
        script.begin.push_back(None);
        script.begin.push_back(Some(begin()));
        script.gates.extend([StepGate::Run, StepGate::Ended]);
        let (mut d, _) = driver(script);
        let payload = depot_build().to_payload().unwrap();
        let mut applied = Vec::new();
        call_applying(&mut d, vec![payload.clone()], &mut applied, true);
        assert!(d.gate.commands.is_empty(), "no room's game yet");
        assert!(d.take_log().iter().any(|l| l.contains("refused 1 action")));
        call_applying(&mut d, vec![payload.clone()], &mut applied, true);
        assert_eq!(d.gate.commands, vec![payload.clone()]);
        call_applying(&mut d, Vec::new(), &mut applied, true);
        assert_eq!(d.phase(), &Phase::Ended);
        call_applying(&mut d, vec![payload], &mut applied, true);
        assert_eq!(d.gate.commands.len(), 1, "the room's game is over");
    }

    #[test]
    fn the_players_own_actions_come_back_with_their_tickets() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        script.gates.extend([
            StepGate::Load(Load {
                file: None,
                next_step: 1,
            }),
            StepGate::Run,
            StepGate::Run,
            StepGate::Wait,
        ]);
        // The room orders, for step 2, another player's action and then this
        // player's first command (the gate numbers it 0).
        let mut own = command_event(2, 2, &depot_build());
        if let EventBody::Command {
            player, client_seq, ..
        } = &mut own.body
        {
            *player = ME;
            *client_seq = 0;
        }
        script
            .events
            .extend([vec![], vec![command_event(1, 2, &depot_build()), own]]);
        let (mut d, _) = driver(script);
        let payload = depot_build().to_payload().unwrap();
        let mut tickets = Vec::new();
        for commands in [vec![(7, payload)], Vec::new()] {
            d.on_step(commands, &mut |batch| {
                tickets.extend(batch.actions.iter().map(|o| o.ticket));
                Ok(batch.lanes.then(Vec::new))
            });
        }
        assert_eq!(tickets, [None, Some(7)], "the ticket the mod was given");
        assert!(d.take_refused().is_empty());
    }

    #[test]
    fn a_command_the_room_refuses_fails_its_ticket() {
        let mut script = Script::default();
        script.begin.push_back(None);
        script.begin.push_back(Some(begin()));
        script.gates.extend([StepGate::Run, StepGate::Run]);
        let (mut d, _) = driver(script);
        let payload = depot_build().to_payload().unwrap();
        // Before the room's game: refused at once.
        d.on_step(vec![(3, payload.clone())], &mut |_| Ok(None));
        assert_eq!(
            d.take_refused(),
            [(3, "the game is not following a room's game".to_owned())]
        );
        // Handed over as the gate's command 0, then refused by the room.
        d.on_step(vec![(4, payload)], &mut |_| Ok(None));
        d.game.notice(Notice::Refused {
            command: 0,
            reason: tpf3mp_proto::IntentRejection::RateLimited,
        });
        let refused = d.take_refused();
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].0, 4);
        assert!(
            refused[0].1.starts_with("the room refused it"),
            "{refused:?}"
        );
    }

    const PAUSED: Updates = Updates::Exactly(0);

    #[test]
    fn before_the_room_begins_the_game_steps_as_it_would() {
        let (mut d, mut calls) = driver(Script::default());
        assert_eq!(call(&mut d, &mut calls), Updates::Own);
        assert_eq!(d.phase(), &Phase::BeforeBegin);
    }

    #[test]
    fn a_world_up_in_the_lobby_is_told_to_the_agent_once_and_not_in_the_rooms_game() {
        let mut script = Script::default();
        script.begin.extend([None, None, None, Some(begin())]);
        script.gates.push_back(StepGate::Wait);
        let (mut d, state) = driver_with(script);
        let mut calls = Vec::new();
        // No world up yet: nothing told.
        assert_eq!(call(&mut d, &mut calls), Updates::Own);
        assert!(d.gate.worlds_up.is_empty());
        // The mod says a world's GUI started: the next call tells it, once.
        state.lock().unwrap().world_up = Some(1);
        assert_eq!(call(&mut d, &mut calls), Updates::Own);
        assert_eq!(d.gate.worlds_up, vec![1]);
        assert!(d.take_log().iter().any(|l| l.contains("world 1 is up")));
        assert_eq!(call(&mut d, &mut calls), Updates::Own);
        assert_eq!(d.gate.worlds_up, vec![1], "once a world");
        // The room begins; a world that starts in its game is the room's.
        state.lock().unwrap().world_up = Some(2);
        call(&mut d, &mut calls);
        assert_eq!(d.phase(), &Phase::Running);
        call(&mut d, &mut calls);
        assert_eq!(d.gate.worlds_up, vec![1]);
    }

    #[test]
    fn released_steps_run_one_by_one_and_a_withheld_one_holds_the_world() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        script.gates.extend([
            StepGate::Load(Load {
                file: None,
                next_step: 1,
            }),
            StepGate::Run,
            StepGate::Run,
            StepGate::Wait,
        ]);
        let (mut d, mut calls) = driver(script);
        assert_eq!(
            call(&mut d, &mut calls),
            Updates::Exactly(2),
            "two released steps ran"
        );
        assert_eq!(d.phase(), &Phase::Running);
        // The room withholds the next step: the game's paused path.
        assert_eq!(call(&mut d, &mut calls), PAUSED);
        assert_eq!(d.gate.loaded, vec![1]);
        assert_eq!(d.gate.ran, 2);
        d.game.notices.push("Speed(Speed(400))".into());
        call(&mut d, &mut calls);
        assert!(
            d.take_log()
                .iter()
                .any(|line| line == "the room says: Speed(Speed(400))"),
            "what the room says is logged"
        );
    }

    #[test]
    fn a_room_far_ahead_is_caught_up_a_bounded_number_of_steps_a_call() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        script.gates.extend((0..40).map(|_| StepGate::Run));
        let (mut d, mut calls) = driver(script);
        let full = Updates::Exactly(MAX_STEPS_PER_CALL);
        assert_eq!(call(&mut d, &mut calls), full);
        assert_eq!(call(&mut d, &mut calls), full);
        assert_eq!(
            call(&mut d, &mut calls),
            Updates::Exactly(40 - 2 * MAX_STEPS_PER_CALL)
        );
        assert_eq!(call(&mut d, &mut calls), PAUSED);
        assert_eq!(d.gate.ran, 40);
    }

    #[test]
    fn a_lost_agent_holds_the_world() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        script.gates.extend([StepGate::Run, StepGate::Run]);
        script.fail_after = true;
        let (mut d, mut calls) = driver(script);
        assert_eq!(
            call(&mut d, &mut calls),
            Updates::Exactly(2),
            "the steps ran, their report failed"
        );
        assert!(matches!(d.phase(), Phase::Holding(_)));
        assert_eq!(call(&mut d, &mut calls), PAUSED);
    }

    #[test]
    fn the_speed_is_the_rooms_from_the_room_s_game_on_until_it_ends() {
        let mut script = Script::default();
        script.begin.push_back(None);
        script.begin.push_back(Some(begin()));
        script
            .gates
            .extend([StepGate::Run, StepGate::Wait, StepGate::Ended]);
        let (mut d, mut calls) = driver(script);
        call(&mut d, &mut calls);
        assert!(!d.in_room(), "before the room begins, the game's own speed");
        call(&mut d, &mut calls);
        assert!(d.in_room());
        call(&mut d, &mut calls);
        assert!(d.in_room(), "withheld");
        call(&mut d, &mut calls);
        assert!(!d.in_room(), "after it ends, the game's own speed again");
        assert_eq!(
            calls,
            vec![Updates::Own, Updates::Exactly(1), PAUSED, Updates::Own]
        );
    }

    #[test]
    fn a_change_in_the_speed_row_asks_the_room_and_joining_asks_nothing() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        let (mut d, mut calls) = driver(script);
        d.chosen_speed(4);
        assert!(
            d.gate.speeds.is_empty(),
            "before the room's game, nothing is asked"
        );
        call(&mut d, &mut calls);
        // Entering the room's game at 2x: taken as it is.
        d.chosen_speed(2);
        d.chosen_speed(2);
        assert!(d.gate.speeds.is_empty());
        d.chosen_speed(4);
        d.chosen_speed(0);
        d.chosen_speed(0);
        assert_eq!(d.gate.speeds, vec![Speed(400), Speed::PAUSED]);
        d.chosen_speed(1000);
        assert_eq!(
            d.gate.speeds.last(),
            Some(&Speed::MAX),
            "capped at the room's fastest"
        );
    }

    #[test]
    fn the_multiplayer_window_hears_the_room_through_the_game_control() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        let heard = vec![
            Notice::Speed(Speed(200)),
            Notice::Diverged {
                step: 50,
                lanes: vec![3],
            },
        ];
        script.notices.push_back(heard.clone());
        let (mut d, state) = driver_with(script);
        let mut calls = Vec::new();
        call(&mut d, &mut calls);
        let state = state.lock().unwrap();
        assert_eq!(state.me, Some(begin().player), "the game's own player");
        assert_eq!(state.room_notices, heard, "what the room said, in order");
    }

    #[test]
    fn what_the_player_says_reaches_the_room_in_its_game_only() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        let (mut d, mut calls) = driver(script);
        let text = |s: &str| ChatText::new(s).unwrap();
        d.say(text("anyone there?"));
        assert!(
            d.gate.said.is_empty(),
            "before the room's game, nobody hears"
        );
        assert!(
            d.take_log()
                .iter()
                .any(|line| line.contains("outside the room's game")),
            "and the log says so"
        );
        call(&mut d, &mut calls);
        d.say(text("on my way"));
        assert_eq!(d.gate.said, vec![text("on my way")]);
    }

    #[test]
    fn after_the_room_ends_the_game_steps_on_its_own() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        script.gates.push_back(StepGate::Ended);
        let (mut d, mut calls) = driver(script);
        assert_eq!(call(&mut d, &mut calls), Updates::Own);
        assert_eq!(d.phase(), &Phase::Ended);
        assert_eq!(call(&mut d, &mut calls), Updates::Own);
    }

    fn order(event: u64, file: &Path) -> SaveOrder {
        SaveOrder {
            event,
            file: file.to_owned(),
        }
    }

    #[test]
    fn a_save_holds_the_world_until_the_game_saved_and_is_moved_to_the_room() {
        let dir = std::env::temp_dir().join(format!("tpf3mp-step-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let written = dir.join("written.sav");
        let wanted = dir.join("save-7.sav");
        std::fs::write(&written, b"world").unwrap();

        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        script.gates.extend([
            StepGate::Save(order(7, &wanted)),
            StepGate::Run,
            StepGate::Wait,
        ]);
        let (mut d, state) = driver_with(script);
        let mut calls = Vec::new();
        assert_eq!(call(&mut d, &mut calls), PAUSED, "asked the game to save");
        let name = state.lock().unwrap().save_requests.clone();
        assert_eq!(name, vec![format!("tpf3mp_{}_7", std::process::id())]);
        assert_eq!(call(&mut d, &mut calls), PAUSED, "no answer yet: held");
        state.lock().unwrap().save_answer = Some(Ok(written.clone()));
        assert_eq!(
            call(&mut d, &mut calls),
            Updates::Exactly(1),
            "saved and reported: the steps go on"
        );
        assert_eq!(d.gate.saves, vec![Ok(())]);
        assert!(
            !written.exists() && wanted.exists(),
            "moved to the room's file"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_save_the_game_could_not_make_is_reported_failed_and_the_game_goes_on() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        script.gates.extend([
            StepGate::Save(order(8, Path::new("never.sav"))),
            StepGate::Run,
        ]);
        let (mut d, state) = driver_with(script);
        let mut calls = Vec::new();
        call(&mut d, &mut calls);
        state.lock().unwrap().save_answer = Some(Err("disk full".into()));
        assert_eq!(call(&mut d, &mut calls), Updates::Exactly(1));
        assert_eq!(d.gate.saves, vec![Err("disk full".into())]);
        assert_eq!(d.phase(), &Phase::Running);
    }

    #[test]
    fn the_rooms_save_is_loaded_and_its_world_plays_once_it_is_up() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        let file = PathBuf::from("worlds/room.sav");
        script.gates.extend([
            StepGate::Load(Load {
                file: Some(file.clone()),
                next_step: 101,
            }),
            StepGate::Run,
            StepGate::Wait,
        ]);
        let (mut d, state) = driver_with(script);
        let mut calls = Vec::new();
        assert_eq!(call(&mut d, &mut calls), PAUSED, "asked to load");
        assert_eq!(state.lock().unwrap().load_requests, vec![file]);
        assert_eq!(call(&mut d, &mut calls), PAUSED, "still loading");
        assert!(d.gate.loaded.is_empty());
        // The loaded world's GUI started.
        state.lock().unwrap().load_done = true;
        assert_eq!(call(&mut d, &mut calls), Updates::Exactly(1));
        assert_eq!(d.gate.loaded, vec![101]);
        assert!(
            d.take_log()
                .iter()
                .any(|line| line.contains("from its save, from step 101"))
        );
    }
}

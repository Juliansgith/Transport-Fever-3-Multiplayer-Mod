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
//!   game's own speed says;
//! - on anything it cannot follow (the agent gone, a malformed message, a
//!   save it cannot load yet) none, for good: the world stands still rather
//!   than run on apart from the room's (fail closed).
//!
//! Answering the step's own speed call rather than skipping calls is how
//! TPF2MP paced TPF2 (`tpf2-multiplayer/native/src/speedhook.cpp`, which
//! also describes the game's batch pacing).
//!
//! This module knows nothing of the process: the detour hands it the
//! game's step as a closure, and the room's side is a [`RoomGate`], the
//! real [`Session`] in the game and a script in the tests.

use tpf3mp_bridge::{Begin, Game, Notice, Session, SessionError, StepGate};
use tpf3mp_proto::{Event, LaneDigest, Speed};

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
    /// After `poll_step` said Run: the steps that may run as one batch.
    fn batch(&mut self, game: &mut HookGame, max: u32) -> Result<u32, SessionError>;
    fn after_step(&mut self, game: &mut HookGame) -> Result<u64, SessionError>;
    fn loaded(&mut self, next_step: u64) -> Result<(), SessionError>;
    fn request_speed(&mut self, speed: Speed) -> Result<(), SessionError>;
}

impl RoomGate for Session {
    fn try_begin(&mut self) -> Result<Option<Begin>, SessionError> {
        Session::try_begin(self)
    }
    fn poll_step(&mut self, game: &mut HookGame) -> Result<StepGate, SessionError> {
        Session::poll_step(self, game)
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
}

/// The game, as the session sees it, until the hook applies actions: it
/// counts the events the room orders and reports what it is told. Applying
/// an event to the world (PLAN Part 2, Dev B), lanes and saving come next;
/// until then it reports no lanes and refuses to save.
#[derive(Debug, Default)]
pub struct HookGame {
    pub events: u64,
    pub notices: Vec<String>,
}

impl Game for HookGame {
    fn apply(&mut self, _event: &Event) {
        self.events += 1;
    }

    fn lanes(&mut self) -> Vec<LaneDigest> {
        Vec::new()
    }

    fn save(&mut self, _file: &std::path::Path) -> Result<(), String> {
        Err("the hook cannot save the world yet".into())
    }

    fn notice(&mut self, notice: Notice) {
        self.notices.push(format!("{notice:?}"));
    }
}

/// What the detour hands each call to: a [`StepDriver`] over any room.
pub trait StepHandler: Send {
    fn on_step(&mut self, run: &mut dyn FnMut(Updates)) -> Outcome;
    fn take_log(&mut self) -> Vec<String>;
    /// In the room's game: the game's speed is then the room's, and one call
    /// of the game's step must be one update.
    fn in_room(&self) -> bool;
    /// The speed the player picked in the game's speed row (the game's own
    /// speed: 0 paused, 1 for 1x, ...).
    fn chosen_speed(&mut self, speedup: u64);
}

impl<G: RoomGate + Send> StepHandler for StepDriver<G> {
    fn on_step(&mut self, run: &mut dyn FnMut(Updates)) -> Outcome {
        StepDriver::on_step(self, run)
    }
    fn take_log(&mut self) -> Vec<String> {
        StepDriver::take_log(self)
    }
    fn in_room(&self) -> bool {
        matches!(self.phase, Phase::Running | Phase::Holding(_))
    }
    fn chosen_speed(&mut self, speedup: u64) {
        StepDriver::chosen_speed(self, speedup);
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

pub struct StepDriver<G> {
    gate: G,
    game: HookGame,
    phase: Phase,
    /// The speed row's last value in the room's game, once seen.
    chosen: Option<u64>,
    /// Lines for the hook's log.
    log: Vec<String>,
}

impl<G: RoomGate> StepDriver<G> {
    pub fn new(gate: G) -> Self {
        Self {
            gate,
            game: HookGame::default(),
            phase: Phase::BeforeBegin,
            chosen: None,
            log: Vec::new(),
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

    /// In place of one call of the game's step: `run` runs the game's own
    /// step, exactly once, with the updates given.
    pub fn on_step(&mut self, run: &mut dyn FnMut(Updates)) -> Outcome {
        let updates = self.updates();
        for notice in self.game.notices.drain(..) {
            self.log.push(format!("the room says: {notice}"));
        }
        run(updates);
        if let Updates::Exactly(steps) = updates {
            for _ in 0..steps {
                if let Err(error) = self.gate.after_step(&mut self.game) {
                    self.hold(format!("reporting a step: {error}"));
                    break;
                }
            }
        }
        Outcome { updates }
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
                    self.phase = Phase::Running;
                }
                Ok(None) => return Updates::Own,
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
                    return match self.gate.batch(&mut self.game, MAX_STEPS_PER_CALL) {
                        Ok(steps) => Updates::Exactly(steps.max(1)),
                        Err(error) => {
                            self.hold(format!("reading the steps released: {error}"));
                            Updates::Exactly(0)
                        }
                    };
                }
                Ok(StepGate::Wait) => return Updates::Exactly(0),
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
                        self.hold(format!(
                            "the room sent a save ({}), and loading one is not implemented yet",
                            file.display()
                        ));
                        return Updates::Exactly(0);
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

    fn hold(&mut self, reason: String) {
        self.log
            .push(format!("holding the world (fail closed): {reason}"));
        self.phase = Phase::Holding(reason);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::{collections::VecDeque, path::PathBuf};

    use tpf3mp_bridge::Load;
    use tpf3mp_proto::RulesName;

    use super::*;

    /// A room that answers from a script.
    #[derive(Default)]
    pub(crate) struct Script {
        pub(crate) begin: VecDeque<Option<Begin>>,
        pub(crate) gates: VecDeque<StepGate>,
        pub(crate) ran: u64,
        pub(crate) loaded: Vec<u64>,
        pub(crate) fail_after: bool,
        pub(crate) speeds: Vec<Speed>,
    }

    impl RoomGate for Script {
        fn try_begin(&mut self) -> Result<Option<Begin>, SessionError> {
            Ok(self.begin.pop_front().flatten())
        }
        fn poll_step(&mut self, _game: &mut HookGame) -> Result<StepGate, SessionError> {
            match self.gates.front() {
                // A Run stays until the step ran, as the session's does.
                Some(StepGate::Run) => Ok(StepGate::Run),
                _ => Ok(self.gates.pop_front().unwrap_or(StepGate::Wait)),
            }
        }
        /// The Runs in a row at the front of the script are one batch.
        fn batch(&mut self, _game: &mut HookGame, max: u32) -> Result<u32, SessionError> {
            let runs = self
                .gates
                .iter()
                .take_while(|gate| **gate == StepGate::Run)
                .count();
            Ok(u32::try_from(runs).unwrap_or(u32::MAX).min(max))
        }
        fn after_step(&mut self, _game: &mut HookGame) -> Result<u64, SessionError> {
            if self.fail_after {
                return Err(SessionError::AgentGone);
            }
            assert_eq!(
                self.gates.pop_front(),
                Some(StepGate::Run),
                "a step ran unreleased"
            );
            self.ran += 1;
            Ok(self.ran)
        }
        fn loaded(&mut self, next_step: u64) -> Result<(), SessionError> {
            self.loaded.push(next_step);
            Ok(())
        }
        fn request_speed(&mut self, speed: Speed) -> Result<(), SessionError> {
            self.speeds.push(speed);
            Ok(())
        }
    }

    pub(crate) fn begin() -> Begin {
        Begin {
            rules: RulesName::new("native").unwrap(),
            steps_per_second: 5,
            checkpoint_interval: 50,
            saves: PathBuf::from("saves"),
        }
    }

    /// The calls of the game's step, with the updates each ran.
    type Calls = Vec<Updates>;

    fn driver(script: Script) -> (StepDriver<Script>, Calls) {
        (StepDriver::new(script), Vec::new())
    }

    fn call(driver: &mut StepDriver<Script>, calls: &mut Calls) -> Updates {
        let before = calls.len();
        let outcome = driver.on_step(&mut |updates| calls.push(updates));
        assert_eq!(calls.len(), before + 1, "the game's step runs once a call");
        assert_eq!(calls.last(), Some(&outcome.updates));
        outcome.updates
    }

    const PAUSED: Updates = Updates::Exactly(0);

    #[test]
    fn before_the_room_begins_the_game_steps_as_it_would() {
        let (mut d, mut calls) = driver(Script::default());
        assert_eq!(call(&mut d, &mut calls), Updates::Own);
        assert_eq!(d.phase(), &Phase::BeforeBegin);
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
    fn a_save_it_cannot_load_or_a_lost_agent_holds_the_world() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        script.gates.push_back(StepGate::Load(Load {
            file: Some(PathBuf::from("room.sav")),
            next_step: 101,
        }));
        script.gates.extend([StepGate::Run, StepGate::Run]);
        let (mut d, mut calls) = driver(script);
        assert_eq!(call(&mut d, &mut calls), PAUSED);
        assert!(matches!(d.phase(), Phase::Holding(_)));
        // Held for good: later calls run no update, whatever the room says.
        assert_eq!(call(&mut d, &mut calls), PAUSED);
        assert_eq!(d.gate.ran, 0);
        assert!(d.take_log().iter().any(|l| l.contains("fail closed")));

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
    fn after_the_room_ends_the_game_steps_on_its_own() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        script.gates.push_back(StepGate::Ended);
        let (mut d, mut calls) = driver(script);
        assert_eq!(call(&mut d, &mut calls), Updates::Own);
        assert_eq!(d.phase(), &Phase::Ended);
        assert_eq!(call(&mut d, &mut calls), Updates::Own);
    }
}

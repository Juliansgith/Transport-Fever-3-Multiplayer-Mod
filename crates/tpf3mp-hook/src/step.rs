//! The step gate in the game: what the detour on the game's simulation step
//! does each time the game calls it.
//!
//! Transport Fever 3's `GameSim::Step` (as TPF2's) runs as many simulation
//! updates per call as the game's speed says: one at 1x, none while paused.
//! With the game held at 1x, one call is one update, which is the unit the
//! room orders. So the detour replaces each call with [`StepDriver::on_step`]:
//!
//! - it runs the game's own step once for each step the room has released,
//!   up to [`MAX_STEPS_PER_CALL`], so a room faster than the game's own 1x
//!   pace catches up;
//! - it runs nothing while the room withholds the next step (the room is
//!   paused, or a player is behind), and the game's world stands still;
//! - before a room has begun a game, and after it has ended, it runs the
//!   game's step as the game would;
//! - on anything it cannot follow (the agent gone, a malformed message, a
//!   save it cannot load yet) it holds: the world stands still rather than
//!   run on apart from the room's (fail closed).
//!
//! This module knows nothing of the process: the detour hands it the
//! game's step as a closure, and the room's side is a [`RoomGate`], the
//! real [`Session`] in the game and a script in the tests.

use tpf3mp_bridge::{Begin, Game, Notice, Session, SessionError, StepGate};
use tpf3mp_proto::{Event, LaneDigest};

/// Most steps one call of the game's step runs, catching up with the room:
/// at the game's 1x (5 calls a second), rooms up to 16x keep up.
pub const MAX_STEPS_PER_CALL: u32 = 16;

/// The room's side of the gate: what the detour needs of a [`Session`].
pub trait RoomGate {
    fn try_begin(&mut self) -> Result<Option<Begin>, SessionError>;
    fn poll_step(&mut self, game: &mut HookGame) -> Result<StepGate, SessionError>;
    fn after_step(&mut self, game: &mut HookGame) -> Result<u64, SessionError>;
    fn loaded(&mut self, next_step: u64) -> Result<(), SessionError>;
}

impl RoomGate for Session {
    fn try_begin(&mut self) -> Result<Option<Begin>, SessionError> {
        Session::try_begin(self)
    }
    fn poll_step(&mut self, game: &mut HookGame) -> Result<StepGate, SessionError> {
        Session::poll_step(self, game)
    }
    fn after_step(&mut self, game: &mut HookGame) -> Result<u64, SessionError> {
        Session::after_step(self, game)
    }
    fn loaded(&mut self, next_step: u64) -> Result<(), SessionError> {
        Session::loaded(self, next_step)
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
    fn on_step(&mut self, run: &mut dyn FnMut()) -> Outcome;
    fn take_log(&mut self) -> Vec<String>;
    /// In the room's game: the game's speed is then the room's, and one call
    /// of the game's step must be one update.
    fn in_room(&self) -> bool;
}

impl<G: RoomGate + Send> StepHandler for StepDriver<G> {
    fn on_step(&mut self, run: &mut dyn FnMut()) -> Outcome {
        StepDriver::on_step(self, run)
    }
    fn take_log(&mut self) -> Vec<String> {
        StepDriver::take_log(self)
    }
    fn in_room(&self) -> bool {
        matches!(self.phase, Phase::Running | Phase::Holding(_))
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

/// What one call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Outcome {
    /// Times the game's own step ran.
    pub ran: u32,
}

pub struct StepDriver<G> {
    gate: G,
    game: HookGame,
    phase: Phase,
    /// Lines for the hook's log.
    log: Vec<String>,
}

impl<G: RoomGate> StepDriver<G> {
    pub fn new(gate: G) -> Self {
        Self {
            gate,
            game: HookGame::default(),
            phase: Phase::BeforeBegin,
            log: Vec::new(),
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

    /// In place of one call of the game's step; `run` runs the game's own
    /// step once.
    pub fn on_step(&mut self, run: &mut dyn FnMut()) -> Outcome {
        let mut outcome = Outcome::default();
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
                Ok(None) => {
                    run();
                    outcome.ran = 1;
                    return outcome;
                }
                Err(error) => self.hold(format!("before the game began: {error}")),
            }
        }
        match self.phase {
            Phase::BeforeBegin | Phase::Holding(_) => return outcome,
            Phase::Ended => {
                run();
                outcome.ran = 1;
                return outcome;
            }
            Phase::Running => {}
        }
        while outcome.ran < MAX_STEPS_PER_CALL {
            match self.gate.poll_step(&mut self.game) {
                Ok(StepGate::Run) => {
                    run();
                    outcome.ran += 1;
                    if let Err(error) = self.gate.after_step(&mut self.game) {
                        self.hold(format!("reporting a step: {error}"));
                        break;
                    }
                }
                Ok(StepGate::Wait) => break,
                Ok(StepGate::Load(load)) => match load.file {
                    // The world every player starts from: the one this
                    // game has loaded, which the launcher started for the
                    // room.
                    None => {
                        if let Err(error) = self.gate.loaded(load.next_step) {
                            self.hold(format!("taking the loaded world: {error}"));
                            break;
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
                        break;
                    }
                },
                Ok(StepGate::Ended) => {
                    self.log
                        .push("the room's game ended; the game runs on its own".into());
                    self.phase = Phase::Ended;
                    break;
                }
                Err(error) => {
                    self.hold(error.to_string());
                    break;
                }
            }
        }
        outcome
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
    }

    impl RoomGate for Script {
        fn try_begin(&mut self) -> Result<Option<Begin>, SessionError> {
            Ok(self.begin.pop_front().flatten())
        }
        fn poll_step(&mut self, _game: &mut HookGame) -> Result<StepGate, SessionError> {
            Ok(self.gates.pop_front().unwrap_or(StepGate::Wait))
        }
        fn after_step(&mut self, _game: &mut HookGame) -> Result<u64, SessionError> {
            if self.fail_after {
                return Err(SessionError::AgentGone);
            }
            self.ran += 1;
            Ok(self.ran)
        }
        fn loaded(&mut self, next_step: u64) -> Result<(), SessionError> {
            self.loaded.push(next_step);
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

    fn driver(script: Script) -> (StepDriver<Script>, u32) {
        (StepDriver::new(script), 0)
    }

    fn call(driver: &mut StepDriver<Script>, count: &mut u32) -> Outcome {
        driver.on_step(&mut || *count += 1)
    }

    #[test]
    fn before_the_room_begins_the_game_steps_as_it_would() {
        let (mut d, mut ran) = driver(Script::default());
        assert_eq!(call(&mut d, &mut ran).ran, 1);
        assert_eq!(ran, 1);
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
        let (mut d, mut ran) = driver(script);
        assert_eq!(call(&mut d, &mut ran).ran, 2, "two released steps ran");
        assert_eq!(ran, 2);
        assert_eq!(d.phase(), &Phase::Running);
        // The room withholds the next step: the game's step does not run.
        assert_eq!(call(&mut d, &mut ran).ran, 0);
        assert_eq!(ran, 2);
        assert_eq!(d.gate.loaded, vec![1]);
        assert_eq!(d.gate.ran, 2);
    }

    #[test]
    fn a_room_far_ahead_is_caught_up_a_bounded_number_of_steps_a_call() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        script.gates.extend((0..40).map(|_| StepGate::Run));
        let (mut d, mut ran) = driver(script);
        assert_eq!(call(&mut d, &mut ran).ran, MAX_STEPS_PER_CALL);
        assert_eq!(call(&mut d, &mut ran).ran, MAX_STEPS_PER_CALL);
        assert_eq!(call(&mut d, &mut ran).ran, 40 - 2 * MAX_STEPS_PER_CALL);
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
        let (mut d, mut ran) = driver(script);
        assert_eq!(call(&mut d, &mut ran).ran, 0);
        assert!(matches!(d.phase(), Phase::Holding(_)));
        // Held for good: later calls run nothing, whatever the room says.
        assert_eq!(call(&mut d, &mut ran).ran, 0);
        assert_eq!(ran, 0);
        assert!(d.take_log().iter().any(|l| l.contains("fail closed")));

        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        script.gates.extend([StepGate::Run, StepGate::Run]);
        script.fail_after = true;
        let (mut d, mut ran) = driver(script);
        assert_eq!(
            call(&mut d, &mut ran).ran,
            1,
            "the step ran, its report failed"
        );
        assert!(matches!(d.phase(), Phase::Holding(_)));
        assert_eq!(call(&mut d, &mut ran).ran, 0);
    }

    #[test]
    fn the_speed_is_the_rooms_from_the_room_s_game_on_until_it_ends() {
        let mut script = Script::default();
        script.begin.push_back(None);
        script.begin.push_back(Some(begin()));
        script
            .gates
            .extend([StepGate::Run, StepGate::Wait, StepGate::Ended]);
        let (mut d, mut ran) = driver(script);
        call(&mut d, &mut ran);
        assert!(!d.in_room(), "before the room begins, the game's own speed");
        call(&mut d, &mut ran);
        assert!(d.in_room());
        call(&mut d, &mut ran);
        assert!(!d.in_room(), "after it ends, the game's own speed again");
    }

    #[test]
    fn after_the_room_ends_the_game_steps_on_its_own() {
        let mut script = Script::default();
        script.begin.push_back(Some(begin()));
        script.gates.push_back(StepGate::Ended);
        let (mut d, mut ran) = driver(script);
        assert_eq!(call(&mut d, &mut ran).ran, 0);
        assert_eq!(d.phase(), &Phase::Ended);
        assert_eq!(call(&mut d, &mut ran).ran, 1);
    }
}

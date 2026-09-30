//! `--auto-load`: for playtests, the game loads a save by itself from its
//! main menu, once, with no Start Game to press.
//!
//! The launcher puts the save's name in the game's environment
//! ([`tpf3mp_ipc::AUTO_LOAD_ENV`]). On the main menu's frames, where the
//! hook already loads the room's world for a guest (`crate::menu`), it loads
//! this save the same way, before the room's game begins and only while
//! the room has not asked for a world of its own. What happens is logged.
//!
//! [`AutoLoad`] decides when to try, apart from the game, so it can be tested.

use std::time::{Duration, Instant};

use crate::menu::Served;

/// How long to wait before trying again after the menu could not load.
pub const RETRY: Duration = Duration::from_secs(2);
/// How many failed tries end the auto-load.
pub const TRIES: u32 = 5;

/// The auto-load's progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoLoad {
    save: Option<String>,
    failures: u32,
    not_before: Option<Instant>,
}

impl AutoLoad {
    /// From the environment's value: nothing to do without a name.
    pub fn new(value: Option<String>) -> Self {
        let save = value
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        Self {
            save,
            failures: 0,
            not_before: None,
        }
    }

    /// From this process's environment.
    pub fn from_env() -> Self {
        Self::new(std::env::var(tpf3mp_ipc::AUTO_LOAD_ENV).ok())
    }

    /// The save to load now, if one waits and it is time.
    pub fn due(&self, now: Instant) -> Option<&str> {
        if self.not_before.is_some_and(|at| now < at) {
            return None;
        }
        self.save.as_deref()
    }

    /// What the menu answered, or `None` when it had no state to load from.
    /// Returns the line to log, if any.
    pub fn answered(&mut self, served: Option<Served>, now: Instant) -> Option<String> {
        let save = self.save.clone()?;
        match served {
            Some(Served::Started) => {
                self.save = None;
                Some(format!(
                    "auto-load: the main menu is loading the save {save}; the game starts it by itself"
                ))
            }
            // Loading something already: ask again next frame.
            Some(Served::Busy) | None => None,
            Some(Served::Failed(why)) => {
                self.failures += 1;
                if self.failures >= TRIES {
                    self.save = None;
                    Some(format!(
                        "auto-load: gave up on the save {save} after {TRIES} tries: {why}"
                    ))
                } else {
                    self.not_before = Some(now + RETRY);
                    Some(format!(
                        "auto-load: the main menu could not load the save {save} (try {} of {TRIES}): {why}",
                        self.failures
                    ))
                }
            }
        }
    }

    /// The room asked for its own world, or its game began: this save is
    /// not wanted any more.
    pub fn cancel(&mut self) -> Option<String> {
        self.save.take().map(|save| {
            format!("auto-load: the save {save} is not loaded: the room's world comes instead")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_name_nothing_is_loaded() {
        let now = Instant::now();
        assert_eq!(AutoLoad::new(None).due(now), None);
        assert_eq!(AutoLoad::new(Some("  ".into())).due(now), None);
    }

    #[test]
    fn a_started_load_is_the_only_one() {
        let now = Instant::now();
        let mut auto = AutoLoad::new(Some(" mptest ".into()));
        assert_eq!(auto.due(now), Some("mptest"));
        assert!(
            auto.answered(Some(Served::Started), now)
                .unwrap()
                .contains("mptest")
        );
        assert_eq!(auto.due(now), None);
        assert_eq!(auto.answered(Some(Served::Started), now), None);
    }

    #[test]
    fn a_busy_menu_or_none_is_asked_again_next_frame() {
        let now = Instant::now();
        let mut auto = AutoLoad::new(Some("mptest".into()));
        assert_eq!(auto.answered(Some(Served::Busy), now), None);
        assert_eq!(auto.answered(None, now), None);
        assert_eq!(auto.due(now), Some("mptest"));
    }

    #[test]
    fn failures_wait_between_tries_and_end_it() {
        let mut now = Instant::now();
        let mut auto = AutoLoad::new(Some("mptest".into()));
        for try_ in 1..TRIES {
            let line = auto
                .answered(Some(Served::Failed("no".into())), now)
                .unwrap();
            assert!(line.contains(&format!("try {try_} of {TRIES}")), "{line}");
            assert_eq!(auto.due(now), None, "waits");
            now += RETRY;
            assert_eq!(auto.due(now), Some("mptest"));
        }
        let line = auto
            .answered(Some(Served::Failed("no".into())), now)
            .unwrap();
        assert!(line.contains("gave up"), "{line}");
        assert_eq!(auto.due(now + RETRY), None);
    }

    #[test]
    fn the_rooms_world_cancels_it() {
        let mut auto = AutoLoad::new(Some("mptest".into()));
        assert!(auto.cancel().is_some());
        assert_eq!(auto.due(Instant::now()), None);
        assert_eq!(auto.cancel(), None);
    }
}

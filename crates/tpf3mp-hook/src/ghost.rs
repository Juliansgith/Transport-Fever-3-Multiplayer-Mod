//! Pending builds, "build ghosts" (docs/HOOKS.md, "Pending builds"), off
//! unless [`ENV`] is `1`.
//!
//! In a room, a player's road or track appears only when the room's turn
//! applies it. This keeps, in the player's own game, the builds they handed
//! to the room that the room has not answered yet, so the GUI can show them
//! at once and the player can build on from them
//! (`investigation/TF3_BUILD_GHOST_2026-10-02.md`). A ghost is never part of
//! any game's world: nothing here touches the engine, and the room's action
//! is the only thing that builds.
//!
//! - **Added** when `command()` queues a `BuildRoad` or `BuildTrack` for the
//!   room ([`added`]), under its ticket. Other actions have no ghost.
//! - **Gone** when that ticket is answered ([`answered`]): this game applied
//!   the room's action (the real road is there now) or never will (refused,
//!   failed). Also all at once when the room's game stops or another world
//!   loads ([`clear`]): a ghost outlives neither.
//! - **Read** by the GUI with `pending()` ([`to_lua`]): each ghost's ticket
//!   and its action, as `take()` hands actions over, oldest first. The mod's
//!   capture joins a new build's loose ends onto them (`tpf3mp/ghost.lua`).
//!
//! At most [`MAX_PENDING`] are kept; past that a new build has no ghost (it
//! still goes to the room). Fail closed: a ghost the hook cannot keep is not
//! shown, never guessed.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use tpf3mp_proto::action::Action;
use tpf3mp_proto::lua::{LuaValue, action_to_lua};

/// `1` turns pending builds on; anything else, or unset, leaves them off.
pub const ENV: &str = "TPF3MP_HOOK_BUILD_GHOST";
/// Most builds kept pending at once.
pub const MAX_PENDING: usize = 32;

static ENABLED: AtomicBool = AtomicBool::new(false);
static LEDGER: Mutex<Ledger> = Mutex::new(Ledger::new());

/// Whether pending builds are wanted, from [`ENV`]'s value: on only when it
/// says `1`, `on`, `true` or `yes`.
pub fn wanted(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "on" | "true" | "yes")
    )
}

/// Reads [`ENV`] and switches pending builds on or off; `true` when on.
pub fn configure_from_env() -> bool {
    let on = wanted(std::env::var(ENV).ok().as_deref());
    ENABLED.store(on, Ordering::Release);
    on
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}

/// One build handed to the room and not answered yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Ghost {
    pub ticket: u64,
    pub action: Action,
}

/// The pending builds, oldest first.
#[derive(Debug, Default)]
pub struct Ledger {
    ghosts: VecDeque<Ghost>,
}

impl Ledger {
    pub const fn new() -> Self {
        Self {
            ghosts: VecDeque::new(),
        }
    }

    /// Keeps `action`, queued for the room under `ticket`, if it is a road
    /// or track build and there is room. Whether it was kept.
    pub fn add(&mut self, ticket: u64, action: &Action) -> bool {
        if !matches!(action, Action::BuildRoad(_) | Action::BuildTrack(_))
            || self.ghosts.len() >= MAX_PENDING
            || self.ghosts.iter().any(|g| g.ticket == ticket)
        {
            return false;
        }
        self.ghosts.push_back(Ghost {
            ticket,
            action: action.clone(),
        });
        true
    }

    /// `ticket` was answered: its ghost, if any, goes. Whether one went.
    pub fn answered(&mut self, ticket: u64) -> bool {
        let before = self.ghosts.len();
        self.ghosts.retain(|g| g.ticket != ticket);
        self.ghosts.len() != before
    }

    pub fn clear(&mut self) {
        self.ghosts.clear();
    }

    pub fn len(&self) -> usize {
        self.ghosts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ghosts.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Ghost> {
        self.ghosts.iter()
    }

    /// `pending()`'s answer: `{ { ticket =, action = }, ... }`, oldest
    /// first, each action as `take()` hands it over (metres). A ghost whose
    /// action has no table form is left out.
    pub fn to_lua(&self) -> LuaValue {
        let mut list = Vec::with_capacity(self.ghosts.len());
        for ghost in &self.ghosts {
            let Ok(action) = action_to_lua(&ghost.action) else {
                continue;
            };
            #[allow(clippy::cast_precision_loss)]
            let entry = LuaValue::Table(vec![
                (
                    LuaValue::string("ticket"),
                    LuaValue::Number(ghost.ticket as f64),
                ),
                (LuaValue::string("action"), action),
            ]);
            #[allow(clippy::cast_precision_loss)]
            list.push((LuaValue::Number((list.len() + 1) as f64), entry));
        }
        LuaValue::Table(list)
    }
}

fn ledger() -> MutexGuard<'static, Ledger> {
    LEDGER.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `command()` queued `action` under `ticket`: kept as a ghost when pending
/// builds are on and it is a road or track build.
pub fn added(ticket: u64, action: &Action) -> bool {
    enabled() && ledger().add(ticket, action)
}

/// `ticket` was answered.
pub fn answered(ticket: u64) -> bool {
    ledger().answered(ticket)
}

/// Every ghost goes: the room's game stopped, or another world loaded.
pub fn clear() {
    ledger().clear();
}

/// The ghosts for `pending()`; `None` while pending builds are off.
pub fn to_lua() -> Option<LuaValue> {
    enabled().then(|| ledger().to_lua())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use tpf3mp_proto::action::{
        Link, Polyline, Pos, Resolve, RoadBuild, Structure, Tangent, Tram, Vertex,
    };
    use tpf3mp_proto::{BoundedVec, Text};

    fn road(x: i32) -> Action {
        let vertex = |x: i32| Vertex {
            pos: Pos { x, y: 0, z: 0 },
            resolve: Resolve::New,
        };
        let polyline = Polyline::new(
            BoundedVec::new(vec![vertex(x), vertex(x + 50_000)]).unwrap(),
            BoundedVec::new(vec![Link {
                from: 0,
                to: 1,
                tangent0: Tangent {
                    x: 1_000_000,
                    y: 0,
                    z: 0,
                },
                tangent1: Tangent {
                    x: 1_000_000,
                    y: 0,
                    z: 0,
                },
                structure: Structure::Ground,
                kind: None,
                decorations: BoundedVec::empty(),
                locked: false,
                owned: true,
                lanes: BoundedVec::empty(),
                precedence: None,
            }])
            .unwrap(),
            BoundedVec::empty(),
        )
        .unwrap();
        Action::BuildRoad(RoadBuild {
            street: Text::new("streets/standard/town_medium.lua").unwrap(),
            style: None,
            bus_lane: false,
            tram: Tram::None,
            polyline,
        })
    }

    #[test]
    fn off_unless_asked() {
        assert!(!wanted(None));
        assert!(!wanted(Some("")));
        assert!(!wanted(Some("0")));
        assert!(!wanted(Some("off")));
        assert!(wanted(Some("1")));
        assert!(wanted(Some(" Yes ")));
    }

    #[test]
    fn a_build_is_pending_until_its_ticket_is_answered() {
        let mut ledger = Ledger::new();
        assert!(ledger.add(7, &road(0)));
        assert!(ledger.add(8, &road(100_000)));
        assert_eq!(ledger.len(), 2);
        assert!(ledger.answered(7));
        assert!(!ledger.answered(7), "answered once");
        assert_eq!(ledger.iter().map(|g| g.ticket).collect::<Vec<_>>(), [8]);
        ledger.clear();
        assert!(ledger.is_empty());
    }

    #[test]
    fn only_road_and_track_builds_have_ghosts() {
        let mut ledger = Ledger::new();
        let seen = Action::NotificationSeen { notification: 1 };
        assert!(!ledger.add(1, &seen));
        assert!(ledger.is_empty());
    }

    #[test]
    fn a_ticket_is_kept_once_and_the_ledger_is_bounded() {
        let mut ledger = Ledger::new();
        assert!(ledger.add(1, &road(0)));
        assert!(!ledger.add(1, &road(0)), "the same ticket twice");
        for ticket in 2..=MAX_PENDING as u64 {
            assert!(ledger.add(ticket, &road(0)));
        }
        assert!(!ledger.add(1000, &road(0)), "full: no ghost, never a guess");
        assert_eq!(ledger.len(), MAX_PENDING);
    }

    #[test]
    fn pending_lists_tickets_and_actions_in_metres() {
        let mut ledger = Ledger::new();
        ledger.add(3, &road(1_500));
        let LuaValue::Table(list) = ledger.to_lua() else {
            panic!("a table")
        };
        assert_eq!(list.len(), 1);
        let (key, LuaValue::Table(entry)) = &list[0] else {
            panic!("an entry")
        };
        assert_eq!(*key, LuaValue::Number(1.0));
        assert!(entry.contains(&(LuaValue::string("ticket"), LuaValue::Number(3.0))));
        let action = &entry
            .iter()
            .find(|(k, _)| *k == LuaValue::string("action"))
            .unwrap()
            .1;
        assert_eq!(
            tpf3mp_proto::lua::action_from_lua(action).unwrap(),
            road(1_500)
        );
    }
}

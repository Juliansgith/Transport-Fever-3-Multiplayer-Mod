//! The hook through which a room's canonical rules take part in sequencing.

use std::sync::Arc;

use tpf3mp_proto::{Event, Payload, PlayerId, RulesName, RulesOffer, Text};

/// A room's canonical rules.
///
/// The sequencer calls [`validate`](Ruleset::validate) for every intent before
/// ordering it, and [`apply`](Ruleset::apply) for every ordered event, in log
/// order, immediately after ordering it. The ruleset's state therefore always
/// matches the event log, and a validation sees every earlier event.
pub trait Ruleset: Send + 'static {
    /// Accepts an intent, or refuses it with a ruleset-defined code that is
    /// sent back to the player.
    fn validate(&self, player: &PlayerId, payload: &Payload) -> Result<(), u16>;

    fn apply(&mut self, event: &Event);

    /// The ruleset's whole state, for compacting a room's log: a fresh
    /// ruleset that [restores](Ruleset::restore) it and then applies the
    /// events after must end exactly where this one does. `None`, the
    /// default, means the ruleset cannot say, and its rooms keep their whole
    /// log.
    ///
    /// The state outlives the process: a server upgraded meanwhile restores
    /// it with newer rules. So the bytes should name their own format, and
    /// `restore` refuse a format it does not know; the room is then set
    /// aside, not restored wrong.
    fn save(&self) -> Option<Vec<u8>> {
        None
    }

    /// Takes on a state that [`save`](Ruleset::save) produced.
    fn restore(&mut self, state: &[u8]) -> Result<(), String> {
        let _ = state;
        Err("this ruleset cannot restore a saved state".into())
    }
}

/// Creates the ruleset of each new room.
pub type RulesetFactory = Arc<dyn Fn() -> Box<dyn Ruleset> + Send + Sync>;

/// Orders every intent without interpreting it.
#[derive(Debug, Default)]
pub struct AcceptAll;

impl Ruleset for AcceptAll {
    fn validate(&self, _player: &PlayerId, _payload: &Payload) -> Result<(), u16> {
        Ok(())
    }

    fn apply(&mut self, _event: &Event) {}

    fn save(&self) -> Option<Vec<u8>> {
        Some(Vec::new())
    }

    fn restore(&mut self, state: &[u8]) -> Result<(), String> {
        if state.is_empty() {
            Ok(())
        } else {
            Err("accepting every intent keeps no state".into())
        }
    }
}

/// The code a room refuses an intent with whose payload is no action the
/// room's games can read ([`GameActions`]).
pub const MALFORMED_ACTION: u16 = u16::MAX;

/// Orders every intent whose payload is an action the games can read
/// (`tpf3mp_proto::action`: the schema version, every bound, nothing left
/// over), as [`AcceptAll`] orders anything. A game that is handed an action
/// it cannot read holds its world for good (fail closed), so one payload no
/// game reads, from a broken or a hostile client, would stop every game of
/// the room, and every game that joins later and replays it. Refused here,
/// it is never ordered: the sender is told [`MALFORMED_ACTION`].
///
/// The server and the players' games must share the action schema version
/// exactly, as they share the protocol version: a server of another schema
/// refuses every action, so deploy it together with a release whose actions
/// changed.
#[derive(Debug, Default)]
pub struct GameActions;

impl Ruleset for GameActions {
    fn validate(&self, _player: &PlayerId, payload: &Payload) -> Result<(), u16> {
        tpf3mp_proto::action::Action::from_payload(payload)
            .map(|_| ())
            .map_err(|_| MALFORMED_ACTION)
    }

    fn apply(&mut self, _event: &Event) {}

    fn save(&self) -> Option<Vec<u8>> {
        Some(Vec::new())
    }

    fn restore(&mut self, state: &[u8]) -> Result<(), String> {
        if state.is_empty() {
            Ok(())
        } else {
            Err("checking actions keeps no state".into())
        }
    }
}

/// Rules a host can pick for a room.
#[derive(Clone)]
pub struct RulesChoice {
    /// What the room log records, so a recovered room keeps its rules.
    pub name: RulesName,
    pub description: Text<200>,
    pub factory: RulesetFactory,
}

/// The rules a server offers its rooms, the default first.
#[derive(Clone)]
pub struct RulesMenu {
    choices: Vec<RulesChoice>,
}

/// The name of the game's own rules: everything any player does is ordered
/// as it is, and the game's economy runs as it does alone.
pub const NATIVE: &str = "native";

impl RulesMenu {
    /// Only the game's own rules, including its economy.
    pub fn native() -> Self {
        Self::single(RulesChoice {
            name: RulesName::new(NATIVE).expect("short name"),
            description: Text::new("The game's own rules and economy, as in single player.")
                .expect("short description"),
            factory: Arc::new(|| Box::new(AcceptAll)),
        })
    }

    /// The game's own rules, as [`RulesMenu::native`], for rooms of real
    /// games: every intent must be an action the games can read
    /// ([`GameActions`]). The same name and the same (empty) saved state,
    /// so a room recorded under either restores under the other.
    pub fn native_checked() -> Self {
        Self::single(RulesChoice {
            name: RulesName::new(NATIVE).expect("short name"),
            description: Text::new("The game's own rules and economy, as in single player.")
                .expect("short description"),
            factory: Arc::new(|| Box::new(GameActions)),
        })
    }

    /// Just `choice`.
    pub fn single(choice: RulesChoice) -> Self {
        Self {
            choices: vec![choice],
        }
    }

    /// Also offers `choice`, replacing any of the same name.
    #[must_use]
    pub fn with(mut self, choice: RulesChoice) -> Self {
        match self.choices.iter_mut().find(|c| c.name == choice.name) {
            Some(existing) => *existing = choice,
            None => self.choices.push(choice),
        }
        self
    }

    /// The choice named `name`, or the default without a name.
    pub fn find(&self, name: Option<&str>) -> Option<&RulesChoice> {
        match name {
            None => self.choices.first(),
            Some(name) => self.choices.iter().find(|c| c.name.as_str() == name),
        }
    }

    /// What a host picks from.
    pub fn offers(&self) -> Vec<RulesOffer> {
        self.choices
            .iter()
            .map(|c| RulesOffer {
                name: c.name.clone(),
                description: c.description.clone(),
            })
            .collect()
    }
}

impl Default for RulesMenu {
    fn default() -> Self {
        Self::native()
    }
}

impl std::fmt::Debug for RulesMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.choices.iter().map(|c| c.name.as_str()))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_the_first_and_names_pick_the_rest() {
        let menu = RulesMenu::native().with(RulesChoice {
            name: RulesName::new("strict").unwrap(),
            description: Text::new("Validated").unwrap(),
            factory: Arc::new(|| Box::new(AcceptAll)),
        });
        assert_eq!(menu.find(None).unwrap().name.as_str(), NATIVE);
        assert_eq!(menu.find(Some("strict")).unwrap().name.as_str(), "strict");
        assert!(menu.find(Some("other")).is_none());
        let offers = menu.offers();
        assert_eq!(offers.len(), 2);
        assert_eq!(offers[0].name.as_str(), NATIVE);
    }

    #[test]
    fn game_actions_take_a_readable_action_and_refuse_anything_else() {
        let rules = GameActions;
        let player = PlayerId(tpf3mp_proto::FixedBytes([7; 32]));
        let readable = tpf3mp_proto::action::Action::NotificationSeen { notification: 3 }
            .to_payload()
            .unwrap();
        assert_eq!(rules.validate(&player, &readable), Ok(()));
        let mut trailing = readable.as_bytes().to_vec();
        trailing.push(0);
        for bad in [vec![], vec![0xff; 9], trailing] {
            let payload = Payload::new(bad).unwrap();
            assert_eq!(rules.validate(&player, &payload), Err(MALFORMED_ACTION));
        }
        // The same empty state as AcceptAll: rooms restore under either.
        assert_eq!(rules.save(), AcceptAll.save());
        let mut restored = GameActions;
        assert!(restored.restore(&[]).is_ok());
        assert!(restored.restore(&[1]).is_err());
    }
}

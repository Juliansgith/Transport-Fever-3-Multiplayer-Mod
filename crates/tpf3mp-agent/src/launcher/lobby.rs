//! The launcher's lobby in the game (D17): what the main menu's Multiplayer
//! window shows ([`view`]), the launcher actions its buttons stand for
//! ([`action`]), and the game's link while no room session holds it
//! ([`IdleLink`]).
//!
//! The launcher owns one link to its game for as long as it runs. While a
//! room session runs, its [`Bridge`](crate::bridge::Bridge) holds the link
//! and passes the lobby both ways (`BridgeOptions::lobby`); otherwise the
//! launcher serves it here: it answers the hook's hello, sends the lobby
//! whenever it changes and hands the window's actions back. A session takes
//! the link over already greeted, and gives it back when it ends.

use tpf3mp_bridge::{
    BRIDGE_VERSION, LobbyAction, LobbyConnection, LobbyLine, LobbyMember, LobbyRoom, LobbyView,
    MAX_LOBBY_CHAT, ToAgent, ToHook, check_version, decode, encode,
};
use tpf3mp_proto::{BoundedVec, Text};
use tracing::{debug, info, warn};

use super::api::{self, Action, Connection, MemberContent, Phase, State};
use crate::bridge::{BridgeFault, HookLink};

/// The lobby the menu's window shows, from what the launcher shows.
pub(crate) fn view(state: &State) -> LobbyView {
    let chat: Vec<LobbyLine> = state
        .chat
        .iter()
        .rev()
        .take(MAX_LOBBY_CHAT)
        .rev()
        .map(|line| LobbyLine {
            from: Text::lossy(&line.from),
            text: Text::lossy(&line.text),
            you: line.you,
        })
        .collect();
    let room = state.room.as_ref().map(|room| LobbyRoom {
        name: Text::lossy(&room.name),
        rules: Text::lossy(&room.rules),
        invite: room.invite.as_deref().map(Text::lossy),
        running: room.phase == Phase::Running,
        you_own: room.you_own,
        max_players: room.max_players,
        has_password: room.has_password,
        members: BoundedVec::new(
            room.members
                .iter()
                .filter_map(|member| {
                    Some(LobbyMember {
                        player: api::parse_player(&member.id)?,
                        name: Text::lossy(&member.name),
                        ready: member.ready,
                        connected: member.connected,
                        owner: member.owner,
                        you: member.you,
                        same_content: match member.content {
                            MemberContent::Same => Some(true),
                            MemberContent::Differs => Some(false),
                            MemberContent::Unknown => None,
                        },
                    })
                })
                .take(usize::from(tpf3mp_proto::MAX_ROOM_MEMBERS))
                .collect(),
        )
        .unwrap_or_default(),
    });
    LobbyView {
        connection: match state.connection {
            Connection::Disconnected => LobbyConnection::Disconnected,
            Connection::Connecting => LobbyConnection::Connecting,
            Connection::Connected => LobbyConnection::Connected,
        },
        server: Text::lossy(
            state
                .server_name
                .as_deref()
                .or(state.server.as_deref())
                .unwrap_or_default(),
        ),
        name: Text::lossy(&state.name),
        error: state.error.as_deref().map(Text::lossy),
        notice: state.notices.last().map(|notice| Text::lossy(notice)),
        room,
        chat: BoundedVec::new(chat).unwrap_or_default(),
    }
}

/// The launcher action a button of the menu's window stands for. Connect
/// goes to the server the launcher plays on (D12): the window names none.
pub(crate) fn action(action: LobbyAction, state: &State) -> Action {
    match action {
        LobbyAction::Connect { name } => Action::Connect {
            server: state.server.clone().unwrap_or_default(),
            name: name.as_str().to_owned(),
        },
        LobbyAction::Disconnect => Action::Disconnect,
        LobbyAction::Create {
            room,
            max_players,
            password,
        } => Action::Create {
            room: room.as_str().to_owned(),
            max_players,
            password: password.map(|password| password.as_str().to_owned()),
            rules: None,
        },
        LobbyAction::Join { invite, password } => Action::Join {
            invite: invite.as_str().to_owned(),
            password: password.map(|password| password.as_str().to_owned()),
        },
        LobbyAction::Ready { ready } => Action::Ready { ready },
        LobbyAction::Start => Action::Start,
        LobbyAction::Kick { player } => Action::Kick {
            player: api::player_hex(&player),
        },
        LobbyAction::Chat { text } => Action::Chat {
            text: text.as_str().to_owned(),
        },
        LobbyAction::Leave => Action::Leave,
    }
}

/// The game's link while no room session holds it.
pub(crate) struct IdleLink<L> {
    link: L,
    /// The game's build, once its hook said hello and was answered.
    build: Option<String>,
    buf: Vec<u8>,
    /// The lobby the hook was last sent.
    told: Option<LobbyView>,
}

impl<L: HookLink> IdleLink<L> {
    /// A new link, which no hook has greeted yet.
    pub(crate) fn new(link: L) -> Self {
        Self::resumed(link, None)
    }

    /// A link a room session gave back, greeted if `build` says so. The
    /// lobby is sent again at once.
    pub(crate) fn resumed(link: L, build: Option<String>) -> Self {
        Self {
            link,
            build,
            buf: Vec::new(),
            told: None,
        }
    }

    /// The game's build, if its hook said hello.
    pub(crate) fn build(&self) -> Option<&str> {
        self.build.as_deref()
    }

    /// A link a room session gave back when it ended. A game that no
    /// longer runs left no hook on it, so its build does not come back:
    /// otherwise the launcher would go on showing a game attached, and its
    /// window would never offer to start the game again.
    pub(crate) fn given_back(link: L, build: Option<String>, game_runs: bool) -> Self {
        Self::resumed(link, build.filter(|_| game_runs))
    }

    /// The game that said hello on this link closed: the link waits for
    /// the next one.
    pub(crate) fn forget_game(self) -> Self {
        Self::new(self.link)
    }

    /// The link and the game's build, for a room session to take over.
    pub(crate) fn into_parts(self) -> (L, Option<String>) {
        (self.link, self.build)
    }

    /// One round: beats for the hook, reads it, answers its hello and sends
    /// `lobby` if the hook has not seen it yet. Returns the actions the
    /// player took in the menu's window.
    pub(crate) fn pump(&mut self, lobby: &LobbyView) -> Result<Vec<LobbyAction>, BridgeFault> {
        self.link.heartbeat();
        let mut actions = Vec::new();
        while self.link.recv(&mut self.buf)? {
            match decode::<ToAgent>(&self.buf)? {
                ToAgent::Hello { version, build } => {
                    if let Err(error) = check_version(version) {
                        warn!(%error, "the game's hook speaks another bridge version");
                        self.build = None;
                        continue;
                    }
                    info!(%build, "the game's hook attached");
                    // A game started again says hello again: answer it anew.
                    self.link.send(&encode(&ToHook::Hello {
                        version: BRIDGE_VERSION,
                    })?)?;
                    self.build = Some(build.as_str().to_owned());
                    self.told = None;
                }
                ToAgent::Lobby(action) if self.build.is_some() => actions.push(action),
                ToAgent::Log { message } => info!(hook = %message),
                other => debug!(?other, "the game said something outside a room session"),
            }
        }
        if self.build.is_some() && self.told.as_ref() != Some(lobby) {
            let bytes = encode(&ToHook::Lobby(lobby.clone()))?;
            if self.link.send(&bytes)? {
                self.told = Some(lobby.clone());
            }
        }
        Ok(actions)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    use tpf3mp_proto::{FixedBytes, PlayerId};

    use super::*;
    use crate::launcher::{ChatLine, Member, Room};

    /// Both ends of a link in memory: what each side sent the other.
    #[derive(Clone, Default)]
    pub(crate) struct FakeLink {
        pub(crate) to_hook: Arc<Mutex<VecDeque<Vec<u8>>>>,
        pub(crate) to_agent: Arc<Mutex<VecDeque<Vec<u8>>>>,
    }

    impl FakeLink {
        pub(crate) fn hook_says(&self, message: &ToAgent) {
            self.to_agent
                .lock()
                .unwrap()
                .push_back(encode(message).unwrap());
        }

        pub(crate) fn hook_hears(&self) -> Vec<ToHook> {
            self.to_hook
                .lock()
                .unwrap()
                .drain(..)
                .map(|bytes| decode(&bytes).unwrap())
                .collect()
        }
    }

    impl HookLink for FakeLink {
        fn send(&mut self, message: &[u8]) -> Result<bool, BridgeFault> {
            self.to_hook.lock().unwrap().push_back(message.to_vec());
            Ok(true)
        }
        fn recv(&mut self, buf: &mut Vec<u8>) -> Result<bool, BridgeFault> {
            match self.to_agent.lock().unwrap().pop_front() {
                Some(bytes) => {
                    *buf = bytes;
                    Ok(true)
                }
                None => Ok(false),
            }
        }
        fn heartbeat(&mut self) {}
        fn peer_heartbeat(&self) -> u64 {
            0
        }
    }

    fn hello() -> ToAgent {
        ToAgent::Hello {
            version: BRIDGE_VERSION,
            build: Text::lossy("40408"),
        }
    }

    fn lobby(name: &str) -> LobbyView {
        LobbyView {
            name: Text::lossy(name),
            ..LobbyView::default()
        }
    }

    #[test]
    fn a_game_at_its_menu_is_greeted_sent_the_lobby_and_heard() {
        let fake = FakeLink::default();
        let mut idle = IdleLink::new(fake.clone());
        // Before its hello, nothing is sent and nothing taken.
        fake.hook_says(&ToAgent::Lobby(LobbyAction::Start));
        assert!(idle.pump(&lobby("Ann")).unwrap().is_empty());
        assert!(fake.hook_hears().is_empty());

        fake.hook_says(&hello());
        fake.hook_says(&ToAgent::Lobby(LobbyAction::Ready { ready: true }));
        let actions = idle.pump(&lobby("Ann")).unwrap();
        assert_eq!(actions, vec![LobbyAction::Ready { ready: true }]);
        assert_eq!(idle.build(), Some("40408"));
        assert_eq!(
            fake.hook_hears(),
            vec![
                ToHook::Hello {
                    version: BRIDGE_VERSION
                },
                ToHook::Lobby(lobby("Ann"))
            ]
        );
        // Unchanged, it is not sent again; changed, it is.
        idle.pump(&lobby("Ann")).unwrap();
        assert!(fake.hook_hears().is_empty());
        idle.pump(&lobby("Ann B")).unwrap();
        assert_eq!(fake.hook_hears(), vec![ToHook::Lobby(lobby("Ann B"))]);

        // Given back by a session, the lobby goes out again at once.
        let (link, build) = idle.into_parts();
        let mut idle = IdleLink::resumed(link, build);
        idle.pump(&lobby("Ann B")).unwrap();
        assert_eq!(fake.hook_hears(), vec![ToHook::Lobby(lobby("Ann B"))]);
    }

    #[test]
    fn a_hook_of_another_version_is_not_greeted() {
        let fake = FakeLink::default();
        let mut idle = IdleLink::new(fake.clone());
        fake.hook_says(&ToAgent::Hello {
            version: BRIDGE_VERSION + 1,
            build: Text::lossy("40408"),
        });
        fake.hook_says(&ToAgent::Lobby(LobbyAction::Start));
        assert!(idle.pump(&lobby("Ann")).unwrap().is_empty());
        assert!(fake.hook_hears().is_empty());
        assert_eq!(idle.build(), None);
    }

    fn state() -> State {
        let ann = PlayerId(FixedBytes([1; 32]));
        State {
            name: "Ann".into(),
            player: Some(ann.to_string()),
            server: Some("tpf3mp.example.org:29470".into()),
            server_name: Some("EU".into()),
            connection: Connection::Connected,
            error: Some("that room is full".into()),
            notices: vec!["old".into(), "new".into()],
            room: Some(Room {
                name: "Alps".into(),
                rules: "native".into(),
                phase: Phase::Lobby,
                invite: Some("K7QM2X".into()),
                you_own: true,
                max_players: 4,
                has_password: false,
                members: vec![
                    Member {
                        id: api::player_hex(&ann),
                        name: "Ann".into(),
                        platform: "Windows x86-64".into(),
                        ready: true,
                        connected: true,
                        owner: true,
                        you: true,
                        content: MemberContent::Same,
                    },
                    Member {
                        id: "not a player".into(),
                        name: "?".into(),
                        platform: String::new(),
                        ready: false,
                        connected: false,
                        owner: false,
                        you: false,
                        content: MemberContent::Unknown,
                    },
                ],
            }),
            chat: (0..50)
                .map(|n| ChatLine {
                    from: "Bo".into(),
                    text: format!("line {n}"),
                    you: false,
                })
                .collect(),
            ..State::default()
        }
    }

    #[test]
    fn the_menus_window_sees_what_the_launcher_shows() {
        let view = view(&state());
        assert_eq!(view.connection, LobbyConnection::Connected);
        assert_eq!(view.server.as_str(), "EU", "as players see it");
        assert_eq!(view.error.as_ref().unwrap().as_str(), "that room is full");
        assert_eq!(view.notice.as_ref().unwrap().as_str(), "new", "the newest");
        assert!(encode(&ToHook::Lobby(view.clone())).is_ok());
        let room = view.room.unwrap();
        assert!(!room.running && room.you_own);
        assert_eq!(room.invite.unwrap().as_str(), "K7QM2X");
        assert_eq!(room.members.len(), 1, "a member it cannot name is left out");
        assert_eq!(room.members[0].same_content, Some(true));
        assert_eq!(view.chat.len(), MAX_LOBBY_CHAT);
        assert_eq!(
            view.chat.last().unwrap().text.as_str(),
            "line 49",
            "the newest"
        );
    }

    #[test]
    fn the_windows_buttons_are_the_launchers_actions_on_its_own_server() {
        let state = state();
        assert_eq!(
            action(
                LobbyAction::Connect {
                    name: Text::lossy("Ann")
                },
                &state
            ),
            Action::Connect {
                server: "tpf3mp.example.org:29470".into(),
                name: "Ann".into()
            }
        );
        assert_eq!(
            action(
                LobbyAction::Create {
                    room: Text::lossy("Alps"),
                    max_players: 4,
                    password: None,
                },
                &state
            ),
            Action::Create {
                room: "Alps".into(),
                max_players: 4,
                password: None,
                rules: None,
            }
        );
        let bo = PlayerId(FixedBytes([2; 32]));
        assert_eq!(
            action(LobbyAction::Kick { player: bo }, &state),
            Action::Kick {
                player: api::player_hex(&bo)
            }
        );
        assert_eq!(action(LobbyAction::Start, &state), Action::Start);
        assert_eq!(action(LobbyAction::Leave, &state), Action::Leave);
    }

    #[test]
    fn a_closed_game_leaves_no_build_behind_so_it_can_be_started_again() {
        let running = IdleLink::given_back(FakeLink::default(), Some("40408".into()), true);
        assert_eq!(
            running.build(),
            Some("40408"),
            "a running game stays attached"
        );
        let closed = IdleLink::given_back(FakeLink::default(), Some("40408".into()), false);
        assert_eq!(closed.build(), None, "a closed game does not");
        assert_eq!(running.forget_game().build(), None);
    }
}

//! The launcher window, clicked through as a player would, over a stand-in
//! launcher that records what the window asks for.

#![allow(clippy::unwrap_used)]

use std::cell::RefCell;

use eframe::egui::{self, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use tpf3mp_agent::launcher::{
    Action, Connection, Differences, Member, MemberContent, Phase, Room, RulesChoice, State,
};
use tpf3mp_launcher::{
    app::{Extras, LauncherApp},
    backend::Backend,
};

#[derive(Default)]
struct Recorder {
    state: RefCell<State>,
    actions: RefCell<Vec<Action>>,
}

impl Backend for Recorder {
    fn state(&self) -> State {
        self.state.borrow().clone()
    }

    fn act(&self, action: Action) {
        self.actions.borrow_mut().push(action);
    }

    fn busy(&self) -> bool {
        false
    }
}

fn window(state: State) -> Harness<'static, LauncherApp<Recorder>> {
    let recorder = Recorder {
        state: RefCell::new(state),
        ..Recorder::default()
    };
    let app = LauncherApp::new(
        recorder,
        Extras {
            logs: None,
            updater: None,
        },
    );
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 1000.0))
        .build_ui_state(|ui, app: &mut LauncherApp<Recorder>| app.show(ui), app);
    harness.run_steps(4);
    harness
}

fn actions(harness: &Harness<'static, LauncherApp<Recorder>>) -> Vec<Action> {
    harness.state().backend().actions.borrow().clone()
}

fn member(name: &str, owner: bool, you: bool, ready: bool) -> Member {
    Member {
        id: format!("{name}-key"),
        name: name.to_owned(),
        platform: "Windows x86-64".to_owned(),
        ready,
        connected: true,
        owner,
        you,
        content: MemberContent::Same,
    }
}

fn in_room(members: Vec<Member>, you_own: bool) -> State {
    State {
        name: "Ann".into(),
        connection: Connection::Connected,
        room: Some(Room {
            name: "Friday trains".into(),
            rules: "native".into(),
            phase: Phase::Lobby,
            invite: Some("tpf3mp.example.org:29470 TPF3MP1.abc".into()),
            you_own,
            max_players: 4,
            has_password: false,
            members,
        }),
        ..State::default()
    }
}

#[test]
fn connecting_uses_the_server_and_name_offered() {
    let mut window = window(State {
        name: "Ann".into(),
        server: Some("tpf3mp.example.org:29470".into()),
        ..State::default()
    });
    window.get_by_label("Connect").click();
    window.run_steps(4);
    assert_eq!(
        actions(&window),
        [Action::Connect {
            server: "tpf3mp.example.org:29470".into(),
            name: "Ann".into(),
        }]
    );
}

#[test]
fn an_invite_pasted_as_the_server_connects_and_joins() {
    let mut window = window(State::default());
    let server = window.get_by_role_and_label(Role::TextInput, "Server");
    server.focus();
    server.type_text("tpf3mp.example.org:29470 TPF3MP1.abc");
    let name = window.get_by_role_and_label(Role::TextInput, "Your name");
    name.focus();
    name.type_text("Bob");
    window.run_steps(4);
    window.get_by_label("Connect").click();
    window.run_steps(4);
    assert_eq!(
        actions(&window),
        [Action::Connect {
            server: "tpf3mp.example.org:29470 TPF3MP1.abc".into(),
            name: "Bob".into(),
        }]
    );
}

#[test]
fn a_room_is_created_with_the_rules_the_host_picks() {
    let mut window = window(State {
        name: "Ann".into(),
        connection: Connection::Connected,
        rules: vec![
            RulesChoice {
                name: "native".into(),
                description: "The game's own rules and economy".into(),
            },
            RulesChoice {
                name: "strict".into(),
                description: "Checked by the server".into(),
            },
        ],
        ..State::default()
    });
    let room = window.get_by_role_and_label(Role::TextInput, "Room name");
    room.focus();
    room.type_text("Friday trains");
    window.run_steps(4);
    window.get_by_label("Create room").click();
    window.run_steps(4);
    assert_eq!(
        actions(&window),
        [Action::Create {
            room: "Friday trains".into(),
            max_players: 4,
            password: None,
            rules: Some("native".into()),
        }]
    );
}

#[test]
fn the_owner_starts_once_everyone_is_ready() {
    let mut window = window(in_room(
        vec![
            member("Ann", true, true, true),
            member("Bob", false, false, true),
        ],
        true,
    ));
    window.get_by_label("Start game").click();
    window.run_steps(4);
    assert_eq!(actions(&window), [Action::Start]);
}

#[test]
fn removing_a_player_asks_first() {
    let mut window = window(in_room(
        vec![
            member("Ann", true, true, false),
            member("Bob", false, false, false),
        ],
        true,
    ));
    window.get_by_label("Remove").click();
    window.run_steps(4);
    assert!(actions(&window).is_empty(), "nothing before the answer");
    window.get_by_label("Remove them").click();
    window.run_steps(4);
    assert_eq!(
        actions(&window),
        [Action::Kick {
            player: "Bob-key".into()
        }]
    );
}

#[test]
fn a_player_whose_mods_differ_sees_what_to_change() {
    let mut state = in_room(
        vec![
            member("Ann", true, false, false),
            Member {
                content: MemberContent::Differs,
                ..member("Bob", false, true, false)
            },
        ],
        false,
    );
    state.content_diff = Some(Differences {
        summary: "you lack stations 3".into(),
        missing: vec!["stations 3".into()],
        changed: vec![("trains".into(), "1.2".into(), "1.1".into())],
        ..Differences::default()
    });
    let window = window(state);
    window.get_by_label("Your game differs from the room's");
    window.get_by_label("Mods you lack:");
    window.get_by_label_contains("stations 3");
    window.get_by_label_contains("trains: the room has 1.2, you have 1.1");
    window.get_by_label("differ");
}

#[test]
fn chat_is_sent_to_the_room() {
    let mut window = window(in_room(vec![member("Ann", true, true, false)], true));
    let chat = window.get_by_role_and_label(Role::TextInput, "Message");
    chat.focus();
    chat.type_text("good luck");
    window.run_steps(4);
    window.get_by_label("Send").click();
    window.run_steps(4);
    assert_eq!(
        actions(&window),
        [Action::Chat {
            text: "good luck".into()
        }]
    );
}

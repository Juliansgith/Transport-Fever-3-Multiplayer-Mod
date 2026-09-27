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
    window_with(
        state,
        Extras {
            updater: None,
            probe: None,
        },
    )
}

fn window_with(state: State, extras: Extras) -> Harness<'static, LauncherApp<Recorder>> {
    let recorder = Recorder {
        state: RefCell::new(state),
        ..Recorder::default()
    };
    let app = LauncherApp::new(recorder, extras);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 1000.0))
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
            invite: Some("K7QM2X".into()),
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
    server.type_text("tpf3mp.example.org:29470 K7QM2X");
    let name = window.get_by_role_and_label(Role::TextInput, "Your name");
    name.focus();
    name.type_text("Bob");
    window.run_steps(4);
    window.get_by_label("Connect").click();
    window.run_steps(4);
    assert_eq!(
        actions(&window),
        [Action::Connect {
            server: "tpf3mp.example.org:29470 K7QM2X".into(),
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
fn a_launcher_older_than_the_server_says_where_to_get_the_new_one() {
    let window = window(State {
        name: "Ann".into(),
        outdated: true,
        error: Some("this client speaks protocol 3 but the server speaks 4: update TPF3-MP".into()),
        ..State::default()
    });
    window.get_by_label("This TPF3-MP is older than the server's");
    window.get_by_label("the TPF3-MP releases");
}

#[test]
fn the_operators_notice_stands_out() {
    let window = window(State {
        name: "Ann".into(),
        connection: Connection::Connected,
        announcement: Some("Restarting for an update in 5 minutes".into()),
        ..State::default()
    });
    window.get_by_label("From the server:");
    window.get_by_label("Restarting for an update in 5 minutes");
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

#[test]
fn diagnostics_can_be_switched_off() {
    let window = window(State {
        diagnostics: Some(true),
        ..State::default()
    });
    window.get_by_label("Send diagnostics").click();
    let mut window = window;
    window.run_steps(2);
    assert_eq!(actions(&window), [Action::Diagnostics { on: false }]);

    // A launcher that sends none shows no switch.
    let window = self::window(State::default());
    assert!(window.query_by_label("Send diagnostics").is_none());
}

#[test]
fn the_game_is_started_from_a_room() {
    let window = window(in_room(vec![member("Ann", true, true, false)], true));
    window.get_by_label("Start Transport Fever 3").click();
    let mut window = window;
    window.run_steps(2);
    assert_eq!(actions(&window), [Action::LaunchGame]);

    // Outside a room there is nothing for the game to connect to.
    let window = self::window(State {
        connection: Connection::Connected,
        ..State::default()
    });
    assert!(window.query_by_label("Start Transport Fever 3").is_none());
}

/// A narrow window puts everything in one column: the same controls, and
/// they still work.
#[test]
fn a_narrow_window_offers_the_same() {
    let recorder = Recorder {
        state: RefCell::new(in_room(vec![member("Ann", true, true, false)], true)),
        ..Recorder::default()
    };
    let app = LauncherApp::new(
        recorder,
        Extras {
            updater: None,
            probe: None,
        },
    );
    let mut window = Harness::builder()
        .with_size(egui::vec2(640.0, 2000.0))
        .build_ui_state(|ui, app: &mut LauncherApp<Recorder>| app.show(ui), app);
    window.run_steps(4);
    for label in ["Leave room", "Invite code", "K7QM2X", "Copy invite", "Send"] {
        window.get_by_label(label);
    }
    window.get_by_label("Start Transport Fever 3").click();
    window.run_steps(2);
    assert_eq!(actions(&window), [Action::LaunchGame]);
}

/// A package built for its own server offers no other (D12): the server
/// is shown, not asked for, and an invite may go with the name.
#[test]
fn a_package_with_its_own_server_offers_no_other() {
    let own = State {
        name: "Ann".into(),
        server: Some("tpf3mp.example.org:29470".into()),
        server_fixed: true,
        ..State::default()
    };
    let mut window = window(own.clone());
    assert!(
        window
            .query_by_role_and_label(Role::TextInput, "Server")
            .is_none(),
        "no server to type"
    );
    window.get_by_label("tpf3mp.example.org:29470");
    // A package that names its server shows the name, not the address.
    let named = self::window(State {
        server_name: Some("EU".into()),
        ..own.clone()
    });
    named.get_by_label("EU");
    assert!(named.query_by_label("tpf3mp.example.org:29470").is_none());
    window.get_by_label("Connect").click();
    window.run_steps(2);
    assert_eq!(
        actions(&window),
        [Action::Connect {
            server: String::new(),
            name: "Ann".into(),
        }]
    );

    let mut window = self::window(own);
    let invite = window.get_by_role_and_label(Role::TextInput, "Invite");
    invite.focus();
    invite.type_text("K7QM2X");
    window.run_steps(4);
    window.get_by_label("Connect").click();
    window.run_steps(2);
    assert_eq!(
        actions(&window),
        [Action::Connect {
            server: "K7QM2X".into(),
            name: "Ann".into(),
        }]
    );
}

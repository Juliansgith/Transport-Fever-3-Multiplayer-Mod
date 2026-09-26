//! Renders the launcher's screens to images for a person to look at:
//!
//! ```text
//! cargo test -p tpf3mp-launcher --test screenshots -- --ignored
//! ```
//!
//! The images land in `target/launcher-screenshots/`. They need a GPU (or a
//! software renderer), so the test is not part of the normal run.

#![allow(clippy::unwrap_used)]

use std::{cell::RefCell, path::PathBuf};

use eframe::egui;
use egui_kittest::Harness;
use tpf3mp_agent::launcher::{
    Action, ChatLine, Connection, Differences, Game, Member, MemberContent, Phase, Room,
    RulesChoice, State, World,
};
use tpf3mp_launcher::{
    app::{Extras, LauncherApp},
    backend::Backend,
};

struct Still(RefCell<State>);

impl Backend for Still {
    fn state(&self) -> State {
        self.0.borrow().clone()
    }

    fn act(&self, _action: Action) {}

    fn busy(&self) -> bool {
        false
    }
}

fn render(name: &str, state: State) {
    let app = LauncherApp::new(
        Still(RefCell::new(state)),
        Extras {
            logs: Some(PathBuf::from("logs")),
            updater: None,
        },
    );
    let mut harness = Harness::builder()
        .with_size(egui::vec2(780.0, 680.0))
        .wgpu()
        .build_ui_state(|ui, app: &mut LauncherApp<Still>| app.show(ui), app);
    harness.run_steps(4);
    let image = harness.render().unwrap();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/launcher-screenshots");
    std::fs::create_dir_all(&dir).unwrap();
    image.save(dir.join(format!("{name}.png"))).unwrap();
}

fn member(name: &str, owner: bool, you: bool, content: MemberContent) -> Member {
    Member {
        id: format!("{name}-key"),
        name: name.to_owned(),
        platform: if name == "Cat" {
            "macOS arm64".to_owned()
        } else {
            "Windows x86-64".to_owned()
        },
        ready: owner,
        connected: true,
        owner,
        you,
        content,
    }
}

#[test]
#[ignore = "renders images for review; needs a GPU or a software renderer"]
fn screens() {
    render(
        "1-connect",
        State {
            name: "Ann".into(),
            server: Some("tpf3mp.example.org:29470".into()),
            ..State::default()
        },
    );
    render(
        "2-lobby",
        State {
            name: "Ann".into(),
            player: Some("p-3f2a91c0d4e5b6a7".into()),
            connection: Connection::Connected,
            server_version: Some("0.1.0".into()),
            support_id: Some("s-8c21f0a9d3e4b5c6d7e8f90a1b2c3d4e".into()),
            rules: vec![
                RulesChoice {
                    name: "native".into(),
                    description: "The game's own rules and economy, as in single player.".into(),
                },
                RulesChoice {
                    name: "strict".into(),
                    description: "Checked by the server, which runs the economy.".into(),
                },
            ],
            ..State::default()
        },
    );
    let room = Room {
        name: "Friday trains".into(),
        rules: "native".into(),
        phase: Phase::Lobby,
        invite: Some(
            "tpf3mp.example.org:29470 TPF3MP1.ox--2JVdnoyTKISNZaIeyqvx0Plu5-vbOaI0q3h219a2qr94Qc2rUcc2kcOiA_cg"
                .into(),
        ),
        you_own: false,
        max_players: 4,
        has_password: false,
        members: vec![
            member("Ann", true, false, MemberContent::Same),
            member("Bob", false, true, MemberContent::Differs),
            member("Cat", false, false, MemberContent::Same),
        ],
    };
    render(
        "3-room-with-differences",
        State {
            name: "Bob".into(),
            player: Some("p-9b8c7d6e5f4a3b2c".into()),
            connection: Connection::Connected,
            server_version: Some("0.1.0".into()),
            support_id: Some("s-8c21f0a9d3e4b5c6d7e8f90a1b2c3d4e".into()),
            room: Some(room.clone()),
            content_diff: Some(Differences {
                summary: String::new(),
                missing: vec!["more_stations 2".into()],
                extra: vec!["big_map 1".into()],
                changed: vec![("urbangames_vehicles".into(), "1.4".into(), "1.3".into())],
                ..Differences::default()
            }),
            chat: vec![ChatLine {
                from: "Ann".into(),
                text: "Update your vehicle pack and we can start".into(),
                you: false,
            }],
            ..State::default()
        },
    );
    render(
        "4-game-running",
        State {
            name: "Ann".into(),
            player: Some("p-3f2a91c0d4e5b6a7".into()),
            connection: Connection::Connected,
            server_version: Some("0.1.0".into()),
            support_id: Some("s-8c21f0a9d3e4b5c6d7e8f90a1b2c3d4e".into()),
            room: Some(Room {
                phase: Phase::Running,
                you_own: true,
                members: vec![
                    member("Ann", true, true, MemberContent::Same),
                    member("Bob", false, false, MemberContent::Same),
                    member("Cat", false, false, MemberContent::Same),
                ],
                ..room
            }),
            game: Game {
                attached: Some("tpf3".into()),
                world: World::Playing,
                step: Some(18_240),
                speed: 200,
                ..Game::default()
            },
            chat: vec![
                ChatLine {
                    from: "Bob".into(),
                    text: "Station at the harbour is done".into(),
                    you: false,
                },
                ChatLine {
                    from: "Ann".into(),
                    text: "Nice, connecting the coal mine now".into(),
                    you: true,
                },
            ],
            notices: vec!["rejoined the room".into()],
            ..State::default()
        },
    );
}

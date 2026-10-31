//! Two releases' servers at once (DECISIONS.md, D33): an old one drains,
//! keeping its running game for its players and opening none, then hands
//! the game over once nobody is connected; the newer one takes it in, and
//! its players resume it there.

#![allow(clippy::unwrap_used)]

mod common;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use common::{FAST, Player, RunningServer, TestClient, seat};
use tpf3mp_agent::ClientError;
use tpf3mp_net::Identity;
use tpf3mp_proto::{
    CreateRoom, Event, EventBody, Invite, JoinRoom, Payload, RequestError, RoomSettings, TurnStart,
};
use tpf3mp_server::HandoverConfig;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tpf3mp-drain-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn commands(player: &Player) -> usize {
    player
        .applied
        .iter()
        .filter(|event| matches!(event.body, EventBody::Command { .. }))
        .count()
}

async fn play_all(players: Vec<Player>, done: fn(&Player) -> bool) -> Vec<Player> {
    let tasks: Vec<_> = players
        .into_iter()
        .map(|mut player| {
            tokio::spawn(async move {
                player.play_until(done).await;
                player
            })
        })
        .collect();
    let mut finished = Vec::new();
    for task in tasks {
        finished.push(task.await.unwrap());
    }
    finished
}

/// A player's game while it is away: who, and what it applied.
struct Away {
    identity: Arc<Identity>,
    follower: Option<tpf3mp_agent::TurnFollower>,
    start: Option<TurnStart>,
    applied: Vec<Event>,
    executed: u64,
}

/// Closes each player's connection, keeping its game and its seat.
async fn go_away(players: Vec<Player>) -> Vec<Away> {
    let mut away = Vec::new();
    for player in players {
        let Player {
            test,
            follower,
            start,
            applied,
            executed,
            ..
        } = player;
        let identity = Arc::clone(&test.identity);
        test.client.close().await;
        away.push(Away {
            identity,
            follower,
            start,
            applied,
            executed,
        });
    }
    away
}

/// Brings each player back on `server` with its invite, resuming after its
/// last applied turn.
async fn come_back(server: &RunningServer, away: Vec<Away>, invite: &Invite) -> Vec<Player> {
    let mut back = Vec::new();
    for old in away {
        let mut player = Player::new(server.client_as(old.identity, "back").await);
        let resume = old.follower.as_ref().map(|f| f.resume_point());
        player.follower = old.follower;
        player.start = old.start;
        player.applied = old.applied;
        player.executed = old.executed;
        player
            .client()
            .join_room(JoinRoom {
                scan_plan: tpf3mp_proto::ScanPlan::DEFAULT,
                invite: *invite,
                password: None,
                resume,
            })
            .await
            .unwrap();
        back.push(player);
    }
    back
}

fn create() -> CreateRoom {
    CreateRoom {
        name: tpf3mp_proto::Text::new("late").unwrap(),
        max_players: 4,
        password: None,
        settings: RoomSettings::DEFAULT,
        rules: None,
        listing: None,
        competitive: false,
    }
}

#[tokio::test]
async fn a_draining_server_keeps_its_game_opens_none_and_hands_it_over() {
    let base = temp("handover");
    let (old_data, new_data, handover) =
        (base.join("old"), base.join("new"), base.join("handover"));
    let secret = [3; 32];
    let old_dir = old_data.clone();
    let to = handover.clone();
    let old = RunningServer::start(move |config| {
        config.data_dir = Some(old_dir);
        config.secret = secret;
        // A game nobody returns to would be deleted long before it is
        // handed over: draining, it is not.
        config.abandoned_timeout = Duration::from_millis(50);
        let mut handover = HandoverConfig::new(to);
        handover.after = Duration::from_millis(300);
        config.handover = Some(handover);
    })
    .await;
    let new_dir = new_data.clone();
    let from = handover.clone();
    let new = RunningServer::start(move |config| {
        config.data_dir = Some(new_dir);
        config.secret = secret;
        config.import_dir = Some(from);
    })
    .await;

    // A game on the old server.
    let mut clients: Vec<TestClient> = vec![old.client("ann").await, old.client("bob").await];
    let mut seats: Vec<&mut TestClient> = clients.iter_mut().collect();
    let invite = seat(&mut seats, FAST).await;
    clients[0].client.start_game().await.unwrap();
    let players: Vec<Player> = clients.into_iter().map(Player::new).collect();
    players[0]
        .client()
        .send_intent(1, Payload::new(vec![1]).unwrap())
        .await
        .unwrap();
    let players = play_all(players, |p| commands(p) == 1 && p.executed >= 20).await;

    // The newer server is up: the old one drains.
    assert!(old.stats.start_draining());
    assert!(!old.stats.start_draining(), "once");
    let late = old.client("cat").await;
    assert!(late.client.welcome().draining);
    assert!(!new.client("dan").await.client.welcome().draining);
    let refused = late.client.create_room(create()).await;
    assert!(
        matches!(refused, Err(ClientError::Refused(RequestError::Draining))),
        "no new rooms: {refused:?}"
    );
    let refused = late
        .client
        .join_room(JoinRoom {
            scan_plan: tpf3mp_proto::ScanPlan::DEFAULT,
            invite,
            password: None,
            resume: None,
        })
        .await;
    assert!(
        matches!(refused, Err(ClientError::Refused(RequestError::Draining))),
        "no new players: {refused:?}"
    );
    drop(late);

    // Its own players play on while they are there.
    players[1]
        .client()
        .send_intent(2, Payload::new(vec![2]).unwrap())
        .await
        .unwrap();
    let players = play_all(players, |p| commands(p) == 2 && p.executed >= 40).await;
    assert_eq!(old.stats.rooms(), 1);

    // Nobody connected: handed over, and taken in.
    let away = go_away(players).await;
    new.wait_for_rooms(1).await;
    old.wait_for_rooms(0).await;
    let handed = old.stats.render_metrics();
    assert!(
        handed.contains("tpf3mp_games_handed_over_total 1\n"),
        "{handed}"
    );
    assert!(handed.contains("tpf3mp_draining 1\n"), "{handed}");
    let taken = new.stats.render_metrics();
    assert!(taken.contains("tpf3mp_games_taken_in_total 1\n"), "{taken}");
    assert!(
        !old_data
            .join(format!("{}.log", room_of(&new_data)))
            .exists()
    );

    // Its players resume it on the newer server, the log unbroken.
    let players = come_back(&new, away, &invite).await;
    players[0]
        .client()
        .send_intent(3, Payload::new(vec![3]).unwrap())
        .await
        .unwrap();
    let players = play_all(players, |p| commands(p) == 3 && p.executed >= 70).await;
    let shared = players[0].applied.len().min(players[1].applied.len());
    assert_eq!(players[0].applied[..shared], players[1].applied[..shared]);
    let seqs: Vec<u64> = players[0].applied.iter().map(|event| event.seq).collect();
    assert!(
        seqs.windows(2).all(|pair| pair[1] == pair[0] + 1),
        "{seqs:?}"
    );
    old.shut_down().await;
    new.shut_down().await;
    std::fs::remove_dir_all(&base).unwrap();
}

/// The room's name in a data directory with one game logged.
fn room_of(data: &Path) -> String {
    std::fs::read_dir(data)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "log"))
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .unwrap()
}

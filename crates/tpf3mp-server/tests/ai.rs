//! AI players (docs/AI_PLAYERS.md): seats the room's owner adds and
//! removes, the member whose game drives them, the actions it sends for
//! them and the memo the room hands on.

#![allow(clippy::unwrap_used)]

mod common;

use common::{FAST, Player, RunningServer, TestClient, seat};
use tpf3mp_agent::ClientError;
use tpf3mp_proto::{
    AiDifficulty, EventBody, IntentRejection, MAX_AI_SEATS, Payload, PlayerId, RequestError,
    RoomPhase,
};

fn payload(bytes: &[u8]) -> Payload {
    Payload::new(bytes.to_vec()).unwrap()
}

/// The commands a player applied: who, and what.
fn commands(player: &Player) -> Vec<(PlayerId, Vec<u8>)> {
    player
        .applied
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::Command {
                player, payload, ..
            } => Some((*player, payload.as_bytes().to_vec())),
            _ => None,
        })
        .collect()
}

/// The AI players the events seated, in order: who, and how well.
fn seated_ais(player: &Player) -> Vec<(PlayerId, AiDifficulty)> {
    let mut seated = Vec::new();
    for event in &player.applied {
        match &event.body {
            EventBody::AiJoined {
                player, difficulty, ..
            } => seated.push((*player, *difficulty)),
            EventBody::AiLeft { player } => seated.retain(|(p, _)| p != player),
            _ => {}
        }
    }
    seated
}

async fn lobby(clients: &mut [TestClient]) {
    let mut seats: Vec<&mut TestClient> = clients.iter_mut().collect();
    seat(&mut seats, FAST).await;
}

#[tokio::test]
async fn a_server_seats_no_ai_players_unless_its_operator_turns_them_on() {
    // AI players are not yet released (docs/AI_PLAYERS.md): off by
    // default, the server says so and refuses them, even its room's owner.
    let server = RunningServer::start(|_| {}).await;
    let mut clients = vec![server.client("ann").await, server.client("bob").await];
    assert!(!clients[0].client.welcome().ai_players);
    lobby(&mut clients).await;
    let refused = clients[0].client.add_ai(AiDifficulty::Hard).await;
    assert!(
        matches!(
            refused,
            Err(ClientError::Refused(RequestError::AiPlayersOff))
        ),
        "{refused:?}"
    );
    // Turned on, it says so.
    let on = RunningServer::start(|config| config.ai_players = true).await;
    assert!(on.client("cat").await.client.welcome().ai_players);
}

#[tokio::test]
async fn the_owner_seats_ai_players_in_the_lobby_and_only_the_owner() {
    let server = RunningServer::start(|config| config.ai_players = true).await;
    let mut clients = vec![server.client("ann").await, server.client("bob").await];
    lobby(&mut clients).await;

    clients[0].client.add_ai(AiDifficulty::Hard).await.unwrap();
    let refused = clients[1].client.add_ai(AiDifficulty::Easy).await;
    assert!(
        matches!(refused, Err(ClientError::Refused(RequestError::NotOwner))),
        "{refused:?}"
    );
    for _ in 1..MAX_AI_SEATS {
        clients[0].client.add_ai(AiDifficulty::Easy).await.unwrap();
    }
    let full = clients[0].client.add_ai(AiDifficulty::Normal).await;
    assert!(
        matches!(full, Err(ClientError::Refused(RequestError::TooManyAis))),
        "{full:?}"
    );

    let mut ann = Player::new(clients.remove(0));
    ann.play_until(|p| p.room.as_ref().is_some_and(|r| r.ais.len() == MAX_AI_SEATS))
        .await;
    let room = ann.room.as_ref().unwrap();
    assert_eq!(room.phase, RoomPhase::Lobby);
    assert_eq!(room.ais[0].difficulty, AiDifficulty::Hard);
    assert!(room.ais.iter().all(|ai| ai.player.is_ai()));
    assert!(
        room.ais.iter().all(|ai| ai.driver.is_none()),
        "nobody drives an AI before the game"
    );
    let mut ids: Vec<PlayerId> = room.ais.iter().map(|ai| ai.player).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), MAX_AI_SEATS, "every seat its own id");

    // The owner removes one; another takes its seat, and so its id.
    let first = room.ais[0].player;
    ann.client().remove_ai(first).await.unwrap();
    ann.client().add_ai(AiDifficulty::Normal).await.unwrap();
    ann.play_until(|p| {
        p.room
            .as_ref()
            .is_some_and(|r| r.ais.iter().any(|ai| ai.difficulty == AiDifficulty::Normal))
    })
    .await;
    let room = ann.room.as_ref().unwrap();
    let again = room
        .ais
        .iter()
        .find(|ai| ai.difficulty == AiDifficulty::Normal)
        .unwrap();
    assert_eq!(again.player, first, "the lowest free seat, its id again");
}

#[tokio::test]
async fn the_owners_game_drives_the_ais_and_their_actions_are_ordered_as_theirs() {
    let server = RunningServer::start(|config| config.ai_players = true).await;
    let mut clients = vec![server.client("ann").await, server.client("bob").await];
    lobby(&mut clients).await;
    clients[0].client.add_ai(AiDifficulty::Easy).await.unwrap();
    clients[0].client.start_game().await.unwrap();
    let mut players: Vec<Player> = clients.into_iter().map(Player::new).collect();

    // Ann owns the room and keeps up with it: she drives the AI.
    players[0].play_until(|p| !p.ai_seats.is_empty()).await;
    let seat = players[0].ai_seats[0].player;
    assert!(seat.is_ai());
    assert_eq!(players[0].ai_seats[0].difficulty, AiDifficulty::Easy);

    players[0]
        .client()
        .send_ai_intent(seat, 0, payload(&[1, 2, 3]))
        .await
        .unwrap();
    // Bob does not drive it: refused, never ordered.
    players[1]
        .client()
        .send_ai_intent(seat, 0, payload(&[6, 6, 6]))
        .await
        .unwrap();
    players[1].play_until(|p| !p.ai_rejections.is_empty()).await;
    assert_eq!(
        players[1].ai_rejections[0],
        (seat, 0, IntentRejection::NotDriver)
    );
    for player in &mut players {
        player
            .play_until(|p| commands(p).iter().any(|(who, _)| who.is_ai()))
            .await;
        assert_eq!(seated_ais(player), vec![(seat, AiDifficulty::Easy)]);
        assert_eq!(
            commands(player),
            vec![(seat, vec![1, 2, 3])],
            "the AI's action, as the AI's, and nothing of Bob's"
        );
    }
    // The AI joined the game after the players, at its start.
    let first = &players[1].applied;
    let joined = first
        .iter()
        .position(|e| matches!(e.body, EventBody::AiJoined { .. }))
        .unwrap();
    assert!(
        first[..joined]
            .iter()
            .all(|e| matches!(e.body, EventBody::PlayerJoined { .. })),
        "{first:?}"
    );
}

#[tokio::test]
async fn the_ai_and_its_memo_pass_to_another_game_when_its_driver_leaves() {
    let server = RunningServer::start(|config| config.ai_players = true).await;
    let mut clients = vec![server.client("ann").await, server.client("bob").await];
    lobby(&mut clients).await;
    clients[0].client.start_game().await.unwrap();
    let mut players: Vec<Player> = clients.into_iter().map(Player::new).collect();
    let (mut ann, mut bob) = (players.remove(0), players.remove(0));
    tokio::join!(
        ann.play_until(|p| p.executed >= 5),
        bob.play_until(|p| p.executed >= 5)
    );
    let mut players = vec![ann, bob];

    // Seated during the game: it joins with the room's next event.
    players[0]
        .client()
        .add_ai(AiDifficulty::Normal)
        .await
        .unwrap();
    players[0].play_until(|p| !p.ai_seats.is_empty()).await;
    let seat = players[0].ai_seats[0].player;
    assert!(players[0].ai_seats[0].memo.is_none());
    players[0]
        .client()
        .send_ai_memo(seat, payload(b"half a bus line"))
        .await
        .unwrap();
    // The memo is in before Ann leaves: her next action comes after it.
    players[0]
        .client()
        .send_ai_intent(seat, 0, payload(&[4]))
        .await
        .unwrap();
    players[0]
        .play_until(|p| commands(p).iter().any(|(who, _)| *who == seat))
        .await;

    let ann = players.remove(0);
    ann.client().leave_room().await.unwrap();
    let mut bob = players.remove(0);
    bob.play_until(|p| !p.ai_seats.is_empty()).await;
    assert_eq!(bob.ai_seats[0].player, seat);
    assert_eq!(
        bob.ai_seats[0].memo.as_ref().map(Payload::as_bytes),
        Some(&b"half a bus line"[..]),
        "the memo goes with the seat"
    );
    bob.play_until(|p| {
        p.room
            .as_ref()
            .is_some_and(|r| r.ais[0].driver == Some(p.client().player()))
    })
    .await;
    bob.client()
        .send_ai_intent(seat, 1, payload(&[5]))
        .await
        .unwrap();
    bob.play_until(|p| commands(p).iter().any(|(_, what)| what == &[5]))
        .await;

    // Removed: its leaving is ordered, and Bob drives nothing.
    bob.client().remove_ai(seat).await.unwrap();
    bob.play_until(|p| p.ai_seats.is_empty() && seated_ais(p).is_empty())
        .await;
}

#[tokio::test]
async fn nobody_drives_an_ai_in_a_game_nobody_keeps_up_with() {
    let server = RunningServer::start(|config| config.ai_players = true).await;
    let mut clients = vec![server.client("ann").await];
    lobby(&mut clients).await;
    clients[0].client.add_ai(AiDifficulty::Easy).await.unwrap();
    clients[0].client.start_game().await.unwrap();
    let mut ann = Player::new(clients.remove(0));
    // A game that never reports its progress holds the room at its load:
    // its world is not one an AI can look at.
    ann.reports_progress = false;
    ann.play_for(std::time::Duration::from_millis(600)).await;
    assert_eq!(ann.ai_drives, 0, "never handed the seat");
    assert!(ann.room.as_ref().unwrap().ais[0].driver.is_none());
}

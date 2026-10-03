//! What a player's build tool shows reaches the other members of the
//! running game, and nobody else (protocol 17, docs/PROTOCOL.md "Game
//! messages from the client").

#![allow(clippy::unwrap_used)]

mod common;

use std::time::Duration;

use common::{FAST, Player, RunningServer, TestClient, room, seat};
use tpf3mp_proto::{MAX_PREVIEW, Payload};

fn shown(n: u8) -> Payload {
    Payload::new(vec![n; 64]).unwrap()
}

/// Plays every player a little at a time, as their games would side by
/// side, until each has `done`: the room paces to the slowest.
async fn play_all(players: &mut [Player], done: impl Fn(&Player) -> bool) {
    let deadline = std::time::Instant::now() + common::WAIT;
    while !players.iter().all(&done) {
        assert!(std::time::Instant::now() < deadline, "timed out playing");
        for player in players.iter_mut() {
            player.play_for(Duration::from_millis(10)).await;
        }
    }
}

#[tokio::test]
async fn a_preview_reaches_the_other_members_of_the_running_game_only() {
    let server = RunningServer::start(|_| {}).await;
    let mut clients = vec![
        server.client("ann").await,
        server.client("bob").await,
        server.client("cat").await,
    ];
    let mut seats: Vec<&mut TestClient> = clients.iter_mut().collect();
    seat(&mut seats, FAST).await;
    let mut eve = server.client("eve").await;
    eve.client
        .create_room(room("elsewhere", FAST))
        .await
        .unwrap();
    // In the lobby nobody is shown anything.
    clients[0]
        .client
        .send_preview(Some(shown(1)))
        .await
        .unwrap();
    clients[0].client.start_game().await.unwrap();
    let mut players: Vec<Player> = clients.into_iter().map(Player::new).collect();
    play_all(&mut players, |p| p.executed >= 20).await;
    let ann = players[0].client().player();
    players[0]
        .client()
        .send_preview(Some(shown(2)))
        .await
        .unwrap();
    play_all(&mut players[1..], |p| !p.previews.is_empty()).await;
    for player in &players[1..] {
        assert_eq!(
            player.previews,
            vec![(ann, Some(shown(2)))],
            "the game's preview, not the lobby's"
        );
    }
    players[0].client().send_preview(None).await.unwrap();
    play_all(&mut players[1..], |p| p.previews.len() == 2).await;
    for player in &players[1..] {
        assert_eq!(player.previews[1], (ann, None), "and then nothing");
    }
    // One over the size the room shows is not shown.
    let large = Payload::new(vec![7; MAX_PREVIEW + 1]).unwrap();
    players[0].client().send_preview(Some(large)).await.unwrap();
    players[0]
        .client()
        .send_preview(Some(shown(3)))
        .await
        .unwrap();
    play_all(&mut players, |p| {
        p.previews.len() == 3 || p.previews.is_empty()
    })
    .await;
    assert_eq!(players[1].previews[2], (ann, Some(shown(3))));
    // Nobody sees their own, and another room sees nothing.
    players[0].play_for(Duration::from_millis(200)).await;
    assert!(players[0].previews.is_empty());
    let eve_heard = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            if let Some(tpf3mp_agent::ClientEvent::Preview { .. }) = eve.events.recv().await {
                return;
            }
        }
    })
    .await;
    assert!(eve_heard.is_err(), "another room heard a preview");
    server.shut_down().await;
}

#[tokio::test]
async fn a_flood_of_previews_is_cut_and_disconnects_nobody() {
    let server = RunningServer::start(|_| {}).await;
    let mut clients = vec![server.client("ann").await, server.client("bob").await];
    let mut seats: Vec<&mut TestClient> = clients.iter_mut().collect();
    seat(&mut seats, FAST).await;
    clients[0].client.start_game().await.unwrap();
    let mut players: Vec<Player> = clients.into_iter().map(Player::new).collect();
    play_all(&mut players, |p| p.executed >= 20).await;
    for n in 0..60 {
        players[0]
            .client()
            .send_preview(Some(shown(n)))
            .await
            .unwrap();
    }
    for _ in 0..25 {
        for player in &mut players {
            player.play_for(Duration::from_millis(10)).await;
        }
    }
    let heard = players[1].previews.len();
    assert!(
        (1..=15).contains(&heard),
        "{heard} of 60 previews relayed: a burst of ten, then five a second"
    );
    // The game goes on for both.
    let at = players[1].executed;
    play_all(&mut players, |p| p.executed >= at + 20).await;
    assert!(players.iter().all(|p| !p.closed));
    server.shut_down().await;
}

//! How long a room lives once its players are gone (OPERATIONS.md, "Room
//! lifetime"): a lobby closes as soon as its last member leaves; a running
//! game nobody is connected to waits out its grace period for its players,
//! out of the room list, and closes with its log unless one returns; a game
//! restored after a restart gets the same grace.

#![allow(clippy::unwrap_used)]

mod common;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use common::{FAST, Player, RunningServer, TestClient, join, room};
use tpf3mp_proto::{CreateRoom, Invite, RoomListing, Text};
use tpf3mp_server::ServerConfig;

/// The grace period of these tests.
const GRACE: Duration = Duration::from_millis(1500);

fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tpf3mp-lifetime-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn logs(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "log"))
                .count()
        })
        .unwrap_or(0)
}

fn configured(dir: &Path, grace: Duration) -> impl FnOnce(&mut ServerConfig) {
    let dir = dir.to_owned();
    move |config: &mut ServerConfig| {
        config.data_dir = Some(dir);
        config.secret = [9; 32];
        config.abandoned_timeout = grace;
    }
}

fn public(name: &str) -> CreateRoom {
    CreateRoom {
        listing: Some(RoomListing {
            map: Text::new("temperate").unwrap(),
            year: 1850,
            companies: 1,
        }),
        ..room(name, FAST)
    }
}

/// Creates a public room owned by `owner` and starts its game, played until
/// it ran a few steps.
async fn running_game(owner: TestClient) -> (Invite, Player) {
    let (invite, _) = owner.client.create_room(public("lingering")).await.unwrap();
    owner
        .client
        .declare_content(common::content(1))
        .await
        .unwrap();
    owner.client.set_ready(true).await.unwrap();
    owner.client.start_game().await.unwrap();
    let mut player = Player::new(owner);
    player.play_until(|p| p.executed >= 3).await;
    (invite, player)
}

/// How many rooms the list shows, asked by a client of its own.
async fn listed(server: &RunningServer) -> usize {
    let looker = server.client("looker").await;
    let page = looker.client.list_rooms(0).await.unwrap();
    page.rooms.len()
}

#[tokio::test]
async fn a_lobby_closes_as_soon_as_its_last_member_leaves() {
    // A grace far longer than the test: a lobby does not wait it out.
    let server = RunningServer::start(|config| {
        config.abandoned_timeout = Duration::from_secs(600);
    })
    .await;
    let ann = server.client("ann").await;
    ann.client.create_room(public("short")).await.unwrap();
    assert_eq!(listed(&server).await, 1);
    ann.client.leave_room().await.unwrap();
    server.wait_for_rooms(0).await;
    assert_eq!(listed(&server).await, 0, "gone from the list");

    // Disconnecting instead of leaving closes it as well.
    let bob = server.client("bob").await;
    bob.client.create_room(public("shorter")).await.unwrap();
    server.wait_for_rooms(1).await;
    drop(bob);
    server.wait_for_rooms(0).await;
    server.shut_down().await;
}

#[tokio::test]
async fn a_game_everyone_left_closes_after_its_grace_and_leaves_the_list_at_once() {
    let dir = data_dir("grace");
    let server = RunningServer::start(configured(&dir, GRACE)).await;
    let (_invite, player) = running_game(server.client("ann").await).await;
    assert_eq!(listed(&server).await, 1, "listed while played");
    assert_eq!(logs(&dir), 1);

    // Every game closed: nobody is connected.
    drop(player);
    tokio::time::sleep(GRACE / 3).await;
    assert_eq!(server.stats.rooms(), 1, "held for its players a while");
    assert_eq!(listed(&server).await, 0, "but out of the list meanwhile");

    server.wait_for_rooms(0).await;
    assert_eq!(logs(&dir), 0, "its log deleted with it");
    server.shut_down().await;
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_player_who_returns_within_the_grace_keeps_the_game() {
    let dir = data_dir("returns");
    let server = RunningServer::start(configured(&dir, GRACE)).await;
    let ann = server.client("ann").await;
    let identity = Arc::clone(&ann.identity);
    let (invite, player) = running_game(ann).await;
    drop(player);
    tokio::time::sleep(GRACE / 3).await;

    // Back before the grace ran out, as after a crash or a network drop.
    let ann = server.client_as(identity, "ann").await;
    ann.client.join_room(join(&invite)).await.unwrap();
    tokio::time::sleep(GRACE * 2).await;
    assert_eq!(server.stats.rooms(), 1, "the game stays while she plays");
    assert_eq!(listed(&server).await, 1, "and is listed again");
    assert_eq!(logs(&dir), 1);

    // Leaving for good, the last member, closes it at once.
    ann.client.leave_room().await.unwrap();
    server.wait_for_rooms(0).await;
    assert_eq!(logs(&dir), 0);
    server.shut_down().await;
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_restored_game_closes_unless_a_player_returns_within_the_grace() {
    let dir = data_dir("restored");
    // A grace long enough to see the restart through.
    let server = RunningServer::start(configured(&dir, Duration::from_secs(600))).await;
    let ann = server.client("ann").await;
    let identity = Arc::clone(&ann.identity);
    let (invite, player) = running_game(ann).await;
    drop(player);
    server.shut_down().await;

    // Restored, and rejoined in time: it stays past the grace.
    let server = RunningServer::start(configured(&dir, GRACE)).await;
    assert_eq!(server.stats.rooms(), 1, "restored");
    assert_eq!(listed(&server).await, 0, "a restored room is never listed");
    let ann = server.client_as(Arc::clone(&identity), "ann").await;
    ann.client.join_room(join(&invite)).await.unwrap();
    tokio::time::sleep(GRACE * 2).await;
    assert_eq!(server.stats.rooms(), 1, "kept for the player who returned");
    drop(ann);
    server.shut_down().await;

    // Restored again, and nobody returns: it closes with its log.
    let server = RunningServer::start(configured(&dir, GRACE)).await;
    assert_eq!(server.stats.rooms(), 1, "restored");
    server.wait_for_rooms(0).await;
    assert_eq!(logs(&dir), 0, "its log deleted");
    // Its invite finds nothing now, and the next start restores nothing.
    let ann = server.client_as(Arc::clone(&identity), "ann").await;
    assert!(ann.client.join_room(join(&invite)).await.is_err());
    drop(ann);
    server.shut_down().await;
    let server = RunningServer::start(configured(&dir, GRACE)).await;
    assert_eq!(server.stats.rooms(), 0, "nothing left to restore");
    server.shut_down().await;
    std::fs::remove_dir_all(&dir).unwrap();
}

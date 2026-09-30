//! Snapshots at the edges: who may join a running game, who may fetch or
//! upload a world, and the world an owner hands a room to start from. The whole flow, with games that really save and load, is
//! in `tpf3mp-testkit`'s scenarios.

#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;

use common::{FAST, Player, RunningServer, TestClient, content, join, seat};
use tpf3mp_agent::{ClientError, ClientEvent, Worlds, transfer};
use tpf3mp_net::read_message;
use tpf3mp_proto::{
    BULK_REQUEST_MAX_FRAME, BULK_RESPONSE_MAX_FRAME, BulkOpen, BulkRequest, BulkResponse,
    FixedBytes, Invite, Request, RequestError, SavedWorld, SnapshotId, WorldOffer,
};
use tpf3mp_server::{ServerConfig, SnapshotConfig};

fn saving(dir: &Path) -> impl FnOnce(&mut ServerConfig) + use<> {
    let dir = dir.to_owned();
    move |config: &mut ServerConfig| {
        config.snapshots = Some(SnapshotConfig::new(dir.join("snapshots")));
    }
}

fn saving_and_logging(dir: &Path, secret: [u8; 32]) -> impl FnOnce(&mut ServerConfig) + use<> {
    let dir = dir.to_owned();
    move |config: &mut ServerConfig| {
        config.snapshots = Some(SnapshotConfig::new(dir.join("snapshots")));
        config.data_dir = Some(dir.join("rooms"));
        config.secret = secret;
    }
}

/// Seats the clients, starts the game and plays it a little, every player
/// at once: the room holds its clock until all have loaded.
async fn running(mut clients: Vec<TestClient>) -> (Vec<Player>, Invite) {
    let mut seats: Vec<&mut TestClient> = clients.iter_mut().collect();
    let invite = seat(&mut seats, FAST).await;
    clients[0].client.start_game().await.unwrap();
    let tasks: Vec<_> = clients
        .into_iter()
        .map(|client| {
            tokio::spawn(async move {
                let mut player = Player::new(client);
                player.play_until(|p| p.executed >= 1).await;
                player
            })
        })
        .collect();
    let mut players = Vec::new();
    for task in tasks {
        players.push(task.await.unwrap());
    }
    (players, invite)
}

/// Joins the running game of `invite` as a newcomer whose game runs the
/// content `content_of`, or who declared none.
async fn newcomer(
    player: &TestClient,
    invite: &Invite,
    content_of: Option<u8>,
) -> Result<tpf3mp_proto::RoomView, ClientError> {
    if let Some(value) = content_of {
        player.client.declare_content(content(value)).await?;
    }
    player.client.join_room(join(invite)).await
}

#[tokio::test]
async fn a_newcomer_must_run_the_games_content() {
    let dir = tempfile::tempdir().unwrap();
    let server = RunningServer::start(saving(dir.path())).await;
    let (_players, invite) = running(vec![server.client("ann").await]).await;
    let mut cat = server.client("cat").await;
    for wrong in [None, Some(2)] {
        assert_eq!(
            newcomer(&cat, &invite, wrong).await.unwrap_err(),
            ClientError::Refused(RequestError::ContentMismatch),
            "content {wrong:?}"
        );
    }
    // The refusal says how the newcomer's game differs from the game's.
    let diff = cat.content_diff().await.unwrap();
    let builds = diff.game.unwrap();
    assert_eq!(
        (builds.room.as_str(), builds.yours.as_str()),
        ("build-1", "build-2")
    );
    let room = newcomer(&cat, &invite, Some(1)).await.unwrap();
    assert_eq!(room.members.len(), 2, "a seat at the running game");
    server.shut_down().await;
}

#[tokio::test]
async fn a_restored_game_still_says_how_a_newcomer_differs() {
    let dir = tempfile::tempdir().unwrap();
    let secret = [4; 32];
    let server = RunningServer::start(saving_and_logging(dir.path(), secret)).await;
    let (mut players, invite) = running(vec![server.client("ann").await]).await;
    players[0].play_until(|p| p.executed >= 20).await;
    drop(players);
    server.shut_down().await;

    // The game's content survives in its log.
    let server = RunningServer::start(saving_and_logging(dir.path(), secret)).await;
    let mut cat = server.client("cat").await;
    assert_eq!(
        newcomer(&cat, &invite, Some(3)).await.unwrap_err(),
        ClientError::Refused(RequestError::ContentMismatch)
    );
    let builds = cat.content_diff().await.unwrap().game.unwrap();
    assert_eq!(
        (builds.room.as_str(), builds.yours.as_str()),
        ("build-1", "build-3")
    );
    server.shut_down().await;
}

#[tokio::test]
async fn a_kicked_player_stays_out_even_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let secret = [3; 32];
    let server = RunningServer::start(saving_and_logging(dir.path(), secret)).await;
    let (mut players, invite) = running(vec![server.client("ann").await]).await;
    // Bob joins the running game, then Ann removes him.
    let bob = server.client("bob").await;
    let bob_identity = std::sync::Arc::clone(&bob.identity);
    newcomer(&bob, &invite, Some(1)).await.unwrap();
    players[0].client().kick(bob.client.player()).await.unwrap();
    drop(bob);
    let refused = ClientError::Refused(RequestError::BadInvite);
    let bob = server
        .client_as(std::sync::Arc::clone(&bob_identity), "bob")
        .await;
    assert_eq!(newcomer(&bob, &invite, Some(1)).await.unwrap_err(), refused);
    drop(bob);
    // Ann plays on, so the kick is logged.
    players[0].play_until(|p| p.executed >= 20).await;
    drop(players);
    server.shut_down().await;

    let server = RunningServer::start(saving_and_logging(dir.path(), secret)).await;
    let bob = server.client_as(bob_identity, "bob").await;
    assert_eq!(
        newcomer(&bob, &invite, Some(1)).await.unwrap_err(),
        refused,
        "the restored room remembers the kick"
    );
    server.shut_down().await;
}

#[tokio::test]
async fn nobody_fetches_a_world_they_were_not_offered() {
    let dir = tempfile::tempdir().unwrap();
    let server = RunningServer::start(saving(dir.path())).await;
    let (players, _invite) = running(vec![server.client("ann").await]).await;
    // A member of a running game, and someone in no room at all.
    let outsider = server.client("eve").await;
    for client in [players[0].client(), &outsider.client] {
        let (_send, mut recv) = client
            .bulk()
            .open(BulkOpen::Fetch {
                snapshot: SnapshotId(FixedBytes([0x5a; 32])),
            })
            .await
            .unwrap();
        let answer = read_message::<BulkResponse>(&mut recv, BULK_RESPONSE_MAX_FRAME)
            .await
            .unwrap();
        assert_eq!(answer, BulkResponse::Unavailable);
    }
    server.shut_down().await;
}

#[tokio::test]
async fn nobody_uploads_a_world_nobody_asked_for() {
    let dir = tempfile::tempdir().unwrap();
    let server = RunningServer::start(saving(dir.path())).await;
    let (players, _invite) = running(vec![server.client("ann").await]).await;
    let (_send, mut recv) = players[0]
        .client()
        .bulk()
        .open(BulkOpen::Serve {
            snapshot: SnapshotId(FixedBytes([0x5a; 32])),
        })
        .await
        .unwrap();
    // The server ends the stream without asking for anything.
    let request = read_message::<BulkRequest>(&mut recv, BULK_REQUEST_MAX_FRAME).await;
    assert!(
        request.as_ref().is_err_and(|error| error.is_disconnect()),
        "{request:?}"
    );
    server.shut_down().await;
}

/// A save of `len` bytes, cut into a player's store: the world the owner
/// hands over.
fn owners_save(dir: &Path, len: usize) -> (Worlds, Vec<u8>, SavedWorld) {
    let worlds = Worlds::open(&dir.join("ann"), 1 << 30).unwrap();
    let bytes: Vec<u8> = (0..len).map(|i| (i * 7 % 251) as u8).collect();
    let file = dir.join("start.sav");
    std::fs::write(&file, &bytes).unwrap();
    let (_, world) = worlds.ingest_copy(&file).unwrap();
    assert!(file.is_file(), "the player's own save stays");
    (worlds, bytes, world)
}

/// Starts the room, waiting while the world it starts from is still on its
/// way: the room refuses to start before it has it.
async fn start_once_the_world_is_there(owner: &TestClient) {
    for _ in 0..200 {
        match owner.client.start_game().await {
            Ok(()) => return,
            Err(ClientError::Refused(RequestError::StartWorldPending)) => {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            Err(error) => panic!("the room did not start: {error}"),
        }
    }
    panic!("the world the room starts from never arrived");
}

#[tokio::test]
async fn a_room_starts_from_the_world_its_owner_handed_over() {
    let dir = tempfile::tempdir().unwrap();
    let server = RunningServer::start(saving(dir.path())).await;
    let mut ann = server.client("ann").await;
    let mut bob = server.client("bob").await;
    seat(&mut [&mut ann, &mut bob], FAST).await;
    let (worlds, bytes, world) = owners_save(dir.path(), 300_000);

    // Only the owner names it.
    assert_eq!(
        bob.client
            .requests()
            .done(Request::StartWorld(world))
            .await
            .unwrap_err(),
        ClientError::Refused(RequestError::NotOwner)
    );
    ann.client
        .requests()
        .done(Request::StartWorld(world))
        .await
        .unwrap();
    let asked = ann
        .wait_for(|event| match event {
            ClientEvent::Upload { event, snapshot } => Some((event, snapshot)),
            _ => None,
        })
        .await;
    assert_eq!(asked, (0, world.snapshot), "the room asks for it at once");
    assert_eq!(
        ann.client.start_game().await.unwrap_err(),
        ClientError::Refused(RequestError::StartWorldPending),
        "no game before the room has its world"
    );
    transfer::upload_world(&ann.client.bulk(), &worlds, world.snapshot)
        .await
        .unwrap();
    start_once_the_world_is_there(&ann).await;

    // Every member, the owner too, is handed that world to load, from the
    // game's first turn.
    for player in [&mut ann, &mut bob] {
        let start = player
            .wait_for(|event| match event {
                ClientEvent::TurnStream(start) => Some(start),
                _ => None,
            })
            .await;
        assert_eq!(
            start.world,
            Some(WorldOffer {
                snapshot: world.snapshot,
                size: world.size,
            })
        );
        assert_eq!(
            (start.next_turn, start.next_event, start.sealed_through),
            (1, 1, 0)
        );
    }
    // And can fetch it, byte for byte.
    let bobs = Worlds::open(&dir.path().join("bob"), 1 << 30).unwrap();
    let offer = WorldOffer {
        snapshot: world.snapshot,
        size: world.size,
    };
    let (file, _) = transfer::fetch_world(&bob.client.bulk(), &bobs, offer, |_| {})
        .await
        .unwrap();
    assert_eq!(std::fs::read(file).unwrap(), bytes);
    server.shut_down().await;
}

#[tokio::test]
async fn a_room_is_handed_no_world_where_the_server_keeps_none() {
    let dir = tempfile::tempdir().unwrap();
    let server = RunningServer::start(|_| {}).await;
    let mut ann = server.client("ann").await;
    seat(&mut [&mut ann], FAST).await;
    let (_worlds, _, world) = owners_save(dir.path(), 1000);
    assert_eq!(
        ann.client
            .requests()
            .done(Request::StartWorld(world))
            .await
            .unwrap_err(),
        ClientError::Refused(RequestError::WorldsNotKept)
    );
    // The room starts as before, from the owner's game.
    ann.client.start_game().await.unwrap();
    server.shut_down().await;
}

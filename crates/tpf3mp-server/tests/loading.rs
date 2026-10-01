//! Members' loading progress (`GameMessage::Loading`, PROTOCOL.md "Rooms"):
//! relayed to the room in its member list, at most about two a second while
//! it stays fetching, at once when the stage changes or ends.

#![allow(clippy::unwrap_used)]

mod common;

use std::time::{Duration, Instant};

use common::{FAST, RunningServer, join, room};
use tpf3mp_proto::{LoadingStage, PlayerId, RoomView};

fn stage_of(room: &RoomView, player: PlayerId) -> Option<LoadingStage> {
    room.members
        .iter()
        .find(|member| member.player == player)
        .and_then(|member| member.loading)
}

#[tokio::test]
async fn a_members_loading_progress_reaches_the_room_throttled() {
    let server = RunningServer::start(|_| {}).await;
    let mut ann = server.client("ann").await;
    let bob = server.client("bob").await;
    let (invite, _) = ann.client.create_room(room("loading", FAST)).await.unwrap();
    bob.client.join_room(join(&invite)).await.unwrap();
    let bob_id = bob.client.player();

    bob.client
        .report_loading(Some(LoadingStage::Fetching { percent: 10 }))
        .await
        .unwrap();
    ann.room_where(|room| stage_of(room, bob_id) == Some(LoadingStage::Fetching { percent: 10 }))
        .await;
    // A burst of fetching steps: the room shows at most about two a second,
    // so 11..=30 inside 400 ms do not all come through.
    let start = Instant::now();
    for percent in 11..=30u8 {
        bob.client
            .report_loading(Some(LoadingStage::Fetching { percent }))
            .await
            .unwrap();
    }
    assert!(
        start.elapsed() < Duration::from_millis(300),
        "a quick burst"
    );
    // A stage change shows at once, whatever came just before.
    bob.client
        .report_loading(Some(LoadingStage::Loading))
        .await
        .unwrap();
    let mut seen = Vec::new();
    ann.room_where(|room| {
        let stage = stage_of(room, bob_id);
        seen.push(stage);
        stage == Some(LoadingStage::Loading)
    })
    .await;
    let fetching = seen
        .iter()
        .filter(|stage| matches!(stage, Some(LoadingStage::Fetching { .. })))
        .count();
    assert!(fetching <= 2, "throttled: {seen:?}");
    // And its end.
    bob.client.report_loading(None).await.unwrap();
    ann.room_where(|room| {
        room.members
            .iter()
            .any(|member| member.player == bob_id && member.loading.is_none())
    })
    .await;
    server.shut_down().await;
}

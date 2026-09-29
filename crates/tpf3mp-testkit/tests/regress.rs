//! The regression harness against a real server: scenarios played through
//! the whole stack a game uses (client, bridge, link, session, gate), two
//! or more games to a room, judged by every replica's checks and by
//! comparing their worlds.

#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use tokio::sync::Mutex;
use tpf3mp_proto::Speed;

use tpf3mp_testkit::regress::{
    library,
    run::{HarnessPlan, LocalServer, Outcome, run_scenario},
    script::Scenario,
};

/// One room at a time, at a quarter of the harness's speed: test builds are
/// unoptimized and CI machines small, and a starved agent fails a room.
static ONE_AT_A_TIME: Mutex<()> = Mutex::const_new(());

fn gentle(server: &LocalServer, scenario: Arc<Scenario>) -> HarnessPlan {
    let mut plan = server.plan(scenario);
    plan.speed = Speed(400);
    plan
}

fn scenario(name: &str) -> Arc<Scenario> {
    library::scenarios()
        .into_iter()
        .find(|s| s.name == name)
        .unwrap()
}

fn show(outcome: &Outcome) -> String {
    let lag: Vec<_> = outcome.reports.iter().flat_map(|r| r.lag.clone()).collect();
    format!(
        "{}: {} steps in {:?}, lag {lag:?}, failures: {:#?}",
        outcome.scenario,
        outcome.steps(),
        outcome.elapsed,
        outcome.failures
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_smoke_scenarios_pass_in_rooms_of_two() {
    let _turn = ONE_AT_A_TIME.lock().await;
    let server = LocalServer::start().unwrap();
    let smoke: Vec<_> = library::scenarios()
        .into_iter()
        .filter(|s| s.smoke)
        .collect();
    assert!(smoke.len() >= 4);
    for scenario in smoke {
        let outcome = run_scenario(gentle(&server, scenario)).await.unwrap();
        assert!(outcome.passed(), "{}", show(&outcome));
        assert!(outcome.reports.len() >= 2);
        assert!(
            outcome
                .reports
                .iter()
                .all(|r| r.checks.iter().all(|c| c.failure.is_none()))
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn four_players_build_at_once() {
    let _turn = ONE_AT_A_TIME.lock().await;
    let server = LocalServer::start().unwrap();
    let outcome = run_scenario(gentle(&server, scenario("crowd")))
        .await
        .unwrap();
    assert!(outcome.passed(), "{}", show(&outcome));
    assert_eq!(outcome.reports.len(), 4);
    let sent: usize = outcome.reports.iter().map(|r| r.sent).sum();
    assert_eq!(sent, 28, "each of the four actors sent its seven actions");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_replica_that_drifts_fails_the_scenario() {
    let _turn = ONE_AT_A_TIME.lock().await;
    let server = LocalServer::start().unwrap();
    let mut plan = gentle(&server, scenario("bus-line"));
    plan.drift = Some((1, 600));
    let outcome = run_scenario(plan).await.unwrap();
    assert!(!outcome.passed());
    assert!(
        outcome
            .failures
            .iter()
            .any(|f| f.contains("diverged") || f.contains("differs from r0")),
        "{}",
        show(&outcome)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_long_run_of_quick_actions_stays_under_the_rooms_limit() {
    let _turn = ONE_AT_A_TIME.lock().await;
    let server = LocalServer::start().unwrap();
    // More acts than a room's burst, each ordered within milliseconds on
    // a fast machine: without pacing, the room refuses some as too fast.
    let mut script = library::Script::default();
    for i in 0..100 {
        let (a, b) = (library::at(i * 10, 0), library::at(i * 10, 5));
        script = script.act(0, library::road(vec![library::new(a), library::new(b)]));
    }
    let scenario = script
        .expect(tpf3mp_testkit::regress::script::Check::StreetEdges(100))
        .scenario("quick", "100 quick actions", false, 1);
    let mut plan = server.plan(Arc::new(scenario));
    plan.speed = Speed(400);
    let gap = plan.min_gap;
    let outcome = run_scenario(plan).await.unwrap();
    assert!(outcome.passed(), "{}", show(&outcome));
    // Where a round trip takes longer than the room's limit asks, pacing
    // never shows; so check it does not only by its absence of refusals.
    assert!(outcome.elapsed >= gap * 99, "{:?}", outcome.elapsed);
}

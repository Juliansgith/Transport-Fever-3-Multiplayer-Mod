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

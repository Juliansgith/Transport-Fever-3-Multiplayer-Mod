//! Players' diagnostics: the lines a client's recorder holds reach the
//! server, redacted, under the session its player sees as the support ID;
//! the admin endpoint lists and gives them; a server that keeps none says
//! so, and the client stops sending.

#![allow(clippy::unwrap_used)]

mod common;

use std::time::Duration;

use common::{RunningServer, WAIT, new_identity};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tpf3mp_agent::{ClientError, connect, diagnostics::Recorder};
use tpf3mp_proto::{
    DiagnosticBatch, DiagnosticEvent, DiagnosticLevel, Request, RequestError, Text,
};
use tpf3mp_server::{DiagnosticsConfig, serve_admin};

fn keeping(dir: &std::path::Path) -> impl FnOnce(&mut tpf3mp_server::ServerConfig) + '_ {
    |config| {
        config.diagnostics = Some(DiagnosticsConfig {
            dir: dir.to_owned(),
            keep_for: Duration::from_secs(3600),
            max_total: 1 << 30,
        });
    }
}

#[tokio::test]
async fn a_players_lines_reach_the_server_under_their_session() {
    let dir = tempfile::tempdir().unwrap();
    let server = RunningServer::start(keeping(dir.path())).await;
    let recorder = Recorder::new();
    let mut options = server.options(new_identity(), "ann");
    options.diagnostics = Some(recorder.clone());
    let (client, _events) = connect(options).await.unwrap();
    let session = client.welcome().session_id.to_string();
    recorder.record(
        DiagnosticLevel::Warn,
        "tpf3mp_agent::bridge",
        r"cannot load C:\Users\Ann\AppData\Local\TPF3-MP\worlds\w-1.sav from 192.0.2.7",
    );
    recorder.record(
        DiagnosticLevel::Info,
        "tpf3mp_launcher",
        "the launcher closes",
    );
    // Closing sends what is left, without waiting for the next round.
    client.close().await;

    let lines = tokio::time::timeout(WAIT, async {
        loop {
            if let Some(lines) = server.stats.session_diagnostics(&session).unwrap()
                && lines.iter().filter(|b| **b == b'\n').count() == 2
            {
                return String::from_utf8(lines).unwrap();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the lines reach the server");
    assert!(
        lines.contains("cannot load <path>/w-1.sav from <ip>"),
        "{lines}"
    );
    assert!(!lines.contains("Ann"), "{lines}");
    assert!(lines.contains(r#""level":"warn""#), "{lines}");
    assert!(lines.contains("the launcher closes"), "{lines}");
    assert!(recorder.is_empty(), "sent lines are not kept");

    // The operator reads them by the support ID.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let admin = listener.local_addr().unwrap();
    tokio::spawn(serve_admin(listener, server.stats.clone()));
    let (status, listed) = get(admin, "/diagnostics").await;
    assert_eq!(status, 200);
    assert!(listed.contains(&session), "{listed}");
    let (status, body) = get(admin, &format!("/diagnostics/{session}")).await;
    assert_eq!(status, 200);
    assert_eq!(body, lines);
    assert_eq!(get(admin, "/diagnostics/../../etc/passwd").await.0, 404);
    assert_eq!(
        get(admin, "/diagnostics/s-00000000000000000000000000000000")
            .await
            .0,
        404
    );
    server.shut_down().await;
}

#[tokio::test]
async fn a_server_that_keeps_none_says_so() {
    let server = RunningServer::start(|_| {}).await;
    let ann = server.client("ann").await;
    let refused = ann
        .client
        .request(Request::Diagnostics(batch(1)))
        .await
        .unwrap_err();
    assert!(
        matches!(
            refused,
            ClientError::Refused(RequestError::DiagnosticsNotKept)
        ),
        "{refused:?}"
    );
    server.shut_down().await;
}

#[tokio::test]
async fn diagnostics_never_use_up_a_players_other_requests() {
    let dir = tempfile::tempdir().unwrap();
    let server = RunningServer::start(keeping(dir.path())).await;
    let ann = server.client("ann").await;
    // Far more than the diagnostics budget allows at once.
    let mut limited = 0;
    for _ in 0..30 {
        match ann.client.request(Request::Diagnostics(batch(1))).await {
            Ok(_) => {}
            Err(ClientError::Refused(RequestError::RateLimited)) => limited += 1,
            Err(error) => panic!("{error:?}"),
        }
    }
    assert!(limited > 0, "diagnostics have a budget of their own");
    // Other requests still go through.
    ann.client
        .create_room(common::room("table", common::FAST))
        .await
        .unwrap();
    server.shut_down().await;
}

fn batch(len: usize) -> DiagnosticBatch {
    let event = DiagnosticEvent {
        at_ms: 1,
        level: DiagnosticLevel::Info,
        target: Text::new("tpf3mp").unwrap(),
        text: Text::new("a line").unwrap(),
    };
    DiagnosticBatch::new(vec![event; len]).unwrap()
}

/// One GET to the admin endpoint; its status code and body.
async fn get(address: std::net::SocketAddr, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).await.unwrap();
    let status = answer
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap();
    let body = answer
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_owned())
        .unwrap_or_default();
    (status, body)
}

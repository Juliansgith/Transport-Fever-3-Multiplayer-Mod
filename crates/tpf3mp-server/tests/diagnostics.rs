//! Players' diagnostics: the lines a client's recorder holds reach the
//! server, redacted, under the session its player sees as the support code,
//! every one with its source and the launcher's run; the hook's and the
//! game's logs go the same way; the admin endpoint lists and gives them, by
//! session, by run and by source; a server that keeps none says so, and
//! the client stops sending; switched off, nothing goes.

#![allow(clippy::unwrap_used)]

mod common;

use std::time::Duration;

use common::{RunningServer, WAIT, new_identity};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tpf3mp_agent::{ClientError, connect, diagnostics::Recorder, game_logs::Sources};
use tpf3mp_proto::{
    DiagnosticBatch, DiagnosticEvent, DiagnosticLevel, Request, RequestError, Text,
};
use tpf3mp_server::{DiagnosticsConfig, serve_admin};

fn keeping(dir: &std::path::Path) -> impl FnOnce(&mut tpf3mp_server::ServerConfig) + '_ {
    |config| {
        config.diagnostics = Some(DiagnosticsConfig::new(
            dir.to_owned(),
            Duration::from_secs(3600),
            1 << 30,
        ));
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
    // Every line carries the launcher's run and its source.
    let run = format!(r#""run":"{}""#, recorder.run());
    for line in lines.lines() {
        assert!(line.contains(&run), "{line}");
    }
    assert!(lines.contains(r#""source":"agent""#), "{lines}");
    assert!(lines.contains(r#""source":"launcher""#), "{lines}");

    // The operator reads them by the support code.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let admin = listener.local_addr().unwrap();
    tokio::spawn(serve_admin(listener, server.stats.clone()));
    let (status, listed) = get(admin, "/diagnostics").await;
    assert_eq!(status, 200);
    assert!(listed.contains(&session), "{listed}");
    // Each session names its player, by ID and by the name the lobby shows.
    assert!(listed.contains(r#""name": "ann""#), "{listed}");
    assert!(listed.contains(r#""player": "p-"#), "{listed}");
    // Found by name, in any case, and by nothing else.
    let (status, found) = get(admin, "/diagnostics?name=ANN").await;
    assert_eq!(status, 200);
    assert!(found.contains(&session), "{found}");
    let (_, none) = get(admin, "/diagnostics?name=bob").await;
    assert!(!none.contains(&session), "{none}");
    assert_eq!(get(admin, "/diagnostics?nope=1").await.0, 404);
    let (status, body) = get(admin, &format!("/diagnostics/{session}")).await;
    assert_eq!(status, 200);
    // First who it is, then the lines.
    let (who, rest) = body.split_once('\n').unwrap();
    assert!(who.starts_with(r#"{"who":"#), "{who}");
    assert!(who.contains(r#""kind":"session""#), "{who}");
    assert!(who.contains(r#""name":"ann""#), "{who}");
    assert_eq!(rest, lines);
    assert_eq!(get(admin, "/diagnostics/../../etc/passwd").await.0, 404);
    let other = if session == "AB2CD3" {
        "EF4GH5"
    } else {
        "AB2CD3"
    };
    assert_eq!(get(admin, &format!("/diagnostics/{other}")).await.0, 404);
    server.shut_down().await;
}

/// The hook's log and the game's, from where they stood when the run
/// began, and the game's error reports, go to the server played on,
/// redacted, each line tagged; the operator reads them by the run's code,
/// one source at a time; the opt-out stops them.
#[tokio::test]
async fn the_hooks_and_the_games_logs_reach_the_server_by_source() {
    let dir = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let hook = files.path().join("hook.log");
    let crash = files.path().join("crash_dump");
    std::fs::create_dir_all(&crash).unwrap();
    std::fs::write(&hook, "[0] from an earlier game\n").unwrap();
    std::fs::write(crash.join("stdout.txt"), "").unwrap();
    let server = RunningServer::start(keeping(dir.path())).await;
    let recorder = Recorder::new();
    let mut sources = Sources::new(hook.clone(), vec![crash.clone()]);
    let mut options = server.options(new_identity(), "ann");
    options.diagnostics = Some(recorder.clone());
    let (client, _events) = connect(options).await.unwrap();
    let append = |path: &std::path::Path, text: &str| {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    };
    append(&hook, "[1] loading C:\\Users\\Ann\\Saved Games\\x.sav\n");
    append(
        &crash.join("stdout.txt"),
        "[2026-10-02 15:53:49Z - ERROR    - Main Thread - Main ]  oops\n",
    );
    std::fs::write(crash.join("e_1.json"), "\"userId\": \"125253817\",\n").unwrap();
    std::fs::write(crash.join("e_0.dmp"), [0u8; 16]).unwrap();
    sources.record(&recorder, std::time::Instant::now(), true);
    client.close().await;

    let run = recorder.run().to_string();
    let all = tokio::time::timeout(WAIT, async {
        loop {
            if let Some(lines) = server.stats.diagnostics_of(&run, None).unwrap()
                && lines.iter().filter(|b| **b == b'\n').count() == 3
            {
                return String::from_utf8(lines).unwrap();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the lines reach the server");
    assert!(!all.contains("earlier game"), "from the run's start: {all}");
    assert!(!all.contains("Ann") && !all.contains("125253817"), "{all}");
    assert!(all.contains("<path>/x.sav"), "{all}");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let admin = listener.local_addr().unwrap();
    tokio::spawn(serve_admin(listener, server.stats.clone()));
    for (source, file) in [
        ("hook", "hook.log"),
        ("game", "stdout.txt"),
        ("crash", "e_1.json"),
    ] {
        let (status, body) = get(admin, &format!("/diagnostics/{run}?source={source}")).await;
        assert_eq!(status, 200);
        assert_eq!(
            body.lines().count(),
            2,
            "who, then one line: {source}: {body}"
        );
        assert!(body.starts_with(r#"{"who":{"code":""#), "{body}");
        assert!(
            body.lines().next().unwrap().contains(r#""kind":"run""#),
            "{body}"
        );
        assert!(body.contains(&format!(r#""target":"{file}""#)), "{body}");
    }
    assert_eq!(
        get(admin, &format!("/diagnostics/{run}?source=dmp"))
            .await
            .0,
        404
    );

    // Off: nothing more is read or sent.
    recorder.set_on(false);
    append(&hook, "[2] after the switch\n");
    sources.record(&recorder, std::time::Instant::now(), true);
    assert!(recorder.is_empty());
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

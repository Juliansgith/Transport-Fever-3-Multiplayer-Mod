//! The operator's notices: told to everyone connected, in a room or not,
//! from the server's handle and through the admin endpoint.

#![allow(clippy::unwrap_used)]

mod common;

use common::{FAST, RunningServer, room};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tpf3mp_agent::ClientEvent;
use tpf3mp_proto::Text;
use tpf3mp_server::serve_admin;

#[tokio::test]
async fn everyone_connected_hears_the_operator() {
    let server = RunningServer::start(|_| {}).await;
    let mut ann = server.client("ann").await;
    let mut bob = server.client("bob").await;
    ann.client.create_room(room("table", FAST)).await.unwrap();
    let told = server
        .stats
        .announce(Text::new("Restarting for an update in 5 minutes").unwrap());
    assert_eq!(told, 2, "one in a room, one not");
    for player in [&mut ann, &mut bob] {
        let text = player
            .wait_for(|event| match event {
                ClientEvent::Notice(text) => Some(text),
                _ => None,
            })
            .await;
        assert_eq!(text.as_str(), "Restarting for an update in 5 minutes");
    }
    server.shut_down().await;
}

/// One request to the admin endpoint; its status code and body.
async fn post(address: std::net::SocketAddr, body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let request = format!(
        "POST /announce HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
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

#[tokio::test]
async fn the_admin_endpoint_announces_a_line_of_text() {
    let server = RunningServer::start(|_| {}).await;
    let mut ann = server.client("ann").await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let admin = listener.local_addr().unwrap();
    let serving = tokio::spawn(serve_admin(listener, server.stats.clone()));

    let (status, body) = post(admin, "Back in two minutes").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, "told 1 connections\n");
    let text = ann
        .wait_for(|event| match event {
            ClientEvent::Notice(text) => Some(text),
            _ => None,
        })
        .await;
    assert_eq!(text.as_str(), "Back in two minutes");

    // Nothing to say, or too much, is refused and told to nobody.
    assert_eq!(post(admin, "   ").await.0, 400);
    assert_eq!(post(admin, &"x".repeat(281)).await.0, 400);

    serving.abort();
    server.shut_down().await;
}

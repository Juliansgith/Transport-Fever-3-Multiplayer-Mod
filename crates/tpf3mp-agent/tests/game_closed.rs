//! A game that closes while its session runs. Once its hook attached, the
//! launcher's word that the game's process has exited ends the session at
//! once, as a hook that stopped responding does: not after the heartbeat
//! limit, which while a world loads is ten minutes, during which the
//! player could not start the game again. Seen in a two-player test.

#![allow(clippy::unwrap_used)]

use std::{collections::VecDeque, net::SocketAddr, sync::Arc, time::Duration};

use tokio::{sync::mpsc, task::JoinHandle};
use tpf3mp_agent::{
    Client, ConnectOptions, Events,
    bridge::{Bridge, BridgeFault, BridgeOptions, Control, HookLink},
    connect,
};
use tpf3mp_bridge::{BRIDGE_VERSION, ToAgent, encode};
use tpf3mp_net::{
    Identity, ServerIdentity, ServerTrust, read_message, read_preamble, server_config,
    write_message, write_preamble,
};
use tpf3mp_proto::{
    CONTROL_MAX_FRAME, ClientMessage, PROTOCOL_VERSION, ServerMessage, SessionId, Text, Welcome,
};

/// A hook that says what it is given to say, takes everything, and whose
/// heartbeat never moves: a game that is gone, or still loading.
struct ScriptedHook {
    said: VecDeque<Vec<u8>>,
}

impl HookLink for ScriptedHook {
    fn send(&mut self, _message: &[u8]) -> Result<bool, BridgeFault> {
        Ok(true)
    }

    fn recv(&mut self, buf: &mut Vec<u8>) -> Result<bool, BridgeFault> {
        match self.said.pop_front() {
            Some(message) => {
                *buf = message;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn heartbeat(&mut self) {}

    fn peer_heartbeat(&self) -> u64 {
        0
    }
}

fn hello() -> Vec<u8> {
    encode(&ToAgent::Hello {
        version: BRIDGE_VERSION,
        build: Text::new("test").unwrap(),
    })
    .unwrap()
}

/// A server that welcomes one client and then listens to it.
async fn connected() -> (Client, Events, JoinHandle<()>) {
    let identity = ServerIdentity::self_signed(&["localhost"]).unwrap();
    let leaf = identity.leaf().clone();
    let endpoint = quinn::Endpoint::server(
        server_config(identity).unwrap(),
        "127.0.0.1:0".parse().unwrap(),
    )
    .unwrap();
    let address: SocketAddr = endpoint.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let connection = endpoint.accept().await.unwrap().await.unwrap();
        let (mut send, mut recv) = connection.accept_bi().await.unwrap();
        read_preamble(&mut recv).await.unwrap();
        write_preamble(&mut send, PROTOCOL_VERSION).await.unwrap();
        let _hello: ClientMessage = read_message(&mut recv, CONTROL_MAX_FRAME).await.unwrap();
        let welcome = ServerMessage::Welcome(Welcome {
            server_version: Text::new("test").unwrap(),
            session_id: SessionId("AB2CD3".parse().unwrap()),
            rules: Vec::new(),
        });
        write_message(&mut send, &welcome, CONTROL_MAX_FRAME)
            .await
            .unwrap();
        while read_message::<ClientMessage>(&mut recv, CONTROL_MAX_FRAME)
            .await
            .is_ok()
        {}
        drop((endpoint, connection, send));
    });
    let player = Arc::new(Identity::generate().unwrap().0);
    let (client, events) = connect(ConnectOptions::new(
        address,
        "localhost",
        ServerTrust::Pinned(leaf),
        player,
        Text::new("player").unwrap(),
    ))
    .await
    .unwrap();
    (client, events, server)
}

/// Runs a bridge on `hook`, the heartbeat limits the defaults (minutes),
/// and tells it the game closed.
async fn closed_game(hook: ScriptedHook) -> JoinHandle<Result<(), BridgeFault>> {
    let (client, mut events, server) = connected().await;
    let (controls, controls_rx) = mpsc::channel(4);
    controls.send(Control::GameClosed).await.unwrap();
    tokio::spawn(async move {
        let mut bridge = Bridge::new(hook, BridgeOptions::default()).with_controls(controls_rx);
        let ended = bridge.run(&client, &mut events).await;
        drop(controls);
        server.abort();
        ended.map(|_| ())
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_game_ends_its_session_at_once() {
    let session = closed_game(ScriptedHook {
        said: VecDeque::from([hello()]),
    })
    .await;
    let ended = tokio::time::timeout(Duration::from_secs(5), session)
        .await
        .expect("the session ends when the game closes, not after the heartbeat limit")
        .unwrap();
    assert!(
        matches!(ended, Err(BridgeFault::GameClosed)),
        "the session failed for the game closing: {ended:?}"
    );
    assert_eq!(
        BridgeFault::GameClosed.to_string(),
        "Transport Fever 3 closed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_game_that_closed_before_it_joined_leaves_the_session_waiting() {
    // No hook attached: the session keeps waiting for the next game.
    let session = closed_game(ScriptedHook {
        said: VecDeque::new(),
    })
    .await;
    let waiting = tokio::time::timeout(Duration::from_secs(1), session).await;
    assert!(waiting.is_err(), "the session ended: {waiting:?}");
}

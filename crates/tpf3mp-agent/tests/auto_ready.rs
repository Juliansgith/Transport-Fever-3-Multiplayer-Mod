//! A player is marked ready by the agent once their game has a world up with
//! the mod linked (`ToAgent::WorldUp`), in the room's lobby: nobody has to
//! press Ready. Once a world: a player who then says Not ready stays so until
//! another world is up. Never outside the lobby.

#![allow(clippy::unwrap_used)]

use std::{
    net::SocketAddr,
    sync::{Arc, mpsc as std_mpsc},
    time::Duration,
};

use tokio::sync::mpsc;
use tpf3mp_agent::{
    ConnectOptions,
    bridge::{Bridge, BridgeFault, BridgeOptions, Control, HookLink},
    connect,
};
use tpf3mp_bridge::{BRIDGE_VERSION, ToAgent, encode};
use tpf3mp_net::{
    Identity, ServerIdentity, ServerTrust, read_message, read_preamble, server_config,
    write_message, write_preamble,
};
use tpf3mp_proto::{
    CONTROL_MAX_FRAME, ChatText, ClientMessage, FixedBytes, PROTOCOL_VERSION, PlayerId, Request,
    Response, RoomId, RoomPhase, RoomSettings, RoomView, RulesName, ServerMessage, SessionId, Text,
    Welcome,
};

/// A hook that says what the test hands it, when it does, and takes
/// everything.
struct ScriptedHook {
    said: std_mpsc::Receiver<Vec<u8>>,
}

impl HookLink for ScriptedHook {
    fn send(&mut self, _message: &[u8]) -> Result<bool, BridgeFault> {
        Ok(true)
    }

    fn recv(&mut self, buf: &mut Vec<u8>) -> Result<bool, BridgeFault> {
        match self.said.try_recv() {
            Ok(message) => {
                *buf = message;
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }

    fn heartbeat(&mut self) {}

    fn peer_heartbeat(&self) -> u64 {
        0
    }
}

fn room(phase: RoomPhase) -> RoomView {
    RoomView {
        id: RoomId(FixedBytes([3; 16])),
        name: Text::new("Friday trains").unwrap(),
        rules: RulesName::new("native").unwrap(),
        owner: PlayerId(FixedBytes([1; 32])),
        max_players: 4,
        has_password: false,
        phase,
        settings: RoomSettings {
            steps_per_second: 5,
            input_delay_ms: 100,
            checkpoint_interval: 50,
        },
        members: Vec::new(),
    }
}

/// A server that completes the handshake, announces a room in `phase`
/// `announce_after` later, answers every request done and passes it on.
async fn server(
    phase: RoomPhase,
    announce_after: Duration,
) -> (SocketAddr, ServerTrust, mpsc::UnboundedReceiver<Request>) {
    let identity = ServerIdentity::self_signed(&["localhost"]).unwrap();
    let leaf = identity.leaf().clone();
    let endpoint = quinn::Endpoint::server(
        server_config(identity).unwrap(),
        "127.0.0.1:0".parse().unwrap(),
    )
    .unwrap();
    let address = endpoint.local_addr().unwrap();
    let (heard_tx, heard) = mpsc::unbounded_channel();
    tokio::spawn(async move {
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
        tokio::time::sleep(announce_after).await;
        write_message(
            &mut send,
            &ServerMessage::RoomUpdate(room(phase)),
            CONTROL_MAX_FRAME,
        )
        .await
        .unwrap();
        while let Ok(message) = read_message::<ClientMessage>(&mut recv, CONTROL_MAX_FRAME).await {
            if let ClientMessage::Request { id, request } = message {
                let _ = heard_tx.send(request);
                let answer = ServerMessage::Response {
                    id,
                    result: Ok(Response::Done),
                };
                if write_message(&mut send, &answer, CONTROL_MAX_FRAME)
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
        drop((endpoint, connection, send));
    });
    (address, ServerTrust::Pinned(leaf), heard)
}

/// What the test drives: the hook's words, the front end's controls and what
/// the server heard.
struct Session {
    hook: std_mpsc::Sender<Vec<u8>>,
    controls: mpsc::Sender<Control>,
    heard: mpsc::UnboundedReceiver<Request>,
    bridge: tokio::task::JoinHandle<()>,
}

impl Session {
    async fn start(phase: RoomPhase) -> Self {
        Self::announced_after(phase, Duration::ZERO).await
    }

    async fn announced_after(phase: RoomPhase, announce_after: Duration) -> Self {
        let (address, trust, heard) = server(phase, announce_after).await;
        let player = Arc::new(Identity::generate().unwrap().0);
        let (client, mut events) = connect(ConnectOptions::new(
            address,
            "localhost",
            trust,
            player,
            Text::new("player").unwrap(),
        ))
        .await
        .unwrap();
        let (hook, said) = std_mpsc::channel();
        let (controls, controls_rx) = mpsc::channel(8);
        let bridge = tokio::spawn(async move {
            let mut bridge = Bridge::new(ScriptedHook { said }, BridgeOptions::default())
                .with_controls(controls_rx);
            let _ = bridge.run(&client, &mut events).await;
        });
        let session = Self {
            hook,
            controls,
            heard,
            bridge,
        };
        session.hook_says(&ToAgent::Hello {
            version: BRIDGE_VERSION,
            build: Text::new("test").unwrap(),
        });
        session
    }

    fn hook_says(&self, message: &ToAgent) {
        self.hook.send(encode(message).unwrap()).unwrap();
    }

    /// The next request the server hears.
    async fn next(&mut self) -> Request {
        tokio::time::timeout(Duration::from_secs(20), self.heard.recv())
            .await
            .expect("the server hears a request")
            .unwrap()
    }

    /// The requests the server hears until the hook's chat `marker`, which
    /// the hook says after what the test watches, and a moment more.
    async fn until(&mut self, marker: &str) -> Vec<Request> {
        self.hook_says(&ToAgent::Chat {
            text: ChatText::new(marker).unwrap(),
        });
        let mut heard = Vec::new();
        loop {
            match self.next().await {
                Request::Chat(text) if text.as_str() == marker => break,
                other => heard.push(other),
            }
        }
        // Requests go out on tasks of their own: one sent before the
        // marker may land just after it.
        while let Ok(Some(request)) =
            tokio::time::timeout(Duration::from_millis(300), self.heard.recv()).await
        {
            heard.push(request);
        }
        heard
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.bridge.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_world_up_in_the_lobby_marks_the_player_ready() {
    let mut session = Session::start(RoomPhase::Lobby).await;
    assert!(
        session.until("attached").await.is_empty(),
        "an attached hook alone marks nobody ready"
    );
    session.hook_says(&ToAgent::WorldUp { world: 1 });
    assert_eq!(session.next().await, Request::SetReady(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn not_ready_after_a_world_up_stays_until_another_world_is_up() {
    let mut session = Session::start(RoomPhase::Lobby).await;
    session.hook_says(&ToAgent::WorldUp { world: 1 });
    assert_eq!(session.next().await, Request::SetReady(true));
    // The player presses Not ready.
    session.controls.send(Control::Ready(false)).await.unwrap();
    assert_eq!(session.next().await, Request::SetReady(false));
    // The same world again, or an older one: the player stays not ready.
    session.hook_says(&ToAgent::WorldUp { world: 1 });
    session.hook_says(&ToAgent::WorldUp { world: 0 });
    let heard = session.until("same world").await;
    assert!(
        !heard.contains(&Request::SetReady(true)),
        "the player said Not ready for this world: {heard:?}"
    );
    // Another world: ready again.
    session.hook_says(&ToAgent::WorldUp { world: 2 });
    assert_eq!(session.next().await, Request::SetReady(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_world_up_outside_the_lobby_marks_nobody_ready() {
    let mut session = Session::start(RoomPhase::Running).await;
    session.hook_says(&ToAgent::WorldUp { world: 1 });
    let heard = session.until("running").await;
    assert!(
        !heard.contains(&Request::SetReady(true)),
        "the room's game runs: {heard:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_world_up_before_the_room_is_announced_waits_for_the_lobby() {
    let mut session = Session::announced_after(RoomPhase::Lobby, Duration::from_millis(500)).await;
    session.hook_says(&ToAgent::WorldUp { world: 1 });
    assert_eq!(session.next().await, Request::SetReady(true));
}

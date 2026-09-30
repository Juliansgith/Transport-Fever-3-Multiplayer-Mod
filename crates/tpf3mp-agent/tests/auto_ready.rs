//! A player is marked ready by the agent once their game has a world up with
//! the mod linked (`ToAgent::WorldUp`), in the room's lobby: nobody has to
//! press Ready. Once a world: a player who then says Not ready stays so until
//! another world is up. Never outside the lobby. A game at its main menu
//! (`ToAgent::MenuUp`) marks a guest ready too, since it loads the room's
//! world from there, but never the room's owner, whose world the room plays.

#![allow(clippy::unwrap_used)]

use std::{
    net::SocketAddr,
    sync::{Arc, mpsc as std_mpsc},
    time::Duration,
};

use tokio::sync::mpsc;
use tpf3mp_agent::{
    ConnectOptions, Worlds,
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

fn room(phase: RoomPhase, owner: PlayerId) -> RoomView {
    RoomView {
        id: RoomId(FixedBytes([3; 16])),
        name: Text::new("Friday trains").unwrap(),
        rules: RulesName::new("native").unwrap(),
        owner,
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
    owner: PlayerId,
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
            &ServerMessage::RoomUpdate(room(phase, owner)),
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

/// Whose game is at the menu.
#[derive(Clone, Copy)]
enum Seat {
    Guest,
    Owner,
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
        Self::with(phase, announce_after, Seat::Guest, None).await
    }

    /// At the main menu: a guest or the room's owner, with an agent that
    /// keeps worlds in `worlds` or none.
    async fn at_menu(phase: RoomPhase, seat: Seat, worlds: Option<Worlds>) -> Self {
        Self::with(phase, Duration::ZERO, seat, worlds).await
    }

    async fn with(
        phase: RoomPhase,
        announce_after: Duration,
        seat: Seat,
        worlds: Option<Worlds>,
    ) -> Self {
        let player = Arc::new(Identity::generate().unwrap().0);
        let owner = match seat {
            Seat::Owner => player.player(),
            Seat::Guest => PlayerId(FixedBytes([1; 32])),
        };
        let (address, trust, heard) = server(phase, announce_after, owner).await;
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
            let options = BridgeOptions {
                worlds,
                ..BridgeOptions::default()
            };
            let mut bridge = Bridge::new(ScriptedHook { said }, options).with_controls(controls_rx);
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

fn worlds(tag: &str) -> Worlds {
    let dir = std::env::temp_dir().join(format!("tpf3mp-auto-ready-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Worlds::open(&dir, 1 << 30).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_guest_at_the_main_menu_is_marked_ready_once_per_arrival() {
    let mut session = Session::at_menu(RoomPhase::Lobby, Seat::Guest, Some(worlds("guest"))).await;
    session.hook_says(&ToAgent::MenuUp { menu: 1 });
    assert_eq!(session.next().await, Request::SetReady(true));
    session.controls.send(Control::Ready(false)).await.unwrap();
    assert_eq!(session.next().await, Request::SetReady(false));
    session.hook_says(&ToAgent::MenuUp { menu: 1 });
    let heard = session.until("same menu").await;
    assert!(
        !heard.contains(&Request::SetReady(true)),
        "Not ready holds for this arrival: {heard:?}"
    );
    session.hook_says(&ToAgent::MenuUp { menu: 2 });
    assert_eq!(session.next().await, Request::SetReady(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_owner_at_the_main_menu_is_not_marked_ready() {
    let mut session = Session::at_menu(RoomPhase::Lobby, Seat::Owner, Some(worlds("owner"))).await;
    session.hook_says(&ToAgent::MenuUp { menu: 1 });
    let heard = session.until("owner at menu").await;
    assert!(
        !heard.contains(&Request::SetReady(true)),
        "the room plays the owner's world, which needs one up: {heard:?}"
    );
    // A world up is the owner's way to ready.
    session.hook_says(&ToAgent::WorldUp { world: 1 });
    assert_eq!(session.next().await, Request::SetReady(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_guest_at_the_menu_without_worlds_or_outside_the_lobby_is_not_marked_ready() {
    let mut session = Session::at_menu(RoomPhase::Lobby, Seat::Guest, None).await;
    session.hook_says(&ToAgent::MenuUp { menu: 1 });
    let heard = session.until("no worlds").await;
    assert!(!heard.contains(&Request::SetReady(true)), "{heard:?}");

    let mut session =
        Session::at_menu(RoomPhase::Running, Seat::Guest, Some(worlds("running"))).await;
    session.hook_says(&ToAgent::MenuUp { menu: 1 });
    let heard = session.until("running").await;
    assert!(!heard.contains(&Request::SetReady(true)), "{heard:?}");
}

//! QUIC over plain UDP sockets, the fallback for network stacks that refuse
//! quinn's socket options (Wine and Proton): a connection carries streams
//! both ways, large enough to take many datagrams, against a server on
//! quinn's own socket and one on a plain socket.

#![allow(clippy::unwrap_used)]

use std::{net::Ipv4Addr, sync::Arc};

use quinn::{AsyncUdpSocket, Endpoint, EndpointConfig, TokioRuntime};
use tpf3mp_net::{ServerIdentity, ServerTrust, client_config, server_config, udp};

fn plain_socket() -> Arc<dyn AsyncUdpSocket> {
    udp::plain(std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap()).unwrap()
}

/// An echo server on `socket`, and the trust a client needs for it.
fn echo_server(socket: Arc<dyn AsyncUdpSocket>) -> (Endpoint, ServerTrust) {
    let identity = ServerIdentity::self_signed(&["localhost"]).unwrap();
    let trust = ServerTrust::Pinned(identity.leaf().clone());
    let endpoint = Endpoint::new_with_abstract_socket(
        EndpointConfig::default(),
        Some(server_config(identity).unwrap()),
        socket,
        Arc::new(TokioRuntime),
    )
    .unwrap();
    let accepting = endpoint.clone();
    tokio::spawn(async move {
        while let Some(incoming) = accepting.accept().await {
            tokio::spawn(async move {
                let connection = incoming.await.unwrap();
                while let Ok((mut send, mut recv)) = connection.accept_bi().await {
                    let data = recv.read_to_end(16 << 20).await.unwrap();
                    send.write_all(&data).await.unwrap();
                    send.finish().unwrap();
                    let _ = send.stopped().await;
                }
            });
        }
    });
    (endpoint, trust)
}

async fn echo_through(server: &Endpoint, trust: ServerTrust) {
    let mut client = Endpoint::new_with_abstract_socket(
        EndpointConfig::default(),
        None,
        plain_socket(),
        Arc::new(TokioRuntime),
    )
    .unwrap();
    client.set_default_client_config(client_config(trust).unwrap());
    let connection = client
        .connect(server.local_addr().unwrap(), "localhost")
        .unwrap()
        .await
        .unwrap();
    let data: Vec<u8> = (0..2_000_000u32).map(|i| (i % 251) as u8).collect();
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    send.write_all(&data).await.unwrap();
    send.finish().unwrap();
    assert_eq!(recv.read_to_end(16 << 20).await.unwrap(), data);
    connection.close(0u32.into(), b"");
    client.wait_idle().await;
}

#[tokio::test]
async fn a_plain_socket_carries_quic_to_a_plain_server() {
    let (server, trust) = echo_server(plain_socket());
    echo_through(&server, trust).await;
}

#[tokio::test]
async fn a_plain_socket_carries_quic_to_quinns_own_socket() {
    let (server, trust) = echo_server(udp::bind((Ipv4Addr::LOCALHOST, 0).into()).unwrap());
    echo_through(&server, trust).await;
}

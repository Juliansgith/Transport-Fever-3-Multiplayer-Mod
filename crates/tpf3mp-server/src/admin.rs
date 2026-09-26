//! The admin endpoint: Prometheus metrics at `/metrics`, a health check at
//! `/healthz`, and `POST /announce`, which tells everyone connected the
//! request's body (up to 280 bytes of UTF-8), over plain HTTP. It has no
//! authentication, so it must only listen on a private address: loopback,
//! or a VPN interface.

use std::{sync::Arc, time::Duration};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
};
use tracing::warn;

use tpf3mp_proto::ChatText;

use crate::ServerStats;

/// Largest request head the endpoint reads.
const MAX_REQUEST: usize = 8 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Requests answered at once; more wait in the listen backlog. A scraper
/// needs one.
const MAX_CONNECTIONS: usize = 16;
/// Pause after a failed accept, such as when the process is out of file
/// descriptors, so the loop neither spins nor gives up.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// Serves the admin endpoint for as long as the server runs. A failed
/// accept is logged and retried: the endpoint must not vanish silently.
pub async fn serve_admin(listener: TcpListener, stats: ServerStats) {
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let Ok(slot) = Arc::clone(&slots).acquire_owned().await else {
            return;
        };
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(error) => {
                warn!(%error, "the admin endpoint cannot accept a connection");
                tokio::time::sleep(ACCEPT_BACKOFF).await;
                continue;
            }
        };
        let stats = stats.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(REQUEST_TIMEOUT, answer(stream, &stats)).await;
            drop(slot);
        });
    }
}

async fn answer(mut stream: TcpStream, stats: &ServerStats) -> std::io::Result<()> {
    let mut head = Vec::with_capacity(512);
    let mut chunk = [0; 512];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await?;
        if read == 0 || head.len() + read > MAX_REQUEST {
            return Ok(());
        }
        head.extend_from_slice(&chunk[..read]);
    }
    let end = head
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map_or(head.len(), |at| at + 4);
    let request_line = head.split(|byte| *byte == b'\r').next().unwrap_or_default();
    let (status, content_type, body) = match request_line {
        b"POST /announce HTTP/1.1" | b"POST /announce HTTP/1.0" => {
            match read_body(&mut stream, &head[..end], head[end..].to_vec()).await? {
                Some(text) => match ChatText::new(text.trim()) {
                    Ok(text) if !text.as_str().is_empty() => {
                        let told = stats.announce(text);
                        ("200 OK", "text/plain", format!("told {told} connections\n"))
                    }
                    _ => (
                        "400 Bad Request",
                        "text/plain",
                        "the notice must be 1 to 280 bytes of printable text\n".to_owned(),
                    ),
                },
                None => (
                    "400 Bad Request",
                    "text/plain",
                    "send the notice as the body, with a Content-Length\n".to_owned(),
                ),
            }
        }
        b"GET /metrics HTTP/1.1" | b"GET /metrics HTTP/1.0" => (
            "200 OK",
            "text/plain; version=0.0.4",
            stats.render_metrics(),
        ),
        b"GET /healthz HTTP/1.1" | b"GET /healthz HTTP/1.0" => {
            ("200 OK", "text/plain", "ok\n".to_owned())
        }
        _ => ("404 Not Found", "text/plain", "not found\n".to_owned()),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}

/// The request's body as UTF-8, read to its `Content-Length`, of at most a
/// kilobyte: a notice is a line.
async fn read_body(
    stream: &mut TcpStream,
    head: &[u8],
    mut body: Vec<u8>,
) -> std::io::Result<Option<String>> {
    const MAX_BODY: usize = 1024;
    let head = String::from_utf8_lossy(head);
    let Some(length) = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    }) else {
        return Ok(None);
    };
    if length > MAX_BODY {
        return Ok(None);
    }
    let mut chunk = [0; 512];
    while body.len() < length {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(None);
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(length);
    Ok(String::from_utf8(body).ok())
}

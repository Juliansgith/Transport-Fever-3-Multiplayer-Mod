//! UDP sockets for QUIC endpoints, with a fallback for network stacks that
//! refuse quinn's socket options.
//!
//! quinn sets a socket up for segmentation offload, ECN and packet info
//! before it uses it, and gives up when the stack refuses one of those
//! options. Wine and Proton, which run the Windows build on Linux and the
//! Steam Deck, refuse `getsockopt(IPV6_V6ONLY)` on an IPv4 socket
//! (WSAEOPNOTSUPP, "OS Error 10045"), and quinn-udp asks it of every socket,
//! up to its latest release. [`bind`] then falls back to a plain socket:
//! one datagram per call, no ECN, which QUIC runs over as well, only
//! without those optimisations.

use std::{
    fmt,
    future::Future,
    io::{self, IoSliceMut},
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, ready},
};

use quinn::{
    AsyncUdpSocket, Runtime, TokioRuntime, UdpPoller,
    udp::{RecvMeta, Transmit, UdpSocketState},
};
use tokio::io::ReadBuf;

/// Binds a UDP socket on `addr` for a QUIC endpoint: quinn's own socket
/// where the network stack takes its options, else a plain one, with a
/// warning that says why.
pub fn bind(addr: SocketAddr) -> io::Result<Arc<dyn AsyncUdpSocket>> {
    wrap(std::net::UdpSocket::bind(addr)?)
}

/// [`bind`] for a socket already bound.
pub fn wrap(socket: std::net::UdpSocket) -> io::Result<Arc<dyn AsyncUdpSocket>> {
    match UdpSocketState::new((&socket).into()) {
        Ok(_) => TokioRuntime.wrap_udp_socket(socket),
        Err(error) => {
            tracing::warn!(
                %error,
                "the network stack refuses quinn's UDP socket options (as Wine and Proton do); \
                 using a plain UDP socket"
            );
            plain(socket)
        }
    }
}

/// A plain UDP socket for a QUIC endpoint: no socket options beyond
/// non-blocking, one datagram per send and per receive.
pub fn plain(socket: std::net::UdpSocket) -> io::Result<Arc<dyn AsyncUdpSocket>> {
    socket.set_nonblocking(true)?;
    Ok(Arc::new(PlainSocket {
        io: tokio::net::UdpSocket::from_std(socket)?,
    }))
}

struct PlainSocket {
    io: tokio::net::UdpSocket,
}

impl fmt::Debug for PlainSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlainSocket")
            .field("local", &self.io.local_addr().ok())
            .finish()
    }
}

impl AsyncUdpSocket for PlainSocket {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        Box::pin(Writable {
            socket: self,
            waiting: None,
        })
    }

    fn try_send(&self, transmit: &Transmit) -> io::Result<()> {
        // max_transmit_segments is 1, so a transmit is one datagram; split
        // it anyway should quinn ever batch.
        let size = transmit
            .segment_size
            .unwrap_or(transmit.contents.len())
            .max(1);
        for datagram in transmit.contents.chunks(size) {
            self.io.try_send_to(datagram, transmit.destination)?;
        }
        Ok(())
    }

    fn poll_recv(
        &self,
        cx: &mut Context,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        let (Some(buf), Some(meta)) = (bufs.first_mut(), meta.first_mut()) else {
            return Poll::Ready(Ok(0));
        };
        let mut read = ReadBuf::new(buf);
        let addr = ready!(self.io.poll_recv_from(cx, &mut read))?;
        let len = read.filled().len();
        *meta = RecvMeta {
            addr,
            len,
            stride: len,
            ecn: None,
            dst_ip: None,
        };
        Poll::Ready(Ok(1))
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.io.local_addr()
    }
}

type WritableFuture = Pin<Box<dyn Future<Output = io::Result<()>> + Send + Sync>>;

/// Waits for the socket to take a datagram, each poller with its own
/// waker, as quinn asks.
struct Writable {
    socket: Arc<PlainSocket>,
    waiting: Option<WritableFuture>,
}

impl fmt::Debug for Writable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Writable").finish_non_exhaustive()
    }
}

impl UdpPoller for Writable {
    fn poll_writable(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        let this = &mut *self;
        let waiting = this.waiting.get_or_insert_with(|| {
            let socket = Arc::clone(&this.socket);
            Box::pin(async move { socket.io.writable().await })
        });
        let result = ready!(waiting.as_mut().poll(cx));
        this.waiting = None;
        Poll::Ready(result)
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    #[test]
    fn a_working_stack_gets_quinns_socket() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();
        let socket = bind((Ipv4Addr::LOCALHOST, 0).into()).unwrap();
        assert!(!format!("{socket:?}").contains("PlainSocket"));
    }
}

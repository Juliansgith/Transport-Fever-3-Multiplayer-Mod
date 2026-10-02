use std::{fmt, path::Path, sync::Arc, time::Duration};

use quinn::crypto::rustls::{NoInitialCipherSuite, QuicClientConfig, QuicServerConfig};
use rustls::{
    DigitallySignedStruct, SignatureScheme,
    client::{
        VerifierBuilderError, WebPkiServerVerifier,
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    },
    crypto::CryptoProvider,
    pki_types::{
        CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime,
        pem::{self, PemObject},
    },
};
use thiserror::Error;
use tpf3mp_proto::ALPN;

/// A connection with no traffic, not even keep-alives, is dropped after this.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Clients send keep-alives to hold their NAT binding open. The server sends
/// none, so a client that goes silent times out.
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(5);
/// How much unread data a client may send the server on its stream, and on
/// the whole connection. The server reads its control stream as frames
/// arrive, so these bound what a connection can make it hold without
/// slowing honest clients.
const SERVER_STREAM_WINDOW: u32 = 256 * 1024;
const SERVER_CONNECTION_WINDOW: u32 = 512 * 1024;
/// Turn streams the server may have open to a client at once. There is one
/// per game the client enters; the rest leave room for one that is ending.
const TURN_STREAMS: u32 = 4;
/// Streams a client may have open to the server at once: its control stream
/// and one bulk stream, for a snapshot.
const BIDI_STREAMS: u32 = 2;

/// How many bytes of advisory datagrams one connection holds in each
/// direction. A pointer datagram is a few dozen bytes and the room rate-limits
/// them (crates/tpf3mp-server/src/connection.rs), so this is a buffer for a
/// burst, not a queue that has to be drained in time.
const DATAGRAM_WINDOW: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum TlsError {
    #[error("cannot read PEM data: {0}")]
    Pem(#[from] pem::Error),
    #[error("no certificate found in the certificate file")]
    NoCertificate,
    #[error("TLS configuration rejected: {0}")]
    Rustls(#[from] rustls::Error),
    #[error("certificate generation failed: {0}")]
    Generate(#[from] rcgen::Error),
    #[error("the TLS configuration offers no cipher suite QUIC can use")]
    NoInitialCipherSuite(#[from] NoInitialCipherSuite),
    #[error("cannot build the certificate verifier: {0}")]
    Verifier(#[from] VerifierBuilderError),
}

/// The server's certificate chain and private key.
pub struct ServerIdentity {
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
}

impl Clone for ServerIdentity {
    fn clone(&self) -> Self {
        Self {
            chain: self.chain.clone(),
            key: self.key.clone_key(),
        }
    }
}

impl ServerIdentity {
    /// Loads a PEM certificate chain (leaf first) and its PEM private key,
    /// such as the files an ACME client writes.
    pub fn from_pem_files(certificate: &Path, key: &Path) -> Result<Self, TlsError> {
        let chain = CertificateDer::pem_file_iter(certificate)?.collect::<Result<Vec<_>, _>>()?;
        if chain.is_empty() {
            return Err(TlsError::NoCertificate);
        }
        let key = PrivateKeyDer::from_pem_file(key)?;
        Ok(Self { chain, key })
    }

    /// Generates a self-signed certificate for development servers. Clients
    /// must pin it with [`ServerTrust::Pinned`].
    pub fn self_signed(names: &[&str]) -> Result<Self, TlsError> {
        let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
        let generated = rcgen::generate_simple_self_signed(names)?;
        let key = PrivatePkcs8KeyDer::from(generated.signing_key.serialize_der());
        Ok(Self {
            chain: vec![generated.cert.der().clone()],
            key: key.into(),
        })
    }

    /// The leaf certificate, for pinning.
    pub fn leaf(&self) -> &CertificateDer<'static> {
        // Both constructors guarantee a non-empty chain.
        &self.chain[0]
    }
}

impl fmt::Debug for ServerIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print the private key.
        f.debug_struct("ServerIdentity")
            .field("certificates", &self.chain.len())
            .finish_non_exhaustive()
    }
}

/// How a client decides whether to trust the server's certificate.
#[derive(Debug, Clone)]
pub enum ServerTrust {
    /// Public certificate authorities (Mozilla's root set). For deployed
    /// servers with a real certificate, e.g. from Let's Encrypt.
    WebPki,
    /// Exactly this certificate. For development servers.
    Pinned(CertificateDer<'static>),
}

pub fn server_config(identity: ServerIdentity) -> Result<quinn::ServerConfig, TlsError> {
    let mut tls = rustls::ServerConfig::builder_with_provider(crypto_provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(identity.chain, identity.key)?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let quic = QuicServerConfig::try_from(tls)?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(quic));
    config.transport_config(Arc::new(server_transport()));
    Ok(config)
}

pub fn client_config(trust: ServerTrust) -> Result<quinn::ClientConfig, TlsError> {
    let mut roots = rustls::RootCertStore::empty();
    match trust {
        ServerTrust::WebPki => roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
        ServerTrust::Pinned(certificate) => roots.add(certificate)?,
    }
    let mut tls = rustls::ClientConfig::builder_with_provider(crypto_provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let quic = QuicClientConfig::try_from(tls)?;
    let mut config = quinn::ClientConfig::new(Arc::new(quic));
    config.transport_config(Arc::new(client_transport()));
    Ok(config)
}

/// The TLS a server's tunnel listener serves: its own identity, over
/// HTTP/1.1 for the WebSocket upgrade.
pub fn tunnel_server_tls(identity: ServerIdentity) -> Result<Arc<rustls::ServerConfig>, TlsError> {
    let mut tls = rustls::ServerConfig::builder_with_provider(crypto_provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(identity.chain, identity.key)?;
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(tls))
}

/// The TLS a tunnel's client speaks. See [`TunnelVerifier`] for what it
/// trusts.
pub(crate) fn tunnel_client_tls(
    trust: &ServerTrust,
) -> Result<Arc<rustls::ClientConfig>, TlsError> {
    let provider = crypto_provider();
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let webpki =
        WebPkiServerVerifier::builder_with_provider(Arc::new(roots), Arc::clone(&provider))
            .build()?;
    let pinned = match trust {
        ServerTrust::Pinned(certificate) => Some(certificate.clone()),
        ServerTrust::WebPki => None,
    };
    let verifier = Arc::new(TunnelVerifier {
        webpki,
        pinned,
        provider: Arc::clone(&provider),
    });
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(tls))
}

/// Trusts a tunnel's certificate if public authorities vouch for it for the
/// tunnel's host, or if it is exactly the server's pinned certificate. A
/// proxy in front of a server with a pinned identity shows a public
/// certificate of its own. Either way the QUIC connection inside checks the
/// server's identity again, end to end.
#[derive(Debug)]
struct TunnelVerifier {
    webpki: Arc<WebPkiServerVerifier>,
    pinned: Option<CertificateDer<'static>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for TunnelVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if self
            .pinned
            .as_ref()
            .is_some_and(|pinned| pinned.as_ref() == end_entity.as_ref())
        {
            return Ok(ServerCertVerified::assertion());
        }
        self.webpki
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// What a client may make the server hold. A client opens its control
/// stream and at most one bulk stream at a time; datagrams carry the advisory
/// traffic (a player's pointer, tpf3mp-proto/src/datagram.rs), so the
/// connection has to say it will take them: with the buffer off, every
/// `send_datagram` is refused and a pointer never reaches the other game.
fn server_transport() -> quinn::TransportConfig {
    let mut transport = base_transport();
    transport
        .max_concurrent_bidi_streams(BIDI_STREAMS.into())
        .max_concurrent_uni_streams(0u32.into())
        .stream_receive_window(SERVER_STREAM_WINDOW.into())
        .receive_window(SERVER_CONNECTION_WINDOW.into());
    datagrams(&mut transport);
    transport
}

/// The server opens turn streams and the advisory datagrams back.
fn client_transport() -> quinn::TransportConfig {
    let mut transport = base_transport();
    transport
        .keep_alive_interval(Some(KEEP_ALIVE_INTERVAL))
        .max_concurrent_bidi_streams(0u32.into())
        .max_concurrent_uni_streams(TURN_STREAMS.into());
    datagrams(&mut transport);
    transport
}

/// Both ends of a connection take advisory datagrams and hold a window of
/// them: a player's pointer is a datagram, and with the receive buffer off
/// every `send_datagram` is refused, so the pointer never reaches the other
/// game. The window is a buffer for a burst, not a queue that has to be drained
/// in time: the room rate-limits what it passes on
/// (crates/tpf3mp-server/src/connection.rs) and drops what a member is too far
/// behind for.
fn datagrams(transport: &mut quinn::TransportConfig) {
    transport
        .datagram_receive_buffer_size(Some(DATAGRAM_WINDOW))
        .datagram_send_buffer_size(DATAGRAM_WINDOW);
}

fn base_transport() -> quinn::TransportConfig {
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(
        quinn::IdleTimeout::try_from(IDLE_TIMEOUT).expect("30 s is a valid QUIC idle timeout"),
    ));
    transport
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A player's pointer only reaches the other game as a datagram, and
    /// quinn refuses every `send_datagram` on a connection whose receive
    /// buffer is off. Both ends of a room's connection have to say they will
    /// take them; this is the check that keeps the next change from quietly
    /// turning the pointer off again. quinn keeps the buffer private, so this
    /// reads it out of the configuration it prints.
    #[test]
    fn both_ends_of_a_connection_take_advisory_datagrams() {
        for (which, transport) in [
            ("server", server_transport()),
            ("client", client_transport()),
        ] {
            let printed = format!("{transport:?}");
            assert!(
                printed.contains(&format!(
                    "datagram_receive_buffer_size: Some({DATAGRAM_WINDOW})"
                )),
                "the {which} end would refuse every pointer: {printed}"
            );
        }
    }

    /// The advisory traffic a player produces is a pointer a few times a
    /// second, so the window has to hold a burst of them rather than a
    /// trickle; and it is a buffer, not a queue the game has to drain.
    #[test]
    fn the_datagram_window_holds_a_burst_of_pointers() {
        let frames = DATAGRAM_WINDOW / 128;
        assert!(
            frames > 64,
            "{frames} frames is too few for the room's rate limit of 120 a second"
        );
    }
}

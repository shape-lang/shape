//! QUIC transport implementation (requires `quic` feature).
//!
//! Provides multiplexed, 0-RTT capable transport with built-in TLS 1.3
//! via the `quinn` crate. Supports connection migration for mobile/roaming
//! scenarios.
//!
//! # Feature flag
//!
//! Enable with `quic` feature in `shape-wire`:
//! ```toml
//! shape-wire = { path = "../shape-wire", features = ["quic"] }
//! ```

use super::{Connection, Transport, TransportError};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

/// Configuration for creating a QUIC transport.
pub struct QuicTransportConfig {
    /// Timeout for establishing new connections.
    pub connect_timeout: Duration,
    /// Local endpoint bind address for the QUIC client.
    pub bind_addr: SocketAddr,
    /// SNI / server name to validate against the peer certificate.
    pub server_name: String,
    /// Quinn client config (TLS roots, ALPN, etc.).
    pub client_config: quinn::ClientConfig,
}

/// QUIC-based transport using `quinn`.
///
/// Provides multiplexed streams, 0-RTT connection establishment,
/// built-in TLS 1.3, and connection migration support.
pub struct QuicTransport {
    /// Timeout for establishing new connections.
    pub connect_timeout: Duration,
    /// Quinn endpoint for initiating connections.
    endpoint: quinn::Endpoint,
    /// Runtime handle for blocking on async operations.
    runtime: tokio::runtime::Handle,
    /// Server name used for TLS/SNI verification.
    server_name: String,
}

impl QuicTransport {
    /// Create a QUIC transport from explicit configuration.
    pub fn with_config(config: QuicTransportConfig) -> Result<Self, TransportError> {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|e| TransportError::ConnectionFailed(format!("no tokio runtime: {}", e)))?;

        let mut endpoint = quinn::Endpoint::client(config.bind_addr)
            .map_err(|e| TransportError::ConnectionFailed(format!("bind endpoint: {}", e)))?;
        endpoint.set_default_client_config(config.client_config);

        Ok(Self {
            connect_timeout: config.connect_timeout,
            endpoint,
            runtime,
            server_name: config.server_name,
        })
    }

    /// Create a QUIC transport from a prebuilt quinn client config.
    pub fn with_client_config(
        client_config: quinn::ClientConfig,
        server_name: impl Into<String>,
    ) -> Result<Self, TransportError> {
        let config = QuicTransportConfig {
            connect_timeout: Duration::from_secs(10),
            bind_addr: "0.0.0.0:0"
                .parse()
                .map_err(|e| TransportError::ConnectionFailed(format!("bind addr parse: {}", e)))?,
            server_name: server_name.into(),
            client_config,
        };
        Self::with_config(config)
    }

    /// Create a QUIC transport from root certificate DER blobs.
    ///
    /// This is the production constructor: provide explicit trust anchors
    /// and peer server name.
    pub fn with_trust_anchors(
        root_certs_der: Vec<Vec<u8>>,
        server_name: impl Into<String>,
    ) -> Result<Self, TransportError> {
        let mut roots = rustls::RootCertStore::empty();
        for der in root_certs_der {
            roots
                .add(rustls::pki_types::CertificateDer::from(der))
                .map_err(|e| TransportError::ConnectionFailed(format!("add root cert: {}", e)))?;
        }

        let mut client_crypto = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        client_crypto.alpn_protocols = vec![b"shape/1".to_vec()];

        let client_config = quinn::ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto).map_err(|e| {
                TransportError::ConnectionFailed(format!("QUIC client config: {}", e))
            })?,
        ));

        Self::with_client_config(client_config, server_name)
    }

    /// Create a new QUIC transport with a self-signed certificate (for development).
    ///
    /// For production, use [`QuicTransport::with_config`] with proper TLS certificates.
    pub fn new_self_signed() -> Result<Self, TransportError> {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .map_err(|e| TransportError::ConnectionFailed(format!("cert generation: {}", e)))?;
        let cert_der = cert.cert.der().to_vec();
        Self::with_trust_anchors(vec![cert_der], "localhost")
    }
}

impl Transport for QuicTransport {
    fn send(&self, destination: &str, payload: &[u8]) -> Result<Vec<u8>, TransportError> {
        let dest_addr = destination
            .parse()
            .map_err(|e| TransportError::ConnectionFailed(format!("{}", e)))?;
        let endpoint = self.endpoint.clone();
        let payload = payload.to_vec();
        let timeout = self.connect_timeout;
        let server_name = self.server_name.clone();

        self.runtime.block_on(async move {
            let connecting = endpoint
                .connect(dest_addr, &server_name)
                .map_err(|e| TransportError::ConnectionFailed(format!("{}", e)))?;
            let connection = tokio::time::timeout(timeout, connecting)
                .await
                .map_err(|_| TransportError::Timeout)?
                .map_err(|e| TransportError::ConnectionFailed(format!("{}", e)))?;

            let (mut send_stream, mut recv_stream) = connection
                .open_bi()
                .await
                .map_err(|e| TransportError::SendFailed(format!("open stream: {}", e)))?;

            // Write length-prefixed framed payload
            let framed = super::framing::encode_framed(&payload);
            let len = (framed.len() as u32).to_be_bytes();
            send_stream
                .write_all(&len)
                .await
                .map_err(|e| TransportError::SendFailed(format!("{}", e)))?;
            send_stream
                .write_all(&framed)
                .await
                .map_err(|e| TransportError::SendFailed(format!("{}", e)))?;
            send_stream
                .finish()
                .map_err(|e| TransportError::SendFailed(format!("finish: {}", e)))?;

            // Read length-prefixed framed response
            let mut len_buf = [0u8; 4];
            recv_stream
                .read_exact(&mut len_buf)
                .await
                .map_err(|e| TransportError::ReceiveFailed(format!("{}", e)))?;
            let resp_len = u32::from_be_bytes(len_buf) as usize;

            if resp_len > super::tcp::MAX_PAYLOAD_SIZE {
                return Err(TransportError::PayloadTooLarge {
                    size: resp_len,
                    max: super::tcp::MAX_PAYLOAD_SIZE,
                });
            }

            let mut buf = vec![0u8; resp_len];
            recv_stream
                .read_exact(&mut buf)
                .await
                .map_err(|e| TransportError::ReceiveFailed(format!("{}", e)))?;

            super::framing::decode_framed(&buf)
        })
    }

    fn connect(&self, destination: &str) -> Result<Box<dyn Connection>, TransportError> {
        let dest_addr = destination
            .parse()
            .map_err(|e| TransportError::ConnectionFailed(format!("{}", e)))?;
        let endpoint = self.endpoint.clone();
        let timeout = self.connect_timeout;
        let runtime = self.runtime.clone();
        let server_name = self.server_name.clone();

        let connection = self.runtime.block_on(async move {
            let connecting = endpoint
                .connect(dest_addr, &server_name)
                .map_err(|e| TransportError::ConnectionFailed(format!("{}", e)))?;
            let conn = tokio::time::timeout(timeout, connecting)
                .await
                .map_err(|_| TransportError::Timeout)?
                .map_err(|e| TransportError::ConnectionFailed(format!("{}", e)))?;
            Ok::<_, TransportError>(conn)
        })?;

        Ok(Box::new(QuicConnection {
            connection,
            runtime,
        }))
    }
}

/// A persistent QUIC connection.
///
/// Each `send`/`recv` pair opens a new bidirectional stream, taking
/// advantage of QUIC's built-in multiplexing.
pub struct QuicConnection {
    connection: quinn::Connection,
    runtime: tokio::runtime::Handle,
}

impl Connection for QuicConnection {
    fn send(&mut self, payload: &[u8]) -> Result<(), TransportError> {
        let framed = super::framing::encode_framed(payload);
        self.runtime.block_on(async {
            let mut send_stream = self
                .connection
                .open_uni()
                .await
                .map_err(|e| TransportError::SendFailed(format!("open stream: {}", e)))?;

            let len = (framed.len() as u32).to_be_bytes();
            send_stream
                .write_all(&len)
                .await
                .map_err(|e| TransportError::SendFailed(format!("{}", e)))?;
            send_stream
                .write_all(&framed)
                .await
                .map_err(|e| TransportError::SendFailed(format!("{}", e)))?;
            send_stream
                .finish()
                .map_err(|e| TransportError::SendFailed(format!("finish: {}", e)))?;

            Ok(())
        })
    }

    fn recv(&mut self, timeout: Option<Duration>) -> Result<Vec<u8>, TransportError> {
        self.runtime.block_on(async {
            let accept_fut = self.connection.accept_uni();
            let mut recv_stream = if let Some(t) = timeout {
                tokio::time::timeout(t, accept_fut)
                    .await
                    .map_err(|_| TransportError::Timeout)?
            } else {
                accept_fut.await
            }
            .map_err(|e| TransportError::ReceiveFailed(format!("accept stream: {}", e)))?;

            let mut len_buf = [0u8; 4];
            recv_stream
                .read_exact(&mut len_buf)
                .await
                .map_err(|e| TransportError::ReceiveFailed(format!("{}", e)))?;
            let len = u32::from_be_bytes(len_buf) as usize;

            if len > super::tcp::MAX_PAYLOAD_SIZE {
                return Err(TransportError::PayloadTooLarge {
                    size: len,
                    max: super::tcp::MAX_PAYLOAD_SIZE,
                });
            }

            let mut buf = vec![0u8; len];
            recv_stream
                .read_exact(&mut buf)
                .await
                .map_err(|e| TransportError::ReceiveFailed(format!("{}", e)))?;

            super::framing::decode_framed(&buf)
        })
    }

    fn close(&mut self) -> Result<(), TransportError> {
        self.connection.close(0u32.into(), b"closed");
        Ok(())
    }

    fn supports_sidecars(&self) -> bool {
        true
    }

    fn send_sidecar(&mut self, payload: &[u8]) -> Result<(), TransportError> {
        // QUIC sidecars use separate unidirectional streams for parallelism
        self.send(payload)
    }

    fn recv_any(&mut self, timeout: Option<Duration>) -> Result<Vec<u8>, TransportError> {
        // QUIC accepts from any incoming unidirectional stream
        self.recv(timeout)
    }
}

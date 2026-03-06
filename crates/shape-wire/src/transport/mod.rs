//! Transport abstraction for Shape inter-node communication.
//!
//! Provides a trait-based transport layer that decouples the wire format
//! from the underlying network protocol. Implementations include TCP
//! (always available) and QUIC (behind the `quic` feature flag).

use std::sync::Arc;
use std::time::Duration;

/// Errors from transport operations.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("connection failed: {0}")]
    ConnectionFailed(String),
    #[error("send failed: {0}")]
    SendFailed(String),
    #[error("receive failed: {0}")]
    ReceiveFailed(String),
    #[error("timeout")]
    Timeout,
    #[error("connection closed")]
    ConnectionClosed,
    #[error("payload too large: {size} bytes (max {max})")]
    PayloadTooLarge { size: usize, max: usize },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Core transport abstraction for Shape inter-node communication.
///
/// Implementations provide one-shot request/response (`send`) and
/// persistent connection (`connect`) semantics.
pub trait Transport: Send + Sync {
    /// Send a payload to `destination` and return the response.
    ///
    /// This is a one-shot operation: connect, send, receive, close.
    fn send(&self, destination: &str, payload: &[u8]) -> Result<Vec<u8>, TransportError>;

    /// Establish a persistent connection to `destination`.
    fn connect(&self, destination: &str) -> Result<Box<dyn Connection>, TransportError>;
}

/// A persistent, bidirectional connection to a remote node.
pub trait Connection: Send {
    /// Send a framed payload over the connection.
    fn send(&mut self, payload: &[u8]) -> Result<(), TransportError>;

    /// Receive a framed payload. If `timeout` is `None`, blocks indefinitely.
    fn recv(&mut self, timeout: Option<Duration>) -> Result<Vec<u8>, TransportError>;

    /// Close the connection gracefully.
    fn close(&mut self) -> Result<(), TransportError>;

    /// Whether this connection supports out-of-band sidecar delivery.
    ///
    /// When true, sidecars can be sent in parallel on separate streams
    /// (e.g. QUIC unidirectional streams). When false, sidecars are sent
    /// sequentially on the same connection via `send()`.
    fn supports_sidecars(&self) -> bool {
        false
    }

    /// Send a sidecar payload. Default: falls back to `send()`.
    fn send_sidecar(&mut self, payload: &[u8]) -> Result<(), TransportError> {
        self.send(payload)
    }

    /// Receive any incoming message (regular or sidecar). Default: falls back to `recv()`.
    fn recv_any(&mut self, timeout: Option<Duration>) -> Result<Vec<u8>, TransportError> {
        self.recv(timeout)
    }
}

impl<T: Transport + ?Sized> Transport for Arc<T> {
    fn send(&self, destination: &str, payload: &[u8]) -> Result<Vec<u8>, TransportError> {
        self.as_ref().send(destination, payload)
    }

    fn connect(&self, destination: &str) -> Result<Box<dyn Connection>, TransportError> {
        self.as_ref().connect(destination)
    }
}

pub mod factory;
pub mod framing;
pub mod memoized;
pub mod tcp;

#[cfg(feature = "quic")]
pub mod quic;

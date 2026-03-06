//! Transport factory and protocol configuration.
//!
//! Owns transport-construction policy for built-in transports so higher
//! layers (VM/runtime) stay protocol-agnostic and delegate to shape-wire.

use std::sync::Arc;

use super::Transport;
use super::tcp::TcpTransport;

/// Supported wire transport kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    Tcp,
    #[cfg(feature = "quic")]
    Quic,
}

#[cfg(feature = "quic")]
#[derive(Debug, Clone)]
pub struct QuicFactoryConfig {
    pub server_name: String,
    pub root_certs_der: Vec<Vec<u8>>,
    pub connect_timeout: Option<std::time::Duration>,
}

#[cfg(feature = "quic")]
fn quic_config_cell() -> &'static std::sync::RwLock<Option<QuicFactoryConfig>> {
    static CELL: std::sync::OnceLock<std::sync::RwLock<Option<QuicFactoryConfig>>> =
        std::sync::OnceLock::new();
    CELL.get_or_init(|| std::sync::RwLock::new(None))
}

/// Configure global QUIC settings used by `TransportKind::Quic`.
#[cfg(feature = "quic")]
pub fn set_quic_config(config: QuicFactoryConfig) {
    if let Ok(mut guard) = quic_config_cell().write() {
        *guard = Some(config);
    }
}

/// Clear global QUIC settings.
#[cfg(feature = "quic")]
pub fn clear_quic_config() {
    if let Ok(mut guard) = quic_config_cell().write() {
        *guard = None;
    }
}

/// Return the currently configured global QUIC settings, if any.
#[cfg(feature = "quic")]
pub fn get_quic_config() -> Option<QuicFactoryConfig> {
    quic_config_cell()
        .read()
        .ok()
        .and_then(|guard| guard.clone())
}

/// Build a transport instance from protocol kind.
pub fn create_transport(kind: TransportKind) -> Result<Arc<dyn Transport>, String> {
    match kind {
        TransportKind::Tcp => Ok(Arc::new(TcpTransport::default())),
        #[cfg(feature = "quic")]
        TransportKind::Quic => {
            let cfg = get_quic_config().ok_or_else(|| {
                "transport.quic(): QUIC transport is not configured. \
Call shape_vm::configure_quic_transport(server_name, root_certs_der, connect_timeout) \
before requesting a QUIC transport."
                    .to_string()
            })?;

            let mut quic =
                super::quic::QuicTransport::with_trust_anchors(cfg.root_certs_der, cfg.server_name)
                    .map_err(|e| format!("transport.quic(): QUIC init: {}", e))?;
            if let Some(timeout) = cfg.connect_timeout {
                quic.connect_timeout = timeout;
            }
            Ok(Arc::new(quic))
        }
    }
}

/// Wire-layer transport provider boundary.
///
/// `shape-wire` ships `ShapeWireProvider` as the default implementation,
/// but embedders can provide alternate providers.
pub trait WireTransportProvider: Send + Sync {
    fn create_transport(&self, kind: TransportKind) -> Result<Arc<dyn Transport>, String>;
}

/// Default provider backed by shape-wire transport implementations.
#[derive(Debug, Default, Clone, Copy)]
pub struct ShapeWireProvider;

impl WireTransportProvider for ShapeWireProvider {
    fn create_transport(&self, kind: TransportKind) -> Result<Arc<dyn Transport>, String> {
        create_transport(kind)
    }
}

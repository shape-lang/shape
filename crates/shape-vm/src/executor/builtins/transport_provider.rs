//! Transport provider boundary for VM transport builtins.
//!
//! The VM depends on the provider trait only; `shape-wire` is the default
//! implementation via `ShapeWireProvider`.

use std::sync::{Arc, OnceLock, RwLock};

use shape_wire::transport::factory::ShapeWireProvider;
pub use shape_wire::transport::factory::WireTransportProvider;

fn provider_cell() -> &'static RwLock<Arc<dyn WireTransportProvider>> {
    static CELL: OnceLock<RwLock<Arc<dyn WireTransportProvider>>> = OnceLock::new();
    CELL.get_or_init(|| RwLock::new(Arc::new(ShapeWireProvider)))
}

/// Set a custom wire transport provider for VM transport builtins.
pub fn set_transport_provider(provider: Arc<dyn WireTransportProvider>) {
    if let Ok(mut guard) = provider_cell().write() {
        *guard = provider;
    }
}

/// Restore the default `shape-wire` transport provider.
pub fn reset_transport_provider() {
    if let Ok(mut guard) = provider_cell().write() {
        *guard = Arc::new(ShapeWireProvider);
    }
}

/// Get the active transport provider.
pub fn transport_provider() -> Arc<dyn WireTransportProvider> {
    provider_cell()
        .read()
        .ok()
        .map(|guard| guard.clone())
        .unwrap_or_else(|| Arc::new(ShapeWireProvider))
}

/// Configure global QUIC settings used by `transport.quic()`.
#[cfg(feature = "quic")]
pub fn configure_quic_transport(
    server_name: String,
    root_certs_der: Vec<Vec<u8>>,
    connect_timeout: Option<std::time::Duration>,
) {
    shape_wire::transport::factory::set_quic_config(
        shape_wire::transport::factory::QuicFactoryConfig {
            server_name,
            root_certs_der,
            connect_timeout,
        },
    );
}

/// Clear global QUIC settings used by `transport.quic()`.
#[cfg(feature = "quic")]
pub fn clear_quic_transport_config() {
    shape_wire::transport::factory::clear_quic_config();
}

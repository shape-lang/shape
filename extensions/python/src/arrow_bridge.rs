//! DataTable <-> DataFrame Arrow IPC bridge.
//!
//! Shape's DataTable uses Arrow columnar format internally. Python's
//! pandas/polars/pyarrow ecosystem also speaks Arrow IPC. This module
//! provides zero-copy (or near-zero-copy) transfer between the two:
//!
//! 1. **Shape -> Python**: Serialize a DataTable as Arrow IPC bytes,
//!    pass through the ABI, reconstruct as `pyarrow.RecordBatch` on
//!    the Python side.
//!
//! 2. **Python -> Shape**: The Python function returns a RecordBatch
//!    serialized as Arrow IPC, which we deserialize back into a DataTable.
//!
//! This avoids the overhead of element-wise msgpack serialization for
//! large tabular data.

/// Convert Shape DataTable (Arrow IPC bytes) to a format suitable for
/// Python consumption.
///
/// Stub -- the actual implementation will use pyo3 + pyarrow to create
/// a `pyarrow.RecordBatch` from the IPC buffer.
pub fn datatable_to_python_ipc(_ipc_bytes: &[u8]) -> Result<Vec<u8>, String> {
    // In the real implementation, this is essentially a pass-through
    // since both sides speak Arrow IPC. The bytes can be handed directly
    // to `pyarrow.ipc.open_stream()`.
    Err("arrow_bridge: DataTable -> Python not yet implemented".into())
}

/// Convert Python DataFrame (Arrow IPC bytes) back to Shape DataTable format.
///
/// Stub -- the actual implementation will serialize the pyarrow RecordBatch
/// to IPC bytes which Shape can ingest directly.
pub fn python_ipc_to_datatable(_ipc_bytes: &[u8]) -> Result<Vec<u8>, String> {
    Err("arrow_bridge: Python -> DataTable not yet implemented".into())
}

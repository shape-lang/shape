//! Snapshotting and resumability support
//!
//! Provides binary, diff-friendly snapshots via a content-addressed store.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use shape_value::{EnumPayload, EnumValue, PrintResult, PrintSpan, Upvalue, ValueWord, ValueWordExt};

use crate::event_queue::WaitCondition;
use crate::hashing::{HashDigest, hash_bytes};
use shape_ast::ast::{DataDateTimeRef, DateTimeExpr, EnumDef, TimeReference, TypeAnnotation};
use shape_ast::data::Timeframe;

use crate::data::DataFrame;
use shape_value::datatable::DataTable;

/// Schema version for the snapshot binary format.
///
/// This version is embedded in every [`ExecutionSnapshot`] via the `version`
/// field. Readers should check this value to determine whether they can
/// decode a snapshot or need migration logic.
///
/// Version history:
/// - v5 (current): ValueWord-native serialization — `nanboxed_to_serializable`
///   and `serializable_to_nanboxed` operate on ValueWord directly without
///   intermediate ValueWord conversion. Format is wire-compatible with v4
///   (same `SerializableVMValue` enum), so v4 snapshots deserialize
///   correctly without migration.
pub const SNAPSHOT_VERSION: u32 = 5;

pub(crate) const DEFAULT_CHUNK_LEN: usize = 4096;
pub(crate) const BYTE_CHUNK_LEN: usize = 256 * 1024;

/// Content-addressed snapshot store
#[derive(Clone)]
pub struct SnapshotStore {
    root: PathBuf,
}

impl SnapshotStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(root.join("blobs"))
            .with_context(|| format!("failed to create snapshot blob dir at {}", root.display()))?;
        fs::create_dir_all(root.join("snapshots"))
            .with_context(|| format!("failed to create snapshot dir at {}", root.display()))?;
        Ok(Self { root })
    }

    fn blob_path(&self, hash: &HashDigest) -> PathBuf {
        self.root
            .join("blobs")
            .join(format!("{}.bin.zst", hash.hex()))
    }

    fn snapshot_path(&self, hash: &HashDigest) -> PathBuf {
        self.root
            .join("snapshots")
            .join(format!("{}.bin.zst", hash.hex()))
    }

    pub fn put_blob(&self, data: &[u8]) -> Result<HashDigest> {
        let hash = hash_bytes(data);
        let path = self.blob_path(&hash);
        if path.exists() {
            return Ok(hash);
        }
        let compressed = zstd::stream::encode_all(data, 0)?;
        let mut file = fs::File::create(&path)?;
        file.write_all(&compressed)?;
        Ok(hash)
    }

    pub fn get_blob(&self, hash: &HashDigest) -> Result<Vec<u8>> {
        let path = self.blob_path(hash);
        let mut file = fs::File::open(&path)
            .with_context(|| format!("snapshot blob not found: {}", path.display()))?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        let decompressed = zstd::stream::decode_all(&buf[..])?;
        Ok(decompressed)
    }

    pub fn put_struct<T: Serialize>(&self, value: &T) -> Result<HashDigest> {
        let bytes = bincode::serialize(value)?;
        self.put_blob(&bytes)
    }

    pub fn get_struct<T: for<'de> Deserialize<'de>>(&self, hash: &HashDigest) -> Result<T> {
        let bytes = self.get_blob(hash)?;
        Ok(bincode::deserialize(&bytes)?)
    }

    pub fn put_snapshot(&self, snapshot: &ExecutionSnapshot) -> Result<HashDigest> {
        let bytes = bincode::serialize(snapshot)?;
        let hash = hash_bytes(&bytes);
        let path = self.snapshot_path(&hash);
        if !path.exists() {
            let compressed = zstd::stream::encode_all(&bytes[..], 0)?;
            let mut file = fs::File::create(&path)?;
            file.write_all(&compressed)?;
        }
        Ok(hash)
    }

    pub fn get_snapshot(&self, hash: &HashDigest) -> Result<ExecutionSnapshot> {
        let path = self.snapshot_path(hash);
        let mut file = fs::File::open(&path)
            .with_context(|| format!("snapshot not found: {}", path.display()))?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        let decompressed = zstd::stream::decode_all(&buf[..])?;
        Ok(bincode::deserialize(&decompressed)?)
    }

    /// List all snapshots in the store, returning (hash, snapshot) pairs.
    ///
    /// **Note:** This method eagerly loads and deserializes every snapshot in the
    /// store directory into memory. For stores with many snapshots this may
    /// become a bottleneck. A future improvement could return a lazy iterator
    /// that streams snapshot metadata (hash + `created_at_ms`) without
    /// deserializing full payloads until requested — e.g. via a
    /// `SnapshotEntry { hash, created_at_ms }` header read, deferring full
    /// `ExecutionSnapshot` deserialization to an explicit `.load()` call.
    pub fn list_snapshots(&self) -> Result<Vec<(HashDigest, ExecutionSnapshot)>> {
        let snapshots_dir = self.root.join("snapshots");
        if !snapshots_dir.exists() {
            return Ok(Vec::new());
        }
        let mut results = Vec::new();
        for entry in fs::read_dir(&snapshots_dir)? {
            let entry = entry?;
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // Files are named "<hex>.bin.zst"
                if let Some(hex) = name.strip_suffix(".bin.zst") {
                    let hash = HashDigest::from_hex(hex);
                    match self.get_snapshot(&hash) {
                        Ok(snap) => results.push((hash, snap)),
                        Err(_) => continue, // skip corrupt entries
                    }
                }
            }
        }
        // Sort by creation time, newest first
        results.sort_by(|a, b| b.1.created_at_ms.cmp(&a.1.created_at_ms));
        Ok(results)
    }

    /// Delete a snapshot file by hash.
    pub fn delete_snapshot(&self, hash: &HashDigest) -> Result<()> {
        let path = self.snapshot_path(hash);
        fs::remove_file(&path)
            .with_context(|| format!("failed to delete snapshot: {}", path.display()))?;
        Ok(())
    }
}

/// A serializable snapshot of a Shape program's execution state.
///
/// The `version` field records which [`SNAPSHOT_VERSION`] was used to
/// produce this snapshot. Readers must check this value before
/// deserializing the referenced sub-snapshots (semantic, context, VM)
/// to ensure binary compatibility or apply migration logic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionSnapshot {
    /// Schema version — should equal [`SNAPSHOT_VERSION`] at write time.
    /// Used by readers to detect format changes and apply migrations.
    pub version: u32,
    pub created_at_ms: i64,
    pub semantic_hash: HashDigest,
    pub context_hash: HashDigest,
    pub vm_hash: Option<HashDigest>,
    pub bytecode_hash: Option<HashDigest>,
    /// Path of the script that was executing when the snapshot was taken
    #[serde(default)]
    pub script_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticSnapshot {
    pub exported_symbols: HashSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSnapshot {
    pub data_load_mode: crate::context::DataLoadMode,
    pub data_cache: Option<DataCacheSnapshot>,
    pub current_id: Option<String>,
    pub current_row_index: usize,
    pub variable_scopes: Vec<HashMap<String, VariableSnapshot>>,
    pub reference_datetime: Option<chrono::DateTime<chrono::Utc>>,
    pub current_timeframe: Option<Timeframe>,
    pub base_timeframe: Option<Timeframe>,
    pub date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
    pub range_start: usize,
    pub range_end: usize,
    pub range_active: bool,
    pub type_alias_registry: HashMap<String, TypeAliasRuntimeEntrySnapshot>,
    pub enum_registry: HashMap<String, EnumDef>,
    #[serde(default)]
    pub struct_type_registry: HashMap<String, shape_ast::ast::StructTypeDef>,
    pub suspension_state: Option<SuspensionStateSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableSnapshot {
    pub value: SerializableVMValue,
    pub kind: shape_ast::ast::VarKind,
    pub is_initialized: bool,
    pub is_function_scoped: bool,
    pub format_hint: Option<String>,
    pub format_overrides: Option<HashMap<String, SerializableVMValue>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeAliasRuntimeEntrySnapshot {
    pub base_type: String,
    pub overrides: Option<HashMap<String, SerializableVMValue>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspensionStateSnapshot {
    pub waiting_for: WaitCondition,
    pub resume_pc: usize,
    pub saved_locals: Vec<SerializableVMValue>,
    pub saved_stack: Vec<SerializableVMValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmSnapshot {
    pub ip: usize,
    pub stack: Vec<SerializableVMValue>,
    pub locals: Vec<SerializableVMValue>,
    pub module_bindings: Vec<SerializableVMValue>,
    pub call_stack: Vec<SerializableCallFrame>,
    pub loop_stack: Vec<SerializableLoopContext>,
    pub timeframe_stack: Vec<Option<Timeframe>>,
    pub exception_handlers: Vec<SerializableExceptionHandler>,
    /// Content hash of the function blob that the top-level IP belongs to.
    /// Used for relocating the IP after recompilation.
    #[serde(default)]
    pub ip_blob_hash: Option<[u8; 32]>,
    /// Instruction offset within the function blob for the top-level IP.
    /// Computed as `ip - function_entry_point` when saving; reconstructed
    /// to absolute IP on restore. Only meaningful when `ip_blob_hash` is `Some`.
    #[serde(default)]
    pub ip_local_offset: Option<usize>,
    /// Function ID that the top-level IP belongs to.
    /// Used as a fallback when `ip_blob_hash` is not available.
    #[serde(default)]
    pub ip_function_id: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableCallFrame {
    pub return_ip: usize,
    pub locals_base: usize,
    pub locals_count: usize,
    pub function_id: Option<u16>,
    pub upvalues: Option<Vec<SerializableVMValue>>,
    /// Content hash of the function blob (for content-addressed state capture).
    /// When present, `local_ip` stores the instruction offset relative to the
    /// function's entry point rather than an absolute IP.
    #[serde(default)]
    pub blob_hash: Option<[u8; 32]>,
    /// Instruction offset within the function blob.
    /// Computed as `ip - function_entry_point` when saving; reconstructed to
    /// absolute IP on restore. Only meaningful when `blob_hash` is `Some`.
    #[serde(default)]
    pub local_ip: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableLoopContext {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableExceptionHandler {
    pub catch_ip: usize,
    pub stack_size: usize,
    pub call_depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SerializableVMValue {
    Int(i64),
    Number(f64),
    Decimal(rust_decimal::Decimal),
    String(String),
    Bool(bool),
    None,
    Some(Box<SerializableVMValue>),
    Unit,
    Timeframe(Timeframe),
    Duration(shape_ast::ast::Duration),
    Time(chrono::DateTime<chrono::FixedOffset>),
    TimeSpan(i64), // millis
    TimeReference(TimeReference),
    DateTimeExpr(DateTimeExpr),
    DataDateTimeRef(DataDateTimeRef),
    Array(Vec<SerializableVMValue>),
    Function(u16),
    TypeAnnotation(TypeAnnotation),
    TypeAnnotatedValue {
        type_name: String,
        value: Box<SerializableVMValue>,
    },
    Enum(EnumValueSnapshot),
    Closure {
        function_id: u16,
        upvalues: Vec<SerializableVMValue>,
    },
    ModuleFunction(String),
    TypedObject {
        schema_id: u64,
        /// Serialized slots: each slot is 8 bytes (raw bits for simple, serialized heap values for heap slots)
        slot_data: Vec<SerializableVMValue>,
        heap_mask: u64,
    },
    Range {
        start: Option<Box<SerializableVMValue>>,
        end: Option<Box<SerializableVMValue>>,
        inclusive: bool,
    },
    Ok(Box<SerializableVMValue>),
    Err(Box<SerializableVMValue>),
    PrintResult(PrintableSnapshot),
    SimulationCall {
        name: String,
        params: HashMap<String, SerializableVMValue>,
    },
    FunctionRef {
        name: String,
        closure: Option<Box<SerializableVMValue>>,
    },
    DataReference {
        datetime: chrono::DateTime<chrono::FixedOffset>,
        id: String,
        timeframe: Timeframe,
    },
    Future(u64),
    DataTable(BlobRef),
    TypedTable {
        schema_id: u64,
        table: BlobRef,
    },
    RowView {
        schema_id: u64,
        table: BlobRef,
        row_idx: usize,
    },
    ColumnRef {
        schema_id: u64,
        table: BlobRef,
        col_id: u32,
    },
    IndexedTable {
        schema_id: u64,
        table: BlobRef,
        index_col: u32,
    },
    /// Binary-serialized typed array (raw bytes via BlobRef).
    TypedArray {
        element_kind: TypedArrayElementKind,
        blob: BlobRef,
        len: usize,
    },
    /// Binary-serialized matrix (raw f64 bytes, row-major).
    Matrix {
        blob: BlobRef,
        rows: u32,
        cols: u32,
    },
    /// Dedicated HashMap variant preserving type identity.
    HashMap {
        keys: Vec<SerializableVMValue>,
        values: Vec<SerializableVMValue>,
    },
    /// Placeholder for sidecar-split large blobs (Phase 3B).
    /// Metadata fields preserve TypedArray len and Matrix rows/cols
    /// so reassembly can reconstruct the exact original variant.
    SidecarRef {
        sidecar_id: u32,
        blob_kind: BlobKind,
        original_hash: HashDigest,
        /// For TypedArray: element count. For Matrix: row count. Otherwise 0.
        meta_a: u32,
        /// For Matrix: column count. Otherwise 0.
        meta_b: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumValueSnapshot {
    pub enum_name: String,
    pub variant: String,
    pub payload: EnumPayloadSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EnumPayloadSnapshot {
    Unit,
    Tuple(Vec<SerializableVMValue>),
    Struct(Vec<(String, SerializableVMValue)>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrintableSnapshot {
    pub rendered: String,
    pub spans: Vec<PrintSpanSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PrintSpanSnapshot {
    Literal {
        text: String,
        start: usize,
        end: usize,
        span_id: String,
    },
    Value {
        text: String,
        start: usize,
        end: usize,
        span_id: String,
        variable_name: Option<String>,
        raw_value: Box<SerializableVMValue>,
        type_name: String,
        current_format: String,
        format_params: HashMap<String, SerializableVMValue>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobRef {
    pub hash: HashDigest,
    pub kind: BlobKind,
}

/// Element type for typed array binary serialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypedArrayElementKind {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BlobKind {
    DataTable,
    /// Raw typed array bytes (element type encoded separately).
    TypedArray(TypedArrayElementKind),
    /// Raw f64 bytes in row-major order.
    Matrix,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkedBlob {
    pub chunk_hashes: Vec<HashDigest>,
    pub total_len: usize,
    pub chunk_len: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableDataTable {
    pub ipc_chunks: ChunkedBlob,
    pub type_name: Option<String>,
    pub schema_id: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableDataFrame {
    pub id: String,
    pub timeframe: Timeframe,
    pub timestamps: ChunkedBlob,
    pub columns: Vec<SerializableDataFrameColumn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableDataFrameColumn {
    pub name: String,
    pub values: ChunkedBlob,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheKeySnapshot {
    pub id: String,
    pub timeframe: Timeframe,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedDataSnapshot {
    pub key: CacheKeySnapshot,
    pub historical: SerializableDataFrame,
    pub current_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveBufferSnapshot {
    pub key: CacheKeySnapshot,
    pub rows: ChunkedBlob,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataCacheSnapshot {
    pub historical: Vec<CachedDataSnapshot>,
    pub live_buffer: Vec<LiveBufferSnapshot>,
}

pub(crate) fn store_chunked_vec<T: Serialize>(
    values: &[T],
    chunk_len: usize,
    store: &SnapshotStore,
) -> Result<ChunkedBlob> {
    let chunk_len = chunk_len.max(1);
    if values.is_empty() {
        return Ok(ChunkedBlob {
            chunk_hashes: Vec::new(),
            total_len: 0,
            chunk_len,
        });
    }
    let mut hashes = Vec::new();
    for chunk in values.chunks(chunk_len) {
        let bytes = bincode::serialize(chunk)?;
        let hash = store.put_blob(&bytes)?;
        hashes.push(hash);
    }
    Ok(ChunkedBlob {
        chunk_hashes: hashes,
        total_len: values.len(),
        chunk_len,
    })
}

pub(crate) fn load_chunked_vec<T: DeserializeOwned>(
    chunked: &ChunkedBlob,
    store: &SnapshotStore,
) -> Result<Vec<T>> {
    if chunked.total_len == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(chunked.total_len);
    for hash in &chunked.chunk_hashes {
        let bytes = store.get_blob(hash)?;
        let chunk: Vec<T> = bincode::deserialize(&bytes)?;
        out.extend(chunk);
    }
    out.truncate(chunked.total_len);
    Ok(out)
}

/// Store raw bytes in content-addressed chunks (256 KB each).
pub fn store_chunked_bytes(data: &[u8], store: &SnapshotStore) -> Result<ChunkedBlob> {
    if data.is_empty() {
        return Ok(ChunkedBlob {
            chunk_hashes: Vec::new(),
            total_len: 0,
            chunk_len: BYTE_CHUNK_LEN,
        });
    }
    let mut hashes = Vec::new();
    for chunk in data.chunks(BYTE_CHUNK_LEN) {
        let hash = store.put_blob(chunk)?;
        hashes.push(hash);
    }
    Ok(ChunkedBlob {
        chunk_hashes: hashes,
        total_len: data.len(),
        chunk_len: BYTE_CHUNK_LEN,
    })
}

/// Load raw bytes from content-addressed chunks.
pub fn load_chunked_bytes(chunked: &ChunkedBlob, store: &SnapshotStore) -> Result<Vec<u8>> {
    if chunked.total_len == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(chunked.total_len);
    for hash in &chunked.chunk_hashes {
        let bytes = store.get_blob(hash)?;
        out.extend_from_slice(&bytes);
    }
    out.truncate(chunked.total_len);
    Ok(out)
}

/// Reinterpret a byte slice as a slice of `T` (must be properly aligned and sized).
///
/// # Safety
/// The byte slice must have a length that is a multiple of `size_of::<T>()`.
fn bytes_as_slice<T: Copy>(bytes: &[u8]) -> &[T] {
    let elem_size = std::mem::size_of::<T>();
    assert!(
        bytes.len() % elem_size == 0,
        "byte slice length {} not a multiple of element size {}",
        bytes.len(),
        elem_size
    );
    let len = bytes.len() / elem_size;
    unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const T, len) }
}

/// Reinterpret a slice of `T` as raw bytes.
fn slice_as_bytes<T>(data: &[T]) -> &[u8] {
    let byte_len = data.len() * std::mem::size_of::<T>();
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, byte_len) }
}

// ===================== Conversion helpers =====================

/// Serialize a ValueWord value to SerializableVMValue without materializing ValueWord.
///
/// For inline types (f64, i48, bool, None, Unit, Function), this avoids any heap
/// allocation by reading directly from the ValueWord tag/payload. For heap types,
/// it uses `as_heap_ref()` to inspect the HeapValue directly.
pub fn nanboxed_to_serializable(
    nb: &ValueWord,
    store: &SnapshotStore,
) -> Result<SerializableVMValue> {
    use shape_value::tags::{is_tagged, get_tag, TAG_INT, TAG_BOOL, TAG_NONE, TAG_UNIT, TAG_FUNCTION, TAG_MODULE_FN, TAG_HEAP};

    let bits = nb.raw_bits();
    if !is_tagged(bits) {
        return Ok(SerializableVMValue::Number(nb.as_f64().unwrap()));
    }
    match get_tag(bits) {
        TAG_INT => Ok(SerializableVMValue::Int(nb.as_i64().unwrap())),
        TAG_BOOL => Ok(SerializableVMValue::Bool(nb.as_bool().unwrap())),
        TAG_NONE => Ok(SerializableVMValue::None),
        TAG_UNIT => Ok(SerializableVMValue::Unit),
        TAG_FUNCTION => Ok(SerializableVMValue::Function(nb.as_function_id().unwrap())),
        TAG_MODULE_FN => Ok(SerializableVMValue::ModuleFunction(format!(
            "native#{}",
            nb.as_module_function().unwrap()
        ))),
        TAG_HEAP => {
            // Handle unified arrays (bit-47 tagged) only.
            if shape_value::tags::is_unified_heap(nb.raw_bits()) {
                let kind = unsafe { shape_value::tags::unified_heap_kind(nb.raw_bits()) };
                if kind == shape_value::tags::HEAP_KIND_ARRAY as u16 {
                    let arr = unsafe {
                        shape_value::unified_array::UnifiedArray::from_heap_bits(nb.raw_bits())
                    };
                    let items: Vec<SerializableVMValue> = (0..arr.len())
                        .map(|i| {
                            let elem = unsafe { shape_value::ValueWord::clone_from_bits(*arr.get(i).unwrap()) };
                            nanboxed_to_serializable(&elem, store).unwrap_or(SerializableVMValue::None)
                        })
                        .collect();
                    return Ok(SerializableVMValue::Array(items));
                }
                return Ok(SerializableVMValue::None);
            }
            // cold-path: as_heap_ref retained — multi-variant serialization dispatch
            let hv = nb.as_heap_ref().unwrap(); // cold-path
            heap_value_to_serializable(hv, store)
        }
        _ => Ok(SerializableVMValue::None), // References and other tags should not appear in snapshots
    }
}

/// Serialize a HeapValue to SerializableVMValue.
fn heap_value_to_serializable(
    hv: &shape_value::heap_value::HeapValue,
    store: &SnapshotStore,
) -> Result<SerializableVMValue> {
    use shape_value::heap_value::HeapValue;

    Ok(match hv {
        HeapValue::String(s) => SerializableVMValue::String((**s).clone()),
        HeapValue::Decimal(d) => SerializableVMValue::Decimal(*d),
        HeapValue::BigInt(i) => SerializableVMValue::Int(*i),
        HeapValue::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for v in arr.iter() {
                out.push(nanboxed_to_serializable(v, store)?);
            }
            SerializableVMValue::Array(out)
        }
        HeapValue::Closure {
            function_id,
            upvalues,
        } => {
            let mut ups = Vec::new();
            for up in upvalues.iter() {
                let nb = up.get();
                ups.push(nanboxed_to_serializable(&nb, store)?);
            }
            SerializableVMValue::Closure {
                function_id: *function_id,
                upvalues: ups,
            }
        }
        HeapValue::TypedObject {
            schema_id,
            slots,
            heap_mask,
        } => {
            let mut slot_data = Vec::with_capacity(slots.len());
            for i in 0..slots.len() {
                if *heap_mask & (1u64 << i) != 0 {
                    let hv_inner = slots[i].as_heap_value();
                    slot_data.push(heap_value_to_serializable(hv_inner, store)?);
                } else {
                    // Non-heap slot: raw bits are inline ValueWord representation.
                    // Reconstruct the ValueWord and serialize it properly.
                    let nb = unsafe { ValueWord::clone_from_bits(slots[i].raw()) };
                    slot_data.push(nanboxed_to_serializable(&nb, store)?);
                }
            }
            SerializableVMValue::TypedObject {
                schema_id: *schema_id,
                slot_data,
                heap_mask: *heap_mask,
            }
        }
        HeapValue::HostClosure(_)
        | HeapValue::Rare(shape_value::heap_value::RareHeapData::ExprProxy(_))
        | HeapValue::Rare(shape_value::heap_value::RareHeapData::FilterExpr(_))
        | HeapValue::TaskGroup { .. }
        | HeapValue::TraitObject { .. }
        | HeapValue::ProjectedRef(_)
        | HeapValue::NativeView(_) => {
            return Err(anyhow::anyhow!(
                "Cannot snapshot transient value: {}",
                hv.type_name()
            ));
        }
        HeapValue::Enum(ev) => SerializableVMValue::Enum(enum_to_snapshot(ev, store)?),
        HeapValue::Some(inner) => {
            SerializableVMValue::Some(Box::new(nanboxed_to_serializable(inner, store)?))
        }
        HeapValue::Ok(inner) => {
            SerializableVMValue::Ok(Box::new(nanboxed_to_serializable(inner, store)?))
        }
        HeapValue::Err(inner) => {
            SerializableVMValue::Err(Box::new(nanboxed_to_serializable(inner, store)?))
        }
        HeapValue::Range {
            start,
            end,
            inclusive,
        } => SerializableVMValue::Range {
            start: match start {
                Some(v) => Some(Box::new(nanboxed_to_serializable(v, store)?)),
                None => None,
            },
            end: match end {
                Some(v) => Some(Box::new(nanboxed_to_serializable(v, store)?)),
                None => None,
            },
            inclusive: *inclusive,
        },
        HeapValue::Temporal(shape_value::heap_value::TemporalData::Timeframe(tf)) => SerializableVMValue::Timeframe(*tf),
        HeapValue::Temporal(shape_value::heap_value::TemporalData::Duration(d)) => SerializableVMValue::Duration(d.clone()),
        HeapValue::Temporal(shape_value::heap_value::TemporalData::DateTime(t)) => SerializableVMValue::Time(*t),
        HeapValue::Temporal(shape_value::heap_value::TemporalData::TimeSpan(span)) => SerializableVMValue::TimeSpan(span.num_milliseconds()),
        HeapValue::Temporal(shape_value::heap_value::TemporalData::TimeReference(tr)) => SerializableVMValue::TimeReference((**tr).clone()),
        HeapValue::Temporal(shape_value::heap_value::TemporalData::DateTimeExpr(expr)) => SerializableVMValue::DateTimeExpr((**expr).clone()),
        HeapValue::Temporal(shape_value::heap_value::TemporalData::DataDateTimeRef(dref)) => SerializableVMValue::DataDateTimeRef((**dref).clone()),
        HeapValue::Rare(shape_value::heap_value::RareHeapData::TypeAnnotation(ta)) => SerializableVMValue::TypeAnnotation((**ta).clone()),
        HeapValue::Rare(shape_value::heap_value::RareHeapData::TypeAnnotatedValue { type_name, value }) => {
            SerializableVMValue::TypeAnnotatedValue {
                type_name: type_name.clone(),
                value: Box::new(nanboxed_to_serializable(value, store)?),
            }
        }
        HeapValue::Rare(shape_value::heap_value::RareHeapData::PrintResult(pr)) => {
            SerializableVMValue::PrintResult(print_result_to_snapshot(pr, store)?)
        }
        HeapValue::Rare(shape_value::heap_value::RareHeapData::SimulationCall(data)) => {
            let mut out = HashMap::new();
            for (k, v) in data.params.iter() {
                // SimulationCallData.params stores ValueWord (structural boundary in shape-value)
                out.insert(k.clone(), nanboxed_to_serializable(&v.clone(), store)?);
            }
            SerializableVMValue::SimulationCall {
                name: data.name.clone(),
                params: out,
            }
        }
        HeapValue::FunctionRef { name, closure } => SerializableVMValue::FunctionRef {
            name: name.clone(),
            closure: match closure {
                Some(c) => Some(Box::new(nanboxed_to_serializable(c, store)?)),
                None => None,
            },
        },
        HeapValue::Rare(shape_value::heap_value::RareHeapData::DataReference(data)) => SerializableVMValue::DataReference {
            datetime: data.datetime,
            id: data.id.clone(),
            timeframe: data.timeframe,
        },
        HeapValue::Future(id) => SerializableVMValue::Future(*id),
        HeapValue::DataTable(dt) => {
            let ser = serialize_datatable(dt, store)?;
            let hash = store.put_struct(&ser)?;
            SerializableVMValue::DataTable(BlobRef {
                hash,
                kind: BlobKind::DataTable,
            })
        }
        HeapValue::TableView(shape_value::heap_value::TableViewData::TypedTable { schema_id, table }) => {
            let ser = serialize_datatable(table, store)?;
            let hash = store.put_struct(&ser)?;
            SerializableVMValue::TypedTable {
                schema_id: *schema_id,
                table: BlobRef {
                    hash,
                    kind: BlobKind::DataTable,
                },
            }
        }
        HeapValue::TableView(shape_value::heap_value::TableViewData::RowView {
            schema_id,
            table,
            row_idx,
        }) => {
            let ser = serialize_datatable(table, store)?;
            let hash = store.put_struct(&ser)?;
            SerializableVMValue::RowView {
                schema_id: *schema_id,
                table: BlobRef {
                    hash,
                    kind: BlobKind::DataTable,
                },
                row_idx: *row_idx,
            }
        }
        HeapValue::TableView(shape_value::heap_value::TableViewData::ColumnRef {
            schema_id,
            table,
            col_id,
        }) => {
            let ser = serialize_datatable(table, store)?;
            let hash = store.put_struct(&ser)?;
            SerializableVMValue::ColumnRef {
                schema_id: *schema_id,
                table: BlobRef {
                    hash,
                    kind: BlobKind::DataTable,
                },
                col_id: *col_id,
            }
        }
        HeapValue::TableView(shape_value::heap_value::TableViewData::IndexedTable {
            schema_id,
            table,
            index_col,
        }) => {
            let ser = serialize_datatable(table, store)?;
            let hash = store.put_struct(&ser)?;
            SerializableVMValue::IndexedTable {
                schema_id: *schema_id,
                table: BlobRef {
                    hash,
                    kind: BlobKind::DataTable,
                },
                index_col: *index_col,
            }
        }
        HeapValue::NativeScalar(v) => {
            if let Some(i) = v.as_i64() {
                SerializableVMValue::Int(i)
            } else {
                SerializableVMValue::Number(v.as_f64())
            }
        }
        HeapValue::HashMap(d) => {
            let mut out_keys = Vec::with_capacity(d.keys.len());
            let mut out_values = Vec::with_capacity(d.values.len());
            for k in d.keys.iter() {
                out_keys.push(nanboxed_to_serializable(k, store)?);
            }
            for v in d.values.iter() {
                out_values.push(nanboxed_to_serializable(v, store)?);
            }
            SerializableVMValue::HashMap {
                keys: out_keys,
                values: out_values,
            }
        }
        HeapValue::Set(d) => {
            let mut out = Vec::with_capacity(d.items.len());
            for item in d.items.iter() {
                out.push(nanboxed_to_serializable(item, store)?);
            }
            SerializableVMValue::Array(out)
        }
        HeapValue::Deque(d) => {
            let mut out = Vec::with_capacity(d.items.len());
            for item in d.items.iter() {
                out.push(nanboxed_to_serializable(item, store)?);
            }
            SerializableVMValue::Array(out)
        }
        HeapValue::PriorityQueue(d) => {
            let mut out = Vec::with_capacity(d.items.len());
            for item in d.items.iter() {
                out.push(nanboxed_to_serializable(item, store)?);
            }
            SerializableVMValue::Array(out)
        }
        HeapValue::Content(node) => SerializableVMValue::String(format!("{}", node)),
        HeapValue::Instant(t) => {
            SerializableVMValue::String(format!("<instant:{:?}>", t.elapsed()))
        }
        HeapValue::IoHandle(data) => {
            let status = if data.is_open() { "open" } else { "closed" };
            SerializableVMValue::String(format!("<io_handle:{}:{}>", data.path, status))
        }
        HeapValue::SharedCell(arc) => nanboxed_to_serializable(&arc.read().unwrap(), store)?,
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::I64(a)) => {
            let blob = store_chunked_bytes(slice_as_bytes(a.as_slice()), store)?;
            let hash = store.put_struct(&blob)?;
            SerializableVMValue::TypedArray {
                element_kind: TypedArrayElementKind::I64,
                blob: BlobRef {
                    hash,
                    kind: BlobKind::TypedArray(TypedArrayElementKind::I64),
                },
                len: a.len(),
            }
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::F64(a)) => {
            let blob = store_chunked_bytes(slice_as_bytes(a.as_slice()), store)?;
            let hash = store.put_struct(&blob)?;
            SerializableVMValue::TypedArray {
                element_kind: TypedArrayElementKind::F64,
                blob: BlobRef {
                    hash,
                    kind: BlobKind::TypedArray(TypedArrayElementKind::F64),
                },
                len: a.len(),
            }
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::Bool(a)) => {
            let blob = store_chunked_bytes(slice_as_bytes(a.as_slice()), store)?;
            let hash = store.put_struct(&blob)?;
            SerializableVMValue::TypedArray {
                element_kind: TypedArrayElementKind::Bool,
                blob: BlobRef {
                    hash,
                    kind: BlobKind::TypedArray(TypedArrayElementKind::Bool),
                },
                len: a.len(),
            }
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::I8(a)) => {
            let blob = store_chunked_bytes(slice_as_bytes(a.as_slice()), store)?;
            let hash = store.put_struct(&blob)?;
            SerializableVMValue::TypedArray {
                element_kind: TypedArrayElementKind::I8,
                blob: BlobRef {
                    hash,
                    kind: BlobKind::TypedArray(TypedArrayElementKind::I8),
                },
                len: a.len(),
            }
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::I16(a)) => {
            let blob = store_chunked_bytes(slice_as_bytes(a.as_slice()), store)?;
            let hash = store.put_struct(&blob)?;
            SerializableVMValue::TypedArray {
                element_kind: TypedArrayElementKind::I16,
                blob: BlobRef {
                    hash,
                    kind: BlobKind::TypedArray(TypedArrayElementKind::I16),
                },
                len: a.len(),
            }
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::I32(a)) => {
            let blob = store_chunked_bytes(slice_as_bytes(a.as_slice()), store)?;
            let hash = store.put_struct(&blob)?;
            SerializableVMValue::TypedArray {
                element_kind: TypedArrayElementKind::I32,
                blob: BlobRef {
                    hash,
                    kind: BlobKind::TypedArray(TypedArrayElementKind::I32),
                },
                len: a.len(),
            }
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::U8(a)) => {
            let blob = store_chunked_bytes(slice_as_bytes(a.as_slice()), store)?;
            let hash = store.put_struct(&blob)?;
            SerializableVMValue::TypedArray {
                element_kind: TypedArrayElementKind::U8,
                blob: BlobRef {
                    hash,
                    kind: BlobKind::TypedArray(TypedArrayElementKind::U8),
                },
                len: a.len(),
            }
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::U16(a)) => {
            let blob = store_chunked_bytes(slice_as_bytes(a.as_slice()), store)?;
            let hash = store.put_struct(&blob)?;
            SerializableVMValue::TypedArray {
                element_kind: TypedArrayElementKind::U16,
                blob: BlobRef {
                    hash,
                    kind: BlobKind::TypedArray(TypedArrayElementKind::U16),
                },
                len: a.len(),
            }
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::U32(a)) => {
            let blob = store_chunked_bytes(slice_as_bytes(a.as_slice()), store)?;
            let hash = store.put_struct(&blob)?;
            SerializableVMValue::TypedArray {
                element_kind: TypedArrayElementKind::U32,
                blob: BlobRef {
                    hash,
                    kind: BlobKind::TypedArray(TypedArrayElementKind::U32),
                },
                len: a.len(),
            }
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::U64(a)) => {
            let blob = store_chunked_bytes(slice_as_bytes(a.as_slice()), store)?;
            let hash = store.put_struct(&blob)?;
            SerializableVMValue::TypedArray {
                element_kind: TypedArrayElementKind::U64,
                blob: BlobRef {
                    hash,
                    kind: BlobKind::TypedArray(TypedArrayElementKind::U64),
                },
                len: a.len(),
            }
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::F32(a)) => {
            let blob = store_chunked_bytes(slice_as_bytes(a.as_slice()), store)?;
            let hash = store.put_struct(&blob)?;
            SerializableVMValue::TypedArray {
                element_kind: TypedArrayElementKind::F32,
                blob: BlobRef {
                    hash,
                    kind: BlobKind::TypedArray(TypedArrayElementKind::F32),
                },
                len: a.len(),
            }
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::Matrix(m)) => {
            let raw_bytes = slice_as_bytes(m.data.as_slice());
            let blob = store_chunked_bytes(raw_bytes, store)?;
            let hash = store.put_struct(&blob)?;
            SerializableVMValue::Matrix {
                blob: BlobRef {
                    hash,
                    kind: BlobKind::Matrix,
                },
                rows: m.rows,
                cols: m.cols,
            }
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::FloatSlice {
            parent,
            offset,
            len,
        }) => {
            // Materialize the slice to an owned float array for serialization
            let start = *offset as usize;
            let end = start + *len as usize;
            let owned: Vec<f64> = parent.data[start..end].to_vec();
            let blob = store_chunked_bytes(slice_as_bytes(&owned), store)?;
            let hash = store.put_struct(&blob)?;
            SerializableVMValue::TypedArray {
                element_kind: TypedArrayElementKind::F64,
                blob: BlobRef {
                    hash,
                    kind: BlobKind::TypedArray(TypedArrayElementKind::F64),
                },
                len: *len as usize,
            }
        }
        HeapValue::Char(c) => SerializableVMValue::String(c.to_string()),
        HeapValue::Iterator(_)
        | HeapValue::Generator(_)
        | HeapValue::Concurrency(shape_value::heap_value::ConcurrencyData::Mutex(_))
        | HeapValue::Concurrency(shape_value::heap_value::ConcurrencyData::Atomic(_))
        | HeapValue::Concurrency(shape_value::heap_value::ConcurrencyData::Lazy(_))
        | HeapValue::Concurrency(shape_value::heap_value::ConcurrencyData::Channel(_)) => {
            return Err(anyhow::anyhow!(
                "Cannot snapshot transient value: {}",
                hv.type_name()
            ));
        }
    })
}

/// Deserialize a SerializableVMValue directly to ValueWord, avoiding ValueWord intermediate.
///
/// For inline types (Int, Number, Bool, None, Unit, Function), this constructs the ValueWord
/// directly using inline constructors. For heap types, it uses typed ValueWord constructors
/// (from_string, from_array, from_decimal, etc.) to skip any intermediate conversion.
pub fn serializable_to_nanboxed(
    value: &SerializableVMValue,
    store: &SnapshotStore,
) -> Result<ValueWord> {
    Ok(match value {
        SerializableVMValue::Int(i) => ValueWord::from_i64(*i),
        SerializableVMValue::Number(n) => ValueWord::from_f64(*n),
        SerializableVMValue::Decimal(d) => ValueWord::from_decimal(*d),
        SerializableVMValue::String(s) => ValueWord::from_string(std::sync::Arc::new(s.clone())),
        SerializableVMValue::Bool(b) => ValueWord::from_bool(*b),
        SerializableVMValue::None => ValueWord::none(),
        SerializableVMValue::Unit => ValueWord::unit(),
        SerializableVMValue::Function(f) => ValueWord::from_function(*f),
        SerializableVMValue::ModuleFunction(_name) => ValueWord::from_module_function(0),
        SerializableVMValue::Some(v) => ValueWord::from_some(serializable_to_nanboxed(v, store)?),
        SerializableVMValue::Ok(v) => ValueWord::from_ok(serializable_to_nanboxed(v, store)?),
        SerializableVMValue::Err(v) => ValueWord::from_err(serializable_to_nanboxed(v, store)?),
        SerializableVMValue::Timeframe(tf) => ValueWord::from_timeframe(*tf),
        SerializableVMValue::Duration(d) => ValueWord::from_duration(d.clone()),
        SerializableVMValue::Time(t) => ValueWord::from_time(*t),
        SerializableVMValue::TimeSpan(ms) => {
            ValueWord::from_timespan(chrono::Duration::milliseconds(*ms))
        }
        SerializableVMValue::TimeReference(tr) => ValueWord::from_time_reference(tr.clone()),
        SerializableVMValue::DateTimeExpr(expr) => ValueWord::from_datetime_expr(expr.clone()),
        SerializableVMValue::DataDateTimeRef(dref) => {
            ValueWord::from_data_datetime_ref(dref.clone())
        }
        SerializableVMValue::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for v in arr.iter() {
                out.push(serializable_to_nanboxed(v, store)?);
            }
            ValueWord::from_array(std::sync::Arc::new(out))
        }
        SerializableVMValue::TypeAnnotation(ta) => ValueWord::from_type_annotation(ta.clone()),
        SerializableVMValue::TypeAnnotatedValue { type_name, value } => {
            ValueWord::from_type_annotated_value(
                type_name.clone(),
                serializable_to_nanboxed(value, store)?,
            )
        }
        SerializableVMValue::Range {
            start,
            end,
            inclusive,
        } => ValueWord::from_range(
            match start {
                Some(v) => Some(serializable_to_nanboxed(v, store)?),
                None => None,
            },
            match end {
                Some(v) => Some(serializable_to_nanboxed(v, store)?),
                None => None,
            },
            *inclusive,
        ),
        SerializableVMValue::Future(id) => ValueWord::from_future(*id),
        SerializableVMValue::FunctionRef { name, closure } => ValueWord::from_function_ref(
            name.clone(),
            match closure {
                Some(c) => Some(serializable_to_nanboxed(c, store)?),
                None => None,
            },
        ),
        SerializableVMValue::DataReference {
            datetime,
            id,
            timeframe,
        } => ValueWord::from_data_reference(*datetime, id.clone(), *timeframe),
        SerializableVMValue::DataTable(blob) => {
            let ser: SerializableDataTable = store.get_struct(&blob.hash)?;
            ValueWord::from_datatable(std::sync::Arc::new(deserialize_datatable(ser, store)?))
        }
        SerializableVMValue::TypedTable { schema_id, table } => {
            let ser: SerializableDataTable = store.get_struct(&table.hash)?;
            ValueWord::from_typed_table(
                *schema_id,
                std::sync::Arc::new(deserialize_datatable(ser, store)?),
            )
        }
        SerializableVMValue::RowView {
            schema_id,
            table,
            row_idx,
        } => {
            let ser: SerializableDataTable = store.get_struct(&table.hash)?;
            ValueWord::from_row_view(
                *schema_id,
                std::sync::Arc::new(deserialize_datatable(ser, store)?),
                *row_idx,
            )
        }
        SerializableVMValue::ColumnRef {
            schema_id,
            table,
            col_id,
        } => {
            let ser: SerializableDataTable = store.get_struct(&table.hash)?;
            ValueWord::from_column_ref(
                *schema_id,
                std::sync::Arc::new(deserialize_datatable(ser, store)?),
                *col_id,
            )
        }
        SerializableVMValue::IndexedTable {
            schema_id,
            table,
            index_col,
        } => {
            let ser: SerializableDataTable = store.get_struct(&table.hash)?;
            ValueWord::from_indexed_table(
                *schema_id,
                std::sync::Arc::new(deserialize_datatable(ser, store)?),
                *index_col,
            )
        }
        SerializableVMValue::TypedArray {
            element_kind,
            blob,
            len,
        } => {
            let chunked: ChunkedBlob = store.get_struct(&blob.hash)?;
            let raw = load_chunked_bytes(&chunked, store)?;
            match element_kind {
                TypedArrayElementKind::I64 => {
                    let data: Vec<i64> = bytes_as_slice::<i64>(&raw)[..*len].to_vec();
                    ValueWord::from_int_array(std::sync::Arc::new(
                        shape_value::TypedBuffer::from_vec(data),
                    ))
                }
                TypedArrayElementKind::F64 => {
                    let data: Vec<f64> = bytes_as_slice::<f64>(&raw)[..*len].to_vec();
                    let aligned = shape_value::AlignedVec::from_vec(data);
                    ValueWord::from_float_array(std::sync::Arc::new(
                        shape_value::AlignedTypedBuffer::from_aligned(aligned),
                    ))
                }
                TypedArrayElementKind::Bool => {
                    let data: Vec<u8> = raw[..*len].to_vec();
                    ValueWord::from_bool_array(std::sync::Arc::new(
                        shape_value::TypedBuffer::from_vec(data),
                    ))
                }
                TypedArrayElementKind::I8 => {
                    let data: Vec<i8> = bytes_as_slice::<i8>(&raw)[..*len].to_vec();
                    ValueWord::from_i8_array(std::sync::Arc::new(
                        shape_value::TypedBuffer::from_vec(data),
                    ))
                }
                TypedArrayElementKind::I16 => {
                    let data: Vec<i16> = bytes_as_slice::<i16>(&raw)[..*len].to_vec();
                    ValueWord::from_i16_array(std::sync::Arc::new(
                        shape_value::TypedBuffer::from_vec(data),
                    ))
                }
                TypedArrayElementKind::I32 => {
                    let data: Vec<i32> = bytes_as_slice::<i32>(&raw)[..*len].to_vec();
                    ValueWord::from_i32_array(std::sync::Arc::new(
                        shape_value::TypedBuffer::from_vec(data),
                    ))
                }
                TypedArrayElementKind::U8 => {
                    let data: Vec<u8> = raw[..*len].to_vec();
                    ValueWord::from_u8_array(std::sync::Arc::new(
                        shape_value::TypedBuffer::from_vec(data),
                    ))
                }
                TypedArrayElementKind::U16 => {
                    let data: Vec<u16> = bytes_as_slice::<u16>(&raw)[..*len].to_vec();
                    ValueWord::from_u16_array(std::sync::Arc::new(
                        shape_value::TypedBuffer::from_vec(data),
                    ))
                }
                TypedArrayElementKind::U32 => {
                    let data: Vec<u32> = bytes_as_slice::<u32>(&raw)[..*len].to_vec();
                    ValueWord::from_u32_array(std::sync::Arc::new(
                        shape_value::TypedBuffer::from_vec(data),
                    ))
                }
                TypedArrayElementKind::U64 => {
                    let data: Vec<u64> = bytes_as_slice::<u64>(&raw)[..*len].to_vec();
                    ValueWord::from_u64_array(std::sync::Arc::new(
                        shape_value::TypedBuffer::from_vec(data),
                    ))
                }
                TypedArrayElementKind::F32 => {
                    let data: Vec<f32> = bytes_as_slice::<f32>(&raw)[..*len].to_vec();
                    ValueWord::from_f32_array(std::sync::Arc::new(
                        shape_value::TypedBuffer::from_vec(data),
                    ))
                }
            }
        }
        SerializableVMValue::Matrix { blob, rows, cols } => {
            let chunked: ChunkedBlob = store.get_struct(&blob.hash)?;
            let raw = load_chunked_bytes(&chunked, store)?;
            let data: Vec<f64> = bytes_as_slice::<f64>(&raw).to_vec();
            let aligned = shape_value::AlignedVec::from_vec(data);
            let matrix = shape_value::heap_value::MatrixData::from_flat(aligned, *rows, *cols);
            ValueWord::from_matrix(std::sync::Arc::new(matrix))
        }
        SerializableVMValue::HashMap { keys, values } => {
            let mut k_out = Vec::with_capacity(keys.len());
            for k in keys.iter() {
                k_out.push(serializable_to_nanboxed(k, store)?);
            }
            let mut v_out = Vec::with_capacity(values.len());
            for v in values.iter() {
                v_out.push(serializable_to_nanboxed(v, store)?);
            }
            ValueWord::from_hashmap_pairs(k_out, v_out)
        }
        SerializableVMValue::SidecarRef { .. } => {
            return Err(anyhow::anyhow!(
                "SidecarRef must be reassembled before deserialization"
            ));
        }
        SerializableVMValue::Enum(ev) => {
            // EnumPayload stores ValueWord internally (structural boundary in shape-value)
            let enum_val = snapshot_to_enum(ev, store)?;
            enum_val
        }
        SerializableVMValue::Closure {
            function_id,
            upvalues,
        } => {
            let mut ups = Vec::new();
            for v in upvalues.iter() {
                ups.push(Upvalue::new(serializable_to_nanboxed(v, store)?));
            }
            ValueWord::from_heap_value(shape_value::heap_value::HeapValue::Closure {
                function_id: *function_id,
                upvalues: ups,
            })
        }
        SerializableVMValue::TypedObject {
            schema_id,
            slot_data,
            heap_mask,
        } => {
            let mut slots = Vec::with_capacity(slot_data.len());
            let mut new_heap_mask: u64 = 0;
            for (i, sv) in slot_data.iter().enumerate() {
                if *heap_mask & (1u64 << i) != 0 {
                    // Heap slot: deserialize to ValueWord, then convert to slot.
                    // Backward compat: old snapshots may have inline types (Number,
                    // Bool, None, Unit, Function) marked as heap. nb_to_slot handles
                    // this by storing them as inline ValueSlots with is_heap=false.
                    let nb = serializable_to_nanboxed(sv, store)?;
                    let (slot, is_heap) = crate::type_schema::nb_to_slot(&nb);
                    slots.push(slot);
                    if is_heap {
                        new_heap_mask |= 1u64 << i;
                    }
                } else {
                    // Simple slot: extract f64 raw bits
                    let n = match sv {
                        SerializableVMValue::Number(n) => *n,
                        _ => 0.0,
                    };
                    slots.push(shape_value::ValueSlot::from_number(n));
                }
            }
            ValueWord::from_heap_value(shape_value::heap_value::HeapValue::TypedObject {
                schema_id: *schema_id,
                slots: slots.into_boxed_slice(),
                heap_mask: new_heap_mask,
            })
        }
        SerializableVMValue::PrintResult(pr) => {
            // PrintResult contains ValueWord internally (structural boundary in shape-value)
            let print_result = snapshot_to_print_result(pr, store)?;
            ValueWord::from_print_result(print_result)
        }
        SerializableVMValue::SimulationCall { name, params } => {
            // SimulationCallData stores ValueWord (structural boundary in shape-value)
            let mut out = HashMap::new();
            for (k, v) in params.iter() {
                out.insert(k.clone(), serializable_to_nanboxed(v, store)?.clone());
            }
            ValueWord::from_simulation_call(name.clone(), out)
        }
    })
}

fn enum_to_snapshot(value: &EnumValue, store: &SnapshotStore) -> Result<EnumValueSnapshot> {
    Ok(EnumValueSnapshot {
        enum_name: value.enum_name.clone(),
        variant: value.variant.clone(),
        payload: match &value.payload {
            EnumPayload::Unit => EnumPayloadSnapshot::Unit,
            EnumPayload::Tuple(values) => {
                let mut out = Vec::new();
                for v in values.iter() {
                    out.push(nanboxed_to_serializable(v, store)?);
                }
                EnumPayloadSnapshot::Tuple(out)
            }
            EnumPayload::Struct(map) => {
                let mut out = Vec::new();
                for (k, v) in map.iter() {
                    out.push((k.clone(), nanboxed_to_serializable(v, store)?));
                }
                EnumPayloadSnapshot::Struct(out)
            }
        },
    })
}

fn snapshot_to_enum(snapshot: &EnumValueSnapshot, store: &SnapshotStore) -> Result<ValueWord> {
    Ok(ValueWord::from_enum(EnumValue {
        enum_name: snapshot.enum_name.clone(),
        variant: snapshot.variant.clone(),
        payload: match &snapshot.payload {
            EnumPayloadSnapshot::Unit => EnumPayload::Unit,
            EnumPayloadSnapshot::Tuple(values) => {
                let mut out = Vec::new();
                for v in values.iter() {
                    out.push(serializable_to_nanboxed(v, store)?);
                }
                EnumPayload::Tuple(out)
            }
            EnumPayloadSnapshot::Struct(map) => {
                let mut out = HashMap::new();
                for (k, v) in map.iter() {
                    out.insert(k.clone(), serializable_to_nanboxed(v, store)?);
                }
                EnumPayload::Struct(out)
            }
        },
    }))
}

fn print_result_to_snapshot(
    result: &PrintResult,
    store: &SnapshotStore,
) -> Result<PrintableSnapshot> {
    let mut spans = Vec::new();
    for span in result.spans.iter() {
        match span {
            PrintSpan::Literal {
                text,
                start,
                end,
                span_id,
            } => spans.push(PrintSpanSnapshot::Literal {
                text: text.clone(),
                start: *start,
                end: *end,
                span_id: span_id.clone(),
            }),
            PrintSpan::Value {
                text,
                start,
                end,
                span_id,
                variable_name,
                raw_value,
                type_name,
                current_format,
                format_params,
            } => {
                let mut params = HashMap::new();
                for (k, v) in format_params.iter() {
                    params.insert(k.clone(), nanboxed_to_serializable(&v.clone(), store)?);
                }
                spans.push(PrintSpanSnapshot::Value {
                    text: text.clone(),
                    start: *start,
                    end: *end,
                    span_id: span_id.clone(),
                    variable_name: variable_name.clone(),
                    raw_value: Box::new(nanboxed_to_serializable(raw_value.as_ref(), store)?),
                    type_name: type_name.clone(),
                    current_format: current_format.clone(),
                    format_params: params,
                });
            }
        }
    }
    Ok(PrintableSnapshot {
        rendered: result.rendered.clone(),
        spans,
    })
}

fn snapshot_to_print_result(
    snapshot: &PrintableSnapshot,
    store: &SnapshotStore,
) -> Result<PrintResult> {
    let mut spans = Vec::new();
    for span in snapshot.spans.iter() {
        match span {
            PrintSpanSnapshot::Literal {
                text,
                start,
                end,
                span_id,
            } => {
                spans.push(PrintSpan::Literal {
                    text: text.clone(),
                    start: *start,
                    end: *end,
                    span_id: span_id.clone(),
                });
            }
            PrintSpanSnapshot::Value {
                text,
                start,
                end,
                span_id,
                variable_name,
                raw_value,
                type_name,
                current_format,
                format_params,
            } => {
                let mut params = HashMap::new();
                for (k, v) in format_params.iter() {
                    params.insert(k.clone(), serializable_to_nanboxed(v, store)?.clone());
                }
                spans.push(PrintSpan::Value {
                    text: text.clone(),
                    start: *start,
                    end: *end,
                    span_id: span_id.clone(),
                    variable_name: variable_name.clone(),
                    raw_value: Box::new(serializable_to_nanboxed(raw_value, store)?.clone()),
                    type_name: type_name.clone(),
                    current_format: current_format.clone(),
                    format_params: params,
                });
            }
        }
    }
    Ok(PrintResult {
        rendered: snapshot.rendered.clone(),
        spans,
    })
}

pub(crate) fn serialize_dataframe(
    df: &DataFrame,
    store: &SnapshotStore,
) -> Result<SerializableDataFrame> {
    let mut columns: Vec<_> = df.columns.iter().collect();
    columns.sort_by(|a, b| a.0.cmp(b.0));
    let mut serialized_cols = Vec::with_capacity(columns.len());
    for (name, values) in columns.into_iter() {
        let blob = store_chunked_vec(values, DEFAULT_CHUNK_LEN, store)?;
        serialized_cols.push(SerializableDataFrameColumn {
            name: name.clone(),
            values: blob,
        });
    }
    Ok(SerializableDataFrame {
        id: df.id.clone(),
        timeframe: df.timeframe,
        timestamps: store_chunked_vec(&df.timestamps, DEFAULT_CHUNK_LEN, store)?,
        columns: serialized_cols,
    })
}

pub(crate) fn deserialize_dataframe(
    serialized: SerializableDataFrame,
    store: &SnapshotStore,
) -> Result<DataFrame> {
    let timestamps: Vec<i64> = load_chunked_vec(&serialized.timestamps, store)?;
    let mut columns = HashMap::new();
    for col in serialized.columns.into_iter() {
        let values: Vec<f64> = load_chunked_vec(&col.values, store)?;
        columns.insert(col.name, values);
    }
    Ok(DataFrame {
        id: serialized.id,
        timeframe: serialized.timeframe,
        timestamps,
        columns,
    })
}

fn serialize_datatable(dt: &DataTable, store: &SnapshotStore) -> Result<SerializableDataTable> {
    let mut buf = Vec::new();
    let schema = dt.inner().schema();
    let mut writer = arrow_ipc::writer::FileWriter::try_new(&mut buf, schema.as_ref())?;
    writer.write(dt.inner())?;
    writer.finish()?;
    let ipc_chunks = store_chunked_vec(&buf, BYTE_CHUNK_LEN, store)?;
    Ok(SerializableDataTable {
        ipc_chunks,
        type_name: dt.type_name().map(|s| s.to_string()),
        schema_id: dt.schema_id(),
    })
}

fn deserialize_datatable(
    serialized: SerializableDataTable,
    store: &SnapshotStore,
) -> Result<DataTable> {
    let bytes = load_chunked_vec(&serialized.ipc_chunks, store)?;
    let cursor = std::io::Cursor::new(bytes);
    let mut reader = arrow_ipc::reader::FileReader::try_new(cursor, None)?;
    let batch = reader
        .next()
        .transpose()?
        .context("no RecordBatch in DataTable snapshot")?;
    let mut dt = DataTable::new(batch);
    if let Some(name) = serialized.type_name {
        dt = DataTable::with_type_name(dt.into_inner(), name);
    }
    if let Some(schema_id) = serialized.schema_id {
        dt = dt.with_schema_id(schema_id);
    }
    Ok(dt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{Float64Array, Int64Array, RecordBatch};
    use arrow_schema::{DataType, Field, Schema};
    use std::sync::Arc;

    /// Helper: build a 2-column Arrow DataTable (id: i64, value: f64).
    fn make_test_table(ids: &[i64], values: &[f64]) -> DataTable {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("value", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from(ids.to_vec())),
                Arc::new(Float64Array::from(values.to_vec())),
            ],
        )
        .unwrap();
        DataTable::new(batch)
    }

    #[test]
    fn test_datatable_snapshot_roundtrip_preserves_original_data() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store");

        let original_dt = make_test_table(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10], &[150.0; 10]);
        assert_eq!(original_dt.row_count(), 10);
        let original_nb = ValueWord::from_datatable(Arc::new(original_dt));

        // Serialize to snapshot store
        let store = SnapshotStore::new(&store_path).unwrap();
        let serialized = nanboxed_to_serializable(&original_nb, &store).unwrap();

        // Restore from snapshot — must have ORIGINAL data
        let restored_nb = serializable_to_nanboxed(&serialized, &store).unwrap();
        let restored = restored_nb.clone();
        let dt = restored
            .as_datatable()
            .expect("Expected DataTable after restore");
        assert_eq!(
            dt.row_count(),
            10,
            "restored table should have original 10 rows"
        );
    }

    #[test]
    fn test_indexed_table_snapshot_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(dir.path().join("store")).unwrap();

        let dt = make_test_table(&[1, 2, 3], &[10.0, 20.0, 30.0]);
        let original_nb = ValueWord::from_indexed_table(1, Arc::new(dt), 0);

        let serialized = nanboxed_to_serializable(&original_nb, &store).unwrap();

        // Verify the serialized form is IndexedTable, NOT ColumnRef
        match &serialized {
            SerializableVMValue::IndexedTable {
                schema_id,
                index_col,
                ..
            } => {
                assert_eq!(schema_id, &1);
                assert_eq!(index_col, &0);
            }
            other => panic!(
                "Expected SerializableVMValue::IndexedTable, got {:?}",
                std::mem::discriminant(other)
            ),
        }

        let restored = serializable_to_nanboxed(&serialized, &store)
            .unwrap()
            .clone();
        let (schema_id, table, index_col) = restored
            .as_indexed_table()
            .expect("Expected ValueWord::IndexedTable");
        assert_eq!(schema_id, 1);
        assert_eq!(index_col, 0);
        assert_eq!(table.row_count(), 3);
    }

    #[test]
    fn test_typed_table_snapshot_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(dir.path().join("store")).unwrap();

        let dt = make_test_table(&[10, 20], &[1.5, 2.5]);
        let original_nb = ValueWord::from_typed_table(42, Arc::new(dt));

        let serialized = nanboxed_to_serializable(&original_nb, &store).unwrap();
        let restored = serializable_to_nanboxed(&serialized, &store)
            .unwrap()
            .clone();

        let (schema_id, table) = restored
            .as_typed_table()
            .expect("Expected ValueWord::TypedTable");
        assert_eq!(schema_id, 42);
        assert_eq!(table.row_count(), 2);
        let vals = table.get_f64_column("value").unwrap();
        assert!((vals.value(0) - 1.5).abs() < f64::EPSILON);
        assert!((vals.value(1) - 2.5).abs() < f64::EPSILON);
    }

    // ---- Phase 3A: Collection serialization tests ----

    #[test]
    fn test_float_array_typed_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(dir.path().join("store")).unwrap();

        let data: Vec<f64> = (0..1000).map(|i| i as f64 * 1.5).collect();
        let aligned = shape_value::AlignedVec::from_vec(data.clone());
        let buf = shape_value::AlignedTypedBuffer::from_aligned(aligned);
        let nb = ValueWord::from_float_array(Arc::new(buf));

        let serialized = nanboxed_to_serializable(&nb, &store).unwrap();
        match &serialized {
            SerializableVMValue::TypedArray {
                element_kind, len, ..
            } => {
                assert_eq!(*element_kind, TypedArrayElementKind::F64);
                assert_eq!(*len, 1000);
            }
            other => panic!(
                "Expected TypedArray, got {:?}",
                std::mem::discriminant(other)
            ),
        }

        let restored = serializable_to_nanboxed(&serialized, &store).unwrap();
        // cold-path: as_heap_ref retained — test assertion
        let hv = restored.as_heap_ref().unwrap(); // cold-path
        match hv {
            shape_value::heap_value::HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::F64(a)) => {
                assert_eq!(a.len(), 1000);
                for i in 0..1000 {
                    assert!((a.as_slice()[i] - data[i]).abs() < f64::EPSILON);
                }
            }
            _ => panic!("Expected TypedArray(F64) after restore"),
        }
    }

    #[test]
    fn test_int_array_typed_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(dir.path().join("store")).unwrap();

        let data: Vec<i64> = (0..500).map(|i| i * 3 - 100).collect();
        let buf = shape_value::TypedBuffer::from_vec(data.clone());
        let nb = ValueWord::from_int_array(Arc::new(buf));

        let serialized = nanboxed_to_serializable(&nb, &store).unwrap();
        match &serialized {
            SerializableVMValue::TypedArray {
                element_kind, len, ..
            } => {
                assert_eq!(*element_kind, TypedArrayElementKind::I64);
                assert_eq!(*len, 500);
            }
            other => panic!(
                "Expected TypedArray, got {:?}",
                std::mem::discriminant(other)
            ),
        }

        let restored = serializable_to_nanboxed(&serialized, &store).unwrap();
        // cold-path: as_heap_ref retained — test assertion
        let hv = restored.as_heap_ref().unwrap(); // cold-path
        match hv {
            shape_value::heap_value::HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::I64(a)) => {
                assert_eq!(a.len(), 500);
                for i in 0..500 {
                    assert_eq!(a.as_slice()[i], data[i]);
                }
            }
            _ => panic!("Expected TypedArray(I64) after restore"),
        }
    }

    #[test]
    fn test_matrix_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(dir.path().join("store")).unwrap();

        let data: Vec<f64> = (0..12).map(|i| i as f64).collect();
        let aligned = shape_value::AlignedVec::from_vec(data.clone());
        let matrix = shape_value::heap_value::MatrixData::from_flat(aligned, 3, 4);
        let nb = ValueWord::from_matrix(std::sync::Arc::new(matrix));

        let serialized = nanboxed_to_serializable(&nb, &store).unwrap();
        match &serialized {
            SerializableVMValue::Matrix { rows, cols, .. } => {
                assert_eq!(*rows, 3);
                assert_eq!(*cols, 4);
            }
            other => panic!("Expected Matrix, got {:?}", std::mem::discriminant(other)),
        }

        let restored = serializable_to_nanboxed(&serialized, &store).unwrap();
        // cold-path: as_heap_ref retained — test assertion
        let hv = restored.as_heap_ref().unwrap(); // cold-path
        match hv {
            shape_value::heap_value::HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::Matrix(m)) => {
                assert_eq!(m.rows, 3);
                assert_eq!(m.cols, 4);
                for i in 0..12 {
                    assert!((m.data.as_slice()[i] - data[i]).abs() < f64::EPSILON);
                }
            }
            _ => panic!("Expected TypedArray(Matrix) after restore"),
        }
    }

    #[test]
    fn test_hashmap_typed_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(dir.path().join("store")).unwrap();

        let keys = vec![
            ValueWord::from_string(Arc::new("a".to_string())),
            ValueWord::from_string(Arc::new("b".to_string())),
        ];
        let values = vec![ValueWord::from_i64(1), ValueWord::from_i64(2)];
        let nb = ValueWord::from_hashmap_pairs(keys, values);

        let serialized = nanboxed_to_serializable(&nb, &store).unwrap();
        match &serialized {
            SerializableVMValue::HashMap { keys, values } => {
                assert_eq!(keys.len(), 2);
                assert_eq!(values.len(), 2);
            }
            other => panic!("Expected HashMap, got {:?}", std::mem::discriminant(other)),
        }

        let restored = serializable_to_nanboxed(&serialized, &store).unwrap();
        // cold-path: as_heap_ref retained — test assertion
        let hv = restored.as_heap_ref().unwrap(); // cold-path
        match hv {
            shape_value::heap_value::HeapValue::HashMap(d) => {
                assert_eq!(d.keys.len(), 2);
                assert_eq!(d.values.len(), 2);
                // Verify index works (key lookup)
                let key_a = ValueWord::from_string(Arc::new("a".to_string()));
                let idx = d.find_key(&key_a);
                assert!(idx.is_some(), "should find key 'a' in rebuilt index");
            }
            _ => panic!("Expected HashMap after restore, got {:?}", hv.kind()),
        }
    }

    #[test]
    fn test_bool_array_typed_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SnapshotStore::new(dir.path().join("store")).unwrap();

        let data: Vec<u8> = vec![1, 0, 1, 1, 0];
        let buf = shape_value::TypedBuffer::from_vec(data.clone());
        let nb = ValueWord::from_bool_array(Arc::new(buf));

        let serialized = nanboxed_to_serializable(&nb, &store).unwrap();
        match &serialized {
            SerializableVMValue::TypedArray {
                element_kind, len, ..
            } => {
                assert_eq!(*element_kind, TypedArrayElementKind::Bool);
                assert_eq!(*len, 5);
            }
            other => panic!(
                "Expected TypedArray, got {:?}",
                std::mem::discriminant(other)
            ),
        }

        let restored = serializable_to_nanboxed(&serialized, &store).unwrap();
        // cold-path: as_heap_ref retained — test assertion
        let hv = restored.as_heap_ref().unwrap(); // cold-path
        match hv {
            shape_value::heap_value::HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::Bool(a)) => {
                assert_eq!(a.len(), 5);
                assert_eq!(a.as_slice(), &data);
            }
            _ => panic!("Expected TypedArray(Bool) after restore"),
        }
    }

    /// Comprehensive bincode round-trip test for ALL snapshot component types,
    /// exercising every type in the deserialization chain including Decimal.
    #[test]
    fn test_all_snapshot_components_bincode_roundtrip() {
        use rust_decimal::Decimal;
        use shape_ast::ast::VarKind;
        use shape_ast::data::Timeframe;

        // 1. SerializableVMValue with Decimal
        let decimal_val = SerializableVMValue::Decimal(Decimal::new(31415, 4)); // 3.1415
        let bytes = bincode::serialize(&decimal_val).expect("serialize Decimal ValueWord");
        let decoded: SerializableVMValue =
            bincode::deserialize(&bytes).expect("deserialize Decimal ValueWord");
        match decoded {
            SerializableVMValue::Decimal(d) => assert_eq!(d, Decimal::new(31415, 4)),
            _ => panic!("wrong variant"),
        }

        // 2. VmSnapshot with Decimal in stack/locals/module_bindings
        let vm_snap = VmSnapshot {
            ip: 42,
            stack: vec![
                SerializableVMValue::Int(1),
                SerializableVMValue::Decimal(Decimal::new(99999, 2)),
                SerializableVMValue::String("hello".into()),
            ],
            locals: vec![SerializableVMValue::Decimal(Decimal::new(0, 0))],
            module_bindings: vec![SerializableVMValue::Number(3.14)],
            call_stack: vec![],
            loop_stack: vec![],
            timeframe_stack: vec![],
            exception_handlers: vec![],
            ip_blob_hash: None,
            ip_local_offset: None,
            ip_function_id: None,
        };
        let bytes = bincode::serialize(&vm_snap).expect("serialize VmSnapshot");
        let decoded: VmSnapshot = bincode::deserialize(&bytes).expect("deserialize VmSnapshot");
        assert_eq!(decoded.ip, 42);
        assert_eq!(decoded.stack.len(), 3);

        // 3. ContextSnapshot with Decimal in variable scopes
        let ctx_snap = ContextSnapshot {
            data_load_mode: crate::context::DataLoadMode::Async,
            data_cache: None,
            current_id: Some("test".into()),
            current_row_index: 0,
            variable_scopes: vec![{
                let mut scope = HashMap::new();
                scope.insert(
                    "price".into(),
                    VariableSnapshot {
                        value: SerializableVMValue::Decimal(Decimal::new(15099, 2)),
                        kind: VarKind::Let,
                        is_initialized: true,
                        is_function_scoped: false,
                        format_hint: None,
                        format_overrides: None,
                    },
                );
                scope
            }],
            reference_datetime: None,
            current_timeframe: Some(Timeframe::m1()),
            base_timeframe: None,
            date_range: None,
            range_start: 0,
            range_end: 0,
            range_active: false,
            type_alias_registry: HashMap::new(),
            enum_registry: HashMap::new(),
            struct_type_registry: HashMap::new(),
            suspension_state: None,
        };
        let bytes = bincode::serialize(&ctx_snap).expect("serialize ContextSnapshot");
        let decoded: ContextSnapshot =
            bincode::deserialize(&bytes).expect("deserialize ContextSnapshot");
        assert_eq!(decoded.variable_scopes.len(), 1);

        // 4. SemanticSnapshot
        let sem_snap = SemanticSnapshot {
            exported_symbols: HashSet::new(),
        };
        let bytes = bincode::serialize(&sem_snap).expect("serialize SemanticSnapshot");
        let _decoded: SemanticSnapshot =
            bincode::deserialize(&bytes).expect("deserialize SemanticSnapshot");

        // 5. ExecutionSnapshot (top-level)
        let exec_snap = ExecutionSnapshot {
            version: SNAPSHOT_VERSION,
            created_at_ms: 1234567890,
            semantic_hash: HashDigest::from_hex("abc123"),
            context_hash: HashDigest::from_hex("def456"),
            vm_hash: Some(HashDigest::from_hex("789aaa")),
            bytecode_hash: Some(HashDigest::from_hex("bbb000")),
            script_path: Some("/tmp/test.shape".into()),
        };
        let bytes = bincode::serialize(&exec_snap).expect("serialize ExecutionSnapshot");
        let decoded: ExecutionSnapshot =
            bincode::deserialize(&bytes).expect("deserialize ExecutionSnapshot");
        assert_eq!(decoded.version, SNAPSHOT_VERSION);
    }
}

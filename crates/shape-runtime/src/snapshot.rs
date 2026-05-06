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
// EnumPayload, EnumValue, PrintResult, PrintSpan, Upvalue, ValueWord,
// ValueWordExt imports removed alongside the deleted value-(de)serialization
// functions (Phase 2b). The strict-typed slot-serialization API will land
// in a follow-up commit using (bits, NativeKind) pairs threaded from the
// FunctionBlob's per-slot kind metadata.

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
    /// A closure value carrying the function body id, its capture signature
    /// id, and the raw capture payloads.
    ///
    /// Track A.2A: `function_id` is widened to `u32` (from `u16`) to match
    /// the raw `TypedClosureHeader` field width, and a new `type_id: u32`
    /// carries the `ClosureTypeId` needed to re-resolve the
    /// `ClosureLayout` side-table on the receiver. Deserialization hard-
    /// errors when no layout is available — no legacy
    /// `HeapValue::Closure` fallback exists per the v2 closure closeout
    /// directive.
    Closure {
        function_id: u32,
        type_id: u32,
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
//
// The slot-(de)serialization functions (`nanboxed_to_serializable`,
// `serializable_to_nanboxed`, `serializable_to_nanboxed_with_layouts`,
// plus `enum_*`/`print_result_*` adapters) were deleted in Phase 2b
// alongside the `ValueWord` imports. Their replacement is a kind-
// threaded `slot_to_serializable(bits, kind, store)` /
// `serializable_to_slot(sv, expected_kind, store)` pair (mirrors the
// wire_conversion shape). The new API lands in a follow-up commit when
// stdlib mass migration (Phase 2c) and shape-vm cascade reveal the
// concrete consumer needs.

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


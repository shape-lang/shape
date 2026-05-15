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
    // ADR-005 §Forbidden / Q10 forward pointer: snapshot must NOT
    // re-introduce Box<HeapValue> slot wrapping. The current schema-+-slot
    // serialization layout aligns with ADR-005 §3 (typed slot bits + the
    // schema; deserialization reconstructs the typed pointer; no
    // intermediate HeapValue materialization). Audit of this path for full
    // ADR-005 conformance is queued for a future cluster (cluster #1
    // audit Q10). See docs/adr/005-typed-slot-construction.md.
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

    // ── W17-snapshot-roundtrip extension (ADR-006 §2.7.5.1, 2026-05-11) ──
    //
    // Wire-format arms for the post-W14/W15/W16/Wave-2.5 HeapKinds that
    // had no `SerializableVMValue` arm pre-W17-snapshot-roundtrip. Each
    // arm pairs 1:1 with a `HeapKind` ordinal and is post-proof per
    // §2.7.5.1: the discriminator (variant tag) carries the kind, the
    // payload carries the per-kind serialized data. Adding a new
    // `HeapKind` variant requires extending this enum in lockstep.
    //
    // Arm-by-arm coverage policy (per §2.7.5.1):
    //
    // - **Full payload round-trip** when the inner state is trivially
    //   serializable (HashSet keys are Arc<String>; PriorityQueue heap
    //   is Vec<i64>; Atomic value is i64; Char/BigInt are scalar).
    // - **Opaque-stub round-trip** when the inner state carries cross-
    //   value references that would re-introduce the deleted
    //   Arc<HeapValue> generic serializer shape (Iterator carries a
    //   closure-self share; Channel/Deque queues carry KindedSlot
    //   payloads of arbitrary kinds; FilterExpr is an AST tree of
    //   query nodes; Reference points into another heap object;
    //   SharedCell is binding-storage with parallel-kind track; Mutex
    //   and Lazy each carry a nested KindedSlot payload). The opaque
    //   stub carries the kind discriminator plus a per-arm descriptor
    //   string and surfaces a structured runtime error on resume —
    //   no silent corruption (the §2.7.4 invariant).
    //
    // Adding deep payload serialization for the opaque-stub arms lands
    // in follow-up sub-clusters per CLAUDE.md "Forbidden rationalizations"
    // (no `Arc<HeapValue>` generic serializer, no Bool-default
    // fallback — surface-and-stop is the right shape for the deep arms).

    /// `HeapKind::HashSet` — string-keyed insertion-ordered set (Wave 13).
    /// Round-trips the key array verbatim (per ADR-006 §2.7.15 string-
    /// only keyspace).
    HashSet { keys: Vec<String> },

    /// `HeapKind::Iterator` — lazy iterator carrier (W13 §2.7.16).
    /// Iterator state carries (a) a closure-self share for transform
    /// closures and (b) source-buffer references; serializing the
    /// graph requires walking the closure capture set, which re-
    /// introduces the Arc<HeapValue> generic serializer shape
    /// §2.7.5.1 forbids. Stored opaquely; resume surfaces an error
    /// citing the W17-snapshot-iterator follow-up.
    IteratorOpaque,

    /// `HeapKind::Result` — typed-Arc Result<T,E> carrier (Wave 14 §2.7.17).
    /// `Ok` / `Err` arms already exist for the pre-bulldozer
    /// scalar-payload form; this arm wraps a `KindedSlot`-payloaded
    /// `ResultData` per the post-§2.7.17 typed-Arc shape: the
    /// discriminator (is_ok) plus the inner serializable payload.
    ResultData {
        is_ok: bool,
        payload: Box<SerializableVMValue>,
    },

    /// `HeapKind::Option` — typed-Arc Option<T> carrier (Wave 14 §2.7.17).
    /// Mirror of `ResultData`: the discriminator (is_some) plus the
    /// inner payload (or `None` sentinel when is_some == false).
    OptionData {
        is_some: bool,
        payload: Option<Box<SerializableVMValue>>,
    },

    /// `HeapKind::Deque` — heterogeneous-element double-ended queue
    /// (Wave 15 §2.7.19). The element-payload storage is
    /// `Arc<HeapValue>` per the ADR-005 §1 single-discriminator
    /// shape; the items array is round-trippable as `Vec<SerializableVMValue>`
    /// once each element is projected through `slot_to_serializable`.
    /// Opaque at landing — per-element projection over an arbitrary
    /// `Arc<HeapValue>` walks the same generic-serializer shape
    /// §2.7.5.1 forbids; the per-element kinded path lands when the
    /// Deque method-tier wires its KindedSlot return-shape.
    DequeOpaque { len: usize },

    /// `HeapKind::Channel` — concurrency-primitive carrier
    /// (Wave 15 §2.7.20). The inner queue holds `KindedSlot` payloads
    /// of arbitrary kinds; same per-element-projection blocker as
    /// `DequeOpaque`. Closed-flag round-trips; queue contents land
    /// in the W17-snapshot-channel-queue follow-up.
    ChannelOpaque { closed: bool, len: usize },

    /// `HeapKind::PriorityQueue` — i64-priority min-heap
    /// (Wave 15 §2.7.18). i64-priority-only storage means full
    /// payload round-trip — the heap-ordered i64 vec encodes losslessly.
    PriorityQueueHeap { heap: Vec<i64> },

    /// `HeapKind::Reference` — `&expr` / `&mut expr` reference handle
    /// (Wave 8). The reference's target lives inside the same VM's
    /// heap; round-tripping requires tracking target identity across
    /// snapshot boundaries which is unspecified by ADR-006 §2.7. The
    /// W17-snapshot-references follow-up answers the identity question.
    ReferenceOpaque,

    /// `HeapKind::FilterExpr` — query-DSL AST tree (Wave-γ §2.7.9).
    /// Carries `Arc<FilterNode>` whose `And/Or/Not` branches recurse
    /// into other `FilterExpr` shares — round-tripping requires a
    /// dedicated AST serializer per the §2.7.9 pure-discriminator
    /// shape. Opaque at landing; the W17-snapshot-filter-expr
    /// follow-up lands the tree serializer.
    FilterExprOpaque,

    /// `HeapKind::SharedCell` — binding-storage interior-mutability
    /// carrier (Wave 8 §2.7.8). Carries the parallel-kind track and
    /// reaches into the cell payload's HeapKind dispatch; same per-
    /// element-projection blocker as Deque/Channel. Round-tripping
    /// also bumps into the binding-identity question (two `var x`
    /// bindings that share a cell observe each other's mutations,
    /// so cell identity must survive the snapshot).
    SharedCellOpaque,

    /// `HeapKind::Mutex` — single-typed-payload exclusion cell
    /// (Wave 2.5 §2.7.25). Inner `Option<KindedSlot>` payload of
    /// arbitrary kind; round-trip requires per-kind projection. The
    /// `MutexEmpty` discriminator distinguishes "no inner payload"
    /// (transient post-`take`) from "payload present, opaque on
    /// landing".
    MutexOpaque { has_value: bool },

    /// `HeapKind::Atomic` — atomic i64 cell (Wave 2.5 §2.7.25).
    /// Full payload round-trip — `AtomicI64::load(SeqCst)` reads the
    /// value, `AtomicI64::new(value)` restores. Memory ordering is
    /// `SeqCst` per the §2.7.25 ruling.
    AtomicI64 { value: i64 },

    /// `HeapKind::Lazy` — initialize-once cell (Wave 2.5 §2.7.25).
    /// Carries an initializer-closure `KindedSlot` (kind
    /// `Ptr(HeapKind::Closure)`) and a cached-value slot. Both halves
    /// land opaque pending the W17-snapshot-closure follow-up which
    /// also blocks `SerializableVMValue::Closure` deep round-trip
    /// (the existing Closure arm carries function_id + type_id +
    /// upvalues as `Vec<SerializableVMValue>` — restoration requires
    /// the ClosureLayout side-table on the receiver, which is itself
    /// part of the snapshot's program payload).
    LazyOpaque { is_initialized: bool },

    /// `HeapKind::Char` — single Unicode scalar value (Wave 12 §2.7.13).
    /// Full payload round-trip — `char` already serializes via serde.
    Char(char),

    /// `HeapKind::BigInt` — arbitrary-precision int (currently `Arc<i64>`
    /// per the Phase-2 deletion of the full bigint impl). Round-trips
    /// as i64; a future typed-payload BigInt rebuild updates the wire
    /// format.
    BigInt(i64),
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
//
// W17-snapshot-roundtrip (Phase 2d Wave 2.6, 2026-05-11): the
// kind-threaded `slot_to_serializable` / `serializable_to_slot` pair
// lands here per ADR-006 §2.7.5.1. The contract is:
//
// - `slot_to_serializable(bits, kind, store)`:
//   dispatch on `kind` to project the slot's raw u64 bits into the
//   matching `SerializableVMValue` arm. Scalar kinds project trivially
//   (`Int64` → `Int`, `Float64` → `Number`, `Bool` → `Bool`, etc.).
//   Heap kinds (`Ptr(HeapKind::*)`) dispatch via
//   `slot.as_heap_value()` + `HeapValue::*` match per the §2.7.6 / Q8
//   carrier-API bound — no `Arc<HeapValue>` generic serializer.
//
// - `serializable_to_slot(sv, expected_kind, store)`:
//   inverse projection — discriminator must match `expected_kind` (or
//   the function returns a structured kind-mismatch error). On success
//   returns `(bits, NativeKind)` ready to push to a stack/local slot
//   via `clone_with_kind` discipline.
//
// Both functions return `Result<_, String>` with structured error
// messages; the §2.7.5.1 forbidden shapes (Bool-default fallback,
// `Arc<HeapValue>` generic serializer, silent Option wrapping) are
// refused on sight. Unsupported heap kinds surface clean — the
// caller observes a runtime error rather than corrupted state.

use shape_value::{HeapKind, KindedSlot, NativeKind, ValueSlot};
use std::sync::Arc;

/// Project a `(bits, kind)` slot pair into its `SerializableVMValue` arm.
///
/// Per ADR-006 §2.7.5.1: scalar kinds project from raw u64 bits via the
/// canonical sign-extension / bitcast rules; heap kinds (`Ptr(HeapKind::*)`)
/// recover their typed `Arc<T>` via `ValueSlot::from_raw(bits).as_heap_value()`
/// + `HeapValue::*` match, then serialize per-arm. Heap kinds whose deep
/// payload requires the `Arc<HeapValue>` generic serializer shape
/// (§2.7.5.1 forbidden) project to their opaque-stub arm instead.
///
/// The `_store` parameter is reserved for chunked-blob arms that
/// off-line large payloads (DataTable IPC, TypedArray binary). Scalar
/// + simple-heap arms do not touch the store.
pub fn slot_to_serializable(
    bits: u64,
    kind: NativeKind,
    _store: &SnapshotStore,
) -> std::result::Result<SerializableVMValue, String> {
    use SerializableVMValue as SV;
    match kind {
        NativeKind::Int64 => Ok(SV::Int(bits as i64)),
        NativeKind::Int32 => Ok(SV::Int(bits as i32 as i64)),
        NativeKind::Int16 => Ok(SV::Int(bits as i16 as i64)),
        NativeKind::Int8 => Ok(SV::Int(bits as i8 as i64)),
        NativeKind::UInt64 => Ok(SV::Int(bits as i64)),
        NativeKind::UInt32 => Ok(SV::Int((bits as u32) as i64)),
        NativeKind::UInt16 => Ok(SV::Int((bits as u16) as i64)),
        NativeKind::UInt8 => Ok(SV::Int((bits as u8) as i64)),
        NativeKind::IntSize => Ok(SV::Int(bits as isize as i64)),
        NativeKind::UIntSize => Ok(SV::Int((bits as usize) as i64)),
        NativeKind::Float64 => Ok(SV::Number(f64::from_bits(bits))),
        // Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14):
        // ADR-006 §2.7.5 amendment adds F32 + Char as 4-byte scalar
        // variants. Wire-format projection: F32 widens to `SV::Number`
        // via `f64::from(f32)` (lossless); Char projects to `SV::Char`
        // by recovering the codepoint from the low 32 bits.
        NativeKind::Float32 => Ok(SV::Number(f64::from(f32::from_bits(bits as u32)))),
        NativeKind::Char => match char::from_u32(bits as u32) {
            Some(c) => Ok(SV::Char(c)),
            None => Err(format!(
                "slot_to_serializable: NativeKind::Char slot has invalid \
                 codepoint bits 0x{:x} — construction-side contract violated",
                bits,
            )),
        },
        NativeKind::Bool => Ok(SV::Bool(bits != 0)),
        NativeKind::NullableInt64
        | NativeKind::NullableInt32
        | NativeKind::NullableInt16
        | NativeKind::NullableInt8
        | NativeKind::NullableUInt64
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt8
        | NativeKind::NullableIntSize
        | NativeKind::NullableUIntSize
        | NativeKind::NullableFloat64 => {
            // Nullable scalar wire-format: surface-and-stop. The
            // canonical None sentinel (NaN for Float, MIN for signed,
            // MAX for unsigned) differs per kind; the §2.7.5.1
            // post-proof shape needs an explicit `Nullable<T>`
            // amendment with the sentinel rule. Tracked as
            // W17-snapshot-nullable follow-up.
            Err(format!(
                "slot_to_serializable: W17-snapshot-roundtrip surface — \
                 nullable-scalar kind {kind:?} has no SerializableVMValue \
                 arm at landing. The post-proof sentinel-rule amendment \
                 is the W17-snapshot-nullable follow-up. \
                 ADR-006 §2.7.5.1.",
            ))
        }
        NativeKind::String => {
            // String kind: bits is `Arc::into_raw(Arc<String>)`.
            // SAFETY: per the §2.7.6 String-arm construction contract,
            // a kind=String slot's bits encode a strong-count share on
            // an `Arc<String>` allocation. Reconstruct, clone, restore.
            if bits == 0 {
                return Err("slot_to_serializable: String slot with null bits".into());
            }
            unsafe {
                let arc = Arc::<String>::from_raw(bits as *const String);
                let cloned = (*arc).clone();
                let _ = Arc::into_raw(arc); // restore the original share
                Ok(SV::String(cloned))
            }
        }
        // Wave 2 Agent B W12-StringV2-DecimalV2-NativeKind-additions
        // (ADR-006 §2.7.5 amendment, 2026-05-14): the v2-raw `*const StringObj`
        // carrier projects to the same `SV::String` wire shape as
        // `NativeKind::String` — `StringObj::as_str` reads the UTF-8 payload
        // directly off the `repr(C)` carrier. The slot bits are NOT an
        // `Arc<T>` pointer, so we do not reconstruct + clone an `Arc`; we
        // borrow the inner `&str` (`StringObj::as_str` is `unsafe fn`
        // returning a `'static` borrow tied to the carrier's lifetime; the
        // slot owns one v2-retain share so the carrier is live for the
        // duration of this call).
        NativeKind::StringV2 => {
            if bits == 0 {
                return Err("slot_to_serializable: StringV2 slot with null bits".into());
            }
            // SAFETY: per the §2.7.5 amendment construction contract,
            // kind=StringV2 means bits = `ptr as u64` where `ptr` points to
            // a live `StringObj` whose refcount has been bumped to claim
            // this share. We borrow the inner UTF-8 bytes via
            // `StringObj::as_str`.
            let ptr = bits as *const shape_value::v2::string_obj::StringObj;
            let s: &str = unsafe { shape_value::v2::string_obj::StringObj::as_str(ptr) };
            Ok(SV::String(s.to_string()))
        }
        // Wave 2 Agent B: the v2-raw `*const DecimalObj` carrier projects
        // to the same `SV::Decimal` wire shape as
        // `NativeKind::Ptr(HeapKind::Decimal)` — `DecimalObj::value` returns
        // the inline `rust_decimal::Decimal` directly off the `repr(C)`
        // carrier. Same construction-side contract as `StringV2`.
        NativeKind::DecimalV2 => {
            if bits == 0 {
                return Err("slot_to_serializable: DecimalV2 slot with null bits".into());
            }
            // SAFETY: per the §2.7.5 amendment construction contract,
            // kind=DecimalV2 means bits = `ptr as u64` pointing to a live
            // `DecimalObj` with bumped refcount.
            let ptr = bits as *const shape_value::v2::decimal_obj::DecimalObj;
            let value = unsafe { shape_value::v2::decimal_obj::DecimalObj::value(ptr) };
            Ok(SV::Decimal(value))
        }
        NativeKind::Ptr(heap_kind) => slot_heap_to_serializable(bits, heap_kind),
    }
}

/// Project a heap-kinded slot to its `SerializableVMValue` arm.
///
/// Per the canonical typed-pointer recovery pattern (CLAUDE.md
/// "The 5-arm receiver-recovery soundness rule"): the slot bits for
/// `kind=Ptr(HeapKind::X)` are `Arc::into_raw(Arc<XData>) as u64`,
/// NOT `*const HeapValue`. Casting to `*const HeapValue` is wrong-
/// type recovery and segfaults. Each arm reconstructs the typed
/// `Arc<T>`, clones it (bumping the strong count for our read),
/// rebuilds the original share, and reads through the cloned Arc.
fn slot_heap_to_serializable(
    bits: u64,
    expected_kind: HeapKind,
) -> std::result::Result<SerializableVMValue, String> {
    use SerializableVMValue as SV;
    use shape_value::heap_value::{
        AtomicData, ChannelData, DequeData, HashSetData, LazyData, MutexData,
        OptionData, PriorityQueueData, ResultData,
    };
    if bits == 0 {
        return Err(format!(
            "slot_to_serializable: Ptr({expected_kind:?}) slot with null bits",
        ));
    }
    match expected_kind {
        HeapKind::String => {
            // SAFETY: bits = `Arc::into_raw(Arc<String>)` per the
            // ValueSlot::from_string_arc construction contract.
            unsafe {
                let arc = Arc::<String>::from_raw(bits as *const String);
                let cloned = (*arc).clone();
                let _ = Arc::into_raw(arc);
                Ok(SV::String(cloned))
            }
        }
        HeapKind::Decimal => unsafe {
            let arc = Arc::<rust_decimal::Decimal>::from_raw(bits as *const rust_decimal::Decimal);
            let v = *arc;
            let _ = Arc::into_raw(arc);
            Ok(SV::Decimal(v))
        },
        HeapKind::BigInt => unsafe {
            let arc = Arc::<i64>::from_raw(bits as *const i64);
            let v = *arc;
            let _ = Arc::into_raw(arc);
            Ok(SV::BigInt(v))
        },
        HeapKind::Char => {
            // Char is inline-scalar per the §2.7 raw-bits encoding
            // (the bits are the u32 codepoint).
            let cp = bits as u32;
            match char::from_u32(cp) {
                Some(c) => Ok(SV::Char(c)),
                None => Err(format!(
                    "slot_to_serializable: Char arm: invalid codepoint {cp:#x}"
                )),
            }
        }
        HeapKind::HashSet => unsafe {
            let arc = Arc::<HashSetData>::from_raw(bits as *const HashSetData);
            let keys: Vec<String> = arc.keys.iter().map(|k| (**k).clone()).collect();
            let _ = Arc::into_raw(arc);
            Ok(SV::HashSet { keys })
        },
        HeapKind::PriorityQueue => unsafe {
            let arc = Arc::<PriorityQueueData>::from_raw(bits as *const PriorityQueueData);
            let heap: Vec<i64> = (*arc.heap).clone();
            let _ = Arc::into_raw(arc);
            Ok(SV::PriorityQueueHeap { heap })
        },
        HeapKind::Atomic => unsafe {
            let arc = Arc::<AtomicData>::from_raw(bits as *const AtomicData);
            let v = arc.load();
            let _ = Arc::into_raw(arc);
            Ok(SV::AtomicI64 { value: v })
        },
        HeapKind::Lazy => unsafe {
            let arc = Arc::<LazyData>::from_raw(bits as *const LazyData);
            let is_init = arc.is_initialized();
            let _ = Arc::into_raw(arc);
            Ok(SV::LazyOpaque {
                is_initialized: is_init,
            })
        },
        HeapKind::Mutex => unsafe {
            let arc = Arc::<MutexData>::from_raw(bits as *const MutexData);
            // get() always returns Some — MutexData::new always
            // installs a payload. has_value is true unless the
            // inner is a Bool-zero (the canonical no-op None
            // sentinel per §2.7.25).
            let inner = arc.get();
            let has_value =
                !(matches!(inner.kind(), NativeKind::Bool) && inner.slot().raw() == 0);
            drop(inner);
            let _ = Arc::into_raw(arc);
            Ok(SV::MutexOpaque { has_value })
        },
        HeapKind::Channel => unsafe {
            let arc = Arc::<ChannelData>::from_raw(bits as *const ChannelData);
            let closed = arc.is_closed();
            let len = arc.len();
            let _ = Arc::into_raw(arc);
            Ok(SV::ChannelOpaque { closed, len })
        },
        HeapKind::Deque => unsafe {
            let arc = Arc::<DequeData>::from_raw(bits as *const DequeData);
            let len = arc.items.len();
            let _ = Arc::into_raw(arc);
            Ok(SV::DequeOpaque { len })
        },
        HeapKind::Result => unsafe {
            let arc = Arc::<ResultData>::from_raw(bits as *const ResultData);
            let is_ok = arc.is_ok;
            let payload_kind = arc.payload.kind();
            let payload_bits = arc.payload.slot().raw();
            let inner = serializable_inner_kinded(payload_bits, payload_kind)?;
            let _ = Arc::into_raw(arc);
            Ok(SV::ResultData {
                is_ok,
                payload: Box::new(inner),
            })
        },
        HeapKind::Option => unsafe {
            let arc = Arc::<OptionData>::from_raw(bits as *const OptionData);
            let is_some = arc.is_some;
            let payload = if is_some {
                let payload_kind = arc.payload.kind();
                let payload_bits = arc.payload.slot().raw();
                Some(Box::new(serializable_inner_kinded(payload_bits, payload_kind)?))
            } else {
                None
            };
            let _ = Arc::into_raw(arc);
            Ok(SV::OptionData { is_some, payload })
        },
        // Reference/FilterExpr/SharedCell/Iterator: discriminator-only
        // round-trip. The typed Arcs exist but their deep walks are
        // follow-up work; we don't even need to touch the Arc on the
        // way out for the opaque round-trip path.
        HeapKind::Reference => Ok(SV::ReferenceOpaque),
        HeapKind::FilterExpr => Ok(SV::FilterExprOpaque),
        HeapKind::SharedCell => Ok(SV::SharedCellOpaque),
        HeapKind::Iterator => Ok(SV::IteratorOpaque),
        // Future is inline u64 per §2.7.4.
        HeapKind::Future => Ok(SV::Future(bits)),

        // Pre-existing complex shapes: surface-and-stop per §2.7.5.1.
        // These have rich pre-bulldozer SerializableVMValue arms whose
        // construction requires more than typed-Arc recovery (Range
        // bounds carry KindedSlot endpoints; TypedObject carries the
        // schema_id + parallel-kind track over its fields; TypedArray
        // carries a typed-buffer payload and lands in a sidecar blob;
        // DataTable / TableView / HashMap / Temporal / TaskGroup /
        // IoHandle / NativeView / NativeScalar / Content / ClosureRaw
        // each have their own multi-step landing path).
        other => Err(format!(
            "slot_to_serializable: W17-snapshot-roundtrip surface — \
             HeapKind::{other:?} arm has no in-session SerializableVMValue \
             projection. Tracked as W17-snapshot-{other:?} follow-up per \
             docs/cluster-audits/phase-2d-playbook.md §3. \
             ADR-006 §2.7.5.1.",
        )),
    }
}

/// Inner KindedSlot serialization for Result/Option payloads.
/// Bool-zero short-circuit (unit-shape None marker) returns
/// `SerializableVMValue::Unit`; other kinds route through the
/// canonical `slot_to_serializable` path with a sentinel store.
fn serializable_inner_kinded(
    bits: u64,
    kind: NativeKind,
) -> std::result::Result<SerializableVMValue, String> {
    if matches!(kind, NativeKind::Bool) && bits == 0 {
        return Ok(SerializableVMValue::Unit);
    }
    match kind {
        NativeKind::Int64 => Ok(SerializableVMValue::Int(bits as i64)),
        NativeKind::Float64 => Ok(SerializableVMValue::Number(f64::from_bits(bits))),
        NativeKind::Bool => Ok(SerializableVMValue::Bool(bits != 0)),
        NativeKind::String => {
            if bits == 0 {
                return Ok(SerializableVMValue::None);
            }
            unsafe {
                let arc = Arc::<String>::from_raw(bits as *const String);
                let cloned = (*arc).clone();
                let _ = Arc::into_raw(arc);
                Ok(SerializableVMValue::String(cloned))
            }
        }
        _ => Err(format!(
            "serializable_inner_kinded: W17-snapshot-roundtrip surface — \
             inner Result/Option payload kind {kind:?} is not in the \
             initial scalar set; deep payload arms land in follow-up. \
             ADR-006 §2.7.5.1.",
        )),
    }
}

/// Inverse of [`slot_to_serializable`] — project a `SerializableVMValue`
/// back into a `(bits, NativeKind)` pair for placement into a stack
/// or local slot.
///
/// `expected_kind` is the post-proof kind the caller has already
/// committed to (from `FrameDescriptor.slots[i]` or the parallel
/// stack-kind track). A discriminator-vs-expected-kind mismatch
/// surfaces as a structured error rather than a Bool-default
/// fallback (§2.7.7 #9 / §2.7.5.1 forbidden).
pub fn serializable_to_slot(
    sv: &SerializableVMValue,
    expected_kind: NativeKind,
    _store: &SnapshotStore,
) -> std::result::Result<(u64, NativeKind), String> {
    use SerializableVMValue as SV;
    // Scalar projections — discriminator must match `expected_kind`'s
    // family (signed/unsigned/float/bool/string/heap).
    match (sv, expected_kind) {
        (SV::Int(i), NativeKind::Int64) => Ok((*i as u64, NativeKind::Int64)),
        (SV::Int(i), NativeKind::Int32) => Ok(((*i as i32) as u64, NativeKind::Int32)),
        (SV::Int(i), NativeKind::Int16) => Ok(((*i as i16 as i32) as u64, NativeKind::Int16)),
        (SV::Int(i), NativeKind::Int8) => Ok(((*i as i8 as i32) as u64, NativeKind::Int8)),
        (SV::Int(i), NativeKind::UInt64) => Ok((*i as u64, NativeKind::UInt64)),
        (SV::Int(i), NativeKind::UInt32) => Ok(((*i as u32) as u64, NativeKind::UInt32)),
        (SV::Int(i), NativeKind::UInt16) => Ok(((*i as u16) as u64, NativeKind::UInt16)),
        (SV::Int(i), NativeKind::UInt8) => Ok(((*i as u8) as u64, NativeKind::UInt8)),
        (SV::Int(i), NativeKind::IntSize) => Ok((*i as isize as u64, NativeKind::IntSize)),
        (SV::Int(i), NativeKind::UIntSize) => Ok((*i as u64, NativeKind::UIntSize)),
        (SV::Number(f), NativeKind::Float64) => Ok((f.to_bits(), NativeKind::Float64)),
        (SV::Bool(b), NativeKind::Bool) => Ok((if *b { 1 } else { 0 }, NativeKind::Bool)),
        (SV::String(s), NativeKind::String) => {
            let arc = Arc::new(s.clone());
            let raw = Arc::into_raw(arc) as u64;
            Ok((raw, NativeKind::String))
        }
        (SV::None | SV::Unit, NativeKind::Bool) => Ok((0, NativeKind::Bool)),

        // Heap kinds — discriminator must align with `expected_kind`'s
        // `HeapKind::*`. Reconstructing typed-Arc payloads is the
        // inverse of `slot_heap_to_serializable`; see per-arm coverage.
        (sv, NativeKind::Ptr(hk)) => serializable_to_heap_slot(sv, hk),

        // Wildcards — surface-and-stop. No Bool-default fallback.
        (other_sv, other_kind) => Err(format!(
            "serializable_to_slot: W17-snapshot-roundtrip surface — \
             SerializableVMValue arm {} cannot satisfy expected kind \
             {other_kind:?}. Discriminator-vs-kind mismatch is a structured \
             error, not a Bool-default fallback (§2.7.5.1 forbidden). \
             ADR-006 §2.7.5.1.",
            serializable_arm_name(other_sv),
        )),
    }
}

/// Inverse of [`slot_heap_to_serializable`] — reconstruct a heap-kinded
/// slot from its serialized arm. Returns `(bits, NativeKind)` ready
/// to push to a slot. The reconstructed slot owns one strong-count
/// share on the typed `Arc<T>` carrier.
fn serializable_to_heap_slot(
    sv: &SerializableVMValue,
    heap_kind: HeapKind,
) -> std::result::Result<(u64, NativeKind), String> {
    use SerializableVMValue as SV;
    use shape_value::heap_value::{
        AtomicData, HashSetData, OptionData, PriorityQueueData, ResultData,
    };
    match (sv, heap_kind) {
        (SV::String(s), HeapKind::String) => {
            // String can flow via either the dedicated NativeKind::String
            // or as Ptr(HeapKind::String) per ADR-005 §2 String exception.
            let arc = Arc::new(s.clone());
            let raw = Arc::into_raw(arc) as u64;
            Ok((raw, NativeKind::Ptr(HeapKind::String)))
        }
        (SV::Char(c), HeapKind::Char) => {
            // Char is an inline scalar in the HeapValue arm (Arc<char>
            // would be wasteful for a 4-byte value); slot ABI carries
            // it as raw bits.
            let bits = (*c as u32) as u64;
            Ok((bits, NativeKind::Ptr(HeapKind::Char)))
        }
        (SV::BigInt(n), HeapKind::BigInt) => {
            let arc = Arc::new(*n);
            let raw = Arc::into_raw(arc) as u64;
            Ok((raw, NativeKind::Ptr(HeapKind::BigInt)))
        }
        (SV::Decimal(d), HeapKind::Decimal) => {
            let arc = Arc::new(*d);
            let raw = Arc::into_raw(arc) as u64;
            Ok((raw, NativeKind::Ptr(HeapKind::Decimal)))
        }
        (SV::HashSet { keys }, HeapKind::HashSet) => {
            let arcs: Vec<Arc<String>> = keys.iter().map(|k| Arc::new(k.clone())).collect();
            let data = HashSetData::from_keys(arcs);
            let arc = Arc::new(data);
            let raw = Arc::into_raw(arc) as u64;
            Ok((raw, NativeKind::Ptr(HeapKind::HashSet)))
        }
        (SV::PriorityQueueHeap { heap }, HeapKind::PriorityQueue) => {
            let mut pq = PriorityQueueData::new();
            // Push values back through the public API to maintain
            // heap invariant. The serialized array is heap-order, not
            // sorted-order, so direct copy would be equivalent here —
            // but going through `push` is the safer canonical path.
            for &v in heap {
                pq.push(v);
            }
            let arc = Arc::new(pq);
            let raw = Arc::into_raw(arc) as u64;
            Ok((raw, NativeKind::Ptr(HeapKind::PriorityQueue)))
        }
        (SV::AtomicI64 { value }, HeapKind::Atomic) => {
            let arc = Arc::new(AtomicData::new(*value));
            let raw = Arc::into_raw(arc) as u64;
            Ok((raw, NativeKind::Ptr(HeapKind::Atomic)))
        }
        (SV::ResultData { is_ok, payload }, HeapKind::Result) => {
            // Inner payload kind: we need the expected kind for the
            // inner slot to dispatch. The §2.7.17 typed-Arc Result
            // carrier doesn't statically pin the inner kind; the
            // serialized discriminator is used to pick the inner kind
            // (Int→Int64, String→String, Bool→Bool, Number→Float64,
            // Unit→Bool-zero placeholder).
            let inner_slot = inner_kinded_from_serializable(payload)?;
            let data = if *is_ok {
                ResultData::ok(inner_slot)
            } else {
                ResultData::err(inner_slot)
            };
            let arc = Arc::new(data);
            let raw = Arc::into_raw(arc) as u64;
            Ok((raw, NativeKind::Ptr(HeapKind::Result)))
        }
        (SV::OptionData { is_some, payload }, HeapKind::Option) => {
            let data = if *is_some {
                match payload {
                    Some(p) => OptionData::some(inner_kinded_from_serializable(p)?),
                    None => return Err(
                        "serializable_to_slot: OptionData is_some=true but payload=None — \
                         malformed wire shape; expected Some(SerializableVMValue) for \
                         is_some=true. ADR-006 §2.7.5.1."
                            .to_string(),
                    ),
                }
            } else {
                OptionData::none()
            };
            let arc = Arc::new(data);
            let raw = Arc::into_raw(arc) as u64;
            Ok((raw, NativeKind::Ptr(HeapKind::Option)))
        }

        // Opaque arms — surface-and-stop on restore. These produced
        // discriminator-only wire shapes; the inner payload is lost.
        // Restoring as a structured runtime error rather than a
        // placeholder lets the caller observe the missing capability
        // cleanly (§2.7.4 invariant).
        (SV::IteratorOpaque, HeapKind::Iterator)
        | (SV::DequeOpaque { .. }, HeapKind::Deque)
        | (SV::ChannelOpaque { .. }, HeapKind::Channel)
        | (SV::ReferenceOpaque, HeapKind::Reference)
        | (SV::FilterExprOpaque, HeapKind::FilterExpr)
        | (SV::SharedCellOpaque, HeapKind::SharedCell)
        | (SV::MutexOpaque { .. }, HeapKind::Mutex)
        | (SV::LazyOpaque { .. }, HeapKind::Lazy) => Err(format!(
            "serializable_to_slot: W17-snapshot-roundtrip surface — \
             {heap_kind:?} arm restored from opaque wire shape; \
             deep payload reconstruction is the W17-snapshot-{:?} \
             follow-up. ADR-006 §2.7.5.1.",
            heap_kind,
        )),

        // Anything else: the discriminator doesn't pair with the
        // expected heap_kind. Surface-and-stop, no fabrication.
        (other_sv, hk) => Err(format!(
            "serializable_to_slot: W17-snapshot-roundtrip surface — \
             SerializableVMValue arm {} cannot satisfy expected heap kind \
             Ptr({hk:?}). Either the wire-format arm has no inverse \
             projection (deep follow-up) or the discriminator is \
             mismatched. ADR-006 §2.7.5.1.",
            serializable_arm_name(other_sv),
        )),
    }
}

/// Inverse for Result/Option inner payloads — discriminator-driven
/// scalar projection (Int→Int64, String→String, Bool→Bool,
/// Number→Float64, Unit→Bool-zero).
fn inner_kinded_from_serializable(
    sv: &SerializableVMValue,
) -> std::result::Result<KindedSlot, String> {
    use SerializableVMValue as SV;
    match sv {
        SV::Int(i) => Ok(KindedSlot::new(
            ValueSlot::from_raw(*i as u64),
            NativeKind::Int64,
        )),
        SV::Number(f) => Ok(KindedSlot::new(
            ValueSlot::from_raw(f.to_bits()),
            NativeKind::Float64,
        )),
        SV::Bool(b) => Ok(KindedSlot::new(
            ValueSlot::from_raw(if *b { 1 } else { 0 }),
            NativeKind::Bool,
        )),
        SV::String(s) => Ok(KindedSlot::from_string_arc(Arc::new(s.clone()))),
        SV::Unit | SV::None => Ok(KindedSlot::new(
            ValueSlot::from_raw(0),
            NativeKind::Bool,
        )),
        other => Err(format!(
            "inner_kinded_from_serializable: W17-snapshot-roundtrip surface — \
             SerializableVMValue arm {} has no in-session inner-payload \
             projection. Tracked as follow-up. ADR-006 §2.7.5.1.",
            serializable_arm_name(other),
        )),
    }
}

/// One-line discriminator name for diagnostic messages.
fn serializable_arm_name(sv: &SerializableVMValue) -> &'static str {
    use SerializableVMValue as SV;
    match sv {
        SV::Int(_) => "Int",
        SV::Number(_) => "Number",
        SV::Decimal(_) => "Decimal",
        SV::String(_) => "String",
        SV::Bool(_) => "Bool",
        SV::None => "None",
        SV::Some(_) => "Some",
        SV::Unit => "Unit",
        SV::Timeframe(_) => "Timeframe",
        SV::Duration(_) => "Duration",
        SV::Time(_) => "Time",
        SV::TimeSpan(_) => "TimeSpan",
        SV::TimeReference(_) => "TimeReference",
        SV::DateTimeExpr(_) => "DateTimeExpr",
        SV::DataDateTimeRef(_) => "DataDateTimeRef",
        SV::Array(_) => "Array",
        SV::Function(_) => "Function",
        SV::TypeAnnotation(_) => "TypeAnnotation",
        SV::TypeAnnotatedValue { .. } => "TypeAnnotatedValue",
        SV::Enum(_) => "Enum",
        SV::Closure { .. } => "Closure",
        SV::ModuleFunction(_) => "ModuleFunction",
        SV::TypedObject { .. } => "TypedObject",
        SV::Range { .. } => "Range",
        SV::Ok(_) => "Ok",
        SV::Err(_) => "Err",
        SV::PrintResult(_) => "PrintResult",
        SV::SimulationCall { .. } => "SimulationCall",
        SV::FunctionRef { .. } => "FunctionRef",
        SV::DataReference { .. } => "DataReference",
        SV::Future(_) => "Future",
        SV::DataTable(_) => "DataTable",
        SV::TypedTable { .. } => "TypedTable",
        SV::RowView { .. } => "RowView",
        SV::ColumnRef { .. } => "ColumnRef",
        SV::IndexedTable { .. } => "IndexedTable",
        SV::TypedArray { .. } => "TypedArray",
        SV::Matrix { .. } => "Matrix",
        SV::HashMap { .. } => "HashMap",
        SV::SidecarRef { .. } => "SidecarRef",
        SV::HashSet { .. } => "HashSet",
        SV::IteratorOpaque => "IteratorOpaque",
        SV::ResultData { .. } => "ResultData",
        SV::OptionData { .. } => "OptionData",
        SV::DequeOpaque { .. } => "DequeOpaque",
        SV::ChannelOpaque { .. } => "ChannelOpaque",
        SV::PriorityQueueHeap { .. } => "PriorityQueueHeap",
        SV::ReferenceOpaque => "ReferenceOpaque",
        SV::FilterExprOpaque => "FilterExprOpaque",
        SV::SharedCellOpaque => "SharedCellOpaque",
        SV::MutexOpaque { .. } => "MutexOpaque",
        SV::AtomicI64 { .. } => "AtomicI64",
        SV::LazyOpaque { .. } => "LazyOpaque",
        SV::Char(_) => "Char",
        SV::BigInt(_) => "BigInt",
    }
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


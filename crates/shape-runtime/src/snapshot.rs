//! Snapshotting and resumability support
//!
//! Provides binary, diff-friendly snapshots via a content-addressed store.

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// STAGE-R5 (ADR-006 §2.7.30.5 + §2.7.7 parallel-kind track): the
    /// per-slot `NativeKind` for `stack`, captured at serialize time. The
    /// `SV::SharedCell` BODY arm is carrier-ambiguous from its
    /// discriminator alone (it may sit in a Reference-kinded slot or a
    /// SharedCell-kinded slot); the restore driver reads the REAL kind
    /// here to pick the carrier (`link_promoted_reference` vs
    /// `link_shared_cell`). Empty (`serde(default)`) on pre-R5 snapshots —
    /// restore then falls back to the SV discriminator heuristic.
    #[serde(default)]
    pub stack_kinds: Vec<shape_value::NativeKind>,
    /// Per-slot `NativeKind` for `module_bindings` (same role as
    /// `stack_kinds`).
    #[serde(default)]
    pub module_binding_kinds: Vec<shape_value::NativeKind>,
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
    HashSet {
        keys: Vec<String>,
    },

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
    DequeOpaque {
        len: usize,
    },

    /// `HeapKind::Channel` — concurrency-primitive carrier
    /// (Wave 15 §2.7.20). The inner queue holds `KindedSlot` payloads
    /// of arbitrary kinds; same per-element-projection blocker as
    /// `DequeOpaque`. Closed-flag round-trips; queue contents land
    /// in the W17-snapshot-channel-queue follow-up.
    ChannelOpaque {
        closed: bool,
        len: usize,
    },

    /// `HeapKind::PriorityQueue` — i64-priority min-heap
    /// (Wave 15 §2.7.18). i64-priority-only storage means full
    /// payload round-trip — the heap-ordered i64 vec encodes losslessly.
    PriorityQueueHeap {
        heap: Vec<i64>,
    },

    /// `HeapKind::Reference` — `&expr` / `&mut expr` reference handle
    /// (Wave 8; STAGE-R5 serialize-through, ADR-006 §2.7.30.5).
    ///
    /// This is the **back-edge** form for a reference into a promoted
    /// cell: the cell BODY was already emitted (by whichever slot — a
    /// sibling `Reference` OR the `SharedCell` module binding — first
    /// reached the cell ptr; see `SerializeIdentityCtx`). `handle` keys
    /// the shared identity-map so this reference and the body's cell
    /// dedupe to ONE restored `Arc<SharedCell>`. `is_mut` is carried,
    /// reserved-not-read in v0.3.3 (§2.7.30.5).
    ///
    /// ONLY `RefTarget::PromotedCell` reaches this arm (KL-4 guard,
    /// §2.7.30.7): a non-promoted `Local` / `ModuleBinding` /
    /// `TypedField` reference has no owning cell, so reading its bits as
    /// `*const SharedCell` would be a wild-free — those arms keep the
    /// opaque clean-refuse.
    Reference {
        handle: u64,
        is_mut: bool,
    },

    /// `HeapKind::SharedCell` BODY (STAGE-R5, ADR-006 §2.7.30.5).
    ///
    /// The once-emitted payload of a promoted cell. `handle` is the
    /// cell's identity token (assigned by the FIRST arm — `Reference`
    /// or `SharedCell` — to reach the cell ptr); `inner` is the deep
    /// walk of `cell.value` + `cell.kind()` via `slot_to_serializable`.
    /// On restore Pass 1 materializes exactly one `Arc<SharedCell>` per
    /// handle into the identity-map; Pass 2 links every back-edge
    /// (`SharedCellRef` / `Reference`) carrying the same handle.
    SharedCell {
        handle: u64,
        inner: Box<SerializableVMValue>,
    },

    /// `HeapKind::SharedCell` back-edge (STAGE-R5, ADR-006 §2.7.30.5).
    ///
    /// A later `SharedCell`-kinded slot reaching an already-emitted cell
    /// ptr emits this instead of re-emitting the body. `handle` resolves
    /// to the once-materialized `Arc<SharedCell>` in the restore
    /// identity-map (Pass 2, `Arc::increment_strong_count`). NOT a new
    /// `HeapKind` ordinal — a WIRE discriminator resolving to
    /// `Ptr(HeapKind::SharedCell)` (§2.7.5.1 4-table lockstep unchanged).
    SharedCellRef {
        handle: u64,
    },

    /// `HeapKind::FilterExpr` — query-DSL AST tree (Wave-γ §2.7.9).
    /// Carries `Arc<FilterNode>` whose `And/Or/Not` branches recurse
    /// into other `FilterExpr` shares — round-tripping requires a
    /// dedicated AST serializer per the §2.7.9 pure-discriminator
    /// shape. Opaque at landing; the W17-snapshot-filter-expr
    /// follow-up lands the tree serializer.
    FilterExprOpaque,

    // NOTE (STAGE-R5): the old discriminator-only `SharedCellOpaque`
    // arm is REPLACED by the `SharedCell { handle, inner }` BODY arm +
    // `SharedCellRef { handle }` back-edge arm declared above. Cell
    // identity now survives the snapshot via the shared identity-map
    // (ADR-006 §2.7.30.5).
    /// `HeapKind::Mutex` — single-typed-payload exclusion cell
    /// (Wave 2.5 §2.7.25). Inner `Option<KindedSlot>` payload of
    /// arbitrary kind; round-trip requires per-kind projection. The
    /// `MutexEmpty` discriminator distinguishes "no inner payload"
    /// (transient post-`take`) from "payload present, opaque on
    /// landing".
    MutexOpaque {
        has_value: bool,
    },

    /// `HeapKind::Atomic` — atomic i64 cell (Wave 2.5 §2.7.25).
    /// Full payload round-trip — `AtomicI64::load(SeqCst)` reads the
    /// value, `AtomicI64::new(value)` restores. Memory ordering is
    /// `SeqCst` per the §2.7.25 ruling.
    AtomicI64 {
        value: i64,
    },

    /// `HeapKind::Lazy` — initialize-once cell (Wave 2.5 §2.7.25).
    /// Carries an initializer-closure `KindedSlot` (kind
    /// `Ptr(HeapKind::Closure)`) and a cached-value slot. Both halves
    /// land opaque pending the W17-snapshot-closure follow-up which
    /// also blocks `SerializableVMValue::Closure` deep round-trip
    /// (the existing Closure arm carries function_id + type_id +
    /// upvalues as `Vec<SerializableVMValue>` — restoration requires
    /// the ClosureLayout side-table on the receiver, which is itself
    /// part of the snapshot's program payload).
    LazyOpaque {
        is_initialized: bool,
    },

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
// Snapshot byte-slice reinterpret helpers; staged for the typed-buffer
// serialization path, currently uncalled.
#[allow(dead_code)]
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
#[allow(dead_code)]
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

/// STAGE-R5 serialize-side shared identity context (ADR-006 §2.7.30.5).
///
/// Threaded through every slot of a single VM-state snapshot walk so that
/// a promoted cell reached by N carriers (a `Reference(PromotedCell)` on
/// the stack + the `SharedCell` module binding it points at) is emitted
/// EXACTLY ONCE (the BODY, `SV::SharedCell { handle, inner }`) and every
/// later carrier emits a back-edge handle. `handle_of` keys on the
/// underlying `Arc<SharedCell>` allocation pointer (cast to `*const ()`),
/// so two carriers pointing at the same cell intern to the same handle.
///
/// `in_progress` is the reserve-before-recurse cycle guard: a cell whose
/// interior reaches back into itself finds its handle already present and
/// emits a back-edge rather than re-recursing.
///
/// NO `ValueWord`-shape carrier, NO tagged token: `handle` is a plain
/// `u64` counter and the key is a raw provenance pointer used only as a
/// `HashMap` key (never dereferenced for the key role).
#[derive(Default)]
pub struct SerializeIdentityCtx {
    /// `Arc<SharedCell>` allocation ptr → assigned handle.
    handle_of: std::collections::HashMap<*const (), u64>,
    /// Next handle to assign (monotonic).
    next_handle: u64,
    /// Cells whose BODY emission is mid-recursion (cycle guard).
    in_progress: std::collections::HashSet<*const ()>,
}

impl SerializeIdentityCtx {
    pub fn new() -> Self {
        Self::default()
    }
}

/// STAGE-R5 restore-side two-pass link context with abort-ledger
/// (ADR-006 §2.7.30.5 / §2.7.30.4).
///
/// Pass 1 materializes each `SV::SharedCell` BODY into exactly one
/// `Arc<SharedCell>` (recorded in `identity_map` as the raw ptr + a base
/// share held by the map). Pass 2 resolves every back-edge
/// (`SV::SharedCellRef` / `SV::Reference`) and every body-slot to that one
/// cell via `Arc::increment_strong_count`.
///
/// `retained` is the ABORT-LEDGER: every share handed out (base
/// materialization + each link increment) is recorded. On `Err` the
/// caller reverse-walks (LIFO) and releases each, so a mid-link failure
/// leaves NO leaked strong-count (which would break §2.7.30.4
/// deferred-Drop) and NO double-free. Mirrors the W5 / cluster-1.5
/// `clone_slot_kinded` retain-before-claim discipline
/// (`vm_state_snapshot.rs:295`).
///
/// `in_progress` is the Pass-1 VISITED-SET cycle guard: a cell whose
/// interior is itself a `Ptr(HeapKind::Reference)` back into a cell mid-
/// materialization is detected here and cleanly surface-refused (NOT a
/// depth bound), with the ledger balancing all retained shares.
#[derive(Default)]
pub struct RestoreLinkCtx {
    /// handle → materialized `*const SharedCell` (one base share held).
    identity_map: std::collections::HashMap<u64, u64>,
    /// Pass-1 cycle guard: handles whose body is mid-materialization.
    in_progress: std::collections::HashSet<u64>,
    /// Abort-ledger: every share handed out, in claim order. Reverse-walk
    /// (LIFO) to release on `Err`.
    retained: Vec<RetainedShare>,
}

/// One abort-ledger entry: a strong-count share to release on abort.
enum RetainedShare {
    /// An `Arc<SharedCell>` share at this raw ptr.
    SharedCell(u64),
}

impl RestoreLinkCtx {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of un-released base shares currently in the abort-ledger.
    pub fn ledger_len(&self) -> usize {
        self.retained.len()
    }

    /// Reverse-walk (LIFO) the ledger, releasing every base materialization
    /// share. Called on BOTH abort and restore-finish:
    ///
    /// - **Abort:** the per-slot link shares already returned to installed
    ///   slots are owned by those slots (their Drop releases them); the
    ///   ledger holds ONLY the Pass-1 base shares held by the identity-map,
    ///   so releasing them here balances the materialization with no
    ///   double-free of any slot-owned share. On a mid-link failure the
    ///   driver also drops the slots it already pushed (standard VM
    ///   teardown), so the net is balanced.
    /// - **Finish (success):** the identity-map was scaffolding; every real
    ///   Reference / SharedCell slot holds its OWN share (Pass 2), so the
    ///   base shares are surplus and must be released exactly once. Same
    ///   reverse-walk.
    ///
    /// Idempotent: `retained` is drained, so a second call is a no-op.
    pub fn release_base_shares(&mut self) {
        use shape_value::v2::closure_layout::SharedCell;
        while let Some(entry) = self.retained.pop() {
            match entry {
                RetainedShare::SharedCell(ptr) => unsafe {
                    Arc::decrement_strong_count(ptr as *const SharedCell);
                },
            }
        }
        self.identity_map.clear();
        self.in_progress.clear();
    }
}

/// Project a `(bits, kind)` slot pair into its `SerializableVMValue` arm.
///
/// This is the single-slot public entry. It owns a fresh
/// [`SerializeIdentityCtx`], so promoted-cell dedupe spans only this one
/// slot's deep walk. The whole-VM snapshot driver instead threads ONE
/// ctx across every slot via [`slot_to_serializable_ctx`] so a reference
/// and its referent cell — living in different slots — dedupe to one
/// restored cell (ADR-006 §2.7.30.5).
pub fn slot_to_serializable_ctx(
    bits: u64,
    kind: NativeKind,
    store: &SnapshotStore,
    ctx: &mut SerializeIdentityCtx,
) -> std::result::Result<SerializableVMValue, String> {
    match kind {
        NativeKind::Ptr(heap_kind) => slot_heap_to_serializable(bits, heap_kind, store, ctx),
        // Scalar + string + v2-raw kinds carry no promoted-cell identity;
        // delegate to the ctx-free body (it never touches `ctx`).
        _ => slot_to_serializable(bits, kind, store),
    }
}

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
        // R5b-2-bool-null-sentinel-cluster (ADR-006 §2.7 + §2.7.5 +
        // §2.7.7/Q9, 2026-05-19): `NativeKind::Null` is the canonical
        // absence-of-value discriminator. Pre-disposition this was
        // encoded as `(0u64, NativeKind::Bool)` colliding with
        // legitimate `false` bool values; post-disposition the kind
        // IS the discriminator. Project to `SV::None`.
        NativeKind::Null => Ok(SV::None),
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
        NativeKind::Ptr(heap_kind) => {
            // Fresh per-slot identity ctx: single-slot dedupe only. The
            // whole-VM driver uses `slot_to_serializable_ctx` to share
            // one ctx across slots (ADR-006 §2.7.30.5).
            let mut ctx = SerializeIdentityCtx::new();
            slot_heap_to_serializable(bits, heap_kind, _store, &mut ctx)
        }
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
    store: &SnapshotStore,
    ctx: &mut SerializeIdentityCtx,
) -> std::result::Result<SerializableVMValue, String> {
    use SerializableVMValue as SV;
    use shape_value::heap_value::{
        AtomicData, ChannelData, DequeData, HashSetData, LazyData, MutexData, OptionData,
        PriorityQueueData, ResultData,
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
            let has_value = !(matches!(inner.kind(), NativeKind::Bool) && inner.slot().raw() == 0);
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
                Some(Box::new(serializable_inner_kinded(
                    payload_bits,
                    payload_kind,
                )?))
            } else {
                None
            };
            let _ = Arc::into_raw(arc);
            Ok(SV::OptionData { is_some, payload })
        },
        // STAGE-R5 serialize-through (ADR-006 §2.7.30.5). The Reference
        // arm DISCRIMINATES on the `RefTarget` variant (KL-4 guard,
        // §2.7.30.7): only `PromotedCell` has an owning `Arc<SharedCell>`
        // and serializes-through; `Local` / `ModuleBinding` / `TypedField`
        // have NO owning cell — reading their bits as `*const SharedCell`
        // would be a wild-free, so they keep the opaque clean-refuse.
        HeapKind::Reference => serialize_reference(bits, store, ctx),
        HeapKind::FilterExpr => Ok(SV::FilterExprOpaque),
        HeapKind::SharedCell => serialize_shared_cell(bits, store, ctx),
        HeapKind::Iterator => Ok(SV::IteratorOpaque),
        // Future is inline u64 per §2.7.4.
        HeapKind::Future => Ok(SV::Future(bits)),

        // ── W17-snapshot-roundtrip container arms (2026-06-02) ─────────
        // TypedObject / TypedArray / HashMap<string,string> / Range —
        // the four §2.7.5.1 container/value shapes that the VmState
        // round-trip (frames Array<FrameState> + module_bindings
        // Map<string,any>) consumes. Each reconstructs the typed `Arc<T>`
        // (or v2-raw carrier) via the canonical 5-arm receiver-recovery
        // pattern, reads through a borrow (no ownership taken), and
        // projects to the matching `SerializableVMValue` arm. No
        // `Box<HeapValue>`, no ValueWord, no Bool-default.
        HeapKind::Range => {
            // SAFETY: bits = `Arc::into_raw(Arc<RangeData>)` per the
            // `ValueSlot::from_range` construction contract (slot.rs:309).
            // Clone-on-read + restore-share leaves the slot's share intact.
            unsafe {
                let arc = Arc::<shape_value::heap_value::RangeData>::from_raw(
                    bits as *const shape_value::heap_value::RangeData,
                );
                let start = arc.start;
                let end = arc.end;
                let inclusive = arc.inclusive;
                let _ = Arc::into_raw(arc);
                Ok(SV::Range {
                    start: Some(Box::new(SV::Int(start))),
                    end: Some(Box::new(SV::Int(end))),
                    inclusive,
                })
            }
        }
        HeapKind::TypedObject => {
            // The slot bits are a `*const TypedObjectStorage` carrier
            // (`from_typed_object` legacy Arc carrier OR `from_typed_object_raw`
            // v2-raw carrier — identical `#[repr(C)]` layout, HeapHeader
            // at offset 0). We read the struct fields through a borrow
            // WITHOUT touching the refcount (no ownership taken on the
            // way out); the schema_id + per-field parallel kind track
            // (`field_kinds[i]`) drive a per-field `slot_to_serializable`
            // recursion. ADR-006 §2.5 / §2.3.
            //
            // SAFETY: per the slot construction contract the bits point
            // to a live `TypedObjectStorage`; the borrow is valid for the
            // duration of the field reads (the slot keeps its share).
            let storage: &shape_value::heap_value::TypedObjectStorage =
                unsafe { &*(bits as *const shape_value::heap_value::TypedObjectStorage) };
            let schema_id = storage.schema_id;
            let heap_mask = storage.heap_mask;
            let slots = storage.slots();
            let n = slots.len();
            let mut slot_data: Vec<SerializableVMValue> = Vec::with_capacity(n);
            for i in 0..n {
                let field_bits = slots[i].raw();
                let field_kind = storage.field_kinds[i];
                let sv = slot_to_serializable_ctx(field_bits, field_kind, store, ctx)?;
                slot_data.push(sv);
            }
            Ok(SV::TypedObject {
                schema_id,
                slot_data,
                heap_mask,
            })
        }
        HeapKind::TypedArray => {
            // v2-raw flat-struct monomorphic carrier (`docs/runtime-v2-spec.md`):
            // the element-type discriminant is stamped at HeapHeader offset 7.
            // Scalar element kinds project to a generic `SV::Array(Vec<scalar>)`
            // — a store-free, lossless in-session round-trip. Heap-element
            // arrays (String / Decimal / TypedObject element type) surface
            // clean: their deep element walk lands in follow-up (and the
            // VmState `frames` array-of-FrameState path reads its elements
            // through the bespoke decode walk in `executor/resume.rs`, not
            // through this scalar projection).
            use shape_value::v2::typed_array::{
                ELEM_TYPE_BOOL, ELEM_TYPE_F32, ELEM_TYPE_F64, ELEM_TYPE_I8, ELEM_TYPE_I16,
                ELEM_TYPE_I32, ELEM_TYPE_I64, ELEM_TYPE_U8, ELEM_TYPE_U16, ELEM_TYPE_U32,
                TypedArray, read_elem_type,
            };
            let ptr = bits as *const u8;
            // SAFETY: the slot construction contract guarantees a live,
            // element-type-stamped TypedArray carrier at `bits`.
            let elem = unsafe { read_elem_type(ptr) };
            let elems: Vec<SerializableVMValue> = unsafe {
                match elem {
                    ELEM_TYPE_F64 => TypedArray::<f64>::as_slice(ptr as *const TypedArray<f64>)
                        .iter()
                        .map(|v| SV::Number(*v))
                        .collect(),
                    ELEM_TYPE_I64 => TypedArray::<i64>::as_slice(ptr as *const TypedArray<i64>)
                        .iter()
                        .map(|v| SV::Int(*v))
                        .collect(),
                    ELEM_TYPE_I32 => TypedArray::<i32>::as_slice(ptr as *const TypedArray<i32>)
                        .iter()
                        .map(|v| SV::Int(*v as i64))
                        .collect(),
                    ELEM_TYPE_I16 => TypedArray::<i16>::as_slice(ptr as *const TypedArray<i16>)
                        .iter()
                        .map(|v| SV::Int(*v as i64))
                        .collect(),
                    ELEM_TYPE_I8 => TypedArray::<i8>::as_slice(ptr as *const TypedArray<i8>)
                        .iter()
                        .map(|v| SV::Int(*v as i64))
                        .collect(),
                    ELEM_TYPE_BOOL | ELEM_TYPE_U8 => {
                        TypedArray::<u8>::as_slice(ptr as *const TypedArray<u8>)
                            .iter()
                            .map(|v| {
                                if elem == ELEM_TYPE_BOOL {
                                    SV::Bool(*v != 0)
                                } else {
                                    SV::Int(*v as i64)
                                }
                            })
                            .collect()
                    }
                    ELEM_TYPE_U16 => TypedArray::<u16>::as_slice(ptr as *const TypedArray<u16>)
                        .iter()
                        .map(|v| SV::Int(*v as i64))
                        .collect(),
                    ELEM_TYPE_U32 => TypedArray::<u32>::as_slice(ptr as *const TypedArray<u32>)
                        .iter()
                        .map(|v| SV::Int(*v as i64))
                        .collect(),
                    ELEM_TYPE_F32 => TypedArray::<f32>::as_slice(ptr as *const TypedArray<f32>)
                        .iter()
                        .map(|v| SV::Number(f64::from(*v)))
                        .collect(),
                    other_elem => {
                        return Err(format!(
                            "slot_to_serializable: W17-snapshot-roundtrip surface — \
                             TypedArray element-type discriminant {other_elem} is \
                             not in the scalar round-trip set (heap-element arrays \
                             — String / Decimal / TypedObject — land in follow-up). \
                             ADR-006 §2.7.5.1."
                        ));
                    }
                }
            };
            Ok(SV::Array(elems))
        }
        HeapKind::HashMap => {
            // K1 string→string monomorphization only. The slot bits are
            // `Arc::into_raw(Arc<HashMapKindedRef>)` per `from_hashmap`
            // (slot.rs:244). We clone-on-read the outer Arc, dispatch on
            // the `HashMapKindedRef::String` variant, and read the
            // `*const StringObj` keys + values via the v2-raw TypedArray
            // walk (mirror of `json_value.rs::heap_to_json_value`). All
            // other value monomorphizations are K3 (heap-value track
            // amendment) and surface clean.
            use shape_value::heap_value::HashMapKindedRef;
            // SAFETY: bits = `Arc::into_raw(Arc<HashMapKindedRef>)`.
            let arc = unsafe { Arc::<HashMapKindedRef>::from_raw(bits as *const HashMapKindedRef) };
            let kref: HashMapKindedRef = (*arc).clone();
            let _ = Arc::into_raw(arc); // restore the slot's original share
            match &kref {
                HashMapKindedRef::String(map_arc) => {
                    let n = map_arc.len();
                    let mut keys: Vec<SerializableVMValue> = Vec::with_capacity(n);
                    let mut values: Vec<SerializableVMValue> = Vec::with_capacity(n);
                    for i in 0..n {
                        // SAFETY: keys/values buffers are live for the
                        // lifetime of `map_arc`; elements are `*const
                        // StringObj` per the String monomorphization.
                        unsafe {
                            let kp = shape_value::v2::typed_array::TypedArray::get_unchecked(
                                map_arc.keys,
                                i as u32,
                            );
                            let vp: *const shape_value::v2::string_obj::StringObj =
                                *(*map_arc.values).data.add(i);
                            keys.push(SV::String(
                                shape_value::v2::string_obj::StringObj::as_str(kp).to_owned(),
                            ));
                            values.push(SV::String(
                                shape_value::v2::string_obj::StringObj::as_str(vp).to_owned(),
                            ));
                        }
                    }
                    Ok(SV::HashMap { keys, values })
                }
                other_v => Err(format!(
                    "slot_to_serializable: W17-snapshot-roundtrip surface — \
                     HashMap value-monomorphization {} is K3 (the heap-value \
                     kinded-track amendment); only HashMap<string,string> \
                     round-trips at this scope. ADR-006 §2.7.5.1.",
                    hashmap_kinded_ref_arm_name(other_v),
                )),
            }
        }

        // Pre-existing complex shapes: surface-and-stop per §2.7.5.1.
        // These have rich pre-bulldozer SerializableVMValue arms whose
        // construction requires more than typed-Arc recovery (DataTable /
        // TableView / Temporal / TaskGroup / IoHandle / NativeView /
        // NativeScalar / Content / ClosureRaw each have their own
        // multi-step landing path).
        other => Err(format!(
            "slot_to_serializable: W17-snapshot-roundtrip surface — \
             HeapKind::{other:?} arm has no in-session SerializableVMValue \
             projection. Tracked as W17-snapshot-{other:?} follow-up per \
             docs/cluster-audits/phase-2d-playbook.md §3. \
             ADR-006 §2.7.5.1.",
        )),
    }
}

/// STAGE-R5: serialize a `HeapKind::Reference` slot (ADR-006 §2.7.30.5).
///
/// The slot bits are `Arc::into_raw(Arc<RefTarget>) as u64` (reference.rs).
/// We recover the `Arc<RefTarget>` (restoring the share on the way out)
/// and DISCRIMINATE on the variant (the KL-4 guard, §2.7.30.7):
///
/// - `PromotedCell { cell, .. }` — the owning carrier. Recover the
///   underlying `Arc<SharedCell>` ptr, intern it in the identity-map. If
///   this is the FIRST carrier to reach the cell, emit the BODY
///   (`SV::SharedCell { handle, inner }`); otherwise emit the back-edge
///   (`SV::Reference { handle, is_mut }`).
/// - `Local` / `ModuleBinding` / `TypedField` — NO owning cell. Reading
///   their bits as `*const SharedCell` is a wild-free. Keep the opaque
///   clean-refuse: surface a structured error, never serialize-through.
fn serialize_reference(
    bits: u64,
    store: &SnapshotStore,
    ctx: &mut SerializeIdentityCtx,
) -> std::result::Result<SerializableVMValue, String> {
    use shape_value::reference::RefTarget;
    // SAFETY: kind == Ptr(HeapKind::Reference) ⇒ bits = Arc::into_raw(
    // Arc<RefTarget>) per reference.rs:11-12. Recover, inspect, restore
    // the original share (we do NOT consume it).
    let arc = unsafe { Arc::<RefTarget>::from_raw(bits as *const RefTarget) };
    let result = match &*arc {
        RefTarget::PromotedCell { cell, .. } => {
            // Recover the underlying Arc<SharedCell> allocation ptr as the
            // identity key. `Arc::as_ptr` does NOT touch the refcount.
            // `is_mut` is reserved-not-read in v0.3.3 (§2.7.30.5 option a);
            // the PromotedCell carrier records no mutability today, so the
            // back-edge defaults to `is_mut: false`.
            let cell_ptr = Arc::as_ptr(cell) as *const ();
            emit_or_backedge_cell(cell_ptr, cell, store, ctx, RefArmKind::Reference)
        }
        RefTarget::Local { .. }
        | RefTarget::ModuleBinding { .. }
        | RefTarget::TypedField { .. }
        // V3-S5 Seam #2 (2026-06-05): an `IndexedElement` ref owns a
        // `TypedArrayPtr` array share but has no owning `SharedCell` identity
        // to serialize — same clean-refuse class as the other non-promoted
        // refs. Snapshotting an array-element reference is out-of-territory
        // (no cycle/identity participation); surface rather than fabricate.
        | RefTarget::IndexedElement { .. } => Err(
            "serialize_reference: STAGE-R5 KL-4 guard — a non-promoted \
             reference (Local / ModuleBinding / TypedField / IndexedElement) \
             has no owning SharedCell; serializing-through would read its bits \
             as *const SharedCell (a wild-free). Clean-refuse by design. \
             ADR-006 §2.7.30.7."
                .to_string(),
        ),
    };
    let _ = Arc::into_raw(arc); // restore the slot's original share
    result
}

/// STAGE-R5: serialize a `HeapKind::SharedCell` slot (ADR-006 §2.7.30.5).
///
/// The slot bits are `Arc::into_raw(Arc<SharedCell>) as u64`. Recover the
/// `Arc<SharedCell>` (restoring the share) and intern it. First carrier
/// emits the BODY; a later carrier emits the `SV::SharedCellRef` back-edge.
fn serialize_shared_cell(
    bits: u64,
    store: &SnapshotStore,
    ctx: &mut SerializeIdentityCtx,
) -> std::result::Result<SerializableVMValue, String> {
    use shape_value::v2::closure_layout::SharedCell;
    // SAFETY: kind == Ptr(HeapKind::SharedCell) ⇒ bits = Arc::into_raw(
    // Arc<SharedCell>) per `op_alloc_shared_*` (stack.rs:376). Recover,
    // inspect, restore the original share.
    let arc = unsafe { Arc::<SharedCell>::from_raw(bits as *const SharedCell) };
    let cell_ptr = Arc::as_ptr(&arc) as *const ();
    let result = emit_or_backedge_cell(cell_ptr, &arc, store, ctx, RefArmKind::SharedCell);
    let _ = Arc::into_raw(arc); // restore the slot's original share
    result
}

/// Which serialize arm reached the cell — selects the back-edge shape.
#[derive(Clone, Copy)]
enum RefArmKind {
    Reference,
    SharedCell,
}

/// The either-arm body: the FIRST arm to reach a cell ptr emits the BODY
/// (`SV::SharedCell { handle, inner }`); a LATER arm emits a back-edge
/// (`SV::Reference` for the Reference arm / `SV::SharedCellRef` for the
/// SharedCell arm). This is the round-3 fix for the asymmetry where only
/// the SharedCell arm emitted the body — so `return &x` with the stack
/// reference serialized before the module binding never emitted the body.
///
/// `cell` is a borrow of the live `Arc<SharedCell>` (the caller restored
/// the slot's share); we read its `value` + `kind()` through `lock()` and
/// recurse via `slot_to_serializable_ctx` (so a cell whose interior
/// contains another reference participates in the same dedupe + cycle
/// guard).
fn emit_or_backedge_cell(
    cell_ptr: *const (),
    cell: &Arc<shape_value::v2::closure_layout::SharedCell>,
    store: &SnapshotStore,
    ctx: &mut SerializeIdentityCtx,
    arm: RefArmKind,
) -> std::result::Result<SerializableVMValue, String> {
    use SerializableVMValue as SV;
    // Already interned → back-edge. (Either a sibling carrier emitted the
    // body, OR we are mid-recursion on this very cell — the cycle case;
    // `in_progress` makes the back-edge terminate.)
    if let Some(&handle) = ctx.handle_of.get(&cell_ptr) {
        return Ok(match arm {
            RefArmKind::Reference => SV::Reference {
                handle,
                is_mut: false,
            },
            RefArmKind::SharedCell => SV::SharedCellRef { handle },
        });
    }
    // First reach → assign handle, reserve-before-recurse (cycle guard),
    // deep-walk the cell payload, emit the BODY.
    let handle = ctx.next_handle;
    ctx.next_handle += 1;
    ctx.handle_of.insert(cell_ptr, handle);
    ctx.in_progress.insert(cell_ptr);

    // Read the cell's value bits + kind under the lock (the kind companion
    // is fixed at construction; the value bits are stable while we hold
    // the lock). Drop the guard before recursing so a re-entrant reach on
    // the SAME cell (cycle) does not deadlock — the `in_progress` /
    // `handle_of` entry already routes it to a back-edge.
    let (value_bits, value_kind) = {
        let guard = cell.lock();
        (*guard, cell.kind())
    };
    let inner = slot_to_serializable_ctx(value_bits, value_kind, store, ctx)?;

    ctx.in_progress.remove(&cell_ptr);
    Ok(SV::SharedCell {
        handle,
        inner: Box::new(inner),
    })
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
/// STAGE-R5 Pass 1 — materialize every `SV::SharedCell` BODY reachable
/// from `sv` into exactly one `Arc<SharedCell>` per handle, recorded in
/// `ctx.identity_map` (ADR-006 §2.7.30.5). Recurses through the cell
/// interior so nested-body cells are materialized too. The base
/// materialization share is recorded in the abort-ledger.
///
/// A cell whose interior is a `Ptr(HeapKind::Reference)` cycle back into a
/// cell mid-materialization is detected via `ctx.in_progress` (the
/// VISITED-SET, NOT a depth bound) and cleanly surface-refused; the caller
/// runs `ctx.abort_release()` so all retained shares balance.
pub fn materialize_cell_bodies(
    sv: &SerializableVMValue,
    store: &SnapshotStore,
    ctx: &mut RestoreLinkCtx,
) -> std::result::Result<(), String> {
    use SerializableVMValue as SV;
    use shape_value::v2::closure_layout::SharedCell;
    match sv {
        SV::SharedCell { handle, inner } => {
            if ctx.identity_map.contains_key(handle) {
                // Already materialized by a sibling body slot — the wire
                // shape should only emit one body per handle, but be
                // idempotent: do not double-materialize.
                return Ok(());
            }
            if ctx.in_progress.contains(handle) {
                return Err(format!(
                    "materialize_cell_bodies: STAGE-R5 cycle surface — cell \
                     handle {handle} reached itself mid-materialization \
                     (Ptr(HeapKind::Reference) interior cycle). Clean-refuse \
                     with abort-ledger balancing. ADR-006 §2.7.30.5."
                ));
            }
            ctx.in_progress.insert(*handle);
            // First materialize any nested bodies in the interior.
            materialize_cell_bodies(inner, store, ctx)?;
            // Reconstruct the cell's interior slot (bits + kind). For a
            // back-edge interior (Reference/SharedCellRef) Pass 1 has
            // already materialized the target body, so the link resolves.
            let expected = expected_kind_for_cell_inner(inner);
            let (value_bits, value_kind) = serializable_to_slot_ctx(inner, expected, store, ctx)?;
            // Build the one owning cell. `Arc::into_raw` hands the base
            // share to the identity-map; record it in the ledger.
            let cell = Arc::new(SharedCell::new(value_bits, value_kind));
            let ptr = Arc::into_raw(cell) as u64;
            ctx.identity_map.insert(*handle, ptr);
            ctx.retained.push(RetainedShare::SharedCell(ptr));
            ctx.in_progress.remove(handle);
            Ok(())
        }
        // Recurse into compound arms that can carry a nested body.
        SV::TypedObject { slot_data, .. } => {
            for f in slot_data {
                materialize_cell_bodies(f, store, ctx)?;
            }
            Ok(())
        }
        // Back-edges + leaf arms hold no body to materialize.
        _ => Ok(()),
    }
}

/// Pick the `expected_kind` for a cell BODY's interior slot from its
/// serialized discriminator. The cell `value` is restored through
/// `serializable_to_slot_ctx`; a Reference/SharedCellRef interior resolves
/// to its `Ptr(HeapKind::*)` so the link path fires.
fn expected_kind_for_cell_inner(sv: &SerializableVMValue) -> NativeKind {
    use SerializableVMValue as SV;
    match sv {
        SV::Reference { .. } => NativeKind::Ptr(HeapKind::Reference),
        SV::SharedCell { .. } | SV::SharedCellRef { .. } => NativeKind::Ptr(HeapKind::SharedCell),
        SV::Int(_) => NativeKind::Int64,
        SV::Number(_) => NativeKind::Float64,
        SV::Bool(_) => NativeKind::Bool,
        SV::String(_) => NativeKind::String,
        SV::None | SV::Unit => NativeKind::Null,
        other => expected_heap_field_kind(other),
    }
}

/// STAGE-R5 restore entry with the shared two-pass link context.
///
/// The whole-VM restore driver runs Pass 1 ([`materialize_cell_bodies`])
/// over every slot first, then Pass 2 by calling this per slot. The new
/// `Reference` / `SharedCell` / `SharedCellRef` arms resolve their handle
/// against `ctx.identity_map` (materialized in Pass 1); everything else
/// delegates to the ctx-free [`serializable_to_slot`].
pub fn serializable_to_slot_ctx(
    sv: &SerializableVMValue,
    expected_kind: NativeKind,
    store: &SnapshotStore,
    ctx: &mut RestoreLinkCtx,
) -> std::result::Result<(u64, NativeKind), String> {
    use SerializableVMValue as SV;
    match (sv, expected_kind) {
        // Body / back-edge into a Reference slot → an Arc<RefTarget::
        // PromotedCell> owning one share on the resolved cell.
        (SV::SharedCell { handle, .. }, NativeKind::Ptr(HeapKind::Reference))
        | (SV::Reference { handle, .. }, NativeKind::Ptr(HeapKind::Reference)) => {
            link_promoted_reference(*handle, ctx)
        }
        // Body / back-edge into a SharedCell slot → an Arc<SharedCell>
        // owning one share on the resolved cell.
        (SV::SharedCell { handle, .. }, NativeKind::Ptr(HeapKind::SharedCell))
        | (SV::SharedCellRef { handle }, NativeKind::Ptr(HeapKind::SharedCell)) => {
            link_shared_cell(*handle, ctx)
        }
        // Everything else — ctx-free.
        _ => serializable_to_slot(sv, expected_kind, store),
    }
}

/// Pass 2 — resolve a handle to its materialized `Arc<SharedCell>` and
/// hand out ONE owned share for a SharedCell-kinded slot.
///
/// Share accounting: the identity-map holds the base materialization
/// share (recorded in the ledger, released at restore-finish). We
/// `increment_strong_count` then `from_raw` + `into_raw` so the returned
/// slot bits own exactly ONE share, transferred to the caller's slot — it
/// is NOT in the abort-ledger (the slot's own Drop owns it once installed).
fn link_shared_cell(
    handle: u64,
    ctx: &mut RestoreLinkCtx,
) -> std::result::Result<(u64, NativeKind), String> {
    use shape_value::v2::closure_layout::SharedCell;
    if ctx.in_progress.contains(&handle) {
        return Err(format!(
            "link_shared_cell: STAGE-R5 cycle surface — handle {handle} is \
             still mid-materialization (in_progress VISITED-SET): a \
             SharedCell whose interior is a Ptr(HeapKind::Reference) cycles \
             back into itself. Clean-refuse with abort-ledger balancing. \
             ADR-006 §2.7.30.5."
        ));
    }
    let ptr = *ctx.identity_map.get(&handle).ok_or_else(|| {
        format!(
            "link_shared_cell: STAGE-R5 surface — handle {handle} has no \
             materialized cell (Pass-1 body missing). ADR-006 §2.7.30.5."
        )
    })?;
    // SAFETY: ptr is a live Arc<SharedCell> (Pass 1 materialized it; the
    // identity-map holds the base share). Bump one share, transfer it into
    // the returned slot bits via into_raw.
    unsafe {
        Arc::increment_strong_count(ptr as *const SharedCell);
        let cell = Arc::<SharedCell>::from_raw(ptr as *const SharedCell);
        let raw = Arc::into_raw(cell) as u64;
        Ok((raw, NativeKind::Ptr(HeapKind::SharedCell)))
    }
}

/// Pass 2 — resolve a handle to its materialized `Arc<SharedCell>`, hand
/// out one owned share, and wrap it in an `Arc<RefTarget::PromotedCell>`
/// for a Reference-kinded slot. The cell's `kind()` is the ref's projected
/// kind. The returned slot bits own one `Arc<RefTarget>` share whose inner
/// `cell` field owns one `Arc<SharedCell>` share — both released by the
/// Reference slot's normal Drop (NOT in the abort-ledger).
fn link_promoted_reference(
    handle: u64,
    ctx: &mut RestoreLinkCtx,
) -> std::result::Result<(u64, NativeKind), String> {
    use shape_value::reference::RefTarget;
    use shape_value::v2::closure_layout::SharedCell;
    if ctx.in_progress.contains(&handle) {
        return Err(format!(
            "link_promoted_reference: STAGE-R5 cycle surface — handle \
             {handle} is still mid-materialization (in_progress VISITED-SET): \
             a SharedCell whose interior references back into itself. \
             Clean-refuse with abort-ledger balancing. ADR-006 §2.7.30.5."
        ));
    }
    let ptr = *ctx.identity_map.get(&handle).ok_or_else(|| {
        format!(
            "link_promoted_reference: STAGE-R5 surface — handle {handle} has \
             no materialized cell (Pass-1 body missing). ADR-006 §2.7.30.5."
        )
    })?;
    // SAFETY: ptr is a live Arc<SharedCell>. Bump one share and reconstruct
    // the Arc so the RefTarget::PromotedCell owns exactly that share.
    let (raw, _kind) = unsafe {
        Arc::increment_strong_count(ptr as *const SharedCell);
        let cell = Arc::<SharedCell>::from_raw(ptr as *const SharedCell);
        let projected_kind = cell.kind();
        let rt = Arc::new(RefTarget::PromotedCell {
            cell,
            kind: projected_kind,
        });
        (Arc::into_raw(rt) as u64, projected_kind)
    };
    Ok((raw, NativeKind::Ptr(HeapKind::Reference)))
}

pub fn serializable_to_slot(
    sv: &SerializableVMValue,
    expected_kind: NativeKind,
    store: &SnapshotStore,
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
        (sv, NativeKind::Ptr(hk)) => serializable_to_heap_slot(sv, hk, store),

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

/// Map a `SerializableVMValue` to the `NativeKind` its restored field
/// slot should carry — the per-field analogue of the shape-vm
/// `expected_kind_from_serializable`. Used by the TypedObject restore arm
/// to pick each field's `expected_kind` from its discriminator. The
/// returned kind is a hint; `serializable_to_slot` re-derives the actual
/// kind from the arm and surfaces on mismatch (no Bool-default).
fn expected_heap_field_kind(sv: &SerializableVMValue) -> NativeKind {
    use SerializableVMValue as SV;
    match sv {
        SV::Int(_) => NativeKind::Int64,
        SV::Number(_) => NativeKind::Float64,
        SV::Bool(_) => NativeKind::Bool,
        SV::String(_) => NativeKind::String,
        SV::None | SV::Unit => NativeKind::Bool,
        SV::Decimal(_) => NativeKind::Ptr(HeapKind::Decimal),
        SV::BigInt(_) => NativeKind::Ptr(HeapKind::BigInt),
        SV::Char(_) => NativeKind::Ptr(HeapKind::Char),
        SV::Range { .. } => NativeKind::Ptr(HeapKind::Range),
        SV::TypedObject { .. } => NativeKind::Ptr(HeapKind::TypedObject),
        SV::HashMap { .. } => NativeKind::Ptr(HeapKind::HashMap),
        SV::Array(_) => NativeKind::Ptr(HeapKind::TypedArray),
        SV::HashSet { .. } => NativeKind::Ptr(HeapKind::HashSet),
        SV::ResultData { .. } => NativeKind::Ptr(HeapKind::Result),
        SV::OptionData { .. } => NativeKind::Ptr(HeapKind::Option),
        // Pre-existing complex arms — surface clean rather than guess.
        _ => NativeKind::Bool,
    }
}

/// Inverse of [`slot_heap_to_serializable`] — reconstruct a heap-kinded
/// slot from its serialized arm. Returns `(bits, NativeKind)` ready
/// to push to a slot. The reconstructed slot owns one strong-count
/// share on the typed `Arc<T>` carrier.
fn serializable_to_heap_slot(
    sv: &SerializableVMValue,
    heap_kind: HeapKind,
    store: &SnapshotStore,
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
                    None => {
                        return Err(
                            "serializable_to_slot: OptionData is_some=true but payload=None — \
                         malformed wire shape; expected Some(SerializableVMValue) for \
                         is_some=true. ADR-006 §2.7.5.1."
                                .to_string(),
                        );
                    }
                }
            } else {
                OptionData::none()
            };
            let arc = Arc::new(data);
            let raw = Arc::into_raw(arc) as u64;
            Ok((raw, NativeKind::Ptr(HeapKind::Option)))
        }

        // ── W17-snapshot-roundtrip container restore arms (2026-06-02) ──
        // Inverse of the `slot_heap_to_serializable` container arms.
        (
            SV::Range {
                start,
                end,
                inclusive,
            },
            HeapKind::Range,
        ) => {
            // RangeData carries i64 bounds + step + inclusive. The wire
            // shape only persists start/end/inclusive; step defaults to 1
            // (the surface-syntax `start..end` shape per RangeData docstring).
            let start_i = match start.as_deref() {
                Some(SV::Int(i)) => *i,
                _ => {
                    return Err("serializable_to_slot: Range restore — start bound is not \
                     an Int (only i64 ranges are representable). ADR-006 §2.7.23."
                        .to_string());
                }
            };
            let end_i = match end.as_deref() {
                Some(SV::Int(i)) => *i,
                _ => {
                    return Err("serializable_to_slot: Range restore — end bound is not \
                     an Int (only i64 ranges are representable). ADR-006 §2.7.23."
                        .to_string());
                }
            };
            let data = shape_value::heap_value::RangeData::new(start_i, end_i, 1, *inclusive);
            let arc = Arc::new(data);
            let raw = Arc::into_raw(arc) as u64;
            Ok((raw, NativeKind::Ptr(HeapKind::Range)))
        }
        (
            SV::TypedObject {
                schema_id,
                slot_data,
                heap_mask,
            },
            HeapKind::TypedObject,
        ) => {
            // Rebuild a `TypedObjectStorage` from the per-field restored
            // slots. Each field restores through `serializable_to_slot`
            // (recursion); the returned `(bits, kind)` populate the slot
            // array and the parallel `field_kinds` track. The
            // `from_typed_object` (Arc carrier) constructor moves one
            // strong-count share into the slot. ADR-006 §2.3 / §2.5.
            use shape_value::ValueSlot;
            let n = slot_data.len();
            let mut slots: Vec<ValueSlot> = Vec::with_capacity(n);
            let mut field_kinds: Vec<NativeKind> = Vec::with_capacity(n);
            for (i, fsv) in slot_data.iter().enumerate() {
                let expected = expected_heap_field_kind(fsv);
                let (fbits, fkind) = serializable_to_slot(fsv, expected, store).map_err(|msg| {
                    format!(
                        "serializable_to_slot: TypedObject restore field[{i}] \
                             (schema_id={schema_id}): {msg}"
                    )
                })?;
                slots.push(ValueSlot::from_raw(fbits));
                field_kinds.push(fkind);
            }
            let field_kinds_arc: Arc<[NativeKind]> = field_kinds.into();
            // Allocate via the v2-raw `_new` carrier (refcount=1 on the
            // HeapHeader at offset 0) so the slot's release path
            // (`drop_with_kind` → `TypedObjectStorage::release_elem` →
            // carrier-side `_drop` + `std::alloc::dealloc`) matches the
            // allocation. The legacy `Arc::new(...)` + `Arc::into_raw`
            // carrier would mismatch the allocator at drop time (the
            // `length_typed_object_empty` allocator-pair SIGABRT class
            // per the v2-raw-heap-audit). ADR-006 §2.3 amendment (Wave 2
            // Agent D1/D2).
            let ptr = shape_value::heap_value::TypedObjectStorage::_new(
                *schema_id,
                slots.into_boxed_slice(),
                *heap_mask,
                field_kinds_arc,
            );
            Ok((ptr as u64, NativeKind::Ptr(HeapKind::TypedObject)))
        }
        (SV::HashMap { keys, values }, HeapKind::HashMap) => {
            // K1 string→string restore only — mirror of the K1
            // `project_concrete_return::HashMapStringString` builder.
            // Non-string value arms (K3) never produce an `SV::HashMap`
            // with string values, so a non-String value here is a
            // malformed wire shape; surface clean.
            use shape_value::heap_value::{HashMapData, HashMapKindedRef};
            use shape_value::v2::string_obj::StringObj;
            if keys.len() != values.len() {
                return Err(format!(
                    "serializable_to_slot: HashMap restore — keys/values length \
                     mismatch (keys={}, values={}). Malformed wire shape. \
                     ADR-006 §2.7.5.1.",
                    keys.len(),
                    values.len(),
                ));
            }
            let mut data: HashMapData<*const StringObj> = HashMapData::new();
            for (k, v) in keys.iter().zip(values.iter()) {
                let (ks, vs) = match (k, v) {
                    (SV::String(ks), SV::String(vs)) => (ks, vs),
                    _ => {
                        return Err("serializable_to_slot: HashMap restore — only \
                         HashMap<string,string> round-trips at this scope; \
                         a non-String key/value pair is K3 (heap-value track) \
                         or malformed. ADR-006 §2.7.5.1."
                            .to_string());
                    }
                };
                let value_ptr = StringObj::new(vs.as_str()) as *const StringObj;
                // SAFETY: `value_ptr` is a fresh StringObj (refcount = 1);
                // `insert` takes ownership of that single share + allocates
                // a fresh key StringObj internally.
                unsafe {
                    data.insert(ks.as_str(), value_ptr);
                }
            }
            let kref = Arc::new(HashMapKindedRef::String(Arc::new(data)));
            let raw = Arc::into_raw(kref) as u64;
            Ok((raw, NativeKind::Ptr(HeapKind::HashMap)))
        }
        (SV::Array(elems), HeapKind::TypedArray) => {
            // Restore a scalar-element TypedArray. The wire shape lost the
            // exact element type (an `SV::Array` of `SV::Int` could have
            // been i8/i16/i32/i64/u*); we pick the widest matching scalar
            // carrier from the first element's discriminator (Int → i64,
            // Number → f64, Bool → u8/ELEM_TYPE_BOOL). An empty array maps
            // to a stamped zero-length i64 carrier. Heterogeneous or
            // non-scalar elements surface clean — those came from a
            // non-scalar source and don't round-trip through this arm.
            use shape_value::v2::typed_array::{
                ELEM_TYPE_BOOL, ELEM_TYPE_F64, ELEM_TYPE_I64, TypedArray, stamp_elem_type,
            };
            // Classify by the first element (empty → i64).
            let first = elems.first();
            let raw_bits = match first {
                None | Some(SV::Int(_)) => {
                    let mut v: Vec<i64> = Vec::with_capacity(elems.len());
                    for e in elems {
                        match e {
                            SV::Int(i) => v.push(*i),
                            _ => {
                                return Err("serializable_to_slot: TypedArray restore — \
                                 heterogeneous element (expected all Int). \
                                 ADR-006 §2.7.5.1."
                                    .to_string());
                            }
                        }
                    }
                    let arr = TypedArray::<i64>::from_slice(&v);
                    unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64) };
                    arr as usize as u64
                }
                Some(SV::Number(_)) => {
                    let mut v: Vec<f64> = Vec::with_capacity(elems.len());
                    for e in elems {
                        match e {
                            SV::Number(f) => v.push(*f),
                            _ => {
                                return Err("serializable_to_slot: TypedArray restore — \
                                 heterogeneous element (expected all Number). \
                                 ADR-006 §2.7.5.1."
                                    .to_string());
                            }
                        }
                    }
                    let arr = TypedArray::<f64>::from_slice(&v);
                    unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_F64) };
                    arr as usize as u64
                }
                Some(SV::Bool(_)) => {
                    let mut v: Vec<u8> = Vec::with_capacity(elems.len());
                    for e in elems {
                        match e {
                            SV::Bool(b) => v.push(if *b { 1 } else { 0 }),
                            _ => {
                                return Err("serializable_to_slot: TypedArray restore — \
                                 heterogeneous element (expected all Bool). \
                                 ADR-006 §2.7.5.1."
                                    .to_string());
                            }
                        }
                    }
                    let arr = TypedArray::<u8>::from_slice(&v);
                    unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_BOOL) };
                    arr as usize as u64
                }
                Some(other) => {
                    return Err(format!(
                        "serializable_to_slot: TypedArray restore — element arm \
                         {} is not in the scalar round-trip set (Int / Number / \
                         Bool). Heap-element arrays land in follow-up. \
                         ADR-006 §2.7.5.1.",
                        serializable_arm_name(other),
                    ));
                }
            };
            Ok((raw_bits, NativeKind::Ptr(HeapKind::TypedArray)))
        }

        // Clean-refuse-by-design arms (RULED disposition) — these heap
        // kinds wrap a *live, in-process resource* (an in-flight
        // iterator cursor, a deque/channel buffer, a query-DSL filter
        // node) that is intrinsically not snapshot-restorable. The wire
        // shape is discriminator-only by design; restoration refuses
        // cleanly rather than fabricating a placeholder. This is not a
        // pending follow-up — it is the terminal behavior (§2.7.4
        // invariant).
        (SV::IteratorOpaque, HeapKind::Iterator)
        | (SV::DequeOpaque { .. }, HeapKind::Deque)
        | (SV::ChannelOpaque { .. }, HeapKind::Channel)
        | (SV::FilterExprOpaque, HeapKind::FilterExpr) => Err(format!(
            "serializable_to_slot: W17-snapshot-roundtrip surface — \
             {heap_kind:?} is clean-refuse by design (live in-process \
             resource, not snapshot-restorable). ADR-006 §2.7.5.1.",
        )),

        // STAGE-R5: the Reference / SharedCell serialize-through arms
        // require the two-pass `RestoreLinkCtx` (identity-map resolution).
        // Reaching them via the ctx-FREE `serializable_to_slot` path means
        // a caller restored a promoted-reference snapshot without the
        // whole-VM two-pass driver — surface-and-stop, never fabricate.
        (SV::Reference { .. }, HeapKind::Reference)
        | (SV::SharedCell { .. }, HeapKind::SharedCell)
        | (SV::SharedCell { .. }, HeapKind::Reference)
        | (SV::SharedCellRef { .. }, HeapKind::SharedCell) => Err(format!(
            "serializable_to_slot: STAGE-R5 surface — {heap_kind:?} \
             serialize-through arm requires the two-pass RestoreLinkCtx \
             driver (serializable_to_slot_ctx); the ctx-free path cannot \
             resolve the identity-map handle. ADR-006 §2.7.30.5.",
        )),

        // Opaque arms — surface-and-stop on restore. These produced
        // discriminator-only wire shapes; the inner payload is lost.
        (SV::MutexOpaque { .. }, HeapKind::Mutex) | (SV::LazyOpaque { .. }, HeapKind::Lazy) => {
            Err(format!(
                "serializable_to_slot: W17-snapshot-roundtrip surface — \
             {heap_kind:?} arm restored from opaque wire shape; \
             deep payload reconstruction is the W17-snapshot-{:?} \
             follow-up. ADR-006 §2.7.5.1.",
                heap_kind,
            ))
        }

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
        SV::Unit | SV::None => Ok(KindedSlot::new(ValueSlot::from_raw(0), NativeKind::Bool)),
        other => Err(format!(
            "inner_kinded_from_serializable: W17-snapshot-roundtrip surface — \
             SerializableVMValue arm {} has no in-session inner-payload \
             projection. Tracked as follow-up. ADR-006 §2.7.5.1.",
            serializable_arm_name(other),
        )),
    }
}

/// One-line discriminator name for `HashMapKindedRef` value
/// monomorphizations (K3 surface diagnostics).
fn hashmap_kinded_ref_arm_name(kref: &shape_value::heap_value::HashMapKindedRef) -> &'static str {
    use shape_value::heap_value::HashMapKindedRef as K;
    match kref {
        K::I64(_) => "I64",
        K::F64(_) => "F64",
        K::Bool(_) => "Bool",
        K::Char(_) => "Char",
        K::String(_) => "String",
        K::Decimal(_) => "Decimal",
        K::TypedObject(_) => "TypedObject",
        K::TraitObject(_) => "TraitObject",
        K::HashMap(_) => "HashMap",
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
        SV::Reference { .. } => "Reference",
        SV::SharedCell { .. } => "SharedCell",
        SV::SharedCellRef { .. } => "SharedCellRef",
        SV::FilterExprOpaque => "FilterExprOpaque",
        SV::MutexOpaque { .. } => "MutexOpaque",
        SV::AtomicI64 { .. } => "AtomicI64",
        SV::LazyOpaque { .. } => "LazyOpaque",
        SV::Char(_) => "Char",
        SV::BigInt(_) => "BigInt",
    }
}

// DataTable snapshot (de)serialization via Arrow IPC; staged ahead of the
// snapshot wire path that drives it.
#[allow(dead_code)]
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

#[allow(dead_code)]
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
mod opaque_disposition_tests {
    //! Track A / A3 (2026-06-02): the restore-side opaque arms split into
    //! two dispositions. Iterator / Deque / Channel / FilterExpr wrap a
    //! live in-process resource and are **clean-refuse by design** (the
    //! RULED terminal behavior, not a pending follow-up). Reference /
    //! SharedCell / Mutex / Lazy keep the "deep payload reconstruction is
    //! the W17-snapshot follow-up" wording (Mutex/Lazy reset disposition +
    //! Reference/SharedCell identity-handle disposition are owned by other
    //! workstreams). All eight still surface-and-stop; only the message
    //! text differs.

    use super::*;
    use shape_value::{HeapKind, NativeKind};

    fn store() -> (tempfile::TempDir, SnapshotStore) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let st = SnapshotStore::new(tmp.path()).expect("snapshot store");
        (tmp, st)
    }

    /// A3: the four live-resource arms read as clean-refuse-by-design.
    #[test]
    fn clean_refuse_by_design_arms_carry_design_wording() {
        let (_tmp, st) = store();
        let cases = [
            (SerializableVMValue::IteratorOpaque, HeapKind::Iterator),
            (SerializableVMValue::DequeOpaque { len: 0 }, HeapKind::Deque),
            (
                SerializableVMValue::ChannelOpaque {
                    closed: false,
                    len: 0,
                },
                HeapKind::Channel,
            ),
            (SerializableVMValue::FilterExprOpaque, HeapKind::FilterExpr),
        ];
        for (sv, hk) in cases {
            let err = serializable_to_slot(&sv, NativeKind::Ptr(hk), &st)
                .expect_err("live-resource arm must surface-and-stop");
            assert!(
                err.contains("clean-refuse by design"),
                "{hk:?} should read clean-refuse-by-design, got: {err}"
            );
            assert!(
                !err.contains("follow-up"),
                "{hk:?} is terminal, not a follow-up, got: {err}"
            );
        }
    }

    /// A3: Mutex / Lazy stay on the follow-up wording (their dispositions
    /// belong to other workstreams). Reference / SharedCell migrated to
    /// STAGE-R5 serialize-through; their ctx-free path surfaces the
    /// two-pass-required message (see `reference_arms_require_ctx_driver`).
    #[test]
    fn deferred_arms_keep_followup_wording() {
        let (_tmp, st) = store();
        let cases = [
            (
                SerializableVMValue::MutexOpaque { has_value: false },
                HeapKind::Mutex,
            ),
            (
                SerializableVMValue::LazyOpaque {
                    is_initialized: false,
                },
                HeapKind::Lazy,
            ),
        ];
        for (sv, hk) in cases {
            let err = serializable_to_slot(&sv, NativeKind::Ptr(hk), &st)
                .expect_err("deferred arm must surface-and-stop");
            assert!(
                err.contains("follow-up"),
                "{hk:?} should keep follow-up wording, got: {err}"
            );
            assert!(
                !err.contains("clean-refuse by design"),
                "{hk:?} must not be relabeled clean-refuse, got: {err}"
            );
        }
    }

    /// STAGE-R5: the Reference / SharedCell serialize-through arms surface
    /// cleanly when reached via the ctx-free `serializable_to_slot` path —
    /// they require the two-pass `RestoreLinkCtx` driver. No fabrication,
    /// no wild-free.
    #[test]
    fn reference_arms_require_ctx_driver() {
        let (_tmp, st) = store();
        let cases = [
            (
                SerializableVMValue::Reference {
                    handle: 0,
                    is_mut: false,
                },
                HeapKind::Reference,
            ),
            (
                SerializableVMValue::SharedCellRef { handle: 0 },
                HeapKind::SharedCell,
            ),
        ];
        for (sv, hk) in cases {
            let err = serializable_to_slot(&sv, NativeKind::Ptr(hk), &st)
                .expect_err("ctx-free serialize-through arm must surface");
            assert!(
                err.contains("STAGE-R5") && err.contains("RestoreLinkCtx"),
                "{hk:?} should require the two-pass driver, got: {err}"
            );
        }
    }

    // ── STAGE-R5 serialize-through round-trip (ADR-006 §2.7.30.5) ────────
    use shape_value::reference::RefTarget;
    use shape_value::v2::closure_layout::SharedCell;

    /// Build slot bits for a `Ptr(HeapKind::SharedCell)` slot owning one
    /// share on `cell`.
    fn shared_cell_slot(cell: &Arc<SharedCell>) -> u64 {
        Arc::into_raw(Arc::clone(cell)) as u64
    }

    /// Build slot bits for a `Ptr(HeapKind::Reference)` slot holding a
    /// `RefTarget::PromotedCell` that owns one share on `cell`.
    fn promoted_ref_slot(cell: &Arc<SharedCell>) -> u64 {
        let rt = RefTarget::PromotedCell {
            cell: Arc::clone(cell),
            kind: cell.kind(),
        };
        Arc::into_raw(Arc::new(rt)) as u64
    }

    /// Drop a `Ptr(HeapKind::Reference)` slot's owned share.
    fn drop_ref_slot(bits: u64) {
        unsafe { Arc::decrement_strong_count(bits as *const RefTarget) };
    }

    /// Drop a `Ptr(HeapKind::SharedCell)` slot's owned share.
    fn drop_cell_slot(bits: u64) {
        unsafe { Arc::decrement_strong_count(bits as *const SharedCell) };
    }

    /// A LIVE promoted return-ref + its referent SharedCell round-trip:
    /// the stack reference is serialized FIRST (emits the BODY), the module
    /// binding cell SECOND (emits the back-edge). On restore BOTH dedupe to
    /// ONE cell; the restored reference reads the correct value.
    #[test]
    fn promoted_ref_and_referent_dedupe_to_one_cell() {
        let (_tmp, st) = store();
        // The referent cell holds int 99.
        let cell = Arc::new(SharedCell::new(99u64, NativeKind::Int64));
        let ref_bits = promoted_ref_slot(&cell);
        let cell_bits = shared_cell_slot(&cell);

        // SERIALIZE — one shared ctx, reference slot FIRST (the asymmetry
        // case the round-3 design fixes).
        let mut ictx = SerializeIdentityCtx::new();
        let sv_ref = serialize_reference(ref_bits, &st, &mut ictx).unwrap();
        let sv_cell = serialize_shared_cell(cell_bits, &st, &mut ictx).unwrap();

        // First arm (reference) emitted the BODY; second arm (cell) the
        // back-edge — same handle.
        match (&sv_ref, &sv_cell) {
            (
                SerializableVMValue::SharedCell { handle: h1, inner },
                SerializableVMValue::SharedCellRef { handle: h2 },
            ) => {
                assert_eq!(h1, h2, "both carriers share one handle");
                assert!(
                    matches!(**inner, SerializableVMValue::Int(99)),
                    "body inner = Int(99), got {inner:?}"
                );
            }
            other => panic!("expected body+back-edge, got {other:?}"),
        }

        // RESTORE — two-pass with the real per-slot kinds.
        let mut link = RestoreLinkCtx::new();
        materialize_cell_bodies(&sv_ref, &st, &mut link).unwrap();
        materialize_cell_bodies(&sv_cell, &st, &mut link).unwrap();
        let (r_ref_bits, r_ref_kind) = serializable_to_slot_ctx(
            &sv_ref,
            NativeKind::Ptr(HeapKind::Reference),
            &st,
            &mut link,
        )
        .unwrap();
        let (r_cell_bits, r_cell_kind) = serializable_to_slot_ctx(
            &sv_cell,
            NativeKind::Ptr(HeapKind::SharedCell),
            &st,
            &mut link,
        )
        .unwrap();
        assert_eq!(r_ref_kind, NativeKind::Ptr(HeapKind::Reference));
        assert_eq!(r_cell_kind, NativeKind::Ptr(HeapKind::SharedCell));

        // Recover the restored reference's cell + the restored cell slot;
        // assert they alias ONE allocation (dedupe) and read value 99.
        unsafe {
            let rt = Arc::<RefTarget>::from_raw(r_ref_bits as *const RefTarget);
            let restored_cell = Arc::<SharedCell>::from_raw(r_cell_bits as *const SharedCell);
            match &*rt {
                RefTarget::PromotedCell {
                    cell: ref_cell,
                    kind,
                } => {
                    assert_eq!(*kind, NativeKind::Int64);
                    assert_eq!(
                        Arc::as_ptr(ref_cell),
                        Arc::as_ptr(&restored_cell),
                        "reference and module-binding dedupe to ONE restored cell"
                    );
                    let guard = ref_cell.lock();
                    assert_eq!(*guard, 99, "restored ref reads the correct value");
                }
                other => panic!("expected PromotedCell, got {other:?}"),
            }
            // Balance the restored slot shares.
            drop(rt);
            drop(restored_cell);
        }
        // base scaffolding share + finish.
        link.release_base_shares();
        // Balance the original serialize-side slot shares.
        drop_ref_slot(ref_bits);
        drop_cell_slot(cell_bits);
    }

    /// Module-scope `let r = &x` shape: the SharedCell binding is reached
    /// FIRST (emits body), the reference SECOND (emits back-edge). Mirror
    /// ordering of the test above — both orders must round-trip.
    #[test]
    fn shared_cell_first_then_reference_roundtrips() {
        let (_tmp, st) = store();
        let cell = Arc::new(SharedCell::new(7u64, NativeKind::Int64));
        let cell_bits = shared_cell_slot(&cell);
        let ref_bits = promoted_ref_slot(&cell);

        let mut ictx = SerializeIdentityCtx::new();
        let sv_cell = serialize_shared_cell(cell_bits, &st, &mut ictx).unwrap();
        let sv_ref = serialize_reference(ref_bits, &st, &mut ictx).unwrap();

        match (&sv_cell, &sv_ref) {
            (
                SerializableVMValue::SharedCell { handle: h1, .. },
                SerializableVMValue::Reference { handle: h2, .. },
            ) => assert_eq!(h1, h2),
            other => panic!("expected cell-body + ref-back-edge, got {other:?}"),
        }

        let mut link = RestoreLinkCtx::new();
        materialize_cell_bodies(&sv_cell, &st, &mut link).unwrap();
        materialize_cell_bodies(&sv_ref, &st, &mut link).unwrap();
        let (r_cell_bits, _) = serializable_to_slot_ctx(
            &sv_cell,
            NativeKind::Ptr(HeapKind::SharedCell),
            &st,
            &mut link,
        )
        .unwrap();
        let (r_ref_bits, _) = serializable_to_slot_ctx(
            &sv_ref,
            NativeKind::Ptr(HeapKind::Reference),
            &st,
            &mut link,
        )
        .unwrap();
        unsafe {
            let rt = Arc::<RefTarget>::from_raw(r_ref_bits as *const RefTarget);
            let restored_cell = Arc::<SharedCell>::from_raw(r_cell_bits as *const SharedCell);
            if let RefTarget::PromotedCell { cell: ref_cell, .. } = &*rt {
                assert_eq!(Arc::as_ptr(ref_cell), Arc::as_ptr(&restored_cell));
                assert_eq!(*ref_cell.lock(), 7);
            } else {
                panic!("expected PromotedCell");
            }
            drop(rt);
            drop(restored_cell);
        }
        link.release_base_shares();
        drop_cell_slot(cell_bits);
        drop_ref_slot(ref_bits);
    }

    /// KL-4 guard: a non-promoted reference (Local / ModuleBinding /
    /// TypedField) in a snapshot CLEAN-REFUSES on serialize — reading its
    /// bits as `*const SharedCell` would be a wild-free.
    #[test]
    fn non_promoted_reference_clean_refuses() {
        let (_tmp, st) = store();
        for rt in [
            RefTarget::Local {
                frame_index: 0,
                slot_index: 3,
                kind: NativeKind::Int64,
            },
            RefTarget::ModuleBinding {
                binding_idx: 1,
                kind: NativeKind::Int64,
            },
        ] {
            let bits = Arc::into_raw(Arc::new(rt)) as u64;
            let mut ictx = SerializeIdentityCtx::new();
            let err = serialize_reference(bits, &st, &mut ictx)
                .expect_err("non-promoted ref must clean-refuse");
            assert!(
                err.contains("KL-4 guard") && err.contains("wild-free"),
                "expected KL-4 wild-free refusal, got: {err}"
            );
            unsafe { Arc::decrement_strong_count(bits as *const RefTarget) };
        }
    }

    /// The abort-ledger balances on an injected mid-link failure: after a
    /// Pass-2 error and `release_base_shares`, the cell's strong count
    /// returns to the caller's single original share (no leak, no
    /// double-free → no SIGABRT at drop).
    #[test]
    fn abort_ledger_balances_on_midlink_failure() {
        let (_tmp, st) = store();
        let cell = Arc::new(SharedCell::new(5u64, NativeKind::Int64));

        // A body emitted by a reference slot.
        let ref_bits = promoted_ref_slot(&cell);
        // Capture the baseline AFTER the serialize-side slot exists, so the
        // abort must return to exactly this count.
        let start = Arc::strong_count(&cell);
        let mut ictx = SerializeIdentityCtx::new();
        let sv_ref = serialize_reference(ref_bits, &st, &mut ictx).unwrap();

        let mut link = RestoreLinkCtx::new();
        materialize_cell_bodies(&sv_ref, &st, &mut link).unwrap();
        // Pass-2 link succeeds for the ref slot...
        let (r_ref_bits, _) = serializable_to_slot_ctx(
            &sv_ref,
            NativeKind::Ptr(HeapKind::Reference),
            &st,
            &mut link,
        )
        .unwrap();
        // ...then a SUBSEQUENT slot fails (inject a back-edge with an
        // unknown handle).
        let bad = SerializableVMValue::SharedCellRef { handle: 9999 };
        let res =
            serializable_to_slot_ctx(&bad, NativeKind::Ptr(HeapKind::SharedCell), &st, &mut link);
        assert!(res.is_err(), "unknown-handle back-edge must surface");

        // ABORT: release the slot we already built + the base shares.
        drop_ref_slot(r_ref_bits);
        link.release_base_shares();

        // The cell is back to exactly the original share count.
        assert_eq!(
            Arc::strong_count(&cell),
            start,
            "abort-ledger balanced: no leaked / double-freed share"
        );
        drop_ref_slot(ref_bits);
    }

    /// A SharedCell whose interior is a `Ptr(HeapKind::Reference)` cycle
    /// back into itself is detected via the in_progress VISITED-SET (NOT a
    /// depth bound) and cleanly surface-refused; the abort-ledger balances
    /// every retained share (no leaked Arc strong-count cycle that would
    /// break §2.7.30.4 deferred-Drop).
    #[test]
    fn self_referential_cell_cycle_surface_refuses() {
        let (_tmp, st) = store();
        // Wire shape: cell handle 0 whose value is a reference back to
        // handle 0 (a self-cycle). This is the topology a runtime Arc cycle
        // (cell A → ref → cell A) serializes to.
        let cyclic_body = SerializableVMValue::SharedCell {
            handle: 0,
            inner: Box::new(SerializableVMValue::Reference {
                handle: 0,
                is_mut: false,
            }),
        };
        let mut link = RestoreLinkCtx::new();
        let err = materialize_cell_bodies(&cyclic_body, &st, &mut link)
            .expect_err("self-referential cell cycle must surface-refuse");
        assert!(
            err.contains("cycle surface") && err.contains("VISITED-SET"),
            "expected in_progress cycle refusal, got: {err}"
        );
        // The ledger holds no completed base share for the aborted body
        // (the body never finished materializing), so release is a clean
        // no-op — no leaked strong-count.
        let before = link.ledger_len();
        link.release_base_shares();
        assert_eq!(
            before, 0,
            "no base share was committed for the cycle-aborted body"
        );
    }
}

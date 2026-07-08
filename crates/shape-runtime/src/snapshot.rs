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

// ── WF-2G GAP A: native/stdlib ModuleFn qualified-name resolver ──
//
// A `Ptr(HeapKind::ModuleFn)` slot's bits are a module-fn id — a
// process-local index into `VirtualMachine::module_fn_table`, whose entries
// are native Rust `Arc<dyn Fn>` bodies (there is NO content hash and the
// bodies are NOT wire-transferred; every node re-registers the identical
// stdlib deterministically). The sound cross-process identity is therefore
// the qualified export name `module::export` (e.g. `std::core::json::stringify`),
// carried by `SerializableVMValue::ModuleFunction(String)`.
//
// The projection (`slot_heap_to_serializable`) and its restore inverse
// (`serializable_to_heap_slot`) are shape-runtime free functions that cannot
// reach the VM's `module_fn_table`. The id↔name mapping is threaded to them
// through a thread-local install-once table — the same ambient-resolver shape
// as `type_schema::current_registry()`. `VirtualMachine::populate_module_objects`
// installs it after registering every module-fn (id order == registration
// order == deterministic across nodes). This is NOT a tag/kind bridge and
// carries no ValueWord shape — it is a plain `id → "module::export"` map.
thread_local! {
    /// `module_fn_id` (Vec index) → qualified export name `module::export`.
    static MODULE_FN_NAME_BY_ID: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// qualified export name `module::export` → `module_fn_id`.
    static MODULE_FN_ID_BY_NAME: std::cell::RefCell<HashMap<String, u64>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Install the module-fn qualified-name table for snapshot projection +
/// restore on the current thread. `names[id]` is the qualified name of the
/// module-fn registered with `module_fn_id == id`. Called by
/// `VirtualMachine::populate_module_objects` on both the snapshotting host
/// (projection: id → name) and the resuming host (restore: name → id).
pub fn install_module_fn_name_table(names: Vec<String>) {
    let mut by_name: HashMap<String, u64> = HashMap::with_capacity(names.len());
    for (id, name) in names.iter().enumerate() {
        // Duplicate names would be a stdlib-registration bug; last-writer
        // wins deterministically (registration order is fixed).
        by_name.insert(name.clone(), id as u64);
    }
    MODULE_FN_ID_BY_NAME.with(|m| *m.borrow_mut() = by_name);
    MODULE_FN_NAME_BY_ID.with(|m| *m.borrow_mut() = names);
}

/// Projection: resolve a module-fn id to its qualified export name.
/// `None` when the name table was never installed or the id is out of range
/// (surface-and-stop — never fabricate a name).
fn resolve_module_fn_name(id: u64) -> Option<String> {
    MODULE_FN_NAME_BY_ID.with(|m| m.borrow().get(id as usize).cloned())
}

/// Restore: resolve a qualified export name back to its module-fn id on the
/// resuming host. `None` when the module is absent here (clean-refuse — never
/// fabricate an id).
fn resolve_module_fn_id(name: &str) -> Option<u64> {
    MODULE_FN_ID_BY_NAME.with(|m| m.borrow().get(name).copied())
}

/// Schema version for the snapshot binary format.
///
/// This version is embedded in every [`ExecutionSnapshot`] via the `version`
/// field. Readers should check this value to determine whether they can
/// decode a snapshot or need migration logic.
///
/// Version history:
/// - v5: ValueWord-native serialization — `nanboxed_to_serializable`
///   and `serializable_to_nanboxed` operate on ValueWord directly without
///   intermediate ValueWord conversion. Format is wire-compatible with v4
///   (same `SerializableVMValue` enum), so v4 snapshots deserialize
///   correctly without migration.
/// - v6 (current, WF-2B snapshot-resume Stage 0/1): per-frame
///   `SerializableCallFrame.upvalue_kinds` wire field (retires the
///   no-layout Bool-default fabrication at `executor/snapshot.rs`, ADR-006
///   §2.7.8/Q10 — restore reads the recorded per-upvalue `NativeKind`
///   instead of guessing) + `ExecutionSnapshot.code_manifest` / `label`
///   envelope fields (CodeManifest blob-graph persistence, design §4.3).
///   The bincode wire encoding is non-self-describing, so this is a hard
///   version bump: older snapshots refuse cleanly, never Bool-default.
/// - v7 (current, GC Phase 5 — real-gc-cycle-collection.md §0 #4 /
///   §6): the snapshot identity-map is GENERALIZED from
///   `SharedCell`/`Reference` to EVERY cycle-capable `HeapKind`
///   (TypedObject, heap-element TypedArray, TypedObject-valued HashMap)
///   via the new `SerializableVMValue::HeapNode { handle, body }` +
///   `HeapRef { handle }` wire arms. The FIRST slot to reach a node's
///   allocation ptr emits `HeapNode` (the body); every later reach emits
///   `HeapRef` — breaking object/array/map reference cycles (which
///   previously INFINITE-RECURSED the structural serializer) and deduping
///   shared identity (which previously DUPLICATED the node on resume). The
///   new interned-body / back-reference variants change the non-self-
///   describing bincode layout, so this is a hard version bump: a v6
///   snapshot version-REFUSES cleanly at the load guard
///   (`SnapshotStore::get_snapshot`), never misparses, never Bool-defaults.
pub const SNAPSHOT_VERSION: u32 = 7;

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
        let snapshot: ExecutionSnapshot = bincode::deserialize(&decompressed)?;
        // GC Phase 5 (v6→v7): explicit version-equality guard. The bincode
        // wire encoding is non-self-describing, so a v6 blob whose
        // referenced `VmSnapshot` uses the pre-generalization layout must be
        // caught HERE — before the `VmSnapshot`/`ContextSnapshot` sub-objects
        // are trusted — rather than misparsed against the v7 `HeapNode` /
        // `HeapRef` arms. The `ExecutionSnapshot` envelope's `version` field
        // is stable across the bump (its own layout is unchanged), so it
        // deserializes cleanly and we refuse on the value, never Bool-default.
        if snapshot.version != SNAPSHOT_VERSION {
            anyhow::bail!(
                "unsupported snapshot version {} (this build reads version {}). \
                 The snapshot wire format changed at v6→v7 (GC Phase 5 identity-map \
                 generalization); older snapshots are refused cleanly rather than \
                 misparsed. Re-capture the snapshot with a matching build.",
                snapshot.version,
                SNAPSHOT_VERSION,
            );
        }
        Ok(snapshot)
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

    /// Resolve a user-supplied hash (full or short prefix) to a stored
    /// snapshot's full [`HashDigest`].
    ///
    /// `shape snapshot list` prints a truncated 16-char hash; feeding that
    /// straight to `--resume` used to build a store path that does not exist
    /// and surfaced as a cryptic `No such file or directory (os error 2)`.
    /// This resolves an exact match first, then falls back to a unique
    /// prefix scan (git-style), and otherwise returns a clean, actionable
    /// error naming the prefix — never a raw I/O error.
    pub fn resolve_hash(&self, prefix: &str) -> Result<HashDigest> {
        let normalized = prefix.strip_prefix("sha256:").unwrap_or(prefix);
        // Exact match first (full 64-char hash).
        let exact = HashDigest::from_hex(normalized);
        if self.snapshot_path(&exact).exists() {
            return Ok(exact);
        }
        // Unique-prefix match against the stored envelopes.
        let matches: Vec<HashDigest> = self
            .list_snapshots()?
            .into_iter()
            .map(|(h, _)| h)
            .filter(|h| h.hex().starts_with(normalized))
            .collect();
        match matches.len() {
            0 => anyhow::bail!(
                "no snapshot found matching '{}'. Run 'shape snapshot list' to see available snapshots.",
                prefix
            ),
            1 => Ok(matches.into_iter().next().unwrap()),
            n => anyhow::bail!(
                "'{}' is ambiguous - it matches {} snapshots. Use more characters of the hash.",
                prefix,
                n
            ),
        }
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
    /// Transitional monolithic-program twin. Written alongside
    /// [`code_manifest`](Self::code_manifest) through the staged migration
    /// (design §4.3.1 / §4.3.3) so same-node resume lands before the
    /// blob-graph load path is complete. Dropped at the cross-node close.
    pub bytecode_hash: Option<HashDigest>,
    /// Content hash of the [`CodeManifest`] object (design §4.3.2 / Q16):
    /// the authoritative code reference (per-`FunctionBlob` content hashes +
    /// permission union). `Some` from Stage 1 onward; `None` on legacy
    /// snapshots that predate the manifest.
    #[serde(default)]
    pub code_manifest: Option<HashDigest>,
    /// Path of the script that was executing when the snapshot was taken
    #[serde(default)]
    pub script_path: Option<String>,
    /// Reserved for snapshot-management tooling (`shape snapshot list`,
    /// design §4.12.2). Landed at the Stage-0/1 bump so list/inspect need
    /// no later format change.
    #[serde(default)]
    pub label: Option<String>,
}

/// Content-addressed code manifest referenced by an [`ExecutionSnapshot`]
/// (design §4.3.2, Q16 blob-graph persistence).
///
/// The manifest names the exact per-`FunctionBlob` content hashes a resume
/// must verify and fetch, instead of pinning a single monolithic program
/// blob. This is what lets snapshots of the same program dedup blobs, lets a
/// remote node fetch code per-function, and lets permission re-verification
/// happen at blob granularity. Portable by construction: every field is a
/// fixed-width content hash or a permission name — no host pointers, no
/// program-relative ids.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeManifest {
    /// Content hash of this manifest's own (sorted) blob list — the
    /// program identity a resume checks first.
    pub program_root_hash: [u8; 32],
    /// Sorted `FunctionBlob` content hashes — the transitive closure of the
    /// code the snapshot references.
    pub blobs: Vec<[u8; 32]>,
    /// Blob containing the function that `VmSnapshot.ip` belongs to, when
    /// known (content-addressed programs). `None` for non-content-addressed
    /// programs where the monolithic `bytecode_hash` twin is authoritative.
    #[serde(default)]
    pub entry: Option<[u8; 32]>,
    /// Linker union of the required permissions (permission names), recorded
    /// so resume re-verifies `union ⊆ granted` before any bytecode executes
    /// (design §4.7.3). Independently checkable because each blob's content
    /// hash already covers its permission names.
    #[serde(default)]
    pub required_permissions: Vec<String>,
}

impl CodeManifest {
    /// Build a manifest from a set of `FunctionBlob` content hashes, the
    /// entry-function hash, and the linker permission union. Sorts the blob
    /// list and derives `program_root_hash` from it so the same program
    /// always produces the same root hash.
    pub fn from_blobs(
        mut blobs: Vec<[u8; 32]>,
        entry: Option<[u8; 32]>,
        mut required_permissions: Vec<String>,
    ) -> Self {
        blobs.sort_unstable();
        blobs.dedup();
        required_permissions.sort_unstable();
        required_permissions.dedup();
        let mut root_input = Vec::with_capacity(blobs.len() * 32);
        for h in &blobs {
            root_input.extend_from_slice(h);
        }
        let program_root_hash = crate::hashing::hash_bytes_to_array(&root_input);
        Self {
            program_root_hash,
            blobs,
            entry,
            required_permissions,
        }
    }
}

/// Engine-owned envelope halves the dispatch loop cannot reach on its own
/// (design §4.3.4). Installed on the VM by the host at program load, beside
/// the snapshot store, so the in-loop suspension consumer can persist a
/// complete envelope without re-entering engine state.
#[derive(Debug, Clone)]
pub struct SnapshotEnvelopeSeed {
    /// Content hash of the (already-persisted) [`SemanticSnapshot`]. The host
    /// writes the `SemanticSnapshot` object once at load and records its hash
    /// here — `exported_symbols` are fixed after load.
    pub semantic_hash: HashDigest,
    /// Path of the executing script, for envelope metadata / tooling.
    pub script_path: Option<String>,
}

/// Persist a captured VM snapshot as a complete content-addressed envelope
/// (design §4.3.4). Free function so the dispatch shell can call it with what
/// the loop already has (Constraint 8 keeps it off the JIT ABI).
///
/// Write ordering follows §4.3.5: content-addressed sub-objects first
/// (idempotent — a content-addressed put is a no-op if present), the
/// `ExecutionSnapshot` envelope last, so a crash never yields a
/// referenced-but-missing object.
#[allow(clippy::too_many_arguments)]
pub fn persist_execution_state(
    store: &SnapshotStore,
    seed: &SnapshotEnvelopeSeed,
    ctx: Option<&crate::context::ExecutionContext>,
    vm_snapshot: &VmSnapshot,
    manifest: &CodeManifest,
    bytecode_hash: Option<HashDigest>,
    label: Option<String>,
) -> Result<HashDigest> {
    // 1. Context envelope half. `None` (embedded paths without a persistent
    //    context) is the NoPersistentContext barrier at the call site; here
    //    we require a context to snapshot.
    let context = match ctx {
        Some(ctx) => ctx.snapshot(store)?,
        None => {
            return Err(anyhow::anyhow!(
                "persist_execution_state: no execution context to snapshot"
            ));
        }
    };
    let context_hash = store.put_struct(&context)?;

    // 2. Content-addressed sub-objects (idempotent puts).
    let code_manifest_hash = store.put_struct(manifest)?;
    let vm_hash = store.put_struct(vm_snapshot)?;

    // 3. Envelope last (atomic entry point).
    let snapshot = ExecutionSnapshot {
        version: SNAPSHOT_VERSION,
        created_at_ms: chrono::Utc::now().timestamp_millis(),
        semantic_hash: seed.semantic_hash.clone(),
        context_hash,
        vm_hash: Some(vm_hash),
        bytecode_hash,
        code_manifest: Some(code_manifest_hash),
        script_path: seed.script_path.clone(),
        label,
    };
    let snapshot_hash = store.put_snapshot(&snapshot)?;
    Ok(snapshot_hash)
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
    /// WF-3F snapshot origin flag (design §4.4 / §4.5.1 step 4). `true` when
    /// this snapshot was captured by the Ctrl+C interrupt-save path, whose
    /// `ip` is a rewound un-executed instruction that expects a PRISTINE
    /// operand stack. `false` (default, incl. all pre-WF-3F snapshots) marks a
    /// `snapshot()`-call origin, whose `ip` is the post-call site that
    /// CONSUMES `snapshot()`'s `Ok(Snapshot::Resumed)` return value. Resume
    /// pushes the resume marker ONLY when this is `false`; pushing it for an
    /// interrupt-origin snapshot shifts the operand stack by one slot and
    /// corrupts the pending call (the release-blocking silent-corruption bug).
    #[serde(default)]
    pub interrupt_saved: bool,
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
    /// Per-upvalue `NativeKind`, recorded at capture (design §4.2.4, ADR-006
    /// §2.7.8/Q10). Retires the no-layout Bool-default fabrication: when a
    /// frame's closure carries no layout side-table, restore reads the
    /// recorded kind here instead of guessing `Bool`. `None` only when the
    /// frame has no upvalues; a no-layout frame WITH upvalues always records
    /// this so restore never fabricates a kind.
    #[serde(default)]
    pub upvalue_kinds: Option<Vec<shape_value::NativeKind>>,
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

    /// Legacy snapshot arm for the old typed-Arc `ResultData` carrier.
    /// Restore normalizes this arm to the schema-backed `__Result`
    /// `TypedObjectStorage` carrier; snapshot bytes must not recreate a
    /// live `Arc<ResultData>` path.
    ResultData {
        is_ok: bool,
        payload: Box<SerializableVMValue>,
    },

    /// Legacy snapshot arm for the old typed-Arc `OptionData` carrier.
    /// Restore normalizes this arm to the schema-backed `__Option`
    /// `TypedObjectStorage` carrier; snapshot bytes must not recreate a
    /// live `Arc<OptionData>` path.
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

    /// GC Phase 5 (snapshot v7, real-gc-cycle-collection.md §0 #4 / §6):
    /// a cycle-capable heap NODE tagged with an identity `handle`.
    ///
    /// `body` is the node's ordinary serialized shape — `TypedObject`,
    /// `Array` (heap-element TypedArray), or `HashMap` (TypedObject-valued).
    /// The FIRST slot to reach the node's allocation ptr during the shared
    /// [`SerializeIdentityCtx`] walk emits this BODY; every later reach emits
    /// a [`SerializableVMValue::HeapRef`] back-reference carrying the same
    /// `handle`. This is what breaks an object/array/map reference CYCLE (a
    /// self-linked `type Node { var next: Node? }` previously infinite-
    /// recursed the serializer) and DEDUPS a shared node (two carriers
    /// previously produced two copies on resume, losing identity).
    ///
    /// On restore, Pass 1 ([`materialize_cell_bodies`]) materializes each
    /// `HeapNode` into EXACTLY ONE heap allocation per handle (recorded in
    /// the restore identity-map with a base share on the abort-ledger);
    /// forward children and back-references resolve to that one allocation
    /// via the per-`HeapKind` retain primitive. This generalizes the
    /// `SharedCell`/`Reference` identity machinery — it does NOT replace it.
    HeapNode {
        handle: u64,
        body: Box<SerializableVMValue>,
    },

    /// GC Phase 5 (snapshot v7): a back-reference to a previously-emitted
    /// [`SerializableVMValue::HeapNode`].
    ///
    /// Emitted whenever the serialize walk re-reaches a node's allocation
    /// ptr already interned in [`SerializeIdentityCtx`] — an ancestor (the
    /// cycle case) or a completed sibling subtree (the dedup case). On
    /// restore it resolves against the identity-map (materialized in Pass 1)
    /// and hands out one additional retained share for the referencing slot.
    /// It carries no kind — restore resolves the node's `NativeKind` from the
    /// identity-map entry recorded at materialization, never fabricated from
    /// bits (ADR-006 §2.7.7).
    HeapRef {
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
    /// GC Phase 5 (v7): handle → materialized cycle-capable heap NODE, as
    /// `(allocation ptr as u64, node NativeKind)`. Populated by Pass-1
    /// `materialize_cell_bodies` for every `SV::HeapNode` (TypedObject /
    /// heap-element TypedArray / TypedObject-valued HashMap); resolved by
    /// forward children, `SV::HeapRef` back-references, and Pass-2 top-level
    /// slots to the ONE allocation per handle. The kind is the identity-map
    /// entry's canonical `NativeKind::Ptr(HeapKind::*)` — recorded at
    /// materialization, never fabricated from bits (no parallel discriminator:
    /// this is the same `(bits, kind)` pair the slot ABI uses, ADR-006
    /// §2.7.7). Separate from `identity_map` so the `SharedCell`/`Reference`
    /// path is untouched (handles are unique across the shared counter).
    heap_node_map: std::collections::HashMap<u64, (u64, NativeKind)>,
    /// Pass-1 cycle guard: handles whose body is mid-materialization.
    in_progress: std::collections::HashSet<u64>,
    /// Abort-ledger: every BASE materialization share handed out, as
    /// `(allocation ptr, node NativeKind)`, in claim order. Reverse-walk
    /// (LIFO) to release on abort OR restore-finish. Generalized (GC Phase 5,
    /// v7) from `SharedCell`-only to every cycle-capable heap NODE carrier —
    /// the per-`HeapKind` release primitive is selected by dispatching on the
    /// recorded `NativeKind` (ADR-005 §1 single-discriminator: no parallel
    /// ledger sum-type projecting 1:1 to `HeapKind`; the canonical slot-ABI
    /// kind IS the discriminator).
    retained: Vec<(u64, NativeKind)>,
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
        use shape_value::heap_value::{HashMapKindedRef, TypedObjectStorage};
        use shape_value::v2::closure_layout::SharedCell;
        use shape_value::v2::heap_element::HeapElement;
        use shape_value::v2::typed_array::release_v2_typed_array;
        // LIFO reverse-walk: children were materialized (and their bases
        // pushed) AFTER their parents, so releasing bottom-up means a
        // parent's memory-decrement never dereferences an already-freed
        // child. Each release is a single decrement-and-maybe-free
        // (`release_elem` / `release_v2_typed_array` / `Arc::decrement`) —
        // never an unconditional free — so a node still referenced by a real
        // (Pass-2-installed) slot survives; only the surplus scaffolding
        // share is retired. Balances on both abort and success (§2.7.30.5).
        while let Some((ptr, kind)) = self.retained.pop() {
            match kind {
                NativeKind::Ptr(HeapKind::SharedCell) => unsafe {
                    Arc::decrement_strong_count(ptr as *const SharedCell);
                },
                NativeKind::Ptr(HeapKind::TypedObject) => unsafe {
                    TypedObjectStorage::release_elem(ptr as *const TypedObjectStorage);
                },
                NativeKind::Ptr(HeapKind::TypedArray) => unsafe {
                    release_v2_typed_array(ptr as *mut u8);
                },
                NativeKind::Ptr(HeapKind::HashMap) => unsafe {
                    Arc::decrement_strong_count(ptr as *const HashMapKindedRef);
                },
                other => debug_assert!(
                    false,
                    "release_base_shares: unexpected ledger kind {other:?} — only \
                     SharedCell / TypedObject / TypedArray / HashMap base shares \
                     are recorded"
                ),
            }
        }
        self.identity_map.clear();
        self.heap_node_map.clear();
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
/// Project a live `TypedObjectStorage` (read through a borrow — no share
/// taken) into `SV::TypedObject`. Shared by the direct `HeapKind::TypedObject`
/// slot arm and the `ELEM_TYPE_TYPED_OBJECT` heap-element-array element walk
/// (WF-2G GAP B). Each field recurses through `slot_to_serializable_ctx` on the
/// per-field parallel `field_kinds` track (ADR-006 §2.5 / §2.3 / §2.7.7 — no
/// kind fabricated from bits, no Bool-default).
/// GC Phase 5 (snapshot v7): intern a cycle-capable heap node's allocation
/// ptr into the shared identity ctx, emitting a [`SerializableVMValue::HeapNode`]
/// BODY on the first reach and a [`SerializableVMValue::HeapRef`] back-reference
/// on every later reach.
///
/// This is the general-`HeapKind` analogue of [`emit_or_backedge_cell`] (which
/// remains the dedicated `SharedCell`/`Reference` path). `node_ptr` is the
/// node's allocation address used ONLY as a `HashMap` key (never dereferenced
/// for the key role) — the same raw-provenance-pointer identity discipline as
/// the cell path. Insert-before-recurse: the handle is recorded in `handle_of`
/// BEFORE `build_body` recurses, so a payload that reaches back into the same
/// node (the cycle case) finds the handle already present and emits a back-edge
/// rather than recursing forever. NO `ValueWord` shape, NO tag/kind decode: the
/// handle is a plain `u64` counter and identity is the raw ptr alone.
fn intern_or_backedge(
    node_ptr: *const (),
    store: &SnapshotStore,
    ctx: &mut SerializeIdentityCtx,
    build_body: impl FnOnce(
        &SnapshotStore,
        &mut SerializeIdentityCtx,
    ) -> std::result::Result<SerializableVMValue, String>,
) -> std::result::Result<SerializableVMValue, String> {
    if let Some(&handle) = ctx.handle_of.get(&node_ptr) {
        // Already interned — an ancestor mid-recurse (cycle) or a completed
        // sibling subtree (dedup). Emit the back-reference; do NOT recurse.
        return Ok(SerializableVMValue::HeapRef { handle });
    }
    let handle = ctx.next_handle;
    ctx.next_handle += 1;
    ctx.handle_of.insert(node_ptr, handle);
    let body = build_body(store, ctx)?;
    Ok(SerializableVMValue::HeapNode {
        handle,
        body: Box::new(body),
    })
}

fn typed_object_storage_to_serializable(
    storage: &shape_value::heap_value::TypedObjectStorage,
    store: &SnapshotStore,
    ctx: &mut SerializeIdentityCtx,
) -> std::result::Result<SerializableVMValue, String> {
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
    Ok(SerializableVMValue::TypedObject {
        schema_id,
        slot_data,
        heap_mask,
    })
}

fn slot_heap_to_serializable(
    bits: u64,
    expected_kind: HeapKind,
    store: &SnapshotStore,
    ctx: &mut SerializeIdentityCtx,
) -> std::result::Result<SerializableVMValue, String> {
    use SerializableVMValue as SV;
    use shape_value::heap_value::{
        AtomicData, HashSetData, LazyData, MutexData, OptionData, PriorityQueueData, ResultData,
    };
    // WF-2G GAP A: `Ptr(HeapKind::ModuleFn)` bits are an inline-scalar
    // module-fn id (NOT a pointer), and id 0 is the first-registered fn —
    // a valid value, not a null pointer. Exempt it from the null-pointer
    // guard so it reaches the ModuleFn projection arm below.
    if bits == 0 && !matches!(expected_kind, HeapKind::ModuleFn) {
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
            use shape_value::heap_value::HashSetElementKind;
            let arc = Arc::<HashSetData>::from_raw(bits as *const HashSetData);
            let serializable = match arc.element_kind() {
                HashSetElementKind::String => {
                    let keys: Vec<String> =
                        arc.string_keys().iter().map(|k| (**k).clone()).collect();
                    Ok(SV::HashSet { keys })
                }
                HashSetElementKind::I64 => Err(
                    "slot_to_serializable: HashSet<int> snapshot is not yet represented"
                        .to_string(),
                ),
            };
            let _ = Arc::into_raw(arc);
            serializable
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
        // CLEAN-REFUSE at snapshot() time (user-ruled disposition,
        // 2026-05-29). A live channel/queue buffer is an in-process
        // resource whose contents cannot be honestly serialized; the
        // earliest honest refuse point is capture, not resume. Refuse
        // here (naming the type) instead of silently dropping the payload
        // into a discriminator-only opaque wire shape. No Arc is touched
        // — the slot retains its share and drops on its own path.
        HeapKind::Channel => Err(
            "snapshot cannot capture a live Channel: an in-process channel \
             buffer (queued values + closed flag) is a live resource, not \
             snapshot-restorable — clean-refuse by design (ADR-006 §2.7.5.1)"
                .to_string(),
        ),
        HeapKind::Deque => Err(
            "snapshot cannot capture a live Deque: an in-process double-ended \
             queue buffer is a live resource, not snapshot-restorable — \
             clean-refuse by design (ADR-006 §2.7.5.1)"
                .to_string(),
        ),
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
        // CLEAN-REFUSE at snapshot() time (user-ruled disposition,
        // 2026-05-29) — same rationale as Channel/Deque above.
        HeapKind::FilterExpr => Err(
            "snapshot cannot capture a live FilterExpr: a query-DSL filter \
             node (And/Or/Not predicate tree) is a live in-process resource, \
             not snapshot-restorable — clean-refuse by design (ADR-006 §2.7.5.1)"
                .to_string(),
        ),
        HeapKind::SharedCell => serialize_shared_cell(bits, store, ctx),
        HeapKind::Iterator => Err(
            "snapshot cannot capture a live Iterator: an in-flight iterator \
             cursor is a live in-process resource, not snapshot-restorable — \
             clean-refuse by design (ADR-006 §2.7.5.1)"
                .to_string(),
        ),
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
            //
            // GC Phase 5 (v7): intern the storage ptr for identity so a
            // TypedObject reached from ≥2 slots (or a `var`-field cycle back
            // into itself) emits ONE `HeapNode` body + `HeapRef` back-edges
            // instead of duplicating / infinite-recursing (§0 #4 / §6).
            intern_or_backedge(bits as *const (), store, ctx, move |store, ctx| {
                // SAFETY: per the slot construction contract the bits point
                // to a live `TypedObjectStorage`; the borrow is valid for the
                // duration of the field reads (the slot keeps its share).
                let storage: &shape_value::heap_value::TypedObjectStorage =
                    unsafe { &*(bits as *const shape_value::heap_value::TypedObjectStorage) };
                typed_object_storage_to_serializable(storage, store, ctx)
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
                ELEM_TYPE_BOOL, ELEM_TYPE_DECIMAL, ELEM_TYPE_F32, ELEM_TYPE_F64, ELEM_TYPE_I8,
                ELEM_TYPE_I16, ELEM_TYPE_I32, ELEM_TYPE_I64, ELEM_TYPE_STRING,
                ELEM_TYPE_TYPED_OBJECT, ELEM_TYPE_U8, ELEM_TYPE_U16, ELEM_TYPE_U32, TypedArray,
                read_elem_type,
            };
            let ptr = bits as *const u8;
            // SAFETY: the slot construction contract guarantees a live,
            // element-type-stamped TypedArray carrier at `bits`.
            let elem = unsafe { read_elem_type(ptr) };
            // GC Phase 5 (v7): a TypedObject-element array is the only
            // cycle-capable TypedArray shape (its elements can be shared /
            // reference back into a containing object). Intern the array ptr
            // for identity AND each element storage ptr, so a shared/cyclic
            // element emits ONE body + back-edges rather than duplicating /
            // infinite-recursing (§0 #4 / §6). Scalar + String/Decimal-element
            // arrays hold only leaves — no identity needed; they keep the
            // pre-v7 un-wrapped `SV::Array` wire shape below.
            if elem == ELEM_TYPE_TYPED_OBJECT {
                return intern_or_backedge(ptr as *const (), store, ctx, move |store, ctx| {
                    use shape_value::heap_value::TypedObjectStorage;
                    // SAFETY: element-type stamp is TYPED_OBJECT ⇒ the buffer
                    // holds `*const TypedObjectStorage` owning pointers.
                    let slice = unsafe {
                        TypedArray::<*const TypedObjectStorage>::as_slice(
                            ptr as *const TypedArray<*const TypedObjectStorage>,
                        )
                    };
                    let mut out: Vec<SerializableVMValue> = Vec::with_capacity(slice.len());
                    for &p in slice.iter() {
                        // Intern each element TypedObject: a node shared with
                        // another element (or a containing object) dedupes to
                        // one handle; a self-referential element cycle-breaks.
                        let sv = intern_or_backedge(
                            p as *const (),
                            store,
                            ctx,
                            move |store, ctx| {
                                // SAFETY: `p` is a live element storage owned by
                                // the array; borrow-read, take no share.
                                let storage: &TypedObjectStorage = unsafe { &*p };
                                typed_object_storage_to_serializable(storage, store, ctx)
                            },
                        )?;
                        out.push(sv);
                    }
                    Ok(SV::Array(out))
                });
            }
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
                    // ── WF-2G GAP B: heap-element arrays ───────────────────
                    // The element buffer holds owning heap pointers
                    // (`*const StringObj` / `*const DecimalObj` /
                    // `*const TypedObjectStorage` per typed_array.rs drop
                    // dispatch). We read each element THROUGH the buffer
                    // borrow (the slot keeps its share; no per-element
                    // ownership taken) and project via the same typed
                    // carriers as `marshal.rs` / `json_value.rs`. No
                    // Bool-default, no raw-bits reinterpretation — the
                    // element type is the producer-side `_pad` stamp.
                    ELEM_TYPE_STRING => {
                        use shape_value::v2::string_obj::StringObj;
                        TypedArray::<*const StringObj>::as_slice(
                            ptr as *const TypedArray<*const StringObj>,
                        )
                        .iter()
                        .map(|&p| SV::String(StringObj::as_str(p).to_owned()))
                        .collect()
                    }
                    ELEM_TYPE_DECIMAL => {
                        use shape_value::v2::decimal_obj::DecimalObj;
                        TypedArray::<*const DecimalObj>::as_slice(
                            ptr as *const TypedArray<*const DecimalObj>,
                        )
                        .iter()
                        .map(|&p| SV::Decimal(DecimalObj::value(p)))
                        .collect()
                    }
                    // ELEM_TYPE_TYPED_OBJECT is handled by the interned
                    // early-return above (GC Phase 5, v7) and never reaches
                    // this scalar/leaf match.
                    other_elem => {
                        return Err(format!(
                            "slot_to_serializable: W17-snapshot-roundtrip surface — \
                             TypedArray element-type discriminant {other_elem} is \
                             not in the round-trip set (scalar kinds + heap-element \
                             String / Decimal / TypedObject). Nested-array / \
                             trait-object / callable element carriers land in \
                             follow-up. ADR-006 §2.7.5.1."
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
                // GC Phase 5 (v7): a `HashMap<string, TypedObject>` can hold
                // shared/cyclic nodes. Intern the map ptr for identity and
                // route each value TypedObject through the shared ctx so a
                // node shared across two keys (or cyclic) dedupes / cycle-
                // breaks. Keys are strings (§2.7.15 string-only keyspace);
                // values are `HeapNode`/`HeapRef`. The map is emitted as a
                // `HeapNode` body wrapping `SV::HashMap { keys, values }`.
                HashMapKindedRef::TypedObject(map_arc) => {
                    use shape_value::heap_value::{TypedObjectPtr, TypedObjectStorage};
                    use shape_value::v2::string_obj::StringObj;
                    use shape_value::v2::typed_array::TypedArray;
                    intern_or_backedge(bits as *const (), store, ctx, move |store, ctx| {
                        let n = map_arc.len();
                        let mut keys: Vec<SerializableVMValue> = Vec::with_capacity(n);
                        let mut values: Vec<SerializableVMValue> = Vec::with_capacity(n);
                        for i in 0..n {
                            // SAFETY: keys buffer holds `*const StringObj`;
                            // values buffer holds `TypedObjectPtr` — both live
                            // for `map_arc`'s lifetime. Borrow-read the value
                            // ptr (no share taken; `TypedObjectPtr` has a Drop,
                            // so we take `&TypedObjectPtr` and read `.as_ptr()`
                            // rather than moving it).
                            let (kstr, vptr) = unsafe {
                                let kp = TypedArray::get_unchecked(map_arc.keys, i as u32);
                                let vref: &TypedObjectPtr = &*(*map_arc.values).data.add(i);
                                (StringObj::as_str(kp).to_owned(), vref.as_ptr())
                            };
                            keys.push(SV::String(kstr));
                            let vsv = intern_or_backedge(
                                vptr as *const (),
                                store,
                                ctx,
                                move |store, ctx| {
                                    // SAFETY: `vptr` is a live element storage
                                    // owned by the map; borrow-read, take none.
                                    let storage: &TypedObjectStorage = unsafe { &*vptr };
                                    typed_object_storage_to_serializable(storage, store, ctx)
                                },
                            )?;
                            values.push(vsv);
                        }
                        Ok(SV::HashMap { keys, values })
                    })
                }
                other_v => Err(format!(
                    "slot_to_serializable: W17-snapshot-roundtrip surface — \
                     HashMap value-monomorphization {} is K3 (the heap-value \
                     kinded-track amendment); only HashMap<string,string> \
                     and HashMap<string,TypedObject> round-trip at this scope. \
                     ADR-006 §2.7.5.1.",
                    hashmap_kinded_ref_arm_name(other_v),
                )),
            }
        }

        // WF-2G GAP A: native/stdlib module-function value. The slot bits
        // are the module-fn id (an inline scalar index into the VM's
        // `module_fn_table` — NOT an `Arc<T>` pointer; clone/drop are
        // no-ops per kinded_slot.rs:1148). Project to the qualified export
        // name so the snapshot is self-contained across processes: native
        // stdlib fns have no content hash and are re-registered identically
        // on every node, so `module::export` is the sound identity carried
        // by `SerializableVMValue::ModuleFunction(String)`. Surface-and-stop
        // (never fabricate) if the id has no registered name.
        HeapKind::ModuleFn => match resolve_module_fn_name(bits) {
            Some(name) => Ok(SV::ModuleFunction(name)),
            None => Err(format!(
                "slot_to_serializable: ModuleFn slot id {bits} has no registered \
                 qualified name (module-fn name table not installed on this \
                 thread, or id out of range). Surface-and-stop — never \
                 fabricate a name. ADR-006 §2.7.5.1."
            )),
        },

        // Pre-existing complex shapes: surface-and-stop per §2.7.5.1.
        // These have rich pre-bulldozer SerializableVMValue arms whose
        // construction requires more than typed-Arc recovery (DataTable /
        // TableView / Temporal / TaskGroup / IoHandle / NativeView /
        // NativeScalar / Content / ClosureRaw each have their own
        // multi-step landing path).
        unsupported @ (HeapKind::Closure
        | HeapKind::DataTable
        | HeapKind::TaskGroup
        | HeapKind::Temporal
        | HeapKind::TableView
        | HeapKind::Content
        | HeapKind::Instant
        | HeapKind::IoHandle
        | HeapKind::NativeScalar
        | HeapKind::NativeView
        | HeapKind::TraitObject
        | HeapKind::Matrix
        | HeapKind::MatrixSlice) => Err(format!(
            "slot_to_serializable: W17-snapshot-roundtrip surface — \
             HeapKind::{unsupported:?} arm has no in-session SerializableVMValue \
             projection. Tracked as W17-snapshot-{other:?} follow-up per \
             docs/cluster-audits/phase-2d-playbook.md §3. \
             ADR-006 §2.7.5.1.",
            other = unsupported,
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
            ctx.retained
                .push((ptr, NativeKind::Ptr(HeapKind::SharedCell)));
            ctx.in_progress.remove(handle);
            Ok(())
        }
        // GC Phase 5 (v7): a cycle-capable heap NODE. Materialize it into
        // exactly ONE allocation per handle (recorded in `heap_node_map` with
        // a base share on the abort-ledger); forward children + back-edges
        // resolve to it. Idempotent on a repeat handle (dedup across slots).
        SV::HeapNode { handle, body } => {
            if ctx.heap_node_map.contains_key(handle) {
                return Ok(());
            }
            materialize_node_base(*handle, body, store, ctx)
        }
        // A back-reference materializes nothing — it resolves in
        // `resolve_child` / Pass-2 against the already-materialized node.
        SV::HeapRef { .. } => Ok(()),
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

/// GC Phase 5 (v7): materialize a single `SV::HeapNode` body into exactly one
/// heap allocation per handle, recording `handle → (ptr, kind)` in
/// `ctx.heap_node_map` and pushing the base materialization share onto the
/// abort-ledger. Dispatches on the body shape (TypedObject / heap-element
/// Array / TypedObject-valued HashMap). This is the general-`HeapKind`
/// analogue of the `SV::SharedCell` body materialization in
/// [`materialize_cell_bodies`].
///
/// The node is recorded in `heap_node_map` BEFORE its children are filled
/// (for TypedObject / TypedArray, via record-before-fill), so a child that
/// references back into this node (the CYCLE case) resolves to the one
/// allocation instead of duplicating or infinite-recursing. Every child edge
/// is materialized through [`resolve_child`], which hands the parent slot ONE
/// retained share; a genuine cycle therefore round-trips as a real cycle
/// (later reclaimed by the GC), preserving identity exactly (§0 #4 / §6).
fn materialize_node_base(
    handle: u64,
    body: &SerializableVMValue,
    store: &SnapshotStore,
    ctx: &mut RestoreLinkCtx,
) -> std::result::Result<(), String> {
    use SerializableVMValue as SV;
    match body {
        SV::TypedObject {
            schema_id,
            slot_data,
            heap_mask,
        } => materialize_typed_object_node(handle, *schema_id, slot_data, *heap_mask, store, ctx),
        SV::Array(elems) => materialize_typed_object_array_node(handle, elems, store, ctx),
        SV::HashMap { keys, values } => {
            materialize_typed_object_hashmap_node(handle, keys, values, store, ctx)
        }
        other => Err(format!(
            "materialize_node_base: GC-Phase-5 surface — HeapNode body arm {} \
             is not a cycle-capable node shape (expected TypedObject / Array / \
             HashMap). Malformed v7 wire shape. ADR-006 §2.7.5.1 / §2.7.30.5.",
            serializable_arm_name(other),
        )),
    }
}

/// GC Phase 5 (v7): materialize a `HeapNode{TypedObject}` with record-before-
/// fill so a `var`-field cycle back into the object round-trips.
///
/// Allocates a SHELL (all slots placeholder-zero, `field_kinds` all `Null`,
/// `heap_mask = 0` so the shell is safe to drop before it is filled), records
/// `handle → (ptr, Ptr(TypedObject))` and the base share, then fills each
/// field via [`resolve_child`] (a self-reference resolves to this very ptr).
/// After all fields are filled the real per-field `NativeKind` track and the
/// (wire) `heap_mask` are installed so `Drop` releases exactly the heap fields.
fn materialize_typed_object_node(
    handle: u64,
    schema_id: u64,
    slot_data: &[SerializableVMValue],
    heap_mask: u64,
    store: &SnapshotStore,
    ctx: &mut RestoreLinkCtx,
) -> std::result::Result<(), String> {
    use shape_value::heap_value::TypedObjectStorage;
    use shape_value::{NativeKind, ValueSlot};
    let n = slot_data.len();
    // Shell: placeholder slots + Null field_kinds + heap_mask 0 (drop-safe).
    let placeholder_slots: Box<[ValueSlot]> =
        (0..n).map(|_| ValueSlot::from_raw(0)).collect::<Vec<_>>().into_boxed_slice();
    let placeholder_kinds: Arc<[NativeKind]> =
        (0..n).map(|_| NativeKind::Null).collect::<Vec<_>>().into();
    let ptr = TypedObjectStorage::_new(schema_id, placeholder_slots, 0, placeholder_kinds);
    // Record identity + base BEFORE filling, so a self-referential field
    // resolves to this ptr (the cycle case).
    ctx.heap_node_map
        .insert(handle, (ptr as u64, NativeKind::Ptr(HeapKind::TypedObject)));
    ctx.retained
        .push((ptr as u64, NativeKind::Ptr(HeapKind::TypedObject)));
    // Fill fields, collecting the real per-field kinds.
    let mut real_kinds: Vec<NativeKind> = Vec::with_capacity(n);
    for (i, fsv) in slot_data.iter().enumerate() {
        let (fbits, fkind) = resolve_child(fsv, store, ctx).map_err(|msg| {
            format!("materialize_typed_object_node: field[{i}] (schema_id={schema_id}): {msg}")
        })?;
        // SAFETY: `ptr` is a live `_new`-allocated shell; `i` is in-bounds; the
        // single-word slot write goes through the raw interior-mutable cell
        // (no `&TypedObjectStorage` formed — see `write_slot_in_place`). The
        // prior placeholder bits (0) own no share, so we discard the return.
        let _prior = unsafe { TypedObjectStorage::write_slot_in_place(ptr, i, fbits) };
        real_kinds.push(fkind);
    }
    // Install the real field-kind track + heap_mask (from the wire, which was
    // read off the original storage). Raw place-writes: no `&mut Self` formed.
    // SAFETY: `ptr` live; the fields are POD-owning after fill; assigning
    // `field_kinds` drops the placeholder `Arc<[Null]>` exactly once.
    unsafe {
        *std::ptr::addr_of_mut!((*ptr).field_kinds) = real_kinds.into();
        *std::ptr::addr_of_mut!((*ptr).heap_mask) = heap_mask;
    }
    Ok(())
}

/// GC Phase 5 (v7): materialize a `HeapNode{Array}` as a heap-element
/// `TypedArray<*const TypedObjectStorage>` (`ELEM_TYPE_TYPED_OBJECT` — the
/// only cycle-capable array shape; scalar / String / Decimal arrays are not
/// interned). Record-before-push: the array ptr is recorded before elements
/// are pushed, so an element that references back into this array resolves to
/// the one allocation. Each element is materialized via [`resolve_child`].
fn materialize_typed_object_array_node(
    handle: u64,
    elems: &[SerializableVMValue],
    store: &SnapshotStore,
    ctx: &mut RestoreLinkCtx,
) -> std::result::Result<(), String> {
    use shape_value::NativeKind;
    use shape_value::heap_value::TypedObjectStorage;
    use shape_value::v2::typed_array::{ELEM_TYPE_TYPED_OBJECT, TypedArray, stamp_elem_type};
    let out = TypedArray::<*const TypedObjectStorage>::with_capacity(elems.len() as u32);
    // SAFETY: fresh carrier from this module's allocator; stamp the element
    // discriminant before any push / drop reads it.
    unsafe { stamp_elem_type(out as *mut u8, ELEM_TYPE_TYPED_OBJECT) };
    ctx.heap_node_map
        .insert(handle, (out as u64, NativeKind::Ptr(HeapKind::TypedArray)));
    ctx.retained
        .push((out as u64, NativeKind::Ptr(HeapKind::TypedArray)));
    for (i, esv) in elems.iter().enumerate() {
        let (ebits, ekind) = resolve_child(esv, store, ctx)
            .map_err(|msg| format!("materialize_typed_object_array_node: elem[{i}]: {msg}"))?;
        if ekind != NativeKind::Ptr(HeapKind::TypedObject) {
            return Err(format!(
                "materialize_typed_object_array_node: elem[{i}] resolved to {ekind:?}, \
                 expected Ptr(TypedObject) — a HeapNode-wrapped array is TypedObject-\
                 element only. Malformed v7 wire shape. ADR-006 §2.7.5.1."
            ));
        }
        // SAFETY: `out` is a live TYPED_OBJECT-stamped carrier; `ebits` is a
        // `*const TypedObjectStorage` owning one share (from `resolve_child`),
        // transferred into the array by `push`.
        unsafe {
            TypedArray::<*const TypedObjectStorage>::push(out, ebits as *const TypedObjectStorage);
        }
    }
    Ok(())
}

/// GC Phase 5 (v7): materialize a `HeapNode{HashMap}` as a
/// `HashMap<string, TypedObject>` (`HashMapKindedRef::TypedObject`). Values
/// are materialized (dedup / cycle-break) via [`resolve_child`]. Record-after:
/// the `Arc<HashMapKindedRef>` identity ptr only exists once built, so a cycle
/// that routes back THROUGH the map is not representable here (map values that
/// reference the map are out-of-scope, surfacing cleanly) — but a map holding
/// a shared or self-cyclic node round-trips (the node's identity is recorded
/// during its own materialization, before the map closes over it).
fn materialize_typed_object_hashmap_node(
    handle: u64,
    keys: &[SerializableVMValue],
    values: &[SerializableVMValue],
    store: &SnapshotStore,
    ctx: &mut RestoreLinkCtx,
) -> std::result::Result<(), String> {
    use SerializableVMValue as SV;
    use shape_value::NativeKind;
    use shape_value::heap_value::{HashMapData, HashMapKindedRef, TypedObjectPtr};
    if keys.len() != values.len() {
        return Err(format!(
            "materialize_typed_object_hashmap_node: keys/values length mismatch \
             (keys={}, values={}). Malformed v7 wire shape. ADR-006 §2.7.5.1.",
            keys.len(),
            values.len(),
        ));
    }
    // Build incrementally; on error the local `data` drops, releasing every
    // inserted value share (HashMapData<TypedObjectPtr> Drop) — no leak.
    let mut data: HashMapData<TypedObjectPtr> = HashMapData::new();
    for (k, v) in keys.iter().zip(values.iter()) {
        let key_str = match k {
            SV::String(s) => s,
            _ => {
                return Err(
                    "materialize_typed_object_hashmap_node: non-String key — the \
                     §2.7.15 keyspace is string-only. ADR-006 §2.7.5.1."
                        .to_string(),
                );
            }
        };
        let (vbits, vkind) = resolve_child(v, store, ctx)
            .map_err(|msg| format!("materialize_typed_object_hashmap_node: value: {msg}"))?;
        if vkind != NativeKind::Ptr(HeapKind::TypedObject) {
            // Release the just-materialized value share before surfacing.
            retain_release_one_node(vbits, vkind);
            return Err(format!(
                "materialize_typed_object_hashmap_node: value resolved to {vkind:?}, \
                 expected Ptr(TypedObject) — a HeapNode-wrapped map is TypedObject-\
                 valued only. ADR-006 §2.7.5.1."
            ));
        }
        // `insert` transfers the one value share into the map (the map's Drop
        // retires it). `TypedObjectPtr::new` takes ownership without a bump.
        unsafe {
            data.insert(
                key_str.as_str(),
                TypedObjectPtr::new(vbits as *const _),
            );
        }
    }
    let kref = Arc::new(HashMapKindedRef::TypedObject(Arc::new(data)));
    let ptr = Arc::into_raw(kref) as u64;
    ctx.heap_node_map
        .insert(handle, (ptr, NativeKind::Ptr(HeapKind::HashMap)));
    ctx.retained.push((ptr, NativeKind::Ptr(HeapKind::HashMap)));
    Ok(())
}

/// GC Phase 5 (v7): materialize a child edge (a TypedObject field, an array
/// element, or a map value), transferring ONE share to the caller's slot.
///
/// - `HeapNode`: if not yet materialized, materialize it (records identity +
///   base share); then bump one share for the caller. Handles forward
///   children (deep) and dedup (already-recorded → just bump).
/// - `HeapRef`: resolve the handle against `heap_node_map` (recorded earlier
///   in the sequential fill — a HeapRef only ever points at an already-visited
///   node) and bump one share. This is the cycle / dedup back-edge.
/// - leaf / scalar / string / SharedCell-family: delegate to the ctx-free
///   `serializable_to_slot` (fresh alloc owning one share). SharedCell /
///   Reference field values surface cleanly there (they are the dedicated
///   two-pass path, out of the generalized node scope).
fn resolve_child(
    sv: &SerializableVMValue,
    store: &SnapshotStore,
    ctx: &mut RestoreLinkCtx,
) -> std::result::Result<(u64, NativeKind), String> {
    use SerializableVMValue as SV;
    match sv {
        SV::HeapNode { handle, body } => {
            if !ctx.heap_node_map.contains_key(handle) {
                materialize_node_base(*handle, body, store, ctx)?;
            }
            let (ptr, kind) = *ctx.heap_node_map.get(handle).ok_or_else(|| {
                format!("resolve_child: HeapNode handle {handle} not materialized")
            })?;
            retain_one_node(ptr, kind);
            Ok((ptr, kind))
        }
        SV::HeapRef { handle } => {
            let (ptr, kind) = *ctx.heap_node_map.get(handle).ok_or_else(|| {
                format!(
                    "resolve_child: GC-Phase-5 surface — HeapRef handle {handle} has no \
                     materialized node (Pass-1 body missing or a cycle routes back \
                     through a record-after container, out of round-trip scope). \
                     ADR-006 §2.7.30.5."
                )
            })?;
            retain_one_node(ptr, kind);
            Ok((ptr, kind))
        }
        leaf => {
            let expected = expected_heap_field_kind(leaf);
            serializable_to_slot(leaf, expected, store)
        }
    }
}

/// GC Phase 5 (v7): bump ONE refcount share on a materialized heap node, via
/// the per-`HeapKind` retain primitive. Identity-map dispatch on the recorded
/// `NativeKind::Ptr(HeapKind::*)` — no `is_heap()` probe, no bits decode.
fn retain_one_node(ptr: u64, kind: NativeKind) {
    match kind {
        NativeKind::Ptr(HeapKind::TypedObject) => unsafe {
            shape_value::v2::refcount::v2_retain(ptr as *const shape_value::v2::heap_header::HeapHeader);
        },
        NativeKind::Ptr(HeapKind::TypedArray) => unsafe {
            shape_value::v2::typed_array::retain_v2_typed_array(ptr as *mut u8);
        },
        NativeKind::Ptr(HeapKind::HashMap) => unsafe {
            Arc::increment_strong_count(ptr as *const shape_value::heap_value::HashMapKindedRef);
        },
        _ => {
            debug_assert!(
                false,
                "retain_one_node: non-node kind {kind:?} — heap_node_map only \
                 records TypedObject / TypedArray / HashMap identities"
            );
        }
    }
}

/// GC Phase 5 (v7): retire ONE share on a materialized heap node (the error-
/// path inverse of [`retain_one_node`], decrement-and-maybe-free per kind).
fn retain_release_one_node(ptr: u64, kind: NativeKind) {
    use shape_value::heap_value::{HashMapKindedRef, TypedObjectStorage};
    use shape_value::v2::heap_element::HeapElement;
    use shape_value::v2::typed_array::release_v2_typed_array;
    match kind {
        NativeKind::Ptr(HeapKind::TypedObject) => unsafe {
            TypedObjectStorage::release_elem(ptr as *const TypedObjectStorage);
        },
        NativeKind::Ptr(HeapKind::TypedArray) => unsafe {
            release_v2_typed_array(ptr as *mut u8);
        },
        NativeKind::Ptr(HeapKind::HashMap) => unsafe {
            Arc::decrement_strong_count(ptr as *const HashMapKindedRef);
        },
        _ => {}
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
        // GC Phase 5 (v7): a top-level slot holding a cycle-capable node (body
        // or back-edge) resolves to the ONE Pass-1-materialized allocation and
        // takes one owned share. `expected_kind` is ignored — the identity-map
        // entry carries the authoritative recorded `NativeKind` (never
        // fabricated from bits). Pass 1 (`materialize_cell_bodies`) already
        // ran over every top-level slot, so the handle is present.
        (SV::HeapNode { handle, .. }, _) | (SV::HeapRef { handle }, _) => {
            let (ptr, kind) = *ctx.heap_node_map.get(handle).ok_or_else(|| {
                format!(
                    "serializable_to_slot_ctx: GC-Phase-5 surface — heap node handle \
                     {handle} has no Pass-1 materialization. ADR-006 §2.7.30.5."
                )
            })?;
            retain_one_node(ptr, kind);
            Ok((ptr, kind))
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

/// Derive the `expected_kind` for [`serializable_to_slot`] from a
/// [`SerializableVMValue`] discriminator alone (design §4.2 / ADR-006
/// §2.7.5.1: the SV variant is the authoritative carrier of the slot's kind).
///
/// Used by restore paths that do NOT persist a parallel kind track next to the
/// value (the `ExecutionContext` variable scopes carry a `VarKind`, not a
/// `NativeKind`). For carrier-ambiguous BODY arms (`SV::SharedCell`) the
/// caller must instead thread the real per-slot kind via
/// [`serializable_to_slot_ctx`]; this returns the cell carrier as the
/// standalone default. Complex/unmappable arms return `NativeKind::Bool`, which
/// makes [`serializable_to_slot`] surface a clean projection error rather than
/// silently Bool-defaulting a real heap value (Constraint 3).
pub fn expected_kind_from_serializable(sv: &SerializableVMValue) -> NativeKind {
    use SerializableVMValue as SV;
    match sv {
        SV::Int(_) => NativeKind::Int64,
        SV::Number(_) => NativeKind::Float64,
        SV::Bool(_) => NativeKind::Bool,
        SV::String(_) => NativeKind::String,
        SV::None | SV::Unit => NativeKind::Null,
        SV::Decimal(_) => NativeKind::Ptr(HeapKind::Decimal),
        SV::BigInt(_) => NativeKind::Ptr(HeapKind::BigInt),
        SV::Char(_) => NativeKind::Ptr(HeapKind::Char),
        SV::HashSet { .. } => NativeKind::Ptr(HeapKind::HashSet),
        SV::PriorityQueueHeap { .. } => NativeKind::Ptr(HeapKind::PriorityQueue),
        SV::AtomicI64 { .. } => NativeKind::Ptr(HeapKind::Atomic),
        SV::ResultData { .. } | SV::OptionData { .. } => NativeKind::Ptr(HeapKind::TypedObject),
        SV::IteratorOpaque => NativeKind::Ptr(HeapKind::Iterator),
        SV::DequeOpaque { .. } => NativeKind::Ptr(HeapKind::Deque),
        SV::ChannelOpaque { .. } => NativeKind::Ptr(HeapKind::Channel),
        SV::Reference { .. } => NativeKind::Ptr(HeapKind::Reference),
        SV::SharedCell { .. } => NativeKind::Ptr(HeapKind::SharedCell),
        SV::SharedCellRef { .. } => NativeKind::Ptr(HeapKind::SharedCell),
        SV::FilterExprOpaque => NativeKind::Ptr(HeapKind::FilterExpr),
        SV::MutexOpaque { .. } => NativeKind::Ptr(HeapKind::Mutex),
        SV::LazyOpaque { .. } => NativeKind::Ptr(HeapKind::Lazy),
        SV::TypedObject { .. } => NativeKind::Ptr(HeapKind::TypedObject),
        SV::Range { .. } => NativeKind::Ptr(HeapKind::Range),
        SV::HashMap { .. } => NativeKind::Ptr(HeapKind::HashMap),
        SV::Array(_) => NativeKind::Ptr(HeapKind::TypedArray),
        SV::ModuleFunction(_) => NativeKind::Ptr(HeapKind::ModuleFn),
        _ => NativeKind::Bool,
    }
}

pub fn serializable_to_slot(
    sv: &SerializableVMValue,
    expected_kind: NativeKind,
    store: &SnapshotStore,
) -> std::result::Result<(u64, NativeKind), String> {
    use SerializableVMValue as SV;
    // GC Phase 5 (v7): a standalone (ctx-free) single-value restore of a
    // cycle-capable node. The symmetric `slot_to_serializable` owns a fresh
    // `SerializeIdentityCtx`, so a lone TypedObject / heap-element array /
    // TypedObject-valued map is emitted HeapNode-wrapped (and a self-cyclic
    // value carries an internal HeapRef). Spin a matching local two-pass
    // `RestoreLinkCtx` so the value round-trips with its internal identity /
    // cycles intact — the same machinery the whole-VM driver threads across
    // slots, scoped here to one value. (Recursive field/element restores never
    // reach here as a HeapNode — `resolve_child` intercepts those under the
    // shared ctx, so this only fires at a genuine standalone top level.)
    if matches!(sv, SV::HeapNode { .. } | SV::HeapRef { .. }) {
        let mut ctx = RestoreLinkCtx::new();
        let result = (|| {
            materialize_cell_bodies(sv, store, &mut ctx)?;
            serializable_to_slot_ctx(sv, expected_kind, store, &mut ctx)
        })();
        // Base shares are scaffolding; the returned slot owns its own Pass-2
        // share. Release on both success and error (LIFO, balanced).
        ctx.release_base_shares();
        return result;
    }
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
        (SV::None | SV::Unit, NativeKind::Null) => Ok((0, NativeKind::Null)),
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
        SV::None | SV::Unit => NativeKind::Null,
        SV::Decimal(_) => NativeKind::Ptr(HeapKind::Decimal),
        SV::BigInt(_) => NativeKind::Ptr(HeapKind::BigInt),
        SV::Char(_) => NativeKind::Ptr(HeapKind::Char),
        SV::Range { .. } => NativeKind::Ptr(HeapKind::Range),
        SV::TypedObject { .. } => NativeKind::Ptr(HeapKind::TypedObject),
        SV::HashMap { .. } => NativeKind::Ptr(HeapKind::HashMap),
        SV::Array(_) => NativeKind::Ptr(HeapKind::TypedArray),
        SV::HashSet { .. } => NativeKind::Ptr(HeapKind::HashSet),
        // Legacy ResultData/OptionData snapshot arms normalize to the
        // canonical schema-backed `__Result` / `__Option` typed objects.
        // Nested typed-object fields must not silently restore old
        // Arc<ResultData> / Arc<OptionData> carriers.
        SV::ResultData { .. } | SV::OptionData { .. } => NativeKind::Ptr(HeapKind::TypedObject),
        // WF-2G GAP A: a module-fn field inside a module-binding TypedObject.
        SV::ModuleFunction(_) => NativeKind::Ptr(HeapKind::ModuleFn),
        // Pre-existing complex arms — surface clean rather than guess.
        _ => NativeKind::Bool,
    }
}

/// Inverse of [`slot_heap_to_serializable`] — reconstruct a heap-kinded
/// slot from its serialized arm. Returns `(bits, NativeKind)` ready
/// to push to a slot. The reconstructed slot owns one strong-count
/// share on the typed `Arc<T>` carrier.
/// Rebuild a `TypedObjectStorage` from a serialized `SV::TypedObject`, returning
/// the v2-raw carrier pointer as `u64` (refcount = 1 on the HeapHeader). Shared
/// by the direct `HeapKind::TypedObject` restore arm and the
/// `ELEM_TYPE_TYPED_OBJECT` heap-element-array rebuild (WF-2G GAP B).
///
/// Each field restores through `serializable_to_slot` (recursion); the returned
/// `(bits, kind)` populate the slot array and the parallel `field_kinds` track.
/// Allocation goes through the v2-raw `_new` carrier so the slot's release path
/// (`drop_with_kind` → `TypedObjectStorage::release_elem` → carrier-side `_drop`
/// + `std::alloc::dealloc`) matches the allocation — the legacy
/// `Arc::new(...)` + `Arc::into_raw` carrier would mismatch the allocator at
/// drop time (the `length_typed_object_empty` allocator-pair SIGABRT class per
/// the v2-raw-heap-audit). ADR-006 §2.3 / §2.5 amendment (Wave 2 Agent D1/D2).
fn sv_typed_object_to_ptr(
    schema_id: u64,
    slot_data: &[SerializableVMValue],
    heap_mask: u64,
    store: &SnapshotStore,
) -> std::result::Result<u64, String> {
    use shape_value::ValueSlot;
    let n = slot_data.len();
    let mut slots: Vec<ValueSlot> = Vec::with_capacity(n);
    let mut field_kinds: Vec<NativeKind> = Vec::with_capacity(n);
    for (i, fsv) in slot_data.iter().enumerate() {
        let expected = expected_heap_field_kind(fsv);
        let (fbits, fkind) = serializable_to_slot(fsv, expected, store).map_err(|msg| {
            format!("serializable_to_slot: TypedObject restore field[{i}] (schema_id={schema_id}): {msg}")
        })?;
        slots.push(ValueSlot::from_raw(fbits));
        field_kinds.push(fkind);
    }
    let field_kinds_arc: Arc<[NativeKind]> = field_kinds.into();
    let ptr = shape_value::heap_value::TypedObjectStorage::_new(
        schema_id,
        slots.into_boxed_slice(),
        heap_mask,
        field_kinds_arc,
    );
    Ok(ptr as u64)
}

fn serializable_to_heap_slot(
    sv: &SerializableVMValue,
    heap_kind: HeapKind,
    store: &SnapshotStore,
) -> std::result::Result<(u64, NativeKind), String> {
    use SerializableVMValue as SV;
    use shape_value::heap_value::{AtomicData, HashSetData, PriorityQueueData};
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
        (SV::ModuleFunction(name), HeapKind::ModuleFn) => {
            // WF-2G GAP A restore. Re-resolve the qualified export name back
            // to a module-fn id against the resuming host's registration
            // (installed by `populate_module_objects`; deterministic order ⇒
            // id parity with the origin). The id is an inline scalar (clone/
            // drop no-op), so no share is minted. Clean-refuse if the module
            // is absent on the resumer — never fabricate an id.
            match resolve_module_fn_id(name) {
                Some(id) => Ok((id, NativeKind::Ptr(HeapKind::ModuleFn))),
                None => Err(format!(
                    "serializable_to_slot: ModuleFunction '{name}' is not \
                     registered on the resuming host (module absent, or the \
                     module-fn name table was not installed before restore). \
                     Clean-refuse — never fabricate an id. ADR-006 §2.7.5.1."
                )),
            }
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
            // W88B compatibility normalization: a persisted old kind track
            // can still ask for HeapKind::Result, but restore must not
            // allocate a fresh Arc<ResultData>. Build the canonical
            // schema-backed `__Result` typed object and return its real
            // kind to the caller.
            let inner_slot = inner_kinded_from_serializable(payload)?;
            Ok(build_builtin_result_typed_object_slot(*is_ok, inner_slot))
        }
        (SV::OptionData { is_some, payload }, HeapKind::Option) => {
            // W88B compatibility normalization: a persisted old kind track
            // can still ask for HeapKind::Option, but restore must not
            // allocate a fresh Arc<OptionData>. Build the canonical
            // schema-backed `__Option` typed object and return its real
            // kind to the caller.
            let inner_slot = if *is_some {
                match payload {
                    Some(p) => inner_kinded_from_serializable(p)?,
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
                KindedSlot::none()
            };
            Ok(build_builtin_option_typed_object_slot(*is_some, inner_slot))
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
        (SV::ResultData { is_ok, payload }, HeapKind::TypedObject) => {
            let payload = inner_kinded_from_serializable(payload)?;
            Ok(build_builtin_result_typed_object_slot(*is_ok, payload))
        }
        (SV::OptionData { is_some, payload }, HeapKind::TypedObject) => {
            let payload = if *is_some {
                match payload {
                    Some(p) => inner_kinded_from_serializable(p)?,
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
                KindedSlot::none()
            };
            Ok(build_builtin_option_typed_object_slot(*is_some, payload))
        }
        (
            SV::TypedObject {
                schema_id,
                slot_data,
                heap_mask,
            },
            HeapKind::TypedObject,
        ) => {
            let ptr = sv_typed_object_to_ptr(*schema_id, slot_data, *heap_mask, store)?;
            Ok((ptr, NativeKind::Ptr(HeapKind::TypedObject)))
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
                // ── WF-2G GAP B: heap-element array restore ────────────────
                // Build the monomorphized heap-element carrier (mirror of
                // `marshal.rs::ToSlot for Vec<Arc<HeapValue>>`). Each element
                // constructor mints a fresh owning heap pointer (refcount = 1)
                // and `push` transfers that single share into the array — no
                // extra retain, no double-count. Homogeneity is pre-validated
                // BEFORE allocation so the error path never leaks a partial
                // array. ADR-006 §2.3 / §2.7.5.1.
                Some(SV::String(_)) => {
                    use shape_value::v2::string_obj::StringObj;
                    use shape_value::v2::typed_array::ELEM_TYPE_STRING;
                    // Pre-validate homogeneity (no allocation yet).
                    let mut strs: Vec<&str> = Vec::with_capacity(elems.len());
                    for e in elems {
                        match e {
                            SV::String(s) => strs.push(s.as_str()),
                            _ => {
                                return Err("serializable_to_slot: TypedArray restore — \
                                 heterogeneous element (expected all String). \
                                 ADR-006 §2.7.5.1."
                                    .to_string());
                            }
                        }
                    }
                    let out = TypedArray::<*const StringObj>::with_capacity(elems.len() as u32);
                    unsafe {
                        stamp_elem_type(out as *mut u8, ELEM_TYPE_STRING);
                        for s in strs {
                            // Fresh StringObj (refcount = 1); the array owns it.
                            let p = StringObj::new(s) as *const StringObj;
                            TypedArray::<*const StringObj>::push(out, p);
                        }
                    }
                    out as usize as u64
                }
                Some(SV::Decimal(_)) => {
                    use shape_value::v2::decimal_obj::DecimalObj;
                    use shape_value::v2::typed_array::ELEM_TYPE_DECIMAL;
                    let mut decs: Vec<rust_decimal::Decimal> = Vec::with_capacity(elems.len());
                    for e in elems {
                        match e {
                            SV::Decimal(d) => decs.push(*d),
                            _ => {
                                return Err("serializable_to_slot: TypedArray restore — \
                                 heterogeneous element (expected all Decimal). \
                                 ADR-006 §2.7.5.1."
                                    .to_string());
                            }
                        }
                    }
                    let out = TypedArray::<*const DecimalObj>::with_capacity(elems.len() as u32);
                    unsafe {
                        stamp_elem_type(out as *mut u8, ELEM_TYPE_DECIMAL);
                        for d in decs {
                            // Fresh DecimalObj (refcount = 1); the array owns it.
                            let p = DecimalObj::new(d) as *const DecimalObj;
                            TypedArray::<*const DecimalObj>::push(out, p);
                        }
                    }
                    out as usize as u64
                }
                Some(SV::TypedObject { .. }) => {
                    use shape_value::heap_value::TypedObjectStorage;
                    use shape_value::v2::typed_array::{
                        ELEM_TYPE_TYPED_OBJECT, release_v2_typed_array,
                    };
                    // Build the carrier up-front, stamped, then push each
                    // element storage (refcount = 1 from `sv_typed_object_to_ptr`
                    // — the array takes that single share; no retain, no
                    // double-count). If any field restore fails mid-way, drop
                    // the whole partially-built array via `release_v2_typed_array`
                    // (walks + retires every pushed element share) before
                    // surfacing — balanced accounting, no leak. ADR-006 §2.3.
                    let out =
                        TypedArray::<*const TypedObjectStorage>::with_capacity(elems.len() as u32);
                    unsafe { stamp_elem_type(out as *mut u8, ELEM_TYPE_TYPED_OBJECT) };
                    for e in elems {
                        let build = match e {
                            SV::TypedObject {
                                schema_id,
                                slot_data,
                                heap_mask,
                            } => sv_typed_object_to_ptr(*schema_id, slot_data, *heap_mask, store),
                            _ => Err("serializable_to_slot: TypedArray restore — \
                                 heterogeneous element (expected all TypedObject). \
                                 ADR-006 §2.7.5.1."
                                .to_string()),
                        };
                        match build {
                            Ok(bits) => unsafe {
                                TypedArray::<*const TypedObjectStorage>::push(
                                    out,
                                    bits as *const TypedObjectStorage,
                                );
                            },
                            Err(msg) => {
                                unsafe { release_v2_typed_array(out as *mut u8) };
                                return Err(format!(
                                    "serializable_to_slot: TypedArray<TypedObject> \
                                     restore element: {msg}"
                                ));
                            }
                        }
                    }
                    out as usize as u64
                }
                Some(other) => {
                    return Err(format!(
                        "serializable_to_slot: TypedArray restore — element arm \
                         {} is not in the round-trip set (scalar Int / Number / \
                         Bool + heap-element String / Decimal / TypedObject). \
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

        // DEFINED-RESET arms (user-ruled disposition, 2026-05-29). These
        // wrap a value/initializer that the discriminator-only wire shape
        // does not carry; rather than refuse, they resume to a defined,
        // deterministic reset state with NO stale payload:
        //   • Mutex → unlocked / empty (holds the canonical Null absence
        //     sentinel; single-threaded landing is always unlocked).
        //   • Lazy  → unforced / uninitialized (no initializer, no cached
        //     value); the next `lazy.get()` surfaces cleanly rather than
        //     returning a wrong/empty value.
        // Fresh `Arc::new(...)` + `Arc::into_raw` matches the Mutex/Lazy
        // Clone/Drop carrier (`Arc::{increment,decrement}_strong_count`),
        // so the restored slot owns exactly one share.
        (SV::MutexOpaque { .. }, HeapKind::Mutex) => {
            let m = Arc::new(shape_value::heap_value::MutexData::new(
                shape_value::kinded_slot::KindedSlot::none(),
            ));
            let raw = Arc::into_raw(m) as u64;
            Ok((raw, NativeKind::Ptr(HeapKind::Mutex)))
        }
        (SV::LazyOpaque { .. }, HeapKind::Lazy) => {
            let l = Arc::new(shape_value::heap_value::LazyData::uninitialized());
            let raw = Arc::into_raw(l) as u64;
            Ok((raw, NativeKind::Ptr(HeapKind::Lazy)))
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
/// Number→Float64, Unit/None→Null).
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
        SV::Unit | SV::None => Ok(KindedSlot::none()),
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
        K::Callable(_) => "Callable",
        K::HashMap(_) => "HashMap",
    }
}

fn build_builtin_result_typed_object_slot(is_ok: bool, payload: KindedSlot) -> (u64, NativeKind) {
    use crate::type_schema::builtin_schemas::{RESULT_VARIANT_ERR, RESULT_VARIANT_OK};
    let (_registry, schemas) =
        crate::type_schema::TypeSchemaRegistry::with_stdlib_types_and_builtin_ids();
    build_builtin_variant_typed_object_slot(
        schemas.result as u64,
        if is_ok {
            RESULT_VARIANT_OK
        } else {
            RESULT_VARIANT_ERR
        },
        payload,
    )
}

fn build_builtin_option_typed_object_slot(is_some: bool, payload: KindedSlot) -> (u64, NativeKind) {
    use crate::type_schema::builtin_schemas::{OPTION_VARIANT_NONE, OPTION_VARIANT_SOME};
    let (_registry, schemas) =
        crate::type_schema::TypeSchemaRegistry::with_stdlib_types_and_builtin_ids();
    build_builtin_variant_typed_object_slot(
        schemas.option as u64,
        if is_some {
            OPTION_VARIANT_SOME
        } else {
            OPTION_VARIANT_NONE
        },
        payload,
    )
}

fn build_builtin_variant_typed_object_slot(
    schema_id: u64,
    variant: i64,
    payload: KindedSlot,
) -> (u64, NativeKind) {
    use crate::type_schema::builtin_schemas::OPTION_PAYLOAD;
    use shape_value::TypedObjectStorage;

    let payload_slot = payload.slot();
    let payload_kind = payload.kind();
    let payload_bits = payload_slot.raw();
    let heap_mask = if payload_bits != 0 && snapshot_field_is_heap_like(payload_kind) {
        1u64 << OPTION_PAYLOAD
    } else {
        0
    };
    let field_kinds: Arc<[NativeKind]> =
        Arc::from(vec![NativeKind::Int64, payload_kind].into_boxed_slice());
    let ptr = TypedObjectStorage::_new(
        schema_id,
        vec![ValueSlot::from_int(variant), payload_slot].into_boxed_slice(),
        heap_mask,
        field_kinds,
    );
    std::mem::forget(payload);
    (ptr as u64, NativeKind::Ptr(HeapKind::TypedObject))
}

fn snapshot_field_is_heap_like(kind: NativeKind) -> bool {
    matches!(
        kind,
        NativeKind::String | NativeKind::StringV2 | NativeKind::DecimalV2 | NativeKind::Ptr(_)
    )
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
        SV::HeapNode { .. } => "HeapNode",
        SV::HeapRef { .. } => "HeapRef",
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
mod l5_typed_object_result_option_snapshot_tests {
    use super::{
        SerializableVMValue as SV, SnapshotStore, serializable_to_slot, slot_to_serializable,
    };
    use crate::type_schema::TypeSchemaRegistry;
    use crate::type_schema::builtin_schemas::{
        OPTION_PAYLOAD, OPTION_VARIANT_NONE, OPTION_VARIANT_SOME, RESULT_PAYLOAD,
        RESULT_VARIANT_ERR, RESULT_VARIANT_OK,
    };
    use shape_value::{HeapKind, KindedSlot, NativeKind, TypedObjectStorage, ValueSlot};
    use std::sync::Arc;

    fn variant_object(schema_id: u64, variant: i64, payload: KindedSlot) -> KindedSlot {
        let payload_slot = payload.slot();
        let payload_kind = payload.kind();
        let payload_bits = payload_slot.raw();
        let heap_mask = if payload_bits != 0
            && matches!(
                payload_kind,
                NativeKind::String
                    | NativeKind::StringV2
                    | NativeKind::DecimalV2
                    | NativeKind::Ptr(_)
            ) {
            1u64 << OPTION_PAYLOAD
        } else {
            0
        };
        let field_kinds: Arc<[NativeKind]> =
            Arc::from(vec![NativeKind::Int64, payload_kind].into_boxed_slice());
        let ptr = TypedObjectStorage::_new(
            schema_id,
            vec![ValueSlot::from_int(variant), payload_slot].into_boxed_slice(),
            heap_mask,
            field_kinds,
        );
        std::mem::forget(payload);
        KindedSlot::from_typed_object_raw(ptr)
    }

    fn restored_storage(bits: u64, kind: NativeKind) -> KindedSlot {
        assert_eq!(kind, NativeKind::Ptr(HeapKind::TypedObject));
        KindedSlot::new(ValueSlot::from_raw(bits), kind)
    }

    fn storage(slot: &KindedSlot) -> &TypedObjectStorage {
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        unsafe { &*(slot.raw() as *const TypedObjectStorage) }
    }

    #[test]
    fn schema_backed_option_none_snapshot_restore_preserves_null_payload_kind() {
        let (_registry, schemas) = TypeSchemaRegistry::with_stdlib_types_and_builtin_ids();
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");
        let none = variant_object(
            schemas.option as u64,
            OPTION_VARIANT_NONE,
            KindedSlot::none(),
        );

        let sv = slot_to_serializable(none.raw(), none.kind(), &store).expect("serialize none");
        // GC Phase 5 (v7): TypedObjects are identity-interned, so the snapshot
        // is a `HeapNode` body wrapping `SV::TypedObject`. Unwrap for the shape
        // assertion; the round-trip (via `serializable_to_slot`, which spins a
        // local two-pass for a standalone HeapNode) is unchanged.
        let body = match &sv {
            SV::HeapNode { body, .. } => &**body,
            other => panic!("expected HeapNode(TypedObject), got {other:?}"),
        };
        match body {
            SV::TypedObject {
                schema_id,
                slot_data,
                ..
            } => {
                assert_eq!(*schema_id, schemas.option as u64);
                assert!(matches!(slot_data[0], SV::Int(OPTION_VARIANT_NONE)));
                assert!(matches!(slot_data[OPTION_PAYLOAD], SV::None));
            }
            other => panic!("expected typed-object snapshot, got {other:?}"),
        }

        let (bits, kind) =
            serializable_to_slot(&sv, NativeKind::Ptr(HeapKind::TypedObject), &store)
                .expect("restore none typed object");
        let restored = restored_storage(bits, kind);
        let restored_storage = storage(&restored);
        assert_eq!(restored_storage.schema_id, schemas.option as u64);
        assert_eq!(
            restored_storage.field_kinds[OPTION_PAYLOAD],
            NativeKind::Null
        );
    }

    #[test]
    fn legacy_option_data_snapshot_restores_to_schema_backed_typed_object() {
        let (_registry, schemas) = TypeSchemaRegistry::with_stdlib_types_and_builtin_ids();
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");
        let sv = SV::OptionData {
            is_some: false,
            payload: None,
        };

        let (bits, kind) =
            serializable_to_slot(&sv, NativeKind::Ptr(HeapKind::TypedObject), &store)
                .expect("restore legacy option");
        let restored = restored_storage(bits, kind);
        let restored_storage = storage(&restored);
        assert_eq!(restored_storage.schema_id, schemas.option as u64);
        assert_eq!(restored_storage.slots()[0].as_i64(), OPTION_VARIANT_NONE);
        assert_eq!(
            restored_storage.field_kinds[OPTION_PAYLOAD],
            NativeKind::Null
        );
    }

    #[test]
    fn wf2g_module_fn_projection_and_restore_round_trip() {
        // WF-2G GAP A: a `Ptr(HeapKind::ModuleFn)` slot (inline-scalar
        // module-fn id) projects to `SV::ModuleFunction(qualified_name)` and
        // restores back to the SAME id via the installed name table. Id 0 is
        // exercised explicitly (it was formerly swallowed by the null-pointer
        // guard). No SIGABRT at drop (ModuleFn clone/drop are no-ops).
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");

        super::install_module_fn_name_table(vec![
            "std::core::json::stringify".to_string(),
            "std::core::math::sqrt".to_string(),
        ]);

        for (id, name) in [(0u64, "std::core::json::stringify"), (1, "std::core::math::sqrt")] {
            // Project.
            let sv = slot_to_serializable(id, NativeKind::Ptr(HeapKind::ModuleFn), &store)
                .expect("project module fn");
            match &sv {
                SV::ModuleFunction(n) => assert_eq!(n, name, "projected qualified name"),
                other => panic!("expected ModuleFunction, got {other:?}"),
            }
            // Restore.
            let (bits, kind) =
                serializable_to_slot(&sv, NativeKind::Ptr(HeapKind::ModuleFn), &store)
                    .expect("restore module fn");
            assert_eq!(bits, id, "restored id parity");
            assert_eq!(kind, NativeKind::Ptr(HeapKind::ModuleFn));
        }

        // An unresolvable name on a resumer missing the module — clean refuse.
        let missing = SV::ModuleFunction("absent::module::fn".to_string());
        assert!(
            serializable_to_slot(&missing, NativeKind::Ptr(HeapKind::ModuleFn), &store).is_err(),
            "unresolvable module fn name must clean-refuse, never fabricate an id"
        );
    }

    #[test]
    fn legacy_result_data_snapshot_restores_to_schema_backed_typed_object() {
        let (_registry, schemas) = TypeSchemaRegistry::with_stdlib_types_and_builtin_ids();
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");
        let sv = SV::ResultData {
            is_ok: false,
            payload: Box::new(SV::String("bad".to_string())),
        };

        let (bits, kind) =
            serializable_to_slot(&sv, NativeKind::Ptr(HeapKind::TypedObject), &store)
                .expect("restore legacy result");
        let restored = restored_storage(bits, kind);
        let restored_storage = storage(&restored);
        assert_eq!(restored_storage.schema_id, schemas.result as u64);
        assert_eq!(restored_storage.slots()[0].as_i64(), RESULT_VARIANT_ERR);
        assert_eq!(
            restored_storage.field_kinds[RESULT_PAYLOAD],
            NativeKind::String
        );
    }

    #[test]
    fn legacy_result_option_old_expected_kind_normalizes_to_typed_object() {
        let (_registry, schemas) = TypeSchemaRegistry::with_stdlib_types_and_builtin_ids();
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");
        let result_sv = SV::ResultData {
            is_ok: true,
            payload: Box::new(SV::Int(42)),
        };
        let option_sv = SV::OptionData {
            is_some: false,
            payload: None,
        };

        let (result_bits, result_kind) =
            serializable_to_slot(&result_sv, NativeKind::Ptr(HeapKind::Result), &store)
                .expect("restore legacy result with old expected kind");
        let result = restored_storage(result_bits, result_kind);
        let result_storage = storage(&result);
        assert_eq!(result_storage.schema_id, schemas.result as u64);
        assert_eq!(result_storage.slots()[0].as_i64(), RESULT_VARIANT_OK);
        assert_eq!(
            result_storage.field_kinds[RESULT_PAYLOAD],
            NativeKind::Int64
        );

        let (option_bits, option_kind) =
            serializable_to_slot(&option_sv, NativeKind::Ptr(HeapKind::Option), &store)
                .expect("restore legacy option with old expected kind");
        let option = restored_storage(option_bits, option_kind);
        let option_storage = storage(&option);
        assert_eq!(option_storage.schema_id, schemas.option as u64);
        assert_eq!(option_storage.slots()[0].as_i64(), OPTION_VARIANT_NONE);
        assert_eq!(option_storage.field_kinds[OPTION_PAYLOAD], NativeKind::Null);
    }

    #[test]
    fn typed_object_field_legacy_result_data_restores_field_as_typed_object() {
        let (_registry, schemas) = TypeSchemaRegistry::with_stdlib_types_and_builtin_ids();
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");
        let sv = SV::TypedObject {
            schema_id: 9_999,
            slot_data: vec![SV::ResultData {
                is_ok: true,
                payload: Box::new(SV::Int(7)),
            }],
            heap_mask: 1,
        };

        let (bits, kind) =
            serializable_to_slot(&sv, NativeKind::Ptr(HeapKind::TypedObject), &store)
                .expect("restore typed object with legacy result field");
        let outer = restored_storage(bits, kind);
        let outer_storage = storage(&outer);
        assert_eq!(
            outer_storage.field_kinds[0],
            NativeKind::Ptr(HeapKind::TypedObject)
        );

        let inner_slot = KindedSlot::new(outer_storage.slots()[0], outer_storage.field_kinds[0]);
        let inner_storage = storage(&inner_slot);
        assert_eq!(inner_storage.schema_id, schemas.result as u64);
        assert_eq!(inner_storage.slots()[0].as_i64(), RESULT_VARIANT_OK);
        std::mem::forget(inner_slot);
    }

    #[test]
    fn schema_backed_result_and_option_snapshot_as_typed_objects() {
        let (_registry, schemas) = TypeSchemaRegistry::with_stdlib_types_and_builtin_ids();
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");
        let ok = variant_object(
            schemas.result as u64,
            RESULT_VARIANT_OK,
            KindedSlot::from_int(9),
        );
        let some = variant_object(
            schemas.option as u64,
            OPTION_VARIANT_SOME,
            KindedSlot::from_int(5),
        );

        let ok_sv = slot_to_serializable(ok.raw(), ok.kind(), &store).expect("serialize ok");
        let some_sv =
            slot_to_serializable(some.raw(), some.kind(), &store).expect("serialize some");

        // GC Phase 5 (v7): TypedObjects are identity-interned → `HeapNode`-
        // wrapped. Unwrap the body before asserting the schema.
        let unwrap = |sv: &SV| -> SV {
            match sv {
                SV::HeapNode { body, .. } => (**body).clone(),
                other => panic!("expected HeapNode(TypedObject), got {other:?}"),
            }
        };
        assert!(matches!(
            unwrap(&ok_sv),
            SV::TypedObject {
                schema_id,
                ..
            } if schema_id == schemas.result as u64
        ));
        assert!(matches!(
            unwrap(&some_sv),
            SV::TypedObject {
                schema_id,
                ..
            } if schema_id == schemas.option as u64
        ));
    }
}

#[cfg(test)]
mod wf2g_gap_b_heap_element_array_tests {
    //! WF-2G GAP B (2026-07-06): heap-element TypedArray snapshot round-trip.
    //! A runtime `TypedArray` whose elements are heap pointers
    //! (`Array<string>` / `Array<Decimal>` / `Array<TypedObject>`) must
    //! project into `SV::Array(Vec<SV>)` via typed carriers and restore into
    //! an element-equal carrier. Each test drives BOTH the projection and the
    //! restore, asserts element equality, and releases BOTH the origin and the
    //! restored carrier so the share-accounting is proven balanced (a
    //! double-free would SIGABRT under the test harness; a leak would trip
    //! miri / the leak sanitizer in the deep-test lane).

    use super::{
        SerializableVMValue as SV, SnapshotStore, serializable_to_slot, slot_to_serializable,
    };
    use shape_value::v2::decimal_obj::DecimalObj;
    use shape_value::v2::string_obj::StringObj;
    use shape_value::v2::typed_array::{
        ELEM_TYPE_DECIMAL, ELEM_TYPE_STRING, ELEM_TYPE_TYPED_OBJECT, TypedArray, read_elem_type,
        release_v2_typed_array, stamp_elem_type,
    };
    use shape_value::{HeapKind, NativeKind, TypedObjectStorage, ValueSlot};
    use std::sync::Arc;

    #[test]
    fn string_array_snapshot_round_trips() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");

        // Build a runtime Array<string> = ["alice", "bob", "carol"].
        let arr = TypedArray::<*const StringObj>::with_capacity(3);
        unsafe {
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_STRING);
            for s in ["alice", "bob", "carol"] {
                TypedArray::<*const StringObj>::push(arr, StringObj::new(s) as *const StringObj);
            }
        }
        let bits = arr as usize as u64;

        // Project.
        let sv = slot_to_serializable(bits, NativeKind::Ptr(HeapKind::TypedArray), &store)
            .expect("project Array<string>");
        match &sv {
            SV::Array(elems) => {
                let got: Vec<&str> = elems
                    .iter()
                    .map(|e| match e {
                        SV::String(s) => s.as_str(),
                        other => panic!("expected SV::String, got {other:?}"),
                    })
                    .collect();
                assert_eq!(got, vec!["alice", "bob", "carol"]);
            }
            other => panic!("expected SV::Array, got {other:?}"),
        }

        // Restore into a fresh carrier (fresh-process resume shape).
        let (rbits, rkind) =
            serializable_to_slot(&sv, NativeKind::Ptr(HeapKind::TypedArray), &store)
                .expect("restore Array<string>");
        assert_eq!(rkind, NativeKind::Ptr(HeapKind::TypedArray));
        unsafe {
            let rptr = rbits as *const u8;
            assert_eq!(read_elem_type(rptr), ELEM_TYPE_STRING);
            let slice = TypedArray::<*const StringObj>::as_slice(
                rptr as *const TypedArray<*const StringObj>,
            );
            let got: Vec<&str> = slice.iter().map(|&p| StringObj::as_str(p)).collect();
            assert_eq!(got, vec!["alice", "bob", "carol"], "restored element-equal");
        }

        // Balanced drop — neither carrier double-frees.
        unsafe {
            release_v2_typed_array(arr as *mut u8);
            release_v2_typed_array(rbits as *mut u8);
        }
    }

    #[test]
    fn decimal_array_snapshot_round_trips() {
        use rust_decimal::Decimal;
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");

        let vals = [
            Decimal::new(15, 1),   // 1.5
            Decimal::new(2500, 2), // 25.00
            Decimal::new(-7, 0),   // -7
        ];
        let arr = TypedArray::<*const DecimalObj>::with_capacity(vals.len() as u32);
        unsafe {
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_DECIMAL);
            for d in vals {
                TypedArray::<*const DecimalObj>::push(arr, DecimalObj::new(d) as *const DecimalObj);
            }
        }
        let bits = arr as usize as u64;

        let sv = slot_to_serializable(bits, NativeKind::Ptr(HeapKind::TypedArray), &store)
            .expect("project Array<Decimal>");
        match &sv {
            SV::Array(elems) => {
                let got: Vec<Decimal> = elems
                    .iter()
                    .map(|e| match e {
                        SV::Decimal(d) => *d,
                        other => panic!("expected SV::Decimal, got {other:?}"),
                    })
                    .collect();
                assert_eq!(got, vals.to_vec());
            }
            other => panic!("expected SV::Array, got {other:?}"),
        }

        let (rbits, rkind) =
            serializable_to_slot(&sv, NativeKind::Ptr(HeapKind::TypedArray), &store)
                .expect("restore Array<Decimal>");
        assert_eq!(rkind, NativeKind::Ptr(HeapKind::TypedArray));
        unsafe {
            let rptr = rbits as *const u8;
            assert_eq!(read_elem_type(rptr), ELEM_TYPE_DECIMAL);
            let slice = TypedArray::<*const DecimalObj>::as_slice(
                rptr as *const TypedArray<*const DecimalObj>,
            );
            let got: Vec<Decimal> = slice.iter().map(|&p| DecimalObj::value(p)).collect();
            assert_eq!(got, vals.to_vec(), "restored element-equal");
        }

        unsafe {
            release_v2_typed_array(arr as *mut u8);
            release_v2_typed_array(rbits as *mut u8);
        }
    }

    /// Build a single-i64-field TypedObjectStorage (heap_mask = 0, no heap
    /// fields) with a fresh refcount = 1. Caller owns the single share.
    fn make_int_object(schema_id: u64, val: i64) -> *const TypedObjectStorage {
        let field_kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64].into_boxed_slice());
        TypedObjectStorage::_new(
            schema_id,
            vec![ValueSlot::from_int(val)].into_boxed_slice(),
            0,
            field_kinds,
        )
    }

    #[test]
    fn typed_object_array_snapshot_round_trips() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");

        // Array<TypedObject> with two single-int-field rows.
        let arr = TypedArray::<*const TypedObjectStorage>::with_capacity(2);
        unsafe {
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_TYPED_OBJECT);
            // Each `_new` gives refcount = 1; push transfers that share.
            TypedArray::<*const TypedObjectStorage>::push(arr, make_int_object(4242, 10));
            TypedArray::<*const TypedObjectStorage>::push(arr, make_int_object(4242, 20));
        }
        let bits = arr as usize as u64;

        let sv = slot_to_serializable(bits, NativeKind::Ptr(HeapKind::TypedArray), &store)
            .expect("project Array<TypedObject>");
        // GC Phase 5 (v7): a TypedObject-element array is now identity-interned,
        // so it projects as a `HeapNode` body wrapping `SV::Array`, and each
        // element TypedObject is itself a `HeapNode`. Unwrap both levels; the
        // round-trip identity is unchanged.
        let array_body = match &sv {
            SV::HeapNode { body, .. } => &**body,
            other => panic!("expected HeapNode(Array), got {other:?}"),
        };
        match array_body {
            SV::Array(elems) => {
                assert_eq!(elems.len(), 2);
                let got: Vec<(u64, i64)> = elems
                    .iter()
                    .map(|e| match e {
                        SV::HeapNode { body, .. } => match &**body {
                            SV::TypedObject {
                                schema_id,
                                slot_data,
                                ..
                            } => {
                                let v = match &slot_data[0] {
                                    SV::Int(i) => *i,
                                    other => panic!("expected SV::Int field, got {other:?}"),
                                };
                                (*schema_id, v)
                            }
                            other => panic!("expected HeapNode(TypedObject), got {other:?}"),
                        },
                        other => panic!("expected HeapNode element, got {other:?}"),
                    })
                    .collect();
                assert_eq!(got, vec![(4242, 10), (4242, 20)]);
            }
            other => panic!("expected SV::Array body, got {other:?}"),
        }

        let (rbits, rkind) =
            serializable_to_slot(&sv, NativeKind::Ptr(HeapKind::TypedArray), &store)
                .expect("restore Array<TypedObject>");
        assert_eq!(rkind, NativeKind::Ptr(HeapKind::TypedArray));
        unsafe {
            let rptr = rbits as *const u8;
            assert_eq!(read_elem_type(rptr), ELEM_TYPE_TYPED_OBJECT);
            let slice = TypedArray::<*const TypedObjectStorage>::as_slice(
                rptr as *const TypedArray<*const TypedObjectStorage>,
            );
            let got: Vec<(u64, i64)> = slice
                .iter()
                .map(|&p| {
                    let storage: &TypedObjectStorage = &*p;
                    (storage.schema_id, storage.slots()[0].as_i64())
                })
                .collect();
            assert_eq!(got, vec![(4242, 10), (4242, 20)], "restored element-equal");
        }

        unsafe {
            release_v2_typed_array(arr as *mut u8);
            release_v2_typed_array(rbits as *mut u8);
        }
    }
}

#[cfg(test)]
mod opaque_disposition_tests {
    //! Wave 7 resumability (2026-07-07): the six opaque-marker heap arms
    //! now honor their user-ruled dispositions (2026-05-29).
    //!
    //! * **Clean-refuse** — Iterator / Deque / Channel / FilterExpr wrap a
    //!   live in-process resource that is intrinsically not
    //!   snapshot-restorable. They refuse at the earliest honest point:
    //!   `snapshot()` **encode** returns a distinguishable per-type error
    //!   naming the type ("snapshot cannot capture a live <Type>: …")
    //!   instead of silently dropping the payload into a discriminator-only
    //!   wire shape. The restore-side arm stays as terminal defense-in-depth
    //!   (an externally-supplied opaque wire shape still refuses cleanly).
    //! * **Defined-reset** — Mutex / Lazy resume to a defined, deterministic
    //!   reset state with no stale payload: Mutex → unlocked/empty (Null
    //!   absence sentinel), Lazy → unforced/uninitialized. Restore returns
    //!   `Ok` with a fresh reset carrier, not an error.
    //!
    //! Reference / SharedCell are STAGE-R5 serialize-through (identity-map
    //! two-pass); their ctx-free path surfaces the two-pass-required message.

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

    /// CLEAN-REFUSE at snapshot()-encode time: a live Iterator / Deque /
    /// Channel / FilterExpr carrier refuses at capture with a
    /// distinguishable per-type message naming the type — NEVER a silent
    /// drop to an opaque `Ok(...)` wire shape. This is the earliest honest
    /// refuse point (the user-ruled preference).
    #[test]
    fn clean_refuse_types_refuse_at_snapshot_encode() {
        use shape_value::heap_value::{ChannelData, DequeData};
        use shape_value::iterator_state::{IteratorSource, IteratorState};
        use shape_value::kinded_slot::KindedSlot;
        use shape_value::{FilterLiteral, FilterNode, FilterOp, ValueSlot};

        let (_tmp, st) = store();

        // Live carriers held in owning KindedSlots — each releases its
        // Arc share on Drop at scope end, so no leak / no double-free.
        let channel = KindedSlot::from_channel(Arc::new(ChannelData::new()));
        let deque = KindedSlot::from_deque(Arc::new(DequeData::new()));
        let iterator = KindedSlot::from_iterator(Arc::new(IteratorState::new(
            IteratorSource::Range {
                start: 0,
                end: 3,
                step: 1,
            },
        )));
        let filter_bits = Arc::into_raw(Arc::new(FilterNode::Compare {
            column: "x".to_string(),
            op: FilterOp::Eq,
            value: FilterLiteral::Int(1),
        })) as u64;
        let filter = KindedSlot::new(
            ValueSlot::from_raw(filter_bits),
            NativeKind::Ptr(HeapKind::FilterExpr),
        );

        let cases = [
            (&channel, "Channel"),
            (&deque, "Deque"),
            (&iterator, "Iterator"),
            (&filter, "FilterExpr"),
        ];
        for (ks, name) in cases {
            let err = slot_to_serializable(ks.slot().raw(), ks.kind(), &st).expect_err(
                "live clean-refuse carrier must refuse at snapshot()-encode, not serialize-through",
            );
            assert!(
                err.contains("snapshot cannot capture a live") && err.contains(name),
                "{name} encode must produce a distinguishable clean-refuse \
                 message naming the type, got: {err}"
            );
        }
    }

    // NOTE: the four live-resource arms also stay terminal clean-refuse on
    // the restore side (defense-in-depth against an externally-supplied
    // opaque wire shape), covered by
    // `clean_refuse_by_design_arms_carry_design_wording` above.

    /// DEFINED-RESET on restore: Mutex → unlocked/empty (Null absence
    /// sentinel), Lazy → unforced/uninitialized. Both return `Ok` with a
    /// fresh reset carrier — NOT an error, and with no stale payload.
    #[test]
    fn mutex_lazy_reset_to_defined_state_on_restore() {
        use shape_value::heap_value::{LazyData, MutexData};

        let (_tmp, st) = store();

        // Mutex resets to empty (Null absence sentinel), unlocked.
        let (mbits, mkind) = serializable_to_slot(
            &SerializableVMValue::MutexOpaque { has_value: true },
            NativeKind::Ptr(HeapKind::Mutex),
            &st,
        )
        .expect("Mutex must resume to a defined reset state, not refuse");
        assert_eq!(mkind, NativeKind::Ptr(HeapKind::Mutex));
        // Reconstruct the owning share and inspect the reset value.
        let m = unsafe { Arc::<MutexData>::from_raw(mbits as *const MutexData) };
        assert_eq!(
            m.get().kind(),
            NativeKind::Null,
            "reset Mutex must hold the Null absence sentinel (empty), no stale payload"
        );
        assert!(m.try_lock(), "reset Mutex must be unlocked");
        drop(m); // release the one restored share

        // Lazy resets to uninitialized (no cached value, no initializer).
        let (lbits, lkind) = serializable_to_slot(
            &SerializableVMValue::LazyOpaque {
                is_initialized: true,
            },
            NativeKind::Ptr(HeapKind::Lazy),
            &st,
        )
        .expect("Lazy must resume to a defined reset state, not refuse");
        assert_eq!(lkind, NativeKind::Ptr(HeapKind::Lazy));
        let l = unsafe { Arc::<LazyData>::from_raw(lbits as *const LazyData) };
        assert!(
            !l.is_initialized(),
            "reset Lazy must be unforced/uninitialized (no cached value)"
        );
        assert!(
            l.take_initializer().is_none(),
            "reset Lazy carries no initializer — forcing it surfaces cleanly, no stale payload"
        );
        drop(l); // release the one restored share
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

/// GC Phase 5 (snapshot v7, real-gc-cycle-collection.md §0 #4 / §6):
/// the identity-map is generalized from `SharedCell`/`Reference` to every
/// cycle-capable `HeapKind`. These tests exercise the new `HeapNode` /
/// `HeapRef` wire arms at the slot/ctx level (the same tier as the STAGE-R5
/// SharedCell round-trip tests above): an OBJECT cycle round-trips with
/// identity, containers holding shared/cyclic nodes round-trip, a doubly-
/// referenced acyclic object dedups to one node, and a v6 snapshot is
/// version-refused.
#[cfg(test)]
mod gc_phase5_identity_tests {
    use super::{
        RestoreLinkCtx, SerializableVMValue as SV, SerializeIdentityCtx, SnapshotStore,
        materialize_cell_bodies, serializable_to_slot_ctx, slot_to_serializable_ctx,
    };
    use shape_value::heap_value::{
        HashMapData, HashMapKindedRef, TypedObjectPtr, TypedObjectStorage,
    };
    use shape_value::v2::heap_element::HeapElement;
    use shape_value::v2::heap_header::HeapHeader;
    use shape_value::v2::refcount::{v2_get_refcount, v2_retain};
    use shape_value::v2::typed_array::{
        ELEM_TYPE_TYPED_OBJECT, TypedArray, release_v2_typed_array, stamp_elem_type,
    };
    use shape_value::{HeapKind, NativeKind, ValueSlot};
    use std::sync::Arc;

    fn store() -> (tempfile::TempDir, SnapshotStore) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let st = SnapshotStore::new(tmp.path()).expect("snapshot store");
        (tmp, st)
    }

    /// Read a v2-header refcount off any header-carrier ptr (header @ offset 0).
    fn rc(ptr: u64) -> u32 {
        unsafe { v2_get_refcount(ptr as *const HeapHeader) }
    }

    /// Build a fresh single-`int`-field TypedObject (acyclic, refcount 1).
    fn make_int_object(int_val: i64) -> *mut TypedObjectStorage {
        let slots = vec![ValueSlot::from_int(int_val)].into_boxed_slice();
        let field_kinds: Arc<[NativeKind]> = vec![NativeKind::Int64].into();
        TypedObjectStorage::_new(42, slots, 0, field_kinds)
    }

    /// Build a self-linked `type Node { first: int, next: Node? }` object:
    /// field 0 = `Int(seed)`, field 1 = a `Ptr(TypedObject)` pointing at the
    /// node itself. Returns a ptr with refcount 2 = {caller holder, self-edge}
    /// — exactly the runtime shape of `let n = Node(...); n.next = n`.
    fn make_self_cyclic_object(seed: i64) -> *mut TypedObjectStorage {
        let slots = vec![ValueSlot::from_int(seed), ValueSlot::from_raw(0)].into_boxed_slice();
        let field_kinds: Arc<[NativeKind]> =
            vec![NativeKind::Int64, NativeKind::Ptr(HeapKind::TypedObject)].into();
        // heap_mask 0 initially: field 1 is a null placeholder (drop-safe).
        let ptr = TypedObjectStorage::_new(9, slots, 0, field_kinds);
        unsafe {
            // n.next = n : the self-edge owns one share.
            v2_retain(ptr as *const HeapHeader);
            let _ = TypedObjectStorage::write_slot_in_place(ptr, 1, ptr as u64);
            *std::ptr::addr_of_mut!((*ptr).heap_mask) = 1 << 1;
        }
        ptr
    }

    /// Break a self-cyclic object's `next` self-edge (heap_mask→0 so the drop
    /// walk never touches a freed peer) and retire `count` shares. Safe,
    /// double-free-free teardown for the tests' hand-built cyclic graphs.
    unsafe fn dismantle(ptr: *mut TypedObjectStorage, count: u32) {
        unsafe {
            *std::ptr::addr_of_mut!((*ptr).heap_mask) = 0;
            let _ = TypedObjectStorage::write_slot_in_place(ptr, 1, 0);
            for _ in 0..count {
                TypedObjectStorage::release_elem(ptr);
            }
        }
    }

    /// (a) An OBJECT cycle (`type Node { var next: Node? }` self-linked)
    /// round-trips: the restored node's `next` field aliases the restored node
    /// ITSELF (identity preserved), with no infinite recursion and no
    /// duplication. Pre-v7 this INFINITE-RECURSED the serializer.
    #[test]
    fn object_cycle_roundtrips_with_identity() {
        let (_tmp, st) = store();
        let o = make_self_cyclic_object(42);

        // SERIALIZE through one shared ctx.
        let mut ictx = SerializeIdentityCtx::new();
        let sv = slot_to_serializable_ctx(
            o as u64,
            NativeKind::Ptr(HeapKind::TypedObject),
            &st,
            &mut ictx,
        )
        .expect("serialize self-cyclic object (no infinite recursion)");

        // Wire shape: HeapNode body whose `next` field is a HeapRef back to
        // the SAME handle (the cycle broken by identity).
        match &sv {
            SV::HeapNode { handle, body } => match &**body {
                SV::TypedObject {
                    slot_data,
                    heap_mask,
                    ..
                } => {
                    assert_eq!(*heap_mask, 1 << 1, "field 1 (next) is the heap self-edge");
                    assert!(matches!(slot_data[0], SV::Int(42)));
                    match &slot_data[1] {
                        SV::HeapRef { handle: h2 } => {
                            assert_eq!(h2, handle, "next is a back-edge to the node itself")
                        }
                        other => panic!("expected HeapRef self-edge, got {other:?}"),
                    }
                }
                other => panic!("expected TypedObject body, got {other:?}"),
            },
            other => panic!("expected HeapNode, got {other:?}"),
        }

        // RESTORE via the two-pass driver.
        let mut link = RestoreLinkCtx::new();
        materialize_cell_bodies(&sv, &st, &mut link).expect("pass 1");
        let (rbits, rkind) = serializable_to_slot_ctx(
            &sv,
            NativeKind::Ptr(HeapKind::TypedObject),
            &st,
            &mut link,
        )
        .expect("pass 2");
        assert_eq!(rkind, NativeKind::Ptr(HeapKind::TypedObject));

        // IDENTITY: the restored node's `next` slot points at the restored
        // node itself — one allocation, a real cycle, not two copies.
        let ro = rbits as *const TypedObjectStorage;
        let next_bits = unsafe { (*ro).slots()[1].raw() };
        assert_eq!(
            next_bits, rbits,
            "restored Node.next aliases the SAME restored node (identity preserved, no duplication)"
        );
        assert_eq!(unsafe { (*ro).slots()[0].raw() }, 42, "scalar field survives");

        // Refcount after base-share release = {Pass-2 external slot, self-edge}.
        link.release_base_shares();
        assert_eq!(rc(rbits), 2, "external slot + one self-edge — no leaked/extra share");

        // Teardown (both restored + original cyclic graphs).
        unsafe {
            dismantle(rbits as *mut TypedObjectStorage, 2);
            dismantle(o, 2);
        }
    }

    /// (c) A doubly-referenced (shared, ACYCLIC) object resumes as ONE node:
    /// two slots holding the same object dedupe to a single restored
    /// allocation (pre-v7 they duplicated into two).
    #[test]
    fn shared_acyclic_object_dedupes_to_one_node() {
        let (_tmp, st) = store();
        let a = make_int_object(7);
        // Two slots share `a` (two owning shares).
        unsafe { v2_retain(a as *const HeapHeader) };
        let slot0 = a as u64;
        let slot1 = a as u64;

        let mut ictx = SerializeIdentityCtx::new();
        let sv0 = slot_to_serializable_ctx(
            slot0,
            NativeKind::Ptr(HeapKind::TypedObject),
            &st,
            &mut ictx,
        )
        .unwrap();
        let sv1 = slot_to_serializable_ctx(
            slot1,
            NativeKind::Ptr(HeapKind::TypedObject),
            &st,
            &mut ictx,
        )
        .unwrap();
        // First carrier = body; second = back-edge to the same handle.
        let h = match (&sv0, &sv1) {
            (SV::HeapNode { handle, .. }, SV::HeapRef { handle: h2 }) => {
                assert_eq!(handle, h2, "both carriers share one identity handle");
                *handle
            }
            other => panic!("expected body + back-edge, got {other:?}"),
        };
        let _ = h;

        let mut link = RestoreLinkCtx::new();
        materialize_cell_bodies(&sv0, &st, &mut link).unwrap();
        materialize_cell_bodies(&sv1, &st, &mut link).unwrap();
        let (r0, _) = serializable_to_slot_ctx(
            &sv0,
            NativeKind::Ptr(HeapKind::TypedObject),
            &st,
            &mut link,
        )
        .unwrap();
        let (r1, _) = serializable_to_slot_ctx(
            &sv1,
            NativeKind::Ptr(HeapKind::TypedObject),
            &st,
            &mut link,
        )
        .unwrap();
        assert_eq!(r0, r1, "two carriers of one shared object dedupe to ONE restored node");
        link.release_base_shares();
        assert_eq!(rc(r0), 2, "exactly two slot shares on the one deduped node");

        // Teardown: restored node (2 slot shares) + original (2 shares).
        unsafe {
            TypedObjectStorage::release_elem(r0 as *const TypedObjectStorage);
            TypedObjectStorage::release_elem(r0 as *const TypedObjectStorage);
            TypedObjectStorage::release_elem(a);
            TypedObjectStorage::release_elem(a);
        }
    }

    /// (b-array) A TypedArray holding a self-cyclic node round-trips with
    /// identity: the restored array's element is a node whose `next` aliases
    /// itself, and the array holds exactly that one allocation.
    #[test]
    fn typed_array_holding_cyclic_node_roundtrips() {
        let (_tmp, st) = store();
        let o = make_self_cyclic_object(5); // rc 2 {holder, self-edge}
        let arr = TypedArray::<*const TypedObjectStorage>::with_capacity(1);
        unsafe {
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_TYPED_OBJECT);
            v2_retain(o as *const HeapHeader); // array's element share
            TypedArray::<*const TypedObjectStorage>::push(arr, o as *const TypedObjectStorage);
        }

        let mut ictx = SerializeIdentityCtx::new();
        let sv = slot_to_serializable_ctx(
            arr as u64,
            NativeKind::Ptr(HeapKind::TypedArray),
            &st,
            &mut ictx,
        )
        .expect("serialize array-of-cyclic-object (no infinite recursion)");
        // HeapNode{Array([HeapNode{TypedObject...}])}
        match &sv {
            SV::HeapNode { body, .. } => match &**body {
                SV::Array(elems) => {
                    assert_eq!(elems.len(), 1);
                    assert!(matches!(elems[0], SV::HeapNode { .. }));
                }
                other => panic!("expected Array body, got {other:?}"),
            },
            other => panic!("expected HeapNode, got {other:?}"),
        }

        let mut link = RestoreLinkCtx::new();
        materialize_cell_bodies(&sv, &st, &mut link).unwrap();
        let (rbits, rkind) = serializable_to_slot_ctx(
            &sv,
            NativeKind::Ptr(HeapKind::TypedArray),
            &st,
            &mut link,
        )
        .unwrap();
        assert_eq!(rkind, NativeKind::Ptr(HeapKind::TypedArray));

        // The restored array's element 0 is a node whose `next` aliases itself.
        let elem0 = unsafe {
            TypedArray::<*const TypedObjectStorage>::get_unchecked(
                rbits as *const TypedArray<*const TypedObjectStorage>,
                0,
            )
        };
        let elem_next = unsafe { (*elem0).slots()[1].raw() };
        assert_eq!(
            elem_next, elem0 as u64,
            "restored array element's next aliases the element itself (cyclic identity preserved)"
        );

        link.release_base_shares();

        // Teardown. Restored: arr' rc 1 (external), elem0 rc 2 {array edge, self}.
        unsafe {
            *std::ptr::addr_of_mut!((*(elem0 as *mut TypedObjectStorage)).heap_mask) = 0;
            let _ = TypedObjectStorage::write_slot_in_place(elem0 as *mut TypedObjectStorage, 1, 0);
            release_v2_typed_array(rbits as *mut u8); // frees arr', releases one elem edge
            TypedObjectStorage::release_elem(elem0); // retire the orphaned self-edge
            // Original serialize-side graph.
            *std::ptr::addr_of_mut!((*o).heap_mask) = 0;
            let _ = TypedObjectStorage::write_slot_in_place(o, 1, 0);
            release_v2_typed_array(arr as *mut u8); // frees orig arr, releases one o edge
            TypedObjectStorage::release_elem(o); // holder
            TypedObjectStorage::release_elem(o); // orphaned self-edge
        }
    }

    /// (b-hashmap) A `HashMap<string, TypedObject>` holding a shared node
    /// round-trips with identity: two keys mapping to the SAME object dedupe
    /// to one restored allocation.
    #[test]
    fn hashmap_holding_shared_node_roundtrips() {
        let (_tmp, st) = store();
        let a = make_int_object(11); // rc 1
        let mut data: HashMapData<TypedObjectPtr> = HashMapData::new();
        unsafe {
            // Two keys share `a` (two owning shares held by the map).
            v2_retain(a as *const HeapHeader);
            v2_retain(a as *const HeapHeader);
            data.insert("x", TypedObjectPtr::new(a));
            data.insert("y", TypedObjectPtr::new(a));
        }
        let kref = Arc::new(HashMapKindedRef::TypedObject(Arc::new(data)));
        let map_bits = Arc::into_raw(kref) as u64;

        let mut ictx = SerializeIdentityCtx::new();
        let sv = slot_to_serializable_ctx(
            map_bits,
            NativeKind::Ptr(HeapKind::HashMap),
            &st,
            &mut ictx,
        )
        .expect("serialize map-of-shared-object");
        // HeapNode{HashMap{keys:[x,y], values:[HeapNode, HeapRef]}}
        match &sv {
            SV::HeapNode { body, .. } => match &**body {
                SV::HashMap { keys, values } => {
                    assert_eq!(keys.len(), 2);
                    assert!(matches!(values[0], SV::HeapNode { .. }));
                    assert!(matches!(values[1], SV::HeapRef { .. }));
                }
                other => panic!("expected HashMap body, got {other:?}"),
            },
            other => panic!("expected HeapNode, got {other:?}"),
        }

        let mut link = RestoreLinkCtx::new();
        materialize_cell_bodies(&sv, &st, &mut link).unwrap();
        let (rbits, rkind) = serializable_to_slot_ctx(
            &sv,
            NativeKind::Ptr(HeapKind::HashMap),
            &st,
            &mut link,
        )
        .unwrap();
        assert_eq!(rkind, NativeKind::Ptr(HeapKind::HashMap));

        // Both restored values alias ONE node (dedup).
        let restored = unsafe { Arc::<HashMapKindedRef>::from_raw(rbits as *const HashMapKindedRef) };
        match &*restored {
            HashMapKindedRef::TypedObject(map_arc) => {
                assert_eq!(map_arc.len(), 2);
                let (v0, v1) = unsafe {
                    let v0: &TypedObjectPtr = &*(*map_arc.values).data.add(0);
                    let v1: &TypedObjectPtr = &*(*map_arc.values).data.add(1);
                    (v0.as_ptr(), v1.as_ptr())
                };
                assert_eq!(v0, v1, "both map values dedupe to ONE restored node");
                assert_eq!(unsafe { (*v0).slots()[0].raw() }, 11, "shared node value survives");
            }
            _ => panic!("expected TypedObject-valued map"),
        }
        let _ = Arc::into_raw(restored); // restore the slot share

        link.release_base_shares();
        // Teardown.
        unsafe {
            Arc::decrement_strong_count(rbits as *const HashMapKindedRef); // restored map
            Arc::decrement_strong_count(map_bits as *const HashMapKindedRef); // original map
        }
    }

    /// (d) A plain scalar round-trip is UNAFFECTED by the generalization
    /// (no HeapNode wrapping for non-cycle-capable values).
    #[test]
    fn scalar_slot_roundtrip_unwrapped() {
        let (_tmp, st) = store();
        let mut ictx = SerializeIdentityCtx::new();
        let sv = slot_to_serializable_ctx(99u64, NativeKind::Int64, &st, &mut ictx).unwrap();
        assert!(matches!(sv, SV::Int(99)), "scalars are never HeapNode-wrapped");
    }

    /// (e) A v6 snapshot is version-REFUSED cleanly at v7 — never misparsed,
    /// never Bool-defaulted.
    #[test]
    fn v6_snapshot_is_version_refused() {
        use super::{ExecutionSnapshot, SNAPSHOT_VERSION};
        use crate::hashing::HashDigest;
        let (_tmp, st) = store();
        assert_eq!(SNAPSHOT_VERSION, 7, "this build reads v7");
        let stale = ExecutionSnapshot {
            version: 6,
            created_at_ms: 0,
            semantic_hash: HashDigest::from_hex(&"0".repeat(64)),
            context_hash: HashDigest::from_hex(&"0".repeat(64)),
            vm_hash: None,
            bytecode_hash: None,
            code_manifest: None,
            script_path: None,
            label: None,
        };
        let hash = st.put_snapshot(&stale).expect("write v6 envelope");
        let err = st
            .get_snapshot(&hash)
            .expect_err("a v6 snapshot must be refused at v7");
        let msg = format!("{err}");
        assert!(
            msg.contains("unsupported snapshot version 6") && msg.contains("v6→v7"),
            "expected a clean version-refusal, got: {msg}"
        );
    }

    /// (b-mutual, independent adversarial 2026-07-08) A TWO-NODE mutual cycle
    /// (`A.next = B`, `B.next = A` across DISTINCT objects) round-trips with
    /// identity: restored `A'` and `B'` point at each other (A'->B'->A'), two
    /// distinct allocations forming ONE cycle, no duplication, no infinite
    /// recursion. Exercises interning a FORWARD child (B nested inside A's
    /// field) followed by a back-edge to an already-interned ANCESTOR (A) —
    /// the canonical cross-object cycle the self-loop case does not fully
    /// cover, and the shape that would infinite-recurse if the serialize ctx
    /// were forked per field instead of threaded.
    #[test]
    fn two_node_mutual_cycle_roundtrips_with_identity() {
        let (_tmp, st) = store();
        let mk = |seed: i64| -> *mut TypedObjectStorage {
            let slots =
                vec![ValueSlot::from_int(seed), ValueSlot::from_raw(0)].into_boxed_slice();
            let field_kinds: Arc<[NativeKind]> =
                vec![NativeKind::Int64, NativeKind::Ptr(HeapKind::TypedObject)].into();
            TypedObjectStorage::_new(9, slots, 0, field_kinds)
        };
        let a = mk(1); // rc 1 (holder)
        let b = mk(2); // rc 1 (holder)
        unsafe {
            // A.next = B (B's forward-edge share) ; B.next = A (A's back-edge share)
            v2_retain(b as *const HeapHeader);
            let _ = TypedObjectStorage::write_slot_in_place(a, 1, b as u64);
            *std::ptr::addr_of_mut!((*a).heap_mask) = 1 << 1;
            v2_retain(a as *const HeapHeader);
            let _ = TypedObjectStorage::write_slot_in_place(b, 1, a as u64);
            *std::ptr::addr_of_mut!((*b).heap_mask) = 1 << 1;
        }

        // SERIALIZE from A — must terminate.
        let mut ictx = SerializeIdentityCtx::new();
        let sv = slot_to_serializable_ctx(
            a as u64,
            NativeKind::Ptr(HeapKind::TypedObject),
            &st,
            &mut ictx,
        )
        .expect("serialize mutual cycle (no infinite recursion)");
        // HeapNode(hA){TO{Int(1), HeapNode(hB){TO{Int(2), HeapRef(hA)}}}}
        let h_a = match &sv {
            SV::HeapNode { handle, body } => match &**body {
                SV::TypedObject { slot_data, .. } => {
                    match &slot_data[1] {
                        SV::HeapNode { body: bb, .. } => match &**bb {
                            SV::TypedObject { slot_data: sd_b, .. } => match &sd_b[1] {
                                SV::HeapRef { handle: hb } => assert_eq!(
                                    hb, handle,
                                    "B.next back-edges to A's handle (mutual cycle broken)"
                                ),
                                other => panic!("expected HeapRef to A, got {other:?}"),
                            },
                            other => panic!("expected nested TypedObject(B), got {other:?}"),
                        },
                        other => panic!("expected nested HeapNode(B), got {other:?}"),
                    }
                    *handle
                }
                other => panic!("expected TypedObject body, got {other:?}"),
            },
            other => panic!("expected HeapNode, got {other:?}"),
        };
        let _ = h_a;

        // RESTORE via the two-pass driver.
        let mut link = RestoreLinkCtx::new();
        materialize_cell_bodies(&sv, &st, &mut link).expect("pass 1");
        let (ra, rkind) = serializable_to_slot_ctx(
            &sv,
            NativeKind::Ptr(HeapKind::TypedObject),
            &st,
            &mut link,
        )
        .expect("pass 2");
        assert_eq!(rkind, NativeKind::Ptr(HeapKind::TypedObject));

        // IDENTITY: A'->B'->A', two distinct allocations, one cycle.
        let ra_ptr = ra as *const TypedObjectStorage;
        let rb = unsafe { (*ra_ptr).slots()[1].raw() };
        let rb_ptr = rb as *const TypedObjectStorage;
        assert_ne!(ra, rb, "A' and B' are distinct allocations (no collapse)");
        let rb_next = unsafe { (*rb_ptr).slots()[1].raw() };
        assert_eq!(
            rb_next, ra,
            "B'.next aliases A' — the mutual cycle's identity is preserved"
        );
        assert_eq!(unsafe { (*ra_ptr).slots()[0].raw() }, 1, "A' scalar survives");
        assert_eq!(unsafe { (*rb_ptr).slots()[0].raw() }, 2, "B' scalar survives");

        link.release_base_shares();
        // A' rc2 {external ra, B'->A' back-edge}; B' rc1 {A'->B' forward-edge}.
        assert_eq!(rc(ra), 2, "A': external slot + B's back-edge");
        assert_eq!(rc(rb), 1, "B': A's forward-edge only (base released)");

        // Teardown: break every edge first (heap_mask->0, next->0) so no drop
        // walk touches a freed peer, then retire shares to 0. Counts gated by
        // the rc asserts above, so a miscount fails an assert, never crashes.
        unsafe {
            let rap = ra as *mut TypedObjectStorage;
            let rbp = rb as *mut TypedObjectStorage;
            *std::ptr::addr_of_mut!((*rap).heap_mask) = 0;
            let _ = TypedObjectStorage::write_slot_in_place(rap, 1, 0);
            *std::ptr::addr_of_mut!((*rbp).heap_mask) = 0;
            let _ = TypedObjectStorage::write_slot_in_place(rbp, 1, 0);
            TypedObjectStorage::release_elem(ra_ptr);
            TypedObjectStorage::release_elem(ra_ptr);
            TypedObjectStorage::release_elem(rb_ptr);

            *std::ptr::addr_of_mut!((*a).heap_mask) = 0;
            let _ = TypedObjectStorage::write_slot_in_place(a, 1, 0);
            *std::ptr::addr_of_mut!((*b).heap_mask) = 0;
            let _ = TypedObjectStorage::write_slot_in_place(b, 1, 0);
            TypedObjectStorage::release_elem(a);
            TypedObjectStorage::release_elem(a);
            TypedObjectStorage::release_elem(b);
            TypedObjectStorage::release_elem(b);
        }
    }
}

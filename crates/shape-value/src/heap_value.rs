//! Heap-allocated value types reachable through `HeapValue`.
//!
//! After the strict-typing Phase-2 bulldozer (option C — heterogeneous
//! collections / dynamic single-value wrappers excised), `HeapValue` carries
//! only typed payloads:
//!
//! - typed primitives (string, decimal, bigint, char, future-id),
//! - typed handles (datatable, content, instant, io-handle, native scalars),
//! - typed object slots (`TypedObject` with `Box<[ValueSlot]>`),
//! - the typed-closure-raw block (`ClosureRaw`),
//! - typed array buckets (`TypedArrayData`),
//! - typed temporal data (`TemporalData`),
//! - typed table views (`TableViewData`).
//!
//! Variants that previously held `ValueWord` (the deleted dynamic word) —
//! `Some`/`Ok`/`Err`/`Range`/`TraitObject`/`FunctionRef`,
//! `HashMap`/`Set`/`Deque`/`PriorityQueue`, `Iterator`/`Generator`/
//! `ProjectedRef`, `Concurrency` (Mutex/Atomic/Lazy/Channel), `Rare`,
//! `Enum`, `Array` (heterogeneous-element), `HostClosure` — were removed
//! together with their `*Data` structs. The corresponding `HeapKind`
//! ordinals are preserved (annotated "(removed)" in `heap_variants.rs`)
//! and await monomorphized typed replacements per `docs/runtime-v2-spec.md`.

use crate::aligned_vec::AlignedVec;
use std::fmt;
use std::sync::Arc;

// ── Matrix storage (used by TypedArrayData::Matrix and FloatSlice) ──────────

/// Flat, SIMD-aligned matrix storage (row-major order).
#[derive(Debug, Clone)]
pub struct MatrixData {
    pub data: AlignedVec<f64>,
    pub rows: u32,
    pub cols: u32,
}

impl MatrixData {
    /// Create a zero-initialized matrix.
    pub fn new(rows: u32, cols: u32) -> Self {
        let len = (rows as usize) * (cols as usize);
        let mut data = AlignedVec::with_capacity(len);
        for _ in 0..len {
            data.push(0.0);
        }
        Self { data, rows, cols }
    }

    /// Create from a flat data buffer.
    pub fn from_flat(data: AlignedVec<f64>, rows: u32, cols: u32) -> Self {
        debug_assert_eq!(data.len(), (rows as usize) * (cols as usize));
        Self { data, rows, cols }
    }

    /// Get element at (row, col).
    #[inline]
    pub fn get(&self, row: u32, col: u32) -> f64 {
        self.data[(row as usize) * (self.cols as usize) + (col as usize)]
    }

    /// Set element at (row, col).
    #[inline]
    pub fn set(&mut self, row: u32, col: u32, val: f64) {
        self.data[(row as usize) * (self.cols as usize) + (col as usize)] = val;
    }

    /// Get a row slice.
    #[inline]
    pub fn row_slice(&self, row: u32) -> &[f64] {
        let start = (row as usize) * (self.cols as usize);
        &self.data[start..start + self.cols as usize]
    }

    /// Get shape as (rows, cols).
    #[inline]
    pub fn shape(&self) -> (u32, u32) {
        (self.rows, self.cols)
    }

    /// Get a row's data as a slice (alias for `row_slice`).
    #[inline]
    pub fn row_data(&self, row: u32) -> &[f64] {
        self.row_slice(row)
    }
}

// ── NativeScalar — width-preserving native ABI scalars ──────────────────────

/// Native ABI-width scalars used by C ABI / `extern C fn` boundaries.
///
/// These values preserve their ABI width across VM boundaries so C wrappers
/// can avoid lossy `i64` normalization.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NativeScalar {
    I8(i8),
    U8(u8),
    I16(i16),
    U16(u16),
    I32(i32),
    I64(i64),
    U32(u32),
    U64(u64),
    Isize(isize),
    Usize(usize),
    Ptr(usize),
    F32(f32),
}

impl NativeScalar {
    #[inline]
    pub fn type_name(&self) -> &'static str {
        match self {
            NativeScalar::I8(_) => "i8",
            NativeScalar::U8(_) => "u8",
            NativeScalar::I16(_) => "i16",
            NativeScalar::U16(_) => "u16",
            NativeScalar::I32(_) => "i32",
            NativeScalar::I64(_) => "i64",
            NativeScalar::U32(_) => "u32",
            NativeScalar::U64(_) => "u64",
            NativeScalar::Isize(_) => "isize",
            NativeScalar::Usize(_) => "usize",
            NativeScalar::Ptr(_) => "ptr",
            NativeScalar::F32(_) => "f32",
        }
    }

    #[inline]
    pub fn is_truthy(&self) -> bool {
        match self {
            NativeScalar::I8(v) => *v != 0,
            NativeScalar::U8(v) => *v != 0,
            NativeScalar::I16(v) => *v != 0,
            NativeScalar::U16(v) => *v != 0,
            NativeScalar::I32(v) => *v != 0,
            NativeScalar::I64(v) => *v != 0,
            NativeScalar::U32(v) => *v != 0,
            NativeScalar::U64(v) => *v != 0,
            NativeScalar::Isize(v) => *v != 0,
            NativeScalar::Usize(v) => *v != 0,
            NativeScalar::Ptr(v) => *v != 0,
            NativeScalar::F32(v) => *v != 0.0 && !v.is_nan(),
        }
    }

    #[inline]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            NativeScalar::I8(v) => Some(*v as i64),
            NativeScalar::U8(v) => Some(*v as i64),
            NativeScalar::I16(v) => Some(*v as i64),
            NativeScalar::U16(v) => Some(*v as i64),
            NativeScalar::I32(v) => Some(*v as i64),
            NativeScalar::I64(v) => Some(*v),
            NativeScalar::U32(v) => Some(*v as i64),
            NativeScalar::U64(v) => i64::try_from(*v).ok(),
            NativeScalar::Isize(v) => i64::try_from(*v).ok(),
            NativeScalar::Usize(v) => i64::try_from(*v).ok(),
            NativeScalar::Ptr(v) => i64::try_from(*v).ok(),
            NativeScalar::F32(_) => None,
        }
    }

    #[inline]
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            NativeScalar::U8(v) => Some(*v as u64),
            NativeScalar::U16(v) => Some(*v as u64),
            NativeScalar::U32(v) => Some(*v as u64),
            NativeScalar::U64(v) => Some(*v),
            NativeScalar::Usize(v) => Some(*v as u64),
            NativeScalar::Ptr(v) => Some(*v as u64),
            NativeScalar::I8(v) if *v >= 0 => Some(*v as u64),
            NativeScalar::I16(v) if *v >= 0 => Some(*v as u64),
            NativeScalar::I32(v) if *v >= 0 => Some(*v as u64),
            NativeScalar::I64(v) if *v >= 0 => Some(*v as u64),
            NativeScalar::Isize(v) if *v >= 0 => Some(*v as u64),
            _ => None,
        }
    }

    #[inline]
    pub fn as_i128(&self) -> Option<i128> {
        match self {
            NativeScalar::I8(v) => Some(*v as i128),
            NativeScalar::U8(v) => Some(*v as i128),
            NativeScalar::I16(v) => Some(*v as i128),
            NativeScalar::U16(v) => Some(*v as i128),
            NativeScalar::I32(v) => Some(*v as i128),
            NativeScalar::U32(v) => Some(*v as i128),
            NativeScalar::I64(v) => Some(*v as i128),
            NativeScalar::U64(v) => Some(*v as i128),
            NativeScalar::Isize(v) => Some(*v as i128),
            NativeScalar::Usize(v) => Some(*v as i128),
            NativeScalar::Ptr(v) => Some(*v as i128),
            NativeScalar::F32(_) => None,
        }
    }

    #[inline]
    pub fn as_f64(&self) -> f64 {
        match self {
            NativeScalar::I8(v) => *v as f64,
            NativeScalar::U8(v) => *v as f64,
            NativeScalar::I16(v) => *v as f64,
            NativeScalar::U16(v) => *v as f64,
            NativeScalar::I32(v) => *v as f64,
            NativeScalar::I64(v) => *v as f64,
            NativeScalar::U32(v) => *v as f64,
            NativeScalar::U64(v) => *v as f64,
            NativeScalar::Isize(v) => *v as f64,
            NativeScalar::Usize(v) => *v as f64,
            NativeScalar::Ptr(v) => *v as f64,
            NativeScalar::F32(v) => *v as f64,
        }
    }
}

impl std::fmt::Display for NativeScalar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NativeScalar::I8(v) => write!(f, "{v}"),
            NativeScalar::U8(v) => write!(f, "{v}"),
            NativeScalar::I16(v) => write!(f, "{v}"),
            NativeScalar::U16(v) => write!(f, "{v}"),
            NativeScalar::I32(v) => write!(f, "{v}"),
            NativeScalar::I64(v) => write!(f, "{v}"),
            NativeScalar::U32(v) => write!(f, "{v}"),
            NativeScalar::U64(v) => write!(f, "{v}"),
            NativeScalar::Isize(v) => write!(f, "{v}"),
            NativeScalar::Usize(v) => write!(f, "{v}"),
            NativeScalar::Ptr(v) => write!(f, "0x{v:x}"),
            NativeScalar::F32(v) => write!(f, "{v}"),
        }
    }
}

// ── Native type layouts (used by C ABI native views) ─────────────────────────

/// Field layout metadata for `type C` structs.
#[derive(Debug, Clone)]
pub struct NativeLayoutField {
    pub name: String,
    pub c_type: String,
    pub offset: u32,
    pub size: u32,
    pub align: u32,
}

/// Runtime layout descriptor for one native type.
#[derive(Debug, Clone)]
pub struct NativeTypeLayout {
    pub name: String,
    pub abi: String,
    pub size: u32,
    pub align: u32,
    pub fields: Vec<NativeLayoutField>,
}

impl NativeTypeLayout {
    #[inline]
    pub fn field(&self, name: &str) -> Option<&NativeLayoutField> {
        self.fields.iter().find(|field| field.name == name)
    }
}

/// Pointer-backed zero-copy view into native memory.
#[derive(Debug, Clone)]
pub struct NativeViewData {
    pub ptr: usize,
    pub layout: Arc<NativeTypeLayout>,
    pub mutable: bool,
}

// ── I/O handles ──────────────────────────────────────────────────────────────

/// I/O handle kind discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IoHandleKind {
    File = 0,
    TcpStream = 1,
    TcpListener = 2,
    UdpSocket = 3,
    ChildProcess = 4,
    PipeReader = 5,
    PipeWriter = 6,
    Custom = 7,
}

/// The underlying OS resource wrapped by an IoHandle.
pub enum IoResource {
    File(std::fs::File),
    TcpStream(std::net::TcpStream),
    TcpListener(std::net::TcpListener),
    UdpSocket(std::net::UdpSocket),
    ChildProcess(std::process::Child),
    PipeReader(std::process::ChildStdout),
    PipeWriter(std::process::ChildStdin),
    PipeReaderErr(std::process::ChildStderr),
    /// Type-erased resource for custom I/O handles (e.g. memoized transports).
    Custom(Box<dyn std::any::Any + Send>),
}

impl std::fmt::Debug for IoResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IoResource::File(_) => write!(f, "File(...)"),
            IoResource::TcpStream(_) => write!(f, "TcpStream(...)"),
            IoResource::TcpListener(_) => write!(f, "TcpListener(...)"),
            IoResource::UdpSocket(_) => write!(f, "UdpSocket(...)"),
            IoResource::ChildProcess(_) => write!(f, "ChildProcess(...)"),
            IoResource::PipeReader(_) => write!(f, "PipeReader(...)"),
            IoResource::PipeWriter(_) => write!(f, "PipeWriter(...)"),
            IoResource::PipeReaderErr(_) => write!(f, "PipeReaderErr(...)"),
            IoResource::Custom(_) => write!(f, "Custom(...)"),
        }
    }
}

/// Data for IoHandle variant (Arc-wrapped at the HeapValue level to keep
/// HeapValue small and to enable cluster #2 marshal `FromSlot for
/// Arc<IoHandleData>`).
///
/// Wraps an OS resource (file, socket, process) in an Arc<Mutex<Option<IoResource>>>
/// so it can be shared and closed. The `Option` is `None` after close().
/// Rust's `Drop` closes the underlying resource if not already closed.
///
/// Storage: `HeapValue::IoHandle(Arc<IoHandleData>)`. The variant Arc is
/// the marshal-layer's typed handle (per cluster #2 option γ in
/// `docs/defections.md` 2026-05-06); the inner `Arc<Mutex<...>>` is the
/// shared resource lock. Cloning the variant is one atomic op.
#[derive(Clone)]
pub struct IoHandleData {
    pub kind: IoHandleKind,
    pub resource: Arc<std::sync::Mutex<Option<IoResource>>>,
    pub path: String,
    pub mode: String,
}

impl std::fmt::Debug for IoHandleData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IoHandleData")
            .field("kind", &self.kind)
            .field("path", &self.path)
            .field("mode", &self.mode)
            .field(
                "open",
                &self.resource.lock().map(|g| g.is_some()).unwrap_or(false),
            )
            .finish()
    }
}

impl IoHandleData {
    /// Create a new file handle.
    pub fn new_file(file: std::fs::File, path: String, mode: String) -> Self {
        Self {
            kind: IoHandleKind::File,
            resource: Arc::new(std::sync::Mutex::new(Some(IoResource::File(file)))),
            path,
            mode,
        }
    }

    /// Create a new TCP stream handle.
    pub fn new_tcp_stream(stream: std::net::TcpStream, addr: String) -> Self {
        Self {
            kind: IoHandleKind::TcpStream,
            resource: Arc::new(std::sync::Mutex::new(Some(IoResource::TcpStream(stream)))),
            path: addr,
            mode: "rw".to_string(),
        }
    }

    /// Create a new TCP listener handle.
    pub fn new_tcp_listener(listener: std::net::TcpListener, addr: String) -> Self {
        Self {
            kind: IoHandleKind::TcpListener,
            resource: Arc::new(std::sync::Mutex::new(Some(IoResource::TcpListener(
                listener,
            )))),
            path: addr,
            mode: "listen".to_string(),
        }
    }

    /// Create a new UDP socket handle.
    pub fn new_udp_socket(socket: std::net::UdpSocket, addr: String) -> Self {
        Self {
            kind: IoHandleKind::UdpSocket,
            resource: Arc::new(std::sync::Mutex::new(Some(IoResource::UdpSocket(socket)))),
            path: addr,
            mode: "rw".to_string(),
        }
    }

    /// Create a handle wrapping a spawned child process.
    pub fn new_child_process(child: std::process::Child, cmd: String) -> Self {
        Self {
            kind: IoHandleKind::ChildProcess,
            resource: Arc::new(std::sync::Mutex::new(Some(IoResource::ChildProcess(child)))),
            path: cmd,
            mode: "process".to_string(),
        }
    }

    /// Create a handle wrapping a child stdout pipe.
    pub fn new_pipe_reader(stdout: std::process::ChildStdout, label: String) -> Self {
        Self {
            kind: IoHandleKind::PipeReader,
            resource: Arc::new(std::sync::Mutex::new(Some(IoResource::PipeReader(stdout)))),
            path: label,
            mode: "r".to_string(),
        }
    }

    /// Create a handle wrapping a child stdin pipe.
    pub fn new_pipe_writer(stdin: std::process::ChildStdin, label: String) -> Self {
        Self {
            kind: IoHandleKind::PipeWriter,
            resource: Arc::new(std::sync::Mutex::new(Some(IoResource::PipeWriter(stdin)))),
            path: label,
            mode: "w".to_string(),
        }
    }

    /// Create a handle wrapping a child stderr pipe.
    pub fn new_pipe_reader_err(stderr: std::process::ChildStderr, label: String) -> Self {
        Self {
            kind: IoHandleKind::PipeReader,
            resource: Arc::new(std::sync::Mutex::new(Some(IoResource::PipeReaderErr(
                stderr,
            )))),
            path: label,
            mode: "r".to_string(),
        }
    }

    /// Create a handle wrapping a custom type-erased resource.
    pub fn new_custom(resource: Box<dyn std::any::Any + Send>, label: String) -> Self {
        Self {
            kind: IoHandleKind::Custom,
            resource: Arc::new(std::sync::Mutex::new(Some(IoResource::Custom(resource)))),
            path: label,
            mode: "custom".to_string(),
        }
    }

    /// Check if the handle is still open.
    pub fn is_open(&self) -> bool {
        self.resource.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    /// Close the handle, returning true if it was open.
    pub fn close(&self) -> bool {
        if let Ok(mut guard) = self.resource.lock() {
            guard.take().is_some()
        } else {
            false
        }
    }
}

// ── HashMap storage (Stage C P1(b), 2026-05-07) ─────────────────────────────

/// HashMap storage — two parallel Phase 2d Array buffers (string keys +
/// heap-allocated values) plus an eager bucket-index for O(1) lookup.
///
/// Stage C HashMap-marshal P1(b) per supervisor sign-off. Reuses Phase
/// 2d Array's `TypedBuffer<Arc<String>>` and `TypedBuffer<Arc<HeapValue>>`
/// shapes verbatim — no new buffer-storage type. Insertion order is the
/// canonical storage; the `index` is a sidecar acceleration structure for
/// `executor/objects/hashmap_methods.rs`-style O(1) lookup.
///
/// **Eager bucket-only at first landing** (per supervisor sign-off):
/// `index` is built at construction and maintained incrementally on
/// insert/remove. The `shape_id` hidden-class fast-path that the
/// pre-bulldozer arch used for ≤64-string-keyed-maps is **deferred to a
/// separate optimization workstream** — refused here as
/// architectural-decision-bundling per supervisor watchlist.
///
/// Element-type discrimination is body-side via Rust types: `FromSlot`
/// impls for `Vec<(Arc<String>, Arc<String>)>` (string-string) and
/// `Vec<(Arc<String>, Arc<HeapValue>)>` (polymorphic-value) both decode
/// the same `HeapValue::HashMap` slot, with the Vec element type pinning
/// which payload pattern the body expects. Same option ε pattern as
/// Phase 2d Array's `Vec<Arc<String>>` / `Vec<Arc<HeapValue>>` impls.
#[derive(Debug)]
pub struct HashMapData {
    /// Insertion-ordered keys (string-typed buffer).
    pub keys: Arc<crate::typed_buffer::TypedBuffer<Arc<String>>>,
    /// Insertion-ordered values (heap-allocated, polymorphic at the
    /// HeapValue arm — body-side `FromSlot` impl pins the element shape).
    pub values: Arc<crate::typed_buffer::TypedBuffer<Arc<HeapValue>>>,
    /// Eager bucket-index: hash → list of indices into `keys`/`values`
    /// arrays. Enables O(1) lookup at the user-facing `map.get(key)`
    /// path. Hash is computed via FNV-1a over the key string bytes.
    pub index: std::collections::HashMap<u64, Vec<u32>>,
}

impl HashMapData {
    /// Build an empty HashMapData with no entries.
    pub fn new() -> Self {
        Self {
            keys: Arc::new(crate::typed_buffer::TypedBuffer::from_vec(Vec::new())),
            values: Arc::new(crate::typed_buffer::TypedBuffer::from_vec(Vec::new())),
            index: std::collections::HashMap::new(),
        }
    }

    /// Build from parallel `Vec`s of keys and values, computing the
    /// bucket index eagerly. Panics if `keys.len() != values.len()`.
    pub fn from_pairs(keys: Vec<Arc<String>>, values: Vec<Arc<HeapValue>>) -> Self {
        assert_eq!(
            keys.len(),
            values.len(),
            "HashMapData::from_pairs: keys/values length mismatch"
        );
        let mut index: std::collections::HashMap<u64, Vec<u32>> =
            std::collections::HashMap::new();
        for (i, k) in keys.iter().enumerate() {
            index
                .entry(fnv1a_hash(k.as_bytes()))
                .or_default()
                .push(i as u32);
        }
        Self {
            keys: Arc::new(crate::typed_buffer::TypedBuffer::from_vec(keys)),
            values: Arc::new(crate::typed_buffer::TypedBuffer::from_vec(values)),
            index,
        }
    }

    /// Number of entries.
    #[inline]
    pub fn len(&self) -> usize {
        self.keys.data.len()
    }

    /// Whether the map is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.keys.data.is_empty()
    }

    /// Look up a value by string key. O(1) via the bucket index plus a
    /// short bucket scan for collision disambiguation.
    pub fn get(&self, key: &str) -> Option<&Arc<HeapValue>> {
        let hash = fnv1a_hash(key.as_bytes());
        let bucket = self.index.get(&hash)?;
        for &idx in bucket {
            let i = idx as usize;
            if self.keys.data[i].as_str() == key {
                return Some(&self.values.data[i]);
            }
        }
        None
    }

    /// Whether the map contains the given key.
    #[inline]
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    // ── Mutation API (W13-hashmap-mutation, 2026-05-10) ─────────────────────
    //
    // The post-§2.7.4 storage shape (`Arc<TypedBuffer<Arc<String>>>` keys +
    // `Arc<TypedBuffer<Arc<HeapValue>>>` values + eager bucket-index)
    // already has refcount-aware payload types: `Arc<String>` keys and
    // `Arc<HeapValue>` values both clone via a single atomic refcount bump
    // and drop via `Arc::decrement_strong_count` — no parallel `NativeKind`
    // track is needed at this layer (§2.7.7's parallel-kind invariant
    // applies to the kinded stack, not to typed `Arc<HeapValue>` payloads,
    // which carry their own discriminator via `HeapValue::kind()`).
    //
    // The mutation entry-points therefore take `Arc<String>` / `Arc<HeapValue>`
    // directly. The `shape-vm`-side handlers (`v2_set` / `v2_delete` /
    // `v2_merge` in `executor/objects/hashmap_methods.rs`) project the
    // `KindedSlot` carrier args into these typed Arcs via the existing
    // `result_slot_to_heap_value_arc` / `as_string_key` helpers, then
    // `Arc::make_mut` the receiver `Arc<HashMapData>` so that shared
    // references stay immutable (clone-on-write).

    /// Insert or overwrite a key/value entry. If the key already exists,
    /// the value is replaced in-place (the old `Arc<HeapValue>` is dropped,
    /// releasing its refcount). Otherwise the entry is appended to the
    /// insertion-ordered buffers and registered in the bucket index.
    pub fn insert(&mut self, key: Arc<String>, value: Arc<HeapValue>) {
        let hash = fnv1a_hash(key.as_bytes());
        // Look for an existing entry under the same hash bucket.
        if let Some(bucket) = self.index.get(&hash) {
            for &idx in bucket {
                let i = idx as usize;
                if self.keys.data[i].as_str() == key.as_str() {
                    // Overwrite in place. `Arc::make_mut` on the values
                    // buffer is the §2.7.4 / playbook clone-on-write path:
                    // a shared buffer is cloned (cheap — element-wise Arc
                    // bumps), a uniquely-owned buffer is mutated directly.
                    let values_buf = Arc::make_mut(&mut self.values);
                    values_buf.data[i] = value;
                    return;
                }
            }
        }
        // New entry: append to keys + values, then register in the index.
        let new_idx = self.keys.data.len();
        Arc::make_mut(&mut self.keys).data.push(key);
        Arc::make_mut(&mut self.values).data.push(value);
        self.index.entry(hash).or_default().push(new_idx as u32);
    }

    /// Remove the entry under `key`. Returns `true` if the key was present
    /// (and removed), `false` if no entry existed. The bucket index is
    /// updated to reflect the buffer's post-removal indices: every entry
    /// after the removed slot shifts down by one position.
    pub fn remove(&mut self, key: &str) -> bool {
        let hash = fnv1a_hash(key.as_bytes());
        // Locate the position within the bucket whose stored key matches.
        let removed_idx: usize = {
            let Some(bucket) = self.index.get(&hash) else {
                return false;
            };
            let mut found: Option<usize> = None;
            for (bucket_pos, &idx) in bucket.iter().enumerate() {
                if self.keys.data[idx as usize].as_str() == key {
                    found = Some(bucket_pos);
                    break;
                }
            }
            let bucket_pos = match found {
                Some(p) => p,
                None => return false,
            };
            // Take the index, drop the bucket borrow before re-borrowing
            // the index mutably below.
            let bucket = self.index.get_mut(&hash).expect("bucket present");
            let removed_idx = bucket.swap_remove(bucket_pos) as usize;
            if bucket.is_empty() {
                self.index.remove(&hash);
            }
            removed_idx
        };
        // Remove from the parallel buffers via Arc::make_mut.
        Arc::make_mut(&mut self.keys).data.remove(removed_idx);
        Arc::make_mut(&mut self.values).data.remove(removed_idx);
        // Shift down every index in the bucket map that pointed at a
        // position past `removed_idx`.
        for bucket in self.index.values_mut() {
            for slot in bucket.iter_mut() {
                if (*slot as usize) > removed_idx {
                    *slot -= 1;
                }
            }
        }
        true
    }

    /// Merge entries from `other` into `self`. Keys present in both maps
    /// take the value from `other` (last-write-wins, matching `Object.assign`
    /// / `dict.update` semantics). Per-entry insert path — the bucket index
    /// is maintained incrementally.
    pub fn merge(&mut self, other: &HashMapData) {
        let n = other.len();
        for i in 0..n {
            let key = Arc::clone(&other.keys.data[i]);
            let value = Arc::clone(&other.values.data[i]);
            self.insert(key, value);
        }
    }
}

impl Default for HashMapData {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for HashMapData {
    fn clone(&self) -> Self {
        // Arc::clone on the buffers (shared structural sharing is fine —
        // HashMapData is treated as immutable at the marshal boundary;
        // mutation goes through Arc::make_mut on the shape-vm side, out
        // of this crate's scope).
        Self {
            keys: Arc::clone(&self.keys),
            values: Arc::clone(&self.values),
            index: self.index.clone(),
        }
    }
}

// ── HashSet storage (Wave 13 W13-hashset-rebuild, 2026-05-10) ───────────────

/// HashSet storage — one keyspace, no values. Mirror of `HashMapData`
/// with the values buffer dropped.
///
/// ADR-006 §2.7.15 / Q16 amendment (mirror of §2.7.9 FilterExpr / §2.7.13
/// Reference precedent for the cardinality-amendment shape, but Set is
/// a HashMap *sibling* — full `HeapValue::HashSet` arm rather than
/// pure-discriminator). Reuses the Stage C P1(b) Phase 2d Array shape
/// (`TypedBuffer<Arc<String>>`) for the keys buffer verbatim. Insertion
/// order is the canonical storage; the `index` is a sidecar acceleration
/// structure for O(1) `set.has(key)`.
///
/// **String-only keyspace at landing** (per the W9-set-methods owner
/// audit's Path A scope and the §2.7.15 Q16 ruling). Heterogeneous-
/// element keysets (int-keyed, TypedObject-keyed) are explicitly
/// out-of-scope; the Path B (`TypedSet<T>` per element kind) rebuild
/// is a future Phase-2c amendment with measurement.
#[derive(Debug)]
pub struct HashSetData {
    /// Insertion-ordered keys (string-typed buffer).
    pub keys: Arc<crate::typed_buffer::TypedBuffer<Arc<String>>>,
    /// Eager bucket-index: hash → list of indices into `keys` array.
    /// Enables O(1) lookup at `set.has(key)`. Hash is FNV-1a over the
    /// key string bytes — same as `HashMapData::index`.
    pub index: std::collections::HashMap<u64, Vec<u32>>,
}

impl HashSetData {
    /// Build an empty HashSetData with no entries.
    pub fn new() -> Self {
        Self {
            keys: Arc::new(crate::typed_buffer::TypedBuffer::from_vec(Vec::new())),
            index: std::collections::HashMap::new(),
        }
    }

    /// Build from a `Vec<Arc<String>>` of keys, computing the bucket
    /// index eagerly. Duplicate keys in the input are collapsed
    /// (insertion-order preserved, first occurrence wins).
    pub fn from_keys(keys: Vec<Arc<String>>) -> Self {
        let mut out = Self::new();
        for k in keys {
            out.insert(k);
        }
        out
    }

    /// Number of entries.
    #[inline]
    pub fn len(&self) -> usize {
        self.keys.data.len()
    }

    /// Whether the set is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.keys.data.is_empty()
    }

    /// Whether the set contains the given key. O(1) via the bucket
    /// index plus a short bucket scan for collision disambiguation.
    pub fn contains(&self, key: &str) -> bool {
        let hash = fnv1a_hash(key.as_bytes());
        let Some(bucket) = self.index.get(&hash) else {
            return false;
        };
        for &idx in bucket {
            let i = idx as usize;
            if self.keys.data[i].as_str() == key {
                return true;
            }
        }
        false
    }

    // ── Mutation API (Wave 13 W13-hashset-rebuild, 2026-05-10) ──────────────
    //
    // Mirror of HashMapData's W13-hashmap-mutation API with the values
    // buffer dropped. `Arc::make_mut` clone-on-write over the inner
    // `Arc<TypedBuffer<Arc<String>>>` keys plus parallel bucket-index
    // maintenance — same shape, one less buffer to mutate.

    /// Insert a key. Returns `true` if the key was newly added,
    /// `false` if it was already present (no-op in the latter case).
    pub fn insert(&mut self, key: Arc<String>) -> bool {
        let hash = fnv1a_hash(key.as_bytes());
        if let Some(bucket) = self.index.get(&hash) {
            for &idx in bucket {
                let i = idx as usize;
                if self.keys.data[i].as_str() == key.as_str() {
                    return false;
                }
            }
        }
        let new_idx = self.keys.data.len();
        Arc::make_mut(&mut self.keys).data.push(key);
        self.index.entry(hash).or_default().push(new_idx as u32);
        true
    }

    /// Remove the entry under `key`. Returns `true` if the key was
    /// present (and removed), `false` if no entry existed. The bucket
    /// index is updated to reflect the buffer's post-removal indices:
    /// every entry after the removed slot shifts down by one position
    /// (mirror of `HashMapData::remove`).
    pub fn remove(&mut self, key: &str) -> bool {
        let hash = fnv1a_hash(key.as_bytes());
        let removed_idx: usize = {
            let Some(bucket) = self.index.get(&hash) else {
                return false;
            };
            let mut found: Option<usize> = None;
            for (bucket_pos, &idx) in bucket.iter().enumerate() {
                if self.keys.data[idx as usize].as_str() == key {
                    found = Some(bucket_pos);
                    break;
                }
            }
            let bucket_pos = match found {
                Some(p) => p,
                None => return false,
            };
            let bucket = self.index.get_mut(&hash).expect("bucket present");
            let removed_idx = bucket.swap_remove(bucket_pos) as usize;
            if bucket.is_empty() {
                self.index.remove(&hash);
            }
            removed_idx
        };
        Arc::make_mut(&mut self.keys).data.remove(removed_idx);
        for bucket in self.index.values_mut() {
            for slot in bucket.iter_mut() {
                if (*slot as usize) > removed_idx {
                    *slot -= 1;
                }
            }
        }
        true
    }
}

impl Default for HashSetData {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for HashSetData {
    fn clone(&self) -> Self {
        Self {
            keys: Arc::clone(&self.keys),
            index: self.index.clone(),
        }
    }
}

/// FNV-1a hash for byte slices. Matches the `v2/typed_map.rs` hash
/// function so that key-hash semantics are consistent across the
/// HashMap-marshal layer and any future cross-cluster perf path.
#[inline]
fn fnv1a_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

// ── Deque storage (W15-deque, ADR-006 §2.7.19 / Q20, 2026-05-10) ───────────

/// Double-ended queue storage. Heterogeneous element kinds are stored as
/// `Arc<HeapValue>` payloads (mirror of `HashMapData::values` per ADR-005
/// §1 single-discriminator) — the deque is element-kind-agnostic at
/// landing, in line with the W13-hashmap precedent.
///
/// ADR-006 §2.7.19 / Q20 amendment (Wave 15 W15-deque, 2026-05-10).
/// Mirror of the §2.7.15 HashSet shape (full `HeapValue::Deque` arm,
/// NOT pure-discriminator like FilterExpr / SharedCell): receivers
/// flow through `slot.as_heap_value()` for receiver classification at
/// method dispatch (`d.pushBack(...)` / `d.popFront()` / `d.size()`).
///
/// **Heterogeneous-element keyspace at landing.** Element kinds that
/// can be heap-wrapped (string / int via `BigInt(Arc<i64>)` / typed
/// arrays / typed objects / hashmaps / etc.) are accepted by the
/// mutation API; bare `Float64` / `Bool` results are rejected (no
/// matching `HeapValue::*` arm exists post-§2.3). Same coverage shape
/// as `HashMapData::values` storage (`hashmap_methods.rs::
/// result_slot_to_heap_value_arc`).
///
/// Per the W15-deque audit: `VecDeque<Arc<HeapValue>>` chosen over the
/// alternative `Vec<u64>` + parallel `Vec<NativeKind>` (per §2.7.7
/// stack ABI) — Deque is heterogeneous-element, not scalar-only, so
/// the parallel-kind track shape would force every push site to
/// carry both bits and kind through the deque API. The
/// `Arc<HeapValue>` shape collapses both into a single payload at the
/// element tier and matches the Stage C P1(b) HashMap precedent.
#[derive(Debug)]
pub struct DequeData {
    /// Insertion-ordered double-ended queue of heap-allocated element
    /// payloads. Element kinds are recovered via the canonical ADR-005
    /// §1 single-discriminator `HeapValue` match at the read site.
    pub items: std::collections::VecDeque<Arc<HeapValue>>,
}

impl DequeData {
    /// Build an empty DequeData with no elements.
    pub fn new() -> Self {
        Self {
            items: std::collections::VecDeque::new(),
        }
    }

    /// Build from a `Vec<Arc<HeapValue>>`. Insertion order is the
    /// front-to-back walk order.
    pub fn from_items(items: Vec<Arc<HeapValue>>) -> Self {
        Self {
            items: std::collections::VecDeque::from(items),
        }
    }

    /// Number of elements.
    #[inline]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the deque is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Borrow the front element without removing it. `None` when empty.
    pub fn peek_front(&self) -> Option<&Arc<HeapValue>> {
        self.items.front()
    }

    /// Borrow the back element without removing it. `None` when empty.
    pub fn peek_back(&self) -> Option<&Arc<HeapValue>> {
        self.items.back()
    }

    /// Borrow the element at `index` (front-counted). `None` when out
    /// of bounds.
    pub fn get(&self, index: usize) -> Option<&Arc<HeapValue>> {
        self.items.get(index)
    }

    // ── Mutation API (W15-deque, 2026-05-10) ────────────────────────────────
    //
    // Mirror of HashMapData / HashSetData clone-on-write shape — callers
    // wrap mutation in `Arc::make_mut(&mut arc).push_back(...)` so the
    // shared-receiver semantics are preserved per ADR-006 §2.7.4.

    /// Push an element onto the back of the deque.
    pub fn push_back(&mut self, value: Arc<HeapValue>) {
        self.items.push_back(value);
    }

    /// Push an element onto the front of the deque.
    pub fn push_front(&mut self, value: Arc<HeapValue>) {
        self.items.push_front(value);
    }

    /// Remove and return the back element. `None` when empty.
    pub fn pop_back(&mut self) -> Option<Arc<HeapValue>> {
        self.items.pop_back()
    }

    /// Remove and return the front element. `None` when empty.
    pub fn pop_front(&mut self) -> Option<Arc<HeapValue>> {
        self.items.pop_front()
    }
}

impl Default for DequeData {
    fn default() -> Self {
        Self::new()
    }
}

// ── Channel storage (Wave 15 W15-channel, ADR-006 §2.7.20 / Q21,
// 2026-05-10) ──────────────────────────────────────────────────────────────

/// MPSC-style synchronous channel storage.
///
/// ADR-006 §2.7.20 / Q21 amendment (Wave 15 W15-channel-rebuild,
/// 2026-05-10). Channel is a concurrency primitive; unlike the
/// HashMap/HashSet siblings (insertion-ordered immutable-on-clone
/// keys-buffer with `Arc::make_mut` clone-on-write), Channel needs
/// **interior mutability** so that two `Arc<ChannelData>` shares of
/// the same channel observe each other's `send` / `recv` mutations
/// (the producer and consumer endpoints share the same buffer). The
/// inner state therefore lives behind a `Mutex<ChannelInner>`; the
/// outer `Arc` is purely a refcount carrier.
///
/// **Sync same-thread path only at landing.** Cross-task / cross-
/// thread blocking `recv()` (the canonical async-channel use case)
/// requires integration with the §2.7.4 task-scheduler boundary
/// (`shape-vm/src/executor/task_scheduler.rs`), which is itself a
/// phase-2c surface; per the W15 playbook the async paths SURFACE
/// cleanly. The sync path (same-thread `send` then `recv`) lands
/// here end-to-end.
///
/// **Element typing.** The buffer stores `KindedSlot` payloads
/// directly so heterogeneous-element queues are first-class (a
/// channel can carry ints, strings, or typed objects without a
/// per-element-kind specialisation). Each slot owns one strong-
/// count share for heap-bearing kinds; the `KindedSlot::Drop`
/// dispatch retires shares cleanly when the channel itself drops.
/// This is the same shape `concurrency_methods.rs` (Mutex/Atomic/
/// Lazy) will use when those primitives rebuild — Channel is the
/// first concurrency primitive to land kinded.
///
/// **Closed flag.** `closed: bool` records whether the producer
/// side has signalled end-of-stream. After `close()` further
/// `send()` calls return a closed-channel error; `recv()` continues
/// to drain queued elements and only errors once the queue is
/// empty (canonical drain-on-close semantics).
#[derive(Debug)]
pub struct ChannelData {
    inner: std::sync::Mutex<ChannelInner>,
}

/// Inner mutable state of a `ChannelData`. Held under `Mutex` so
/// concurrent `Arc<ChannelData>` shares observe each other's
/// mutations.
#[derive(Debug)]
struct ChannelInner {
    /// FIFO queue of pending kinded elements.
    queue: std::collections::VecDeque<crate::kinded_slot::KindedSlot>,
    /// Producer-side end-of-stream signal. Once set, further
    /// `send()` calls return a closed-channel error and `recv()`
    /// drains remaining elements before erroring.
    closed: bool,
}

impl ChannelData {
    /// Build an empty open channel.
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(ChannelInner {
                queue: std::collections::VecDeque::new(),
                closed: false,
            }),
        }
    }

    /// Number of pending elements. Useful for diagnostics; not part
    /// of the user-facing method surface.
    pub fn len(&self) -> usize {
        self.inner.lock().expect("channel mutex poisoned").queue.len()
    }

    /// Whether the queue currently holds zero pending elements.
    pub fn is_empty(&self) -> bool {
        self.inner
            .lock()
            .expect("channel mutex poisoned")
            .queue
            .is_empty()
    }

    /// Whether `close()` has been called.
    pub fn is_closed(&self) -> bool {
        self.inner.lock().expect("channel mutex poisoned").closed
    }

    /// Append `slot` to the queue.
    ///
    /// Returns `Ok(())` on success, `Err(())` if the channel is
    /// already closed (callers surface this as a runtime error
    /// from the `send` method body).
    pub fn send(&self, slot: crate::kinded_slot::KindedSlot) -> Result<(), ()> {
        let mut inner = self.inner.lock().expect("channel mutex poisoned");
        if inner.closed {
            // Drop the slot — its share retires through KindedSlot::Drop.
            drop(slot);
            return Err(());
        }
        inner.queue.push_back(slot);
        Ok(())
    }

    /// Pop the front element non-blocking.
    ///
    /// Returns `Some(slot)` if an element was available, `None`
    /// otherwise. Per ADR §2.7.20 the same-thread sync path is the
    /// supported surface; blocking `recv()` (await-style) requires
    /// the §2.7.4 task-scheduler boundary and is SURFACE'd at the
    /// method body.
    pub fn try_recv(&self) -> Option<crate::kinded_slot::KindedSlot> {
        self.inner
            .lock()
            .expect("channel mutex poisoned")
            .queue
            .pop_front()
    }

    /// Mark the channel closed. Idempotent — calling close on an
    /// already-closed channel is a no-op.
    pub fn close(&self) {
        self.inner.lock().expect("channel mutex poisoned").closed = true;
    }
}

impl Default for ChannelData {
    fn default() -> Self {
        Self::new()
    }
}

// ── PriorityQueue storage (Wave 15 W15-priority-queue, 2026-05-10) ──────────

/// PriorityQueue storage — i64-priority min-heap.
///
/// ADR-006 §2.7.18 / Q19 amendment (mirror of §2.7.15 HashSet precedent
/// for the cardinality-amendment shape). Storage is a binary min-heap
/// laid out in a single `Vec<i64>` over an `Arc<TypedBuffer<i64>>`
/// so the buffer-Arc pattern matches the rest of the typed-Arc heap
/// family (clone-on-write via `Arc::make_mut`, single atomic refcount
/// at slot drop).
///
/// **i64-priority-only at landing** (per the Wave 15 audit and the
/// §2.7.18 Q19 ruling). Heterogeneous-payload priority queues
/// (TypedObject-payload, payload-with-comparator-closure) are
/// explicitly out-of-scope; the playbook called out the i64-priority-
/// only design as the simpler valid path, and the smoke target
/// (`pq.push(3); pq.push(1); pq.push(2); pq.pop() == 1`) is exercised
/// end-to-end on this shape. A typed-payload rebuild (`PriorityQueue
/// <T, K>` with key-extractor and arbitrary `T` payloads) is a future
/// Phase-2c amendment with measurement.
///
/// The heap invariant is "min-heap": the minimum priority sits at
/// index 0 (`peek()` / `pop()` return it). Standard
/// sift-up-on-push / sift-down-on-pop maintenance, `O(log n)` per
/// push/pop.
#[derive(Debug)]
pub struct PriorityQueueData {
    /// Heap-ordered i64 priorities. Index 0 is the current min.
    /// Backed by an `Arc<TypedBuffer<i64>>` so a HeapValue clone is a
    /// single atomic refcount bump and `Arc::make_mut` is the
    /// canonical clone-on-write entry per the W13-hashmap-mutation
    /// precedent.
    pub heap: Arc<crate::typed_buffer::TypedBuffer<i64>>,
}

impl PriorityQueueData {
    /// Build an empty PriorityQueueData with no entries.
    pub fn new() -> Self {
        Self {
            heap: Arc::new(crate::typed_buffer::TypedBuffer::from_vec(Vec::new())),
        }
    }

    /// Number of entries.
    #[inline]
    pub fn len(&self) -> usize {
        self.heap.data.len()
    }

    /// Whether the queue is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.heap.data.is_empty()
    }

    /// Peek at the minimum (root) without removing it. Returns `None`
    /// for an empty queue.
    pub fn peek(&self) -> Option<i64> {
        self.heap.data.first().copied()
    }

    /// Push a value, restoring the min-heap invariant via sift-up.
    /// Mirror of W13-hashmap-mutation `insert`: `Arc::make_mut`
    /// clone-on-write over the inner `Arc<TypedBuffer<i64>>`.
    pub fn push(&mut self, value: i64) {
        let buf = Arc::make_mut(&mut self.heap);
        buf.data.push(value);
        let last = buf.data.len() - 1;
        sift_up(&mut buf.data, last);
    }

    /// Pop the minimum value, restoring the min-heap invariant via
    /// sift-down. Returns `None` for an empty queue. Mirror of
    /// W13-hashmap-mutation `remove`: `Arc::make_mut` clone-on-write.
    pub fn pop(&mut self) -> Option<i64> {
        let buf = Arc::make_mut(&mut self.heap);
        if buf.data.is_empty() {
            return None;
        }
        let last = buf.data.len() - 1;
        buf.data.swap(0, last);
        let min = buf.data.pop();
        if !buf.data.is_empty() {
            sift_down(&mut buf.data, 0);
        }
        min
    }

    /// Return the heap contents as a flat `Vec<i64>` in heap-array
    /// order (NOT sorted). Used for the `toArray` method's `Vec<int>`
    /// projection; for the sorted form see `to_sorted_vec`.
    pub fn to_vec(&self) -> Vec<i64> {
        self.heap.data.clone()
    }

    /// Return the heap contents as a sorted `Vec<i64>` (ascending —
    /// pop-order). Used for the `toSortedArray` method.
    pub fn to_sorted_vec(&self) -> Vec<i64> {
        let mut v = self.heap.data.clone();
        v.sort_unstable();
        v
    }
}

impl Default for PriorityQueueData {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for DequeData {
    fn clone(&self) -> Self {
        // Per-element `Arc<HeapValue>` clone bumps each element's strong-
        // count share; the resulting `VecDeque` is structurally
        // independent of the source. Mirror of `HashSetData::clone`'s
        // `Arc::clone(&keys)` shape but without the buffer-Arc indirection
        // (the per-element `Arc<HeapValue>` already provides the share).
        Self {
            items: self.items.iter().map(Arc::clone).collect(),
        }
    }
}

impl Clone for PriorityQueueData {
    fn clone(&self) -> Self {
        Self {
            heap: Arc::clone(&self.heap),
        }
    }
}

/// Sift up: restore the min-heap invariant after a push at index `i`
/// by walking parent links upward, swapping while the child is less
/// than its parent.
#[inline]
fn sift_up(data: &mut [i64], mut i: usize) {
    while i > 0 {
        let parent = (i - 1) / 2;
        if data[i] < data[parent] {
            data.swap(i, parent);
            i = parent;
        } else {
            break;
        }
    }
}

/// Sift down: restore the min-heap invariant after a pop-replacement
/// at index `i` by walking down the smaller child link, swapping
/// while a child is less than the current node.
#[inline]
fn sift_down(data: &mut [i64], mut i: usize) {
    let n = data.len();
    loop {
        let left = 2 * i + 1;
        let right = 2 * i + 2;
        let mut smallest = i;
        if left < n && data[left] < data[smallest] {
            smallest = left;
        }
        if right < n && data[right] < data[smallest] {
            smallest = right;
        }
        if smallest == i {
            break;
        }
        data.swap(i, smallest);
        i = smallest;
    }
}

// ── TaskGroup storage (ADR-006 §2.3) ────────────────────────────────────────

/// Task-group payload. Extracted from the inline
/// `HeapValue::TaskGroup { kind, task_ids }` struct variant per ADR-006 §2.3
/// so `HeapValue::TaskGroup` becomes a single-tuple `Arc<T>` payload like
/// every other ADR-006 §2.3 heap arm.
///
/// The struct preserves the `kind` discriminant and `task_ids` list verbatim
/// — clone semantics live on the enclosing `Arc<TaskGroupData>` (one atomic
/// refcount bump). Phase 1.B migrates the cascade pattern-match sites
/// (`shape-vm::executor::async_ops`, `shape-jit::ffi::async_ops`,
/// `shape-runtime::wire_conversion`, ...) from struct-variant destructuring
/// to `task_group.kind` / `task_group.task_ids` field reads.
#[derive(Debug, Clone)]
pub struct TaskGroupData {
    pub kind: u8,
    pub task_ids: Vec<u64>,
}

// ── TypedObject storage (ADR-006 §2.3 / §2.5) ───────────────────────────────

/// Schema-keyed object storage. Extracted from the inline
/// `HeapValue::TypedObject { schema_id, slots, heap_mask }` struct
/// variant per ADR-006 §2.3, so that:
///
/// 1. `HeapValue::TypedObject` becomes `HeapValue::TypedObject(Arc<TypedObjectStorage>)`
///    — a typed `Arc<T>` payload like every other ADR-006 §2.3 heap arm.
/// 2. The `Drop` impl (Step 5) lives on `TypedObjectStorage` and dispatches
///    per-field on `NativeKind` from the embedded `field_kinds: Arc<[NativeKind]>`
///    table — no schema-registry probe and no cross-crate function-pointer
///    hook at drop time.
///
/// Field invariants (ADR-006 §2.3):
///
/// - `schema_id` is the registry key for the TypeSchema. Kept for wire /
///   snapshot round-trip and downstream schema-aware code (printing,
///   marshal); not consulted at drop time.
/// - `slots` is a per-field 8-byte storage array. Field at index `i`
///   stores its bits per the schema's `FieldType` for that field.
/// - `heap_mask` has bit `i` set iff slot `i` holds a heap pointer
///   (`Arc<T>` raw pointer per ADR-006 §2.4). Bits beyond `slots.len()`
///   must be zero.
/// - `field_kinds` is an `Arc<[NativeKind]>` of length `slots.len()`,
///   one entry per field, carrying the proven `NativeKind` for that
///   field's slot bits. The Arc payload is **shared per-schema**: the
///   construction path (in `shape-runtime`) maps `schema_id ⇒ Arc<[NativeKind]>`
///   once per schema (one HashMap probe at the first construction; cached
///   for subsequent constructions) and clones the cached Arc into each
///   instance — so 1M Customer-objects of the same shape share one
///   `[NativeKind]` allocation. Drop is then constant-time per slot
///   without any cross-crate registry call.
///
/// Why the `Arc<[NativeKind]>` (Option B' per supervisor ruling, ADR-006 §17):
///
/// - **Option B** (per-instance `Box<[NativeKind]>`) was rejected: it
///   duplicates the same NativeKind sequence across every instance of a
///   schema (1M × 8 fields = 16MB cumulative duplication).
/// - **Option C** (function-pointer hook in shape-value installed by
///   shape-runtime) was rejected: it adds a cross-crate runtime hook for
///   metadata that's already known at construction time.
/// - **Option B'** (this one) does the lookup once at construction (where
///   the schema is in scope) and shares the result via Arc — 8-byte
///   pointer per instance, single payload allocation per schema, no
///   probe at drop. Per Q8 spirit: the schema lookup happens, but it's
///   profile-driven *preempted* to construction time and cached.
///
/// `TypedObjectStorage` is `pub` with `pub` fields so the existing
/// destructuring call sites can migrate by reading
/// `storage.schema_id` / `storage.slots` / `storage.heap_mask`. The
/// struct is intentionally not `Clone` — clone semantics belong to the
/// enclosing `Arc<TypedObjectStorage>` (one atomic refcount bump).
#[derive(Debug)]
pub struct TypedObjectStorage {
    /// Registry key for the TypeSchema describing each slot's `FieldType`.
    pub schema_id: u64,
    /// Per-field 8-byte storage. Length matches the schema's field count.
    pub slots: Box<[crate::slot::ValueSlot]>,
    /// Bit `i` set ⇔ slot `i` holds a heap pointer that participates in
    /// Arc refcount discipline. Bits beyond `slots.len()` must be zero.
    pub heap_mask: u64,
    /// Per-field `NativeKind` table — same length as `slots`. **Shared
    /// per-schema** via `Arc`: every instance of the same schema clones
    /// the same payload (one atomic refcount bump per construction).
    /// Consulted by `Drop` to dispatch per-slot `Arc::decrement_strong_count`
    /// without any schema-registry probe.
    pub field_kinds: std::sync::Arc<[crate::native_kind::NativeKind]>,
}

impl TypedObjectStorage {
    /// Construct a new `TypedObjectStorage`.
    ///
    /// Construction-side contract (callers in `shape-runtime`):
    ///
    /// 1. `slots.len() == field_kinds.len()` — one kind per slot.
    /// 2. For each bit `i` set in `heap_mask`, `field_kinds[i]` must be
    ///    a heap-pointer kind (`NativeKind::String` or
    ///    `NativeKind::Ptr(_)`) and the slot's `u64` must be the raw
    ///    pointer of an `Arc::into_raw::<T>` for the matching `T`. Drop
    ///    relies on this for soundness.
    /// 3. `field_kinds` should be the per-schema cached `Arc<[NativeKind]>`
    ///    (callers maintain a `schema_id ⇒ Arc<[NativeKind]>` cache to
    ///    avoid per-instance allocation).
    ///
    /// Returns the storage by value; the canonical wrap is
    /// `Arc::new(TypedObjectStorage::new(...))` immediately followed by
    /// `HeapValue::TypedObject(arc)` or `ValueSlot::from_typed_object(arc)`.
    #[inline]
    pub fn new(
        schema_id: u64,
        slots: Box<[crate::slot::ValueSlot]>,
        heap_mask: u64,
        field_kinds: std::sync::Arc<[crate::native_kind::NativeKind]>,
    ) -> Self {
        debug_assert_eq!(
            slots.len(),
            field_kinds.len(),
            "TypedObjectStorage::new: slots/field_kinds length mismatch \
             (slots={}, field_kinds={}) — every slot must have a proven NativeKind",
            slots.len(),
            field_kinds.len(),
        );
        Self { schema_id, slots, heap_mask, field_kinds }
    }
}

impl Drop for TypedObjectStorage {
    /// ADR-006 §2.5: walk `heap_mask`, dispatch per-slot on
    /// `field_kinds[i]`, and call the matching
    /// `Arc::decrement_strong_count::<T>` for the slot's typed pointer.
    /// Non-heap slots (heap_mask bit clear) are no-ops.
    ///
    /// Soundness contract (must hold by construction; see
    /// `TypedObjectStorage::new`):
    ///
    /// - For every `i` where `heap_mask >> i & 1 == 1`, the slot's `u64`
    ///   bits are the result of `Arc::into_raw::<T>` where `T` matches
    ///   `field_kinds[i]`. The mapping is:
    ///     - `NativeKind::String`           → `Arc<String>`
    ///     - `NativeKind::Ptr(HeapKind::String)`        → `Arc<String>`
    ///     - `NativeKind::Ptr(HeapKind::TypedArray)`    → `Arc<TypedArrayData>`
    ///     - `NativeKind::Ptr(HeapKind::TypedObject)`   → `Arc<TypedObjectStorage>`
    ///     - `NativeKind::Ptr(HeapKind::HashMap)`       → `Arc<HashMapData>`
    ///     - `NativeKind::Ptr(HeapKind::Decimal)`       → `Arc<rust_decimal::Decimal>`
    ///     - `NativeKind::Ptr(HeapKind::BigInt)`        → `Arc<i64>`
    ///     - `NativeKind::Ptr(HeapKind::DataTable)`     → `Arc<DataTable>`
    ///     - `NativeKind::Ptr(HeapKind::IoHandle)`      → `Arc<IoHandleData>`
    ///     - `NativeKind::Ptr(HeapKind::NativeView)`    → `Arc<NativeViewData>`
    ///     - `NativeKind::Ptr(HeapKind::Content)`       → `Arc<ContentNode>`
    ///     - `NativeKind::Ptr(HeapKind::Instant)`       → `Arc<Instant>`
    ///     - `NativeKind::Ptr(HeapKind::Temporal)`      → `Arc<TemporalData>`
    ///     - `NativeKind::Ptr(HeapKind::TableView)`     → `Arc<TableViewData>`
    ///     - `NativeKind::Ptr(HeapKind::TaskGroup)`     → `Arc<TaskGroupData>`
    /// - `NativeKind::Ptr(HeapKind::{Closure, Future, Char, NativeScalar})`
    ///   correspond to `HeapValue` variants that do **not** carry an
    ///   `Arc<T>` slot payload (closure uses `OwnedClosureBlock` whose
    ///   refcount is managed by its own Drop; the others are inline
    ///   scalars). A heap_mask bit set with one of those kinds is a
    ///   soundness violation by construction; the Drop arms hit
    ///   `unreachable!` in debug and silently no-op in release rather
    ///   than guess at the slot bits.
    fn drop(&mut self) {
        use crate::heap_value::HeapKind;
        use crate::native_kind::NativeKind;

        // Defensive: if construction left a length mismatch (debug_assert
        // catches it earlier), drop only the prefix where both bookkeeping
        // structures agree. Better a leak than UB.
        let n = self.slots.len().min(self.field_kinds.len());
        for i in 0..n {
            // heap_mask is u64; bits beyond 63 cannot be addressed today.
            // Schemas with >64 fields are out of scope until the bitmap
            // widens (no caller produces that; documented invariant).
            if i >= 64 {
                break;
            }
            if (self.heap_mask >> i) & 1 == 0 {
                continue;
            }
            let bits = self.slots[i].raw();
            if bits == 0 {
                continue;
            }
            // SAFETY (each arm): the construction-side contract guarantees
            // that for every set heap_mask bit, the slot's bits are the
            // result of `Arc::into_raw::<T>` where `T` matches `field_kinds[i]`.
            // We reclaim exactly one strong-count share per slot via
            // `Arc::decrement_strong_count::<T>` and then never look at the
            // bits again.
            unsafe {
                match self.field_kinds[i] {
                    // Both NativeKind::String and Ptr(HeapKind::String)
                    // resolve to the same Arc<String> payload — the field
                    // type's String is the named exception (ADR-005 §2).
                    NativeKind::String => {
                        std::sync::Arc::decrement_strong_count(bits as *const String);
                    }
                    NativeKind::Ptr(hk) => match hk {
                        HeapKind::String => {
                            std::sync::Arc::decrement_strong_count(bits as *const String);
                        }
                        HeapKind::TypedArray => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const TypedArrayData,
                            );
                        }
                        HeapKind::TypedObject => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const TypedObjectStorage,
                            );
                        }
                        HeapKind::HashMap => {
                            std::sync::Arc::decrement_strong_count(bits as *const HashMapData);
                        }
                        // Wave 13 W13-hashset-rebuild (ADR-006 §2.7.15
                        // / Q16, 2026-05-10): a TypedObject field of
                        // kind `NativeKind::Ptr(HeapKind::HashSet)` holds
                        // slot bits = `Arc::into_raw(Arc<HashSetData>)`.
                        // Same dispatch shape as the HashMap arm above
                        // (HashSet is a HashMap sibling per §2.7.15) —
                        // retire one `Arc<HashSetData>` strong-count
                        // share at storage drop.
                        HeapKind::HashSet => {
                            std::sync::Arc::decrement_strong_count(bits as *const HashSetData);
                        }
                        // Wave 15 W15-deque (ADR-006 §2.7.19 / Q20,
                        // 2026-05-10): a TypedObject field of kind
                        // `NativeKind::Ptr(HeapKind::Deque)` holds slot
                        // bits = `Arc::into_raw(Arc<DequeData>)`.
                        // Same dispatch shape as the HashSet arm above
                        // (Deque is a HashSet sibling per §2.7.19) —
                        // retire one `Arc<DequeData>` strong-count
                        // share at storage drop.
                        HeapKind::Deque => {
                            std::sync::Arc::decrement_strong_count(bits as *const DequeData);
                        }
                        // Wave 15 W15-channel-rebuild (ADR-006 §2.7.20
                        // / Q21, 2026-05-10): a TypedObject field of
                        // kind `NativeKind::Ptr(HeapKind::Channel)`
                        // holds slot bits =
                        // `Arc::into_raw(Arc<ChannelData>)`. Same
                        // dispatch shape as the HashSet arm above —
                        // retire one `Arc<ChannelData>` strong-count
                        // share at storage drop. The inner
                        // `Mutex<ChannelInner>` Drop runs at
                        // refcount=0, retiring queued `KindedSlot`
                        // payloads.
                        HeapKind::Channel => {
                            std::sync::Arc::decrement_strong_count(bits as *const ChannelData);
                        }
                        HeapKind::Decimal => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const rust_decimal::Decimal,
                            );
                        }
                        HeapKind::BigInt => {
                            std::sync::Arc::decrement_strong_count(bits as *const i64);
                        }
                        HeapKind::DataTable => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const crate::datatable::DataTable,
                            );
                        }
                        HeapKind::IoHandle => {
                            std::sync::Arc::decrement_strong_count(bits as *const IoHandleData);
                        }
                        HeapKind::NativeView => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const NativeViewData,
                            );
                        }
                        HeapKind::Content => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const crate::content::ContentNode,
                            );
                        }
                        HeapKind::Instant => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const std::time::Instant,
                            );
                        }
                        HeapKind::Temporal => {
                            std::sync::Arc::decrement_strong_count(bits as *const TemporalData);
                        }
                        HeapKind::TableView => {
                            std::sync::Arc::decrement_strong_count(bits as *const TableViewData);
                        }
                        HeapKind::TaskGroup => {
                            std::sync::Arc::decrement_strong_count(bits as *const TaskGroupData);
                        }
                        // Wave-γ G-heap-filter-expr (ADR-006 §2.3 / §2.7.6
                        // / Q8 amendment): FilterExpr fields hold one
                        // `Arc::into_raw(Arc<FilterNode>)` strong-count
                        // share. Pre-amendment, FilterExpr-typed slot bits
                        // were mislabeled as `HeapKind::NativeView`; this
                        // arm dispatches them as the correct payload type.
                        HeapKind::FilterExpr => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const crate::value::FilterNode,
                            );
                        }
                        // Wave 8 W8-T26 (ADR-006 §2.7.13 / Q14, 2026-05-10):
                        // a TypedObject field of kind
                        // `NativeKind::Ptr(HeapKind::Reference)` holds slot
                        // bits = `Arc::into_raw(Arc<RefTarget>)` directly
                        // (mirror of FilterExpr's pure-discriminator-style
                        // dispatch — NOT a `Box<HeapValue>` wrap). At
                        // storage drop, retire one `Arc<RefTarget>`
                        // strong-count share. Same dispatch shape as the
                        // FilterExpr §2.7.9 amendment.
                        HeapKind::Reference => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const crate::reference::RefTarget,
                            );
                        }
                        // W13-iterator-state (ADR-006 §2.7.16 / Q17,
                        // 2026-05-10): a TypedObject field of kind
                        // `NativeKind::Ptr(HeapKind::Iterator)` holds
                        // slot bits = `Arc::into_raw(Arc<IteratorState>)`
                        // directly (mirror of FilterExpr / Reference
                        // typed-Arc dispatch — NOT a `Box<HeapValue>`
                        // wrap). At storage drop, retire one
                        // `Arc<IteratorState>` strong-count share. Same
                        // dispatch shape as the FilterExpr §2.7.9
                        // amendment.
                        HeapKind::Iterator => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const crate::iterator_state::IteratorState,
                            );
                        }
                        // Wave 15 W15-priority-queue (ADR-006 §2.7.18 /
                        // Q19, 2026-05-10): a TypedObject field of kind
                        // `NativeKind::Ptr(HeapKind::PriorityQueue)` holds
                        // slot bits = `Arc::into_raw(Arc<
                        // PriorityQueueData>)`. Same dispatch shape as
                        // the HashSet arm above (PriorityQueue is a
                        // HashSet sibling per §2.7.18) — retire one
                        // `Arc<PriorityQueueData>` strong-count share at
                        // storage drop.
                        HeapKind::PriorityQueue => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const PriorityQueueData,
                            );
                        }
                        // Round 2.5b W7-closure-retain-parallel (ADR-006
                        // §2.7.11 / Q12, 2026-05-09 — lockstep with vm-tier
                        // Round 2.5 close `5fa4b19`): when a TypedObject
                        // field of kind `NativeKind::Ptr(HeapKind::Closure)`
                        // is dropped along with the storage, slot bits are
                        // `Arc::into_raw(Arc<HeapValue>)` pointing to a
                        // `HeapValue::ClosureRaw(OwnedClosureBlock)` arm.
                        // Round 2 close (`06cdfce`) committed to this
                        // slot-bits shape via `slot.as_heap_value()`.
                        // Same dispatch shape as the FilterExpr §2.7.9
                        // amendment.
                        HeapKind::Closure => {
                            std::sync::Arc::decrement_strong_count(bits as *const HeapValue);
                        }
                        // `Ptr(HeapKind::Future)` carries the future-id u64
                        // directly in `bits` (inline scalar — no `Arc<T>`
                        // payload). See `async_ops/mod.rs` §"Wave 6.5 /
                        // E-async migration" docstring.
                        HeapKind::Future => {}
                        // Wave 8 W8-T25 (ADR-006 §2.7.12 / Q13 amendment,
                        // 2026-05-10): when a TypedObject field of kind
                        // `NativeKind::Ptr(HeapKind::SharedCell)` is dropped
                        // along with the storage, slot bits are
                        // `Arc::into_raw(Arc<SharedCell>)`. Retires one
                        // `Arc<SharedCell>` strong-count share — the inner
                        // `SharedCell::Drop` then runs, releasing its
                        // interior payload via its persistent `kind`
                        // companion (§2.7.8 / Q10 lockstep). Same dispatch
                        // shape as the FilterExpr §2.7.9 amendment.
                        HeapKind::SharedCell => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const crate::v2::closure_layout::SharedCell,
                            );
                        }
                        // `HeapKind::Char` carries codepoint bits inline.
                        // A heap_mask bit set on a Char field is a
                        // construction-side bug per the original soundness
                        // contract: Char is not an `Arc<T>`-payload kind,
                        // so the field should never have been classified
                        // as heap.
                        HeapKind::Char => {
                            debug_assert!(
                                false,
                                "TypedObjectStorage::drop: heap_mask bit {} set with \
                                 inline-scalar kind Char (schema_id={}); \
                                 construction-side soundness violation",
                                i, self.schema_id
                            );
                        }
                        // `HeapKind::NativeScalar` kinded carrier pending
                        // phase-2c kinded redesign (ADR-006 §2.7.4). When
                        // it lands, this arm wires its release per the
                        // chosen share carrier. Until then, a non-zero
                        // pointer with this kind is a construction-side
                        // bug — no Bool-default fallback (forbidden #9).
                        HeapKind::NativeScalar => {
                            debug_assert!(
                                false,
                                "TypedObjectStorage::drop: NativeScalar kinded carrier \
                                 pending phase-2c kinded redesign (ADR-006 §2.7.4); \
                                 schema_id={}, bit {}",
                                self.schema_id, i
                            );
                        }
                    },
                    // Non-heap NativeKinds (integers, floats, bool) should
                    // never have their heap_mask bit set. Same construction
                    // soundness contract as above.
                    other => {
                        debug_assert!(
                            false,
                            "TypedObjectStorage::drop: heap_mask bit {} set with \
                             non-heap NativeKind {:?} (schema_id={}); \
                             construction-side soundness violation",
                            i, other, self.schema_id
                        );
                    }
                }
            }
        }
    }
}

// ── TypedArray buckets ──────────────────────────────────────────────────────

/// Typed array data — consolidates IntArray, FloatArray, BoolArray, Matrix,
/// I8Array..F32Array, and FloatArraySlice into a single HeapValue variant.
///
/// Phase 2d Array cluster (2026-05-07) added the `String` and `HeapValue`
/// arms. `String` carries `Vec<Arc<String>>` for `Array<string>`. `HeapValue`
/// carries `Vec<Arc<HeapValue>>` for `Array<X>` where X is itself a heap-
/// allocated typed value (e.g. `Array<DataTable>`, `Array<Array<string>>`,
/// `Array<TypedObject>`). Element-kind discrimination at the `HeapValue`
/// arm is a body-side type contract (option β / option ε pattern from
/// cluster #3): each `FromSlot for Vec<Arc<X>>` impl pattern-matches the
/// expected inner `HeapValue::*` variant, panicking on mismatch as a
/// spec-permitted consistency check (`docs/runtime-v2-spec.md`).
#[derive(Debug, Clone)]
pub enum TypedArrayData {
    I64(Arc<crate::typed_buffer::TypedBuffer<i64>>),
    F64(Arc<crate::typed_buffer::AlignedTypedBuffer>),
    Bool(Arc<crate::typed_buffer::TypedBuffer<u8>>),
    Matrix(Arc<MatrixData>),
    I8(Arc<crate::typed_buffer::TypedBuffer<i8>>),
    I16(Arc<crate::typed_buffer::TypedBuffer<i16>>),
    I32(Arc<crate::typed_buffer::TypedBuffer<i32>>),
    U8(Arc<crate::typed_buffer::TypedBuffer<u8>>),
    U16(Arc<crate::typed_buffer::TypedBuffer<u16>>),
    U32(Arc<crate::typed_buffer::TypedBuffer<u32>>),
    U64(Arc<crate::typed_buffer::TypedBuffer<u64>>),
    F32(Arc<crate::typed_buffer::TypedBuffer<f32>>),
    String(Arc<crate::typed_buffer::TypedBuffer<Arc<String>>>),
    HeapValue(Arc<crate::typed_buffer::TypedBuffer<Arc<crate::heap_value::HeapValue>>>),
    FloatSlice {
        parent: Arc<MatrixData>,
        offset: u32,
        len: u32,
    },
}

impl TypedArrayData {
    #[inline]
    pub fn type_name(&self) -> &'static str {
        match self {
            TypedArrayData::I64(_) => "Vec<int>",
            TypedArrayData::F64(_) => "Vec<number>",
            TypedArrayData::Bool(_) => "Vec<bool>",
            TypedArrayData::Matrix(_) => "Mat<number>",
            TypedArrayData::I8(_) => "Vec<i8>",
            TypedArrayData::I16(_) => "Vec<i16>",
            TypedArrayData::I32(_) => "Vec<i32>",
            TypedArrayData::U8(_) => "Vec<u8>",
            TypedArrayData::U16(_) => "Vec<u16>",
            TypedArrayData::U32(_) => "Vec<u32>",
            TypedArrayData::U64(_) => "Vec<u64>",
            TypedArrayData::F32(_) => "Vec<f32>",
            TypedArrayData::String(_) => "Vec<string>",
            TypedArrayData::HeapValue(_) => "Vec<heap>",
            TypedArrayData::FloatSlice { .. } => "Vec<number>",
        }
    }

    #[inline]
    pub fn is_truthy(&self) -> bool {
        match self {
            TypedArrayData::I64(a) => !a.is_empty(),
            TypedArrayData::F64(a) => !a.is_empty(),
            TypedArrayData::Bool(a) => !a.is_empty(),
            TypedArrayData::Matrix(m) => m.data.len() > 0,
            TypedArrayData::I8(a) => !a.is_empty(),
            TypedArrayData::I16(a) => !a.is_empty(),
            TypedArrayData::I32(a) => !a.is_empty(),
            TypedArrayData::U8(a) => !a.is_empty(),
            TypedArrayData::U16(a) => !a.is_empty(),
            TypedArrayData::U32(a) => !a.is_empty(),
            TypedArrayData::U64(a) => !a.is_empty(),
            TypedArrayData::F32(a) => !a.is_empty(),
            TypedArrayData::String(a) => !a.is_empty(),
            TypedArrayData::HeapValue(a) => !a.is_empty(),
            TypedArrayData::FloatSlice { len, .. } => *len > 0,
        }
    }
}

impl fmt::Display for TypedArrayData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypedArrayData::I64(a) => {
                write!(f, "Vec<int>[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            TypedArrayData::F64(a) => {
                write!(f, "Vec<number>[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    if *v == v.trunc() && v.abs() < 1e15 {
                        write!(f, "{}", *v as i64)?;
                    } else {
                        write!(f, "{}", v)?;
                    }
                }
                write!(f, "]")
            }
            TypedArrayData::Bool(a) => {
                write!(f, "Vec<bool>[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", *v != 0)?;
                }
                write!(f, "]")
            }
            TypedArrayData::Matrix(m) => {
                write!(f, "<Mat<number>:{}x{}>", m.rows, m.cols)
            }
            TypedArrayData::I8(a) => {
                write!(f, "Vec<i8>[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            TypedArrayData::I16(a) => {
                write!(f, "Vec<i16>[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            TypedArrayData::I32(a) => {
                write!(f, "Vec<i32>[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            TypedArrayData::U8(a) => {
                write!(f, "Vec<u8>[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            TypedArrayData::U16(a) => {
                write!(f, "Vec<u16>[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            TypedArrayData::U32(a) => {
                write!(f, "Vec<u32>[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            TypedArrayData::U64(a) => {
                write!(f, "Vec<u64>[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            TypedArrayData::F32(a) => {
                write!(f, "Vec<f32>[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            TypedArrayData::String(a) => {
                write!(f, "Vec<string>[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{}\"", v)?;
                }
                write!(f, "]")
            }
            TypedArrayData::HeapValue(a) => {
                write!(f, "Vec<heap>[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            TypedArrayData::FloatSlice {
                parent,
                offset,
                len,
            } => {
                let slice = &parent.data[*offset as usize..(*offset + *len) as usize];
                write!(f, "Vec<number>[")?;
                for (i, v) in slice.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    if *v == v.trunc() && v.abs() < 1e15 {
                        write!(f, "{}", *v as i64)?;
                    } else {
                        write!(f, "{}", v)?;
                    }
                }
                write!(f, "]")
            }
        }
    }
}

// ── Temporal data ───────────────────────────────────────────────────────────

/// Temporal data — consolidates Time, Duration, TimeSpan, Timeframe,
/// TimeReference, DateTimeExpr, and DataDateTimeRef.
#[derive(Debug, Clone)]
pub enum TemporalData {
    DateTime(chrono::DateTime<chrono::FixedOffset>),
    Duration(shape_ast::ast::Duration),
    TimeSpan(chrono::Duration),
    Timeframe(shape_ast::data::Timeframe),
    TimeReference(Box<shape_ast::ast::TimeReference>),
    DateTimeExpr(Box<shape_ast::ast::DateTimeExpr>),
    DataDateTimeRef(Box<shape_ast::ast::DataDateTimeRef>),
}

impl TemporalData {
    #[inline]
    pub fn type_name(&self) -> &'static str {
        match self {
            TemporalData::DateTime(_) => "time",
            TemporalData::Duration(_) => "duration",
            TemporalData::TimeSpan(_) => "timespan",
            TemporalData::Timeframe(_) => "timeframe",
            TemporalData::TimeReference(_) => "time_reference",
            TemporalData::DateTimeExpr(_) => "datetime_expr",
            TemporalData::DataDateTimeRef(_) => "data_datetime_ref",
        }
    }

    #[inline]
    pub fn is_truthy(&self) -> bool {
        true
    }
}

impl fmt::Display for TemporalData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TemporalData::DateTime(t) => write!(f, "{}", t),
            TemporalData::Duration(d) => write!(f, "{:?}", d),
            TemporalData::TimeSpan(ts) => write!(f, "{}", ts),
            TemporalData::Timeframe(tf) => write!(f, "{:?}", tf),
            TemporalData::TimeReference(_) => write!(f, "<time_ref>"),
            TemporalData::DateTimeExpr(_) => write!(f, "<datetime_expr>"),
            TemporalData::DataDateTimeRef(_) => write!(f, "<data_datetime_ref>"),
        }
    }
}

// ── Table view data ─────────────────────────────────────────────────────────

/// Table view data — consolidates TypedTable, RowView, ColumnRef, and IndexedTable.
#[derive(Debug, Clone)]
pub enum TableViewData {
    TypedTable {
        schema_id: u64,
        table: Arc<crate::datatable::DataTable>,
    },
    RowView {
        schema_id: u64,
        table: Arc<crate::datatable::DataTable>,
        row_idx: usize,
    },
    ColumnRef {
        schema_id: u64,
        table: Arc<crate::datatable::DataTable>,
        col_id: u32,
    },
    IndexedTable {
        schema_id: u64,
        table: Arc<crate::datatable::DataTable>,
        index_col: u32,
    },
}

impl TableViewData {
    #[inline]
    pub fn type_name(&self) -> &'static str {
        match self {
            TableViewData::TypedTable { .. } => "typed_table",
            TableViewData::RowView { .. } => "row",
            TableViewData::ColumnRef { .. } => "column",
            TableViewData::IndexedTable { .. } => "indexed_table",
        }
    }

    #[inline]
    pub fn is_truthy(&self) -> bool {
        match self {
            TableViewData::TypedTable { table, .. } => table.row_count() > 0,
            TableViewData::RowView { .. } => true,
            TableViewData::ColumnRef { .. } => true,
            TableViewData::IndexedTable { table, .. } => table.row_count() > 0,
        }
    }
}

impl fmt::Display for TableViewData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TableViewData::TypedTable { table, .. } => write!(
                f,
                "<typed_table:{}x{}>",
                table.row_count(),
                table.column_count()
            ),
            TableViewData::RowView { row_idx, .. } => write!(f, "<row:{}>", row_idx),
            TableViewData::ColumnRef { col_id, .. } => write!(f, "<column:{}>", col_id),
            TableViewData::IndexedTable { table, .. } => write!(
                f,
                "<indexed_table:{}x{}>",
                table.row_count(),
                table.column_count()
            ),
        }
    }
}

// ── Generate HeapValue, HeapKind, kind(), is_truthy(), type_name() ──────────
//
// All generated from the single source of truth in define_heap_types!().
crate::define_heap_types!();

// ── Manual Clone for HeapValue ──────────────────────────────────────────────
//
// ADR-006 §2.3 + Step 6: every heap-resident variant carries `Arc<T>` so its
// clone is one atomic refcount bump — no allocation, no payload copy. Inline
// scalars (`Future`, `Char`, `NativeScalar`) clone by `Copy`. `ClosureRaw`
// delegates to `OwnedClosureBlock::clone`, which already does a single
// `retain_typed_closure` refcount bump on the v2 closure block plus an Arc
// bump on the layout pointer.
//
// This impl is purely mechanical Arc::clone delegation — there is no
// `vw_clone` / `vw_drop` bookkeeping (the strict-typed bulldozer deleted
// every `ValueWord`-bearing variant).
impl Clone for HeapValue {
    fn clone(&self) -> Self {
        match self {
            // ADR-006 §2.3: Arc bump only — no allocation, no payload copy.
            HeapValue::String(v) => HeapValue::String(Arc::clone(v)),
            HeapValue::Decimal(v) => HeapValue::Decimal(Arc::clone(v)),
            HeapValue::BigInt(v) => HeapValue::BigInt(Arc::clone(v)),
            HeapValue::Future(v) => HeapValue::Future(*v),
            HeapValue::Char(v) => HeapValue::Char(*v),
            HeapValue::DataTable(v) => HeapValue::DataTable(Arc::clone(v)),
            HeapValue::Content(v) => HeapValue::Content(Arc::clone(v)),
            HeapValue::Instant(v) => HeapValue::Instant(Arc::clone(v)),
            HeapValue::IoHandle(v) => HeapValue::IoHandle(Arc::clone(v)),
            HeapValue::NativeScalar(v) => HeapValue::NativeScalar(*v),
            HeapValue::NativeView(v) => HeapValue::NativeView(Arc::clone(v)),
            HeapValue::TypedObject(s) => HeapValue::TypedObject(Arc::clone(s)),
            // OwnedClosureBlock::clone is one refcount bump on the typed
            // closure block + one Arc bump on the shared layout.
            HeapValue::ClosureRaw(v) => HeapValue::ClosureRaw(v.clone()),
            HeapValue::TaskGroup(v) => HeapValue::TaskGroup(Arc::clone(v)),
            HeapValue::TypedArray(v) => HeapValue::TypedArray(Arc::clone(v)),
            HeapValue::Temporal(v) => HeapValue::Temporal(Arc::clone(v)),
            HeapValue::TableView(v) => HeapValue::TableView(Arc::clone(v)),
            HeapValue::HashMap(v) => HeapValue::HashMap(Arc::clone(v)),
            // Wave 13 W13-hashset-rebuild (ADR-006 §2.7.15 / Q16,
            // 2026-05-10): mirror of HashMap — single strong-count bump
            // on the shared `Arc<HashSetData>`, no payload copy.
            HeapValue::HashSet(v) => HeapValue::HashSet(Arc::clone(v)),
            // Wave 15 W15-deque (ADR-006 §2.7.19 / Q20, 2026-05-10):
            // mirror of HashSet — single strong-count bump on the
            // shared `Arc<DequeData>`, no payload copy. Per-element
            // `Arc<HeapValue>` shares stay shared with the source.
            HeapValue::Deque(v) => HeapValue::Deque(Arc::clone(v)),
            // Wave-γ G-heap-filter-expr (ADR-006 §2.3 / Q8 amendment):
            // FilterExpr Arcs share the typed-Arc clone shape — single
            // strong-count bump, no payload copy.
            HeapValue::FilterExpr(v) => HeapValue::FilterExpr(Arc::clone(v)),
            // Wave 8 W8-T26 (ADR-006 §2.7.13 / Q14, 2026-05-10):
            // Reference Arcs share the typed-Arc clone shape — single
            // strong-count bump on the shared `Arc<RefTarget>`, no
            // payload copy.
            HeapValue::Reference(v) => HeapValue::Reference(Arc::clone(v)),
            // W13-iterator-state (ADR-006 §2.7.16 / Q17, 2026-05-10):
            // Iterator Arcs share the typed-Arc clone shape — single
            // strong-count bump on the shared `Arc<IteratorState>`. The
            // inner state is `Clone`-by-derive (typed-Arc payloads in
            // every field), but the outer `Arc` bump is the canonical
            // shared-receiver path.
            HeapValue::Iterator(v) => HeapValue::Iterator(Arc::clone(v)),
            // Wave 15 W15-channel-rebuild (ADR-006 §2.7.20 / Q21,
            // 2026-05-10): Channel Arcs share the typed-Arc clone shape
            // — single strong-count bump on the shared
            // `Arc<ChannelData>`. The inner `ChannelData` carries a
            // `Mutex<ChannelInner>` so two `Arc<ChannelData>` shares
            // observe each other's mutations; cloning the outer Arc
            // hands out a fresh endpoint of the same channel.
            HeapValue::Channel(v) => HeapValue::Channel(Arc::clone(v)),
            // Wave 15 W15-priority-queue (ADR-006 §2.7.18 / Q19,
            // 2026-05-10): mirror of HashSet — single strong-count bump
            // on the shared `Arc<PriorityQueueData>`, no payload copy.
            HeapValue::PriorityQueue(v) => HeapValue::PriorityQueue(Arc::clone(v)),
        }
    }
}

// ── Shared comparison helpers ────────────────────────────────────────────────

/// Cross-type numeric equality: BigInt vs Decimal.
#[inline]
fn bigint_decimal_eq(a: &i64, b: &rust_decimal::Decimal) -> bool {
    rust_decimal::Decimal::from(*a) == *b
}

/// Cross-type numeric equality: NativeScalar vs BigInt.
#[inline]
fn native_scalar_bigint_eq(a: &NativeScalar, b: &i64) -> bool {
    a.as_i64().is_some_and(|v| v == *b)
}

/// Cross-type numeric equality: NativeScalar vs Decimal.
#[inline]
fn native_scalar_decimal_eq(a: &NativeScalar, b: &rust_decimal::Decimal) -> bool {
    match a {
        NativeScalar::F32(v) => {
            rust_decimal::Decimal::from_f64_retain(*v as f64).is_some_and(|v| v == *b)
        }
        _ => a
            .as_i128()
            .map(|n| rust_decimal::Decimal::from_i128_with_scale(n, 0))
            .is_some_and(|to_dec| to_dec == *b),
    }
}

/// Cross-type typed array equality: IntArray vs FloatArray (element-wise i64-as-f64).
#[inline]
fn int_float_array_eq(
    ints: &crate::typed_buffer::TypedBuffer<i64>,
    floats: &crate::typed_buffer::AlignedTypedBuffer,
) -> bool {
    ints.len() == floats.len()
        && ints
            .iter()
            .zip(floats.iter())
            .all(|(x, y)| (*x as f64) == *y)
}

/// Matrix structural equality (row/col dimensions + element-wise).
#[inline]
fn matrix_eq(a: &MatrixData, b: &MatrixData) -> bool {
    a.rows == b.rows
        && a.cols == b.cols
        && a.data.len() == b.data.len()
        && a.data.iter().zip(b.data.iter()).all(|(x, y)| x == y)
}

/// NativeView identity comparison.
#[inline]
fn native_view_eq(a: &NativeViewData, b: &NativeViewData) -> bool {
    a.ptr == b.ptr && a.mutable == b.mutable && a.layout.name == b.layout.name
}

/// Structural equality for two `TypedArrayData` payloads.
///
/// ADR-006 §2.3: `HeapValue::TypedArray` carries `Arc<TypedArrayData>`,
/// so the outer pattern binds the Arc and forwards the inner-enum
/// dispatch here. Centralising the per-arm dispatch keeps both
/// `structural_eq` and `equals` honest about which arms genuinely
/// compare structurally vs which fall through to `false`.
#[inline]
fn typed_array_structural_eq(a: &TypedArrayData, b: &TypedArrayData) -> bool {
    match (a, b) {
        (TypedArrayData::I64(x), TypedArrayData::I64(y)) => x == y,
        (TypedArrayData::F64(x), TypedArrayData::F64(y)) => x == y,
        (TypedArrayData::I64(x), TypedArrayData::F64(y)) => int_float_array_eq(x, y),
        (TypedArrayData::F64(x), TypedArrayData::I64(y)) => int_float_array_eq(y, x),
        (TypedArrayData::Bool(x), TypedArrayData::Bool(y)) => x == y,
        (TypedArrayData::I8(x), TypedArrayData::I8(y)) => x == y,
        (TypedArrayData::I16(x), TypedArrayData::I16(y)) => x == y,
        (TypedArrayData::I32(x), TypedArrayData::I32(y)) => x == y,
        (TypedArrayData::U8(x), TypedArrayData::U8(y)) => x == y,
        (TypedArrayData::U16(x), TypedArrayData::U16(y)) => x == y,
        (TypedArrayData::U32(x), TypedArrayData::U32(y)) => x == y,
        (TypedArrayData::U64(x), TypedArrayData::U64(y)) => x == y,
        (TypedArrayData::F32(x), TypedArrayData::F32(y)) => x == y,
        (TypedArrayData::Matrix(x), TypedArrayData::Matrix(y)) => matrix_eq(x, y),
        (
            TypedArrayData::FloatSlice { parent: p1, offset: o1, len: l1 },
            TypedArrayData::FloatSlice { parent: p2, offset: o2, len: l2 },
        ) => {
            let s1 = &p1.data[*o1 as usize..(*o1 + *l1) as usize];
            let s2 = &p2.data[*o2 as usize..(*o2 + *l2) as usize];
            s1 == s2
        }
        _ => false,
    }
}

// ── Display ─────────────────────────────────────────────────────────────────

impl fmt::Display for HeapValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeapValue::Char(c) => write!(f, "{}", c),
            HeapValue::String(s) => write!(f, "{}", s),
            HeapValue::TypedObject(_) => write!(f, "{{...}}"),
            HeapValue::ClosureRaw(owned) => {
                // SAFETY: OwnedClosureBlock's invariant guarantees the
                // pointer is live for the duration of `&self`.
                let fid = unsafe {
                    crate::v2::closure_raw::typed_closure_function_id(owned.as_ptr())
                };
                write!(f, "<closure:{}>", fid)
            }
            HeapValue::Decimal(d) => write!(f, "{}", d),
            HeapValue::BigInt(i) => write!(f, "{}", i),
            HeapValue::DataTable(dt) => {
                write!(f, "<datatable:{}x{}>", dt.row_count(), dt.column_count())
            }
            HeapValue::TableView(tv) => write!(f, "{}", tv),
            HeapValue::Content(node) => write!(f, "{}", node),
            HeapValue::Instant(t) => write!(f, "<instant:{:?}>", t.elapsed()),
            HeapValue::IoHandle(data) => {
                let status = if data.is_open() { "open" } else { "closed" };
                write!(f, "<io_handle:{}:{}>", data.path, status)
            }
            HeapValue::Future(id) => write!(f, "<future:{}>", id),
            HeapValue::TaskGroup(tg) => {
                write!(f, "<task_group:{}>", tg.task_ids.len())
            }
            HeapValue::Temporal(td) => write!(f, "{}", td),
            HeapValue::NativeScalar(v) => write!(f, "{v}"),
            HeapValue::NativeView(v) => write!(
                f,
                "<{}:{}@0x{:x}>",
                if v.mutable { "cmut" } else { "cview" },
                v.layout.name,
                v.ptr
            ),
            HeapValue::TypedArray(ta) => write!(f, "{}", ta),
            HeapValue::HashMap(d) => {
                write!(f, "{{")?;
                for (i, (k, v)) in d.keys.data.iter().zip(d.values.data.iter()).enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{}\": {}", k, v)?;
                }
                write!(f, "}}")
            }
            // Wave 13 W13-hashset-rebuild (ADR-006 §2.7.15 / Q16,
            // 2026-05-10): one-keyspace mirror of HashMap's Display
            // shape — `{"a", "b", ...}` braces with comma-separated
            // quoted strings, no values.
            HeapValue::HashSet(d) => {
                write!(f, "{{")?;
                for (i, k) in d.keys.data.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{}\"", k)?;
                }
                write!(f, "}}")
            }
            // Wave 15 W15-deque (ADR-006 §2.7.19 / Q20, 2026-05-10):
            // render front-to-back as `Deque[elem1, elem2, ...]` —
            // dispatch each element through the canonical ADR-005 §1
            // single-discriminator `HeapValue` Display.
            HeapValue::Deque(d) => {
                write!(f, "Deque[")?;
                for (i, v) in d.items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            // Wave-γ G-heap-filter-expr (ADR-006 §2.3 amendment): no
            // user-facing FilterExpr literal exists; render as an opaque
            // tag for diagnostics. Construction-side bug if a FilterExpr
            // ever escapes into a user-visible Display path.
            HeapValue::FilterExpr(_) => write!(f, "<filter_expr>"),
            // Wave 8 W8-T26 (ADR-006 §2.7.13 / Q14, 2026-05-10): no
            // user-facing reference literal exists; render as an opaque
            // tag for diagnostics. References are within-program data
            // and don't cross any user-visible Display surface.
            HeapValue::Reference(_) => write!(f, "<ref>"),
            // W13-iterator-state (ADR-006 §2.7.16 / Q17, 2026-05-10):
            // iterator pipelines have no user-facing literal — render
            // as an opaque tag. Terminal operations (collect / forEach
            // / reduce / etc.) materialise the elements; an iterator
            // reaching the Display surface is "still lazy" by
            // construction.
            HeapValue::Iterator(_) => write!(f, "<iterator>"),
            // Wave 15 W15-channel-rebuild (ADR-006 §2.7.20 / Q21,
            // 2026-05-10): channels are concurrency primitives with no
            // user-facing literal; render as an opaque tag annotated
            // with current queue length and closed flag for
            // diagnostics.
            HeapValue::Channel(c) => {
                let len = c.len();
                let state = if c.is_closed() { "closed" } else { "open" };
                write!(f, "<channel:{}:{}>", state, len)
            }
            // Wave 15 W15-priority-queue (ADR-006 §2.7.18 / Q19,
            // 2026-05-10): one-keyspace mirror of HashSet's Display
            // shape — bracketed comma-separated values in heap-array
            // order. NOTE: heap-array order is not sort-order; for
            // sorted output the user must call `pq.toSortedArray()`.
            HeapValue::PriorityQueue(d) => {
                write!(f, "PriorityQueue[")?;
                for (i, v) in d.heap.data.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
        }
    }
}

// ── Hand-written methods (complex per-variant logic) ────────────────────────

impl HeapValue {
    /// Obtain a [`crate::vm_closure_handle::VmClosureHandle`] over this
    /// heap value, if it is a `HeapValue::ClosureRaw`.
    ///
    /// Closure spec §14.2: the handle is the stable read API for
    /// closure state. Returns `None` for non-closure heap values.
    #[inline]
    pub fn as_closure_handle(&self) -> Option<crate::vm_closure_handle::VmClosureHandle<'_>> {
        match self {
            HeapValue::ClosureRaw(owned) => {
                // SAFETY: `OwnedClosureBlock::from_raw` upholds that
                // `as_header_ptr()` points to a live `TypedClosureHeader`
                // whose layout matches `owned.layout()`; both remain valid
                // for the duration of the `&self` borrow.
                let handle = unsafe {
                    crate::vm_closure_handle::VmClosureHandle::raw(
                        owned.as_header_ptr(),
                        owned.layout().as_ref(),
                    )
                };
                Some(handle)
            }
            _ => None,
        }
    }

    /// Structural equality comparison for HeapValue.
    ///
    /// ADR-006 §2.3: `TypedArray` and `Temporal` payloads are now
    /// `Arc<TypedArrayData>` / `Arc<TemporalData>`; the per-arm dispatch
    /// dereferences the Arc once at the outer match and forwards into the
    /// inner enum via `typed_array_structural_eq` / direct `match`.
    pub fn structural_eq(&self, other: &HeapValue) -> bool {
        match (self, other) {
            (HeapValue::Char(a), HeapValue::Char(b)) => a == b,
            (HeapValue::String(a), HeapValue::String(b)) => a == b,
            // Cross-type: Char from string indexing vs String literal
            (HeapValue::Char(c), HeapValue::String(s))
            | (HeapValue::String(s), HeapValue::Char(c)) => {
                let mut buf = [0u8; 4];
                let cs = c.encode_utf8(&mut buf);
                cs == s.as_str()
            }
            (HeapValue::Decimal(a), HeapValue::Decimal(b)) => a == b,
            (HeapValue::BigInt(a), HeapValue::BigInt(b)) => a == b,
            (HeapValue::NativeScalar(a), HeapValue::NativeScalar(b)) => a == b,
            (HeapValue::NativeView(a), HeapValue::NativeView(b)) => native_view_eq(a, b),
            (HeapValue::Future(a), HeapValue::Future(b)) => a == b,
            (HeapValue::Temporal(a), HeapValue::Temporal(b)) => match (a.as_ref(), b.as_ref()) {
                (TemporalData::DateTime(x), TemporalData::DateTime(y)) => x == y,
                _ => false,
            },
            (HeapValue::Content(a), HeapValue::Content(b)) => a == b,
            (HeapValue::Instant(a), HeapValue::Instant(b)) => a == b,
            (HeapValue::IoHandle(a), HeapValue::IoHandle(b)) => {
                std::sync::Arc::ptr_eq(&a.resource, &b.resource)
            }
            (HeapValue::TypedArray(a), HeapValue::TypedArray(b)) => {
                typed_array_structural_eq(a.as_ref(), b.as_ref())
            }
            _ => false,
        }
    }

    /// Check equality between two heap values.
    #[inline]
    pub fn equals(&self, other: &HeapValue) -> bool {
        match (self, other) {
            (HeapValue::Char(a), HeapValue::Char(b)) => a == b,
            (HeapValue::String(a), HeapValue::String(b)) => a == b,
            // Cross-type: Char from string indexing vs String literal
            (HeapValue::Char(c), HeapValue::String(s))
            | (HeapValue::String(s), HeapValue::Char(c)) => {
                let mut buf = [0u8; 4];
                let cs = c.encode_utf8(&mut buf);
                cs == s.as_str()
            }
            (HeapValue::TypedObject(a), HeapValue::TypedObject(b)) => {
                // ADR-006 §2.3: payloads are `Arc<TypedObjectStorage>`;
                // pointer-equality is the fast path for shared storage.
                if Arc::ptr_eq(a, b) {
                    return true;
                }
                if a.schema_id != b.schema_id
                    || a.slots.len() != b.slots.len()
                    || a.heap_mask != b.heap_mask
                {
                    return false;
                }
                for i in 0..a.slots.len() {
                    // Both heap-mask and primitive-mask: compare raw bits
                    // for primitives. For heap slots, raw-bit equality is
                    // also conservatively correct since `ValueSlot` heap
                    // payloads are typed pointers — pointer-equality
                    // implies value-equality for shared Arc'd payloads.
                    if a.slots[i].raw() != b.slots[i].raw() {
                        return false;
                    }
                }
                true
            }
            // Track A.5: the canonical closure variant compares by function id.
            (HeapValue::ClosureRaw(a), HeapValue::ClosureRaw(b)) => {
                // SAFETY: both blocks are live per OwnedClosureBlock invariant.
                let fa = unsafe { crate::v2::closure_raw::typed_closure_function_id(a.as_ptr()) };
                let fb = unsafe { crate::v2::closure_raw::typed_closure_function_id(b.as_ptr()) };
                fa == fb
            }
            (HeapValue::Decimal(a), HeapValue::Decimal(b)) => a == b,
            (HeapValue::BigInt(a), HeapValue::BigInt(b)) => a == b,
            (HeapValue::BigInt(a), HeapValue::Decimal(b)) => bigint_decimal_eq(a.as_ref(), b.as_ref()),
            (HeapValue::Decimal(a), HeapValue::BigInt(b)) => bigint_decimal_eq(b.as_ref(), a.as_ref()),
            (HeapValue::DataTable(a), HeapValue::DataTable(b)) => Arc::ptr_eq(a, b),
            (HeapValue::TableView(a), HeapValue::TableView(b)) => match (a.as_ref(), b.as_ref()) {
                (
                    TableViewData::TypedTable { schema_id: s1, table: t1 },
                    TableViewData::TypedTable { schema_id: s2, table: t2 },
                ) => s1 == s2 && Arc::ptr_eq(t1, t2),
                (
                    TableViewData::RowView { schema_id: s1, row_idx: r1, table: t1 },
                    TableViewData::RowView { schema_id: s2, row_idx: r2, table: t2 },
                ) => s1 == s2 && r1 == r2 && Arc::ptr_eq(t1, t2),
                (
                    TableViewData::ColumnRef { schema_id: s1, col_id: c1, table: t1 },
                    TableViewData::ColumnRef { schema_id: s2, col_id: c2, table: t2 },
                ) => s1 == s2 && c1 == c2 && Arc::ptr_eq(t1, t2),
                (
                    TableViewData::IndexedTable { schema_id: s1, index_col: c1, table: t1 },
                    TableViewData::IndexedTable { schema_id: s2, index_col: c2, table: t2 },
                ) => s1 == s2 && c1 == c2 && Arc::ptr_eq(t1, t2),
                _ => false,
            },
            (HeapValue::Content(a), HeapValue::Content(b)) => a == b,
            (HeapValue::Instant(a), HeapValue::Instant(b)) => a == b,
            (HeapValue::IoHandle(a), HeapValue::IoHandle(b)) => {
                Arc::ptr_eq(&a.resource, &b.resource)
            }
            (HeapValue::Future(a), HeapValue::Future(b)) => a == b,
            (HeapValue::Temporal(a), HeapValue::Temporal(b)) => match (a.as_ref(), b.as_ref()) {
                (TemporalData::DateTime(x), TemporalData::DateTime(y)) => x == y,
                (TemporalData::Duration(x), TemporalData::Duration(y)) => x == y,
                (TemporalData::TimeSpan(x), TemporalData::TimeSpan(y)) => x == y,
                (TemporalData::Timeframe(x), TemporalData::Timeframe(y)) => x == y,
                _ => false,
            },
            (HeapValue::NativeScalar(a), HeapValue::NativeScalar(b)) => a == b,
            (HeapValue::NativeView(a), HeapValue::NativeView(b)) => native_view_eq(a, b),
            (HeapValue::TypedArray(a), HeapValue::TypedArray(b)) => {
                typed_array_structural_eq(a.as_ref(), b.as_ref())
            }
            // Cross-type numeric
            (HeapValue::NativeScalar(a), HeapValue::BigInt(b)) => native_scalar_bigint_eq(a, b.as_ref()),
            (HeapValue::BigInt(a), HeapValue::NativeScalar(b)) => native_scalar_bigint_eq(b, a.as_ref()),
            (HeapValue::NativeScalar(a), HeapValue::Decimal(b)) => {
                native_scalar_decimal_eq(a, b.as_ref())
            }
            (HeapValue::Decimal(a), HeapValue::NativeScalar(b)) => {
                native_scalar_decimal_eq(b, a.as_ref())
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod closure_variant_regression {
    //! N2 — pin Track A.5's deletion of the legacy `HeapValue::Closure`
    //! variant. After the Phase 2b HeapKind trim, the `Closure` ordinal
    //! is no longer pre-bulldozer-stable (it moved from 3 to 2 along
    //! with the rest of the trim), but the discriminator must still map
    //! to the `ClosureRaw` pipeline.
    use super::*;

    #[test]
    fn heap_kind_closure_routes_to_closure_raw() {
        // The Closure HeapKind discriminator is what HeapValue::ClosureRaw
        // returns from `kind()`; verify the routing is intact.
        // (The numeric ordinal is structural — see heap_variants.rs — and
        // not load-bearing for any external consumer per the Phase 2b
        // audit.)
        let _ = HeapKind::Closure;
    }
}

#[cfg(test)]
mod typed_object_storage_drop {
    //! ADR-006 §2.5 / Step 5: pin the Drop impl's behaviour on
    //! `TypedObjectStorage`. The contract tested:
    //!
    //! 1. Heap-mask bits cause `Arc::decrement_strong_count::<T>` for the
    //!    matching `field_kinds[i]` payload type.
    //! 2. Non-heap slots (heap_mask bit clear) are no-ops — even with
    //!    non-zero raw bits (those bits are scalar field contents, not
    //!    typed pointers).
    //! 3. `field_kinds` itself is shared via Arc — multiple instances of
    //!    the same schema share one `[NativeKind]` allocation.
    use super::*;
    use crate::native_kind::NativeKind;
    use crate::slot::ValueSlot;
    use std::sync::Arc;

    #[test]
    fn drop_decrements_arc_string_for_heap_string_slot() {
        let s: Arc<String> = Arc::new("phase-1a".to_string());
        // Hold a second strong ref so the test can observe the count drop.
        let witness = Arc::clone(&s);
        assert_eq!(Arc::strong_count(&witness), 2);

        let slot = ValueSlot::from_string_arc(s);
        let kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::String]);
        let storage = TypedObjectStorage::new(
            42,
            vec![slot].into_boxed_slice(),
            0b1, // bit 0 set
            kinds,
        );

        // Construction stored the Arc raw pointer; nothing dropped yet.
        assert_eq!(Arc::strong_count(&witness), 2);

        drop(storage);

        // Drop walked heap_mask, dispatched on NativeKind::String, and
        // released the slot's strong count via Arc::decrement_strong_count.
        assert_eq!(Arc::strong_count(&witness), 1);
    }

    #[test]
    fn drop_is_noop_for_non_heap_slot_with_non_zero_bits() {
        // Non-heap slot — heap_mask bit clear. Raw bits are an i64 value;
        // Drop must not interpret them as a pointer.
        let slot = ValueSlot::from_int(0x1234_5678);
        let kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64]);
        let storage = TypedObjectStorage::new(
            7,
            vec![slot].into_boxed_slice(),
            0, // no heap bits
            kinds,
        );
        // Just dropping the storage must not crash / dereference the bits.
        drop(storage);
    }

    #[test]
    fn drop_skips_zero_pointer_slots() {
        // Heap-mask bit set but the slot was zeroed (e.g. moved-out) —
        // Drop must not call Arc::decrement_strong_count on null.
        let slot = ValueSlot::from_raw(0);
        let kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::String]);
        let storage = TypedObjectStorage::new(
            9,
            vec![slot].into_boxed_slice(),
            0b1,
            kinds,
        );
        drop(storage);
    }

    #[test]
    fn field_kinds_arc_is_shared_across_instances() {
        // Option B' invariant: two instances of the same schema clone the
        // same Arc<[NativeKind]> (one payload allocation per schema).
        let kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64, NativeKind::Bool]);
        let kinds_count_before = Arc::strong_count(&kinds);

        let storage_a = TypedObjectStorage::new(
            1,
            vec![ValueSlot::from_int(0), ValueSlot::from_bool(true)].into_boxed_slice(),
            0,
            Arc::clone(&kinds),
        );
        let storage_b = TypedObjectStorage::new(
            1,
            vec![ValueSlot::from_int(1), ValueSlot::from_bool(false)].into_boxed_slice(),
            0,
            Arc::clone(&kinds),
        );

        // Two instances + the test's own handle = +2 over the baseline.
        assert_eq!(Arc::strong_count(&kinds), kinds_count_before + 2);

        // Both instances point at the same payload (no per-instance copy).
        assert!(Arc::ptr_eq(&storage_a.field_kinds, &storage_b.field_kinds));
        assert!(Arc::ptr_eq(&storage_a.field_kinds, &kinds));

        drop(storage_a);
        drop(storage_b);
        // Each Drop released its share; only the test's handle remains.
        assert_eq!(Arc::strong_count(&kinds), kinds_count_before);
    }

    #[test]
    fn drop_handles_mixed_heap_and_scalar_fields() {
        // Realistic shape: int + string + bool. Only the string slot
        // participates in refcount; the int/bool slots are scalar bits.
        let s: Arc<String> = Arc::new("mixed".to_string());
        let witness = Arc::clone(&s);
        assert_eq!(Arc::strong_count(&witness), 2);

        let slots = vec![
            ValueSlot::from_int(99),
            ValueSlot::from_string_arc(s),
            ValueSlot::from_bool(true),
        ]
        .into_boxed_slice();
        let kinds: Arc<[NativeKind]> = Arc::from(vec![
            NativeKind::Int64,
            NativeKind::String,
            NativeKind::Bool,
        ]);
        let storage = TypedObjectStorage::new(
            13,
            slots,
            0b010, // only bit 1 (the string) is heap
            kinds,
        );

        drop(storage);
        assert_eq!(Arc::strong_count(&witness), 1);
    }

    #[test]
    fn drop_decrements_arc_typed_object_for_heap_pointer_slot() {
        // Nested TypedObject: outer storage holds an Arc<TypedObjectStorage>
        // in slot 0 via NativeKind::Ptr(HeapKind::TypedObject).
        let inner_kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64]);
        let inner = Arc::new(TypedObjectStorage::new(
            100,
            vec![ValueSlot::from_int(7)].into_boxed_slice(),
            0,
            inner_kinds,
        ));
        let inner_witness = Arc::clone(&inner);
        assert_eq!(Arc::strong_count(&inner_witness), 2);

        let outer_kinds: Arc<[NativeKind]> =
            Arc::from(vec![NativeKind::Ptr(HeapKind::TypedObject)]);
        let outer = TypedObjectStorage::new(
            101,
            vec![ValueSlot::from_typed_object(inner)].into_boxed_slice(),
            0b1,
            outer_kinds,
        );

        drop(outer);
        assert_eq!(Arc::strong_count(&inner_witness), 1);
    }
}

#[cfg(test)]
mod hashmap_mutation {
    //! W13-hashmap-mutation (2026-05-10): pin the `insert` / `remove` /
    //! `merge` API contracts on `HashMapData`. The mutation entry-points
    //! are the storage-layer counterparts of `v2_set` / `v2_delete` /
    //! `v2_merge` in `shape-vm/executor/objects/hashmap_methods.rs` (the
    //! handlers project `KindedSlot` carriers into typed Arcs and route
    //! through these methods via `Arc::make_mut` clone-on-write).
    use super::*;
    use std::sync::Arc;
    fn k(s: &str) -> Arc<String> {
        Arc::new(s.to_string())
    }
    fn v_i(i: i64) -> Arc<HeapValue> {
        Arc::new(HeapValue::BigInt(Arc::new(i)))
    }

    #[test]
    fn insert_appends_new_entry_and_grows_index() {
        let mut m = HashMapData::new();
        m.insert(k("a"), v_i(1));
        m.insert(k("b"), v_i(2));
        assert_eq!(m.len(), 2);
        assert_eq!(m.keys.data[0].as_str(), "a");
        assert_eq!(m.keys.data[1].as_str(), "b");
        // Bucket index has registrations for both keys' hashes.
        let h_a = fnv1a_hash(b"a");
        let h_b = fnv1a_hash(b"b");
        assert!(m.index.get(&h_a).is_some());
        assert!(m.index.get(&h_b).is_some());
    }

    #[test]
    fn insert_overwrites_existing_value_and_keeps_len() {
        let mut m = HashMapData::new();
        m.insert(k("a"), v_i(1));
        m.insert(k("a"), v_i(99));
        assert_eq!(m.len(), 1);
        // get() returns the new value (BigInt(99), per v_i).
        let got = m.get("a").expect("present");
        match got.as_ref() {
            HeapValue::BigInt(b) => assert_eq!(**b, 99),
            other => panic!("unexpected value arm: {:?}", other.kind()),
        }
    }

    #[test]
    fn insert_overwrite_releases_old_value_share() {
        // Pin the in-place overwrite path drops the old Arc<HeapValue> share.
        let old: Arc<HeapValue> = v_i(1);
        let witness = Arc::clone(&old);
        assert_eq!(Arc::strong_count(&witness), 2);

        let mut m = HashMapData::new();
        m.insert(k("a"), old);
        assert_eq!(Arc::strong_count(&witness), 2);
        m.insert(k("a"), v_i(99));
        // Map no longer holds the original value's share; only the witness remains.
        assert_eq!(Arc::strong_count(&witness), 1);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn remove_present_key_drops_entry_returns_true() {
        let mut m = HashMapData::new();
        m.insert(k("a"), v_i(1));
        m.insert(k("b"), v_i(2));
        assert!(m.remove("a"));
        assert_eq!(m.len(), 1);
        assert!(m.get("a").is_none());
        // "b" should still be reachable — bucket index was updated.
        let got = m.get("b").expect("b present");
        match got.as_ref() {
            HeapValue::BigInt(bi) => assert_eq!(**bi, 2),
            _ => panic!("expected BigInt(2)"),
        }
    }

    #[test]
    fn remove_missing_key_returns_false_and_is_noop() {
        let mut m = HashMapData::new();
        m.insert(k("a"), v_i(1));
        assert!(!m.remove("nope"));
        assert_eq!(m.len(), 1);
        assert!(m.get("a").is_some());
    }

    #[test]
    fn remove_releases_value_share() {
        let v: Arc<HeapValue> = v_i(42);
        let witness = Arc::clone(&v);
        assert_eq!(Arc::strong_count(&witness), 2);

        let mut m = HashMapData::new();
        m.insert(k("a"), v);
        assert_eq!(Arc::strong_count(&witness), 2);
        assert!(m.remove("a"));
        assert_eq!(Arc::strong_count(&witness), 1);
    }

    #[test]
    fn merge_copies_other_entries_with_last_write_wins() {
        let mut a = HashMapData::new();
        a.insert(k("x"), v_i(1));
        a.insert(k("shared"), v_i(10));

        let mut b = HashMapData::new();
        b.insert(k("y"), v_i(2));
        b.insert(k("shared"), v_i(99));

        a.merge(&b);
        assert_eq!(a.len(), 3);
        // x preserved
        match a.get("x").expect("x").as_ref() {
            HeapValue::BigInt(bi) => assert_eq!(**bi, 1),
            _ => panic!(),
        }
        // y added
        match a.get("y").expect("y").as_ref() {
            HeapValue::BigInt(bi) => assert_eq!(**bi, 2),
            _ => panic!(),
        }
        // shared overwritten by b's value
        match a.get("shared").expect("shared").as_ref() {
            HeapValue::BigInt(bi) => assert_eq!(**bi, 99),
            _ => panic!(),
        }
    }

    #[test]
    fn smoke_set_set_delete_size() {
        // Mirrors the W13-hashmap-mutation smoke goal at the storage layer:
        //   let m = HashMap()
        //   m.set("a", 1); m.set("b", 2); m.delete("a"); m.size() == 1
        let mut m = HashMapData::new();
        m.insert(k("a"), v_i(1));
        m.insert(k("b"), v_i(2));
        assert!(m.remove("a"));
        assert_eq!(m.len(), 1);
        assert!(m.get("a").is_none());
        assert!(m.get("b").is_some());
    }

    #[test]
    fn arc_make_mut_clone_on_write_does_not_disturb_shared_handle() {
        // The shape-vm-side handlers `Arc::make_mut` the receiver share —
        // this tests that the underlying `HashMapData::clone()` (which
        // Arc::clone()s the inner buffers + clones the bucket-index
        // HashMap) preserves the pre-mutation observer's view.
        let mut owned = Arc::new(HashMapData::new());
        Arc::make_mut(&mut owned).insert(k("a"), v_i(1));
        // Snapshot share — second observer.
        let snapshot = Arc::clone(&owned);
        // Mutate via the local share — should clone-on-write.
        Arc::make_mut(&mut owned).insert(k("b"), v_i(2));
        assert_eq!(owned.len(), 2);
        // Snapshot is undisturbed.
        assert_eq!(snapshot.len(), 1);
        assert!(snapshot.get("a").is_some());
        assert!(snapshot.get("b").is_none());
    }
}

#[cfg(test)]
mod hashset_mutation {
    //! W13-hashset-rebuild (ADR-006 §2.7.15 / Q16, 2026-05-10): pin the
    //! `insert` / `remove` / `contains` API contracts on `HashSetData`.
    //! Mirror of `hashmap_mutation` with the values column dropped.
    use super::*;
    use std::sync::Arc;
    fn k(s: &str) -> Arc<String> {
        Arc::new(s.to_string())
    }

    #[test]
    fn empty_set_has_zero_len_and_is_empty() {
        let s = HashSetData::new();
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
        assert!(!s.contains("a"));
    }

    #[test]
    fn insert_returns_true_for_new_key_false_for_duplicate() {
        let mut s = HashSetData::new();
        assert!(s.insert(k("a")));
        assert!(s.insert(k("b")));
        assert_eq!(s.len(), 2);
        // Duplicate insert is a no-op.
        assert!(!s.insert(k("a")));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn contains_finds_inserted_keys() {
        let mut s = HashSetData::new();
        s.insert(k("a"));
        s.insert(k("b"));
        assert!(s.contains("a"));
        assert!(s.contains("b"));
        assert!(!s.contains("c"));
    }

    #[test]
    fn remove_returns_true_for_present_false_for_missing() {
        let mut s = HashSetData::new();
        s.insert(k("a"));
        s.insert(k("b"));
        assert!(s.remove("a"));
        assert!(!s.contains("a"));
        assert!(s.contains("b"));
        assert_eq!(s.len(), 1);
        // Missing-key remove is a no-op.
        assert!(!s.remove("c"));
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn from_keys_collapses_duplicates_first_wins() {
        let s = HashSetData::from_keys(vec![k("a"), k("b"), k("a"), k("c")]);
        assert_eq!(s.len(), 3);
        assert_eq!(s.keys.data[0].as_str(), "a");
        assert_eq!(s.keys.data[1].as_str(), "b");
        assert_eq!(s.keys.data[2].as_str(), "c");
    }

    #[test]
    fn smoke_target_add_two_then_size() {
        // Storage-layer counterpart of the W13-hashset-rebuild smoke
        // target: `let s = Set(); s.add("a"); s.add("b"); print(s.size())`
        // outputs 2.
        let mut s = HashSetData::new();
        s.insert(k("a"));
        s.insert(k("b"));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn arc_make_mut_clone_on_write_preserves_other_share() {
        // Pin the §2.7.4 / playbook clone-on-write invariant: when
        // `Arc<HashSetData>` has multiple shares, `Arc::make_mut`
        // clones the inner `HashSetData` so the other share stays
        // immutable. Mirror of `hashmap_mutation`'s clone-on-write
        // test.
        let mut a = Arc::new(HashSetData::new());
        Arc::make_mut(&mut a).insert(k("a"));
        let snapshot = Arc::clone(&a);
        // After the snapshot, mutating `a` clones the inner data.
        Arc::make_mut(&mut a).insert(k("b"));
        assert_eq!(a.len(), 2);
        // Snapshot retains the pre-mutation length.
        assert_eq!(snapshot.len(), 1);
        assert!(snapshot.contains("a"));
        assert!(!snapshot.contains("b"));
    }
}

#[cfg(test)]
mod deque_mutation {
    //! W15-deque (ADR-006 §2.7.19 / Q20, 2026-05-10): pin the
    //! `push_front` / `push_back` / `pop_front` / `pop_back` API
    //! contracts on `DequeData`. Mirror of `hashset_mutation` with
    //! the bucket-index dropped (Deque is order-preserving with no
    //! deduplication, so no parallel hash structure is needed).
    use super::*;
    use std::sync::Arc;

    fn s(text: &str) -> Arc<HeapValue> {
        Arc::new(HeapValue::String(Arc::new(text.to_string())))
    }

    fn i(n: i64) -> Arc<HeapValue> {
        Arc::new(HeapValue::BigInt(Arc::new(n)))
    }

    #[test]
    fn empty_deque_has_zero_len_and_is_empty() {
        let d = DequeData::new();
        assert_eq!(d.len(), 0);
        assert!(d.is_empty());
        assert!(d.peek_front().is_none());
        assert!(d.peek_back().is_none());
        assert!(d.get(0).is_none());
    }

    #[test]
    fn push_back_and_pop_front_preserve_fifo_order() {
        // FIFO: push 1,2,3 to back, pop from front yields 1,2,3.
        let mut d = DequeData::new();
        d.push_back(i(1));
        d.push_back(i(2));
        d.push_back(i(3));
        assert_eq!(d.len(), 3);
        let p1 = d.pop_front().expect("front");
        assert!(matches!(p1.as_ref(), HeapValue::BigInt(b) if **b == 1));
        let p2 = d.pop_front().expect("front");
        assert!(matches!(p2.as_ref(), HeapValue::BigInt(b) if **b == 2));
        let p3 = d.pop_front().expect("front");
        assert!(matches!(p3.as_ref(), HeapValue::BigInt(b) if **b == 3));
        assert!(d.pop_front().is_none());
    }

    #[test]
    fn push_front_and_pop_back_preserve_reverse_order() {
        // Reverse: push 1,2,3 to front, pop from back yields 1,2,3.
        let mut d = DequeData::new();
        d.push_front(i(1));
        d.push_front(i(2));
        d.push_front(i(3));
        // Now layout is [3, 2, 1] front-to-back.
        let p1 = d.pop_back().expect("back");
        assert!(matches!(p1.as_ref(), HeapValue::BigInt(b) if **b == 1));
        let p2 = d.pop_back().expect("back");
        assert!(matches!(p2.as_ref(), HeapValue::BigInt(b) if **b == 2));
        let p3 = d.pop_back().expect("back");
        assert!(matches!(p3.as_ref(), HeapValue::BigInt(b) if **b == 3));
    }

    #[test]
    fn smoke_target_push_back_push_front_pop_back() {
        // Storage-layer counterpart of the W15-deque smoke target:
        // `let d = Deque(); d.push_back(1); d.push_front(0); d.pop_back()`
        // returns `1`. After the two pushes the layout is [0, 1]
        // front-to-back; pop_back yields 1.
        let mut d = DequeData::new();
        d.push_back(i(1));
        d.push_front(i(0));
        assert_eq!(d.len(), 2);
        let popped = d.pop_back().expect("back");
        assert!(matches!(popped.as_ref(), HeapValue::BigInt(b) if **b == 1));
        // Front element retained.
        assert_eq!(d.len(), 1);
        let front = d.peek_front().expect("front").clone();
        assert!(matches!(front.as_ref(), HeapValue::BigInt(b) if **b == 0));
    }

    #[test]
    fn peek_front_back_and_get_borrow_without_removing() {
        let mut d = DequeData::new();
        d.push_back(s("a"));
        d.push_back(s("b"));
        d.push_back(s("c"));
        assert_eq!(d.len(), 3);
        let front = d.peek_front().expect("front");
        assert!(matches!(front.as_ref(), HeapValue::String(t) if t.as_str() == "a"));
        let back = d.peek_back().expect("back");
        assert!(matches!(back.as_ref(), HeapValue::String(t) if t.as_str() == "c"));
        let mid = d.get(1).expect("idx 1");
        assert!(matches!(mid.as_ref(), HeapValue::String(t) if t.as_str() == "b"));
        // Length unchanged after read-only borrows.
        assert_eq!(d.len(), 3);
    }

    #[test]
    fn from_items_preserves_insertion_order() {
        let d = DequeData::from_items(vec![s("a"), s("b"), s("c")]);
        assert_eq!(d.len(), 3);
        assert!(matches!(d.get(0).unwrap().as_ref(), HeapValue::String(t) if t.as_str() == "a"));
        assert!(matches!(d.get(1).unwrap().as_ref(), HeapValue::String(t) if t.as_str() == "b"));
        assert!(matches!(d.get(2).unwrap().as_ref(), HeapValue::String(t) if t.as_str() == "c"));
    }
}

mod priority_queue_mutation {
    //! W15-priority-queue (ADR-006 §2.7.18 / Q19, 2026-05-10): pin the
    //! `push` / `pop` / `peek` / heap-invariant API contracts on
    //! `PriorityQueueData`. Mirror of `hashset_mutation` for the
    //! cardinality-amendment shape, with i64-priority-only payload
    //! semantics per the §2.7.18 ruling.
    use super::*;
    use std::sync::Arc;

    #[test]
    fn empty_pq_has_zero_len_and_is_empty() {
        let pq = PriorityQueueData::new();
        assert_eq!(pq.len(), 0);
        assert!(pq.is_empty());
        assert_eq!(pq.peek(), None);
    }

    #[test]
    fn push_increases_len_and_pop_returns_min() {
        // Storage-layer counterpart of the W15-priority-queue smoke
        // target: `pq.push(3); pq.push(1); pq.push(2); pq.pop() == 1`.
        let mut pq = PriorityQueueData::new();
        pq.push(3);
        pq.push(1);
        pq.push(2);
        assert_eq!(pq.len(), 3);
        assert_eq!(pq.peek(), Some(1));
        assert_eq!(pq.pop(), Some(1));
        assert_eq!(pq.len(), 2);
    }

    #[test]
    fn pop_returns_none_on_empty() {
        let mut pq = PriorityQueueData::new();
        assert_eq!(pq.pop(), None);
    }

    #[test]
    fn pop_yields_ascending_order() {
        // Pin the min-heap invariant: repeated `pop()` yields keys
        // in ascending order regardless of insertion order.
        let mut pq = PriorityQueueData::new();
        for v in [5, 3, 7, 1, 9, 4, 2, 8, 6] {
            pq.push(v);
        }
        let mut out = Vec::new();
        while let Some(v) = pq.pop() {
            out.push(v);
        }
        assert_eq!(out, vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn to_sorted_vec_returns_ascending_without_consuming() {
        let mut pq = PriorityQueueData::new();
        for v in [3, 1, 4, 1, 5, 9, 2, 6] {
            pq.push(v);
        }
        let sorted = pq.to_sorted_vec();
        assert_eq!(sorted, vec![1, 1, 2, 3, 4, 5, 6, 9]);
        // Original PQ is undisturbed.
        assert_eq!(pq.len(), 8);
    }

    #[test]
    fn arc_make_mut_clone_on_write_preserves_other_share() {
        // Pin the §2.7.4 / playbook clone-on-write invariant: when
        // `Arc<DequeData>` has multiple shares, `Arc::make_mut` clones
        // the inner `DequeData` so the other share stays immutable.
        // Mirror of `hashset_mutation`'s clone-on-write test.
        let mut a = Arc::new(DequeData::new());
        Arc::make_mut(&mut a).push_back(i(1));
        let snapshot = Arc::clone(&a);
        // After the snapshot, mutating `a` clones the inner data.
        Arc::make_mut(&mut a).push_back(i(2));
        assert_eq!(a.len(), 2);
        // Snapshot retains the pre-mutation length.
        assert_eq!(snapshot.len(), 1);
    }
}

mod channel_storage {
    //! W15-channel-rebuild (ADR-006 §2.7.20 / Q21, 2026-05-10): pin the
    //! `send` / `try_recv` / `close` / `is_closed` / `len` / `is_empty`
    //! API contracts on `ChannelData`. Sync same-thread path only —
    //! cross-task blocking `recv()` is the §2.7.4 task-scheduler
    //! boundary tracked separately.
    use super::*;
    use crate::kinded_slot::KindedSlot;
    use std::sync::Arc;

    #[test]
    fn empty_channel_has_zero_len_and_is_empty_open() {
        let c = ChannelData::new();
        assert_eq!(c.len(), 0);
        assert!(c.is_empty());
        assert!(!c.is_closed());
        assert!(c.try_recv().is_none());
    }

    #[test]
    fn send_then_try_recv_round_trips_int() {
        // Storage-layer counterpart of the W15-channel-rebuild smoke
        // target: `let c = Channel(); c.send(1); c.recv()` returns 1.
        let c = ChannelData::new();
        c.send(KindedSlot::from_int(1)).expect("send on open channel");
        let got = c.try_recv().expect("queued element");
        assert_eq!(got.as_i64(), Some(1));
        assert!(c.is_empty());
    }

    #[test]
    fn fifo_send_recv_order() {
        // Producer pushes 1, 2, 3; consumer drains in the same order.
        let c = ChannelData::new();
        c.send(KindedSlot::from_int(1)).unwrap();
        c.send(KindedSlot::from_int(2)).unwrap();
        c.send(KindedSlot::from_int(3)).unwrap();
        assert_eq!(c.len(), 3);
        assert_eq!(c.try_recv().unwrap().as_i64(), Some(1));
        assert_eq!(c.try_recv().unwrap().as_i64(), Some(2));
        assert_eq!(c.try_recv().unwrap().as_i64(), Some(3));
        assert!(c.try_recv().is_none());
    }

    #[test]
    fn close_blocks_further_sends_but_drains_queued() {
        // After close, send returns Err but queued elements still
        // recv cleanly (canonical drain-on-close semantics).
        let c = ChannelData::new();
        c.send(KindedSlot::from_int(7)).unwrap();
        c.close();
        assert!(c.is_closed());
        // Further send is rejected; the rejected slot is dropped
        // (refcount discipline preserved through KindedSlot::Drop).
        assert!(c.send(KindedSlot::from_int(8)).is_err());
        // Queued element still drains.
        assert_eq!(c.try_recv().unwrap().as_i64(), Some(7));
        assert!(c.try_recv().is_none());
    }

    #[test]
    fn shared_arc_send_recv_observes_other_share() {
        // Two `Arc<ChannelData>` shares of the same channel observe
        // each other's mutations — the producer/consumer-endpoints
        // shape. Distinct from HashSet/HashMap (which are Arc::make_mut
        // clone-on-write) — Channel uses interior mutability via
        // Mutex.
        let producer = Arc::new(ChannelData::new());
        let consumer = Arc::clone(&producer);
        producer.send(KindedSlot::from_int(42)).unwrap();
        assert_eq!(consumer.len(), 1);
        let got = consumer.try_recv().unwrap();
        assert_eq!(got.as_i64(), Some(42));
        // After consumer drained, producer-side observes empty.
        assert!(producer.is_empty());
    }

    #[test]
    fn dropping_channel_with_heap_payloads_retires_shares() {
        // Refcount discipline: KindedSlot payloads queued in the
        // channel own one strong-count share; dropping the channel
        // (last `Arc<ChannelData>` share retired) must drop each
        // queued slot and retire its inner share. Any Arc-leak would
        // surface as a non-zero strong-count after the channel
        // dropped.
        let s = Arc::new("payload".to_string());
        let weak = Arc::downgrade(&s);
        let c = ChannelData::new();
        c.send(KindedSlot::from_string_arc(s)).unwrap();
        // The queued slot owns the only strong share.
        assert_eq!(weak.strong_count(), 1);
        drop(c);
        assert_eq!(
            weak.strong_count(),
            0,
            "dropped Channel must retire queued KindedSlot shares"
        );
    }

    #[test]
    fn closed_send_drops_rejected_payload_share() {
        // After close, a rejected send must NOT leak the payload
        // share — KindedSlot::Drop runs on the rejected slot.
        let c = ChannelData::new();
        c.close();
        let s = Arc::new("rejected".to_string());
        let weak = Arc::downgrade(&s);
        let slot = KindedSlot::from_string_arc(s);
        assert_eq!(weak.strong_count(), 1);
        // The rejected send consumes the slot and drops it internally.
        assert!(c.send(slot).is_err());
        assert_eq!(
            weak.strong_count(),
            0,
            "rejected-send slot must drop, not leak"
        );
        // `Arc<PriorityQueueData>` has multiple shares, `Arc::make_mut`
        // clones the inner data so the other share stays immutable.
        // Mirror of `hashset_mutation`'s clone-on-write test.
        let mut a = Arc::new(PriorityQueueData::new());
        Arc::make_mut(&mut a).push(5);
        let snapshot = Arc::clone(&a);
        // After the snapshot, mutating `a` clones the inner data.
        Arc::make_mut(&mut a).push(1);
        assert_eq!(a.len(), 2);
        assert_eq!(a.peek(), Some(1));
        // Snapshot retains the pre-mutation state.
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot.peek(), Some(5));
    }
}

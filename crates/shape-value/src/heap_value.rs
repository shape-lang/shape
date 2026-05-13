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

// ── Matrix storage (carries `HeapKind::Matrix` and `HeapKind::MatrixSlice`) ──
//
// ADR-006 §2.7.22 amendment (Round 18 S3 W12-matrix-floatslice-heapkind-exit,
// 2026-05-13): Matrix is a single Matrix value (NOT a buffer-of-Matrix), and
// exits the `TypedArrayData` carrier hierarchy. `HeapKind::Matrix = 34` +
// `HeapValue::Matrix(Arc<MatrixData>)`; FloatSlice projection becomes
// `HeapKind::MatrixSlice = 35` + `HeapValue::MatrixSlice(Arc<MatrixSliceData>)`.
// The prior §2.7.22 Q23 ruling (Matrix lives under `HeapKind::TypedArray` via
// `TypedArrayData::Matrix`) is superseded — see §2.7.22 amendment text.

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

/// Row/column projection into a parent `MatrixData` (`{ parent, offset, len }`).
///
/// ADR-006 §2.7.22 amendment (Round 18 S3 W12-matrix-floatslice-heapkind-exit,
/// 2026-05-13): FloatSlice exits the `TypedArrayData` carrier hierarchy as a
/// category-error. It is a projection-into-a-Matrix, not a buffer of floats.
/// The carrier is `Arc<MatrixSliceData>` with kind `Ptr(HeapKind::MatrixSlice)`,
/// constructed by `Matrix.row(i)` / `Matrix.col(i)` projection methods.
///
/// Aliasing semantics: the projection shares the parent Matrix's buffer
/// (mutating through the projection writes through to the parent), preserved
/// from the pre-amendment `TypedArrayData::FloatSlice` shape. The `parent`
/// Arc retains one strong-count share for the projection's lifetime.
#[derive(Debug, Clone)]
pub struct MatrixSliceData {
    pub parent: Arc<MatrixData>,
    pub offset: u32,
    pub len: u32,
}

impl MatrixSliceData {
    /// Construct a projection into a parent matrix.
    #[inline]
    pub fn new(parent: Arc<MatrixData>, offset: u32, len: u32) -> Self {
        Self { parent, offset, len }
    }

    /// Borrow the underlying slice into the parent's flat data buffer.
    #[inline]
    pub fn as_slice(&self) -> &[f64] {
        let off = self.offset as usize;
        let n = self.len as usize;
        &self.parent.data.as_slice()[off..off + n]
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

// ── TraitObjectStorage now lives at lines ~1790+ (W17-trait-object-storage merge) ──
// The placeholder shape from W17-typed-carrier-bundle-A checkpoint 1 is
// superseded by the real `TraitObjectStorage { value: Arc<TypedObjectStorage>,
// vtable: Arc<VTable> }` from W17-trait-object-storage's close commit
// (`e58218c`). 4-table lockstep landed there; HeapKind::TraitObject = 29
// is the assigned ordinal.

// ── HashMap storage (Stage C P1(b), 2026-05-07) ─────────────────────────────

/// Parametric value-buffer for `HashMapData`, per ADR-006 §2.7.24 Q25.B.
///
/// W17-typed-carrier-bundle-A commit 1/4 (2026-05-11): replaces the previous
/// `Arc<TypedBuffer<Arc<HeapValue>>>` shape with a discriminated enum where
/// each arm pins the per-value kind at the variant level. No parallel kind
/// track is needed at this layer — the variant tag IS the kind.
///
/// The `HeapValue` arm is retained during commits 1-3 to keep existing
/// readers/writers compiling; commit 4 deletes it (Q25.A/B forbidden-pattern
/// list). Construction sites migrate to the specialized arms in commit 2;
/// read sites add per-arm dispatch in commit 3.
#[derive(Debug)]
pub enum HashMapValueBuf {
    // Scalar-keyed value arms (mirror of TypedArrayData scalar arms).
    I64(Arc<crate::typed_buffer::TypedBuffer<i64>>),
    F64(Arc<crate::typed_buffer::TypedBuffer<f64>>),
    Bool(Arc<crate::typed_buffer::TypedBuffer<u8>>),
    String(Arc<crate::typed_buffer::TypedBuffer<Arc<String>>>),
    // Heap-typed value arms — payload type mirrors the matching `HeapValue`
    // variant's `Arc<T>` payload so refcount discipline reuses the existing
    // `Arc::clone` / `Arc::drop` paths.
    Decimal(Arc<crate::typed_buffer::TypedBuffer<Arc<rust_decimal::Decimal>>>),
    BigInt(Arc<crate::typed_buffer::TypedBuffer<Arc<i64>>>),
    DateTime(Arc<crate::typed_buffer::TypedBuffer<Arc<TemporalData>>>),
    Timespan(Arc<crate::typed_buffer::TypedBuffer<Arc<TemporalData>>>),
    Duration(Arc<crate::typed_buffer::TypedBuffer<Arc<TemporalData>>>),
    Instant(Arc<crate::typed_buffer::TypedBuffer<Arc<std::time::Instant>>>),
    Char(Arc<crate::typed_buffer::TypedBuffer<char>>),
    TypedObject(Arc<crate::typed_buffer::TypedBuffer<Arc<TypedObjectStorage>>>),
    TraitObject(Arc<crate::typed_buffer::TypedBuffer<Arc<TraitObjectStorage>>>),
    // The polymorphic `HeapValue(Arc<TypedBuffer<Arc<HeapValue>>>)` catch-all
    // was DELETED in checkpoint 4 of W17-typed-carrier-bundle-A per ADR-006
    // §2.7.24 Q25.B. Same discipline as Q25.A for TypedArrayData — no
    // production caller produces this shape; the value buffer is fully
    // per-variant strict-typed.
}

impl HashMapValueBuf {
    /// Number of values in the buffer.
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            HashMapValueBuf::I64(b) => b.data.len(),
            HashMapValueBuf::F64(b) => b.data.len(),
            HashMapValueBuf::Bool(b) => b.data.len(),
            HashMapValueBuf::String(b) => b.data.len(),
            HashMapValueBuf::Decimal(b) => b.data.len(),
            HashMapValueBuf::BigInt(b) => b.data.len(),
            HashMapValueBuf::DateTime(b) => b.data.len(),
            HashMapValueBuf::Timespan(b) => b.data.len(),
            HashMapValueBuf::Duration(b) => b.data.len(),
            HashMapValueBuf::Instant(b) => b.data.len(),
            HashMapValueBuf::Char(b) => b.data.len(),
            HashMapValueBuf::TypedObject(b) => b.data.len(),
            HashMapValueBuf::TraitObject(b) => b.data.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterate over all values materialized as `Arc<HeapValue>`. Helper
    /// for callers that need to walk the buffer once during the
    /// W17-typed-carrier-bundle-A transition. Commit 3 audits readers and
    /// converts hot paths to per-arm dispatch; this generic helper covers
    /// cold paths (Display, JSON/XML marshal) that don't need per-element
    /// specialisation.
    pub fn iter_heap_arcs(&self) -> impl Iterator<Item = Arc<HeapValue>> + '_ {
        let n = self.len();
        (0..n).map(move |i| self.value_at(i))
    }

    /// Materialize the value at index `i` as an owned `Arc<HeapValue>`.
    /// Cheap thin Arc::clone for the HeapValue arm; for typed arms the
    /// element is wrapped into a freshly-constructed HeapValue.
    ///
    /// This is the read-side accessor used by callers that haven't yet
    /// migrated to per-arm dispatch (commits 1-3 of W17-typed-carrier-
    /// bundle-A). Commit 3 audits and rewrites match-on-HeapValue sites
    /// into exhaustive HashMapValueBuf matches; commit 4 deletes the
    /// HeapValue arm.
    pub fn value_at(&self, i: usize) -> Arc<HeapValue> {
        match self {
            HashMapValueBuf::I64(b) => {
                Arc::new(HeapValue::NativeScalar(NativeScalar::I64(b.data[i])))
            }
            HashMapValueBuf::F64(_) => {
                // NativeScalar has no F64 arm (only F32); F64 values need to
                // round-trip through a Temporal-style typed payload or the
                // caller migrates to per-arm dispatch in commit 3. No
                // construction site for HashMapValueBuf::F64 on this branch,
                // so reaching this arm is a wiring bug.
                unreachable!(
                    "HashMapValueBuf::F64 read via value_at — no construction \
                     site on this branch (W17-typed-carrier-bundle-A commit 1)"
                )
            }
            HashMapValueBuf::Bool(_) => {
                // NativeScalar has no Bool arm. Same disposition as F64
                // above — caller migrates to per-arm dispatch.
                unreachable!(
                    "HashMapValueBuf::Bool read via value_at — no construction \
                     site on this branch (W17-typed-carrier-bundle-A commit 1)"
                )
            }
            HashMapValueBuf::String(b) => Arc::new(HeapValue::String(Arc::clone(&b.data[i]))),
            HashMapValueBuf::Decimal(b) => Arc::new(HeapValue::Decimal(Arc::clone(&b.data[i]))),
            HashMapValueBuf::BigInt(b) => Arc::new(HeapValue::BigInt(Arc::clone(&b.data[i]))),
            HashMapValueBuf::DateTime(b)
            | HashMapValueBuf::Timespan(b)
            | HashMapValueBuf::Duration(b) => {
                Arc::new(HeapValue::Temporal(Arc::clone(&b.data[i])))
            }
            HashMapValueBuf::Instant(b) => Arc::new(HeapValue::Instant(Arc::clone(&b.data[i]))),
            HashMapValueBuf::Char(b) => Arc::new(HeapValue::Char(b.data[i])),
            HashMapValueBuf::TypedObject(b) => {
                Arc::new(HeapValue::TypedObject(Arc::clone(&b.data[i])))
            }
            HashMapValueBuf::TraitObject(_) => {
                // No construction site for the TraitObject arm on this
                // branch. Reaching this point indicates the
                // W17-trait-object-storage carrier landed independently;
                // surface-and-stop until the boxing thunk and HeapValue
                // arm are in place (§2.7.24 Q25.C).
                unreachable!(
                    "HashMapValueBuf::TraitObject reader called before \
                     W17-trait-object-storage / W17-trait-object-emission landed; \
                     ADR-006 §2.7.24 Q25.C"
                )
            }
        }
    }
}

impl Clone for HashMapValueBuf {
    fn clone(&self) -> Self {
        match self {
            HashMapValueBuf::I64(b) => HashMapValueBuf::I64(Arc::clone(b)),
            HashMapValueBuf::F64(b) => HashMapValueBuf::F64(Arc::clone(b)),
            HashMapValueBuf::Bool(b) => HashMapValueBuf::Bool(Arc::clone(b)),
            HashMapValueBuf::String(b) => HashMapValueBuf::String(Arc::clone(b)),
            HashMapValueBuf::Decimal(b) => HashMapValueBuf::Decimal(Arc::clone(b)),
            HashMapValueBuf::BigInt(b) => HashMapValueBuf::BigInt(Arc::clone(b)),
            HashMapValueBuf::DateTime(b) => HashMapValueBuf::DateTime(Arc::clone(b)),
            HashMapValueBuf::Timespan(b) => HashMapValueBuf::Timespan(Arc::clone(b)),
            HashMapValueBuf::Duration(b) => HashMapValueBuf::Duration(Arc::clone(b)),
            HashMapValueBuf::Instant(b) => HashMapValueBuf::Instant(Arc::clone(b)),
            HashMapValueBuf::Char(b) => HashMapValueBuf::Char(Arc::clone(b)),
            HashMapValueBuf::TypedObject(b) => HashMapValueBuf::TypedObject(Arc::clone(b)),
            HashMapValueBuf::TraitObject(b) => HashMapValueBuf::TraitObject(Arc::clone(b)),
        }
    }
}

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
    /// Insertion-ordered values, parametric per ADR-006 §2.7.24 Q25.B.
    /// Variant tag IS the per-value kind — no parallel kind track.
    pub values: HashMapValueBuf,
    /// Eager bucket-index: hash → list of indices into `keys`/`values`
    /// arrays. Enables O(1) lookup at the user-facing `map.get(key)`
    /// path. Hash is computed via FNV-1a over the key string bytes.
    pub index: std::collections::HashMap<u64, Vec<u32>>,
}

impl HashMapData {
    /// Build an empty HashMapData with no entries.
    ///
    /// W17-typed-carrier-bundle-A checkpoint 4/4: the post-§2.7.24 Q25.B
    /// default arm is `HashMapValueBuf::TypedObject` — when callers
    /// `.set(key, value)` for the first time, the per-value-kind dispatch
    /// in `insert` selects the matching specialized arm. The TypedObject
    /// default lets cold callers that never write (just read empty
    /// entries/values arrays) avoid panicking on an unselected arm.
    pub fn new() -> Self {
        Self {
            keys: Arc::new(crate::typed_buffer::TypedBuffer::from_vec(Vec::new())),
            values: HashMapValueBuf::TypedObject(Arc::new(
                crate::typed_buffer::TypedBuffer::from_vec(Vec::new()),
            )),
            index: std::collections::HashMap::new(),
        }
    }

    /// Build from parallel `Vec`s of keys and values, computing the
    /// bucket index eagerly. Panics if `keys.len() != values.len()`.
    ///
    /// W17-typed-carrier-bundle-A checkpoint 4/4: dispatches to a
    /// specialized `HashMapValueBuf` arm via per-element `HeapValue`
    /// inspection per ADR-006 §2.7.24 Q25.B (mirror of
    /// `TypedArrayData::build_specialized_from_heap_arcs`). Heterogeneous-
    /// arm value vectors surface (production callers that produced mixed
    /// HeapValue arms migrate by either uniform-typing the input or
    /// using a typed-arm constructor).
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
        let values_buf = Self::specialize_values(values);
        Self {
            keys: Arc::new(crate::typed_buffer::TypedBuffer::from_vec(keys)),
            values: values_buf,
            index,
        }
    }

    /// W17-typed-carrier-bundle-A checkpoint 4/4: dispatch a
    /// `Vec<Arc<HeapValue>>` into the matching specialized
    /// `HashMapValueBuf` arm. Empty inputs default to `TypedObject`.
    /// Mirror of `TypedArrayData::build_specialized_from_heap_arcs`.
    fn specialize_values(values: Vec<Arc<HeapValue>>) -> HashMapValueBuf {
        use crate::typed_buffer::TypedBuffer;
        if values.is_empty() {
            return HashMapValueBuf::TypedObject(Arc::new(TypedBuffer::from_vec(Vec::new())));
        }
        let first = &values[0];
        match first.as_ref() {
            HeapValue::String(_) => {
                let data: Vec<Arc<String>> = values
                    .iter()
                    .map(|v| match v.as_ref() {
                        HeapValue::String(s) => Arc::clone(s),
                        other => panic!(
                            "HashMapData::from_pairs: heterogeneous value arms \
                             (expected String, got {:?})",
                            other.kind()
                        ),
                    })
                    .collect();
                HashMapValueBuf::String(Arc::new(TypedBuffer::from_vec(data)))
            }
            HeapValue::Decimal(_) => {
                let data: Vec<Arc<rust_decimal::Decimal>> = values
                    .iter()
                    .map(|v| match v.as_ref() {
                        HeapValue::Decimal(d) => Arc::clone(d),
                        other => panic!(
                            "HashMapData::from_pairs: heterogeneous value arms \
                             (expected Decimal, got {:?})",
                            other.kind()
                        ),
                    })
                    .collect();
                HashMapValueBuf::Decimal(Arc::new(TypedBuffer::from_vec(data)))
            }
            HeapValue::BigInt(_) => {
                let data: Vec<Arc<i64>> = values
                    .iter()
                    .map(|v| match v.as_ref() {
                        HeapValue::BigInt(b) => Arc::clone(b),
                        other => panic!(
                            "HashMapData::from_pairs: heterogeneous value arms \
                             (expected BigInt, got {:?})",
                            other.kind()
                        ),
                    })
                    .collect();
                HashMapValueBuf::BigInt(Arc::new(TypedBuffer::from_vec(data)))
            }
            HeapValue::TypedObject(_) => {
                let data: Vec<Arc<TypedObjectStorage>> = values
                    .iter()
                    .map(|v| match v.as_ref() {
                        HeapValue::TypedObject(s) => Arc::clone(s),
                        other => panic!(
                            "HashMapData::from_pairs: heterogeneous value arms \
                             (expected TypedObject, got {:?})",
                            other.kind()
                        ),
                    })
                    .collect();
                HashMapValueBuf::TypedObject(Arc::new(TypedBuffer::from_vec(data)))
            }
            HeapValue::Char(_) => {
                let data: Vec<char> = values
                    .iter()
                    .map(|v| match v.as_ref() {
                        HeapValue::Char(c) => *c,
                        other => panic!(
                            "HashMapData::from_pairs: heterogeneous value arms \
                             (expected Char, got {:?})",
                            other.kind()
                        ),
                    })
                    .collect();
                HashMapValueBuf::Char(Arc::new(TypedBuffer::from_vec(data)))
            }
            other => panic!(
                "HashMapData::from_pairs: HeapValue arm {:?} not yet \
                 supported post-§2.7.24 Q25.B — add a specialized \
                 HashMapValueBuf arm.",
                other.kind()
            ),
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
    ///
    /// Returns an **owned** `Arc<HeapValue>` rather than a borrow because
    /// post-§2.7.24 the value buffer can be a specialized variant
    /// (`HashMapValueBuf::I64`, etc.) where there is no underlying
    /// `Arc<HeapValue>` to borrow — the materialized HeapValue is
    /// freshly constructed by `HashMapValueBuf::value_at`. For the
    /// HeapValue arm this is a thin atomic refcount bump.
    pub fn get(&self, key: &str) -> Option<Arc<HeapValue>> {
        let hash = fnv1a_hash(key.as_bytes());
        let bucket = self.index.get(&hash)?;
        for &idx in bucket {
            let i = idx as usize;
            if self.keys.data[i].as_str() == key {
                return Some(self.values.value_at(i));
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
    ///
    /// Per ADR-006 §2.7.24 Q25.B this mutation entry-point assumes the
    /// values buffer is the `HashMapValueBuf::HeapValue` arm. Specialized
    /// arms get their own typed mutation entry-points in commit 2; until
    /// the migration completes, attempting to insert into a specialized
    /// arm panics (unreachable in current callers — commit 4 deletes the
    /// HeapValue arm and the specialized paths replace this entry-point).
    pub fn insert(&mut self, key: Arc<String>, value: Arc<HeapValue>) {
        let hash = fnv1a_hash(key.as_bytes());
        // Look for an existing entry under the same hash bucket.
        if let Some(bucket) = self.index.get(&hash) {
            for &idx in bucket {
                let i = idx as usize;
                if self.keys.data[i].as_str() == key.as_str() {
                    // Overwrite in place via the HeapValue-arm-aware
                    // helper. Specialized arms have their own typed
                    // insert paths (commit 2 / commit 3).
                    self.insert_heap_overwrite_at(i, value);
                    return;
                }
            }
        }
        // New entry: append to keys + values, then register in the index.
        let new_idx = self.keys.data.len();
        Arc::make_mut(&mut self.keys).data.push(key);
        self.push_heap_value(value);
        self.index.entry(hash).or_default().push(new_idx as u32);
    }

    /// W17-typed-carrier-bundle-A checkpoint 4/4: per-arm overwrite-in-place
    /// at index `i`. Dispatches on the current `HashMapValueBuf` arm and
    /// requires `value`'s HeapValue arm to match.
    fn insert_heap_overwrite_at(&mut self, i: usize, value: Arc<HeapValue>) {
        match (&mut self.values, value.as_ref()) {
            (HashMapValueBuf::String(buf), HeapValue::String(s)) => {
                Arc::make_mut(buf).data[i] = Arc::clone(s);
            }
            (HashMapValueBuf::Decimal(buf), HeapValue::Decimal(d)) => {
                Arc::make_mut(buf).data[i] = Arc::clone(d);
            }
            (HashMapValueBuf::BigInt(buf), HeapValue::BigInt(b)) => {
                Arc::make_mut(buf).data[i] = Arc::clone(b);
            }
            (HashMapValueBuf::TypedObject(buf), HeapValue::TypedObject(s)) => {
                Arc::make_mut(buf).data[i] = Arc::clone(s);
            }
            (HashMapValueBuf::Char(buf), HeapValue::Char(c)) => {
                Arc::make_mut(buf).data[i] = *c;
            }
            (existing, incoming) => panic!(
                "HashMapData::insert: value arm mismatch (buf={:?}, value={:?}) — \
                 callers must produce values of the same kind as the existing entries; \
                 ADR-006 §2.7.24 Q25.B forbids mid-life arm changes",
                std::mem::discriminant(existing),
                incoming.kind()
            ),
        }
    }

    /// W17-typed-carrier-bundle-A checkpoint 4/4: append a value to the
    /// current arm. On an empty map (TypedObject default from `new()`),
    /// the first push selects the arm; subsequent pushes require the
    /// same HeapValue kind.
    fn push_heap_value(&mut self, value: Arc<HeapValue>) {
        // Empty-values first-push: re-target the arm to match `value`'s
        // kind. `keys` may already have the new key pushed by `insert`
        // (the caller `insert` pushes key first, then this); the values
        // arm-selection key is the buffer's own emptiness.
        if matches!(&self.values, HashMapValueBuf::TypedObject(b) if b.data.is_empty()) {
            self.values = match value.as_ref() {
                HeapValue::String(_) => HashMapValueBuf::String(Arc::new(
                    crate::typed_buffer::TypedBuffer::from_vec(Vec::new()),
                )),
                HeapValue::Decimal(_) => HashMapValueBuf::Decimal(Arc::new(
                    crate::typed_buffer::TypedBuffer::from_vec(Vec::new()),
                )),
                HeapValue::BigInt(_) => HashMapValueBuf::BigInt(Arc::new(
                    crate::typed_buffer::TypedBuffer::from_vec(Vec::new()),
                )),
                HeapValue::TypedObject(_) => HashMapValueBuf::TypedObject(Arc::new(
                    crate::typed_buffer::TypedBuffer::from_vec(Vec::new()),
                )),
                HeapValue::Char(_) => HashMapValueBuf::Char(Arc::new(
                    crate::typed_buffer::TypedBuffer::from_vec(Vec::new()),
                )),
                _ => {
                    // Other kinds default to TypedObject (already set);
                    // the next match arm will panic with the precise reason.
                    return self.push_heap_value_typed_arm(value);
                }
            };
        }
        self.push_heap_value_typed_arm(value)
    }

    /// Append to the (already-selected) typed arm.
    fn push_heap_value_typed_arm(&mut self, value: Arc<HeapValue>) {
        match (&mut self.values, value.as_ref()) {
            (HashMapValueBuf::String(buf), HeapValue::String(s)) => {
                Arc::make_mut(buf).data.push(Arc::clone(s));
            }
            (HashMapValueBuf::Decimal(buf), HeapValue::Decimal(d)) => {
                Arc::make_mut(buf).data.push(Arc::clone(d));
            }
            (HashMapValueBuf::BigInt(buf), HeapValue::BigInt(b)) => {
                Arc::make_mut(buf).data.push(Arc::clone(b));
            }
            (HashMapValueBuf::TypedObject(buf), HeapValue::TypedObject(s)) => {
                Arc::make_mut(buf).data.push(Arc::clone(s));
            }
            (HashMapValueBuf::Char(buf), HeapValue::Char(c)) => {
                Arc::make_mut(buf).data.push(*c);
            }
            (existing, incoming) => panic!(
                "HashMapData::push: value arm {:?} does not match buf {:?} \
                 (ADR-006 §2.7.24 Q25.B — values buffer is monomorphic)",
                incoming.kind(),
                std::mem::discriminant(existing),
            ),
        }
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
        self.remove_value_at(removed_idx);
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

    /// Per-arm remove-at-index for the values buffer. Each arm reaches
    /// into its typed buffer via `Arc::make_mut`. Refcount discipline is
    /// per-arm at compile time — no `Arc<HeapValue>` dispatch hop.
    fn remove_value_at(&mut self, i: usize) {
        match &mut self.values {
            HashMapValueBuf::I64(b) => {
                Arc::make_mut(b).data.remove(i);
            }
            HashMapValueBuf::F64(b) => {
                Arc::make_mut(b).data.remove(i);
            }
            HashMapValueBuf::Bool(b) => {
                Arc::make_mut(b).data.remove(i);
            }
            HashMapValueBuf::String(b) => {
                Arc::make_mut(b).data.remove(i);
            }
            HashMapValueBuf::Decimal(b) => {
                Arc::make_mut(b).data.remove(i);
            }
            HashMapValueBuf::BigInt(b) => {
                Arc::make_mut(b).data.remove(i);
            }
            HashMapValueBuf::DateTime(b)
            | HashMapValueBuf::Timespan(b)
            | HashMapValueBuf::Duration(b) => {
                Arc::make_mut(b).data.remove(i);
            }
            HashMapValueBuf::Instant(b) => {
                Arc::make_mut(b).data.remove(i);
            }
            HashMapValueBuf::Char(b) => {
                Arc::make_mut(b).data.remove(i);
            }
            HashMapValueBuf::TypedObject(b) => {
                Arc::make_mut(b).data.remove(i);
            }
            HashMapValueBuf::TraitObject(b) => {
                Arc::make_mut(b).data.remove(i);
            }
        }
    }

    /// Merge entries from `other` into `self`. Keys present in both maps
    /// take the value from `other` (last-write-wins, matching `Object.assign`
    /// / `dict.update` semantics). Per-entry insert path — the bucket index
    /// is maintained incrementally.
    pub fn merge(&mut self, other: &HashMapData) {
        let n = other.len();
        for i in 0..n {
            let key = Arc::clone(&other.keys.data[i]);
            let value = other.values.value_at(i);
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
        // of this crate's scope). The value-buffer enum's hand-written
        // Clone impl Arc::clones the inner typed buffer per arm.
        Self {
            keys: Arc::clone(&self.keys),
            values: self.values.clone(),
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

// ── Result / Option storage (ADR-006 §2.7.17 / Q18, W14-variant-codegen) ────
//
// Wave 14 W14-variant-codegen amendment: Result<T,E> and Option<T> are
// represented as kinded carriers `Arc<ResultData>` / `Arc<OptionData>`
// holding (a) a `is_ok` / `is_some` discriminator boolean and (b) a single
// payload `KindedSlot` carrying one strong-count share for the inner
// value. Mirrors the §2.7.16 IteratorState typed-Arc shape and the §2.5
// AnyError schema-keyed kind discipline (per-slot kind threaded
// alongside slot bits, drop dispatched on the kind label). Slot bits at
// the §2.7.7 stack tier are `Arc::into_raw(Arc<ResultData>)` /
// `Arc::into_raw(Arc<OptionData>)` directly with kind labels
// `NativeKind::Ptr(HeapKind::Result)` / `NativeKind::Ptr(HeapKind::Option)`.
//
// The payload `KindedSlot` lives inside the typed-Arc so the value's
// strong-count share is owned by the wrapper for the wrapper's lifetime;
// `KindedSlot::Drop` retires the inner share when the wrapper Drop runs
// (Arc refcount reaches zero). On clone, `KindedSlot::Clone` bumps the
// inner share. Same recursion-through-Arc discipline as
// `IteratorTransform::Map(Arc<HeapValue>)` per §2.7.16.

/// Result<T, E> carrier. `is_ok` discriminates Ok vs Err; `payload` carries
/// the inner value (`T` for Ok, `E` for Err). Both arms share the same
/// payload slot — the variant tag is the discriminator, not the slot's
/// physical layout.
#[derive(Debug)]
pub struct ResultData {
    pub is_ok: bool,
    pub payload: crate::kinded_slot::KindedSlot,
}

impl ResultData {
    /// Construct an Ok-tagged result.
    #[inline]
    pub fn ok(payload: crate::kinded_slot::KindedSlot) -> Self {
        Self { is_ok: true, payload }
    }

    /// Construct an Err-tagged result.
    #[inline]
    pub fn err(payload: crate::kinded_slot::KindedSlot) -> Self {
        Self { is_ok: false, payload }
    }
}

impl Clone for ResultData {
    /// Per-field clone — `KindedSlot::Clone` bumps the payload's
    /// strong-count share.
    fn clone(&self) -> Self {
        Self {
            is_ok: self.is_ok,
            payload: self.payload.clone(),
        }
    }
}

/// Option<T> carrier. `is_some` discriminates Some vs None; `payload`
/// carries the inner value for Some. For None the payload is a
/// `KindedSlot::none()` placeholder (Bool-kind, zero bits) so
/// `KindedSlot::Drop` is a no-op.
#[derive(Debug)]
pub struct OptionData {
    pub is_some: bool,
    pub payload: crate::kinded_slot::KindedSlot,
}

impl OptionData {
    /// Construct a Some-tagged option.
    #[inline]
    pub fn some(payload: crate::kinded_slot::KindedSlot) -> Self {
        Self { is_some: true, payload }
    }

    /// Construct a None-tagged option (payload is a no-op KindedSlot).
    #[inline]
    pub fn none() -> Self {
        Self {
            is_some: false,
            payload: crate::kinded_slot::KindedSlot::none(),
        }
    }
}

impl Clone for OptionData {
    /// Per-field clone — `KindedSlot::Clone` bumps the payload's
    /// strong-count share. For None the payload is a zero-bits Bool
    /// slot; clone is a no-op refcount-wise.
    fn clone(&self) -> Self {
        Self {
            is_some: self.is_some,
            payload: self.payload.clone(),
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

// ── Mutex / Atomic / Lazy storage (Wave 17 W17-concurrency,
//    ADR-006 §2.7.25, 2026-05-11) ──────────────────────────────────────────
//
// W17-concurrency rebuild: Mutex, Atomic, and Lazy are the three
// concurrency primitives left SURFACE'd by the strict-typing Phase-2
// bulldozer (the `HeapValue::Concurrency(ConcurrencyData::*)` enum form
// was deleted alongside `ValueWord`; see the deletion-fate comment in
// `executor/objects/concurrency_methods.rs:1-22`). Each lands as its own
// typed-Arc HeapValue arm per ADR-006 §2.3 / §2.7.25, mirror of the
// §2.7.20 Channel rebuild structure:
//
// - `Mutex<T>` carries a single `KindedSlot` payload protected by a
//   `Mutex<MutexInner>`. Like `ChannelData`, two `Arc<MutexData>` shares
//   observe each other's mutations — the canonical "shared cell with
//   exclusion" shape. `lock()` is a no-op marker at landing (single-
//   threaded VM); the contract is that the inner value is mutated under
//   exclusion. `try_lock()` mirrors `lock()`. `set(value)` swaps the
//   inner payload (KindedSlot Drop retires the prior share).
// - `Atomic<i64>` carries a `std::sync::atomic::AtomicI64` for the
//   atomic operations (`load`, `store`, `fetch_add`, `fetch_sub`,
//   `compare_exchange`). i64-only at landing per the playbook's
//   "i64-priority" / "string-only" precedents (W15-priority-queue,
//   W13-hashset). A typed-payload `Atomic<T>` is a future amendment
//   with measurement.
// - `Lazy<T>` carries a `Mutex<LazyInner>` wrapping `(initializer:
//   Option<KindedSlot>, value: Option<KindedSlot>)`. `get()` returns
//   the cached value or runs the initializer closure (closure-call
//   path unlocked by W17-make-closure, merged at `aa47364`).
//   `is_initialized()` returns whether the value has been computed.
//
// Forbidden shapes refused (per CLAUDE.md "Renames to refuse on sight"
// + playbook §3 W17-concurrency forbidden list):
//
// - Generic "concurrency primitive" wrapper (`ConcurrencyData` enum
//   shape from the deleted form). Each primitive is its own typed-Arc
//   HeapValue arm.
// - Inline-scalar Mutex/Atomic carriers (these are always heap — the
//   semantic identity is "this is a shared cell with mutation", which
//   has no inline-scalar reduction).
// - Re-using `HeapKind::SharedCell` for Mutex (different semantics —
//   `SharedCell` is binding-storage interior-mutability for `var`
//   binding-form values, while `MutexData` is a runtime synchronization
//   primitive user code asks for explicitly).

/// `Mutex<T>` storage — a single typed payload protected by a Rust
/// `Mutex` so concurrent `Arc<MutexData>` shares observe each other's
/// mutations (the canonical "shared cell with exclusion" shape, mirror
/// of `ChannelData`'s `Mutex<ChannelInner>` interior-mutability shape).
///
/// At landing the VM is single-threaded so `lock()` / `try_lock()` are
/// no-op markers — the contract they preserve is "the inner value is
/// mutated under exclusion" (the same contract user code reasons
/// about). When the VM grows real concurrency, the same `Mutex` here
/// will serialize concurrent `lock()` calls without API churn.
///
/// The inner `Option<KindedSlot>` carries one strong-count share for
/// the wrapped value when present; `take()` / `replace()` discipline
/// preserves the share-discipline across `set(...)` (the old slot
/// drops, the new slot is owned by the cell).
#[derive(Debug)]
pub struct MutexData {
    inner: std::sync::Mutex<MutexInner>,
}

#[derive(Debug)]
struct MutexInner {
    /// Wrapped value. `None` only transiently between `take` and replace
    /// during `set(...)` — never observable externally.
    value: Option<crate::kinded_slot::KindedSlot>,
}

impl MutexData {
    /// Build a `MutexData` wrapping `value`.
    pub fn new(value: crate::kinded_slot::KindedSlot) -> Self {
        Self {
            inner: std::sync::Mutex::new(MutexInner { value: Some(value) }),
        }
    }

    /// `lock()` — at landing a no-op marker (single-threaded VM). When
    /// the runtime grows real concurrency, this is the acquire point
    /// for the inner `std::sync::Mutex`.
    pub fn lock(&self) {
        let _g = self.inner.lock().expect("mutex poisoned");
    }

    /// `try_lock()` — at landing always returns true (single-threaded
    /// VM; there's no contention to fail). Mirror of `lock()`.
    pub fn try_lock(&self) -> bool {
        self.inner.try_lock().is_ok()
    }

    /// Read the current value (clone of the inner `KindedSlot`).
    /// `KindedSlot::Clone` bumps the inner share so the returned slot
    /// is independently owned.
    pub fn get(&self) -> crate::kinded_slot::KindedSlot {
        let inner = self.inner.lock().expect("mutex poisoned");
        inner
            .value
            .as_ref()
            .expect("mutex value present")
            .clone()
    }

    /// Replace the wrapped value. The prior slot drops here
    /// (`KindedSlot::Drop` retires its inner share); the new slot is
    /// owned by the cell.
    pub fn set(&self, new_value: crate::kinded_slot::KindedSlot) {
        let mut inner = self.inner.lock().expect("mutex poisoned");
        inner.value = Some(new_value);
    }
}

/// `Atomic<i64>` storage — wraps a `std::sync::atomic::AtomicI64` for
/// the atomic operations exposed by the `Atomic.load` / `store` /
/// `fetch_add` / `fetch_sub` / `compare_exchange` method surface.
///
/// **i64-only at landing** per the playbook's typed-payload deferral
/// precedent (W15-priority-queue i64-priority-only). A typed-payload
/// `Atomic<T>` is a future Phase-2c amendment with measurement.
///
/// Memory ordering is `SeqCst` (sequential consistency) throughout —
/// the simplest semantically-correct ordering. Relaxed-ordering
/// optimizations are a measured follow-up.
#[derive(Debug)]
pub struct AtomicData {
    value: std::sync::atomic::AtomicI64,
}

impl AtomicData {
    /// Build an `AtomicData` with initial value `init`.
    pub fn new(init: i64) -> Self {
        Self {
            value: std::sync::atomic::AtomicI64::new(init),
        }
    }

    /// Atomic load (SeqCst).
    pub fn load(&self) -> i64 {
        self.value.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Atomic store (SeqCst).
    pub fn store(&self, v: i64) {
        self.value.store(v, std::sync::atomic::Ordering::SeqCst)
    }

    /// Atomic fetch-add (SeqCst). Returns the prior value.
    pub fn fetch_add(&self, delta: i64) -> i64 {
        self.value
            .fetch_add(delta, std::sync::atomic::Ordering::SeqCst)
    }

    /// Atomic fetch-sub (SeqCst). Returns the prior value.
    pub fn fetch_sub(&self, delta: i64) -> i64 {
        self.value
            .fetch_sub(delta, std::sync::atomic::Ordering::SeqCst)
    }

    /// Atomic compare-exchange (SeqCst). Returns the prior value
    /// regardless of success — callers infer success by comparing to
    /// `expected`.
    pub fn compare_exchange(&self, expected: i64, new_v: i64) -> i64 {
        match self.value.compare_exchange(
            expected,
            new_v,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        ) {
            Ok(prev) => prev,
            Err(prev) => prev,
        }
    }
}

/// `Lazy<T>` storage — wraps an initializer closure (`KindedSlot` of
/// kind `Ptr(HeapKind::Closure)`) and a cached value slot. `get()`
/// runs the initializer the first time and caches the result;
/// subsequent calls return the cached value.
///
/// Closure-call dispatch (`vm.call_value_immediate_nb`) is unlocked by
/// the W17-make-closure partial-gate (merged at `aa47364`); see
/// `executor/objects/concurrency_methods.rs::v2_lazy_get` for the
/// closure-call body.
///
/// **`Mutex<LazyInner>` for interior mutability**: like `ChannelData`,
/// two `Arc<LazyData>` shares observe each other's initialization
/// state. The `OnceCell`-style "init only happens once" guarantee is
/// preserved by the inner Mutex serializing concurrent `get()` calls
/// when the runtime grows real concurrency. At landing (single-
/// threaded VM) the mutex is uncontended.
#[derive(Debug)]
pub struct LazyData {
    inner: std::sync::Mutex<LazyInner>,
}

#[derive(Debug)]
struct LazyInner {
    /// Initializer closure (`KindedSlot` of kind `Ptr(HeapKind::Closure)`).
    /// `None` after first successful `get()` — the closure is dropped
    /// once its result is cached.
    initializer: Option<crate::kinded_slot::KindedSlot>,
    /// Cached value. `None` before first `get()`, `Some` after.
    value: Option<crate::kinded_slot::KindedSlot>,
}

impl LazyData {
    /// Build a `LazyData` wrapping `initializer` (expected to be a
    /// closure `KindedSlot`).
    pub fn new(initializer: crate::kinded_slot::KindedSlot) -> Self {
        Self {
            inner: std::sync::Mutex::new(LazyInner {
                initializer: Some(initializer),
                value: None,
            }),
        }
    }

    /// Whether `get()` has been called and the value cached.
    pub fn is_initialized(&self) -> bool {
        self.inner
            .lock()
            .expect("lazy mutex poisoned")
            .value
            .is_some()
    }

    /// Read the cached value if present, else `None`. The closure-call
    /// path (running the initializer) lives in the handler tier — this
    /// method is the storage-tier cache lookup. Returns a clone of the
    /// cached slot (one strong-count share bumped).
    pub fn cached(&self) -> Option<crate::kinded_slot::KindedSlot> {
        self.inner
            .lock()
            .expect("lazy mutex poisoned")
            .value
            .as_ref()
            .cloned()
    }

    /// Take the initializer closure (for the handler tier to invoke
    /// via `vm.call_value_immediate_nb`). Returns `None` if the value
    /// is already cached (caller should use `cached()` instead).
    pub fn take_initializer(&self) -> Option<crate::kinded_slot::KindedSlot> {
        let mut inner = self.inner.lock().expect("lazy mutex poisoned");
        if inner.value.is_some() {
            return None;
        }
        inner.initializer.take()
    }

    /// Cache the result of running the initializer. The initializer
    /// slot has already been dropped (via `take_initializer`); this
    /// installs the result. If a value was concurrently cached
    /// (impossible at single-threaded landing, but defensive for
    /// future concurrency), the new value drops cleanly via
    /// `KindedSlot::Drop`.
    pub fn store_result(&self, value: crate::kinded_slot::KindedSlot) {
        let mut inner = self.inner.lock().expect("lazy mutex poisoned");
        // The take_initializer caller is the only path that should
        // reach store_result, so value is None here at the
        // single-threaded landing.
        inner.value = Some(value);
    }
}

// ── TraitObject storage (W17-trait-object-storage, ADR-006 §2.7.24 Q25.C,
//    2026-05-11) ──────────────────────────────────────────────────────────────

/// `dyn Trait` storage — the typed-Arc replacement for the
/// bulldozer-deleted `HeapValue::TraitObject { value: Box<u64>,
/// vtable: Arc<VTable> }`. Pairs the boxed data half (always a
/// `TypedObject` per §Q25.C.4 universal-dyn ruling — scalars/strings
/// that implement traits are boxed into `TypedObject` first; the
/// auto-boxing rule lifts Rust's object-safety restrictions at the
/// cost of one heap indirection per `dyn` coerce) with the vtable
/// half (shared `Arc<VTable>` so per-impl vtables are constructed
/// once and IC-cached per §Q25.C.6).
///
/// **Forbidden alternative.** `Box<u64>` data half is explicitly
/// refused (ADR-006 §Q25.E #3 — kind-blind raw-bits storage, same
/// defection-attractor as the deleted ValueWord). The data half is
/// kinded by being a typed object with a schema — `Arc<TypedObjectStorage>`
/// recovers the per-field kind table via the schema_id.
///
/// **Identity contract.** `Arc::ptr_eq` on the vtable Arc is the
/// canonical equality for the §Q25.C.2 `Self`-arg runtime check;
/// `vtable.concrete_type_id` is the IC-stabilization key per §Q25.C.6.
///
/// Mirror of the §2.7.20 / §2.7.25 typed-Arc shape — refcount
/// discipline goes through the kind label (`HeapKind::TraitObject = 29`)
/// in `clone_with_kind` / `drop_with_kind`, NOT through `HeapValue`.
/// Method-receiver classification flows through `slot.as_heap_value()`
/// → `HeapValue::TraitObject(arc)` per ADR-005 §1 single-discriminator;
/// the `op_dyn_method_call` opcode handler (compiler-emission tier)
/// uses the recovered `Arc<TraitObjectStorage>` to look up the method
/// in `vtable.methods` and dispatch the appropriate `VTableEntry`.
#[derive(Debug)]
pub struct TraitObjectStorage {
    /// The data half of the fat pointer — owned, heap-allocated as a
    /// `TypedObject`. Always present (never null); universal-dyn
    /// per-method auto-boxing makes the boxed value a real TypedObject
    /// even for scalar concrete types (per §Q25.C.1).
    pub value: Arc<TypedObjectStorage>,

    /// The vtable half of the fat pointer. Shared via `Arc` across
    /// all `TraitObjectStorage` instances built from the same
    /// `(impl Trait for Type)` pair — vtable construction happens
    /// once per impl, the resulting `Arc<VTable>` is cached and
    /// cloned into each boxing site. IC stabilizes on
    /// `Arc::as_ptr(&vtable)` per §Q25.C.6.
    pub vtable: Arc<crate::value::VTable>,
}

impl TraitObjectStorage {
    /// Build a `TraitObjectStorage` from its two halves. The caller
    /// owns one strong-count share on each Arc; the resulting struct
    /// owns both shares. Wrap in `Arc::new(...)` immediately followed
    /// by `HeapValue::TraitObject(arc)` or
    /// `ValueSlot::from_trait_object(arc)`.
    #[inline]
    pub fn new(value: Arc<TypedObjectStorage>, vtable: Arc<crate::value::VTable>) -> Self {
        Self { value, vtable }
    }

    /// Convenience: look up a method by name in the vtable. Returns
    /// `None` for an unknown method (the dispatch tier surfaces this
    /// as a runtime error — under universal-dyn there is no compile-
    /// time `ETO-002` for "method not in trait" since the trait's
    /// declared method set is the surface checked at compile time;
    /// runtime lookup failures indicate a vtable-construction bug).
    #[inline]
    pub fn method(&self, name: &str) -> Option<&crate::value::VTableEntry> {
        self.vtable.methods.get(name)
    }

    /// Identity check for the §Q25.C.2 `Self`-arg runtime contract.
    /// `Arc::ptr_eq` on vtable Arcs is the tightest comparison; both
    /// `TraitObjectStorage` instances must share the same vtable
    /// allocation (which happens when both came from the same
    /// `(impl Trait for Type)` pair).
    #[inline]
    pub fn vtable_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.vtable, &other.vtable)
    }
}

impl Clone for TraitObjectStorage {
    /// Per-field clone — each `Arc` bumps its strong count by one.
    /// Cloning a `TraitObjectStorage` produces a fat-pointer carrier
    /// that observes the same underlying TypedObject and dispatches
    /// against the same VTable.
    fn clone(&self) -> Self {
        Self {
            value: Arc::clone(&self.value),
            vtable: Arc::clone(&self.vtable),
        }
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

// ── Range storage (W15-range, ADR-006 §2.7.23 / Q24, 2026-05-10) ────────────

/// Range value carrier — an inclusive-or-exclusive integer interval with
/// step. Built by `MakeRange` from the surface syntax `start..end` (exclusive)
/// and `start..=end` (inclusive); produced as a typed `Arc<RangeData>` slot
/// labeled `NativeKind::Ptr(HeapKind::Range)`.
///
/// **Distinct from `IteratorState`.** Range is a value with identity
/// (`r.start`, `r.end`, `r.contains(x)`, `print(r)` -> `0..10`) — an
/// `IteratorState` is a stateful pipeline with a cursor. The `.iter()`
/// receiver method on Range converts a `RangeData` into a fresh
/// `IteratorState` with `IteratorSource::Range { start, end_exclusive,
/// step }`, where `end_exclusive` is `end + step` for inclusive ranges
/// (so `0..=10` step 1 produces values 0..11). `IteratorSource::Range`
/// already models the post-conversion shape (W13-iterator-state, ADR-006
/// §2.7.16); `RangeData` is the pre-iter receiver value.
///
/// **Bounds storage.** Today only `i64` integer ranges are representable.
/// The `Option<i64>` shape used by the deleted pre-bulldozer Range payload
/// (open ranges `..end`, `start..`, `..`) is deliberately NOT modeled here:
/// the surface syntax for open ranges still compiles via `op_make_range`
/// pushing a `PushNull` for the missing side, but the SURFACE handler
/// rejects them per the playbook's surface-and-stop discipline (open
/// ranges need an iterator-tier semantic for `for i in 0..` infinite
/// loops which is its own ADR follow-up). `step` is always positive —
/// matching the pre-strict-typing `0..n` Rust-shape semantics.
#[derive(Debug, Clone)]
pub struct RangeData {
    /// Inclusive lower bound.
    pub start: i64,
    /// Upper bound. When `inclusive == true`, the value `end` itself is
    /// reachable; when `inclusive == false`, `end` is exclusive (the
    /// surface-syntax `start..end` shape).
    pub end: i64,
    /// Per-iteration increment. Always positive; defaults to 1 from the
    /// `MakeRange` opcode (the surface syntax has no step suffix today).
    pub step: i64,
    /// Whether the upper bound is reachable (`start..=end` shape).
    pub inclusive: bool,
}

impl RangeData {
    /// Construct a fresh range with the given bounds and step.
    #[inline]
    pub fn new(start: i64, end: i64, step: i64, inclusive: bool) -> Self {
        Self {
            start,
            end,
            step,
            inclusive,
        }
    }

    /// Construct an exclusive range `start..end` with step 1 (matching
    /// the surface-syntax `0..n` shape).
    #[inline]
    pub fn exclusive(start: i64, end: i64) -> Self {
        Self::new(start, end, 1, false)
    }

    /// Construct an inclusive range `start..=end` with step 1.
    #[inline]
    pub fn inclusive(start: i64, end: i64) -> Self {
        Self::new(start, end, 1, true)
    }

    /// Effective exclusive end — `end + step` for inclusive ranges,
    /// `end` for exclusive ranges. Matches the upper bound used by
    /// `IteratorSource::Range`'s `end` field (which is exclusive by
    /// W13-iterator-state's contract).
    #[inline]
    pub fn end_exclusive(&self) -> i64 {
        if self.inclusive {
            self.end.saturating_add(self.step)
        } else {
            self.end
        }
    }

    /// Element count. Mirrors `IteratorSource::Range::len` so a range
    /// and its post-`.iter()` IteratorState report the same count. For
    /// non-positive step or an empty interval, returns 0.
    #[inline]
    pub fn len(&self) -> usize {
        let end = self.end_exclusive();
        if self.step <= 0 || end <= self.start {
            return 0;
        }
        let span = (end - self.start) as u64;
        let step = self.step as u64;
        ((span + step - 1) / step) as usize
    }

    /// Whether the range yields zero elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether `value` falls within the range. The check is bound-aware
    /// (inclusive vs exclusive end) but does NOT enforce step alignment
    /// — `(0..10).contains(5)` is true regardless of step. This matches
    /// the pre-bulldozer surface-syntax shape: `range.contains` is a
    /// bound test, not a "would .iter() yield this exact value" probe.
    #[inline]
    pub fn contains(&self, value: i64) -> bool {
        if value < self.start {
            return false;
        }
        if self.inclusive {
            value <= self.end
        } else {
            value < self.end
        }
    }

    /// Materialize the range into a `Vec<i64>` of every yielded value
    /// (mirror of `.iter().collect()` for the pre-bulldozer
    /// `range.toArray()` method shape). Empty range -> empty vec.
    pub fn to_vec_i64(&self) -> Vec<i64> {
        let n = self.len();
        let mut out = Vec::with_capacity(n);
        let mut v = self.start;
        for _ in 0..n {
            out.push(v);
            v += self.step;
        }
        out
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

    /// In-place write of slot `idx` through a shared `&TypedObjectStorage`
    /// (i.e. through an `Arc<TypedObjectStorage>` with refcount > 1).
    /// Returns the prior `(bits, kind)` so the caller can run
    /// `drop_with_kind` on the released share. The caller transfers
    /// ownership of `new_bits` (one strong-count share for heap kinds) to
    /// the slot.
    ///
    /// This is the Q14 / ADR-006 §2.7.13 in-place write path for
    /// `RefTarget::TypedField` projection writes — the receiver `Arc`
    /// is shared between the ref carrier and the originating binding,
    /// so `Arc::get_mut` / `Arc::make_mut` cannot apply (refcount > 1
    /// by construction, and `TypedObjectStorage` is intentionally not
    /// `Clone` per the §2.5 documentation). The `Box<[ValueSlot]>`
    /// inside the storage is logically owned; the single-word `u64`
    /// inside each `ValueSlot` is written atomically (single-word
    /// aligned store on every supported architecture).
    ///
    /// # Safety
    ///
    /// Callers must guarantee:
    ///
    /// 1. **Single-threaded write**: the VM is single-threaded, and the
    ///    refs that drive this path are constrained by the §3.1
    ///    ref-escape analysis to stay within their originating task
    ///    (refs cannot cross task boundaries — error B0014
    ///    `NonSendableAcrossTaskBoundary`). No other thread may hold an
    ///    `&Arc<TypedObjectStorage>` to the same storage at the same
    ///    time the write executes.
    /// 2. **No aliased `&mut ValueSlot`**: callers must NOT mint a
    ///    `&mut ValueSlot` to slot `idx` from any path while this write
    ///    is in flight. The Q14 dispatch in `op_deref_store` /
    ///    `op_set_index_ref` is the only caller, and it operates on
    ///    `&TypedObjectStorage` exclusively.
    /// 3. **Kind invariance**: `new_kind` must equal
    ///    `self.field_kinds[idx]`. The Q14 RefTarget carries the
    ///    projected slot's kind at construction (`MakeFieldRef` sources
    ///    it from `field_type_tag`); the post-proof `§2.7.5.1` contract
    ///    forbids mid-life kind changes for typed fields. The caller
    ///    debug_asserts this before calling.
    /// 4. **`heap_mask` bit consistency**: for heap-kinded slots
    ///    (NativeKind::String or Ptr(HeapKind::_)), the corresponding
    ///    `heap_mask` bit must already be set per the `TypedObjectStorage::new`
    ///    construction-side contract, AND the prior bits must be a
    ///    valid `Arc::into_raw::<T>` for the slot's kind. The returned
    ///    `prior_bits` is exactly that share; the caller releases it via
    ///    `drop_with_kind` after running the post-write barrier.
    ///
    /// Q14 / ADR-006 §2.7.13. Mirror of the `clone_with_kind` /
    /// `drop_with_kind` symmetry used by `RefTarget::Local` and
    /// `RefTarget::ModuleBinding` writes (`stack_write_kinded` and
    /// `module_binding_write_kinded` already encapsulate this pattern
    /// for non-projected places; this is the projected-place mirror).
    #[inline]
    pub unsafe fn write_slot_in_place(
        &self,
        idx: usize,
        new_bits: u64,
    ) -> u64 {
        debug_assert!(
            idx < self.slots.len(),
            "TypedObjectStorage::write_slot_in_place: idx {} out of bounds (slots.len = {})",
            idx,
            self.slots.len(),
        );
        // SAFETY: see method contract. Single-threaded VM; refs cannot
        // escape across task boundaries; no aliased `&mut ValueSlot`
        // outstanding by construction; `Box<[ValueSlot]>` is `Sized`-laid-
        // out and the slot's `u64` is naturally aligned. We cast through
        // `&[ValueSlot]` -> `*const ValueSlot` -> `*mut ValueSlot` to
        // perform the single-word write. The slot's `field_kinds[idx]`
        // is the kind invariant; the caller already debug_asserted kind
        // equality, so the slot's heap-mask bit (if set) still applies
        // to the new bits.
        let slot_ptr = self.slots.as_ptr().add(idx) as *mut crate::slot::ValueSlot;
        let prior = (*slot_ptr).raw();
        *slot_ptr = crate::slot::ValueSlot::from_raw(new_bits);
        prior
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
                        // W17-concurrency (ADR-006 §2.7.25, 2026-05-11):
                        // Mutex / Atomic / Lazy mirror the Channel arm.
                        // A TypedObject field of kind
                        // `NativeKind::Ptr(HeapKind::Mutex/Atomic/Lazy)`
                        // holds slot bits =
                        // `Arc::into_raw(Arc<MutexData/AtomicData/LazyData>)`.
                        // Retire one strong-count share at storage drop.
                        HeapKind::Mutex => {
                            std::sync::Arc::decrement_strong_count(bits as *const MutexData);
                        }
                        HeapKind::Atomic => {
                            std::sync::Arc::decrement_strong_count(bits as *const AtomicData);
                        }
                        HeapKind::Lazy => {
                            std::sync::Arc::decrement_strong_count(bits as *const LazyData);
                        }
                        // W17-trait-object-storage (ADR-006 §2.7.24 / Q25.C,
                        // 2026-05-11): a TypedObject field of kind
                        // `NativeKind::Ptr(HeapKind::TraitObject)` holds
                        // slot bits = `Arc::into_raw(Arc<TraitObjectStorage>)`.
                        // Retire one strong-count share at storage drop;
                        // the auto-derived `TraitObjectStorage::Drop`
                        // then releases the inner value + vtable Arcs.
                        // Same dispatch shape as the Channel / concurrency
                        // primitives — `dyn` carriers are first-class
                        // typed-Arc payloads.
                        HeapKind::TraitObject => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const TraitObjectStorage,
                            );
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
                        // W15-range (ADR-006 §2.7.23 / Q24, 2026-05-10):
                        // a TypedObject field of kind
                        // `NativeKind::Ptr(HeapKind::Range)` holds slot
                        // bits = `Arc::into_raw(Arc<RangeData>)` directly
                        // (typed-Arc shape, mirror of HashMap / HashSet
                        // / Iterator). At storage drop, retire one
                        // `Arc<RangeData>` strong-count share.
                        HeapKind::Range => {
                            std::sync::Arc::decrement_strong_count(bits as *const RangeData);
                        }
                        // Wave 14 W14-variant-codegen (ADR-006 §2.7.17
                        // / Q18, 2026-05-10): a TypedObject field of
                        // kind `NativeKind::Ptr(HeapKind::Result)` /
                        // `NativeKind::Ptr(HeapKind::Option)` holds
                        // `Arc::into_raw(Arc<ResultData>) as u64` /
                        // `Arc::into_raw(Arc<OptionData>) as u64`. Same
                        // dispatch shape as the Iterator arm above.
                        HeapKind::Result => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const ResultData,
                            );
                        }
                        HeapKind::Option => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const OptionData,
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
                        HeapKind::Future => {
                            // No-op: future-id inline scalar.
                        }
                        // W17-comptime-vm-dispatch (ADR-006 §2.7.26,
                        // 2026-05-12): `Ptr(HeapKind::ModuleFn)` carries
                        // the module-fn-id u64 directly in `bits`
                        // (inline scalar — no `Arc<T>` payload). Used
                        // by `populate_module_objects` for typed-
                        // object module-binding field slots. Same
                        // dispatch shape as `HeapKind::Future` —
                        // no refcount work, but the kind label is
                        // `Ptr(HeapKind::ModuleFn)` so the dispatch
                        // shell can route the slot's bits to
                        // `invoke_module_fn_id_stub` at CallValue time.
                        HeapKind::ModuleFn => {
                            // No-op: module-fn-id inline scalar.
                        }
                        // ADR-006 §2.7.22 amendment (Round 18 S3,
                        // 2026-05-13): a TypedObject field of kind
                        // `NativeKind::Ptr(HeapKind::Matrix)` /
                        // `NativeKind::Ptr(HeapKind::MatrixSlice)` holds
                        // slot bits = `Arc::into_raw(Arc<MatrixData>)` /
                        // `Arc::into_raw(Arc<MatrixSliceData>)` directly.
                        // Retire one strong-count share at storage drop.
                        // Same typed-Arc pure-discriminator dispatch shape
                        // as the §2.7.9 FilterExpr / §2.7.13 Reference
                        // amendments — `as_heap_value()` is unsound on
                        // these bits; the kind label IS the dispatch.
                        HeapKind::Matrix => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const MatrixData,
                            );
                        }
                        HeapKind::MatrixSlice => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const MatrixSliceData,
                            );
                        }
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
    // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13): the legacy
    // `Matrix(Arc<MatrixData>)` variant is DELETED. Matrix is a category-error
    // here — it is a single Matrix value, not a buffer-of-Matrix. Matrix now
    // lives at `HeapKind::Matrix = 34` + `HeapValue::Matrix(Arc<MatrixData>)`
    // with the typed-Arc pure-discriminator dispatch shape (mirror of
    // §2.7.9 FilterExpr). See §2.7.22 amendment text + S3 close commit.
    I8(Arc<crate::typed_buffer::TypedBuffer<i8>>),
    I16(Arc<crate::typed_buffer::TypedBuffer<i16>>),
    I32(Arc<crate::typed_buffer::TypedBuffer<i32>>),
    U8(Arc<crate::typed_buffer::TypedBuffer<u8>>),
    U16(Arc<crate::typed_buffer::TypedBuffer<u16>>),
    U32(Arc<crate::typed_buffer::TypedBuffer<u32>>),
    U64(Arc<crate::typed_buffer::TypedBuffer<u64>>),
    F32(Arc<crate::typed_buffer::TypedBuffer<f32>>),
    String(Arc<crate::typed_buffer::TypedBuffer<Arc<String>>>),
    // ── ADR-006 §2.7.24 Q25.A monomorphic specializations ─────────────────
    // W17-typed-carrier-bundle-A commit 1/4 (2026-05-11): added alongside
    // the legacy HeapValue arm. Construction sites migrate in commit 2;
    // read sites in commit 3; commit 4 deletes the HeapValue arm. Per-element
    // kind is uniform per variant — variant tag IS the kind, no parallel
    // kind track.
    Decimal(Arc<crate::typed_buffer::TypedBuffer<Arc<rust_decimal::Decimal>>>),
    BigInt(Arc<crate::typed_buffer::TypedBuffer<Arc<i64>>>),
    DateTime(Arc<crate::typed_buffer::TypedBuffer<Arc<TemporalData>>>),
    Timespan(Arc<crate::typed_buffer::TypedBuffer<Arc<TemporalData>>>),
    Duration(Arc<crate::typed_buffer::TypedBuffer<Arc<TemporalData>>>),
    Instant(Arc<crate::typed_buffer::TypedBuffer<Arc<std::time::Instant>>>),
    Char(Arc<crate::typed_buffer::TypedBuffer<char>>),
    TypedObject(Arc<crate::typed_buffer::TypedBuffer<Arc<TypedObjectStorage>>>),
    /// Trait-object element array. Carrier shape mirrors §Q25.C's
    /// `Arc<TraitObjectStorage>`. **No construction site on this branch** —
    /// `HeapKind::TraitObject = 29` and `TraitObjectStorage`'s real
    /// fat-pointer shape come from W17-trait-object-storage (parallel
    /// dispatch). The variant exists here for type-shape completeness.
    TraitObject(Arc<crate::typed_buffer::TypedBuffer<Arc<TraitObjectStorage>>>),
    // The polymorphic `HeapValue(Arc<TypedBuffer<Arc<HeapValue>>>)` catch-all
    // was DELETED in checkpoint 4 of W17-typed-carrier-bundle-A per ADR-006
    // §2.7.24 Q25.A. Every construction site migrated to a specialized
    // variant in checkpoint 2; every reader was filled with real per-arm
    // bodies in checkpoint 3. Do not reintroduce under any rename — see
    // §Q25.E #1 forbidden pattern list.
    // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13): the legacy
    // `FloatSlice { parent, offset, len }` variant is DELETED. It is a
    // projection-into-a-Matrix (category-error in a TypedArrayData carrier).
    // The projection lives at `HeapKind::MatrixSlice = 35` +
    // `HeapValue::MatrixSlice(Arc<MatrixSliceData>)` with the typed-Arc
    // pure-discriminator dispatch shape (mirror of §2.7.9 FilterExpr).
    // See §2.7.22 amendment text + S3 close commit.
}

impl TypedArrayData {
    /// W17-typed-carrier-bundle-A checkpoint 2/4: build a strict-typed
    /// `TypedArrayData` variant from a `Vec<Arc<HeapValue>>` of uniform-arm
    /// elements per ADR-006 §2.7.24 Q25.A. Dispatch on the first element's
    /// `HeapValue` arm and require all subsequent elements to match. The
    /// resulting variant carries the element kind at the variant level —
    /// no parallel kind track, no polymorphic catch-all. Heterogeneous-arm
    /// inputs surface a structured error.
    ///
    /// Returns the variant directly (no Arc wrapper). Callers wrap via
    /// `Arc::new(...)` then `KindedSlot::from_typed_array(...)` /
    /// `HeapValue::TypedArray(...)` per their carrier needs.
    pub fn build_specialized_from_heap_arcs(
        elems: Vec<Arc<HeapValue>>,
    ) -> Result<Self, String> {
        use crate::typed_buffer::TypedBuffer;
        if elems.is_empty() {
            let buf: TypedBuffer<Arc<TypedObjectStorage>> = TypedBuffer::from_vec(Vec::new());
            return Ok(TypedArrayData::TypedObject(Arc::new(buf)));
        }
        let first = &elems[0];
        match first.as_ref() {
            HeapValue::String(_) => {
                let mut data: Vec<Arc<String>> = Vec::with_capacity(elems.len());
                for e in elems.iter() {
                    match e.as_ref() {
                        HeapValue::String(s) => data.push(Arc::clone(s)),
                        other => return Err(format!(
                            "TypedArrayData::build_specialized: heterogeneous \
                             heap arms (expected String, got {:?})",
                            other.kind()
                        )),
                    }
                }
                Ok(TypedArrayData::String(Arc::new(TypedBuffer::from_vec(data))))
            }
            HeapValue::Decimal(_) => {
                let mut data: Vec<Arc<rust_decimal::Decimal>> = Vec::with_capacity(elems.len());
                for e in elems.iter() {
                    match e.as_ref() {
                        HeapValue::Decimal(d) => data.push(Arc::clone(d)),
                        other => return Err(format!(
                            "TypedArrayData::build_specialized: heterogeneous \
                             heap arms (expected Decimal, got {:?})",
                            other.kind()
                        )),
                    }
                }
                Ok(TypedArrayData::Decimal(Arc::new(TypedBuffer::from_vec(data))))
            }
            HeapValue::BigInt(_) => {
                let mut data: Vec<Arc<i64>> = Vec::with_capacity(elems.len());
                for e in elems.iter() {
                    match e.as_ref() {
                        HeapValue::BigInt(b) => data.push(Arc::clone(b)),
                        other => return Err(format!(
                            "TypedArrayData::build_specialized: heterogeneous \
                             heap arms (expected BigInt, got {:?})",
                            other.kind()
                        )),
                    }
                }
                Ok(TypedArrayData::BigInt(Arc::new(TypedBuffer::from_vec(data))))
            }
            HeapValue::TypedObject(_) => {
                let mut data: Vec<Arc<TypedObjectStorage>> = Vec::with_capacity(elems.len());
                for e in elems.iter() {
                    match e.as_ref() {
                        HeapValue::TypedObject(s) => data.push(Arc::clone(s)),
                        other => return Err(format!(
                            "TypedArrayData::build_specialized: heterogeneous \
                             heap arms (expected TypedObject, got {:?})",
                            other.kind()
                        )),
                    }
                }
                Ok(TypedArrayData::TypedObject(Arc::new(TypedBuffer::from_vec(data))))
            }
            HeapValue::Char(_) => {
                let mut data: Vec<char> = Vec::with_capacity(elems.len());
                for e in elems.iter() {
                    match e.as_ref() {
                        HeapValue::Char(c) => data.push(*c),
                        other => return Err(format!(
                            "TypedArrayData::build_specialized: heterogeneous \
                             heap arms (expected Char, got {:?})",
                            other.kind()
                        )),
                    }
                }
                Ok(TypedArrayData::Char(Arc::new(TypedBuffer::from_vec(data))))
            }
            other => Err(format!(
                "TypedArrayData::build_specialized: HeapValue arm {:?} not yet \
                 supported post-§2.7.24 Q25.A — add a specialized TypedArrayData arm.",
                other.kind()
            )),
        }
    }

    #[inline]
    pub fn type_name(&self) -> &'static str {
        match self {
            TypedArrayData::I64(_) => "Vec<int>",
            TypedArrayData::F64(_) => "Vec<number>",
            TypedArrayData::Bool(_) => "Vec<bool>",
            TypedArrayData::I8(_) => "Vec<i8>",
            TypedArrayData::I16(_) => "Vec<i16>",
            TypedArrayData::I32(_) => "Vec<i32>",
            TypedArrayData::U8(_) => "Vec<u8>",
            TypedArrayData::U16(_) => "Vec<u16>",
            TypedArrayData::U32(_) => "Vec<u32>",
            TypedArrayData::U64(_) => "Vec<u64>",
            TypedArrayData::F32(_) => "Vec<f32>",
            TypedArrayData::String(_) => "Vec<string>",
            TypedArrayData::Decimal(_) => "Vec<decimal>",
            TypedArrayData::BigInt(_) => "Vec<int>",
            TypedArrayData::DateTime(_) => "Vec<datetime>",
            TypedArrayData::Timespan(_) => "Vec<timespan>",
            TypedArrayData::Duration(_) => "Vec<duration>",
            TypedArrayData::Instant(_) => "Vec<instant>",
            TypedArrayData::Char(_) => "Vec<char>",
            TypedArrayData::TypedObject(_) => "Vec<object>",
            TypedArrayData::TraitObject(_) => "Vec<dyn>",
        }
    }

    #[inline]
    pub fn is_truthy(&self) -> bool {
        match self {
            TypedArrayData::I64(a) => !a.is_empty(),
            TypedArrayData::F64(a) => !a.is_empty(),
            TypedArrayData::Bool(a) => !a.is_empty(),
            TypedArrayData::I8(a) => !a.is_empty(),
            TypedArrayData::I16(a) => !a.is_empty(),
            TypedArrayData::I32(a) => !a.is_empty(),
            TypedArrayData::U8(a) => !a.is_empty(),
            TypedArrayData::U16(a) => !a.is_empty(),
            TypedArrayData::U32(a) => !a.is_empty(),
            TypedArrayData::U64(a) => !a.is_empty(),
            TypedArrayData::F32(a) => !a.is_empty(),
            TypedArrayData::String(a) => !a.is_empty(),
            TypedArrayData::Decimal(a) => !a.is_empty(),
            TypedArrayData::BigInt(a) => !a.is_empty(),
            TypedArrayData::DateTime(a) => !a.is_empty(),
            TypedArrayData::Timespan(a) => !a.is_empty(),
            TypedArrayData::Duration(a) => !a.is_empty(),
            TypedArrayData::Instant(a) => !a.is_empty(),
            TypedArrayData::Char(a) => !a.is_empty(),
            TypedArrayData::TypedObject(a) => !a.is_empty(),
            TypedArrayData::TraitObject(a) => !a.is_empty(),
        }
    }

    /// In-place write of element `idx` through a shared
    /// `&TypedArrayData` (i.e. through an `Arc<TypedArrayData>` with
    /// refcount > 1, or via the inner `Arc<TypedBuffer<T>>` likewise
    /// shared). Returns the prior bits for the heap-pointer arms
    /// (currently `String`) so the caller can run `drop_with_kind` on
    /// the released share. For scalar arms, the returned bits are the
    /// prior scalar's u64 representation (no Arc-share release needed).
    ///
    /// This is the Q14 / ADR-006 §2.7.13 in-place write path for
    /// `RefTarget::TypedIndex` projection writes — same constraints as
    /// `TypedObjectStorage::write_slot_in_place`. The receiver Arc is
    /// shared between the ref carrier and the originating binding,
    /// so `Arc::make_mut` would clone the buffer and break ref
    /// semantics.
    ///
    /// `new_bits` is the kind-encoded element value: integer arms use
    /// `new_bits as <T>` (two's complement / zero-extension); float
    /// arms use `f64::from_bits(new_bits)` (matching the
    /// `typed_array_read_index_raw` symmetry); the `String` arm
    /// interprets `new_bits` as `Arc::into_raw::<String>` (one share
    /// transferred to the buffer).
    ///
    /// Variants without a single statically-sourceable scalar element
    /// kind (`HeapValue`, `FloatSlice`, `Matrix`) return `None` —
    /// `typed_array_element_kind` already rejects construction of a
    /// `TypedIndex` projection over those variants, so this path is
    /// unreachable for them. Returning `None` rather than panicking
    /// preserves a defensive surface for any future projection-builder
    /// gap.
    ///
    /// # Safety
    ///
    /// Same contract as `TypedObjectStorage::write_slot_in_place`:
    ///
    /// 1. Single-threaded write (VM is single-threaded; refs constrained
    ///    by §3.1 escape analysis to stay within their originating
    ///    task).
    /// 2. No aliased `&mut TypedBuffer<T>` outstanding for the target
    ///    buffer.
    /// 3. `new_bits` interprets correctly under the variant's element
    ///    kind. The caller (`write_ref_target` in `variables/mod.rs`)
    ///    debug_asserts kind equality between popped `val_kind` and
    ///    the `RefTarget::TypedIndex { elem_kind, .. }` projection.
    /// 4. `idx` is within bounds (`idx < self.element_count()`); the
    ///    caller bounds-checks at `MakeIndexRef` / `SetIndexRef`
    ///    construction time.
    ///
    /// Q14 / ADR-006 §2.7.13.
    pub unsafe fn write_index_in_place(
        &self,
        idx: usize,
        new_bits: u64,
    ) -> Option<u64> {
        // SAFETY (each arm): the buffer `Arc<TypedBuffer<T>>` (or
        // `Arc<AlignedTypedBuffer>` for F64) is shared via the typed
        // `Arc` payload. The `TypedBuffer<T>` itself owns a `Vec<T>`,
        // which is `Sized`-laid-out with contiguous slots. We cast
        // through `&TypedBuffer<T>` -> `*const TypedBuffer<T>` ->
        // `*mut TypedBuffer<T>` to access the inner Vec's slot at
        // `idx`. The element's `T` is naturally aligned per the
        // platform's Vec invariants, and the write is single-word
        // (or smaller — i8/u8 are still atomic at machine level via
        // a sub-word store) given the caller's single-threaded
        // contract.
        let prior = match self {
            TypedArrayData::I64(buf) => {
                let buf_ptr = std::sync::Arc::as_ptr(buf) as *mut crate::typed_buffer::TypedBuffer<i64>;
                let slot = (*buf_ptr).data.as_mut_ptr().add(idx);
                let p = *slot as u64;
                *slot = new_bits as i64;
                p
            }
            TypedArrayData::F64(buf) => {
                let buf_ptr = std::sync::Arc::as_ptr(buf) as *mut crate::typed_buffer::AlignedTypedBuffer;
                let slot = (*buf_ptr).data.as_mut_ptr().add(idx);
                let p = (*slot).to_bits();
                *slot = f64::from_bits(new_bits);
                p
            }
            TypedArrayData::Bool(buf) => {
                let buf_ptr = std::sync::Arc::as_ptr(buf) as *mut crate::typed_buffer::TypedBuffer<u8>;
                let slot = (*buf_ptr).data.as_mut_ptr().add(idx);
                let p = *slot as u64;
                *slot = if new_bits != 0 { 1u8 } else { 0u8 };
                p
            }
            TypedArrayData::I8(buf) => {
                let buf_ptr = std::sync::Arc::as_ptr(buf) as *mut crate::typed_buffer::TypedBuffer<i8>;
                let slot = (*buf_ptr).data.as_mut_ptr().add(idx);
                let p = *slot as i64 as u64;
                *slot = new_bits as i8;
                p
            }
            TypedArrayData::I16(buf) => {
                let buf_ptr = std::sync::Arc::as_ptr(buf) as *mut crate::typed_buffer::TypedBuffer<i16>;
                let slot = (*buf_ptr).data.as_mut_ptr().add(idx);
                let p = *slot as i64 as u64;
                *slot = new_bits as i16;
                p
            }
            TypedArrayData::I32(buf) => {
                let buf_ptr = std::sync::Arc::as_ptr(buf) as *mut crate::typed_buffer::TypedBuffer<i32>;
                let slot = (*buf_ptr).data.as_mut_ptr().add(idx);
                let p = *slot as i64 as u64;
                *slot = new_bits as i32;
                p
            }
            TypedArrayData::U8(buf) => {
                let buf_ptr = std::sync::Arc::as_ptr(buf) as *mut crate::typed_buffer::TypedBuffer<u8>;
                let slot = (*buf_ptr).data.as_mut_ptr().add(idx);
                let p = *slot as u64;
                *slot = new_bits as u8;
                p
            }
            TypedArrayData::U16(buf) => {
                let buf_ptr = std::sync::Arc::as_ptr(buf) as *mut crate::typed_buffer::TypedBuffer<u16>;
                let slot = (*buf_ptr).data.as_mut_ptr().add(idx);
                let p = *slot as u64;
                *slot = new_bits as u16;
                p
            }
            TypedArrayData::U32(buf) => {
                let buf_ptr = std::sync::Arc::as_ptr(buf) as *mut crate::typed_buffer::TypedBuffer<u32>;
                let slot = (*buf_ptr).data.as_mut_ptr().add(idx);
                let p = *slot as u64;
                *slot = new_bits as u32;
                p
            }
            TypedArrayData::U64(buf) => {
                let buf_ptr = std::sync::Arc::as_ptr(buf) as *mut crate::typed_buffer::TypedBuffer<u64>;
                let slot = (*buf_ptr).data.as_mut_ptr().add(idx);
                let p = *slot;
                *slot = new_bits;
                p
            }
            TypedArrayData::F32(buf) => {
                let buf_ptr = std::sync::Arc::as_ptr(buf) as *mut crate::typed_buffer::TypedBuffer<f32>;
                let slot = (*buf_ptr).data.as_mut_ptr().add(idx);
                let p = (*slot as f64).to_bits();
                *slot = f64::from_bits(new_bits) as f32;
                p
            }
            TypedArrayData::String(buf) => {
                // Element type is `Arc<String>`. The slot stores the
                // current share; `new_bits` is `Arc::into_raw::<String>`
                // of the incoming share. Reconstruct the prior `Arc<String>`
                // to surface its `Arc::into_raw` pointer for the caller
                // to drop, then place the new `Arc<String>` constructed
                // from `new_bits`.
                let buf_ptr = std::sync::Arc::as_ptr(buf) as *mut crate::typed_buffer::TypedBuffer<Arc<String>>;
                let slot = (*buf_ptr).data.as_mut_ptr().add(idx);
                // Move-out the prior Arc<String> without dropping (the
                // caller releases via `drop_with_kind(prior_bits, String)`),
                // and move-in the new one from `new_bits`.
                let prior_arc: Arc<String> = std::ptr::read(slot);
                let prior = Arc::into_raw(prior_arc) as u64;
                let new_arc: Arc<String> = Arc::from_raw(new_bits as *const String);
                std::ptr::write(slot, new_arc);
                prior
            }
            // Variants without a single statically-sourceable scalar
            // element kind. Construction-side `MakeIndexRef` /
            // `SetIndexRef` already rejects these via
            // `typed_array_element_kind`'s `None` return; reaching this
            // arm is a construction-side bug, not a soundness gap.
            //
            // W17-typed-carrier-bundle-A commit 1/4: the new
            // §2.7.24 Q25.A specialized arms (Decimal / BigInt /
            // DateTime / Timespan / Duration / Instant / Char /
            // TypedObject / TraitObject) are also rejected here — they
            // are heap-typed-Arc-element buffers whose write paths go
            // through dedicated per-arm typed entry-points (commit 2-3
            // wiring; commit 4 deletes the HeapValue arm).
            TypedArrayData::Decimal(_)
            | TypedArrayData::BigInt(_)
            | TypedArrayData::DateTime(_)
            | TypedArrayData::Timespan(_)
            | TypedArrayData::Duration(_)
            | TypedArrayData::Instant(_)
            | TypedArrayData::Char(_)
            | TypedArrayData::TypedObject(_)
            | TypedArrayData::TraitObject(_) => return None,
        };
        Some(prior)
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
            // ── ADR-006 §2.7.24 Q25.A specialized arms ──────────────────
            // W17-typed-carrier-bundle-A commit 1/4: Display impls per
            // arm matching the per-variant `HeapValue::*` Display shape.
            TypedArrayData::Decimal(a) => {
                write!(f, "Vec<decimal>[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            TypedArrayData::BigInt(a) => {
                write!(f, "Vec<int>[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", **v)?;
                }
                write!(f, "]")
            }
            TypedArrayData::DateTime(a)
            | TypedArrayData::Timespan(a)
            | TypedArrayData::Duration(a) => {
                let label = match self {
                    TypedArrayData::DateTime(_) => "Vec<datetime>",
                    TypedArrayData::Timespan(_) => "Vec<timespan>",
                    TypedArrayData::Duration(_) => "Vec<duration>",
                    _ => unreachable!(),
                };
                write!(f, "{}[", label)?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            TypedArrayData::Instant(a) => {
                write!(f, "Vec<instant>[")?;
                for (i, _v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "<instant>")?;
                }
                write!(f, "]")
            }
            TypedArrayData::Char(a) => {
                write!(f, "Vec<char>[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "'{}'", v)?;
                }
                write!(f, "]")
            }
            TypedArrayData::TypedObject(a) => {
                write!(f, "Vec<object>[")?;
                for (i, _v) in a.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{{...}}")?;
                }
                write!(f, "]")
            }
            TypedArrayData::TraitObject(a) => {
                // No construction site on this branch; this arm is
                // unreachable until W17-trait-object-storage lands.
                write!(f, "Vec<dyn>[<{} elements>]", a.data.len())
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
            // W15-range (ADR-006 §2.7.23 / Q24, 2026-05-10): Range Arcs
            // share the typed-Arc clone shape — single strong-count bump
            // on the shared `Arc<RangeData>`, no payload copy. RangeData
            // is small ({i64, i64, i64, bool}) so copies would be cheap,
            // but the shared-Arc shape matches the dispatch pattern of
            // every other heap arm.
            HeapValue::Range(v) => HeapValue::Range(Arc::clone(v)),
            // Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18,
            // 2026-05-10): Result/Option Arcs share the typed-Arc
            // clone shape — single strong-count bump on the shared
            // `Arc<ResultData>` / `Arc<OptionData>`. The inner
            // `KindedSlot` payload's share is preserved by the
            // shared Arc; ResultData/OptionData Clone (defined in
            // this file) does an inner KindedSlot Clone if the Arc
            // is unwrapped via Arc::make_mut.
            HeapValue::Result(v) => HeapValue::Result(Arc::clone(v)),
            HeapValue::Option(v) => HeapValue::Option(Arc::clone(v)),
            // W17-trait-object-storage (ADR-006 §2.7.24 / Q25.C,
            // 2026-05-11): TraitObject Arcs share the typed-Arc clone
            // shape — single strong-count bump on the shared
            // `Arc<TraitObjectStorage>`. Inner Arcs (`value: Arc<TypedObjectStorage>`
            // + `vtable: Arc<VTable>`) stay shared with the source;
            // `Arc::ptr_eq` on the vtable preserves the §Q25.C.2
            // `Self`-arg runtime identity contract across the clone.
            HeapValue::TraitObject(v) => HeapValue::TraitObject(Arc::clone(v)),
            // W17-concurrency (ADR-006 §2.7.25, 2026-05-11): Mutex /
            // Atomic / Lazy Arcs share the typed-Arc clone shape —
            // single strong-count bump on the shared inner Arc, no
            // payload copy. Cloning a Mutex/Lazy yields a fresh
            // "endpoint" share of the same protected cell; Atomic
            // shares observe each other's load/store/fetch
            // operations. Same shape as Channel.
            HeapValue::Mutex(v) => HeapValue::Mutex(Arc::clone(v)),
            HeapValue::Atomic(v) => HeapValue::Atomic(Arc::clone(v)),
            HeapValue::Lazy(v) => HeapValue::Lazy(Arc::clone(v)),
            // W17-comptime-vm-dispatch (ADR-006 §2.7.26, 2026-05-12):
            // ModuleFn is an inline-scalar payload (no Arc).
            HeapValue::ModuleFn(v) => HeapValue::ModuleFn(*v),
            // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13): Matrix
            // and MatrixSlice arms share the typed-Arc clone shape — single
            // strong-count bump on the shared `Arc<MatrixData>` /
            // `Arc<MatrixSliceData>`. MatrixSlice's inner `parent: Arc<MatrixData>`
            // share stays shared with the source (cloning the slice does not
            // copy the parent matrix data). The HeapValue arm exists for the
            // ADR-005 §1 / ADR-006 §2.3 `HeapKind`↔`HeapValue` symmetry but
            // calling `slot.as_heap_value()` on Matrix/MatrixSlice-labeled
            // slot bits is unsound — slot bits are
            // `Arc::into_raw(Arc<MatrixData>)` / `Arc::into_raw(Arc<MatrixSliceData>)`,
            // NOT `Box<HeapValue>`. Pure-discriminator dispatch shape, mirror
            // of §2.7.9 FilterExpr / §2.7.13 Reference.
            HeapValue::Matrix(v) => HeapValue::Matrix(Arc::clone(v)),
            HeapValue::MatrixSlice(v) => HeapValue::MatrixSlice(Arc::clone(v)),
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
                let n = d.keys.data.len();
                for i in 0..n {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    let k = &d.keys.data[i];
                    let v = d.values.value_at(i);
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
            // W15-range (ADR-006 §2.7.23 / Q24, 2026-05-10): user-visible
            // literal form — `start..end` for exclusive, `start..=end`
            // for inclusive. Step is not part of the surface syntax (no
            // explicit step suffix) so it's not rendered. This matches
            // the pre-strict-typing print format and the `0..10` /
            // `0..=10` source syntax round-trip.
            HeapValue::Range(r) => {
                if r.inclusive {
                    write!(f, "{}..={}", r.start, r.end)
                } else {
                    write!(f, "{}..{}", r.start, r.end)
                }
            }
            // Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18,
            // 2026-05-10): render as `Ok(<inner>)` / `Err(<inner>)` /
            // `Some(<inner>)` / `None`. The inner Display goes
            // through the runtime kinded value-formatter at the
            // VM-tier `printing.rs` site (the heap_value `Display`
            // impl is a fallback for diagnostic prints; rich
            // formatting goes through the executor's
            // `format_kinded`). Renders inner as `<…>` opaque tag
            // here — the kinded formatter handles full pretty-print.
            HeapValue::Result(r) => {
                if r.is_ok {
                    write!(f, "Ok(<...>)")
                } else {
                    write!(f, "Err(<...>)")
                }
            }
            HeapValue::Option(o) => {
                if o.is_some {
                    write!(f, "Some(<...>)")
                } else {
                    write!(f, "None")
                }
            }
            // W17-trait-object-storage (ADR-006 §2.7.24 / Q25.C,
            // 2026-05-11): `dyn Trait` carriers render as an opaque
            // tag annotated with the first trait name (multi-trait
            // inheritance shows just the first) and the inner schema
            // id of the boxed TypedObject. Pretty-printing via the
            // boxed receiver's user-defined `Display`-equivalent is
            // a compiler-emission tier concern (call the trait's
            // `.format(self)` method through the vtable); the
            // storage-tier formatter is diagnostic-only.
            HeapValue::TraitObject(t) => {
                let trait_name = t
                    .vtable
                    .trait_names
                    .first()
                    .map(|s| s.as_str())
                    .unwrap_or("?");
                write!(
                    f,
                    "<dyn {} #{}>",
                    trait_name, t.value.schema_id
                )
            }
            // W17-concurrency (ADR-006 §2.7.25, 2026-05-11): concurrency
            // primitives have no user-facing literal — render as opaque
            // tags annotated with diagnostic state. Mirror of Channel's
            // `<channel:state:len>` shape.
            HeapValue::Mutex(_) => write!(f, "<mutex>"),
            HeapValue::Atomic(a) => write!(f, "<atomic:{}>", a.load()),
            HeapValue::Lazy(l) => {
                if l.is_initialized() {
                    write!(f, "<lazy:initialized>")
                } else {
                    write!(f, "<lazy:pending>")
                }
            }
            // W17-comptime-vm-dispatch (ADR-006 §2.7.26, 2026-05-12):
            // ModuleFn references render as `<module_fn:id>`.
            HeapValue::ModuleFn(id) => write!(f, "<module_fn:{}>", id),
            // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13):
            // Matrix renders as `<Mat<number>:rows x cols>`, mirroring the
            // pre-amendment `TypedArrayData::Matrix` Display shape.
            // MatrixSlice renders as a flat `Vec<number>[...]` over the
            // projection's element slice, mirroring the pre-amendment
            // `TypedArrayData::FloatSlice` Display shape. These Display
            // surfaces are diagnostic fallbacks; pretty-printing of
            // Matrix/MatrixSlice values goes through `printing.rs` at
            // the VM tier.
            HeapValue::Matrix(m) => {
                write!(f, "<Mat<number>:{}x{}>", m.rows, m.cols)
            }
            HeapValue::MatrixSlice(s) => {
                let slice = s.as_slice();
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
            // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13): Matrix
            // equality is structural (rows + cols + element-wise compare);
            // MatrixSlice equality is element-wise over the projection slice
            // (parent identity is NOT required — two slices with identical
            // elements compare equal even when projecting from different
            // parents). Mirror of the pre-amendment TypedArrayData::Matrix /
            // FloatSlice equality semantics.
            (HeapValue::Matrix(a), HeapValue::Matrix(b)) => matrix_eq(a, b),
            (HeapValue::MatrixSlice(a), HeapValue::MatrixSlice(b)) => {
                a.as_slice() == b.as_slice()
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
            // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13): Matrix
            // and MatrixSlice equality match the structural_eq shape above.
            (HeapValue::Matrix(a), HeapValue::Matrix(b)) => matrix_eq(a, b),
            (HeapValue::MatrixSlice(a), HeapValue::MatrixSlice(b)) => {
                a.as_slice() == b.as_slice()
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
    }
}

// ── W14-variant-codegen unit tests (ADR-006 §2.7.17 / Q18) ──────────────────

#[cfg(test)]
mod result_option_storage {
    //! W14-variant-codegen (ADR-006 §2.7.17 / Q18, 2026-05-10): pin the
    //! `ResultData::ok` / `err` and `OptionData::some` / `none` API
    //! contracts.
    use super::*;
    use crate::kinded_slot::KindedSlot;
    use std::sync::Arc;

    #[test]
    fn ok_carrier_is_ok_true() {
        let payload = KindedSlot::from_int(42);
        let r = ResultData::ok(payload);
        assert!(r.is_ok);
        // Payload kind preserved.
        assert_eq!(r.payload.as_i64(), Some(42));
    }

    #[test]
    fn err_carrier_is_ok_false() {
        let payload = KindedSlot::from_string_arc(Arc::new("oops".to_string()));
        let r = ResultData::err(payload);
        assert!(!r.is_ok);
        // Payload string preserved.
        assert_eq!(r.payload.as_str(), Some("oops"));
    }

    #[test]
    fn result_clone_bumps_payload_share() {
        // The storage carrier's Clone path goes through KindedSlot's
        // explicit Clone impl, which dispatches retain on the payload's
        // kind. Verify that cloning the wrapper preserves the payload's
        // Arc identity (the inner Arc<String> share is bumped, not
        // duplicated).
        let payload_arc = Arc::new("hello".to_string());
        let payload = KindedSlot::from_string_arc(Arc::clone(&payload_arc));
        let r1 = ResultData::ok(payload);
        let r2 = r1.clone();
        // Pointer equality on the inner String: both wrappers
        // reference the same Arc<String> (the kinded clone bumped
        // the share, did not deep-copy the string body).
        assert_eq!(r1.payload.as_str(), Some("hello"));
        assert_eq!(r2.payload.as_str(), Some("hello"));
        // Original Arc retains an extra share from each KindedSlot
        // (1 own + 2 wrappers = 3 strong refs; the local `payload_arc`
        // is the third).
        assert!(Arc::strong_count(&payload_arc) >= 2);
    }

    #[test]
    fn some_carrier_is_some_true() {
        let payload = KindedSlot::from_bool(true);
        let o = OptionData::some(payload);
        assert!(o.is_some);
        assert_eq!(o.payload.as_bool(), Some(true));
    }

    #[test]
    fn none_carrier_is_some_false() {
        let o = OptionData::none();
        assert!(!o.is_some);
        // None payload is a placeholder Bool-kind zero-bits slot —
        // KindedSlot::Drop is a no-op on it (verified by the slot
        // raw bits and kind).
        assert_eq!(o.payload.slot().raw(), 0);
    }

    #[test]
    fn smoke_target_ok_int_then_unwrap() {
        // Storage-layer counterpart of the W14-variant-codegen smoke
        // target: `let r = Ok(42); if r.is_ok() { print(r.unwrap_ok()) }`
        // outputs 42. The is_ok / unwrap_ok pair surfaces at the
        // storage tier as `r.is_ok` + `r.payload.as_i64()`.
        let r = ResultData::ok(KindedSlot::from_int(42));
        assert!(r.is_ok);
        assert_eq!(r.payload.as_i64(), Some(42));
    }

    #[test]
    fn arc_wrap_typed_pointer_round_trip() {
        // Pin the typed-Arc raw-pointer dispatch contract: wrap an
        // Arc<ResultData> via Arc::into_raw, recover via
        // Arc::from_raw, verify pointer identity. This is the slot-
        // bits transit path that the §2.7.17 dispatch tables retire
        // in `clone_with_kind` / `drop_with_kind`.
        let arc = Arc::new(ResultData::ok(KindedSlot::from_int(7)));
        let bits = Arc::into_raw(arc) as u64;
        // Recover and verify is_ok.
        let arc2: Arc<ResultData> =
            unsafe { Arc::from_raw(bits as *const ResultData) };
        assert!(arc2.is_ok);
        assert_eq!(arc2.payload.as_i64(), Some(7));
        drop(arc2);
    }
}

#[cfg(test)]
mod concurrency_storage {
    //! W17-concurrency (ADR-006 §2.7.25, 2026-05-11): pin the `lock`
    //! / `try_lock` / `set` / `get` API contracts on `MutexData`, the
    //! `load` / `store` / `fetch_add` / `fetch_sub` /
    //! `compare_exchange` contracts on `AtomicData`, and the
    //! `is_initialized` / `cached` / `take_initializer` /
    //! `store_result` contracts on `LazyData`. Storage-tier only —
    //! closure-call integration for `Lazy.get` lives at the handler
    //! tier (`executor/objects/concurrency_methods.rs`).
    use super::*;
    use crate::kinded_slot::KindedSlot;
    use std::sync::Arc;

    // ── MutexData ──────────────────────────────────────────────────

    #[test]
    fn mutex_new_holds_initial_value() {
        let m = MutexData::new(KindedSlot::from_int(42));
        assert_eq!(m.get().as_i64(), Some(42));
    }

    #[test]
    fn mutex_lock_is_noop_at_landing() {
        let m = MutexData::new(KindedSlot::from_int(0));
        m.lock();
        // lock returns; observable state unchanged.
        assert_eq!(m.get().as_i64(), Some(0));
    }

    #[test]
    fn mutex_try_lock_returns_true_uncontended() {
        let m = MutexData::new(KindedSlot::from_int(0));
        assert!(m.try_lock());
    }

    #[test]
    fn mutex_set_replaces_value_and_drops_prior() {
        // Storage-layer counterpart of the smoke target's
        // `m.set(5); print(m.value)`.
        let m = MutexData::new(KindedSlot::from_int(0));
        m.set(KindedSlot::from_int(5));
        assert_eq!(m.get().as_i64(), Some(5));
    }

    #[test]
    fn mutex_set_with_heap_payload_retires_shares() {
        // The prior slot drops cleanly when `set` replaces it; no
        // Arc-leak. A heap-bearing payload's strong-count returns to
        // zero after `set` and `drop(mutex)`.
        let s = Arc::new("initial".to_string());
        let weak = Arc::downgrade(&s);
        let m = MutexData::new(KindedSlot::from_string_arc(s));
        assert_eq!(weak.strong_count(), 1);
        m.set(KindedSlot::from_int(7));
        assert_eq!(
            weak.strong_count(),
            0,
            "Mutex.set must drop prior heap payload share"
        );
        drop(m);
    }

    #[test]
    fn mutex_shared_arc_observes_set_mutations() {
        // Two `Arc<MutexData>` shares of the same mutex observe each
        // other's mutations — the producer/consumer-endpoints shape
        // (mirror of Channel).
        let m1 = Arc::new(MutexData::new(KindedSlot::from_int(0)));
        let m2 = Arc::clone(&m1);
        m1.set(KindedSlot::from_int(99));
        assert_eq!(m2.get().as_i64(), Some(99));
    }

    // ── AtomicData ─────────────────────────────────────────────────

    #[test]
    fn atomic_new_holds_initial_value() {
        let a = AtomicData::new(7);
        assert_eq!(a.load(), 7);
    }

    #[test]
    fn atomic_store_replaces_value() {
        let a = AtomicData::new(0);
        a.store(42);
        assert_eq!(a.load(), 42);
    }

    #[test]
    fn atomic_fetch_add_returns_prior_and_increments() {
        // Smoke-target storage layer: a starts at 0, fetch_add(1)
        // returns 0 (prior), load() returns 1.
        let a = AtomicData::new(0);
        let prior = a.fetch_add(1);
        assert_eq!(prior, 0);
        assert_eq!(a.load(), 1);
    }

    #[test]
    fn atomic_fetch_sub_returns_prior_and_decrements() {
        let a = AtomicData::new(10);
        let prior = a.fetch_sub(3);
        assert_eq!(prior, 10);
        assert_eq!(a.load(), 7);
    }

    #[test]
    fn atomic_compare_exchange_swaps_on_match() {
        let a = AtomicData::new(5);
        let prior = a.compare_exchange(5, 99);
        assert_eq!(prior, 5);
        assert_eq!(a.load(), 99);
    }

    #[test]
    fn atomic_compare_exchange_keeps_on_mismatch() {
        let a = AtomicData::new(5);
        let prior = a.compare_exchange(7, 99);
        assert_eq!(prior, 5);
        assert_eq!(a.load(), 5);
    }

    #[test]
    fn atomic_shared_arc_observes_other_share() {
        let a1 = Arc::new(AtomicData::new(0));
        let a2 = Arc::clone(&a1);
        a1.store(42);
        assert_eq!(a2.load(), 42);
        a2.fetch_add(8);
        assert_eq!(a1.load(), 50);
    }

    // ── LazyData ───────────────────────────────────────────────────

    #[test]
    fn lazy_new_is_not_initialized() {
        let dummy_closure = KindedSlot::from_int(0);
        // Note: at the storage tier we don't actually call the
        // closure — that lives at the handler tier. The closure
        // payload here is just any `KindedSlot`; `is_initialized`
        // looks at the cached value, not the initializer.
        let l = LazyData::new(dummy_closure);
        assert!(!l.is_initialized());
    }

    #[test]
    fn lazy_take_initializer_then_store_result_marks_initialized() {
        // Simulates the handler-tier `lazy.get()` flow: take the
        // initializer, "run it" (the test substitutes a result), then
        // cache the result. After store_result, is_initialized=true
        // and cached() returns the stored value.
        let l = LazyData::new(KindedSlot::from_int(0));
        let init = l
            .take_initializer()
            .expect("initializer present before first get");
        assert!(!l.is_initialized());
        // "Run the initializer" — at storage-tier test we just drop
        // the initializer slot and synthesize a result.
        drop(init);
        l.store_result(KindedSlot::from_int(42));
        assert!(l.is_initialized());
        let got = l.cached().expect("cached after store_result");
        assert_eq!(got.as_i64(), Some(42));
    }

    #[test]
    fn lazy_take_initializer_returns_none_after_caching() {
        // After cache is populated, `take_initializer` returns None
        // — the handler tier's get() uses this to detect "already
        // initialized, use cached() instead".
        let l = LazyData::new(KindedSlot::from_int(0));
        let _init = l.take_initializer().unwrap();
        l.store_result(KindedSlot::from_int(7));
        assert!(l.take_initializer().is_none());
    }

    #[test]
    fn lazy_cached_returns_none_before_init() {
        let l = LazyData::new(KindedSlot::from_int(0));
        assert!(l.cached().is_none());
    }

    #[test]
    fn lazy_dropping_lazy_with_heap_payload_retires_shares() {
        // Refcount discipline: the cached `KindedSlot` owns one
        // strong-count share; dropping the LazyData retires it.
        let s = Arc::new("cached_value".to_string());
        let weak = Arc::downgrade(&s);
        let l = LazyData::new(KindedSlot::from_int(0));
        l.store_result(KindedSlot::from_string_arc(s));
        assert_eq!(weak.strong_count(), 1);
        drop(l);
        assert_eq!(
            weak.strong_count(),
            0,
            "Dropped LazyData must retire cached KindedSlot's share"
        );
    }
}

#[cfg(test)]
mod trait_object_storage {
    //! W17-trait-object-storage (ADR-006 §2.7.24 / Q25.C, 2026-05-11):
    //! pin the `TraitObjectStorage` API + refcount-discipline contracts.
    //! Storage-tier only — `OpCode::BoxTraitObject` /
    //! `OpCode::DynMethodCall` emission and end-to-end dyn-coerce smoke
    //! live in W17-trait-object-emission (round 2 of Wave 2.6).
    //!
    //! Coverage:
    //!   - Construction (`TraitObjectStorage::new`)
    //!   - vtable / value field access
    //!   - `method()` lookup
    //!   - `vtable_eq()` identity contract per §Q25.C.2
    //!   - Clone bumps both inner Arc strong counts
    //!   - Drop retires both inner Arc strong counts
    //!   - `KindedSlot::from_trait_object` retain-on-clone parity
    //!   - `KindedSlot::from_trait_object` drop-decrement parity
    //!   - `Arc<TraitObjectStorage>` clone roundtrip (via clone_with_kind
    //!     contract through the kind label, not through HeapValue)
    //!   - End-to-end retain/drop balance over multiple clones
    use super::*;
    use crate::kinded_slot::KindedSlot;
    use crate::native_kind::NativeKind;
    use crate::value::{VTable, VTableEntry};
    use std::collections::HashMap;
    use std::sync::Arc;

    /// Build a minimal `TypedObjectStorage` for tests — single i64 field,
    /// no heap-typed slots. Mirror of the shape used by concurrency
    /// tests' `KindedSlot::from_int` payloads.
    fn make_object(value: i64) -> Arc<TypedObjectStorage> {
        let mut slots: Vec<crate::slot::ValueSlot> = Vec::with_capacity(1);
        slots.push(crate::slot::ValueSlot::from_int(value));
        let field_kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64]);
        Arc::new(TypedObjectStorage::new(
            42, // schema_id — arbitrary
            slots.into_boxed_slice(),
            0,  // heap_mask: no heap slots
            field_kinds,
        ))
    }

    /// Build a minimal `VTable` for tests — one `Direct` method entry.
    fn make_vtable(trait_name: &str, concrete_type_id: u32, method: &str) -> Arc<VTable> {
        let mut methods: HashMap<String, VTableEntry> = HashMap::new();
        methods.insert(
            method.to_string(),
            VTableEntry::Direct { function_id: 7 },
        );
        Arc::new(VTable {
            trait_names: vec![trait_name.to_string()],
            concrete_type_id,
            methods,
        })
    }

    #[test]
    fn new_holds_value_and_vtable_arcs() {
        let obj = make_object(1);
        let vt = make_vtable("Animal", 100, "name");
        let storage = TraitObjectStorage::new(Arc::clone(&obj), Arc::clone(&vt));
        // Both halves remain accessible; vtable's concrete_type_id
        // matches what the impl declared.
        assert_eq!(storage.value.schema_id, 42);
        assert_eq!(storage.vtable.concrete_type_id, 100);
        assert_eq!(storage.vtable.trait_names[0], "Animal");
    }

    #[test]
    fn method_lookup_returns_entry_for_known_method() {
        let obj = make_object(1);
        let vt = make_vtable("Animal", 100, "name");
        let storage = TraitObjectStorage::new(obj, vt);
        let entry = storage
            .method("name")
            .expect("known method present in vtable");
        match entry {
            VTableEntry::Direct { function_id } => assert_eq!(*function_id, 7),
            _ => panic!("expected Direct entry"),
        }
    }

    #[test]
    fn method_lookup_returns_none_for_unknown_method() {
        let obj = make_object(1);
        let vt = make_vtable("Animal", 100, "name");
        let storage = TraitObjectStorage::new(obj, vt);
        assert!(storage.method("speak").is_none());
    }

    #[test]
    fn vtable_eq_identifies_same_vtable_share() {
        // Per §Q25.C.2, vtable-identity check uses `Arc::ptr_eq`. Two
        // carriers built from the same vtable Arc compare equal.
        let obj1 = make_object(1);
        let obj2 = make_object(2);
        let vt = make_vtable("Animal", 100, "name");
        let s1 = TraitObjectStorage::new(obj1, Arc::clone(&vt));
        let s2 = TraitObjectStorage::new(obj2, Arc::clone(&vt));
        assert!(s1.vtable_eq(&s2));
    }

    #[test]
    fn vtable_eq_rejects_distinct_vtables() {
        // Two carriers built from distinct vtables — even if the
        // trait name matches — fail the identity check.
        let obj1 = make_object(1);
        let obj2 = make_object(2);
        let vt1 = make_vtable("Animal", 100, "name");
        let vt2 = make_vtable("Animal", 100, "name");
        let s1 = TraitObjectStorage::new(obj1, vt1);
        let s2 = TraitObjectStorage::new(obj2, vt2);
        assert!(!s1.vtable_eq(&s2));
    }

    #[test]
    fn clone_bumps_both_inner_arcs() {
        // TraitObjectStorage::clone is a pair of Arc bumps. Verify
        // each inner Arc's strong-count increases by one.
        let obj = make_object(1);
        let vt = make_vtable("Animal", 100, "name");
        let obj_weak = Arc::downgrade(&obj);
        let vt_weak = Arc::downgrade(&vt);
        let storage = TraitObjectStorage::new(Arc::clone(&obj), Arc::clone(&vt));
        // Now: external obj/vt + storage's clones = 2 strong each.
        // Drop the externals so we can observe the storage's owned shares.
        drop(obj);
        drop(vt);
        assert_eq!(obj_weak.strong_count(), 1);
        assert_eq!(vt_weak.strong_count(), 1);

        // Clone storage — both inner Arcs should bump.
        let storage2 = storage.clone();
        assert_eq!(obj_weak.strong_count(), 2);
        assert_eq!(vt_weak.strong_count(), 2);

        drop(storage);
        assert_eq!(obj_weak.strong_count(), 1);
        assert_eq!(vt_weak.strong_count(), 1);

        drop(storage2);
        assert_eq!(obj_weak.strong_count(), 0);
        assert_eq!(vt_weak.strong_count(), 0);
    }

    #[test]
    fn kinded_slot_from_trait_object_drop_decrement() {
        // The §2.7.7 retain-on-read protocol via `KindedSlot::Drop`
        // must retire one `Arc<TraitObjectStorage>` share when the
        // slot drops. Verify the strong-count returns to zero after
        // both the original Arc and the KindedSlot drop.
        let obj = make_object(99);
        let vt = make_vtable("Animal", 100, "name");
        let storage = Arc::new(TraitObjectStorage::new(obj, vt));
        let weak = Arc::downgrade(&storage);
        let slot = KindedSlot::from_trait_object(Arc::clone(&storage));
        // External: 1 share. Slot: 1 share. Total: 2.
        assert_eq!(weak.strong_count(), 2);

        // Drop the external Arc — slot still holds one share.
        drop(storage);
        assert_eq!(weak.strong_count(), 1);

        // Drop the slot — retires the last share.
        drop(slot);
        assert_eq!(weak.strong_count(), 0);
    }

    #[test]
    fn kinded_slot_clone_bumps_share() {
        // KindedSlot::Clone retains via clone_with_kind — verify the
        // share count tracks correctly across clones.
        let obj = make_object(1);
        let vt = make_vtable("Animal", 100, "name");
        let storage = Arc::new(TraitObjectStorage::new(obj, vt));
        let weak = Arc::downgrade(&storage);
        let slot1 = KindedSlot::from_trait_object(Arc::clone(&storage));
        let slot2 = slot1.clone();
        let slot3 = slot2.clone();
        // External + 3 slots = 4 shares.
        assert_eq!(weak.strong_count(), 4);

        drop(storage);
        drop(slot1);
        drop(slot2);
        // 1 slot remaining.
        assert_eq!(weak.strong_count(), 1);
        drop(slot3);
        assert_eq!(weak.strong_count(), 0);
    }

    #[test]
    fn kinded_slot_kind_label_is_ptr_trait_object() {
        // The kind label must be `NativeKind::Ptr(HeapKind::TraitObject)`
        // for the §2.7.7 dispatch tables (clone_with_kind /
        // drop_with_kind) to find the correct arm. This pins the
        // construction-side contract.
        let obj = make_object(1);
        let vt = make_vtable("Animal", 100, "name");
        let storage = Arc::new(TraitObjectStorage::new(obj, vt));
        let slot = KindedSlot::from_trait_object(storage);
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TraitObject));
    }

    #[test]
    fn slot_bits_recover_to_typed_arc_via_canonical_pattern() {
        // The canonical recovery pattern (per `3ac2f11` precedent):
        // bits = slot.raw(), Arc::from_raw, clone, into_raw. Verify
        // that round-tripping the raw bits through Arc::from_raw
        // recovers an Arc with the expected vtable identity.
        let obj = make_object(7);
        let vt = make_vtable("Animal", 100, "name");
        let storage = Arc::new(TraitObjectStorage::new(obj, vt));
        let original_vt_ptr = Arc::as_ptr(&storage.vtable);
        let slot = KindedSlot::from_trait_object(Arc::clone(&storage));
        let bits = slot.slot().raw();

        // SAFETY: bits came from KindedSlot::from_trait_object which
        // stores `Arc::into_raw(Arc<TraitObjectStorage>)`. The slot
        // owns the share; we leak the recovered Arc back to keep the
        // slot's share intact for normal drop discipline.
        let recovered: Arc<TraitObjectStorage> =
            unsafe { Arc::from_raw(bits as *const TraitObjectStorage) };
        let cloned = Arc::clone(&recovered);
        let _ = Arc::into_raw(recovered); // restore slot's share

        // Recovered Arc points to the same storage — same vtable Arc.
        assert!(Arc::ptr_eq(&cloned.vtable, &storage.vtable));
        // Pointer-equality on the inner vtable's raw pointer matches.
        assert_eq!(Arc::as_ptr(&cloned.vtable), original_vt_ptr);

        drop(cloned);
        drop(slot);
        drop(storage);
    }

    #[test]
    fn dropping_trait_object_with_typed_object_payload_retires_payload_share() {
        // Refcount discipline at the carrier level: drop the
        // TraitObjectStorage Arc to zero strong count, and verify
        // the inner TypedObject Arc's strong count returns to zero
        // too. This is the end-to-end retain/drop balance for the
        // fat-pointer carrier.
        let obj = make_object(99);
        let vt = make_vtable("Animal", 100, "name");
        let obj_weak = Arc::downgrade(&obj);
        let vt_weak = Arc::downgrade(&vt);
        let storage = Arc::new(TraitObjectStorage::new(obj, vt));
        // After move: storage holds the only shares of obj + vt.
        assert_eq!(obj_weak.strong_count(), 1);
        assert_eq!(vt_weak.strong_count(), 1);

        drop(storage);
        assert_eq!(obj_weak.strong_count(), 0, "TypedObject share must retire");
        assert_eq!(vt_weak.strong_count(), 0, "VTable share must retire");
    }
}

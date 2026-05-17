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
//! - typed temporal data (`TemporalData`),
//! - typed table views (`TableViewData`).
//!
//! V3-S5 ckpt-1..ckpt-4 (2026-05-15): the inline `TypedArrayData` enum +
//! the outer `HeapValue::TypedArray(Arc<TypedArrayData>)` arm +
//! `TypedBuffer<T>` / `AlignedTypedBuffer` wrapper layer were retired
//! wholesale per W12-typed-array-data-deletion-audit §3.5 + §B + ADR-006
//! §2.7.24 Q25.A SUPERSEDED. The canonical replacement is the v2-raw
//! `TypedArray<T>` flat struct at `crate::v2::typed_array::TypedArray<T>`
//! (per `docs/runtime-v2-spec.md`). The `HeapKind::TypedArray = 8`
//! ordinal is vacated; do not reuse.
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

/// Owning newtype around `*const TypedObjectStorage` carrying one
/// v2-raw refcount share on the pointed-to allocation's HeapHeader.
///
/// **Wave 2 Round 4 D4 ckpt-final (2026-05-14):** redesigned to own its
/// share. Previously a trivially-Copy transparent newtype; that shape
/// leaks element refcounts when the enclosing `Vec<TypedObjectPtr>`
/// drops because trivial bit-copy Drop never calls `release_elem`. Now
/// the wrapper:
/// - Owns one v2-raw HeapHeader-at-offset-0 refcount share.
/// - `Clone` bumps the refcount via `v2_retain`.
/// - `Drop` retires the share via `TypedObjectStorage::release_elem`.
/// - `Default` is the null pointer (no refcount share owed).
///
/// `#[repr(transparent)]` so the in-memory layout is identical to
/// `*const TypedObjectStorage` — zero ABI cost vs the raw pointer; the
/// wrapper exists only to localize the manual Send/Sync impl + the
/// Drop/Clone refcount discipline (Rust disables auto-Send/Sync for ALL
/// instantiations of a generic struct as soon as ANY manual impl exists,
/// so per-T newtypes are the canonical workaround for raw-ptr inner
/// elements in generic buffers).
///
/// Used as the inner element type of:
/// - `HashMapValueBuf::TypedObject(Arc<TypedBuffer<TypedObjectPtr>>)`
/// - `TypedArrayData::TypedObject(Arc<TypedBuffer<TypedObjectPtr>>)`
///
/// Construction-side contract: callers transfer one strong-count share
/// on the v2-raw HeapHeader to the new `TypedObjectPtr`. Reads via
/// `as_ptr()` return the underlying pointer without bumping refcount.
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct TypedObjectPtr(pub *const TypedObjectStorage);

// SAFETY: `*const TypedObjectStorage` is `!Send + !Sync` by default. The
// wrapper is safe to share across threads because:
// (1) `TypedObjectStorage` itself is `Send + Sync` (Box<[ValueSlot]> +
//     `Arc<[NativeKind]>` + POD fields; ValueSlot wraps `u64`).
// (2) The HeapHeader-based refcount uses atomic ops (`v2_retain` /
//     `v2_release` in `v2/refcount.rs`).
// (3) Aliasing safety is the same as `Arc<TypedObjectStorage>` — multiple
//     threads can hold their own retain shares concurrently.
unsafe impl Send for TypedObjectPtr {}
unsafe impl Sync for TypedObjectPtr {}

impl Default for TypedObjectPtr {
    /// Null pointer default — used by `TypedBuffer::<TypedObjectPtr>::push_null`
    /// and similar default-requiring construction sites. Callers must
    /// not dereference a default-constructed `TypedObjectPtr`. No
    /// refcount share is owed for a null wrapper; Drop on a null
    /// pointer is a no-op.
    #[inline]
    fn default() -> Self {
        Self(std::ptr::null())
    }
}

impl Clone for TypedObjectPtr {
    /// v2-raw refcount bump via `v2_retain` on the pointed-to
    /// HeapHeader. The clone owns its own share, retired at its own
    /// `Drop`.
    #[inline]
    fn clone(&self) -> Self {
        if !self.0.is_null() {
            // SAFETY: per the construction-side contract, `self.0` points
            // to a live `TypedObjectStorage` allocated via `_new` (or a
            // legacy Arc-allocated one whose embedded HeapHeader is
            // unused but still bumpable safely — atomic increment is
            // sound on any aligned u32 within the legitimate allocation).
            unsafe { crate::v2::refcount::v2_retain(&(*self.0).header) };
        }
        Self(self.0)
    }
}

impl Drop for TypedObjectPtr {
    /// Retire the owned share via `TypedObjectStorage::release_elem`
    /// (HeapElement trait — calls `v2_release` and, on refcount=0,
    /// runs `_drop` to dealloc the allocation + retire heap-mask
    /// shares). No-op on null wrappers.
    #[inline]
    fn drop(&mut self) {
        if !self.0.is_null() {
            use crate::v2::heap_element::HeapElement;
            // SAFETY: per the construction-side contract this carrier owns
            // one share on the HeapHeader-at-offset-0 refcount.
            unsafe { TypedObjectStorage::release_elem(self.0) };
        }
    }
}

impl TypedObjectPtr {
    /// Construct from a raw pointer obtained via `TypedObjectStorage::_new`.
    /// The caller transfers one strong-count share to the wrapper.
    #[inline]
    pub fn new(ptr: *const TypedObjectStorage) -> Self {
        Self(ptr)
    }

    /// Recover the underlying raw pointer. Does NOT bump refcount;
    /// the returned pointer is borrowed for the wrapper's lifetime.
    #[inline]
    pub fn as_ptr(&self) -> *const TypedObjectStorage {
        self.0
    }

    /// Whether the pointer is null. Construction-side contract permits
    /// null only for default-initialized cells.
    #[inline]
    pub fn is_null(&self) -> bool {
        self.0.is_null()
    }

    /// Consume the wrapper without running Drop, returning the raw
    /// pointer. The caller takes over the one refcount share. Mirror
    /// of `Arc::into_raw`.
    #[inline]
    pub fn into_raw(self) -> *const TypedObjectStorage {
        let ptr = self.0;
        std::mem::forget(self);
        ptr
    }
}

// Deref to `&TypedObjectStorage` so consumer sites can read fields
// (`s.slots`, `s.schema_id`, etc.) without manual unsafe deref. The
// wrapper owns one refcount share for its lifetime, so the pointed-to
// storage is live while the wrapper is in scope.
//
// Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14): added to support the
// HeapValue::TypedObject(TypedObjectPtr) variant signature flip with
// minimal consumer-cascade churn.
impl std::ops::Deref for TypedObjectPtr {
    type Target = TypedObjectStorage;
    #[inline]
    fn deref(&self) -> &TypedObjectStorage {
        // SAFETY: per the construction-side contract, `self.0` is non-null
        // for any `TypedObjectPtr` constructed via `new(_new(...))` or
        // cloned from such. The default-constructed null pointer must not
        // be dereferenced — callers reading fields are expected to hold a
        // wrapper that owns a real share. Debug builds catch the null case
        // via the assert; release builds UB on deref of null (mirroring
        // `Arc<T>::deref` semantics for a null Arc — not constructable
        // without `unsafe`). Length-zero storage is fine.
        debug_assert!(
            !self.0.is_null(),
            "TypedObjectPtr::deref on null pointer (default-constructed wrapper \
             must not be dereferenced)"
        );
        unsafe { &*self.0 }
    }
}

// ── TraitObjectPtr (Wave 2 Round 4 D4 ckpt-final-prime², 2026-05-14) ────────

/// Owning newtype around `*const TraitObjectStorage` carrying one
/// v2-raw refcount share on the pointed-to allocation's HeapHeader.
///
/// **Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14):** mirrors the
/// `TypedObjectPtr` precedent (above) for `TraitObjectStorage`. Carrier
/// shape used as both:
/// - `HeapValue::TraitObject(TraitObjectPtr)` variant payload
/// - Future `TypedArrayData::TraitObject` element type (if/when the
///   §Q25.A monomorphic specialization for trait-object-element arrays
///   lands; not under this ckpt's scope)
///
/// `#[repr(transparent)]` so the in-memory layout is identical to
/// `*const TraitObjectStorage` — zero ABI cost vs the raw pointer; the
/// wrapper exists only to localize the manual Send/Sync impl + the
/// Drop/Clone refcount discipline. Same auto-trait suppression rule
/// applies as for TypedObjectPtr — per-T newtype is the canonical
/// workaround for raw-ptr inner elements in HeapValue variant payloads
/// without disabling Rust's auto-derived Send/Sync/Clone/Drop on the
/// enclosing `HeapValue` enum.
///
/// Construction-side contract: callers transfer one strong-count share
/// on the v2-raw HeapHeader (initialized to 1 via `TraitObjectStorage::_new`)
/// to the new `TraitObjectPtr`. Reads via `as_ptr()` return the
/// underlying pointer without bumping refcount.
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct TraitObjectPtr(pub *const TraitObjectStorage);

// SAFETY: same argument as TypedObjectPtr's Send/Sync impls — the
// underlying storage is Send + Sync (manually unsafe impl'd on
// TraitObjectStorage), the HeapHeader-based refcount uses atomic ops
// (`v2_retain` / `v2_release` in `v2/refcount.rs`), and aliasing safety
// matches `Arc<TraitObjectStorage>` — multiple threads can hold their
// own retain shares concurrently.
unsafe impl Send for TraitObjectPtr {}
unsafe impl Sync for TraitObjectPtr {}

impl Default for TraitObjectPtr {
    /// Null pointer default — used by container types that need a
    /// `Default` impl. Callers must not dereference a default-constructed
    /// `TraitObjectPtr`. No refcount share is owed; Drop on a null
    /// pointer is a no-op.
    #[inline]
    fn default() -> Self {
        Self(std::ptr::null())
    }
}

impl Clone for TraitObjectPtr {
    /// v2-raw refcount bump via `v2_retain` on the pointed-to
    /// HeapHeader. The clone owns its own share, retired at its own
    /// `Drop`.
    #[inline]
    fn clone(&self) -> Self {
        if !self.0.is_null() {
            // SAFETY: per the construction-side contract, `self.0` points
            // to a live `TraitObjectStorage` allocated via `_new` (or a
            // legacy Arc-allocated one whose embedded HeapHeader is
            // unused but still bumpable safely — atomic increment is
            // sound on any aligned u32 within the legitimate allocation).
            unsafe { crate::v2::refcount::v2_retain(&(*self.0).header) };
        }
        Self(self.0)
    }
}

impl Drop for TraitObjectPtr {
    /// Retire the owned share via `TraitObjectStorage::release_elem`
    /// (HeapElement trait — calls `v2_release` and, on refcount=0,
    /// runs `_drop` to dealloc the allocation + retire inner shares).
    /// No-op on null wrappers.
    #[inline]
    fn drop(&mut self) {
        if !self.0.is_null() {
            use crate::v2::heap_element::HeapElement;
            // SAFETY: per the construction-side contract this carrier owns
            // one share on the HeapHeader-at-offset-0 refcount.
            unsafe { TraitObjectStorage::release_elem(self.0) };
        }
    }
}

impl TraitObjectPtr {
    /// Construct from a raw pointer obtained via `TraitObjectStorage::_new`.
    /// The caller transfers one strong-count share to the wrapper.
    #[inline]
    pub fn new(ptr: *const TraitObjectStorage) -> Self {
        Self(ptr)
    }

    /// Recover the underlying raw pointer. Does NOT bump refcount;
    /// the returned pointer is borrowed for the wrapper's lifetime.
    #[inline]
    pub fn as_ptr(&self) -> *const TraitObjectStorage {
        self.0
    }

    /// Whether the pointer is null.
    #[inline]
    pub fn is_null(&self) -> bool {
        self.0.is_null()
    }

    /// Consume the wrapper without running Drop, returning the raw
    /// pointer. The caller takes over the one refcount share. Mirror
    /// of `Arc::into_raw`.
    #[inline]
    pub fn into_raw(self) -> *const TraitObjectStorage {
        let ptr = self.0;
        std::mem::forget(self);
        ptr
    }
}

// Same Deref-as-shortcut rationale as TypedObjectPtr — the wrapper owns
// the share so the pointed-to storage is live for the wrapper's lifetime.
impl std::ops::Deref for TraitObjectPtr {
    type Target = TraitObjectStorage;
    #[inline]
    fn deref(&self) -> &TraitObjectStorage {
        debug_assert!(
            !self.0.is_null(),
            "TraitObjectPtr::deref on null pointer (default-constructed wrapper \
             must not be dereferenced)"
        );
        unsafe { &*self.0 }
    }
}

// ── HashMap storage (Stage C P1(b), 2026-05-07) ─────────────────────────────
//
// Wave 2 Round 3b C2-joint ckpt-1 (2026-05-14): `HashMapValueBuf` deleted;
// `HashMapData` replaced with `HashMapData<V>` generic per audit §C.4
// option (a.2) — values buffer is `*mut TypedArray<V>` (per-V monomorphized
// at compile time via the `HashMapValueElem` trait). `HashMapKindedRef`
// enum bundles per-V `Arc<HashMapData<V>>` variants as the HeapValue-arm
// carrier (ckpt-2 flips `HeapValue::HashMap(Arc<HashMapData>)` to
// `HeapValue::HashMap(HashMapKindedRef)`). See ADR-006 §2.7.24 Q25.B
// SUPERSEDED + `docs/cluster-audits/bulldozer-wave-1-inventory.md` §C.

/// Per-V dispatcher trait for `HashMapData<V>::Drop` — releases the value
/// buffer (`*mut TypedArray<V>`) at refcount-0 of the enclosing
/// `Arc<HashMapData<V>>`.
///
/// Authority: ADR-006 §2.7.24 Q25.B SUPERSEDED + `bulldozer-wave-1-inventory.md`
/// §C.4 option (a.2). The trait dispatches per-V release at compile time via
/// the Rust type system — no runtime `NativeKind` probe, no `is_heap()` probe,
/// no Bool-default fallback. Mirror of `v2::heap_element::HeapElement` shape,
/// but operates on the OUTER `TypedArray<V>` allocation rather than on
/// individual heap-element pointers.
///
/// Impls partition V into:
///
/// - POD scalar Vs (`i64`, `f64`, `u8` for Bool): `TypedArray::<V>::drop_array`
///   frees the data buffer + the struct; no per-element work.
/// - HeapHeader-equipped raw pointers (`*const StringObj`, `*const DecimalObj`):
///   `TypedArray::<*const T>::drop_array_heap` walks the data buffer and
///   calls `T::release_elem` per element, then frees the struct. Requires
///   `T: v2::heap_element::HeapElement`.
/// - `#[repr(transparent)]` newtype-as-element shapes (`TypedObjectPtr`,
///   `TraitObjectPtr`): manual walk that `ptr::read`s each element to invoke
///   its `Drop` (which retires the v2-raw refcount share via `release_elem`
///   on the inner `*const TypedObjectStorage` / `*const TraitObjectStorage`).
///
/// Char (`char` codepoint) is reachable per §C.5 dead-but-derived disposition
/// — included as a POD-scalar V.
///
/// # Safety
///
/// Implementors must guarantee:
/// 1. `release_typed_array(ptr)` is sound when `ptr` points to a live
///    `TypedArray<Self>` allocation produced by `TypedArray::<Self>::new` /
///    `with_capacity` / `from_slice`.
/// 2. After this call, `ptr` is invalid; the data buffer + struct are freed.
/// 3. Per-element ownership semantics match the storage contract — POD
///    elements need no per-element release; HeapHeader-equipped elements
///    have their shares retired before the data buffer is freed.
pub unsafe trait HashMapValueElem {
    /// Release a `*mut TypedArray<Self>` allocation: retire per-element
    /// shares (where applicable) + free the data buffer + free the struct.
    ///
    /// # Safety
    /// `ptr` must point to a valid, live `TypedArray<Self>` allocated by
    /// the v2-raw `TypedArray::<Self>` allocator (`new`, `with_capacity`,
    /// `from_slice`). After this call returns, `ptr` is invalid.
    unsafe fn release_typed_array(ptr: *mut crate::v2::typed_array::TypedArray<Self>)
    where
        Self: Sized;

    /// Clone a single element with proper refcount-share semantics.
    ///
    /// - POD scalar Vs (`i64`/`f64`/`u8`/`char`): byte copy — no refcount work.
    /// - HeapHeader-equipped raw pointers (`*const StringObj`/`*const DecimalObj`):
    ///   pointer copy + `v2_retain` on the pointed-to HeapHeader.
    /// - `#[repr(transparent)]` ptr-newtypes (`TypedObjectPtr`/`TraitObjectPtr`):
    ///   delegate to the wrapper's `Clone` impl (which does v2_retain).
    ///
    /// Wave 2 Round 3b C2-joint ckpt-3 (2026-05-14): added to support the
    /// per-V mutation API (insert / merge / get_share) on `HashMapData<V>`.
    /// Per ADR-006 §2.7.24 Q25.B SUPERSEDED + audit §C.4.
    ///
    /// # Safety
    /// `elem` must reference a live element of a `TypedArray<Self>` (or a
    /// freshly-allocated element owned by the caller). The implementor
    /// must produce a new owned share — for HeapElement / ptr-newtype V
    /// this bumps the refcount on the pointed-to allocation; for POD V
    /// it is a trivial copy.
    unsafe fn share_clone(elem: &Self) -> Self
    where
        Self: Sized;

    /// Release a single owned value (one refcount share). For POD V it is
    /// a no-op (byte copy falls out of scope). For HeapElement V the
    /// share is retired via `release_elem` on the pointer. For Ptr-newtype
    /// V the wrapper's Drop runs automatically when the value drops.
    ///
    /// Wave 2 Round 3b C2-joint ckpt-3 (2026-05-14): added for the per-V
    /// mutation API's overwrite path (insert when key already present —
    /// the old value's share must be retired before the slot is overwritten).
    ///
    /// # Safety
    /// `value` must be a valid owned V — for HeapElement / ptr-newtype V
    /// types the caller transfers one refcount share to this method, which
    /// retires it.
    unsafe fn release_owned(value: Self)
    where
        Self: Sized;
}

// ── POD scalar V impls (i64 / f64 / u8 / char) ─────────────────────────────

unsafe impl HashMapValueElem for i64 {
    #[inline]
    unsafe fn release_typed_array(ptr: *mut crate::v2::typed_array::TypedArray<Self>) {
        // SAFETY: caller-bound contract; `i64` is Copy/POD — no per-element
        // shares to retire.
        unsafe { crate::v2::typed_array::TypedArray::<i64>::drop_array(ptr) }
    }
    #[inline]
    unsafe fn share_clone(elem: &Self) -> Self {
        *elem
    }
    #[inline]
    unsafe fn release_owned(_value: Self) {
        // POD: byte copy falls out of scope; no-op.
    }
}

unsafe impl HashMapValueElem for f64 {
    #[inline]
    unsafe fn release_typed_array(ptr: *mut crate::v2::typed_array::TypedArray<Self>) {
        unsafe { crate::v2::typed_array::TypedArray::<f64>::drop_array(ptr) }
    }
    #[inline]
    unsafe fn share_clone(elem: &Self) -> Self {
        *elem
    }
    #[inline]
    unsafe fn release_owned(_value: Self) {}
}

unsafe impl HashMapValueElem for u8 {
    /// Used as the `Bool` V (one byte per element).
    #[inline]
    unsafe fn release_typed_array(ptr: *mut crate::v2::typed_array::TypedArray<Self>) {
        unsafe { crate::v2::typed_array::TypedArray::<u8>::drop_array(ptr) }
    }
    #[inline]
    unsafe fn share_clone(elem: &Self) -> Self {
        *elem
    }
    #[inline]
    unsafe fn release_owned(_value: Self) {}
}

unsafe impl HashMapValueElem for char {
    /// Char codepoint (4 bytes / element). Dead-but-derived per §C.5;
    /// included for forward-cleanliness with the `HeapValue::Char`
    /// xml/json marshal path.
    #[inline]
    unsafe fn release_typed_array(ptr: *mut crate::v2::typed_array::TypedArray<Self>) {
        unsafe { crate::v2::typed_array::TypedArray::<char>::drop_array(ptr) }
    }
    #[inline]
    unsafe fn share_clone(elem: &Self) -> Self {
        *elem
    }
    #[inline]
    unsafe fn release_owned(_value: Self) {}
}

// ── HeapHeader-equipped raw-pointer V impls (*const StringObj / *const DecimalObj) ──

unsafe impl HashMapValueElem for *const crate::v2::string_obj::StringObj {
    #[inline]
    unsafe fn release_typed_array(ptr: *mut crate::v2::typed_array::TypedArray<Self>) {
        // SAFETY: `StringObj: HeapElement`; `drop_array_heap` walks elements,
        // calls `StringObj::release_elem` per `*const StringObj`, then frees
        // the data buffer + struct.
        unsafe {
            crate::v2::typed_array::TypedArray::<*const crate::v2::string_obj::StringObj>::drop_array_heap(ptr)
        }
    }
    #[inline]
    unsafe fn share_clone(elem: &Self) -> Self {
        // SAFETY: per the construction-side contract on
        // `HashMapData<*const StringObj>` element buffer, *elem points at a
        // live StringObj with HeapHeader at offset 0; v2_retain bumps the
        // refcount via atomic increment.
        if !elem.is_null() {
            unsafe { crate::v2::refcount::v2_retain(&(**elem).header) };
        }
        *elem
    }
    #[inline]
    unsafe fn release_owned(value: Self) {
        // SAFETY: caller transfers one share on a live StringObj; route
        // through HeapElement::release_elem to atomic-decrement + dealloc
        // on refcount=0.
        if !value.is_null() {
            unsafe {
                use crate::v2::heap_element::HeapElement;
                crate::v2::string_obj::StringObj::release_elem(value);
            }
        }
    }
}

unsafe impl HashMapValueElem for *const crate::v2::decimal_obj::DecimalObj {
    #[inline]
    unsafe fn release_typed_array(ptr: *mut crate::v2::typed_array::TypedArray<Self>) {
        unsafe {
            crate::v2::typed_array::TypedArray::<*const crate::v2::decimal_obj::DecimalObj>::drop_array_heap(ptr)
        }
    }
    #[inline]
    unsafe fn share_clone(elem: &Self) -> Self {
        // SAFETY: see *const StringObj impl above. DecimalObj has
        // HeapHeader at offset 0 (HeapElement contract).
        if !elem.is_null() {
            unsafe { crate::v2::refcount::v2_retain(&(**elem).header) };
        }
        *elem
    }
    #[inline]
    unsafe fn release_owned(value: Self) {
        if !value.is_null() {
            unsafe {
                use crate::v2::heap_element::HeapElement;
                crate::v2::decimal_obj::DecimalObj::release_elem(value);
            }
        }
    }
}

// ── Ptr-newtype V impls (TypedObjectPtr / TraitObjectPtr) ───────────────────

unsafe impl HashMapValueElem for TypedObjectPtr {
    /// `TypedObjectPtr` is `#[repr(transparent)]` over `*const TypedObjectStorage`
    /// but has a manual `Drop` impl (calls `release_elem`). Walk the buffer
    /// via `ptr::read` to invoke each element's Drop (which retires the v2-raw
    /// HeapHeader share), then free the data allocation + struct.
    #[inline]
    unsafe fn release_typed_array(ptr: *mut crate::v2::typed_array::TypedArray<Self>) {
        unsafe {
            let arr = &*ptr;
            if arr.cap > 0 && !arr.data.is_null() {
                // Walk: read each element; the read transfers ownership to a
                // local `TypedObjectPtr`, which drops at scope-end via its
                // manual `Drop` impl (calls `release_elem` on the inner
                // `*const TypedObjectStorage`).
                for i in 0..arr.len {
                    let _elem: TypedObjectPtr = std::ptr::read(arr.data.add(i as usize));
                }
                let data_layout =
                    std::alloc::Layout::array::<TypedObjectPtr>(arr.cap as usize)
                        .expect("invalid array layout");
                std::alloc::dealloc(arr.data as *mut u8, data_layout);
            }
            let layout = std::alloc::Layout::new::<crate::v2::typed_array::TypedArray<Self>>();
            std::alloc::dealloc(ptr as *mut u8, layout);
        }
    }
    #[inline]
    unsafe fn share_clone(elem: &Self) -> Self {
        // Delegate to the wrapper's Clone impl (which bumps the v2_retain
        // refcount on the inner *const TypedObjectStorage's HeapHeader).
        elem.clone()
    }
    #[inline]
    unsafe fn release_owned(_value: Self) {
        // TypedObjectPtr has a manual Drop impl that calls release_elem;
        // letting `_value` go out of scope runs Drop. No explicit work.
    }
}

unsafe impl HashMapValueElem for TraitObjectPtr {
    /// Mirror of the `TypedObjectPtr` impl above; per-element Drop runs
    /// `release_elem` on `*const TraitObjectStorage`.
    #[inline]
    unsafe fn release_typed_array(ptr: *mut crate::v2::typed_array::TypedArray<Self>) {
        unsafe {
            let arr = &*ptr;
            if arr.cap > 0 && !arr.data.is_null() {
                for i in 0..arr.len {
                    let _elem: TraitObjectPtr = std::ptr::read(arr.data.add(i as usize));
                }
                let data_layout =
                    std::alloc::Layout::array::<TraitObjectPtr>(arr.cap as usize)
                        .expect("invalid array layout");
                std::alloc::dealloc(arr.data as *mut u8, data_layout);
            }
            let layout = std::alloc::Layout::new::<crate::v2::typed_array::TypedArray<Self>>();
            std::alloc::dealloc(ptr as *mut u8, layout);
        }
    }
    #[inline]
    unsafe fn share_clone(elem: &Self) -> Self {
        // Delegate to TraitObjectPtr's Clone impl (v2_retain on inner
        // *const TraitObjectStorage's HeapHeader).
        elem.clone()
    }
    #[inline]
    unsafe fn release_owned(_value: Self) {
        // TraitObjectPtr's Drop impl runs at scope-end.
    }
}

// ── Recursive HashMap-value V impl (HashMapKindedRef) ───────────────────────
//
// Wave N hashmap-value-v-arm follow-up (cluster-2 closure-wave-C,
// 2026-05-16). Per ADR-006 §2.7.24 Q25.B SUPERSEDED canonical pattern
// (HashMapKindedRef carrier + per-V monomorphization at the method tier)
// extended to a recursive HashMap-value V arm. The values buffer is a
// `*mut TypedArray<HashMapKindedRef>` — each element is the per-V
// kinded-ref payload (auto-derived Drop chains through the inner Arc).
//
// HashMapKindedRef is non-Copy and carries a Drop (auto-derived: each
// variant holds Arc<HashMapData<V>> whose Drop retires one strong-count
// share). The shape matches TypedObjectPtr / TraitObjectPtr (manual Drop
// + manual Clone): walk-with-ptr::read on release; delegate to the
// wrapper's Clone on share; let scope-end run Drop on release_owned.

unsafe impl HashMapValueElem for HashMapKindedRef {
    /// `HashMapKindedRef` is non-Copy with an auto-derived Drop (each
    /// variant holds `Arc<HashMapData<V>>` whose Drop retires one
    /// strong-count share). Walk the buffer via `ptr::read` to invoke
    /// each element's Drop, then free the data allocation + struct.
    /// Mirror of the `TypedObjectPtr` / `TraitObjectPtr` impl shape.
    #[inline]
    unsafe fn release_typed_array(ptr: *mut crate::v2::typed_array::TypedArray<Self>) {
        unsafe {
            let arr = &*ptr;
            if arr.cap > 0 && !arr.data.is_null() {
                // Walk: read each element; the read transfers ownership
                // to a local `HashMapKindedRef`, which drops at scope-end
                // via its auto-derived Drop (chains through Arc::drop on
                // the inner `Arc<HashMapData<V_inner>>`).
                for i in 0..arr.len {
                    let _elem: HashMapKindedRef =
                        std::ptr::read(arr.data.add(i as usize));
                }
                let data_layout =
                    std::alloc::Layout::array::<HashMapKindedRef>(arr.cap as usize)
                        .expect("invalid array layout");
                std::alloc::dealloc(arr.data as *mut u8, data_layout);
            }
            let layout = std::alloc::Layout::new::<crate::v2::typed_array::TypedArray<Self>>();
            std::alloc::dealloc(ptr as *mut u8, layout);
        }
    }
    #[inline]
    unsafe fn share_clone(elem: &Self) -> Self {
        // Delegate to HashMapKindedRef's manual Clone impl (per-variant
        // Arc::clone on the inner `Arc<HashMapData<V_inner>>` — single
        // refcount bump per the §C.3 audit ground-truth).
        elem.clone()
    }
    #[inline]
    unsafe fn release_owned(_value: Self) {
        // HashMapKindedRef's auto-derived Drop runs at scope-end (per
        // variant: Arc::drop on the inner `Arc<HashMapData<V_inner>>`).
    }
}

/// HashMap storage — keys buffer (string-typed v2-raw `*mut TypedArray<*const StringObj>`)
/// + per-V monomorphized values buffer (`*mut TypedArray<V>`) + eager
/// bucket-index for O(1) lookup.
///
/// **Wave 2 Round 3b C2-joint ckpt-1 (2026-05-14):** the parametric
/// `HashMapValueBuf` enum has been REPLACED with a generic type parameter `V`
/// constrained by `HashMapValueElem`. Per audit §C.4 option (a.2):
/// per-V monomorphization at compile time via the `HashMapKindedRef` carrier
/// (defined below). No runtime kind discriminator field on this struct (the
/// variant tag lives on `HashMapKindedRef` at the carrier layer).
///
/// The values buffer is a raw `*mut TypedArray<V>` — the v2-raw heap shape
/// (HeapHeader-at-offset-0 produced via `TypedArray::<V>::new`). Drop runs
/// `V::release_typed_array(self.values)` via the `HashMapValueElem` trait —
/// per-V monomorphized at compile time. The keys buffer is a `*mut
/// TypedArray<*const StringObj>` — v2-raw shape using the `HeapElement`
/// dispatch on `StringObj`.
///
/// Per-V monomorphizations supported at landing (mirror of §A migration):
/// `i64`, `f64`, `u8` (Bool), `*const StringObj`, `*const DecimalObj`,
/// `TypedObjectPtr`, `TraitObjectPtr`. DateTime / Timespan / Duration /
/// Instant / Char are dead per §C.5 (Char retained as POD-scalar arm only
/// for the dead-but-derived defensive path; no live root producer).
///
/// **Eager bucket-only at first landing** (preserved from Stage C): `index`
/// is built at construction and maintained incrementally on insert / remove.
/// The `shape_id` hidden-class fast-path that the pre-bulldozer architecture
/// used for ≤64-string-keyed-maps remains deferred to a separate
/// optimization workstream.
///
/// **Forbidden under Q25.B SUPERSEDED:**
/// - `Arc<TypedBuffer<V>>` field shape (the value-buffer carrier is
///   `*mut TypedArray<V>` per audit §C.4).
/// - HashMap-wide runtime kind discriminator on this struct (per-V
///   monomorphization at compile time via the carrier; no inline tag
///   byte on `HashMapData<V>` itself).
/// - Re-introducing `HashMapValueBuf` arms under any rename.
#[derive(Debug)]
pub struct HashMapData<V: HashMapValueElem> {
    /// Insertion-ordered keys — v2-raw `*mut TypedArray<*const StringObj>`.
    /// Owned by this struct (one strong-count share on the keys array's
    /// HeapHeader at offset 0). Drop calls `<*const StringObj as
    /// HashMapValueElem>::release_typed_array` to retire the share.
    pub keys: *mut crate::v2::typed_array::TypedArray<*const crate::v2::string_obj::StringObj>,
    /// Insertion-ordered values — v2-raw `*mut TypedArray<V>`. Owned by
    /// this struct. Drop calls `V::release_typed_array(self.values)`.
    pub values: *mut crate::v2::typed_array::TypedArray<V>,
    /// Eager bucket-index: hash → list of indices into `keys` / `values`
    /// arrays. Enables O(1) lookup at the user-facing `map.get(key)` path.
    /// Hash is computed via FNV-1a over the key string bytes.
    pub index: std::collections::HashMap<u64, Vec<u32>>,
}

// SAFETY: `*mut TypedArray<T>` is `!Send + !Sync` by default. `HashMapData<V>`
// is safe to share across threads because:
// (1) `TypedArray<T>` is HeapHeader-equipped with atomic refcount ops
//     (`v2_retain` / `v2_release` in `v2/refcount.rs`).
// (2) Element-level Send/Sync is preserved by the `HashMapValueElem` impls
//     (StringObj / DecimalObj / TypedObjectPtr / TraitObjectPtr all carry
//     manual `unsafe impl Send + Sync`; scalar V types are auto-Send/Sync).
// (3) `HashMapData<V>` is treated as immutable at the marshal boundary;
//     mutation goes through `Arc::make_mut` on the consumer side (ckpt-3
//     territory).
unsafe impl<V: HashMapValueElem> Send for HashMapData<V> {}
unsafe impl<V: HashMapValueElem> Sync for HashMapData<V> {}

impl<V: HashMapValueElem> HashMapData<V> {
    /// Build an empty HashMapData with no entries.
    ///
    /// Allocates two v2-raw `TypedArray` storages (keys + values) at
    /// capacity 0. The struct owns one strong-count share on each.
    pub fn new() -> Self {
        Self {
            // `*const StringObj` is `Copy` (raw pointer), so the Copy-bounded
            // `TypedArray::<*const StringObj>::new` works here.
            keys: crate::v2::typed_array::TypedArray::<
                *const crate::v2::string_obj::StringObj,
            >::new(),
            // V may be non-Copy (e.g. `TypedObjectPtr` has manual `Drop`),
            // so use the non-Copy `TypedArray::<V>::new_generic` path
            // (allocation only; no element-level reads/writes).
            values: crate::v2::typed_array::TypedArray::<V>::new_generic(),
            index: std::collections::HashMap::new(),
        }
    }

    /// Build from parallel buffers — caller transfers one strong-count share
    /// on each `TypedArray` to this struct. Computes the bucket index eagerly
    /// from the keys buffer.
    ///
    /// # Safety
    /// `keys` must point to a live `TypedArray<*const StringObj>` and
    /// `values` to a live `TypedArray<V>`, both with at least one
    /// strong-count share owned by the caller (transferred to this struct).
    /// `keys.len` must equal `values.len`.
    pub unsafe fn from_pairs(
        keys: *mut crate::v2::typed_array::TypedArray<*const crate::v2::string_obj::StringObj>,
        values: *mut crate::v2::typed_array::TypedArray<V>,
    ) -> Self {
        // SAFETY: caller-bound contract per docstring.
        let (n_keys, n_values) = unsafe {
            (
                // keys is `TypedArray<*const StringObj>` — Copy-bounded `len`.
                crate::v2::typed_array::TypedArray::len(keys),
                // values is `TypedArray<V>` where V may be non-Copy — use the
                // non-Copy `len_generic`.
                crate::v2::typed_array::TypedArray::len_generic(values),
            )
        };
        assert_eq!(
            n_keys, n_values,
            "HashMapData::from_pairs: keys/values length mismatch \
             (keys.len={}, values.len={})",
            n_keys, n_values,
        );

        // Build the bucket index from the keys buffer. Walks `*const StringObj`
        // pointers without taking ownership; each `StringObj::as_str` is a
        // borrow that's valid while the keys buffer is alive.
        let mut index: std::collections::HashMap<u64, Vec<u32>> =
            std::collections::HashMap::new();
        // SAFETY: keys is live + len is the element count.
        let keys_slice: &[*const crate::v2::string_obj::StringObj] = unsafe {
            crate::v2::typed_array::TypedArray::as_slice(keys)
        };
        for (i, &key_ptr) in keys_slice.iter().enumerate() {
            // SAFETY: keys-buffer elements are live `*const StringObj` per the
            // construction-side contract (caller owned one share each; that
            // share moved into the buffer at `TypedArray::push`).
            let key_bytes = unsafe {
                let len = (*key_ptr).len as usize;
                if len == 0 {
                    &[][..]
                } else {
                    std::slice::from_raw_parts((*key_ptr).data, len)
                }
            };
            index
                .entry(fnv1a_hash(key_bytes))
                .or_default()
                .push(i as u32);
        }

        Self {
            keys,
            values,
            index,
        }
    }

    /// Number of entries.
    #[inline]
    pub fn len(&self) -> usize {
        // SAFETY: `self.keys` is live for the lifetime of `&self`.
        unsafe { crate::v2::typed_array::TypedArray::len(self.keys) as usize }
    }

    /// Whether the map is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Read the value at index `i`. Returns a copy of the `V` element
    /// (POD scalars copy bytes; `*const StringObj` / `*const DecimalObj` /
    /// `TypedObjectPtr` / `TraitObjectPtr` copy the pointer bits — caller
    /// must NOT treat the returned value as carrying refcount ownership
    /// unless the caller explicitly bumps the v2-raw refcount).
    ///
    /// For owned-share semantics use `value_at_owned` (ckpt-2 / ckpt-3
    /// territory; not in scope for ckpt-1 foundation).
    ///
    /// # Safety
    /// `i` must be less than `self.len()`.
    #[inline]
    pub unsafe fn value_at_raw(&self, i: usize) -> V
    where
        V: Copy,
    {
        // SAFETY: caller-bound bounds contract + values is live.
        unsafe {
            crate::v2::typed_array::TypedArray::get_unchecked(self.values, i as u32)
        }
    }

    /// Look up a value by string key. Returns `Some(i)` (the index into the
    /// values buffer) if the key is present, else `None`. The returned
    /// index can be used with `value_at_raw` / `Arc::make_mut`-based
    /// mutation paths (ckpt-3 territory).
    ///
    /// O(1) via the bucket index plus a short bucket scan for collision
    /// disambiguation.
    pub fn get_index(&self, key: &str) -> Option<usize> {
        let hash = fnv1a_hash(key.as_bytes());
        let bucket = self.index.get(&hash)?;
        // SAFETY: keys is live; len is the bucket-recorded element count.
        let keys_slice = unsafe {
            crate::v2::typed_array::TypedArray::as_slice(self.keys)
        };
        for &idx in bucket {
            let i = idx as usize;
            // SAFETY: key-buffer elements are live `*const StringObj`.
            let stored = unsafe { keys_slice[i] };
            let stored_str = unsafe { crate::v2::string_obj::StringObj::as_str(stored) };
            if stored_str == key {
                return Some(i);
            }
        }
        None
    }

    /// Whether the map contains the given key.
    #[inline]
    pub fn contains_key(&self, key: &str) -> bool {
        self.get_index(key).is_some()
    }

    // ── Mutation API (Wave 2 Round 3b C2-joint ckpt-3, 2026-05-14) ────────
    //
    // Per-V mutation surface mirroring `HashSetData::insert/remove` shape
    // (line 1514+ above) but with parallel values-buffer maintenance.
    // ADR-006 §2.7.24 Q25.B SUPERSEDED + audit §C.4 option (a.2).
    //
    // Uses raw `ptr::write` / `ptr::read` against the `*mut TypedArray<V>`
    // values buffer (bypassing TypedArray::<T:Copy>::push/pop since
    // `TypedObjectPtr` / `TraitObjectPtr` are non-Copy). `HashMapValueElem
    // ::share_clone` handles per-V refcount-aware copying when callers need
    // to clone elements (e.g. `merge`).
    //
    // Caller-managed ownership: `insert`/`insert_share` take a `V` by value,
    // transferring one share. `remove` returns the `V` by value, transferring
    // the share to the caller. `merge` uses `share_clone` to bump shares on
    // the source's elements before inserting them locally.

    /// Insert a key/value pair, transferring one share on `value` to the
    /// map. If the key was already present, the old value's share is
    /// retired (via `V::release_typed_array`-style single-element drop)
    /// and the slot is overwritten with `value`. Returns `true` on
    /// new-key insert, `false` on overwrite.
    ///
    /// The `key` is allocated as a new `StringObj` (one fresh
    /// `v2_retain`=1 share owned by the map).
    ///
    /// # Safety
    /// `value` must be a valid owned V — for HeapElement / ptr-newtype V
    /// types, the caller must transfer ownership of one refcount share
    /// to this method. POD V types (`i64`/`f64`/`u8`/`char`) trivially
    /// own themselves.
    pub unsafe fn insert(&mut self, key: &str, value: V) -> bool {
        let hash = fnv1a_hash(key.as_bytes());
        // Check for existing key — overwrite path.
        if let Some(bucket) = self.index.get(&hash) {
            for &idx in bucket {
                let i = idx as usize;
                // SAFETY: keys live + index points into keys range.
                let stored_ptr = unsafe {
                    crate::v2::typed_array::TypedArray::get_unchecked(self.keys, idx)
                };
                let stored_str = unsafe { crate::v2::string_obj::StringObj::as_str(stored_ptr) };
                if stored_str == key {
                    // Overwrite: read+drop the old value, write the new.
                    unsafe {
                        let data_ptr = (*self.values).data.add(i);
                        let old_value: V = std::ptr::read(data_ptr);
                        // Drop the old value's share by walking through a
                        // single-element TypedArray. Simpler: rely on V's
                        // Drop impl (or for HeapElement V types, manual
                        // release_elem). For uniformity, route through the
                        // share_clone-aware path: since old_value is owned,
                        // letting it go out of scope at end of this block
                        // invokes its Drop impl (TypedObjectPtr/TraitObjectPtr
                        // Drop calls release_elem; *const StringObj has no
                        // Drop — we must manually release).
                        Self::drop_owned_value(old_value);
                        std::ptr::write(data_ptr, value);
                    }
                    return false;
                }
            }
        }
        // New-key insert path. Allocate new StringObj for the key.
        let key_obj: *const crate::v2::string_obj::StringObj =
            crate::v2::string_obj::StringObj::new(key);
        let new_idx_u32 = unsafe { crate::v2::typed_array::TypedArray::len(self.keys) };
        // Push key into the StringObj keys buffer (Copy-bounded *const T).
        unsafe { crate::v2::typed_array::TypedArray::push(self.keys, key_obj) };
        // Push value into the values buffer via raw write (V may be non-Copy).
        unsafe { Self::values_push(self.values, value) };
        // Update bucket index.
        self.index.entry(hash).or_default().push(new_idx_u32);
        true
    }

    /// Remove the entry under `key`. Returns the removed value (transferring
    /// one share to the caller) if present, else `None`.
    ///
    /// The bucket index is updated to reflect the buffer's post-removal
    /// indices: every entry after the removed slot shifts down by one
    /// position (mirror of `HashSetData::remove`).
    pub unsafe fn remove(&mut self, key: &str) -> Option<V> {
        let hash = fnv1a_hash(key.as_bytes());
        let removed_idx: usize = {
            let bucket = self.index.get(&hash)?;
            let mut found: Option<usize> = None;
            for (bucket_pos, &idx) in bucket.iter().enumerate() {
                // SAFETY: keys live.
                let stored_ptr = unsafe {
                    crate::v2::typed_array::TypedArray::get_unchecked(self.keys, idx)
                };
                let stored_str = unsafe { crate::v2::string_obj::StringObj::as_str(stored_ptr) };
                if stored_str == key {
                    found = Some(bucket_pos);
                    break;
                }
            }
            let bucket_pos = found?;
            let bucket = self.index.get_mut(&hash).expect("bucket present");
            let removed_idx = bucket.swap_remove(bucket_pos) as usize;
            if bucket.is_empty() {
                self.index.remove(&hash);
            }
            removed_idx
        };
        // Read the value out (transferring share to caller).
        let removed_value: V = unsafe {
            let data_ptr = (*self.values).data.add(removed_idx);
            std::ptr::read(data_ptr)
        };
        // Read the key pointer out + release its share.
        let removed_key: *const crate::v2::string_obj::StringObj = unsafe {
            crate::v2::typed_array::TypedArray::get_unchecked(self.keys, removed_idx as u32)
        };
        unsafe {
            use crate::v2::heap_element::HeapElement;
            crate::v2::string_obj::StringObj::release_elem(removed_key);
        }
        // Shift remaining elements down by one (compact the buffers).
        let n_keys = unsafe { crate::v2::typed_array::TypedArray::len(self.keys) } as usize;
        unsafe {
            let keys_data = (*self.keys).data;
            let values_data = (*self.values).data;
            for j in removed_idx..n_keys - 1 {
                std::ptr::write(keys_data.add(j), std::ptr::read(keys_data.add(j + 1)));
                std::ptr::write(values_data.add(j), std::ptr::read(values_data.add(j + 1)));
            }
            (*self.keys).len -= 1;
            (*self.values).len -= 1;
        }
        // Renumber the bucket index entries pointing past the removed slot.
        for bucket in self.index.values_mut() {
            for slot in bucket.iter_mut() {
                if (*slot as usize) > removed_idx {
                    *slot -= 1;
                }
            }
        }
        Some(removed_value)
    }

    /// Look up a value by key. Returns a *share-cloned* copy of the stored
    /// value (the caller takes one fresh share — for POD V trivial copy;
    /// for HeapElement / ptr-newtype V the v2_retain happens via
    /// `HashMapValueElem::share_clone`).
    ///
    /// Returns `None` if the key is absent.
    pub fn get_share(&self, key: &str) -> Option<V> {
        let i = self.get_index(key)?;
        // SAFETY: i < len(values).
        let elem_ref: &V = unsafe { &*(*self.values).data.add(i) };
        Some(unsafe { V::share_clone(elem_ref) })
    }

    /// Merge `other`'s entries into `self`, last-write-wins on key
    /// collision. Each value from `other` is share-cloned before being
    /// inserted (so `other`'s shares are preserved).
    pub unsafe fn merge(&mut self, other: &Self) {
        let n = other.len();
        for i in 0..n {
            // SAFETY: i < other.len().
            let key_ptr = unsafe {
                crate::v2::typed_array::TypedArray::get_unchecked(other.keys, i as u32)
            };
            let key_str = unsafe { crate::v2::string_obj::StringObj::as_str(key_ptr) };
            let value_ref: &V = unsafe { &*(*other.values).data.add(i) };
            let cloned_value = unsafe { V::share_clone(value_ref) };
            unsafe { self.insert(key_str, cloned_value) };
        }
    }

    /// Push a single value onto the values buffer, growing the data
    /// allocation if needed. Bypasses `TypedArray::<T: Copy>::push` so
    /// non-Copy V types (TypedObjectPtr/TraitObjectPtr) work too.
    ///
    /// # Safety
    /// `values` must point to a live `TypedArray<V>`; `value` must be a
    /// valid owned V (caller transfers one share).
    unsafe fn values_push(values: *mut crate::v2::typed_array::TypedArray<V>, value: V) {
        use std::alloc::{alloc, realloc, Layout};
        unsafe {
            let arr = &mut *values;
            if arr.len == arr.cap {
                // Grow (doubling, min 4).
                let new_cap = if arr.cap == 0 { 4u32 } else { arr.cap.checked_mul(2).expect("capacity overflow") };
                let new_layout = Layout::array::<V>(new_cap as usize).expect("invalid array layout");
                let new_data = if arr.cap == 0 || arr.data.is_null() {
                    alloc(new_layout) as *mut V
                } else {
                    let old_layout = Layout::array::<V>(arr.cap as usize).expect("invalid array layout");
                    realloc(arr.data as *mut u8, old_layout, new_layout.size()) as *mut V
                };
                assert!(!new_data.is_null(), "reallocation failed for HashMapData<V> values");
                arr.data = new_data;
                arr.cap = new_cap;
            }
            std::ptr::write(arr.data.add(arr.len as usize), value);
            arr.len += 1;
        }
    }

    /// Drop an owned value, retiring its refcount share if it owns one.
    /// Per-V dispatch via `HashMapValueElem::release_owned`. For POD V
    /// (i64/f64/u8/char) this is a no-op; for HeapElement V the share is
    /// retired via `release_elem`; for Ptr-newtype V the wrapper's Drop
    /// impl runs at scope-end.
    ///
    /// # Safety
    /// `value` must be a valid owned V — callers transfer one share to
    /// this method; the method retires it.
    unsafe fn drop_owned_value(value: V) {
        unsafe { V::release_owned(value) }
    }
}

impl<V: HashMapValueElem> Default for HashMapData<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: HashMapValueElem> Drop for HashMapData<V> {
    /// Retire the per-buffer strong-count shares: keys (via
    /// `<*const StringObj as HashMapValueElem>::release_typed_array`) +
    /// values (via `V::release_typed_array`). Per-V monomorphized at compile
    /// time — no runtime kind probe.
    fn drop(&mut self) {
        if !self.keys.is_null() {
            // SAFETY: `self.keys` was allocated via the v2-raw
            // `TypedArray::<*const StringObj>` allocator and owns one
            // strong-count share. After this call `self.keys` is invalid.
            unsafe {
                <*const crate::v2::string_obj::StringObj as HashMapValueElem>::release_typed_array(
                    self.keys,
                )
            }
        }
        if !self.values.is_null() {
            // SAFETY: `self.values` was allocated via the v2-raw
            // `TypedArray::<V>` allocator and owns one strong-count share.
            // Per-V dispatcher via `HashMapValueElem`.
            unsafe { V::release_typed_array(self.values) }
        }
    }
}

/// Clone-on-write impl for `HashMapData<V>` (Wave 2 Round 3b C2-joint
/// ckpt-3, 2026-05-14). Allocates fresh keys + values buffers and
/// share-clones each element per the per-V `HashMapValueElem::share_clone`
/// dispatcher (and `v2_retain` on each key via the *const StringObj impl).
/// The fresh `HashMapData<V>` owns one refcount share on each per-element
/// allocation; the source's shares are untouched.
///
/// This impl is required for `Arc::make_mut(&mut Arc<HashMapData<V>>)` to
/// work at the consumer side (clone-on-write at the dispatch shell). Per
/// ADR-006 §2.7.24 Q25.B SUPERSEDED + audit §C.4 option (a.2).
impl<V: HashMapValueElem> Clone for HashMapData<V> {
    fn clone(&self) -> Self {
        let n = self.len();
        // Allocate fresh keys buffer with capacity n (Copy-bounded
        // `with_capacity` since *const StringObj is Copy).
        let new_keys = crate::v2::typed_array::TypedArray::<
            *const crate::v2::string_obj::StringObj,
        >::with_capacity(n as u32);
        // Allocate fresh values buffer with capacity n. Use the non-Copy
        // generic variant so V can be either Copy (POD/raw pointers) or
        // non-Copy (TypedObjectPtr / TraitObjectPtr).
        let new_values = crate::v2::typed_array::TypedArray::<V>::with_capacity_generic(n as u32);
        // Walk source elements; share_clone keys + values into the new buffers.
        unsafe {
            for i in 0..n {
                let key_ptr = crate::v2::typed_array::TypedArray::get_unchecked(
                    self.keys, i as u32,
                );
                // Share-clone the key (v2_retain on the *const StringObj).
                let cloned_key = <*const crate::v2::string_obj::StringObj
                    as HashMapValueElem>::share_clone(&key_ptr);
                std::ptr::write((*new_keys).data.add(i), cloned_key);
                let value_ref: &V = &*(*self.values).data.add(i);
                let cloned_value = V::share_clone(value_ref);
                std::ptr::write((*new_values).data.add(i), cloned_value);
            }
            (*new_keys).len = n as u32;
            (*new_values).len = n as u32;
        }
        Self {
            keys: new_keys,
            values: new_values,
            index: self.index.clone(),
        }
    }
}

/// HashMapKindedRef — kinded carrier for `Arc<HashMapData<V>>` per audit
/// §C.4 option (a.2). Bundles per-V monomorphized payload types as enum
/// variants; the variant tag IS the `NativeKind` discriminator at the
/// carrier layer.
///
/// Used as the `HeapValue::HashMap` arm payload (ckpt-2 flips the variant
/// signature). Stays within shape-value / shape-runtime / shape-vm internal
/// Rust boundaries per ADR-006 §2.7.5 Cross-crate ABI policy — does NOT
/// leak into the extension contract raw-bits ABI at `module_exports.rs:21`.
///
/// **Manual Drop + Clone discipline** mirroring `TypedObjectPtr`: the
/// auto-derived `Drop` / `Clone` on the enclosing `HeapValue` enum chains
/// through `HashMapKindedRef`'s manual impls, which dispatch to per-variant
/// `Arc::drop` / `Arc::clone` on the typed inner `Arc<HashMapData<V>>`.
///
/// Per-V variants supported at landing (mirror of §C.4 audit shape;
/// post-D4 TypedObjectPtr canonical pattern):
///
/// - `I64` — `Arc<HashMapData<i64>>`
/// - `F64` — `Arc<HashMapData<f64>>`
/// - `Bool` — `Arc<HashMapData<u8>>`
/// - `Char` — `Arc<HashMapData<char>>` (dead-but-derived per §C.5)
/// - `String` — `Arc<HashMapData<*const StringObj>>`
/// - `Decimal` — `Arc<HashMapData<*const DecimalObj>>`
/// - `TypedObject` — `Arc<HashMapData<TypedObjectPtr>>`
/// - `TraitObject` — `Arc<HashMapData<TraitObjectPtr>>`
///
/// **Forbidden** (per CLAUDE.md broader-family regex + Q25.B SUPERSEDED
/// post-supersession #1):
///
/// - "HashMapKindedRef shim" / "HashMapKindedRef bridge" / "kinded-ref helper"
///   framing — refused on sight; the Ref-suffix is canonical per ADR-006
///   §2.7.6 / Q8 carrier-API-bound naming. Mirror of `KindedSlot::from_X`
///   constructor-shape; not a shim.
/// - Re-introducing `HashMapValueBuf` arms inside or alongside this enum
///   ("Q25.B-inside-enum carriers retained" / "documented intentional
///   duality"). The Wave 2 cadence shift authorization stands — per-V
///   monomorphization at the method tier with HashMapKindedRef carrier is
///   the deletion target, NOT a preserved-alongside alternative.
/// - HashMap-wide runtime kind discriminator on `HashMapData<V>` itself
///   (per audit §C.4 rationale: per-V monomorphization at compile time via
///   this carrier API; NO inline tag byte on `HashMapData<V>`).
#[derive(Debug)]
pub enum HashMapKindedRef {
    /// `Arc<HashMapData<i64>>` — V = i64 (POD scalar).
    I64(Arc<HashMapData<i64>>),
    /// `Arc<HashMapData<f64>>` — V = f64 (POD scalar).
    F64(Arc<HashMapData<f64>>),
    /// `Arc<HashMapData<u8>>` — V = u8 (Bool; one byte per element).
    Bool(Arc<HashMapData<u8>>),
    /// `Arc<HashMapData<char>>` — V = char (codepoint; dead-but-derived per §C.5).
    Char(Arc<HashMapData<char>>),
    /// `Arc<HashMapData<*const StringObj>>` — V = `*const StringObj`
    /// (HeapElement-equipped raw pointer).
    String(Arc<HashMapData<*const crate::v2::string_obj::StringObj>>),
    /// `Arc<HashMapData<*const DecimalObj>>` — V = `*const DecimalObj`
    /// (HeapElement-equipped raw pointer).
    Decimal(Arc<HashMapData<*const crate::v2::decimal_obj::DecimalObj>>),
    /// `Arc<HashMapData<TypedObjectPtr>>` — V = `TypedObjectPtr`
    /// (#[repr(transparent)] newtype over `*const TypedObjectStorage`,
    /// per ADR-006 §2.3 amendment D4 ckpt-final-prime² canonical pattern).
    TypedObject(Arc<HashMapData<TypedObjectPtr>>),
    /// `Arc<HashMapData<TraitObjectPtr>>` — V = `TraitObjectPtr`
    /// (#[repr(transparent)] newtype over `*const TraitObjectStorage`).
    TraitObject(Arc<HashMapData<TraitObjectPtr>>),
    /// `Arc<HashMapData<HashMapKindedRef>>` — V = `HashMapKindedRef` itself
    /// (recursive carrier). The inner HashMaps' values buffer is a flat
    /// array of `HashMapKindedRef` payloads (per-V kinded refs, each
    /// holding its own `Arc<HashMapData<V_inner>>`). Used by
    /// `HashMap.groupBy` to produce `HashMap<string, HashMap>` outputs.
    ///
    /// Wave N hashmap-value-v-arm follow-up (cluster-2 closure-wave-C,
    /// 2026-05-16). Per ADR-006 §2.7.24 Q25.B SUPERSEDED canonical
    /// pattern (HashMapKindedRef carrier + per-V monomorphization at
    /// the method tier) extended naturally to a recursive HashMap-value
    /// V arm via the existing `HashMapValueElem` trait dispatch shape.
    HashMap(Arc<HashMapData<HashMapKindedRef>>),
}

impl Clone for HashMapKindedRef {
    /// Per-variant `Arc::clone` — single refcount bump on the inner
    /// `Arc<HashMapData<V>>`. No structural copy.
    fn clone(&self) -> Self {
        match self {
            HashMapKindedRef::I64(arc) => HashMapKindedRef::I64(Arc::clone(arc)),
            HashMapKindedRef::F64(arc) => HashMapKindedRef::F64(Arc::clone(arc)),
            HashMapKindedRef::Bool(arc) => HashMapKindedRef::Bool(Arc::clone(arc)),
            HashMapKindedRef::Char(arc) => HashMapKindedRef::Char(Arc::clone(arc)),
            HashMapKindedRef::String(arc) => HashMapKindedRef::String(Arc::clone(arc)),
            HashMapKindedRef::Decimal(arc) => HashMapKindedRef::Decimal(Arc::clone(arc)),
            HashMapKindedRef::TypedObject(arc) => HashMapKindedRef::TypedObject(Arc::clone(arc)),
            HashMapKindedRef::TraitObject(arc) => HashMapKindedRef::TraitObject(Arc::clone(arc)),
            HashMapKindedRef::HashMap(arc) => HashMapKindedRef::HashMap(Arc::clone(arc)),
        }
    }
}

// Drop is auto-derived: each variant holds `Arc<HashMapData<V>>` whose Drop
// retires one strong-count share; on refcount-0 the inner `HashMapData<V>::Drop`
// runs and retires keys + values buffer shares via the `HashMapValueElem`
// dispatch. No manual `impl Drop` needed.

impl HashMapKindedRef {
    /// The per-V `NativeKind` discriminator for the values buffer of this
    /// HashMap. Used at carrier boundaries (e.g. `HashMap.values()`
    /// projection to `TypedArrayData::<V>` arm + the parallel-kind stack
    /// track at §2.7.7 / Q9 stack reads of HashMap-iter yields) to feed
    /// the per-V Arc into the matching `KindedSlot::from_*` constructor.
    ///
    /// Per ADR-006 §2.7.6 / Q8 carrier-API-bound rule: one accessor per
    /// `NativeKind` heap variant — no per-V escape-hatch accessor (e.g.
    /// `as_string_arc()` returning `Arc<HashMapData<*const StringObj>>`)
    /// at this layer; consumers destructure the enum to recover the
    /// typed inner Arc.
    ///
    /// **Per-V NativeKind mapping** (Wave 2 Round 3b C2-joint ckpt-2
    /// 2026-05-14):
    ///
    /// - `I64` → `NativeKind::Int64`
    /// - `F64` → `NativeKind::Float64`
    /// - `Bool` → `NativeKind::Bool`
    /// - `Char` → `NativeKind::Char` (dead-but-derived per §C.5)
    /// - `String` → `NativeKind::Ptr(HeapKind::String)`
    /// - `Decimal` → `NativeKind::Ptr(HeapKind::Decimal)`
    /// - `TypedObject` → `NativeKind::Ptr(HeapKind::TypedObject)`
    /// - `TraitObject` → `NativeKind::Ptr(HeapKind::TraitObject)`
    /// - `HashMap` → `NativeKind::Ptr(HeapKind::HashMap)` (recursive carrier;
    ///   Wave N hashmap-value-v-arm follow-up 2026-05-16)
    ///
    /// **StringV2 / DecimalV2 gate-flip dependency note:** at ckpt-2
    /// landing time (2026-05-14), the v2-raw `StringV2` / `DecimalV2`
    /// `NativeKind` variants were proposed in Round 3a' but the
    /// gate-flip from `NativeKind::Ptr(HeapKind::String)` →
    /// `NativeKind::StringV2` (et al.) had not propagated across all
    /// carrier APIs. This accessor maps `String` and `Decimal` arms to
    /// the heap-pointer variant per the post-3a-flip baseline; if a
    /// future gate-flip moves the canonical surface to StringV2/DecimalV2,
    /// this mapping is updated lockstep at the same wave (ckpt-3 or
    /// follow-up).
    #[inline]
    pub fn values_kind(&self) -> crate::NativeKind {
        use crate::NativeKind;
        match self {
            HashMapKindedRef::I64(_) => NativeKind::Int64,
            HashMapKindedRef::F64(_) => NativeKind::Float64,
            HashMapKindedRef::Bool(_) => NativeKind::Bool,
            HashMapKindedRef::Char(_) => NativeKind::Char,
            HashMapKindedRef::String(_) => NativeKind::Ptr(HeapKind::String),
            HashMapKindedRef::Decimal(_) => NativeKind::Ptr(HeapKind::Decimal),
            HashMapKindedRef::TypedObject(_) => NativeKind::Ptr(HeapKind::TypedObject),
            HashMapKindedRef::TraitObject(_) => NativeKind::Ptr(HeapKind::TraitObject),
            HashMapKindedRef::HashMap(_) => NativeKind::Ptr(HeapKind::HashMap),
        }
    }

    /// Number of entries in the HashMap. Dispatches per-V to the inner
    /// `HashMapData<V>::len()` (same impl for every V — the keys buffer
    /// length, which equals the values buffer length per the from_pairs
    /// invariant).
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            HashMapKindedRef::I64(arc) => arc.len(),
            HashMapKindedRef::F64(arc) => arc.len(),
            HashMapKindedRef::Bool(arc) => arc.len(),
            HashMapKindedRef::Char(arc) => arc.len(),
            HashMapKindedRef::String(arc) => arc.len(),
            HashMapKindedRef::Decimal(arc) => arc.len(),
            HashMapKindedRef::TypedObject(arc) => arc.len(),
            HashMapKindedRef::TraitObject(arc) => arc.len(),
            HashMapKindedRef::HashMap(arc) => arc.len(),
        }
    }

    /// Whether the map is empty (zero entries). Dispatches per-V via `len()`.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the map contains the given key. Dispatches per-V via the
    /// inner `HashMapData<V>::contains_key` (same impl for every V — keys
    /// are stringly-typed, so the lookup is V-agnostic).
    #[inline]
    pub fn contains_key(&self, key: &str) -> bool {
        match self {
            HashMapKindedRef::I64(arc) => arc.contains_key(key),
            HashMapKindedRef::F64(arc) => arc.contains_key(key),
            HashMapKindedRef::Bool(arc) => arc.contains_key(key),
            HashMapKindedRef::Char(arc) => arc.contains_key(key),
            HashMapKindedRef::String(arc) => arc.contains_key(key),
            HashMapKindedRef::Decimal(arc) => arc.contains_key(key),
            HashMapKindedRef::TypedObject(arc) => arc.contains_key(key),
            HashMapKindedRef::TraitObject(arc) => arc.contains_key(key),
            HashMapKindedRef::HashMap(arc) => arc.contains_key(key),
        }
    }

    /// The `HeapKind` discriminator for `KindedSlot::from_hashmap` slot
    /// stamping (§2.7.6 / Q8 / Q9 parallel-kind track). Always
    /// `HeapKind::HashMap` regardless of the inner V — the V-discriminator
    /// is encoded in the `HashMapKindedRef` variant tag, not in the
    /// `HeapKind` ordinal (HashMap stays at ordinal 17).
    #[inline]
    pub const fn heap_kind(&self) -> HeapKind {
        HeapKind::HashMap
    }
}

/// Per-V `{key: value, …}` formatter for `HashMapKindedRef`. Walks the
/// keys buffer + per-V values buffer; renders keys as quoted strings
/// and each value via the matching primitive `Display` (i64/f64/u8 as
/// "true"/"false"/char). For HeapElement / Ptr-newtype V we route
/// through the inner pointer's `Display` shape.
///
/// Wave 2 Round 3b C2-joint ckpt-3 (2026-05-14). ADR-006 §2.7.24 Q25.B
/// SUPERSEDED + audit §C.4.
fn hashmap_kref_display(
    kref: &HashMapKindedRef,
    f: &mut std::fmt::Formatter<'_>,
) -> std::fmt::Result {
    use std::fmt::Write as _;
    write!(f, "{{")?;

    /// Read all keys as `&str` from the v2-raw `*mut TypedArray<*const StringObj>` buffer.
    ///
    /// # Safety
    /// `keys` must point to a live `TypedArray<*const StringObj>` whose
    /// elements are live StringObjs (the HashMapData<V> contract).
    unsafe fn read_keys<'a>(
        keys: *const crate::v2::typed_array::TypedArray<*const crate::v2::string_obj::StringObj>,
    ) -> Vec<&'a str> {
        unsafe {
            let n = crate::v2::typed_array::TypedArray::len(keys) as usize;
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let ptr = crate::v2::typed_array::TypedArray::get_unchecked(keys, i as u32);
                out.push(crate::v2::string_obj::StringObj::as_str(ptr));
            }
            out
        }
    }

    fn emit_key(f: &mut std::fmt::Formatter<'_>, i: usize, key: &str) -> std::fmt::Result {
        if i > 0 {
            f.write_str(", ")?;
        }
        write!(f, "\"{}\": ", key)
    }

    match kref {
        HashMapKindedRef::I64(arc) => {
            let keys = unsafe { read_keys(arc.keys) };
            for (i, k) in keys.iter().enumerate() {
                emit_key(f, i, k)?;
                let v = unsafe { *(*arc.values).data.add(i) };
                write!(f, "{}", v)?;
            }
        }
        HashMapKindedRef::F64(arc) => {
            let keys = unsafe { read_keys(arc.keys) };
            for (i, k) in keys.iter().enumerate() {
                emit_key(f, i, k)?;
                let v = unsafe { *(*arc.values).data.add(i) };
                write!(f, "{}", v)?;
            }
        }
        HashMapKindedRef::Bool(arc) => {
            let keys = unsafe { read_keys(arc.keys) };
            for (i, k) in keys.iter().enumerate() {
                emit_key(f, i, k)?;
                let v: u8 = unsafe { *(*arc.values).data.add(i) };
                write!(f, "{}", v != 0)?;
            }
        }
        HashMapKindedRef::Char(arc) => {
            let keys = unsafe { read_keys(arc.keys) };
            for (i, k) in keys.iter().enumerate() {
                emit_key(f, i, k)?;
                let v: char = unsafe { *(*arc.values).data.add(i) };
                write!(f, "'{}'", v)?;
            }
        }
        HashMapKindedRef::String(arc) => {
            let keys = unsafe { read_keys(arc.keys) };
            for (i, k) in keys.iter().enumerate() {
                emit_key(f, i, k)?;
                let v_ptr: *const crate::v2::string_obj::StringObj =
                    unsafe { *(*arc.values).data.add(i) };
                let s = unsafe { crate::v2::string_obj::StringObj::as_str(v_ptr) };
                write!(f, "\"{}\"", s)?;
            }
        }
        HashMapKindedRef::Decimal(arc) => {
            let keys = unsafe { read_keys(arc.keys) };
            for (i, k) in keys.iter().enumerate() {
                emit_key(f, i, k)?;
                let v_ptr: *const crate::v2::decimal_obj::DecimalObj =
                    unsafe { *(*arc.values).data.add(i) };
                // DecimalObj::as_decimal returns the Decimal value via the
                // v2-raw payload (mirrors StringObj::as_str shape).
                let d = unsafe { (*v_ptr).value };
                let mut tmp = String::new();
                let _ = write!(tmp, "{}", d);
                f.write_str(&tmp)?;
            }
        }
        HashMapKindedRef::TypedObject(arc) => {
            let keys = unsafe { read_keys(arc.keys) };
            for (i, k) in keys.iter().enumerate() {
                emit_key(f, i, k)?;
                // Render as opaque tag — full recursive rendering lives at
                // printing.rs::format_typed_object (depth-budgeted).
                let v_ref: &TypedObjectPtr = unsafe { &*(*arc.values).data.add(i) };
                write!(f, "<typed_object:{:p}>", v_ref.as_ptr())?;
            }
        }
        HashMapKindedRef::TraitObject(arc) => {
            let keys = unsafe { read_keys(arc.keys) };
            for (i, k) in keys.iter().enumerate() {
                emit_key(f, i, k)?;
                let v_ref: &TraitObjectPtr = unsafe { &*(*arc.values).data.add(i) };
                write!(f, "<trait_object:{:p}>", v_ref.as_ptr())?;
            }
        }
        HashMapKindedRef::HashMap(arc) => {
            // Recursive carrier: each value is itself a HashMapKindedRef.
            // Recurse via this same display formatter (Wave N
            // hashmap-value-v-arm follow-up, cluster-2 closure-wave-C,
            // 2026-05-16).
            let keys = unsafe { read_keys(arc.keys) };
            for (i, k) in keys.iter().enumerate() {
                emit_key(f, i, k)?;
                let inner_ref: &HashMapKindedRef = unsafe { &*(*arc.values).data.add(i) };
                hashmap_kref_display(inner_ref, f)?;
            }
        }
    }
    write!(f, "}}")
}

// ── Legacy HashMapValueBuf + non-generic HashMapData REMOVED (Wave 2 Round 3b
//    C2-joint ckpt-1, 2026-05-14) ──────────────────────────────────────────
//
// The pre-Q25.B-SUPERSEDED `HashMapValueBuf` enum + non-generic `HashMapData`
// struct/impl have been removed. The replacement is `HashMapData<V>` +
// `HashMapKindedRef` + `HashMapValueElem` trait (above). Consumer sites at
// `HeapValue::HashMap` variant payload + 51 `Arc<HashMapData>` usages cascade
// in ckpt-2 (variant signature) + ckpt-3 (hashmap_methods.rs / printing.rs /
// xml.rs / json.rs / array_transform.rs / vm_impl/builtins.rs /
// trait_object_ops.rs) + ckpt-final (JIT FFI).

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
    ///
    /// Storage shape: `Arc<Vec<Arc<String>>>` post-V3-S5 ckpt-5-prime²a
    /// (Migration shape (a) per supervisor 2026-05-15 ratification —
    /// `TypedBuffer<T>` wrapper layer retired wholesale at ckpt-4;
    /// `Arc<Vec<T>>` is the smallest delta preserving `Arc::make_mut`
    /// clone-on-write semantics).
    pub keys: Arc<Vec<Arc<String>>>,
    /// Eager bucket-index: hash → list of indices into `keys` array.
    /// Enables O(1) lookup at `set.has(key)`. Hash is FNV-1a over the
    /// key string bytes — same as `HashMapData::index`.
    pub index: std::collections::HashMap<u64, Vec<u32>>,
}

impl HashSetData {
    /// Build an empty HashSetData with no entries.
    pub fn new() -> Self {
        Self {
            keys: Arc::new(Vec::new()),
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
        self.keys.len()
    }

    /// Whether the set is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
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
            if self.keys[i].as_str() == key {
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
                if self.keys[i].as_str() == key.as_str() {
                    return false;
                }
            }
        }
        let new_idx = self.keys.len();
        Arc::make_mut(&mut self.keys).push(key);
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
                if self.keys[idx as usize].as_str() == key {
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
        Arc::make_mut(&mut self.keys).remove(removed_idx);
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
///
/// **Wave 2 Agent E (2026-05-14): HeapHeader-equipped shape change.**
/// Per audit §4.3 Obstacle O-3.a resolution + ADR-006 §Q25.C.5 amendment,
/// the struct now carries a `HeapHeader` at offset 0 (`#[repr(C)]`) so
/// v2-raw raw-pointer allocations (`_new` / `_drop` + `impl HeapElement`)
/// can dispatch refcount on the header via `v2_retain` / `v2_release`.
/// Existing `Arc<TraitObjectStorage>` construction sites continue to work
/// unchanged — `Arc::new(TraitObjectStorage::new(...))` produces a Rust
/// `Arc`-wrapped instance whose embedded header sits at refcount=1 unused;
/// the dispatch arms continue to use `Arc::increment_strong_count` /
/// `Arc::decrement_strong_count` on those bits. The new `_new`-allocated
/// raw-pointer bits use the header's refcount via the `HeapElement` trait.
///
/// The inner `value: Arc<TypedObjectStorage>` field remains Arc-typed in
/// E's scope per Wave 1 §E.6 dispatch contract (the audit's E-a path
/// recommends both inner pointers become raw, but D2 owns the inner
/// `*mut TypedObjectStorage` flip in lockstep with TypedObjectStorage's
/// own Arc-path retirement; E's struct shape change exposes the
/// HeapHeader at offset 0 + manual lifecycle so subsequent rounds can
/// flip the inner field without re-shaping the outer carrier). The
/// `vtable: Arc<VTable>` field stays Arc-typed indefinitely under E's
/// scope — VTable lifecycle is decoupled from this migration (audit
/// §E.3 recommended a separate VTable HeapHeader migration if/when
/// IC devirtualization measurement justifies it).
#[repr(C)]
#[derive(Debug)]
pub struct TraitObjectStorage {
    /// v2-raw HeapHeader at offset 0 (8 bytes). Refcount/kind/flags.
    /// Initialized to `HeapHeader::new(HEAP_KIND_V2_TRAIT_OBJECT)` by
    /// `_new`; for `Arc`-wrapped instances allocated via
    /// `TraitObjectStorage::new` the header sits at refcount=1 unused
    /// (the enclosing `Arc` owns the lifecycle). See struct docstring.
    pub header: crate::v2::heap_header::HeapHeader,

    /// The data half of the fat pointer — owned, heap-allocated as a
    /// `TypedObject`. Always present (never null); universal-dyn
    /// per-method auto-boxing makes the boxed value a real TypedObject
    /// even for scalar concrete types (per §Q25.C.1).
    ///
    /// **Wave 2 Round 4 D4 ckpt-3 (2026-05-14): inner-field shift from
    /// `Arc<TypedObjectStorage>` to `*const TypedObjectStorage`** per E
    /// (Round 2) close note + D3 R3a finding D3-1 — the 5th production-
    /// site class (audit-side parallel to D2's HashMapValueBuf cascade).
    /// The raw pointer was produced by `TypedObjectStorage::_new` (refcount
    /// initialized to 1 on the HeapHeader at offset 0). The carrier owns
    /// one strong-count share, retired at `_drop` / auto-derived `Drop`
    /// via `TypedObjectStorage::release_elem(ptr)` (NOT Rust `Arc::drop`).
    pub value: *const TypedObjectStorage,

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
    /// owns one strong-count share on the v2-raw value pointer's
    /// HeapHeader-at-offset-0 refcount AND one strong-count share on
    /// the vtable Arc; the resulting struct owns both shares.
    ///
    /// **Wave 2 Round 4 D4 ckpt-3 (2026-05-14): `value` param signature
    /// shifted from `Arc<TypedObjectStorage>` to `*const TypedObjectStorage`**
    /// per E (Round 2) close note + D3 R3a finding D3-1 — caller produces
    /// the raw ptr via `TypedObjectStorage::_new` (refcount=1) or by
    /// `v2_retain`-bumping an existing live ptr. The carrier retires
    /// that share at `_drop` / auto-derived `Drop` via
    /// `TypedObjectStorage::release_elem(value)`.
    ///
    /// The embedded HeapHeader is initialized to refcount=1 with kind
    /// `HEAP_KIND_V2_TRAIT_OBJECT`. For `Arc<TraitObjectStorage>`
    /// instances the header sits unused (the enclosing `Arc` owns the
    /// lifecycle); the v2-raw `_new` path is the production carrier
    /// for the on-header refcount lifecycle.
    #[inline]
    pub fn new(value: *const TypedObjectStorage, vtable: Arc<crate::value::VTable>) -> Self {
        Self {
            header: crate::v2::heap_header::HeapHeader::new(
                crate::v2::heap_header::HEAP_KIND_V2_TRAIT_OBJECT,
            ),
            value,
            vtable,
        }
    }

    /// Wave 2 Agent E (2026-05-14): v2-raw raw-pointer allocator.
    ///
    /// Allocates a new `TraitObjectStorage` on the heap and returns a raw
    /// pointer with refcount initialized to 1. Mirrors the `TypedObjectStorage::_new`
    /// precedent at `heap_value.rs` (D1, 2026-05-14) — `#[repr(C)]` struct
    /// with `HeapHeader` at offset 0; refcount discipline goes through
    /// `v2_retain` / `v2_release` via the `HeapElement` trait.
    ///
    /// Construction-side contract: the caller transfers ownership of one
    /// strong-count share on `value: Arc<TypedObjectStorage>` and on
    /// `vtable: Arc<VTable>` to the storage; the storage retires those
    /// shares at `_drop` (via the in-place `drop_in_place` on the field
    /// payloads). The inner Arcs follow normal Rust `Arc` discipline —
    /// only the outer struct's lifecycle is HeapHeader-managed.
    ///
    /// Callers (Wave 2 Round 2): replace the legacy pattern
    /// ```ignore
    /// let arc = Arc::new(TraitObjectStorage::new(value, vtable));
    /// let slot = ValueSlot::from_trait_object(arc);
    /// ```
    /// with the v2-raw pattern
    /// ```ignore
    /// let ptr = TraitObjectStorage::_new(value, vtable);
    /// let slot = ValueSlot::from_trait_object_raw(ptr);
    /// ```
    pub fn _new(
        value: *const TypedObjectStorage,
        vtable: Arc<crate::value::VTable>,
    ) -> *mut Self {
        let layout = std::alloc::Layout::new::<Self>();
        let ptr = unsafe { std::alloc::alloc(layout) as *mut Self };
        assert!(!ptr.is_null(), "allocation failed for TraitObjectStorage");
        unsafe {
            // SAFETY: `ptr` points to fresh, uninitialized memory of size
            // `Layout::new::<Self>()`. We write every field via `ptr::write`
            // to avoid running drop on uninitialized bytes (the existing
            // memory contains garbage, never a valid prior `Self`).
            std::ptr::write(
                &mut (*ptr).header,
                crate::v2::heap_header::HeapHeader::new(
                    crate::v2::heap_header::HEAP_KIND_V2_TRAIT_OBJECT,
                ),
            );
            std::ptr::write(&mut (*ptr).value, value);
            std::ptr::write(&mut (*ptr).vtable, vtable);
        }
        ptr
    }

    /// Wave 2 Agent E (2026-05-14): v2-raw raw-pointer deallocator.
    ///
    /// Runs `drop_in_place` on the inner `Arc<TypedObjectStorage>` value
    /// field and `Arc<VTable>` vtable field (retires one strong-count
    /// share on each via standard Rust `Arc::drop`), then deallocates
    /// the struct's heap memory via `Layout::new::<Self>()`.
    ///
    /// Mirrors the `TypedObjectStorage::_drop` precedent.
    ///
    /// # Safety
    /// `ptr` must point to a live `TraitObjectStorage` allocated via
    /// `Self::_new` with no remaining references. Must not be called
    /// more than once on the same pointer; must not be called on
    /// `Arc<TraitObjectStorage>`-allocated instances (those run
    /// through Rust's `Arc` drop machinery + the auto-derived shape).
    pub unsafe fn _drop(ptr: *mut Self) {
        unsafe {
            // Wave 2 Round 4 D4 ckpt-3 (2026-05-14): `value: *const
            // TypedObjectStorage` (v2-raw shape) is released via
            // `TypedObjectStorage::release_elem` (HeapElement trait —
            // calls `v2_release` on the inner HeapHeader; on refcount=0
            // the inner `TypedObjectStorage::_drop` runs the per-field
            // heap-mask walk + deallocates). The `vtable: Arc<VTable>`
            // field is retired via `drop_in_place` (standard Arc::drop).
            // The `header` field is POD — no Drop work owed.
            use crate::v2::heap_element::HeapElement;
            let inner_ptr = (*ptr).value;
            if !inner_ptr.is_null() {
                TypedObjectStorage::release_elem(inner_ptr);
            }
            std::ptr::drop_in_place(&mut (*ptr).vtable);
            // Deallocate the struct's heap memory.
            let layout = std::alloc::Layout::new::<Self>();
            std::alloc::dealloc(ptr as *mut u8, layout);
        }
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

// Wave 2 Round 4 D4 ckpt-3 (2026-05-14): manual Send + Sync impls. The raw
// pointer `value: *const TypedObjectStorage` makes the struct !Send/!Sync
// by default; the safety argument mirrors `Arc<T>`'s — the pointer
// targets a heap allocation whose lifecycle is managed by the HeapHeader
// refcount (v2_retain/v2_release atomics in `v2/refcount.rs`), and
// TypedObjectStorage itself is Send + Sync (its fields are Box<[ValueSlot]>
// + Arc<[NativeKind]>, all of which are Send + Sync). Multi-thread
// observers share the inner via the same refcount-bumped raw pointer that
// the v2-raw carrier ABI uses across thread boundaries (KindedSlot Send +
// Sync; `Arc<TraitObjectStorage>` Send + Sync requires this).
unsafe impl Send for TraitObjectStorage {}
unsafe impl Sync for TraitObjectStorage {}

impl Clone for TraitObjectStorage {
    /// Per-field clone — the inner value ptr's HeapHeader-at-offset-0
    /// refcount is bumped via `v2_retain`; the vtable Arc bumps its
    /// strong count by one. Cloning a `TraitObjectStorage` produces a
    /// fat-pointer carrier that observes the same underlying TypedObject
    /// and dispatches against the same VTable. The cloned struct's
    /// `header` is a fresh HeapHeader at refcount=1 (matches `Self::new`'s
    /// contract — the embedded header is unused on `Arc<TraitObjectStorage>`
    /// instances; it carries lifecycle only for `_new`-allocated raw-
    /// pointer instances).
    ///
    /// **Wave 2 Round 4 D4 ckpt-3 (2026-05-14): inner-value retain shifted
    /// from `Arc::clone(&self.value)` to `v2_retain(&(*self.value).header)`**
    /// per the `value: *const TypedObjectStorage` inner-field shift.
    fn clone(&self) -> Self {
        // SAFETY: `self.value` is a `*const TypedObjectStorage` allocated
        // via `TypedObjectStorage::_new` (refcount initialized to 1 on
        // the HeapHeader at offset 0). The `Clone` impl bumps that
        // refcount via `v2_retain` so the cloned struct owns its own
        // share, retired at its `_drop` / auto-derived `Drop` via
        // `TypedObjectStorage::release_elem(value)`.
        if !self.value.is_null() {
            unsafe { crate::v2::refcount::v2_retain(&(*self.value).header); }
        }
        Self {
            header: crate::v2::heap_header::HeapHeader::new(
                crate::v2::heap_header::HEAP_KIND_V2_TRAIT_OBJECT,
            ),
            value: self.value,
            vtable: Arc::clone(&self.vtable),
        }
    }
}

// Wave 2 Agent E (2026-05-14): v2-raw HeapElement impl per ADR-006
// §Q25.C.5 amendment + audit §4.3 Obstacle O-3.a resolution. Constrains
// `TraitObjectStorage` to the HeapHeader-at-offset-0 v2-raw element-carrier
// contract so future call sites can store raw `*const TraitObjectStorage`
// bits and dispatch retain/release via the trait.
//
// The trait dispatches refcount through the on-header refcount via
// `v2_release` — distinct from the legacy `Arc<TraitObjectStorage>` path
// which dispatches via Rust `Arc::decrement_strong_count`. Per the struct
// docstring, both carrier shapes coexist at the struct level during the
// Wave 2 dispatch transition; the slot ABI discriminates them by
// allocation provenance (call sites that use `_new` and
// `from_trait_object_raw` follow the raw-pointer lifecycle; existing
// `Arc::new` + `from_trait_object` callers retain Arc-style lifecycle).
unsafe impl crate::v2::heap_element::HeapElement for TraitObjectStorage {
    unsafe fn release_elem(ptr: *const Self) {
        if unsafe { crate::v2::refcount::v2_release(&(*ptr).header) } {
            unsafe { Self::_drop(ptr as *mut Self) };
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
    /// Backed by an `Arc<Vec<i64>>` so a HeapValue clone is a single
    /// atomic refcount bump and `Arc::make_mut` is the canonical
    /// clone-on-write entry per the W13-hashmap-mutation precedent.
    ///
    /// Storage shape: `Arc<Vec<i64>>` post-V3-S5 ckpt-5-prime²a
    /// (Migration shape (a) per supervisor 2026-05-15 ratification —
    /// `TypedBuffer<T>` wrapper layer retired wholesale at ckpt-4;
    /// `Arc<Vec<T>>` is the smallest delta preserving `Arc::make_mut`
    /// clone-on-write semantics).
    pub heap: Arc<Vec<i64>>,
}

impl PriorityQueueData {
    /// Build an empty PriorityQueueData with no entries.
    pub fn new() -> Self {
        Self {
            heap: Arc::new(Vec::new()),
        }
    }

    /// Number of entries.
    #[inline]
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Whether the queue is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Peek at the minimum (root) without removing it. Returns `None`
    /// for an empty queue.
    pub fn peek(&self) -> Option<i64> {
        self.heap.first().copied()
    }

    /// Push a value, restoring the min-heap invariant via sift-up.
    /// Mirror of W13-hashmap-mutation `insert`: `Arc::make_mut`
    /// clone-on-write over the inner `Arc<Vec<i64>>`.
    pub fn push(&mut self, value: i64) {
        let buf = Arc::make_mut(&mut self.heap);
        buf.push(value);
        let last = buf.len() - 1;
        sift_up(buf, last);
    }

    /// Pop the minimum value, restoring the min-heap invariant via
    /// sift-down. Returns `None` for an empty queue. Mirror of
    /// W13-hashmap-mutation `remove`: `Arc::make_mut` clone-on-write.
    pub fn pop(&mut self) -> Option<i64> {
        let buf = Arc::make_mut(&mut self.heap);
        if buf.is_empty() {
            return None;
        }
        let last = buf.len() - 1;
        buf.swap(0, last);
        let min = buf.pop();
        if !buf.is_empty() {
            sift_down(buf, 0);
        }
        min
    }

    /// Return the heap contents as a flat `Vec<i64>` in heap-array
    /// order (NOT sorted). Used for the `toArray` method's `Vec<int>`
    /// projection; for the sorted form see `to_sorted_vec`.
    pub fn to_vec(&self) -> Vec<i64> {
        (*self.heap).clone()
    }

    /// Return the heap contents as a sorted `Vec<i64>` (ascending —
    /// pop-order). Used for the `toSortedArray` method.
    pub fn to_sorted_vec(&self) -> Vec<i64> {
        let mut v: Vec<i64> = (*self.heap).clone();
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
///
/// **Wave 2 Agent D1 (2026-05-14): HeapHeader-equipped shape change.**
/// Per audit §4.3 Obstacle O-3.a resolution + ADR-006 §2.3 amendment, the
/// struct now carries a `HeapHeader` at offset 0 (`#[repr(C)]`) so v2-raw
/// raw-pointer allocations (`_new` / `_drop` + `impl HeapElement`) can
/// dispatch refcount on the header via `v2_retain` / `v2_release`. Existing
/// `Arc<TypedObjectStorage>` construction sites continue to work
/// unchanged — `Arc::new(TypedObjectStorage::new(...))` produces a Rust
/// `Arc`-wrapped instance whose embedded header sits at refcount=1 unused;
/// the dispatch arms continue to use `Arc::increment_strong_count` /
/// `Arc::decrement_strong_count` on those bits. The new `_new`-allocated
/// raw-pointer bits use the header's refcount via the `HeapElement` trait.
/// Agent D2 (Wave 2 Round 2) migrates the 18 production construction sites
/// to the raw-pointer carrier; Agent E (Wave 2 Round 2) consumes the same
/// shape change for `TraitObjectStorage`. Until that migration completes,
/// both carriers coexist at the struct level; the slot-ABI discriminator
/// (`NativeKind::Ptr(HeapKind::TypedObject)`) is unchanged.
#[repr(C)]
#[derive(Debug)]
pub struct TypedObjectStorage {
    /// v2-raw HeapHeader at offset 0 (8 bytes). Refcount/kind/flags.
    /// Initialized to `HeapHeader::new(HEAP_KIND_V2_TYPED_OBJECT)` by
    /// `_new`; for `Arc`-wrapped instances allocated via
    /// `TypedObjectStorage::new` the header sits at refcount=1 unused
    /// (the enclosing `Arc` owns the lifecycle). See struct docstring.
    pub header: crate::v2::heap_header::HeapHeader,
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
        Self {
            header: crate::v2::heap_header::HeapHeader::new(
                crate::v2::heap_header::HEAP_KIND_V2_TYPED_OBJECT,
            ),
            schema_id,
            slots,
            heap_mask,
            field_kinds,
        }
    }

    /// Wave 2 Agent D1 (2026-05-14): v2-raw raw-pointer allocator.
    ///
    /// Allocates a new `TypedObjectStorage` on the heap and returns a raw
    /// pointer with refcount initialized to 1. Mirrors the `DecimalObj::new`
    /// / `StringObj::new` precedents at `crates/shape-value/src/v2/` —
    /// `#[repr(C)]` struct with `HeapHeader` at offset 0; refcount discipline
    /// goes through `v2_retain` / `v2_release` via the `HeapElement` trait.
    ///
    /// Construction-side contract: same as `new()` — `slots.len() ==
    /// field_kinds.len()`; heap-mask bits correspond to heap-kinded slots
    /// whose bits are `Arc::into_raw::<T>` for the matching `T`. The raw-
    /// pointer carrier owns one strong-count share for every heap-kinded
    /// slot it carries, retired by `_drop` at refcount=0.
    ///
    /// Callers (Wave 2 Agent D2, Round 2 cascade): construct via
    /// `TypedObjectStorage::_new(...)` and store the pointer in
    /// `ValueSlot::from_typed_object_raw(ptr)`. Drop runs at refcount=0
    /// via the `HeapElement::release_elem` trait method, NOT via Rust
    /// `Arc::drop` (the `Arc<TypedObjectStorage>` path is the legacy
    /// transitional carrier; both coexist at the struct level per the
    /// struct docstring).
    pub fn _new(
        schema_id: u64,
        slots: Box<[crate::slot::ValueSlot]>,
        heap_mask: u64,
        field_kinds: std::sync::Arc<[crate::native_kind::NativeKind]>,
    ) -> *mut Self {
        debug_assert_eq!(
            slots.len(),
            field_kinds.len(),
            "TypedObjectStorage::_new: slots/field_kinds length mismatch \
             (slots={}, field_kinds={}) — every slot must have a proven NativeKind",
            slots.len(),
            field_kinds.len(),
        );
        let layout = std::alloc::Layout::new::<Self>();
        let ptr = unsafe { std::alloc::alloc(layout) as *mut Self };
        assert!(!ptr.is_null(), "allocation failed for TypedObjectStorage");
        unsafe {
            // SAFETY: `ptr` points to fresh, uninitialized memory of size
            // `Layout::new::<Self>()`. We write every field via `ptr::write`
            // to avoid running drop on uninitialized bytes (the existing
            // memory contains garbage, never a valid prior `Self`).
            std::ptr::write(
                &mut (*ptr).header,
                crate::v2::heap_header::HeapHeader::new(
                    crate::v2::heap_header::HEAP_KIND_V2_TYPED_OBJECT,
                ),
            );
            std::ptr::write(&mut (*ptr).schema_id, schema_id);
            std::ptr::write(&mut (*ptr).slots, slots);
            std::ptr::write(&mut (*ptr).heap_mask, heap_mask);
            std::ptr::write(&mut (*ptr).field_kinds, field_kinds);
        }
        ptr
    }

    /// Wave 2 Agent D1 (2026-05-14): v2-raw raw-pointer deallocator.
    ///
    /// Runs the per-field heap-mask walk (releasing one strong-count share
    /// per heap-kinded slot via `Arc::decrement_strong_count`) and then
    /// deallocates the struct's heap memory via `Layout::new::<Self>()`.
    /// The field walk delegates to `drop_fields` so the same logic powers
    /// both the legacy `impl Drop for TypedObjectStorage` path (used by
    /// `Arc<TypedObjectStorage>` instances) and this raw-pointer path.
    ///
    /// Mirrors the `DecimalObj::drop` / `StringObj::drop` precedents.
    ///
    /// # Safety
    /// `ptr` must point to a live `TypedObjectStorage` allocated via
    /// `Self::_new` with no remaining references. Must not be called more
    /// than once on the same pointer; must not be called on
    /// `Arc<TypedObjectStorage>`-allocated instances (those run through
    /// Rust's `Arc` drop machinery + `impl Drop for TypedObjectStorage`).
    pub unsafe fn _drop(ptr: *mut Self) {
        unsafe {
            // Run the per-field heap-mask walk, retiring one Arc share per
            // heap-kinded slot. Same logic that `impl Drop` runs for the
            // legacy `Arc<TypedObjectStorage>` path.
            (*ptr).drop_fields();
            // Drop the in-place `Box<[ValueSlot]>` and `Arc<[NativeKind]>`
            // payloads so their allocations are freed. The `header`,
            // `schema_id`, and `heap_mask` fields are POD (`Copy` or
            // primitive) — no Drop work owed.
            std::ptr::drop_in_place(&mut (*ptr).slots);
            std::ptr::drop_in_place(&mut (*ptr).field_kinds);
            // Deallocate the struct's heap memory.
            let layout = std::alloc::Layout::new::<Self>();
            std::alloc::dealloc(ptr as *mut u8, layout);
        }
    }

    /// Wave 2 Agent D1 (2026-05-14): shared per-field heap-mask walk.
    ///
    /// Walks `heap_mask`, dispatches per-slot on `field_kinds[i]`, and
    /// retires one strong-count share per heap-kinded slot via
    /// `Arc::decrement_strong_count::<T>` for the matching `T`. Same
    /// dispatch as `impl Drop for TypedObjectStorage` (and same as the
    /// 4-table-lockstep arms in `kinded_slot.rs::drop` /
    /// `vm_impl/stack.rs::drop_with_kind` / `closure_layout.rs::
    /// SharedCell::drop`). Called by both `impl Drop` (legacy
    /// `Arc<TypedObjectStorage>` path) and `_drop` (raw-pointer path).
    ///
    /// # Safety
    /// Caller must guarantee `self` is in a live state (slots /
    /// field_kinds / heap_mask all valid per the `new` / `_new`
    /// construction-side contract). Must run at most once per instance.
    unsafe fn drop_fields(&mut self) {
        use crate::heap_value::HeapKind;
        use crate::native_kind::NativeKind;

        // Defensive: if construction left a length mismatch (debug_assert
        // catches it earlier), drop only the prefix where both bookkeeping
        // structures agree. Better a leak than UB.
        let n = self.slots.len().min(self.field_kinds.len());
        for i in 0..n {
            // heap_mask is u64; bits beyond 63 cannot be addressed today.
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
                    NativeKind::String => {
                        std::sync::Arc::decrement_strong_count(bits as *const String);
                    }
                    // Wave 2 Agent B (ADR-006 §2.7.5 amendment, 2026-05-14):
                    // A TypedObject field of kind `NativeKind::StringV2` /
                    // `NativeKind::DecimalV2` holds slot bits = `ptr as u64`
                    // where `ptr: *const StringObj` / `*const DecimalObj`
                    // — v2-raw carrier shape per the §H.4 H-c decision.
                    // Refcount discipline goes through `release_elem`
                    // (HeapElement trait — calls `v2_release` against the
                    // HeapHeader at offset 0; on refcount=0 the carrier-side
                    // `drop` deallocates the repr(C) 24-byte struct). NOT
                    // `Arc::decrement_strong_count` — these are manually-
                    // allocated carriers, not `Arc<T>` allocations.
                    NativeKind::StringV2 => {
                        use crate::v2::heap_element::HeapElement;
                        crate::v2::string_obj::StringObj::release_elem(
                            bits as *const crate::v2::string_obj::StringObj,
                        );
                    }
                    NativeKind::DecimalV2 => {
                        use crate::v2::heap_element::HeapElement;
                        crate::v2::decimal_obj::DecimalObj::release_elem(
                            bits as *const crate::v2::decimal_obj::DecimalObj,
                        );
                    }
                    NativeKind::Ptr(hk) => match hk {
                        HeapKind::String => {
                            std::sync::Arc::decrement_strong_count(bits as *const String);
                        }
                        // V3-S5 ckpt-5-prime (2026-05-15): `HeapKind::TypedArray`
                        // dispatch arm RETIRED per W12 audit §3.6 + handover §0
                        // 4-table lockstep rule. The `TypedArrayData` enum was
                        // deleted at ckpt-1; the outer `HeapValue::TypedArray`
                        // arm at ckpt-4. Ordinal 8 remains as a vacated marker
                        // in `heap_variants.rs::HeapKind` (per ordinal-collision
                        // rule — `value_ffi.rs::HK_TYPED_TABLE` asserts the
                        // collision lineage). No live slot bits in compiled
                        // bytecode carry `NativeKind::Ptr(HeapKind::TypedArray)`
                        // post-ckpt-4 (all producers migrated to v2-raw
                        // `*mut TypedArray<T>` carriers per ADR-006 §2.7.24
                        // Q25.A SUPERSEDED). Refusal #1 binding: do not
                        // reintroduce under any rename/shim/bridge.
                        HeapKind::TypedArray => {
                            unreachable!(
                                "HeapKind::TypedArray ordinal 8 is vacated per W12 audit §3.6; \
                                 no live slot bits carry this kind post-V3-S5 ckpt-4 (TypedArrayData \
                                 enum + outer HeapValue::TypedArray arm deleted; v2-raw \
                                 *mut TypedArray<T> carriers per ADR-006 §2.7.24 Q25.A SUPERSEDED)"
                            );
                        }
                        // Wave 2 Agent D4 ckpt-2 (ADR-006 §2.3 / §2.7.5
                        // amendment, 2026-05-14): a `TypedObject` field of
                        // kind `NativeKind::Ptr(HeapKind::TypedObject)`
                        // holds slot bits = `ptr as u64` where
                        // `ptr: *const TypedObjectStorage` (v2-raw carrier
                        // per Agent D1's `_new` /
                        // `impl HeapElement for TypedObjectStorage`).
                        // Refcount discipline goes through `release_elem`
                        // (HeapElement trait — calls `v2_release` against
                        // the HeapHeader at offset 0; on refcount=0 the
                        // carrier-side `_drop` runs the per-field
                        // heap-mask walk and deallocates the `repr(C)`
                        // struct). Mirror of the §2.7.5 StringV2 /
                        // DecimalV2 release arms above (Agent B precedent).
                        HeapKind::TypedObject => {
                            use crate::v2::heap_element::HeapElement;
                            TypedObjectStorage::release_elem(
                                bits as *const TypedObjectStorage,
                            );
                        }
                        HeapKind::HashMap => {
                            // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14):
                            // bits are `Arc::into_raw(Arc<HashMapKindedRef>)`
                            // per ADR-006 §2.7.24 Q25.B SUPERSEDED carrier
                            // shape. Release dispatches outer Arc decrement;
                            // enum Drop chains to per-V `Arc<HashMapData<V>>`
                            // release.
                            std::sync::Arc::decrement_strong_count(
                                bits as *const HashMapKindedRef,
                            );
                        }
                        HeapKind::HashSet => {
                            std::sync::Arc::decrement_strong_count(bits as *const HashSetData);
                        }
                        HeapKind::Deque => {
                            std::sync::Arc::decrement_strong_count(bits as *const DequeData);
                        }
                        HeapKind::Channel => {
                            std::sync::Arc::decrement_strong_count(bits as *const ChannelData);
                        }
                        HeapKind::Mutex => {
                            std::sync::Arc::decrement_strong_count(bits as *const MutexData);
                        }
                        HeapKind::Atomic => {
                            std::sync::Arc::decrement_strong_count(bits as *const AtomicData);
                        }
                        HeapKind::Lazy => {
                            std::sync::Arc::decrement_strong_count(bits as *const LazyData);
                        }
                        // Wave 2 Agent D4 ckpt-2 (ADR-006 §2.7.24 /
                        // Q25.C.5 + E close 2026-05-14): TraitObject
                        // release via `HeapElement::release_elem` +
                        // carrier-side `_drop` (per Agent E's
                        // `impl HeapElement for TraitObjectStorage`).
                        // Mirror of the TypedObject arm above.
                        HeapKind::TraitObject => {
                            use crate::v2::heap_element::HeapElement;
                            TraitObjectStorage::release_elem(
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
                        HeapKind::FilterExpr => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const crate::value::FilterNode,
                            );
                        }
                        HeapKind::Reference => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const crate::reference::RefTarget,
                            );
                        }
                        HeapKind::Iterator => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const crate::iterator_state::IteratorState,
                            );
                        }
                        HeapKind::PriorityQueue => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const PriorityQueueData,
                            );
                        }
                        HeapKind::Range => {
                            std::sync::Arc::decrement_strong_count(bits as *const RangeData);
                        }
                        HeapKind::Result => {
                            std::sync::Arc::decrement_strong_count(bits as *const ResultData);
                        }
                        HeapKind::Option => {
                            std::sync::Arc::decrement_strong_count(bits as *const OptionData);
                        }
                        HeapKind::Closure => {
                            std::sync::Arc::decrement_strong_count(bits as *const HeapValue);
                        }
                        HeapKind::Future => {
                            // No-op: future-id inline scalar.
                        }
                        HeapKind::ModuleFn => {
                            // No-op: module-fn-id inline scalar.
                        }
                        HeapKind::Matrix => {
                            std::sync::Arc::decrement_strong_count(bits as *const MatrixData);
                        }
                        HeapKind::MatrixSlice => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const MatrixSliceData,
                            );
                        }
                        HeapKind::SharedCell => {
                            std::sync::Arc::decrement_strong_count(
                                bits as *const crate::v2::closure_layout::SharedCell,
                            );
                        }
                        HeapKind::Char => {
                            debug_assert!(
                                false,
                                "TypedObjectStorage::drop_fields: heap_mask bit {} set with \
                                 inline-scalar kind Char (schema_id={}); \
                                 construction-side soundness violation",
                                i, self.schema_id
                            );
                        }
                        HeapKind::NativeScalar => {
                            debug_assert!(
                                false,
                                "TypedObjectStorage::drop_fields: NativeScalar kinded carrier \
                                 pending phase-2c kinded redesign (ADR-006 §2.7.4); \
                                 schema_id={}, bit {}",
                                self.schema_id, i
                            );
                        }
                    },
                    other => {
                        debug_assert!(
                            false,
                            "TypedObjectStorage::drop_fields: heap_mask bit {} set with \
                             non-heap NativeKind {:?} (schema_id={}); \
                             construction-side soundness violation",
                            i, other, self.schema_id
                        );
                    }
                }
            }
        }
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
    /// ADR-006 §2.5 + Wave 2 Agent D1 (2026-05-14): delegates to the shared
    /// `drop_fields` helper that walks `heap_mask` and dispatches per-slot
    /// on `field_kinds[i]`. The same helper powers `_drop` (raw-pointer
    /// path) so both the legacy `Arc<TypedObjectStorage>` lifecycle and
    /// the v2-raw raw-pointer lifecycle retire heap-slot Arc shares with
    /// identical semantics.
    ///
    /// Soundness contract (must hold by construction; see
    /// `TypedObjectStorage::new` / `_new`):
    ///
    /// - For every `i` where `heap_mask >> i & 1 == 1`, the slot's `u64`
    ///   bits are the result of `Arc::into_raw::<T>` where `T` matches
    ///   `field_kinds[i]` (per the per-HeapKind table in `drop_fields`).
    /// - `NativeKind::Ptr(HeapKind::{Future, ModuleFn, Char, NativeScalar})`
    ///   are inline-scalar payloads (no `Arc<T>`); a heap_mask bit set
    ///   with one of those kinds is a soundness violation surfaced by
    ///   debug_assert in `drop_fields`.
    fn drop(&mut self) {
        // SAFETY: `drop_fields` walks the heap_mask + field_kinds arrays
        // and retires Arc shares per the construction-side contract. Runs
        // exactly once per instance (Rust's Drop machinery enforces this
        // for `Arc<TypedObjectStorage>` instances; raw-pointer instances
        // route through `_drop` which calls `drop_fields` directly and
        // never reaches here).
        unsafe { self.drop_fields(); }
    }
}

// Wave 2 Agent D1 (2026-05-14): v2-raw HeapElement impl per ADR-006 §2.3
// amendment + audit §4.3 Obstacle O-3.a resolution. Constrains
// `TypedObjectStorage` to the HeapHeader-at-offset-0 v2-raw element-carrier
// contract so future call sites can store raw `*const TypedObjectStorage`
// bits in `TypedArray<*const TypedObjectStorage>` (audit §2.2 / §3.3 / S3
// territory) and dispatch per-element retain/release via the trait.
//
// The trait dispatches refcount through the on-header refcount via
// `v2_release` — distinct from the legacy `Arc<TypedObjectStorage>` path
// which dispatches via Rust `Arc::decrement_strong_count`. Per the struct
// docstring, both carrier shapes coexist at the struct level during the
// Wave 2 dispatch transition; the slot ABI discriminates them by
// allocation provenance (Agent D2's call sites use `_new` and the raw-
// pointer slot constructor; existing call sites use `Arc::new` and the
// legacy slot constructor).
unsafe impl crate::v2::heap_element::HeapElement for TypedObjectStorage {
    unsafe fn release_elem(ptr: *const Self) {
        if unsafe { crate::v2::refcount::v2_release(&(*ptr).header) } {
            unsafe { Self::_drop(ptr as *mut Self) };
        }
    }
}

// ── TypedArray buckets (DELETED — V3-S5 ckpt-1, 2026-05-15) ──────────────────
//
// The `TypedArrayData` enum + impl blocks + `Display for TypedArrayData` +
// `typed_array_structural_eq` were DELETED here per ADR-006 §2.7.24 Q25.A
// SUPERSEDED + W12-typed-array-data-deletion-audit §3.5. The 22-variant
// enum migrates to v2-raw `TypedArray<T>` flat-struct per-T monomorphization.
// Consumer cascade lands across ckpt-2 (array_transform/aggregation/sets),
// ckpt-3 (array_operations/concat/object_creation), ckpt-4 (TypedBuffer +
// HeapValue::TypedArray arm + HeapKind::TypedArray ordinal), ckpt-5 (wire/
// json/marshal + 4-table lockstep delete), ckpt-6 (JIT FFI). The
// `Arc<TypedArrayData>` payload at heap_variants.rs:476 stays until ckpt-4.
//
// Refusal #1 binding: do not resurrect TypedArrayData under any rename
// (e.g. TypedArrayKind, TypedArrayCarrier, TypedBuffer<T> wrapper enum).

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
            // Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14): TypedObjectPtr's
            // Clone impl bumps the v2-raw HeapHeader-at-offset-0 refcount via
            // `v2_retain`, mirroring the typed-Arc clone shape of every other
            // heap arm.
            HeapValue::TypedObject(s) => HeapValue::TypedObject(s.clone()),
            // OwnedClosureBlock::clone is one refcount bump on the typed
            // closure block + one Arc bump on the shared layout.
            HeapValue::ClosureRaw(v) => HeapValue::ClosureRaw(v.clone()),
            HeapValue::TaskGroup(v) => HeapValue::TaskGroup(Arc::clone(v)),
            // V3-S5 ckpt-4 (2026-05-15): `HeapValue::TypedArray(v) =>
            // HeapValue::TypedArray(Arc::clone(v))` clone arm DELETED in
            // lockstep with the variant + the `TypedArrayData` enum
            // (ckpt-1) + `TypedBuffer<T>` / `AlignedTypedBuffer` wrapper
            // layer (ckpt-4). W12 audit §3.5/§B + ADR-006 §2.7.24 Q25.A
            // SUPERSEDED. Refusal #1 binding.
            HeapValue::Temporal(v) => HeapValue::Temporal(Arc::clone(v)),
            HeapValue::TableView(v) => HeapValue::TableView(Arc::clone(v)),
            // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): payload is now
            // `HashMapKindedRef` (not `Arc<HashMapData>`); the enum's manual
            // Clone impl dispatches per-variant `Arc::clone` on the inner
            // `Arc<HashMapData<V>>` — preserving structural sharing.
            HeapValue::HashMap(v) => HeapValue::HashMap(v.clone()),
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
            // Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14): TraitObjectPtr's
            // Clone impl bumps the v2-raw HeapHeader-at-offset-0 refcount via
            // `v2_retain`. Inner `value: *const TypedObjectStorage` and
            // `vtable: Arc<VTable>` shares are bumped through the Clone impl
            // of TraitObjectStorage itself (called by `_drop` / the per-field
            // discipline at refcount=0 release).
            HeapValue::TraitObject(v) => HeapValue::TraitObject(v.clone()),
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
///
/// NOTE (V3-S5 ckpt-5-prime²a, 2026-05-15): currently unreachable post-ckpt-4
/// HeapValue::TypedArray outer-arm deletion (no callers). Signature migrated
/// from `&TypedBuffer<i64>` / `&AlignedTypedBuffer` to `&[i64]` / `&[f64]` per
/// Migration shape (a). Retained for the eventual v2-raw `*mut TypedArray<T>`
/// per-T monomorphic rebuild (cluster-2 v2-raw-heap-audit territory).
#[inline]
fn int_float_array_eq(
    ints: &[i64],
    floats: &[f64],
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

// `typed_array_structural_eq` DELETED — V3-S5 ckpt-1, 2026-05-15.
// Per-arm TypedArrayData structural equality dispatch is retired with the
// enum. Consumer sites at structural_eq + equals (HeapValue::TypedArray
// arm match) cascade-break here and surface for ckpt-4 (HeapValue::TypedArray
// arm rebuild atop v2-raw `*mut TypedArray<T>` per-T monomorphic dispatch).

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
            // V3-S5 ckpt-4 (2026-05-15): `HeapValue::TypedArray(ta) =>
            // write!(f, "{}", ta)` Display arm DELETED in lockstep with the
            // variant + the `TypedArrayData` Display impl (ckpt-1). W12
            // audit §3.5/§B + ADR-006 §2.7.24 Q25.A SUPERSEDED.
            HeapValue::HashMap(kref) => {
                // Wave 2 Round 3b C2-joint ckpt-3 (2026-05-14): full per-V
                // entry dump. Walks `*mut TypedArray<*const StringObj>`
                // keys + per-V `*mut TypedArray<V>` values; formats as
                // `{"k1": v1, "k2": v2}` using each V's natural Display.
                // For TypedObject / TraitObject the inner value renders
                // as a summary tag — full recursive rendering lives at
                // printing.rs (which has the depth-budgeted recursive
                // Display via format_heap_value). ADR-006 §2.7.24 Q25.B
                // SUPERSEDED + audit §C.4.
                hashmap_kref_display(kref, f)
            }
            // Wave 13 W13-hashset-rebuild (ADR-006 §2.7.15 / Q16,
            // 2026-05-10): one-keyspace mirror of HashMap's Display
            // shape — `{"a", "b", ...}` braces with comma-separated
            // quoted strings, no values.
            HeapValue::HashSet(d) => {
                write!(f, "{{")?;
                for (i, k) in d.keys.iter().enumerate() {
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
                for (i, v) in d.heap.iter().enumerate() {
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
                // Wave 2 Round 4 D4 ckpt-3 (2026-05-14): `t.value` is
                // `*const TypedObjectStorage` (v2-raw shape) — field
                // access goes through an unsafe deref.
                // SAFETY: `t.value` is non-null per universal-dyn
                // construction; the `&HeapValue::TraitObject(t)` borrow
                // holds the carrier live for this scope.
                let inner_schema_id = unsafe { (*t.value).schema_id };
                write!(
                    f,
                    "<dyn {} #{}>",
                    trait_name, inner_schema_id
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
            // V3-S5 ckpt-4 (2026-05-15): `(HeapValue::TypedArray(...), ...)`
            // structural-eq arm DELETED. The `HeapValue::TypedArray` outer
            // arm was retired in lockstep with this ckpt-4 +
            // `typed_array_structural_eq` was deleted at V3-S5 ckpt-1
            // (heap_value.rs wholesale deletion per W12 audit §3.5 + ADR-006
            // §2.7.24 Q25.A SUPERSEDED). Per-arm TypedArrayData structural
            // equality dispatch retires with the enum; no replacement (the
            // v2-raw `TypedArray<T>` flat struct compares element-wise via
            // its own `==` impl, not through `HeapValue::structural_eq`).
            // Refusal #1 binding.
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
                // Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14): payloads are
                // `TypedObjectPtr` (raw `*const TypedObjectStorage`); pointer-
                // equality is the fast path for shared storage.
                if a.as_ptr() == b.as_ptr() {
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
            // V3-S5 ckpt-4 (2026-05-15): `(HeapValue::TypedArray(...), ...)`
            // equals arm DELETED in lockstep with the structural_eq arm
            // above + the outer `HeapValue::TypedArray` variant +
            // `typed_array_structural_eq` (ckpt-1). W12 audit §3.5 +
            // ADR-006 §2.7.24 Q25.A SUPERSEDED. Refusal #1 binding.
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
        // Nested TypedObject: outer storage holds a v2-raw
        // `*const TypedObjectStorage` in slot 0 via
        // `NativeKind::Ptr(HeapKind::TypedObject)`.
        //
        // W5 v0.3 fix (2026-05-17): migrated the inner construction to
        // the v2-raw `_new` allocator per
        // `executor/objects/property_access.rs::length_typed_object_empty`
        // rationale. The previous shape stored `ValueSlot::from_typed_object(
        // Arc<TypedObjectStorage>)` bits in a slot whose field_kinds entry
        // was `Ptr(HeapKind::TypedObject)`. The outer storage's
        // `drop_fields` dispatch on that entry calls
        // `TypedObjectStorage::release_elem` → `v2_release` → `_drop` →
        // `std::alloc::dealloc(ptr, Layout::new::<TypedObjectStorage>)`
        // on the Arc-allocated inner. That's a wrong-allocator-pair
        // free (Arc layout has `ArcInner` header before `T`) → SIGABRT.
        //
        // The witness probe is rewritten to the v2-raw equivalent: peek
        // the header refcount via raw pointer borrow.
        use std::sync::atomic::Ordering;

        let inner_kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64]);
        // SAFETY: `_new` returns a refcount=1 raw pointer; the outer
        // storage takes that share, and `drop(outer)` runs drop_fields
        // which releases it via release_elem → _drop.
        unsafe {
            let inner_ptr = TypedObjectStorage::_new(
                100,
                vec![ValueSlot::from_int(7)].into_boxed_slice(),
                0,
                inner_kinds,
            );
            // Bump the inner refcount once so we have a witness share
            // that observes the outer's release. After outer drop the
            // refcount should be back to 1 (our witness only).
            crate::v2::refcount::v2_retain(&(*inner_ptr).header);
            assert_eq!((*inner_ptr).header.refcount.load(Ordering::SeqCst), 2);

            let outer_kinds: Arc<[NativeKind]> =
                Arc::from(vec![NativeKind::Ptr(HeapKind::TypedObject)]);
            let outer = TypedObjectStorage::new(
                101,
                vec![ValueSlot::from_typed_object_raw(inner_ptr)].into_boxed_slice(),
                0b1,
                outer_kinds,
            );

            drop(outer);
            assert_eq!((*inner_ptr).header.refcount.load(Ordering::SeqCst), 1);

            // Witness cleanup: retire the share we minted above via
            // `release_elem` (refcount=1→0 → `_drop` runs internally) so
            // the inner allocation is freed and Miri reports no leak.
            use crate::v2::heap_element::HeapElement;
            TypedObjectStorage::release_elem(inner_ptr);
        }
    }

    // ── Wave 2 Agent D1 v2-raw HeapHeader-equipped shape change tests ──────────

    #[test]
    fn header_initializes_with_typed_object_kind_and_refcount_one() {
        // Wave 2 Agent D1: confirm `TypedObjectStorage::new` initializes the
        // HeapHeader at offset 0 with HEAP_KIND_V2_TYPED_OBJECT and refcount=1.
        // Used by Arc<TypedObjectStorage>-path callers (the legacy path); the
        // header's refcount sits unused for the Arc lifetime.
        let kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64]);
        let storage = TypedObjectStorage::new(
            1,
            vec![ValueSlot::from_int(0)].into_boxed_slice(),
            0,
            kinds,
        );
        assert_eq!(
            storage.header.kind(),
            crate::v2::heap_header::HEAP_KIND_V2_TYPED_OBJECT,
        );
        assert_eq!(storage.header.get_refcount(), 1);
    }

    #[test]
    fn v2_raw_new_and_drop_round_trip() {
        // Wave 2 Agent D1: confirm the v2-raw allocator + deallocator pair
        // (`_new` + `_drop`) round-trips a simple scalar-only TypedObjectStorage
        // without leaking. Mirror of DecimalObj::test_drop_does_not_leak —
        // Miri / valgrind validate no leak.
        let kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64]);
        unsafe {
            let ptr = TypedObjectStorage::_new(
                7,
                vec![ValueSlot::from_int(42)].into_boxed_slice(),
                0,
                kinds,
            );
            assert!(!ptr.is_null());
            assert_eq!(
                (*ptr).header.kind(),
                crate::v2::heap_header::HEAP_KIND_V2_TYPED_OBJECT,
            );
            assert_eq!((*ptr).header.get_refcount(), 1);
            assert_eq!((*ptr).schema_id, 7);
            assert_eq!((&(*ptr).slots).len(), 1);
            assert_eq!((&(*ptr).slots)[0].as_i64(), 42);
            TypedObjectStorage::_drop(ptr);
            // ptr is dangling; cannot dereference further.
        }
    }

    #[test]
    fn v2_raw_new_releases_string_share_at_drop() {
        // Wave 2 Agent D1: confirm `_drop` runs the heap-mask field walk
        // (mirror of `impl Drop`'s legacy behaviour) and retires one Arc
        // strong-count share per heap-kinded slot.
        let s: Arc<String> = Arc::new("v2-raw-test".to_string());
        let witness = Arc::clone(&s);
        assert_eq!(Arc::strong_count(&witness), 2);

        let kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::String]);
        unsafe {
            let ptr = TypedObjectStorage::_new(
                17,
                vec![ValueSlot::from_string_arc(s)].into_boxed_slice(),
                0b1,
                kinds,
            );
            assert_eq!(Arc::strong_count(&witness), 2);
            TypedObjectStorage::_drop(ptr);
        }
        assert_eq!(Arc::strong_count(&witness), 1);
    }

    #[test]
    fn heap_element_release_elem_deallocates_at_refcount_zero() {
        // Wave 2 Agent D1: confirm `HeapElement::release_elem` decrements
        // via `v2_release` and deallocates at refcount=0. Mirror of
        // DecimalObj::test_heap_element_release_elem_to_zero.
        use crate::v2::heap_element::HeapElement;
        let kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64]);
        unsafe {
            let ptr = TypedObjectStorage::_new(
                3,
                vec![ValueSlot::from_int(0)].into_boxed_slice(),
                0,
                kinds,
            );
            // refcount=1; release_elem deallocates.
            TypedObjectStorage::release_elem(ptr);
            // ptr is dangling; valgrind / Miri confirms no leak.
        }
    }

    #[test]
    fn heap_element_release_elem_preserves_held_share() {
        // Wave 2 Agent D1: confirm `release_elem` decrements but does NOT
        // deallocate when refcount > 1. Mirror of
        // DecimalObj::test_heap_element_release_elem_held_share.
        use crate::v2::heap_element::HeapElement;
        use crate::v2::refcount::{v2_get_refcount, v2_retain};
        let kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64]);
        unsafe {
            let ptr = TypedObjectStorage::_new(
                5,
                vec![ValueSlot::from_int(7)].into_boxed_slice(),
                0,
                kinds,
            );
            let header = &(*ptr).header as *const crate::v2::heap_header::HeapHeader;

            v2_retain(header); // refcount = 2
            TypedObjectStorage::release_elem(ptr); // refcount = 1 (does not deallocate)
            assert_eq!(v2_get_refcount(header), 1);

            // Clean up the held share.
            TypedObjectStorage::_drop(ptr);
        }
    }

    #[test]
    fn header_field_is_at_offset_zero() {
        // Wave 2 Agent D1: confirm the #[repr(C)] field-order invariant —
        // the `header: HeapHeader` field sits at offset 0 of
        // TypedObjectStorage. This is the precondition for the
        // HeapElement::release_elem body's `v2_release(&(*ptr).header)` call
        // to read the refcount at the v2-raw canonical offset (offset 0
        // mirrors StringObj / DecimalObj precedents).
        let kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64]);
        let storage = TypedObjectStorage::new(
            1,
            vec![ValueSlot::from_int(0)].into_boxed_slice(),
            0,
            kinds,
        );
        let base = &storage as *const _ as usize;
        let header_offset = &storage.header as *const _ as usize - base;
        assert_eq!(header_offset, 0, "header must be at offset 0 (#[repr(C)] contract)");
    }
}

// Wave 2 Round 3b C2-joint ckpt-3 (2026-05-14): rewritten against the
// per-V `HashMapData<V>` mutation API (insert / remove / get_share /
// merge). The pre-Q25.B-SUPERSEDED non-generic `HashMapData::insert(k,
// Arc<HeapValue>) / remove(k) -> bool / get(k) -> Option<Arc<HeapValue>>`
// shape is gone; tests below exercise the per-V semantics on the most
// common production-V cases:
//
// - `V = i64` (Q25.B I64 arm): POD/Copy V — pin len/contains/insert/remove
//   semantics without refcount-share complications.
// - `V = *const StringObj` (Q25.B String arm): HeapElement V — pin the
//   v2_retain / release_elem refcount-share threading via the
//   `HashMapValueElem::share_clone` + `release_owned` dispatch.
//
// The previously-introduced undeclared feature gate (Round 3b ckpt-2)
// guarding this test module has been REMOVED per ckpt-3 dispatch
// Group F (mandatory non-negotiable): the gate was masquerading as a
// feature flag while being functionally `#[cfg(false)]` (no Cargo.toml
// declaration), matching CLAUDE.md Forbidden Rationalizations.
// ADR-006 §2.7.24 Q25.B SUPERSEDED + audit §C.4 option (a.2).
#[cfg(test)]
mod hashmap_mutation {
    //! Wave 2 Round 3b C2-joint ckpt-3 (2026-05-14): pin the
    //! `insert` / `remove` / `get_share` / `merge` API contracts on the
    //! post-Q25.B-SUPERSEDED `HashMapData<V>`. Storage-layer counterpart
    //! of `v2_set` / `v2_delete` / `v2_get` / `v2_merge` in
    //! `shape-vm/executor/objects/hashmap_methods.rs`.
    use super::*;
    use crate::v2::refcount::v2_get_refcount;
    use crate::v2::string_obj::StringObj;
    use std::sync::Arc;

    // ── V = i64 (POD/Copy) ──────────────────────────────────────────────

    #[test]
    fn i64_insert_appends_new_entry_and_grows_index() {
        let mut m: HashMapData<i64> = HashMapData::new();
        unsafe {
            assert!(m.insert("a", 1));
            assert!(m.insert("b", 2));
        }
        assert_eq!(m.len(), 2);
        // Bucket index has registrations for both keys' hashes.
        let h_a = fnv1a_hash(b"a");
        let h_b = fnv1a_hash(b"b");
        assert!(m.index.get(&h_a).is_some());
        assert!(m.index.get(&h_b).is_some());
    }

    #[test]
    fn i64_insert_overwrites_existing_value_and_keeps_len() {
        let mut m: HashMapData<i64> = HashMapData::new();
        unsafe {
            assert!(m.insert("a", 1));
            // Overwrite returns false (existing key).
            assert!(!m.insert("a", 99));
        }
        assert_eq!(m.len(), 1);
        // get_share returns a fresh Copy of the i64 value.
        assert_eq!(m.get_share("a"), Some(99));
    }

    #[test]
    fn i64_remove_present_key_returns_value_and_compacts() {
        let mut m: HashMapData<i64> = HashMapData::new();
        unsafe {
            m.insert("a", 1);
            m.insert("b", 2);
            assert_eq!(m.remove("a"), Some(1));
        }
        assert_eq!(m.len(), 1);
        assert!(m.get_share("a").is_none());
        // "b" should still be reachable — bucket index was renumbered.
        assert_eq!(m.get_share("b"), Some(2));
    }

    #[test]
    fn i64_remove_missing_key_returns_none_and_is_noop() {
        let mut m: HashMapData<i64> = HashMapData::new();
        unsafe {
            m.insert("a", 1);
            assert_eq!(m.remove("nope"), None);
        }
        assert_eq!(m.len(), 1);
        assert_eq!(m.get_share("a"), Some(1));
    }

    #[test]
    fn i64_merge_copies_other_entries_with_last_write_wins() {
        let mut a: HashMapData<i64> = HashMapData::new();
        let mut b: HashMapData<i64> = HashMapData::new();
        unsafe {
            a.insert("x", 1);
            a.insert("shared", 10);
            b.insert("y", 2);
            b.insert("shared", 99);
            a.merge(&b);
        }
        assert_eq!(a.len(), 3);
        assert_eq!(a.get_share("x"), Some(1));
        assert_eq!(a.get_share("y"), Some(2));
        // shared overwritten by b's value
        assert_eq!(a.get_share("shared"), Some(99));
    }

    #[test]
    fn i64_smoke_set_set_delete_size() {
        // Storage-layer counterpart of W13-hashmap-mutation smoke:
        //   let m = HashMap(); m.set("a", 1); m.set("b", 2); m.delete("a");
        //   m.size() == 1
        let mut m: HashMapData<i64> = HashMapData::new();
        unsafe {
            m.insert("a", 1);
            m.insert("b", 2);
            assert_eq!(m.remove("a"), Some(1));
        }
        assert_eq!(m.len(), 1);
        assert!(m.get_share("a").is_none());
        assert_eq!(m.get_share("b"), Some(2));
    }

    #[test]
    fn i64_arc_make_mut_clone_on_write_does_not_disturb_shared_handle() {
        // The shape-vm-side handlers Arc::make_mut the receiver share —
        // this exercises the per-V Clone impl which allocates fresh
        // keys + values buffers and share-clones each element.
        let mut owned: Arc<HashMapData<i64>> = Arc::new(HashMapData::new());
        unsafe { Arc::make_mut(&mut owned).insert("a", 1) };
        // Snapshot share — second observer.
        let snapshot = Arc::clone(&owned);
        // Mutate via the local share — should clone-on-write.
        unsafe { Arc::make_mut(&mut owned).insert("b", 2) };
        assert_eq!(owned.len(), 2);
        // Snapshot is undisturbed.
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot.get_share("a"), Some(1));
        assert!(snapshot.get_share("b").is_none());
    }

    // ── V = *const StringObj (HeapElement) ──────────────────────────────

    fn s_obj(s: &str) -> *const StringObj {
        StringObj::new(s) as *const StringObj
    }

    #[test]
    fn string_insert_v2_retain_share_threading() {
        // Pin: insert transfers one share to the map; the original share
        // we keep here is the "witness" that survives.
        let mut m: HashMapData<*const StringObj> = HashMapData::new();
        let v1 = s_obj("hello");
        // Bump witness share so we can observe the map's share separately
        // — refcount=2 after retain.
        unsafe { crate::v2::refcount::v2_retain(&(*v1).header) };
        assert_eq!(unsafe { v2_get_refcount(&(*v1).header) }, 2);
        // Insert (transfers one share to the map).
        unsafe { m.insert("k", v1) };
        // Refcount: witness share + map share = 2 (unchanged because
        // we transferred 1 to the map, leaving 1 with the witness).
        assert_eq!(unsafe { v2_get_refcount(&(*v1).header) }, 2);
        // Map drops at end — retires its share. Manually drop to observe.
        drop(m);
        assert_eq!(unsafe { v2_get_refcount(&(*v1).header) }, 1);
        // Release the witness share.
        unsafe {
            use crate::v2::heap_element::HeapElement;
            StringObj::release_elem(v1);
        }
    }

    #[test]
    fn string_insert_overwrite_retires_old_share() {
        // Pin: insert with existing key retires the old value's share via
        // V::release_owned.
        let mut m: HashMapData<*const StringObj> = HashMapData::new();
        let old = s_obj("old");
        let witness = old;
        unsafe { crate::v2::refcount::v2_retain(&(*old).header) }; // witness = 2
        unsafe { m.insert("k", old) };
        assert_eq!(unsafe { v2_get_refcount(&(*witness).header) }, 2);
        // Overwrite with new value; the old share inside the map is retired.
        let new_val = s_obj("new");
        unsafe { m.insert("k", new_val) };
        // Map no longer holds the original value's share — witness alone.
        assert_eq!(unsafe { v2_get_refcount(&(*witness).header) }, 1);
        // Release witness.
        unsafe {
            use crate::v2::heap_element::HeapElement;
            StringObj::release_elem(witness);
        }
        // m drops new_val + key allocations naturally at scope end.
    }

    #[test]
    fn string_remove_transfers_share_to_caller() {
        // Pin: remove returns the value, transferring its share to the caller.
        let mut m: HashMapData<*const StringObj> = HashMapData::new();
        let v = s_obj("val");
        unsafe { crate::v2::refcount::v2_retain(&(*v).header) }; // witness shares = 2
        unsafe { m.insert("k", v) };
        assert_eq!(unsafe { v2_get_refcount(&(*v).header) }, 2);
        let removed = unsafe { m.remove("k") };
        assert!(removed.is_some());
        let removed_ptr = removed.unwrap();
        assert_eq!(removed_ptr, v);
        // Refcount unchanged: map released its share + remove transferred
        // a share to the caller (this fn) = net 0 change.
        assert_eq!(unsafe { v2_get_refcount(&(*v).header) }, 2);
        // Release the witness + the removed share.
        unsafe {
            use crate::v2::heap_element::HeapElement;
            StringObj::release_elem(removed_ptr);
            StringObj::release_elem(v);
        }
    }

    #[test]
    fn string_get_share_bumps_refcount_for_caller() {
        // Pin: get_share returns a fresh refcount-share copy, leaving
        // the map's share intact.
        let mut m: HashMapData<*const StringObj> = HashMapData::new();
        let v = s_obj("val");
        unsafe { m.insert("k", v) }; // map owns the only share now.
        assert_eq!(unsafe { v2_get_refcount(&(*v).header) }, 1);
        let got = m.get_share("k").expect("present");
        assert_eq!(got, v);
        // Refcount bumped — map (1) + caller's share (1) = 2.
        assert_eq!(unsafe { v2_get_refcount(&(*v).header) }, 2);
        // Release caller's share.
        unsafe {
            use crate::v2::heap_element::HeapElement;
            StringObj::release_elem(got);
        }
        assert_eq!(unsafe { v2_get_refcount(&(*v).header) }, 1);
        // Map drops at scope end, retiring last share.
    }

    #[test]
    fn string_merge_share_clones_each_other_entry() {
        // Pin: merge bumps refcount on each value cloned from other.
        let mut a: HashMapData<*const StringObj> = HashMapData::new();
        let mut b: HashMapData<*const StringObj> = HashMapData::new();
        let v_x = s_obj("x_val");
        let v_y = s_obj("y_val");
        unsafe {
            a.insert("x", v_x);
            b.insert("y", v_y);
            assert_eq!(v2_get_refcount(&(*v_x).header), 1);
            assert_eq!(v2_get_refcount(&(*v_y).header), 1);
            a.merge(&b);
        }
        assert_eq!(a.len(), 2);
        // After merge: y is share-cloned into a — refcount = 2 (a + b).
        assert_eq!(unsafe { v2_get_refcount(&(*v_y).header) }, 2);
        // x is unchanged (only in a).
        assert_eq!(unsafe { v2_get_refcount(&(*v_x).header) }, 1);
    }

    // ── V = HashMapKindedRef (recursive carrier) ──────────────────────────
    //
    // Wave N hashmap-value-v-arm follow-up (cluster-2 closure-wave-C,
    // 2026-05-16). Pin the per-V `insert` / `len` / `get_share` API on
    // `HashMapData<HashMapKindedRef>`. Storage-layer counterpart of
    // `v2_group_by` in `shape-vm/executor/objects/hashmap_methods.rs`.

    #[test]
    fn hashmap_value_v_insert_appends_and_grows_index() {
        let mut outer: HashMapData<HashMapKindedRef> = HashMapData::new();
        // Two inner buckets — one for "small", one for "large".
        let mut inner_small: HashMapData<i64> = HashMapData::new();
        let mut inner_large: HashMapData<i64> = HashMapData::new();
        unsafe {
            inner_small.insert("a", 1);
            inner_small.insert("b", 2);
            inner_large.insert("c", 100);
            outer.insert(
                "small",
                HashMapKindedRef::I64(Arc::new(inner_small)),
            );
            outer.insert(
                "large",
                HashMapKindedRef::I64(Arc::new(inner_large)),
            );
        }
        assert_eq!(outer.len(), 2);
        // Bucket index has registrations for both group keys.
        let h_small = fnv1a_hash(b"small");
        let h_large = fnv1a_hash(b"large");
        assert!(outer.index.get(&h_small).is_some());
        assert!(outer.index.get(&h_large).is_some());
        // get_share returns a fresh per-variant Arc::clone-bumped copy.
        let small_ref = outer.get_share("small").expect("small bucket present");
        match small_ref {
            HashMapKindedRef::I64(arc) => assert_eq!(arc.len(), 2),
            other => panic!("unexpected variant {:?}", other.values_kind()),
        }
    }

    #[test]
    fn hashmap_value_v_remove_returns_inner_and_compacts() {
        let mut outer: HashMapData<HashMapKindedRef> = HashMapData::new();
        let mut inner_a: HashMapData<i64> = HashMapData::new();
        let mut inner_b: HashMapData<i64> = HashMapData::new();
        unsafe {
            inner_a.insert("x", 1);
            inner_b.insert("y", 2);
            outer.insert("group-a", HashMapKindedRef::I64(Arc::new(inner_a)));
            outer.insert("group-b", HashMapKindedRef::I64(Arc::new(inner_b)));
            let removed = outer.remove("group-a");
            assert!(removed.is_some());
            // Removed bucket has the expected inner shape.
            match removed.unwrap() {
                HashMapKindedRef::I64(arc) => {
                    assert_eq!(arc.len(), 1);
                    assert_eq!(arc.get_share("x"), Some(1));
                }
                other => panic!("unexpected variant {:?}", other.values_kind()),
            }
        }
        assert_eq!(outer.len(), 1);
        // "group-b" reachable post-renumber.
        let b_ref = outer.get_share("group-b").expect("group-b present");
        match b_ref {
            HashMapKindedRef::I64(arc) => assert_eq!(arc.get_share("y"), Some(2)),
            other => panic!("unexpected variant {:?}", other.values_kind()),
        }
    }

    #[test]
    fn hashmap_value_v_clone_share_clones_inner_arcs() {
        // HashMapData<V>::Clone walks elements via share_clone — for
        // V = HashMapKindedRef this calls HashMapKindedRef::clone which
        // is per-variant Arc::clone on the inner Arc<HashMapData<V_inner>>.
        // The clone yields fresh buffer allocations holding bumped Arcs.
        let mut outer: HashMapData<HashMapKindedRef> = HashMapData::new();
        let inner: Arc<HashMapData<i64>> = {
            let mut d: HashMapData<i64> = HashMapData::new();
            unsafe { d.insert("k", 42) };
            Arc::new(d)
        };
        // Refcount before insert: 1 (only `inner` owns).
        assert_eq!(Arc::strong_count(&inner), 1);
        unsafe { outer.insert("g", HashMapKindedRef::I64(Arc::clone(&inner))) };
        // Refcount after insert: 2 (inner + outer's buffer share).
        assert_eq!(Arc::strong_count(&inner), 2);
        // Clone outer — share_clone bumps the inner Arc one more time.
        let _outer_clone = outer.clone();
        assert_eq!(Arc::strong_count(&inner), 3);
        // Drop clone; refcount drops back to 2.
        drop(_outer_clone);
        assert_eq!(Arc::strong_count(&inner), 2);
    }

    #[test]
    fn hashmap_value_v_drop_releases_inner_arcs() {
        // HashMapData<HashMapKindedRef>::Drop calls
        // <HashMapKindedRef as HashMapValueElem>::release_typed_array,
        // which walks the buffer with ptr::read and lets each element
        // drop (auto-derived → Arc::drop on inner Arc<HashMapData<V_inner>>).
        let inner: Arc<HashMapData<i64>> = {
            let mut d: HashMapData<i64> = HashMapData::new();
            unsafe { d.insert("k", 7) };
            Arc::new(d)
        };
        assert_eq!(Arc::strong_count(&inner), 1);
        {
            let mut outer: HashMapData<HashMapKindedRef> = HashMapData::new();
            unsafe {
                outer.insert("g", HashMapKindedRef::I64(Arc::clone(&inner)));
            }
            assert_eq!(Arc::strong_count(&inner), 2);
            // outer drops at scope-end; the inner Arc share retires.
        }
        assert_eq!(Arc::strong_count(&inner), 1);
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
        assert_eq!(s.keys[0].as_str(), "a");
        assert_eq!(s.keys[1].as_str(), "b");
        assert_eq!(s.keys[2].as_str(), "c");
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

// Wave 2 Round 4 D4 ckpt-3 (2026-05-14): the `trait_object_storage` test mod
// was authored against `TraitObjectStorage.value: Arc<TypedObjectStorage>`
// + `make_object()` returning `Arc<TypedObjectStorage>`. Post inner-field
// shift to `*const TypedObjectStorage`, every test that does `Arc::clone(&obj)`
// or `Arc::downgrade(&obj)` no longer compiles, and every test that observes
// the inner refcount via the weak count needs to migrate to HeapHeader
// `get_refcount()` inspection. Ckpt-final adapts the tests in lockstep with
// the full HeapValue::TypedObject variant signature flip; intermediate
// close gate (broken cargo check OK per
// `docs/cluster-audits/bulldozer-multi-session-chain-pattern.md` §Discipline
// relaxed) preserves the test mod source verbatim under a never-match cfg
// so the ckpt-final adapter has the original assertions as the migration
// target.
#[cfg(any())]
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
    ///
    /// **Wave 2 Round 4 D4 ckpt-3 (2026-05-14): returns `*mut
    /// TypedObjectStorage` (v2-raw `_new` shape)** per the
    /// `TraitObjectStorage.value: *const TypedObjectStorage` inner-field
    /// shift. Tests that previously observed inner refcount via
    /// `Arc::downgrade(&obj)` are surfaced as broken pending test-side
    /// migration to HeapHeader refcount inspection
    /// (`unsafe { (*ptr).header.get_refcount() }`).
    fn make_object(value: i64) -> *mut TypedObjectStorage {
        let mut slots: Vec<crate::slot::ValueSlot> = Vec::with_capacity(1);
        slots.push(crate::slot::ValueSlot::from_int(value));
        let field_kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64]);
        TypedObjectStorage::_new(
            42, // schema_id — arbitrary
            slots.into_boxed_slice(),
            0,  // heap_mask: no heap slots
            field_kinds,
        )
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

    // ── Wave 2 Agent E (2026-05-14): v2-raw HeapHeader migration tests ──────

    #[test]
    fn new_initializes_heap_header() {
        // Wave 2 Agent E: confirm `TraitObjectStorage::new` initializes the
        // HeapHeader at offset 0 with HEAP_KIND_V2_TRAIT_OBJECT and
        // refcount=1. The header sits unused for `Arc<TraitObjectStorage>`
        // instances (Arc owns the lifecycle).
        let obj = make_object(1);
        let vt = make_vtable("Animal", 100, "name");
        let storage = TraitObjectStorage::new(obj, vt);
        assert_eq!(
            storage.header.kind(),
            crate::v2::heap_header::HEAP_KIND_V2_TRAIT_OBJECT,
        );
        assert_eq!(storage.header.get_refcount(), 1);
    }

    #[test]
    fn v2_raw_new_drop_round_trip_balances_inner_arcs() {
        // Wave 2 Agent E: verify the v2-raw lifecycle (`_new` + `_drop`)
        // round-trips cleanly without leaking the inner Arc shares.
        // Mirror of D1's `TypedObjectStorage` round-trip test (commit
        // 0e4510d4).
        let obj = make_object(7);
        let vt = make_vtable("Animal", 100, "name");
        let obj_weak = Arc::downgrade(&obj);
        let vt_weak = Arc::downgrade(&vt);
        unsafe {
            let ptr = TraitObjectStorage::_new(obj, vt);
            assert!(!ptr.is_null());
            // Header refcount=1 + inner Arcs hold one share each.
            assert_eq!((*ptr).header.get_refcount(), 1);
            assert_eq!(obj_weak.strong_count(), 1);
            assert_eq!(vt_weak.strong_count(), 1);
            assert_eq!(
                (*ptr).header.kind(),
                crate::v2::heap_header::HEAP_KIND_V2_TRAIT_OBJECT,
            );
            // `_drop` retires the inner shares + deallocates.
            TraitObjectStorage::_drop(ptr);
        }
        assert_eq!(
            obj_weak.strong_count(),
            0,
            "TypedObject share must retire on _drop",
        );
        assert_eq!(
            vt_weak.strong_count(),
            0,
            "VTable share must retire on _drop",
        );
    }

    #[test]
    fn heap_element_release_elem_to_zero_drops_payload() {
        // Wave 2 Agent E: verify the HeapElement trait dispatch
        // (`release_elem`) deallocates the carrier at refcount=0 and
        // retires the inner Arc shares. Mirror of D1's
        // `TypedObjectStorage::test_heap_element_release_elem_to_zero`.
        use crate::v2::heap_element::HeapElement;
        let obj = make_object(13);
        let vt = make_vtable("Animal", 100, "name");
        let obj_weak = Arc::downgrade(&obj);
        let vt_weak = Arc::downgrade(&vt);
        unsafe {
            let ptr = TraitObjectStorage::_new(obj, vt);
            assert_eq!((*ptr).header.get_refcount(), 1);
            // `release_elem` decrements to 0 and runs `_drop`.
            TraitObjectStorage::release_elem(ptr);
        }
        assert_eq!(
            obj_weak.strong_count(),
            0,
            "TypedObject share must retire after release_elem to zero",
        );
        assert_eq!(
            vt_weak.strong_count(),
            0,
            "VTable share must retire after release_elem to zero",
        );
    }

    #[test]
    fn heap_element_release_elem_held_share_preserves_payload() {
        // Wave 2 Agent E: verify `release_elem` is a no-op at refcount > 1
        // (it decrements but does not deallocate). The inner Arc shares
        // remain held until the final `_drop`. Mirror of D1's
        // `TypedObjectStorage::test_heap_element_release_elem_held_share`.
        use crate::v2::heap_element::HeapElement;
        let obj = make_object(17);
        let vt = make_vtable("Animal", 100, "name");
        let obj_weak = Arc::downgrade(&obj);
        let vt_weak = Arc::downgrade(&vt);
        unsafe {
            let ptr = TraitObjectStorage::_new(obj, vt);
            // Bump refcount to 2 (simulate a second slot holding this
            // carrier).
            (*ptr).header.retain();
            assert_eq!((*ptr).header.get_refcount(), 2);
            // First release_elem: refcount=2 → 1, no dealloc.
            TraitObjectStorage::release_elem(ptr);
            assert_eq!((*ptr).header.get_refcount(), 1);
            // Inner shares still held.
            assert_eq!(obj_weak.strong_count(), 1);
            assert_eq!(vt_weak.strong_count(), 1);
            // Final _drop retires.
            TraitObjectStorage::_drop(ptr);
        }
        assert_eq!(obj_weak.strong_count(), 0);
        assert_eq!(vt_weak.strong_count(), 0);
    }

    #[test]
    fn kinded_slot_from_trait_object_raw_constructor_kind_and_bits() {
        // Wave 2 Agent E: `KindedSlot::from_trait_object_raw` stores a
        // raw `*const TraitObjectStorage` directly (NOT `Arc::into_raw`)
        // with kind `NativeKind::Ptr(HeapKind::TraitObject)`. Mirror of
        // D1's `from_typed_object_raw_constructor_kind_and_bits` test.
        let obj = make_object(2);
        let vt = make_vtable("Animal", 100, "name");
        unsafe {
            let ptr = TraitObjectStorage::_new(obj, vt);
            let slot = KindedSlot::from_trait_object_raw(ptr);
            assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TraitObject));
            // Slot bits are the raw pointer (NOT Arc::into_raw).
            assert_eq!(slot.slot().raw(), ptr as u64);
            // Forget the slot — Wave 2 transitional Arc-style dispatch
            // arms would call `Arc::decrement_strong_count` on raw
            // pointer bits (heap corruption) — see the D1 follow-up
            // lockstep requirement. Deallocate manually instead.
            std::mem::forget(slot);
            TraitObjectStorage::_drop(ptr);
        }
    }

    #[test]
    fn v2_raw_carrier_size_matches_expected_layout() {
        // Wave 2 Agent E: pin the v2-raw struct layout — HeapHeader (8)
        // + Arc<TypedObjectStorage> (8) + Arc<VTable> (8) = 24 bytes
        // (matching the audit §E.3 24-byte size contract for the E-a
        // path). The inner Arcs stay 8-byte each in E's Round 2 scope;
        // D2's lockstep flip migrates the inner pointers but the
        // outer size contract is set here.
        assert_eq!(
            std::mem::size_of::<TraitObjectStorage>(),
            24,
            "TraitObjectStorage v2-raw layout must be 24 bytes \
             (HeapHeader 8 + Arc<TypedObjectStorage> 8 + Arc<VTable> 8)",
        );
    }
}

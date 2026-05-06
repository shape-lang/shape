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

/// Data for IoHandle variant (boxed to keep HeapValue small).
///
/// Wraps an OS resource (file, socket, process) in an Arc<Mutex<Option<IoResource>>>
/// so it can be shared and closed. The `Option` is `None` after close().
/// Rust's `Drop` closes the underlying resource if not already closed.
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

// ── TypedArray buckets ──────────────────────────────────────────────────────

/// Typed array data — consolidates IntArray, FloatArray, BoolArray, Matrix,
/// I8Array..F32Array, and FloatArraySlice into a single HeapValue variant.
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
// All surviving variants own fields whose `Clone` already reclaims-and-clones
// the resource (Arc refcount bump, Box deep-clone, struct field clone). With
// the strict-typed bulldozer's deletion of every `ValueWord`-bearing variant,
// there is no longer any `vw_clone` / `vw_drop` bookkeeping to pair, so this
// impl is purely mechanical delegation.
impl Clone for HeapValue {
    fn clone(&self) -> Self {
        match self {
            HeapValue::String(v) => HeapValue::String(v.clone()),
            HeapValue::Decimal(v) => HeapValue::Decimal(*v),
            HeapValue::BigInt(v) => HeapValue::BigInt(*v),
            HeapValue::Future(v) => HeapValue::Future(*v),
            HeapValue::Char(v) => HeapValue::Char(*v),
            HeapValue::DataTable(v) => HeapValue::DataTable(v.clone()),
            HeapValue::Content(v) => HeapValue::Content(v.clone()),
            HeapValue::Instant(v) => HeapValue::Instant(v.clone()),
            HeapValue::IoHandle(v) => HeapValue::IoHandle(v.clone()),
            HeapValue::NativeScalar(v) => HeapValue::NativeScalar(*v),
            HeapValue::NativeView(v) => HeapValue::NativeView(v.clone()),
            HeapValue::TypedObject {
                schema_id,
                slots,
                heap_mask,
            } => HeapValue::TypedObject {
                schema_id: *schema_id,
                slots: slots.clone(),
                heap_mask: *heap_mask,
            },
            HeapValue::ClosureRaw(v) => HeapValue::ClosureRaw(v.clone()),
            HeapValue::TaskGroup { kind, task_ids } => HeapValue::TaskGroup {
                kind: *kind,
                task_ids: task_ids.clone(),
            },
            HeapValue::TypedArray(v) => HeapValue::TypedArray(v.clone()),
            HeapValue::Temporal(v) => HeapValue::Temporal(v.clone()),
            HeapValue::TableView(v) => HeapValue::TableView(v.clone()),
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

// ── Display ─────────────────────────────────────────────────────────────────

impl fmt::Display for HeapValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeapValue::Char(c) => write!(f, "{}", c),
            HeapValue::String(s) => write!(f, "{}", s),
            HeapValue::TypedObject { .. } => write!(f, "{{...}}"),
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
            HeapValue::TaskGroup { task_ids, .. } => {
                write!(f, "<task_group:{}>", task_ids.len())
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
            (HeapValue::Temporal(TemporalData::DateTime(a)), HeapValue::Temporal(TemporalData::DateTime(b))) => a == b,
            (HeapValue::Content(a), HeapValue::Content(b)) => a == b,
            (HeapValue::Instant(a), HeapValue::Instant(b)) => a == b,
            (HeapValue::IoHandle(a), HeapValue::IoHandle(b)) => {
                std::sync::Arc::ptr_eq(&a.resource, &b.resource)
            }
            (HeapValue::TypedArray(TypedArrayData::I64(a)), HeapValue::TypedArray(TypedArrayData::I64(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::F64(a)), HeapValue::TypedArray(TypedArrayData::F64(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::I64(a)), HeapValue::TypedArray(TypedArrayData::F64(b))) => int_float_array_eq(a, b),
            (HeapValue::TypedArray(TypedArrayData::F64(a)), HeapValue::TypedArray(TypedArrayData::I64(b))) => int_float_array_eq(b, a),
            (HeapValue::TypedArray(TypedArrayData::Bool(a)), HeapValue::TypedArray(TypedArrayData::Bool(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::I8(a)), HeapValue::TypedArray(TypedArrayData::I8(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::I16(a)), HeapValue::TypedArray(TypedArrayData::I16(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::I32(a)), HeapValue::TypedArray(TypedArrayData::I32(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::U8(a)), HeapValue::TypedArray(TypedArrayData::U8(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::U16(a)), HeapValue::TypedArray(TypedArrayData::U16(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::U32(a)), HeapValue::TypedArray(TypedArrayData::U32(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::U64(a)), HeapValue::TypedArray(TypedArrayData::U64(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::F32(a)), HeapValue::TypedArray(TypedArrayData::F32(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::Matrix(a)), HeapValue::TypedArray(TypedArrayData::Matrix(b))) => matrix_eq(a, b),
            (
                HeapValue::TypedArray(TypedArrayData::FloatSlice { parent: p1, offset: o1, len: l1 }),
                HeapValue::TypedArray(TypedArrayData::FloatSlice { parent: p2, offset: o2, len: l2 }),
            ) => {
                let s1 = &p1.data[*o1 as usize..(*o1 + *l1) as usize];
                let s2 = &p2.data[*o2 as usize..(*o2 + *l2) as usize];
                s1 == s2
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
            (
                HeapValue::TypedObject {
                    schema_id: s1,
                    slots: sl1,
                    heap_mask: hm1,
                },
                HeapValue::TypedObject {
                    schema_id: s2,
                    slots: sl2,
                    heap_mask: hm2,
                },
            ) => {
                if s1 != s2 || sl1.len() != sl2.len() || hm1 != hm2 {
                    return false;
                }
                for i in 0..sl1.len() {
                    // Both heap-mask and primitive-mask: compare raw bits
                    // for primitives. For heap slots, raw-bit equality is
                    // also conservatively correct since `ValueSlot` heap
                    // payloads are typed pointers — pointer-equality
                    // implies value-equality for shared Arc'd payloads.
                    if sl1[i].raw() != sl2[i].raw() {
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
            (HeapValue::BigInt(a), HeapValue::Decimal(b)) => bigint_decimal_eq(a, b),
            (HeapValue::Decimal(a), HeapValue::BigInt(b)) => bigint_decimal_eq(b, a),
            (HeapValue::DataTable(a), HeapValue::DataTable(b)) => Arc::ptr_eq(a, b),
            (
                HeapValue::TableView(TableViewData::TypedTable { schema_id: s1, table: t1 }),
                HeapValue::TableView(TableViewData::TypedTable { schema_id: s2, table: t2 }),
            ) => s1 == s2 && Arc::ptr_eq(t1, t2),
            (
                HeapValue::TableView(TableViewData::RowView { schema_id: s1, row_idx: r1, table: t1 }),
                HeapValue::TableView(TableViewData::RowView { schema_id: s2, row_idx: r2, table: t2 }),
            ) => s1 == s2 && r1 == r2 && Arc::ptr_eq(t1, t2),
            (
                HeapValue::TableView(TableViewData::ColumnRef { schema_id: s1, col_id: c1, table: t1 }),
                HeapValue::TableView(TableViewData::ColumnRef { schema_id: s2, col_id: c2, table: t2 }),
            ) => s1 == s2 && c1 == c2 && Arc::ptr_eq(t1, t2),
            (
                HeapValue::TableView(TableViewData::IndexedTable { schema_id: s1, index_col: c1, table: t1 }),
                HeapValue::TableView(TableViewData::IndexedTable { schema_id: s2, index_col: c2, table: t2 }),
            ) => s1 == s2 && c1 == c2 && Arc::ptr_eq(t1, t2),
            (HeapValue::Content(a), HeapValue::Content(b)) => a == b,
            (HeapValue::Instant(a), HeapValue::Instant(b)) => a == b,
            (HeapValue::IoHandle(a), HeapValue::IoHandle(b)) => {
                Arc::ptr_eq(&a.resource, &b.resource)
            }
            (HeapValue::Future(a), HeapValue::Future(b)) => a == b,
            (HeapValue::Temporal(TemporalData::DateTime(a)), HeapValue::Temporal(TemporalData::DateTime(b))) => a == b,
            (HeapValue::Temporal(TemporalData::Duration(a)), HeapValue::Temporal(TemporalData::Duration(b))) => a == b,
            (HeapValue::Temporal(TemporalData::TimeSpan(a)), HeapValue::Temporal(TemporalData::TimeSpan(b))) => a == b,
            (HeapValue::Temporal(TemporalData::Timeframe(a)), HeapValue::Temporal(TemporalData::Timeframe(b))) => a == b,
            (HeapValue::NativeScalar(a), HeapValue::NativeScalar(b)) => a == b,
            (HeapValue::NativeView(a), HeapValue::NativeView(b)) => native_view_eq(a, b),
            (HeapValue::TypedArray(TypedArrayData::I64(a)), HeapValue::TypedArray(TypedArrayData::I64(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::F64(a)), HeapValue::TypedArray(TypedArrayData::F64(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::I64(a)), HeapValue::TypedArray(TypedArrayData::F64(b))) => int_float_array_eq(a, b),
            (HeapValue::TypedArray(TypedArrayData::F64(a)), HeapValue::TypedArray(TypedArrayData::I64(b))) => int_float_array_eq(b, a),
            (HeapValue::TypedArray(TypedArrayData::Bool(a)), HeapValue::TypedArray(TypedArrayData::Bool(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::I8(a)), HeapValue::TypedArray(TypedArrayData::I8(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::I16(a)), HeapValue::TypedArray(TypedArrayData::I16(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::I32(a)), HeapValue::TypedArray(TypedArrayData::I32(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::U8(a)), HeapValue::TypedArray(TypedArrayData::U8(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::U16(a)), HeapValue::TypedArray(TypedArrayData::U16(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::U32(a)), HeapValue::TypedArray(TypedArrayData::U32(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::U64(a)), HeapValue::TypedArray(TypedArrayData::U64(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::F32(a)), HeapValue::TypedArray(TypedArrayData::F32(b))) => a == b,
            (HeapValue::TypedArray(TypedArrayData::Matrix(a)), HeapValue::TypedArray(TypedArrayData::Matrix(b))) => matrix_eq(a, b),
            (
                HeapValue::TypedArray(TypedArrayData::FloatSlice { parent: p1, offset: o1, len: l1 }),
                HeapValue::TypedArray(TypedArrayData::FloatSlice { parent: p2, offset: o2, len: l2 }),
            ) => {
                let s1 = &p1.data[*o1 as usize..(*o1 + *l1) as usize];
                let s2 = &p2.data[*o2 as usize..(*o2 + *l2) as usize];
                s1 == s2
            }
            // Cross-type numeric
            (HeapValue::NativeScalar(a), HeapValue::BigInt(b)) => native_scalar_bigint_eq(a, b),
            (HeapValue::BigInt(a), HeapValue::NativeScalar(b)) => native_scalar_bigint_eq(b, a),
            (HeapValue::NativeScalar(a), HeapValue::Decimal(b)) => {
                native_scalar_decimal_eq(a, b)
            }
            (HeapValue::Decimal(a), HeapValue::NativeScalar(b)) => {
                native_scalar_decimal_eq(b, a)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod closure_variant_regression {
    //! N2 — pin Track A.5's deletion of the legacy `HeapValue::Closure`
    //! variant. The `HeapKind::Closure` ordinal is preserved for ABI
    //! stability and must continue to map to the `ClosureRaw` pipeline.
    use super::*;

    #[test]
    fn heap_kind_closure_ordinal_stable() {
        // HeapKind::Closure = 3 per the ordinal table in heap_variants.rs.
        assert_eq!(HeapKind::Closure as u8, 3);
    }
}

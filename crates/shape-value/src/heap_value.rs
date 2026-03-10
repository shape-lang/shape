//! Compact heap-allocated value types for ValueWord TAG_HEAP.
//!
//! `HeapValue` is the heap backing store for ValueWord. Every type that cannot
//! be stored inline in the ValueWord 8-byte encoding gets a dedicated HeapValue variant.
//!
//! The enum definition, `HeapKind` discriminant, `kind()`, `is_truthy()`, and
//! `type_name()` are all generated from the single source of truth in
//! `define_heap_types!` (see `heap_variants.rs`).
//!
//! `equals()` and `structural_eq()` remain hand-written because they have
//! complex per-variant logic (e.g. cross-type numeric comparison).

use crate::aligned_vec::AlignedVec;
use crate::value_word::ValueWord;
use chrono::{DateTime, FixedOffset};
use shape_ast::data::Timeframe;
use std::collections::HashMap;
use std::sync::Arc;

// ── Collection type data structures ─────────────────────────────────────────

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
}

/// Lazy iterator state — supports chained transforms without materializing intermediates.
#[derive(Debug, Clone)]
pub struct IteratorState {
    pub source: ValueWord,
    pub position: usize,
    pub transforms: Vec<IteratorTransform>,
    pub done: bool,
}

/// A lazy transform in an iterator chain.
#[derive(Debug, Clone)]
pub enum IteratorTransform {
    Map(ValueWord),
    Filter(ValueWord),
    Take(usize),
    Skip(usize),
    FlatMap(ValueWord),
}

/// Generator function state machine.
#[derive(Debug, Clone)]
pub struct GeneratorState {
    pub function_id: u16,
    pub state: u16,
    pub locals: Box<[ValueWord]>,
    pub result: Option<Box<ValueWord>>,
}

/// Data for SimulationCall variant (boxed to keep HeapValue small).
#[derive(Debug, Clone)]
pub struct SimulationCallData {
    pub name: String,
    pub params: HashMap<String, ValueWord>,
}

/// Data for DataReference variant (boxed to keep HeapValue small).
#[derive(Debug, Clone)]
pub struct DataReferenceData {
    pub datetime: DateTime<FixedOffset>,
    pub id: String,
    pub timeframe: Timeframe,
}

/// A projection applied to a base reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefProjection {
    TypedField {
        type_id: u16,
        field_idx: u16,
        field_type_tag: u16,
    },
}

/// Heap-backed projected reference data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedRefData {
    pub base: ValueWord,
    pub projection: RefProjection,
}

/// Data for HashMap variant (boxed to keep HeapValue small).
///
/// Uses bucket chaining (`HashMap<u64, Vec<usize>>`) so that hash collisions
/// are handled correctly — each bucket stores all indices whose key hashes
/// to the same u64.
#[derive(Debug, Clone)]
pub struct HashMapData {
    pub keys: Vec<ValueWord>,
    pub values: Vec<ValueWord>,
    pub index: HashMap<u64, Vec<usize>>,
    /// Optional shape (hidden class) for O(1) index-based access.
    /// None means "dictionary mode" (fallback to hash-based lookup).
    pub shape_id: Option<crate::shape_graph::ShapeId>,
}

impl HashMapData {
    /// Look up the index of `key` in this HashMap, returning `Some(idx)` if found.
    #[inline]
    pub fn find_key(&self, key: &ValueWord) -> Option<usize> {
        let hash = key.vw_hash();
        let bucket = self.index.get(&hash)?;
        bucket
            .iter()
            .copied()
            .find(|&idx| self.keys[idx].vw_equals(key))
    }

    /// Build a bucketed index from the keys vector.
    pub fn rebuild_index(keys: &[ValueWord]) -> HashMap<u64, Vec<usize>> {
        let mut index: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, k) in keys.iter().enumerate() {
            index.entry(k.vw_hash()).or_default().push(i);
        }
        index
    }

    /// Compute a ShapeId for this HashMap if all keys are strings and count <= 64.
    ///
    /// Returns `None` (dictionary mode) if any key is non-string or there are
    /// more than 64 properties.
    pub fn compute_shape(keys: &[ValueWord]) -> Option<crate::shape_graph::ShapeId> {
        if keys.is_empty() || keys.len() > 64 {
            return None;
        }
        let mut key_hashes = Vec::with_capacity(keys.len());
        for k in keys {
            if let Some(s) = k.as_str() {
                key_hashes.push(crate::shape_graph::hash_property_name(s));
            } else {
                return None; // Non-string key → dictionary mode
            }
        }
        crate::shape_graph::shape_for_hashmap_keys(&key_hashes)
    }

    /// Look up a string property using the shape for O(1) index-based access.
    ///
    /// Returns the value at the shape-determined index, or `None` if this
    /// HashMap has no shape or the property isn't in the shape.
    #[inline]
    pub fn shape_get(&self, property: &str) -> Option<&ValueWord> {
        let shape_id = self.shape_id?;
        let prop_hash = crate::shape_graph::hash_property_name(property);
        let idx = crate::shape_graph::shape_property_index(shape_id, prop_hash)?;
        self.values.get(idx)
    }
}

/// Data for Set variant (boxed to keep HeapValue small).
///
/// Uses bucket chaining for collision-safe O(1) membership tests.
#[derive(Debug, Clone)]
pub struct SetData {
    pub items: Vec<ValueWord>,
    pub index: HashMap<u64, Vec<usize>>,
}

impl SetData {
    /// Check if the set contains the given item.
    #[inline]
    pub fn contains(&self, item: &ValueWord) -> bool {
        let hash = item.vw_hash();
        if let Some(bucket) = self.index.get(&hash) {
            bucket.iter().any(|&idx| self.items[idx].vw_equals(item))
        } else {
            false
        }
    }

    /// Add an item to the set. Returns true if the item was newly inserted.
    pub fn insert(&mut self, item: ValueWord) -> bool {
        if self.contains(&item) {
            return false;
        }
        let hash = item.vw_hash();
        let idx = self.items.len();
        self.items.push(item);
        self.index.entry(hash).or_default().push(idx);
        true
    }

    /// Remove an item from the set. Returns true if the item was present.
    pub fn remove(&mut self, item: &ValueWord) -> bool {
        let hash = item.vw_hash();
        if let Some(bucket) = self.index.get(&hash) {
            if let Some(&idx) = bucket.iter().find(|&&idx| self.items[idx].vw_equals(item)) {
                self.items.swap_remove(idx);
                self.rebuild_index_from_items();
                return true;
            }
        }
        false
    }

    /// Build a bucketed index from the items vector.
    pub fn rebuild_index(items: &[ValueWord]) -> HashMap<u64, Vec<usize>> {
        let mut index: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, k) in items.iter().enumerate() {
            index.entry(k.vw_hash()).or_default().push(i);
        }
        index
    }

    fn rebuild_index_from_items(&mut self) {
        self.index = Self::rebuild_index(&self.items);
    }

    /// Create from items, deduplicating.
    pub fn from_items(items: Vec<ValueWord>) -> Self {
        let mut set = SetData {
            items: Vec::with_capacity(items.len()),
            index: HashMap::new(),
        };
        for item in items {
            set.insert(item);
        }
        set
    }
}

/// Data for PriorityQueue variant — binary min-heap.
///
/// Items are ordered by their numeric value (via `as_number_coerce()`).
/// For non-numeric items, insertion order is preserved as a FIFO fallback.
#[derive(Debug, Clone)]
pub struct PriorityQueueData {
    pub items: Vec<ValueWord>,
}

impl PriorityQueueData {
    pub fn new() -> Self {
        PriorityQueueData { items: Vec::new() }
    }

    pub fn from_items(mut items: Vec<ValueWord>) -> Self {
        let mut pq = PriorityQueueData { items };
        pq.heapify();
        pq
    }

    /// Compare two ValueWords for heap ordering (min-heap).
    /// Returns Ordering::Less if a should be higher priority (closer to root).
    #[inline]
    fn cmp_items(a: &ValueWord, b: &ValueWord) -> std::cmp::Ordering {
        match (a.as_number_coerce(), b.as_number_coerce()) {
            (Some(fa), Some(fb)) => fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => {
                // Fall back to string comparison
                let sa = format!("{}", a);
                let sb = format!("{}", b);
                sa.cmp(&sb)
            }
        }
    }

    /// Push an item and sift up to maintain heap invariant.
    pub fn push(&mut self, item: ValueWord) {
        self.items.push(item);
        self.sift_up(self.items.len() - 1);
    }

    /// Pop the minimum item (root) and restore heap invariant.
    pub fn pop(&mut self) -> Option<ValueWord> {
        if self.items.is_empty() {
            return None;
        }
        let last = self.items.len() - 1;
        self.items.swap(0, last);
        let result = self.items.pop();
        if !self.items.is_empty() {
            self.sift_down(0);
        }
        result
    }

    /// Peek at the minimum item without removing.
    pub fn peek(&self) -> Option<&ValueWord> {
        self.items.first()
    }

    fn sift_up(&mut self, mut idx: usize) {
        while idx > 0 {
            let parent = (idx - 1) / 2;
            if Self::cmp_items(&self.items[idx], &self.items[parent]) == std::cmp::Ordering::Less {
                self.items.swap(idx, parent);
                idx = parent;
            } else {
                break;
            }
        }
    }

    fn sift_down(&mut self, mut idx: usize) {
        let len = self.items.len();
        loop {
            let left = 2 * idx + 1;
            let right = 2 * idx + 2;
            let mut smallest = idx;

            if left < len
                && Self::cmp_items(&self.items[left], &self.items[smallest])
                    == std::cmp::Ordering::Less
            {
                smallest = left;
            }
            if right < len
                && Self::cmp_items(&self.items[right], &self.items[smallest])
                    == std::cmp::Ordering::Less
            {
                smallest = right;
            }

            if smallest != idx {
                self.items.swap(idx, smallest);
                idx = smallest;
            } else {
                break;
            }
        }
    }

    fn heapify(&mut self) {
        if self.items.len() <= 1 {
            return;
        }
        for i in (0..self.items.len() / 2).rev() {
            self.sift_down(i);
        }
    }
}

/// Data for Deque variant — double-ended queue backed by VecDeque.
#[derive(Debug, Clone)]
pub struct DequeData {
    pub items: std::collections::VecDeque<ValueWord>,
}

impl DequeData {
    pub fn new() -> Self {
        DequeData {
            items: std::collections::VecDeque::new(),
        }
    }

    pub fn from_items(items: Vec<ValueWord>) -> Self {
        DequeData {
            items: items.into(),
        }
    }
}

/// Width-aware native scalar for C interop.
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

// ── Concurrency primitive data structures ────────────────────────────────────

/// Interior-mutable concurrent wrapper. Only type (besides Atomic/Lazy) with
/// interior mutability — `&Mutex<T>` can mutate the inner value via `lock()`.
#[derive(Debug, Clone)]
pub struct MutexData {
    pub inner: Arc<std::sync::Mutex<ValueWord>>,
}

impl MutexData {
    pub fn new(value: ValueWord) -> Self {
        Self {
            inner: Arc::new(std::sync::Mutex::new(value)),
        }
    }
}

/// Atomic integer for lock-free concurrent access.
/// Only supports integer values (load/store/compare_exchange).
#[derive(Debug, Clone)]
pub struct AtomicData {
    pub inner: Arc<std::sync::atomic::AtomicI64>,
}

impl AtomicData {
    pub fn new(value: i64) -> Self {
        Self {
            inner: Arc::new(std::sync::atomic::AtomicI64::new(value)),
        }
    }
}

/// Lazy-initialized value — compute once, cache forever.
/// The initializer closure runs at most once; subsequent accesses return the cached result.
#[derive(Debug, Clone)]
pub struct LazyData {
    /// Closure that produces the value (None after initialization).
    pub initializer: Arc<std::sync::Mutex<Option<ValueWord>>>,
    /// Cached result (None until first access).
    pub value: Arc<std::sync::Mutex<Option<ValueWord>>>,
}

impl LazyData {
    pub fn new(initializer: ValueWord) -> Self {
        Self {
            initializer: Arc::new(std::sync::Mutex::new(Some(initializer))),
            value: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Check if the value has been initialized.
    pub fn is_initialized(&self) -> bool {
        self.value.lock().map(|g| g.is_some()).unwrap_or(false)
    }
}

/// MPSC channel endpoint (sender or receiver).
///
/// A `Channel()` call creates a sender/receiver pair. Both share the same
/// underlying `mpsc::channel`. The sender can be cloned (multi-producer),
/// while the receiver is wrapped in a Mutex for shared access.
#[derive(Debug, Clone)]
pub enum ChannelData {
    Sender {
        tx: Arc<std::sync::mpsc::Sender<ValueWord>>,
        closed: Arc<std::sync::atomic::AtomicBool>,
    },
    Receiver {
        rx: Arc<std::sync::Mutex<std::sync::mpsc::Receiver<ValueWord>>>,
        closed: Arc<std::sync::atomic::AtomicBool>,
    },
}

impl ChannelData {
    /// Create a new sender/receiver pair.
    pub fn new_pair() -> (Self, Self) {
        let (tx, rx) = std::sync::mpsc::channel();
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        (
            ChannelData::Sender {
                tx: Arc::new(tx),
                closed: closed.clone(),
            },
            ChannelData::Receiver {
                rx: Arc::new(std::sync::Mutex::new(rx)),
                closed,
            },
        )
    }

    /// Check if the channel is closed.
    pub fn is_closed(&self) -> bool {
        match self {
            ChannelData::Sender { closed, .. } | ChannelData::Receiver { closed, .. } => {
                closed.load(std::sync::atomic::Ordering::Relaxed)
            }
        }
    }

    /// Close the channel.
    pub fn close(&self) {
        match self {
            ChannelData::Sender { closed, .. } | ChannelData::Receiver { closed, .. } => {
                closed.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    /// Whether this is the sender end.
    pub fn is_sender(&self) -> bool {
        matches!(self, ChannelData::Sender { .. })
    }
}

// ── Generate HeapValue, HeapKind, kind(), is_truthy(), type_name() ──────────
//
// All generated from the single source of truth in define_heap_types!().
crate::define_heap_types!();

// ── Hand-written methods (complex per-variant logic) ────────────────────────

impl HeapValue {
    /// Structural equality comparison for HeapValue.
    ///
    /// Used by ValueWord::PartialEq when two heap-tagged values have different
    /// Arc pointers but may contain equal data.
    pub fn structural_eq(&self, other: &HeapValue) -> bool {
        match (self, other) {
            (HeapValue::Char(a), HeapValue::Char(b)) => a == b,
            (HeapValue::String(a), HeapValue::String(b)) => a == b,
            (HeapValue::Array(a), HeapValue::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x == y)
            }
            (HeapValue::Decimal(a), HeapValue::Decimal(b)) => a == b,
            (HeapValue::BigInt(a), HeapValue::BigInt(b)) => a == b,
            (HeapValue::Some(a), HeapValue::Some(b)) => a == b,
            (HeapValue::Ok(a), HeapValue::Ok(b)) => a == b,
            (HeapValue::Err(a), HeapValue::Err(b)) => a == b,
            (HeapValue::NativeScalar(a), HeapValue::NativeScalar(b)) => a == b,
            (HeapValue::NativeView(a), HeapValue::NativeView(b)) => {
                a.ptr == b.ptr && a.mutable == b.mutable && a.layout.name == b.layout.name
            }
            (HeapValue::Mutex(a), HeapValue::Mutex(b)) => Arc::ptr_eq(&a.inner, &b.inner),
            (HeapValue::Atomic(a), HeapValue::Atomic(b)) => Arc::ptr_eq(&a.inner, &b.inner),
            (HeapValue::Lazy(a), HeapValue::Lazy(b)) => Arc::ptr_eq(&a.value, &b.value),
            (HeapValue::Future(a), HeapValue::Future(b)) => a == b,
            (HeapValue::ExprProxy(a), HeapValue::ExprProxy(b)) => a == b,
            (HeapValue::Time(a), HeapValue::Time(b)) => a == b,
            (HeapValue::HashMap(d1), HeapValue::HashMap(d2)) => {
                d1.keys.len() == d2.keys.len()
                    && d1.keys.iter().zip(d2.keys.iter()).all(|(a, b)| a == b)
                    && d1.values.iter().zip(d2.values.iter()).all(|(a, b)| a == b)
            }
            (HeapValue::Set(s1), HeapValue::Set(s2)) => {
                s1.items.len() == s2.items.len() && s1.items.iter().all(|item| s2.contains(item))
            }
            (HeapValue::Content(a), HeapValue::Content(b)) => a == b,
            (HeapValue::Instant(a), HeapValue::Instant(b)) => a == b,
            (HeapValue::IoHandle(a), HeapValue::IoHandle(b)) => {
                std::sync::Arc::ptr_eq(&a.resource, &b.resource)
            }
            (HeapValue::IntArray(a), HeapValue::IntArray(b)) => a == b,
            (HeapValue::FloatArray(a), HeapValue::FloatArray(b)) => a == b,
            (HeapValue::IntArray(a), HeapValue::FloatArray(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| (*x as f64) == *y)
            }
            (HeapValue::FloatArray(a), HeapValue::IntArray(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| *x == (*y as f64))
            }
            (HeapValue::BoolArray(a), HeapValue::BoolArray(b)) => a == b,
            (HeapValue::I8Array(a), HeapValue::I8Array(b)) => a == b,
            (HeapValue::I16Array(a), HeapValue::I16Array(b)) => a == b,
            (HeapValue::I32Array(a), HeapValue::I32Array(b)) => a == b,
            (HeapValue::U8Array(a), HeapValue::U8Array(b)) => a == b,
            (HeapValue::U16Array(a), HeapValue::U16Array(b)) => a == b,
            (HeapValue::U32Array(a), HeapValue::U32Array(b)) => a == b,
            (HeapValue::U64Array(a), HeapValue::U64Array(b)) => a == b,
            (HeapValue::F32Array(a), HeapValue::F32Array(b)) => a == b,
            (HeapValue::Matrix(a), HeapValue::Matrix(b)) => {
                a.rows == b.rows
                    && a.cols == b.cols
                    && a.data.len() == b.data.len()
                    && a.data.iter().zip(b.data.iter()).all(|(x, y)| x == y)
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
            (HeapValue::Array(a), HeapValue::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.vw_equals(y))
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
                    let is_heap = (hm1 & (1u64 << i)) != 0;
                    if is_heap {
                        // Deep compare heap values (strings, arrays, objects, etc.)
                        let a_nb = sl1[i].as_heap_nb();
                        let b_nb = sl2[i].as_heap_nb();
                        if !a_nb.vw_equals(&b_nb) {
                            return false;
                        }
                    } else {
                        // Raw bit compare for primitives (f64, i64, bool)
                        if sl1[i].raw() != sl2[i].raw() {
                            return false;
                        }
                    }
                }
                true
            }
            (
                HeapValue::Closure {
                    function_id: f1, ..
                },
                HeapValue::Closure {
                    function_id: f2, ..
                },
            ) => f1 == f2,
            (HeapValue::Decimal(a), HeapValue::Decimal(b)) => a == b,
            (HeapValue::BigInt(a), HeapValue::BigInt(b)) => a == b,
            (HeapValue::BigInt(a), HeapValue::Decimal(b)) => rust_decimal::Decimal::from(*a) == *b,
            (HeapValue::Decimal(a), HeapValue::BigInt(b)) => *a == rust_decimal::Decimal::from(*b),
            (HeapValue::DataTable(a), HeapValue::DataTable(b)) => Arc::ptr_eq(a, b),
            (
                HeapValue::TypedTable {
                    schema_id: s1,
                    table: t1,
                },
                HeapValue::TypedTable {
                    schema_id: s2,
                    table: t2,
                },
            ) => s1 == s2 && Arc::ptr_eq(t1, t2),
            (
                HeapValue::RowView {
                    schema_id: s1,
                    row_idx: r1,
                    table: t1,
                },
                HeapValue::RowView {
                    schema_id: s2,
                    row_idx: r2,
                    table: t2,
                },
            ) => s1 == s2 && r1 == r2 && Arc::ptr_eq(t1, t2),
            (
                HeapValue::ColumnRef {
                    schema_id: s1,
                    col_id: c1,
                    table: t1,
                },
                HeapValue::ColumnRef {
                    schema_id: s2,
                    col_id: c2,
                    table: t2,
                },
            ) => s1 == s2 && c1 == c2 && Arc::ptr_eq(t1, t2),
            (
                HeapValue::IndexedTable {
                    schema_id: s1,
                    index_col: c1,
                    table: t1,
                },
                HeapValue::IndexedTable {
                    schema_id: s2,
                    index_col: c2,
                    table: t2,
                },
            ) => s1 == s2 && c1 == c2 && Arc::ptr_eq(t1, t2),
            (HeapValue::HashMap(d1), HeapValue::HashMap(d2)) => {
                d1.keys.len() == d2.keys.len()
                    && d1
                        .keys
                        .iter()
                        .zip(d2.keys.iter())
                        .all(|(a, b)| a.vw_equals(b))
                    && d1
                        .values
                        .iter()
                        .zip(d2.values.iter())
                        .all(|(a, b)| a.vw_equals(b))
            }
            (HeapValue::Set(s1), HeapValue::Set(s2)) => {
                s1.items.len() == s2.items.len() && s1.items.iter().all(|item| s2.contains(item))
            }
            (HeapValue::Content(a), HeapValue::Content(b)) => a == b,
            (HeapValue::Instant(a), HeapValue::Instant(b)) => a == b,
            (HeapValue::IoHandle(a), HeapValue::IoHandle(b)) => {
                Arc::ptr_eq(&a.resource, &b.resource)
            }
            (HeapValue::Mutex(a), HeapValue::Mutex(b)) => Arc::ptr_eq(&a.inner, &b.inner),
            (HeapValue::Atomic(a), HeapValue::Atomic(b)) => Arc::ptr_eq(&a.inner, &b.inner),
            (HeapValue::Lazy(a), HeapValue::Lazy(b)) => Arc::ptr_eq(&a.value, &b.value),
            (HeapValue::Range { .. }, HeapValue::Range { .. }) => false,
            (HeapValue::Enum(a), HeapValue::Enum(b)) => {
                a.enum_name == b.enum_name && a.variant == b.variant
            }
            (HeapValue::Some(a), HeapValue::Some(b)) => a.vw_equals(b),
            (HeapValue::Ok(a), HeapValue::Ok(b)) => a.vw_equals(b),
            (HeapValue::Err(a), HeapValue::Err(b)) => a.vw_equals(b),
            (HeapValue::Future(a), HeapValue::Future(b)) => a == b,
            (HeapValue::Time(a), HeapValue::Time(b)) => a == b,
            (HeapValue::Duration(a), HeapValue::Duration(b)) => a == b,
            (HeapValue::TimeSpan(a), HeapValue::TimeSpan(b)) => a == b,
            (HeapValue::Timeframe(a), HeapValue::Timeframe(b)) => a == b,
            (HeapValue::FunctionRef { name: n1, .. }, HeapValue::FunctionRef { name: n2, .. }) => {
                n1 == n2
            }
            (HeapValue::ProjectedRef(a), HeapValue::ProjectedRef(b)) => a == b,
            (HeapValue::DataReference(a), HeapValue::DataReference(b)) => {
                a.datetime == b.datetime && a.id == b.id && a.timeframe == b.timeframe
            }
            (HeapValue::NativeScalar(a), HeapValue::NativeScalar(b)) => a == b,
            (HeapValue::NativeView(a), HeapValue::NativeView(b)) => {
                a.ptr == b.ptr && a.mutable == b.mutable && a.layout.name == b.layout.name
            }
            (HeapValue::IntArray(a), HeapValue::IntArray(b)) => a == b,
            (HeapValue::FloatArray(a), HeapValue::FloatArray(b)) => a == b,
            (HeapValue::IntArray(a), HeapValue::FloatArray(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| (*x as f64) == *y)
            }
            (HeapValue::FloatArray(a), HeapValue::IntArray(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| *x == (*y as f64))
            }
            (HeapValue::BoolArray(a), HeapValue::BoolArray(b)) => a == b,
            (HeapValue::I8Array(a), HeapValue::I8Array(b)) => a == b,
            (HeapValue::I16Array(a), HeapValue::I16Array(b)) => a == b,
            (HeapValue::I32Array(a), HeapValue::I32Array(b)) => a == b,
            (HeapValue::U8Array(a), HeapValue::U8Array(b)) => a == b,
            (HeapValue::U16Array(a), HeapValue::U16Array(b)) => a == b,
            (HeapValue::U32Array(a), HeapValue::U32Array(b)) => a == b,
            (HeapValue::U64Array(a), HeapValue::U64Array(b)) => a == b,
            (HeapValue::F32Array(a), HeapValue::F32Array(b)) => a == b,
            (HeapValue::Matrix(a), HeapValue::Matrix(b)) => {
                a.rows == b.rows
                    && a.cols == b.cols
                    && a.data.len() == b.data.len()
                    && a.data.iter().zip(b.data.iter()).all(|(x, y)| x == y)
            }
            // Cross-type numeric
            (HeapValue::NativeScalar(a), HeapValue::BigInt(b)) => {
                a.as_i64().is_some_and(|v| v == *b)
            }
            (HeapValue::BigInt(a), HeapValue::NativeScalar(b)) => {
                b.as_i64().is_some_and(|v| *a == v)
            }
            (HeapValue::NativeScalar(a), HeapValue::Decimal(b)) => match a {
                NativeScalar::F32(v) => {
                    rust_decimal::Decimal::from_f64_retain(*v as f64).is_some_and(|v| v == *b)
                }
                _ => a
                    .as_i128()
                    .map(|n| rust_decimal::Decimal::from_i128_with_scale(n, 0))
                    .is_some_and(|to_dec| to_dec == *b),
            },
            (HeapValue::Decimal(a), HeapValue::NativeScalar(b)) => match b {
                NativeScalar::F32(v) => {
                    rust_decimal::Decimal::from_f64_retain(*v as f64).is_some_and(|v| *a == v)
                }
                _ => b
                    .as_i128()
                    .map(|n| rust_decimal::Decimal::from_i128_with_scale(n, 0))
                    .is_some_and(|to_dec| *a == to_dec),
            },
            _ => false,
        }
    }
}

//! Single source of truth for HeapValue variants.
//!
//! `define_heap_types!` generates:
//! - `HeapValue` enum
//! - `HeapKind` enum (discriminant)
//! - `HeapValue::kind()` method
//! - `HeapValue::is_truthy()` method
//! - `HeapValue::type_name()` method
//!
//! `equals()`, `structural_eq()`, and `Display` remain hand-written because
//! they have complex per-variant logic (e.g. cross-type numeric comparison).
//!
//! Strict-typing bulldozer (Phase 2): every variant whose payload depended on
//! the deleted `ValueWord` (the v1 dynamic-tag word) has been removed from
//! `HeapValue`, along with the supporting `*Data` structs. The `HeapKind`
//! discriminator preserves its ordinal numbering for ABI stability — gone
//! variants now appear only in `HeapKind` (reserved). Heterogeneous-element
//! collections (`HashMap`, `Set`, `Deque`, `PriorityQueue`), dynamic
//! single-value wrappers (`Some`/`Ok`/`Err`/`Range`/`TraitObject`/
//! `FunctionRef`), and dynamic capture/control-flow holders (`Iterator`,
//! `Generator`, `Concurrency`, `Rare`, `Enum`, `Array`, `HostClosure`,
//! `ProjectedRef`) are awaiting monomorphized typed replacements per
//! `docs/runtime-v2-spec.md`.

/// All HeapValue variant data lives here as a single source of truth.
///
/// Because Rust macro hygiene makes it impossible to use identifiers across
/// macro boundaries (the `_v` in a pattern introduced by one macro cannot be
/// referenced by tokens captured from a different call site), we define both
/// the variant table AND the dispatch expressions in the SAME macro.
///
/// `define_heap_types!` takes no arguments — the variant table is embedded.
/// The public types and `impl` blocks are generated inside the expansion.
///
/// Callers import this via `crate::define_heap_types!()`.
#[macro_export]
macro_rules! define_heap_types {
    () => {
        /// Discriminator for HeapValue variants, usable without full pattern match.
        ///
        /// The discriminant order is ABI-stable. New variants MUST be appended
        /// at the end. Variants marked "(removed)" no longer have a matching
        /// `HeapValue` arm — the ordinal is reserved.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(u8)]
        pub enum HeapKind {
            String,             // 0
            Array,              // 1  (removed — heterogeneous-element array; use TypedArray)
            TypedObject,        // 2
            Closure,            // 3
            Decimal,            // 4
            BigInt,             // 5
            HostClosure,        // 6  (removed — dynamic FFI; awaiting monomorphized signature)
            DataTable,          // 7
            TypedTable,         // 8  (deprecated — use TableView)
            RowView,            // 9  (deprecated — use TableView)
            ColumnRef,          // 10 (deprecated — use TableView)
            IndexedTable,       // 11 (deprecated — use TableView)
            Range,              // 12 (removed — dynamic Range payload; awaiting monomorphized Range<T>)
            Enum,               // 13 (removed — heterogeneous payload; awaiting per-variant TypedStruct)
            Some,               // 14 (removed — dynamic Option payload; awaiting monomorphized Option<T>)
            Ok,                 // 15 (removed — dynamic Result payload; awaiting monomorphized Result<T,E>)
            Err,                // 16 (removed — dynamic Result payload; awaiting monomorphized Result<T,E>)
            Future,             // 17
            TaskGroup,          // 18
            TraitObject,        // 19 (removed — dynamic value payload; awaiting typed redesign)
            ExprProxy,          // 20 (deprecated — use Rare)
            FilterExpr,         // 21 (deprecated — use Rare)
            Time,               // 22 (deprecated — use Temporal)
            Duration,           // 23 (deprecated — use Temporal)
            TimeSpan,           // 24 (deprecated — use Temporal)
            Timeframe,          // 25 (deprecated — use Temporal)
            TimeReference,      // 26 (deprecated — use Temporal)
            DateTimeExpr,       // 27 (deprecated — use Temporal)
            DataDateTimeRef,    // 28 (deprecated — use Temporal)
            TypeAnnotation,     // 29 (deprecated — use Rare)
            TypeAnnotatedValue, // 30 (deprecated — use Rare)
            PrintResult,        // 31 (deprecated — use Rare)
            SimulationCall,     // 32 (deprecated — use Rare)
            FunctionRef,        // 33 (removed — dynamic closure payload)
            DataReference,      // 34 (deprecated — use Rare)
            Number,             // 35
            Bool,               // 36
            None,               // 37
            Unit,               // 38
            Function,           // 39
            ModuleFunction,     // 40
            HashMap,            // 41 (removed — heterogeneous-keyed; awaiting monomorphized typed buckets)
            Content,            // 42
            Instant,            // 43
            IoHandle,           // 44
            SharedCell,         // 45 (deprecated — retired in Track A.1C.3)
            NativeScalar,       // 46
            NativeView,         // 47
            IntArray,           // 48 (deprecated — use TypedArray)
            FloatArray,         // 49 (deprecated — use TypedArray)
            BoolArray,          // 50 (deprecated — use TypedArray)
            Matrix,             // 51 (deprecated — use TypedArray)
            Iterator,           // 52 (removed — dynamic capture/source; awaiting typed iterator design)
            Generator,          // 53 (removed — dynamic state)
            Mutex,              // 54 (deprecated — use Concurrency)
            Atomic,             // 55 (deprecated — use Concurrency)
            Lazy,               // 56 (deprecated — use Concurrency)
            I8Array,            // 57 (deprecated — use TypedArray)
            I16Array,           // 58 (deprecated — use TypedArray)
            I32Array,           // 59 (deprecated — use TypedArray)
            U8Array,            // 60 (deprecated — use TypedArray)
            U16Array,           // 61 (deprecated — use TypedArray)
            U32Array,           // 62 (deprecated — use TypedArray)
            U64Array,           // 63 (deprecated — use TypedArray)
            F32Array,           // 64 (deprecated — use TypedArray)
            Set,                // 65 (removed — heterogeneous-element)
            Deque,              // 66 (removed — heterogeneous-element)
            PriorityQueue,      // 67 (removed — heterogeneous-element)
            Channel,            // 68 (deprecated — use Concurrency)
            Char,               // 69
            ProjectedRef,       // 70 (removed — dynamic index)
            FloatArraySlice,    // 71 (deprecated — use TypedArray)
            // ===== New consolidated ordinals =====
            TypedArray,         // 72
            Temporal,           // 73
            Rare,               // 74 (removed — held ValueWord-bearing TypeAnnotatedValue)
            Concurrency,        // 75 (removed — Mutex/Lazy/Channel held ValueWord)
            TableView,          // 76
        }

        /// Compact heap-allocated value. Strict-typed variants only — every
        /// payload is either a typed primitive (`i64`, `char`, `f64` via
        /// `TypedArray`), a typed structure (`TypedObject` slots, typed FFI
        /// pointers, typed temporal data), or a typed handle.
        ///
        /// Variants whose payloads depended on the deleted `ValueWord`
        /// dynamic word were removed in the strict-typing Phase-2 bulldozer.
        /// See the corresponding `HeapKind` ordinals (annotated "(removed)")
        /// for the migration target.
        #[derive(Debug)]
        pub enum HeapValue {
            // ===== Typed primitives =====
            String(std::sync::Arc<String>),
            Decimal(rust_decimal::Decimal),
            BigInt(i64),
            Future(u64),
            Char(char),
            // ===== Typed handles / data stores =====
            DataTable(std::sync::Arc<$crate::datatable::DataTable>),
            Content(Box<$crate::content::ContentNode>),
            Instant(Box<std::time::Instant>),
            IoHandle(Box<$crate::heap_value::IoHandleData>),
            NativeScalar($crate::heap_value::NativeScalar),
            NativeView(Box<$crate::heap_value::NativeViewData>),
            // ===== Struct variants =====
            /// Object value with schema-defined typed slots.
            TypedObject {
                schema_id: u64,
                slots: Box<[$crate::slot::ValueSlot]>,
                heap_mask: u64,
            },
            /// Track A.5 — the canonical closure representation.
            ///
            /// Raw `TypedClosureHeader`-backed closure. Captures live in a
            /// typed C-laid-out block allocated by
            /// `closure_raw::alloc_typed_closure` and owned by the embedded
            /// [`OwnedClosureBlock`]. Cloning / dropping this variant
            /// manages the block's refcount in lockstep with the enclosing
            /// `Arc<HeapValue>`. Readers go through `VmClosureHandle` for
            /// the stable read API. There is no legacy fallback.
            ClosureRaw($crate::v2::closure_raw::OwnedClosureBlock),
            TaskGroup {
                kind: u8,
                task_ids: Vec<u64>,
            },
            // ===== Consolidated wrapper variants =====
            TypedArray($crate::heap_value::TypedArrayData),
            Temporal($crate::heap_value::TemporalData),
            TableView($crate::heap_value::TableViewData),
        }

        impl HeapValue {
            /// Get the kind discriminator for fast dispatch without full pattern matching.
            #[inline]
            pub fn kind(&self) -> HeapKind {
                match self {
                    HeapValue::String(..) => HeapKind::String,
                    HeapValue::Decimal(..) => HeapKind::Decimal,
                    HeapValue::BigInt(..) => HeapKind::BigInt,
                    HeapValue::Future(..) => HeapKind::Future,
                    HeapValue::Char(..) => HeapKind::Char,
                    HeapValue::DataTable(..) => HeapKind::DataTable,
                    HeapValue::Content(..) => HeapKind::Content,
                    HeapValue::Instant(..) => HeapKind::Instant,
                    HeapValue::IoHandle(..) => HeapKind::IoHandle,
                    HeapValue::NativeScalar(..) => HeapKind::NativeScalar,
                    HeapValue::NativeView(..) => HeapKind::NativeView,
                    HeapValue::TypedObject { .. } => HeapKind::TypedObject,
                    HeapValue::ClosureRaw(..) => HeapKind::Closure,
                    HeapValue::TaskGroup { .. } => HeapKind::TaskGroup,
                    HeapValue::TypedArray(..) => HeapKind::TypedArray,
                    HeapValue::Temporal(..) => HeapKind::Temporal,
                    HeapValue::TableView(..) => HeapKind::TableView,
                }
            }

            /// Check if this heap value is truthy.
            #[inline]
            pub fn is_truthy(&self) -> bool {
                match self {
                    HeapValue::String(_v) => !_v.is_empty(),
                    HeapValue::Decimal(_v) => !_v.is_zero(),
                    HeapValue::BigInt(_v) => *_v != 0,
                    HeapValue::Future(_) => true,
                    HeapValue::Char(_) => true,
                    HeapValue::DataTable(_v) => _v.row_count() > 0,
                    HeapValue::Content(_) => true,
                    HeapValue::Instant(_) => true,
                    HeapValue::IoHandle(_v) => _v.is_open(),
                    HeapValue::NativeScalar(_v) => _v.is_truthy(),
                    HeapValue::NativeView(_v) => _v.ptr != 0,
                    HeapValue::TypedObject { slots, .. } => !slots.is_empty(),
                    HeapValue::ClosureRaw(..) => true,
                    HeapValue::TaskGroup { .. } => true,
                    HeapValue::TypedArray(ta) => ta.is_truthy(),
                    HeapValue::Temporal(td) => td.is_truthy(),
                    HeapValue::TableView(tv) => tv.is_truthy(),
                }
            }

            /// Get the type name for this heap value.
            #[inline]
            pub fn type_name(&self) -> &'static str {
                match self {
                    HeapValue::String(_) => "string",
                    HeapValue::Decimal(_) => "decimal",
                    HeapValue::BigInt(_) => "int",
                    HeapValue::Future(_) => "future",
                    HeapValue::Char(_) => "char",
                    HeapValue::DataTable(_) => "datatable",
                    HeapValue::Content(_) => "content",
                    HeapValue::Instant(_) => "instant",
                    HeapValue::IoHandle(_) => "io_handle",
                    HeapValue::NativeScalar(v) => v.type_name(),
                    HeapValue::NativeView(v) => {
                        if v.mutable {
                            "cmut"
                        } else {
                            "cview"
                        }
                    }
                    HeapValue::TypedObject { .. } => "object",
                    HeapValue::ClosureRaw(..) => "closure",
                    HeapValue::TaskGroup { .. } => "task_group",
                    HeapValue::TypedArray(ta) => ta.type_name(),
                    HeapValue::Temporal(td) => td.type_name(),
                    HeapValue::TableView(tv) => tv.type_name(),
                }
            }
        }
    };
}

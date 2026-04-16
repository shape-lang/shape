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
        /// The discriminant order is ABI-stable (checked by tests in tags.rs).
        /// New variants MUST be appended at the end.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(u8)]
        pub enum HeapKind {
            String,             // 0
            Array,              // 1
            TypedObject,        // 2
            Closure,            // 3
            Decimal,            // 4
            BigInt,             // 5
            HostClosure,        // 6
            DataTable,          // 7
            TypedTable,         // 8  (deprecated — use TableView)
            RowView,            // 9  (deprecated — use TableView)
            ColumnRef,          // 10 (deprecated — use TableView)
            IndexedTable,       // 11 (deprecated — use TableView)
            Range,              // 12
            Enum,               // 13
            Some,               // 14
            Ok,                 // 15
            Err,                // 16
            Future,             // 17
            TaskGroup,          // 18
            TraitObject,        // 19
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
            FunctionRef,        // 33
            DataReference,      // 34 (deprecated — use Rare)
            Number,             // 35
            Bool,               // 36
            None,               // 37
            Unit,               // 38
            Function,           // 39
            ModuleFunction,     // 40
            HashMap,            // 41
            Content,            // 42
            Instant,            // 43
            IoHandle,           // 44
            SharedCell,         // 45
            NativeScalar,       // 46
            NativeView,         // 47
            IntArray,           // 48 (deprecated — use TypedArray)
            FloatArray,         // 49 (deprecated — use TypedArray)
            BoolArray,          // 50 (deprecated — use TypedArray)
            Matrix,             // 51 (deprecated — use TypedArray)
            Iterator,           // 52
            Generator,          // 53
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
            Set,                // 65
            Deque,              // 66
            PriorityQueue,      // 67
            Channel,            // 68 (deprecated — use Concurrency)
            Char,               // 69
            ProjectedRef,       // 70
            FloatArraySlice,    // 71 (deprecated — use TypedArray)
            // ===== New consolidated ordinals =====
            TypedArray,         // 72
            Temporal,           // 73
            Rare,               // 74
            Concurrency,        // 75
            TableView,          // 76
        }

        /// Compact heap-allocated value for ValueWord TAG_HEAP.
        ///
        /// Every type that cannot be inlined in ValueWord has a dedicated variant here.
        /// Inline ValueWord types (f64, i48, bool, None, Unit, Function, ModuleFunction)
        /// are never stored in HeapValue.
        #[derive(Debug, Clone)]
        pub enum HeapValue {
            // ===== Tuple variants =====
            String(std::sync::Arc<String>),
            Array($crate::value::VMArray),
            Decimal(rust_decimal::Decimal),
            BigInt(i64),
            HostClosure($crate::value::HostCallable),
            DataTable(std::sync::Arc<$crate::datatable::DataTable>),
            HashMap(Box<$crate::heap_value::HashMapData>),
            Set(Box<$crate::heap_value::SetData>),
            Deque(Box<$crate::heap_value::DequeData>),
            PriorityQueue(Box<$crate::heap_value::PriorityQueueData>),
            Content(Box<$crate::content::ContentNode>),
            Instant(Box<std::time::Instant>),
            IoHandle(Box<$crate::heap_value::IoHandleData>),
            Enum(Box<$crate::enums::EnumValue>),
            Some(Box<$crate::value_word::ValueWord>),
            Ok(Box<$crate::value_word::ValueWord>),
            Err(Box<$crate::value_word::ValueWord>),
            Future(u64),
            // NOTE: Number(f64), Bool(bool), Function(u16), ModuleFunction(usize) — REMOVED.
            // These were shadow variants duplicating inline ValueWord tags.
            // HeapKind ordinals 35-40 are reserved (ABI stability).
            NativeScalar($crate::heap_value::NativeScalar),
            NativeView(Box<$crate::heap_value::NativeViewData>),
            Iterator(Box<$crate::heap_value::IteratorState>),
            Generator(Box<$crate::heap_value::GeneratorState>),
            Char(char),
            ProjectedRef(Box<$crate::heap_value::ProjectedRefData>),
            // ===== Struct variants =====
            TypedObject {
                schema_id: u64,
                slots: Box<[$crate::slot::ValueSlot]>,
                heap_mask: u64,
            },
            Closure {
                function_id: u16,
                upvalues: Vec<$crate::value::Upvalue>,
            },
            Range {
                start: Option<Box<$crate::value_word::ValueWord>>,
                end: Option<Box<$crate::value_word::ValueWord>>,
                inclusive: bool,
            },
            TaskGroup {
                kind: u8,
                task_ids: Vec<u64>,
            },
            TraitObject {
                value: Box<$crate::value_word::ValueWord>,
                vtable: std::sync::Arc<$crate::value::VTable>,
            },
            FunctionRef {
                name: String,
                closure: Option<Box<$crate::value_word::ValueWord>>,
            },
            // ===== Shared mutable cell for closure capture =====
            SharedCell(std::sync::Arc<std::sync::RwLock<$crate::value_word::ValueWord>>),
            // NOTE: None and Unit unit variants — REMOVED.
            // These were shadow variants duplicating inline ValueWord tags.
            // HeapKind ordinals 37-38 are reserved (ABI stability).
            // ===== Consolidated wrapper variants =====
            TypedArray($crate::heap_value::TypedArrayData),
            Temporal($crate::heap_value::TemporalData),
            Rare($crate::heap_value::RareHeapData),
            Concurrency($crate::heap_value::ConcurrencyData),
            TableView($crate::heap_value::TableViewData),
        }

        impl HeapValue {
            /// Get the kind discriminator for fast dispatch without full pattern matching.
            #[inline]
            pub fn kind(&self) -> HeapKind {
                match self {
                    // Tuple
                    HeapValue::String(..) => HeapKind::String,
                    HeapValue::Array(..) => HeapKind::Array,
                    HeapValue::Decimal(..) => HeapKind::Decimal,
                    HeapValue::BigInt(..) => HeapKind::BigInt,
                    HeapValue::HostClosure(..) => HeapKind::HostClosure,
                    HeapValue::DataTable(..) => HeapKind::DataTable,
                    HeapValue::HashMap(..) => HeapKind::HashMap,
                    HeapValue::Set(..) => HeapKind::Set,
                    HeapValue::Deque(..) => HeapKind::Deque,
                    HeapValue::PriorityQueue(..) => HeapKind::PriorityQueue,
                    HeapValue::Content(..) => HeapKind::Content,
                    HeapValue::Instant(..) => HeapKind::Instant,
                    HeapValue::IoHandle(..) => HeapKind::IoHandle,
                    HeapValue::NativeScalar(..) => HeapKind::NativeScalar,
                    HeapValue::NativeView(..) => HeapKind::NativeView,
                    HeapValue::Iterator(..) => HeapKind::Iterator,
                    HeapValue::Generator(..) => HeapKind::Generator,
                    HeapValue::Char(..) => HeapKind::Char,
                    HeapValue::ProjectedRef(..) => HeapKind::ProjectedRef,
                    HeapValue::Enum(..) => HeapKind::Enum,
                    HeapValue::Some(..) => HeapKind::Some,
                    HeapValue::Ok(..) => HeapKind::Ok,
                    HeapValue::Err(..) => HeapKind::Err,
                    HeapValue::Future(..) => HeapKind::Future,
                    // Struct
                    HeapValue::TypedObject { .. } => HeapKind::TypedObject,
                    HeapValue::Closure { .. } => HeapKind::Closure,
                    HeapValue::Range { .. } => HeapKind::Range,
                    HeapValue::TaskGroup { .. } => HeapKind::TaskGroup,
                    HeapValue::TraitObject { .. } => HeapKind::TraitObject,
                    HeapValue::FunctionRef { .. } => HeapKind::FunctionRef,
                    // SharedCell
                    HeapValue::SharedCell(..) => HeapKind::SharedCell,
                    // Consolidated
                    HeapValue::TypedArray(..) => HeapKind::TypedArray,
                    HeapValue::Temporal(..) => HeapKind::Temporal,
                    HeapValue::Rare(..) => HeapKind::Rare,
                    HeapValue::Concurrency(..) => HeapKind::Concurrency,
                    HeapValue::TableView(..) => HeapKind::TableView,
                }
            }

            /// Check if this heap value is truthy.
            #[inline]
            pub fn is_truthy(&self) -> bool {
                match self {
                    HeapValue::String(_v) => !_v.is_empty(),
                    HeapValue::Array(_v) => !_v.is_empty(),
                    HeapValue::Decimal(_v) => !_v.is_zero(),
                    HeapValue::BigInt(_v) => *_v != 0,
                    HeapValue::HostClosure(_) => true,
                    HeapValue::DataTable(_v) => _v.row_count() > 0,
                    HeapValue::HashMap(_v) => !_v.keys.is_empty(),
                    HeapValue::Set(_v) => !_v.items.is_empty(),
                    HeapValue::Deque(_v) => !_v.items.is_empty(),
                    HeapValue::PriorityQueue(_v) => !_v.items.is_empty(),
                    HeapValue::Content(_) => true,
                    HeapValue::Instant(_) => true,
                    HeapValue::IoHandle(_v) => _v.is_open(),
                    HeapValue::NativeScalar(_v) => _v.is_truthy(),
                    HeapValue::NativeView(_v) => _v.ptr != 0,
                    HeapValue::Iterator(_v) => !_v.done,
                    HeapValue::Generator(_v) => _v.state != u16::MAX,
                    HeapValue::Char(_) => true,
                    HeapValue::ProjectedRef(_) => true,
                    HeapValue::Enum(_) => true,
                    HeapValue::Some(_) => true,
                    HeapValue::Ok(_) => true,
                    HeapValue::Err(_) => false,
                    HeapValue::Future(_) => true,
                    // Struct
                    HeapValue::TypedObject { slots, .. } => !slots.is_empty(),
                    HeapValue::Closure { .. } => true,
                    HeapValue::Range { .. } => true,
                    HeapValue::TaskGroup { .. } => true,
                    HeapValue::TraitObject { value, .. } => value.is_truthy(),
                    HeapValue::FunctionRef { .. } => true,
                    // SharedCell
                    HeapValue::SharedCell(arc) => arc.read().unwrap().is_truthy(),
                    // Consolidated — delegate to inner enum
                    HeapValue::TypedArray(ta) => ta.is_truthy(),
                    HeapValue::Temporal(td) => td.is_truthy(),
                    HeapValue::Rare(rd) => rd.is_truthy(),
                    HeapValue::Concurrency(cd) => cd.is_truthy(),
                    HeapValue::TableView(tv) => tv.is_truthy(),
                }
            }

            /// Get the type name for this heap value.
            #[inline]
            pub fn type_name(&self) -> &'static str {
                match self {
                    HeapValue::String(_) => "string",
                    HeapValue::Array(_) => "array",
                    HeapValue::Decimal(_) => "decimal",
                    HeapValue::BigInt(_) => "int",
                    HeapValue::HostClosure(_) => "host_closure",
                    HeapValue::DataTable(_) => "datatable",
                    HeapValue::HashMap(_) => "hashmap",
                    HeapValue::Set(_) => "set",
                    HeapValue::Deque(_) => "deque",
                    HeapValue::PriorityQueue(_) => "priority_queue",
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
                    HeapValue::Iterator(_) => "iterator",
                    HeapValue::Generator(_) => "generator",
                    HeapValue::Char(_) => "char",
                    HeapValue::ProjectedRef(_) => "reference",
                    HeapValue::Enum(_) => "enum",
                    HeapValue::Some(_) => "option",
                    HeapValue::Ok(_) => "result",
                    HeapValue::Err(_) => "result",
                    HeapValue::Future(_) => "future",
                    // Struct
                    HeapValue::TypedObject { .. } => "object",
                    HeapValue::Closure { .. } => "closure",
                    HeapValue::Range { .. } => "range",
                    HeapValue::TaskGroup { .. } => "task_group",
                    HeapValue::TraitObject { .. } => "trait_object",
                    HeapValue::FunctionRef { .. } => "function",
                    // SharedCell
                    HeapValue::SharedCell(_) => "shared_cell",
                    // Consolidated — delegate to inner enum
                    HeapValue::TypedArray(ta) => ta.type_name(),
                    HeapValue::Temporal(td) => td.type_name(),
                    HeapValue::Rare(rd) => rd.type_name(),
                    HeapValue::Concurrency(cd) => cd.type_name(),
                    HeapValue::TableView(tv) => tv.type_name(),
                }
            }
        }
    };
}

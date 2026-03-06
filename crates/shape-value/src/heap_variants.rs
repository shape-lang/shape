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
            TypedTable,         // 8
            RowView,            // 9
            ColumnRef,          // 10
            IndexedTable,       // 11
            Range,              // 12
            Enum,               // 13
            Some,               // 14
            Ok,                 // 15
            Err,                // 16
            Future,             // 17
            TaskGroup,          // 18
            TraitObject,        // 19
            ExprProxy,          // 20
            FilterExpr,         // 21
            Time,               // 22
            Duration,           // 23
            TimeSpan,           // 24
            Timeframe,          // 25
            TimeReference,      // 26
            DateTimeExpr,       // 27
            DataDateTimeRef,    // 28
            TypeAnnotation,     // 29
            TypeAnnotatedValue, // 30
            PrintResult,        // 31
            SimulationCall,     // 32
            FunctionRef,        // 33
            DataReference,      // 34
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
            IntArray,           // 48
            FloatArray,         // 49
            BoolArray,          // 50
            Matrix,             // 51
            Iterator,           // 52
            Generator,          // 53
            Mutex,              // 54
            Atomic,             // 55
            Lazy,               // 56
            I8Array,            // 57
            I16Array,           // 58
            I32Array,           // 59
            U8Array,            // 60
            U16Array,           // 61
            U32Array,           // 62
            U64Array,           // 63
            F32Array,           // 64
            Set,                // 65
            Deque,              // 66
            PriorityQueue,      // 67
            Channel,            // 68
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
            ExprProxy(std::sync::Arc<String>),
            FilterExpr(std::sync::Arc<$crate::value::FilterNode>),
            Time(chrono::DateTime<chrono::FixedOffset>),
            Duration(shape_ast::ast::Duration),
            TimeSpan(chrono::Duration),
            Timeframe(shape_ast::data::Timeframe),
            // NOTE: Number(f64), Bool(bool), Function(u16), ModuleFunction(usize) — REMOVED.
            // These were shadow variants duplicating inline ValueWord tags.
            // HeapKind ordinals 35-40 are reserved (ABI stability).
            TimeReference(Box<shape_ast::ast::TimeReference>),
            DateTimeExpr(Box<shape_ast::ast::DateTimeExpr>),
            DataDateTimeRef(Box<shape_ast::ast::DataDateTimeRef>),
            TypeAnnotation(Box<shape_ast::ast::TypeAnnotation>),
            PrintResult(Box<$crate::value::PrintResult>),
            SimulationCall(Box<$crate::heap_value::SimulationCallData>),
            DataReference(Box<$crate::heap_value::DataReferenceData>),
            NativeScalar($crate::heap_value::NativeScalar),
            NativeView(Box<$crate::heap_value::NativeViewData>),
            // ===== Typed collection variants =====
            IntArray(std::sync::Arc<$crate::typed_buffer::TypedBuffer<i64>>),
            FloatArray(std::sync::Arc<$crate::typed_buffer::AlignedTypedBuffer>),
            BoolArray(std::sync::Arc<$crate::typed_buffer::TypedBuffer<u8>>),
            Matrix(Box<$crate::heap_value::MatrixData>),
            // ===== Width-specific typed arrays =====
            I8Array(std::sync::Arc<$crate::typed_buffer::TypedBuffer<i8>>),
            I16Array(std::sync::Arc<$crate::typed_buffer::TypedBuffer<i16>>),
            I32Array(std::sync::Arc<$crate::typed_buffer::TypedBuffer<i32>>),
            U8Array(std::sync::Arc<$crate::typed_buffer::TypedBuffer<u8>>),
            U16Array(std::sync::Arc<$crate::typed_buffer::TypedBuffer<u16>>),
            U32Array(std::sync::Arc<$crate::typed_buffer::TypedBuffer<u32>>),
            U64Array(std::sync::Arc<$crate::typed_buffer::TypedBuffer<u64>>),
            F32Array(std::sync::Arc<$crate::typed_buffer::TypedBuffer<f32>>),
            Iterator(Box<$crate::heap_value::IteratorState>),
            Generator(Box<$crate::heap_value::GeneratorState>),
            // ===== Concurrency primitives =====
            Mutex(Box<$crate::heap_value::MutexData>),
            Atomic(Box<$crate::heap_value::AtomicData>),
            Lazy(Box<$crate::heap_value::LazyData>),
            Channel(Box<$crate::heap_value::ChannelData>),
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
            TypedTable {
                schema_id: u64,
                table: std::sync::Arc<$crate::datatable::DataTable>,
            },
            RowView {
                schema_id: u64,
                table: std::sync::Arc<$crate::datatable::DataTable>,
                row_idx: usize,
            },
            ColumnRef {
                schema_id: u64,
                table: std::sync::Arc<$crate::datatable::DataTable>,
                col_id: u32,
            },
            IndexedTable {
                schema_id: u64,
                table: std::sync::Arc<$crate::datatable::DataTable>,
                index_col: u32,
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
            TypeAnnotatedValue {
                type_name: String,
                value: Box<$crate::value_word::ValueWord>,
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
                    HeapValue::IntArray(..) => HeapKind::IntArray,
                    HeapValue::FloatArray(..) => HeapKind::FloatArray,
                    HeapValue::BoolArray(..) => HeapKind::BoolArray,
                    HeapValue::Matrix(..) => HeapKind::Matrix,
                    HeapValue::Iterator(..) => HeapKind::Iterator,
                    HeapValue::Generator(..) => HeapKind::Generator,
                    HeapValue::Mutex(..) => HeapKind::Mutex,
                    HeapValue::Atomic(..) => HeapKind::Atomic,
                    HeapValue::Lazy(..) => HeapKind::Lazy,
                    HeapValue::Channel(..) => HeapKind::Channel,
                    HeapValue::I8Array(..) => HeapKind::I8Array,
                    HeapValue::I16Array(..) => HeapKind::I16Array,
                    HeapValue::I32Array(..) => HeapKind::I32Array,
                    HeapValue::U8Array(..) => HeapKind::U8Array,
                    HeapValue::U16Array(..) => HeapKind::U16Array,
                    HeapValue::U32Array(..) => HeapKind::U32Array,
                    HeapValue::U64Array(..) => HeapKind::U64Array,
                    HeapValue::F32Array(..) => HeapKind::F32Array,
                    HeapValue::Enum(..) => HeapKind::Enum,
                    HeapValue::Some(..) => HeapKind::Some,
                    HeapValue::Ok(..) => HeapKind::Ok,
                    HeapValue::Err(..) => HeapKind::Err,
                    HeapValue::Future(..) => HeapKind::Future,
                    HeapValue::ExprProxy(..) => HeapKind::ExprProxy,
                    HeapValue::FilterExpr(..) => HeapKind::FilterExpr,
                    HeapValue::Time(..) => HeapKind::Time,
                    HeapValue::Duration(..) => HeapKind::Duration,
                    HeapValue::TimeSpan(..) => HeapKind::TimeSpan,
                    HeapValue::Timeframe(..) => HeapKind::Timeframe,
                    HeapValue::TimeReference(..) => HeapKind::TimeReference,
                    HeapValue::DateTimeExpr(..) => HeapKind::DateTimeExpr,
                    HeapValue::DataDateTimeRef(..) => HeapKind::DataDateTimeRef,
                    HeapValue::TypeAnnotation(..) => HeapKind::TypeAnnotation,
                    HeapValue::PrintResult(..) => HeapKind::PrintResult,
                    HeapValue::SimulationCall(..) => HeapKind::SimulationCall,
                    HeapValue::DataReference(..) => HeapKind::DataReference,
                    // Struct
                    HeapValue::TypedObject { .. } => HeapKind::TypedObject,
                    HeapValue::Closure { .. } => HeapKind::Closure,
                    HeapValue::TypedTable { .. } => HeapKind::TypedTable,
                    HeapValue::RowView { .. } => HeapKind::RowView,
                    HeapValue::ColumnRef { .. } => HeapKind::ColumnRef,
                    HeapValue::IndexedTable { .. } => HeapKind::IndexedTable,
                    HeapValue::Range { .. } => HeapKind::Range,
                    HeapValue::TaskGroup { .. } => HeapKind::TaskGroup,
                    HeapValue::TraitObject { .. } => HeapKind::TraitObject,
                    HeapValue::TypeAnnotatedValue { .. } => HeapKind::TypeAnnotatedValue,
                    HeapValue::FunctionRef { .. } => HeapKind::FunctionRef,
                    // SharedCell
                    HeapValue::SharedCell(..) => HeapKind::SharedCell,
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
                    HeapValue::IntArray(_v) => !_v.is_empty(),
                    HeapValue::FloatArray(_v) => !_v.is_empty(),
                    HeapValue::BoolArray(_v) => !_v.is_empty(),
                    HeapValue::I8Array(_v) => !_v.is_empty(),
                    HeapValue::I16Array(_v) => !_v.is_empty(),
                    HeapValue::I32Array(_v) => !_v.is_empty(),
                    HeapValue::U8Array(_v) => !_v.is_empty(),
                    HeapValue::U16Array(_v) => !_v.is_empty(),
                    HeapValue::U32Array(_v) => !_v.is_empty(),
                    HeapValue::U64Array(_v) => !_v.is_empty(),
                    HeapValue::F32Array(_v) => !_v.is_empty(),
                    HeapValue::Matrix(_v) => _v.data.len() > 0,
                    HeapValue::Iterator(_v) => !_v.done,
                    HeapValue::Generator(_v) => _v.state != u16::MAX,
                    HeapValue::Mutex(_) => true,
                    HeapValue::Atomic(_v) => {
                        _v.inner.load(std::sync::atomic::Ordering::Relaxed) != 0
                    }
                    HeapValue::Lazy(_v) => _v.is_initialized(),
                    HeapValue::Channel(_v) => !_v.is_closed(),
                    HeapValue::Enum(_) => true,
                    HeapValue::Some(_) => true,
                    HeapValue::Ok(_) => true,
                    HeapValue::Err(_) => false,
                    HeapValue::Future(_) => true,
                    HeapValue::ExprProxy(_) => true,
                    HeapValue::FilterExpr(_) => true,
                    HeapValue::Time(_) => true,
                    HeapValue::Duration(_) => true,
                    HeapValue::TimeSpan(_) => true,
                    HeapValue::Timeframe(_) => true,
                    HeapValue::TimeReference(_) => true,
                    HeapValue::DateTimeExpr(_) => true,
                    HeapValue::DataDateTimeRef(_) => true,
                    HeapValue::TypeAnnotation(_) => true,
                    HeapValue::PrintResult(_) => true,
                    HeapValue::SimulationCall(_) => true,
                    HeapValue::DataReference(_) => true,
                    // Struct
                    HeapValue::TypedObject { slots, .. } => !slots.is_empty(),
                    HeapValue::Closure { .. } => true,
                    HeapValue::TypedTable { table, .. } => table.row_count() > 0,
                    HeapValue::RowView { .. } => true,
                    HeapValue::ColumnRef { .. } => true,
                    HeapValue::IndexedTable { table, .. } => table.row_count() > 0,
                    HeapValue::Range { .. } => true,
                    HeapValue::TaskGroup { .. } => true,
                    HeapValue::TraitObject { value, .. } => value.is_truthy(),
                    HeapValue::TypeAnnotatedValue { value, .. } => value.is_truthy(),
                    HeapValue::FunctionRef { .. } => true,
                    // SharedCell
                    HeapValue::SharedCell(arc) => arc.read().unwrap().is_truthy(),
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
                    HeapValue::IntArray(_) => "Vec<int>",
                    HeapValue::FloatArray(_) => "Vec<number>",
                    HeapValue::BoolArray(_) => "Vec<bool>",
                    HeapValue::I8Array(_) => "Vec<i8>",
                    HeapValue::I16Array(_) => "Vec<i16>",
                    HeapValue::I32Array(_) => "Vec<i32>",
                    HeapValue::U8Array(_) => "Vec<u8>",
                    HeapValue::U16Array(_) => "Vec<u16>",
                    HeapValue::U32Array(_) => "Vec<u32>",
                    HeapValue::U64Array(_) => "Vec<u64>",
                    HeapValue::F32Array(_) => "Vec<f32>",
                    HeapValue::Matrix(_) => "Mat<number>",
                    HeapValue::Iterator(_) => "iterator",
                    HeapValue::Generator(_) => "generator",
                    HeapValue::Mutex(_) => "mutex",
                    HeapValue::Atomic(_) => "atomic",
                    HeapValue::Lazy(_) => "lazy",
                    HeapValue::Channel(_) => "channel",
                    HeapValue::Enum(_) => "enum",
                    HeapValue::Some(_) => "option",
                    HeapValue::Ok(_) => "result",
                    HeapValue::Err(_) => "result",
                    HeapValue::Future(_) => "future",
                    HeapValue::ExprProxy(_) => "expr_proxy",
                    HeapValue::FilterExpr(_) => "filter_expr",
                    HeapValue::Time(_) => "time",
                    HeapValue::Duration(_) => "duration",
                    HeapValue::TimeSpan(_) => "timespan",
                    HeapValue::Timeframe(_) => "timeframe",
                    HeapValue::TimeReference(_) => "time_reference",
                    HeapValue::DateTimeExpr(_) => "datetime_expr",
                    HeapValue::DataDateTimeRef(_) => "data_datetime_ref",
                    HeapValue::TypeAnnotation(_) => "type_annotation",
                    HeapValue::PrintResult(_) => "print_result",
                    HeapValue::SimulationCall(_) => "simulation_call",
                    HeapValue::DataReference(_) => "data_reference",
                    // Struct
                    HeapValue::TypedObject { .. } => "object",
                    HeapValue::Closure { .. } => "closure",
                    HeapValue::TypedTable { .. } => "typed_table",
                    HeapValue::RowView { .. } => "row",
                    HeapValue::ColumnRef { .. } => "column",
                    HeapValue::IndexedTable { .. } => "indexed_table",
                    HeapValue::Range { .. } => "range",
                    HeapValue::TaskGroup { .. } => "task_group",
                    HeapValue::TraitObject { .. } => "trait_object",
                    HeapValue::TypeAnnotatedValue { value, .. } => value.type_name(),
                    HeapValue::FunctionRef { .. } => "function",
                    // SharedCell
                    HeapValue::SharedCell(_) => "shared_cell",
                }
            }
        }
    };
}

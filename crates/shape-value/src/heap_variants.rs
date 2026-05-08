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
        /// One variant per surviving `HeapValue` arm — no dead variants
        /// expressible. Trimmed in Phase 2b alongside the
        /// `NativeKind::Ptr(HeapKind)` extension; see
        /// `docs/defections.md` 2026-05-06 (HeapKind trim +
        /// NativeKind::Ptr extension) for the audit findings and rejected
        /// alternatives.
        ///
        /// The previous 77-variant surface (with `(removed)` /
        /// `(deprecated)` annotations) preserved ordinals "for ABI
        /// stability"; the bulldozer deleted the `tags.rs`
        /// ordinal-stability test that made that contract load-bearing,
        /// so the dead variants no longer had a justification to
        /// remain in the source.
        ///
        // ADR-005: HeapKind is the canonical heap-shape discriminator.
        // Layers above HeapValue take Arc<HeapValue> and dispatch on
        // HeapValue::kind() rather than introducing parallel discriminators.
        // See docs/adr/005-typed-slot-construction.md.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ::serde::Serialize, ::serde::Deserialize)]
        #[repr(u8)]
        pub enum HeapKind {
            String,        // 0
            TypedObject,   // 1
            Closure,       // 2  (matches HeapValue::ClosureRaw via the Closure ordinal)
            Decimal,       // 3
            BigInt,        // 4
            DataTable,     // 5
            Future,        // 6
            TaskGroup,     // 7
            TypedArray,    // 8
            Temporal,      // 9
            TableView,     // 10
            Content,       // 11
            Instant,       // 12
            IoHandle,      // 13
            NativeScalar,  // 14
            NativeView,    // 15
            Char,          // 16
            HashMap,       // 17  (Stage C P1(b), 2026-05-07)
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
        ///
        // ADR-005: HeapValue is the single discriminator for heap-resident
        // values. New variants are added here when a new heap shape is
        // genuinely required; layers above (ConcreteReturn, TypedFieldValue,
        // marshal helpers, JIT FFI carriers) must NOT introduce parallel
        // sum types whose variants project 1:1 to HeapKind. The single
        // explicit exception is `TypedFieldValue::String(Arc<String>)`, named
        // and bounded in ADR-005. See docs/adr/005-typed-slot-construction.md.
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
            IoHandle(std::sync::Arc<$crate::heap_value::IoHandleData>),
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
            // ===== Stage C HashMap-marshal P1(b) =====
            /// HashMap with string keys + heap-allocated values.
            /// Two-buffer storage reusing Phase 2d Array shapes (keys via
            /// `TypedArrayData::String`-equivalent buffer; values via
            /// `TypedArrayData::HeapValue`-equivalent buffer) plus an eager
            /// bucket-index for O(1) `map.get(key)`. See
            /// `$crate::heap_value::HashMapData` for the storage shape.
            /// Stage C P1(b), 2026-05-07.
            HashMap(std::sync::Arc<$crate::heap_value::HashMapData>),
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
                    HeapValue::HashMap(..) => HeapKind::HashMap,
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
                    HeapValue::HashMap(d) => !d.is_empty(),
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
                    HeapValue::HashMap(_) => "hashmap",
                }
            }
        }
    };
}

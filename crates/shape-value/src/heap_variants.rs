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
            // Pure-discriminator variant — no corresponding `HeapValue` arm
            // (FilterExpr payloads live as `Arc<FilterNode>` directly in slot
            // bits, never wrapped in `HeapValue`). Added to fix the
            // type-confusion soundness gap surfaced by Wave-α D-raw-helpers
            // (commit `a27c0e4`): the filter-expression branch of
            // `executor/logical/mod.rs` previously reused
            // `HeapKind::NativeView` to label `Arc<FilterNode>` payloads,
            // and the runtime `clone_with_kind` / `drop_with_kind` tables
            // dispatched the same label as `Arc<NativeViewData>` —
            // wrong-type retain/release. ADR-006 §2.3 / §2.7.6 / Q8
            // amendment (Wave-γ G-heap-filter-expr).
            FilterExpr,    // 18  (Wave-γ G-heap-filter-expr, 2026-05-09)
            // ADR-006 §2.7.13 / Q14 (Wave 8 W8-T26, 2026-05-10):
            // Reference-value carrier — replaces the deleted
            // `nanboxed::RefTarget` / `RefProjection` `ValueWord`-shaped
            // enum. Slot bits are `Arc::into_raw(Arc<RefTarget>) as u64`
            // directly (mirror of FilterExpr's pure-discriminator-style
            // dispatch — `as_heap_value()` is undefined on
            // Reference-labeled bits; recovery is `Arc::from_raw::
            // <RefTarget>(bits)`). The `clone_with_kind` /
            // `drop_with_kind` dispatch tables retain/release
            // `Arc<RefTarget>` directly, NOT through `HeapValue`. The
            // `HeapValue::Reference` arm below exists ONLY to preserve
            // the ADR-005 §1 / ADR-006 §2.3 `HeapKind`↔`HeapValue`
            // symmetry property; no caller materializes a `Reference`
            // through `HeapValue` pattern matching.
            Reference,     // 19  (Wave 8 W8-T26, 2026-05-10)
            // Pure-discriminator variant — no corresponding `HeapValue` arm
            // (`Arc<SharedCell>` cell-pointer slots live as
            // `Arc::into_raw(Arc<SharedCell>) as u64` directly in the
            // kinded stack / module-binding store / cell-storage slots,
            // never wrapped in `HeapValue`). Added so the §2.7.7 / §2.7.8
            // parallel-kind tracks can label `*const SharedCell` slots
            // distinctly — the precondition for unblocking
            // `op_alloc_shared_local` / `op_alloc_shared_module_binding`.
            // Same pure-discriminator role as `HeapKind::FilterExpr`:
            // `as_heap_value()` is unsound on `SharedCell`-labeled bits;
            // heap dispatch goes through the kind label, not through
            // `HeapValue` materialisation. ADR-006 §2.7.12 / Q13 amendment
            // (Wave 8 W8-T25, mirror of §2.7.9 FilterExpr precedent).
            // NOTE: ordinal 20 (not the originally drafted 19) — T26 took
            // 19 first at merge time.
            SharedCell,    // 20  (Wave 8 W8-T25, 2026-05-10)
            // ADR-006 §2.7.15 / Q16 amendment (Wave 13 W13-hashset-rebuild,
            // 2026-05-10): one-keyspace Set carrier, structurally a mirror
            // of the Stage C P1(b) `HeapKind::HashMap(Arc<HashMapData>)`
            // shape with the values buffer dropped. Full HeapValue arm
            // (NOT pure-discriminator like FilterExpr / SharedCell): Set
            // values flow through `slot.as_heap_value()` for receiver
            // classification at method dispatch (`set.add(...)` /
            // `set.has(...)` / `set.union(other)`), can be stored in
            // TypedObject slots and `TypedArrayData::HeapValue` buffers.
            // See §2.7.15 for the full justification + the rejected Path
            // B (`TypedSet<T>` per element kind) alternative.
            HashSet,       // 21  (Wave 13 W13-hashset-rebuild, 2026-05-10)
            // ADR-006 §2.7.16 / Q17 (W13-iterator-state, 2026-05-10):
            // Lazy-iterator carrier — replaces the deleted
            // `heap_value::IteratorState` / `IteratorTransform`
            // `ValueWord`-shaped enums. Slot bits are
            // `Arc::into_raw(Arc<IteratorState>) as u64`; unlike
            // FilterExpr / Reference / SharedCell, the matching
            // `HeapValue::Iterator(Arc<IteratorState>)` arm IS used at
            // runtime — iterator method handlers recover the typed Arc
            // via `slot.as_heap_value()` per ADR-005 §1
            // single-discriminator. The `clone_with_kind` /
            // `drop_with_kind` dispatch tables retain/release
            // `Arc<IteratorState>` directly, NOT through the
            // `HeapValue` arm (mirror of other typed-Arc
            // refcount-dispatch arms — refcount discipline goes
            // through the kind label).
            Iterator,      // 22  (W13-iterator-state, 2026-05-10)
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
        // ADR-006 §2.3: each heap-resident variant carries a typed
        // `Arc<T>` payload (single atomic refcount bump on clone, single
        // decrement on drop, no `Box<HeapValue>` wrapping). Inline scalars
        // (`Future(u64)`, `Char(char)`, `NativeScalar`) stay inline because
        // their payloads fit in a register and have no heap state.
        // `ClosureRaw` continues to use `OwnedClosureBlock` because that
        // type already manages its own refcount via the v2 typed-closure
        // header. See docs/adr/006-value-and-memory-model.md §2.3.
        #[derive(Debug)]
        pub enum HeapValue {
            // ===== Typed primitives =====
            String(std::sync::Arc<String>),
            // ADR-006 §2.3: `Decimal` is 16 bytes inline; wrapping in `Arc`
            // shrinks the enum payload to a pointer so the slot's clone is
            // a single atomic increment.
            Decimal(std::sync::Arc<rust_decimal::Decimal>),
            // ADR-006 §2.3: `BigInt`'s inner `i64` is the v2 placeholder
            // for an arbitrary-precision integer. Wrapping in `Arc` keeps
            // the variant cheap to clone today and gives the Drop discipline
            // a single typed `Arc::decrement_strong_count::<i64>` site for
            // when the payload widens to a real big-int representation.
            BigInt(std::sync::Arc<i64>),
            // Future-id is an inline scalar — no heap state.
            Future(u64),
            // Char is an inline scalar — no heap state.
            Char(char),
            // ===== Typed handles / data stores =====
            DataTable(std::sync::Arc<$crate::datatable::DataTable>),
            // ADR-006 §2.3: `Box<ContentNode>` migrated to `Arc<ContentNode>`
            // so clones are an atomic refcount bump rather than a deep copy.
            Content(std::sync::Arc<$crate::content::ContentNode>),
            // ADR-006 §2.3: `Box<Instant>` migrated to `Arc<Instant>`. The
            // inner `Instant` is `Copy` but the boxing cost was paid at
            // every clone; the Arc bump replaces it.
            Instant(std::sync::Arc<std::time::Instant>),
            IoHandle(std::sync::Arc<$crate::heap_value::IoHandleData>),
            // NativeScalar is `Copy` and ≤ 16 bytes — kept inline.
            NativeScalar($crate::heap_value::NativeScalar),
            // ADR-006 §2.3: `Box<NativeViewData>` migrated to
            // `Arc<NativeViewData>` to match `from_native_view(Arc<…>)`.
            NativeView(std::sync::Arc<$crate::heap_value::NativeViewData>),
            // ===== Struct variants =====
            /// Object value with schema-defined typed slots.
            ///
            // ADR-006 §2.3: payload is `Arc<TypedObjectStorage>` rather
            // than the previous inline `{ schema_id, slots, heap_mask }`
            // struct. The Drop impl (Step 5) lives on the inner struct
            // and dispatches per-field on `NativeKind` looked up via
            // `schema_id` (Q8 ruling). See
            // docs/adr/006-value-and-memory-model.md.
            TypedObject(std::sync::Arc<$crate::heap_value::TypedObjectStorage>),
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
            // ADR-006 §2.3: `TaskGroup { kind, task_ids }` struct variant
            // collapsed to a single-tuple `Arc<TaskGroupData>` payload so
            // every heap variant follows the typed-Arc shape. Field reads
            // are now `tg.kind` / `tg.task_ids` (Phase 1.B caller migration).
            TaskGroup(std::sync::Arc<$crate::heap_value::TaskGroupData>),
            // ===== Consolidated wrapper variants =====
            // ADR-006 §2.3: the inline `TypedArrayData` enum migrated to
            // `Arc<TypedArrayData>`. Each `TypedArrayData` arm already
            // carries an `Arc<TypedBuffer<T>>` so the outer Arc is a thin
            // refcount over the discriminant + inner buffer Arc — single
            // pointer payload, single atomic clone.
            TypedArray(std::sync::Arc<$crate::heap_value::TypedArrayData>),
            // ADR-006 §2.3: `TemporalData` enum was inline (size = largest
            // variant ≈ 32 bytes including `Box`'s overhead). `Arc` reduces
            // the slot payload to a single pointer.
            Temporal(std::sync::Arc<$crate::heap_value::TemporalData>),
            // ADR-006 §2.3: `TableViewData` enum migrated to `Arc<…>` to
            // match the canonical typed-Arc shape; its arms already carry
            // `Arc<DataTable>` internally.
            TableView(std::sync::Arc<$crate::heap_value::TableViewData>),
            // ===== Stage C HashMap-marshal P1(b) =====
            /// HashMap with string keys + heap-allocated values.
            /// Two-buffer storage reusing Phase 2d Array shapes (keys via
            /// `TypedArrayData::String`-equivalent buffer; values via
            /// `TypedArrayData::HeapValue`-equivalent buffer) plus an eager
            /// bucket-index for O(1) `map.get(key)`. See
            /// `$crate::heap_value::HashMapData` for the storage shape.
            /// Stage C P1(b), 2026-05-07.
            HashMap(std::sync::Arc<$crate::heap_value::HashMapData>),
            // ===== Wave-γ G-heap-filter-expr (2026-05-09) =====
            /// Filter-expression tree (`Arc<FilterNode>`) used by the query
            /// DSL's `And` / `Or` / `Not` opcodes (`executor/logical/mod.rs`).
            /// In current code FilterExpr payloads are emitted directly to
            /// the kinded stack as `Arc::into_raw(Arc<FilterNode>) as u64`
            /// with kind `NativeKind::Ptr(HeapKind::FilterExpr)` and never
            /// wrapped in `HeapValue`. The arm exists to preserve the
            /// ADR-005 §1 invariant that every `HeapKind` discriminator has
            /// a `HeapValue` arm of the same shape — kind() / is_truthy() /
            /// type_name() / drop_with_kind() / clone_with_kind() must
            /// dispatch a `HeapKind::FilterExpr` slot as `Arc<FilterNode>`,
            /// not `Arc<NativeViewData>` (the pre-Wave-γ type-confusion gap
            /// surfaced by Wave-α D-raw-helpers, commit `a27c0e4`).
            FilterExpr(std::sync::Arc<$crate::value::FilterNode>),
            // ===== Wave 13 W13-hashset-rebuild (ADR-006 §2.7.15 / Q16,
            // 2026-05-10) =====
            /// One-keyspace Set carrier. Mirror of
            /// `HeapValue::HashMap(Arc<HashMapData>)` with the values
            /// buffer dropped: insertion-ordered `Arc<TypedBuffer<Arc<
            /// String>>>` keys + eager FNV-1a bucket index for O(1)
            /// `set.has(key)`. See `$crate::heap_value::HashSetData` for
            /// the storage shape.
            ///
            /// Full HeapValue arm (NOT pure-discriminator like FilterExpr
            /// / SharedCell): Set values flow through `as_heap_value()`
            /// for method-receiver classification per the §2.7.15 amendment.
            HashSet(std::sync::Arc<$crate::heap_value::HashSetData>),
            // ===== Wave 8 W8-T26 (ADR-006 §2.7.13 / Q14, 2026-05-10) =====
            /// Reference-value carrier (`Arc<RefTarget>`) used by the
            /// `MakeRef` / `MakeFieldRef` / `MakeIndexRef` /
            /// `DerefLoad` / `DerefStore` / `SetIndexRef` opcode family
            /// (`executor/variables/mod.rs`). In current code Reference
            /// payloads are emitted directly to the kinded stack as
            /// `Arc::into_raw(Arc<RefTarget>) as u64` with kind
            /// `NativeKind::Ptr(HeapKind::Reference)` and never wrapped
            /// in `HeapValue`. The arm exists to preserve the ADR-005
            /// §1 / ADR-006 §2.3 `HeapKind`↔`HeapValue` symmetry — same
            /// pattern as `HeapValue::FilterExpr` (§2.7.9). Calling
            /// `slot.as_heap_value()` on a Reference-labeled slot is
            /// undefined behavior; the canonical recovery is
            /// `Arc::from_raw::<RefTarget>(bits)`.
            Reference(std::sync::Arc<$crate::reference::RefTarget>),
            // ===== W13-iterator-state (ADR-006 §2.7.16 / Q17, 2026-05-10) =====
            /// Lazy iterator pipeline carrier (`Arc<IteratorState>`).
            /// Used by `Array.iter` / `String.iter` / `HashMap.iter` /
            /// `Range.iter` factories and the `ITERATOR_METHODS` PHF
            /// (`map` / `filter` / `take` / `skip` / `flatMap` /
            /// `enumerate` / `chain` / `collect` / `forEach` /
            /// `reduce` / `count` / `any` / `all` / `find`). Slot bits
            /// are `Arc::into_raw(Arc<IteratorState>) as u64`; recovery
            /// goes through `slot.as_heap_value()` →
            /// `HeapValue::Iterator(arc)` per ADR-005 §1
            /// single-discriminator (the lazy-transforms / source /
            /// cursor triple is opaque at the dispatch shell —
            /// terminals walk `arc.transforms` and dispatch per stage).
            Iterator(std::sync::Arc<$crate::iterator_state::IteratorState>),
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
                    HeapValue::TypedObject(..) => HeapKind::TypedObject,
                    HeapValue::ClosureRaw(..) => HeapKind::Closure,
                    HeapValue::TaskGroup(..) => HeapKind::TaskGroup,
                    HeapValue::TypedArray(..) => HeapKind::TypedArray,
                    HeapValue::Temporal(..) => HeapKind::Temporal,
                    HeapValue::TableView(..) => HeapKind::TableView,
                    HeapValue::HashMap(..) => HeapKind::HashMap,
                    HeapValue::HashSet(..) => HeapKind::HashSet,
                    HeapValue::FilterExpr(..) => HeapKind::FilterExpr,
                    HeapValue::Reference(..) => HeapKind::Reference,
                    HeapValue::Iterator(..) => HeapKind::Iterator,
                }
            }

            /// Check if this heap value is truthy.
            #[inline]
            pub fn is_truthy(&self) -> bool {
                match self {
                    HeapValue::String(_v) => !_v.is_empty(),
                    HeapValue::Decimal(_v) => !_v.is_zero(),
                    HeapValue::BigInt(_v) => **_v != 0,
                    HeapValue::Future(_) => true,
                    HeapValue::Char(_) => true,
                    HeapValue::DataTable(_v) => _v.row_count() > 0,
                    HeapValue::Content(_) => true,
                    HeapValue::Instant(_) => true,
                    HeapValue::IoHandle(_v) => _v.is_open(),
                    HeapValue::NativeScalar(_v) => _v.is_truthy(),
                    HeapValue::NativeView(_v) => _v.ptr != 0,
                    HeapValue::TypedObject(s) => !s.slots.is_empty(),
                    HeapValue::ClosureRaw(..) => true,
                    HeapValue::TaskGroup(..) => true,
                    HeapValue::TypedArray(ta) => ta.is_truthy(),
                    HeapValue::Temporal(td) => td.is_truthy(),
                    HeapValue::TableView(tv) => tv.is_truthy(),
                    HeapValue::HashMap(d) => !d.is_empty(),
                    HeapValue::HashSet(d) => !d.is_empty(),
                    // Filter-expression trees are always truthy when present.
                    HeapValue::FilterExpr(_) => true,
                    // Reference values are always truthy when present
                    // (a live ref points at a reachable place).
                    HeapValue::Reference(_) => true,
                    // Iterator values are always truthy when present
                    // (a live iterator is a usable pipeline value, even
                    // if its terminal evaluation will yield zero
                    // elements after filter / take 0 etc.).
                    HeapValue::Iterator(_) => true,
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
                    HeapValue::TypedObject(..) => "object",
                    HeapValue::ClosureRaw(..) => "closure",
                    HeapValue::TaskGroup(..) => "task_group",
                    HeapValue::TypedArray(ta) => ta.type_name(),
                    HeapValue::Temporal(td) => td.type_name(),
                    HeapValue::TableView(tv) => tv.type_name(),
                    HeapValue::HashMap(_) => "hashmap",
                    HeapValue::HashSet(_) => "hashset",
                    HeapValue::FilterExpr(_) => "filter_expr",
                    HeapValue::Reference(_) => "ref",
                    HeapValue::Iterator(_) => "iterator",
                }
            }
        }
    };
}

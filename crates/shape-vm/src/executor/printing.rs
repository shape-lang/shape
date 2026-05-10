//! VM-native value formatting (ADR-006 §2.7.4 — output adapter).
//!
//! Formats runtime values held in a [`KindedSlot`] for `print()` /
//! `format()` / REPL display. The pre-bulldozer implementation keyed off
//! the deleted `ValueWord` carrier and `tag_bits::*` decode helpers; per
//! ADR-006 §2.7.4 the formatter moves to a kinded carrier — `NativeKind`
//! drives inline-scalar dispatch, and heap arms are read via
//! `slot.as_heap_value()` + `HeapValue` match (Q8 ruling, preserves
//! ADR-005 §1 single-discriminator).
//!
//! [`PrintResult`] and [`PrintSpan`] live in `shape_runtime::print_result`
//! per §2.7.4; consumers of this formatter pair its output with those
//! span-metadata carriers when feeding the output adapter.
//!
//! # Phase-1b-vm migration scope (E-printing close)
//!
//! Wave 6.5 / Wave-α `E-printing` ports the formatter SHAPE off the
//! deleted `ValueWord` API: the public surface now takes `&KindedSlot`,
//! dispatches on `NativeKind` for the inline-scalar arms (Int*, UInt*,
//! IntSize, UIntSize, Bool, Float64), and reads heap-bearing kinds via
//! the typed `Arc<T>` payload reconstruction shared with
//! `KindedSlot::Drop` / `clone_with_kind`. Heap arms whose payload
//! formatting depends on Phase-2c surfaces (TypedObject schema lookup,
//! Content rendering, Temporal/DateTime formatting, TableView, Iterator
//! / Generator state) surface as `todo!("phase-2c — see ADR-006
//! §2.7.4")` placeholders rather than papering over with ValueWord-shape
//! recovery, per the playbook's surface-and-stop discipline.

use shape_runtime::type_schema::TypeSchemaRegistry;
use shape_value::heap_value::{
    HashMapData, HeapKind, HeapValue, TypedArrayData, TypedObjectStorage,
};
use shape_value::{KindedSlot, NativeKind, ValueSlot};

// Re-export the runtime-tier `PrintResult`/`PrintSpan` carriers for
// formatter consumers — keeps the post-§2.7.4 import path coherent for
// callers that still reach into `shape_vm::executor::printing` for the
// output-adapter types.
pub use shape_runtime::print_result::{PrintResult, PrintSpan};

/// Formatter for runtime values represented as [`KindedSlot`].
///
/// Uses [`TypeSchemaRegistry`] to format `TypedObject` payloads with
/// their schema-declared field names. Optionally accepts a reference
/// resolver (Phase-2c) that dereferences ref-kind slots to their target
/// values; absent the resolver, refs print as `<ref>`.
///
/// ADR-005 §1 single-discriminator preserved: heap arms are read via
/// the slot's typed pointer + `HeapValue` match (Q8 ruling). No
/// per-heap-variant accessors on `KindedSlot`; no parallel sum types.
pub struct ValueFormatter<'a> {
    /// Type schema registry for `TypedObject` field resolution.
    schema_registry: &'a TypeSchemaRegistry,
    /// Optional reference-resolver hook. Phase-2c wire-up: when present,
    /// `Ref`-kind slots are dereferenced and the target formatted in
    /// their place; when absent, refs print as `<ref>`.
    deref_fn: Option<&'a dyn Fn(&KindedSlot) -> Option<KindedSlot>>,
}

impl<'a> ValueFormatter<'a> {
    /// Create a formatter without a reference resolver — `Ref`-kind
    /// slots will print as `<ref>`.
    pub fn new(schema_registry: &'a TypeSchemaRegistry) -> Self {
        Self {
            schema_registry,
            deref_fn: None,
        }
    }

    /// Create a formatter with a reference resolver. The resolver is
    /// invoked when a ref-kind slot is encountered; if it returns
    /// `Some(target)` the target is formatted in place, otherwise the
    /// ref prints as `<ref>`.
    pub fn with_deref(
        schema_registry: &'a TypeSchemaRegistry,
        deref_fn: &'a dyn Fn(&KindedSlot) -> Option<KindedSlot>,
    ) -> Self {
        Self {
            schema_registry,
            deref_fn: Some(deref_fn),
        }
    }

    /// Primary entry point: format a runtime value to a string.
    ///
    /// At the top level, raw strings render unquoted (so `print("hi")`
    /// prints `hi`, not `"hi"`). Inside containers (TypedObject fields,
    /// HashMap values, heap-array elements) strings are quoted via
    /// [`Self::format_kinded_nested`].
    pub fn format_kinded(&self, slot: &KindedSlot) -> String {
        self.format_kinded_inner(slot, 0, false)
    }

    /// Format a runtime value as it appears nested inside another
    /// container — quotes string values to disambiguate `{name: "alice"}`
    /// vs `{name: alice}`.
    pub fn format_kinded_nested(&self, slot: &KindedSlot) -> String {
        self.format_kinded_inner(slot, 0, true)
    }

    /// Recursive helper with depth tracking. Caps recursion at depth 50
    /// to bound output for cyclic / deeply nested values.
    ///
    /// `quote_strings` controls whether `String`-kind slots render as
    /// `"hello"` (true, nested) or `hello` (false, top-level).
    fn format_kinded_inner(&self, slot: &KindedSlot, depth: usize, quote_strings: bool) -> String {
        if depth > 50 {
            return "[max depth reached]".to_string();
        }

        let bits = slot.slot.raw();
        match slot.kind {
            // ── Inline scalars ──────────────────────────────────────────
            NativeKind::Bool => slot.slot.as_bool().to_string(),
            NativeKind::Int8
            | NativeKind::NullableInt8
            | NativeKind::Int16
            | NativeKind::NullableInt16
            | NativeKind::Int32
            | NativeKind::NullableInt32
            | NativeKind::Int64
            | NativeKind::NullableInt64
            | NativeKind::IntSize
            | NativeKind::NullableIntSize => slot.slot.as_i64().to_string(),
            NativeKind::UInt64 | NativeKind::NullableUInt64 => {
                // ADR-006 §2.7.7 / Wave 6.5 D-v2-array-detect — v2 typed
                // arrays flow through the kinded API as raw `*mut TypedArray<T>`
                // bits stamped with `NativeKind::UInt64` (no Arc, no
                // refcount). Probe the heap-header `_pad` byte to recognize
                // a v2 typed array pointer; fall back to integer rendering
                // for plain UInt64 scalars.
                if let Some(view) = crate::executor::v2_handlers::v2_array_detect::as_v2_typed_array(
                    bits,
                    slot.kind,
                ) {
                    self.format_v2_typed_array(&view)
                } else {
                    slot.slot.as_u64().to_string()
                }
            }
            NativeKind::UInt8
            | NativeKind::NullableUInt8
            | NativeKind::UInt16
            | NativeKind::NullableUInt16
            | NativeKind::UInt32
            | NativeKind::NullableUInt32
            | NativeKind::UIntSize
            | NativeKind::NullableUIntSize => slot.slot.as_u64().to_string(),
            NativeKind::Float64 | NativeKind::NullableFloat64 => {
                format_number(slot.slot.as_f64())
            }
            // ── String (top-level NativeKind::String) ───────────────────
            NativeKind::String => {
                if bits == 0 {
                    return "None".to_string();
                }
                // SAFETY: per the construction-side contract on every
                // `KindedSlot::from_string_arc`-shaped producer, `String`
                // kind means the slot stores `Arc::into_raw::<String>`
                // bits and the slot owns one strong-count share. Read
                // the inner `&str` for the lifetime of `&self`.
                let s: &String = unsafe { &*(bits as *const String) };
                if quote_strings {
                    format!("\"{}\"", s)
                } else {
                    s.clone()
                }
            }
            // ── Heap-pointer kinds: dispatch via HeapKind ───────────────
            NativeKind::Ptr(hk) => self.format_heap_kind(bits, hk, depth, quote_strings),
        }
    }

    /// Heap-arm formatter: dispatches on `HeapKind` directly to read the
    /// matching typed `Arc<T>` payload (Q8 — no per-heap-variant accessor
    /// on the carrier; the kind discriminant is local).
    ///
    /// `quote_strings` propagates from the entry point — when this heap
    /// arm is itself a child of a TypedObject/HashMap/array and the parent
    /// asked for nested formatting, the leaf string render quotes.
    fn format_heap_kind(
        &self,
        bits: u64,
        hk: HeapKind,
        depth: usize,
        quote_strings: bool,
    ) -> String {
        if bits == 0 {
            return "None".to_string();
        }
        match hk {
            HeapKind::String => {
                // SAFETY: typed-Arc payload per §2.4.
                let s: &String = unsafe { &*(bits as *const String) };
                if quote_strings {
                    format!("\"{}\"", s)
                } else {
                    s.clone()
                }
            }
            HeapKind::Decimal => {
                let d: &rust_decimal::Decimal =
                    unsafe { &*(bits as *const rust_decimal::Decimal) };
                format!("{}D", d)
            }
            HeapKind::BigInt => {
                let i: &i64 = unsafe { &*(bits as *const i64) };
                i.to_string()
            }
            HeapKind::Char => {
                // `Char`-kind stores codepoint bits inline (no Arc<T>).
                match char::from_u32(bits as u32) {
                    Some(c) => c.to_string(),
                    None => "[Invalid char]".to_string(),
                }
            }
            HeapKind::TypedArray => {
                let arr: &TypedArrayData = unsafe { &*(bits as *const TypedArrayData) };
                self.format_typed_array(arr, depth)
            }
            HeapKind::TypedObject => {
                // ADR-006 §2.7.4 / §2.7.6 / Q8 — walk the per-field slots,
                // dispatch each on `field_kinds[i]` (the per-schema
                // `Arc<[NativeKind]>` co-located with the storage), and
                // resolve field names via the schema registry. SAFETY:
                // construction-side contract on `KindedSlot::from_typed_object`
                // — `TypedObject`-kind bits are `Arc::into_raw(Arc<TypedObjectStorage>)`.
                let storage: &TypedObjectStorage =
                    unsafe { &*(bits as *const TypedObjectStorage) };
                self.format_typed_object(storage, depth)
            }
            HeapKind::HashMap => {
                // ADR-006 §2.7.4 — HashMapData stores parallel
                // `Vec<Arc<String>>` keys and `Vec<Arc<HeapValue>>` values.
                // Render each entry by recursing through the heap-value
                // variant, with the Q8 single-discriminator dispatch
                // (`HeapValue` match) preserved at the value side. SAFETY:
                // construction-side contract on `KindedSlot::from_hashmap`.
                let map: &HashMapData = unsafe { &*(bits as *const HashMapData) };
                self.format_hashmap(map, depth)
            }
            HeapKind::HashSet => {
                // Wave 13 W13-hashset-rebuild (ADR-006 §2.7.15 / Q16,
                // 2026-05-10): HashSetData stores a single
                // `Vec<Arc<String>>` keys buffer (mirror of HashMapData
                // with the values buffer dropped). Render as
                // `{"a", "b", ...}`. SAFETY: construction-side contract
                // on `KindedSlot::from_hashset`.
                let _ = depth;
                let set: &shape_value::heap_value::HashSetData =
                    unsafe { &*(bits as *const shape_value::heap_value::HashSetData) };
                self.format_hashset(set)
            }
            HeapKind::DataTable => {
                let dt: &shape_value::DataTable =
                    unsafe { &*(bits as *const shape_value::DataTable) };
                format!("{}", dt)
            }
            HeapKind::Content => {
                let node: &shape_value::content::ContentNode =
                    unsafe { &*(bits as *const shape_value::content::ContentNode) };
                format!("{}", node)
            }
            HeapKind::Instant => {
                let t: &std::time::Instant =
                    unsafe { &*(bits as *const std::time::Instant) };
                format!("<instant:{:?}>", t.elapsed())
            }
            HeapKind::IoHandle => {
                let data: &shape_value::heap_value::IoHandleData =
                    unsafe { &*(bits as *const shape_value::heap_value::IoHandleData) };
                let status = if data.is_open() { "open" } else { "closed" };
                format!("<io_handle:{}:{}>", data.path, status)
            }
            HeapKind::NativeView => {
                let v: &shape_value::heap_value::NativeViewData =
                    unsafe { &*(bits as *const shape_value::heap_value::NativeViewData) };
                format!(
                    "<{}:{}@0x{:x}>",
                    if v.mutable { "cmut" } else { "cview" },
                    v.layout.name,
                    v.ptr
                )
            }
            HeapKind::Temporal => {
                todo!(
                    "phase-2c — see ADR-006 §2.7.4: Temporal formatting \
                     (DateTime / Duration / TimeSpan / Timeframe) needs \
                     the kinded constructor surface from Wave 5e DateTime \
                     ctor body migration"
                );
            }
            HeapKind::TableView => {
                let tv: &shape_value::heap_value::TableViewData =
                    unsafe { &*(bits as *const shape_value::heap_value::TableViewData) };
                format!("{}", tv)
            }
            HeapKind::TaskGroup => {
                let tg: &shape_value::heap_value::TaskGroupData =
                    unsafe { &*(bits as *const shape_value::heap_value::TaskGroupData) };
                let kind_str = match tg.kind {
                    0 => "All",
                    1 => "Race",
                    2 => "Any",
                    3 => "Settle",
                    _ => "Unknown",
                };
                format!("[TaskGroup:{}({})]", kind_str, tg.task_ids.len())
            }
            HeapKind::Closure => {
                // ClosureRaw payload uses `OwnedClosureBlock` rather than
                // `Arc<HeapValue>`; the formatter walked the legacy
                // `HeapValue::ClosureRaw` arm via `as_closure_handle()`.
                // The kinded read needs the §2.7.8 cell-storage rebuild
                // (`B7-closure-cells`) before the closure's `function_id`
                // can be reached without going through ValueWord.
                todo!(
                    "phase-2c — see ADR-006 §2.7.4 / §2.7.8: closure \
                     formatting needs kinded ClosureRaw read (§2.7.8 / Q10 \
                     B7-closure-cells extension)"
                );
            }
            HeapKind::Future => {
                // Future-id is an inline scalar payload on its `HeapKind`;
                // a `KindedSlot` flagged `Ptr(HeapKind::Future)` carries
                // the future id directly in `bits`.
                format!("[Future:{}]", bits)
            }
            HeapKind::NativeScalar => {
                // NativeScalar is `Copy`/inline (≤ 16 bytes); the kinded
                // surface for the `repr(C)` packed payload lands with
                // Wave 5c native-interop body migration.
                let _ = bits;
                todo!(
                    "phase-2c — see ADR-006 §2.7.4: NativeScalar formatting \
                     needs the kinded native-interop carrier (Wave 5c \
                     dispatch_native_interop_builtin)"
                );
            }
            HeapKind::FilterExpr => {
                // Wave-γ G-heap-filter-expr (ADR-006 §2.3 / §2.7.6 / Q8
                // amendment): FilterExpr trees are a transient query-DSL
                // value; they don't have a user-facing print form. Render
                // as an opaque tag for diagnostics.
                let _ = bits;
                "<filter_expr>".to_string()
            }
            HeapKind::Reference => {
                // ADR-006 §2.7.13 / Q14 (Wave 8 W8-T26, 2026-05-10):
                // Reference values are within-program data emitted by the
                // `MakeRef` family and consumed locally by `DerefLoad` /
                // `DerefStore` / `SetIndexRef`. They don't have a
                // user-facing print form; render as an opaque tag.
                let _ = bits;
                "<ref>".to_string()
            }
            HeapKind::SharedCell => {
                // Wave 8 W8-T25 (ADR-006 §2.7.12 / Q13 amendment,
                // 2026-05-10): `SharedCell` cell-pointer slots are an
                // interior-only cell-pointer shape; user-facing prints
                // go through `op_load_shared_local` /
                // `op_load_shared_capture` which strip the SharedCell
                // outer label and dispatch on the cell's interior kind.
                // Reaching this arm with a SharedCell-labeled slot at
                // a print surface is a kind-source bug. Render as an
                // opaque tag for diagnostics.
                let _ = bits;
                "<shared_cell>".to_string()
            }
            HeapKind::Iterator => {
                // W13-iterator-state (ADR-006 §2.7.16 / Q17,
                // 2026-05-10): iterator pipelines have no user-facing
                // print form — terminals materialise their elements;
                // an Iterator slot reaching the Display surface is
                // "still lazy" by construction. Render as an opaque
                // tag.
                let _ = bits;
                "<iterator>".to_string()
            }
        }
    }

    /// Format a v2 typed array (raw `*mut TypedArray<T>` pointer) as
    /// `[1, 2, 3]`. Element type comes from the heap-header `_pad` byte
    /// stamped at allocation time; element bits / kind come from the
    /// canonical kinded read helper
    /// (`v2_array_detect::read_element`).
    fn format_v2_typed_array(
        &self,
        view: &crate::executor::v2_handlers::v2_array_detect::V2TypedArrayView,
    ) -> String {
        use crate::executor::v2_handlers::v2_array_detect::read_element;
        let mut out = String::with_capacity(2 + view.len as usize * 4);
        out.push('[');
        for i in 0..view.len {
            if i > 0 {
                out.push_str(", ");
            }
            // Per-element rendering — the element kind is one of
            // Float64 / Int64 / Int32 / Bool per the v2 typed-array
            // contract. Format each through the canonical scalar arms.
            if let Some((bits, kind)) = read_element(view, i) {
                let elem_slot =
                    KindedSlot::new(ValueSlot::from_raw(bits), kind);
                out.push_str(&self.format_kinded_inner(&elem_slot, 0, true));
                std::mem::forget(elem_slot);
            } else {
                out.push_str("?");
            }
        }
        out.push(']');
        out
    }

    /// Format the inline-typed-array variants. Each element is formatted
    /// per its native scalar kind (no schema lookup needed); the heap
    /// element variants surface as Phase-2c todo!().
    fn format_typed_array(&self, arr: &TypedArrayData, depth: usize) -> String {
        match arr {
            TypedArrayData::I64(a) => {
                let elems: Vec<String> = a.iter().map(|v| v.to_string()).collect();
                format!("[{}]", elems.join(", "))
            }
            TypedArrayData::F64(a) => {
                let elems: Vec<String> = a.iter().map(|v| format_array_float(*v)).collect();
                format!("[{}]", elems.join(", "))
            }
            TypedArrayData::FloatSlice {
                parent,
                offset,
                len,
            } => {
                let off = *offset as usize;
                let slice_len = *len as usize;
                let data = &parent.data[off..off + slice_len];
                let elems: Vec<String> = data.iter().map(|v| format_array_float(*v)).collect();
                format!("[{}]", elems.join(", "))
            }
            TypedArrayData::Bool(a) => {
                let elems: Vec<String> = a
                    .iter()
                    .map(|v| if *v != 0 { "true" } else { "false" }.to_string())
                    .collect();
                format!("[{}]", elems.join(", "))
            }
            TypedArrayData::I8(a) => {
                let elems: Vec<String> = a.data.iter().map(|v| v.to_string()).collect();
                format!("[{}]", elems.join(", "))
            }
            TypedArrayData::I16(a) => {
                let elems: Vec<String> = a.data.iter().map(|v| v.to_string()).collect();
                format!("[{}]", elems.join(", "))
            }
            TypedArrayData::I32(a) => {
                let elems: Vec<String> = a.data.iter().map(|v| v.to_string()).collect();
                format!("[{}]", elems.join(", "))
            }
            TypedArrayData::U8(a) => {
                let elems: Vec<String> = a.data.iter().map(|v| v.to_string()).collect();
                format!("[{}]", elems.join(", "))
            }
            TypedArrayData::U16(a) => {
                let elems: Vec<String> = a.data.iter().map(|v| v.to_string()).collect();
                format!("[{}]", elems.join(", "))
            }
            TypedArrayData::U32(a) => {
                let elems: Vec<String> = a.data.iter().map(|v| v.to_string()).collect();
                format!("[{}]", elems.join(", "))
            }
            TypedArrayData::U64(a) => {
                let elems: Vec<String> = a.data.iter().map(|v| v.to_string()).collect();
                format!("[{}]", elems.join(", "))
            }
            TypedArrayData::F32(a) => {
                let elems: Vec<String> = a
                    .data
                    .iter()
                    .map(|v| format_array_float(*v as f64))
                    .collect();
                format!("[{}]", elems.join(", "))
            }
            TypedArrayData::Matrix(m) => {
                format!("<Mat<number>:{}x{}>", m.rows, m.cols)
            }
            TypedArrayData::String(a) => {
                // Inside an array, strings render quoted to disambiguate
                // `["a", "b"]` from `[a, b]` (matches the TypedObject-field
                // and HashMap-value rule).
                let elems: Vec<String> =
                    a.iter().map(|s| format!("\"{}\"", s.as_str())).collect();
                format!("[{}]", elems.join(", "))
            }
            TypedArrayData::HeapValue(buf) => {
                // ADR-005 §1 single-discriminator: each element is an
                // `Arc<HeapValue>` carried directly in the buffer; recurse
                // through `HeapValue` match.
                let elems: Vec<String> = buf
                    .data
                    .iter()
                    .map(|hv| self.format_heap_value(hv.as_ref(), depth + 1))
                    .collect();
                format!("[{}]", elems.join(", "))
            }
        }
    }

    /// Apply a reference-resolver if configured, formatting the
    /// dereferenced target. Returns `<ref>` when no resolver is wired up.
    ///
    /// Reserved for the Phase-2c ref-kind landing — until refs gain
    /// their own NativeKind variant (or the kinded-ref ABI lands), this
    /// helper is unused by the dispatch path above and stays here so the
    /// resolver hook on `with_deref` keeps a coherent signature.
    #[allow(dead_code)]
    fn format_ref(&self, slot: &KindedSlot, depth: usize) -> String {
        if let Some(deref) = &self.deref_fn {
            if let Some(resolved) = deref(slot) {
                return self.format_kinded_inner(&resolved, depth + 1, true);
            }
        }
        "<ref>".to_string()
    }

    // ──────────────────────────────────────────────────────────────────────
    // TypedObject + HashMap helpers (ADR-006 §2.7.4 / §2.7.6 / Q8)
    // ──────────────────────────────────────────────────────────────────────

    /// Format a `TypedObjectStorage` as `{field1: val1, field2: val2}`.
    ///
    /// Field names come from the schema registry when the storage's
    /// `schema_id` resolves; otherwise positional `_0`, `_1` placeholders
    /// are used so the formatter degrades gracefully when the registry is
    /// not populated for a runtime-built object (e.g. anonymous record
    /// literals before schema registration).
    ///
    /// Each slot is reified as a `KindedSlot { slot, kind: field_kinds[i] }`
    /// and recursed through `format_kinded_inner` with `quote_strings = true`
    /// so nested string fields render `"…"`. Slots are *borrowed*: we do
    /// NOT clone the slot bits or transfer ownership; the parent
    /// `TypedObjectStorage` keeps holding all heap shares for the
    /// lifetime of `&self`.
    fn format_typed_object(&self, storage: &TypedObjectStorage, depth: usize) -> String {
        let schema = self.schema_registry.get_by_id(storage.schema_id as u32);
        let n = storage.slots.len().min(storage.field_kinds.len());
        let mut out = String::with_capacity(2 + n * 8);
        out.push('{');
        for i in 0..n {
            if i > 0 {
                out.push_str(", ");
            }
            // Field name: prefer the schema-resolved name; fall back to
            // a positional placeholder so the formatter still produces
            // human-readable output for schema-less objects.
            let name: &str = schema
                .and_then(|s| s.fields.get(i).map(|f| f.name.as_str()))
                .unwrap_or("_");
            if name == "_" {
                out.push_str(&format!("_{}", i));
            } else {
                out.push_str(name);
            }
            out.push_str(": ");
            // Reify the slot as a borrowed `KindedSlot` for the recursive
            // formatter call. This carrier never owns a strong-count share
            // — it is dropped via `mem::forget` at the end of the loop
            // iteration so the parent storage retains every payload.
            let slot = ValueSlot::from_raw(storage.slots[i].raw());
            let kinded = KindedSlot::new(slot, storage.field_kinds[i]);
            let rendered = self.format_kinded_inner(&kinded, depth + 1, true);
            out.push_str(&rendered);
            std::mem::forget(kinded);
        }
        out.push('}');
        out
    }

    /// Format a `HashSetData` as `{"a", "b", ...}`. Wave 13
    /// W13-hashset-rebuild (ADR-006 §2.7.15) — one-keyspace mirror of
    /// HashMap's render shape with the values column dropped.
    fn format_hashset(&self, set: &shape_value::heap_value::HashSetData) -> String {
        let n = set.keys.data.len();
        let mut out = String::with_capacity(2 + n * 6);
        out.push('{');
        for (i, k) in set.keys.data.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("\"{}\"", k));
        }
        out.push('}');
        out
    }

    /// Format a `HashMapData` as `{key1: val1, key2: val2}`.
    ///
    /// Each value is an `Arc<HeapValue>` — dispatched through the
    /// canonical ADR-005 §1 single-discriminator `HeapValue` match.
    fn format_hashmap(&self, map: &HashMapData, depth: usize) -> String {
        let n = map.keys.data.len().min(map.values.data.len());
        let mut out = String::with_capacity(2 + n * 8);
        out.push('{');
        for i in 0..n {
            if i > 0 {
                out.push_str(", ");
            }
            let key = &map.keys.data[i];
            out.push_str(&format!("\"{}\"", key));
            out.push_str(": ");
            out.push_str(&self.format_heap_value(&map.values.data[i], depth + 1));
        }
        out.push('}');
        out
    }

    /// Format a `HeapValue` reference (the value side of `HashMapData`'s
    /// `TypedBuffer<Arc<HeapValue>>` and the heterogeneous element arm of
    /// `TypedArrayData::HeapValue`). Dispatches via the ADR-005 §1
    /// single-discriminator `HeapValue` match.
    fn format_heap_value(&self, hv: &HeapValue, depth: usize) -> String {
        if depth > 50 {
            return "[max depth reached]".to_string();
        }
        match hv {
            HeapValue::String(s) => format!("\"{}\"", s),
            HeapValue::Decimal(d) => format!("{}D", d),
            HeapValue::BigInt(b) => b.as_ref().to_string(),
            HeapValue::Char(c) => format!("'{}'", c),
            HeapValue::Future(id) => format!("[Future:{}]", id),
            HeapValue::TypedArray(arr) => self.format_typed_array(arr.as_ref(), depth),
            HeapValue::TypedObject(o) => self.format_typed_object(o.as_ref(), depth),
            HeapValue::HashMap(m) => self.format_hashmap(m.as_ref(), depth),
            HeapValue::HashSet(s) => self.format_hashset(s.as_ref()),
            HeapValue::DataTable(t) => format!("{}", t),
            HeapValue::Content(n) => format!("{}", n),
            HeapValue::Instant(t) => format!("<instant:{:?}>", t.elapsed()),
            HeapValue::IoHandle(h) => {
                let status = if h.is_open() { "open" } else { "closed" };
                format!("<io_handle:{}:{}>", h.path, status)
            }
            HeapValue::NativeView(v) => format!(
                "<{}:{}@0x{:x}>",
                if v.mutable { "cmut" } else { "cview" },
                v.layout.name,
                v.ptr
            ),
            HeapValue::TableView(tv) => format!("{}", tv),
            HeapValue::TaskGroup(tg) => {
                let kind_str = match tg.kind {
                    0 => "All",
                    1 => "Race",
                    2 => "Any",
                    3 => "Settle",
                    _ => "Unknown",
                };
                format!("[TaskGroup:{}({})]", kind_str, tg.task_ids.len())
            }
            HeapValue::NativeScalar(_) => "<native_scalar>".to_string(),
            HeapValue::Temporal(_) => "<temporal>".to_string(),
            HeapValue::ClosureRaw(_) => "<closure>".to_string(),
            HeapValue::FilterExpr(_) => "<filter_expr>".to_string(),
            HeapValue::Reference(_) => "<ref>".to_string(),
            HeapValue::Iterator(_) => "<iterator>".to_string(),
        }
    }
}

/// Format a number, removing unnecessary decimal places.
fn format_number(n: f64) -> String {
    if n.is_nan() {
        "NaN".to_string()
    } else if n.is_infinite() {
        if n.is_sign_positive() {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        }
    } else if n.fract() == 0.0 && n.abs() < 1e15 {
        // Integer-like floats: always show .0 to distinguish from int.
        format!("{}.0", n as i64)
    } else {
        n.to_string()
    }
}

/// Format a single float for inclusion in a typed-array element list.
/// Mirrors `format_number` but returns the integer-shape (`{n}.0`) for
/// whole-number floats that fit comfortably in `i64`.
fn format_array_float(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        format!("{}.0", v as i64)
    } else {
        format!("{}", v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn create_test_registry() -> TypeSchemaRegistry {
        TypeSchemaRegistry::new()
    }

    #[test]
    fn test_format_inline_scalars() {
        let reg = create_test_registry();
        let formatter = ValueFormatter::new(&reg);

        assert_eq!(
            formatter.format_kinded(&KindedSlot::from_int(42)),
            "42"
        );
        assert_eq!(
            formatter.format_kinded(&KindedSlot::from_int(-100)),
            "-100"
        );
        assert_eq!(
            formatter.format_kinded(&KindedSlot::from_bool(true)),
            "true"
        );
        assert_eq!(
            formatter.format_kinded(&KindedSlot::from_bool(false)),
            "false"
        );
        assert_eq!(
            formatter.format_kinded(&KindedSlot::from_number(3.14)),
            "3.14"
        );
    }

    #[test]
    fn test_format_integer_like_float_shows_decimal_point() {
        let reg = create_test_registry();
        let formatter = ValueFormatter::new(&reg);
        assert_eq!(
            formatter.format_kinded(&KindedSlot::from_number(1.0)),
            "1.0"
        );
        assert_eq!(
            formatter.format_kinded(&KindedSlot::from_number(-5.0)),
            "-5.0"
        );
        assert_eq!(
            formatter.format_kinded(&KindedSlot::from_number(100.0)),
            "100.0"
        );
    }

    #[test]
    fn test_format_special_floats() {
        assert_eq!(format_number(f64::NAN), "NaN");
        assert_eq!(format_number(f64::INFINITY), "Infinity");
        assert_eq!(format_number(f64::NEG_INFINITY), "-Infinity");
    }

    #[test]
    fn test_format_string() {
        let reg = create_test_registry();
        let formatter = ValueFormatter::new(&reg);

        let s = KindedSlot::from_string_arc(Arc::new("hello".to_string()));
        assert_eq!(formatter.format_kinded(&s), "hello");
    }

    #[test]
    fn test_format_decimal() {
        let reg = create_test_registry();
        let formatter = ValueFormatter::new(&reg);

        let d = KindedSlot::from_decimal(Arc::new(rust_decimal::Decimal::from(42)));
        assert_eq!(formatter.format_kinded(&d), "42D");

        let d2 = KindedSlot::from_decimal(Arc::new(rust_decimal::Decimal::new(314, 2)));
        assert_eq!(formatter.format_kinded(&d2), "3.14D");
    }

    #[test]
    fn test_format_bigint() {
        let reg = create_test_registry();
        let formatter = ValueFormatter::new(&reg);

        let b = KindedSlot::from_bigint(Arc::new(123_i64));
        assert_eq!(formatter.format_kinded(&b), "123");
    }

    #[test]
    fn test_format_char() {
        let reg = create_test_registry();
        let formatter = ValueFormatter::new(&reg);

        let c = KindedSlot::from_char('A');
        assert_eq!(formatter.format_kinded(&c), "A");

        let c2 = KindedSlot::from_char('λ');
        assert_eq!(formatter.format_kinded(&c2), "λ");
    }
}

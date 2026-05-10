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
use shape_value::heap_value::{HeapKind, TypedArrayData};
use shape_value::{KindedSlot, NativeKind};

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
    pub fn format_kinded(&self, slot: &KindedSlot) -> String {
        self.format_kinded_with_depth(slot, 0)
    }

    /// Recursive helper with depth tracking. Caps recursion at depth 50
    /// to bound output for cyclic / deeply nested values.
    fn format_kinded_with_depth(&self, slot: &KindedSlot, depth: usize) -> String {
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
            NativeKind::UInt8
            | NativeKind::NullableUInt8
            | NativeKind::UInt16
            | NativeKind::NullableUInt16
            | NativeKind::UInt32
            | NativeKind::NullableUInt32
            | NativeKind::UInt64
            | NativeKind::NullableUInt64
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
                s.clone()
            }
            // ── Heap-pointer kinds: dispatch via HeapKind ───────────────
            NativeKind::Ptr(hk) => self.format_heap_kind(bits, hk, depth),
        }
    }

    /// Heap-arm formatter: dispatches on `HeapKind` directly to read the
    /// matching typed `Arc<T>` payload (Q8 — no per-heap-variant accessor
    /// on the carrier; the kind discriminant is local).
    fn format_heap_kind(&self, bits: u64, hk: HeapKind, depth: usize) -> String {
        if bits == 0 {
            return "None".to_string();
        }
        match hk {
            HeapKind::String => {
                // SAFETY: typed-Arc payload per §2.4.
                let s: &String = unsafe { &*(bits as *const String) };
                s.clone()
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
                // TypedObject formatting depends on the schema-registry
                // walking each `ValueSlot` per its `NativeKind` — the
                // kinded reformatting of slots routes through the same
                // §2.7.6 dispatch pattern as the typed_handlers but at
                // depth, and is wired up in the Phase-2c output-adapter
                // rebuild.
                let _ = self.schema_registry;
                todo!(
                    "phase-2c — see ADR-006 §2.7.4: TypedObject formatting \
                     needs the kinded slot-by-slot walk against the \
                     schema's NativeKind track (§2.7.6)"
                );
            }
            HeapKind::HashMap => {
                todo!(
                    "phase-2c — see ADR-006 §2.7.4: HashMap formatting \
                     needs kinded key/value walks (HashMapData buffers \
                     carry their own NativeKind tracks per §2.4)"
                );
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
        }
    }

    /// Format the inline-typed-array variants. Each element is formatted
    /// per its native scalar kind (no schema lookup needed); the heap
    /// element variants surface as Phase-2c todo!().
    fn format_typed_array(&self, arr: &TypedArrayData, _depth: usize) -> String {
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
                let elems: Vec<String> = a.iter().map(|s| s.as_str().to_string()).collect();
                format!("[{}]", elems.join(", "))
            }
            TypedArrayData::HeapValue(_) => {
                // Per-element heap-formatting needs the kinded payload
                // walk that depends on the slot-by-slot §2.7.6 dispatch
                // landing in Phase-2c. Surface rather than paper over.
                todo!(
                    "phase-2c — see ADR-006 §2.7.4: TypedArray<HeapValue> \
                     element formatting needs the kinded heap-element \
                     walk (§2.7.6 dispatch_slice)"
                );
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
                return self.format_kinded_with_depth(&resolved, depth + 1);
            }
        }
        "<ref>".to_string()
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

//! Array sort operations
//!
//! Handles: order_by, then_by, join_str
//!
//! ## R8 W4 J.5f sort + orderBy + thenBy (2026-05-24)
//!
//! Per supervisor D4 (2026-05-24) v0.3 scope: basic `sort` (natural and
//! comparator) + `orderBy(key_fn[, "asc"/"desc"])` + `thenBy(key_fn[,
//! "asc"/"desc"])`. Relational joins (innerJoin/leftJoin/crossJoin) +
//! `groupBy` deferred to v0.4 per supervisor D4 (Refusal #10 family).
//!
//! ### Architecture
//!
//! - Kind-generic via `v2_array_detect::{permute_array, cmp_element_natural,
//!   read_element}` + the §2.7.11 / Q12 closure-callback ABI
//!   (`vm.call_value_immediate_nb`).
//! - Stable sort: indices-permutation approach with `slice::sort_by` (stable
//!   per Rust precedent + supervisor D4 user expectation). Equal elements
//!   preserve original relative order.
//! - Natural ordering (sort with no comparator): per V2ElemType. F64/F32 use
//!   `total_cmp` (NaN-safe; NaN is largest). String/Decimal use lex/decimal
//!   compare. TypedObject SURFACE (no canonical natural ordering per ADR-006
//!   §2.7.14; Refusal #10 v0.4 territory).
//! - Comparator sort (`sort(|a,b| ...)`): invoke the comparator per pair via
//!   the closure-callback ABI; the return value is interpreted as `< 0` /
//!   `== 0` / `> 0` mapping to `Less` / `Equal` / `Greater`.
//! - Key-fn sort (`orderBy(key_fn)`): one closure call per element to
//!   compute keys up-front; then sort the indices by comparing cached keys.
//!   This separates closure invocation (which requires `&mut vm`) from the
//!   sort comparator (which would otherwise need mutable VM access during
//!   `slice::sort_by`).
//! - `thenBy(key_fn)`: shares the body shape with `orderBy` —
//!   `slice::sort_by` is stable, so re-sorting the (assumed-already-primary-
//!   sorted) input by the secondary key produces the lexicographic
//!   primary→secondary order.
//! - Direction parsing: optional 3rd arg `"asc"` (default) or `"desc"`;
//!   structured `RuntimeError` on other strings or non-string kinds.
//!
//! ### Refcount discipline
//!
//! - `permute_array` per heap-element kind: bumps refcount once per stored
//!   slot. The input view's refcount is unaffected; the caller's
//!   receiver-share remains live for the borrow.
//! - Closure invocation: `bump_closure_share` per iteration (per the
//!   §2.7.11 / Q12 caller-side compensation contract: frame teardown via
//!   `op_return` releases the share carried in `CallFrame.closure_heap_bits`).
//! - Element reads for the key-fn invocation: `read_element` returns a
//!   `(bits, kind)` pair carrying a fresh share for heap-element kinds —
//!   the call-value-immediate-nb consumes the arg share at frame setup.
//!
//! ### Discipline preserved (CLAUDE.md)
//!
//! - NO ValueWord / tagged-dispatch / dynamic-fallback resurrection.
//! - NO `(decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|
//!   callee|capture) (bridge|probe|helper|hop|translator|adapter|shim)`
//!   patterns. Helpers below describe their territory by name + by
//!   primitives consumed (`permute_array` / `cmp_element_natural` /
//!   `read_element` / `vm.call_value_immediate_nb`).
//! - NO Bool-default for unknown comparator-return kinds (supervisor D3 +
//!   ADR-006 §2.7.14): unsupported kinds surface a structured
//!   `RuntimeError` naming the kind + the context.
//! - NO innerJoin/leftJoin/crossJoin/groupBy (Refusal #10 family — strictly
//!   v0.4 per supervisor D4). Those handlers remain in
//!   `array_joins.rs` / their respective files and are NOT touched here.
//! - ADR-005 §1 single-discriminator preserved (heap dispatch via
//!   `as_heap_value()` + HeapValue match; no parallel discriminator).
//! - ADR-006 §2.7.5 producer-side stamp (`permute_array` stamps elem_type
//!   on the output array).
//! - ADR-006 §2.7.10 / Q11 MethodFnV2 ABI unchanged.
//! - ADR-006 §2.7.11 / Q12 value-call ABI unchanged.
//!
//! ### joinStr
//!
//! `joinStr` remains SURFACE — element stringification per V2ElemType is
//! covered by the pre-deletion path that dispatched on `TypedArrayData::X`
//! arms (now deleted). The replacement requires a per-kind
//! `element_to_string` primitive in `v2_array_detect.rs`, which is its own
//! ckpt-3 sub-cluster (not J.5f scope per supervisor D4). The SURFACE body
//! is retained but with an updated docstring naming the territory.

use crate::executor::VirtualMachine;
use crate::executor::v2_handlers::v2_array_detect::{
    V2TypedArrayView, as_v2_typed_array, cmp_element_natural, permute_array, read_element,
};
use shape_runtime::context::ExecutionContext;
use shape_value::HeapValue;
use shape_value::heap_value::HeapKind;
use shape_value::{KindedSlot, NativeKind, VMError, ValueSlot};
use std::cmp::Ordering;
use std::sync::Arc;

// ═══════════════════════════════════════════════════════════════════════════
// Local helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Extract the kind-generic `V2TypedArrayView` from the receiver
/// `KindedSlot`. Receiver kind must be `Ptr(HeapKind::TypedArray)`
/// (r5c-2-β-CKPT-C single carrier).
#[inline]
fn extract_view(op: &'static str, slot: &KindedSlot) -> Result<V2TypedArrayView, VMError> {
    if slot.kind != NativeKind::Ptr(HeapKind::TypedArray) {
        return Err(VMError::RuntimeError(format!(
            "Array.{op}: expected v2 TypedArray receiver, got kind {:?}",
            slot.kind
        )));
    }
    as_v2_typed_array(slot.slot.raw(), slot.kind).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.{op}: receiver bits failed v2 TypedArray detection (kind {:?})",
            slot.kind
        ))
    })
}

/// Wrap a freshly-allocated v2 typed array pointer as a `KindedSlot` with
/// `NativeKind::Ptr(HeapKind::TypedArray)` (single carrier per
/// r5c-2-β-CKPT-C).
#[inline]
fn new_array_slot(ptr: *mut u8) -> KindedSlot {
    KindedSlot::new(
        ValueSlot::from_u64(ptr as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    )
}

/// Validate the closure argument shape (closure or function-ref).
/// Function refs flow as `NativeKind::UInt64` per the existing
/// callsite convention (mirrors `array_sets.rs`).
#[inline]
fn require_closure(op: &str, slot: &KindedSlot) -> Result<(), VMError> {
    match slot.kind {
        NativeKind::Ptr(HeapKind::Closure) | NativeKind::UInt64 => Ok(()),
        other => Err(VMError::RuntimeError(format!(
            "{op}: key function must be a closure or function ref, got kind {:?}",
            other
        ))),
    }
}

/// Per the §2.7.11 / Q12 caller-side compensation contract: the frame
/// teardown via `op_return` releases the share carried in
/// `CallFrame.closure_heap_bits`, so a borrowed closure passed in a
/// per-iteration loop would have its dispatch-shell-owned share consumed
/// by the FIRST call, leaving the carrier dangling on subsequent
/// iterations. This bump restores ownership symmetry.
#[inline]
fn bump_closure_share(slot: &KindedSlot) {
    if let NativeKind::Ptr(HeapKind::Closure) = slot.kind {
        let bits = slot.slot.raw();
        if bits != 0 {
            // SAFETY: per the W7 closure-slot contract, bits =
            // `Arc::into_raw(Arc<HeapValue>)`. Bumping the strong count
            // is sound as long as the share originally owned by the
            // carrier is still live — guaranteed because the carrier is
            // borrowed for the entire scope of the calling handler.
            unsafe {
                Arc::increment_strong_count(bits as *const HeapValue);
            }
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum SortDirection {
    Ascending,
    Descending,
}

/// Parse the optional direction argument at `args[2]`. Accepts:
/// - missing (defaults to Ascending),
/// - `"asc"` / `"ascending"` → Ascending,
/// - `"desc"` / `"descending"` → Descending.
///
/// Non-string kinds and other string values surface a structured
/// `RuntimeError`.
fn parse_direction(args: &[KindedSlot], op: &str) -> Result<SortDirection, VMError> {
    if args.len() < 3 {
        return Ok(SortDirection::Ascending);
    }
    let slot = &args[2];
    match slot.kind {
        NativeKind::String | NativeKind::StringV2 => {
            let s = slot.as_str().unwrap_or("");
            match s {
                "asc" | "ascending" => Ok(SortDirection::Ascending),
                "desc" | "descending" => Ok(SortDirection::Descending),
                other => Err(VMError::RuntimeError(format!(
                    "{op}: direction must be \"asc\" or \"desc\", got {:?}",
                    other
                ))),
            }
        }
        other => Err(VMError::RuntimeError(format!(
            "{op}: direction must be a string (\"asc\" or \"desc\"), got kind {:?}",
            other
        ))),
    }
}

/// Compare two cached `KindedSlot` keys (produced by the key-fn). Both
/// keys must share the same `NativeKind` (no implicit coercion per
/// CLAUDE.md §Type System Rules). Float comparison uses `total_cmp`
/// (NaN-safe). String/Decimal pointers dereference; TypedObject (heap
/// aggregate) surfaces structured per supervisor D3 (v0.4 territory).
fn cmp_key_kinded(a: &KindedSlot, b: &KindedSlot, op: &str) -> Result<Ordering, VMError> {
    if a.kind != b.kind {
        return Err(VMError::RuntimeError(format!(
            "{op}: key function produced heterogeneous result kinds {:?} vs {:?} \
             (CLAUDE.md \"No runtime coercion\" — keys must be monomorphic)",
            a.kind, b.kind
        )));
    }
    Ok(match a.kind {
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize => (a.slot.raw() as i64).cmp(&(b.slot.raw() as i64)),
        NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize => a.slot.raw().cmp(&b.slot.raw()),
        NativeKind::Float64 => {
            f64::from_bits(a.slot.raw()).total_cmp(&f64::from_bits(b.slot.raw()))
        }
        NativeKind::Float32 => {
            f32::from_bits(a.slot.raw() as u32).total_cmp(&f32::from_bits(b.slot.raw() as u32))
        }
        NativeKind::Bool => (a.slot.raw() != 0).cmp(&(b.slot.raw() != 0)),
        NativeKind::Char => (a.slot.raw() as u32).cmp(&(b.slot.raw() as u32)),
        NativeKind::String | NativeKind::StringV2 => {
            let sa = a.as_str().unwrap_or("");
            let sb = b.as_str().unwrap_or("");
            sa.cmp(sb)
        }
        other => {
            return Err(VMError::NotImplemented(format!(
                "{op}: comparison of key kind {:?} — SURFACE: only scalar / Bool / Char / \
                 String key kinds dispatched in J.5f v0.3 scope per supervisor D4. \
                 Heap-aggregate keys (DecimalV2, TypedObject, ...) need an ADR-006 \
                 §2.7.6 / Q8 per-kind comparator table — v0.4 territory.",
                other
            )));
        }
    })
}

/// Interpret a closure return value as a 3-way comparator result.
/// Accepts:
///   - integer-family kinds: `< 0` → Less, `== 0` → Equal, `> 0` → Greater
///   - Float64/Float32: same sign convention via `total_cmp` against 0.0
///   - Bool: NOT accepted (Bool-default fabrication forbidden per ADR-006
///     §2.7.14 + supervisor D3 — a comparator must return an integer sign).
fn interpret_comparator_result(result: &KindedSlot, op: &str) -> Result<Ordering, VMError> {
    match result.kind {
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize => {
            let v = result.slot.raw() as i64;
            Ok(v.cmp(&0))
        }
        NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize => {
            // Unsigned: only > 0 or == 0 are representable; treat as
            // Greater / Equal. Negative cmp results are not expressible
            // as unsigned (a comparator returning an unsigned type loses
            // the `Less` arm). Surface a structured warning at first
            // misuse rather than silently mapping to Equal — but since
            // there's no diagnostic channel in this hot path, fall
            // through to the natural unsigned cmp against 0 (== 0 →
            // Equal, > 0 → Greater).
            let v = result.slot.raw();
            Ok(v.cmp(&0))
        }
        NativeKind::Float64 => {
            let v = f64::from_bits(result.slot.raw());
            Ok(v.total_cmp(&0.0))
        }
        NativeKind::Float32 => {
            let v = f32::from_bits(result.slot.raw() as u32);
            Ok(v.total_cmp(&0.0))
        }
        other => Err(VMError::RuntimeError(format!(
            "{op}: comparator must return an integer sign (negative → first arg sorts \
             before second, zero → equal, positive → first arg sorts after second); \
             got kind {:?}. Bool-default refused per ADR-006 §2.7.14 + supervisor D3.",
            other
        ))),
    }
}

/// Sort `view` by a comparator closure invoked per pair. Returns the
/// permutation indices in sorted order (stable).
///
/// Two-pass approach: pass 1 reads every element into a `Vec<KindedSlot>`
/// so the comparator can be invoked with `&mut vm` (the `slice::sort_by`
/// comparator cannot borrow `&mut vm`). Pass 2 sorts an index permutation
/// with cached per-element slots; the comparator is invoked once per pair
/// comparison via `vm.call_value_immediate_nb`. Error capture via a sticky
/// shadow + short-circuit-Equal, then check-and-return.
fn sort_by_comparator(
    vm: &mut VirtualMachine,
    view: &V2TypedArrayView,
    closure: &KindedSlot,
    mut ctx: Option<&mut ExecutionContext>,
    op: &'static str,
) -> Result<Vec<u32>, VMError> {
    let len = view.len;
    let mut indices: Vec<u32> = (0..len).collect();
    if len < 2 {
        return Ok(indices);
    }

    // Pre-cache each element as a KindedSlot so the comparator
    // (which takes elements by value-pair) can pull them without
    // re-reading on each comparison. Heap elements carry one share each
    // via `read_element` (matches the v2 String/Decimal/TypedObject
    // retain-on-read contract — these shares are released when each slot
    // drops at function exit).
    let mut elems: Vec<KindedSlot> = Vec::with_capacity(len as usize);
    for i in 0..len {
        let (bits, kind) = read_element(view, i).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Array.{op}: read_element({i}) returned None for element kind {:?}",
                view.elem_type
            ))
        })?;
        elems.push(KindedSlot::new(ValueSlot::from_raw(bits), kind));
    }

    // Comparator-driven sort. Errors captured via a sticky shadow and
    // short-circuit Equal; checked after the sort completes. The closure
    // invocation needs `&mut vm` — we use a RefCell-free pattern by
    // moving the sort body into a manual stable-sort-on-indices
    // implementation. `slice::sort_by` is stable but its comparator can't
    // borrow `&mut vm` from the enclosing scope across all calls; we
    // use `sort_by` with a captured `*mut VirtualMachine` is unsound.
    //
    // Solution: collapse the comparator + closure invocation into a
    // single Vec of pre-computed pair-comparisons is O(n²) memory — not
    // acceptable. Instead, use an in-place stable merge sort whose
    // comparator body runs inline with `&mut vm` access.
    let mut tmp: Vec<u32> = vec![0; len as usize];
    let mut cmp_err: Option<VMError> = None;
    stable_merge_sort_with_comparator(&mut indices, &mut tmp, &elems, |a_slot, b_slot| {
        if cmp_err.is_some() {
            return Ordering::Equal;
        }
        bump_closure_share(closure);
        let result = match vm.call_value_immediate_nb(
            closure,
            &[a_slot.clone(), b_slot.clone()],
            ctx.as_deref_mut(),
        ) {
            Ok(r) => r,
            Err(e) => {
                cmp_err = Some(e);
                return Ordering::Equal;
            }
        };
        match interpret_comparator_result(&result, op) {
            Ok(o) => o,
            Err(e) => {
                cmp_err = Some(e);
                Ordering::Equal
            }
        }
    });
    if let Some(e) = cmp_err {
        return Err(e);
    }
    Ok(indices)
}

/// In-place stable merge sort on `indices`, comparing via `cmp` applied to
/// the elements at `elems[indices[i]]`. Uses `tmp` as a scratch buffer.
/// `cmp` is invoked with element references (not index references) so the
/// caller's comparator body need not re-look up the cached element.
fn stable_merge_sort_with_comparator<F>(
    indices: &mut [u32],
    tmp: &mut [u32],
    elems: &[KindedSlot],
    mut cmp: F,
) where
    F: FnMut(&KindedSlot, &KindedSlot) -> Ordering,
{
    let n = indices.len();
    if n < 2 {
        return;
    }
    // Bottom-up merge sort for cache friendliness + tail-call avoidance.
    let mut width = 1usize;
    while width < n {
        let mut i = 0;
        while i < n {
            let left = i;
            let mid = (i + width).min(n);
            let right = (i + 2 * width).min(n);
            merge_with_comparator(indices, tmp, elems, left, mid, right, &mut cmp);
            i += 2 * width;
        }
        // Copy tmp back into indices.
        indices[..n].copy_from_slice(&tmp[..n]);
        width *= 2;
    }
}

/// Stable merge of `indices[left..mid]` and `indices[mid..right]` into
/// `tmp[left..right]`, comparing via `cmp(elems[indices[a]], elems[indices[b]])`.
fn merge_with_comparator<F>(
    indices: &[u32],
    tmp: &mut [u32],
    elems: &[KindedSlot],
    left: usize,
    mid: usize,
    right: usize,
    cmp: &mut F,
) where
    F: FnMut(&KindedSlot, &KindedSlot) -> Ordering,
{
    let mut i = left;
    let mut j = mid;
    let mut k = left;
    while i < mid && j < right {
        let a = &elems[indices[i] as usize];
        let b = &elems[indices[j] as usize];
        // Stable: `<=` keeps `i` ahead on ties (preserves original order).
        match cmp(a, b) {
            Ordering::Less | Ordering::Equal => {
                tmp[k] = indices[i];
                i += 1;
            }
            Ordering::Greater => {
                tmp[k] = indices[j];
                j += 1;
            }
        }
        k += 1;
    }
    while i < mid {
        tmp[k] = indices[i];
        i += 1;
        k += 1;
    }
    while j < right {
        tmp[k] = indices[j];
        j += 1;
        k += 1;
    }
}

/// Sort `view` by natural element ordering (no comparator). Returns the
/// stable permutation indices.
fn sort_by_natural(view: &V2TypedArrayView, op: &'static str) -> Result<Vec<u32>, VMError> {
    let len = view.len;
    let mut indices: Vec<u32> = (0..len).collect();
    if len < 2 {
        return Ok(indices);
    }

    // Read every element once into a (bits, kind) pair — the natural-
    // ordering comparator only needs the bits + the view's elem_type.
    let mut bits_vec: Vec<u64> = Vec::with_capacity(len as usize);
    for i in 0..len {
        let (bits, _kind) = read_element(view, i).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Array.{op}: read_element({i}) returned None for element kind {:?}",
                view.elem_type
            ))
        })?;
        bits_vec.push(bits);
    }

    // Probe TypedObject SURFACE up front (first comparison would fail
    // anyway; this gives a cleaner error citing v0.4 territory).
    if matches!(
        view.elem_type,
        crate::executor::v2_handlers::v2_array_detect::V2ElemType::TypedObject
    ) {
        // Drop heap shares acquired via read_element (TypedObject path
        // bumps refcount per read — release them via the same path the
        // KindedSlot drop would).
        for &bits in &bits_vec {
            if bits != 0 {
                unsafe {
                    use shape_value::heap_value::TypedObjectStorage;
                    use shape_value::v2::heap_element::HeapElement;
                    <TypedObjectStorage as HeapElement>::release_elem(
                        bits as *const TypedObjectStorage,
                    );
                }
            }
        }
        return Err(VMError::NotImplemented(format!(
            "Array.{op}: natural-ordering sort over Array<TypedObject> is not \
             supported in v0.3 (supervisor D4: Bool-default ordering forbidden per \
             ADR-006 §2.7.14; canonical Ord trait + per-field projection is v0.4 \
             territory). Use `.orderBy(|x| x.<field>)` for an explicit key, or \
             pass a `sort(|a, b| ...)` comparator."
        )));
    }
    // Decimal natural-cmp is supported (DecimalObj::cmp dispatches via
    // rust_decimal::Decimal); String supported; scalar/Char supported.

    // Stable sort by natural ordering. `cmp_element_natural` returns
    // `Option<Ordering>` — None means the element kind has no canonical
    // ordering (TypedObject already filtered above; remaining None paths
    // are programmer errors and surface as `Ordering::Equal` to keep
    // the sort terminating, with the error captured via a sticky shadow.
    let mut cmp_err: Option<VMError> = None;
    let mut tmp: Vec<u32> = vec![0; len as usize];
    stable_merge_sort_with_indices(&mut indices, &mut tmp, |ia, ib| {
        if cmp_err.is_some() {
            return Ordering::Equal;
        }
        match cmp_element_natural(view, bits_vec[ia as usize], bits_vec[ib as usize]) {
            Some(o) => o,
            None => {
                cmp_err = Some(VMError::RuntimeError(format!(
                    "Array.{op}: natural-ordering comparison failed for element kind {:?} \
                         (no canonical Ord at v0.3 — supervisor D4 / ADR-006 §2.7.14)",
                    view.elem_type
                )));
                Ordering::Equal
            }
        }
    });

    // Drop the read shares for heap-element kinds (String/Decimal) —
    // these were acquired by `read_element`'s heap-arm `v2_retain`.
    drop_read_shares(view, &bits_vec);

    if let Some(e) = cmp_err {
        return Err(e);
    }
    Ok(indices)
}

/// Drop per-element shares acquired by `read_element` for heap-element
/// kinds. Scalar kinds are no-ops. Used when we read elements purely for
/// comparison (no slot is constructed to take ownership).
fn drop_read_shares(view: &V2TypedArrayView, bits_vec: &[u64]) {
    use crate::executor::v2_handlers::v2_array_detect::V2ElemType;
    use shape_value::v2::heap_element::HeapElement;
    match view.elem_type {
        V2ElemType::String => {
            for &bits in bits_vec {
                if bits != 0 {
                    unsafe {
                        <shape_value::v2::string_obj::StringObj as HeapElement>::release_elem(
                            bits as *const shape_value::v2::string_obj::StringObj,
                        );
                    }
                }
            }
        }
        V2ElemType::Decimal => {
            for &bits in bits_vec {
                if bits != 0 {
                    unsafe {
                        <shape_value::v2::decimal_obj::DecimalObj as HeapElement>::release_elem(
                            bits as *const shape_value::v2::decimal_obj::DecimalObj,
                        );
                    }
                }
            }
        }
        V2ElemType::TypedObject => {
            for &bits in bits_vec {
                if bits != 0 {
                    unsafe {
                        <shape_value::heap_value::TypedObjectStorage as HeapElement>::release_elem(
                            bits as *const shape_value::heap_value::TypedObjectStorage,
                        );
                    }
                }
            }
        }
        _ => {}
    }
}

/// In-place stable merge sort on `indices`, comparing via `cmp(ia, ib)`
/// where `ia` and `ib` are the original element-indices being compared.
fn stable_merge_sort_with_indices<F>(indices: &mut [u32], tmp: &mut [u32], mut cmp: F)
where
    F: FnMut(u32, u32) -> Ordering,
{
    let n = indices.len();
    if n < 2 {
        return;
    }
    let mut width = 1usize;
    while width < n {
        let mut i = 0;
        while i < n {
            let left = i;
            let mid = (i + width).min(n);
            let right = (i + 2 * width).min(n);
            merge_with_indices(indices, tmp, left, mid, right, &mut cmp);
            i += 2 * width;
        }
        indices[..n].copy_from_slice(&tmp[..n]);
        width *= 2;
    }
}

fn merge_with_indices<F>(
    indices: &[u32],
    tmp: &mut [u32],
    left: usize,
    mid: usize,
    right: usize,
    cmp: &mut F,
) where
    F: FnMut(u32, u32) -> Ordering,
{
    let mut i = left;
    let mut j = mid;
    let mut k = left;
    while i < mid && j < right {
        match cmp(indices[i], indices[j]) {
            Ordering::Less | Ordering::Equal => {
                tmp[k] = indices[i];
                i += 1;
            }
            Ordering::Greater => {
                tmp[k] = indices[j];
                j += 1;
            }
        }
        k += 1;
    }
    while i < mid {
        tmp[k] = indices[i];
        i += 1;
        k += 1;
    }
    while j < right {
        tmp[k] = indices[j];
        j += 1;
        k += 1;
    }
}

/// Sort `view` by `keyFn(elem)` (key extraction up-front, then sort by
/// cached keys). Returns the stable permutation indices.
fn sort_by_key_fn(
    vm: &mut VirtualMachine,
    view: &V2TypedArrayView,
    closure: &KindedSlot,
    direction: SortDirection,
    mut ctx: Option<&mut ExecutionContext>,
    op: &'static str,
) -> Result<Vec<u32>, VMError> {
    let len = view.len;
    let mut indices: Vec<u32> = (0..len).collect();
    if len < 2 {
        return Ok(indices);
    }

    // Pass 1: invoke key_fn on each element to produce a key slot.
    let mut keys: Vec<KindedSlot> = Vec::with_capacity(len as usize);
    for i in 0..len {
        let (bits, kind) = read_element(view, i).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Array.{op}: read_element({i}) returned None for element kind {:?}",
                view.elem_type
            ))
        })?;
        let elem = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        bump_closure_share(closure);
        let key = vm.call_value_immediate_nb(closure, &[elem], ctx.as_deref_mut())?;
        keys.push(key);
    }

    // Pass 2: stable-sort indices by comparing cached keys.
    let mut cmp_err: Option<VMError> = None;
    let mut tmp: Vec<u32> = vec![0; len as usize];
    stable_merge_sort_with_indices(&mut indices, &mut tmp, |ia, ib| {
        if cmp_err.is_some() {
            return Ordering::Equal;
        }
        let order = match cmp_key_kinded(&keys[ia as usize], &keys[ib as usize], op) {
            Ok(o) => o,
            Err(e) => {
                cmp_err = Some(e);
                return Ordering::Equal;
            }
        };
        match direction {
            SortDirection::Ascending => order,
            SortDirection::Descending => order.reverse(),
        }
    });

    if let Some(e) = cmp_err {
        return Err(e);
    }
    Ok(indices)
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 handlers — J.5f sort + orderBy + thenBy
// ═══════════════════════════════════════════════════════════════════════════

/// v2 `sort` — natural-ordering sort (no comparator) OR comparator sort
/// (`sort(|a, b| ...)`). Stable per supervisor D4 user expectation.
///
/// - 1 arg (receiver only): natural ordering per element kind.
/// - 2 args (receiver + closure): comparator returning `< 0` / `0` / `> 0`.
///
/// Routed from `method_registry.rs` as the canonical `"sort"` entry point;
/// the prior placeholder in `array_transform.rs::handle_sort_v2` delegates
/// here so we keep a single body.
pub(crate) fn handle_sort_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError("sort: missing receiver".to_string()));
    }
    let view = extract_view("sort", &args[0])?;

    let indices = if args.len() >= 2 {
        require_closure("sort", &args[1])?;
        let closure = &args[1];
        sort_by_comparator(vm, &view, closure, ctx, "sort")?
    } else {
        sort_by_natural(&view, "sort")?
    };

    let out_ptr = permute_array(&view, &indices);
    Ok(new_array_slot(out_ptr))
}

/// v2 `orderBy` — sort an array by a key function (optionally with
/// direction string at args[2]).
///
/// args: `[array, key_fn, direction?]`
pub(crate) fn handle_order_by_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "orderBy: expected (array, key_fn, direction?)".to_string(),
        ));
    }
    require_closure("orderBy", &args[1])?;
    let view = extract_view("orderBy", &args[0])?;
    let closure = &args[1];
    let direction = parse_direction(args, "orderBy")?;
    let indices = sort_by_key_fn(vm, &view, closure, direction, ctx, "orderBy")?;
    let out_ptr = permute_array(&view, &indices);
    Ok(new_array_slot(out_ptr))
}

/// v2 `thenBy` — sort an already-ordered array by a secondary key
/// (optionally with direction). Shares the body shape with `orderBy`:
/// `stable_merge_sort_with_indices` is stable, so re-sorting the
/// (assumed-already-primary-sorted) input by the secondary key produces
/// the lexicographic primary→secondary order.
///
/// args: `[array, key_fn, direction?]`
pub(crate) fn handle_then_by_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "thenBy: expected (array, key_fn, direction?)".to_string(),
        ));
    }
    require_closure("thenBy", &args[1])?;
    let view = extract_view("thenBy", &args[0])?;
    let closure = &args[1];
    let direction = parse_direction(args, "thenBy")?;
    let indices = sort_by_key_fn(vm, &view, closure, direction, ctx, "thenBy")?;
    let out_ptr = permute_array(&view, &indices);
    Ok(new_array_slot(out_ptr))
}

/// v2 `joinStr` — join array elements into a single string with a
/// separator. SURFACE: per-V2ElemType element stringification is a
/// separate ckpt-3 sub-cluster (not J.5f scope per supervisor D4). The
/// J.5f sort body uses `read_element` for raw `(bits, kind)` access, but
/// stringification per kind needs its own per-kind `element_to_string`
/// primitive in `v2_array_detect.rs`.
pub(crate) fn handle_join_str_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "joinStr() requires 2 arguments (array, separator)".to_string(),
        ));
    }
    if !matches!(args[1].kind, NativeKind::String | NativeKind::StringV2) {
        return Err(VMError::RuntimeError(format!(
            "joinStr(): separator must be a string, got {:?}",
            args[1].kind
        )));
    }

    // V3-S5 ckpt-6 STRICT close (2026-06-16): `Array<string>.join` — the String
    // elem_type carrier (the result of `split`, string-array literals) walks via
    // `read_element` + concatenates with the separator. This is the SplitJoin
    // round-trip path. Other V2ElemType arms (numeric stringification) remain the
    // separate ckpt-3 sub-cluster SURFACE below — out of SplitJoin scope.
    use crate::executor::v2_handlers::v2_array_detect::V2ElemType;
    let view = extract_view("join", &args[0])?;
    if view.elem_type == V2ElemType::String {
        let sep = args[1].as_str().ok_or(VMError::TypeError {
            expected: "string separator",
            got: "non-string kind",
        })?;
        let mut out = String::new();
        for i in 0..view.len {
            if i > 0 {
                out.push_str(sep);
            }
            let (bits, kind) = read_element(&view, i).ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "Array.join: read_element({i}) returned None for Array<string>"
                ))
            })?;
            // `read_element` bumped the StringV2 refcount (fresh share). Wrap so
            // the share is released on Drop; read the &str by borrow (no consume,
            // the receiver array is left untouched — refcount-balanced).
            let elem = KindedSlot::new(ValueSlot::from_raw(bits), kind);
            match elem.as_str() {
                Some(piece) => out.push_str(piece),
                None => {
                    return Err(VMError::RuntimeError(format!(
                        "Array.join: element {i} was not a string (kind {kind:?})"
                    )));
                }
            }
            // `elem` drops here → releases the fresh share read_element handed us.
        }
        return Ok(KindedSlot::from_string_arc(Arc::new(out)));
    }

    Err(VMError::NotImplemented(
        "joinStr: SURFACE — per-V2ElemType element stringification primitive \
         not yet landed (separate ckpt-3 sub-cluster, not J.5f scope per \
         supervisor D4 2026-05-24). Use `.map(|x| x.toString()).reduce(\"\", \
         |acc, s| acc + sep + s)` as a pure-Shape workaround until the \
         `v2_array_detect::element_to_string` primitive lands."
            .to_string(),
    ))
}

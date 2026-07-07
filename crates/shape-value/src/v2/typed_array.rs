//! Typed contiguous array for v2 runtime.
//!
//! `TypedArray<T>` is a 24-byte `#[repr(C)]` heap object with a `HeapHeader`,
//! a pointer to a contiguous `T` buffer, length, and capacity. The compiler
//! monomorphizes: `Array<number>` and `Array<i32>` are different `TypedArray`
//! instantiations with no element-level type checking.
//!
//! ## Memory layout (24 bytes)
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//!   0       8   header (HeapHeader — refcount at offset 0)
//!   8       8   data (*mut T — pointer to contiguous T buffer)
//!  16       4   len (element count)
//!  20       4   cap (allocated capacity)
//! ```

use super::heap_header::{HEAP_KIND_V2_TYPED_ARRAY, HeapHeader};
use crate::{HeapKind, HeapValue, NativeKind};
use std::alloc::{Layout, alloc, dealloc, realloc};
use std::ptr;

/// Typed contiguous array with refcounted header.
///
/// Allocated on the heap via raw allocator. The `data` pointer points to a
/// separate allocation holding `cap` elements of type `T`.
#[repr(C)]
pub struct TypedArray<T> {
    /// 8-byte v2 heap header (refcount at offset 0).
    pub header: HeapHeader,
    /// Pointer to contiguous T buffer.
    pub data: *mut T,
    /// Number of elements currently stored.
    pub len: u32,
    /// Allocated capacity in number of elements.
    pub cap: u32,
}

// Compile-time size assertion.
const _: () = {
    assert!(std::mem::size_of::<TypedArray<f64>>() == 24);
    assert!(std::mem::size_of::<TypedArray<i32>>() == 24);
    assert!(std::mem::size_of::<TypedArray<u8>>() == 24);
    // Wave 2 Agent A1 (2026-05-14) — F32 + Char scalar monomorphizations.
    assert!(std::mem::size_of::<TypedArray<f32>>() == 24);
    assert!(std::mem::size_of::<TypedArray<char>>() == 24);
};

impl<T: Copy> TypedArray<T> {
    /// Allocate a new empty TypedArray with capacity 0.
    ///
    /// Returns a raw pointer to the heap-allocated array. The caller is
    /// responsible for eventually calling `drop_array` to free it.
    pub fn new() -> *mut Self {
        Self::with_capacity(0)
    }

    /// Allocate a new TypedArray with the given capacity.
    ///
    /// Returns a raw pointer to the heap-allocated array.
    pub fn with_capacity(cap: u32) -> *mut Self {
        let layout = Layout::new::<Self>();
        let ptr = unsafe { alloc(layout) as *mut Self };
        assert!(!ptr.is_null(), "allocation failed for TypedArray");

        let data = if cap > 0 {
            let data_layout = Layout::array::<T>(cap as usize).expect("invalid array layout");
            let data_ptr = unsafe { alloc(data_layout) as *mut T };
            assert!(!data_ptr.is_null(), "allocation failed for TypedArray data");
            data_ptr
        } else {
            ptr::null_mut()
        };

        unsafe {
            ptr::write(
                ptr,
                Self {
                    header: HeapHeader::new(HEAP_KIND_V2_TYPED_ARRAY),
                    data,
                    len: 0,
                    cap,
                },
            );
        }

        ptr
    }

    /// Create a TypedArray from a slice, copying all elements.
    pub fn from_slice(slice: &[T]) -> *mut Self {
        let len = slice.len() as u32;
        let ptr = Self::with_capacity(len);
        unsafe {
            if len > 0 {
                ptr::copy_nonoverlapping(slice.as_ptr(), (*ptr).data, slice.len());
            }
            (*ptr).len = len;
        }
        ptr
    }

    /// Get an element by index, returning `None` if out of bounds.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn get(this: *const Self, index: u32) -> Option<T> {
        unsafe {
            if index >= (*this).len {
                None
            } else {
                Some(ptr::read((*this).data.add(index as usize)))
            }
        }
    }

    /// Get an element by index without bounds checking.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`, and `index` must
    /// be less than the array's length.
    #[inline]
    pub unsafe fn get_unchecked(this: *const Self, index: u32) -> T {
        unsafe { ptr::read((*this).data.add(index as usize)) }
    }

    /// Set an element by index. Panics if out of bounds.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn set(this: *mut Self, index: u32, val: T) {
        unsafe {
            assert!(
                index < (*this).len,
                "TypedArray::set index {} out of bounds (len {})",
                index,
                (*this).len
            );
            ptr::write((*this).data.add(index as usize), val);
        }
    }

    /// Push an element, growing the buffer if necessary (doubling strategy).
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    pub unsafe fn push(this: *mut Self, val: T) {
        unsafe {
            let arr = &mut *this;
            if arr.len == arr.cap {
                Self::grow(this);
            }
            let arr = &mut *this;
            // If `grow` refused because the per-execution memory ceiling was
            // breached, capacity is unchanged and `len == cap` still holds.
            // Writing here would be an out-of-bounds store, so we skip the
            // write and leave `len` unchanged. The breach was recorded in the
            // thread-local budget (see `grow`) and the VM surfaces it as a
            // clean `VMError` at the next dispatch safepoint — execution is
            // being torn down, so the un-stored element is intentionally
            // dropped rather than corrupting memory. This never panics.
            if arr.len == arr.cap {
                return;
            }
            ptr::write(arr.data.add(arr.len as usize), val);
            arr.len += 1;
        }
    }

    /// Pop the last element, returning `None` if empty.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    pub unsafe fn pop(this: *mut Self) -> Option<T> {
        unsafe {
            let arr = &mut *this;
            if arr.len == 0 {
                None
            } else {
                arr.len -= 1;
                Some(ptr::read(arr.data.add(arr.len as usize)))
            }
        }
    }

    /// Get the number of elements.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn len(this: *const Self) -> u32 {
        unsafe { (*this).len }
    }

    /// Get the allocated capacity.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn capacity(this: *const Self) -> u32 {
        unsafe { (*this).cap }
    }

    /// Check if the array is empty.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn is_empty(this: *const Self) -> bool {
        unsafe { (*this).len == 0 }
    }

    /// Get the elements as a slice.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn as_slice<'a>(this: *const Self) -> &'a [T] {
        unsafe {
            if (*this).len == 0 {
                &[]
            } else {
                std::slice::from_raw_parts((*this).data, (*this).len as usize)
            }
        }
    }

    /// Get the elements as a mutable slice.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn as_mut_slice<'a>(this: *mut Self) -> &'a mut [T] {
        unsafe {
            if (*this).len == 0 {
                &mut []
            } else {
                std::slice::from_raw_parts_mut((*this).data, (*this).len as usize)
            }
        }
    }

    /// Deallocate the array and its data buffer.
    ///
    /// # Safety
    /// `ptr` must point to a `TypedArray<T>` that was allocated by this module.
    /// After calling this, `ptr` is invalid.
    pub unsafe fn drop_array(ptr: *mut Self) {
        unsafe {
            let arr = &*ptr;
            // Free the data buffer if it was allocated.
            if arr.cap > 0 && !arr.data.is_null() {
                let data_layout =
                    Layout::array::<T>(arr.cap as usize).expect("invalid array layout");
                dealloc(arr.data as *mut u8, data_layout);
            }
            // Free the TypedArray struct itself.
            let layout = Layout::new::<Self>();
            dealloc(ptr as *mut u8, layout);
        }
    }

    /// Grow the data buffer (doubling strategy, minimum 4).
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    unsafe fn grow(this: *mut Self) {
        unsafe {
            let arr = &mut *this;
            let new_cap = if arr.cap == 0 {
                4
            } else {
                arr.cap.checked_mul(2).expect("capacity overflow")
            };
            let new_layout = Layout::array::<T>(new_cap as usize).expect("invalid array layout");

            // Enforce the per-execution per-buffer heap ceiling, if installed.
            // A doubling realloc can jump several GB in a single instruction,
            // so a memory ceiling — not the instruction cap — is what bounds
            // the RSS of an allocation-heavy runaway loop (the canonical case:
            // one buffer growing without bound). Over the ceiling => fail
            // in-process here rather than letting RSS climb until the host
            // OOM-killer reaps the process. No ceiling (CLI default) => no-op.
            // Over the ceiling: record the breach on the thread-local budget
            // and REFUSE to grow (leave cap/len unchanged) rather than
            // `panic!`-ing. A panic here aborts the whole process (exit 101)
            // — on a serve node that kills every in-flight request; on the
            // CLI it is an uncatchable crash on untrusted input. The VM's
            // caller (`push`) sees the unchanged capacity and skips its write;
            // the dispatch loop drains the recorded breach and surfaces a
            // clean `VMError` (graceful non-101 exit). Because the offending
            // buffer never grows past this point, its size is bounded exactly
            // at the ceiling — the memory DoS is contained here.
            if let Err(e) = crate::v2::alloc_budget::check_size(new_layout.size() as u64) {
                crate::v2::alloc_budget::record_breach(e);
                return;
            }

            let new_data = if arr.cap == 0 || arr.data.is_null() {
                alloc(new_layout) as *mut T
            } else {
                let old_layout =
                    Layout::array::<T>(arr.cap as usize).expect("invalid array layout");
                realloc(arr.data as *mut u8, old_layout, new_layout.size()) as *mut T
            };
            assert!(!new_data.is_null(), "reallocation failed for TypedArray");

            arr.data = new_data;
            arr.cap = new_cap;
        }
    }
}

/// **Shared** read-only element-pointer enumeration for a heap-element
/// `TypedArray` — the single source of truth for *which* elements are the
/// array's heap children (real-gc-cycle-collection.md §3.4).
///
/// Walks `data[0..len]` through the 8-byte pointer view. Every heap-element
/// monomorphization (`*const StringObj` / `*const DecimalObj` /
/// `*const TypedObjectStorage` / `*const TraitObjectStorage` /
/// `*const TypedArrayElem`) is layout-identical to `TypedArray<*const u8>`
/// (repr(C): header @0, data @8 = one 8-byte pointer regardless of `T`,
/// len @16, cap @20), so one non-generic walk serves them all. Yields each
/// stored element pointer as raw bits, in slot order, for **every** element in
/// `0..len` (no null/zero filter) so the destructive release walk it powers
/// (`drop_array_heap`) stays byte-identical to its pre-extraction form —
/// consumers apply their own per-edge filter (the GC visitor skips null).
///
/// This is the ONE enumeration both the destructive `Drop` path
/// (`drop_array_heap`, which releases each yielded element via the
/// monomorphized `T::release_elem`) and the read-only GC cycle visitor
/// (`crate::gc_visit::for_each_typed_array_heap_child`, `gc` feature, which
/// traces each yielded element) consume, so the collector's trace and the Drop
/// walk cannot drift on the array's heap-child edge set.
///
/// ## Scope boundary (§3.5 deferral — documented, not silent)
///
/// The 16-byte `CallableArrayElem` carrier is NOT a uniform 8-byte pointer
/// element (it packs inline function/module ids beside closure `Arc` shares);
/// it is walked by its own `drop_array_callable` and is **not** enumerated
/// here. Its GC edge set — like container-internal edges (HashMap/HashSet/
/// Deque values) and the header-less closure `OwnedMutable`/`Shared` interior
/// captures — belongs to the Phase 3.5 side-table mechanism (§3.5) and is a
/// known deferral, not a parity gap.
///
/// # Safety
/// `ptr` must point to a live `TypedArray<*const T>` whose element type is a
/// HeapHeader-equipped pointer carrier (`T: HeapElement`) — i.e. its stamped
/// `_pad` discriminant is one of the heap-element discriminants. The yielded
/// bits are borrowed views; this function performs no refcount work.
pub(crate) unsafe fn for_each_typed_array_elem_ptr<F>(ptr: *const u8, mut f: F)
where
    F: FnMut(*const u8),
{
    // SAFETY: every pointer-element monomorphization shares this layout.
    let arr = unsafe { &*(ptr as *const TypedArray<*const u8>) };
    if arr.data.is_null() {
        return;
    }
    for i in 0..arr.len {
        // SAFETY: `i < len <= cap` and `data` is non-null ⇒ in-bounds read of
        // a stored element pointer. Pointers are `Copy`; no ownership moves.
        let elem = unsafe { *arr.data.add(i as usize) };
        f(elem);
    }
}

/// Heap-element-aware drop dispatch for `TypedArray<*const T>` where `T:
/// HeapElement`.
///
/// Per ADR-006 §2.7.24 Q25.A SUPERSEDED + R20 S2-prime audit deliverable (b)
/// §4.1.B decision: `drop_array_heap` walks the element buffer and calls
/// `T::release_elem(elem_ptr)` for each stored pointer, then frees the data
/// buffer + the TypedArray struct itself. Per-T dispatch is monomorphized at
/// compile time via the `HeapElement` trait — no runtime `NativeKind` probe.
///
/// Pairs with the POD-element `drop_array` for `T: Copy` (above). Callers
/// pick at compile time based on whether the element type is POD (plain
/// scalar like f64/i64) or HeapHeader-equipped (`*const StringObj` /
/// `*const DecimalObj` / ...).
impl<T: super::heap_element::HeapElement> TypedArray<*const T> {
    /// Deallocate the array, releasing per-element shares via
    /// `T::release_elem`, then freeing the data buffer + the struct.
    ///
    /// The destructive element walk **enumerates through the shared
    /// [`for_each_typed_array_elem_ptr`] primitive** — the very enumeration the
    /// read-only GC cycle visitor consumes — and releases each yielded element
    /// via the monomorphized `T::release_elem`. Read-here / release-there over
    /// one primitive ⇒ the Drop walk and the collector's trace cannot drift
    /// (real-gc-cycle-collection.md §3.4). Only the enumeration moved to the
    /// shared primitive; the per-element release (and its Miri provenance) is
    /// unchanged, so Drop semantics stay byte-identical.
    ///
    /// # Safety
    /// `ptr` must point to a `TypedArray<*const T>` that was allocated by
    /// this module. Each stored `*const T` must be a valid pointer to a
    /// live `T` allocation with at least one refcount share owned by this
    /// array. After this call, `ptr` is invalid.
    pub unsafe fn drop_array_heap(ptr: *mut Self) {
        unsafe {
            let arr = &*ptr;
            if arr.cap > 0 && !arr.data.is_null() {
                // Enumerate the element buffer via the SHARED primitive (the
                // same one the read-only GC visitor consumes, §3.4) and release
                // each yielded element share via the monomorphized
                // `T::release_elem`. Same 0..len order, same per-element
                // release, byte-identical to the pre-extraction inline walk.
                for_each_typed_array_elem_ptr(ptr as *const u8, |elem_ptr| {
                    T::release_elem(elem_ptr as *const T);
                });
                // Free the data buffer.
                let data_layout =
                    Layout::array::<*const T>(arr.cap as usize).expect("invalid array layout");
                dealloc(arr.data as *mut u8, data_layout);
            }
            // Free the TypedArray struct itself.
            let layout = Layout::new::<Self>();
            dealloc(ptr as *mut u8, layout);
        }
    }
}

// ── Element-type discriminants — canonical home ──────────────────────────────
//
// The compile-time element type `T` of a `TypedArray<T>` is preserved at
// runtime in the `_pad` byte (offset 7) of the `HeapHeader`. The bytecode
// compiler / VM allocation handlers stamp this byte immediately after
// `with_capacity`; `retain_v2_typed_array` / `release_v2_typed_array` below
// (and the `shape-vm` consumer paths in `v2_handlers/v2_array_detect.rs`,
// re-exporting these constants) read it to pick the monomorphized
// `drop_array` / `drop_array_heap`.
//
// This is the *canonical* definition; `shape-vm::executor::v2_handlers::
// v2_array_detect` re-exports it via `pub use`. r5c-2-β-δ-(α): moved here
// from `v2_array_detect` so the kind-blind release function below — needed
// by the 4 `Ptr(HeapKind::TypedArray)` lockstep dispatch tables, two of
// which live in this `shape-value` crate (`kinded_slot.rs`, `closure_layout.
// rs`, `heap_value.rs`) — can dispatch without a constant duplicated across
// the `shape-vm` crate boundary.

/// `_pad`-byte discriminant for an unstamped / unknown element type.
pub const ELEM_TYPE_UNKNOWN: u8 = 0;
/// `_pad`-byte discriminant for `TypedArray<f64>`.
pub const ELEM_TYPE_F64: u8 = 1;
/// `_pad`-byte discriminant for `TypedArray<i64>`.
pub const ELEM_TYPE_I64: u8 = 2;
/// `_pad`-byte discriminant for `TypedArray<i32>`.
pub const ELEM_TYPE_I32: u8 = 3;
/// `_pad`-byte discriminant for `TypedArray<u8>` carrying `bool` elements.
pub const ELEM_TYPE_BOOL: u8 = 4;
/// `_pad`-byte discriminant for `TypedArray<i8>`.
pub const ELEM_TYPE_I8: u8 = 5;
/// `_pad`-byte discriminant for `TypedArray<u8>` carrying `u8` elements.
pub const ELEM_TYPE_U8: u8 = 6;
/// `_pad`-byte discriminant for `TypedArray<i16>`.
pub const ELEM_TYPE_I16: u8 = 7;
/// `_pad`-byte discriminant for `TypedArray<u16>`.
pub const ELEM_TYPE_U16: u8 = 8;
/// `_pad`-byte discriminant for `TypedArray<u32>`.
pub const ELEM_TYPE_U32: u8 = 9;
// Discriminant 10 reserved for `Array<u64>` (deferred — see v2_array_detect).
/// `_pad`-byte discriminant for `TypedArray<f32>`.
pub const ELEM_TYPE_F32: u8 = 11;
/// `_pad`-byte discriminant for `TypedArray<char>`.
pub const ELEM_TYPE_CHAR: u8 = 12;
/// `_pad`-byte discriminant for `TypedArray<*const StringObj>`.
pub const ELEM_TYPE_STRING: u8 = 13;
/// `_pad`-byte discriminant for `TypedArray<*const DecimalObj>`.
pub const ELEM_TYPE_DECIMAL: u8 = 14;
/// `_pad`-byte discriminant for `TypedArray<*const TypedObjectStorage>`.
pub const ELEM_TYPE_TYPED_OBJECT: u8 = 15;
/// `_pad`-byte discriminant for `TypedArray<*const TypedArrayElem>` — a
/// nested array whose elements are themselves v2-raw `TypedArray<U>` pointers
/// (any inner element monomorphization). Construction strict-typing close
/// (USER RULING 2026-06-05): `[[1,2],[3,4]]` lowers the outer literal to this
/// carrier. The element pointer is a `*const TypedArrayElem` (HeapHeader at
/// offset 0), and per-element release dispatches through the kind-erased
/// [`release_v2_typed_array`], which reads the INNER array's own `_pad`
/// element-type discriminant to pick the inner monomorphized drop. No
/// runtime NativeKind probe at the outer layer; the inner discriminant is the
/// inner array's own producer-side stamp (ADR-006 §2.7.5).
pub const ELEM_TYPE_TYPED_ARRAY: u8 = 16;
/// `_pad`-byte discriminant for `TypedArray<*const TraitObjectStorage>` —
/// the backing carrier for `Array<dyn Trait>` literals (Phase 4b W16.2-B
/// op_new_array-trait-object-element, 2026-06-05). Per ADR-006 §2.7.5
/// stamp-at-compile-time + §2.7.24 Q25.C, the stored element is a
/// `*const TraitObjectStorage` (HeapHeader at offset 0); per-element release
/// dispatches through `TraitObjectStorage::release_elem` (heap_value.rs:3092)
/// which calls `v2_release` on the on-header refcount and, at refcount=0,
/// `_drop`s the inner TypedObject share + the vtable Arc. Mirror of
/// `ELEM_TYPE_TYPED_OBJECT`.
pub const ELEM_TYPE_TRAIT_OBJECT: u8 = 17;
/// `_pad`-byte discriminant for `TypedArray<CallableArrayElem>` — the backing
/// carrier for compile-time-proven `Array<Function<...>>` literals. Elements are
/// small descriptors, not `HeapHeader` pointers: closures own one
/// `Arc<HeapValue>` share, named functions carry an inline `UInt64` function id,
/// and module functions carry an inline `Ptr(HeapKind::ModuleFn)` id.
pub const ELEM_TYPE_CALLABLE: u8 = 18;

/// Read the element-type discriminant stamped in the `_pad` byte (offset 7).
///
/// # Safety
/// `ptr` must point to a live `TypedArray<T>` (HeapHeader at offset 0).
#[inline]
pub unsafe fn read_elem_type(ptr: *const u8) -> u8 {
    unsafe { *ptr.add(7) }
}

/// Stamp the element-type discriminant into the `_pad` byte (offset 7) of a
/// freshly-allocated `TypedArray<T>` header.
///
/// This is the write-side sibling of [`read_elem_type`] and the canonical
/// home of the stamp (alongside the `ELEM_TYPE_*` constants). `shape-vm`'s
/// `v2_handlers::v2_array_detect::stamp_elem_type` is the VM-side allocation
/// helper (with a null guard for the producer opcodes); this `shape-value`
/// entry lets cross-crate carrier producers that cannot reach the `shape-vm`
/// crate (the marshal-layer `ToSlot<Vec<Arc<HeapValue>>>` in `shape-runtime`,
/// STAGE K2) stamp the discriminant without duplicating the offset constant.
///
/// # Safety
/// `ptr` must point to a live `TypedArray<T>` (HeapHeader at offset 0) and be
/// non-null. `elem_type` must be the discriminant matching the array's
/// element monomorphization `T`.
#[inline]
pub unsafe fn stamp_elem_type(ptr: *mut u8, elem_type: u8) {
    unsafe { *ptr.add(7) = elem_type };
}

/// HeapHeader-view newtype for a NESTED `TypedArray` element.
///
/// `[[1,2],[3,4]]` lowers to `TypedArray<*const TypedArrayElem>`. Each stored
/// element is a `*const TypedArrayElem` — really a `*mut TypedArray<U>` for
/// some inner element monomorphization `U`, viewed only through its
/// `HeapHeader` at offset 0. The outer array never needs to know `U`: retain
/// touches only the refcount at offset 0; release dispatches through the
/// kind-erased [`release_v2_typed_array`], which reads the inner array's own
/// `_pad` discriminant. This keeps the per-T monomorphization discipline —
/// the outer carrier is a single concrete `TypedArray<*const TypedArrayElem>`
/// instantiation, NOT an `Arc<TypedArrayData>` / `TypedBuffer<T>` parallel
/// carrier (CLAUDE.md §Forbidden) — while the inner drop stays exact.
#[repr(C)]
pub struct TypedArrayElem {
    /// 8-byte v2 heap header (refcount at offset 0, element-type `_pad` at
    /// offset 7). This is the only field the outer array ever touches.
    pub header: HeapHeader,
}

// HeapElement impl per ADR-006 §2.7.24 Q25.A SUPERSEDED + §4.1.B decision.
// `release_elem` retires one share of the inner array via the kind-erased
// `release_v2_typed_array`, which reads the inner `_pad` discriminant and
// runs the matching inner monomorphized `drop_array` / `drop_array_heap`.
// No runtime NativeKind probe at this (outer) layer; the inner discriminant
// is the inner array's own producer-side stamp.
unsafe impl super::heap_element::HeapElement for TypedArrayElem {
    unsafe fn release_elem(ptr: *const Self) {
        unsafe { release_v2_typed_array(ptr as *mut u8) };
    }
}

/// Exact callable carrier shape stored in `TypedArray<CallableArrayElem>`.
///
/// This is intentionally not a `*const HeapHeader` element. Only closure values
/// are `Arc<HeapValue>` shares; named functions and module functions are inline
/// IDs. The `kind` byte records which callable shape the bits represent so array
/// reads can push the same `(bits, NativeKind)` shape consumed by call dispatch.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallableArrayElemKind {
    Closure = 1,
    FunctionId = 2,
    ModuleFn = 3,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallableArrayElem {
    pub bits: u64,
    pub kind: CallableArrayElemKind,
}

impl CallableArrayElem {
    #[inline]
    pub fn from_native_kind(bits: u64, kind: NativeKind) -> Option<Self> {
        let kind = match kind {
            NativeKind::Ptr(HeapKind::Closure) => CallableArrayElemKind::Closure,
            NativeKind::UInt64 => CallableArrayElemKind::FunctionId,
            NativeKind::Ptr(HeapKind::ModuleFn) => CallableArrayElemKind::ModuleFn,
            _ => return None,
        };
        Some(Self { bits, kind })
    }

    #[inline]
    pub fn native_kind(self) -> NativeKind {
        match self.kind {
            CallableArrayElemKind::Closure => NativeKind::Ptr(HeapKind::Closure),
            CallableArrayElemKind::FunctionId => NativeKind::UInt64,
            CallableArrayElemKind::ModuleFn => NativeKind::Ptr(HeapKind::ModuleFn),
        }
    }

    /// Retain one read/clone share for this callable element.
    ///
    /// # Safety
    /// For `Closure`, `bits` must be a live `Arc::into_raw(Arc<HeapValue>)`
    /// pointer. Inline function ids perform no refcount operation.
    #[inline]
    pub unsafe fn retain(self) {
        if self.bits == 0 {
            return;
        }
        if self.kind == CallableArrayElemKind::Closure {
            unsafe {
                std::sync::Arc::increment_strong_count(self.bits as *const HeapValue);
            }
        }
    }

    /// Release one owned share for this callable element.
    ///
    /// # Safety
    /// For `Closure`, `bits` must be a live `Arc::into_raw(Arc<HeapValue>)`
    /// pointer owned by the caller. Inline function ids perform no refcount
    /// operation.
    #[inline]
    pub unsafe fn release(self) {
        if self.bits == 0 {
            return;
        }
        if self.kind == CallableArrayElemKind::Closure {
            unsafe {
                std::sync::Arc::decrement_strong_count(self.bits as *const HeapValue);
            }
        }
    }
}

/// **Shared** read-only element enumeration for a `TypedArray<CallableArrayElem>`
/// — the single source of truth for the callable array's per-element carriers
/// (real-gc-cycle-collection.md §3.4).
///
/// Walks `data[0..len]` yielding each 16-byte `CallableArrayElem` by value (it
/// is `Copy`), in slot order, for **every** element (no filter) so both the
/// destructive release walk (`drop_array_callable`) and the read-only GC cycle
/// visitor (`crate::gc_visit`, `gc` feature) consume ONE enumeration and cannot
/// drift on *which* elements the array owns. `CallableArrayElem` is 16 bytes
/// (`{ bits: u64, kind: u8 }`, align 8) — NOT the 8-byte pointer view the
/// `for_each_typed_array_elem_ptr` primitive walks — so it has its own reader.
///
/// # Safety
/// `ptr` must point to a live `TypedArray<CallableArrayElem>` (its `_pad`
/// discriminant is `ELEM_TYPE_CALLABLE`). The yielded elements are `Copy`
/// borrowed views; this function performs no refcount work.
pub(crate) unsafe fn for_each_typed_array_callable_elem<F>(ptr: *const u8, mut f: F)
where
    F: FnMut(CallableArrayElem),
{
    // SAFETY: caller guarantees a live `TypedArray<CallableArrayElem>`.
    let arr = unsafe { &*(ptr as *const TypedArray<CallableArrayElem>) };
    if arr.data.is_null() {
        return;
    }
    for i in 0..arr.len {
        // SAFETY: `i < len <= cap` and `data` non-null ⇒ in-bounds read of a
        // stored element. `CallableArrayElem` is `Copy`; no ownership moves.
        let elem = unsafe { *arr.data.add(i as usize) };
        f(elem);
    }
}

impl TypedArray<CallableArrayElem> {
    /// Deallocate a callable array, releasing each stored closure share exactly
    /// once and freeing the element buffer + typed-array header.
    ///
    /// The element walk enumerates through the SHARED
    /// [`for_each_typed_array_callable_elem`] primitive — the same enumeration
    /// the read-only GC cycle visitor consumes — and releases each yielded
    /// element via `CallableArrayElem::release`. Read-here / release-there over
    /// one primitive ⇒ the Drop walk and the collector's trace cannot drift on
    /// the callable array's element set (real-gc-cycle-collection.md §3.4).
    ///
    /// # Safety
    /// `ptr` must point to a live `TypedArray<CallableArrayElem>` allocated by
    /// this module. Each closure element must own one `Arc<HeapValue>` share.
    pub unsafe fn drop_array_callable(ptr: *mut Self) {
        unsafe {
            let arr = &*ptr;
            if arr.cap > 0 && !arr.data.is_null() {
                for_each_typed_array_callable_elem(ptr as *const u8, |elem| {
                    elem.release();
                });
                let data_layout = Layout::array::<CallableArrayElem>(arr.cap as usize)
                    .expect("invalid array layout");
                dealloc(arr.data as *mut u8, data_layout);
            }
            let layout = Layout::new::<Self>();
            dealloc(ptr as *mut u8, layout);
        }
    }
}

/// Retain (bump the refcount of) a v2-raw `*mut TypedArray<T>` carrier.
///
/// This is the retain half of the `NativeKind::Ptr(HeapKind::TypedArray)`
/// dispatch arm shared by the four lockstep clone/drop tables (VM stack
/// `clone_with_kind`, `KindedSlot::clone`, `SharedCell::clone`,
/// `TypedObjectStorage` field clone). The element type is irrelevant for a
/// retain — only the `HeapHeader` refcount at offset 0 is touched.
///
/// # Safety
/// `ptr` must be a non-null `*mut TypedArray<T>` produced by this module's
/// allocator (`with_capacity` / `with_capacity_generic` / `from_slice`).
#[inline]
pub unsafe fn retain_v2_typed_array(ptr: *mut u8) {
    unsafe { super::refcount::v2_retain(ptr as *const HeapHeader) };
}

/// Release one refcount share of a v2-raw `*mut TypedArray<T>` carrier; on
/// the last share, read the stamped element type and free the array via the
/// matching monomorphized `drop_array` / `drop_array_heap`.
///
/// This is the release half of the `NativeKind::Ptr(HeapKind::TypedArray)`
/// dispatch arm. POD element kinds (`f64` / `i64` / `i32` / `i8` / `u8` /
/// `i16` / `u16` / `u32` / `f32` / `char` / `bool`) route to `drop_array`;
/// the heap-element kinds (`String` / `Decimal` / `TypedObject`) route to
/// `drop_array_heap`, which walks the buffer releasing per-element shares.
///
/// # Safety
/// `ptr` must be a non-null `*mut TypedArray<T>` produced by this module's
/// allocator, with `T` matching the stamped `_pad` discriminant, and the
/// caller must own exactly one refcount share being retired here. After the
/// last share is retired the pointer is invalid.
pub unsafe fn release_v2_typed_array(ptr: *mut u8) {
    unsafe {
        if !super::refcount::v2_release(ptr as *const HeapHeader) {
            return;
        }
        // Refcount reached zero — this thread owns the deallocation.
        // GC Phase 3a use-after-free guard: drop any stale candidate-buffer
        // entry before deallocating (additive `#[cfg]`-gated no-op, gc off).
        #[cfg(feature = "gc")]
        crate::gc::gc_note_object_freed(ptr as usize);
        match read_elem_type(ptr) {
            ELEM_TYPE_F64 => TypedArray::<f64>::drop_array(ptr as *mut TypedArray<f64>),
            ELEM_TYPE_I64 => TypedArray::<i64>::drop_array(ptr as *mut TypedArray<i64>),
            ELEM_TYPE_I32 => TypedArray::<i32>::drop_array(ptr as *mut TypedArray<i32>),
            ELEM_TYPE_BOOL | ELEM_TYPE_U8 => {
                TypedArray::<u8>::drop_array(ptr as *mut TypedArray<u8>)
            }
            ELEM_TYPE_I8 => TypedArray::<i8>::drop_array(ptr as *mut TypedArray<i8>),
            ELEM_TYPE_I16 => TypedArray::<i16>::drop_array(ptr as *mut TypedArray<i16>),
            ELEM_TYPE_U16 => TypedArray::<u16>::drop_array(ptr as *mut TypedArray<u16>),
            ELEM_TYPE_U32 => TypedArray::<u32>::drop_array(ptr as *mut TypedArray<u32>),
            ELEM_TYPE_F32 => TypedArray::<f32>::drop_array(ptr as *mut TypedArray<f32>),
            ELEM_TYPE_CHAR => TypedArray::<char>::drop_array(ptr as *mut TypedArray<char>),
            ELEM_TYPE_STRING => TypedArray::<*const super::string_obj::StringObj>::drop_array_heap(
                ptr as *mut TypedArray<*const super::string_obj::StringObj>,
            ),
            ELEM_TYPE_DECIMAL => {
                TypedArray::<*const super::decimal_obj::DecimalObj>::drop_array_heap(
                    ptr as *mut TypedArray<*const super::decimal_obj::DecimalObj>,
                )
            }
            ELEM_TYPE_TYPED_OBJECT => {
                TypedArray::<*const crate::heap_value::TypedObjectStorage>::drop_array_heap(
                    ptr as *mut TypedArray<*const crate::heap_value::TypedObjectStorage>,
                )
            }
            // Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05) —
            // `Array<dyn Trait>` carrier. Each element is a `*const
            // TraitObjectStorage`; `TraitObjectStorage::release_elem`
            // (heap_value.rs:3092) retires one share via the on-header
            // refcount, `_drop`-ing the inner TypedObject + vtable at 0.
            ELEM_TYPE_TRAIT_OBJECT => {
                TypedArray::<*const crate::heap_value::TraitObjectStorage>::drop_array_heap(
                    ptr as *mut TypedArray<*const crate::heap_value::TraitObjectStorage>,
                )
            }
            ELEM_TYPE_TYPED_ARRAY => {
                // Nested array. Each element is a `*const TypedArrayElem`
                // (inner `TypedArray<U>` viewed through its HeapHeader);
                // `TypedArrayElem::release_elem` re-enters this function for
                // the inner array, reading the inner `_pad` discriminant.
                TypedArray::<*const TypedArrayElem>::drop_array_heap(
                    ptr as *mut TypedArray<*const TypedArrayElem>,
                )
            }
            ELEM_TYPE_CALLABLE => TypedArray::<CallableArrayElem>::drop_array_callable(
                ptr as *mut TypedArray<CallableArrayElem>,
            ),
            // An unstamped (`ELEM_TYPE_UNKNOWN`) or unrecognised discriminant
            // at refcount-0 means the producer-side stamp contract was
            // violated. The element-buffer monomorphization is unknown so a
            // typed `drop_array` cannot run; free only the 24-byte struct
            // header (leaking the element buffer is strictly preferable to
            // a misaligned `dealloc` / use-after-free). This is a hard bug
            // upstream — surface it loudly in debug builds.
            other => {
                debug_assert!(
                    false,
                    "release_v2_typed_array: TypedArray at {:p} has unstamped \
                     element-type discriminant {} — producer-side stamp_elem_type \
                     contract violated (ADR-006 §2.7.7)",
                    ptr, other
                );
                let layout = Layout::new::<TypedArray<u8>>();
                dealloc(ptr, layout);
            }
        }
    }
}

/// GC Phase 3a (real-gc-cycle-collection.md §0 ratification / §3.3 CollectWhite):
/// **memory-only** free of a cycle-garbage `TypedArray`.
///
/// Frees the element buffer + the 24-byte struct header **WITHOUT** releasing
/// any per-element heap share. Only heap-element arrays (`String` / `Decimal` /
/// `TypedObject` / `TraitObject` / nested `TypedArray`) can be cycle members;
/// their element buffer is a contiguous run of 8-byte pointers, so the buffer
/// layout is element-type-agnostic (`*const ()` × `cap`) and the header is a
/// fixed 24 bytes for every `T`. The stored element shares are **not** retired
/// here — the White cycle peers are freed by the CollectWhite recursion, and
/// live (Black) children already had this array's edge removed by the
/// un-restored trial-decrement. Running the destructive element walk would
/// double-free / prematurely free, so it MUST be skipped.
///
/// POD-element arrays own no per-element share and can never be cycle members;
/// this asserts (debug) it is only ever handed a heap-element (or unstamped)
/// array and frees the buffer as an 8-byte-pointer run regardless.
///
/// # Safety
/// `ptr` must point to a live heap-element `TypedArray<*const T>` that the
/// collector has proven a White cycle member. Called at most once; the pointer
/// is invalid afterwards.
#[cfg(feature = "gc")]
pub unsafe fn free_v2_typed_array_memory_only(ptr: *mut u8) {
    unsafe {
        let arr = &*(ptr as *const TypedArray<*const u8>);
        let cap = arr.cap as usize;
        let data = arr.data as *mut u8;
        debug_assert!(
            matches!(
                read_elem_type(ptr),
                ELEM_TYPE_STRING
                    | ELEM_TYPE_DECIMAL
                    | ELEM_TYPE_TYPED_OBJECT
                    | ELEM_TYPE_TRAIT_OBJECT
                    | ELEM_TYPE_TYPED_ARRAY
                    | ELEM_TYPE_CALLABLE
                    | ELEM_TYPE_UNKNOWN
            ),
            "free_v2_typed_array_memory_only: POD-element array cannot be a cycle member"
        );
        if cap > 0 && !data.is_null() {
            // Buffer layout is element-type-dependent: every heap-*pointer*
            // element type is an 8-byte `*const _` run, but the CALLABLE
            // carrier stores 16-byte `CallableArrayElem` records
            // (`{ bits: u64, kind: u8 }`, align 8). Deallocating a CALLABLE
            // buffer as an 8-byte-pointer run is a mismatched-`dealloc` (UB) —
            // the real-#31 array is a CALLABLE array, so this branch is
            // load-bearing for the closure-in-array cycle free.
            let data_layout = if read_elem_type(ptr) == ELEM_TYPE_CALLABLE {
                Layout::array::<CallableArrayElem>(cap).expect("invalid array layout")
            } else {
                Layout::array::<*const u8>(cap).expect("invalid array layout")
            };
            dealloc(data, data_layout);
        }
        // Struct header is a fixed 24 bytes for every `T` (asserted above).
        let layout = Layout::new::<TypedArray<u8>>();
        dealloc(ptr, layout);
    }
}

/// GC §3.5-part2 (real-gc-cycle-collection.md): break the A→B cycle edge of a
/// White callable `TypedArray` (node A of the closure-in-array Finding #31
/// cycle) by zeroing the `bits` of every `Closure`-kind element in its buffer.
///
/// After this, the array's own `drop_array_callable` walk sees `bits == 0` at
/// each closure slot and `CallableArrayElem::release` no-ops there — so when the
/// array is later freed by the natural RC cascade
/// (`SharedCell::Drop → release_v2_typed_array → drop_array_callable`) it does
/// NOT re-drop the closure VALUE `Arc<HeapValue>` share. The collector drives
/// that share to zero itself by dropping the value Arc directly (§3.5-part2
/// Model 1), so re-dropping it here would be a double-free. `FunctionId` /
/// `ModuleFn` elements are inline (`release` is a no-op), so they are left
/// untouched.
///
/// # Safety
/// `ptr` must point to a live `TypedArray<CallableArrayElem>` (its `_pad`
/// discriminant is `ELEM_TYPE_CALLABLE`) that the collector has proven a White
/// cycle member. Performs no refcount work — only zeroes the neutered slots'
/// `bits`.
#[cfg(feature = "gc")]
pub unsafe fn gc_neuter_callable_closure_edges(ptr: *mut u8) {
    unsafe {
        debug_assert_eq!(
            read_elem_type(ptr),
            ELEM_TYPE_CALLABLE,
            "gc_neuter_callable_closure_edges: not a CALLABLE array"
        );
        let arr = &*(ptr as *const TypedArray<CallableArrayElem>);
        if arr.data.is_null() {
            return;
        }
        for i in 0..arr.len {
            // SAFETY: `i < len <= cap` and `data` non-null ⇒ in-bounds.
            let elem_ptr = arr.data.add(i as usize);
            if (*elem_ptr).kind == CallableArrayElemKind::Closure {
                (*elem_ptr).bits = 0;
            }
        }
    }
}

// Allocation + size-only operations available for non-Copy element types
// (e.g. `TypedObjectPtr` with manual `Drop`). Per ADR-006 §2.7.24 Q25.B
// SUPERSEDED + Wave 2 Round 3b C2-joint ckpt-1 — `HashMapData<V>` (in
// `crates/shape-value/src/heap_value.rs`) instantiates `TypedArray<V>` for
// `V = TypedObjectPtr` / `TraitObjectPtr` (transparent newtypes with manual
// Drop), which are not `Copy`. The methods here are size/allocation only —
// no `ptr::read` / `ptr::write` that would require `T: Copy` for soundness.
//
// Methods needing element copy semantics (`get_unchecked`, `set`, `push`,
// `pop`, `from_slice`) remain bounded by `T: Copy` in the impl block above;
// non-Copy element types use the per-element-Drop-aware paths in
// `HashMapValueElem::release_typed_array`.
impl<T> TypedArray<T> {
    /// Allocate a new empty TypedArray with capacity 0 — non-Copy variant.
    ///
    /// Returns a raw pointer to the heap-allocated array. The caller is
    /// responsible for eventually freeing it via `HashMapValueElem::
    /// release_typed_array` (for `HashMapData<V>` value buffers) or the
    /// equivalent per-T release path.
    #[doc(alias = "new")]
    pub fn new_generic() -> *mut Self {
        Self::with_capacity_generic(0)
    }

    /// Allocate a new TypedArray with the given capacity — non-Copy variant.
    ///
    /// Returns a raw pointer to the heap-allocated array. No elements are
    /// written; the data buffer is uninitialized memory of length `cap *
    /// size_of::<T>()`.
    #[doc(alias = "with_capacity")]
    pub fn with_capacity_generic(cap: u32) -> *mut Self {
        let layout = Layout::new::<Self>();
        let ptr = unsafe { alloc(layout) as *mut Self };
        assert!(!ptr.is_null(), "allocation failed for TypedArray");

        let data = if cap > 0 {
            let data_layout = Layout::array::<T>(cap as usize).expect("invalid array layout");
            let data_ptr = unsafe { alloc(data_layout) as *mut T };
            assert!(!data_ptr.is_null(), "allocation failed for TypedArray data");
            data_ptr
        } else {
            ptr::null_mut()
        };

        unsafe {
            ptr::write(
                ptr,
                Self {
                    header: HeapHeader::new(HEAP_KIND_V2_TYPED_ARRAY),
                    data,
                    len: 0,
                    cap,
                },
            );
        }

        ptr
    }

    /// Get the number of elements — non-Copy variant.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn len_generic(this: *const Self) -> u32 {
        unsafe { (*this).len }
    }

    /// Get the allocated capacity — non-Copy variant.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn capacity_generic(this: *const Self) -> u32 {
        unsafe { (*this).cap }
    }

    /// Check if the array is empty — non-Copy variant.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn is_empty_generic(this: *const Self) -> bool {
        unsafe { (*this).len == 0 }
    }

    /// Get the elements as a slice — non-Copy variant.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn as_slice_generic<'a>(this: *const Self) -> &'a [T] {
        unsafe {
            if (*this).len == 0 {
                &[]
            } else {
                std::slice::from_raw_parts((*this).data, (*this).len as usize)
            }
        }
    }
}

#[cfg(test)]
// 3.14 is an arbitrary test float, not a PI approximation.
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;

    #[test]
    fn test_size_of_typed_array() {
        assert_eq!(std::mem::size_of::<TypedArray<f64>>(), 24);
        assert_eq!(std::mem::size_of::<TypedArray<i32>>(), 24);
        assert_eq!(std::mem::size_of::<TypedArray<i64>>(), 24);
        assert_eq!(std::mem::size_of::<TypedArray<u8>>(), 24);
    }

    #[test]
    fn test_field_offsets() {
        let arr = TypedArray::<f64>::with_capacity(0);
        unsafe {
            let base = arr as *const u8 as usize;
            let header_offset = &(*arr).header as *const _ as usize - base;
            let data_offset = &(*arr).data as *const _ as usize - base;
            let len_offset = &(*arr).len as *const _ as usize - base;
            let cap_offset = &(*arr).cap as *const _ as usize - base;

            assert_eq!(header_offset, 0);
            assert_eq!(data_offset, 8);
            assert_eq!(len_offset, 16);
            assert_eq!(cap_offset, 20);

            TypedArray::drop_array(arr);
        }
    }

    /// WF-3B Defect B: pushing past the per-execution memory ceiling must NOT
    /// panic (a panic aborts the whole process / kills a serve node). `grow`
    /// records the breach and refuses to grow; `push` sees the unchanged
    /// capacity and skips its write, so the buffer is bounded exactly at the
    /// ceiling and the caller can surface a clean error. A green result here
    /// (rather than a test-binary abort) is the "no panic" proof.
    #[test]
    fn push_past_ceiling_does_not_panic_and_bounds_buffer() {
        use crate::v2::alloc_budget::{self, BudgetGuard};
        // Ceiling = 128 bytes → an i64 buffer may hold at most 16 elements
        // (128 / 8). The doubling grow trips when the NEW buffer would exceed
        // the ceiling.
        let _g = BudgetGuard::new(Some(128));
        let arr = TypedArray::<i64>::new();
        unsafe {
            for i in 0..100_000_i64 {
                TypedArray::push(arr, i);
            }
            // Buffer never grew past the ceiling: cap * 8 bytes <= 128.
            assert!(
                TypedArray::capacity(arr) as usize * 8 <= 128,
                "buffer must be bounded at the ceiling, got cap {}",
                TypedArray::capacity(arr)
            );
            // len is bounded by cap (push skipped once growth was refused).
            assert!(TypedArray::len(arr) <= TypedArray::capacity(arr));
            TypedArray::drop_array(arr);
        }
        // A breach was recorded for the VM to surface.
        assert!(
            alloc_budget::take_breach().is_some(),
            "grow must record a breach on refusal"
        );
    }

    #[test]
    fn test_new_empty() {
        let arr = TypedArray::<f64>::new();
        unsafe {
            assert_eq!(TypedArray::len(arr), 0);
            assert_eq!(TypedArray::capacity(arr), 0);
            assert!(TypedArray::is_empty(arr));
            assert_eq!((*arr).header.kind(), HEAP_KIND_V2_TYPED_ARRAY);
            assert_eq!((*arr).header.get_refcount(), 1);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_with_capacity() {
        let arr = TypedArray::<f64>::with_capacity(16);
        unsafe {
            assert_eq!(TypedArray::len(arr), 0);
            assert_eq!(TypedArray::capacity(arr), 16);
            assert!(TypedArray::is_empty(arr));
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_push_and_get_f64() {
        let arr = TypedArray::<f64>::new();
        unsafe {
            TypedArray::push(arr, 1.0);
            TypedArray::push(arr, 2.5);
            TypedArray::push(arr, 3.14);

            assert_eq!(TypedArray::len(arr), 3);
            assert!(!TypedArray::is_empty(arr));

            assert_eq!(TypedArray::get(arr, 0), Some(1.0));
            assert_eq!(TypedArray::get(arr, 1), Some(2.5));
            assert_eq!(TypedArray::get(arr, 2), Some(3.14));
            assert_eq!(TypedArray::get(arr, 3), None); // out of bounds

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_push_and_get_i32() {
        let arr = TypedArray::<i32>::new();
        unsafe {
            TypedArray::push(arr, 42);
            TypedArray::push(arr, -7);
            TypedArray::push(arr, 0);

            assert_eq!(TypedArray::len(arr), 3);
            assert_eq!(TypedArray::get(arr, 0), Some(42));
            assert_eq!(TypedArray::get(arr, 1), Some(-7));
            assert_eq!(TypedArray::get(arr, 2), Some(0));
            assert_eq!(TypedArray::get(arr, 3), None);

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_push_and_get_i64() {
        let arr = TypedArray::<i64>::new();
        unsafe {
            TypedArray::push(arr, i64::MAX);
            TypedArray::push(arr, i64::MIN);

            assert_eq!(TypedArray::get(arr, 0), Some(i64::MAX));
            assert_eq!(TypedArray::get(arr, 1), Some(i64::MIN));

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_push_and_get_u8_bool() {
        let arr = TypedArray::<u8>::new();
        unsafe {
            TypedArray::push(arr, 1u8); // true
            TypedArray::push(arr, 0u8); // false
            TypedArray::push(arr, 1u8); // true

            assert_eq!(TypedArray::len(arr), 3);
            assert_eq!(TypedArray::get(arr, 0), Some(1));
            assert_eq!(TypedArray::get(arr, 1), Some(0));
            assert_eq!(TypedArray::get(arr, 2), Some(1));

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_get_unchecked() {
        let arr = TypedArray::<f64>::from_slice(&[10.0, 20.0, 30.0]);
        unsafe {
            assert_eq!(TypedArray::get_unchecked(arr, 0), 10.0);
            assert_eq!(TypedArray::get_unchecked(arr, 1), 20.0);
            assert_eq!(TypedArray::get_unchecked(arr, 2), 30.0);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_set() {
        let arr = TypedArray::<f64>::from_slice(&[1.0, 2.0, 3.0]);
        unsafe {
            TypedArray::set(arr, 1, 99.0);
            assert_eq!(TypedArray::get(arr, 1), Some(99.0));

            // Other elements unchanged
            assert_eq!(TypedArray::get(arr, 0), Some(1.0));
            assert_eq!(TypedArray::get(arr, 2), Some(3.0));

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn test_set_out_of_bounds() {
        let arr = TypedArray::<f64>::from_slice(&[1.0, 2.0]);
        unsafe {
            TypedArray::set(arr, 5, 99.0);
            // Leak is fine in a panic test
        }
    }

    #[test]
    fn test_pop() {
        let arr = TypedArray::<i32>::from_slice(&[10, 20, 30]);
        unsafe {
            assert_eq!(TypedArray::pop(arr), Some(30));
            assert_eq!(TypedArray::len(arr), 2);

            assert_eq!(TypedArray::pop(arr), Some(20));
            assert_eq!(TypedArray::len(arr), 1);

            assert_eq!(TypedArray::pop(arr), Some(10));
            assert_eq!(TypedArray::len(arr), 0);

            assert_eq!(TypedArray::pop(arr), None);
            assert!(TypedArray::is_empty(arr));

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_from_slice() {
        let data = [1.0f64, 2.0, 3.0, 4.0, 5.0];
        let arr = TypedArray::from_slice(&data);
        unsafe {
            assert_eq!(TypedArray::len(arr), 5);
            assert_eq!(TypedArray::capacity(arr), 5);

            for (i, &expected) in data.iter().enumerate() {
                assert_eq!(TypedArray::get(arr, i as u32), Some(expected));
            }

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_from_empty_slice() {
        let arr = TypedArray::<f64>::from_slice(&[]);
        unsafe {
            assert_eq!(TypedArray::len(arr), 0);
            assert_eq!(TypedArray::capacity(arr), 0);
            assert!(TypedArray::is_empty(arr));
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_as_slice() {
        let arr = TypedArray::from_slice(&[10i32, 20, 30]);
        unsafe {
            let s = TypedArray::as_slice(arr);
            assert_eq!(s, &[10, 20, 30]);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_as_mut_slice() {
        let arr = TypedArray::from_slice(&[1.0f64, 2.0, 3.0]);
        unsafe {
            let s = TypedArray::as_mut_slice(arr);
            s[1] = 99.0;
            assert_eq!(TypedArray::get(arr, 1), Some(99.0));
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_as_slice_empty() {
        let arr = TypedArray::<f64>::new();
        unsafe {
            let s = TypedArray::as_slice(arr);
            assert!(s.is_empty());
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_capacity_growth() {
        let arr = TypedArray::<f64>::new();
        unsafe {
            // Start with cap 0, first push should grow to 4
            TypedArray::push(arr, 1.0);
            assert!(TypedArray::capacity(arr) >= 1);

            // Push enough to trigger several doublings
            for i in 2..=20 {
                TypedArray::push(arr, i as f64);
            }
            assert_eq!(TypedArray::len(arr), 20);

            // Verify all values
            for i in 0..20 {
                assert_eq!(TypedArray::get(arr, i), Some((i + 1) as f64));
            }

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_header_kind() {
        let arr = TypedArray::<f64>::new();
        unsafe {
            assert_eq!((*arr).header.kind(), HEAP_KIND_V2_TYPED_ARRAY);
            assert_eq!((*arr).header.get_refcount(), 1);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_drop_safety() {
        // Create and drop many arrays to verify no leaks (under Miri/valgrind).
        unsafe {
            for _ in 0..100 {
                let arr = TypedArray::<f64>::new();
                for i in 0..50 {
                    TypedArray::push(arr, i as f64);
                }
                TypedArray::drop_array(arr);
            }
            // Empty arrays
            for _ in 0..100 {
                let arr = TypedArray::<i32>::new();
                TypedArray::drop_array(arr);
            }
        }
    }

    #[test]
    fn test_get_out_of_bounds_returns_none() {
        let arr = TypedArray::<f64>::new();
        unsafe {
            // Empty array: any index is out of bounds
            assert_eq!(TypedArray::get(arr, 0), None);
            assert_eq!(TypedArray::get(arr, 100), None);
            assert_eq!(TypedArray::get(arr, u32::MAX), None);

            TypedArray::push(arr, 1.0);
            assert_eq!(TypedArray::get(arr, 0), Some(1.0));
            assert_eq!(TypedArray::get(arr, 1), None);

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_refcount_with_typed_array() {
        use crate::v2::refcount::{v2_get_refcount, v2_release, v2_retain};

        let arr = TypedArray::<f64>::from_slice(&[1.0, 2.0]);
        unsafe {
            let header_ptr = arr as *const HeapHeader;

            assert_eq!(v2_get_refcount(header_ptr), 1);

            v2_retain(header_ptr);
            assert_eq!(v2_get_refcount(header_ptr), 2);

            assert!(!v2_release(header_ptr)); // 2 -> 1
            assert_eq!(v2_get_refcount(header_ptr), 1);

            // Don't call v2_release to 0 here since we use drop_array for cleanup
            TypedArray::drop_array(arr);
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // drop_array_heap tests per ADR-006 §2.7.24 Q25.A SUPERSEDED + R20
    // S2-prime audit deliverable (b) §4.1.B.
    // ──────────────────────────────────────────────────────────────────────

    #[test]
    fn test_drop_array_heap_string_obj() {
        use crate::v2::string_obj::StringObj;
        unsafe {
            // Allocate a TypedArray<*const StringObj> with capacity 4.
            let arr: *mut TypedArray<*const StringObj> = TypedArray::with_capacity(4);
            // Push 3 StringObj pointers.
            let s1 = StringObj::new("hello");
            let s2 = StringObj::new("world");
            let s3 = StringObj::new("!");
            TypedArray::push(arr, s1 as *const StringObj);
            TypedArray::push(arr, s2 as *const StringObj);
            TypedArray::push(arr, s3 as *const StringObj);
            assert_eq!(TypedArray::len(arr), 3);
            // drop_array_heap releases per-element shares then dealloc the
            // buffer + struct.
            TypedArray::<*const StringObj>::drop_array_heap(arr);
        }
    }

    #[test]
    fn test_drop_array_heap_decimal_obj() {
        use crate::v2::decimal_obj::DecimalObj;
        use rust_decimal::Decimal;
        use rust_decimal::prelude::FromPrimitive;
        unsafe {
            let arr: *mut TypedArray<*const DecimalObj> = TypedArray::with_capacity(4);
            let d1 = DecimalObj::new(Decimal::from_f64(1.5).unwrap());
            let d2 = DecimalObj::new(Decimal::from_f64(2.5).unwrap());
            let d3 = DecimalObj::new(Decimal::ZERO);
            TypedArray::push(arr, d1 as *const DecimalObj);
            TypedArray::push(arr, d2 as *const DecimalObj);
            TypedArray::push(arr, d3 as *const DecimalObj);
            assert_eq!(TypedArray::len(arr), 3);
            TypedArray::<*const DecimalObj>::drop_array_heap(arr);
        }
    }

    #[test]
    fn test_drop_array_heap_empty() {
        use crate::v2::string_obj::StringObj;
        unsafe {
            // Empty TypedArray (no allocated buffer).
            let arr: *mut TypedArray<*const StringObj> = TypedArray::new();
            TypedArray::<*const StringObj>::drop_array_heap(arr);
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Wave 2 Agent A1 (2026-05-14) — F32 + Char monomorphization smokes.
    // ──────────────────────────────────────────────────────────────────────

    #[test]
    fn test_size_of_typed_array_f32_char() {
        assert_eq!(std::mem::size_of::<TypedArray<f32>>(), 24);
        assert_eq!(std::mem::size_of::<TypedArray<char>>(), 24);
    }

    #[test]
    fn test_push_and_get_f32() {
        let arr = TypedArray::<f32>::new();
        unsafe {
            TypedArray::push(arr, 1.5_f32);
            TypedArray::push(arr, 2.25_f32);
            TypedArray::push(arr, std::f32::consts::PI);
            assert_eq!(TypedArray::len(arr), 3);
            assert_eq!(TypedArray::get(arr, 0), Some(1.5_f32));
            assert_eq!(TypedArray::get(arr, 1), Some(2.25_f32));
            assert_eq!(TypedArray::get(arr, 2), Some(std::f32::consts::PI));
            assert_eq!(TypedArray::get(arr, 3), None);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_push_and_get_char() {
        let arr = TypedArray::<char>::new();
        unsafe {
            TypedArray::push(arr, 'a');
            TypedArray::push(arr, '☃');
            TypedArray::push(arr, '👋');
            assert_eq!(TypedArray::len(arr), 3);
            assert_eq!(TypedArray::get(arr, 0), Some('a'));
            assert_eq!(TypedArray::get(arr, 1), Some('☃'));
            assert_eq!(TypedArray::get(arr, 2), Some('👋'));
            assert_eq!(TypedArray::get(arr, 3), None);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_from_slice_f32() {
        let data: [f32; 5] = [1.0, 2.0, 3.0, 4.0, 5.0];
        let arr = TypedArray::from_slice(&data);
        unsafe {
            assert_eq!(TypedArray::len(arr), 5);
            for (i, &expected) in data.iter().enumerate() {
                assert_eq!(TypedArray::get(arr, i as u32), Some(expected));
            }
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_from_slice_char() {
        let data = ['h', 'i', '!'];
        let arr = TypedArray::from_slice(&data);
        unsafe {
            assert_eq!(TypedArray::len(arr), 3);
            for (i, &expected) in data.iter().enumerate() {
                assert_eq!(TypedArray::get(arr, i as u32), Some(expected));
            }
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_drop_array_heap_with_held_share() {
        use crate::v2::refcount::{v2_get_refcount, v2_retain};
        use crate::v2::string_obj::StringObj;
        unsafe {
            // Allocate one StringObj with refcount 2 (one for the array, one held
            // externally). drop_array_heap should decrement to 1, not deallocate.
            let arr: *mut TypedArray<*const StringObj> = TypedArray::with_capacity(2);
            let s = StringObj::new("shared");
            v2_retain(&(*s).header); // refcount = 2
            TypedArray::push(arr, s as *const StringObj);

            TypedArray::<*const StringObj>::drop_array_heap(arr);

            // External share still valid; refcount should be 1.
            assert_eq!(v2_get_refcount(&(*s).header), 1);
            assert_eq!(StringObj::as_str(s), "shared");
            // Clean up.
            StringObj::drop(s);
        }
    }

    // ── r5c-2-β-δ-(α): kind-blind retain / release regression tests ─────────

    /// `retain_v2_typed_array` bumps the on-header refcount; a paired
    /// `release_v2_typed_array` retires it. A POD-element array at refcount
    /// 1 is freed by the single release that drives the count to zero.
    #[test]
    fn release_v2_typed_array_pod_drop_balance() {
        use crate::v2::refcount::v2_get_refcount;
        unsafe {
            let arr = TypedArray::<i64>::with_capacity(4);
            TypedArray::push(arr, 10);
            TypedArray::push(arr, 20);
            TypedArray::push(arr, 30);
            super::stamp_elem_type_for_test(arr as *mut u8, ELEM_TYPE_I64);
            let hdr = arr as *const HeapHeader;
            assert_eq!(v2_get_refcount(hdr), 1);

            // Retain (mirror of the `Ptr(HeapKind::TypedArray)` clone arm).
            retain_v2_typed_array(arr as *mut u8);
            assert_eq!(v2_get_refcount(hdr), 2);

            // Release once — back to 1, NOT freed.
            release_v2_typed_array(arr as *mut u8);
            assert_eq!(v2_get_refcount(hdr), 1);
            // The array is still live and readable.
            assert_eq!(TypedArray::get(arr, 1), Some(20));

            // Final release drives the count to 0 → free (no leak, no
            // double-free; ASAN/miri would flag a misaligned dealloc).
            release_v2_typed_array(arr as *mut u8);
        }
    }

    /// A heap-element array (`TypedArray<*const StringObj>`) released at
    /// refcount 0 must route through `drop_array_heap`, retiring each
    /// element's per-pointer share. An externally-held element share
    /// survives the array's free.
    #[test]
    fn release_v2_typed_array_heap_elem_drop_balance() {
        use crate::v2::refcount::{v2_get_refcount, v2_retain};
        use crate::v2::string_obj::StringObj;
        unsafe {
            let arr = TypedArray::<*const StringObj>::with_capacity(2);
            super::stamp_elem_type_for_test(arr as *mut u8, ELEM_TYPE_STRING);
            let s = StringObj::new("kept");
            v2_retain(&(*s).header); // refcount 2: one for the array, one external.
            TypedArray::push(arr, s as *const StringObj);

            retain_v2_typed_array(arr as *mut u8); // array refcount 2.
            release_v2_typed_array(arr as *mut u8); // → 1, array still live.
            assert_eq!(StringObj::as_str(s), "kept");

            // Final release → array refcount 0 → drop_array_heap walks the
            // buffer, releasing the element's per-pointer share (2 → 1).
            release_v2_typed_array(arr as *mut u8);
            assert_eq!(v2_get_refcount(&(*s).header), 1);
            StringObj::drop(s);
        }
    }

    /// A repeated retain/release cycle (mirror of a closure-captured array
    /// read many times) leaves the refcount balanced — no drift.
    #[test]
    fn release_v2_typed_array_repeated_cycle_balances() {
        use crate::v2::refcount::v2_get_refcount;
        unsafe {
            let arr = TypedArray::<i64>::with_capacity(1);
            TypedArray::push(arr, 99);
            super::stamp_elem_type_for_test(arr as *mut u8, ELEM_TYPE_I64);
            let hdr = arr as *const HeapHeader;
            for _ in 0..1000 {
                retain_v2_typed_array(arr as *mut u8);
                release_v2_typed_array(arr as *mut u8);
            }
            assert_eq!(v2_get_refcount(hdr), 1);
            assert_eq!(TypedArray::get(arr, 0), Some(99));
            release_v2_typed_array(arr as *mut u8);
        }
    }
}

/// Test-only `_pad`-byte element-type stamp. The production stamp lives in
/// `shape-vm`'s `v2_array_detect::stamp_elem_type`; this mirror lets the
/// `shape-value` crate's own unit tests exercise `release_v2_typed_array`'s
/// stamped-element-type dispatch without a `shape-vm` dependency.
#[cfg(test)]
pub(crate) unsafe fn stamp_elem_type_for_test(ptr: *mut u8, elem_type: u8) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        *ptr.add(7) = elem_type;
    }
}

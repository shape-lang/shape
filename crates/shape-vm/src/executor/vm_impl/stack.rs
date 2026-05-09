//! Typed VM stack — kinded data + parallel `NativeKind` track (ADR-006 §2.7.7 / Q9).
//!
//! The VM stack carries two parallel arrays in lockstep:
//!
//! - `stack: Vec<u64>` — 8-byte raw payload per slot.
//! - `kinds: Vec<NativeKind>` — 1-byte interpretation per slot.
//!
//! Index invariant: `stack.len() == kinds.len()` at every API boundary; the
//! first `sp` slots are live, the remainder are pre-allocated dead space
//! (kind = `Bool` by convention so dead bits never leak refcount).
//!
//! WB2.4 retain-on-read uses the parallel kind track for kind-aware
//! clone/drop dispatch via [`clone_with_kind`] / [`drop_with_kind`]. No
//! tag decode, no `is_heap()` probe, no `as_heap_ref()` hop — the kind is
//! locally available at every retain/release site by construction (the
//! producing opcode emits it).
//!
//! The pre-Wave-6 `vw_clone(bits)` / `vw_drop(bits)` helpers (which did
//! tag-decode internally) are replaced by `clone_with_kind(bits, kind)` /
//! `drop_with_kind(bits, kind)`, which mirror `KindedSlot::Clone` /
//! `KindedSlot::Drop` (the canonical refcount-dispatch table in
//! `crates/shape-value/src/kinded_slot.rs`).
//!
//! See `docs/adr/006-value-and-memory-model.md` §2.7.7 and §17 Q9.

use super::super::*;
use shape_value::{
    KindedSlot, NativeKind, ValueSlot,
    heap_value::{
        HashMapData, HeapKind, IoHandleData, NativeViewData, TableViewData, TaskGroupData,
        TemporalData, TypedArrayData, TypedObjectStorage,
    },
};
use std::sync::Arc;

// ────────────────────────────────────────────────────────────────────────────
// `clone_with_kind` / `drop_with_kind` — WB2.4 retain-on-read primitives
// ────────────────────────────────────────────────────────────────────────────
//
// These mirror `KindedSlot::Clone` / `KindedSlot::Drop` in
// `crates/shape-value/src/kinded_slot.rs`. The dispatch tables MUST stay in
// lockstep — divergence is a refcount bug. If a new heap-bearing
// `NativeKind` variant lands, both this module's helpers and the
// `KindedSlot` impls must be updated together.

/// WB2.4 retain-on-read: bump the matching `Arc<T>` strong-count for a
/// heap-bearing kind, no-op for inline scalars (ADR-006 §2.7.7).
///
/// Mirror of `KindedSlot::clone` in `shape-value/src/kinded_slot.rs`.
#[inline]
pub(crate) fn clone_with_kind(bits: u64, kind: NativeKind) {
    if bits == 0 {
        return;
    }
    // SAFETY: per the construction-side contract on every push site, when
    // `kind` selects a heap arm the `bits` are the result of
    // `Arc::into_raw::<T>` for the matching `T`. We bump exactly one
    // strong-count share.
    unsafe {
        match kind {
            NativeKind::String => {
                Arc::increment_strong_count(bits as *const String);
            }
            NativeKind::Ptr(hk) => match hk {
                HeapKind::String => {
                    Arc::increment_strong_count(bits as *const String);
                }
                HeapKind::TypedArray => {
                    Arc::increment_strong_count(bits as *const TypedArrayData);
                }
                HeapKind::TypedObject => {
                    Arc::increment_strong_count(bits as *const TypedObjectStorage);
                }
                HeapKind::HashMap => {
                    Arc::increment_strong_count(bits as *const HashMapData);
                }
                HeapKind::Decimal => {
                    Arc::increment_strong_count(bits as *const rust_decimal::Decimal);
                }
                HeapKind::BigInt => {
                    Arc::increment_strong_count(bits as *const i64);
                }
                HeapKind::DataTable => {
                    Arc::increment_strong_count(bits as *const shape_value::DataTable);
                }
                HeapKind::IoHandle => {
                    Arc::increment_strong_count(bits as *const IoHandleData);
                }
                HeapKind::NativeView => {
                    Arc::increment_strong_count(bits as *const NativeViewData);
                }
                HeapKind::Content => {
                    Arc::increment_strong_count(
                        bits as *const shape_value::content::ContentNode,
                    );
                }
                HeapKind::Instant => {
                    Arc::increment_strong_count(bits as *const std::time::Instant);
                }
                HeapKind::Temporal => {
                    Arc::increment_strong_count(bits as *const TemporalData);
                }
                HeapKind::TableView => {
                    Arc::increment_strong_count(bits as *const TableViewData);
                }
                HeapKind::TaskGroup => {
                    Arc::increment_strong_count(bits as *const TaskGroupData);
                }
                // Char: inline-scalar payload (codepoint bits). No-op.
                HeapKind::Char => {}
                // Closure / Future / NativeScalar: no `Arc<T>` slot
                // payload routed through this path. A non-zero pointer
                // here is a construction-side bug; debug-assert and
                // silently no-op in release.
                HeapKind::Closure | HeapKind::Future | HeapKind::NativeScalar => {
                    debug_assert!(
                        false,
                        "clone_with_kind: non-zero bits with non-Arc-payload kind {:?}",
                        hk
                    );
                }
            },
            // Inline scalars: no refcount payload.
            NativeKind::Float64
            | NativeKind::NullableFloat64
            | NativeKind::Int8
            | NativeKind::NullableInt8
            | NativeKind::UInt8
            | NativeKind::NullableUInt8
            | NativeKind::Int16
            | NativeKind::NullableInt16
            | NativeKind::UInt16
            | NativeKind::NullableUInt16
            | NativeKind::Int32
            | NativeKind::NullableInt32
            | NativeKind::UInt32
            | NativeKind::NullableUInt32
            | NativeKind::Int64
            | NativeKind::NullableInt64
            | NativeKind::UInt64
            | NativeKind::NullableUInt64
            | NativeKind::IntSize
            | NativeKind::NullableIntSize
            | NativeKind::UIntSize
            | NativeKind::NullableUIntSize
            | NativeKind::Bool => {}
        }
    }
}

/// WB2.4 retain-on-read inverse: decrement the matching `Arc<T>`
/// strong-count for a heap-bearing kind, no-op for inline scalars
/// (ADR-006 §2.7.7).
///
/// Mirror of `KindedSlot::drop` in `shape-value/src/kinded_slot.rs`.
#[inline]
pub(crate) fn drop_with_kind(bits: u64, kind: NativeKind) {
    if bits == 0 {
        return;
    }
    // SAFETY: per the construction-side contract on every push site, when
    // `kind` selects a heap arm the `bits` are the result of
    // `Arc::into_raw::<T>` for the matching `T`. We retire exactly one
    // strong-count share.
    unsafe {
        match kind {
            NativeKind::String => {
                Arc::decrement_strong_count(bits as *const String);
            }
            NativeKind::Ptr(hk) => match hk {
                HeapKind::String => {
                    Arc::decrement_strong_count(bits as *const String);
                }
                HeapKind::TypedArray => {
                    Arc::decrement_strong_count(bits as *const TypedArrayData);
                }
                HeapKind::TypedObject => {
                    Arc::decrement_strong_count(bits as *const TypedObjectStorage);
                }
                HeapKind::HashMap => {
                    Arc::decrement_strong_count(bits as *const HashMapData);
                }
                HeapKind::Decimal => {
                    Arc::decrement_strong_count(bits as *const rust_decimal::Decimal);
                }
                HeapKind::BigInt => {
                    Arc::decrement_strong_count(bits as *const i64);
                }
                HeapKind::DataTable => {
                    Arc::decrement_strong_count(bits as *const shape_value::DataTable);
                }
                HeapKind::IoHandle => {
                    Arc::decrement_strong_count(bits as *const IoHandleData);
                }
                HeapKind::NativeView => {
                    Arc::decrement_strong_count(bits as *const NativeViewData);
                }
                HeapKind::Content => {
                    Arc::decrement_strong_count(
                        bits as *const shape_value::content::ContentNode,
                    );
                }
                HeapKind::Instant => {
                    Arc::decrement_strong_count(bits as *const std::time::Instant);
                }
                HeapKind::Temporal => {
                    Arc::decrement_strong_count(bits as *const TemporalData);
                }
                HeapKind::TableView => {
                    Arc::decrement_strong_count(bits as *const TableViewData);
                }
                HeapKind::TaskGroup => {
                    Arc::decrement_strong_count(bits as *const TaskGroupData);
                }
                // Char: inline-scalar payload. No-op.
                HeapKind::Char => {}
                HeapKind::Closure | HeapKind::Future | HeapKind::NativeScalar => {
                    debug_assert!(
                        false,
                        "drop_with_kind: non-zero bits with non-Arc-payload kind {:?}",
                        hk
                    );
                }
            },
            NativeKind::Float64
            | NativeKind::NullableFloat64
            | NativeKind::Int8
            | NativeKind::NullableInt8
            | NativeKind::UInt8
            | NativeKind::NullableUInt8
            | NativeKind::Int16
            | NativeKind::NullableInt16
            | NativeKind::UInt16
            | NativeKind::NullableUInt16
            | NativeKind::Int32
            | NativeKind::NullableInt32
            | NativeKind::UInt32
            | NativeKind::NullableUInt32
            | NativeKind::Int64
            | NativeKind::NullableInt64
            | NativeKind::UInt64
            | NativeKind::NullableUInt64
            | NativeKind::IntSize
            | NativeKind::NullableIntSize
            | NativeKind::UIntSize
            | NativeKind::NullableUIntSize
            | NativeKind::Bool => {}
        }
    }
}

impl VirtualMachine {
    // ── Index invariant assertion (debug-build cross-check) ────────────────

    /// Debug-build invariant: `stack.len() == kinds.len()` at every public
    /// API boundary. Compiles out in release builds (ADR-006 §2.7.7).
    #[inline(always)]
    pub(crate) fn debug_assert_kinds_in_sync(&self) {
        debug_assert_eq!(
            self.stack.len(),
            self.kinds.len(),
            "ADR-006 §2.7.7 index invariant: stack data and kinds tracks must stay in lockstep"
        );
    }

    // ── Kinded push / pop / read primitives (ADR-006 §2.7.7) ───────────────

    /// Cold path for `push_kinded`: grow the stack or return `StackOverflow`.
    /// Releases the overflow-dropped share via `drop_with_kind` (FR.1 / WB2.x).
    #[cold]
    #[inline(never)]
    pub(crate) fn push_kinded_slow(
        &mut self,
        bits: u64,
        kind: NativeKind,
    ) -> Result<(), VMError> {
        if self.sp >= self.config.max_stack_size {
            // Release the share that would have been pushed.
            drop_with_kind(bits, kind);
            return Err(VMError::StackOverflow);
        }
        let new_len = self.sp * 2 + 1;
        self.stack.reserve(new_len - self.stack.len());
        self.kinds.reserve(new_len - self.kinds.len());
        while self.stack.len() < new_len {
            self.stack.push(0u64);
            self.kinds.push(NativeKind::Bool);
        }
        self.stack[self.sp] = bits;
        self.kinds[self.sp] = kind;
        self.sp += 1;
        self.debug_assert_kinds_in_sync();
        Ok(())
    }

    /// Push a value onto the typed VM stack with its `NativeKind`
    /// (ADR-006 §2.7.7). The bits' interpretation is recorded in the
    /// parallel kinds track in lockstep with the data slot.
    ///
    /// **Ownership**: the caller transfers one strong-count share (for
    /// heap-bearing kinds) into the slot. The slot retires the share via
    /// `drop_with_kind` on subsequent overwrite / pop / VM teardown.
    #[inline(always)]
    pub(crate) fn push_kinded(&mut self, bits: u64, kind: NativeKind) -> Result<(), VMError> {
        if self.sp >= self.stack.len() {
            return self.push_kinded_slow(bits, kind);
        }
        unsafe {
            let dptr = self.stack.as_mut_ptr().add(self.sp);
            let kptr = self.kinds.as_mut_ptr().add(self.sp);
            std::ptr::write(dptr, bits);
            std::ptr::write(kptr, kind);
        }
        self.sp += 1;
        Ok(())
    }

    /// Pop the topmost slot from the typed VM stack, returning the raw bits
    /// **plus** their `NativeKind` (ADR-006 §2.7.7).
    ///
    /// **Ownership**: the slot's strong-count share (for heap-bearing kinds)
    /// transfers to the caller. The caller is responsible for retiring it
    /// via `drop_with_kind` (or transferring it elsewhere). Pop does NOT
    /// auto-drop — the bits are handed out live.
    #[inline(always)]
    pub(crate) fn pop_kinded(&mut self) -> Result<(u64, NativeKind), VMError> {
        if self.sp == 0 {
            return Err(VMError::StackUnderflow);
        }
        self.sp -= 1;
        let (bits, kind);
        unsafe {
            let dptr = self.stack.as_mut_ptr().add(self.sp);
            let kptr = self.kinds.as_mut_ptr().add(self.sp);
            bits = std::ptr::read(dptr as *const u64);
            kind = std::ptr::read(kptr as *const NativeKind);
            // Replace the dead slot with safe sentinel bits. Bool kind +
            // zero bits = no-op for `drop_with_kind` if anyone reads it.
            std::ptr::write(dptr, 0u64);
            std::ptr::write(kptr, NativeKind::Bool);
        }
        Ok((bits, kind))
    }

    /// Read an **owning share** of the slot at `idx` as a `KindedSlot`
    /// (ADR-006 §2.7.7). Bumps the underlying `Arc<T>` strong-count via
    /// `clone_with_kind` so the returned `KindedSlot` has an independent
    /// share; the slot itself stays live on the stack.
    ///
    /// Use this at every site that hands a slot to a runtime-tier carrier
    /// (`Vec<KindedSlot>` for builtin args, snapshot serialization, etc.).
    #[inline]
    pub(crate) fn read_owned_kinded(&self, idx: usize) -> KindedSlot {
        debug_assert!(idx < self.sp, "read_owned_kinded: idx out of live range");
        let bits = self.stack[idx];
        let kind = self.kinds[idx];
        clone_with_kind(bits, kind);
        KindedSlot::new(ValueSlot::from_raw(bits), kind)
    }

    /// Read the raw bits + kind at `idx` as a borrow (no refcount change).
    /// The caller MUST NOT drop the returned bits — the slot still owns
    /// the share. Symmetric to the pre-Wave-6 `stack_read_raw`.
    #[inline(always)]
    pub(crate) fn stack_read_kinded_raw(&self, idx: usize) -> (u64, NativeKind) {
        (self.stack[idx], self.kinds[idx])
    }

    /// Write a fresh kinded value into `stack[idx]`, releasing the previous
    /// occupant via `drop_with_kind`. The new slot owns the strong-count
    /// share transferred in by the caller (ADR-006 §2.7.7).
    #[inline(always)]
    pub(crate) fn stack_write_kinded(&mut self, idx: usize, bits: u64, kind: NativeKind) {
        let old_bits = self.stack[idx];
        let old_kind = self.kinds[idx];
        drop_with_kind(old_bits, old_kind);
        self.stack[idx] = bits;
        self.kinds[idx] = kind;
    }

    /// Take ownership of the slot at `idx`, replacing it with the
    /// zero/Bool sentinel. Does NOT drop — the caller owns the bits.
    #[inline(always)]
    pub(crate) fn stack_take_kinded(&mut self, idx: usize) -> (u64, NativeKind) {
        let bits = self.stack[idx];
        let kind = self.kinds[idx];
        self.stack[idx] = 0u64;
        self.kinds[idx] = NativeKind::Bool;
        (bits, kind)
    }

    /// Truncate the stack to `len` slots, dropping the share for every
    /// removed slot via `drop_with_kind` (ADR-006 §2.7.7 WB2.4).
    #[inline]
    pub(crate) fn truncate_stack(&mut self, len: usize) {
        if len >= self.sp {
            return;
        }
        for i in len..self.sp {
            let bits = self.stack[i];
            let kind = self.kinds[i];
            drop_with_kind(bits, kind);
            self.stack[i] = 0u64;
            self.kinds[i] = NativeKind::Bool;
        }
        self.sp = len;
        self.debug_assert_kinds_in_sync();
    }

    // ── Hash and frame helpers ─────────────────────────────────────────────

    pub(crate) fn blob_hash_for_function(&self, func_id: u16) -> Option<FunctionHash> {
        self.function_hashes
            .get(func_id as usize)
            .copied()
            .flatten()
    }

    pub(crate) fn current_locals_base(&self) -> usize {
        self.call_stack
            .last()
            .map(|frame| frame.base_pointer)
            .unwrap_or(0)
    }

    /// Look up the FrameDescriptor for the currently executing function.
    /// Returns None if no call frame is active or the active function has
    /// no FrameDescriptor (legacy bytecode).
    #[inline]
    pub(crate) fn current_frame_descriptor(
        &self,
    ) -> Option<&crate::type_tracking::FrameDescriptor> {
        let func_id = self.call_stack.last()?.function_id?;
        let func = self.program.functions.get(func_id as usize)?;
        func.frame_descriptor.as_ref()
    }

    // ────────────────────────────────────────────────────────────────────
    // ADR-006 §2.7.7 transitional shims — legacy-name forwarders to the
    // kinded API. Pre-Wave-6 callers in non-territory files (exceptions,
    // foreign_marshal, state_builtins, etc. — Waves 7-9) still call these
    // names. Each forwarder records `NativeKind::Bool` as the per-slot
    // kind, which is leak-free (Drop / Clone are no-ops for the Bool arm
    // regardless of the underlying bits). Migration of those call sites
    // to `push_kinded(bits, real_kind)` is owned by their respective
    // waves; until then the forwarders preserve the index invariant.
    //
    // Forbidden patterns this preserves: none. The shims do not decode
    // bits, do not probe tags, and do not reintroduce `vw_clone` /
    // `vw_drop`. The Bool default is the §2.7 sentinel — explicitly
    // Drop-safe per `KindedSlot::Drop` and `clone_with_kind`/`drop_with_kind`
    // in this module.
    // ────────────────────────────────────────────────────────────────────

    /// **Transitional shim.** Push raw u64 bits onto the stack with a
    /// `Bool` default kind. Callers that know their value's `NativeKind`
    /// should use `push_kinded(bits, kind)` directly.
    #[inline(always)]
    pub fn push_raw_u64(&mut self, bits: u64) -> Result<(), VMError> {
        self.push_kinded(bits, NativeKind::Bool)
    }

    /// **Transitional shim.** Pop raw u64 bits, discarding the per-slot
    /// `NativeKind`. Callers that care about the kind should use
    /// `pop_kinded()` directly.
    #[inline(always)]
    pub fn pop_raw_u64(&mut self) -> Result<u64, VMError> {
        self.pop_kinded().map(|(bits, _kind)| bits)
    }

    /// **Transitional shim.** Push raw f64 bits with `Float64` kind.
    #[inline(always)]
    pub(crate) fn push_raw_f64(&mut self, value: f64) -> Result<(), VMError> {
        let bits = if value.is_nan() {
            f64::NAN.to_bits()
        } else {
            value.to_bits()
        };
        self.push_kinded(bits, NativeKind::Float64)
    }

    /// **Transitional shim.** Pop raw f64 bits.
    #[inline(always)]
    pub(crate) fn pop_raw_f64(&mut self) -> Result<f64, VMError> {
        self.pop_kinded().map(|(bits, _k)| f64::from_bits(bits))
    }

    /// **Transitional shim.** Push raw native i64 bits with `Int64` kind.
    #[inline(always)]
    pub fn push_native_i64(&mut self, value: i64) -> Result<(), VMError> {
        self.push_kinded(value as u64, NativeKind::Int64)
    }

    /// **Transitional shim.** Pop raw native i64 bits.
    #[inline(always)]
    pub fn pop_native_i64(&mut self) -> Result<i64, VMError> {
        self.pop_kinded().map(|(bits, _k)| bits as i64)
    }

    /// **Transitional shim.** Push raw native bool bits with `Bool` kind.
    #[inline(always)]
    pub fn push_native_bool(&mut self, value: bool) -> Result<(), VMError> {
        self.push_kinded(value as u64, NativeKind::Bool)
    }

    /// **Transitional shim.** Pop raw native bool bits.
    #[inline(always)]
    pub fn pop_native_bool(&mut self) -> Result<bool, VMError> {
        self.pop_kinded().map(|(bits, _k)| bits != 0)
    }

    /// **Transitional shim.** Borrow-read raw bits at `idx`.
    #[inline(always)]
    pub(crate) fn stack_read_raw(&self, idx: usize) -> u64 {
        self.stack[idx]
    }

    /// **Transitional shim.** Owning-share read of raw bits at `idx`.
    /// Bumps the per-slot share via `clone_with_kind`.
    #[inline(always)]
    pub(crate) fn stack_read_owned(&self, idx: usize) -> u64 {
        let bits = self.stack[idx];
        let kind = self.kinds[idx];
        clone_with_kind(bits, kind);
        bits
    }

    /// **Transitional shim.** Write raw bits at `idx`, retiring the
    /// previous occupant via `drop_with_kind`. New slot kind defaults
    /// to `Bool` (Drop-safe sentinel).
    #[inline(always)]
    pub(crate) fn stack_write_raw(&mut self, idx: usize, value: u64) {
        let old_bits = self.stack[idx];
        let old_kind = self.kinds[idx];
        drop_with_kind(old_bits, old_kind);
        self.stack[idx] = value;
        self.kinds[idx] = NativeKind::Bool;
    }

    /// **Transitional shim.** Take ownership of slot `idx`, replacing it
    /// with the zero/Bool sentinel.
    #[inline(always)]
    pub(crate) fn stack_take_raw(&mut self, idx: usize) -> u64 {
        let bits = self.stack[idx];
        self.stack[idx] = 0u64;
        self.kinds[idx] = NativeKind::Bool;
        bits
    }

    /// **Transitional shim.** Read-only `&[u64]` view of a stack range.
    #[inline(always)]
    pub(crate) fn stack_slice_raw(&self, range: std::ops::Range<usize>) -> &[u64] {
        &self.stack[range]
    }

    /// **Transitional shim.** Peek the top N raw u64 values without popping.
    /// Returns `slice[0]` = deepest, `slice[count-1]` = TOS.
    #[inline(always)]
    pub(crate) fn peek_args_slice(&self, count: usize) -> Result<&[u64], VMError> {
        if count > self.sp {
            return Err(VMError::StackUnderflow);
        }
        let start = self.sp - count;
        Ok(&self.stack[start..self.sp])
    }

    /// **Transitional shim.** `&[u64]` view of module bindings.
    #[inline(always)]
    pub(crate) fn bindings_slice_raw(&self) -> &[u64] {
        &self.module_bindings
    }

    /// **Transitional shim.** Borrow-read raw bits from module binding.
    #[inline(always)]
    pub(crate) fn binding_read_raw(&self, idx: usize) -> u64 {
        self.module_bindings[idx]
    }

    /// **Transitional shim.** Owning-share read of module binding bits.
    /// Module bindings do not yet have a parallel kind track; treat as
    /// `Bool` (no-op clone). Wave 7 owns the binding kind track.
    #[inline(always)]
    pub(crate) fn binding_read_owned(&self, idx: usize) -> u64 {
        // Bool-default — clone is a no-op. Wave 7 will source the kind
        // from a parallel module-bindings kind track.
        self.module_bindings[idx]
    }

    /// **Transitional shim.** Write raw bits into module binding `idx`.
    /// Pre-Wave-7: drop is a no-op (Bool default).
    #[inline(always)]
    pub(crate) fn binding_write_raw(&mut self, idx: usize, value: u64) {
        self.module_bindings[idx] = value;
    }

    /// **Transitional shim.** Take ownership of module binding `idx`.
    #[inline(always)]
    pub(crate) fn binding_take_raw(&mut self, idx: usize) -> u64 {
        let bits = self.module_bindings[idx];
        self.module_bindings[idx] = 0u64;
        bits
    }

    /// **Transitional shim — DELETED PATH.** Top-of-stack tag inspection.
    /// Pre-Wave-6 typed opcodes used these to dispatch between tagged-
    /// fallback and native fast paths; with Wave 6's parallel kind
    /// track, the kind is known by construction so probing is unnecessary.
    /// These shims always return `false` (the native fast path is
    /// universal post-Wave-6); the typed-arithmetic / typed-comparison
    /// migration owns wiring the call sites to the kinds track.
    #[inline(always)]
    pub(crate) fn stack_top_both_i48(&self) -> bool {
        false
    }
    #[inline(always)]
    pub(crate) fn stack_top_is_i48(&self) -> bool {
        false
    }
    #[inline(always)]
    pub(crate) fn stack_top_both_f64(&self) -> bool {
        false
    }
    #[inline(always)]
    pub(crate) fn stack_top_is_f64(&self) -> bool {
        false
    }
    #[inline(always)]
    pub(crate) fn stack_top_is_bool(&self) -> bool {
        false
    }

    /// **Transitional shim.** Pre-Wave-6 NaN-tagged i64 push. With native
    /// kinds, this is just `push_native_i64` — typed-arithmetic
    /// migration removes the tagged/native distinction.
    #[inline(always)]
    pub(crate) fn push_tagged_i64(&mut self, value: i64) -> Result<(), VMError> {
        self.push_native_i64(value)
    }

    /// **Transitional shim.** Pre-Wave-6 NaN-tagged i64 pop.
    #[inline(always)]
    pub(crate) fn pop_tagged_i64(&mut self) -> Result<i64, VMError> {
        self.pop_native_i64()
    }

    /// **Transitional shim.** Pre-Wave-6 NaN-tagged bool push.
    #[inline(always)]
    pub(crate) fn push_tagged_bool(&mut self, value: bool) -> Result<(), VMError> {
        self.push_native_bool(value)
    }

    /// **Transitional shim.** Pre-Wave-6 NaN-tagged bool pop.
    #[inline(always)]
    pub(crate) fn pop_tagged_bool(&mut self) -> Result<bool, VMError> {
        self.pop_native_bool()
    }

    /// **Transitional shim.** Pre-Wave-6 cold-path push. Now backed by
    /// `push_kinded_slow` with a Bool default kind.
    #[cold]
    #[inline(never)]
    pub fn push_raw_u64_slow(&mut self, bits: u64) -> Result<(), VMError> {
        self.push_kinded_slow(bits, NativeKind::Bool)
    }

    /// **Transitional shim.** Pre-Wave-6 callable that returned a
    /// `ValueWord` (deleted). Returns the raw u64 bits of the topmost
    /// pop instead — callers that needed a `ValueWord` are pre-existing
    /// errors not in Wave 6 territory.
    #[inline(always)]
    pub fn pop(&mut self) -> Result<u64, VMError> {
        self.pop_raw_u64()
    }

    /// `NONE_BITS` constant kept for legacy callers (snapshot.rs, etc.).
    /// Post-deletion of `ValueWord`, this is just `0u64` — the
    /// zero/Bool sentinel.
    pub(crate) const NONE_BITS: u64 = 0u64;

    // ── Pre-existing peek-by-closure helper (compatibility) ───────────────

    /// Read-only inspection of stack[idx] via a closure receiving raw
    /// bits + kind. Replaces the pre-Wave-6 `stack_peek_raw` (which
    /// reconstructed a temporary `ValueWord`). The closure must NOT
    /// retain the bits past its return.
    #[inline(always)]
    pub(crate) fn stack_peek_kinded<F, R>(&self, idx: usize, f: F) -> R
    where
        F: FnOnce(u64, NativeKind) -> R,
    {
        f(self.stack[idx], self.kinds[idx])
    }

    /// **Transitional shim.** Pre-Wave-6 `stack_peek_raw` accepted a
    /// closure receiving `&ValueWord`. Post-deletion, the closure
    /// receives the raw `u64` bits — pre-existing call sites that
    /// invoked `ValueWord` accessors are pre-existing errors not in
    /// Wave 6 territory.
    #[inline(always)]
    pub(crate) fn stack_peek_raw<F, R>(&self, idx: usize, f: F) -> R
    where
        F: FnOnce(u64) -> R,
    {
        f(self.stack[idx])
    }
}

#[cfg(test)]
mod kinded_stack_tests {
    use super::*;
    use crate::executor::VMConfig;

    fn make_vm() -> VirtualMachine {
        VirtualMachine::new(VMConfig::default())
    }

    #[test]
    fn push_pop_int_round_trip() {
        let mut vm = make_vm();
        vm.push_kinded(42u64, NativeKind::Int64).unwrap();
        let (bits, kind) = vm.pop_kinded().unwrap();
        assert_eq!(bits, 42u64);
        assert_eq!(kind, NativeKind::Int64);
    }

    #[test]
    fn push_pop_bool_round_trip() {
        let mut vm = make_vm();
        vm.push_kinded(1u64, NativeKind::Bool).unwrap();
        let (bits, kind) = vm.pop_kinded().unwrap();
        assert_eq!(bits, 1u64);
        assert_eq!(kind, NativeKind::Bool);
    }

    #[test]
    fn pop_underflow() {
        let mut vm = make_vm();
        assert!(vm.pop_kinded().is_err());
    }

    #[test]
    fn parallel_track_invariant_holds() {
        let mut vm = make_vm();
        for i in 0..100i64 {
            vm.push_kinded(i as u64, NativeKind::Int64).unwrap();
        }
        vm.debug_assert_kinds_in_sync();
        for _ in 0..100 {
            let (_b, _k) = vm.pop_kinded().unwrap();
        }
        vm.debug_assert_kinds_in_sync();
    }

    /// ADR-006 §2.7.7 WB2.4: reading owned hands out an independent share.
    #[test]
    fn read_owned_kinded_bumps_refcount() {
        let mut vm = make_vm();
        let arc = Arc::new("hello".to_string());
        let weak = Arc::downgrade(&arc);
        let bits = Arc::into_raw(arc) as u64;
        vm.push_kinded(bits, NativeKind::String).unwrap();
        assert_eq!(weak.strong_count(), 1, "stack owns the only share");
        let kinded = vm.read_owned_kinded(vm.sp - 1);
        assert_eq!(weak.strong_count(), 2, "read_owned bumped refcount");
        // Drop the kinded carrier — refcount → 1 (stack still holds the share).
        drop(kinded);
        assert_eq!(weak.strong_count(), 1, "carrier drop retired its share");
        // Pop the stack slot and retire its share.
        let (b, k) = vm.pop_kinded().unwrap();
        drop_with_kind(b, k);
        assert_eq!(weak.strong_count(), 0, "stack pop + drop retired the last");
    }

    /// ADR-006 §2.7.7 WB2.4: truncate releases every dropped share.
    #[test]
    fn truncate_stack_releases_shares() {
        let mut vm = make_vm();
        let arc = Arc::new("truncate test".to_string());
        let weak = Arc::downgrade(&arc);
        let bits = Arc::into_raw(arc) as u64;
        vm.push_kinded(bits, NativeKind::String).unwrap();
        assert_eq!(weak.strong_count(), 1);
        vm.truncate_stack(0);
        assert_eq!(weak.strong_count(), 0, "truncate dropped the share");
    }

    /// Inline scalars: clone/drop are no-ops on the bits.
    #[test]
    fn inline_scalars_no_refcount_dispatch() {
        // Just confirms that clone/drop on Int64/Bool/Float64 don't crash
        // with arbitrary "pointer-shaped" bits.
        clone_with_kind(0xDEAD_BEEFu64, NativeKind::Int64);
        drop_with_kind(0xDEAD_BEEFu64, NativeKind::Int64);
        clone_with_kind(0u64, NativeKind::Float64);
        drop_with_kind(0u64, NativeKind::Float64);
        clone_with_kind(1u64, NativeKind::Bool);
        drop_with_kind(1u64, NativeKind::Bool);
    }

    /// Zero bits short-circuit refcount dispatch even on heap kinds.
    #[test]
    fn zero_bits_safe_on_heap_kinds() {
        clone_with_kind(0u64, NativeKind::String);
        drop_with_kind(0u64, NativeKind::String);
        clone_with_kind(0u64, NativeKind::Ptr(HeapKind::TypedObject));
        drop_with_kind(0u64, NativeKind::Ptr(HeapKind::TypedObject));
    }
}

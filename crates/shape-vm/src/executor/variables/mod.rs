//! Variable operations for the VM executor
//!
//! Handles: LoadLocal, StoreLocal, LoadModuleBinding, StoreModuleBinding,
//! LoadClosure, StoreClosure, CloseUpvalue, MakeRef, MakeFieldRef,
//! MakeIndexRef, DerefLoad, DerefStore, SetIndexRef, plus the typed
//! Owned/Shared capture and shared-local/module-binding opcodes.
//!
//! ## Wave-δ B6-round-2 close (2026-05-09)
//!
//! Final close for cluster B's `variables/mod.rs`. Migration sources kind
//! through the §2.7.7 stack parallel-`Vec<NativeKind>` track, the §2.7.8
//! cell-storage parallel-kind tracks (`OwnedClosureBlock` /
//! `ClosureLayout::capture_native_kinds` per Wave-γ G-owned-closure-block
//! commit `cb0bf86`, `module_binding_*_kinded` API per Wave-γ
//! G-module-bindings-kind commit `27e2918`, `SharedCell::kind()` per
//! Wave-α B8-shared-cell), and the FrameDescriptor's per-local kind
//! track (`current_frame_descriptor()?.slot(idx)`).
//!
//! The polymorphic legacy paths that dispatched on `tag_bits::is_tagged`
//! / `tag_bits::get_tag` (the deleted ValueWord NaN-box discriminator) and
//! `nanboxed::RefTarget` / `RefProjection` (the deleted ValueWord
//! reference encoding) are surfaced as `NotImplemented(SURFACE)` per
//! playbook §7 REVISED #4. The §2.7.4 / Phase-2c amendment territory
//! includes:
//!
//! - `read_ref_target` / `write_ref_target` / `resolve_ref_value` /
//!   `set_matrix_row_element` / `cow_matrix_write` —
//!   `nanboxed::RefTarget` and `RefProjection` are deleted; the
//!   reference-value carrier needs a kinded redesign (probable shape:
//!   `KindedSlot` carries a `RefTarget`-like enum at the runtime tier).
//! - `op_make_ref` / `op_make_field_ref` / `op_make_index_ref` —
//!   construct `RefTarget` ValueWord-shaped bits; depend on the same
//!   redesign as above.
//! - `op_deref_load` / `op_deref_store` / `op_set_index_ref` — pop a
//!   `RefTarget`-bearing slot, project through `RefProjection::TypedField`
//!   / `Index` / `MatrixRow`; same dependency.
//! - `op_load_owned_mutable_capture` / `op_store_owned_mutable_capture`
//!   polymorphic — Wave D's typed Load/Store opcodes have replaced these;
//!   the polymorphic versions called the deleted `ValueWord::from_raw_bits`
//!   / `vw_clone` / `vw_drop` for the Ptr arm. Surfaced.
//! - `op_load_local_clone` already migrated in Wave-α to
//!   `stack_read_kinded_raw` + `clone_with_kind` — preserved here.
//!
//! No new forbidden-pattern introductions: zero `vw_clone` / `vw_drop` /
//! `tag_bits::*` / `as_heap_ref` / `NativeKind::Unknown` / Bool-default
//! fallbacks. Every kind sourced from a §2.7.7/§2.7.8 parallel track or
//! the FrameDescriptor's per-local kind. SURFACE markers cite the
//! deleted shape by name per CLAUDE.md "describe deleted code by name".

use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::VirtualMachine,
    memory::{record_heap_write, write_barrier_slot},
};
use shape_value::{HeapKind, NativeKind, VMError};

impl VirtualMachine {
    /// Resolve the `ClosureLayout` for the currently-executing closure
    /// frame.
    ///
    /// Returns `None` if the frame is not a closure call or the function
    /// has no registered layout.
    #[inline]
    fn current_closure_layout(
        &self,
    ) -> Option<std::sync::Arc<shape_value::v2::closure_layout::ClosureLayout>> {
        let frame = self.call_stack.last()?;
        let func_id = frame.function_id?;
        self.program
            .closure_function_layouts
            .get(func_id as usize)
            .and_then(|l| l.as_ref())
            .cloned()
    }

    /// Read the raw u64 bits stored behind upvalue `upvalue_idx`.
    /// `frame.upvalues` is the post-`ValueWord`-deletion `Vec<u64>` raw
    /// payload (`executor/mod.rs:200`); for `OwnedMutable` captures the
    /// bits are a `*mut T` cell pointer, for `Shared` captures a
    /// `*const SharedCell` Arc share. Used by every typed and ptr
    /// capture handler below.
    #[inline]
    fn read_capture_raw_pointer_bits(&self, upvalue_idx: u16) -> Result<u64, VMError> {
        let frame = self.call_stack.last().ok_or_else(|| {
            VMError::RuntimeError(
                "mutable/shared capture access outside a call frame".to_string(),
            )
        })?;
        let upvalues = frame.upvalues.as_ref().ok_or_else(|| {
            VMError::RuntimeError(
                "mutable/shared capture access in a frame without upvalues".to_string(),
            )
        })?;
        let bits = upvalues.get(upvalue_idx as usize).copied().ok_or_else(|| {
            VMError::RuntimeError(format!(
                "capture index {} not found in closure",
                upvalue_idx
            ))
        })?;
        Ok(bits)
    }

    #[inline(always)]
    pub(in crate::executor) fn exec_variables(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            LoadLocal => self.op_load_local(instruction)?,
            LoadLocalTrusted => self.op_load_local_trusted(instruction)?,
            LoadLocalMove => self.op_load_local_move(instruction)?,
            LoadLocalClone => self.op_load_local_clone(instruction)?,
            StoreLocal => self.op_store_local(instruction)?,
            StoreLocalTyped => self.op_store_local_typed(instruction)?,
            StoreLocalDrop => self.op_store_local_drop(instruction)?,
            LoadLocalI64 => self.op_load_local_i64(instruction)?,
            LoadLocalU64 => self.op_load_local_u64(instruction)?,
            LoadLocalF64 => self.op_load_local_f64(instruction)?,
            LoadLocalI32 => self.op_load_local_i32(instruction)?,
            LoadLocalU32 => self.op_load_local_u32(instruction)?,
            LoadLocalI16 => self.op_load_local_i16(instruction)?,
            LoadLocalU16 => self.op_load_local_u16(instruction)?,
            LoadLocalI8 => self.op_load_local_i8(instruction)?,
            LoadLocalU8 => self.op_load_local_u8(instruction)?,
            LoadLocalBool => self.op_load_local_bool(instruction)?,
            LoadLocalPtr => self.op_load_local_ptr(instruction)?,
            StoreLocalI64 => self.op_store_local_i64(instruction)?,
            StoreLocalU64 => self.op_store_local_u64(instruction)?,
            StoreLocalF64 => self.op_store_local_f64(instruction)?,
            StoreLocalI32 => self.op_store_local_i32(instruction)?,
            StoreLocalU32 => self.op_store_local_u32(instruction)?,
            StoreLocalI16 => self.op_store_local_i16(instruction)?,
            StoreLocalU16 => self.op_store_local_u16(instruction)?,
            StoreLocalI8 => self.op_store_local_i8(instruction)?,
            StoreLocalU8 => self.op_store_local_u8(instruction)?,
            StoreLocalBool => self.op_store_local_bool(instruction)?,
            StoreLocalPtr => self.op_store_local_ptr(instruction)?,
            LoadModuleBinding => self.op_load_module_binding(instruction)?,
            StoreModuleBinding => self.op_store_module_binding(instruction)?,
            StoreModuleBindingTyped => self.op_store_module_binding_typed(instruction)?,
            LoadModuleBindingI64 => self.op_load_module_binding_i64(instruction)?,
            LoadModuleBindingU64 => self.op_load_module_binding_u64(instruction)?,
            LoadModuleBindingF64 => self.op_load_module_binding_f64(instruction)?,
            LoadModuleBindingI32 => self.op_load_module_binding_i32(instruction)?,
            LoadModuleBindingU32 => self.op_load_module_binding_u32(instruction)?,
            LoadModuleBindingI16 => self.op_load_module_binding_i16(instruction)?,
            LoadModuleBindingU16 => self.op_load_module_binding_u16(instruction)?,
            LoadModuleBindingI8 => self.op_load_module_binding_i8(instruction)?,
            LoadModuleBindingU8 => self.op_load_module_binding_u8(instruction)?,
            LoadModuleBindingBool => self.op_load_module_binding_bool(instruction)?,
            LoadModuleBindingPtr => self.op_load_module_binding_ptr(instruction)?,
            StoreModuleBindingI64 => self.op_store_module_binding_i64(instruction)?,
            StoreModuleBindingU64 => self.op_store_module_binding_u64(instruction)?,
            StoreModuleBindingF64 => self.op_store_module_binding_f64(instruction)?,
            StoreModuleBindingI32 => self.op_store_module_binding_i32(instruction)?,
            StoreModuleBindingU32 => self.op_store_module_binding_u32(instruction)?,
            StoreModuleBindingI16 => self.op_store_module_binding_i16(instruction)?,
            StoreModuleBindingU16 => self.op_store_module_binding_u16(instruction)?,
            StoreModuleBindingI8 => self.op_store_module_binding_i8(instruction)?,
            StoreModuleBindingU8 => self.op_store_module_binding_u8(instruction)?,
            StoreModuleBindingBool => self.op_store_module_binding_bool(instruction)?,
            StoreModuleBindingPtr => self.op_store_module_binding_ptr(instruction)?,
            LoadClosure => self.op_load_closure(instruction)?,
            StoreClosure => self.op_store_closure(instruction)?,
            CloseUpvalue => self.op_close_upvalue(instruction)?,
            MakeRef => self.op_make_ref(instruction)?,
            MakeFieldRef => self.op_make_field_ref(instruction)?,
            MakeIndexRef => self.op_make_index_ref(instruction)?,
            DerefLoad => self.op_deref_load(instruction)?,
            DerefStore => self.op_deref_store(instruction)?,
            SetIndexRef => self.op_set_index_ref(instruction)?,
            LoadOwnedMutableCapture => self.op_load_owned_mutable_capture(instruction)?,
            StoreOwnedMutableCapture => self.op_store_owned_mutable_capture(instruction)?,
            LoadOwnedMutableCaptureI64 => self.op_load_owned_mutable_capture_i64(instruction)?,
            LoadOwnedMutableCaptureU64 => self.op_load_owned_mutable_capture_u64(instruction)?,
            LoadOwnedMutableCaptureF64 => self.op_load_owned_mutable_capture_f64(instruction)?,
            LoadOwnedMutableCaptureI32 => self.op_load_owned_mutable_capture_i32(instruction)?,
            LoadOwnedMutableCaptureU32 => self.op_load_owned_mutable_capture_u32(instruction)?,
            LoadOwnedMutableCaptureI16 => self.op_load_owned_mutable_capture_i16(instruction)?,
            LoadOwnedMutableCaptureU16 => self.op_load_owned_mutable_capture_u16(instruction)?,
            LoadOwnedMutableCaptureI8 => self.op_load_owned_mutable_capture_i8(instruction)?,
            LoadOwnedMutableCaptureU8 => self.op_load_owned_mutable_capture_u8(instruction)?,
            LoadOwnedMutableCaptureBool => self.op_load_owned_mutable_capture_bool(instruction)?,
            LoadOwnedMutableCapturePtr => self.op_load_owned_mutable_capture_ptr(instruction)?,
            StoreOwnedMutableCaptureI64 => self.op_store_owned_mutable_capture_i64(instruction)?,
            StoreOwnedMutableCaptureU64 => self.op_store_owned_mutable_capture_u64(instruction)?,
            StoreOwnedMutableCaptureF64 => self.op_store_owned_mutable_capture_f64(instruction)?,
            StoreOwnedMutableCaptureI32 => self.op_store_owned_mutable_capture_i32(instruction)?,
            StoreOwnedMutableCaptureU32 => self.op_store_owned_mutable_capture_u32(instruction)?,
            StoreOwnedMutableCaptureI16 => self.op_store_owned_mutable_capture_i16(instruction)?,
            StoreOwnedMutableCaptureU16 => self.op_store_owned_mutable_capture_u16(instruction)?,
            StoreOwnedMutableCaptureI8 => self.op_store_owned_mutable_capture_i8(instruction)?,
            StoreOwnedMutableCaptureU8 => self.op_store_owned_mutable_capture_u8(instruction)?,
            StoreOwnedMutableCaptureBool => self.op_store_owned_mutable_capture_bool(instruction)?,
            StoreOwnedMutableCapturePtr => self.op_store_owned_mutable_capture_ptr(instruction)?,
            LoadSharedCapture => self.op_load_shared_capture(instruction)?,
            StoreSharedCapture => self.op_store_shared_capture(instruction)?,
            LoadSharedCaptureI64 => self.op_load_shared_capture_i64(instruction)?,
            LoadSharedCaptureU64 => self.op_load_shared_capture_u64(instruction)?,
            LoadSharedCaptureF64 => self.op_load_shared_capture_f64(instruction)?,
            LoadSharedCaptureI32 => self.op_load_shared_capture_i32(instruction)?,
            LoadSharedCaptureU32 => self.op_load_shared_capture_u32(instruction)?,
            LoadSharedCaptureI16 => self.op_load_shared_capture_i16(instruction)?,
            LoadSharedCaptureU16 => self.op_load_shared_capture_u16(instruction)?,
            LoadSharedCaptureI8 => self.op_load_shared_capture_i8(instruction)?,
            LoadSharedCaptureU8 => self.op_load_shared_capture_u8(instruction)?,
            LoadSharedCaptureBool => self.op_load_shared_capture_bool(instruction)?,
            LoadSharedCapturePtr => self.op_load_shared_capture_ptr(instruction)?,
            StoreSharedCaptureI64 => self.op_store_shared_capture_i64(instruction)?,
            StoreSharedCaptureU64 => self.op_store_shared_capture_u64(instruction)?,
            StoreSharedCaptureF64 => self.op_store_shared_capture_f64(instruction)?,
            StoreSharedCaptureI32 => self.op_store_shared_capture_i32(instruction)?,
            StoreSharedCaptureU32 => self.op_store_shared_capture_u32(instruction)?,
            StoreSharedCaptureI16 => self.op_store_shared_capture_i16(instruction)?,
            StoreSharedCaptureU16 => self.op_store_shared_capture_u16(instruction)?,
            StoreSharedCaptureI8 => self.op_store_shared_capture_i8(instruction)?,
            StoreSharedCaptureU8 => self.op_store_shared_capture_u8(instruction)?,
            StoreSharedCaptureBool => self.op_store_shared_capture_bool(instruction)?,
            StoreSharedCapturePtr => self.op_store_shared_capture_ptr(instruction)?,
            AllocSharedLocal => self.op_alloc_shared_local(instruction)?,
            LoadSharedLocal => self.op_load_shared_local(instruction)?,
            StoreSharedLocal => self.op_store_shared_local(instruction)?,
            DropSharedLocal => self.op_drop_shared_local(instruction)?,
            AllocSharedModuleBinding => self.op_alloc_shared_module_binding(instruction)?,
            LoadSharedModuleBinding => self.op_load_shared_module_binding(instruction)?,
            StoreSharedModuleBinding => self.op_store_shared_module_binding(instruction)?,
            _ => unreachable!(
                "exec_variables called with non-variable opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────
    // Closure upvalue Load/Store
    // ─────────────────────────────────────────────────────────────────────

    /// `LoadClosure { upvalue_idx }`: read the raw u64 capture bits and
    /// push them onto the stack.
    ///
    /// SURFACE (ADR-006 §2.7.4 — Phase-2c) — the polymorphic LoadClosure
    /// path does not source a `NativeKind` for the pushed slot. Per
    /// playbook §2 kind-sourcing rules the closure-capture kind comes
    /// from `current_closure_layout()?.capture_native_kind(idx)` (the
    /// Wave-γ G-owned-closure-block API at `closure_layout.rs:971`),
    /// which DOES classify the capture slot — but the polymorphic
    /// `LoadClosure` opcode is dispatched against captures of any
    /// `CaptureKind` (Immutable / OwnedMutable / Shared). For
    /// `Immutable` captures the raw u64 IS the value (kind matches
    /// `capture_native_kind`); for `OwnedMutable` it's a `*mut T` cell
    /// pointer (the inner-typed Load handler must run, the polymorphic
    /// path corrupts width); for `Shared` it's `*const SharedCell`
    /// (same — the SharedCell handler must run).
    ///
    /// The compiler's Wave D / Wave E flip emits the typed
    /// `LoadOwnedMutableCapture<Kind>` / `LoadSharedCapture<Kind>`
    /// opcodes; the polymorphic `LoadClosure` is the legacy dispatch
    /// for `Immutable` captures only. For Immutable captures the kind
    /// IS the layout's `capture_native_kind(upvalue_idx)`. Migrated.
    fn op_load_closure(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(upvalue_idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(upvalue_idx)?;
        // ADR-006 §2.7.8 / Q10: kind comes from the closure layout's
        // per-capture `NativeKind` track (Wave-γ G-owned-closure-block
        // commit `cb0bf86`). For `Immutable` captures this IS the slot's
        // kind; for `OwnedMutable` / `Shared` the typed dispatch runs
        // through `LoadOwnedMutableCapture<Kind>` / `LoadSharedCapture<Kind>`
        // and never reaches this polymorphic shell.
        let kind = self
            .current_closure_layout()
            .map(|l| l.capture_native_kind(upvalue_idx as usize))
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "LoadClosure[{}]: no closure layout registered for the current frame",
                    upvalue_idx
                ))
            })?;
        // WB2.4 retain-on-read: bump the heap refcount for the pushed
        // share. The closure block continues to own its own share.
        crate::executor::vm_impl::stack::clone_with_kind(bits, kind);
        self.push_kinded(bits, kind)
    }

    /// `StoreClosure { upvalue_idx }`: pop a kinded slot and store the
    /// new capture bits into `frame.upvalues[upvalue_idx]`.
    ///
    /// The compiler emits this for `Immutable` captures only when the
    /// closure body upgrades the capture to mutable. For
    /// `OwnedMutable` / `Shared` captures the typed Store opcodes run.
    fn op_store_closure(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(upvalue_idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (new_bits, src_kind) = self.pop_kinded()?;
        let layout = self.current_closure_layout().ok_or_else(|| {
            // Drop the popped share — we own it but cannot install it.
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            VMError::RuntimeError(format!(
                "StoreClosure[{}]: no closure layout registered for the current frame",
                upvalue_idx
            ))
        })?;
        let cell_kind = layout.capture_native_kind(upvalue_idx as usize);
        // Mid-life kind change refused per ADR-006 §2.7.8 — the layout's
        // `capture_native_kind` is set at construction (constant per
        // `ClosureTypeId`) and immutable. A `Store` whose source kind
        // disagrees would silently misclassify the cell on retire.
        if cell_kind != src_kind {
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            return Err(VMError::RuntimeError(format!(
                "StoreClosure[{}]: source kind {:?} does not match capture kind {:?} \
                 (ADR-006 §2.7.8 / Q10 — capture kind is fixed at closure construction)",
                upvalue_idx, src_kind, cell_kind
            )));
        }
        let frame = self.call_stack.last_mut().ok_or_else(|| {
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            VMError::RuntimeError("StoreClosure outside a call frame".into())
        })?;
        let upvalues = frame.upvalues.as_mut().ok_or_else(|| {
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            VMError::RuntimeError("StoreClosure in a frame without upvalues".into())
        })?;
        let slot = upvalues.get_mut(upvalue_idx as usize).ok_or_else(|| {
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            VMError::RuntimeError(format!(
                "Upvalue index {} not found in closure",
                upvalue_idx
            ))
        })?;
        record_heap_write();
        let old_bits = *slot;
        write_barrier_slot(old_bits, new_bits);
        *slot = new_bits;
        // Release the previous capture's heap share (if any) via
        // `drop_with_kind` using the cell's persistent kind.
        crate::executor::vm_impl::stack::drop_with_kind(old_bits, cell_kind);
        Ok(())
    }

    /// `CloseUpvalue`: legacy no-op — closures capture by value through
    /// `OwnedClosureBlock` cell layout; "closing" is implicit on capture.
    fn op_close_upvalue(&mut self, _instruction: &Instruction) -> Result<(), VMError> {
        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────
    // OwnedMutable capture: polymorphic legacy + typed handlers
    // ─────────────────────────────────────────────────────────────────────

    /// `LoadOwnedMutableCapture { idx }` — polymorphic legacy entry.
    ///
    /// SURFACE (ADR-006 §2.7.4 — Phase-2c): the polymorphic body
    /// matched on `layout.capture_inner_kind(idx)` and pushed a
    /// `ValueWord::from_raw_bits(bits)` for the Ptr arm via the deleted
    /// `vw_clone(cell_bits)` retain. Both `ValueWord` and `vw_clone` are
    /// deleted (CLAUDE.md "Forbidden code"). The typed Wave D opcodes
    /// (`LoadOwnedMutableCapture<Kind>`) replace this dispatch
    /// per-FieldKind; the compiler's Wave E flip emits the typed form
    /// for every capture site. This polymorphic shell stays as a SURFACE
    /// marker until it is removed from the bytecode entirely (out of
    /// B6 territory — bytecode-level cleanup).
    fn op_load_owned_mutable_capture(
        &mut self,
        _instruction: &Instruction,
    ) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "LoadOwnedMutableCapture (polymorphic): the deleted ValueWord/vw_clone \
             dispatch path is replaced by per-FieldKind LoadOwnedMutableCapture<Kind> \
             opcodes (Wave D). Polymorphic shell remains as a SURFACE marker per \
             ADR-006 §2.7.4 / Phase-2c — the compiler's Wave E flip emits the typed \
             form; this shell should be removed from the bytecode dispatch in a \
             follow-up cleanup wave (out of B6 territory)."
                .into(),
        ))
    }

    /// `StoreOwnedMutableCapture { idx }` — polymorphic legacy entry.
    ///
    /// SURFACE — same gap as `op_load_owned_mutable_capture`.
    fn op_store_owned_mutable_capture(
        &mut self,
        _instruction: &Instruction,
    ) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "StoreOwnedMutableCapture (polymorphic): paired with the LoadOwnedMutableCapture \
             SURFACE — the deleted ValueWord/vw_drop release path is replaced by typed \
             StoreOwnedMutableCapture<Kind> opcodes (Wave D). Polymorphic shell remains \
             as a SURFACE marker per ADR-006 §2.7.4 / Phase-2c."
                .into(),
        ))
    }

    fn op_load_owned_mutable_capture_i64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut i64;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        // SAFETY: `cell_ptr` was produced by `alloc_owned_mutable_i64` in
        // `op_make_closure`. The interior FieldKind is determined by
        // `layout.capture_inner_kind(idx) == FieldKind::I64`.
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_i64(cell_ptr) };
        self.push_kinded(value as u64, NativeKind::Int64)
    }

    fn op_load_owned_mutable_capture_u64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u64;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_u64(cell_ptr) };
        self.push_kinded(value, NativeKind::UInt64)
    }

    fn op_load_owned_mutable_capture_f64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut f64;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_f64(cell_ptr) };
        self.push_kinded(value.to_bits(), NativeKind::Float64)
    }

    fn op_load_owned_mutable_capture_i32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut i32;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_i32(cell_ptr) };
        // Sign-extend to 8-byte slot.
        self.push_kinded(value as i64 as u64, NativeKind::Int32)
    }

    fn op_load_owned_mutable_capture_u32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u32;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_u32(cell_ptr) };
        self.push_kinded(value as u64, NativeKind::UInt32)
    }

    fn op_load_owned_mutable_capture_i16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut i16;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_i16(cell_ptr) };
        self.push_kinded(value as i64 as u64, NativeKind::Int16)
    }

    fn op_load_owned_mutable_capture_u16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u16;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_u16(cell_ptr) };
        self.push_kinded(value as u64, NativeKind::UInt16)
    }

    fn op_load_owned_mutable_capture_i8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut i8;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_i8(cell_ptr) };
        self.push_kinded(value as i64 as u64, NativeKind::Int8)
    }

    fn op_load_owned_mutable_capture_u8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u8;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_u8(cell_ptr) };
        self.push_kinded(value as u64, NativeKind::UInt8)
    }

    fn op_load_owned_mutable_capture_bool(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut bool;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_owned_mutable_bool(cell_ptr) };
        self.push_kinded(value as u64, NativeKind::Bool)
    }

    /// `LoadOwnedMutableCapturePtr { idx }` — typed Ptr load with kind
    /// threaded via the §2.7.8 OwnedClosureBlock per-capture kind track.
    ///
    /// Wave-γ G-owned-closure-block (commit `cb0bf86`) added
    /// `ClosureLayout::capture_native_kind(i)` and
    /// `OwnedClosureBlock::read_capture_kinded(i)`. The `OwnedMutable`
    /// cell's interior kind is `layout.capture_native_kind(idx)` —
    /// the layout descriptor classifies the inner payload's
    /// `NativeKind` (heap-bearing arms like
    /// `Ptr(HeapKind::TypedArray)`, `String`, etc., or inline scalars
    /// for narrower-kind override paths). The Load reads the bits via
    /// the typed `read_owned_mutable_ptr` helper and pushes with the
    /// layout-resolved kind — WB2.4 retain-on-read bumps the heap
    /// share via `clone_with_kind`.
    fn op_load_owned_mutable_capture_ptr(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u64;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        // ADR-006 §2.7.8 / Q10 (Wave-γ G-owned-closure-block close,
        // commit `cb0bf86`): the layout's `capture_native_kinds[idx]`
        // classifies the cell's interior payload. For Ptr-typed
        // captures the kind is the heap arm
        // (`Ptr(HeapKind::TypedArray)`, etc.) or `String`.
        let layout = self.current_closure_layout().ok_or_else(|| {
            VMError::RuntimeError(
                "LoadOwnedMutableCapturePtr without registered ClosureLayout".to_string(),
            )
        })?;
        let kind = layout.capture_native_kind(idx as usize);
        // SAFETY: `cell_ptr` was produced by `alloc_owned_mutable_ptr`
        // in `op_make_closure`. The cell stores one `u64` cell with
        // `Arc<T>::into_raw` bits per construction-side contract; the
        // layout's `capture_native_kinds[idx]` carries the matching
        // `NativeKind` (Wave-γ G-owned-closure-block lockstep
        // invariant).
        let cell_bits = unsafe { shape_value::v2::closure_raw::read_owned_mutable_ptr(cell_ptr) };
        // WB2.4 retain-on-read: cell continues to own its share; the
        // pushed stack slot needs an independent share.
        crate::executor::vm_impl::stack::clone_with_kind(cell_bits, kind);
        self.push_kinded(cell_bits, kind)
    }

    fn op_store_owned_mutable_capture_i64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = src_bits as i64;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut i64;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_i64(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_u64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (new_value, _src_kind) = self.pop_kinded()?;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u64;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_u64(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_f64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = f64::from_bits(src_bits);
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut f64;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_f64(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_i32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = src_bits as i32;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut i32;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_i32(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_u32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = src_bits as u32;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u32;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_u32(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_i16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = src_bits as i16;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut i16;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_i16(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_u16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = src_bits as u16;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u16;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_u16(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_i8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = src_bits as i8;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut i8;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_i8(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_u8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = src_bits as u8;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u8;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_u8(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_owned_mutable_capture_bool(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = src_bits != 0;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut bool;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_bool(cell_ptr, new_value) };
        Ok(())
    }

    /// `StoreOwnedMutableCapturePtr { idx }` — typed Ptr store with
    /// kind threaded via the §2.7.8 OwnedClosureBlock per-capture kind
    /// track. Releases the prior cell payload's heap share via
    /// `drop_with_kind` using the layout-resolved kind, then installs
    /// the new payload (which retains the share transferred on the
    /// stack pop).
    fn op_store_owned_mutable_capture_ptr(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (new_bits, src_kind) = self.pop_kinded()?;
        let layout = self.current_closure_layout().ok_or_else(|| {
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            VMError::RuntimeError(
                "StoreOwnedMutableCapturePtr without registered ClosureLayout".to_string(),
            )
        })?;
        let cell_kind = layout.capture_native_kind(idx as usize);
        // ADR-006 §2.7.8 mid-life kind change refusal — capture kind is
        // set at construction (constant per `ClosureTypeId`) and must
        // match the source kind of every Store.
        if cell_kind != src_kind {
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            return Err(VMError::RuntimeError(format!(
                "StoreOwnedMutableCapturePtr[{}]: source kind {:?} does not match \
                 capture kind {:?} (ADR-006 §2.7.8 / Q10 — kind fixed at construction)",
                idx, src_kind, cell_kind
            )));
        }
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *mut u64;
        if cell_ptr.is_null() {
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            return Err(VMError::RuntimeError(
                "OwnedMutable capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        // Read the previous payload bits before overwriting so we can
        // release that share via `drop_with_kind` after the write.
        // SAFETY: `cell_ptr` was produced by `alloc_owned_mutable_ptr`;
        // it stores exactly one `u64` cell.
        let prev_bits = unsafe { shape_value::v2::closure_raw::read_owned_mutable_ptr(cell_ptr) };
        unsafe { shape_value::v2::closure_raw::write_owned_mutable_ptr(cell_ptr, new_bits) };
        // Release the previous cell payload's heap share per ADR-006
        // §2.7.8 retain-on-overwrite. The cell now owns `new_bits`
        // (the share transferred from the popped stack slot).
        crate::executor::vm_impl::stack::drop_with_kind(prev_bits, cell_kind);
        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────
    // Shared capture: polymorphic legacy + typed handlers
    // ─────────────────────────────────────────────────────────────────────

    /// `LoadSharedCapture { idx }` — polymorphic legacy entry.
    ///
    /// Migrated to the §2.7.8 `SharedCell::kind()` API (Wave-α
    /// B8-shared-cell) — the cell's kind is set at construction and
    /// read alongside the payload bits for every Load.
    fn op_load_shared_capture(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        // SAFETY: `cell_ptr` was produced by `Arc::into_raw(Arc::new(...))`
        // in `op_make_closure`; the closure block keeps the share alive.
        let cell_ref = unsafe { &*cell_ptr };
        let kind = cell_ref.kind();
        let payload_bits = {
            let guard = cell_ref.lock();
            *guard
        };
        // WB2.4 retain-on-read: cell keeps its share, stack push needs
        // an independent share.
        crate::executor::vm_impl::stack::clone_with_kind(payload_bits, kind);
        self.push_kinded(payload_bits, kind)
    }

    /// `StoreSharedCapture { idx }` — polymorphic legacy entry.
    ///
    /// Migrated to use `SharedCell::kind()` for retain-on-overwrite.
    /// Pops the kinded source, verifies the source kind matches the
    /// cell kind (§2.7.8 mid-life kind-change refusal), takes the
    /// cell's lock to swap bits, releases the prior payload's heap
    /// share via `drop_with_kind` outside the lock.
    fn op_store_shared_capture(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (new_bits, src_kind) = self.pop_kinded()?;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        // SAFETY: same invariants as `op_load_shared_capture`.
        let cell_ref = unsafe { &*cell_ptr };
        let cell_kind = cell_ref.kind();
        if cell_kind != src_kind {
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            return Err(VMError::RuntimeError(format!(
                "StoreSharedCapture[{}]: source kind {:?} does not match cell kind {:?} \
                 (ADR-006 §2.7.8 / Q10 — SharedCell kind is fixed at construction)",
                idx, src_kind, cell_kind
            )));
        }
        record_heap_write();
        let prev_bits = {
            let mut guard = cell_ref.lock();
            let prev = *guard;
            *guard = new_bits;
            prev
        };
        // Release the previous payload's heap share outside the lock.
        crate::executor::vm_impl::stack::drop_with_kind(prev_bits, cell_kind);
        Ok(())
    }

    fn op_load_shared_capture_i64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_shared_i64(cell_ptr) };
        self.push_kinded(value as u64, NativeKind::Int64)
    }

    fn op_load_shared_capture_u64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_shared_u64(cell_ptr) };
        self.push_kinded(value, NativeKind::UInt64)
    }

    fn op_load_shared_capture_f64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_shared_f64(cell_ptr) };
        self.push_kinded(value.to_bits(), NativeKind::Float64)
    }

    fn op_load_shared_capture_i32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_shared_i32(cell_ptr) };
        self.push_kinded(value as i64 as u64, NativeKind::Int32)
    }

    fn op_load_shared_capture_u32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_shared_u32(cell_ptr) };
        self.push_kinded(value as u64, NativeKind::UInt32)
    }

    fn op_load_shared_capture_i16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_shared_i16(cell_ptr) };
        self.push_kinded(value as i64 as u64, NativeKind::Int16)
    }

    fn op_load_shared_capture_u16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_shared_u16(cell_ptr) };
        self.push_kinded(value as u64, NativeKind::UInt16)
    }

    fn op_load_shared_capture_i8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_shared_i8(cell_ptr) };
        self.push_kinded(value as i64 as u64, NativeKind::Int8)
    }

    fn op_load_shared_capture_u8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_shared_u8(cell_ptr) };
        self.push_kinded(value as u64, NativeKind::UInt8)
    }

    fn op_load_shared_capture_bool(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        let value = unsafe { shape_value::v2::closure_raw::read_shared_bool(cell_ptr) };
        self.push_kinded(value as u64, NativeKind::Bool)
    }

    /// `LoadSharedCapturePtr { idx }` — typed Ptr load via
    /// `SharedCell::kind()` (Wave-β B6 round-1, commit `c785174`).
    fn op_load_shared_capture_ptr(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        // SAFETY: same invariants as `op_load_shared_capture`.
        let cell_ref = unsafe { &*cell_ptr };
        let kind = cell_ref.kind();
        let payload_bits = {
            let guard = cell_ref.lock();
            *guard
        };
        crate::executor::vm_impl::stack::clone_with_kind(payload_bits, kind);
        self.push_kinded(payload_bits, kind)
    }

    fn op_store_shared_capture_i64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = src_bits as i64;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_shared_i64(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_u64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (new_value, _src_kind) = self.pop_kinded()?;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_shared_u64(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_f64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = f64::from_bits(src_bits);
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_shared_f64(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_i32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = src_bits as i64 as i32;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_shared_i32(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_u32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = src_bits as u32;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_shared_u32(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_i16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = src_bits as i64 as i16;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_shared_i16(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_u16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = src_bits as u16;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_shared_u16(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_i8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = src_bits as i64 as i8;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_shared_i8(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_u8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = src_bits as u8;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_shared_u8(cell_ptr, new_value) };
        Ok(())
    }

    fn op_store_shared_capture_bool(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let new_value = src_bits != 0;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        record_heap_write();
        unsafe { shape_value::v2::closure_raw::write_shared_bool(cell_ptr, new_value) };
        Ok(())
    }

    /// `StoreSharedCapturePtr { idx }` — typed Ptr store via
    /// `SharedCell::kind()` (Wave-β B6 round-1, commit `c785174`).
    fn op_store_shared_capture_ptr(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (new_bits, src_kind) = self.pop_kinded()?;
        let bits = self.read_capture_raw_pointer_bits(idx)?;
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            return Err(VMError::RuntimeError(
                "Shared capture pointer is null".to_string(),
            ));
        }
        // SAFETY: see `op_load_shared_capture`.
        let cell_ref = unsafe { &*cell_ptr };
        let cell_kind = cell_ref.kind();
        if cell_kind != src_kind {
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            return Err(VMError::RuntimeError(format!(
                "StoreSharedCapturePtr[{}]: source kind {:?} does not match cell kind \
                 {:?} (ADR-006 §2.7.8 / Q10 — SharedCell kind fixed at construction)",
                idx, src_kind, cell_kind
            )));
        }
        record_heap_write();
        let prev_bits = {
            let mut guard = cell_ref.lock();
            let prev = *guard;
            *guard = new_bits;
            prev
        };
        crate::executor::vm_impl::stack::drop_with_kind(prev_bits, cell_kind);
        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────
    // Shared local opcodes (AllocSharedLocal / Load / Store / Drop)
    // ─────────────────────────────────────────────────────────────────────

    fn op_alloc_shared_local(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        // Wave 8 W8-T25 close (ADR-006 §2.7.12 / Q13 amendment,
        // 2026-05-10): with `HeapKind::SharedCell` now in the heap-
        // variants enum and wired through every Q8/Q10 dispatch table
        // (`clone_with_kind` / `drop_with_kind` in `vm_impl/stack.rs`,
        // `KindedSlot::clone` / `KindedSlot::drop` in `kinded_slot.rs`,
        // `SharedCell::drop` in `v2/closure_layout.rs`,
        // `TypedObjectStorage::drop` in `heap_value.rs`), the parallel-
        // kind track has the discriminator required to label
        // `*const SharedCell` cell-pointer bits.
        //
        // Lifecycle (per `bytecode/opcode_defs.rs:1426`):
        //   1. Pop the initial value (raw bits + payload `NativeKind`)
        //      off the kinded stack — the share owned by the popped
        //      slot transfers into the cell's `value` field.
        //   2. Allocate `Arc::new(SharedCell::new(value_bits, value_kind))`.
        //      `SharedCell::new` records the cell's persistent kind
        //      companion per §2.7.8 / Q10 — the lockstep invariant the
        //      Drop matrix relies on.
        //   3. `Arc::into_raw(arc) as u64` produces the cell-pointer
        //      bits; the local slot becomes the unique strong-count
        //      owner.
        //   4. Write the cell-pointer + `NativeKind::Ptr(HeapKind::SharedCell)`
        //      kind into the local slot via `stack_write_kinded`. The
        //      previous occupant (zero/Bool sentinel from frame
        //      pre-init) is released as a no-op.
        //
        // Forbidden shapes refused on sight:
        //   * §2.7.8 #9 Bool-default fallback for the cell's interior
        //     kind — `value_kind` is sourced from the popped stack
        //     slot's parallel-kind track (the same kind the producer
        //     wrote at push time).
        //   * `(decode|tag|kind|dispatch|...) (bridge|probe|helper|hop|
        //     translator|adapter|shim)` defection-attractor framing
        //     for the cell-pointer share — the `Arc<SharedCell>`
        //     retain/release goes through the §2.7.7 / §2.7.8 dispatch
        //     tables directly (the `HeapKind::SharedCell` arm), not
        //     through any "bridge" or "probe".
        use shape_value::v2::closure_layout::SharedCell;
        use std::sync::Arc as StdArc;

        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (value_bits, value_kind) = self.pop_kinded()?;
        // SAFETY: `SharedCell::new(bits, kind)` records the kind
        // companion in lockstep with the value bits per §2.7.8 / Q10.
        // `pop_kinded` transferred the share ownership out of the
        // stack slot into our local; passing it to the cell transfers
        // the share into the cell's `value` field. `SharedCell::Drop`
        // will retire that share via `drop_with_kind(value_bits, value_kind)`
        // when the last `Arc<SharedCell>` share retires.
        let cell = StdArc::new(SharedCell::new(value_bits, value_kind));
        let cell_bits = StdArc::into_raw(cell) as u64;
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            // Reclaim the share we were about to install. Use the same
            // `Arc::from_raw` shape `op_drop_shared_local` uses.
            unsafe {
                drop(StdArc::from_raw(cell_bits as *const SharedCell));
            }
            return Err(VMError::RuntimeError(format!(
                "AllocSharedLocal: slot {} out of bounds (stack len {})",
                idx,
                self.stack.len()
            )));
        }
        // The frame-init sentinel at `slot` is `(NONE_BITS, Bool)`;
        // `stack_write_kinded` releases that no-op then installs the
        // new (cell_bits, SharedCell) pair in lockstep.
        self.stack_write_kinded(
            slot,
            cell_bits,
            NativeKind::Ptr(HeapKind::SharedCell),
        );
        Ok(())
    }

    fn op_load_shared_local(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;

        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            return Err(VMError::RuntimeError(format!(
                "LoadSharedLocal: slot {} out of bounds (stack len {})",
                idx,
                self.stack.len()
            )));
        }
        let bits = self.stack[slot];
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "LoadSharedLocal: Shared local pointer is null (not initialised or already dropped)"
                    .to_string(),
            ));
        }
        // SAFETY: see `op_load_shared_capture`.
        let cell_ref = unsafe { &*cell_ptr };
        let kind = cell_ref.kind();
        let payload_bits = {
            let guard = cell_ref.lock();
            *guard
        };
        crate::executor::vm_impl::stack::clone_with_kind(payload_bits, kind);
        self.push_kinded(payload_bits, kind)
    }

    fn op_store_shared_local(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;

        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (new_bits, src_kind) = self.pop_kinded()?;
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            return Err(VMError::RuntimeError(format!(
                "StoreSharedLocal: slot {} out of bounds (stack len {})",
                idx,
                self.stack.len()
            )));
        }
        let bits = self.stack[slot];
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            return Err(VMError::RuntimeError(
                "StoreSharedLocal: Shared local pointer is null".to_string(),
            ));
        }
        // SAFETY: see `op_load_shared_capture`.
        let cell_ref = unsafe { &*cell_ptr };
        let cell_kind = cell_ref.kind();
        if cell_kind != src_kind {
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            return Err(VMError::RuntimeError(format!(
                "StoreSharedLocal[{}]: source kind {:?} does not match cell kind {:?} \
                 (ADR-006 §2.7.8 / Q10 — SharedCell kind fixed at construction)",
                idx, src_kind, cell_kind
            )));
        }
        record_heap_write();
        let prev_bits = {
            let mut guard = cell_ref.lock();
            let prev = *guard;
            *guard = new_bits;
            prev
        };
        crate::executor::vm_impl::stack::drop_with_kind(prev_bits, cell_kind);
        Ok(())
    }

    fn op_drop_shared_local(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;
        use std::sync::Arc as StdArc;

        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            return Err(VMError::RuntimeError(format!(
                "DropSharedLocal: slot {} out of bounds (stack len {})",
                idx,
                self.stack.len()
            )));
        }
        let bits = self.stack[slot];
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "DropSharedLocal: Shared local pointer is null".to_string(),
            ));
        }
        // Take the slot via `stack_take_kinded` to clear both the bits
        // and kind track in lockstep (zero/Bool sentinel after take).
        let _ = self.stack_take_kinded(slot);
        // Reclaim the Arc strong-count share allocated by
        // `op_alloc_shared_local`. The cell payload's interior share
        // (if any) was already released by the matching
        // `StoreSharedLocal`/`Drop`-time release path inside the Arc's
        // `Drop` glue — `SharedCell`'s `Drop` calls `drop_with_kind`
        // on its inner payload using its persistent kind.
        unsafe {
            drop(StdArc::from_raw(cell_ptr));
        }
        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────
    // Shared module-binding opcodes
    // ─────────────────────────────────────────────────────────────────────

    fn op_alloc_shared_module_binding(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        // Wave 8 W8-T25 close (ADR-006 §2.7.12 / Q13 amendment,
        // 2026-05-10): paired with `op_alloc_shared_local` above. The
        // Wave-γ G-module-bindings-kind `module_binding_write_kinded`
        // API was already in place; only the `HeapKind::SharedCell`
        // amendment was missing.
        //
        // Lifecycle (per `bytecode/opcode_defs.rs:1494`):
        //   1. Pop the initial value (raw bits + payload `NativeKind`)
        //      off the kinded stack.
        //   2. Allocate `Arc::new(SharedCell::new(value_bits, value_kind))`.
        //   3. `Arc::into_raw(arc) as u64` and write into
        //      `module_bindings[idx]` via `module_binding_write_kinded`
        //      with kind `NativeKind::Ptr(HeapKind::SharedCell)`.
        //   4. Register `idx` with `shared_module_bindings` so the
        //      VM-Drop special-case loop reclaims the Arc share via
        //      `Arc::from_raw` (the kind-aware second loop sees the
        //      zero/Bool sentinel left behind by the special-case loop
        //      and is a no-op — see `executor/mod.rs::Drop for VirtualMachine`).
        //
        // Forbidden shapes refused on sight: same as
        // `op_alloc_shared_local` — no Bool-default fallback, no
        // `(decode|tag|...) (bridge|probe|...)` defection-attractor
        // framing.
        use shape_value::v2::closure_layout::SharedCell;
        use std::sync::Arc as StdArc;

        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (value_bits, value_kind) = self.pop_kinded()?;
        // SAFETY: same construction-side contract as
        // `op_alloc_shared_local`. The popped slot's share transfers
        // into the cell's `value` field; `SharedCell::Drop` retires it
        // via `drop_with_kind` at refcount=0.
        let cell = StdArc::new(SharedCell::new(value_bits, value_kind));
        let cell_bits = StdArc::into_raw(cell) as u64;
        let index = idx as usize;
        // `module_binding_write_kinded` grows the parallel tracks if
        // `index` is past the current end (via `module_binding_pad_to_kinded`),
        // releases the previous occupant via `drop_with_kind`, and
        // installs `(cell_bits, NativeKind::Ptr(HeapKind::SharedCell))`
        // in lockstep.
        self.module_binding_write_kinded(
            index,
            cell_bits,
            NativeKind::Ptr(HeapKind::SharedCell),
        );
        // Register the slot so VM-Drop reclaims the Arc<SharedCell>
        // share via `Arc::from_raw`. The kind-aware second loop in
        // `Drop for VirtualMachine` zeroes both bits and kind first,
        // so the parallel-kind dispatch is a no-op for this slot at
        // teardown — the explicit `Arc::from_raw` retire is the sole
        // release path.
        self.shared_module_bindings.insert(index);
        Ok(())
    }

    fn op_load_shared_module_binding(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;

        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let index = idx as usize;
        if index >= self.module_bindings_len() {
            return Err(VMError::RuntimeError(format!(
                "LoadSharedModuleBinding: slot {} out of bounds (module_bindings len {})",
                index,
                self.module_bindings_len()
            )));
        }
        let (bits, _stored_kind) = self.module_binding_read_kinded_raw(index);
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            return Err(VMError::RuntimeError(
                "LoadSharedModuleBinding: Shared module binding pointer is null".to_string(),
            ));
        }
        // SAFETY: see `op_load_shared_capture`.
        let cell_ref = unsafe { &*cell_ptr };
        let kind = cell_ref.kind();
        let payload_bits = {
            let guard = cell_ref.lock();
            *guard
        };
        crate::executor::vm_impl::stack::clone_with_kind(payload_bits, kind);
        self.push_kinded(payload_bits, kind)
    }

    fn op_store_shared_module_binding(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::v2::closure_layout::SharedCell;

        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (new_bits, src_kind) = self.pop_kinded()?;
        let index = idx as usize;
        if index >= self.module_bindings_len() {
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            return Err(VMError::RuntimeError(format!(
                "StoreSharedModuleBinding: slot {} out of bounds (module_bindings len {})",
                index,
                self.module_bindings_len()
            )));
        }
        let (bits, _stored_kind) = self.module_binding_read_kinded_raw(index);
        let cell_ptr = bits as *const SharedCell;
        if cell_ptr.is_null() {
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            return Err(VMError::RuntimeError(
                "StoreSharedModuleBinding: Shared module binding pointer is null".to_string(),
            ));
        }
        let cell_ref = unsafe { &*cell_ptr };
        let cell_kind = cell_ref.kind();
        if cell_kind != src_kind {
            crate::executor::vm_impl::stack::drop_with_kind(new_bits, src_kind);
            return Err(VMError::RuntimeError(format!(
                "StoreSharedModuleBinding[{}]: source kind {:?} does not match cell \
                 kind {:?} (ADR-006 §2.7.8 / Q10 — SharedCell kind fixed at \
                 construction)",
                index, src_kind, cell_kind
            )));
        }
        record_heap_write();
        let prev_bits = {
            let mut guard = cell_ref.lock();
            let prev = *guard;
            *guard = new_bits;
            prev
        };
        crate::executor::vm_impl::stack::drop_with_kind(prev_bits, cell_kind);
        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────
    // LoadLocal / LoadLocalTrusted (polymorphic) — kind from FrameDescriptor
    // ─────────────────────────────────────────────────────────────────────

    /// `LoadLocal { idx }` — kind from `FrameDescriptor.slots[idx]` per
    /// playbook §2 kind-sourcing rules. The slot's bits are read raw
    /// and pushed kinded; `clone_with_kind` bumps the heap share so
    /// the slot stays live.
    pub(in crate::executor) fn op_load_local(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocal slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        // ADR-006 §2.7.7 / playbook §2: source kind from the parallel
        // kind track (the lockstep `kinds[slot]` matches the producing
        // typed Store / typed initial-write). Replaces the deleted
        // `tag_bits::is_tagged` / `get_tag` / `ValueWord::clone_from_bits`
        // legacy dispatch (CLAUDE.md "Forbidden code" #3 — runtime
        // tag_bits dispatch).
        let (bits, kind) = self.stack_read_kinded_raw(slot);
        // WB2.4 retain-on-read: bump the heap refcount so the pushed
        // share is independent of the slot's share.
        crate::executor::vm_impl::stack::clone_with_kind(bits, kind);
        self.push_kinded(bits, kind)
    }

    /// `LoadLocalTrusted { idx }` — same shape as `LoadLocal`. The
    /// "trusted" contract used to mean the compiler skipped runtime
    /// tag-bit validation; post-ADR-006 §2.7.7 every slot has a
    /// concrete kind in the parallel track, so the trusted vs untrusted
    /// distinction is no longer about tag dispatch — both paths read
    /// the slot's lockstep `(bits, kind)` directly.
    #[inline(always)]
    pub(in crate::executor) fn op_load_local_trusted(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalTrusted slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let (bits, kind) = self.stack_read_kinded_raw(slot);
        crate::executor::vm_impl::stack::clone_with_kind(bits, kind);
        self.push_kinded(bits, kind)
    }

    /// `LoadLocalMove { idx }` — transfer ownership: zero out the slot
    /// (kind goes to the no-op Bool sentinel) and push the bits +
    /// original kind onto the stack with no refcount change.
    fn op_load_local_move(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalMove slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let (bits, kind) = self.stack_take_kinded(slot);
        self.push_kinded(bits, kind)
    }

    /// `LoadLocalClone { idx }` — clone semantics: bump the heap share
    /// via `clone_with_kind`, push the share onto the stack; slot
    /// stays live.
    fn op_load_local_clone(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalClone slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let (bits, kind) = self.stack_read_kinded_raw(slot);
        crate::executor::vm_impl::stack::clone_with_kind(bits, kind);
        self.push_kinded(bits, kind)
    }

    /// `StoreLocalDrop { idx }` — pop a kinded source, install into
    /// the slot via `stack_write_kinded` (which releases the prior
    /// occupant's share via `drop_with_kind` using the slot's prior
    /// kind track entry — the canonical retain-on-overwrite path).
    fn op_store_local_drop(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            self.kinds.resize(slot + 1, NativeKind::Bool);
        }
        let (new_bits, new_kind) = self.pop_kinded()?;
        record_heap_write();
        write_barrier_slot(self.stack[slot], new_bits);
        self.stack_write_kinded(slot, new_bits, new_kind);
        Ok(())
    }

    /// `StoreLocal { idx }` — pop a kinded source and install into the
    /// slot. The §2.7.7 stack parallel-kind track and the slot's
    /// existing kind handle the retain-on-overwrite via
    /// `stack_write_kinded`.
    pub(in crate::executor) fn op_store_local(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            self.kinds.resize(slot + 1, NativeKind::Bool);
        }
        let (new_bits, new_kind) = self.pop_kinded()?;
        record_heap_write();
        write_barrier_slot(self.stack[slot], new_bits);
        self.stack_write_kinded(slot, new_bits, new_kind);
        Ok(())
    }

    /// `StoreLocalTyped { idx, width }` — pop a kinded numeric source
    /// and width-truncate (sub-i64 integer kinds) before storing the
    /// raw native bits into the slot.
    fn op_store_local_typed(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::TypedLocal(idx, width)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            self.kinds.resize(slot + 1, NativeKind::Bool);
        }
        let (src_bits, src_kind) = self.pop_kinded()?;
        let truncated_bits: u64 = if let Some(int_w) = width.to_int_width() {
            int_w.truncate(src_bits as i64) as u64
        } else {
            src_bits
        };
        record_heap_write();
        write_barrier_slot(self.stack[slot], truncated_bits);
        self.stack_write_kinded(slot, truncated_bits, src_kind);
        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────
    // Typed local Load/Store (per-Kind handlers)
    // ─────────────────────────────────────────────────────────────────────

    fn op_load_local_i64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalI64 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        self.push_kinded(bits, NativeKind::Int64)
    }

    fn op_load_local_u64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalU64 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        self.push_kinded(bits, NativeKind::UInt64)
    }

    fn op_load_local_f64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalF64 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        self.push_kinded(bits, NativeKind::Float64)
    }

    fn op_load_local_i32(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalI32 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        let value = bits as i32;
        self.push_kinded(value as i64 as u64, NativeKind::Int32)
    }

    fn op_load_local_u32(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalU32 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        let value = bits as u32;
        self.push_kinded(value as u64, NativeKind::UInt32)
    }

    fn op_load_local_i16(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalI16 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        let value = bits as i16;
        self.push_kinded(value as i64 as u64, NativeKind::Int16)
    }

    fn op_load_local_u16(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalU16 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        let value = bits as u16;
        self.push_kinded(value as u64, NativeKind::UInt16)
    }

    fn op_load_local_i8(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalI8 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        let value = bits as i8;
        self.push_kinded(value as i64 as u64, NativeKind::Int8)
    }

    fn op_load_local_u8(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalU8 slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        let value = bits as u8;
        self.push_kinded(value as u64, NativeKind::UInt8)
    }

    fn op_load_local_bool(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalBool slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let bits = unsafe { *(self.stack.as_ptr().add(slot) as *const u64) };
        self.push_kinded(bits, NativeKind::Bool)
    }

    fn op_load_local_ptr(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "LoadLocalPtr slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        // ADR-006 §2.7.7 / playbook §2: kind comes from the §2.7.7 stack
        // parallel-kind track; the slot's lockstep entry classifies the
        // Ptr variant (e.g. `Ptr(HeapKind::TypedArray)`, `String`).
        let (bits, kind) = self.stack_read_kinded_raw(slot);
        self.push_kinded(bits, kind)
    }

    fn op_store_local_i64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            self.kinds.resize(slot + 1, NativeKind::Bool);
        }
        let (src_bits, _src_kind) = self.pop_kinded()?;
        record_heap_write();
        write_barrier_slot(self.stack[slot], src_bits);
        self.stack_write_kinded(slot, src_bits, NativeKind::Int64);
        Ok(())
    }

    fn op_store_local_u64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            self.kinds.resize(slot + 1, NativeKind::Bool);
        }
        let (src_bits, _src_kind) = self.pop_kinded()?;
        record_heap_write();
        write_barrier_slot(self.stack[slot], src_bits);
        self.stack_write_kinded(slot, src_bits, NativeKind::UInt64);
        Ok(())
    }

    fn op_store_local_f64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            self.kinds.resize(slot + 1, NativeKind::Bool);
        }
        let (src_bits, _src_kind) = self.pop_kinded()?;
        record_heap_write();
        write_barrier_slot(self.stack[slot], src_bits);
        self.stack_write_kinded(slot, src_bits, NativeKind::Float64);
        Ok(())
    }

    fn op_store_local_i32(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            self.kinds.resize(slot + 1, NativeKind::Bool);
        }
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let value = src_bits as i32 as i64 as u64;
        record_heap_write();
        write_barrier_slot(self.stack[slot], value);
        self.stack_write_kinded(slot, value, NativeKind::Int32);
        Ok(())
    }

    fn op_store_local_u32(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            self.kinds.resize(slot + 1, NativeKind::Bool);
        }
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let value = src_bits as u32 as u64;
        record_heap_write();
        write_barrier_slot(self.stack[slot], value);
        self.stack_write_kinded(slot, value, NativeKind::UInt32);
        Ok(())
    }

    fn op_store_local_i16(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            self.kinds.resize(slot + 1, NativeKind::Bool);
        }
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let value = src_bits as i16 as i64 as u64;
        record_heap_write();
        write_barrier_slot(self.stack[slot], value);
        self.stack_write_kinded(slot, value, NativeKind::Int16);
        Ok(())
    }

    fn op_store_local_u16(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            self.kinds.resize(slot + 1, NativeKind::Bool);
        }
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let value = src_bits as u16 as u64;
        record_heap_write();
        write_barrier_slot(self.stack[slot], value);
        self.stack_write_kinded(slot, value, NativeKind::UInt16);
        Ok(())
    }

    fn op_store_local_i8(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            self.kinds.resize(slot + 1, NativeKind::Bool);
        }
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let value = src_bits as i8 as i64 as u64;
        record_heap_write();
        write_barrier_slot(self.stack[slot], value);
        self.stack_write_kinded(slot, value, NativeKind::Int8);
        Ok(())
    }

    fn op_store_local_u8(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            self.kinds.resize(slot + 1, NativeKind::Bool);
        }
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let value = src_bits as u8 as u64;
        record_heap_write();
        write_barrier_slot(self.stack[slot], value);
        self.stack_write_kinded(slot, value, NativeKind::UInt8);
        Ok(())
    }

    fn op_store_local_bool(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            self.kinds.resize(slot + 1, NativeKind::Bool);
        }
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let value = (src_bits != 0) as u64;
        record_heap_write();
        write_barrier_slot(self.stack[slot], value);
        self.stack_write_kinded(slot, value, NativeKind::Bool);
        Ok(())
    }

    fn op_store_local_ptr(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        if slot >= self.stack.len() {
            self.stack.resize_with(slot + 1, || Self::NONE_BITS);
            self.kinds.resize(slot + 1, NativeKind::Bool);
        }
        let (src_bits, src_kind) = self.pop_kinded()?;
        record_heap_write();
        write_barrier_slot(self.stack[slot], src_bits);
        // Ptr stores propagate the source's heap kind so the slot's
        // parallel kind track records the matching `Ptr(HeapKind::*)`
        // / `String` arm — `stack_write_kinded` handles
        // retain-on-overwrite of the prior occupant.
        self.stack_write_kinded(slot, src_bits, src_kind);
        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────
    // Module-binding Load/Store (polymorphic + typed)
    // ─────────────────────────────────────────────────────────────────────

    /// `LoadModuleBinding { idx }` — Wave-γ G-module-bindings-kind
    /// (commit `27e2918`) — read kinded bits from the parallel
    /// module-binding kind track and push with `clone_with_kind`
    /// retain-on-read.
    pub(in crate::executor) fn op_load_module_binding(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (bits, kind) = self.module_binding_read_kinded_raw(idx as usize);
        // WB2.4 retain-on-read: the binding slot keeps its share, the
        // pushed slot needs an independent share.
        crate::executor::vm_impl::stack::clone_with_kind(bits, kind);
        self.push_kinded(bits, kind)
    }

    // ────────────────────────────────────────────────────────────────────
    // `MakeRef` / `MakeFieldRef` / `MakeIndexRef` / `DerefLoad` /
    // `DerefStore` / `SetIndexRef` — kinded RefTarget redesign per
    // ADR-006 §2.7.13 / Q14 (Wave 8 W8-T26, 2026-05-10).
    //
    // The deleted carrier (`nanboxed::RefTarget` / `RefProjection`
    // packed into a TAG_REF `ValueWord`) is replaced by typed-`Arc`
    // `RefTarget` payloads emitted to the kinded stack with kind
    // `NativeKind::Ptr(HeapKind::Reference)`. Each `RefTarget` variant
    // carries the `NativeKind` of the **projected slot**, threaded
    // from the producing-opcode emit per §2.7.7 / §2.7.8 / §2.7.10 /
    // §2.7.11 invariant — no tag-bit decoding, no kind fabrication
    // at projection time, no `is_heap()` probe.
    //
    // Slot bits for a Reference-labeled slot are
    // `Arc::into_raw(Arc<RefTarget>) as u64` directly (mirror of the
    // §2.7.9 FilterExpr precedent — `slot.as_heap_value()` is undefined
    // on Reference-labeled bits; recovery is `Arc::from_raw::<RefTarget>`).
    // ────────────────────────────────────────────────────────────────────

    /// `MakeRef { Operand::Local(slot) | Operand::ModuleBinding(idx) }` —
    /// constructs a `RefTarget::Local { frame_index, slot_index, kind }`
    /// or `RefTarget::ModuleBinding { binding_idx, kind }` and pushes
    /// it onto the kinded stack as
    /// `Arc::into_raw(Arc<RefTarget>) as u64` with kind
    /// `NativeKind::Ptr(HeapKind::Reference)`. The kind is sourced from
    /// the §2.7.7 stack parallel-kind track (for locals) or the §2.7.8
    /// module-binding parallel-kind track (for module bindings) — never
    /// fabricated.
    pub(in crate::executor) fn op_make_ref(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::HeapKind;
        let rt = match instruction.operand {
            Some(Operand::Local(local_idx)) => {
                let frame_index = self
                    .call_stack
                    .len()
                    .checked_sub(1)
                    .ok_or_else(|| {
                        VMError::RuntimeError(
                            "MakeRef Local outside any call frame".into(),
                        )
                    })? as u32;
                let bp = self.current_locals_base();
                let slot = bp + local_idx as usize;
                if slot >= self.stack.len() {
                    return Err(VMError::RuntimeError(format!(
                        "MakeRef Local {} out of bounds (stack len {})",
                        local_idx,
                        self.stack.len()
                    )));
                }
                // Kind sourced from the §2.7.7 parallel-kind track at
                // construction time. The producing typed-Store / typed-
                // initial-write emitted this kind; refs capture it
                // verbatim, no fabrication.
                let (_bits, kind) = self.stack_read_kinded_raw(slot);
                shape_value::RefTarget::Local {
                    frame_index,
                    slot_index: local_idx as u32,
                    kind,
                }
            }
            Some(Operand::ModuleBinding(binding_idx)) => {
                // Kind sourced from the §2.7.8 module-binding parallel-
                // kind track at construction time.
                let (_bits, kind) =
                    self.module_binding_read_kinded_raw(binding_idx as usize);
                shape_value::RefTarget::ModuleBinding {
                    binding_idx: binding_idx as u32,
                    kind,
                }
            }
            _ => return Err(VMError::InvalidOperand),
        };
        // Wrap in Arc<RefTarget>, transfer the strong-count share onto
        // the stack via `Arc::into_raw`. Slot bits are the `Arc<RefTarget>`
        // pointer directly per the §2.7.9 FilterExpr precedent — NOT a
        // `Box<HeapValue>` wrap.
        let arc = std::sync::Arc::new(rt);
        let bits = std::sync::Arc::into_raw(arc) as u64;
        self.push_kinded(bits, NativeKind::Ptr(HeapKind::Reference))
    }

    /// `MakeFieldRef { Operand::TypedField{type_id, field_idx,
    /// field_type_tag} }` — pops a base-ref carrier from the stack,
    /// resolves the receiver to an `Arc<TypedObjectStorage>`, and
    /// pushes a projected `RefTarget::TypedField` ref. The projected
    /// slot's kind is sourced from `field_type_tag` via
    /// `field_tag_to_native_kind` (heap arms + inline scalars) — never
    /// fabricated.
    pub(in crate::executor) fn op_make_field_ref(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::HeapKind;
        let Some(Operand::TypedField {
            field_idx,
            field_type_tag,
            ..
        }) = instruction.operand
        else {
            return Err(VMError::InvalidOperand);
        };
        // Source the projected slot's NativeKind from the operand-encoded
        // field_type_tag — playbook §2 kind-sourcing rules. Surface (no
        // fabrication, no Bool-default fallback per §2.7.7 #9) when the
        // tag is FIELD_TAG_ANY / FIELD_TAG_UNKNOWN.
        let projected_kind = crate::executor::typed_object_ops::field_tag_to_native_kind(
            field_type_tag,
        )
        .ok_or_else(|| {
            VMError::NotImplemented(format!(
                "MakeFieldRef SURFACE: field_type_tag {} (FIELD_TAG_ANY / \
                 FIELD_TAG_UNKNOWN) has no statically-sourceable NativeKind \
                 — ADR-006 §2.7.13 / Q14 forbids fabrication. Producing emitter \
                 must stamp a concrete tag.",
                field_type_tag
            ))
        })?;
        // Pop the base-ref carrier. The stack transfers one
        // `Arc<RefTarget>` strong-count share to us via `pop_kinded`.
        let (base_bits, base_kind) = self.pop_kinded()?;
        if base_kind != NativeKind::Ptr(HeapKind::Reference) {
            // Release the popped share even on failure — stack ownership
            // discipline (§2.7.7 / WB2.4).
            crate::executor::vm_impl::stack::drop_with_kind(base_bits, base_kind);
            return Err(VMError::RuntimeError(format!(
                "MakeFieldRef expected Reference receiver, got {:?}",
                base_kind
            )));
        }
        // Resolve the base RefTarget. We hold one strong-count share via
        // `base_bits`; recover the `Arc<RefTarget>` and read it. We must
        // chase the base ref's *receiver* (the underlying TypedObject) —
        // for a Local/ModuleBinding base, read the place's bits to get
        // the TypedObject's `Arc::into_raw` pointer; for a TypedField
        // base (chained projection), recursively resolve the receiver
        // through the parent.
        // SAFETY: kind == Ptr(HeapKind::Reference) is the §2.7.9-style
        // 1:1 dispatch-table invariant — `base_bits` came from
        // `Arc::into_raw::<RefTarget>` at the matching MakeRef /
        // MakeFieldRef / MakeIndexRef site.
        let base_arc: std::sync::Arc<shape_value::RefTarget> =
            unsafe { std::sync::Arc::from_raw(base_bits as *const shape_value::RefTarget) };
        let receiver = match self.resolve_typed_object_receiver(&base_arc) {
            Ok(r) => r,
            Err(e) => {
                // Drop the base ref before bubbling the error (the share
                // we held via base_arc auto-drops here as it goes out of
                // scope).
                drop(base_arc);
                return Err(e);
            }
        };
        // The base ref share retires here as `base_arc` goes out of scope
        // (the popped share was transferred to base_arc; base_arc::drop
        // decrements it).
        drop(base_arc);
        // Bounds check against the receiver's slot count.
        if (field_idx as usize) >= receiver.slots.len() {
            return Err(VMError::RuntimeError(format!(
                "MakeFieldRef field_idx {} out of bounds (slot count {})",
                field_idx,
                receiver.slots.len()
            )));
        }
        let rt = shape_value::RefTarget::TypedField {
            receiver,
            field_offset: field_idx as u32,
            kind: projected_kind,
        };
        let arc = std::sync::Arc::new(rt);
        let bits = std::sync::Arc::into_raw(arc) as u64;
        self.push_kinded(bits, NativeKind::Ptr(HeapKind::Reference))
    }

    /// `MakeIndexRef` — pops [base_ref, index] from the kinded stack
    /// (top is index), resolves the receiver to an `Arc<TypedArrayData>`,
    /// and pushes a `RefTarget::TypedIndex` ref. The element kind is
    /// sourced by matching on the receiver `TypedArrayData` variant —
    /// the producing opcode (`NewTypedArray*`) emitted typed elements,
    /// so the variant identifies the element kind unambiguously.
    pub(in crate::executor) fn op_make_index_ref(
        &mut self,
        _instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::HeapKind;
        // Pop index (top of stack) — must be Int64 per the producing
        // opcode emitting integer index expressions.
        let (idx_bits, idx_kind) = self.pop_kinded()?;
        if idx_kind != NativeKind::Int64 {
            crate::executor::vm_impl::stack::drop_with_kind(idx_bits, idx_kind);
            return Err(VMError::RuntimeError(format!(
                "MakeIndexRef expected Int64 index, got {:?}",
                idx_kind
            )));
        }
        let index = idx_bits as i64;
        if index < 0 {
            return Err(VMError::RuntimeError(format!(
                "MakeIndexRef negative index {}",
                index
            )));
        }
        // Pop the base-ref carrier.
        let (base_bits, base_kind) = self.pop_kinded()?;
        if base_kind != NativeKind::Ptr(HeapKind::Reference) {
            crate::executor::vm_impl::stack::drop_with_kind(base_bits, base_kind);
            return Err(VMError::RuntimeError(format!(
                "MakeIndexRef expected Reference receiver, got {:?}",
                base_kind
            )));
        }
        // SAFETY: kind == Ptr(HeapKind::Reference) — see op_make_field_ref.
        let base_arc: std::sync::Arc<shape_value::RefTarget> =
            unsafe { std::sync::Arc::from_raw(base_bits as *const shape_value::RefTarget) };
        let receiver = match self.resolve_typed_array_receiver(&base_arc) {
            Ok(r) => r,
            Err(e) => {
                drop(base_arc);
                return Err(e);
            }
        };
        drop(base_arc);
        // Source the element NativeKind from the receiver's variant —
        // the producing opcode (`NewTypedArray*`) committed to a typed
        // variant; the variant is the kind-source. No fabrication.
        let elem_kind = typed_array_element_kind(&receiver).ok_or_else(|| {
            VMError::NotImplemented(format!(
                "MakeIndexRef SURFACE: TypedArrayData variant has no \
                 statically-sourceable element NativeKind (e.g. \
                 HeapValue / FloatSlice / Matrix) — ADR-006 §2.7.13 / Q14"
            ))
        })?;
        let rt = shape_value::RefTarget::TypedIndex {
            receiver,
            index: index as u64,
            elem_kind,
        };
        let arc = std::sync::Arc::new(rt);
        let bits = std::sync::Arc::into_raw(arc) as u64;
        self.push_kinded(bits, NativeKind::Ptr(HeapKind::Reference))
    }

    /// `DerefLoad { Operand::Local(idx) }` — reads the ref-bearing local
    /// (without consuming the slot's share), recovers the `RefTarget`,
    /// reads the projected slot's `(bits, kind)`, runs `clone_with_kind`
    /// to bump the underlying heap share, and pushes the value onto the
    /// kinded stack. The local's ref-share stays live (the binding
    /// retains it).
    pub(in crate::executor) fn op_deref_load(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::HeapKind;
        let Some(Operand::Local(local_idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + local_idx as usize;
        if slot >= self.stack.len() {
            return Err(VMError::RuntimeError(format!(
                "DerefLoad slot {} out of bounds (stack len {})",
                local_idx,
                self.stack.len()
            )));
        }
        // Read the ref-bearing local without consuming its share — the
        // local retains the `Arc<RefTarget>` share; we borrow.
        let (ref_bits, ref_kind) = self.stack_read_kinded_raw(slot);
        if ref_kind != NativeKind::Ptr(HeapKind::Reference) {
            return Err(VMError::RuntimeError(format!(
                "DerefLoad expected Reference local, got {:?}",
                ref_kind
            )));
        }
        // SAFETY: kind == Ptr(HeapKind::Reference) — `ref_bits` is an
        // `Arc::into_raw::<RefTarget>` pointer and the slot keeps one
        // share live for us.
        let rt: &shape_value::RefTarget =
            unsafe { &*(ref_bits as *const shape_value::RefTarget) };
        let (out_bits, out_kind) = self.read_ref_target(rt)?;
        // WB2.4 retain-on-read: bump the projected share so the pushed
        // slot's share is independent of the place's share. The place
        // retains its own ownership (the local / module-binding /
        // typed-object-field / typed-array-element keeps its share).
        crate::executor::vm_impl::stack::clone_with_kind(out_bits, out_kind);
        self.push_kinded(out_bits, out_kind)
    }

    /// `DerefStore { Operand::Local(idx) }` — pops the kinded value to
    /// store, reads the ref-bearing local (without consuming its share),
    /// recovers the `RefTarget`, releases the projected place's prior
    /// occupant via `drop_with_kind`, and writes the new bits. The
    /// stored value's share transfers to the place.
    pub(in crate::executor) fn op_deref_store(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::HeapKind;
        let Some(Operand::Local(local_idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        // Pop the value to store FIRST — we own its share. If the ref-
        // shape check fails below, we'll release this share before
        // returning the error.
        let (val_bits, val_kind) = self.pop_kinded()?;
        let bp = self.current_locals_base();
        let slot = bp + local_idx as usize;
        if slot >= self.stack.len() {
            crate::executor::vm_impl::stack::drop_with_kind(val_bits, val_kind);
            return Err(VMError::RuntimeError(format!(
                "DerefStore slot {} out of bounds (stack len {})",
                local_idx,
                self.stack.len()
            )));
        }
        let (ref_bits, ref_kind) = self.stack_read_kinded_raw(slot);
        if ref_kind != NativeKind::Ptr(HeapKind::Reference) {
            crate::executor::vm_impl::stack::drop_with_kind(val_bits, val_kind);
            return Err(VMError::RuntimeError(format!(
                "DerefStore expected Reference local, got {:?}",
                ref_kind
            )));
        }
        // SAFETY: same as DerefLoad.
        let rt: &shape_value::RefTarget =
            unsafe { &*(ref_bits as *const shape_value::RefTarget) };
        // Cross-check: the popped value's kind matches the projected
        // slot's kind (§2.7.5.1 stack-contents-are-post-proof). On a
        // mismatch, surface — never silently fabricate.
        let projected_kind = rt.projected_kind();
        debug_assert_eq!(
            val_kind, projected_kind,
            "DerefStore kind drift: popped {:?}, place {:?} — \
             ADR-006 §2.7.13 invariant violated",
            val_kind, projected_kind
        );
        // The write_ref_target helper takes ownership of val_bits and
        // releases the prior occupant via drop_with_kind. record_heap_write
        // is invoked inside per the GC discipline.
        record_heap_write();
        self.write_ref_target(rt, val_bits, val_kind)
    }

    /// `SetIndexRef { Operand::Local(idx) }` — variant of `DerefStore`
    /// for the `arr[i] = value` shape. Pops [index, value] (top is value),
    /// reads the ref-bearing local, pre-projects through the index to
    /// build a one-shot `TypedIndex`-projection write, and writes the
    /// value into the array element. Conceptually equivalent to
    /// `MakeIndexRef + DerefStore` collapsed into one opcode.
    pub(in crate::executor) fn op_set_index_ref(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use shape_value::HeapKind;
        let Some(Operand::Local(local_idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        // Pop value (top of stack), then index.
        let (val_bits, val_kind) = self.pop_kinded()?;
        let (idx_bits, idx_kind) = self.pop_kinded()?;
        if idx_kind != NativeKind::Int64 {
            crate::executor::vm_impl::stack::drop_with_kind(val_bits, val_kind);
            crate::executor::vm_impl::stack::drop_with_kind(idx_bits, idx_kind);
            return Err(VMError::RuntimeError(format!(
                "SetIndexRef expected Int64 index, got {:?}",
                idx_kind
            )));
        }
        let index = idx_bits as i64;
        if index < 0 {
            crate::executor::vm_impl::stack::drop_with_kind(val_bits, val_kind);
            return Err(VMError::RuntimeError(format!(
                "SetIndexRef negative index {}",
                index
            )));
        }
        let bp = self.current_locals_base();
        let slot = bp + local_idx as usize;
        if slot >= self.stack.len() {
            crate::executor::vm_impl::stack::drop_with_kind(val_bits, val_kind);
            return Err(VMError::RuntimeError(format!(
                "SetIndexRef slot {} out of bounds (stack len {})",
                local_idx,
                self.stack.len()
            )));
        }
        let (ref_bits, ref_kind) = self.stack_read_kinded_raw(slot);
        if ref_kind != NativeKind::Ptr(HeapKind::Reference) {
            crate::executor::vm_impl::stack::drop_with_kind(val_bits, val_kind);
            return Err(VMError::RuntimeError(format!(
                "SetIndexRef expected Reference local, got {:?}",
                ref_kind
            )));
        }
        // SAFETY: same as DerefLoad / DerefStore.
        let rt: &shape_value::RefTarget =
            unsafe { &*(ref_bits as *const shape_value::RefTarget) };
        // Resolve the ref to the receiving TypedArray, then construct a
        // synthetic `TypedIndex` projection for the element write.
        let receiver = self.resolve_typed_array_receiver(rt)?;
        let elem_kind = typed_array_element_kind(&receiver).ok_or_else(|| {
            VMError::NotImplemented(format!(
                "SetIndexRef SURFACE: TypedArrayData variant has no \
                 statically-sourceable element NativeKind — \
                 ADR-006 §2.7.13 / Q14"
            ))
        })?;
        // Cross-check: val_kind matches the array element kind.
        debug_assert_eq!(
            val_kind, elem_kind,
            "SetIndexRef kind drift: popped value {:?}, element {:?} — \
             ADR-006 §2.7.13 invariant violated",
            val_kind, elem_kind
        );
        let synthetic = shape_value::RefTarget::TypedIndex {
            receiver,
            index: index as u64,
            elem_kind,
        };
        record_heap_write();
        self.write_ref_target(&synthetic, val_bits, val_kind)
    }

    // ────────────────────────────────────────────────────────────────────
    // RefTarget resolution + read/write helpers (ADR-006 §2.7.13).
    // ────────────────────────────────────────────────────────────────────

    /// Resolve a `RefTarget` to its underlying `Arc<TypedObjectStorage>`
    /// receiver. For chained projections (TypedField → TypedField), walks
    /// the inner ref. Returns an error if the ref points at a non-
    /// TypedObject place (e.g. an array or scalar local), which is a
    /// construction-side bug.
    fn resolve_typed_object_receiver(
        &self,
        rt: &shape_value::RefTarget,
    ) -> Result<std::sync::Arc<shape_value::heap_value::TypedObjectStorage>, VMError> {
        use shape_value::HeapKind;
        match rt {
            shape_value::RefTarget::Local {
                frame_index,
                slot_index,
                kind,
            } => {
                if *kind != NativeKind::Ptr(HeapKind::TypedObject) {
                    return Err(VMError::RuntimeError(format!(
                        "MakeFieldRef base must reference a TypedObject; got {:?}",
                        kind
                    )));
                }
                let frame =
                    self.call_stack.get(*frame_index as usize).ok_or_else(|| {
                        VMError::RuntimeError(format!(
                            "RefTarget::Local frame_index {} out of bounds",
                            frame_index
                        ))
                    })?;
                let slot = frame.base_pointer + *slot_index as usize;
                let (bits, _) = self.stack_read_kinded_raw(slot);
                // SAFETY: kind == Ptr(HeapKind::TypedObject) means the
                // bits are `Arc::into_raw::<TypedObjectStorage>`. We bump
                // the strong-count to hand the caller an independent
                // share (the local retains its own share).
                unsafe {
                    std::sync::Arc::increment_strong_count(
                        bits as *const shape_value::heap_value::TypedObjectStorage,
                    );
                    Ok(std::sync::Arc::from_raw(
                        bits as *const shape_value::heap_value::TypedObjectStorage,
                    ))
                }
            }
            shape_value::RefTarget::ModuleBinding { binding_idx, kind } => {
                if *kind != NativeKind::Ptr(HeapKind::TypedObject) {
                    return Err(VMError::RuntimeError(format!(
                        "MakeFieldRef base must reference a TypedObject; got {:?}",
                        kind
                    )));
                }
                let (bits, _) =
                    self.module_binding_read_kinded_raw(*binding_idx as usize);
                unsafe {
                    std::sync::Arc::increment_strong_count(
                        bits as *const shape_value::heap_value::TypedObjectStorage,
                    );
                    Ok(std::sync::Arc::from_raw(
                        bits as *const shape_value::heap_value::TypedObjectStorage,
                    ))
                }
            }
            shape_value::RefTarget::TypedField {
                receiver,
                field_offset,
                kind,
            } => {
                if *kind != NativeKind::Ptr(HeapKind::TypedObject) {
                    return Err(VMError::RuntimeError(format!(
                        "Chained MakeFieldRef base must reference a TypedObject; got {:?}",
                        kind
                    )));
                }
                let bits = receiver.slots[*field_offset as usize].raw();
                unsafe {
                    std::sync::Arc::increment_strong_count(
                        bits as *const shape_value::heap_value::TypedObjectStorage,
                    );
                    Ok(std::sync::Arc::from_raw(
                        bits as *const shape_value::heap_value::TypedObjectStorage,
                    ))
                }
            }
            shape_value::RefTarget::TypedIndex { .. } => Err(VMError::RuntimeError(
                "MakeFieldRef chained off a TypedIndex base is unsupported \
                 — array elements aren't TypedObject-typed at the projection \
                 layer (ADR-006 §2.7.13)"
                    .into(),
            )),
        }
    }

    /// Resolve a `RefTarget` to its underlying `Arc<TypedArrayData>`
    /// receiver. Symmetric to `resolve_typed_object_receiver`.
    fn resolve_typed_array_receiver(
        &self,
        rt: &shape_value::RefTarget,
    ) -> Result<std::sync::Arc<shape_value::heap_value::TypedArrayData>, VMError> {
        use shape_value::HeapKind;
        match rt {
            shape_value::RefTarget::Local {
                frame_index,
                slot_index,
                kind,
            } => {
                if *kind != NativeKind::Ptr(HeapKind::TypedArray) {
                    return Err(VMError::RuntimeError(format!(
                        "MakeIndexRef / SetIndexRef base must reference a TypedArray; got {:?}",
                        kind
                    )));
                }
                let frame =
                    self.call_stack.get(*frame_index as usize).ok_or_else(|| {
                        VMError::RuntimeError(format!(
                            "RefTarget::Local frame_index {} out of bounds",
                            frame_index
                        ))
                    })?;
                let slot = frame.base_pointer + *slot_index as usize;
                let (bits, _) = self.stack_read_kinded_raw(slot);
                unsafe {
                    std::sync::Arc::increment_strong_count(
                        bits as *const shape_value::heap_value::TypedArrayData,
                    );
                    Ok(std::sync::Arc::from_raw(
                        bits as *const shape_value::heap_value::TypedArrayData,
                    ))
                }
            }
            shape_value::RefTarget::ModuleBinding { binding_idx, kind } => {
                if *kind != NativeKind::Ptr(HeapKind::TypedArray) {
                    return Err(VMError::RuntimeError(format!(
                        "MakeIndexRef / SetIndexRef base must reference a TypedArray; got {:?}",
                        kind
                    )));
                }
                let (bits, _) =
                    self.module_binding_read_kinded_raw(*binding_idx as usize);
                unsafe {
                    std::sync::Arc::increment_strong_count(
                        bits as *const shape_value::heap_value::TypedArrayData,
                    );
                    Ok(std::sync::Arc::from_raw(
                        bits as *const shape_value::heap_value::TypedArrayData,
                    ))
                }
            }
            shape_value::RefTarget::TypedField {
                receiver,
                field_offset,
                kind,
            } => {
                if *kind != NativeKind::Ptr(HeapKind::TypedArray) {
                    return Err(VMError::RuntimeError(format!(
                        "MakeIndexRef base via TypedField must project a TypedArray; got {:?}",
                        kind
                    )));
                }
                let bits = receiver.slots[*field_offset as usize].raw();
                unsafe {
                    std::sync::Arc::increment_strong_count(
                        bits as *const shape_value::heap_value::TypedArrayData,
                    );
                    Ok(std::sync::Arc::from_raw(
                        bits as *const shape_value::heap_value::TypedArrayData,
                    ))
                }
            }
            shape_value::RefTarget::TypedIndex { .. } => Err(VMError::RuntimeError(
                "MakeIndexRef chained off a TypedIndex base is unsupported \
                 — element-of-element projection is not modelled (ADR-006 \
                 §2.7.13)"
                    .into(),
            )),
        }
    }

    /// Read the projected slot of a `RefTarget` as `(bits, kind)` —
    /// borrows the place's share (the place retains ownership). Caller
    /// is responsible for `clone_with_kind` if pushing onto the stack.
    fn read_ref_target(
        &self,
        rt: &shape_value::RefTarget,
    ) -> Result<(u64, NativeKind), VMError> {
        match rt {
            shape_value::RefTarget::Local {
                frame_index,
                slot_index,
                kind,
            } => {
                let frame =
                    self.call_stack.get(*frame_index as usize).ok_or_else(|| {
                        VMError::RuntimeError(format!(
                            "DerefLoad: RefTarget::Local frame_index {} out of bounds",
                            frame_index
                        ))
                    })?;
                let slot = frame.base_pointer + *slot_index as usize;
                let (bits, _stored_kind) = self.stack_read_kinded_raw(slot);
                Ok((bits, *kind))
            }
            shape_value::RefTarget::ModuleBinding { binding_idx, kind } => {
                let (bits, _stored_kind) =
                    self.module_binding_read_kinded_raw(*binding_idx as usize);
                Ok((bits, *kind))
            }
            shape_value::RefTarget::TypedField {
                receiver,
                field_offset,
                kind,
            } => {
                let bits = receiver.slots[*field_offset as usize].raw();
                Ok((bits, *kind))
            }
            shape_value::RefTarget::TypedIndex {
                receiver,
                index,
                elem_kind,
            } => {
                let bits = typed_array_read_index_raw(receiver, *index as usize)?;
                Ok((bits, *elem_kind))
            }
        }
    }

    /// Write `(val_bits, val_kind)` into the projected slot of a
    /// `RefTarget`, releasing the prior occupant's share via
    /// `drop_with_kind`. Caller transfers ownership of `val_bits` to
    /// the place.
    fn write_ref_target(
        &mut self,
        rt: &shape_value::RefTarget,
        val_bits: u64,
        val_kind: NativeKind,
    ) -> Result<(), VMError> {
        match rt {
            shape_value::RefTarget::Local {
                frame_index,
                slot_index,
                kind,
            } => {
                let frame =
                    self.call_stack.get(*frame_index as usize).ok_or_else(|| {
                        crate::executor::vm_impl::stack::drop_with_kind(
                            val_bits, val_kind,
                        );
                        VMError::RuntimeError(format!(
                            "DerefStore: RefTarget::Local frame_index {} out of bounds",
                            frame_index
                        ))
                    })?;
                let slot = frame.base_pointer + *slot_index as usize;
                // Cross-check: the place's stored kind matches the ref's
                // captured kind (drift = construction-side bug).
                let (prior_bits, prior_kind) = self.stack_read_kinded_raw(slot);
                debug_assert_eq!(
                    prior_kind, *kind,
                    "DerefStore: place kind drift (stored {:?}, ref {:?}) — \
                     ADR-006 §2.7.13",
                    prior_kind, kind
                );
                write_barrier_slot(prior_bits, val_bits);
                self.stack_write_kinded(slot, val_bits, val_kind);
                Ok(())
            }
            shape_value::RefTarget::ModuleBinding { binding_idx, kind } => {
                let (prior_bits, prior_kind) =
                    self.module_binding_read_kinded_raw(*binding_idx as usize);
                debug_assert_eq!(
                    prior_kind, *kind,
                    "DerefStore: module-binding kind drift (stored {:?}, ref {:?}) — \
                     ADR-006 §2.7.13",
                    prior_kind, kind
                );
                write_barrier_slot(prior_bits, val_bits);
                self.module_binding_write_kinded(
                    *binding_idx as usize,
                    val_bits,
                    val_kind,
                );
                Ok(())
            }
            shape_value::RefTarget::TypedField { .. }
            | shape_value::RefTarget::TypedIndex { .. } => {
                // Writing through a TypedField / TypedIndex projection
                // requires mutable access to the receiver's slot buffer.
                // The current `Arc<TypedObjectStorage>` / `Arc<TypedArrayData>`
                // shape is shared-immutable; in-place mutation goes
                // through the same v2-raw-heap aliasing class CLAUDE.md
                // tracks as a separate workstream (see "v2-raw-heap-audit").
                // Surface here per playbook §7 REVISED #4 (no Bool-default,
                // no fabrication) — the kinded RefTarget redesign is
                // landed; the projection-write path lands when the
                // raw-heap-mutation rebuild closes that workstream.
                crate::executor::vm_impl::stack::drop_with_kind(val_bits, val_kind);
                Err(VMError::NotImplemented(
                    "DerefStore / SetIndexRef SURFACE: writing through a \
                     TypedField / TypedIndex projection requires the v2-raw-heap \
                     mutation rebuild — Arc<TypedObjectStorage> / \
                     Arc<TypedArrayData> are shared-immutable carriers \
                     today. ADR-006 §2.7.13 / Q14 lands the kinded ref \
                     carrier; the projection-write rebuild is tracked as \
                     the v2-raw-heap-audit follow-up (CLAUDE.md \"Known \
                     Constraints\")."
                        .into(),
                ))
            }
        }
    }

    /// `StoreModuleBinding { idx }` — Wave-γ G-module-bindings-kind:
    /// pop kinded source and write via `module_binding_write_kinded`,
    /// which releases the prior slot's share via `drop_with_kind`.
    pub(in crate::executor) fn op_store_module_binding(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (new_bits, new_kind) = self.pop_kinded()?;
        record_heap_write();
        // module_binding_write_kinded grows the parallel tracks if
        // necessary and runs drop_with_kind on the prior occupant.
        self.module_binding_write_kinded(idx as usize, new_bits, new_kind);
        Ok(())
    }

    /// `StoreModuleBindingTyped { idx, width }` — pop kinded numeric
    /// source, width-truncate, write via the kinded module-binding
    /// API.
    pub(in crate::executor) fn op_store_module_binding_typed(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::TypedModuleBinding(idx, width)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, src_kind) = self.pop_kinded()?;
        let truncated_bits = if let Some(int_w) = width.to_int_width() {
            int_w.truncate(src_bits as i64) as u64
        } else {
            src_bits
        };
        record_heap_write();
        self.module_binding_write_kinded(idx as usize, truncated_bits, src_kind);
        Ok(())
    }

    fn op_load_module_binding_i64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (bits, _kind) = self.module_binding_read_kinded_raw(idx as usize);
        self.push_kinded(bits, NativeKind::Int64)
    }

    fn op_load_module_binding_u64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (bits, _kind) = self.module_binding_read_kinded_raw(idx as usize);
        self.push_kinded(bits, NativeKind::UInt64)
    }

    fn op_load_module_binding_f64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (bits, _kind) = self.module_binding_read_kinded_raw(idx as usize);
        self.push_kinded(bits, NativeKind::Float64)
    }

    fn op_load_module_binding_i32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (bits, _kind) = self.module_binding_read_kinded_raw(idx as usize);
        let value = bits as i32 as i64 as u64;
        self.push_kinded(value, NativeKind::Int32)
    }

    fn op_load_module_binding_u32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (bits, _kind) = self.module_binding_read_kinded_raw(idx as usize);
        let value = bits as u32 as u64;
        self.push_kinded(value, NativeKind::UInt32)
    }

    fn op_load_module_binding_i16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (bits, _kind) = self.module_binding_read_kinded_raw(idx as usize);
        let value = bits as i16 as i64 as u64;
        self.push_kinded(value, NativeKind::Int16)
    }

    fn op_load_module_binding_u16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (bits, _kind) = self.module_binding_read_kinded_raw(idx as usize);
        let value = bits as u16 as u64;
        self.push_kinded(value, NativeKind::UInt16)
    }

    fn op_load_module_binding_i8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (bits, _kind) = self.module_binding_read_kinded_raw(idx as usize);
        let value = bits as i8 as i64 as u64;
        self.push_kinded(value, NativeKind::Int8)
    }

    fn op_load_module_binding_u8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (bits, _kind) = self.module_binding_read_kinded_raw(idx as usize);
        let value = bits as u8 as u64;
        self.push_kinded(value, NativeKind::UInt8)
    }

    fn op_load_module_binding_bool(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (bits, _kind) = self.module_binding_read_kinded_raw(idx as usize);
        let value = ((bits as u8) != 0) as u64;
        self.push_kinded(value, NativeKind::Bool)
    }

    /// `LoadModuleBindingPtr { idx }` — Wave-γ G-module-bindings-kind:
    /// the parallel kind track now classifies the binding's
    /// heap-bearing arm; read kinded bits + bump the share via
    /// `clone_with_kind`.
    fn op_load_module_binding_ptr(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (bits, kind) = self.module_binding_read_kinded_raw(idx as usize);
        // WB2.4 retain-on-read: the binding slot keeps its share, the
        // pushed slot needs an independent share. The kind track
        // classifies the heap arm — `clone_with_kind` runs the matching
        // `Arc<T>::increment_strong_count` for `Ptr(HeapKind::*)` /
        // `String`, no-op for inline scalars.
        crate::executor::vm_impl::stack::clone_with_kind(bits, kind);
        self.push_kinded(bits, kind)
    }

    fn op_store_module_binding_i64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (value, _src_kind) = self.pop_kinded()?;
        record_heap_write();
        self.module_binding_write_kinded(idx as usize, value, NativeKind::Int64);
        Ok(())
    }

    fn op_store_module_binding_u64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (value, _src_kind) = self.pop_kinded()?;
        record_heap_write();
        self.module_binding_write_kinded(idx as usize, value, NativeKind::UInt64);
        Ok(())
    }

    fn op_store_module_binding_f64(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (value, _src_kind) = self.pop_kinded()?;
        record_heap_write();
        self.module_binding_write_kinded(idx as usize, value, NativeKind::Float64);
        Ok(())
    }

    fn op_store_module_binding_i32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let value = src_bits as i32 as i64 as u64;
        record_heap_write();
        self.module_binding_write_kinded(idx as usize, value, NativeKind::Int32);
        Ok(())
    }

    fn op_store_module_binding_u32(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let value = src_bits as u32 as u64;
        record_heap_write();
        self.module_binding_write_kinded(idx as usize, value, NativeKind::UInt32);
        Ok(())
    }

    fn op_store_module_binding_i16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let value = src_bits as i16 as i64 as u64;
        record_heap_write();
        self.module_binding_write_kinded(idx as usize, value, NativeKind::Int16);
        Ok(())
    }

    fn op_store_module_binding_u16(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let value = src_bits as u16 as u64;
        record_heap_write();
        self.module_binding_write_kinded(idx as usize, value, NativeKind::UInt16);
        Ok(())
    }

    fn op_store_module_binding_i8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let value = src_bits as i8 as i64 as u64;
        record_heap_write();
        self.module_binding_write_kinded(idx as usize, value, NativeKind::Int8);
        Ok(())
    }

    fn op_store_module_binding_u8(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let value = src_bits as u8 as u64;
        record_heap_write();
        self.module_binding_write_kinded(idx as usize, value, NativeKind::UInt8);
        Ok(())
    }

    fn op_store_module_binding_bool(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (src_bits, _src_kind) = self.pop_kinded()?;
        let value = (src_bits != 0) as u64;
        record_heap_write();
        self.module_binding_write_kinded(idx as usize, value, NativeKind::Bool);
        Ok(())
    }

    /// `StoreModuleBindingPtr { idx }` — Wave-γ G-module-bindings-kind:
    /// pop kinded heap source and install via the kinded module-
    /// binding API; the prior occupant's share is released via
    /// `drop_with_kind` using the prior kind track entry.
    fn op_store_module_binding_ptr(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::ModuleBinding(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let (new_bits, src_kind) = self.pop_kinded()?;
        record_heap_write();
        // The source kind classifies the heap arm; module_binding_write_kinded
        // routes prior-occupant release through drop_with_kind with the
        // prior kind track entry.
        self.module_binding_write_kinded(idx as usize, new_bits, src_kind);
        Ok(())
    }

    // ─────────────────────────────────────────────────────────────────────
    // V1.1B ownership-aware local opcodes (MoveLocal / CloneLocal / DropLocal)
    // ─────────────────────────────────────────────────────────────────────

    /// `MoveLocal(idx)` — transfer ownership of the local slot onto
    /// the stack. Reads the slot via `stack_take_kinded` (which clears
    /// the slot to the zero/Bool sentinel without releasing) and
    /// pushes onto the stack.
    pub(in crate::executor) fn op_move_local(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "MoveLocal slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let (bits, kind) = self.stack_take_kinded(slot);
        self.push_kinded(bits, kind)
    }

    /// `CloneLocal(idx)` — clone the local slot's value via
    /// `clone_with_kind`, leaving the slot live with its own share.
    pub(in crate::executor) fn op_clone_local(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "CloneLocal slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let (bits, kind) = self.stack_read_kinded_raw(slot);
        crate::executor::vm_impl::stack::clone_with_kind(bits, kind);
        self.push_kinded(bits, kind)
    }

    /// `DropLocal(idx)` — release the local slot's value via
    /// `drop_with_kind` and zero the slot.
    pub(in crate::executor) fn op_drop_local(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let Some(Operand::Local(idx)) = instruction.operand else {
            return Err(VMError::InvalidOperand);
        };
        let bp = self.current_locals_base();
        let slot = bp + idx as usize;
        debug_assert!(
            slot < self.stack.len(),
            "DropLocal slot {} out of bounds (stack len {})",
            slot,
            self.stack.len()
        );
        let (bits, kind) = self.stack_take_kinded(slot);
        crate::executor::vm_impl::stack::drop_with_kind(bits, kind);
        Ok(())
    }
}

// ────────────────────────────────────────────────────────────────────────
// `RefTarget::TypedIndex` element-kind / read helpers (ADR-006 §2.7.13).
//
// `TypedArrayData` variants are typed-element buffers; the variant is
// the kind-source. `MakeIndexRef` / `SetIndexRef` / `DerefLoad` over a
// `TypedIndex` projection match the variant to recover the element
// `NativeKind` and read the element at `index`.
//
// Variants without a statically-sourceable scalar element kind
// (`HeapValue`, `FloatSlice`, `Matrix`) surface back to the caller via
// `None` — the §2.7.13 invariant forbids fabrication. These variants
// land when the v2-raw-heap mutation rebuild closes (CLAUDE.md
// "v2-raw-heap-audit" follow-up).
// ────────────────────────────────────────────────────────────────────────

/// Element `NativeKind` of a `TypedArrayData` variant, or `None` for
/// variants whose element kind isn't a single inline-scalar kind.
#[inline]
fn typed_array_element_kind(
    arr: &shape_value::heap_value::TypedArrayData,
) -> Option<NativeKind> {
    use shape_value::heap_value::TypedArrayData;
    Some(match arr {
        TypedArrayData::I64(_) => NativeKind::Int64,
        TypedArrayData::F64(_) => NativeKind::Float64,
        TypedArrayData::Bool(_) => NativeKind::Bool,
        TypedArrayData::I8(_) => NativeKind::Int8,
        TypedArrayData::I16(_) => NativeKind::Int16,
        TypedArrayData::I32(_) => NativeKind::Int32,
        TypedArrayData::U8(_) => NativeKind::UInt8,
        TypedArrayData::U16(_) => NativeKind::UInt16,
        TypedArrayData::U32(_) => NativeKind::UInt32,
        TypedArrayData::U64(_) => NativeKind::UInt64,
        TypedArrayData::F32(_) => NativeKind::Float64,
        TypedArrayData::String(_) => NativeKind::String,
        // Variants without a single statically-sourceable scalar
        // element kind. Caller surfaces (no fabrication per §2.7.7 #9).
        TypedArrayData::HeapValue(_)
        | TypedArrayData::FloatSlice { .. }
        | TypedArrayData::Matrix(_) => return None,
    })
}

/// Read the `index`-th element of a `TypedArrayData` as raw bits.
/// Returns an error for out-of-bounds reads or for variants that don't
/// support raw-bits read (HeapValue / FloatSlice / Matrix — the
/// element layout isn't a single u64 slot in those shapes).
#[inline]
fn typed_array_read_index_raw(
    arr: &shape_value::heap_value::TypedArrayData,
    index: usize,
) -> Result<u64, VMError> {
    use shape_value::heap_value::TypedArrayData;
    let bits = match arr {
        TypedArrayData::I64(buf) => *buf
            .data
            .get(index)
            .ok_or_else(|| VMError::IndexOutOfBounds {
                index: index as i32,
                length: buf.data.len(),
            })? as u64,
        TypedArrayData::F64(buf) => buf
            .data
            .get(index)
            .ok_or_else(|| VMError::IndexOutOfBounds {
                index: index as i32,
                length: buf.data.len(),
            })?
            .to_bits(),
        TypedArrayData::Bool(buf) => *buf.data.get(index).ok_or_else(|| {
            VMError::IndexOutOfBounds {
                index: index as i32,
                length: buf.data.len(),
            }
        })? as u64,
        TypedArrayData::I8(buf) => *buf.data.get(index).ok_or_else(|| {
            VMError::IndexOutOfBounds {
                index: index as i32,
                length: buf.data.len(),
            }
        })? as i64 as u64,
        TypedArrayData::I16(buf) => *buf.data.get(index).ok_or_else(|| {
            VMError::IndexOutOfBounds {
                index: index as i32,
                length: buf.data.len(),
            }
        })? as i64 as u64,
        TypedArrayData::I32(buf) => *buf.data.get(index).ok_or_else(|| {
            VMError::IndexOutOfBounds {
                index: index as i32,
                length: buf.data.len(),
            }
        })? as i64 as u64,
        TypedArrayData::U8(buf) => *buf.data.get(index).ok_or_else(|| {
            VMError::IndexOutOfBounds {
                index: index as i32,
                length: buf.data.len(),
            }
        })? as u64,
        TypedArrayData::U16(buf) => *buf.data.get(index).ok_or_else(|| {
            VMError::IndexOutOfBounds {
                index: index as i32,
                length: buf.data.len(),
            }
        })? as u64,
        TypedArrayData::U32(buf) => *buf.data.get(index).ok_or_else(|| {
            VMError::IndexOutOfBounds {
                index: index as i32,
                length: buf.data.len(),
            }
        })? as u64,
        TypedArrayData::U64(buf) => *buf.data.get(index).ok_or_else(|| {
            VMError::IndexOutOfBounds {
                index: index as i32,
                length: buf.data.len(),
            }
        })?,
        TypedArrayData::F32(buf) => (*buf.data.get(index).ok_or_else(|| {
            VMError::IndexOutOfBounds {
                index: index as i32,
                length: buf.data.len(),
            }
        })? as f64)
            .to_bits(),
        TypedArrayData::String(buf) => {
            // Each element is `Arc<String>`; the slot bits are
            // `Arc::into_raw::<String>(arc)`. Bump the strong-count so
            // the read produces an independent share for the caller.
            let s_arc = buf.data.get(index).ok_or_else(|| VMError::IndexOutOfBounds {
                index: index as i32,
                length: buf.data.len(),
            })?;
            let raw = std::sync::Arc::as_ptr(s_arc) as u64;
            // Caller (`read_ref_target`) does not bump — this read path
            // is borrow-only; the buffer keeps its share. The caller
            // (`op_deref_load`) runs `clone_with_kind(bits, String)` to
            // bump the strong-count for the pushed slot.
            raw
        }
        TypedArrayData::HeapValue(_)
        | TypedArrayData::FloatSlice { .. }
        | TypedArrayData::Matrix(_) => {
            return Err(VMError::NotImplemented(
                "DerefLoad through TypedIndex SURFACE: HeapValue / \
                 FloatSlice / Matrix variants don't support raw-bits \
                 element read — ADR-006 §2.7.13 / Q14"
                    .into(),
            ));
        }
    };
    Ok(bits)
}

// ────────────────────────────────────────────────────────────────────────
// Test module — gated until the deleted ValueWord / ValueWordExt ABI
// is replaced (Phase-2c host-API rebuild per ADR-006 §2.7.4).
//
// The pre-deletion tests built `BytecodeProgram`s and asserted on
// `ValueWord` accessors (`as_i64()`, `as_f64()`, `as_bool()`,
// `as_string()`, `from_i64()`, etc.). All of those types are deleted
// per CLAUDE.md "Forbidden code" #1 (the strict-typing bulldozer
// removed `ValueWord` entirely). The replacement test surface uses
// `KindedSlot` (the §2.7 runtime-tier carrier) plus the per-slot
// `NativeKind` parallel track — a Phase-2c test-harness rebuild that
// is out of B6 territory.
//
// The whole module is gated so the file compiles cleanly without
// reintroducing any deleted-ABI imports. The tests themselves are
// preserved in git history at `c785174` (Wave-β B6 round-1) for
// reference when the host-API rebuild lands.
// ────────────────────────────────────────────────────────────────────────
#[cfg(any())]
mod tests {
    // SURFACE (ADR-006 §2.7.4 / Phase-2c): the pre-deletion test body
    // used the deleted `ValueWord` / `ValueWordExt` accessors plus the
    // deleted ValueWord-shape stack shims (the bulldozer removed both
    // alongside `ValueWord` itself per CLAUDE.md Forbidden Patterns
    // §"Forbidden code" #1, and §2.7.7's Forbidden #6 last bullet
    // refused the shim layer once `ValueWord` was gone). Restoring
    // this test module requires the Phase-2c
    // host-API rebuild (the runtime-tier `KindedSlot` carrier needs
    // surface-equivalent assertions for `as_i64()` / `as_f64()` /
    // `as_bool()` / `as_string_arc()` etc.). Tracked as Phase-2c
    // test-harness work; out of B6 territory.
}

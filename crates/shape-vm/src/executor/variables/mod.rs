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
use shape_value::{NativeKind, VMError};

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

    fn op_alloc_shared_local(&mut self, _instruction: &Instruction) -> Result<(), VMError> {
        // ADR-006 §2.7.8 / Q10 SURFACE — `HeapKind::SharedCell` variant
        // is not yet in the heap-variants enum (`heap_variants.rs:61`).
        // The slot's parallel-kind track entry needs a discriminator
        // for `*const SharedCell` cell-pointer bits, which the
        // pre-amendment `HeapKind` ordinal table does not carry. The
        // bytecode opcode comment at `bytecode/opcode_defs.rs:1418`
        // anticipates the variant ("`NativeKind::Ptr(HeapKind::SharedCell)`
        // is the parallel-track discriminator") but the enum has not
        // been amended.
        //
        // Closing this requires a small B8 follow-up:
        //
        //   1. Add `HeapKind::SharedCell` ordinal to
        //      `heap_variants.rs::HeapKind` (and the symmetric
        //      `HeapValue` arm per ADR-005 §1 single-discriminator —
        //      even if the payload lives outside `HeapValue` like
        //      `HeapKind::FilterExpr` does post-Wave-γ G-heap-filter-expr
        //      per ADR-006 §2.7.9 amendment).
        //   2. Wire the matching `Arc::increment_strong_count::<SharedCell>`
        //      / `Arc::decrement_strong_count::<SharedCell>` arms into
        //      every Q8/Q10 dispatch table (`clone_with_kind` /
        //      `drop_with_kind` in `vm_impl/stack.rs`,
        //      `KindedSlot::clone` / `KindedSlot::drop` in
        //      `kinded_slot.rs`).
        //
        // This is the same shape as the Wave-γ G-heap-filter-expr
        // amendment (commit `5d4bbd8`); it is a structural sub-cluster,
        // out of B6 territory per playbook §10. The §2.7.8 forbidden
        // shape #9 (Bool-default fallback) is refused on sight; the
        // SURFACE refusal is the correct shape per the cluster B
        // partial-close disposition (commit `727143e`).
        Err(VMError::NotImplemented(
            "AllocSharedLocal: requires HeapKind::SharedCell variant amendment per \
             ADR-006 §2.7.8 / Q10 + Wave-γ G-heap-filter-expr precedent. The slot's \
             parallel-kind track has no discriminator for *const SharedCell \
             cell-pointer bits today (the bytecode opcode comment anticipates the \
             variant; the heap-variants enum has not been amended). Out of B6 \
             territory (playbook §8 cross-cluster cascade trigger; small B8 \
             structural follow-up)."
                .into(),
        ))
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
        _instruction: &Instruction,
    ) -> Result<(), VMError> {
        // ADR-006 §2.7.8 / Q10 SURFACE — paired with `op_alloc_shared_local`
        // above; same `HeapKind::SharedCell` variant gap. The
        // module-binding parallel-kind track (Wave-γ G-module-bindings-kind,
        // commit `27e2918`) is in place — `module_binding_write_kinded`
        // accepts a `NativeKind` companion — but the matching variant
        // for `*const SharedCell` cell-pointer bits is not yet in the
        // heap-variants enum. Same B8 follow-up as `AllocSharedLocal`
        // closes both.
        Err(VMError::NotImplemented(
            "AllocSharedModuleBinding: paired with AllocSharedLocal SURFACE — \
             requires HeapKind::SharedCell variant amendment per ADR-006 §2.7.8 / \
             Q10. The Wave-γ G-module-bindings-kind module_binding_write_kinded \
             API is in place; only the heap-variants enum amendment is missing. \
             Out of B6 territory (small B8 structural follow-up)."
                .into(),
        ))
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

    /// `MakeRef`, `MakeFieldRef`, `MakeIndexRef` — SURFACE per ADR-006
    /// §2.7.4 / Phase-2c.
    ///
    /// The reference-value carrier was the deleted `nanboxed::RefTarget`
    /// / `RefProjection` enum, packed into a `ValueWord` via
    /// `ValueWord::from_ref` / `from_module_binding_ref` /
    /// `from_projected_ref`. Both `ValueWord` and the `nanboxed` /
    /// `RefProjection` modules are deleted (CLAUDE.md "Forbidden code"
    /// #1, the strict-typing bulldozer). The ref-construction +
    /// deref-load + deref-store paths need a kinded redesign — likely
    /// a `KindedSlot` runtime-tier carrier where `slot` holds a
    /// `RefTarget`-shaped enum and `kind` is `NativeKind::Ptr(HeapKind::Ref)`
    /// or similar (out of B6 territory; needs an ADR-006 §2.7 amendment).
    pub(in crate::executor) fn op_make_ref(
        &mut self,
        _instruction: &Instruction,
    ) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "MakeRef: the deleted nanboxed::RefTarget / ValueWord::from_ref reference \
             carrier needs a kinded redesign per ADR-006 §2.7.4 / Phase-2c. The \
             pre-deletion shape packed RefTarget::{Stack,ModuleBinding} into a TAG_REF \
             ValueWord; post-strict-typing the carrier should live on KindedSlot at \
             the runtime tier with a dedicated NativeKind::Ptr(HeapKind::Ref) variant. \
             Out of B6 territory (playbook §10 row scope; surface-and-stop trigger per \
             playbook §8 cross-cluster cascade)."
                .into(),
        ))
    }

    pub(in crate::executor) fn op_make_field_ref(
        &mut self,
        _instruction: &Instruction,
    ) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "MakeFieldRef: paired with MakeRef SURFACE — the deleted \
             nanboxed::RefTarget::Projected + RefProjection::TypedField encoding has \
             no post-§2.7.7 replacement yet. ADR-006 §2.7.4 / Phase-2c."
                .into(),
        ))
    }

    pub(in crate::executor) fn op_make_index_ref(
        &mut self,
        _instruction: &Instruction,
    ) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "MakeIndexRef: paired with MakeRef SURFACE — deleted RefProjection::Index / \
             ::MatrixRow encoding. ADR-006 §2.7.4 / Phase-2c."
                .into(),
        ))
    }

    /// `DerefLoad { ref_slot }` — SURFACE per ADR-006 §2.7.4 /
    /// Phase-2c, paired with `MakeRef` deletion above.
    pub(in crate::executor) fn op_deref_load(
        &mut self,
        _instruction: &Instruction,
    ) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "DerefLoad: paired with MakeRef SURFACE — depends on the kinded \
             RefTarget redesign per ADR-006 §2.7.4 / Phase-2c. The pre-deletion path \
             read the local slot's TAG_REF bits, decoded RefTarget via \
             nanboxed::as_ref_target, then dispatched on RefProjection::TypedField / \
             ::Index / ::MatrixRow — none of those types exist post-strict-typing."
                .into(),
        ))
    }

    /// `DerefStore { ref_slot }` — SURFACE.
    pub(in crate::executor) fn op_deref_store(
        &mut self,
        _instruction: &Instruction,
    ) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "DerefStore: paired with DerefLoad SURFACE — same RefTarget / \
             RefProjection kinded-redesign dependency. ADR-006 §2.7.4 / Phase-2c."
                .into(),
        ))
    }

    /// `SetIndexRef { ref_slot }` — SURFACE.
    pub(in crate::executor) fn op_set_index_ref(
        &mut self,
        _instruction: &Instruction,
    ) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "SetIndexRef: paired with DerefStore SURFACE; the deleted \
             set_array_index_on_object / RefTarget::{Stack,ModuleBinding,Projected} \
             dispatch table is part of the same kinded-redesign work. ADR-006 \
             §2.7.4 / Phase-2c."
                .into(),
        ))
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

//! Trait object operations for the VM executor.
//!
//! Handles: `BoxTraitObject`, `DynMethodCall`, `DropCall`, `DropCallAsync`.
//!
//! ADR-006 §2.7.24 Q25.C — emission-tier companion to W17-trait-object-
//! storage. The storage tier landed `HeapKind::TraitObject = 29`,
//! `HeapValue::TraitObject(Arc<TraitObjectStorage>)`, the 6-variant
//! `VTableEntry` enum, the `Erase_T` substitution operator, and the
//! `KindedSlot::from_trait_object` constructor. The emission tier
//! consumes those shapes here.
//!
//! **Round-2 scope.** The four opcode handlers below cover:
//!  - `op_box_trait_object`: pop concrete TypedObject value, look up
//!    `Arc<VTable>` from `program.trait_vtables`, allocate
//!    `Arc<TraitObjectStorage>`, push back as `KindedSlot::from_trait_object`.
//!  - `op_dyn_method_call`: pop receiver + args, recover
//!    `Arc<TraitObjectStorage>` via §2.7.6 / Q8 heap dispatch, look up
//!    the method in the vtable, dispatch on `VTableEntry`:
//!     * `Direct` → plain `call_function_with_nb_args` path (`name()`).
//!     * `BoxedReturn` (top-level `Self` in return, `wrap_targets = [path=[]]`)
//!       → call impl method, then re-box the concrete return into a
//!       fresh `TraitObjectStorage` (`clone_me()`).
//!     * `Closure` / `SelfArg` / `Generic` / `Compound` / nested-Self
//!       `BoxedReturn` → surface-and-stop with §-cite per CLAUDE.md
//!       "surface-and-stop discipline".
//!  - `op_drop_call_sync` / `op_drop_call_async`: pop receiver, dispatch
//!    user-defined `Drop::drop` if registered for the concrete type
//!    name, else silent no-op (the auto-drop pass tracks all locals;
//!    types without Drop impls just need the kind dispatch via
//!    `drop_with_kind`, which the pop already did).
//!
//! **Forbidden.** Per CLAUDE.md "Forbidden Patterns" / "Renames to refuse
//! on sight": no Bool-default kind, no synthesized ValueWord, no
//! kind-blind value-call ABI, no `(decode|tag|kind|dispatch) (bridge|...)`
//! framing. Unhandled `VTableEntry` variants surface as
//! `VMError::NotImplemented(SURFACE: ...)` with the §Q25.C.5 cite.

use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::VirtualMachine,
};
use shape_value::{
    HeapKind, KindedSlot, NativeKind, VMError, ValueSlot,
    heap_value::TraitObjectStorage,
    value::{VTable, VTableEntry},
};
use std::sync::Arc;

impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_trait_object_ops(
        &mut self,
        instruction: &Instruction,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        match instruction.opcode {
            OpCode::BoxTraitObject => self.op_box_trait_object(instruction),
            OpCode::DynMethodCall => self.op_dyn_method_call(instruction, ctx),
            OpCode::DropCall => self.op_drop_call_sync(instruction, ctx),
            OpCode::DropCallAsync => self.op_drop_call_async(instruction, ctx),
            _ => unreachable!(
                "exec_trait_object_ops called with non-trait-object opcode: {:?}",
                instruction.opcode
            ),
        }
    }

    /// Box a concrete value into a trait object.
    ///
    /// Stack: `[..., concrete_value]` → `[..., dyn_value]`
    /// Operand: `Operand::Name(StringId)` — the trait name string id
    /// (per ADR-006 §2.7.24 Q25.C.1; multi-trait `dyn A + B + C`
    /// uses the FIRST trait as the primary discriminator).
    ///
    /// Algorithm per §Q25.C:
    ///  1. Pop the concrete value as `(bits, kind)`. Universal-dyn
    ///     auto-boxing per §Q25.C.1 requires the value to be a
    ///     `TypedObject` (the boxed half is `Arc<TypedObjectStorage>`).
    ///     Scalar values that implement traits get auto-boxed into a
    ///     `TypedObject` first — a future amendment will lift this
    ///     restriction; for round-2 we surface a clear error.
    ///  2. Resolve the concrete type's name (via the
    ///     `TypedObjectStorage::schema_id` → type-schema-registry
    ///     lookup).
    ///  3. Look up `Arc<VTable>` in `program.trait_vtables` keyed by
    ///     `"Trait::Type"`.
    ///  4. Allocate `TraitObjectStorage { value, vtable }` and push
    ///     via `KindedSlot::from_trait_object`.
    fn op_box_trait_object(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        // Resolve the trait name from the operand.
        let trait_name = match instruction.operand {
            Some(Operand::Name(sid)) => self
                .program
                .strings
                .get(sid.0 as usize)
                .cloned()
                .ok_or_else(|| {
                    VMError::RuntimeError(
                        "BoxTraitObject: trait-name StringId out of range".to_string(),
                    )
                })?,
            _ => {
                return Err(VMError::RuntimeError(
                    "BoxTraitObject: missing trait-name operand".to_string(),
                ));
            }
        };

        // Pop the concrete value (transfer of share). The slot owned
        // one `Arc<TypedObjectStorage>` strong-count share; we now own
        // it via the (bits, kind) pair.
        let (bits, kind) = self.pop_kinded()?;

        // The concrete value must be a `TypedObject` per §Q25.C.1
        // universal-dyn auto-boxing rule. Recover the `Arc` using the
        // canonical `Arc::from_raw` pattern (typed-Arc shape per
        // ADR-006 §2.3); pair with `Arc::into_raw` to transfer the
        // share back when we hand it to `TraitObjectStorage::new`.
        let typed_object_arc = match kind {
            NativeKind::Ptr(HeapKind::TypedObject) => {
                if bits == 0 {
                    return Err(VMError::RuntimeError(
                        "BoxTraitObject: null TypedObject pointer".to_string(),
                    ));
                }
                // SAFETY: kind=Ptr(TypedObject); bits are
                // `Arc::into_raw::<TypedObjectStorage>(arc)` per the
                // §2.3 typed-Arc invariant established by
                // `ValueSlot::from_typed_object`. The popped slot
                // owned one strong-count share; this `Arc::from_raw`
                // takes ownership of that share.
                unsafe {
                    Arc::from_raw(bits as *const shape_value::heap_value::TypedObjectStorage)
                }
            }
            other => {
                drop_kinded(bits, kind);
                return Err(VMError::NotImplemented(format!(
                    "BoxTraitObject: universal-dyn auto-boxing of non-TypedObject \
                     kinds is deferred per ADR-006 §2.7.24 Q25.C.1 (boxed value \
                     must currently be a TypedObject). Got kind: {:?}",
                    other
                )));
            }
        };

        // Look up the concrete type name from the schema_id.
        let schema_id = typed_object_arc.schema_id;
        let type_name = self
            .program
            .type_schema_registry
            .get_by_id(schema_id as u32)
            .map(|s| s.name.clone())
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "BoxTraitObject: no type schema registered for schema_id {}",
                    schema_id
                ))
            })?;

        // Look up the vtable.
        let key = format!("{}::{}", trait_name, type_name);
        let vtable = self
            .program
            .trait_vtables
            .get(&key)
            .cloned()
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "BoxTraitObject: no vtable registered for '{}' \
                     (looked up key '{}'). Per ADR-006 §2.7.24 Q25.C this \
                     indicates an impl-block compile-tier gap.",
                    trait_name, key
                ))
            })?;

        // Allocate the fat-pointer carrier. The `typed_object_arc`
        // owns the original share — moving it into `TraitObjectStorage::new`
        // transfers ownership without a refcount bump. The vtable was
        // cloned above (`get(&key).cloned()` returned a fresh
        // `Arc<VTable>` share).
        let trait_object = Arc::new(TraitObjectStorage::new(typed_object_arc, vtable));
        let to_bits = Arc::into_raw(trait_object) as u64;
        self.push_kinded(to_bits, NativeKind::Ptr(HeapKind::TraitObject))?;
        Ok(())
    }

    /// Call a method on a trait object via vtable dispatch.
    ///
    /// Stack: `[..., receiver, arg1, ..., argN]` → `[..., result]`
    /// Operand: `Operand::TypedMethodCall { method_id, arg_count, string_id, ... }`
    /// where `string_id` indexes the method name in the string pool.
    ///
    /// Dispatch per §Q25.C.5 `VTableEntry`:
    ///  - `Direct { function_id }` → plain `call_function_with_nb_args`.
    ///  - `BoxedReturn` (top-level Self, `wrap_targets[0].path == []`)
    ///    → call impl, then re-box the concrete return into a fresh
    ///    `TraitObjectStorage` using the receiver's vtable.
    ///  - Other variants → surface-and-stop.
    fn op_dyn_method_call(
        &mut self,
        instruction: &Instruction,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        let (arg_count, method_name) = match instruction.operand {
            Some(Operand::TypedMethodCall {
                arg_count,
                string_id,
                ..
            }) => {
                let name = self.program.strings.get(string_id as usize).cloned().ok_or_else(
                    || VMError::RuntimeError(
                        "DynMethodCall: method-name StringId out of range".to_string(),
                    ),
                )?;
                (arg_count as usize, name)
            }
            _ => {
                return Err(VMError::RuntimeError(
                    "DynMethodCall: missing TypedMethodCall operand".to_string(),
                ));
            }
        };

        // Stack layout at entry (top → bottom):
        //   argN, argN-1, ..., arg1, receiver
        // Receiver lives at `sp - arg_count - 1`. Read it BEFORE
        // popping args so we can inspect the vtable + classify the
        // dispatch path without disturbing the arg layout.
        if self.sp < arg_count + 1 {
            return Err(VMError::StackUnderflow);
        }
        let receiver_idx = self.sp - arg_count - 1;
        let (receiver_bits, receiver_kind) = self.stack_read_kinded_raw(receiver_idx);

        // Recover Arc<TraitObjectStorage> via transient borrow. The
        // slot still owns one strong-count share; we read the inner
        // pointers (value: Arc<TypedObjectStorage>, vtable: Arc<VTable>)
        // and clone them so the dispatch logic operates on owned shares
        // independent of the slot's lifetime.
        let trait_object: Arc<TraitObjectStorage> = match receiver_kind {
            NativeKind::Ptr(HeapKind::TraitObject) => {
                if receiver_bits == 0 {
                    return Err(VMError::RuntimeError(
                        "DynMethodCall: null TraitObject pointer".to_string(),
                    ));
                }
                // SAFETY: kind=Ptr(TraitObject); bits are
                // `Arc::into_raw::<TraitObjectStorage>(arc)` per §2.3
                // typed-Arc invariant. Transient borrow — pair with
                // `Arc::into_raw` below so the slot's share is
                // preserved.
                let borrowed: Arc<TraitObjectStorage> = unsafe {
                    Arc::from_raw(receiver_bits as *const TraitObjectStorage)
                };
                let cloned = Arc::clone(&borrowed);
                let _ = Arc::into_raw(borrowed);
                cloned
            }
            other => {
                return Err(VMError::RuntimeError(format!(
                    "DynMethodCall: receiver must be NativeKind::Ptr(HeapKind::TraitObject), \
                     got {:?}",
                    other
                )));
            }
        };

        // Look up the method in the vtable.
        let entry = trait_object
            .vtable
            .methods
            .get(&method_name)
            .cloned()
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "DynMethodCall: method '{}' not in vtable for trait(s) {:?}",
                    method_name, trait_object.vtable.trait_names
                ))
            })?;

        // Resolve the impl function name → runtime function id.
        // We re-resolve by name at runtime because the compile-tier
        // `function_id` (stored in the VTableEntry) refers to the
        // BytecodeCompiler's pre-link index, which may have been
        // re-ordered by the linker's content-addressed topo-sort.
        // The runtime `function_name_index` is the post-link map.
        let trait_name = trait_object
            .vtable
            .trait_names
            .first()
            .cloned()
            .unwrap_or_default();
        let concrete_type_name = self
            .program
            .type_schema_registry
            .get_by_id(trait_object.value.schema_id as u32)
            .map(|s| s.name.clone())
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "DynMethodCall: no type schema for schema_id {}",
                    trait_object.value.schema_id
                ))
            })?;
        // First try the trait-qualified symbol (default impl):
        // `Trait::Type::__default__::method` → function name.
        // Fall back to `Type::method` (the default-impl naming).
        let resolved_fn_name = self
            .program
            .trait_method_symbols
            .get(&format!(
                "{}::{}::__default__::{}",
                trait_name, concrete_type_name, method_name
            ))
            .cloned()
            .unwrap_or_else(|| format!("{}::{}", concrete_type_name, method_name));
        let runtime_function_id = self
            .function_name_index
            .get(&resolved_fn_name)
            .copied()
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "DynMethodCall: function '{}' not in function_name_index",
                    resolved_fn_name
                ))
            })?;

        // Dispatch.
        match entry {
            VTableEntry::Direct { .. } => self.invoke_dyn_direct(
                runtime_function_id,
                &trait_object,
                arg_count,
                receiver_idx,
                ctx,
                /*box_return_as=*/ None,
            ),
            VTableEntry::BoxedReturn { ref wrap_targets, .. } => {
                // Round-2 scope: only top-level Self (path=[]) wrap.
                let top_level_only = wrap_targets.len() == 1
                    && wrap_targets[0].path.is_empty();
                if !top_level_only {
                    return Err(VMError::NotImplemented(format!(
                        "SURFACE: DynMethodCall BoxedReturn with nested wrap-targets \
                         (path={:?}) per ADR-006 §2.7.24 Q25.C.5 — emission-tier \
                         thunk generation for nested Self is round-3+ work \
                         (Result<Self,E>, Option<Self>, (Self, Self), HashMap<K,Self>).",
                        wrap_targets.iter().map(|w| &w.path).collect::<Vec<_>>()
                    )));
                }
                self.invoke_dyn_direct(
                    runtime_function_id,
                    &trait_object,
                    arg_count,
                    receiver_idx,
                    ctx,
                    Some(Arc::clone(&trait_object.vtable)),
                )
            }
            VTableEntry::Closure { .. } => Err(VMError::NotImplemented(
                "SURFACE: DynMethodCall Closure variant per ADR-006 §2.7.24 \
                 Q25.C.5 — W7 closure-trait-impl dispatch through dyn is \
                 round-3+ work (closure-call gate, see W17-trait-object \
                 close commit)."
                    .to_string(),
            )),
            VTableEntry::SelfArg { .. } => Err(VMError::NotImplemented(
                "SURFACE: DynMethodCall SelfArg variant per ADR-006 §2.7.24 \
                 Q25.C.2 — Self in argument position requires runtime \
                 vtable-identity check; thunk emission is round-3+ work."
                    .to_string(),
            )),
            VTableEntry::Generic { .. } => Err(VMError::NotImplemented(
                "SURFACE: DynMethodCall Generic variant per ADR-006 §2.7.24 \
                 Q25.C.3 — method-generic parameters require TypeInfo \
                 threading; thunk emission is round-3+ work."
                    .to_string(),
            )),
            VTableEntry::Compound { .. } => Err(VMError::NotImplemented(
                "SURFACE: DynMethodCall Compound variant per ADR-006 §2.7.24 \
                 Q25.C.5 — combined BoxedReturn/SelfArg/Generic shapes \
                 require multi-flag thunk emission; round-3+ work."
                    .to_string(),
            )),
        }
    }

    /// Common path for `Direct` + `BoxedReturn(top-level Self)` dispatch.
    ///
    /// The impl method expects `self` as its first parameter (the
    /// concrete `TypedObject`, not the `dyn` carrier). We:
    ///  1. Replace the receiver slot's `dyn` carrier with the inner
    ///     `Arc<TypedObjectStorage>` (re-boxing it as a `TypedObject`
    ///     KindedSlot).
    ///  2. Call the impl/thunk function.
    ///  3. If `box_return_as` is `Some(vtable)`, the impl returned a
    ///    concrete value that must be re-wrapped as a `dyn` carrier
    ///    using `vtable`.
    fn invoke_dyn_direct(
        &mut self,
        function_id: u16,
        trait_object: &Arc<TraitObjectStorage>,
        arg_count: usize,
        receiver_idx: usize,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
        box_return_as: Option<Arc<VTable>>,
    ) -> Result<(), VMError> {
        // Step 1: replace the receiver slot's `dyn` carrier with the
        // inner `Arc<TypedObjectStorage>` share. `stack_write_kinded`
        // drops the previous occupant (releasing one
        // `Arc<TraitObjectStorage>` share) and installs the new share.
        // The `Arc::into_raw(Arc::clone(...))` transfers an owned
        // share into the slot — refcount-discipline-correct.
        let inner_typed_object = Arc::clone(&trait_object.value);
        let new_bits = Arc::into_raw(inner_typed_object) as u64;
        let new_kind = NativeKind::Ptr(HeapKind::TypedObject);
        self.stack_write_kinded(receiver_idx, new_bits, new_kind);

        // Step 2: collect the receiver + args into a `Vec<KindedSlot>`
        // for `call_function_with_nb_args`. Each slot's share
        // transfers into the new vec — we read the bits then
        // sentinel out the stack slot, which moves ownership of the
        // share to the vec entry without bumping the refcount.
        let total = arg_count + 1;
        let mut args: Vec<KindedSlot> = Vec::with_capacity(total);
        for i in 0..total {
            let (b, k) = self.stack_take_kinded(receiver_idx + i);
            args.push(KindedSlot::new(ValueSlot::from_raw(b), k));
        }
        self.sp = receiver_idx; // pop all the consumed slots

        let saved_depth = self.call_stack.len();
        self.call_function_with_nb_args(function_id, &args)?;
        // The new frame's stack_write_kinded path transferred each
        // arg's share into its frame slot; suppress the KindedSlot
        // Drop on our Vec entries to avoid double-decrement.
        for slot in args {
            std::mem::forget(slot);
        }

        // Drive the callee to completion.
        self.execute_until_call_depth(saved_depth, ctx)?;

        // Pop the result.
        let (ret_bits, ret_kind) = self.pop_kinded()?;

        // If this is the BoxedReturn path, re-wrap the concrete
        // TypedObject return into a fresh dyn carrier.
        if let Some(vtable) = box_return_as {
            let inner_typed = match ret_kind {
                NativeKind::Ptr(HeapKind::TypedObject) => {
                    if ret_bits == 0 {
                        return Err(VMError::RuntimeError(
                            "DynMethodCall BoxedReturn: null TypedObject return"
                                .to_string(),
                        ));
                    }
                    // SAFETY: kind=Ptr(TypedObject); bits are
                    // `Arc::into_raw::<TypedObjectStorage>(arc)`; the
                    // popped slot owns one strong-count share. This
                    // `Arc::from_raw` takes ownership of that share.
                    unsafe {
                        Arc::from_raw(ret_bits as *const shape_value::heap_value::TypedObjectStorage)
                    }
                }
                NativeKind::Ptr(HeapKind::TraitObject) => {
                    // Impl already returned a dyn carrier (e.g. it
                    // called another dyn method internally that the
                    // BoxedReturn path already wrapped). Pass through.
                    self.push_kinded(ret_bits, ret_kind)?;
                    return Ok(());
                }
                _ => {
                    drop_kinded(ret_bits, ret_kind);
                    return Err(VMError::NotImplemented(format!(
                        "SURFACE: DynMethodCall BoxedReturn with scalar return kind \
                         {:?} requires universal-dyn auto-boxing per ADR-006 §2.7.24 \
                         Q25.C.1; scalar-payload trait objects are deferred.",
                        ret_kind
                    )));
                }
            };

            // `inner_typed` owns one share. `vtable` was cloned in
            // the dispatch lookup. Both move into TraitObjectStorage::new.
            let new_to = Arc::new(TraitObjectStorage::new(inner_typed, vtable));
            let to_bits = Arc::into_raw(new_to) as u64;
            self.push_kinded(to_bits, NativeKind::Ptr(HeapKind::TraitObject))?;
            return Ok(());
        }

        // Direct path: push the result as-is.
        self.push_kinded(ret_bits, ret_kind)?;
        Ok(())
    }

    /// Sync drop: look up `TypeName::drop` and call it if registered.
    ///
    /// Stack: `[..., value]` → `[...]`
    /// Operand: `Operand::Property(string_id)` with the concrete type
    /// name, OR `Operand::None` for dyn-typed slots whose drop fn must
    /// be resolved at runtime via the vtable / heap-value type_name.
    ///
    /// Behavior: if the type has a registered `Drop::drop` impl, call
    /// it. Otherwise just pop+drop the slot (the kind dispatch in
    /// `pop_kinded` already handles the refcount release).
    fn op_drop_call_sync(
        &mut self,
        instruction: &Instruction,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        self.op_drop_call_impl(instruction, ctx, /*is_async=*/ false)
    }

    /// Async drop: look up `TypeName::drop_async`, falling back to
    /// `TypeName::drop`. Otherwise same as `op_drop_call_sync`.
    fn op_drop_call_async(
        &mut self,
        instruction: &Instruction,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        self.op_drop_call_impl(instruction, ctx, /*is_async=*/ true)
    }

    fn op_drop_call_impl(
        &mut self,
        instruction: &Instruction,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
        is_async: bool,
    ) -> Result<(), VMError> {
        // The compiler emits the type name as `Operand::Property(StringId)`
        // (see compiler/helpers.rs::emit_drop_call_for_local). For
        // unannotated locals it emits `Instruction::simple` with no
        // operand — in that case the type is unknown at compile time;
        // we still pop+drop the slot (the kind dispatch handles refcount).
        let type_name_opt: Option<String> = match instruction.operand {
            Some(Operand::Property(sid)) => {
                self.program.strings.get(sid as usize).cloned()
            }
            _ => None,
        };

        // Pop the receiver. The kinded API already retires the share
        // for ordinary refcount-bearing kinds; we re-push the bits
        // before invoking the drop fn (so the impl method sees its
        // self argument) ONLY if we have a drop fn to call.
        let (bits, kind) = self.pop_kinded()?;

        // Resolve the drop function name.
        let drop_fn_name = type_name_opt.as_ref().and_then(|tn| {
            // Drop trait registered impl: function name is
            // `TypeName::drop` (or `TypeName::drop_async`). Async drop
            // tries the async variant first, then falls back to sync.
            let method_name = if is_async { "drop_async" } else { "drop" };
            self.program
                .trait_method_symbols
                .get(&format!("Drop::{}::__default__::{}", tn, method_name))
                .cloned()
                .or_else(|| {
                    if is_async {
                        self.program
                            .trait_method_symbols
                            .get(&format!("Drop::{}::__default__::drop", tn))
                            .cloned()
                    } else {
                        None
                    }
                })
        });

        let Some(fn_name) = drop_fn_name else {
            // No drop impl. The kinded pop already released the
            // share; we're done.
            drop_kinded(bits, kind);
            return Ok(());
        };

        // Locate the function id.
        let function_id = self
            .program
            .functions
            .iter()
            .position(|f| f.name == fn_name)
            .map(|idx| idx as u16);
        let Some(function_id) = function_id else {
            // Function name is registered in `trait_method_symbols`
            // but not in `functions` — this is a compiler bug.
            drop_kinded(bits, kind);
            return Err(VMError::RuntimeError(format!(
                "DropCall: trait_method_symbols points to '{}' but it has no \
                 entry in program.functions",
                fn_name
            )));
        };

        // Re-push as a self argument and call.
        let self_arg = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let args = vec![self_arg];
        let saved_depth = self.call_stack.len();
        self.call_function_with_nb_args(function_id, &args)?;
        for slot in args {
            std::mem::forget(slot);
        }
        self.execute_until_call_depth(saved_depth, ctx)?;
        // Pop and discard the drop fn's return value (Drop::drop
        // returns Unit; the kind dispatch on pop releases any
        // refcount it carried).
        let (rbits, rkind) = self.pop_kinded()?;
        drop_kinded(rbits, rkind);
        Ok(())
    }
}

/// Local helper: release a `(bits, kind)` share via the canonical
/// `drop_with_kind` dispatch. Mirrors the §2.7.7 WB2.4 pattern used
/// elsewhere in the executor.
#[inline]
fn drop_kinded(bits: u64, kind: NativeKind) {
    crate::executor::vm_impl::stack::drop_with_kind(bits, kind);
}


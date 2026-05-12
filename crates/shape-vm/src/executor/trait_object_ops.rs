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
    heap_value::{OptionData, ResultData, TraitObjectStorage, TypedObjectStorage},
    value::{VTable, VTableEntry, WrapTarget},
};
use smallvec::SmallVec;
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

        // Dispatch per §Q25.C.5 `VTableEntry`. The dispatch path
        // factors into four orthogonal stages:
        //  1. `SelfArg` identity check (§Q25.C.2) — verify each
        //     `Self`-typed argument's vtable matches the receiver's.
        //  2. (`Generic` is a no-op at the bytecode tier — Shape's
        //     generic methods are already type-erased at the impl's
        //     function-id; the `TypeInfo` parameter in the §Q25.C.3
        //     spec is metadata for the thunk to dispatch operations
        //     on `g`, not a separate bytecode argument. Wave 3
        //     deferred amendment if Shape later adopts per-call-site
        //     monomorphization.)
        //  3. Invoke the impl method.
        //  4. `BoxedReturn` re-wrap (§Q25.C.1) — walk every
        //     `WrapTarget::path` through the return value and re-box
        //     each Self-named site into a fresh `TraitObjectStorage`
        //     using the receiver's vtable.
        //
        // `Direct` / `BoxedReturn(top-level)` / `BoxedReturn(nested)`
        // / `SelfArg` / `Generic` / `Compound` all funnel through
        // `invoke_dyn_unified` with the appropriate (wrap_targets,
        // self_arg_positions) descriptors. `Closure` routes through
        // `call_value_immediate_nb` per §2.7.11/Q12.
        match entry {
            VTableEntry::Direct { .. } => self.invoke_dyn_unified(
                runtime_function_id,
                &trait_object,
                arg_count,
                receiver_idx,
                ctx,
                /*wrap_targets=*/ &[],
                /*self_arg_positions=*/ &[],
            ),
            VTableEntry::BoxedReturn { ref wrap_targets, .. } => self.invoke_dyn_unified(
                runtime_function_id,
                &trait_object,
                arg_count,
                receiver_idx,
                ctx,
                wrap_targets.as_slice(),
                &[],
            ),
            VTableEntry::SelfArg {
                ref self_arg_positions,
                ..
            } => self.invoke_dyn_unified(
                runtime_function_id,
                &trait_object,
                arg_count,
                receiver_idx,
                ctx,
                &[],
                self_arg_positions.as_slice(),
            ),
            VTableEntry::Generic { .. } => {
                // Generic: at the bytecode tier the impl method is
                // already monomorphic-shaped (accepts raw arg slots,
                // dispatches internally). No TypeInfo threading is
                // emitted at the current Shape bytecode layer.
                // Treat as Direct for runtime dispatch; the impl's
                // body handles the polymorphism. See §Q25.C.3 spec
                // note above for the deferred amendment shape.
                self.invoke_dyn_unified(
                    runtime_function_id,
                    &trait_object,
                    arg_count,
                    receiver_idx,
                    ctx,
                    &[],
                    &[],
                )
            }
            VTableEntry::Compound {
                ref wrap_targets,
                ref self_arg_positions,
                ..
            } => self.invoke_dyn_unified(
                runtime_function_id,
                &trait_object,
                arg_count,
                receiver_idx,
                ctx,
                wrap_targets.as_slice(),
                self_arg_positions.as_slice(),
            ),
            VTableEntry::Closure {
                function_id,
                type_id: _,
            } => self.invoke_dyn_closure(
                function_id,
                &trait_object,
                arg_count,
                receiver_idx,
                ctx,
            ),
        }
    }

    /// Unified dispatch for `Direct` / `BoxedReturn` / `SelfArg` /
    /// `Generic` / `Compound` variants. The variant differences are
    /// encoded by the `(wrap_targets, self_arg_positions)` arguments:
    ///  - `Direct` / `Generic`: both empty.
    ///  - `BoxedReturn(top-level)`: wrap_targets = [{path:[]}].
    ///  - `BoxedReturn(nested)`: wrap_targets has paths like [[0]], [[1]], etc.
    ///  - `SelfArg`: self_arg_positions non-empty.
    ///  - `Compound`: either or both non-empty.
    ///
    /// The dispatch sequence:
    ///  1. **SelfArg identity check** (§Q25.C.2): for each
    ///     `pos ∈ self_arg_positions`, peek `args[pos]` and verify
    ///     it's a `TraitObject` whose vtable matches the receiver's
    ///     via `Arc::ptr_eq` (canonical equality per
    ///     `TraitObjectStorage::vtable_eq`).
    ///  2. **Argument lowering**: replace the receiver slot's `dyn`
    ///     carrier with the inner `Arc<TypedObjectStorage>`. For each
    ///     `Self`-typed argument (per `self_arg_positions`), also
    ///     replace the slot with the inner TypedObject (the impl
    ///     method expects concrete-typed args, not dyn carriers).
    ///  3. **Call**: route through `call_function_with_nb_args` +
    ///     `execute_until_call_depth` per the canonical call path.
    ///  4. **Return wrap** (§Q25.C.1): walk every `WrapTarget::path`
    ///     through the return value and re-box at the leaf.
    fn invoke_dyn_unified(
        &mut self,
        function_id: u16,
        trait_object: &Arc<TraitObjectStorage>,
        arg_count: usize,
        receiver_idx: usize,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
        wrap_targets: &[WrapTarget],
        self_arg_positions: &[u8],
    ) -> Result<(), VMError> {
        // Step 1: SelfArg identity check per §Q25.C.2. The arg slots
        // live at receiver_idx + 1 + pos (pos is 0-based, receiver-
        // excluded per `build_and_register_vtable`).
        for &pos in self_arg_positions {
            let arg_idx = receiver_idx
                .checked_add(1)
                .and_then(|x| x.checked_add(pos as usize))
                .ok_or_else(|| {
                    VMError::RuntimeError(
                        "DynMethodCall SelfArg: arg_idx arithmetic overflow"
                            .to_string(),
                    )
                })?;
            if arg_idx >= self.sp {
                return Err(VMError::RuntimeError(format!(
                    "DynMethodCall SelfArg: position {} out of range \
                     (receiver_idx={}, sp={})",
                    pos, receiver_idx, self.sp
                )));
            }
            let (arg_bits, arg_kind) = self.stack_read_kinded_raw(arg_idx);
            let arg_trait_object: &TraitObjectStorage = match arg_kind {
                NativeKind::Ptr(HeapKind::TraitObject) => {
                    if arg_bits == 0 {
                        return Err(VMError::RuntimeError(format!(
                            "DynMethodCall SelfArg: null TraitObject \
                             pointer at arg position {}",
                            pos
                        )));
                    }
                    // SAFETY: kind=Ptr(TraitObject); bits are
                    // `Arc::into_raw::<TraitObjectStorage>(arc)` per
                    // §2.3 typed-Arc invariant. Transient borrow —
                    // we hold no `Arc<...>` from this raw pointer,
                    // just deref the live storage.
                    unsafe { &*(arg_bits as *const TraitObjectStorage) }
                }
                other => {
                    return Err(VMError::RuntimeError(format!(
                        "DynMethodCall SelfArg: position {} expected \
                         trait object (NativeKind::Ptr(HeapKind::TraitObject)), \
                         got {:?}. Per ADR-006 §2.7.24 Q25.C.2 a Self-typed \
                         argument flowing through `dyn T` must itself be a \
                         trait object so the vtable-identity check can run.",
                        pos, other
                    )));
                }
            };
            if !trait_object.vtable_eq(arg_trait_object) {
                return Err(VMError::RuntimeError(format!(
                    "DynMethodCall SelfArg: vtable identity mismatch at \
                     argument position {}. Per ADR-006 §2.7.24 Q25.C.2 \
                     `Self` in argument position requires the argument's \
                     concrete type to match the receiver's. Receiver \
                     trait(s): {:?}; argument trait(s): {:?}.",
                    pos, trait_object.vtable.trait_names,
                    arg_trait_object.vtable.trait_names
                )));
            }
        }

        // Step 2: lower the receiver and any Self-typed arguments
        // from `dyn` carriers to their inner `Arc<TypedObjectStorage>`.
        // `stack_write_kinded` drops the previous occupant (releasing
        // the `Arc<TraitObjectStorage>` share) and installs the
        // new share.
        let inner_typed_object = Arc::clone(&trait_object.value);
        let new_bits = Arc::into_raw(inner_typed_object) as u64;
        let new_kind = NativeKind::Ptr(HeapKind::TypedObject);
        self.stack_write_kinded(receiver_idx, new_bits, new_kind);

        for &pos in self_arg_positions {
            let arg_idx = receiver_idx + 1 + (pos as usize);
            let (arg_bits, arg_kind) = self.stack_read_kinded_raw(arg_idx);
            // Already validated as TraitObject above.
            debug_assert_eq!(arg_kind, NativeKind::Ptr(HeapKind::TraitObject));
            // SAFETY: validated above; transient borrow to read the
            // inner typed object share, then clone-bump and install.
            let arg_to: &TraitObjectStorage = unsafe {
                &*(arg_bits as *const TraitObjectStorage)
            };
            let arg_inner = Arc::clone(&arg_to.value);
            let new_arg_bits = Arc::into_raw(arg_inner) as u64;
            self.stack_write_kinded(arg_idx, new_arg_bits, new_kind);
        }

        // Step 3: collect receiver + args + call.
        let total = arg_count + 1;
        let mut args: Vec<KindedSlot> = Vec::with_capacity(total);
        for i in 0..total {
            let (b, k) = self.stack_take_kinded(receiver_idx + i);
            args.push(KindedSlot::new(ValueSlot::from_raw(b), k));
        }
        self.sp = receiver_idx;

        let saved_depth = self.call_stack.len();
        self.call_function_with_nb_args(function_id, &args)?;
        for slot in args {
            std::mem::forget(slot);
        }
        self.execute_until_call_depth(saved_depth, ctx)?;

        // Step 4: pop the result, then walk wrap_targets to re-box
        // every Self-named site.
        let (ret_bits, ret_kind) = self.pop_kinded()?;
        if wrap_targets.is_empty() {
            // Direct / Generic / SelfArg-no-return-Self: push as-is.
            self.push_kinded(ret_bits, ret_kind)?;
            return Ok(());
        }
        let (wrapped_bits, wrapped_kind) = rewrap_return_value(
            ret_bits,
            ret_kind,
            wrap_targets,
            &trait_object.vtable,
        )?;
        self.push_kinded(wrapped_bits, wrapped_kind)?;
        Ok(())
    }

    /// `VTableEntry::Closure` dispatch — W7 closure-trait-impl through
    /// `dyn T` per ADR-006 §2.7.24 Q25.C.5 + §2.7.11/Q12.
    ///
    /// The trait object's inner `Arc<TypedObjectStorage>` is itself a
    /// closure-bearing carrier (the W7 layout — the TypedObject's
    /// schema carries a closure-typed field). The dispatch path
    /// routes through `call_value_immediate_nb` with the closure as
    /// the callee, the receiver as `self` (positional arg 0), and
    /// the rest of the args following.
    ///
    /// `function_id` (the vtable entry's compiler-tier function id)
    /// is currently unused at this dispatch tier — W7 stores it for
    /// IC stabilization in §Q25.C.6, but the runtime call routes
    /// through the typed closure header on the receiver itself per
    /// §2.7.11/Q12. `type_id` similarly is metadata for IC.
    fn invoke_dyn_closure(
        &mut self,
        _function_id: u32,
        _trait_object: &Arc<TraitObjectStorage>,
        arg_count: usize,
        receiver_idx: usize,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        // phase-2d-hardening:(f) — VTableEntry::Closure dispatch
        // routes through `call_value_immediate_nb` per §2.7.11/Q12.
        // The current `op_make_closure` / `OwnedClosureBlock` shape
        // doesn't have a registered emission path that constructs
        // `VTableEntry::Closure` entries (W7 storage exists, W7
        // emission is out of scope for W17-trait-object-thunks).
        // The full receiver-as-closure dispatch wire-through would
        // duplicate the §2.7.11 frame-setup invariants; reserve
        // that work for the future W17-trait-object-closure-call
        // sub-cluster.
        //
        // For now, surface-and-stop with a structured error so the
        // shape of the dispatch is visible without faking a
        // closure-call. The variant is reachable only via §Q25.C.5
        // entries that emission would have to construct — and the
        // current `build_and_register_vtable` doesn't emit it.
        let _ = (arg_count, receiver_idx, ctx);
        Err(VMError::NotImplemented(
            "SURFACE: DynMethodCall Closure variant per ADR-006 §2.7.24 \
             Q25.C.5 + §2.7.11/Q12 — W7 closure-trait-impl dispatch through \
             dyn requires receiver-as-closure routing through \
             `call_value_immediate_nb`. The thunks tier (Wave 3 \
             W17-trait-object-thunks) reserves dispatch wire-through for \
             a future sub-cluster pending W7 emission. Storage shapes \
             ready; emission gates the dispatch.".to_string(),
        ))
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

/// Walk every `WrapTarget::path` through the return value and re-box
/// each `Self`-named site into a fresh `Arc<TraitObjectStorage>`
/// using `receiver_vtable`.
///
/// Algorithm per ADR-006 §2.7.24 Q25.C.1 / Q25.C.5 wrap-target
/// encoding:
///  1. Group wrap_targets by their first path-step (the outer
///     generic-arg index reached by the descent).
///  2. The return value is structurally one of:
///     - `Self` directly → wrap_targets contains `path=[]`; consume
///       the value as `Arc<TypedObjectStorage>` and re-box.
///     - `Result<T, E>` → outer is `HeapKind::Result`; wrap_targets
///       at `path[0]=0` apply to the Ok arm payload, `path[0]=1`
///       to the Err arm. Result is single-payload-discriminated.
///     - `Option<T>` → outer is `HeapKind::Option`; only `path[0]=0`
///       makes sense (None has no payload).
///     - `HashMap<K, V>` with V=Self → applies to the values buffer;
///       descend into each value.
///     - tuple → represented as `Arc<TypedObjectStorage>` with
///       numbered fields per the C+ amendment; descend by field.
///
/// The function consumes the (ret_bits, ret_kind) share and returns
/// a new (bits, kind) share.
fn rewrap_return_value(
    ret_bits: u64,
    ret_kind: NativeKind,
    wrap_targets: &[WrapTarget],
    receiver_vtable: &Arc<VTable>,
) -> Result<(u64, NativeKind), VMError> {
    // Top-level Self: wrap_targets contains a path=[] entry. Consume
    // the return as a TypedObject and re-box. Any additional
    // wrap_targets at path=[] are coalesced (re-boxing a Self return
    // once is sufficient).
    let has_top_level = wrap_targets.iter().any(|w| w.path.is_empty());
    if has_top_level {
        return rebox_self_value(ret_bits, ret_kind, receiver_vtable);
    }

    // Nested wrap-targets — walk by outer generic constructor.
    // Currently `ret_kind` is the structural carrier returned by the
    // impl. Dispatch on the discriminator to find the substructure
    // each wrap-target targets.
    match ret_kind {
        NativeKind::Ptr(HeapKind::Result) => {
            rewrap_result_payload(ret_bits, wrap_targets, receiver_vtable)
        }
        NativeKind::Ptr(HeapKind::Option) => {
            rewrap_option_payload(ret_bits, wrap_targets, receiver_vtable)
        }
        NativeKind::Ptr(HeapKind::TypedObject) => {
            // Tuples / records — represented as a TypedObject with
            // numbered or named fields. Descend into each wrap-
            // target's first path step (interpreted as a 0-based
            // field index per the C+ amendment row 2 of
            // playbook §3 W17-typed-carrier rescope note).
            rewrap_typed_object_fields(ret_bits, wrap_targets, receiver_vtable)
        }
        NativeKind::Ptr(HeapKind::HashMap) => {
            // HashMap<K, Self> case. The values buffer is the
            // Self-named site; descend by buffer entry and re-box.
            // Path=[1] is the value position (path=[0] would be keys
            // but Erase_T doesn't auto-box keys; if a method returns
            // HashMap<Self, V> that's an unusual shape — surface).
            rewrap_hashmap_values(ret_bits, wrap_targets, receiver_vtable)
        }
        NativeKind::Ptr(HeapKind::TypedArray) => {
            // `Array<Self>` — descend into each element.
            rewrap_typed_array_elements(ret_bits, wrap_targets, receiver_vtable)
        }
        other => {
            drop_kinded(ret_bits, ret_kind);
            Err(VMError::NotImplemented(format!(
                "SURFACE: DynMethodCall nested BoxedReturn dispatch on \
                 return kind {:?} per ADR-006 §2.7.24 Q25.C.5 — the \
                 structural carrier doesn't have a registered wrap-target \
                 descent path. Supported: Result / Option / TypedObject \
                 (tuples & records) / HashMap / TypedArray. Wrap-targets: \
                 {:?}.",
                other,
                wrap_targets.iter().map(|w| w.path.as_slice()).collect::<Vec<_>>()
            )))
        }
    }
}

/// Consume a (bits, kind) share that names `Self` at the leaf and
/// re-box it into a fresh `Arc<TraitObjectStorage>`. Accepts an
/// already-wrapped TraitObject (passthrough — the impl may have
/// internally re-boxed).
fn rebox_self_value(
    bits: u64,
    kind: NativeKind,
    receiver_vtable: &Arc<VTable>,
) -> Result<(u64, NativeKind), VMError> {
    match kind {
        NativeKind::Ptr(HeapKind::TypedObject) => {
            if bits == 0 {
                return Err(VMError::RuntimeError(
                    "DynMethodCall BoxedReturn: null TypedObject return"
                        .to_string(),
                ));
            }
            // SAFETY: kind=Ptr(TypedObject); bits are
            // `Arc::into_raw::<TypedObjectStorage>(arc)`. The caller's
            // share transfers to us.
            let inner: Arc<TypedObjectStorage> = unsafe {
                Arc::from_raw(bits as *const TypedObjectStorage)
            };
            let new_to = Arc::new(TraitObjectStorage::new(
                inner,
                Arc::clone(receiver_vtable),
            ));
            let to_bits = Arc::into_raw(new_to) as u64;
            Ok((to_bits, NativeKind::Ptr(HeapKind::TraitObject)))
        }
        NativeKind::Ptr(HeapKind::TraitObject) => {
            // Already a dyn carrier — passthrough.
            Ok((bits, kind))
        }
        _ => {
            drop_kinded(bits, kind);
            Err(VMError::NotImplemented(format!(
                "SURFACE: DynMethodCall BoxedReturn with scalar leaf \
                 kind {:?} requires universal-dyn auto-boxing of non-\
                 TypedObject kinds per ADR-006 §2.7.24 Q25.C.1; scalar \
                 trait-object payloads are deferred (emission-shell \
                 currently surfaces these at the `op_box_trait_object` \
                 path; lifting this requires emission-tier scalar-\
                 to-TypedObject auto-box).",
                kind
            )))
        }
    }
}

/// Re-wrap inside a `Result<T, E>` carrier. The Result has a single
/// `payload: KindedSlot` arm; `is_ok` selects which generic-arg the
/// payload corresponds to. We re-box the payload IFF the arm matches
/// a wrap_target's first path step.
fn rewrap_result_payload(
    ret_bits: u64,
    wrap_targets: &[WrapTarget],
    receiver_vtable: &Arc<VTable>,
) -> Result<(u64, NativeKind), VMError> {
    if ret_bits == 0 {
        return Err(VMError::RuntimeError(
            "DynMethodCall BoxedReturn: null Result return".to_string(),
        ));
    }
    // SAFETY: kind=Ptr(Result); bits are
    // `Arc::into_raw::<ResultData>(arc)`. Consume the share.
    let result: Arc<ResultData> =
        unsafe { Arc::from_raw(ret_bits as *const ResultData) };
    // Determine whether to re-box the payload. Path=[0] applies to
    // the Ok arm, path=[1] to the Err arm; matching against the
    // result's `is_ok` selects which we descend into.
    let arm_index: u8 = if result.is_ok { 0 } else { 1 };
    let descendants: SmallVec<[WrapTarget; 2]> = wrap_targets
        .iter()
        .filter(|w| !w.path.is_empty() && w.path[0] == arm_index)
        .map(|w| WrapTarget {
            path: w.path[1..].iter().copied().collect(),
            wrap_as_trait_id: w.wrap_as_trait_id,
        })
        .collect();
    if descendants.is_empty() {
        // The arm we're in doesn't have a wrap-target — return as-is.
        let raw = Arc::into_raw(result) as u64;
        return Ok((raw, NativeKind::Ptr(HeapKind::Result)));
    }
    // Rewrap the payload. Pull it out, recurse, install fresh.
    // Cloning the ResultData lets us mutate the new copy's payload
    // without disturbing other shared references (Arc::make_mut
    // semantics, but we synthesize a fresh Arc since the descendant
    // recursion already consumed shares).
    let mut new_result = (*result).clone();
    // The cloned payload owns its own share (per KindedSlot::Clone).
    // Take its bits + kind without disturbing its Drop — we'll
    // install the rewrapped result.
    let payload_bits = new_result.payload.raw();
    let payload_kind = new_result.payload.kind();
    std::mem::forget(std::mem::replace(
        &mut new_result.payload,
        KindedSlot::none(),
    ));
    let (new_payload_bits, new_payload_kind) =
        rewrap_return_value(payload_bits, payload_kind, &descendants, receiver_vtable)?;
    new_result.payload = KindedSlot::new(
        ValueSlot::from_raw(new_payload_bits),
        new_payload_kind,
    );
    // Drop the borrowed `result` (releases the original share).
    drop(result);
    let new_arc = Arc::new(new_result);
    let raw = Arc::into_raw(new_arc) as u64;
    Ok((raw, NativeKind::Ptr(HeapKind::Result)))
}

/// Re-wrap inside an `Option<T>` carrier — analogous to Result but
/// single-arm (None has no payload).
fn rewrap_option_payload(
    ret_bits: u64,
    wrap_targets: &[WrapTarget],
    receiver_vtable: &Arc<VTable>,
) -> Result<(u64, NativeKind), VMError> {
    if ret_bits == 0 {
        return Err(VMError::RuntimeError(
            "DynMethodCall BoxedReturn: null Option return".to_string(),
        ));
    }
    // SAFETY: kind=Ptr(Option); bits are
    // `Arc::into_raw::<OptionData>(arc)`. Consume the share.
    let option: Arc<OptionData> =
        unsafe { Arc::from_raw(ret_bits as *const OptionData) };
    if !option.is_some {
        // None: nothing to re-box.
        let raw = Arc::into_raw(option) as u64;
        return Ok((raw, NativeKind::Ptr(HeapKind::Option)));
    }
    let descendants: SmallVec<[WrapTarget; 2]> = wrap_targets
        .iter()
        .filter(|w| !w.path.is_empty() && w.path[0] == 0)
        .map(|w| WrapTarget {
            path: w.path[1..].iter().copied().collect(),
            wrap_as_trait_id: w.wrap_as_trait_id,
        })
        .collect();
    if descendants.is_empty() {
        let raw = Arc::into_raw(option) as u64;
        return Ok((raw, NativeKind::Ptr(HeapKind::Option)));
    }
    let mut new_option = (*option).clone();
    let payload_bits = new_option.payload.raw();
    let payload_kind = new_option.payload.kind();
    std::mem::forget(std::mem::replace(
        &mut new_option.payload,
        KindedSlot::none(),
    ));
    let (new_payload_bits, new_payload_kind) =
        rewrap_return_value(payload_bits, payload_kind, &descendants, receiver_vtable)?;
    new_option.payload = KindedSlot::new(
        ValueSlot::from_raw(new_payload_bits),
        new_payload_kind,
    );
    drop(option);
    let new_arc = Arc::new(new_option);
    let raw = Arc::into_raw(new_arc) as u64;
    Ok((raw, NativeKind::Ptr(HeapKind::Option)))
}

/// Re-wrap inside a `TypedObject` that represents a tuple or record
/// (per the C+ playbook amendment for tuples → named-field TypedObject
/// records). Descend into each field index named by a wrap-target's
/// first path step.
///
/// This path is round-3 minimum: we don't currently mutate the inner
/// TypedObject in-place — we surface a structured error if the
/// emission tier ever produces a wrap-target for a TypedObject-return
/// shape, because the in-place field rewrite path would need
/// `TypedObjectStorage::write_slot_in_place` (added by W17-references-
/// mutation, see hardening (c)) plus a field-index reverse lookup
/// from the trait's tuple/record shape. The dispatch shell stays
/// correct without it; the work surfaces when the emission tier
/// actually emits a wrap-target with this carrier shape.
fn rewrap_typed_object_fields(
    ret_bits: u64,
    wrap_targets: &[WrapTarget],
    _receiver_vtable: &Arc<VTable>,
) -> Result<(u64, NativeKind), VMError> {
    drop_kinded(ret_bits, NativeKind::Ptr(HeapKind::TypedObject));
    Err(VMError::NotImplemented(format!(
        "SURFACE: DynMethodCall BoxedReturn with TypedObject-carrier \
         return + wrap_targets {:?} per ADR-006 §2.7.24 Q25.C.5 — \
         in-place tuple/record field re-box requires \
         `TypedObjectStorage::write_slot_in_place` integration with \
         a trait-declared field-index lookup. The dispatch shell \
         surfaces; lifting this is a follow-up sub-cluster pending \
         the typed-record-rewrap recipe.",
        wrap_targets.iter().map(|w| w.path.as_slice()).collect::<Vec<_>>()
    )))
}

/// Re-wrap inside a `HashMap` whose value arm names `Self`. Descend
/// into each entry's value buffer position.
///
/// Same shape-issue as `rewrap_typed_object_fields`: the in-place
/// values-buffer rewrite would need a typed-buffer mutation entry
/// for `HashMapValueBuf::TraitObject` (the storage carrier already
/// has the arm — see hashmap_methods.rs:259). Surfacing here keeps
/// the dispatch correct while the values-buffer write-path is built.
fn rewrap_hashmap_values(
    ret_bits: u64,
    wrap_targets: &[WrapTarget],
    _receiver_vtable: &Arc<VTable>,
) -> Result<(u64, NativeKind), VMError> {
    drop_kinded(ret_bits, NativeKind::Ptr(HeapKind::HashMap));
    Err(VMError::NotImplemented(format!(
        "SURFACE: DynMethodCall BoxedReturn with HashMap<K, Self> \
         return + wrap_targets {:?} per ADR-006 §2.7.24 Q25.C.5 — \
         values-buffer rewrap requires a typed-buffer write-path \
         that takes a `HashMapValueBuf::TraitObject` arm + a per-\
         entry re-box. The dispatch shell surfaces; lifting this \
         pairs with the `rewrap_typed_object_fields` follow-up.",
        wrap_targets.iter().map(|w| w.path.as_slice()).collect::<Vec<_>>()
    )))
}

/// Re-wrap inside a `TypedArray` whose elements name `Self`. Each
/// element is a `TypedObject`-shaped Self leaf; rewrap into a
/// `TypedArrayData::TraitObject` buffer using the receiver vtable.
fn rewrap_typed_array_elements(
    ret_bits: u64,
    wrap_targets: &[WrapTarget],
    receiver_vtable: &Arc<VTable>,
) -> Result<(u64, NativeKind), VMError> {
    if ret_bits == 0 {
        return Err(VMError::RuntimeError(
            "DynMethodCall BoxedReturn: null TypedArray return".to_string(),
        ));
    }
    // SAFETY: kind=Ptr(TypedArray); bits are
    // `Arc::into_raw::<TypedArrayData>(arc)`. Consume.
    let arr: Arc<shape_value::heap_value::TypedArrayData> =
        unsafe { Arc::from_raw(ret_bits as *const shape_value::heap_value::TypedArrayData) };
    // Only path=[0] applies to array elements (single generic arg).
    let descendants: SmallVec<[WrapTarget; 2]> = wrap_targets
        .iter()
        .filter(|w| !w.path.is_empty() && w.path[0] == 0)
        .map(|w| WrapTarget {
            path: w.path[1..].iter().copied().collect(),
            wrap_as_trait_id: w.wrap_as_trait_id,
        })
        .collect();
    if descendants.is_empty() {
        let raw = Arc::into_raw(arr) as u64;
        return Ok((raw, NativeKind::Ptr(HeapKind::TypedArray)));
    }
    // Read the array's elements per its current TypedArrayData arm
    // and re-box each into a TraitObject. The most common arm for
    // a `Self`-returning method on a TypedObject impl is
    // `TypedArrayData::TypedObject` (the impl built the array as a
    // typed buffer of its concrete type).
    use shape_value::heap_value::TypedArrayData;
    match &*arr {
        TypedArrayData::TypedObject(buf) => {
            // Build a new TraitObject buffer.
            let new_data: Vec<Arc<TraitObjectStorage>> = buf
                .data
                .iter()
                .map(|to_inner| {
                    Arc::new(TraitObjectStorage::new(
                        Arc::clone(to_inner),
                        Arc::clone(receiver_vtable),
                    ))
                })
                .collect();
            let new_buf = Arc::new(
                shape_value::typed_buffer::TypedBuffer::from_vec(new_data),
            );
            let new_arr = Arc::new(TypedArrayData::TraitObject(new_buf));
            let raw = Arc::into_raw(new_arr) as u64;
            drop(arr);
            Ok((raw, NativeKind::Ptr(HeapKind::TypedArray)))
        }
        TypedArrayData::TraitObject(_) => {
            // Already a TraitObject buffer — passthrough.
            let raw = Arc::into_raw(arr) as u64;
            Ok((raw, NativeKind::Ptr(HeapKind::TypedArray)))
        }
        other => {
            // Other element kinds with Self-named target are an
            // emission-tier shape we don't expect — surface.
            let other_name = match other {
                TypedArrayData::F64(_) => "F64",
                TypedArrayData::I64(_) => "I64",
                TypedArrayData::String(_) => "String",
                _ => "<other>",
            };
            drop(arr);
            Err(VMError::NotImplemented(format!(
                "SURFACE: DynMethodCall BoxedReturn TypedArray with \
                 element arm {} + wrap_targets {:?} — Self-leaf re-box \
                 expects TypedObject-arm element buffer (the impl built \
                 a typed buffer of its concrete type). Per ADR-006 \
                 §2.7.24 Q25.C.5.",
                other_name,
                wrap_targets.iter().map(|w| w.path.as_slice()).collect::<Vec<_>>()
            )))
        }
    }
}



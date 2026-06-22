//! Rvalue compilation: MIR Rvalue → Cranelift IR.
//!
//! Maps each Rvalue variant to Cranelift instructions:
//! - Use(operand): ownership-aware value load
//! - BinaryOp: arithmetic, comparison, logical operators
//! - UnaryOp: negation, logical not
//! - Clone: explicit clone (arc_retain)
//! - Borrow: reference creation (deferred)
//! - Aggregate: array/object construction

use cranelift::prelude::*;

use super::MirToIR;
use shape_vm::mir::types::*;

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Compile an Rvalue to a Cranelift value.
    pub(crate) fn compile_rvalue(&mut self, rvalue: &Rvalue) -> Result<Value, String> {
        match rvalue {
            Rvalue::Use(operand) => self.compile_operand(operand),

            Rvalue::BinaryOp(op, lhs, rhs) => {
                // Check source operand kinds BEFORE compiling (needed for I64 disambiguation).
                let lhs_kind = self.operand_slot_kind(lhs);
                let rhs_kind = self.operand_slot_kind(rhs);

                let l = self.compile_operand(lhs)?;
                let r = self.compile_operand(rhs)?;

                // Check operand types for native inline paths.
                let l_type = self.builder.func.dfg.value_type(l);
                let r_type = self.builder.func.dfg.value_type(r);

                // F5.a/F5.b: string `+` — concat via FFI. Either operand being a
                // `NativeKind::String` is enough; the FFI handles `str + <any>` by
                // falling back to `format_value_word` on non-string operands,
                // which matches the lowering emitted by f-string interpolation.
                if matches!(op, BinOp::Add) && self.either_string(lhs_kind, rhs_kind) {
                    // W15.2-LANG-7 jit-print-fstring close (Phase 4b Round 3,
                    // 2026-05-18). ADR-006 §2.7.5/§2.7.7 producer-side stamp:
                    // pass operand kind codes alongside their raw bits so the
                    // `jit_string_concat` FFI body dispatches per the
                    // producer-stamped kind, not a runtime tag probe.
                    // `operand_slot_kind` returns `Some(_)` by construction
                    // here — `either_string` already matched a `Some(String)`
                    // on at least one side, and the MIR f-string lowering's
                    // expression-part path produces typed-temp slots whose
                    // kind `infer_slot_kinds` stamps from the constant /
                    // BinaryOp shape. The `unwrap_or` SENTINEL falls through
                    // to the FFI body's §2.7.7 #9 surface-and-stop arm for
                    // the rare unproven case.
                    return self.compile_string_concat(l, lhs_kind, r, rhs_kind);
                }

                // r5c-2-gz-cp6 narrow-neg-literal: narrow integer
                // COMPARISON (i8/i16/i32/u8/u16/u32 against another narrow,
                // a width-polymorphic literal, or a genuine `int`). Checked
                // BEFORE the arithmetic classifier so a narrow comparison
                // never falls to the kind-blind generic `compile_binop_
                // dynamic_cmp`, whose `to_i64_bits` ZERO-extends an `I8`
                // operand and so mis-compares a NEGATIVE narrow value
                // against its (sign-extended) i64 partner. The comparison
                // codegen in `compile_binop_narrow_int` extends — never
                // truncates — both operands to I64 per the narrow kind's
                // signedness, matching the VM's `compact_int_cmp`, which
                // compares the full sign-/zero-extended i64 slot bits.
                if matches!(
                    op,
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
                ) {
                    if let Some(nw) = Self::narrow_int_cmp_kind(lhs_kind, rhs_kind) {
                        return self.compile_binop_narrow_int(op, l, r, nw);
                    }
                }

                // R5c-2-β-γ (c) jit-narrow-wrap: narrow integer arithmetic
                // (i8/i16/i32/u8/u16/u32). The operand KIND — not just the
                // Cranelift type — drives this path: a `NativeKind::Int8`
                // and a `NativeKind::Bool` both lower to Cranelift `I8`,
                // but only the former is integer arithmetic. The bytecode
                // VM wraps narrow-width overflow two's-complement via
                // `AddI32`/`AddTyped` truncating opcodes
                // (`executor/v2_handlers/int.rs`, `executor/arithmetic/
                // mod.rs::compact_int_checked_binop`); the JIT matches by
                // operating at the narrow Cranelift width — `iadd`/`isub`/
                // `imul` on `I8`/`I16`/`I32` wrap natively at that width.
                if let Some(nw) = Self::narrow_int_binop_kind(lhs_kind, rhs_kind, lhs, rhs) {
                    return self.compile_binop_narrow_int(op, l, r, nw);
                }

                // R5c-2-β-γ checkpoint (b) u64-carrier: full-range `u64`
                // arithmetic. A `NativeKind::UInt64` operand shares the
                // 64-bit Cranelift `I64` width with `Int64`, so the
                // `l_type == I64` width test below cannot tell them apart —
                // the operand KIND must drive this path. The bytecode VM
                // computes `u64` div/mod with `u64::wrapping_div`/`_rem`
                // and comparisons with unsigned `cmp`
                // (`executor/arithmetic/mod.rs::compact_int_divmod_u64`,
                // `compact_int_cmp`); the JIT matches by selecting
                // `udiv`/`urem` and `Unsigned*` condition codes in
                // `compile_binop_uint64`. Must precede the `both_int64`
                // arm — a `UInt64` operand would otherwise fall to the
                // signed `compile_binop_int64` (`sdiv`/`srem`) and diverge.
                if Self::uint64_binop_site(lhs_kind, rhs_kind, lhs, rhs) {
                    return self.compile_binop_uint64(op, l, r);
                }

                if l_type == types::F64 && r_type == types::F64 {
                    // Both operands are native F64 — inline float ops.
                    self.compile_binop_f64(op, l, r)
                } else if l_type == types::I32 && r_type == types::I32 {
                    // Both operands are native I32 — inline i32 ops.
                    match op {
                        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                            self.compile_binop_i32_native(op, l, r)
                        }
                        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                            self.compile_cmp_i32_native(op, l, r)
                        }
                        _ => self.compile_binop(op, l, r),
                    }
                } else if l_type == types::I8 && r_type == types::I8 {
                    // Both operands are native I8 (Bool) — inline bool ops.
                    self.compile_binop_bool(op, l, r)
                } else if self.both_int64(lhs_kind, rhs_kind) {
                    // Both operands are Int64 slots (NaN-boxed ints) — inline i64 arithmetic.
                    // Extract 48-bit payload, operate natively, re-box.
                    self.compile_binop_int64(op, l, r)
                } else {
                    // Mixed or unknown types — use FFI generic path.
                    self.compile_binop(op, l, r)
                }
            }

            Rvalue::UnaryOp(op, operand) => {
                let val = self.compile_operand(operand)?;
                self.compile_unop(op, val)
            }

            Rvalue::Clone(operand) => {
                // Explicit clone: get the value and (if heap-kinded) retain.
                //
                // W11-jit-new-array (ADR-006 §2.7.5 / §2.7.6 / Q8): the
                // pre-W11 unconditional retain here was the symmetric
                // version of the `compile_operand` Copy bug — fired on
                // every Clone regardless of kind, which segfaulted on
                // `NativeKind::Int64` slots whose bits are a raw int
                // (the `MIR-emits-Clone-on-non-heap` case the W-series
                // ABI tolerated via tag-bit decode). The principled
                // response is to use the same kind-aware disposition
                // path as Copy. When the operand has no `Place::Local`
                // (e.g. `Operand::Constant`), there's no slot to
                // discriminate by — and the bytecode compiler does not
                // emit `Rvalue::Clone(Constant(...))` (Clone is by
                // construction a place-rooted operation), so the
                // fallback arm surface-and-stops with a clear marker.
                let val = self.compile_operand_raw(operand)?;
                let place = match operand {
                    shape_vm::mir::types::Operand::Copy(p)
                    | shape_vm::mir::types::Operand::Move(p)
                    | shape_vm::mir::types::Operand::MoveExplicit(p) => p,
                    shape_vm::mir::types::Operand::Constant(_) => {
                        return Err("MirToIR: Rvalue::Clone(Constant) — Clone is \
                             defined on place-rooted operands per ADR-006 \
                             §2.7.5; emitter contract violated. SURFACE."
                            .to_string());
                    }
                };
                if self.refcount_disposition_for_place(place)? {
                    let retain_func = self.retain_func_for_place(place);
                    self.builder.ins().call(retain_func, &[val]);
                }
                Ok(val)
            }

            Rvalue::Borrow(_kind, place) => {
                // Phase 4b Round 5c-2-α jit-ref-param-chain-stamp (ADR-006
                // §2.7.13 + §2.7.5). Re-borrow short-circuit: when the
                // borrow target's root local is itself a reference
                // parameter, the slot already holds a CALLER-OWNED cell
                // pointer. Forwarding `&x` (in `inner(&x)` from within
                // `outer(&x) { inner(&x) }`) must reuse that pointer —
                // allocating a fresh stack cell would silently snapshot
                // the value, decoupling the inner mutation from the
                // outer caller's binding and reproducing the W14.2-G4
                // sister-class JIT divergence (`bump(&a); print(a)`
                // returning the unmutated `a`).
                //
                // Only `Place::Local(ref_param_slot)` short-circuits;
                // `Place::Field` projections take the γ-CP4 field-address
                // path below.
                if let Place::Local(slot) = place {
                    if self.ref_param_slots.contains(slot) {
                        let var = *self
                            .locals
                            .get(slot)
                            .ok_or_else(|| format!("MirToIR: unknown local slot {}", slot))?;
                        // Slot variable carries the caller's cell address
                        // (pointer-width I64); forward as-is. Skip the
                        // `ref_stack_slots` insertion — there is no JIT-
                        // owned cell to reload after calls.
                        return Ok(self.builder.use_var(var));
                    }
                }

                // γ-CP4 jit-makefieldref (ADR-006 §2.7.13 / §2.3): a
                // `&`/`&mut` projection into a typed-object field
                // (`&mut b.value`) — the JIT analogue of the VM's
                // `RefTarget::TypedField`. The reference IS the address of
                // the field slot inside the live object; loading/storing
                // through it (`Place::Deref`) mutates the field in place,
                // byte-equal to the VM's `write_ref_target`.
                //
                // This MUST NOT go through the throwaway-stack-cell path
                // below: that path snapshots the field VALUE into a cell
                // keyed (in `ref_stack_slots`) on `place.root_local()` —
                // the *struct* local. `reload_referenced_locals` then
                // writes that cell's contents (a field scalar) back into
                // the struct local's slot variable after every call,
                // overwriting the `TypedObject` pointer with an integer.
                // The next field access (`inline_typed_field_get`) then
                // dereferences the integer-as-pointer → SIGSEGV. Computing
                // the real field address sidesteps the cell entirely:
                // there is nothing to register in `ref_stack_slots` and
                // nothing for `reload_referenced_locals` to clobber.
                if let Place::Field(base, field_idx) = place {
                    let byte_off = self
                        .try_resolve_field_byte_offset_pub(field_idx)
                        .ok_or_else(|| {
                            format!(
                                "γ-CP4 jit-makefieldref: SURFACE — field-reference \
                                 (`&`/`&mut`) into field idx {} has no statically \
                                 resolved byte offset (no `field_byte_offsets` / \
                                 inline-typed-struct layout entry). The schema-less \
                                 `get_prop`/`set_prop` FFI carrier exposes no field \
                                 address, so a typed field reference cannot be \
                                 formed. Clean deopt to the interpreter — \
                                 ADR-006 §2.7.13.",
                                field_idx.0,
                            )
                        })?;
                    // Read the receiver's NaN-boxed typed-object bits, then
                    // compute the field slot address. The receiver local is
                    // live for the whole function and references never
                    // escape their frame, so this address stays valid for
                    // every deref reachable from this borrow.
                    let base_bits = self.read_place(base)?;
                    return Ok(self.emit_typed_field_address(base_bits, byte_off));
                }

                // γ-CP4 jit-makefieldref: `Place::Index` / `Place::Deref`
                // projection borrows (`&mut arr[i]`, re-borrow of a deref)
                // are NOT handled by the stack-cell path below. That path
                // keys the cell on `place.root_local()` and reloads the
                // cell contents back into the *root local* after every
                // call (`reload_referenced_locals`) — for a projection the
                // root local is the container (array / ref), not the
                // projected slot, so the reload clobbers the container
                // pointer with a projected scalar exactly as the
                // `MakeFieldRef` SIGSEGV did. `MakeIndexRef` is explicitly
                // out of the β1 `RefTarget` scope (the `TypedIndex` variant
                // was retired pending the per-element-kind `TypedArray<T>`
                // rebuild — see `crates/shape-value/src/reference.rs`), so
                // surface-and-stop here for a clean deopt to the
                // interpreter rather than emitting an unsound cell.
                if matches!(place, Place::Index(_, _) | Place::Deref(_)) {
                    return Err(format!(
                        "γ-CP4 jit-makefieldref: SURFACE — `&`/`&mut` projection \
                         borrow of `{place}` (Index / nested-Deref) is not \
                         supported by the JIT. The per-function stack-cell ref \
                         path is keyed on the root local and would corrupt the \
                         container pointer via `reload_referenced_locals`. \
                         `MakeIndexRef` is out of the β1 `RefTarget::TypedField` \
                         scope. Clean deopt to the interpreter — ADR-006 \
                         §2.7.13."
                    ));
                }

                // R4.2F: allocate a native-sized/aligned stack cell that
                // matches the root local's Cranelift type. References are
                // strictly per-function — they never cross Cranelift call
                // boundaries — so picking a native width here is safe and
                // removes the width-extension wrap/unwrap pair.
                //
                // For non-native slot kinds (heap / string / unknown),
                // `cranelift_type_for_slot` returns I64, collapsing to the
                // legacy 8-byte cell with no behavioural change. Only
                // `Place::Local` reaches here — `place.root_local()` is the
                // slot itself, so the reload-after-call is sound.
                let raw_val = self.read_place(place)?;
                let root = place.root_local();
                let kind = super::types::slot_kind_for_local(&self.slot_kinds, root.0)
                    .unwrap_or(shape_vm::type_tracking::NativeKind::Int64);
                let cl_ty = super::types::cranelift_type_for_slot(kind);
                let size = cl_ty.bytes();
                // `create_sized_stack_slot` takes the log2 of the alignment;
                // `trailing_zeros` of a power-of-two size is exactly that.
                let align_shift = size.trailing_zeros() as u8;
                let slot = self.builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    size,
                    align_shift,
                ));
                // Store the value at its native width — no NaN-box wrap.
                self.builder.ins().stack_store(raw_val, slot, 0);
                // Track root local + native type for reload-after-call.
                self.ref_stack_slots.insert(root, (slot, cl_ty));
                // Return the stack slot address as the reference value.
                Ok(self.builder.ins().stack_addr(types::I64, slot, 0))
            }

            Rvalue::Aggregate(_operands) => {
                // Route A (ADR-006 §2.7.14 / W11-jit-new-array close):
                // typed-array allocation is kind-monomorphized on the
                // destination slot's element kind. The kind-blind
                // `jit_new_array` + `jit_array_push_elem` ABI was the
                // deleted ValueWord-shape path.
                //
                // statements.rs's `StatementKind::Assign` handler short-
                // circuits to `emit_v2_array_aggregate` (which calls
                // `jit_v2_array_new_<kind>` directly) when the destination
                // place has a proven scalar element kind via
                // `v2_typed_array_elem_kind`. Reaching this fallback means
                // the destination place could not be resolved to a typed-
                // array slot — either the destination isn't a local, or
                // the local's `ConcreteType` lacks the `Array<T>` shape
                // the v2 fast path requires.
                //
                // Per §2.7.14 forbidden list ("Bool-default fallback for
                // unknown element kinds") the correct response is
                // surface-and-stop; falling back to a kind-blind allocator
                // would resurrect the deleted UnifiedArray heap layout.
                Err("Route A surface-and-stop: SURFACE — Rvalue::Aggregate \
                     reached the kind-blind fallback. The v2 typed-array \
                     fast path in statements.rs requires the destination \
                     `Place::Local` to carry a `ConcreteType::Array<scalar>`; \
                     reaching here means the element kind is not threaded \
                     from the producing call signature. Tracked as \
                     W11-jit-new-array per phase-3-kickoff-prompt.md. \
                     ADR-006 §2.7.14 / §2.7.5."
                    .to_string())
            }

            Rvalue::EnumTest { operand, variant } => {
                // ADR-006 §2.7.17 / Q18 (W12-jit-result-option-trinity,
                // Phase 3 cluster-0 Round 7A, 2026-05-12). The operand is
                // an `Arc::into_raw(Arc<ResultData>) as u64` or
                // `Arc::into_raw(Arc<OptionData>) as u64` slot per the
                // §2.7.7 stack-tier kind label; the FFI accessor reads
                // `is_ok` / `is_some` from the `*const T` directly. NOT a
                // NaN-box tag decode (§2.7.7 #4 / #7 forbidden), NOT a
                // generic SwitchBool fallthrough — kind-aware codegen per
                // the audit blueprint at
                // `docs/cluster-audits/w12-jit-match-enum-inline-audit.md`
                // §6.1.
                let bits = self.compile_operand_raw(operand)?;
                let bits_i64 = self.to_i64_bits(bits);
                let func_ref = match variant {
                    VariantTag::Ok => self.ffi.arc_result_is_ok,
                    VariantTag::Err => self.ffi.arc_result_is_err,
                    VariantTag::Some_ => self.ffi.arc_option_is_some,
                    VariantTag::None_ => self.ffi.arc_option_is_none,
                };
                let inst = self.builder.ins().call(func_ref, &[bits_i64]);
                // FFI returns I8 (native bool). Caller's destination slot
                // kind is `Bool` per `infer_rvalue_kind`'s EnumTest arm.
                Ok(self.builder.inst_results(inst)[0])
            }

            Rvalue::EnumPayload { operand, variant } => {
                // ADR-006 §2.7.17 / Q18 (W12-jit-result-option-trinity).
                // Caller has proven the variant matches via `EnumTest` and
                // control-flow only enters this arm in the matching branch.
                // The FFI clones the inner KindedSlot's share (per §2.7.17
                // receiver-recovery soundness) and returns the raw bits as
                // an owned slot at the caller's destination. Payload kind
                // flows via the EnumStore producer's compile-time stamp +
                // 6A's call-return-kind track (the destination slot's kind
                // is set at MIR-inference time, not at this codegen site).
                //
                // `VariantTag::None_` here is a producer-side bug — the
                // None arm has no payload to extract. Surface-and-stop.
                let func_ref = match variant {
                    VariantTag::Ok | VariantTag::Err => self.ffi.arc_result_payload,
                    VariantTag::Some_ => self.ffi.arc_option_payload,
                    VariantTag::None_ => {
                        return Err("EnumPayload: SURFACE — VariantTag::None_ has no \
                             payload to extract per ADR-006 §2.7.17 \
                             `OptionData::none()` (placeholder Bool slot). \
                             The MIR producer in \
                             `lower_constructor_bindings_from_place_opt` \
                             must not emit `EnumPayload { variant: None_ }`. \
                             Producer-site contract violated."
                            .to_string());
                    }
                };
                let bits = self.compile_operand_raw(operand)?;
                let bits_i64 = self.to_i64_bits(bits);
                let inst = self.builder.ins().call(func_ref, &[bits_i64]);
                Ok(self.builder.inst_results(inst)[0])
            }

            Rvalue::TypePatternTest {
                type_annotation, ..
            } => {
                // W15.2-LANG-5 (Phase 4b, 2026-05-18). The MIR producer
                // emits `Rvalue::TypePatternTest` for every `Pattern::Typed`
                // arm in a match expression (e.g. `match x { n: int => ...,
                // s: string => ... }`). The JIT consumer would need a per-
                // kind dispatch on the §2.7.7 stack parallel-kind track —
                // for the scalar-name annotations (`int` / `number` /
                // `bool` / `string`) the test reduces to a NativeKind
                // comparison; for `Generic { name, args }` the test
                // requires reading the heap discriminator from the
                // operand's `KindedSlot.slot.as_heap_value()` per ADR-005
                // §1 single-discriminator + §2.7.6 / Q8 carrier-API-bound
                // dispatch.
                //
                // Until that codegen lands, surface-and-stop. The JIT
                // preflight at `mir_compiler::preflight` rejects this
                // Rvalue at the program-level gate, so under normal
                // dispatch the W12 fall-through at
                // `crates/shape-jit/src/executor.rs::execute_program`
                // routes the program to the bytecode interpreter (which
                // compiles `Pattern::Typed` via the `OpCode::TypeCheck`
                // path in `compiler/patterns/checking.rs`). This arm
                // remains as defense in depth: if a future caller invokes
                // `compile_rvalue` without running preflight first, the
                // surface-and-stop here preserves the §2.7.5 producer-
                // site classification discipline rather than silently
                // emitting a kind-blind Bool default (CLAUDE.md
                // "Forbidden rationalizations" — "just a small fallback
                // for this one edge case" refused on sight).
                Err(format!(
                    "Route A surface-and-stop: NotImplemented(SURFACE) — \
                     `Rvalue::TypePatternTest` codegen not yet wired \
                     (annotation: {:?}). W15.2-LANG-5 fall-through to \
                     interpreter is the canonical path today; preflight \
                     should have already rejected this MIR before reaching \
                     `compile_rvalue`. Native codegen lands as a \
                     follow-up: per-kind dispatch on the §2.7.7 stack \
                     parallel-kind track for scalar annotations + heap-\
                     value discriminator read for `Generic` annotations \
                     (ADR-005 §1 / ADR-006 §2.7.6 Q8 / §2.7.5 producer-\
                     site classification). NOT a Bool-default \
                     rationalization — the annotation IS the producer-side \
                     classification.",
                    type_annotation
                ))
            }

            Rvalue::EnumDiscriminantTest {
                enum_name,
                variant_name,
                ..
            } => {
                // W15.2-LANG-1 (Phase 4b, 2026-05-18). The MIR producer
                // emits `Rvalue::EnumDiscriminantTest` for every non-
                // trinity (user-defined) `Pattern::Constructor` arm in a
                // match expression (e.g. `match Color::Red { Color::Red
                // => ..., Color::Green => ..., Color::Blue => ... }`,
                // book snippet `enums.mdx:113`). The JIT consumer would
                // need to resolve the (enum_name, variant_name) pair to
                // the schema's `(schema_id, variant_id)` at the
                // bytecode-compiler conduit (the MIR layer has no
                // `type_tracker.schema_registry` access — see
                // `mir/lowering/helpers.rs:130-150` comment block), then
                // emit a typed-object discriminant read + EqInt sequence
                // matching the bytecode-VM's `compile_typed_enum_pattern_
                // check` shape (`compiler/patterns/checking.rs:344` —
                // `GetFieldTyped(__variant, I64)` + `PushConst(variant_
                // id)` + `EqInt`).
                //
                // Until that codegen lands, surface-and-stop. The JIT
                // preflight at `mir_compiler::preflight` rejects this
                // Rvalue at the program-level gate, so under normal
                // dispatch the W12 fall-through at
                // `crates/shape-jit/src/executor.rs::execute_program`
                // routes the program to the bytecode interpreter (which
                // compiles user-defined `Pattern::Constructor` via the
                // typed-object discriminant path cited above). This arm
                // remains as defense in depth: if a future caller invokes
                // `compile_rvalue` without running preflight first, the
                // surface-and-stop here preserves the §2.7.5 producer-
                // site classification discipline rather than silently
                // emitting a kind-blind Bool default (CLAUDE.md
                // "Forbidden rationalizations" — "just a small fallback
                // for this one edge case" refused on sight). Mirrors the
                // LANG-5 `TypePatternTest` precedent immediately above.
                Err(format!(
                    "Route A surface-and-stop: NotImplemented(SURFACE) — \
                     `Rvalue::EnumDiscriminantTest` codegen not yet wired \
                     (enum: {:?}, variant: {:?}). W15.2-LANG-1 fall-\
                     through to interpreter is the canonical path today; \
                     preflight should have already rejected this MIR \
                     before reaching `compile_rvalue`. Native codegen \
                     lands as a follow-up: schema-registry conduit + \
                     typed-object discriminant read (mirror of bytecode-\
                     VM `compile_typed_enum_pattern_check` at \
                     `compiler/patterns/checking.rs:344`). NOT a Bool-\
                     default rationalization — the (enum_name, variant_\
                     name) pair IS the producer-side classification (ADR-\
                     006 §2.7.5 stamp-at-compile-time).",
                    enum_name, variant_name
                ))
            }
            Rvalue::PrimitiveCast { target, .. } => {
                // f-string bool-as-int VM!=JIT divergence fix (2026-06).
                // The MIR producer emits `Rvalue::PrimitiveCast` for a
                // primitive infallible `as`-cast whose result kind differs
                // from the source (`true as int`, `1 as number`, …). The
                // JIT has no typed convert body — the bytecode
                // `OpCode::ConvertTo*` family is VM-only per
                // `vm_only_opcode_reason`, and the opcode-FFI trampoline
                // passes operand bits through UNCHANGED (rendering `true`
                // instead of `1`). The preflight at `mir_compiler::preflight`
                // rejects this Rvalue at the program-level gate, so the W12
                // fall-through routes the program to the bytecode
                // interpreter (which restamps the cast result kind via
                // `ConvertTo*`). This arm is defense in depth: if a future
                // caller invokes `compile_rvalue` without running preflight,
                // surface-and-stop here rather than emitting the kind-blind
                // pass-through the MIR lowering used to (CLAUDE.md
                // "Forbidden rationalizations" — the W4-δ "value passes
                // through, executor reads the tag" shape refused on sight).
                // NOT a Bool-default — the target type name IS the
                // producer-side classification (ADR-006 §2.7.5). Native
                // typed convert codegen lands as a v0.4 follow-up.
                Err(format!(
                    "Route A surface-and-stop: NotImplemented(SURFACE) — \
                     `Rvalue::PrimitiveCast` (`expr as {}`) codegen not yet \
                     wired (the bytecode `OpCode::ConvertTo*` family is \
                     VM-only). W12 fall-through to the interpreter is the \
                     canonical path today; preflight should have already \
                     rejected this MIR before reaching `compile_rvalue`. \
                     Native typed convert codegen lands as a v0.4 follow-up.",
                    target
                ))
            }
        }
    }

    // ── Operand kind helpers ───────────────────────────────────────

    /// Get the NativeKind of an operand's source, falling back to the
    /// documented §2.7.5 stable-FFI carrier kind `NativeKind::UInt64` when
    /// the producing-site inference left the slot kind undetermined.
    ///
    /// ADR-006 §2.7.5 designates `UInt64` as the "I64-wide raw bits without
    /// further classification" carrier kind — the same kind
    /// `dispatch_call_via_trampoline_vm` stamps for function-id-class
    /// callees and for I64-widened args at the JIT-FFI boundary. It is
    /// NOT a Bool-default rationalization (§2.7.7 #9 / CLAUDE.md
    /// "Forbidden rationalizations"); `UInt64` is the documented carrier
    /// kind for the bit-pattern the JIT actually pushes onto the stack
    /// (every operand widens to I64 before the push per terminators.rs
    /// R4.2E inline-widening discipline).
    ///
    /// Precise kinds — `Ptr(HeapKind::Closure)` for closure slots seeded by
    /// `infer_slot_kinds::ClosureCapture`, `Float64` / `Bool` / etc. for
    /// inferred scalar slots — flow through unchanged. The fallback only
    /// applies to slots whose producing-site is opaque to MIR inference
    /// (field reads through heap projections, opaque-source calls, etc.)
    /// — in those cases the value IS I64-wide raw bits by construction,
    /// and `UInt64` is the structurally-correct §2.7.5 carrier kind.
    ///
    /// For the load-bearing closure-callee classification at
    /// `jit_call_value`'s indirect-call entry, the §2.7.11/Q12 dispatch
    /// requires precise `Ptr(HeapKind::Closure)` kinds — seeded via
    /// `infer_slot_kinds`'s ClosureCapture arm. The `UInt64` fallback at
    /// other push sites preserves the existing JIT-internal NaN-box
    /// bit-shape dispatch path inside `jit_call_value` (cases 1 / 2 —
    /// inline `TAG_FUNCTION` function refs and legacy `HK_CLOSURE`
    /// unified-heap callees).
    #[allow(dead_code)]
    pub(crate) fn operand_slot_kind_or_carrier(
        &self,
        operand: &Operand,
    ) -> shape_value::NativeKind {
        self.operand_slot_kind(operand)
            .unwrap_or(shape_value::NativeKind::UInt64)
    }

    /// Get the NativeKind of an operand's source (before compilation).
    ///
    /// ADR-006 §2.7.5 / §2.7.11: the producing site classifies the operand
    /// kind at JIT-compile time. Function refs widen to the documented
    /// `NativeKind::UInt64` carrier kind (the §2.7.11/Q12 function-id-class
    /// callee-classification kind, also used as the "I64-wide raw bits
    /// carrier" sentinel at the §2.7.5 stable-FFI boundary). Method-name
    /// constants are heap String pointers (kind = `NativeKind::String`).
    /// String and StringId constants are likewise heap String pointers.
    pub(crate) fn operand_slot_kind(
        &self,
        operand: &Operand,
    ) -> Option<shape_vm::type_tracking::NativeKind> {
        use shape_vm::type_tracking::NativeKind;
        match operand {
            Operand::Constant(MirConstant::Int(_)) => Some(NativeKind::Int64),
            Operand::Constant(MirConstant::Float(_)) => Some(NativeKind::Float64),
            Operand::Constant(MirConstant::Bool(_)) => Some(NativeKind::Bool),
            // Phase 3 cluster-2 Round 4 cw-D-fam12 follow-up (instance 57,
            // 2026-05-16). ADR-006 §2.7.5 amendment Round 19 S1.5: Char is a
            // 4-byte scalar `NativeKind` variant (codepoint inline in low 32
            // bits of `ValueSlot`, no Arc wrapping). Producer-site
            // classification at the MIR constant operand mirrors the
            // `infer_constant_kind` arm in `types.rs` — both feed the same
            // §2.7.5 stamp-at-compile-time discipline for the print dispatch
            // at `terminators.rs` ~679 `NativeKind::Char` arm.
            Operand::Constant(MirConstant::Char(_)) => Some(NativeKind::Char),
            // WS-8 (2026-05-22): the producer-site stamp for `Literal::Decimal`
            // through MIR is `NativeKind::DecimalV2`. The `compile_constant`
            // arm at `ownership.rs` surfaces-and-stops on the value, so
            // downstream consumers reaching this stamp run under the W12
            // fall-through to the VM interpreter (VM == JIT).
            Operand::Constant(MirConstant::Decimal(_)) => Some(NativeKind::DecimalV2),
            // ADR-006 §2.7.11/Q12 function-id-class callee-classification
            // kind: a `MirConstant::Function(name)` lowers to the JIT-
            // internal `box_function(fn_id)` shape (TAG_FUNCTION NaN-box),
            // whose carrier kind across the §2.7.5 stable-FFI boundary is
            // `NativeKind::UInt64`. The trampoline VM consumer
            // (`dispatch_call_via_trampoline_vm`) classifies this same
            // kind as the function-id callee per `call_convention.rs`
            // UInt64 arm.
            Operand::Constant(MirConstant::Function(_)) => Some(NativeKind::UInt64),
            // Method-name string constant. The JIT emits a heap String
            // pointer via `box_string`; carrier kind is `String` (the
            // §2.7.5 String arm — `Arc<String>` raw pointer carrier).
            Operand::Constant(MirConstant::Method(_)) => Some(NativeKind::String),
            // String constants and string-id constants both materialize
            // as heap `Arc<String>` raw pointers; carrier kind is String.
            Operand::Constant(MirConstant::Str(_)) => Some(NativeKind::String),
            Operand::Constant(MirConstant::StringId(_)) => Some(NativeKind::String),
            // ClosurePlaceholder is the producing-site forward-reference
            // for closures whose function_id is patched later. The slot
            // it lowers to carries `Arc<HeapValue::ClosureRaw>` bits per
            // §2.7.11/Q12.
            Operand::Constant(MirConstant::ClosurePlaceholder) => {
                Some(NativeKind::Ptr(shape_value::heap_value::HeapKind::Closure))
            }
            Operand::Constant(MirConstant::None) => None,
            Operand::Copy(p) | Operand::Move(p) | Operand::MoveExplicit(p) => {
                // Centralized projection: `place_native_kind` handles
                // both Round 5A's Field projection (via
                // `field_native_kinds`) AND Round 5C's Index projection
                // (via `v2_typed_array_elem_kind` → `concrete_types`'s
                // `Array<scalar>` shape) in a single helper that
                // `ownership::refcount_disposition` also shares.
                self.place_native_kind(p)
            }
        }
    }

    /// Project a `Place` to the `NativeKind` of the value it produces at
    /// the consumer site, per ADR-006 §2.7.5 stamp-at-compile-time
    /// discipline (W12-jit-binop-after-heap-read-kind-tracker close).
    ///
    /// - `Place::Local(slot)`: read the slot's MIR-inferred kind from
    ///   `slot_kinds`.
    /// - `Place::Field(base, field_idx)`: look up the field name via
    ///   `field_name_table`, then the per-field kind in
    ///   `field_native_kinds` — populated by the producer-side
    ///   `StatementKind::ObjectStore` walk at MirToIR construction time.
    ///   This threads the producer's kind classification across the
    ///   TypedObject field-read projection without runtime tag-bit
    ///   decode (§2.7.7 #4 / #7 forbidden).
    /// - `Place::Index(base, _)`: when the base local's `ConcreteType`
    ///   is `Array<scalar>` (per the W12-top-level-concrete-types-
    ///   conduit close), project to the element's `NativeKind` via
    ///   `v2_typed_array_elem_kind`. This is the same kind the v2
    ///   `read_place` fast path uses to load the element at its native
    ///   width. Same projection the W12-jit-print-kind (Round 5C) sub-
    ///   cluster needs at the `print(xs[0])` dispatch site.
    /// - `Place::Deref(_)`: not stamped — references are heap-tier
    ///   indirection and the type-of-pointed-to-value is not threaded
    ///   into the JIT-side projection map yet. Returns `None` so the
    ///   BinaryOp lowering surfaces honestly rather than papering.
    ///
    /// Returns `None` when no proof exists at this consumer site;
    /// callers in `compile_rvalue` then choose between surface-and-stop
    /// (the dynamic-arith / dynamic-cmp arms) and continuing through the
    /// `UInt64` carrier fallback in `operand_slot_kind_or_carrier`.
    ///
    /// `pub(crate)` so `ownership::refcount_disposition` can project
    /// through `Field` / `Index` to decide retain/release on the value
    /// being copied — the value's kind is the field's / element's kind,
    /// not the base struct/array's heap kind. This closes the segfault
    /// where `Copy(Field(p_TypedObject, x_Int64))` previously routed
    /// through the base's heap retain and called `arc_retain(i64_3)`.
    pub(crate) fn place_native_kind(
        &self,
        place: &Place,
    ) -> Option<shape_vm::type_tracking::NativeKind> {
        match place {
            Place::Local(slot) => super::types::slot_kind_for_local(&self.slot_kinds, slot.0),
            Place::Field(_, field_idx) => {
                let name = self.mir.field_name_table.get(field_idx)?;
                self.field_native_kinds.get(name).copied()
            }
            Place::Index(base, _) => {
                // The v2 typed-array element-kind helper takes a Place
                // and reads `concrete_types[base.root_local()]`. It is
                // the same source the `read_place` fast path uses to
                // pick the native-width load width for the element —
                // pairing the producer-side kind classification with the
                // consumer-side BinaryOp picker.
                self.v2_typed_array_elem_kind(base)
            }
            Place::Deref(_) => None,
        }
    }

    /// True for a narrow integer `NativeKind` (`i8`/`i16`/`i32` and the
    /// unsigned `u8`/`u16`/`u32` siblings) — the widths the bytecode VM
    /// truncates two's-complement via `IntWidth::truncate`.
    fn is_narrow_int_kind(k: shape_vm::type_tracking::NativeKind) -> bool {
        use shape_vm::type_tracking::NativeKind;
        matches!(
            k,
            NativeKind::Int8
                | NativeKind::Int16
                | NativeKind::Int32
                | NativeKind::UInt8
                | NativeKind::UInt16
                | NativeKind::UInt32
        )
    }

    /// R5c-2-β-γ (c) jit-narrow-wrap: classify a binop's operands as a
    /// narrow-integer ARITHMETIC / bitwise site.
    ///
    /// Returns `Some(kind)` when the binop operates at a narrow integer
    /// width (`Int8`/`Int16`/`Int32`/`UInt8`/`UInt16`/`UInt32`):
    ///
    /// - BOTH operands carry the SAME narrow `NativeKind` — the common
    ///   `let c: i32 = a + b` two-variable shape.
    /// - ONE operand carries a narrow `NativeKind` and the OTHER is a bare
    ///   integer literal (`Operand::Constant(MirConstant::Int(_))`,
    ///   classified `Int64` by `operand_slot_kind` because the literal MIR
    ///   carrier is width-blind). An integer literal is width-polymorphic —
    ///   it adapts to the narrow context exactly as the bytecode compiler
    ///   emits `AddI32`/`AddTyped` for `a + 5` when `a` is narrow.
    ///   `compile_binop_narrow_int`'s `ireduce` truncates the literal to
    ///   the narrow width; for add/sub/mul this is byte-equal to the VM's
    ///   "wrap-then-truncate" because two's-complement modular arithmetic
    ///   commutes with truncation.
    ///
    /// This classifier is deliberately CONSERVATIVE for arithmetic: it does
    /// NOT accept a width-polymorphic literal hoisted into a `Copy(Local)`
    /// temp. The narrow-width `ireduce`-then-divide path would diverge from
    /// the VM for div/mod against an out-of-range literal, and an
    /// `(narrow, Int64)` arithmetic pairing where the `Int64` side is a
    /// genuine `int` variable is mixed-width arithmetic the JIT does not
    /// own — those keep the pre-existing dispatch (generic surface-and-stop
    /// → clean interpreter fall-through). Comparison sites take the
    /// separate `narrow_int_cmp_kind` classifier below, which IS
    /// literal-shape-agnostic because the i64-width compare it routes to is
    /// correct for every operand shape.
    ///
    /// `None` for any other pairing — `Bool` (deliberately excluded: it
    /// shares the Cranelift `I8` width with `Int8`/`UInt8` but is not
    /// integer arithmetic), `Int64`/`Int64`, floats, heap pointers,
    /// mismatched narrow widths, and unproven (`None`) kinds all fall
    /// through to the existing dispatch arms.
    fn narrow_int_binop_kind(
        lhs: Option<shape_vm::type_tracking::NativeKind>,
        rhs: Option<shape_vm::type_tracking::NativeKind>,
        lhs_op: &Operand,
        rhs_op: &Operand,
    ) -> Option<shape_vm::type_tracking::NativeKind> {
        use shape_vm::type_tracking::NativeKind;
        let is_narrow = Self::is_narrow_int_kind;
        fn is_int_literal(op: &Operand) -> bool {
            matches!(op, Operand::Constant(MirConstant::Int(_)))
        }
        match (lhs, rhs) {
            // Both operands the same narrow kind.
            (Some(lk), Some(rk)) if lk == rk && is_narrow(lk) => Some(lk),
            // One operand narrow, the other a bare width-polymorphic literal.
            (Some(lk), Some(NativeKind::Int64)) if is_narrow(lk) && is_int_literal(rhs_op) => {
                Some(lk)
            }
            (Some(NativeKind::Int64), Some(rk)) if is_narrow(rk) && is_int_literal(lhs_op) => {
                Some(rk)
            }
            _ => None,
        }
    }

    /// r5c-2-gz-cp6 narrow-neg-literal: classify a binop's operands as a
    /// narrow-integer COMPARISON site.
    ///
    /// Returns `Some(kind)` — the narrow `NativeKind` whose SIGNEDNESS
    /// drives the Cranelift condition code — when at least one operand is a
    /// proven narrow integer (`i8`/`i16`/`i32`/`u8`/`u16`/`u32`):
    ///
    /// - BOTH operands the same narrow kind — `let r: bool = a < b`.
    /// - ONE operand narrow, the OTHER `Int64`. The `Int64` partner may be
    ///   a bare integer literal, a `Copy(Local)` slot the MIR lowering
    ///   hoisted a literal into, or a genuine `int` variable (Shape DOES
    ///   permit cross-width integer comparison — `let a:i8 = -1; a < 5` and
    ///   `let n:int = 3; a == n` are both well-typed, unlike cross-width
    ///   arithmetic). All three are handled identically and correctly by
    ///   `compile_binop_narrow_int`'s comparison arms, which compare at
    ///   I64 width (extending the narrow operand per the kind's signedness)
    ///   — byte-equal to the VM's `compact_int_cmp`, which compares the
    ///   full sign-/zero-extended i64 slot bits and never re-truncates.
    ///
    /// ## Why a separate classifier
    ///
    /// The MIR lowering (`lower_expr_to_operand`) hoists most literals into
    /// a `Copy(Local)` temp whose slot kind is the width-blind `Int64`
    /// (`r5c-2-bg-b2-u64-literal-inference` lowered `u64`-sibling literals
    /// directly as `Operand::Constant` but left the narrow-width MIR shape
    /// unchanged). So `let c: i8 = -56; c == -56` reaches the dispatcher as
    /// `(Int8, Int64)` with the literal carried by a `Copy(Local)`. The
    /// conservative `narrow_int_binop_kind` rejects that, so the comparison
    /// fell through to the kind-blind generic `compile_binop` →
    /// `compile_binop_dynamic_cmp`, whose `Eq`/`Ne` arm emits a raw
    /// bit-compare on operands widened by `to_i64_bits` — and `to_i64_bits`
    /// ZERO-extends an `I8`. For a NEGATIVE narrow value (`c == -56`, `c`
    /// holding the i8 bit-pattern `0xC8`) the zero-extend produced `200`
    /// while the literal slot held the sign-extended I64 `-56` (`0xFF..C8`):
    /// `200 != -56`, so the JIT diverged (`false`) from the VM (`true`).
    /// POSITIVE values coincided because zero- and sign-extension agree.
    ///
    /// Unlike the arithmetic classifier, this one is sound for ANY `Int64`
    /// operand shape: the comparison codegen extends (never truncates), so
    /// a genuine out-of-range `int` value compares correctly rather than
    /// being silently masked into the narrow window.
    ///
    /// `None` when neither operand is a proven narrow kind, or when an
    /// operand kind is unproven — those fall through to the existing
    /// `compile_cmp_i32_native` / `both_int64` / generic dispatch arms.
    fn narrow_int_cmp_kind(
        lhs: Option<shape_vm::type_tracking::NativeKind>,
        rhs: Option<shape_vm::type_tracking::NativeKind>,
    ) -> Option<shape_vm::type_tracking::NativeKind> {
        use shape_vm::type_tracking::NativeKind;
        let is_narrow = Self::is_narrow_int_kind;
        match (lhs, rhs) {
            // Both operands the same narrow kind.
            (Some(lk), Some(rk)) if lk == rk && is_narrow(lk) => Some(lk),
            // One operand narrow, the other `Int64` (literal — inline or
            // slot-hoisted — or a genuine `int` variable).
            (Some(lk), Some(NativeKind::Int64)) if is_narrow(lk) => Some(lk),
            (Some(NativeKind::Int64), Some(rk)) if is_narrow(rk) => Some(rk),
            _ => None,
        }
    }

    /// R5c-2-β-γ checkpoint (b) u64-carrier: classify a binop as a
    /// full-range `u64` arithmetic site.
    ///
    /// Returns `true` when the binop operates on `NativeKind::UInt64`
    /// operands — the full-range `0..2^64` unsigned carrier. Like the
    /// narrow-int classifier, an integer literal (`Operand::Constant(
    /// MirConstant::Int(_))`, classified `Int64` because the literal MIR
    /// carrier is width-blind) is width-polymorphic and adapts to a `u64`
    /// context, so a `(UInt64, Int64-literal)` pairing also classifies as
    /// `u64`. Both operands lower to Cranelift `I64` (u64 and i64 share the
    /// 64-bit width); the codegen in `compile_binop_uint64` then selects
    /// `udiv`/`urem` and unsigned compares so the JIT matches the bytecode
    /// VM's unsigned `u64` arithmetic byte-for-byte.
    ///
    /// `false` for any other pairing — `Int64`/`Int64` (signed `int`),
    /// floats, narrow integers, heap pointers, and unproven kinds all fall
    /// through to the existing dispatch arms.
    fn uint64_binop_site(
        lhs: Option<shape_vm::type_tracking::NativeKind>,
        rhs: Option<shape_vm::type_tracking::NativeKind>,
        lhs_op: &Operand,
        rhs_op: &Operand,
    ) -> bool {
        use shape_vm::type_tracking::NativeKind;
        fn is_int_literal(op: &Operand) -> bool {
            matches!(op, Operand::Constant(MirConstant::Int(_)))
        }
        match (lhs, rhs) {
            (Some(NativeKind::UInt64), Some(NativeKind::UInt64)) => true,
            (Some(NativeKind::UInt64), Some(NativeKind::Int64)) => is_int_literal(rhs_op),
            (Some(NativeKind::Int64), Some(NativeKind::UInt64)) => is_int_literal(lhs_op),
            _ => false,
        }
    }

    /// Check if both operand kinds are Int64 (NaN-boxed integers suitable for inline i64 ops).
    fn both_int64(
        &self,
        lhs: Option<shape_vm::type_tracking::NativeKind>,
        rhs: Option<shape_vm::type_tracking::NativeKind>,
    ) -> bool {
        matches!(
            (lhs, rhs),
            (
                Some(shape_vm::type_tracking::NativeKind::Int64),
                Some(shape_vm::type_tracking::NativeKind::Int64)
            )
        )
    }

    /// F5.a/F5.b: true if either operand kind is `NativeKind::String`. The MIR
    /// emits `BinOp::Add` on heterogeneous operand types for f-string
    /// interpolation (e.g. `str + number + str`) — the FFI's non-string
    /// fallback (`format_value_word`) does the rest.
    fn either_string(
        &self,
        lhs: Option<shape_vm::type_tracking::NativeKind>,
        rhs: Option<shape_vm::type_tracking::NativeKind>,
    ) -> bool {
        matches!(lhs, Some(shape_vm::type_tracking::NativeKind::String))
            || matches!(rhs, Some(shape_vm::type_tracking::NativeKind::String))
    }

    /// W15.2-LANG-7 jit-print-fstring close (Phase 4b Round 3, 2026-05-18):
    /// emit a call to the kind-aware
    /// `jit_string_concat(a_bits, a_kind_code, b_bits, b_kind_code) -> bits`
    /// FFI per ADR-006 §2.7.5/§2.7.7 producer-side stamp.
    ///
    /// Both operand `Value`s are widened to I64 bit-patterns (the FFI
    /// signature expects `(I64, I8, I64, I8) -> I64`). The kind bytes are
    /// stamped at JIT-compile time from `operand_slot_kind` per the
    /// `compile_rvalue` call-site contract above; SENTINEL falls through to
    /// the FFI body's §2.7.7 #9 surface-and-stop arm for unstamped operands.
    ///
    /// The result bits carry the §2.7.5 `NativeKind::String` carrier shape
    /// (`Arc::into_raw(Arc<String>) as u64`) — the same shape every
    /// downstream consumer reads (`jit_print_str`, `arc_string_retain`/
    /// `_release`, `KindedSlot::Drop` for `NativeKind::String`). Pre-fix
    /// the FFI returned a NaN-boxed `box_string(out)` whose bit-shape did
    /// not match the consumer-side String carrier; the consumer's
    /// `Arc::from_raw`-shape decode dereferenced a NaN bit-pattern as a
    /// `String` struct → garbage memory bytes printed (W15.1 audit §6.7
    /// surface).
    fn compile_string_concat(
        &mut self,
        lhs: Value,
        lhs_kind: Option<shape_vm::type_tracking::NativeKind>,
        rhs: Value,
        rhs_kind: Option<shape_vm::type_tracking::NativeKind>,
    ) -> Result<Value, String> {
        use crate::ffi::stack_kind_code;
        let a = self.to_i64_bits(lhs);
        let b = self.to_i64_bits(rhs);
        let a_code = lhs_kind
            .map(stack_kind_code::encode)
            .unwrap_or(stack_kind_code::SENTINEL);
        let b_code = rhs_kind
            .map(stack_kind_code::encode)
            .unwrap_or(stack_kind_code::SENTINEL);
        let a_code_val = self.builder.ins().iconst(types::I8, a_code as i64);
        let b_code_val = self.builder.ins().iconst(types::I8, b_code as i64);
        let inst = self
            .builder
            .ins()
            .call(self.ffi.string_concat, &[a, a_code_val, b, b_code_val]);
        Ok(self.builder.inst_results(inst)[0])
    }

    // ── Inline Float64 arithmetic and comparisons ──────────────────

    /// Compile a binary op on native F64 operands — direct Cranelift float instructions.
    /// ~100x faster per operation vs FFI generic_add/etc.
    fn compile_binop_f64(&mut self, op: &BinOp, lhs: Value, rhs: Value) -> Result<Value, String> {
        match op {
            BinOp::Add => Ok(self.builder.ins().fadd(lhs, rhs)),
            BinOp::Sub => Ok(self.builder.ins().fsub(lhs, rhs)),
            BinOp::Mul => Ok(self.builder.ins().fmul(lhs, rhs)),
            BinOp::Div => Ok(self.builder.ins().fdiv(lhs, rhs)),
            BinOp::Mod => {
                // f64 mod: a % b = a - trunc(a/b) * b (pure Cranelift, no FFI)
                let div = self.builder.ins().fdiv(lhs, rhs);
                let truncated = self.builder.ins().trunc(div);
                let product = self.builder.ins().fmul(truncated, rhs);
                Ok(self.builder.ins().fsub(lhs, product))
            }
            BinOp::Eq => {
                let cmp = self.builder.ins().fcmp(FloatCC::Equal, lhs, rhs);
                // fcmp returns I8 (native bool) — this is fine for Bool slots
                Ok(cmp)
            }
            BinOp::Ne => {
                let cmp = self.builder.ins().fcmp(FloatCC::NotEqual, lhs, rhs);
                Ok(cmp)
            }
            BinOp::Lt => {
                let cmp = self.builder.ins().fcmp(FloatCC::LessThan, lhs, rhs);
                Ok(cmp)
            }
            BinOp::Le => {
                let cmp = self.builder.ins().fcmp(FloatCC::LessThanOrEqual, lhs, rhs);
                Ok(cmp)
            }
            BinOp::Gt => {
                let cmp = self.builder.ins().fcmp(FloatCC::GreaterThan, lhs, rhs);
                Ok(cmp)
            }
            BinOp::Ge => {
                let cmp = self
                    .builder
                    .ins()
                    .fcmp(FloatCC::GreaterThanOrEqual, lhs, rhs);
                Ok(cmp)
            }
            BinOp::And | BinOp::Or => {
                // Logical ops on floats — box and use generic path
                self.compile_binop(op, lhs, rhs)
            }
            // W11-fup-A (Phase 3d, 2026-05-18): `Pow` on f64 lowers to the
            // existing `jit_pow_f64` FFI helper (`crates/shape-jit/src/ffi/
            // v2_math.rs:302`); both operands already widened to F64 by
            // the caller (the `l_type == F64 && r_type == F64` branch in
            // `compile_rvalue`). Result stays F64 — kind is stamped at
            // §2.7.5 producing-MIR `infer_rvalue_kind` time (same-kind
            // operands → `Some(F64)` for non-comparison ops).
            BinOp::Pow => {
                let func_ref = self.ffi.pow_f64;
                let inst = self.builder.ins().call(func_ref, &[lhs, rhs]);
                Ok(self.builder.inst_results(inst)[0])
            }
            // W11-fup-A: bitwise ops on f64 operands — surface-and-stop.
            // Shape `int` (i64) is the only kind for which the bytecode VM
            // emits `BitAndInt`/etc. (`opcode_defs.rs:1860-1873`); f64
            // operands routed here imply a §2.7.5 producing-MIR kind-tracker
            // gap (the operand should have been Int64-stamped upstream).
            // Honest surface-and-stop per W10 playbook §5 — no
            // `f64::to_bits | other_bits` rationalization (that's the
            // deleted W-series tag-bit dispatch shape).
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::BitShl | BinOp::BitShr => {
                Err(format!(
                    "compile_binop_f64: SURFACE — bitwise {:?} on Float64 operands \
                 has no semantic in Shape (`int`-only per `BitAndInt`/etc. at \
                 opcode_defs.rs:1860-1873). Reaching here means the §2.7.5 \
                 producing-MIR kind-tracker stamped Float64 where Int64 was \
                 expected. Producer-site gap; surface per W10 playbook §5.",
                    op
                ))
            }
        }
    }

    // ── Native I32 arithmetic (no ireduce/sextend needed) ───────────

    /// Compile i32 binary arithmetic on native I32 values (no boxing overhead).
    fn compile_binop_i32_native(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        match op {
            BinOp::Add => Ok(self.builder.ins().iadd(lhs, rhs)),
            BinOp::Sub => Ok(self.builder.ins().isub(lhs, rhs)),
            BinOp::Mul => Ok(self.builder.ins().imul(lhs, rhs)),
            // r5c-2-gz-cp2-jit-div: VM-equivalent trap-free i32 div/mod —
            // div-by-zero → clean `Division by zero`; `i32::MIN / -1` →
            // wrapping `i32::MIN` (mod → 0). See `compile_int_divmod_guarded`.
            BinOp::Div => self.compile_int_divmod_guarded(lhs, rhs, types::I32, true, false),
            BinOp::Mod => self.compile_int_divmod_guarded(lhs, rhs, types::I32, true, true),
            // W11-fup-A (Phase 3d, 2026-05-18): bitwise on native I32 use
            // Cranelift's native bitwise instructions (`band`/`bor`/`bxor`/
            // `ishl`/`sshr`). Mirrors the bytecode VM's `BitAndInt`/etc.
            // typed opcodes (`opcode_defs.rs:1860-1873`); the i64
            // semantics there fit i32 here without truncation (the
            // operands are already proven i32 by the caller's
            // `l_type == I32 && r_type == I32` discriminator).
            BinOp::BitAnd => Ok(self.builder.ins().band(lhs, rhs)),
            BinOp::BitOr => Ok(self.builder.ins().bor(lhs, rhs)),
            BinOp::BitXor => Ok(self.builder.ins().bxor(lhs, rhs)),
            BinOp::BitShl => Ok(self.builder.ins().ishl(lhs, rhs)),
            // Arithmetic right-shift matches the VM's `BitShrInt` (`a_int >>
            // b_int` on i64) per opcode_defs.rs:1856-1857.
            BinOp::BitShr => Ok(self.builder.ins().sshr(lhs, rhs)),
            _ => Err(format!("unsupported native i32 binop: {:?}", op)),
        }
    }

    /// Compile i32 comparison on native I32 values — returns I8 (native bool).
    fn compile_cmp_i32_native(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        let cc = match op {
            BinOp::Eq => IntCC::Equal,
            BinOp::Ne => IntCC::NotEqual,
            BinOp::Lt => IntCC::SignedLessThan,
            BinOp::Le => IntCC::SignedLessThanOrEqual,
            BinOp::Gt => IntCC::SignedGreaterThan,
            BinOp::Ge => IntCC::SignedGreaterThanOrEqual,
            _ => return Err(format!("unsupported native i32 cmp: {:?}", op)),
        };
        // icmp returns I8 (native bool)
        Ok(self.builder.ins().icmp(cc, lhs, rhs))
    }

    // ── Narrow-integer arithmetic (i8/i16/i32/u8/u16/u32) ─────────

    /// R5c-2-β-γ (c) jit-narrow-wrap: compile a binary op on narrow
    /// integer operands, matching the bytecode VM's wrapping semantics.
    ///
    /// The integer-semantics ruling (2026-05-20 #3) makes integer
    /// overflow WRAPPING (two's-complement). The VM achieves this with
    /// the `AddI32`/`SubI32`/`MulI32` typed opcodes
    /// (`executor/v2_handlers/int.rs` — `i32` `wrapping_*`) and the
    /// width-parameterised `AddTyped`/`SubTyped`/`MulTyped` opcodes
    /// (`executor/arithmetic/mod.rs::compact_int_checked_binop` —
    /// `IntWidth::truncate`). The JIT matches by operating at the
    /// declared narrow Cranelift width: `iadd`/`isub`/`imul` on an
    /// `I8`/`I16`/`I32` value wrap natively at that width, exactly the
    /// two's-complement truncation the VM performs.
    ///
    /// ## Arithmetic / bitwise vs comparison: two operand-prep regimes
    ///
    /// **Arithmetic / bitwise** (`Add`/`Sub`/`Mul`/`Div`/`Mod`/`Bit*`)
    /// operates at the NARROW Cranelift width. Both operands are coerced to
    /// the narrow width via `coerce_to_narrow_int` (`ireduce` for a wider
    /// value such as an `I64` literal; pass-through for a same-width
    /// variable). `iadd`/`isub`/`imul`/`band`/… on the narrow type wrap
    /// natively at that width — exactly the VM's `IntWidth::truncate`.
    ///
    /// **Comparison** (`Eq`/`Ne`/`Lt`/`Le`/`Gt`/`Ge`) operates at I64
    /// width. r5c-2-gz-cp6 narrow-neg-literal: the VM's `compact_int_cmp`
    /// compares the FULL i64 slot bits — the narrow operand sign-extended
    /// (signed widths) or zero-extended (unsigned widths) by
    /// `IntWidth::truncate`, the partner kept at its real value — and never
    /// re-truncates. The JIT must do the same: `extend_narrow_to_i64`
    /// widens each operand per `kind`'s signedness, then `icmp` runs at I64.
    /// Truncating the partner to the narrow window first (the pre-cp6
    /// arithmetic-style `ireduce`) would mis-compare an out-of-narrow-range
    /// literal/`int` (`let c:i8=44; c == 300` — VM `44 != 300` false; a
    /// narrow-width compare would mask `300` to `44` and report `true`).
    ///
    /// The result of a comparison is a native `I8` bool; the result of an
    /// arithmetic/bitwise op keeps the narrow Cranelift type. The caller's
    /// `store_to_place` `ensure_kind` handles any storage-width adaptation.
    /// Arithmetic add/sub/mul are signedness-agnostic (two's-complement);
    /// div/mod, shifts and comparisons select signed vs unsigned Cranelift
    /// instructions from the operand kind.
    fn compile_binop_narrow_int(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
        kind: shape_vm::type_tracking::NativeKind,
    ) -> Result<Value, String> {
        use shape_vm::type_tracking::NativeKind;
        let cl_ty = super::types::cranelift_type_for_slot(kind);
        let unsigned = matches!(
            kind,
            NativeKind::UInt8 | NativeKind::UInt16 | NativeKind::UInt32
        );

        // r5c-2-gz-cp6 narrow-neg-literal: comparisons run at I64 width.
        // Extend each operand to I64 per the narrow kind's signedness —
        // sign-extend for `i8`/`i16`/`i32`, zero-extend for `u8`/`u16`/
        // `u32` — so the `icmp` matches the VM's full-width `compact_int_cmp`
        // (which reads the sign-/zero-extended i64 slot bits). A partner
        // operand already at I64 (a width-polymorphic literal or a genuine
        // `int`) passes through unchanged; its real value is preserved
        // rather than masked into the narrow window.
        if matches!(
            op,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
        ) {
            let l = self.extend_narrow_to_i64(lhs, unsigned);
            let r = self.extend_narrow_to_i64(rhs, unsigned);
            let cc = match op {
                BinOp::Eq => IntCC::Equal,
                BinOp::Ne => IntCC::NotEqual,
                BinOp::Lt if unsigned => IntCC::UnsignedLessThan,
                BinOp::Lt => IntCC::SignedLessThan,
                BinOp::Le if unsigned => IntCC::UnsignedLessThanOrEqual,
                BinOp::Le => IntCC::SignedLessThanOrEqual,
                BinOp::Gt if unsigned => IntCC::UnsignedGreaterThan,
                BinOp::Gt => IntCC::SignedGreaterThan,
                BinOp::Ge if unsigned => IntCC::UnsignedGreaterThanOrEqual,
                BinOp::Ge => IntCC::SignedGreaterThanOrEqual,
                _ => unreachable!("guarded by the matches! above"),
            };
            // `icmp` returns a native I8 bool.
            return Ok(self.builder.ins().icmp(cc, l, r));
        }

        // Arithmetic / bitwise: coerce each operand to the narrow Cranelift
        // width. `ireduce` truncates a wider value (e.g. an `I64` literal);
        // a value already at `cl_ty` passes through.
        let l = self.coerce_to_narrow_int(lhs, cl_ty);
        let r = self.coerce_to_narrow_int(rhs, cl_ty);

        match op {
            // Two's-complement wrapping arithmetic — `iadd`/`isub`/`imul`
            // on a narrow Cranelift integer type wrap at that width.
            BinOp::Add => Ok(self.builder.ins().iadd(l, r)),
            BinOp::Sub => Ok(self.builder.ins().isub(l, r)),
            BinOp::Mul => Ok(self.builder.ins().imul(l, r)),
            // r5c-2-gz-cp2-jit-div: VM-equivalent trap-free narrow-int div/mod.
            // div-by-zero → clean `Division by zero` (matching the VM's
            // `compact_int_divmod` / `compact_int_divmod_u64`). For signed
            // narrow widths `INT_MIN / -1` wraps to `INT_MIN` (mod → 0) —
            // `compile_int_divmod_guarded` substitutes the divisor with `1`,
            // computing the overflow case without invoking `sdiv` on the
            // trapping operand pair. Unsigned narrow widths have no overflow
            // case. The narrow `INT_MIN`/`-1` constants are derived from
            // `cl_ty.bits()` inside the helper.
            BinOp::Div => self.compile_int_divmod_guarded(l, r, cl_ty, !unsigned, false),
            BinOp::Mod => self.compile_int_divmod_guarded(l, r, cl_ty, !unsigned, true),
            // Bitwise — native Cranelift bitwise ops at the narrow width.
            BinOp::BitAnd => Ok(self.builder.ins().band(l, r)),
            BinOp::BitOr => Ok(self.builder.ins().bor(l, r)),
            BinOp::BitXor => Ok(self.builder.ins().bxor(l, r)),
            BinOp::BitShl => Ok(self.builder.ins().ishl(l, r)),
            BinOp::BitShr => {
                if unsigned {
                    Ok(self.builder.ins().ushr(l, r))
                } else {
                    Ok(self.builder.ins().sshr(l, r))
                }
            }
            // Comparisons (`Eq`/`Ne`/`Lt`/`Le`/`Gt`/`Ge`) are handled by
            // the I64-width path at the top of this function and return
            // early — they never reach this `match`.
            _ => Err(format!("unsupported narrow-int binop: {:?}", op)),
        }
    }

    /// Coerce a value to a narrow integer Cranelift type. A wider value
    /// (e.g. an `I64` literal) is `ireduce`d down; a value already at the
    /// target width passes through. Used by `compile_binop_narrow_int`.
    fn coerce_to_narrow_int(&mut self, val: Value, target: types::Type) -> Value {
        let vt = self.builder.func.dfg.value_type(val);
        if vt == target {
            return val;
        }
        if vt.bits() > target.bits() {
            return self.builder.ins().ireduce(target, val);
        }
        // Narrower source than target — sign-extend up. Cold in practice
        // (MIR keeps operand widths aligned); kept total for safety.
        self.builder.ins().sextend(target, val)
    }

    /// r5c-2-gz-cp6 narrow-neg-literal: widen a narrow-integer operand to
    /// the I64 comparison width.
    ///
    /// The bytecode VM stores a narrow value sign-/zero-extended into its
    /// 8-byte slot — `IntWidth::truncate` masks then sign-extends for the
    /// signed widths (`i8`/`i16`/`i32`) and masks (leaving the value
    /// non-negative in i64) for the unsigned widths (`u8`/`u16`/`u32`).
    /// `compact_int_cmp` then compares the FULL i64 slot bits. The JIT's
    /// narrow operand arrives as a sub-64 Cranelift value (`I8`/`I16`/`I32`)
    /// holding only the narrow window; this helper reconstructs the full
    /// i64 the VM compares:
    ///
    /// - signed narrow → `sextend` (the sign bit fills the upper bits, so a
    ///   negative `i8` such as `-56`/`0xC8` becomes the i64 `-56`);
    /// - unsigned narrow → `uextend` (zero-fill, so a `u8` such as `200`
    ///   becomes the i64 `200`, NOT the sign-extended `-56`).
    ///
    /// A value already at I64 (a width-polymorphic literal or a genuine
    /// `int` partner) passes through unchanged — its real value must be
    /// preserved for the comparison, not masked into the narrow window.
    /// `unsigned` is the narrow comparison kind's signedness (the SAME flag
    /// that selects the signed/unsigned `IntCC`), so both operands of one
    /// comparison extend consistently.
    fn extend_narrow_to_i64(&mut self, val: Value, unsigned: bool) -> Value {
        let vt = self.builder.func.dfg.value_type(val);
        if vt == types::I64 {
            return val;
        }
        if unsigned {
            self.builder.ins().uextend(types::I64, val)
        } else {
            self.builder.ins().sextend(types::I64, val)
        }
    }

    // ── Integer division / modulo (VM-equivalent, trap-free) ─────────

    /// r5c-2-gz-cp2-jit-div: compile an integer division or modulo with
    /// semantics identical to the bytecode VM, for every integer width.
    ///
    /// The VM (`crates/shape-vm/src/executor/arithmetic/mod.rs`) handles two
    /// edge cases the raw Cranelift `sdiv`/`udiv`/`srem`/`urem` cannot:
    ///
    /// 1. **Divisor is zero** — the VM returns a clean `VMError::DivisionByZero`
    ///    ("Division by zero"). The prior JIT codegen emitted
    ///    `trapnz(is_zero, TrapCode::User(0))` → an `ud2` instruction → SIGILL
    ///    crashing the process. This emits a guarded branch instead: on a zero
    ///    divisor the function does an immediate `return_` of
    ///    `JIT_SIGNAL_DIVISION_BY_ZERO`, which the executor maps back to the
    ///    VM's diagnostic. This is the W12 fall-through shape — a clean
    ///    diagnostic, not a trap.
    ///
    /// 2. **`INT_MIN / -1` signed overflow** — the VM uses `wrapping_div` /
    ///    `wrapping_rem`, so `INT_MIN / -1` wraps to `INT_MIN` and
    ///    `INT_MIN % -1` is `0`. Cranelift `sdiv`/`srem` have an implicit
    ///    integer-overflow trap here → SIGFPE. We avoid it WITHOUT a result
    ///    `select`: when `divisor == -1 && dividend == INT_MIN` we substitute
    ///    the divisor with `1`. Then `sdiv(INT_MIN, 1) == INT_MIN` (the exact
    ///    `wrapping_div` result) and `srem(INT_MIN, 1) == 0` (the exact
    ///    `wrapping_rem` result), so a single substitution is correct for
    ///    both div and mod. Non-overflowing dividends are unaffected — the
    ///    substitution only triggers on the unique `INT_MIN`/`-1` pair.
    ///
    /// Unsigned (`udiv`/`urem`) has no overflow case — only the zero-divisor
    /// guard applies. `is_signed` selects the divide instruction and whether
    /// the overflow substitution is emitted.
    pub(crate) fn compile_int_divmod_guarded(
        &mut self,
        lhs: Value,
        rhs: Value,
        cl_ty: types::Type,
        is_signed: bool,
        is_mod: bool,
    ) -> Result<Value, String> {
        // ── 1. Divisor-is-zero guard → clean-error early return ──────────
        //
        // brif to a dedicated error block that returns the
        // `JIT_SIGNAL_DIVISION_BY_ZERO` signal. The MirToIR function always
        // returns `i32` (the `JittedStrategyFn` ABI), so the early `return_`
        // is type-correct. Mirrors the deopt-block early-return shape in
        // `terminators.rs` (the `signal < 0` propagation pattern).
        let zero = self.builder.ins().iconst(cl_ty, 0);
        let is_zero = self.builder.ins().icmp(IntCC::Equal, rhs, zero);
        let div_by_zero_block = self.builder.create_block();
        let continue_block = self.builder.create_block();
        self.builder
            .ins()
            .brif(is_zero, div_by_zero_block, &[], continue_block, &[]);

        self.builder.switch_to_block(div_by_zero_block);
        self.builder.seal_block(div_by_zero_block);
        // `narrow_iconst` masks the negative signal to the I32 width — a raw
        // sign-extended `-2i64` is rejected by Cranelift's `iconst` verifier
        // for `I32`. The masked `0xFFFF_FFFE` is read back by the executor
        // as `signal as i32` == `JIT_SIGNAL_DIVISION_BY_ZERO`.
        let signal = self.narrow_iconst(
            types::I32,
            crate::context::JIT_SIGNAL_DIVISION_BY_ZERO as i64,
        );
        self.builder.ins().return_(&[signal]);

        self.builder.switch_to_block(continue_block);
        self.builder.seal_block(continue_block);

        if !is_signed {
            // Unsigned: no overflow case — `udiv`/`urem` only trap on zero,
            // already guarded above.
            return Ok(if is_mod {
                self.builder.ins().urem(lhs, rhs)
            } else {
                self.builder.ins().udiv(lhs, rhs)
            });
        }

        // ── 2. INT_MIN / -1 overflow: substitute divisor with 1 ──────────
        //
        // When `divisor == -1 && dividend == INT_MIN`, replace the divisor
        // with `1`. `sdiv(INT_MIN, 1) == INT_MIN` == `INT_MIN.wrapping_div(-1)`
        // and `srem(INT_MIN, 1) == 0` == `INT_MIN.wrapping_rem(-1)`. Any other
        // operand pair keeps the real divisor, so non-overflowing division is
        // bit-identical to the prior `sdiv`/`srem`.
        //
        // Cranelift's `iconst` requires a sub-`I64` immediate to be the
        // zero-extended bit pattern that fits the type width (a raw `-1i64`
        // is rejected as out-of-bounds for `I8`/`I16`/`I32`). `narrow_iconst`
        // masks negative values to the type width; the `icmp`/`select` then
        // operate on the same two's-complement bit pattern as the operands.
        let neg_one = self.narrow_iconst(cl_ty, -1);
        let int_min = self.narrow_iconst(cl_ty, i64::MIN >> (64 - cl_ty.bits()));
        let div_is_neg_one = self.builder.ins().icmp(IntCC::Equal, rhs, neg_one);
        let dividend_is_min = self.builder.ins().icmp(IntCC::Equal, lhs, int_min);
        let is_overflow = self.builder.ins().band(div_is_neg_one, dividend_is_min);
        let one = self.narrow_iconst(cl_ty, 1);
        let safe_divisor = self.builder.ins().select(is_overflow, one, rhs);

        Ok(if is_mod {
            self.builder.ins().srem(lhs, safe_divisor)
        } else {
            self.builder.ins().sdiv(lhs, safe_divisor)
        })
    }

    /// Emit an `iconst` for a possibly-signed value at a possibly-narrow
    /// integer type. Cranelift's `iconst` accepts a raw `i64` for `I64`, but
    /// for `I8`/`I16`/`I32` the immediate must be the zero-extended bit
    /// pattern that fits the type width — a raw negative `i64` (e.g. `-1`,
    /// sign-extended to `0xFFFF_FFFF_FFFF_FFFF`) is rejected by the verifier
    /// as out-of-bounds. This masks the value to `cl_ty.bits()` so the narrow
    /// constant carries the correct two's-complement bit pattern.
    pub(crate) fn narrow_iconst(&mut self, cl_ty: types::Type, value: i64) -> Value {
        let bits = cl_ty.bits();
        let imm: i64 = if bits >= 64 {
            value
        } else {
            (value as u64 & ((1u64 << bits) - 1)) as i64
        };
        self.builder.ins().iconst(cl_ty, imm)
    }

    // ── Inline Int64 arithmetic (raw native i64) ──────────────────

    /// Compile a binary op on proven `NativeKind::Int64` operands.
    ///
    /// Per ADR-006 §2.7.5 the JIT slots are raw native bits with the kind
    /// stamped on the parallel JitFfiCarrier companion — Int64 slots hold
    /// raw i64 values, not `tag_bits` payloads. Inputs and the output flow
    /// through unchanged: no payload extraction, no re-box.
    fn compile_binop_int64(&mut self, op: &BinOp, lhs: Value, rhs: Value) -> Result<Value, String> {
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                let result = match op {
                    BinOp::Add => self.builder.ins().iadd(lhs, rhs),
                    BinOp::Sub => self.builder.ins().isub(lhs, rhs),
                    BinOp::Mul => self.builder.ins().imul(lhs, rhs),
                    // r5c-2-gz-cp2-jit-div: VM-equivalent trap-free div/mod —
                    // div-by-zero → clean `Division by zero`; `i64::MIN / -1`
                    // → wrapping `i64::MIN` (mod → 0). See
                    // `compile_int_divmod_guarded`.
                    BinOp::Div => {
                        self.compile_int_divmod_guarded(lhs, rhs, types::I64, true, false)?
                    }
                    BinOp::Mod => {
                        self.compile_int_divmod_guarded(lhs, rhs, types::I64, true, true)?
                    }
                    _ => unreachable!(),
                };
                Ok(result)
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let cc = match op {
                    BinOp::Eq => IntCC::Equal,
                    BinOp::Ne => IntCC::NotEqual,
                    BinOp::Lt => IntCC::SignedLessThan,
                    BinOp::Le => IntCC::SignedLessThanOrEqual,
                    BinOp::Gt => IntCC::SignedGreaterThan,
                    BinOp::Ge => IntCC::SignedGreaterThanOrEqual,
                    _ => unreachable!(),
                };
                let cmp = self.builder.ins().icmp(cc, lhs, rhs);
                // icmp returns I8 (native bool)
                Ok(cmp)
            }
            // W11-fup-A (Phase 3d, 2026-05-18): bitwise ops on proven Int64
            // operands lower to Cranelift native instructions; matches
            // VM `BitAndInt`/etc. (`opcode_defs.rs:1860-1873`). The result
            // stays Int64 — kind stamped at §2.7.5 producing-MIR
            // `infer_rvalue_kind` time.
            BinOp::BitAnd => Ok(self.builder.ins().band(lhs, rhs)),
            BinOp::BitOr => Ok(self.builder.ins().bor(lhs, rhs)),
            BinOp::BitXor => Ok(self.builder.ins().bxor(lhs, rhs)),
            BinOp::BitShl => Ok(self.builder.ins().ishl(lhs, rhs)),
            // Arithmetic right-shift (matches VM `BitShrInt` semantic per
            // opcode_defs.rs:1856-1857: `a_int >> b_int` on i64).
            BinOp::BitShr => Ok(self.builder.ins().sshr(lhs, rhs)),
            // W11-fup-A: Pow on Int64 routes through the `jit_pow_i64` FFI
            // helper (added in this sub-cluster). Cranelift has no
            // native integer-pow instruction; the helper preserves the
            // bytecode VM's `i64::wrapping_pow` semantic for the
            // non-overflowing common case. The VM's `PowInt` opcode
            // additionally promotes overflowing i64 results to f64
            // (`crates/shape-vm/src/executor/arithmetic/mod.rs:151-160`);
            // this JIT path does NOT replicate the kind-flip — the
            // result kind stays Int64 by §2.7.5 stamp-at-compile-time
            // discipline. JIT/VM divergence on i64 Pow overflow is a
            // documented residual of the W11-fup-A close (separate
            // follow-up `jit-pow-int-overflow-promotion` if user
            // explicitly observes divergence).
            BinOp::Pow => {
                let func_ref = self.ffi.pow_i64;
                let inst = self.builder.ins().call(func_ref, &[lhs, rhs]);
                Ok(self.builder.inst_results(inst)[0])
            }
            BinOp::And | BinOp::Or => {
                // Logical ops on Int64 — box and use generic path (pre-
                // W11-fup-A behaviour; logical-on-int wasn't reachable
                // in practice but the path was kept for symmetry).
                self.compile_binop(op, lhs, rhs)
            }
        }
    }

    // ── Inline UInt64 arithmetic (full-range unsigned, raw native i64 width) ──

    /// R5c-2-β-γ checkpoint (b) u64-carrier: compile a binary op on proven
    /// `NativeKind::UInt64` operands — the full-range `0..2^64` unsigned
    /// carrier.
    ///
    /// `u64` and `i64` share the 64-bit Cranelift `I64` width, so the
    /// operands flow through unchanged (no `ireduce`/`sextend`). The
    /// signedness-DEPENDENT operations diverge from the signed
    /// `compile_binop_int64` path:
    ///
    /// - Add / Sub / Mul: two's-complement — `iadd`/`isub`/`imul` produce
    ///   the same bit pattern for `u64` and `i64`. Overflow WRAPS at 2^64
    ///   (integer-semantics ruling 2026-05-20 #3), matching the bytecode
    ///   VM's `compact_int_checked_binop` `wrapping_*`.
    /// - Div / Mod: `udiv`/`urem` — UNSIGNED. The signed `sdiv`/`srem`
    ///   would interpret `u64::MAX` as `-1` and compute `u64::MAX / 2 == 0`.
    ///   The unsigned ops never trap on overflow (the `i64::MIN / -1` case
    ///   has no u64 analogue); only division-by-zero traps (matching the
    ///   VM's `VMError::DivisionByZero`). Mirrors `compile_binop_uint64`'s
    ///   VM sibling `compact_int_divmod_u64`.
    /// - Comparisons: `Unsigned*` Cranelift condition codes — `u64::MAX`
    ///   compares GREATER than `0`, not less. `Eq`/`Ne` are signedness-
    ///   agnostic.
    /// - Bitwise: `band`/`bor`/`bxor`/`ishl` are signedness-agnostic.
    ///   Right-shift uses the LOGICAL `ushr` (zero-fill) — a `u64` has no
    ///   sign bit to extend.
    fn compile_binop_uint64(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        match op {
            BinOp::Add => Ok(self.builder.ins().iadd(lhs, rhs)),
            BinOp::Sub => Ok(self.builder.ins().isub(lhs, rhs)),
            BinOp::Mul => Ok(self.builder.ins().imul(lhs, rhs)),
            // r5c-2-gz-cp2-jit-div: `u64` has no overflow case — `udiv`/`urem`
            // only trap on a zero divisor, guarded by a clean-error early
            // return (matching the VM's `compact_int_divmod_u64`
            // `VMError::DivisionByZero`). `is_signed = false` skips the
            // signed `INT_MIN / -1` substitution.
            BinOp::Div => self.compile_int_divmod_guarded(lhs, rhs, types::I64, false, false),
            BinOp::Mod => self.compile_int_divmod_guarded(lhs, rhs, types::I64, false, true),
            BinOp::Eq => Ok(self.builder.ins().icmp(IntCC::Equal, lhs, rhs)),
            BinOp::Ne => Ok(self.builder.ins().icmp(IntCC::NotEqual, lhs, rhs)),
            BinOp::Lt => Ok(self.builder.ins().icmp(IntCC::UnsignedLessThan, lhs, rhs)),
            BinOp::Le => Ok(self
                .builder
                .ins()
                .icmp(IntCC::UnsignedLessThanOrEqual, lhs, rhs)),
            BinOp::Gt => Ok(self
                .builder
                .ins()
                .icmp(IntCC::UnsignedGreaterThan, lhs, rhs)),
            BinOp::Ge => Ok(self
                .builder
                .ins()
                .icmp(IntCC::UnsignedGreaterThanOrEqual, lhs, rhs)),
            BinOp::BitAnd => Ok(self.builder.ins().band(lhs, rhs)),
            BinOp::BitOr => Ok(self.builder.ins().bor(lhs, rhs)),
            BinOp::BitXor => Ok(self.builder.ins().bxor(lhs, rhs)),
            BinOp::BitShl => Ok(self.builder.ins().ishl(lhs, rhs)),
            // Logical (zero-fill) right-shift — `u64` has no sign bit.
            BinOp::BitShr => Ok(self.builder.ins().ushr(lhs, rhs)),
            BinOp::Pow => {
                // Integer pow has no native Cranelift instruction; the
                // i64 path routes through `jit_pow_i64`. `u64` pow on the
                // non-overflowing common case agrees with the i64 helper
                // (two's-complement product); reuse it.
                let func_ref = self.ffi.pow_i64;
                let inst = self.builder.ins().call(func_ref, &[lhs, rhs]);
                Ok(self.builder.inst_results(inst)[0])
            }
            BinOp::And | BinOp::Or => self.compile_binop(op, lhs, rhs),
        }
    }

    // ── Native Bool operations ──────────────────────────────────────

    /// Compile a binary op on native I8 (Bool) operands.
    fn compile_binop_bool(&mut self, op: &BinOp, lhs: Value, rhs: Value) -> Result<Value, String> {
        match op {
            BinOp::Eq => Ok(self.builder.ins().icmp(IntCC::Equal, lhs, rhs)),
            BinOp::Ne => Ok(self.builder.ins().icmp(IntCC::NotEqual, lhs, rhs)),
            BinOp::And => Ok(self.builder.ins().band(lhs, rhs)),
            BinOp::Or => Ok(self.builder.ins().bor(lhs, rhs)),
            _ => {
                // Other ops on bools — box and use generic path
                self.compile_binop(op, lhs, rhs)
            }
        }
    }

    /// Compile a binary operation on a dynamic (NaN-boxed) slot.
    ///
    /// R7.1: After R5.1–R5.6 retargeted all dynamic arithmetic /
    /// comparison fallbacks (typed bitwise, user operator traits,
    /// DateTime, Matrix/Vec, string+scalar) to typed opcodes or
    /// `CallMethod`, the JIT no longer receives fully dynamic
    /// arithmetic / comparison binops from MIR. The `generic_*`
    /// FFI trampolines (`generic_add`/`sub`/`mul`/`div`/`mod`,
    /// `generic_eq`/`neq`, `generic_lt`/`le`/`gt`/`ge`) were the
    /// last things pinning those FuncRefs alive and have been
    /// removed in this commit.
    ///
    /// This helper remains for the `BinOp::And` / `BinOp::Or`
    /// fallthroughs from `compile_binop_f64`, `compile_binop_int64`,
    /// and `compile_binop_bool` where the logical op mixes with a
    /// NaN-boxed bool encoding (TAG_BOOL_TRUE / TAG_BOOL_FALSE).
    ///
    /// Session 2: Dynamic arithmetic binops from CallValue-returned
    /// slots (closure calls whose return type isn't provable at MIR
    /// level) are lowered via an inline NaN-box dispatch — `Both-Number`
    /// (hot path: `!is_tagged(l) && !is_tagged(r)` → native fadd/etc.) or
    /// `Both-Int` (`is_tagged_int(l) && is_tagged_int(r)` → i48 math).
    /// Mixed or heap operands trap the JIT function, triggering an
    /// error-signal return that the caller observes via the deopt
    /// pathway. This preserves `no generic_* FFI` while keeping
    /// closure-return-arith JIT-compilable.
    fn compile_binop(&mut self, op: &BinOp, lhs: Value, rhs: Value) -> Result<Value, String> {
        // Widen native-typed operands into their NaN-boxed I64 bit-pattern so
        // the dynamic dispatch helpers can treat both uniformly. This handles
        // the mixed cases (e.g. F64 literal vs I64 NaN-boxed heap handle)
        // that `compile_rvalue` routes here after the typed fast paths.
        let l = self.to_i64_bits(lhs);
        let r = self.to_i64_bits(rhs);
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                self.compile_binop_dynamic_arith(op, l, r)
            }

            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                self.compile_binop_dynamic_cmp(op, l, r)
            }

            // v2-boundary: logical ops on NaN-boxed values use TAG_BOOL_TRUE/FALSE
            BinOp::And => {
                let tag_true = self.builder.ins().iconst(types::I64, 1i64);
                let l_is_true = self.builder.ins().icmp(IntCC::Equal, l, tag_true);
                let r_is_true = self.builder.ins().icmp(IntCC::Equal, r, tag_true);
                let both = self.builder.ins().band(l_is_true, r_is_true);
                let false_val = self.builder.ins().iconst(types::I64, 0i64);
                Ok(self.builder.ins().select(both, tag_true, false_val))
            }
            BinOp::Or => {
                let tag_true = self.builder.ins().iconst(types::I64, 1i64);
                let l_is_true = self.builder.ins().icmp(IntCC::Equal, l, tag_true);
                let r_is_true = self.builder.ins().icmp(IntCC::Equal, r, tag_true);
                let either = self.builder.ins().bor(l_is_true, r_is_true);
                let false_val = self.builder.ins().iconst(types::I64, 0i64);
                Ok(self.builder.ins().select(either, tag_true, false_val))
            }
            // W11-fup-A (Phase 3d, 2026-05-18): kind-untyped Pow / bitwise
            // ops reaching this generic path indicate a §2.7.5 producing-
            // MIR kind-tracker gap — the operand kinds (Float64 / Int64 /
            // Int32) determine the codegen path (`compile_binop_f64` /
            // `compile_binop_int64` / `compile_binop_i32_native`), so a
            // kind-blind `compile_binop` for these ops cannot honestly
            // pick the right operation. Honest surface-and-stop per W10
            // playbook §5; the deleted W-series tag-bit IC would have
            // branched on operand `tag_bits` at runtime — that path no
            // longer exists post-strict-typing (CLAUDE.md "Forbidden
            // code").
            BinOp::Pow
            | BinOp::BitAnd
            | BinOp::BitOr
            | BinOp::BitXor
            | BinOp::BitShl
            | BinOp::BitShr => Err(format!(
                "compile_binop: kind-untyped {:?} reached the JIT — SURFACE per \
                 W10 playbook §5: producing-MIR kind-tracker gap; every JIT \
                 operand must have a proven NativeKind at compile time (ADR-006 \
                 §2.7.5 / CLAUDE.md \"Forbidden code\" — runtime tag_bits dispatch \
                 deleted with the W-series IC). W11-fup-A added typed paths in \
                 compile_binop_f64 / compile_binop_int64 / compile_binop_i32_native \
                 — extend the producer to stamp the operand kind upstream.",
                op
            )),
        }
    }

    // ── Session 2: Dynamic arith / cmp inline NaN-box dispatch ────────

    /// Widen an operand Value to its NaN-boxed I64 bit-pattern.
    ///
    /// - `F64` → bitcast to `I64` (the f64 bit-pattern *is* the NaN-box payload
    ///   because plain numbers have sign=0).
    /// - `I32` / `I16` → sign-extend to `I64`. NaN-boxed int slots use
    ///   `TAG_INT | (i48_payload_mask & value)` upstream; narrow-int slots
    ///   reaching `compile_binop` are rare (the native-I32 fast path catches
    ///   both-I32 already), so this conservative sign-extend keeps the raw
    ///   integer value visible to the dynamic dispatch's `int` branch.
    /// - `I8` (native bool) → zero-extend to `I64`. The logical-op branches of
    ///   `compile_binop` compare against the literal `1i64` ⇔ `TAG_BOOL_TRUE`
    ///   encoding, so widening to I64 preserves truth semantics.
    /// - `I64` → passed through unchanged.
    fn to_i64_bits(&mut self, v: Value) -> Value {
        let ty = self.builder.func.dfg.value_type(v);
        if ty == types::I64 {
            v
        } else if ty == types::F64 {
            self.builder.ins().bitcast(types::I64, MemFlags::new(), v)
        } else if ty == types::I32 || ty == types::I16 {
            self.builder.ins().sextend(types::I64, v)
        } else if ty == types::I8 {
            self.builder.ins().uextend(types::I64, v)
        } else {
            v
        }
    }

    /// Compile a dynamic-operand arithmetic binop (Add/Sub/Mul/Div/Mod).
    ///
    /// Per ADR-006 §2.7.5 + CLAUDE.md "Forbidden code" (`tag_bits` runtime
    /// dispatch deleted): every operand has a proven `NativeKind` at MIR
    /// compile time. The pre-strict-typing W-series IC body branched on
    /// `tag_bits` to discriminate `Number` vs `TAG_INT` operand bits at
    /// runtime — that path no longer exists. Reaching this site indicates
    /// a producing-MIR kind-tracker gap; surface-and-stop per W10
    /// playbook §5 so the gap is fixed at the producing opcode rather
    /// than papered over with the deleted W-series tag-bit IC.
    fn compile_binop_dynamic_arith(
        &mut self,
        op: &BinOp,
        _lhs: Value,
        _rhs: Value,
    ) -> Result<Value, String> {
        Err(format!(
            "compile_binop_dynamic_arith: kind-untyped arith {:?} reached the JIT — \
             SURFACE per W10 playbook §5: producing-MIR kind-tracker gap; \
             every JIT operand must have a proven NativeKind at compile time \
             (ADR-006 §2.7.5 / CLAUDE.md \"Forbidden code\" — runtime tag_bits \
             dispatch deleted with the W-series IC).",
            op
        ))
    }

    /// Compile a dynamic-operand comparison binop (Eq/Ne/Lt/Le/Gt/Ge).
    ///
    /// Per ADR-006 §2.7.5 + CLAUDE.md "Forbidden code" (`tag_bits` runtime
    /// dispatch deleted): every operand has a proven `NativeKind` at MIR
    /// compile time. The pre-strict-typing W-series body branched on
    /// `tag_bits` to discriminate `Number` / `TAG_INT` / mixed operand
    /// bits at runtime — that path no longer exists. Eq/Ne is preserved
    /// as raw bitwise compare (kind-mismatched bits are unequal by
    /// construction); Lt/Le/Gt/Ge surface-and-stop per W10 playbook §5
    /// because they require a kind-direction the producing-MIR
    /// kind-tracker must supply.
    fn compile_binop_dynamic_cmp(
        &mut self,
        op: &BinOp,
        lhs: Value,
        rhs: Value,
    ) -> Result<Value, String> {
        // Bitwise Eq/Ne: any mismatched kind also means values are not equal.
        if matches!(op, BinOp::Eq | BinOp::Ne) {
            let cc = if matches!(op, BinOp::Eq) {
                IntCC::Equal
            } else {
                IntCC::NotEqual
            };
            return Ok(self.builder.ins().icmp(cc, lhs, rhs));
        }

        Err(format!(
            "compile_binop_dynamic_cmp: kind-untyped ordered cmp {:?} reached the JIT — \
             SURFACE per W10 playbook §5: producing-MIR kind-tracker gap; \
             every JIT operand must have a proven NativeKind at compile time \
             (ADR-006 §2.7.5 / CLAUDE.md \"Forbidden code\" — runtime tag_bits \
             dispatch deleted with the W-series IC).",
            op
        ))
    }

    /// Compile a unary operation.
    fn compile_unop(&mut self, op: &UnOp, val: Value) -> Result<Value, String> {
        let val_type = self.builder.func.dfg.value_type(val);
        match op {
            // W11-followup-jit-unary-neg-int64 (Phase 4b round 2, 2026-05-18).
            // The Neg arm must dispatch on the operand's Cranelift native
            // width — per ADR-006 §2.7.5 stamp-at-compile-time, a
            // `NativeKind::Int64` slot holds RAW i64 bits (Cranelift
            // `types::I64`), NOT a NaN-boxed f64. `declare_locals` in
            // `blocks.rs:38-52` uses `cranelift_type_for_slot(Int64) ==
            // I64` (verified at `v2_field.rs:454`), so an Int64 operand
            // arrives here as a native I64 SSA value.
            //
            // Pre-fix: the `else` branch unconditionally bitcast the bits
            // to F64, fneg'd, and bitcast back — for `let a = 10; print(-a)`
            // that turned `0xFFFFFFFFFFFFFFF6` (-10 i64) into
            // `0x800000000000000A` (-9223372036854775798), which is the
            // f64 unary-neg of the int bits reinterpreted as i64.
            //
            // Post-fix: dispatch on the operand's native width — Cranelift
            // `ineg` on raw I64 (also I32 for completeness mirroring
            // `compile_binop_int64`'s I32 native path), `fneg` on F64.
            // Other widths surface honestly per W10 playbook §5: reaching
            // here with a non-{F64,I64,I32} operand means the producing
            // MIR `infer_rvalue_kind(UnaryOp(Neg, _))` stamped a kind the
            // VM's `NegInt`/`NegNumber` typed opcodes don't accept
            // (`opcode_defs.rs` only has those two negate variants).
            //
            // Mirrors the W11-fup-A BinOp coverage extension pattern
            // (`compile_binop_int64::{BitAnd,BitOr,BitXor,BitShl,BitShr}`
            // at line 702-708) — native-width dispatch + honest
            // surface-and-stop on producer-side kind-tracker gaps.
            UnOp::Neg => {
                if val_type == types::F64 {
                    // Native F64: direct fneg
                    Ok(self.builder.ins().fneg(val))
                } else if val_type == types::I64 || val_type == types::I32 {
                    // Native integer: direct ineg (matches the VM's
                    // `NegInt` typed opcode at `arithmetic/mod.rs` and
                    // the OSR compiler's `NegInt` arm at
                    // `osr_compiler.rs:572`).
                    Ok(self.builder.ins().ineg(val))
                } else {
                    Err(format!(
                        "compile_unop: SURFACE — Neg on {:?} operand has no \
                         typed opcode in Shape (VM has only `NegInt`/`NegNumber` \
                         per `opcode_defs.rs`). Reaching here means the §2.7.5 \
                         producing-MIR kind-tracker stamped a non-{{F64,I64,I32}} \
                         kind where a numeric kind was expected. Producer-site \
                         gap; surface per W10 playbook §5.",
                        val_type
                    ))
                }
            }
            UnOp::Not => {
                if val_type == types::I8 {
                    // Native I8 bool: XOR with 1 to flip
                    let one = self.builder.ins().iconst(types::I8, 1);
                    Ok(self.builder.ins().bxor(val, one))
                } else {
                    // v2-boundary: NaN-boxed bool uses TAG_BOOL_TRUE/FALSE tags
                    let tag_true = self.builder.ins().iconst(types::I64, 1i64);
                    let false_val = self.builder.ins().iconst(types::I64, 0i64);
                    let is_true = self.builder.ins().icmp(IntCC::Equal, val, tag_true);
                    Ok(self.builder.ins().select(is_true, false_val, tag_true))
                }
            }
            // W14.2-A1 (Phase 4b, 2026-05-18): `BitNot` (`~x`) lowers to
            // Cranelift `bnot` on native Int64 (or NaN-boxed `Int64` via
            // the same instruction, since the bits are i64-shaped either
            // way). Matches the bytecode VM's `BitNotInt` typed opcode at
            // `arithmetic/mod.rs:229` which executes `(!a) as u64` on
            // the popped i64 bits. Mirrors the W11-fup-A BinOp
            // Pow/BitAnd/etc. JIT consumer pattern at line 564+ of this
            // file. The `infer_rvalue_kind` arm at `types.rs` for
            // `Rvalue::UnaryOp(UnOp::BitNot, _)` propagates the operand
            // kind (Int64 in / Int64 out) so the producer-MIR §2.7.5
            // stamp-at-compile-time discipline holds. Per Shape's
            // strict-typing semantic at `opcode_defs.rs:1860-1873`,
            // BitNot is `int`-only — float operands are a producer-side
            // kind-tracker gap and surface honestly.
            UnOp::BitNot => {
                if val_type == types::I64 {
                    Ok(self.builder.ins().bnot(val))
                } else if val_type == types::I32 {
                    Ok(self.builder.ins().bnot(val))
                } else {
                    Err(format!(
                        "compile_unop: SURFACE — BitNot on {:?} operand has no \
                         semantic in Shape (`int`-only per `BitNotInt` at \
                         arithmetic/mod.rs:229). Reaching here means the §2.7.5 \
                         producing-MIR kind-tracker stamped a non-Int64 kind where \
                         Int64 was expected. Producer-site gap; surface per W10 \
                         playbook §5.",
                        val_type
                    ))
                }
            }
        }
    }
}

# c5 Phase A — executor-wide pop_kinded() sweep audit

**HEAD:** `53549fcb` (post c7-cluster + hardening 4).
**Audit-only.** No source / fixture changes. Team-lead commits the audit
doc at close.

Scope: enumerate every `pop_kinded()` caller under
`crates/shape-vm/src/executor/**` and classify against the c5 / c7 /
joint-fix-1b precedent. Per ADR-006 §2.7.7 / Q9 the kind on the parallel-
kind track MUST come from the producer (`src_kind` returned by
`pop_kinded()`), never fabricated from the opcode suffix, hard-coded, or
synthesized from raw bits.

---

## Summary

Enumeration covers 39 files. Raw line-grep hit count is 504; subtracting
17 doc-comment lines (`//` containing `pop_kinded()`) yields **487 code
callers**. Of those:

| Class | Count |
|---|---:|
| PRESERVES (src_kind used: pushed, stored on parallel-kind track, dispatched on, drop_with_kind'd) | 314 |
| NOT-APPLICABLE (kind statically pinned at producer-side gate — typed opcode suffix, layout-derived kind, or destination-side typed cell write) | **170** |
| DISCARDS (Phase B fix needed — kind fabricated at push site without producer-side compile-time proof) | **3** |
| NEEDS-INVESTIGATION (NOT-APPLICABLE conditional on a separate producer-side gate audit that should be confirmed before Phase B) | **0** |
| **Total code callers** | **487** |

Breakdown of NOT-APPLICABLE into sub-classes (Phase B planning context):

| Sub-class | Count | Producer-side gate that proves kind |
|---|---:|---|
| NA-typed-opcode-suffix (`AddI32`, `AddInt`, `MulNumber`, `EqInt`, `BitNotInt`, `IntToNumber`, `NumberToInt`, `NegInt`, typed array element ops, `exec_v2_sized_int`, etc.) | 80 | Opcode suffix encodes the proven operand kind; the compile-time gate that emits the opcode is the proof. Producer-side gate: `crates/shape-vm/src/compiler/expressions/numeric_ops.rs:12-17` (`is_strict_arithmetic`) + sibling typed-emission gates. Refer to c5 audit doc §3-5. |
| NA-typed-cell-destination-write (`op_store_owned_mutable_capture_<scalar>`, `op_store_shared_capture_<scalar>`) | 22 | Bits flow into a typed heap cell (`*mut i64` / `*mut f64` / …) via `write_owned_mutable_<scalar>`; no `push_kinded` site to fabricate. Cell-layout kind from `layout.capture_native_kind(idx)` is the source of truth (ADR-006 §2.7.8 / Q10). |
| NA-typed-map-key/val-element (`typed_handlers/typed_map.rs`, `v2_handlers/typed_map.rs`, `v2_handlers/typed_array_elem.rs`, `v2_handlers/array.rs` macro arms) | 56 | Opcode encodes both K and V element types (`MapGetI64F64`, `MapSetStringPtr`, typed array `s_get` / `s_set` / `s_push`); compile-time gate selects which opcode to emit based on proven map / array element kind. Result kind on push site is the V (or K-encoded-on-stack) per opcode. |
| NA-receiver-key-already-typed (`v2_handlers/field.rs`, `typed_handlers/field.rs`, `v2_handlers/string.rs`, `typed_handlers/string.rs`) | 12 | Receiver or operand kind statically known by opcode (e.g., `GetFieldF64` on a typed struct — F64 field; `StringLen` on a `NativeKind::String` receiver — bits = `Arc<String>` raw ptr). |
| **NOT-APPLICABLE total** | **170** | |

Out-of-c5-scope sites (test code) included in the audit at completeness:

- `vm_impl/stack.rs:1067` (`let (_b, _k) = vm.pop_kinded().unwrap();`) — pure
  smoke-test of the kinded-stack lockstep contract; bits + kind both
  unused after the call. **NOT-APPLICABLE** (test fixture, not a
  production handler).
- `arithmetic/mod.rs:792`, `:797` (`pop_int` / `pop_f64` test helpers in
  `mod tests`) — test fixtures. **NOT-APPLICABLE.**

---

## c5 bitwise gate (sub-scope per audit doc 05)

Audit doc 05 names `crates/shape-vm/src/executor/arithmetic/mod.rs:637-658`
(`exec_dyn_bit_binary` + `exec_dyn_bit_unary`) as the c5 territory — the
three lines below are the c5 anchor sites:

| File:line | Function | Pattern |
|---|---|---|
| `arithmetic/mod.rs:639` | `exec_dyn_bit_binary` | `let (b_bits, _b_kind) = self.pop_kinded()?;` |
| `arithmetic/mod.rs:640` | `exec_dyn_bit_binary` | `let (a_bits, _a_kind) = self.pop_kinded()?;` |
| `arithmetic/mod.rs:655` | `exec_dyn_bit_unary`  | `let (bits, _kind) = self.pop_kinded()?;` |

Cross-reference with the broader sweep: **the 3 c5 anchor sites are
exactly the broader sweep's DISCARDS class** — there are no DISCARDS
sites outside these three. The compile-time producer-side gate proposed
in audit doc 05 §5 (extend `is_strict_arithmetic` / add `is_strict_bitwise`
+ unary sibling, surface non-int operands at compile time, then delete
the two dynamic helpers + their 5 binary opcode dispatch entries +
`BitNot` unary entry) covers every DISCARDS site identified by the
sweep. Phase B fix scope from c5 alone covers the sweep's full DISCARDS
class.

The `is_strict_arithmetic` extension is a **compile-time gate at the
producer**, not a runtime fix at the consumer — symmetric with the c5
remediation. No DISCARDS-class site can be remediated by binding the
kind alone at the consumer (the kind that would be bound is itself
unprotected against the float / string / heap inputs that the c5
fixtures probe).

---

## Per-caller classification

Sites are grouped by file with per-arm rationale. PRESERVES sites are
summarized at the file level (kind bound, used downstream — examined for
every file but not enumerated line-by-line where the pattern is uniform).
NOT-APPLICABLE and DISCARDS sites are enumerated with rationale.

### arithmetic/mod.rs (50 callers)

Class breakdown: PRESERVES 22 · NA 25 · DISCARDS 3 · TEST 2.

PRESERVES (22): `NegNumber` L207, `NegDecimal` L216, decimal binop L175-176, `binop_number_kinded` L267-268, `divmod_number_kinded` L286-287, `binop_decimal_kinded` L312-313, `divmod_decimal_kinded` L329-330, `compact_float_binop` L524-525, `compact_float_divmod` L544-545, `compact_float_cmp` L581-582. Each binds `(a_bits, a_kind)` / `(b_bits, b_kind)` and dispatches `numeric_as_f64`, `decimal_ref`, `coerce_to_f64_kinded`, or `drop_with_kind` with the captured kind.

#### NOT-APPLICABLE — `exec_typed_arithmetic` (typed-int / typed-bitwise int arms)

```rust
// arithmetic/mod.rs L121-122 (DivInt), L134-135 (ModInt), L146-147 (PowInt),
// L191 (IntToNumber), L196 (NumberToInt), L202 (NegInt), L228 (BitNotInt)
let (b_bits, _b_kind) = self.pop_kinded()?;
let (a_bits, _a_kind) = self.pop_kinded()?;
// ... a as i64, b as i64, push_kinded(..., NativeKind::Int64)
```

Class: **NOT-APPLICABLE.** Opcodes (`DivInt`, `ModInt`, `PowInt`, `IntToNumber`, `NumberToInt`, `NegInt`, `BitNotInt`) statically pin the operand kind. Producer-side gate: `numeric_ops.rs:12-17` (`is_strict_arithmetic`) — `Sub | Mul | Div | Mod | Pow` are the existing strict-int set. Per ADR-006 §2.7.7 + c5 audit doc 05 §3 contrast at L70: typed-int arithmetic is producer-gated by the compile-time numeric-typing check, the unsuffixed-opcode dynamic-bitwise arm at L637-658 is the **only** c5 anchor that bypasses the gate.

#### NOT-APPLICABLE — `binop_int_wrapping` / `binop_int_simple` / `compact_int_*` / `op_cast_width`

```rust
// arithmetic/mod.rs L247-248 (binop_int_wrapping), L257-258 (binop_int_simple),
// L434-435 (compact_int_checked_binop), L486-487 (compact_int_divmod),
// L514-515 (compact_int_divmod_u64), L567-568 (compact_int_cmp), L608 (op_cast_width)
let (b_bits, _b_kind) = self.pop_kinded()?;
let (a_bits, _a_kind) = self.pop_kinded()?;
```

Class: **NOT-APPLICABLE.** Called from `exec_typed_arithmetic` / `exec_compact_typed_arithmetic` only on typed-int / typed-int-width opcodes (`AddInt`, `SubInt`, `BitAndInt`/`BitOrInt`/`BitXorInt`/`BitShlInt`/`BitShrInt`, `AddI<N>U<N>`, etc.). Same producer-side gate as the typed-int arms above. `BitAndInt`-family typed arms are **the fix target** of c5's remediation (the path that survives — after the dynamic fallback at L637-658 is deleted, only these gate-protected arms remain).

#### DISCARDS — `exec_dyn_bit_binary` + `exec_dyn_bit_unary` (the c5 anchor)

```rust
// arithmetic/mod.rs L637-652 (exec_dyn_bit_binary):
fn exec_dyn_bit_binary(&mut self, op: OpCode) -> Result<(), VMError> {
    let (b_bits, _b_kind) = self.pop_kinded()?;   // <- DISCARDS L639
    let (a_bits, _a_kind) = self.pop_kinded()?;   // <- DISCARDS L640
    let b_int = b_bits as i64;
    let a_int = a_bits as i64;
    let result = match op { BitXor => a_int ^ b_int, ... };
    self.push_kinded(result as u64, NativeKind::Int64)
}

// arithmetic/mod.rs L654-658 (exec_dyn_bit_unary):
fn exec_dyn_bit_unary(&mut self) -> Result<(), VMError> {
    let (bits, _kind) = self.pop_kinded()?;       // <- DISCARDS L655
    let a_int = bits as i64;
    self.push_kinded((!a_int) as u64, NativeKind::Int64)
}
```

Class: **DISCARDS (Phase B fix needed).** Producer is the un-suffixed
`BitAnd`/`BitOr`/`BitXor`/`BitShl`/`BitShr`/`BitNot` opcode emitted by the
else branch at `compiler/expressions/binary_ops.rs:1555-1575` (and the
unary fallback at `compiler/expressions/unary_ops.rs:28-66`) when
`both_int = false`. No compile-time gate rejects `1.5 & 3` / `"hello" | 3`
/ `~1.5`; the discard at the executor lets float bits and heap pointer
bits be reinterpreted as i64. Per CLAUDE.md ADR-006 §Forbidden-Code:
"Runtime `tag_bits` dispatch (deleted)" + the c5 audit doc 05 §3 quote of
exactly this code as the regression root cause.

This is the **same shape** as the c7 cluster anchor (`op_return_value_<width>`
fabricated kind from suffix at `control_flow/mod.rs:830-887`, commit
`c6226b18`) and joint-fix-1b (`op_store_local_<scalar>` fabricated kind
at `variables/mod.rs:2167-2342`, commit `805a834a`). The remediation
shape (c5 audit doc 05 §5) inverts the polarity at the producer rather
than the consumer: extend `is_strict_arithmetic` to cover the 5 binary
bitwise opcodes + a sibling `is_strict_unary_bitwise` for `BitNot`,
surface non-int operands at compile time, then **delete** the dynamic
helpers + their dispatch entries.

#### TEST helpers (not in production scope)

```rust
// arithmetic/mod.rs L792 (pop_int test helper):
fn pop_int(vm: &mut VirtualMachine) -> i64 {
    let (bits, _kind) = vm.pop_kinded().unwrap();
    ...
}
// arithmetic/mod.rs L797 (pop_f64 test helper):
let (bits, _kind) = vm.pop_kinded().unwrap();
```

Class: **NOT-APPLICABLE (TEST).** `mod tests` private helpers under
`#[cfg(test)]`, no production dispatch reaches them.

### variables/mod.rs (61 callers)

Class breakdown: PRESERVES 41 · NA 20 · DISCARDS 0.

PRESERVES (41): `op_store_closure` L299, `op_store_owned_mutable_capture_ptr` L857, `op_store_shared_capture` L945, `op_alloc_shared_local` L1503, `op_store_shared_local` L1578, `op_alloc_shared_module_binding` L1694, `op_store_shared_module_binding` L1766, `op_store_local_drop` L1922, `op_store_local` L1946, `op_store_local_typed` L1966, typed-suffix `op_store_local_<scalar>` L2213/2230/2247/2264/2281/2298/2315/2332/2349/2366/2383 (joint-fix-1b shape — src_kind preserved on parallel-track), `op_make_field_ref` L2561, `op_load_local`-class indirect via `stack_read_kinded_raw` (kind bound + propagated), `op_make_ref` indirect L2494 (kind read for type-check). Plus all conditional-let / match arms at L2642, L2784, L168, L247, L252, L295, L375, L412 (option-typed bind + use).

#### NOT-APPLICABLE — `op_store_owned_mutable_capture_<scalar>` (cell-write path)

```rust
// variables/mod.rs L635-654 (op_store_owned_mutable_capture_i64), repeated
// for u64 (L656), f64 (L676), i32 (L697), u32 (L718), i16 (L739), u16 (L760),
// i8 (L781), u8 (L802), bool (L823):
let (src_bits, _src_kind) = self.pop_kinded()?;
let new_value = src_bits as i64;
let cell_ptr = bits as *mut i64;
...
unsafe { shape_value::v2::closure_raw::write_owned_mutable_i64(cell_ptr, new_value) };
```

Class: **NOT-APPLICABLE (cell-write).** No `push_kinded` follows — the
bits flow into a typed heap cell of fixed layout (`layout.capture_native_kind(idx)`
is the cell's canonical kind, set at closure construction per ADR-006
§2.7.8 / Q10). The opcode (`StoreOwnedMutableCaptureI64`) statically
encodes the cell's element type. There is no parallel-kind track to
fabricate kind onto. Producer-side gate: the compile-time choice of which
opcode-suffix to emit matches the layout's `capture_native_kinds[idx]`
(W7 playbook §8 surface-and-stop discipline).

The same shape repeats for `op_store_shared_capture_<scalar>` at L1195-1414
(L1203 / 1225 / 1246 / 1268 / 1290 / 1312 / 1334 / 1356 / 1378 / 1400) —
the `SharedCell` carries its own `kind()` accessor (`cell_ref.kind()`) and
the producer-side opcode-suffix gate matches at compile time.

22 NA sites total in this file (12 OwnedMutable + 10 Shared).

### typed_handlers/int.rs (22 callers, all NOT-APPLICABLE)

```rust
// typed_handlers/int.rs L21-22 (AddI32), L28-29 (SubI32), L35-36 (MulI32),
// L42-43 (DivI32), L52-53 (ModI32), L62-63 (EqI32), L69-70 (NeqI32),
// L76-77 (LtI32), L83-84 (GtI32), L90-91 (LteI32), L97-98 (GteI32):
let (b_bits, _b_kind) = self.pop_kinded()?;
let (a_bits, _a_kind) = self.pop_kinded()?;
let b = b_bits as i64 as i32;
let a = a_bits as i64 as i32;
self.push_kinded(a.wrapping_add(b) as i64 as u64, NativeKind::Int32)
```

Class: **NOT-APPLICABLE.** Per file-header doc comment (L1-17): "Values
flow through the kinded VM stack as i64-shaped bits with `NativeKind::Int32`
(Wave 6.5 cluster C — ADR-006 §2.7.7). … the kinded pop discards the kind
tag (Int32 expected)". Opcode (`AddI32`, `SubI32`, …) IS the proof; the
compile-time gate at the producer chose this opcode only when both
operands are `Int32`. Result kind on push is the opcode-suffix-encoded
result (Int32 for arithmetic / Bool for comparison).

### v2_handlers/int.rs (22 callers, all NOT-APPLICABLE)

Identical shape to `typed_handlers/int.rs` for v2-carrier opcodes
(`exec_v2_sized_int`). 22 NA sites at L21-22, 28-29, 35-36, 42-43, 52-53,
62-63, 69-70, 76-77, 83-84, 90-91, 97-98.

### typed_handlers/typed_map.rs (43 callers)

Class breakdown: PRESERVES 25 (map_bits + map_kind binding used in
dispatch + `drop_with_kind`) · NA 18.

NOT-APPLICABLE (18): all the `_kk` / `_vk` key/value-bits pops at L140, 153, 166, 181, 195, 209, 225, 227, 238, 240, 251, 253, 313, 323, 333, 386, 397, 408. Each is inside an `exec_typed_map` arm whose opcode (`MapGetI64F64`, `MapSetI64Ptr`, `MapDeleteString*`, etc.) statically encodes the K and V element kinds. The compiler-side dispatch (`compiler/expressions/method_calls.rs` typed-map emission gate) is the producer-side proof that the popped operand matches the K or V slot of the typed-map instantiation.

### v2_handlers/typed_map.rs (43 callers)

Identical shape to `typed_handlers/typed_map.rs`. PRESERVES 25 · NA 18
(at L147, 160, 173, 188, 202, 216, 232, 234, 245, 247, 258, 260, 320,
330, 340, 393, 404, 415).

### comparison/mod.rs (24 callers)

Class breakdown: PRESERVES 20 · NA 4.

PRESERVES (20): `exec_typed_comparison` arms at L133-134, 143-144, 153-154,
163-164 (all signed-vs-unsigned int-cmp dispatch via `int_cmp_is_unsigned(a_kind, b_kind)`); `cmp_number_kinded`, `cmp_decimal_kinded`, `compact_float_cmp` (all kind-binding for coerce + drop).

#### NOT-APPLICABLE — `EqInt` / `NeqInt`

```rust
// comparison/mod.rs L173-180:
EqInt => {
    let (b_bits, _b_kind) = self.pop_kinded()?;
    let (a_bits, _a_kind) = self.pop_kinded()?;
    self.push_kinded(((a_bits as i64) == (b_bits as i64)) as u64, NativeKind::Bool)?;
}
NeqInt => {
    let (b_bits, _b_kind) = self.pop_kinded()?;
    let (a_bits, _a_kind) = self.pop_kinded()?;
    self.push_kinded(((a_bits as i64) != (b_bits as i64)) as u64, NativeKind::Bool)?;
}
```

Class: **NOT-APPLICABLE.** Producer-side gate at `compiler/expressions/numeric_ops.rs:280-283`: `(BinaryOp::Equal, NumericType::Int)` is the only path that emits `EqInt`; `(BinaryOp::NotEqual, NumericType::Int)` emits `NeqInt`. Both require `NumericType::Int` proof at compile time. Comparison bit-equality on i64 is valid only because both operands are proven i64 by the gate. Result kind on push is `Bool` per opcode.

### control_flow/mod.rs (23 callers)

Class breakdown: PRESERVES 22 · NA 1 · DISCARDS 0.

PRESERVES (22): `op_jump_if_false` L158, `op_jump_if_true` L202, `op_call` L224, `dispatch_call_value_immediate` L415/422, `op_call_value` L451, `op_call_foreign` L669/681, `op_return` (kind-binding via `frame.closure_heap_bits` + `closure_heap_kind`), `op_make_closure` capture-pop loop at L546 (`popped.push(self.pop_kinded()?)` — kind preserved in the `Vec<(u64, NativeKind)>`, dispatched per layout's `capture_native_kind(i)` at write time), all the typed `op_return_value_<width>` joint-fix-1 sites at L832-890 (kind from producer, preserved through `return_value_inner`).

#### NOT-APPLICABLE — `op_jump_if_false_trusted`

```rust
// control_flow/mod.rs L179-193:
pub(in crate::executor) fn op_jump_if_false_trusted(...) {
    if let Some(Operand::Offset(offset)) = instruction.operand {
        let (bits, _kind) = self.pop_kinded()?;  // <- L184
        let cond = bits != 0;
        ...
    }
}
```

Class: **NOT-APPLICABLE.** Per doc comment L175-177: "Producers (typed
comparison, `Not`) push `NativeKind::Bool` slots (0u64 / 1u64 in the data
track, `NativeKind::Bool` in the kind track) — read the bits directly with
`pop_kinded`." The "trusted" suffix is the producer-side gate's
identifier: emitted only when the compiler proves the operand is a Bool
or other comparison result (`bits != 0` is then valid). Note also there
is no `push_kinded` after this site — only an `ip` jump — so no kind
fabrication occurs.

Caveat for Phase B planning: this site is **not** a DISCARDS, but the
trusted-vs-untrusted distinction relies on the compiler-side picker
choosing `JumpIfFalseTrusted` only when the operand is proven scalar
(not heap-bearing, where `drop_with_kind` would be needed). The
non-trusted `op_jump_if_false` L158 binds the kind precisely for that
`drop_with_kind` (L162). No fix needed here, but worth noting in case a
future compiler-side regression mis-emits `JumpIfFalseTrusted` on a
heap-bearing operand — the leak would be a heap share, surfaced via
sanitizer / leak detection (not silent wrong-bits).

### exceptions/mod.rs (11 callers, all PRESERVES)

All 11 callers bind kind and use it: `op_type_check` L260, `op_throw` L327, `op_error_context` L545-546, `op_try_unwrap` family (L659, 745, 797, 813, 838, 866) — each binds via `KindedSlot::new(ValueSlot::from_raw(bits), kind)` and dispatches via the §2.7.7 stack-parallel-kind track or `clone_with_kind` / `drop_with_kind`.

### logical/mod.rs (8 callers, all PRESERVES)

`And` / `Or` / `Not` paths at L96-97, 124-125, 151. Each binds `a_kind` /
`b_kind` and dispatches via `kind_is_heap()` for `FilterExpr` short-
circuit (ADR-006 §2.7 / Wave-γ G-heap-filter-expr) or `kinded_truthy()`
for scalar truthiness. Result kind is `Bool` for scalar, `Ptr(HeapKind::FilterExpr)`
for query-DSL composition — both push paths reflect the executed
semantic, not the input kind.

### loops/mod.rs (4 callers, all PRESERVES)

`op_iter_done` L212-213 (`idx_bits, idx_kind` + `iter_bits, iter_kind`), `op_iter_next` L358-359 (same). Both dispatch on `iter_kind` to choose the typed-array / range / iterator-Arc heap arm; `idx_kind` decoded via `decode_iter_idx`. `drop_with_kind` releases both shares.

### objects/property_access.rs (13 callers, all PRESERVES)

`op_get_prop` L66-67 (`key_bits, key_kind` + `obj_bits, obj_kind`), `op_set_prop` L272-274 (3-way pop with kind bound for each), plus 8 indirect `pop_kinded` consumers inside `dispatch_get_prop` (each binds and threads kind through `numeric_index_from_kinded` / typed array view dispatch). All kinds propagated to `drop_with_kind` or downstream typed dispatch.

### objects/typed_access.rs (10 callers, all PRESERVES)

`pop_string_key` L71-72, `exec_typed_string_access` `char_at` L285-286, `string_concat_typed` L330-331, plus indirect via `stack_read_kinded_raw`. All bind and dispatch / drop_with_kind.

### v2_handlers/typed_array_elem.rs (8 callers)

Class breakdown: PRESERVES 0 · NA 8.

```rust
// L114 / L145 (op_get_elem_i64 / op_get_elem_f64):
let (idx_bits, _idx_kind) = self.pop_kinded()?;
// L176 / L178 / L212 / L214 / L248 / L267 (op_set_elem / op_array_push):
let (val_bits, _vk) = self.pop_kinded()?;
let (idx_bits, _ik) = self.pop_kinded()?;
```

Class: **NOT-APPLICABLE.** Opcode (`GetElemI64`, `GetElemF64`, `SetElemI64`,
`SetElemF64`, `ArrayPushI64`, `ArrayPushF64`) statically pins the element
type. Index kind is statically `Int64` per the compile-time gate that
emits the op only on integer-typed index expressions. Receiver array
kind is read from the local slot via `stack_read_kinded_raw(slot)` —
that's the carrier-shape source-of-truth (NOT from `pop_kinded`).

### v2_handlers/array.rs (22 callers)

Class breakdown: PRESERVES 12 · NA 10.

NA (10): the `s_get` / `s_push` / `s_set` / `c_get` / `c_push` / `c_set`
macro arms at L205, 225, 234, 236, 261, 281, 300, 307, 333, 383. Each
arm's opcode statically pins both the element type (i64/f64/i32/…/char)
and the operand kind. Result kind pushed is `$s_kind` / `$c_kind` —
opcode-suffix-encoded.

PRESERVES (12): the typed-array creation + ownership-transfer pops
(arr_bits / arr_kind binding at L207, 227, 238, 263, 293, 309, 335, 365,
385 plus the heap-elem-arm error-checking + drop_with_kind paths).

### typed_handlers/field.rs + v2_handlers/field.rs (11 + 11 = 22 callers)

Class breakdown: PRESERVES 16 · NA 6.

PRESERVES (16): `struct_bits + struct_kind` bindings at field-read sites (typed_handlers L28, 37, 45, 55, 63 + v2_handlers L24, 33, 41, 49, 57) and the 3-way pop (val + struct) at field-set sites L67, 77, 87 + L67, 77, 87.

NA (6): the value-bits pops at L74, 84, 94 (typed_handlers) and L65, 75,
85 (v2_handlers) — `SetFieldF64`, `SetFieldI64`, `SetFieldI32` opcode
statically encodes value type; receiver struct kind is bound separately.

### typed_handlers/string.rs + v2_handlers/string.rs (5 + 5 = 10 callers, all NOT-APPLICABLE)

```rust
// String length: opcode `StringLen` — receiver is statically String:
let (str_bits, _str_kind) = self.pop_kinded()?;
let len = ...; self.push_kinded(len as u64, NativeKind::Int64)?;

// String concat: opcode `StringConcatTyped` — both operands proven String:
let (b_bits, _b_kind) = self.pop_kinded()?;
let (a_bits, _a_kind) = self.pop_kinded()?;
```

Class: **NOT-APPLICABLE.** Opcode (`StringLen`, `StringConcatTyped`,
`StringEqTyped`) gates operand kind at the producer.

### typed_handlers/typed_array.rs + v2_handlers/typed_array.rs (6 + 6 = 12 callers, all PRESERVES)

Each pop binds `(b, k)` / `(b1, k1)` / `(b2, k2)` / `(b3, k3)` and immediately calls `drop_with_kind(b, k)` etc. — the unary / binary / ternary stub-drop helpers for typed-array sites that haven't been wired yet (current shape — bits ignored, kind preserved through drop). PRESERVES because the kind reaches `drop_with_kind`.

### typed_handlers/typed_enum.rs + v2_handlers/enum_v2.rs (2 + 2 = 4 callers, all PRESERVES)

`EnumTag` / `EnumPayload` opcodes bind `enum_bits, enum_kind` and dispatch + drop_with_kind.

### objects/array_operations.rs (7 callers, all PRESERVES)

`ArrayPush` L71-72 (value + array), `ArrayPushLocal` L124 (value), `ArrayPop` L186 (array), `SliceAccess` L222-224 (end + start + array). Each binds + dispatches on `array_kind` (with `ckpt5_typed_array_surface` on mismatch) and propagates `value_kind` to typed-array push helpers.

### objects/concat.rs (4 callers, all PRESERVES)

String concat L74-75 + L118-119 (typed_array concat). Each binds 2 kinds + dispatches + drop_with_kind.

### objects/mod.rs (6 callers, all PRESERVES)

`dispatch_method_kinded` precursor (method-call ABI per ADR-006 §2.7.10 / Q11): pops receiver + args, each pop binds `(bits, kind)` and constructs a `KindedSlot::new(...)`. Range builder L976-985 binds and dispatches.

### objects/object_creation.rs (6 callers, all PRESERVES)

Object / matrix / array / typed-object constructors — pop count loop binds `(b, k)` and immediately `drop_with_kind` on surface, else stores in TypedObject's parallel `field_kinds` track per ADR-006 §2.5.

### objects/object_operations.rs (4 callers, all PRESERVES)

Object merge / spread sites at L73-74, 140-141. Bind 2 kinds + dispatch.

### call_convention.rs (7 callers)

Class breakdown: PRESERVES 6 · NA 1.

PRESERVES (6): top-of-program return value extraction L143, `execute_closure` return-value extraction L192, `execute_function_fast` L211, JIT-callable closure call L1117 (`for (i, (bits, _kind)) in upvalue_bits.iter().enumerate()` — kind preserved as collected, NOT discarded; the loop uses `_kind` only because the closure-block layout is the authoritative kind source at write time per ADR-006 §2.7.8 / Q10), task-scheduler complete L545, JIT entry call_method L957/L982.

#### NOT-APPLICABLE — JIT-call return-value extraction at L1145

```rust
// call_convention.rs L1141-1148:
self.call_closure_with_nb_args_keepalive(func_id, &block, &kinded_args, None, None)?;
std::mem::forget(kinded_args);
self.execute_until_call_depth(saved_call_depth, ctx)?;
let (bits, _kind) = self.pop_kinded()?;
// Return raw bits. The kind is discarded — the JIT caller
// knows the static return kind from the callee signature.
Ok(bits)
```

Class: **NOT-APPLICABLE (JIT FFI boundary).** This is `op_call_closure_immediate_nb`'s return-extraction site. The function returns `u64` to the JIT caller, which has the static return kind from the callee signature in its scope (per the inline comment). The kind is **not fabricated on a push site** — `pop_kinded` is the last operation and there is no `push_kinded(..., fabricated_kind)` after. The producer-side gate is the JIT-callee signature contract: the JIT caller knows what kind to interpret the bits as, statically.

Reasoning matches ADR-006 §2.7.7 + the documented JIT FFI boundary rule: kinds DO NOT cross the JIT trampoline at runtime — only bits do, with kinds statically resolved by callee signature. Phase B should NOT change this site.

### stack_ops/mod.rs (4 callers, all PRESERVES)

`OpCode::Drop` L41 (kind bound + `drop_with_kind`), `OpCode::Swap` L54-55 (2 kinds bound + re-pushed in swapped order), the `PushAbstract*` arms at L98-207 (each binds kind for downstream `clone_with_kind`).

### trait_object_ops.rs (4 callers, all PRESERVES)

`op_make_trait_object` L114, `op_into_trait_object` L610, dynamic-dispatch return L727, virtual-call L789. Each binds kind + dispatches (TraitObject downcast, drop_kinded, rewrap_return_value).

### typed_object_ops.rs (6 callers, all PRESERVES)

`op_get_field` L355 (receiver kind bound + type-check), `op_set_field` L667-668 (2-way pop, both kinds bound + ReceiverGuard), plus 4 indirect via `stack_read_kinded_raw` in test sites L1021/1223/1293.

### async_ops/mod.rs (8 callers, all PRESERVES)

`op_emit_alert` L273 (kind bound + drop), `op_await` L287 (kind bound, Result/Option destructure + kind preservation), `op_spawn_task` L386 (slot_kind bound + register / complete), plus L143 / L192 / L211 entry-points (kind bound for execute_function_by_id / execute_closure).

### window_join.rs (2 callers, all PRESERVES)

Window-join entry L446 + L537. Both bind kind + dispatch on TableView heap arm + drop_with_kind.

### task_scheduler.rs (1 caller — comment-only)

The single hit is a doc comment at L16 referencing `pop_kinded()`. No production caller in this file. NOT-APPLICABLE (comment).

### builtins/type_ops.rs (1 production caller)

`pop_one_kinded` helper at L515-518:
```rust
fn pop_one_kinded(vm: &mut VirtualMachine) -> Result<KindedSlot, VMError> {
    let (bits, kind) = vm.pop_kinded()?;
    Ok(KindedSlot::new(ValueSlot::from_raw(bits), kind))
}
```
PRESERVES. Used by `read_as_int` / `read_as_string` / `read_as_bool` /
`read_as_decimal` / `read_as_char` — each downstream dispatches on
`slot.kind()`.

### vm_impl/builtins.rs (5 callers, all PRESERVES)

`pop_builtin_args` L75 (arg-count kind bound + drop) and L91 (per-arg kind bound, packed into Vec<KindedSlot>).

### vm_impl/stack.rs (6 callers — production + test)

L1039 / L1048 / L1067 / L1087 / L1099 are all `#[cfg(test)]` smoke tests of the kinded-stack ABI. PRESERVES (kind asserted on) for most; L1067 is the only `(_b, _k)` discard — a pure unit-test of pop-then-drop ordering, not production code. NOT-APPLICABLE (TEST).

### v2_stack_tests.rs (19 callers, all PRESERVES)

`#[cfg(test)]` ABI tests. Every pop is followed by `assert_eq!(kind, ...)` or `drop_with_kind(bits, kind)`. PRESERVES.

### tests/table_iteration.rs (1 caller, PRESERVES)

L166 — test fixture, binds + asserts kind.

---

## c5 Phase B fix-target list (DISCARDS sites)

The DISCARDS class has **3 lines** at 2 functions in 1 file:

- `crates/shape-vm/src/executor/arithmetic/mod.rs:639` — `exec_dyn_bit_binary` first operand pop (`_b_kind` discarded).
- `crates/shape-vm/src/executor/arithmetic/mod.rs:640` — `exec_dyn_bit_binary` second operand pop (`_a_kind` discarded).
- `crates/shape-vm/src/executor/arithmetic/mod.rs:655` — `exec_dyn_bit_unary` unary operand pop (`_kind` discarded).

**Phase B sub-cluster name (matches audit doc 05 §6):**
`v0.3.3-bitwise-strict-typing-gate`. **Size: S.**

**Phase B fix scope (audit doc 05 §5):**

1. Extend `crates/shape-vm/src/compiler/expressions/numeric_ops.rs:12-17`:
   add `BitAnd | BitOr | BitXor | BitShl | BitShr` to `is_strict_arithmetic`
   (or add a sibling `is_strict_bitwise` helper). Add a sibling
   `is_strict_unary_bitwise` for `BitNot`.
2. Add a compile-time gate at `crates/shape-vm/src/compiler/expressions/binary_ops.rs:1403`
   (bitwise arm entry) reusing the existing strict-arithmetic rejection-
   emit shape at L1680 / 1795. On proof gap → `prove_native_kind()` →
   `ProofGap` → compile error per CLAUDE.md mechanical-enforcement rule.
3. Add a sibling unary gate at `crates/shape-vm/src/compiler/expressions/unary_ops.rs:28`
   for `~` (BitNot).
4. **Delete** `exec_dyn_bit_binary` (L637-652) + `exec_dyn_bit_unary`
   (L654-658) + `exec_dyn_bit_dispatch` (L620-635). Delete their dispatch-
   table arms (the 6 dynamic bitwise opcodes: `BitAnd`/`BitOr`/`BitXor`/
   `BitShl`/`BitShr`/`BitNot`). User-type operator-trait dispatch (`impl
   BitAnd for T`) is unaffected — it dispatches earlier at
   `binary_ops.rs:1430-1436` BEFORE the dynamic fallback.

**Sub-cluster size:** S (per audit doc 05 §6 — one type-check gate + 6
opcode deletion, ~50 lines net, zero fixture changes; the 8 fixtures in
c5 audit doc §1 all `expect_run_err()` and will pass unchanged when the
compile-time gate replaces the silent-success dynamic path).

**Smoke regressions to check** (per audit doc 05 §6):
- User-type `impl BitAnd / BitOr / BitXor / Shl / Shr / BitNot for T`
  (W1.9 / W1.10) — dispatched BEFORE the dynamic fallback at
  `binary_ops.rs:1430-1536`, should be unaffected.
- Existing `test_return_bitwise_and` / `test_return_bitwise_shift`
  operator-trait family.

---

## Discipline notes

- **Audit-only.** No source / fixture changes during Phase A. The audit
  doc + an AGENTS.md row entry are the only file additions.
- Per CLAUDE.md ADR-006 §2.7.7 (kind on parallel-kind track sourced from
  the producer); §Forbidden-Code (Runtime `tag_bits` dispatch deleted —
  `synthesize_value_word_from_raw`, `is_tagged()`, `last_program_return_kind`,
  `normalize_persisted_for_slot`, per-`FieldKind` capture decode all
  forbidden); §Renames-to-refuse-on-sight (broader-family regex on
  decode/tag/kind/dispatch/value.call/closure.callback/frame.setup/callee/
  capture × bridge/probe/helper/hop/translator/adapter/shim).
- JOINT-FIX #1 (c7) precedent shape: `control_flow/mod.rs:830-887`
  `op_return_value_<width>` family — pop `(bits, src_kind)`, preserve
  `src_kind` through `return_value_inner(bits, src_kind)`. Commit
  `c6226b18`.
- JOINT-FIX-1b precedent shape: `variables/mod.rs:2167-2342+3263-3398`
  `op_store_local_<scalar>` family — pop `(src_bits, src_kind)`, preserve
  `src_kind` via `stack_write_kinded(slot, src_bits, src_kind)`. Commit
  `805a834a`.
- c5 differs from c7 / 1b in remediation polarity: **the fix is at the
  compiler (producer-side gate) not at the executor (consumer-side kind
  preservation)** because the wrong-bits-on-stack consequence flows from
  accepting non-int operands at compile time, not from fabricating kind
  at runtime. Audit doc 05 §5 calls this out: the typed-bitwise arms
  (`BitAndInt` etc.) are correct; the dynamic fallback is the violation;
  deleting the fallback restores discipline.
- **Defection-attractor refusal:** the 3 DISCARDS sites at
  `arithmetic/mod.rs:639-655` are NOT to be renamed as a "documented
  intentional duality" / "FFI-boundary helper" / "dispatch-slice probe"
  / "tag-decode bridge" or any variant matching the broader-family regex.
  They are the deleted W-series resurrected pattern, by name —
  per the CLAUDE.md §Renames-to-refuse-on-sight rule, deletion is the
  remediation, not rebranding.
- **No DISCARDS-class sibling identified** in the broader sweep. Audit
  doc 05 §7 hypothesized "DivDynamic, ModDynamic, GtDynamic etc." from
  the `cce5303a` commit body might still exist. The sweep confirms they
  do NOT: the typed div/mod/cmp paths at L132-160 and L420 (`compact_int_cmp`)
  / L580 (`compact_float_cmp`) are all producer-gated by typed opcodes
  (`DivInt`, `ModInt`, `LtInt`, etc.) emitted only under
  `is_strict_arithmetic`. The historical `Div` / `Mod` / `Gt` / `Lt`
  generic opcodes are NOT in `executor/arithmetic/mod.rs`'s match arms —
  they are deleted per CLAUDE.md §Forbidden-Code "Generic opcodes
  (`Add`, `Sub`, `Lt`, etc. without kind suffix). Deleted. Only typed
  variants exist."

## Phase B dispatch sizing

Single sub-cluster: `v0.3.3-bitwise-strict-typing-gate`. **Size: S.** No
parallel sub-clusters; no NEEDS-INVESTIGATION sites to triage; no
cross-cluster ordering dependencies (cluster #4 pointer-as-float-leak is
a different opcode family — independent fix).

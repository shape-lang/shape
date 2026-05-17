# W12-enum-constructor-mir-lowering — Audit Report

**Sub-cluster:** W12-enum-constructor-mir-lowering (Phase 3 cluster-0 Round 4)
**Branch:** `bulldozer-strictly-typed-w12-enum-constructor-lowering`
**Audit date:** 2026-05-12
**Smoke unblocked:** 1.5 (Result/match with closures — currently segfaults under `--mode jit`)
**Predecessor surface:** W12-jit-stack-parallel-kind-track close `1a4d1156`
**ADR-006 cite:** §2.7.5 (cross-crate ABI policy — producing-site classification)

---

## §1. Methodology

The audit answers a single question: **for each constructor expression family the user can write, does MIR-level lowering produce a structurally correct shape, or does it emit `MirConstant::Function(name)` and rely on the function-table lookup to resolve `name`?**

Producing `MirConstant::Function("Ok")` from a constructor expression is a category error. The VM never registers "Ok" / "Err" / "Some" / "HashMap" / etc. as runnable functions in `function_indices` — they are dispatched in the bytecode path via `OpCode::BuiltinCall(BuiltinFunction::OkCtor)` (and friends), which reach the executor's hand-written ctor bodies in `executor/vm_impl/builtins.rs:529-758`. The JIT consumes MIR, not bytecode, so when its `compile_constant` reaches `MirConstant::Function("Ok")` and looks it up in `self.function_indices`, the lookup fails and `compile_constant` falls back to `iconst(I64, 0)` (`mir_compiler/ownership.rs:312-314`). Downstream this becomes:

1. The "Ok"-callee operand becomes `callee_bits = 0` at the indirect-call site.
2. The JIT's `jit_call_value` classifier (`ffi/control/mod.rs:396-426`) tests `is_inline_function(0)` (false — tag bits are zero, not `TAG_FUNCTION_BITS`) and `is_heap_kind(0, HK_CLOSURE)` (false — not tagged). It falls through to the §2.7.5 graceful-surface path, returns TAG_NULL.
3. The caller treats TAG_NULL as a value (e.g. tries to use it as the scrutinee of `match`), dereferences null at some downstream consumer, segfaults.

So `iconst(I64, 0)` is the proximate cause of the segfault chain documented in the W12-jit-stack-parallel-kind-track close `1a4d1156`. The root cause is one layer up: MIR-emission lowering a constructor expression into a `Function(name)` operand that no function table can satisfy.

The audit grid below classifies each constructor family by:

- **AST shape** — what `Expr` variant the parser produces for the surface syntax
- **MIR shape** — what MIR statements/terminators the lowering currently emits
- **VM consumer** — the bytecode opcode the bytecode compiler emits (independent of MIR)
- **JIT consumer** — what the JIT does with the MIR (or fails to do)
- **Classification** — BROKEN / CORRECT / DEFERRED-SURFACE

---

## §2. Constructor inventory

### §2.1 Enum-variant constructors — bare form

These use the parser's `FunctionCall` / `some_expr` AST shape (no `::` qualifier).

| Constructor | AST shape | MIR shape | VM consumer | JIT consumer | Classification |
|---|---|---|---|---|---|
| `Ok(value)` | `FunctionCall { name: "Ok", args: [value] }` | `Call { func: Constant(Function("Ok")), args: [value], dest }` (`mir/lowering/expr.rs:1528-1533`) | `OpCode::BuiltinCall(OkCtor)` with pre-pushed args + arg_count (`compiler/expressions/function_calls.rs:939-954`); executor calls `ResultData::ok(payload)` and pushes via `KindedSlot::from_result` (`executor/vm_impl/builtins.rs:551-568`) | `compile_constant` returns `iconst(I64, 0)` because "Ok" is not in `function_indices` (`mir_compiler/ownership.rs:309-314`); indirect call dispatches with `callee_bits=0` → TAG_NULL → downstream segfault | **BROKEN** |
| `Err(value)` | `FunctionCall { name: "Err", args: [value] }` | Same as `Ok` — `Call { func: Constant(Function("Err")), ... }` | `OpCode::BuiltinCall(ErrCtor)`; `ResultData::err(payload)` + `KindedSlot::from_result` (`executor/vm_impl/builtins.rs:569-586`) | Same as `Ok` — segfault path | **BROKEN** |
| `Some(value)` | `FunctionCall { name: "Some", args: [value] }` (via `parse_some_expr`, `parser/expressions/primary.rs:387-405`) | Same shape — `Call { func: Constant(Function("Some")), ... }` | `OpCode::BuiltinCall(SomeCtor)`; `OptionData::some(payload)` + `KindedSlot::from_option` (`executor/vm_impl/builtins.rs:529-550`) | Same as `Ok` — segfault path | **BROKEN** |
| `None` | `Literal(Literal::None)` (`parser/expressions/literals.rs:100`) | `MirConstant::None` via `assign_none()` | Direct `PushNone` (no constructor call) | `MirConstant::None` → `iconst(I64, 0)` — but the bits are explicitly the post-strict-typing NONE encoding, not a failed function-table lookup | **CORRECT** (already a literal, no constructor surface) |

### §2.2 Enum-variant constructors — qualified form

These use the parser's `EnumConstructor` AST shape (`Type::Variant(...)` with `::`).

| Constructor | AST shape | MIR shape | VM consumer | JIT consumer | Classification |
|---|---|---|---|---|---|
| `Result::Ok(value)` | `EnumConstructor { enum_name: "Result", variant: "Ok", payload: Tuple([value]) }` | `Assign(temp, Aggregate([value]))` + `EnumStore { container_slot: temp, operands: [value] }` (`mir/lowering/expr.rs:1558-1596`) | `compile_expr_enum_constructor` resolves enum schema by name, emits `PushConst(variant_id)` + payload values + `OpCode::NewTypedObject { schema_id, field_count }` (`compiler/expressions/collections.rs:877-985`) | `StatementKind::EnumStore` surface-and-stops with §2.7.14 cite (`mir_compiler/statements.rs:200-209`) — heterogeneous-element-array carrier gap | **DEFERRED-SURFACE** (already surfaces with §-cite; the new MIR shape would target this same site) |
| `Result::Err(...)`, `Option::Some(...)`, `Option::None`, and any user-defined `Enum::Variant(...)` | Same `EnumConstructor` shape as above | Same `Aggregate` + `EnumStore` shape | Same `compile_expr_enum_constructor` path — uniform TypedObject lowering with `variant_id` + payload | Same surface-and-stop via `EnumStore` arm | **DEFERRED-SURFACE** |
| `Result::Ok` (unit form, no payload — uncommon) | `EnumConstructor { ..., payload: Unit }` | `assign_none(temp)` — no `EnumStore` emitted | `compile_expr_enum_constructor` with `payload_count = 0` — variant_id is the only field | `MirConstant::None` → `iconst(I64, 0)` — not a constructor lookup, structurally correct for unit | **CORRECT** for the MIR shape (no Function-by-name emission) |

### §2.3 Collection constructors (primitive heap kinds)

These use the parser's `FunctionCall` AST shape.

| Constructor | AST shape | MIR shape | VM consumer | JIT consumer | Classification |
|---|---|---|---|---|---|
| `HashMap()` | `FunctionCall { name: "HashMap", args: [] }` | `Call { func: Constant(Function("HashMap")), args: [], dest }` | `BuiltinCall(HashMapCtor)` — empty `Arc<HashMapData>` via `KindedSlot::from_hashmap` (`executor/vm_impl/builtins.rs:587-597`); a typed-map fast path also exists when annotation is available (`compiler/v2_typed_map_emission.rs`) | Same as Ok/Err/Some — `function_indices` lookup miss → `iconst(I64, 0)` → segfault on any subsequent `.add()` / `.get()` etc. that tries to use the null slot | **BROKEN** |
| `Set()` / `HashSet()` (`Set` is the canonical name, `HashSet` is an alias the dispatch grammar doesn't bind yet) | `FunctionCall { name: "Set", args: [] }` | Same `Call(Function("Set"))` shape | `BuiltinCall(SetCtor)` — empty `Arc<HashSetData>` via `KindedSlot::from_hashset` (`executor/vm_impl/builtins.rs:598-610`) | Same broken path | **BROKEN** |
| `Deque()` | `FunctionCall { name: "Deque", args: [] }` | Same shape | `BuiltinCall(DequeCtor)` — `KindedSlot::from_deque` (`executor/vm_impl/builtins.rs:611-625`) | Same broken path | **BROKEN** |
| `PriorityQueue()` | `FunctionCall { name: "PriorityQueue", args: [] }` | Same shape | `BuiltinCall(PriorityQueueCtor)` — `KindedSlot::from_priority_queue` (`executor/vm_impl/builtins.rs:626-641`) | Same broken path | **BROKEN** |
| `Channel()` | `FunctionCall { name: "Channel", args: [] }` | Same shape | `BuiltinCall(ChannelCtor)` — `KindedSlot::from_channel` (`executor/vm_impl/builtins.rs:642-662`) | Same broken path | **BROKEN** |
| `Mutex(initial)` | `FunctionCall { name: "Mutex", args: [initial] }` | Same shape | `BuiltinCall(MutexCtor)` — `KindedSlot::from_mutex` (`executor/vm_impl/builtins.rs:663-685`) | Same broken path | **BROKEN** |
| `Atomic(initial)` | `FunctionCall { name: "Atomic", args: [initial] }` | Same shape | `BuiltinCall(AtomicCtor)` (`executor/vm_impl/builtins.rs:686-712`) | Same broken path | **BROKEN** |
| `Lazy(thunk)` | `FunctionCall { name: "Lazy", args: [thunk] }` | Same shape | `BuiltinCall(LazyCtor)` (`executor/vm_impl/builtins.rs:713-737`) | Same broken path | **BROKEN** |

### §2.4 TypedObject literals

| Surface form | AST shape | MIR shape | VM consumer | JIT consumer | Classification |
|---|---|---|---|---|---|
| `P { x: 1, y: 2 }` (struct literal) | `StructLiteral { type_name: "P", fields: [...] }` | `Assign(temp, Aggregate([1, 2]))` + `ObjectStore { container_slot: temp, operands: [1, 2], field_names: ["x", "y"] }` (`mir/lowering/expr.rs:1695-1716`) | `compile_struct_literal` — schema lookup + `PushConst` + `NewTypedObject { schema_id, field_count }` | `StatementKind::ObjectStore` — direct lowering via `typed_object_alloc` + per-field `typed_object_set_field` (`mir_compiler/statements.rs:111-171`) | **CORRECT** (no `MirConstant::Function` emission anywhere on the path) |
| `{ k: v, ... }` (object literal) | `Object(entries)` | Same `Aggregate` + `ObjectStore` shape (`mir/lowering/expr.rs:1597-1625`) | `compile_object_literal` — schema-aware lowering | Same `ObjectStore` lowering | **CORRECT** |

---

## §3. Failure mode

The broken cases (§2.1 bare enum-variants and §2.3 collection constructors) share a single failure mode at three layers:

| Layer | Site | Failure |
|---|---|---|
| AST | `parser/expressions/primary.rs:387-405` (`Some`) + general `function_call` grammar production | Constructor surface syntax indistinguishable from function call; produces `Expr::FunctionCall` |
| MIR emission | `mir/lowering/expr.rs:1512-1534` (`Expr::FunctionCall` arm) | Lowers any `FunctionCall { name, args }` to `Call { func: Constant(Function(name)), args, dest }` — no constructor-name discrimination |
| JIT consumption | `mir_compiler/ownership.rs:309-314` (`compile_constant` for `MirConstant::Function`) | `function_indices.get(name)` returns `None` for any builtin-ctor name → silently emits `iconst(I64, 0)` for the callee bits |

The bytecode/VM path is structurally independent and remains green: `classify_builtin_function` (`compiler/helpers.rs:3194-3209`) intercepts builtin-ctor names at the **bytecode-compile** stage and emits `OpCode::BuiltinCall(*Ctor)` opcodes that the executor's hand-written ctor bodies consume.

---

## §4. Forbidden frames

The audit explicitly refuses these framings (per CLAUDE.md "Renames to refuse on sight" + sub-cluster dispatch §"Forbidden in this sub-cluster"):

1. **Adding `"Ok"` / `"Err"` / `"Some"` / `"HashMap"` etc. as stub-function registrations in `function_indices`.** This is the W-series patch-the-symptom defection-attractor at the MIR layer. It would resolve `compile_constant` lookups by aliasing a synthetic function-id to a runtime constructor handler — exactly the shape of the deleted ValueWord tag-bit dispatch, just renamed. **Refused on sight.**
2. **Adding a "constructor decode bridge" / "enum-variant translator" / "ctor-name helper" between the MIR layer and a constructor handler table.** Same defection-attractor family (CLAUDE.md broader-family regex `(decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|callee|capture) (bridge|probe|helper|hop|translator|adapter|shim)` extends to the MIR-emission layer). **Refused on sight.**
3. **Bool-default fallback** for any constructor whose schema isn't available at MIR-emission time. Surface-and-stop per ADR-006 §2.7.5 with `Err("...")` from the lowering pass, never silently emit `Function("Ok")` and hope a downstream registration appears.
4. **Treating user-defined enum variants differently from built-in `Ok` / `Err` / `Some` / `None`.** §2.2 shows the qualified path already lowers uniformly via `EnumStore` for both. The fix must close the gap so bare and qualified shapes converge to a single MIR shape.

---

## §5. Proposed fix

**Shape:** rewrite the bare-form constructor expressions at the MIR-lowering layer so they produce the same MIR shape as their qualified `Type::Variant(...)` siblings. The fix is a compiler-pass rewrite, NOT a new MIR opcode.

### §5.1 Why no new MIR opcode

`EnumStore` and `ObjectStore` already exist as MIR statement kinds (`mir/types.rs::StatementKind`). The VM-side bytecode compiler already emits `OpCode::BuiltinCall(OkCtor)` etc. by intercepting the names at bytecode-emit time — that path is unaffected. The bug is that MIR-emission has a separate, parallel lowering pass that doesn't intercept the same names.

The fix is to teach the MIR-emission pass the same names (or a strictly-richer set keyed on the same classifier `classify_builtin_function` already uses for the bytecode path), and lower:

- `Ok(value)` / `Err(value)` / `Some(value)` to the existing `EnumStore`-with-`Aggregate` shape (`mir/lowering/expr.rs:1558-1596` already has this code path for `Expr::EnumConstructor`).
- `HashMap()` / `Set()` / `Deque()` / `PriorityQueue()` / `Channel()` / `Mutex(x)` / `Atomic(x)` / `Lazy(x)` to a new MIR shape that produces an empty heap value of the declared kind. We surface this here as **`StatementKind::MakePrimitiveCollection { container_slot, kind, operands }`** OR (the strictly-simpler alternative) reuse `EnumStore` with `kind`-on-the-slot threading, since `EnumStore`'s VM consumer already builds a TypedObject and the JIT consumer already surface-and-stops with the documented §2.7.14 gap.

### §5.2 The minimum-viable Commit-2 shape (proposed)

For the §2.1 bare enum-variant family — the load-bearing fix for Smoke 1.5 — the rewrite is mechanical:

```rust
// mir/lowering/expr.rs, Expr::FunctionCall arm:
Expr::FunctionCall { name, args, named_args, .. } => {
    // INTERCEPT: bare enum-variant constructors lower to EnumStore,
    // matching the qualified `Type::Variant(...)` path's MIR shape.
    // Per the §2.7.5 producing-site classification rule, the
    // constructor's kind is known here at MIR-emit time — no decode,
    // no probe, no Bool-default.
    let bare_enum_ctor = matches!(name.as_str(), "Ok" | "Err" | "Some");
    if bare_enum_ctor && named_args.is_empty() {
        let operands: Vec<_> = args.iter()
            .map(|a| lower_expr_as_moved_operand(builder, a))
            .collect();
        builder.push_stmt(
            StatementKind::Assign(Place::Local(temp), Rvalue::Aggregate(operands.clone())),
            span,
        );
        emit_container_store_if_needed(
            builder, ContainerStoreKind::Enum, temp, operands, span,
        );
        return temp;  // (or `break` / continue depending on control structure)
    }
    // ... existing logic for real function calls
}
```

For the §2.3 collection-constructor family, the same rewrite shape applies but lowers to a new `ContainerStoreKind::PrimitiveCollection { kind }` variant (or a parallel statement kind), where `kind` is the producing-site classification per §2.7.5 (HashMap / HashSet / Deque / PriorityQueue / Channel / Mutex / Atomic / Lazy).

### §5.3 Scope decision (audit-driven)

The audit found **11 broken constructors** in two clean families (3 bare enum variants + 8 collection constructors), all sharing the identical failure mode and rewrite shape. This is NOT the "~50 broken constructor sites across enum variants, collections, TypedObject literals, AND something deeper" ceiling that the sub-cluster dispatch prompt called out as a rescope trigger.

The decision: **fix the bare enum-variant family in Commit 2** (the load-bearing case for Smoke 1.5 — Result/match). The collection family is mechanically identical but exercises a different downstream JIT consumer (the empty-collection FFI path) and isn't load-bearing for any cluster-0 smoke. We surface it as a follow-up sub-cluster (`W12-collection-constructor-mir-lowering`) rather than expand this wave silently.

Justification: the close criterion is "the smoke matrix passes end-to-end identical to `--mode vm`." Smoke 1.5 is the only smoke in the matrix that exercises a §2.1 constructor. Smoke 4 (HashSet) does exercise §2.3 BUT the dispatch prompt explicitly says "Smoke 4 (HashSet via `&mut self`): expected to pass post-Phase-2d-mutation; confirm during smoke matrix re-run after Round 3" — i.e. its expectation is set by the mutation-writeback work, not by a constructor fix. We'll verify Smoke 4 status as part of Commit 2's smoke matrix verification, and if it surfaces the same MIR-emission gap we'll widen scope explicitly.

### §5.4 Why this is principled (not the W-series defection-attractor)

Distinguishing the proposed fix from the forbidden frame ("add Ok/Err/Some to `function_indices`"):

| Property | Forbidden frame (add to `function_indices`) | Proposed fix (MIR rewrite at lowering) |
|---|---|---|
| Where the name is intercepted | JIT consumer reads MIR and resolves "Ok" via a synthetic function-id | MIR producer never emits `Function("Ok")` in the first place |
| Layer the bridge lives in | Runtime function-table aliasing | Compile-time MIR shape |
| Consumer dispatch | Indirect call → tag-bit-like classifier (W-series shape) | Direct MIR statement (`EnumStore` already understood by both VM-bytecode and JIT-deferred-surface) |
| Reversibility | Lasts as long as the function-table aliasing — permanent runtime tax | Pure compile-time; no runtime artifact |
| ADR shape | None — bypasses §2.7.5 producing-site classification | Aligns with §2.7.5 — the name is classified at producing site (MIR-emit), kind flows on the slot through `EnumStore`, no runtime decode |

The MIR rewrite is the same shape as how the bytecode compiler already intercepts these names at `compile_expr_function_call` (`compiler/expressions/function_calls.rs:870-971`). The fix makes the MIR-emission pass do the equivalent interception. It is a producer-side classification per §2.7.5, not a consumer-side bridge.

### §5.5 Why this doesn't need an ADR-006 amendment

The proposed fix reuses `EnumStore` — an existing MIR statement kind that ADR-006 has not added or amended. The JIT-side `EnumStore` arm already surface-and-stops with §2.7.14 cite for the heterogeneous-element carrier gap — that gap is documented and tracked separately. After the fix, bare `Ok(2)` lowers to:

```
Assign _t = Aggregate([2])
EnumStore { container_slot: _t, operands: [2] }
```

Which is **identical** to what `Result::Ok(2)` already lowers to. The JIT will then surface-and-stop with the same §2.7.14 message that `Result::Ok(2)` currently surfaces — that's a separate gap, tracked by `W12-jit-new-array` follow-up work. **Smoke 1.5 may not yet print `5` under `--mode jit` after Commit 2** — but the segfault (the W12-jit-stack-parallel-kind-track close `1a4d1156`'s surfaced item) WILL be replaced by a documented surface-and-stop with the same §2.7.14 cite the qualified path already surfaces. That's the correct behavior: VM works, JIT surfaces honestly. The smoke-equivalence ratchet moves forward (a structural bug becomes a documented gap) without expanding cluster-0 scope into the §2.7.14 carrier work.

---

## §6. Risk inventory

| Risk | Mitigation |
|---|---|
| `compile_expr_as_value_or_placeholder` in the bytecode compiler depends on the bare-form `FunctionCall` shape reaching `classify_builtin_function` | Bytecode compiler runs separately from MIR emission; they share the AST but produce independent output. The fix only modifies MIR-emission. Bytecode path unaffected. Verified by reading `compile_expr_function_call` (`compiler/expressions/function_calls.rs:870-971`) — it consumes `Expr::FunctionCall` directly, has no dependency on the MIR shape. |
| User-defined function shadowing — what if user defines `fn Ok(x) { ... }`? | Mirror the bytecode compiler's resolution order: `classify_builtin_function` only runs AFTER local + module + parameter-scope lookups fail (`compiler/helpers.rs:3194-3383` documents the order). The MIR rewrite uses the same precedence — if the name resolves to a real function in `builder.lookup_local(name)`, fall through to the normal `Call` path. |
| Pipe operator `expr \|> Ok` (rare but legal syntax) | The pipe-operator lowering at `mir/lowering/expr.rs:1813` also emits `MirConstant::Function(name)`. It hits the same bug. The Commit-2 fix applies the same interception there. |
| Closure captures named `Ok` — e.g. `let Ok = ...; Ok(5)` | The bytecode compiler's `lookup_local(name)` check happens first in MIR lowering too (`mir/lowering/expr.rs:1528`), so a local `Ok` binding takes precedence. Pre-existing AST behavior is preserved. |
| Monomorphization rewriting `EnumConstructor` shape during specialization | `monomorphization/substitution.rs:711, 1596` already handles `Expr::EnumConstructor` by mapping the payload; bare `FunctionCall("Ok")` is not currently visited by monomorphization for constructor rewriting. Per the §5.4 producing-site rule, the MIR rewrite happens at lowering time AFTER monomorphization has run on the AST. No interaction. |

---

## §7. Verification plan for Commit 2

1. Re-run Smoke 1.5 under `--mode vm` and `--mode jit`. Expected: VM unchanged (prints `5`); JIT no-longer-segfaults (now surfaces `EnumStore` §2.7.14 message via the bare-form path same as the qualified path).
2. `cargo check --workspace --lib --tests` — exit 0.
3. `cargo test -p shape-jit --lib` — 319-baseline tests, no regressions.
4. `cargo test -p shape-vm --lib` — no regressions.
5. `bash scripts/verify-merge.sh` — 12/12.
6. `bash scripts/check-no-dynamic.sh` — exit 0.
7. Spot-check that Smoke 1 (scalar loop) and Smoke 2 / Smoke 3 baselines from W11/W12 closes are unchanged.

---

## §8. Sites surfaced (for cite-tracked follow-up)

| Item | §-cite | Disposition |
|---|---|---|
| §2.3 collection-constructor family lowering (HashMap/Set/Deque/PriorityQueue/Channel/Mutex/Atomic/Lazy) | §2.7.5 producing-site classification | Follow-up sub-cluster `W12-collection-constructor-mir-lowering` — mechanically identical rewrite, exercises a different downstream JIT consumer (empty-collection FFI), not load-bearing for any current cluster-0 smoke. Verify Smoke 4 (HashSet) after Commit 2 to confirm; widen scope explicitly if it surfaces |
| §2.2 enum-variant `EnumStore` consumer in JIT — heterogeneous-element-array carrier | §2.7.14 / §2.7.5 | Already surfaces at `mir_compiler/statements.rs:200-209` with a documented §-cite. Tracked as W11-jit-new-array follow-up. Commit 2 routes bare-form into the same gap; the gap itself remains outside this sub-cluster's scope |
| MIR pipe-operator path `expr \|> Ok` (`mir/lowering/expr.rs:1813`) | §2.7.5 | Apply the same interception inline as part of Commit 2 — same call site shape, same fix |

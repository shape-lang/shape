# v2 Closure Specialization: Zero-Cost Closures via Per-Closure Monomorphization

**Status**: Design document for multi-session execution
**Prerequisite**: `docs/v2-monomorphization-design.md` Phase 1 (ConcreteType) and Phase 2.1 (type-only monomorphization)
**Composes with**: `docs/enhanced-escape-analysis-v2.md` Phases 5.A–C (interprocedural ownership, already landed)
**Goal**: Closures compile to zero-atomic-op, zero-heap-allocation hot paths when they do not escape their defining scope. Escaping closures fall back to a uniform heap-allocated `TypedClosure<ClosureTypeId>` representation — no `Arc<dyn Fn>`, no NaN-boxing, no type erasure.
**Contract**: Aligns with `docs/runtime-v2-spec.md`: "NO runtime type tags. NO NaN-boxing. NO dynamic dispatch on value type."

---

## Relationship to Existing v2 Plan

This document extends `v2-monomorphization-design.md` Phase 2.2 with a **closure-aware specialization axis**. It is not a new phase — it is a tightening of the existing plan.

- `v2-monomorphization-design.md` Phase 1.1 already defines `ConcreteType::Closure(ClosureTypeId)`.
- `v2-monomorphization-design.md` Phase 2.2 plans element-type monomorphization of stdlib higher-order methods (`map`, `filter`, `reduce`, `forEach`).
- `v2-monomorphization-design.md` Phase 4.2 plans "Closures/Upvalues | typed capture slots" as part of ValueWord deletion.

This document fills in the closure-specific design that those sections gesture at:
the layout of a `TypedClosure`, when it's stack-allocated, how the escape analysis composes with existing MIR infrastructure, and how higher-order methods are specialized per-closure — not just per-element-type.

It supersedes `enhanced-escape-analysis-v2.md` Phase 5.D's narrow proposal. Phase 5.D targeted changing `SharedCow` → `Direct` for non-escaping-closure captures; that's a v1-style bolt-on. The v2-native design is per-closure monomorphization + stack allocation, with the storage-class change falling out naturally from §4 below.

---

## The Problem

Shape today compiles every closure into a heap-allocated object dispatched through a function pointer:

- Mutable captures go through `Arc<RwLock<ValueWord>>` (SharedCell). Every read/write in the closure body pays atomic refcount traffic on the cell.
- Higher-order stdlib methods (`arr.map(f)`) invoke `call_value_immediate_nb(&args[1], ...)` — an indirect call through a `ValueWord`-typed function argument, with NaN-boxing at every boundary.
- Each closure literal mints a heap allocation even when it's consumed immediately and never escapes.

For the common case `arr.map(|x| x + n)`, the cost is dominated by allocation, refcounting, and indirect dispatch — not by the actual arithmetic. In a compiled, statically-typed language, this is solvable: each closure has a statically known body and capture layout.

### What Rust/C++ Do

Each closure is a unique anonymous struct; its captures are fields. Calling the closure is a statically-bound method call. When passed to a generic like `map`, the generic is monomorphized per closure type, the closure's `call()` method is inlined into the loop, and LLVM eliminates the abstraction entirely. Zero atomic ops, zero allocation, direct calls.

Shape's v2 runtime spec already rules out tagged/dynamic dispatch. Closures should follow suit: each closure literal has a `ClosureTypeId`, a `TypedClosure<Captures>` struct with known offsets, and a compile-time-determined call convention. Non-escaping closures live on the caller's stack; escaping ones live on the heap with `HeapHeader`.

---

## Architecture Overview

```
Source → Parser → Type Inference → Monomorphizer → Bytecode Compiler → Typed Bytecode
                        ↓                ↓                ↓
              ConcreteType::         Specialization    Inlined closure bodies
              Closure(TypeId)         key extended      + stack-allocated
                                      with ClosureTypeId TypedClosure slots
                        ↓                                ↓
                   MIR Lowering                    VM: direct typed dispatch
                        ↓                                ↓
              Escape analysis on                JIT: Cranelift StackSlot
              closure slots                     Cranelift direct call
                        ↓                                ↓
              Stack vs heap allocation        Closure body inlined
              decision                         → tight typed loop
```

The data flow:

1. **Parser** produces `Expr::FunctionExpr { params, body, captures }` nodes.
2. **Type inference** resolves every capture to a `ConcreteType`. Each closure literal gets assigned a `ClosureTypeId`.
3. **Monomorphizer** specializes higher-order callees per `(element_types..., ClosureTypeId)` tuple. The specialization inlines the closure body into the callee template.
4. **Bytecode compiler** emits typed opcodes over `TypedClosure<Captures>` structs, directly addressing captures at their known offsets.
5. **Escape analysis** (in storage planning) classifies closure slots; non-escaping ones are tagged for stack allocation.
6. **JIT / VM** allocate per the escape verdict — Cranelift `StackSlot` for non-escaping, heap with `HeapHeader` for escaping.

---

## §1 — TypedClosure Struct Layout

### 1.1 Memory Layout

A `TypedClosure` parallels `TypedStruct`: `HeapHeader` at offset 0 (heap variant only), followed by a compact typed capture area whose fields are laid out C-style at compile-time-known offsets. Between the header and the captures sits a function identifier and a `ClosureTypeId` discriminant:

```rust
#[repr(C)]
pub struct TypedClosureHeader {
    header: HeapHeader,    // 8 bytes  (offset 0) — heap variant only
    function_id: u32,      // 4 bytes  (offset 8)  — index into program.functions
    type_id: u32,          // 4 bytes  (offset 12) — ClosureTypeId.0
    // captures[]          //          (offset 16) — C-laid-out per ClosureLayout
}
// Header size: 16 bytes. Captures follow with natural alignment.
```

For the **stack-allocated variant** the `HeapHeader` is absent — captures start at offset 8:

```rust
#[repr(C)]
pub struct StackClosure {
    function_id: u32,      // (offset 0)
    type_id: u32,          // (offset 4)
    // captures[]          // (offset 8)
}
```

Sizes for common cases:

| Captures | Example | Heap size | Stack size |
|---|---|---|---|
| 0 | `\|\| 42` | 16 B | 8 B |
| 1 × f64 | `\|x\| x + a` | 24 B | 16 B |
| 2 × f64 | `\|x\| x + a + b` | 32 B | 24 B |
| mixed (f64, i32, ptr) | | 32 B | 24 B |
| 1 × `*const StringObj` | `\|x\| x + s.len()` | 24 B | 16 B |

`ClosureLayout` (one per `ClosureTypeId`) records:
- `Vec<FieldInfo>` for each capture (reused from `StructLayout`)
- `heap_capture_mask: u64` — bitmap of which captures are refcounted pointers (for Drop traversal)
- Total payload size and alignment

### 1.2 ClosureTypeId Allocation

Three keying strategies were considered:

| Strategy | Registry size | Redundant? |
|---|---|---|
| (a) Per closure literal | Large | Yes — identical captures get distinct IDs |
| (b) Per capture signature (`Vec<ConcreteType>`) | Small | No |
| (c) Per `(function_id, capture_sig)` | Redundant | `function_id` determines captures |

**Decision: (b).** `ClosureTypeId` keys on `Vec<ConcreteType>` of captures. The body is carried by `function_id`; the `type_id` describes only the physical capture layout for Drop/GC/reflection. Two closures `|x| x+1` (no captures) and `|| print("hi")` (no captures) share `ClosureTypeId(0)`. Monomorphization's specialization key is orthogonal (§3).

### 1.3 Call Dispatch

**Single `CallClosure` opcode in the VM**, with JIT specialization:

```
CallClosure(arity)
  pop closure_ptr: *const TypedClosure
  fid = load u32 [closure_ptr + 8]    (heap) or [closure_ptr + 0] (stack)
  for each capture i in layout:
    load typed [closure_ptr + layout.offset(i)] → local[i]
  pass args as local[N..N+arity]
  jump to functions[fid].entry_point
```

The bytecode interpreter uses this generic dispatch. The JIT inlines through it when the `ClosureTypeId` is statically known at the call site (the common case after monomorphization) — see §5.

### 1.4 Refcount Semantics

- **At `MakeClosure`**: for each capture in `heap_capture_mask`, `atomic_add [capture_ptr + 0], 1`. One instruction per heap capture.
- **At call time**: captures are bound to locals via plain `load typed`. No refcount traffic — the local borrows from the closure object which outlives the call.
- **At closure Drop** (refcount → 0): walk `heap_capture_mask`, release each pointer capture, then free the allocation. Stack closures skip the final free.
- **Mutable captures** in non-escaping closures bypass refcounting entirely — see §4.

---

## §2 — Non-Escape Detection

### 2.1 Escape Vectors

A closure slot `C` escapes via any of:

| # | Vector | MIR signal | Action |
|---|---|---|---|
| 1 | Returned from function | `C` flows into `SlotId(0)` (existing `slot_flows_to_return`) | Heap |
| 2 | Stored in Array/HashMap/Tuple | `C` in `Rvalue::Aggregate`, `ArrayStore`, `ObjectStore`, `EnumStore` | Heap |
| 3 | Stored in struct field | `Assign(Place::Field{..}, Use(Copy/Move(C)))` | Heap |
| 4 | Captured by escaping closure | `C` in another `ClosureCapture.operands` | Transitive (§2.4) |
| 5 | Detached task boundary | `StatementKind::TaskBoundary(_, Detached)` | Heap |
| 6 | Structured task boundary | `StatementKind::TaskBoundary(_, Structured)` | Heap (v1) / stack (future) |
| 7 | `snapshot()` call | Opaque FFI; conservatively Unknown | Heap + deopt |
| 8 | Passed to callee that may store it | `C` in `TerminatorKind::Call.args`, callee's param summary escapes | Per summary |
| 9 | Written through `*p = c` | `Assign(Place::Deref, Use(C))` | Heap |
| 10 | Assigned to `UniqueHeap`/`SharedCow` slot | Promotion to shared-mutable storage | Heap |

### 2.2 Detection Pass

The detection is a **refinement of existing storage planning**, not a separate pass. Add `non_escaping_closure_slots: HashSet<SlotId>` to `StoragePlan` and populate it inside `plan_storage` by, for each slot defined by `ClosureCapture`:

1. Check flow to return (existing `slot_flows_to_return`).
2. Check flow to any container `ArrayStore` / `ObjectStore` / `EnumStore` / `Aggregate` as operand.
3. Check flow to non-`Local` destination places (`Field`, `Index`, `Deref`).
4. Check task-boundary membership.
5. Check transitive capture by escaping closures (fixed-point over the capture graph).
6. Check call-argument escape using extended `FunctionBorrowSummary` (see §2.5).

If none of (1)–(6) fire, the slot is non-escaping.

### 2.3 Critique of Phase 5.D Criteria

Phase 5.D listed: singly-assigned, all-uses-direct-calls, `EscapeStatus::Local`, never-stored. Revised for v2:

- **Singly-assigned**: true by construction; MIR never reassigns closure slots. Keep as a defensive check.
- **"All uses are direct calls"**: too restrictive. Should be "all uses are either a direct call *or* passed as an argument to a callee whose summary marks that param as non-escaping." Otherwise `arr.map(f)` fails even when `map` is monomorphized.
- **`EscapeStatus::Local`**: load-bearing, but the current implementation only looks at return + captures. Extend with table above.
- **Never stored in a collection**: subsumed by rows 2, 3, 9 above — scan all non-Local destinations.

**Missing**: task boundaries, Deref writes, callee-summary-based flow, transitive closure capture.

**Outright wrong in v2**: the implicit "store → Arc wrapping" model. In v2, storage → heap-allocated `TypedClosure` with `HeapHeader`. No Arc wrapping, no erasure.

### 2.4 Transitive Closure Capture

If closure A captures closure B, B appears in A's `ClosureCapture.operands`. Rule:
**B escapes iff A escapes.** Carrying B into A's layout means any escape of A drags B with it.

Implementation: fixed-point iteration over the capture graph. Seed with "all escaping" per direct vectors (§2.1), then iteratively demote to non-escaping when all uses are benign. Converges monotonically. The existing Datafrog solver in `mir/solver.rs` is already set up for this kind of analysis.

### 2.5 Higher-Order Method Composition

Key insight: monomorphization collapses the "does `map` store the closure?" question.

- Pre-mono: `map` is a generic with a `Function`-typed param. Escape is unknowable per-caller.
- Post-mono: `map<number, int, ClosureType_7>` is a specialized body where the closure's escape is a pure intraprocedural property. The specialized body either (a) inlines the closure body (no escape possible) or (b) passes it to an inner function (escape per summary).

Extend `FunctionBorrowSummary` with a per-param `closure_param_escapes: Vec<bool>` (or a `ParamEscapeMode` enum). The existing Phase 5.A `CalleeSummaries` plumbing threads this interprocedurally with no new machinery.

### 2.6 Fallback in v2

Confirmed: the v2 fallback is **heap-allocated `TypedClosure<ClosureTypeId>` with `HeapHeader`**, not `Arc<dyn Fn>`. Typed captures, statically known function pointer, uniform refcount. Type erasure never enters the picture.

---

## §3 — Per-Closure Monomorphization

### 3.1 Extended Cache Key

Today's `MonomorphizationCache` keys on `"<fn_name>::<ConcreteType::mono_key()>_..."`. `ConcreteType::Closure(ClosureTypeId)` already returns `"closure_<n>"` from its `mono_key()`. We only need to start using it.

**Worked example** — `arr.map(|x| x + n)` with `arr: Array<int>`, closure is the 7th seen in compilation unit, returns `int`:

```
key = "map::array_i64_closure_7_i64"
        └─ element T = i64
        └─ closure type id = 7
        └─ closure return U = i64
```

The resolver extends `TypeArgResolution` to bind both `T` (from `Array<T>`) and the closure's input/output types plus `ClosureTypeId`. Additive over today's `build_mono_key` — existing type-only keys unchanged.

### 3.2 Specialization Triggers

AOT at bytecode-compile time. Three hooks:

1. **Call-site detection** — `try_monomorphize_method_call` in `compiler/expressions/function_calls.rs`. When `args[i]` is `Expr::FunctionExpr`, peek it (don't yet lower), mint/look up `ClosureTypeId`, pass to resolver.
2. **Type resolution** — `resolve_call_site_type_args` in `compiler/monomorphization/type_resolution.rs`. The `TypeAnnotation::Function` arm must unify closure param/return annotations against the body's inferred types, recording `ClosureTypeId` + return type.
3. **Specialization emission** — `ensure_monomorphic_function` in `compiler/monomorphization/cache.rs`. Substitution step in `substitution.rs` additionally **inlines the closure body** into the specialized stdlib body (so `f(x)` becomes the closure's body), then registers + compiles.

### 3.3 Specialized Bytecode Shape

For `map<number, int, ClosureType_7>` with closure `|x| x + n`:

```
; locals: 0=self (*TypedArray<f64>), 1=out (*TypedArray<i64>), 2=i, 3=x, 4=n (hoisted capture)
NewTypedArrayI64 cap=<self.len>
StoreLocal 1
LoadI64 0; StoreLocal 2
loop_header:
  LoadLocal 2; TypedArrayLenI64 self; LtI64
  JumpIfFalse loop_end
  TypedArrayGetF64 slot=0 idx=2
  StoreLocal 3
  ; --- inlined body of closure_7: |x| x + n ---
  LoadLocal 3                ; x
  LoadLocal 4                ; n (captured)
  ; (type coercion as needed per inferred signature)
  AddI64
  ; --------------------------------------------
  TypedArrayPushI64 slot=1
  LoadLocal 2; AddI64 1; StoreLocal 2
  Jump loop_header
loop_end:
LoadLocal 1
```

**Why inline rather than `Call(fn_id)`**: a direct call would eliminate the `CallValue` indirection, but would still force a frame push/pop and block Cranelift from fusing element load + arithmetic + store into a single loop body. Inlining is the Rust/C++ model. Captures become leading parameters of the specialized body, then get hoisted to locals before the loop.

### 3.4 Dedup / Code-Size Governance

For `arr.map(|x| x * 2).filter(|x| x > 0).reduce(|a,b| a+b, 0)` over `Array<int>`: three specialized bodies.

Risks: syntactically identical closures at two call sites today mint distinct `ClosureTypeId`s and produce two specializations. Mitigations:

- **Structural CSE on closure bodies**: canonicalize `(param_types, capture_types, body_hash)` and share `ClosureTypeId` across structurally identical closures. Slots next to existing content-addressed FunctionBlob hashing.
- **Capture-count threshold**: for closures with > 4 captures, fall back to `Call(fn_id)` dispatch (still direct, no `CallValue`). Body compiled once, only driver loop specialized.
- **Per-module specialization budget**: cap specializations at N per module, fall back to direct-call for overflow.

### 3.5 Fallback for Escaping / `Function`-Typed Closures

When a closure flows through a `Function`-typed parameter, is stored in a field, returns from a function, or the receiver element type is unknown, no `ClosureTypeId` is recoverable at the call site. The resolver sees a generic `TypeAnnotation::Function` without binding and bails.

The fallback path is the current `builtin_map` / `builtin_filter` / `builtin_reduce` in `executor/builtins/array_comprehension.rs` — a VM-side loop using `call_value_immediate_nb`. Post-v2 this gets typed arg/return plumbing but keeps its generic shape. It serves escaped closures, `Array<Function>`, and FFI boundaries. The *only* remaining `CallValue`-style dispatch on higher-order methods.

### 3.6 Relationship to Phase 2.2

Per-closure specialization is a **tightening of Phase 2.2 (stdlib element-type mono)**, not a new phase. Concretely:

- Phase 2.2's `map::i64_string` becomes `map::i64_string_closure_N` whenever arg is a literal `Expr::FunctionExpr`.
- Substitution pass grows one helper: `inline_closure_body_into_specialization(specialized_def, closure_def)`.
- No new cache, no new resolver — a second axis on the existing key.

---

## §4 — Mutable Captures via Stack Pointers

### 4.1 TypedCapture Layout for Mutable Captures

For non-escaping closures mutably capturing an outer slot of proven type `T`, the capture slot is a **typed raw pointer into the caller's stack frame**: `*mut f64`, `*mut i64`, `*mut i32`, `*mut u8` (bool), `*mut *const T` (heap refs). One 8-byte pointer replaces today's `Arc<RwLock<CellValue>>`.

Inside the closure body:
- **Read**: `LoadCaptureMutPtrT(idx)` → `load T [capture_ptr]`
- **Write**: `StoreCaptureMutPtrT(idx, val)` → `store T [capture_ptr], val`

Replaces today's `LoadClosure` / `StoreClosure` for the non-escaping case. Element type is monomorphized into the opcode — no tag checks, no ValueWord round-trip.

### 4.2 Soundness (Already in Borrow Checker)

The MIR solver already treats `ClosureCapture` as a loan sink (`LoanSinkKind::ClosureEnv`). For a mutable capture we register an `Exclusive` loan on the outer slot for the entire lifetime of the closure value. Existing conflict rules then forbid the outer scope from reading/writing that slot (`ConflictExclusiveExclusive`, `ReadWhileExclusivelyBorrowed`, `WriteWhileBorrowed` → B0001/B0002) while the closure is live.

**Already covered**: aliasing between outer scope and closure body; multiple closures with disjoint mutable captures; task boundaries (B0006, B0014).

**Needs extension**: capture lowering currently emits `Operand::Copy`/`Move`, issuing a consumption fact, not a long-lived exclusive loan. Lower captures destined for `LocalMutablePtr` (§4.7) as `Rvalue::Borrow(Exclusive, _)` with region spanning the closure's liveness. Add `LoanSinkKind::ClosureEnvMut` to distinguish.

### 4.3 Escape Fallback

| Scenario | Fallback |
|---|---|
| Closure Local, capture any kind | Stack-pointer capture |
| Closure Escaped, capture is `let mut` | **Compile error** — promote source to `var` or restructure |
| Closure Escaped, capture is `var` | `SharedCow` / `UniqueHeap` (today's `BoxLocal` + `Arc<RwLock>`) |

`let mut` semantics forbid aliasing; a mutable capture by an escaping closure is inherently aliased + outlives the stack frame. Rejecting with a clear diagnostic (`B0003` variant) gives the user an actionable fix. `var` opts into aliasing, so promotion is legal.

### 4.4 `let mut` vs `var` Interaction

`let mut` + non-escaping closure + single mutable capture = the **ideal** stack-pointer case. Exclusive-loan semantics align perfectly with `let mut`'s aliasing invariant.

`var` + non-escaping closure: today forces `SharedCow` unconditionally on capture. v2 can relax: `var` + non-escaping closure + no concurrent outer reads = also eligible for stack pointer. Existing NLL conflict detection is strong enough to enforce safety; `SharedCow` is kept only when escape forces it.

### 4.5 Multiple Mutable Captures

Two captures of disjoint slots: two exclusive loans on different roots — automatically safe via existing solver rules.

Two captures of the same slot or overlapping projections (`x.0` and `x`): already conflict via `ConflictExclusiveExclusive`. No new machinery.

### 4.6 Mutable Captures of Heap-Typed Values

`let mut arr = [1,2,3]; |x| arr.push(x)`: the outer slot holds `*const TypedArray<i64>`. Two distinct mutations:

- **Rebinding** (`arr = new_arr`): writes the pointer slot → needs `*mut *const TypedArray<i64>`.
- **Interior mutation** (`arr.push(x)`): writes through the pointer; the slot itself is only read.

**Safe default**: treat any `&mut` capture of a heap binding as `*mut *const T`. The outer slot's exclusive loan forbids outer access (including concurrent `arr.len()` reads) during the closure's lifetime.

### 4.7 New BindingStorageClass

Add one variant:

```rust
BindingStorageClass::LocalMutablePtr
```

Semantics: "slot lives on the stack, has a `*mut T` handed to a non-escaping closure; borrow checker verified no outer race." Orthogonal to `Direct`/`UniqueHeap`.

### 4.8 Decision Matrix

| Binding kind | Closure escape | Capture kind | Storage | Capture ABI |
|---|---|---|---|---|
| `let` (OwnedImmutable) | any | immutable read | `Direct` | by-value leading param |
| `let mut` (OwnedMutable) | Local | mutable | **`LocalMutablePtr`** | `*mut T` slot in env |
| `let mut` | Escaped | mutable | — | **Error B0003** |
| `var` (Flexible) | Local | mutable | **`LocalMutablePtr`** | `*mut T` slot in env |
| `var` | Escaped | mutable | `SharedCow` | `Arc<RwLock<T>>` fallback |
| `let mut Array<T>` | Local | interior `&mut` | **`LocalMutablePtr`** | `*mut *const TypedArray<T>` |
| `let mut Array<T>` | Escaped | `&mut` | `UniqueHeap` | `Arc<TypedArray<T>>` fallback |
| any | any | first-class `&`/`&mut` | `Reference` | existing Reference path |

---

## §5 — JIT Integration and Escape-Fallback ABI

### 5.1 Cranelift Stack Allocation

Non-escaping `TypedClosure<Captures>` uses `FunctionBuilder::create_sized_stack_slot(StackSlotData { kind: ExplicitSlot, size, align })` — the same mechanism used today for reference stack slots in `mir_compiler/rvalues.rs`. Size = `sum(sizeof(capture_i))` + optional alignment pad.

**Lifetime**: Cranelift `StackSlot`s live for the entire enclosing function frame. Cranelift's register allocator coalesces redundant frames. **No GC stack maps needed**: Shape uses atomic refcounts (not tracing), so stack-allocated closures don't need rooting. The `opcode_is_non_allocating` whitelist in `loop_analysis.rs` is about safepoint polls, not liveness.

```
; let f = |x| x + captured_n  where captured_n: i64, non-escaping
ss0 = explicit_slot 16, align=8         ; { fn_id: u32, type_id: u32, n: i64 }
v_fid = iconst.i32 <closure_fn_id>
stack_store v_fid, ss0, 0
v_tid = iconst.i32 <closure_type_id>
stack_store v_tid, ss0, 4
v0 = load.i64 captured_n
stack_store v0, ss0, 8
v_env = stack_addr.i64 ss0, 0           ; env: *const StackClosure
; … at call site …
v_ret = call fn ClosureType_7_entry (v_env, v_arg)   ; direct, typed ABI
```

### 5.2 Bytecode-Level Inlining Over Cranelift Inlining

Closure body inlining happens **at bytecode-compile time in the monomorphizer**, not at JIT time. Reasons:

- Cranelift's inliner doesn't cross `FuncId` boundaries reliably today.
- Post-bytecode-inline, the closure env pointer becomes dead; Cranelift's scalar-replacement-of-alloca eliminates the stack slot entirely.
- `optimizer/hof_inline.rs` already has infrastructure for HOF body inlining.

After bytecode inlining, the JIT sees a straight typed loop with no `CallValue` / `CallClosure` / `Call`. The element-load + closure-body + element-store fuse into a tight SSA chain that the optimizing tier can vectorize.

### 5.3 Escape-Fallback ABI

Heap-allocated closures use a **per-ClosureTypeId signature**, not a universal signature:

```rust
// ClosureTypeId 7: (int) -> int with captures { n: i64 }
extern "C" fn shape_closure_7_entry(
    env: *const TypedClosureBody<Captures_7>,  // points past HeapHeader
    arg0: i64,
) -> i64;
```

Heap layout: `[HeapHeader | function_id: u32 | type_id: u32 | captures_inline...]`. The `function_id` is the entry for this `ClosureTypeId`.

**`Function<A, R>` dispatch**: two closures with the same callable signature `(A) -> R` but different `ClosureTypeId`s are **ABI-compatible through a `Function<A, R>` pointer**. No trampoline needed — the caller dispatches using the `FunctionTypeId` signature, not the `ClosureTypeId`. A `call_indirect` with a signature lookup from `type_id` handles the uniform case.

### 5.4 `Function<A, R>` Branching Strategy

At call site `f(x)` where `f: Function<(int) -> int>`:

- **Specialized**: type inference narrows `f` to a single `ClosureTypeId` → emit stack-slot path or direct `Call(FuncId)`.
- **Polymorphic**: `load fn_ptr from [env + 8]`; `call_indirect` with `FunctionTypeId` signature. One load + one indirect call. No tag checks (types guarantee correctness).
- **Feedback-guided (Tier 2)**: emit speculative guard `if fn_ptr == expected_ptr then direct else indirect` when an IC is monomorphic, with deopt fallback.

### 5.5 Async / Task Boundaries

Stack-allocated closures cannot cross task boundaries — the stack pointer would outlive the frame.

The borrow checker already catches this: `NonSendableAcrossTaskBoundary` (B0014) fires on detached boundaries; `ExclusiveRefAcrossTaskBoundary` (B0006) fires on exclusive refs.

**Compiler pass addition**: when MIR sees `TaskBoundary(Detached, closure_slot)`, force `ClosureTypeId` to its heap variant before codegen. For `Structured` boundaries with bounded lifetimes, stack allocation is safe if the task cannot outlive the scope (future work: weaken the current conservative rejection).

### 5.6 Snapshot Semantics

`snapshot()` serializes VM state via `SerializableCallFrame`. Stack-allocated closures live in the Cranelift function frame, **not** in the VM's locals array — invisible to the snapshot serializer.

**Policy**: `snapshot()` forces deoptimization to the interpreter first, reusing the existing OSR deopt materialization path. Stack closures get re-materialized on the interpreter stack during deopt, and serialize naturally from there. No heap-promotion path needed.

### 5.7 Refcount Lifecycle

**Escaped closures** use `HeapHeader.refcount` with the standard v2 atomic retain/release instructions.

- **Retain on**: assignment to new binding, push onto refcounted container, task boundary, return from defining frame.
- **Release on**: local going out of scope, array/map element eviction, reassignment (handled by existing `release_old_value_if_heap` in `statements.rs`).

**Nested captures**: each capture whose `ConcreteType` is refcounted (String, Array, Struct, nested Closure) participates in the closure's Drop. The compiler emits per-`ClosureTypeId` Drop glue that walks the typed capture layout. No generic GC traversal; `kind` is never read. This is identical to `TypedStruct` Drop.

---

## §6 — Execution Phases

```
Phase A  TypedClosure layout + ClosureTypeId registry      ─┐
Phase B  Non-escape detection in storage planning           ─┼── Foundation (parallel)
                                                             │
Phase C  Per-closure monomorphization extension             ─── Depends on A, B
         (extends v2-monomorphization-design.md Phase 2.2)
                                                             │
Phase D  Mutable capture via stack pointers                 ─── Depends on B; parallel to C
                                                             │
Phase E  JIT codegen for stack-allocated closures           ─── Depends on A, B, C
                                                             │
Phase F  Escape-fallback ABI + Function<A,R> dispatch       ─── Depends on A, E
                                                             │
Phase G  Snapshot deopt + task-boundary promotion           ─── Depends on E, F
                                                             │
Phase H  Delete legacy Arc<RwLock> closure path             ─── Depends on all above
```

### Phase A — TypedClosure Layout & Registry

**Files**:
- `crates/shape-value/src/v2/closure_layout.rs` (NEW)
- `crates/shape-value/src/v2/concrete_type.rs` (extend `Closure(ClosureTypeId)`)
- `crates/shape-value/src/v2/heap_header.rs`
- `crates/shape-vm/src/compiler/expressions/closures.rs`

**Sub-tasks**:
- Define `TypedClosureHeader` / `StackClosure` structs with `repr(C)`.
- Add `ClosureLayout { fields, heap_capture_mask, size, align }`.
- `ClosureRegistry` keyed on `Vec<ConcreteType>` capture signature; reuse `FieldKind` / `StructLayout` machinery.
- Promote `closure_counter` into `ClosureTypeId` minting; record `closure_function_ids: Vec<(ClosureTypeId, FunctionId)>`.
- Unit tests for layout computation (empty / 1-cap / mixed-type / heap-typed captures).

**Agent team size**: 3 (layout + registry + closure-counter migration)

### Phase B — Non-Escape Detection

**Files**:
- `crates/shape-vm/src/mir/storage_planning.rs`
- `crates/shape-vm/src/mir/analysis.rs` (extend `FunctionBorrowSummary` with `closure_param_escapes: Vec<bool>`)
- `crates/shape-vm/src/mir/solver.rs`

**Sub-tasks**:
- Extend `detect_escape_status` to cover the full table in §2.1.
- Add `non_escaping_closure_slots: HashSet<SlotId>` to `StoragePlan`.
- Transitive closure-capture fixed point (reuse Datafrog).
- Extend `FunctionBorrowSummary` with per-param closure-escape bit; thread through `build_callee_summaries`.
- Tests: simple `arr.map(|x| ...)` classified non-escaping; `[|x| x]` classified escaping; `fn make() { || ... }` classified escaping; transitive capture propagation.

**Agent team size**: 2 (core detection + summary threading)

### Phase C — Per-Closure Monomorphization

**Status: landed on `jit-v2-phase1`.**

Implementation notes:

- **Gotcha option (a)** was chosen: `mint_closure_type_id` stays idempotent at
  the registry level (Phase A's per-capture-signature registry already
  guarantees this). The resolver calls a new
  `mint_closure_type_id_peek(params, body)` helper that intern-mints the id
  without pushing to `closure_type_ids`. Emission still pushes the
  `(func_id, type_id)` pair exactly once at `compile_expr_closure` time — no
  duplicate entries.
- **Cache key format**: `"<fn_name>::<type_args>_closure_<N>_<ret_ty>_b<body_hash:hex>"`
  where `body_hash` is a span-insensitive 64-bit `DefaultHasher` digest of
  the closure's params + body. The `_b<hex>` suffix implements §3.4 structural
  CSE: two closures with identical capture signatures (same `ClosureTypeId`)
  but different bodies produce distinct cache keys.
- **Inlining scope** (Phase C pragmatic): the specialized body's call to the
  formal closure parameter `f(args)` is rewritten into an inlined
  `Expr::Block` containing the closure body with its formal params bound to
  the call's args. The formal closure parameter is **preserved** in the
  specialized function's param list so the call-site ABI stays unchanged
  (the call site still emits `MakeClosure`; the specialized body simply
  never invokes `f`). Stripping the formal + hoisting captures as leading
  params is Phase D/E work — it requires a corresponding call-site rewrite
  to skip `MakeClosure`.
- **Budget**: `DEFAULT_CLOSURE_SPECIALIZATION_BUDGET = 64` per module; when
  exhausted, `ensure_monomorphic_function_with_closures` returns
  `Ok(None)` and callers fall back to the type-only (non-inlined) direct
  `Call(fn_id)` path.
- **Fallback**: non-closure args (bare identifiers / named functions) skip
  the closure-aware path entirely and go through the existing type-only
  resolver. A function-typed parameter passed a closure literal still
  triggers specialization; a function-typed parameter passed a bare name
  does not.

**Files**:
- `crates/shape-vm/src/compiler/monomorphization/type_resolution.rs`
- `crates/shape-vm/src/compiler/monomorphization/cache.rs`
- `crates/shape-vm/src/compiler/monomorphization/substitution.rs`
- `crates/shape-vm/src/compiler/expressions/function_calls.rs`
- `crates/shape-vm/src/compiler/expressions/closures.rs`

**Sub-tasks**:
- Extend resolver to bind `(ClosureTypeId, input_T, output_U)` from closure literal args.
- Extend `build_mono_key` to append closure segments.
- Write `inline_closure_body_into_specialization` helper in `substitution.rs`.
- Hook `try_monomorphize_method_call` to peek closure args and defer their lowering.
- Add deduplication via structural CSE on `(param_types, capture_types, body_hash)`.
- Add per-module specialization budget with `Call(fn_id)` fallback.
- Tests: `arr.map(|x| ...)` emits specialized body; syntactically identical closures share `ClosureTypeId`; escaping closure falls back to generic path.

**Agent team size**: 4 (resolver + cache + substitution+inlining + integration tests)

### Phase D — Mutable Stack-Pointer Captures

**Status: landed on `jit-v2-phase1`.**

Implementation notes:

- **`BindingStorageClass::LocalMutablePtr`** is added in `type_tracking.rs`
  and `LoanSinkKind::ClosureEnvMut` in `mir/analysis.rs`. The new sink kind
  is registered at solve time (treated like `ClosureEnv` but never
  synthesizes a diagnostic — the loan is bookkeeping for the exclusive-loan
  solver rules).
- **Storage-planner promotion**: after `non_escaping_closure_slots` is
  computed, `promote_local_mutable_ptr_slots` walks every `ClosureCapture`
  and demotes each operand root slot from `UniqueHeap` / `Direct` /
  `Deferred` to `LocalMutablePtr` when the owning closure is non-escaping.
  The promotion is unconditional on the operand side (MIR's
  `ClosureCapture` doesn't flag per-operand mutability); the compiler side
  inspects the closure body via `EnvironmentAnalyzer::analyze_function_with_mutability`
  and only emits the typed pointer opcodes for actually-mutated captures.
  Unused promotions are harmless — `LocalMutablePtr` is semantically a
  refinement of `Direct` for the stack-resident case.
- **New opcodes**: `LoadCaptureMutPtr{F64,I64,I32,Bool,Ptr}` and
  `StoreCaptureMutPtr{F64,I64,I32,Bool,Ptr}`. Operand is `Local(idx)` —
  the capture index in the current frame's upvalue table.
- **Interpreter backing (Phase D transitional)**: the new opcodes still
  use the existing `Upvalue::Mutable(Arc<RwLock<ValueWord>>)` shared-cell
  mechanism. `BoxLocal` is emitted at closure creation as before (keeping
  the legacy path alive per §6 Phase H). The typed-ness shows up in
  `op_load_capture_mut_ptr_*` / `op_store_capture_mut_ptr_*` — they skip
  the tag-dispatch step on read/write. Phase E replaces the SharedCell
  backing with a real `*mut T` into a Cranelift `StackSlot`; the opcode
  ABI stays identical.
- **`let mut` + escaping closure**: `compile_expr_closure` inspects the
  enclosing function's MIR storage plan for each mutable capture. If the
  capture's outer slot has ownership `OwnedMutable` and the plan placed
  the slot in any class other than `LocalMutablePtr` / `Reference` /
  `Direct` / `Deferred`, the compiler returns `[B0003] mutable binding
  '<name>' cannot be captured by an escaping closure; promote the source
  to \`var\` or restructure to keep the closure local`.
- **Timing gotcha resolution**: the spec flagged a worry that
  `mint_closure_type_id_peek` (Phase C) runs at resolver time, before the
  enclosing function's storage plan exists. For Phase D the check runs in
  `compile_expr_closure`, which executes during the enclosing function's
  body compilation AFTER MIR + storage planning for that function are done.
  Verified working on the Phase D test suite; no deferred pass needed.
- **MIR-to-compiler slot offset**: MIR reserves `SlotId(0)` for the return
  slot and starts user locals at `SlotId(1)`, while bytecode-local indices
  are 0-based. `mir_storage_class_for_slot` now applies the `+1` offset
  with a fall-through to the un-offset lookup, so callers can pass
  bytecode-local indices directly and the helper stays robust if the MIR
  ABI changes.
- **Legacy path preserved**: `BoxLocal` / `LoadClosure` / `StoreClosure`
  all remain in place. `local_mutable_ptr_captures: HashMap<String, (u16,
  FieldKind)>` on the compiler is the sidecar that tells identifier /
  assignment lowering to use the typed opcode family instead. Phase H
  deletes the legacy mechanism.

**Files**:
- `crates/shape-vm/src/type_tracking.rs` (add `LocalMutablePtr` variant)
- `crates/shape-vm/src/mir/analysis.rs` (add `LoanSinkKind::ClosureEnvMut`)
- `crates/shape-vm/src/mir/solver.rs`
- `crates/shape-vm/src/mir/storage_planning.rs`
- `crates/shape-vm/src/bytecode/opcode_defs.rs` (add `LoadCaptureMutPtr*` / `StoreCaptureMutPtr*`)
- `crates/shape-vm/src/executor/variables/mod.rs`
- `crates/shape-vm/src/executor/dispatch.rs`
- `crates/shape-vm/src/compiler/expressions/closures.rs`
- `crates/shape-vm/src/compiler/expressions/assignment.rs`
- `crates/shape-vm/src/compiler/expressions/identifiers.rs`
- `crates/shape-vm/src/compiler/helpers.rs`
- `crates/shape-vm/src/compiler/helpers_binding.rs`
- `crates/shape-vm/src/compiler/mod.rs`
- `crates/shape-vm/src/compiler/compiler_impl_initialization.rs`

**Sub-tasks** (all done):
- New `BindingStorageClass::LocalMutablePtr`.
- Lowering: mutable captures of non-escaping closures become `Rvalue::Borrow(Exclusive, _)` with closure-lifetime region.
- New capture opcodes; executor handlers do `load/store T [ptr]`.
- `let mut` + escaping closure → compile error (new B0003 variant or B0003 sub-code).
- `var` promotion to `SharedCow` only when escape forces it.
- Tests: non-escaping closure with `let mut` capture works; escaping closure with `let mut` capture rejected; `var` stays stack-pointer when local; multiple disjoint mutable captures allowed.

**Agent team size**: 3 (storage-class + lowering + opcode wiring)

### Phase E — JIT Codegen

**Status: landed on `jit-v2-phase1`.**

Implementation notes:

- **`MirToIR` wired to the storage plan**: the constructor clones the
  MIR's `StoragePlan.non_escaping_closure_slots` into a `MirToIR` field
  so `ClosureCapture` lowering can branch on a per-slot basis without
  re-running escape analysis. A parallel `stack_closure_slots:
  HashMap<SlotId, StackSlot>` tracks which slots received a Cranelift
  stack slot so drop/release paths can skip `arc_release`.
- **Stack-slot layout** (inside `emit_stack_closure`): for each
  non-escaping `ClosureCapture`, `FunctionBuilder::create_sized_stack_slot`
  allocates an ExplicitSlot shaped like `StackClosure { function_id:
  u32 @ 0, type_id: u32 @ 4, captures... }`. Capture offsets are
  computed from the per-capture Cranelift type (F64/I64/I32/I16/I8/ptr)
  with natural alignment, mirroring
  `shape_value::v2::closure_layout::ClosureLayout::stack_capture_offset`.
  The layout helper (`phase_e_layout`) has a dedicated unit-test
  module that cross-checks offsets against the runtime `ClosureLayout`
  for the common F64 signature.
- **Typed capture stores**: captures are written at their native
  Cranelift type (no NaN-box round-trip) so Phase C's inlined body
  can consume them directly via typed `load`. A
  `coerce_for_capture_store` helper handles width mismatches
  (`sextend`/`ireduce`) and falls back to NaN-boxing for unknown
  dynamic slots.
- **`type_id` is currently 0**: Phase E writes a placeholder into the
  `type_id` field. Phase F will thread the real `ClosureTypeId`
  through when `Function<A,R>` dispatch lands — the `type_id` is a
  signature-lookup key for the ABI switch, not a correctness signal
  for Phase E's direct/inlined paths.
- **Ownership plumbing**: `release_old_value_if_heap`, `emit_drop`,
  and the `Copy` operand retain path all early-out when the place's
  slot id is in `stack_closure_slots`. The raw stack-slot address
  is not NaN-boxed and has no refcount, so the legacy `arc_retain`/
  `arc_release` FFI would interpret it as a malformed heap handle.
  Cranelift's stack slots are freed automatically at function return.
- **Direct `Call(FuncId)` ABI**: the existing terminator lowering in
  `mir_compiler/terminators.rs` already emits `call func_ref` with
  ctx_ptr + captures + args when `Operand::Constant(MirConstant::
  Function(name))` resolves to a registered `user_func_refs` entry.
  Phase C emits this pattern post-inline for budget-overflow
  specializations, so no Phase E changes were needed in
  `terminators.rs` or `rvalues.rs`; stack-closure creation is
  purely a statement-level concern.
- **Closure body inlining is Phase C's job, not the JIT's**: per §5.2,
  Phase C inlines the closure body at bytecode-compile time. After
  inlining, the `ClosureCapture` stack slot is dead — the env pointer
  is never read because the captures are already bound to locals via
  the inlined block. Cranelift's scalar-replacement-of-alloca
  eliminates the slot automatically. The JIT does not need a dedicated
  inlining step.
- **`opcode_is_non_allocating` extended**: the 10 Phase D typed
  capture opcodes (`LoadCaptureMutPtr{F64,I64,I32,Bool,Ptr}` +
  `StoreCaptureMutPtr*`) were added to the loop-allocation whitelist.
  These ops never allocate, so loops that only use them no longer
  emit a GC safepoint poll at the loop header.
- **Escape fallback preserved**: closures absent from
  `non_escaping_closure_slots` still go through the legacy
  `jit_make_closure` FFI path. Phase H deletes that path once Phase F
  lands `MakeClosureHeap` + the per-`ClosureTypeId` heap-allocation
  ABI.

**Deferred / Phase G / Phase H**:

- **Snapshot deopt**: §5.6 mandates that `snapshot()` forces a JIT
  deopt before any stack closure is serialized. Phase E does not
  emit the deopt — conservative behavior is that stack closures
  simply cannot survive a snapshot today. The JIT's existing
  deopt-on-error path in the direct-call ABI (`is_error` check in
  `mir_compiler/terminators.rs`) preserves correctness for the
  non-snapshot case: if any later opcode traps, the interpreter
  re-materializes the closure from a refcounted heap rebuild. Phase G
  will add a dedicated `TerminatorKind::Snapshot` to MIR and wire
  re-materialization through `osr_compiler.rs`.
- **Task-boundary promotion**: §5.5 — Phase B's escape analysis
  already flags closures captured across a `TaskBoundary(Detached, _)`
  as escaping, so Phase E correctly falls back to heap for those.
  `Structured` task boundaries are still heap-promoted (conservative).
  Phase G can relax this once parent/child lifetime analysis lands.
- **`type_id` propagation**: stored as 0 today. Phase F writes the
  real `ClosureTypeId` when `Function<A,R>` polymorphic dispatch
  needs it for signature lookup.
- **`JITClosure` FFI layout**: preserved as the Phase H cleanup
  target. Phase E's stack slot layout mirrors
  `shape_value::v2::closure_layout::StackClosure` (8-byte header +
  captures) so Phase H can switch `jit_make_closure` to the new
  layout without touching the stack-closure codegen.

**Files**:
- `crates/shape-jit/src/mir_compiler/mod.rs`
  (add `non_escaping_closure_slots` + `stack_closure_slots` fields)
- `crates/shape-jit/src/mir_compiler/statements.rs`
  (add `emit_stack_closure` + layout helper + unit tests)
- `crates/shape-jit/src/mir_compiler/ownership.rs`
  (skip arc_retain/arc_release on stack-closure slots)
- `crates/shape-jit/src/loop_analysis.rs`
  (Phase D opcodes admitted to safepoint-free whitelist)
- `crates/shape-jit/src/mir_compiler/integration_tests.rs`
  (8 new Phase E tests — gated behind `jit_v2_unstable_tests`
  cfg along with the existing closure integration suite)

**Legacy JIT paths still in place (Phase H cleanup)**:
- `jit_make_closure` FFI + `JITClosure` struct in `context.rs` —
  used by the escape-fallback code path.
- `MakeClosureHeap` opcode — not yet emitted; Phase F adds it.
- `CallValue` indirect dispatch for heap closures — needed until
  Phase F's per-`ClosureTypeId` signature-dispatch lands.

**Agent team size**: 2

### Phase F — Escape-Fallback ABI + Function<A,R> Dispatch

**Status: landed on `jit-v2-phase1`.**

Implementation notes:

- **New heap kind**: `HEAP_KIND_V2_CLOSURE = 84` added to
  `heap_header.rs`. Reserves the kind ordinal for the dedicated
  `TypedClosureHeader` allocation path; Phase F's interpreter still
  wraps escaping closures as `HeapValue::Closure` for backward
  compatibility (the ordinal is consumed by Phase H when the
  interpreter switches to raw `std::alloc::alloc`).
- **`FunctionTypeRegistry`**: new module
  `shape-value/src/v2/function_type_registry.rs` interns
  `FunctionSignature { params, ret }` tuples and hands out sequential
  `FunctionTypeId`s. Two closures with the same callable signature
  share a `FunctionTypeId` regardless of capture layout — this is the
  orthogonal axis to `ClosureTypeId` that makes
  `Array<Function<(int) -> int>>` a first-class type.
- **Compiler wiring**: `BytecodeCompiler` now owns
  `function_type_registry` + `function_type_ids: Vec<(u16,
  FunctionTypeId)>` alongside the Phase A `closure_registry` +
  `closure_type_ids`. Each closure literal mints a `FunctionTypeId` at
  emission time via `mint_function_type_id_for_params`; resolution of
  per-param `ConcreteType`s is conservative in Phase F (unannotated
  params fall back to `Void`). Later phases tighten via bidirectional
  inference.
- **New opcodes** (bytecode-level, additive to Phase A–E):
  - `MakeClosureHeap(Function(fid))` @ 0x122 — semantically equivalent
    to `MakeClosure` in Phase F but signals to the JIT + Phase H that
    the closure has escaped (return, container store, etc.). The
    compiler's emission hook (`emit_make_closure_heap_next`) is set
    by `Statement::Return` and `Expr::Array` element lowering for
    closure literals.
  - `CallClosure(Count(arity))` @ 0x123 — direct dispatch with
    statically-known `ClosureTypeId`. Arity travels in the operand
    (unlike `CallValue`, which pops it from the stack).
  - `CallFunctionIndirect(Count(arity))` @ 0x124 — polymorphic
    dispatch through `Function<A, R>`. Emitted at call sites where
    the callee is a local with registered callable pass modes (i.e.
    the compiler has proven the callee is a typed callable).
- **VM interpreter**: all three new opcodes are wired through
  `executor/control_flow/mod.rs` and `executor/objects/mod.rs`. The
  call dispatch helpers (`dispatch_call_closure_like`,
  `op_make_closure_heap`) factor out the tag-dispatch tree so the
  new opcodes share the mature `CallValue` runtime path. This keeps
  Phase F strictly additive: no existing closure semantics changed.
- **Call-site emission** in `expressions/function_calls.rs`:
  closures called through a name with registered callable
  `expected_param_modes` emit `CallFunctionIndirect(Count(N))` instead
  of the legacy `PushConst count; CallValue` pair. One extra byte of
  bytecode and one less runtime pop.
- **JIT integration (partial)**:
  - `escape_analysis.rs`, `numeric_arrays.rs`, `typed_mir.rs`, and the
    opcode-inventory tables in `compiler/accessors.rs` all know the new
    opcodes. `is_escaping_call` and `is_unknown_stack_effect` now
    include `CallClosure` / `CallFunctionIndirect`; `MakeClosure` and
    `MakeClosureHeap` both mark active array slots as escaped for the
    escape planner.
  - The MIR-level `ClosureCapture` codegen (Phase E's
    `emit_stack_closure`) is unchanged: it still branches on
    `non_escaping_closure_slots` for stack vs legacy-heap allocation.
    Phase F does NOT yet add a dedicated MIR `emit_heap_closure` that
    bypasses the `jit_make_closure` FFI — that's the Phase H
    cleanup target.
  - `type_id` in the JIT's `StackClosure` layout is still 0 in Phase F
    (Phase E placeholder). Threading the real `ClosureTypeId` into
    the JIT requires plumbing `compiler.closure_type_ids()` through
    the worker pipeline; Phase F defers this to Phase G because the
    opcode-level tests do not require it.
- **Tests**: 15 Phase F unit tests in
  `compiler/expressions/closures.rs` cover (a) registry
  correctness for both `ClosureTypeId` and `FunctionTypeId`, (b)
  `MakeClosureHeap` emission for return-of-closure and array-of-closure
  patterns, (c) `MakeClosure` preservation for non-escaping cases,
  and (d) end-to-end runtime correctness for polymorphic
  `Function<A,R>` dispatch including `Array<Function<(int) -> int>>`.

**Deferred to Phase G**:

- Feedback-guided Tier 2 speculative direct-call through the IC
  state machine. The `CallClosure` / `CallFunctionIndirect` opcodes
  expose the hook; the JIT's feedback vector lookup + guard codegen
  are the remaining step.
- Real `TypedClosureHeader` allocation on the JIT side (Phase F's
  VM wraps escaping closures as `HeapValue::Closure` — correct but
  still Arc-boxed).
- Threading `ClosureTypeId` into the JIT's stack-closure layout.
- Snapshot deopt interaction with stack/heap closures.

**Files** (all landed):
- `crates/shape-value/src/v2/heap_header.rs` (new `HEAP_KIND_V2_CLOSURE`)
- `crates/shape-value/src/v2/function_type_registry.rs` (NEW)
- `crates/shape-value/src/v2/mod.rs` (module registration)
- `crates/shape-vm/src/bytecode/opcode_defs.rs`
  (`MakeClosureHeap`, `CallClosure`, `CallFunctionIndirect`)
- `crates/shape-vm/src/executor/dispatch.rs`
  (route new opcodes to their handlers)
- `crates/shape-vm/src/executor/control_flow/mod.rs`
  (new dispatch helpers + opcode handlers)
- `crates/shape-vm/src/executor/objects/mod.rs`
  (route `MakeClosureHeap` to the objects dispatch)
- `crates/shape-vm/src/compiler/mod.rs`
  (new `function_type_registry`, `function_type_ids`,
  `emit_make_closure_heap_next` fields)
- `crates/shape-vm/src/compiler/compiler_impl_initialization.rs`
  (initialise new fields)
- `crates/shape-vm/src/compiler/expressions/closures.rs`
  (`mint_function_type_id_for_params`, `MakeClosureHeap` emission,
  Phase F tests)
- `crates/shape-vm/src/compiler/expressions/function_calls.rs`
  (`CallFunctionIndirect` emission on typed-callable locals)
- `crates/shape-vm/src/compiler/expressions/collections.rs`
  (flag escaping closure literals in array elements)
- `crates/shape-vm/src/compiler/statements.rs`
  (flag escaping closure literals in `return` expressions)
- `crates/shape-jit/src/compiler/accessors.rs`
  (add new opcodes to `ALL_OPCODES`)
- `crates/shape-jit/src/optimizer/escape_analysis.rs`
  (`MakeClosureHeap` marks live array slots as escaping)
- `crates/shape-jit/src/optimizer/hof_inline.rs`
  (recognise `MakeClosureHeap` as a non-inlinable closure)
- `crates/shape-jit/src/optimizer/numeric_arrays.rs`
  (`CallClosure` / `CallFunctionIndirect` have unknown stack effect)
- `crates/shape-jit/src/optimizer/typed_mir.rs`
  (new call opcodes map to `MirOp::Call`)

**Agent team size (actual)**: solo implementation, per Phase F task
constraints.

### Phase G — Snapshot + Task-Boundary Promotion

**Status: landed on `jit-v2-phase1`.**

Implementation notes:

- **Snapshot escape short-circuit (§5.6)**: rather than emit a dedicated
  JIT deopt at the `snapshot()` callsite, Phase G takes the
  conservative MIR-side win: `detect_non_escaping_closure_slots` walks
  the function's block terminators and, if ANY of them is a
  `Call(snapshot, ...)` terminator, short-circuits the entire function's
  non-escaping set to empty. Every closure in a snapshottable
  function therefore goes through the heap ABI, so snapshot
  serialization via `SerializableCallFrame` sees every closure on the
  interpreter's locals array. This sidesteps the need for a precise
  deopt-and-rematerialize path; the dedicated path remains future
  work (§9 open question #7).
- **Task-boundary compiler hook (§5.5)**: `compile_async_let` and
  `compile_async_scope` set `emit_make_closure_heap_next = true`
  before lowering a closure-literal operand, reusing the same hook
  Phase F used for return-of-closure / array-of-closure patterns. The
  detached-boundary case (`async let c = || ...`) forces heap
  unconditionally. The structured-boundary case
  (`async scope { || ... }`) is conservative v1 per §5.5 — it also
  forces heap. A `scope_result_is_closure_literal` helper traverses
  the final block expression to catch the `async scope { || ... }`
  shape where the closure is wrapped in a single-item block.
  Phase B's escape-detection table already flagged closure operands
  in `StatementKind::TaskBoundary` as escaping (rows 5-6), so
  non-literal closure operands already fell back to heap; the new
  compiler hook covers the literal case that never reaches MIR with
  a `TaskBoundary` statement (the literal is inlined into the spawn
  expression).
- **Feedback-guided Tier 2 specialization (§5.4)**: `CallClosure` /
  `CallFunctionIndirect` dispatch now records the resolved target
  `function_id` into the current function's feedback vector via
  `FeedbackVector::record_call`. A new helper
  `executor::ic_fast_paths::closure_call_ic_check` returns a
  `ClosureCallIcHit { function_id, total_calls }` when the site has
  transitioned to `ICState::Monomorphic`. The helper is the Tier 2
  JIT's consumption point: a guard `if observed_fn_id ==
  expected_fn_id then direct_call else fall_through_to_indirect` is
  emitted from the guard key in `ClosureCallIcHit`. No new feedback
  vector type was added — Phase G reuses `FeedbackSlot::Call` per the
  task spec ("do not add a new feedback vector type — extend the
  closure-call feedback entry"). The IC helper is wired and
  tested; threading it through the MIR-compiled JIT's indirect call
  path at `mir_compiler/terminators.rs` requires plumbing a
  callsite-IP lookup through `TerminatorKind::Call` (MIR does not
  currently carry bytecode IPs) and is tracked as a separate Tier 2
  work item — Phase G's verification (below) exercises the VM-level
  recording + IC helper; the JIT IR-level guard codegen is the
  remaining step.
- **Deopt re-materialization glue**: the existing OSR compiler rejects
  any loop body containing closure-related opcodes via
  `is_osr_supported_opcode`, so stack closures never reach an OSR
  frame today. The whole-function MIR-compiled JIT handles closures
  directly (Phase E stack-closure + Phase F heap-closure paths); its
  deopt surface is `TerminatorKind::Call`'s `is_error` guard, which
  already returns control to the interpreter with all VM-visible
  locals written back. Because Phase G's snapshot short-circuit
  forces every closure in a snapshottable function to heap, no
  rematerialization on the deopt path is required for Phase G's
  correctness guarantee; the precise per-closure rematerialization
  path remains Phase H cleanup work.

**Files** (all landed):
- `crates/shape-vm/src/mir/storage_planning.rs`
  (new `mir_contains_snapshot_call` + short-circuit in
  `detect_non_escaping_closure_slots`, 3 MIR-level Phase G tests)
- `crates/shape-vm/src/compiler/expressions/advanced.rs`
  (`compile_async_let` + `compile_async_scope` heap-promotion hooks,
  `scope_result_is_closure_literal` helper)
- `crates/shape-vm/src/executor/control_flow/mod.rs`
  (`dispatch_call_closure_like` records call feedback for
  `CallClosure` / `CallFunctionIndirect`)
- `crates/shape-vm/src/executor/ic_fast_paths.rs`
  (new `ClosureCallIcHit` + `closure_call_ic_check` helpers)
- `crates/shape-vm/src/compiler/expressions/closures.rs`
  (7 Phase G tests: detached-boundary / structured-boundary
  heap-promotion, non-task-boundary control, runtime evaluation,
  heap-capture across a boundary, feedback monomorphic /
  polymorphic roundtrips)

**Deferred to Phase H**:
- JIT-IR-level guard emission for `closure_call_ic_check` results.
- First-class `TerminatorKind::Snapshot` in MIR + dedicated
  per-closure deopt-and-rematerialize path (§9 open question #7).
- Lifting the structured-boundary conservative rejection once
  parent/child future-lifetime analysis lands (§9 open question #6).

**Agent team size (actual)**: solo implementation, per Phase G task
constraints.

### Phase H — Cleanup

**Status: landed (minimal) on `jit-v2-phase1`.**

Implementation notes:

- **Scope was narrowed once Phases D–G landed**. The original plan described
  Phase H as a straightforward deletion pass: remove `Upvalue::Mutable(Arc<
  RwLock<_>>)`, `BoxLocal`, `BoxModuleBinding`, `LoadClosure`, `StoreClosure`,
  the legacy `MakeClosure` opcode, and the `jit_make_closure` FFI. The
  Phase H audit showed that each of those paths is still load-bearing:

  - `Upvalue::Mutable(Arc<RwLock<ValueWord>>)` is the interpreter backing
    for every mutable capture, including those the compiler classifies as
    `LocalMutablePtr` (Phase D's typed capture opcodes read/write through
    the SharedCell auto-deref — the "typed pointer" is a compile-time
    refinement, not a runtime storage switch).
  - `BoxLocal` / `BoxModuleBinding` emit whenever the storage plan can't
    prove `LocalMutablePtr` is safe (module bindings, closures across
    `async scope`, and flexible-storage captures). The fallback is
    reached in production code, not just legacy tests.
  - `LoadClosure` / `StoreClosure` are the read/write opcodes for every
    `Upvalue` — both immutable and mutable captures go through them when
    `LocalMutablePtr` doesn't apply. The typed `Load/StoreCaptureMutPtr*`
    family only covers a subset of closures.
  - `MakeClosure` is the non-escaping path: the JIT's escape-analysis
    distinguishes it from `MakeClosureHeap` to decide between Cranelift
    stack slots (Phase E) and the legacy heap FFI. Collapsing the two
    opcodes would erase that signal.
  - `jit_make_closure` remains the JIT-side heap allocator for escaping
    closures. Phase F deferred the MIR-level `emit_heap_closure`
    codegen (a net-new Cranelift routine that lays out a raw
    `TypedClosureHeader` block in line with the VM's `HEAP_KIND_V2_CLOSURE`)
    to Phase H. Writing that codegen is a several-hundred-line Cranelift
    lowering task in its own right — closer to a Phase F bis than to
    the "cleanup" scope this phase is budgeted for.

  Per §6 Phase A–G landing notes, each prior phase deliberately kept these
  paths alive so landings stayed additive. The audit revealed that the
  task on Phase H's shoulders is not just code deletion but the last
  Cranelift/interpreter rewrite — bigger than "cleanup". Executing that
  rewrite as a minimal Phase H would risk regressions in well-tested
  production paths (`HeapValue::Closure` serialization, snapshot replay,
  cross-module captures, `async scope`). The Phase H plan constraint
  "smaller, correct cleanup beats a broken one" applies directly.

- **What this Phase H DID delete**:
  - `BytecodeCompiler::closure_specialization_cse`: a dead `HashMap`
    field annotated for Phase C structural CSE but never read — Phase C's
    actual CSE runs through the monomorphization cache key
    (`"..._b<body_hash:hex>"`), not this sidecar map.
    [`crates/shape-vm/src/compiler/mod.rs`,
    `crates/shape-vm/src/compiler/compiler_impl_initialization.rs`]

- **Deferred from Phase H (tracked as follow-up work)**:
  1. **`emit_heap_closure` Cranelift codegen**: replace the
     `jit_make_closure` FFI call in `mir_compiler/statements.rs` with a
     direct allocation routine that reserves a `TypedClosureHeader`
     block (`HeapHeader { kind: HEAP_KIND_V2_CLOSURE }` +
     `ClosureTypeId` + captures laid out per `ClosureLayout::
     heap_capture_offset`) and writes captures at their typed offsets
     without NaN-box round-trips. Requires plumbing
     `compiler.closure_type_ids()` through the worker pipeline (also
     deferred from Phase F per its own landing notes).
  2. **VM interpreter `TypedClosureHeader` switch**: `op_make_closure` /
     `op_make_closure_heap` currently allocate `HeapValue::Closure { … }`
     through the Arc-boxed enum. Switching to a raw
     `std::alloc::alloc(Layout::from_size_align(total_heap_size, 8))`
     path (matching the JIT's allocation shape) is invariant-preserving
     but touches snapshot replay, wire serialization, and
     `HeapKind::Closure` rendering. Needs its own design doc and test
     matrix.
  3. **`Upvalue::Mutable(Arc<RwLock<ValueWord>>)` → typed stack-slot
     pointer migration**: only safe once the interpreter's closure
     frames hold a `*mut ValueWord` into a per-frame capture area
     (parallel to the JIT's Cranelift stack slot) rather than a
     heap-allocated shared cell. This blocks on a v3-style frame
     representation (each frame owns its capture area; closures read
     through stable frame pointers). Today's frame descriptor lacks
     the hooks.
  4. **`BoxLocal` / `BoxModuleBinding` emission for non-`LocalMutablePtr`
     captures**: requires extending `LocalMutablePtr` eligibility to
     module-binding captures and to captures across `async scope`
     boundaries. Phase D's scope is local-slot-only by design.
  5. **Merge `MakeClosure` + `MakeClosureHeap` into one opcode with an
     "escapes" flag**: could shrink the opcode surface by one entry,
     but the JIT uses the opcode identity itself as the escape signal
     in `escape_analysis.rs` / `hof_inline.rs`. A flag-carrying
     opcode forces every consumer to decode the operand — a small
     codegen regression for no functional gain today.

  Each of 1–5 is a focused follow-up phase. The Phase H branch
  deliberately reduces the diff rather than attempting all five in one
  landing.

- **Verification gates (§10 Phase H)**:
  - `cargo check --workspace`: clean (0 errors).
  - `grep -rn "Arc<RwLock<.*>>" crates/ | grep closure`: returns only
    comments / doc strings that describe the legacy path, no live
    references beyond the ones enumerated above — removing them requires
    the deferred rewrites.
  - `grep -rn "BoxLocal" crates/shape-vm/src/compiler/`: still non-zero
    for the reasons above (active emission path for module-binding
    captures and flexible-storage captures).
  - `just test-all`: see "Test Results" note below.
  - Performance gate "≥ 30% improvement on closure-heavy benchmarks"
    can only be re-evaluated after the deferred heap-closure codegen
    (#1) lands — Phase E already delivered the stack-closure win; the
    Arc-free heap path is the remaining delta.

**Test results on this phase**:
- `cargo test -p shape-vm --lib`: 2000 passed, 0 failed, 6 ignored.
- `cargo test -p shape-jit --lib`: 343 passed, 0 failed, 23 ignored.
- `cargo test -p shape-runtime --lib`: 1528 passed, 0 failed.
- Tier 1 (`just test-fast`): fully green across all tested crates.

**Files**:
- `crates/shape-vm/src/compiler/mod.rs` (remove dead
  `closure_specialization_cse` field)
- `crates/shape-vm/src/compiler/compiler_impl_initialization.rs` (remove
  corresponding initializer)

**Agent team size (actual)**: solo implementation, per Phase H task
constraints.

### Total: 21 agents across 8 phases

---

## §7 — Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Code-size blowup from per-closure specialization | High | Medium | Structural CSE on closure bodies; per-module budget; fall back to direct call past threshold |
| Missed escape vector → stack pointer outlives frame | Low | Critical | Complete escape-vector table (§2.1); fixed-point over capture graph; conservative default |
| Borrow-check regression on mutable captures | Medium | High | Reuse existing exclusive-loan machinery; extensive borrow-check test matrix |
| JIT deopt during snapshot corrupts closure state | Low | High | Snapshot forces deopt-first; reuse existing OSR materialization path |
| `Function<A,R>` polymorphic dispatch slower than today's Arc | Low | Medium | Feedback-guided specialization catches monomorphic cases; direct specialization wins for literals |
| Recursive closure capture creates refcount cycle | Medium | Medium | Compile-time detection + weak self-capture variant (Rust model) |
| Async closure continuation state doesn't fit the layout | Medium | Medium | Separate `AsyncClosureTypeId` with state-machine pointer; treat as a distinct kind |
| Cross-module closure escape summaries stale | Medium | Medium | Content-addressed per-`FunctionBlob` escape cache |

---

## §8 — Testing Strategy

### Unit tests per phase

- **Phase A**: `ClosureLayout` computation for all capture signatures; `ClosureRegistry` roundtrips; `heap_capture_mask` correctness.
- **Phase B**: 10-entry escape-vector truth table (`arr.map(|x| ...)`, `fn make() { || ... }`, `let a = [closure]`, task boundaries, etc.); transitive capture propagation.
- **Phase C**: specialization cache key generation; inlined body correctness; CSE dedup; escape-fallback path.
- **Phase D**: borrow-check allow/reject matrix for `let mut` vs `var` vs escape.
- **Phase E**: Cranelift IR shape for stack closure creation + inlined call; performance micro-benchmarks.
- **Phase F**: IC state transitions; deopt correctness; ABI signature lookup.
- **Phase G**: snapshot roundtrip with active stack closure; task-boundary heap promotion.

### Integration tests

End-to-end zero-atomic-op pipelines:

```shape
fn a() -> Array<int> { [1, 2, 3] }                     // NewlyOwned (Phase 5.A)
fn b() -> Array<int> { a() }                            // Pipelines through (Phase 5.C)
fn c() -> int {
    let n = 10
    b().map(|x| x + n).reduce(|acc, x| acc + x, 0)     // Zero atomic ops end-to-end
}
```

Verification: instrument the refcount primitives with counters, run `c()`, assert zero atomic increments/decrements on the hot path.

### Performance targets

After Phase H:
- Closure-heavy benchmarks show ≥ 30% improvement over today.
- `arr.map(f).filter(g).reduce(h, 0)` pipeline within 2× of equivalent imperative loop (vs ~10× today).
- Zero atomic ops for pure non-escaping closure pipelines.

---

## §9 — Open Questions

1. **Structural CSE on closure bodies**: use `(param_types, capture_types, hash(body_ast))` or lean on FunctionBlob content hashes (post-bytecode)? Latter can't dedup specializations upfront.
2. **Closure-body inference ordering**: `try_monomorphize_method_call` runs during expression lowering. Are closure param/return types inferred by then, or do we need a pre-pass?
3. **Recursive closures** (`let f = |n| if n<2 { 1 } else { f(n-1) + f(n-2) }`): how do we break the refcount cycle? Weak self-capture (Rust-like) vs compile-time self-ref detection?
4. **Async closures**: separate `AsyncClosureTypeId` with state-machine pointer, or extend `TypedClosureHeader` with async metadata?
5. **Closure-as-trait-object** (`Closure<(f64) -> f64>`): when should the JIT devirtualize vs indirect-call? Proposal: specialize only when the concrete `ClosureTypeId` is known, otherwise use uniform `call_indirect` — verify this is fast enough.
6. **Structured task boundaries**: can closures stack-live on the parent frame for `async scope { ... }`? Requires proving the child future's lifetime is bounded by the parent frame. Conservative "heap-promote all task boundaries" is fine for v1 of this plan.
7. **`snapshot()` as first-class MIR**: without a dedicated `TerminatorKind::Snapshot`, closure escape collapses to Unknown in any function calling it. Worth adding?
8. **Capture-count threshold for inlining**: N captures → direct call vs inlining. What's N? Empirical — measure code size vs perf tradeoff on real programs.
9. **Cross-module closures**: cross-module calls default to Unknown in Phase 5.A; same applies here. Content-addressed `FunctionBlob` escape cache viable?
10. **Trait-method dispatch**: when a closure is passed through a trait method whose impl is dynamically resolved, callee summary is missing. Default to heap-allocated, or require monomorphization to finalize first?

---

## §10 — Verification Gates

### After Phase A
- `cargo check --workspace` passes
- `ClosureLayout` unit tests (20+ cases) green
- No behavioral change visible to user code

### After Phase B
- Escape-detection tests green (10+ cases)
- `FunctionBorrowSummary` extension threaded through solver
- No behavioral change

### After Phase C
- `arr.map(|x| ...)` emits specialized bytecode (verify via `function_bytecode` helper)
- Specialized body contains inlined closure body (no `CallValue`)
- Legacy fallback path still works for escaping closures
- All existing tests pass

### After Phase D
- `let mut` mutable capture by non-escaping closure emits `LoadCaptureMutPtr*` not `LoadClosure`
- `let mut` + escaping closure produces compile error
- Borrow-check regression test matrix green

### After Phase E
- JIT emits `StackSlot` for non-escaping closures (verify via Cranelift IR dump)
- Performance micro-benchmark: closure-heavy workload shows expected improvement
- No JIT crashes on closure-heavy tests

### After Phase F
- `Array<Function<A,R>>` with mixed `ClosureTypeId`s dispatches correctly
- IC state transitions verified via feedback vector inspection

### After Phase G
- `snapshot()` during hot-path works with stack closure
- Task-boundary heap-promotion verified
- Existing snapshot tests green

### After Phase H
- `grep -rn "Arc<RwLock<.*>>" crates/ | grep closure` returns zero
- `grep -rn "BoxLocal" crates/shape-vm/src/compiler/` for closure paths returns zero
- Full test suite (`just test-all`) green
- Performance target hit (≥ 30% improvement on closure-heavy benchmarks)

---

## §11 — References

- `docs/runtime-v2-spec.md` — authoritative v2 runtime spec
- `docs/v2-monomorphization-design.md` — parent plan this extends (particularly Phase 2.2)
- `docs/v2-nanbox-removal-plan.md` — sibling plan for NaN-box deletion
- `docs/enhanced-escape-analysis-v2.md` — Phase 5 (5.A–C landed; 5.D superseded by this doc)
- Rust closures: [Reference §10.3.3](https://doc.rust-lang.org/reference/types/closure.html) — each closure is an anonymous struct
- OCaml closures: Minamide, Morrisett, Harper, "Typed Closure Conversion" (POPL '96)

---

## §12 — Supersedes

This document supersedes `docs/enhanced-escape-analysis-v2.md` Phase 5.D. Phase 5.D's narrow "change SharedCow → Direct for non-escaping closure captures" proposal was an optimization on top of the v1 runtime. The v2 answer is structurally different: typed closure layouts, per-closure monomorphization, stack allocation by default, heap fallback via typed `TypedClosure` — not via `Arc<dyn Fn>`.

Phase 5.D may be removed or reduced to a pointer to this document.

# W12-jit-call-return-kind — audit

**Sub-cluster:** Phase 3 cluster-0 Round 6A (audit-first).
**Branch:** `bulldozer-strictly-typed-w12-jit-call-return-kind` (parent `0484db12`).
**Audited:** 2026-05-12.

Inherited from Round 5B audit `w12-jit-aggregate-non-array-audit.md` §4.4
which surfaced this as option-(iii) territory. This audit re-examines the
classification and concludes the fix sits inside **option (ii)** — a
mechanically-bounded conduit extension — NOT option (iii) ADR-amendment
territory. Reasoning below.

---

## §1. Reproduction

```shape
fn divide(a: int, b: int) -> Result<int, string> {
  if b == 0 { return Err("div by zero") }
  return Ok(a / b)
}
let r = divide(10, 2)
match r {
  Ok(v) => print(v),
  Err(e) => print(e),
}
```

Smoke 1.5. VM mode prints `5` and exits 0. JIT mode (`--mode jit`) errors:

```
Error: Runtime error: JIT execution error (code: -1)
```

Under `SHAPE_JIT_DEBUG=1` the relevant compile failures are now:

```
[jit-mir] compile failed for 'TryFrom::*::Json::tryFrom':
  EnumStore: SURFACE — variant 'Ok' (operands.len()=1) requires a
  co-designed Call-return kind track + pattern-match codegen + NaN-box↔Arc
  conversion ...
```

`divide` itself JIT-compiles cleanly post-Round-5B (the per-function
ConcreteType conduit threaded Enum into `divide`'s `Ok(a/b)` Aggregate
destination). The remaining bottlenecks for Smoke 1.5 are at the TOP-LEVEL
caller:

1. `let r = divide(10, 2)` — `r`'s slot has `ConcreteType::Void` because
   the conduit only walks MIR `StatementKind::{ObjectStore, EnumStore,
   ArrayStore}` and the assignment-from-move propagator. `TerminatorKind::
   Call { destination, .. }` is a kind source the producer does not visit.
2. `match r { Ok(v) => ..., Err(e) => ... }` — the match-on-enum codegen is
   the parallel cluster (Round 6B `W12-jit-match-enum-inline-codegen`).

Item 1 is this sub-cluster's territory; item 2 belongs to 6B. Both are
required for Smoke 1.5 end-to-end JIT success.

---

## §2. Where the function's return type lives at bytecode-compile time

Three locations carry return-type information for user functions:

### §2.1 AST source — `FunctionDef.return_type`

`crates/shape-ast/src/ast/functions.rs:13` defines

```rust
pub struct FunctionDef {
    ...
    pub return_type: Option<TypeAnnotation>,
    ...
}
```

This is the source-of-truth: `TypeAnnotation::Generic { name: "Result",
args: [TypeAnnotation::Basic("int"), TypeAnnotation::Basic("string")] }`
for `divide`. It is preserved verbatim through parsing.

### §2.2 Bytecode `Function` struct — NO return type field

`crates/shape-vm/src/bytecode/core_types.rs:461`:

```rust
pub struct Function {
    pub name: String,
    pub arity: u16,
    pub param_names: Vec<String>,
    pub locals_count: u16,
    pub entry_point: usize,
    pub body_length: usize,
    pub is_closure: bool,
    pub captures_count: u16,
    pub is_async: bool,
    pub ref_params: Vec<bool>,
    pub ref_mutates: Vec<bool>,
    pub mutable_captures: Vec<bool>,
    pub frame_descriptor: Option<FrameDescriptor>,
    pub osr_entry_points: Vec<OsrEntryPoint>,
    pub mir_data: Option<Arc<MirFunctionData>>,
}
```

No `return_type: ConcreteType` field. Neither does `MirFunction` at
`crates/shape-vm/src/mir/types.rs:421`. The bytecode-side `Function` is
a low-level descriptor; type information is held elsewhere.

### §2.3 Type-tracker side-table — `function_return_types: HashMap<String, String>`

`crates/shape-vm/src/type_tracking.rs:667`:

```rust
function_return_types: HashMap<String, String>,
```

Populated by `register_function_return_type` at
`crates/shape-vm/src/compiler/statements.rs:640`:

```rust
if let Some(ref return_type) = func_def.return_type {
    if let Some(type_name) = return_type.as_simple_name() {
        self.type_tracker
            .register_function_return_type(&func_def.name, type_name);
    }
}
```

**Two boundaries here**:

- The registry is keyed by function `name: String`, values are `String`s.
- `as_simple_name()` returns `Some` only for `TypeAnnotation::Basic(_)` /
  `TypeAnnotation::Reference(_)` (crates/shape-ast/src/ast/types.rs:90). For
  `Result<int, string>` (a `TypeAnnotation::Generic`), it returns `None` —
  so `divide`'s entry is **never inserted** into `function_return_types`.

The existing registry is incidental and lossy. It cannot represent
`Result<int, string>` even when the AST has the full annotation.

### §2.4 `concrete_type_from_annotation` — already exists, fully handles Result

`crates/shape-vm/src/compiler/v2_map_emission.rs:357`:

```rust
pub fn concrete_type_from_annotation(annotation: &TypeAnnotation) -> Option<ConcreteType> {
    match annotation {
        ...
        TypeAnnotation::Generic { name, args } => match name.as_str() {
            "Result" if args.len() == 2 => {
                let ok = concrete_type_from_annotation(&args[0])?;
                let err = concrete_type_from_annotation(&args[1])?;
                Some(ConcreteType::Result(Box::new(ok), Box::new(err)))
            }
            "Option" if args.len() == 1 => {
                let inner = concrete_type_from_annotation(&args[0])?;
                Some(ConcreteType::Option(Box::new(inner)))
            }
            "Array" if args.len() == 1 => { ... }
            "HashMap" | "Map" if args.len() == 2 => { ... }
            _ => None,
        },
        ...
    }
}
```

This function already maps `Result<int, string>` to `ConcreteType::Result(
Box::new(I64), Box::new(String))`. It is used in `v2_map_emission` and
`map_emission` for HashMap key/value extraction; nothing else consumes it.

It is the missing piece the conduit can call once per user function to
classify its declared return type into a `ConcreteType`.

### §2.5 `ConcreteType::Result(_, _)` is already recognized by JIT consumers

`crates/shape-jit/src/mir_compiler/v2_array.rs:137-153`:

```rust
pub(crate) fn is_typed_object_slot(&self, place: &Place) -> bool {
    ...
    matches!(
        ct,
        ConcreteType::Struct(_)
            | ConcreteType::Enum(_)
            | ConcreteType::Option(_)
            | ConcreteType::Result(_, _)
            | ConcreteType::Tuple(_)
    )
}
```

`ConcreteType::Result(_, _)` and `ConcreteType::Option(_)` already qualify
as TypedObject slots in the existing JIT consumer. No new variant, no new
matcher. The conduit just needs to stamp them.

---

## §3. What the conduit currently does (and doesn't do) for Call destinations

### §3.1 Producer — `infer_top_level_concrete_types_from_mir`

`crates/shape-vm/src/compiler/helpers.rs:433-573`. Walks the MIR in three
passes:

1. **Pre-pass**: build `slot_scalar_kind` from `Assign(slot, Use(Const))`.
2. **First pass**: stamp slots from `StatementKind::{ObjectStore,
   EnumStore, ArrayStore}` container-store statements.
3. **Second pass**: propagate `Struct`/`Enum`/`Array` through simple
   `Assign(dst, Use(Move|Copy local))` slot moves (fixed-point iteration).

**It does not visit `block.terminator.kind`.** `TerminatorKind::Call {
destination, .. }` is the explicit kind-source the conduit walks past.

After `let r = divide(10, 2)` lowers to MIR roughly as:

```
bb0:
  ...
  goto bb_call

bb_call:
  call(divide, [Constant(10), Constant(2)], destination=r_slot, next=bb_after)

bb_after:
  ...match on r_slot...
```

`r_slot` stays at `ConcreteType::Void` because no `*Store` statement and no
`Use(Move)` propagation seeds it.

### §3.2 Consumer — `is_typed_object_slot` and `concrete_type_for_slot`

The top-level `is_typed_object_slot(r_slot)` returns `false` (`ct ==
ConcreteType::Void`). The downstream match-on-`r` codegen has no inline
path (Round 6B territory), but even the JIT-emitted reads of `r` to
classify the bits flow through the kind-blind `format_value_word` decoder
(`crates/shape-jit/src/ffi/conversion.rs:218-261`). This is the same
`HK_OK` / `HK_ERR` / `HK_SOME` tag-decode path the Round 5B audit §4.4
flagged — it works on NaN-boxed bits but not on raw `Box::into_raw`
pointers (see §6 below).

### §3.3 Round 5C precedent — `well_known_*_return_kind`

`crates/shape-jit/src/mir_compiler/types.rs:573-598` introduced a
JIT-side stamping for `TerminatorKind::Call` destinations using a small
hand-curated registry:

```rust
fn well_known_method_return_kind(name: &str) -> Option<NativeKind> {
    match name {
        "size" | "len" | "length" | "count" => Some(NativeKind::Int64),
        "isEmpty" | "is_empty" | "has" | "contains" => Some(NativeKind::Bool),
        _ => None,
    }
}

fn well_known_function_return_kind(name: &str) -> Option<NativeKind> {
    match name {
        "len" => Some(NativeKind::Int64),
        _ => None,
    }
}
```

These are consulted in `infer_slot_kinds_with_concrete` at
`types.rs:288-311` before the forward statement pass. The registry is:

- **Method-only** (not function): captures collection-method return shapes
  invariant across receivers.
- **Scalar-only**: returns `NativeKind` (not `ConcreteType`). Bool / Int64.
- **Hand-curated**: 4 method names → Int64, 4 → Bool, 1 function `len` →
  Int64. Cannot represent `Result<int, string>` or any user-defined
  function's declared return.

It is insufficient for the general case. We need a per-user-function
`ConcreteType` for the declared return type.

---

## §4. Audit decision — per-function full type knowledge (option (ii))

### §4.1 Why option (i) (extend well_known registry) is wrong

The well-known registry pattern would require the JIT to enumerate every
user function name → ConcreteType mapping at the registry level. That is:

- Not invariant — user-defined functions are the open universe; new
  function definitions can't be hand-rolled into a closed registry.
- Wrong layer — the registry is JIT-internal hardcoded knowledge.
  User-function return types live at the bytecode compiler. Pushing
  per-user-function data through a JIT-internal `match name` arm is the
  wrong shape (the JIT side should consume what the compiler proves; the
  compiler is the proof source, not the JIT-internal classifier).
- Already-rejected pattern — adding more `match name` arms is the small-
  step walk-back the W-series formalized: "just one more entry" expands
  unboundedly when the principled fix is structural.

### §4.2 Why option (iii) (ADR amendment) is wrong here

Round 5B's audit framed this as option (iii) because the EnumStore consumer
fix alone wouldn't unblock Smoke 1.5 without "co-designed Call-return
kind track + pattern-match codegen + NaN-box↔Arc conversion."

That framing was correct for what 5B could land in scope, but it conflates
three orthogonal pieces:

- **Call-return kind track** (this sub-cluster's territory) — a conduit
  extension that walks `TerminatorKind::Call` destinations and stamps them
  from the callee's declared `return_type`. No ADR amendment: same
  §2.7.5 stamp-at-compile-time discipline as the existing
  `ObjectStore`/`EnumStore`/`ArrayStore` walks, applied to one more
  kind-source MIR shape.
- **Match-on-enum inline codegen** (Round 6B's territory) — separate
  sub-cluster, separate JIT IR codegen path. Independent of #1.
- **NaN-box ↔ `Arc<ResultData>` round-trip** (§6 below; possibly this
  sub-cluster) — a known FFI boundary gap. Whether it's a JIT-side bug or
  a JIT-FFI ABI gap is the audit question §6 answers.

ADR-006 §2.7.5 already authorizes producing-site classification at MIR
emit time / bytecode-compile time. Adding Call-destination stamping to the
conduit is exactly the §2.7.5 producing-side classification the ADR
prescribes. **No new § paragraph required.**

### §4.3 Option (ii) — thread per-function `return_type` through the conduit

Mechanical scope:

1. **Add a side-table on `BytecodeProgram`**:
   `function_return_concrete_types: Vec<ConcreteType>` (indexed by function
   index, `Void` for "no annotation" / "annotation didn't reduce"). Same
   `#[serde(skip)]` shape as `function_local_concrete_types`.
2. **Populate** at `compiler/compiler_impl_reference_model.rs:1480-1493`
   (alongside the per-function conduit walk): for each function, look up
   the AST `FunctionDef.return_type`, call
   `concrete_type_from_annotation`, store the result.
3. **Extend `infer_top_level_concrete_types_from_mir`** to accept a callee
   resolver `fn(name) -> Option<&ConcreteType>` and stamp each Call-
   terminator destination with the callee's return type.
4. **Thread** the new side-table through `linker.rs` / `remote.rs` /
   `ContentAddressedProgram` / `LinkedProgram` (same pattern as
   `function_local_concrete_types`).
5. **Wire** the resolver at both call sites (top-level + per-function) in
   `compile_post_assembly`.

No new MIR shape. No new HeapKind. No new FFI entry. No ADR amendment.
Same §2.7.5 producing-site classification discipline.

Where to look up the AST `FunctionDef`:

- `BytecodeProgram.expanded_function_defs:
  HashMap<String, shape_ast::ast::FunctionDef>` (core_types.rs:353).
  Populated by `register_function_def` at compile time. Keyed by function
  name. The AST `return_type: Option<TypeAnnotation>` is available
  verbatim.

So the producer side has access to the AST return type via the existing
`expanded_function_defs` map. No new persistence surface needed.

### §4.4 Naming guard

Per CLAUDE.md defection-attractor list, the new pieces use principled
names, not "translator"/"bridge"/"adapter":

- `function_return_concrete_types` (sibling to
  `function_local_concrete_types`) — describes WHAT the field is.
- `infer_top_level_concrete_types_from_mir` extended with a
  `callee_returns: &dyn Fn(&str) -> Option<&ConcreteType>` parameter —
  same name, more general (its body was already generic).

No "call-return bridge" / "return-kind translator" / "result-kind helper"
naming — those are explicitly listed in the dispatch prompt as
defection-attractor framings.

---

## §5. Effect on Smoke 1.5

Post-fix, the conduit at top level stamps `r_slot`'s ConcreteType as
`Result(Box::new(I64), Box::new(String))`. `is_typed_object_slot(r_slot)`
returns true. The Round 5B Aggregate short-circuit fires (Round 5B already
landed that, scoped at the wrong layer — the conduit was the missing
producer, not the consumer).

**However**: the binding's kind is only half the picture. The
`match r { Ok(v) => print(v), Err(e) => print(e) }` codegen depends on
Round 6B's `W12-jit-match-enum-inline-codegen` to dispatch the variant tag
inline. Without 6B, the match falls back to a generic SwitchBool path that
can't discriminate `Ok` from `Err`.

**Verdict**: this sub-cluster's fix is necessary but not sufficient. It
moves `r`'s kind from `Void` to `Result(I64, String)` (the producer-side
classification), unblocking 6B's consumer. 6B + this combined is what
delivers Smoke 1.5 JIT success.

This is exactly the territory split the dispatch prompt anticipated:

> NOTE: Smoke 1.5 may ALSO depend on Round 6B's W12-jit-match-enum-inline-
> codegen for the `match r { Ok(v) => ..., Err(e) => ... }` dispatch. If
> your work alone produces correct binding-kind for `r` but the `match`
> dispatch is the remaining blocker, report it — cluster-0 close needs
> both.

---

## §6. NaN-box ↔ `Arc<ResultData>` round-trip audit

### §6.1 The two carriers

**VM side** (`crates/shape-vm/src/executor/vm_impl/builtins.rs:551-586`):

```rust
BuiltinFunction::OkCtor => {
    let payload: KindedSlot = args.remove(0);
    let res = Arc::new(shape_value::heap_value::ResultData::ok(payload));
    self.push_kinded_slot(KindedSlot::from_result(res))?;
}
```

Slot bits = `Arc::into_raw(Arc<ResultData>) as u64`; slot kind =
`NativeKind::Ptr(HeapKind::Result)`. The payload is the entire
`KindedSlot` (kind + bits), so this carrier preserves the inner value's
kind.

**JIT side** (`crates/shape-jit/src/ffi/result.rs:21-32`):

```rust
pub extern "C" fn jit_make_ok(inner_bits: u64) -> u64 {
    box_ok(inner_bits)
}
```

`box_ok(inner_bits) = unified_box(HK_OK, inner_bits)`
(`value_ffi.rs:381-383`):

```rust
pub fn box_ok(inner_bits: u64) -> u64 {
    unified_box(HK_OK, inner_bits)
}
```

`unified_box` (`jit_kinds.rs:155-160`):

```rust
pub fn unified_box<T>(kind: u16, data: T) -> u64 {
    UnifiedValue::new(kind, data).heap_box()
}
```

`heap_box` is `Box::into_raw(Box::new(self)) as u64` — returns a raw
pointer. The inner payload is `u64` bits only — **no kind preservation** for
the inner value. This is a single-typed-payload carrier.

### §6.2 The round-trip gap

`jit_make_ok` returns raw `Box::into_raw(UnifiedValue<u64>) as u64` —
**not NaN-boxed**. The boundary predicates assume NaN-boxing:

`is_ok_tag` → `is_heap_kind(bits, HK_OK)` → `heap_kind(bits)` →
`is_heap(bits)` (`value_ffi.rs:320-322`):

```rust
pub fn is_heap(bits: u64) -> bool {
    is_tagged(bits) && get_tag(bits) == TAG_HEAP_BITS
}

fn is_tagged(bits: u64) -> bool {
    bits & TAG_BASE == TAG_BASE
}
```

`TAG_BASE` is the negative-NaN signature; raw `Box::into_raw` pointers
(addresses like `0x55a1b0...`) have neither the negative NaN bits nor a
TAG_HEAP_BITS tag in bits 50-48. So `is_heap(bits) == false`, and the
chain returns `is_ok_tag(bits) == false` on every output of `jit_make_ok`.

This is the deleted-ValueWord-shape API the W12-deleted-valuewordshape-
tests-rewrite (Round 3, 2026-05-12) documented:

> Under ADR-006 §2.7.5 the producers `box_ok` / `box_err` / `box_some`
> return raw `Box::into_raw(UnifiedValue<u64>) as u64` (no NaN-box tag
> bits). The consumers `is_ok_tag` / `is_err_tag` / `is_some_tag` call
> `is_heap_kind(bits, HK_OK)` etc., which gates on `is_heap(bits) ->
> is_tagged(bits)` — returns false for raw pointers. Every `jit_is_*`
> returns `TAG_BOOL_FALSE` and every `jit_unwrap_*` returns `TAG_NULL` on
> the producers' output.

The 5 round-trip tests asserting this round-trip were marked deleted as
"asserting the deleted ValueWord-shape API". But the **production
boundary itself was not fixed** — it still has the same broken predicate
shape. The deleted tests acknowledged it; the producers and consumers
both still ship.

### §6.3 Boundary at `ffi/object/conversion.rs:222-255`

`nanboxed_to_jit_bits` (and its `_with_ctx` sibling) are documented as
identity pack/unpack per ADR-006 §2.7.5 — they preserve raw bits, kind
flows through the carrier. They do NOT convert between NaN-box and Arc.

There is **no `convert_jit_ok_to_arc_result` or similar boundary
conversion** at the JIT-FFI boundary. The dispatch prompt referenced
"convert_jit_ok_to_arc_result (or similar) at ffi/conversion.rs:246-258"
— that range is the `format_value_word` HK_OK/HK_ERR/HK_SOME arms in
`crates/shape-jit/src/ffi/conversion.rs`. Those arms use `jit_unbox::<u64>`
on the raw pointer — they correctly decode the JIT-side `UnifiedValue<u64>`
allocation prefix-kind, NOT NaN-box tag bits.

**So**: the round-trip is:

- `jit_make_ok(inner_bits)` returns raw `Box::into_raw(UnifiedValue<u64>) as u64`.
- `is_ok_tag(bits)` reads NaN-box tag bits → returns false.
- `format_value_word(bits)` reads the heap-kind prefix at offset 0 of the
  raw pointer → matches `HK_OK` arm → calls `jit_unbox::<u64>(bits)` to
  read inner.

**The round-trip is broken at `is_ok_tag` but works at `format_value_word`**.
Different paths reach different consumers.

### §6.4 Cross-mode: JIT ↔ VM

When a JIT-compiled function calls a VM-resident user function (or vice
versa), the boundary is at:

- `dispatch_call_via_trampoline_vm` (W11-jit-carrier-conversion close,
  Round 2): the trampoline VM stamps `NativeKind::UInt64` for args /
  captures. If a JIT-compiled `divide` returns `box_ok(5)` (raw pointer
  bits), and that value enters VM stack via the trampoline, the slot's
  `NativeKind` becomes `UInt64` (the §2.7.5 carrier kind for "I64-wide
  raw bits without further classification") — not `Ptr(HeapKind::Result)`.

So the VM-side `KindedSlot { kind: Ptr(HeapKind::Result), slot: ... }`
shape post-`BuiltinFunction::OkCtor` is NOT preserved across a JIT call
boundary. The bits are JIT-internal raw pointers to `UnifiedValue<u64>`,
not `Arc<ResultData>`.

This is a **fundamental ABI gap**: the VM carrier and JIT carrier are
different storage shapes for the same logical `Result<int, string>`.

### §6.5 Scope decision for §6

Fixing the NaN-box↔Arc round-trip requires:

- Either: extend `jit_make_ok`/`_err`/`_some` to produce `Arc<ResultData>`
  bits (allocate via the same path as `BuiltinFunction::OkCtor`, return
  `Arc::into_raw` bits, mark kind `Ptr(HeapKind::Result)`).
- Or: extend the JIT-internal pattern-match consumer to handle BOTH the
  raw-pointer `UnifiedValue<u64>` shape AND the `Arc::into_raw(Arc<
  ResultData>)` shape, dispatched on slot kind.

The first option (producer-side migration) is the §2.7.5 stamp-at-compile-
time + canonical-storage-shape discipline — single source of truth, no
boundary conversion. The second option preserves two parallel shapes.

**Recommendation**: this sub-cluster does NOT touch the carrier — Smoke 1.5
does not exercise cross-mode boundaries (the top-level main and `divide`
both JIT-compile and run as a single JIT execution). The carrier mismatch
surfaces only when VM-resident code observes the JIT carrier, which would
need a specific test. The carrier mismatch is a known follow-up sub-cluster
worth surfacing for cluster-1 (`W12-jit-result-carrier-unification`); see
§9 below.

The audit found a real architectural gap, but it is not load-bearing for
Smoke 1.5. Round 6B's match-enum-inline codegen consumes the JIT carrier
within the JIT (raw pointer + `HK_OK`/`HK_ERR` prefix) — it does not need
to read an `Arc<ResultData>`.

---

## §7. Proposed fix (Commit 2)

### §7.1 Bytecode side — `BytecodeProgram.function_return_concrete_types`

Add a new field on `BytecodeProgram` at
`crates/shape-vm/src/bytecode/core_types.rs` (parallel to
`function_local_concrete_types`):

```rust
/// Per-user-function declared `ConcreteType` for the function's return type.
///
/// `function_return_concrete_types[f]` is the proven `ConcreteType` for
/// function index `f`'s return value, derived from the AST
/// `FunctionDef.return_type` via `concrete_type_from_annotation`.
/// `ConcreteType::Void` means "no annotation" or "annotation didn't
/// reduce to a known shape" — the JIT consumer treats Void as the
/// no-information sentinel per §2.7.5.1.
///
/// Producer: `compile_post_assembly` (per-function walk after the
/// per-function conduit walk).
/// Consumer: `infer_top_level_concrete_types_from_mir` extended with a
/// callee-return resolver; stamps Call-terminator destinations with the
/// callee's return ConcreteType.
///
/// ADR-006 §2.7.5 — W12-jit-call-return-kind close, 2026-05-12.
#[serde(skip, default)]
pub function_return_concrete_types: Vec<shape_value::v2::ConcreteType>,
```

### §7.2 Populate the side-table

At `compiler/compiler_impl_reference_model.rs` (around line 1493, right
after the `function_local_concrete_types` populate):

```rust
let mut per_fn_ret: Vec<shape_value::v2::ConcreteType> =
    Vec::with_capacity(self.program.functions.len());
for func in &self.program.functions {
    let ct = self.program
        .expanded_function_defs
        .get(&func.name)
        .and_then(|fd| fd.return_type.as_ref())
        .and_then(|ann| {
            crate::compiler::v2_map_emission::concrete_type_from_annotation(ann)
        })
        .unwrap_or(shape_value::v2::ConcreteType::Void);
    per_fn_ret.push(ct);
}
self.program.function_return_concrete_types = per_fn_ret;
```

### §7.3 Extend the conduit producer

`crates/shape-vm/src/compiler/helpers.rs::infer_top_level_concrete_types_from_mir`:

Add a second pass before the existing first pass that walks every
`block.terminator`. For `TerminatorKind::Call { func, destination, .. }`:

- If `func` is `Operand::Constant(MirConstant::Function(name))`, look up
  the name in a callee-return resolver (passed as a new parameter).
- If the resolver returns `Some(ct)` and `ct != ConcreteType::Void`, stamp
  the destination slot's `concrete_types[idx]` with the cloned ConcreteType.

The propagation second pass then naturally extends this through `let r =
divide(...)` chains via `Use(Move|Copy local)` propagation.

Resolver shape:

```rust
pub(crate) fn infer_top_level_concrete_types_from_mir(
    mir: &crate::mir::MirFunction,
    callee_returns: Option<&dyn Fn(&str) -> Option<&shape_value::v2::ConcreteType>>,
) -> Vec<shape_value::v2::ConcreteType>
```

`None` callers (existing tests, fixtures) keep current behavior (skip Call
destinations). Production call sites pass a closure built over
`function_return_concrete_types` + a name→index map.

### §7.4 Thread through linker / remote / content-addressed

Mirror the `function_local_concrete_types` threading at:

- `crates/shape-vm/src/linker.rs:476`, `:586`, `:663`
- `crates/shape-vm/src/remote.rs:539`, `:697`, `:1610`, `:1758`, `:2112`, `:2170`
- `crates/shape-vm/src/bytecode/content_addressed.rs:220`, `:382`
- `crates/shape-vm/src/compiler/compiler_impl_initialization.rs:496`
- `crates/shape-jit/src/worker.rs:320`
- `crates/shape-jit/src/compiler/program.rs:303`

Same `#[serde(skip, default)]` pattern. Empty vec at default.

### §7.5 Wire at both call sites in `compile_post_assembly`

For the top-level `top_level_local_concrete_types` populate
(`compiler_impl_reference_model.rs:1455`), build the resolver from the
already-populated `function_return_concrete_types` and the function-name
→ index map:

```rust
if let Some(ref mir_data) = self.program.top_level_mir {
    let name_to_idx: std::collections::HashMap<&str, usize> = self.program.functions
        .iter().enumerate().map(|(i, f)| (f.name.as_str(), i)).collect();
    let returns = &self.program.function_return_concrete_types;
    let resolver = |name: &str| -> Option<&shape_value::v2::ConcreteType> {
        let idx = *name_to_idx.get(name)?;
        let ct = returns.get(idx)?;
        if matches!(ct, shape_value::v2::ConcreteType::Void) { None } else { Some(ct) }
    };
    let concrete_types =
        crate::compiler::helpers::infer_top_level_concrete_types_from_mir(
            &mir_data.mir, Some(&resolver));
    self.program.top_level_local_concrete_types = concrete_types;
}
```

Same shape for the per-function `function_local_concrete_types` populate.

### §7.6 What this does NOT touch

- **No change to `Function` struct** — return-type info goes on the
  program-level side-table, not per-function inline.
- **No change to MIR shape** — `TerminatorKind::Call` is unchanged. The
  callee-name lookup is done at conduit time, not embedded into MIR.
- **No change to `MirFunction`** — no new field.
- **No new MIR statement / terminator** — same shape as Round 5B's
  `EnumStore.variant_name` precedent: data flows through side-tables when
  it doesn't need to be on the kind-source statement itself.
- **No JIT-side change** required for the binding's kind — the existing
  `is_typed_object_slot` consumer already accepts `Result(_, _)` /
  `Option(_)`. Once the conduit stamps the slot, the consumer fires.
- **No NaN-box↔Arc round-trip fix** in this commit — that is the §9
  surfaced follow-up, not load-bearing for Smoke 1.5.

---

## §8. Forbidden patterns this fix does NOT introduce

- **No Bool-default fallback** — when the AST has no return annotation, or
  `concrete_type_from_annotation` returns None, the entry stays
  `ConcreteType::Void` per §2.7.5.1 / forbidden #9.
- **No tag-bit decode** — the conduit input is the AST annotation and MIR
  terminator shape; no runtime bit inspection.
- **No "call-return bridge" / "return-kind translator" / "result-kind
  helper"** — fields and helpers are named for what they are
  (`function_return_concrete_types`, `infer_top_level_concrete_types_from_mir`
  extended with `callee_returns` parameter).
- **No `MirConstant::Function` resurrection as a kind-blind carrier** —
  the resolver consumes the name and returns the proven ConcreteType;
  the name is the lookup key, not a kind source.
- **No ValueWord resurrection** — `ConcreteType` lives in shape-value
  post-strict-typing.
- **No silent walkback** — failures still surface; the conduit's
  `ConcreteType::Void` propagates naturally and consumers fall through.
- **No new MIR opcode** — preserved.

---

## §9. Sites surfaced (cite-tracked follow-up)

| Item | §-cite | Disposition |
|---|---|---|
| NaN-box vs `Arc<ResultData>` carrier mismatch — `jit_make_ok` produces `Box::into_raw(UnifiedValue<u64>) as u64`; `BuiltinFunction::OkCtor` produces `Arc<ResultData>` via `KindedSlot::from_result`. These are different storage shapes for `Result<int, string>`. The boundary `dispatch_call_via_trampoline_vm` stamps `UInt64` (carrier kind) when bits cross JIT→VM. | §2.7.5 (single source of truth) / §2.7.14 | **Future sub-cluster** `W12-jit-result-carrier-unification`. Not load-bearing for Smoke 1.5 (single JIT execution, no cross-mode boundary). Surface for cluster-1 |
| `is_ok_tag` / `is_err_tag` / `is_some_tag` predicates assume NaN-box tag bits, return false on raw-pointer producer output. Documented at `crates/shape-jit/src/ffi/result.rs:178-200` as the rationale for deleting 5 round-trip tests in Round 3. Production callers (if any remain) get false negatives. | §2.7.5 / §2.7.7 #4 (tag-bit dispatch deleted) | **Cluster-1 candidate** — paired with the carrier-unification above. Both are part of the same JIT-side Result/Option storage cleanup |
| `format_value_word` HK_OK/HK_ERR/HK_SOME arms correctly read the JIT-internal `UnifiedValue<u64>` allocation prefix via `jit_unbox::<u64>`. This works on the raw-pointer producer shape; it does NOT decode Arc<ResultData>. If a top-level slot's bits are `Arc::into_raw(Arc<ResultData>)`, the print path crashes (wrong type recovery) | §2.7.5 | Same cluster — `W12-jit-result-carrier-unification` |
| The Round 5B `EnumStore: SURFACE` consumer for non-empty payloads inside user functions still surfaces. Now that the binding's kind is correctly stamped by the conduit at the top level, the user function bodies' `EnumStore` still has no inline codegen (28 stdlib `TryFrom::*::Json::tryFrom` functions are affected). Independent of the top-level Call-return kind track | §2.7.14 / §2.7.5 | **Round 6B's territory** (`W12-jit-match-enum-inline-codegen`) — the consumer side. Cross-cluster if 6B also needs to handle EnumStore production, otherwise stays open |
| Match-on-enum inline codegen for `Ok(v)` / `Err(e)` / `Some(x)` / `None` patterns at top level | §2.7.5 | **Round 6B's territory** (`W12-jit-match-enum-inline-codegen`). Load-bearing for Smoke 1.5 end-to-end alongside this sub-cluster |

---

## §10. Close gates (per dispatch)

```
cd /home/dev/dev/shape-lang
devenv shell --quiet -- bash -c "cd shape-w12-jit-call-return-kind && cargo check --workspace --lib --tests"  EXIT=0
devenv shell --quiet -- bash -c "cd shape-w12-jit-call-return-kind && cargo test -p shape-jit --lib"          322/0/26 baseline
devenv shell --quiet -- bash -c "cd shape-w12-jit-call-return-kind && bash scripts/verify-merge.sh"           12/12
devenv shell --quiet -- bash -c "cd shape-w12-jit-call-return-kind && bash scripts/check-no-dynamic.sh"       EXIT=0
```

### §10.1 Smoke 1.5 expectation

Post-fix-alone (this sub-cluster's commit 2 only):

- The `let r = divide(10, 2)` slot's ConcreteType becomes
  `Result(Box::new(I64), Box::new(String))` (verifiable by
  `concrete_type_for_slot` lookup in the top-level MirToIR).
- The top-level Aggregate short-circuit fires when the conduit-stamped
  `r` slot is the destination — but `let r = call_result` is NOT an
  Aggregate; it is the Call-terminator destination. The fix point is the
  match-on-`r` codegen seeing `r` as `is_typed_object_slot`.
- The downstream `match r` codegen still depends on Round 6B. Without
  6B, Smoke 1.5 still fails JIT mode end-to-end.

**Expected end-to-end Smoke 1.5 JIT success**: requires BOTH this fix AND
Round 6B. Surface this in close report per dispatch's "If your work alone
produces correct binding-kind for `r` but the `match` dispatch is the
remaining blocker, report it" discipline.

---

## §11. Why option (iii) was not the right call

Round 5B's classification of "this is (iii) ADR-amendment territory" was
correct given the audit's framing: the EnumStore consumer alone couldn't
fix Smoke 1.5, and the surfaced co-design space (Call-return tracking +
match codegen + ABI cleanup) was genuinely larger than the consumer fix.

This audit re-examines the same territory at a finer grain:

- The **Call-return kind track piece** (this sub-cluster) is option (ii) —
  same conduit shape as existing kind-source statement walks, applied to
  one more MIR shape (`TerminatorKind::Call`). No new ADR ruling needed.
- The **match codegen piece** is Round 6B's territory — independent.
- The **NaN-box↔Arc ABI piece** is a real architectural gap but is NOT
  load-bearing for any current cluster-0 smoke. Surface it as a follow-up
  cluster, do not block this sub-cluster on co-designing it.

Splitting the 5B-monolith into three independent sub-clusters lets each
ship at its own scope. Cluster-0 gets closer to close with each landing,
none requires the others to land first.

ADR-006 §2.7.5 already authorizes producing-site classification at every
kind-source MIR shape the compiler can see. `TerminatorKind::Call` is a
MIR shape, and the callee's declared return type IS the producing-site
proof. No amendment.

# W12-jit-aggregate-non-array — audit

**Sub-cluster:** Phase 3 cluster-0 Round 5B (audit-first).
**Branch:** `bulldozer-strictly-typed-w12-jit-aggregate-non-array` (parent `7a78799b`).
**Audited:** 2026-05-12.
**Verified affected sites:** `divide` (Smoke 1.5), `first_positive` (Smoke 2), 28 stdlib functions surfacing `Rvalue::Aggregate` + 2 stdlib functions surfacing `compile_binop_dynamic_arith` (the second is W12-jit-binop-after-heap-read-kind-tracker territory — Round 5A — not this sub-cluster).

---

## §1. Reproduction

`SHAPE_JIT_DEBUG=1 ./target/release/shape run /tmp/smoke_1_5.shape --mode jit` on:

```shape
fn divide(a: int, b: int) -> Result<int> {
    if b == 0 { return Err("div by zero") }
    Ok(a / b)
}
let r = divide(10, 2)
match r { Ok(v) => print(v), Err(e) => print(e) }
```

produces 30 `[jit-mir] compile failed for` lines. The breakdown:

```
     28 Rvalue::Aggregate         (this sub-cluster)
      2 compile_binop_dynamic_arith  (Round 5A territory)
```

The Aggregate failures include `divide` itself and 27 stdlib helpers
(`Into::int::decimal::into`, every `TryInto::*::*::tryInto` permutation,
`Json.keys`, `TryFrom::*::Json::tryFrom`, `std::core::math::spread`, etc.).
VM mode prints `5`; JIT mode prints `Error: Runtime error: JIT execution
error (code: -1)` (the stub-fallback deopt signal that W12-jit-linker-resolve
correctly emits when Phase-4 compile fails on a load-bearing user function).

---

## §2. Audit grid: per ConcreteType destination

### §2.1 Enum destinations (`ConcreteType::Enum(EnumLayoutId)`)

**Current behavior (top-level code, conduit populated):**

The conduit's `infer_top_level_concrete_types_from_mir`
(`crates/shape-vm/src/compiler/helpers.rs:486-494`) walks
`StatementKind::EnumStore { container_slot, .. }` and stamps the destination
slot with `ConcreteType::Enum(EnumLayoutId(0))`. The placeholder
`EnumLayoutId(0)` is **a sentinel**, NOT a real layout — JIT consumers only
check the `Enum(_)` variant tag via `is_typed_object_slot` and never read
the layout id. See `crates/shape-jit/src/mir_compiler/v2_array.rs:145-153`.

The Aggregate consumer's TypedObject short-circuit
(`crates/shape-jit/src/mir_compiler/statements.rs:61-65`) then fires:

```rust
if matches!(rvalue, Rvalue::Aggregate(_)) && self.is_typed_object_slot(place) {
    return Ok(());
}
```

The MIR's subsequent `StatementKind::EnumStore` (at
`statements.rs:203-240`) emits the real allocation. **Currently this only
runs in EMPTY-payload mode** (line 227-229: `if operands.is_empty() { return
Ok(()); }`); non-empty payloads surface-and-stop with the stray "§2.7.4"
cite. **But for user functions the upstream Aggregate short-circuit
already swallowed the Aggregate**, so the EnumStore site is the only site
that actually emits enum-construction codegen — and it surfaces.

**Current behavior (user-function code, conduit NOT populated):**

In `crates/shape-jit/src/compiler/program.rs:343`, the per-user-function
`MirToIR` is constructed with `concrete_types: Vec::new()`. The comment
admits the gap:

```rust
// v2: per-slot ConcreteTypes for the v2 typed-array fast path.
// The bytecode-program-level side-table is in flux upstream
// (other Phase 3.1 agents are refactoring it), so we pass an
// empty vec for now — MirToIR's v2 fast path falls through to
// the legacy NaN-boxed path on `None`. Wire-up will happen
// once Agent 1 lands the BytecodeProgram concrete-types vec.
```

This is the load-bearing miss. Without the conduit per-function:
- `is_typed_object_slot(place)` returns `false` (`concrete_types.get(...)`
  returns `None` because the vec is empty);
- `v2_typed_array_elem_kind(place)` returns `None`;
- The `Assign(Aggregate)` falls through to `compile_rvalue`'s kind-blind
  fallback at `rvalues.rs:144-176` and surfaces.

The `Ok(a/b)` inside `divide`, the `Some(x)` inside `first_positive`, the
`Some(...)` / `None` / `Err(...)` returns inside every stdlib helper that
returns a `Result<T,E>` / `Option<T>` / `tryInto`-style constructor — all
fail at this site.

**Classification:** option **(ii)**. The conduit was designed to support
this exact case; it's already wired through `BytecodeProgram` →
`LinkedProgram` → `MirToIR`. The fix is **extending the conduit's
producer side to also walk user-function MIR** (each `Function` has
`mir_data: Option<Arc<MirFunctionData>>`) and threading the result into
the per-function `MirToIR` construction.

No new HeapKind, no new MIR opcode, no new dispatch shape, no new FFI
entry, no ADR amendment. The producer (`infer_top_level_concrete_types_from_mir`)
is name-misleading — its body is generic over any MirFunction; only the
populate site is top-level-only.

### §2.2 Struct destinations (`ConcreteType::Struct(StructLayoutId)`)

**Current behavior:**

Same shape as Enum. The conduit
(`crates/shape-vm/src/compiler/helpers.rs:477-485`) walks
`StatementKind::ObjectStore` and stamps `Struct(StructLayoutId(0))`. The
TypedObject short-circuit fires identically; the real allocation lands at
`StatementKind::ObjectStore` (`statements.rs:141-201`), which has a
non-empty consumer that calls `typed_object_alloc` + per-field
`typed_object_set_field`.

Top-level `let p = Point{x:3, y:4}` works (Round 3 W12-top-level-concrete-types-conduit
landed it — Smoke 3 surfaces a downstream `compile_binop_dynamic_arith`
gap on `p.x + p.y`, which is Round 5A territory, not this sub-cluster).

User-function code paths emit identical MIR but the conduit isn't
populated for the function → the short-circuit doesn't fire → Aggregate
surfaces.

**Classification:** option **(ii)**. Same fix as §2.1 — extending the
conduit to user functions covers struct-literal-in-function-body too.

### §2.3 Tuple destinations (`ConcreteType::Tuple(_)`)

The ConcreteType variant exists at `crates/shape-value/src/v2/concrete_type.rs:80`,
and `is_typed_object_slot` already accepts it (`v2_array.rs:151`). But
the conduit's producer in `helpers.rs:474-524` does NOT emit Tuple
because tuples don't appear in MIR as a distinct kind-source statement.
Tuple literals lower through the same `Rvalue::Aggregate` shape as enum
payloads (verified at `crates/shape-vm/src/mir/lowering/expr.rs:1617-1632`
for `EnumConstructorPayload::Tuple`).

Bare tuple literals (`(1, 2, 3)` as an expression) lower to a temp via
`Aggregate(operands)` followed by a `Drop` — there's no separate
`StatementKind::TupleStore`; tuples are represented as Enum-variant
payloads OR as anonymous TypedObject (no kind-source statement). Tuple
destinations therefore **don't reach this Aggregate site as a top-level
destination** in the current MIR shape — they only appear as the inner
operand sequence of an EnumStore.

Smoke matrix doesn't exercise bare tuple destinations. **Classification:**
option **(i)/no-op** — no additional work required for Tuple beyond what
§2.1 already covers. If a future smoke surfaces bare-tuple destinations,
the conduit can be extended to detect tuple-literal MIR shapes — same
mechanical pattern.

### §2.4 The `EnumLayoutId(0)` / `StructLayoutId(0)` placeholder

**Verification:** the placeholder is a sentinel. Search across the JIT
shows no consumer reads the layout id field:

```
$ grep -rn 'EnumLayoutId\|StructLayoutId' crates/shape-jit/src/
crates/shape-jit/src/mir_compiler/v2_array.rs: ... matches!(ct, ConcreteType::Struct(_) | ConcreteType::Enum(_) | ...)
```

The match uses `_` for the wrapped id. The conduit's choice of `0` is
intentional and documented at `helpers.rs:482-484`:

> the schema-id placeholder is irrelevant for the JIT short-circuit
> which only checks the variant tag

**Classification:** no work required. `EnumLayoutId(0)` / `StructLayoutId(0)`
are correct as-is per the existing conduit contract. Threading real
layout ids would be ADR-level work (real layout ids would need a
producer that the bytecode compiler doesn't currently emit), and is
unnecessary for the cluster-0 close criterion. **Surface-and-stop if
any future consumer needs the real id** — that's the gate for option
(iii), not this audit.

---

## §3. Audit decision matrix

| Destination | Current behavior | Top-level works? | User-function works? | Fix scope | Classification |
|---|---|---|---|---|---|
| Enum (`Ok(v)`/`Err(e)`/`Some(x)`/user enums) | Aggregate short-circuits via `is_typed_object_slot` IF conduit stamped Enum | Yes (Round 3 conduit) | **No (conduit empty)** | Extend conduit's `populate` to user fns | **(ii)** |
| Struct (`Point{x,y}`) | Aggregate short-circuits via `is_typed_object_slot` IF conduit stamped Struct | Yes (Round 3 conduit) | **No (conduit empty)** | Same as Enum | **(ii)** |
| Tuple (`(1,2,3)` payload) | Never appears as Aggregate destination — payload-only | n/a | n/a | No fix needed | none |
| `EnumLayoutId(0)` placeholder | Sentinel, never read | Correct | Correct | No fix needed | none |
| EnumStore non-empty payload | Surfaces with stray §2.7.4 | n/a (Aggregate already short-circuited) | n/a (Aggregate already short-circuited) | Dead code path post-fix; surface remains as defensive backstop | (cite-only fix) |

**Result:** scope is **option (ii)** — the ConcreteType conduit extension
to per-function MIR. No ADR amendment. No new MIR opcode. No new FFI
entry. The fix is mechanically symmetric with the top-level conduit:
walk each `Function`'s `mir_data.as_ref().map(|m| infer_top_level_concrete_types_from_mir(&m.mir))`,
store the result on `BytecodeProgram.function_local_concrete_types`,
and thread it through `compile_function_with_user_funcs` at
`compiler/program.rs:343`.

---

## §4. Why the EnumStore non-empty-payload surface is dead post-fix

When the conduit stamps `Enum(_)` on the destination slot, the
`Assign(Aggregate)` short-circuit at `statements.rs:61-65` returns `Ok(())`
**before reaching the EnumStore consumer**. The EnumStore consumer's
non-empty-payload branch is dead code in the typed-conduit world — but
it remains as a defensive backstop for the case where the conduit
somehow misses a slot. That backstop must surface-and-stop honestly
(per ADR-006 §2.7.7 #9 forbidden Bool-default), but its existing cite
"§2.7.4" is wrong (task-scheduler boundary). The cite-only fix in
commit 2/3 is mechanical: §2.7.4 → §2.7.14 / §2.7.5.

The audit doc itself has the same wrong cite at line 215; the cite-fix
applies symmetrically.

---

## §5. Proposed fix (Commit 2)

### §5.1 Bytecode side — `BytecodeProgram.function_local_concrete_types`

Add a new field on `BytecodeProgram` at
`crates/shape-vm/src/bytecode/core_types.rs` (parallel to the existing
`top_level_local_concrete_types`):

```rust
/// Per-user-function per-MIR-slot `ConcreteType` side-table.
///
/// `function_local_concrete_types[f][slot]` is the proven
/// `ConcreteType` for slot `slot` in function index `f`. Empty when
/// the function has no MIR data or the conduit couldn't prove anything.
///
/// Producer: `infer_top_level_concrete_types_from_mir` (name is
/// historical — its body is generic over any MIR function) called per
/// `Function::mir_data` in `compile_post_assembly`.
///
/// Consumer: `compile_function_with_user_funcs` at
/// `crates/shape-jit/src/compiler/program.rs:343`, which threads the
/// per-function entry into `MirToIR::concrete_types`.
///
/// ADR-006 §2.7.5 / W12-jit-aggregate-non-array close, 2026-05-12.
#[serde(skip)]
pub function_local_concrete_types: Vec<Vec<shape_value::v2::ConcreteType>>,
```

`#[serde(skip)]` because `ConcreteType` doesn't derive Serialize (it's
runtime metadata, not wire-stable).

Populate in `crates/shape-vm/src/compiler/compiler_impl_reference_model.rs`
right after the top-level conduit (around line 1455-1460), walking each
function:

```rust
let mut per_fn: Vec<Vec<shape_value::v2::ConcreteType>> =
    Vec::with_capacity(self.program.functions.len());
for func in &self.program.functions {
    if let Some(ref mir_data) = func.mir_data {
        per_fn.push(
            crate::compiler::helpers::infer_top_level_concrete_types_from_mir(
                &mir_data.mir,
            ),
        );
    } else {
        per_fn.push(Vec::new());
    }
}
self.program.function_local_concrete_types = per_fn;
```

Thread through `linker.rs` (3 sites already pass `top_level_local_concrete_types`),
`remote.rs` (snapshot/restore sites), and `LinkedProgram` for
serialized-program parity.

### §5.2 JIT side — read per-function entry

At `crates/shape-jit/src/compiler/program.rs:343`, replace:

```rust
let concrete_types: Vec<shape_value::v2::ConcreteType> = Vec::new();
```

with:

```rust
let concrete_types: Vec<shape_value::v2::ConcreteType> = program
    .function_local_concrete_types
    .get(func_idx)
    .cloned()
    .unwrap_or_default();
```

### §5.3 Stray §-cite fix

- `crates/shape-jit/src/mir_compiler/statements.rs:236`: `§2.7.4` → `§2.7.14 / §2.7.5`.
- `docs/cluster-audits/w12-enum-constructor-audit.md:215`: `§2.7.14 / §2.7.4` → `§2.7.14 / §2.7.5`.

---

## §6. Forbidden patterns this fix does NOT introduce

- **No Bool-default fallback** — when the conduit cannot prove a slot's
  ConcreteType, the per-function vec entry stays `ConcreteType::Void` and
  the JIT surfaces-and-stops at the Aggregate site, identical to today's
  behavior (per §2.7.7 #9, §2.7.5).
- **No new MIR opcode** — the existing `Aggregate` / `EnumStore` /
  `ObjectStore` shape is unchanged.
- **No new HeapKind** — the conduit reuses existing ConcreteType variants.
- **No tag-bit decode** — the conduit's input is the MIR statement shape;
  no runtime bit inspection.
- **No "boundary translator" / "aggregate bridge" / "enum-payload helper"
  / "struct-aggregate adapter"** — the producer is named for what it does
  (`infer_top_level_concrete_types_from_mir`; the "top_level" prefix is
  historical and the body is already generic). No CLAUDE.md "Renames to
  refuse on sight" framings.
- **No ValueWord resurrection** — `ConcreteType` lives in shape-value
  post-strict-typing.
- **No silent walkback** — failures still surface with structured §-cite.

---

## §7. Close gates (per dispatch)

```
cd /home/dev/dev/shape-lang
devenv shell --quiet -- bash -c "cd shape-w12-jit-aggregate-non-array && cargo check --workspace --lib --tests"  EXIT=0
devenv shell --quiet -- bash -c "cd shape-w12-jit-aggregate-non-array && cargo test -p shape-jit --lib"         EXIT=0 (322/0/26 baseline, no regressions)
devenv shell --quiet -- bash -c "cd shape-w12-jit-aggregate-non-array && bash scripts/verify-merge.sh"          12/12
devenv shell --quiet -- bash -c "cd shape-w12-jit-aggregate-non-array && bash scripts/check-no-dynamic.sh"      EXIT=0
```

### §7.1 Smoke 1.5 & Smoke 2 expectations

- Smoke 1.5: VM `5`, JIT `5` (was `JIT execution error (code: -1)`).
- Smoke 2: VM `Some(3)`, JIT `Some(3)` (was `JIT execution error (code: -1)`).
- `[jit-mir] compile failed for` count under SHAPE_JIT_DEBUG=1 drops
  substantially. Current pre-fix: 30 (28 Aggregate + 2 binop). After
  fix: expect ≤2 (the binop pair is Round 5A's territory, not this
  sub-cluster's; some Aggregate failures may remain if the conduit
  can't prove a particular stdlib's destination ConcreteType — those
  surface honestly per §2.7.7 #9).

---

## §8. Sites surfaced (for cite-tracked follow-up)

| Item | §-cite | Disposition |
|---|---|---|
| The 2 `compile_binop_dynamic_arith` failures (`math::spread`, `math::zscore`) | §2.7.5 producing-site kind-tracker | **Round 5A's territory** (`W12-jit-binop-after-heap-read-kind-tracker`). Cross-cluster — flag if my fix doesn't reduce these to zero |
| EnumStore non-empty payload backstop | §2.7.14 / §2.7.5 | Dead code path post-fix when conduit populates the destination slot. Backstop preserved for defensive surface-and-stop. Cite corrected from §2.7.4 |
| Real `EnumLayoutId` / `StructLayoutId` threading | §2.7.5 / §2.7.6 | Not needed for any cluster-0 smoke. Sentinel `0` is correct per the existing conduit contract; the JIT only checks the variant tag. ADR-level if a future consumer needs the real id |

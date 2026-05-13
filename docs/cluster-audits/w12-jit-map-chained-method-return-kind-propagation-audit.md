# W12-jit-map-chained-method-return-kind-propagation audit

Phase 3 cluster-0 Round 14 W12-map-chained audit-first deliverable.
Three audit deliverables per the dispatch prompt: (§1) layer identification,
(§2) cluster-0 disposition, (§3) surface-and-stop if ADR-amendment territory.

Branch: `bulldozer-strictly-typed-w12-jit-map-chained-method-return-kind-propagation`
Parent: `a846ddfa` (post-Round-13-merge + Round 14 dispatch metadata).
Date: 2026-05-13.

## §0. Scope recap and binding precedents

Kickoff Smoke 2 JIT (`[1,2,3,4,5].map(|x|x*2).sum()` → `30`) fails JIT with:

```
Error: Runtime error: JIT compilation failed: Route A surface-and-stop:
NotImplemented(SURFACE) — `print` Call-terminator operand NativeKind is
None
```

VM mode produces `30` (confirmed via
`./target/release/shape run --mode vm /tmp/kickoff_smoke2_full.shape` at
this commit). Surface-fire site is
`crates/shape-jit/src/mir_compiler/terminators.rs:572-612` —
`_ =>` arm of the print-dispatch kind switch (line 320), reached because
`operand_slot_kind(&args[0])` returns `None`.

Binding precedents on the §2.7.5 producing-site conduit (in order):

- **Round 6A close** (`9cd5bbe0`) — landed
  `BytecodeProgram.function_return_concrete_types: Vec<ConcreteType>`
  side-table populated from `FunctionDef.return_type` via
  `concrete_type_from_annotation`, consumed by
  `infer_top_level_concrete_types_from_mir_with_returns` as the
  `callee_returns` resolver at the Call-terminator destination pass
  (`helpers.rs:712`). Closes
  `MirConstant::Function(_)` Call destination stamping.
- **Round 11-trinity Part b** (`5b113145`) — extended the JIT-side
  `well_known_method_return_kind` registry with the parametric companion
  `parametric_method_return_kind_from_receiver` in
  `crates/shape-jit/src/mir_compiler/types.rs:921`. Classifies
  `Array<T>.sum/mean/min/max/get/first/last/pop`, `HashMap.get`,
  `Mutex.get`, `Atomic.load/fetch_*/compare_exchange`, `Lazy.get` by
  reading `concrete_types[args[0].root_local()]`. Receiver-recovery
  soundness verbatim per §2.7.5.
- **Round 13 T1'** (`fd028c1b`) — closed cross-crate trait-method
  return-kind conduit via gap 1 (struct identity propagation through
  slot moves in `helpers.rs::infer_top_level_concrete_types_from_mir_with_resolvers`)
  + gap 2 (chained existing data through trait_method_symbols ×
  function_return_concrete_types — no new BytecodeProgram side-table) +
  gap 3 (impl-method return-type backfill from trait declaration). Same
  shape as 6A.

## §1. Layer identification

### §1.1 Trace

Source program (`/tmp/kickoff_smoke2_full.shape`):

```shape
let xs = [1, 2, 3, 4, 5]
let doubled = xs.map(|x| x * 2)
print(doubled.sum())
```

MIR for the top-level entry (per `crates/shape-vm/src/mir/lowering/expr.rs:1847-1869`):

- `xs.map(|x| x*2)` lowers via `Expr::MethodCall` arm at
  `expr.rs:1866` to `Call(MirConstant::Method("map"), [xs_op, closure_op], dst=doubled_slot, ...)`.
- `doubled.sum()` lowers to `Call(MirConstant::Method("sum"), [doubled_op], dst=tmp_slot, ...)`.
- `print(tmp)` lowers to `Call(MirConstant::Function("print"), [tmp_op], ...)`.

(MIR lowering is independent of the bytecode emitter's method-call
monomorphization/inlining — the bytecode trace's
`Call(Function(FunctionId(195)))` is the monomorphized VM-side
specialization; the JIT consumes the MIR which still carries
`MirConstant::Method("map")`. Verified at
`compiler_impl_reference_model.rs:1423` where `top_level_mir` is built
directly from AST.)

JIT-side kind classification chain at
`crates/shape-jit/src/mir_compiler/types.rs::infer_slot_kinds_with_concrete`:

1. `kinds[doubled_slot]` (destination of `.map()` Call):
   - `func = MirConstant::Method("map")` →
     `well_known_method_return_kind("map")` returns `None` (no entry at
     `types.rs:793-805`; not an invariant-name method).
   - Falls through to
     `parametric_method_return_kind_from_receiver("map", args, concrete_types)`
     at `types.rs:921`. `args[0]` is move of `xs_slot`;
     `concrete_types[xs_slot] = Array(I64)` (stamped by the `ArrayStore`
     producer pass at `helpers.rs:657-682`). The `match (name, receiver_ct)`
     at `types.rs:942-993` has **no `("map", ConcreteType::Array(_))` arm**
     — returns `None`.
   - `kinds[doubled_slot]` stays `None`.

2. `concrete_types[doubled_slot]` (the bytecode-side ConcreteType
   vector):
   - The bytecode-side producer
     `infer_top_level_concrete_types_from_mir_with_resolvers`
     (`crates/shape-vm/src/compiler/helpers.rs:494`) has FOUR
     destination-stamping passes: `ObjectStore` (line 563), `EnumStore`
     (line 572), `ArrayStore` (line 657), and `Call-terminator with
     MirConstant::Function(_)` (line 710-738) via `callee_returns`,
     plus the `MirConstant::Method(_)` arm gated on
     `struct_names[receiver_slot]` (line 824-873) for trait-method
     dispatch on user-defined struct receivers (T1' commit 2).
   - **No pass stamps `MirConstant::Method(_)` destinations when the
     receiver is a built-in container** (`Array<T>`, `HashMap<K,V>`,
     `Mutex<T>`, etc.). `struct_names[xs_slot]` is `None` (not a
     `StructLiteral` site), so the T1' `method_returns` resolver pass
     at line 824 skips this Call entirely (line 855-858: `None =>
     continue`).
   - `concrete_types[doubled_slot]` stays `ConcreteType::Void`.

3. `kinds[tmp_slot]` (destination of `.sum()` Call):
   - `func = MirConstant::Method("sum")` → `well_known_method_return_kind("sum")`
     returns `None`.
   - Falls through to `parametric_method_return_kind_from_receiver("sum", args, concrete_types)`.
     `args[0]` is move of `doubled_slot`;
     `concrete_types[doubled_slot] = Void`. Line 939:
     `if matches!(receiver_ct, ConcreteType::Void) { return None; }`.
   - `kinds[tmp_slot]` stays `None`.

4. `operand_slot_kind(&args[0])` at the print Call-terminator
   (`terminators.rs:311`) reads `kinds[tmp_slot] = None`.

5. Surface fires at `terminators.rs:572` (`_ =>` arm of the print
   dispatch switch).

### §1.2 Layer disposition

**§1 (a) — Conduit extension.** Same structural shape as Round 11-trinity
Part b (which extended `well_known_method_return_kind` invariant-name
classifier with the parametric `parametric_method_return_kind_from_receiver`
companion), generalised one tier upstream: the **bytecode-side producer**
needs a parametric receiver+method classifier producing `ConcreteType`
(not `NativeKind`) so the chained-method receiver lookup picks up the
proven shape on the second hop. This is identical to the T1' closure
shape: extend the existing producer at
`helpers.rs::infer_top_level_concrete_types_from_mir_with_resolvers`
(`helpers.rs:494`) with a new MirConstant::Method Call-terminator
destination-stamping pass that fires when the receiver+method pair has a
derivable parametric `ConcreteType`.

This is NOT a §2.7.7 parallel-track propagation gap (the kind track
itself is fine — the surface-fire site `terminators.rs:572` reads from
`kinds[tmp_slot]`, the parallel kind vector, which faithfully reflects
the upstream `concrete_types[tmp_slot] = Void`). And this is NOT MIR
lowering or chain composition (`Expr::MethodCall` lowering at
`mir/lowering/expr.rs:1847` is correct: it emits Call with
receiver-as-args[0] as the §2.7.5 / Q11 convention requires).

### §1.3 Fix shape (scoped, bounded)

The fix shape mirrors the JIT-side
`parametric_method_return_kind_from_receiver` classifier exactly,
producing `ConcreteType` at the bytecode-side producer. Receiver+method
pairs needing classification (working backwards from the kickoff Smoke 2
chain):

- `Array<T>.map(closure) → Array<closure_return_ct>` — receiver
  `ConcreteType::Array(T)`, closure operand at `args[1]` (per MIR
  lowering convention at `expr.rs:1856-1865`), closure return ct
  resolves through `function_return_concrete_types[closure_function_id]`
  when the closure operand is a `Move` of a slot stamped via
  `StatementKind::ClosureCapture { function_id: Some(fid), .. }`. When
  the closure's `function_id` is `None` (rare; non-monomorphized inline
  shape) the destination ConcreteType stays `Void` per §2.7.5.1 (no
  fabricated default).
- `Array<T>.filter(closure) → Array<T>` — type-preserving on element
  type; ignores closure return except for boolean semantic.
- `Array<T>.flatMap(closure) → Array<closure_return_inner_ct>` —
  expects closure return to be `Array<U>`; result is `Array<U>`.
- `Array<T>.slice(start, end) → Array<T>` — type-preserving.
- `Array<T>.reverse() → Array<T>` — type-preserving.
- `Array<T>.sort() → Array<T>` — type-preserving.
- `Array<T>.concat(other: Array<T>) → Array<T>` — type-preserving.
- `Array<T>.take(n) / .skip(n) → Array<T>` — type-preserving.
- `Array<T>.zip(other: Array<U>) → Array<(T, U)>` — tuple result (out
  of cluster-0 scope unless the kickoff smoke matrix needs it).

Cluster-0 Smoke 2 minimally needs the `("map", Array(T))` arm with
closure-return-ct resolution (closure-return path via
`function_return_concrete_types`). The remaining arms above are added
in the same pass for completeness (mirror of trinity Part b's
8-receiver+method-pair landing).

Resolving the closure operand's return ConcreteType:

1. Find the closure operand among `args[1..]` (skip the receiver at
   `args[0]`). For `.map(closure)` it's `args[1]`. For methods that
   take multiple closures (e.g. `.reduce(initial, closure)`) it would
   be `args[2]` — bounded per-method.
2. Resolve the operand's root_local. The slot must have been written by
   a `StatementKind::ClosureCapture { closure_slot, function_id: Some(fid), .. }`
   statement.
3. Look up `function_return_concrete_types[fid]`. Use the
   already-computed `per_fn_ret` vector built at
   `compiler_impl_reference_model.rs:1468-1487`.
4. When any link in the chain is missing (operand isn't a Move/Copy of a
   Local, the slot wasn't written by ClosureCapture, the function_id is
   None, the closure's `function_return_concrete_types[fid]` is Void) —
   the destination ConcreteType stays Void per §2.7.5.1 (the existing
   discipline; not a fabricated default).

The new pass runs AFTER the existing
`MirConstant::Function(_)` Call-terminator destination pass at
`helpers.rs:710-738` and AFTER the slot-move propagation pass at
`helpers.rs:763-799`, so:

- Function-call destinations are stamped first (e.g. a method call to a
  user function via desugaring).
- Slot-move chains propagate Array-typed slots from their construction
  sites to user-visible bindings (`let xs = [1,2,3]; let ys = xs; ys.map(...)`).
- The new MirConstant::Method pass then sees receivers via propagated
  slot identities, with up-to-date `concrete_types[receiver_slot]`.

The new pass must NOT run after T1's `method_returns` resolver pass
(`helpers.rs:824`), because that pass already handles
`MirConstant::Method` for user struct receivers — to avoid double-write
contention, the new built-in-receiver pass and the existing user-struct
pass are mutually exclusive on receiver kind (built-in receiver:
`concrete_types[receiver_slot]` is `Array(_)`/`HashMap(_,_)`/`Mutex(_)`/
`Atomic`/`Lazy(_)`/`Set(_)`/... etc.; user-struct receiver:
`struct_names[receiver_slot]` is Some). The two passes are
non-overlapping on the same receiver slot by construction.

## §2. Cluster-0 disposition

Kickoff Smoke 2 JIT (`[1,2,3,4,5].map(|x|x*2).sum()` → `30`) is the
specific blocked smoke per the Round 14 dispatch table. Confirmed at
this commit via `SHAPE_JIT_DEBUG=1 ./target/release/shape run --mode jit
/tmp/kickoff_smoke2_full.shape` — surface message is verbatim per the
prompt:

```
[jit-mir] print: SURFACE §2.7.5 — operand NativeKind not proven (None)
or unwired heap arm. ...
Error: Runtime error: JIT compilation failed: Route A surface-and-stop:
NotImplemented(SURFACE) — `print` Call-terminator operand NativeKind is
None; either the §2.7.5 producer-site classification conduit doesn't
stamp this operand's kind at the upstream MIR shape ...
```

Pre-Round-14 baseline (post-Round-13-merge `b53f090d`) was producing
the same surface — verified by the Round 14 dispatch metadata commit
`a846ddfa` which names Smoke 2 JIT as still blocked. Per supervisor's
Q2 ruling: cluster-0 absorbs since this is a kickoff smoke blocker.
Disposition: **cluster-0 territory, proceed to code edit**.

Adjacent forms also reachable from the same conduit gap (not cluster-0
blocking, but covered by the same fix):

- `xs.filter(|x| x > 0).sum()` — same shape; `.filter()` is
  type-preserving on element.
- `xs.map(...).map(...).sum()` — chained map twice; same shape, same
  arm fires twice.
- `m.get(k).map(...)` — `HashMap.get(k)` returns `Option<V>`, then
  `.map(closure)` on `Option<U>`. Outside Smoke 2's chain shape but
  same conduit gap class — note for cluster-1 (Option/Result method
  chains) if it surfaces.

## §3. ADR-amendment posture

**No ADR amendment required.** The fix shape is a conduit extension at
the bytecode-side producer (`helpers.rs::infer_top_level_concrete_types_from_mir_with_resolvers`),
not a §2.7.7 parallel-track invariant extension, not a §2.7.10/Q11
ABI shape change, not a §2.7.5.1 wire-format change. The Round 14
dispatch prompt's surface-and-stop trigger ("§2.7.7 parallel-track
invariant extension for chained-method intermediate results") does NOT
fire — the parallel-kind track is correctly carrying `kinds[tmp_slot]
= None`, faithfully reflecting that the upstream
`concrete_types[tmp_slot]` is `Void`. The fix is one tier upstream of
the kind track at the conduit producer; the kind track itself doesn't
need extension.

Same precedent fit:

- Round 6A (`9cd5bbe0`) — first
  conduit extension at this producer: added the
  `MirConstant::Function(_)` Call-destination stamping pass to fix
  the `let r = divide(10, 2); match r { Ok(v) => ... }` chain. No ADR
  amendment.
- Round 11-trinity Part b (`5b113145`) — added the JIT-side
  `parametric_method_return_kind_from_receiver` to fix
  `let s = m.size()` for built-in receivers. No ADR amendment.
- Round 13 T1' (`fd028c1b`) — added the
  `MirConstant::Method(_)` Call-destination stamping pass for
  user-struct receivers via `struct_names` + `method_returns`
  resolver. No ADR amendment.

This is the same shape applied to built-in receivers + closure-return-ct
resolution — the missing fourth pass at the bytecode-side producer.

## §4. Refuse-on-sight discipline

Per the dispatch prompt's "Refuse on sight" list:

- "bool-default for unproven chained-method return kind" — the fix
  returns `Void` per §2.7.5.1 when any link in the closure-resolution
  chain is missing. No Bool-default.
- "bridge/probe/helper/hop/translator/adapter/shim descriptor for the
  propagation layer" — the new pass is named for what it does
  (`parametric_method_return_concrete_type_from_receiver_and_closure`,
  symmetric with the JIT-side `parametric_method_return_kind_from_receiver`).
- "preserve kind-blind fallback because intermediate slot kind isn't
  always available" — the existing surface-and-stop at
  `terminators.rs:572` is the correct response. No fallback added.
- Hard-coded ".map" / ".sum" arm — the fix is structurally bounded
  per the receiver+method pair classification (mirror of trinity Part
  b's 8-pair shape). The chain composition itself is structural (Call
  destination of one method becomes the receiver of the next), not a
  hard-coded chain shape.

## §5. Implementation budget

Estimated source LoC for the bounded fix (cluster-0 minimum: enough to
close Smoke 2):

- New `parametric_method_return_concrete_type_from_receiver_and_closure`
  helper in `helpers.rs` — ~80-100 LoC mirroring the JIT-side
  classifier shape (function signature + receiver-slot recovery +
  closure-slot recovery + function_return_concrete_types lookup +
  match arms for the 9 type-preserving and closure-resolving methods).
- New Call-terminator destination stamping pass in
  `infer_top_level_concrete_types_from_mir_with_resolvers` — ~30-40
  LoC. Runs after slot-move propagation, before the T1' user-struct
  pass.
- ClosureCapture statement scan (pre-pass) to build `slot →
  function_id` map for closure-operand recovery — ~15-20 LoC. The
  `function_return_concrete_types` side-table already exists (Round
  6A). The MIR shape `StatementKind::ClosureCapture { closure_slot,
  function_id, .. }` at `mir/types.rs:418` already carries
  `function_id: Option<u16>`.
- Unit tests in
  `crates/shape-vm/src/compiler/helpers.rs::tests` (or sibling test
  file if exists) — ~10-15 tests mirror trinity Part b's 12-test
  shape: per-method positive + closure-null + closure-Void-return +
  receiver-not-built-in + slot-move propagation through chain.

Total estimate: ~150-200 LoC source + tests.

Out of cluster-0 scope but folded into the same pass for
completeness (no extra cost beyond match arms):

- `.filter`, `.flatMap`, `.slice`, `.reverse`, `.sort`, `.concat`,
  `.take`, `.skip` arms for `Array<T>` (type-preserving or
  closure-resolving — same pattern).

## §6. Close gates

Post-implementation close gates (per dispatch prompt "Close criterion
(migrating-close)"):

- Kickoff Smoke 2 JIT (`[1,2,3,4,5].map(|x|x*2).sum()` → `30`) produces
  `30` matching VM output.
- `cargo check --workspace --lib --tests` EXIT=0 inside devenv.
- `cargo test -p shape-jit --lib` no regressions from baseline 383
  + new tests.
- `bash scripts/verify-merge.sh` 12/12.
- `bash scripts/check-no-dynamic.sh` EXIT=0.
- AGENTS.md row → `closed`.
- Status doc subsection
  `### W12-jit-map-chained-method-return-kind-propagation close (2026-05-13)`.

## §7. Conclusion

The failure is **§1 (a) conduit extension**, NOT ADR amendment territory.
Cluster-0 disposition confirmed: kickoff Smoke 2 JIT blocked, fold the
fix into Round 14 W12-map-chained per supervisor's Q2 ruling. Proceed
to implementation per §1.3 fix shape.

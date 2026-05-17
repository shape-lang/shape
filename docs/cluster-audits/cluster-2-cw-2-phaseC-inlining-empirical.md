# Cluster-2 closure-wave-2 Phase-C inlining empirical disposition + fix

## §0 — Metadata

- **Date:** 2026-05-16
- **Canonical parent HEAD:** `ca8300f0` (post-cluster-2-inventory-audit-day
  close; includes cluster-2-closure-wave-1 merge `cc5ceb0e` which
  RESOLVED hypothesis (a) per §A.2 of cluster-2-inventory.md).
- **Branch:** `bulldozer-strictly-typed-cluster-2-cw-2-phaseC-inlining`
- **Worktree:** `/home/dev/dev/shape-lang/shape-cluster-2-cw-2-phaseC-inlining`
- **Sub-cluster:** `cluster-2-cw-2-phaseC-inlining` (hypothesis (b)
  empirical re-verification + fix per inventory §A.3 dispatch
  recommendation).
- **Supervisor disposition (β2, 2026-05-16):** dispatch hypothesis (b)
  closure-wave-2 per cluster-2-inventory.md §A.3 "defer-within-cluster-2
  (sequential after first-phase empirical re-verification)" — Phase 1
  empirical disposition, Phase 2 fix scope unknown until empirical phase.
- **Pathway:** (iii) bounded source-change discipline — SHAPE_JIT_DEBUG-
  gated `eprintln!` for empirical Phase 1; substantive source change
  bounded to `crates/shape-vm/src/compiler/monomorphization/
  substitution.rs` for Phase 2.

## §1 — Reproduction shape

Smoke 2 canonical fixture `/tmp/smokes/s2.shape` (equivalent to
`/tmp/cluster-2-cw2-s2-fixture.shape`):

```shape
let xs = [1, 2, 3, 4, 5]
let doubled = xs.map(|x| x * 2)
let s = doubled.sum()
print(s)
```

Pre-fix at canonical `ca8300f0`:

| Run | Result |
|---|---|
| VM (`--mode vm`) | `30`, rc=0 |
| JIT (`--mode jit`) | `30`, rc=0 (correctness preserved per closure-wave-1) |

Post-fix at this branch HEAD:

| Run | Result |
|---|---|
| VM (`--mode vm`) | `30`, rc=0 |
| JIT (`--mode jit`) | `30`, rc=0 |

Full 4-fixture smoke matrix at branch HEAD:

| Smoke | VM | JIT | VM==JIT |
|---|---|---|---|
| 1 (`for i in 0..100 { sum += i }`) | `4950` | `4950` | ✓ |
| 2 (`[1,2,3,4,5].map(|x|x*2).sum()`) | `30` | `30` | ✓ |
| 3 (canonical `let t = X{}`) | `x` | `x` | ✓ |
| 4 (`Set + .add + .size`) | `2` | `2` | ✓ |

## §2 — Phase 1 empirical disposition

### §2.1 Instrumentation added

SHAPE_JIT_DEBUG-gated thread-local counters + per-visit traces inside
`inline_closure_body_into_specialization` (substitution.rs:2247) +
`inline_closure_calls_in_expr` (substitution.rs:2452):

- `[phaseC-empirical] specialization fn=X body stmt count BEFORE inline = N`
  + per-statement discriminant dump
- `[phaseC-empirical] FunctionCall name=A closure_param=B matched=bool`
  per visit
- `[phaseC-empirical] MethodCall method=M args_count=N` per visit
- `[phaseC-empirical] For statement encountered (body_stmts=N)`
- Counter summary: `fn_call_total / fn_call_match / method_call / for_stmt`

Pattern matches the existing 28-site `SHAPE_JIT_DEBUG` infrastructure
(closure.rs:261/269/438; terminators.rs:621/712; cache.rs:725).

### §2.2 Trace output at canonical `ca8300f0` for Smoke 2 fixture

```
[mono-phaseC] inline_closure_body_into_specialization
  fn=Vec.map::i64_i64_closure_0_i64_be504985afd3f65e9 closure_param=f
  closure_param_count=1 closure_body_stmts=1
[phaseC-empirical] specialization fn=Vec.map::i64_i64_closure_0_i64_be504985afd3f65e9
  body stmt count BEFORE inline = 3
[phaseC-empirical]   pre-body[0] discriminant=VariableDecl
[phaseC-empirical]   pre-body[1] discriminant=Expression
[phaseC-empirical]   pre-body[2] discriminant=Expression
[phaseC-empirical] specialization fn=Vec.map::i64_i64_closure_0_i64_be504985afd3f65e9
  body stmt count AFTER inline = 3 fn_call_total=0 fn_call_match=0
  method_call=0 for_stmt=0
[phaseC-empirical]   post-body[0] discriminant=VariableDecl
[phaseC-empirical]   post-body[1] discriminant=Expression
[phaseC-empirical]   post-body[2] discriminant=Expression
```

**Single load-bearing observation:** all four AST-visit counters are
zero. The Phase-C inliner walks the 3 top-level statements of the
specialized `Vec.map` body but `inline_closure_calls_in_expr` never
descends into any `Expr::FunctionCall`, `Expr::MethodCall`, or
`Statement::For` node. Post-inline body stmt count + discriminants are
identical to pre-inline — Phase C did NOT mutate the AST.

### §2.3 Per-explanation disposition (4 explanations from
cluster-2-v3s6f-empirical-verification.md §3.4)

| # | Explanation | Disposition | Evidence |
|---|---|---|---|
| 1 | Phase-C transform DID happen at AST layer; MIR-lowering retained callable shape | **REFUTED** | `fn_call_total=0` — the FunctionCall arm was never even reached. No transform decision made. Pre-body == post-body discriminants. |
| 2 | `closure_param_name="f"` doesn't match `Expr::FunctionCall.name` | **REFUTED** | `fn_call_total=0` means we never compared any name. No FunctionCall visit means no name mismatch to surface. |
| 3 | Recursion did not descend into `MethodCall.args` at the right level | **REFUTED at the named layer; CONFIRMED at a NEW deeper layer** | `method_call=0` AND `for_stmt=0` (so `Statement::For` is not the wrapper) AND `fn_call_total=0` (so no descent below). The actual gap: `inline_closure_calls_in_expr` (substitution.rs:2491-2703) lacks an `Expr::For` arm — Vec.map's body is `Statement::Expression(Expr::For(...))` and `Expr::For` falls through to `other => other.clone()`. The recursion never descends into the for-loop body. **This is a §"exhaustive match rule" gap per CLAUDE.md.** |
| 4 | Post-Phase-C transform re-introduces a Call shape | **REFUTED** | The Call was never removed in the first place. The MIR `Call(Copy(SlotId(2)), [item])` in bb3/bb4 reflects faithful lowering of the un-modified specialized AST. |

**Architectural-source root cause:** `inline_closure_calls_in_expr` is
missing arms for the following `Expr` variants that can wrap
sub-expressions which can carry the formal closure parameter call:
`Expr::For`, `Expr::While`, `Expr::Loop`, `Expr::Let`, `Expr::Match`,
`Expr::Break(Some)`, `Expr::TryOperator`, `Expr::Await`,
`Expr::AsyncScope`, `Expr::Spread`. All silently passed through
`other => other.clone()`. For Vec.map's `for item in self {
result.push(f(item)) }`, the dominant missing arm is `Expr::For`.

### §2.4 Subsidiary observation surfaced during Phase 1 (post-fix)

After adding the `Expr::For` arm and rebuilding, the Smoke 2 fixture
regressed on VM: `Error: Runtime error: no method 'sum' on receiver
kind Int64`. Re-run with `SHAPE_JIT_MIR_TRACE=1` showed the MIR shape
post-fix:

```
bb4: 5 stmts, term=Return                                          // ← BUG
  s[0]: Assign(Local(SlotId(10)), Use(Copy(Index(...))))  // item = self[idx]
  s[1]: Assign(Local(SlotId(14)), Use(Move(Local(SlotId(10)))))  // x = item
  s[2]: Assign(Local(SlotId(16)), Use(Constant(Int(2))))  // 2
  s[3]: Assign(Local(SlotId(15)), BinaryOp(Mul, Copy(14), Copy(16)))  // x * 2
  s[4]: Assign(Local(SlotId(0)), Use(Move(Local(SlotId(15)))))    // ← RETURN
```

bb4 (the for-loop body) ends with `TerminatorKind::Return` instead of
continuing to the push call. The inlined block's tail-expression was
becoming a function-level return.

**Architectural-source for the subsidiary bug:** the arrow-function /
pipe-lambda parser at
`crates/shape-ast/src/parser/expressions/functions.rs:62-64` /
`:134-135` wraps an expression-form closure body (`|x| x * 2`) as
`vec![Statement::Return(Some(expr), Span::DUMMY)]`. When
`build_inlined_closure_block` (substitution.rs:2791) inlined that
statement verbatim via `BlockItem::Statement(stmt.clone())`, the
`Statement::Return` was preserved inside the inlined block, and MIR
lowering's `lower_return_control_flow` emitted `Assign(SlotId(0),
body_value) + TerminatorKind::Return` inside the for-loop body of the
specialized fn.

**Severity:** the subsidiary observation is a same-cluster-2-cw-2
in-scope fix (per the closure-wave-1 in-scope-recovery pattern
documented in cluster-2-inventory.md §A.2 "Reading 4 candidate" — when
a hypothesis-fix sub-agent surfaces a consumer-side gap mid-execution,
in-scope fix is preferable to surface-and-stop-then-dispatch-separately).
Fix landed in the same `build_inlined_closure_block` rewrite (see §3
below).

## §3 — Phase 2 fix

### §3.1 Fix scope

**Phase-2 fix REQUIRED** per Phase-1 empirical disposition. Bounded to
`crates/shape-vm/src/compiler/monomorphization/substitution.rs`. Two
arms:

1. **Add missing `Expr` arms to `inline_closure_calls_in_expr`** at
   substitution.rs:2699 (before the `other => other.clone()`
   fall-through): `Expr::For`, `Expr::While`, `Expr::Loop`, `Expr::Let`,
   `Expr::Match`, `Expr::Break(Some)`, `Expr::TryOperator`, `Expr::Await`,
   `Expr::AsyncScope`, `Expr::Spread`. Each delegates to `rec_box` /
   `rec_vec` on sub-expressions; pattern matches the existing
   `Expr::If` / `Expr::Block` arms. Note: `Statement::While`,
   `Statement::For`, `Statement::If` are already handled in
   `inline_closure_calls_in_statement` (substitution.rs:2298-2447); the
   gap was at the expression layer where `for`/`while`/`loop`/`match`
   appear as value-returning expressions per Shape's expression-oriented
   semantics.

2. **Rewrite `build_inlined_closure_block`'s trailing-statement
   handling** at substitution.rs:2791 to lift a trailing
   `Statement::Return(Some(expr), _)` AND `Statement::Expression(expr, _)`
   into `BlockItem::Expression(expr)` instead of the previous
   `BlockItem::Statement(stmt.clone())`. The lift preserves the closure
   body's tail-expression value semantics AND prevents
   `Statement::Return` from emitting a function-level
   `lower_return_control_flow` inside the for-loop body of the
   specialized fn.

### §3.2 LoC delta + files

```
crates/shape-vm/src/compiler/monomorphization/substitution.rs |
  +228 / -2
```

Single file, +226 net. The empirical-phase traces account for ~100 LoC
(SHAPE_JIT_DEBUG-gated; kept as standing observability per the existing
28-site infrastructure pattern). Fix-phase substantive code is ~120 LoC
across the two arms above.

### §3.3 Post-fix verification

#### §3.3.1 Post-fix MIR shape (bb4 — the for-loop body)

```
bb4: 5 stmts, term=Call { func: Constant(Method("push")),
                          args: [Copy(Local(SlotId(3))),
                                 Move(Local(SlotId(13)))],
                          destination: Local(SlotId(12)),
                          next: BasicBlockId(6) }
  s[0]: Assign(Local(SlotId(10)), Use(Copy(Index(...))))
  s[1]: Assign(Local(SlotId(14)), Use(Move(Local(SlotId(10)))))
  s[2]: Assign(Local(SlotId(16)), Use(Constant(Int(2))))
  s[3]: Assign(Local(SlotId(15)), BinaryOp(Mul, Copy(14), Copy(16)))   ← inlined x*2
  s[4]: Assign(Local(SlotId(13)), Use(Copy(Local(SlotId(15)))))        ← push arg
```

bb4's terminator is now `Call(Method("push"), ...)` — the proper push
call with the inlined `x*2` result as its arg. `BinaryOp(Mul, ...)` is
present at s[3]. No `Call(Copy(SlotId(2)), [item])` (the un-inlined
indirect call). No `Return` terminator inside the for-loop body. The
load-bearing acceptance criterion ("post-MIR bb4 shows INLINED body
BinaryOp:Mul for the Vec.map(|x| x*2) fixture instead of
`Call(Copy(SlotId(2)), [item])`") is **MET**.

#### §3.3.2 Post-fix runtime (4-fixture smoke matrix)

All 4 smoke fixtures execute identically under VM and JIT modes (see
§1 table). Smoke 2 JIT remains `30` — correctness preserved.

#### §3.3.3 Post-fix Phase-C empirical trace

```
[phaseC-empirical] specialization fn=Vec.map::i64_i64_closure_0_i64_be504985afd3f65e9
  body stmt count AFTER inline = 3 fn_call_total=1 fn_call_match=1
  method_call=1 for_stmt=0
[phaseC-empirical]   MethodCall method="push" args_count=1
[phaseC-empirical]   FunctionCall name="f" closure_param="f" matched=true
```

Counter advances: `method_call=1` (the `result.push(...)` MethodCall
visited via the recursion now descending through `Expr::For`),
`fn_call_total=1` + `fn_call_match=1` (the `f(item)` FunctionCall
matched the closure parameter and was replaced by
`build_inlined_closure_block`).

### §3.4 ADR-fit

ADR-006 §2.7.5 stamp-at-compile-time. Phase-C inlining is a
compile-time optimization that eliminates a runtime indirect call (the
deleted kind-blind `op_call_value` / `call_value_immediate_*` pattern
per CLAUDE.md §"Renames to refuse on sight" Q12 family) by emitting the
closure body inline at the call site. The fix completes Phase-C's
contract: its AST traversal now reaches every position where a
closure-param call can syntactically appear. The carrier ABI for the
post-inline AST is identical to the pre-Phase-C path (the closure
parameter slot continues to receive `MakeClosure` per the Phase-C scope
note at substitution.rs:2257-2273 — Phase D/E/H work for capture
hoisting + closure-param stripping is unchanged).

### §3.5 Cascade-site count

0 — bounded to `substitution.rs` as predicted by inventory §A.3
"cascade-site estimate: 0 (bounded to the inliner + its callers)".

## §4 — Imprecisions surfaced

Per the imprecision-instance-counting discipline tracked across the
phase-3 cluster trajectory (41 cumulative through cluster-0+1 close
attempt + cluster-2 closure-wave-1 close + cluster-2-inventory-audit-day
close):

### Imprecision 33 (cluster-2-cw-2 instance 1)

Dispatch text §A.3 / cluster-2-v3s6f-empirical-verification.md §3.4
explanation 3 named the recursion-descent gap at the `MethodCall.args`
layer ("recursion at substitution.rs:2516 calls `rec_vec(args)` which
should reach the nested `f(item)`. Verifiable by per-arm trace.").
Empirical disposition surfaced the gap at the `Expr::For` arm (one
layer outer than the named MethodCall.args), with the MethodCall.args
arm itself working correctly (`method_call=1` post-fix). Same shape as
the architectural-prediction-subclass pattern observed across V3-S6a..e
(per phase-3-cluster-0-status.md §7825-7829, instances 37-41) — the
named layer is correct in itself; the gap is one layer inside the named
layer. The empirical-verification deliverable surfaced this same
pattern at the producer-side vs consumer-side boundary (§2.4) — here it
recurs at the inner-vs-outer expression layer.

### Imprecision 34 (cluster-2-cw-2 instance 2)

Dispatch text "Phase 2 (fix): scope unknown until empirical phase;
bounded to substitution.rs inliner + callers per empirical-verification
§3.5; (b) MAY be resolved by (a)'s side-effects (no fix needed) per
inventory §A.3 Reading-4-pattern prediction." Reading-4-pattern
prediction REFUTED for the §A.3 case — (b) was NOT resolved by (a)'s
side-effects. (a)'s fix made the bb3 Call execute per-iteration (which
unblocked correctness via the runtime indirect-call infrastructure
working correctly per closure-wave-1 close), but the Phase-C AST inliner
remained un-fired for the Vec.map fixture (pre-flight prediction:
"(b) MAY be resolved by (a)'s side-effects"; ground-truth: Phase-C IS
invoked but produces a no-op transform due to the Expr::For
recursion-descent gap surfaced as imprecision 33).

### Imprecision 35 (cluster-2-cw-2 instance 3)

Phase-2 fix arm 1 (add Expr::For arm) was structurally complete per the
load-bearing acceptance criterion ("post-MIR bb4 shows INLINED body
BinaryOp:Mul"). But it introduced a subsidiary regression (bb4
terminator became `Return` instead of `Goto(push call continuation)`)
that surfaced as a VM runtime error (`no method 'sum' on receiver kind
Int64`). The subsidiary regression's root cause is the
arrow-function parser wrapping closure-body expressions as
`Statement::Return(Some(expr), _)` per
`crates/shape-ast/src/parser/expressions/functions.rs:62-64`, AND
`build_inlined_closure_block` (substitution.rs:2791) preserving that
`Statement::Return` verbatim inside the inlined block. The pre-existing
test suite for `build_inlined_closure_block` (substitution.rs:2825..)
did not catch this because the existing tests appear to use
`Statement::Expression` closure bodies + the existing Phase-C call
sites never invoked the inliner end-to-end on a closure-body wrapped as
`Statement::Return` (closure-wave-1's fix at the for-loop iterator
state machine was the prerequisite to making the inlined block reach
MIR lowering as the for-body argument). In-scope fix landed in the
same `build_inlined_closure_block` rewrite per the in-scope-recovery
pattern documented in inventory §A.2.

Cumulative count: 41 (pre-cluster-2-cw-2) + 3 (cluster-2-cw-2 above) =
**44 imprecision instances across phase-3 trajectory**.

## §5 — Out-of-scope observations (Round 2 tracking territory)

Per dispatch §B section "2 UNCOVERED user-fn classes (Round 2 tracking
territory; do NOT pre-empt)": none surfaced. The fix in this dispatch is
bounded to Phase-C closure-aware monomorphization and does not touch
Class B (closure body with inferred typed-array param) or Class C
(intermediate slots from method-chain composition).

## §6 — CLAUDE.md modification candidates (flag only)

None surfaced. The fix is bounded to existing infrastructure
(`inline_closure_calls_in_expr` recursion + `build_inlined_closure_block`
output shape). No new opcode, no new HeapKind, no ADR-006 amendment, no
new forbidden pattern, no new defection-attractor framing.

## §7 — Ceiling-c + D-α status

- **Ceiling-c (architectural-prediction-subclass instance count):**
  3 new instances surfaced (imprecisions 33, 34, 35). All caught at the
  empirical layer (not at merge time). Cumulative 44 across phase-3
  trajectory; pattern continues to manifest at every empirical
  disposition that crosses an architecturally-deep boundary.
- **D-α (sub-cluster cleanness):** cluster-2-cw-2 was clean per the
  in-scope-recovery pattern (Reading-4-binding from inventory §A.2):
  subsidiary regression surfaced (imprecision 35) was fixed in-scope
  within the same `build_inlined_closure_block` rewrite. No
  surface-and-stop separate dispatch required. 4/4 smoke matrix VM ==
  JIT preserved + 12/12 verify-merge.sh + check-no-dynamic.sh EXIT=0 +
  cargo check --workspace --lib --tests EXIT=0.

## §8 — Close gate verification

| Gate | Result |
|---|---|
| `cargo check --workspace --lib --tests` (via devenv shell) | EXIT=0 |
| `bash scripts/verify-merge.sh` (via devenv shell) | 12/12 PASS |
| `bash scripts/check-no-dynamic.sh` | EXIT=0 |
| Smoke matrix 4/4 VM == JIT at canonical fixture | PASS (1=4950/4950; 2=30/30; 3=x/x; 4=2/2) |
| Smoke 2 JIT returns 30 (correctness preserved) | PASS |
| Post-MIR bb4 shows inlined BinaryOp:Mul instead of un-inlined Call | PASS (load-bearing acceptance criterion MET per §3.3.1) |
| Deliverable doc with per-explanation disposition + fix scope | PASS (this doc) |
| AGENTS.md row appended | TODO at close commit |
| NO `Co-Authored-By: Claude` trailer | TODO at close commit |

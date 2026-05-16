# Cluster-2 V3-S6f empirical verification — Smoke 2 JIT runtime-execution gap disposition

## §0 — Metadata

- **Date:** 2026-05-16
- **Canonical HEAD:** `bb5b2109` (phase-3 cluster-0+1 close attempt with V3-S6 5-checkpoint chain merged at `50e5c024`).
- **Branch:** `bulldozer-strictly-typed-cluster-2-empirical-verification`
- **Worktree:** `/home/dev/dev/shape-lang/shape-cluster-2-empirical-verification`
- **Sub-cluster:** `cluster-2-empirical-verification`
- **Supervisor disposition (β2, 2026-05-16):** Smoke 2 JIT runtime-execution gap (rc=124 TIMEOUT inside specialized `Vec.map` body) is the cluster-2 canonical fixture for §2.7.5 JIT-side conduit completion. This deliverable empirically disposes the V3-S6e sub-agent's hypothesis enumeration (a)/(b)/(c) — NO hypothesis fixes, NO tracing-crate migration, NO architectural rebuild (those are closure-wave territory dispatched after this disposition lands).
- **Pathway:** (iii) bounded source-change discipline — SHAPE_JIT_DEBUG-gated `eprintln!` only; pattern matches existing infrastructure exactly.

## §1 — Reproduction shape

### Fixture

`/tmp/cluster-2-s2-fixture.shape`:

```shape
// V3-S6f empirical-verification fixture: Smoke 2 canonical form.
// Per phase-3-cluster-0-status.md §7785: rc=124 TIMEOUT under --mode jit.
let xs = [1, 2, 3, 4, 5]
let doubled = xs.map(|x| x * 2)
let s = doubled.sum()
print(s)
```

### Environment

```
$ cd /home/dev/dev/shape-lang/shape-cluster-2-empirical-verification
$ devenv shell --quiet -- bash -c "cd $PWD && cargo build --release --bin shape"
$ ./target/release/shape run --mode jit /tmp/cluster-2-s2-fixture.shape    # JIT
$ ./target/release/shape run --mode vm  /tmp/cluster-2-s2-fixture.shape    # VM baseline
```

### Outcome (verified at HEAD bb5b2109 with bounded SHAPE_JIT_DEBUG trace
points added per §7)

| Run | Result |
|---|---|
| VM (`--mode vm`) | prints `30`, rc=0 |
| JIT (`--mode jit`) | hangs forever; SIGTERM after 30 s; rc=124 |

Verbatim repro:

```
$ timeout 30 ./target/release/shape run --mode jit /tmp/cluster-2-s2-fixture.shape > /tmp/jit_run.out 2>&1; echo "JIT_RC=$?"
JIT_RC=124
```

The JIT successfully *compiles* every function — `[jit-debug] compilation
OK, about to execute...` fires (`crates/shape-jit/src/executor.rs:264`),
including the specialized `Vec.map::i64_i64_closure_0_i64_be504985afd3f65e9`
(emitted by closure-aware monomorphization per §3 below). It is the
*runtime execution* of that specialized body that loops forever.

## §2 — Hypothesis (a) for-loop iterator state-machine for v2 typed-array source

### §2.1 Existing instrumentation coverage at HEAD bb5b2109

- `crates/shape-jit/src/mir_compiler/mod.rs:968` — `SHAPE_JIT_MIR_TRACE`
  prints every basic-block + statement + terminator of every JIT-compiled
  MIR function. This is the single piece of existing instrumentation that
  surfaces hypothesis (a) directly: by enabling it, the specialized
  `Vec.map` body's MIR shape becomes visible verbatim.
- `crates/shape-jit/src/compiler/program.rs:725` (`[jit-mir] compiling
  '...' idx=...`) — confirms WHICH functions reach MIR-JIT compilation.

No `SHAPE_JIT_*` env-var trace sites exist in `crates/shape-vm/src/mir/`
at HEAD bb5b2109 (grep `SHAPE_JIT_DEBUG\|SHAPE_JIT_MIR_TRACE
crates/shape-vm/` returns zero). The MIR-lowering pass that *produces*
the iterator-state-machine shape lives in `shape-vm` and was previously
un-instrumented; the consumer trace at `shape-jit/mir_compiler/mod.rs:968`
was sufficient to surface the broken MIR shape but not to identify which
producer-site emitted it.

### §2.2 New trace points added

One SHAPE_JIT_DEBUG-gated `eprintln!` at the architectural-source root
cause site (per §2.4 below). The trace point is in `crates/shape-vm/src/`
even though `SHAPE_JIT_DEBUG` is the env-var name — every existing
SHAPE_JIT_DEBUG site fires unconditionally under `--mode jit` and the
trace describes a JIT-relevant MIR shape, so the name is consistent with
existing infrastructure semantics. The trace point in
`crates/shape-vm/src/compiler/monomorphization/cache.rs` (§3.2 below) is
also under `SHAPE_JIT_DEBUG` for the same reason — closure-aware
monomorphization affects JIT execution exclusively (the bytecode-VM-side
`compile_for_loop` at `crates/shape-vm/src/compiler/loops.rs:298` is the
live VM execution path and is unaffected by MIR-side specialization).

**Pre-flight observation (architectural-prediction-subclass instance
29 — see §8):** the dispatch's required-reading entry §6 cited
`crates/shape-jit/src/mir_compiler/terminators.rs` line 176 (V3-S6c
routing block) and lines 621/712 (existing SHAPE_JIT_DEBUG sites in the
same file) as hypothesis (a) territory. Ground-truth at HEAD bb5b2109
verified all three cites; the terminators.rs:621/712 sites are
print-path surface-and-stop traces, NOT iterator-state-machine
emission traces. Hypothesis (a) territory is in `shape-vm/src/mir/
lowering/` (producer side), not `shape-jit/src/mir_compiler/`
(consumer side) — the dispatch's locus description was producer-side-
under-qualified. Per the Q3 recursive pre-flight binding, surfaced
back at this section without expanding the cite (the cite-as-given was
valid for the JIT-side consumer; the producer-side root cause needed
its own ground-truth pass).

#### Trace point — `crates/shape-vm/src/mir/lowering/expr.rs:1036` (line numbers approximate; see §7)

Gated SHAPE_JIT_DEBUG site in `lower_for_expr`'s generic (non-Range)
iterator branch:

```rust
} else {
    // Generic iterator path (non-range iterators).
    // This is a placeholder — full iterator protocol not yet implemented in MIR.
    if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
        eprintln!(
            "[mir-for-expr-generic-stub] lower_for_expr generic-\
             iterator STUB emitted: iter_slot SwitchBool, pattern \
             slot assigned MirConstant::None, body block, \
             unconditional Goto(header) — no iterator advance, no \
             termination condition. iter_kind={:?} span={:?}",
            std::any::type_name_of_val(for_expr.iterable.as_ref()),
            span,
        );
    }
    builder.push_scope();
    let iter_slot = lower_expr_to_temp(builder, &for_expr.iterable);
    // ...
}
```

The pre-existing `// This is a placeholder — full iterator protocol not
yet implemented in MIR.` comment at `expr.rs:1037` (HEAD bb5b2109,
unchanged) already documents the gap. The trace point makes the gap
runtime-observable under SHAPE_JIT_DEBUG=1.

### §2.3 Trace output evidence (verbatim from
`SHAPE_JIT_DEBUG=1 SHAPE_JIT_MIR_TRACE=1 timeout 5 ./target/release/shape
run --mode jit /tmp/cluster-2-s2-fixture.shape`)

#### Specialized `Vec.map` MIR shape (verbatim from
`[mir-trace]` lines bracketed by `compiling 'Vec.map::i64_i64_closure_0_i64_be504985afd3f65e9' idx=195`)

```
[jit-mir] compiling 'Vec.map::i64_i64_closure_0_i64_be504985afd3f65e9' idx=195
[mir-trace] bb0: 4 stmts, term=Goto(BasicBlockId(2))
[mir-trace]   s[0]: Assign(Local(SlotId(4)), Aggregate([]))
[mir-trace]   s[1]: Assign(Local(SlotId(3)), Use(Move(Local(SlotId(4)))))
[mir-trace]   s[2]: Assign(Local(SlotId(6)), Use(Copy(Local(SlotId(1)))))
[mir-trace]   s[3]: Assign(Local(SlotId(5)), Use(Constant(None)))
[mir-trace] bb1: 0 stmts, term=Return
[mir-trace] bb2: 0 stmts, term=SwitchBool { operand: Copy(Local(SlotId(6))), true_bb: BasicBlockId(3), false_bb: BasicBlockId(4) }
[mir-trace] bb3: 2 stmts, term=Call { func: Copy(Local(SlotId(2))), args: [Copy(Local(SlotId(8)))], destination: Local(SlotId(11)), next: BasicBlockId(5) }
[mir-trace]   s[0]: Assign(Local(SlotId(7)), Use(Constant(None)))
[mir-trace]   s[1]: Assign(Local(SlotId(8)), Use(Copy(Local(SlotId(7)))))
[mir-trace] bb4: 1 stmts, term=Return
[mir-trace]   s[0]: Assign(Local(SlotId(0)), Use(Move(Local(SlotId(3)))))
[mir-trace] bb5: 0 stmts, term=Call { func: Constant(Method("push")), args: [Copy(Local(SlotId(3))), Move(Local(SlotId(11)))], destination: Local(SlotId(10)), next: BasicBlockId(6) }
[mir-trace] bb6: 2 stmts, term=Goto(BasicBlockId(2))
[mir-trace]   s[0]: Assign(Local(SlotId(9)), Use(Copy(Local(SlotId(10)))))
[mir-trace]   s[1]: Assign(Local(SlotId(5)), Use(Copy(Local(SlotId(9)))))
[mir-trace] bb7: 0 stmts, term=Goto(BasicBlockId(1))
[jit-mir] Compiled function 'Vec.map::i64_i64_closure_0_i64_be504985afd3f65e9' via MirToIR
```

#### Slot map for the specialized fn (derived; design observation
from reading the MIR — see §0 "design observation, not ground truth"
annotation)

- Slot 1 = `self` (receiver) — Vec.map's first param
- Slot 2 = `f` (closure pointer) — Vec.map's second param (preserved
  per Phase-C inlining scope-note at `substitution.rs:2257-2273`)
- Slot 3 = `result` (the `let mut result = []` binding)
- Slot 4 = the `[]` empty-array literal temp
- Slot 5 = for-expr loop-result temp (`Constant(None)` per stub)
- Slot 6 = `iter_slot` (`Copy(Slot 1)` — the iterable evaluated once)
- Slot 7 = `elem_slot` (`Constant(None)` per stub)
- Slot 8 = `item` (the pattern binding destructured from `elem_slot`)
- Slot 11 = call destination (Slot 11 = f(item) result)
- Slots 9/10 = push call destination + write-back

#### Generic-iterator-stub trace (verbatim from `[mir-for-expr-generic-stub]` lines)

```
[mir-for-expr-generic-stub] lower_for_expr generic-iterator STUB emitted:
  iter_slot SwitchBool, pattern slot assigned MirConstant::None, body
  block, unconditional Goto(header) — no iterator advance, no
  termination condition. iter_kind="shape_ast::ast::expressions::Expr"
  span=Span { start: 1074, end: 1140 }
[mir-for-expr-generic-stub] ... span=Span { start: 4869, end: 4977 }
[mir-for-expr-generic-stub] ... span=Span { start: 8573, end: 8742 }
```

The first span (1074..1140) corresponds to the `for item in self`
loop inside the `Vec.map` body in
`crates/shape-runtime/stdlib-src/core/vec.shape:53-55`. The second
(4869..4977) and third (8573..8742) spans are sibling for-loops in
the same stdlib file (e.g. `Vec.filter` at `vec.shape:42-48` and
`Vec.reduce` at `vec.shape:60-64`) — all consumed by the same
generic-iterator stub path.

### §2.4 Disposition — **CONFIRMED**

Hypothesis (a) is **CONFIRMED** at the architectural-source layer with
a refined locus from the dispatch's enumeration.

**Architectural-source root cause:** the MIR-lowering pass for `for x
in iter` over non-Range iterables (`crates/shape-vm/src/mir/lowering/
expr.rs:1035-1094` at HEAD bb5b2109) emits a stub iterator state
machine:

1. evaluates the iterable expression once into `iter_slot`
   (line 1040 — `let iter_slot = lower_expr_to_temp(builder, &for_expr.iterable);`);
2. assigns the loop-result temp to `MirConstant::None` (lines 1046-1052);
3. emits `TerminatorKind::Goto(header)` (line 1053);
4. in the header, emits `TerminatorKind::SwitchBool { operand:
   Copy(iter_slot), true_bb: body_block, false_bb: after }`
   (lines 1056-1063) — `SwitchBool` branches on the truthiness of
   the iterable, which for a v2-raw `TypedArray<i64>` pointer per
   ADR-006 §2.3 is always non-null (truthy);
5. in the body block, assigns the pattern slot to `MirConstant::None`
   (lines 1066-1072), destructures bindings from that None (lines
   1073-1079);
6. lowers the body (line 1081);
7. emits unconditional `TerminatorKind::Goto(header)` (line 1090).

There is no `IterDone` / `IterNext` opcode emission, no counter
advance, no termination condition. The generic-iterator branch is a
literal placeholder (per the inline comment at `expr.rs:1037` —
unchanged at HEAD bb5b2109 — "This is a placeholder — full iterator
protocol not yet implemented in MIR.").

**JIT execution consequence (empirically confirmed):** the JIT
faithfully lowers this MIR to Cranelift IR. At runtime, `bb2`'s
`SwitchBool(Copy(iter_slot))` reads the non-null `TypedArray<i64>`
pointer from `Slot 6` as truthy, jumps to `bb3`, executes the body
(item=None, f(item) call, push result), `bb6` unconditional
`Goto(bb2)` — infinite loop. The `[jit-debug] compilation OK, about
to execute...` line fires (`executor.rs:264`) followed by infinite
loop in the JIT-emitted body; no further trace fires (the ARC counter
report at `executor.rs:282-294` fires AFTER `jit_fn` returns, which
never happens).

**Refinement from dispatch's enumeration:** the dispatch named the
locus as "crates/shape-jit/src/mir_compiler/ iterator state-machine
emission + JIT-compiled iterator for v2 TypedArray source." The
actual locus is `crates/shape-vm/src/mir/lowering/expr.rs:1035-1094`
— a PRODUCER-side gap (MIR shape itself is broken), not a CONSUMER-
side gap (JIT iterator codegen). The JIT correctly lowers the MIR it
receives; the MIR it receives is the broken-stub shape. This is the
architectural-prediction-subclass pattern observed across V3-S6a..e
(per phase-3-cluster-0-status.md §7825-7829, instances 37-41) —
pre-flight catches the layer explicitly enumerated, misses the next
inner layer; here it misses producer-side vs consumer-side.

**Empirically supplementary observation (design observation, not
ground truth):** the `lower_for_loop` function at `crates/shape-vm/
src/mir/lowering/stmt.rs:464-559` ALSO contains a structurally-similar
ForIn stub at lines 471-516. That function's `Statement::For` arm in
`lower_statement` (`stmt.rs:72-74`) was empirically NEVER called for
the Smoke 2 fixture (zero hits on a temporary unconditional
`eprintln!` placed at the dispatch line during empirical verification
— removed pre-close per bounded source-change discipline). Every
for-loop in the fixture's reachable AST (the user-level `for x in
xs` AND the stdlib-side `for item in self` inside Vec.map's body)
arrives at MIR lowering wrapped in `Statement::Expression(Expr::For(
ForExpr, _))`, dispatched through `lower_expr` to `lower_for_expr`.
Whether the `Statement::For` arm is reachable by *any* AST shape at
HEAD bb5b2109 is unconfirmed; the dispatch's required-reading entry
§5 (stmt.rs line 471 ForIn stub) is structurally identical to the
expr.rs site but is in a code path the empirical test did not
exercise. The disposition above stands on the expr.rs:1035-1094 site
exclusively.

### §2.5 Recommended closure-wave shape

**Territory:** `crates/shape-vm/src/mir/lowering/expr.rs:1035-1094`
(`lower_for_expr` generic-iterator branch) PLUS the structurally-
identical `lower_for_loop` ForIn arm at `stmt.rs:471-516` (whether or
not it is empirically reachable — landing the iterator protocol at one
site without the mirror site invites future drift per the
parallel-implementation defection-attractor pattern).

**Migration design (design observation, not ground truth — closure-
wave agent must verify the design fits the actual ADR-006 §2.7 family
constraints):**

The bytecode-VM-side `compile_for_loop` at `crates/shape-vm/src/
compiler/loops.rs:298-516` is the reference implementation — it
already emits a complete iterator state machine with `IterDone` /
`IterNext` opcodes and an explicit index counter. The MIR-side
lowering needs the same shape lowered to MIR statements + terminators:

1. Allocate an index counter slot, initialize to 0
2. Loop header: emit `IterDone` (or equivalent MIR pseudo-statement)
   reading iter_slot + idx_local → bool slot
3. Terminator: `SwitchBool { operand: bool_slot, true_bb: after,
   false_bb: body_block }` (NOT branching on `iter_slot` truthiness)
4. Body: emit `IterNext` reading iter_slot + idx_local → pattern slot;
   destructure pattern; lower body
5. Pre-Goto-back-to-header: increment idx_local
6. `Goto(header)`

The complication: MIR uses `StatementKind` and `Rvalue` variants, not
opcodes directly. The `IterDone` / `IterNext` opcodes are bytecode-
level constructs. A MIR-level iterator protocol would either:

- (i) introduce new `StatementKind::IterDone` / `IterNext` variants
  (changes the MIR enum surface; ripples to every MIR consumer per
  the "exhaustive match rule" in CLAUDE.md ~8+ file cascade);
- (ii) lower iterator advancement to existing primitives —
  `TypedArray<T>::len` PHF method dispatch + index access (`xs[idx]`),
  modelled as `MirConstant::Method("len")` Call terminators + Place::
  Index reads. The MIR shape would match the bytecode shape for
  typed-array iteration via index-based access.

**Approach (ii)** is the lower-disruption migration: it reuses
existing MIR vocabulary (Call terminator, Place::Index, BinaryOp::Add
for counter increment, BinaryOp::Lt for bounds check) and does not
introduce new MIR variants. Approach (i) is more direct but requires
the exhaustive-match cascade and JIT-side codegen for the new
statement kinds.

**ADR-fit cite:** ADR-006 §2.7.5 stamp-at-compile-time — the iterator
kind (TypedArray<i64> vs HashMap iter vs HashSet iter vs Range iter
etc.) is a *compile-time* property derivable from the iterable's
ConcreteType. The MIR-lowering pass should dispatch on
iterable-ConcreteType at compile time and emit the type-specialized
state machine for each kind (no runtime tag check, no `is_iterator()`
probe — those would be §Renames-to-refuse-on-sight defection-
attractor descriptors).

**Agent partition shape recommendation:** single sub-cluster, single
agent. The migration is structurally bounded — two MIR-lowering
functions (the expr.rs:1035 generic branch + the stmt.rs:471 ForIn
arm if confirmed reachable; if not, just the expr.rs site) + per-
ConcreteType-iterable dispatch (Array, HashMap, HashSet, Deque,
PriorityQueue, Range — each ≤ 50 LoC mirror of the bytecode-VM-side
loops.rs:298-516 reference). Estimated 400-600 LoC total. Approach
(ii) — no new MIR variants — keeps the cross-crate cascade to zero
new MIR consumer arms.

**Cascade-site estimate:** 0 (approach ii — only the two producer
sites change). Approach (i) cascade-site estimate: ~8 files per the
exhaustive-match rule.

## §3 — Hypothesis (b) closure-call indirect-call shape inside inlined specialization

### §3.1 Existing instrumentation coverage at HEAD bb5b2109

- `crates/shape-jit/src/ffi/object/closure.rs:261/269/438` —
  SHAPE_JIT_DEBUG-gated retain/release traces (JIT-side closure FFI
  lifecycle). These fire WHEN the JIT-emitted code actually executes
  retain/release calls; they do not fire when the JIT body loops
  before reaching a retain call.
- `crates/shape-jit/src/mir_compiler/terminators.rs:176` — V3-S6c
  routing block (compile-time direct-FuncRef call when the side-table
  has a specialized index; bypasses the trampoline). This fires at
  JIT-compile time; visible via SHAPE_JIT_DEBUG indirectly via the
  surrounding `[jit-debug]` traces.
- No existing trace at the *closure-aware monomorphization Phase-C
  inlining call site* (`crates/shape-vm/src/compiler/monomorphization/
  cache.rs:720`). The dispatch's required-reading entry §7 cited
  `substitution.rs:2247` `inline_closure_body_into_specialization`
  as hypothesis (b) territory; verified at HEAD bb5b2109. The cite is
  the inliner *definition*, not a call-site trace point.

### §3.2 New trace points added

One SHAPE_JIT_DEBUG-gated trace at the Phase-C inlining call site
(see §7 for exact text):

`crates/shape-vm/src/compiler/monomorphization/cache.rs:720` (within
the for-loop at line 718) — `[mono-phaseC]
inline_closure_body_into_specialization fn=... closure_param=...
closure_param_count=... closure_body_stmts=...`. Plus a paired trace
on the `is_err()` branch (line 728) — `[mono-phaseC] inline FAILED
for fn=...`.

### §3.3 Trace output evidence

```
[mono-phaseC] inline_closure_body_into_specialization fn=Vec.map::i64_i64_closure_0_i64_be504985afd3f65e9 closure_param=f closure_param_count=1 closure_body_stmts=1
```

The `[mono-phaseC] inline FAILED` trace does NOT fire — meaning
Phase-C closure inlining returned `Ok(())` for the
`Vec.map::i64_i64_closure_0_i64_be504985afd3f65e9` specialization.
The specialized function was registered + compiled with the
closure-aware key (per the name suffix `_closure_0_i64_b...` which is
only produced by `build_mono_key_with_closures` at
`crates/shape-vm/src/compiler/monomorphization/type_resolution.rs:703-735`).

### §3.4 Disposition — **PARTIAL** (residual sub-finding surfaces;
not the dominant cause of Smoke 2 JIT TIMEOUT)

Hypothesis (b) is **PARTIAL** — the closure-call shape inside the
inlined specialization shows an UNEXPECTED-AT-EMPIRICAL-LAYER
discrepancy between trace-confirmed Phase-C inlining success and the
post-MIR-lowering Call-terminator in `bb3`:

**Empirical evidence:** Phase-C inlining was invoked and returned Ok(())
for the specialization (per §3.3 trace). The expected post-inlining AST
shape inside the specialized `Vec.map` body is
`result.push({ let x = item; x * 2 })` per the `inline_closure_calls_in_expr`
match at `substitution.rs:2463-2479` (FunctionCall whose name matches
the closure param name is replaced by `build_inlined_closure_block` at
`substitution.rs:2601-2630`). MIR lowering of that inlined AST would
produce `Assign(slot_x, Copy(item_slot))` + `Assign(slot_result,
BinaryOp(Mul, Copy(slot_x), Constant(Int(2))))` inside `bb3` — NO
`Call` terminator.

**Observed MIR (per §2.3):** `bb3` terminator is `Call { func:
Copy(Local(SlotId(2))), args: [Copy(Local(SlotId(8)))], destination:
Local(SlotId(11)), next: BasicBlockId(5) }`. `SlotId(2)` is the `f`
parameter slot per the slot-map in §2.3 (design observation, not
ground truth). The Call shape indicates the specialized fn IS still
calling its closure parameter at runtime — Phase-C inlining at the
AST layer did not produce the expected post-inlining MIR shape.

**Possible explanations (each requires its own empirical pass to
disposition definitively):**

1. The Phase-C inlining DID transform the AST but the MIR-lowering of
   the inlined `Expr::Block` retained a callable shape — unlikely per
   the lowering pipeline, but verifiable by adding an AST-dump trace
   inside `compile_function` for the specialized def.
2. The `closure_param_name` passed to Phase-C ("f") does not match the
   `Expr::FunctionCall { name }` shape in the AST after type-only
   substitution. Verifiable by tracing `inline_closure_calls_in_expr`
   match-arm decisions inside the recursive walk.
3. The Phase-C inlining ran on the correct AST but the recursion did
   not descend into `Expr::MethodCall { receiver, method: "push",
   args: [...] }`'s `args` vec at the right level — the recursion at
   `substitution.rs:2516` calls `rec_vec(args)` which should reach the
   nested `f(item)`. Verifiable by per-arm trace.
4. A separate post-Phase-C transform (mir_schema_threading, borrow
   solver, closure_function_ids back-patching at
   `functions.rs:491-549`) re-introduces a Call shape from a different
   producer-site.

**Severity for Smoke 2 JIT TIMEOUT:** even if Phase-C inlining had
produced the expected `BinaryOp(Mul, ...)` shape in bb3, the
hypothesis (a) infinite loop at the bb2 `SwitchBool(iter_slot)` /
bb6 unconditional `Goto(bb2)` cycle would STILL hang the JIT-
emitted code. Hypothesis (b) is NOT the dominant root cause of the
TIMEOUT; the dominant root cause is hypothesis (a). Hypothesis (b)
is a separate latent gap that would only become observable AFTER
hypothesis (a) is fixed.

### §3.5 Recommended closure-wave shape

**Disposition:** defer hypothesis (b) to a separate cluster-2 sub-
cluster, dispatched AFTER hypothesis (a)'s closure-wave (per §2.5
above) lands. The reasoning:

1. With hypothesis (a) outstanding, the JIT cannot reach the bb3
   Call to exercise hypothesis (b) at runtime — the infinite loop at
   bb2 means bb3 executes once-then-loops-forever. Even if Phase-C
   inlining were trivially fixed, the JIT body still hangs.
2. With hypothesis (a) FIXED, the bb3 Call shape becomes a
   per-iteration `f(item)` indirect call to the `__closure_0` user
   function — the call-value ABI per ADR-006 §2.7.11 / Q12 + the
   compile-time direct-FuncRef routing per the V3-S6c routing block
   at terminators.rs:176. Whether the JIT correctly executes that
   call shape (or whether Phase-C inlining failure cascades to
   another sub-gap) is empirically dispositionable only AFTER bb2's
   termination is structurally correct.

**Territory if hypothesis (b) closure-wave is dispatched:**
`crates/shape-vm/src/compiler/monomorphization/substitution.rs:2247`
`inline_closure_body_into_specialization` + its recursive helpers
(`inline_closure_calls_in_statement` at substitution.rs:2298,
`inline_closure_calls_in_expr` at substitution.rs:2452,
`build_inlined_closure_block` at substitution.rs:2601). Plus the
post-Phase-C MIR-build pipeline (`compile_function` at
functions.rs:181 → `lower_function_detailed` at mir/lowering/mod.rs:607).

**Cascade-site estimate:** 0 (bounded to the inliner + its callers).

**ADR-fit cite:** ADR-006 §2.7.10 / Q11 (MethodFnV2 ABI) + §2.7.11 /
Q12 (value-call ABI). Phase-C inlining is a compile-time optimization
that fits under §2.7.5 stamp-at-compile-time discipline — it
eliminates a runtime indirect call by emitting the closure body
inline. Failure to inline correctly is a §2.7.5 conduit gap at the
AST→MIR boundary.

## §4 — Hypothesis (c) receiver-self slot kind threading

### §4.1 Existing instrumentation coverage at HEAD bb5b2109

- `crates/shape-vm/src/compiler/helpers.rs:494`
  `infer_top_level_concrete_types_from_mir_with_resolvers` — the
  V3-S6e conduit producer site at line 623 reading
  `mir.local_typed_array_element_types`. No SHAPE_JIT_DEBUG-gated
  trace at HEAD bb5b2109.
- `crates/shape-vm/src/mir/lowering/stmt.rs::lower_var_decl` —
  the V3-S6e producer-side site populating
  `mir.local_typed_array_element_types`. No SHAPE_JIT_DEBUG-gated
  trace at HEAD bb5b2109.
- `crates/shape-jit/src/mir_compiler/terminators.rs:176` — the V3-S6c
  routing block that passes the receiver bits as raw `*const
  TypedArray<i64>`. SHAPE_JIT_DEBUG-gated traces at the surrounding
  print-path arm (`:621` / `:712`) but not at the routing block
  itself.

### §4.2 New trace points added

None. Hypothesis (c) territory is exclusively in the compile-time
classifier (`function_local_concrete_types[specialized_idx][self_slot]
= Array(I64)` stamping) and the MIR shape observed in §2.3 already
disposes the question: the JIT execution gap is purely structural
(infinite loop at bb2 `SwitchBool` + bb6 `Goto` cycle), independent
of slot kind. The receiver kind on `Slot 1` does not affect the
infinite-loop behaviour — `SwitchBool(Copy(Slot 6))` reads Slot 6 =
Copy(Slot 1) (the iterable's raw bits) as truthy whether stamped as
`Array(I64)` or `Void`.

### §4.3 Trace output evidence

N/A — no new trace; existing MIR trace at §2.3 is sufficient to
dispose.

### §4.4 Disposition — **REFUTED**

Hypothesis (c) is **REFUTED** as the Smoke 2 JIT TIMEOUT root cause.
Receiver-self slot kind threading from V3-S6c routing — whether or
not `function_local_concrete_types[specialized_idx][self_slot] =
Array(I64)` is stamped — does not affect the structural infinite
loop in the specialized fn's MIR.

**Reasoning chain (design observation, anchored on the empirical MIR
shape in §2.3):**

1. The MIR-emitted infinite loop is purely structural: `bb2`
   terminator `SwitchBool { operand: Copy(SlotId(6)), true_bb: bb3,
   false_bb: bb4 }` reads Slot 6's *raw bits as a u64* and tests
   non-zero. The kind of Slot 6 (Array(I64) vs Void vs anything else)
   is consumed by *downstream* JIT codegen for typed access (e.g.
   `as_typed_array_i64`); it is NOT consumed by `SwitchBool`'s
   truthiness test.
2. The receiver bits at Slot 1 are non-null pointer per ADR-006 §2.3
   (`TypedArray<*const i64>` carrier) — non-null bits are truthy
   under `SwitchBool` regardless of NativeKind stamp.
3. `bb6` terminator `Goto(bb2)` is unconditional — there is no
   exit condition the receiver kind could affect.

**Independence from V3-S6c routing:** the V3-S6c routing at
terminators.rs:176 successfully passes the receiver bits and the JIT
successfully reaches the specialized fn's body — `[jit-debug]
compilation OK, about to execute...` fires and the specialized fn
begins executing. The hang is INSIDE the specialized fn's body, NOT
at the receiver-routing boundary. Confirmed by the absence of any
SIGSEGV / `Route A surface-and-stop` / `kind not proven` error
message in the JIT output (per `/tmp/jit_combo3.out` empirical
capture).

**Independence from V3-S6e stamping:** the V3-S6e conduit successfully
stamps `concrete_types[result_slot] = Array(I64)` for the `let mut
result = []` binding inside the specialized fn (per the V3-S6e close
report at phase-3-cluster-0-status.md §7761). That stamping fixed the
compile-time `W11-jit-new-array Aggregate` SURFACE; it does not affect
runtime execution. The TIMEOUT is at runtime, not at compile time.

### §4.5 N/A

Hypothesis (c) is refuted; no closure-wave recommendation needed.
Receiver-self slot kind threading remains correct per V3-S6c +
V3-S6e; no follow-up surfaced under this disposition.

## §5 — Cross-hypothesis observations

### Multi-hypothesis contribution

Per the dispatch's "MULTI-HYPOTHESIS DISPOSITION" clause: hypothesis
(a) is the dominant root cause of the Smoke 2 JIT TIMEOUT. Hypothesis
(b) is a latent secondary gap — Phase-C inlining is invoked but the
post-MIR-lowering shape shows an unexpected `Call` terminator;
severity is moot until hypothesis (a) is fixed. Hypothesis (c) is
refuted. There is no need for closure-wave partition supervisor-
level re-scope — hypotheses (a) and (b) are sequentially dispatched,
(c) is closed.

### Architectural-prediction subclass pattern observations

This empirical-verification dispatch surfaced one new instance of
the V3-S6 chain's architectural-prediction subclass pattern (per
phase-3-cluster-0-status.md §7821-7831, instances 37-41):

- **Instance 29-equivalent (pre-flight observation, see §8):** the
  dispatch's required-reading entry §6 cited
  `terminators.rs:176/621/712` as hypothesis (a) territory (JIT-side
  consumer). The actual locus is `mir/lowering/expr.rs:1035-1094`
  (shape-vm-side producer). This is the architectural-prediction-
  subclass pattern at the dispatch-prompt layer: the cite-as-given
  was valid for the consumer side; the producer-side root cause
  needed its own ground-truth pass. The miss was caught at empirical-
  verification step 2 (running the JIT and inspecting the MIR trace
  with the consumer-side instrumentation) — defense-in-depth via the
  existing `SHAPE_JIT_MIR_TRACE` site surfaced the broken MIR shape,
  then producer-side grep located the architectural-source.

The pattern's defense-in-depth chain — pre-flight ground-truth +
existing instrumentation + empirical execution — held at this
dispatch's layer (no bad code merged; the deliverable identifies the
root cause and the bounded scope of the fix). The cumulative
imprecision-instance count grows by 1 to 42 (one instance surfaced
during cluster-2 empirical work; see §8 below for full enumeration).

### Q25.C TraitObject rebuild territory intersection

Per the dispatch's question 10: **none of the three V3-S6f
hypotheses intersect Q25.C TraitObject rebuild territory.** Hypothesis
(a) is about iterator state-machine emission in MIR (independent of
trait-object dispatch); hypothesis (b) is about closure-call inlining
(independent of trait-object dispatch); hypothesis (c) is about
receiver-self slot kind threading (concrete TypedArray<i64> receiver,
not a `dyn Trait` receiver). Q25.C's `dyn T = box(X{})` runtime
auto-boxing rule does not affect this fixture or its root cause.

The Q25.C absorb-vs-separate decision (cluster-1.5-vs-cluster-2
absorption per audit section I, dispatched later) is therefore
**independent of this empirical-verification disposition.** Cluster-2
can absorb the hypothesis-(a) and hypothesis-(b) closure-waves without
gating on Q25.C disposition.

## §6 — Closure-wave dispatch recommendation

### Per-confirmed-hypothesis class territory + cascade-site estimate

| Hypothesis | Disposition | Territory | Cascade estimate | ADR-fit |
|---|---|---|---|---|
| (a) | CONFIRMED (dominant) | `crates/shape-vm/src/mir/lowering/expr.rs:1035-1094` + `crates/shape-vm/src/mir/lowering/stmt.rs:471-516` (if reachable) | 0 (approach ii) / ~8 (approach i) | §2.7.5 stamp-at-compile-time (per-iterable-ConcreteType monomorphic state machine) |
| (b) | PARTIAL (latent, dispatched after (a)) | `crates/shape-vm/src/compiler/monomorphization/substitution.rs:2247` + recursive helpers + post-Phase-C MIR-build pipeline | 0 (bounded to inliner + its callers) | §2.7.10 / Q11 (method dispatch ABI) + §2.7.11 / Q12 (value-call ABI) — Phase-C inlining is the §2.7.5 conduit at the AST→MIR boundary |
| (c) | REFUTED | n/a | n/a | n/a |

### Agent partition shape recommendation

**Single agent, single sub-cluster.** Hypothesis (a) is a structurally
bounded migration (one or two MIR-lowering functions + per-iterable-
ConcreteType dispatch). The reference implementation already exists
at `crates/shape-vm/src/compiler/loops.rs:298-516` (bytecode-VM-side).
Approach (ii) keeps the cross-crate cascade at zero. Estimated 400-
600 LoC total.

Hypothesis (b) follow-up sub-cluster: single agent, dispatched AFTER
(a)'s closure-wave merges. Empirical-verification first — confirm
which of the four possible explanations in §3.4 holds — before any
fix. Estimated 100-300 LoC for the empirical-verification phase; fix
scope unknown until empirical phase completes.

**No multi-agent parallel dispatch needed.** The two hypothesis
sub-clusters are sequentially dependent (b cannot be empirically
dispositioned until a is fixed — the bb3 Call terminator only
executes once-then-hangs at HEAD, so Phase-C inlining failure modes
are not observable at runtime).

## §7 — Bounded source-change inventory

Per pathway (iii) discipline: SHAPE_JIT_DEBUG-gated `eprintln!`-only,
matching existing infrastructure pattern exactly. No tracing-crate
migration; no hypothesis fixes; no new env-vars; no defection-
attractor descriptor framings.

### File:line + content + commit-shape

#### Trace point 1 — generic-iterator stub locus
- **File:** `crates/shape-vm/src/mir/lowering/expr.rs`
- **Lines:** approximately 1036-1062 (after the `} else {` of the
  generic-iterator branch and before `builder.push_scope();`)
- **Content:** SHAPE_JIT_DEBUG-gated `[mir-for-expr-generic-stub]`
  `eprintln!` describing the stub iterator shape (iter_slot SwitchBool,
  pattern slot = MirConstant::None, body block, unconditional
  Goto(header), no advance, no termination)
- **Diff size:** +27 lines (eprintln! + leading comment block + closing
  brace)
- **Reason:** existing instrumentation at HEAD bb5b2109 surfaces the
  broken MIR via `SHAPE_JIT_MIR_TRACE` (downstream JIT-side consumer
  trace), but no producer-side trace identifies the architectural-
  source root cause. This trace closes that gap.

#### Trace point 2 — Phase-C inlining call site
- **File:** `crates/shape-vm/src/compiler/monomorphization/cache.rs`
- **Lines:** approximately 720-741 (within the for-loop body at line
  718)
- **Content:** SHAPE_JIT_DEBUG-gated `[mono-phaseC]
  inline_closure_body_into_specialization fn=... closure_param=...
  closure_param_count=... closure_body_stmts=...` trace at the
  invocation, plus a paired `[mono-phaseC] inline FAILED for fn=...`
  trace on the `is_err()` branch.
- **Diff size:** +24 lines
- **Reason:** confirms Phase-C inlining IS invoked for the specialized
  fn (refutes "Phase-C didn't run" as the hypothesis-(b) explanation),
  surfaces the four-explanation residual described in §3.4.

### Total LoC delta

+51 lines added across 2 files (both in `crates/shape-vm/src/`); 0
lines deleted. No source change outside the bounded trace-point
scope.

### Pre-flight cleanup commitment

Trace points landed under this sub-cluster are migrated to the
tracing crate in the future cluster-2 closure-wave per the cluster-2
audit's section H tracing-crate-migration territory (per dispatch's
binding β2 supervisor disposition 2026-05-16 — "tracing-crate
migration is separate cluster-2 closure-wave territory; bounded
source-change discipline below"). This sub-cluster's traces are
intentionally additive — they use the existing `std::env::var_os(
"SHAPE_JIT_DEBUG").is_some() { eprintln!(...) }` pattern so the
cluster-2 tracing-crate migration agent can replace them mechanically
alongside the existing 26 SHAPE_JIT_* sites in one pass.

## §8 — Imprecision-pattern instance log

Per dispatch's §8 binding: enumerate instance 27 (team-lead Q1 → R6
disposition build-on-claim), instance 28 (supervisor R6 dispatch
prompt file-path expansion → team-lead pre-flight catch), and new
instances 29+ if surfaced.

### Instance 27 (carried forward from dispatch text)

- **Source:** team-lead Q1 surfacing → supervisor R6 disposition
  build-on-claim
- **Catch layer:** step 1 pre-flight
- **Provenance:** dispatch text reproduction of phase-3-cluster-0-
  status.md §"Wave 3 Stabilize Round 3 V3-S6 chain close" prior-art

### Instance 28 (carried forward from dispatch text)

- **Source:** supervisor R6 dispatch prompt file-path expansion
- **Catch layer:** team-lead pre-flight validation per Q3 binding
- **Provenance:** dispatch text reproduction of the Q3 recursive
  pre-flight binding extended 2026-05-16

### Instance 29 (NEW — surfaced during this sub-cluster's empirical work)

- **Source layer:** dispatch's required-reading entry §6 locus
  description ("crates/shape-jit/src/mir_compiler/terminators.rs line
  176 (V3-S6c routing block) + lines 621/712 (existing SHAPE_JIT_DEBUG
  sites in the same file)" as hypothesis (a) territory)
- **Imprecision shape:** locus was JIT-side consumer (terminators.rs);
  actual architectural-source root cause is shape-vm-side producer
  (`mir/lowering/expr.rs:1035-1094`). The cite-as-given was valid for
  the consumer side; the producer-side root cause needed its own
  ground-truth pass.
- **Catch layer:** step 2 empirical verification (running the JIT,
  inspecting MIR trace via existing `SHAPE_JIT_MIR_TRACE` at
  `crates/shape-jit/src/mir_compiler/mod.rs:968`, then producer-side
  grep)
- **Provenance:** dispatch text §6 required-reading entry +
  hypothesis (a) locus description. Per Q3 recursive pre-flight
  binding, surfaced back at §2.2 without expanding the cite (the
  cite-as-given was valid for the JIT-side consumer; the producer-
  side root cause needed its own ground-truth pass).
- **Architectural-prediction-subclass parallel:** this is the same
  pattern as V3-S6e (per phase-3-cluster-0-status.md §7829, instance
  41) — "compile-time fix sufficient → wrong (runtime execution
  layer revealed V3-S6f inner gap)". Here: "JIT-side consumer locus
  sufficient → wrong (shape-vm-side producer is the architectural-
  source)". The chain's defense-in-depth via empirical execution
  step held.

### Instance 30 (NEW — surfaced during empirical work)

- **Source layer:** team-lead's own pre-flight ground-truth at step
  1 (this agent's reading of `lower_for_loop` at
  `crates/shape-vm/src/mir/lowering/stmt.rs:464` as the locus
  exercising the Smoke 2 fixture)
- **Imprecision shape:** the `Statement::For` arm in `lower_statement`
  is empirically NOT exercised by the Smoke 2 fixture (or by the
  simpler `fn main() { for x in xs { ... } }` repro). For-loops
  parse as `Statement::Expression(Expr::For(ForExpr, _))` and
  dispatch through `lower_for_expr` (expr.rs:919), NOT through
  `lower_for_loop`. Both functions contain structurally-similar
  ForIn stubs, but only the expr.rs site is reachable for the
  fixtures empirically exercised.
- **Catch layer:** step 2 empirical verification — temporary
  unconditional `eprintln!` placed at the `Statement::For` arm in
  `lower_statement` showed zero hits across two fixtures (Smoke 2 +
  `fn main`-wrapped for-in), surfaced the dispatch-mismatch
  between the dispatch's required-reading entry §5 (stmt.rs:51-57
  Vec.map body) → lower_for_loop in stmt.rs assumption and the
  actual lower_for_expr dispatch path in expr.rs.
- **Provenance:** team-lead empirical-execution step 2. Whether
  the `Statement::For` arm is reachable by *any* AST shape at HEAD
  bb5b2109 is unconfirmed — surfaced at §2.4 as "design observation,
  not ground truth" per Q3 binding.
- **Architectural-prediction-subclass parallel:** related to the
  V3-S6c → V3-S6d "single intended consumer" → wrong pattern
  (instance 38) — here the imprecision is "expected single dispatch
  path for for-loops" → wrong (two AST-level shapes dispatch through
  two different MIR-lowering paths; both contain structurally-
  identical stubs).

### Cumulative count update

41 cumulative pre-cluster-2 + 2 new instances (29 + 30) surfaced
during this empirical-verification sub-cluster = **43 cumulative
imprecision-pattern instances all caught pre-merge across phase-3
cluster-0+1+2 trajectory.** Zero bad-code merges into canonical.

## §9 — Open questions for supervisor disposition

### Q1 — `lower_for_loop` ForIn arm reachability

**Surface:** the `Statement::For` arm in `lower_statement`
(`crates/shape-vm/src/mir/lowering/stmt.rs:72-74`) was empirically NOT
exercised by the Smoke 2 fixture (or by a simpler `fn main` for-in
repro). The `lower_for_loop` function at `stmt.rs:464-559` contains
a structurally-identical ForIn stub at lines 471-516 to the
`lower_for_expr` stub at `expr.rs:1035-1094`. The dispatch's
required-reading entry §5 named the stmt.rs:51-57 vec.shape lines
as the hypothesis-(a) territory; the corresponding MIR-lowering
function in the dispatch's required-reading entry implicitly was
`lower_for_loop` at stmt.rs:464.

**Open question:** is the `Statement::For` arm reachable by any AST
shape at HEAD bb5b2109? If yes, the hypothesis-(a) closure-wave must
land the iterator-protocol fix at BOTH sites (per §2.5 — landing the
fix at one site without the mirror invites future drift). If no, the
`Statement::For` arm is dead code and the closure-wave should also
delete the dead path per the parallel-implementation defection-
attractor refusal rule (CLAUDE.md §Parallel-implementation across
producer/consumer carrier-shape boundaries).

**Recommendation:** the hypothesis-(a) closure-wave agent runs a
fixture-coverage pass first (e.g. test suite + handful of corner
fixtures including async-for, comptime-for, list-comprehension, etc.)
with a temporary unconditional `eprintln!` at the `Statement::For`
arm. If zero hits across the full test surface, delete the dead arm;
else land the fix at both sites.

### Q2 — Phase-C inlining post-MIR shape discrepancy

**Surface:** §3.4 enumerates four possible explanations for the
empirical observation that Phase-C inlining returned `Ok(())` for the
specialized fn but the post-MIR-lowering shape in bb3 still shows
`Call { func: Copy(SlotId(2)), ... }` (where SlotId(2) is the
closure parameter `f`, per the design observation in §2.3).

**Open question:** which of the four explanations holds? The
empirical disposition above is PARTIAL because the dominant root
cause (hypothesis (a)) is the JIT TIMEOUT — Phase-C inlining failure
is latent and severity is moot until hypothesis (a) is fixed.

**Recommendation:** dispatch the hypothesis-(b) follow-up sub-cluster
AFTER hypothesis-(a) closure-wave merges. The follow-up's first
phase is empirical verification of which explanation holds (additional
SHAPE_JIT_DEBUG traces at the AST-dump layer, the recursive walk's
match-arm decisions, and the post-Phase-C MIR-build pipeline). The
fix scope is unknown until empirical phase completes.

---

*End of cluster-2 V3-S6f empirical verification deliverable.*

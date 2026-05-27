# Cluster #6 — Borrow-check bypass (validator misses violations)

**Audit-only.** No source/fixture changes. No commits.
**HEAD:** workspace HEAD at audit start (post-`8bbd2f99`).
**Binary:** `./target/release/shape run <file>.shape`
**Cluster sources (2 tests):**
- `borrow_refs::violations::violation_ref_in_let_binding`
- `borrow_refs::violations::violation_ref_in_nested_expression`

## 1. Minimal repros

### 1a. `violation_ref_in_let_binding` — silently runs

Fixture (verbatim, `tools/shape-test/tests/borrow_refs/violations.rs:32-40`):

```shape
let x = 5
let r = &x
```

Fixture expects `.expect_run_err_contains("B0003")`.

**Actual at HEAD (`/tmp/violation_let.shape`):**
```
[jit-fallback] function main failed JIT compile: ... slot 3 has LocalTypeInfo::NonCopy
but MIR inference did not prove its NativeKind. ... ; running under interpreter
{
  "Bool": false
}
EXIT=0
```

Program runs to completion, returns `Bool(false)`. No B0003. No reject of any
kind. The JIT-fallback noise is unrelated W11 surface; the interpreter swallows
the program and produces a value.

### 1b. `violation_ref_in_nested_expression` — SEGFAULT (worse than audit-log)

Fixture (`violations.rs:138-147`):
```shape
fn f(&x) { x }
let a = 5
let b = f(&a) + &a
```

Fixture expects `.expect_run_err_contains("Cannot apply")`.

**Actual at HEAD (`/tmp/violation_nested.shape`):**
```
Segmentation fault (core dumped)
EXIT=139
```

The 2026-05-26 classification log saw "no method 'add' on receiver kind Int64"
— today it segfaults. **Regression escalated** between audit-log timestamp and
HEAD. Wrapping the same shape in a function body (`fn run() { ...; b }`) also
SIGSEGVs at EXIT=139, so this is not module-scope-only — it is a true
runtime-reaches-`&` shape that the compiler should have refused.

### Baselines that still reject cleanly (control)

```shape
# ref-in-return — rejects B0003 cleanly
fn f() { let x = 5; return &x }
f()
→ Error: [B0003] cannot return or store a reference that outlives its owner

# ref-stored-in-array — rejects B0004 cleanly
let x = 5
[&x]
→ Error: [B0004] cannot store a reference in an array — ...
```

The borrow-check infrastructure exists and fires on related shapes — but does
NOT fire on the two cluster-#6 shapes.

## 2. Root cause

**Commit `8bbd2f99` ("fix(v0.3): R8 W9 B5+B9 bundle — module-scope ref_borrow
+ stdlib annotations", 2026-05-25)** deleted both categorical
`if ref_borrow.is_some()` B0003 rejection sites at module-scope let-bindings
in `crates/shape-vm/src/compiler/statements.rs` (the surviving comment at
lines 4830-4835 documents the deletion verbatim: *"R8 W9 B9: removed
categorical ban on module-scope `ref_borrow`. The MIR borrow solver is the
documented sole authority"*).

The deletion's justification (per `docs/cluster-audits/v0.3-r8w9-borrow-b0003-audit.md`
§3 as cited in the commit body) was that local-scope `let r = &x` accepts the
same shape so module-scope must match, and that the MIR borrow solver would
catch the escape. **That second assumption is empirically false** for the
shapes under test:

- `crates/shape-vm/src/mir/solver.rs:201-298` only pushes `escaped_loans` when
  `dest_slot == SlotId(0)` (the function return slot) or when a loan flows
  into a `LoanSink::ClosureEnv` / `LoanSink::ReturnSlot`. A top-level
  `let r = &x` lowers to a `StoreModuleBinding` store, not a MIR
  `Assign(Place::Local(SlotId(0)), ...)`, and module-binding stores are not
  modeled as escape sinks in `solver.rs`.
- The B9 commit body itself flags a related gap: *"arithmetic-through-reference
  (`r + 1`) inference is unresolved at BOTH module-scope and local-scope
  post-fix; separate v0.4 territory not in B9 scope"* — meaning the deletion
  shipped while a known follow-on gap was already on the table.
- The 1b nested-expression segfault is a separate shape: the call-site
  `f(&a) + &a` synthesizes a `&` operand whose runtime carrier flows into the
  `Add` dispatcher and crashes the VM (Int64 + Ref operand-kind mismatch
  reaching native dispatch). Pre-B9 the borrow-check pipeline aborted
  compilation; post-B9 the program builds and the runtime hits an invalid
  receiver-kind dispatch.

The cluster classification document's hypothesis ("LSP-B Wave 1 reference-mode
classifier extension shadows runtime borrow-solver") is **not supported** by
the LSP-B commit content: commit `4b43c12a` adds an `infer_lsp_display_ref_modes`
classifier that is explicitly **display-only** (commit body: *"Pure display
signal; nothing flows into codegen"*). The real bisect anchor is the B9
deletion in 8bbd2f99, not LSP-B Wave 1.

## 3. Bisect anchor

```
$ git log --oneline --grep="B9\|ref_borrow\|module-scope" -- crates/shape-vm/src/compiler/statements.rs
8bbd2f99 fix(v0.3): R8 W9 B5+B9 bundle — module-scope ref_borrow + stdlib annotations
15395d7b Phase 5.B: propagate return ownership hint to let-bindings
```

`8bbd2f99` body (verbatim):
> B9 — borrow-solver B0003 false positive: delete categorical ban on
> `ref_borrow` at module-scope let bindings (statements.rs:786 + :4829).
> Local-scope already accepts the same shape; MIR borrow solver is the
> documented sole authority. Per audit
> docs/cluster-audits/v0.3-r8w9-borrow-b0003-audit.md §3. Fix size S
> (~16 LoC deletion).

Diff confirms: both `if ref_borrow.is_some() { return Err(ShapeError::SemanticError { message: "[B0003]..." }) }` blocks deleted; replaced with the
comment that survives at `statements.rs:4830-4835`.

`git log --oneline -20 -- crates/shape-vm/src/mir/solver.rs` shows the most
recent solver edits are 2026-mid (W12 / Phase 5 ownership work). No solver
change since `8bbd2f99` added a module-binding escape sink to compensate for
the deleted compiler guard.

## 4. Affected subsystem (file:line)

| Site | What should happen | Status |
|---|---|---|
| `crates/shape-vm/src/compiler/statements.rs:783-794` (pre-B9) | Compile-time reject when `ref_borrow.is_some()` on `let x = &y` at module-scope (B0003) | **Deleted by `8bbd2f99`**; surviving comment lines 783-794 + 4827-4835 mark the deletion sites. |
| `crates/shape-vm/src/mir/solver.rs:201-298` (`scan_function_for_borrows`) | Push `escaped_loans` for any loan reaching `StoreModuleBinding` (module-scope let-binding stores) | **Never modeled.** Sinks limited to `SlotId(0)` return slot + `LoanSinkKind::ClosureEnv` + `LoanSinkKind::ReturnSlot`. Module-binding stores absent. |
| `crates/shape-vm/src/compiler/expressions/{assignment.rs:645, collections.rs:227}` | Reject `&` inside array-literal / inside binary-op-RHS expression context (B0004 + ref-as-operand) | Array-literal still rejects (1b/control 2 passes); binary-op-operand path **not refused** at compile time. |

## 5. Sub-cluster name + size estimate

**Sub-cluster name:** `borrow-check-module-binding-and-bin-op-escape` (BC-MBO).

**Size estimate:** 2 test fixtures in cluster #6 + likely 1-3 latent shapes
not yet exercised (e.g., `let r = &arr[0]` at module scope, `&x` as
match-scrutinee, `&x ?? y`). Fix is bounded:

- (a) **Compiler-side**: re-add a narrow module-binding ref_borrow guard at
  `statements.rs:4830` that ONLY triggers when the MIR solver did NOT mark the
  loan as safely-returned (mirroring the local-scope path). ~10-20 LoC.
- (b) **Solver-side**: add `LoanSinkKind::ModuleBindingStore` and push it for
  any loan that flows into `StoreModuleBinding`/`StoreModuleBindingTyped`.
  ~20-40 LoC in `solver.rs` + matching emission in MIR lowering.
- (c) **Binary-op operand guard**: refuse `Expr::Binary { lhs/rhs: Expr::Ref(_) }`
  at semantic-check time (or stamp the operand kind such that `Add` dispatch
  rejects before native call). ~10 LoC in `compiler/expressions/binary.rs` or
  a new MIR check.

Total: S/M fix; one commit per sub-cluster (a/b/c). No new opcodes; no new
runtime carriers. Aligns with CLAUDE.md "no runtime coercion / no dynamic
fallback" — the fix lives strictly in semantic-check + MIR escape-sink
extension.

## 6. Dependencies

**Standalone with one soft overlap.**

- **Standalone**: borrow-check + MIR escape-sink extension is a discrete
  subsystem (`mir/solver.rs` + `compiler/statements.rs` + semantic-check at
  binary-op site). No carrier-shape work; no JIT codegen; no Forbidden-Pattern
  surface.
- **Soft overlap with closures_hof S2 var-capture (BindingStorageClass)**:
  `BindingStorageClass` in `type_tracking.rs:290` is the lifetime-lattice
  consumer of escape facts. If S2 also touches escape-class assignment, the
  module-binding escape-sink addition (sub-cluster fix (b)) should land BEFORE
  S2 so S2 reads the corrected facts. If S2 only touches closure-capture
  classification, no ordering dependency.
- **LSP-B Wave 1 reference-mode (`4b43c12a`)** is **not** a dependency.
  Commit body confirms LSP-B is display-only; nothing flows into codegen or
  borrow-check. The classification doc's hypothesis is incorrect.
- **No overlap with V3-S5 ckpt-5/ckpt-6 op_new_array work** (the 30
  SCOPE-RECLAIM siblings in `borrow_refs.md`). Those failures are typed-array
  carrier rebuilds; cluster #6 is pure borrow-check semantic-check rejection.

## 7. User-binding restated

"Violations should reject" — no silent unsafety. Both cluster sources currently
execute without semantic-check rejection: 1a returns `Bool(false)`; 1b
segfaults the VM. Fix must restore compile-time rejection (B0003 for 1a;
B0004 or a new B-code for 1b) before either test can pass.

# v0.3.3 fix-dispatch cluster #9 — closures_hof S2 — var-capture upvalue allocation broken

**HEAD:** `70507224` (post-v0.3.2; closure-touch files unchanged since `82f049dd` audit baseline).
**Cluster size:** 23 tests (closures_hof S2, FN-REG-CORRECTNESS) — see classification doc.
**Discipline:** AUDIT-ONLY. No source/fixture changes. No commits. No `git stash`. Repros via `cargo run` only.
**Owning files (read-only):**
- `crates/shape-vm/src/executor/variables/mod.rs` (`read_capture_raw_pointer_bits`, owned-mutable + shared-module-binding handlers)
- `crates/shape-vm/src/executor/call_convention.rs` (all 3 `CallFrame` constructors — `upvalues: None` invariant)
- `crates/shape-vm/src/executor/control_flow/mod.rs` (`op_make_closure`, captures-region producer)
- `crates/shape-vm/src/compiler/expressions/closures.rs` (capture-emit dispatcher; Tracks A.1C.2 / A.1C.2b / A.1C.3)
- `crates/shape-vm/src/compiler/expressions/identifiers.rs` (body-side `LoadOwnedMutableCapture` emit, A.1C.3 `LoadSharedModuleBinding` emit)
- `crates/shape-vm/src/compiler/expressions/assignment.rs` (body-side `StoreOwnedMutableCapture` + `StoreSharedModuleBinding` emit)

---

## 1. Minimal repro (FN-REG-CORRECTNESS confirmed — TWO distinct surfaces in S2)

S2 is heterogeneous: the cluster's 23 tests collapse to one of two failure surfaces depending on whether the var-bound binding is a **module binding** (top-level `var`) or a **local** (`let` of a closure that captures a `var`).

### Repro A — module-binding `var` + forEach (misaligned-pointer-dereference panic)

```shape
var total = 0
[1, 2, 3].forEach(|x| { total = total + x })
print(total)
```

Run (`cargo run --bin shape -- run /tmp/repro_s2_a.shape`):

```
[jit-fallback] function main failed JIT compile: Runtime error: JIT compilation failed:
  Main code contains unsupported constructs: JitPreflightReport {
    vm_only_opcodes: [AllocSharedModuleBinding, LoadSharedModuleBinding], ... };
  running under interpreter
thread 'main' panicked at crates/shape-vm/src/executor/variables/mod.rs:1747:33:
misaligned pointer dereference: address must be a multiple of 0x8 but is 0x1
thread caused non-unwinding panic. aborting.
```

Reading `bits as *const SharedCell` where the bits' low nibble = `0x1`. The slot was **not** previously promoted via `AllocSharedModuleBinding`, so it still holds a raw `Int64` value (`0` initialiser bits, or a partially-incremented value), not an `Arc<SharedCell>` pointer.

### Repro B — local `let` closure + module-binding `var` mutation (clean runtime error)

```shape
var counter = 0
let bump = || { counter = counter + 1 }
bump()
print(counter)
```

Run:

```
Error: Runtime error: mutable/shared capture access in a frame without upvalues (line 3)
```

This is the canonical SURFACE message named in the classification doc.

Both repros reproduce on first run at HEAD `70507224`. Repro A is the more dangerous shape (hard panic at unsafe deref, not a `Result::Err` surface).

---

## 2. Root cause

S2 collapses to **one structural bug with two failure modes**: the §2.7.8 / Q10 cell-storage parallel-kind ABI for mutable captures is **partially wired** — the compiler-emit and runtime-read sides disagree on where the cell pointer lives.

### Surface 1 — `frame.upvalues` is never populated (Repro B path)

All three `CallFrame` constructors in `call_convention.rs` hard-code `upvalues: None`:

| Constructor | File:Line | `upvalues` field |
|---|---|---|
| `call_function_with_nb_args` (non-closure call) | `call_convention.rs:609` | `None` |
| `call_closure_with_nb_args_keepalive` (closure call) | `call_convention.rs:739` | `None` |
| sentinel-fill path | `call_convention.rs:1295` | `None` |

`grep -rn "frame.upvalues = \|.upvalues = Some" crates/shape-vm/src/executor/`: **zero writers**. Only readers (`gc_integration.rs:187/330`, `vm_state_snapshot.rs`, `snapshot.rs:359`, and `variables/mod.rs:90`) — all reachable via `read_capture_raw_pointer_bits` (`variables/mod.rs:84-102`).

But `op_load_owned_mutable_capture_i64` and siblings (`variables/mod.rs:399-664` — 11 typed Load + Store opcodes) call `read_capture_raw_pointer_bits(idx)` to fetch the cell pointer. With `frame.upvalues = None`, every one of these handlers fails at line 92 with `"mutable/shared capture access in a frame without upvalues"`.

The captures DO exist — `call_closure_with_nb_args_keepalive` writes them into the frame's **local-slot window** at `base_pointer + capture_idx` (`call_convention.rs:770-778`, using `read_capture_kinded` + `clone_with_kind` + `stack_write_kinded`). The compiler-side `Load*OwnedMutableCapture` opcodes are emitted with `Operand::Local(idx)` but the runtime then routes through `frame.upvalues` instead of the local-slot window — **ABI mismatch**.

Either the body should be emitting `LoadLocalPtr` / `LoadLocal*` for owned-mutable captures (reading from the local-slot window where the call-convention put them), OR the call-convention must populate `frame.upvalues` from the captures before installing them as locals. The current state has both producer and consumer wired to different conventions.

### Surface 2 — module-binding `var` slot never promoted before closure body runs (Repro A path)

For module-binding `var` (top-level `var total = 0`), the compile-time A.1C.3 path (`closures.rs:1343-1375`) emits `LoadModuleBinding + AllocSharedModuleBinding + LoadModuleBinding` **inside the closure-construction block** (before `op_make_closure`). After this sequence, `self.shared_module_bindings` contains the scoped name, so subsequent outer reads (`identifiers.rs:414-423`) emit `LoadSharedModuleBinding`.

For `[1,2,3].forEach(|x| { total = total + x })`, the closure body's `total = total + x` is `StoreSharedModuleBinding` + `LoadSharedModuleBinding` (closure-body identifier emit). But the closure body is compiled as a **separate function** — its compile-time view of `self.shared_module_bindings` should be inherited from the enclosing scope.

The empirical signature (`bits` = `0x...1`, not a valid heap-aligned pointer) shows the slot was never replaced by an `Arc::into_raw(SharedCell)` value. Likely cause: the promotion sequence runs only on the **first** capture-emit site, but `forEach`'s closure-body compilation walks the body BEFORE the outer capture-emit runs (or the `shared_module_bindings` set is not propagated through the nested closure-compilation scope), so the body emits `LoadSharedModuleBinding` against an un-promoted slot.

Per ADR-006 §2.7.8 / Q10, every `Vec<u64>` cell-bearing store must carry a parallel `Vec<NativeKind>`. The module-binding store does (`module_binding_write_kinded` at `op_alloc_shared_module_binding:1707-1711` writes `(cell_bits, NativeKind::Ptr(HeapKind::SharedCell))`). But the **invariant** that "outer reads happen-after the alloc" is enforced only by compile-time order, and the per-closure body-compilation pass appears to either (a) compile the body before the alloc opcodes for the enclosing scope have been emitted, or (b) compile the body in a sub-compiler whose `shared_module_bindings` set is not yet populated.

### Why this regressed

The §2.7.8 / Q10 cell-storage parallel-kind extension (`Wave-α` G-module-bindings-kind commit `27e2918`) added `SharedCell::kind()` and migrated `op_load_shared_module_binding` / `op_store_shared_module_binding` to the kinded path. The compile-emit side (`AllocSharedModuleBinding` insertion + `shared_module_bindings` membership-tracking) was extended for Track A.1C.3 (commit `05eb1d6d`, "Phase 4b Round 3 Surface-1 BUNDLE" 2026-04-something), but the **per-closure-body sub-compiler propagation of `shared_module_bindings`** was not. Independently, the OwnedMutable Track A.1C.2b path (`LoadOwnedMutableCapture*` typed family, "Wave D" per the SURFACE message at `variables/mod.rs:372-379`) was wired on the compiler side but the runtime's `read_capture_raw_pointer_bits` reads from `frame.upvalues`, which was never plumbed through the §2.7.8 frame-setup migration.

The unification — captures via `OwnedClosureBlock::read_capture_kinded` into the frame's local-slot window — landed for the §2.7.10/Q11 method-dispatch ABI and §2.7.11/Q12 value-call ABI (the W7 / Round 13 T5 work referenced verbatim at `call_convention.rs:763-769`), but the `frame.upvalues` Option remained as a vestigial field that the OwnedMutable opcodes still target. Classic forbidden-pattern shape: the §2.7.8 cell-storage parallel-kind work was treated as "extend the existing path", and the existing path (`frame.upvalues`) was never deleted or migrated. CLAUDE.md "Renames to refuse on sight" applies — this matches the Q10 "Cell-storage parallel-kind extension" forbidden-shape #1 ("Cell store as `Vec<KindedSlot>` … `KindedSlot` is a runtime-tier carrier, not the storage-tier shape") but inverted: the runtime is reading from a non-existent cell store while the producer wrote into the typed local-slot window.

---

## 3. Bisect anchor (history map)

`git log --oneline -- crates/shape-vm/src/compiler/expressions/closures.rs`:

| Commit | Title | Relevance |
|---|---|---|
| `19de5ef2` | W18.3 hard retire c-string syntax | unrelated to captures. |
| `05eb1d6d` | Phase 4b Round 3 Surface-1 BUNDLE: 1A LANG-W13-3-iife-closure-capture VM-side fix + three producer-side stamps per ADR-006 §2.7.5 | **PRIMARY SUSPECT** — Surface-1 BUNDLE extended IIFE closure-capture but only on the producer side. The compile-time `shared_module_bindings` propagation through nested closure-body compilation is the gap not closed by this commit. |
| `028b8f47` | phase-1b-vm Wave-β C-expressions: migrate compiler/expressions/* off ValueWord | Mass migration off ValueWord — likely the point at which `LoadOwnedMutableCapture*` typed family was added (the "Wave D" path the runtime SURFACE message names at `variables/mod.rs:374-378`). |
| `d1a2955f` | closures: capture_as_value native decode + B-1 test predicate update | capture-side native-decode flip. |
| `9de12a68` | A2-refined task #17 prep — track Shared capture inner kinds | introduced the Shared-capture inner-kind tracking that A.1C.3 depends on. |

`git log --oneline -- crates/shape-value/src/v2/closure_layout.rs crates/shape-value/src/v2/closure_raw.rs` (5-most-recent):

| Commit | Title | Relevance |
|---|---|---|
| `825126aa` | R5c-2-β-δ Family α: re-instate HeapKind::TypedArray clone/drop arm | unrelated. |
| `aefe77e5` | Phase 4b Round 5b-2 R5b2-bool-null-sentinel-cluster fix | mostly NativeKind::Null addition; touches SharedCell drop arms via parallel-kind. |
| `10a2a011` | W8-T25: HeapKind::SharedCell variant amendment + dispatch wiring (close) | **MAJOR SUSPECT** — extended `SharedCell` with kind-tracked drop/clone, made `op_alloc_shared_module_binding` the canonical promotion path. The `frame.upvalues = None` vestigial state predates this; W8-T25 did not delete it. |

No bisect-by-binary was run (audit-only). The two suspects are `05eb1d6d` (Surface-1 BUNDLE, A.1C.3 producer side) and `10a2a011` / `27e2918` (W8-T25 SharedCell kind amendment) — both landed in the 2026-04 to 2026-05 window of ADR-006 §2.7.8 / Q10 migration.

---

## 4. Affected subsystem (file:line for the broken sites)

| Site | File:Line | Issue |
|---|---|---|
| **`frame.upvalues = None` invariant** | `crates/shape-vm/src/executor/call_convention.rs:609, 739, 1295` | All three CallFrame constructors hardwire `None`; no writer exists. |
| **`read_capture_raw_pointer_bits` reads `frame.upvalues`** | `crates/shape-vm/src/executor/variables/mod.rs:84-102` | Reader path for 11 typed `Load*OwnedMutableCapture` + 11 typed `Store*OwnedMutableCapture` handlers (`variables/mod.rs:368-1000+`). |
| **A.1C.3 `LoadSharedModuleBinding` emit** | `crates/shape-vm/src/compiler/expressions/identifiers.rs:414-423` | Predicated on `shared_module_bindings.contains(&scoped_name)`. Set membership is populated at `closures.rs:1370` but not propagated through nested closure-body sub-compilation. |
| **A.1C.3 `AllocSharedModuleBinding` emit** | `crates/shape-vm/src/compiler/expressions/closures.rs:1360-1371` | Emits the alloc only at the outer-scope capture site; the inner closure body is compiled by a sub-compiler that sees a fresh `shared_module_bindings` set. |
| **Frame-setup captures path** | `crates/shape-vm/src/executor/call_convention.rs:770-778` | Writes captures into local-slot window — RIGHT shape per ADR-006 §2.7.8, but disconnected from the body's `Load*OwnedMutableCapture` reads. |
| **OwnedMutable polymorphic SURFACE** | `crates/shape-vm/src/executor/variables/mod.rs:368-397` | Bare `LoadOwnedMutableCapture` / `StoreOwnedMutableCapture` (no kind suffix) return `NotImplemented(SURFACE)` — Phase-2c follow-up cleanup. |

---

## 5. Sub-cluster name + size estimate

- **Sub-cluster name:** `S2-var-capture-upvalue-frame-setup-mismatch`
- **Effort estimate:** **M** (medium). Not S, because the fix is two-headed:
  - **Head 1 (frame.upvalues):** either delete the `upvalues: Option<Vec<u64>>` field entirely and migrate `Load*OwnedMutableCapture*` to read from the local-slot window (preferred — matches §2.7.8 / Q10 + the W7 frame-setup work that already populates locals), OR plumb captures into `frame.upvalues` at the 3 constructor sites. The first is the ADR-006-aligned shape but requires touching 22+ opcodes in `variables/mod.rs`.
  - **Head 2 (shared_module_bindings propagation):** propagate the `shared_module_bindings` set into nested closure-body sub-compilations OR move the `AllocSharedModuleBinding` emit to the binding's declaration site (`var total = 0` directly emits the promotion) so all subsequent reads — outer or inner — see a SharedCell-promoted slot. The second shape is cleaner and matches how local Shared captures work (`AllocSharedLocal` is emitted at the first capture site, and the local-slot read path checks `shared_locals` membership — `closures.rs:1294-1313`).
- **Test-count breakdown:** 23 tests — empirically split ~50/50 between the two surfaces but not all 23 were repro'd; needs per-test classification at fix time.

---

## 6. Dependencies

- **Cluster #8 (S1 — closure-param type-inference loss):** orthogonal mechanism (bidirectional inference gap at let-binding time, no caller context) but **shares the closure compile path**. Any rebuild of `compile_expr_closure` for S1 must preserve / extend the §2.7.8 capture-kind plumbing this cluster depends on. Fix-order: S1 first (smaller surface, no runtime ABI touch), then S2.
- **Cluster #6 (borrow-check-bypass — `BindingStorageClass` lattice):** S2 touches the same `BindingStorageClass::SharedCow` / `SharedAtomicMut` lattice (`closures.rs:1270-1273` calls `set_binding_storage_class_for_name(captured, BindingStorageClass::SharedCow)`). If #6 fix changes the storage-class assignment for `var`-bound module bindings, S2's `shared_module_bindings` propagation logic must be re-checked.
- **ADR-006 §2.7.8 / Q10 cell-storage parallel-kind invariant:** the fix MUST preserve the kind-track lockstep — `op_alloc_shared_module_binding` already writes `(cell_bits, NativeKind::Ptr(HeapKind::SharedCell))` in lockstep via `module_binding_write_kinded`. Any new alloc-emit site (e.g. at binding declaration) must use the same kinded write API.
- **Cluster #10 (closures S10 — `Undefined variable: total` in forEach side-effect):** the classification doc names this as "same family as S2 (mutable-capture upvalue missing) but surfaces as scope-resolution failure". Likely fixed by the same Head-2 propagation work — verify post-fix.
- **`§5.16 JIT-lowering followup` (supervisor 2026-05-25):** the JIT preflight currently reports `AllocSharedModuleBinding` and `LoadSharedModuleBinding` as `vm_only_opcodes` and falls back to the interpreter (`[jit-fallback]` line in both repros). The fix should NOT introduce new JIT-unsupported opcodes; if a new alloc-at-declaration emit site is chosen, the JIT preflight needs a matching update.

---

## 7. Defection-attractor refusals on sight

Per CLAUDE.md "Renames to refuse on sight" + ADR-006 §2.7.8 forbidden-shape list:

- **"Add `frame.upvalues` plumbing as a transitional bridge"** — refused. The frame already has local-slot storage for captures (per §2.7.8 the cell-storage parallel-kind extension that landed); adding a second storage track for the same data is a parallel-implementation-across-producer/consumer-boundary defection per CLAUDE.md "Defection-attractor framings refused on sight" #1.
- **"Promote slot to Shared with `NativeKind::Bool`-default until a typed path exists"** — refused per ADR-006 §2.7.8 #6 (transitional Bool-default fallback).
- **"Surface-and-stop the misaligned-deref panic as a SURFACE message"** — refused as a defection. The misaligned-deref is FN-REG-CORRECTNESS (canonical user pattern, `var counter; forEach { counter = ... }`); surface-and-stop here would be the "rationalize by deferring" pattern from CLAUDE.md "Forbidden rationalizations".
- **"`KindedUpvalueSlot` carrier as a runtime-tier bridge"** — refused per ADR-006 §2.7.8 forbidden-shape #1 (Cell store as `Vec<KindedSlot>`).
- **"Add per-FieldKind capture-decode hop in `read_capture_raw_pointer_bits`"** — refused per CLAUDE.md broader-family regex (`(decode|tag|kind|dispatch|value.call|...) (bridge|probe|helper|hop|translator|adapter|shim)`).

The correct surface-and-stop shape (`NotImplemented(SURFACE)`) IS used by the bare polymorphic `op_load_owned_mutable_capture` shell at `variables/mod.rs:368-380` — that's correct, because the typed family supersedes it. But the typed family itself reads from a non-existent `frame.upvalues`, which is the bug under audit.

---

## 8. Audit close

- 2 distinct minimal repros, both verified at HEAD `70507224` via `cargo run --bin shape -- run`.
- Single structural root cause (frame-setup / capture-storage ABI mismatch) with 2 surface modes (misaligned-deref panic + clean-error).
- Bisect anchors: `05eb1d6d` (Surface-1 BUNDLE producer-side), `10a2a011` (W8-T25 SharedCell), `028b8f47` (Wave-β C-expressions migration).
- Sub-cluster `S2-var-capture-upvalue-frame-setup-mismatch`, size M.
- Dependencies on clusters #8, #6, #10 + §5.16 JIT preflight.
- No source / fixture changes. No commits. No `git stash`. Audit doc is the only artifact.

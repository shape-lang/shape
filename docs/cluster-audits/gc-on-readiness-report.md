# GC-on Readiness Report (Finding #31 — per-iteration garbage-cycle leak)

**Lane:** `wave7/gc-on-readiness` (worktree `shape-w7-gcon`, off `main` HEAD with all GC phases merged).
**Date:** 2026-07-08.
**Scope:** PHASE 1 — VALIDATE gc-on. Build `--features gc`, measure the Finding #31
RSS-bounded proof, run the full gc-on suite, confirm gc-off parity, measure gc-on
hot-loop overhead. **This lane does NOT flip the default** (that is the user go/no-go).

---

## VERDICT: **GO-WITH-CAVEATS** (independently verified; four caveats in §7, none a NO-GO trigger)

gc-on demonstrably fixes the Finding #31 leak end-to-end: the real closure-in-array
cycle (`var arr = []; arr.push(|| arr.len())`, expressed with the required element-type
annotation) has **BOUNDED RSS with gc-on** and **linearly-GROWING (unbounded) RSS with
gc-off**, corroborated by valgrind (0 definitely/indirectly-lost with gc-on vs
606 KB lost with gc-off at N=4000). The full `--features gc` suite is green with **zero
regressions** vs the gc-off baseline (integration failure sets byte-identical). gc-off is a
compile-time no-op for the hot path (all GC barrier/safepoint/teardown code is
`#[cfg(feature = "gc")]`-gated). The safepoint poll is unmeasurable on a tight loop.

This VALIDATE was checked by an independent from-scratch VERIFY pass (own builds, own
RSS sweep, own valgrind, own suite runs) that **reproduced every load-bearing number** and
**confirmed the flip is not done**. It downgraded the verdict from a bare GO to
GO-WITH-CAVEATS on four honest caveats the first draft under-disclosed:
(1) the flagship `for i in 0..N { leak() }` program **panics at teardown**
(`wire_conversion.rs:507` — a callable-array is not wire-serializable, exit 101), present
**identically in gc-off** and only *after* the measured loop, so the RSS/leak proof is
unaffected — but it is a panic, not a clean exit; (2) collection overhead on **pathological
all-garbage cycle-churn is ~15–17%**, higher than the ~7–8% the allocation-saturated row
below shows (the safepoint poll and RC barrier remain ~0% on compute-bound and
normal-allocation code); (3) the **snapshot v6→v7 identity-map change is NOT gc-gated**, so
it already ships in the default build regardless of the flip; (4) the bare untyped `var arr
= []` form fails via a **runtime `op_new_array(0)` SURFACE**, not the clean compile error the
draft described — only the annotated `Array<() -> int>` form is collectable end-to-end.
None of the four blocks the flip; all are documented in §7.

---

## UPDATE 2026-07-08 — FLIP APPLIED + CAVEAT 1 FIXED (user go decision)

The user gave the GO. Two changes landed on top of this VALIDATE:

1. **The flip is applied — and it is TWO-part, not one.** The report's §8
   "single edit" was INCOMPLETE: `shape-vm/gc` covers only the interpreter tier.
   `shape-jit` has its OWN `gc` feature gating the JIT-tier hooks
   (`jit_write_barrier` decrement-candidate logic, `jit_gc_safepoint` STW park),
   and Cargo features do not forward across the shape-vm→shape-jit boundary, so
   the shipped JIT-on binary would have run gc-ON interpreter + gc-OFF JIT —
   leaking cycles mutated on JIT-native field-store paths. (The report's own
   validation command had to pass `--features gc` to `-p shape-jit` explicitly,
   an implicit admission of the second feature.) The completed flip:
   - `crates/shape-vm/Cargo.toml:54` → `default = ["jit", "gc"]` (interpreter
     tier + every shape-vm consumer + the default `cargo test` suite).
   - `bin/shape-cli/Cargo.toml` → `default = ["jit", "gc"]` with a NEW
     `gc = ["shape-vm/gc", "shape-jit?/gc"]` feature, ORTHOGONAL to `jit`. The
     `shape-jit?/gc` weak-dep enables the JIT-tier barriers in the shipped
     binary. `cargo tree` confirms: default build → `shape-jit ... default,gc`;
     `--no-default-features --features jit` → `shape-jit ... default` (no gc, so
     the jit-WITHOUT-gc barrier-cost build is preserved).
   JIT-tier proof: the `shape-jit --features gc` suite is green (496/0/20-ign)
   including `jit_set_field_overwrite_barrier_buffers_and_collects_object_cycle`,
   `jit_produced_typed_object_cycle_is_collected`, and
   `jit_write_barrier_buffers_surviving_typed_object`.
   `just check-clean` (full workspace + all targets, now with shape-jit/gc
   unified) and `just check-no-dynamic` are both EXIT 0; `cargo check -p shape-vm
   --no-default-features --features jit` still builds (gc-off remains
   expressible — all gc code stays `#[cfg(feature="gc")]`-gated).
   NOTE: the separate `../shape-app` workspace (playground/notebook server) is
   not covered by this repo's build graph; if it embeds the VM to run untrusted
   code, it needs its own gc enablement — verify separately.
2. **Caveat 1 (the teardown panic) is fixed.** The trailing callable-array is
   now wire-serializable: `crates/shape-runtime/src/wire_conversion.rs`
   `v2_typed_array_to_wire` gained an `ELEM_TYPE_CALLABLE` arm that projects
   each `CallableArrayElem` to a display placeholder (`<closure>` /
   `<function:{id}>` / `<module_fn:{id}>`), mirroring the pre-existing scalar
   `HeapValue::ClosureRaw` → `"<closure>"` and `HeapValue::ModuleFn(id)` →
   `"<module_fn:{id}>"` arms in `heap_to_wire`. This is the host-boundary
   DISPLAY projection (`slot_to_envelope` → `ProgramExecutorResult.wire_value`,
   printed by the CLI via `serde_json`), NOT a round-trip transport encoder, so
   the placeholder cannot corrupt a genuine wire round-trip (closures were never
   reconstructible via `wire_to_slot` at either the scalar or array level).
   The leak-only repro (`f31_50000.shape`) now exits **0** (was exit 101); the
   REPL projects a callable array to `[<closure>, <closure>]`. Covered by the
   regression test `v2_callable_typed_array_projects_to_display_placeholders`.

Caveats 2–6 are unchanged (orthogonal: the bare-untyped-form runtime SURFACE,
the JIT-fallback of the closure-push opcode, the not-gc-gated snapshot v7, the
opt-in perf cost, and the Phase-6 cross-thread-cycle deferral all remain as
documented).

---

## 1. Build

| Build | Command | Result |
|-------|---------|--------|
| gc-on | `cargo build --release --bin shape --features gc` | clean, exit 0 (4 pre-existing shape-vm warnings) |
| gc-off (default) | `cargo build --release --bin shape` | clean, exit 0 |

Binary size: gc-on 68,096,264 B vs gc-off 67,974,888 B (+121 KB of collector/barrier code).

## 2. Finding #31 — RSS-bounded proof (THE load-bearing measurement)

**Repro** (`f31_*.shape`), the real per-iteration fresh garbage cycle:

```shape
fn leak() {
    let mut arr: Array<() -> int> = []   // element-type annotation required (see caveat §7)
    arr.push(|| arr.len())               // closure captures arr; pushed into arr → 4-node cycle
}
for i in 0..N { leak() }                 // each iteration leaks one dead A↔B↔C↔D cycle
```

Each iteration builds a 4-node cycle **A** (`TypedArray` CALLABLE) → **B**
(`Arc<HeapValue::ClosureRaw>`) → **C** (closure block) → **D** (`Arc<SharedCell>`) → **A**
that is unreachable at end of `leak()`. Plain reference-counting cannot reclaim it
(the SharedCell back-edge pins A's refcount > 0). The dispatch-loop safepoint
(`dispatch.rs` `maybe_collect(256)`) runs Bacon–Rajan `CollectCycles` and reclaims it.

**Peak RSS (`/usr/bin/time -v`, KB):**

| N (iterations) | gc-off | gc-on |
|---:|---:|---:|
| 50,000    | 41,484  | 37,656 |
| 200,000   | 69,056  | 37,724 |
| 500,000   | 125,240 | 37,636 |
| 1,000,000 | 219,456 | 36,552 |
| 2,000,000 | 406,192 | 37,284 |
| 4,000,000 | 781,376 | 38,156 |

- **gc-off: linear, unbounded** — ~190 bytes/iteration (69 MB → 781 MB from 200 K → 4 M);
  extrapolates to the multi-GB blowup of the original finding at 33 M+ iterations.
- **gc-on: FLAT / BOUNDED** — ~36–38 MB regardless of N. The per-iteration dead cycle is
  reclaimed at the safepoint; steady state does not grow. **#31 is bounded end-to-end.**

## 3. Valgrind on the real workload (N=4,000, `--mode vm`)

`valgrind --leak-check=full`:

| Category | gc-off | gc-on |
|---|---|---|
| definitely lost | **98,880 B / 3,990 blocks** (~1 array header/iter) | **0 B / 0 blocks** |
| indirectly lost | **507,600 B / 11,970 blocks** (~3 nodes/iter) | **0 B / 0 blocks** |
| possibly lost | 2,000,969 B / 7,485 blocks | 1,999,167 B / 7,438 blocks |
| still reachable | 377,279 B / 964 blocks | 398,479 B / 983 blocks |

- gc-off leaks the whole 4-node cycle every iteration (3,990 definitely-lost blocks ≈ N).
- **gc-on: 0 definitely-lost, 0 indirectly-lost — the cycle is fully reclaimed.**
- The ~2 MB "possibly lost" is **identical in both** (string interner / thread-local
  buffers / JIT structures valgrind can't trace interior pointers into) — pre-existing,
  unrelated to the #31 cycle, and does not differ by feature.

This is corroborated at the Rust-carrier level by `gc::tests::
collect_real_closure_in_array_finding31_frees_all_four_nodes`,
`closure_in_array_finding31_bounded_over_iterations`, and
`live_closure_in_array_cycle_is_not_collected` (a live external ref correctly prevents
collection; ScanBlack restores refcounts; nothing is prematurely freed).

## 4. Full `--features gc` suite (unfiltered) vs gc-off baseline

| Suite | gc-on | gc-off baseline | Regression? |
|---|---|---|---|
| shape-value + shape-vm + shape-runtime (lib) | **4266 passed, 0 failed** | 4225 passed, 0 failed | **none** (+41 gc-gated tests) |
| shape-jit (`--test-threads=1`) | **499 passed, 0 failed, 21 ignored** | — | 21 ignored are pre-existing SURFACE/deep-tests-gated |
| shape-test integration (`--no-fail-fast --test-threads=1`) | 7240 passed, **40 failed** | 7240 passed, 40 failed | **none — failure sets IDENTICAL** |

The +41 gc-on-only tests include all Finding #31 / #82 collector tests
(`gc::tests::*`, `executor::tests::gc_teardown::*`) and the barrier / coordinator
tests — all green. No crash (SIGABRT/SIGILL/SIGSEGV) in any gc-on run.

### 4.1 shape-test integration — the 40 failures are ALL pre-existing (not gc-induced)

The 40 failures cluster as: **33 HashMap closure-transformation**
(`hashmap_filter` / `hashmap_map` / `hashmap_group_by` — the documented pre-existing
typed-closure-inference / monomorphization cluster), **3 snapshot**
(`snapshot_preserves_variables`, `snapshot_returns_hash_on_first_run`,
`scoped_contract_snapshot_requires_explicit_import` — the snapshot code carries **zero**
`#[cfg(feature="gc")]` gating, so it is byte-for-byte identical gc-on vs gc-off), and
**4 other** (`optional_chaining_on_existing`, `http_get_with_invalid_url`,
`edge_early_return_skips_subsequent_operations`, and the `zzz_resource_bound_probe::
runaway_loop_fails_in_process` M2 probe — confirmed failing on gc-off in isolation).

**None** of the 40 touch GC / cycles / refcounts / Drop / memory reclamation.

**Baseline diff (the rigorous no-regression proof):** the gc-off baseline was run with the
identical invocation (`cargo test -p shape-test --no-fail-fast -- --test-threads=1`). Result:
**7240 passed, 40 failed — the exact same 40 test names.** `comm`/`diff` of the two sorted
unique-failure sets is **empty in both directions** (zero new failures under gc-on, zero
"fixed"). The failure sets are byte-for-byte IDENTICAL, so gc-on introduces **no regression**
in the integration suite.

## 5. gc-off parity

- All GC code is `#[cfg(feature = "gc")]`-gated in both `shape-value` and `shape-vm`
  (features default OFF). Feature-off, the barriers, candidate buffer, safepoint
  `maybe_collect`, and teardown sweep compile to nothing — the default build is a strict
  no-op. The safepoint block in `dispatch.rs` and the teardown block in `executor/mod.rs`
  are each wrapped whole in `#[cfg(feature = "gc")]`.
- gc-off default suite (value/vm/runtime): **4225 passed, 0 failed**.
- `just check-no-dynamic`: **EXIT 0** (forbidden-symbol gate unaffected by the GC work).

## 6. gc-on hot-loop overhead

The safepoint poll is `maybe_collect(256)` at every 1024 interpreter instructions:
`OnceLock` load (coordinator) + one relaxed `AtomicBool` load (`stop_requested`) +
two thread-local borrows (`ensure_registered`, `candidate_buffer_len`).

| Workload (mode) | gc-off | gc-on | Overhead |
|---|---|---|---|
| Tight arithmetic loop, 20 M iters (interpreter, `--mode vm`) | ~2.065 s | ~2.015 s | **none measurable** (gc-on marginally faster; within run-to-run noise) |
| Tight arithmetic loop, 100 M iters (default JIT) | ~10.03 s | ~9.86 s | **none measurable** |
| Heap-alloc-saturated loop, 5 M array allocs (interpreter) | ~1.85 s | ~2.00 s | **~7–8%** |
| Heap-alloc-saturated loop, 5 M array allocs (default JIT) | ~1.87 s | ~2.00 s | **~7%** |
| Pathological all-garbage cycle-churn (#31 N=500k, quiescent, VERIFY) | ~0.36 s | ~0.42 s | **~15–17%** |

- **The safepoint poll itself is unmeasurable** on a tight loop — a single relaxed load,
  amortized 1/1024 instructions. (An early 4% reading was pure CPU contention from a
  concurrent test suite; it vanished under a quiescent re-measure. VERIFY independently
  confirmed ~0% on tight interpreter, tight JIT, and rc-1 allocation loops.)
- The measurable cost is the **RC increment/decrement barrier** (`gc_increment_barrier` /
  `gc_decrement_precheck`) that fires on every heap refcount op for cycle-capable kinds:
  **~7–8% on allocation-saturated code**, near-zero on compute-bound code, and — when the
  workload is *nothing but* dead cycles being collected every safepoint (the #31 pathology
  itself) — **~15–17%** (the collection work, not just the barrier). This is the intrinsic
  price of cycle collection and only affects opt-in gc-on builds.

## 7. Residual risk

1. **CAVEAT (VERIFY) — the flagship program panics at teardown, not a clean exit.**
   **[RESOLVED 2026-07-08 — see UPDATE block above; the callable-array wire arm
   now projects to a display placeholder and the repro exits 0.]**
   `for i in 0..N { leak() }` (the leak-only form) exits **101 with a panic** at
   `crates/shape-runtime/src/wire_conversion.rs:507` — the trailing callable-array is not
   wire-serializable (`panic!("TypedArray wire conversion requires a known producer-side
   element stamp")`). This is present **byte-identically in gc-off** (it is not gc-gated
   code) and fires **only after** the measured loop completes, so it does **not** affect the
   RSS-bounded or valgrind proof (both sample during/at the steady-state loop). A
   trailing-scalar variant of the repro exits 0. Disclosed here because the VALIDATE draft
   said the program "runs" without naming the panic. Orthogonal to GC; a wire-serialization
   gap, not a leak.
2. **CAVEAT (VERIFY) — the bare untyped #31 source form fails at RUNTIME, not compile.**
   `var arr = []; arr.push(|| arr.len())` surfaces a **runtime `op_new_array(0)` SURFACE**
   (empty-array element kind unresolved at the new-array opcode), not the clean compile
   error the VALIDATE draft described. The form works with the explicit annotation
   `let mut arr: Array<() -> int> = []`. **The annotated form is the identical 4-node cycle
   topology and is fully collected** (proven above), so **#31 is collectable end-to-end** —
   but only the annotated form; the bare form never reaches a running leak. Type/opcode
   ergonomics gap, orthogonal to GC.
3. **The closure-push opcode JIT-falls-back to the interpreter** (`TypedArrayPushCallable`
   has no FrameDescriptor — a separate pre-existing JIT SURFACE, v0.4). The GC still bounds
   the workload because the safepoint lives in the interpreter dispatch loop that executes
   the fallback. Not a GC concern, but the #31 cycle is currently collected on the
   interpreter path, not a JIT-native one.
4. **Perf: ~7–8% on allocation-saturated workloads, ~15–17% on all-garbage cycle-churn**
   (RC barrier + collection), §6; ~0% on compute-bound and normal-allocation code.
   Acceptable for opt-in; would warrant a barrier-fast-path review before making gc
   unconditional.
5. **The snapshot v6→v7 identity-map change is NOT gc-gated — it already ships in the
   default build.** GC Phase 5 generalized the snapshot identity-map to all cycle-capable
   HeapKinds and bumped the format v6→v7; this code carries **zero** `#[cfg(feature="gc")]`
   gating (which is *why* the 3 snapshot integration failures are identical gc-on/gc-off).
   Consequence: the snapshot behavior/format change is live in the default `shape` binary
   regardless of whether the gc feature is flipped. Not a regression (baseline is
   byte-identical), but the reader should know it is not toggled by the flip.
6. **Cross-thread `SharedAtomic` cycles are the documented Phase-6 deferral.** The
   validated proof is single-VM-task cycles (Finding #31). Phase-3b cross-worker
   stop-the-world rendezvous IS merged (the coordinator can stop other workers), but a
   cycle whose edges span async-worker / JIT threads via `SharedAtomicMut` is explicitly
   out of v1 scope per the design (`docs/design/real-gc-cycle-collection.md` §Phase 6) and
   is **NOT collected**.

## 8. The exact one-line flip mechanism (APPLIED 2026-07-08 — see UPDATE block above)

Add `gc` to the shipped VM's default features. Single edit in
`crates/shape-vm/Cargo.toml` line 54:

```toml
default = ["jit"]          # →
default = ["jit", "gc"]
```

`shape-vm/gc = ["shape-value/gc"]` forwards to `shape-value`, so this single change turns
on the metadata, barriers, safepoint, and teardown sweep for the default `shape` binary
(shape-cli inherits shape-vm's default features). No other edit is required.

---

## Reproduction commands

```bash
# build
direnv exec /home/dev/dev/shape-lang cargo build --release --bin shape --features gc
direnv exec /home/dev/dev/shape-lang cargo build --release --bin shape            # gc-off

# #31 RSS proof (repro in scratchpad f31_*.shape)
/usr/bin/time -v ./shape-gcoff run f31_1000000.shape   # RSS ~219 MB (growing)
/usr/bin/time -v ./shape-gcon  run f31_1000000.shape   # RSS ~37 MB  (bounded)

# valgrind
valgrind --leak-check=full ./shape-gcoff run --mode vm f31_4000.shape   # 98,880 definitely lost
valgrind --leak-check=full ./shape-gcon  run --mode vm f31_4000.shape   # 0 definitely lost

# suites
direnv exec /home/dev/dev/shape-lang cargo test -p shape-value -p shape-vm -p shape-runtime --features gc
direnv exec /home/dev/dev/shape-lang cargo test -p shape-jit --features gc -- --test-threads=1
direnv exec /home/dev/dev/shape-lang bash -c 'ulimit -v 50331648 && cargo test -p shape-test --features shape-vm/gc --no-fail-fast -- --test-threads=1'
direnv exec /home/dev/dev/shape-lang just check-no-dynamic   # EXIT 0
```

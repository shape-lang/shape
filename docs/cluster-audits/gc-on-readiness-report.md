# GC-on Readiness Report (Finding #31 — per-iteration garbage-cycle leak)

**Lane:** `wave7/gc-on-readiness` (worktree `shape-w7-gcon`, off `main` HEAD with all GC phases merged).
**Date:** 2026-07-08.
**Scope:** PHASE 1 — VALIDATE gc-on. Build `--features gc`, measure the Finding #31
RSS-bounded proof, run the full gc-on suite, confirm gc-off parity, measure gc-on
hot-loop overhead. **This lane does NOT flip the default** (that is the user go/no-go).

---

## VERDICT: **GO** (with one honest caveat + one perf note, below)

gc-on demonstrably fixes the Finding #31 leak end-to-end: the real closure-in-array
cycle (`var arr = []; arr.push(|| arr.len())`, expressed with the required element-type
annotation) has **BOUNDED RSS with gc-on** and **linearly-GROWING (unbounded) RSS with
gc-off**, corroborated by valgrind (0 definitely/indirectly-lost with gc-on vs
606 KB lost with gc-off at N=4000). The full `--features gc` suite is green with **zero
regressions** vs the gc-off baseline. gc-off is a compile-time no-op (all GC code is
`#[cfg(feature = "gc")]`-gated). The safepoint poll is unmeasurable on a tight loop.

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

- **The safepoint poll itself is unmeasurable** on a tight loop — a single relaxed load,
  amortized 1/1024 instructions. (An early 4% reading was pure CPU contention from a
  concurrent test suite; it vanished under a quiescent re-measure.)
- The measurable cost is the **RC increment/decrement barrier** (`gc_increment_barrier` /
  `gc_decrement_precheck`) that fires on every heap refcount op for cycle-capable kinds:
  **~7–8% on allocation-saturated code**, near-zero on compute-bound code. This is the
  intrinsic price of cycle collection and only affects opt-in gc-on builds.

## 7. Residual risk

1. **CAVEAT — the bare untyped #31 source form does not compile (type system, not GC).**
   `var arr = []; arr.push(|| arr.len())` is rejected by strict typing: an empty array's
   element type must be inferable, and a closure element pushed to a bare `[]` is not
   resolvable. The form compiles and runs with the explicit annotation
   `let mut arr: Array<() -> int> = []`. **The annotated form is the identical 4-node
   cycle topology and is fully collected** (proven above), so **#31 is collectable
   end-to-end** — but a user writing the bare form gets a compile error, not a running
   leak. This is a type-system ergonomics gap, orthogonal to GC.
2. **The closure-push opcode JIT-falls-back to the interpreter** (`TypedArrayPushCallable`
   has no FrameDescriptor — a separate pre-existing JIT SURFACE, v0.4). The GC still bounds
   the workload because the safepoint lives in the interpreter dispatch loop that executes
   the fallback. Not a GC concern, but the #31 cycle is currently collected on the
   interpreter path, not a JIT-native one.
3. **Perf: ~7–8% on allocation-saturated workloads** (RC barrier), §6. Acceptable for
   opt-in; would warrant a barrier-fast-path review before making gc unconditional.
4. **Cross-thread `SharedAtomic` cycles are the documented Phase-6 deferral.** The
   validated proof is single-VM-task cycles (Finding #31). Phase-3b cross-worker
   stop-the-world rendezvous IS merged (the coordinator can stop other workers), but a
   cycle whose edges span async-worker / JIT threads via `SharedAtomicMut` is explicitly
   out of v1 scope per the design (`docs/design/real-gc-cycle-collection.md` §Phase 6).

## 8. The exact one-line flip mechanism (for the user go/no-go — NOT done in this lane)

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

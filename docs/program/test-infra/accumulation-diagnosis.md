# Test-infra memory diagnosis (#206)

**Date:** 2026-07-28 · **Baseline commit:** `38345e03` · **Branch:** `wave1-infra`
**Host:** 94 GB RAM, `nproc` = 32, GNU `ld`, no `mold`/`lld` installed.

## Headline

**The ticket's premise is refuted as the cause of the incident. There is no
per-test accumulation on the interpreter path.** A single process runs the
entire 3,583-test `shape-vm --lib` suite at `--test-threads=1` with a peak RSS
of **113–124 MB**.

That statement is narrower than it looks, and the narrowing matters:
`shape-vm` does not link `shape-jit`, so the anchor measurement is evidence
about the interpreter only. On the JIT path there **is** a real, unbounded,
leak-by-design (Cranelift `JITModule` pages are never freed), measured here at
+18 MB across 60 JIT-armed tests versus a matched control. It is bounded in
practice only because `.with_jit()` is opt-in and rare — an accident, not a
design. Detail in the JIT section, which corrects an earlier "ruled out"
verdict in this same document.

The ~60 GB OOM is a **build-phase** failure, not a test-run failure:
`cargo test -p shape-test` links ~60 integration-test binaries, each a
workspace-sized static link costing ~1.8 GB of linker RSS, and Cargo releases
them to the jobserver simultaneously. Peak build RSS is linear in `jobs`:

| `--jobs` | peak tree RSS | processes at peak | GB per job |
|---:|---:|---:|---:|
| 8  | 15.1 GB | 41  | 1.89 |
| 16 | 29.2 GB | 81  | 1.83 |
| 32 (default = nproc) | **≥29.8 GB and still climbing when the 20 GB safety cap fired at t=6.3s** | 157 | — |

Extrapolating the measured 1.86 GB/job to the default `jobs = 32` predicts
**~59 GB**, which matches the reported ~60 GB. Two concurrent worktrees each
building at default parallelism is not required to explain the incident; one is
almost enough.

## Method

Two samplers were added, both of which hard-kill their subject if it crosses a
cap, so a memory investigation can never itself OOM the host:

- `scripts/rss-profile.sh` — samples one test process's `VmRSS` and, at each
  sample, the number of tests completed. Emits `(elapsed, tests_done, rss_kb)`,
  which is the per-test retention profile directly, with no timestamp joining.
- `scripts/rss-tree-profile.sh` — sums RSS across a whole process *tree*
  (`setsid` + process-group match). `cargo test` is cargo + N rustc + N linkers
  + test binaries; only a tree measurement attributes peak memory between the
  build and run phases.

No test semantics were changed and no test was skipped or deleted.

## Evidence per suspected source

### Ruled out — per-test accumulation in the VM (Arc cycles, VM teardown)

`shape-vm --lib`, all 3,583 tests, one process, `--test-threads=1`:

```
tests=296   rss= 65 MB      tests=1794  rss= 86 MB      tests=3014  rss= 89 MB
tests=594   rss= 68 MB      tests=2083  rss= 87 MB      tests=3488  rss= 92 MB
tests=1001  rss= 77 MB      tests=2551  rss= 88 MB      tests=3531  rss= 92 MB
tests=1265  rss= 84 MB      tests=2784  rss= 87 MB      peak       113 MB
```

Steady-state least-squares slope past warmup: **9.2 KB/test**. RSS goes flat
after ~1,500 tests and even dips (2,551 → 2,784 tests: 88 → 87 MB), which is
inconsistent with monotonic retention. Whatever Arc cycles a dropped
`VirtualMachine` leaves behind, they do not add up to a memory problem: 3,583
VM lifecycles cost ~27 MB total.

At default parallelism (32 threads) the same suite peaks at 1.17 GB in 8s —
that is 32 concurrent VMs, i.e. concurrency, not accumulation.

There is also a mechanism, not just a curve. `VirtualMachine` has an explicit
`impl Drop` (`crates/shape-vm/src/executor/mod.rs:703-880`) that releases shared
module bindings, the live stack window, and module bindings by kind — and then,
at `:857-878`, runs a **teardown cycle collection**:

```rust
#[cfg(feature = "gc")]
{
    shape_value::gc_coordinator::collect_under_stop(shape_value::gc::collect_cycles);
    shape_value::gc::clear_candidate_buffer();
}
```

Its comment names exactly the case that would otherwise leak: a module-scope
self-referential cycle "becomes unreachable only *here* … so the mid-run
safepoint collector never observed it becoming garbage." The final
`clear_candidate_buffer()` exists so a VM reusing the OS thread starts clean.
So dropping a VM with live Arc cycles does **not** permanently leak them, given
`gc` — which is default-on for shape-vm (`Cargo.toml:57`).

Three residual surfaces remain, none large enough to matter here but worth
naming rather than implying the path is spotless:

1. The candidate buffer is thread-local, so a cycle whose decrements happened on
   a different thread than the dropping thread is not in it.
2. `crates/shape-vm/src/executor/mod.rs:847-855` admits in-code that if
   `module_bindings` outgrew `module_binding_kinds`, the tail is zeroed but not
   released and "the corresponding strong-count share leaks". This is the only
   VM-teardown path that concedes a leak; it is the best remaining candidate for
   a small per-test residue, though the flat curve argues it rarely fires.
3. With `gc` off the whole block is absent and cycles leak for the process
   lifetime.

**Split-feature hazard, recorded while we were here.** `gc` is default-ON for
shape-vm but default-OFF for shape-jit (`crates/shape-jit/Cargo.toml:50`).
shape-cli reconciles them (`default = ["jit","gc"]`, `gc = ["shape-vm/gc",
"shape-jit?/gc"]`), but any build that is not shape-cli — e.g.
`cargo test -p shape-jit` — gets a gc-on interpreter with a gc-OFF JIT, where
`jit_write_barrier` compiles to a no-op. shape-vm's own manifest comment claimed
"Default OFF" while line 57 had it ON; that contradiction is fixed in this
branch.

### JIT code pages — CONFIRMED live leak, but not the incident

**This section previously said "ruled out". That was wrong, and its evidence was
invalid.** It is corrected in place rather than quietly amended, because the
original error is instructive.

The original argument was: `jit` (44 tests) peaks at 313 MB while `operators`
(618 tests) peaks at 363 MB, so JIT targets do not dominate. Both halves are
broken:

- **`shape-vm` does not depend on `shape-jit` at all.** Its `[dependencies]`
  lists no `shape-jit` and no `cranelift-*`; `crates/shape-vm/Cargo.toml:67-70`
  says so outright ("Cranelift codegen itself lives in the shape-jit crate;
  shape-vm carries no cranelift deps"). `shape-vm/jit` being default-on gates
  only OSR/tier-dispatch hooks. So the 3,583-test measurement that anchors this
  document **cannot** observe a JIT page: zero occurrences, zero bytes. It is
  evidence about the interpreter, and says nothing about the JIT either way.
- **The `jit` shape-test target never turns the JIT on.** `ShapeTest` defaults
  to `ExecMode::Vm`; the opt-in is `.with_jit()`, and
  `grep -rc 'with_jit()' tools/shape-test/tests/jit/` returns **0** in all three
  files. `tools/shape-test/tests/jit/correctness.rs:1-5` admits it: "we verify
  correctness by running code through the interpreter and trusting the JIT must
  match." The comparison was interpreter versus interpreter.

The leak is real. One `JITModule` is built per `JITExecutor::execute_program`
call (`crates/shape-jit/src/executor.rs:666`) and dropped at end of scope.
Cranelift's `Memory::drop` deliberately leaks — it `mem::forget`s its
allocations "to guarantee validity of function pointers". `JITModule::free_memory`
exists in the crate and is **never called anywhere in this workspace**. The
pages come from the Rust global allocator, page-rounded and `mprotect`ed, so
they count against RSS permanently and the allocator can never reuse them.

**Measured, with a matched control.** In `comptime`, the most JIT-dense target:
60 tests drawn from modules that call `.with_jit()` against 60 from modules that
do not — same binary, each cold in its own process, `--test-threads=1`:

```
60 tests from JIT-armed modules:  growth 68.6 MB, peak 77 MB
60 tests from non-JIT modules:    growth 50.6 MB, peak 63 MB
                                  delta  +18.0 MB growth, +14 MB peak
```

Equal counts and both cold, so the allocator warm-up that confounds unequal
comparisons is held constant. The delta is an **upper bound** on the JIT's
share: these tests run each program twice (the helpers are literally named
`expect_vm_and_jit_output`), so the extra interpreter pass and the cache keys it
mints are inside the +18 MB too. Order of magnitude: a few hundred KB per JIT
execution, consistent with a page-rounded three-arena `JITModule` holding a
full-stdlib compile.

**Why it is still not the incident.** The arming surface is small and bounded:
`.with_jit()` appears 68 times repo-wide across 10 shape-test targets
(`annotations_comptime` 17, `comptime` 15, `type_aliases_unions` 9,
`stdlib_statistics` 8, `arrays_vectors` 7, then single digits). Each target is
its own process, so the leak cannot compound across targets. The worst observed
whole-process peak among them is `comptime` at 79 MB.

**It is a latent scaling risk, not a closed issue.** Three independent accidents
mask it: shape-vm not linking shape-jit, `.with_jit()` being opt-in and rare,
and the heavy in-crate shape-jit suites being `deep-tests`-gated. Any one of
those changing — someone adding a broad JIT parity sweep, say — turns it into
hundreds of KB per test of unbounded growth. The remedy already exists in the
tree, unwired: `JitCodeCache` (`crates/shape-jit/src/jit_cache.rs:53`) is a
content-hash-keyed native-pointer cache referenced by nothing outside its own
file and test module. Wiring it, or holding one long-lived `JITCompiler`, is the
known fix. It is also why ~118 stdlib functions are recompiled on every
`execute_program` call: `compile_program_selective` walks all of
`program.functions` with no cache lookup at any point.

For contrast, `crates/shape-jit/src/jit_matrix.rs:59,79` is correctly balanced
(`from_arc` leaks one strong reference, `Drop` reconstitutes it). Recorded so it
is not re-flagged.

### Reference: run-phase peaks by shape-test target

Retained as baseline data. Read these as *interpreter* numbers unless the target
appears in the `.with_jit()` list above.

| shape-test binary | tests | peak RSS | arms JIT? |
|---|---:|---:|---|
| `numeric_conversions_jit` | 118 | 390 MB | 2 sites |
| `operators` | 618 | 363 MB | no |
| `control_flow` | 490 | 360 MB | no |
| `lsp` | 507 | 336 MB | 1 site |
| `jit` | 44 | 313 MB | **no** |
| `async_concurrency` | — | 262 MB | no |
| `snapshots_resume` | — | 180 MB | no |
| `comptime` | 274 | 79 MB (serial) | 15 sites |

### Static caches — unbounded by key, saturating on these suites

The naive form of this verdict ("a monotonic cache would show a constant slope;
ours flattens, so no unbounded cache") is too weak, because the caches that
exist are not keyed by test.

**The discriminator is key space, not test count.** Every process-global holder
below is keyed by something whose distinct-value count is bounded by the
suite's distinct *sources / types / field-name sets*, not by how many tests run.
A suite that reuses fixtures saturates them; a workload that mints a **new key
per test** (synthesized module sources, generated type names, generated field
sets) makes the very same structures unbounded. So "flattens after ~1,500
tests" is the signature of a finite key space filling up — which is consistent
with the measurements and is *not* a guarantee for a different workload.

The unbounded-by-key holders, ranked by bytes per key:

| site | shape | key |
|---|---|---|
| `crates/shape-runtime/src/module_loader/mod.rs:54` | `PARSE_CACHE: OnceLock<Mutex<HashMap<String, Arc<Program>>>>`, insert-only, no eviction | **entire module source text** → entire parsed AST |
| `crates/shape-runtime/src/type_schema/current.rs:69` | `DEFAULT_SCHEMA_REGISTRY: LazyLock<Arc<TypeSchemaRegistry>>`; grows via `RwLock` interiors `by_content`, `predeclared_cache`, `predeclared_by_id` in `type_schema/registry.rs` | distinct ordered field-name tuples |
| `crates/shape-value/src/shape_graph_current.rs:106` | `DEFAULT_SHAPE_TABLE: LazyLock<Arc<ShapeTableHandle>>`; `shapes` Vec clones its parent's property Vec per push (O(k²) for a depth-k chain), plus a `transition_log` drained only if a JIT tier manager polls | distinct hidden-class transitions |
| `crates/shape-jit/src/ffi/string.rs:176` | `intern_pool: OnceLock<Mutex<HashMap<String, Arc<String>>>>`, bumps a strong count per call and never releases | distinct string-literal content |

The first three share a **fallback trap** worth flagging: `current_registry()`
and `try_current_shape_table()` return the process-global default whenever no
task-local scope is installed, and never return `None`. So every test that does
not wrap execution in a scope writes into the one global object.

Ruled out by contrast, so they are not re-flagged:
`crates/shape-value/src/string_intern.rs:66` is hard-capped at
`INTERN_CAP = 8192` entries of ≤32 bytes; the `phf` method-registry tables are
compile-time immutable; `crates/shape-jit/src/jit_cache.rs` is dead code with no
owner (it is *not* a live JIT code cache — see the JIT section, where wiring it
is the proposed fix).

One tripwire for a future reader: `OPTCHAIN_COUNTER`
(`crates/shape-ast/src/transform/desugar.rs:15`) never resets, so desugared
identifier names drift monotonically across programs in one process. It holds no
memory itself, but it is exactly the shape of thing that could become an
unbounded *key generator* for the schema and shape tables above if a desugared
name ever reaches one of those keys.

### Real leaks that exist but are not this incident

A static/teardown code sweep run alongside the measurements did find genuine
leaks. They are recorded here because "the OOM was the build" must not be heard
as "the runtime is clean". Each is real; none explains the incident, and the
measurements above say why.

**Headline — `box_column_result` leaks a whole `Vec<f64>` per call.**
`crates/shape-jit/src/ffi/value_ffi.rs:510` does
`Box::leak(data.into_boxed_slice())` with a doc comment saying the caller must
free it. Verified: **no `free_column` / `drop_column` / `shape_free_column`
exists anywhere in the workspace** (grep returns zero hits). Verified call
sites: 7 in `crates/shape-jit/src/ffi_symbols/series/mod.rs`, 8 in
`crates/shape-jit/src/ffi_symbols/intrinsics/mod.rs`. This leaks per **call**,
not per test or per program, so a long-running JIT column-math workload leaks
without bound. Tracked as **#208**.

Why it does not show up in these measurements — and note this is a *different*
reason from the one first written here. The initial explanation ("the T1 @ 100 /
T2 @ 10k tier thresholds mean few tests JIT-compile") is wrong about the
mechanism: those thresholds gate only the `JitCompilationBackend` worker path
(`crates/shape-jit/src/worker.rs:19`). The `JITExecutor::execute_program` path
is ahead-of-time whole-program compilation with **no threshold at all** — it
fires on first execution. The real reasons the column leak stays invisible are
that `shape-vm` does not link `shape-jit` (so the anchor measurement cannot
reach it), that `.with_jit()` is opt-in and used in only 10 targets, and that
the series/intrinsic column API is a narrow path few of those touch. It leaks
per *call inside JIT-emitted code*, so its profile is bimodal: zero for almost
every test, then unbounded for any test that runs a hot column loop. A bimodal
leak cannot show up as a smooth per-test slope, which is exactly why a real
production-path defect can sit behind a flat RSS curve.

The static-cache inventory that used to be duplicated here now lives in
"Static caches — unbounded by key" above, with the key-space framing that
actually explains the flattening.

### Per-module attribution, and why the residual slope is not retention

Running each top-level module as its own slice (`--test-threads=1`) localises
the whole residual to `executor::`:

| module | tests | peak | slope |
|---|---:|---:|---:|
| `compiler::` | 1,753 | 97 MB | **−1,203 B/test** (releases) |
| `executor::` | 1,148 | 92 MB | **+12,764 B/test** |
| `mir::` / `bytecode::` / `tier::` / `feature_tests::` | 10–21 | 13–66 MB | too few samples |

Two controls then establish that even this residual is not retention.

**Control 1 — order independence.** The 1,256 `executor::` tests run from an
explicit `--exact` list, forward and reversed:

```
executor fwd  peak=93 MB  slope=13736 B/test   (26.5s)
executor rev  peak=93 MB  slope=13147 B/test   (19.3s)
```

Identical. The slope is not an artifact of heavier tests sorting later.

**Control 2 — warm heap.** The *same* 1,256 tests, measured inside the full
suite, where ~1,800 compiler tests have already run and freed their heap:

```
executor standalone (cold heap):      slope = +13736 B/test, peak  93 MB
executor within full run (warm heap): slope =  -1448 B/test, peak 110 MB
```

The slope **inverts**. That is the signature of an allocator ramping to its
working-set high-water mark, not of a leak: once the process already holds a
heap, executor tests reuse memory freed by the compiler tests and RSS stops
growing. This is the strongest single piece of evidence against the
accumulation hypothesis, and it is why the full-suite peak sits at 113–124 MB
rather than climbing.

Consequence for the tripwire: the **peak** bound is the meaningful guard. The
slope bound is a secondary signal that partly measures allocator warm-up, and
should not be read as a leak rate.

### Confirmed — build-phase link fan-out

`cargo test -p shape-test --no-run` after touching `tools/shape-test/src/lib.rs`
(forces relink of all ~60 test binaries), default parallelism:

```
t=0.0  procs=2    tree= 0.0 GB  top=direnv/5MB
t=1.0  procs=2    tree= 0.7 GB  top=rustc/621MB
t=3.0  procs=2    tree= 1.1 GB  top=rustc/989MB
t=4.1  procs=5    tree= 1.6 GB  top=rustc/578MB
t=5.2  procs=157  tree=10.5 GB  top=ld/248MB      <-- fan-out
t=6.3  procs=161  tree=28.4 GB  top=ld/859MB      <-- +18 GB in one second
                                                   (safety cap fired here)
```

The transition at t=5.2s is the whole defect: 5 processes become 157 as every
test binary starts linking at once. Individual `ld` processes peak at ~1.7 GB;
test binaries are ~230 MB each with full debuginfo (16 GB of `target/debug/deps`).

### Corroborating live incident, 2026-07-28

The measured runs in the table above are themselves the incident: a cold
`cargo test -p shape-test --no-run` at default `jobs = nproc = 32`, issued from
this worktree during Phase-1 prep, throttled the host. Independently observed
from outside the run: **~49 GiB held in concurrent linker chains**
(cc → collect2 → ld), one workspace-sized `ld` measured at **1.42 GiB live**,
memory-pressure avg10 **99%**, host load **121**.

That external 1.42 GiB/link figure and this investigation's 1.86 GB/job slope
were arrived at separately and agree, which is what makes the extrapolation to
~59 GB at `jobs = 32` trustworthy rather than a single-source estimate. Note the
two figures measure different things and should not be conflated: 1.42 GiB is
one `ld`'s live RSS, 1.86 GB is the per-job cost of the whole chain (rustc +
cc + collect2 + ld) amortised across a build.

**Reproducing the build-phase table is itself the hazard.** Anyone re-measuring
it must do so under `scripts/rss-tree-profile.sh` with a cap well under free
memory, and must not run it concurrently with any other cargo invocation.

### Operational rules now binding on this workspace

1. Never build all of shape-test at once. Build exactly the one target needed:
   `cargo test -p shape-test --test <name> --no-run`. Measured cost of that
   form after touching `tools/shape-test/src/lib.rs`: **1.9 GB peak, 6
   processes**, of which a single `ld` is 1.72 GB. That one linker is the
   irreducible floor for any shape-test build — but it is ~30x cheaper than the
   all-targets form at default parallelism, and it stays flat regardless of the
   `jobs` setting because there is only ever one link in flight.
2. One cargo invocation at a time, per worktree and across worktrees.
3. `.cargo/config.toml`'s `[build] jobs` bound is not optional and must not be
   removed; the first build after it lands is a cold-ish rebuild and will be
   slow at `jobs = 8`. That is the intended trade.

## Interventions measured

| intervention | peak build RSS |
|---|---:|
| default (`jobs=32`) | ~59 GB (projected; ≥29.8 GB measured before cap) |
| `jobs=16` | 29.2 GB |
| **`jobs=8`** | **15.1 GB** (14.2 GB re-measured via config) |
| `jobs=8` + `split-debuginfo=unpacked` | 11.7 GB |
| `jobs=8` + `debug=line-tables-only` | 12.0 GB |

Parallelism is the dominant lever by a wide margin; the debuginfo levers buy
another ~20% and cost debuggability, so they are **not** applied. The fix is the
`jobs` bound alone.

## Why the existing guard never fired

`justfile` carries `test-mem-cap-kib := "50331648"` (48 GiB) applied as
`ulimit -v`. That is a **per-process** virtual-memory cap. The real failure is
~32 concurrent processes of ~1.8 GB each; no single one approaches 48 GiB, so
this backstop cannot fire for this failure mode. It remains useful for the
runaway-single-test case it was written for; it is simply aimed at a different
shape of failure.

## The two siblings

**The 613-test bulk hang does not reproduce at `38345e03`.** The suite is
`tests/operators` (now 618 tests). At `--test-threads=1` it completes in 105s
with a flat 52 → 55 MB RSS; at default parallelism, 618 passed in 11.8s. The
most likely historical explanation is that a bulk run was performed right after
a change, so it included a full rebuild; the build drove the host into swap and
the run appeared to hang. That is consistent with the original report that
subsets and `--exact` runs were fine — those were typically issued against an
already-built target directory.

**The 42-failure parallel-load spook does not reproduce either.** The suite
returns **exactly 7 failures in both modes** — `--test-threads=1` (89.5s) and
default 32-thread parallelism (21.7s) — and they are the known pre-existing #204
set. Parallelism changed the failure count by zero here. This does not prove the
flake is gone, but it removes memory pressure as an explanation on a
non-contended host, and points at #205's nondeterministic selection instead.

## Fix landed

1. `.cargo/config.toml` — `[build] jobs = 8`. Declared bound: **peak build RSS
   ≤ ~16 GB** (measured 14.2–15.1 GB).
2. `scripts/rss-tripwire.sh` + `just rss-tripwire` — CI-runnable, two guards:
   - the build-parallelism bound is still configured and `≤ 12`;
   - one process over the full `shape-vm --lib` slice stays under **256 MB peak**
     (measured 113–121 MB) and **24 KB/test slope** (measured 9.2 KB/test).

Both bounds sit at roughly 2–3x measured, enough to absorb allocator and libtest
noise while still failing loudly on a real regression.

## Open items

- The residual ~9 KB/test interpreter slope is explained as allocator warm-up
  plus finite-key-space caches saturating, and it inverts on a warm heap. It was
  not chased to a single named allocation; the tripwire bounds it either way.
  The cheap next probe, if anyone wants the last word: sample
  `TypeSchemaRegistry`'s `predeclared_by_id` / `by_content` lengths,
  `ShapeTransitionTable::shape_count()` plus the transition-log length, and
  `PARSE_CACHE`'s len at tests 100 / 1,000 / 3,000. That attributes the slope
  directly and shows which key space is still climbing versus saturated.
- **The tripwire does not cover the JIT.** It runs `shape-vm --lib`, which
  cannot link shape-jit, so a JIT page-leak regression is invisible to it. A
  second slice over a `.with_jit()`-dense target (`comptime` or
  `annotations_comptime`) would close that gap. Not added here because its
  bound would be set on a leak we already know is live and unfixed — the honest
  order is fix `JitCodeCache` wiring first, then bound it.
- The 60 GB incident was a build, but if a *runtime* OOM is ever observed, note
  which binary it was: `tools/shape-test` links shape-jit and runs thousands of
  Shape programs in one process, so the JIT page leak and the `box_column_result`
  leak are both live candidates there in a way they are not in `shape-vm --lib`.
- `mold`/`lld` are not installed on this host. Either would cut per-link RSS
  substantially and allow raising `jobs` back up; that is a toolchain/infra
  change, deliberately not made here.
- `.cargo/config.toml`'s comment notes an infra-level `CARGO_BUILD_JOBS`
  deployment as the intended longer-term home for this bound.

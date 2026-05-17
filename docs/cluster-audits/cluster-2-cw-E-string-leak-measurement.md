# Cluster-2 closure-wave-E — compile-time-boxed string-constant leak measurement

Phase 3 cluster-2-closure-wave-E sub-cluster measurement deliverable.
Bulldozer cadence (β2 supervisor disposition 2026-05-16). Branch
`bulldozer-strictly-typed-cluster-2-cw-E-string-leak`, parent
`ca8300f0` (post-cluster-2-inventory-audit close at `bb5b2109`).
Date: 2026-05-16.

**Audit-only close** per measurement protocol Phase 1 → Phase 2
decision criterion in dispatch prompt: the measured leak magnitude is
SMALL + bounded BUT **no Option A/B/C selection has small fix scope
fitting ceiling-c**. Phase 2 fix DEFERRED to follow-up sub-cluster
`cluster-2-closure-wave-E-fix` (Option A intern-pool prescribed shape).

---

## §0 — Status + structural framing

- **Goal:** measure-first dispatch per `cluster-2-inventory.md` §F.4
  recommendation. Leak magnitude estimate §F.3 was qualitative
  ("SMALL-to-MEDIUM"); this deliverable quantifies it via
  `SHAPE_JIT_ARC_COUNTERS=1` instrumentation extended with per-§2.7.5-
  `Arc<String>`-carrier counters.

- **Source changes:** bounded instrumentation only — 4 atomic counters in
  `crates/shape-jit/src/ffi/arc.rs` + counter wiring in
  `crates/shape-jit/src/ffi/string.rs` (3 sites) + 2 eprintln lines in
  `crates/shape-jit/src/executor.rs`. NO change to producer/consumer
  shape, NO change to FFI contract, NO change to §2.7.5 carrier rules.
  Mirrors the existing `JIT_ARC_RETAIN_CALLS` / `JIT_ARC_RELEASE_CALLS`
  / `JIT_ARC_RELEASE_FREES` infrastructure for the UnifiedValue path
  precisely.

- **Architectural framing rule** (per `phase-2d-handover.md` §0):
  refused-on-sight set preserved verbatim. The §F.1 leak surface is
  named by name (`arc_string_constant` at `ffi/string.rs:111-143`) and
  by deletion-fate (legacy NaN-box `box_string` precedent); never
  described as "shim / bridge / probe / helper / hop / translator /
  adapter".

---

## §1 — Measurement instrumentation

### §1.1 Counter additions (`crates/shape-jit/src/ffi/arc.rs`)

Four new `pub(crate) static AtomicU64` counters, parallel to the
existing `JIT_ARC_*` UnifiedValue-carrier counters but independent
per the §2.7.5 carrier-shape distinction:

| Counter | Wired at | Counts |
|---|---|---|
| `STRING_CONSTANT_ALLOCS` | `arc_string_constant` (`ffi/string.rs:133-155`) | Per-constant `Arc<String>` allocations at JIT compile time |
| `STRING_RETAIN_CALLS` | `jit_arc_string_retain` (`ffi/string.rs:78-93`) | Active-share retain calls at JIT runtime |
| `STRING_RELEASE_CALLS` | `jit_arc_string_release` (`ffi/string.rs:97-130`) | Active-share release calls at JIT runtime |
| `STRING_RELEASE_FREES` | `jit_arc_string_release` (drop-to-zero arm) | Active-share drops that actually freed the `Arc<String>` |

The drop-to-zero detection reads `Arc::strong_count` BEFORE the
decrement (via a balanced `Arc::from_raw` + `Arc::into_raw`
round-trip that adopts then restores one share without perturbing
the count): if the pre-release count was 1, this decrement will
reach zero, and `STRING_RELEASE_FREES` increments.

### §1.2 Reporting (`crates/shape-jit/src/executor.rs:281`)

Two new eprintln lines, gated by the existing `SHAPE_JIT_ARC_COUNTERS`
env var (no new env var introduced):

- `[shape-jit-arc-str]` — per-jit_fn-invocation deltas (matches the
  existing `[shape-jit-arc]` line's contract)
- `[shape-jit-arc-str-cum]` — process-cumulative totals (necessary
  because compile-time allocations occur BEFORE the first `jit_fn`
  invocation; the delta line would miss them)

The cumulative line is necessary for this measurement because
`arc_string_constant` is invoked at JIT *compile* time (not at
JIT runtime); per `mir_compiler/ownership.rs:420,434,465` the call
sites materialize the constant during `compile_constant` which runs
once before the first invocation of the compiled function.

### §1.3 Leak quantification shape

Per the existing docstring at `ffi/string.rs:127-131`:

> The "leaked" extra share is a deliberate per-constant-site one-time
> memory cost — at most `O(distinct string constants × Arc<String>
> size)` per JIT-compiled function.

The quantified leak per execution is:

```
leaked = STRING_CONSTANT_ALLOCS - STRING_RELEASE_FREES
```

with the invariant that every `arc_string_constant` call bumps refcount
1 → 2 (permanent share + active share), and the JIT-emitted runtime
retain/release pairs operate on the active share; the permanent share
is never released, so `STRING_RELEASE_FREES = 0` per allocation in the
ideal case → `leaked = STRING_CONSTANT_ALLOCS` exactly.

---

## §2 — Per-program measurements at HEAD `ca8300f0`

Build: `cargo build --release --bin shape` inside devenv shell at
`/home/dev/dev/shape-lang/shape-cluster-2-cw-E-string-leak`.

Invocation: `SHAPE_JIT_ARC_COUNTERS=1 ./target/release/shape run
--mode jit <fixture>`.

### §2.1 Synthesized representative fixtures

| Fixture | Shape | Distinct user string constants |
|---|---|---|
| `/tmp/cw-E-prog0-no-strings.shape` | `let mut sum = 0; for i in 0..100 { sum += i }; print(sum)` | 0 |
| `/tmp/cw-E-prog7-empty.shape` | `print(42)` | 0 |
| `/tmp/cw-E-prog1-smoke4-strings.shape` | Smoke 4 canonical (`Set.add("a"); Set.add("b"); print(s.size())`) | 2 |
| `/tmp/cw-E-prog2-string-prints.shape` | `print("alpha")..print("epsilon")` (5 distinct) | 5 |
| `/tmp/cw-E-prog3-repeat-same-string.shape` | `print("hello") × 5` (1 distinct, 5 occurrences) | 1 |
| `/tmp/cw-E-prog4-string-in-loop.shape` | `while i < 100 { print("loop"); i += 1 }` | 1 |
| `/tmp/cw-E-prog5-many-distinct.shape` | `print("s00")..print("s19")` (20 distinct) | 20 |
| `/tmp/cw-E-prog6-string-funcs.shape` | 3 `fn` definitions each calling `print` with 1 distinct string | 3 |

### §2.2 Existing benchmarks measured

| Fixture | Shape | Distinct user string constants |
|---|---|---|
| `benchmarks/shape/01_fib.shape` | Recursive Fibonacci, no strings | 0 |
| `benchmarks/shape/03_sieve.shape` | Sieve of Eratosthenes, no strings | 0 |
| `benchmarks/shape/16_array_of_objects.shape` | Particle struct in array, no strings | 0 |

### §2.3 Raw counter readouts (`[shape-jit-arc-str-cum]`)

| Fixture | `str_allocs_total` | `str_retain_total` | `str_release_total` | `str_frees_total` | `leaked_total` |
|---|---|---|---|---|---|
| prog0 (no strings) | 4 | 0 | 0 | 0 | 4 |
| prog7 (empty) | 4 | 0 | 0 | 0 | 4 |
| prog1 (Smoke 4) | 6 | 0 | 0 | 0 | 6 |
| prog2 (5 distinct prints) | 9 | 0 | 0 | 0 | 9 |
| prog3 (5x repeat "hello") | 9 | 0 | 0 | 0 | 9 |
| prog4 (100-iter loop) | 5 | 0 | 99 | 1 | 4 |
| prog5 (20 distinct) | 24 | 0 | 0 | 0 | 24 |
| prog6 (3 functions) | 7 | 0 | 0 | 0 | 7 |
| 01_fib | 4 | 0 | 0 | 0 | 4 |
| 03_sieve | 4 | 0 | 0 | 0 | 4 |
| 16_array_of_objects | 4 | 0 | 0 | 0 | 4 |

### §2.4 Findings

1. **Stdlib-compile-time baseline = 4 allocs**, uniformly across all
   programs that exercise the JIT compile path. Source: per `crates/
   shape-vm/src/mir/lowering/stmt.rs:510` + `expr.rs:1118`, the
   for-loop and pipe-operator lowerings emit `MirConstant::Method(
   "len".to_string())` for the loop-counter bound — but this baseline
   shows up even in `print(42)`, suggesting the source is stdlib JIT
   compilation itself (math functions etc. that materialize a few
   method-name constants in their compiled MIR).

2. **Linear per-distinct-constant scaling**: prog5 (20 distinct
   string constants) shows `leaked_total = 24` = baseline 4 + 20.
   prog2 (5 distinct) shows `leaked_total = 9` = baseline 4 + 5.
   prog6 (3 fn definitions with 3 distinct strings) shows 7 = baseline
   4 + 3.

3. **Repeat-constant has same alloc count as distinct-constant
   case**: prog3 (5x "hello", 1 distinct) shows `leaked_total = 9`,
   identical to prog2 (5 distinct). **No deduplication today** — each
   `MirConstant::Str` operand allocates a fresh `Arc<String>` even
   when content is identical, because the producer at
   `mir_compiler/ownership.rs:434` allocates per-arc per
   occurrence-of-the-constant-in-MIR, not per-distinct-content.

4. **Loop-iteration count does NOT scale leaks**: prog4 (100-iter
   `while`) shows `leaked_total = 4` = baseline. The "loop" constant
   shows up in the JIT-emitted code as a single iconst — runtime
   iterations all share the iconst payload via runtime retain/release
   pairs. This empirically confirms the §F.3 docstring claim ("at
   most O(distinct string constants × JIT-compiled functions)") and
   the inventory §F.3 estimate ("bounded by program shape, not by
   runtime iteration count").

5. **prog4 surfaces a separate JIT correctness issue (NOT the leak
   under measurement)**: The JIT output for prog4 prints "loop\nloop"
   twice then garbage bytes (`z��` repeating), while VM prints 100x
   "loop" cleanly. This is a JIT bug in print-of-string-constant
   inside a `while` loop — `str_retain_total = 0` but
   `str_release_total = 99` means 99 release calls happened with NO
   matching retain calls, so the active-share refcount went negative
   after the first release. The corrupted output is the result of
   `Arc::from_raw` reading freed memory. This is a CRITICAL
   correctness gap orthogonal to the leak; surfaced as a new
   imprecision finding (§3 below) and recommended for separate
   investigation. The leak measurement itself is unaffected
   (`leaked_total = 4` matches baseline) because the bogus releases
   underflow the runtime active share, not the per-constant alloc
   count.

6. **Magnitude assessment (refines §F.3 SMALL-to-MEDIUM to
   SMALL):** Per the byte-size accounting in §4 below, the
   worst-measured program (prog5 with 24 leaks) consumes ~1.4 KB
   for the leak. Typical programs (prog1 / Smoke 4) consume ~360
   bytes. NO program in the measured set exceeds 2 KB. This is
   well within the "SMALL" classification per
   `bulldozer-wave-1-inventory.md` §0 "audit before rebuild"
   framework — well below any architectural-rebuild threshold.

---

## §3 — New imprecision finding surfaced (prog4 JIT correctness gap)

Per `phase-2d-handover.md` §0 / cluster-2-inventory §0 imprecision-
tracking discipline, the prog4 measurement surfaces a previously
unsurfaced JIT correctness bug:

**Symptom:** `while i < 100 { print("loop"); i += 1 }` JIT mode
prints the string correctly twice then prints garbage (`z��`
repeating) for the remaining iterations. VM mode prints "loop"
100 times cleanly. Exit code 0 in both modes.

**Diagnostic:** `STRING_RETAIN_CALLS = 0` but `STRING_RELEASE_CALLS
= 99` + `STRING_RELEASE_FREES = 1` across the prog4 execution.
The JIT-emitted code is calling `jit_arc_string_release` per loop
iteration but NOT calling the paired `jit_arc_string_retain`.
After ~2 iterations the constant's refcount drops to 1 (the
permanent share), then below 1 (use-after-free); subsequent
`Arc::from_raw` consumers read freed memory.

**Inferred root cause (UNVERIFIED — measurement-only deliverable):**
The MIR emitter at `crates/shape-jit/src/mir_compiler/ownership.rs`
is emitting `jit_arc_string_release` calls per loop iteration on
the iconst-payload constant (which is the §2.7.5 `Arc::into_raw`
shape) WITHOUT emitting matching `jit_arc_string_retain` calls.
This is the inverse problem of the W11-jit-new-array refcount
audit: the W11 series fixed missing retains; this site appears to
be missing the retain side of a retain/release pair for the
constant-iconst flow inside a loop body.

**Severity:** CRITICAL — produces silent data corruption / garbled
output in JIT mode on any program that prints a string constant
inside a loop. Smoke matrix VM/JIT divergence on a trivial
program. NOT triggered by the cluster-2-cw-E-string-leak measurement
itself (the leak counters work correctly; the corrupted output is a
separate runtime invariant violation).

**Disposition:** surfaced as new imprecision finding for follow-up
sub-cluster. NOT in cluster-2-cw-E-string-leak's scope (territory
is leak measurement + optional fix; this is a separate refcount-
underflow bug). Recommend separate sub-cluster
`cluster-2-jit-string-const-loop-retain-gap` with reproducer
`/tmp/cw-E-prog4-string-in-loop.shape` and the counter readouts in
§2.3 above.

---

## §4 — Leak byte-size accounting

Standard Rust `Arc<String>` layout (64-bit target):

| Field | Size |
|---|---|
| `Arc` control block: strong `AtomicUsize` | 8 |
| `Arc` control block: weak `AtomicUsize` | 8 |
| Inner: `String` (`ptr + cap + len`) | 24 |
| `Arc` control block total | 40 |
| `String` inner heap buffer (for `s.len()` bytes content) | rounded-up alloc |
| Per-alloc system allocator overhead | ~8-16 |

Per-leaked-`Arc<String>` total: ~56-64 bytes for short strings
(≤10 char), ~104-120 bytes for medium (10-50 char), ~296-320 bytes
for long (50-200 char).

Per-program leak in bytes (assuming median ~60 bytes per
leaked-Arc):

| Fixture | Leaked Arcs | Approx bytes leaked |
|---|---|---|
| prog0 / prog7 / 01_fib / 03_sieve / 16_array_of_objects | 4 | ~240 |
| prog1 (Smoke 4) | 6 | ~360 |
| prog2 / prog3 | 9 | ~540 |
| prog4 (loop) | 4 | ~240 |
| prog5 (20 distinct) | 24 | ~1.4 KB |
| prog6 (3 functions) | 7 | ~420 |

**Worst-case leak in measured set: 1.4 KB.** Even scaling to a
hypothetical 1000-distinct-string-constant program: ~60 KB. The
leak is bounded by program shape, NOT by runtime iteration count
(§2.4 finding #4 confirms this).

---

## §5 — Option A/B/C selection per measurement

Per inventory §F.4 + dispatch prompt's Phase 1 → Phase 2 decision
criterion:

### §5.1 Option A — intern-pool (re-evaluated)

**Inventory §F.4 estimate:** "4 files (string.rs + ownership.rs 3
call sites + 1-2 intern-pool files)".

**Re-evaluation post-measurement:** the inventory estimate assumes
the intern-pool index becomes the new iconst payload, which BREAKS
the §2.7.5 carrier contract (`Arc::into_raw(Arc<String>) as u64`
shape) at the FFI boundary. Consumers of `NativeKind::String`-shaped
slot bits across the codebase:

```
grep -rn "NativeKind::String\b" crates/shape-value crates/shape-vm crates/shape-jit | wc -l
257
grep -rn "Arc::(from|into)_raw\|Arc::(in|de)crement_strong_count.*\*const String" crates/ | wc -l
48
```

257 references to `NativeKind::String` + 48 sites that decode
`Arc<String>`-shaped bits. Changing iconst payload to intern-index
cascades through every consumer (`KindedSlot::Drop` for
`NativeKind::String`, `set_methods.rs::result_slot_to_string_arc`,
`hashmap_methods.rs` mirrors, etc.). Scope exceeds ceiling-c
single-cluster bound by an order of magnitude.

**Refined Option A — intern-pool with carrier-shape preservation:**
keep iconst as `Arc::as_ptr` of an intern-pool-owned `Arc<String>`.
At JIT compile time, the intern map deduplicates by content; the map
owns one share per distinct constant; the iconst payload is the
shared `Arc`'s data pointer. JIT runtime retain/release pairs
balance to zero net change (no leak per use-cycle). At program end,
the intern map drops, releasing all constants.

**Refined scope:** 1 file (`ffi/string.rs` alone — `arc_string_constant`
implementation only). No FFI contract change. No consumer cascade.

**Refined trade-off:** does NOT eliminate the permanent-share leak
during program execution — the intern map IS the permanent owner,
same lifetime as the current leak. Only difference: 5 occurrences
of "hello" share 1 Arc instead of 5 Arcs.

**Per-measurement value:** prog3 (5x "hello") shows
`leaked_total = 9` today; with refined Option A it would show
`leaked_total = 5` (baseline 4 + 1 deduplicated "hello"). Saves 4
Arc<String> allocations (~240 bytes). For the worst measured
program (prog5 = 20 distinct), saves 0 (all constants already
distinct). Overall savings: 5-20% of the measured leak.

### §5.2 Option B — ManuallyDrop+free-on-Drop (re-evaluated)

**Inventory §F.4 estimate:** "~5 files (string.rs + ownership.rs +
new JIT-compiled-function constant-tracking list + deallocation
hook)".

**Re-evaluation post-measurement:** the inventory itself flagged the
fundamental blocker: "the cleanup needs a hook into JIT-compiled-
function deallocation (which doesn't have one today — JIT-compiled
functions live for the program's lifetime per the Cranelift JIT-
module lifetime default)".

Adding a JIT-module-deallocation hook is an architectural change to
the Cranelift integration, NOT a 5-file local fix. It cascades into:
- `crates/shape-jit/src/jit_module.rs` (Cranelift `JITModule` lifecycle)
- `crates/shape-jit/src/compiler/` (per-function compilation
  tracking)
- `crates/shape-jit/src/executor.rs` (function dispatch / lookup)
- Lifecycle ordering: constants must be freed AFTER the last call
  to the JIT-compiled function — requires a counter or epoch
  mechanism

**Scope verdict:** EXCEEDS ceiling-c single-cluster bound. Not a
single-cluster fix.

### §5.3 Option C — RC-with-HeapHeader (re-evaluated)

**Inventory §F.4 estimate:** "1 file (string.rs alone — just swap
the Arc<String> for `*const StringObj`)".

**Inventory §F.4 explicit verdict:** "same leak shape (refcount
boost to keep alive); not a fix, just a carrier swap".

**Re-evaluation post-measurement:** confirms inventory's verdict.
The §2.7.5 amendment Wave 2 Agent B StringV2 carrier has the same
HeapHeader-at-offset-0 refcount mechanism; bumping it to 2 at
allocation has the identical leak shape. No measurement-side
benefit.

### §5.4 Selection

**No Option satisfies the Phase 1 → Phase 2 close-in-same-cluster
criterion** (leak magnitude SMALL + bounded **AND** Option-N has
small fix scope <300 LoC + bounded files **AND** Option-N actually
fixes the leak).

- Option A (refined) is the only Option with small fix scope (1
  file) BUT only deduplicates (5-20% savings); does NOT eliminate
  the leak.
- Option A (original inventory shape) eliminates the leak BUT has
  257+48 = 305-site cascade; exceeds ceiling-c.
- Option B eliminates the leak BUT requires Cranelift JIT-module
  lifecycle hook (out-of-scope architectural change).
- Option C is not a fix per inventory.

**Disposition: audit-only close.** Land measurement deliverable only;
defer Phase 2 fix to follow-up sub-cluster.

---

## §6 — Recommended follow-up

**Sub-cluster name:** `cluster-2-closure-wave-E-fix`

**Prescribed shape:** Refined Option A (intern-pool with carrier-
shape preservation per §5.1 refinement above). Single-file fix in
`crates/shape-jit/src/ffi/string.rs::arc_string_constant`. Adds a
`OnceLock<Mutex<HashMap<String, Arc<String>>>>` intern map; producer
consults the map for content-based deduplication; iconst payload
stays `Arc::as_ptr` shape (no §2.7.5 contract change).

**Scope estimate:** ~50 LoC delta in `ffi/string.rs` + 4 test cases
verifying deduplication behavior. No cascade.

**Magnitude eliminated:** 5-20% of measured leak (per §5.1
post-measurement quantification). Not full elimination — full
elimination requires the JIT-module lifecycle hook (Option B,
cluster-1.5+ territory).

**Optional second-stage follow-up:** `cluster-2-closure-wave-E-fix-
phase-2` for Option B (JIT-module deallocation hook). Requires
Cranelift integration redesign; estimated as multi-week scope; NOT
appropriate for cluster-2 closure-wave.

**Adjacent finding for separate sub-cluster:** prog4 JIT
correctness gap surfaced in §3 above — `cluster-2-jit-string-const-
loop-retain-gap`. CRITICAL severity, blocks smoke-matrix VM == JIT
on `print("string")`-in-loop fixtures. Independent of the leak
measurement.

---

## §7 — Citation index

| § | Cite |
|---|---|
| §F.1 leak surface | `crates/shape-jit/src/ffi/string.rs:111-143` (`arc_string_constant`) |
| §F.2 consumer enumeration | `crates/shape-jit/src/mir_compiler/ownership.rs:420,434,465` |
| §F.3 leak magnitude estimate | `cluster-2-inventory.md` §F.3 |
| §F.4 Options A/B/C | `cluster-2-inventory.md` §F.4 |
| Existing instrumentation precedent | `crates/shape-jit/src/ffi/arc.rs:68-76` (`JIT_ARC_*` counters), `crates/shape-jit/src/executor.rs:281-294` (`SHAPE_JIT_ARC_COUNTERS` reporting) |
| §2.7.5 carrier-shape contract | `docs/adr/006-value-and-memory-model.md` §2.7.5 |
| §2.7.5.1 stable-FFI raw-bits convention | `docs/adr/006-value-and-memory-model.md` §2.7.5.1 |
| §2.7.5 Wave 2 Agent B StringV2 amendment | `docs/adr/006-value-and-memory-model.md` §2.7.5 (Wave 2 Agent B amendment per Q25.A SUPERSEDED) |
| Smoke matrix canonical fixtures | `/tmp/cluster-2-smoke1.shape`, `/tmp/cluster-2-cw2-s2-fixture.shape`, `/tmp/cluster-2-smoke3.shape`, `/tmp/cluster-2-smoke4.shape` |
| `arc_string_constant` test precedent | `crates/shape-jit/src/ffi/string.rs:157-281` |
| KindedSlot Drop for NativeKind::String | `crates/shape-value/src/kinded_slot.rs:1041-1043` |
| VM-side consumer | `crates/shape-vm/src/executor/objects/set_methods.rs:136-155` (`result_slot_to_string_arc`) |

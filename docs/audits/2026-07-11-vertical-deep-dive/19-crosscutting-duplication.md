# Vertical Deep-Dive 19: Cross-cutting — Duplication, Split-brain & Dead Code

Auditor 19 of 19 · 2026-07-11 · territory: the whole `shape/` workspace (all `crates/`, `bin/`, `tools/`, `extensions/`) as a cross-crate sweep.
Working tree audited **as-is** (dirty, post-`ce332ca2`). All paths relative to `/home/dev/dev/shape-lang/shape/` unless noted.

Method: systematic greps + table extraction/diffing (scratchpad scripts), verification of prior audit leads against the working tree (docs/codebase-index/0*-dead-code-suspects.md are 2026-05-08 vintage and partially stale in BOTH directions), empirical `--mode vm` vs `--mode jit` runs with the prebuilt `target/debug/shape`, and execution of the project's own enforcement scripts (`scripts/check-no-dynamic.sh` — exit 0 on this tree).

## 0. Executive summary

### Overall health verdict

For a codebase that has been through nine documented waves of
delete-the-dynamic-path surgery, the cross-cutting hygiene is **better than
the priors suggested**: the notorious HeapKind dispatch-table fan-out (now
6 tables across 3 crates) is held in lockstep by *mechanical enforcement*
(`scripts/verify-merge.sh` CHECK 5/6/6b + `scripts/check-no-dynamic.sh` +
a JIT-vs-VM registry cross-check test), and this audit independently
re-verified all 36 `HeapKind` variants present in all four core tables plus
a consistent 13-explicit / 23-frozen-baseline split on the JIT side. The
forbidden-pattern gate passes on the dirty working tree
(`bash scripts/check-no-dynamic.sh` → `EXIT=0`, §2.1). The vmjit-diff
differential harness (469-program corpus, `tools/vmjit-diff/`) is genuinely
excellent engineering with a disciplined known-red allowlist that currently
pins only 2 divergences — each with a dated root-cause citation.

The debt is real but concentrated: (1) a **live, reproducible VM-vs-JIT
wrong-result divergence** (raw f64 bits printed as an int — reproduced in
this audit, §9.1) that is pinned-but-unfixed; (2) a **stale legacy
`HeapHeader` duplicate** (`crates/shape-value/src/heap_header.rs`) whose
`MAX_VARIANT = HashMap` makes `HeapKind::from_u16` reject 18 of 36 valid
variants — the exact bug the 2026-05-08 dead-code index flagged, "fixed"
once, and structurally re-regressed as the enum grew (§5.2, §9.2);
(3) **hand-duplicated struct-layout constants in the JIT** with no
compile-time cross-assert against the canonical `shape-value` definitions
(§4.4); (4) a triplicated `kind_type_name` table in shape-vm that has
**already measurably diverged** (§4.2); (5) the wire path silently
stringifying 8+ heap kinds into `"<hashset:phase-2c>"`-style placeholders
while the sibling snapshot path serializes the same kinds properly (§5.4);
and (6) a large doc-vs-code drift surface: CLAUDE.md documents an
11-variant `NativeKind` — the real enum has 30 variants; CLAUDE.md
documents `shape-common` as "Shared utilities across crates" — the crate
has no `src/` at all and is not a workspace member (§5.5, §8).

### Top-10 findings

| # | Sev | Finding | Evidence |
|---|-----|---------|----------|
| 1 | P1 | Live VM-vs-JIT wrong-result divergence: HOF return prints raw f64 bit pattern under JIT (`6.0` vs `4618441417868443648`), both exit 0. Known/pinned, routed to WF-3A, still shipping. | Reproduced this audit, transcript §9.1; pin at `tools/vmjit-diff/known-red.json` |
| 2 | P1 | Legacy `heap_header.rs` split-brain: `MAX_VARIANT = HashMap` (ordinal 17) while the real enum has 36 variants; `HeapKind::from_u16`/`HeapHeader::kind()` return `None` for 18 valid kinds; struct exported as `shape_value::HeapHeader` and read by production code. | `crates/shape-value/src/heap_header.rs:160,123`; §5.2, §9.2 |
| 3 | P1 | Wire serialization silently converts 8+ heap kinds into placeholder strings (`"<hashset:phase-2c>"`, `"<mutex:phase-2c>"` …) while the snapshot path serializes the same kinds correctly — split-brain between the two persistence paths. | `crates/shape-runtime/src/wire_conversion.rs:742,747,767,803` vs `snapshot.rs` SV arms; §5.4 |
| 4 | P1 | 23 of 36 HeapKinds ride a frozen "legacy HeapHeader fallback" in the JIT retain/release dispatch — the carrier-shape-confusion class that already produced 3 documented segfault families. Frozen + gate-checked, but latent UB one producer-site away. | `scripts/verify-merge.sh:248-271` baseline list; §5.3 |
| 5 | P2 | `kind_type_name` triplicated in shape-vm and **already diverged**: `HashMap→"map"` vs `"hashmap"`, `BigInt→"bigint"` vs `"int"`, `DataTable→"table"` vs `"datatable"`, integer widths collapsed to `"int"`. | `executor/arithmetic/mod.rs:742` vs `executor/objects/typed_access.rs:577`; diff in §4.2 |
| 6 | P2 | JIT hand-duplicates memory-layout constants (`STRING_OBJ_DATA_OFFSET=8`, `LEN_OFFSET=16`, header offsets) with zero compile-time cross-assert against `shape_value`'s canonical `OFFSET_*` constants sitting one dependency edge away. | `crates/shape-jit/src/mir_compiler/v2_string.rs:22-24`, `v2_array.rs:43-46`, `v2_field.rs:39-50`; §4.4 |
| 7 | P2 | `expected_kind_from_serializable` duplicated across crates (shape-runtime `pub` + shape-vm private copy), both ending in `_ => NativeKind::Bool` wildcards that will silently mis-kind any future `SerializableVMValue` variant. | `shape-runtime/src/snapshot.rs:3167` vs `shape-vm/src/executor/snapshot.rs:1086`; §4.1 |
| 8 | P2 | CLAUDE.md crate map documents `shape-common` as "Shared utilities across crates" — the crate contains **only a Cargo.toml**, no `src/`, not a workspace member. CLAUDE.md's `NativeKind` list names 11 variants; the real enum has 30. | `crates/shape-common/` listing; `crates/shape-value/src/native_kind.rs:32-`; §5.5, §8 |
| 9 | P2 | `emit_binary_op` is production-dead (`#[allow(dead_code)]`, only `#[cfg(test)]` callers) yet its doc-comment claims it is "the **sole** path through which the compiler emits any arithmetic / comparison opcode"; the dead `audit-dynamic-fallback` feature flag documents watching a fallback that no longer exists. | `compiler/helpers.rs:1692,1719`; `crates/shape-vm/Cargo.toml:82`; §3.4, §9.6 |
| 10 | P2 | Dead-crate skeletons + stale index: `shape-types` (only `data/`), `shape-common` (only Cargo.toml); `docs/codebase-index/00-dead-code-suspects.md` (2026-05-08) partially stale — 3 of 7 high-confidence suspects since fixed, 1 re-regressed, others unresolved. | §3.5, §8 |

### Scores

- **Feature completeness (of this vertical's subject — the anti-drift /
  anti-duplication infrastructure): 78/100.** The enforcement machinery
  exists and passes (verify-merge's 15 checks, check-no-dynamic, the
  `no_dynamic.rs` sentinel test, vmjit-diff, `registry_cross_check`), but
  coverage holes remain: no layout-constant cross-asserts, no lockstep check
  for the `kind_type_name`-class secondary tables, wire-vs-snapshot parity
  unchecked, and the dead-code index has no refresh cadence.
- **Code quality: 68/100.** Modern idiom, disciplined
  `NotImplemented(SURFACE: …)` error handling, unusually good SAFETY-comment
  coverage on `unsafe`; dragged down by 9,600-line files, ~3,300 `unsafe`
  occurrences across shape-vm/value/jit, 125 `#[allow(dead_code)]`
  attributes, triplicated hand-tables, and commit-log-as-comments
  archaeology that materially obscures load-bearing code.

### Biggest risk

The single biggest cross-cutting risk is the **frozen JIT retain/release
fallback for 23 of 36 HeapKinds** combined with **uncrosschecked layout
constants**. Both are instances of one failure shape: the JIT encodes
assumptions about shape-value memory layout as local literals or default
match arms, and nothing in the type system connects them to the canonical
definitions. This class already produced three documented segfault families
(W12 string-carrier, W15.2 closure-carrier, r5c-2-β-δ TypedArray-carrier —
named in `scripts/verify-merge.sh:225-228`), and the r5c-2-β-δ incident is
instructive: a dispatch arm was *removed* on the belief that no live slot
carried the kind, and that belief was empirically false
(`executor/vm_impl/stack.rs:262-281` carries the re-instatement note). The
verify-merge baseline freeze stops silent *growth* of the fallback set, but
the 23 existing kinds remain one new producer-site away from a
wrong-carrier refcount operation — which is UB, not an error return.

## 1. Architecture & code structure map (cross-cutting lens)

### 1.1 Workspace inventory with measured LOC

Measured with `find <crate>/src -name '*.rs' | xargs wc -l` on the working
tree (2026-07-11). Total workspace Rust: **~712k LOC** across ~920 files.

| Crate / dir | LOC | Files | Cross-cutting role |
|---|---:|---:|---|
| `crates/shape-vm` | 263,881 | 294 | Interpreter + bytecode compiler; hosts 2 of the 4 core HeapKind lockstep tables (`executor/vm_impl/stack.rs`) and the biggest files in the repo |
| `crates/shape-runtime` | 121,300 | 233 | Type system, stdlib, snapshot + wire + JSON marshal family (4 of the 8 conversion tables) |
| `crates/shape-jit` | 68,335 | 126 | Cranelift lowering; tables 5/6 of the HeapKind lockstep (`mir_compiler/ownership.rs`, `ffi/v2/collection_arc.rs`); duplicated layout constants |
| `tools/shape-lsp` | 49,420 | 51 | LSP; carries its own 2,944-line type-inference layer (`src/type_inference.rs`) parallel to shape-runtime's |
| `crates/shape-ast` | 32,629 | 93 | Grammar + AST; near-zero unsafe, near-zero debt markers |
| `crates/shape-value` | 31,555 | 40 | Canonical value model: `heap_variants.rs` (`define_heap_types!` macro — single source of truth for the enum), `kinded_slot.rs`, `v2/` raw carriers, `gc.rs` |
| `bin/shape-cli` | 14,819 | 42 | CLI; owns the two-tier `gc = ["shape-vm/gc", "shape-jit?/gc"]` feature join |
| `crates/shape-wire` | 4,061 | 16 | MessagePack + QUIC transport |
| `crates/shape-abi-v1` | 3,263 | 3 | Stable C ABI, Permission enum |
| `tools/shape-fuzz` | 1,821 | 5 | Fuzz harness |
| `tools/shape-test` | 1,697 (+347 test files) | 3 | Integration harness; `tests/` holds 7,260 `#[test]` fns in 347 files |
| `extensions/python` | 1,392 | 5 | PyO3 runtime; structurally parallel to typescript |
| `tools/xtask` | 1,296 | 1 | Workspace automation |
| `extensions/typescript` | 952 | 4 | deno_core runtime |
| `crates/shape-diagnostics` | 729 | 4 | LSDS diagnostics (absent from CLAUDE.md crate map) |
| `crates/shape-macros` | 678 | 1 | Proc macros |
| `crates/shape-viz/*` | 8,098 | ~20 | Split into `shape-viz-core` (all the code) + `shape-viz-native` (a `main.rs` shell) — split is real, no remnant duplication found |
| `crates/shape-types` | **0** | 0 | Empty skeleton, only `data/ES.1m.mktd`; documented as reserved (CLAUDE.md crate map) — accurate |
| `crates/shape-common` | **0** | 0 | **Only a `Cargo.toml`** (with real dependency declarations!); NOT in `[workspace] members` (`Cargo.toml:2-21`); CLAUDE.md documents it as "Shared utilities across crates" — inaccurate |
| `tools/vmjit-diff` | 0 Rust (Node.js `.mjs`) | — | VM-vs-JIT differential harness; not a cargo member; 469-program corpus |

### 1.2 The dispatch-table topology (the load-bearing duplication surface)

The single `HeapKind` enum (36 variants, `crates/shape-value/src/heap_variants.rs:63-101`,
generated through the `define_heap_types!` macro at `heap_variants.rs:37`)
fans out into per-kind `match` tables that must stay in lockstep:

| # | Table | File | `HeapKind::` refs | Enforced by |
|---|---|---|---:|---|
| 1 | VM stack clone/drop | `crates/shape-vm/src/executor/vm_impl/stack.rs:229,627` | 126 | verify-merge CHECK 6 |
| 2 | `KindedSlot::Clone`/`Drop` | `crates/shape-value/src/kinded_slot.rs:947,1415` | 172 | verify-merge CHECK 6 |
| 3 | `SharedCell::drop` | `crates/shape-value/src/v2/closure_layout.rs` | 97 | verify-merge CHECK 6 |
| 4 | `TypedObjectStorage::drop` | `crates/shape-value/src/heap_value.rs` | 89 | verify-merge CHECK 6 |
| 5 | JIT retain/release dispatch | `crates/shape-jit/src/mir_compiler/ownership.rs:678,728` | 13 explicit kinds | verify-merge CHECK 6b |
| 6 | JIT per-kind FFI bodies | `crates/shape-jit/src/ffi/v2/collection_arc.rs` | retain/release pairs | verify-merge CHECK 6b pairing scan |
| — | GC cycle-capable subset | `crates/shape-value/src/gc.rs:344` | 17 kinds (intentional subset) | none (design-doc bound) |

Beyond these, per-kind knowledge appears in at least 8 *conversion* tables
(§4.3) and 4 *type-name* tables (§4.2) that are NOT under any lockstep
check.

### 1.3 Conversion/marshal layer map

Eight files, ~18,140 LOC total (measured), each holding an independent
per-`HeapValue`/`HeapKind` or per-`NativeKind` conversion `match`:

| File | LOC | Direction |
|---|---:|---|
| `crates/shape-runtime/src/snapshot.rs` | 6,133 | slots ⇄ `SerializableVMValue` (v7, identity-mapped) |
| `crates/shape-runtime/src/marshal.rs` | 2,904 | polyglot/FFI value marshal |
| `crates/shape-vm/src/executor/snapshot.rs` | 2,428 | VM-side snapshot glue (duplicates helpers from runtime snapshot, §4.1) |
| `crates/shape-jit/src/ffi/conversion.rs` | 1,902 | JIT boundary conversions |
| `crates/shape-runtime/src/wire_conversion.rs` | 1,674 | heap ⇄ `WireValue` (with phase-2c placeholder-string arms, §5.4) |
| `crates/shape-runtime/src/json_value.rs` | 1,406 | heap ⇄ JSON |
| `crates/shape-vm/src/executor/control_flow/foreign_marshal.rs` | 1,321 | extern-C in/out marshal |
| `crates/shape-vm/src/executor/vm_state_snapshot.rs` | 372 | whole-VM state capture |

### 1.4 Entry points relevant to this vertical

- Enforcement: `scripts/verify-merge.sh` (15 checks; CHECK 5 ordinal
  collisions, CHECK 6 4-table lockstep, CHECK 6b JIT tables 5/6, CHECK 8
  dispatch-table missing-brace), `scripts/check-no-dynamic.sh`
  (forbidden-symbol grep gate), sentinel test
  `crates/shape-vm/src/executor/tests/no_dynamic.rs`.
- Differential: `tools/vmjit-diff/run-diff.mjs` + `known-red.json`;
  `registry_cross_check` test module in
  `crates/shape-jit/src/mir_compiler/types.rs:3799-3880` (iterates the VM
  PHF method registry and cross-checks JIT return-kind tables — with its own
  pinned-drift list that "must only shrink").
- Dead-code ledger: `docs/codebase-index/00-dead-code-suspects.md`
  (245 lines, 29 suspects, dated 2026-05-08).

## 2. Feature completeness (anti-drift infrastructure): implemented / partial / stubbed

For a cross-cutting vertical, "the feature" is the machinery that keeps
duplicated surfaces honest. Distinguishing CODE EXISTS from WORKS:

### 2.1 Forbidden-pattern gate — WORKS (run this audit)

```
$ bash scripts/check-no-dynamic.sh; echo "EXIT=$?"
EXIT=0
```

Run against the dirty working tree 2026-07-11. The companion sentinel test
`crates/shape-vm/src/executor/tests/no_dynamic.rs` exists as documented in
CLAUDE.md §Mechanical enforcement. Grep for live forbidden symbols
(`synthesize_value_word_from_raw`, `exec_*_dynamic_fallback`,
`SlotKind::Dynamic`, generic `AddDynamic`-family opcodes) came back clean:
the `Dynamic` opcode family exists only as tombstone comments
(`crates/shape-vm/src/bytecode/opcode_defs.rs:87-105` — "DELETED -
strict-typing sweep Phase 2", with the ordinal range 0x10-0x16/0x20-0x25
explicitly reserved-not-reused at `opcode_defs.rs:598`). **No live
forbidden-pattern code was found anywhere in the workspace** — the P0 class
this audit was primed for is absent.

### 2.2 4-table HeapKind lockstep — WORKS (independently re-verified)

I extracted the `HeapKind::` variant set from each of the four core tables
and diffed against the enum: **all 36 variants present in all 4 tables**
(kinded_slot.rs, vm_impl/stack.rs, closure_layout.rs, heap_value.rs —
identical 36-element sets). `gc.rs` covers a 17-kind subset, which is the
intentional cycle-capable set per `gc.rs:329-344`
(`cycle_capable_direct_header`), not drift.

### 2.3 JIT tables 5/6 — WORKS but 23/36 kinds are a frozen fallback

`ownership.rs` has explicit retain+release arms for exactly 13 kinds
(Atomic, Channel, Closure, Deque, HashMap, HashSet, Lazy, Mutex, Option,
PriorityQueue, Result, TypedArray, TypedObject — extracted by grep). The
other 23 are the `jit_lockstep_baseline` frozen list in
`scripts/verify-merge.sh:248-271`, riding a legacy `arc_retain`/`arc_release`
HeapHeader fallback. The check enforces baseline-may-only-shrink and
retain/release pairing plus FFI-symbol resolution. 13 + 23 = 36 ✓. Status:
mechanically consistent, semantically risky (§5.3).

### 2.4 VM-vs-JIT differential harness — WORKS

469 corpus programs (`ls tools/vmjit-diff/corpus | wc -l` → 469), three
tiers (book fences / acceptance programs / synthetic repros), a known-red
allowlist with exactly 2 pinned entries as of the 2026-07-05 baseline, and
an explicit anti-rot rule: "a MATCH on a listed id is flagged for removal"
(`known-red.json` description field). The harness closed 4 divergence
classes in WF-1A and the pins carry dated reclassification citations. The
1 remaining deterministic pin still reproduces (§9.1); the other is a
proven pre-existing nondeterministic flake (comptime FrameDescriptor gap).

### 2.5 Registry cross-check — WORKS, with 3 pinned soundness drifts

`crates/shape-jit/src/mir_compiler/types.rs:3799-3880` iterates shape-vm's
PHF method-registry maps and cross-checks the JIT return-kind tables
against real VM handlers, with a pinned-drift list that fails the build if
a pin stops reproducing ("pinned soundness bugs — list must only shrink",
`types.rs:3877`). Known pins include the `Array<int>.mean()` drift
(`types.rs:1263-1268`: VM `v2_int_avg` returns Float64 while the JIT read
expected otherwise) and the `HashMap.iter` carrier mismatch
(`types.rs:3860`). These are real, acknowledged VM/JIT semantic gaps held
visible by the test rather than fixed — partial credit.

### 2.6 Dead-code ledger — STALE (no refresh since 2026-05-08)

`docs/codebase-index/00-dead-code-suspects.md` verification against the
working tree (§3.5): of the 7 high-confidence suspects, `ffi/window.rs`
tombstone **deleted** ✓, `JITSignalBuilder` **deleted** ✓ (zero grep hits in
`crates/shape-jit/src/context.rs`, file gone), `MAX_VARIANT` **"fixed" then
re-regressed** ✗ (§9.2), `comptime_concrete` module-scope
`#![allow(dead_code)]` **still present** (`compiler/comptime_concrete.rs:77`),
`ValueSlot::from_heap` still has 4 non-test caller lines (transitional, as
documented), `MirConstant::StringId(u32)` still present
(`mir/types.rs:205`). The ledger has no date-of-last-verification field and
`docs/codebase-index.md` still points to it as current.

### 2.7 Not implemented at all (gaps in the anti-drift machinery)

- **No layout-constant cross-asserts** JIT↔value (§4.4) — despite
  `shape-jit/src/context.rs:150-158` proving the team knows the
  `const _: () = assert!(offset_of!(...))` idiom and uses it for JITContext.
- **No lockstep/consistency check for the secondary per-kind tables**
  (`kind_type_name` ×3, `format_heap_kind`, JIT `receiver_type_name`) —
  and §4.2 shows one has already diverged.
- **No wire-vs-snapshot serialization parity check** (§5.4).
- **No CI dead-code sweep** (`cargo +nightly udeps` or equivalent);
  `#[allow(dead_code)]` count is unbudgeted (125 occurrences workspace-wide, §3.3).

## 3. Code quality (workspace-wide lens)

### 3.1 Unsafe census

Measured with `grep -rE "unsafe (fn|\{|impl)"` per crate on the working tree
(counts are *sites*, i.e. lines declaring an unsafe fn/block/impl, not the
`unsafe` keyword count, which is higher):

| Crate | unsafe sites | SAFETY comments | Ratio | Note |
|---|---:|---:|---|---|
| `crates/shape-vm` | 1,271 | 269 | ~0.21 | Raw slot bits ⇄ `Arc` pointers on every hot path |
| `crates/shape-value` | 835 | 197 | ~0.24 | v2 raw carriers, refcount dispatch tables |
| `crates/shape-jit` | 724 | 77 | ~0.11 | `extern "C"` FFI surface; weakest SAFETY-comment coverage |
| `crates/shape-runtime` | 356 | 65 | ~0.18 | SIMD, plugin loader, event queue |
| `crates/shape-abi-v1` | 31 | — | — | C ABI by definition |
| `extensions/python` | 21 | — | — | PyO3 boundary |
| `extensions/typescript` | 19 | — | — | deno_core boundary |
| `bin/shape-cli` | 5 | — | — | |
| `tools/shape-test` | 2 | — | — | |
| shape-ast / shape-wire / shape-macros / shape-lsp / shape-common | **0** | — | — | Parser, wire codec, LSP are fully safe Rust |

Total: **~3,264 unsafe sites**, essentially all concentrated in the four
value-model crates. The concentration itself is the right design — the
parser, LSP, and wire codec being 0-unsafe is a genuine achievement at this
scale. Many "sites" are single-expression accessor wrappers
(`unsafe { (*arr_u8).len }`) rather than open-coded pointer arithmetic.

**Sample audit of the scariest sites** (read in full this audit):

1. **`crates/shape-value/src/heap_header.rs:164` — `from_u16` transmute
   guarded by a hand-maintained constant.**
   `unsafe { std::mem::transmute(v as u8) }` guarded by
   `v <= Self::MAX_VARIANT as u16` where `MAX_VARIANT = HeapKind::HashMap`
   (line 160) — ordinal 17 of a 36-variant enum. Today the guard is *too
   small*, so the function is safe-but-wrong (returns `None` for 18 valid
   kinds, §5.2/§9.2). But the failure geometry is nasty: the comment says
   "IMPORTANT: Update this when adding new HeapKind variants" — a future
   "fix" that sets `MAX_VARIANT` past the true last ordinal makes the
   transmute fabricate out-of-range enum values, which is instant UB. A
   guard constant that the type system cannot check, sitting next to a
   transmute, in a file that has already drifted once, is the single
   scariest unsafe pattern found in this sweep.
2. **`crates/shape-vm/src/executor/objects/array_basic.rs:392`** —
   `unsafe { (*(p as *const shape_value::HeapHeader)).kind }`: header
   deref through the **legacy** `HeapHeader` type on a fresh **v2**
   TypedArray allocation (it is itself a `debug_assert` against
   `HEAP_KIND_V2_TYPED_ARRAY`, array_basic.rs:389-395). Low direct risk —
   but it demonstrates the two same-named header structs (§9.10) being
   used interchangeably, sound only while their layouts happen to
   coincide.
3. **`crates/shape-jit/src/ffi/v2/mod.rs:579` —
   `pub extern "C" fn jit_v2_array_push(arr: *mut HeapHeader, bits: u64, elem_size: u8)`.**
   Callable from JIT-generated machine code; `elem_size` is trusted
   blindly. A codegen bug that passes elem_size 8 for a 1-byte element
   array is a heap overflow with no runtime check. The compile-time
   contract is documented, but this is the class of function where the
   verify-merge FFI-symbol-resolution check (CHECK 6b) is the only net.
4. **`crates/shape-jit/src/ffi/call_method/mod.rs:115` —
   `unsafe fn receiver_type_name`** reads the heap-allocation kind prefix
   at offset 0 for `UInt64`-carrier opaque-bits receivers. The doc-comment
   (lines 108–114) explicitly argues why this is not tag-bit dispatch —
   good — but the function also embeds a fourth independent
   kind→type-name vocabulary (§4.2).
5. **`crates/shape-vm/src/executor/v2_handlers/v2_array_detect.rs:363`** —
   header read that *is* defensive: checks `kind` metadata first, then
   null, then `header.kind != HEAP_KIND_V2_TYPED_ARRAY`, then validates
   the elem-type byte. This is the pattern sites 2–3 should follow.
6. **`crates/shape-value/src/kinded_slot.rs:947,1415`** — the
   `Clone`/`Drop` dispatch tables call
   `Arc::increment_strong_count::<T>`/`decrement_strong_count::<T>` with a
   per-arm concrete `T`. A single wrong `T` in one arm is a wrong-type
   refcount op on a live allocation (the exact class of the FilterExpr
   incident memorialized at `heap_variants.rs:103-110`). 172 `HeapKind::`
   references in this one file; guarded by verify-merge CHECK 6.
7. **`crates/shape-jit/src/ffi/object/closure.rs:1149-1150`** —
   `ptr::write(ptr as *mut HeapHeader, HeapHeader::new(HEAP_KIND_V2_CLOSURE))`
   stamping a header into a fresh allocation; layout agreement with
   shape-value is asserted only by the *test* at
   `mir_compiler/statements.rs:1744-1751` (`HEAP_KIND_V2_CLOSURE == 84`,
   offsets 0/4/6), not a `const` assert.
8. **`crates/shape-value/src/v2/closure_raw.rs:636`** — capture-cell
   pointer arithmetic via `SHARED_CELL_VALUE_OFFSET`; this one IS tied to
   the struct definition by a const-assert (`closure_layout.rs:205`) —
   the positive example the rest of the JIT constants should copy (§4.4).

Verdict: unsafe usage is systemically *documented* (SAFETY comments are
the norm in shape-value/vm) but *unevenly verified* — shape-jit has the
lowest comment ratio and the most layout assumptions, and the mechanical
nets (CHECK 6/6b, the statements.rs layout test) cover the dispatch tables
but not the constant duplication (§4.4).

### 3.2 Complexity hotspots

Largest files (`find … | xargs wc -l | sort -rn`), all in the compiler/value
layers:

| File | LOC |
|---|---:|
| `crates/shape-vm/src/compiler/statements.rs` | 9,663 |
| `crates/shape-vm/src/compiler/helpers.rs` | 9,058 |
| `crates/shape-vm/src/compiler/expressions/function_calls.rs` | 9,052 |
| `crates/shape-value/src/heap_value.rs` | 8,142 |
| `crates/shape-vm/src/compiler/functions.rs` | 6,579 |
| `crates/shape-runtime/src/snapshot.rs` | 6,133 |
| `crates/shape-vm/src/compiler/monomorphization/type_resolution.rs` | 5,442 |
| `crates/shape-runtime/src/type_system/inference/inference_tests.rs` | 5,409 |
| `crates/shape-vm/src/executor/v2_handlers/v2_array_detect.rs` | 5,355 |
| `crates/shape-runtime/src/type_system/inference/items.rs` | 4,903 |
| `crates/shape-vm/src/compiler/expressions/closures.rs` | 4,856 |
| `crates/shape-jit/src/mir_compiler/types.rs` | 4,785 |
| `crates/shape-vm/src/compiler/expressions/binary_ops.rs` | 4,659 |
| `crates/shape-runtime/src/type_system/inference/expressions.rs` | 4,495 |

Three of the top four are the bytecode compiler — 27.7k LOC in three files.
These files mix emission, inference glue, and per-op special cases; the
"Exhaustive Match Rule" in CLAUDE.md (8+ files per new AST variant) is a
direct consequence of this monolith shape. Nothing here is broken, but
every cross-cutting change lands in a 9k-line file, which is where the
comment-archaeology problem (§3.6) hurts most.

### 3.3 Dead-code annotations

- **125 `allow(dead_code)` occurrences** workspace-wide (grep count,
  includes 7 module-level `#![allow(dead_code)]`).
- The module-level ones are the honest, documented kind — a distinct
  "Intentional-future" convention:
  - `crates/shape-jit/src/optimizer/bounds.rs:8` and
    `numeric_arrays.rs:11`: "part of the JIT optimization-planning
    subsystem … built but not yet consumed by the live lowering path".
    A whole optimizer planning layer compiled but never called.
  - `crates/shape-vm/src/executor/ic_fast_paths.rs:15`: "scaffolding
    awaiting the MethodFnV2 IC wiring tracked in
    V2_METHOD_DISPATCH_AUDIT.md. The property IC fast paths are already
    live … Allow dead_code module-wide for the un-wired halves rather than
    churning per-item annotations."
  - `crates/shape-vm/src/compiler/comptime_concrete.rs:77` — flagged by
    the 2026-05-08 dead-code ledger, still present.
  - Plus 3 test-support modules (fine).
- The convention is defensible (a dated pointer to the wiring plan beats
  bare `#[allow]`), but there is no budget or expiry: `ic_fast_paths.rs`
  and the optimizer planners have been "awaiting wiring" across multiple
  waves with no consuming code in the working tree.

### 3.4 Stale doc on load-bearing-looking dead code

`crates/shape-vm/src/compiler/helpers.rs:1692-1718` — `emit_binary_op` is
`#[allow(dead_code)]` (line 1718) and its only callers are in the
`#[cfg(test)]` region of the same file (helpers.rs:7425 onward — verified
by grep; the sole non-test-file mention is a doc-comment in
`binary_ops.rs:217`). Yet its doc-comment says:

> "This function is now the **sole** path through which the compiler emits
> any arithmetic / comparison opcode — typed or Dynamic. … Every `*Dynamic`
> opcode this shim still emits is reserved for …" (helpers.rs:1691-1704)

Both halves are false on this tree: (a) it is not the sole path — it is
not a production path at all; (b) there are no `*Dynamic` opcodes to emit —
the family is deleted with tombstones at
`crates/shape-vm/src/bytecode/opcode_defs.rs:87-105` and the function body
itself carries the strict-typing-sweep note. The companion Cargo feature
`audit-dynamic-fallback` (`crates/shape-vm/Cargo.toml:78-83`) still
documents itself as "print a stderr line every time `emit_binary_op` falls
back to a `*Dynamic` opcode" — a feature flag watching for a fallback that
cannot occur. This is not a forbidden-pattern violation (no live dynamic
code), but it is exactly the kind of stale authority that misleads the
next agent into believing a dynamic path exists. See §9.6.

### 3.5 Dead code census

- **`crates/shape-types`**: no `src/`, only `data/ES.1m.mktd`. Matches the
  CLAUDE.md description ("Empty crate skeleton … reserved for a planned
  move") — accurately documented dead weight.
- **`crates/shape-common`**: only a `Cargo.toml` (verified by `ls`), with
  real dependency declarations inside, and NOT in `[workspace] members`
  (root `Cargo.toml:2-21` — 18 members, shape-common absent). Unlike
  shape-types, CLAUDE.md documents this as a live crate ("Shared utilities
  across crates") — doc-vs-reality drift, §8.
- **`crates/shape-viz` split**: both halves (`shape-viz-core`,
  `shape-viz-native`) are workspace members; no orphaned pre-split module
  found under `crates/shape-viz/` besides the two member dirs. The split
  left no remnant duplication.
- **Retired `c"..."` content-string syntax**: no `c_string`/content-string
  rule survives in `crates/shape-ast/src/shape.pest` (grep clean). The
  W18.3 retirement was completed in the grammar.
- **`unimplemented!`/`todo!` inventory**: shape-vm 96, shape-jit 47,
  shape-runtime 3, shape-value 1, others 0. In shape-vm, 59 of 96 sit in
  two test files (`executor/tests/iterator_ops.rs` 31,
  `executor/tests/mod.rs` 28 — deliberately-failing kinded-ABI test
  scaffolds). Live production `todo!()` bodies: 5 —
  `executor/vm_impl/program.rs:506` (`module_bindings_snapshot`),
  `program.rs:623` (`push_value`), `executor/printing.rs:430` (Closure
  formatting), `printing.rs:447` (NativeScalar formatting),
  `executor/call_convention.rs:374` (suspension shape). All five carry
  dated ADR-006 §2.7.4 phase-2c deferral notes and are "fail loudly"
  by design; none panicked in the empirical probes run this audit
  (`print(closure)` takes a different path — and exposes a different bug,
  §9.7).
- **`NotImplemented` / SURFACE discipline**: 226 `NotImplemented` + 311
  `SURFACE` mentions in shape-vm, 214 SURFACE in shape-jit. This is the
  sanctioned surface-and-stop idiom (CLAUDE.md §2.7.8: "surface-and-stop
  with `NotImplemented(SURFACE)` instead") — counted here as *tracked
  incompleteness*, not dead code.
- **Ledger verification** (`docs/codebase-index/00-dead-code-suspects.md`,
  2026-05-08): see §2.6 — 3 of 7 high-confidence suspects fixed, 1
  re-regressed (`MAX_VARIANT`), 3 unresolved-but-documented. The ledger
  has had no refresh in 9 weeks of heavy churn.

### 3.6 Idiom, naming, error handling

- Edition 2024 across the workspace (`Cargo.toml:28`), `Result`-based
  error handling throughout, zero `unwrap()`-panic culture on the paths
  read (errors thread `VMError` with structured variants).
- Naming is disciplined and grep-friendly: the `jit_v2_*` FFI prefix, the
  `SV::` serialization arms, `HEAP_KIND_V2_*` constants.
- The workspace's most distinctive quality problem is
  **commit-log-as-comments**: load-bearing files are strewn with wave
  citations ("Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14)",
  "R5b-2-bool-null-sentinel-cluster", "Wave-γ G-heap-filter-expr") — see
  any 20 lines of `kind_type_name` (§4.2) or the 40-line ordinal-vacation
  comment inside the `HeapKind` enum (`heap_variants.rs:71-92`). Each
  individual note is defensible provenance; in aggregate they roughly
  double the visual size of dispatch tables, and — measurably — the
  *comments* have been copy-pasted along with the code and then diverged
  independently of it (the three `kind_type_name` copies carry three
  different abbreviations of the same W12 note). Provenance belongs in
  `git blame`; invariants belong in comments.

## 4. Duplication & DRY violations

### 4.1 `expected_kind_from_serializable` — duplicated across crates, ALREADY diverged

- Canonical: `crates/shape-runtime/src/snapshot.rs:3167`
  (`pub fn expected_kind_from_serializable`), used by runtime restore and
  re-exported through `context/mod.rs:567`.
- Copy: `crates/shape-vm/src/executor/snapshot.rs:1086` (private
  `fn expected_kind_from_serializable`), used by the VM-side restore at
  `executor/snapshot.rs:366,384`.

Diffed both this audit (full arm-by-arm). **They have diverged**: the
runtime copy has `SV::ModuleFunction(_) => NativeKind::Ptr(HeapKind::ModuleFn)`
(snapshot.rs:3194); the VM copy has no such arm, so a `ModuleFunction`
serialized value reaching the VM copy falls through
`_ => NativeKind::Bool` (executor/snapshot.rs:1128). Both copies end in
the same `_ => NativeKind::Bool` wildcard, meaning any future
`SerializableVMValue` variant added to one match silently mis-kinds in the
other with no compile error — the exact failure mode exhaustive matches
exist to prevent. Mitigation that keeps this at P2 rather than P1: the VM
copy's doc-comment (executor/snapshot.rs:1084-1085) notes that
`serializable_to_slot` "surfaces a structured kind-mismatch error" when the
Bool guess is wrong — so the drift manifests as a spurious restore *error*,
not a mis-typed live slot. Still: two hand-synced 30-arm matches, one
`pub` and importable by the other crate, is a pure DRY failure — shape-vm
already imports the runtime's `serializable_to_kinded_slot` family
(`context/mod.rs:567`), so the private copy has no reason to exist.

### 4.2 `kind_type_name` — THREE copies in one crate, plus a fourth vocabulary in the JIT

Three private functions with the same name and purpose inside shape-vm:

| Copy | Location | Vocabulary sample |
|---|---|---|
| A | `executor/arithmetic/mod.rs:742` | `Int8→"i8"`, `Int64→"int"`, `UInt32→"u32"`, `HashMap→"map"`, `BigInt→"bigint"`, `DataTable→"table"` |
| B | `executor/objects/typed_access.rs:577` | ALL 20 integer kinds collapsed → `"int"`, `HashMap→"hashmap"`, `BigInt→"int"`, `DataTable→"datatable"` |
| C | `executor/comparison/mod.rs:701` | Same mapping as A (diff shows only comment-wording differences + comment-detail drift on the FilterExpr/Reference arms) |

And the JIT's `receiver_type_name`
(`crates/shape-jit/src/ffi/call_method/mod.rs:115`) is a **fourth**
vocabulary: every integer width — `Int8` through `NullableUIntSize` —
maps to `Some("number".to_string())` (lines 124-144), which contradicts
all three VM copies AND the language's own type lattice (CLAUDE.md: "`int`
and `number` are separate. They don't unify."). Consequences:

- The same wrong-typed operation produces a different type name in its
  error message depending on which executor module caught it
  (`"map"` vs `"hashmap"`, `"bigint"` vs `"int"`), and under JIT an int
  receiver is described as `"number"` — actively misleading in a language
  whose central discipline is that int and number never unify.
- Divergence is not hypothetical; it is present in the working tree today
  (diff output captured in scratchpad `ktn_*.txt`).
- None of the four tables is under any lockstep check (verify-merge
  CHECK 6/6b covers the *refcount* tables only).

Copies A and C are ~60 lines each and byte-similar modulo comments — a
single `pub(crate) fn` in one module (or a method on `NativeKind` in
shape-value, where `HeapKind` already lives) removes the entire class.

### 4.3 The conversion-table family — 8 independent per-kind matches, ~18k LOC

Every serialization/marshal boundary re-enumerates the value model:

| File | LOC | Per-kind match over |
|---|---:|---|
| `crates/shape-runtime/src/snapshot.rs` | 6,133 | `HeapValue`/slot ⇄ `SerializableVMValue` |
| `crates/shape-runtime/src/marshal.rs` | 2,904 | polyglot boundary values |
| `crates/shape-vm/src/executor/snapshot.rs` | 2,428 | VM restore glue (incl. the §4.1 copy) |
| `crates/shape-jit/src/ffi/conversion.rs` | 1,902 | JIT boundary |
| `crates/shape-runtime/src/wire_conversion.rs` | 1,674 | heap ⇄ `WireValue` |
| `crates/shape-runtime/src/json_value.rs` | 1,406 | heap ⇄ JSON |
| `crates/shape-vm/src/executor/control_flow/foreign_marshal.rs` | 1,321 | extern-C in/out |
| `crates/shape-vm/src/executor/vm_state_snapshot.rs` | 372 | whole-VM capture |

This fan-out is partially inherent (each target format genuinely differs),
but the *coverage* discipline differs wildly per file: snapshot.rs handles
HashSet/Deque/Mutex/Channel properly while wire_conversion.rs stringifies
them into placeholders (§5.4), and json_value.rs makes a third set of
choices. There is no cross-table conformance test asserting "every
HeapKind either round-trips or returns a structured error in each of the
8 boundaries". Adding a HeapKind today requires visiting up to 8 files
with only 2 of them (the refcount tables' siblings) gate-checked.

### 4.4 JIT-local layout constants — 5 modules re-declare shape-value's canonical offsets

Canonical definitions (with the struct they describe, in shape-value):

- `crates/shape-value/src/v2/heap_header.rs:230-232` —
  `OFFSET_REFCOUNT: usize = 0`, `OFFSET_KIND: usize = 4`,
  `OFFSET_FLAGS: usize = 6`.
- `crates/shape-value/src/v2/string_obj.rs:102-103` —
  `OFFSET_DATA: usize = 8`, `OFFSET_LEN: usize = 16`.

Hand-duplicates inside shape-jit (all verified this audit):

| JIT file | Constant(s) | Shadowing |
|---|---|---|
| `mir_compiler/v2_string.rs:22-24` | `STRING_OBJ_DATA_OFFSET: i32 = 8`, `STRING_OBJ_LEN_OFFSET: i32 = 16` | `StringObj::OFFSET_DATA/OFFSET_LEN` |
| `mir_compiler/v2_array.rs:43-46` | `DATA_PTR_OFFSET: i32 = 8`, `LEN_OFFSET: i32 = 16` | `TypedArray` header layout |
| `mir_compiler/v2_field.rs:39-45` | `V2_HEADER_REFCOUNT_OFFSET/KIND/FLAGS: u32 = 0/4/6` | `HeapHeader::OFFSET_*` |
| `mir_compiler/v2_refcount.rs:48` | `V2_REFCOUNT_OFFSET: i32 = 0` | `HeapHeader::OFFSET_REFCOUNT` — a *second* JIT-internal copy |
| `mir_compiler/places.rs:27` | `TYPED_OBJ_SLOT_DATA_OFFSET` | TypedObjectStorage slot base |

So the refcount offset alone exists in at least three declarations across
two crates (shape-value canonical + two shape-jit locals of different
integer types). What exists as mitigation: a `#[test]`
(`mir_compiler/statements.rs:1744-1751`) asserting
`HeapHeader::OFFSET_REFCOUNT == 0` etc., and `HEAP_KIND_V2_CLOSURE == 84`
(statements.rs:1741). What does NOT exist: any assert tying
`STRING_OBJ_DATA_OFFSET`/`DATA_PTR_OFFSET`/`TYPED_OBJ_SLOT_DATA_OFFSET`
to the shape-value structs — and the codebase demonstrably knows the right
idiom, because `crates/shape-value/src/v2/closure_layout.rs:205` ties
`SHARED_CELL_VALUE_OFFSET` to the struct with a const-assert, and
`crates/shape-jit/src/context.rs` uses `offset_of!` asserts for
JITContext. If anyone reorders a field in `StringObj` or `TypedArray`,
the JIT compiles clean and emits loads from the wrong offset — silent
wrong-results or UB, caught only by end-to-end tests. Cheap fix: replace
each local literal with `shape_value::v2::string_obj::StringObj::OFFSET_DATA as i32`
or add `const _: () = assert!(...)` next to each local.

### 4.5 python vs typescript extensions — parallel structure, intentionally divergent bodies

Module layout is a clone: both crates have `lib.rs` / `marshaling.rs` /
`runtime.rs` / `error_mapping.rs` (python adds `arrow_bridge.rs`), and the
Shape-facing builtin surface is line-identical
(`extensions/python/src/lib.rs:33,39` vs
`extensions/typescript/src/lib.rs:33,39` — `pub builtin fn eval` /
`import`). The bodies, however, are per-runtime: `diff
marshaling.rs marshaling.rs` shows 499 changed lines across 397+206 total —
these are NOT copy-paste clones drifting, they are genuinely different
marshalers (PyO3 vs deno_core) behind the shared `LanguageRuntimeVTable`
contract (`crates/shape-abi-v1/src/lib.rs:722`). Verdict: the vtable is
doing its job; the only DRY residue is the duplicated builtin-declaration
prelude and error-category scaffolding (python's `error_mapping.rs` is 234
lines vs typescript's 66 — python maps more exception classes, not drift).
No action needed beyond keeping the vtable authoritative.

### 4.6 Duplicated magic numbers — mostly held to one definition

- **Tier thresholds** (T1@100, T2@10k): single source at
  `crates/shape-vm/src/tier.rs:33-34`. The values recur elsewhere only in
  comments/tests (e.g. `crates/shape-jit/src/executor.rs:106` "(T1@100 /
  T2@10k)" comment; `tier.rs:796` test loop) — comment-drift risk only.
- **OSR threshold 1000**: single const `DEFAULT_OSR_THRESHOLD`
  (`tier.rs:132`).
- **`HEAP_KIND_V2_CLOSURE = 84`**: defined in shape-value, pinned by the
  JIT test `statements.rs:1740-1741` ("The plan fixes HEAP_KIND_V2_CLOSURE
  at 84") — cross-checked, good.
- The layout offsets (§4.4) are the exception, not the rule.

## 5. Split-brain analysis

### 5.1 VM vs JIT semantics — one live wrong-result divergence, three pinned registry drifts, one message-vocabulary fork

**Live divergence, reproduced this audit** (full transcript in §9.1): the
HOF return-kind confusion program prints `6.0` under `--mode vm` and
`4618441417868443648` under `--mode jit`, both exit 0. Pinned in
`tools/vmjit-diff/known-red.json` with a dated reclassification to WF-3A
(type-system root: unprovable HOF return type unified against an `: int`
annotation; the JIT stamps Int64 and prints the f64 payload's bits). The
pin discipline is exemplary — the divergence still ships.

**Pinned registry drifts**: `registry_cross_check`
(`crates/shape-jit/src/mir_compiler/types.rs:3799-3880`) iterates the VM
PHF method registry and cross-checks JIT return-kind tables, failing the
build if a pinned drift *stops* reproducing ("list must only shrink").
Current pins: `Array<int>.mean()` (VM `v2_int_avg` returns Float64,
`types.rs:1263-1268`), `HashMap.iter` carrier mismatch (`types.rs:3860`),
`HashMap.get`. These are known VM/JIT semantic gaps held visible rather
than fixed.

**Message-vocabulary fork** (§4.2): VM error paths call an int receiver
`"int"`/`"i8"`/…; the JIT's `receiver_type_name` calls every integer
width `"number"` (`ffi/call_method/mod.rs:124-144`). Same program, same
error, different type name depending on tier — in a language whose
central rule is int≠number.

**What keeps this class contained**: the vmjit-diff harness
(469-program corpus measured this audit; three tiers; anti-rot rule that
a MATCH on a pinned id is itself a failure) plus registry_cross_check.
This is real, working split-brain *management* — but the split-brain
itself (two independent implementations of language semantics) is
architectural and permanent; the harness converts silent drift into
pinned drift, it does not prevent drift.

### 5.2 Legacy `HeapHeader` vs v2 `HeapHeader` — two structs, one name, one wrong constant

Two modules in shape-value define a `HeapHeader`:

- `crates/shape-value/src/heap_header.rs` — exported as
  `shape_value::HeapHeader` (`lib.rs:70`), carries `FLAG_MARKED` /
  `FLAG_PINNED` / `FLAG_READONLY`, and hosts
  `impl HeapKind { MAX_VARIANT, from_u16 }`.
- `crates/shape-value/src/v2/heap_header.rs` — exported as
  `shape_value::V2HeapHeader` (`lib.rs:107`), carries the canonical
  `OFFSET_REFCOUNT/KIND/FLAGS` constants and the `HEAP_KIND_V2_*` kind
  space; this is what all v2 carriers (`string_obj.rs:14`,
  `typed_array.rs:19`, `closure_raw.rs:39`, `refcount.rs:7`) import.

The legacy module's `MAX_VARIANT = HeapKind::HashMap` (heap_header.rs:160)
is ordinal 17 of the 36-variant enum (`heap_variants.rs:63-101`; HashMap
is annotated `// 17`). Therefore `HeapKind::from_u16` (heap_header.rs:164)
and `HeapHeader::heap_kind()` (heap_header.rs:123) return `None` for the
18 kinds added after Stage C — FilterExpr, Reference, SharedCell, HashSet,
Iterator, Option, Result, Deque, PriorityQueue, Mutex, Atomic, Channel,
Lazy, Range, TraitObject, ModuleFn, Matrix, MatrixSlice. The module's own
unit tests (heap_header.rs:279-296) only probe ordinals ≤ 16 plus the
`MAX_VARIANT + 1 → None` boundary, so they pass forever regardless of how
stale `MAX_VARIANT` gets — the test suite *certifies the bug*.

Mitigating discovery: grep found **no production caller** of
`from_u16`/`heap_kind()` outside the module itself — production sites read
`header.kind` as a raw u16 and compare against `HEAP_KIND_V2_*` constants
(`executor/objects/array_basic.rs:392`,
`v2_handlers/v2_array_detect.rs:363-365`), which is unaffected. So this is
a latent-API split-brain, not an active data-corruption bug: an exported,
documented, tested API that lies for half the enum, one `use` statement
away from production. The 2026-05-08 dead-code ledger flagged exactly this
constant; it was fixed at HashMap-time and the enum kept growing — the
constant cannot NOT re-regress, because nothing mechanical ties it to the
enum (a `#[cfg(test)]` exhaustive-match trick or
`strum::EnumCount` would). §9.2.

### 5.3 JIT retain/release: 13 explicit arms + 23 frozen-fallback kinds

`crates/shape-jit/src/mir_compiler/ownership.rs` has explicit
retain/release arms for 13 kinds; the other 23 (`String, Decimal, BigInt,
DataTable, Future, TaskGroup, Temporal, TableView, Content, Instant,
IoHandle, NativeScalar, NativeView, Char, FilterExpr, Reference,
SharedCell, Iterator, Range, TraitObject, ModuleFn, Matrix, MatrixSlice` —
the `jit_lockstep_baseline` array, `scripts/verify-merge.sh:248-271`) ride
the legacy `arc_retain`/`arc_release` HeapHeader fallback. The script's own
comment (verify-merge.sh:220-224) states the stakes plainly: the legacy
carrier has "refcount at offset +4", the typed-Arc carrier has "refcount
at offset -16", and "This exact class has already produced three
documented segfault families (W12 string-carrier, W15.2 closure-carrier,
r5c-2-β-δ TypedArray-carrier)."

The freeze rule (baseline may only shrink; new variants need a dated audit
note proving legacy carrier shape) is the right containment. The residual
risk is asymmetric: the *check* verifies which table an arm is in, not
which carrier shape the kind's *producers* actually emit. If any producer
site migrates one of the 23 kinds to a typed-Arc carrier without touching
ownership.rs, the JIT applies +4-offset refcount ops to an allocation
whose refcount is at -16 — silent memory corruption. The r5c-2-β-δ
history shows beliefs about "what carrier is live for kind K" have been
empirically wrong before.

### 5.4 Wire vs snapshot persistence — same kinds, one path serializes, the other stringifies

`crates/shape-runtime/src/wire_conversion.rs` converts at least 10 heap
shapes into irreversible placeholder *strings* (all verified on the
working tree):

| Kind | wire_conversion.rs | Snapshot path |
|---|---|---|
| HashSet | `:742` → `"<hashset:phase-2c>"` | `SV::HashSet { .. }` real arm (snapshot.rs:3179 kind map) |
| Deque | `:747` → `"<deque:phase-2c>"` | `SV::DequeOpaque` |
| Channel | `:767` → `"<channel:phase-2c>"` | `SV::ChannelOpaque` |
| Mutex | `:803` → `"<mutex:phase-2c>"` | `SV::MutexOpaque` |
| Atomic | `:804` → `"<atomic:phase-2c>"` | `SV::AtomicI64` |
| Lazy | `:805` → `"<lazy:phase-2c>"` | `SV::LazyOpaque` |
| TraitObject | `:356,:811` → `"<trait_object:phase-2c>"` | serialized via HeapNode identity map (v7) |
| TableView | `:734` → `"<table_view:phase-2c>"` | — |
| Matrix / MatrixSlice | `:823,:826` → `"<matrix:RxC:phase-2c>"` | — |
| Content | `:633` → `"<content:phase-2c-rebuild>"` | — |

The split-brain: the snapshot path (`snapshot.rs`, v7) learned to carry
these kinds — several with full identity-mapped cycle support — while the
wire path still holds the phase-2c deferral. A distributed program that
sends a HashSet over the wire receives the literal string
`"<hashset:phase-2c>"` on the other side, **typed as a string**, with no
error. Given the project's priority spine explicitly includes
"resumability+distributed" and "polyglot×distributed must compose", a
silently-lossy wire boundary for 10 kinds is a P1: it converts a type-safe
value into a poisoned string that will fail (or worse, not fail) far from
the cause. The right behavior under strict typing is a structured
serialization error, which the codebase's own SURFACE discipline already
prescribes elsewhere. §9.4.

### 5.5 Doc-vs-code split-brain (CLAUDE.md as a second implementation)

CLAUDE.md is treated as binding by every agent that touches this repo, so
its factual claims function as a parallel implementation of the
architecture — and several have drifted:

1. **`NativeKind` variant list**: CLAUDE.md names 11 variants ("Float64 /
   Int64 / Int32 / Int8 / Bool / Unit / Null / Ptr(HeapKind) / String /
   StringV2 / DecimalV2"). The real enum has **30** variants
   (`crates/shape-value/src/native_kind.rs:32-` — counted this audit:
   full signed/unsigned width ladder Int8–UIntSize, `Float32`, `Char`,
   plus a `Nullable*` twin for every scalar). An agent "adding the missing
   NativeKind arm" from the CLAUDE.md list would produce a non-exhaustive
   mental model of 1/3 the real surface.
2. **`shape-common`**: documented as "Shared utilities across crates";
   reality: a `Cargo.toml` with dependency declarations and **no `src/`**,
   not in `[workspace] members` (root `Cargo.toml:2-21`). Anyone told to
   "put shared helpers in shape-common" lands in a ghost crate.
3. **`shape-diagnostics`** (729 LOC, 4 files, a real workspace member and
   the home of the LSDS diagnostic format that ADR-006 declares "the
   primary diagnostic format") is **absent from the CLAUDE.md crate map**
   entirely.
4. **`emit_binary_op` doc + `audit-dynamic-fallback` feature** (§3.4):
   both describe a Dynamic-opcode reality deleted several waves ago.
5. **`tools/shape-fuzz`** (1,821 LOC, a workspace member per root
   Cargo.toml:15) is also missing from the crate map.

None of these is code-level unsoundness; all of them are the mechanism by
which the *next* split-brain gets written.

### 5.6 Cargo feature-flag matrix — the gc two-tier join, verified crate by crate

The GC flip (`ce332ca2`, "Make GC a default feature (both tiers)") created
a feature that must be enabled in **two places** that no Cargo mechanism
links:

| Crate | `default` | `gc` definition |
|---|---|---|
| `crates/shape-value` | `[]` (Cargo.toml:27) | `gc = []` — metadata + candidate buffer |
| `crates/shape-vm` | `["jit", "gc"]` (Cargo.toml:57) | `gc = ["shape-value/gc"]` (:76) — interpreter barriers |
| `crates/shape-jit` | `[]` (Cargo.toml:52) | `gc = ["shape-value/gc"]` (:69) — 8 `cfg(feature = "gc")` gates on `jit_write_barrier` / `jit_gc_safepoint` |
| `bin/shape-cli` | `["jit", "gc"]` (:13) | `gc = ["shape-vm/gc", "shape-jit?/gc"]` (:26) — **the only place the two tiers are joined** |

The CLI's comment block (Cargo.toml:15-25) is excellent and names the
hazard exactly: "Without `shape-jit/gc` the shipped JIT-on binary would
run gc-on interpreter + gc-OFF JIT, leaking cycles mutated on jit frames."
The shipped binary is correct. But the hazard is live for every *other*
binary/test crate that links both:

- **`tools/shape-test`** (Cargo.toml:9-21): depends on
  `shape-vm = { workspace = true }` (default features → `jit`+`gc` →
  `shape-value/gc` on) AND `shape-jit = { workspace = true }` (default
  `[]` → **`shape-jit/gc` OFF**), with `default = []` locally and no gc
  join. Workspace dep entries (root Cargo.toml:49-53) do not disable or
  add features. Net: **the integration-test harness runs the exact
  mismatched configuration** — gc-on interpreter + no-op JIT
  write-barriers — **that the CLI comment says leaks cycles and that the
  shipped binary never runs.** The 11,800-test suite's integration tier
  is therefore validating a configuration nobody ships, and NOT validating
  the one everybody ships. This is the precise gap the project memory
  flagged abstractly ("the interpreter #31 test can't catch the JIT gap"),
  now confirmed structurally in the manifests. §9.3.
- Any external consumer (shape-app / shape-server, outside this
  workspace) that depends on shape-vm+shape-jit must reproduce the CLI's
  join by hand; nothing in the crates themselves fails the build or warns
  on the mismatch. A `compile_error!` under
  `#[cfg(all(feature-detect mismatch))]` is impossible cross-crate, but a
  runtime assert at JIT-executor init ("shape-vm built with gc, shape-jit
  without") is not.

Other lockstep-ish features checked: `quic = ["shape-wire/quic"]`
(shape-vm:77) — single forward, fine. `deep-tests` exists independently in
4 crates (shape-ast:12, shape-runtime:89, shape-vm:58, shape-jit:64) with
no forwarding — by design (just recipes enable them as a set), but the
same "must-enable-together" shape with only a Justfile as the join.
`jit-trace` similarly co-features across shape-vm:62-66/shape-jit:56 with
a comment-only contract.

## 6. ADR & spec conformance (rule-by-rule for this territory)

Marker density: 20 `// ADR-005` and 547 `// ADR-006` comment markers
across the workspace (grep counts) — the marker convention is followed.

### ADR-005 (single discriminator)

| Rule | Verdict | Evidence |
|---|---|---|
| §1 `HeapValue` is the canonical discriminator; no sum types projecting 1:1 to HeapKind | **CONFORMS with a named tension** | No new parallel runtime discriminator found. `SerializableVMValue` (snapshot) and `WireValue` (wire) are per-format enums whose variants project onto heap kinds via `expected_kind_from_serializable` — they are serialization schemas, not runtime dispatchers, and ADR-005 lists snapshot serialization among the layers that "take `Arc<HeapValue>` and dispatch on kind". The projection function being duplicated across crates (§4.1) is where the 1:1 mapping actually drifts. |
| §2 Single `TypedFieldValue::String(Arc<String>)` exception | **CONFORMS** | `crates/shape-runtime/src/type_schema/mod.rs:30,58` — the exception is present, commented as the §Decision §2 exception, and no second exception variant was found. |
| §4 Uniform slot ABI, no VM/JIT boundary conversion | **CONFORMS** (spot-checked) | `ValueSlot` typed-pointer constructors in shape-value; no `Box<HeapValue>` slot wrapping found in new code paths read. |
| §Forbidden `from_heap_arc` catch-all | **CONFORMS** | grep: no `from_heap_arc` in the workspace. |

### ADR-006 (value & memory model) — rules binding a cross-cutting audit

| Rule | Verdict | Evidence |
|---|---|---|
| §2.7.7/Q9: VM stack = parallel `Vec<u64>` + `Vec<NativeKind>`; forbidden `Vec<KindedSlot>` stack, 16-byte slots, `Option<NativeKind>` placeholders | **CONFORMS** | `executor/vm_impl/stack.rs:6` ("`kinds: Vec<NativeKind>` — 1-byte interpretation per slot"); `Vec<KindedSlot>` occurrences are builtin-args/row buffers, explicitly sanctioned (`vm_impl/builtins.rs:4`, `stack.rs:1223` comment). |
| §2.7.7 forbidden transitional names (`push_raw_u64`, `pop_raw_u64`, `push_native_i64`, `stack_read_owned`, `stack_peek_raw`) | **CONFORMS** | grep across all crates: **0 hits** outside comments. |
| §2.7.8/Q10: cell storage parallel-kind tracks | **CONFORMS** | `executor/mod.rs:332` `module_binding_kinds: Vec<NativeKind>`; `:693,:782,:885` document the closure-heap-bits / teardown tracks. |
| §2.7.6/Q8: KindedSlot API = one constructor + ≤1 scalar accessor per variant; **NO per-heap-variant accessors** | **VIOLATION (unsanctioned exception)** | `crates/shape-value/src/kinded_slot.rs:762` — `pub fn as_typed_object_storage(&self) -> Option<&TypedObjectStorage>` is a per-heap-variant accessor for `Ptr(HeapKind::TypedObject)`, landed 2026-06-26 (`d81eb0ac`), used by 10+ production files (`execution.rs`, `typed_object_ops.rs`, `property_access.rs`, `vm_impl/modules.rs`, …). It sits 10 lines above the comment block restating the rule ("Heap variants do NOT get per-variant accessors here", kinded_slot.rs:791-800). The ADR forbids exactly this shape three times (006 §lines 875, 896, 1326) and **re-affirmed the rule after this accessor landed** while rejecting `as_temporal()`/`as_instant()` (006 line 6156: "The §2.7.6 / Q8 forbidden-shape rule against per-heap-variant accessors stands"), and separately rejected the analogous `as_ref_target()` (006:2873). No amendment names `as_typed_object_storage`. The function is kind-guarded and memory-safe — this is a governance breach, not a soundness bug — but it is the precise drift-attractor shape (per-kind carrier API growth) the ADR was written to stop, and per the ADR's own text it needs either deletion (route through the 5-arm receiver-recovery pattern) or a named, bounded amendment. §9.5. |
| §2.7.9: FilterExpr = pure-discriminator kind, all Q8/Q10 tables carry `Arc<FilterNode>` arms | **CONFORMS** | FilterExpr arms present in all four core tables (§2.2 extraction); origin story memorialized at `heap_variants.rs:101-110`. |
| §2.7.10/Q11 MethodFnV2 ABI; no `MethodFn`/`MethodFnLegacy`/side-slice | **CONFORMS** | `MethodFnV2` imported at `ic_fast_paths.rs:18`; grep for forbidden ABI names: 0 live hits. |
| §2.7.11/Q12 value-call ABI; no `call_value_legacy`/`call_value_raw_u64` | **CONFORMS** | grep: 0 live hits. |
| §Forbidden (CLAUDE.md) `ValueWord`, generic opcodes, `SlotKind::Dynamic`, `exec_*_dynamic_fallback` | **CONFORMS** | `bash scripts/check-no-dynamic.sh` → EXIT=0 on the dirty tree (run this audit); Dynamic opcode family exists only as tombstones (`opcode_defs.rs:87-105`) with ordinals reserved-not-reused (`:598`). |
| Mechanical enforcement (`prove_native_kind` private ProofGap, sentinel test, verify-merge) | **CONFORMS** | Sentinel `executor/tests/no_dynamic.rs` present; verify-merge 15 checks read in full; check-no-dynamic green. |
| §2.7 `KindedSlot` must not leak into typed VM↔JIT slot ABI | **CONFORMS** (spot-checked) | JIT FFI signatures read this audit take raw `u64`/typed pointers + `NativeKind` (e.g. `jit_v2_array_push(*mut HeapHeader, u64, u8)`), not `KindedSlot`. |

**Net ADR verdict**: 12 of 13 rules checked conform, several with
mechanical enforcement — an unusually strong showing. The one violation
(`as_typed_object_storage`) matters precisely because everything else
holds: the ADR regime works, so an unsanctioned exception that survives
two subsequent rule re-affirmations is a process failure worth fixing
while it is still one accessor and not five.

## 7. Test coverage in-territory

### 7.1 Counts

`#[test]` function counts per crate (grep, includes `#[cfg(test)]` unit
modules per the project convention):

| Crate | `#[test]` fns | `#[ignore]` |
|---|---:|---:|
| tools/shape-test (integration `tests/`) | 7,260 | 2 |
| crates/shape-vm | 3,109 | 80 |
| crates/shape-runtime | 1,518 | 0 |
| crates/shape-jit | 849 | 26 |
| tools/shape-lsp | 763 | 0 |
| crates/shape-ast | 608 | 0 |
| crates/shape-value | 480 | 0 |
| bin/shape-cli | 284 | 21 |
| crates/shape-wire | 60 | 0 |
| **Total (measured)** | **~14,900** | **129** |

(The CLAUDE.md figure "~11,800 tests" is an undercount against raw
`#[test]` attributes; some counted fns are cfg-gated behind `deep-tests`
and never run in default tiers, so both numbers are defensible.)

### 7.2 Anti-drift tests specifically (this vertical's concern)

- **`no_dynamic.rs` sentinel** (`crates/shape-vm/src/executor/tests/`) —
  present; asserts forbidden symbols absent, complementing the
  `check-no-dynamic.sh` grep gate (which passed EXIT=0 this audit, §2.1).
- **`registry_cross_check`** (`shape-jit/src/mir_compiler/types.rs:3799-3880`)
  — the only test that mechanically couples the VM method registry to JIT
  return-kind tables. Its "pinned drift list must only shrink" design
  means a *fixed* drift fails the build until the pin is removed —
  actively anti-rot.
- **JIT layout tests** (`mir_compiler/statements.rs:1740-1751`) — pin
  `HEAP_KIND_V2_CLOSURE == 84` and HeapHeader offsets 0/4/6. Coverage
  hole: no equivalent for the StringObj/TypedArray/TypedObjectStorage
  offsets the JIT also hardcodes (§4.4).
- **Legacy heap_header tests** (`heap_header.rs:279-296,314-321`) —
  present and *counterproductive*: they only exercise ordinals within the
  stale `MAX_VARIANT`, so they green-light the very staleness they should
  catch. A single `for` over a `strum`-derived variant list (or a
  `match`-exhaustiveness canary) would flip them from certifying the bug
  to catching it.
- **vmjit-diff** (469 programs) — not a cargo test; run via
  `tools/vmjit-diff/run-diff.mjs`. Not wired into any `just` gate found in
  the Justfile-adjacent scripts, so its execution cadence is manual/
  workflow-driven — a MATCH-rot pin could sit unnoticed between runs.

### 7.3 Ignored tests — do the reasons hold?

- shape-jit's 26 `#[ignore]`s: CLAUDE.md documents ~23 as the
  `test_jit_width_aware_*` / kernel-compilation family gated on the
  stdlib-JIT-caching follow-up — count is in the documented ballpark
  (23 + a few post-doc additions), reasons documented and plausible.
- shape-vm's 80: largest single block; sampling shows kinded-ABI
  phase-2c scaffolds (the same family as the 59 test-file
  `unimplemented!`s, §3.5) — deliberate deferred surface, consistent with
  the §2.7.4 deferral notes.
- shape-cli's 21: includes the 4 `tests/stdlib/simulation.rs` sims
  documented as blocked on V3-S5 ckpt-5/6 SURFACE territory — matches
  CLAUDE.md's Known Constraints.
- shape-test's 2: negligible.
- **Net**: ignore-hygiene is above average — every sampled `#[ignore]`
  traces to a documented follow-up. The risk is volume drift: 129 ignored
  tests with no single ledger tracking when each block's unblocking
  condition fires.

### 7.4 Gaps (tests that should exist for this vertical and do not)

1. No exhaustiveness canary tying `MAX_VARIANT` to the enum (§7.2).
2. No cross-copy consistency test for the three `kind_type_name`s or the
   JIT `receiver_type_name` vocabulary (§4.2).
3. No wire-vs-snapshot parity test ("every HeapKind either round-trips or
   errors structurally in BOTH paths") — would have caught §5.4 at
   introduction time.
4. No test that `expected_kind_from_serializable` (both copies) is
   exhaustive over `SerializableVMValue` — both end in `_ =>` wildcards,
   so the compiler cannot catch it either (§4.1).
5. No CI configuration test that shape-test's feature set matches the
   shipped binary's (`gc` on both tiers) — §5.6/§9.3.
6. No scheduled re-run of vmjit-diff in any committed gate script.

## 8. Book / docs vs reality for this vertical

The user-facing book (`/home/dev/dev/shape-lang/shape-web/book/` —
`book-site/` + `snippets/`) documents the language, not workspace
internals; no book page makes claims about crate structure, dispatch
tables, or dead code (grep for `shape-common`/`shape-types` in book
sources: no hits). For this vertical the binding "documentation" is
CLAUDE.md, `docs/codebase-index*`, and the ADRs — audited here as
doc-vs-code split-brain:

### 8.1 CLAUDE.md factual drift (all verified this audit)

| CLAUDE.md claim | Reality | Severity |
|---|---|---|
| `shape-common`: "Shared utilities across crates" | Cargo.toml only, no `src/`, not a workspace member | Misleads agents (ghost crate) |
| `NativeKind` "Variants: Float64/Int64/Int32/Int8/Bool/Unit/Null/Ptr/String/StringV2/DecimalV2" (11) | 30 variants incl. full width ladder + `Nullable*` twins (`native_kind.rs:32-`) | Materially wrong mental model |
| Crate map omits `shape-diagnostics` (LSDS home) and `tools/shape-fuzz` | Both are workspace members (root Cargo.toml:10,15) | Discovery gap |
| "Permission enum (16 perms) `shape-abi-v1/src/lib.rs:996`" | Enum at `lib.rs:1063` (line 996 is `NotFound = 4` in another enum) | Line-cite rot |
| "`LanguageRuntimeVTable` `lib.rs:722`" | `lib.rs:742` | Line-cite rot |
| "`BindingStorageClass` `type_tracking.rs:286`" (also cited in ADR-006) | `type_tracking.rs:359` | Line-cite rot in TWO binding docs |
| "Tier thresholds `tier.rs:17-87`", "IC state machine `feedback.rs:9-128`", "Wire protocol v1 `shape-wire/src/lib.rs:51`" | Spot-checked: all three still accurate | — |
| "~11,800 tests" | ~14,900 `#[test]` fns measured (incl. gated) | Stale-low |

ADR-006's own code cites have rotted too:
`CallFrame.closure_heap_bits (executor/mod.rs:188)` → actually
`executor/mod.rs:244`. Line-number cites in living documents rot by
construction; the fix is anchor comments (which the codebase already has —
`// ADR-006` markers) instead of line numbers.

### 8.2 codebase-index staleness

- `docs/codebase-index/00-dead-code-suspects.md` self-dates to the
  "three-agent indexing pass on 2026-05-08" (line 3) and has not been
  refreshed since — 9 weeks of the heaviest churn in the repo's history
  (W12–W18, GC phases 0–5, strict-flip). Verified sample (§2.6): 3 of 7
  high-confidence suspects fixed, `MAX_VARIANT` fixed-then-re-regressed,
  rest unresolved. The file's own §16-22 describes the `MAX_VARIANT` bug
  and its fix — and the bug is back, one enum-growth later, exactly as
  the mechanism predicts.
- The index root (`docs/codebase-index.md`) markets the per-domain files
  as "the source of truth" with no last-verified stamps per entry.

### 8.3 What the docs get right

CLAUDE.md's Forbidden Patterns section, the ADR-006 rule text, the
verify-merge/check-no-dynamic descriptions, the gc two-tier explanation in
`bin/shape-cli/Cargo.toml:15-25`, and the known-constraints list were all
verified accurate against the working tree in this audit — the
*normative* documentation is in far better shape than the *descriptive*
inventory documentation. The pattern: rules age well, snapshots don't.

## 9. Bugs & correctness risks found

### 9.1 [P1] Live VM-vs-JIT wrong-result divergence (HOF return kind)

Reproduced this audit on the prebuilt working-tree binary:

```
$ cat …/finding_s1_unknown_hof_return_kind_confusion.shape
fn apply(f, x) { f(x) }
fn ret_num(x) { x * 2.0 }
let r = apply(ret_num, 3.0)
let bad: int = r
print(bad)
print(bad % 4)

$ shape run --mode vm  …  →  6.0
                             0        (exit 0)
$ shape run --mode jit …  →  4618441417868443648
                             0        (exit 0)
```

`4618441417868443648 == 0x4018000000000000 == f64 6.0`'s raw bits printed
as an i64. Note the VM side is also wrong in a quieter way: `bad` is
annotated `: int` yet prints `6.0`, and `bad % 4` prints `0` in both modes
— the checker admitted a number into an int binding (the
`ReliableOnly`-adjacent type-hole family). Pinned in `known-red.json`
(class `hof-return-kind-raw-bits`) with root-cause routed to WF-3A
(HM let-generalization + D2 numeric-conversion error). Correctly triaged;
still a shipping wrong-result divergence, and the *only* deterministic
red in the 469-program corpus.

### 9.2 [P1] `HeapKind::MAX_VARIANT` re-regressed — exported API wrong for 18 of 36 kinds

`crates/shape-value/src/heap_header.rs:160`: `MAX_VARIANT = HeapKind::HashMap`
(ordinal 17; enum has 36 variants, `heap_variants.rs:63-101`). Effects:
`HeapKind::from_u16` returns `None` and `HeapHeader::heap_kind()` returns
`None` for every kind added after 2026-05-07 (Option, Result, HashSet,
Deque, Mutex, Atomic, Channel, Reference, SharedCell, Iterator, Range,
TraitObject, ModuleFn, Matrix, MatrixSlice, FilterExpr, PriorityQueue,
Lazy). No production caller today (§5.2) — severity P1 not for active
corruption but because (a) it is an exported `shape_value::HeapHeader`
API (`lib.rs:70`) whose first future caller inherits a 50%-wrong function,
(b) the adjacent `unsafe transmute` turns a future over-correction of the
constant into UB (§3.1 item 1), and (c) it is a *documented, previously
fixed* bug that regressed silently — proof the current test design cannot
hold it. Fix is one line + one canary test; deleting the legacy module
entirely (its only in-crate users are the v2 files importing the *v2*
header) is better.

### 9.3 [P1] Integration suite tests a configuration nobody ships (gc two-tier feature mismatch)

`tools/shape-test/Cargo.toml:12-13` links `shape-vm` (workspace default →
`gc` on → interpreter barriers + `shape-value/gc`) and `shape-jit`
(workspace default `[]` → **`gc` OFF** → the 8 `cfg(feature = "gc")`
hooks including `jit_write_barrier` compile as no-ops). The shipped binary
joins both via `bin/shape-cli/Cargo.toml:26`
(`gc = ["shape-vm/gc", "shape-jit?/gc"]`), whose own comment states the
mismatched config "leak[s] cycles mutated on jit frames". Consequence:
the 7,260-test integration tier runs gc-on-VM/gc-off-JIT — it can neither
catch JIT-barrier bugs (hooks are no-ops) nor reproduce shipped-binary
behavior (which has them live). This structurally confirms, in the
manifests, the two-tier-flip hazard class from the GC readiness report.
Fix: add `gc = ["shape-vm/gc", "shape-jit/gc"]` +
`default = ["gc"]` (or explicit features on the dep line) to shape-test,
and consider a runtime init assert in shape-jit ("built without gc but
shape-value/gc is enabled") to catch every future consumer.

### 9.4 [P1] Wire serialization silently stringifies 10 heap kinds

`crates/shape-runtime/src/wire_conversion.rs:356,633,734,742,747,767,803,
804,805,811,823,826` (§5.4 table): HashSet, Deque, Channel, Mutex, Atomic,
Lazy, TraitObject, TableView, Matrix/MatrixSlice, Content all become
placeholder `WireValue::String`s with no error. In a strict-typed language
with a distributed-execution priority spine, a `HashMap<string,HashSet<int>>`
sent over the wire arrives as `HashMap<string,string>` full of
`"<hashset:phase-2c>"` — type-level lying at the process boundary. The
sibling snapshot path already serializes most of these kinds properly
(`SV::HashSet`/`DequeOpaque`/`MutexOpaque`/… — snapshot.rs kind map at
3167-3196), so the encoders exist; the wire path just predates them.
Minimum fix: replace placeholders with a structured
`WireError::Unserializable(kind)`; real fix: reuse the snapshot arms.

### 9.5 [P2] `KindedSlot::as_typed_object_storage` — unsanctioned ADR-006 Q8 per-heap-variant accessor

`crates/shape-value/src/kinded_slot.rs:762`, landed `d81eb0ac` 2026-06-26,
10+ production call-site files. Memory-safe (kind-guarded, null-guarded,
miri-provenance-aware) but violates the thrice-stated Q8 carrier-API bound
that was re-affirmed *after* it landed (ADR-006 line 6156 rejects
`as_temporal()`/`as_instant()` on the same grounds while this accessor
already existed). Either delete (fold into the 5-arm receiver-recovery
pattern) or write the amendment naming it and its bound. Details §6.

### 9.6 [P2] Stale authority: `emit_binary_op` doc + `audit-dynamic-fallback` feature describe deleted dynamic dispatch

`compiler/helpers.rs:1691-1704` ("sole path … typed or Dynamic", "Every
`*Dynamic` opcode this shim still emits") on a `#[allow(dead_code)]`
test-only fn; `crates/shape-vm/Cargo.toml:78-83` feature watching for
Dynamic fallback emissions that cannot occur (opcodes tombstoned at
`opcode_defs.rs:87-105`). In a codebase whose CLAUDE.md dedicates a
section to refusing dynamic-dispatch rationalizations, live doc text
asserting "this shim still emits *Dynamic opcodes" is the exact seed
material for the next walk-back. Delete the fn (or fix the doc) and the
feature flag.

### 9.7 [P2] `print(closure)` prints a garbage integer (raw bits), exit 0

Reproduced this audit:

```
$ cat print_closure.shape
let f = |x: int| x + 1
print(f)
$ shape run print_closure.shape
18445899648779419843        (exit 0)
```

`18445899648779419843 = 0xFFFD_0000_0000_00C3` — a high-tag-prefixed
encoded payload (`0xFFFD << 48 | 0xC3`, i.e. what looks like a small
function id under a 16-bit tag prefix) printed as a decimal u64. The
tag-prefix shape deserves a look from the VM vertical: post
strict-typing there should be no tagged-word encodings in printable
positions (this may be a legitimate compile-time function-reference
constant, but it is being handed to `print` uninterpreted either way).
Notably this does NOT hit the loud
`todo!("phase-2c … closure formatting")` arm at
`executor/printing.rs:430` — the value reaches `print` through a path
that bypasses the kinded formatter entirely, which means the phase-2c
"fail loudly rather than print garbage" design intent
(printing.rs:425-434) is already being end-run. Split-brain between the
two print paths; worth folding into the closure-formatting follow-up.

### 9.8 [P2] `expected_kind_from_serializable` cross-crate drift (ModuleFunction)

Runtime copy maps `SV::ModuleFunction → Ptr(HeapKind::ModuleFn)`
(`shape-runtime/src/snapshot.rs:3194`); the VM copy
(`shape-vm/src/executor/snapshot.rs:1086-1131`) lacks the arm and
wildcards it to `NativeKind::Bool`. A snapshot slot holding a module
function restored through the VM-side path mis-kinds and surfaces a
kind-mismatch error instead of restoring — a restore-path availability
bug for a kind the runtime path handles. Both copies' `_ => Bool`
wildcards guarantee the next drift too. §4.1.

### 9.9 [P2] Divergent runtime type-name vocabularies in user-facing errors

Three `kind_type_name` copies in shape-vm disagree (`"map"`/`"hashmap"`,
`"bigint"`/`"int"`, `"table"`/`"datatable"`, width-preserving vs
collapsed ints) and the JIT calls all ints `"number"`
(§4.2, full cites there). User-visible inconsistency; trivially unifiable.

### 9.10 [P2] Legacy-vs-v2 HeapHeader module duality invites wrong-import bugs

Two exported types named `HeapHeader` (`shape_value::HeapHeader` legacy,
`shape_value::V2HeapHeader` alias — `lib.rs:70,107`) with identical field
layout today but different companion constants and kind spaces.
`executor/objects/array_basic.rs:392` reads the *legacy* struct's `.kind`
field on a fresh **v2** TypedArray allocation (it is a `debug_assert`
comparing against `HEAP_KIND_V2_TYPED_ARRAY` — array_basic.rs:389-395) —
correct only because the two structs' layouts coincide. Nothing asserts
they stay coincident, and the import choice (`shape_value::HeapHeader`
where the v2 header is meant) shows the name collision already causes
wrong-module grabs in practice. Merge or delete the legacy module
(§9.2 shares the fix).

## 10. What is done well

Named decisions worth preserving (each verified against the tree, not
just the docs):

1. **`define_heap_types!` single-source enum**
   (`crates/shape-value/src/heap_variants.rs:37`). The HeapKind enum is
   generated in one macro with per-ordinal comments and a memorialized
   ordinal-vacation policy (the 40-line TypedArray-ordinal-8 note). Every
   incident that shaped the enum (FilterExpr type-confusion, the 77→36
   trim) is written into the source at the point of maximum relevance.
2. **Exit-code-based merge gating** (`scripts/verify-merge.sh`, 15
   checks). The script's header explicitly rejects `grep -c` of cargo
   output in favor of list-comparison semantics; CHECK 6b's
   baseline-may-only-SHRINK design (verify-merge.sh:230-234) is the right
   ratchet shape — it makes the frozen fallback set monotonically
   decrease and turns "someone fixed a kind" into a mandatory baseline
   edit rather than silent slack.
3. **The vmjit-diff harness discipline** (`tools/vmjit-diff/`). 469
   programs, three corpus tiers, and — the standout — the known-red.json
   contract: "a MATCH on a listed id is flagged for removal … this file
   must never become a dumping ground that greens a red gate." Every pin
   carries a dated root-cause citation and reclassification history.
   Compare this to the industry default (a skip-list that only grows).
4. **`registry_cross_check`'s must-only-shrink pin list**
   (`mir_compiler/types.rs:3877`) — same ratchet philosophy applied to
   VM-registry-vs-JIT-table coupling, and it runs as a plain cargo test.
5. **Surface-and-stop error discipline.** 500+ `SURFACE`-tagged
   NotImplemented sites instead of silent fallbacks; the five live
   production `todo!()`s all carry dated ADR-section deferral notes
   (§3.5). Incompleteness in this codebase is loud and traceable — the
   opposite of the W-series walk-back pattern it was designed against.
6. **The "Intentional-future" module-level dead-code convention**
   (`optimizer/bounds.rs:3-8`, `ic_fast_paths.rs:9-15`): dead code is
   labeled with *why* it exists and *what* wires it in, rather than
   scattered per-item `#[allow]`s.
7. **Unsafe concentration + zero-unsafe periphery.** Parser, LSP, wire
   codec, macros: 0 unsafe sites. The entire unsafe surface lives in the
   four crates that own the value model, where the SAFETY-comment norm is
   established (§3.1).
8. **`bin/shape-cli/Cargo.toml`'s gc feature documentation**
   (lines 15-25): a feature-unification hazard explained precisely at the
   only place it can be fixed, including the failure mode of getting it
   wrong. If shape-test had copied this block, §9.3 would not exist.
9. **The const-assert exemplars**: `closure_layout.rs:205` (SHARED_CELL
   offset tied to struct) and `statements.rs:1740-1751` (JIT header
   layout pinned by test) prove the team knows how to mechanically couple
   duplicated constants — §4.4 is a coverage gap, not a knowledge gap.
10. **Tombstoning deleted opcode ordinals** (`opcode_defs.rs:87-105,598`):
    deleted Dynamic opcodes' ordinals are reserved-not-reused with the
    deletion fate named inline — the correct way to make deleted code
    stay deleted.

## 11. What is done poorly / tech debt

1. **Snapshot inventories rot with no refresh cadence.** The dead-code
   ledger (2026-05-08), CLAUDE.md's crate map and line cites, ADR-006's
   line cites — all drifted (§8). The project has ratchets for *code*
   drift but none for *inventory-doc* drift; the `MAX_VARIANT`
   fix-then-re-regress cycle is the canonical cost.
2. **Hand-maintained lockstep constants without mechanical coupling.**
   `MAX_VARIANT` (§9.2), the 5-module JIT offset constants (§4.4), the
   `kind_type_name` triplets (§4.2) — all are "update this when X
   changes" comments where a const-assert, an `EnumCount`, or a single
   shared fn was available. Everything the verify-merge greps *do* cover
   has held; everything left to comments has drifted.
3. **9k-line compiler files** (§3.2). statements.rs + helpers.rs +
   function_calls.rs = 27.7k LOC; combined with comment archaeology the
   effective navigation cost is higher than the line count suggests, and
   the "~8+ files per AST variant" rule is a symptom.
4. **Comment archaeology as provenance** (§3.6): wave IDs and dated
   citations copy-pasted into (and now diverging across) duplicated code
   blocks. The provenance is valuable; its storage location (inline,
   duplicated, unversioned) is the problem.
5. **Two persistence stacks at different maturity** (§5.4): snapshot v7
   went through five GC phases of hardening while wire_conversion still
   holds phase-2c placeholders — nobody owns "the serialization boundary"
   as one surface.
6. **Test-crate feature matrix unmanaged** (§9.3): the integration
   harness's Cargo.toml was never revisited after the gc-default flip;
   there is no manifest-level test that dev/test/ship configurations
   agree.
7. **Dead scaffolding without expiry**: optimizer planning layer
   (bounds.rs, numeric_arrays.rs), `ic_fast_paths` non-property halves,
   `emit_binary_op`, `audit-dynamic-fallback`, `shape-common`,
   `shape-types` — each individually justified, collectively ~an entire
   subsystem of compiled-but-unreachable code with no wave assigned to
   either wire it in or delete it.
8. **Error-message vocabulary was never designed** (§4.2/§9.9): type
   names shown to users are whatever each dispatch site's local table
   says, including the JIT calling ints "number" — in the language whose
   flagship rule is int≠number.

## 12. Prioritized recommendations

### P0 — none required for soundness

No live forbidden-pattern code, no confirmed active memory-unsafety from
this vertical's findings. The items below are ordered by
risk-times-imminence.

### P1 (do before the next release tag)

1. **Fix or fence `MAX_VARIANT`** (§9.2): preferred — delete
   `crates/shape-value/src/heap_header.rs`'s `from_u16`/`MAX_VARIANT`
   surface entirely (no production callers; ~1 hour incl. test updates).
   Minimum — set to the true last variant + add an exhaustiveness canary
   (`match`-based or `strum::EnumCount`) so the next enum growth fails
   compile. Effort: S.
2. **Join the gc features in `tools/shape-test`** (§9.3): one Cargo.toml
   line, then one integration re-run of the gc suite under the corrected
   config. Add a shape-jit init-time assert for the mismatch class.
   Effort: S code, M validation.
3. **Wire-path structured errors** (§9.4): replace the 10+ placeholder
   strings in wire_conversion.rs with a structured unserializable-kind
   error (S), then port the snapshot arms (M). Add the wire-vs-snapshot
   parity test (§7.4 item 3).
4. **Keep the HOF divergence pin visible** (§9.1): no new action —
   confirm WF-3A ownership; the pin discipline is working. Effort: 0.

### P2 (hygiene wave, batchable)

5. **Deduplicate `expected_kind_from_serializable`** (§4.1/§9.8): delete
   the shape-vm copy, import the runtime `pub fn`; replace both `_ =>`
   wildcards with exhaustive arms. Effort: S.
6. **Unify `kind_type_name`** (§4.2/§9.9): one
   `pub fn NativeKind::type_name(&self)` in shape-value; migrate the 3 VM
   copies + JIT `receiver_type_name`'s scalar arms; decide the canonical
   vocabulary once (int must not be "number"). Effort: S-M.
7. **Const-assert the JIT layout constants** (§4.4): 5 modules × 1-3
   asserts against `shape_value::v2` canonical constants, or replace the
   literals with re-exports. Effort: S.
8. **Resolve `as_typed_object_storage`** (§9.5): ADR amendment naming it
   (with the Q8 bound) or removal — either is fine; the status quo
   (unsanctioned, thrice-forbidden shape in live code) is not. Effort: S
   (amendment) / M (removal).
9. **Delete stale authority** (§9.6): `emit_binary_op` + its doc +
   `audit-dynamic-fallback` feature. Effort: S.
10. **Refresh the dead-code ledger with a verified-on date per entry**
    (§8.2) and add `cargo +nightly udeps` (or a sanctioned equivalent) to
    a scheduled gate. Effort: M.
11. **CLAUDE.md corrections** (§5.5/§8.1): NativeKind variant list,
    shape-common status, add shape-diagnostics + shape-fuzz rows, refresh
    the three drifted line cites (or replace line numbers with grep
    anchors). Effort: S.
12. **Schedule vmjit-diff** (§7.4 item 6): run the 469-corpus in the
    merge gate or a nightly, so MATCH-rot on pins surfaces within a day.
    Effort: S.

---

*End of report. Audit performed 2026-07-11 on the dirty working tree at
`/home/dev/dev/shape-lang/shape` (post-`ce332ca2`), read-only; scratch
artifacts under the session scratchpad
(`verticals/crosscutting-duplication/`). All transcripts in §2.1, §9.1,
§9.7 are verbatim from the prebuilt `target/debug/shape` binary of this
tree.*

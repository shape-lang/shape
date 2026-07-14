# Vertical Deep-Dive 18: ADRs & Engineering Governance Docs

Auditor 18 of 19 — ultra-deep-dive audit, 2026-07-11.
Territory: `docs/adr/` (001–006), `docs/runtime-v2-spec.md`, `docs/design/`, `docs/rfcs/`,
`docs/vision/`, `docs/cluster-audits/`, `docs/defections.md`, `CONTEXT.md`, `CLAUDE.md`,
`AGENTS.md`, `docs/codebase-index.md` + `docs/codebase-index/`, plus the mechanical
enforcement layer those docs claim (`scripts/check-no-dynamic.sh`, `scripts/verify-merge.sh`,
sentinel test `crates/shape-vm/src/executor/tests/no_dynamic.rs`).

All file:line cites are against the DIRTY working tree at audit time (branch `main`,
HEAD `ce332ca2` + uncommitted changes). All shell transcripts were actually run; the two
`libshape_ext_*` extension-load banner lines are stripped from transcripts per audit brief.

## 0. Executive summary

### Verdict

This is simultaneously the **best-instrumented governance system** I have seen in a codebase
of this size and a system whose **root pointers are broken**. The strengths are real and
unusual: a monotonic forbidden-symbol gate that actually runs and passes
(`scripts/check-no-dynamic.sh`, EXIT=0 at working tree), a 15-check exit-code-based merge
gate, a defections log with a disciplined format, 567 `// ADR-005`/`// ADR-006` marker
comments across 142 source files, and core ABI rules (MethodFnV2, value-call, stack
parallel-kind, TypedFieldValue) that code **demonstrably conforms to**.

The failures are concentrated at the *top* of the precedence chain and at *cross-reference
integrity*. `docs/runtime-v2-spec.md` — the document CLAUDE.md twice names "the
authoritative spec" and which self-describes as "**Authoritative spec — all implementation
must conform to this document**" — blesses, by name, the exact machinery CLAUDE.md's
Forbidden Patterns section deletes and refuses: the `ValueBits` shim,
`exec_arithmetic_dynamic_fallback`, `synthesize_value_word_from_raw`, and "FFI-boundary
helpers retained for the dynamic-fallback bridge" (a literal refuse-on-sight phrase). Its
last commit (`53a527e1`, Wave E+4.5) *added* the ValueWord-at-host-boundary section; nothing
has touched it since the May bulldozer deleted that world. ADR-001 and ADR-002 mandate
NaN-boxing as canonical and still carry Status: Accepted. ADR-006 — 7,536 lines, 430 KB,
append-only — cites its own sections that don't exist (§2.7.26 is cited by AGENTS.md for
`HeapKind::ModuleFn`; the section was never written), contradicts itself on the
`SharedCell` ordinal (§2.3/§2.7.12 say 19; code and §2.7.13 say `Reference`=19,
`SharedCell`=20), and contains a rule (Q8's "heap dispatch via `as_heap_value()`") that
later amendments made **unsatisfiable** for v2-raw carriers — code routed around it with
`KindedSlot::as_typed_object_storage` (an accessor on Q8's explicit forbidden list) and no
amendment was ever written.

Net: the enforcement *machinery* holds the line better than the *documents* do. Doc truth
drifts from code truth fastest at (a) file:line pointers (13 of 16 sampled quick-reference
pointers stale), (b) "Status" headers on pre-bulldozer planning docs, and (c) ADR-006
amendment numbering under multi-agent append pressure.

### Top-10 findings

| # | Sev | Finding | Evidence |
|---|-----|---------|----------|
| 1 | P0 (governance) | `docs/runtime-v2-spec.md` is labeled authoritative but mandates the forbidden pre-bulldozer world: ValueBits shim, `exec_arithmetic_dynamic_fallback`, `synthesize_value_word_from_raw`, "dynamic-fallback bridge" phrasing; its own §Non-Goals contradicts its §Host-Boundary section | `runtime-v2-spec.md:3,17,33,362-396,399-404`; last commit `53a527e1` *added* the ValueWord section; CLAUDE.md cites it as authoritative in §Architecture and ADR-006 §2.7.2 pins the slot ABI to it |
| 2 | P1 | ADR-006 §2.7.6/Q8 violated-in-effect and unamended: `KindedSlot::as_typed_object_storage` is a per-heap-variant accessor on Q8's explicit forbidden list (`as_typed_object()` named at ADR line 1334); exists because Q8's prescribed `as_heap_value()` dispatch is unsound on v2-raw TypedObject bits; zero mentions in ADR-006/defections.md | `kinded_slot.rs:762-790` vs `006-value-and-memory-model.md:1326-1339`; used in ≥10 files |
| 3 | P1 | AGENTS.md HeapKind ordinal table cites nonexistent "ADR-006 §2.7.26" as the authority for `HeapKind::ModuleFn` (ordinal 33); ADR-006 numbering skips §2.7.25→§2.7.27 and §10→§12; Q3 absent from §17 | AGENTS.md ordinal table row 33; `grep '§2.7.26' 006-*.md` → 0 defining hits; ModuleFn appears in ADR-006 only in passing (`006:4505`) |
| 4 | P1 | ADR-001/ADR-002 carry `Status: Accepted` yet mandate the NaN-boxed/tagged world that is now forbidden on sight; no Superseded marker anywhere in either file; ADR-003's central mechanism (single MethodDescriptor source, structural no-drift) was never built | `001-value-model.md:5,41`; `002-abi-unification.md:5,53`; no `struct MethodDescriptor` in tree; `MethodTable` (shape-runtime) and PHF maps (shape-vm) still independent |
| 5 | P1 | ADR-006 internal ordinal contradiction: §2.3 (line 226) and §2.7.12 (line 2383) assign `SharedCell` ordinal 19; code has `Reference` = 19, `SharedCell` = 20 (merge-time bump documented only in a code comment) | `heap_variants.rs:127-144` ("`Reference, // 19`", "`SharedCell, // 20`... 19 first at merge time") vs `006:226,2383,2428` |
| 6 | P1 | ADR-006 §1 bindings partially fiction: `SharedAtomic`/`SharedAtomicMut` never added to `BindingStorageClass`; §1.4 override syntax rejected by the parser (empirically); book documents `var` as always-refcounted, contradicting §1.2 smart-inference; CLAUDE.md states the §1.2 design as if current | `type_tracking.rs:359-368` (6 variants, no atomics); transcript §2.2 (`var x: Direct int` → "Undefined variable: 'int'"); book `variables.mdx:12` |
| 7 | P1 | LSDS "primary compiler output" (ADR-006 §9, CLAUDE.md) is opt-in and skeletal: `--diagnostics json` emits `diagnostic_id:"RUNTIME"` for a compile-time type error, no expected/found witnesses, `fixes:[]`; §14 metric "≥95% errors with witness + fix-diff" plainly unmet | transcript §2.5; `006:3824-3877,5992-5994` |
| 8 | P2 | Pointer rot across the navigation layer: 13/16 sampled file:line pointers in `codebase-index.md` + CLAUDE.md stale (e.g. `BindingStorageClass` 286→359, `drop_heap` 112→426, `HeapValue` 87→430, `pub enum Permission` 996→1063); `TypedArrayData` entry points at a type deleted 2026-05-15; CLAUDE.md cites `compiler/type_tracking.rs` (wrong dir) and `packages/duckdb/` (doesn't exist) | §5.4 drift table; `ls packages/` → `xgboost` only |
| 9 | P2 | `prove_native_kind` enforcement narrative overstated: exactly one production call site (`helpers_binding.rs:585`); most typed-opcode proof flows through `proof_gap_unresolved_operand` + the `last_emitted_native_kind` tracker the baseline itself marks "replace with prove_native_kind" (limit 8) | `rg prove_native_kind` → 1 call site; `check-no-dynamic-baseline.txt:20`; SB-8b design doc §0 "the current lie" |
| 10 | P2 | defections.md usage cliff: 36 dated entries in 2026-05-06..08, then 1 (05-13) + 1 (07-06) across two months of W15/W17/Phase-2d/GC work, while CLAUDE.md still mandates logging; justfile `verify-phase-5` says the sentinel test is "not yet wired up" though it exists and is wired | date histogram §7.2; `justfile:193-197` vs `executor/tests/mod.rs:43` |

Additional P2s in §9: stale planning docs presented without supersession banners
(v2-nanbox-removal-plan's Step 6 still reads "`ValueBits` shim: LANDED" — the canonical
defection, presented as success); `ValueSlot::from_heap` live callers past its §14 deletion
metric; AGENTS.md structural corruption (table row fused into a heading, 3× duplicate
"## Status values" sections); K3 draft ADR amendment proposes section/Q numbers that
already collide with ratified content.

### Scores

- **Feature completeness (governance system as designed): 62/100.** The enforcement layer
  (gates, sentinel, markers, defections format) is built and running; the documentation
  layer's core promises — accurate authoritative spec, superseded-status hygiene,
  amendment-per-HeapKind, LSDS-primary, ADR-003's structural no-drift — are respectively
  broken, absent, skipped once, unshipped, and unbuilt.
- **Code quality (gate scripts + sentinel + marker discipline): 82/100.** verify-merge.sh
  and check-no-dynamic.sh are careful, exit-code-based, self-documenting; the sentinel's
  fragment-assembled needle (so the grep gate can't trip on its own test) is a genuinely
  nice touch. Docked for the stale justfile comment, single-pattern sentinel coverage vs
  CLAUDE.md's plural claim, and heuristic-only CHECK 10.

### Biggest risk

The governance system's threat model is "an agent rationalizes a forbidden pattern back
in." Its main defense is documents. Today the highest-precedence runtime document
(`runtime-v2-spec.md`, "all implementation must conform") *instructs* the forbidden
pattern, and the fallback defenses (CLAUDE.md text + grep gate) are what actually hold the
line. That inversion is survivable while the current maintainer generation remembers the
May bulldozer; it fails silently the first time a fresh agent (or human) resolves the
CLAUDE.md-vs-spec conflict in the spec's favor — precisely the class of failure
(documentation-led regression) this repo has already paid 4–6 weeks for once, per its own
defections log. The fix is cheap (one supersession banner + a §Host-Boundary deletion);
the cost of not fixing it compounds with every new contributor.

## 1. Architecture & code structure map

### 1.1 The governance estate — inventory and size

All numbers from `wc -l` at working-tree HEAD:

| Artifact | Lines | Role | Freshness class |
|---|---|---|---|
| `docs/adr/001-value-model.md` | 158 | NanBoxed canonical, VMValue boundary-only (2026-02-19) | **stale — describes forbidden world, no Superseded marker** |
| `docs/adr/002-abi-unification.md` | 188 | Single NaN-boxing scheme VM+JIT (2026-02-19) | **stale — same** |
| `docs/adr/003-method-registry.md` | 160 | Single MethodDescriptor source (2026-02-19) | **stale — mechanism never built** (§6.3) |
| `docs/adr/004-native-c-interop.md` | 219 | `extern C` in core (2026-02-25) | partially current (one NanBoxed mention at :78) |
| `docs/adr/005-typed-slot-construction.md` | 300 | Single-discriminator + typed slots (2026-05-08) | current; §3 superseded by ADR-006 §12.2 (marked) |
| `docs/adr/006-value-and-memory-model.md` | **7,536** | Canonical value & memory model, living amendment log | current-but-eroding (§3.3, §9.6) |
| `docs/runtime-v2-spec.md` | 410 | "Authoritative spec" for the typed runtime | **self-contradictory** (§9.1) |
| `docs/defections.md` | 7,210 | Considered-but-rejected-compromise log | active-then-dormant (§7.2) |
| `CLAUDE.md` | 386 | Agent-facing project rules + Forbidden Patterns | current with factual errors (§8.2) |
| `AGENTS.md` | 1,131 | Live agent roster + HeapKind ordinal table | active, structurally corrupted (§3.4) |
| `CONTEXT.md` | 983 | Canonical language/semantics glossary | current — **but untracked in git** (§9.8) |
| `docs/codebase-index.md` + `codebase-index/` (7 files) | 2,598 | Concept-to-location navigation | paths hold, line numbers rot (§5.4) |
| `docs/design/` (16 entries) | 19,060 | Ratified 2026-07-05/07 design lanes | fresh (all dated ≤6 days old) |
| `docs/rfcs/` (8 files) | 3,876 | Speculative drafts, all 2026-05-18 | dormant drafts, honestly labeled |
| `docs/vision/` (8 files) | 2,835 | 2026-02-13 vision-era plans | **stale — pre-strict-typing world** |
| `docs/cluster-audits/` | **364 files, 112,835 lines** | Per-wave working audit trail | mixed; append-heavy, ~60 untracked |
| Top-level planning docs (`v2-nanbox-removal-plan`, `ownership-aware-runtime-v2`, `enhanced-escape-analysis-v2`, `wave4-container-migration`, `native-typed-arrays-v2`, `v2-closure-specialization`, `v2-monomorphization-design`, `nanbox-removal-session-prompt`, `v0.3-roadmap`, …) | ~4,400 | 2026-04-18-era "Alignment: 100%" phase docs | **stale — describe deleted world as landed success** |

Total estate ≈ **160k lines of governance/audit prose** for ≈ a similar-order Rust codebase.
The cluster-audits directory alone is 70% of it.

### 1.2 The enforcement layer (what makes this "architecture" and not just prose)

The docs claim four mechanical enforcement points; all four exist:

| Mechanism | Location | Verified state |
|---|---|---|
| Forbidden-symbol monotonic baseline gate | `scripts/check-no-dynamic.sh` (61 lines) + `docs/check-no-dynamic-baseline.txt` (31 patterns) | **runs green**: `bash scripts/check-no-dynamic.sh; EXIT=0` at working tree (transcript §2.6) |
| 15-check merge gate | `scripts/verify-merge.sh` (605 lines) | checks 1–14 incl. 6b: cargo gates, forbidden symbols, merge markers, HeapKind ordinal collisions, 4-table + JIT 5/6-table lockstep, take-both regex misses, receiver-recovery heuristic, `grep -c` anti-pattern, JIT `HK_*` ordinals, colon-return-type **doc-drift guard**, HeapKind wildcard guard — CLAUDE.md's "15 checks as of 2026-07-05" claim matches |
| Sentinel Rust test | `crates/shape-vm/src/executor/tests/no_dynamic.rs` (99 lines) | exists, wired (`executor/tests/mod.rs` includes it); covers exactly **one** pattern (Bool-default slot fabrication), not "forbidden symbols" plural as CLAUDE.md implies |
| Proof-seal type | `ProofGap` + private `ProofGapSeal(())` in `crates/shape-vm/src/type_tracking.rs:1235-1244` | conforms: constructor module-private; `kinds_consistent` is literal `expected == claimed` (`type_tracking.rs:1326-1328`) — no hidden relaxation |

Marker-comment discipline: exactly-spelled `// ADR-005` appears **20×**, `// ADR-006` **547×**,
across **142 source files** (`grep -rlE "// ADR-00[56]" crates/ bin/ tools/`). Free-text
"ADR-006" mentions are far higher (2,909 mentions in 319 files) — the ADR is genuinely the
codebase's reference vocabulary, not shelf-ware.

### 1.3 Document precedence chain (as documents declare it)

```
CLAUDE.md  ──"authoritative spec"──▶  docs/runtime-v2-spec.md      (slot ABI, typed opcodes)
CLAUDE.md  ──"canonical"──────────▶  docs/adr/006-*.md             (value & memory model)
ADR-006    ──"preserved verbatim"─▶  ADR-005 §1,2,4,Forbidden      (via §12.2)
ADR-006 §2.7 ─"must NOT leak into"▶  runtime-v2-spec slot ABI      (KindedSlot boundary)
codebase-index.md ─"ADR text is authoritative"─▶ ADRs
AGENTS.md ordinal table ──cites──▶  ADR-006 §2.7.x amendments      (incl. nonexistent §2.7.26)
```

The chain has a cycle-free shape on paper but a real conflict in content: runtime-v2-spec's
Status + Host-Boundary sections mandate what CLAUDE.md/ADR-006 forbid (§9.1). Nothing in the
chain says which wins; in practice agents treat CLAUDE.md as supreme, which is backwards
relative to the "authoritative spec" labeling.

### 1.4 Data flow of a rule (how governance actually reaches code)

1. Decision made → appended to ADR-006 as a §2.7.x amendment (all 25+ HeapKind/ABI decisions
   since 2026-05-09 took this path — e.g. §2.7.9 FilterExpr, §2.7.15 HashSet, §2.7.24 Q25 bundle).
2. Summary mirrored into CLAUDE.md §ADR-006 bullet list (manually — this is where drift enters;
   see NativeKind variant-list error, §8.2).
3. Load-bearing code sites get `// ADR-006 §x.y.z` markers (547 sites).
4. Forbidden shapes get a regex row in `check-no-dynamic-baseline.txt` (31 rows) and/or a
   verify-merge CHECK.
5. Considered-but-rejected alternatives go to `docs/defections.md` (66 dated headings;
   discipline collapsed after 2026-05-08 — §7.2).

Steps 1, 3, 4 hold up well. Steps 2 and 5 are the drift generators.

## 2. Feature completeness — the governance system's own promises

Each row distinguishes "the mechanism exists" from "it works end-to-end as documented".

### 2.1 Implemented and working (verified)

| Promise | Source | Verification |
|---|---|---|
| Forbidden-symbol gate, monotonic non-increasing | CLAUDE.md §Mechanical enforcement | ran it; EXIT=0 (transcript §2.6). Baseline format (limit/PCRE/note per row) matches the script's parser exactly |
| verify-merge 15 checks, exit-code based | CLAUDE.md §Mechanical enforcement | all 15 CHECK banners present (`grep "=== CHECK" scripts/verify-merge.sh`); anti-`grep -c` rationale documented in header (lines 11-16) |
| `ProofGap` unfabricatable | CLAUDE.md §Mechanical enforcement | `ProofGapSeal(())` module-private (`type_tracking.rs:1244`); only `proof_gap()` (private) mints; `proof_gap_unresolved_operand` is pub but produces a *gap*, not a pass — safe direction |
| MethodFnV2 ABI shape | ADR-006 §2.7.10/Q11 | signature at `method_registry.rs:48-53` matches ADR text token-for-token: `fn(&mut VirtualMachine, args: &[KindedSlot], ctx: Option<&mut ExecutionContext>) -> Result<KindedSlot, VMError>` |
| Value-call ABI + closure kind lockstep | ADR-006 §2.7.11/Q12 | `op_call_value` at `executor/control_flow/mod.rs:468`; `CallFrame.closure_heap_kind: Option<NativeKind>` at `executor/mod.rs:256`; surface-and-stop error text citing §2.7.8/Q10 at `control_flow/mod.rs:1244` |
| Stack parallel-kind track | ADR-006 §2.7.7/Q9 | `vm_impl/stack.rs:6-8` (`kinds: Vec<NativeKind>` module doc), lockstep debug assert at `:986-990` naming the ADR invariant |
| TypedFieldValue = 12 variants incl. bounded String exception | ADR-005 §2 | `type_schema/mod.rs:42-64`: exactly F64/I64/I8/U8/I16/U16/I32/U32/U64/Bool/String(Arc<String>)/Heap(Arc<HeapValue>), with the ADR justification reproduced in the comment block |
| No `from_heap_arc` catch-all | ADR-006 §13/Q6 | zero definitions in tree; only doc-comment mentions of the *rejected* shape (`slot.rs:81,104`) and the differently-named specialized `build_specialized_*_from_heap_arcs` array builders |
| `Box<HeapValue>` banned in new code | ADR-006 §2.3 | exactly 1 non-comment occurrence: the `#[deprecated]` `ValueSlot::from_heap` itself (`slot.rs:84-95`) |

### 2.2 Implemented on paper, not in the language (ADR-006 §1 bindings)

ADR-006 §1.4 specifies override syntax (`var x: Direct int = 5` shape). Empirical:

```
$ shape run override.shape        # var x: Direct int = 5
error[RUNTIME]: Bytecode compilation failed: Semantic error: Undefined variable: 'int'
  --> <input>:1:15
 1 | var x: Direct int = 5
```

And `BindingStorageClass` (`crates/shape-vm/src/type_tracking.rs:359-368`) has **no
`SharedAtomic` / `SharedAtomicMut` variants** — the two variants ADR-006 §3.2 and CLAUDE.md
("extended with `SharedAtomic`, `SharedAtomicMut`") describe as the extension. The lattice
today: Direct / UniqueHeap / SharedCow / Static / Shared / Table-family. §1 of the ADR is a
design commitment, not a description; neither the ADR (Status: Accepted) nor CLAUDE.md
(present tense) flags this.

### 2.3 The strict-typing contract itself — empirically true

The core claim every governance doc leans on ("if the type can't be proven, it is a compile
error; no coercion; no truthiness") holds at the shipped binary:

```
$ shape run strict1.shape         # let x: int = "hello"
error[RUNTIME]: ... string is not compatible with int
$ shape run strict2.shape         # let n = 5 ; if n { ... }
error[RUNTIME]: ... int is not compatible with bool
$ shape run strict3.shape         # int + number
error[RUNTIME]: ... int is not compatible with number
```

(This matters historically: project memory records a 2026-05-29 finding that the default
diagnostic mode suppressed almost all type errors. At today's working tree that bypass is
gone on these paths.)

### 2.4 LSDS — "primary compiler output" is aspirational

ADR-006 §9.1 ("LSDS is the primary output format... Renderers consume LSDS") and §14
("≥95% of compiler errors emit LSDS with witness + fix-diff fields populated"). Reality:

```
$ shape run --diagnostics json lsds.shape     # let x: int = "hello"
{"diagnostic_id":"RUNTIME","severity":"error","location":{"line":1,"col":1,"span":[0,0]},
 "message":"Bytecode compilation failed: Semantic error: Could not solve type constraints:\n  string is not compatible with int",
 "fixes":[]}
```

Opt-in flag, not primary; `diagnostic_id` is the generic `RUNTIME` for a *compile-time* type
error; no expected/found type witnesses (§9.3); empty `fixes` (§9.4); span is `[0,0]`. A
skeleton exists (`crates/shape-diagnostics`, and `docs/lsds-migration-plan.md` records one
2026-05-08 session) — the §14 metric is unmet by a wide margin and no doc says so.

### 2.5 ADR-003 — decided, never built

ADR-003's Decision §1-§5 (single `MethodDescriptor` const source generating both the
type-checker `MethodTable` and the runtime PHF maps; `MethodId(u16)` dispatch; structural
no-drift guarantee): **none of it exists**. `grep -rn "struct MethodDescriptor" crates/ tools/ bin/`
→ 0 hits; `MethodTable` (`shape-runtime/src/type_system/checking/method_table.rs`) and the
PHF maps (`shape-vm/src/executor/objects/method_registry.rs`) remain two independently
maintained registries — the exact drift risk ADR-003 §Problems.1 names. The ADR still says
Status: Accepted with no progress/abandonment note. (The drift is not hypothetical; sibling
audit 04-vm-interpreter's territory, but as governance: an Accepted ADR whose decision was
silently dropped is indistinguishable, for a reader, from one that landed.)

### 2.6 Gate run transcript

```
$ bash scripts/check-no-dynamic.sh
EXIT=0
```

Silent pass = no regression above any baseline row and no progress below it. Notable baseline
rows still >0 (all verified to be doc-comments/dead-gated code, not live paths):
`synthesize_value_word_from_raw` limit 12 (comment mentions describing the *deleted* symbol),
`exec_arithmetic_dynamic_fallback` limit 5 (same), `(nan_box|NanBox|NanTag)` limit 17,
`capture_as_value` limit 12, `last_emitted_native_kind` limit 8 — the last one is live code
the baseline itself annotates "replace with prove_native_kind" (see §9.9).

### 2.7 Completeness scoreboard

- **Built & enforced**: forbidden-symbol gate, merge gate, ProofGap seal, marker discipline,
  ABI shapes (Q9/Q10/Q11/Q12), TypedFieldValue, defections format. 
- **Built, partially observed**: KindedSlot Q8 bound (violated by one accessor, §6.2.5);
  ADR-005 cluster #7 (never executed, §6.1.4); `from_heap` deletion metric (missed, §6.2.9).
- **Decided, unbuilt**: ADR-003 descriptor unification; ADR-006 §1 binding forms
  (`SharedAtomic*`, override syntax); LSDS-primary; PES (honestly gated "Phase 5, behind
  feature flag" — fine).
- **Decided, superseded, unmarked**: ADR-001, ADR-002, vision-era docs, 2026-04-18 planning
  docs, runtime-v2-spec's Status + Host-Boundary sections.

## 3. Code quality (gate scripts, sentinel, and document engineering)

### 3.1 Shell gates — high quality

`scripts/check-no-dynamic.sh`: `set -euo pipefail`; per-symbol *monotonic non-increasing*
baseline rather than binary presence (lets deletion progress ratchet: "Once a symbol's
baseline reaches 0 it stays at 0 forever", baseline header rule 7); explicitly scopes to
source trees and documents why docs are excluded ("they discuss the forbidden patterns by
name as part of the enforcement contract", script lines 11-13); handles `rg` exit-1-on-zero-
matches correctly (`|| true` + awk sum). One real weakness: it greps comments too, so the
baseline must carry nonzero limits for symbols that are fully deleted from live code
(e.g. `synthesize_value_word_from_raw` limit 12) — a reader of the baseline can't tell
"12 comment mentions" from "12 live call sites" without doing the grep themselves.

`scripts/verify-merge.sh`: 605 lines, exit-code discipline throughout, and its header is a
postmortem in itself (lines 4-31: the `grep -c '^error\['` false-green incident, the 8
take-both regex misses, the discarded-module-opening-line class). Checks encode *learned*
failure modes, not generic lint: CHECK 5/12 (HeapKind ordinal collisions, including
JIT-private `HK_*`), CHECK 6/6b (4-table + JIT retain/release lockstep — directly enforcing
ADR-006 §2.7.7/§2.7.8's dispatch-table discipline), CHECK 13 (colon-return-type **doc**-drift
guard over CLAUDE.md/README/docs/stdlib/LSP-hover — the only gate here that checks documents).
CHECK 10 is honestly labeled "HEURISTIC — review-not-fail".

### 3.2 Sentinel test — clever, narrow

`no_dynamic.rs` assembles its needle from fragments at runtime (`["unwrap", "_or((0,"].concat()`)
so the plain-text shell gate can never trip on the sentinel's own source — a subtle
self-interference bug avoided by design (lines 13-15, 80-84). But it covers exactly one of
the baseline's 31 patterns. CLAUDE.md's "Sentinel test ... asserts forbidden symbols are
absent" (plural) oversells it, and `justfile:193-197` still says the sentinel "is not yet
wired up ... When it lands, add it here" — it landed; the `verify-phase-5` recipe never got
updated and still prints a TODO instead of running it (§9.10).

### 3.3 ADR-006 as a document — erosion under append pressure

Concrete structural defects, all verifiable by heading grep:

- **Section numbering**: `## 10` jumps to `## 12` (no §11 anywhere: `grep -n "^## 11"` → 0).
- **Amendment placement**: §2.7.16 physically sits inside §12 (line 3958, directly after
  §12.2); §2.7.6.A/§2.7.27/§2.7.28/§2.7.29 sit inside §15 Visibility (lines 6110-6904);
  §2.7.30/§2.7.31 sit *after* §17 Resolved Questions (7474, 7515).
- **Ordering**: §2.7.19 precedes §2.7.18 (lines 4121 vs 4140); §2.7.22 exists twice — the
  live Q23 amendment (4478) and its SUPERSEDED predecessor (4616), with the superseded one
  *after* the superseder.
- **Q-number split-brain**: §17 holds Q1-Q10 (Q3 absent — `grep "### Q3"` → 0 hits, and no
  "Q3 " prose mention at all); Q11-Q25+ live only as inline §2.7.x amendments. Two competing
  homes for rulings, and one ruling number (Q3) lost entirely.
- **Self-contradiction**: §2.3 (line 226) and §2.7.12 (lines 2383, 2428) assign
  `HeapKind::SharedCell` ordinal **19**; §2.7.13 assigns `Reference` ordinal 19 (line 2760);
  code resolves it as `Reference // 19`, `SharedCell // 20` with a merge-time note that lives
  only in `heap_variants.rs:141-142` ("T26 took 19 first at merge time"). The ADR was never
  corrected — three ADR passages still teach the wrong ordinal for a value that four
  dispatch tables key on.
- **Phantom section**: `HeapKind::ModuleFn` (ordinal 33) is justified by "ADR-006 §2.7.26"
  in AGENTS.md (roster row + ordinal-table row 801) — that section does not exist; ADR-006
  skips §2.7.25 → §2.7.27, and `ModuleFn` appears in ADR-006 only in unrelated passing
  mentions (lines 602, 631, 720, 1238). The 4-table lockstep merge gate (CHECK 6) verified
  the *code*; nobody wrote the *amendment* the roster claims landed.

None of these makes the individual rulings wrong; together they make the document
increasingly unciteable — the failure mode for an append-only canon whose readers navigate
by section number.

### 3.4 AGENTS.md — live but damaged

Actively maintained (rows dated through 2026-07-11, today) yet: three "## Status values"
headings (lines 715, 722, 735, 752), of which line 715 is a heading *fused onto a roster
table row* — visible take-both merge damage in the coordination file whose merge gate exists
to catch exactly that class elsewhere; 15 rows stuck at "AWAITING CLOSE" / 18 "| active"
including a 2026-05-14 dispatch never closed out. Single roster rows run to ~9,000 words
(the W17-comptime-vm-dispatch row), making the "registry" unscannable — the file itself
carries a "Maintainability policy" (files ≤500 lines) it exempts itself from.

### 3.5 Unsafe usage in-territory

The governance vertical's only Rust code is the sentinel test (no `unsafe`) and the
gate-adjacent `ProofGap` machinery (no `unsafe`). The `unsafe` blocks encountered while
conformance-sampling (`kinded_slot.rs:762-790` accessor, `closure_raw.rs:160`) carry SAFETY
comments tied to ADR invariants — quality is fine; the *governance* problem with the first
one is Q8 conformance, not soundness hygiene (§6.2.5).

### 3.6 Dead code in-territory

- `#[cfg(any())]`-gated modules referencing deleted APIs, kept "until a follow-up lands":
  `tools/shape-test/tests/datetime_stdlib/main.rs:14-19` (three submodules written against
  deleted `ValueWord`/`ValueWordExt`/`vmarray_from_vec`, gated 2026-05-10) and
  `crates/shape-vm/src/compiler/comptime.rs:2985-2987` (`tests_deferred` using
  `ValueWord::from_i64` etc.). Dead-gated code that *names* forbidden symbols is how comment
  baselines stay pinned above zero — cheap to delete, two months un-deleted.
- `docs/codebase-index/01/02/03-dead-code-suspects.md`: superseded inputs to the collated
  `00-dead-code-suspects.md` (which says "collated ... 2026-05-08"), still present, not
  referenced by the index TOC — a dead-code list that is itself dead code (§4.3).

## 4. Duplication & DRY violations

### 4.1 The forbidden-pattern list exists in five places

The same rule set (no ValueWord, no generic opcodes, no Bool-default, rename families) is
maintained in: CLAUDE.md §Forbidden Patterns (~120 lines), ADR-006 §13 + per-amendment
"Forbidden alternatives" blocks, `docs/check-no-dynamic-baseline.txt` (31 regex rows),
`scripts/verify-merge.sh` CHECKs, and `docs/defections.md` preamble. Divergence is real and
measurable: the baseline has rows CLAUDE.md never mentions (R6 carrier-UB Arc-on-`_new`
patterns, rows 41-42; W17 Stage-0 `let expected = NativeKind::Bool`, row 43) and CLAUDE.md
has refuse-on-sight phrases with no baseline row (`"documented intentional duality"`,
`"carrier unification via boundary deletion"` — prose-only, unenforceable by grep and not
in the gate). Risk: an agent auditing against one list passes; against another fails. The
baseline is the de-facto master (it's executable); no doc says so.

### 4.2 runtime-v2-spec.md duplicates ADR-006 §2/§4 — at an older revision

TypedArray/TypedStruct/HeapHeader layouts, slot ABI, tier design appear in both. The spec's
copies predate the May bulldozer; ADR-006's are current. Because CLAUDE.md sends readers to
the spec for "the authoritative spec" and to ADR-006 for "key rules", both get read, and
they disagree (ValueBits/ValueWord sections, §9.1). Duplication would be harmless if the
spec had a supersession banner; it has the opposite ("all implementation must conform").

### 4.3 codebase-index dead-code lists ×4

`00-dead-code-suspects.md` (collated, 29 suspects) coexists with its three source files
`01/02/03-dead-code-suspects.md`. The index TOC references only `00`. Updates since
2026-05-08 (if any) go to one copy or none. Concrete staleness: `02-runtime.md:95` still
carries a full entry for the `TypedArrayData` enum — deleted from `shape-value` on
2026-05-15 per the W12 deletion audit (CLAUDE.md cluster-0 log) and confirmed absent
(`grep "enum TypedArrayData" crates/shape-value/src/` → 0).

### 4.4 CLAUDE.md ADR summaries vs ADR text

CLAUDE.md restates ADR-006 rules in compressed form; two restatements are now factually
wrong (NativeKind variant list — lists `Unit`, which is not a variant, omits 20 of the 30
that are, `native_kind.rs:32-110+`; `prove_native_kind ... in compiler/type_tracking.rs` —
the file is `crates/shape-vm/src/type_tracking.rs`; there is no `compiler/type_tracking.rs`).
Compression is fine; unversioned compression that silently rots is the §1.4-step-2 drift
generator.

### 4.5 Defections format duplicated by cluster-audit "refused candidates" blocks

Post-2026-05-08, rejected alternatives are recorded inside per-wave cluster-audit files and
inside ADR-006 amendments ("Forbidden alternatives this rules out", e.g. §2.7.9 lines
1747-1770) rather than in defections.md — which explains part of the defections cliff
(§7.2): the *practice* survived, the *single log* didn't. Same information, now sharded
across 364 files with no index.

## 5. Split-brain analysis — same truth held in two places

### 5.1 runtime-v2-spec.md vs itself (the worst case: split-brain inside one file)

One 410-line file contains both worlds:

- **Contract section (line 38)**: "There are NO runtime type tags ... No `ValueWord`."
- **Status section (line 17)**: "`ValueBits` + `ValueWordExt` remain as a shim for the
  dynamic-fallback paths ... and `exec_arithmetic_dynamic_fallback` for value sites the
  compiler cannot prove."
- **Host Boundary section (lines 362-396)**: documents `synthesize_value_word_from_raw`,
  `execute() -> Result<ValueWord, VMError>`, a SlotKind→ValueWord decode table including
  `String / Dynamic / Unknown → ValueWord::from_raw_bits(bits)` — i.e. the deleted
  `SlotKind::Dynamic/Unknown` variants and the deleted synthesizer, presented as the current
  API ("Status: Wave E+4.5 (2026-04-27)").
- **Non-Goals section (line 403)**: "No `ValueWord` ↔ native conversion layer that persists
  past Step 6."

Working-tree code truth: `ValueWord` has **no definition anywhere** (`grep "enum|struct|type ValueWord"`
→ 0 definitions; `shape-value/src/lib.rs:7`: "There is no `ValueWord`"), and every source
mention is a comment about the deleted symbol or `#[cfg(any())]`-dead test code. The spec's
Status and Host-Boundary sections describe a world deleted ~2 months ago, under an
"Authoritative — all implementation must conform" banner.

### 5.2 ADR-001/002 vs ADR-005/006

ADR-001 Decision §1: "NanBoxed is the canonical runtime representation" (`001:41`).
ADR-002 Decision §1: "The VM's 3-bit tag scheme becomes canonical", with `tags.rs` constants
to share (`002:53,66-86`). ADR-005 §4 + ADR-006 §2.1: tag-free typed slots; NaN-box
reintroduction forbidden (ADR-006 §13: "No NaN-box or low-bit-tag reintroduction anywhere").
Both old ADRs still read `Status: Accepted (2026-02-19)`. Neither carries a Superseded
marker, and ADR-005/006 don't name them in their supersession chains (ADR-006 §12.2 handles
only ADR-005). A reader doing what ADR conventions promise — "check Status before relying" —
gets an affirmative green light on the forbidden architecture. The referenced artifacts are
gone (`shape-value/src/nanboxed.rs`, `shape-jit/src/nan_boxing.rs`, `tags.rs` — `find` → 0),
so code can't silently follow them, but plan-writing agents can.

### 5.3 Sentinel test vs shell gate

Two implementations of "forbidden patterns are absent": the shell gate covers 31 patterns
monotonic-against-baseline; the Rust sentinel covers 1 pattern binary. CLAUDE.md describes
them as equivalent layers. Divergence direction is safe (shell ⊃ sentinel) but the claimed
redundancy ("the prohibition survives even when the recipe is skipped", `no_dynamic.rs:7`)
holds for 1/31 patterns only.

### 5.4 Navigation-layer pointer rot (doc-vs-code line drift)

Sampled quick-reference pointers, working tree:

| Pointer (doc) | Claimed | Actual | Drift |
|---|---|---|---|
| `codebase-index.md` `shape.pest:760-771` `variable_decl` | 760 | 778 | +18 |
| `codebase-index.md` `field_types.rs:35` `FieldType` | 35 | 38 | +3 |
| `codebase-index.md` `native_kind.rs:32` `NativeKind` | 32 | 32 | exact |
| `codebase-index.md` `heap_variants.rs:56` `HeapKind` | 56 | 63 | +7 |
| `codebase-index.md` + CLAUDE.md `type_tracking.rs:286` `BindingStorageClass` | 286 | 359 | +73 |
| `codebase-index.md` `type_tracking.rs:299` `BindingSemantics` | 299 | 372 | +73 |
| CLAUDE.md `executor/mod.rs:188` `closure_heap_bits` | 188 | 243-256 | +55 |
| CLAUDE.md `shape-abi-v1/lib.rs:996` `Permission` | 996 | 1063 | +67 |
| CLAUDE.md `shape-abi-v1/lib.rs:722` `LanguageRuntimeVTable` | 722 | 742 | +20 |
| CLAUDE.md `core.rs:218` `to_annotation` | 218 | 362 | +144 |
| CLAUDE.md `tier.rs:17-87`, `feedback.rs:9-128`, `wire/lib.rs:51` | — | — | still accurate |
| `02-runtime.md:49` `drop_heap` `slot.rs:112` | 112 | 426 | +314 |
| ADR-006 §16 `type_tracking.rs:286-310` | 286 | 359 | +73 |

Paths and symbol names resolve in every sampled case; line numbers are unreliable beyond
±10. Worse than line rot, `02-runtime.md` has **content rot**: line 48 states per-FieldType
`ValueSlot` constructors "are not yet implemented" (they are — `slot.rs` has ~20 of them,
`from_string_arc` at :112); line 71 documents `HeapValue::TypedObject { schema_id, slots,
heap_mask }` as a struct variant (current: tuple `TypedObject(TypedObjectPtr)`); line 221
documents `Decimal(rust_decimal::Decimal)` unwrapped (current: `Arc<...>` per ADR-006 §2.3);
line 95 documents the deleted `TypedArrayData` enum. The index's own preamble promises "the
'Key rules / invariants' lines are the binding constraints" — several now bind readers to a
pre-bulldozer model.

### 5.5 AGENTS.md ordinal table vs ADR-006 vs code

Three copies of the HeapKind ordinal assignment exist: `heap_variants.rs` (source of truth),
AGENTS.md "HeapKind ordinal table (live, 2026-05-11)", ADR-006 §2.7.x amendment texts. Drift
found: ADR-006 says SharedCell=19 in three places while code+AGENTS say Reference=19 /
SharedCell=20 (§3.3); AGENTS row 33 cites nonexistent §2.7.26. The mechanical guard (CHECK 5)
protects the *code* enum against collisions; nothing guards the two prose copies.

### 5.6 CLAUDE.md vs shipped repo facts

- "packages/ | Pure Shape packages (e.g. `packages/duckdb/`)" — `ls packages/` → `xgboost`
  only. 
- "Crate Map ... shape-types | Empty crate skeleton" — accurate (verified `crates/shape-types/`
  has only `data/`), and a good example of the *right* way to document a trap.
- NativeKind variant list wrong both directions (§4.4).
- "just verify-merge ... 15 checks" — matches script (§1.2). 

### 5.7 Design docs vs ADR-006 (healthy split, worth naming)

The 2026-07-05/07 ratified design lane (`docs/design/*.md`) consistently opens with
"Binding constraints: CLAUDE.md §Forbidden Patterns, ADR-006 §..., runtime-v2-spec.md"
(e.g. `snapshot-resume.md` header, `comptime-excellence.md` header). This is governance
working as intended — new design explicitly subordinates itself to the canon. It also means
every one of those docs transitively endorses the broken runtime-v2-spec as a binding
constraint, which is how the §5.1 split-brain propagates forward.

## 6. ADR & spec conformance — rule by rule

Ratings: **current** (rules bind and code conforms), **partially-stale** (some rules
superseded/violated without amendment), **stale** (describes a dead world).

### 6.1 ADR-005: Typed Slot Construction Discipline — **current**, with one dead letter

| Rule | Conformance | Evidence |
|---|---|---|
| §1 Single-discriminator: no sum types projecting 1:1 onto HeapKind above HeapValue | **CONFORMS in new code; grandfathered violation persists** | `TypedFieldValue` clean (§6.1.2). `ConcreteReturn` still carries the heap arms §1 names as the cleanup target *plus* per-HeapKind `DataTable(Arc<DataTable>)` / `IoHandle(Arc<IoHandleData>)` arms (`typed_module_exports.rs:55-183`) — each carries a defections-cited disposition dated 2026-05-06/07 (pre-ADR), but see §6.1.4 |
| §2 String exception, named and bounded | **CONFORMS exactly** | `type_schema/mod.rs:42-64`: 12 variants; String(Arc<String>) justified in-comment with the ADR's own wording; no second exception added anywhere sampled |
| §3 Typed slot storage, per-FieldType constructors | **CONFORMS (as corrected by ADR-006 §2.4)** | `slot.rs:98+` ~20 per-FieldType constructors, `Arc::into_raw` storage; ADR-006 §12.2 formally corrected §3's examples |
| §4 Uniform slot ABI VM↔JIT | **CONFORMS by construction where checkable** | 8-byte `ValueSlot`; KindedSlot confined to runtime tier; JIT KindedSlot usage is at the §2.7.5 marshal boundary (`shape-jit/src/ffi/conversion.rs`), not codegen slots — full JIT-side verification is vertical 05's territory |
| §Forbidden: no new ConcreteReturn heap arms while cluster #7 pending | **HELD since 2026-05-08 on the evidence available** | all heap-arm doc comments date 2026-05-06/07; no newer-dated arm found. But: |
| §Visibility: marker comments at 6 named sites | **CONFORMS** | markers present at `heap_variants.rs`, `slot.rs`, `native_kind.rs`, `typed_module_exports.rs`, `json_value.rs`; CLAUDE.md subsection exists; defections cross-ref exists (2026-05-08 entry); sentinel "optional" — one exists (different pattern) |

**6.1.4 The dead letter**: §Implementation-roadmap Layer 2 / "cluster #7" — fold
ConcreteReturn's heap arms into one `Heap(Arc<HeapValue>)`. Two months on: not executed, not
re-scheduled, not renounced. The `// ADR-005` marker at `typed_module_exports.rs:49-54`
still says "scheduled for cluster #7 cleanup". The parallel discriminator the ADR predicted
would drift has instead *accreted six variants of documentation defending each arm* — the
enum is now ~18 variants with per-arm dispositions. That is drift with excellent paperwork.

### 6.2 ADR-006: Value & Memory Model — **partially-stale** (living, load-bearing, eroding)

| Rule | Conformance | Evidence |
|---|---|---|
| §1 bindings: `let`/`let mut`/`var` storage-class model, §1.4 override syntax, §3.2 `SharedAtomic*` | **NOT IMPLEMENTED** | transcript §2.2 (override syntax rejected); `type_tracking.rs:359-368` lacks both atomic variants; book teaches the *old* always-RC `var` (§8.1) |
| §2.1 tag-free typed slots, no ValueWord | **CONFORMS** | no ValueWord definition in tree; stack is `Vec<u64>` + `Vec<NativeKind>` |
| §2.3 HeapValue payloads = typed `Arc<T>` | **CONFORMS** | every current arm carries `Arc<T>`/typed ptr (`heap_variants.rs:430+`); the two raw-carrier exceptions (`TypedObjectPtr`, `TraitObjectPtr`, `OwnedClosureBlock`) are §2.7.24-amended |
| §2.4 per-FieldType ValueSlot constructors | **CONFORMS** | `slot.rs` constructor block with ADR marker |
| §2.6/Q6/§13 no `from_heap_arc` catch-all | **CONFORMS** | 0 definitions (§2.1 table) |
| §2.7.5 KindedSlot not in VM↔JIT slot ABI | **CONFORMS as sampled** | JIT uses it only in ffi/marshal files |
| §2.7.6/Q8 carrier API bounded by NativeKind cardinality; **no per-heap-variant accessors** | **VIOLATED, unamended** | `KindedSlot::as_typed_object_storage` (`kinded_slot.rs:762`) dispatches on `Ptr(HeapKind::TypedObject)` — a per-heap-variant accessor. It sits ~15 lines above the comment block restating the prohibition (`kinded_slot.rs:791-797`). ≥21 external call sites (execution.rs, result_option_carrier.rs, …). Root cause: Q8's prescribed heap dispatch (`slot.as_heap_value()`) is *unsound* for v2-raw `_new`-carrier TypedObject bits, so code needed a typed borrow — a legitimate need that required a Q-amendment and never got one. Flagged by the 2026-07-04 audit (finding 11); still open; no defections.md entry |
| §2.7.6 "~150 LoC carrier total" | **exceeded 14×** | `kinded_slot.rs` = 2,143 lines. Partly Miri-provenance plumbing and per-constructor docs — but the Q8 rationale used the size bound as the argument against Option 1; the bound quietly died |
| §2.7.7/Q9 stack parallel-kind track; no Bool-default fabrication | **CONFORMS** | `vm_impl/stack.rs` lockstep asserts; the `kinds.push(NativeKind::Bool)` in `push_kinded_slow` (`stack.rs:1022,1051`) fills *unreached capacity* above sp with zeroed slots and is immediately overwritten at sp — filler, not fabrication (though `Null` would be more honest filler) |
| §2.7.8/Q10 cell-storage kinds; surface-and-stop not Bool-default | **CONFORMS** | `OwnedClosureBlock::read_capture_kinded` (`closure_raw.rs:160`); `CallFrame.closure_heap_kind` lockstep panic-messages cite §2.7.8/Q10 (`control_flow/mod.rs:1244,1306`); baseline rows 39/40/43 pin the Bool-default shapes at 0 |
| §2.7.9 FilterExpr | **CONFORMS incl. the symmetry arm** | code has both `HeapKind::FilterExpr` (ord 18) and the ADR-documented symmetry arm `HeapValue::FilterExpr(Arc<FilterNode>)` (`heap_variants.rs`); the amendment (006:1701-1770) covers both, incl. the `as_heap_value()`-is-UB caveat. CLAUDE.md's one-line summary ("no HeapValue arm") is imprecise; the ADR is right |
| §2.7.10/Q11 MethodFnV2 | **CONFORMS exactly** | §2.1 table |
| §2.7.11/Q12 value-call ABI | **CONFORMS** | §2.1 table; forbidden shim names all at baseline 0 |
| §2.7.12 SharedCell ordinal | **code right, ADR wrong** | §3.3 (ordinal 19 vs 20 contradiction) |
| §2.7.24 Q25 carrier bundle | **CONFORMS as sampled** | `TypedObjectPtr` in HeapValue arm; `TypedArrayData` deleted (W12 audit target confirmed deleted) |
| §9 LSDS primary output | **NOT MET** | transcript §2.4 |
| §13 forbidden list | **CONFORMS** | each item spot-checked: no from_heap_arc, no modal-types subsystem (no new solver crate; `mir/solver.rs` + `storage_planning.rs` extended), no runtime interning, no NaN-box files, no VM↔JIT conversion found at sampled boundaries |
| §14 success metrics | **2 of 8 verifiable ones missed, unacknowledged** | "`from_heap` callers: 0 at end of Phase 1.B" — definition + live fallback caller remain (`json.rs:383`, `#[allow(deprecated)]`, "until per-variant constructors land in Phase 2c"); "LSDS ≥95%" — §2.4. No doc records the misses |
| §15 visibility mechanisms | **CONFORMS** | markers (567), CLAUDE.md section, defections entry 2026-05-08, baseline rows |

**Amendment discipline** deserves explicit credit and one debit. Credit: 25+ §2.7.x
amendments each carry decision + rationale + rejected-alternatives + mechanical-lockstep
fanout lists (e.g. §2.7.9's 7-site fanout, all verified present in code). Debit: the one
missing amendment (§2.7.26/ModuleFn — claimed landed by the closing agent's AGENTS.md row,
never written) shows the discipline is enforced by agent conscientiousness, not by any gate:
CHECK 6 verified the ordinal tables; nothing verifies "the cited ADR section exists".

### 6.3 ADR-001 / ADR-002 / ADR-003 — **stale**

- **ADR-001** (NanBoxed canonical; VMValue boundary-only; ExternalValue enum; `cargo xtask
  vmvalue` CI gate): the entire decision stack is superseded. Neither `NanBoxed` nor
  `VMValue` nor `ExternalValue` exist as types; the guard (`cargo xtask vmvalue`) has no
  corresponding xtask module today. Status line unchanged since 2026-02-19. Rate: **stale,
  must be marked Superseded-by-ADR-005/006**.
- **ADR-002** (share one 3-bit NaN tag scheme via `shape-value/src/tags.rs`): file deleted;
  scheme forbidden; the useful surviving idea (unified `HeapHeader`) landed in mutated form
  (8-byte v2 header at `v2/heap_header.rs` vs the ADR's 16-byte `repr(C, align(16))` sketch
  with len/cap/aux inline). Rate: **stale**.
- **ADR-003** (MethodDescriptor single source): never built (§2.5). The *problem* it names
  is still live — two registries, manual sync. Unlike 001/002 this ADR isn't superseded by
  anything; it's simply unexecuted with Status: Accepted. Rate: **stale-as-status /
  still-valid-as-intent** — the most dangerous flavor, since re-reading it today would be
  reasonable and nothing warns you the estimate/landscape (e.g. "MethodFn handler" types) predates
  three ABI rewrites.

### 6.4 ADR-004: Native C Interop — **partially-stale**

Spot-checked decisions vs tree: `extern C fn ... from "lib"` syntax present in grammar and
compiler (`shape.pest` externs; `ForeignFunctionEntry` metadata); pointer-cell intrinsics
`__native_ptr_new_cell` etc. exist (`grep` hits in shape-runtime intrinsics); `out`-param
support is in CLAUDE.md's language list; JIT contract "CallForeign remains VM-managed"
matches the JIT's surface-and-stop posture. One anachronism: §4 "return is decoded directly
to `NanBoxed`" (`004:78`) — the marshal layer is now the §2.7.29 kind-threaded
`foreign_marshal` (`executor/control_flow/foreign_marshal.rs`, ADR-006-marked). Deep
conformance of marshaling rules is vertical 13's territory; as a *document*, ADR-004 needs
a one-line §4 correction, otherwise holds. Rate: **partially-stale**.

### 6.5 runtime-v2-spec.md — **self-contradictory; highest-priority doc fix in this vertical**

Covered in §5.1/§9.1. Sections "Contract", "Primitive Types", "TypedArray", "TypedStruct",
"VM Stack", "JIT", "Non-Goals" remain a fair description of the typed path and are what
CLAUDE.md means by "authoritative". Sections "Status (as of 2026-04-18)" and "Host Boundary:
ValueWord as Serialization Format" (a *later* addition, Wave E+4.5 2026-04-27) mandate the
deleted world. The file predates the defection-log era and was never revisited despite being
the most-cited spec in the estate (cited by CLAUDE.md twice, ADR-006 §2.7, every 2026-07
design doc's "Binding constraints" line).

## 7. Test coverage in-territory (enforcement coverage of the rule set)

For a docs vertical, "test coverage" = which binding rules have mechanical enforcement.

### 7.1 Coverage matrix

| Rule family | Gate | Coverage quality |
|---|---|---|
| Deleted-symbol names (ValueWord synth, dynamic fallbacks, SlotKind::Dynamic, shim renames) | baseline rows 13-38 | **good** — regex-pinned, monotonic |
| Bool-default fabrication | baseline rows 39/40/43 + sentinel test + `prove_native_kind` doc contract | **good** — triple-covered |
| Arc-op-on-`_new`-carrier UB | baseline rows 41/42 | **good** — added post-incident (three documented segfault families, verify-merge 6b header) |
| HeapKind ordinal & lockstep | verify-merge CHECK 5/6/6b/12/14 | **good** — but code-only; prose ordinal copies unguarded (§5.5) |
| Take-both merge damage | CHECK 4/7/8/9 | **good** — yet AGENTS.md itself carries un-caught damage because gates scan code, not AGENTS.md (§3.4) |
| Doc drift | CHECK 13 only (colon-return-type) | **one rule out of hundreds** — the only doc-content gate in the repo |
| ADR cross-reference integrity (cited §§ exist; Status vs reality; codebase-index pointers) | **none** | how §2.7.26, SharedCell=19, `packages/duckdb`, and 13 stale pointers survived |
| Single-discriminator (no new 1:1 HeapKind projections) | **none mechanical** | review-only; `ConcreteReturn` growth would not trip any gate |
| Q8 accessor bound | **none** | `as_typed_object_storage` survived 2 months + one prior audit flag |
| KindedSlot out of JIT slot ABI | **none** | review-only |
| defections.md logging duty | **none** (unenforceable by nature) | observed decay §7.2 |

The pattern: **name-shaped rules are gated; shape-shaped rules are not.** Every enforcement
win is a regex over identifiers; every observed violation lives in structure (an accessor's
dispatch pattern, an enum's variant set, a citation graph) that regex can't see.

### 7.2 defections.md usage measurement

Dated headings histogram (`grep -E "^#{2,3} .*2026-"`):

```
2026-04-18: 1 (reconstructed)   2026-05-08: 5
2026-05-05: 1 (reconstructed)   2026-05-13: 1
2026-05-06: 23                  2026-07-06: 1
2026-05-07: 34
```

62 of 66 entries fall in a 72-hour window. Between 2026-05-13 and 2026-07-06 — the period
covering W14-W18, Phase 2d/3, the strict-flip, v0.3.3 correctness work, and the GC build —
zero entries, while CLAUDE.md continued to mandate "Log considered-but-rejected compromises
in docs/defections.md". The practice partially migrated into ADR amendments' "Forbidden
alternatives" blocks and cluster-audit "refused candidates" sections (§4.5), which preserves
the *content* but breaks the log's stated purpose ("future sessions read this to recognize
the pattern" — nobody can read 364 files). The 2026-07-06 entry (ModuleFn name-indirection,
a *bounded deviation* honestly logged with a re-open trigger) shows the format still works
when used.

### 7.3 Sentinel & gate self-tests

- The sentinel test's needle-fragmentation is itself a regression-proofing of the gate
  (§3.2). 
- `verify-merge.sh --fast` mode documented in-script; CHECK 2's bench-gate history is
  recorded both in-script and in CLAUDE.md (consistent, verified same wording anchor
  "2026-07-05").
- No test asserts the baseline file parses (a malformed row would be skipped silently by the
  `[[ -z ... ]]` guard — a limit typo'd to empty string would silently drop a pattern). Minor.

### 7.4 In-code tests referenced by governance docs

- `prove_native_kind` SB-8b tests exist (`type_tracking.rs:1365-1447`+) and assert
  no-pass-through, rejection of SB-10 (UInt64-for-HashMap) and SB-12 (Bool-for-Null) lies —
  matches the doc narrative. But production wiring is a single call site
  (`compiler/helpers_binding.rs:585`); the mechanical-enforcement story CLAUDE.md tells
  ("emit code cannot fabricate 'I proved it'") is true of that one funnel while most typed
  emission still flows through the tracker the baseline marks for replacement
  (`last_emitted_native_kind`, limit 8).
- CLAUDE.md-cited regression test `constraints.rs:1193` (`BuiltinTypes::function()`
  preserves TypeVars) — not re-verified here (type-system vertical 02 territory).

## 8. Book / docs vs reality for this vertical

### 8.1 Book vs ADR-006

The book (`shape-web/book/book-site/`) never mentions ADRs (grep "ADR" over content → 0) —
correct choice for user docs. But where the book states the memory model it states the
**pre-ADR-006 design**: `fundamentals/variables.mdx` llm_summary + line 12: "`var` is shared
with reference counting", "`var` uses shared ownership with copy-on-write". ADR-006 §1.2's
whole point is that `var` *infers* Direct/UniqueHeap/SharedCow/SharedAtomic* and is usually
**not** refcounted ("Refcount on escape, not mutability", §3.3). Today the book matches the
*implementation* (always-SharedCow `var`) and contradicts the *ratified ADR*. Whichever way
§1 eventually resolves, one of {book, ADR-006 §1, code} must move; currently all three are
mutually inconsistent (code has no SharedAtomic*; ADR has inference; book has always-shared).

### 8.2 CLAUDE.md vs measured reality

| CLAUDE.md claim | Reality |
|---|---|
| "strict typing enforced, no fallback" | **TRUE** — transcripts §2.3 |
| "prove_native_kind() ... in compiler/type_tracking.rs. The Rust type system enforces this" | file is `shape-vm/src/type_tracking.rs` (no `compiler/` prefix); enforcement exists but guards one call funnel (§7.4) |
| NativeKind variant list (11 named incl. `Unit`) | 30 variants; **no `Unit` variant exists**; list omits Float32/Char/all Nullable*/widths (`native_kind.rs:32-110`) |
| "Sentinel test ... asserts forbidden symbols are absent" | asserts exactly one pattern |
| `BindingStorageClass (type_tracking.rs:286)` | :359 |
| `packages/duckdb/` example | no such package; `packages/xgboost` only |
| "15 checks as of 2026-07-05" (verify-merge) | **TRUE** |
| "shape-types = empty crate skeleton, don't look here" | **TRUE** — model documentation of a trap |
| Known-Constraints list (v2-raw residuals, ignored-test inventory) | spot-checks consistent; e.g. the 4 `simulation.rs` ignores and ~23 shape-jit ignores framing matches `just test-all` doc |

### 8.3 README/book claims about governance

Nothing in the book or README claims the governance system itself (no "we use ADRs" outward
claim), so no outward-facing overclaim exists. The overclaims are all *inward*: docs
describing docs (runtime-v2-spec "authoritative", AGENTS.md citing phantom amendments,
justfile TODO claiming the sentinel doesn't exist).

## 9. Bugs & correctness risks found

Severity scale per brief: P0 unsound/wrong-results/security, P1 broken feature, P2 paper cut.
For a governance vertical, "unsound" = a doc defect that can *cause* unsound code or nullify
the defense system.

1. **P0(governance) — runtime-v2-spec.md mandates the forbidden world.**
   `runtime-v2-spec.md:3` ("Authoritative — all implementation must conform") vs `:17`
   (ValueBits shim + `exec_arithmetic_dynamic_fallback` as accepted architecture), `:33-34`
   ("V5.7 conversions.rs FFI-boundary helpers retained for the dynamic-fallback bridge" —
   a CLAUDE.md refuse-on-sight family phrase), `:362-396` (deleted
   `synthesize_value_word_from_raw` + `SlotKind::Dynamic/Unknown` documented as current
   API). Failure scenario: any fresh agent tasked "implement per spec" reintroduces the
   canonical defection with documentation on its side; CLAUDE.md itself says the last such
   rename cost 4-6 weeks. The repo's defense today is that CLAUDE.md contradicts the spec
   and agents read CLAUDE.md first — an ordering accident, not a design.
2. **P1 — Q8 accessor violation, live, growing, unamended.** `kinded_slot.rs:762`
   `as_typed_object_storage` vs ADR-006 §2.7.6/Q8 explicit bound; ≥21 call sites; flagged
   2026-07-04, no action, no amendment, no defections entry. Either amend Q8 (the honest fix
   — the accessor exists because `as_heap_value()` is unsound on v2-raw TypedObject carriers)
   or delete the accessor. The current state teaches agents that Q8 is aspirational.
3. **P1 — phantom ADR citation for a shipped HeapKind.** `HeapKind::ModuleFn=33` justified
   by "ADR-006 §2.7.26 amendment landed" (AGENTS.md roster + ordinal table row 801); section
   never written. All future ModuleFn work has no ruling text to conform to; the ordinal
   table's authority column is now partially fictional.
4. **P1 — ADR-006 teaches a wrong dispatch ordinal 3×.** SharedCell=19 at `006:226,2383,2428`
   vs code Reference=19/SharedCell=20 (`heap_variants.rs:127-144`). An agent hand-writing a
   serializer or FFI table from ADR text (the ADR is cited as authoritative over the index)
   would corrupt Reference/SharedCell dispatch — the exact wrong-destructor UB class §2.7.9
   exists to prevent.
5. **P1 — ADR-001/002 Status: Accepted on the forbidden architecture** (§5.2, §6.3). One-line
   Superseded banners fix it.
6. **P1 — ADR-003 accepted-but-never-built with live drift risk it was written to kill**
   (§2.5). Needs a Status decision: implement, re-scope, or reject.
7. **P1 — ADR-006 §1 / book / code three-way contradiction on `var`** (§8.1, §2.2).
8. **P2 — CONTEXT.md untracked.** `git status` → `?? CONTEXT.md` while CLAUDE.md's §Domain
   docs names it the canonical single-context file. One `git add`. Also ~60 untracked
   `wave*.md` cluster-audits — working state invisible to any other clone.
9. **P2 — `last_emitted_native_kind` tracker still live (limit 8)** while the baseline note
   says "replace with prove_native_kind"; combined with the single `prove_native_kind` call
   site this means the proof story is thinner than CLAUDE.md §Mechanical-enforcement
   implies. (Code-side depth belongs to vertical 03; the governance defect is the
   overstated narrative.)
10. **P2 — justfile `verify-phase-5` stale TODO** (`justfile:193-197`) claiming the sentinel
    isn't wired; it is (`executor/tests/mod.rs` includes `no_dynamic`). The recipe passes
    while doing less than it advertises.
11. **P2 — 2026-04-18 planning docs advertise "Alignment: 100%" for the deleted world**;
    worst instance `v2-nanbox-removal-plan.md:18` — "**Step 6 — ValueBits shim: LANDED**",
    i.e. the repo's canonical defection is still presented as a success story with no
    banner pointing at the May correction. `docs/vision/*` (2026-02-13) similar
    (`p0-p11-module-capability-roadmap.md` still "Status: Active" for a VMValue-retirement
    plan).
12. **P2 — codebase-index content rot** (§5.4): "constructors not yet implemented" (they
    are), struct shapes 2 generations old, deleted `TypedArrayData` entry, dead-code list ×4.
13. **P2 — dead-gated `#[cfg(any())]` test modules referencing deleted APIs** (§3.6): the
    permanent-"until follow-up" shape the Forbidden-Patterns preamble warns about, in test
    form.
14. **P2 — AGENTS.md structural corruption + stale rows** (§3.4).

## 10. What is done well

- **The monotonic baseline gate design** (`check-no-dynamic.sh`): per-symbol ratchet with
  "once 0, forever 0", doc-trees deliberately out of scope with the reason written down.
  This is the best anti-backslide mechanism in the estate and it demonstrably works
  (EXIT=0; forbidden shapes stayed dead through ~2 months of heavy churn).
- **verify-merge.sh as executable postmortem**: every check traces to a named incident
  (grep-c false-green; take-both misses; JIT retain/release segfault families with three
  named precedents in the 6b header). Institutional memory in the only place agents can't
  skip.
- **ProofGap's seal type** — using Rust module privacy so "emit code cannot fabricate a
  pass" is a compiler-checked property of the *governance* mechanism itself.
- **ADR-006 amendment anatomy**: decision + rationale + rejected alternatives + refuse-list +
  mechanical fanout enumeration (§2.7.9 is a model — 7-site lockstep list, all present in
  code). Also honest supersession *inside* the doc when practiced (§2.7.22 SUPERSEDED tag).
- **The defections.md format** and its best entries (N7/N9 dispositions; the 2026-07-06
  ModuleFn entry logging a *taken* deviation with an explicit re-open trigger). The genre —
  "log the rationalization while it's fresh" — is rare and valuable.
- **Marker-comment discipline at scale**: 567 exact `// ADR-00x` markers; sampled markers
  sit on the actual load-bearing lines (TypedFieldValue block, ConcreteReturn header,
  slot.rs constructor block), not decorative headers.
- **TypedFieldValue** as an exhibit of a rule surviving contact: the String exception stayed
  bounded (no second exception found anywhere), two months and many waves later.
- **Honest trap documentation** in CLAUDE.md (shape-types "do not look here"; `format()`
  shadowing; the deep-tests SIGILL note) — the anti-pattern of most CLAUDE.md files (feature
  advertising) is mostly avoided in favor of hazard marking.
- **CHECK 13** — a *doc*-drift gate wired into the merge path; the only one, but the pattern
  is proven and cheap to extend (§12 P1-3).
- **The 2026-07 design lane's "Binding constraints" headers** — new design docs explicitly
  subordinate themselves to canon, by section number.

## 11. What is done poorly / tech debt

- **No supersession hygiene.** Nothing in the estate marks a doc dead. Five classes of
  reader-facing wrongness (§9.1, 9.5, 9.11) are all the same missing one-line banner. The
  estate has an append-only culture without the corresponding tombstone culture.
- **ADR-006 as an infinite file.** 7.5k lines, amendments physically scattered into §12/§15/
  post-§17, duplicate and out-of-order section numbers, a lost Q3, a phantom §2.7.26 —
  navigation is now expert-only. The document needs a structural re-shard (per-amendment
  files + a generated index), which its own append-only merge-conflict rationale
  (defections.md preamble) actually supports.
- **Citation integrity unverified.** Rulings are cited by section number hundreds of times
  (547 markers + AGENTS + audits) and nothing checks the target exists — the cheapest
  possible gate (grep the cited anchor) doesn't exist, and §2.7.26 proves the cost.
- **The navigation layer has no refresh protocol.** codebase-index was built in one
  2026-05-08 three-agent pass and rots since; entries carry no as-of commit, so a reader
  can't tell "verified recently" from "archaeology".
- **defections.md decayed into shards** (§7.2, §4.5) — the practice lives, the log lies
  by omission.
- **AGENTS.md conflates registry with archive.** 9,000-word closed rows from May sit between
  live July rows; its own 500-line policy exempts it "until closed historical rows are
  archived out" — an unexecuted promise from May.
- **cluster-audits/ is write-only memory.** 364 files / 112.8k lines, no index, ~60
  untracked; CLAUDE.md links 6 of them as binding entry points, and prior-audit experience
  (this file's brief included) is that they're "leads but often STALE" — there is no way to
  tell which except re-verification.
- **Metrics without follow-up.** ADR-006 §14 defines 8 success metrics "so we measure rather
  than rationalize"; no measurement report exists for any of them; two verifiable ones are
  missed silently (§6.2).

## 12. Prioritized recommendations

**P0 — stop the authoritative-spec inversion (≤1 day)**
1. `runtime-v2-spec.md`: delete or banner the "Status (as of 2026-04-18)" and "Host
   Boundary: ValueWord as Serialization Format" sections ("SUPERSEDED 2026-05 strict-typing
   bulldozer — see CLAUDE.md §Forbidden Patterns, ADR-006 §2.7.7"); keep the Contract/layout
   sections. Re-stamp the header with an as-of commit.
2. Add Superseded banners: ADR-001, ADR-002 ("Superseded by ADR-005/ADR-006 §2.1 —
   NaN-boxing is a forbidden pattern"), `docs/vision/*` and the 2026-04-18 planning docs
   (one-line header each; especially `v2-nanbox-removal-plan.md` whose Step-6 "LANDED" text
   is the canonical defection presented as success).

**P1 — restore rule/citation integrity (≤1 week)**
1. Write the missing ADR-006 §2.7.26 (ModuleFn) from the AGENTS.md close-out row (the
   content already exists there); fix SharedCell ordinal in §2.3/§2.7.12; renumber or
   anchor-alias the scattered amendments; restore/officially-retire Q3.
2. Rule on `as_typed_object_storage`: amend Q8 (bounded per-raw-carrier borrow accessors
   for `_new`-carrier kinds, named and justified — the ADR-005-§2-style shape) or delete it.
   Log either way in defections.md.
3. Extend CHECK 13's pattern into a CHECK 15 "citation integrity": every `§2.7.\d+` cited in
   AGENTS.md/code markers must match a heading in ADR-006; every ADR Status line must be one
   of Accepted/Superseded(by)/Rejected. ~30 lines of bash in the existing style.
4. Decide ADR-003 (implement / re-scope / reject) and mark it.
5. `git add CONTEXT.md` (+ triage the untracked cluster-audits).
6. Fix CLAUDE.md factual errors (NativeKind list, prove_native_kind path, sentinel plural,
   packages/duckdb, the three stale line numbers) and the justfile verify-phase-5 TODO
   (wire the sentinel: `cargo test -p shape-vm --lib no_dynamic`).

**P2 — decay control (ongoing, cheap)**
1. Add as-of commit stamps to codebase-index entries touched by any wave; delete
   `01/02/03-dead-code-suspects.md`; fix the §5.4 content-rot entries.
2. Archive AGENTS.md closed rows to `docs/agents-archive/`; repair the fused heading.
3. Re-affirm the defections.md duty or officially replace it with "ADR amendment
   Forbidden-alternatives blocks + quarterly digest"; the current ambiguity is worse than
   either choice.
4. Delete the `#[cfg(any())]` ValueWord-referencing test modules (rewrite tickets exist in
   their own doc comments); drop the corresponding baseline limits toward 0.
5. Reconcile ADR-006 §1 / book / implementation on `var` semantics — pick the model, banner
   the losers, and record the §14 metric misses.

## Appendix A — full baseline-gate state at working tree

Recomputed each row with the gate's own counting function (same `rg -c -P` + awk sum over
`crates bin tools extensions`):

| Limit | Actual | Note (from baseline) |
|---|---|---|
| 12 | 12 | W-series ValueWord synthesizer (deleted) |
| 0 | 0 | W-series runtime return-kind stamp |
| 1 | 1 | W-series persistence normalizer |
| 5 | 5 | dynamic arithmetic fallback handler |
| 0 | 0 | dynamic comparison fallback handler |
| 0 | 0 | W4-d Convert<X>To<Y> opcode pattern |
| 0 | 0 | W-series typed→tagged rebox helper |
| 8 | 8 | sparse kind tracker (replace with prove_native_kind) |
| 12 | 12 | closure capture deletion progress |
| 17 | 17 | NanBox residuals |
| 0 | 0 | deleted SlotKind variants |
| 0 ×9 | 0 | rejected-rename literals (ValueBits shim, FFI-boundary bridge, …) |
| 0 ×3 | 0 | rename regex families (tag-decode / decoder-bridge / synthesis-bridge) |
| 0 ×4 | 0 | W7 value-call family + transitional shim names |
| 0 ×2 | 0 | Bool-default fabrication shapes (§2.7.7) |
| 0 ×2 | 0 | R6 Arc-op-on-`_new`-carrier UB shapes |
| 0 | 0 | W17 Stage-0 upvalue Bool-default |

Two readings. Positive: **every zero row is still zero** — no forbidden shape regressed
through two months of heavy churn; the ratchet holds. Negative: **every nonzero row equals
its limit exactly** — zero deletion progress on any residual since the limits were last
lowered. The five nonzero families (`synthesize_value_word_from_raw` comments ×12,
`exec_arithmetic_dynamic_fallback` comments ×5, `normalize_persisted_for_slot` ×1,
`last_emitted_native_kind` ×8 live, `capture_as_value` ×12, NanBox residuals ×17) are
plateaued, and because comment-mentions and live code count the same, the baseline can't
distinguish "12 tombstone comments" (fine forever) from "8 live tracker uses" (the thing
the note says to replace). A `live-code-only` second column — or just deleting the
tombstone-comment mentions and dropping limits to true-live counts — would restore signal.

## Appendix B — additional empirical probes of documented rules

Numeric-conversion rule (explicit `as` cast, D2 truncation — CLAUDE.md/memory):

```
$ shape run conv1.shape           # let n: number = 3.7 ; let i: int = n as int
[jit-fallback] function main failed JIT compile: ... vm_only_opcodes: [ConvertToInt] ...; running under interpreter
3
```

Correct value (truncate 3.7→3) — and a governance-relevant surprise: the deopt banner is
printed to the *user* on a plain `shape run`, exposing internal preflight vocabulary
(`JitPreflightReport`) on a documented-and-supported language feature.

Bare unparameterized generic (memory: "Option/HashMap/Array exist ONLY in `<T>` form"):

```
$ shape run bare_generic.shape    # let x: Option = None
error[RUNTIME]: ... Generic { base: Concrete(Reference(TypePath { segments: ["Option"], 
qualified: "Option" })), args: [Variable(TypeVar("T62"))] } is not compatible with Option
```

Rule enforced — via an error message that leaks the raw internal `Type` debug
representation. Against ADR-006 §9's LSDS witness model ("expected/found type witnesses"),
this is the distance still to travel: the *check* is right, the *diagnostic* is a Rust
`Debug` dump.

Let-generalization (user-ruled 2026-05-31, CLAUDE.md-adjacent memory):

```
$ shape run letgen.shape          # fn get_none() { None } ; let a: Option<int> = get_none()
[jit-fallback] ... Route A surface-and-stop: SURFACE — direct call to `get_none` ... has no
compile-time-proven FrameDescriptor.return_kind ... no runtime inference or Null fallback. 
ADR-006 §2.7.5.; running under interpreter
letgen ok
```

Two governance observations: (a) the surface-and-stop discipline is genuinely wired — the
JIT refuses to lower without a kind proof and *cites the ADR section* rather than guessing
(exactly the anti-Bool-default behavior §2.7.8 demands); (b) an ADR citation is printed to
end users on a supported feature via the fallback banner — governance vocabulary has leaked
into the product UX. The discipline is right; the reporting channel isn't.

## Appendix C — the audit-resolution tracker (governance mechanism not covered above)

`docs/cluster-audits/audit-2026-07-04-resolution-status.md` (119 lines) is a live tracker
mapping the 2026-07-04 claimed-vs-real audit's 29 confirmed crit/high findings to current
state, with a verification-provenance legend that distinguishes **who** verified each row
("`Fable` = independent-model reproduction; `refuter` = fixing workflow's adversarial
verifier; `claimed` = workflow reported fixed, NOT independently re-verified"). Rows carry
merge hashes, refuted-then-fixed history (e.g. #10 async overlap: regression caught by
supervisor differential, then gated), and honest 🟡/🏗 partials (#31 GC row documents a
*blocked* JIT barrier with the discovered unsoundness spelled out).

This is the strongest single governance artifact in the estate — claim-provenance tracking
at finding granularity. Two gaps, both consistent with this report's theme:

- **Coverage boundary**: it tracks the audit's §6 numbered findings; the audit's
  *inconsistency list* items (same file, `claimed-vs-real.md:145-157`) got no rows — which
  is precisely where item 11 (`as_typed_object_storage` Q8 violation) has sat unactioned
  since 2026-07-04 (`grep -n "as_typed_object\|Q8" audit-2026-07-04-resolution-status.md`
  → no tracker row).
- **Reachability**: it lives among 364 cluster-audit files and isn't linked from CLAUDE.md's
  entry-point list, so its survival as a practice depends on the current supervisor
  generation remembering it exists.

## Appendix D — design/rfcs/vision inventory with staleness ratings

| Doc | Date | Status header | Audit rating |
|---|---|---|---|
| `design/00-priority-spine-overview.md` | 2026-07-05 | RATIFIED (user), 54 defaults + Q13 override | current |
| `design/comptime-excellence.md` | 2026-07-05 | RATIFIED, rev 2 post-adversarial-review | current |
| `design/snapshot-resume.md` | 2026-07-05 | RATIFIED | current |
| `design/real-gc-cycle-collection.md` | 2026-07-07 | DESIGN — RATIFIED, rejected-option named | current |
| `design/ffi-rebuild.md`, `polyglot-distributed-integration.md`, others | 2026-07 | per-doc | current (headers carry binding-constraints lines — §5.7) |
| `rfcs/001..008` | all 2026-05-18 | Draft | dormant-honest (labeled Draft; no false claims; several — CARS graveyard, semantic-graph DB, realtime LLM channel — are far-horizon) |
| `vision/distributed-comptime-async-vision.md` | 2026-02-13 | "Design Decisions (Final)" | **stale** — pre-strict-typing decisions presented as final |
| `vision/implementation-plan.md` | 2026-02-13 | none | **stale** |
| `vision/p0-p11-module-capability-roadmap.md` | undated | "Status: Active" | **stale** — an *Active* VMValue-retirement roadmap for a type that no longer exists |
| `vision/rfc-borrow-lifetimes-ergonomics-v1.md` + 3 others | 2026-02..04 | mixed | stale-to-superseded (borrow/scoping ground now ruled by ADR-006 §1/§3) |

The gradient is clean: docs authored under the post-2026-07-05 discipline (ratification
record + binding-constraints header + adversarial-review rev note) are trustworthy on their
face; everything authored before the May bulldozer needs a banner; the 2026-04-18 "Alignment:
100%" family is actively misleading (§9.11). The estate knows how to write good docs *now*
— it hasn't gone back to disarm the old ones.

## Appendix E — methodology

- Read fully: all 6 ADRs, runtime-v2-spec.md, CLAUDE.md, check-no-dynamic.sh + baseline,
  no_dynamic.rs, codebase-index.md; structurally (headings + targeted sections): ADR-006
  (~1,200 of 7,536 lines read verbatim), defections.md, AGENTS.md, verify-merge.sh,
  CONTEXT.md, 12 planning docs, 10 design/rfcs/vision heads, resolution-status tracker.
- Conformance greps: ValueWord/ValueBits/synthesize_*/exec_*_fallback definitions vs
  mentions; `// ADR-005|006` markers; from_heap_arc; `Box<HeapValue>`; MethodFnV2;
  KindedSlot API surface; prove_native_kind call sites; MethodDescriptor;
  SharedAtomic variants; NativeKind/HeapKind/HeapValue variant sets; 16 file:line pointers
  resolved.
- Executed: `scripts/check-no-dynamic.sh` (EXIT=0) + per-row recount; 8 Shape programs via
  the prebuilt working-tree binary (strict1-3, override, lsds, conv1, bare_generic, letgen)
  — transcripts inline at §2.2-2.4, Appendix B.
- Not exercised (out of scope / budget): cargo test runs (sibling verticals own code-side
  suites); JIT codegen-level slot-ABI verification (vertical 05); marshal-rule depth
  (vertical 13). No project file modified; only this report written.

*Report ends. Sibling reports cover code-side depth of the JIT (05), VM (04), and polyglot
marshal (13) surfaces named here.*

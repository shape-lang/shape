# Shape Language — Whole-Project Deep-Dive Synthesis

**Date:** 2026-07-12
**Scope:** Distillation of 19 vertical audit reports (21,923 lines) covering the entire `shape/` workspace plus the `shape-web`, `shape-registry`, `shape-app`, `shape-mcp`, `shape-infra`, and `packages/` satellites.
**Method:** Each vertical was audited by an independent agent that read the code, ran programs against the freshly-built working-tree binary, checked ADR conformance rule-by-rule, and had its report adversarially critiqued (and amended where it failed). This document is the cross-cutting synthesis; the per-vertical reports (`01`–`19`) hold the full evidence.

> **Working-tree note:** the tree is dirty with in-progress work (GC-default flip, W17 snapshot, strict-flip campaign). Everything below describes the working tree as audited, not the last tag (v0.3.2). All empirical claims were run against a debug binary built from this tree.

---

## 0. The one-paragraph verdict

Shape is a genuinely ambitious, genuinely substantial statically-typed language (~717k lines of Rust across 12 crates + tooling) whose **core engineering is real and often excellent** — the strict-typing spine is enforced, the MIR borrow solver is a real Datafrog NLL solver, the Bacon-Rajan cycle collector bounds RSS on its flagship workload, snapshot/resume and polyglot×distributed execution actually work end-to-end, and the anti-drift enforcement machinery (forbidden-symbol gate, merge ratchets, vmjit-diff) is better than most production compilers. But the project has **one systemic fault that dominates everything else: the shipped default execution mode (JIT) is a different, less-safe implementation of the language than the mode the tests exercise (VM), and the feedback loop that would have caught the divergence is severed** — local `main` is 1,872 commits ahead of the CI that still validates a 6-week-old snapshot. The result is a language whose *interpreter* is trustworthy and whose *default binary* segfaults or silently returns wrong answers on ordinary programs, while the documentation describes a third language that partly doesn't exist. The gap between "the code exists and is well-built" and "the shipped default works end-to-end" is the entire story of this audit.

---

## 1. Scorecard

Feature-completeness and code-quality scores are the auditors' own (0–100), assigned per vertical against that vertical's own ambition.

| # | Vertical | Compl. | Qual. | One-line state |
|---|----------|:---:|:---:|----------------|
| 01 | Parser, grammar & AST | 78 | 62 | Modern surface parses well; dead trading-DSL grammar + edge crashes |
| 02 | Type system & inference | 68 | 72 | Strict spine real; one soft-check unsoundness; features dead via split-brain |
| 03 | Bytecode compiler & MIR | 82 | 62 | Sophisticated MIR; overstated enforcement; live float-truncation hole |
| 04 | VM interpreter & dispatch | 78 | 72 | Disciplined, ADR-clean, correct — one closure-carrier split-brain latent |
| 05 | **JIT (Cranelift)** | **38** | 72 | Large AOT compiler documented as a tiered JIT that does not run |
| 06 | Value / memory / GC | 82 | **88** | Best-built vertical; GC leaks the *ordinary* user cycle shape |
| 07 | Runtime stdlib & builtins | 72 | 80 | Native core strong; three-way method-table drift; capabilities inert-by-default |
| 08 | Core language semantics | 72 | 63 | Interpreter excellent; **default JIT mode memory-unsafe** |
| 09 | Async & concurrency | 55 | 70 | Real but sharply bounded; isolated-VM globals read as zero |
| 10 | Comptime & annotations | 58 | 74 | Core landed; falls off a cliff at module boundaries |
| 11 | Security & capabilities | 58 | 72 | Engine works where wired; config surface fails open |
| 12 | Snapshots & distributed | 78 | 82 | Strong; refutes the "dead stub" history end-to-end |
| 13 | Polyglot & FFI | 58 | 78 | Scalar core real; documented flagship shapes runtime-refused |
| 14 | Tooling & DX | 78 | 74 | Broad and real; diagnostics render worse than infra supports |
| 15 | Testing & quality infra | 72 | 74 | Elaborate; **feedback loop severed (stale CI, gates can't fail)** |
| 16 | Ecosystem (registry/app/MCP) | 68 | 74 | Each piece clean; each pinned at a different language epoch |
| 17 | Book vs reality | 82 | 78 | 80.6% fence-truth (up from 47%); exclusions still hide failures |
| 18 | ADRs & governance | 62 | — | Enforcement excellent; document layer inverted/eroding |
| 19 | Cross-cutting duplication | — | 68 | No live forbidden code; lockstep-by-comment debt |

**Reading the spread:** the *values* subsystem (06) and the *bytecode/MIR* subsystem (03) are the quality peaks; the *JIT* (05) is the trough (38 completeness) because it is documented as something it is not. The clustering of async / comptime / polyglot / security-config all at 55–58 completeness is not coincidence — they share the same failure mode (§3.4, the compile-accepted/runtime-refused belt).

---

## 2. Unified P0 register — the defects that actually matter

Across the 19 reports there are ~15 P0-tagged findings. De-duplicated and severity-ranked across the whole project, these are the ones that would block a serious release. **The top cluster is all one root cause: the default mode is JIT and the JIT is unsound.**

### Tier A — memory-unsafe / wrong-results in the DEFAULT shipped mode

| # | Defect | Evidence | Report |
|---|--------|----------|--------|
| A1 | **Conditional `break`-with-scalar from a value loop SIGSEGVs** (exit 139, no diagnostic) under default `--mode jit`; gdb shows `jit_arc_string_retain(bits=7)` — an `int` dereferenced as `Arc<String>`. `--mode vm` fully correct. | gdb transcript | 08 |
| A2 | **Array/object match patterns lose ALL refutation checks under JIT** — `[x]` matches `[1,2,3]`, `{x:0,y:0}` matches `{3,4}`; raw scrutinee pointer used as the switch condition. | `mir/lowering/expr.rs:1915-1917` | 08 |
| A3 | **`arr.slice(1,3)` returns corrupted length, `arr.sort()` runtime-errors** under JIT; both correct under VM. | run transcripts | 08 |
| A4 | **Book's first DateTime fence prints a wrong result then SIGSEGVs** under JIT (also L364/L404); VM fine; all hidden behind `runnable=false`. | `fundamentals/datetime.mdx:19` | 17 |
| A5 | **Live VM-vs-JIT wrong-result divergence**: a HOF return prints `6.0` (vm) vs `4618441417868443648` (jit), both exit 0. Pinned known-red, still shipping. | `known-red.json`, WF-3A | 19 |
| A6 | **Duration literals silently wrong under JIT** — `print(1.5d)` gives JIT `0` vs VM `PT129600S`, exit 0, no fallback. | run transcript | 05 |

### Tier B — unsoundness / corruption reachable in the VM too

| # | Defect | Evidence | Report |
|---|--------|----------|--------|
| B1 | **`HashMap<string,int>.set("a","oops")` compiles** (soft `CheckMode::Synth`); a later typed read does pointer arithmetic on the string's heap pointer (prints `106644639314833`). *Annotating the map is what opens the hole* — the exact pointer-reinterpretation class the strict flip existed to kill. | `bidirectional.rs:483`, `expressions.rs` | 02 |
| B2 | **Float range endpoints silently truncate** — `for i in 0.9..3.9` iterates 0,1,2 via **live `NumberToInt` emission** — the exact opcode family CLAUDE.md forbids. | `compiler/loops.rs:74,85,1688,1699` | 03 |
| B3 | **`comptime { [1,2,3] }` silently embeds garbage** (prints `33D`, dies on `Ptr(Decimal)`) via an unsound `as_heap_value()` read on v2-raw TypedArray bits; the existing test can't catch it (never reads the array back). | `comptime.rs:1551` | 10 |
| B4 | **`CallFrame.closure_heap_bits` carrier split-brain** — live calls store `Arc<HeapValue>` bits, snapshot-restore stores a raw closure-block ptr under the *same* kind label, teardown/introspection assume opposite shapes. Runs silently on **every stdlib module-fn dispatch**; becomes deterministic heap corruption when the release-blocking closure-snapshot work lands. | `call_convention.rs:1126` vs `snapshot.rs:527` | 04 |
| B5 | **Deferred async tasks run on an isolated VM with uninitialized globals** — `GLOBAL=100; task computes GLOBAL+1 → prints 1`, no diagnostic. | run transcript | 09 |

### Tier C — security / governance P0s

| # | Defect | Evidence | Report |
|---|--------|----------|--------|
| C1 | **`shape.toml` dotted-key `fs.write = false` is silently ignored** (serde-rename vs TOML dotted-key collision, no `deny_unknown_fields`) — write stays OPEN with no warning. Only the quoted `"fs.write" = false` form enforces. | permissions parsing, §9.1 | 11 |
| C2 | **All CI signal is 6 weeks stale** — local `main` is 1,872 commits ahead of `origin/main` (frozen at v0.3.2). The GC flip, W17, and the entire strict-flip campaign have **zero CI history**; releases shipped from red CI. | git rev-list | 15 |
| C3 | **`runtime-v2-spec.md` — labeled "Authoritative, all implementation must conform" — mandates the FORBIDDEN world** (ValueBits shim, `exec_arithmetic_dynamic_fallback`, `synthesize_value_word_from_raw`, `SlotKind::Dynamic`). A documentation-led path straight back to the repo's canonical 4–6-week defection. | doc header + body | 18 |
| C4 | **shape-app playground/notebook executes untrusted Shape unsandboxed** — `ShapeEngine::new()` with no `PermissionSet` (=all allowed) and no instruction cap; demonstrated arbitrary read of `/etc/hostname`. Mitigated only by systemd hardening. | `stdlib_cache.rs` | 16 |

**The unifying observation:** A1–A6 would all have been caught by a VM-vs-JIT differential gate — which exists (`vmjit-diff`, three harnesses) but whose corpus is stale and whose fuzzer is compiled out (`#![cfg(any())]`), and whose CI never runs on current commits (C2). The defect *class* is known (a fossil record of ~12 past silent divergences); the *detector* was allowed to rot. This is the whole project's pathology in miniature: good machinery, severed feedback.

---

## 3. Systemic cross-cutting patterns

These are the patterns that recur across ≥3 verticals. They matter more than any single finding because they predict where the *next* bug will be.

### 3.1 The VM/JIT split-brain (the master pattern)

The language is implemented **twice** — once as the `shape-vm` tree-walking-ish typed interpreter, once as the `shape-jit` Cranelift lowering — and the two have **drifted into different languages**, with the *default* binary running the less-safe one.

- **What "JIT" actually is:** not the tiered JIT the docs describe. `TierManager` is never constructed in production (`init.rs:82` is `None` with no `Some` assignment anywhere); `compile_single_function`/`compile_optimizing_function` are `Err("deprecated")` stubs; OSR, deopt, feedback speculation, and two `JitCodeCache`s are dead mass (~5k LOC). The **one live path** is default-mode whole-program selective AOT via MirToIR — and it is genuinely fast (20.8× measured on a hot loop) and correctness-*honest* via 11+ whole-program deopt gates (05).
- **But the gates leak.** They route closures, `?`, `??`, `as`-casts, traits, `Ok/Err/Some` destructuring, object literals, globals-read-in-functions, `impl Drop`, and async to the interpreter — so *idiomatic* Shape essentially never JITs (05, 08). The programs healthy enough to JIT are then the ones that hit the *un-gated* holes: A1–A6.
- **Dual heap-header layouts** compound it: JIT `UnifiedValue` (kind@0/refcount@4) vs VM `HeapHeader` (refcount@0/kind@4), discriminated by convention only; `read_heap_kind` on the wrong shape reads refcount bits as kind (05). 23 of 36 HeapKinds ride a *frozen legacy* retain/release fallback in the JIT that has already produced 3 documented segfault families (19).
- **The tests can't see any of it** because the harness defaults `ExecMode::Vm` while the binary defaults JIT (08). A test can pass in CI while the shipped binary gives the opposite answer (`t57_match_array_length_mismatch`).

**Implication:** either the JIT must reach VM parity (large) or the default must flip to VM until it does (small, and the correct interim move). The current state — ship the unverified mode by default, test the verified one — is indefensible for a language that markets soundness.

### 3.2 The three-layer type-checking split-brain

Type information is computed in one place and **thrown away** in another. The inference engine (`type_system/inference/`) computes facts the bytecode compiler's *independent* typing pass (`compiler/.../binary_ops.rs`) re-derives and contradicts.

- **Flow narrowing** is implemented and unit-tested at `inference/statements.rs:820-905` but discarded by `binary_ops.rs:240`; the `null` spelling doesn't even parse (02, 08). **Dead end-to-end despite being coded and tested.**
- **`instanceof` union narrowing**, **closure return-type checking**, **Option/Result match exhaustiveness** — same engine-vs-compiler gap (02).
- The **method tables are a three-way version of the same disease** (07): checker `method_table.rs` ↔ runtime PHF `method_registry.rs` ↔ `*_methods.shape`, drifting *both* directions — methods that type-check then crash (`string.isEmpty()`, `HashMap.keys()`), and working runtime methods the checker rejects (`number.floor()`, the *entire* Option/Result method surface). No cross-validation test exists.
- Registry-vs-checker again in the VM: `RANGE_METHODS` PHF has 10 methods, all rejected by the compiler (04).

**Implication:** there is no single source of truth for "what type does this expression have" or "what methods does this type have." Every feature that depends on inference facts surviving to codegen is at risk. A shared, tested type/method oracle consumed by *both* passes is the structural fix.

### 3.3 The soft-check / analysis-bypass unsoundness class

The strict-typing *spine* is real (this is the project's headline achievement — see §4), but it is enforced by the **constraint solver**, and several emit-tier paths trust the solver instead of re-proving.

- **`prove_native_kind` — the mechanically-enforced proof obligation — covers 3 of ~1,102 emit sites** (03, 18). The `ProofGap` seal is real and sound *where used*; most typed emission flows through the baseline-pinned `last_emitted_native_kind` tracker the baseline itself marks "replace with prove_native_kind."
- `plan_coercion` accepts lossy `Int+Number`/`u64+number` at the emitter tier, guarded only by the upstream solver — **any analysis-bypassing entry path inherits the hole** (03). B1 and B2 are exactly this: a soft `Synth` check and a direct `NumberToInt` emission that the solver didn't stop.

**Implication:** "the strict flip is enforced" is true at the *whole-program* level for programs that go through the solver, and false at the *emitter* level for any path that reaches codegen another way (annotated generics, f-string interpolation, comptime array embedding). The enforcement narrative in CLAUDE.md is stronger than the mechanism.

### 3.4 The compile-accepted / runtime-refused belt (and the `op_new_array(0)` landmine)

Across async, comptime, polyglot, and stdlib, a large surface **compiles cleanly and then aborts at runtime** on a "SURFACE" / `NotImplemented` stub. This is the shared failure mode behind the 55–58 completeness cluster.

- Polyglot: `cslice/cmut_slice/cview/cmut/callback/cstring?` all compile, all refused; **every out-param `extern C fn` with a non-void return aborts** on `op_new_array(2)` — and that is the shipped `duckdb` package's dominant shape and the book's DuckDB example (13).
- Stdlib: `String.toBool()/graphemes()`, `HashMap.keys()/values()` type-check then hit V3-S5 stubs (07).
- Comptime: `expression`/`block` annotation targets compile then die on `op_new_array(0)`; `binding` target doesn't parse though it's in the validator vocabulary and the book (10).
- GC: the *canonical Finding #31 source form* (`var arr = []`) can't run at all — `op_new_array(0)` V3-S5 SURFACE (06).

**`op_new_array(0)`/`(N)` (the "V3-S5 TypedArray" surface) is a single recurring landmine** hit independently by four verticals. It is the highest-leverage single fix in the project: it silently caps GC, comptime, out-params, and annotation targets simultaneously.

**Implication:** the compiler's "accept" and the runtime's "implement" sets are not reconciled. A compile-time marshalability/implementability gate (reject at compile time what the runtime will refuse) would convert a belt of silent runtime aborts into honest compile errors — and is explicitly what the polyglot design's "primary gate" was supposed to be but never built (13).

### 3.5 Config & documentation fail-open + doc-code inversion

Where the engine is sound, the **surface that configures it fails open**, and the **documents that should constrain the code instead describe a forbidden or non-existent world**.

- Security config: dotted-key ignored (C1), empty `[permissions]` = allow-all, unknown keys silently accepted, `*.example.com` matches `evilexample.com`, capability checks no-op unless a config section exists (07, 11). The *engine* enforces 7/17 permissions correctly and is non-bypassable from Shape — but the path from user intent to a correct permission set is riddled with silent-accept.
- Doc inversion: `runtime-v2-spec.md` mandates the forbidden world (C3); ADR-001/002 sit `Status: Accepted` on the now-refuse-on-sight NaN-boxing architecture; ADR-006 (7,536 lines) has a **phantom §2.7.26** that `AGENTS.md` cites to justify shipped code, a `SharedCell` ordinal wrong in three places (a wrong-destructor-UB class if anyone codes from the text), and a lost Q3 (18).
- CLAUDE.md is stale in *both* directions on the `NativeKind` variant list (lists nonexistent `Unit`, omits ~20 of 30 real variants), documents phantom forms (tuples, `import/export`, array-rest destructuring, `fn(int)->int` type syntax), and mis-cites `core.rs:218` (drifted to :401/:406) — which cost auditors real probe budget (02, 08, 19).
- codebase-index: 13/16 sampled file:line pointers stale; declares implemented constructors "not yet implemented" and documents deleted types (18).

**Implication:** the docs are actively dangerous to code from in several named places, and the "authoritative" spec is the most dangerous of all. §7 recommends demoting/rewriting the inverted docs before they seed a regression.

### 3.6 The severed verification loop (why all of the above accumulated)

Every pattern above is a *detectable* class. The reason they accumulated is that **the detectors don't run on current code**:

- Local `main` 1,872 commits ahead of `origin/main` → all GitHub CI validates a 6-week-old snapshot (C2). Releases shipped from 8+ consecutive red CI runs.
- **Gates that cannot fail by construction:** `verify-merge.sh` CHECKs 4/7/11 are silent no-ops (rg 15.1 rejects the `-E/--include` flags, error swallowed by `2>/dev/null || true`; a planted merge-marker goes undetected — live-proven); coverage has no `--fail-under` and wraps every step in `continue-on-error: true`; the fuzzer has never fuzzed (corpus-replay only; `mutation.rs`/`minimizer.rs` unreachable from the binary; `--seed` accepted and ignored); nightly fuzz forces exit 1 every run so real divergences are indistinguishable from expected ones (15).
- `check-no-dynamic` and the differential-gate *are* sound and *do* pass at HEAD — but CLAUDE.md's claim that they run "on every CI run and pre-commit" is false on both (15).

**Implication:** this is the root cause behind the root causes. Re-establishing a green, current, push-triggered CI that actually gates releases — and repairing the three no-op checks and the compiled-out fuzzer — is the highest-leverage *process* fix in the project. Without it, fixing A1–C4 doesn't stay fixed.

### 3.7 Vestigial trading-DSL heritage

Shape was evidently born as a financial/trading DSL and became a general language; the corpse is load-bearing in places:

- ~170 lines of **dead-but-harmful** trading grammar still *hijack user identifiers*: `fn back(...)`/`forward(...)` become Duration literals evaluating to 0; bare `5m` compiles to 0; SQL `window/join/optimize/alert` rules are unreachable yet green-tested in isolation; `data[...]` is permanently reserved with a self-defeating error hint (01).
- JITContext still carries `in_position`/`entry_price` fields (05); the MCP `llms.txt` still teaches retired `c"..."` syntax (16).

**Implication:** this is pure liability with no upside — it produces *silent wrong results* on ordinary user code (a function named `back`). Deleting the trading DSL grammar and its ABI vestiges is a low-risk, high-clarity cleanup.

---

## 4. What is done well (specific, named)

The project's reputation for the Forbidden-Patterns saga makes it easy to assume the worst; the audit found the opposite in the core. These are real, verified strengths:

1. **The strict-typing spine is genuinely enforced** (02, 03). `int ≠ number` (they never unify), no truthiness coercion, `string→int` rejected, the lossless numeric-conversion lattice matches the ruled spec *exactly*, HM let-generalization with value restriction works, trait bounds/supertraits enforced, user-enum exhaustiveness checked. The `ReliableOnly` bypass (a past CATASTROPHIC finding) is *genuinely deleted*; `Strict` is the only default. Verified across ~90 + ~60 probe programs. **`ValueWord` is genuinely gone — `check-no-dynamic` exits 0 on the dirty tree; no live forbidden-pattern code exists anywhere in the workspace** (19). The multi-session war against dynamic dispatch was, in the end, won in the code.
2. **The MIR layer is real compiler engineering** (03): a Datafrog non-lexical-lifetimes borrow solver (verified non-lexical), RC-on-escape storage planning, and a *solver-verified borrow-repair engine*. This is graduate-level and mostly correct.
3. **The Bacon-Rajan cycle collector works and bounds RSS on its flagship** (06): barriers, STW rendezvous, teardown sweep, shared drop/GC edge enumeration — RSS flat at ~75 MB from 250k to 2M iterations on the closure-in-array cycle. The value model (06) is the highest code-quality vertical in the project (88/100), with ADR-005/006 single-discriminator discipline conformant at *every* GC site.
4. **Snapshot / resume / distributed genuinely works** (12), decisively refuting its own "dead stubs" history: `snapshot()`/full-resume, mid-loop SIGINT interrupt-resume (the discriminating test — resume continues at i=3M without recompute), function-frame checkpoints with typed heap, `@remote` content-addressed transfer to a live `shape serve` node with receiver-side zero-trust hash + permission-union recompute + grant gate. **Polyglot×distributed composition re-verified genuine** (`REMOTE_PY=105`). The receiver zero-trust pipeline is exemplary.
5. **The VM interpreter is disciplined and correct** (04, 08): typed slot stack with parallel `NativeKind` track, kind-dispatched retain/release, PHF method dispatch, enforced resource limits, graceful errors. The forbidden-pattern sweep is clean in live code. The interpreter is the trustworthy heart of the system.
6. **The polyglot foreign-call core is well-designed** (13): a shared `invoke_foreign_kinded` core, the ratified Q13 three-class error channel (foreign exception → Err; nonconforming return → `TypeConformanceError` Err; host gap → class-2), a two-layer ABI fingerprint gate that refuses skewed vtables (which this audit's own setup exercised), an anti-UB rule that raw C pointers are never `Ptr(HeapKind)`, and permission-before-`dlopen` ordering.
7. **The native stdlib core is correct** (07): crypto/json/compress/csv/regex/unicode/toml/yaml/xml/msgpack/arrow validated against known vectors, strict-typing-clean, with a genuinely well-designed two-tier `ConcreteReturn`/`TypedReturn` marshal layer.
8. **The LSP is a serious, well-tested server** (14): 49.4k LOC, 767/767 green tests, riding the *real* compiler (not a fork); `vmjit-diff` is exemplary tooling.
9. **The anti-drift enforcement machinery is better than most production compilers** (18, 19): the monotonic forbidden-symbol gate, the 15-check merge gate that encodes real postmortems, 567 exact ADR marker comments anchoring code to rulings, `vmjit-diff`'s known-red allowlist discipline, and `registry_cross_check`. The *mechanisms* are excellent — the problem is purely that some aren't wired to run (§3.6).

The through-line: **the deeper and more core the subsystem, the better it is built.** Value model, MIR, GC, VM, strict-typing — the load-bearing center is strong. The rot is at the edges, the seams, and the second (JIT) implementation.

---

## 5. What is done poorly / tech-debt clusters

1. **The JIT-as-documented fiction** (05): a ~69k-LOC AOT compiler wearing a tiered-JIT costume, with ~5k LOC of dead machinery (worker, osr_compiler, two unwired caches, optimizer passes with zero consumers) maintained warm, plus the book and `--help` advertising fabricated API (`vm.register_jit_function`, `--trace-jit`) that the binary rejects.
2. **Complexity god-objects** (03): `compile_statement` 1,736 LOC, `compile_expr_method_call` 1,501 LOC, a 178-field `BytecodeCompiler` struct, `statements.rs` at 9,663 LOC. These are the same files that carry the type-checking split-brain — the size is why the two passes drifted.
3. **Dead-code mass throughout**: `evolution.rs` 315 LOC write-only (02), `physical_binding.rs` 5 unsafe blocks behind safe fns with zero callers (02), `ic_fast_paths.rs` ~80% dead (04), `AnnotationContext` 417 LOC zero-consumer documenting a *dead legacy contract the LSP renders as hover help* (10), the entire channel subsystem 245 LOC dead (09), 9 of 15 async opcodes never emitted (09), the dead did-you-mean module that would recommend nonexistent JS APIs (01).
4. **Lockstep-by-comment instead of by-mechanism** (19): `expected_kind_from_serializable` duplicated across shape-runtime/shape-vm and *already diverged*; `kind_type_name` triplicated with live divergence (4 vocabularies for user-facing type names); JIT re-declares layout constants in 5 modules with zero const-asserts; the legacy `heap_header.rs` `MAX_VARIANT` re-regressed wrong for 18/36 kinds with its own tests certifying the bug.
5. **The marshal/dispatch boilerplate tax**: `marshal.rs` ~1,200 lines of arity-monomorphization boilerplate (07); four copy-pasted dispatch loops in `dispatch.rs` that already caused a real enforcement gap — resource limits skipped on the fast path (04); ~400 lines dead marshal code in polyglot, one path encoding a *different* conformance regime than the ratified oracle (13).
6. **The governance document layer is eroding under append pressure** (18): ADR-006 at 7,536 lines with phantom sections and wrong ordinals; the defections log went dormant after 2026-05-08 (62 of 66 entries in a 72-hour window, then nothing through W14–W18/Phase-2d/GC); practice sharded into 364 unindexed cluster-audit files; `CONTEXT.md` (the canonical domain doc) is *untracked in git*.

---

## 6. Feature completeness — the honest matrix

Distilled from all verticals. "Works" = verified end-to-end against the binary in the VM path (the trustworthy mode).

| Feature | Status | Note |
|---|---|---|
| Functions, generics, traits, closures/HOF, match, enums | **Works (VM)** | Core surface solid (01, 08) |
| Strict typing / numeric lattice / let-generalization | **Works** | Genuinely enforced (02) |
| Result / `?` / `!!` / error handling | **Works (VM)** | (08) |
| Pattern matching (all payload kinds) | **Works (VM)** / **broken (JIT)** | Refutation lost under JIT (A2) |
| Drop / RAII / escape-deferred drop | **Works (VM)** | (08); `-> &UserType` escape broken (06) |
| Modules (import/export/mod/use) | **Partial** | Runs; **privacy not enforced anywhere** (08) |
| Snapshot / resume / `@remote` distributed | **Works** | Strong (12) |
| Cycle GC | **Partial** | Flagship bounded; **ordinary `Option<Node>` cycle leaks** (06) |
| Comptime core + 4 flagship stdlib patterns | **Works (single-file)** | **Breaks at module boundaries** (10) |
| Annotations (`@before/@after/@comptime`) | **Partial** | Hooks silently don't fire; state-rebuild broken (10) |
| Async (`async fn/let/scope`, join, `await`) | **Partial** | Real overlap only for stdlib calls + zero-arg scalar fns; else silently serial (09) |
| Channels / streams / `for await` | **Stub / dead** | No cross-task comms primitive exists (09) |
| Polyglot python / typescript / extern C (scalar) | **Works** | (13) |
| FFI out-params / named-type returns / TS transpile | **Stub (runtime-refused)** | Book's flagship shapes abort (13) |
| Capability sandbox (engine) | **Works (7/17 perms)** | Non-bypassable from Shape (11) |
| Capability sandbox (config → default run) | **Fails open** | Inert unless config present; dotted-key ignored (11) |
| Tiered JIT (T1@100 / T2@10k) / OSR / deopt | **Fiction** | Never runs; stubs return `Err("deprecated")` (05) |
| JIT (whole-program AOT, default mode) | **Fast but unsound** | Segfaults/wrong-results on ordinary programs (§3.1) |
| bigint / tuples / flow-narrowing / instanceof | **Documented, don't work** | Parse errors or discarded facts (02) |
| REPL | **Partial** | Silently swallows all `print()` output (14) |
| `shape check` (multi-file) | **Broken** | False errors on every project (14) |
| Ed25519 signing (primitive) | **Works** | **Never invoked on run/serve** (11); registry has no trust root (16) |

---

## 7. Book vs reality

The "denominator trap" (a curated 240-fence gate green while the full universe was ~47% true) has been **substantially, genuinely remediated** (17): the full 707-fence universe now measures **570/707 (80.6%) green**, the runnable denominator grew 240→565 (all green, real snapshot-resume round-trip fixtures), and the flip campaign is visible in git history. This is real progress and deserves credit.

What remains is structural and matches the project-wide pattern:

- **Exclusion still hides failure.** `runnable=false` silently hides 51 failing examples, including whole broken stdlib pages (`property_testing.mdx` 0/4 fails its own strict-typing gate; finance/iot/physics 0-green). The DateTime JIT segfault (A4) hides behind exactly this flag.
- **Output is essentially ungated.** Only 11/565 fences pin stdout; 3 confirmed wrong `//-output` claims sit gate-green.
- **83% of "JIT" gate legs are interpreter fallback** — so the gate that should catch §3.1 mostly doesn't exercise the JIT at all.
- **The three highest-traffic surfaces are the worst and have zero CI:** `shape/README.md` (both front-page examples fail to compile), the landing page (3/5 hero samples are fiction — `struct`, `u64`, `@db_schema`, `@host`, `emit`, `Snapshot::None` don't exist), and `shape/examples/` (5/6 files fail). The `llms-full.txt` LLM export is 71 pages stale.
- **The biggest documentation lie is the JIT chapter** (`jit-compilation.mdx`): "Tier 2 compilation is shipped" + fabricated API, describing a subsystem that provably does not run (05, 17).

CLAUDE.md is itself stale in named, load-bearing ways (§3.5) — the docs the *next agent* codes against are part of the debt.

---

## 8. Codebase structure map

~717k lines of Rust. The distribution tells the architecture story:

| Crate / tool | Rust LOC | Role & health |
|---|---:|---|
| `shape-vm` | 265,085 | Interpreter + bytecode compiler + MIR + executor. The center of gravity; disciplined core, god-object files. |
| `shape-runtime` | 121,678 | Type system, inference, type schemas, stdlib, method registry. Strong core; the inference↔compiler seam is where §3.2 lives. |
| `tools/shape-test` | 103,250 | Integration harness. Elaborate; **defaults to VM**, hiding §3.1. |
| `shape-jit` | 69,211 | Cranelift AOT. ~5k dead; documented as something it isn't (05). |
| `tools/shape-lsp` | 49,567 | LSP. Large, 767/767 green, rides real compiler (14). |
| `shape-ast` | 32,629 | Pest grammar + AST + transforms. Modern surface good; dead trading DSL (01). |
| `shape-value` | 31,555 | Value model + GC. **Best-built vertical** (06). |
| `bin/shape-cli` | 21,134 | 26 subcommands, 20+ working (14). |
| `shape-wire` | 4,061 | MessagePack + QUIC (client-only, gated off). |
| `shape-abi-v1` | 3,263 | Permission enum, `LanguageRuntimeVTable`. |
| `shape-diagnostics` | 854 | LSDS — declared primary, actually inverted (14). |
| `shape-types` | ~0 | **Empty skeleton** (documented; type code lives in shape-runtime). |
| `shape-common` | — | **Ghost crate** (Cargo.toml only, not a workspace member) (19). |
| extensions (py/ts) | 2,344 | PyO3 + deno_core (13). |

**Pipeline:** pest → AST (+ desugar transforms) → two-pass bytecode compiler (type inference *and* an independent second typing pass — the §3.2 seam) → typed-slot VM interpreter *or* MirToIR→Cranelift AOT (the §3.1 fork) → optional snapshot/wire serialization.

**Satellites** (separate workspaces, each pinned at a different language epoch — §16): `shape-registry` (Axum, Ed25519 but no trust root), `shape-app` (playground/notebook, unsandboxed execution), `shape-mcp` (retrieval tools broken by book reorg), `packages/` (duckdb/xgboost no longer compile against HEAD), `shape-infra` (NixOS, strong systemd hardening carrying the app's missing sandbox).

---

## 9. Prioritized recommendations

### P0 — do before any release claims soundness

1. **Flip the default execution mode to VM until the JIT reaches differential parity** (§3.1). This single change neutralizes A1–A6. It is a `cli_args.rs` default change plus a `--mode jit` opt-in. The alternative (fix the JIT) is months; this is hours and honest.
2. **Re-establish current, green, push-triggered CI that gates releases** (§3.6, C2) and repair the three no-op `verify-merge` checks, the compiled-out fuzzer, and the coverage `--fail-under`. Nothing else stays fixed without this.
3. **Wire and un-stale the VM-vs-JIT differential gate** (`vmjit-diff` + `differential_fuzz`) and run it on current commits. It already knows the defect class; it just isn't allowed to see the code.
4. **Fix the soft-check unsoundness B1** (annotated generic-method value args must go through the same proof the fresh-var path uses) and **delete the live `NumberToInt` float-range emission B2**. These reopen the exact hole the strict flip closed.
5. **Fix the `shape.toml` dotted-key silent-ignore C1** and add `deny_unknown_fields` to the permissions surface; make empty `[permissions]` deny-by-default. **Install a sandbox on the shape-app execution path** (C4).
6. **Demote/relabel `runtime-v2-spec.md` C3** — strip the "authoritative" header and the forbidden-world mandate before it seeds a regression. This is a doc edit that prevents a 4–6-week defection.

### P1 — the systemic fixes

7. **Build the single type/method oracle** consumed by both the inference engine and the bytecode compiler (§3.2). This is the structural cure for flow-narrowing, closure-return checking, Option/Result exhaustiveness, and the three-way method-table drift. Largest item on the list; highest structural payoff.
8. **Implement `op_new_array(0)/(N)` (the V3-S5 TypedArray surface)** (§3.4). One fix un-caps GC's canonical form, comptime arrays, FFI out-params, and annotation targets simultaneously.
9. **Add a compile-time implementability/marshalability gate** that rejects at compile time what the runtime will refuse (§3.4) — converts the silent-runtime-abort belt into honest compile errors.
10. **Resolve the `CallFrame.closure_heap_bits` carrier split-brain B4** before the closure-snapshot work lands and turns it from latent to deterministic corruption.
11. **Delete the trading-DSL grammar and ABI vestiges** (§3.7) — pure liability producing silent wrong results on user identifiers.
12. **Fix the GC completeness tail** (06): enumerate header-less `Arc`-backed cycle roots (`Option`/`Result`/`HashMap`/...) so the ordinary `Option<Node>` cycle stops leaking under the shipped gc-on default.

### P2 — hygiene & honesty

13. Reconcile the docs to code (§3.5): the `NativeKind` variant list, the phantom language forms in CLAUDE.md, the stale codebase-index pointers, ADR-006's phantom §2.7.26 and wrong `SharedCell` ordinals, ADR-001/002 status headers.
14. Break up the god-objects (`statements.rs` 9,663 LOC, the 178-field compiler struct) — they are why the passes drifted.
15. Convert lockstep-by-comment to lockstep-by-mechanism (const-asserts on layout constants, a single `kind_type_name`, dedup `expected_kind_from_serializable`).
16. Fix the ecosystem version-skew (16): re-pin the registry serde contract, repair the MCP doc-ID maps, recompile the sample packages against HEAD, give the registry a real trust root.
17. Fix the DX paper-cuts that hit every user: REPL swallowing `print()`, `shape check` false-erroring projects, filenames missing from diagnostics, internal jargon leaking to stderr on hello-world.

---

## 10. Closing assessment

Shape is **not** a project in trouble at its core — it is a project whose core is strong and whose *verification and honesty layers* have decayed faster than its engineering. The strict-typing war was won in the code; the borrow solver, GC, snapshot system, and interpreter are real and often excellent; the enforcement machinery is world-class *where it runs*. The danger is concentrated and nameable: **a default mode that ships unverified, a test suite that verifies the wrong mode, a CI that verifies old code, and a documentation set that in several places describes a language that either doesn't exist or is explicitly forbidden.** None of the top defects are deep research problems — they are a default flag, a CI reconnection, a differential gate un-staled, a handful of soft-check plugs, and a documentation reconciliation. The engineering to fix them is already in the building; it mostly needs to be *pointed at the current code* and *told the truth about what ships*.

The single most valuable sentence for the maintainers: **the interpreter is the language you can trust today; make the binary run it by default, make CI current, and let the differential gate tell you when the JIT has caught up.**

---

*Per-vertical evidence: reports `01`–`19` in this directory (21,923 lines). Each carries its own executive summary, architecture map, feature-completeness matrix, DRY/split-brain analysis, ADR conformance table, in-territory test audit, book-vs-reality section, bug register with repro transcripts, and prioritized recommendations.*

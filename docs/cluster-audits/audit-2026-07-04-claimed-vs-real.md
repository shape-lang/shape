# Shape v0.3.2 Codebase Audit — Synthesis Report

**Scope:** repo `/home/dev/dev/shape-lang/shape`, binary v0.3.2 at main HEAD `1fb805b3`, audited 2026-07-04 by ~40 finder agents with adversarial verification of critical findings and six targeted gap-closure passes. All statements below are grounded in the collected evidence; no finding was refuted during verification (verdicts were CONFIRMED or OVERSTATED — overstated items appear here with corrected wording).

---

## 1. Executive answers

**Q1: Does comptime fully work, and is it ergonomic? — Partial. No, it does not fully work; ergonomics are split between excellent and broken.** The core engine is real: comptime blocks and functions (with recursion, generics, a runaway-loop watchdog that interrupts infinite loops while completing 10M-iteration loops), `implements()`, `build_config()` returning correct `linux/x86_64/0.3.2`, comptime type fields, function-target annotation wrappers with correct outer/inner chaining, and working `extend`/`replace body`/`set return`/`remove target` directives inspectable via a genuinely nice `shape expand-comptime` tool. But against a "fully works" bar it fails hard: **(a)** `comptime post { set return string }` on `fn answer() { 42 }` bypasses body-vs-signature checking and reproducibly **segfaults the shipped binary** (exit 139) — the equivalent explicit annotation is correctly rejected at compile time — a confirmed critical strict-typing soundness hole (functions_annotations.rs:1492). **(b)** Annotation hooks are **silently dropped whenever the calling code actually JIT-compiles under the default mode** (verified: before-hook short-circuit `{result: 99}` returns 5000 under jit, 99 under vm) — a validation/policy annotation is unenforced in the shipped default. **(c)** The introspection surface powering every documented derive/schema recipe is broken: `target.fields` descriptors carry garbage keys (`{is_valid: "host", parse: "string", stringify: [], _3: false}` instead of the documented `{name, type, annotations, optional}`) from a schema-id collision, so `field.name` raises `Undefined property: name`; `type_info()` is non-functional; `error("...")` destroys the user's message, printing `[comptime error] <Bool> (line 1)`; `warning()` is a total silent no-op. Where implemented, diagnostics are clean; where broken, users see internal audit jargon ("V3-S5 ckpt-5", "REFUSED ON SIGHT"). The CLAUDE.md claim of comptime "enabling user-defined LLM integration patterns in stdlib" has zero stdlib instances.

**Q2: Does polyglot fully work? — Broken. It does not work at all, end-to-end, in any shipped release.** The VM's `op_call_foreign` handler is an unconditional surface-and-stop stub (crates/shape-vm/src/executor/control_flow/mod.rs:854-903) that drains the args and returns `Not implemented: op_call_foreign: phase-2c — extern C FFI rebuild (ADR-006 §2.7.4 / §2.7.5)`. Every `fn python` and `fn typescript` call fails with that error in both `--mode vm` and `--mode jit`, even when the extension loads successfully ("Loaded module: python v0.1.0"). The JIT tier is independently dead: `jit_call_foreign_impl` is `todo!()` (shape-jit/src/ffi/control/mod.rs:931) and the foreign bridge hard-refuses all dynamic-error runtimes — which is 100% of Python/TypeScript functions since the compiler mandates `Result<T>` returns for them. The stub landed 2026-05-09 (commit 173b8798) and is in tags v0.3.0/v0.3.1/v0.3.2, while the book documents the feature extensively (marshalling tables, async, NumPy) with no availability caution. The bundled `python`/`typescript` eval namespaces are unreachable under every import syntax including the one the book shows; the documented "TypeScript transpilation" does not exist (bare deno_core, no swc/deno_ast); no permission gates foreign code (no Ffi/Native variant in the 16-permission enum), so when invocation is rebuilt, foreign bodies will run with full process authority. The scaffolding (extension loading, discovery precedence, `Result<T>` compile enforcement, the PyO3/deno_core extension sides) is real and good — the break is entirely host-side, at the one point that matters.

**Q3: Does distributed computing with per-function remote transfer and pause/resume fully work? — Broken for both asked capabilities.** *Per-function transfer:* the Rust machinery exists and works at library level — `build_minimal_blobs_by_hash` computes transitive dependency closures, permissions are folded into `FunctionBlob::compute_hash`, the linker unions permissions, and `test_remote_function_call_over_tcp` passes for f64 args. But no Shape program can use it: the stdlib declares `__call(addr, fn_ref, args)` with 3 params while the Rust builtin registers 2, so `@remote` and `remote::__call` both die with `expected 2 arg(s), got 3` — and even with matching arity the `__call` body is a hardcoded surface-and-stop `Err`. The receiver rejects all closure/upvalue-carrying calls. The only working remote path is `remote::execute(addr, source_string)`, which ships **source text**, not a content-addressed function blob. *Pause/resume:* completely non-functional. `snapshot()` raises a suspension sentinel (`SNAPSHOT_FUTURE_ID` = u64::MAX) that no host code catches — every variant exits 1 with the leaked internal error `Suspended on future 18446744073709551615` and persists nothing; both `--resume` entry points (execution.rs:190/209) are unconditional Phase-2c stubs; Ctrl+C prints "Interrupting — saving snapshot..." then saves nothing; the entire `std::core::state` runtime set is stubbed; and `state::hash` silently returns a near-constant digest for every input (all args collapse to Bool before hashing, marshal.rs:2295-2298).

**Q4: Are C bindings ergonomic and first-class? — Broken at runtime; first-class on paper only.** The declaration-side design genuinely is first-class: one-line `extern C fn` declarations against plain `.so` files with zero C-side boilerplate, a broad compile-side type map, strict call-site checking, out-param stub generation, an excellent `[native-dependencies]` resolver with sha256 lockfiles and precise preflight errors, and broad LSP awareness. But the shipped binary cannot execute a single native call: `link_native_function`, `invoke_linked_function`, and `op_call_foreign` are all phase-2c stubs, and linking is **eager**, so any program that merely *declares* an extern C fn dies at startup with `Failed to link native function ... phase-2c` — verified end-to-end against a real gcc-built `.so` in both modes. The static layer also contradicts the book: the Core Syntax `cview<T>`/`cmut<T>` example is rejected (only `CView`/`CMut` compile), `cstring` params cannot accept Shape strings at all, ptr↔int conversion is inexpressible (the book's own out-param example fails to compile), `shape check` cannot parse frontmatter so native-deps scripts are un-checkable, and there is no FFI permission in the sandbox model — a latent bypass once the rebuild lands. Additionally the LSP falsely red-flags every valid extern C declaration (it hardcodes `validate_type_annotations(true)`, demanding `Result<T>` returns the compiler does not require).

---

## 2. Claimed-vs-real matrix

| Vertical | Verdict | State in one sentence | Worst gap |
|---|---|---|---|
| Comptime | Partial | Core engine, directives, and hooks are real; introspection, diagnostics, and several target kinds are broken. | `set return` bypasses type checking and segfaults the binary (exit 139). |
| Polyglot: Python | Broken | Scaffolding complete on both sides; invocation impossible. | `op_call_foreign` stub fails every call in every shipped v0.3.x tag. |
| Polyglot: TypeScript | Broken | Extension loads, compile checks fire, V8 invocation unreachable; "TypeScript" would be plain JS anyway. | Same CallForeign stub, plus `jit_call_foreign_impl` is `todo!()`. |
| C interop | Broken | First-class front-end and dependency tooling; runtime is a facade. | Eager linking aborts any program merely declaring an extern C fn. |
| Distributed / remote | Partial | Source-string `remote::execute` works end-to-end; per-function transfer works only in Rust unit tests. | `@remote`/`remote::__call` non-functional (3-vs-2 arity mismatch + stub body). |
| Snapshot / resume | Broken | Substantial internal round-trip machinery wired to nothing user-reachable. | `snapshot()` always exits 1 with a leaked sentinel; both resume paths are stubs. |
| Type system | Mostly works | Strict default is real (all seven mandated probes are compile errors); the 2026-05-29 ReliableOnly bypass is gone. | `impl Trait for` builtins compiles then fails at runtime dispatch; no narrowing mechanism works. |
| Async | Partial | Full syntax surface parses and produces correct values — serially. | Zero concurrency: two 1s tasks take ~2000ms; stdlib calls don't compile inside async fns; `await time::sleep` panics the process. |
| Core language | Mostly works | Match/enums/Result/borrow checker pass end-to-end; book is stale pessimistically. | Drop never fires for function-returned values (silent resource leak); slices `[1..]`/`[..=]` return silently wrong values. |
| Stdlib core/math | Partial | core/math, collections, log, linalg, set, random all correct. | rotation/interpolation/testing/property_testing/stochastic/optimize all broken with no book caution. |
| Stdlib native/domain | Partial | file/csv/regex/crypto/compress/unicode/io largely solid. | http segfaults on every call; finance domain doesn't even parse; json/msgpack/toml/yaml serialization stubbed. |
| Security | Partial | Well-factored, unit-tested primitives; Ed25519 chain works end-to-end. | All three enforcement tiers are inert in the shipped CLI — `serve --sandbox strict` is discarded; live sandbox-escape file write proven. |
| JIT | Partial | Real ~3.6x speedup on the narrow construct set that compiles; fallback usually preserves correctness. | Tiered compilation (T1@100/T2@10k) is completely inert; runtime JIT failure re-runs the program, duplicating side effects. |
| Tooling | Mostly works | Projects, build/sign/verify, LSP, MCP book tools, serve safety rails all work. | Book documents an HTTP MCP REST server that does not exist; `shape repl` docs describe the undocumented `shape tui`. |

---

## 3. Book accuracy

**Overall: 450 of 616 gate-run examples pass (73%); 158 fail (26%).** The book is wrong in *both* directions — it documents broken features as working (polyglot, http, msgpack, state, finance) and working features as broken (spread, comprehensions, NumericVec methods, trait bounds, set/random, unicode::graphemes, destructuring-from-binding). The truth-gate has evidently not been re-run against HEAD.

**Per-area pass rates (worst first):**

| Chunk | Pass/Total | Rate |
|---|---|---|
| std-domain | 1/9 | 11% |
| polyglot-c | 4/10 | 40% |
| adv-exec | 5/12 | 42% |
| std-native-1 | 33/70 | 47% |
| std-core-2 | 17/32 | 53% |
| adv-distributed | 8/15 | 53% |
| std-native-2 | 25/37 | 68% |
| std-math | 14/20 | 70% |
| tooling | 17/23 | 74% |
| adv-comptime | 12/15 | 80% |
| fund-1 / fund-2 | 75/92, 75/94 | 82%, 80% |
| fund-4-ownership | 48/58 | 83% |
| std-core-1 | 34/37 | 92% |
| getting-started | 20/21 | 95% |
| fund-3 | 39/41 | 95% |

**Worst chapters:** the entire domain stdlib section (finance/iot/physics — most modules fail parse or strict-typing at import), the polyglot/C chapters (every example dead on the CallForeign stub), advanced execution (state/hash/`--trace-jit` all broken), and std-native-1 (json/msgpack/toml/yaml/http/time).

**Notable failing examples:**
- `HashMap.filter(|k,v| v > 1).len()` — documented "verified at HEAD" — prints a **different garbage integer each run with exit 0** under default JIT (e.g. `4816013124661467136`); under vm it errors `no method 'filterIndexed' on receiver kind Ptr(HashMap)` (the compiler rewrites arity-2 filter/map to Array-only `filterIndexed`/`mapIndexed`).
- enums.mdx `find_first([1,2,3,4,5], |x| x > 3)` prints `found: 1.0` instead of `Some(4)` — inline closure passed to a `(number)->bool` param mis-evaluates for every element, both modes.
- resource-management.mdx's drop-error guarantee ("error is logged, other drops still run, return value preserved"): an error in `b.drop()` aborts with `Runtime error: Index 5 out of bounds (length 1)`, exit 1; `a.drop()` never runs.
- content-addressed-bytecode.mdx's `__original__(args)` convention (all 5 annotation examples) silently returns run-varying pointer garbage (`base(5)` → `205160873083618`) instead of 12; the compiler injects `let args = [params...]`, and the array silently passes where an int is expected.
- operators.mdx compound-assign example documents `print(x) // 4`; actual (and arithmetically correct) output is 1 — a documentation arithmetic error.
- remote.mdx examples all use `localhost:9527`, which the transport rejects (`invalid socket address syntax`); only IP literals work.
- module-distribution.mdx documents a `shape bundle` subcommand that does not exist, wrong `keys trust` flags, and FORMAT_VERSION 3 vs the code's 4 (package_bundle.rs:21).
- Stale-pessimistic cluster: ~10 objects-arrays.mdx features marked broken/SURFACE all work at HEAD (object/array spread, comprehensions, `let {x,y}=point`, sort(cmp)/reduce/flatMap, all six NumericVec methods, nested typed structs, overlapping-key merge); traits.mdx trait-bound and default-method examples marked runnable=false run green under both modes; set.mdx/random.mdx carry "not available in v0.3.3" cautions for fully working modules.
- Cross-cutting audit datum: 33 of 73 fundamentals runnable=true examples (and ~62% of all 738 scratchpad programs) whole-program-deopt to the interpreter under the default `--mode jit`.

---

## 4. Feature gaps ranked

1. **The entire foreign-function family is dead in every shipped release (confirmed critical).** One stub — `op_call_foreign` at control_flow/mod.rs:854-903 — kills `fn python`, `fn typescript`, and extern C simultaneously, in both execution modes, since 2026-05-09; eager linking makes even *declaring* extern C fatal (native_abi.rs:72-104). Nothing in the tiered test suite catches it: the only Python e2e tests are feature-gated out of every tier *and* use signatures the compiler now rejects.

2. **The three-tier security model is inert and the execution server is unsandboxed (confirmed critical, live-proven).** `shape serve --sandbox strict` (the default) is parsed then discarded (`let _sandbox = config.sandbox;` serve_cmd.rs:430); a wire client running `file::write_text` against strict mode wrote `sandbox_escape.txt` to disk with `success: true`. `load_program_with_permissions` (program.rs:357,381) has **zero call sites** — the advertised "permissions baked into content hash, checked at load time" check never runs. `shape.toml [permissions]` parses but is unenforced (fs.read=false still reads /etc/hostname). No resource limit ever fires via the CLI. `check_permission` treats `granted_permissions: None` as allow-all, and the only production ModuleContext hardcodes None (modules.rs:699-706).

3. **Pause/resume is entirely non-functional (confirmed critical)** — see Q3. Every axis is dead: create (sentinel uncaught), resume (stubs), interrupt-save (no producer of `ShapeError::Interrupted`), state primitives (W17 stubs), content hashing (`state::hash` Bool-collapse).

4. **JIT default-mode correctness class (confirmed critical, multiple bugs).** (a) A runtime JIT failure with un-carved-out signal (-1, the dominant class) re-runs the whole program under the interpreter, **duplicating already-executed side effects** — `print("BEFORE")` prints twice; 19/738 programs hit this path. (b) **i64 overflow split-brain**: default-mode JIT silently wraps (`i64::MAX + 1` → `-9223372036854775808`, exit 0) while VM raises the ruled checked error — a direct violation of the release-blocking 2026-06-01 D3 ruling, invisible to the 117-case conformance suite because ShapeTest executes VM-only. (c) Annotation hooks silently dropped when JIT compiles. (d) HashMap.filter garbage. (e) Tiered compilation is entirely inert: `enable_tiered_compilation`/`set_backend`/`register_jit_function` have zero production callers, and the interpreter's promoted-function dispatch site is itself a NotImplemented stub — the promote-to-native pipeline was never end-to-end tested.

5. **Async is a serial-execution facade (confirmed critical).** `async let` evaluates eagerly inline (advanced.rs:745 compiles the RHS before SpawnTask); two 1000ms tasks measure 2000-2001ms in both modes. `join race` runs every branch to completion; `join any` never skips failures; scope cancellation is vacuous. Any module-qualified stdlib call inside an async fn fails to compile (`Unknown qualified call 'io::open'`, function_calls.rs:3185), and top-level `await time::sleep(10)` Rust-panics the process (`can call blocking only when running on the multi-threaded runtime`, modules.rs:733, exit 101).

6. **Per-function remote transfer has no working user-facing path (confirmed critical)** — arity mismatch plus stub body (remote_builtins.rs:318-340), closure rejection at the receiver (remote.rs:731), and a receiver that panics on missing dependency blobs (finder-verified, not independently re-verified).

7. **RAII Drop is broken at every escape boundary (confirmed critical via direct repro).** A Drop-bearing value returned from a function and bound in a caller local is **never dropped** (functions.rs:2335-2348 skips the producer DropCall; the caller never re-arms it); a value captured by a *returned closure* is dropped prematurely at lexical scope exit while the closure still reads it afterward (use-after-finalize); an error in drop() aborts instead of being contained. Non-escaping paths (reverse order, per-iteration, early return, module scope) all work.

8. **bigint is a dead type (confirmed critical).** No literal, no annotation coercion, no cast, no constructor — all four paths verified failing — and the runtime payload is an `Arc<i64>` placeholder (heap_variants.rs:437-442) incapable of arbitrary precision, while integer-types.mdx recommends it four times as the overflow escape hatch. `decimal` works (exact `0.1D + 0.2D = 0.3D`) but its `D` suffix appears nowhere in the book, it has no JIT codegen, and lacks `round()`/`abs()`.

9. **Reference cycles leak unboundedly under the pure-RC/no-GC runtime (confirmed high).** A closure-in-captured-array cycle grew RSS 33MB → 4.1GB over 20M iterations (~210 B/iter, no crash); the book documents "Arc everywhere" with zero mention of cycle leaks and no weak-ref mitigation. Steady-state non-cyclic allocation is leak-free (flat 32.8MB over 20M iterations).

10. **Parser exponential blowup (confirmed critical).** 12 nested parens or 12 nested calls hangs `shape run` and `shape check` for >120s (~3.5-4x per level, pest PEG backtracking) — a user-reachable front-end DoS on legitimate code.

11. **Type-system edges (finder-verified highs, not independently re-verified):** `impl Trait for` builtin types compiles but every call fails (`no method 'shout' on receiver kind String`); no narrowing works (`instanceof` does not narrow despite operators.mdx:159; `null` is not even a token); string/HashMap builtin method args are runtime-checked only; `fn empty() { [] }` behind let-generalization compiles then hits the `op_new_array(0)` V3-S5 stub; `Result.unwrap()` does not dispatch anywhere (`Generic{Result<int,string>} cannot have fields`).

12. **Stdlib breadth failures (confirmed criticals):** http segfaults on every call (request never sent, both modes, no workaround); xml::stringify dumps core; std::finance fails to parse with the shipped compiler; msgpack 100% stubbed; json stringify/navigation/typed-parse broken; toml/yaml parse+stringify stubbed; time::now broken and time::benchmark rejects the book's own int argument.

---

## 5. Code quality

### Duplication / DRY — top clusters

**ADR-mandated (by design, but under-gated):** the 4-table HeapKind lockstep (stack.rs clone/drop_with_kind, kinded_slot.rs, closure_layout.rs, heap_value.rs; ~1,500-2,000 LOC) is required by ADR-006 §2.3/§2.7.7 and currently has **zero drift** (all 36 variants present 4/4, re-verified). But the verify-merge CHECK 6 gate is name-presence-only (`rg -q "HeapKind::X"`), not semantic, and closure_raw.rs:1542-1584 proves two of the four tables could collapse to delegators without violating the ADR.

**Accidental, ranked by dup-LOC × drift-risk:**
- **JIT retain/release tables are a 5th/6th HeapKind dispatch table OUTSIDE the lockstep gate** (ownership.rs:264-338, collection_arc.rs:227-417): 13 arms then a wrong-layout legacy default that has already caused three documented segfault classes; a new HeapKind passes CI with no JIT arm.
- **JIT hand-mirror of method-registry return kinds** (mir_compiler/types.rs:943-997 + two parametric tables vs method_registry.rs:260-860) — the sync mechanism is a comment ("Verified against method_registry.rs"); a registry return-kind change silently drifts JIT slots.
- **v2_array_detect.rs**: ~2,500+ LOC of per-V2ElemType hand expansion (628-LOC `concat_arrays` repeating a 16-line arm 14×) despite the file's own `scan_scalar!` macro proving the pattern.
- **44 byte-identical Load/Store handlers** in variables/mod.rs (~580 LOC) whose width suffix is never consulted.
- **SIMD reduction kernels copied verbatim VM↔JIT** (~350 LOC; a semantics fix in one crate silently diverges aggregation results).
- **marshal.rs**: 21 per-arity registration near-clones (~1,170 LOC → ~100 with a macro).
- **Type-representation web**: 7 overlapping type enums with 13+ pairwise converters and the `"number"|"float"|"f64"` string match re-implemented at 13+ sites (helpers.rs:5429 accepts "f32" as F64 where others differ).
- **Three parallel value-graph serializers** (wire 1,579 / snapshot 3,233 / json 790 LOC) with verbatim-duplicated helpers (`field_is_heap_like` twice; `build_builtin_result/option/variant` ~90 LOC duplicated).
- **Intrinsics dual dispatch** (OVERSTATED→corrected): working simd-backed impls in shape-runtime are unreachable because the compiler routes the same `__intrinsic_*` names to a ~37-variant NotImplemented stub in shape-vm; 11 entries of measured name-list drift across 4+ hand-synced lists. The affected functions (rolling_mean, correlation, median) are book-cautioned as v0.4, so this is medium-severity duplication debt, not critical shipped breakage — but a new intrinsic touches ~8 files.
- Builtin method signatures across 5 hand-synced lists, with a drift bug already recorded in method_table.rs:650 comments.

### Dead code inventory (~10,500 LOC)

- **shape-gc crate, 5,312 LOC**: reachable only via shape-vm's `gc` feature, which **fails to compile** (8 E0599 errors on deleted NaN-box APIs `raw_bits`/`get_raw`) — dead *and* bit-rotted.
- **25 orphan .rs files (~4,750 LOC)** absent from every mod tree: typed_emission.rs (495), concrete_conv.rs (565), crash_handler.rs (390), the 7-file typed_handlers/ cluster, two near-identical 445-line enum_layout.rs copies (diff = 20 lines), typed_enum.rs (382). One orphan self-documents "itself not declared as a module anywhere — never compiled."
- shape-vm's `jit` feature + 5 optional cranelift deps exist solely for orphan jit_ffi_integration.rs.
- ~25 unconditional deps with zero identifier usage (rayon/pest/pest_derive in shape-runtime; serde/serde_json/rmp-serde/anyhow in shape-jit; 7 in shape-vm).
- 173 `#[allow(dead_code)]` (sampled: ~1/3 truly dead, rest stale allows on live code).
- `TypeDiagnosticMode::ReliableOnly` — the variant behind the 2026-05-29 catastrophic bug — never constructed, not deleted.
- `IntrinsicsRegistry` — registered, consumed by nothing.
- **`load_program_with_permissions` / `load_linked_program_with_permissions`** — the load-time security gate, zero call sites (see §4.2).
- crates/shape-types (no Cargo.toml, one unreferenced 21K data file); shape-viz-native (3-line hello-world workspace member); typed_access_bench.rs (empty criterion group).
- **Stale gate rationale:** CLAUDE.md's reason for excluding `--benches` from `just check-clean` no longer holds — `cargo check -p shape-vm --benches` finishes clean in 24s.

### Split-brains ranked by actual divergence

1. **VM/JIT i64 overflow** — checked error vs silent wrap in the *default* mode (arithmetic/mod.rs:150 vs rvalues.rs:1559-1563, whose comment still cites the superseded 2026-05-20 wrapping ruling). Confirmed end-to-end.
2. **LSP vs compiler on extern C** — LSP hardcodes `validate_type_annotations(true)` (diagnostics.rs:1533), red-flagging book-verbatim code `shape check` passes.
3. **Two live `FieldType` enums with opposite size semantics** (shape-runtime: every variant 8 bytes; shape-value v2_struct_layout: natural sizes) — same identifier, both consumed, wrong offsets waiting to happen.
4. **CLAUDE.md trait syntax vs binary** — `fn method(self)` and `extends` both rejected by the shipped compiler; the book is correct, CLAUDE.md is the odd one out.
5. **shape-mcp llms.txt wrong on 5/5 sampled facts** (int i48, retired `c"..."`, Vec-only, trait syntax, import syntax) while the MCP tools correctly serve book pages.
6. **char three-way inconsistent** — literal types as int, `let c: char` is a compile error, TypedObject char fields accept then erase to int (FieldType has no Char variant that NativeKind has); plus the self-documented dual char carrier (NativeKind::Char vs Ptr(HeapKind::Char)).
7. **Match guards** — only `where` parses; the book's llm_summary and CLAUDE.md teach `if` guards, a parse error.
8. **Book build docs** — README/REWRITE_PLAN describe a deleted mdBook at shape/docs/book; the actual book is Astro Starlight.
9. **Array<T>/Vec<T> dual naming**, with errors.rs:211 printing `Vec<...>` even when the user wrote Array.
10. **Version split-brain** — binary reports 0.3.2 while the book and the binary's own error strings speak as v0.3.3; no v0.3.3 tag exists.
11. **KindedSlot** carries `as_typed_object_storage` (kinded_slot.rs:655), a per-heap-variant accessor its own Q8 comment 30 lines below forbids.
12. **tree-sitter grammar drift** — no `extern`, no `out` params, no `f$`/`f#` prefixes present in the pest grammar.
13. json8.shape deterministic stdout divergence vm vs jit *even through the fallback path* (`{frames: 4, module_bindings: "Alice"}` vs `Str("Alice")`) — both outputs debug-repr leaks.

---

## 6. Confirmed critical/high findings table

All rows below survived adversarial verification (CONFIRMED) or direct-repro gap passes; OVERSTATED rows carry corrected wording and severity.

| Finding | Severity | Verdict | Corrected statement |
|---|---|---|---|
| op_call_foreign stub kills all polyglot (python/TS/C paths merged) | Critical | CONFIRMED | Every foreign call fails in both modes with `Not implemented: op_call_foreign: phase-2c — extern C FFI rebuild (ADR-006 §2.7.4 / §2.7.5)` (control_flow/mod.rs:854-903); present in tags v0.3.0-v0.3.2; book documents the feature as working. |
| extern C runtime stubbed; declaration alone is fatal | Critical | CONFIRMED | link/invoke stubs (native_abi.rs:72-104) + eager linking (execution.rs:469-501) abort any program declaring an extern C fn: `Failed to link native function ... phase-2c`. |
| JIT foreign path dead independently | Critical | CONFIRMED | `jit_call_foreign_impl` is `todo!()` (ffi/control/mod.rs:931-946); foreign_bridge.rs:160-171 refuses all dynamic-error runtimes; unreachable anyway — interpreter stub fires first. |
| "Extension V8 runtime never invoked" | Critical | OVERSTATED | Feature still completely broken (ts_invoke unreachable by any route), but ts_compile IS reached during JIT foreign-function linking — a garbage TS body surfaces a real "TypeScript compilation error ... Uncaught SyntaxError" as a [jit-fallback] diagnostic. |
| Comptime `set return` segfault | Critical | CONFIRMED | Applied `set return string` skips body-vs-signature checking (functions_annotations.rs:1492); int body reinterpreted as string pointer → SIGSEGV exit 139; explicit annotation correctly rejected. |
| Annotation wrappers "bypassed by print()/f-string syntax" | Critical→High | OVERSTATED | Real defect is JIT-vs-interpreter divergence: hooks execute under vm/after fallback but are silently dropped whenever the caller actually JIT-compiles (JIT calls the un-wrapped impl); the silent bypass still contradicts the book's loud-SURFACE claim. |
| @remote / remote::__call non-functional | Critical | CONFIRMED | stdlib declares 3-param `__call` (remote.shape:81), Rust registers 2 (remote_builtins.rs:318) → `expected 2 arg(s), got 3`; body is a surface-and-stop Err anyway (remote_builtins.rs:328-340). |
| snapshot() never completes | Critical | CONFIRMED | SNAPSHOT_FUTURE_ID (u64::MAX) raised at builtins.rs:306-309 with zero consumers; every run exits 1 with `Suspended on future 18446744073709551615`, nothing persisted. |
| --resume can never succeed | Critical | CONFIRMED | resume_snapshot/recompile_and_resume (execution.rs:190/209) unconditionally error "snapshot rebuild depends on the deleted ValueWord carrier ... Phase-2c"; CLI still advertises the flag. |
| async let zero concurrency | Critical | CONFIRMED | RHS compiled as a normal call before SpawnTask (advanced.rs:745); two 1000ms tasks = 2000-2001ms both modes; book claims "run concurrently". |
| stdlib calls uncompilable in async fns | Critical | CONFIRMED (broadened) | ANY module-qualified call (builtin or pure-Shape) fails `Unknown qualified call` inside async fns unless the module was called from sync code earlier in source order (function_calls.rs:3185). |
| Top-level `await time::sleep` panics | Critical | CONFIRMED | Rust panic `can call blocking only when running on the multi-threaded runtime` at modules.rs:733, exit 101. |
| http segfaults on every call | Critical | CONFIRMED | SIGSEGV in options-arg unmarshalling (`FromSlot for Vec<(Arc<String>,Arc<HeapValue>)>`) before any request is sent, both modes; all alternate call paths compile-blocked. |
| xml::stringify dumps core | Critical | CONFIRMED | Book-documented object-literal call SIGSEGVs via unsound pointer reinterpretation (marshal.rs:938); genuine HashMap args hit an unconditional stub (xml.rs:334). |
| std::finance unusable | Critical | CONFIRMED | Every sub-module except trait-only `interfaces` fails import: parse errors, unregistered `@warmup`, semantic errors; backtest::engine crashes the compiler with a stack overflow. |
| json module mostly dead | Critical | CONFIRMED | stringify unconditional Err stub (json.rs:502-511, "pending N7"); all 5 navigation methods fail on Ptr(TypedObject); typed parse errors `Field ... exceeds field_kinds length 0`; `as?` not in grammar. |
| msgpack 100% stubbed | Critical | CONFIRMED | All four functions return `pending N4/N6 marshal — ADR-006 §2.7.4` (msgpack_module.rs:52-118); book shows working examples. |
| time module broken | Critical | CONFIRMED | Only millis()/sleep_sync() work; now()/stopwatch() hit the Discriminant(5)/K3 projection error; benchmark rejects int iterations via a kind-blind raw-bits f64 read (stdlib_time.rs:128) and never executes the callback. |
| JIT double execution of side effects | Critical | CONFIRMED | Un-carved-out signal -1 takes the outer-Err path (executor.rs:886-891) into a whole-program interpreter re-run; `print("BEFORE")` prints twice, exit 0; 19/738 programs hit this path. |
| Tiered compilation inert | Critical | CONFIRMED | enable_tiered_compilation/set_backend/register_jit_function: zero production callers; tier backend paths return "deprecated"; op_call dispatch of promoted functions is itself a NotImplemented stub (control_flow/mod.rs:285-300). |
| Intrinsics dual-dispatch ships broken stdlib | Critical→Medium | OVERSTATED | The split-brain and duplication are real (working impls unreachable behind stubbed BuiltinFunction arms; 11-entry name drift), but rolling_mean/correlation/median are book-cautioned v0.4 with runnable=false — duplication debt, not undisclosed breakage. |
| Parser exponential blowup | Critical | CONFIRMED | ~3.5-4x per nesting level (shape.pest:1099 ordered choice); depth 12 exceeds 150s in run *and* check. |
| HashMap.filter garbage under JIT | Critical | CONFIRMED | Compiler rewrites arity-2 filter/map to filterIndexed/mapIndexed (function_calls.rs:4714); vm errors cleanly, default-JIT prints a different pointer-like int each run, exit 0, no diagnostic. |
| Drop-error guarantee unimplemented | Critical | CONFIRMED | op_drop_call_impl propagates drop errors with `?`; no log-and-continue path; remaining drops skipped, return value lost; the one unit test named for the guarantee never exercises an erroring drop. |
| Comptime descriptor key corruption | Critical | CONFIRMED | Nested row descriptors bind to the wrong TypedObject schema (e.g. json's is_valid/parse/stringify); `field.name`/`field.type`/`param.const` raise `Undefined property`, breaking the uncautioned target.fields contract. |
| state::hash constant digest | Critical | CONFIRMED (refined) | All args collapse to Bool(bits != 0) before hashing (marshal.rs:2295-2298) → only two possible digests; distinct values collide silently. |
| `__original__(args)` returns garbage | Critical | CONFIRMED (refined) | Compiler injects `let args = [params...]`; multi-param → arity compile error; single-param → array silently passed as int, printing run-varying heap-pointer garbage. |
| i64 overflow VM/JIT split-brain | Critical | CONFIRMED (gap pass) | Default-mode JIT wraps (plain iadd/isub/imul, rvalues.rs:1559-1563) where VM errors per the D3 ruling; `add_pair(i64::MAX,1)` → `-9223372036854775808` exit 0 under jit, error exit 1 under vm; conformance suite is VM-only so structurally blind. |
| serve --sandbox strict is a no-op | Critical | CONFIRMED (live escape) | `let _sandbox = config.sandbox;` (serve_cmd.rs:430); wire-executed `file::write_text` under strict mode wrote to disk, `success: true`. |
| Load-time permission check dead code | Critical | CONFIRMED | load_program_with_permissions/load_linked_program_with_permissions (program.rs:357,381): zero call sites; remote path uses plain load_program (remote.rs:743). |
| bigint unconstructible | Critical | CONFIRMED | No literal/annotation/cast/constructor path; payload is an Arc\<i64\> placeholder (heap_variants.rs:437-442); book recommends it 4× for arbitrary precision. |
| Drop broken at escape boundaries | Critical | CONFIRMED | Returned values never drop (functions.rs:2335-2348 skip is never re-armed); returned-closure captures drop prematurely (use-after-finalize: "dropped 9" prints before the closure successfully reads the capture). |
| Reference cycles leak unboundedly | High | CONFIRMED | Closure-in-captured-array cycle: 33MB→4.1GB over 20M iterations, no crash; no-cycle control flat; shipped binary excludes shape-gc entirely; book never mentions cycle leaks. |
| LSP falsely errors on valid extern C | High | CONFIRMED | LSP hardcodes `validate_type_annotations(true)` (diagnostics.rs:1533) demanding Result\<T\> the compiler (functions_foreign.rs:30) doesn't require for native ABI; book-verbatim code shows red E0400 in the editor while `shape check` passes. |

---

## 7. What verifiably works

The project deserves credit for a substantial verified core:

- **Strict typing is genuinely enforced by the shipped binary.** Default is `TypeDiagnosticMode::Strict` (compiler_impl_initialization.rs:141); all seven mandated probes are compile errors in run, check, and REPL; the catastrophic ReliableOnly pointer-reinterpretation bypass is gone and no unsoundness was reproduced through generics, fields, arrays, or Option laundering.
- **Core language is real end-to-end:** the full match/enum/Result/`?`/`!!`/`??` surface, the borrow checker (B0001/B0002/B0005/B0006/B0012 all fire, NLL last-use verified), traits per book syntax including supertraits, bounds, where clauses, and default methods; generics and let-generalization (old identity-returns-Null and typed-closure-in-array residuals fixed); cross-file relative imports in script mode (a previously known gap, now fixed).
- **Memory accounting is balanced on steady-state paths:** 20M-iteration allocation and closure soaks held RSS flat (32.8-36.0MB) in both modes — the hand-written clone/drop dispatch tables are refcount-correct where exercised; non-escaping Drop ordering is exactly right.
- **DateTime works completely** — every datetime.mdx example passed, including IANA timezones, leap-year clamping, and duration arithmetic. **Decimal arithmetic is exact** (0.1D + 0.2D = 0.3D, 28-digit division).
- **Numeric conversion ruleset (D1/D2, literal adoption) conforms in VM mode across all 12 probes**, backed by a real 117-case suite running in CI tiers.
- **The stdlib workhorses are solid:** file, csv, regex, unicode (including graphemes, better than documented), crypto (known-vector verified sha/hmac, ed25519 sign/verify with tamper rejection), compress (gzip/zstd/deflate roundtrips), core math/linalg/log/set/random, most of io including processes and TCP/UDP.
- **The Ed25519 signing chain works end-to-end** (keys generate → sign → verify; corrupted bundles refused at load), and **wire protocol round-trips genuinely work** against both servers (version/validate/execute, ping/execute) — the transport and codec layers are sound.
- **The JIT is real where it engages:** 3.6x on a 10M-iteration top-level loop, and the fallback discipline preserved correct output and exit codes in nearly all of 738 differential runs (the exceptions are itemized above).
- **Tooling substantially delivers:** projects, `.shapec` build, docstrings into hover, span-precise doc diagnostics, an LSP with 26 capabilities that survived every crash probe with a clean shutdown, an MCP server whose previously-reported module-docs gap is fixed, and `shape serve` TLS/loopback safety rails.
- **Verified non-drift:** the ADR-006 4-table HeapKind lockstep has zero current drift; W12 deletion is complete (TypedArrayData/TypedBuffer gone); the ADR-006 normative ABI checks (MethodFnV2 signature, parallel kind track, value-call KindedSlot ABI, sealed ProofGap) all pass in code.
- **Book availability banners, where present, are usually accurate** — snapshot/state/rolling/HashMap-methods cautions match binary behavior exactly; the failures are concentrated where cautions are missing or stale.

---

## 8. Top 10 recommendations (by leverage)

1. **Rebuild the foreign-call runtime or retract the feature from the book.** One handler (`op_call_foreign`) plus `native_abi::link/invoke` gates three documented verticals (Python, TypeScript, C). Until rebuilt, add a prominent unavailability caution to polyglot.mdx / python-extension.mdx / typescript-extension.mdx / native-c-interop.mdx, make linking lazy so declarations aren't fatal, and add an *Ffi/Native* permission before the path goes live. Re-enable a foreign e2e test in a CI tier so the path can never silently die again.
2. **Fix the JIT default-mode correctness class before anything else JIT-related** — or make `--mode vm` the default until done: (a) carve out or classify signal -1 so mid-run failures never re-execute the program; (b) emit checked i64 add/sub/mul per the D3 ruling; (c) route annotated functions through their wrappers; (d) fix the filterIndexed/mapIndexed rewrite. Stand up a permanent VM-vs-JIT differential harness (the 738-program diff found every divergence cheaply) and make the numeric-conversion suite run under both modes.
3. **Wire the security model end-to-end:** feed `shape.toml [permissions]/[sandbox]` and `serve --sandbox` into `granted_permissions`/`scope_constraints`; call `load_program_with_permissions` on the serve/remote paths; add a regression test asserting the live sandbox-escape write is refused; expose resource-limit flags on `shape run`; fix `wire-serve`'s PATH-dependent subprocess.
4. **Re-run the full book truth-gate at HEAD and fix both directions.** 158 failing examples plus a large stale-pessimistic cluster steering users away from working features (spread, comprehensions, NumericVec, trait bounds, set/random). Add missing cautions to stochastic/testing/property_testing/rotation/interpolation/optimize/msgpack, and fix mechanical doc errors (operators `// 4`, localhost vs IP, `shape bundle`, keys-trust flags, FORMAT_VERSION).
5. **Fix the comptime marshal root cause family in one pass:** the Bool-collapse in vec!-declared builtin args (comptime_builtins.rs:466-470) destroys `error()` messages, silences `warning()`, breaks `type_info()`, and drives `state::hash`'s constant digest; the schema-id collision corrupts every target descriptor. One marshal fix plus one schema-registration fix restores the entire documented derive/introspection story — and close the `set return` type-check bypass (a one-line verification hole with a segfault attached).
6. **Fix escape-boundary Drop per ADR-006 §2.7.30:** re-arm the caller-side drop obligation for returned values (functions.rs:2335-2348), extend escape detection beyond bare-identifier tail returns to closure captures, and implement the documented drop-error containment.
7. **Make async honest:** either implement deferred task semantics (closure-thunk `async let`, real race/any/cancellation) or rewrite async.mdx to describe the serial cooperative model; independently fix the async-fn module-call compile bug (function_calls.rs:3185) and the `time::sleep` block_in_place panic (modules.rs:733) — those two make async+I/O unusable regardless of semantics.
8. **Restore per-function remote transfer:** fix the 3-vs-2 `__call` arity mismatch, implement the `__call` body over the already-working Rust Call path, thread the upvalue kind track through the receiver, and replace the missing-blob panic with the never-constructed `RemoteErrorKind::MissingModuleFunction`. Add the missing tests for permission-in-hash and permission-union claims.
9. **Delete the dead mass and harden the gates:** remove or fix shape-gc (the `gc` feature doesn't compile), delete the 25 orphan files, ReliableOnly, IntrinsicsRegistry, the orphan-only `jit` feature + cranelift deps, and unused dependencies; rejoin `--benches` to `just check-clean` (the stated exclusion reason is stale); extend the verify-merge lockstep gate to the JIT retain/release tables and generate the JIT return-kind tables from the method registry instead of hand-syncing.
10. **User-facing polish with outsized trust impact:** fix the pest exponential backtracking (compiler unusable at 12 nesting levels); stop labeling compile errors "Runtime error"; strip internal audit jargon (ADR sections, ckpt-5, "REFUSED ON SIGHT") from user-facing error text; replace the leaked `Suspended on future 18446744073709551615` with a clear unavailability message; reconcile the 0.3.2/0.3.3 version split-brain; fix the LSP's false extern-C error and completion textEdits; document decimal's `D` suffix and either implement or remove bigint.
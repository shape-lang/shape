# Vertical Deep-Dive Audit 05: JIT Compiler (Cranelift) — `crates/shape-jit/`

Auditor: 05 of 19 (ultra-deep-dive, 2026-07-11)
Territory: `crates/shape-jit/` — mir_compiler/, compiler/, executor.rs, context.rs, ffi/, ffi_refs.rs, ffi_symbols/, plus the shape-vm tiering surface it plugs into (tier.rs, executor/osr.rs, deopt.rs) where needed to judge end-to-end reality.
Working tree state: DIRTY (audited as-is, commit ce332ca2 + uncommitted changes).

---

## 0. Executive summary

### Health verdict

The shape-jit crate is a **large (~69,200 LOC), heavily-engineered, honest-but-narrow AOT compiler wearing the clothes of a tiered JIT**. What actually ships behind the default `--mode jit` is a single ahead-of-time selective compile of the whole program (`compile_program_selective`), guarded by **eleven+ whole-program "SURFACE" deopt gates** that route most non-trivial language features (traits, `?`, `??`, object literals, user enums in `match`, scalar-returning string methods, module-binding access from function bodies, imported stdlib calls, user `Drop` impls, top-level `comptime`, generic-struct specializations, `as` casts, async) back to the bytecode interpreter. The famous tiering story — Tier 1 @ 100 calls, Tier 2 @ 10k calls, background compilation, OSR, feedback-guided speculation, deopt tables — is **fully implemented as data structures and unit tests but is dead in production**: `TierManager` is never constructed outside tests, and the two backend entry points it would call (`compile_single_function`, `compile_optimizing_function`) are hard `Err("deprecated")` stubs. Where the JIT does engage (numeric/int arithmetic, loops, direct calls, recursion, typed arrays, f-strings, string constants/equality), it was correct on almost everything I threw at it and measured **~18.6x faster than the interpreter** on an arithmetic loop — but probing also found **one live P0 silent-wrong-output divergence the gate inventory misses (Duration literals: JIT prints `0`, VM prints `PT129600S`, no fallback diagnostic — §9.0)**, which demonstrates the structural weakness of per-symptom gating. The engineering discipline around VM==JIT parity (surface-and-stop instead of silent wrongness) is genuinely good; the honesty of the *documentation* about what remains is genuinely bad.

### Top-10 findings

| # | Severity | Finding | Evidence |
|---|----------|---------|----------|
| 0 | **P0 (wrong results, live)** | Duration literals silently produce wrong output in the default mode: `print(1.5d)` → JIT `0` vs VM `PT129600S`, exit 0, no `[jit-fallback]`; MirToIR has zero Duration lowering and no gate rejects the carrier | Repro transcript §9.0; `grep Duration src/mir_compiler/*.rs` = 0 hits |
| 1 | **P0 (doc/reality)** | Tiered JIT (T1@100/T2@10k), background compilation, OSR, and feedback-guided speculation are dead in production: `TierManager` is never constructed outside tests/benches; `tier_manager` is `None` forever | `crates/shape-vm/src/executor/vm_impl/init.rs:82` (`tier_manager: None`, no `Some` assignment anywhere in `crates/`, `bin/`, `tools/`); `TierManager::new` callers: only `tier.rs` tests, `executor/osr.rs` tests, `benches/vm_benchmarks.rs` |
| 2 | **P0 (doc/reality)** | The tier-promotion backend entry points are unconditional `Err` stubs, so even a wired TierManager could never install whole-function native code | `crates/shape-jit/src/compiler/program.rs:587-601` (`compile_single_function` → `Err("Tier 1 JIT is deprecated")`), `:607-622` (`compile_optimizing_function` → `Err("Optimizing JIT is deprecated")`) — yet the book says "Tier 2 compilation is shipped: `compile_optimizing_function` … runs feedback-guided optimizing compilation" (`shape-web/book/book-site/src/content/docs/advanced/jit-compilation.mdx` note ~line 262) |
| 3 | **P1** | Default-mode JIT coverage of the language surface is narrow: ≥11 whole-program deopt gates send traits, `?`, `??`, object literals, user-enum `match`, string scalar methods, functions touching module bindings, imported stdlib fn calls, `impl Drop`, top-level comptime, and `::struct_` generic specializations to the interpreter | `crates/shape-jit/src/executor.rs:138-662` (8 gates), `compiler/program.rs:642-660` (module bindings), `:761-773` (`::struct_`), `compiler/strategy.rs:27-58` (comptime); all reproduced empirically in §2 with `[jit-fallback]` transcripts |
| 4 | **P1** | Silent `return TAG_NULL` arms remain in `jit_call_value` dispatch — a silent-wrong-output class only partially masked by upstream deopt gates | `crates/shape-jit/src/ffi/control/mod.rs:~690-720` (two `return TAG_NULL;` arms with only `tracing::debug!`), acknowledged as root cause in `executor.rs:440-462` (W17-marshal deopt text) |
| 5 | **P1** | OSR is triple-dead: tier_manager `None` + `Function.osr_entry_points` never populated by any producer + OSR-compile only requested via dead tier channel | `crates/shape-vm/src/executor/osr.rs:69-84` requires both; all 8 producers write `osr_entry_points: vec![]` (`execution.rs:1179`, `linker.rs:737`, `resume.rs:1005,1085,1137`, `snapshot.rs:1166`, `remote.rs:3215`) |
| 6 | **P1** | Two mutually-incompatible heap headers coexist: JIT `UnifiedValue{kind:u16@0, refcount@4}` vs VM `HeapHeader{refcount:AtomicU32@0, kind:u16@4}`; `read_heap_kind` on the wrong shape silently reads refcount bits as a kind. Convention-only guard | `crates/shape-jit/src/ffi/jit_kinds.rs:85-96` vs `crates/shape-value/src/heap_header.rs:44-54` |
| 7 | **P1 (ADR tension)** | Live runtime tag-bit discrimination in JIT-emitted code: `emit_index_to_i64` emits a `bits >= 0xFFF8…` runtime test to pick int-payload vs f64 decode for kind-unproven array indexes — ADR-006 §2.7.5 says "no runtime kind discrimination from the bits themselves" | `crates/shape-jit/src/mir_compiler/places.rs:522-550`, called from 4 array-access paths (`places.rs:600,653,696,711`) |
| 8 | **P2** | `compile_binop_dynamic_cmp` lowers kind-untyped `==`/`!=` as raw 64-bit bitwise compare — diverges from IEEE semantics for `-0.0 == 0.0` (bitwise false) and `NaN == NaN` (bitwise true) if kind-unproven f64s ever reach it | `crates/shape-jit/src/mir_compiler/rvalues.rs:1987-1995` |
| 9 | **P2** | The crate's only VM↔JIT differential-fuzz harness is compiled out entirely (`#![cfg(any())]`), and both `JitCodeCache` implementations (shape-jit and shape-vm copies — a duplication) have zero production call sites; every `shape run` re-JITs the ~118-function stdlib prelude from scratch | `crates/shape-jit/tests/differential_fuzz.rs:17`, `crates/shape-jit/src/jit_cache.rs` + `crates/shape-vm/src/blob_cache_v2.rs:124` (no `JitCodeCache::new` callers outside the defining files/tests) |
| 10 | **P2** | Book page `advanced/jit-compilation.mdx` contains fabricated API: `vm.register_jit_function`, `jit_dispatch_table`, `JitFnPtr` do not exist anywhere in shape-vm; `FunctionEntry::Pending` is never produced; "Background Compilation" text describes the dead tier path as live | grep across `crates/shape-vm/src` returns zero hits for all three names; `FunctionEntry::Pending` used only in `mixed_table.rs` self-tests |
| 11 | **P1 (latent, GC-on default)** | `jit_gc_safepoint` — the JIT half of the GC stop-the-world rendezvous (Phase 3b, commit 0c792bbf) — is defined, flag-wired, and documented as "called at every loop back-edge", but **no codegen site emits it**: it is absent from `FFIFuncRefs`, absent from every `ffi_symbols` registry and from `ffi_builder.rs`, and `loop_analysis`'s `needs_gc_safepoint` classification has zero consumers. JIT loops can never ack a cross-thread stop request | `ffi/gc.rs:14-30` vs `grep -rn safepoint src/` (only definition/comments/loop_analysis self-tests); `grep needs_gc_safepoint src/ --exclude loop_analysis.rs` → 0 hits; `grep jit_gc_safepoint src/ffi_symbols src/compiler` → 0 hits. Full chain in §9 B18 + Appendix B.3 |

### Scores

- **Feature completeness: 38/100.** The advertised feature set (tiered JIT + OSR + deopt + speculation + JIT cache) is ~20% reality; the actually-shipped feature (AOT selective compile of numeric/scalar code with honest interpreter fallback) works well but covers a minority of the language surface.
- **Code quality: 72/100.** Modern, well-commented, defensively-written Rust with a real surface-and-stop discipline, 849 in-crate tests, and panic containment at every boundary — held back by enormous dead-machinery mass, 647 unsafe blocks with only ~129 SAFETY comments, stale doc comments that contradict the code, and single functions/files of extreme size (types.rs 4,785 lines).

### Biggest risk

The biggest risk is **institutionalized aspirational documentation around tiering**: CLAUDE.md, the CLI `--help` text (`cli_args.rs:78-85`), executor.rs doc comments, and the book all describe a tiered, feedback-driven JIT with background promotion that does not exist at runtime — and one layer of the book ("JIT Dispatch Table", `register_jit_function`) describes API that has *never* existed in the tree. Every downstream consumer (release notes, MCP docs served to LLMs, performance claims to users) inherits this fiction. Meanwhile the real mechanism — eleven whole-program deopt gates — means a typical real-world program (uses a trait, or `?`, or an object literal, or a global) silently runs 100% interpreted while the user believes they are on the JIT path, with only a stderr line (that scrolls away) saying otherwise. Second-order risk: the silent `TAG_NULL` returns in `jit_call_value` and the convention-only separation of two heap-header layouts are exactly the class of silent-wrong-output/UB bug the SURFACE gates were built to prevent — the gates guard known entry points, not the mechanism itself.

---

## 1. Architecture & code structure map

### 1.1 Totals

`find crates/shape-jit -name '*.rs' | xargs wc -l` → **69,211 lines** across ~100 files.

| Module | LOC | Responsibility |
|--------|-----|----------------|
| `src/mir_compiler/` | 27,732 | **The live codegen path.** MirToIR: lowers the bytecode compiler's MIR (`top_level_mir` / `Function.mir_data`) to Cranelift IR. Split into places (reads/writes/addresses), rvalues (operators), statements, terminators, ownership (refcount emission), typed v2 lowerings (v2_array, v2_int, v2_field, v2_string, v2_refcount, v2_typed_map, v2_call_abi), bounds elision, conversions, plus 8 regression-test modules |
| `src/ffi/` | 17,889 | 237 `#[unsafe(no_mangle)] extern "C"` trampolines callable from JIT-emitted code: call_method (VM method-registry trampoline), control (jit_call_value, trampoline VM), typed_object, object/, v2/ (typed collection ops), conversion, string, result, arc (refcount FFI + leak counters), gc (safepoint + write barrier), simd, async_ops, value_ffi (NaN-box sentinel layout), jit_kinds (JitAlloc/UnifiedValue heap shapes) |
| `src/root files` | 7,957 | `executor.rs` (JITExecutor — the `--mode jit` entry), `context.rs` (JITContext ABI struct, 1,173 LOC), `core.rs` (legacy docs + kernel tests), `worker.rs` (dead tier backend), `osr_compiler.rs` (dead-in-prod OSR loop compiler), `loop_analysis.rs`, `jit_cache.rs` (dead), `mixed_table.rs`, `jit_array.rs`, `jit_matrix.rs`, `numeric_compiler.rs` (legacy), `ffi_refs.rs`, `foreign_bridge.rs`, `error.rs`, `lib.rs` |
| `src/optimizer/` | 5,356 | Staged analysis passes, module-level `#[allow(dead_code)]` (`lib.rs:54`): bounds, numeric_arrays, escape_analysis, vectorization, licm, loop_lowering, hof_inline, call_path, cross_function (Tier2CacheKey), table_queryable |
| `src/compiler/` | 4,912 | `JITCompiler` proper: program.rs (compile_program_selective — the real pipeline), strategy.rs (top-level body compile), accessors.rs (bytecode preflight), ffi_builder.rs (declares the 178 FFI symbols into Cranelift), setup.rs, kernel_ir.rs + deep-tests-gated a1d2/a1e/c2 test modules |
| `src/ffi_symbols/` | 4,489 | Named-symbol registry for series/vector/simulation/math/object intrinsics; module-level `#[allow(dead_code)]` (`lib.rs:36`) — much staged ahead of call sites |
| `tests/` | 876 | Two integration files; the differential fuzz harness is `#![cfg(any())]`-disabled (`tests/differential_fuzz.rs:17`) |

### 1.2 Execution data flow (what actually runs)

```
shape run file.shape          (default --mode jit, cli_args.rs:89-96)
  └─ JITExecutor::execute_program                (executor.rs:88)
       ├─ REPL? → interpreter                     (executor.rs:111)
       ├─ top-level comptime? → interpreter       (executor.rs:138)
       ├─ compile bytecode via the SAME pipeline as --mode vm
       │    (compile_program_for_inspection, executor.rs:186)
       ├─ trait/impl declared? → interpreter      (executor.rs:210)
       └─ execute_with_jit                        (executor.rs:321)
            ├─ 7 more whole-program deopt gates   (executor.rs:350-662, §2.3)
            ├─ JITCompiler::compile_program_selective   (program.rs:632)
            │    ├─ module-binding-in-fn gate     (program.rs:642)
            │    ├─ per-function preflight: bytecode preflight OR MIR preflight
            │    ├─ ::struct_ specialization gate (program.rs:761)
            │    ├─ declare all fns, compile main (compile_strategy_with_user_funcs)
            │    ├─ Phase 4: MirToIR per compatible fn; failures demoted
            │    └─ finalize; unresolved ref → clean compile-stage Err (program.rs:914)
            ├─ link foreign fns; build JITContext; link function table + names
            ├─ clone bytecode into a fresh trampoline VM  (executor.rs:828-836)
            ├─ run native code: signal = jit_fn(&mut jit_ctx)   (executor.rs:898)
            ├─ negative signal → carved-out VM-parity errors    (executor.rs:962-1039)
            └─ result via return_type_tag (typed) else surface-and-stop (executor.rs:1047-1108)
Fallback on any Err: eprintln "[jit-fallback] …" + execute_compiled(same bytecode) (executor.rs:262-300)
```

### 1.3 Key types

- **`JITContext`** (`context.rs:560`): the `repr` ABI struct passed to every jitted function. Carries a NaN-boxed `locals: [u64; 256]`, `stack: [u64; 512]` **plus a parallel `stack_kinds: [u8; 512]` track** (context.rs:594, the JIT analog of the VM's §2.7.7 parallel-kind vec), function table pointer/len, function-names pointer (for UFCS method lookup), exec-context pointer, foreign-bridge pointer, gc safepoint flag pointer, async suspension fields, and simulation/dataframe fields (`column_ptrs`, `row_count` — the crate's backtesting heritage, see §3.4).
- **`JittedStrategyFn = unsafe extern "C" fn(*mut JITContext) -> i32`**: signal-returning convention. Negative signals: `-2` div-by-zero, `-3` trampoline error, `-4` index OOB, `-5` int overflow (`context.rs:90-140`); each is mapped to the exact interpreter diagnostic in `executor.rs:972-1032`.
- **`MixedFunctionTable`** (`mixed_table.rs:13`): `Native(*const u8) | Interpreted(u16) | Pending(u16)`. `Pending` is **never produced** by the compiler — only `Native`/`Interpreted` are inserted (program.rs:960/967); `Pending` exists for the dead background-promotion story.
- **`MirToIR`** (`mir_compiler/mod.rs`): per-function lowering context; carries slot-kind side tables (`Vec<Option<NativeKind>>`), concrete-type conduit (`top_level_local_concrete_types`), shared/owned capture side-tables (A.1D.2/A.1E), and the FFI func-ref bundle built by `ffi_builder.rs` (178 symbols).
- **`JitAlloc<T>` / `UnifiedValue<T>`** (`ffi/jit_kinds.rs:67-96`): JIT-private heap shapes; `kind: u16` discriminator at offset 0, data at offset 8. See §5.2 for the header split-brain with shape-value's `HeapHeader`.
- **Return-tag protocol**: jitted main stamps `jit_ctx.return_type_tag` ∈ {F64, I64, I32, BOOL, STRING, UNIT}; `RETURN_TAG_NANBOXED` (0) reaching the host boundary is an explicit surface-and-stop error, not a decode (executor.rs:1070-1108).

### 1.4 Entry points

1. **`JITExecutor::execute_program`** — the only production entry (`bin/shape-cli/src/commands/script_cmd.rs:1589`, `repl_cmd.rs:527`, `repl/mod.rs:487`; REPL immediately delegates to the interpreter, executor.rs:111).
2. **`JitCompilationBackend`** (worker.rs) — implements shape-vm's `CompilationBackend` for the TierManager; **unreachable in production** (§2.4).
3. **`compile_osr_loop`** (osr_compiler.rs:971 LOC) — real Cranelift loop compiler with OsrEntryPoint metadata; reachable only through the dead backend.
4. **Simulation kernels** (`compile_simulation_kernel`, `compile_correlated_kernel`) — a separate `fn(cursor, series_ptrs, state) -> i32` ABI for backtesting; tested in core.rs but no CLI surface found in-territory.

### 1.5 Largest files (wc -l, working tree)

| LOC | File | Content |
|-----|------|---------|
| 4,785 | mir_compiler/types.rs | NativeKind↔Cranelift type mapping; builtin-method return-kind classification tables; `registry_cross_check` |
| 2,471 | mir_compiler/terminators.rs | `compile_terminator` (one ~2,140-line fn: calls, returns, branches, trait emission, return-kind stamping) |
| 2,282 | mir_compiler/integration_tests.rs | deep-tests-gated end-to-end source→JIT tests (151 `#[test]`s) |
| 2,270 | mir_compiler/mod.rs | MirToIR struct, preflight, side-tables (grew +517 lines in the dirty tree) |
| 2,107 | mir_compiler/rvalues.rs | binop/unop/cast lowering incl. typed f64/i64/i32/bool paths |
| 1,940 | mir_compiler/statements.rs | StatementKind lowering + ObjectStore/schema stamping |
| 1,939 | ffi/v2/mod.rs | typed v2 collection FFI |
| 1,902 | ffi/conversion.rs | JIT↔runtime value conversion (33 tests) |
| 1,893 | mir_compiler/places.rs | place read/write/address, inline array access, write barriers |
| 1,695 | ffi/object/closure.rs | closure heap objects (25 tests) |
| 1,609 | compiler/accessors.rs | bytecode preflight: supported-opcode table + `vm_only_opcode_reason` + parity matrices |
| 1,490 | mir_compiler/v2_array.rs | TypedArray fast-path codegen |
| 1,399 | ffi_symbols/object_symbols.rs | object FFI symbol registry |
| 1,368 | core.rs | legacy docs, kernel + width-aware tests (mostly `#[ignore]`) |
| 1,326 | ffi/call_method/mod.rs | `jit_call_method` VM-registry trampoline |

### 1.6 FFI reference architecture

Three layers keep the 237 extern symbols organized: (1) `ffi_refs.rs` — the **34-field `FFIFuncRefs`** bundle of Cranelift FuncRefs actually referenced by MirToIR codegen (down from ~240 historic fields; R7.1 deleted the 11 `generic_*` dynamic-dispatch trampolines, R7.3 audited all 34 as live — ffi_refs.rs:1-23); (2) `ffi_symbols/` — name→address registration into the JITBuilder; (3) `compiler/ffi_builder.rs` — declares 178 symbols (`r!` count) into each module compilation. A new helper must be touched in at all three (documented at ffi_refs.rs:19-23) — a three-file ceremony but with clear instructions.

### 1.7 Dirty working-tree delta (audited as-is)

`git diff --stat -- crates/shape-jit` at audit time: **19 files, +2,042/−324** — an active, uncommitted workstream. Characterization from the diff content:

- `mir_compiler/ownership.rs` (+414): a new fixed-point dataflow analysis (`mir_move_then_read_divergence_reason`) — reverse-postorder in/out move-sets over the CFG to detect move-then-read divergences precisely (replacing a coarser heuristic; this is the "aliased-CoW" containment class).
- `mir_compiler/mod.rs` (+517): ObjectStore schema-stamp SURFACE machinery (`unsupported_object_store_schemas`, `object_store_alloc_surface`, per-place/operand/rvalue schema stamp collection) — i.e., narrowing the object-store deopt from blanket to schema-aware.
- `ffi/call_method/mod.rs` (+152), `ffi/typed_object/{allocation,field_access}.rs` (+129 combined): kinded receiver/field-access hardening.
- `executor.rs` (+91): the `??` null-coalesce gate (`has_null_coalesce_residual`) is part of the uncommitted delta.

Direction of travel is consistent with the containment strategy: more precise gates, kinded FFI hardening. No tiering work is in flight.

---

## 2. Feature completeness

Method: every claim below is either (a) an empirical run of the prebuilt working-tree binary (`target/debug/shape run`, default `--mode jit`), with the `[jit-fallback]` stderr line as ground truth for "did it actually JIT", or (b) a file:line cite. "CODE EXISTS" ≠ "WORKS END-TO-END" is the organizing distinction.

### 2.1 What actually JIT-compiles and runs natively (verified end-to-end)

All of the following ran with **no** `[jit-fallback]` line and produced VM-identical output:

| Program shape | Result | Notes |
|---|---|---|
| `for` loop, int arithmetic, `let mut`, fn call (`hot(1000000)`) | `499999500000` ✓ | |
| Arithmetic + `%` in 20M-iteration loop | `159999992` ✓, **jit: 3,107 ms vs vm: 57,813 ms (~18.6x)** | debug-build binary; Cranelift output quality is build-mode independent, interpreter dispatch is not — treat the ratio as an upper bound |
| Recursion (`fib(25)`) | `75025` ✓ | direct user-fn calls JIT (program.rs Phase 2 pre-declaration enables JIT→JIT calls) |
| `while` + `if/else` + `%` (`collatz(27)`) | `111` ✓ | |
| Numeric array literal + index in loop (`arr[i]`) | `7.0` ✓ | typed-array fast path (v2_array.rs) |
| String constant + `print` | `hello world` ✓ | string constants JIT; string *methods* do not (§2.3) |
| Division by zero | `Error: Runtime error: Division by zero` ✓ | signal `-2` carve-out, executor.rs:973; **loses `(line 1)`** vs VM |
| `int` overflow (`i64::MAX * 3`) | structured overflow error ✓ | signal `-5`, executor.rs:994; **loses operand values + line** vs VM's "integer multiplication overflow: result of 9223372036854775807 and 3 …" |

### 2.2 Tiering / OSR / deopt / speculation — CODE EXISTS, PRODUCTION-DEAD

This is the central completeness finding. Chain of evidence:

1. `VirtualMachine.tier_manager: Option<TierManager>` is initialized `None` (`crates/shape-vm/src/executor/vm_impl/init.rs:82`) and **no assignment to `Some(...)` exists anywhere** in `crates/`, `bin/`, `tools/` (grep for `tier_manager = `, `TierManager::new` — hits only in `tier.rs` tests, `executor/osr.rs` tests, `benches/vm_benchmarks.rs:403,417`).
2. Therefore `TierManager::record_call` (tier.rs:214, the T1@100/T2@10k promotion logic), `record_loop_iteration` (tier.rs:425, OSR trigger @1000 back-edges), `poll_tier_completions` (init.rs:310, called every 1024 instructions — a permanent no-op), and `current_feedback_vector` (init.rs:346, returns `None` when tier_manager is `None` → **feedback vectors are never allocated**, so the IC state machine in feedback.rs never runs in production) are all dead.
3. Even if wired: `JitCompilationBackend::compile_function` (worker.rs:201) calls `compile_single_function` / `compile_optimizing_function`, which are unconditional stubs: `Err("Tier 1 JIT is deprecated")` (program.rs:600), `Err("Optimizing JIT is deprecated")` (program.rs:621). Only the OSR branch (worker.rs:39, → `osr_compiler::compile_osr_loop`) has a real body.
4. Even if OSR compiled: `try_osr_entry` (executor/osr.rs:69-95) needs BOTH `tier_manager.get_osr_code(...)` AND a matching `func.osr_entry_points` entry — and every producer of `Function` writes `osr_entry_points: vec![]` / `Vec::new()` (`execution.rs:1179`, `linker.rs:737`, `resume.rs:1005/1085/1137`, `snapshot.rs:1166`, `remote.rs:3215`). The bytecode compiler never emits OSR entry metadata.
5. Deopt: `DeoptTracker` (shape-vm/src/deopt.rs) and `DeoptInfo` tables are populated only from `CompilationResult.deopt_points` via `poll_completions` — same dead channel. The MirToIR path emits no deopt guards (its failure mode is compile-time `Err`, not runtime guard exits).

Verdict: **Tier structure = implemented+tested library code; runtime tiering = 0% shipped.** The one live "tier" is the AOT selective compile at startup.

### 2.3 The deopt-gate inventory — what falls back to the interpreter (verified)

Whole-program gates, in evaluation order, each producing an `eprintln!` `[jit-fallback]` + interpreter run of the same bytecode:

| Gate | Where | Trigger | Empirically confirmed |
|---|---|---|---|
| REPL persistence | executor.rs:111 | any REPL cell | (by code read) |
| Top-level `comptime` | executor.rs:138 + strategy.rs:27-58 | `program_has_top_level_comptime` | (by code read; exactly-once side-effect rationale) |
| **Trait/impl declared** | executor.rs:210-234 | ANY `trait` or `impl` item in source | ✓ `trait Greet`/`impl Greet for Dog` → fallback, prints `woof` |
| V2 typed-opcode verifier failure | executor.rs:350 | `verify_v2_typed_opcodes` errors | (by code read) |
| Imported-const inline | executor.rs:387 | `has_imported_const_inline` | (by code read) |
| **W17 marshal residual** | executor.rs:440 | direct calls to imported stdlib fns | ✓ trivial closure `let add1 = \|x: int\| x + 1; print(add1(41))` deopts via this flag |
| **TryUnwrap `?`** | executor.rs:505 | `has_try_unwrap_residual` | ✓ `parse("42")?` → fallback, prints `43` |
| Reference-escape promotion | executor.rs:552 | `has_reference_escape_promotion` | (by code read; PromotedCell has no JIT lowering) |
| **Null-coalesce `??`** | executor.rs:582 | `has_null_coalesce_residual` | ✓ `maybe(true) ?? 0` → fallback, prints `7` |
| **Scalar-move-lift** | executor.rs:597 + executor.rs:10-37 | `NewObject` opcode, `clone`/`diff` typed methods, operator-trait dispatch sites | ✓ object literal `{name:"x", value:42}` → fallback, prints `42` |
| **User `impl Drop`** | executor.rs:639-662 | any `Drop::*` in trait_method_symbols | (by code read; JIT `emit_drop` releases refcounts but never dispatches user Drop bodies — ownership.rs) |
| **Module-binding in fn body** | program.rs:642-660 | any function containing Load/StoreModuleBinding | ✓ `fn add_base(x) { x + base }` with top-level `let base` → fallback, prints `105` |
| `::struct_` generic specialization | program.rs:761-773 | generic free fn specialized on struct arg | (by code read; UAF rationale) |
| MIR preflight blockers | mir_compiler/mod.rs:603-780 | `Pattern::Typed`, user-enum `Pattern::Constructor` discriminants, `EnumPayload` binders (SIGSEGV rationale documented at mod.rs:~700), complex places, `ClosureCapture` w/o fn id | ✓ `match` on user enum → "Route A surface-and-stop … enum constructor `Shape2::Square`" |
| Scalar-returning string methods | (MirToIR compile error) | `.len()` etc. on proven String receiver | ✓ `s.len()` → fallback: "NaN-boxed f64 … NEITHER is the raw native scalar the proven destination slot expects" |
| Phase-4 fn-compile failure w/ native reference | program.rs:899-940 | e.g. `Vec.map` closures | ✓ `arr.map(\|x\| x*2.0)` → "finalize could not resolve … main_f196_Vec.map__f64_f64_closure_0" |

**Net effect**: programs using traits, `?`, `??`, object literals, user enums in match, closures-through-stdlib-HOFs, string methods, globals-from-functions, or user Drop run **fully interpreted** under the default mode. The intersection that truly JITs — scalar numerics, control flow, direct calls, numeric arrays, string constants — is real but narrow.

### 2.4 Partial / stubbed / missing inside the JIT itself

- **`jit_call_value` ModuleFn dispatch**: `dispatch_module_fn_call` is `todo!()`-class per executor.rs:435-439 comment; the live arm returns `TAG_NULL` silently (`ffi/control/mod.rs:~690-720`) — the W17 gate exists precisely to mask this.
- **Simulation/vector/matrix intrinsics**: `ffi_symbols/vector/mod.rs:30` `todo!("phase-2c …")`; `ffi/call_method/matrix.rs:72,80,88,96` `todo!`; `ffi/object/object_ops.rs:114,136` `todo!` — extern "C" `todo!()` bodies **abort the process** (can't unwind), acknowledged in the `#[ignore]` text at `ffi_symbols/simulation/mod.rs:118`.
- **JIT code caching**: two `JitCodeCache` implementations (shape-jit/src/jit_cache.rs — rich, with dependency invalidation + Tier2CacheKey; shape-vm/src/blob_cache_v2.rs:124 — trivial) — **zero production call sites for either**. Every `shape run` re-JITs everything including the stdlib prelude (the known deep-tests root cause: "JIT-compiles ~118 stdlib functions per test", Cargo.toml:33-40).
- **Optimizer passes** (escape analysis, LICM, vectorization, HOF-inline, loop lowering): compiled, unit-tested, but the module is `#[allow(dead_code)]` (`lib.rs:54`) — staged "ahead of their drive sites". `CallPathPlan` IS consulted by strategy compilation (call_path.rs is referenced from program lowering), but the vectorization/LICM passes have no production driver I could find.
- **Async**: `ffi/async_ops.rs` exists (316 LOC) with suspension fields in JITContext (context.rs:612-620), but async programs carry MIR shapes that fail preflight → interpreter (consistent with prior audits' async findings; not re-verified end-to-end here).

### 2.5 GC two-tier wiring (focus-note verification) — CORRECT at working tree

The memory-flagged trap ("shape-vm/gc does not forward to shape-jit/gc") is **fixed as of ce332ca2 and holds in the dirty working tree**:

- `bin/shape-cli/Cargo.toml:13` `default = ["jit", "gc"]`, `:26` `gc = ["shape-vm/gc", "shape-jit?/gc"]` — weak-dep forwarding covers the JIT tier whenever `jit` (default) is on.
- `crates/shape-vm/Cargo.toml:57` `default = ["jit", "gc"]`, `:76` `gc = ["shape-value/gc"]`.
- `crates/shape-jit/Cargo.toml` `gc = ["shape-value/gc"]`, default `[]` — correct: the crate alone defaults off, the shipped binary turns it on via shape-cli.
- JIT hooks are real under the feature: `jit_gc_safepoint` parks on the coordinator at loop back-edges (`ffi/gc.rs:30-52`), `jit_write_barrier` forwards to `shape_value::gc::gc_jit_write_barrier` (`ffi/gc.rs:67-74`); barrier emission sites at `mir_compiler/places.rs:788,1381`, `ffi/data.rs:476`, `ffi/typed_object/field_access.rs:100`, `ffi/object/object_ops.rs:98`.
- One doc-rot nit: `ffi/gc.rs:9-10` still opens with "No-op stubs kept for JIT codegen call-site compatibility; … no tracing collector exists" — contradicted by the real Phase-3b body 20 lines below (P2).

### 2.6 Second probe batch — the JIT/deopt frontier in finer grain

| Program | JIT natively? | Evidence / deopt reason |
|---|---|---|
| `let x = 3 as number; print(x * 2.0)` | **NO** | WS-12: `ConvertTo*`/`TryConvertTo*` are VM-only — the entire `as`-cast opcode family is unlowered (accessors.rs:692-704). Output correct via fallback: `6.0` |
| f-string interpolation `print(f"value is {n}")` | **YES** | `value is 42`, no fallback — string interpolation lowers natively (notable positive) |
| `type Point {x,y}` construct + `p.x + p.y` | **NO** | `compile_binop_dynamic_arith: kind-untyped arith Add … producing-MIR kind-tracker gap` — typed-struct **field reads don't yield proven kinds** at top level, so even declared-`number` fields can't feed typed arithmetic |
| `match n { 0 => …, 1 => …, _ => … }` on int | **YES** | `200`, no fallback — literal int patterns lower natively |
| `match maybe(true) { Some(v) => …, None => … }` | **NO** | MIR preflight: `EnumPayload … receiver-recovery soundness gap at the user-fn return-kind boundary per ADR-006 §2.7.17` — even the **trinity** (`Some`/`Ok`/`Err`) payload binders deopt when the scrutinee crosses a user-fn return |
| `async fn` + `await` | **NO** | bytecode preflight: `vm_only_opcodes: [Await]` — the whole async opcode family is VM-only (accessors.rs `is_vm_only_async_opcode`) |
| `loop { break 42 }` value-break | **YES** | `42`, no fallback |
| string equality `a == b` | **YES** | `1`, no fallback (`EqString` typed opcode natively lowered) |
| `arr.len()` on `Array<number>` | **YES** | `3`, no fallback — array `.len()` has native codegen (`v2_array_len`, v2_array.rs:1398) while *string* `.len()` deopts (§2.3): same method name, different receiver lowering maturity |
| `arr.push(3.0)` + read-back | **YES** | `3.0`, no fallback (`emit_v2_array_push_call`, v2_array.rs:648) |
| inclusive range `for i in 0..=10` | **YES** | `55`, no fallback |
| multi-return-`string` fn (`grade(85)`) | **YES** | `B`, no fallback — string-typed returns work when kinds are proven |
| `number` params/returns (`avg(3.0,5.0)`) | **YES** | `4.0`, no fallback |
| generic fn `identity<T>(x: T)` monomorphized | **NO** | "Route A SURFACE — direct call to `identity` … has no compile-time-proven FrameDescriptor.return_kind. W36 named-function callgraph requires a static return-kind" — generic instantiations lack stamped return kinds |
| **duration literal `print(1.5d)`** | **RUNS NATIVELY AND PRINTS THE WRONG ANSWER** | JIT: `0` / VM: `PT129600S`, both exit 0, **no `[jit-fallback]` line** — a live, un-gated silent VM≠JIT divergence at working tree (finding **B0**, §9.0) |

Combined with §2.1/§2.3, the real frontier is: **scalar numerics, bool logic, control flow (including value-`break` and int-literal `match`), direct calls/recursion, numeric arrays, f-strings, string constants JIT; everything that touches heap-shaped carriers across a boundary (enum payloads, struct fields into arithmetic, casts, closures via HOFs, methods returning scalars from heap receivers) deopts.**

### 2.7 Bytecode-preflight inventory (`vm_only_opcode_reason`, accessors.rs:613-733)

The full list of opcodes that force interpreter routing at the bytecode-preflight layer, each with a written reason:

1. **All async opcodes** (`is_vm_only_async_opcode` → `VM_ONLY_ASYNC_REASON`).
2. **`AllocSharedModuleBinding` / `LoadSharedModuleBinding` / `StoreSharedModuleBinding`** — "A.1C.3 outer-scope Shared module-binding; Cranelift lowering pending".
3. **The 12-opcode `ConvertTo*`/`TryConvertTo*` family** — with an honest confession embedded: the old FFI trampoline (`ffi/generic_builtin.rs::dispatch_opcode`, :169-181) is a deliberate pass-through that returns operand bits unchanged, so a JIT'd `x as int` used to "silently yield the unconverted value (`true` instead of `1`, or a raw `Ptr(Option)` pointer printed as garbage)" (accessors.rs:666-680). The stub is still live code; only the preflight gate keeps it unreachable.
4. **`CallForeign`** — deliberate permanent routing to the shared interpreter core `invoke_foreign_kinded` so polyglot semantics *cannot* diverge by construction (ffi-rebuild §4.9 J1; out-of-line lowering J2 deferred as pure perf).

Everything else in the opcode set is claimed supported (`ALL_OPCODES`-driven `all_opcodes_pass_preflight` invariant, accessors.rs:278-283), with `is_supported_builtin` now returning `true` unconditionally (accessors.rs:735-739) — builtins route through dedicated lowerings or the generic builtin trampoline.

Note the layering: opcode-preflight (this list) → MIR-preflight (§2.3 blockers) → executor gates (§2.3 table) → Phase-4 per-function compile failures → finalize deopt. Five sieves, maintained in four files.

### 2.8 MirToIR lowering coverage (MIR-node level)

- **StatementKind** arms handled in statements.rs: `Assign`, `Drop`, `ModuleBindingStore`, `Nop`, `TaskBoundary` — the full current StatementKind surface (no wildcard-swallow).
- **TerminatorKind** arms in terminators.rs: `Call`, `Goto`, `Return`, `SwitchBool`, `Unreachable` — complete for the MIR's terminator vocabulary.
- **Rvalue** variants referenced in rvalues.rs: `Use`, `BinaryOp`, `UnaryOp`, `Aggregate`, `Borrow`, `Clone`, `PrimitiveCast`, `FuzzyComparison`, `EnumTest`, `EnumDiscriminantTest`, `EnumPayload`, `TypePatternTest` — of which `EnumDiscriminantTest`/`EnumPayload`/`TypePatternTest` are surface-and-stop (also preflight-rejected, §2.3) and `Aggregate` partially surfaces (heterogeneous-element carriers → Route A SURFACE, per the enum-constructor transcript).
- So MirToIR is **structurally exhaustive** over MIR (every node kind has an arm) but **semantically partial** (several arms are refusals). The structural exhaustiveness is the right shape: adding a MIR variant forces a JIT decision at compile time.

### 2.9 The VM-boundary trampolines in detail

The two load-bearing runtime bridges deserve precise description, since every JIT'd program that calls a non-JIT'd function crosses them:

**`jit_call_value`** (`ffi/control/mod.rs:~350-830`, extern "C"): the indirect-call dispatcher. Its 60-line doc comment (control/mod.rs:350-413) is the crate's best ADR-006 conformance essay, naming the three callee shapes it accepts — (1) inline function ids (`TAG_FUNCTION` NaN-box, produced for bare `FunctionRef` constants), (2) raw-Arc closures (`Arc::into_raw(Arc<HeapValue::ClosureRaw>)` with `NativeKind::Ptr(HeapKind::Closure)` sourced from the `stack_kinds` lockstep track — explicitly NOT probed from the bits), (3) legacy `unified_box(HK_CLOSURE, JITClosure)` — and enumerating the "forbidden alternatives (refuse on sight)": tag-bit callee probes, Bool-default kinds, silent no-ops, ValueWord resurrection. Dispatch preference: JIT function-table direct call (uniform I64 Cranelift signature, ≤8 args) → trampoline VM (`dispatch_call_via_trampoline_vm`) for null-slot callees, carrying `(bits, kind)` pairs as `KindedSlot`s. The trampoline VM is the full interpreter loaded with a clone of the unlinked bytecode (executor.rs:828-836) so function-ids agree.

**`jit_call_method`** (`ffi/call_method/mod.rs:599-~1300`, extern "C"): the method dispatcher. Pops `arg_count` (raw i64), the method name (unified-heap `Arc<String>`, kind-track-verified as `NativeKind::String`, refusing SENTINEL — mod.rs:637-670), then args as (bits, kind) pairs; classifies the receiver; tries UFCS user-method lookup via the function-names table (`find_function_by_name("Type::method")`); otherwise dispatches into the VM's PHF method registry. Scalar results come back NaN-boxed (`box_number`/`TAG_BOOL_*`) — the §5.2 ABI-conversion point, and the reason scalar-returning string methods must deopt.

**The HOF FFI family is a field of armed `todo!()`s**: `jit_control_fold`, `jit_control_reduce` (delegates to fold), `jit_control_map`, `jit_control_filter`, `jit_control_foreach`, `jit_control_find` (control/mod.rs:843-900+) are all extern "C" `todo!()` bodies pending the "kinded TypedArray\<T\> rebuild per ADR-006 §2.7.6/Q8" — SIGABRT if ever reached (same class as B11). Today `arr.map(...)` never gets that far (Phase-4 closure-compile failure deopts first, §2.3), i.e. the abort is shielded by an *unrelated* failure. If closure compilation is fixed before the HOF rebuild, these stubs become reachable process-kills.

### 2.10 The OSR compiler: high-quality, triply-unreachable

`osr_compiler.rs` (971 LOC) deserves its own note because it is the most complete piece of the dead tier stack:

- **Supported opcode set** (grep `OpCode::` in the file): 64 opcodes — the full typed arithmetic/comparison family (`AddInt/AddNumber/…/EqString/GteString`), `PowInt/PowNumber`, `CastWidth`, `IntToNumber`/`NumberToInt`, locals/module-binding load/store (incl. typed variants), loop control (`LoopStart/LoopEnd/Break/Continue`), jumps, `BuiltinCall`, `Dup/Swap/Pop`. That is a wider *bytecode* menu than the legacy direct-compile path retained.
- **Entry protocol**: snapshot live locals into a context buffer (`JIT_LOCALS_CAP = 256`, locals at byte offset 64 — osr_compiler.rs:45-50), run the compiled loop, on success return the exit-IP branch; on **deopt return `u64::MAX`** with all live locals stored back so the interpreter resumes coherently (osr_compiler.rs:938-950 emits the deopt block).
- **The interpreter half** (`try_osr_entry`, shape-vm executor/osr.rs:55-120) mirrors this faithfully — including a guard for stale metadata and kind-checked local snapshots per §2.7.5.1 (panics on out-of-range `local_kinds` as "verifier bug").
- And none of it can run: §2.2 items 1/3/4 (no TierManager, tier backend delivers OSR results into a channel nobody polls meaningfully, `osr_entry_points` never populated). Two of its four worker-level tests still run (worker.rs:477-651, `test_backend_osr_compiles_simple_loop`, `test_backend_osr_blacklists_unsupported_loop`) — green tests over unreachable machinery.

---

## 3. Code quality

### 3.1 Idiom & error handling

- The crate is consistently `Result<_, String>`-based for compile paths — every lowering failure carries a long, cite-rich message naming the ADR clause, the audit doc, and the follow-up workstream. This is unusual and mostly good (the messages ARE the institutional memory), but it means error identity is stringly-typed: `JitError` (error.rs, 41 LOC) exists yet nearly everything returns `String`, so callers match on substrings or nothing. `executor.rs` wraps into `ShapeError::RuntimeError` with `location: None` — which is why JIT-path runtime errors **lose line numbers** (verified: div-by-zero under jit lacks the VM's `(line 1)`; §2.1).
- Panic containment is deliberate and layered: `catch_unwind` around `compile_program_selective` (executor.rs:698) and around `finalize_definitions` with panic-hook suppression (program.rs:914-919). The `TrampolineGuard` drop guard (executor.rs:841-847) unsets the thread-local trampoline VM even on panic. Good.
- The nested-result contract (`Result<Result<T>>`) in `execute_with_jit` (executor.rs:305-321) to separate compile-stage failure (fall through, re-run) from soundly-executed runtime error (propagate, do NOT re-run — would double side effects) is a subtle correctness fix done properly and documented with its motivating bug (r5c-2-gz-cp2-jit-div).

### 3.2 Unsafe usage

Counts (grep, src/ only): **647 `unsafe {` blocks, 69 `unsafe fn`, 7 `unsafe impl`, 237 `#[unsafe(no_mangle)]` extern "C" symbols — vs ~129 `SAFETY`/`# Safety` comments.** So roughly 1 in 5 unsafe sites carries a written justification.

- The bulk is inherent: 237 FFI trampolines take raw pointers from JIT-emitted machine code; `lib.rs:25-30` documents the module-wide `#[allow(clippy::not_unsafe_ptr_arg_deref)]` decision with a coherent rationale (the deref contract is upheld by the codegen site).
- The riskiest idioms:
  - `unsafe fn from_heap_bits(bits: u64) -> &'static Self` (jit_kinds.rs:~126) — fabricates `&'static` from raw bits; callers "vouch" kind consistency per §2.7.5. Any producer stamping the wrong kind is instant UB with no debug-mode check beyond a null assert.
  - `unsafe impl Send for CompilationResult/TierManager/JitCompilationBackend/JitCodeCache` (tier.rs:129,170; worker.rs:189; jit_cache.rs; blob_cache_v2.rs:129-130) — raw-pointer Send blessings justified by single-threaded-use comments; acceptable but each is a latent footgun if threading changes.
  - `std::mem::transmute` of the finalized code pointer to `JittedStrategyFn` (program.rs:971) — standard JIT practice, fine.
  - extern "C" `todo!()` bodies (matrix.rs:72-96, vector/mod.rs:30, object_ops.rs:114/136): `todo!` panics cannot unwind across `extern "C"` → **SIGABRT process kill**, acknowledged in `ffi_symbols/simulation/mod.rs:118`'s ignore text. These are reachable only from codegen that currently never emits calls to them, but they are armed process-aborts sitting in a library.

### 3.3 Complexity hotspots

- **`compile_terminator` (mir_compiler/terminators.rs:95-~2239): a single ~2,140-line function** — the file has exactly one other pre-95 method. It handles every call/return/branch shape including trait-method emission, trampoline dispatch, return-kind stamping. This is the highest-risk maintenance surface in the crate.
- `jit_call_method` (ffi/call_method/mod.rs:599-~1300): ~700-line extern "C" trampoline doing receiver classification, UFCS user-method lookup, VM method-registry dispatch, and result reboxing.
- `mir_compiler/types.rs`: 4,785 lines, mostly return-kind classification tables for builtin methods + `registry_cross_check` test (types.rs:3605) that iterates shape-vm's PHF registry to keep tables honest — the right mitigation for a table this size.
- `compile_program_selective` (program.rs:632-973): 340 lines, 5 phases, inline gate logic; readable but dense.
- Deeply-commented single files elsewhere keep functions small; the tail of the distribution is fine.

### 3.4 Naming & vestigial domain vocabulary

The crate's public vocabulary is still the **backtesting/trading DSL it originated as**: `JittedStrategyFn`, `compile_strategy`, `JITContext.in_position/entry_price/unrealized_pnl_pct` (context.rs:562-565), `JITDataFrame`, simulation kernels with OHLCV column tests (core.rs:207-462), doc headers advertising "0.1-1µs per row … backtesting" (core.rs:6). None of this is language-runtime vocabulary; it actively misleads readers about what `compile_strategy` does today (it compiles the top-level program body). P2 rename debt.

Related vestige: `JITConfig` (context.rs:983-1000) carries `jit_threshold: usize` (default 100) — a per-config tier threshold that nothing reads on the live AOT path; it duplicates `Tier::BaselineJit.threshold()` in shape-vm (tier.rs:30-37), a third copy of the "100" constant, all dead.

Comment rot referencing deleted types as if live: `compiler/accessors.rs:~624-626` describes the A.1D.2 capture dispatch as going "through the `*mut ValueWord` cell bits stored in the slot" — `ValueWord` was deleted in Phase 2; the live cell carrier is not a ValueWord. (Distinct from the legitimate deletion-history docs in `ffi/object/conversion.rs:1-47`, which discuss ValueWord *as deleted* — that style is CLAUDE.md-conformant.)

### 3.5 Dead code in-territory

- Whole modules `#[allow(dead_code)]`: `ffi_symbols` (lib.rs:36), `foreign_bridge` partially (lib.rs:39), `mir_compiler` (lib.rs:47 — declared "staged WIP", though it is the LIVE path — the allow hides genuinely dead sub-items inside the live module), `optimizer` (lib.rs:54). 16 file-level/`item` `allow(dead_code)` markers total.
- Production-dead but maintained: `worker.rs` (651), `osr_compiler.rs` (971), `jit_cache.rs` (467), `numeric_compiler.rs` (105, legacy generic-opcode compiler kept for `compile_program` — itself only reachable from `#[ignore]`d tests per core.rs:137-171), `mixed_table::Pending` + `promote_to_native` (mixed_table.rs:21,123 — only self-tests).
- **The entire `src/optimizer/` planning subsystem is unconsumed.** Grep for `CallPathPlan`, `analyze_call_path`, `OptimizationPlan`, `build_plan` outside `src/optimizer/`: **zero hits**. The twelve phase-numbered passes (bounds, typed_mir, loop_lowering, numeric_arrays, vectorization, call_path, table_queryable, hof_inline, correctness, licm, escape_analysis, cross_function) form a self-contained pipeline whose output nothing in codegen reads; several are honestly headed "Intentional-future" (bounds.rs:3). The **live** optimization is elsewhere: `mir_compiler/bounds_elision.rs` (a separate, MIR-level analysis) is genuinely consulted at places.rs:1213 and :1437 (`self.bounds_elision.is_trusted(arr, iv)`).
- `tests/differential_fuzz.rs` (632 LOC): `#![cfg(any())]` — dead by declaration (line 17).
- Rough estimate: **~4,500-5,500 LOC (7-8%) of the crate is production-unreachable machinery**, most of it tier/OSR/cache infrastructure awaiting a wiring that has been "deprecated" at the only call sites that would use it.

---

## 4. Duplication & DRY violations

1. **Two `JitCodeCache` types.** `crates/shape-jit/src/jit_cache.rs:53` (entries + dependency reverse-index + `Tier2CacheKey`, 467 LOC) vs `crates/shape-vm/src/blob_cache_v2.rs:124-160` (bare `HashMap<FunctionHash, *const u8>`). Same name, same concept, different capability, **neither used in production** (no constructor call sites outside the defining files/tests). Divergence danger: low today (both dead), but the book documents the shape-jit one as the live cache — whichever gets wired first orphans the other.
2. **Two compatibility classifiers feeding one decision.** Bytecode-level `preflight_instructions` (compiler/accessors.rs) vs MIR-level `mir_compiler::preflight` (mod.rs:603). `compile_program_selective` ORs them (`bytecode_ok || mir_ok`, program.rs:722) — a function can pass on MIR grounds while its bytecode preflight fails, or vice versa. The comment block at program.rs:681-721 (A.1D.2/A.1E history) shows the gate lists have already drifted at least three times and require synchronized edits (`vm_only_opcode_reason` removals ↔ MIR side-tables). Divergence consequence: Phase-4 compile failures caught late (finalize deopt) instead of early — visible in the `Vec.map` finalize-fallback transcript in §2.3.
3. **`build_sub_program` manual 30+-field struct rebuild** (worker.rs:295-335) — re-lists every `BytecodeProgram` field, choosing per-field between clone/empty/default. Any new field forces an edit here (compile-error-guarded, good) but the clone-vs-zero decision is duplicated knowledge of what OSR compilation needs; wrong choice = silent mis-compilation of OSR loops (currently unreachable, so latent).
4. **VM error semantics re-implemented in JIT codegen.** Int-overflow checked arithmetic exists twice: VM `binop_int_checked` (`executor/arithmetic/mod.rs:150-152` per executor.rs comment) and JIT guarded-branch emission returning `JIT_SIGNAL_INT_OVERFLOW` (context.rs:140 + terminators/rvalues emission). Same for div-by-zero and index OOB. The executor maps signals back to matching messages (executor.rs:972-1032) but has already drifted in fidelity: JIT loses operand values and line info (§2.1 transcript). This is the structural cost of two executors — see §5.
5. **Kind-encoding tables in lockstep.** `ffi/stack_kind_code.rs` maps `NativeKind`↔u8 with `Ptr(HeapKind)` encoded as `128 + ordinal` ("`(code - 128) as HeapKind`", stack_kind_code.rs:44-49) — a fourth parallel table keyed on HeapKind ordinals (the verify-merge gate's "4-table HeapKind lockstep" concern applies in-territory). An ordinal insertion in shape-value silently re-means every byte >128 in JIT-stamped kind tracks; only the merge-gate grep protects this.
6. **Return-kind tables vs VM PHF method registry** (mir_compiler/types.rs, 4.8k lines of classification): duplicated knowledge of every builtin method's return type. Mitigated properly by `registry_cross_check` (types.rs:3605) which iterates the real `phf::Map<&str, MethodFnV2>` from shape-vm (Cargo.toml dev-dep note, lines 28-33) — this is the model the other duplications should follow.
7. **Function-name mangling scheme** `format!("{}_f{}_{}", name, idx, func.name.replace("::", "__"))` appears at program.rs:796, :840, :958 — three copies of the naming contract that `function_table[idx]`/`compiled_functions` lookups depend on; a drift in one produces lookup misses, not compile errors.

---

## 5. Split-brain analysis

### 5.1 The structural split-brain: two executors of one language

The JIT is a full second implementation of Shape semantics. The deopt-gate inventory (§2.3) is best read as a **fossil record of past VM≠JIT divergences** — every gate documents a real, observed behavioral split before it was walled off:

| Past divergence (from the gate's own text) | Where recorded |
|---|---|
| comptime side effects fired TWICE under jit (`comptime { print("SIDE") }`) | executor.rs:115-137 |
| trait method calls returned `None` instead of impl result | executor.rs:210-220 |
| `set::from_array([1,2,3])` — JIT exit 0 with `{"Integer": -1407…}` garbage vs VM clean error | executor.rs:332-349 |
| `print(IMPORTED_CONST)` — VM=2 / JIT=0 zero-init bits | executor.rs:370-386 |
| `serialize([1,2,3]).len()` — VM ec1 clean surface / JIT ec0 garbage | executor.rs:405-439 |
| `?` operator — VM=42 / JIT=Integer(137_900_062_693_984) (pointer bits as int) | executor.rs:464-504 |
| escaped-ref deref — VM=5 / JIT=\<stack-pointer\> | executor.rs:536-551 |
| `??` would leak the whole `Some(v)` wrapper | executor.rs:571-581 |
| user Drop bodies silently elided by JIT scope exit | executor.rs:612-638 |
| module-binding reads from fn bodies — VM=100 / JIT=0 | program.rs:642-660 |
| `Result<string,string>` match-destruct — deterministic SIGSEGV ec=139 | mir_compiler/mod.rs:~690-720 (EnumPayload preflight text) |
| schema-id collision on fallback recompile (`MakeFieldRef field_idx N out of bounds`) | executor.rs:281-299 |

The current strategy is honest containment, not convergence: each new divergence gets a gate, and the JIT-side root causes are batched to "v0.4 JIT-lowering followup" (executor.rs:436-439 et al.). Residual live drift found in this audit even inside the gated region: error-fidelity loss (line numbers, overflow operands — §2.1).

### 5.2 Value-representation split-brain

- VM: 8-byte typed slots + parallel `Vec<NativeKind>` (ADR-006 §2.7.7), no tagged words.
- JIT: NaN-boxed `u64` stack/locals (`value_ffi.rs` local tag layout: TAG_BASE 0xFFF8…, 3-bit tag @bits 50-48) + parallel `stack_kinds: [u8;512]` byte track + typed native slots where kinds are proven.
- The bridge: typed return tags at the host boundary (executor.rs:1047+), kind-byte pairing at trampoline boundaries (call_method/mod.rs:599+). CLAUDE.md's ADR-005 §4 claim "**VM and JIT share the slot ABI — no conversion at the boundary**" is **not true on the trampoline path**: `jit_call_method`'s VM dispatch boxes scalar results back into NaN-box f64/`TAG_BOOL_*` sentinels — the crate's own error text says so verbatim ("the `jit_call_method` VM trampoline boxes the scalar result via `box_number(.. as f64)` … NEITHER of which is the raw native scalar the proven destination slot expects", §2.3 string-method transcript).

### 5.3 Heap-header split-brain

`UnifiedValue<T>` = `{kind:u16 @0, flags:u8 @2, _reserved:u8 @3, refcount:AtomicU32 @4, data @8}` (jit_kinds.rs:85-96) vs shape-value `HeapHeader` = `{refcount:AtomicU32 @0, kind:u16 @4, flags:u8 @6, _pad:u8 @7}` (heap_header.rs:44-54). Same 8-byte envelope, **fields at swapped offsets**. `read_heap_kind(bits)` on a v2-raw (`HeapHeader`) allocation returns the low 16 bits of the refcount; a refcount read on a `UnifiedValue` reads kind+flags. Nothing at the type level distinguishes the two — both travel as `u64` with `NativeKind::Ptr(HeapKind)` stamps. The old "magic byte at offset 3" scheme from project memory is **gone** (grep for magic/MAGIC across shape-jit/shape-vm/shape-value: zero hits); discrimination is now 100% producer-discipline. Every FFI function taking `bits: u64` must know which allocator produced it; the 2026-05-17 W5 fixture bugs (allocator-pair mismatch SIGABRTs, CLAUDE.md Known Constraints) show this class is not hypothetical.

### 5.4 Doc-vs-code split-brains (in-territory)

1. `core.rs:1-92` module docs describe the deleted BytecodeToIR f64-only model, "Tier 2 (Not Yet Implemented …)" lists Call/arrays/closures as future — while the live MirToIR handles calls and typed arrays. The examples use `function`/`data[0].close`/ternary — not even current Shape syntax.
2. `executor.rs:68-71` doc comment: "Tier-up thresholds at T1@100 / T2@10k on hot functions are preserved by the underlying `compile_program_selective` pipeline" — **false**; that function contains no tier logic (program.rs:632-973), and the tier path is dead (§2.2).
3. `cli_args.rs:78-85` `--mode jit` help: "tiered: interpreter → baseline @ T1=100 calls → optimizing @ T2=10k calls" — same fiction, shipped in `--help` output.
4. `rvalues.rs:1843-1851` doc comment describes a live "inline NaN-box dispatch (Both-Number hot path …)" for closure-return arithmetic; the actual `compile_binop_dynamic_arith` (rvalues.rs:1954-1968) is a surface-and-stop `Err`. The comment documents a deleted (or never-landed) mechanism — dangerous because it invites re-implementation of the forbidden pattern it describes.
5. `ffi/gc.rs:9-10` "no tracing collector exists" vs the real gc-feature body below (§2.5).
6. Book split-brains catalogued in §8.

### 5.5 Config split-brain

`jit-trace` exists as a feature on BOTH shape-vm (Cargo.toml:61-65) and shape-jit (Cargo.toml:53-56) with a doc note that they're co-features toggled together via the CLI `--trace-jit` — a two-place toggle that can half-enable diagnostics if one side is missed. Worse: the default build compiles the flag OUT entirely while still advertising it (B16, §9).

### 5.6 Cross-crate JITContext layout contract

Within shape-jit, every hardcoded JITContext byte offset is compile-time verified against the real `#[repr(C)]` struct via `offset_of!` assertions (context.rs:145-175: TIMESTAMPS_PTR 24, COLUMN_PTRS 32, LOCALS 64, STACK 2112, …) — exemplary. But shape-vm's OSR entry (`executor/osr.rs:40-50`) hardcodes its own mirror of the layout (`CTX_U64_SIZE = 800`, `LOCALS_U64_OFFSET = 8`, comment-level byte map "stack_ptr: byte 6208, gc_safepoint_flag_ptr: byte 6328…") **in a crate that cannot name `JITContext`** (dependency direction: shape-jit → shape-vm, not vice versa), so no `offset_of!` can protect it. Any field insertion into JITContext silently invalidates shape-vm's OSR buffer arithmetic. Latent today (OSR dead), a booby trap the day OSR is wired. A shared `#[repr(C)]` context-layout crate (or moving the buffer builder into shape-jit) removes the hazard.

---

## 6. ADR & spec conformance

Marker density in-territory: 70 files carry `ADR-005`/`ADR-006` markers; 228 references to `§2.7.5` alone. Rule-by-rule:

| Rule | Verdict | Evidence |
|---|---|---|
| **ADR-005 §1 single discriminator** (HeapValue canonical; no 1:1 parallel sum types) | **PARTIAL** | JIT introduces `JitAlloc.kind: u16` with private ordinals ≥128 (`HK_JIT_FUNCTION`=128, `HK_JIT_TABLE_REF`=130, `HK_JIT_OBJECT`=131, jit_kinds.rs:30-34) — a second heap discriminator namespace deliberately outside `HeapKind`, plus the swapped-layout header (§5.3). Not a HeapKind-projecting sum type (the letter survives), but it is a parallel discrimination scheme with drift risk on the shared ordinal space (`128 + HeapKind ordinal` in stack_kind_code.rs:44-49) |
| **ADR-005 §4 / ADR-006 uniform slot ABI** ("VM and JIT share the slot ABI — no conversion at the boundary") | **NON-CONFORMANT** on trampoline paths | jit_call_method boxes scalars to NaN-box on return (own error text, §2.3); JIT stack is NaN-boxed u64, VM stack is typed slots — conversion happens at every trampoline crossing |
| **ADR-006 §2.7.5** (raw u64 + NativeKind stamped at compile time; "no runtime kind discrimination from the bits themselves") | **MOSTLY CONFORMANT, 2 live exceptions** | Conforms: typed RETURN_TAG protocol with surface-and-stop on NANBOXED (executor.rs:1070-1108); kind-byte pairing at FFI (call_method/mod.rs:604-670 refuses SENTINEL/mismatched kinds, no Bool-default); value_ffi.rs documents its sentinels as compile-time-referenced constants, not runtime decode (value_ffi.rs:10-20). Violations: **`emit_index_to_i64`** (places.rs:522-550) emits a runtime `bits >= TAG_BASE` test selecting int-payload vs f64 decode — runtime kind discrimination from bits, live on 4 array-index paths (places.rs:600,653,696,711); **`compile_binop_dynamic_cmp`** Eq/Ne as raw bitwise icmp on kind-untyped operands (rvalues.rs:1987-1995) |
| **ADR-006 §2.7.7 / Q9** (parallel kind track; no `Option<NativeKind>`/Unknown; no Bool-default; no `Vec<KindedSlot>` stack; no 16-byte slots) | **CONFORMANT** | `JITContext.stack_kinds: [u8;512]` lockstep track (context.rs:586-594); `SENTINEL=255` is an uninitialized-slot marker whose *read* is treated as a surface error, not a dispatchable Unknown (stack_kind_code.rs:20-27, call_method/mod.rs:645-659); compile-time-only `Option<NativeKind>` in `slot_kind_for_local` is the permitted inference-metadata layer (types.rs:26-40) |
| **ADR-006 §2.7.10 / Q11** (MethodFnV2 kinded handler ABI) | **PARTIAL** | The VM-side registry is consumed via trampoline with (bits,kind) pairs assembled to KindedSlot; but JIT-side ModuleFn dispatch is `todo!()`-class with silent TAG_NULL arms (executor.rs:435-439, ffi/control/mod.rs:~704-715) — the executor's own text calls the §2.7.10/Q11 JIT rebuild "v0.4 / planned" |
| **ADR-006 §2.7.11 / Q12** (value-call ABI; no kind-from-raw-bits; no Bool-default captures) | **PARTIAL** | jit_call_value classifies callees via the kind companion and pairs args as KindedSlots for the trampoline (ffi/control/mod.rs:~720-730); nonconformances are the silent TAG_NULL SURFACE arms (should be hard errors per surface-and-stop) and UInt64-class NaN-box callee shapes (TAG_FUNCTION inline) persisting as a JIT-internal callee encoding |
| **§2.7.14 surface-and-stop discipline** | **CONFORMANT & exemplary at compile-time; LEAKY at runtime** | 11+ compile-stage gates (§2.3) conform. Runtime FFI SURFACE hits return TAG_NULL with `tracing::debug!` only — the discipline's own definition ("surface-and-stop with NotImplemented(SURFACE)") is not met at runtime dispatch arms |
| **Forbidden Patterns (CLAUDE.md)** — ValueWord, generic opcodes, `is_tagged()` handlers, Convert opcodes, SlotKind::Dynamic, exec_*_dynamic_fallback, rename families | **NO LIVE VIOLATION FOUND** | `bash scripts/check-no-dynamic.sh` at working tree: exits 0, no regressions vs baseline. `compile_binop_dynamic_arith/cmp` carry the word "dynamic" but are surface-and-stop stubs (arith) / bitwise-eq-only (cmp) — not restored dispatch. The NaN-box layout in value_ffi.rs is explicitly argued as JIT-internal sentinel encoding (value_ffi.rs:10-20); I judge the *letter* respected, but places.rs:522-550 (runtime tag test) is the closest thing to a live forbidden shape in-territory and deserves an explicit ruling |
| **ADR-006 §2.7.9** (`HeapKind::FilterExpr` as pure-discriminator label; every dispatch table must carry the arm) | **CONFORMANT** | FilterExpr arms present in the JIT's kind-classification tables: v2_call_abi.rs:150, v2_field.rs:72,114, v2_array.rs:72; byte-track decode arm `18 => HeapKind::FilterExpr` (stack_kind_code.rs:221); `HK_FILTER_EXPR` mirror constant (value_ffi.rs:185) |
| **ADR-006 §2.7.8/Q10 no-Bool-default in refcount/ownership decisions** | **CONFORMANT** | `refcount_disposition` (ownership.rs:455-461) returns surface-and-stop `Err` when a slot's NativeKind is unresolvable — "no Bool-default fall-through" stated and implemented; shared by `Rvalue::Clone` via `refcount_disposition_for_place` (ownership.rs:606-610) |
| **runtime-v2-spec.md** ("typed, zero-tag native values … opcodes encode type") | **CONFORMANT for proven kinds; NaN-box persists as the unproven-kind carrier** | v2_int/v2_array/v2_field lowerings use raw native Cranelift types; unproven slots ride NaN-boxed u64 — the spec's "no dynamic fallback" is enforced by compile-Err, not by absence of the tagged encoding |

**Requested explicit ruling for the owner**: `emit_index_to_i64` (places.rs:522) and bitwise dynamic Eq (rvalues.rs:1987) are small, bounded, and arguably pragmatic — but they are the seed shape of the W-series pattern ("just one decode at the boundary"). Either bless them with a named ADR carve-out (like TypedFieldValue::String) or delete them by requiring proven index/operand kinds.

---

## 7. Test coverage in-territory

### 7.1 Counts

- **849 `#[test]` functions** in `crates/shape-jit` (src + tests dirs).
- **26 `#[ignore]` attributes**; reason breakdown (grep):
  - 16 × `"v2: tests deleted BytecodeToIR path; covered by mir_compiler::integration_tests"` (core.rs width-aware/inline-array/reference tests)
  - 2 × Tier-1 whole-function deprecated (worker.rs:348, :433)
  - 1 × `"W11/§2.7.4: deleted JitArray/jit_array_info API"` (core.rs:692)
  - 1 × simulation `jit_call_value` extern-C `todo!()` SIGABRT (ffi_symbols/simulation/mod.rs:118)
  - 2 × same-constraint kinded-ABI SURFACE waits (ffi/control, ffi/async_ops)
  - remainder: archival `cfg(any())`-style notes
- 8 dedicated regression-test modules inside mir_compiler (closure_dispatch 933 LOC, short_circuit 233, ref_param 209, groupby_surface 193, jit_array_param 180, array_builder 178, fuzzy_comparison 122, field_ref) — these encode past miscompilations as unit tests; assertion quality is high (exact expected values, kind assertions, refcount balance checks).
- `registry_cross_check` (mir_compiler/types.rs:3605) cross-validates the JIT's builtin return-kind tables against shape-vm's real PHF method registry — the single best anti-drift test in the crate.

Per-file distribution of the test mass (files with >15 `#[test]`s):

| Tests | File | Runs by default? |
|---|---|---|
| 151 | mir_compiler/integration_tests.rs | **No** — deep-tests-gated |
| 64 | mir_compiler/v2_int.rs | Yes |
| 49 | mir_compiler/types.rs | Yes (incl. registry_cross_check) |
| 44 | ffi/v2/mod.rs | Yes |
| 35 | ffi/v2_math.rs | Yes |
| 34 | mir_compiler/closure_dispatch_regression_tests.rs | Yes |
| 33 | ffi/conversion.rs | Yes |
| 26 | mir_compiler/statements.rs; ffi/v2/collection_arc.rs | Yes |
| 25 | ffi/object/closure.rs; core.rs (mostly `#[ignore]`) | Yes/ignored |
| 24 | compiler/accessors.rs | Yes |
| 17 | mir_compiler/v2_array_tests.rs | **No** — deep-tests-gated |
| 16 | mir_compiler/v2_field.rs | Yes |

The default `cargo test -p shape-jit --lib` binary contains **517 tests** (verified via the spot-check run's "511 filtered out" + 6). The remaining ~330 of the 849 total live in deep-tests-gated modules (integration_tests 151, v2_array_tests 17, a1d2/a1e/c2 test modules) and the disabled tests/ dir — i.e. **~39% of the crate's test mass is not even compiled into a default invocation**, and of the 517 compiled, the 26 `#[ignore]`d are skipped at runtime.

### 7.2 Do the ignore reasons still hold?

**Yes, with one caveat.** The 16 BytecodeToIR ignores are permanently valid: the direct `compile_program` path now routes to the legacy `compile_numeric_program` which cannot handle typed opcodes (documented at core.rs:137-171); hand-built `BytecodeProgram`s without `top_level_mir` cannot take the live path. The caveat: their stated replacement — "covered by `mir_compiler::integration_tests`" — is **deep-tests-gated** (`#[cfg(all(test, feature = "deep-tests"))]`, mir_compiler/mod.rs:43-47, compiler/mod.rs:19-26), so a default `cargo test -p shape-jit` run exercises **neither** the old tests nor their claimed replacement. The equivalent-coverage claim holds only under `just test-deep`.

The deep-tests gating rationale itself (each heavy test JIT-compiles ~118 stdlib functions; SIGILL race at default parallelism — Cargo.toml:33-40) remains true *because the JIT cache is unwired* (§2.4): fixing the cache would dissolve the gating reason.

### 7.3 The big hole: no active differential testing

`tests/differential_fuzz.rs` — the only in-crate VM↔JIT parity fuzz harness — is disabled wholesale (`#![cfg(any())]`, line 17) because it consumed deleted `ValueWordExt`/`to_typed_scalar` APIs; its rebuild is "tracked as part of the §2.7.4 Phase 2c FFI rebuild" (header comment). Given §5.1's fossil record of a dozen silent-wrong-output divergences, the absence of an automated differential gate is the most consequential test gap in this vertical. The de-facto differential gate is the book truth-gate (out-of-crate) plus hand-run smoke pairs.

### 7.4 What the in-crate tests do NOT cover

- The tier/OSR/deopt structures are tested **as libraries** (tier.rs: 25 tests; osr.rs: 5+; worker.rs OSR path: 2 live) — green tests over production-dead wiring create false confidence; nothing fails when the production wiring is absent.
- No test asserts the deopt-gate inventory (§2.3) matches the set of actually-unsound lowerings — a gate could be removed while its root cause persists and no in-crate test would notice.
- No negative test that `jit_call_value`'s TAG_NULL SURFACE arms are unreachable from gated programs.
- Error-message parity (line numbers, overflow operands) is untested — the drift in §2.1 is invisible to CI.

### 7.5 Spot-check run

`direnv exec … cargo test -p shape-jit --lib registry_cross_check` — result recorded in §13 addendum (run kicked off during audit; see note there).

---

## 8. Book/docs vs reality for this vertical

Source: `/home/dev/dev/shape-lang/shape-web/book/book-site/src/content/docs/advanced/jit-compilation.mdx` (482 lines), CLAUDE.md, README-level claims.

| Claim | Reality | Verdict |
|---|---|---|
| "Tier 2 compilation is **shipped**: `compile_optimizing_function` (crates/shape-jit/src/worker.rs) runs feedback-guided optimizing compilation when a function crosses the 10,000-call threshold" (mdx :262-268) | `compile_optimizing_function` is in program.rs (not worker.rs) and is `Err("Optimizing JIT is deprecated")` (program.rs:607-622); the 10k threshold can never fire (tier_manager None, §2.2) | **FALSE** |
| "Promotion thresholds are checked at function entry … compilation request is submitted" (mdx Tier table + Background Compilation diagram) | Machinery exists; never constructed in production. The described `try_recv()`-at-safe-points loop (`poll_tier_completions`) runs every 1024 instructions as a no-op | **FALSE at runtime** (accurate as a description of dead code) |
| "The VM maintains a dispatch table … `pub type JitFnPtr` … `vm.register_jit_function(function_id, ptr)`" (mdx JIT Dispatch Table section) | **None of these identifiers exist anywhere in shape-vm** (grep: zero hits for `register_jit_function`, `jit_dispatch_table`, `JitFnPtr`) | **FABRICATED API** |
| `FunctionEntry::Pending` "awaiting background compilation … will promote" (mdx MixedFunctionTable) | Variant exists (mixed_table.rs:21) but is never produced by any compile path; only its own unit tests construct it | **MISLEADING** |
| "JIT output is cached by blob content hash … compiled exactly once and reused everywhere" (mdx Content-Addressed JIT Cache) | `JitCodeCache` (both copies) has zero production call sites; every run recompiles everything including the stdlib prelude | **FALSE** |
| "`--mode jit` semantics … On JIT-compile failure the executor falls through to the bytecode interpreter … `[jit-fallback]` diagnostic" (mdx :225-250) | Accurate and verified empirically (§2.3 transcripts) | **TRUE** |
| "Tier-up promotion is preserved on hot functions per the T1@100 / T2@10k thresholds" (mdx :246) | False — same dead-tier chain | **FALSE** |
| The shipped-vs-planned note honestly flags IC devirtualization as NOT shipped, "no `DevirtAnalysis` type, no `CallGraph` type" (mdx :258-268) | Correct — and proves the book *can* be honest; the adjacent "Tier 2 is shipped" sentence shows the calibration failed anyway | **MIXED** |
| "Inlining is governed by a per-program `CallPathPlan` … `analyze_call_path` walks every `Call` instruction … the JIT consults the same `CallPathPlan` regardless of tier" (mdx Inlining Policy) | `CallPathPlan`/`analyze_call_path` are referenced by **nothing outside `src/optimizer/`** (grep: zero consumer hits); the plan is computed by an unconsumed subsystem (§3.5). The heuristics described (≤4 args, depth 4→6) exist in the dead pass only | **FALSE as runtime behavior** |
| "Use `--trace-jit=shape_jit=debug` to promote per-function diagnostics" (book + shipped `--help` text) | Default binary: `error: unexpected argument '--trace-jit' found` — the flag is compiled out with the `jit-trace` feature (B16) | **FALSE for the shipped binary** |
| CLAUDE.md: "JIT (shape-jit): Cranelift codegen via MirToIR, tiered (Tier 1 baseline @ 100 calls, Tier 2 optimizing @ 10k), OSR for hot loops, deoptimization back to interpreter" | MirToIR: true. Tiered/OSR/deopt: dead (§2.2) | **HALF-TRUE** |
| CLAUDE.md Known Constraints: shape-jit heavy tests deep-gated; ~23 JIT `#[ignore]`s listed by name | Verified: 26 ignores, names match (`test_jit_width_aware_*`, `test_jit_inline_array_*`, `test_jit_*_kernel_compilation`, `test_backend_compiles_whole_function`) | **TRUE** |
| `--help` text (cli_args.rs:78-85): "tiered: interpreter → baseline @ T1=100 → optimizing @ T2=10k" | Dead path; the help text describes fiction to every user | **FALSE** |

Net: the fall-through semantics section of the book is accurate and empirically verified; the tiering/caching/dispatch-table half describes a system that does not run, including one section of invented API. Given the project's book-gate policy ("every implemented feature must be in the book + covered by a gate-runnable example"), the inverse rule is being violated: the book documents unimplemented features as shipped.

---

## 9. Bugs & correctness risks found

Severity scale: P0 = unsound/wrong-results/security; P1 = broken feature; P2 = paper cut.

### 9.0 P0 — live silent wrong output found during this audit

**B0 (P0): Duration literals produce silent wrong output under the default mode — no gate, no fallback, exit 0.**

Repro (working-tree binary, default `--mode jit`):

```
$ cat probe_dur1.shape
let a = 1.5d
print(a)

$ shape run probe_dur1.shape            # default (jit)
0
$ shape run --mode vm probe_dur1.shape
PT129600S                               # 1.5 days = 129,600 s — correct
$ echo $?                               # both exit 0
0
```

Arithmetic compounds it: `print(1.5d + 2.5d)` → JIT `0`, VM `PT345600S`. There is **no `[jit-fallback]` line** — the program JIT-compiles "successfully" and prints garbage. Evidence for root-cause locality: `grep -rn Duration src/mir_compiler/*.rs` → **zero hits** — MirToIR has no Duration lowering at all (Duration *methods* exist only in the FFI at `ffi/call_method/duration.rs` on the JIT-private `JITDuration` unified-heap shape), yet nothing in the gate stack (bytecode preflight, MIR preflight, executor gates) rejects a duration-literal program. This is precisely the silent-wrong-output class the eleven gates were built to contain — an uncovered carrier slipped through the gate inventory. Immediate cheap fix: gate programs containing Duration constants/opcodes; root fix: lower the Duration carrier (or route through the trampoline with a proven kind).

This finding also demonstrates the systemic risk of the per-symptom gate strategy (§11.6): the gate list enumerates known divergences; anything not yet enumerated ships as silent wrongness.

### 9.1 P1

**B1 (P1, feature-broken-as-advertised): Tiered JIT / OSR / deopt / feedback speculation do not run.**
Full chain in §2.2. Not a latent bug — a shipped feature (CLI help, book, CLAUDE.md) that is structurally unreachable. Repro: no program can ever tier up; `poll_tier_completions` no-ops forever (`init.rs:82` None + no setter).

**B2 (P1, silent-wrong-output class): runtime SURFACE arms return `TAG_NULL` instead of erroring.**
`ffi/control/mod.rs` (jit_call_value: callee-bits-mismatch arm and unrecognized-callee-kind arm, ~:690-720) and `ffi/call_method/mod.rs:604-670` (SENTINEL kind-byte, non-String method-name kind) all `return TAG_NULL` with only `tracing::debug!` — invisible in a default run (no subscriber). Every arm is a producer-contract violation, i.e. a compiler bug, being converted into a silent null at user level. The W17-marshal deopt gate (executor.rs:440) exists because exactly this arm produced "VM=ec1 SURFACE / JIT=ec0 garbage". Gates guard the *known* producers; the arms remain for unknown ones. Per the project's own surface-and-stop rule these should hard-signal (e.g. `SIGNAL_TRAMPOLINE_ERROR` with a stored message — the mechanism already exists, executor.rs:1015-1032).

**B3 (P1, diagnostics divergence): JIT-path runtime errors lose source location and operand detail.**
Three repro transcripts: div-by-zero — VM: `Division by zero (line 1)`, JIT: `Division by zero`; overflow — VM: `integer multiplication overflow: result of 9223372036854775807 and 3 exceeds…(line 1)`, JIT: generic `integer overflow: result exceeds…`; out-of-bounds — VM: `Index 7 out of bounds (length 3) (line 3)`, JIT: `Index out of bounds`. Root cause: signals carry no operands/location and executor maps them with `location: None` (executor.rs:976-1013). For a language whose selling point is diagnostics (LSDS), the default mode degrading error quality vs `--mode vm` is a real user-facing regression — and it applies precisely on the programs healthy enough to JIT.

**B4 (P1, latent UB): dual heap-header layouts with convention-only discrimination.**
§5.3. `read_heap_kind` on a `HeapHeader`-shaped allocation reads refcount bits as kind; refcount ops on a `UnifiedValue` corrupt kind/flags. No runtime or debug assertion distinguishes the shapes; the W5 close (CLAUDE.md Known Constraints) documents real SIGABRTs from precisely an allocator-pair mismatch. Any future FFI function accepting `Ptr(HeapKind::X)` bits from both producers is one refactor away from UB.

**B5 (P1 if reachable, else P2): `EnumPayload` UB documented as live-at-HEAD in preflight text.**
mir_compiler/mod.rs (~:690-720) states the `jit_arc_*_payload` cast is UB when the producer threads an `Arc<HeapValue>` instead of strict `Arc<ResultData>`: "Empirically observed at HEAD … deterministic ec=139 SIGSEGV". The preflight gate blocks the known MIR shape, but the text itself says the root cause (receiver-recovery extension) is unfixed — the gate is the only thing between users and a segfault, and gates are opt-out on every new MIR producer.

### 9.2 P2

**B6: `emit_index_to_i64` runtime tag test + NaN aliasing edge.** places.rs:522-550: bits ≥ 0xFFF8_0000_0000_0000 are treated as tagged ints. A genuine f64 index whose bit pattern is a negative quiet NaN aliases into the int path (payload extraction of garbage). Reachability requires a NaN index — already a bug in user code — but the failure mode is a wrong index rather than an error. Also the ADR tension (§6).

**B7: kind-untyped `==`/`!=` compiled as bitwise compare.** rvalues.rs:1987-1995. `-0.0 == 0.0` → false (IEEE says true), `NaN == NaN` → true (IEEE says false) for any operands that reach the dynamic path (closure-returned values per the surrounding comments). VM comparison handlers use typed semantics → potential silent VM≠JIT divergence within the *supported* surface. Mitigating: most closure-heavy programs currently deopt wholesale (§2.3), shrinking reachability.

**B8: stale/step-on-rake doc comments.** (a) rvalues.rs:1843-1851 describes a live NaN-box dual-path dispatch that the code refuses — an invitation to re-implement a forbidden pattern; (b) executor.rs:68-71 claims tier-up preservation; (c) core.rs:1-92 documents the deleted compiler with non-Shape syntax examples; (d) ffi/gc.rs:9-10 claims no collector exists.

**B9: `[jit-fallback]` diagnostics are walls of internal jargon.** Repro: the `?`-operator program prints a ~1,400-character stderr paragraph citing supervisor ratification dates, audit file paths, and ADR sections (§2.3 transcript) — on a perfectly ordinary user program. The information belongs in `--trace-jit`; the stderr line should be one sentence.

**B10: `scalar_move_lift_exposed_jit_surface` string-matches method names.** executor.rs:23-33: any `CallMethod` whose method string is `clone` or `diff` deopts the whole program — including user-defined methods that merely share the name. Over-deopt (safe direction), but surprising and undocumented.

**B11: extern "C" `todo!()` bodies = process aborts.** matrix.rs:72-96, vector/mod.rs:30, object_ops.rs:114/136, **plus the six HOF trampolines** `jit_control_fold/reduce/map/filter/foreach/find` (control/mod.rs:843-900+, §2.9) — currently unreferenced or shielded by earlier deopts, but any future emission reaching them SIGABRTs the process instead of erroring (unwind across extern "C"). The `#[ignore]` at simulation/mod.rs:118 proves this already bites in tests. Fixing closure compilation (P1.5(b)/§12) without first rebuilding the HOF stubs would arm them.

**B12: justfile `verify-phase-5` claims the no-dynamic sentinel test "is not yet wired up" (justfile:193-197) while `crates/shape-vm/src/executor/tests/no_dynamic.rs` exists** — the recipe never runs it (cross-territory nit; noted because CLAUDE.md cites the sentinel as mechanical enforcement for this vertical's forbidden patterns).

**B13 (P2, feature narrowness with soundness pedigree): typed-struct field reads don't produce proven kinds for arithmetic.**
Repro (§2.6): `type Point { x: number, y: number }; let p = Point{…}; print(p.x + p.y)` deopts with "kind-untyped arith Add … producing-MIR kind-tracker gap" — despite both fields being *declared* `number`. The declared field type exists in the schema; the kind tracker fails to thread it through the field-read place into the binop operand. This single gap likely accounts for a large share of real-program deopts (any struct-field arithmetic). Fix belongs in the producing-MIR kind tracker per the error's own instruction.

**B14 (P1 coverage, correctly contained): the entire `as`-cast opcode family is unlowered, and its old FFI body is a live silent-pass-through stub.**
`vm_only_opcode_reason` gates all 12 `ConvertTo*`/`TryConvertTo*` opcodes (accessors.rs:692-704); `dispatch_opcode` (ffi/generic_builtin.rs:169-181) still returns operand bits unchanged — the documented pre-gate behavior was `x as int` yielding `true`-bits or raw pointers printed as garbage. Correct today by gating; the stub should be converted to a hard signal so a future preflight regression cannot silently resurrect the divergence.

**B15 (P2): trinity payload binders (`Some(v)`/`Ok(v)`/`Err(e)`) deopt whenever the scrutinee crosses a user-fn return** (§2.6 probe: `match maybe(true) { Some(v) => … }` → EnumPayload §2.7.17 receiver-recovery gap). Since idiomatic Shape returns `Option`/`Result` from nearly every fallible function, this gate alone interpreter-routes most idiomatic error-handling code — compounding with the `?` gate (B-class: same root, the return-kind boundary).

**B16 (P2, shipped-help lies): `--trace-jit` is advertised by the shipped binary's own `--help` yet rejected by its parser.**
Repro: `shape run --help` prints "Use `--trace-jit=shape_jit=debug` to promote per-function diagnostics" (the `--mode jit` value docs); `shape run --trace-jit=shape_jit=debug file.shape` → `error: unexpected argument '--trace-jit' found`. The flag is compiled out with the off-by-default `jit-trace` feature (shape-jit Cargo.toml:53-56) while the help prose is unconditional. Every fallback diagnostic (§2.3) also points users at this nonexistent flag. Either compile the flag in unconditionally (runtime no-op without the feature) or feature-gate the help text.

**B17 (P2, book): the Inlining Policy section documents an unconsumed analysis as the JIT's live policy** — see §3.5/§8; the runtime consults no `CallPathPlan`.

**B18 (P1 latent under the gc-on default): the JIT half of the GC stop-the-world rendezvous is unemitted — JIT loops never poll the safepoint.**
Evidence chain (each step independently verified at working tree):

1. The hook exists and is real: `jit_gc_safepoint(ctx)` (ffi/gc.rs:30-52) loads `ctx.gc_safepoint_flag_ptr` and, gc-feature-on, parks on `shape_value::gc_coordinator::jit_safepoint_park()` when a stop is in progress. Its doc comment states: "This function is called at every loop back-edge in JIT-compiled code" and calls itself "the JIT back-edge half of the cross-worker stop-the-world rendezvous" (GC Phase 3b, landed in commit 0c792bbf).
2. The flag is wired: `JITContext::default()` points `gc_safepoint_flag_ptr` at the coordinator's `stop_requested` byte under `feature = "gc"` (context.rs:713-720), and the coordinator side documents the park entry as "called from `jit_gc_safepoint`" (shape-value/src/gc_coordinator.rs:219).
3. **Nothing emits the call.** `grep -rn "safepoint" crates/shape-jit/src` outside ffi/gc.rs hits only: context.rs field/comments, and `loop_analysis.rs` — which computes a per-loop `needs_gc_safepoint` classification (loop_analysis.rs:38,395-484) that **no other file consumes** (`grep needs_gc_safepoint` excluding loop_analysis.rs → zero hits). There is no `gc_safepoint` field in `FFIFuncRefs` (ffi_refs.rs — only `write_barrier` at :252), no registration in any `ffi_symbols/*` table, and no declaration in `compiler/ffi_builder.rs` (only `write_barrier: r!("jit_write_barrier")` at :223). The symbol is not even *nameable* from Cranelift IR in the current builder, so no future emission can work without also touching the three-file FFI ceremony (§1.6).
4. Contrast with the write barrier, whose emission IS real: `mir_compiler/places.rs:823` (`.call(self.ffi.write_barrier, …)`) plus the FFI-internal barrier calls (data.rs:476, typed_object/field_access.rs:100, object/object_ops.rs:98). So GC Phase 2 (barriers) landed both halves; Phase 3b (rendezvous) landed only the callee half.

Consequence analysis: with the shipped default (`gc` on in both tiers, `--mode jit` default), a thread executing a JIT-compiled hot loop cannot acknowledge a stop-the-world request until the whole jitted `main` returns. Today this is masked because `--mode jit` execution is effectively single-threaded run-to-completion and collections trigger from interpreter safepoints/teardown on the same thread. The moment JIT execution coexists with a concurrent collection trigger (async tasks + JIT, or the wired-tier future where jitted functions run inside the interpreter loop), the rendezvous stalls for the duration of the loop — unbounded pause or deadlock depending on coordinator semantics. It is also a live doc-vs-code split: ffi/gc.rs's own header asserts emission that does not exist, and the Phase-3b readiness reports (GO-WITH-CAVEATS, commits 5be24572/c5be82cd) validated "RSS bounded end-to-end" on paths that never enter a JIT loop mid-collection. Cheap first step: register the symbol + emit the poll at MirToIR loop back-edges gated on the (already-computed) `needs_gc_safepoint` classification; the analysis pass finally gets its consumer.

**B19 (P2, ADR-tension inventory): live runtime tag/heap-kind probes in reachable FFI handlers.**
Beyond the two codegen-side discriminations already flagged for ruling (§6: places.rs:522, rvalues.rs:1987), the *handler* side retains live bit-probing on legacy paths: `heap_kind(obj_bits)` dispatch in `jit_set_prop`/`jit_get_prop` (ffi/object/object_ops.rs:83-122, property_access.rs:48-118), `is_heap_kind(callee_bits, HK_CLOSURE)` as the documented "zero-capture dual-carrier check" in `jit_call_value` (ffi/control/mod.rs:683), `is_heap_kind(..., HK_STRING/HK_COLUMN_REF)` receiver/arg classification in `ffi/call_method/object.rs:16-46`, and `is_number(value_bits)` guards across all of `ffi/math.rs` (:25-169). Each carries an ADR-006 §2.7.5 rationalization comment ("reads a field from a heap-resident struct, not tag-bit dispatch" / "JIT-internal sentinel encoding"). The *letter* of the Forbidden Patterns list survives (these are not the deleted `shape_value::tag_bits` family), but functionally these are runtime type tests on raw bits inside dispatch handlers — the exact mechanism ADR-006 §2.7.10 forbids for the VM tier ("no `is_heap()` probe"). Most sit on paths that current gates make unreachable (object literals deopt; column refs are simulation legacy), but `math.rs` and the `jit_call_value` dual-carrier check are reachable today. Same disposition as §6's ruling request: bless by name with a bounded carve-out, or migrate the remaining probes to kind-track sourcing.

---

## 10. What is done well

1. **The surface-and-stop discipline is real and consistently applied at compile stage.** Eleven+ gates each: (a) name the observed divergence with concrete values, (b) cite the audit doc and ADR clause, (c) route to a *correct* execution instead of a wrong one. Compare with the usual industry failure mode (ship the wrong-answer JIT path); Shape chose correctness over coverage everywhere it noticed a problem. Empirically: every deopt I triggered produced VM-identical output.
2. **The double-execution fix is textbook.** WF-1A (program.rs:863-940): failed Phase-4 functions are left undefined so finalize fails at COMPILE stage — converting a "partially-executed native frame then re-run everything" side-effect-duplication bug into a clean pre-execution deopt, with the panic-hook suppression narrowly scoped and the reasoning written down. Likewise the nested-result contract separating compile-failure from program-error (executor.rs:305-321).
3. **Schema-id collision avoidance on fallback** (executor.rs:281-299): re-using the already-built inspection bytecode via `execute_compiled` instead of recompiling — eliminates a whole differential class (`MakeFieldRef field_idx N out of bounds`) and is faster.
4. **`registry_cross_check`** (types.rs:3605): cross-validating JIT return-kind tables against the VM's live PHF registry (verified passing: 6/6 in 0.06s at working tree). The correct pattern for keeping parallel tables honest — should be replicated for the other parallel structures (§4.5).
5. **Kind-track hygiene at FFI boundaries.** `jit_call_method` refuses SENTINEL/mismatched kind bytes rather than Bool-defaulting (call_method/mod.rs:645-670) — conformant with the hardest-won ADR rule in the project's history, in the place it would be easiest to cheat.
6. **VM-parity error signals.** Carving div-by-zero/index-OOB/int-overflow out of the negative signal space and mapping them to the interpreter's diagnostics (context.rs:90-140, executor.rs:962-1032) is the right architecture for error parity — B3's fidelity loss is an implementation gap, not a design gap.
7. **Panic containment + trampoline lifetime guards** (executor.rs:698, :841-847; program.rs:914-919): a JIT that cannot take down the host process on compile bugs, and a thread-local VM pointer that cannot dangle across unwinds.
8. **Observability counters built for refcount audits**: the arc retain/release/free + string-alloc leak counters behind zero-cost tracing gates (executor.rs:871-951) — designed specifically to catch the W-series leak class, with cumulative and per-run deltas.
9. **Honest gating of heavy tests** (deep-tests) with a written root cause (stdlib JIT-compile cost, SIGILL race) rather than deleted or flaky tests.
10. **Real measured wins where the JIT applies**: 18.6x on an arithmetic loop with exact-match output (§2.1) — the core numeric pipeline (v2_int/v2_array typed lowerings, bounds elision) demonstrably works.
11. **The live bounds-elision pass carries a written soundness argument.** `mir_compiler/bounds_elision.rs:1-45` states the five-condition induction proof (`iv` starts at 0, non-negative step, `iv < bnd` header test, `bnd` captured from `arr.length`, no reassignment of `arr` or `bnd` between capture and access) before eliding any check, and the conservative reassignment clause is enforced, not assumed. This is how unsafe-adjacent optimization should be documented — and it is actually consumed (places.rs:1213,1437), unlike the optimizer/ subsystem.
12. **Compile-time layout self-verification** for all in-crate JITContext offsets via `offset_of!` const asserts (context.rs:145-175) — the cross-crate mirror is the residual gap (§5.6), but inside the crate a field reorder cannot silently miscompile.

---

## 11. What is done poorly / tech debt

1. **Aspirational-tiering documentation debt, everywhere.** Four independent surfaces (book, CLAUDE.md, CLI help, in-code doc comments) describe a tiered runtime that is dead. This is the single most corrosive item: it poisons every downstream summary, and it hid the deprecation of `compile_single_function`/`compile_optimizing_function` (which turned the entire tier/worker/OSR/deopt stack into dead mass) from every doc surface.
2. **~5k LOC of dead machinery kept warm.** tier backend, OSR compiler, two caches, `Pending`, deopt plumbing — all maintained, tested, and unreachable. Either wire it (the MirToIR-era design decision is unmade: what does "Tier 1 whole-function" even mean when compile is per-program AOT?) or delete it to the git history like ValueWord was.
3. **Vestigial trading-DSL identity.** `JittedStrategyFn`, `compile_strategy`, `in_position/entry_price` fields in the core ABI struct, OHLCV test fixtures, "backtesting" module docs (§3.4) — the crate reads as a repurposed quant kernel, which it is; the vocabulary now actively obscures the language-runtime architecture.
4. **Monolith functions.** One ~2,140-line `compile_terminator` (terminators.rs:95-2239) and a ~700-line `jit_call_method` — the two most semantics-dense code paths in the crate are also its least decomposed.
5. **Stringly-typed errors.** `Result<_, String>` everywhere with essay-length messages; no error enum for gates vs codegen bugs vs producer-contract violations; `location: None` on every wrap (→ B3).
6. **The deopt-gate mechanism doesn't scale.** Gates are per-symptom, ordered, whole-program, and maintained by hand in two files (executor.rs + program.rs) plus MIR preflight. Each new language feature needs a gate decision; forgetting one ships silent wrongness (the fossil record shows ~12 already found). A single positive-list ("these MIR shapes are proven") would invert the failure mode from unsafe-by-omission to slow-by-omission.
7. **Per-run compilation waste.** No JIT cache in production + trampoline VM built from a full `bytecode.clone()` per execution (executor.rs:828-836) + stdlib prelude (~118+ fns, in this audit's array-probe transcript index f196 implies ≥196 functions) re-JIT'd on every `shape run`. This is also the root cause of the deep-tests CI gating (§7.2).
8. **`#[allow(dead_code)]` at module scope on the LIVE module** (`mir_compiler`, lib.rs:47-48) — suppresses the compiler's ability to spot genuinely-dead items inside the most important 27k lines of the crate.
9. **SAFETY-comment coverage ~20%** for 647 unsafe blocks (§3.2), in a crate whose entire FFI surface is unsafe-by-construction.
10. **Diagnostic ergonomics** (B9): internal governance prose leaking to user stderr.

---

## 12. Prioritized recommendations

### P0 — do first (days)

0. **Close the Duration hole (B0).** Add Duration constants/opcodes to the gate stack today (one preflight arm — hours), then decide lower-vs-trampoline as the root fix. Sweep for sibling un-gated carriers the same way this audit found it: literal-type × operation probe matrix under both modes diffing stdout (the §13 method, automatable in an afternoon).
1. **Make the docs stop lying about tiering.** Edit book `jit-compilation.mdx` (delete/mark-planned: Tier table as runtime behavior, Background Compilation, JIT Dispatch Table [fabricated API], JIT cache, `Pending`), fix `cli_args.rs:78-85` help text, fix `executor.rs:68-71` + `core.rs:1-92` doc comments, and correct CLAUDE.md's pipeline line. Effort: ~1 day. (This is the cheapest P0-risk elimination available in this vertical.)
2. **Decide the tier stack's fate explicitly.** Either (a) delete `worker.rs`, `osr_compiler.rs`, both `JitCodeCache`s, `Pending`, tier plumbing (leaving `tier.rs` in shape-vm for the record or deleting it too), or (b) write the ADR for MirToIR-era tiering and wire `TierManager` construction. Option (a) is ~2 days and removes ~5k LOC of false-confidence surface; (b) is weeks. Do not leave it ambient.

### P1 — correctness hardening (1-2 weeks)

3. **Convert runtime SURFACE `TAG_NULL` arms into hard signals** (B2): reuse `SIGNAL_TRAMPOLINE_ERROR` + `JIT_RUNTIME_ERROR` thread-local so producer-contract violations become clean runtime errors matching surface-and-stop. Effort: 1-2 days + regression tests.
4. **Restore differential testing** (§7.3): rebuild `differential_fuzz.rs` on the kinded ABI (the blockers named in its header are all resolved types now), and add an error-parity check (message class + location presence) to catch B3-style drift. Effort: ~1 week.
5. **Fix diagnostics parity** (B3): thread a location table (bytecode ip → span already exists in debug_info) into the signal-return path, and extend the overflow signal with an operand-carrying side channel. Effort: 2-4 days.
6. **Put a discriminant on the heap-header split** (B4): give `UnifiedValue`/`JitAlloc` a debug-mode canary field (or unify on `HeapHeader` layout — offsets are the only difference) so misrouted pointers fail fast instead of reading refcount-as-kind. Effort: 2-3 days.
7. **Get an ADR ruling on the two runtime-bits discriminations** (§6 ruling request: places.rs:522, rvalues.rs:1987) plus the handler-side probe inventory (B19): bless-with-carve-out or delete-by-proof. Effort: hours (ruling) + 1-2 days (either outcome).
7b. **Emit the GC safepoint poll in JIT loops** (B18): add `gc_safepoint` to `FFIFuncRefs` + `ffi_builder.rs` + symbol registry, emit the call at MirToIR loop back-edges gated on `loop_analysis::needs_gc_safepoint` (the classification already exists, unconsumed), and add a two-thread stop-under-JIT-loop test. Until then, document in the GC readiness notes that the Phase-3b rendezvous does not cover JIT frames. Effort: 2-3 days.

### P1.5 — coverage unlocks with the best value-per-effort (ordered by leverage)

The deopt frontier analysis (§2.6) suggests the highest-yield JIT-coverage work, in order:

- **(a) Thread declared struct-field types through the kind tracker** (B13) — unlocks struct-field arithmetic, probably the single most common deopt in typed-object-using programs. The schema information already exists; this is plumbing, not design. ~1 week.
- **(b) §2.7.17 receiver-recovery at the user-fn return boundary** (B15) — unlocks trinity `match`-destructure AND is the same root as the `?`-operator gate; two gates retired with one fix. ~1-2 weeks (it is the named v0.4 workstream).
- **(c) Per-kind `ConvertTo*` bodies** (B14) — mechanical per-kind codegen; unlocks `as` casts. ~3-5 days.
- **(d) Module-binding side-table for function bodies** (W39 F1 gate) — unlocks any function referencing a global. Design cost: needs the shared static side-table the gate text specifies.

### P2 — debt & ergonomics (as capacity allows)

8. Wire the JIT cache (pick ONE implementation, delete the other) keyed on blob content hash — dissolves per-run stdlib recompile and likely the deep-tests gating root cause. Effort: ~1 week incl. invalidation tests.
9. Shorten `[jit-fallback]` stderr to one sentence + reason code; move essays behind `--trace-jit` — and make `--trace-jit` actually exist in the default build first (B16). Hours.
10. Split `compile_terminator` and `jit_call_method` along their match arms; rename the strategy/trading vocabulary (`JittedStrategyFn` → `JittedProgramFn` etc.). 3-5 days, mechanical.
11. Delete the stale NaN-box-dispatch doc comment (rvalues.rs:1843-1851) and gc.rs:9-10 header; fix `verify-phase-5` to actually run the existing sentinel test (B12). Hours.
12. Raise SAFETY-comment coverage on the ~70 highest-risk unsafe sites (`from_heap_bits*`, Send impls, transmutes). 2-3 days.

---

## 13. Addendum: run/measurement log

All runs on the prebuilt working-tree debug binary, scratch dir `…/scratchpad/verticals/jit/`. Extension-load warnings elided per audit brief.

```
# hot loop (JIT, default mode) — no fallback line
$ shape run hotloop.shape            → 499999500000

# benchmark, 20M-iteration arithmetic loop
$ shape run --mode jit bench.shape   → 159999992   (3,107 ms)
$ shape run --mode vm  bench.shape   → 159999992   (57,813 ms)

# deopt-gate probes (stderr [jit-fallback] + correct interpreted output)
trait/impl        → Wave-20A user-trait-method JIT SURFACE …            ; output: woof
`?` operator      → c4-4B TryUnwrap SURFACE (ADR-006 §2.7.14) …         ; output: 43
`??` operator     → v0.3.3 null-coalesce SURFACE …                      ; output: 7
object literal    → Wave-17 scalar-move-lift SURFACE (`NewObject`) …    ; output: 42
enum match        → Route A SURFACE — enum constructor `Shape2::Square` ; output: 9.0
string .len()     → scalar-returning string method … no sound codegen   ; output: 11
global-in-fn      → W39 F1 module-binding SURFACE (LoadModuleBinding)   ; output: 105
closure `add1(41)`→ R8 W9 B1 W17-marshal SURFACE …                      ; output: 42
arr.map(|x|x*2)   → WF-1A finalize could not resolve main_f196_Vec.map… ; output: 20.0

# native-path probes (NO fallback)
fib(25) recursion → 75025 ; collatz(27) while-loop → 111
numeric array idx → 7.0   ; string constant print  → hello world
f-string interp   → value is 42
match on int lits → 200   ; loop { break 42 }      → 42

# second-batch deopt probes (stderr [jit-fallback] + correct interpreted output)
`3 as number`       → WS-12 ConvertTo* VM-only                          ; output: 6.0
struct field arith  → compile_binop_dynamic_arith kind-tracker gap      ; output: 7.0
match Some(v)/None  → EnumPayload §2.7.17 receiver-recovery gap         ; output: 7
async fn + await    → JitPreflightReport { vm_only_opcodes: [Await] }   ; output: 5
generic identity<T> → Route A: no proven FrameDescriptor.return_kind    ; output: 42

# third-batch native-path probes (NO fallback)
string == string    → 1    ; arr.len() → 3    ; arr.push + index → 3.0
for i in 0..=10 sum → 55   ; grade(85) multi-return-string fn → B ; avg(3.0,5.0) → 4.0

# P0 divergence found (NO fallback, both exit 0)
print(1.5d)         → jit: 0        vm: PT129600S
print(1.5d + 2.5d)  → jit: 0        vm: PT345600S

# --trace-jit advertised by --help, rejected by parser
$ shape run --trace-jit=shape_jit=debug f.shape
  error: unexpected argument '--trace-jit' found

# error parity
div-by-zero  jit: "Division by zero"            vm: "Division by zero (line 1)"
overflow     jit: "integer overflow: result…"   vm: "integer multiplication overflow: result of 9223372036854775807 and 3 … (line 1)"

# gates & tests
$ bash scripts/check-no-dynamic.sh               → exit 0, no regressions
$ cargo test -p shape-jit --lib registry_cross_check
    → ok. 6 passed; 0 failed; 511 filtered out; finished in 0.06s
    (i.e. default --lib test binary contains 517 tests at working tree)
```

Static counts: shape-jit = 69,211 LOC / ~100 files; 849 `#[test]`; 26 `#[ignore]`; 647 `unsafe {`; 69 `unsafe fn`; 7 `unsafe impl`; 237 `no_mangle` FFI symbols; 178 FFI symbols declared to Cranelift (ffi_builder.rs `r!` count); 129 SAFETY comments; 16 `allow(dead_code)`; 70 files with ADR markers.

Working-tree delta: 19 shape-jit files modified, +2,042/−324 (§1.7).

---

## Appendix A: representative full diagnostic transcripts (verbatim evidence)

### A.1 The `?`-operator gate, as a user sees it (stderr, single line wrapped here)

```
[jit-fallback] function main failed JIT compile: Runtime error: c4-4B TryUnwrap (`?` operator)
SURFACE (ADR-006 §2.7.14): the program contains an `OpCode::TryUnwrap` (the `?` operator) whose
unwrap-or-early-return semantics MIR collapses to a transparent copy at `mir/lowering/expr.rs:2594`.
The JIT-emitted code calls the inner expression via the trampoline (`dispatch_call_via_trampoline_vm`),
stores the trampoline's heap-Result/Option `u64` into a slot whose parallel-kind tracker records the
SUCCESS type's NativeKind (e.g. `Int64` for `Result<int,_>`), and the I64-wide arm of
`TerminatorKind::Return` at `mir_compiler/terminators.rs:1801-1813` stamps `RETURN_TAG_I64` on pointer
bits — silent-wrong-output `VM=42, JIT=Integer(137_900_062_693_984)` per
`regression::jit::jit_trampoline_result_callvalue`. Whole-program deopting to the bytecode interpreter
via this `[jit-fallback]` path preserves VM == JIT semantics (`op_try_unwrap` at
`executor/exceptions/mod.rs:658` executes the unwrap soundly through `read_result` / `read_option` /
`return_value_inner`). Tracked per supervisor 2026-05-28 c4-4B ratification +
`docs/cluster-audits/v0.3.3/04-pointer-as-float-leak.md` §4B (Sub-cluster 4B FN-REG-CORRECTNESS /
RELEASE-BLOCKING; this SURFACE-deopt is the ratified v0.3.3 fix shape); running under interpreter
```

~1,400 characters of internal governance prose on stderr for `let v = parse("42")?` (B9). The program then prints the correct `43`.

### A.2 The module-binding gate

```
[jit-fallback] function main failed JIT compile: Runtime error: JIT compilation failed: W39 F1
module-binding function-body SURFACE (ADR-006 §2.7.14): function 'add_base' contains
LoadModuleBinding at bytecode instruction 2001. Module bindings are not MIR places, so the JIT
function-body lowering has no compile-time side table for this storage. Running native top-level
code and then interpreting such a function through the trampoline VM would read an unsynchronized
module-binding array (observed VM=100 / JIT=0 on f1-shared-module-binding.shape). Whole-program
deopting to the bytecode interpreter via the existing `[jit-fallback]` path preserves VM == JIT
semantics until module-binding lowering is rebuilt with static metadata. total_accesses=1;
running under interpreter
```

Note `instruction 2001` for a 5-line program — the instruction index counts the whole compiled unit including the ~2,000-instruction stdlib prelude, incidentally confirming the per-run prelude compilation cost (§11.7).

### A.3 The shipped `--help` paragraph (verbatim; both fictions in one block)

```
- jit: Use JIT compilation (tiered: interpreter → baseline @ T1=100 calls → optimizing @ T2=10k
  calls). The toplevel script and every reachable function attempt JIT-compile when possible; on
  JIT-compile failure the executor falls through to the bytecode interpreter (NOT silent-no-output)
  and emits a `[jit-fallback]` diagnostic to stderr. Use `--trace-jit=shape_jit=debug` to promote
  per-function diagnostics. See `book/advanced/jit-compilation` "--mode jit semantics" for the full
  path (3) binding

  [default: jit]
```

Reality: no tiering (§2.2); `--trace-jit` → `error: unexpected argument '--trace-jit' found` (B16). The middle sentence (fall-through semantics) is accurate.

### A.4 Kind-tracker gap on struct fields (the highest-leverage coverage bug, B13)

```
[jit-fallback] function main failed JIT compile: Runtime error: JIT compilation failed:
compile_binop_dynamic_arith: kind-untyped arith Add reached the JIT — SURFACE per W10 playbook §5:
producing-MIR kind-tracker gap; every JIT operand must have a proven NativeKind at compile time
(ADR-006 §2.7.5 / CLAUDE.md "Forbidden code" — runtime tag_bits dispatch deleted with the W-series
IC).; running under interpreter
```

Program: `type Point { x: number, y: number }; let p = Point { x: 3.0, y: 4.0 }; print(p.x + p.y)`. Both operands are declared `number`; the kind tracker fails to carry field-read kinds into the binop. Output correct via fallback: `7.0`.

---

## Appendix B: independent second-pass verification (auditor 05, same day, fresh probe set)

A second, independently-constructed probe set was run against the same working-tree binary to re-verify the report's headline claims from scratch (different programs, different greps), and it is this pass that surfaced B18/B19. Everything below is primary evidence, not restatement.

### B.1 Tier-death chain — independent re-verification

Re-derived from zero, without reference to §2.2's citations:

```
$ grep -rn "set_backend\|JitCompilationBackend" bin/shape-cli/src crates/shape-vm/src --include='*.rs'
(no output)
$ grep -rln "JitCompilationBackend|set_backend" <whole repo, target/ excluded>
crates/shape-jit/src/lib.rs          # re-export only
crates/shape-jit/src/worker.rs       # definition + its own tests
crates/shape-jit/src/osr_compiler.rs # doc reference
$ grep -rn "TierManager::new" crates/shape-vm/src --include='*.rs' | grep -v test
(no output — every constructor call is inside #[cfg(test)] modules)
$ grep -rn "tier_manager" crates/shape-vm/src | grep "= \|Some("
executor/osr.rs:512,544,552; control_flow/mod.rs:285; vm_impl/init.rs:311
   → all are `if let Some(ref …)` READS; the only WRITE is init.rs:82 `tier_manager: None`
```

Same conclusion by a disjoint route: the backend trait object is never constructed anywhere outside shape-jit, and the manager that would drive it is never `Some`. Additionally, `bin/shape-cli/Cargo.toml:14` shows the `jit` feature = `["shape-jit", "shape-vm/jit"]` — the `shape-vm/jit` flag it forwards to "gates OSR/tier-dispatch integration code in the executor" (shape-vm Cargo.toml:66-69), i.e. the default build *compiles in* the dead tier-dispatch call sites, paying their code size for zero function.

Empirical corroboration on a bigger loop than §2.1's: a 30M-iteration `while` loop (`s = s + i*2 - i/3`):

```
--mode jit: 749999985000000   real 3.096s
--mode vm : 749999985000000   real 1m4.314s     (20.8x)
```

A live OSR tier would have promoted this loop at back-edge 1,000 (tier.rs `DEFAULT_OSR_THRESHOLD = 1000`); instead the interpreter ground through all 30M iterations at full dispatch cost — behavioral confirmation that no promotion path exists, on top of the structural proof. Recursion contrast: `fib(28)` measured jit 3.15s vs vm 4.64s (only 1.47x) — the JIT's win concentrates in loop bodies; call-heavy recursion is dominated by the uniform-I64 call convention and per-call overhead in the debug build.

### B.2 Second-pass gate/coverage probe matrix (disjoint program set from §13)

| Probe program | Mode-jit outcome | Ground truth |
|---|---|---|
| `fn add(a:int,b:int)->int` + call | native (no fallback), `5` | ✓ matches vm |
| `fn get(x)->Result<int,string>` + `get(42)?` inside a fn | `[jit-fallback] … c4-4B TryUnwrap SURFACE …`, then `Ok(42)` | correct via interpreter |
| `trait Named { fn name() -> string; }` + `impl Named for Dog` | `[jit-fallback] … Wave-20A user-trait-method JIT SURFACE …`, then `dog` | correct via interpreter; also confirmed trait methods use implicit receivers — `fn name(self)` is a compile error ("method receivers are implicit"), contradicting CLAUDE.md's trait example `fn method(self)` (cross-territory doc nit for auditor 01/17) |
| `match r { Ok(v) => print(v), Err(e) => print(e) }` | `[jit-fallback] … EnumPayload … variant = Ok … variant = Err …` (both arms listed), then `7` | correct via interpreter; confirms §2.6/B15 with a Result (first pass used Option) |
| `maybe(5) ?? 0` | `[jit-fallback] … null-coalesce SURFACE …`, then `5` | correct |
| `let f = \|x: int\| x * 3; print(f(7))` | `[jit-fallback] … W17-marshal-return-arms SURFACE …`, then `21` | even a *directly-invoked local closure* deopts via the imported-stdlib-call flag — the `has_w17_marshal_residual` classifier over-approximates beyond its nominal "imported stdlib fns" trigger (the prelude's closure plumbing trips it). Over-deopt = safe direction, but it means **no closure-calling program JITs at all** |
| `fn dist2(p: Point) -> number { p.x*p.x + p.y*p.y }` | `[jit-fallback] … finalize could not resolve … main_f195_dist2 …`, then `25.0` | struct-param functions fail Phase-4 compile (same class as B13, here surfacing as a finalize deopt because top-level referenced the demoted fn) |
| `fn sum(arr: Array<number>)` loop + `data.push(i as number)` | `[jit-fallback] … vm_only_opcodes: [ConvertToNumber] …`, then `499500.0` | one `as` cast anywhere in top-level code deopts the whole program (B14, bytecode-preflight layer this time) |
| `let base = 100; fn addbase(x:int)->int { x + base }` | `[jit-fallback] … W39 F1 … 'addbase' contains LoadModuleBinding at bytecode instruction 2001 …`, then `105` | confirms §2.3 gate + the ~2,000-instruction prelude observation (A.2) |
| `type P { x:int }; let p = P{x:42}; print(p.x)` | native (no fallback), `42` | top-level struct construct + field read DOES JIT — the deopt in the dist2 row is specifically about struct values crossing *function* boundaries |
| `fn greet(n:string)->string { "hi " + n }` | native (no fallback), `hi bob` | string concat + string param/return JIT when kinds are proven |
| `print("before"); print(10/0)` | `before` printed exactly ONCE, then `Error: Runtime error: Division by zero` | **empirically confirms the r5c-2 nested-result contract** (§3.1/§10.2): the runtime error propagates directly; no interpreter re-run doubles the side effect |
| `let mut total = 0; for x in [10,20,30] { total = total + x }` | native, `60` | for-over-array-literal JITs |

Delta vs the first pass worth recording: the closure row sharpens §2.3 — the practical rule for users is "any closure anywhere → interpreter", regardless of whether it flows through a HOF.

### B.3 GC integration — full two-sided verification (feature graph + emission)

Feature-graph (the memory-flagged two-tier trap), verified at all four Cargo.tomls:

```
shape-cli  : default = ["jit","gc"];  jit = ["shape-jit","shape-vm/jit"];  gc = ["shape-vm/gc","shape-jit?/gc"]
shape-vm   : default = ["jit","gc"];  gc = ["shape-value/gc"]      (jit = [] — flag only, no cranelift dep)
shape-jit  : default = [];            gc = ["shape-value/gc"]
shape-value: gc = []
```

→ the shipped binary enables the collector in both tiers; the `?`-weak-dep form means `--no-default-features --features gc` (interpreter-only gc) and `--features jit` (jit-without-gc, for barrier-cost measurement) both remain constructible, exactly as the Cargo.toml comments claim. **CONFIRMED FIXED** relative to the project-memory trap.

Emission-side (the new gap, B18): summarized evidence chain in §9; the decisive greps were

```
$ grep -rn "safepoint" crates/shape-jit/src --include='*.rs' | grep -v ffi/gc.rs
  → context.rs (field + comments), loop_analysis.rs (unconsumed classification + its tests) ONLY
$ grep -rn "needs_gc_safepoint" crates/shape-jit/src --include='*.rs' | grep -v loop_analysis.rs
  → (empty)
$ grep -n "jit_gc_safepoint" src/ffi_symbols/*.rs src/ffi_symbols/*/*.rs src/compiler/ffi_builder.rs
  → (empty)   # not even registered as a callable symbol
```

Also confirmed on the positive side: `jit_write_barrier` IS registered (ffi_symbols/object_symbols.rs:505-506, declared at :1372-1373; ffi_builder.rs:223) and emitted (places.rs:823 with the compile-time-baked kind tag per places.rs:788), and the gc-feature test suite in ffi/gc.rs (three substantive tests: barrier-buffers-survivor, JIT-produced-cycle-collected, set-field-overwrite-sink-collected, ffi/gc.rs:91-266) exercises the barrier path end-to-end through the real v2-raw carriers — good tests, but all of them drive the FFI directly; none executes a JIT-compiled loop under a concurrent stop request, which is exactly the hole B18 identifies.

### B.4 The JIT-internal NaN-box layout — two-tier HK constant audit (supplements §5.2/§6)

`ffi/value_ffi.rs` was read end-to-end. Facts the main body compresses:

- The tag scheme is fully self-contained (no `shape_value::tag_bits` import — that module is deleted): `TAG_BASE = 0xFFF8…`, 3-bit tag at bits 50-48 (`TAG_HEAP/INT/BOOL/NONE/UNIT/FUNCTION`), i48 payload, plus `UNIFIED_HEAP_FLAG` (bit 47) and a low `HEAP_OWNED_BIT` masked off on pointer reads (value_ffi.rs:49-105). Compile-time layout asserts pin every sentinel into negative-NaN space (value_ffi.rs:248-266).
- The HK constant table is **two-tier by design** (W17-jit-legacy-ordinal-disambiguation, value_ffi.rs:146-245): Tier 1 = 10 canonical constants aliased directly to `HeapKind as u16` (`HK_STRING`, `HK_TYPED_OBJECT`, `HK_CLOSURE`, `HK_DECIMAL`, `HK_BIG_INT`, `HK_DATATABLE`, `HK_HASHMAP`, `HK_FUTURE`, `HK_TASK_GROUP`, `HK_FILTER_EXPR`); Tier 2 = **39 JIT-private ordinals rebased to `JIT_LEGACY_HK_BASE = 256`**, each annotated with the runtime-HeapKind ordinal it *used to collide with* (e.g. `HK_ARRAY: 256 — was 1 (collided HeapKind::TypedObject)`, `HK_SOME: 264 — was 14 (collided HeapKind::NativeScalar)`). A `const _:` assert enforces the base sits above HeapKind + the 128..132 `HK_JIT_*` block + `v2_struct` 132 (value_ffi.rs:237-245), and verify-merge CHECK 12 greps the invariant.
- Assessment: the collision-renumbering is disciplined damage control on a real past hazard (39 dispatch labels silently aliasing runtime kinds), and the compile-time assert + merge-gate belt-and-braces is the right shape. But it also quantifies the §5.2 split-brain: **49 JIT-private kind labels** exist alongside the runtime's ~34 HeapKinds, and the Tier-2 block's own comment concedes the guard exists so that a runtime slot crossing the boundary "cannot collide with a JIT-internal `JitAlloc<T>` prefix" — i.e. mixed-provenance bits at this boundary are an anticipated event, not a type-system impossibility.
- Deprecation debt inside the same file: the `box_ok`/`box_err`/`box_some`/`unbox_result_inner` carriers are annotated "Retired … Generated JIT code must not use these helpers … remain only as compatibility definitions for old boundary conversion/tests until that surface is deleted" (value_ffi.rs:360-367) — live pub functions whose doc says they must not be called; a `#[deprecated]` attribute would let the compiler enforce what the comment requests.
- Confirmed deletion (good): `box_typed_object`/`unbox_typed_object`/`is_typed_object` and the JIT-private inline-cell TypedObject struct are gone (Wave-7 Phase C, value_ffi.rs:517-524); TypedObjects now ride the v2-raw `*mut TypedObjectStorage` carrier uniformly — this is the migration whose GC payoff the ffi/gc.rs cycle tests prove.
- The "JIT magic byte at offset 3" from project memory (`project_jit_vm_value_safety.md`) is **gone**: `grep -rni magic` across shape-jit/shape-vm/shape-value → zero relevant hits; `UnifiedValue` offset 3 is a zeroed `_reserved` byte (jit_kinds.rs:93). The memory note is stale; discrimination is now purely producer-side kind stamping (which is what makes B4's convention-only header split load-bearing).

### B.5 Test-mass distribution — deep-tests gating quantified (supplements §7.1)

Per-module `#[test]` counts in the `deep-tests`-gated set (grep -c per file):

```
integration_tests.rs 151   closure_dispatch_regression_tests.rs 34
v2_array_tests.rs     17   groupby_surface_regression_tests.rs  11
short_circuit          10   typedarray_ptr_regression_tests.rs   10
a1e_tests.rs            9   array_builder_regression_tests.rs     9
field_ref               8   ref_param_regression_tests.rs         6
a1d2_tests.rs           6   jit_array_param_regression_tests.rs   5
fuzzy_comparison        3                                  Σ ≈ 279
```

279 of 849 tests (33%) are compiled only under `deep-tests`; with the 26 `#[ignore]`s and the cfg'd-out fuzz harness, the default `cargo test -p shape-jit` exercises ~57% of the written test mass. The gating *reason* (each heavy test JIT-compiles the full stdlib prelude; SIGILL race at default parallelism) was re-confirmed as still-standing from Cargo.toml:33-40 and mir_compiler/mod.rs:37-44 — and remains downstream of the unwired JIT cache (§2.4): fix the cache, dissolve the gate.

Also verified: the 26 `#[ignore]` texts are exceptional documentation — the simulation ignore (ffi_symbols/simulation/mod.rs:118) is a 12-line essay explaining precisely why the test would SIGABRT (extern-C `todo!()` in `jit_call_value`'s per-row loop), which sibling tests stay green and why, and the re-enable condition. Ignore hygiene in this crate is a model for the rest of the workspace.

### B.6 Executor fallback architecture — line-level walk (supplements §1.2)

A full read of `executor.rs` (1,149 lines) confirms the §2.3 gate order and adds two structural observations the main body should carry:

1. **The mixed table is discarded at the only call site.** `compile_program_selective` returns `(jit_fn, MixedFunctionTable)`; the executor binds it as `let (jit_fn, _mixed_table) = …` (executor.rs:702). The interpreted-function story at runtime is instead: null `function_table` entries + the thread-local trampoline VM (`set_trampoline_vm`, executor.rs:832-836, with the `TrampolineGuard` drop guard). So the book's MixedFunctionTable narrative (§8) is doubly wrong: `Pending` never produced, and the table itself is write-only in production.
2. **The trampoline VM setup deliberately bypasses the linker** — `trampoline_bytecode.content_addressed = None` before `load_program` (executor.rs:828-831) because the linker's topological renumbering would break JIT↔interpreter function-id parity. Correct, but it means content-addressed dispatch and `--mode jit` are mutually exclusive paths sharing one bytecode format — a constraint nowhere stated in the content-addressed-bytecode book chapter (cross-ref for auditor 17).
3. The RETURN_TAG match (executor.rs:1047-1108) was verified arm-by-arm: F64/I64/I32/BOOL/UNIT are pure bit reinterpretations; STRING takes ownership of an `Arc<String>` via `from_raw` + clone + drop (balanced); the `_ =>` arm is a hard error carrying the raw bits in hex — the surface-and-stop terminus, with the deleted `typed_scalar_to_wire`/`value_word_to_wire` decode helpers explicitly memorialized as forbidden in the trailing comment block (executor.rs:1135-1148). No decode path survives at the host boundary. CONFORMANT.

### B.6b The `jit_call_value` stack protocol — line-level walk (supplements §2.9)

The indirect-call FFI's pop discipline (ffi/control/mod.rs:420-530) was verified against the ADR-006 §2.7.7/Q9 lockstep invariant it claims:

1. **`arg_count` pop**: raw i64 (per the MIR-side `iconst(I64, args.len())` push at terminators.rs:681); the vacated slot's kind byte is immediately overwritten with `stack_kind_code::SENTINEL` — mirroring the VM's `pop_kinded` dead-slot discipline (`vm_impl/stack.rs:706` per the comment). Every subsequent pop repeats this hygiene write.
2. **Args pop as `(bits, kind)` pairs** read from the SAME slot index of `stack`/`stack_kinds` (the lockstep invariant), reversed to source order. A `None` from `stack_kind_code::decode` — i.e. a SENTINEL or reserved byte reaching a consumer — is treated as a producer kind-source gap: logged with the §2.7.7 #9 citation and refused (currently via `return TAG_NULL` — the B2 arm; the *classification* is conformant, the *response* is the finding).
3. **Callee pop + kind decode**: the callee's kind **is** the §2.7.11/Q12 dispatch discriminator — explicitly no tag-bit decode on `callee_bits`, no `is_heap()` probe (both cited as forbidden in the arm comments). Dispatch mirrors the VM's `dispatch_call_value_immediate` (`executor/control_flow/mod.rs:389`) arm-for-arm: `Ptr(HeapKind::Closure)` → borrow the existing `OwnedClosureBlock` into the trampoline VM (never re-materialize from raw captures — unsound for cell-storage captures, per comment); `UInt64/Int64`-class → function-id path; anything else → surface.
4. **Return-share transfer**: a successful trampoline `execute_closure` returns a `KindedSlot`; the FFI takes its raw bits and `std::mem::forget`s the slot so the owning share transfers to the JIT destination (control/mod.rs:262-267) — the refcount-balance idiom the W11 arc counters (§10.8) exist to audit.

Verdict: the protocol itself is the most ADR-conformant §2.7.11 implementation in either tier; its two weaknesses are the silent-null failure response (B2) and the fact that the sibling `dispatch_module_fn_call` landing point is still `todo!()` (control/mod.rs:283-297).

### B.6c JITContext ABI struct — vestigial-field inventory and capacity limits

Full-field read of `JITContext` (context.rs:560-720):

- **Vestigial trading-domain fields still in the hot ABI struct**: `in_position: bool`, `position_side: i32`, `entry_price`/`unrealized_pnl_pct` (NaN-boxed), `timestamps_ptr`, `column_ptrs/column_count/row_count/current_row` (DataFrame iteration), `alert_pipeline_ptr`, `simulation_mode/simulation_state_ptr/simulation_state_size` — roughly a third of the struct serves the retired backtesting product (§3.4). They are not free: every field participates in the cross-crate offset contract (§5.6), so deleting them later is an ABI-breaking event that gets more expensive the longer they stay.
- **Fixed capacities without emitted guards**: `locals: [u64; 256]`, `stack: [u64; 512]` + `stack_kinds: [u8; 512]`. FFI-side pops check `stack_ptr == 0` underflow (control/mod.rs:433 etc.), but I found no *overflow* check on the JIT-emitted push side — the emitted code indexes `stack[stack_ptr++]` by construction-time layout. Today the compiler bounds stack depth statically per expression tree, and deep *call* recursion uses the native machine stack (not `ctx.stack`), so overflow requires a single expression pushing >512 intermediate FFI operands — improbable but unguarded; a compile-time max-depth assertion in MirToIR would close it for free.
- **Async fields are wired but idle**: `event_queue_ptr`, `suspension_state`, `iterations_since_yield`, `yield_threshold` (default 0 = never yield) — the cooperative-scheduling half of a JIT-async story whose opcodes are all VM-only (§2.7); another instance of callee-half-without-emitter, structurally identical to B18.

### B.7 MirToIR side-table inventory and the per-function compile ceremony (supplements §1.3)

`compile_function_with_user_funcs` (program.rs:253-581) was read line-by-line; the per-function MirToIR instantiation threads **nine distinct side-tables**, each carrying a compile-time-proven fact from the bytecode compiler into codegen. This is the concrete embodiment of ADR-006 §2.7.5 "stamp at compile time", and also the maintenance surface a future contributor must understand before touching the pipeline:

1. **`slot_kinds: Vec<Option<NativeKind>>`** from `Function.frame_descriptor` (program.rs:362-366) — the per-slot kind proof; `None` = surface-and-stop downstream, never Bool-default (mod.rs:199-203).
2. **`concrete_types: Vec<ConcreteType>`** from `BytecodeProgram.function_local_concrete_types[func_idx]` (program.rs:385-389) — the W12 Round-5B conduit that extended the top-level `Point{}`-literal short-circuit to user-fn bodies (`Ok(v)`/`Err(e)`/`Some(x)` inside 28 stdlib helpers, per the comment at :367-384).
3. **`function_indices: HashMap<String, u16>`** built from the ORIGINAL program's functions (program.rs:390-397) because the sub-program's function list is deliberately emptied — a name→id map whose correctness rests on the mangling convention (§4.7).
4. **`user_func_refs/arities/return_kinds`** — the direct-call plumbing (JIT→JIT calls bypass FFI).
5. **`closure_function_layouts: HashMap<u16, Arc<ClosureLayout>>`** (program.rs:398-410) — Closure-spec Phase H1 natural-width capture offsets, bypassing the legacy `jit_make_closure` FFI.
6. **Monomorph routing context** (program.rs:427-442): the V3-S6b side-table keyed `(call_site_span, caller_function_id)` re-routes `MirConstant::Method` sites to direct FuncRef calls — and must be threaded from the *original* program because the sub-program clears it; a subtle aliasing trap documented in place.
7. **Operator-trait dispatch sites** (program.rs:443-450) — re-emits `Rvalue::BinaryOp` at trait-dispatch spans as method-call IR (W10 user-trait fix).
8. **Bounds-elision plan** (program.rs:451-456) — the live optimization pass (§10.11).
9. **`field_byte_offsets` pre-populated from the schema registry** (program.rs:457-478) — the W14.2-E SURFACE-A2 *soundness* fix: without the pre-pass, trait-impl bodies reading `self.field` fell through to `jit_get_prop`, whose `heap_kind(obj_bits)` probe returns `None` on v2-raw carriers → `TAG_NULL` garbage. Note what this fix admits: the legacy probe path (B19) was the *default* consequence of a missing side-table entry — the probe is the fallthrough, the stamp is the opt-in. Inverting that default is the structural fix B19's ruling should weigh.

Plus the capture-slot registrations (`register_owned_mutable_capture_slots`, program.rs:489-500 — A.1D.2) and the shared-local cell allocation (`initialize_shared_local_slots`, program.rs:515-523 — Session 1 Commit 3, whose end-to-end preflight gate is still closed pending the "outer-frame cell-identity handshake", accessors.rs:640-652).

**Call convention** (program.rs:530-563, R4.2E): every user function takes `(ctx_ptr, capture0..N, param0..M)` as uniform `I64`s and returns `I32` (the signal); at entry, each param is narrowed to its proven slot kind — `bitcast F64`, `ireduce I32/I16/I8` — with an explicit "No NaN-box tag stripping — raw bit-patterns only" note. Closures receive captures as leading native args (`effective_arity = captures_count + arity`, program.rs:271/801). The uniform-I64 ABI is simple and correct but means every f64 argument crosses a GPR↔XMM bitcast at each call boundary — part of why the recursion speedup (1.47x, B.1) lags the loop speedup (20.8x); a typed-signature ABI keyed on the FrameDescriptor is the natural successor once return-kind stamping is universal.

### B.7b Self-audit tooling the crate already exports (unused leverage)

`lib.rs:75-86` publicly re-exports a parity-matrix toolkit that this audit's recommendations could be mechanized on top of: `JitParityEntry`/`JitParityTarget`/`JitPreflightReport`, `build_full_opcode_parity_matrix`, `build_full_builtin_parity_matrix`, `build_program_parity_matrix`, `get_incomplete_opcodes`, `get_unsupported_opcodes`, `preflight_blob_jit_compatibility`. These generate exactly the "which opcodes/builtins does the JIT claim vs support" inventory that §2.7 assembled by hand — but nothing in `bin/` or `tools/` consumes them (grep: no callers outside shape-jit and its tests). Two cheap wins: (a) a `shape jit-coverage <file>` CLI subcommand printing the preflight report would replace the stderr-essay guesswork for users hitting deopts; (b) a CI job diffing `build_full_opcode_parity_matrix()` output across commits would catch silent coverage regressions (a gate added/removed) the way the B0 Duration hole was *not* caught. Similarly, `maybe_emit_numeric_metrics` (program.rs:78-126) computes a typed-opcode coverage percentage per program behind the `jit-trace` feature — with B16 fixed, this becomes a free coverage telemetry channel.

### B.8 Cross-territory handoff notes (found in-territory, owned elsewhere)

1. **Parser (auditor 01):** `match` as a trailing expression in a block (`fn f(x) { match x { … } }` without `return`) fails to parse: "unexpected \`}\`, expected something else" — the book and CLAUDE.md present match-as-expression. Also `HashMap<string, int>()` constructor syntax fails to parse ("Syntax error near: , int>()") while CLAUDE.md lists `HashMap()` construction.
2. **CLAUDE.md maintainers (auditor 18):** the trait example `fn method(self)` is rejected by the shipped parser ("method receivers are implicit. Use \`method name(...)\` without \`self\`"). CLAUDE.md's Language Features table needs the implicit-receiver form.
3. **GC vertical (auditor 06):** B18 — the Phase-3b rendezvous has no JIT-side emission; the GO-WITH-CAVEATS readiness verdict (commits 5be24572/c5be82cd) should gain this caveat explicitly.
4. **Book vertical (auditor 17):** the fabricated JIT Dispatch Table API (§8), plus the undocumented mutual exclusion between content-addressed linking and `--mode jit` (B.6 item 2).
5. **Project memory:** `project_jit_vm_value_safety.md` (magic byte at offset 3) is stale — mechanism deleted (B.4); `project_jit_closure_fix.md` remains accurate as the named blocker for the shared-local preflight gate (accessors.rs:640-652 cites it).

### B.9 Second-pass raw command log

```
# tier-death re-derivation
grep -rn "set_backend\|JitCompilationBackend" bin/shape-cli/src crates/shape-vm/src   → (empty)
grep -rn "TierManager::new" crates/shape-vm/src | grep -v test                        → (empty)
grep -rn "tier_manager" crates/shape-vm/src | grep "= "                               → init.rs:82 None only

# GC feature graph
grep -n gc bin/shape-cli/Cargo.toml crates/shape-vm/Cargo.toml crates/shape-jit/Cargo.toml crates/shape-value/Cargo.toml

# safepoint emission gap (B18)
grep -rn safepoint crates/shape-jit/src | grep -v ffi/gc.rs      → context.rs + loop_analysis.rs only
grep -rn needs_gc_safepoint crates/shape-jit/src | grep -v loop_analysis.rs           → (empty)
grep -n jit_gc_safepoint src/ffi_symbols/*.rs src/ffi_symbols/*/*.rs src/compiler/ffi_builder.rs → (empty)

# probes (all default --mode jit unless noted; outputs in B.2)
simple add(2,3)=5 · tryop `?`→fallback→Ok(42) · traits→fallback→dog · match Ok/Err→fallback→7
coalesce→fallback→5 · closure f(7)→fallback→21 · dist2→fallback→25.0 · arrsum+`as`→fallback→499500.0
globread→fallback→105 · objtop p.x=42 native · strret "hi bob" native · forloop 60 native
strops .len()→fallback→11 · arrmap→fallback→[2,4,6,8] · fib(28)=317811 native
sideeffect: "before" ×1 then div-zero error (no re-run)

# timings (debug binary)
hot.shape 30M-iter while:  jit 3.096s  vm 64.314s   (20.8x)
fib(28):                   jit 3.154s  vm  4.637s   (1.47x)

# counts
find crates/shape-jit -name '*.rs' | xargs wc -l                 → 69,211 total
grep -rc '#[test]' src                                           → 836 (src) + 13 (tests dirs)
deep-gated per-module counts                                     → B.5 table (Σ 279)
unsafe: 647 blocks / 69 unsafe fn;  extern "C" fn tokens: 446;   ADR-005 markers: 4 files; ADR-006 refs: 508 lines
git diff --stat -- crates/shape-jit                              → 19 files, +2,042/−324
```

### B.10 Corrections and confirmations ledger for this report

| Claim in main body | Second-pass disposition |
|---|---|
| §2.2 tier-death chain | **CONFIRMED** by disjoint greps + 30M-loop behavioral test (B.1) |
| §2.5 "jit_gc_safepoint parks … at loop back-edges" | **CORRECTED** — the *function* parks; nothing *emits* it (B18). §2.5's wording inherited the source comment's fiction |
| §2.1 ~18.6x speedup | **CONFIRMED** at 20.8x on a different loop; recursion ratio much lower (1.47x, B.1) |
| §2.3 closure deopt "via stdlib-HOF Phase-4 failure" | **SHARPENED** — direct local-closure invocation also deopts (W17 flag), so closure coverage is 0% (B.2) |
| §10.2 double-execution fix | **EMPIRICALLY CONFIRMED** (`before` printed once, B.2) |
| Project memory "JIT magic byte at offset 3" | **STALE** — mechanism deleted; update `project_jit_vm_value_safety.md` (B.4) |
| GC two-tier Cargo trap (project memory) | **CONFIRMED FIXED** at working tree (B.3) |
| CLAUDE.md trait example `fn method(self)` | **STALE vs parser** — explicit `self` rejected ("method receivers are implicit"); cross-territory (B.2) |

### B.10b Gate-retirement dependency map (for the v0.4 JIT-lowering workstream planner)

The eleven+ gates do not need eleven fixes; they cluster on four root causes. Retiring a root retires every gate above it:

| Root cause | Gates it retires | Named fix |
|---|---|---|
| Return-kind boundary: user-fn returns don't stamp the strict carrier kind (§2.7.17 receiver-recovery) | `?` (TryUnwrap), `??` (null-coalesce), EnumPayload preflight (all trinity `match`-destructure), generic-fn Route A | P1.5(b) — one workstream, four gates |
| Kind tracker doesn't thread schema field types through Place::Field reads | struct-field arithmetic (B13), struct-param Phase-4 failures, part of the trait-method divergence | P1.5(a) |
| No static side-table for non-MIR storage (module bindings; shared cells) | W39 F1 module-binding gate, the four `AllocSharedLocal`-family opcodes, A.1C.3 module-binding opcodes | the "static metadata" rebuild the gate text specifies |
| Kinded value-call/handler ABI rebuild unfinished on the JIT side (`dispatch_module_fn_call` todo, HOF stubs, closure Phase-4 failures) | W17-marshal gate, closure deopts, `Vec.map` finalize deopts, the six armed HOF `todo!()`s (B11) | phase-2c §2.7.10/Q11+§2.7.11/Q12 completion |
| (standalone) `ConvertTo*` typed bodies | the 12-opcode `as`-cast family (B14) | P1.5(c), mechanical |
| (standalone) Drop-trait dispatch in `emit_drop` | user-`impl Drop` gate | scoped codegen addition |

Sequencing implication: the first two rows are prerequisites for most real-program coverage; the trait/impl whole-program gate (Wave-20A) sits atop BOTH the kind-tracker and return-kind roots and should be retired last.

### B.11 Final scoring rationale after the second pass

Scoring inputs, restated for the roll-up auditor:

- **Live surface**: 1 of 3 advertised JIT levels (AOT selective) actually executes; tiering and OSR are structurally unreachable (B.1), and the JIT cache is unwired — so "feature-complete" must be judged against the AOT path alone.
- **Correctness on the live surface**: one silent-wrong-output found across two probe passes (B0, Duration), zero on the gated paths — every deopt produced VM-identical output.
- **Coverage of the live surface**: numerics/control-flow/strings-constants JIT; closures, Result/Option destructuring, casts, traits, struct-params, globals-in-fns all deopt (B.2 matrix).

The second pass moved no score but sharpened both justifications. **Feature completeness stays 38/100**: the second probe batch found one more native-path win (top-level struct construct + field read, B.2) but also two coverage-narrowing sharpenings (all closures deopt, not just HOF-routed ones; struct-*param* functions fail Phase-4) — net neutral against the §2.1/§2.6 frontier. **Code quality stays 72/100**: B.6b/B.7 confirmed the conformance discipline is deeper than the first pass credited (lockstep pop hygiene, nine stamped side-tables, share-transfer idioms), but B18 is a materially worse omission than anything in the first pass's P2 list — a landed "Phase complete" commit (0c792bbf) whose emission half does not exist, undetected by its own readiness validation. Those two adjustments cancel. The deciding factor keeping quality above 70 despite B0/B18: every failure found in both passes was a *missing* mechanism or an *over-honest* refusal, never a fabricated proof — the crate does not lie to itself in code, only in prose.

— end of report —


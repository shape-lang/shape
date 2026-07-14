# Vertical Deep-Dive Audit 10: Comptime & Annotations

**Date:** 2026-07-11
**Auditor:** vertical auditor 10 of 19 (ultra-deep-dive audit)
**Scope:** comptime evaluation machinery (comptime blocks, `comptime fn`, `comptime for`, comptime builtins: `type_info`, `implements`, `warning`, `error`, `build_config`, `string_lit`, `item_fn`), the annotation system (`annotation` definitions with `before`/`after`/`comptime pre|post`/`on_define`/`metadata`, target validation, chaining), generated-function visibility, comptime in imported modules.
**Tree state:** dirty working tree at `main` (HEAD ce332ca2), audited as-is.
**Method:** ~70 empirical Shape programs executed against the prebuilt working-tree binary (`target/debug/shape`, default `--mode jit`), plus source reads across `crates/shape-vm/src/compiler/comptime*.rs`, `functions_annotations.rs`, `crates/shape-runtime/src/annotation_context.rs`, stdlib `.shape` sources, the book chapters, and ADR-005/006. All transcripts in this report are pasted from actual runs (extension-loader lines filtered). A same-day second verification pass independently re-ran the four highest-severity findings (§9.1 array garbage, §9.2 qualified `target.name`, §9.4 `on_define` silence, §9.12 ctx state shift) — **all reproduced byte-identically** — and added a second probe wave (the `n`-series programs) covering annotation-definition arguments, the shipped finance stdlib annotations, comptime-field specialization, annotated methods, nested comptime, and the grammar/LSP surfaces (§1.6-§1.7, §2.16-§2.18, §7.5, §8.6, §9.16-§9.19).

---

## 0. Executive summary

### Overall health verdict

The comptime/annotation vertical is in **substantially better shape than the 2026-07-04 audit's stale leads suggested, but with a hard cliff at module boundaries and several silent-wrong-behavior bugs at the comptime→runtime value boundary**. The WF-1B/WF-3D core genuinely landed: every documented comptime builtin works end-to-end, all four flagship stdlib LLM/derive patterns (`@json_schema`, `@to_json`, `@llm_tool`, `@prompt`) produce byte-for-byte the output the book promises, generated free functions are visible from user code, `extend`/`replace body`/`replace module`/`set return`/`set param`/`remove target` directives all apply for entry-file targets, and the diagnostics pipeline (LSDS `C0001`/`C0002`, jargon firewall, 5-second watchdog, `--diagnostics json`) is genuinely well-engineered.

The cliff: **the flagship codegen pattern breaks the moment the annotated type lives inside any module** — same-file `mod` blocks corrupt the payload one way, imported modules another (`target.name` arrives module-qualified, so `extend (f"fn {target.name}_x…")` generates unparseable source). Since real programs put types in modules, "comptime excellence" currently holds only for single-file scripts. On top of that sit a P0 type-confusion bug (comptime array results silently embed as garbage Decimal literals via an unsound `as_heap_value()` read), silently-dropped `on_define`/`metadata` hooks for two of three documented target kinds, a dormant `comptime for` statement form, and three annotation target kinds (`expression`, `block`, `await_expr`) that compile but die at runtime on a `op_new_array` SURFACE stub. Every runtime-hook annotation also forces a whole-program JIT deopt that spews internal audit jargon to stderr on the default mode — the exact vocabulary the comptime jargon-firewall (P10) was built to keep away from users.

The second verification pass sharpened the picture on the annotation side: the *only* production-scale annotation consumers in the codebase (finance stdlib) tell an unflattering story — `@warmup` was deliberately stripped to an inert marker and `@indicator` is an un-compilable fossil on a dead handler contract with zero users and zero tests — and two more default-mode cliffs surfaced (annotated `extend`-methods hard-error under JIT where `--mode vm` works; the book's comptime-field specialization form silently loses the field).

### Top-10 findings

| # | Severity | Finding | Evidence |
|---|----------|---------|----------|
| 1 | **P0** | `comptime { [1,2,3] }` silently embeds **garbage** (prints `33D`; element access dies on a `Ptr(Decimal)` SURFACE error) — unsound `as_heap_value()` on a v2-raw TypedArray pointer misreads it as `HeapValue::Decimal` | §9.1 transcript; `comptime.rs:1551` fallthrough; deleted arm at `comptime.rs:1560-1564` |
| 2 | **P1** | `extend (expr)` codegen breaks for targets in **imported modules**: `target.name` is module-qualified (`gen4::Gizmo`), generated source `fn gen4::Gizmo_tag()…` fails to parse | §9.2 transcript (error-probe shows `PAYLOAD=[fn gen4::Gizmo_tag() -> int { 7 }]`) |
| 3 | **P1** | `extend (expr)` codegen also breaks for targets in **same-file `mod` blocks** — payload corrupts differently (`found \``\``); module compile error then demotes to a runtime "Unknown qualified call" | §9.3 transcript |
| 4 | **P1** | `on_define`/`metadata` hooks **silently never fire** for function targets unless the annotation also declares `before`/`after`; never fire at all for module targets. Type targets fire | §9.4 transcripts; silent `continue` at `functions_annotations.rs:555-557` |
| 5 | **P1** | `expression`, `block`, and `await_expr` annotation targets compile but **fail at runtime** on the `op_new_array(0)` V3-S5 ckpt-5 SURFACE stub (wrapper args-array construction) | §9.5 transcripts |
| 6 | **P1** | `comptime for` is broken both ways: as a statement it is **dormant** ("pending the phase-2c … rebuild"), ranges don't even parse; inside annotation handlers it runs but the loop variable is typed `unknown`, so the book's own field-loop pattern fails `string + unknown` (plain `for` works — the stdlib quietly uses plain `for`) | §9.6 transcripts; grammar `shape.pest:152`; `derive.shape:40` |
| 7 | **P1** | Every runtime-hook annotation (and comptime-`set param`-defaulted function) forces a **whole-program JIT deopt** with multi-line internal-jargon stderr noise on the default mode (`[jit-fallback] … WF-1A signal-reexec (audit 2026-07-04 §4(a)) …`, Cranelift verifier arity mismatch for `set param`) | §9.7 transcripts |
| 8 | **P1** | `comptime { {k: v} }` anonymous-object results silently embed as **null** — runtime dies with "expected object … got scalar"; `build_config()` (TypedObject) works | §9.8 transcript |
| 9 | **P2** | `type_info` misclassifies enums as `TypedObject` (TypeKind has no `Enum` variant); chained `type_info` on array/generic sources returns `Unresolved`; array types render as non-source syntax `[string]` | §9.9 transcript; classifier at `comptime_target.rs:56-87` |
| 10 | **P1** | The `before`-hook `state:` rebuild contract is broken by a field-offset shift: after rebuilding, `after` reads `ctx.state == []` (the event_log) and `ctx.event_log == None`; the new state is lost. Trait-method dispatch inside comptime blocks fails in all three trait/impl comptime configurations | §9.12/§9.13 transcripts |

(Also notable below the top-10: `expand-comptime` blind to `extend (expr)`/`replace module` output, comptime warnings filed under `<synthetic>`, book documents the renamed-away `ctx.__impl` — §9.10, §9.14.)

Second-pass additions (verified against the same binary, transcripts in §9.16-§9.19):

| # | Severity | Finding | Evidence |
|---|----------|---------|----------|
| 11 | **P1** | The shipped stdlib `@indicator` annotation (`std::finance::annotations::indicator`) **does not compile when applied** — its handlers use a legacy `self`-based contract (`self.name`, `ctx.get("registry")`) that no longer resolves; `unknown + unknown` strict-typing error at the application site | §9.16 transcript; `indicator.shape:26,33` |
| 12 | **P1** | Annotated `method`s in `extend` blocks **hard-fail at runtime under the default JIT mode** (`JIT method dispatch for `get_x` resolved `Point.get_x` but that method was not JIT-compiled`) while working correctly under `--mode vm` — a VM/JIT behavioral divergence, worse than the function-target whole-program deopt | §9.17 transcript |
| 13 | **P1** | The book's comptime-field specialization form `type Celsius = Unit { symbol: "°C" }` (`comptime.mdx:317`) parses and instantiates but the field is **gone at runtime** (`Undefined property: symbol`); the base type's comptime field works (via interpreter deopt) | §9.18 transcript; grammar `shape.pest:101,110-115` |
| 14 | **P2** | Nested comptime blocks type as `unknown` (`comptime { comptime { 20 } + 22 }` → strict-typing error); comptime blocks cannot call plain same-file functions (by-design isolation, but undiagnosed as such: plain "Undefined function") | §9.19 transcripts |

### Feature-completeness score: **58/100**

Everything works impressively in a single entry file — builtins, directives, stdlib derive/LLM patterns, specialization, watchdog, diagnostics — but the vertical loses ~25 points for the module-boundary cliff (findings 2, 3) that invalidates the flagship pattern in any real program layout, ~13 more for the dormant/broken sub-features (comptime for, three runtime target kinds, on_define inconsistency, array/object literal projection), and ~4 more from the second pass: the only two non-demo stdlib annotations are a broken fossil and a deliberately emptied marker (finding 11/§2.17), annotated methods hard-fail on the default mode (finding 12), and the book's comptime-field specialization form loses the field (finding 13).

### Code-quality score: **74/100**

The comptime core is thoughtfully engineered: single execution model (mini-VM reusing the real compiler+VM), jargon firewall with acceptance-probe tests, LSDS-first diagnostics, watchdog, extensive ADR-006 marker comments (42 across the four core files), and honest surface-and-stop stubs instead of silent fallbacks. Points off for: two ~3,300-3,600-line files with 200+-line functions, `unsafe` raw-pointer plumbing concentrated at the KindedSlot readback boundary with one demonstrably unsound fallthrough (finding 1), a dead-code module (`comptime_concrete.rs` — `#[allow(dead_code)]` "until the wiring lands"), weak assertions in exactly the tests that would have caught finding 1, and the target-descriptor schema being defined four separate times that must stay in sync by hand.

### Biggest risk

The user's priority spine names "comptime excellence" as a flagship. The flagship demo — an annotation reads `target.fields` and generates a function — is genuinely excellent in a demo file and **silently falls off a cliff in the first real project** that puts `type User` in a module: the same annotation that worked yesterday emits an unparseable payload with an error message ("invalid replacement module payload") that names neither the annotation author's bug (there is none) nor the real cause (qualified `target.name`). Combined with the P0 array-literal corruption — which the existing test `ct_34_comptime_array` cannot catch because it never reads the array back — the pattern is: the paths users will hit *second* (modules, arrays, objects) fail silently or confusingly right after the paths they hit *first* work beautifully. That asymmetry is how a flagship feature earns a reputation for flakiness.

---

## 1. Architecture & code structure map

### 1.1 Module inventory (working tree, `wc -l`)

| Module | LOC | Responsibility |
|--------|-----|----------------|
| `crates/shape-vm/src/compiler/comptime.rs` | 3,599 | Mini-VM driver: wraps comptime statements into a synthetic `Program`, compiles with a nested `BytecodeCompiler` in comptime mode, executes in a fresh `VirtualMachine`; KindedSlot→AST literal/expr readback (`nb_to_expr`, `nb_to_literal`, `typed_object_to_object_expr`); builtin forwarder synthesis; bare-type-identifier rewriting; annotation-handler execution entry points |
| `crates/shape-vm/src/compiler/comptime_builtins.rs` | 1,491 | The `__comptime__` extension module: `implements`, `warning`, `error`, `build_config`, `type_info`, `item_fn`, `string_lit`, `__emit_*` directive intrinsics; `ComptimeDirective` + `ComptimeDiagnostic` thread-local channels; payload parsers (`parse_module_items_payload`, `parse_function_body_payload`, `parse_type_annotation_payload`); `TypeReflectionSnapshot` |
| `crates/shape-vm/src/compiler/comptime_target.rs` | 967 | `ComptimeTarget` descriptor builder (`from_function`, type/module variants): builds the `target` typed object (kind/name/fields/params/return_type/annotations/captures + `type_ref` descriptors) from AST, as stamped v2-raw typed arrays of `__ComptimeFieldDescriptor`/`__ComptimeParamDescriptor` objects |
| `crates/shape-vm/src/compiler/comptime_diagnostics.rs` | 260 | LSDS-routed comptime diagnostics: `C0001` errors / `C0002` warnings, jargon firewall integration, span anchoring at the driving construct |
| `crates/shape-vm/src/compiler/comptime_concrete.rs` | 393 | `ConstantValue` typed comptime carrier — **dead code**: `#[allow(dead_code)]` pending 4d-migration wiring (per `docs/codebase-index/01-compilation.md:544`) |
| `crates/shape-vm/src/compiler/functions_annotations.rs` | 3,310 | Annotation lifecycle compilation: comptime pre/post signature-directive pre-pass; wrapper bytecode emission for `before`/`after` (args-array build, before-return-contract classification, short-circuit jump graph, ctx construction); `on_define`/`metadata` handler-call emission |
| `crates/shape-runtime/src/annotation_context.rs` | 417 | Runtime primitives handed to handlers as `ctx`: `AnnotationRegistry`, `AnnotationContext` (cache/state/named registries/events/data-range) |
| `crates/shape-ast/src/transform/comptime_extends.rs` | 261 | Static pre-pass materializing extends implied by directly-declared comptime handlers (AST-only, no execution) |
| `bin/shape-cli/src/commands/expand_comptime_cmd.rs` | 367 | `shape expand-comptime` report command |
| `crates/shape-runtime/stdlib-src/serde/derive.shape` | 97 | `@json_schema` stdlib annotation (pure Shape) |
| `crates/shape-runtime/stdlib-src/serde/serialize.shape` | 57 | `@to_json` stdlib annotation |
| `crates/shape-runtime/stdlib-src/llm/tools.shape` | 94 | `@llm_tool` + `@prompt` stdlib annotations |

Total in-territory Rust: ~10,700 LOC (+248 LOC pure-Shape stdlib). Supporting surfaces live in `statements.rs` (Item::Comptime at `:1870`, module-comptime handling around `:5608-5676`, annotation registration `:3238-3492`), `expressions/mod.rs` (comptime expression form; `run_comptime_annotation_handlers_for_target` at `:634`; expression/block/binding wrapper emission), `functions.rs` (wrapper-vs-plain compile decision; `emit_annotation_lifecycle_calls` tail call at `:1238`), and the grammar (`shape.pest:145-152` comptime block/for; `:372-429` annotation defs).

### 1.2 Execution model (answering the focus question: "is it a separate interpreter?")

Comptime is **not a separate interpreter**. It is the production compiler and the production VM run recursively at compile time:

1. The outer `BytecodeCompiler` encounters a comptime construct (top-level `Item::Comptime`, a `comptime { }` expression, or an annotation `comptime pre/post` handler at an application site).
2. `execute_comptime_with_context` (`comptime.rs:625`) wraps the statements into a synthetic `fn __comptime_block__()` inside a synthetic `Program`, prepends: builtin forwarders (`comptime.rs:264` — thin Shape functions forwarding to the `__comptime__` extension namespace), comptime trait/struct/impl context items (J-CT.2), and `comptime fn` helpers.
3. A **nested `BytecodeCompiler`** with `set_comptime_mode(true)` and `allow_internal_comptime_namespace = true` compiles it (`comptime.rs:751-761`) — same type checker, same strict-typing rules.
4. A **fresh `VirtualMachine`** executes the result (`execute_in_runtime_with_module_bindings`, `comptime.rs:1287`) inside a tokio runtime (so extension async functions work at comptime), with a 5-second watchdog thread flipping an interrupt flag (`comptime.rs:1331-1337` — empirically verified, §9.11).
5. Directives (`extend`, `set return`, …) and diagnostics (`warning()`, comptime `print()`) do not flow through return values; the `__comptime__` builtins push them onto **thread-local buffers** (`comptime_builtins.rs:170-171` `COMPTIME_DIRECTIVES`/`COMPTIME_DIAGNOSTICS`), drained by the caller after `vm.execute` (`comptime.rs:1345-1346`).
6. The block's **value** returns as a `KindedSlot` and is projected back into AST via `nb_to_expr` (expression form) or `nb_to_literal` (fallback) — this readback boundary is where the P0 lives (§9.1).
7. For annotation handlers, the `target` descriptor is pre-built by `comptime_target.rs` as a real TypedObject and injected as a module binding (`__target_arg__`/`__ctx_arg__`, rebound to the mini-program's schema registry by `rebind_typed_object_bindings_to_bytecode_schemas`, `comptime.rs:806`).

Consequences of this design: comptime code obeys the same strict-typing rules as runtime code (verified: `string + unknown` is rejected inside handlers, §9.6); the comptime environment sees **no runtime locals** (verified §2, scope test) because the synthetic program simply doesn't contain them; and any VM-tier stub (like the deleted TypedArray construction) is inherited by comptime.

### 1.3 Directive statement surface

`shape-ast/src/ast/statements.rs:30-66` defines the comptime-only statements: `Extend`, `RemoveTarget`, `SetParamType`, `SetParamTypeExpr`, `SetParamValue`, `SetReturnType`, `SetReturnExpr`, `ReplaceBody`, `ReplaceBodyExpr`, `ReplaceModuleExpr`, `ExtendItemsExpr`. Expression-payload forms evaluate in the mini-VM and route through `__emit_*` intrinsics into `ComptimeDirective`s (`comptime_builtins.rs:129`); the compiler applies them after handler execution (signature directives in the pre-pass `apply_function_comptime_signature_directives_for_analysis`, `functions_annotations.rs:24`; item generation at the annotation site).

### 1.4 Key types

- `ComptimeExecutionResult { value: KindedSlot, directives, warnings }` (`comptime.rs:77`) — the GENERIC_CARRIER handoff (ADR-006 §2.7.4) from mini-VM to outer compiler.
- `ComptimeDirective` (`comptime_builtins.rs:129`) — enum of emitted directives.
- `ComptimeTarget` (`comptime_target.rs:45`) — the `target` descriptor; serialized to a TypedObject whose schema names+order must match the hand-maintained `TypeAnnotation` mirrors in `comptime.rs:90-262` (split-brain risk, §5.2).
- `TypeReflectionSnapshot` (`comptime_builtins.rs:112`) — outer compiler's type environment snapshot for `type_info`/`implements`.
- `AnnotationDef`/`AnnotationHandler` (shape-ast) + `CompiledAnnotation` (registered at `statements.rs:3238-3492` with per-hook function ids).

### 1.5 Data flow for the flagship pattern

`@json_schema() type User` → parse → annotation registration (handlers compiled as ordinary functions) → at the type's compile site, `run_comptime_annotation_handlers_for_target` builds `ComptimeTarget::from_*`, runs the handler in the mini-VM → handler evaluates f-strings, calls `string_lit` (a `__comptime__` builtin that renders a quoted/escaped Shape literal, `comptime_builtins.rs:1174`) and `extend(payload)` → `__emit_extend_items` parses payload via `parse_extend_items_slot` → `parse_module_items_payload` (probe-wraps in `mod __module_probe__ { … }`, `comptime_builtins.rs:384`) → resulting `Item`s are spliced beside the target and compiled by the **full driver** (`compile_function`) so they carry `mir_data` and JIT natively (the WF-3D root fix, per `flagship_wf3d.rs:22-26` — empirically confirmed: no jit-fallback noise on F1-shaped programs, §2).

### 1.6 Grammar surface (shape.pest, exact working-tree lines)

| Rule | Line | Shape |
|---|---|---|
| `comptime_keyword` | `shape.pest:143` | field-level `comptime` marker inside type bodies |
| `comptime_block` | `:147` | `&comptime_kw ~ "comptime" ~ block_expr` — the expression form |
| `comptime_for_expr` | `:152` | iterable is **`postfix_expr`** — the root cause of the range-literal parse gap (§9.6): `0..3` is not a postfix expr |
| `annotated_expr` | `:156` | `annotation+ ~ postfix_expr` — how `@traced_expr(...) (20 + 22)` attaches |
| `comptime_field_overrides` | `:110-115` | the `type Celsius = Unit { symbol: "°C" }` specialization payload (used at `:101` type-alias and `:1108` `as`-cast positions) — parses, broken at runtime (§9.18) |
| `annotation` (use-site) | `:352` | `@name(args)` attachment |
| `annotation_def` | `:372-374` | `"annotation" ~ ident ~ ("(" ~ annotation_def_params? ~ ")")? ~ "{" ~ annotation_body ~ "}"` |
| `annotation_def_params` | `:376-378` | plain ident list — **no variadic form** at definition level (contrast handler params) |
| `annotation_targets_decl` | `:390-392` | `targets: [ ... ]` |
| `annotation_target_kind` | `:394-402` | the 7 kinds: `function`, `type`, `module`, `expression`, `block`, `await_expr`, `binding` — `binding` has no attachment production anywhere in the grammar (§5.5) |
| `annotation_handler` / `_kind` | `:409-419` | `on_define` / `before` / `after` / `metadata` / `comptime pre\|post` |
| `annotation_handler_param` | `:427-429` | `("..." ~ ident) \| ident` — the undocumented prefix-variadic (§8.2) |

Two structural observations from the grammar itself: (a) the `binding` target kind exists only as validator vocabulary — there is no grammar position for an annotation before `let`, confirming §5.5 statically; (b) handler params allow `...rest` but `annotation_def_params` does not, so an annotation cannot declare a variadic public signature even though its handlers can receive one — an asymmetry no doc mentions.

### 1.7 LSP / tooling surfaces in-territory

- **`tools/shape-lsp/src/annotation_discovery.rs` (233 LOC):** dynamic annotation discovery — local `AnnotationDef` items plus imported modules via `ModuleCache` (`discover_from_imports_with_cache`, `:71-99`). Deliberately zero hardcoded builtins ("Annotations are now fully defined in Shape stdlib, not hardcoded in Rust", `:67-70`) — consistent with the stdlib-dogfood design (§10.6). Its 3 unit tests only pin the *absence* of hardcoded names (`:184-232`); no positive-path test discovers a real annotation from a real module in this file.
- **`tools/shape-lsp/src/completion/annotations.rs` (121 LOC):** `@`-triggered completions with snippet insert (`name(${1})` when the def has params) and hover documentation rendered from the annotation's doc comment (`render_annotation_documentation`). Param names come from the def's identifier list — since annotation-def params carry no types (grammar §1.6), completions can show names but never types.
- **`tools/shape-test/tests/lsp/comptime.rs` (23 LOC, 1 test):** a single end-to-end test that a `comptime post` + `extend target { method sum() ... }` generated method executes (`expect_number(3.0)`) — despite the `lsp/` path this asserts runtime behavior, not LSP behavior; LSP-side comptime awareness (e.g. completions for generated methods, `type_info` hover) has **no test coverage at all**.
- **`bin/shape-cli/src/commands/expand_comptime_cmd.rs` (367 LOC):** covered in §2.12/§5.4; second-pass verbatim transcripts:

```text
$ shape expand-comptime t13d_full_plainfor.shape     # flagship extend (expr) free-fn generation
No comptime expansions found for .../t13d_full_plainfor.shape.

$ shape expand-comptime t10_extend_target.shape      # extend target { method label ... }
Comptime expansion report: .../t10_extend_target.shape
Functions (post-comptime): 1
fn Point.label(self: Point) -> ()
Generated extends: 1
extend Point:
  method label
```

Note the reported signature `fn Point.label(self: Point) -> ()` — the generated method actually returns `string` (t10 prints `Point`); the report shows `()` because it renders the pre-inference AST annotation, not the checked type. A third accuracy gap in the tool beyond the two blind spots of §5.4.
- **`crates/shape-ast/src/transform/comptime_extends.rs` (261 LOC):** a *static* AST pre-pass that clones direct `extend ... { }` directives out of `comptime pre/post` handler bodies into synthetic top-level `Item::Extend`s **without executing anything** (`augment_program_with_generated_extends`, `:17-24`). This is what gives the LSP and other AST-only consumers visibility into directly-declared generated methods. By construction it cannot see computed directives (`extend (expr)`, `item_fn`), which is the mechanistic explanation for the expand-comptime blindness (§5.4): the tool's "Generated extends" section is fed by this static transform, not by the mini-VM (verified: `expand_comptime_cmd.rs:72` calls `shape_ast::transform::collect_generated_annotation_extends(&program)`).

---

## 2. Feature completeness

Legend: **WORKS** = verified end-to-end by running a program against the working-tree binary; **PARTIAL** = works with material caveats; **BROKEN** = code exists, fails empirically; **DORMANT** = intentionally disabled with a surface-and-stop error; **MISSING** = no implementation.

### 2.1 Comptime blocks

| Feature | Status | Evidence |
|---|---|---|
| Expression form `let x = comptime { "dev" }` | **WORKS** | t01: prints `dev` |
| Top-level side-effect form | **WORKS** | t02: `warning[C0002]` at compile, then `runtime ran` |
| Inside function bodies | **WORKS** | t53: `let base = comptime { 40 }` → prints 42 |
| Scope isolation (no runtime locals) | **WORKS** | t17: `Undefined variable: 'marker'` with comptime-trace note |
| Int/float/bool/string results | **WORKS** | t34/t34d: `42`, `true`, `5.0`, `hello world` |
| **Array results** | **BROKEN (P0)** | t34b: prints `33D`, indexing dies — §9.1 |
| **Anonymous object results** | **BROKEN (P1)** | t35: embeds null, runtime "got scalar" — §9.8 |
| TypedObject results (`build_config()`) | **WORKS** | t54: `cfg.debug`→`true`, `cfg.version`→`0.3.2` |
| Multiple blocks, conditionals, arithmetic | **WORKS** | shape-test `blocks.rs` ct_21/ct_22/ct_27 + spot-checks |
| 5-second watchdog | **WORKS** | t47: `[C0001] compile-time execution exceeded the 5-second limit` after ~5s of a `while true` loop |
| `print()` at comptime | **WORKS** | t38: prints during compilation |

t34b transcript (the P0):

```text
$ shape run t34b_arr_direct.shape       # let arr = comptime { [1,2,3] }; print(arr); print(arr[0])
V2 bytecode verification warning: 4 violation(s) found
  - V2 typed opcode NewTypedArrayI64 at offset 98 in function '__main__' has no FrameDescriptor
  ...
33D
Error: Runtime error: Not implemented: SURFACE: GetProp on Ptr(Decimal) not yet kinded — ...
```

### 2.2 Comptime builtins

| Builtin | Status | Evidence |
|---|---|---|
| `warning(msg)` | **WORKS** | C0002 warning, program continues (t02) |
| `error(msg)` | **WORKS** | C0001 halts compilation with span + comptime-trace note (t03); LSDS JSON verified via `--diagnostics json` (§2.7) |
| `implements(Type, Trait)` | **WORKS** | t04: bare identifiers accepted (`implements(Dog, Speak)`→true; string form `implements(Dog, "Ord")`→false). Bare-ident rewriting at any nesting depth (`comptime.rs:390-458`) |
| `build_config()` | **WORKS** | t05/t37/t54: `target_os`→`linux`, `comptime_api`→`1`, `debug`→`true`, `version`→`0.3.2` |
| `type_info(T)` | **PARTIAL** | t06: `.name`/`.kind`/`.fields[i].name/.type` all resolve for entry-file structs. Gaps: enums report `kind=TypedObject`; chained/parametrized lookups `Unresolved` (§9.9) |
| `string_lit(s)` | **WORKS** | flagship t13d; renderer at `comptime_builtins.rs:1174` |
| `item_fn(name, ret, value)` | **WORKS** (narrow) | typed fragment carrier for zero-arg free fns; keyword-name validation at `comptime_builtins.rs:409-459` |

`type_info` field-descriptor rows carry `name`, `type`, `annotations`, `optional`, `type_ref{name,kind,source}` (`comptime.rs:112-170`), matching `target.fields` rows — verified via the stdlib deriver which consumes `field.type_ref.kind` (`derive.shape:43`).

### 2.3 `comptime fn`

| Feature | Status | Evidence |
|---|---|---|
| Definition + call from comptime | **WORKS** | t07: `.trim()` string method works inside |
| Chaining + recursion | **WORKS** | t39: `quad(10)`→40, `fact(6)`→720 |
| Runtime-call rejection | **WORKS** | t18: `'helper' is declared as `comptime fn` and can only be called from comptime contexts` |
| Body skipped in runtime bytecode | **WORKS** (by code read) | `functions.rs:761-763` |

### 2.4 `comptime for`

| Form | Status | Evidence |
|---|---|---|
| Top-level statement `comptime for i in 0..3` | **BROKEN (parse)** | grammar iterable is `postfix_expr` (`shape.pest:152`) — range literal rejected: `unexpected `.`, expected identifier` |
| Top-level statement `comptime for x in [1,2,3]` / `(0..3)` | **DORMANT** | `comptime-for unroll outside a comptime block is dormant pending the phase-2c ComptimeExecutionResult / Literal-projection rebuild (ADR-006 §2.4 / §2.7.4)` |
| Inside annotation handlers, iteration only | **WORKS** | t15: nested `comptime for field in target.fields { comptime for ann in field.annotations { … } }` iterates correctly |
| Inside handlers, loop-var in typed position | **BROKEN (P1)** | t13a/t13b: `props = props + field.name` → `Cannot apply `+` to a `string` and a `unknown`` — even the f-string wrapped form fails. Plain `for` works (t13c) |
| Ordinary `for` inside `comptime { }` | **WORKS** | t28: `for i in 0..3 { warning(...) }` unrolls at compile time |

The stdlib itself avoids `comptime for` — `derive.shape:40`, `serialize.shape`, `tools.shape` all use plain `for` over `target.fields`/`target.params`, while the book's field-annotations example (`comptime.mdx:345-358`) shows `comptime for`. §8 covers the doc mismatch.

### 2.5 Annotation lifecycle — function targets

| Feature | Status | Evidence |
|---|---|---|
| `before(args, ctx)` wrapping | **WORKS** | t09: `[math] before` printed before impl |
| `after(args, result, ctx)` | **WORKS** | t09: result observed (`result=5`), returned |
| before → array = args rewrite | **WORKS** | t20: `[args[0] * 2]` → `show(21)` prints 42 |
| before → `{ result: v }` short-circuit | **WORKS** | t19: prints 99, impl body's `print("impl ran")` never executes |
| Stacked annotation order | **WORKS** | t16: `a before, b before, b after, a after` — matches book |
| `comptime post` + `set return (expr)` | **WORKS** | t12: unannotated fn gets `-> string`, `let g: string = greet(...)` type-checks |
| `set param name = expr` (default value) | **WORKS** | t45: `greet()` (no args) prints `hi` — but triggers a Cranelift **verifier error** JIT deopt (§9.7) |
| `replace body ("return 42")` | **WORKS** | t24: prints 42 |
| `remove target` | **WORKS** | t25 |
| `on_define` / `metadata` | **BROKEN (P1)** | fire only when `before`/`after` also present (t29b vs t29c/t29d) — §9.4 |
| Variadic handler params `(target, ctx, first, ...rest)` | **WORKS** | t46b: `first=a rest_len=2`. NOTE: definition-params can't be variadic (`shape.pest:376-378`), and use-site arg count is not validated against def params (3 args passed to `multi(first)` accepted) |
| const-param specialization | **WORKS** | t49: handler runs per distinct const callsite as `query__const_0`, `query__const_1`; base template gets no handler run when never called (t48 vs t48b) |
| `target.params` / `target.return_type` / `p.const` | **WORKS** | t48b: `param n: int const=true`, `ret=string` |

### 2.6 Annotation lifecycle — type / module / expression / block / binding / await targets

| Target kind | Status | Evidence |
|---|---|---|
| `type` + `extend target { method … }` | **WORKS** | t10: `p.label()` → `Point` |
| `type` + `extend (expr)` free-fn generation | **WORKS** (entry file only) | t13d flagship, t33 (called from another fn), F1/F4 gates |
| `type` + `on_define` | **WORKS** | t29e (with V2-verifier deopt noise) |
| `module` + `replace module (expr)` | **WORKS** | t11: replaced `charge` returns `true` |
| `module` + `on_define` | **BROKEN (silent)** | t29f: nothing fires |
| `expression` | **BROKEN (runtime)** | t27: `op_new_array(0)` SURFACE stub — §9.5 |
| `block` | **BROKEN (runtime)** | t32: same stub |
| `await_expr` | **BROKEN (runtime)** | t50: same stub |
| `binding` | **MISSING (parse)** | t30/t30b: `@logged() let x = …` fails to parse at top level and in fn bodies, despite `binding` being a validated target kind (`shape.pest:401`, book table) |
| Target validation | **WORKS** | t26: `Annotation 'fn_only' cannot be applied to a type. Allowed targets: function` |
| Definition-hook target restriction | **WORKS** (by code read) | `compiler_impl_reference_model.rs:1720-1730`, `statements.rs:3450-3492` |

### 2.7 Diagnostics pipeline

| Feature | Status | Evidence |
|---|---|---|
| LSDS `C0001`/`C0002` ids | **WORKS** | all transcripts |
| `--diagnostics json` | **WORKS** | `{"diagnostic_id":"C0001","severity":"error","location":{"line":1,...},"message":"compile-time hard failure","notes":[{"message":"during compile-time evaluation of a compile-time block"}]}` |
| Comptime-trace note | **WORKS** | "during compile-time evaluation of the @probe annotation handler" etc. |
| Jargon firewall | **WORKS** (comptime channel) | t47 watchdog message is the clean sentence; unit tests pin P10 fragments (`comptime_diagnostics.rs:136-203`). **But** `[jit-fallback]` stderr noise leaks the same jargon un-firewalled (§9.7) |
| Span fidelity | **PARTIAL** | error spans anchor at the driving construct (good); warnings report file **`<synthetic>`** and handler-relative lines (§9.10) |

### 2.8 Stdlib LLM/derive patterns (the book's flagship claim)

All four verified byte-identical to the book's documented output:

```text
$ shape run t40_json_schema_stdlib.shape
{"type": "object", "title": "User", "properties": {"id": {"type": "integer", "description": "Unique identifier"}, "name": {"type": "string"}, "email": {"type": "string"}}, "required": ["id", "name"]}

$ shape run t41_to_json_stdlib.shape
{ "id": 1, "name": "Ada" }

$ shape run t42_llm_tool.shape
{"name": "get_weather", "description": "Get current weather for a city", "parameters": {"type": "object", "properties": {"city": {"type": "string"}, "units": {"type": "string"}}, "required": ["city", "units"]}}

$ shape run t43_prompt_ok.shape
ok
$ shape run t44_prompt_typo.shape
error[RUNTIME]: ... [C0001] @prompt: placeholder {audence} has no matching parameter on 'weather_prompt'
```

These are pure-Shape annotations on the public comptime contract (`derive.shape`, `serialize.shape`, `tools.shape`) — the "user-defined LLM integration patterns in stdlib/userland" claim from CLAUDE.md is **real**, with the module-boundary caveat (§9.2/§9.3).

### 2.9 Generated-function visibility

| Case | Status | Evidence |
|---|---|---|
| Called from top level | **WORKS** | t13d, F1 gate |
| Called from another function | **WORKS** | t33: `fn caller() { User_label() }` |
| Called from `main()` per book | **WORKS** | t40/t41 (with a benign `[jit-fallback]` on `main` return-kind proof) |
| Generated **method** dispatch | **WORKS** | t41 `u.to_json()`; F4 gate `w.label()` |
| Target in same-file `mod` | **BROKEN** | t51 — §9.3 |
| Target in imported module | **BROKEN** | t31b-f — §9.2 |
| Name collision with user fn | **SILENT last-wins** | t52 prints 2 (generated wins) — but t52b shows plain duplicate user fns are also silent last-wins, so this is a general language gap the codegen surface inherits |

### 2.10 Comptime in imported modules

Import syntax is `use module` / `from module use { item }` (no path imports — `import "./x.shape"` is a parse error, consistent with the known relative-import gap). Findings:

- Annotation **imports** work: `from std::serde::derive use { @json_schema }` resolves (t40); the four-step resolution order in `compiler_impl_reference_model.rs:926-968` matches the book.
- Annotation handlers **do run** for imported-module targets (the error probe fired — §9.2), and comptime output is suppressed during import pre-resolution (`COMPTIME_OUTPUT_SUPPRESSED`, `comptime_builtins.rs:182`): the `warning()` in gen3 never surfaced on import (t31e), which also means **imported-module comptime warnings are silently dropped** rather than re-emitted at the import site.
- But name-splicing codegen breaks per findings 2 and 3.

### 2.11 Comptime fields on types

`type Unit { comptime symbol: string = "m" }` parses and compiles (t14); shape-test `blocks.rs` covers instance/typed/inline variants (ct_40 family). The book's follow-on form `type Celsius = Unit { symbol: "°C" }` is grammar-supported (`shape.pest:101,110-115`) — exercised in the second pass and **BROKEN end-to-end** (field lost at runtime); base-type field reads work only via an interpreter deopt. Full analysis §2.18/§9.18.

### 2.12 `shape expand-comptime`

Works for `set return` (shows transformed signature) and `extend target` (lists generated methods); reports **"No comptime expansions found"** for `extend (expr)` free functions and `replace module` — precisely the two flagship codegen forms (§9.10). `--expand` shorthand and `--module`/`--function` filters exist (`cli_args`).

### 2.13 Runtime `ctx` contract (function targets)

The working-tree ctx schema is `{ target: Function, state: {}, event_log: [] }` — the field is **`ctx.target`**, not the book's `ctx.__impl` (renamed per §4.1.5/OQ-12; emission comment at `functions_annotations.rs:2871-2878`, consumer `remote.shape:183-189`).

| Contract element | Status | Evidence |
|---|---|---|
| `ctx.target` = callable original impl | **WORKS** | t57c: `before` calls `ctx.target(args[0])`, short-circuits `{result: r+100}` → `base(5)` prints 110 |
| `ctx.__impl` (book name) | **GONE** | t57b: `Undefined property: __impl (line 4)` — doc drift (§9.14) |
| `ctx.state` initial `{}` / `ctx.event_log` initial `[]` | **WORKS** | t58c: `after state={} log=[]` |
| `before` returns `{args, state: …}` → rebuilt ctx | **BROKEN (P1)** | t58b: `after state=[] log=None` — one-field offset shift, new state lost (§9.12) |
| Annotation on `async fn` | **WORKS** | t65: `[aio] before` then `42` |
| `@remote` named-import resolution | **WORKS** | t66: compiles + prints (resolution only, no server — same scope as the book's gated example) |

### 2.14 Additional directive forms (all verified)

| Form | Status | Evidence |
|---|---|---|
| `set param name: Type` (type concretization on unannotated param) | **WORKS** | t60: `@concretize() fn open_it(path)` + `set param path: string` + `set return ("string")` → runs, prints `db.sqlite` — the connector-pattern core |
| `replace body { … }` (inline statement block) | **WORKS** | t62: prints 77 |
| `extend TypeName { … }` (named type, not `target`) | **WORKS** | t61: `v.sum()` → 7 |
| `comptime pre` + `comptime post` both fire, in order | **WORKS** | t59: `PRE ran` then `POST ran` |
| `error()` in `comptime pre` field validation (serde-derive guard from the book) | **WORKS** | t64: `[C0001] field 'score' must not be number` anchored at `@checked()` application site |

### 2.15 Comptime traits/impls (J-CT.2)

**BROKEN in every configuration** (§9.13): plain trait+impl methods are invisible inside comptime blocks; `comptime trait` + `comptime impl` trips a self-inflicted "comptime alignment mismatch"; mixed forms fail the same validator legitimately. The J-CT.2 machinery (`comptime.rs:613-694`) exists but cannot currently be exercised by any user program I could construct.

### 2.16 Second-wave probes (n-series)

| Feature | Status | Evidence |
|---|---|---|
| Annotation-definition arguments (`annotation tagged(label)` + `@tagged("hot")` → handler reads `label`) | **WORKS** | n02: prints `label=hot` before the wrapped call |
| Annotation argument referencing the annotated fn's own parameter (`@warm(p + 1)` on `fn g(series, p)`) | **WORKS** for `before` hooks | n03: `g(1, 2)` prints `warm=3` — the arg expression evaluates at call time in the wrapper scope where `p` is bound. **This contradicts the stdlib's own doc** in `warmup.shape:18-21`, which claims "any lifecycle handler causes the annotation argument to be compiled at module/definition scope … producing 'Undefined variable: period'" — either that limitation was since fixed for runtime hooks (and the doc plus the handler-stripping of `@warmup` are stale), or it still holds only for `on_define`/`metadata` (definition-scope hooks). Both readings mean `warmup.shape`'s stated rationale no longer matches observable behavior |
| `implements()` with a genuine trait/impl pair inside `comptime { }` | **WORKS** | n09b: `implements(A, Ord2)` → true branch, `implements(A, "Missing")` → false branch; both warnings filed under `<synthetic>:4:1` (re-confirming §9.10) |
| Nested comptime blocks | **BROKEN (type)** | n06 — §9.19 |
| Comptime block calling a plain (non-comptime) same-file fn | **REJECTED** (by-design isolation, weak diagnostic) | n07: `Undefined function: 'plain'` — correct semantics per §1.2, but the message doesn't say *why* (a "runtime functions are not visible at comptime; declare it `comptime fn`" note would); contrast the excellent inverse message (t18) |
| `metadata()` handler alongside `before` | **WORKS** (fires, result unobservable) | n10: runs clean; the metadata object's only consumer is the handler-call emission at `functions_annotations.rs:568-570` — there is **no user-facing query surface** (no `annotations_of()`/reflection builtin), so metadata is write-only today |
| Annotated `method` in an `extend` block | **BROKEN under default JIT** | n08e — §9.17 |
| Inherent `impl Point { }` (no trait) | **parse error** (out-of-territory context) | n08c: `unexpected identifier 'impl'` — methods on types go through `extend Type { }`; noted because every annotation-on-method user will trip it first |

### 2.17 The other stdlib annotation consumers: finance (`@indicator`, `@warmup`)

The four LLM/derive patterns (§2.8) are not the only shipped annotations. `stdlib-src/finance/annotations/` holds two more, and they tell a very different story:

- **`@indicator` (`indicator.shape`, 63 LOC) is a broken fossil.** Its handlers are written against a contract that no longer exists: `self.name` (`:33,:49` — no `self` binding exists in the current `(args, ctx)` handler signature), `ctx.get("registry")` / `ctx.get("cache")` (`:26,:36` — the current ctx is `{target, state, event_log}`, §2.13; no `.get`), `on_define(ctx)` with 1 param (current contract is `(target, ctx)`). Applying it fails to compile (§9.16). Its doc comment still advertises "registers the function in the indicator registry and enables memoization" — none of which can execute.
- **`@warmup` (`warmup.shape`, 30 LOC) is a deliberate empty shell** — `pub annotation warmup(period) { }` with a 22-line doc comment explaining *why* the handlers were removed (no-op hooks + the definition-scope argument-binding problem). It is used pervasively: **29 application sites** across the finance stdlib (grep over `stdlib-src`, imports and the definition excluded), including arg expressions over fn params (`@warmup(period + 1)` `atr.shape:12`, `@warmup(period * 3)` `trend.shape:22`) — all evaluated never, by design. `@indicator`, by contrast, has **zero** stdlib application sites (its only stdlib mention is a `@see` cross-reference in `warmup.shape:23`) — consistent with it being un-compilable. So the *only* production-scale annotation consumer in the codebase uses annotations purely as inert markers.
- The application syntax used there is also grammatically notable: `pub @warmup(1) fn obv(...)` (`volume.shape:10`) — annotation *between* `pub` and `fn` on one line, a form no book chapter shows.

Net: outside the four demo-grade LLM/derive patterns, the stdlib's own experience with runtime-hook annotations was bad enough that one annotation was stripped to a marker and the other was left to rot un-compilable. That is a stronger signal about the runtime-hook contract's real-world usability than any of my synthetic probes.

### 2.18 Comptime-field specialization (`type Celsius = Unit { symbol: "°C" }`)

Follow-up to §2.11's deferred item — now exercised (§9.18): the **base** type's comptime field is readable at runtime (`Unit {}` → `u.symbol` prints `m`, with a JIT deopt: `MirToIR: unresolved direct field read .symbol … deopt to the bytecode interpreter`), but the **specialized alias** loses the field entirely: `Celsius {}` → `c.symbol` → `Undefined property: symbol (line 4)`. Also note the base-type behavior contradicts the book's own semantics ("excluded from runtime object storage/layout", `comptime.mdx` §Comptime Fields) — the value *is* reachable through the interpreter's dynamic property path rather than being compile-time-folded, which is why the JIT (which refuses the unproven field read) had to deopt.

---

## 3. Code quality

### 3.1 Idiom & naming

Positive: the core files are heavily and honestly documented — nearly every nontrivial decision carries a comment naming the design source (comptime-excellence §-refs, ADR-006 §-refs, wave/cluster provenance). Naming is consistent (`execute_comptime_*`, `nb_to_*` readback family, `parse_*_payload` family, `__comptime__` namespace, `__ComptimeTypeInfo`/`__ComptimeFieldDescriptor`/`__ComptimeTypeRef` reserved schemas). Errors are `Result`-routed, no panics on user input observed in ~55 runs.

Negative: legacy naming residue — the `nb_*` prefix ("nanboxed") survives on ~10 functions (`nb_to_literal`, `nb_to_expr`, `nb_str`, `nb_string`, `nb_string_array`) three refactors after nan-boxing was deleted; `comptime.rs:655-661` even documents keeping `nb_str` "so existing callers … don't need to be re-touched". `vmvalue_to_literal` (`comptime.rs:1378`) is a one-line alias for `nb_to_literal` kept for the same reason — two names for one function.

### 3.2 Error handling

- User-facing comptime failures are consistently upgraded to LSDS diagnostics with the driving construct's span and a comptime-trace note (`comptime_diagnostics.rs:52-89`) — the best error-handling discipline I found anywhere in this vertical.
- The jargon firewall (`clean_comptime_message` / `sanitize_comptime_internal`) is tested against an explicit P10 forbidden-fragment list (`comptime_diagnostics.rs:136-163`) including `ckpt`, `ADR-`, `§`, `REFUSED` — and it demonstrably works on the comptime channel (watchdog message, t47).
- **Silent-drop hot spots**: (a) `emit_annotation_lifecycle_calls_for_target` silently `continue`s when `lookup_compiled_annotation` misses (`functions_annotations.rs:555-557`) — implicated in the on_define no-op (§9.4); (b) unknown module-binding names are silently dropped at `comptime.rs:1321-1324`; (c) imported-module comptime warnings are suppressed and never re-emitted (§2.10); (d) anonymous-object comptime results degrade to `Literal::None` without a diagnostic (§9.8).

### 3.3 Unsafe usage

Counts (grep `unsafe`, working tree): `comptime.rs` 4, `comptime_builtins.rs` 4, `comptime_target.rs` 9, `functions_annotations.rs` 0, `comptime_diagnostics.rs` 0, `comptime_concrete.rs` 0. Character:

- `comptime_target.rs` (9): v2-raw `TypedArray`/`StringObj`/`TypedObjectStorage` construction for the target descriptor (`:125-160`, `:390-422`, `:771`). Justified by the v2-raw carrier design; invariants stated inline.
- `comptime.rs` (4): typed-pointer recovery in readback (`:1537` TypedObject deref with a stated SAFETY contract; `read_typed_object_field` share-bumping).
- `comptime_builtins.rs` (4): same class (`:306`, `:398-407` `heap_value_from_typed_object_slot` with explicit retain).

6 `SAFETY:` comments across the 17 sites — under-annotated relative to the project's own standard, though most uncommented sites are adjacent repetitions of a commented pattern. **The one unjustified-in-effect site is not an `unsafe` block in this vertical at all** but the safe-looking `slot_for_hv.as_heap_value()` fallthrough (`comptime.rs:1439`, `:1551`), which encapsulates `&*(bits as *const HeapValue)` (`shape-value/src/slot.rs:405-408`) and is reached with v2-raw non-`Arc<HeapValue>` bits — the P0 (§9.1). The file's own comments prove the authors knew this class (`comptime.rs:1418-1428` refuses exactly this for TypedObject) yet the array kinds still fall through.

### 3.4 Complexity hotspots

Longest functions (awk over fn starts):

| Function | LOC | File |
|---|---|---|
| `compile_annotation_wrapper` | 509 | `functions_annotations.rs` |
| `create_comptime_builtins_module` | 499 | `comptime_builtins.rs` |
| `collect_scoped_names_in_expr` | 354 | `functions_annotations.rs` |
| `execute_comptime_with_annotation_handler` | 229 | `comptime.rs` |
| `materialize_computed_comptime_extends` | 217 | `functions_annotations.rs` |
| `rebind_typed_object_bindings_to_bytecode_schemas` | 166 | `comptime.rs` |

`compile_annotation_wrapper` (509 LOC) hand-emits the full wrapper bytecode graph — args array, ctx object, before-contract classification (IsArray/IsObject branches), short-circuit jumps, after-call, void-return special case. It is the least-testable code in the vertical (0 unit tests in the file) and the site of the book's cited line-number anchors, several of which have drifted (§8.3). `create_comptime_builtins_module` (499 LOC) is a flat registration list — long but low-risk.

### 3.5 Dead code in-territory

- `comptime_concrete.rs` — entire 393-LOC module `#![allow(dead_code)]` (`:77`): the "ConstantValue typed comptime carrier" for a 4d migration that never wired in. It contains 16 unit tests exercising code nothing calls. Either wire or delete; today it is pure maintenance surface.
- `comptime.rs:591` `#[allow(dead_code)]` on `execute_comptime` (superseded by `execute_comptime_with_context`; kept as a thin forwarder) and `:1003` on `execute_comptime_with_target`.
- `vmvalue_to_literal` alias (§3.1).

### 3.6 Robustness observations from testing

- The compiler never crashed across ~55 adversarial programs — failures are diagnostics or (at worst) runtime SURFACE stubs. No SIGSEGV observed even on the P0 misread (the garbage stays inside Decimal formatting).
- Two identical "V2 bytecode verification warning" blocks print for one program (t13c) — the verifier runs (and prints) twice per compile.
- A failed module compile (t51) did not stop execution: the error printed, then the program continued and failed later with "Unknown qualified call" — error-recovery leaks a half-compiled program into execution.

---

## 4. Duplication & DRY violations

### 4.1 The before-return contract is implemented twice (verbatim, by design-note)

The runtime `before` return classification (array→args / object→{args,result,state} / other→ignore) is emitted in two places:

- `apply_before_result_contract` / `_with_short_circuit` / `_inner` at `expressions/mod.rs:700-784` (used by expression/block/await wrappers), and
- inline in `compile_annotation_wrapper` at `functions_annotations.rs:2972-3001` (IsArray/IsObject builtin emissions).

The book itself states the function-target copy is "the verbatim same shape inline" (`annotations.mdx:249-252`). Divergence is **dangerous** here: this is the semantic contract of every annotation; if one copy adds a case (e.g. a new `state` rebuild rule), function targets and expression targets silently disagree. Today they already effectively diverge in outcome, since the expression-side copy feeds an args-array built by `op_new_array` (dead stub, §9.5), so only one copy is actually exercisable — a latent trap for whoever revives the expression path.

### 4.2 Two type classifiers with different answers

- `classify_bare_type_name` (`comptime_builtins.rs:1275-1305`) — registry-driven (snapshot struct/enum/alias/type-param lookups), no parametrized types: `type_info("Option<int>")` → `Unresolved` (verified, t36).
- `type_ref_kind_from_source` (`comptime_target.rs:56-87`) — string-prefix heuristics (`Array<`, `HashMap<`, `Result<`, `Option<`, `=>`, `dyn `, leading-uppercase→`TypedObject`), no registry: would classify `Option<int>` as `Option`.

Same conceptual function ("what kind is this type name"), two implementations, incompatible results depending on whether you ask `type_info(...)` or read `field.type_ref.kind`. The stdlib deriver consumes the *second* (`derive.shape:43`); a user following the book's `type_info` docs gets the *first*.

### 4.3 Scalar readback dispatch duplicated between `nb_to_literal` and `nb_to_expr`

`comptime.rs:1393-1430` and `:1471-1541` repeat the same Int64/Float64/Bool/String/Char arms — including a byte-identical 8-line comment about the Bool/None sentinel pasted in both (`:1397-1404` and `:1485-1490`). The functions have already drifted once in a way that matters: `nb_to_expr` grew the safe TypedObject typed-pointer arm (`:1527-1539`), while `nb_to_literal` returns `Literal::None` for TypedObject (`:1426-1428`) — deliberate, but the pattern of "fix one, remember the other" is exactly how the array-kind hole (§9.1) survives in **both** copies' `as_heap_value()` fallthrough today.

### 4.4 Descriptor schema defined four times

The `target`/field/param descriptor shape exists as:

1. Reserved runtime schemas: `__ComptimeTarget`, `__ComptimeFieldDescriptor`, `__ComptimeParamDescriptor`, `__ComptimeTypeRef`, `__ComptimeBuildConfig` (`builtin_schemas.rs:233-291`);
2. Hand-built `TypeAnnotation` mirrors for the comptime type-checker: `comptime_target_param_type`, `comptime_field_descriptor_annotation`, `comptime_param_descriptor_annotation`, `comptime_ctx_param_type` (`comptime.rs:90-262`);
3. The imperative constructors in `comptime_target.rs` (field push order);
4. The book's field tables (`comptime.mdx:138-147`).

The code admits the coupling is order-sensitive: "Field NAMES + ORDER match the `__ComptimeContext` reserved schema … so typed field access … resolves the right offsets" (`comptime.rs:87-89`). A previous real bug of exactly this class is memorialized at `comptime_builtins.rs:874-880` (anonymous `{kind,name}` schema registering fields in reverse order → swapped-offset reads). Nothing mechanical enforces the sync today.

### 4.5 Forwarder return-field hints duplicate builtin schemas

`COMPTIME_BUILTIN_FORWARDERS` hardcodes `build_config`'s and `type_info`'s return-field lists (`comptime.rs:33-53`) with a "must stay in sync with the `__ComptimeBuildConfig` schema (builtin_schemas.rs)" comment (`:29-32`). Hand-sync again; a field added to the schema but not the hint silently loses typed field access in comptime blocks.

### 4.6 Bare-identifier rewriting duplicated against the outer checker

`rewrite_comptime_type_symbol_args` (`comptime.rs:390-458`, a 68-line statement walker + 132-line expression walker) must mirror the outer type-checker's acceptance of bare type identifiers (`inference/access.rs` `type_symbol_ident_args`, per the comment at `comptime.rs:387-389` — "the two paths agree"). Any new statement variant must be added to both walkers; the walker already had to enumerate all 20+ Statement variants by hand.

---

## 5. Split-brain analysis

### 5.1 VM vs JIT on annotated functions (active divergence, currently masked by deopt)

Annotation wrappers intentionally ship **no `mir_data`** (`functions.rs:1128-1137`, "WF-1A Item 3 (anno-jit-parity): skip mir_data for annotation-wrapper functions … Shipping the unwrapped MIR" would let the JIT bypass the hooks). Under the default `--mode jit`, that makes every program with a runtime-hook annotation take a **whole-program deopt** with a loud stderr banner (§9.7). Worse, `set param` (comptime default value) produces a bytecode/JIT **arity split-brain** that reaches the Cranelift verifier:

```text
[jit-fallback] function main failed JIT compile: ... Failed to define function (strategy):
Compilation(Verifier(VerifierErrors([VerifierError { ... message: "mismatched argument count
for `v4 = call fn21(v0)`: got 1, expected 2" }]))); running under interpreter
```

The bytecode side applied the comptime-injected default (call with 1 arg), the JIT side compiled the callee with 2 params. Correctness survives only because the deopt is whole-program. The moment someone makes the deopt function-granular, `greet()` returns garbage under JIT. This is the highest-risk split-brain in the vertical.

The flagship generated **free functions** are deliberately NOT split-brained anymore — WF-3D's root fix compiles them through the full driver so they JIT natively (`flagship_wf3d.rs:22-26`), and the VM==JIT parity gates (F1/F2/F4 `_vm`/`_jit` test pairs) pin it. Verified: t13d/t42 run without jit-fallback noise.

### 5.2 Comptime mini-VM type environment vs outer compiler

The mini-VM compiles handler bodies against the hand-mirrored `TypeAnnotation` schemas (§4.4-2) rather than the outer compiler's actual types. Observable drift today: `comptime for` loop variables lose the `FieldDescriptor` element type entirely (typed `unknown`, §9.6) while plain `for` over the same array is fully typed — two loop forms over one value with different static types. Also `target.annotations`/`target.captures` are declared `Array<unknown>` (`comptime.rs:245-260`), so any typed use of those fields hits the same `unknown` wall.

### 5.3 Book vs code (multiple, in both directions)

- `comptime.mdx:113-120` carries a **v0.4-preview caution** claiming "*applying* a hook to a real target — emitting directives such as set return, set param, replace body, or extend and having them take effect — is planned for v0.4 and not available in v0.3.3". **Empirically false on the working tree**: all five directive families apply (t10-t12, t24-t25, t45). The same page then documents `extend (expr)` as working, with gated snippets. The caution box is stale post-WF-3D and now actively understates the product.
- `comptime.mdx:345-358` demonstrates `comptime for field in target.fields` — the form whose loop variable is untyped (§9.6); the stdlib's own derivers use plain `for`. A user copying the book's loop and doing anything typed with `field` gets a confusing strict-typing error.
- `annotations.mdx` documents `binding` as an applicable target kind (table at :158-163); no grammar rule allows an annotation before `let` (t30/t30b parse errors).
- `annotations.mdx:322` "Variadic final parameter is supported" — true, but the syntax (`...rest`, prefix) is never shown; the natural guess `rest...` is a parse error.
- Line-number anchors in the book have drifted: e.g. `validate_annotation_target_usage` is cited at `compiler_impl_reference_model.rs:1015-1062` but the definition-hook restriction now sits at `:1720-1730`; `on_define` emission cited at `functions_annotations.rs:111-194` now begins at `:582`. The book's practice of citing volatile line numbers guarantees this class of rot.

### 5.4 `expand-comptime` vs the real comptime engine

The CLI report re-derives "what comptime did" through its own lens (`expand_comptime_cmd.rs`, 367 LOC) and disagrees with the engine: it surfaces `set return` signature changes and `extend target` methods, but reports "No comptime expansions found" for `extend (expr)`-generated free functions and `replace module` results (§9.10). Two sources of truth for "the expanded program", one of them blind to the flagship surface the book explicitly tells users to inspect with this command (`comptime.mdx:364-372`).

### 5.5 Grammar vs validator target kinds

`AnnotationTargetKind` has seven variants and the use-site validator enforces them (t26 works), but the grammar can only *attach* annotations to items, expressions, blocks, and awaits — not bindings (`shape.pest` has no annotation slot on `let`). One enum, two disagreeing surfaces (parse-time reachability vs validate-time vocabulary).

### 5.6 Suppressed-diagnostics flag vs import pre-resolution

`COMPTIME_OUTPUT_SUPPRESSED` (`comptime_builtins.rs:182-195`) suppresses handler print/warning output during module import pre-resolution. The suppression is a process-wide thread-local toggled by compile phase — a second "who is compiling right now" state that must agree with the module loader's actual phase. Empirically it currently over-suppresses: legitimate `warning()` diagnostics from imported-module handlers vanish entirely (t31e — the probe warning never printed even though the handler executed and its `extend` failed).

### 5.7 Two "annotation context" implementations — one real (bytecode), one dead (Rust), documenting different contracts

Second-pass discovery. The runtime `ctx` handed to handlers is built **in bytecode** by the wrapper emitter as a 3-field `{target, state, event_log}` object (`functions_annotations.rs:2871-2895`, §2.13). But `crates/shape-runtime/src/annotation_context.rs` (417 LOC) implements a *parallel Rust-side* `AnnotationContext` with a much richer advertised API — `cache` (memoization), `state`, `registry(name)` named registries, `emit(event, data)`/`events()`, `data_range` — reachable via `ExecutionContext::annotation_context()` (`context/registries.rs:23-29`). Empirical consumer census: **the only non-self references in the entire workspace are one unit test** (`registries.rs:324-337`); no VM handler, builtin, or wrapper reads or writes it. The wiring that would connect it is an explicit stub: `execute_on_define_handler` (`registries.rs:66-73`, "Currently a stub until VM-based closure handling is implemented") calls `sync_pattern_registry_from_annotation_context`, whose body is `{}` (`registries.rs:80`).

Worse, the dead module's rustdoc (`annotation_context.rs:11-36`) documents the **legacy handler contract** — `on_define(fn, ctx)`, `before(fn, args, ctx)`, `ctx.registry("patterns")` — and cites an example file `stdlib/finance/annotations/pattern.shape` that **does not exist** in the tree (find returns nothing). This is the same dead contract `@indicator` (§9.16) was written against, which closes the loop on how that fossil happened: the stdlib annotation and the Rust module document each other's era, and neither matches the shipped bytecode contract. Anyone (human or LLM) reading `annotation_context.rs` as the authority on "what ctx offers" will write a second `@indicator`. (`AnnotationRegistry` in the same file *is* live — registered from `context/mod.rs` and `lib.rs:164,237` — so the module cannot simply be deleted wholesale; the dead half is `AnnotationContext` + `AnnotationCache` + `NamedRegistry` + `EmittedEvent` + `DataRangeState`.)

---

## 6. ADR & spec conformance

42 `ADR-005`/`ADR-006` marker comments across the four core comptime files — the marker discipline is followed. Rule-by-rule:

### 6.1 ADR-006 §2.7 / Q7 — KindedSlot as the GENERIC_CARRIER for runtime values of statically-unknown kind

**CONFORMS.** `ComptimeExecutionResult.value: KindedSlot` (`comptime.rs:77-85`) with an explicit §2.7/Q7 citation; comptime is a canonical GENERIC_CARRIER site (mini-VM returns arbitrary values to the outer compiler). Module-binding injection transfers shares explicitly (`comptime.rs:1306-1326`, `mem::forget` + `module_binding_write_kinded`).

### 6.2 ADR-006 §2.7.6 / Q8 — KindedSlot API bounded; heap dispatch via `slot.as_heap_value()` + HeapValue match; no per-heap-variant accessors

**CONFORMS with one blessed exception, one violation-in-effect.** Scalar accessors used are the bounded set (`as_i64`/`as_f64`/`as_bool`/`as_str`/`as_char`). `as_typed_object_storage()` (`kinded_slot.rs:762`, used at `comptime_builtins.rs:632`) is a per-heap-variant accessor, but it exists because `as_heap_value()` is *unsound* on v2-raw TypedObject bits — the §2.7.16 receiver-recovery amendment blesses direct typed-pointer recovery for these carriers (documented at `comptime.rs:1511-1526`). The violation-in-effect: the Q8-canonical `as_heap_value()` dispatch in `nb_to_expr`/`nb_to_literal` (`comptime.rs:1439`, `:1551`) is reached by v2-raw **array** slots for which the same soundness argument applies and no arm exists — producing the P0 misread (§9.1). Q8's dispatch idiom is being applied to bits it is no longer valid for; the ADR's own receiver-recovery rule (§2.7.16) is violated on this path.

### 6.3 ADR-006 §2.7.5 — kinds stamped at compile time, never fabricated from raw bits

**CONFORMS in code, violated in outcome.** No comptime code fabricates a `NativeKind` from raw bits. But the readback fallthrough *interprets* bits under a wrong assumption (Arc<HeapValue> layout), which is the same failure class the rule exists to prevent. The `unwrap_or(0)`/`unwrap_or(false)` scalar-accessor defaults in `nb_to_literal` (`comptime.rs:1394-1404`) are benign (kind already proven) but are silent-default shaped; a debug assert would be more in keeping with §2.7.8's surface-and-stop discipline.

### 6.4 ADR-006 §2.4 (per-FieldType constructors) + Q6 (no `from_heap_arc` catch-all)

**CONFORMS.** No `from_heap_arc` in the compiler tree (grep clean). Construction goes through named constructors: `KindedSlot::from_string_arc` (`comptime.rs:663`), `from_bool`, `from_temporal`/`from_instant` (the Q23 amendment explicitly migrated `comptime.rs::typed_array_element_kinded`'s inline constructions to these — ADR-006 line 6150), `typed_object_for_named_schema` for descriptor objects.

### 6.5 ADR-005 §1 — single discriminator; no parallel HeapKind-projecting sum types

**CONFORMS.** `ComptimeDirective` (`comptime_builtins.rs:129`) discriminates *directives*, not heap shapes. The dead `comptime_concrete.rs` `ConstantValue` is a compiler-tier constant model (Int/Float/Bool/String/…), which is the same shape as `Literal` — borderline parallel-model but never wired (dead code), and the ADR-006 Q-log already routes it to the phase-2c rebuild.

### 6.6 CLAUDE.md Forbidden Patterns (ValueWord, dynamic fallback, generic opcodes)

**CONFORMS — no live occurrences.** All `ValueWord` mentions in territory are doc comments describing deleted code by name/deletion-fate, except `comptime.rs:2985-3120` `mod tests_deferred` which references `ValueWord::from_i64` in real test code — but the module is `#[cfg(any())]`-gated (never compiles). It can never compile again (the type is gone); it is a fossil, not a shim (see §11). No dynamic-fallback handlers, no `SlotKind::Dynamic`, no `Convert*To*` additions in territory. The dormant `comptime for` and the V3-S5 array stubs are proper surface-and-stop errors, not silent fallbacks — the discipline held here.

### 6.7 ADR-006 §2.7.8 / Q10 — no Bool-default for cell reads; surface-and-stop

**CONFORMS**, with the regression test for the closest historical violation present and passing by inspection: the 2026-06-21 `comptime { false }` → `null` bug (Bool-kinded zero conflated with none sentinel) is pinned by `comptime_false_bool_materializes_as_false_not_null` (`comptime.rs:1799-1818`) and documented at both dispatch copies.

### 6.8 Runtime-v2 spec — typed opcodes require compile-time proof

**PARTIAL.** The comptime mini-VM emits typed opcodes (`NewTypedArrayI64`, `TypedArrayPushI64`, `StringConcatTyped`) for handler bodies, but the resulting `__main__` mini-program repeatedly **fails V2 bytecode verification** ("has no FrameDescriptor", t13c/t34/t46b) — printed twice per compile. The verifier violations are warnings, not errors, so the mini-VM runs unverified typed opcodes. Additionally `emit_annotation_lifecycle_calls`-generated `on_define` handler bytecode fails verification at the *outer* program level (t29e: `NewTypedArrayString … has no FrameDescriptor. R8 W7 G.5 SURFACE (ADR-006 §2.7.14) — JIT refuses unverified V2 typed opcodes`), forcing interpreter fallback. Spec conformance gap: comptime-generated bytecode does not meet the FrameDescriptor contract the JIT requires.

### 6.9 ADR-006 §9 / LSDS — LSDS is the primary diagnostic format

**CONFORMS exemplarily** for the comptime channel (`comptime_diagnostics.rs` — LSDS-first, terminal derived, JSON renderer verified end-to-end). **Non-conformant neighbors leak into the comptime UX**: `[jit-fallback]` and "V2 bytecode verification warning" messages triggered by comptime/annotation artifacts are raw `eprintln`-style text (not LSDS), and they carry the exact P10-forbidden vocabulary (`ADR-006 §…`, `ckpt-5`, `REFUSED ON SIGHT`, `W12-typed-array-data-deletion audit`) that `comptime_message_has_jargon` scrubs elsewhere.

---

## 7. Test coverage in-territory

### 7.1 Counts

**Integration (shape-test):**

| Suite | Tests |
|---|---|
| `tests/comptime/blocks.rs` | 27 |
| `tests/comptime/annotations.rs` | 25 |
| `tests/comptime/type_info_chained.rs` | 21 |
| `tests/comptime/functions.rs` | 12 |
| `tests/comptime/flagship_wf3d.rs` | 8 (F1-F4 × VM/JIT) |
| `tests/annotations_comptime/type_mutation.rs` | 14 |
| `tests/annotations_comptime/directives.rs` | 11 |
| `tests/annotations_comptime/showcases.rs` | 10 (stdlib derive/LLM × VM/JIT) |
| `tests/annotations_comptime/on_define.rs` | 8 |
| `tests/annotations_comptime/code_gen.rs` | 8 |
| `tests/annotations_comptime/runtime_hooks.rs` | 1 |
| **Total** | **145** |

**Unit (`#[cfg(test)]` in source):** `comptime.rs` 51, `comptime_concrete.rs` 16 (all dead — module is `#[cfg]`-live but nothing calls the tested code), `comptime_target.rs` 13, `comptime_builtins.rs` 7, `comptime_diagnostics.rs` 7, `functions_annotations.rs` **0**.

### 7.2 Assertion quality

- **Good:** `type_info_chained.rs` pins exact concatenated reflection strings (`"TypedObject|id:int:Int|nickname:string:String:true"`); `flagship_wf3d.rs` asserts return values (not stdout) with an explicit, correct rationale for why stdout is untrustworthy under JIT (`flagship_wf3d.rs:8-18`); `showcases.rs` runs the stdlib patterns through real cross-module imports under both VM and JIT; the diagnostics firewall tests pin exact forbidden fragments.
- **Bad — the assertions that mattered were weak:** `ct_34_comptime_array` (`blocks.rs:218-229`) binds `let ITEMS: Array<int> = comptime { [1,2,3] }` and asserts only `expect_run_ok()` + an unrelated print. It never reads `ITEMS` back — which is precisely why the P0 garbage-embedding (§9.1) is green in CI. The explicit `Array<int>` annotation on a value that actually materializes as a Decimal also passes the type checker silently.
- **Bad — mislabeled suite:** `tests/annotations_comptime/on_define.rs` is headed "Covers: on_define firing when annotated item is defined" — **not one of its 8 tests uses `on_define`**; all use `comptime post`. The one hook family with a silent-drop bug (§9.4) has zero integration coverage under a filename that claims otherwise.

### 7.3 Ignored / deferred tests

- 4 × `#[ignore = "phase-2c — comptime rebuild against typed-Arc HeapValue layout — see ADR-006 §2.4"]`: 1 placeholder in `comptime.rs:1785`, 3 in `comptime_target.rs:705,721,886`. The reason **partially no longer holds**: the rebuild these placeholders wait for has substantially landed (the KindedSlot readback works for scalars/strings/TypedObjects), yet the placeholders remain unimplemented rather than replaced with real assertions. The 2 live regression tests beside the placeholder (Bool/None sentinel) show what should exist for arrays/objects.
- `comptime.rs:2985` `#[cfg(any())] mod tests_deferred` (~135 LOC): permanently un-compilable (asserts against deleted `ValueWord` API). Dead weight that will never be revived as-is.

### 7.4 Gaps (ranked by risk)

1. **Comptime→runtime readback of composite values** — no test reads back an array or anonymous object from a comptime block (P0/P1 both live here).
2. **Annotated targets inside modules** — zero tests apply a codegen annotation to a type in a `mod` or an imported module (both broken, §9.2/9.3). `showcases.rs` imports the *annotation* cross-module but always applies it to an entry-file type.
3. **`on_define`/`metadata`** — zero real coverage (§7.2).
4. **Wrapper emission** — the 509-line `compile_annotation_wrapper` has no unit tests; contract behavior is only covered end-to-end for the array/short-circuit paths (t19/t20 equivalents exist in `annotations.rs` ct_16).
5. **`comptime for` typing** — no test uses a `comptime for` loop variable in a typed position (the book's pattern).
6. **Negative-path parse tests** — no tests pin the binding-target parse gap or variadic-def-params rejection, so these silently disagree with the book.

### 7.5 Second-pass inventory: unit-test names, adjacent suites, differential corpus

**`comptime.rs` unit tests (21 named `test_*` + 30 more in the readback/regression families):** the visible spine is `test_comptime_simple_return` / `_string_return` / `_arithmetic` (`comptime.rs:2994-3082`), extension-registry plumbing (`test_comptime_with_sync_extension`, `_extension_registry_flows_through_compiler`, `:3083-3161`), the `vmvalue_to_literal` scalar family (int/number/string/bool/none/unit, `:3162-3197` — note: **no array, no object** variant, the §9.1/§9.8 gap in miniature), block parse/execute (`:3198`), builtin availability + gating (`:3235-3335`), target handoff (`test_comptime_with_target_simple` / `_from_function` / `_handler_end_to_end` / `_accesses_target_params`, `:3336-3541`), and the comptime-fn runtime-exclusion pair (`:3542-3599`). The scalar-only readback coverage is direct evidence for §7.2's conclusion: the exact test that would have caught the P0 (a `vmvalue_to_literal`/`nb_to_expr` **array** case) is the one missing from an otherwise systematic family.

**Adjacent suites (checked for territory overlap, mostly out-of-scope despite names):**
- `tools/shape-test/tests/type_inference/stress_annotations.rs` (847 LOC, 71 tests) — despite the name, tests *type annotations* (`let x: int = 42`), not `@`-annotations; zero overlap with this vertical.
- `tools/shape-test/tests/structs_types/generics_comptime.rs` (262 LOC, 20 tests) — generics tests (`fn id<T>`, generic structs); no comptime construct appears in any of them. The filename is a misnomer that inflates any grep-based estimate of comptime coverage.
- `tools/shape-test/tests/lsp/comptime.rs` (1 test) — see §1.7; runtime assertion, not LSP.

**VM/JIT differential corpus:** `tools/vmjit-diff/corpus/` carries **43 ACC-prefixed comptime/annotation programs** (`ACC__comptime__*` 33, `ACC__annotations__*` 10) plus `D__advanced__comptime*` book-fence extracts. This is real defense for the §5.1 split-brain class *for the programs it contains* — but the corpus probes are small single-file programs (probe1-8, b1-b6, pa-pf families), so the module-boundary and composite-value failures of §9.1-§9.3 are structurally outside what the differential can catch (both tiers fail identically there, so no diff fires).

---

## 8. Book/docs vs reality

Chapters audited: `advanced/comptime.mdx` (381 lines), `advanced/annotations.mdx` (559), `advanced/comptime-llm-patterns.mdx` (178), `advanced/comptime-annotations-cookbook.mdx` (~450), plus CLAUDE.md claims.

### 8.1 Where the book UNDERSELLS the product (stale caution)

`comptime.mdx:113-120`: "*applying* a hook to a real target — emitting directives such as `set return`, `set param`, `replace body`, or `extend` and having them take effect … is planned for v0.4 and not available in v0.3.3." **All of these work today** (t10-t12, t24-t25, t45, flagship gates). The caution box contradicts the rest of its own chapter (which documents `extend (expr)` with a worked example at :219-229 and gated snippets). Post-WF-3D this box must go.

### 8.2 Where the book OVERSELLS

| Claim | Reality |
|---|---|
| `comptime.mdx:345-358` — `comptime for field in target.fields` as the field-inspection idiom | loop var is `unknown`-typed; string concat with `field.name`/`field.type` is a compile error (§9.6). Equality comparisons happen to pass (t15/t55), which is why the book's own examples — which only compare — don't expose it. The stdlib derivers use plain `for` |
| `annotations.mdx:158-163` — `binding` target row | no grammar support; annotation before `let` is a parse error everywhere (t30, t30b) |
| cookbook "Mechanics Status (Current)": "await annotation `before` wraps the awaited input", "compile-time hooks … for function/type/expression/await/binding targets" | await/expression/block runtime wrappers die on the `op_new_array` stub (t27/t32/t50); binding targets don't parse |
| cookbook: "definition-time lifecycle hooks (`on_define`/`metadata`) enforced to function/type targets" | function-target hooks silently don't fire without runtime hooks; module hooks never fire (§9.4). ("Enforced-to" as target validation is accurate; firing is not) |
| `annotations.mdx:203-207` — `on_define`/`metadata` rows in the lifecycle table | same |
| `comptime.mdx:364-372` — "Use \[expand-comptime\] to inspect generated wrappers/specializations" | blind to `extend (expr)` free functions and `replace module` (§9.10) |
| `annotations.mdx:322` — "Variadic final parameter is supported" | true only for handler params with undocumented prefix syntax `...rest`; `rest...` (the natural reading) is a parse error (t46 vs t46b) |
| `annotations.mdx:225-244, 492-500` — `ctx.__impl` as the function-target impl reference | renamed to `ctx.target`; `ctx.__impl` is a runtime error (§9.14) |
| `annotations.mdx:266-273` — `before` object-return `state:` rebuild "carries the new state" | rebuilt ctx is offset-shifted; the new state is lost and `ctx.state` reads the old event_log (§9.12) |
| `comptime.mdx` Comptime Builtins table — omits `type_info`, `string_lit`, `item_fn` | all three are live user-facing builtins (`COMPTIME_BUILTIN_FORWARDERS`, `comptime.rs:20-66`); `type_info` is even a flagship WF-3D feature with its own gates. The book documents them nowhere in the builtin table (type_info appears only via llm-patterns "How it works" prose) |
| `llm_common_mistakes` in `comptime-llm-patterns.mdx` frontmatter: "The generated function is a free function named after the target" | true in entry files; generates unparseable code for module-resident targets (§9.2/9.3) — the book never mentions the module restriction |

### 8.3 Citation rot

The annotations chapter cites source line numbers extensively (`functions_annotations.rs:1441-1452`, `:1502-1693`, `compiler_impl_reference_model.rs:1015-1062`, …). Spot-checks show drift: the definition-hook restriction now sits at `compiler_impl_reference_model.rs:1720-1730` (cited `:1003-1012`); `emit_annotation_handler_call` begins at `functions_annotations.rs:582` (cited `:111-194`). The convention of hard line numbers in the book guarantees decay; anchor to function names instead.

### 8.4 Book-gate blind spot

The cookbook's `runnable=true` Recipe 2 / 2b fences only **declare** annotations (handlers with placeholder bodies) and never apply them (verified by extraction+run: both compile and print nothing, t55/t56). They gate parse+declare, not the recipe's semantics — consistent with the known denominator-trap finding on the book truth-gate. Note both trigger yet another jit-fallback variant ("top-level code has no MIR data") merely by containing comptime handlers.

### 8.5 CLAUDE.md claims

- "ergonomic annotations + comptime (enabling user-defined LLM integration patterns in stdlib/userland)" — **substantiated** (§2.8), single-file caveat.
- Language-features list: "`comptime { }` blocks executed at compile time, `comptime for`, comptime builtins (`type_info`, `implements`, `warning`, `error`, `build_config`)" — all verified except `comptime for`, which as a *top-level statement* is dormant + range-parse-broken; only handler-internal iteration works untyped.
- "Annotations: `@annotation name { @before { }, @after { }, @comptime { } }` with target validation and chaining" — the *syntax sketch* is wrong (real syntax is `before(args, ctx) { }` handlers, no `@`-prefixed hooks; `comptime pre/post` not `@comptime`); validation and chaining claims are accurate.

### 8.6 Second-pass book checks

- **Comptime fields (`comptime.mdx:309-323`):** the chapter presents `type Unit { comptime symbol: string = "m" }` plus the specialization `type Celsius = Unit { symbol: "°C" }` and states comptime fields are "available at compile time / excluded from runtime object storage/layout". Measured (§9.18): the specialization form **loses the field at runtime** (`Undefined property: symbol`), and the base form's field is served by a runtime dynamic-property read that forces a JIT deopt — i.e. it is *not* excluded from runtime storage in the shipped implementation; it is excluded only from the *typed* layout, which is precisely why the JIT refuses the read. Both halves of the book sentence are wrong in opposite directions.
- **`@indicator`'s doc comment is documentation-as-fiction** (`indicator.shape:1-20`): a 20-line rustdoc-style header with a worked `@example` documenting registry registration + memoization for an annotation that cannot compile at any application site (§9.16). This doc is exactly what the LSP hover/completion pipeline (§1.7) will render to users typing `@ind…` in a finance context — the tooling faithfully serves broken guidance.
- **`warmup.shape`'s self-documentation is stale in the other direction** (`warmup.shape:12-22`): it justifies its emptiness by a limitation ("any lifecycle handler causes the annotation argument to be compiled at module/definition scope … 'Undefined variable: period'") that no longer reproduces for runtime hooks (n03 — `@warm(p + 1)` with a `before` handler works and sees `p` bound at call time). The stdlib may be leaving a working feature unused based on an obsolete constraint.
- **No book chapter shows the `pub @warmup(1) fn obv(...)` inline application position** that the stdlib itself uses 29 times (§2.17) — the annotations chapter only ever shows annotations on their own line above the item.

---

## 9. Bugs & correctness risks found

All reproduced on the working tree with `target/debug/shape run` (default JIT mode). Scratch programs preserved under the audit scratchpad (`verticals/comptime-annotations/`).

### 9.1 P0 — comptime array results embed as type-confused Decimal garbage

```shape
let arr = comptime { [1, 2, 3] }
print(arr)
print(arr[0])
```

```text
V2 bytecode verification warning: 4 violation(s) found
  - V2 typed opcode NewTypedArrayI64 at offset 98 in function '__main__' has no FrameDescriptor
  - V2 typed opcode TypedArrayPushI64 at offset 101 ... (×3)
33D
Error: Runtime error: Not implemented: SURFACE: GetProp on Ptr(Decimal) not yet kinded — requires
the W17-typed-carrier-monomorphization replacement ... Key kind observed: Int64. (line 3)
```

String arrays print `737D`; in an f-string the int array printed `1073D` (t34). **Mechanism:** the mini-VM builds a v2-raw typed array; at readback, `nb_to_expr`'s scalar arms don't match, and the heap fallthrough calls `slot.as_heap_value()` (`comptime.rs:1551`) — a blind `&*(bits as *const HeapValue)` (`slot.rs:405-408`) on bits that are **not** an `Arc<HeapValue>` raw pointer. The array's header bytes happen to decode as the `Decimal` discriminant, so the match takes `HeapValue::Decimal(d)` (`comptime.rs:1557`) and embeds `Literal::Decimal(<reinterpreted memory>)`. The intended TypedArray arm is the explicitly deleted one (`comptime.rs:1560-1564`, "Comptime materialization of v2-raw `TypedArray<T>` arrays lands at ckpt-6") — but because the misread matches Decimal *before* reaching the `other => Err(...)` arm, the planned surface-and-stop never fires. This is the §2.7.16 receiver-recovery soundness violation in live code: silent wrong results from a type-confused heap read (undefined behavior in the Rust sense — today it prints garbage; a different allocation layout could crash). The declared binding type (`Array<int>`, in ct_34) does not catch it.

**Fix shape:** add explicit `NativeKind::Ptr(HeapKind::…)` arms for the v2-raw array kinds that error cleanly ("unsupported comptime literal value") *before* the `as_heap_value()` fallthrough — the same pattern already applied for TypedObject at `comptime.rs:1527-1539` — until ckpt-6 lands real materialization.

### 9.2 P1 — `extend (expr)` codegen broken for imported-module targets (qualified `target.name`)

Module `gen4.shape` defines `@labeled4` (type-target, `extend (f"fn {target.name}_tag() …")`) and applies it to `type Gizmo`. Importing it:

```text
$ shape run t31f.shape          # use gen4; print(gen4::use_tag4())
error[RUNTIME]: Bytecode compilation failed: Semantic error: [C0001] invalid replacement module
payload: expected something else, found `}`
   = note: during compile-time evaluation of the @labeled4 annotation handler
```

Replacing `extend` with `error(f"PAYLOAD=[{payload}]")` exposes the cause — the same handler sees different `target.name` in the two compiles:

```text
direct run:    PAYLOAD=[fn Gizmo_tag() -> int { 7 }]
imported run:  PAYLOAD=[fn gen4::Gizmo_tag() -> int { 7 }]     ← `fn gen4::Gizmo_tag` is not parseable
```

Every name-splicing generator — including stdlib `@json_schema`/`@llm_tool` — breaks for module-resident targets. The error message ("invalid replacement module payload", from `parse_module_items_payload`'s reuse at `comptime_builtins.rs:386` via `parse_extend_items_slot:629`) misattributes the failure and names neither the qualified name nor the offending generated source.

### 9.3 P1 — `extend (expr)` also broken for same-file `mod` targets, with error-recovery leak

```text
$ shape run t51_mod_same_file.shape   # mod inner { @json_schema() type Point {...} }
Error: Runtime error: Bytecode compilation failed: Semantic error: [C0001] invalid replacement
module payload: expected an identifier, found `\`
Runtime error: Undefined function/Unknown qualified call 'inner::Point_json_schema'. ...
```

Different corruption (a literal backslash survives into the payload — f-string `\{` escapes are not processed the same way on this path), *and* the module's compile error does not abort the program: execution continues and fails later at the call site. Two bugs: the payload corruption, and compile-error demotion.

### 9.4 P1 — `on_define`/`metadata` silently dropped for two of three documented target kinds

```text
annotation reg2() { targets:[function]  on_define(target, ctx){ print(...) } }
@reg2() fn hello2() -> int { 2 }         →  "call result: 2"        (hook never fires)

annotation reg3 = on_define + metadata    →  "call result: 3"        (neither fires)

annotation reg1 = on_define + metadata + before
                                          →  on_define fired: hello  ✓
                                             metadata fired: hello   ✓
                                             before fired            ✓

targets:[type],   on_define only          →  type on_define fired: Blob  ✓ (plus V2-verifier JIT deopt)
targets:[module], on_define only          →  (nothing)                ✗
```

Function-target lifecycle emission happens only at the tail of the wrapper compile path (`functions.rs:1238`); annotations without runtime hooks never route there, and `emit_annotation_lifecycle_calls_for_target`'s `let Some(...) else { continue }` (`functions_annotations.rs:555-557`) guarantees silence rather than an internal error. Module targets: `emit_annotation_lifecycle_calls_for_module` exists (`functions_annotations.rs:530`) and is called from `statements.rs:6096`, but empirically never fires for the `@registered() mod` shape. No diagnostic in any failing case.

### 9.5 P1 — expression / block / await annotation targets die at runtime on the args-array stub

```text
$ shape run t27_expr_target.shape    # let v = @traced_expr("expr") (20 + 22)
Error: Runtime error: Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5 consumer-cascade
tier 3 surface. ... (line 8)
```

Identical failure for `block` (t32) and `await_expr` (t50). The non-function wrapper emitters build the handler's `args` array with the stubbed `op_new_array`, while function-target wrappers use the live typed-array opcodes — which is why function targets work. Compiles clean; fails only at runtime. The book marks expression/await examples `runnable=false` but sells all three kinds in its target table, and the cookbook's status section claims the await mechanics are implemented.

### 9.6 P1 — `comptime for`: dormant statement form, range parse gap, untyped loop vars

```text
comptime for i in 0..3 { ... }        →  parse error: unexpected `.`, expected identifier
comptime for x in [1,2,3] { ... }     →  [SEMANTIC] comptime-for unroll outside a comptime block is
                                          dormant pending the phase-2c ComptimeExecutionResult /
                                          Literal-projection rebuild (ADR-006 §2.4 / §2.7.4)
# inside annotation handler:
comptime for field in target.fields { props = props + field.name }
                                      →  [C0001] Cannot apply `+` to a `string` and a `unknown`.
# same with f-string wrap `props + f"{field.name}"` — same error (t13b)
# plain for: works, fully typed (t13c prints props=idname)
```

The grammar's iterable is `postfix_expr` (`shape.pest:152`) so range literals can't parse. Handler-internal `comptime for` gets a special lowering that loses the `FieldDescriptor` element type that plain `for` retains. The book teaches the broken form (§8.2).

### 9.7 P1 — annotation runtime hooks force whole-program JIT deopt with un-firewalled jargon; `set param` reaches the Cranelift verifier

Every program using `before`/`after` prints (default mode):

```text
[jit-fallback] function main failed JIT compile: Runtime error: JIT compilation failed: WF-1A
signal-reexec (audit 2026-07-04 §4(a)): JIT finalize could not resolve a native reference to a
function that failed Phase-4 JIT compile (can't resolve symbol main_f195_add). Whole-program deopt
to the bytecode interpreter at COMPILE stage ...; running under interpreter
```

`set param` default values go further — bytecode and JIT disagree on arity:

```text
[jit-fallback] ... Compilation(Verifier(VerifierErrors([VerifierError { ... "mismatched argument
count for `v4 = call fn21(v0)`: got 1, expected 2" }]))); running under interpreter
```

Results are correct (interpreter), but: (a) performance silently degrades to interpreter for the *whole program* the moment one annotation exists; (b) the stderr text leaks the exact internal vocabulary (`WF-1A`, `audit 2026-07-04 §4(a)`, `ADR-006 §2.7.14`, `V3-S5 ckpt-5`, `REFUSED ON SIGHT`) the P10 jargon firewall exists to suppress; (c) the verifier-level arity split is a latent wrong-results bug if deopt granularity ever changes (§5.1).

### 9.8 P1 — anonymous-object comptime results silently embed as null

```text
$ shape run t35_comptime_obj.shape    # let cfg = comptime { { host: "localhost", port: 8080 } }
Error: Runtime error: TypeError: expected object, array, string, or other heap value, got scalar (line 2)
```

The object literal inside the mini-VM doesn't produce a TypedObject-kinded slot the readback recognizes, so it degrades to `Literal::None` with **no compile-time diagnostic**; the failure surfaces at first field access, far from the cause. Contrast `build_config()` (a schema'd TypedObject): fully works (t54). Same readback layer as 9.1, milder failure mode.

### 9.9 P2 — `type_info` classification gaps

```text
enum Color { Red, Green, Blue }  →  type_info(Color).kind == "TypedObject"     (no Enum kind)
type Wrap { inner: Option<int>, items: Array<string> }
  → fields[0].type == "int"      (Option-ness erased into the row's `optional` flag — defensible but lossy)
  → fields[1].type == "[string]" (non-source syntax; `type_info("[string]").kind == "Unresolved"` — chaining dead-ends)
type_info("Option<int>").kind == "Unresolved"   (no parametrized-type support)
```

The enum flattening is a *documented* interim (`comptime_builtins.rs:1292-1298`), but it makes enum-aware derive macros impossible to write, and the two classifiers disagree (§4.2). `type_info` on `number`/unknown names is correct per the F2 gates (`Number`/`Unresolved`).

### 9.10 P2 — tooling/diagnostics paper cuts

- `expand-comptime` reports "No comptime expansions found" for programs whose annotations generate free functions via `extend (expr)` or rewrite modules via `replace module` — the flagship surfaces (transcripts §2.12).
- Comptime warnings carry file **`<synthetic>`** (`warning[C0002]: ... --> <synthetic>:6:1`) instead of the user's file; lines are handler-relative for annotation warnings. Errors get real spans; warnings don't.
- "V2 bytecode verification warning" blocks print **twice** per compile (t13c, t34, t46b).
- Imported-module handler `warning()`s are swallowed entirely (§2.10).
- `@range(0.0, 1.0)` field-annotation args stringify as `0`/`1` (float rendering drops the decimal, t15) — args are stringified at descriptor-build time (`FieldAnnotation = (String, Vec<String>)`, `comptime_target.rs:49`), so numeric fidelity is lost before the handler sees them.

### 9.12 P1 — `before` state-rebuild contract loses the new state via a field-offset shift

```shape
annotation counted2() {
  targets: [function]
  before(args, ctx) { { args: args, state: { calls: 1 } } }
  after(args, result, ctx) { print(f"after state={ctx.state} log={ctx.event_log}"); result }
}
@counted2()
fn work(x: int) -> int { x }
print(work(7))
```

```text
baseline (before returns bare args):   after state={} log=[]        ← correct
with state rebuild (above):            after state=[] log=None      ← state is the OLD event_log;
                                                                      event_log is absent; {calls: 1} gone
7
```

The book documents this contract as working: "the rebuilt `ctx` carries the new state and a fresh empty `event_log`" (`annotations.mdx:270-273`). The observed values are a textbook one-field offset shift: the function-target ctx schema is 3-field `{target, state, event_log}` (`functions_annotations.rs:2871-2895`), while the rebuild path constructs a 2-field object — reading `state` at the old offset lands on the rebuilt object's `event_log`, and `event_log`'s offset is past the end (`None`). This is the §4.4 schema-order bug class, live in the shipped wrapper contract. Any stateful annotation (rate limiter, memoizer, circuit breaker — cookbook Recipes 6-10) silently loses its state.

### 9.13 P1 — trait-method dispatch inside comptime blocks is unusable in all configurations

```text
# (a) plain trait + plain impl, method called inside comptime { }:
[C0001] Method 'describe' not found on type 'Cfg'

# (b) comptime trait + comptime impl (the J-CT.2 form):
[C0001] comptime alignment mismatch: trait 'Describe' is_comptime=true, impl for 'Cfg'
        is_comptime=false — both must agree

# (c) plain trait + comptime impl:
comptime alignment mismatch: trait 'Describe' is_comptime=false, impl ... is_comptime=true
```

(b) is self-inflicted: `execute_comptime_with_context` deliberately clears `is_comptime` on impl blocks before injecting them into the mini-program ("we clear `is_comptime` on the cloned blocks", `comptime.rs:668-687`) but injects trait defs **unmodified** (`comptime.rs:678-680`) — so the mini-VM's own alignment validator always sees a mismatch for the exact configuration the feature requires. (a) is by-design-ish (runtime impls aren't imported into the mini-VM) but means there is **no** working path to `value.method()` on user traits at comptime.

### 9.14 P2 — book documents the renamed-away `ctx.__impl`

`annotations.mdx` builds three subsections on `ctx.__impl` (schema table :225-231, field semantics :234-238, `@remote` mechanics :492-500). The working tree renamed it to `ctx.target` (§4.1.5/OQ-12; `functions_annotations.rs:2871-2878` — "no stringly `ctx[\"__impl\"]` lookup"; `remote.shape:183-189`). Empirically `ctx.__impl` → `Undefined property: __impl` (t57b); `ctx.target` works (t57c). Anyone implementing the book's canonical short-circuit-redirection pattern hits a runtime error.

### 9.15 Verified-good negative paths (no bug)

For balance, adversarial probes that behaved correctly: comptime scope isolation (t17); comptime-fn-from-runtime rejection with a precise message (t18); target-kind validation (t26); watchdog kills a comptime infinite loop at 5s with a clean firewalled message (t47); `error()` in a handler halts compilation with the annotation site's span (t44/gen4); recursion in `comptime fn` (t39); before-contract short-circuit never runs the impl (t19); stacked-annotation ordering matches spec (t16); keyword-named generated functions are rejected by `is_valid_generated_function_name` (`comptime_builtins.rs:409-459`).

**Second-pass re-verification note:** findings 9.1, 9.2, 9.4 and 9.12 were independently re-run from the preserved programs before the additions below were made; all four reproduced byte-identically (same `33D` garbage + `Ptr(Decimal)` SURFACE for 9.1; same `PAYLOAD=[fn gen4::Gizmo_tag() -> int { 7 }]` for 9.2; same silent `call result: 2` for 9.4; same `after state=[] log=None` for 9.12).

### 9.16 P1 — shipped stdlib `@indicator` annotation cannot compile at any application site

```shape
from std::finance::annotations::indicator use { @indicator }
@indicator()
fn double(x: int) -> int { x * 2 }
print(double(21))
```

```text
$ shape run n01_indicator.shape
error[RUNTIME]: Bytecode compilation failed: Semantic error: Cannot infer types for binary
operation `Add`: operand types are `unknown` and `unknown`. Strict typing requires both operands
to have a known concrete type at compile time. Add a type annotation to disambiguate.
  --> <input>:5:1
```

The failing expression is the handler's own `self.name + ":" + args.toString()` (`indicator.shape:33`): `self` is not a binding in the current `(args, ctx)` handler contract, so both operands infer `unknown`. Every other element of the file is equally legacy — `ctx.get("registry")` / `ctx.get("cache")` against a ctx that is `{target, state, event_log}` (§2.13), `on_define(ctx)` with the wrong arity, a bare-value `before` return expecting short-circuit semantics the current contract reserves for `{result: …}` objects. The annotation is importable (resolution succeeds — the error is *inside* the handler body at application time), so a user is led all the way to the application site before the failure, and the diagnostic blames *their* file (`--> <input>:5:1`) with a generic inference message that names neither the stdlib file nor the handler. Zero stdlib application sites exist (§2.17), and no test anywhere applies `@indicator` — which is how a broken contract fossil ships.

### 9.17 P1 — annotated `extend`-block methods hard-fail under default JIT mode (work under `--mode vm`)

```shape
annotation traced() {
  targets: [function]
  before(args, ctx) { print("traced before") }
}
type Point { x: int }
extend Point {
  @traced()
  method get_x() -> int { self.x }
}
let p = Point { x: 5 }
print(p.get_x())
```

```text
$ shape run n08e_extend_method_ann.shape            # default mode (jit)
Error: Runtime error: JIT method dispatch for `get_x` resolved `Point.get_x` but that method
was not JIT-compiled

$ shape run --mode vm n08e_extend_method_ann.shape
traced before
5
```

This is a **VM/JIT behavioral divergence with no deopt safety net** — worse than the function-target case (§9.7), where the whole-program deopt at least preserves correctness. The mechanism composes two known designs into a failure: annotation wrappers deliberately ship no `mir_data` (§5.1, `functions.rs:1128-1137`), and JIT *method dispatch* apparently requires the resolved method to be JIT-compiled rather than falling back to the interpreter for that method. Function-target annotations trip the whole-program deopt during `main`'s compile; method-target annotations get past compile and die at the dispatch site. Any user who moves an annotated function into an `extend` block converts a noisy-but-working program into a hard runtime error on the default mode. No test covers an annotated method (the t10/F4-class tests all use *generated* methods, which take the full-driver path and JIT natively).

### 9.18 P1 — comptime-field specialization (`type X = Base { field: v }`) loses the field

```text
$ shape run n04b_unit_direct.shape    # type Unit { comptime symbol: string = "m" }; Unit{}.symbol
[jit-fallback] ... MirToIR: unresolved direct field read `.symbol` (field idx 0) lacks a
statically proven typed-object byte offset ... deopt to the bytecode interpreter ...
m                                     ← base type works (via interpreter dynamic property)

$ shape run n04_comptime_field_followon.shape   # + type Celsius = Unit { symbol: "C" }; Celsius{}.symbol
[jit-fallback] ... (same deopt)
Error: Runtime error: Undefined property: symbol (line 4)
```

The specialization form is first-class grammar (`comptime_field_overrides`, `shape.pest:110-115`, reachable from the `:101` type-alias production and the `:1108` `as`-cast) and a worked example in the book (`comptime.mdx:317`); the alias type instantiates fine but carries no `symbol` at all. Additionally the base-type behavior shows comptime fields are *not* compile-time-folded as documented — the read survives to runtime as a dynamic property (JIT-unprovable, hence the deopt), meaning every comptime-field access in a hot path silently drops the whole program to the interpreter.

### 9.19 P2 — nested comptime blocks are untyped; cross-boundary calls get an unhelpful diagnostic

```text
$ shape run n06_nested_comptime.shape   # let x = comptime { comptime { 20 } + 22 }
error[SEMANTIC]: [C0001] Cannot infer types for binary operation `Add`: operand types are
`unknown` and `int`. ...
   = note: during compile-time evaluation of a compile-time block

$ shape run n07_comptime_calls_runtimefn.shape   # fn plain(x: int) -> int; comptime { plain(4) }
error[SEMANTIC]: [C0001] Undefined function: 'plain'
```

The outer comptime compiler types a *nested* `comptime { }` expression as `unknown` instead of either flattening it (a comptime block inside a comptime block is just a block) or projecting its literal type — inconsistent with the top-level behavior where `comptime { 20 }` embeds a typed `int` literal. The n07 rejection is correct semantics (§1.2 scope isolation) but the message is the generic undefined-function error; it should name the comptime/runtime boundary and suggest `comptime fn`, matching the quality of the inverse-direction message (t18).

---

## 10. What is done well

1. **One execution model, not a second interpreter.** Comptime reuses the production compiler and VM recursively (`comptime.rs:722-773`). Handler code obeys the exact strict-typing rules of runtime code; there is no drift-prone "comptime dialect" evaluator. This is the single most important architectural decision in the vertical and it is right.

2. **The jargon firewall with an executable acceptance probe.** `clean_comptime_message`/`sanitize_comptime_internal` plus a test that greps rendered output for the P10 forbidden-fragment list (`comptime_diagnostics.rs:136-163`) — including the subtle case of preserving a *user's* `§` while stripping internal ones (`:181-192`). Turning "no internal jargon reaches users" from a wish into a pinned test is exemplary.

3. **LSDS-first diagnostics with machine-readable parity.** One `Diagnostic` source of truth, terminal and JSON renderers derived from it; `--diagnostics json` emits the same `C0001` + comptime-trace note as the human output (verified end-to-end §2.7). WF-3D's F3 gates pin the JSON shape independently of the process-global format.

4. **The comptime watchdog.** A 5-second interrupt thread around every mini-VM run (`comptime.rs:1331-1337`) with the timeout mapped to a clean user sentence. Compile-time user code cannot hang the compiler (empirically verified, t47).

5. **The WF-3D root fix for generated functions.** Generated items compile through the full driver so they carry `mir_data` and JIT natively instead of poisoning the whole program (`flagship_wf3d.rs:22-26`); the VM==JIT parity gates assert return values, with a written rationale for why stdout assertions lie under JIT (`flagship_wf3d.rs:8-18`). The gates themselves double as a model of how to test this vertical.

6. **Stdlib patterns on the public contract.** `@json_schema`/`@to_json`/`@llm_tool`/`@prompt` are 248 lines of ordinary Shape using only `target.*`, `error()`, `string_lit`, `extend (…)` — no compiler builtins (`derive.shape`, `serialize.shape`, `tools.shape`). That the flagship demos are dogfood, not intrinsics, validates the whole design's claim to user-extensibility. `@llm_tool`'s strictness rules (missing return type / untyped param = named compile errors) are real and tested (t44).

7. **Bare-type-identifier ergonomics done at the right layer.** `type_info(User)`/`implements(Dog, Speak)` accept bare identifiers via a fully-recursive AST rewrite (`comptime.rs:380-458`) matched by the outer checker, so users don't write string literals for type names.

8. **Surface-and-stop discipline held.** The dormant `comptime for` unroll, the deleted TypedArray materialization arm, and the module-payload parse failures all *stop with named errors* rather than silently falling back (with the one P0 exception where a misread bypasses the stop — §9.1). The forbidden-pattern vocabulary from CLAUDE.md is absent from live code in this territory.

9. **Directive transport via drained thread-locals** (`comptime_builtins.rs:170-231`) is simple, reentrancy-safe (cleared before each run, drained after), and keeps `extend`/`warning` usable from arbitrarily nested handler code without threading a context parameter through the mini-VM.

10. **Honest provenance comments.** Nearly every regression fix in the readback layer documents the exact prior bug (the swapped-offset schema at `comptime_builtins.rs:874-880`, the `false`→`null` sentinel conflation at `comptime.rs:1397-1404`, the `build_config()` SIGSEGV at `:1511-1526`). The file teaches its own failure history — rare and valuable.

---

## 11. What is done poorly / tech debt

1. **The comptime→runtime readback layer is the vertical's soundness chokepoint and it is unfinished.** Scalars/strings/TypedObject work; arrays are UB-adjacent garbage (§9.1); anonymous objects silently null (§9.8). The layer's own comments prove the authors knew the `as_heap_value()` trap — they fixed it twice (TypedObject, FilterExpr-class) and left the array kinds falling through. Pending "ckpt-6" is not an excuse for a *silent* hole; the stop-arm is one match-arm away.

2. **Module-boundary support was never part of the flagship's definition of done.** WF-3D gated F1-F4 in entry files only; no test, book sentence, or code path addresses `target.name` qualification or payload escaping inside `mod` (§9.2/9.3). The result: the priority-spine feature demos perfectly and breaks on the first realistic project layout.

3. **`functions_annotations.rs` is a 3,310-line, zero-unit-test bytecode emitter** whose central function is 509 lines of hand-rolled jump graphs. The before-contract is duplicated into `expressions/mod.rs` (§4.1). Any change to the wrapper ABI is currently verified only by end-to-end tests of the happy paths.

4. **Lifecycle emission is path-dependent.** Whether `on_define` fires depends on which *compile path* the target took (wrapper vs plain vs module), not on the annotation's declaration (§9.4). Hook dispatch belongs in one place keyed on the compiled annotation, with a hard error (not `continue`) when a registered handler can't be resolved.

5. **Dead and fossil code:** `comptime_concrete.rs` (393 LOC + 16 tests, `#![allow(dead_code)]` since the 4d migration stalled); `#[cfg(any())] mod tests_deferred` in `comptime.rs` (~135 LOC asserting against the deleted `ValueWord` API — can never compile again); 4 `phase-2c` ignored placeholder tests whose blocking reason has substantially expired; `nb_*` naming residue and the `vmvalue_to_literal` alias (§3.1).

6. **Hand-synced quadruplicated descriptor schemas** (§4.4) with a memorialized prior bug of exactly the class the duplication invites, and no mechanical lockstep check (the project has precedent for such checks — the HeapKind 4-table lockstep in verify-merge — but none covers `__Comptime*` schemas vs their `TypeAnnotation` mirrors).

7. **The default-mode UX for this vertical is noisy to the point of alarming.** Six distinct stderr banners were triggered by ordinary comptime/annotation programs in this audit ([jit-fallback] × 4 variants, V2-verification warnings printed twice, module pre-resolution warnings). A user's first annotation prints a 7-line internal audit dump before their own output. The comptime channel's P10 firewall demonstrates the team knows better; the neighboring channels weren't held to it.

8. **`comptime for` exists in three half-states** (parse-broken for ranges / dormant at top level / untyped in handlers) while the book teaches it and the grammar advertises it. Either finish the phase-2c unroll or make the parser reject it with the dormant message everywhere — the current split maximizes confusion.

9. **Annotation argument fidelity:** field-annotation args are stringified into `Vec<String>` at descriptor build (`comptime_target.rs:49`), losing numeric shape (`0.0` → `"0"`, §9.10). Handlers consuming `@range(0.0, 1.0)` cannot recover the author's values exactly — a wrong foundation for the validation-generation patterns the book advertises.

10. **The legacy annotation era was never cleaned up, and it still teaches.** Three artifacts from the pre-`(args, ctx)` contract survive in the tree, each actively misleading: the dead Rust-side `AnnotationContext` half of `annotation_context.rs` with its stale rustdoc and phantom `pattern.shape` example (§5.7); the un-compilable stdlib `@indicator` (§9.16); and the `@warmup` doc rationale that no longer reproduces (§2.17/n03). Together they form a self-consistent *wrong* documentation set for anyone spelunking the annotation system — the LSP will even render `@indicator`'s fictional docs as hover help (§8.6). One cleanup pass (delete the dead ctx half, rewrite or delete `@indicator`, re-test `@warmup`'s constraint) removes all three.

---

## 12. Prioritized recommendations

### P0 (soundness — do first)

1. **Close the `as_heap_value()` fallthrough in `nb_to_expr`/`nb_to_literal`** (`comptime.rs:1439`, `:1551`): add explicit arms for every v2-raw `NativeKind::Ptr(HeapKind::…)` the mini-VM can return (arrays, hashmaps, anything non-`Arc<HeapValue>`) that produce the clean "unsupported comptime literal value" error until real materialization lands. One day of work; converts silent garbage into a named compile error. Add readback tests that *consume* the value (fix `ct_34_comptime_array` to index and compare).

### P1 (flagship-blocking)

2. **Unqualify `target.name` (or add `target.local_name`) for module-resident targets** and make generated-item splicing module-aware; fix the `\{` escape handling on the same-file-`mod` path. Add F1/F4-style gates for: type in `mod`, type in imported module, generated fn called cross-module. (Estimated: days — the descriptor build and the payload-splice site both need a module context they already have access to.)
3. **Make lifecycle-hook emission path-independent** and turn the silent `continue` at `functions_annotations.rs:555-557` into an internal error; emit `on_define`/`metadata` for plain (non-wrapped) functions and modules. Rename or populate `tests/annotations_comptime/on_define.rs` with real `on_define` assertions.
4. **Type the `comptime for` loop variable** from the iterable's element annotation inside handlers (parity with plain `for`), fix the range-literal parse (`shape.pest:152` — accept `expression` not `postfix_expr`), and either implement or grammar-reject the top-level form. Update `comptime.mdx` to match whichever lands.
5. **Either wire the expression/block/await wrapper args-array to the live typed-array construction** (as function-target wrappers already do) or reject those targets at compile time with a clear "not yet supported at runtime" error. Compile-clean/runtime-die is the worst of both. Update the cookbook status list.
6. **Quiet the default mode:** route `[jit-fallback]` for the *known-by-design* annotation-wrapper deopt through a single terse LSDS-styled notice (and through the jargon firewall), and fix the double-printed V2-verification warning. Track the `set param` arity split (§5.1/§9.7) as its own JIT bug — it is a wrong-results landmine behind the deopt.
7. **Fix the ctx-rebuild field shift** (§9.12): rebuild the function-target ctx with the full 3-field `{target, state, event_log}` schema (or read the rebuilt object through its own schema id, never the original offsets). Add a state-round-trip test — every stateful cookbook recipe depends on it.
8. **Decide comptime traits** (§9.13): either stop clearing `is_comptime` asymmetrically (clear the trait too, or neither) so J-CT.2 is exercisable, or reject `comptime trait`/`comptime impl` at parse with a "not yet supported" message. Today's behavior is a maze with no exit.

### P1 additions from the second pass

8b. **Fix or delete stdlib `@indicator`** (§9.16): either rewrite it on the current `(args, ctx)`/`ctx.target` contract (hours — `remote.shape` is the working template) or remove the file; in both cases add a stdlib-wide test that *applies* every `pub annotation` at least once so contract fossils cannot ship. Re-evaluate `@warmup`'s handler-stripping rationale against n03's evidence that runtime-hook args now bind fn params correctly.
8c. **Make JIT method dispatch deopt (not error) on non-JIT-compiled methods** (§9.17): the annotated-method case proves a hard-error path exists where every other wrapper case deopts. Likely a one-site fix in the method-dispatch resolution; add an annotated-`extend`-method test to the F-gate family under both modes.
8d. **Implement or reject comptime-field specialization** (§9.18): `type Celsius = Unit { symbol: "°C" }` must either carry the overridden field or fail at compile time with a named error; and decide whether comptime fields are compile-time-folded (book semantics — also fixes the per-access JIT deopt) or typed runtime fields. The current half-state fails both definitions.
8e. **Type nested comptime blocks** (§9.19; trivial: treat inner `comptime {}` inside comptime context as a plain block) and upgrade the comptime→runtime undefined-function diagnostic to name the boundary.

### P2 (quality-of-life)

9. Teach `expand-comptime` about `extend (expr)` free functions and `replace module` output — it already runs the real pipeline, it only fails to report these item classes.
10. Fix comptime-warning spans: carry the real file (not `<synthetic>`) and map handler lines to the application site like errors do; re-emit imported-module handler warnings at the import site instead of swallowing them.
11. Add an `Enum` TypeKind (the classifier already has `enum_defs` at hand, `comptime_builtins.rs:1288-1299`), unify the two type classifiers (§4.2), render array types in source syntax (`Array<string>`), and support parametrized names in `type_info`.
12. Preserve annotation-argument values (typed, not stringified) in field descriptors; document the `...rest` variadic syntax; delete `comptime_concrete.rs` and `tests_deferred` or wire them; replace the 4 phase-2c placeholder tests with real readback assertions; add a lockstep test between `__Comptime*` schemas and their `TypeAnnotation` mirrors.
13. Book fixes: remove the stale v0.4-preview caution from `comptime.mdx`; rename `ctx.__impl` → `ctx.target` everywhere; re-anchor source citations to function names; document the `...rest` syntax and the module-boundary restriction on name-splicing codegen until fixed.

---

## Appendix: test-program index

~70 programs, scratchpad `verticals/comptime-annotations/`: t01-t07 builtins/blocks; t09-t12 lifecycle basics; t13* flagship + minimizations; t14-t17 fields/field-annotations/stacking/scope; t18-t20 contracts; t24-t26 replace/remove/validation; t27-t32 non-function targets + comptime-for forms + on_define matrix (t29*); t31* imported-module codegen (+ modtest/gen*.shape); t33-t39 visibility/value-kinds/type_info/build_config/print/recursion; t40-t44 stdlib LLM/derive patterns; t45-t50 set-param/variadic/watchdog/target-params/const-spec/await; t51-t56 mod-in-file/collision/comptime-in-fn/build_config-object/cookbook-recipes; t57-t66 ctx contract (target/__impl/state)/pre-post/set-param-type/extend-named/replace-body-block/comptime-traits/pre-error/async/remote.

Second pass (n-series): n01 stdlib `@indicator` application (§9.16); n02 annotation-def args; n03 annotation arg over fn param (`@warm(p + 1)`); n04/n04b comptime-field base + specialization (§9.18); n05b-d HashMap controls (failures reproduced outside comptime — out-of-territory, excluded); n06 nested comptime (§9.19); n07 comptime→runtime call isolation; n08c/n08d/n08e inherent-impl controls + annotated extend-method under jit/vm (§9.17); n09b `implements` with real trait/impl; n10 metadata handler. Plus two `expand-comptime` transcripts (§1.7).


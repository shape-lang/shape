# Comptime Excellence — Design Doc (WF-D, ratified 2026-07-05)

**Status:** RATIFIED 2026-07-05 (user) — all twelve recommended defaults adopted, no overrides (overview §3 Q36–Q47; §8 record below). Drafted as v2 under the WF-D priority-spine 2026-07-05 ruling: comptime must be *excellent and ergonomic*, not merely un-broken. Revised 2026-07-05 against the three-lens adversarial review: §4.5.7 (generation surface) and §4.1.5 (runtime-hook contract) written; showcases rewritten contract-clean (incl. the `\{`/`\}` escaping reality); fixed-arity kind checks made class-aware (§4.2.2a/2b); directive sequencing strengthened to a whole-program pre-pass (§4.5.1); captures matrix un-foreclosed for function targets; open questions 10–12 added; review rebuttals recorded in §5.
**Implements against:** `docs/cluster-audits/fix-plan-2026-07-05-workflows.md` (WF-1B `comptime-marshal-family` + WF-1B ergonomics amendment).
**Binding constraints:** `CLAUDE.md` §Forbidden Patterns, ADR-006 (`docs/adr/006-value-and-memory-model.md`) §2.7.4/§2.7.5/§2.7.7/§2.7.8, ADR-005 single-discriminator, `docs/runtime-v2-spec.md`.
**Read-only provenance:** every file:line below was verified against workspace HEAD `1fb805b3` on 2026-07-05. No code was modified.

---

## 1. Goals & non-goals

### Goals

1. **A stable, documented introspection contract.** `target.fields`, `target.params`, `type_info()` and the annotation/directive surfaces become a versioned API with exact row shapes, a compat stance, and book tables that are normative.
2. **Diagnostics at the Zig/Rust bar.** `error()` preserves the user's message with a source span; `warning()` surfaces through the compiler diagnostic channel (LSDS) instead of a bare `eprintln!`; comptime failures carry a comptime stack trace; **zero internal jargon** ("V3-S5 ckpt-5", "REFUSED ON SIGHT", ADR section numbers) ever reaches user-facing text.
3. **Fix the two root causes correctly:** (a) the Bool-collapse variadic marshal — replaced by per-position typed `KindedSlot` carriers whose kinds come from the VM's §2.7.7 parallel kind track (never fabricated); (b) the descriptor schema-identity collision — replaced by named, concrete, deterministically-registered schemas so an id can never mean two different things.
4. **Directive type-safety.** Every comptime directive that mutates a signature (`set return`, `set param`, `replace body`) re-enters the normal strict-typing pipeline. The `set return` SIGSEGV becomes a compile error. `__original__` becomes a properly typed call, not an injected untyped array.
5. **Ergonomics upgrades ranked by leverage** — error quality, discoverability, expansion inspection (LSP + CLI), field-level type introspection.
6. **Two polished stdlib showcases** that run green in the book gate under vm AND jit: a derive-style schema/serialization pattern and an LLM-integration pattern (making the CLAUDE.md "user-defined LLM integration patterns in stdlib/userland" claim true — it currently has zero instances).
7. **A book chapter plan** that makes the documented contract and the shipped binary agree in both directions.

### Non-goals

- **No new comptime execution engine.** The per-block VM (`comptime.rs:301/334/713/765`), the 5s watchdog, recursion/generics via the normal pipeline — all verified working and kept as-is.
- **No macro/token-stream system.** Shape's comptime is semantic (runs real Shape code against typed descriptors); we are not adding a syntactic quasi-quoting tier (token templates with hygiene/splice rules) in this round. The v1 **generation surface** is *source-string emission through the normal parser* — computed Shape source, parsed once, compiled by the strict pipeline. That is the existing `replace body (expr)` / `replace module (expr)` mechanism (`parse_function_body_payload` / `parse_module_items_payload`, comptime_builtins.rs:196-212/:214-226, both already accept source text) generalized by one directive (`extend (expr)`), not a new syntactic tier. The full specification — syntax, escaping, hygiene, and error spans into generated code — is **§4.5.7**; both stdlib showcases (§4.9) are written against exactly that surface and nothing more.
- **No JIT-of-comptime.** Comptime blocks run interpreted in the compiler; that is fine (they are short). JIT correctness for *generated* runtime code (annotation wrappers) is WF-1A(c) territory; this doc only states the acceptance criteria it must meet.
- **No comptime I/O surface.** Comptime stays deterministic (see §4.8 permission story).
- **Not in scope here:** the annotation-hooks-dropped-under-JIT bug (WF-1A sub-fix (c)); `state::hash` (retired automatically by the marshal fix, verified in WF-1B's symptom sweep).

---

## 2. Current state (recon, file:line grounded)

### 2.1 What works (keep)

- **Engine:** comptime blocks/fns execute in a dedicated per-block VM — `execute_comptime` (`crates/shape-vm/src/compiler/comptime.rs:301`), `execute_comptime_with_context` (:334), `execute_comptime_with_target` (:713), `execute_comptime_with_annotation_handler` (:765). Target/ctx injected as pre-set module bindings `__target_arg__`/`__ctx_arg__` via `module_binding_write_kinded` (comptime.rs:991-1016). Watchdog: 5s wall-clock interrupt thread (comptime.rs:1018-1027).
- **`implements(type, trait)`** works — registered via the fixed-arity `register_typed_fn_2` (`comptime_builtins.rs:261`), which derives `arg_kinds` from `FromSlot::NATIVE_KIND` (`marshal.rs:1237`) — the proven-correct registration pattern.
- **`build_config()`** works, including the R2 concrete-named-schema fix (`comptime_builtins.rs:339-350`) — an existing proof that named concrete schemas are the right answer to descriptor identity (§4.3 generalizes it).
- **Directive grammar + application:** `remove target` / `set param` / `set return` / `replace body` / `extend` (directive statements shape.pest:504-527; compiled at statements.rs:587-676; applied in `functions_annotations.rs` directive loop :1408-1585). Citation precision: the `extend` available inside comptime blocks today is the **type-extension statement** (shape.pest:264-266, `extend type_name { documented_method_def* }`) serialized to a JSON-AST payload and applied via `apply_comptime_extend` (functions_annotations.rs:1105-1128, including `extend target { ... }` name substitution) — a *fixed-AST* form. The computed `extend (expr)` items-emission directive is NEW grammar, fully specified in §4.5.7. `replace body` creates an `__original__<name>` shadow function (:1502-1544) — the shadow mechanism is sound; only the call convention is broken (§2.2-D).
- **Annotation chaining** with declaration-order = outermost-first (`compile_chained_annotations`, functions_annotations.rs:1624); target descriptor built by `ComptimeTarget::to_nanboxed` (comptime_target.rs:193-341).
- **`shape expand-comptime`** (bin/shape-cli/src/commands/expand_comptime_cmd.rs, 367 lines) — already at parity with `cargo expand`; keep and extend (§4.7).

### 2.2 Broken root causes

**A. Bool-collapse variadic marshal.** `register_typed_function` (`crates/shape-runtime/src/marshal.rs:2261`) stamps every arg `NativeKind::Bool` at registration (`:2284 arg_kinds: params.iter().map(|_| NativeKind::Bool)`) and at dispatch wraps raw u64 bits as `KindedSlot::new(ValueSlot::from_raw(bits), NativeKind::Bool)` (:2295-2298; async twin :2346, :2350-2353). The comment says "Phase 2c lands proper per-position kind threading" — never landed. Consequences: `error()` prints `[comptime error] <Bool>` (comptime_builtins.rs:325-332), `warning()` silently no-ops (:303-305), `type_info()` falls back to the `__type_info_marshal_pending__` sentinel (:481-492), `state::hash` collapses to a two-value digest (audit §6). **This standing Bool-placeholder is itself the ADR-006 §2.7.8-forbidden Bool-default shape living at the marshal boundary.** Crucially, the caller *already has the true kinds*: `invoke_module_fn_id_stub` (`crates/shape-vm/src/executor/vm_impl/modules.rs:674-718`) receives `args: &[KindedSlot]` sourced from the §2.7.7 stack parallel-kind track, then **flattens them to `Vec<u64>` raw bits at :716** before calling `typed.invoke`. The information is destroyed one call before the body needs it.

**B. Descriptor schema-identity collision.** `ComptimeTarget::to_nanboxed` pre-registers *positional predeclared-Any* schemas by field-name list (comptime_target.rs:200-218: `register_predeclared_any_schema`) and builds rows via `typed_object_from_pairs` (`crates/shape-runtime/src/type_schema/mod.rs:228`). Its resolver `lookup_schema_for_fields` (mod.rs:190-212) does an order-sensitive predeclared lookup, then an **order-insensitive field-SET match across all named schemas in the ambient registry** (:196-206), then auto-registers. The registry is thread/task-scoped (`type_schema/current.rs:45,123`), so the descriptor's `schema_id` — resolved in the *compiler's* ambient registry — is later dereferenced in the *handler VM's* registry, where the same numeric id maps to a different schema (audit: json's `{is_valid, parse, stringify, _3}`), so `field.name` → "Undefined property: name". `ensure_next_schema_id_above` (mod.rs:111) exists precisely because SchemaId numbering is per-registry. Two distinct aliasing mechanisms, both eliminated by §4.3.

**C. `set return` type-check bypass → SIGSEGV.** The `SetReturnType` arm (functions_annotations.rs:1492-1501) rejects only a conflict with an *explicit* annotation; when `return_type` is `None` it stamps the directive's type with no body-vs-signature re-check. `fn answer() { 42 }` + `comptime post { set return string }` reinterprets an int as a string pointer → exit 139 (audit CONFIRMED). The explicit-annotation path is correctly rejected, proving the checker exists and is simply not re-run.

**D. `__original__(args)` garbage.** `replace body` injects `let args = [param1, ...]` with `type_annotation: None` (functions_annotations.rs:1546-1572); calling `__original__(args)` passes one array where N scalars are expected — multi-param is an arity error, single-param silently reinterprets an array pointer as int (audit: `base(5)` → run-varying garbage like `205160873083618`). All 5 examples in content-addressed-bytecode.mdx use this broken convention.

**E. Diagnostics quality.** `warning()` is `eprintln!` with no span (comptime_builtins.rs:304); `error()` produces opaque `[comptime error] ... (line 1)` (:332); comptime VM failures surface internal executor strings ("V3-S5 ckpt-5", "REFUSED ON SIGHT" — live in `executor/objects/array_transform.rs`, `iterator_methods.rs`) directly to users. No comptime stack traces. Nothing flows through LSDS (ADR-006 §9), which is contractually the primary diagnostic format.

**F. `type_info` truncation.** `TypeInfo = {name, kind}` only; `fields: Array<FieldInfo>` deferred on V3-S5 ckpt-5/6 `Array<TypedObject>` carriers (comptime_builtins.rs:432-451). Even post-marshal-fix, `target.fields` is the only field-level API.

### 2.3 Documented contract vs reality

- Book (`shape-web/book/book-site/src/content/docs/advanced/comptime.mdx:140-146`) documents `target.fields` rows as `{name, type, annotations}` — code builds `{name, type, annotations, optional}` (comptime_target.rs:296-301). The `optional` key is undocumented.
- comptime.mdx:149-150: `ctx` is "reserved" — no design exists.
- comptime.mdx builtins table omits `type_info`.
- comptime-annotations-cookbook.mdx: every recipe iterates `target.fields` → all broken by root cause B; :143 notes type-target application (`@ann` on `type`) is v0.4-planned.
- CLAUDE.md claims comptime enables "user-defined LLM integration patterns in stdlib/userland" — zero instances in `crates/shape-runtime/stdlib-src/`.

---

## 3. Constraints (binding, quoted)

1. **CLAUDE.md §Forbidden Patterns** — no `ValueWord` under any rename; no generic opcodes; no dynamic fallback; refusal regex: `(decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|callee|capture) (bridge|probe|helper|hop|translator|adapter|shim)`. "Tags don't exist post-strict-typing." Any marshal design must be describable without those nouns because it *is not* one of those things: kinds are read from where they were stamped, never reconstructed.
2. **ADR-006 §2.7.5 (cross-crate ABI policy):** "stable ABI surfaces (extension contracts, persisted formats, FFI handoffs to non-Rust callers) stay on raw bits + parallel `NativeKind`. **Internal Rust dispatch (trait objects, function pointers, structs, enums) uses `KindedSlot`.**" `TypedInvoke` is an internal Rust trait object — it must carry `KindedSlot`, not `&[u64]`.
3. **ADR-006 §2.7.5 / §2.7.7:** kinds are stamped at compile/registration time and travel on the parallel kind track; "never fabricated from raw bits."
4. **ADR-006 §2.7.8 (Q10):** "no Bool-default … (surface-and-stop with `NotImplemented(SURFACE)` instead)." The variadic Bool-placeholder at marshal.rs:2284/2295 is a standing violation of this rule; it is a **deletion target**, not something to be wrapped.
5. **ADR-006 §2.7.6 (Q8):** KindedSlot API bounded — one constructor + ≤1 scalar accessor per variant; heap dispatch only via `slot.as_heap_value()` + `HeapValue` match (with the documented v2-raw receiver-recovery exceptions already marked in `comptime_builtins.rs:377-411`).
6. **ADR-005 §1 single-discriminator:** no new sum types projecting 1:1 onto `HeapKind`. Descriptor rows stay `TypedObjectStorage` behind `HeapValue`.
7. **No runtime schema synthesis for comptime descriptors.** *Correction of an earlier draft's premise:* `typed_object_from_pairs`'s missing-schema panic (type_schema/mod.rs:230-235) is effectively unreachable today, because `lookup_schema_for_fields` auto-registers an anonymous `FieldType::Any` schema on total miss (mod.rs:208-211, doc-commented "Auto-register an anonymous schema for ad-hoc field sets"). The binding rule this design enforces is therefore stronger than "keep the panic": comptime descriptor construction never reaches *either* the panic *or* the silent Any-synthesis — descriptor schemas are registered deterministically ahead of use (concrete FieldTypes, §4.3) and resolve to the SAME schema in the handler VM. The residual Any auto-registration serving non-comptime callers is a live hazard (a field-tag-poisoning feeder against ADR-006's never-fabricate rule — §2.7.5/§2.7.7 family; the R2 code comment's "§2.7.13/Q14" pointer is imprecise, that section is the RefTarget redesign) recorded in §4.3.7 with a follow-up pointer; its redesign belongs to the workflows that own the wire/const-eval ad-hoc paths.
8. **Strict typing:** no runtime coercion; no `ProofGap` fabrication (`prove_native_kind()` constructor is module-private); typed opcodes require compile-time proof. Directives that change signatures must re-enter the checker, not bypass it.
9. **LSDS (ADR-006 §9)** is the primary diagnostic format: "Terminal, LSP, and MCP renderers consume LSDS … LSDS is the source of truth."
10. **Fix-plan global rules** (fix-plan-2026-07-05-workflows.md §0): surface-and-stop discipline (rule 9); adversarial verify (rule 8); benchmarks untouchable (CLAUDE.md).

---

## 4. Design

### 4.0 One-paragraph shape of the fix

The VM already holds true per-argument kinds at every builtin call site (the §2.7.7 stack kind track feeds `invoke_module_fn_id_stub` with `&[KindedSlot]`). The single wrong move in the entire pipeline is flattening those kinds away at `modules.rs:716` and re-wrapping the bits with `NativeKind::Bool` at `marshal.rs:2295`. The fix is **deletion of the flatten + placeholder**, not addition of machinery: `TypedInvoke` becomes a `&[KindedSlot]` trait object (which §2.7.5 already mandates for internal Rust dispatch), and the variadic registration path loses its Bool stamp entirely. Descriptor identity is fixed the same way `build_config` was fixed in R2: **named concrete schemas, registered deterministically in every registry, resolved by name — never by positional id or field-set inference.** Everything else in this doc (diagnostics, directive safety, showcases) builds on those two deletions.

### 4.1 The introspection contract (stable API v1)

The following shapes are the **normative comptime introspection contract, version 1**. The book tables are kept in sync with this section by hand and gate-checked against the shipped binary (acceptance §7 P4/P13); no table-generation mechanism is claimed or planned for v1.

#### 4.1.1 Target descriptor (`target`, available in `@comptime` annotation handlers and `comptime pre/post` blocks with a target)

```
target: ComptimeTarget {
  kind:        string        // "function" | "type" | "module" | "expression"
                             // | "block" | "await" | "binding"
  name:        string        // per-kind semantics: see table below; "" where unnamed
  doc:         string?       // /// doc comment on the item, verbatim; None if absent
  fields:      Array<FieldDescriptor>   // type targets; empty otherwise
  params:      Array<ParamDescriptor>   // function targets; empty otherwise
  return_type: string?                  // function targets; None if inferred
  annotations: Array<AnnotationDescriptor>  // annotations on the target, declaration order
  captures:    Array<CaptureDescriptor>     // closure-valued binding targets; empty otherwise
}

FieldDescriptor {
  name:        string
  type:        string                   // canonical rendering (§4.1.1b); top-level T? unwrapped to T
  annotations: Array<AnnotationDescriptor>
  optional:    bool                     // true iff declared type was T? at top level
}

ParamDescriptor {
  name:  string
  type:  string                         // canonical rendering (§4.1.1b); top-level T? rendered inline as "T?"
  const: bool
}

AnnotationDescriptor {
  name: string
  args: Array<string>                   // EVALUATED argument values, canonically
                                        // rendered, invocation order (see the
                                        // stringification ruling below)
}

CaptureDescriptor {
  name:    string
  type:    string                       // canonical rendering (§4.1.1b)
  mutable: bool                         // capture is mutated (or captured &mut)
  by_ref:  bool                         // captured by reference, not by value
}
```

Rulings encoded here:

- **`optional` IS part of the v1 contract.** The code already builds it (comptime_target.rs:296-301) and it is load-bearing for the derive showcase (§4.9.1). The book row `{name, type, annotations}` is corrected to `{name, type, annotations, optional}` (open question 2 confirms).
- **Target-level `annotations` is `Array<AnnotationDescriptor>`, not `Array<string>`.** (Revised from the first draft, which kept the current names-only code shape, comptime_target.rs:326-327.) One row shape everywhere — the same `AnnotationDescriptor` used on fields — and it is load-bearing: the `@prompt` showcase (§4.9.2) must read its own invocation argument. **Handler-own-args mechanism (normative):** an annotation handler reads its own arguments by looking itself up by name in `target.annotations` — pure userland, no dedicated builtin. The cookbook ships the three-line helper (`fn own_args(target, name) -> Array<string>`). **Duplicate-application ruling (v1):** applying the same annotation twice to one target is a **compile error** (clean diagnostic naming both application sites) — so `own_args` is unambiguous by construction, never a silent outermost-wins guess. If duplicate application turns out to be wanted (e.g. repeated `@example`), the v2 mechanism is an invocation index on the handler context, not relaxed lookup — open question 12.
- **`args` stringification (normative):** annotation arguments are **comptime-evaluated, then canonically rendered**. Grammar allows full expressions (`annotation_args = expression list`, shape.pest:360-362) and sibling designs require it (`@remote(build_config("WORKER_ADDR"))`, distributed-function-transfer.md §4.1.2); the descriptor carries the **evaluated value**: strings render as their *content* (no surrounding quotes — `placeholders_of` in §4.9.2 consumes content), `int`/`number`/`bool` render as canonical literals, and any argument that is not comptime-evaluable to one of those scalar/string types is a compile error at the application site ("annotation arguments must evaluate at compile time to string/int/number/bool in v1"). Acknowledged v1 limit: `@default(3)` and `@default("3")` render identically — typed annotation args are the v2 companion of type-valued descriptors (folded into open question 4).
- **`doc` is part of v1.** The item's `///` doc comment, verbatim string, `None` if absent. The AST already attaches doc comments to top-level items — `attach_item_doc_comment` covers `Item::Function`, `Item::StructType`, `Item::Trait`, `Item::Enum`, `Item::Module` (crates/shape-ast/src/parser/mod.rs:229-243; grammar `doc_comment? ~ item_core`, shape.pest:25) — so this is descriptor plumbing, not parser work. Load-bearing for `@llm_tool` (tool descriptions derive from docs). There is no separate `doc_of()` builtin — it is a field; the showcases read `target.doc`.
- **`captures` is `Array<CaptureDescriptor>`, not names-only.** Sourcing (corrected from the first draft's overstatement): the comptime descriptor is built **compiler-side** (`comptime_target.rs`), where closure/capture analysis and full surface types are in hand — populating `{name, type, mutable}` needs no blob change. Blob metadata today carries `is_closure`/`captures_count`/`mutable_captures: Vec<bool>` (content_addressed.rs:42-50) but **no per-capture by-ref flag and no per-capture surface types** — `by_ref` is new capture-analysis surfacing (does the captured binding bind a `&`/`&mut` reference), scoped explicitly into S6. It stays in the v1 row because WF-2C's `@remote` comptime pre-flight hard-depends on exactly `{name, type, mutable, by_ref}` (`distributed-function-transfer.md` §4.1.2b); WF-2C's *runtime*-closure-value checks additionally need the flags stamped into blob metadata — that persistence is WF-2C territory, additive `#[serde(default)]`.
- **`type` fields are strings in v1.** Type-valued descriptors (the Zig bar: `field.type` being a type you can pass back to `implements`/`type_info`) are the v2 aspiration; in v1 the string composes with `implements(field.type, "Serialize")` and `type_info(field.type)` because both accept names — this already gives Shape semantic queries Rust proc-macros cannot do. v2 (first-class `type` values) is explicitly deferred and listed as open question 4.
- **Committed additive extension (not in the v1 freeze, delivered with WF-2C):** `required_permissions: Array<string>` on function targets — the target's transitive permission set, sourced from the linker's `required_permissions` union (content-addressed blob metadata). Additive-only evolution (§4.1.4) explicitly permits this; it is recorded here so the WF-2C dependency is a named contract extension, not an ambush. Until it lands, accessing it is an ordinary "no such field" compile error (the key does not exist yet — no sentinel, no empty-array lie). See open question 10.

#### 4.1.1a Per-kind populated keys (normative)

| `kind` | `name` | `doc` | `fields` | `params` | `return_type` | `annotations` | `captures` |
|---|---|---|---|---|---|---|---|
| `"function"` | function name | ✓ | `[]` | ✓ | ✓ (`None` if inferred) | ✓ | ✓ from capture analysis (`[]` for non-capturing functions — all top-level fns; populated for nested/closure-shaped functions that capture) |
| `"type"` | type name | ✓ | ✓ | `[]` | `None` | ✓ | `[]` |
| `"module"` | module path (`std::serde` form) | ✓ | `[]` | `[]` | `None` | ✓ | `[]` |
| `"binding"` | binding name | `None` | `[]` | `[]` | `None` | ✓ | ✓ if the bound value is a closure, else `[]` |
| `"expression"` / `"block"` / `"await"` | `""` (unnamed) | `None` | `[]` | `[]` | `None` | `[]` | `[]` |

(Function-row `captures` revised from the first draft's unconditional `[]`: WF-2C's `@remote` pre-flight (`distributed-function-transfer.md` §4.1.2b) must refuse mutable/by-ref captures **on the annotated function** at compile time — with function-kind captures hard-wired empty, that pre-flight would have had no data source. A top-level fn yields `[]` naturally; the key exists and is truthful for every function kind.)

The internal kind string `await_expr` is renamed to the user-facing `"await"` in the contract; kind strings are plain words, chosen once, frozen. The book table reproduces this matrix verbatim.

#### 4.1.1b Canonical type rendering (normative for every `type`/`return_type` string)

The string is the **canonical Shape surface rendering** of the declared type:

| Declared | Rendered string | Notes |
|---|---|---|
| `int`, `number`, `bool`, `string`, `decimal`, `bigint` | verbatim | surface names, never internal names (`number`, not "Float"/"f64") |
| `Array<int>` | `Array<int>` | no spaces inside `<>` |
| `HashMap<string, Array<int>>` | `HashMap<string, Array<int>>` | single space after comma |
| `(int, string)` | `(int, string)` | tuples |
| `fn(int, string) -> bool` | `fn(int, string) -> bool` | function types |
| `Option<T>` / `T?` at top level of a **field** | `T` with `optional: true` | the one and only unwrapping rule |
| `T?` at top level of a **param/return/capture** | `T?` inline | no `optional` key exists there |
| `T?` **nested** inside a generic (`Array<string?>`) | rendered inline where it appears | `optional` does NOT fire; it is a top-level-of-field flag only |
| unannotated param (type not yet resolved at directive time, §4.5.1) | `_` | documented placeholder. **Honest limit: unannotated param types are not introspectable in v1** — there is no name to hand to `type_info`, and no recovery path. A handler that needs param types must require annotations and `error()` naming the un-annotated param (both showcases do exactly this, §4.9.2) |

This table is frozen with the contract; `json_type_for(field.type)` in the showcases is written against exactly these strings, and a gate test round-trips each row.

#### 4.1.2 `type_info(T) -> TypeInfo`

```
TypeInfo {
  name:   string      // canonical rendering (§4.1.1b): "User", "Array<int>", "number"
  kind:   TypeKind    // enum: Int Number Bool String Decimal BigInt Array
                      //       HashMap Option Result TypedObject TraitObject
                      //       Function Tuple Unit Unresolved
  fields: Array<FieldDescriptor>   // NEW in this design; TypedObject kinds only,
                                   // empty for all other kinds
}
```

- **TypeKind is user-language, not compiler-language** (revised from the first draft, which copied the internal enum verbatim). Two changes vs the current `stdlib-src/core/types.shape` enum (comptime_builtins.rs:435-437): (i) `Float` → **`Number`** — the surface type is `number` and a user who writes `number` everywhere must not have to learn a different word to switch on it; the book table maps every surface name to its kind (`int→Int`, `number→Number`, …). (ii) `TypeVar` + `Unknown` collapse into one **`Unresolved`** kind with defined user semantics: "the queried name did not resolve to a concrete type at this point (e.g. an unsubstituted generic parameter); `name` holds the rendered form (§4.1.1b `_` row)". Compiler-internal distinctions between kinds of unresolvedness are not user-actionable and stay internal. The `types.shape` enum declaration is updated in the same stage under the single-owner lockstep rule (§4.3.6) — the stdlib enum and this contract cannot drift.
- Layout otherwise matches the existing `stdlib-src/core/types.shape` schema (comptime_builtins.rs:430-437), extended with `fields` reusing the **same** `FieldDescriptor` row as `target.fields` — one row shape, one schema, no split introspection story (the stdlib's dormant `FieldInfo {name, type_name}` declaration is replaced by `FieldDescriptor` in the same lockstep stage). `target.fields` remains as the ergonomic accessor for annotation handlers; `type_info(T).fields` is the general query. They are the same rows produced by the same builder.
- **Scope note (named gap, not an accident):** v1 introspection covers **fields and params only** — there is no method/impl/trait-conformance enumeration (Zig's `@typeInfo` decls equivalent). Builder-over-methods and Debug-forwarding derive patterns are therefore out of v1 reach by design; fields+params suffice for the two flagship showcase families (schema derivation, prompt/tool validation). Method/impl introspection is scoped into the contract-v2 discussion together with type-valued descriptors — open question 4 (extended).
- **Sequencing dependency:** `Array<TypedObject>` field carriers are V3-S5 ckpt-5/6 territory (comptime_builtins.rs:444-451). The *contract* (schema + book) lands in WF-1B; if the carrier work has not landed when WF-1B closes, `type_info(...).fields` on a TypedObject **surfaces a clean LSDS error** ("field introspection for `type_info` is not yet available; use `target.fields` in an annotation handler") — surface-and-stop, no empty-array lie, no Bool-default. `target.fields` does not have this dependency (comptime_target.rs already builds object arrays through the v2-raw TypedArray path, :245-274). See open question 3.
- Bare type identifiers keep being rewritten to string literals at call sites (`rewrite_type_info_ident_args`, comptime.rs:254) — unchanged.

#### 4.1.3 Builtin surface (module `__comptime__`, user-facing)

| Builtin | Signature | Status after this design |
|---|---|---|
| `implements` | `(type_name: string, trait_name: string) -> bool` | unchanged (works) |
| `type_info` | `(type_name: string) -> TypeInfo` | fixed by §4.2 + extended per §4.1.2 |
| `build_config` | `() -> BuildConfig {debug, version, target_os, target_arch}` | schema construction migrated per §4.3.6; surface unchanged |
| `build_config` (keyed) | `(key: string) -> string?` | NEW (additive): deterministic build-key window, see below |
| `warning` | `(msg: string) -> ()` | fixed by §4.2, routed per §4.4 |
| `error` | `(msg: string) -> never` | fixed by §4.2, routed per §4.4 |

- **Keyed `build_config(key)`** exists because sibling designs already consume it (`distributed-function-transfer.md` blesses `@remote(build_config("WORKER_ADDR"))` as the non-toy deployment form). Source of keys: a declared `[build.config]` table in `shape.toml` (string→string, checked into the project, hash-tracked — part of the content-addressed input set). NOT environment variables at compile time — that would break deterministic comptime (§4.8) and content-addressed reproducibility. Missing key → `None`. **Resolution mechanism (Shape has no arity/return-type overloading — module registration is name-keyed last-writer-wins, `module_exports.rs:421`):** the two forms are resolved at **compile time by arity**, not at runtime. The compiler rewrites a one-argument `build_config(expr)` call site to the internal registered name `__build_config_key` (registered `(key: string) -> string?`, gated like other `__`-internal emitters but reachable from compiler-rewritten sites — the same reachability class as `__into_*`); a zero-argument call resolves to `build_config` (`() -> BuildConfig`). Precedent for comptime call-site rewriting: `rewrite_type_info_ident_args` (comptime.rs:254). Users see exactly one documented name; any other arity is the ordinary arity error. Ratification of the `[build.config]` surface: open question 11.
- **`never` representation (as required for call-site checking):** `never` in the table is the semantic contract; in v1 the compiler-visible registered return type is `()` (`ConcreteType::Unit`) with the diverging behavior carried by the invoke's `Err` channel — the compiler does not yet model divergence, so code after `error()` is unreachable-but-typechecked, which is sound (never lies about a value) if unrefined; a true bottom type is out of scope. **User-visible consequence (documented, not hidden):** `()`-typed `error()` cannot appear in expression position — `let t = if cond { "T" } else { error("...") }` is a type mismatch in v1 (Zig's `noreturn @compileError` pattern does not port). The book documents the statement-position pattern (guard-`error()` first, then bind — §4.9.2 demonstrates it with a comment), §4.6 carries the failure row, and true `never` typing is the named v2 item.
- All builtins migrate from `params: vec![]` registration to **declared `ModuleParam` schemas** (name + `type_name`). **Scope honesty (S1):** the compiler's call-site checking for module functions is verified for *arity* only (the audit's "expected 2 arg(s), got 3"); per-argument **type** checking against `ModuleParam.type_name` is NOT assumed to exist — S1's scope explicitly includes verifying it and, if absent, adding it (it is the load-bearing premise for deleting the `error()`/`warning()` runtime fallback arms, §4.2.6). If it exists, S1 names the code path in the close-out; if it must be added, it is an ordinary declared-signature check in the existing call-compilation path — no new checker.
- The seven `__emit_*` internal directive emitters stay internal (gated, not user-callable) and keep their fixed-arity registrations.

#### 4.1.4 Compat / versioning stance

- The descriptor row shapes above are **frozen for v1**. Evolution is **additive-only**: new keys may be added in a minor release; existing keys are never renamed, retyped, or removed without a major-version deprecation cycle (documented in the book's comptime chapter under "Stability").
- `build_config()` gains one additive key: `comptime_api: int` (value `1`), so user annotation libraries can feature-gate against future contract revisions without string-parsing `version`.
- The schemas behind the contract are **reserved named schemas** (§4.3): `__ComptimeTarget`, `__ComptimeFieldDescriptor`, `__ComptimeParamDescriptor`, `__ComptimeAnnotationDescriptor`, `__ComptimeCaptureDescriptor`, `TypeInfo`, `__ComptimeBuildConfig`. Reservation is an **explicit `reserved: bool` flag on `TypeSchema` set at registration**, not a `__`-prefix naming convention — `TypeInfo` is user-visible-by-name (users write `let ti: TypeInfo`) and carries no prefix, so prefix-based protection would miss it (§4.3.5 uses the flag). Reserved names are documented as implementation detail; user code never *constructs* them ad-hoc (it uses field access on `target`/`ti`), so renaming *fields* is the only breaking surface — hence the additive-only rule above is sufficient.
- **`TypeInfo` has exactly one registration owner** (§4.3.6): the reserved registration in `builtin_schemas.rs`. The `stdlib-src/core/types.shape` declaration is updated to the identical 3-field shape in the same stage, and registration is idempotent **only for identical field lists** — a name-collision with a different field list is a startup panic (in-tree invariant), never a silent last-writer-wins.

#### 4.1.5 Runtime annotation-hook contract (`before`/`after`): typed `ctx.target` + kinded args (normative)

This section is the surface `distributed-function-transfer.md` §4.1.2/§4.1.2b hard-depends on ("WF-1B's typed-ctx / kinded-annotation-carrier design") and the named deliverable its OQ-12 demands of WF-1B (runtime-hook typed target accessor — surface name, descriptor type, compile-time resolution inside specialized handlers). It was missing from the first draft; it is normative here, and it adopts `ctx.target` — the surface name the distributed doc's OQ-12 proposes. Two distinct contexts, named precisely so sibling docs cite the right one:

1. **Comptime handlers (`@comptime` blocks/handlers)** get the **`target` descriptor binding** (§4.1.1) and the `ctx` compile-context binding (§4.4). There is no `ctx.target` in comptime handlers — reading target metadata in comptime pre-flights is `target.*` (the distributed doc's §4.1.2b already cites the bare `target` descriptor; the two docs agree).
2. **Runtime hooks** — `before(fn, args, ctx)` / `after(fn, args, result, ctx)` (grammar comment shape.pest:368-369; wrapper compilation `compile_annotation_wrapper`, functions_annotations.rs:2035-2060) — are **compiled per application site** (`specialize_annotation_runtime_handlers` → `compile_specialized_annotation_handler`, functions_annotations.rs:1995-2025), so inside a specialized handler the annotated function and its signature are statically known. The v1 contract:
   - **`ctx.target`** is a typed function value statically bound in the specialized handler to the annotated function's original implementation (the same referent as `__original__`, §4.5.5). Calling it is an ordinary typed call. No stringly `ctx["__impl"]` lookup, no `?? args[0]` fallback — if the binding is unavailable that is an annotation-machinery bug and fails loudly at compile time (this is exactly the contract `@remote` requires).
   - **`args` is a specialization-time pack, not a runtime heterogeneous array.** At handler specialization, `args` is bound 1:1 positionally onto the target's declared params; each `args[i]` with a **compile-time-constant index** is a typed access carrying that param's compile-time-proven type/kind (kinds from the §2.7.7 track — never collapsed; the current wrapper's kind-flattening args-array construction is replaced in the same S1/S3 work). Whole-pack forwarding is written by passing the bare `args` name where a call expects the target's parameter list — `ctx.target(args)`, or a compiler-elaborated internal like WF-2C's `__call_raising(addr, ctx.target, args)` — and **elaborates positionally at specialization time** to `f(a0, a1, …, aN)` (no new splat syntax; the pack name is only legal in these elaborated positions, anything else is a compile error naming the rule). This is the "positional elaboration in specialized handlers" the distributed design names as a hard requirement, and it holds precisely because specialization makes arity and per-position types static.
   - **Not in v1:** runtime-computed indexing `args[i]` with a non-constant `i`, and any uniform `Array<T>` view over heterogeneous params — both would require a runtime dynamic carrier (refused; deleted-family). A hook needing a uniform view (logging, capture) generates per-param code from `target.params` in its `@comptime` companion — a cookbook recipe, same mechanism as §4.5.5's forward-all recipe.
   - `result` in `after` carries the target's declared return type; `{ result: v }` short-circuit from `before` is the existing convention (restated in the distributed doc; JIT parity gated by P9/WF-1A(c)).

**Principle: the kinds already exist at the caller; stop destroying them.** No kind is ever derived from raw bits anywhere in this design.

#### Data flow (after)

```
VM stack (Vec<u64> data ∥ Vec<NativeKind> kinds — §2.7.7, stamped at compile time)
        │  pop N args with kinds (existing)
        ▼
invoke_module_fn_id_stub(fn_id, args: &[KindedSlot])          modules.rs:674
        │  [DELETED: raw_bits flatten at modules.rs:716]
        ▼
TypedInvoke  ==  Arc<dyn Fn(&[KindedSlot], &ModuleContext) -> Result<TypedReturn, String>>
        │  (§2.7.5: internal Rust trait objects use KindedSlot)
        ├── fixed-arity wrappers (register_typed_fn_N): check
        │     slots[i].kind() ∈ kind-class(P_i)  (the SAME §4.2.2a class rule
        │     as the variadic path — strict == was the first draft's shape and
        │     is rejected, see 2b) → mismatch is an Err with a clean
        │     diagnostic (was: blind from_slot on trusted bits); then a
        │     kind-directed typed read within the class (2b).
        └── variadic wrappers (register_typed_function): pass &[KindedSlot]
              through UNCHANGED — the body sees true kinds.
              [DELETED: KindedSlot::new(from_raw(bits), NativeKind::Bool) at
               marshal.rs:2295-2298 and the arg_kinds Bool stamp at :2284]
```

Concrete changes:

1. **`TypedInvoke` / `TypedAsyncInvoke` signature change**: `&[u64]` → `&[KindedSlot]` (async: `Vec<u64>` → `Vec<KindedSlot>`). Precise blast radius (corrected citations): the aliases themselves are **private** type aliases in `marshal.rs:1183` / `:1908`; the public ABI carriers are the `TypedModuleFunction.invoke` / `TypedModuleAsyncFunction.invoke` fields (`typed_module_exports.rs:397` / `:423`) — both files change, plus the **13** `register_typed_fn_N` fixed-arity wrappers (`_0` through `_6` + the `_full` variants, marshal.rs:1189-1801) and the direct test callers (`executor/tests/io_integration.rs:112/:412`, `comptime_builtins.rs:806/:822/:872`). This is exactly the migration ADR-006 §2.7.5 prescribes for `ModuleFn`-class internal trait objects. All registration helpers in `marshal.rs` update mechanically; bodies of fixed-arity helpers are unchanged except the added class check + kind-directed read (2b).
2. **`arg_kinds` at variadic registration** (marshal.rs:2284): derived from the declared `ModuleParam.type_name` where params are declared (a total, closed `type_name → kind-class` mapping for the marshal-legal set, see point 2a); for genuinely variadic registrations (empty `params`), `arg_kinds` is empty and per-call kinds come solely from the caller's kind track. **No position ever carries a placeholder kind.** An unmappable `type_name` at registration is a `panic!` at startup (registration is compile-time-of-the-runtime, all callers are in-tree) — not a runtime fallback.

   **2a. Kind-equivalence rule (normative — strict `==` would break real builds; applies to BOTH the variadic and the fixed-arity paths).** `NativeKind` at HEAD has both `String` and `StringV2` (plus `DecimalV2`) as legal string/decimal carriers (`crates/shape-value/src/native_kind.rs:177/:198`; both are v2-raw heap-pointer carriers per the §2.7.5 amendment — and structurally DISTINCT: `Arc<String>` vs manually-refcounted `repr(C)` `StringObj`, per the variant's own parallel-discriminator note). The registration schema cannot know which the compiler stamped at a given call site — `Array<string>` element reads stamp `StringV2` by design, so StringV2 bits reaching a builtin boundary is an ordinary program, not an edge case. The first draft's strict `slots[i].kind() == P_i::NATIVE_KIND` on the fixed-arity path was therefore internally inconsistent with this rule (it would spuriously reject — and `implements`, a fixed-arity two-string registration named in the P12 regression floor, is exactly this shape). Both paths compare **kind classes**:
   - `int` ⇔ {`Int64`} · `number` ⇔ {`Float64`} · `bool` ⇔ {`Bool`} — scalars are exact.
   - `string` ⇔ {`String`, `StringV2`} · `decimal` ⇔ {`DecimalV2`} — the class is the check; per-variant reads are 2b's job.
   - object/array/other heap `type_name`s ⇔ {`Ptr(_)`} — **membership in the Ptr family only**. The dispatch shell does NOT re-derive `HeapKind` granularity from `type_name` (that would duplicate the type system at a runtime boundary); the discriminator for heap payloads remains `slot.as_heap_value()` + `HeapValue` match in the body, per ADR-005 §1.
   - **declared `T?`** ⇔ kind-class(`T`) ∪ {`Null`} ∪ the dedicated nullable carrier where one exists (`NullableFloat64` for `number?` — its in-band NaN sentinel is that carrier's documented contract, native_kind.rs:36/marshal.rs:113-125). Absence travels as the stamped `NativeKind::Null` discriminator (kind IS the signal, bits ignored, per the R5b-2 disposition) — never as a probed bit pattern.
   This is still a comparison of two stamped sources (caller track vs registration schema) — the class widening only acknowledges that the schema is the coarser of the two stamps. It is an internal-consistency assertion, not the type system.

   **2b. Kind-directed typed read within the class (fixed-arity).** `FromSlot::from_slot(bits)` is kind-blind by construction (`Arc<String>`'s impl is pinned to `NativeKind::String`, marshal.rs:135-151 — it cannot read `StringObj` bits, and letting it try would turn the 2a check into silent UB). Fixed-arity params therefore read through a class-aware entry point — `from_kinded(slot: &KindedSlot) -> Result<Self, KindClassMismatch>` — whose impl for each param type **matches on the stamped kind and reads each carrier natively**: `String` → the existing `Arc` clone path; `StringV2` → the `StringObj` content read under the `v2_retain`/`v2_release` discipline (`v2/refcount.rs`); single-carrier scalars have exactly one arm; `Option<T>` maps stamped `Null` → `None`, class(`T`) member → `Some(read)`. **Why this is not forbidden machinery:** the match scrutinee is the *stamped kind from the caller's §2.7.7 track* — never bits; the arms are the same per-variant dispatch the variadic bodies already perform (2a's own text) hoisted into the typed reader; each arm reads its own carrier natively with no structural-equivalence bridging between the two string carriers (the H-c per-carrier-discriminator decision is respected, not papered over). This is choice (b) of the review's three options — chosen over (a) a "call sites only ever stamp String" invariant, which is false at HEAD (`Array<string>` reads), and over (c) paired registrations per carrier, which would double the registration surface and put carrier choice in user-invisible API identity.
3. **`invoke_module_fn_id_stub`** (modules.rs:711-718): the `Typed` arm passes `args` straight through. Where `arg_kinds` is non-empty, the dispatch shell checks `args[i].kind()` against `arg_kinds[i]` **as an assertion of consistency between two stamped sources** (caller track vs registration schema) and surfaces a clean LSDS error on mismatch. This is a check between two pieces of stamped metadata — not decoding, not synthesis.
4. **Async twin** (marshal.rs:2330-2356) gets the identical treatment; `Vec<KindedSlot>` is owned across the await per the existing ownership discipline (KindedSlot's Drop is kind-dispatched per §2.7.7's table).
5. **Extension-facing raw ABI is untouched.** `RawCallableInvoker.invoke` (`module_exports.rs:21`, `unsafe fn(*mut c_void, &u64, &[u64])`) stays raw-bits per §2.7.5's stable-ABI carve-out; the KindedSlot construction for that path already happens inside shape-runtime from the typed registry. This design does not touch the extension contract.
6. **Comptime builtins cleanup:** with true kinds arriving, delete the sentinel/fallback bodies — `__type_info_marshal_pending__` (comptime_builtins.rs:481-492), the `<{:?}>` kind-name fallback in `error()` (:325-332), the silent `if let Some(...)` skip in `warning()` (:303-305). Post-fix, a non-string argument to `error()` is a *compile-time type error at the call site* (declared `ModuleParam` schemas, §4.1.3). **Gating:** this deletion is sequenced AFTER S1's verify-or-add of per-argument call-site type checking (§4.1.3) — if the call-site check did not exist and were not added, a non-string arg would reach dispatch and surface only as the stamped-vs-stamped internal-invariant LSDS error, which fails the §4.4 diagnostics bar; the fallback arms are removed only once the compile-time error is demonstrated by an acceptance probe, not "because they should be dead."

**Retired symptoms (WF-1B symptom sweep validates each):** `error()` message loss, `warning()` no-op, `type_info()` sentinel, `state::hash` two-value digest, `param.const`/descriptor scalar corruption, and every downstream consumer of the variadic path — json/msgpack/toml/yaml/stdlib_time registrations (feed WF-2E) **plus** `executor/builtins/remote_builtins.rs` and `executor/state_builtins/core.rs`, which register through the same `register_typed_function` and sit in S1's behavioral blast radius (their probes belong to WF-2C/WF-2B respectively; S1's module-diff regression scope names them).

#### Why this is not forbidden machinery

The forbidden family is *reconstruction of type information from untyped bits at a boundary* (tag decode, kind synthesis, Bool-default). This design moves in the opposite direction: it **removes** the only point where typed information was being erased and re-guessed. Kinds flow stamped-source → carrier → body with zero re-derivation. The one new runtime comparison (`args[i].kind() == arg_kinds[i]`) compares two independently-stamped values and fails loudly on disagreement — the same shape as the existing fixed-arity `slots.len()` arity check.

### 4.3 Fix architecture B — schema registration that cannot collide

**Principle: descriptor schemas are named, concrete, and pre-registered in every registry that will ever dereference them. Identity by name at construction; ids become registry-local plumbing that never crosses a registry boundary with load-bearing meaning.**

1. **Reserved named schemas with concrete FieldTypes** land in `builtin_schemas.rs` (the same home as the R2 `__ComptimeBuildConfig` fix, comptime_builtins.rs:339-350 precedent), each registered with the `reserved` flag (§4.1.4):
   - `__ComptimeTarget` — `{kind: String, name: String, doc: OptionString, fields: Array, params: Array, return_type: OptionString, annotations: Array, captures: Array}`
   - `__ComptimeFieldDescriptor` — `{name: String, type: String, annotations: Array, optional: Bool}`
   - `__ComptimeParamDescriptor` — `{name: String, type: String, const: Bool}`
   - `__ComptimeAnnotationDescriptor` — `{name: String, args: Array}`
   - `__ComptimeCaptureDescriptor` — `{name: String, type: String, mutable: Bool, by_ref: Bool}`
   - `TypeInfo` — extended with `fields: Array` (single-owner rule, §4.3.6)
   Concrete FieldTypes, **never** `FieldType::Any` — the R2 comment (comptime_builtins.rs:339-350) already documents why Any-schemas poison field-tag sourcing (a `NativeKind` can never be fabricated from `Any` — ADR-006's never-fabricate rule, §2.7.5/§2.7.7 family; the R2 comment's own "§2.7.13/Q14" section pointer is imprecise and not carried forward here).
2. **Registration is part of registry initialization** — every `TypeSchemaRegistry` that can host comptime execution (the compiler's ambient registry AND each comptime/handler VM's bytecode registry) registers these names during setup, exactly where `__ComptimeBuildConfig` is registered today. Registration is idempotent by name.
3. **Construction resolves by name:** new API `typed_object_for_named_schema(schema_name: &str, fields: &[(&str, KindedSlot)]) -> KindedSlot` in `type_schema/mod.rs` alongside `typed_object_from_pairs`. It looks up the schema **by name** in the ambient registry, verifies the field list matches the schema exactly (names + arity; order-normalized to schema order), and constructs. Missing schema or field mismatch = panic with an internal-invariant message (both are in-tree bugs, not user states). `ComptimeTarget::to_nanboxed` (comptime_target.rs:193-341) migrates every `typed_object_from_pairs` call to this; the positional `register_predeclared_any_schema` calls at :200-218 are **deleted**.
4. **Descriptor construction happens inside the handler VM's registry scope.** `to_nanboxed` is invoked under the same `SyncRegistryScope` (type_schema/current.rs) that the handler VM executes under, so the `schema_id` baked into the descriptor's `TypedObjectStorage` is, by construction, an id of the registry that will dereference it. Belt: named-lookup makes ids deterministic per-registry. Suspenders: build-in-the-consumer's-scope makes even the id's registry match. There is no cross-registry id traffic left, hence nothing to "translate" (see §5.3 for the rejected translation-table alternative).
5. **`lookup_schema_for_fields` hardening (targeted, not a rewrite):** the order-insensitive field-SET match over all named schemas (mod.rs:196-206) is a wrong-schema hazard for *any* caller whose field-name set coincides with an unrelated named type. Comptime no longer uses this path at all (point 3). Independently, **both** inference paths (the ordered name-list match, mod.rs:142-154, and the field-set match, mod.rs:196-206) are narrowed to skip schemas carrying the `reserved` flag (§4.1.4 — flag-based, because `TypeInfo` has no `__` prefix and prefix-narrowing would leave it field-set-bindable by any ad-hoc `{name, kind, fields}` object), so ad-hoc object construction can never silently bind to a contract schema or vice versa. Broader redesign of ad-hoc schema inference is out of scope (it serves wire/const-eval paths owned by other workflows) — but see §4.3.7.
6. **`type_info` and `build_config` follow, plus the single-owner lockstep rule:**
   - the `register_predeclared_any_schema(&["kind","name"])` at comptime_builtins.rs:452-455 is deleted; `build_type_info_heap_value` constructs via `typed_object_for_named_schema("TypeInfo", ...)`.
   - **`build_config` migrates too** (revised from the first draft's "unchanged (works)"): its body today reaches `__ComptimeBuildConfig` through `typed_object_from_pairs`' inference chain (the R2 comment at comptime_builtins.rs:339-350 says outright that resolution happens via the order-insensitive field-set match — i.e. the *working* named-schema builtin currently survives on the very lookup path point 5 narrows). S2 moves it to `typed_object_for_named_schema("__ComptimeBuildConfig", ...)` in the same commit as the narrowing, so nothing ever depends on inference order luck.
   - **Lockstep rule for stdlib-visible contract types:** `builtin_schemas.rs` is the sole registration owner for `TypeInfo` (and the `TypeKind` enum shape it references); `stdlib-src/core/types.shape` is updated to the identical declarations in the same stage, with a gate test asserting field-list equality between the two sources at startup. Registration idempotence is defined as identical-field-list only; mismatch = startup panic (§4.1.4).
7. **§4.3.7 Residual hazard note (explicit, with follow-up pointer):** after this design, `lookup_schema_for_fields`' terminal **Any auto-registration** (mod.rs:208-211) remains live for non-comptime callers (wire/const-eval/FFI ad-hoc objects). Every object it synthesizes carries all-`FieldType::Any` fields — the exact field-tag-poisoning feeder (ADR-006 never-fabricate rule, §2.7.5/§2.7.7 family) the R2 fix removed from `build_config`'s path. This design removes every comptime consumer of it and narrows its aliasing surface (point 5) but does not delete it; deletion requires migrating the wire/const-eval producers to declared schemas and is recorded as a named follow-up for the workflow that owns those paths (WF-2E adjacency), cross-referenced in `docs/defections.md` if any implementation-time pressure appears to widen its use.

### 4.4 Diagnostics: quality bar and mechanism

**Bar (calibrated against Zig `@compileError`, Rust `compile_error!`/span-anchored proc-macro diagnostics, Nim `error(msg, node)`):**

1. `error("msg")` fails the build with the user's exact message, a source span, and a comptime trace.
2. `warning("msg")` surfaces as a real compiler warning (LSDS severity `warning`) with a span — visible in terminal output AND LSP squiggles.
3. Any uncaught comptime-VM failure (type error, watchdog timeout, builtin error) is wrapped in a comptime-context diagnostic with a trace — never a bare runtime string.
4. **Zero internal jargon.** No ADR references, checkpoint names, phase names, or refusal-list vocabulary in any user-reachable comptime message.

**Mechanism:**

- **`ComptimeDiagnosticSink`** (new, `crates/shape-vm/src/compiler/comptime.rs`): a `Vec<LsdsDiagnostic>` accumulator owned by the `BytecodeCompiler`, threaded into comptime execution. `ModuleContext` (`crates/shape-runtime/src/module_exports.rs`) gains one optional field `comptime_diagnostics: Option<&ComptimeDiagnosticEmitter>` following the exact precedent of `vm_state: Option<&VmStateSnapshot>` (modules.rs:699-709 shows the construction site). `warning()` emits `severity: warning`; `error()` returns a structured `ComptimeUserError { message }` (carried through the existing `Err(String)` channel as a typed prefix the *driver* strips — the driver, not the renderer, owns formatting).
- **Span provenance (v1 — driver-scoped):** builtins have no span plumbing today, and inventing per-instruction span threading through the marshal is out of proportion. Instead the comptime **driver** (`execute_comptime*`) always knows the span of the construct it is executing — the `comptime` block, the `@annotation` application site, the directive statement (spans exist on all of these in the AST). Every diagnostic emitted during that execution is anchored to that span. This matches the Rust proc-macro baseline (diagnostics point at the macro invocation) and is honest about granularity. **v2 (per-statement spans inside comptime code)** rides on the comptime VM's existing instruction-span table used for runtime error line reporting; it upgrades the *trace* (below), not the anchor, and is scheduled as a Stage-6 polish item, not a contract change.
- **Comptime stack trace:** on any comptime failure, the driver walks the comptime VM's call frames (function names are known; per-function spans exist in the AST registry) and attaches a `comptime_trace` array to the LSDS diagnostic: `[{fn: "derive_row", line: 14}, {fn: "@json_schema handler", line: 3}, {at: "applied to type User", file: "main.shape", line: 7}]`. Terminal renderer prints it indented under the error; LSP shows the anchor span with the trace in the hover/diagnostic body.
- **Jargon firewall:** all comptime-VM failures pass through one wrapping point in the driver before becoming diagnostics. The wrapper puts the internal string into the LSDS `internal` field (machine-visible, renderer-hidden by default; shown under `--verbose-internal-errors`) and sets `message` to a clean user sentence ("this operation is not available in compile-time code" / "compile-time execution exceeded the 5-second limit" / the user's own `error()` text). A gate test greps rendered output of a failing-comptime corpus for the forbidden strings (`ckpt`, `ADR-`, `REFUSED`, `V3-S5`, `§`, `phase-2c`) — acceptance §7 P10. This is a **presentation** firewall on the renderer side of LSDS; the internal strings themselves also get cleaned opportunistically in WF-3B, but the firewall guarantees the bar regardless.
- **`ctx` proposal** (currently "reserved", comptime.mdx:149): v1 exposes `ctx = { module_path: string, file: string }` — read-only compile context. Revised from the first draft's `{module_path, file, build}`: a `ctx.build` key would be a second way to reach what `build_config()` already returns — the exact two-ways-to-one-thing duplication this section refuses for diagnostics, refused for build info too. `build_config()` is the one build-info surface. Diagnostics likewise stay on the `warning()`/`error()` builtins rather than `ctx` methods (uniform with Zig/Nim). Open question 5. (`ctx` here is the *comptime* context; the runtime hook `ctx` with `ctx.target` is §4.1.5 — distinct bindings, distinct docs sections.)
- **Comptime debugging story (v1, one mechanism):** `print()` inside comptime code is routed by the driver to an **LSDS `note` diagnostic** anchored at the driver span, rendered in terminal output during compilation under a `comptime:` prefix — Zig-`@compileLog`-class visibility with zero new API. Mechanically: the comptime VM's output stream is captured by the driver (S4 wires it whether or not today's comptime VM already captures — no reliance on current behavior) and fed through the same `ComptimeDiagnosticSink` plumbing. Inspecting *expansions* (as opposed to execution) stays `shape expand-comptime` / the §4.7.5 lens. The book's comptime chapter gets a "Debugging comptime code" section teaching both (§4.10.1).

### 4.5 Directive type-safety (every directive re-enters the checker)

**Principle: directives edit the AST *before* type checking of the mutated function, never after. There is exactly one body-vs-signature checker and every path goes through it.**

1. **Ordering invariant — whole-program directive pre-pass ("pass 1.5").** The first draft's "recompile the mutated function" rule was too weak: directives fire today interleaved with per-function pass-2 compilation (annotation handling is checked immediately before compiling each function, functions.rs:1095-1105), and `set return`/`set param` change the function's **public** signature — so any *caller* type-checked against the pass-1 registration (or compiled earlier in pass 2) would hold stale types even if the mutated function itself were recompiled. The invariant is therefore program-level: **no function body is compiled until every signature-mutating directive in the program has been applied and the registration table reflects post-directive signatures.** Pipeline: pass 1 (register declared signatures) → **pass 1.5** (execute all comptime blocks / annotation `@comptime` handlers that carry directives, in declaration order across the module — the same defined order annotation chaining already uses; apply `remove target` / `set param` / `set return` to the AST **and** update the registration table as each applies; `replace body` / `extend` run in the same pre-pass for one observable order) → pass 2 (compile all bodies against the final table). Handlers that read other items' signatures observe the table's state at their execution point — declaration-order semantics, book-documented, deterministic. Comptime-callable functions the pre-pass itself needs are compiled on demand by the comptime pipeline as today; the invariant governs the *runtime* program's pass 2. No directive-specific type checker exists; the fix is sequencing (now the right amount of it), so the explicit-annotation path (which today correctly rejects, functions_annotations.rs:1493-1497) and the directive path are *the same path*. Gate: P5's cross-function staleness twins.
2. **`set return`** (root cause C): with (1), `fn answer() { 42 }` + `set return string` compiles in pass 2 with `return_type = Some(string)` and fails in the standard checker exactly as the explicit annotation does. The conflict-with-explicit check (:1493) is kept as the better early error. **Both payload forms are covered:** the type-annotation form AND the expression-payload form `set return (expr)` (`set_return_expr_payload`, shape.pest:520-521; compiled via `emit_comptime_set_return_expr_directive`, statements.rs:650-656) — the expression evaluates in the comptime VM to a **canonical type rendering string** (§4.1.1b), is parsed as a type annotation (parse failure = comptime error naming the directive and the string it produced), and from there is identical to the annotation form: same pre-pass, same registration-table update, same pass-2 re-check.
3. **`set param <name>: type`** — same recompile rule; a body using the param inconsistently with the stamped type is a standard type error.
4. **`set param <name> = expr`** — the injected default literal (functions_annotations.rs:1479-1490) is type-checked against the (declared or inferred) param type by the standard default-value path; a mismatch is a compile error naming the directive.
5. **`replace body` / `__original__`** (root cause D) — the injected `let args = [param1, ...]` (functions_annotations.rs:1546-1572) is **deleted**. New convention:
   - `__original__` is an alias for the shadow function with **the original's exact signature** (the shadow already has it — :1507-1520 clones params and return type). Calling it is an ordinary typed call: `__original__(x, y)`. Wrong arity/types = ordinary compile errors.
   - Generic "forward all arguments" recipes do it in the annotation handler via codegen from `target.params` (the handler knows the param names; `replace body`'s replacement source is *generated* comptime output, so generating `__original__(a, b, c)` with the real names is one interpolation). The cookbook gains this exact recipe.
   - All 5 `content-addressed-bytecode.mdx` examples and the cookbook migrate from `__original__(args)` to direct forwarding. This is a **documented contract break** of a convention that never worked (it returns garbage today, audit §6) — there is nothing working to preserve. Open question 6 ratifies.
   - The `args` name stops being magic; no hidden bindings are injected into user-visible scope at all.
6. **`extend` / `replace module`** — emitted items compile through the normal pipeline already (fixed-AST form via `apply_comptime_extend`, functions_annotations.rs:1105-1128); gate tests pin that. The full v1 **generation surface** — including the new computed `extend (expr)` form both showcases use — is §4.5.7.

#### 4.5.7 The v1 code-generation surface: syntax, escaping, hygiene, error spans (normative)

This is the section §1's non-goals promise. The v1 surface is **source-string emission through the normal parser** — no token templates, no quasi-quote tier, no new escaping language. Everything below is checkable against that one commitment.

**1. Surface inventory.** Four generation constructs, all comptime-directive statements:

| Construct | Payload | Emits | Status |
|---|---|---|---|
| `extend target { method defs }` | fixed AST at parse time (`target` name substituted, functions_annotations.rs:1109-1118) | methods on the target type | exists today; kept for fixed-shape method emission (cannot compute names/bodies) |
| `extend (expr)` | `expr: string` evaluated in the comptime VM | **module items** parsed from the string | NEW (this design; grammar below) |
| `replace body (expr)` | `expr: string` | the mutated function's statements | exists (`parse_function_body_payload`, comptime_builtins.rs:196-212) |
| `replace module (expr)` | `expr: string` | module items | exists (`parse_module_items_payload`, comptime_builtins.rs:214-226) |

**2. Grammar for the computed form.** `extend_items_stmt = { "extend" ~ "(" ~ expression ~ ")" }`, added to the comptime-block statement alternation alongside `replace_body_expr_payload`/`replace_module_expr_payload` (shape.pest:522-525 pattern). Unambiguous against the existing `extend type_name {` statement: `(` is not a `type_name` start, one-token lookahead. Legal in `comptime` blocks with a target and in `@comptime` annotation handlers; the payload expression must have type `string` (ordinary comptime type error otherwise).

**3. Payload semantics.** The string is parsed by the **normal Shape parser** as a sequence of top-level items (the `parse_module_items_payload` path): `fn`, `type`, `enum`, `trait`, `impl`, and `extend TypeName { ... }` type-extension statements are all legal — so generated code can emit free functions, methods (via type-extension), and trait impls through one mechanism. Generated source may **not** itself contain `comptime` blocks, directives, or annotations that carry `@comptime` handlers in v1 — no recursive expansion; expansion terminates by construction (one level, applied in pass 1.5, §4.5.1). Emitted items are inserted at the annotated item's module scope with module-wide visibility and are compiled by the strict pipeline in pass 2 exactly like hand-written source — every type rule, `prove_native_kind`, and capability check applies unchanged.

**4. Interpolation & escaping — there is no new escaping system.** The payload is an ordinary Shape string expression, normally an f-string. The three visually-similar brace uses in the first draft's showcases collapse to two real rules, both pre-existing:
   - `{expr}` inside an f-string is **f-string interpolation**, evaluated in the comptime VM (so `{target.name}` splices the type name, `{string_lit(schema)}` splices a computed literal).
   - Literal braces in generated source are the f-string escapes **`\{` and `\}`** (the existing escape set: `\n \t \r \\ \" \' \0 \{ \} \$ \#` — string_literals.rs:97-132). There is **no `{{`/`}}` escaping in Shape** (the first draft's showcases wrongly assumed it; corrected in §4.9).
   - Embedding a computed string as a *string literal inside generated source* needs quote/escape-correct rendering: the stdlib ships **`std::comptime::string_lit(s: string) -> string`** (renders `s` as a valid Shape string literal, escaping quotes/backslashes/braces). Both showcases use it; hand-rolled quote-juggling is the cookbook's named anti-pattern.

**5. Hygiene (honest statement).** Source-string emission has **no hygiene system** — generated identifiers are ordinary module-scope names. Two rules make this safe and predictable: (a) *deterministic naming convention*: generated items derive names from the target (`{target.name}_json_schema`), documented per annotation; (b) *collisions are loud*: a generated item colliding with a user-written item (or with another generated item) is the ordinary duplicate-definition compile error, upgraded to name **both** definition sites, with the generated one attributed as "generated by `@json_schema` on `User` (main.shape:7)". No gensym in v1; a uniqueness helper is deferred until real usage demands it. Generated code refers to user items by qualified name like any source; capture of user identifiers cannot happen silently because nothing is spliced *into* user scopes — items only *add* to module scope.

**6. Error spans into generated code.** Generated source is registered with the compiler's source map as a **synthetic expansion source** named `<@json_schema on User at main.shape:7>`. Diagnostics from generated code follow the §4.4 driver-anchor rule: the primary span is the **annotation application site** (what the user can act on), and the LSDS diagnostic body carries (i) the failing generated-source excerpt with line/col *within the expansion*, (ii) the comptime trace, (iii) a hint naming `shape expand-comptime` for the full text. This holds for both failure classes: payload **parse** failure ("generated code failed to parse") and pass-2 **type** failure in generated items — a user never sees a bare span into a file they never wrote (§4.6 rows; probe P14). This matches the Rust baseline (errors anchor at the macro invocation, expansion shown as context) and is honest about v1 granularity.

**7. Tooling visibility.** Post-parse, generated items are **ordinary items**: LSP completions, hover, and go-to-definition see them with no new machinery; go-to-definition on a generated name resolves to the annotation application site (its anchor span). `shape expand-comptime` renders the full generated source (existing renderer, §4.7.5 lens reuses it).

**8. Determinism & content addressing.** Expansion runs exactly once per application site per compilation (pass 1.5); generated items' content is a pure function of source + annotation args + `[build.config]` (all hash-tracked inputs), so content-addressed function hashes remain reproducible (§4.8's determinism stance is what makes this true).

### 4.6 Error paths (unified)

| Failure | Surface (after) |
|---|---|
| `error("msg")` in comptime | LSDS error, message verbatim, span = comptime construct, comptime trace attached; build fails; exit 1 |
| `warning("msg")` | LSDS warning, span, build continues; visible in terminal + LSP |
| Type error inside comptime code | Standard LSDS type error + comptime trace ("during compile-time evaluation of …") |
| Watchdog (5s) | LSDS error "compile-time execution exceeded the 5-second limit", span + trace |
| Directive on missing param/target | LSDS error naming the directive and target (exists today as strings :1434-1437, :1459-1462; upgraded to LSDS with spans) |
| Post-directive signature violation | Standard body-vs-signature LSDS error, notes "return type set by comptime directive at <span>" |
| Kind mismatch at builtin dispatch (stamped-vs-stamped disagreement) | LSDS internal-invariant error with `internal` detail; never silent, never reinterpreted |
| Comptime calls an I/O builtin | LSDS error "not permitted at compile time" (§4.8) |
| Generated source fails to **parse** | LSDS error anchored at the annotation application site: "generated code failed to parse", with the failing generated-source excerpt + line/col within the expansion, comptime trace, and a `shape expand-comptime` hint (§4.5.7.6); probe P14 |
| Generated code fails **type check** (pass 2) | Standard LSDS type error, anchored at the application site, body carries the generated-source excerpt + expansion line/col (§4.5.7.6) — never a bare span into source the user never wrote; probe P14 |
| Generated item name collision (vs user item or another generated item) | Duplicate-definition LSDS error naming **both** definition sites; the generated one attributed "generated by `@ann` on `X` (file:line)" (§4.5.7.5) |
| Annotation applied outside its declared `targets:` list | LSDS error at the application site naming the annotation, the actual target kind, and the declared list (validation exists today; upgraded to LSDS with span) |
| Same annotation applied twice to one target | LSDS error naming both application sites (v1 duplicate-application ruling, §4.1.1; open question 12) |
| `error()` in expression position | Ordinary type error (`error()` is `()`-typed in v1, §4.1.3); message suggests the statement-position guard pattern; true `never` typing is the named v2 item |

### 4.7 Ergonomics upgrades ranked by leverage

1. **Diagnostics (§4.4)** — highest leverage; every comptime user hits errors before they hit features. (WF-1B)
2. **Introspection contract fixed + honest (§4.1-4.3)** — unblocks every documented recipe. (WF-1B)
3. **`__original__` typed convention (§4.5.5)** — unblocks the whole wrapper/AOP pattern family. (WF-1B)
4. **`type_info(T).fields` (§4.1.2)** — closes the gap to Zig `std.meta.fields`; derive patterns stop needing an annotation target to see fields. (WF-1B contract; carrier-dependent implementation)
5. **Expansion inspection in the editor:** LSP code lens "▸ show comptime expansion" on annotated/`comptime`-bearing items, reusing `expand-comptime`'s renderer (`expand_comptime_cmd.rs` already renders per-function via `--function`); plus hover on an `@annotation` name showing the annotation's doc comment + target kinds. CLI parity exists today; this is the discoverability multiplier. (WF-3B or follow-up; open question 7)
6. **Discoverability:** the five builtins get doc comments served through the existing LSP hover pipeline; `shape expand-comptime --list` enumerates expandable items; book chapter restructure (§4.10). (WF-3B + book)
7. **v2 trace granularity** (per-statement comptime trace lines, §4.4) — polish. (follow-up)

### 4.8 Permission story

- **Comptime execution is deterministic and I/O-free.** The comptime VM runs with an **empty `PermissionSet`** — no `FsRead/FsWrite/NetConnect/NetListen/Process/Env`; `Time` only for the watchdog's own accounting (not exposed); `Random` denied. `build_config()` is the sanctioned window onto the build environment. Any stdlib call requiring a permission fails with the clean "not permitted at compile time" diagnostic. Rationale: reproducible builds, no compile-time network surprises, and content-addressed bytecode hashes stay a pure function of source. This aligns with the runtime permission architecture (16-permission enum, `shape-abi-v1/src/lib.rs:996`) rather than inventing a parallel comptime capability system. **Note:** today enforcement is inert program-wide (audit §4.2, WF-1D territory); comptime denial rides WF-1D's `check_permission` wiring by constructing the comptime VM's `ModuleContext` with `granted_permissions: Some(EMPTY)` instead of the current `None`-means-allow-all (modules.rs:705). One-line dependency on WF-1D's semantics, sequenced accordingly (§6).
- **LLM/network patterns are runtime patterns.** The LLM showcase (§4.9.2) does *comptime codegen* (schemas, prompt validation) and *runtime execution* (HTTP call under `NetConnect`). Nothing in the showcase calls the network at compile time. Open question 8 confirms this boundary as policy.

### 4.9 Stdlib showcases

Both live in `crates/shape-runtime/stdlib-src/`, are documented as book recipes, and run in the book gate under vm AND jit. Both are written against contract v1 only — no private hooks.

#### 4.9.1 Derive-style schema/serialization: `@json_schema` + `@to_json`

Primary form (requires type-target annotation application — currently parsed and validated via `targets:` but application on `type` is v0.4-planned per cookbook:143; open question 1 recommends pulling it into scope because every derive story needs it):

All showcase code is written against contract v1 + §4.5.7 only. Helpers named below are **plain userland comptime-callable functions defined in the same stdlib file** (signatures listed so "no private hooks" is checkable): `json_prop(name: string, jtype: string, desc: string) -> string`, `json_type_for(shape_type: string) -> string` (maps §4.1.1b canonical strings — including inline `T?` — to JSON-schema type strings; `error()`s naming the field/param for `"_"` and unsupported types), `json_object_schema(name: string, props: Array<string>, required: Array<string>) -> string`, `emit_field_serializer(name: string, shape_type: string, optional: bool) -> string` (returns generated Shape statements as source text), `json_serializer_body(parts: Array<string>) -> string` (wraps per-field statements in the string-builder scaffold, owning all literal-brace escaping), plus `std::comptime::string_lit` (§4.5.7.4).

```shape
// stdlib-src/serde/derive.shape  (namespace: std::serde)

@annotation json_schema {
    targets: [type]
    @comptime {
        let mut props: Array<string> = []
        let mut required: Array<string> = []
        for field in target.fields {
            let mut desc = ""
            for ann in field.annotations {
                if ann.name == "description" { desc = ann.args[0] }
            }
            props.push(json_prop(field.name, json_type_for(field.type), desc))
            if !field.optional { required.push(f"\"{field.name}\"") }
        }
        let schema = json_object_schema(target.name, props, required)
        // §4.5.7 computed form: {…} = f-string interpolation (comptime values),
        // \{ \} = literal braces in the generated source.
        extend (f"fn {target.name}_json_schema() -> string \{ {string_lit(schema)} \}")
    }
}

@annotation to_json {
    targets: [type]
    @comptime {
        let mut parts: Array<string> = []
        for field in target.fields {
            parts.push(emit_field_serializer(field.name, field.type, field.optional))
        }
        let body = json_serializer_body(parts)
        // Method emission: generated source contains an `extend User { … }`
        // type-extension item — users get `user.to_json()` with normal
        // method dispatch and LSP completion (§4.5.7.7).
        extend (f"extend {target.name} \{ fn to_json(self) -> string \{ {body} \} \}")
    }
}
```

Usage (book example, gate-runnable, hermetic):

```shape
@json_schema @to_json
type User {
    @description("Unique identifier") id: int,
    name: string,
    email: string?,
}

fn main() {
    print(User_json_schema())
    let u = User { id: 1, name: "Ada", email: None }
    print(u.to_json())
}
```

Exercises: `target.fields` incl. `optional` + `annotations` + evaluated `args` (contract v1), the §4.5.7 computed `extend (expr)` form with interpolated names, **method emission** via a generated type-extension item (so the generated surface is `u.to_json()`, not only name-concatenated free functions), and strict-typed generated code (the serializer is fully typed; a field type the deriver cannot handle is a comptime `error()` naming the field — demonstrating the diagnostics bar). `User_json_schema()` stays a free function deliberately: it has no receiver, and trait-level `JsonSchema`-style static dispatch is the named v2 shape (§5 item 12). **Fallback form if the user defers type-target application to v0.4:** the same generators exposed as comptime functions driven by `type_info(User).fields` inside a `comptime` block — same contract surface, worse ergonomics; the doc recommends against shipping only the fallback.

#### 4.9.2 LLM-integration pattern: `@llm_tool` + comptime-validated prompts (`std::llm`)

Fulfills the CLAUDE.md claim with a real, idiomatic pattern: **the type system writes your tool schemas** — the part of LLM integration that is genuinely comptime-shaped.

Helpers (userland, same file; `json_prop`/`json_type_for` shared from `std::serde`): `tool_schema(name: string, desc: string, props: Array<string>) -> string`, `placeholders_of(template: string) -> Array<string>`, `own_args(target, name) -> Array<string>` (the §4.1.1 three-line contract helper), `string_lit` (§4.5.7.4).

```shape
// stdlib-src/llm/tools.shape  (namespace: std::llm)

@annotation llm_tool {
    targets: [function]
    @comptime {
        // Tool description comes from the /// doc comment (target.doc — §4.1.1;
        // there is no doc_of() builtin). Missing pieces fail loudly with the
        // exact remediation, demonstrating the §4.4 diagnostics bar.
        if target.doc == None {
            error(f"@llm_tool requires a /// doc comment on '{target.name}' — it becomes the tool description")
        }
        let desc = target.doc ?? ""   // unreachable fallback: error() above ends the build
                                      // (v1 error() is ()-typed, §4.1.3 — statement-position pattern)
        if target.return_type == None {
            error(f"@llm_tool requires an explicit return type on '{target.name}'")
        }
        let mut props: Array<string> = []
        for p in target.params {
            if p.type == "_" {
                error(f"@llm_tool requires a type annotation on parameter '{p.name}' of '{target.name}' (§4.1.1b: unannotated params are not introspectable)")
            }
            props.push(json_prop(p.name, json_type_for(p.type), ""))
        }
        let schema = tool_schema(target.name, desc, props)
        extend (f"fn {target.name}_tool_def() -> string \{ {string_lit(schema)} \}")
    }
}

// Comptime-validated prompt templates: placeholders checked against params.
@annotation prompt {
    targets: [function]
    @comptime {
        // own_args (§4.1.1): this annotation's own evaluated args from
        // target.annotations — unambiguous because duplicate application
        // of one annotation is a v1 compile error.
        let template = own_args(target, "prompt")[0]
        for ph in placeholders_of(template) {
            let mut found = false
            for p in target.params { if p.name == ph { found = true } }
            if !found {
                error(f"prompt placeholder '\{{ph}\}' has no matching parameter on '{target.name}'")
            }
        }
    }
}
```

Usage:

```shape
/// Get current weather for a city
@llm_tool
fn get_weather(city: string, units: string) -> string {
    f"\{\"city\": \"{city}\", \"temp_c\": 21\}"   // runtime impl; http lives here
                                                  // (\{ \} = literal braces, §4.5.7.4)
}

@prompt("Summarize the weather in {city} for a {audience} audience")
fn weather_prompt(city: string, audience: string) -> string { ... }

fn main() {
    // Book-gate form is hermetic: print the generated tool definition.
    print(get_weather_tool_def())
}
```

- Compile-time value demonstrated: signature→schema derivation (no drift between the function and what the model is told), prompt-placeholder validation (`{audence}` typo = compile error with span), `error()` with a great message when the contract is violated.
- Runtime value (documented, not gate-required until WF-2E's http lands): `std::llm::call(endpoint, tools: [get_weather_tool_def()], ...)` under `NetConnect` — a thin userland client over `std::http`, explicitly *userland-implementable*, which is the CLAUDE.md claim verbatim.
- No LLM call at compile time, ever (§4.8).

### 4.10 Book chapter plan

1. **`advanced/comptime.mdx` — rewrite around contract v1:** normative descriptor tables (incl. `optional`), the five builtins (add `type_info`), `ctx` v1, the permission stance ("comptime is deterministic"), the stability/versioning section (§4.1.4), the generation surface incl. `\{`/`\}` escaping and `string_lit` (§4.5.7), and a **"Debugging comptime code"** section (comptime `print()` → LSDS notes + `expand-comptime`, §4.4). Every example gate-runnable.
2. **`advanced/annotations.mdx` — update:** `__original__` typed convention with the forward-from-`target.params` recipe; the runtime-hook contract (`ctx.target`, args pack — §4.1.5); the duplicate-application rule; diagnostics section showing a real `error()` output with span + trace, plus the `error()`-is-statement-position note.
3. **`advanced/comptime-annotations-cookbook.mdx` — re-verify every recipe** against the fixed contract (all currently broken by root cause B); add: derive walkthrough (§4.9.1), forward-all-args, comptime-validated prompts, `own_args`, `string_lit`.
4. **NEW `advanced/comptime-llm-patterns.mdx`:** the `std::llm` showcase — tool derivation, prompt validation, and the comptime/runtime boundary ("your build never calls a model").
5. **`__original__(args)`-convention migration, grep-audited at implementation time (not "5 examples"):** `advanced/content-addressed-bytecode.mdx` (direct call sites + `state::capture_call(__original__, args)` forms), `advanced/transport-layer.mdx`, `advanced/wire-protocol.mdx`, and `stdlib/core/state.mdx` — all four files carry the broken convention today.
6. **Diagnostics appendix:** comptime error anatomy (span, trace, LSDS ids) in the errors chapter.
7. All chapters enter the book truth-gate (fix-plan rule 7); the WF-1B close gate requires the adv-comptime chunk green.

---

## 5. Alternatives considered & rejected

1. **Keep the Bool placeholder, have bodies self-interpret bits per their own contract** (the current marshal.rs:2288-2299 comment's implied posture). REJECTED: this *is* the §2.7.8-forbidden Bool-default; bodies "interpreting slot bits per their own contract" is kind-from-raw-bits under a politeness rename. It is the audit's root cause, not a design option.
2. **Per-position kind re-derivation at dispatch from `ModuleParam.type_name` alone** (dispatcher builds kinds from strings at call time, ignoring the caller's kind track). REJECTED: for variadic/optional positions the schema is incomplete, tempting an `unwrap_or(Bool)` — the same violation reborn; and it invents a second kind source that can drift from the stamped one. The kind track is the single runtime source; the schema is a registration-time cross-check only (§4.2.3).
3. **Schema-id translation table at the VM boundary** (map compiler-registry ids → handler-registry ids when descriptors cross). REJECTED on sight: this is "boundary translation" per CLAUDE.md §Renames-to-refuse — a permanent conversion layer where a construction-site fix (build in the consumer's registry scope, resolve by name) removes the boundary crossing entirely. A translation table would also silently mask any future registry-scoping bug instead of failing loudly.
4. **Fix collision by making the order-insensitive field-set matcher "smarter"** (e.g. prefer predeclared over named, or hash field types too). REJECTED: keeps identity inferential. Any inference over field names can alias; named resolution cannot. (The narrowing in §4.3.5 is a hazard-reduction for *other* callers, not comptime's identity mechanism.)
5. **`HashMap<string, ...>`-shaped descriptors** ("just make target a dynamic map, sidestep schemas"). REJECTED: Shape has no `any` type; a heterogeneous map value would need exactly the dynamic carrier the language deleted. TypedObject rows with concrete schemas are the strict-typing answer.
6. **A new `ComptimeValue` sum type as the marshal carrier** (variants mirroring HeapKind for descriptor payloads). REJECTED: ADR-005 §1 single-discriminator — a parallel discriminator projecting 1:1 onto HeapKind is the named defection-attractor; `KindedSlot` + `HeapValue` dispatch already covers it.
7. **Keep `__original__(args)` and make the VM unpack the array at the call site** (runtime arity adaptation). REJECTED: runtime unpacking of an untyped array into typed params is dynamic dispatch at a call boundary — deleted-family behavior — and the convention is unfixable within strict typing (the array's element kinds vs param kinds cannot be proven at the wrapper's compile time in general). Direct typed forwarding costs one interpolation in generated code and is fully checked.
8. **A directive-specific "re-check return type" mini-checker** bolted onto the `SetReturnType` arm. REJECTED: two checkers drift; the sequencing fix (§4.5.1) reuses the one true pipeline and closes the same hole for `set param` for free.
9. **Per-builtin span parameters** (`error(msg, line)` Nim-style overload). REJECTED for v1: pushes span bookkeeping onto users; driver-scoped anchoring (§4.4) matches the proc-macro baseline without API surface. Revisit only if v2 trace granularity proves insufficient.
10. **Compile-time LLM calls in the showcase** (comptime code hitting a model endpoint to generate code). REJECTED: violates the deterministic-comptime permission stance (§4.8), makes builds non-reproducible and network-dependent, and would poison content-addressed hashes. The comptime side of LLM integration is schema/prompt derivation; calls are runtime.
11. **Strict `==` kind equality on the fixed-arity marshal path** (the first draft's §4.2.1 shape). REJECTED by this revision as internally inconsistent with §4.2.2a's own premise: `String`/`StringV2` are both legal compiler-stamped string carriers, so strict equality spuriously rejects real programs (`implements` — fixed-arity, two string params, P12 regression floor — is exactly the shape). Superseded by class-aware checks + kind-directed reads (§4.2.2a/2b). The sibling options — (a) a "call sites only stamp `String`" invariant (false at HEAD: `Array<string>` element reads stamp `StringV2` by design) and (c) paired per-carrier registrations (doubles registration surface, leaks carrier identity into API identity) — are rejected in §4.2.2b.
12. **Token-template / quasi-quote generation surface for v1** (hygienic splices, `{schema}`-style template holes as their own syntax). REJECTED for v1 per §1 non-goals; §4.5.7 commits to source-string emission through the normal parser precisely so there is *no* third brace semantics — braces in a payload are ordinary f-string interpolation or `\{`-escaped literals, nothing else. Revisit as a v2 tier only with real usage evidence. Relatedly, **method/trait emission is NOT foreclosed by the string surface**: generated source parses as module items, so `extend Type { … }` methods (used by the §4.9.1 showcase — `u.to_json()`) and full `impl Trait for Type` items already work; the remaining free function (`User_json_schema()`) is free because it has no receiver — trait-static dispatch (`User::json_schema()`-style) is the named v2 shape, not a v1 gap in the generation surface.
13. **Outermost-wins `own_args` under duplicate annotation application** (the first draft's shrug). REJECTED: it silently validates the wrong invocation (`@prompt("A") @prompt("B")` checks "A" twice). v1 makes duplicate application of one annotation a compile error (§4.1.1); the v2 alternative if duplicates become desirable is an invocation index on the handler context — open question 12.
14. **`ctx.build` alongside `build_config()`.** REJECTED (revision): two surfaces for one datum is the duplication §4.4 refuses for diagnostics; `build_config()` is the single build-info window (§4.4, OQ5 revised).

**Review rebuttals (2026-07-05 adversarial review — findings judged wrong or overstated; recorded here instead of design changes):**

- *"`CaptureDescriptor` requires new blob-metadata work (per-capture types/by-ref not in the blob)."* Partially wrong: the comptime descriptor is built **compiler-side** (`comptime_target.rs`) where closure analysis and full surface types are already in hand — `{name, type, mutable}` needs no blob change at all. What IS new work is the per-capture **by-ref analysis surfacing** (scoped into S6, §4.1.1) and blob **persistence** for WF-2C's runtime-closure-value checks (WF-2C territory, additive). The first draft's "already stamped into blob metadata" was the real error and is corrected.
- *"The doc must confirm top-level fn/type items carry doc comments in the AST or scope the work."* Resolved by fact, not scope: `attach_item_doc_comment` already attaches `///` docs to `Item::Function`, `Item::StructType`, `Item::Trait`, `Item::Enum`, `Item::Module` (crates/shape-ast/src/parser/mod.rs:229-243; grammar shape.pest:25). `target.doc` is descriptor plumbing only.
- *"Nothing addresses whether generated names appear in LSP completions/go-to-definition."* No new machinery is needed and none is designed: generated items are ordinary items after parse (§4.5.7.7); completions/hover see them exactly like hand-written items, and go-to-definition resolves to the annotation application site via the expansion anchor span. The lens (§4.7.5) is *additional* inspection, not the only discoverability path.

Implementation-time compromises, if any arise, are logged in `docs/defections.md` per CLAUDE.md.

---

## 6. Implementation plan sketch (ordered, mergeable stages → fix-plan workflows)

All stages: `direnv exec /home/dev/dev/shape-lang <cmd>`; pre-created pinned worktrees; `bash scripts/verify-merge.sh` + `just check-clean` + `just test` per merge; blast-radius module diffs for regression scope (fix-plan §0).

| Stage | Content | Workflow phase | Depends on |
|---|---|---|---|
| **S1** | Marshal fix (§4.2): `TypedInvoke`/`TypedAsyncInvoke` → `&[KindedSlot]`; delete Bool stamp (marshal.rs:2284, :2295-2298, :2346, :2350-2353) + raw-bits flatten (modules.rs:716); `arg_kinds` from `ModuleParam`; class-aware fixed-arity checks + `from_kinded` kind-directed reads (§4.2.2a/2b incl. `T?`/`Null` rows); migrate the 5 comptime builtins to declared params; delete sentinel/fallback arms. Behavioral blast radius includes `remote_builtins.rs` + `state_builtins/core.rs` (same variadic path; named in the module-diff regression scope) | WF-1B `RootFix` (agent 1) | — |
| **S2** | Schema identity fix (§4.3): reserved named concrete schemas; `typed_object_for_named_schema`; migrate `to_nanboxed` + `type_info`; build under handler-VM registry scope; field-set-match reserved-prefix narrowing | WF-1B `RootFix` (agent 2, sequential — same files) | S1 |
| **S3** | Directive safety + generation surface (§4.5): whole-program directive pre-pass ("pass 1.5", §4.5.1); `set return` closes (both payload forms); `__original__` typed convention (delete args injection); NEW `extend (expr)` computed-generation directive + expansion source-map anchoring + `string_lit` helper (§4.5.7); runtime-hook args-pack/`ctx.target` specialization contract (§4.1.5) | WF-1B `TypeCheck` + the 4th lane (fix-plan WF-1B note) | S1 |
| **S4** | Diagnostics core (§4.4): `ComptimeDiagnosticSink`, `ModuleContext.comptime_diagnostics`, LSDS routing for `error()`/`warning()`, driver span anchoring, jargon firewall, comptime trace v1, comptime `print()`→LSDS-note routing; **plus a machine-readable diagnostics CLI surface** (`--diagnostics json` emitting LSDS JSON — none exists in shape-cli today; shape-diagnostics types are already `Serialize`, so this is flag + serializer wiring). P1/P2/P14 observe through it | WF-1B (new lane per the WF-1B ergonomics amendment) | S1 |
| **S5** | Symptom sweep: parallel refuters on all 9 audit symptoms (error/warning/type_info/target.fields/field.name/state::hash/param.const/`__original__`/descriptor keys), each reproducing the original audit probe vm+jit | WF-1B `SymptomSweep` | S1–S4 |
| **S6** | Contract v1 packaging: `TypeInfo.fields` schema + builder (or clean SURFACE if V3-S5 carriers absent, per §4.1.2); `build_config().comptime_api = 1`; keyed `build_config` arity rewrite → `__build_config_key` + `[build.config]` hash-tracked table (§4.1.3, OQ11); `ctx` v1; per-capture **by-ref analysis surfacing** for `CaptureDescriptor` (§4.1.1); duplicate-annotation compile error (§4.1.1); annotation-args evaluated-value stringification (§4.1.1) | WF-1B close | S2, S3; V3-S5 for fields payload |
| **S7** | Comptime permission stance (empty grant set for comptime VMs) | WF-1B tail, semantics via WF-1D | WF-1D `check_permission` wiring |
| **S8** | Showcases (§4.9): `std::serde` derive + `std::llm` tools/prompts + gate examples | WF-1B close gate (the amendment's "two polished stdlib showcases") | S1–S6; type-target application ruling (OQ 1) |
| **S9** | Book plan (§4.10) + jargon string cleanup at the sources | WF-4 book gate; jargon overlap with WF-3B | S1–S8 |
| **S10** | LSP expansion lens + hover discoverability (§4.7.5-6) | WF-3B or follow-up (OQ 7) | S1–S6 |

Downstream unblocking (unchanged from fix-plan): S1 unblocks WF-2E (json/msgpack/toml/yaml/stdlib_time variadic registrations) and WF-2B's `state::hash`. The annotation-hooks-under-JIT fix stays WF-1A(c); acceptance P9 below is the shared gate.

---

## 7. Acceptance tests (e2e probes; every probe runs `--mode vm` AND `--mode jit` unless marked compile-only)

- **P1 `error()` fidelity (compile-only):** `comptime { error("field X needs a type") }` → exit ≠ 0; stderr contains the exact message; the S4 `--diagnostics json` surface (the LSDS observation mechanism — in S4's scope precisely because no machine-readable diagnostics output exists in shape-cli today) emits LSDS JSON with `severity: "error"`, a `location` with the real file/line of the comptime block, and a non-empty `comptime_trace`. NOT present: `<Bool>`, `(line 1)`.
- **P2 `warning()` surfaces (compile-only):** `comptime { warning("deprecated shape") }` → build succeeds; LSDS warning with message + span emitted; visible in terminal output.
- **P3 `type_info`:** `comptime { let ti = type_info(User) ... }` asserts `ti.name == "User"`, `ti.kind == TypeKind::TypedObject`; and (post-carrier) `ti.fields[0].name == "id"`; pre-carrier, `.fields` access yields the clean SURFACE diagnostic, never an empty array, never the sentinel string.
- **P4 descriptor integrity under collision pressure:** a program that imports `json` (the audit's colliding registry neighbor) AND uses an annotation handler iterating `target.fields`, asserting exact `{name, type, annotations, optional}` keys/values for a 3-field type with one `@description` and one `T?` field. Includes `param.const` on a `const` param. (Reproduces + inverts the audit's `{is_valid, parse, stringify, _3}` corruption.)
- **P5 `set return` soundness (compile-only):** the audit probe `fn answer() { 42 }` + `comptime post { set return string }` → **compile error** naming the body/signature conflict and the directive site; exit code is the compile-error code, never 139. Twin probe: a *consistent* `set return int` compiles and runs green. **Cross-function staleness twins (§4.5.1):** a caller `G` of directive-mutated `F`, once declared *before* `F` in source and once *after* — in both orders `G` type-checks against the **post-directive** signature (a `G` written against the pre-directive type is a compile error; a `G` written against the post-directive type runs green). Plus the expr-payload twin: `set return ("string")` behaves identically to `set return string`.
- **P6 `__original__` forwarding:** the content-addressed-bytecode.mdx `base(5)` scenario under the new convention returns exactly `12`, both modes, 20 consecutive runs byte-identical (kills run-varying pointer garbage). Multi-param forward probe: 3-param function, wrapper forwards via generated `__original__(a, b, c)`, asserts result AND that a wrong-arity forward is a compile error.
- **P7 `state::hash` distinctness:** ≥ 3 distinct inputs → 3 distinct digests; equal inputs → equal digests (downstream symptom of S1; shared gate with WF-2B).
- **P8 showcase gates:** §4.9.1 `User_json_schema()` / `u.to_json()` (generated **method** via type-extension emission) and §4.9.2 `get_weather_tool_def()` print exact expected JSON, vm+jit, byte-identical across modes; the `@prompt` typo probe (`{audence}`) is a compile error whose message names the placeholder and the function; the `@llm_tool` guard probes (missing `///` doc, missing return type, unannotated param) each produce the exact §4.9.2 `error()` message.
- **P9 wrappers under JIT (shared with WF-1A(c)):** the audit's before-hook short-circuit probe returns `99` under BOTH modes at a call count that provably triggers JIT compilation. WF-1B's showcases do not close while this diverges.
- **P10 jargon firewall (compile-only, corpus):** a corpus of ≥ 10 failing-comptime programs (type errors, watchdog, bad directives, missing schema ops); rendered stderr for all must contain none of: `ckpt`, `ADR-`, `V3-S5`, `REFUSED`, `§`, `phase-2c`, `SNAPSHOT_FUTURE`, `NotImplemented(SURFACE`. Machine-grep, part of the merge gate.
- **P11 comptime purity:** `comptime { file::read_text("/etc/hostname") }` → clean "not permitted at compile time" compile error (post-WF-1D semantics; pre-WF-1D, test is written and marked blocked-on-WF-1D, not skipped silently).
- **P12 regression floor:** `implements` and `build_config` probes stay green (they work today — the fix must not regress the working half); `shape expand-comptime` output for a directive-bearing module is stable and reflects the post-S3 `__original__` convention.
- **P13 book gate:** adv-comptime chunk (12/15 at audit) → 15/15; the two showcase examples added to the gate corpus pass vm+jit.
- **P14 generated-code failure paths (compile-only):** (a) an annotation whose `extend (expr)` payload is not valid Shape → LSDS error anchored at the application site, containing the generated-source excerpt with expansion line/col and the `expand-comptime` hint — never a raw parser error into an unnamed buffer; (b) a payload that parses but fails pass-2 type checking → standard LSDS type error with the same application-site anchor + excerpt; (c) a generated item colliding with a user-written name → duplicate-definition error naming both sites with the "generated by @ann on X (file:line)" attribution (§4.5.7.5/.6, §4.6 rows).

---

## 8. Open questions for the user

**Ratification record (2026-07-05):** all twelve recommendations below were RATIFIED as recommended by the user's bulk ruling (consolidated as `00-priority-spine-overview.md` §3 Cluster E, Q36–Q47; see its ratification record). No override touches this doc — in particular: type-target application pulled into WF-1B (OQ1), `ctx` v1 = `{module_path, file}` (OQ5), `__original__` direct-typed-forwarding break (OQ6), comptime purity as a language guarantee (OQ8), `[build.config]` + keyed `build_config(key)` (OQ11), and duplicate-annotation compile error (OQ12) are all adopted. §4.1.5's `ctx.target` runtime-hook contract was jointly ratified as the deliverable distributed OQ-12 demanded (overview Q4).

1. **Type-target annotation application scope.** The derive showcase (§4.9.1) needs `@ann` applied to `type` declarations, documented as v0.4-planned (cookbook:143). **Recommend: pull into WF-1B scope** — the annotation machinery, `targets: [type]` validation, and `target.fields` descriptors all already exist; without it the flagship derive pattern ships in its clumsy `comptime`-block form. Ruling requested: in-scope now, or showcase ships in fallback form?
2. **`optional` in the v1 contract.** Code builds it, book omits it. **Recommend: contract includes `optional`; book corrected.** Confirm?
3. **`type_info(T).fields` sequencing.** Contract now + clean SURFACE until V3-S5 `Array<TypedObject>` carriers land (recommended), or hold the whole `fields` key out of contract v1 until carriers exist?
4. **Contract v2 scope (extended).** Three named v2 items, all deferred from v1 by explicit ruling in this doc: (i) first-class `type` values (Zig-style, `field.type` usable as a type, not a string — §4.1.1); (ii) **typed annotation args** (resolving the `@default(3)` vs `@default("3")` v1 indistinguishability, §4.1.1 stringification ruling); (iii) **method/impl/trait-conformance introspection** (Zig `@typeInfo` decls equivalent — §4.1.2 scope note). Do you want these on the roadmap as comptime contract v2, or is the v1 string-composition surface the intended long-term shape?
5. **`ctx` v1 contract.** Proposed (revised): `{ module_path, file }` read-only (§4.4) — `build` dropped from the first draft because `build_config()` is the single build-info surface (no two-ways-to-one-thing). Ratify, extend (what else?), or keep `ctx` reserved another cycle?
6. **`__original__` convention break.** The `__original__(args)` array convention is documented across four book files (content-addressed-bytecode, transport-layer, wire-protocol, stdlib/core/state — §4.10.5, grep-audited) but has never produced correct results. **Recommend: replace with direct typed forwarding, update book, no deprecation shim** (there is nothing working to deprecate). Confirm?
7. **LSP expansion lens placement.** "Show comptime expansion" code lens + annotation hover (§4.7.5): bundle into WF-3B ux-polish, or defer to a post-v0.3.3 follow-up?
8. **Comptime purity as policy.** Ratify "comptime executes with an empty permission set; `build_config()` is the only environment window" (§4.8) as a language guarantee (book-documented). This forecloses future compile-time I/O features (e.g. `comptime include_file(...)`) unless explicitly revisited — if you want an embed-file story later, it should be a dedicated, hash-tracked builtin, not general FsRead. Confirm the strict stance?
9. **`std::llm` namespace.** The LLM showcase introduces `std::llm` (tools + prompt validation now; a thin runtime client over `std::http` once WF-2E lands). Ratify the namespace and the "no model calls at compile time" positioning?
10. **`required_permissions` contract extension.** Ratify the committed-additive `required_permissions: Array<string>` on function targets (§4.1.1, delivered with WF-2C, sourced from the linker's transitive union) — including that until it lands, access is an ordinary "no such field" compile error (no sentinel, no empty-array lie)?
11. **`[build.config]` + keyed `build_config(key)`.** This adds new project-file surface: a declared `[build.config]` string→string table in `shape.toml`, hash-tracked as part of the content-addressed input set (deployment config lives in the project, NOT compile-time env vars — §4.1.3/§4.8 determinism), read via `build_config("KEY") -> string?` resolved by compile-time arity rewrite to the internal `__build_config_key` (no overloading exists in Shape; §4.1.3 mechanism). Sibling dependency: `distributed-function-transfer.md` blesses `@remote(build_config("WORKER_ADDR"))` as the non-toy deployment form. Ratify the table, its hash participation, and the rewrite mechanism?
12. **Duplicate annotation application = compile error (v1).** Applying the same annotation twice to one target is ruled a compile error so `own_args` is unambiguous (§4.1.1; the silent outermost-wins alternative validated the wrong invocation, §5 item 13). If repeated annotations are wanted later (e.g. `@example` × N), the v2 mechanism is an invocation index on the handler context. Ratify the v1 restriction?

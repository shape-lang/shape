# Wave 12C Comptime / Proof-Gap Scope

Date: 2026-07-09

Worker: Wave-12C comptime/proof-gap scout

Scope: answer the current comptime ergonomics/strict-typing questions and rank
remaining proof gaps. This pass inspected source, tests, docs, and the current
sibling book snippet manifest only. It did not run cargo, just, rustc, build,
test, nextest, or book-truth commands.

## Executive Answers

### 1. Is comptime ergonomic today?

Partially. It is ergonomic for the narrow surfaces that now have active tests:

- Plain `comptime { ... }` expressions and top-level blocks are documented and
  have active fixtures for literals, nested blocks, conditionals, arrays,
  diagnostics, and compile-time field access
  (`shape-web/book/book-site/src/content/docs/advanced/comptime.mdx:39`,
  `tools/shape-test/tests/comptime/blocks.rs:13`,
  `tools/shape-test/tests/comptime/blocks.rs:219`,
  `tools/shape-test/tests/comptime/blocks.rs:340`,
  `tools/shape-test/tests/comptime/blocks.rs:377`).
- `comptime fn` is usable for small typed helper computations, including
  recursion and string operations, and is guarded against runtime calls
  (`shape-web/book/book-site/src/content/docs/advanced/comptime.mdx:95`,
  `tools/shape-test/tests/comptime/functions.rs:13`,
  `tools/shape-test/tests/comptime/functions.rs:68`,
  `tools/shape-test/tests/comptime/functions.rs:139`,
  `tools/shape-test/tests/comptime/functions.rs:187`).
- Annotation-driven type/function/module transforms are usable for specific
  targets. Active fixtures cover `extend target`, `replace body`, `replace
  module`, stacked annotation extensions, and stdlib-style derives such as
  `@json_schema`, `@to_json`, `@llm_tool`, and `@prompt`
  (`shape-web/book/book-site/src/content/docs/advanced/annotations.mdx:109`,
  `shape-web/book/book-site/src/content/docs/advanced/annotations.mdx:131`,
  `tools/shape-test/tests/comptime/annotations.rs:267`,
  `tools/shape-test/tests/annotations_comptime/code_gen.rs:8`,
  `tools/shape-test/tests/annotations_comptime/directives.rs:107`,
  `tools/shape-test/tests/annotations_comptime/showcases.rs:18`).
- The flagship WF-3D comptime fixtures explicitly describe four green
  capabilities across VM/JIT: generated free functions, `type_info`,
  LSDS diagnostics, and generated methods
  (`tools/shape-test/tests/comptime/flagship_wf3d.rs:20`).

It is not yet ergonomic for the workflow the book is aiming at:

- Connector/schema inference examples remain disabled and still show string
  return-type generation rather than a typed schema API
  (`shape-web/book/book-site/src/content/docs/advanced/comptime.mdx:266`,
  `shape-web/book/book-site/src/content/docs/advanced/comptime-annotations-cookbook.mdx:31`).
- `set param` cannot yet update the public function call surface. The active
  ignored tests name this as the remaining gap
  (`tools/shape-test/tests/comptime/annotations.rs:631`,
  `tools/shape-test/tests/annotations_comptime/directives.rs:8`,
  `tools/shape-test/tests/annotations_comptime/on_define.rs:154`).
- Expression/await annotation targets, await-based routing, and checkpointed
  workflow examples are still disabled or explicitly rejected
  (`shape-web/book/book-site/src/content/docs/advanced/annotations.mdx:73`,
  `shape-web/book/book-site/src/content/docs/advanced/annotations.mdx:508`,
  `shape-web/book/book-site/src/content/docs/advanced/comptime-annotations-cookbook.mdx:183`,
  `shape-web/book/book-site/src/content/docs/advanced/comptime-annotations-cookbook.mdx:329`,
  `tools/shape-test/tests/comptime/annotations.rs:475`).
- The docs still contain stale or over-broad cautions saying annotation
  application is planned rather than available, while current annotation docs
  and tests prove that some target transforms are active today
  (`shape-web/book/book-site/src/content/docs/advanced/comptime.mdx:113`,
  `shape-web/book/book-site/src/content/docs/advanced/comptime-annotations-cookbook.mdx:95`,
  `shape-web/book/book-site/src/content/docs/advanced/annotations.mdx:17`).

Current book evidence from the sibling manifest is consistent with this mixed
status: the manifest has 756 snippets overall, 29 comptime-related snippets by
path/title match, 21 runnable comptime-related snippets, and 8 disabled
comptime-related snippets. The active supervisor row records the broader
Wave-11 state as 756 extracted snippets, 500 runnable snippets, 256 disabled
snippets, and a 500/500 release-binary book-truth gate
(`AGENTS.md:64`).

### 2. Is comptime strictly typed, or still source-string/directive oriented?

Mixed. The runtime and reflection substrate is much more typed than the old
string-generation story, but the public mutation/codegen surface is still
source-string/directive oriented.

Typed evidence:

- `ComptimeTarget` is materialized as a typed object with typed arrays for
  fields, params, annotations, and captures. `target.fields` and
  `type_info(T).fields` share the same descriptor builder
  (`crates/shape-vm/src/compiler/comptime_target.rs:106`,
  `crates/shape-vm/src/compiler/comptime_target.rs:149`,
  `crates/shape-vm/src/compiler/comptime_target.rs:275`).
- Handler parameters are assigned concrete `target` and `ctx` object types
  before the comptime handler is analyzed
  (`crates/shape-vm/src/compiler/comptime.rs:94`,
  `crates/shape-vm/src/compiler/comptime.rs:1034`,
  `crates/shape-vm/src/compiler/comptime.rs:1120`).
- `build_config` and `type_info` return typed objects through the comptime
  builtin layer, and the compiler rewrites bare type/trait identifiers into
  names only at the builtin boundary
  (`crates/shape-vm/src/compiler/comptime_builtins.rs:399`,
  `crates/shape-vm/src/compiler/comptime_builtins.rs:488`,
  `crates/shape-vm/src/compiler/comptime.rs:323`).
- Comptime return materialization is kinded: `ComptimeExecutionResult` carries
  a `KindedSlot`, and `nb_to_expr` maps primitive kinds and typed objects back
  into typed AST/literal forms. Typed-array return materialization is still a
  documented missing arm
  (`crates/shape-vm/src/compiler/comptime.rs:55`,
  `crates/shape-vm/src/compiler/comptime.rs:1311`,
  `crates/shape-vm/src/compiler/comptime.rs:1520`).
- The generated outputs are re-entered into normal strict compilation for the
  supported target shapes. Function directive processing re-runs type checking
  for directive-mutated signatures before bytecode emission
  (`crates/shape-vm/src/compiler/functions_annotations.rs:1989`).

String/directive evidence:

- The primary directive enum is structured, but most emitted payloads are
  strings or JSON-serialized AST strings at the builtin boundary
  (`crates/shape-vm/src/compiler/comptime_builtins.rs:127`,
  `crates/shape-vm/src/compiler/statements.rs:118`).
- `set return`, `replace body`, and `replace module` first try JSON AST payloads
  and then fall back to parsing textual Shape source
  (`crates/shape-vm/src/compiler/comptime_builtins.rs:233`,
  `crates/shape-vm/src/compiler/comptime_builtins.rs:253`,
  `crates/shape-vm/src/compiler/comptime_builtins.rs:271`).
- `extend (expr)` is explicitly a string-of-Shape-source path. The book tells
  authors to call `string_lit` when generating that source
  (`shape-web/book/book-site/src/content/docs/advanced/comptime.mdx:212`,
  `shape-web/book/book-site/src/content/docs/advanced/comptime.mdx:238`,
  `crates/shape-vm/src/compiler/statements.rs:687`,
  `crates/shape-vm/src/compiler/comptime_builtins.rs:660`,
  `crates/shape-vm/src/compiler/comptime_builtins.rs:676`).
- `set param value` still has a narrow internal builtin surface. The helper is
  effectively fixed to `string, int`, and a local comment says the variadic path
  currently stamps args as Bool
  (`crates/shape-vm/src/compiler/comptime_builtins.rs:591`).
- Annotation metadata is still lossy in places: annotation args are stringified
  through literal raw values or `Debug`, and type annotations are converted back
  into type strings
  (`crates/shape-vm/src/compiler/comptime_target.rs:414`,
  `crates/shape-vm/src/compiler/comptime_target.rs:429`).

So the honest answer is: comptime execution is increasingly typed, while
comptime authoring remains directive/string heavy. It is not yet a hygienic
typed macro/quasiquote system.

## Implementation Lane Toward Typed Comptime APIs

The smallest high-leverage lane should not start by inventing a whole macro
language. It should first replace the known string chokepoints that already have
tests and disabled examples around them.

### Lane A: close current directive correctness gaps

1. Make `set param` update the public function call surface, including arity,
   default metadata, and non-int default values. The ignored tests in
   `annotations.rs`, `directives.rs`, and `on_define.rs` should become active
   acceptance tests.
2. Re-verify `replace module` output against declared signatures and ordinary
   strict typing. There is already an ignored active test naming this exact
   problem
   (`tools/shape-test/tests/annotations_comptime/directives.rs:130`).
3. Keep the current post-directive re-analysis invariant for functions and add
   the equivalent invariant for module replacements. `replace module` currently
   assigns parsed items as the replacement module body; this path needs an
   explicit generated-module typecheck gate before it can be treated as proof
   friendly.

### Lane B: introduce typed fragment values beside source strings

1. Promote `ConstantValue` from a mostly test-only/bridge type into the
   comptime value contract for generated fragments. The file already names this
   as the replacement for raw value plus optional type tag and has variants for
   concrete typed values and opaque bridges
   (`crates/shape-vm/src/compiler/comptime_concrete.rs:1`,
   `crates/shape-vm/src/compiler/comptime_concrete.rs:64`).
2. Add first-class typed fragment values such as `TypeExpr`, `ExprFragment`,
   `StmtFragment`, `ItemFragment`, and `ModuleFragment`. These should carry
   parsed AST plus source spans/hygiene marks, not Shape source text.
3. Add internal directive builtins that accept those fragments directly:
   `__emit_set_return_type_typed(TypeExpr)`,
   `__emit_replace_body_typed(Array<StmtFragment>)`,
   `__emit_replace_module_typed(ModuleFragment)`, and
   `__emit_extend_items_typed(Array<ItemFragment>)`.
4. Leave the string paths available as compatibility shims, but route new
   book/examples/tests through typed fragments.

### Lane C: add a small quasiquote API with hygiene

1. Start with typed quote forms that cover the already-supported targets:
   function declarations, method declarations, type expressions, and function
   bodies.
2. Support typed holes for identifiers, type expressions, expressions, and item
   fragments. Identifier holes should not be plain string interpolation.
3. Attach hygiene metadata to generated identifiers so local helper names in an
   annotation cannot accidentally capture runtime locals or collide with user
   declarations.
4. Add an `expand-comptime` display for quoted fragments so authors can inspect
   generated code without making source strings the authoring API
   (`bin/shape-cli/src/commands/expand_comptime_cmd.rs:11`,
   `bin/shape-cli/src/commands/expand_comptime_cmd.rs:123`).

### Lane D: make reflection metadata typed enough for real APIs

1. Replace type strings in `target.params`, `target.return_type`, and
   `type_info` descriptors with a stable `TypeRef`/`TypeExpr` value while
   retaining a `.display` or `.source` field for diagnostics.
2. Preserve annotation arguments as typed values instead of the current lossy
   stringification path.
3. Fix the handler-context type snapshot gap noted in the compiler so
   `type_info(T)` inside annotation bodies resolves against the same user
   module context that created the target
   (`crates/shape-vm/src/compiler/comptime.rs:1198`).

Acceptance tests for the lane:

- Unignore the three `set param` public-surface tests and the generated module
  re-verification test.
- Add quote-based versions of the current WF-3D flagship generated-function and
  generated-method tests. The generated code should not be assembled with
  `f"..."` source strings.
- Add hygiene tests where an annotation emits helper names that collide with a
  user local, user function, imported symbol, and target field name.
- Add typed-fragment negative tests for malformed generated AST, wrong return
  type, stale type references, and invalid module item kinds.
- Keep VM/JIT parity expectations for generated outputs where the current
  flagship fixtures already require it.

## Ranked Global Proof Gaps

The current proof posture is better than it was, but the existing guards are
bounded. The top remaining risks are below, ordered by likely blast radius and
evidence strength.

### P0: unsafe provenance still has unproven raw-carrier surfaces

Evidence:

- The Miri guard is explicitly targeted evidence, not a full UB proof
  (`scripts/check-miri-provenance.sh:4`,
  `scripts/check-miri-provenance.sh:83`).
- The latest Miri expansion docs continue to identify TypedArray raw carriers
  as the largest unproven surface, followed by trait object raw inner/vtable,
  snapshot/wire restore, and JIT/FFI return paths
  (`docs/cluster-audits/w93d-miri-unsafe-proof-next.md:26`,
  `docs/cluster-audits/w94d-miri-unsafe-proof-expansion.md:32`,
  `docs/cluster-audits/w94d-miri-unsafe-proof-expansion.md:42`).
- The ignored/Miri classification doc says the current Miri boundary does not
  prove VM/runtime/JIT/FFI, snapshots, arbitrary programs, all heap carriers,
  all raw consumers, or ignored tests
  (`docs/cluster-audits/w86c-ignored-tests-and-miri-classification.md:145`,
  `docs/cluster-audits/w86c-ignored-tests-and-miri-classification.md:222`).

Concrete verifier proposals:

- Add targeted Miri probes for typed-array elements stored inside typed-object
  fields, nested arrays, and arrays crossing module return/state/snapshot
  boundaries.
- Add trait-object Miri probes that exercise raw inner pointer replacement,
  vtable lookup, drop, and clone/copy paths.
- Add snapshot/wire restore provenance probes for typed object, typed array,
  trait object, Result/Option, and JsonValue object/array carriers.
- Add FFI/JIT boundary provenance probes where Miri can reach the host-facing
  wrapper code; document the parts that Miri cannot execute.

### P1: source-only proof guards are not semantic gates

Evidence:

- The typed-opcode checker is source-only. Its current baseline is strong for
  the narrow property it checks: `prove_native_kind = 22`,
  `covered_by_equivalent_static_proof_helper = 515`, `metadata_only = 138`,
  and `unproven_gap = 0`
  (`docs/cluster-audits/w91a-typed-opcode-proof-coverage.md:17`,
  `docs/cluster-audits/w91a-typed-opcode-proof-coverage.md:23`,
  `scripts/check-typed-opcode-proof-coverage.py:1`,
  `scripts/check-typed-opcode-proof-coverage.py:29`).
- The checker classifies Rust source sites by known helper/proof patterns; it
  does not execute compiler/runtime paths
  (`scripts/check-typed-opcode-proof-coverage.py:199`).
- The ignored-test classifier is also source-only and explicitly does not run
  cargo/nextest/Miri
  (`scripts/check-ignored-test-classification.py:1`).
- The ignored-test docs record that cargo-reported ignored projections are
  stale and require a later supervisor lane
  (`docs/cluster-audits/w86c-ignored-tests-and-miri-classification.md:81`,
  `docs/cluster-audits/w86c-ignored-tests-and-miri-classification.md:117`).

Concrete verifier proposals:

- Add a supervisor-only ignored-test projection gate that compares source
  ignore counts with actual cargo/nextest test inventory.
- Keep the typed-opcode source checker, but pair it with generated fixture
  coverage for every class it treats as "covered by equivalent static proof":
  arrays, locals, typed objects, RowView, typed returns, and captures.
- Add negative fixtures that try to select typed opcodes without the recognized
  proof helpers and assert compile-time rejection.

### P1: comptime generated-code correctness still has active named gaps

Evidence:

- `set param` public metadata is ignored in several current fixtures
  (`tools/shape-test/tests/comptime/annotations.rs:631`,
  `tools/shape-test/tests/annotations_comptime/directives.rs:8`,
  `tools/shape-test/tests/annotations_comptime/on_define.rs:154`).
- `replace module` has an ignored test saying generated source is not
  re-verified against declared return types
  (`tools/shape-test/tests/annotations_comptime/directives.rs:130`).
- The disabled comptime docs are concentrated around connector inference,
  await/policy, and checkpoint examples, not the simple working cases
  (`shape-web/book/book-site/src/content/docs/advanced/comptime-annotations-cookbook.mdx:31`,
  `shape-web/book/book-site/src/content/docs/advanced/comptime-annotations-cookbook.mdx:183`,
  `shape-web/book/book-site/src/content/docs/advanced/comptime-annotations-cookbook.mdx:329`).

Concrete verifier proposals:

- Promote the ignored `set param` and `replace module` tests into active
  regression gates as soon as the implementation lands.
- Add `expand-comptime` snapshot tests for generated module/type/function
  output, including failure snapshots for invalid typed fragments.
- Add strict re-analysis tests for every directive that mutates a public
  signature or module item list.

### P2: GC/JIT barrier proof is good for current gates but not complete for
mutation-heavy native JIT

Evidence:

- The GC readiness report records that the user-facing GC default flip and JIT
  forwarding were applied and that JIT-tier proof tests were green
  (`docs/cluster-audits/gc-on-readiness-report.md:40`).
- The same report says the mutation-heavy JIT barrier cost remains unmeasured
  because current JIT mutation probes deopt, and it asks for a native-JIT
  mutation benchmark or MIR/FFI barrier review before claiming the barrier is
  cheap enough
  (`docs/cluster-audits/gc-on-readiness-report.md:97`,
  `docs/cluster-audits/gc-on-readiness-report.md:134`).

Concrete verifier proposals:

- Add a native-JIT mutation fixture that does not deopt and exercises
  typed-object field writes, array writes, object writes, and closure captures
  under GC-on.
- Add a barrier audit that maps each JIT store-lowering path to either a proven
  write barrier or an explicit non-heap/non-GC value proof.
- Add a bounded RSS regression for a JIT allocation/mutation loop once the JIT
  can keep those probes native.

### P2: distributed, snapshot, transport, and polyglot claims are outside most
current proof guards

Evidence:

- The Wave-6 distributed/comptime/proof triage already called out that typed
  opcode and Miri/source guards do not prove distributed, FFI, snapshot, or
  permission semantics
  (`docs/cluster-audits/wave6-disabled-distributed-comptime-proof-triage.md:469`).
- The same triage recommended dedicated comptime and proof matrices rather than
  treating the source guards as end-to-end correctness
  (`docs/cluster-audits/wave6-disabled-distributed-comptime-proof-triage.md:424`,
  `docs/cluster-audits/wave6-disabled-distributed-comptime-proof-triage.md:461`).
- Current Wave-9 through Wave-11 registry state records real progress on
  snapshot-store selection, receiver snapshot resume, and dynamic polyglot
  transfer, but those were focused e2e lanes, not global proof closure
  (`AGENTS.md:47`, `AGENTS.md:48`, `AGENTS.md:49`, `AGENTS.md:64`).

Concrete verifier proposals:

- Keep distributed/snapshot/polyglot e2e gates as their own supervisor lane,
  with explicit coverage rows for TCP, TLS/auth, store selection, receiver
  context, resume, dynamic runtime opt-in, and extern-C process abort handling.
- Add snapshot serialization invariants that compare compile-time static type
  metadata with restored runtime carriers.
- Add permission/security negative fixtures for non-loopback remote call, TLS
  trust roots, dynamic runtime opt-in, and host API access.

### P3: docs and proof-status drift can still hide real gaps

Evidence:

- Comptime docs still say some annotation application is not available, while
  current tests and pages show function/type/module transforms working.
- The compiler's current comptime handler `ctx` type is `{ module_path, file }`,
  but older cookbook prose still describes higher-level runtime-style contexts
  (`crates/shape-vm/src/compiler/comptime.rs:74`,
  `crates/shape-vm/src/compiler/comptime.rs:1120`,
  `shape-web/book/book-site/src/content/docs/advanced/annotations.mdx:312`).
- Some old `phase-2c` ignored unit tests remain in comptime internals even
  while public shape-test surfaces have moved on
  (`crates/shape-vm/src/compiler/comptime.rs:1717`,
  `crates/shape-vm/src/compiler/comptime_target.rs:621`).

Concrete verifier proposals:

- Add a docs-status sweep after Wave-12A's disabled-snippet classification:
  mark stale cautions separately from true feature gaps.
- Split old `phase-2c` internal ignores into current buckets:
  deleted-v1-path, still-relevant implementation gap, or diagnostic-only.
- Add a cheap manifest auditor that reports disabled snippets by feature area
  and reason tag, then compare that to source ignore classifications.

## Recommended Next Lanes

1. **Typed comptime directives lane**: close `set param` metadata and
   generated-module re-verification first; then introduce typed fragment
   directive builtins and quote-based replacements for the current string
   examples.
2. **Comptime hygiene lane**: add identifier hygiene, typed holes, and collision
   tests before promoting quasiquote as the recommended authoring surface.
3. **Unsafe provenance lane**: expand targeted Miri coverage to typed-array raw
   carriers, trait objects, snapshot/wire restore, and FFI/JIT wrappers.
4. **Source-guard-to-runtime bridge lane**: pair the typed-opcode checker and
   ignored-test classifier with supervisor-only runtime inventory/projection
   gates and fixture coverage for each classified proof family.
5. **GC/JIT mutation lane**: create a non-deopting native-JIT mutation benchmark
   and barrier verifier so the GC/JIT claim covers write-heavy code, not only
   current bounded-RSS and deopted mutation evidence.

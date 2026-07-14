# Wave 41C: Comptime And Annotation Example Matrix

Scope: static audit of focused ShapeTests, compiler-owned feature tests, shipped
stdlib annotation definitions, the sibling book manifest/pages, and the Wave 12,
22, 38, and 39 inventories. No extraction, build, or test command was run.

## Verdict

Shape has a useful but uneven comptime surface. The strongest current examples
are plain scalar comptime, `type_info`, type-target `comptime post`, generated
free functions/methods, three stdlib derives, module replacement, and function
runtime hooks. Each has an explicit VM/JIT proof or a current runnable book row.

The surface is not yet a generally typed macro system:

- `extend (expr)`, `replace module (expr)`, and computed body/type directives
  still accept source text or JSON-shaped AST payloads;
- `item_fn` is the only typed generation fragment, limited to a zero-argument
  free function returning one literal;
- `comptime pre`, `on_define`, and `metadata` have compiler paths but lack a
  focused observable VM/JIT example; `metadata` return values are popped;
- expression/await/block/binding targets are parser or partial compiler shapes,
  not a truthful general runtime-annotation surface;
- runtime hook arguments are array carriers, and `after` may change a declared
  return type without changing the public signature; and
- effect and ownership metadata are not exposed by `ComptimeTarget`, while no
  focused generated-code ownership diagnostic proves full re-entry.

The smallest truthful teaching corpus is therefore the green rows below plus a
few explicitly caveated rows, always paired with the failure or limit in the
same row. Definition-only examples cannot stand in for an applied transform.

## Evidence Rules

`ShapeTest::new` defaults to the bytecode VM
(`tools/shape-test/src/shape_test.rs:80-136`). Of 193 focused tests across the
four suites, only eight call `.with_jit()`: four flagship tests, three stdlib
showcase tests, and one generated-method test. A VM-only ShapeTest is not JIT
evidence.

The current sibling manifest was generated at `2026-07-10T07:10:04.546Z` and
contains 707 snippets: 565 runnable and 142 disabled. Its report, generated at
`2026-07-10T07:13:32.347Z`, records 565/565 passing. Ordinary runnable rows run
both VM and JIT; expected-fail rows require both to fail with the named text.

For the five directly relevant pages there are 41 fences: 31 runnable and 10
disabled. A runnable definition with no use-site proves parsing, registration,
and dual-mode startup only. It does not prove that a hook executes or a directive
changes a target.

Status labels:

- **G**: works today with explicit VM and JIT evidence.
- **C**: works with a named caveat, usually VM-only or definition-only proof.
- **D**: disabled or rejected; the row states whether this is feature, proof, old
  syntax, or fixture-only.
- **T**: target design only, with no current implementation claim.

`MISSING` in a failure column means there is no focused stable diagnostic proof.
That absence is part of the classification, not an invitation to infer one.

## Plain Comptime Matrix

| ID | Construct | Status | Smallest truthful positive | Failure diagnostic or limit |
|---|---|---|---|---|
| B1 | Expression block | **G** | `let x: int = comptime { 1 }; x` (`comptime/blocks.rs:14-21`; book `comptime.mdx:46`) | Capturing `marker` reports `Undefined variable: 'marker'` in both book modes (`blocks.rs:341-354`; book `:76`). JIT mode deliberately deopts top-level comptime (`compiler_impl_reference_model.rs:2407-2414`). |
| B2 | Top-level block and diagnostics | **G** | `comptime { warning("w") }` (`blocks.rs:25-37`; book `:58`) | `comptime { error("boom") }` reports `[C0001]` in explicit VM/JIT flagship pairs (`flagship_wf3d.rs:99-117`). |
| B3 | `comptime fn` | **C** | `comptime fn f() { 1 }; let x = comptime { f() }` has an exact VM assertion (`functions.rs:14-28`); book `:95` starts in both modes | The book discards the helper result, so it is not JIT value proof. Calling `f()` at runtime reports a comptime-only error (`functions.rs:188-197`); calling ordinary `normal_add` from comptime reports `Undefined function: 'normal_add'` (`:201-212`). |
| B4 | Typed scalar materialization | **C** | int/string/bool/number values are baked (`blocks.rs:40-53,324-337`) | MISSING dual-mode type-mismatch pair; only the general type checker protects the destination annotation. |
| B5 | Array materialization | **C** | `let xs: Array<int> = comptime { [1,2,3] }` compiles (`blocks.rs:219-228`) | The test never reads `xs`; there is no focused round-trip or failure diagnostic. Do not teach arbitrary aggregate embedding from this row. |
| B6 | Nested/multiple/conditional blocks | **C** | nested, conditional, arithmetic, and reuse cases are active (`blocks.rs:56-65,95-204,232-254`) | VM-only ShapeTests; no paired JIT semantic assertions beyond B1's deopt contract. |
| B7 | Comptime fields on types | **C** | `Currency::symbol` and `usd.symbol` fold to the declared literal (`blocks.rs:380-433`) | A no-default field has parse-only coverage (`structs_types/generics_comptime.rs:246-257`); no stable use diagnostic is asserted. |
| B8 | `build_config()` | **C** | returning the whole typed object compiles (`blocks.rs:84-92,311-321`) | `ct_49b` documents `c.target_os` as yielding `None` but only asserts the `OS:` prefix (`blocks.rs:436-453`). Field semantics are therefore unproven, not a stable diagnostic. |
| B9 | `implements` | **C** | both string names and bare identifiers return true for a real impl (`functions.rs:159-180,224-245`) | MISSING negative diagnostic; false is a normal query result. VM-only. |
| B10 | `comptime for` | **C** | syntax is accepted and a compiler unroll/rewrite path exists (`type_info_chained.rs:451-467`; `expressions/misc.rs:819-886`) | The focused test is parse-only; the runnable cookbook rows merely define unapplied hooks. The applied end-to-end codegen row remains disabled. |
| B11 | const-template specialization | **D** | Unspecialized base compilation is covered (`functions.rs:2882-2911`) | Distinct-value specialization is ignored at `functions.rs:2913`; connector examples at book `comptime.mdx:266` and cookbook `:31` are disabled active-feature rows. |

## Reflection Matrix

| ID | Construct | Status | Smallest truthful positive | Failure diagnostic or limit |
|---|---|---|---|---|
| I1 | `type_info(T)` name/kind/fields | **G** | `type_info(Point).fields[0].name` and `.type` have exact paired VM/JIT results (`flagship_wf3d.rs:68-94`) | Unknown `NopeXYZ` returns the typed label `Unresolved`; it is not an error. |
| I2 | `TypeRef` siblings | **C** | target params/returns expose `.type_ref.kind/source`; directives accept them (`annotations_comptime/type_mutation.rs:263-335`) | VM-only; current `TypeRef` is a `{name,kind,source}` descriptor, not a hygienic compiler type token (`comptime_target.rs:106-121`). |
| I3 | Generic and enum queries | **C** | Array/Option/Result/HashMap and user enums execute without panic (`type_info_chained.rs:164-304`) | Most tests discard the reflected value and assert only a sentinel string. Payload variants and recursive details are not reflected. |
| I4 | Direct/multilevel reflection | **D** | `type_info(...).name.length` and iteration syntax parse (`type_info_chained.rs:420-467`) | Direct typed-object printing and multilevel/iteration semantics are parse-only, explicitly not runtime proofs. |
| I5 | Function target descriptor | **C** | `target.params[*]`, `return_type`, and typed siblings drive `@llm_tool` and TypeRef tests (`llm/tools.shape:31-67`; `type_mutation.rs:263-335`) | No effect set, pass mode, ownership, or stable symbol identity is present in `ComptimeTarget` (`comptime_target.rs:224-249`). |
| I6 | Type fields and field annotations | **G** | `@json_schema` reads names, optionality, TypeRef kind, and `@description` args with exact VM/JIT output (`showcases.rs:28-56`) | Unsupported nested field types report `has no JSON Schema mapping` (`showcases.rs:59-86`). Annotation args are still stringified (`comptime_target.rs:248-304`). |
| I7 | Module descriptor | **C** | module targets expose item names/types and can drive replacement (`comptime_target.rs:307-329`; `directives.rs:147-218`) | MISSING focused descriptor-field assertion; the dual-mode book row proves replacement, not reflection contents. |
| I8 | `target.annotations`, captures, comptime `ctx` | **C** | carriers exist in the descriptor builder (`comptime_target.rs:224-249,350-376`) | MISSING focused observable examples. `ctx` is a reserved `{state,event_log}` object, not a general typed policy API. |

## Hook And Target Matrix

| ID | Construct | Status | Smallest truthful positive | Failure diagnostic or limit |
|---|---|---|---|---|
| H1 | `comptime post` | **G** | Applied type post-hook adds `summary()` and returns `Ada:7` under JIT (`annotations_comptime/code_gen.rs:172-192`); VM companion shapes are broad | Missing/extra fixed handler args report `missing annotation argument...` / `too many annotation arguments` (`compiler/functions.rs:3039-3071`). |
| H2 | `comptime pre` | **C** | Compiler executes pre before post for function targets (`functions_annotations.rs:776-812`) | No active applied behavioral ShapeTest or explicit JIT pair. The cookbook `serializable` row only defines the annotation. Explicit-type override coverage remains ignored (`compiler/functions.rs:2947`). |
| H3 | `on_define` | **C** | Compiler emits a definition-lifecycle call for function/type/module targets (`functions_annotations.rs:497-578`) | No focused ShapeTest observes it. Applying definition hooks to expression-like targets reports `definition-time lifecycle hooks` (`compiler/functions.rs:3284-3300`). |
| H4 | `metadata` | **C** | Handler registration and definition-target inference are tested (`compiler/statements.rs:8545-8568`) | Handler calls end in `Pop`; no durable metadata registry/read API or focused observable VM/JIT example exists (`functions_annotations.rs:587-665`). Calling this "metadata emission" overstates current behavior. |
| H5 | Fixed annotation args | **G** | `@llm_tool("description")` uses the annotation value during post and matches exact VM/JIT schema output (`showcases.rs:105-132`) | Arity diagnostics are the H1 pair. |
| H6 | Variadic comptime handler args | **D** | Parser/compiler enforce at most one final variadic parameter (`comptime.rs:1071-1088`) | Heterogeneous variadic execution remains ignored (`compiler/functions.rs:3017-3037`). |
| H7 | Named and namespace imports | **G** | Bare and qualified imported `@remote` definitions start in both book modes (`annotations.mdx:403-430`) | These examples deliberately do not call the target; they prove resolution only. Missing annotation names use normal resolution diagnostics, but no focused negative pair is present. |
| H8 | Stacking/order/reuse | **C** | before/after stacking and reuse have VM examples (`annotations_runtime/before_after.rs:60-123,201-225`; `wrapping.rs:52-81`) | Duplicate use of the same annotation reports `more than once` (`type_mutation.rs:340-362`). No explicit stacked VM/JIT pair exists. |
| K1 | Function target | **G** | Applied `before` on `add` is a current dual-mode book row (`annotations.mdx:53-69`) | Applying a type-only annotation to a function reports a target mismatch (`annotation_targets/other_targets.rs:143-161`). |
| K2 | Struct type target | **G** | Applied post-hook adds and calls `Point.label()` in both book modes (`annotations.mdx:109-127`) | Applying a function-only annotation to a type reports a target mismatch (`other_targets.rs:121-138`). |
| K3 | Module target | **G** | Applied module post-hook replaces `payments::charge` in both book modes (`annotations.mdx:131-146`) | `replace module` on a function reports that it is valid only for module targets (`directives.rs:197-218`). Runtime `before/after` do not wrap modules. |
| K4 | Expression target | **D** | VM compiler unit: `remove target` yields Null (`compiler/functions.rs:3372-3391`) | Runtime hooks fail on `op_new_array(0): SURFACE` (`regression/language_surface.rs:76-97`); book `annotations.mdx:73` is disabled as active missing feature. |
| K5 | Await target | **D** | Parser and target validation accept `await @only_await()` (`compiler/functions.rs:3303-3344`) | No executed hook/result proof; book `annotations.mdx:89` is an active-feature disabled row. Extension-backed `@host` rows are separately fixture-only. |
| K6 | Block target | **D** | Enum/validator labels exist | Prefix annotation on a block is rejected in `annotation_targets/other_targets.rs:49-71`; no successful proof. |
| K7 | Binding target | **D** | Enum/validator labels exist | Prefix annotation on `let` fails (`other_targets.rs:29-45`); no successful proof. |
| K8 | Async function, enum, trait | **C** | Definitions with annotations compile (`function_target.rs:162-185`; `type_target.rs:168-215`) | Tests do not invoke the async function or prove enum/trait handler effects; comments still mark these as TDD shapes. |
| K9 | Impl/inline methods | **D** | None | Both forms produce parser diagnostics containing `found identifier` (`comptime/annotations.rs:561-625`). |

## Directive And Generation Matrix

| ID | Construct | Status | Smallest truthful positive | Failure diagnostic or limit |
|---|---|---|---|---|
| D1 | `set param name = expr` | **C** | Existing int/string/bool/number params gain defaults; explicit call args still win (`directives.rs:9-145`) | Unknown name reports `unknown parameter 'missing'` (`:56-76`). VM-only. |
| D2 | `set param name: Type/TypeRef` | **C** | Existing `left` adopts the second param's TypeRef (`type_mutation.rs:314-335`) | It cannot add a parameter: `extra` remains `Undefined variable: 'extra'` (`comptime/annotations.rs:675-695`). |
| D3 | `set return Type/TypeRef` | **C** | Compatible int and TypeRef-derived string returns run (`type_mutation.rs:238-309`) | Body mismatch reports `comptime directive` through normal rechecking (`:212-235`). Explicit source-return override proof is still ignored. |
| D4 | `replace body { ... }` | **C** | `@mock` replaces a function body with `"mocked"` (`type_mutation.rs:187-208`) | Current generated body must type-check; `args` is not injected and reports undefined (`compiler/functions.rs:3425-3448`). VM-only. |
| D5 | `replace body (expr)` | **D** | Implementation accepts JSON statements or source text | Its only focused execution test remains ignored (`compiler/functions.rs:3099-3124`). Do not teach it as executable. |
| D6 | `replace module (expr)` | **G** | Source replacement is applied in the dual-mode module book row and VM ShapeTest (`directives.rs:147-169`) | Generated wrong return type is rejected (`:170-195`); malformed text reports `invalid replacement module payload` (`:242-260`). Current payload is still source text. |
| D7 | `extend target { ... }` | **G** | Generated typed-object method executes under explicit JIT (`code_gen.rs:172-192`) and the type book row | Annotation parameters are not captured into generated method bodies; `default_val` is undefined (`type_mutation.rs:155-184`). |
| D8 | `extend TypeName { ... }` | **C** | Static AST augmentation supports direct method blocks (`shape-ast/transform/comptime_extends.rs:1-149`) | Conditional extraction is not a typed generated-fragment API; broad compiler unit coverage is historical/ignored. Prefer D7 as the teaching proof. |
| D9 | `extend (source_string)` | **G** | Flagship generated free function and method have exact VM/JIT results (`flagship_wf3d.rs:45-63,120-148`) | Parse/type failures re-enter compilation, but this remains unhygienic source generation; `string_lit` only escapes literals. |
| D10 | `extend (item_fn(...))` | **C** | Typed `ItemFragment` creates one zero-arg literal-return free function (`directives.rs:221-239`) | Invalid names and unsupported literal kinds are rejected in `comptime_builtins.rs:560-608`; no focused negative or JIT pair. No params, statements, methods, generics, or effects. |
| D11 | `remove target` | **C** | Type removal is VM-tested (`comptime/annotations.rs:289-307`); expression removal has a VM compiler unit | No dual-mode behavioral pair and no focused "use removed symbol" diagnostic. |
| D12 | `__original__(real_args...)` | **C** | Replacement body calls the original typed function (`compiler/functions.rs:3394-3423,3452-3475`) | Retired `__original__(args)` forwarding is not supported; `args` is undefined. VM compiler units only. |

## Runtime Hook Matrix

| ID | Construct | Status | Smallest truthful positive | Failure diagnostic or limit |
|---|---|---|---|---|
| R1 | `before(args, ctx)` identity/observe | **G** | Function book row applies a before hook in both modes (`annotations.mdx:53-69`) | Wrong target kind is rejected, but hook return shape is mostly runtime-classified rather than statically checked. |
| R2 | `before` rewrites args array | **C** | doubling/swapping/clamping int args works in VM (`annotations_runtime/injection.rs:11-138`) | No arity/type diagnostic matrix. Typed-array nesting has one VM proof (`annotations_comptime/runtime_hooks.rs:6-29`). |
| R3 | `before` object short-circuit | **G** | stdlib `@remote` returns `{result: value}`; fixture row passes VM/JIT with an `Array<int>` result (`annotations.mdx:480-504`) | This proves the one remote carrier path, not general `{args,state,result}` typing. |
| R4 | `ctx.state` / `event_log` | **C** | Wrapper emitters construct these fields | MISSING focused persistence/correlation test. A state replacement rebuilds ctx with a fresh event log; no typed state schema is exposed. |
| R5 | `after(args,result,ctx)` | **C** | Same-type transforms, stacking, strings, and void have broad VM tests (`wrapping.rs:9-184`) | `int -> string` is accepted (`wrapping.rs:32-49`) without changing the source signature. This is a signature-safety gap, not a feature to teach. |
| R6 | `ctx.target` original callable | **C** | after-hook calls original `square` without recursion (`before_after.rs:229-255`) | VM-only. Book prose still says `ctx.__impl`, while current stdlib/test source uses `ctx.target`. |
| R7 | annotation reuse and nested order | **C** | multiple functions and nested before/after layers work in VM (`before_after.rs:87-123,201-225`) | No explicit dual-mode order assertion. Duplicate same-annotation use is rejected by H8. |

## Shipped Stdlib Matrix

| Annotation | Status | Smallest positive | Failure/limit |
|---|---|---|---|
| `std::serde::derive::@json_schema` | **G** | Exact derived schema from typed fields/optionality/description under VM/JIT (`showcases.rs:28-56`) | Nested struct field reports `has no JSON Schema mapping` (`:59-86`). Generation is still source-string `extend`. |
| `std::serde::serialize::@to_json` | **G** | Exact generated method output under VM/JIT (`showcases.rs:88-103`) | Unsupported types call `error` in `serde/serialize.shape:27-55`, but no focused negative test. |
| `std::llm::tools::@llm_tool` | **G** | Exact function-signature schema under VM/JIT (`showcases.rs:105-132`) | Unsupported parameter reports `has no JSON Schema mapping` (`:135-153`). |
| `std::llm::tools::@prompt` | **G** | Current book row runs both modes; VM test validates a correct template (`showcases.rs:155-172`) | Typo reports `{audence}` (`:175-190`); no explicit JIT negative ShapeTest, but the shared book page is dual-mode. |
| `std::core::remote::@remote` | **G** | One loopback fixture forwards a typed-array call in VM/JIT (`annotations.mdx:480`) | This is runtime forwarding, not comptime generation; foreign receivers still need separate fixtures. |
| `std::finance::@warmup` | **C** | Pure marker resolves when the WMA module runs in paired VM/JIT tests (`stdlib_finance/sweep.rs:37-55`; `finance/annotations/warmup.shape`) | It intentionally has no hooks because function-local annotation args are unbound at definition scope. |
| `std::finance::@indicator` | **D** | Definition exists with lifecycle handlers | No focused use-site proof; `metadata` is discarded and the registry/cache API in its body is not established by current hook tests (`finance/annotations/indicator.shape`). |

## Type, Effect, And Ownership Boundary

| Interaction | Status | Current truth | Required diagnostic/proof |
|---|---|---|---|
| Generated signature vs body | **C** | Function directives are re-analyzed and public metadata refreshed (`compiler/functions.rs:840-870`); D3 proves one mismatch | Add VM/JIT pairs for param mode/default/return changes and explicit-source override rejection. |
| Runtime hook signature | **D** | R5 can return a string from an `-> int` target; R2 arrays are not a signature-bound pack | Reject incompatible transforms or make the wrapper's changed signature explicit. |
| Comptime/runtime effect isolation | **C** | Ordinary runtime calls are absent in comptime scope (B3) | `ComptimeTarget` has no effect row; async definitions are not executed through hooks. Add illegal IO/suspend diagnostics and typed capability tests. |
| Generated effects | **T** | No `CheckedExpr<T, Effects, Ownership>` or effect-bearing fragment exists | A generated call requiring undeclared effects must fail normal effect checking after insertion. |
| Ownership/borrows | **C** | Compiled functions enter MIR borrow analysis (`functions.rs:868-870`) | MISSING focused annotation-generated B0005/borrow-escape test and VM/JIT parity. Current target descriptors omit modes/lifetimes. |
| Runtime args ownership | **T** | Arrays cannot encode affine slots, borrows, or exact heterogeneous signatures | Use an immutable signature-bound internal `ArgumentPack<Sig>`; `@remote` only forwards it. |
| Hygiene and symbols | **T** | Source generation interpolates names; current TypeRef also carries source text | Require compiler-owned hygienic symbols and typed lenses, never runtime reconstruction by strings. |
| Compiler re-entry | **T** | Some current directive paths recheck, but JSON/source payloads remain | Every checked item/expression/module fragment must re-enter ordinary name, type, effect, ownership, lifetime, and exhaustiveness checking. |

## Disabled Book Rows

The ten disabled rows on the five focused pages are not one category:

| Row | Inventory class | Honest reason |
|---|---|---|
| `advanced/annotations.mdx:73` | active feature | Expression runtime hooks hit the empty-args array surface. |
| `advanced/annotations.mdx:89` | active feature | Await target parses/validates, but no executed hook/result contract is proven. |
| `advanced/annotations.mdx:508` | fixture-only | `@host` needs extension-provided routing/awaitable infrastructure. |
| `advanced/comptime-annotations-cookbook.mdx:31` | active feature | Connector-driven native schema discovery and computed concrete return type are incomplete. |
| `advanced/comptime-annotations-cookbook.mdx:183` | fixture-only | Await remote routing needs an external provider/peer. |
| `advanced/comptime-annotations-cookbook.mdx:308` | old syntax | Reliability composition uses a retired forwarding shape; this is a rewrite row. |
| `advanced/comptime-annotations-cookbook.mdx:329` | fixture-only | Checkpoint/resume workflow needs snapshot/provider fixture behavior. |
| `advanced/comptime-llm-patterns.mdx:170` | active feature | Bare source-producing `extend` fragment is not a standalone executable program and is not the typed target API. |
| `advanced/comptime.mdx:266` | active feature | Connector-driven native comptime and generated type carrier remain incomplete. |
| `examples/comptime-codegen.mdx:22` | active feature | The full pre/post field-iteration, generated body, external schema, and runtime-hook composition overclaims the current aggregate path. |

## Book Claim Corrections

1. `advanced/comptime.mdx:113-120` says applied directives are planned for v0.4.
   That is stale: active tests prove `set param`, `set return`, inline body
   replacement, type extension, removal, and module replacement. The disabled
   connector workflow is not evidence that every directive is absent.
2. Cookbook cautions at `:95-100` and `:141-146` say applying type hooks is not
   available. Applied type post-hooks and the stdlib derive trio are current.
   The specific broad serializer examples remain illustrative, not the whole
   application pipeline.
3. The seven target-kind table describes validator vocabulary, not seven equal
   executable surfaces. Only function, struct type, and module have honest
   applied end-to-end rows; expression/await are partial and block/binding fail.
4. `on_define`/`metadata` are emitted module-initialization calls, not proven
   compile-time metadata publication. A metadata result is currently discarded.
5. The `ctx.__impl` prose is stale against current `ctx.target` source/tests.
6. Current source/JSON codegen documentation accurately describes the shipped
   mechanism, but it must be labeled legacy relative to the accepted strictly
   typed comptime target: no source strings, parser round-trips, JSON AST,
   dynamic `Any`, or name-based runtime reconstruction.

## Smallest Truthful Next Example Set

Keep the current proof-bounded teaching core to these executable pairs:

1. B1 plus its runtime-local capture failure.
2. B3 plus runtime invocation of a comptime-only function.
3. I1 plus `Unresolved` reflection behavior.
4. H1/K2 `extend target` plus target mismatch.
5. K3/D6 module replacement plus generated-module type error.
6. D3 compatible `set return` plus body mismatch.
7. R1 plus function/type target mismatch.
8. R3 loopback short-circuit plus an explicit note that it proves only forwarding.
9. The four green stdlib annotations, retaining existing validation failures
   and adding the missing negative `@to_json` case.
10. D10 as a separately labeled narrow typed-fragment preview, not a general
    item/body/module generation API.

Before promoting H2, H3, H4, K4-K8, D5, R2, R4-R7, or ownership/effect claims,
add the missing focused positive/negative ShapeTests and explicit JIT companions.
The target-design replacement is sealed typed descriptors, hygienic symbols,
typed lenses, checked fragments, and mandatory normal compiler re-entry.

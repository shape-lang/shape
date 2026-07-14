# Wave 30 Disabled Language-Surface Triage

Date: 2026-07-09
Role: Wave-30A language-surface disabled-book triage worker

Scope honored:

- Read the current sibling manifest at
  `/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`.
- Used static inspection only: manifest JSON, sibling MDX pages, existing audit
  docs, and read-only `rg`/`sed` over local sources.
- Wrote only this report.
- Did not run cargo, rustc, just, nextest, build, tests, book-truth extractor,
  or book-truth gate commands.
- Did not edit production code, sibling book pages, or `AGENTS.md`.

## Manifest

The manifest is authoritative for this audit.

| Metric | Count |
|---|---:|
| Generated | `2026-07-09T23:40:40.617Z` |
| Total snippets | 707 |
| Runnable snippets | 541 |
| Disabled snippets | 166 |
| Deferred snippets | 0 |
| Scoped disabled snippets | 62 |

Scoped disabled pages found: `fundamentals/**`, `advanced/ownership-deep-dive.mdx`,
`advanced/developer-tools.mdx`, and `examples/**`. No disabled snippets were
found under scoped `appendix/**` or `getting-started/**` pages.

## Bucket Counts

| Bucket | Count |
|---|---:|
| Active implementation gap | 19 |
| External/manual/fixture/server/env/permission dependent | 8 |
| Proof/design gap | 5 |
| Preview/out-of-scope | 14 |
| Intentional diagnostic | 8 |
| Stale-green/count-reduction candidate | 3 |
| Old syntax/book rewrite | 5 |
| Total | 62 |

## Row Classification

| Page:line | Snippet id | Bucket | Reason |
|---|---|---|---|
| `advanced/developer-tools.mdx:86` | `E__advanced__developer-tools__0__L86.shape` | Proof/design gap | Page is explicitly planned; `std::debug::HotReloader` is not a Shape-level API even though Rust scaffolding exists. |
| `advanced/developer-tools.mdx:137` | `E__advanced__developer-tools__1__L137.shape` | Proof/design gap | Planned `std::debug::TimeTravel` Shape API; Rust-side time-travel exists but is not exposed as this stdlib surface. |
| `advanced/developer-tools.mdx:238` | `E__advanced__developer-tools__2__L238.shape` | Proof/design gap | Time-travel walkthrough needs a real debug API plus scaffolded target functions/data. |
| `advanced/developer-tools.mdx:320` | `E__advanced__developer-tools__3__L320.shape` | Proof/design gap | Planned `BlobPrefetcher` Shape API; source has runtime prefetch machinery, not user-facing `std::debug` wiring. |
| `advanced/developer-tools.mdx:462` | `E__advanced__developer-tools__4__L462.shape` | Proof/design gap | Execution proofs exist as Rust design/scaffolding; Shape builder surface and proof contract are not executable. |
| `advanced/ownership-deep-dive.mdx:45` | `A__advanced__ownership-deep-dive__0__L45.shape` | Preview/out-of-scope | Storage-class examples are conceptual/LSP-facing and use undefined `parse`, `make_buf`, and `Channel` scaffolding. |
| `advanced/ownership-deep-dive.mdx:54` | `A__advanced__ownership-deep-dive__1__L54.shape` | Preview/out-of-scope | Explicit storage-class prefix syntax is marked v0.4 preview in the page prose. |
| `advanced/ownership-deep-dive.mdx:81` | `A__advanced__ownership-deep-dive__2__L81.shape` | Active implementation gap | Smart `var` move/clone inference needs a self-contained proof or implementation lane; current snippet uses placeholder calls. |
| `advanced/ownership-deep-dive.mdx:141` | `A__advanced__ownership-deep-dive__5__L141.shape` | Intentional diagnostic | Use-after-move example is meant to fail with B0005. |
| `advanced/ownership-deep-dive.mdx:259` | `A__advanced__ownership-deep-dive__11__L259.shape` | Old syntax/book rewrite | This is desugaring pseudocode with undefined `x`/`read`; convert to prose or a real self-contained borrow smoke. |
| `advanced/ownership-deep-dive.mdx:399` | `A__advanced__ownership-deep-dive__13__L399.shape` | Intentional diagnostic | Mixed positive return-borrow example and ambiguous-return error; split if expected-error truth support is added. |
| `advanced/ownership-deep-dive.mdx:425` | `A__advanced__ownership-deep-dive__14__L425.shape` | Intentional diagnostic | Mixed OK cases and expected C0001 task-boundary error, with undefined helper functions. |
| `advanced/ownership-deep-dive.mdx:459` | `A__advanced__ownership-deep-dive__15__L459.shape` | Preview/out-of-scope | `Mutex<T>` method surface is marked not wired in the shipped binary. |
| `advanced/ownership-deep-dive.mdx:470` | `A__advanced__ownership-deep-dive__16__L470.shape` | Preview/out-of-scope | `Atomic<T>` method surface is marked not wired in the shipped binary. |
| `advanced/ownership-deep-dive.mdx:483` | `A__advanced__ownership-deep-dive__17__L483.shape` | Preview/out-of-scope | `Lazy<T>` method surface is marked not wired in the shipped binary. |
| `examples/comptime-codegen.mdx:22` | `C__examples__comptime-codegen__0__L22.shape` | Active implementation gap | In examples scope; page says `comptime for` over typed-target fields is gated on V3-S5 ckpt-6 typed-array-data rebuild. |
| `examples/web-request.mdx:22` | `C__examples__web-request__0__L22.shape` | External/manual/fixture/server/env/permission dependent | Needs a loopback HTTP/API fixture; page also notes native-result marshaling is still gated. |
| `fundamentals/async.mdx:123` | `A__fundamentals__async__7__L123.shape` | Preview/out-of-scope | Named `join all` result object is v0.4-style; current proved surface is ordered homogeneous values, not `.left` fields. |
| `fundamentals/content.mdx:51` | `B__fundamentals__content__1__L51.shape` | Preview/out-of-scope | `Content` trait auto-render and `c"..."` content strings are future/retired presentation surfaces. |
| `fundamentals/content.mdx:61` | `B__fundamentals__content__2__L61.shape` | Preview/out-of-scope | Custom `impl Content` chart rendering is described as v0.4 behavior. |
| `fundamentals/content.mdx:107` | `B__fundamentals__content__4__L107.shape` | Preview/out-of-scope | Auto-table rendering for struct collections is described as v0.4 behavior. |
| `fundamentals/content.mdx:453` | `B__fundamentals__content__14__L453.shape` | Preview/out-of-scope | `ContentFor<Adapter>` dispatch is explicitly v0.4 adapter rendering design. |
| `fundamentals/datetime.mdx:19` | `B__fundamentals__datetime__0__L19.shape` | External/manual/fixture/server/env/permission dependent | `DateTime.now()` is wall-clock/local-timezone dependent; keep out of deterministic default truth unless rewritten. |
| `fundamentals/datetime.mdx:364` | `B__fundamentals__datetime__17__L364.shape` | Stale-green/count-reduction candidate | Fixed timestamp timezone example looks deterministic; local grep shows DateTime parse/format/timezone tests. |
| `fundamentals/datetime.mdx:404` | `B__fundamentals__datetime__19__L404.shape` | Stale-green/count-reduction candidate | Fixed timestamp format/iso/unix example looks deterministic and covered by nearby DateTime literal/std tests. |
| `fundamentals/error-handling.mdx:90` | `A__fundamentals__error-handling__4__L90.shape` | Old syntax/book rewrite | Conceptual `AnyError` shape uses placeholder tokens like `<original value>`; should be prose or a non-Shape block. |
| `fundamentals/error-handling.mdx:186` | `A__fundamentals__error-handling__8__L186.shape` | Active implementation gap | Source-side `TryInto`/`as Type?` through `Convert` and AnyError remains the stated conversion gap. |
| `fundamentals/error-handling.mdx:207` | `A__fundamentals__error-handling__9__L207.shape` | Active implementation gap | Target-side `From` auto-derived `Into` needs `Convert` trait-dispatch/value-call wiring. |
| `fundamentals/error-handling.mdx:224` | `A__fundamentals__error-handling__10__L224.shape` | Active implementation gap | Target-side `TryFrom<Json>` auto-derived `TryInto` shares the conversion/AnyError gap. |
| `fundamentals/error-handling.mdx:275` | `A__fundamentals__error-handling__11__L275.shape` | Active implementation gap | `!!` context operator over fallible values is still called out as an inference/operator gap elsewhere in fundamentals. |
| `fundamentals/error-handling.mdx:287` | `A__fundamentals__error-handling__12__L287.shape` | Active implementation gap | `!!` plus `?` composition needs the same error-context inference path and scaffolded fallible operands. |
| `fundamentals/functions.mdx:229` | `A__fundamentals__functions__14__L229.shape` | Intentional diagnostic | Bad/duplicate named-argument example is meant to fail. |
| `fundamentals/functions.mdx:413` | `A__fundamentals__functions__26__L413.shape` | External/manual/fixture/server/env/permission dependent | `fn python` requires the Python extension runtime and frontmatter fixture. |
| `fundamentals/modules.mdx:80` | `A__fundamentals__modules__7__L80.shape` | External/manual/fixture/server/env/permission dependent | External `mylib` namespace and `input` are not part of the single extracted snippet. |
| `fundamentals/modules.mdx:191` | `A__fundamentals__modules__12__L191.shape` | External/manual/fixture/server/env/permission dependent | Two-file library example needs multi-file module fixtures. |
| `fundamentals/objects-arrays.mdx:37` | `A__fundamentals__objects-arrays__1__L37.shape` | Intentional diagnostic | Out-of-bounds index example is meant to raise a runtime error. |
| `fundamentals/objects-arrays.mdx:366` | `A__fundamentals__objects-arrays__20__L366.shape` | Active implementation gap | `HashMap.keys/values/entries` are declared but still tied to typed-array/consumer-cascade carrier work. |
| `fundamentals/operators.mdx:436` | `A__fundamentals__operators__23__L436.shape` | Active implementation gap | Page caution says `Result<T,E> !! string` is rejected by current binary-operator inference. |
| `fundamentals/operators.mdx:503` | `A__fundamentals__operators__28__L503.shape` | Active implementation gap | Cast-with-comptime-field-overrides surface needs conversion/type-assertion lowering support. |
| `fundamentals/references-borrowing.mdx:30` | `A__fundamentals__references-borrowing__1__L30.shape` | Intentional diagnostic | Use-after-move example is meant to fail with B0005. |
| `fundamentals/references-borrowing.mdx:73` | `A__fundamentals__references-borrowing__4__L73.shape` | Active implementation gap | `var` alias plus `.push` CoW path has a documented VM/JIT correctness blocker. |
| `fundamentals/references-borrowing.mdx:192` | `A__fundamentals__references-borrowing__11__L192.shape` | Intentional diagnostic | Escaping-reference example is meant to fail; split the positive `read_val` case if desired. |
| `fundamentals/references-borrowing.mdx:253` | `A__fundamentals__references-borrowing__13__L253.shape` | Intentional diagnostic | Mixed async task-boundary OK cases and expected exclusive-ref C0001 error. |
| `fundamentals/references-borrowing.mdx:269` | `A__fundamentals__references-borrowing__14__L269.shape` | Old syntax/book rewrite | Quick-reference block has placeholders (`value`, `...`) and should be prose/table, not executable Shape. |
| `fundamentals/resource-management.mdx:139` | `A__fundamentals__resource-management__5__L139.shape` | Stale-green/count-reduction candidate | Definition-only `Drop` trait row may be compile-only or better as prose; recheck in a serialized lane before flipping. |
| `fundamentals/resource-management.mdx:365` | `A__fundamentals__resource-management__13__L365.shape` | External/manual/fixture/server/env/permission dependent | Async drop example depends on database connection/subscription fixtures. |
| `fundamentals/resource-management.mdx:387` | `A__fundamentals__resource-management__14__L387.shape` | External/manual/fixture/server/env/permission dependent | Scoped async resource example depends on DB/query fixtures. |
| `fundamentals/strings.mdx:277` | `A__fundamentals__strings__13__L277.shape` | Active implementation gap | Typed `table(...)` f-string specs parse/lower, but runtime table-format rendering is deferred. |
| `fundamentals/strings.mdx:302` | `A__fundamentals__strings__14__L302.shape` | Active implementation gap | Same typed table-format rendering gap, with color/border options. |
| `fundamentals/strings.mdx:397` | `A__fundamentals__strings__18__L397.shape` | Old syntax/book rewrite | Retired `c"..."` inline styling syntax should remain prose/reference or be removed from executable fences. |
| `fundamentals/tables.mdx:56` | `B__fundamentals__tables__3__L56.shape` | Preview/out-of-scope | Table loading/query example is marked v0.4 preview and also depends on an app loader and file fixture. |
| `fundamentals/tables.mdx:76` | `B__fundamentals__tables__4__L76.shape` | Preview/out-of-scope | `from row in events where ... select ...` query syntax is marked v0.4 preview. |
| `fundamentals/tables.mdx:109` | `B__fundamentals__tables__5__L109.shape` | Active implementation gap | Chained table methods need current `Table<T>` method signature/type propagation alignment. |
| `fundamentals/tables.mdx:125` | `B__fundamentals__tables__6__L125.shape` | Old syntax/book rewrite | Trait body uses pseudo-signatures (`filter(predicate): any`) instead of current `method ... -> ...` syntax. |
| `fundamentals/traits.mdx:71` | `A__fundamentals__traits__3__L71.shape` | Active implementation gap | Named impl call-site dispatch with `using JsonDisplay` remains a trait-dispatch surface. |
| `fundamentals/traits.mdx:172` | `A__fundamentals__traits__8__L172.shape` | Active implementation gap | Generic trait impl dispatch still depends on generic trait argument resolution. |
| `fundamentals/traits.mdx:249` | `A__fundamentals__traits__11__L249.shape` | Active implementation gap | `From` auto-derived `Into` through `as` shares the conversion trait-dispatch gap. |
| `fundamentals/traits.mdx:265` | `A__fundamentals__traits__12__L265.shape` | Active implementation gap | `TryFrom` auto-derived `TryInto` through `as Type?` shares the conversion/AnyError gap. |
| `fundamentals/traits.mdx:330` | `A__fundamentals__traits__14__L330.shape` | Preview/out-of-scope | `extend Table<Row>` example uses table/row-spread/rolling-window design sketch rather than current runnable surface. |
| `fundamentals/traits.mdx:387` | `A__fundamentals__traits__17__L387.shape` | Active implementation gap | Associated type binding/substitution needs end-to-end trait impl resolution. |
| `fundamentals/variables.mdx:82` | `A__fundamentals__variables__4__L82.shape` | Active implementation gap | Same documented `var` alias plus `.push` CoW blocker as references-borrowing. |
| `fundamentals/variables.mdx:168` | `A__fundamentals__variables__8__L168.shape` | External/manual/fixture/server/env/permission dependent | `fs.read("config.txt")` needs a filesystem/import/permission fixture and also crosses `!!` composition. |

## Top Implementation Lanes

1. Trait conversion and error-context lane.
   Obvious files/tests from grep: `crates/shape-vm/src/compiler/statements.rs`,
   `crates/shape-vm/src/compiler/helpers.rs`,
   `crates/shape-vm/src/compiler/expressions/function_calls.rs`,
   `crates/shape-runtime/stdlib-src/core/{from,try_from,into,try_into,json_value}.shape`,
   `tools/shape-test/tests/{traits,error_handling,numeric_conversions}/**`.
   This owns `From`/`TryFrom`, auto-derived `Into`/`TryInto`, `as Type?`,
   named impl dispatch, associated types, and `!!` inference.

2. Ownership, CoW, borrow, and diagnostic split lane.
   Obvious files/tests from grep: `crates/shape-vm/src/compiler/{statements.rs,mod.rs}`,
   `crates/shape-vm/src/compiler/expressions/{advanced.rs,closures.rs}`,
   `crates/shape-vm/src/mir/{solver.rs,return_ownership.rs,lowering/**}`,
   `tools/shape-test/tests/{borrow_refs,async_concurrency,lsp/inlay_storage_class.rs}`.
   Keep intentional diagnostic snippets disabled until expected-error book truth
   exists, but isolate the real `var` alias CoW and async borrow-rule gaps.

3. Table, HashMap, and f-string table-format lane.
   Obvious files/tests from grep:
   `crates/shape-runtime/stdlib-src/core/{hashmap_methods,table_methods,queryable,table_queryable}.shape`,
   `crates/shape-vm/src/executor/objects/hashmap_methods.rs`,
   `crates/shape-vm/src/compiler/expressions/{collections.rs,function_calls.rs}`,
   `crates/shape-vm/src/compiler/string_interpolation.rs`,
   `tools/shape-test/tests/{hashmap,tables_queryable,strings_formatting}/**`.
   This attacks `HashMap.keys/values/entries`, `Table<T>` method propagation,
   `Queryable` syntax drift, and `f"{rows:table(...)}"` rendering.

4. Debug tools Shape-surface/proof-design lane.
   Obvious files from grep: `crates/shape-vm/src/{hot_reload.rs,executor/time_travel.rs}`,
   `crates/shape-runtime/src/{blob_prefetch.rs,execution_proof.rs}`. The decision
   is either expose a real `std::debug` module with tests or convert the Shape
   usage blocks in `advanced/developer-tools.mdx` to non-executable design prose.

5. Small deterministic DateTime/book-only lane.
   Obvious files/tests from grep: `crates/shape-vm/src/executor/{builtins/datetime_builtins.rs,objects/datetime_methods.rs}`,
   `tools/shape-test/tests/{datetime_stdlib,literals/datetime_literals.rs}`.
   Recheck fixed-time examples only; leave `DateTime.now()` host-time dependent.

## Book-Only Or Stale Candidates

- Flip or rewrite after a serialized recheck: `fundamentals/datetime.mdx:364`
  (`B__fundamentals__datetime__17__L364.shape`),
  `fundamentals/datetime.mdx:404`
  (`B__fundamentals__datetime__19__L404.shape`), and
  `fundamentals/resource-management.mdx:139`
  (`A__fundamentals__resource-management__5__L139.shape`).
- Convert to prose/non-Shape fences without production work:
  `advanced/ownership-deep-dive.mdx:259`,
  `fundamentals/error-handling.mdx:90`,
  `fundamentals/references-borrowing.mdx:269`,
  `fundamentals/strings.mdx:397`, and
  `fundamentals/tables.mdx:125`.
- Split mixed diagnostic snippets when expected-error truth support exists:
  `advanced/ownership-deep-dive.mdx:399`,
  `advanced/ownership-deep-dive.mdx:425`,
  `fundamentals/references-borrowing.mdx:192`, and
  `fundamentals/references-borrowing.mdx:253`.

## Uncertainty

This is a static triage. "Stale-green" means plausible count-reduction target
from manifest/page/source inspection, not a proven green snippet. Any flip still
needs the supervisor-owned verification lane.

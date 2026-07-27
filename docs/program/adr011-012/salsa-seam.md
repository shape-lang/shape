# The Salsa seam (#91)

**Authority:** ADR-013 (incremental engine, `SemanticDb` ownership), ADR-011 §1
and §6 (resolved definition identity; one query graph for compiler and tooling),
ruling R16 (this slice's scope and stop line), ruling R17 (non-vacuous tracer).
**Artifact:** `crates/shape-semantic-db/`.
**Consumers:** `bin/shape-cli/src/commands/semantic_facts_cmd.rs` (compiler),
`tools/shape-lsp/src/semantic_seam.rs` (tooling).
**Recorded at:** wave1-spine, on top of `07b57d76`.

ADR-013 leaves four things to the implementation ticket: the exact Salsa
release and features, the physical location of `SemanticDb`, cancellation and
snapshot ownership, and initial query-memory budgets. This document records all
four, plus the seam-local rulings this slice had to make and the questions it
deliberately left open.

## 1. The pinned dependency

```toml
salsa = { version = "=0.28.1", default-features = false, features = [
  "macros",
  "inventory",
  "salsa_unstable",
] }
```

| | |
|---|---|
| Release | `0.28.1`, pinned exactly (`=`). A floating requirement is noncompliant under ADR-013 §2. |
| Lockfile | `salsa 0.28.1` — `a14fdadbf856222e731756d7fdbdf193a7abf8fdab009bb45f48671a42719a84`; `salsa-macros 0.28.1` — `ec5c48c5a4a53a6e2be9762f56566f1325d4394c011c00b3ea86ba2d13411e71`. |
| Salsa's declared MSRV | 1.85 |
| Toolchain this was built and tested on | rustc 1.94.1 (devenv) |
| New transitive dependencies | `boxcar`, `foldhash`, `hashbrown 0.17`, `hashlink`, `intrusive-collections`, `inventory`, `thin-vec`, `typeid`, `salsa-macro-rules`, `salsa-macros` |

Features, and why each is on or off:

- **`macros`** — the `#[salsa::db]` / `input` / `interned` / `tracked` attribute
  API. Without it there is no usable surface.
- **`inventory`** — static ingredient registration used by those attributes. The
  alternative is manual jar registration through `Storage::builder()`, which
  buys nothing here and adds a way to forget an ingredient.
- **`salsa_unstable`** — `Database::memory_usage()`. This is how the
  query-memory budget in §6 is *measured* rather than asserted. The feature is
  unstable in Salsa's API-stability sense; it is used only by
  `SemanticSession::query_memory`, so an upstream change to it cannot affect
  any published fact.
- **`accumulator` (off)** — accumulated values deliberately do not participate
  in a query's result equality, which means they do not participate in early
  cutoff either. Diagnostics here are part of the published fact and must
  participate in both. Turning this on later would be a design change, not a
  convenience.
- **`rayon` (off)** — no parallel query fan-out in this slice.
- **`persistence`, `detailed-trace`, `shuttle`, `compact_str`, `ordermap`,
  `triomphe` (off)** — unused. ADR-013 explicitly does not require
  cross-session persistent memo storage.

**Upgrade policy.** Salsa is pinned exactly, so an upgrade is a deliberate
commit. It must re-run the six edit traces in
`crates/shape-semantic-db/src/acceptance.rs` (they assert engine behaviour —
what re-executes and what backdates — so an engine change surfaces there, not in
production), re-measure §6, and re-record this table. A Salsa release that
changes early-cutoff behaviour is a semantic event for this seam even though it
cannot change a published identity.

## 2. Placement

`SemanticDb` lives in its own workspace crate, `crates/shape-semantic-db`,
depending on `shape-ast`, `salsa`, `sha2` and `hex` — and nothing else.

The alternative was a module inside `shape-runtime` (where the type system
lives) or `shape-vm` (where the bytecode compiler lives). Both were rejected for
the same reason: R16's stop line says the bytecode compiler, programs, mutable
stacks, journals, backend caches and VM/JIT state are never Salsa inputs or
query-owned state. In a separate crate that is enforced by the dependency graph
— a query in this crate *cannot* reach `BytecodeCompiler`, because the crate
cannot see it. Inside `shape-runtime` the same guarantee would rest on review.

The consumers depend on the seam, never the reverse, so nothing here can become
entangled with compilation state later without someone adding a dependency edge
that is visible in a diff.

## 3. What Salsa owns; what Shape owns

Salsa owns database revisions, dependency recording, memo storage, red-green
validation, early cutoff, local interning and concurrent read coordination.

Shape owns every identity. `UnitIdentity`, `DefinitionIdentity` and every
content identity are domain-separated, length-framed SHA-256 digests computed in
`crates/shape-semantic-db/src/identity.rs` by functions that **take no
database**. That is the structural reason no Salsa id can leak into a portable
identity: there is no code path by which one could. `acceptance.rs`'s
`database_local_ids_do_not_reach_published_identities` builds the same program
with the units inserted in reverse order — so every Salsa id differs — and
asserts every published digest is unchanged.

### The identity pre-image

```text
UnitIdentity       = SHA-256( "shape.semantic.unit"       | scheme_version | unit_path )
DefinitionIdentity = SHA-256( "shape.semantic.definition" | scheme_version
                            | UnitIdentity | kind | scope_path | name | same_name_ordinal )
```

Every field is written length-prefixed, so no two distinct field sequences share
a pre-image (`framing_prevents_field_boundary_collisions`).

The declared **name** is in the pre-image. That is not "spelling as authority":
the name is the declaration's structural path inside its unit, the same role a
path segment plays. What is excluded is presentation — doc comments, formatting,
display spellings, byte spans, table positions, and use-site spelling. An import
alias is use-site syntax and never enters a definition's pre-image, which is
exactly why aliasing preserves identity (`alias_edit_*`) while a same-spelled
local declaration gets a different one (`local_shadow_edit_*`).

`same_name_ordinal` is ADR-011 §1's deliberately narrow disambiguator: it counts
only same-kind, same-name declarations in the same scope, in lexical order.
Inserting an unrelated sibling renumbers nothing
(`unrelated_sibling_insertion_does_not_renumber_the_tracer`).

`IDENTITY_SCHEME_VERSION` is 1 and is part of every pre-image, so a scheme change
changes every identity visibly rather than silently.

## 4. The published facts

R16 admits exactly four things into this slice, and the crate publishes exactly
those:

| Fact | Where |
|---|---|
| Resolved `DefinitionIdentity` | `ContractFacts::identity` |
| Normalized base contract | `ContractFacts::contract` (`CallableContract`) |
| Deterministic diagnostics | `ContractFacts::diagnostics`, `CallSiteFacts::diagnostics` |
| Source provenance | `CallableFacts::provenance` |

Two layers, and the split is load-bearing:

- **`ContractFacts` is span-free.** A body edit or a comment cannot change it.
- **`CallableFacts`** = contract facts + provenance + located diagnostics. This
  is the fact `callable_facts(DefinitionIdentity)` returns.

Call-site checking consumes the *contract* layer. That is why a comment that
shifts spans re-publishes a callable fact (the declaration genuinely moved) but
does not re-check a single call site, and why a signature edit re-checks them.
If both layers were one, every whitespace edit would invalidate every dependent
check in the program.

Diagnostics are structured — a frozen code, a severity and sorted key/value
arguments. The rendered message is presentation and is excluded from content
identity, so rewording a message cannot change a published fact. Compiler and
LSP render from the same structured value instead of parsing each other's
strings.

### Query graph

```text
SourceUnit (input: path, text)      ProgramInputs (input: unit map)
        |                                    |
   parsed_unit  (no_eq)                 unit_for_path
        |                                    |
   +----+-------------------+                |
   |                        |                |
declaration_index      unit_provenance       |
 (span-free)            (spans)              |
   |        \                 \              |
   |         \                 \             |
resolve_callable <--------------------------- +
   |                            \
callable_contract ---------------+--> callable_facts
   |
call_site_facts
```

`parsed_unit` is `no_eq`: the AST has no structural equality, so it cannot
backdate. That is deliberate rather than a gap — equality is established one
layer down by the span-free index, which is where early cutoff belongs. The cost
is one re-index per edited unit; the benefit is that nothing above the index has
to care about AST identity.

`definition_sites` (identity → declaration site) exists only for the
identity-keyed public entry point. It depends on every unit, so it is kept off
the ordinary path — resolve, then read facts — which touches only the units
involved.

## 5. Cancellation and snapshot ownership

A `SemanticSession` is single-owner. Mutation goes through `&mut self`
(`insert_unit`, `set_unit_text`), reads through `&self`. Rust's borrow checker
therefore enforces that no read is outstanding when a revision begins — the
invariant Salsa's cancellation machinery exists to protect at runtime.

This slice hands out no cross-thread snapshots (no `salsa::StorageHandle`
clones) and runs no parallel readers, so Shape owns no cancellation policy yet.
The LSP consumes an **ephemeral session per request**: it builds a session from
the buffer text, reads the facts, and drops it. That keeps the editor honest
about identity — it publishes exactly what the compiler publishes — while
deferring the long-lived-session design.

A later slice that keeps a long-lived LSP database must decide explicitly: which
thread owns `Storage`, how `StorageHandle` clones are distributed, and what
happens to an in-flight request when an edit arrives (Salsa's answer is
`Cancelled` unwinding, which the server must catch per request). That decision
is not made here, and nothing here presumes it.

## 6. Initial query-memory budget

Measured with `Database::memory_usage()` on the three-unit tracer program after
demanding every published fact:

| | bytes |
|---|---|
| Salsa struct fields + metadata | 704 |
| Memo storage across all 8 queries | 1,848 |
| **Total Salsa bookkeeping** | **2,552** |

Per query: `callable_facts` 416, `declaration_index` 336, `call_site_facts` 304,
`callable_contract` 296, `resolve_callable` 208, `parsed_unit` 144,
`unit_provenance` 88, `unit_for_path` 56.

**What this number is.** Salsa's own bookkeeping — struct fields, memo slots,
metadata. It is *not* total memory: heap reachable through a memoized value (the
`Arc<Program>` a parse holds) is only reported for ingredients that declare a
`heap_size` function, and this slice declares none. Anyone reading this as "the
seam costs 2.5 KB" would be wrong; the AST dominates and is not counted.

**The budget.** 16 KiB, asserted by
`query_memory_stays_within_the_recorded_budget`. It is a tripwire on structural
growth of the bookkeeping — a query keyed on something unexpectedly large, or a
memo that should have been backdated being stored per revision — not a
performance target. A real memory baseline on a representative workspace is
ADR-013 §7's performance requirement and is not claimed here.

## 7. The stop line, and how it is held

R16: no annotations, generated symbols, method tables, discovery, comptime,
typed Core/MIR, or backend state migration. `BytecodeCompiler`, programs,
mutable expression/function stacks, journals, backend caches and VM/JIT state
are never Salsa inputs, tracked values, or query-owned mutable state.
`BytecodeEmitter` remains an ephemeral mutable consumer.

Held structurally: `shape-semantic-db` depends on `shape-ast`, `salsa`, `sha2`
and `hex`. It has no dependency on `shape-vm`, `shape-runtime`, `shape-jit` or
`shape-wire`, so none of the forbidden state is nameable from a query. The
consumers depend on the seam; the seam depends on no consumer. Nothing in the
existing compilation path was rerouted through Salsa — the tracer path is a
parallel query over the same source text, which is what "narrow" means here.

## 8. The six edit traces

Each trace declares what should re-execute and what should cut off, then
measures it against Salsa's own event stream (`WillExecute` /
`DidValidateMemoizedValue`), not against counters the query bodies increment.
`trace_events_name_the_query_and_the_unit` tests the instrument itself, so a
"0 executions" assertion cannot pass because the matcher never matched.

Tracer (fixed by R17, not substituted): `app::math` declares
`pub fn add(a: int, b: int) -> int`; `app::main` imports it and calls it once.

| # | Edit | Declared | Measured |
|---|---|---|---|
| 1a | Comment appended after the declaration | parse, index, provenance re-execute and backdate; nothing semantic re-executes | `parsed_unit(app::math)` 1, `declaration_index(app::math)` 1, `unit_provenance(app::math)` 1, `callable_contract` 0, `callable_facts` 0, `call_site_facts` 0, `resolve_callable` 0, `parsed_unit(app::main)` 0. Fact content identity unchanged. |
| 1b | Comment inserted *above* the declaration (spans shift) | contract still cuts off; only the callable fact re-executes, because provenance is part of it; no call site re-checked | `declaration_index(app::math)` 1, `unit_provenance(app::math)` 1, `callable_contract` **0**, `callable_facts` **1**, `call_site_facts` **0**. Contract content identity unchanged; `name_span` changed; call-site fact unchanged. |
| 2 | Body edit `a + b` → `b + a` | full cutoff; no published fact changes | `parsed_unit` 1, `declaration_index` 1, `unit_provenance` 1, `callable_contract` 0, `callable_facts` 0, `call_site_facts` 0. Callable fact and call-site fact byte-identical. |
| 3 | Signature edit `a: int` → `a: string` | contract, fact and call-site check re-run; the call site gains a diagnostic | `declaration_index(app::math)` 1, `callable_contract` 1, `callable_facts` 1, `call_site_facts` 1. Identity unchanged (same declaration), content identity changed, `params[0].ty == String`, call site publishes `SEMDB0011` with `expected=string actual=int index=0`. |
| 4 | Import retarget `app::math` → `app::math2` | resolution names a different identity; the abandoned unit is untouched | `parsed_unit(app::main)` 1, `declaration_index(app::main)` 1, `resolve_callable` 1, `declaration_index(app::math2)` 1, `declaration_index(app::math)` **0**, `parsed_unit(app::math)` **0**, `call_site_facts` 1. Callee identity is now `app::math2`'s `add`; the contract is identical, so identity is demonstrably not the contract. |
| 5 | Alias `add` → `add as plus` (call site rewritten) | identity carries through; the callee's contract and fact do not re-execute at all | `declaration_index(app::main)` 1, `resolve_callable` 1, `declaration_index(app::math)` **0**, `callable_contract` **0**, `callable_facts` **0**, `call_site_facts` 1. Callee identity and callee contract identity unchanged; `written_name == "plus"`. |
| 6 | Local `fn add(a: string, b: string) -> string` added to the consuming unit | the homonym resolves to its own identity; the imported unit is not re-examined; the call is re-checked against the homonym | `declaration_index(app::main)` 1, `resolve_callable` 1, `declaration_index(app::math)` **0**, `callable_contract` 1 (new key), `call_site_facts` 1. Callee identity is `app::main`'s `add`, not `app::math`'s; two `SEMDB0011`; one `SEMDB0003` (shadowed import). |

Traces 5 and 6 together are R17's positive-alias / negative-homonym pair, and
they are measured rather than asserted: in 5 the aliased callee's queries do not
run *because* the identity was equal, and in 6 they do not run *because* the
identity was different.

## 9. Seam-local rulings

This slice needed four decisions that Shape has not otherwise fixed. Each is a
decision, not a discovery, and each is a candidate for revision when the full
resolver arrives.

1. **A local declaration shadows an import of the same name**, and the seam
   publishes `SEMDB0003` (warning) saying so. The alternative — rejecting the
   collision outright, as Rust does — is also defensible; local-wins was chosen
   because it makes the homonym's *effect* observable downstream (the call is
   re-checked against the local contract) rather than collapsing to a resolution
   error. This is the seam's rule; Shape's existing compiler is not consulted.
2. **Unit path derives from the file stem** (`unit_path_for_file`). Both
   consumers call that one function, which is what keeps their identities equal.
   Shape's real module-path resolution (shape.toml / frontmatter) is not part of
   this slice.
3. **Call sites are collected in declared forms only** — unit-level expressions
   and variable initializers; expression statements, variable initializers and
   returns inside function bodies; and calls nested in binary/unary operands and
   call arguments. The traversal is intentionally non-exhaustive over `Expr` so a
   new AST variant cannot silently break it. A call in an unsupported form is
   *not published*; it is never published with approximated facts.
4. **Only literal arguments are type-checked at a call site.** A non-literal
   argument is published as `None` with a `SEMDB0012` note naming the gap.
   Expression type inference is not in this slice, and the fact says so rather
   than guessing.

Gaps are published, not hidden: `SEMDB0008`/`SEMDB0009` record an undeclared
result or parameter type, `NormalizedType::NotDeclared` and
`NormalizedType::Unsupported` are explicit values rather than a silent default.

## 10. Not covered by this slice

- **Type names are not resolved.** A named type normalizes to its written path.
  This slice publishes callable identity, not type identity.
- **Inferred result types.** ADR-011 allows unannotated callables to retain
  inferred results; body inference is outside the stop line, so the seam
  publishes `NotDeclared` plus a note.
- **Qualified and namespace-imported calls** (`m::f(...)`, `use m::path`).
- **Methods, traits, impls, annotations, generated symbols** — all beyond the
  stop line by construction.
- **Semantic cycles.** ADR-013 §7.4 requires a cycle to produce a Shape
  diagnostic rather than a Salsa panic. This slice's query graph is acyclic by
  construction (no query can reach itself), so there is nothing to demonstrate;
  the obligation lands with the first slice that can recurse — declaration
  discovery. It is *not* discharged here and should not be counted as such.
- **Performance.** Adopting Salsa is not a performance result (ADR-013 §7). No
  edit-latency or re-execution baseline on a representative workspace is claimed.

## 11. Open questions for the next slice

1. **Unit-path derivation.** Adopting real module resolution will change
   published identities for any file whose module path is not its stem. Since
   identities are meant to be stable across processes, that is a scheme
   migration, and `IDENTITY_SCHEME_VERSION` exists to make it visible.
2. **Shadowing precedence** (§9.1) needs ratification against whatever Shape's
   full resolver does, so that the seam and the compiler cannot disagree about
   which definition a name means.
3. **Long-lived LSP session**: cancellation and snapshot policy (§5).
4. **`parsed_unit`'s `no_eq`.** If re-indexing per keystroke ever shows up in a
   latency measurement, the fix is structural equality on the AST (or a
   syntax-tree library that provides it), not a shim.

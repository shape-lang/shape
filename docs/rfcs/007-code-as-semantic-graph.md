# RFC-007: Code-as-Semantic-Graph Database

- **Status:** Draft
- **Author:** Shape language team
- **Date:** 2026-05-18
- **Discussion target:** `shape-lsp`, `shape-mcp`, `shape-runtime` maintainers

## Summary

Shape already builds, for every function it compiles, a self-describing
record: name, arity, parameter names, content hash, required permissions,
dependency hashes, callee names, type-schema references, foreign
dependencies, and a source map. This record is `FunctionBlob`
(`crates/shape-vm/src/bytecode/content_addressed.rs:33-92`). It is fact-shaped.
It is currently consumed by exactly one client (the linker) for exactly one
purpose (transitive permission union, `crates/shape-vm/src/linker.rs:329`).

This RFC proposes a small, derived **semantic graph database** over those
facts. Nodes are the entities Shape already names — `FunctionBlob`, `Type`,
`Trait`, `CapabilityTag`, `Annotation`. Edges are typed predicates:
`(predicate, subject_hash, object_hash, metadata)`. Storage is SQLite via
`rusqlite`; free-text search over docstrings and `@intent` annotations is
Tantivy. The query surface is an embedded Angle-subset DSL exposed through
`shape-lsp` (workspace symbols, code lenses, side-panel REPL) and through
`shape-mcp` (a single `query_graph` MCP tool).

The substrate change is small but the affordance is large: queries that are
clumsy with grep and impossible with LSP workspace-symbols become one
line. The integration with the rest of the v0.4 RFC series is direct —
RFC-002's `@law`, RFC-004's `#[replaces]`, and RFC-007's own `@intent` are
all just new fact predicates. No new sigils.

**Adoption cost is zero**: files remain the canonical artifact for git, for
diffing, for review, for content-hashing. The graph is *derived* — rebuilt
incrementally from sources via salsa-style invalidation. Users who don't
care about the graph never see it.

**The compiler must never consult the graph.** This RFC is for
IDE/LSP/MCP discovery only. The `Forbidden Patterns` section in
`CLAUDE.md` documents Shape's history with parallel-implementation
defections; a canonical-graph backdoor into the compiler would be exactly
that pattern. The guard-rail is architectural (the graph crate has no
reverse dependency edge to `shape-vm` or `shape-runtime`) and verified by
the existing `just check-no-dynamic` recipe with one added forbidden
symbol.

## Motivation

### The grep failure mode

LLM-author sessions in this codebase consistently fail along the same
shape: a question is asked that should be answerable in seconds —
*"which functions require `FsWrite`?"*, *"what calls
`__intrinsic_typed_array_push`?"*, *"which types implement the `Drop`
trait?"* — and the agent burns 30-90 seconds of tool calls fanning out
grep, ripgrep, and `Glob` invocations across the workspace. The results
are textually correct but structurally noisy: a hit in a comment counts
the same as a hit in a call site; a hit on a trait method name counts the
same as a hit on the trait declaration; a renamed-but-not-deleted
identifier produces phantom matches.

The single most expensive instance of this failure mode in Shape's history
is documented in `CLAUDE.md` under *Forbidden Patterns* — the W-series
`ValueWord` shim regression. A nine-commit detour cost an estimated 4-6
weeks of cumulative cleanup, and its proximate enabler was that "is this
`ValueWord` decode used anywhere?" was answered by reading nine files
across three crates. The structural answer — *which `FunctionBlob`s have
a `calls` edge to `synthesize_value_word_from_raw`, transitively?* — was
unavailable. So the rename survived review.

### Queries grep can't answer

Grep is line-oriented; it has no model of *call*, *implements*, or
*requires*. The following are typical IDE/MCP queries that fall out of a
fact-graph immediately and are infeasible with text tools:

- **Transitive permission audit.** *"Show me every function reachable
  from `main` that requires `NetConnect`, with the path."* The linker
  already computes the union (`linker.rs:329`); the graph generalizes
  this to a `WITH RECURSIVE` query that returns paths, not just sets.
- **Reverse capability search.** *"Which functions can perform `FsWrite`
  without also taking a `FsScoped` constraint?"* A two-predicate join.
- **Type-construction sites.** *"Where is `TypedObjectStorage`
  constructed in user code (not stdlib)?"* `constructs_type` edge,
  filtered on module path.
- **Trait-impl coverage.** *"Which public types implement neither
  `Display` nor `Debug`?"* Anti-join over `implements_trait`.
- **Annotation roll-up.** *"List every function annotated `@intent` with
  text matching 'parse', grouped by realizing module."* `annotated_with`
  + Tantivy text predicate.
- **Cross-language dependency.** *"Which Shape functions call into
  Python via `fn python`?"* `calls_foreign` filtered on `target_lang`
  metadata.

LSP workspace-symbols can answer none of these. Sourcegraph can answer
some at the URL level. A graph rebuilt from `FunctionBlob` answers all
of them in milliseconds at the FunctionBlob counts Shape will reach
(see *SQLite WITH RECURSIVE perf* below).

### Integration with RFC 001 / 002 / 004

The motivation strengthens when other v0.4 RFCs are read together:

- **RFC-002 (laws)** introduces `@law` annotations on functions and
  traits. Without a graph, *"show me every implementation of `Monoid`
  that fails to assert its associativity law"* is grep-and-eyeball.
  With a graph, it's `implements_trait` ∧ ¬(`has_law` → `associativity`).
- **RFC-004 (replaces)** introduces `#[replaces]` for evolving APIs.
  Without a graph, *"which call sites still hit the old API?"* requires
  a code search per replacement. With a graph, `calls` ∧ `refines` is
  the answer.
- **RFC-007 (this RFC)** introduces `@intent`, a property annotation
  capturing a natural-language description of a function's *purpose*
  (distinct from its docstring, which describes *behavior*). LLM tools
  query `realizes_intent` to find candidate functions for a high-level
  task. Files don't have a "purpose" index. Graphs do.

Each RFC contributes new edge predicates. The graph is the integration
surface.

## Guide-level explanation

### Querying from the LSP side-panel REPL

`shape-lsp` gains a `shape.graphQuery` command, surfaced by VSCode and
other LSP clients as a side-panel input. The user types an Angle-subset
query and gets a clickable result list:

```angle
predicate FsWriters() : Function =
  Function F where
    F.requires_permission "FsWrite"

FsWriters()
```

The reply is a list of `FunctionBlob.content_hash` references, each
rendered with `name`, `module`, and a "go to definition" link. The user
clicks; LSP opens the source file at the function's first source-map
entry.

### Code-lens upgrade

`tools/shape-lsp/src/code_lens.rs:43` currently counts references via
text-match (it scope-aware-falls-back, but it remains textual). The
graph swaps this for `calls`-edge cardinality, which is structural
and cross-file:

```text
fn parse_config(src: string) -> Result<Config, ParseError>
//  ^ 12 callers · requires [FsRead, FsScoped] · realizes "parse user config"
```

The three lens elements are three graph queries:
`count calls(_, parse_config)`,
`sum requires_permission(parse_config, _)`,
`select realizes_intent(parse_config, _)`.

Cost: ~50 µs per lens at the FunctionBlob counts the stdlib reaches today
(see *Performance* below). The textual `count_references` it replaces
costs ~2 ms on the same input and is wrong for renamed identifiers.

### Annotating intent

`@intent` is a property annotation. The compiler does nothing with it
(per the never-consult guard-rail) — it lands in `FunctionBlob.metadata`
and becomes a graph edge:

```shape
@intent("Parse a TOML config file, returning a typed Config struct.")
fn parse_config(path: string) -> Result<Config, ParseError> {
    // ...
}
```

The graph extractor reads `@intent` from the function's annotation list,
inserts a `realizes_intent(parse_config_hash, intent_string_id)` fact,
and Tantivy indexes the intent text. LLM agents query intent-text via
the MCP tool. Humans never type it directly.

### Querying from the CLI

```bash
shape graph query 'select Function F where F.requires_permission "NetConnect"'
shape graph rebuild        # full reindex (slow path)
shape graph status         # facts count, last incremental update, index size
shape graph explain <hash> # full fact dump for one FunctionBlob
```

### MCP tool call from an LLM-author session

The existing `shape-mcp` server gains one tool:

```jsonc
// tool: query_graph
{
  "name": "query_graph",
  "description": "Run an Angle-subset query against the workspace semantic graph.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query":   { "type": "string", "description": "Angle-subset query" },
      "limit":   { "type": "integer", "default": 50 },
      "explain": { "type": "boolean", "default": false }
    },
    "required": ["query"]
  }
}
```

Session example. An LLM-author is asked to add a new I/O capability:

> User: *"Add a `read_csv_streaming` function. Make sure it follows the
> same permission pattern as the other CSV readers."*
>
> Agent (internally):
> ```
> query_graph: select Function F where F.imports "std::core::csv" and F.requires_permission "FsRead"
> → [csv::read_records, csv::read_records_typed, csv::iter_records]
> query_graph: select Annotation A from Function F where F = "csv::read_records" and F.annotated_with A
> → [{kind: @intent, text: "Read a CSV file into a Vec of records"},
>    {kind: @example, text: "..."}]
> ```
> Agent: *"The existing CSV readers use `FsRead` only (no `FsScoped`).
> They all carry `@intent` and `@example`. I'll match that pattern."*

The agent's first action stopped being "grep for `csv`". It became "ask
the graph". The graph is right by construction.

## Reference-level explanation

### Storage layout

Two indices, kept strictly separate:

1. **SQLite** (`workspace/.shape/graph.db`) — relational facts.
2. **Tantivy** (`workspace/.shape/text-index/`) — free-text over
   docstrings and `@intent` annotation bodies.

The book-documentation Tantivy index in `shape-mcp/src/content/loader.rs`
(TF-IDF + trigram + synonym expansion over the Astro book) is a **separate
index** with separate input, separate lifecycle, and separate consumer
intent. The two indices share a library (Tantivy) and share nothing else.
The MCP server holds references to both; the existing `search_shape_docs`
tool routes to the book index; the new `query_graph` tool routes to the
workspace graph. They never merge.

SQLite is chosen over a custom store (which is what Glean built) because
the cost/benefit is overwhelming at Shape's scale. Glean's bespoke
storage was a forced move at the scale at which Meta indexes — billions
of facts across tens of millions of source files
([engineering.fb.com Dec 2024](https://engineering.fb.com/2024/12/19/developer-tools/glean-open-source-code-indexing/)).
Shape workspaces in the relevant 5-year window are O(10⁴) functions,
O(10⁵) facts. SQLite handles tens of millions of rows on commodity
hardware with millisecond-grade recursive-CTE traversal when the
edge-endpoint columns are indexed
([sqliteforum.com](https://www.sqliteforum.com/p/sqlite-and-graph-hybrids),
[dev.to/rohansx](https://dev.to/rohansx/sqlite-as-a-graph-database-recursive-ctes-semantic-search-and-why-we-ditched-neo4j-1ai)).
The ceiling that begins to bite — 500k entities at depth-6 traversal — is
two orders of magnitude past the workspace size we are designing for.
When (if) Shape exceeds that, the storage layer is swappable; the
predicate schema is what we are stabilizing here.

### Fact predicate schema — the 10 concrete predicates

Every fact has the same row shape:

```
facts (
    predicate    TEXT     NOT NULL,
    subject      BLOB(32) NOT NULL,   -- subject content hash
    object       BLOB(32),            -- object content hash or interned-string id
    object_text  TEXT,                -- inline string for string-object predicates
    metadata     BLOB,                -- MessagePack-encoded predicate-specific meta
    source       BLOB(32),            -- the FunctionBlob hash the fact was extracted from
    PRIMARY KEY (predicate, subject, object, object_text)
)
CREATE INDEX facts_subject_idx ON facts (subject, predicate);
CREATE INDEX facts_object_idx  ON facts (object, predicate);
```

The minimum-viable predicate set (the 80/20 cut):

| # | Predicate | Subject | Object | Metadata | Sourced from |
|---|---|---|---|---|---|
| 1 | `defines` | Module hash | Function/Type/Trait hash | kind, name | `ModuleGraph` + compiler |
| 2 | `calls` | Function hash | Function hash | call-site count | `FunctionBlob.dependencies` |
| 3 | `calls_foreign` | Function hash | foreign hash | target_lang | `FunctionBlob.foreign_dependencies` |
| 4 | `requires_permission` | Function hash | Permission name | scope (if scoped) | `FunctionBlob.required_permissions` |
| 5 | `imports` | Module hash | Module hash | re-export bool | `ModuleGraph` (`module_graph.rs:148`) |
| 6 | `implements_trait` | Type hash | Trait hash | impl module hash | compiler (`impl Trait for T` items) |
| 7 | `has_method` | Type hash | Function hash | method name | compiler |
| 8 | `constructs_type` | Function hash | Type hash | construction-site count | `FunctionBlob.type_schemas` |
| 9 | `annotated_with` | Function/Type/Trait hash | Annotation-name id | annotation args (MP-encoded) | compiler annotation pass |
| 10 | `realizes_intent` | Function hash | intent-text id | tantivy doc-id | `@intent` annotation extractor |

Reserved (no rows emitted until the relevant RFC lands):

| Predicate | Reserved for |
|---|---|
| `has_law` | RFC-002 — `@law` annotation on traits and functions |
| `refines` | RFC-004 — `#[replaces]` attribute on functions |

The schema does not version-stamp individual rows — the whole DB carries
a `schema_version INTEGER` in a one-row `meta` table, and migrations
follow the SQLite convention (`ALTER TABLE` for additive changes,
`CREATE TABLE _v2 ... INSERT SELECT ... DROP _v1` for breaking ones).

### Ingestion pipeline

```
.shape source files
  │
  │  (existing) shape-vm compiler
  ▼
FunctionBlob {                          ←── canonical fact source
    content_hash, name, arity,
    required_permissions,
    dependencies,
    callee_names,
    type_schemas,
    foreign_dependencies,
    source_map,
    ...
}
  │
  │  (new) graph extractor (shape-vm crate, behind a feature flag)
  ▼
Vec<Fact>
  │
  │  (new) shape-graph crate
  ▼
SQLite + Tantivy  ←── derived index
  │
  ├── shape-lsp ───► hover, code-lens, workspace-symbols, REPL panel
  └── shape-mcp ───► query_graph tool
```

The extractor is deterministic and side-effect-free: it consumes a
`&[FunctionBlob]` and emits `Vec<Fact>`. The same blob always produces
the same fact set, because the blob's content hash by construction
covers every field the extractor reads (verified by adding the extractor
output to the existing hash-stability sentinel test).

### Incremental invalidation — the salsa pattern

The graph is rebuilt from `FunctionBlob`s, which are content-addressed.
The invalidation rule is consequently almost trivial: when a file
changes, the compiler produces new `FunctionBlob`s for the affected
functions; the new hashes differ from the old hashes; the extractor
deletes facts whose `source` is in the old-hash set and inserts facts
whose `source` is in the new-hash set. No cross-file analysis is
required — the same property that gives the linker its incremental story
(rebuild only the blobs whose inputs changed) gives the graph its
incremental story for free.

This is the same shape as rust-analyzer's salsa-based dependency
tracking, but cheaper, because Shape's content-hash discipline is
already stricter than rust-analyzer's query memoization. The empirical
question — *what fraction of facts churn per typical edit?* — is
answerable from `shape-vm` benchmarks: edit-a-function-body changes one
blob hash; edit-a-public-signature changes that blob plus its dependents
(typically ≤10 in stdlib). At those churn rates, full incremental
update is sub-100 ms on a warm SQLite cache, comfortably below the LSP
diagnostic-refresh budget.

GitHub's stack-graphs takes a complementary approach: file-incremental
graph construction without invoking language tooling
([github.blog](https://github.blog/open-source/introducing-stack-graphs/),
[arxiv:2211.01224](https://arxiv.org/pdf/2211.01224)). Stack-graphs is
optimized for the cross-repository case (no build system, no shared
type-checker). Shape has a unified compiler and content-addressed blobs;
salsa-style invalidation is the cheaper fit. The stack-graphs incremental
story remains an existence proof that file-level incrementalism is
production-viable. (Note: GitHub's stack-graphs repo is no longer
actively maintained, but the design and benchmarks remain instructive.)

### Query planner

The Angle-subset DSL we expose is a strict subset of Glean's Angle
([glean.software/docs/query/intro/](https://glean.software/docs/query/intro/)):

- Predicate application: `Function F where F.calls G`
- Conjunction: comma-separated conditions
- Negation: `not F.requires_permission "FsWrite"`
- String literal matching: `F.name = "parse_config"`
- Path queries via recursion: `F transitively_calls G` desugars to a
  SQLite `WITH RECURSIVE` over the `calls` table

Disjunction, aggregation, and `derived predicate` definitions are
deferred to Phase 3. We resist the temptation to support arbitrary
Angle (or worse, arbitrary Cypher) because the query surface is the
stability contract and the smaller surface is easier to keep stable
across SQLite-schema migrations.

The planner is straightforward — every Angle-subset query maps to one
SQL query with at most one CTE. The planner does no cost-based
optimization; SQLite's planner is good enough at the cardinalities we
hit. We do force-pin the index choice on the recursive-CTE join column
(`source` or `subject` depending on the predicate direction) because
SQLite's planner occasionally picks the wrong index on small tables.

### LSP integration points

| Capability | Today | With graph |
|---|---|---|
| `workspaceSymbol` | textual prefix match on identifier index | `defines` predicate + name LIKE |
| `references` (`textDocument/references`) | scope-aware + text fallback (`code_lens.rs:43`) | `calls` predicate reverse lookup |
| `codeLens` | reference count, "N implementations" via grep | reference count via `calls`; permission summary via `requires_permission`; intent text via `realizes_intent` |
| `hover` | type signature + docstring | adds: callers count, transitive permission set, realized intents |
| **`shape.graphQuery`** | — | **new command**: Angle-subset query, results as workspace edits or code-lens list |

The LSP and MCP servers must share the **same** graph file. The
recommended deployment is a single `shape-graph` library, opened
read-only from both servers (SQLite's WAL mode supports unlimited
readers with one writer). The writer is `shape-lsp` (which has access
to compiler events on file save); `shape-mcp` reads only. CLI `shape
graph rebuild` is the cold-start writer.

### MCP tool surface

`shape-mcp` already exposes six tools
(`shape-mcp/src/tools.rs:62-130`). The graph adds one tool —
`query_graph` — and not six. Resisting per-predicate tools (e.g.,
`find_callers`, `find_perms`, `find_traits`) is deliberate: the LLM
benefits from learning one composable surface, the same way it benefits
from learning SQL once rather than seven REST endpoints.

### Relationship to the existing book-documentation Tantivy index

The MCP server's `search_shape_docs` tool indexes the Astro book content
shipped at build time via `include_str!`
(`shape-mcp/src/content/loader.rs:27`). It targets human-authored prose
about the *language*: tutorials, builtin-type explanations, error-handling
guidance. The synonym table maps cross-language vocabulary (`struct →
type`, `lambda → closure`) onto Shape terminology.

The workspace graph indexes user-authored prose about *user code*: the
`@intent` annotation bodies, function-leading docstrings, and (later)
`@law` text. The synonym table does not apply — `@intent` text is
domain-specific to whatever the user is building.

The two indices live in separate Tantivy directories, in separate
processes' working trees (one is baked into the MCP binary; the other
is per-workspace under `.shape/text-index/`). They are not merged; an
LLM that wants both performs two tool calls. This separation is
load-bearing: merging them would conflate "what does Shape do?" with
"what does this workspace do?", which is exactly the kind of source-of-truth
confusion Shape avoids elsewhere.

### Guard-rail: the compiler must never consult the graph

The single most important architectural rule in this RFC.

Shape's history with *parallel implementations across producer/consumer
boundaries* is documented in `CLAUDE.md` § *Forbidden Patterns*. Every
parallel discriminator we have added has eventually drifted; every
"derived index" that became consulted by the producer became a second
canonical source. The W-series ValueWord shim is the most expensive
instance. The graph, if compilers were allowed to read it, would be the
same pattern at a larger scale: type inference would gain a "first check
the graph" path, the graph would gain ad-hoc predicates serving the
compiler's narrow needs, and within a year we would have two type systems.

The mechanical enforcement is layered:

- **No reverse edge in `Cargo.toml`.** The graph crate (`shape-graph`,
  Phase 1) depends on `shape-vm`, `shape-runtime`, `shape-abi-v1`,
  `shape-ast`. **None of those depends on `shape-graph`.** A cyclic
  dependency check (`cargo metadata`-based, run from `xtask`) catches
  any future attempt to add the reverse edge.
- **`just check-no-dynamic` adds one symbol.** The recipe already greps
  the workspace for forbidden runtime symbols. We add `shape_graph::`
  to the deny-list, scoped to `crates/shape-vm/`, `crates/shape-runtime/`,
  `crates/shape-jit/`, `crates/shape-types/`. Build fails on hit.
- **Sentinel test.** A unit test under `tools/xtask/` parses
  `Cargo.toml` for each producer crate and asserts that `shape-graph`
  appears in no `[dependencies]` block.

Files remain canonical. The compiler reads files. The graph reads what
the compiler emits. The arrow does not reverse.

## Drawbacks

- **Graph staleness during edits.** Between a file save and the
  extractor finishing, the graph is briefly inconsistent. We mitigate
  with a `graph_generation: u64` counter incremented per write batch
  and exposed via LSP; the side-panel REPL grays out results from a
  stale generation. The staleness window is the same window during
  which LSP diagnostics are stale, so users already accept it.
- **Storage size growth in large workspaces.** A `FunctionBlob` produces
  on the order of 5-15 facts. At 100k functions (the largest single
  Shape workspace we can imagine in the v0.4 window), 1.5M facts, ~150
  MB SQLite + ~50 MB Tantivy. This is acceptable for `.shape/` and is
  gitignored by default. We add a `shape graph compact` recipe that
  vacuums and rebuilds Tantivy.
- **Query complexity ceiling.** Angle-subset will not answer every
  question; users with hard queries will hand-write SQL against
  `graph.db`. We do not consider this a defect — the escape hatch is
  the underlying SQLite, which is well-documented and stable.
- **MCP integration surface area.** One additional tool (`query_graph`)
  increases MCP's surface by ~17% (1 of 7). The risk is LLM confusion
  between `search_shape_docs` (the book) and `query_graph` (workspace
  facts). Tool descriptions name the distinction explicitly.
- **Index synchronization cost.** The writer is `shape-lsp`; if the LSP
  is not running (e.g., terminal-only development), facts go stale.
  `shape graph rebuild` is the recovery path. We do **not** auto-rebuild
  on `shape build` — the build path stays on the critical-path budget
  and the graph is non-essential.

## Rationale and alternatives

### Why SQLite, not a custom store like Glean

Glean's bespoke columnar store
([github.com/facebookincubator/glean](https://github.com/facebookincubator/glean))
was forced by Meta's scale — billions of facts, federated cross-repo
queries, query-time fact derivation. At Shape's scale, SQLite is
sufficient (and demonstrated viable for graph workloads:
[dev.to/rohansx](https://dev.to/rohansx/sqlite-as-a-graph-database-recursive-ctes-semantic-search-and-why-we-ditched-neo4j-1ai)
covers a production deployment that replaced Neo4j with SQLite recursive
CTEs at low-millions-of-rows scale). The benefits of SQLite are too
numerous to skip: zero-deploy, single file (gitignorable), already a
project dependency, well-understood backup story, and a query language
that every contributor can read. The cost is a ceiling at very large
graph traversals, which we are an order of magnitude away from
encountering.

### Why files stay canonical

This is the *non*-negotiable. Three reasons:

1. **Git is the world's source of truth for code.** Diff, review,
   blame, history — all file-based. A graph that became canonical
   would need a graph-aware git, which does not exist.
2. **Shape's adoption story depends on incrementalism.** A user can
   start writing Shape today, in any editor, with no understanding
   of the graph. The graph appears only if they install `shape-lsp`
   or run `shape graph`. Zero-buy-in is the bar.
3. **The defection-attractor history.** A canonical graph would, by
   the same pattern documented in `CLAUDE.md`, eventually be read by
   the compiler. The only safe topology is *files canonical, graph
   derived, arrow one-directional*.

Glean takes the same position — facts are derived from sources, not
the other way around. The Glean Dec 2024 retrospective is explicit:
*"the source code is always the source of truth"*
([engineering.fb.com](https://engineering.fb.com/2024/12/19/developer-tools/glean-open-source-code-indexing/)).

### Why an Angle-subset DSL, not Cypher or SPARQL

Cypher's grammar is large and tied to Neo4j idioms (variable-length
relationships, pattern-comprehensions) that we'd partially implement
and confuse users with. SPARQL is RDF-centric and over-general for
typed predicates over a fixed schema. Angle was designed for exactly
this problem ([glean.software/docs/query/intro/](https://glean.software/docs/query/intro/));
the subset we adopt is the half of Angle that maps cleanly to SQL.

### Why LSP-integrated, not a separate daemon

A daemon would duplicate `shape-lsp`'s file-watching, compile-event
plumbing, and configuration loading. The marginal cost of adding a
sidecar query thread to the existing LSP is small. The marginal cost
of a separate daemon — plus its IPC story — is large. If a future
deployment needs a headless graph server (CI, batch analysis), the
`shape graph` CLI is that server in `--serve` mode.

### Why the compiler must NEVER consult the graph

Restated for emphasis: see *Guard-rail* above. The forbidden-pattern
history is the primary reason; the architectural cleanliness is the
secondary reason; the operational simplicity (graph can be deleted
without breaking the build) is the tertiary reason. All three reinforce.

## Prior art

- **Glean (Meta).** Open-sourced 2021, retrospective December 2024
  ([engineering.fb.com](https://engineering.fb.com/2024/12/19/developer-tools/glean-open-source-code-indexing/),
  [HN discussion](https://news.ycombinator.com/item?id=42568516)).
  Angle query language ([glean.software](https://glean.software/docs/query/intro/)).
  Schema-first; fact-shaped; derived predicates. The most direct
  influence on this RFC. A recent Glean-on-Haskell case study
  ([simonmar.github.io 2025-05](https://simonmar.github.io/posts/2025-05-22-Glean-Haskell.html))
  validates the approach on a smaller-than-Meta codebase, which is the
  scale Shape will see.
- **Kythe (Google).** Earlier open-source code-indexing fact store.
  Influenced Glean's design. Has indexers for many languages but a
  steeper integration curve.
- **GitHub stack-graphs.**
  ([github.blog](https://github.blog/open-source/introducing-stack-graphs/),
  [arxiv:2211.01224](https://arxiv.org/pdf/2211.01224)). File-incremental,
  language-agnostic name resolution. Powers GitHub's Precise Code
  Navigation. Strongest prior art for incremental update. (No longer
  actively maintained at GitHub; design remains influential.)
- **Sourcegraph SCIP.** Standardized index format for code intelligence.
  Glean supports SCIP as an input format. Shape could emit SCIP from
  the graph in a future RFC.
- **rust-analyzer salsa.** On-demand query memoization with cycle
  detection. The invalidation pattern we adopt for the extractor is
  salsa-shaped, though our content-hash discipline removes the need for
  salsa's query-graph machinery.
- **LSIF.** Older alternative to SCIP. Less expressive but
  well-supported in older LSP clients.
- **Meta Aroma.** Internal completion-via-graph-queries tool. Direct
  validator of the "LLM queries the graph instead of grepping" use
  case at scale, though not open-source.

## Unresolved questions

- **Storage format stability.** Is `schema_version` per-DB sufficient,
  or do we need per-table versioning? Lean toward per-DB until we
  encounter a partial-migration need.
- **Cross-version graph migration.** When Shape ships a breaking
  predicate change (e.g., adding `metadata.scope` to `requires_permission`),
  do we migrate in-place or full-rebuild? Full-rebuild is simpler; the
  rebuild cost at workspace size is sub-second.
- **`shape-lsp` and `shape-mcp` sharing the same graph.** WAL mode
  supports concurrent readers, but the file path is workspace-relative.
  How does `shape-mcp` discover the workspace? Today it doesn't have a
  workspace concept. Proposed: `shape-mcp --workspace=<path>` flag, plus
  an MCP-config convention.
- **Gitability of the graph artifact.** Default is gitignore. But
  there's an argument for shipping a `graph.db` in source releases (a
  package on `shape-registry` could bundle its own graph for downstream
  cross-package queries). Defer to RFC-008 (registry-side graph)
  if that materializes.
- **`@intent` versus docstring.** Do we duplicate text, or do we
  promote the first paragraph of a docstring into `@intent` when no
  explicit `@intent` is given? Lean toward explicit-only — implicit
  promotion was a recurring source of `format()` name-shadowing class
  of bugs.
- **Tantivy version pinning.** The book-docs index pins one Tantivy
  version; the workspace-graph index uses another. Sharing the version
  is operationally simpler but couples release cadences. Defer.

## Future possibilities

- **Federated cross-package graph via `shape-registry`.** Each published
  package ships its own `graph.db`; the registry concatenates them on
  demand. `query_graph` gains a `--include-registry` flag. Enables
  *"which registry packages call into `std::core::http`?"* at registry
  query time. Out of scope for this RFC; a likely RFC-009.
- **LLM-driven graph queries as a high-level discovery surface.** With
  `query_graph` available, an MCP client can build a *"summarize this
  workspace"* tool on top: enumerate top-N traits by impl count, top-N
  intents by realizer count, capability footprint. The graph is the
  affordance; the summarizer is one prompt-engineered tool above it.
- **Integration with a future Shape-Loogle.** A Hoogle-shaped type-based
  search ("find me a function with signature `Array<int> -> int`") maps
  cleanly to the graph — `defines` joined with type-signature metadata.
  No RFC yet, but the predicate slot is reserved.
- **`shape graph diff <ref>`.** Pre-commit hook: *"this commit removes
  the `calls` edge from `Foo::bar` to `legacy_helper`; the only other
  caller of `legacy_helper` is also being removed."* Catches dead-code
  cleanup opportunities visibly.
- **`@law` integration.** Once RFC-002 lands, the `has_law` predicate
  unlocks *"every implementation of `Monoid` should declare the
  associativity law"* as a one-line linter rule. The graph is the
  enforcement surface.

## Phasing and cost

| Phase | Scope | Cost | Gate |
|---|---|---|---|
| **Phase 1** | 10 predicates listed above; `shape-graph` crate with SQLite store; extractor in `shape-vm` behind `graph-index` feature flag; LSP workspace-symbol enhancement; `shape graph {query,rebuild,status,explain}` CLI | 3-5 EM | All 10 predicates emitted; LSP workspace-symbol queries demonstrably faster + scope-aware on stdlib |
| **Phase 2** | MCP `query_graph` tool; Tantivy free-text over `@intent` + docstrings; LSP code-lens upgrade; LSP side-panel REPL | 3 EM | One LLM-author session demonstrably uses `query_graph` instead of grep; staleness counter wired |
| **Phase 3** | Salsa-style incremental updates wired to LSP file-save events; full Glean-parity for the predicate subset; reserved `has_law` / `refines` predicates enabled when RFC-002 / RFC-004 land; cross-version migration recipe | 6-9 EM | Edit-a-function-body produces graph update in under 100ms; predicates added without rebuilding existing facts |

Cost estimates are engineer-months, single engineer, assuming the
candidate predicates above are stable. The first item under Phase 1 —
the extractor — is the bulk of the work, because every predicate emitter
needs its own visitor over the compiler's IR. Most of the others are
plumbing.

Once Phase 1 is in place and one consumer (LSP workspace-symbol) is
demonstrably using it, the marginal cost of each subsequent consumer
(MCP, code-lens, REPL, future RFCs) is small — the substrate has paid
for itself.

---

**Decision sought from reviewers:** approval to proceed with Phase 1
once RFC-002 (`@law`) and RFC-004 (`#[replaces]`) have settled enough
that their reserved predicates can be specified concretely. The graph
design does not block either, but their predicates are the validation
that the schema generalizes.

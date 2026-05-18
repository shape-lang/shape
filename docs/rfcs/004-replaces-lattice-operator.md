# RFC-004: `#[replaces]` — Linker-Enforced Deprecation as a Minimum Lattice Operator

- **Status:** Draft
- **Type:** Feature (language + linker)
- **Dependencies:** Hard dependency on RFC-001 (graveyard + `attribute_directive` grammar)
- **Authors:** Shape language team
- **Date:** 2026-05-18

## Summary

Introduce a single attribute directive, `#[replaces(prior, migration_url = "...")]`,
that declares a new function semantically supersedes an older one. The Shape
**linker** (`crates/shape-vm/src/linker.rs`) refuses to load both blobs into the
same program, producing a hard link-time error that names the migration URL.
The replaced function is implicitly added to RFC-001's graveyard with
`reason="replaced by {new}"`, so any later attempt to reintroduce it must go
through `#[exhume]`.

This RFC deliberately ships **one** operator in the deprecation/refinement
lattice. The fuller cousin attributes — `#[refines]`, `#[generalizes]`,
`#[orthogonal]` — are out of scope and deferred to a later RFC, pending
Shape-Loogle-style search infrastructure. The Mathlib precedent and Rust's
2024H2 orphan-rule retreat both argue that lattice operators are only useful
once developers can *find* the canonical implementation; otherwise they
encode taxonomy nobody navigates.

## Motivation

Shape's compiler/runtime has a documented history of plans that delete a
dynamic-fallback path, then quietly re-introduce it under a more
respectable-sounding name. The **W-series** (W1–W4 + α/δ follow-ups,
9 commits) is the canonical case: `ValueWord` was scheduled for deletion in
`v2-nanbox-removal-plan.md` Step 6, became a "ValueBits shim retained as
documented FFI-boundary bridge" mid-execution, and accumulated 4–6 weeks of
follow-up cleanup. `CLAUDE.md` §Forbidden Patterns now enumerates the
deletion-fate names that must be refused on sight.

The W-series failure mode generalises: *a function gets replaced, the old
function does not get deleted, both keep working, the codebase grows two
parallel implementations, and the LLM-driven editor pulls the wrong one
the next time it touches the code*. The deprecation tooling Shape ships
today does not catch this:

1. `@deprecated` is a `DocTagKind::Deprecated` parsed at
   `crates/shape-ast/src/parser/docs.rs:149` and surfaced through
   `DocComment::deprecated_doc` at `crates/shape-ast/src/ast/docs.rs:83`. It
   is **pure documentation** — no compiler pass and no linker pass consumes
   it.
2. The only behavioural deprecation today is `check_deprecated_apis` at
   `crates/shape-runtime/src/engine/execution.rs:312-333`, which hard-codes
   string matches against two removed builtins (`csv.load` and bare `load(`).
   It is not a general mechanism; it is a regex over source text.
3. There is no `#[replaces]`/structural-deprecation infrastructure. This is
   greenfield work.

Shape's annotation + comptime substrate is also intended to support
LLM-driven authoring workflows in stdlib and userland. Agents authoring
Shape modify the program faster than any human reviewer, and they routinely
produce a "replacement" function while leaving the old one callable. Without
a mechanical lockout, the next agent uses the old function because it appears
first in the file, or appears in a stale embedding, or has more callers. By
the time a human notices, both functions have downstream call sites and the
"replacement" has bifurcated into a parallel code path.

The mechanism in this RFC is intentionally narrow: **one** function asserts
it replaces **another** function by name; the linker refuses to admit both
into the same program. That is enough to turn the W-series defection
attractor into a hard link-time error.

### Why *only* `#[replaces]` and not the full lattice

A complete subsumption lattice would also need `#[refines(prior)]` (stronger
spec, prior still useful), `#[generalizes(prior)]` (prior is a special case,
special case still useful), and `#[orthogonal(prior)]` (visibly similar but
solve different problems). Each is plausible; each is deferred. The full
argument lives in §Rationale; the short version: lattice operators are only
worth their syntactic cost once the toolchain has search infrastructure
strong enough to make them queryable.

## Guide-level explanation

### Declaring a replacement

```shape
@deprecated{ Use parse_user instead — see migration guide. }
fn old_parse_user(raw: string) -> User {
    // ...
}

#[replaces(old_parse_user, migration_url = "https://shape-lang.org/migrations/parse_user_v2")]
fn parse_user(raw: string) -> Result<User, Error> {
    // new fallible spec
}
```

`#[replaces]` says: *this function semantically replaces `old_parse_user`*.
Linking a program that contains both functions is a hard error.

### What the link-time error looks like

```text
error[L004]: replaced function still present in linked program
  --> myapp/users.shape:14:1
   |
14 | fn old_parse_user(raw: string) -> User {
   | ^^^^^^^^^^^^^^^^^ this function is marked replaced
   |
   = note: `parse_user` (declared in users.shape:23) carries
           `#[replaces(old_parse_user, ...)]`
   = note: migration: https://shape-lang.org/migrations/parse_user_v2
   = note: if you need the old function back, remove `#[replaces]` and
           re-introduce it with `#[exhume(reason = "...")]` (see RFC-001)
   = note: this check fires at link time, after content-addressed blobs
           are loaded; both blobs survive in the function store, only
           the program-level link rejects the pairing
```

The error always names the migration URL. The error always names the escape
hatch (`#[exhume]`). Both are mandatory because the goal is to make
"reintroduce the old function" a deliberate, auditable act, not an
accidental side effect of a sloppy revert.

### Migration workflow

1. Author writes new function with `#[replaces(old_name, migration_url = ...)]`.
2. `shape check` and CI fail at link time on the offending program.
3. Author either:
   - deletes `old_parse_user` (clean replacement; old function moves to
     RFC-001's graveyard automatically), or
   - leaves both in the source tree but ensures only one is ever pulled
     into a given program graph (rare, but valid for staged rollouts where
     two binaries link disjoint blob sets).
4. Migration URL stays in the error message indefinitely; even months later,
   a developer attempting to revive the old function hits the same message.

### Relationship to RFC-001's graveyard

RFC-001 introduces a *graveyard* — a registry of intentionally-removed
function identities, each carrying `reason`, `created_by`, and a
re-introduction gate. The first time a program is linked with a
`#[replaces(old)]` edge, the linker:

1. Records `old` in the graveyard with
   `reason = "replaced by {new}"`,
   `created_by = "#[replaces]"`,
   `migration_url = <attr arg>`.
2. Refuses to load the `old` blob now.
3. Refuses to load the `old` blob later — even after `#[replaces]` is
   removed — unless someone explicitly re-introduces `old` via
   `#[exhume(reason = "...")]`.

That last property is the whole point. "We changed our mind, the old
function should come back" is fine, but it must be auditable. Removing
`#[replaces]` is not enough; the author must say *why* the deletion is
being reversed. This is the same shape as ADR-006's surface-and-stop
discipline applied to API evolution rather than dispatch.

### Annotation vs. directive

`#[replaces]` is a **directive**, not an annotation. Annotations
(`@deprecated`, `@example`, user-defined `@retry`) participate in compile-
time evaluation and surface in documentation. Directives instruct the
compiler/linker/runtime to enforce a specific structural property and have
no user-visible runtime behaviour. RFC-001 introduces the
`attribute_directive` grammar production in `crates/shape-ast/src/shape.pest`
specifically to host this distinction; `Replaces` is one of `DirectiveKind`'s
reserved variants. This RFC has a hard dependency on RFC-001's grammar
landing first — it cannot be parsed without it.

## Reference-level explanation

### Linker hook

The enforcement point is one new check in `crates/shape-vm/src/linker.rs`,
adjacent to the existing transitive permission-union fold at
`linker.rs:329-331`:

```rust
// crates/shape-vm/src/linker.rs (post-RFC-004)

// Existing fold (lines 329-331, unchanged):
let total_required_permissions = blobs.iter().fold(PermissionSet::pure(), |acc, blob| {
    acc.union(&blob.required_permissions)
});

// New, structurally identical fold added by RFC-004:
let replaces_edges: Vec<ReplacesEdge> = blobs
    .iter()
    .flat_map(|blob| blob.replaces.iter().cloned())
    .collect();

// New rejection pass:
for edge in &replaces_edges {
    if let Some(replaced_id) = name_to_id.get(edge.prior_name.as_str()) {
        return Err(LinkError::ReplacedFunctionLinked {
            replaced: edge.prior_name.clone(),
            replacement: blobs[name_to_id[edge.new_name.as_str()]].name.clone(),
            migration_url: edge.migration_url.clone(),
        });
    }
}
```

Two design properties matter here:

1. **Structural twin of permission-union.** The existing
   `total_required_permissions` fold at `linker.rs:329-331` is the
   precedent for "walk every blob, accumulate one program-level fact". The
   `#[replaces]` walk has identical shape — different predicate, same
   carrier. There is no new data structure, no new traversal order, no new
   concurrency story. The parallel-link path at `linker.rs:343-414` and
   the sequential path at `linker.rs:509-548` both already iterate
   `blobs.iter()`; the replaces fold inserts into either site with the
   same cost profile.
2. **Hash-time discrimination is automatic.** Because `#[replaces]` baking
   appends to `FunctionBlobHashInput.required_permission_names` semantics
   (see "Interaction with `content_hash`" below), two source-identical
   functions with different `#[replaces]` edges produce different
   `content_hash` values. The linker cannot accidentally treat a
   `#[replaces]`-bearing blob as equivalent to a non-bearing blob.

The new `LinkError` variant slots into the existing enum at
`linker.rs:22-32`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("Missing function blob: {0}")]
    MissingBlob(FunctionHash),
    #[error("Circular dependency detected")]
    CircularDependency,
    #[error("Constant pool overflow: {0} constants exceeds u16 max")]
    ConstantPoolOverflow(usize),
    #[error("String pool overflow: {0} strings exceeds u32 max")]
    StringPoolOverflow(usize),
    // New:
    #[error("Function {replaced} was replaced by {replacement}; \
             see {migration_url}")]
    ReplacedFunctionLinked {
        replaced: String,
        replacement: String,
        migration_url: String,
    },
}
```

### Surfacing the error to host code

`load_program_with_permissions` at
`crates/shape-vm/src/executor/vm_impl/program.rs:344-361` already maps
`LinkError` into the typed host `PermissionError`. RFC-004 adds the sibling
variant:

```rust
pub enum PermissionError {
    LinkError(String),
    InsufficientPermissions { /* ... */ },
    // New:
    ReplacedFunction {
        replaced: String,
        replacement: String,
        migration_url: String,
    },
}
```

`load_program_with_permissions` pattern-matches on the new `LinkError`
variant and emits `PermissionError::ReplacedFunction` instead of the
opaque `LinkError(String)` carrier. The two existing
permission-checked load paths (`load_program_with_permissions` and
`load_linked_program_with_permissions`) get identical treatment.

### `FunctionBlob` extension

The `FunctionBlob` struct at
`crates/shape-vm/src/bytecode/content_addressed.rs:33` gains one field:

```rust
pub struct FunctionBlob {
    // ... existing fields ...

    /// RFC-004: outgoing #[replaces] edges declared by this function.
    /// Each edge names a prior function this blob asserts it replaces.
    #[serde(default)]
    pub replaces: Vec<ReplacesEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplacesEdge {
    pub prior_name: String,
    pub new_name: String,         // owner blob's own name; cached for diag
    pub migration_url: String,
}
```

### Interaction with `content_hash`

`#[replaces]` participates in the content hash exactly as
`required_permissions` does. `FunctionBlobHashInput` at
`content_addressed.rs:97` already takes
`required_permission_names: Vec<&'a str>` at line 113-114 as a
sorted-deterministic projection of `PermissionSet`. RFC-004 adds a parallel
projection:

```rust
#[derive(Serialize)]
struct FunctionBlobHashInput<'a> {
    // ... existing fields ...
    required_permission_names: Vec<&'a str>,
    // New: sorted, deterministic projection of replaces edges.
    replaces_edges: Vec<(&'a str, &'a str)>,  // (prior_name, migration_url)
}
```

This preserves the invariant that *two functions with identical code but
different declared replacement behaviour produce different content
hashes*. A blob that asserts `#[replaces(parse_user)]` is a different
blob, by content identity, from the one that does not — exactly as a blob
declaring `Permission::FsWrite` is different from the one that does not.

### Cross-crate transitive enforcement

Cross-crate `#[replaces]` enforcement reuses the linker's existing
transitive walk. The fold at `linker.rs:329-331` already proves the linker
visits *every* blob across every crate that flows through `Program`; it
does not stop at crate boundaries. The new replaces fold rides on the same
visit. There is no module-resolution change required at
`crates/shape-vm/src/module_resolution.rs:33`
(`build_graph_and_stdlib_names`) — prior-name resolution happens after the
module graph has assembled, against the linker's `name_to_id` map at
`linker.rs:313`.

The consequence: a package A that exports `parse_user` with
`#[replaces(old_parse_user)]` will fail to link in *any* downstream binary
that also transitively pulls in `old_parse_user` from package B, even if
A and B never co-import each other. The error surfaces at the consuming
binary, not at A or B alone. This is the desired property — it mirrors
how `PermissionSet` violations surface at the consuming binary.

### Directive parsing

The `#[replaces]` directive uses RFC-001's `attribute_directive` grammar
production. The recognition path lives in the sibling directive pipeline
at `crates/shape-vm/src/compiler/functions_annotations.rs`, alongside the
existing annotation pipeline (which handles `@annotation`, `@deprecated`,
etc.). `FunctionDef.directives: Vec<Directive>` (the parallel field RFC-001
adds to `FunctionDef.annotations` at
`crates/shape-ast/src/ast/functions.rs:29`) is consumed by a single match
arm:

```rust
for directive in &func_def.directives {
    match &directive.kind {
        DirectiveKind::Replaces { prior, migration_url } => {
            blob.replaces.push(ReplacesEdge {
                prior_name: prior.clone(),
                new_name: func_def.name.clone(),
                migration_url: migration_url.clone(),
            });
        }
        // other RFC-001-reserved directive variants ...
    }
}
```

### `migration_url` contract

`migration_url` is a required keyword argument. The compiler accepts any
string literal; it does not attempt to validate that the URL resolves. The
contract is *the URL is the canonical place a human or LLM goes to learn
why the function was replaced*. The compiler stores it verbatim in the
blob, the linker emits it verbatim in the error, the LSP renders it
verbatim in hover.

The compiler does enforce that the value is a string literal (not a
`comptime` expression). A `comptime`-computed migration URL would be a
load-bearing computation hidden inside an attribute; the principle of
least surprise says directive arguments are syntactically obvious.

## Drawbacks

1. **Cross-crate version conflicts surface earlier and louder.** Today,
   two crates importing different versions of `parse_user` resolve through
   semver pin or warning. Under RFC-004, the version that ships
   `#[replaces]` will refuse to link with the version that does not. This
   is a *feature* — it forces version alignment — but it is a feature
   measured in build-time pain, and is plausibly the most controversial
   property of this RFC. The cargo precedent here is `[replace]` and
   `[patch]` in `Cargo.toml`, which expose the same machinery to package
   authors at the dependency-graph level rather than the function level.
2. **Auto-graveyard adds one RFC-001 entry per `#[replaces]`.** A codebase
   that aggressively rewrites with `#[replaces]` will see graveyard growth
   linear in rewrites. The graveyard is intentionally append-only (RFC-001
   §audit), so this is intended cost, but it is real cost. The defence is
   that the graveyard is the audit log; deletions without audit are
   exactly the W-series failure mode.
3. **"I'll just rename and `#[replaces]`" cargo-cult risk.** Once
   `#[replaces]` exists, the path of least resistance for any rename
   becomes "add `#[replaces]` and call it a day". This trivialises the
   directive — `parse_user_v2` `#[replaces]` `parse_user_v1` is *not* a
   meaningful semantic statement, it is renaming. The mitigation is
   documentation (the official guide insists `#[replaces]` is for
   *semantic* replacement) and reviewer culture; the mechanism cannot
   distinguish renames from real replacements.
4. **The directive can collude with the graveyard to create a one-way
   ratchet.** A chain `f → g → h` of `#[replaces]` edges requires two
   `#[exhume]`s to revive `f`. Plausibly correct (the chain is the audit
   log), but it makes deep-history rollbacks structurally harder than
   shallow ones.

## Rationale and alternatives

### Why not the full lattice (`#[refines]`/`#[generalizes]`/`#[orthogonal]`)

The strongest argument for shipping the minimum operator is *evidence from
two communities that have lived with the alternative*.

**Mathlib.** Mathlib has spent years building exactly the lattice that
`#[refines]`/`#[generalizes]`/`#[orthogonal]` describe — `theorem`/`lemma`
relationships, `simp` priorities, `@[simp ←]` reverse rewrites, the
`refine?` tactic. The lattice is *valuable* in Mathlib because Mathlib has
strong search: Loogle (Haskell-Hoogle-style type-signature search),
LeanSearch (LLM-augmented natural language), Moogle (semantic search), and
the `exact?` tactic that proposes lemmas from the proof state. The Mathlib
community-blog guidance (June 2025) explicitly recommends *learning the
search tools first*, then layering lattice annotations on top. Without the
search substrate, the lattice is dead text. Shape does not have a
Shape-Loogle today. Shipping `#[refines]` without it would encode
relationships that the toolchain cannot help developers navigate.

**Rust orphan rules.** Rust's 2024H2 project goal proposed relaxing the
orphan rule so downstream crates could `impl` upstream traits on upstream
types. The plan retreated: the current state is a `-Z` nightly experiment
scoped to *binary crates only*, gated behind a lint, with no stabilisation
timeline. The retreat was driven by ecosystem-cost analysis — the
diesel-chrono problem (two crates both providing `impl Foo for Bar`,
incompatible, no canonical resolution) showed that lattice operators
without a *registry of canonical implementations* fragment the ecosystem
faster than they unify it. The same risk applies to Shape's lattice: a
package providing `#[refines]` without a canonical-implementation registry
creates a coordination problem that the type system cannot resolve.

The pattern is consistent: **lattice operators require lookup
infrastructure**. Ship the operator only after the infrastructure exists.
For Shape, the operator with the lowest infrastructure dependency is
`#[replaces]`, because it requires only the linker's existing
blob-traversal pass.

### Why linker enforcement and not type-system enforcement

Three reasons:

1. **The linker already has the traversal.** The transitive
   permission-union fold at `linker.rs:329-331` is the precedent. Adding
   a second fold in the same place costs ~10 lines of code and has the
   same complexity profile.
2. **Type-system enforcement would require subtyping.** A type-level
   "`g` replaces `f`" relationship implies a subtype lattice between
   function types, which Shape's type system explicitly does not have
   (per CLAUDE.md §Type System Rules: no runtime coercion, no `any`,
   `int` and `number` do not unify, types are fully determined at compile
   time). Adding a subtype lattice for replacement would be a much larger
   semantic change than this RFC justifies.
3. **The error message is better at link time.** A type-error
   "incompatible function types" is poor diagnostic surface area; a link
   error "you tried to load both `f` and `g` where `g` replaces `f` —
   see <migration_url>" is exactly what the developer needs. The linker
   has all the context (both blobs, both names, the directive); the type
   checker would need to fabricate or thread additional context.

### Why a directive and not an annotation

`@deprecated` is an annotation today and it does nothing the linker can
enforce. The point of introducing the directive distinction (which RFC-001
does) is to *separate things the compiler enforces from things it
documents*. `#[replaces]` is enforced. The compiler refuses programs that
violate it. Putting it under `#[...]` rather than `@...` signals this to
readers, to the LSP, and to RFC-001's directive registry.

### Why not just delete the old function

Clean delete works when the author controls the whole codebase. It fails
when (a) the function is exported from a library — downstream consumers
need the migration URL the link-time error provides; (b) the codebase
straddles a staged transition; or (c) the author wants the audit trail
(`#[replaces]` records *why* the old function went away; `git rm` does
not).

### Cargo `[replace]` / `[patch]` as prior art

Cargo's `[replace]` and `[patch]` sections in `Cargo.toml` let a workspace
override a transitive dependency with a local or fork version. The
mechanism is at the *crate* level, not the function level, and it is
opt-in by the consumer rather than asserted by the author. RFC-004's
`#[replaces]` is the author-asserted, function-level dual: the function
author says "this replaces that", and the consumer linker enforces the
assertion. The two mechanisms are complementary; nothing here precludes a
future Shape `[replace]` package-manifest stanza.

## Prior art

- **Rust RFC 1023 (re-rebalancing coherence).** Established the modern
  orphan rule. The RFC text walks through the same "what makes lattice
  changes hard?" reasoning Shape's `#[replaces]` defers to a later RFC.
  Worth reading in full for the cost-of-lattice arguments.
- **Rust RFC 2451 (re-rebalancing coherence, take two).** Tightened
  coherence; demonstrated that even small lattice changes have years-long
  ecosystem-feedback loops.
- **Rust 2024H2 orphan-rule relaxation goal** (Rust Project Goals,
  *Experiment with relaxing the Orphan Rule*) and the experimental
  nightly flag (rust-lang/rust#136979). The retreat from the ambitious
  RFC to a `-Z` flag scoped to binary crates is the strongest piece of
  evidence Shape can draw on for "ship the minimum operator first".
- **Mathlib search tactics** (Loogle, LeanSearch, Moogle, `exact?`).
  Concrete demonstration that lattice annotations require search
  infrastructure. The June 2025 community-blog post is the canonical
  reference.
- **Java `@Override`.** A precedent for "the compiler refuses a program
  unless the annotated relationship is verifiable". `#[replaces]`
  inherits the directive-checked-by-compiler shape; the difference is the
  enforcement happens at link rather than compile because the linker
  already owns cross-blob traversal.
- **Cargo `[replace]` and `[patch]`.** Crate-level dual of function-level
  `#[replaces]`. Useful precedent for "the package manager / linker is the
  natural enforcement point for cross-unit substitution".

## Unresolved questions

1. **Semver interaction.** Should `#[replaces]` be considered a major
   version bump under Shape's package-versioning model? Argument for: it
   breaks every consumer that imported the old name. Argument against: a
   *clean* delete is also a breaking change, and we do not require a major
   bump for that today. Recommended resolution: defer to the package
   manifest spec; `#[replaces]` is a directive on a function, not a
   package-level invariant.
2. **Migration URL enforcement / parsing.** Should the compiler validate
   the URL is well-formed? Should the build system fetch it to verify it
   resolves? Recommended resolution: no validation at compile time, no
   fetching at build time. The URL is documentation surface; treat it as
   an opaque string. A future linter could optionally fetch in CI mode.
3. **`#[replaces]` chains.** If `g` replaces `f` and `h` replaces `g`,
   does `h` transitively replace `f`? The proposed semantics: yes,
   because both `f` and `g` are in the graveyard, and any program that
   tries to link `f` or `g` fails. The graveyard entries chain
   (`f.reason = "replaced by g"`, `g.reason = "replaced by h"`); the
   error message for `f` should mention the *latest* replacement and the
   *original* migration URL. This needs validation in implementation.
4. **LSP integration.** The existing `@deprecated` doc-tag render at
   `tools/shape-lsp/src/doc_render.rs:120-135` (`tag_body` helper for
   `DocTagKind`) is the natural extension point. A `#[replaces]` directive
   should render as a hover badge ("REPLACED BY `parse_user` — see
   migration") on every reference to the replaced function. The
   implementation can land alongside RFC-001's directive-registry surface
   in the LSP.
5. **Snapshot interaction.** Shape's `snapshot()` captures full VM state.
   A snapshot taken before a `#[replaces]` edge is added, then resumed in
   a binary that has the edge, must not silently lose the call site.
   Recommended resolution: the snapshot already records `FunctionHash`
   per stack frame; the linker check fires before snapshot resumption,
   so an `old` blob that survived in a snapshot would simply fail the
   load step. This should be regression-tested.
6. **Trait-method replacement.** Can `#[replaces]` target a trait method?
   The directive is currently scoped to free functions in this RFC; trait
   methods (per `crates/shape-vm/src/bytecode/content_addressed.rs`
   `trait_method_symbols`) are a separate naming surface. Recommended
   resolution: explicitly out of scope; trait-method replacement is a
   downstream RFC.

## Future possibilities

- **Full subsumption lattice.** Once Shape-Loogle exists, the
  `#[refines]` / `#[generalizes]` / `#[orthogonal]` operators can land
  as a single follow-up RFC. The Mathlib precedent suggests the search
  substrate must come *first* — Loogle predated Mathlib's heaviest use of
  the lattice by years, not the other way around.
- **Shape-Loogle prerequisite.** A type-signature search engine over the
  package registry, indexed by `FunctionBlob.type_schemas` and
  `FunctionBlob.required_permissions`, exposed through the LSP and the
  REPL. This is a separate workstream and is the gating dependency on
  the full lattice.
- **Cross-package canonical-implementation registry.** A package-registry
  feature (`shape-registry`) that flags when two published packages
  declare `#[replaces]` against the same prior function from different
  perspectives. This would resolve the diesel-chrono failure mode by
  making conflicting `#[replaces]` claims visible at publish time, not
  link time.
- **Time-bounded `#[replaces]`.** `#[replaces(old, until = "2027-01")]`
  to schedule automatic upgrade of `#[replaces]` to a deletion. The
  graveyard already records the date; a registry-side cron could
  surface the deadline.
- **`#[replaces]` with predicate.** A future
  `#[replaces(old, if = comptime { ... })]` to scope replacement to a
  subset of call sites (e.g., "replaces `parse_user` only when the
  receiver is `User_v2`"). This is more interesting than it sounds but
  it is firmly future work; the comptime evaluation surface would need
  to extend to directive arguments, which RFC-001 currently forbids.

## Phasing and cost

Estimated total cost: **3 engineer-weeks**, sequenced as three
near-independent landings:

| Phase | Work | Estimated cost |
|------:|------|---------------:|
| 1 | Linker enforcement: extend `LinkError` (`linker.rs:22-32`), add the replaces-fold check adjacent to `linker.rs:329-331` in both sequential and parallel paths, extend `FunctionBlob` (`content_addressed.rs:33`) with `replaces: Vec<ReplacesEdge>`, extend `FunctionBlobHashInput` (`content_addressed.rs:97`) with the sorted projection, wire the host-error variant into `load_program_with_permissions` (`program.rs:344-361`). | 1 week |
| 2 | Directive parsing: recognise `DirectiveKind::Replaces` in `compiler/functions_annotations.rs`; populate `FunctionBlob.replaces` from `FunctionDef.directives`. Depends on RFC-001's `attribute_directive` grammar landing in `shape.pest` and `FunctionDef.directives` landing in `crates/shape-ast/src/ast/functions.rs:29`. | 0.5 week |
| 3 | Diagnostic surface: error rendering, LSP hover integration via `doc_render.rs:120-135`, graveyard wiring with RFC-001's registry, regression tests for single-blob, cross-crate, snapshot-resumption, and chains. | 1.5 weeks |

**Gating dependency.** This RFC is fully gated on RFC-001 landing at the
"behavior_hash baked + directive grammar" stage. Without RFC-001's
`attribute_directive` grammar production, `#[replaces]` cannot be parsed;
without RFC-001's graveyard registry, the auto-graveyard step in Phase 3
has nowhere to write. Phase 1 (the linker enforcement) is technically
landable ahead of RFC-001 with a temporary parser hook, but doing so
would create a directive surface that nothing else uses, and would
encourage the cargo-cult risk identified in §Drawbacks before the
docs/diagnostics are in place. The sequenced ordering — RFC-001 first,
RFC-004 second — is strongly preferred.

**Sentinel test.** A unit test in `crates/shape-vm/src/linker_tests.rs`
should construct a two-blob `Program` where blob B carries
`#[replaces(A)]`, assert `link()` returns
`LinkError::ReplacedFunctionLinked { replaced: "A", .. }`, and assert that
removing the directive lets the link succeed. This guards the W-series
class of regressions where a "small renaming" silently disables the
mechanism.

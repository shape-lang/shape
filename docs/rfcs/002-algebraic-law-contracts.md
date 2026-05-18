# RFC-002: Algebraic-Law Contracts (`@law`)

- **Status:** Draft
- **Author:** Shape language team
- **Created:** 2026-05-18
- **Companion:** RFC-001 (distinctness directives, `#[distinct_from]`)
- **Tracking:** post-v0.3, targets v0.4 surface

---

## Summary

Introduce a closed enumeration of algebraic-law annotations attached to
function definitions:

```shape
@law(idempotent, monoid(zero = "", op = concat))
fn normalize_path(p: string) -> string { ... }

@law(commutative, associative, identity = 0)
fn merge_balances(a: Money, b: Money) -> Money { ... }
```

The compiler property-tests each declared law in a comptime sandbox, fails the
build when a counterexample is found (with proptest-shrunk minimal witness),
and bakes the *verified* law set into the function's content hash alongside
`required_permissions`. Verified law names become first-class queryable
metadata via `laws::query` (Shape REPL/LSDS) and a new MCP tool surface
(`query_shape_laws`, `distinguish_shape_functions`). There is no escape
hatch: a function either verifies its declared laws at the configured tier
or fails to compile.

`@law` keeps the `@`-annotation sigil because it *describes* an externally
verifiable property of the function. Compare RFC-001's `#[distinct_from]`,
which is *directive*-shaped: it instructs the compiler to suppress its
overload-distinctness refusal. The split is the spine of the project's sigil
policy — `@` for verified/queryable properties, `#[...]` for closed-set
compiler directives validated at parse.

## Motivation

The default failure mode for LLM-authored functions is *silent invariant
break*. The LLM emits code that type-checks, passes the spot-test the user
demoed, and quietly violates an algebraic property the caller assumed —
`merge_balances` that isn't commutative, `normalize_path` that isn't
idempotent, a `dedupe` that isn't a monoid. Shape's existing capability
discipline catches *I/O* lies (a function tagged as pure but calling
`std::core::http::get` will not link). It catches nothing in the algebraic
plane. This is the largest semantic-correctness gap in the current
specification.

The framing inversion is the AI-native angle. The 1970s–2010s critique of
contract systems (Eiffel, Spec#, Code Contracts, Idris1 `Verified*`) is that
**humans don't read the law tags**. Verified-typeclass hierarchies in Idris1
were so unergonomic that the entire `contrib/Interfaces/Verified*` module set
was deprecated and split out of the standard library
([Idris2 CHANGELOG](https://github.com/idris-lang/Idris2/blob/main/CHANGELOG.md);
[idris 1.3.0 changelog](https://hackage.haskell.org/package/idris-1.3.0/changelog)).
Code Contracts hit the same wall in .NET and was dropped at the .NET 5
transition ([dotnet/docs#17640](https://github.com/dotnet/docs/issues/17640);
[microsoft/CodeContracts#409](https://github.com/microsoft/CodeContracts/issues/409)).
Spec# never escaped Microsoft Research. Eiffel's Design-by-Contract remains
respected in retrospectives but never crossed the chasm
([Wikipedia](https://en.wikipedia.org/wiki/Eiffel_(programming_language));
[Joab Jackson, 2003](https://www.joabj.com/Writing/Tech/Dev/0308-Eiffel-Design_By_Contract.html)).

The constant across those failures is the *consumer*. Each system assumed a
human would author the law, a human would read it, and a human would feel
its absence. None of those is reliably true. Two things are different now:

1. **LLMs author functions.** Author cost is no longer a constraint on
   per-function annotation count — the model can emit `@law(commutative,
   associative, identity = 0)` on `merge_balances` at zero marginal cost
   beyond a tokenizer hop.
2. **LLMs read functions.** A coding agent picking `merge_balances` out of a
   package registry can synthesize a property like "I need an idempotent
   monoid over `Path`" and ask the language for the bucket of candidates —
   instead of skimming README prose. This is `laws::query`.

Verified law metadata is high-value to a consumer that mechanically queries
it and zero-cost to an author that mechanically emits it. The historical
failure mode does not apply.

A second motivation is hash discipline. Shape already bakes
`required_permissions` into `FunctionBlob.content_hash` (compute path:
`crates/shape-vm/src/bytecode/content_addressed.rs:122-153`). Two functions
with identical instructions but differing permissions produce different
hashes — the registry treats them as distinct artifacts. Algebraic laws
deserve identical treatment: a function that *claims* to be commutative is
not interchangeable with one that does not, and the content-hash plane is
the right place to enforce that.

## Guide-level explanation

### Authoring

```shape
@law(commutative, associative, identity = 0)
fn add(a: int, b: int) -> int { a + b }

@law(idempotent, monoid(zero = "", op = concat))
fn normalize_path(p: string) -> string {
    p.split("/").filter(|s| !s.is_empty()).join("/")
}

@law(pure, total, monotone_in(0))
fn clamp_lower(x: int, floor: int) -> int {
    if x < floor { floor } else { x }
}
```

`@law` takes a parenthesized list of law atoms. Each atom is one of a
**closed** set (v1):

| Atom                          | Arity | Meaning                                                                         |
| ----------------------------- | ----- | ------------------------------------------------------------------------------- |
| `pure`                        | 0     | No side effects (capability-derived).                                           |
| `idempotent`                  | 0     | `f(f(x)) == f(x)`. Unary only.                                                  |
| `commutative`                 | 0     | `f(a, b) == f(b, a)`. Binary only.                                              |
| `associative`                 | 0     | `f(f(a, b), c) == f(a, f(b, c))`. Binary only.                                  |
| `monotone_in(i)`              | 1     | `a <= b ⇒ f(..., a, ...) <= f(..., b, ...)` at param index `i`.                 |
| `injective`                   | 0     | `f(a) == f(b) ⇒ a == b`. Unary.                                                 |
| `total`                       | 0     | Never returns `Err`, never panics, never `Option::None`.                        |
| `identity(e)`                 | 1     | `f(a, e) == a ∧ f(e, a) == a`.                                                  |
| `monoid(zero = ..., op = id)` | 2     | Associativity + identity composite. `op` names a binary fn over the same type.  |
| `inverse(of = id)`            | 1     | `f(g(x)) == x ∧ g(f(x)) == x` where `g = of`.                                   |

`homomorphism` is **excluded from v1.** It requires two function
references plus a structural law over both — v2/v3 territory.

### Failure mode

Counterexample failure surfaces at compile time as a Shape diagnostic with
the proptest-shrunk minimal witness:

```
error[E2001]: declared law violated
  --> src/balances.shape:7:1
   |
 7 | @law(commutative, associative, identity = 0)
 8 | fn merge_balances(a: Money, b: Money) -> Money {
 9 |     Money { cents: a.cents + b.cents, currency: a.currency }
10 | }
   |
   = note: law `commutative` failed at iteration 23 (shrunk in 11 steps)
   = note: witness a = Money { cents: 0, currency: "USD" }
   = note: witness b = Money { cents: 0, currency: "EUR" }
   = note: f(a, b) = Money { cents: 0, currency: "USD" }
   = note: f(b, a) = Money { cents: 0, currency: "EUR" }
   = help: either remove `commutative` from the @law set or fix the
     implementation; currency assignment is asymmetric.
```

There is no `#[allow]`, no `@law(declared_only)`, no `assume` keyword. Drop
the claim or fix the code.

### Discovery (`laws::query`)

Querying returns a **bucket** — the law signature is by design lossy. `sum`
and `xor` are both pure / commutative / associative / `identity(0)` over
`int`; their law signatures are identical. The query API is honest about
this:

```shape
let candidates = laws::query(
    domain = (int, int) -> int,
    laws = [pure, commutative, associative, identity(0)],
);
// candidates: Bucket<FunctionRef> — may contain sum, xor, max-zero-floor, ...
```

To disambiguate, `laws::distinguish` asks for distinguishing witnesses
(either supplied or auto-synthesized from the cross-product of seeded edge
cases):

```shape
let unique = laws::distinguish(
    candidates,
    witnesses = auto,   // or witnesses = [(1, 2, 3), (-1, 1, 0)] for explicit
);
// unique: HashMap<FunctionRef, Map<Witness, Output>>
```

REPL hover, LSP semantic hover, and the package-registry browser all surface
the verified law set on every function. MCP exposes two new tools
(`query_shape_laws`, `distinguish_shape_functions`) over the same machinery
— LLM agents query laws programmatically without recompiling user code.

## Reference-level explanation

### Surface syntax & parsing

`@law` parses as the existing `Annotation` AST node
(`crates/shape-ast/src/ast/functions.rs:202`):

```rust
pub struct Annotation {
    pub name: String,           // = "law"
    pub args: Vec<Expr>,        // each atom is an Expr
    pub span: Span,
}
```

`FunctionDef.annotations: Vec<Annotation>` already collects these
(`functions.rs:29`). A new post-parse pass in `shape-runtime` walks
`annotations`, recognizes `name == "law"`, and lowers each `Expr` arg into a
typed `LawAtom`:

```rust
pub enum LawAtom {
    Pure,
    Idempotent,
    Commutative,
    Associative,
    MonotoneIn(u8),            // param index
    Injective,
    Total,
    Identity(ConstExpr),       // const-evaluable identity element
    Monoid { zero: ConstExpr, op: FunctionRef },
    Inverse { of: FunctionRef },
}
```

Arity / type / param-index validation runs at this lowering step against
the function's declared signature. `commutative` on a unary function is a
compile error before any property test runs.

The argument expression to `@law` is general (`Vec<Expr>`), unlike the
string-only `FieldAnnotation.args: Vec<String>` for type fields
(`crates/shape-runtime/src/type_schema/field_types.rs:205-208`). This is
intentional — `monoid(zero = "", op = concat)` needs an identifier
(function reference) and a literal, not two strings; the existing `@alias`
field annotation (`field_types.rs:240`) is the precedent for the simpler
string-arg shape.

### `pure` derivation (static path)

`pure` is the only law derivable by static analysis. The compiler reuses
the existing capability machinery:

1. Walk the function's call graph (already done for
   `required_permissions`).
2. For every callee, look up
   `crates/shape-runtime/src/stdlib/capability_tags.rs:14`
   `required_permissions(module, function)`.
3. If the transitive union is `PermissionSet::pure()` *and* the function
   has no closure-captured mutable state *and* no `extern C` calls without
   `@law(pure)` themselves, the function is statically `pure`.

If `@law(pure)` is declared but the analysis says otherwise, the compiler
errors at the same point that capability lying errors today. No property
test runs for `pure`.

### Property-test path (all other laws)

The remaining nine atoms are verified by property testing in the existing
comptime sandbox:

1. The compiler emits a synthesized **law harness** comptime block per
   function-with-`@law`, containing one test per atom.
2. The harness is compiled like any other comptime block and executed via
   `crates/shape-vm/src/compiler/comptime.rs:300 execute_comptime`.
3. Each test composes (a) the always-seeded edge-case witness vector
   (`i64::MIN`, `i64::MAX`, `0`, `1`, `-1`, `""`, single-char, single-elem,
   empty, two-elem) with (b) a proptest-driven random vector, runs the
   property, and on failure invokes proptest's integrated shrinker to find
   a minimal counterexample. Integrated shrinking is the deciding feature
   over QuickCheck-style stateless shrinking — generators and shrinkers
   stay synced for composed types (`Money`, nested arrays, etc.)
   ([proptest book — Proptest vs Quickcheck](https://proptest-rs.github.io/proptest/proptest/vs-quickcheck.html)).
4. The harness runs under a per-tier `ResourceLimits`
   (`crates/shape-vm/src/resource_limits.rs:11`):
    - **Tier-1 (scalar / always-on, `just test-fast`):**
      `max_wall_time = 50ms`, `max_instructions = 5_000_000`,
      `max_memory_bytes = 32 MB`, **1000 iterations** per law atom.
      Targets scalar laws over `int`, `number`, `bool`, fixed-size enums.
    - **Tier-2 (collection / `just test`):**
      `max_wall_time = 500ms`, `max_instructions = 50_000_000`,
      `max_memory_bytes = 256 MB`, **200 iterations**.
      Targets laws whose witness type includes `Array<T>`, `HashMap<K,V>`,
      or nested TypedObjects.
5. Tier selection is automatic from the resolved witness type, not
   user-declared. The user does not "pick a tier"; the tier is a property
   of the function being tested.

### Witness generation from TypedObject schemas

Every Shape type has a `FieldDef` schema with `@range`, `@example`, and
`@description` annotations
(`crates/shape-runtime/src/type_schema/field_types.rs:211-247`).
The witness generator dispatches on `FieldType`:

| `FieldType`    | Seeded edge cases                        | Random source                            |
| -------------- | ---------------------------------------- | ---------------------------------------- |
| `I64` / `Int`  | `i64::MIN`, `i64::MAX`, `0`, `1`, `-1`   | `proptest::num::i64::ANY`                |
| `F64`/`Number` | `0.0`, `1.0`, `-1.0`, `f64::EPSILON`, `f64::MIN_POSITIVE`, `f64::MAX` | `proptest::num::f64::POSITIVE | NEGATIVE | ZERO` (NaN excluded — see Unresolved) |
| `Bool`         | `true`, `false`                          | exhaustive                               |
| `String`       | `""`, `" "`, single-char, single-grapheme cluster (emoji), `\0`-bearing | `proptest::string::string_regex(".*")`   |
| TypedObject    | one-per-field-`@example` Cartesian product (bounded) | recursive descent into field types |
| `Array<T>`     | `[]`, `[t0]`, `[t0, t0]`, `[t0, t1]`     | `prop::collection::vec(...)`             |

`@range(min, max)` narrows the random source bounds for that field.
`@example(v)` adds `v` to the seeded vector. The contract is: the seeded
vector is checked **before** the random sweep on every run, so a
regression on a known edge case fails immediately and is not lost to RNG
luck.

### Content-hash extension

`FunctionBlob` gains four parallel fields, mirroring the existing
`required_permission_names: Vec<&str>` sorted-name pattern at
`crates/shape-vm/src/bytecode/content_addressed.rs:113`:

```rust
pub struct FunctionBlob {
    // existing fields ...
    pub required_permissions: PermissionSet,

    /// Sorted, deterministic list of law atom names that were verified
    /// in this build at the recorded tier and iteration count.
    pub verified_law_names: Vec<&'static str>,

    /// Parameterized atoms in `(atom_name, encoded_param)` form,
    /// sorted by atom_name. `identity(0)` → `("identity", "0")`,
    /// `monoid(zero = "", op = concat)` → `("monoid", "zero=\"\",op=concat")`.
    pub verified_law_params: Vec<(&'static str, String)>,

    /// Verification tier (1 = scalar always-on, 2 = collection).
    pub verification_tier: u8,

    /// Iteration count used at the recorded tier. Lowering this from
    /// 1000 to 500 is a registry-wide rehash — see "Drawbacks".
    pub verification_iteration_count: u32,
}
```

`FunctionBlobHashInput` is extended in lockstep so all four fields
participate in the SHA-256 content hash (`content_addressed.rs:97-117`).

**Iteration count is deliberately in the hash.** A function verified at
1000 iterations is a different artifact from the same instructions
verified at 500. Lowering the tier-1 default from 1000 to 500 is a
registry-wide rehash event of identical character to introducing a new
`Permission` variant — both shift every function's content hash. This is
the design's flagship discipline pressure, not an accident. It forces the
language team to treat verification budget as a versioned commitment.

### Bucket discovery and disambiguation

`laws::query` is implemented as a comptime stdlib function over the linker's
function-blob table:

```
fn query(
    domain: TypeSignature,
    laws: Array<LawAtom>,
) -> Bucket<FunctionRef>
```

Filtering is a linear scan over the blob table comparing
`verified_law_names` set-superset and `verified_law_params` exact match,
intersected with signature compatibility. The "bucket" return is
intentional: signature alone never identifies a function — `sum` and `xor`
are formally indistinguishable at this layer.

`laws::distinguish` materializes a witness matrix:

```
fn distinguish(
    bucket: Bucket<FunctionRef>,
    witnesses: WitnessSpec,
) -> HashMap<FunctionRef, Map<Witness, Output>>
```

Algorithm with `witnesses = auto`:

1. For each function in the bucket, collect the union of its parameter
   types' seeded edge-case vectors.
2. Take the Cartesian product, bounded by `min(64, 2^arity * 8)` to keep
   the matrix readable.
3. Evaluate every function on every witness; group by output equivalence
   class.
4. Return the (function, witness, output) matrix; the caller renders or
   filters as appropriate.

`witnesses = explicit(...)` skips the Cartesian step and uses the supplied
list verbatim.

### MCP tool spec

Two additions to `shape-mcp/src/tools.rs:62 tool_definitions()`:

```jsonc
{
  "name": "query_shape_laws",
  "description":
    "Query the Shape package registry for functions matching a domain \
     and set of verified algebraic laws. Returns a bucket (never \
     unique by signature — sum and xor have identical law signatures).",
  "inputSchema": {
    "type": "object",
    "properties": {
      "domain": { "type": "string", "description": "Function signature, e.g. '(int, int) -> int'" },
      "laws": { "type": "array", "items": { "type": "string" },
                "description": "Law atom names, e.g. ['pure','commutative','associative']" },
      "law_params": { "type": "object",
                "description": "Atom params, e.g. { 'identity': '0' }" }
    },
    "required": ["domain", "laws"]
  }
},
{
  "name": "distinguish_shape_functions",
  "description":
    "Given a bucket of candidate functions, return a witness matrix \
     mapping each candidate to its output on a set of distinguishing \
     inputs. Auto-synthesizes witnesses from seeded edge cases by default.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "candidates": { "type": "array", "items": { "type": "string" } },
      "witnesses": {
        "oneOf": [
          { "const": "auto" },
          { "type": "array", "items": { "type": "array" } }
        ]
      }
    },
    "required": ["candidates"]
  }
}
```

### Mechanical enforcement (sentinel test)

Following the precedent of
`crates/shape-vm/src/executor/tests/no_dynamic.rs` (cited in CLAUDE.md
§Mechanical enforcement), a new sentinel test asserts forbidden law-escape
shapes are absent: `crates/shape-vm/src/executor/tests/no_law_escape.rs`.
The greps are exact strings: `"@law(declared_only)"`,
`"law_assume_proof"`, `"verified_laws_skip"`, `"law_escape_hatch"`,
`"#[law_unchecked]"`. Pre-commit `just check-no-dynamic` extends to
include these grep targets. Failure exits non-zero. CLAUDE.md §Forbidden
patterns gains a sub-section "Law-escape forbidden phrases" before
landing.

## Drawbacks

**Compile-time cost.** Every function with `@law` runs a property-test
harness per atom in the comptime sandbox. Tier-1 budget (~50ms per atom,
~10ms typical) keeps `just test-fast` workable; tier-2 (200 iterations,
500ms budget) inflates `just test`. A function with five laws and a
collection witness type costs ~2.5s. On a project with 100 lawed functions
this is 4 minutes added to `just test`. Acceptable; it is the cost of
verification, and the canonical workflow already separates `test-fast`
(tier-1 only) from `test` (tier-1 + tier-2).

**Sum/xor collision.** The law signature `(int, int) -> int + pure +
commutative + associative + identity(0)` matches both `sum` and `xor`
(and `min(a, b) where 0 is the identity` and others). This is *not* a
bug — it is the bucket discipline. Documented as the canonical example
in the guide. `distinguish` is mandatory before substitution.

**Iteration count in content hash.** Lowering tier-1 from 1000 to 500
rehashes the entire registry. This is deliberate, but it is a registry
event with operational cost (re-fetch on existing installs). The first
rehash will be painful and is comparable to adding a new `Permission`
variant. Mitigation: tier defaults are versioned in
`docs/runtime-v2-spec.md`-style style under their own ADR, and only
change at major Shape versions.

**Shrinker integration risk.** Proptest's integrated shrinker depends on
the strategy graph kept around through the test
([proptest book](https://proptest-rs.github.io/proptest/proptest/vs-quickcheck.html)).
Vendoring proptest into the Shape comptime sandbox means surfacing its
strategy DSL inside the sandbox. The simpler path — a stateless
quickcheck-style shrinker hand-rolled to live inside the comptime VM —
loses the composed-type shrinking that makes diagnostics actionable for
TypedObject witnesses. Risk: proptest depends on `std::thread` for
worker pools and on RNG state that must be made deterministic per content
hash. Plan: vendor proptest's deterministic-RNG core (`TestRng`) only,
strip the worker-pool integration, run iterations sequentially in the
comptime VM.

**Recursive-enum witness explosion.** A recursive enum like `enum Tree {
Leaf(int), Node(Tree, Tree) }` has unbounded depth. The witness generator
bounds recursive descent at depth 5 by default; this is a heuristic and
some bugs at depth 6+ will slip past. See Unresolved.

## Rationale and alternatives

**Why property testing instead of SMT for v1?** SMT can prove
`a + b == b + a` over fixed-bitwidth `int` in microseconds; it cannot prove
`normalize_path(normalize_path(p)) == normalize_path(p)` over `string`
without a string theory whose decision procedure is expensive and
incomplete. The property tester is uniform across all law atoms and all
witness types. v1 is the property tester; v3 considers a narrow SMT
discharge for arithmetic laws on bounded scalar types (see Future
possibilities).

**Why no `@law(declared_only)` escape hatch.** The Idris1 `Verified*`
deprecation is the prior-art warning. Idris1 had `VerifiedSemigroup`,
`VerifiedMonoid`, `VerifiedFunctor`, etc. — each a typeclass with proof
obligations a developer had to supply by hand. The result was a hierarchy
nobody used in practice; `contrib/Interfaces/Verified*` was deprecated and
removed from the prelude ([Idris2 CHANGELOG](https://github.com/idris-lang/Idris2/blob/main/CHANGELOG.md)).
The Liquid Haskell `assume` keyword tells a similar story: it shipped as an
escape valve, accreted across the ecosystem as an unproven axiom, and now
its uses are tracked as soundness liabilities
([Tweag, Assumptions for Liquid Haskell in the large](https://www.tweag.io/blog/2023-06-22-lh-assumption-imports/);
[Vazou et al., Refinement Types For Haskell](https://goto.ucsd.edu/~nvazou/refinement_types_for_haskell.pdf)).

This is **the same shape as the W-series defections** named in CLAUDE.md
§Forbidden rationalizations — "Just one decode at the boundary," "Soft-fail
counter for now, harden later," "Document it as out-of-scope." Each
phrase converts a one-time deletion into permanent maintenance debt; each
walked back during execution. A `@law(declared_only)` escape would walk
back the same way: shipped as a transitional concession, accreted, never
removed. We do not have it. If the law cannot be verified at the
configured tier, the function does not compile. If the configured tier
itself is insufficient, the language team raises the tier — the cost is
borne in the right place.

**Why a closed enum, not user-defined laws.** User-defined laws were
considered and rejected for v1. Three reasons:

1. Witness generation for arbitrary user predicates demands a meta-API
   over `FieldType` that exceeds the v1 scope.
2. The bucket-query and MCP tool spec depend on closed atom names; user
   laws would fragment the bucket space into one-bucket-per-user.
3. The closed enum keeps the failure-mode taxonomy small enough that LLM
   coding agents reliably *generate* the right law tag from a docstring.
   Open-set laws lose this property.

v3 considers a `@law trait` mechanism for user-defined laws that namespace
into the bucket space.

**Why ten atoms, not three or thirty.** The ten chosen are the laws an
LLM coding agent is likely to need for substitution: monoid composition
(merge, reduce, fold), idempotence (normalize, dedupe, canonicalize),
monotonicity (sort comparators), injectivity (hash, id-mapping),
totality (no panic, no Err), and `pure` as the capability anchor. The
classical algebraic hierarchy
([mathlib4 Algebra/Group/Defs](https://github.com/leanprover-community/mathlib4/blob/master/Mathlib/Algebra/Group/Defs.lean))
is far richer (group, abelian group, ring, integral domain, field, ...).
v1 is deliberately not that hierarchy — those structures require closed
operations over a carrier *set*, not properties of a single function, and
fit a future `@algebra` annotation on type definitions, not `@law` on
functions.

## Prior art

**[quickcheck-classes](https://hackage.haskell.org/package/quickcheck-classes)
(Andrew Martin, chessai).** Provides QuickCheck properties for common
Haskell typeclasses (`Monoid`, `Functor`, `Foldable`, ...). Closest direct
ancestor: same idea of "law sets attached to a function/typeclass, verified
by property testing." Differs in three respects: (1) Haskell typeclasses
are the unit of verification, not individual functions; (2) no
content-addressed artifact integration; (3) author runs the laws manually
in test, no compile-time enforcement.

**Idris1 `Verified*` interfaces.** The cautionary tale. Deprecated /
removed in the transition to Idris2 because the manual proof obligations
made the interfaces unergonomic at scale
([Idris 1.3.0 changelog](https://hackage.haskell.org/package/idris-1.3.0/changelog);
[Idris2 CHANGELOG](https://github.com/idris-lang/Idris2/blob/main/CHANGELOG.md)).
Property testing instead of proof and LLM-as-consumer instead of
human-as-consumer are the two pivots that change the calculus.

**Mathlib4 algebraic hierarchy + `fast_instance` macro.** Mathlib4
([leanprover-community/mathlib4](https://github.com/leanprover-community/mathlib4),
[Algebra/Group/Defs.lean](https://github.com/leanprover-community/mathlib4/blob/master/Mathlib/Algebra/Group/Defs.lean))
is the canonical reference for *complete* algebraic hierarchies — Shape's
v1 deliberately picks a small subset. The `fast_instance` macro pattern
(synthesizing typeclass instances from existing ones) is the inspiration
for v3's user-defined law trait mechanism: derive a `@law(commutative,
associative)` claim on `g = f` for free when `g` is a wrapper of `f`.

**Spec# (Microsoft Research) and .NET Code Contracts.** Both shipped as
contract systems with runtime + static verification and both lost
momentum. Code Contracts has no .NET 5+ support
([dotnet/docs#17640](https://github.com/dotnet/docs/issues/17640);
[microsoft/CodeContracts#409](https://github.com/Microsoft/CodeContracts/issues/409)),
nullable reference types are the replacement. Spec# never escaped
Research. Diagnosis: human-author + human-consumer model, plus a static
verifier (Boogie) that was a separate tool with separate dev experience.
Shape sidesteps both: comptime sandbox is the same sandbox the rest of
the language runs in.

**Eiffel Design by Contract.** The seminal system. Adoption remained
narrow despite intellectual influence — the modern reading
([Wikipedia](https://en.wikipedia.org/wiki/Eiffel_(programming_language));
[Joab Jackson, 2003](https://www.joabj.com/Writing/Tech/Dev/0308-Eiffel-Design_By_Contract.html))
points to ecosystem-momentum failure and human-author cost as the binding
constraints. Both relaxed in the LLM-author regime.

**Liquid Haskell `assume`.** The escape-hatch trap, explicit. `assume`
ships as a pragmatic concession, accretes as an unproven axiom load
across the ecosystem, and now its uses are tracked as soundness
liabilities ([Tweag](https://www.tweag.io/blog/2023-06-22-lh-assumption-imports/)).
Documented prior-art warning for "no escape hatch."

**Proptest integrated shrinking.** Proptest's strategy-based shrinker
keeps the generation graph alive through the test and shrinks across
composed types coherently, distinguishing it from QuickCheck's stateless
shrinker
([proptest book — vs QuickCheck](https://proptest-rs.github.io/proptest/proptest/vs-quickcheck.html);
[BurntSushi/quickcheck](https://github.com/BurntSushi/quickcheck)). This is
the deciding feature for actionable counterexamples on TypedObject and
nested-array witness types and the reason proptest is vendored rather than
hand-rolling a shrinker.

## Unresolved questions

1. **HashMap witness generation.** `HashMap<K, V>` witness construction
   needs a deterministic iteration order for reproducible counterexamples.
   Strawman: generate as `Vec<(K, V)>`, deduplicate by `K`, insert into a
   `HashMap` with a stable seed. Open: does dedup-by-`K` adequately cover
   collision behavior, or do we need explicit collision-forced witnesses?
2. **Recursive-enum bounded depth.** Default depth bound of 5 is a guess.
   Should depth be a per-type `@witness_depth(N)` annotation? Should it
   participate in the content hash if it does?
3. **Cross-platform property-test seed reproducibility.** `proptest::TestRng`
   is deterministic given a seed, but floating-point witness behavior can
   diverge across x86-64 / aarch64 / wasm32 for some operations. Plan:
   seed is fixed per content hash; counterexamples are reported as
   `(seed, iteration_index)` pairs so divergence is reproducible on the
   reporting platform.
4. **NaN exclusion from F64 witnesses.** `f64::NAN != f64::NAN`, so
   commutativity / associativity / identity laws fail trivially on any
   function that propagates NaN. Default: exclude NaN from the random
   `f64` source; include only via `@example(f64::NAN)` opt-in on the
   parameter's type. Open: is there a use case for which NaN-included
   commutativity is the right default?
5. **`laws::distinguish` LSP integration.** REPL printing is
   straightforward; LSP hover is constrained on space. Strawman: hover
   shows the verified law set; clicking opens an LSP virtual document
   with the full distinguish matrix. The virtual-document plumbing is v3
   territory.
6. **Const-evaluability of `identity(e)`.** The identity element must be
   a const expression (so it's content-hashable). The const-evaluator
   handles literals trivially; does it handle `Money { cents: 0,
   currency: "USD" }`? The comptime evaluator currently does
   (`comptime.rs:300`), but the const-expr lowering for `@law` args needs
   to invoke it explicitly.

## Future possibilities

**v3 SMT discharge for arithmetic laws on small bounded scalars.** For
`@law(commutative, associative)` over `(i32, i32) -> i32` with no
control flow, the property is decidable by bit-blasting in milliseconds.
A z3-bindings backend slots in as an alternative discharge for laws
whose witness type and function shape pass a conservative analyzer; the
property-test path remains the fallback.

**Cross-function `homomorphism`.** v2/v3 adds
`@law(homomorphism(from = f, to = g, via = h))` for the law
`h(f(x, y)) == g(h(x), h(y))`. Witness generation crosses three function
signatures and is materially harder; deferred until the v1 mechanism is
load-bearing in the registry.

**Law-indexed package registry.** The package registry server
(`shape-registry/`) already verifies Ed25519 signatures on
`ModuleSignatureData`. Extending the registry index by
`(domain, law_set)` enables `pkg.shape-lang.dev/laws/(int,int)->int +
commutative + associative + identity(0)` as a first-class URL. Combined
with `laws::distinguish` this becomes the **algebraic search engine** the
v3 prior-art section calls out — Hoogle for behavior, not types.

**Integration with RFC-007 graph DB.** Once the graph-DB workstream lands
(RFC-007, planned), each verified law becomes a `has_law(fn, atom)`
predicate. Queries like `find fn where has_law(fn, commutative) AND
has_law(fn, associative) AND domain(fn) == (Money, Money) -> Money`
become Datalog rather than a stdlib function. `query_shape_laws` becomes
sugar over this layer.

**`@algebra` on type definitions.** Future annotation on `type`
definitions declaring carrier-set structure
(`@algebra(monoid(zero = ..., op = ...))`). Closes the loop with mathlib4's
algebraic-hierarchy framing — laws on functions plus algebras on types
yields the full structure inside the language.

## Phasing and cost

| Phase | Scope                                                                                       | Weeks |
| ----- | ------------------------------------------------------------------------------------------- | ----- |
| v1    | Closed enum (10 atoms), scalar property-test path, content-hash extension, REPL + MCP, sentinel test, no escape hatch | 6 |
| v2    | Collection laws (tier-2), TypedObject witness generation from `@example`/`@range`, `laws::distinguish` matrix UX | 3 |
| v3    | `homomorphism`, narrow SMT discharge for arithmetic-on-bounded-scalar, LSP virtual document, `@algebra` on types | 4 |
| **Total** |                                                                                          | **~13** |

v1 ships the mechanism, the discipline, and the agent-facing query
surface. v2 makes it useful for the collection-heavy code LLMs are
likeliest to author. v3 closes the loop with the formal-methods world and
unlocks the law-indexed registry as a separate workstream.

## Code touchpoints

Code carrying RFC-002 markers (`// RFC-002`) on landing:

- `crates/shape-ast/src/ast/functions.rs:202` — `Annotation` shape (no
  change; reused).
- `crates/shape-runtime/src/type_system/` — new `LawAtom` lowering pass.
- `crates/shape-runtime/src/stdlib/capability_tags.rs:14` — `pure` derivation.
- `crates/shape-vm/src/compiler/comptime.rs:300` — harness driver hook.
- `crates/shape-vm/src/resource_limits.rs:11` — tier presets.
- `crates/shape-vm/src/bytecode/content_addressed.rs:33,97,113,122` —
  four new fields + `FunctionBlobHashInput` extension.
- `crates/shape-vm/src/executor/tests/no_law_escape.rs` — new sentinel test.
- `shape-mcp/src/tools.rs:62` — two new MCP tool definitions.
- `tools/shape-lsp/` — hover string for verified law set.
- `docs/adr/` — companion ADR on tier defaults & rehash discipline.

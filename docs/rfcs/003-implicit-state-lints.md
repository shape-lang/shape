# RFC-003: Implicit-State Lints with Code-Fix

- **Status:** Draft
- **Authors:** Shape Language Team
- **Created:** 2026-05-18
- **Phase:** 1 of 3 (this RFC). Phases 2 (`flow` enum-sugar) and 3 (per-state types) are deliberately deferred to a separate RFC.

## Summary

Three new compiler lints — `S-LINT-001` (`bool-flag-state-field`), `S-LINT-002` (`mutually-exclusive-options`), and `S-LINT-003` (`boolean-blindness-args`) — that detect the boolean-blindness shapes LLMs and humans reach for instinctively when modelling state, plus an LSP code-fix (`shape.promote-to-enum`) that mechanically rewrites struct/enum boundaries on accept. Lints emit through the existing LSDS schema in `shape-diagnostics` and slot into the type-check pass after inference. A module-level `#[strict_state]` directive promotes the warnings to hard errors; the `[lints]` table in `shape.toml` is the project-wide opt-in.

## Motivation

LLMs generate Shape code under a strong prior: when state needs to be tracked, reach for `bool`. Training corpora are dominated by JavaScript, Python, and Go — ecosystems where `is_active: bool`, `is_admin: bool`, `has_loaded: bool` are stylistic norm rather than smell. Even when the underlying domain is obviously a state machine (a TreeNode is *either* a leaf with a `leaf_value` *or* an internal node with `split_feature`/`threshold`), the reflex is to flatten the discriminator into a bool flag and let caller discipline keep invalid states unreachable.

The result is `2^N` reachable configurations of which a tiny fraction are valid. `TreeNode { split_feature: int, threshold: number, left: int, right: int, leaf_value: number, is_leaf: bool }` has ~2 valid shapes encoded in 6 fields whose joint state-space is millions of nonsense combinations. Every consumer must gate on `is_leaf` before reading `leaf_value`; every test must set the gating flag consistently; every JSON deserializer validates combinations the type itself permits. Cost compounds: downstream LLM-generated consumers read `threshold` without checking `is_leaf`; property tests generate the invalid 99% of the state-space by default; adding a third state ("pruned leaf") forces a global audit instead of one match-exhaustiveness error.

The standard fix has been articulated for fifteen years (Minsky's "Make Illegal States Unrepresentable", 2010) and rearticulated continuously (King's "Parse, Don't Validate"; Kalume on Boolean Blindness). Shape has enums, struct variants, and pattern-matching — the language already supports the right shape. What's missing is a forcing function at the moment of authorship.

This RFC is Phase 1 of a three-phase arc:
- **Phase 1 (this RFC):** Detect and offer mechanical rewrites for the three most common bool-blindness shapes. Warning-only by default; project- or module-scoped escalation to error.
- **Phase 2 (future RFC):** A `flow` typestate construct — sugar over enum + transition functions with runtime-checked illegal transitions.
- **Phase 3 (future RFC, feature-flagged):** Per-state types as first-class — each enum variant projects to a distinct type the borrow checker tracks. Refused for inclusion here; scoping creep is exactly how RFCs become unshippable.

Phase 1 alone, well-executed, recovers ~80% of the ergonomic and correctness benefit. The remaining 20% needs design work that is not in scope.

## Guide-level explanation

### `S-LINT-001`: bool-flag-state-field

Diagnostic on struct fields whose names match `/^(is_|has_|was_|did_|should_|can_)/` and whose type is `bool`. Default severity: `Warning`. Promoted to `Error` under `#[strict_state]`.

**Before** (`packages/xgboost/index.shape:11`):

```shape
type TreeNode {
    split_feature: int,
    threshold: number,
    left: int,
    right: int,
    leaf_value: number,
    is_leaf: bool
}
```

**Diagnostic rendered (LSDS terminal output):**

```
warning[S-LINT-001]: field `is_leaf: bool` looks like a state discriminator
  --> packages/xgboost/index.shape:17:5
   |
17 |     is_leaf: bool
   |     ^^^^^^^^^^^^^ promote to enum variant
   |
   = note: fields prefixed `is_`/`has_`/`was_`/`did_`/`should_`/`can_` typed
           bool tend to encode mutually-exclusive states. The 5 sibling
           fields (split_feature, threshold, left, right, leaf_value) are
           reachable in 2^6 = 64 configurations of which only ~2 are valid.
   = help: `shape.promote-to-enum` quickfix rewrites to:
           enum TreeNode {
             Leaf { value: number },
             Split { feature: int, threshold: number, left: int, right: int },
           }
   = rule: RFC-003-§S-LINT-001
```

The LSDS `Diagnostic` payload carries `diagnostic_id = "S-LINT-001"`, `severity = "warning"` (or `"error"` under `#[strict_state]`), `rule = "RFC-003-§S-LINT-001"`, and a `SuggestedFix` whose `diff` field contains the unified-diff for the LSP/MCP quickfix to apply directly.

### `S-LINT-002`: mutually-exclusive-options

Triggers when a struct has ≥2 `Option<T>` fields and the lint's abstract interpreter can prove all constructors leave at least one disjoint pair always-Some/always-None.

**Before** (synthetic, but pattern-matches many LLM-generated parser AST types and config-record types):

```shape
type ParseResult {
    success_value: Option<int>,
    error_message: Option<string>,
}

fn parse_ok(v: int) -> ParseResult {
    ParseResult { success_value: Some(v), error_message: None }
}

fn parse_err(msg: string) -> ParseResult {
    ParseResult { success_value: None, error_message: Some(msg) }
}
```

**Diagnostic:**

```
warning[S-LINT-002]: fields `success_value` and `error_message` are mutually exclusive
  --> example.shape:1:1
   |
 1 | type ParseResult {
   | ^^^^^^^^^^^^^^^^
   |
   = note: abstract interpretation of 2 constructors (`parse_ok`, `parse_err`)
           proves: across every constructor, exactly one of {success_value,
           error_message} is None and the other is Some. The struct admits
           2^2 = 4 shapes; constructors only produce 2.
   = help: refactor to enum:
           enum ParseResult {
               Ok(int),
               Err(string),
           }
           — this is the shape your constructors already encode.
   = rule: RFC-003-§S-LINT-002
```

The diagnostic walks every callable constructor (free function returning the struct, plus inherent methods returning `Self`) and tracks per-field `OptionState` through it. If the join over all constructor exits proves the `success_value × error_message` cross-product never hits `Some × Some` or `None × None`, the lint fires.

### `S-LINT-003`: boolean-blindness-args

Triggers on functions taking ≥2 positional `bool` parameters, or any positional `bool` parameter where the call-site cannot be made readable by the parameter name alone (i.e. the parameter is non-trailing or has a generic name like `flag`, `enabled`, `force`).

**Before** (`stdlib-src/core/io.shape:153`):

```shape
pub builtin fn mkdir(path: string, recursive: bool) -> _;

// Call site:
mkdir("/tmp/build/cache", true)
```

What is `true`? The reader has to consult the signature.

**Diagnostic:**

```
warning[S-LINT-003]: positional bool parameter `recursive: bool` reads
                     unlabeled at call site
  --> stdlib-src/core/io.shape:153:33
    |
153 | pub builtin fn mkdir(path: string, recursive: bool) -> _;
    |                                 ^^^^^^^^^^^^^^^ bool argument
    |
    = note: bool args read as `f(x, true, false, true)` at the call site,
            which forces readers to look up the signature to disambiguate.
    = help: promote to a named-options struct or an enum:
            enum MkdirMode { Recursive, NonRecursive }
            pub builtin fn mkdir(path: string, mode: MkdirMode) -> _;
            // Now: mkdir("/tmp/build/cache", MkdirMode::Recursive)
    = rule: RFC-003-§S-LINT-003
```

The structurally-stronger case is `SimulationConfig` (`stdlib-src/core/simulation.shape:96`):

```shape
type SimulationConfig {
    initial_state: object,
    mode: string,
    collect_results: bool,
    collect_event_log: bool
}
```

Two adjacent bool fields. Construction sites read `SimulationConfig { ..., collect_results: true, collect_event_log: false }` — readable here because record-construction labels the fields, but the *type* still admits the four-way cross-product when only two combinations are domain-meaningful (collect both, collect neither, collect one). S-LINT-001 fires on `collect_results` and `collect_event_log` individually (they match `^(can_|should_|...)/` extended in §S-LINT-001's regex set to also include the `collect_` family — see Unresolved Questions); S-LINT-002 fires on the pair if constructors prove exclusion.

### LSP code-fix: `shape.promote-to-enum`

When `S-LINT-001` or `S-LINT-002` fires, the LSP code-action menu offers `Promote to enum`. Accepting it:

1. Replaces the `type Foo { ... }` with `enum Foo { ... }` whose variants are derived from the constructor abstract-interpretation result (S-LINT-002) or from a default split of `{Active, Inactive}` parameterized by the flag's prefix (S-LINT-001 — e.g. `is_leaf` produces `{Leaf { ... }, Internal { ... }}`).
2. Rewrites every constructor function body so the previous `Self { field_a: Some(v), field_b: None }` becomes `Self::VariantA(v)`.
3. Rewrites every read site so `x.field_a` becomes `match x { Self::VariantA(v) => Some(v), _ => None }` or, if the read is gated on the discriminator, a direct match.
4. Preserves doc-comments, formatting, and unrelated trailing whitespace by operating on the AST and re-emitting only the touched nodes through the source-formatter-preserving printer (the same path that `shape fmt --diff` uses).

If the rewrite would touch sites outside the current workspace (i.e. a public type with downstream consumers in installed packages), the quickfix surfaces a confirmation dialog listing the touched files and refuses to proceed without explicit consent. Workspace-internal rewrites apply immediately.

### `#[strict_state]` directive

Per RFC-001's directive grammar, `#[strict_state]` is a module-level attribute (it modifies compile-time behaviour and stays as `#[...]`, not `@strict_state` — the `@` prefix is reserved for annotations whose semantics may include runtime behaviour). It promotes the severity of S-LINT-001/002/003 from `Warning` to `Error` for everything in the annotated module:

```shape
#[strict_state]

type Connection {
    destination: string,
    is_open: bool,   // ← compile error here, not a warning
}
```

Modules building safety-critical state machines (transport layers, transaction coordinators, parser combinators) opt in to make the bool-blindness shapes hard to land. Authors that prefer warnings stay on the default.

## Reference-level explanation

### Pass placement

The lint pass runs immediately after `analyze_program_with_mode` returns its `TypeCheckResult` at `crates/shape-runtime/src/type_system/checker.rs:566`. Lints consume the populated `semantic_types` map plus the AST and emit LSDS diagnostics into the existing `warnings: Vec<TypeWarning>` vector — extended (or aliased) to `Vec<shape_diagnostics::Diagnostic>` to carry the structured payload through to the LSP/terminal renderers.

The pass is feature-isolated: it runs in `O(n)` over items, does no further inference, and can be disabled via `[lints] disable = ["S-LINT-001"]` in `shape.toml` without affecting type-check correctness.

### S-LINT-001 implementation

AST walk over `Item::StructType(StructTypeDef, _)` at `crates/shape-ast/src/ast/program.rs:66`. For each `StructField` whose name matches the regex `^(is_|has_|was_|did_|should_|can_)[a-z_]+$` and whose `field_type` resolves to `Type::Concrete(TypeAnnotation::Reference("bool"))`, emit an `S-LINT-001` diagnostic with a `SuggestedFix` whose `diff` is the unified-diff produced by the enum-promotion synthesizer.

The synthesizer derives variant names from the prefix:
- `is_leaf` → `{Leaf, Internal}`
- `has_*` / `was_*` → `{Has*, Lacking*}`
- `should_*` → `{Should*, ShouldNot*}`
- `can_*` → `{Can*, Cannot*}`
- `did_*` → `{Did*, DidNot*}`

with the sibling fields partitioned across the two variants by a heuristic (fields named in `{leaf,split,etc}` cluster under their respective variant based on substring match). When partitioning is ambiguous, the synthesizer emits both variants holding all non-discriminator fields and lets the user prune — the warning-level diagnostic is information-bearing even when the fix is imperfect.

### S-LINT-002 implementation: abstract interpretation reusing `try_null_narrowing`

The critical insight: Shape already has flow-sensitive optional-narrowing at `crates/shape-runtime/src/type_system/inference/statements.rs:220` (`extract_narrowings`), `:277` (`try_null_narrowing`), `:291` (`unwrap_optional_type`). The lattice S-LINT-002 needs is a tiny extension of the same machinery.

Define the per-field state lattice:

```
        Top
       /   \
NoneOrSome   ...
   /  \
NoneOnly  SomeOnly
       \ /
       Bottom
```

with 5 points: `{Bottom, NoneOnly, SomeOnly, NoneOrSome, Top}`. `Bottom` = unreachable; `NoneOnly` = field is `None` on every path here; `SomeOnly` = field is `Some(_)` on every path; `NoneOrSome` = both states reachable; `Top` = analysis precision lost (e.g. field assigned from a function call returning `Option<T>` that we can't see through).

Each constructor body is analysed by an abstract interpreter that maintains a `HashMap<FieldName, OptionState>`. Statement-level transfer functions:
- `let f: Option<T> = None` → `NoneOnly`
- `let f: Option<T> = Some(v)` → `SomeOnly`
- `if cond { f = Some(v) } else { f = None }` → join of branches = `NoneOrSome`
- `f = call_returning_optional()` → `Top`

The narrowing infrastructure at `try_null_narrowing` already proves "in this branch, `x: Option<T>` is `None`" or "is `Some(_)`" — the lattice operation is identical. We reuse the unwrapping logic verbatim from `unwrap_optional_type` and the branch-split logic from `extract_narrowings` / `extract_inverse_narrowings`. The estimated implementation effort drops from "design a new abstract interpreter" (~4 weeks, error-prone) to "extend the existing narrowing pass to record per-field state across constructor exits" (~1 week, well-trodden).

The lint fires when, for some pair of `Option<T>`/`Option<U>` fields `(a, b)` on the struct, the join over all constructor exits proves:

```
state(a) ∈ {NoneOnly, SomeOnly}  ∧
state(b) ∈ {NoneOnly, SomeOnly}  ∧
state(a) ≠ state(b)              -- i.e. one Some, one None
```

is true at every exit, AND there exist constructors that hit both `state(a) = SomeOnly, state(b) = NoneOnly` and `state(a) = NoneOnly, state(b) = SomeOnly` (i.e. both half-spaces are actually exercised, ruling out the case of a vestigial field nobody constructs).

When the lint can't prove exclusion (some constructor leaves a pair at `Top` or `NoneOrSome × NoneOrSome`), it stays silent — false negatives are strongly preferred over false positives.

### S-LINT-003 implementation

AST walk over `Item::Function(FunctionDef, _)` at `crates/shape-ast/src/ast/program.rs:48`. For each `FunctionDef` whose parameter list contains ≥2 `bool`-typed positional parameters, OR exactly 1 positional `bool` parameter whose name matches a generic-bool blocklist (`{flag, enabled, force, dry_run, verbose, debug, strict, allow, deny, recursive, async_mode}`), emit an `S-LINT-003` diagnostic.

The suggested fix proposes either:
- **Named-options struct** when ≥3 bool params: gathers all bool params into `struct FnNameOptions { ... }` with each field defaulted to `false`.
- **Enum** when exactly 2 bool params whose cross-product is meaningful as 3-4 named states.
- **Two-variant enum** when 1 bool param whose two values have natural names (`recursive: bool` → `MkdirMode::{Recursive, NonRecursive}`).

The heuristic picking between struct and enum uses the same prefix-derivation logic as S-LINT-001.

### Code-fix mechanics

The quickfix path lives in `tools/shape-lsp/src/code_actions.rs:88` (`get_quick_fixes`), dispatched on `diagnostic_code == "S-LINT-001" | "S-LINT-002"`. The action emits a `WorkspaceEdit` containing `TextEdit`s for:

1. The struct definition file (replace `type` with `enum`, rewrite fields as variants).
2. Every constructor function body in any file in the workspace that constructs the type (rewrite struct-literal to enum-variant constructor).
3. Every field-read site for fields that moved into variant payloads (rewrite `x.field` to a `match x { Variant(v) => v.field, _ => default }` or a destructuring pattern when the surrounding context already pattern-matches).

The implementation pulls the AST printer from `shape fmt`. Comments attach to AST nodes via leading/trailing-trivia attribution; the printer round-trips them. Edits that cross file boundaries propagate through the `ModuleCache` to look up cross-file constructor sites.

### `#[strict_state]` semantics

Lexed as a directive-form attribute at the module-attribute position (top of the source file, before any item). Stored on the `Program` AST node. Read by the lint pass when constructing diagnostics: if `strict_state` is set, `severity` is `Error` instead of `Warning`.

The directive is module-scoped, not item-scoped — bool flags interact across types within a module, and the directive scope mirrors that.

### `shape.toml [lints]` configuration

```toml
[lints]
# Severity overrides per lint code.
# Values: "error", "warning", "info", "hint", "off".
"S-LINT-001" = "warning"   # default
"S-LINT-002" = "warning"
"S-LINT-003" = "off"       # silence project-wide

# Optional per-file overrides via glob:
[lints.per-file]
"tests/**/*.shape" = { "S-LINT-001" = "off" }
"packages/critical/**" = { "S-LINT-001" = "error" }
```

Loaded by `crates/shape-runtime/src/project/project_config.rs:48`'s `ShapeProject` via a new `lints: Option<LintsSection>` field. Per-file overrides resolve at lint-emission time against the diagnostic's primary `Location.file`.

Precedence (highest first): file-level `#[strict_state]` > per-file `[lints.per-file]` override > global `[lints]` override > built-in default.

## Drawbacks

**False positives on legitimately-separate optional fields.** Some structs have two optional fields that *can* both be `Some`/`None` and don't encode a discriminator — `UserProfile { email: Option<string>, phone: Option<string> }`. S-LINT-002 mitigates by requiring constructor evidence that *both* exclusion half-spaces are exercised — a struct with one constructor setting `email: Some, phone: None` won't fire. This pushes false-positives toward zero in exchange for false negatives (structs with one constructor today that gain another next week).

**Code-fix complexity around derived traits.** If the struct has `@derive` annotations (Debug, Clone, Serialize), the enum rewrite must carry them. Derive machinery on enums vs structs differs (`Serialize` may produce different wire formats). The quickfix carries derives literally and surfaces a hint to verify; wire-compatibility is not claimed.

**Ecosystem churn if promoted to hard error too aggressively.** S-LINT-001 fires on existing stdlib (`is_open` in `Connection`, `is_leaf` in `TreeNode`, `converged` in `OptimizeResult` if regex extended). Blanket error-promotion breaks builds. Mitigation: ship warning-only; collect 6 months of telemetry; promote to error only via an edition bump (`edition = "2027"`).

## Rationale and alternatives

**Warning vs error by default.** Warning. Bool-blindness is common in existing Shape code and LLM-generated code; a hard error inflicts major churn before users absorb the rationale. `#[strict_state]` and `[lints]` let motivated authors opt in.

**Named-options struct vs enum for S-LINT-003.** Named-options structs preserve bool semantics with self-documenting call sites (`mkdir("/x", MkdirOptions { recursive: true })`); enums force the discriminator into the type system. The quickfix prefers enums at N ≤ 2 (named states crisper than `Options { recursive: false }`), options structs at N ≥ 3 (cross-product enums explode). Users pick the other via a second quickfix.

**Why not lint at parse time?** (1) Regex matches, but *promotion* requires constructor analysis (S-LINT-002) or type resolution (S-LINT-001 must know `bool` isn't a user alias). Parse time is too early. (2) Parse-time errors don't carry the structured-fix payload that lint-pass errors do.

**Why not lint on call-sites with bare `true`/`false`?** Clippy-style call-site linting catches the symptom; the disease is the type signature accepting bool in the first place. Lint at definition.

**Why not extend the regex to all bool fields?** Many bool fields are legitimate boolean propositions (`encrypted: bool`, `signed: bool`, `paid: bool`). The prefix set is a cultural signal of state-discriminator intent — narrower regex, lower false-positive pressure.

## Prior art

- **Yaron Minsky, "Make Illegal States Unrepresentable" (Effective ML, 2010).** Canonical statement of the principle.
- **Alexis King, "Parse, Don't Validate" (2019).** The boundary between unvalidated and validated data should be a type boundary, not a runtime-check boundary. S-LINT-002 mechanises one case: "any combination of optional fields" → "exactly one of these alternatives".
- **Yves Kalume, "Boolean Blindness in Kotlin" (2024).** Direct analogue of S-LINT-003.
- **Rafael Fernandez's "Make Illegal States Unrepresentable" series (2022–2024).** TypeScript-side discriminated-union refactors. Influences S-LINT-002 — Fernandez does mutual-exclusion proof by hand inspection of constructors, which we mechanise.
- **`eslint-plugin-react`'s `boolean-prop-naming` rule.** Closest existing S-LINT-001 analogue — inverts intent (enforces convention) but validates the regex is widely-recognized.
- **typescript-eslint feature request #515 ("Disallow boolean parameters").** Open since 2021; no implementation, partly because TypeScript lacks enum-with-payloads so the *fix* can't be mechanised. Shape can ship both halves.
- **Rust's clippy `fn_params_excessive_bools`.** Narrower S-LINT-003 sibling (≥3 bool params).
- **F#'s pervasive single-case discriminated unions.** Community convention "wrap your bool in a domain type" predates this RFC.

## Unresolved questions

1. **Code-fix interaction with `@derive`.** If a struct carries `@derive_clone`, the rewritten enum needs the same. Trivial in the simple cases; in cases where the derive macro inspects struct fields specifically, the enum rewrite may break. Should the code-fix bail out when any non-builtin annotation is present, or attempt the rewrite and rely on the user to surface failures?

2. **`#[strict_state]` vs `shape.toml` precedence subtlety.** What happens when `shape.toml` says `"S-LINT-001" = "off"` and a specific module says `#[strict_state]`? Currently the proposal is "module directive wins" — but a project lead might reasonably want `[lints]` to be the final word for consistency. Decide before stabilization.

3. **Extension of the regex set.** The current regex matches the six prefixes documented. Should it extend to `collect_*` (matches `SimulationConfig` flags), `auto_*`, `use_*`, `with_*`? Each extension trades catch-rate for false-positive rate. Suggest: ship the core six, gather a quarter of telemetry, expand based on real corpus evidence.

4. **Relationship to a future `flow` typestate RFC.** Phase 2 introduces `flow Foo { state A, state B, transitions A -> B via promote }` as sugar over enum + a transition-function table. S-LINT-001's mechanical fix produces a plain enum; should it produce a `flow` instead when the surrounding context hints at state-machine semantics? The clean answer is "no, separate concerns" — but if Phase 2 ships within a year, retrofitting all the S-LINT-001-driven enums to `flow` would be annoying.

5. **JSON/wire-format compatibility.** The standard enum-promotion changes how the type serializes (struct → tagged union). For types crossing wire boundaries (RPC payloads, persisted state), this is a breaking change. Should the lint suppress itself when it can detect the struct flows through `Serialize` / wire boundaries, or surface a stronger warning? Conservative answer: surface a stronger warning.

6. **Multi-discriminator structs.** What about `type Event { is_deleted: bool, was_archived: bool }`? Both prefixes match, and the constructors may or may not prove they're mutually exclusive. The two lints (S-LINT-001 and S-LINT-002) overlap here; the diagnostic-emission logic needs a deduplication policy.

## Future possibilities

**Phase 2: `flow` typestate.** Sugar over the enum-plus-transition-function pattern. Authors write:

```shape
flow Connection {
    state Closed,
    state Open { peer: string, since: DateTime },
    transition Closed -> Open via connect(addr: string)
    transition Open -> Closed via disconnect()
}
```

The compiler desugars to an enum + a transition-table + runtime-checked illegal transitions (panic on `disconnect` from `Closed`). S-LINT-001's mechanical fix evolves to optionally produce `flow` instead of bare `enum` when the surrounding code already has transition functions.

**Phase 3: per-state types behind feature flag.** Each variant projects to a distinct type that the borrow checker tracks separately. `let c: Connection::Open = connect(addr)?` gives `c.peer` type-safely. The cost is non-trivial — type inference complexity grows, and the variant-typing interacts with generics, traits, and pattern-matching in ways that need an entire design pass. Deferred indefinitely; feature-flagged when ready.

**Cross-package corpus telemetry.** Lint-fire frequencies could be aggregated (anonymously, opt-in) by the package registry to surface high-bool-density packages and feed back into per-prefix regex tuning.

**Code-fix bundling.** When a single user accepts S-LINT-001 on `TreeNode { ..., is_leaf: bool }`, all downstream consumers (in installed packages) of `TreeNode` get a follow-up "the type you depend on changed shape" diagnostic with their own quickfix to update.

## Phasing and cost

| Phase | Scope | Estimate |
|-------|-------|----------|
| **1a** | S-LINT-001 implementation, regex match + LSDS emission + warning rendering. Ship as warning-only. | ~1 week |
| **1b** | S-LINT-002 implementation. The lattice maps 1:1 onto the existing `try_null_narrowing` / `extract_narrowings` infrastructure (`statements.rs:220`, `:277`, `:291`); the abstract-interpretation pass is a per-constructor join with a 5-point lattice. Implementation is genuinely ~1 week because the underlying narrowing machinery exists. | ~1 week |
| **1c** | S-LINT-003 implementation. Pure AST walk; no inference work. | ~1 week |
| **1d** | LSP code-fix `shape.promote-to-enum`. The longest line item: AST-to-AST rewriter, comment preservation via the formatter-preserving printer, cross-file constructor / read-site updates via `ModuleCache`, confirmation dialog for cross-package edits. | ~2 weeks |
| **1e** | `#[strict_state]` directive lexing/parsing + module-attribute wiring + lint-pass severity override. | ~2 days |
| **1f** | `[lints]` table in `shape.toml` + precedence resolution + per-file glob support. | ~2 days |

**Total Phase 1: 3-4 weeks for the lints, +2 weeks for the code-fix = 5-6 weeks of focused work.**

**Hardening path.** Ship 1a–1f as warning-only. Collect telemetry for 6 months: lint-fire counts, code-fix accept-rate, user-reported false-positive rate. Promote to default-error via an edition bump (e.g. `edition = "2027"` in `shape.toml`) where users explicitly opt in to the stricter rule set. Existing projects on the prior edition see warnings indefinitely until they opt in.

**Phase 2 (`flow`) and Phase 3 (per-state types) are out of scope for this RFC.** They will be proposed in separate RFCs after Phase 1 has shipped and the telemetry is in. Refusing to scope them here is deliberate: each is a 2-3 month design effort with non-trivial implementation surface, and bundling them into one RFC is the standard way these proposals stop landing.

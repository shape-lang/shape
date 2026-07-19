# E1 #17 slice-1 report — public `CheckedBody` builder + `finish()`

Discharges the user-ratified **C2-D1 amendment** (issue #13): C2 shipped the
installation VALIDATOR; E1 ships the public CONSTRUCTION surface its consumers
(E1's typed rewrite directives, slices 3/4/5) build a generated body through.
Branch `adr009/e1`. Review-mandatory slice. New code:
`crates/shape-vm/src/compiler/comptime_fragments/checked_body.rs` (+ a one-line
module declaration in `comptime_fragments/mod.rs`). Supervisor runs the build;
this author does not.

Ratified design (team-lead, after independent anchor verification):
reading (a) compiler-internal Rust builder; `finish()` = construction chokepoint;
typestate `CheckedBodyBuilder<SigState, CapturesState>`; API-foundation-only.

---

## What landed

- **`CheckedBody`** — the provenance-ready carrier: `{ signature: BodySignature,
  captures: CaptureClause, body: Vec<Statement> }`, private fields, accessors +
  `into_body()`. No public constructor and **no `from_source(&str)` / string /
  JSON constructor anywhere** — the only way to obtain one is `finish()`.
- **`BodySignature`** — `{ params: Vec<FunctionParameter>, return_type:
  Option<TypeAnnotation> }`, built from the AST pieces the directive consumer
  already holds off the target `FunctionDef`.
- **`CheckedBodyBuilder<SigState, CapturesState>`** — typestate builder.
  `new() -> <Missing, Missing>`; `.signature(..) -> <Present, _>`;
  `.captures(..) -> <_, Present>`; `.body(..)` in any state; **`finish()`
  implemented ONLY for `<Present, Present>`**.
- **`finish() -> Result<CheckedBody, ShapeError>`** — validates the typed inputs
  and returns the carrier or a named diagnostic, never a silent partial.

## The construction/install split (the review guard — stated explicitly)

`finish()` is the **construction side ONLY**. It validates typed inputs and
returns a provenance-ready `CheckedBody`; it does **not** install, publish,
stamp, or reserve, and holds no `&mut BytecodeCompiler`. This mirrors the shipped
`CheckedItem` precedent ("provenance-READY, not yet reserved",
`comptime_fragments/mod.rs:88-92`): a comptime builtin has no `&mut` compiler
access, so the atomic publish happens later, at the directive consumer, through
the ALREADY-open C2 `InstallTransaction` + §4.2 battery
(`crate::compiler::checked_body`).

The Decision-95 "checks and installs atomically" property is therefore discharged
**by composition**, not by this module alone:

```
finish()                     -> CheckedBody   (construction-side, comptime_fragments::checked_body)
consumer + driver check seq  -> atomic install (C2 seam, compiler::checked_body)
```

**Binding invariant inherited by slices 3/4/5:** every consumer MUST route through
BOTH — `finish()` to obtain the carrier, THEN the driver's shared check sequence /
C2 `InstallTransaction` to publish it — **never either alone**. `finish()` alone
yields a validated-but-un-installed carrier; the C2 transaction alone bypasses the
typed construction guarantees this builder exists to enforce. This split is stated
verbatim in the module doc and on the `CheckedBody` / `finish()` doc comments so
the invariant travels with the code.

## "Never a silent partial" is a type-system guarantee

`finish()` exists only on `CheckedBodyBuilder<Present, Present>`, so finishing a
builder whose signature or capture set was never supplied is **unrepresentable** —
rejected by the Rust type system at compile time, the same discipline as
`ProofGap`'s private constructor. No runtime completeness gate; no partial
`CheckedBody` can be constructed.

The typestate parameters `SigState`/`CapturesState` track SUPPLIED-NESS only and
are named distinctly from Decision-95's SEMANTIC `CheckedBody<Sig, Captures>` type
parameters (the Shape comptime-type face), which land with the Decision-95 Shape
staging surface in a later E-track/C3 slice. Documented at the type.

## Construction rejection matrix (D4: reuse authoritative codes)

| Class | Code | Where checkable at construction |
|---|---|---|
| borrow-mode capture (`&` / `&mut`) | `[C0902]` | `entry.mode.is_borrow()` — no scope needed |
| duplicate capture name | `[C0907]` | name-level dup scan (the subset before slot resolution, which stays the planner's install-time job) |
| empty body | un-numbered named error | `body.is_empty()` |

`[C0902]`/`[C0907]` are the authoritative capture-family codes (`captures.rs`
table; `capture_plan/planner.rs:221`), reused per D4 — no parallel codes minted.
**Empty-body is deliberately left un-numbered**: minting a fresh `C09xx` now would
race E1-D4's concurrent `C0930` "next-free" computation; the message is
self-describing ("a checked generated body must contain at least one statement").
Flagged for a coordinated code allocation at the C092x-follow-up (issue drafted in
the E2 close report Appendix A). An EMPTY capture *clause* is NOT a rejection — it
is the valid Decision-95 "captures nothing, explicitly" complete environment.

## Tests (supervisor runs; filter `cargo test -p shape-vm --lib comptime_fragments::checked_body`)

Per review condition (a) — one NEGATIVE per named rejection class + the typestate
doc note:

| Test | Class |
|---|---|
| `finish_produces_a_carrier_reflecting_the_typed_inputs` | happy path |
| `empty_capture_clause_is_a_valid_complete_environment` | empty clause is NOT a rejection |
| `borrow_mode_capture_is_rejected_c0902` | negative — `[C0902]` (both borrow modes) |
| `duplicate_capture_name_is_rejected_c0907` | negative — `[C0907]` |
| `empty_body_is_rejected` | negative — empty body |
| `typestate_transitions_are_order_independent` | captures-then-signature == signature-then-captures |
| doc note in the test module | the compile-time typestate guarantee (a non-`<Present,Present>` `finish()` does not compile — not runtime-testable) |

## API fit against the slice-3/4/5 emit sites (review condition (b))

The consumers that will wire this builder:

- **Slice 3 (U01 literals → store+index)** and **slice 4 (extend → typed
  carrier)** produce generated method/free-function bodies. They hold the target
  signature (params/return off the `FunctionDef` / `ComptimeTarget`) and a
  declared capture clause (or an explicitly-empty one) — the exact
  `BodySignature` + `CaptureClause` inputs. They call `finish()` for the carrier,
  then route it through the existing `check_generated_function_item` / directive
  install path (the C2 seam) — the composition invariant above.
- **Slice 5 (U02 expr)** resolves the type ref (per the slice-0 verdict) into the
  signature's return/param type; the body carrier is orthogonal and unchanged.

No consumer needs a string body or an inferred capture set, so the no-string /
typestate-complete shape fits. **Per review condition (c)**, any API change forced
at wiring time lands as an append-only delta with a re-review, never a silent
reshape of this surface.

## Boundary (review ruling: API-foundation-only)

No live consumer is wired in slice 1 — matching how `CheckedItem` / `CheckedModule`
landed as carriers before their consumers. Dead-code warnings on the not-yet-
consumed surface are expected and acceptable (ratified). `just check-clean` is
`cargo check` (warnings do not fail it).

## Forbidden-patterns check

No source-string reparse path anywhere on the surface (no `from_source`, no string
constructor) — the opposite of the U-class reparse protocol E1 deletes. No dynamic
fallback, no bridge/probe/helper rename. The construction/install split is a
genuine two-chokepoint composition (each a real, named validation stage), not a
retained-fallback dressed as a seam.

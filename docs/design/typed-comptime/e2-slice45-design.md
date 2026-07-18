# E2 #18 slice 4.5 — serialize producer design proposal (E2-Q2)

**Status:** DESIGN — for supervisor sign-off before any implementation.
**Base:** `692338b0` (slice 4 closed). Review-mandatory, design-first.
**Ruling:** E2-Q2 (USER-RATIFIED 2026-07-18) — the extend-method / computed-body
typed producer serialize.shape needs is pulled into E2 as this slice, keeping
E2-Q1-A totality intact (no surviving reparse arm). Dec-73 checked-splice form,
NO `$` sigil (E2-D3), hygienic, ConstLift only; NOT a public builder API (that
surface is E1's per the C2 D1 amendment); a grammar STOP escalates to the user.

---

## 0. The exact need (what serialize.shape emits today)

`@to_json` (`serde/serialize.shape`) is `targets: [type]`. Its handler computes,
per declared field, a JSON piece and emits ONE item — an extend-METHOD:

```
extend {Type} { method to_json() -> string { <f-string body> } }
```

where `<f-string body>` is a COMPUTED f-string that interpolates `self.{field}`
at RUNTIME. For `type User { id: int, name: string }` the emitted body is the
f-string `f"{ "id": {self.id}, "name": "{self.name}" }"` (object braces literal,
`{self.id}` / `{self.name}` interpolated), and `u.to_json()` returns
`{ "id": 1, "name": "Ada" }` (parity arbiter: showcases.rs `TO_JSON_EXPECTED`).

Two properties make this the E2-Q2 gap (neither expressible by the slice-2/4
surface):

- **P1 — it is an extend METHOD, not a free function.** `item_fn` +
  `parse_extend_items_slot` yield `Item::Function`; there is no typed producer
  for a method on a type.
- **P2 — the body is a runtime-self-interpolating f-string, not a literal.**
  `item_fn` bodies are literal-only (`literal_expr_from_slot`: String / Int /
  Number / Bool). This body reads `self.<field>` at runtime.

---

## 1. Two load-bearing facts that reshape the option space

Before the candidates, two facts from the code that correct the assumed shape of
the surface:

**Fact A — `CheckedItem` and the ExtendItems consumer are already `Item`-general
(so P1 is nearly free).** `CheckedItem { item: Item }` (slice 2) carries ANY
`Item`; `parse_extend_items_slot`'s fragment branch returns
`vec![checked.into_item()]` unconditionally; and the ExtendItems materialization
already has a full `Item::Extend` arm — the pre-pass
(`materialize_computed_comptime_extends`, the `Item::Extend(mut extend, _)` arm)
desugars each method, stamps generated provenance
(`stamp_generated_analysis_method`), reserves a hygienic identity
(`reserve_generated_decl_journaled`), and registers it; pass-2's
`apply_comptime_extend` re-issues the identical reservation and compiles the
body. That arm is what today's `extend (f"extend Type { method }")` SOURCE route
already flows through after `parse_program`. So a typed producer that BUILDS an
`Item::Extend` and stashes it as a `CheckedItem` reuses the ENTIRE carrier +
consumer + provenance/hygiene path with **zero consumer change**. The only new
code is the builder that constructs the `Item::Extend` AST. This is why the
blast radius is a single builtin (§2).

**Fact B — Shape has NO structured f-string AST (so the assumed candidate (a) is
not literally constructible).** An f-string is `Literal::FormattedString { value:
String, mode }` — FLAT TEXT. Its `{...}` holes are parsed at COMPILE time by
`shape_ast::parser::parse_expression_str` (`string_interpolation.rs:277`), the
language's native f-string path, invoked identically for every hand-written
f-string. There is no `Expr::FString { parts: Vec<Segment> }` to "assemble
segments into." So the brief's candidate (a) ("assembled by the builtin into
Expr::FString AST — no parsing") cannot be built as stated: any native f-string
body carries a `value: String` whose holes parse at compile. The only body shape
that touches NO parser at all is a concatenation `Expr` tree (candidate C).

This forces the real fork onto ONE axis: **how the method body is represented**,
with a genuine tension between byte-parity safety (reuse the native interpolation
path → some hole-parsing) and forbidden-pattern purity (build pure `Expr` →
must replicate the interpolation formatter). §3 is where the ruling is needed.

---

## 2. Blast radius (files, grammar, provenance) — feasible, well under the STOP

Independent of which body candidate is chosen, the producer shape is:

- **NEW comptime builtin** in `crates/shape-vm/src/compiler/comptime_builtins.rs`
  (alongside `item_fn`) that builds an `Item::Extend { type_name, methods: [one
  MethodDef] }`, stashes it via `CheckedItem::new(item)` into the existing
  `COMPTIME_CHECKED_ITEMS` store, and returns a `__CheckedItem` handle — exactly
  the slice-2 mechanism. Est. +60–110 lines (builder + arg reading; the higher
  end is candidate B's structured-parts reader).
- **serde/serialize.shape** — rewrite the `@to_json` handler to call the new
  builtin instead of `extend (f"extend {Type} { … }")`.
- **Tests** — a producer-tier unit test module + the existing showcases
  `to_json_serializes_via_stdlib_import_{vm,jit}` as the byte-parity arbiter
  (unchanged), plus a negative pin (§4).
- **Consumer path: NO change** (Fact A) — `parse_extend_items_slot`, ExtendItems,
  the `Item::Extend` materialization arm, and its stamp/reserve/register wiring
  are reused as-is.
- **Grammar: ZERO pest changes.** The producer is a builtin CALL; no new syntax.
  (If any candidate is found to need a grammar change during implementation, that
  is the E2-D4 STOP → user, per the ruling.)
- **Provenance / hygiene / InstallTransaction:** reused, not re-wired — the
  `Item::Extend` arm already runs `stamp_generated_analysis_method` +
  `reserve_generated_decl_journaled` (journaled through the open
  `InstallTransaction`) + `register_function`, the same sequence slices 1–3
  established.

**Honest file estimate: 3–4 files** (builtin, serialize.shape, one test module,
optionally a comptime_fragments doc line). Well under the >10-file STOP. No
grammar. **The minimal subset IS expressible without grammar or a >10-file
ripple** — no escalation on feasibility grounds; the only open question is the
body-representation ruling (§3).

---

## 3. The surface — the method body (THE crux; ruling needed)

All candidates share the outer builder: a builtin (proposed name
**`extend_method`** — reads as "add a method to a type", parallel to `item_fn`;
alternatives: `method_fn`, `type_method`) that builds
`Item::Extend { type_name: TypeName::Simple(type), methods: [MethodDef { name,
params: [], return_type, body, is_async: false, … }] }`. `self` is implicit
(empty params), matching the source `method to_json()` the desugarer already
handles. They differ ONLY in how `body` is produced.

### Candidate A — f-string VALUE carrier (simplest; REJECTED)

`extend_method(type, "to_json", "string", body_value)` where `body_value` is the
computed f-string CONTENT; the builtin builds
`Statement::Expression(Expr::Literal(Literal::FormattedString { value:
body_value, mode: Braces }))`. serialize.shape keeps computing the body text and
passes it in.

- **+** Smallest change; reuses the exact interpolation path → byte-identical.
- **−** The handler AUTHORS the f-string body text. This only moves the authored
  text from inside `extend (f"…")` to a builtin argument — the OUTER directive
  reparse (`parse_program`) is gone, but the handler still hand-writes body
  source. Against the ADR's "computed structure uses typed builders" and closest
  to the E2-Q2 "body-as-text" line. **Recommend REJECT.**

### Candidate B — typed template builder → native f-string (RECOMMENDED)

`extend_method(type, "to_json", "string", parts)` where `parts` is structured
comptime DATA the handler builds from the type's fields — an ordered list whose
elements are either a **literal segment** (ConstLift'd JSON punctuation /
field-name text) or a **self-field splice** (a field NAME to read at runtime).
The builtin ASSEMBLES the `Literal::FormattedString { value }` from the parts,
emitting each splice as a minimal, producer-generated `{self.<field>}` hole and
each segment as escaped literal text.

- **+** The handler passes STRUCTURE, never authored body source — the ADR
  "typed builder" shape. Field names/punctuation cross via ConstLift (comptime
  data → generated literal segments); the runtime field reads are producer-built
  `self.<field>` holes, never handler text.
- **+** Byte-identical: the body is a native f-string compiled through the SAME
  `emit_interpolation_format_call` path as today — no formatter to replicate.
- **~** Residual: the producer-generated `{self.<field>}` holes are resolved by
  the f-string's native `parse_expression_str` at compile (Fact B — unavoidable
  for ANY native f-string). These holes are trivial, structural, and
  producer-controlled (never a full body/item reparse; never the U03 directive
  transport). **Whether this residual native-hole parse is acceptable is the one
  ruling this design needs.** My read: it is — it is the language's own f-string
  mechanism on producer-synthesized field reads, categorically distinct from the
  U03 `parse_program`/`parse_function_body_payload` directive transport E2
  deletes.
- **−** Richer builtin ABI than `item_fn` (reads an array of part descriptors).
  Bounded (~ +40 lines over candidate A).

### Candidate C — typed builder → concatenation Expr (purist; byte-parity risk)

Same structured `parts`, but the builtin builds a pure `Expr` tree: literal
segments as `Literal::String`, splices as `Expr::FieldAccess(self, field)`,
joined by string concatenation, with per-field-type quoting/formatting — ZERO
parser involvement anywhere.

- **+** No `parse_expression_str` at all; fully typed AST; the strictest reading
  of "no source text."
- **−** Byte-parity is NOT free: the current output is produced by the f-string
  interpolation formatter (`emit_interpolation_format_call`, default spec per
  value type). A concatenation must REPLICATE that formatter exactly (int/number/
  bool → string, and the string-field quoting) to stay byte-identical to
  `TO_JSON_EXPECTED`. That is a real, verify-heavy parity surface and a latent
  drift risk if the interpolation default formatting ever changes.
- **−** More builder logic (type-directed quoting/among segments).

### Recommendation

**Candidate B.** It is the ADR-mandated typed-builder shape (handler passes
structure, not authored source), it is byte-parity-safe by construction (reuses
the native interpolation path — the showcases `to_json` rows stay green without a
re-implemented formatter), and its only residual is the language's own f-string
hole-resolution on producer-generated `self.<field>` reads — not the deleted
directive transport. **Fallback:** if the reviewer rules the residual native-hole
parse too close to the refused line, Candidate C, gated on an explicit
byte-parity verification that the concatenation replicates
`emit_interpolation_format_call`'s default formatting for `int`/`number`/`bool`/
`string` fields (a producer-tier golden test against `TO_JSON_EXPECTED` plus each
scalar field type). Candidate A is rejected either way.

**Why NOT a public-builder API / NOT source-reparse-renamed.** `extend_method`
is an INTERNAL comptime builtin (SOH-gated / stdlib-consumed, exactly like
`item_fn`), not a user-facing quote/splice surface — the ADR states "Shape has
no public quote/splice sublanguage," and the public builder + `finish()` surface
is E1's per the C2 D1 amendment. It is not a rename of source-reparse: it never
calls `parse_program` / `parse_function_body_payload`; it constructs `Item::Extend`
+ `MethodDef` AST directly (B/C), and even B's f-string leaf uses the language's
native literal, not a directive transport.

---

## 4. Parity + pins plan

**Byte-parity arbiter (unchanged, must stay green):**
`tools/shape-test/tests/annotations_comptime/showcases.rs::to_json_serializes_via_stdlib_import_{vm,jit}`
(== `TO_JSON_EXPECTED` `{ "id": 1, "name": "Ada" }`), plus the negative
`@to_json` error-path row if present, and `comptime_builtins` unit tests.

**New producer-tier pins (positive):**
- `extend_method` builds an `Item::Extend` with exactly one `MethodDef`, correct
  name / return type / implicit-self params, and the body compiles+runs to the
  expected string (VM). A scalar-coverage fixture over `int` + `string` (and,
  guarding P2, `number`/`bool`) so a formatter regression trips loudly —
  especially load-bearing under Candidate C.
- Hygiene + provenance: the generated method reserves a hygienic identity and
  carries the generated stamp (assert via the same reservation/`generated_symbols`
  witnesses slices 1–3 used), and a failing install rolls it back through the
  open `InstallTransaction` (reuse the slice-3 rollback-pin shape).
- VM+JIT: `to_json` has no closure and no async → native both tiers; a
  `jit_c2_install_native` zero-fallback row (mirroring the e2 sibling rows), if
  the supervisor wants the CLI proof alongside the showcases jit row.

**Negative pin (forbidden-shape guard):**
- Assert the producer path performs NO source reparse of the body/item — e.g. a
  guard/test that `extend_method` produces its `Item::Extend` without invoking
  `parse_program` / `parse_function_body_payload` / `parse_module_items_payload`
  (the deletion-target parsers), so a future "just pass the whole method as text"
  regression fails loudly. (Mechanism TBD at implementation — a construction-site
  assertion or a `check-no-dynamic`-style grep row.)

---

## 5. What this slice does NOT do (scope fence + E1 hand-off)

- **NOT the general quote-item / template feature.** This is the serialize-shaped
  minimal subset: a SINGLE extend-method with a scalar-field JSON body. No
  multi-method emission, no generic method type-params, no arbitrary statement
  bodies, no non-`self` captures. `quote item` / `quote module` and the general
  computed-structure surface remain E-track-future (E2-D4 / E2-D10).
- **NOT a public builder API.** `extend_method` is internal (stdlib-consumed).
  The public `CheckedBody`/`CheckedTemplate` builder + `finish()` surface is
  E1's (C2 D1 amendment). E1 hand-off: when E1's typed rewrite-plan / builder
  surface lands, `extend_method` is either subsumed by that surface or remains
  the internal fast-path it consumes — E1 owns that reconciliation; this slice
  does not pre-empt it.
- **NOT a grammar change.** Zero pest edits (§2). If implementation discovers a
  grammar need, STOP → user (E2-D4).
- **Ordering unchanged:** slice-5's TOTAL U03 deletion stays BLOCKED on this
  slice completing (E2-Q2), so serialize.shape is migrated off the source-string
  arm before the reparse parsers are deleted.

---

## 6. Open question for sign-off

One ruling decides implementation: **is Candidate B's residual native f-string
hole-parse (producer-generated `{self.<field>}` resolved by the language's own
`parse_expression_str`) acceptable, or is Candidate C (pure concatenation Expr,
with a replicated-formatter byte-parity gate) required?** Everything else
(single-builtin blast radius, zero grammar, reused provenance, the parity
arbiter) is settled and feasible.

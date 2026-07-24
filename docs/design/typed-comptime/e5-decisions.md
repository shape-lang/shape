# E5 — Program of record: delete the comptime `.source` reparse fallback

Binding for all E5 implementers/reviewers. Base `9028d2be` (branch `adr009/e5`,
worktree `shape-adr009-e5`). Authority stack: issue #61 (charter, **re-scoped by
the dated USER rulings below**), the E5 design synthesis
(`/tmp/.../scratchpad/e5/e5-design.md`, scouts A/B/C + synthesis), and the
Forbidden-Patterns core (`CLAUDE.md` §Forbidden — `.source` reparse IS a
dynamic-reparse fallback; E5 deletes it, never adds another).

This is the durable program-of-record. Session handoffs live in `/tmp` and are
never committed (memory: handoffs-are-scaffolding); this file + the CKPT reports
+ #61 are the durable authority.

## The charter, and the mis-aimed-step-1 correction (design §0)

E5's charter (#61) is: **delete the comptime type-ref `.source` reparse
fallback** — a DYNAMIC-REPARSE fallback (Forbidden-Patterns core). Today an
applied generic (`Array<int>`, `Option<T>`, `HashMap<K,V>`, `Result<T,E>`) does
NOT reconstruct → it falls UNSTAMPED to the `__ComptimeTypeRef.source` reparse
arm. The deletion is gated by `stamp_for` →
`reconstruct_type_annotation(...).is_ok()` (comptime_target.rs).

**Mis-aimed-step-1 finding (scouts converged; synthesis re-verified against
source).** #61's stated "step 1 = build applied-nominal DESCRIPTOR
substitution" is **not** the mechanism that unblocks the `.source` deletion.
There are two ORTHOGONAL capabilities #61 conflates:

| | Piece 1 — DESCRIPTOR substitution | Piece 2 — SPELLING reconstruction |
|---|---|---|
| answers | "what is `Array<int>`'s declaration SHAPE?" | "what is `Array<int>`'s type_ref SPELLING?" |
| fn | `payload_of` → `substituted_applied_nominal` | `reconstruct_type_annotation` |
| powers | `reflect(Array<int>)` completeness | `stamp_for` → producers stamp → `.source` dies |
| deletion blocker? | **NO** (severable reflect-completeness) | **YES** |

Proof of decoupling: `stamp_for` admits an identity iff
`reconstruct_type_annotation(...).is_ok()` — it never calls `payload_of`. A
perfect `payload_of` descriptor still hits reconstruct's blanket-`Err` Nominal
arm → unstamped → reparse. So the `.source` deletion needs **Piece 2**; Piece 1
is a separate reflect-completeness deliverable.

## USER scope rulings (dated re-disposition of #61 — binding)

Per the standing scope-reclaim ruling (a charter issue's scope moves ONLY by a
dated USER re-disposition, never a supervisor cite), the two genuine forks (B1,
B2) went to the USER and were ruled:

- **B1 — Piece 1 IN (2026-07-24).** E5 OWNS applied-nominal DESCRIPTOR
  substitution (`reflect(Array<int>)` completeness), landed as **CKPT-2** —
  NOT deferred to a separate ticket. (Overrides the design's Piece-2-only
  default.)
- **B2 — records IN, pre-check-SAFE (2026-07-24).** Records ENTER E5: build the
  record-field-name-preservation freeze fact so records spell/reflect, landed as
  **CKPT-3**. The user verdict is that records-in is **pre-check-SAFE** — see the
  CKPT-0 binding invariant below. (Overrides the design's records-OUT default.)

Consequence: E5 is the full arc — spell (CKPT-1), substitute (CKPT-2), records
(CKPT-3), migrate producers (CKPT-4), DELETE `.source` (CKPT-5), re-baseline
(CKPT-6). Both severable feature-expansions are IN by user ruling.

## CKPT-0 — SAFE verdict + the binding invariant

CKPT-0 is the safety scout gate. Verdict: **SAFE** to pull records into E5,
UNDER a binding invariant that is the substrate for CKPT-3:

> **Binding invariant (record hygiene).** A record's frozen IDENTITY and its
> hygienic MEMBER identity strings (`member:record:{record_hex}:{field}`, Dec 57)
> stay BYTE-IDENTICAL across E5. Any field-NAME the record freeze learns (CKPT-3)
> is **spell/reflect-only** — a presentation fact layered beside the identity,
> never folded into the identity hash or the member-identity minting. Records
> stay identity-declaration-order-independent (byte-sorted field names). This is
> what makes records-in pre-check-SAFE: reflection/spelling gains a name, the
> soundness-bearing identity algebra is untouched.

## Ruled (a)-forks (supervisor-ruled from the scout forks)

- **A1 — sibling accessor, NOT a new `FrozenPayloadDescriptor::AppliedNominal`
  enum variant.** `FreezeOverlay::applied_nominal_of` reads the already-derived
  `composites` memo `applied_nominal` field; `bare_nominal_name_of` reads the
  base `frozen_nominal_descriptors`. Smaller blast radius; still ONE code path
  (producer gate + consumer both call `reconstruct_type_annotation`). Do NOT
  ripple the sealed enum. (Landed CKPT-1.)
- **A2 — identity-indirected recursion, bound as an EXPLICIT INVARIANT — NO
  eager nested expansion.** An applied form spells its head then RECURSES on its
  ordered `arg_identities`; a bare-nominal arg is a LEAF spelled by name, never
  field-expanded. `Array<Tree>` (recursive `type Tree { kids: Array<Tree> }`)
  terminates. Nested applied args (`Array<Option<int>>`) terminate because
  `arg_identities` is the finite content-derived decomposition the freeze memo
  interned for every sub-expression (projection.rs CKPT-1 recursive
  sub-expression memoization). Bound in a code comment + the nesting pin. (Landed
  CKPT-1.)
- **A3 — un-applied generic heads STAY the existing named rejection**
  (`unapplied_generic_head_rejection`). Bare `Array` (no args) is NOT spelled.
  `bare_nominal_name_of` returns `None` for any head with declared param kinds
  but no frozen nominal descriptor. (Landed CKPT-1.)
- **A4 — producer classes A/B/D MIGRATE** (thread overlay + field ASTs). (CKPT-4.)
- **A5 — producer classes C/E NAMED surface-and-stop** (unresolved-return
  fallback; bare-string type-payload arm). (CKPT-4.)
- **A6 — records: NAMED Err interim in CKPT-1** (the existing Record arm is a
  loud named rejection); records SPELL in **CKPT-3** (B2 in-scope). Never a
  silent reparse.
- **A7/A8 — only live because Piece 1 is IN (CKPT-2):** container→NominalShape =
  Opaque-for-containers + Enum-for-Option/Result. **A8 RULED OUT of CKPT-2
  (arity-only) — see the "CKPT-2 — LANDED (A8-OUT)" section below.** The
  enum-payload-type freeze enrichment this line originally sketched is DEFERRED
  to issue #87 (A7-uniform `type_argument` recovery makes it redundant for
  soundness/completeness). (CKPT-2.)
- **A9 — co-located legacy `type_info` deletion:** rides E5's ticket vs splits —
  supervisor decides at CKPT-5/6 (no user surface).

## The CKPT-0..6 sequence (E2-D8 total-deletion discipline: additive → migrate/rule → pure delete)

- **CKPT-0 — safety scout.** SAFE verdict + the record-hygiene binding invariant
  above. No code.
- **CKPT-1 — Piece 2 SPELLING reconstruction (additive; DELETES NOTHING).** New
  `applied_nominal_of` + `bare_nominal_name_of` accessors + one new early arm in
  `reconstruct_type_annotation` (BEFORE the `payload_of` arms). projection.rs
  recursively memoizes every composite sub-expression so nested applied forms
  answer. The stamp-gate AUTO-WIDENS (no `stamp_for` edit) → applied generics
  stamp + stop reparsing. Pin 3916 case-1 flips to `Ok(Generic)`; pin 3747's
  comment reworded (payload_of still pends — that is CKPT-2). **← THIS CHECKPOINT
  (report: `e5-ckpt1-report.md`).**
- **CKPT-2 — Piece 1 DESCRIPTOR substitution (soundness-critical; B1 in-scope).**
  Extend `substituted_applied_nominal` with a builtin/enum branch (A7 container
  mapping + A8 enum-payload freeze) so `payload_of(Array<int>)` returns a
  descriptor → `reflect(Array<int>)` completes. Flips pin 3747 POSITIVELY.
- **CKPT-3 — record field-name preservation (B2 in-scope).** A new spell/reflect-only
  record field-name freeze fact under the CKPT-0 binding invariant (identity +
  member strings byte-identical). Records spell/reflect; the Record arm stops
  being a named rejection.
- **CKPT-4 — producer migration + dispositions.** Migrate classes A/B/D (thread
  overlay + field ASTs). Rule classes C/E + any residual as NAMED
  surface-and-stops at the emit builtin / consumer. After this, no live path
  except a ruled surface-and-stop reaches the `.source`/string arms.
- **CKPT-5 — pure total deletion (THE one pure-deletion commit).** Delete the
  `.source` schema field + producer emit/param, the `.source` reparse arm,
  `parse_type_annotation_payload`, the `__type_probe` reparse core, and the
  class-E bare-string type-payload arm. Rewrite the pins that flip. EXTEND
  `no_json_comptime_protocol.rs` with needles for `parse_type_annotation_payload`,
  `__type_probe`, and the `"source"` field literal → 0 across the tree.
  Net-negative production code.
- **CKPT-6 — gate re-baseline.** e1_s5 green, no_json green (extended),
  comptime + annotations_comptime envelope green, `just check-no-dynamic`,
  `just verify-merge`, `no_dynamic.rs` sentinel.

## CKPT-2 — LANDED (A8-OUT; report: `e5-ckpt2-report.md`)

DESCRIPTOR substitution shipped. `payload_of(applied builtin/enum head)` now
returns a COMPLETE, non-fabricating descriptor; `reflect(Array<int>)` /
`reflect(Option<int>)` / `reflect(Result<int,string>)` complete. Additive —
deletes nothing (the `.source` deletion stays CKPT-5). Base `e57a8acd` (CKPT-1).

### The A8-IN → A8-OUT ruling reversal (F1)

The B1 in-scope note above (and the design's original Piece-1 sketch) assumed
**A8-IN** — grow `NominalVariantDescriptor` with `payload_types` + a real
applied-enum substitution + a comptime-schema growth. The E5 CKPT-2 design pass
**refuted the necessity premise** and the supervisor ruled **F1 = A8 OUT
(arity-only)**:

- **A7-uniform recovery makes A8 redundant for soundness/completeness.** A7
  already routes container element/key/value recovery to the orthogonal
  `type_argument` query; the SAME route recovers enum payloads. So the
  arity-only descriptor + `type_argument` is a COMPLETE, SOUND answer with ZERO
  unspecced prerequisites — whereas A8-IN needs an enum generic-param-name freeze
  fact that is not established upstream.
- **A8-OUT plug-in (§1c A8-OUT):** Branch A (builtin head → static
  `builtin_nominal_templates`: 9 containers ⇒ `Opaque`, `Option`/`Result` ⇒
  arity-only `Enum`) + Branch B (user ENUM head → REUSE the param-AGNOSTIC base
  arity-only enum descriptor — SOUND because member ids + arities are `T`-free,
  so the base head descriptor IS the applied answer). No `payload_types` on the
  variant descriptor; no enum-freeze change; no comptime-schema growth.
- **SP-5 consequence (binding):** under A8-OUT `Result<int,string>` and
  `Result<string,int>` produce IDENTICAL descriptors — the swap is visible ONLY
  via `arg_identities` order. The soundness-critical enum-payload mis-type surface
  is therefore `type_argument`, NOT `payload_of`; the control pin
  (`e1_s5_ckpt2_sp5_result_swap_equal_descriptor_visible_only_via_arg_order`)
  asserts on arg ORDER, not the descriptor.
- **Deferred richer packaging → issue #87** ("Enum-variant self-description:
  payload TYPES in the variant descriptor + the enum generic-param-name freeze
  fact"). If #87 is ever taken up, the §1c coupling trap binds: DELETE Branch B's
  reuse-base shortcut (it would return the unsubstituted `[T]`) and do the real
  per-variant substitution.

### F2 — alias-of-applied IN (bounded)

`type Ints = Array<int>` / `type PageOfInt = Page<int>` now reflect (removes a
pre-existing alias-vs-direct asymmetry — the user pulled Piece 1 in SPECIFICALLY
to remove reflect asymmetry). Bounded mechanism, as specced: store
`base_applied_nominals: HashMap<FrozenTypeIdentity, RefinedApplication>` during
the alias fixpoint (keyed by the alias's transparent applied identity, threaded
write-once beside the composite descriptors); the base `Nominal` arm of
`payload_for_identity` calls `substituted_applied_nominal` before the pending
rejection — lazy, symmetric with the overlay memo arm. Stayed within the bounded
fixpoint + base-arm change (no CKPT-2b split needed).

### F3 — Phantom guard IN

`type Phantom<T>{tag:int}` (a generic head whose fields do NOT reference the
param) previously landed a `Struct{tag:int}` base descriptor and reflected/spelled
as if MONOMORPHIC, bypassing A3. Fixed at the rebuild: a struct that declares
NON-EMPTY generic parameters is excluded from `frozen_nominal_descriptors` (the
`is_empty` check is load-bearing — every struct gets a `struct_generic_param_kinds`
entry, empty for non-generic, so `contains_key` alone would wrongly exclude
monomorphic structs). Both `payload_of` and `bare_nominal_name_of` read that map,
so the fix covers reflect AND spell. The A3 pin
(`e1_s5_ckpt2_bare_generic_struct_head_stays_unapplied_rejection_incl_phantom`)
covers BOTH `Box<T>{value:T}` (param used) and `Phantom<T>{tag:int}` (param
unused) — never ships a pin that passes on the broken case.

### A1 / Forbidden-Patterns posture

No new `FrozenPayloadDescriptor::AppliedNominal` variant (A1 — one method-internal
branch + one additive identity-keyed static table + one additive F2 fact). NO
dynamic dispatch, NO tag decode, NO `.source` touch (grep-confirmed zero `.source`
changes), NO new sealed-enum variant. `builtin_nominal_templates` is STATIC
builtin data (not a per-type freeze fact — the binding record/freeze invariant is
untouched). The named `applied_nominal_pending_rejection` STAYS the LOUD fallback
for any head neither builtin nor struct/enum-resolved.

## CKPT-3 — LANDED (record field-name preservation; report: `e5-ckpt3-report.md`)

Records SPELL + REFLECT. B2 in-scope. **Additive — deletes nothing** (the
`.source` deletion stays CKPT-5). Base `1d54eb67` (CKPT-2).

### What landed (CKPT-0 mechanism, additive)

- **STORE + POPULATE.** `RecordFieldDescriptor` grows a `name: String`
  (payloads.rs), populated in `canonical_record` (type_reflection.rs) in the SAME
  rebuild that mints the record identity + the hygienic `member` identity. The
  identity descriptor string (`rendered`) and `record_member_identity` are
  **byte-untouched** — `name` is set on the descriptor ONLY, never threaded into
  either computation.
- **SPELL (load-bearing).** `reconstruct_type_annotation`'s Record arm
  (comptime_builtins.rs) flips from a NAMED Err to building
  `TypeAnnotation::Object({name: <recurse field type identity>}…)` from the
  preserved names, PRESERVING optionality (`{x?:int}` keeps the `?`). A2
  identity-indirected: each field type recurses on its own finite frozen
  `type_identity` (a nested record spells its own fields; an applied/bare arg
  spells by head/name — never eager-expands). This AUTO-WIDENS the shared
  stamp-gate for records (the same `reconstruct(...).is_ok()` predicate — no
  `stamp_for` edit): records now stamp + stop reparsing `.source`.
- **REFLECT ABI.** `COMPTIME_RECORD_FIELD_SCHEMA` (builtin_schemas.rs) +
  `record_field_slot` (payloads.rs) grow an additive `name` string field (Dec-55
  class, mirroring how `__ComptimeFieldDescriptor.name` surfaces a struct field
  name). The type-checker view of `RecordField` (`comptime.rs` `struct_item`,
  the "field names + order match the value carrier exactly" contract) grows the
  matching `name` field — so `reflect(...).fields[i].name` type-checks + reads on
  both engines.

### CKPT-0 SAFE realized — the binding invariant HELD (with PROOF)

The record IDENTITY + hygienic MEMBER identity strings stayed BYTE-IDENTICAL.
Proof is a permanent pin
(`e1_s5_reconstruction::e1_s5_ckpt3_record_identity_and_member_ids_are_byte_identical`)
that asserts the concrete pre-CKPT-3 128-bit values captured on HEAD `1d54eb67`:

- `{x:int,y:string}` identity `(high=4972967358956473603, low=-5404863359470070500)`;
  field `x` member `(5117747860848310177, 1031105497090630829)`; field `y` member
  `(-9035473693977959263, 304561787195158326)` — byte-sorted x before y.
- `{x?:int}` identity `(high=-1802259954908786269, low=-200733891727391745)`; field
  `x` member `(7472345934218968096, -929543014868829712)` — a DISTINCT identity
  from the required form (optionality-significant).

The pin passes at workspace HEAD → the field-name preservation added ZERO
information to the identity/member algebra. The existing
`record_identity_is_field_name_sorted_and_optionality_significant` descriptor-string
pin (`type_reflection/tests.rs`) is a second, independent guard on the identity
descriptor bytes.

### A1/A2 posture

A1 — no new sealed-enum variant; one additive field + one arm change. A2 — the
spelling recurses on field-type IDENTITIES (a recursive `type T = {kids:
Array<T>}` spells its field as `Array<T>` — head + bare `T` — and TERMINATES on
the finite type expression); pinned by
`e1_s5_ckpt3_record_with_nested_record_and_applied_field_terminates`. NO `.source`
touch (grep of the diff: only comments + the interim rejection message string that
FLIPS to spelling). The CKPT-1 interim Record rejection (A6) is REPLACED by the
spelling arm — the CKPT-3 deliverable itself, NOT a machinery deletion. Loud
failure is PRESERVED: a record that genuinely cannot spell can only be one whose
field type cannot reconstruct, and that surfaces through the recursive
`reconstruct_type_annotation(field.type_identity)?` propagating that field type's
OWN named rejection — never a silent gap.

## Anti-walk-back substrate (binding at MAXIMUM)

- The `.source` reparse is a Forbidden-Patterns **dynamic-reparse fallback**.
  CKPT-1..4 build TOWARD its CKPT-5 deletion; **never** toward a new one.
- CKPT-1 is ADDITIVE and touches NEITHER `.source`, the reparse arm,
  `parse_type_annotation_payload`, `__type_probe`, nor `stamp_for`. The new arm
  reads FROZEN identities and spells from them — no reparse, no fabricated
  identity.
- **Named rejections fail LOUD; a silent dynamic-reparse fallback is the worst
  state.** If an applied form genuinely cannot be spelled it is a NAMED
  rejection, NEVER a fallback.
- No new dynamic / reparse / string-source path may be introduced at any
  checkpoint. The `no_json_comptime_protocol.rs` sentinel (extended at CKPT-5) is
  the mechanical tripwire against a future "boundary reparse helper" walk-back —
  the exact Forbidden-Patterns failure this apparatus exists to stop.

## CKPT-4 — producer migration + irreducible-class rulings (ADDITIVE; deletes nothing)

CKPT-4 migrates the reconstructable producers to STAMP, rules the INVALID
`__ComptimeTypeRef` surface LOUD, and SURFACES the one irreducible producer the
design's 5-class inventory mis-dispositioned. It deletes nothing (the `.source`
field, the `.source` reparse arm, `parse_type_annotation_payload`, and
`__type_probe` all survive byte-identical — CKPT-5 is the pure deletion).

### The COMPLETE producer inventory (as verified at HEAD `f5b46958`)

`build_type_ref_descriptor` is the ONLY `__ComptimeTypeRef` constructor. Its live
(non-test) callers + every path feeding the consumer:

| # | Producer site (symbol) | Design class | Disposition realized |
|---|---|---|---|
| 1 | `to_nanboxed` param `type_ref` (`comptime_target.rs` param_objs) | (param) | STAMPS — already `Some(overlay)` from `functions_annotations.rs`; concrete param types reconstruct → stamp |
| 2 | `to_nanboxed` resolved-return `type_ref` | (return) | STAMPS — `stamp_for(overlay, return_type_ast)` |
| 3 | `to_nanboxed` return FALLBACK `build_type_ref_descriptor("unknown", Some("Unresolved"), None)` | **C** | INVALID/`kind:"Unresolved"` → the consumer's broad-INVALID guard rejects LOUD |
| 4 | `build_field_descriptor_array` field `type_ref` | **B** | MIGRATED — gate dropped; stamps the UNWRAPPED-inner AST for optional fields (`option_inner`), full AST otherwise |
| 5 | `ComptimeTarget::from_module` (via `to_nanboxed`), `statements.rs` module handler | **A** | MIGRATED — `module_target_fields` threads each member's declared-type AST; call site acquires the freeze BEFORE `to_nanboxed` and passes `Some(overlay)`. Typed members STAMP; synthetic members (functions/types/modules/annotations, `kind:"Unresolved"`) → LOUD |
| 6 | `for_expression` target (`expressions/mod.rs`) | **A** (expr) | no stampable fields; its sole `type_ref` is the class-C Unresolved return → LOUD. `None` overlay is correct (nothing to stamp) |
| 7 | `build_type_info_heap_value` field rows (`type_reflection.rs`) | **D** | MIGRATED — threads `Some(freeze)` + the struct field ASTs |
| 8 | `build_type_info_heap_value` top-level `type_ref` (new `build_named_type_ref_descriptor`) | **D** | MIGRATED — stamps `Basic(type_name)`; Unresolved names stay INVALID → LOUD |

**Consumer INVALID handling (subsumes class C + A5).** `type_annotation_from_
string_or_type_ref_slot`: any `__ComptimeTypeRef` with `identity == INVALID` is a
RULED named surface-and-stop (LOUD), citing `kind`/`name`. This fronts the
`.source` reparse arm, which is now UNREACHED (every `__ComptimeTypeRef` either
reconstructs off a stamped identity or rejects loud). Covers class C
(`kind:"Unresolved"`), scoped generic parameters, un-applied generic heads, and any
future residual — one predicate, not a per-kind table.

**Records STAMP (not a reject class).** CKPT-3 shipped record-IN spelling, so
record type-refs reconstruct + stamp like any other stampable type. Migrated in
classes A/B/D; NOT ruled loud.

**The separate `comptime:TypeRef` carrier is NOT a `.source`-family producer.**
`build_frozen_type_ref_heap_value` (`type_reflection.rs`) builds the
`COMPTIME_FROZEN_TYPE_REF_SCHEMA` (`"\u{1}comptime:TypeRef"`) carrier —
identity-only (no `name`/`kind`/`source` fields), its own reader
(`frozen_identity_from_ref`), and it ALWAYS carries a valid frozen identity or
rejects loud (`category_of(identity)?`). It never reaches the `.source`/string
arms (the consumer rejects a non-`__ComptimeTypeRef` schema). NOT a 6th class.

### class-E blocker (SURFACED — the completeness finding)

The design's 5-class inventory (§2) dispositioned **class E** — the bare-string
type-payload arm (`type_annotation_from_string_or_type_ref_slot`, `slot.as_str()
→ parse_type_annotation_payload`) — as a NAMED SURFACE-AND-STOP ("reject the
runtime-string carrier loud; Int64-index + TypeRef are the two sanctioned
carriers"). **This disposition is BLOCKED and was NOT applied.**

The bare-string arm is a **SANCTIONED, documented carrier** for the item-generation
builtins: `item_fn(name, return_type: string | TypeRef, value)` — the `string`
half is contract (`comptime_builtins.rs`, item_fn registration comment) — plus
`extend_method` / `build_extend_item_with_method_body`. These have **no sanctioned
Int64/TypeRef alternative today**, and a string TYPE SPELLING inherently requires
`parse_type_annotation_payload` (there is no non-parse path from `"Array<int>"` to
a `TypeAnnotation`). The design's own §2 blast-radius even lists `item_fn` as one
of the three emit sites for class E — but it did not verify item_fn had a migration
path off strings. It does not.

Applying the class-E reject BROKE **~19 tests** (measured): `item_fn` /
`extend_method_producer_tests` (6) / `functions_annotations` generated-install +
expansion-provenance + source-anchor (8) / `annotation_import_pipeline` (2) /
`extension_integration` (3) / `e2_d9_closure_free_tripwire` (1) — all the
item-generation surface. Per the standing ruling *"if migrating a class needs more
than threading overlay+ASTs → SURFACE it, don't force it,"* class E was REVERTED
to its reparse and surfaced here.

**Consequence for CKPT-5.** The `.source` FIELD + the `.source` reparse ARM are now
UNREACHED (fronted by the broad-INVALID ruled reject) → CKPT-5 CAN delete them. But
`parse_type_annotation_payload` + `__type_probe` retain a LIVE caller (the item_fn/
extend string arm) → CKPT-5 CANNOT delete the reparse machinery until item_fn/
extend migrate to a sanctioned type carrier. **DECISION REQUIRED** (not guessed):
(a) give item_fn/extend an Int64 literal-type or `type_ref` carrier + migrate their
call sites (user-facing API change), OR (b) declare the item_fn/extend string a
permanently-sanctioned carrier (then the string arm survives E5 by design — the
`.source` deletion still lands, but the reparse fn does not).

### Exit-criterion status

- **`.source`/`__ComptimeTypeRef`-identity surface: MET.** Every `__ComptimeTypeRef`
  reaching the consumer STAMPS (concrete → identity route) or is a RULED LOUD
  surface-and-stop (INVALID). The `.source` reparse arm is UNREACHED. Pinned by
  `e1_s5_ckpt4_typeref_producers_stamp_invalid_rejects_loud_string_arm_surfaced`
  (producers stamp end-to-end via real `to_nanboxed`; class-C/INVALID reject loud)
  + `e1_s5_ckpt4_unstamped_typeref_is_named_surface_and_stop_not_source_reparse`
  (the rewritten former "falls_through_to_source_arm" pin — an unstamped ref is now
  LOUD, never a `.source` reparse, even with a valid parseable source).
- **bare-STRING arm: BLOCKED/SURFACED** (item_fn/extend, above). The exit pin
  documents it as a live residual (asserts the string carrier STILL reparses).

### Recursive-record termination (deferred CKPT-3 pin, folded in)

`e1_s5_ckpt4_recursive_named_record_reconstructs_and_terminates`: a recursive NAMED
record `type Tree { kids: Array<Tree> }` reconstructs + TERMINATES — the nominal
self-ref `Tree` resolves to the bare-name leaf `Basic("Tree")` (via
`bare_nominal_name_of`), never field-expanding; so `Array<Tree>` spells head +
bare-name arg and stops (A2 identity-indirected recursion on a nominal self-ref,
distinct from the anonymous-record nesting pin).

### Gate (FAILED-name sets vs Step-0 baseline — ZERO regressions)

| Gate | Baseline (`f5b46958`) | Post-CKPT-4 | Verdict |
|---|---|---|---|
| `shape-vm --lib` | 3587 pass / 7 fail | 3589 pass / 7 fail (+2 new pins) | same 7 pre-existing; ZERO new |
| `e1_s5` filter | 29 / 0 | 31 / 0 (+2 pins) | green |
| `no_json` | 2 / 0 | 2 / 0 | green |
| `comptime` | 271 / 3 | 271 / 3 | exact 3 pre-existing; ZERO flips |
| `annotations_comptime` | 117 / 10 | 117 / 10 | exact 10 pre-existing; ZERO flips |
| `just check-clean` | exit 0 | exit 0 | green (1 pre-existing warning) |
| `just check-no-dynamic` | success | success | green |

Pre-existing `shape-vm --lib` fails (all comptime-unrelated): `test_async_let_
binding_is_immutable`, `test_match_arm_empty_array_unprovable_element_is_clean_
compile_error`, `monomorphization::cache::route_tests::{inlined_closure_keeps_
outer_authored_type_ref, nested_exact_calls_close_outer_arguments, unavailable_
and_missing_callsite_evidence}`, `monomorphization::type_resolution::tests::{ws6_
generic_id_ok_arg, ws6b_inferred_result_variable_arg}`. No TP rebaselines were
needed (no producer flipped a test: the concrete-typed emit tests still stamp →
resolve identically; the class-E reject that WOULD have flipped ~19 tests was
reverted and surfaced instead).

`.source`/reparse machinery UNTOUCHED (grep-confirmed byte-identical): schema
`.string_field("source")` (builtin_schemas.rs), `parse_type_annotation_payload`
+ the `fn __type_probe` snippet + the `.source` arm (`string_field_from_typed_
object(storage, &schema, "source")` → `parse_type_annotation_payload(&source)`).

---

## CKPT-5 (+ E5 CLOSE) — the `.source` reparse-fallback DELETION

**This is THE deletion the CLAUDE.md Forbidden-Patterns apparatus exists to
protect: a DYNAMIC-REPARSE FALLBACK removed TOTALLY — no shim, no rename.**
CKPT-1/2/3/4 made every reconstructable type STAMP and every producer
stamp-or-reject-LOUD, so the `.source` reparse arm was already runtime-UNREACHED
(retained byte-for-byte as the E1-D8 residual). CKPT-5 deletes it.

### The precise deletion (user ruling #61, 2026-07-24 — Option 1)

Three targets DELETED (grep-proven 0 hits across `crates/ bin/ tools/ extensions/`,
sentinel-fragment assemblies excepted):

1. **the `.source` SCHEMA FIELD** — `builtin_schemas.rs` `__ComptimeTypeRef`
   `.string_field("source")` (between `kind` and `identity_high`). Field offsets
   are now `name=0, kind=1, identity_high=2, identity_low=3`; every reader is
   name-keyed (`schema.get_field(name)`) so no reader shifted.
2. **the PRODUCER EMIT** — `comptime_target.rs` `build_type_ref_descriptor`'s
   `("source", nb_string(source.to_string()))` pair. The `source: &str` param is
   **renamed `spelling: &str`** and PRESERVED — it is load-bearing for the
   surviving `name`/`kind` reflect-only fields (`type_ref_name_from_source` /
   `type_ref_kind_from_source`), which the U02 corpus reads (serde `derive.shape`:
   `field.type_ref.kind`) and the consumer's INVALID rejection reads. Dropping it
   entirely (as the task text's literal wording suggested) was mechanically
   IMPOSSIBLE without also deleting `name`/`kind`, which are out of CKPT-5 scope
   and user-facing. It is NOT a renamed `.source` fallback (no field is stored for
   reparse); the rename kills the walk-back-attractor name. **Surfaced deviation
   from the literal "drop the param" wording — see the CKPT-5 report.**
3. **the `.source` REPARSE ARM** — `comptime_builtins.rs`
   `type_annotation_from_string_or_type_ref_slot` tail: the
   `let source = string_field_from_typed_object(storage, &schema, "source")?;
   parse_type_annotation_payload(&source)` pair. The preceding `if identity !=
   INVALID { return reconstruct… }` guard collapses to an unconditional
   `reconstruct_type_annotation(overlay, identity)` (identity is proven != INVALID
   by the INVALID arm's early return above it). The identity short-circuit — the
   stamped route — is KEPT.

### PRESERVED (sanctioned, #88 — over-deletion would be a REVIEW FAIL)

`parse_type_annotation_payload`, `fn __type_probe`, and the bare-string
type-payload arm (`comptime_builtins.rs`, the `item_fn` / `extend_method` caller).
These are the SANCTIONED item-generation carrier (`item_fn(name, return_type:
string | TypeRef, value)` — the `string` half is contract; there is no non-parse
path from `"Array<int>"` to an AST). They are NOT the fallback. **Two-sided
precision requirement met: the `.source` fallback DELETED AND the item_fn parser
PRESERVED.** Over-deletion tripwire lives in pin (g)
(`e1_s5_ckpt4_typeref_producers_stamp_invalid_rejects_loud_string_arm_surfaced`),
now asserting the bare-string arm parses BOTH `"int"` (leaf) AND `"Array<int>"`
(applied generic).

### Walk-back now STRUCTURALLY IMPOSSIBLE (anti-walk-back)

- The `.source`-built route-proof pins were REWRITTEN, not left referencing a
  deleted field. The primary anti-walk-back pin
  `e1_s5_stamped_unresolvable_ref_errs_through_full_consumer_never_reparses_valid_source`
  STRENGTHENED: its trap ("a stamped-unresolvable ref silently reparses its valid
  `.source = "int"`") is now impossible — no `.source` field exists to read, no arm
  exists to reparse from. The `"int"` first-arg now feeds only `name`/`kind`; the
  pin proves the stamped-unresolvable identity Errs through the FULL consumer via
  the identity route, guarding against RE-INTRODUCTION of the deleted arm. Pins
  (a)/(b)/(d)/(f) similarly reframed ("###unparseable###"/"string" first args are
  now garbage spellings feeding only name/kind; the identity route is the SOLE
  route). Pin (c)
  (`e1_s5_stamped_unresolvable_identity_is_named_semantic_error_no_fallback`)
  survives unchanged (no `.source` fixture — the invariant that holds before AND
  after).
- **A `.source`-FIXTURE in a shape-test corpus was found + rewritten:**
  `tools/shape-test/tests/annotations_comptime/type_mutation.rs::target_params_and_
  return_expose_type_refs` read `target.return_type_ref.source` from USER Shape
  code (embedded in a Rust raw string, missed by an initial `.shape`-only grep).
  Rewritten to read `target.return_type_ref.kind == "String"` (mirroring the param
  assertion directly above it). This was the sole live `.source` READER outside the
  deleted arm; all other repo `.source` hits are the unrelated `FromQuery.source`
  query-DSL field, `metadata.source_hash`, or historical doc/comment narrative.

### Sentinel extended (`no_json_comptime_protocol.rs`)

Header note (c) FLIPPED — the old "`__type_probe` source-reparse remainder SURVIVES
(E1-D8 residual)" is now the CKPT-5 deletion note. Two NEW needles (assembled from
fragments so the sentinel never spells them contiguously; **precise — they do NOT
ban the preserved `parse_type_annotation_payload`/`__type_probe` item_fn parser**):

- `no_source_field_on_comptime_type_ref_schema` — forbids re-intro of the
  `.string_field("source")` schema field → 0 across `crates/ bin/ tools/
  extensions/`. Structural guard: no field declared ⇒ no name-keyed reader can
  resolve a `.source` field.
- `no_reparse_from_type_ref_source_field` — forbids re-intro of the
  `&schema, "source")` field-read arm shape → 0.

Docstrings corrected (the CKPT-4 MEDIUM finding + collateral): `comptime_target.rs`
`build_type_ref_descriptor` + `stamp_for` + `build_field_descriptor_array` +
`from_module` + `to_nanboxed` no longer assert a LIVE `.source` fall-through;
`comptime_builtins.rs` consumer comments + `reconstruct_type_annotation` docstring +
the `Parameter` variant's error MESSAGE (was "an unstamped ref reparses .source",
now "no `.source` reparse — the fallback is deleted") + the STAGE-2/STAGE-1
historical narrative headers + `type_reflection.rs` reflection comment +
`e5_spelling.rs` module docstring all reflect the deletion.

### Gate (FAILED-name sets vs Step-0 baseline `f5c51332` — ZERO real regressions)

| Gate | Step-0 baseline | Post-CKPT-5 | Verdict |
|---|---|---|---|
| `shape-vm --lib` (parallel) | 3590 pass / 6 fail | 3591 pass / 7 fail | +2 pass sentinel tests; the +1 fail is `route_tests::nested_exact_calls` — a documented pre-existing FLAP (see below); ZERO new in comptime blast radius |
| `shape-vm --lib` (`--threads=1`) | — | run A: 3592/6 · run B: 3591/7 | same binary, two serial runs → `nested_exact_calls` flaps 6↔7: it is NON-DETERMINISTIC, not a regression |
| `e1_s5` filter | 31 / 0 | 31 / 0 | green (all rewritten pins pass) |
| `no_json` | 2 / 0 | 4 / 0 (+2 needles) | green |
| `comptime` | 271 / 3 | 271 / 3 | exact 3 pre-existing named; ZERO flips |
| `annotations_comptime` | 117 / 10 | 117 / 10 (after type_mutation fix) | exact 10 pre-existing; ZERO flips |
| `just check-clean` | exit 0 | exit 0 | green (pre-existing warnings only) |
| `just check-no-dynamic` | success | success | green |
| `just verify-merge` | — | 15 pass / 0 fail | ALL CHECKS PASSED |

**`nested_exact_calls_close_outer_arguments_before_inner_compilation` is a
KNOWN pre-existing flaky `monomorphization::cache::route_tests` — the CKPT-4
decisions gate table above ALREADY lists it among the pre-existing comptime-
unrelated `shape-vm --lib` fails.** It is non-hermetic (fails ALONE with a
cache-empty 0-passed run) and its pass/fail flaps run-to-run even at
`--test-threads=1` (proven: two serial runs of the SAME post-CKPT-5 binary gave 6
then 7 failures). My only new tests are the 2 sentinel needle tests, which are
pure `.rs`-file scanners (compile/execute NO Shape code) and therefore cannot
behaviorally affect the monomorphization cache — they only shift the harness's
parallel test-scheduling, reshuffling the non-hermetic route_tests. The `.source`
deletion touches the comptime `__ComptimeTypeRef` carrier, orthogonal to
`int`/`string` monomorphization evidence. Comptime blast radius (e1_s5, no_json,
comptime, annotations_comptime) is 100% green.

### E5 CLOSE

All type reconstruction landed (CKPT-1 applied-generic + bare-nominal spelling,
CKPT-2 applied-generic descriptor substitution, CKPT-3 record field-name +
spelling, CKPT-4 A/B/D producer migration + INVALID-LOUD ruling), and CKPT-5
DELETED the `.source` reparse fallback totally. The stamped identity route is the
SOLE resolution path for a `__ComptimeTypeRef`; an unstamped/unresolvable ref is a
NAMED surface-and-stop; the stamped->reparse walk-back is structurally impossible.

**Residuals (out of E5, tracked):**
- **#87** — (as filed; unchanged by CKPT-5).
- **#88** — item_fn/extend_method typed-carrier migration. Until it lands, the
  bare-string type-payload arm + `parse_type_annotation_payload`/`__type_probe`
  remain the sanctioned carrier (PRESERVED here, pinned by the pin-(g) tripwire).

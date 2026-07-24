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
  Opaque-for-containers + Enum-for-Option/Result; enrich the enum-payload-type
  freeze projection. (CKPT-2.)
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

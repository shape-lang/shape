# The public feature manifest contract (#112, PF-CONTRACT)

**Authority:** ADR-016 §2 (Shape owns the `PublicFeatureManifest`), §3 (no
manifest-local revisions), §9 (coverage changes are reviewable semantic
changes), §10 (script-tier dimension and ceremony budgets, and the standing
permission to revise these draft contracts before the first accepted pair);
ruling R19.
**Artifacts:** `public-feature-manifest.schema.json`,
`public-feature-manifest.json`,
`scripts/check-adr011-012-public-feature-manifest.mjs`,
`scripts/lib/adr011-012-json-schema.mjs`.
**Gates:** `just check-public-features`; `ci.yml` job `legacy-authority`;
`scripts/verify-merge.sh` CHECK 17.
**Recorded at:** wave2-spine, on top of `fa3f9a84`.

ADR-016 fixes what the manifest must mean. This document records what #112 had
to decide to make it mechanical, and why the committed schema needed amending.

## 1. What the gate is actually defending

Every rule below exists to stop one failure: making a coverage gate green by
editing the denominator. ADR-016's rejected-alternatives list names the shapes
this takes — shrinking the inventory, reclassifying a hard example, demoting a
feature that broke. The gate makes each of them a build failure rather than a
reviewer's judgement call.

| Rule | The move it blocks |
|---|---|
| Status is derived from `status_basis`, never asserted beside it | Declaring a maturity the evidence does not support |
| Status is forward-only, `public -> planned` named explicitly | Relabelling a broken feature `planned` to shed its runnable-evidence obligation |
| Every published `feature_id` stays in the inventory forever | Deleting a row so the coverage denominator falls |
| A `removed` row is frozen verbatim, and a live row's `family` is immutable | Reattaching a retired identity, or repurposing a live one, to a new meaning |
| No revision, counterpart SHA, attestation or verification state | Making the source manifest and the external pair evidence mutually self-referential (§3, §7) |

The last rule is enforced against the **schema** as well as the manifest. A
future schema edit that *declares* an `attestation_digest` property fails the
gate before any manifest can carry one.

## 2. Status is a function, not a field

ADR-016 §2 gives each status a distinct evidence contract, so status is
recoverable from the basis:

```text
never-current                          -> planned
current-executable, no declared limits -> public
current-executable, declared limits    -> experimental
deprecation-transition                 -> deprecated
removal-transition                     -> removed
```

`declared_limits` is what separates the two `current-executable` statuses, and
it is a **schema addition** (§4.1 below). Without it the committed schema mapped
`current-executable` to both `experimental` and `public`, so status was not
derivable and the acceptance criterion "its status is derivable from the
evidence fields alone" could not be met.

The validator proves derivability by substitution: for the load-bearing row,
each of the four statuses it does not hold is injected and must be rejected. All
four are, each naming the specific evidence field that contradicts it.

## 3. Identity permanence is structural, not a lookup

A published `feature_id` is never removed from `features`. A retired feature
becomes a `removed` row carrying a tombstone; a renamed one leaves its old
identifier behind as a tombstone naming the replacement. Because rows only ever
accumulate, "was this ID ever used?" is answered by the current manifest alone —
there is no historical ledger to consult and therefore none to fall out of sync.
That is what makes "IDs are never reused" mechanically checkable rather than a
convention.

Reuse has two shapes, and both are rejected. A **retired** identity is reused by
editing its tombstone row, so a `removed` row is frozen verbatim. A **live**
identity is reused by keeping the ID and changing what it describes; the
mechanical half of that is `family`, a structural classification which is
therefore immutable — a feature that genuinely moves family takes a new identity
and leaves a tombstone. `public_name` is deliberately left free: it is prose, a
typo fix is not a repurposing, and ADR-016 §9 makes the manifest diff the review
surface for prose.

The schema-major migration map adds explicitness at a major boundary: it must be
**total** over the previous manifest's ID set, and each disposition must agree
with the row it describes (a `replaced` ID must actually be a tombstone naming
exactly the new IDs, and those new IDs must not have existed before).

## 4. Schema deltas against the committed baseline

All four are prepublication revisions of a draft contract, which ADR-016 §10
explicitly assigns to this ticket: "Because no accepted manifest pair exists
yet, this is a prepublication revision of the draft contracts owned by
`PF-CONTRACT` and `BOOK-CONTRACT`; it triggers no schema-major identity
migration."

**4.1 `statusBasis.declared_limits` added.** Required for `experimental`,
forbidden for `public`. Reason: §2 above — without it status is not derivable.
It also gives §2's "structured negative evidence for each declared limit" a row
to attach to, instead of one prose sentence.

**4.2 `identityMigration` gains a `revision` branch.** The committed schema
offered only `initial` and `schema-major`. An ordinary revision after the first
accepted manifest therefore had no way to name its predecessor's content
identity, and would have had to claim `initial` (false) or fabricate a
schema-major change. §2 requires the comparison against "the previous accepted
manifest", so the predecessor identity must be expressible at every revision.

**4.3 `manifest_version` relaxed from `const: 1` to `minimum: 1`.** With a
`const`, `previous_manifest.schema_major` (`minimum: 1`) could never be lower
than the current version, making the `schema-major` branch unreachable and its
totality rule unprovable. The expected major is pinned in the validator instead,
matching the house idiom where `check-adr011-012-program-manifest.mjs` pins
counts in JavaScript and cross-checks the schema.

**4.4 Status conditionals tightened.** Four of the five `if` blocks tested
`status` without `required: ["status"]`, so a row lacking `status` satisfied
them vacuously. Latent only because `status` is required elsewhere; a schema
should not depend on a distant keyword for its soundness. The per-status
evidence-class rules were also split: `public` and `deprecated` need runnable
evidence in every required mode, but what "runnable" means depends on the mode,
so that rule moved to the validator (§5).

The prior-identity field `previous_manifest.sha256` is a **manifest** content
identity, not a source revision, and is the single exemption in the
forbidden-field scan. It is declared in exactly one place
(`$defs/priorManifestIdentity`) so the exemption cannot be copied elsewhere.

## 5. Rules the validator owns because JSON Schema cannot express them

- Status derivation, and every inter-manifest rule (forward-only, identity
  permanence, migration totality, prior-identity agreement).
- Content identity: canonical JSON with sorted keys, excluding `$schema` and the
  migration record that names the *previous* manifest. Canonical rather than
  file-bytes so reformatting does not read as a different predecessor.
  `--print-identity` emits it for the next revision to record.
- Runnable-evidence coupling. A mode that executes code owes
  `positive-execution`; a compile- or LSP-only feature owes the structured
  rejection or projection that *is* its observable behaviour. Requiring
  `positive-execution` universally would have been wrong for diagnostic
  features — this rule caught an inconsistency in the gate's own fixtures.
- Mode/evidence agreement: `vm` + `jit` requires `vm-jit-parity` (§5 makes
  parity two real executions); `native-execution` requires a `jit` mode;
  `script-tier` requires `ceremony-budget` (§10).
- Identity permanence in both directions: a `removed` row is frozen verbatim, and
  a live row's `family` cannot change (§3).
- Tombstone integrity: replacements resolve, no self-replacement, no cycles.
- `public_name` uniqueness — two rows sharing one public name is an ambiguous
  inventory.

## 6. The load-bearing row

`language.pipe-operator`, status **public**.

Chosen because its evidence is unambiguous in the sense ADR-016 §2 demands, and
because it exercises the whole obligation surface rather than a corner of it: it
has a grammar rule, a bytecode-compiler meaning, a separate MIR lowering that
the JIT consumes, a type-inference rule, and LSP behaviour.

Status is `public` and not something more cautious because §2 makes bootstrap
status evidence-derived, and forbids demoting a currently-current feature: "A
previously current or currently broken feature cannot be relabeled `planned` to
escape its runnable evidence." The feature is shipped, documented and current,
so `planned` is unavailable; it declares no limits, so `experimental` is
unavailable. `public` is what the evidence derives.

Verified rather than cited: both VM tests named in `status_basis.evidence` were
executed at wave2-spine and pass —
`pipe_chain_preserves_float64_implicit_generic_callsite` (executes
`5.0 |> double |> add_one`, asserts 11.0) and
`pipe_call_does_not_default_unproven_numeric_specialization` (asserts
`"oops" |> add_one` is a compile error, not a coerced call).

`jit` is a declared required mode. The MIR path (`lower_pipe_expr`) is real, so
the obligation is honest; if the Book gate later finds VM/JIT divergence, that is
a defect to repair, and §2 keeps the row `public` while it is repaired rather
than letting it be demoted.

Two dimensions were deliberately **not** declared. `lsp-diagnostics`: the LSP
offers pipe-aware completion and type tracking, not a pipe-specific diagnostic,
and declaring the dimension would create an obligation with no content.
`script-tier`: §10 requires a mechanically reported ceremony count from the
compiler, which does not exist yet (ADR-017 territory) — declaring it would
create an unverifiable obligation.

## 7. Deliberately open

- **The inventory is one row.** ADR-016 §2 calls the manifest a complete
  inventory, and it is not one yet. §3 and R19 anticipate this: complete
  inventories expand in bounded waves from committed exact rows, and the
  inventory ticket owns completeness. #112 lands the contract and one truthful
  row; nothing here claims the inventory is finished, and no field exists in
  which to claim it (a completeness flag would be exactly the mutable
  verification state §3 forbids).
- **`--previous` is unused today.** The manifest is `initial`, so there is no
  predecessor. Once `BOOK-PAIR-PROMOTE` establishes the first accepted pair, the
  gate invocation gains `--previous <accepted manifest>` and the migration record
  stops being `initial`. The rules are implemented and tripwired now so that
  turning them on is a flag, not a design.
- **Where the accepted manifest will live** is `PAIR-PROTOCOL`'s decision, not
  this ticket's: ADR-016 §7 puts it on a protected coordination ref outside both
  source revisions.

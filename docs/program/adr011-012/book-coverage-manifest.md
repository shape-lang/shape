# The Book coverage manifest contract (#113, BOOK-CONTRACT)

**Authority:** ADR-016 §3 (shape-web owns the `BookCoverageManifest`; no
manifest-local revisions; identities carry no line number or ordinal; totality
of the schema-major migration map), §4 (the distributed matrix), §5 (explicit
fence classification, the ratcheted illustrative set, parity as two real
executions), §6 (`BookTruthGate` is the bidirectional gate), §7 (exact pairs are
promoted by external attestation), §9 (coverage changes are reviewable), §10
(script tier, ceremony budgets, concept identities, and the standing permission
to revise these draft contracts before the first accepted pair); ruling R19.
**Artifacts:** `book-coverage-manifest.schema.json`,
`book-coverage-manifest.json`,
`scripts/check-adr011-012-book-coverage-manifest.mjs`,
`scripts/lib/adr011-012-json-schema.mjs`.
**Gates:** `just check-book-coverage`; `just check-book-coverage-gaps`;
`ci.yml` job `legacy-authority`; `scripts/verify-merge.sh` CHECK 18.
**Recorded at:** wave3-spine, on top of `d0939e2b`.

#112 landed the half of the pair that says what must be explained. This document
records what #113 had to decide to make the half that says where the Book
explains it mechanical.

## 1. What the gate is actually defending

#112's gate stops a coverage gate being made green by shrinking the feature
inventory. The coverage manifest is where the same move reappears wearing
different clothes, so each rule below blocks one of them.

| Rule | The move it blocks |
|---|---|
| Classification is derived from the fields a fence carries, never asserted beside them | Relabelling a hard example `illustrative-only` so it stops being executed |
| A fence that was `runnable-gated` may never become `illustrative-only` | The same move spread across two commits |
| Section and fence identities reject line numbers and ordinal positions | Editing a page so that moving prose reads as removing and adding a feature |
| Every published identity stays live or tombstoned, and a tombstone is frozen | Deleting coverage so the covered-dimension count stops falling short |
| A fence identity is declared exactly once; a shared section must agree everywhere | Pointing one identity at two different places, or repointing it at a third |
| The three enums shared with the `PublicFeatureManifest` schema must be identical | An obligation one manifest declares and the other cannot express |
| No revision, counterpart SHA, attestation or verification state | Making the two source manifests and the external pair evidence mutually self-referential (§3, §7) |

The last rule is enforced against the **schema** as well as the manifest, exactly
as in #112: a future schema edit that *declares* an `attestation_digest`
property fails the gate before any manifest can carry one.

## 2. Classification is a function, not a field

ADR-016 §5 gives each classification a distinct obligation, so classification is
recoverable from the fields present:

```text
evidence_role + declared_modes + expectation, no illustrative -> runnable-gated
illustrative alone                                            -> illustrative-only
anything else                                                 -> no derivation, rejected
```

This is the direct analogue of #112's `status_basis`. It matters for the same
reason: without it, "this fence is illustrative" is a label a reviewer must take
on trust, and ADR-016's own rejected-alternatives list names
`runnable=false` without a reason as a way to hide a broken implementation.

The validator proves derivability by substitution in both directions (T4a, T4b):
a `runnable-gated` fence relabelled `illustrative-only` is rejected, and an
`illustrative-only` fence relabelled `runnable-gated` is rejected, each naming
the field that contradicts the claim.

Derivation alone is not enough, because a fence can be reclassified by editing
its fields and its label together. T4c covers that: against the previous accepted
manifest, `runnable-gated -> illustrative-only` is rejected however the fields
were rewritten. §5 ratchets the illustrative set, and this is the ratchet.

## 3. Identity permanence, and the two shapes of reuse

As in #112, identities only accumulate, so "was this identity ever used?" is
answered by the current manifest alone and there is no historical ledger to fall
out of sync. A published section or fence identity is either live or carries a
tombstone with a reason and a citation.

Reuse takes two shapes on this side, and both are rejected:

- a **retired** identity is reused by editing its tombstone, so tombstone rows
  are frozen verbatim (T2a);
- a **live** identity is reused by keeping the identifier and pointing it at
  different material, so a live section identity whose `page` or `anchor`
  changes is rejected (T2b). `title` is deliberately left free — it is prose,
  and §9 makes the manifest diff the review surface for prose.

Two further rules come from the fact that these identities name physical things.
A fence is one code block, so its identity is declared exactly once across the
whole manifest (T2c); a section can legitimately serve several features (§4:
"One section may satisfy several rows"), so it may be declared more than once
provided every declaration is identical.

## 4. Where the artifacts live, and why

ADR-016 §3 opens "The shape-web repository owns one canonical
`BookCoverageManifest`", so the natural reading is that everything here belongs
in shape-web. The decision is to land the schema, the validator and the
manifest in **this** repository, and it rests on four things.

**The canonical manifest does not exist yet.** §7 puts acceptance authority on a
protected coordination ref outside both source revisions, and §10 states plainly
that "no accepted manifest pair exists yet", calling both files "the draft
contracts owned by `PF-CONTRACT` and `BOOK-CONTRACT`". What §3 assigns to
shape-web is ownership of the accepted inventory. What #113 is asked to land is
the contract, and §10 assigns that to this ticket by name.

**A contract that cannot be gated is not a contract.** shape-web is not a gate
surface for this work: it does not run the house JSON-Schema library, and at the
time of writing its working tree carries another lane's 56 uncommitted files.
Landing the contract where nothing executes it would reproduce the failure R19
names in its closing paragraph — a harness that exists on a branch is versioned
evidence, not authority.

**The join needs both manifests.** The rules of §3 that matter most — every
non-removed feature covered, every coverage entry naming a real feature, every
required dimension owned — are joins against the `PublicFeatureManifest`. Its
canonical copy is here. A validator on the other side of the boundary could
check only the half it can see.

**Co-location makes enum agreement mechanical.** The `mode`, `evidenceClass` and
`semanticDimension` enums are one vocabulary split across two schemas: the public
manifest declares obligations in it and the coverage manifest discharges them in
it. A member in one and not the other is an obligation that can be declared and
never satisfied, or satisfied and never declared — a silent divergence, not a
loud one. With both schemas in one tree the validator asserts they are identical
(T7); across repositories that check is not expressible, and would have to become
a convention. This is the strongest of the four reasons, because enum drift is
precisely the kind of quiet parallel divergence this program keeps having to
undo.

Two alternatives were rejected. **Vendoring a copy of the schema into shape-web**
is parallel implementation across a producer/consumer boundary, which this
codebase refuses on sight; there is one schema, and shape-web's gate resolves it
from the paired Shape revision that §7 already requires it to have checked out.
**Splitting the schema from the manifest across the two repositories** breaks the
local `$schema` reference and puts the contract and the data it constrains under
separate review.

Nothing in this ticket therefore needs to land in shape-web, and no patch file
is delivered. The migration obligation is recorded here rather than left
implicit: when `BOOK-PAIR-PROMOTE` (#118) establishes the first accepted pair,
`book-coverage-manifest.json` moves to shape-web at
`book/book-site/book-coverage-manifest.json`. The move is a file move plus two
string changes — the manifest's `$schema` becomes the schema's `$id` URL, and the
validator's default `--manifest` path follows it. The schema and validator stay
here.

### 4.1 The prior-session drafts are adopted, not superseded

Two untracked drafts sit in shape-web at
`book/book-site/book-coverage-manifest.{schema,example}.json`. The schema draft
is adopted as the base of the committed schema, and its two best ideas are kept
unchanged: the `coverageId` definition that mechanically rejects line-number and
ordinal spellings, and the three-domain tombstone table. Seven changes were made.

**4.1.1 `manifest_version` relaxed from `const: 1` to `minimum: 1`.** #112 §4.3's
finding, and it applies identically: with a `const`, `previous_manifest.schema_major`
(`minimum: 1`) can never be lower than the only permitted current value, so the
`schema-major` migration branch is unreachable and its totality rule is
unprovable. The expected major is pinned in the validator instead.

**4.1.2 `identity_migration` gains a `revision` branch.** The draft offered only
`initial` and `schema-major`, so an ordinary revision after the first accepted
manifest had no way to name its predecessor's content identity and would have had
to claim `initial` or fabricate a schema-major change. #112 §4.2's finding.

**4.1.3 `priorManifestIdentity` extracted to a named `$def`.** The draft declared
the `previous_manifest` object inline. The forbidden-field scan exempts exactly
one path, so the exempted shape must exist in exactly one place — otherwise the
exemption is copyable.

**4.1.4 Status conditionals given explicit `required`.** #112 §4.4's finding,
applied to the fence conditionals: an `if` that tests a property without
requiring it is satisfied vacuously by an instance lacking that property.

**4.1.5 ADR-016 §10 added.** The draft predates the amendment. It gains fence
`ceremony`, fence `budget`, the top-level ratcheted `flagship_fence_ids` set,
per-feature `concept_identity_coverage`, a `concept_id` on structured-diagnostic
expectations, and a fourth tombstone domain for concept identities. The shared
enums gain `ceremony-budget` and `script-tier` to match the public schema.

**4.1.6 `native` evidence role bound to its expectation.** ADR-016 §5 says a
fence claiming native execution must name the exact function or realization, so
`evidence_role: native` now requires the `native-execution` expectation and a
`jit` declared mode. In the draft the two were independent, and a `native` role
could be declared with an `exit-success` expectation that proves nothing.

**4.1.7 `ceremony` admits only `none`.** §10 gives meaning to declaring zero
ceremony and defines a mechanical check for it. There is no defined check for
"some", so an enum with other members would be a field the gate cannot verify.
The absence of the key is the absence of a claim.

The **example** draft is superseded rather than adopted. Every identifier in it
is fictional: it covers `annotations.checked-prompt` and
`remote.execution-certainty`, which are not in the `PublicFeatureManifest`, and
cites `advanced/comptime-llm-patterns.mdx`, `stdlib/core/remote.mdx`,
`advanced/wire-protocol.mdx` and `tooling/execution-server.mdx`, none of which
exist in the Book at shape-web `627459a`. Under the join of §5 it would fail on
every row. #112 committed one verified row rather than an illustrative file, and
this ticket does the same.

## 5. Rules the validator owns because JSON Schema cannot express them

- Classification derivation, and the runnable-to-illustrative ratchet.
- Identity permanence in both directions: a live identity that is also
  tombstoned, a tombstone that changed, a live identity that disappeared.
- Uniqueness and agreement: one declaration per fence identity, identical
  declarations for a shared section identity.
- Referential integrity: every `section_id` on a fence, and every `section_ids` /
  `fence_ids` reference in the four coverage maps, resolves to something the same
  feature declares.
- Parity coupling: a fence cited for `vm-jit-parity` must declare both `vm` and
  `jit`, because §5 makes parity two real executions rather than one execution
  and a label.
- Content identity: canonical JSON with sorted keys, excluding `$schema` and the
  migration record that names the *previous* manifest, so reformatting does not
  read as a different predecessor. `--print-identity` emits it.
- Shared-enum agreement with the `PublicFeatureManifest` schema (§4 above).
- Schema-major migration totality over all three identity domains, where the
  prior set is the union of live and tombstoned identities — a tombstone that
  escaped the map would let a retired identity be silently re-minted.
- §10: budgets only on flagship fences, budgets required on every flagship
  fence, budgets tighten but never loosen, and the flagship set is ratcheted.
- The whole bidirectional join of §3, described in §7 below.

## 6. The load-bearing row

`language.pipe-operator`, the single row of the `PublicFeatureManifest`, paired
with the Book's pipe section.

- **Section** `language.pipe-operator.overview` → `fundamentals/operators.mdx`,
  anchor `pipe`.
- **Fence** `language.pipe-operator.chain`, `runnable-gated`, evidence role
  `success`, declared modes `compile`, `vm`, `jit`, expectation `stdout` =
  `"11\n"`.

Verified rather than cited, at shape `d0939e2b` with a `shape` binary built from
that commit, against shape-web committed `627459a`:

- The fence exists. `fundamentals/operators.mdx` at shape-web `627459a` carries
  ` ```shape runnable=true ` under `## Pipe (\`|>\`)`, containing
  `let result = 5 |> double |> inc` and `print(result)`. It is the only pipe
  material in the Book: a scan of all 102 committed `.mdx` files finds `|>` in
  that one file and nowhere else.
- The fence runs. Extracted verbatim and executed both ways:
  `shape run --mode vm` and `shape run --mode jit` each exit 0 and each write
  exactly `11\n` to stdout, confirmed byte-for-byte with `od -c` (the engine
  banner goes to stderr, which is why the existing harness can compare stdout
  byte-exactly).
- The anchor is `pipe`. Derived by the Astro/Starlight slugger, not guessed:
  the built page carries `<h2 id="pipe">`. Worth checking rather than assuming,
  because the same page's `## Ternary (\`? :\`)` and `## Ranges (\`..\`, \`..=\`)`
  slug to `ternary-` and `ranges-` with a trailing hyphen, which the schema's
  `stableId` pattern would have rejected.

`compile` is a declared mode on the fence because a fence that executes under the
VM necessarily compiled; it is an honest mode, not an inferred one. `vm-jit-parity`
is declared because both modes are declared and the harness compares their
stdout, which is §5's two real executions.

## 7. What the pair does not yet satisfy, exactly

Running the §3 join against the committed pair fails with three messages naming
seven gaps, reproducible with `just check-book-coverage-gaps`:

| Kind | Declared by `language.pipe-operator` | Owned by no Book section or fence |
|---|---|---|
| Required mode | `compile`, `vm`, `jit`, `lsp` | `lsp` |
| Required evidence class | `positive-execution`, `negative-rejection`, `vm-jit-parity`, `lsp-projection` | `negative-rejection`, `lsp-projection` |
| Required semantic dimension | 8 dimensions | `types`, `failure`, `unsupported-use`, `compiler-diagnostics` |

This is not a defect in the manifest and it is not deferred work being labelled
out of scope. It is the first measurement the honest denominator produces: the
Book explains the pipe operator's syntax and one successful result, and says
nothing about how the piped type flows, what happens when it does not, or what
the LSP shows. #112's close already anticipated part of this, noting that the
implementation has two type paths for pipes and only one is implemented.

The rule that detects it is live, tripwired (T8a, T8c) and runnable on demand;
nothing is allowlisted, suppressed or counted. Closing the gaps is Book content
work, which is #116's; making the join a required check is #118's, since ADR-016
§6 puts the bidirectional check inside `BookTruthGate` over an exact pair with a
built binary and the full fence universe. What CI gates today is the contract:
`--self-test`, whose twenty-one forced negatives each also assert their
unmutated positive control is accepted.

## 8. The fence-identity gap this contract creates

ADR-016 §5 requires every Shape fence to carry "a stable explicit identity", and
§3 forbids those identities from containing line numbers or ordinal positions.
The schema enforces both.

The Book's current extractor
(`book/book-site/scripts/extract-shape-snippets.mjs`) mints
`<slice>__<page-slug>__<position>__L<line>.shape` — for the row above,
`A__fundamentals__operators__18__L354.shape`, the page's nineteenth Shape fence
opening at line 354. That identity is both an ordinal and a line number, so it is
not expressible in this manifest by construction, and no committed Book fence
carries an identity that is.

The contract therefore fixes the source of a stable identity as an explicit
author-assigned token in the fence info string, joined to the manifest key by the
gate. It does not add that token to any fence: minting identities across the
whole corpus is #115's inventory and #116's edit. What #113 fixes is that they
cannot be minted from position, which is the property that makes moving prose
safe.

## 9. Deliberately open

- **The inventory is one row**, mirroring #112. §3 and R19 anticipate this:
  complete inventories expand in bounded waves from committed exact rows. There
  is no field in which to claim completeness, because a completeness flag would
  be exactly the mutable verification state §3 forbids.
- **`flagship_fence_ids` is empty.** §10 requires the compiler to report a
  structured ceremony count, which does not exist yet (ADR-017 / R23 territory).
  Designating a flagship fence now would create a budget the gate cannot check,
  which is the same reason #112 declined to declare the `script-tier` dimension.
  The rules are implemented and tripwired (T9, T10) so that turning them on is
  data, not design.
- **`--previous` is unused today.** The manifest is `initial`. Once
  `BOOK-PAIR-PROMOTE` establishes the first accepted pair, the gate invocation
  gains `--previous <accepted manifest>` and the migration record stops being
  `initial`.
- **Where the accepted manifest will live** is `PAIR-PROTOCOL`'s decision, not
  this ticket's: ADR-016 §7 puts it on a protected coordination ref outside both
  source revisions.

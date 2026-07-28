# The public feature candidate inventory (#114, PF-INVENTORY)

**Authority:** ADR-016 §2 (the manifest holds an entry for every public
language construct, annotation, stdlib callable or type, compiler-visible
behaviour, CLI workflow, LSP behaviour, provider surface, snapshot/resume
operation and distributed operator workflow; status is evidence-derived and
ambiguity is a blocking gap), §3 (a complete inventory expands in bounded waves
only after its exact stable rows and content hash are committed, and stays open
until every row has one child owner); ruling R19.
**Artifacts:** `public-feature-candidates.json`,
`scripts/generate-adr011-012-public-feature-candidates.mjs`,
`scripts/check-adr011-012-public-feature-candidates.mjs`.
**Gates:** `just check-public-feature-candidates`;
`just regen-public-feature-candidates`; `ci.yml` job `legacy-authority`;
`scripts/verify-merge.sh` CHECK 19.
**Recorded at:** wave3-spine, on top of `ea67e6d3` (#113).

#112 landed the manifest contract with one row. This is the denominator that row
was the first of: **606 candidate rows** across 11 families, plus five named
holes where the scan cannot see.

## 1. What a candidate row is, and what it deliberately is not

A candidate row records **where a public surface was found** and nothing about
how mature it is. It carries a stable `candidate_id`, a `public_name`, a
`family`, a `surface_authority` naming the exact declaration site, and an
`owner`.

It carries no `status`, no `required_modes`, no evidence classes and no semantic
dimensions, and the gate rejects a row that acquires one. That is not a gap
being deferred; it is the ADR's own division of labour. ADR-016 §2 makes status
evidence-derived and forbids marking a feature aspirationally, and #114's
acceptance criteria say in as many words that "P waves classify status, modes,
dimensions, and evidence". A scanner cannot read evidence. A scanner that
guessed would produce exactly the aspirational denominator ADR-016 exists to
prevent — and it would produce 606 guesses at once.

So the inventory is complete in the dimension #114 owns (which surfaces exist)
and empty in the dimension the P waves own (what each one's evidence supports).
`unresolved_classification` states that on the file itself.

## 2. The scan rules

Every source is a narrow, stated rule, because the value of a mechanical
inventory is that a reviewer can re-derive it rather than audit 606 rows by
hand. `scripts/check-adr011-012-public-feature-candidates.mjs` re-runs all of
them on every CI run and fails on any difference.

| Source | Rows | Rule |
|---|---|---|
| `grammar-declarations` | 22 | Alternatives of the `item_core` rule in `crates/shape-ast/src/shape.pest` |
| `grammar-statements` | 18 | Alternatives of the `statement` rule |
| `grammar-operators` | 47 | Literals of the `*_op` rules, plus the precedence chain's inline operator spellings |
| `stdlib-exports` | 441 | `pub fn` / `pub type` / `pub enum` / `pub const`, plus `trait` and `annotation`, across all 110 `crates/shape-runtime/stdlib-src/**/*.shape` |
| `cli-commands` | 37 | Variants of `Commands` and the `*Action` subcommand enums in `bin/shape-cli/src/cli_args.rs` |
| `lsp-capabilities` | 24 | `*_provider` fields of the advertised `ServerCapabilities` in `tools/shape-lsp/src/server.rs` |
| `permissions` | 17 | Variants of `Permission` in `crates/shape-abi-v1/src/lib.rs` |

Three choices inside those rules are worth stating.

**The grammar is scanned at `item_core` and `statement`, not rule-by-rule.** The
file declares 467 rules, most of which are internal — `item_sync_keyword` is not
a language feature. The alternatives of `item_core` and `statement` are the
grammar's *own* statement of what a user may write at those levels, so they are
the public construct set by construction rather than by a reviewer's judgement.
Rules reachable only from inside another construct are not separately public.

**Operators are one row per spelling.** ADR-016 §2 allows a family to share
documentation but requires every member to be present, and `language.pipe-operator`
— #112's load-bearing row — is exactly one such member. The scanner refuses to
emit an operator whose name it does not have recorded, which is how
`approaching` (a real comparison operator in `comparison_op`) was found rather
than silently skipped, and it verifies every inline operator's named grammar rule
exists before emitting a row, so the inline list cannot drift into claiming an
operator the language does not have.

**`pub` is the stdlib's export boundary**, so a non-`pub` declaration is not
scanned. Traits are the exception: the stdlib declares most of them unmarked
(`trait Add` in `core/add.shape` has no `pub`), and they are nonetheless
user-implementable, so they are scanned without requiring the marker.

## 3. Five holes the scan cannot see

ADR-016 §2 makes an ambiguous inventory a blocking gap, so where a surface is
not mechanically separable from internal machinery the scanner records the hole
instead of guessing. `unresolved_scan_gaps` carries five, each with the reason
and the lane that can close it. Dropping one is a gate failure, so a known hole
cannot quietly become an invisible one.

1. **Builtin type methods** — `Array`, `String`, `HashMap`, `DateTime` and
   friends. Registered through compile-time PHF macros in
   `crates/shape-vm/src/executor/objects/method_registry.rs`, which is not a flat
   literal table a source scanner can read. **Count unknown**, and it is the
   largest of the five: these are among the most-used surfaces in the language.
2. **Compiler diagnostics** — §2 makes compiler-visible behaviour public and §10
   gives diagnostics stable concept identities, but identities are minted at
   emitter sites with no catalog to scan. R23 makes the catalog an ADR-017
   deliverable.
3. **Polyglot and foreign-target surfaces** — the `fn python` / `fn typescript` /
   `extern C fn` *constructs* are scanned from the grammar, but the per-target
   callable surfaces are declared by the extension crates. ADR-019 / R25 territory
   (#163, #164).
4. **Execution providers, snapshot/resume operations and distributed operator
   workflows** — §2 names these explicitly. `shape serve`, `shape wire-serve` and
   the snapshot subcommands appear as CLI rows, but §4's operator workflows are
   procedures, not symbols, and have no declaration site.
5. **Builtin global callables** — globals such as `print` are scanned when
   declared `pub` in `core/intrinsics.shape`, but the `BuiltinFunction` registry
   also admits identities with no Shape-side declaration, and those are not
   separable from internal intrinsics without the ADR-011 `IntrinsicCatalog`
   (#110, R18).

The honest reading of the totals is therefore: **606 rows is a floor, not the
count.** Gap 1 alone is plausibly worth a hundred or more.

## 4. Two things the inventory found

Neither was looked for. Both are what a mechanical denominator is for.

**There are 17 permissions, not 16.** `Permission` carries `Ffi` alongside the
sixteen CLAUDE.md documents in three separate places (the project overview, the
crate map, and the security-model section). `Ffi` is a real variant with real
gating; the documentation simply never followed it. Under ADR-016 §1 this is a
public feature shipped without its documentation.

**The superseded hook-decision enum is a live public stdlib surface.** `core/hooks.shape`
declares it `pub` (see #109), and it is row `stdlib.types.core-hooks-hook-decision`.
Current authority is explicit that it should not be: ADR-012 replaces
the spelling-recognized hook-decision protocol with typed Callable Transforms, R20 lists it
among the superseded formulations, and CLAUDE.md's forbidden-patterns section
names it directly. The inventory cannot decide what to do about it — that is
exactly the classification work the P waves own — but it can make sure the row
is not invisible. Under ADR-016 §2 the two honest outcomes are a `removed` row
with a tombstone pointing at Callable Transforms, or a `deprecated` row with
migration evidence. Silence is not one of them.

## 5. Why the gate re-derives rather than ratchets

The sibling gates (#133/#134/#135 legacy baselines) are shrink-only: a set may
get smaller but never larger. This one fails on **any** difference, and reports
shrinkage before growth.

Shrink-only is the right shape for a set you are trying to empty. This is the
opposite: it is the denominator, and the failure it defends against is the
denominator going stale while waves are planned against it. ADR-016 §3 permits
bounded waves "only after their exact stable rows and content hash are
committed", which is only meaningful if "exact" is checked against the tree.

Shrinkage is reported first because it is the one that loses information. A
public surface that disappears from the candidate inventory before it was ever
entered in the manifest never gets the removed-row tombstone §2 requires — it
just stops existing. Growth is ordinary and the diff shows it.

The stored `rows_sha256` is **recomputed** rather than trusted. This was not the
original design: the first version compared the stored field against the derived
one, and its own tripwire T3 caught that editing a row in place while leaving the
field alone passed the gate. A stored hash that is not recomputed is a claim, not
a check.

Six forced negatives, each asserting both that its mutation is rejected and that
the unmutated input is accepted: a dropped surface, a new surface, in-place row
content drift, a dropped scan gap, a candidate row that acquired a `status`, and
a `count` disagreeing with the rows.

## 6. What #114 cannot close, and who must

#114's fourth acceptance criterion is that the inventory "stays open until its
exact rows are partitioned, the complete concrete children and downstream edges
are separately ratified, every child is created non-ready and natively blocked by
this inventory, and the re-fetched graph audit passes".

The exact rows and their content hash are now committed, which is the
precondition ADR-016 §3 sets. The remaining three steps are not this lane's to
take:

- **The wave partition requires user ratification.** ADR-016 §3 says so
  directly — "The concrete wave breakdown requires user ratification. A family
  label or estimated count is not a publishable wave." Proposing a partition here
  and treating it as ratified would be the failure the sentence describes.
- **Creating the child issues is a GitHub write**, which this lane is not
  authorised to make.
- **The graph audit re-fetches the tracker**, which follows the children.

What is deliverable now, and is: the exact rows, their hash, the family
partition the rows already carry (11 families, sizes in §2's table), and the five
named gaps that tell a ratifier which parts of the denominator are not yet
measurable at all. A ratifier should know before partitioning that gap 1 is
unbounded — a wave breakdown that treats 606 as the total will under-plan.

## 7. Deliberately open

- **`candidate_id` is shaped exactly like a `feature_id`** so that promoting a
  row into the `PublicFeatureManifest` keeps its identity. Renaming at that
  boundary would be an identity migration performed before the identity had ever
  been published, which is precisely the confusion §2's permanence rule exists to
  prevent.
- **No row has been entered into the `PublicFeatureManifest`.** That manifest
  still holds one row, `language.pipe-operator`, and entering more is P-wave work
  gated on evidence per §2. The inventory is the input to that work, not a
  shortcut around it.
- **Duplicate candidate ids are recorded, not merged silently.** The field is
  currently empty; if two sources ever derive the same identity it will be
  visible rather than resolved by scan order.

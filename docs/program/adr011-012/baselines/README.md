# ADR-011–016 step-4 migration baselines

**Authority:** `docs/design/typed-comptime/adr011-012-execution-rulings.md`,
"#90 authority enactment — ten required steps", step 4; ruling R14.
**Tickets:** #133 (SEMANTIC-LEGACY-INVENTORY), #134
(ELABORATION-LEGACY-INVENTORY), #135 (TOOLING-EVIDENCE-INVENTORY).
**Source revision:** `d5579d481a132d53222f9c9ef70459b98fb2ed09`.

Step 4 requires a mechanical inventory of the old authority classes, taken at
an exact Shape revision, storing stable semantic owners plus generated counts
and hashes, so that later slices may only *reduce* their assigned legacy sets.
These three artifacts are that inventory, and the checker is the half that makes
"may only reduce" mechanical rather than aspirational.

## Commands

```
just check-legacy-baselines     # growth gate; exit 1 on any rise
just regen-legacy-baselines     # regenerate after real migration progress

node scripts/check-adr011-012-legacy-baselines.mjs --ticket 134
node scripts/generate-adr011-012-legacy-baselines.mjs --ticket 134
```

Set definitions live in `scripts/lib/adr011-012-legacy-sets.mjs` and the scanner
in `scripts/lib/adr011-012-legacy-scan.mjs`. The generator and the gate import
the same definitions, so a baseline can never be produced by a rule the gate
does not enforce.

## The direction rule

A set count may fall and may never rise. The gate fails on three shapes of
growth, because only failing on the total would leave two ways to grow quietly:

1. the set total rose;
2. the set gained an owner path absent from the baseline — a new surface
   carrying old authority, even when the total is flat;
3. an existing owner's count rose while another fell, hiding growth behind a
   flat total.

A hand-edited baseline is rejected too: each file carries `sets_sha256` over its
own rows, and the gate recomputes it before comparing anything.

Regenerating is legitimate and expected — it is how a slice records progress —
and it is never silent, because the committed diff is the review surface.
Regenerating to absorb a rise is the walk-back this gate exists to catch.

## What is frozen

### #133 — `semantic-legacy-inventory.json`

Discovery producers, ambient comptime entry points and observations, live
intrinsic selectors. `sets_sha256` `3a44f9d7ed4a653e33b6c33fda2f86c27be801e8b542279e8342d55a7a29aa8c`.

| Set | Category | Count | Owners |
|---|---|---|---|
| `ambient-builtin-name-selection` | live intrinsic selectors | 137 | 1 |
| `internal-intrinsic-name-prefix-gates` | live intrinsic selectors | 5 | 2 |
| `allow-internal-builtins-gates` | ambient comptime entry points | 37 | 12 |
| `prelude-name-authority` | discovery producers | 20 | 17 |
| `declaration-discovery-producers` | discovery producers | 44 | 4 |
| `legacy-reflection-call-forms` | ambient comptime entry points | 138 | 27 |

### #134 — `elaboration-legacy-inventory.json`

Annotation identities and routes, universal and string descriptors,
generated-type parser consumers, annotation/backend exceptions. `sets_sha256`
`fe2924e51dd89ece1069cced94e593156fe645bdd717bd2c44b5f1f2b873bee0`.

| Set | Category | Count | Owners |
|---|---|---|---|
| `universal-comptime-target` | universal descriptors | 36 | 9 |
| `string-backed-construction` | string descriptors | 32 | 10 |
| `pseudo-pack-and-marker-substitution` | annotation routes | 105 | 8 |
| `hook-decision-protocol` | annotation identities | 258 | 16 |
| `any-typed-carriers` | string descriptors | 224 | 36 |
| `raw-generated-name-minting` | annotation routes | 65 | 33 |
| `annotation-lowering-exceptions` | annotation/backend exceptions | 332 | 10 |
| `backend-annotation-recognizers` | annotation/backend exceptions | 12 | 3 |

### #135 — `tooling-evidence-inventory.json`

Duplicate LSP semantics, stale tests, old documentation claims. `sets_sha256`
`6091666e5b032ff64977ace120a996c0bde6dee5221efea86e3cfc8848de41b6`.

| Set | Category | Count | Owners |
|---|---|---|---|
| `lsp-parallel-validators` | duplicate LSP semantics | 24 | 4 |
| `lsp-message-scraping` | duplicate LSP semantics | 75 | 11 |
| `ignored-tests` | stale tests | 141 | 27 |
| `tests-asserting-legacy-mechanisms` | stale tests | 70 | 15 |
| `legacy-mechanism-doc-claims` | old documentation claims | 237 | 40 |

## Reading these numbers honestly

**Counts are occurrences, not defects.** A nonzero count is the current state of
a mechanism ADR-011/012 replaces. The only claim any number makes is
directional.

**Sets overlap on purpose, so `total_count` is not a site count.**
`annotation-lowering-exceptions` is `hook-decision-protocol` plus
`pseudo-pack-and-marker-substitution` narrowed to the bytecode compiler, and it
re-counts sites those sets already count. Each set is its own ratchet; the
per-file totals are the aggregate for reporting movement, never a population of
distinct legacy sites.

**Some sets are supersets of their category.** `ignored-tests` counts every
`#[ignore]`, and some of those are live feature gaps rather than stale evidence;
per-bucket classification stays with
`scripts/check-ignored-test-classification.py`. `any-typed-carriers` counts
every `FieldType::Any`, not only annotation carriers.

**The doc-claims set counts enforcement prose too.** `CLAUDE.md` and the ADRs
name these mechanisms in order to forbid them, and those mentions count. The
generated baselines themselves are excluded — they quote every mechanism name in
their own patterns, so counting them would make the instrument measure itself.

## Observations worth carrying into the slices

- **One file holds the ambient intrinsic authority.**
  `crates/shape-vm/src/compiler/helpers.rs` is the sole owner of all 137
  terminal-name → `BuiltinFunction` arms, and also defines
  `is_internal_intrinsic_name`, the `__native_` / `__intrinsic_` / `__json_`
  prefix gate. #92 and #177 have a single, unusually well-localized target.
- **The legacy reflection surface is still live on `main`.** 138
  `type_info(...)` / `implements("...")` sites remain, because the deletion
  (`f58a0d85`) exists only on the paused `adr009/e6` branch. See
  `docs/program/adr011-012/e6-disposition.md`, where that commit is dispositioned
  **salvage**; 47 of the 138 are in `tools/shape-test/tests/comptime/type_info_chained.rs`,
  the file that deletion removes.
- **The backends are nearly clean already.** `backend-annotation-recognizers` is
  12, and 10 of those are the `no_legacy_annotation_weave` sentinel that enforces
  the absence. The ratchet's job here is to keep it there, not to drive it down.
- **R23's "eleven parallel validators" undercounts.** The measured
  `fn validate_*` population in `tools/shape-lsp/src` is 24, twelve of them in
  `diagnostics.rs` alone. The ruling's prose figure is not a baseline; this
  number is.

## What a later slice owes

A slice that migrates part of a territory lowers its sets and regenerates, so
the diff shows exactly which owners were retired. A slice that needs to add a
site to a frozen set does not get to regenerate around it: either the new
surface enters through the resolved typed pipeline, or the addition needs
explicit authority recorded with it.

At final deletion the routing authority itself disappears (R14), and these sets
reach zero. Until then, `just check-legacy-baselines` is the only mechanical
statement that the direction is holding.

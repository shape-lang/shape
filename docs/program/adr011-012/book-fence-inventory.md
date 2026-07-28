# The Book fence universe (#115, BOOK-FENCE-INVENTORY)

**Authority:** ADR-016 §3 (stable identities carry no line number or ordinal;
exact revisions stay external), §5 (every Shape fence has a stable explicit
identity and exactly one classification; illustrative fences carry a reason and
a citation; negative examples assert a structured diagnostic identity; parity is
two real executions), §6 (the gate extracts the full Shape-fence universe, and a
percentage over a curated subset cannot satisfy it); ruling R19.
**Artifacts:** `book-fence-inventory.json`,
`scripts/generate-adr011-012-book-fence-inventory.mjs`,
`scripts/check-adr011-012-book-fence-inventory.mjs`.
**Gates:** `just check-book-fences`; `just regen-book-fences`; `ci.yml` job
`legacy-authority`; `scripts/verify-merge.sh` CHECK 20.
**Recorded at:** wave3-spine, on top of `6f5f420f` (#114). Corpus scanned:
shape-web committed `627459a`, recorded here as provenance prose only — §4 below
explains why no revision enters the inventory file.

## 1. The three numbers

| | Count |
|---|---|
| Fences in the Book, every language | **977** |
| Shape fences | **767** |
| Shape fences the committed harness executes | **385** |

All three are reported and none is called "the universe" on its own. That is the
whole point of the ticket, and the gate enforces it: removing the
`executed_subset_note`, or editing `shape_fences` down to equal the executed
count, both fail.

The third number is the one to be careful with. `run-book-truth-gate.mjs` line
577 keeps only `runnable === true && !deferred`, so a green run of the committed
harness is a statement about 385 fences, not 767 and not 977. ADR-016 §6 says
directly that "a percentage threshold, curated subset, unchanged count, or report
from an uncommitted harness cannot satisfy the gate", and this repository has hit
that trap before: a recorded 240/240 green was a curated subset while the real
book-truth rate was near half.

The remaining **382 Shape fences carry `runnable=false`** and are never executed
by anything. Under ADR-016 §5 each of them is either a `runnable-gated` fence
that is failing, or an `illustrative-only` fence that owes a reason and a
citation. Today none of them says which, and §5's rejected-alternatives list
names that exact state: "Allow `runnable=false` without a reason. A broken
implementation can be hidden as documentation."

By language: shape 767, bash 69, rust 37, text 17, toml 14, json 12, lua 3,
vim 3, c 1, python 1, typescript 1, and 52 with no language tag. 102 pages, 1374
sections.

## 2. What a scanned row records, and what it refuses to

A fence row records where the fence is (`page`, `fence_position`,
`section_anchor`), what its info string says (`language`, `runnable_flag`,
`fixture`, `cite`, and the derived `expectation_kind`), the harness identity it
currently has, and a list of **markers** naming what ADR-016 §5 wants and the
corpus does not have.

It carries no `classification`, no `illustrative` block and no `expectation`
object, and the gate rejects a row that acquires one. §5's classification is
`runnable-gated` *with declared modes and an expected outcome*, or
`illustrative-only` *with a nonempty reason and an issue or semantic-authority
citation*. Neither is derivable from a fence-info string: a reason is a
judgement, and a citation is a decision about authority. A scanner that emitted
`classification: "illustrative-only"` for all 382 `runnable=false` fences would
be asserting 382 reviews that never happened — and it would make the
illustrative set, which §5 ratchets, look established rather than unwritten.

So rows carry `classification_candidate` instead, and the candidate is always
accompanied by the markers explaining why it is not yet the real thing.

## 3. The marker totals

| Marker | Count | What it means |
|---|---|---|
| `missing-stable-identity` | 767 | Every Shape fence. No fence declares an `id=` token |
| `no-illustrative-reason` | 382 | Every `runnable=false` fence |
| `no-illustrative-citation` | 382 | Same set |
| `no-declared-expected-value` | 354 | Executed, but gated only on exit-0 and VM/JIT equality |
| `expectation-is-a-rendered-substring` | 10 | `expected-fail=` matches rendered text, not a diagnostic identity |
| `no-owning-section` | 11 | Fence appears before the page's first heading |

`missing-stable-identity` covering all 767 is the headline. ADR-016 §5 requires
"a stable explicit identity" and §3 forbids line numbers and ordinal positions in
it; the committed extractor mints
`<slice>__<page-slug>__<position>__L<line>.shape`, which is both at once. There
is therefore no Shape fence in the Book whose identity #113's coverage schema can
express, and #113's `coverageId` definition rejects those spellings by
construction. This is the single largest piece of Book-side work the ADR implies,
and it is #116's: minting identities is an edit to 767 fences, not an inventory.

`no-declared-expected-value` at 354 of 385 executed fences is the second. Those
fences pass on exit-0 plus VM/JIT stdout equality — a real check, but one that
cannot distinguish correct output from consistently wrong output. Only 21 declare
an expected value.

## 4. The inventory records no shape-web revision

ADR-016 §3 keeps exact revisions out of source manifests and §7 keeps them in the
external `PairCandidate` and attestation, precisely so the two repositories never
pin each other. A `shape_web_sha` field here would be that reciprocal pin, and
the gate rejects one: any bare 40- or 64-character digest in the `corpus` record
fails (T9).

What pins the inventory instead is its own content: `sections_sha256` and
`fences_sha256` over the canonical JSON of the rows. Both are **recomputed** by
the checker rather than trusted, for the reason #114's doc records — a stored
hash that is not recomputed is a claim, not a check.

The scan reads shape-web's **committed** content through `git show HEAD:<path>`,
never the working tree. A documentation repository's working tree is routinely
some lane's uncommitted draft; an inventory derived from it would describe a
corpus nobody else can see and would change under its own feet. At the time of
scanning, shape-web's working tree held 56 modified files that this inventory
correctly does not see.

## 5. Two halves, because the two repositories are not always both present

The gate splits:

- **Integrity** needs only the committed file. It recomputes both hashes, checks
  every count and every marker total against the rows, refuses a row that
  acquired a real classification, rejects a fence identity containing an ordinal
  or line number, rejects a duplicate identity, and rejects a smuggled revision.
  It runs anywhere, including this repository's CI.
- **Currency** needs `--shape-web <path>` and re-derives every row from the
  corpus. It runs locally, and inside `BookTruthGate`, which ADR-016 §7 already
  requires to have both revisions checked out.

The output states which half ran. A gate that silently skipped its re-derivation
and printed "OK" would be the more dangerous shape, so the CI line says
"Integrity only — no shape-web checkout, so currency was NOT verified."

Nine forced negatives, each with a positive control: rows edited without
regenerating; the executed subset presented as the universe; the subset note
removed; a scanned row given a real classification; an identity built from a line
number; one identity declared twice; the gap list emptied; a marker count quietly
lowered; a counterpart revision smuggled into the corpus record.

## 6. The anchor rule is verified, not assumed

Section anchors are derived, and the derivation was checked against the built
site: **1359 of 1359 headings match, 100%**.

That check was worth running. The rule is not github-slugger: it is
github-slugger's output minus one trailing hyphen. `## Pipe (\`|>\`)` slugs to
`pipe-` under the library and is `pipe` in the built page — and #113's schema
rejects a trailing hyphen, so a guessed anchor would have been an anchor the
coverage manifest cannot express. #113's load-bearing row depends on this exact
value.

Validation also found a second rule the first pass had wrong. A page body may
carry its own `# Title` heading, which takes the base slug and pushes a later
same-named `##` to `-1`; scanning from `h2` produced exactly one wrong anchor
across the whole site (`stdlib/core/transport.mdx`, where the body's `# transport`
takes `transport` and the section heading becomes `transport-1`). One wrong
anchor in 1359 is the kind of thing that survives review and breaks a coverage
row a year later.

## 7. Five gaps, and who owns them

Recorded in `unresolved_gaps`; emptying the list is a gate failure.

1. **Stable fence identities** — 767 fences, zero identities. #116.
2. **Illustrative reasons and citations** — 382 fences expressing non-execution
   as a bare `runnable=false`. #116, and it is per-fence judgement, not a
   mechanical pass.
3. **Structured negative expectations** — `expected-fail=` is a substring of
   rendered output; §5 wants a diagnostic identity and essential typed payload.
   #116, blocked on the ADR-017 / R23 diagnostic catalog.
4. **Declared modes** — `declared_modes` records what the harness happens to run
   (VM and JIT, or VM only for fixture-backed fences), not what a fence declares.
   §5 makes declared modes part of the classification. #116.
5. **Native-execution claims** — §5 and R15 require a fence claiming native
   execution to name the exact function or realization and carry a
   `NativeExecutionWitness`. No fence-info token expresses that, so a prose
   native claim is invisible to this scan. #116 plus the R15 witness lane.

## 8. How the counts were verified

The scanner's classification counts were cross-checked against an independent
grep over the same committed content, and agree exactly on every figure: 767
Shape fences, 382 `runnable=false`, 0 `runnable=deferred`, 21 `expected=`, 10
`expected-fail=`, 8 `fixture=`, 0 `id=`.

The independent count also confirms the scanner sees three indented Shape fences
that a naive `^\`\`\`shape` grep misses — the scanner matches leading whitespace
and pairs delimiters by run length, so a fence nested in a list item or an aside
is counted.

No page has an unterminated fence, which the gate also asserts: an unclosed
delimiter would make every subsequent fence boundary on that page unreliable, so
it fails rather than producing plausible-looking rows.

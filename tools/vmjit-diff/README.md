# vmjit-diff — VM-vs-JIT differential harness (WF-0B)

Measurement infrastructure that runs every corpus program under
`shape run --mode vm` and `shape run --mode jit` and diffs stdout + exit code.
It **records** divergences; it does not fix them.

## Layout

| Path | What |
|---|---|
| `corpus/` | Committed corpus: one `.shape` file per program + `manifest.json` (provenance) |
| `build-corpus.mjs` | Regenerates `corpus/` + `SKIPPED.md` from the three source tiers |
| `run-diff.mjs` | The differential runner |
| `known-red.json` | Allowlist of expected divergences, pinned by corpus id, each with an audit citation |
| `synthetic/` | Hand-written known-divergence repro sources (copied into the corpus by the builder) |
| `SKIPPED.md` | Log of every skipped program (network / user-input only — no silent caps) |
| `reports/` | Runner output (gitignored): `report.json` + `report.md` |

## Corpus tiers

1. **book** — all `runnable=true` ```shape fences from the book
   (`shape-web/book/book-site/src/content/docs/**/*.mdx`), extracted with the
   canonical committed extractor
   (`book-site/scripts/extract-shape-snippets.mjs`). Ids keep the snippet-ID
   convention `<slice>__<page-slug>__<fence-pos>__L<line>.shape` — traceable
   to the exact page + line.
2. **acceptance** — the self-asserting programs from
   `docs/cluster-audits/v0.3.3-book-acceptance/programs/<slice>/*.shape` as
   `ACC__<slice>__<file>.shape`. Script mode does not auto-invoke `fn main`,
   so the builder appends a trailing `main()` call where needed (recorded as
   `transform` in the manifest).
3. **synthetic** — `SYN__*.shape` known-divergence repros from `synthetic/`.

## Classification

| Class | Meaning |
|---|---|
| `MATCH` | stdout identical AND exit code identical (incl. identical failures) |
| `VM_FAIL` | vm non-zero exit, jit exit 0 |
| `JIT_FAIL` | jit non-zero exit, vm exit 0 |
| `DIVERGED` | stdout differs, or both non-zero with different codes |
| `TIMEOUT` | either mode exceeded the per-mode timeout (default 10s) |

Exit code: `0` all MATCH or known-red; `1` any unexpected non-MATCH; `2` harness error.
stderr is captured in the report for every non-MATCH (the `[jit-fallback]`
diagnostic and the double-execution class live there) but is NOT diffed.

## Usage

```bash
# Full pipeline (builds the release binary first):
just diff-vmjit

# Against an existing binary, no build:
SHAPE_BIN=/path/to/shape just diff-vmjit-fast
SHAPE_BIN=/path/to/shape just diff-vmjit-fast --limit 20
SHAPE_BIN=/path/to/shape just diff-vmjit-fast --tier synthetic

# Regenerate the corpus (requires shape-web checkout as sibling of the repo root):
just diff-vmjit-corpus
```

Run everything via `direnv exec /home/dev/dev/shape-lang <cmd>` per repo convention.

## Known-red discipline

`known-red.json` entries require an audit/issue citation and are pinned by
exact corpus id. Expected classes at seed time (audit 2026-07-04
`docs/cluster-audits/audit-2026-07-04-claimed-vs-real.md` §4): JIT
double-execution (signal -1 re-run), i64 overflow wrap (D3 violation),
annotation hooks dropped under JIT, HashMap.filter garbage. Only the
synthetic overflow repro is pre-pinned; book-tier ids get pinned from the
first full-corpus report after per-id verification.

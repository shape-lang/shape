# Wave 31 Book Stale-Candidate Probes

Date: 2026-07-10
Supervisor: book-truth completeness campaign

## Question

Wave-30 left a small set of stale-green/count-reduction candidates. This pass
checked the cheap candidates directly against the shipped release binary before
changing any book pages.

## Probe Evidence

Direct candidate probe service: `run-p703833-i31343356.service`.

Environment: release `target/release/shape`, VM and JIT modes, empty
`SHAPE_CONFIG_DIR` so stale local extensions did not load.

| Snippet | VM result | JIT result | Decision |
|---|---|---|---|
| `B__fundamentals__datetime__17__L364.shape` | prints `true`, `true` | segfault | keep disabled; real DateTime JIT blocker |
| `B__fundamentals__datetime__19__L404.shape` | prints six `true` rows | segfault | keep disabled; real DateTime JIT blocker |
| `A__fundamentals__resource-management__5__L139.shape` | parse error on old `drop(self)` trait syntax | same parse error | rewrite to current syntax |
| `B__stdlib__domain__finance__0__L16.shape` | semantic error: required parameter after default | same semantic blocker | keep disabled; finance/domain import gap |
| `E__tooling__frontmatter__0__L12.shape` | missing local `../utils` path dependency | same missing dependency | rewrite to self-contained frontmatter example |

Scratch rewrite probe service: `run-p704843-i31344416.service`.

| Scratch row | Result |
|---|---|
| `trait Drop { method drop() }` | VM passed; JIT deopted safely to interpreter and passed |
| metadata-only frontmatter script | VM/JIT printed `ready` with frontmatter warnings on stderr |

## Book Changes

Changed sibling book pages:

- `../shape-web/book/book-site/src/content/docs/fundamentals/resource-management.mdx`
  - `Drop` trait definition now uses current `method drop()` syntax and is
    runnable.
- `../shape-web/book/book-site/src/content/docs/tooling/frontmatter.mdx`
  - runnable example is now a self-contained metadata-only script.
  - module/dependency/extension config remains documented as TOML because it
    requires matching local files and extension libraries.

## Verification

Static check:

- `git -C ../shape-web diff --check -- book/book-site/src/content/docs/fundamentals/resource-management.mdx book/book-site/src/content/docs/tooling/frontmatter.mdx`

Book-truth verification:

| Gate | Service | Result |
|---|---|---|
| extraction | `run-p705690-i31345315.service` | 707 total / 543 runnable / 164 disabled / 0 deferred |
| slice A | `run-p705909-i31345549.service` | 220/220 passed |
| slice E | `run-p705909-i31345549.service` | 27/27 passed |
| full release book gate | `run-p738434-i31378206.service` | 543/543 passed; report `/tmp/shape-wave31-book-truth-report.json` |

## Remaining Follow-Up

- DateTime examples are not stale-green; they expose a shipped JIT segfault and
  should move to a DateTime JIT implementation lane.
- Finance import remains an active domain/stdlib semantic blocker.
- Frontmatter dependency/extension sections need a future fixture if the book
  gate should execute them as a whole script.

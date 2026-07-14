# Wave 16 Current Disabled Inventory

Date: 2026-07-09
Source: `/home/dev/dev/shape-lang/shape-web/book/book-site/.book-truth-gate/snippets/manifest.json`

## Counts

| Metric | Count |
|---|---:|
| Total snippets | 756 |
| Runnable snippets | 507 |
| Disabled snippets | 249 |

This inventory is the current post-Wave-15 disabled baseline. It supersedes the
older 394/256-style disabled counts in prior discussion and earlier wave
triage notes.

## Largest Disabled Pages

| Page | Disabled | Current lane |
|---|---:|---|
| `stdlib/native/io.mdx` | 25 | External/manual fixtures, async IO, permissions |
| `stdlib/core/state.mdx` | 16 | State/capture/resume implementation |
| `advanced/ownership-deep-dive.mdx` | 10 | Ownership/reference/storage semantics |
| `fundamentals/content.mdx` | 10 | Typed-array/content container parity |
| `advanced/content-addressed-bytecode.mdx` | 9 | State/resume plus content-addressed composition |
| `advanced/security-permissions.mdx` | 9 | Security policy/proof and deterministic harnesses |
| `stdlib/native/http.mdx` | 9 | External HTTP/async fixtures |
| `advanced/transport-layer.mdx` | 7 | Distributed transport proof matrix |
| `stdlib/core/remote.mdx` | 7 | Remote/distributed harness and stale snippets |
| `fundamentals/datetime.mdx` | 6 | DateTime/JIT parity and stale method examples |
| `fundamentals/error-handling.mdx` | 6 | Old syntax/policy and Result ergonomics |
| `fundamentals/traits.mdx` | 6 | Trait/conversion gaps |
| `tooling/python-extension.mdx` | 6 | External extension packaging/harness |
| `advanced/developer-tools.mdx` | 5 | Diagnostics/proof tooling |
| `fundamentals/references-borrowing.mdx` | 5 | Reference/storage semantics |
| `fundamentals/tables.mdx` | 5 | Typed-array/table container parity |
| `stdlib/native/archive.mdx` | 5 | External/manual archive fixtures |
| `stdlib/native/file.mdx` | 5 | Filesystem fixture policy |
| `tooling/typescript-extension.mdx` | 5 | External extension packaging/harness |

## Current Lane Grouping

This is a path-based supervisor grouping, not a final per-snippet root-cause
classification. It is useful for dispatch order and should be refined by each
implementation/proof wave as snippets flip or are confirmed manual-only.

| Lane | Disabled |
|---|---:|
| Native external / IO / HTTP / security harnesses | 56 |
| Distributed / remote / transport / polyglot | 34 |
| Math / typed-array / `Mat` / intrinsic carrier migrations | 31 |
| State / resume / content-addressed bytecode | 27 |
| Comptime / annotations / proof / developer tooling | 17 |
| Ownership / reference / storage semantics | 17 |
| Old syntax / getting-started / policy rewrites | 14 |
| Traits / conversion / testing / property testing | 14 |
| Async-specific page snippets | 1 |
| Other or mixed-domain pages | 38 |

## Supervisor Read

The disabled set is no longer primarily stale drift. The largest remaining
source of raw count is deterministic harness work for native IO/HTTP/security,
but the most completeness-critical implementation/proof blockers are:

1. State/capture/resume and content-addressed bytecode.
2. Distributed/snapshot/polyglot proof matrix.
3. Typed-array/`Mat`/intrinsic carrier migration.
4. Ownership/reference/storage semantics.
5. Trait/conversion/testing/property gaps.
6. Comptime typed reflection/fragments/hygiene.
7. DateTime/JIT parity and other fallback-only surfaces.

Wave-16 dispatch is aligned to this inventory:

- `Wave-16A`: prepare the JIT+GC vs JIT/no-GC barrier perf measurement.
- `Wave-16B`: verify and close the sibling `../shape-app` GC feature gap.
- `Wave-16C`: implement the smallest state-resume lane, `state::caller()`.
- `Wave-16D`: add distributed/snapshot/polyglot proof-matrix rows.


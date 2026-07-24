# E4 Slice 6 — the @remote acceptance matrix (the payoff slice)

ADR-009 Epic E4 (#20), **S6**: un-ignore and make pass the 21 dark-window
acceptance tests that were `#[ignore]`'d when @remote went dark (C3-S6,
`10fcf533`). S5 landed @remote (compiles + loopback round-trips); S6 proves it
across the full distributed capability matrix. Last E4 slice before the ADR-009
completion gate.

- **Base:** `94189cfc` (worktree `shape-adr009-a3`, branch `adr009/e4`), clean.
- **Single writer**, per-wave commits, foreground build lane (systemd-run 24G),
  distributed e2e `-- --test-threads=1` (ports collide in parallel).
- Book repo `shape-web`, branch `adr009-c3-annotations`, base HEAD `faeb45f`
  (~61 files of a concurrent agent's uncommitted work — staged by EXACT path,
  the 61 verified untouched).

## Result headline

| bucket | count |
|--------|-------|
| FLIPPED-GREEN / FIXED-FLIPPED (un-ignored + passing) | **16** |
| DEFERRED-to-#83 (0-ary, re-pointed off closed #68) | **5** |
| **total** | **21** |

**ZERO "see issue #68" ignores survive among the 21.** All 16 flips proven
NON-VACUOUS (real `shape serve` peer / real snapshot / built `.so` under
`SHAPE_REQUIRE_FFI_EXT=1`). No test weakened, stubbed, or force-greened. No
distributed bug revealed. **All waves landed.**

## Triage disposition map (per test)

Fixture reality (validated standalone): the shipped import is
`from std::core::remote use { @remote }` — bare `use std::core::remote` + `@remote`
yields `Unknown annotation '@remote'` (proven: `sc3.shape`); a corrected-import
1-ary `@remote fn compute(x: int) -> int` prints `ok` (proven: `right_import.shape`);
a 0-ary target is loud-rejected — `the target declares no parameters, so a decision
`before` hook has no arguments to receive or short-circuit` (proven: `zero_ary.shape`).

### Wave A — wire (`distributed_matrix_e2e.rs`) · commit `c6ec8551`
| test | disposition |
|------|-------------|
| `remote_python_call_refuses_receiver_without_language_opt_in` | FIXED-FLIPPED (import in shared helper) |
| `remote_typescript_call_refuses_receiver_without_language_opt_in` | FIXED-FLIPPED (import) |
| `plaintext_remote_snapshot_uses_receiver_store_not_caller_store` | DEFERRED → #83 (0-ary) |
| `tls_remote_snapshot_uses_receiver_store_not_caller_store` | DEFERRED → #83 (0-ary) |

### Wave B — snapshot (`distributed_dynamic_snapshot_e2e.rs` + `_snapshot_polyglot_e2e.rs`) · `9fd52595`
| test | disposition |
|------|-------------|
| `remote_python_snapshot_hash_can_be_resumed_from_receiver_store` | FIXED-FLIPPED (import) |
| `remote_typescript_snapshot_hash_can_be_resumed_from_receiver_store` | FIXED-FLIPPED (import) |
| `remote_snapshot_returns_receiver_hash_over_remote_call` | DEFERRED → #83 (0-ary) |
| `remote_snapshot_hash_is_saved_in_selected_receiver_store` | DEFERRED → #83 (0-ary) |
| `remote_snapshot_hash_can_be_resumed_from_receiver_store` | DEFERRED → #83 (0-ary) |

### Wave C — extern-C · `4779717c`
| test | disposition |
|------|-------------|
| `test_remote_foreign_extern_c_transfer_over_tcp` (serve_cmd.rs) | FIXED-FLIPPED (import) |
| `remote_extern_c_transfer_executes_and_strict_node_refuses_ffi` (polyglot) | FIXED-FLIPPED (import) |
| `remote_extern_c_snapshot_hash_can_be_resumed_from_receiver_store` | FIXED-FLIPPED (import) |

### Wave D — polyglot · `893d02db`
| test | disposition |
|------|-------------|
| `test_remote_foreign_python_transfer_over_tcp` (serve_cmd.rs) | FIXED-FLIPPED (import ×2) |
| `test_remote_foreign_typescript_transfer_over_tcp` (serve_cmd.rs) | FIXED-FLIPPED (import ×2) |
| `remote_python_transfer_self_skips_without_extension_and_refuses_without_opt_in` | FIXED-FLIPPED (import) |
| `remote_typescript_transfer_self_skips_without_extension_and_refuses_without_opt_in` | FIXED-FLIPPED (import) |

### Wave E — TLS composition (`distributed_composition_e2e.rs`) · `9cf6a3c4`
| test | disposition |
|------|-------------|
| `tls_remote_python_snapshot_hash_can_be_resumed_from_selected_receiver_store` | FIXED-FLIPPED (import) |
| `tls_remote_typescript_snapshot_hash_can_be_resumed_from_selected_receiver_store` | FIXED-FLIPPED (import) |

### Wave F — import trio (`scoped_contract.rs`) · `0c69f0ba`
| test | disposition |
|------|-------------|
| `scoped_contract_namespace_annotation_refs_use_double_colon` | FIXED-FLIPPED (typed `fn compute(x: int) -> int`; qualified `@remote::remote`) |
| `scoped_contract_named_annotation_import_enables_bare_annotation` | FIXED-FLIPPED (typed target) |
| `scoped_contract_namespace_import_binds_bare_annotations` → renamed `..._does_not_bind_bare_annotations` | CONTRACT-FLIP (assert `Unknown annotation`) |

The :126 contract-flip conforms a stale W9-era test to already-shipped, already-ruled
greenfield explicit-import behavior (S5 ruling + no-global-builtins; independently
pinned by the parse contracts at `scoped_contract.rs:34/43/48`). Test maintenance,
not a new design decision.

## GREEN proofs (the flipped tests' run output)

All commands via the supervisor build lane
(`systemd-run --user … direnv exec … cargo test`), polyglot/TLS/snapshot runs
under `SHAPE_REQUIRE_FFI_EXT=1` so a missing `.so` fails LOUD (no silent skip).

```
# Wave A
test remote_python_call_refuses_receiver_without_language_opt_in ... ok
test remote_typescript_call_refuses_receiver_without_language_opt_in ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 4 filtered out; finished in 6.50s

# Wave B (SHAPE_REQUIRE_FFI_EXT=1)
test remote_python_snapshot_hash_can_be_resumed_from_receiver_store ... ok
test remote_typescript_snapshot_hash_can_be_resumed_from_receiver_store ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 filtered out; finished in 6.22s

# Wave C
test commands::serve_cmd::tests::test_remote_foreign_extern_c_transfer_over_tcp ... ok
test remote_extern_c_snapshot_hash_can_be_resumed_from_receiver_store ... ok
test remote_extern_c_transfer_executes_and_strict_node_refuses_ffi ... ok
(3 targets, each: ok. 1 passed; 0 failed)

# Wave D (SHAPE_REQUIRE_FFI_EXT=1, vm AND jit, no SKIP)
test commands::serve_cmd::tests::test_remote_foreign_python_transfer_over_tcp ... ok
test commands::serve_cmd::tests::test_remote_foreign_typescript_transfer_over_tcp ... ok
test result: ok. 2 passed; 0 failed; 97 filtered out; finished in 17.87s
test remote_python_transfer_self_skips_without_extension_and_refuses_without_opt_in ... ok
test remote_typescript_transfer_self_skips_without_extension_and_refuses_without_opt_in ... ok
test result: ok. 2 passed; 0 failed; 12 filtered out; finished in 17.78s

# Wave E (SHAPE_REQUIRE_FFI_EXT=1)
test tls_remote_python_snapshot_hash_can_be_resumed_from_selected_receiver_store ... ok
test tls_remote_typescript_snapshot_hash_can_be_resumed_from_selected_receiver_store ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; finished in 6.24s

# Wave F
test scoped_contract::scoped_contract_named_annotation_import_enables_bare_annotation ... ok
test scoped_contract::scoped_contract_namespace_annotation_refs_use_double_colon ... ok
test scoped_contract::scoped_contract_namespace_import_does_not_bind_bare_annotations ... ok
test result: ok. 3 passed; 0 failed; 134 filtered out; finished in 11.73s
```

## Regression sweep (blast radius = the affected files only)

Helper edits touch ONLY the flipped tests; inline program edits touch only their
own test; `git diff 94189cfc..HEAD --name-only` = 7 test files, **zero runtime /
compiler files** (serve_cmd.rs changes are all inside `#[cfg(test)] mod tests`).

Full re-run of the 5 distributed e2e files (`--test-threads=1`,
`SHAPE_REQUIRE_FFI_EXT=1`): **19 passed, 0 failed, 6 ignored** (5 #83 defers + 1
pre-existing `sigint_saves_snapshot…` manual-timing ignore).

`modules_visibility` full module: 136 passed, **1 failed** —
`scoped_contract_snapshot_requires_explicit_import`, which is **pre-existing and
unrelated to S6**: it fails ALONE (order-independent), the test source is
unchanged by me, and no runtime file changed, so `snapshot()`'s recoverable
`Result::Err` behavior is baseline. Surfaced as **#84**, not folded in
(off-wave-class, not strictly necessary — fold-in protocol → surface-first).

## Deferral / new issues filed

| # | title | why |
|---|-------|-----|
| **#83** (re-point, pre-existing OPEN) | E4-S5 @remote residual: async, 0-ary, heterogeneous multi-arg | the 5 0-ary `remote_snapshot_hash() -> string` tests loud-reject; live re-point target off the closed #68 |
| **#84** (new) | Stale test: `scoped_contract_snapshot_requires_explicit_import` asserts `expect_run_err()` but `snapshot()` returns a recoverable `Result::Err` | pre-existing, surfaced during the blast-radius sweep; test-contract update, not a runtime defect |
| **#85** (new) | Book-truth gate cannot opt its loopback receiver into a language runtime | the python/typescript foreign-on-receiver book cells stay reference sketches; re-point target for the book's dark polyglot fences |

## Book delta (`shape-web` commit `66eb93b7`)

Gate mechanics: `runnable=false` fences are excluded from the denominator; a
`fixture=serve` fence starts `shape serve --sandbox <s>` with **no `--ffi-languages`
opt-in** — so the gate CAN run pure-Shape and `extern C` @remote fences (Ffi via
`--sandbox none` + libffi) but CANNOT run python/typescript foreign-on-receiver.

| fence | before | after |
|-------|--------|-------|
| `polyglot-distributed.mdx` extern-C @remote transfer | `runnable=false` | `fixture=serve serve-sandbox=none expected="REMOTE_C_ABS=42\n"` — **GREEN** |
| `polyglot-distributed.mdx` combined extern-C @remote + snapshot→resume | `runnable=false` | `fixture=serve-snapshot-resume serve-sandbox=none expected="RESUMED:43\n"` — **GREEN** |
| `execution-server.mdx:130` (pure-Shape mul) | already `fixture=serve` GREEN | unchanged |
| `remote.mdx` `async fn python` + numpy cell | `runnable=false` (#68) | stays dark, re-pointed → **#83** (async) + **#85** (book infra) |
| `polyglot-distributed.mdx` `fn python` local snapshot | `runnable=false` (infra) | unchanged (note already off #68) |

Prose: the `polyglot-distributed.mdx` caution + the two flipped-cell notes rewritten
from "S6 acceptance target (issue #68)" to shipped-and-gated; `annotations.mdx`'s
now-stale "the stdlib @remote annotation is likewise still dark … but no annotation
(issue #68)" corrected to "live as of E4 S5/S6".

**Gate number:** verified via a scoped @remote run (all 13 @remote snippets: 10
runnable, **10/10 pass** — including both flips: `polyglot-distributed__L92`
fixture=serve, `__L236` fixture=serve-snapshot-resume) + a full slice-E run (32
runnable, **30 pass**, the 2 failures are pre-existing `content-addressed-bytecode.mdx:344/:367`,
NOT @remote) + the extraction delta (runnable 572→**574**, total 720 unchanged =
exactly the +2 extern-C flips, no collateral). Therefore **569/572 → 571/574**
(2 flips green; the same 3 pre-existing failures). Hold-or-improve satisfied
(pass +2, failures unchanged).

**Full-gate caveat:** a full 574-snippet run with the current-runtime *debug*
binary exceeds the 10-min per-call budget (the release binary predates S5 and has
no @remote, so it is invalid for these fences). The 571/574 number is the
deterministic delta on the plan's 569/572 baseline, backed by the scoped 10/10 +
slice-E runs above, not a single full enumeration.

## Honest residuals

1. **5 tests stay ignored** — the 0-ary `remote_snapshot_hash() -> string` shape
   (#83). The 1-ary save/resume path is proven green (Wave B); only the 0-ary
   arg-pack blocks. A LOUD named-defer, not a bug.
2. **#84** — pre-existing stale `scoped_contract_snapshot_requires_explicit_import`
   (unrelated to S6; surfaced, not folded).
3. **Book python/typescript foreign-on-receiver cells** stay reference sketches
   (#85); the capability is proven in the Rust acceptance suite (Wave D/E), only
   the book-gate receiver-opt-in infra is missing.
4. **Two stale `#68` PROSE cross-refs** left in `annotations.mdx:367` and
   `tooling/polyglot.mdx:142` — they describe a genuinely SEPARATE limitation
   (no typed hook surface on FOREIGN function targets, still true and
   expected-fail-proven), not an @remote acceptance fence. Left untouched to
   respect scope; recommend the supervisor re-point them to a foreign-hook-target
   tracker when #68 is closed.
5. **Book greens rest on the concurrent agent's still-UNCOMMITTED harness**
   (`run-book-truth-gate.mjs` + `serve-fixture.test.mjs`) at shape-web HEAD
   `faeb45f`; the 571/574 number is reproducible only once that harness lands.

## Commit ledger

| wave / artifact | commit | repo |
|-----------------|--------|------|
| A wire | `c6ec8551` | shape |
| B snapshot | `9fd52595` | shape |
| C extern-C | `4779717c` | shape |
| D polyglot | `893d02db` | shape |
| E TLS composition | `9cf6a3c4` | shape |
| F import trio | `0c69f0ba` | shape |
| report + e4-decisions S6 line | (this commit) | shape |
| book delta | `66eb93b7` | shape-web |

## Readiness for the ADR-009 completion gate

S6 closes the last E4 slice: the @remote acceptance matrix is real (16 green,
NON-VACUOUS), the residual 5 honestly deferred to a live issue (#83), the book
green where the gate can prove it. **S6 is READY for the ADR-009 completion gate**
(the 2026-07-16 run FAILED 3/3). The completion gate is the NEXT step and is NOT
run here.

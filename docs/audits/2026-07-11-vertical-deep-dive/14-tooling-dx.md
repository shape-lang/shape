# Vertical Deep-Dive Audit 14: Tooling & Developer Experience

**Auditor:** 14 of 19 · **Date:** 2026-07-11 · **Working tree:** dirty (audited as-is)
**Territory:** `bin/shape-cli` (all commands, REPL, TUI), `tools/shape-lsp`, `crates/shape-diagnostics` (LSDS), `tools/vmjit-diff`, `tools/xtask`, `editors/`, `shape-lsp-plugin/`, `tree-sitter-shape/`
**Binary under test:** `/home/dev/dev/shape-lang/shape/target/debug/shape` (reports `shape 0.3.2`)

All transcripts in this report were produced against the working tree on 2026-07-11.
Extension-loader lines (`Loaded module: python/typescript`, `Shape engine initialized`)
are filtered from transcripts unless relevant, per audit ground rules.

## 0. Executive summary

### Overall health verdict

The Tooling & DX vertical is in **substantially better shape than the 2026-07-04 audit's
picture of the runtime**: the CLI surface is broad and mostly real (26 subcommands; of the
ones I exercised end-to-end, 20+ work as documented), the REPL is solid with good error
recovery, the LSP is a genuinely large and richly-tested server (49.4k LOC, 763 unit
tests, ~25 advertised capabilities including formatting, rename, call hierarchy, and
child-LSP delegation into embedded Python blocks), and vmjit-diff is a well-engineered
differential harness that ran green (15/15 MATCH) against today's binary. Signing
(keys/sign/verify), snapshots (save/list/info/resume), build, doctest, expand-comptime,
check, serve, and wire-serve all demonstrably work.

The deficits are concentrated in three places. First, **diagnostics architecture is
split-brained**: LSDS is declared "the primary diagnostic format" (CLAUDE.md, ADR-006 §9)
but only three emitter families produce it; the CLI's human renderer bypasses the LSDS
terminal renderer entirely, and `--diagnostics json` reconstructs LSDS by string-parsing
`ShapeError` messages — a lossy LSDS→string→LSDS round-trip that drops file paths,
witnesses, and spans. Second, **`shape check` is broken for its primary audience**: it
cannot resolve imports, so every multi-file project fails with false "Undefined function"
errors, it reports only the first error, and it has no JSON output. Third, **grammar
tooling has silently rotted**: tree-sitter-shape has had one substantive commit ever and
lacks `extern`/polyglot/`out` constructs that the pest grammar gained, while the
`xtask grammar-parity` gate that should catch this is dead code referencing a nonexistent
crate (`shape-core`) and a nonexistent CLI subcommand (`shape parse`).

### Top findings

| # | Severity | Finding | Evidence |
|---|----------|---------|----------|
| 1 | P1 | `shape check` cannot resolve project imports — false "Undefined function" error on every multi-file project; `shape run` on the same project succeeds | §9.1 transcript; `check_cmd.rs:50-52` compiles entry AST with a bare `BytecodeCompiler`, no module loading wired |
| 2 | P1 | `xtask grammar-parity` is dead: default corpus `crates/shape-core/examples` doesn't exist; invokes `shape parse`, a subcommand that doesn't exist | `tools/xtask/src/main.rs:1098,1131-1137`; §9.4 transcript |
| 3 | P1 | tree-sitter grammar drift: no `extern C fn`, no `fn python/typescript`, no `out` params; 1 substantive commit vs continuously-evolved shape.pest | §5.3; `grammar.js` greps vs `shape.pest:46-47,303,462` |
| 4 | P1 | LSDS pipeline inverted: CLI human path uses legacy `ShapeError::format_with_source()`, not the LSDS terminal renderer; JSON path re-parses strings back into LSDS, losing file, span, expected/found | `script_cmd.rs:1613-1662`; `diagnostics_json.rs:1-9,100-111`; §6.2 |
| 5 | P1 | `shape run` diagnostics never carry the script filename — every error says `<input>`; LSDS JSON omits `file` entirely | §9.2 transcripts (all errors); breaks editor/CI error parsing |
| 6 | P1 | `[jit-fallback]` internal jargon (ADR citations, "γ-CP4", "SURFACE", "W36") leaks to stderr on ordinary programs (any `&arr[i]` borrow, any `fn main` project, `snapshot()`) | §9.3 transcripts |
| 7 | P2 | Type errors surface as `error[RUNTIME]` with triple-prefix "Bytecode compilation failed: Semantic error: …" under `--mode jit` (default); same error is `error[SEMANTIC]` in REPL — three different renderings of one error | §9.2 transcript |
| 8 | P2 | Multiple type errors collapse into one diagnostic anchored at line 1 (3 errors → "1 error(s)" in check; single caret at 1:1 in run) | §9.2 multi_err transcript |
| 9 | P2 | ADR-006 `var` storage-class inlay hint is an LSP-side heuristic re-implementation, self-documented as approximate — not wired to the authoritative compiler `BindingStorageClass`; drift risk by construction | `inlay_hints.rs:459,1025-1046`; §6.1 |
| 10 | P2 | Stale-domain residue across DX surfaces: REPL `:equity` "plot equity curve from last backtest", vscode tmLanguage highlights `alert`/`backtest`-era keywords, "Pro feature" message in repl_cmd.rs | §3.4; `repl_cmd.rs:62-64`; tmLanguage keyword list |
| 11 | P2 | `shape.toml` unknown sections silently ignored by CLI: `[package]` typo → `Building package '' v...` + `package-0.0.0.shapec` + `proj@0.0.0`, no warning — while the LSP's toml_support already implements exactly this validation | §9.9 transcript; `toml_support/diagnostics.rs:28,219` vs `project_config.rs:73-82` |
| 12 | P2 | Project mode exits 0 with zero program output when the entry defines `fn main` but never calls it — silent no-op, with only a misleading `[jit-fallback] function main failed JIT compile` on stderr | §9.10 transcript |

(The REPL print-swallowing defect (§9.5) is table-adjacent P1 severity — it is the #1
item in §12's P0 recommendations and was double-verified in §0.1.)

### Scores

**Feature completeness: 78/100.** Nearly every advertised command exists and most work
end-to-end (run/repl/build/sign/verify/keys/snapshot/resume/doctest/expand-comptime/
serve/wire-serve/jit-parity/ext/schema/tree all verified live), but check is broken on
projects, `.shapec` bundles can't be executed directly, LSDS coverage is ~3 of 7 planned
migration sessions, and the editor-grammar toolchain has drifted.

**Code quality: 74/100.** Idiomatic Rust, superb test density in the LSP (763 tests) and
honest self-documenting comments; but a 3,257-line server.rs, a 2,970-line serve_cmd.rs,
duplicated provider-options destructuring boilerplate 6× in main.rs, two commands that
`std::process::exit` instead of returning Result, and dead xtask code.

### Biggest risk

The biggest risk is **diagnostics-quality decay locking in**: every consumer (CLI human,
CLI JSON, REPL, check, LSP) renders errors through a different code path today, and the
LSDS crate that was supposed to unify them is consumed by only three emitter families
while the migration plan (docs/lsds-migration-plan.md, sessions 3–8) has stalled since
2026-05-08. Each new emitter site added in the meantime deepens the eventual migration.
Combined with the filename-less `<input>` locations and the `[jit-fallback]` jargon leaks,
the practical effect is that Shape's compile errors — the single most-touched DX surface —
are markedly worse in the shipped `shape run` default path than the underlying
infrastructure (carets, notes, codes, witnesses) already supports.

### 0.1 Re-verification pass (independent second pass, same day)

Every load-bearing claim in the top-10 table was independently re-reproduced from
scratch in a second pass on 2026-07-11 (fresh scratch programs, fresh transcripts):

- **`shape check` import breakage (finding 1)** — re-reproduced with a different
  two-file project (`from util use { double }`): `shape --mode vm` runs and prints
  `double(21)=42`; `shape check .` reports `error: Semantic error: Undefined function:
  'double' (./src/main.shape)`. Also confirmed check's exit codes are otherwise correct
  (1 on error, 0 on pass) and that a *broken* imported module and a *working* one
  produce the identical false error — consistent with no module resolution at all.
- **JIT-mode error wrapping (finding 7)** — re-reproduced: the same semantic error
  renders `error[RUNTIME]: Bytecode compilation failed: Semantic error: ...` under
  default `--mode jit` and `error[SEMANTIC]: ...` under `--mode vm`; wrapper origin
  confirmed at `crates/shape-jit/src/executor.rs:204`.
- **REPL print-swallowing (§9.5)** — re-reproduced byte-for-byte:
  `printf 'print("direct-print")\n1+1\n:quit\n' | shape repl` outputs only `2`.
  Root-cause cite re-read and confirmed: `ReplAdapter::print` returns
  `KindedSlot::none()` and the comment admits "the structured `PrintResult` is dropped
  until the schema lands" (`crates/shape-runtime/src/output_adapter.rs:70-89`).
- **`xtask grammar-parity` dead (finding 2)** — re-reproduced both legs:
  `shape parse hello2.shape` → `error: unexpected argument 'hello2.shape' found`;
  `ls crates/shape-core` → no such directory.
- **`shape serve` non-loopback refusal (§2.3)** — re-reproduced verbatim.
- **LSP suite (§7.2)** — re-run from the working tree:
  `cargo test -p shape-lsp --lib` → `767 passed; 0 failed; 0 ignored` (33.80s).
- **LSDS non-adoption (finding 4)** — re-greped: `shape_diagnostics::` consumers
  remain exactly three compiler families (`comptime_diagnostics.rs`,
  `functions_foreign.rs:773`, `functions.rs:1357-1427` borrow path) plus the CLI;
  `tools/shape-lsp` has **zero** references.

The second pass also surfaced two findings not in the first pass — shape.toml
unknown-section silence (§9.9) and project-mode silent no-op (§9.10).

## 1. Architecture & code structure map

### 1.1 Component inventory and LOC

| Component | Path | LOC | Role |
|---|---|---|---|
| shape-cli | `bin/shape-cli/src/` | 14,819 | CLI binary: 26 subcommands, line REPL, TUI notebook, extension loading, registry client |
| shape-lsp | `tools/shape-lsp/src/` | 49,420 | LSP server (tower-lsp-server over stdio) |
| shape-diagnostics | `crates/shape-diagnostics/` | 854 | LSDS schema crate + terminal/JSON renderers |
| vmjit-diff | `tools/vmjit-diff/` | 637 (mjs) + 469 corpus programs | VM-vs-JIT differential harness (Node) |
| xtask | `tools/xtask/src/main.rs` | 1,296 | Workspace automation (10 subcommands) |
| tree-sitter-shape | `tree-sitter-shape/grammar.js` | 1,340 | Editor grammar (generated parser in `src/`) |
| editors/ | `editors/vscode`, `editors/neovim` | 460 (ts+lua) | VSCode extension (vsix built), Neovim ftdetect/lspconfig/mason |
| shape-lsp-plugin | `shape-lsp-plugin/` | ~20 (json) | Claude Code LSP plugin manifest pointing at `shape-lsp` |

### 1.2 shape-cli structure

`bin/shape-cli/src/main.rs` (465 LOC) is a flat `match (command, file)` dispatcher over
`cli_args.rs` (536 LOC, clap-derive). Commands live in `commands/` (23 files). The largest:

- `serve_cmd.rs` — 2,970 LOC. In-process execution server (TLS, bearer auth, sandbox
  levels, `--ffi-languages` opt-in for transferred foreign code, max-concurrent gating).
  Verified to bind and print its security posture (§2 transcript).
- `script_cmd.rs` — 2,249 LOC. The `shape run` pipeline: project/front-matter detection,
  native-dependency lock construction from `.shapec` bundles (`script_cmd.rs:874-919`),
  engine setup for VM and JIT modes (`run_engine`, `script_cmd.rs:1558`), interrupt-save
  Ctrl-C snapshot handling (exit 130 path at `script_cmd.rs:1503-1512`), error rendering
  (`print_shape_error`, `script_cmd.rs:1613`).
- `repl_cmd.rs` — 855 LOC. rustyline line REPL with completer/hinter/validator helper,
  history in a dotfile, chart rendering hooks.
- `repl/` + `ui/` — 2,049 LOC. The ratatui TUI notebook (`shape tui`): cells, modal
  editing, progress display, snapshot checkpointing to `data_local_dir` (`tui_cmd.rs:59-64`).
- Registry commands (`register/login/publish/add/remove/search/info`) share
  `registry_client.rs` (559 LOC, reqwest against `https://pkg.shape-lang.dev`,
  `DEFAULT_REGISTRY` in `config/mod.rs`).

Entry-point semantics (verified): `shape <file>` runs a script; bare `shape` in a
directory with `shape.toml` runs `[project].entry`, otherwise launches the REPL
(`main.rs:400-439`). `--resume <hash>` without a file resumes a snapshot (`main.rs:386-398`).

### 1.3 shape-lsp structure

Single `ShapeLanguageServer` (server.rs, 3,257 LOC) with per-feature modules. Data flow:
`document.rs` (open/change tracking) → `shape_ast::parse_program` → `analysis.rs`
(`analyze_program_semantics`) which runs ~12 lint validators plus the **real bytecode
compiler** in `RecoverAll` mode (`analysis.rs:56-58` — `TypeDiagnosticMode::RecoverAll`,
`CompileDiagnosticMode::RecoverAll`) and maps `ShapeError`s to LSP diagnostics, capped at
200 (`analysis.rs:22`). Feature handlers (hover 3,218 LOC, completion/ 2,501+, semantic
tokens 2,864, inlay hints 2,273, definition 1,865, code actions 1,750, formatting 1,630,
signature help 1,270, rename 757, call hierarchy 957, code lens 464, folding 401) sit on
top of `type_inference.rs` (2,944 LOC), which wraps the compiler's real
`TypeInferenceEngine` (`type_inference.rs:19-20,931-934`) with LSP-side fallback
heuristics. `foreign_lsp.rs` (2,551 LOC) maps positions into embedded `fn python` bodies,
generates virtual documents, and manages child language servers (pyright) for delegated
hover/completions/diagnostics. `toml_support/` (2,097 LOC) gives `shape.toml` its own
schema-driven completions/hover/diagnostics. `module_cache.rs` (803 LOC) caches parsed
imports workspace-wide.

### 1.4 shape-diagnostics (LSDS)

`lib.rs` (471 LOC): `Diagnostic` (id, severity, location, expected/found `TypeWitness`,
`SuggestedFix` with confidence + optional diff, `ContextWindow` with cl100k token budget,
rule citation, notes), `DiagnosticBuilder`, process-wide `OutputFormat` atomic
(`set_output_format`/`output_format`, lib.rs:63-81). Renderers: `render/terminal.rs`
(181 LOC, snapshot-tested plain text) and `render/json.rs` (55 LOC, one-object-per-line).
`SCHEMA_VERSION = 1`. The planned LSP and MCP renderers (`render/lsp.rs`, `render/mcp.rs`
per docs/lsds-migration-plan.md sessions 6–7) do not exist.

### 1.5 vmjit-diff

Node harness (`run-diff.mjs`, 376 LOC): runs every corpus program under `--mode vm` and
`--mode jit`, diffs stdout bytes + exit code, classifies MATCH/VM_FAIL/JIT_FAIL/DIVERGED/
TIMEOUT, honors a `known-red.json` allowlist where every entry carries an audit citation,
and is resumable via a JSONL progress file pinned to binary + corpus. Corpus = 469
programs (book runnable fences + acceptance programs + synthetic repros), regenerated by
`build-corpus.mjs`. Baseline 2026-07-05: 466 MATCH / 1 known DIVERGED. Live run today
(15-program subset, debug binary): 15/15 MATCH, exit 0.

### 1.6 xtask

10 subcommands (`main.rs:111-126`): workspace-smoke, vmvalue (inventory/baseline/check —
ValueWord-deletion trend tracking), line-budget, benchmark-specialization, native-docs,
perf-regression-gate, migration-metrics, loc-check, grammar-parity (dead, §9.4), doctest.

## 2. Feature completeness

Method: every claim below tagged **[E2E]** was exercised live against
`target/debug/shape` (built 2026-07-11 10:04 from this working tree); **[CODE]** means
implementation read but not executed; **[STALE-BIN]** means exercised against the Apr-29
`shape-lsp` build (protocol-level only).

### 2.1 CLI commands — verified working end-to-end

| Command | Status | Evidence |
|---|---|---|
| `shape run <file>` (jit + vm) | **WORKS** [E2E] | hello.shape prints `answer: 42` in both modes, exit 0 |
| `shape <file>` (implicit run) | **WORKS** [CODE+E2E] | `main.rs:367-383` file-mode arm; same engine path |
| bare `shape` project mode | **WORKS** [CODE] | `main.rs:400-439` — runs `[project].entry`, clear error if entry missing |
| `shape repl` | **WORKS with P1 caveat** [E2E] | expression echo, fn defs, binding persistence, error recovery all good; **`print()` output silently discarded** (§9.5) |
| `shape tui` | CODE EXISTS, not drivable headless | ratatui notebook (`repl/mod.rs`, 2,049 LOC); requires TTY |
| `shape check <file>` | **WORKS single-file / BROKEN multi-file** [E2E] | passes/fails correctly on one file (exit 0/1); false "Undefined function" on projects with imports (§9.1) |
| `shape check --link` | **WORKS** [E2E] | `check_cmd.rs:57-71` — `eager_link_all()` over loaded program |
| `shape build` | **WORKS** [E2E] | produced `audit-proj-0.1.0.shapec` (240,478 bytes) from minimal project |
| `shape keys generate/trust/list` | **WORKS** [E2E] | Ed25519 keypair generated, public key printed |
| `shape sign` / `shape verify` | **WORKS** [E2E] | signed 1 manifest; verify: `integrity=ok, signature=valid` |
| `shape snapshot list/info/delete` | **WORKS** [E2E] | v7 snapshot listed, info shows hash/version/VM-state, delete removes; prefix matching works; missing hash → clean error with next-step suggestion |
| `shape run --resume <hash>` / `shape --resume` | **WORKS** [E2E] | resumed snapshot, printed `a=10` |
| `shape doctest <md>` | **WORKS** [E2E] | 2-fence file: 1 pass 1 fail, failure listed with line + code |
| `shape expand-comptime` / `run --expand` | **WORKS** [E2E] | post-comptime function report; C0002 warning surfaced |
| `shape jit parity [--builtins]` | **WORKS** [E2E] | 185/200 opcodes JIT-supported; per-row reason strings (async family VM-only with book-truth-100 citation) |
| `shape tree` | **WORKS** [E2E] | correct error outside project; `audit-proj@0.1.0` inside |
| `shape ext list` | **WORKS** [E2E] | installed + available table |
| `shape ext install/remove` | CODE EXISTS [CODE] | `ext_cmd.rs` (180 LOC) — cargo-based source build; not exercised (would hit crates.io + long build) |
| `shape schema status` | **WORKS** [E2E] | "No data-source schema cache found in shape.lock" |
| `shape schema fetch` | CODE EXISTS [CODE] | needs a live data provider (duckdb); not exercised |
| `shape search <q>` | **WORKS** [E2E] | live registry query returned "No packages found matching 'json'"; network errors exit 1 (`search_cmd.rs:43-46`) |
| `shape register/login/publish/add/remove/info` | CODE EXISTS [CODE] | `registry_client.rs` (559 LOC) real HTTP client; not exercised (would mutate the live registry) |
| `shape serve` | **WORKS (binds + posture)** [E2E] | listens, prints `sandbox: Strict, max-concurrent: 4, auth: none`, `granted: [] (pure — no I/O)`; **refuses non-loopback without TLS** (live transcript §2.3) |
| `shape wire-serve` | **WORKS (binds)** [E2E] | `Shape wire-serve listening on 127.0.0.1:19534` |
| `shape run --max-instructions/--max-time-ms/...` | **WORKS** [E2E] | runaway loop killed at cap, exit 1, in both vm and jit modes |
| `shape run --eager-link` | **WORKS** [E2E] | missing `libdoesnotexist.so` fails up-front with a full error list; lazy default runs fine when fn never called |
| `shape run --diagnostics json` | **WORKS with quality caveats** [E2E] | one LSDS JSON object per line on stderr; but degraded content (§6.2, §9.2) |

### 2.2 CLI gaps (missing / broken)

- **`shape run bundle.shapec` unsupported** [E2E]: `Error: failed to read ... stream did
  not contain valid UTF-8`. `execute_file` reads UTF-8 source only
  (`script_cmd.rs:1473-1475`); `.shapec` is only consumable as a *dependency*
  (`script_cmd.rs:874-878`). You can build, sign, verify, and publish a bundle but not
  execute one directly — the build→run loop is open-circuit.
- **No `shape fmt`**: the LSP has a 1,630-LOC formatter (`formatting.rs`) but there is no
  CLI formatting entry point; non-LSP users (CI, pre-commit) can't format.
- **No `--diagnostics json` on `check` or file-mode `shape <file>`**: the flag exists only
  under `shape run` (`cli_args.rs:398-408` vs `Check` at `cli_args.rs:316-324`); the
  natural CI surface (`check`) has no machine output.
- **`shape check` has no multi-error reporting**: single `compile()` call, first error
  only (`check_cmd.rs:50-52`), unlike the LSP which uses `RecoverAll` — same compiler,
  stricter surface for the tool most likely to be scripted.
- **Resource-limit flags absent from file mode**: `shape foo.shape --max-time-ms ...` is
  not accepted; only the `run` subcommand takes `ResourceLimitOptions` (`main.rs:371-380`
  hardcodes `ResourceLimits::unlimited()`).

### 2.3 Security-posture transcript (serve)

```text
$ shape serve --address 0.0.0.0:19555
Error: Refusing to start on non-loopback address 0.0.0.0:19555 without TLS.
Provide --tls-cert and --tls-key, or bind to 127.0.0.1.
```
Non-loopback additionally requires `--auth-token` (refusal, not warning —
`serve_cmd.rs:313-330`). This matches the ratified distributed §4.7 posture.

### 2.4 LSP feature truth

Capabilities registered at initialize (`server.rs:548-744`): completion+resolve, hover,
signature help, document/workspace symbols, definition, declaration, type definition,
implementation, document highlight, references, semantic tokens, inlay hints+resolve,
code actions (options), formatting (full/range/on-type), rename+prepare, code lens+resolve,
folding, call hierarchy, document links, execute command, pull diagnostics.

- **Diagnostics [STALE-BIN, protocol-verified]**: `didOpen` with `let x: int = "hello"`
  produced `publishDiagnostics` with severity 1, code `SEMANTIC`, message "Could not
  solve type constraints: int is not compatible with string". So the wire protocol and
  the compile-based diagnostics pipeline function. Range quality is weak: the returned
  range was `0:0..0:100` — a whole-line sentinel (character 100 on a 20-char line).
- **Diagnostics architecture [CODE]**: real `BytecodeCompiler` in
  `RecoverAll` mode + ~12 semantic validators + doc-comment validators + 200-diagnostic
  cap (`analysis.rs:22-90`). This is the correct architecture (no reimplemented checker
  for diagnostics).
- **Hover/completion/inlay [CODE]**: built on the compiler's real `TypeInferenceEngine`
  (`type_inference.rs:19-20,931,1287,1662`) with LSP-side heuristic fallbacks.
- **Foreign-block delegation [CODE]**: `foreign_lsp.rs` (2,551 LOC) maps positions into
  `fn python` bodies, spawns pyright, forwards hover/completion/diagnostics. 16 unit
  tests; end-to-end unverifiable here (no pyright installed).
- **shape.toml support [CODE]**: dedicated schema, completions, hover, diagnostics
  (`toml_support/`, 2,097 LOC).
- **Working-tree test suite**: see §7 (763 `#[test]`s; suite run initiated — result
  recorded in §7).

### 2.5 Grammar/editor assets

- **tree-sitter-shape**: builds via `Makefile`/npm; grammar drift confirmed (§5.3).
  Queries dir exists. Only substantive update since initial commit: TypePath migration
  (28f4a2c1).
- **VSCode extension**: `shape-lang-0.1.0.vsix` committed (but package.json says 0.1.5 —
  the committed vsix is 5 patch levels stale); TextMate grammar + LSP client
  (`extension.ts`, 215 LOC).
- **Neovim**: ftdetect + native `vim.lsp.start` snippet + mason registry entry pointing
  at `pkg:cargo/shape-lsp` (`editors/neovim/mason/package.yaml`).
- **shape-lsp-plugin**: Claude Code plugin manifest, passes `args: ["serve"]` to
  `shape-lsp` — the binary ignores all args except `--version` (`tools/shape-lsp/src/main.rs:12`),
  so the "serve" arg is a no-op that works by accident.

### 2.6 Additional command-level probes [all E2E]

- **`shape expand-comptime --function <name>`** filter works: a 2-function file filtered
  to `keep` reports exactly `Functions (post-comptime): 1 / fn keep(x: int) -> int` and
  echoes `filter function: keep`.
- **`shape jit parity --builtins`**: `Opcodes: 185/200 JIT-supported` +
  `Builtins: 186/186 JIT-supported` — the builtin surface claims full JIT parity; the 15
  VM-only opcodes are all in the async/event family with an explicit contract citation
  per row.
- **`shape doctest` fence modifiers**: fences are selected by language tag with
  comma-separated modifiers `shape,should_fail` and `shape,ignore`
  (`doctest_cmd.rs:64-70`) — so the doctest harness supports negative tests, which the
  book gate relies on.
- **Registry auth storage**: `config/mod.rs` persists the login token under the
  user-config dir (override via `SHAPE_CONFIG_DIR`, test-locked; §3.2) — no plaintext
  token ever passes through argv except at `shape login --token` itself (which is
  unavoidable but worth a docs warning about shell history).
- **REPL suggestion quality**: undefined names produce ranked did-you-mean suggestions
  ("Did you mean 'min', 'max', or 'map'?") — the resolution-order explanation sentence
  is verbose but informative.

### 2.7 REPL in non-interactive (piped) mode — multiline input breaks

The rustyline `Validator` gives interactive users brace-aware continuation, but piped
stdin is evaluated line-by-line, so a multiline function definition — the first thing a
script or an LLM agent driving `shape repl` will send — explodes into three cascading
errors:

```text
$ printf 'fn mul(a: int, b: int) -> int {\n    a * b\n}\nmul(6, 7)\n:quit\n' | shape repl
error: Parse error: Syntax error near: -> int {
error[E0101]: Undefined variable: 'a'
   1 |     a * b
error[E0001]: unexpected `}`, expected something else
error[RUNTIME]: Undefined function: mul. ... Did you mean 'min', 'max', or 'map'?
```

Note the transcript also reveals a **fifth error-rendering flavor** (§4.5 counts four):
the REPL's structured-parse path prints colored `error[E0001]` with blue gutter pipes —
so within one REPL session a user can see `error: Parse error: ...` (plain, no location),
`error[E0101]` (caret, no color), and `error[E0001]` (caret, ANSI-colored) for three
lines of the same snippet. Each is a different code path in the same binary.

## 3. Code quality

### 3.1 Idiom & error handling

- clap-derive CLI with doc-comment help text is clean and self-documenting
  (`cli_args.rs` throughout); flag docs even cite the book and ADR sections.
- anyhow with `.context()` used consistently in shape-cli; `ShapeError` downcast at
  boundaries (`script_cmd.rs:1507-1511,1616`).
- **Inconsistency**: `run_search` / `run_info` return `()` and call
  `std::process::exit(1)` internally (`search_cmd.rs:43-46`), while all sibling commands
  return `Result` to `main()` (see `main.rs:306-311` — the only two call sites without
  `?`). `check_cmd.rs:99` and `script_cmd.rs:1527` also `process::exit` mid-function,
  skipping destructors — acceptable for a CLI but three different exit conventions in
  one binary.
- `check_cmd.rs:34` declares `let warnings = 0u32;` that is never incremented — "0
  warning(s)" is unconditional; the summary line implies a warning channel that does
  not exist.
- Unconditional ANSI escapes in `check_cmd.rs:63-97` (no TTY detection) — piped/CI
  output gets raw `\x1b[31m` bytes (visible in every captured transcript above).

### 3.2 Unsafe usage

5 `unsafe` sites in the whole territory (grep over shape-cli, shape-lsp,
shape-diagnostics, xtask):

- `bin/shape-cli/src/config/mod.rs:93,103,106` — `std::env::set_var/remove_var` in
  **test-only** code, guarded by a process-wide mutex (`SHAPE_CONFIG_DIR_LOCK`). Justified.
- `bin/shape-cli/src/commands/script_cmd.rs:1115,1689` — `libloading::Library::new`
  (dlopen probing for `check --link` / extension resolution). Inherently unsafe API;
  justified.

No unjustified unsafe. shape-lsp and shape-diagnostics contain zero unsafe.

### 3.3 Complexity hotspots

| File | LOC | Note |
|---|---|---|
| `tools/shape-lsp/src/server.rs` | 3,257 | one god-struct impl; a 310-line stretch before line 785 is the longest inter-fn gap |
| `tools/shape-lsp/src/hover.rs` | 3,218 | hover string assembly for every symbol kind |
| `bin/shape-cli/src/commands/serve_cmd.rs` | 2,970 | server + protocol + sandbox + tests in one file |
| `tools/shape-lsp/src/type_inference.rs` | 2,944 | engine wrapper + heuristics + metadata |
| `bin/shape-cli/src/commands/script_cmd.rs` | 2,249 | run pipeline + native-dep locks + rendering + tests |

None of these are decomposed into submodules despite each having clear internal seams
(serve_cmd: protocol/auth/execution; script_cmd: project-resolution/engine/rendering).

### 3.4 Dead code & stale-domain residue

- `#[allow(dead_code)]` only 3× in territory (2 in `add_cmd.rs` on response fields, 1
  test-only in `code_lens.rs:246`) — low.
- **`xtask grammar-parity` is fully dead** (§9.4): nonexistent corpus default
  (`main.rs:1098` → `crates/shape-core/examples`; no such crate — verified `ls crates/`)
  and nonexistent subcommand (`main.rs:1131-1137` runs `shape parse`, which clap rejects:
  "unexpected argument ... found").
- **Trading-era residue**: REPL `:help` advertises `:equity` — "plot equity curve from
  last backtest" and `:metrics` (`repl_cmd.rs:679-681`); the vscode TextMate grammar
  highlights `alert|optimize|select|order|group|backtest`-era words plus non-keywords
  `interface`, `method`, `module`, `test|it|should|expect|setup|teardown`
  (`shape.tmLanguage.json` keyword rules) — none of these are in the pest
  `item_sync_keyword` set (`shape.pest:22`). "This is a Pro feature" appears in
  `repl_cmd.rs:63` (unreachable today since `jit` is a default feature —
  `bin/shape-cli/Cargo.toml:13` — but stale monetization copy in an OSS repo).
- Doc-comment on `Repl::bootstrap_with_options` claims chart/equity support that depends
  on `shape-viz-core` — functional but domain-specific leftovers from the
  market-data era of the project.

## 4. Duplication & DRY violations

### 4.1 Module-path + front-matter setup duplicated inside script_cmd.rs

The resume path (`script_cmd.rs:339-379`) copies the module-path canonicalization,
project-root detection, dependency resolution, and front-matter module-path application
verbatim from `execute_file` (`script_cmd.rs:1452-1496`) — the copy even carries the
comment "same as execute_file". Any change to project detection must now be made twice;
a missed edit silently makes resumed scripts resolve modules differently from fresh runs.
**Divergence risk: high** (this is exactly the code that decides which files an import
statement finds).

### 4.2 REPL `execute_file` is a third, degraded copy

`repl_cmd.rs:209-226` re-implements "run a file" a third time for `:load`, but only adds
the parent dir to module paths — **no project-root detection, no
`resolve_project_dependencies`, no front-matter parsing**. So `:load` inside a project
behaves differently from `shape run` on the same file. Combined with the ReplAdapter
print-swallowing bug (§9.5), `:load` on our hello.shape produced zero output where
`shape run` prints `answer: 42`.

### 4.3 ExecutionModeArg → ExecutionMode conversion, 3 copies

The identical `match mode { Vm => BytecodeVM, Jit => cfg!(jit)... }` block (including
the `#[cfg(not(feature = "jit"))]` bail with slightly different message text each time)
appears at `repl_cmd.rs:53-67`, `tui_cmd.rs:24-35`, and `script_cmd.rs:156-168`. One
copy says "This is a Pro feature."; the others don't. Textbook drift already visible.

### 4.4 Provider-options destructuring boilerplate in main.rs

`let cli_args::ProviderCommandOptions { extensions, providers_config, extension_dir } =
provider;` + rebuild into `ProviderOptions` occurs **6 times** in `main.rs`
(Run/Repl/Tui/Schema/WireServe/Serve arms). A `From<ProviderCommandOptions> for
ProviderOptions` impl would delete ~60 lines. Low risk, pure noise.

### 4.5 Error rendering: four independent surfaces

The same `ShapeError` renders through four unrelated code paths:

1. `shape run` human: `print_shape_error` → `ShapeError::format_with_source()` /
   `CliErrorRenderer` (`script_cmd.rs:1613-1662`);
2. `shape run --diagnostics json`: `anyhow_to_diagnostics` string-parsing bridge
   (`diagnostics_json.rs:19-98`);
3. REPL: its own error branch inside `run_engine`/`print_execution` (renders
   `error[SEMANTIC]` where run-mode prints `error[RUNTIME]` for the identical program —
   §9.2 transcripts);
4. `shape check`: bare `eprintln!("error: {e} ({path})")` (`check_cmd.rs:75-83`), losing
   line/col/caret entirely.

Divergence is not hypothetical — it is visible today in the three different renderings
of `let x: int = "hello"` (§9.2). **This is the concrete cost of the stalled LSDS
migration** (§6.2).

### 4.6 Line REPL vs TUI REPL

`bin/shape-cli/src/commands/repl_cmd.rs` (855 LOC, rustyline) and `bin/shape-cli/src/repl/`
(2,049 LOC, ratatui) are two full REPL implementations sharing only `ShapeEngine` and
`extension_loading`. Both maintain separate command sets (`:help/:load/:plot/...` vs
modal `:q/clear`), separate result rendering, separate history. The book documents only
the TUI — under the name of the other one (§8.2).

## 5. Split-brain analysis

### 5.1 LSDS as designed vs LSDS as wired (doc-vs-code)

Design (ADR-006 §9, `shape-diagnostics/src/lib.rs:3-6`): "LSDS is the source of truth —
text strings, LSP `Diagnostic` payloads, and MCP tool responses are all derived from it."
Reality: the compiler builds LSDS for exactly three families (B-series borrow errors via
`borrow_error_to_lsds` in `crates/shape-vm/src/compiler/functions.rs`; comptime
C0001/C0002 in `comptime_diagnostics.rs`; foreign-fn checks in `functions_foreign.rs:782`)
— then immediately **flattens to a `ShapeError` string** via the `diagnostic_to_shape_error`
bridge, and the CLI **re-parses the string back into LSDS** for `--diagnostics json`
(`diagnostics_json.rs:5-9`: "they reach the CLI as ShapeError... maps a surfaced
ShapeError back to the Diagnostic shape"). The `split_leading_code` function
(`diagnostics_json.rs:85-98`) literally regexes `[C0001]`-style prefixes out of message
strings to recover the diagnostic id. Everything not carrying a bracket prefix gets a
variant-name pseudo-id (`RUNTIME`, `SEMANTIC`, `TYPE` — `diagnostics_json.rs:61-79`).
The LSP never sees LSDS at all (no `render/lsp.rs`; `analysis.rs` maps `ShapeError` →
`ls_types::Diagnostic` directly). Two representations of "the diagnostic" flow through
the system in opposite directions.

### 5.2 `BindingStorageClass`: compiler vs LSP reimplementation

ADR-006 §2 promises the `var` storage class "surfaces via LSP inlay hint". The compiler's
authoritative classification lives in the MIR storage planner
(`crates/shape-vm/src/type_tracking.rs:286`). The LSP does **not** call it — it ships a
parallel heuristic classifier (`inlay_hints.rs:1025-1046`: async-shape → SharedAtomic(Mut),
closure-capture → SharedCow, primitive → Direct, other heap → UniqueHeap) and honestly
labels its own output "`LSP-side approximation of BindingStorageClass (ADR-006 §2). The
compiler's bytecode pass at crates/shape-vm/src/type_tracking.rs:286 is authoritative.`"
(`inlay_hints.rs:459`) with an `[… approx]` suffix in the hint. Self-aware, but still a
split brain: when the storage planner changes (e.g. the ADR-006 §2.7.30 escape-promotion
rules), the hints drift silently — there is no test tying the two together.

### 5.3 Three grammars for one language

| Surface | File | `extern C fn` | `fn python`/`fn ts` | `out` param | `join all` | maintained? |
|---|---|---|---|---|---|---|
| pest (authoritative) | `crates/shape-ast/src/shape.pest:46-47,303,462` | yes | yes | yes | yes | continuously |
| tree-sitter | `tree-sitter-shape/grammar.js` | **no** (0 greps for `extern`) | **no** | **no** | yes (`grammar.js:1067`) | 1 substantive commit ever (28f4a2c1) |
| TextMate (vscode) | `editors/vscode/syntaxes/shape.tmLanguage.json` | **no** | **no** | **no** | keyword-only | release bumps only |

The designed drift-catcher, `xtask grammar-parity`, is dead (§9.4), so this table can
only get worse. Note the LSP avoids this trap entirely — `grammar_completion.rs:1-30`
derives completion candidates from pest parse errors ("ensures the LSP is always in sync
with the actual language syntax"), which is the right pattern and could in principle be
extended to the other two surfaces.

### 5.4 REPL identity split (doc-vs-code)

The book page `getting-started/repl.mdx` describes `shape repl` as "a notebook-style TUI
built on ratatui — not a plain stdin-line prompt", with modal editing, cell gutters, and
`:q`/`clear` commands. In the shipped binary `shape repl` is exactly the plain
stdin-line rustyline prompt the book denies, and the ratatui notebook is `shape tui`
(`main.rs:166-193`). Every keybinding table in that book page is unreachable from the
command it documents. (Meanwhile CLAUDE.md's own command table lists `repl` and TUI
separately and correctly.)

### 5.5 `jit parity` reason strings vs JIT reality

`shape jit parity` hardcodes per-opcode prose reasons (e.g. the async family's
"book-truth-100 async contract ... jit_join_init returns TAG_NULL and jit_cancel_task is
an extern-C todo"). These strings describe implementation state that lives in shape-jit
and will silently rot when the JIT lands async lowering; there is no mechanism tying the
matrix rows to the translator's actual dispatch table beyond hand-maintenance. Low
severity today (the tool is diagnostics-only), but it is the same doc-vs-code pattern.

### 5.6 Snapshot-store location: flag vs TUI

The global `--snapshot-store` flag / `SHAPE_SNAPSHOT_STORE` env (`cli_args.rs:42-44`) is
honored by run/snapshot subcommands (verified E2E), but the TUI hardcodes
`data_local_dir()/shape/snapshots` (`tui_cmd.rs:59-64`) and ignores both the flag and the
env var. Snapshots taken in the TUI are invisible to `shape snapshot list` under a
configured store.

## 6. ADR & spec conformance

### 6.1 ADR-006 rules binding this territory

| Rule | Verdict | Evidence |
|---|---|---|
| §9 "LSDS is the primary diagnostic format; renderers consume LSDS" | **PARTIAL / INVERTED** | 3 emitter families produce LSDS then downgrade to strings; CLI JSON path reconstructs LSDS from strings (`diagnostics_json.rs:5-9`); human path bypasses LSDS renderer (`script_cmd.rs:1613-1662`); LSP consumes `ShapeError`, not LSDS |
| §9.2 diagnostic JSON shape | **CONFORMS** | `Diagnostic` fields match; serde round-trip + snapshot tests green (14/14, live run §7.3) |
| §9.3 type witnesses (`expected`/`found`) | **SCHEMA ONLY** | fields exist (`lib.rs:152-176`); no emitter populates them — grep finds `.expected(` only in tests; live JSON output for a type error carries neither (§9.2 transcript) |
| §9.4 suggested-fix diffs | **NOT IMPLEMENTED** | `SuggestedFix.diff` populated nowhere outside tests (migration plan session 8, not started) |
| §9.5 context windows (token-budgeted) | **NOT IMPLEMENTED** | `ContextWindow::empty()` only; no tokenizer dependency exists in the crate (`Cargo.toml` has serde+serde_json only) |
| §13.5 "≥95% of compiler errors emit LSDS with expected/found and ≥1 fix" | **NOT MET, not close** | E-series type/semantic errors (~252 sites per `docs/lsds-migration-plan.md`) still string-based; measured live: `let x: int = "hello"` yields `diagnostic_id: "RUNTIME"`, no witnesses, `fixes: []` |
| §1.3 / §2 `var` storage-class surfaced via LSP inlay hint | **APPROXIMATE** | hint exists, config-gated (`shape.inlayHints.bindingStorageClass.enable`, `inlay_hints.rs:141-151`), but heuristic reimplementation, self-labelled approx (§5.2) — not the compiler's `BindingStorageClass` |
| §2.7.5 amendment: `--trace-jit` EnvFilter replaces SHAPE_JIT_* env vars | **CONFORMS** | `cli_args.rs:54-70` (feature-gated flag), `main.rs:39-77` (subscriber ordering vs env_logger handled explicitly) |
| LSDS stability contract (add-only fields) | **CONFORMS** | `SCHEMA_VERSION=1` pinned by test (`lib.rs:410-412`, `lsds_round_trip.rs::schema_version_pinned`) |

### 6.2 The migration-plan ledger (docs/lsds-migration-plan.md vs working tree)

- Session 1 (B-series borrow) — **DONE** (as documented).
- Session 2 (MutabilityError + StructuredParseError → LSDS) — **NOT DONE**: parse errors
  still render as bare `Error: Parse error: Syntax error near: =` with no location
  (§9.2c transcript); `StructuredParse` has its own `CliErrorRenderer` path outside LSDS
  (`script_cmd.rs:1630-1633`).
- Sessions 3–5 (type-system, semantic, runtime families) — **NOT DONE** (measured above).
- Session 6 (LSP renderer) — **NOT DONE**: no `render/lsp.rs` exists.
- Session 7 (MCP renderer + tokenizer) — **NOT DONE**.
- Session 8 (fix diffs) — **NOT DONE**.
- Beyond plan: comptime C-series and foreign-fn diagnostics went LSDS (not in the
  original plan's sessions) — real progress, but the plan document itself is stale about
  it.

### 6.3 Forbidden patterns (CLAUDE.md)

Grep across the whole territory for `ValueWord`, `value_word`, `synthesize_value_word`,
`is_tagged`, `SlotKind::Dynamic`, `dynamic_fallback`, `tag_bits`: **zero hits** in
shape-cli, shape-lsp, shape-diagnostics (xtask's `vmvalue` subcommand family is the
deletion-*tracking* tool, not a use). `KindedSlot` never leaks into shape-cli (0 hits) —
the CLI talks to the engine via `WireValue`, respecting the carrier boundary. No
forbidden renames or bridge vocabulary found in territory code or comments. **CONFORMS.**

### 6.4 ADR-005

No slot construction happens in this territory (CLI/LSP are above the `Arc<HeapValue>`
line); the single-discriminator rule is not exercised here. The two `// ADR-006` marker
comments in territory (`shape-diagnostics/lib.rs:143`, `inlay_hints.rs:39`) are accurate
citations, not touchpoints requiring conformance work.

## 7. Test coverage in-territory

### 7.1 Counts

| Component | Unit tests | Notes |
|---|---|---|
| shape-lsp | **767** (`cargo test -p shape-lsp --lib`, live run below) | densest: hover_tests 86, completion 58, inlay_hints 56, context 56, semantic_tokens 54, diagnostics 48 |
| shape-cli (src) | 91 `#[test]`/`#[tokio::test]` | script_cmd 18, serve_cmd 11, doctest_cmd 11, config 10, registry_client 7 |
| shape-cli (tests/cli) | 19 | script_execution 8 (incl. VM/JIT parity assertions), jit_fallback_diagnostic_matrix 8, tree 3 |
| shape-diagnostics | 14 | schema round-trip, renderer snapshots, version pin |
| vmjit-diff | n/a (is itself a test harness) | 469-program corpus; known-red pins carry audit citations |
| xtask | 0 for grammar-parity (which is broken) | |
| tree-sitter-shape | corpus not present in-repo beyond `queries/`; no parity gate | |

### 7.2 Live suite runs (this audit, working tree)

```text
$ cargo test -p shape-lsp --lib
test result: ok. 767 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 38.17s

$ cargo test -p shape-diagnostics
9 passed (lib) + 5 passed (tests/lsds_round_trip.rs); 0 failed
```

**Zero `#[ignore]`** in shape-lsp and shape-diagnostics. shape-cli's polyglot-serve
tests use an explicit skip-cleanly-with-note pattern instead of `#[ignore]`
(`serve_cmd.rs:2579-2586`: "close the CI-coverage gap without an #[ignore] that silently
runs nothing" — tests skip with a printed note when the extension `.so` is absent and
run whenever present). That is a better-than-baseline convention.

### 7.3 Assertion quality

Spot-checks: `script_execution.rs` asserts exact stdout equality across `--mode vm` and
`--mode jit` for the same script (e.g.
`test_xml_stringify_empty_children_parity_vm_jit`,
`test_unresolvable_empty_array_is_clean_compile_error_both_modes`) — real behavioral
assertions, not smoke. `jit_fallback_diagnostic_matrix.rs` (8 tests) pins the
`[jit-fallback]` stderr contract. hover_tests and diagnostics tests in the LSP assert
message substrings + positions. `diagnostics_json.rs` tests assert code extraction,
severity, line numbers, and note propagation (`diagnostics_json.rs:127-161`).

### 7.4 Gaps

- **No test renders a full CLI human diagnostic end-to-end** (the `error[RUNTIME]:
  Bytecode compilation failed: Semantic error:` triple-prefix of §9.2 would fail any
  reasonable golden test — none exists).
- **No test runs `shape check` against a project with imports** (the §9.1 breakage would
  have been caught by a single fixture).
- **No REPL-level test executes `print()`** (the §9.5 output-swallowing regressed
  invisibly; `ReplAdapter` has no unit test asserting its print output goes anywhere).
- **No pest↔tree-sitter parity test** (the tool for it is dead, §9.4).
- LSP integration tests are unit-level only (handlers called as functions); there is no
  in-repo JSON-RPC-over-stdio harness — my protocol smoke test (§2.4) appears to be the
  first time the stale binary answered a wire request in months.

## 8. Book/docs vs reality for this vertical

### 8.1 What the book gets right

- `advanced/developer-tools.mdx:358-359` honestly labels structural search
  (`shape search --signature`) as "a planned feature. The current `shape search` CLI is
  the registry package search" — matches measured behavior exactly.
- `getting-started/editor-setup-vscode.mdx` / `editor-setup-neovim.mdx` describe the LSP
  feature set (completions, diagnostics, hover, go-to-definition, semantic tokens) —
  all of which are registered capabilities and covered by unit tests.
- `tooling/execution-server.mdx` documents `shape serve`; the binary's startup banner and
  security refusals match (§2.3).

### 8.2 What the book gets wrong

- **`getting-started/repl.mdx` documents the wrong command** (§5.4): "The Shape REPL is a
  notebook-style TUI built on ratatui — not a plain stdin-line prompt", `shape repl`
  start instruction, full modal keybinding tables. Reality: `shape repl` IS the plain
  stdin-line prompt; the described TUI is `shape tui`. Every workflow on that page fails
  under the command it names (typing `i` in the line REPL types the letter i).
  Additionally the page's "typical session" relies on result echo, which works — but any
  tutorial using `print()` in the REPL breaks against §9.5.
- **CLAUDE.md claims an `export` keyword** ("Modules: `import`, `export`, `mod`, `use`").
  The pest grammar has no `export` statement — visibility is `pub`
  (`shape.pest:78-94`); a file using `export fn` produces the unrelated error
  "Undefined variable: export" anchored at the *importing* file (§9.2 transcript).
- **`--mode jit` flag doc promises `[jit-fallback]` diagnostics only "on JIT-compile
  failure"** (`cli_args.rs:78-86`) — accurate as far as it goes, but neither the flag
  doc nor the book's jit page prepares a user for internal-audit vocabulary ("γ-CP4",
  "Route A surface-and-stop", ADR §-numbers) appearing on a hello-world project (§9.3).
- **VSIX drift**: `editors/vscode/package.json` says 0.1.5 but the committed artifact is
  `shape-lang-0.1.0.vsix` — anyone side-loading the checked-in vsix gets a 5-versions-old
  extension.
- `editor-setup-vscode.mdx` instructs `cargo install shape-lsp` (crates.io) — not
  verifiable from this sandbox; flagged for the release-vertical auditor to confirm the
  published crate is current, given the local `shape-lsp` build here was 10 weeks stale.

### 8.3 CLAUDE.md/docs claims verified true

- "LSDS is the primary diagnostic format" — aspirationally stated in CLAUDE.md ADR-006
  §list; the ADR itself and the migration plan are more honest about phase status. Code
  reality is §6.2.
- CLAUDE.md's command table (`shape run/repl/wire-serve/ext install`) — all exist and
  respond correctly.
- `docs/codebase-index.md` tooling entries spot-checked accurate (shape-lsp path,
  CLI path, LSDS crate path).

## 9. Bugs & correctness risks found

Severity scale: P0 unsound/wrong-results/security · P1 broken feature · P2 paper cut.

### 9.1 P1 — `shape check` false-errors every multi-file project

```text
$ cat src/main.shape        # project with shape.toml, entry src/main.shape
from util use { helper }
fn main() { print(helper()) }
main()
$ cat src/util.shape
pub fn helper() -> int { 42 }

$ shape run src/main.shape   # WORKS
42
$ shape check                # FALSE ERROR
error: Semantic error: Undefined function: 'helper' (.../src/main.shape)
check failed: 1 error(s), 0 warning(s)
```

Root cause: `check_cmd.rs:50-52` compiles the entry AST with a bare
`BytecodeCompiler::new()` and never wires module loading (contrast `shape run`, which
calls `crate::module_loading::wire_vm_executor_module_loading` — `script_cmd.rs:1579`).
The command whose entire purpose is CI validation cannot validate any program with an
import. Also: first-error-only (no `RecoverAll`), no JSON output, hardcoded
`warnings = 0`.

### 9.2 P1/P2 cluster — diagnostics quality defects (all transcripts live)

**(a) P1 — no filename, ever, from `shape run`.** Every compile/runtime diagnostic
prints `--> <input>:LINE:COL`; the LSDS JSON omits `file` entirely:

```text
$ shape run type_err.shape
error[RUNTIME]: Bytecode compilation failed: Semantic error: Could not solve type constraints:
  string is not compatible with int
  --> <input>:1:1
$ shape run type_err.shape --diagnostics json
{"diagnostic_id":"RUNTIME","severity":"error","location":{"line":1,"col":1,"span":[0,0]},...}
```

Editors/CI cannot map errors to files in multi-file programs. (`shape check` does print
the path, but without line/col — the two commands have complementary halves of one
usable location.)

**(b) P2 — same error, three renderings.** `let x: int = "hello"`:

| Surface | Rendering |
|---|---|
| `shape run` (default jit) | `error[RUNTIME]: Bytecode compilation failed: Semantic error: Could not solve type constraints: ...` |
| `shape run --mode vm` | `error[RUNTIME]: <same but without "Bytecode compilation failed:" prefix>` (observed on the undefined-function variant) |
| REPL | `error[SEMANTIC]: Could not solve type constraints: ...` |
| `shape check` | `error: Semantic error: ... (file)` — no line/col/caret |

A *compile-time type error* is labeled `RUNTIME` in the flagship path because the JIT
executor wraps compilation failure in `ShapeError::RuntimeError` before the CLI
classifies it (`diagnostics_json.rs:66-68` then maps the variant name to the id).

**(c) P2 — parse errors have no location and a stray double-report.**

```text
$ shape run parse_err.shape       # content: "let x = \n"
Warning: failed to parse source for import pre-resolution: Parse error: Syntax error near: =
Error: Parse error: Syntax error near: =
```

No file, no line, no caret — and the same failure reported twice (once by import
pre-resolution, once by the parse proper). The `StructuredParseError` machinery with a
`CliErrorRenderer` exists (`script_cmd.rs:1630-1633`) but this error class doesn't route
through it.

**(d) P2 — multiple errors collapse into one.** Three wrong `let` bindings on three
lines yield ONE diagnostic whose message concatenates all three constraint failures,
anchored at line 1 col 1 (`shape check` counts "1 error(s)"; `run` shows a single caret
at 1:1). Per-site attribution is lost.

**(e) P2 — call-site type errors point at the definition.** `fn f(a: int)...; f("s")`
puts the caret at `fn f(a: int)` line 1 col 4 — the *definition* — not the offending
call at line 2.

**(f) P2 — comptime warning location degrades.** `warning[C0002]` from a
`comptime { warning(...) }` block at file line 2 renders ` --> <synthetic>:1:1` in human
mode (file lost, wrong line), though the JSON span `[0,43]` is roughly right. The C0001
error path is better (anchored at the block with a caret + trace note) but also says
`<input>` instead of the filename.

### 9.3 P1 — `[jit-fallback]` internal jargon leaks on ordinary programs (default mode)

Three separate everyday constructs produced audit-vocabulary stderr under the default
`--mode jit`:

```text
# (i) any projection borrow:  let r = &a[0]
[jit-fallback] function main failed JIT compile: ... γ-CP4 jit-makefieldref: SURFACE — `&`/`&mut`
projection borrow of `_1[copy _8]` (Index / nested-Deref) is not supported by the JIT. ...
`MakeIndexRef` is out of the β1 `RefTarget::TypedField` scope. Clean deopt to the interpreter
— ADR-006 §2.7.13.; running under interpreter

# (ii) a two-file project with fn main + main()
[jit-fallback] function main failed JIT compile: ... Route A surface-and-stop: SURFACE — direct
call to `main` resolved to function index 196 but has no compile-time-proven
FrameDescriptor.return_kind. W36 named-function callgraph requires a static return-kind proof ...

# (iii) snapshot()
[jit-fallback] function main failed JIT compile: ... WF-1A signal-reexec (audit 2026-07-04 §4(a)) ...
```

The mechanism (deopt + still-correct output) is working as designed, and the message
existing at all is defensible — but the *content* fails the project's own "jargon-free"
bar that comptime diagnostics explicitly enforce via a message firewall
(`comptime_diagnostics.rs:19-21`, "no internal audit vocabulary ever reaches the user
(acceptance P10)"). The JIT-fallback channel has no such firewall, and it fires on
programs as small as ten lines.

### 9.4 P1 — `xtask grammar-parity` is dead code

```text
$ ls crates/shape-core      # default corpus root (main.rs:1098)
ls: cannot access 'crates/shape-core': No such file or directory
$ shape parse hello.shape   # what the task runs per file (main.rs:1131-1137)
error: unexpected argument 'hello.shape' found
```

Both legs of the comparison fail unconditionally: `pest_ok` would be empty for every
corpus (the `shape parse` subcommand does not exist), and the default corpus path
references a crate that isn't in the workspace. The only automated guard against §5.3
grammar drift cannot ever have run against this CLI surface.

### 9.5 P1 — REPL swallows all `print()` output

```text
$ printf 'print("direct-print")\n1+1\n:quit\n' | shape repl
Shape REPL (type :help for commands)
2                                       # <- the echo works; "direct-print" never appears
```

Reproduced in both `--mode jit` and `--mode vm`. Root cause:
`engine.init_repl()` (`repl_cmd.rs:131`) installs `ReplAdapter`
(`crates/shape-runtime/src/engine/mod.rs:225-228`), whose `print` implementation
**discards the PrintResult and returns `none()`**
(`crates/shape-runtime/src/output_adapter.rs:79-89` — the comment says "the structured
`PrintResult` is dropped until the schema lands"). The REPL's renderer only prints a
`WireValue::PrintResult` when it arrives as the *value* of the cell
(`repl_cmd.rs:573-576`), which no longer happens post-v0.3.2 (commit 82f049dd rerouted
print through the OutputAdapter). Consequences: `print`/f-string-driven tutorials fail
silently; `:load script.shape` of any print-based script appears to do nothing. The TUI
calls the same `init_repl()` (`tui_cmd.rs:56`) and is code-level affected the same way.

### 9.6 P1 — `.shapec` bundles cannot be executed

```text
$ shape run proj/audit-proj-0.1.0.shapec
Error: failed to read proj/audit-proj-0.1.0.shapec
Caused by: stream did not contain valid UTF-8
```

`shape build` produces an artifact that `shape run` chokes on with a codec error rather
than either executing it or explaining. Bundles work only as dependencies
(`script_cmd.rs:874-919`).

### 9.7 P2 — assorted paper cuts

- **`Instruction limit exceeded: 100000 >= 100000`** — comparison-dump phrasing where
  "instruction limit (100000) reached" is meant.
- **`shape snapshot list` global pollution**: the default store currently lists 87
  snapshots, dominated by book-truth-gate CI leftovers pointing at one snippet file;
  there is no `--script` filter or age-based GC, so discovering *your* snapshot in the
  default store means scanning the full table.
- **TUI ignores `--snapshot-store`/`SHAPE_SNAPSHOT_STORE`** (§5.6).
- **Misleading `export` error** (§8.2): using the documented-but-nonexistent `export`
  keyword produces "Undefined variable: export" pointing at the *importer's* line 1.
- **LSP diagnostic ranges are whole-line sentinels** — observed `0:0..0:100` on a
  20-char line (stale binary; the range logic in `analysis.rs`/`error_to_diagnostic`
  computes real ranges for many paths, but the fallback pads to column 100).
- **`shape-lsp` accepts and ignores arbitrary argv** (`main.rs:12` checks only
  `--version`), which is why `shape-lsp-plugin`'s `args:["serve"]` works; a typo'd flag
  (e.g. `--stdio`, which many editors pass) is silently ignored rather than acknowledged
  — fine today, but worth an explicit arg parser.

### 9.7b P2 — piped REPL evaluates line-by-line

Non-interactive `shape repl` (stdin piped) has no continuation logic, so any multiline
construct fails with cascading errors (§2.7 transcript). Interactive sessions get
brace-aware continuation from the rustyline `Validator`; scripts and agent drivers get
neither that nor a documented "single-line only" contract. Either honor the validator's
continuation rules on piped input or accept a heredoc-friendly `:{ ... :}` block syntax.

### 9.8 Not bugs (verified working despite suspicion)

- Registry search over live network behaves correctly on both the empty-result and the
  error path (`search_cmd.rs:39-51`).
- Snapshot resume round-trip is real (§2.1) including v7 identity-map state.
- Resource limits fire in JIT mode too — a `--max-time-ms 2000` cap killed the runaway
  loop under default jit with exit 1 at 4.9s total wall time (startup + JIT compile +
  enforcement granularity); the cap is not interpreter-only.
- `.shapec` transitive native-dependency locking has real tests
  (`script_cmd.rs:2048-2143`).

### 9.9 P2 — `shape.toml` unknown sections silently ignored by the CLI (but flagged by the LSP)

A user with cargo muscle-memory who writes the manifest with `[package]` instead of
`[project]` gets no warning anywhere in the CLI — the config deserializes to all-default
and every downstream command degrades quietly:

```text
$ cat shape.toml
[package]                      # wrong section name — Shape wants [project]
name = "audit_demo"
version = "0.1.0"

$ shape build
Building package '' v...       # empty name, no version
Built 1 modules into package-0.0.0.shapec (241362 bytes)

$ shape tree
proj@0.0.0                     # falls back to directory name @ 0.0.0

$ shape check .
Error: shape.toml at './shape.toml' has no [project].entry field
```

Three different degraded behaviors (`''`/`package-0.0.0` artifact name, dirname@0.0.0,
and a missing-entry error that never mentions the actual problem), zero mentions that
`[package]` is not a recognized section. The correct schema is `[project]` with
`name`/`version`/`entry` (`crates/shape-runtime/src/project/project_config.rs:73-82`).

The kicker: **the LSP already implements exactly the missing validation** —
`tools/shape-lsp/src/toml_support/diagnostics.rs:28` ("Check for unknown top-level
sections") and `:219` (`check_unknown_keys`), unit-tested at `:439-454`. An editor user
gets squiggles on `[package]`; a CLI user gets `package-0.0.0.shapec`. Another instance
of the §4.5/§5.1 pattern: the capability exists in one surface and was never wired into
the other. Fix is small: run the same table/key validation in
`parse_shape_project_toml` (or at project load in the CLI) and warn.

### 9.10 P2 — project mode silently does nothing when `entry` defines `fn main` but never calls it

Script-mode semantics (top-level statements run; `fn main` is NOT auto-invoked) leak
into project mode, where they are least expected:

```text
$ cat shape.toml
[project]
name = "audit_demo"
version = "0.1.0"
entry = "src/main.shape"
$ cat src/main.shape
from util use { double }
fn main() -> int {
    print(f"double(21)={double(21)}")
    return 0
}

$ shape          # project mode: runs [project].entry
  Loaded module: python v0.1.0 (from extension directory)
  Loaded module: typescript v0.1.0 (from extension directory)
Shape engine initialized (2 extension modules loaded)
$ echo $?
0                # exit 0, zero program output, no hint

$ printf 'main()\n' >> src/main.shape && shape --mode vm
double(21)=42    # works once main() is explicitly called
```

The book's own examples do append an explicit `main()` call
(`getting-started/installation.mdx:89-93`, `first-query.mdx:116-121`), so this is
"as designed" — but a project whose entry compiles fine, defines `main`, and produces
*no output with exit 0* is a trap, and under default `--mode jit` the only stderr is the
§9.3(ii) `[jit-fallback] function main failed JIT compile: ... Route A
surface-and-stop ...` line, which actively implies `main` was involved in a run. A
one-line "note: `fn main` is defined but never called; project entries run top-level
statements" when the entry defines an uninvoked `main` and produces no top-level
side effects would close the trap. Effort: trivially small.

### 9.11 Not a bug, worth recording — best-in-class error message found during probing

For balance: the `reduce` arity/order error is exactly what every diagnostic here
should aspire to (correct site, names the actual signature, explains the fix):

```text
$ shape run hello.shape        # xs.reduce(0, |acc, x| acc + x) — wrong arg order
error[SEMANTIC]: `reduce` expects a closure (function) as its first argument, got an
int. Shape's `reduce` takes the callback first — the signature is `reduce(f, init)`,
not `reduce(init, f)`.
  --> <input>:6:23
```

(Still says `<input>` — §9.2a applies even to the best message in the codebase.)

## 10. What is done well

1. **LSP diagnostics ride the real compiler.** `analysis.rs:56-58` runs the actual
   `BytecodeCompiler` in `RecoverAll` mode instead of reimplementing a checker. Editor
   diagnostics can therefore never disagree with `shape run` about *whether* something is
   an error (only about rendering). Most young languages get this wrong; Shape didn't.

2. **Grammar-driven completions.** `grammar_completion.rs` derives valid-next-token
   completions from pest's own parse-error `positives` set — a zero-maintenance,
   drift-proof design ("ensures the LSP is always in sync with the actual language
   syntax"). This is exactly the pattern the tree-sitter/TextMate surfaces lack.

3. **vmjit-diff is exemplary verification tooling.** Byte-exact stdout+exit diffing, a
   known-red allowlist where every entry carries an audit citation and stale pins are
   *flagged for removal* when they start matching ("this file must never become a
   dumping ground that greens a red gate" — `known-red.json:4`), resumable JSONL progress
   pinned to binary+corpus, tiered corpus with provenance manifest. Ran green live today.

4. **`shape serve` refuses insecure configurations** rather than warning: non-loopback
   binds require both TLS material and an auth token (`serve_cmd.rs:308-330`), with the
   rationale in the error message. Foreign-language execution is strict opt-in
   (`--ffi-languages`, `cli_args.rs:346-353`) — deny-by-default done properly.

5. **Skip-cleanly test gating** in serve_cmd (`serve_cmd.rs:2579-2586`) — extension-
   dependent tests print a note and return early when the `.so` is absent, and run
   whenever it's present, instead of `#[ignore]`-rotting. 767/767 green LSP tests with
   zero ignores backs this culture up.

6. **The LSDS schema itself is well designed**: stable wire contract with an add-only
   rule and a pinned `SCHEMA_VERSION` test, builder API so emission sites survive schema
   evolution, type witnesses with concrete example values aimed at LLM consumers, token-
   budgeted context windows (`lib.rs` throughout). The design outruns the adoption, but
   the design is right.

7. **Comptime diagnostics have a jargon firewall** (`comptime_diagnostics.rs:19-21`,
   `clean_comptime_message`) — an explicit acceptance criterion ("no internal audit
   vocabulary ever reaches the user") *with enforcement in code*. It makes the
   jit-fallback channel's lack of the same firewall (§9.3) look like the anomaly it is.

8. **Interrupt-save semantics**: Ctrl-C during `shape run` is distinguished from failure
   (`ShapeError::Interrupted` propagates for a resume-command print + exit 130,
   `script_cmd.rs:1503-1512`) — thoughtful CLI citizenship rare at this project age.

9. **Doc-comment CLI help that teaches**: `cli_args.rs` flag docs explain not just what
   but why, citing the governing book pages and ADR amendments (e.g. the `--mode jit`
   fallback contract, the `--ffi-languages` ratification date). `--help` output doubles
   as accurate architecture documentation.

10. **Honest self-labelling of approximations**: the LSP storage-class hint appends
    `[… approx]` and names the authoritative compiler pass in its own tooltip
    (`inlay_hints.rs:459`). When you must ship a heuristic, this is how.

## 11. What is done poorly / tech debt

1. **The LSDS migration stalled after session 1** (2026-05-08) and the system has been
   growing string-based emission sites since. The `diagnostics_json.rs` bridge — parsing
   `[C0001]`-style prefixes back out of display strings — is the kind of stopgap that
   becomes load-bearing; it already has its own unit tests, which is how stopgaps
   calcify. (§5.1, §6.2)

2. **Diagnostic classification is variant-name-driven.** Ids like `RUNTIME`, `SEMANTIC`,
   `VM` (`diagnostics_json.rs:61-79`) are Rust enum variant names leaking into a public
   wire format that promises stable `B0013`/`E0100`-style codes. Machine consumers
   keying on `diagnostic_id` today will break when the E-series migration finally lands.

3. **tree-sitter-shape is effectively unmaintained** — one substantive commit since repo
   creation while the pest grammar changed continuously; its dead parity gate hides the
   rot. Editor syntax highlighting (both TM and TS paths) silently misrenders polyglot
   and extern constructs the book advertises. (§5.3, §9.4)

4. **Two REPLs, both wrong somewhere**: the line REPL swallows print output (§9.5) and
   skips project resolution in `:load` (§4.2); the TUI is undocumented under its real
   name and ignores the snapshot-store flag (§5.6); the book describes the TUI under the
   line REPL's command (§5.4). This surface needs an owner and a decision (merge or
   clearly split).

5. **serve_cmd.rs / script_cmd.rs / server.rs monoliths** — 2,970 / 2,249 / 3,257 LOC
   single files with tests inline. Navigability cost is real: the four error-rendering
   paths of §4.5 hide in these files.

6. **check_cmd is a demo, not a tool** — no module resolution, first-error-only, fake
   warnings counter, unconditional ANSI, no JSON. Everything it needs exists elsewhere
   in the codebase (module_loading wiring, RecoverAll mode, LSDS JSON renderer,
   MultiError fan-out); it just wasn't assembled. (§9.1)

7. **Filename loss at the engine boundary** — the engine knows the script path
   (`engine.script_path()`, used at `script_cmd.rs:1578` for context-file wiring) but
   compile errors uniformly render `<input>`. One plumbing fix would upgrade every
   diagnostic consumer at once. (§9.2a)

8. **Stale built tool binaries in a "working" toolchain**: `shape-lsp` (Apr 29) and
   `xtask` (Mar 12) in `target/debug` while `shape` is current — nothing in the
   dev-loop rebuilds or smoke-tests the LSP, consistent with the LSP wire surface having
   no in-repo harness (§7.4).

9. **Trading-era residue** (`:equity`, `:metrics`, tmLanguage keyword noise, "Pro
   feature" copy) — small individually, but collectively signals no one has walked the
   end-user surfaces recently. (§3.4)

## 12. Prioritized recommendations

### P0 (start this week; all are contained, high-leverage fixes)

1. **Fix REPL print-swallowing** — make `ReplAdapter` buffer and surface rendered print
   output (or have the REPL print it from a captured channel). One file + one REPL test.
   Effort: 0.5 day. Unbreaks every print-based tutorial and `:load`.
2. **Wire module loading into `shape check`** — reuse `script_cmd`'s project resolution +
   `module_loading` wiring; switch to `RecoverAll` + `MultiError` fan-out. Effort: 1-2
   days. Turns the CI command from a false-error generator into the tool it claims to be.
3. **Thread the script filename into diagnostics** — populate `SourceLocation.file` from
   `engine.script_path()` at compile-error construction (or at the CLI render boundary as
   an interim). Effort: 1 day. Every consumer (human, JSON, future LSP-LSDS) improves.

### P1 (this month)

4. **Un-invert LSDS for one high-value family**: implement migration-plan session 3
   (type errors with `expected`/`found` witnesses) and route `shape run`'s human renderer
   through `render::terminal` for LSDS-carrying errors. Kills the `RUNTIME`-id
   misclassification and the triple-prefix. Effort: 1-2 weeks (plan's own estimate 1.5).
5. **Add a jargon firewall to the `[jit-fallback]` channel** mirroring
   `clean_comptime_message`: one user-facing sentence + `--trace-jit` pointer for detail.
   Effort: 2-3 days.
6. **Fix or delete `xtask grammar-parity`**; if fixed (add a `shape parse`/`check`-based
   probe + real corpus, e.g. the vmjit-diff corpus), run it in CI and file the resulting
   tree-sitter gap list (`extern`, polyglot fns, `out`). Effort: 2 days for the gate;
   grammar catch-up is a separate ~1 week.
7. **Make `shape run bundle.shapec` either work or explain** ("bundles are libraries;
   use them as dependencies or `shape run` the project"). Effort: 0.5-2 days depending
   on choice.
8. **Rewrite `getting-started/repl.mdx`** for the actual `shape repl`, add a `shape tui`
   page. Effort: 0.5 day. (Also fix CLAUDE.md's `export` claim → `pub`.)

### P2 (backlog)

9. Per-site multi-error attribution (split the concatenated constraint failures into one
   diagnostic each, anchored at their own lines).
10. `From<ProviderCommandOptions>` impl + extract the 3× ExecutionMode conversion; split
    serve_cmd/script_cmd/server.rs along their natural seams.
11. Wire the LSP storage-class hint to the compiler's `BindingStorageClass` (or add a
    conformance test comparing the heuristic against the planner on a fixture corpus).
12. Snapshot-store hygiene: `shape snapshot list --script <path>` filter, `prune`
    subcommand, and TUI honoring `--snapshot-store`.
13. Add an in-repo LSP wire harness (initialize → didOpen → hover/diagnostics golden
    test) so the stdio surface is exercised by CI, and rebuild `shape-lsp` in the same
    dev-loop as `shape`.
14. Purge trading-era residue (`:equity`, `:metrics`, tmLanguage keyword list, "Pro
    feature" string); regenerate and commit a current `.vsix` or stop committing it.
15. Validate `shape.toml` at CLI load: reuse the LSP's unknown-section/unknown-key
    checks (`toml_support/diagnostics.rs:28,219`) in `parse_shape_project_toml` and
    warn on `[package]`-style mistakes instead of degrading to `package-0.0.0.shapec`
    (§9.9). Effort: 0.5-1 day.
16. Emit a one-line note when a project entry defines `fn main` but never invokes it
    and the run produces no output (§9.10). Effort: hours.

---

*Report generated by auditor 14 (Tooling & Developer Experience), 2026-07-11. All
transcripts reproduced from live runs against the working tree; scratch programs under
the session scratchpad (`verticals/tooling-dx/`).*


# tests/smokes-fallback — JIT-Err-class fallback fixtures

In-repo, git-tracked fallback fixtures that exercise the `[jit-fallback]`
diagnostic-emission path per W12 close (`docs/cluster-audits/v0.3-w12-jit-
mode-semantics-close.md`) §1.1 enumeration. Each fixture triggers a
distinct JIT-Err-class so the W12 fall-through is observable per-class.

W14.2-F1 territory per the W14.1 audit (`docs/cluster-audits/v0.3-w14-
test-coverage-audit.md`) §4 W12 row.

## F' canonical fallback-gate harness

```bash
out=$(timeout 30 ./target/release/shape run --mode "$mode" "$file" 2>&1)
ec=$?
last=$(echo "$out" | tail -1)
fb=$(echo "$out" | grep -c "^\[jit-fallback\]")
```

`2>&1` (vs `2>/dev/null` from the smoke harness) is required here because
the `[jit-fallback]` diagnostic emits to stderr per
`crates/shape-jit/src/executor.rs:150-153` and we must capture it to
assert emission.

**Forbidden** (per imprecision #109 — masks SURFACE-and-stop exit codes):

- `out=$(... | tail -N); ec=$?` — captures `tail`'s exit code, not `shape`'s.
- `ec=${PIPESTATUS[0]}` — non-portable; conflates pipe arrangement with
  capture-then-tail discipline.

## Fixtures

Each row lists the file, the JIT-Err class per W12 close §1.1, and the
expected VM / JIT `(last, ec, [jit-fallback]-count)`. The fall-through
contract is: JIT mode falls through to interpreter so `(last, ec)` for
JIT matches VM, AND the `[jit-fallback]` diagnostic emits exactly once.

| File | JIT-Err class | VM `(last, ec, fb)` | JIT `(last, ec, fb)` |
|------|---------------|----------------------|-----------------------|
| `f1-shared-module-binding.shape` | Kind-source gap at `print` (Route A surface-and-stop, pre-existing baseline) | `(100, 0, 0)` | `(100, 0, 1)` |
| `f2-preflight-shared-binding.shape` | Preflight rejection — `AllocSharedModuleBinding` / `LoadSharedModuleBinding` opcodes | `(<pre-existing VM err>, 1, 0)` | `(<same VM err>, 1, 1)` |
| `f3-preflight-closure-capture.shape` | Preflight rejection — `AllocSharedModuleBinding` in main code (`JitPreflightReport { vm_only_opcodes: [AllocSharedModuleBinding], unsupported_builtins: [] }`) | `(100, 0, 0)` | `(100, 0, 1)` |
| `f4-kind-source-gap-print.shape` | Kind-source gap on `print` operand (Route A — distinct producer site from f1) | `(<VM output>, 0, 0)` | `(<VM output>, 0, 1)` |
| `f6-struct-move-then-read.shape` | Move-then-read divergence — struct `let q = p` `Move`-sources `p`, projected later read `p.x` reads the JIT-nulled slot (ADR-006 §2.7.14) | `(1, 0, 0)` | `(1, 0, 1)` |
| `c1-generated-extend-capture-free.shape` | NONE — positive control (ADR-009 C1 Slice 0): annotation-generated `extend Type { method }` with a closure is JIT-NATIVE | `(42, 0, 0)` | `(42, 0, 0)` |

### ADR-009 C1 Slice-0 preflight (2026-07-14)

`c1-generated-extend-capture-free.shape` is the only ZERO-fallback row in this
matrix: it exists to prove the negative — that
`program_declares_user_trait_or_impl` (`crates/shape-jit/src/executor.rs:39-46`)
does **not** fire for an annotation-generated `extend` (which is `Item::Extend`,
never `Item::Impl`), so the generated method and the closure inside it run as
native JIT code.

The companion finding from Slice 0 is now historical. The integrated native
closure-lowering prerequisite removed the capture double-count and typed the
Shared-cell allocator: ordinary-source immutable, owned-mutable, and scalar
Shared captures now have exact VM/JIT stdout and ZERO-fallback coverage in
`jit_closure_capture_native.rs`. `f3` remains a module-binding/W39 negative
control; it must not be read as evidence that capturing closures generally
deopt. C1's separate `move` / `share` zero-fallback battery is authored in
`jit_generated_capture_native.rs` over five `c1-generated-*` fixtures plus the
ordinary inferred scalar and String nested-share #53 controls. Refcounted
Shared is now an authored native path: N9 in `jit_closure_capture_native.rs`
requires exact String output and zero fallback, while the canonical 36-entry
`HeapKind::ALL` catalog and direct Matrix/MatrixSlice zero/read/write/drop tests
prove the compact kind route and typed lifecycle are complete. These tests are
committed on the C1 branch but have not run under the supervisor lane against
the final head; they are not verified capability claims until that gate passes.
Module-binding capture inside a JIT-compiled function remains blocked by W39
F1.

`f1` is the canonical baseline preserved verbatim from the W12 close
(`docs/cluster-audits/v0.3-w12-jit-mode-semantics-close.md` §3.2) — its
trigger class shifted from "SharedModuleBinding preflight" to "Route A
kind-source-gap" between W12 close and HEAD due to upstream MIR producer
evolution (post-W12 routing changes; the SharedModuleBinding preflight
class is still triggerable via `f2` which uses a closure-capture-promotion
shape that emits the preflight-rejected opcodes).

`f3` keeps the historical filename for harness stability, but its current
observable class is the `AllocSharedModuleBinding` bytecode preflight shown
above. The old W12 expectation, "ClosureCapture missing function_id" from
MirToIR top-level preflight, is stale at HEAD for valid CLI fixtures: closure
capture MIR starts with `function_id: None` during lowering, then the bytecode
compiler back-patches captures with their concrete function ids before JIT
MIR preflight runs. Source-level fixtures therefore cannot leave an unpatched
`ClosureCapture` for MirToIR to reject; this fixture reaches the main-code
bytecode preflight first.

## UNTRIGGERABLE-AT-HEAD classes (per W12 close §1.1 enumeration)

The W12 close enumerated four broad JIT-Err classes. Three are
TRIGGERABLE at HEAD (per fixture matrix above); the rest are
UNTRIGGERABLE-AT-HEAD with the following surface notes:

- **JIT compile panic (`catch_unwind` arm at `executor.rs:224-236`).**
  Requires a JIT codegen bug to provoke; not reachable from valid
  user code. Defense-in-depth class; covered by the same fall-through
  match arm so the diagnostic-emission contract holds structurally.
- **FFI linking failure** (`link_foreign_functions_for_jit` Err at
  `executor.rs:245-249`). Requires a polyglot fn (Python/TypeScript) +
  no registered runtime. At HEAD, such a program reaches Route A
  kind-source-gap surface-and-stop BEFORE FFI linking is attempted (the
  MIR producer surfaces the `print` operand kind gap first). The FFI
  linking surface is reachable only after the kind-source-gap class is
  retired — tracked as v0.4 follow-up once W14.2-F1 sibling fixes
  converge the kind-source frontier.
- **JIT runtime signal (`signal < 0` at `executor.rs:461-466`).** The
  natural triggers (division by zero, array OOB) at HEAD either dump
  core (SIGFPE — bypasses the `signal < 0` return) or silently produce
  wrong output (no JIT-side guard). Clean `signal < 0` return requires
  a JIT-emitted guard that produces a negative signal — not currently
  used by user-reachable paths. Surface-and-stop until JIT-side guard
  emission is wired (separate follow-up beyond W14.2-F1 territory).
- **`RETURN_TAG_NANBOXED` host-boundary kind-source gap (`executor.rs:
  517-527`).** Distinct from the Route A MIR-time kind-source gap
  (which fires AT compile time for the JIT pipeline, not after `jit_fn`
  execution). Most kind-source gaps surface at MIR-compile time
  (Route A) before reaching the host boundary. Hard to trigger
  distinctly at HEAD; UNTRIGGERABLE.
- **MirToIR `ClosureCapture missing function_id` top-level preflight.**
  Stale W12/F3 expectation. The guard remains in the MIR preflight code as
  an internal invariant check, but the source-to-bytecode compiler patches
  valid closure-capture MIR with concrete function ids before the JIT sees it.
  At HEAD, the F3 source reaches `AllocSharedModuleBinding` bytecode preflight
  instead, so this class is not triggerable by a valid CLI fixture.

These five UNTRIGGERABLE classes share the same fall-through `match` arm
in `crates/shape-jit/src/executor.rs::execute_program` as the
TRIGGERABLE classes — the diagnostic-emission contract holds
structurally for them too. Surface-and-stop on the per-class trigger;
the fall-through CODE is exercised by the TRIGGERABLE classes.

## Discipline

- **Fixture-immutability** (Reading 6 META): never edit a fixture to
  make a gate pass. Surface drift to the user instead.
- **In-repo only**: do not re-introduce `/tmp/smokes-fallback/`
  references in new scripts. The W12 close doc references
  `/tmp/smokes-fallback/` as the path used at W12 audit time —
  immutable history, not rewritten.

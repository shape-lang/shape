# E4 S5 — @remote reborn on the HookDecision protocol: slice report (register with S1–S4)

**Worktree:** `shape-adr009-a3`, branch `adr009/e4`. **Base:** `e8045b05` (S4
COMPLETE). **This slice re-implements `@remote` in the FINAL syntax on the S4
HookDecision protocol — @remote's first REAL consumer — closing the #68 dark
window.** Design of record: `scratchpad/s5/e4-s5-design.md` (verdict PROCEED).

## Checkpoint commits (append-only on `e8045b05`)

| Ckpt | Hash | Repo | Scope |
|------|------|------|-------|
| CP1 | `286d8af1` | shape | declarative `HookDecision<Args>` sugar routing |
| CP2 | `1838c9e8` | shape | lift the decision-hook no-captures bound |
| CP3+CP4 | `4d2f235c` | shape | @remote markers + weave substitution + bare-R elaboration |
| CP5 | `e9250bc6` | shape | async + 0-ary loud named-defers; file #83 |
| CP6a | `30335639` | shape | land the stdlib @remote annotation (closes #68 dark window) |
| CP6a-fix | `97eb4916` | shape | flip the C3 dark-window tripwire (@remote is real now) |
| CP6b | `8b4d3695` | shape-web | the book delta — @remote fences live |

## The @remote source as shipped (`stdlib-src/core/remote.shape`)

```shape
builtin fn __remote_impl_ref() -> _;        // weave marker → the impl-shadow fn-ref
builtin fn __remote_arg_pack() -> Array<_>; // weave marker → the [p0..pN-1] pack

pub annotation remote(addr: string) on function {
    before(args) -> HookDecision<Args> {
        return HookDecision::Return(__call_raising(addr, __remote_impl_ref(), __remote_arg_pack()))
    }
}
```

The user-facing surface is identical to the deleted `10fcf533` block
(`@remote("host:port") fn f(...) -> T`); only the internal wiring is rebuilt on
the typed protocol. Import form: `from std::core::remote use { @remote }`
(annotations need explicit import; a bare `use std::core::remote` + `@remote`
yields "Unknown annotation '@remote'").

## The callee-identity mechanism (CRUX — RESOLVED)

S3 deleted `ctx.target`; S4 owns the impl shadow but exposes it only
compiler-internally. S5 reaches it through two compiler-recognized markers that
the decision lowerer SUBSTITUTES at weave time
(`pseudo_tuple::substitute_remote_markers`), inside the `HookDecision::Return(...)`
payload:

| Marker | Substituted with | Lowers to |
|---|---|---|
| `__remote_impl_ref()` | `Expr::Identifier(<impl-shadow SOH name>)` | the shadow's `Constant::Function(id)` → UInt64 fn-ref |
| `__remote_arg_pack()` | `[__c3_p0 .. __c3_pN-1]` array literal | one homogeneous `Array<T>` (OUTER-TypedArray `serialize_arg_pack` arm) |

**Hazard #1 (no recursion): DISCHARGED — proven by execution.** The callee is
the shadow's UInt64, never the wrapper's. A loopback `shape serve` receiver
logged the inbound wire Calls as
`fn="\u{1}hygienic:49562c…"` / `fn="\u{1}hygienic:76c55…"` — the SOH-hygienic
impl SHADOWS, NEVER the wrappers `compute`/`multiply` — and the round-trip
returned the correct values (`compute([10,20,30]).length()=3`, `multiply(6,7)=42`).
Evidence: `scratchpad/s5/impl/executed-no-recursion-proof.log`.

**Hazard #2 (fail LOUD): DISCHARGED — proven by execution + unit test.** A stray
marker that reaches compilation is a loud reject (the analysis pass's
undefined-function reject preempts, plus a compile-tier marker backstop in
`compile_expr_function_call`); a heterogeneous n-ary signature is a CLEAN
named-defer (`build_remote_arg_pack`), never a cryptic array-type mismatch. An
`@remote` call to a down server RAISES loudly ("remote call to 127.0.0.1:1
failed: … Connection refused") — `BEFORE` printed, `AFTER` never — never a
silent arg[0] misdispatch (Q26). There is no `?? args[0]` fallback: if the
impl-ref marker failed to substitute, arg[1] would still be `__remote_impl_ref()`
and the stray-marker reject fires.

## R-typing resolution (Hazard #3 — RESOLVED, no user-facing fork)

`__call_raising` is the RAISING remote primitive: it delivers the callee's value
at the callee's DECLARED return type and RAISES on transport/protocol/remote
failure (Q26). The callee is the impl shadow, whose declared return type IS `R`
by construction, so the short-circuit payload delivers BARE `R`.

- **Before-exit gate** (`guard_before_template_exit_kinds`, CP4): a
  `HookDecision::Return(__call_raising(...))` payload is proven `== R` directly
  (`is_call_raising_payload`) — the still-unsubstituted markers are never
  type-resolved.
- **Emission** (`compile_remote_raising_short_circuit`, `function_calls.rs`, CP4):
  the direct `__call_raising(addr, <shadow>, <pack>)` is elaborated at the
  shadow's BARE `R` (the `compile_remote_call_elaboration` class, but bare-R
  raising, not `Result<R, RemoteError>`).

**Outcome — does `@remote fn f() -> T` compile, or is `Result` required?**
Bare `R` works and RAISES on failure — `Result` is NOT required. Executed:
`@remote fn compute(data: Array<int>) -> int` compiles + runs green.
`@remote fn g(x: int) -> Result<int, E>` ALSO compiles (R is itself a Result:
the payload proves `Result<int,E>` == R, and transport failure still raises —
composes with the propagate path at zero extra work). The user is never forced
to write `-> Result`: `@remote`-the-annotation = raising; `remote::call`
(imperative) = recoverable `Result<R, RemoteError>`.

## First-cut bounds (each a LOUD named-defer — issue #83)

- **async `@remote`** (`reject_async_remote_short_circuit`): rejected — the sync
  `__call_raising` short-circuit would block the async executor thread on the
  wire round-trip (no `__call_async_raising` sibling yet).
- **0-ary `@remote`**: rejected by the generic 0-parameter decision-hook guard
  (`specialize_polymorphic_decision`) — no arguments to short-circuit over.
- **heterogeneous multi-arg `@remote`** (`build_remote_arg_pack`): rejected — the
  positional pack is one homogeneous `Array<T>` (the OUTER-TypedArray wire arm),
  which a mix of parameter types cannot form. Homogeneous multi-param
  (`f(a: int, b: int)`) and single-param (any type, incl. `Array<int>`) work.

Config captures (`addr`) compose (CP2 lifted the S4 no-captures bound); the
`addr` config bakes as a ConstLift prologue constant.

## Gate table (FAILED-name-set IDENTITY vs the `e8045b05` snapshot)

Baselines re-snapshotted at `e8045b05` (`scratchpad/s5/impl/baseline-*.txt`).

| Suite | Baseline @ e8045b05 | After S5 | FAILED-name verdict |
|-------|---------------------|----------|---------------------|
| shape-vm lib | 3554 / 7 / 36 (6 stable + `nested_exact` flap) | 3568 / 6–7 / 36 | IDENTICAL 6-name stable set + the tolerated `nested_exact` flap; +14 = the S5 unit tests |
| annotations_comptime | 117 / 10 | 117 / 10 | IDENTICAL 10-name set |
| modules_visibility | 133 / 1 / 3 | 133 / 1 / 3 | IDENTICAL (the 1 fail + the 3 @remote import-trio ignores are the known set) |
| `just check-clean` | exit 0 | exit 0 | GREEN |

14 NEW executed unit tests (`e4_s5_remote_tests.rs`): CP1 sugar decision routing
(short-circuit + proceed); CP2 config capture resolves + distinct-config bakes;
CP3+CP4 no-recursion structural proof (callee = shadow, never wrapper), bare-R +
Result-R type-check, homogeneous multi-int + Array-arg compile, stray impl-ref /
arg-pack markers fail loud, heterogeneous clean named-defer; CP5 async loud defer
+ 0-ary loud reject. The `remote_stdlib_module_carries_no_annotation_block`
dark-window tripwire is flipped to
`remote_stdlib_module_carries_typed_hookdecision_remote_annotation` (asserts the
typed @remote is present, the legacy shape absent).

## Book truth-gate — before/after (hold-or-improve)

FULL book: **569 / 572, 3 pre-existing reds** — up from the S4 baseline
**564 / 574, 10 reds**. ALL 5 @remote reds are CLEARED (3 green + 2 honest-dark).

**S5b review correction (append-only).** This section first reported **567 / 572,
5 reds**, derived from the per-slice method (the full single-run gate exceeds the
10-min harness cap) with A/B/C *assumed* to carry their S4 baseline reds. The S5b
book lens re-ran the FULL gate independently against the S5 binary and measured
**A 225/225, B 245/245, C 24/24, D 47/48, E 28/30 = 569 / 572, 3 reds**
(comptime.mdx:130, content-addressed-bytecode.mdx:344, :367 — all pre-existing,
non-@remote; ZERO @remote reds). The original figure over-counted reds by 2 in the
SAFE (under-claim) direction — the modules.mdx:50/61 pair the assumption carried
were in fact already green at the measured base. The corrected, executed number is
**569 / 572**; S5 holds-and-improves the gate by more than the report first claimed.

Measured with the S5 binary (vm+jit, `fixture=serve` loopback):
- **@remote pages (filtered gate): 8/8 runnable fences PASS** — remote.mdx
  22/41/77/142/166/187, polyglot-distributed :38, execution-server :130.
- **slice D** (annotations/comptime): 47/48 — IDENTICAL to the S4 baseline (the
  1 red is pre-existing comptime.mdx:130); no regression.
- **slice E** (advanced/tooling): 28/30 — improved from 27/32 (the 3 @remote reds
  resolved: :130 green, :74/:213 honest-dark; the 2 remaining are the
  pre-existing content-addressed-bytecode 344/367).
- The full single-run gate exceeds the 10-min harness cap; the 567/572 composite
  is derived from the per-slice method (the design's established approach) + the
  @remote-filtered gate + the compiler unit tests proving no non-@remote
  regression (my compiler changes are @remote/decision-specific).

Book delta (shape-web `adr009-c3-annotations`, staged by EXACT path — the other
~60 uncommitted files untouched, verified): remote.mdx + execution-server.mdx
@remote fences go real (`fixture=serve`, final syntax); the two COMPOSED
polyglot-distributed cells (:74 extern-C-on-receiver, :213 transfer+snapshot) are
`runnable=false` + honest S6-acceptance notes (issue #68); the 3×3 composition
matrix carries an honest caveat that S5 ships only the pure-Shape wire path.
`@remote fn python` (remote.mdx) stays honest-dark (S6-D). The @remote book
surface was pre-staged by the concurrent C3-annotations book agent; S5 fixed the
two S6 over-reaches (:74/:213) and committed the 3 @remote pages.

## S5 / S6 boundary — the 21 acceptance tests stay `#[ignore]`'d

S5 did NOT flip the S6 acceptance matrix (18 `distributed_*_e2e.rs` + 3
`serve_cmd.rs` + 3 `modules_visibility/scoped_contract.rs`). Left `#[ignore]`'d.

**Reachable-but-deferred (NOTED, not flipped):** the 3 import-trio
`scoped_contract` tests become compile-reachable-and-would-pass now that `@remote`
resolves + installs (they only need `from std::core::remote use { @remote }` /
`@remote::remote(...)` to resolve — both PROVEN working — and never call
`compute`). S6f formally flips them; they stay ignored in S5.

## Issues filed

- **#83** — "E4-S5 @remote residual signatures & flavors: async, 0-ary,
  heterogeneous multi-arg" (`ready-for-agent`). The async-defer and
  heterogeneous-guard messages cite it (interim #20 substituted). No dangling
  cites.

## Honest residuals

- **`[jit-fallback]` on every @remote'd call.** `@remote` rides the
  `__call_raising` ModuleFn dispatch, which hits the known ADR-006 §2.7.14 /
  v0.4 JIT ModuleFn-dispatch gap → one honest `[jit-fallback]` line; the program
  runs correctly under the interpreter (VM == JIT semantics preserved). Not new;
  inherited from the ModuleFn dispatch residual.
- **Markers are not stdlib-origin-gated.** The two weave markers +
  `__call_raising` are protected by being `__`-prefixed, undocumented, and
  out-of-weave-rejected, and `__call_raising` grants no capability beyond the
  already-public `remote::call` (both NetConnect-gated). A user who spells the
  markers inside a decision-hook Return payload could author an @remote-equivalent
  — a minor, greenfield-acceptable surface, not a privilege escalation. Not
  gated to `std::` origin in S5.
- **Stray-marker message.** A truly-undefined bare marker gets the generic
  "Undefined function: '__remote_…'" reject (loud, names the marker) because the
  analysis pass preempts the compile-tier custom message. Both are loud.
- **0-ary @remote uses the generic decision-hook reject** (not an @remote-specific
  message) — loud, but the "declare a void observer" advice is generic. #83
  tracks the fix (allow an empty pack).
- **Async / 0-ary / heterogeneous @remote** — all #83 (loud named-defers).

Worktree clean at `97eb4916` (shape) / `8b4d3695` (shape-web; the other ~60
sibling files untouched).

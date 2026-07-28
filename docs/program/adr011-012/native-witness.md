# The native execution witness (#117)

**Authority:** ruling R15 (`NativeExecutionWitness`, "no slice may relabel
interpreter fallback as native"), ADR-016 §5 (a fence claiming native execution
carries structured evidence of installation, subsequent dispatch, zero covered
fallback, and VM/JIT equality), ADR-011, R19.
**Artifacts:** `crates/shape-vm/src/native_witness.rs` (schema + collector +
assertions), `crates/shape-jit/src/ffi/witness.rs` (the native-entry callback),
`crates/shape-jit/src/compiler/witness_emit.rs` (emission),
`crates/shape-jit/src/witness_tripwires.rs` (execution-level tripwires).
**Consumers:** #187 (per-function deopt granularity), #188 (closure nativity),
#146 (HOF native tracer), #97 (native close criterion), ADR-016's `native`
evidence role.
**Recorded at:** wave4-spine, on top of `60ef72a8`.

R15 fixes what a native claim must bind but leaves the mechanism to this ticket:
where the evidence comes from, what makes it non-vacuous, and what a consumer
asserts against. This document records those decisions, and the limits the
current JIT places on them.

## 1. Why the dispatch event is the whole design

Three of R15's four bindings are compile-time facts and are cheap to record
truthfully:

| Binding | Where it comes from |
|---|---|
| verified artifact | `native_witness::begin_program` digests each unit (name, arity, opcode/operand sequence) before compilation |
| installation | `compiler/program.rs`, at the two `get_finalized_function` sites that enter the function table |
| covered fallback | every refusal site, each carrying a stable `FallbackReasonClass` |

The fourth — "subsequent native dispatch on the covered path" — is the one that
cannot be inferred, and is the one the historical defection kept skipping. A
JIT-to-JIT call is a direct Cranelift `call`; there is no runtime hook on it.
"We compiled it, we installed it, and the program printed the right answer" is
compatible with the installed code never running, because the interpreter
produces the same answer. That is precisely the interpreter-fallback lie.

So the dispatch count comes from **inside the emitted body**: while a witness
session is collecting, the JIT emits `jit_witness_native_entry(unit_index)` as
the first instruction of every compiled function's entry block. Nothing but
executing that machine code can increment the counter.

`Disposition::NativeDispatched` — the only disposition `assert_native_dispatch`
accepts — is *derived* in `finish()` from `installed && dispatches > 0`. No
sequence of recording calls can assert it directly.

### The observer effect, stated

The instrumented artifact is not byte-identical to the artifact an ordinary
`--mode jit` run executes: it carries one extra call per function entry. This is
disclosed in the record's `instrumentation` field (`native-entry-callback`).

What the instrumentation does **not** change is which functions are classified
JIT-compatible, which are installed, and which are refused: those decisions
happen in `compile_program_selective` before any body is built and read nothing
from the witness. A witness therefore proves "this program's function N
compiles, installs, and its native body runs"; it does not prove anything about
instruction-for-instruction equivalence with an uninstrumented build. Perf
measurement (#187, #188, R24) must be taken with the session off, which is the
default — `emit_native_witness_entry` returns immediately when no session is
active, so an ordinary run gets the codegen it got before this landed.

The alternative — always emitting the counter — was rejected: it taxes exactly
the measurements the PERF wave exists to take.

## 2. Where the session lives

`native_witness` is in **shape-vm**, not shape-jit. shape-jit depends on
shape-vm, so the JIT can record into it; the interpreter tier and a
`jit`-feature-off CLI can produce a truthful non-native witness through the same
type; and the tiered T1/T2 path (#187's other half) can record into it later
without a new seam.

The session is **thread-local and inert unless activated**. The whole-program
JIT path compiles, installs and executes on one thread, so a thread-local
session sees every event on that path. A dispatch on another thread would simply
not be counted — which under-reports nativity and can only make a native claim
fail, never succeed spuriously. That asymmetry is why this is not a process
global with atomics: the failure mode of the cheap design is the safe one. It
also gives per-test isolation for free, since the Rust harness runs each test on
its own thread.

`begin_program` is idempotent. The `--mode jit` fall-through runs the same
bytecode on the interpreter, and the interpreter registers units too;
re-registering would wipe the JIT's installation, dispatch and refusal records
and silently turn a fallback witness into an empty one.

## 3. The record

```jsonc
{
  "schema_version": 1,
  "shape_version": "0.3.2",
  "mode": "jit-whole-program",        // or "vm"
  "backend": "cranelift",             // or "none"
  "instrumentation": "native-entry-callback",
  "program_digest": "<sha256 over every unit digest, in index order>",
  "function_count": 198,
  "program_fallback": null,           // or { scope, reason_class, detail }
  "functions": [                      // sorted by (identity, index)
    {
      "function_identity": "hot",
      "function_index": 194,
      "artifact_digest": "<sha256 of name + arity + opcode/operand sequence>",
      "native_installed": true,
      "native_dispatches": 200,
      "interpreter_dispatches": 0,
      "disposition": "native-dispatched",
      "fallback": null
    }
  ],
  "ambiguous_identities": []
}
```

`function_identity` is the bytecode function name, and `__main__` for the
top-level unit (which is a compilation unit the JIT installs and dispatches like
any other, but is not a member of `BytecodeProgram::functions`; it occupies index
`functions.len()`).

**It is the string an ADR-016 `native-execution` expectation puts in
`function_identity`**, so a `BookCoverageManifest` row citing
`{"kind": "native-execution", "function_identity": "hot", "result_oracle": …}`
references the witness without translation. Generated closures share names;
identities that appear more than once are listed in `ambiguous_identities` and
`assert_native_dispatch` refuses them rather than picking one, because a claim
that cannot name one function is not a claim about a function.

### Determinism

No timing, duration, address, thread or count-of-anything-scheduled field
appears. Rows are sorted by `(identity, index)`; serialization is `serde_json`
over ordered structs. Two runs of the same program over the same binary produce
byte-identical JSON — asserted at the collector level
(`serialization_is_stable_and_round_trips`, which also fails on the string
`elapsed`/`duration`/`_ms`/`timestamp`/`address`/`ptr` appearing anywhere) and
end to end
(`witness_tripwires::two_runs_of_the_same_program_produce_identical_witnesses`).

Recording no code address is also what keeps the witness independent of JIT
module lifetime, so #209 (every `JITModule`'s pages leak by design) has no
bearing on it: a recorded installation is a boolean about an event, not a handle.

## 4. The fallback taxonomy

`FallbackReasonClass` is the machine-readable half; `detail` is prose a consumer
never parses. Every current refusal site is classified. Program-scoped:
`repl-persistence`, `top-level-comptime`, `user-trait-or-impl`,
`v2-verifier-unverified`, `imported-const-inline`, `w17-marshal-residual`,
`try-unwrap-residual`, `reference-escape-promotion`, `null-coalesce-residual`,
`scalar-move-lift`, `user-drop-impl`, `module-binding-function-body`,
`generic-struct-specialization`, `main-code-unsupported-construct`,
`top-level-mir-preflight`, `jit-compiler-init`, `foreign-link-failed`,
`jit-compile-error`, `jit-compile-panic`, `return-kind-gap`, `mode-vm`.
Function-scoped: `vm-only-opcode`, `unsupported-builtin`, `no-compilable-body`,
`function-codegen-failed`.

`unclassified` exists so that a refusal site added without a class is *visible*
as an unnamed reason rather than silently rendering as "not reached".

`record_program_fallback` and `record_function_fallback` are first-wins: the
specific site records its class, and the generic catch-all in
`JITExecutor::execute_program`'s `Err` arm cannot overwrite it. A preflight
refusal is the cause; a later codegen demotion of the same unit is a consequence.

## 5. What consumers call

```rust
use shape_vm::native_witness::{assert_native_dispatch, assert_fallback, FallbackReasonClass};

let unit = assert_native_dispatch(&witness, "hot")?;          // #187, #188, #146, #97
let unit = assert_fallback(&witness, "cold", FallbackReasonClass::VmOnlyOpcode)?;
```

`assert_native_dispatch` fails on: a whole-program deopt, an unknown identity, an
ambiguous identity, a unit with no installation, and — the vacuity guard — a unit
that was installed but never entered. `assert_fallback` fails when the function
actually ran native, when no fallback was recorded, and on a class mismatch. Both
have their own non-vacuity controls in
`shape_vm::native_witness::tests`.

A program-scoped fallback answers for a function that has no record of its own,
because a whole-program deopt is a truthful covered fallback for every function
in the program.

## 6. Producing one

```
shape run prog.shape --mode jit --native-witness witness.json
shape run prog.shape --mode jit --native-witness -        # stdout
shape run prog.shape --mode vm  --native-witness vm.json  # truthfully claims nothing native
```

The flag is global, so both `shape run <file>` and the bare `shape <file>` form
accept it. The witness is written on the failure path too — a run that errors is
still a run whose evidence a consumer may need. If no session was collecting, or
the write fails, the CLI says so on stderr rather than leaving a stale file to be
read as evidence.

## 7. What the current JIT lets the witness show — and what it does not

These are properties of the JIT at `60ef72a8`, surfaced *by* the witness. They
are recorded here because a consumer reading a witness needs to know which
limits are the tier's and which are the instrument's.

**A native dispatch count is real and exact.** `hot_only.shape` (a named function
called 200 times from a top-level loop) records `hot` with
`native_dispatches: 200` and `__main__` with `1`.

**One program can already carry both dispositions.** In that same run, `hot` is
`native-dispatched` while twelve other units carry a `vm-only-opcode` covered
fallback naming the exact opcode. The witness holds a native claim and a covered
fallback side by side in one record, which is what #187's acceptance criterion
needs.

**But two *user* functions cannot yet split that way.** In the two-function
fixture (`hot`, plus a `cold` whose `n as number` lowers to the VM-only
`ConvertToNumber`), `cold` is correctly refused with
`vm-only-opcode / opcodes=[ConvertToNumber]` — and then the *whole program*
deopts, because a direct call to a callee that is not in the compiled set is a
whole-program bail:

> Route A surface-and-stop: SURFACE — direct call to `cold` resolved to a
> function index but has no JIT FuncRef (callee not in the compiled set).

So `hot` is not native either, and `assert_native_dispatch(&w, "hot")` returns
`ProgramFellBack`. **This is the defect #187 exists to fix**, not a gap in the
witness: the witness's job here is to make it impossible to claim otherwise, and
`witness_tripwires::a_deopted_function_cannot_produce_a_native_dispatch_witness`
is the tripwire that will flip from "program fell back" to "hot stayed native"
when #187 lands.

**A whole-program deopt taken before compilation has no units.** The top-level
`comptime` and REPL-persistence deopts return before the bytecode exists, so
their witness carries the right `program_fallback` and an empty `functions` list.
`assert_native_dispatch` still correctly refuses every claim; `assert_fallback`
for a named function returns `UnknownFunction` rather than the program reason. If
a consumer needs named units in those two cases, the fix is to register units
from the AST-side program, which this slice did not do.

**Foreign functions cannot yet appear in a partially-native program.** Any
`fn python` / `extern C` program observed here whole-program deopts before the
foreign function is reached, via `w17-marshal-residual` or the Route A bail. The
witness records the reason truthfully; it cannot currently show a foreign
function as a per-function fallback inside an otherwise native program.

**`.map()` with a capturing closure is not silent, and is not native.** The
historical silent-divergence case records `scale` and `__main__` as
`native-dispatched` and the closure as `installed-not-dispatched` — installed,
never entered, and therefore explicitly *not* a native claim. Under the CLI the
run then fails with `JIT codegen for typed-array .map() with a closure argument
is unimplemented — deopting to interpreter`, which despite its wording errors the
run rather than deopting it. That is a separate defect, in #188's territory; the
witness's contribution is that the closure's non-nativity is now a recorded fact
rather than an absence.

## 8. Open

- The tiered T1@100 / T2@10k path records nothing yet. Only the whole-program AOT
  path is instrumented. #187 makes tiered the CLI default and owns extending the
  installation and dispatch hooks to it.
- Interpreter dispatch is counted only when a native frame trampolines into the
  interpreter (`dispatch_call_via_trampoline_vm`). Under `--mode vm` no
  per-function dispatch is counted at all; the `mode-vm` program fallback carries
  the claim instead. Counting every interpreter call would need instrumentation
  on the interpreter's own call path, which is a perf decision this slice did not
  make.
- VM/JIT semantic equality — R15's fifth binding — is *not* in this record. It is
  two executions and a comparison, which ADR-016 §5 already assigns to the
  BookTruthGate's parity obligation. The witness names the function and proves it
  ran native; the gate pairs that with the parity run.

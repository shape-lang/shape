# W12-jit-linker-resolve — Symbol Resolution Audit

**Sub-cluster:** Phase 3 cluster-0 Round 3 — JIT linker symbol resolution.
**Branch:** `bulldozer-strictly-typed-w12-jit-linker-resolve`.
**Parent:** `ea56cdcf` (Round-3 dispatch metadata, descends from Round-2
W11-jit-carrier-conversion merge `ff1ad3e6`).
**Date:** 2026-05-12.

## TL;DR — root cause

The "can't resolve symbol `main_f{idx}_{name}`" panic is **NOT** a naming
mismatch between producer and consumer, **NOT** a missing FFI symbol
registration, and **NOT** an ABI gap. It is a **second-order failure of
the failed-compile stub fallback** in
`crates/shape-jit/src/compiler/program.rs:702-725`:

When `compile_function_with_user_funcs` returns `Err` for any
JIT-compatible function (e.g. an `Rvalue::Aggregate` surface-and-stop
from `Route A`), the surrounding code attempts to install a stub body
that returns `signal = -1` so the caller deopts gracefully:

```rust
let neg = b.ins().iconst(types::I32, -1);
b.ins().return_(&[neg]);
```

That stub body is **rejected by Cranelift's verifier** because
`iconst.i32 -1` violates Cranelift's documented immediate-bounds rule
(see `cranelift-codegen/src/verifier/mod.rs:1644-1665`):

```rust
let bounds_mask = match ctrl_typevar {
    types::I8  => u8::MAX.into(),
    types::I16 => u16::MAX.into(),
    types::I32 => u32::MAX.into(),
    types::I64 => u64::MAX,
    _ => unreachable!(),
};
let value = imm.bits() as u64;
if value & bounds_mask != value {
    errors.fatal((inst, self.context(inst), "constant immediate is out of bounds"))
}
```

For `types::I32`, `-1i64` reinterpreted as `u64` is `0xFFFFFFFFFFFFFFFF`,
which does **not** fit in `u32::MAX = 0xFFFFFFFF`. Cranelift's `iconst`
convention is "pass the I32 bit-pattern as an unsigned-extended `i64`",
not "pass the signed value". The right immediate for `-1i32` is
`(-1i32 as u32) as i64` = `0xFFFFFFFFi64` = `4294967295`.

The stub `define_function` call is wrapped in `let _ = ...` which
silently swallows the error. The declared function id is then left with
**no body defined**. When `finalize_definitions()` later tries to
resolve relocations to that function id, Cranelift's JIT backend panics
in `cranelift-jit/src/backend.rs:345` with `panic!("can't resolve symbol {}", name)`.

The user-facing message in the executor's `catch_unwind` wrapper is:

```
JIT compilation panicked: can't resolve symbol main_f{idx}_{name}
```

## Methodology

1. **Reproduce on Smoke 2.** The kickoff prompt's Smoke 2 (`fn
   first_positive(xs: Array<int>) -> Option<int>` + `print(first_positive([-1, -2, 3, -4]))`)
   fails on a different surface first — `Rvalue::Aggregate` from
   the top-level Array literal — because the top-level
   `concrete_types` conduit is missing (W12-top-level-concrete-types-conduit
   territory; surfaced item #1). Wrapping the smoke in `fn main() { ... }`
   pushes the Array allocation INTO a user function whose `Rvalue::Aggregate`
   still fails, but the failure path now goes through the
   `compile_function_with_user_funcs → stub fallback` path rather than
   `compile_strategy → top-level Err`. The exact panic surfaces as

   ```
   thread 'main' panicked at .../cranelift-jit-0.110.3/src/backend.rs:345:21:
   can't resolve symbol main_f195_main
   ```

   Reproduction file:

   ```shape
   fn first_positive(xs: Array<int>) -> Option<int> {
       for x in xs {
           if x > 0 {
               return Some(x)
           }
       }
       None
   }
   fn main() {
       let arr: Array<int> = [-1, -2, 3, -4]
       print(first_positive(arr))
   }
   main()
   ```

2. **Identify the producing site.** `grep -rn 'declare_function' crates/shape-jit/src/`
   surfaces three sites:
   - `compiler/program.rs:130` — top-level `compile` entry (legacy).
   - `compiler/program.rs:188` — `compile_program` (non-selective, legacy).
   - `compiler/program.rs:657` — `compile_program_selective` Phase 2
     pre-declaration.
   - `compiler/strategy.rs:157` — `compile_strategy_with_user_funcs`
     declares the top-level `main` strategy.
   - `compiler/strategy.rs:348` — simulation-kernel `declare_function`
     (unrelated path).

   The symbol naming convention at the `compile_program_selective`
   call-site (which is what the executor invokes — `executor.rs:125`) is:

   ```rust
   let func_name = format!("{}_f{}_{}", name, idx, func.name.replace("::", "__"));
   ```

   For `name="main"`, `idx=195`, `func.name="main"`, that yields
   `main_f195_main`. The truncation in the kickoff prompt's quote
   ("`main_f...`") is a transcript-display truncation, not a real
   "_f_<truncated>_<rest>" name. The full symbol is well-formed and
   matches what the panic prints.

3. **Identify the consuming site.** `finalize_definitions()` in
   `cranelift-jit-0.110.3/src/backend.rs` walks the relocations of every
   defined function and, for each function-id reference, looks up the
   relocation target. If the target is a declared-but-not-defined
   function and not in the JIT builder's `symbol(...)` table and not
   `Linkage::Preemptible`, it panics:

   ```rust
   } else if linkage == Linkage::Preemptible {
       0 as *const u8
   } else {
       panic!("can't resolve symbol {}", name);
   }
   ```

   The relocation in question is emitted in
   `compiler/strategy.rs:187` (`declare_func_in_func`) — the top-level
   strategy walks `user_func_ids` and calls `declare_func_in_func` for
   **every** user function id, even ones whose bodies later failed to
   compile. The strategy's body then emits an indirect call through
   that ref via `jit_call_value` or `jit_trampoline_call_closure` — the
   relocation is real, the target must exist.

4. **Naming-convention check.** Both producer and consumer use the
   same `format!("{}_f{}_{}", name, idx, func.name.replace("::", "__"))`
   string in `program.rs:646` (Phase 2 declare), `program.rs:686`
   (Phase 4 attempt-to-compile), `program.rs:750` (Phase 5 final
   compiled_functions map). The string itself is well-formed and
   identical at every site. **Naming-mismatch ruled out.**

5. **Missing FFI symbol registration check.** The panicking name is
   `main_f195_main`, not a `jit_v2_*` or `jit_*` FFI shape, so this is
   not a `ffi_symbols/*.rs` issue. **FFI gap ruled out.**

6. **`Linkage::Local` vs `Linkage::Export` / `Preemptible` check.**
   The user function pre-declaration uses `Linkage::Local`
   (`program.rs:657`). The top-level strategy uses `Linkage::Export`
   (`strategy.rs:157`). Neither is `Preemptible`, so the panic at
   `backend.rs:345` is the correct code path — Cranelift's JIT backend
   only allows undefined symbols on `Preemptible` linkage (which is the
   "weak / may be overridden externally" linkage that doesn't apply to
   our model).

7. **Confirm the stub define_function failure.** I inserted a
   diagnostic eprintln around the `let _ = self.module.define_function(fid, &mut stub_ctx);`
   call in `program.rs:723` and re-ran Smoke 2:

   ```
   [jit-mir-audit] STUB define_function failed for fid=funcid299 (idx=195, func='main'):
     Compilation(Verifier(VerifierErrors([VerifierError {
       location: inst0,
       context: Some("v1 = iconst.i32 -1"),
       message: "constant immediate is out of bounds"
     }])))
   can't resolve symbol main_f195_main
   ```

   The verifier message identifies the exact instruction
   (`iconst.i32 -1`) and the exact reason (immediate out of bounds).
   This is reproducible across **every** function that hits the stub
   path — `std::core::math::spread`, `std::core::math::zscore`,
   `Into::int::decimal::into`, all the `TryInto::*` instances, `Json.keys`,
   etc. Each one declares the symbol in Phase 2, fails Phase 4 with an
   `Rvalue::Aggregate` (or `kind-untyped arith`) surface, hits the stub
   fallback, the stub's `iconst.i32 -1` fails verification, and the
   declared symbol has no body.

   The first such failure that fires a relocation from the top-level
   strategy is the one that panics — in Smoke 2's wrapping, that's the
   `main` user function.

## Root cause classification

This is **NOT** a:

- ❌ Naming mismatch (producer and consumer use identical strings).
- ❌ Missing FFI symbol registration (the symbol is a user function,
  not an FFI).
- ❌ Naming convention mismatch (`main_f` vs `main_F` / `__main_f`).
- ❌ ABI gap (signature is well-formed; the body simply never lands).

This **IS** a:

- ✅ Stub-fallback bug — the failed-compile recovery path uses a
  Cranelift API in a way that violates the verifier's documented
  invariant, so the recovery itself fails. The silent
  `let _ = ... define_function(...)` masks the failure. The declared
  but undefined symbol then becomes a load-bearing relocation target
  in the top-level strategy's emitted IR, and `finalize_definitions()`
  panics.

## Cranelift `iconst` immediate-bounds rule

`cranelift-codegen-0.110.3/src/verifier/mod.rs:1644-1665` defines the
rule:

> The immediate's `bits()` (an `i64`) reinterpreted as `u64` must be a
> subset of the controlling-typevar's natural-width mask. For `I32`
> that's `u32::MAX = 0xFFFFFFFF`.

So `iconst.i32 N` is valid iff `(N as u64) <= u32::MAX`. For positive
values up to `2^31 - 1` this is trivially satisfied (`N as u64` is
exactly `N`). For negative values you must pass the **two's-complement
unsigned bit pattern**, not the signed value:

```rust
// WRONG — fails verifier:
self.builder.ins().iconst(types::I32, -1)

// RIGHT — passes verifier:
self.builder.ins().iconst(types::I32, (-1i32 as u32) as i64)
// or equivalently:
self.builder.ins().iconst(types::I32, 0xFFFFFFFFi64)
// or equivalently:
self.builder.ins().iconst(types::I32, (i32::MIN as u32) as i64)  // for INT32_MIN
```

The codebase already follows this convention everywhere else for I8 /
I16 / I32 negative values (all the existing `iconst(types::I32, ...)`
sites in `mir_compiler/*.rs` use positive values or zero). The stub
fallback in `program.rs` is the only site that passes a signed
negative.

## Fix scope

Single-line fix in `crates/shape-jit/src/compiler/program.rs:719`:
replace

```rust
let neg = b.ins().iconst(types::I32, -1);
```

with

```rust
let neg = b.ins().iconst(types::I32, (-1i32 as u32) as i64);
```

Two-line hardening: also replace the silent
`let _ = self.module.define_function(fid, &mut stub_ctx);` with a
visible `eprintln!` on `SHAPE_JIT_DEBUG` so future stub-path failures
don't silently leak into the linker. Per §0 surface-and-stop discipline:
silent error swallow is itself a forbidden pattern when the swallowed
error is load-bearing.

## Verification plan

1. Smoke 2 (`fn main() { ... }` wrapping) under `--mode jit` should:
   - **Either** print `Some(3)` and exit 0 (if the
     `Rvalue::Aggregate` surface inside `main` is not itself
     load-bearing for the smoke — likely false since `main` is the only
     caller of `first_positive`),
   - **Or** print a clean runtime error from the
     `first_positive`-returned-from-stub deopt path (signal -1 →
     deopt-block return propagating).

   It will **NOT** panic with `can't resolve symbol`.

2. Smoke 2 plain form (`print(first_positive([...]))` at top level) will
   continue to fail at the top-level `Rvalue::Aggregate` surface — that
   is W12-top-level-concrete-types-conduit territory, not this
   sub-cluster's. **Surface that as a cross-cluster dependency.**

3. `cargo test -p shape-jit --lib` baseline (316 / 0 / 38) preserved.

4. `verify-merge.sh` passes 12/12.

5. `check-no-dynamic.sh` passes.

## Decisions documented

- **NOT** removed the stub fallback. The fallback exists to allow
  partial JIT compilation: if N user functions JIT-compile cleanly and
  one fails (e.g. on a known surface like `Rvalue::Aggregate`), the
  stubs let the JIT'd part still run and deopt at the call site to the
  failed function. Removing the fallback would force "either all
  functions JIT or none JIT", which is a regression. The fix is to
  make the stub well-formed.
- **NOT** changed the silent `let _ =` to an `eprintln!` everywhere —
  scoped the gating to `SHAPE_JIT_DEBUG=1` per the existing diagnostic
  convention in this file (`program.rs:675-682`, `:687-688`,
  `:697-699`).

## Sites surfaced (cross-cluster dependencies)

- **Smoke 2's plain form depends on W12-top-level-concrete-types-conduit.**
  Even with this fix, `print(first_positive([-1, -2, 3, -4]))` at top
  level fails because the top-level Array literal hits the
  `Rvalue::Aggregate` surface in `mir_compiler::statements` (the
  top-level `concrete_types: Vec::new()` gap at `strategy.rs:205`).
  Surfaced item #1 in `phase-3-cluster-0-status.md`.

- **`Rvalue::Aggregate` Route-A surface fires inside user functions
  too.** The fix unblocks the linker so the stub fires correctly; it
  does NOT change that ~30+ user-function bodies (the cast helpers
  `Into::*::*::into`, `TryInto::*::*::tryInto`, `Json.*`,
  `std::core::math::spread`, `std::core::math::zscore`) currently fail
  Phase-4 compile and route to the stub. Each of those is a separate
  W11-jit-new-array follow-up; un-stubbing them is out of scope for the
  linker-resolution sub-cluster.

- **The `let _ = self.module.define_function(fid, &mut stub_ctx);`
  pattern is itself a load-bearing-error-swallow.** Surfaced; the
  hardening commit adds a `SHAPE_JIT_DEBUG`-gated `eprintln!` so
  future stub-path failures are visible at the surface, not silently
  buried beneath the eventual linker panic.

---

*Audit complete. Fix commit follows.*

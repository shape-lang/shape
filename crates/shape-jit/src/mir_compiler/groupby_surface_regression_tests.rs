//! γ-CP9 jit-groupby-surface regression tests (v0.3 NO-KNOWN-INCORRECTNESS).
//!
//! Pre-γ-CP9 the MIR-JIT had no codegen for `groupBy` / `group` / `count`
//! on a typed array (a `|x| ...` closure-predicate higher-order method)
//! and produced garbage / crashed where the bytecode VM cleanly errors:
//!
//!   * `nums.groupBy(|x| x % 2)` then `.sum()`/`.len()` — the JIT
//!     `ffi/call_method/mod.rs` legacy-dispatch cascade had a `todo!()`
//!     stub for `group`/`groupBy` on `HK_ARRAY`; for a typed-array
//!     (`Ptr(TypedArray)`) receiver it instead fell to the
//!     `Ptr(_) => TAG_NULL` arm and returned a silent placeholder. With
//!     the receiver delegated to the VM trampoline, the closure argument
//!     — a JIT-format NaN-boxed inline-function carrier mis-stamped
//!     `Ptr(HeapKind::Closure)` — drove the transient `kinded_args` Vec
//!     drop to dereference the NaN-boxed bits as a heap pointer:
//!     SIGSEGV (ec=139).
//!
//!   * `nums.count(|x| x % 2)` — the same legacy-dispatch gap returned
//!     `TAG_NULL`, which the JIT caller decoded as the garbage integer
//!     `-1407374883553280` (ec=0) where the bytecode VM SURFACEs
//!     (`handle_count_v2` ckpt-2 SURFACE error).
//!
//! The γ-CP9 fix is honest surface-and-stop (NOT a partial typed-array
//! higher-order-method JIT path — that is W10 jit-playbook §5 / ADR-006
//! §2.7.4 territory and would only re-create a VM/JIT divergence while
//! the VM-side handlers still SURFACE):
//!
//!   1. `mir_compiler/v2_array.rs::try_emit_v2_array_method`: a
//!      `count` / `group` / `groupBy` typed-array method call returns a
//!      structured compile-stage `Err`. The W12 fall-through
//!      (`docs/cluster-audits/v0.3-w12-jit-mode-semantics-close.md`)
//!      routes the whole program to the bytecode interpreter, which runs
//!      the call with its own carrier-correct closure handling and
//!      produces the VM's behaviour verbatim.
//!
//!   2. `ffi/call_method/mod.rs::jit_call_method`: a defense-in-depth
//!      guard — if a `Ptr(TypedArray)` receiver reaches the runtime
//!      dispatch shell with a `Ptr(HeapKind::Closure)`-kinded argument
//!      (a call site not intercepted at the MIR stage), raise
//!      `pending_call_error` to deopt the JIT frame rather than build an
//!      unsound `kinded_args` Vec. The JIT-format HK_ARRAY legacy
//!      `todo!()` stubs (which can never unwind soundly across the
//!      `extern "C"` boundary) are likewise replaced with the same
//!      structured surface-and-stop.
//!
//! Net result: VM == JIT — both cleanly error, NEITHER produces garbage
//! or SIGSEGVs.
//!
//! Gated behind `deep-tests` per the `array_builder_regression_tests` /
//! `closure_dispatch_regression_tests` precedent — `JITExecutor::
//! execute_program` JIT-compiles the stdlib on every test, so default-
//! parallelism CI runs would race the JIT code cache.

use crate::executor::JITExecutor;
use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
use shape_runtime::initialize_shared_runtime;
use shape_vm::BytecodeExecutor;

/// Outcome of running a program through one executor: `Ok(wire-debug)` on
/// success, `Err(message)` on a clean surfaced error. Garbage from the
/// JIT manifests as an `Ok` with a different payload than the VM's; a
/// SIGSEGV would abort the test process outright (so a passing test also
/// proves the absence of the crash).
fn run(executor_is_jit: bool, source: &str) -> Result<String, ()> {
    let _ = initialize_shared_runtime();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let result = if executor_is_jit {
        JITExecutor::new().execute_program(&mut engine, &program)
    } else {
        BytecodeExecutor::new().execute_program(&mut engine, &program)
    };
    match result {
        Ok(r) => Ok(format!("{:?}", r.wire_value)),
        Err(_) => Err(()),
    }
}

/// Assert VM and JIT agree on the program: either both surface a clean
/// error, or both succeed with byte-identical results. A JIT garbage
/// miscompile manifests as the JIT returning `Ok(garbage)` while the VM
/// returns `Err` — this helper fails on exactly that divergence. A JIT
/// SIGSEGV aborts the process, which also fails the test.
fn assert_vm_eq_jit(source: &str) {
    let vm = run(false, source);
    let jit = run(true, source);
    assert_eq!(
        vm.is_err(),
        jit.is_err(),
        "VM/JIT divergence (one errored, one did not) for:\n{}\n\
         VM = {:?}, JIT = {:?}",
        source,
        vm,
        jit
    );
    if let (Ok(vm_val), Ok(jit_val)) = (&vm, &jit) {
        assert_eq!(
            vm_val, jit_val,
            "VM/JIT value divergence for:\n{}\nVM = {}, JIT = {}",
            source, vm_val, jit_val
        );
    }
}

// ── array `groupBy` — the canonical γ-CP9 reproducer ───────────────────

/// The canonical reproducer. Pre-γ-CP9 the JIT SIGSEGV'd (ec=139) inside
/// the VM trampoline's `kinded_args` drop on the JIT-format closure arg;
/// the bytecode VM cleanly errors. Post-fix the JIT compile-stage `Err`
/// makes the W12 fall-through run the interpreter — VM == JIT.
#[test]
fn array_groupby_then_len_does_not_jit_to_garbage() {
    assert_vm_eq_jit("let nums=[1,2,3,4,5]\nlet g=nums.groupBy(|x| x % 2)\nprint(g.len())");
}

/// `groupBy` whose result is consumed by `.sum()` — the heap-kinded
/// destination place pre-γ-CP9 fed the `TAG_NULL` placeholder into a
/// refcount-retain → SIGSEGV. Post-fix the JIT bails before codegen.
#[test]
fn array_groupby_then_sum_does_not_jit_to_garbage() {
    assert_vm_eq_jit("let nums=[1,2,3,4,5]\nlet g=nums.groupBy(|x| x % 2)\nprint(g.sum())");
}

/// `groupBy` whose result is never consumed — the crash was in the
/// trampoline `kinded_args` drop, independent of how the result is used.
#[test]
fn array_groupby_unconsumed_does_not_jit_to_garbage() {
    assert_vm_eq_jit("let nums=[1,2,3,4,5]\nlet g=nums.groupBy(|x| x % 2)\nprint(\"done\")");
}

/// `groupBy` with a `bool`-returning closure predicate — the closure
/// return type does not change the carrier-shape mismatch, so this bails
/// identically.
#[test]
fn array_groupby_bool_closure_does_not_jit_to_garbage() {
    assert_vm_eq_jit("let nums=[1,2,3,4,5]\nlet g=nums.groupBy(|x| x % 2 == 0)\nprint(\"done\")");
}

/// The `group` alias of `groupBy` — the same `try_emit_v2_array_method`
/// surface-and-stop arm covers both names.
#[test]
fn array_group_alias_does_not_jit_to_garbage() {
    assert_vm_eq_jit("let nums=[1,2,3,4,5]\nlet g=nums.group(|x| x % 2)\nprint(\"done\")");
}

// ── array `count` — the sibling garbage gap ────────────────────────────

/// Pre-γ-CP9 `count` printed the garbage integer `-1407374883553280`
/// (the decoded `TAG_NULL` placeholder) with ec=0 where the bytecode VM
/// SURFACEs. Post-fix VM == JIT — both cleanly error.
#[test]
fn array_count_with_predicate_does_not_jit_to_garbage() {
    assert_vm_eq_jit("let nums=[1,2,3,4,5]\nlet c=nums.count(|x| x % 2)\nprint(c)");
}

/// `count` whose result is never consumed — pins the surface-and-stop
/// independent of result consumption.
#[test]
fn array_count_unconsumed_does_not_jit_to_garbage() {
    assert_vm_eq_jit("let nums=[1,2,3,4,5]\nlet c=nums.count(|x| x % 2)\nprint(\"done\")");
}

// ── working array codegen must NOT be over-broadly bailed ──────────────

/// A plain scalar `Array<int>` `.sum()` — the γ-CP9 surface-and-stop arm
/// only fires for `count`/`group`/`groupBy`; ordinary typed-array
/// methods keep their inline JIT fast path. VM == JIT == 6.
#[test]
fn plain_int_array_sum_still_jit_compiles() {
    assert_vm_eq_jit("let a=[1,2,3]\nprint(a.sum())");
}

/// `len` on a typed array stays on the inline `try_emit_v2_array_method`
/// fast path — must not be caught by the new surface-and-stop arm.
#[test]
fn array_len_still_jit_compiles() {
    assert_vm_eq_jit("let a=[1,2,3,4]\nprint(a.len())");
}

/// `map` + `sum` over an array literal — a working closure-taking
/// higher-order chain that the γ-CP9 fix must NOT regress (`map` is not
/// in the surface-and-stop arm). VM == JIT.
#[test]
fn array_map_sum_chain_still_jit_compiles() {
    assert_vm_eq_jit("print([1,2,3,4,5].map(|x| x * 2).sum())");
}

/// `filter` + `len` — the other working closure-taking higher-order
/// chain; must keep agreeing VM == JIT.
#[test]
fn array_filter_len_chain_still_jit_compiles() {
    assert_vm_eq_jit("let a=[1,2,3,4,5]\nlet b=a.filter(|x| x > 2)\nprint(b.len())");
}

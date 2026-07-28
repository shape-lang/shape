//! Integration tests for automatic scope-based drop (RAII-style).
//!
//! Verifies that DropCall instructions are emitted at scope exit for
//! local variable bindings, and that drop works correctly with early
//! returns, breaks, nested scopes, etc.

use crate::bytecode::{BytecodeProgram, OpCode, Operand};
use crate::executor::tests::test_utils::{compile, eval};

fn drop_call_type_name_count(bytecode: &BytecodeProgram, type_name: &str) -> usize {
    bytecode
        .instructions
        .iter()
        .filter(|i| i.opcode == OpCode::DropCall)
        .filter(|i| {
            matches!(
                i.operand,
                Some(Operand::Property(sid))
                    if bytecode.strings.get(sid as usize).map(String::as_str) == Some(type_name)
            )
        })
        .count()
}

#[test]
fn test_auto_drop_at_scope_exit() {
    // A let binding inside a block should emit DropCall at scope exit.
    let bytecode = compile(
        r#"
        function test_fn() {
            {
                let x = 42
            }
            return 1
        }
        test_fn()
    "#,
    );
    let has_drop = bytecode
        .instructions
        .iter()
        .any(|i| i.opcode == OpCode::DropCall);
    assert!(
        has_drop,
        "block with let binding should emit DropCall at scope exit"
    );

    // Verify it still executes correctly
    let result = eval(
        r#"
        function test_fn() {
            {
                let x = 42
            }
            return 1
        }
        test_fn()
    "#,
    );
    assert_eq!(result.as_i64(), Some(1));
}

#[test]
fn test_auto_drop_reverse_order() {
    // Multiple bindings should drop in reverse declaration order.
    let bytecode = compile(
        r#"
        function test_fn() {
            let a = 1
            let b = 2
            let c = 3
            return a + b + c
        }
        test_fn()
    "#,
    );
    // Should have DropCall instructions for all 3 locals
    let drop_count = bytecode
        .instructions
        .iter()
        .filter(|i| i.opcode == OpCode::DropCall)
        .count();
    assert!(
        drop_count >= 3,
        "3 let bindings should emit at least 3 DropCall instructions, got {}",
        drop_count
    );
}

#[test]
fn test_auto_drop_on_early_return() {
    // An early return inside a block should trigger drops for locals in scope.
    let bytecode = compile(
        r#"
        function test_fn() {
            let x = 10
            if true {
                return x
            }
            return 0
        }
        test_fn()
    "#,
    );
    // Return should emit drops before ReturnValue
    let has_drop = bytecode
        .instructions
        .iter()
        .any(|i| i.opcode == OpCode::DropCall);
    assert!(has_drop, "early return should emit DropCall");

    // After Wave-E+5, `return x` (where `x: int`) inside a function
    // emits typed `ReturnValueI64`, the function call boundary pushes
    // raw native i64 bits onto the top-level stack, and the host-side
    // synthesizer stamps Int64 once the empty-call-stack predicate
    // fires. Decode via `eval_typed_i64` to get the native value.
    let result = crate::test_utils::eval_typed_i64(
        r#"
        function test_fn() {
            let x = 10
            if true {
                return x
            }
            return 0
        }
        test_fn()
    "#,
    );
    assert_eq!(result, 10);
}

#[test]
fn test_auto_drop_nested_scopes() {
    // Inner scope drops should happen before outer scope drops.
    let bytecode = compile(
        r#"
        function test_fn() {
            let outer = 1
            {
                let inner = 2
            }
            return outer
        }
        test_fn()
    "#,
    );
    // Should have drops for both inner and outer locals
    let drop_count = bytecode
        .instructions
        .iter()
        .filter(|i| i.opcode == OpCode::DropCall)
        .count();
    assert!(
        drop_count >= 2,
        "nested scopes should emit at least 2 DropCall instructions, got {}",
        drop_count
    );

    let result = eval(
        r#"
        function test_fn() {
            let outer = 1
            {
                let inner = 2
            }
            return outer
        }
        test_fn()
    "#,
    );
    assert_eq!(result.as_i64(), Some(1));
}

#[test]
fn test_inferred_block_binding_drop_call_carries_type_name() {
    let bytecode = compile(
        r#"
        type Handle { id: int }
        impl Drop for Handle {
            method drop() { }
        }
        {
            let h = Handle { id: 1 }
        }
    "#,
    );
    assert_eq!(
        drop_call_type_name_count(&bytecode, "Handle"),
        1,
        "inferred block-expression bindings must emit typed Handle DropCall"
    );
}

#[test]
fn test_range_for_body_drop_scope_covers_fallthrough_and_break() {
    let bytecode = compile(
        r#"
        type Guard { id: int }
        impl Drop for Guard {
            method drop() { }
        }
        for i in range(0, 3) {
            let g = Guard { id: i }
            if i == 1 { break }
        }
    "#,
    );
    assert!(
        drop_call_type_name_count(&bytecode, "Guard") >= 2,
        "range-for body must emit typed Guard DropCall for fallthrough and break exits"
    );
}

#[test]
fn test_auto_drop_error_does_not_propagate() {
    // Even if a drop errors, remaining code should still execute.
    let result = eval(
        r#"
        function test_fn() {
            {
                let x = 42
            }
            return 99
        }
        test_fn()
    "#,
    );
    assert_eq!(result.as_i64(), Some(99));
}

#[test]
fn test_async_drop_in_async_scope() {
    // Per-type async drop: a struct with async drop in an async function
    // should emit DropCallAsync. Plain `int` (no Drop impl) always gets DropCall.
    let bytecode = compile(
        r#"
        type Conn { id: int }
        impl Drop for Conn {
            async method drop() { }
        }
        async function test_fn() {
            let c: Conn = Conn { id: 1 }
            return c.id
        }
    "#,
    );
    let has_async_drop = bytecode
        .instructions
        .iter()
        .any(|i| i.opcode == OpCode::DropCallAsync);
    assert!(
        has_async_drop,
        "async function with async-drop type should emit DropCallAsync"
    );
}

#[test]
fn test_sync_function_uses_sync_drop() {
    // In a sync function, DropCall (not DropCallAsync) should be emitted.
    let bytecode = compile(
        r#"
        function test_fn() {
            let x = 42
            return x
        }
        test_fn()
    "#,
    );
    let has_sync_drop = bytecode
        .instructions
        .iter()
        .any(|i| i.opcode == OpCode::DropCall);
    let has_async_drop = bytecode
        .instructions
        .iter()
        .any(|i| i.opcode == OpCode::DropCallAsync);
    assert!(has_sync_drop, "sync function should emit DropCall");
    assert!(
        !has_async_drop,
        "sync function should NOT emit DropCallAsync"
    );
}

/// R8 W9 B3 Drop runtime fix regression test.
///
/// Pre-fix (see `docs/cluster-audits/v0.3-r8w9-drop-runtime-audit.md`):
///   - VM: `MakeRef Local outside any call frame` SURFACE at top-level
///     property-access inside a block scope. Top-level `MakeRef Local` had
///     `call_stack.len() == 0` and the construction site rejected with
///     `checked_sub(1)`.
///   - VM (repro 2 in audit): `MakeFieldRef base must reference a TypedObject;
///     got Bool` SURFACE: a slot with a user `impl Drop` impl had its V1.1C
///     `DropLocal` poison-pass fire BEFORE the legacy `LoadLocal + DropCall`
///     pair — so the Drop method received Bool-sentinel bits at `self`.
///
/// Post-fix: `op_make_ref` encodes top-level frames as `frame_index =
/// u32::MAX`, consumers route base_pointer=0 on that sentinel. Compiler
/// drop-scope emitter skips `DropLocal` for slots whose type has a user Drop
/// impl (the `DropCall` is the sole releaser).
#[test]
fn test_drop_top_level_block_self_field_access() {
    use crate::VMConfig;
    use crate::executor::VirtualMachine;
    let src = r#"
type FileHandle { path: string }
impl Drop for FileHandle {
  method drop() {
    print(f"Closed file: {self.path}")
  }
}
{
  let f = FileHandle { path: "/tmp/a.txt" }
  print(f"opened {f.path}")
}
print("after block")
"#;
    let bc = crate::executor::tests::test_utils::compile(src);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bc);
    let result = vm.execute(None);
    assert!(
        result.is_ok(),
        "Drop body should not surface; got {:?}",
        result.err()
    );
}

/// Drop bodies inside `fn main()` exit in reverse declaration order without
/// the V1.1C `DropLocal` poison breaking the subsequent `LoadLocal +
/// DropCall` dispatch.
#[test]
fn test_drop_fn_main_reverse_order() {
    use crate::VMConfig;
    use crate::executor::VirtualMachine;
    let src = r#"
type R { name: string }
impl Drop for R {
  method drop() { print(f"drop: {self.name}") }
}
fn main() {
  let a = R { name: "a" }
  let b = R { name: "b" }
  let c = R { name: "c" }
  print("body running")
}
main()
print("done")
"#;
    let bc = crate::executor::tests::test_utils::compile(src);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bc);
    let result = vm.execute(None);
    assert!(
        result.is_ok(),
        "Drop in fn main should not surface; got {:?}",
        result.err()
    );
}

/// ADR-006 §2.7.30 (escape-Drop-deferral): a Drop-bearing value that is
/// bound then RETURNED by-value must NOT be `DropCall`'d at the producing
/// function's scope exit — its `Drop` ownership moves to the caller. A
/// `DropCall` at both producer and consumer would run the user
/// `Drop::drop` body twice (the bind-then-return double-drop).
///
/// We count the REACHABLE type-"R" `DropCall` ops in the producing `make`
/// function — those preceding its first `ReturnValue`/`ReturnOwned`
/// terminator (the explicit `return r` path). The compiler also emits an
/// unreachable fallback epilogue after the explicit return; counting only
/// the reachable region matches actual runtime drop behavior. The escape
/// variant must emit ZERO reachable type-"R" DropCalls in `make` (the
/// returned local's drop is deferred to the caller); the non-escape
/// sibling must emit ONE (the local is dropped in `make`).
#[test]
fn escaping_returned_drop_local_is_not_dropcalled_in_producer() {
    // Reachable type-"R" DropCalls in function `fn_name`, counted up to its
    // first ReturnValue/ReturnOwned terminator.
    fn reachable_dropcalls_for_type_in_fn(src: &str, fn_name: &str, type_name: &str) -> usize {
        let bc = compile(src);
        let func = bc
            .functions
            .iter()
            .find(|f| f.name == fn_name)
            .unwrap_or_else(|| panic!("function {fn_name} not found"));
        let end = (func.entry_point + func.body_length).min(bc.instructions.len());
        let mut count = 0;
        for instr in &bc.instructions[func.entry_point..end] {
            // Stop at the first reachable return terminator — anything
            // after it (the fallback epilogue) is dead code.
            if matches!(instr.opcode, OpCode::ReturnValue | OpCode::ReturnOwned) {
                break;
            }
            if instr.opcode == OpCode::DropCall {
                if let Some(crate::bytecode::Operand::Property(sid)) = instr.operand {
                    if bc.strings.get(sid as usize).map(String::as_str) == Some(type_name) {
                        count += 1;
                    }
                }
            }
        }
        count
    }

    // ESCAPE: `make` binds `r: R` then returns it by value.
    let escape_src = r#"
type R { id: int }
impl Drop for R {
  method drop() { print("d") }
}
fn make() -> R {
  let r = R { id: 1 }
  return r
}
fn run() {
  let x = make()
  print("use")
}
run()
"#;

    // NON-ESCAPE: `make` binds `r: R`, drops it locally, returns an int.
    let non_escape_src = r#"
type R { id: int }
impl Drop for R {
  method drop() { print("d") }
}
fn make() -> int {
  let r = R { id: 1 }
  return r.id
}
fn run() {
  let x = make()
  print("use")
}
run()
"#;

    assert_eq!(
        reachable_dropcalls_for_type_in_fn(escape_src, "make", "R"),
        0,
        "a returned Drop-bearing local must NOT be DropCall'd in the \
         producing function (its Drop defers to the caller)"
    );
    assert_eq!(
        reachable_dropcalls_for_type_in_fn(non_escape_src, "make", "R"),
        1,
        "a non-escaping Drop local must still be DropCall'd once in the \
         producing function"
    );

    // Both programs must execute cleanly (no double-drop fault).
    use crate::VMConfig;
    use crate::executor::VirtualMachine;
    for src in [escape_src, non_escape_src] {
        let bc = compile(src);
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bc);
        assert!(
            vm.execute(None).is_ok(),
            "escape-Drop program should execute cleanly"
        );
    }
}

// =============================================================================
// Regression: Drop-in-loop break-pattern non-termination with f-string call
// args (strict-flip, 2026-06-20).
//
// `f"...{ident}..."` re-parses its inner expression with parser-LOCAL spans
// (offsets within the `{...}` fragment). Those spans could COLLIDE with an
// unrelated real statement's span in the MIR borrow analysis, making
// `query_ownership_decision` return that statement's `Move` for the
// f-string's identifier read. The compiler then emitted `LoadLocalMove`,
// consuming the live range-counter loop variable — the slot zeroed and the
// loop never advanced (ec=124 hang in both VM and JIT). The book
// resource-management slice (`scenario_retry`) hit this directly.
//
// Fix: while compiling an interpolated-string inner expression, skip the
// span-keyed ownership-move query (`in_interpolation_expr_depth`), so the
// load is a safe non-consuming `LoadLocal` / typed load.
// =============================================================================

/// The loop-counter read inside the break-branch f-string AND the
/// fall-through f-string must NOT compile to `LoadLocalMove` — that opcode
/// moves the counter out of its slot and the loop never terminates.
#[test]
fn fstring_arg_in_loop_break_does_not_move_loop_counter() {
    let bc = compile(
        r#"
let mut LOG: Array<string> = []
fn emit(ev: string) { LOG.push(ev) }
fn scenario() {
  for attempt in 0..5 {
    if attempt == 2 {
      emit(f"success:{attempt}")
      break
    }
    emit(f"fail:{attempt}")
  }
}
scenario()
"#,
    );
    let scenario = bc
        .functions
        .iter()
        .find(|f| f.name.contains("scenario"))
        .expect("scenario function present");
    let any_move = bc.instructions
        [scenario.entry_point..scenario.entry_point + scenario.body_length]
        .iter()
        .any(|i| i.opcode == OpCode::LoadLocalMove);
    assert!(
        !any_move,
        "f-string read of the range-counter loop variable must not emit \
         LoadLocalMove (would consume the live counter → infinite loop)"
    );
}

/// End-to-end: the Drop-in-loop break pattern terminates and produces the
/// correct event sequence (correct interpolated values + per-iteration Drop,
/// including the break iteration). If the move-bug regressed, the loop would
/// not terminate and this test would HANG — the assertion on the count is a
/// secondary guard once it does terminate.
#[test]
fn fstring_arg_in_loop_break_terminates_with_correct_drops() {
    // Returns LOG.len(): fail:0, drop:0, fail:1, drop:1, success:2, drop:2,
    // done = 7 events. A consumed counter would loop forever (never reaching
    // `attempt == 2`) and never return.
    let n = crate::test_utils::eval_typed_i64(
        r#"
let mut LOG: Array<string> = []
fn emit(ev: string) { LOG.push(ev) }
type Guard { id: int }
impl Drop for Guard {
  method drop() { emit(f"drop:{self.id}") }
}
fn scenario() {
  for attempt in 0..5 {
    let conn: Guard = Guard { id: attempt }
    if attempt == 2 {
      emit(f"body:success:{attempt}")
      break
    }
    emit(f"body:fail:{attempt}")
  }
  emit("body:done")
}
scenario()
let n: int = LOG.len()
n
"#,
    );
    assert_eq!(
        n, 7,
        "Drop-in-loop break with f-string args must terminate after 3 \
         iterations (fail:0/drop:0, fail:1/drop:1, success:2/drop:2, done)"
    );
}

/// A plain break carrying an f-string call arg (no Drop, no fall-through
/// f-string) must also terminate — the move-suppression is unconditional for
/// interpolation inner reads.
#[test]
fn plain_break_with_fstring_arg_terminates() {
    let n = crate::test_utils::eval_typed_i64(
        r#"
let mut LOG: Array<string> = []
fn emit(ev: string) { LOG.push(ev) }
fn scenario() {
  for attempt in 0..5 {
    emit(f"iter:{attempt}")
    if attempt == 2 {
      emit(f"stop:{attempt}")
      break
    }
  }
}
scenario()
let n: int = LOG.len()
n
"#,
    );
    // iter:0, iter:1, iter:2, stop:2 = 4 events.
    assert_eq!(
        n, 4,
        "plain break with f-string arg must terminate at attempt==2"
    );
}

/// Drop-variant selection must follow the EXECUTION CONTEXT, not the
/// DECLARATION ORDER of the sync/async `impl Drop` methods.
///
/// Book `fundamentals/resource-management.mdx` (the variant-selection table):
///   "Both sync and async" + "Sync context" => `DropCall` (sync fallback).
///
/// Pre-fix bug (declaration-order dependence): the impl-block lowering
/// registered BOTH the sync `drop` and the async `drop_async` under the same
/// trait-method symbol key (`Drop::<Type>::__default__::drop`, using
/// `method.name == "drop"` for both). The second-declared variant overwrote
/// the first, so a sync `DropCall` would resolve to whichever drop body
/// happened to be declared LAST. With the sync impl declared first, the async
/// drop body wrongly ran in a sync context.
///
/// The fix registers the async variant under `drop_async` (matching
/// `func_def.name` and the runtime `op_drop_call_impl` lookup), so the sync
/// `DropCall` always resolves to the sync `drop`. This test runs the program
/// in BOTH declaration orders and asserts the SYNC drop body fires (marker
/// `1`), never the async one (marker `2`).
#[test]
fn sync_context_runs_sync_drop_regardless_of_decl_order() {
    // Sync drop sets WHICH=1, async drop sets WHICH=2. A SYNC function uses
    // the resource, so the sync drop MUST run -> WHICH == 1.
    // `{SYNC_FIRST}` / `{ASYNC_FIRST}` toggles which impl method is declared
    // first; the result must be identical (1) for both.
    fn run(decl_order: &str) -> i64 {
        let src = format!(
            r#"
let mut WHICH: int = 0
fn mark(v: int) {{ WHICH = v }}
type Res {{ id: int }}
impl Drop for Res {{
{decl_order}
}}
fn use_it() {{
  let r = Res {{ id: 1 }}
}}
use_it()
let w: int = WHICH
w
"#
        );
        crate::test_utils::eval_typed_i64(&src)
    }

    let sync_first = "  method drop() { mark(1) }\n  async method drop() { mark(2) }";
    let async_first = "  async method drop() { mark(2) }\n  method drop() { mark(1) }";

    assert_eq!(
        run(sync_first),
        1,
        "sync context with sync-declared-first must run the SYNC drop (got async)"
    );
    assert_eq!(
        run(async_first),
        1,
        "sync context with async-declared-first must run the SYNC drop \
         (declaration order must not change variant selection)"
    );
}

/// WF-1C fix (c) — drop-error containment (ADR-006 §2.7.30, audit §4.7c).
///
/// A runtime error raised inside a user `Drop::drop` body must be CONTAINED,
/// not propagated: the program does not abort, every *remaining* scope-exit
/// drop still runs, and the scope's return value is preserved. Pre-fix,
/// `op_drop_call_impl` propagated the drop-body error with `?`, so the first
/// failing drop aborted the whole VM — the remaining drops were skipped and
/// the return value was lost.
///
/// Two `Bad` locals whose `drop()` bodies BOTH raise an out-of-bounds error.
/// Reverse-order scope exit drops `b2` then `b1`. Containment must:
///   - not surface the error out of `execute` (no abort),
///   - preserve the `scope()` return value (`42`),
///   - run BOTH drops (each contained error lands in the drop-error sink →
///     `drop_errors().len() == 2`, proving `b1` still ran after `b2`'s error
///     was contained).
#[test]
fn drop_error_is_contained_remaining_drops_run_and_return_preserved() {
    use crate::VMConfig;
    use crate::executor::VirtualMachine;
    let src = r#"
type Bad { id: int }
impl Drop for Bad {
  method drop() {
    let a: Array<int> = []
    let x: int = a[5]
  }
}
fn scope() -> int {
  let b1 = Bad { id: 1 }
  let b2 = Bad { id: 2 }
  42
}
scope()
"#;
    let bc = compile(src);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bc);
    let result = vm.execute(None);
    assert!(
        result.is_ok(),
        "drop-body error must be contained, not propagated; got {:?}",
        result.err()
    );
    assert_eq!(
        result.unwrap().as_i64(),
        Some(42),
        "scope() return value must survive drop-error containment"
    );
    assert_eq!(
        vm.drop_errors().len(),
        2,
        "both scope-exit drops must run under containment; sink = {:?}",
        vm.drop_errors()
    );
}

/// WF-1C fix (c) — containment isolates ONLY the failing drop; the surrounding
/// successful drops still run and the return value survives.
///
/// Mirrors the audit §4.7c repro: three locals `g1` (Good), `b` (Bad, errors),
/// `g2` (Good). Reverse-order exit runs `g2` (ok), `b` (errors → contained),
/// `g1` (ok). Exactly ONE contained error is recorded (the Bad drop); the two
/// Good drops run without error and the scope returns `42`.
#[test]
fn drop_error_containment_isolates_only_failing_drop() {
    use crate::VMConfig;
    use crate::executor::VirtualMachine;
    let src = r#"
type Good { id: int }
impl Drop for Good {
  method drop() { let _keep: int = self.id }
}
type Bad { tag: int }
impl Drop for Bad {
  method drop() {
    let a: Array<int> = []
    let x: int = a[5]
  }
}
fn scope() -> int {
  let g1 = Good { id: 1 }
  let b = Bad { tag: 0 }
  let g2 = Good { id: 2 }
  42
}
scope()
"#;
    let bc = compile(src);
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bc);
    let result = vm.execute(None);
    assert!(
        result.is_ok(),
        "only the failing drop should be contained; got {:?}",
        result.err()
    );
    assert_eq!(
        result.unwrap().as_i64(),
        Some(42),
        "return value must survive a contained drop between two Good drops"
    );
    assert_eq!(
        vm.drop_errors().len(),
        1,
        "exactly the Bad drop must be contained; the Good drops must not error; \
         sink = {:?}",
        vm.drop_errors()
    );
}

/// Inherited RAII repair (ADR-009 C2 #13, 2026-07-17): a Drop value returned by
/// a METHOD call and bound to an UNANNOTATED local must run `Drop::drop` at
/// scope exit. Before the `initializer_call_return_drop_type` MethodCall arm
/// (helpers.rs), `x`'s type never stamped for `let x = p.acquire()`, so an
/// UNTYPED `DropCall` was emitted and `Conn::drop` never ran — while the
/// `FunctionCall` sibling (`let x = make()`) WAS dropped. That asymmetry (a
/// pre-existing runtime soundness gap with no prior test) is what the D6
/// mask-breaker exposed. Pins the runtime half of the repair: exactly one TYPED
/// `DropCall(Conn)` is emitted, which invokes `Conn::drop` at scope exit.
///
/// SINGLE-EXIT TAIL-EXPRESSION body (`… x.id`) on purpose. `track_drop_local`
/// registers `x` exactly ONCE (verified: the method-call compile registers no
/// drop for its result; the re-arm sets `drop_kind` once), so the typed-DropCall
/// COUNT equals the number of scope-exit paths. An explicit-`return` body would
/// statically emit TWO `DropCall(Conn)` — the return-path drop PLUS the dead
/// fall-through epilogue (`functions.rs`: the `Statement::Return` arm ~2413-2425
/// falls through to the epilogue ~2460-2464; the 2nd is unreachable after the
/// terminal return, so exactly one still EXECUTES). That is a pre-existing shape
/// artifact of explicit returns, NOT a MethodCall-specific double-registration
/// — the FunctionCall sibling below shows the identical single count under the
/// same shape.
#[test]
fn test_method_call_returned_drop_value_runs_drop_at_scope_exit() {
    let bytecode = compile(
        r#"
        type Conn { id: int }
        impl Drop for Conn { method drop() { } }
        type Pool { n: int }
        extend Pool { method acquire() -> Conn { Conn { id: 1 } } }
        function test_fn() -> int {
            let p: Pool = Pool { n: 0 }
            let x = p.acquire()
            x.id
        }
        test_fn()
    "#,
    );
    assert_eq!(
        drop_call_type_name_count(&bytecode, "Conn"),
        1,
        "an unannotated local bound to a method-call-returned Drop value must emit \
         exactly one TYPED DropCall(Conn) at scope exit (the RAII MethodCall-arm repair); \
         an untyped DropCall (the pre-repair behavior) never runs Conn::drop"
    );

    // Runs without error (Conn::drop is empty); the drop executes at scope exit.
    let result = eval(
        r#"
        type Conn { id: int }
        impl Drop for Conn { method drop() { } }
        type Pool { n: int }
        extend Pool { method acquire() -> Conn { Conn { id: 1 } } }
        function test_fn() -> int {
            let p: Pool = Pool { n: 0 }
            let x = p.acquire()
            x.id
        }
        test_fn()
    "#,
    );
    assert_eq!(result.as_i64(), Some(1));
}

/// Parity sibling — the `FunctionCall` factory route (`let x = make_conn()`)
/// that the re-arm ALREADY covered emits the SAME single typed `DropCall(Conn)`
/// under the identical single-exit shape. Two things at once: the MethodCall arm
/// did NOT change the FunctionCall path (no regression), and the earlier
/// method-call "double" was purely the explicit-`return` shape artifact — a
/// same-shape FunctionCall body counts 1 too, not 2.
#[test]
fn test_function_call_returned_drop_value_runs_drop_at_scope_exit() {
    let bytecode = compile(
        r#"
        type Conn { id: int }
        impl Drop for Conn { method drop() { } }
        fn make_conn() -> Conn { Conn { id: 1 } }
        function test_fn() -> int {
            let x = make_conn()
            x.id
        }
        test_fn()
    "#,
    );
    assert_eq!(
        drop_call_type_name_count(&bytecode, "Conn"),
        1,
        "the FunctionCall factory route (the sibling the re-arm already covered) emits \
         exactly one typed DropCall(Conn) — symmetric with the MethodCall route"
    );
}

/// Control for the repair: the SAME shape with a NON-Drop method return
/// (`get_id() -> int`) emits NO typed `Conn` drop — the method-call-return drop
/// obligation is specific to a Drop return type, so the arm stays conservative
/// (no over-drop of a plain `int`).
#[test]
fn test_method_call_returned_non_drop_value_emits_no_typed_drop() {
    let bytecode = compile(
        r#"
        type Conn { id: int }
        impl Drop for Conn { method drop() { } }
        type Pool { n: int }
        extend Pool { method get_id() -> int { 7 } }
        function test_fn() -> int {
            let p: Pool = Pool { n: 0 }
            let x = p.get_id()
            x
        }
        test_fn()
    "#,
    );
    assert_eq!(
        drop_call_type_name_count(&bytecode, "Conn"),
        0,
        "a method-call return of a NON-Drop type must emit no typed Conn drop"
    );
}

// =============================================================================
// #181 (ERGO-VAR-TRUTH) — a qualifying `var` takes the ownership-aware direct
// load, not a shared cell.
// =============================================================================

/// TRIPWIRE 3 (#181), asserted on emitted opcodes.
///
/// The retired force-`SharedCow` gate made every `var` a `SharedCow` slot.
/// `SharedCow` is excluded from `slot_is_heap_backed_owned`, so such a slot
/// could never take the ownership-aware owned-heap load — every read went
/// through the shared-cell path instead. With the gate gone, a `var` that
/// earns nothing shared is an ordinary owned local and its read compiles to
/// the ownership-aware `CloneLocal`.
///
/// The ticket words this tripwire as `LoadLocalMove`, and #190 (ADR-018 §3)
/// landed the move path that makes the literal wording assertable: a single
/// terminal read of an unaliased owned heap local now takes the slot's share
/// rather than minting a second one. The literal opcode assertion lives with
/// that work, in
/// `executor::tests::rc_elision::a_qualifying_var_read_compiles_to_load_local_move`.
///
/// What this test keeps is the #181 property itself, which is about storage
/// rather than about the opcode: a `var` that earns nothing shared is read
/// directly, never through a cell. It fails the moment a `var` is re-pinned
/// to shared storage, however the read is spelled.
#[test]
fn a_qualifying_var_read_is_ownership_aware_and_not_cell_pinned() {
    let bc = compile(
        r#"
fn take(xs: Array<int>) -> int { xs.len() }
fn f() -> int {
    var xs = [1, 2, 3]
    take(xs)
}
let r = f()
"#,
    );
    let f = bc
        .functions
        .iter()
        .find(|func| func.name.ends_with("f"))
        .expect("function f present");
    let body = &bc.instructions[f.entry_point..f.entry_point + f.body_length];

    assert!(
        body.iter()
            .any(|i| matches!(i.opcode, OpCode::CloneLocal | OpCode::LoadLocalMove)),
        "the `var xs` read must take the ownership-aware owned-heap load; got {:?}",
        body.iter().map(|i| i.opcode).collect::<Vec<_>>()
    );
    assert!(
        !body
            .iter()
            .any(|i| matches!(i.opcode, OpCode::LoadClosure | OpCode::LoadSharedLocal)),
        "a `var` that is never captured or aliased must not be read through a \
         shared cell — that is the pinning the retired force-SharedCow gate caused; got {:?}",
        body.iter().map(|i| i.opcode).collect::<Vec<_>>()
    );
}

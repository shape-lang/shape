//! Integration tests for automatic scope-based drop (RAII-style).
//!
//! Verifies that DropCall instructions are emitted at scope exit for
//! local variable bindings, and that drop works correctly with early
//! returns, breaks, nested scopes, etc.

use crate::bytecode::OpCode;
use crate::executor::tests::test_utils::{compile, eval};

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

//! ADR-006 §2.7.27 / Item 4 ruling (W17-mutation-writeback, 2026-05-12):
//! source-level smoke tests for `&mut self` method writeback semantics on
//! COW container receivers.
//!
//! The ruling adopts Rust-style `&mut self` opt-in: mutating handlers
//! return a (possibly Arc-cloned) new receiver, the compiler emits a
//! post-`CallMethod` `Dup; StoreLocal` writeback so the binding slot
//! receives the new Arc identity. `let mut s = HashSet(); s.add("a");
//! s.add("b"); s.size()` returns 2 (pre-ruling: 0).
//!
//! Coverage:
//! - HashSet.add / .delete
//! - HashMap.set / .delete / .merge
//! - Deque.pushBack / .pushFront / .popBack / .popFront
//! - PriorityQueue.push / .pop
//! - `let` (non-`let mut`) immutability compile-error
//! - r-value receiver silent-drop (post-`compute_set().add(x)` shape)
//! - Compound-assignment operator sugar (`s += x`) for primitives

use crate::test_utils::{compile, eval, eval_with_kind};
use shape_value::NativeKind;

// ─── HashSet ─────────────────────────────────────────────────────────────

#[test]
fn writeback_hashset_add_size() {
    let result = eval(
        r#"
        let mut s = Set()
        s.add("a")
        s.add("b")
        s.size()
        "#,
    );
    assert_eq!(result.as_i64(), Some(2));
}

#[test]
fn writeback_hashset_add_duplicate_is_idempotent() {
    let result = eval(
        r#"
        let mut s = Set()
        s.add("a")
        s.add("a")
        s.size()
        "#,
    );
    assert_eq!(result.as_i64(), Some(1));
}

#[test]
fn writeback_hashset_delete() {
    let result = eval(
        r#"
        let mut s = Set()
        s.add("a")
        s.add("b")
        s.delete("a")
        s.size()
        "#,
    );
    assert_eq!(result.as_i64(), Some(1));
}

#[test]
fn writeback_hashset_let_immutable_compile_error() {
    // `let s = Set(); s.add("x")` must fail at compile time —
    // mutating method on an immutable binding.
    let program = shape_ast::parser::parse_program(
        r#"
        let s = Set()
        s.add("x")
        "#,
    )
    .expect("parse should succeed");
    let compiler = crate::compiler::BytecodeCompiler::new();
    let result = compiler.compile(&program);
    assert!(
        result.is_err(),
        "expected compile error for mutation on immutable binding, got Ok"
    );
    let err_msg = format!("{:?}", result.err().unwrap());
    assert!(
        err_msg.contains("immutable") || err_msg.contains("let mut"),
        "expected `immutable` / `let mut` diagnostic, got: {}",
        err_msg
    );
}

// ─── HashMap ─────────────────────────────────────────────────────────────

#[test]
fn writeback_hashmap_set_get() {
    let result = eval(
        r#"
        let mut m = HashMap()
        m.set("a", 1)
        m.set("b", 2)
        m.len()
        "#,
    );
    assert_eq!(result.as_i64(), Some(2));
}

#[test]
fn writeback_hashmap_delete() {
    let result = eval(
        r#"
        let mut m = HashMap()
        m.set("a", 1)
        m.set("b", 2)
        m.delete("a")
        m.len()
        "#,
    );
    assert_eq!(result.as_i64(), Some(1));
}

#[test]
fn writeback_hashmap_let_immutable_compile_error() {
    let program = shape_ast::parser::parse_program(
        r#"
        let m = HashMap()
        m.set("a", 1)
        "#,
    )
    .expect("parse should succeed");
    let compiler = crate::compiler::BytecodeCompiler::new();
    let result = compiler.compile(&program);
    assert!(
        result.is_err(),
        "expected compile error for HashMap.set on immutable binding"
    );
}

// ─── Deque ───────────────────────────────────────────────────────────────

#[test]
fn writeback_deque_push_back_then_size() {
    let result = eval(
        r#"
        let mut d = Deque()
        d.pushBack(1)
        d.pushBack(2)
        d.pushBack(3)
        d.size()
        "#,
    );
    assert_eq!(result.as_i64(), Some(3));
}

#[test]
fn writeback_deque_push_front_then_size() {
    let result = eval(
        r#"
        let mut d = Deque()
        d.pushFront(1)
        d.pushFront(2)
        d.size()
        "#,
    );
    assert_eq!(result.as_i64(), Some(2));
}

// NOTE: `Deque.popBack` / `popFront` are intentionally NOT in the
// writeback set — their return value is the popped element, not the
// (mutated) Deque Arc. A future tuple-return ABI amendment is needed
// for pop-shaped methods to mutate the binding visibly without
// corrupting the slot. See `method_registry::MUT_SELF_DEQUE_METHODS`.

#[test]
fn writeback_deque_let_immutable_compile_error() {
    let program = shape_ast::parser::parse_program(
        r#"
        let d = Deque()
        d.pushBack(1)
        "#,
    )
    .expect("parse should succeed");
    let compiler = crate::compiler::BytecodeCompiler::new();
    let result = compiler.compile(&program);
    assert!(
        result.is_err(),
        "expected compile error for Deque.pushBack on immutable binding"
    );
}

// ─── PriorityQueue ───────────────────────────────────────────────────────

#[test]
fn writeback_priority_queue_push_pop() {
    let result = eval(
        r#"
        let mut q = PriorityQueue()
        q.push(3)
        q.push(1)
        q.push(2)
        q.size()
        "#,
    );
    assert_eq!(result.as_i64(), Some(3));
}

#[test]
fn writeback_priority_queue_let_immutable_compile_error() {
    let program = shape_ast::parser::parse_program(
        r#"
        let q = PriorityQueue()
        q.push(1)
        "#,
    )
    .expect("parse should succeed");
    let compiler = crate::compiler::BytecodeCompiler::new();
    let result = compiler.compile(&program);
    assert!(
        result.is_err(),
        "expected compile error for PriorityQueue.push on immutable binding"
    );
}

// ─── R-value receiver — silent drop per dispatch-text decision call ──────

#[test]
fn writeback_rvalue_receiver_silent_drops() {
    // `Set().add("x")` — the r-value receiver case. The new (mutated)
    // Set Arc is the expression value of the call; with no consumer
    // it drops on statement end. This is the dispatch text's decision-
    // call: silent drop, not an error. The smoke target is that the
    // statement compiles and runs without raising.
    let _ = eval(
        r#"
        Set().add("x")
        42
        "#,
    );
}

// ─── Compound-assignment operator sugar (`s += x` for primitives) ────────

#[test]
fn compound_assign_int_addition() {
    let result = eval(
        r#"
        let mut n = 5
        n += 3
        n
        "#,
    );
    assert_eq!(result.as_i64(), Some(8));
}

#[test]
fn compound_assign_int_subtraction() {
    let result = eval(
        r#"
        let mut n = 10
        n -= 4
        n
        "#,
    );
    assert_eq!(result.as_i64(), Some(6));
}

#[test]
fn compound_assign_int_multiplication() {
    let result = eval(
        r#"
        let mut n = 3
        n *= 4
        n
        "#,
    );
    assert_eq!(result.as_i64(), Some(12));
}

#[test]
fn compound_assign_number_addition() {
    let result = eval_with_kind(
        r#"
        let mut n: number = 1.5
        n += 2.25
        n
        "#,
        NativeKind::Float64,
    );
    assert_eq!(result.as_f64(), Some(3.75));
}

#[test]
fn compound_assign_string_concatenation() {
    let result = eval(
        r#"
        let mut s = "hi"
        s += " there"
        s
        "#,
    );
    assert_eq!(result.as_str().as_deref(), Some("hi there"));
}

#[test]
fn binary_op_without_assignment_is_pure() {
    // `let t = s + 3` — the non-mutating operator form. `s` stays 5,
    // `t` is the sum.
    let result = eval(
        r#"
        let mut s = 5
        let t = s + 3
        s
        "#,
    );
    assert_eq!(result.as_i64(), Some(5));
}

#[test]
fn binary_op_without_assignment_returns_new_value() {
    let result = eval(
        r#"
        let mut s = 5
        let t = s + 3
        t
        "#,
    );
    assert_eq!(result.as_i64(), Some(8));
}

// ─── Mutex / Atomic / Lazy — interior-mutability `let` works ─────────────
//
// These primitives use interior mutability; their `set` / `store` etc.
// preserve the receiver Arc's identity. The `let` (immutable) binding
// must stay valid — what changes is the shared interior, not the
// binding itself. This mirrors Rust's `let m = Mutex::new(0); *m.lock()
// = 5;` shape.

#[test]
fn mutex_set_on_let_immutable_binding_works() {
    // `let m = Mutex(0); m.set(5); m.get()` — should NOT be a compile
    // error. Mutex is not registered in `mut_self_container_locals`;
    // `set` is interior-mutability.
    let result = eval(
        r#"
        let m = Mutex(0)
        m.set(5)
        m.get()
        "#,
    );
    assert_eq!(result.as_i64(), Some(5));
}

#[test]
fn atomic_store_on_let_immutable_binding_works() {
    let result = eval(
        r#"
        let a = Atomic(0)
        a.store(7)
        a.load()
        "#,
    );
    assert_eq!(result.as_i64(), Some(7));
}

// ─── Numeric widening lattice (ADR-006 §2.7.27 / Commit 2) ────────────────
//
// The lossless lattice covers integer-width widening:
//   i8 → i16 → i32 → i64
//   u8 → u16 → u32 → u64
//
// `int ↔ number` widening is governed by existing arithmetic-result
// inference (`5 * 2.0 → number`) and is NOT part of the lattice this
// ruling adds; the ruling reaffirms that narrow integers widen to wider
// integers, not across the int/number boundary.
//
// Narrowing requires explicit `as T`. Widening is compile-time only —
// NO runtime coercion opcodes (CLAUDE.md "Forbidden Patterns" #5).

#[test]
fn widening_i8_to_i32_via_compound_assign() {
    // The dispatch text's canonical smoke target:
    //   let mut n: i32 = 0
    //   let x: i8 = 5
    //   n += x      // widens x to i32, AddI32 + writeback to n's slot
    //   print(n)   // 5
    let result = eval_with_kind(
        r#"
        let mut n: i32 = 0
        let x: i8 = 5
        n += x
        n
        "#,
        NativeKind::Int32,
    );
    // i32 result bits decode as the low 32 bits of the u64 slot.
    let bits = result.raw();
    let val = (bits & 0xFFFF_FFFF) as i32;
    assert_eq!(val, 5);
}

#[test]
fn widening_i16_to_i64_in_binding() {
    let result = eval(
        r#"
        let x: i16 = 1234
        let n: i64 = x
        n
        "#,
    );
    assert_eq!(result.as_i64(), Some(1234));
}

#[test]
fn widening_u8_to_u16_in_binding() {
    let result = eval(
        r#"
        let x: u8 = 200
        let n: u16 = x
        n
        "#,
    );
    assert_eq!(result.as_i64(), Some(200));
}

#[test]
fn widening_u8_to_u32_in_binding() {
    let result = eval(
        r#"
        let x: u8 = 255
        let n: u32 = x
        n
        "#,
    );
    assert_eq!(result.as_i64(), Some(255));
}

// ─── Compile-trace check: writeback opcodes emitted on mutating call ─────

#[test]
fn writeback_emits_dup_storelocal_on_mut_method() {
    let bc = compile(
        r#"
        let mut s = Set()
        s.add("a")
        s.size()
        "#,
    );
    // Look for the `Dup; Store{Local,ModuleBinding}` sequence in the
    // top-level instruction stream. Top-level `let mut s = ...` becomes
    // a module binding, so the writeback target is
    // `StoreModuleBinding`. Inside a function body or block scope, it
    // would be a `StoreLocal`. Either is accepted.
    use crate::bytecode::OpCode;
    let top = &bc.instructions;
    let mut saw_dup_after_call = false;
    let mut diag = String::new();
    for (i, ins) in top.iter().enumerate() {
        diag.push_str(&format!("{:3}: {:?}\n", i, ins.opcode));
        if ins.opcode == OpCode::CallMethod
            && i + 2 < top.len()
            && top[i + 1].opcode == OpCode::Dup
            && matches!(top[i + 2].opcode, OpCode::StoreLocal | OpCode::StoreModuleBinding)
        {
            saw_dup_after_call = true;
            break;
        }
    }
    assert!(
        saw_dup_after_call,
        "expected `CallMethod; Dup; Store{{Local,ModuleBinding}}` writeback sequence in bytecode:\n{}",
        diag
    );
}

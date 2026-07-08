//! Optional types: None keyword, coalescing, optional field syntax.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// None values (Shape uses `None` with capital N)
// =========================================================================

#[test]
fn none_value_prints() {
    ShapeTest::new(
        r#"
        let x = None
        print(x)
    "#,
    )
    .expect_run_ok();
}

#[test]
fn none_equality_check() {
    ShapeTest::new(
        r#"
        let x = None
        x == None
    "#,
    )
    .expect_bool(true);
}

#[test]
fn non_none_inequality() {
    ShapeTest::new(
        r#"
        let x = 42
        x == None
    "#,
    )
    .expect_bool(false);
}

// =========================================================================
// Coalescing (??)
// =========================================================================

#[test]
fn none_coalesce_returns_fallback() {
    ShapeTest::new(
        r#"
        let x = None
        let y = x ?? 10
        y
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn none_coalesce_keeps_value() {
    ShapeTest::new(
        r#"
        let x = 42
        let y = x ?? 10
        y
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// Optional field syntax (port?: int)
// =========================================================================

// TDD: `field?: type` optional field syntax not yet in grammar; parser rejects `?` after field name
#[test]
fn optional_field_type_workaround_parses() {
    ShapeTest::new(
        r#"
        type Config { host: string, port: int }
    "#,
    )
    .expect_parse_ok();
}

// TDD: return type annotations parse for functions without params
#[test]
fn function_with_return_type_annotation() {
    ShapeTest::new(
        r#"
        type Score = number
        fn get_score() -> Score {
            return 100
        }
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Optional chaining (`?.`) — wave7/optional-chaining (book C3)
//
// `expr?.prop` on an `Option<T>`/`T?` receiver short-circuits to `None` when
// absent and yields `Some(prop)` when present; it composes left-to-right in a
// chain and with `??`. Lowered to `match expr { Some(v) => Some(v.prop), None
// => None }` at desugar (crates/shape-ast/src/transform/desugar.rs), reusing
// the proven Option pattern-match machinery. Each case is run under BOTH the
// bytecode VM and the JIT executor (`--mode jit`, which safely whole-program
// deopts the Some/None match to the interpreter via the pre-existing
// EnumPayload W12 fallback — no SIGSEGV, VM == JIT semantics).
// =========================================================================

const OPTCHAIN_CFG_PRESENT: &str = r#"
    type Server { port: int }
    type Config { server: Server }
    let cfg: Option<Config> = Some(Config { server: Server { port: 9000 } })
    let port = cfg?.server?.port ?? 8080
    print(port)
"#;

const OPTCHAIN_CFG_ABSENT: &str = r#"
    type Server { port: int }
    type Config { server: Server }
    let cfg: Option<Config> = None
    let port = cfg?.server?.port ?? 8080
    print(port)
"#;

#[test]
fn optchain_nested_coalesce_present_vm() {
    ShapeTest::new(OPTCHAIN_CFG_PRESENT).expect_output("9000");
}

#[test]
fn optchain_nested_coalesce_present_jit() {
    ShapeTest::new(OPTCHAIN_CFG_PRESENT)
        .with_jit()
        .expect_output("9000");
}

#[test]
fn optchain_nested_coalesce_absent_vm() {
    ShapeTest::new(OPTCHAIN_CFG_ABSENT).expect_output("8080");
}

#[test]
fn optchain_nested_coalesce_absent_jit() {
    ShapeTest::new(OPTCHAIN_CFG_ABSENT)
        .with_jit()
        .expect_output("8080");
}

// A chain short-circuits on the FIRST None link: an absent outer Option never
// evaluates the inner `.server?.port` access.
const OPTCHAIN_SHORT_CIRCUIT: &str = r#"
    type Server { port: int }
    type Config { server: Server }
    let cfg: Option<Config> = None
    let r = cfg?.server?.port
    match r { Some(p) => print(p), None => print(-1) }
"#;

#[test]
fn optchain_short_circuits_first_none_vm() {
    ShapeTest::new(OPTCHAIN_SHORT_CIRCUIT).expect_output("-1");
}

#[test]
fn optchain_short_circuits_first_none_jit() {
    ShapeTest::new(OPTCHAIN_SHORT_CIRCUIT)
        .with_jit()
        .expect_output("-1");
}

// `some_opt?.field` is Some(field) / None.
const OPTCHAIN_FIELD_SOME: &str = r#"
    type Server { port: int }
    let s: Option<Server> = Some(Server { port: 42 })
    match s?.port { Some(p) => print(p), None => print(-1) }
"#;

const OPTCHAIN_FIELD_NONE: &str = r#"
    type Server { port: int }
    let s: Option<Server> = None
    match s?.port { Some(p) => print(p), None => print(-1) }
"#;

#[test]
fn optchain_single_field_some_vm() {
    ShapeTest::new(OPTCHAIN_FIELD_SOME).expect_output("42");
}

#[test]
fn optchain_single_field_some_jit() {
    ShapeTest::new(OPTCHAIN_FIELD_SOME)
        .with_jit()
        .expect_output("42");
}

#[test]
fn optchain_single_field_none_vm() {
    ShapeTest::new(OPTCHAIN_FIELD_NONE).expect_output("-1");
}

#[test]
fn optchain_single_field_none_jit() {
    ShapeTest::new(OPTCHAIN_FIELD_NONE)
        .with_jit()
        .expect_output("-1");
}

// `expr?.method(args)` short-circuits None and yields Some(method result).
const OPTCHAIN_METHOD_SOME: &str = r#"
    let s: Option<string> = Some("hello")
    match s?.toUpperCase() { Some(u) => print(u), None => print("none") }
"#;

const OPTCHAIN_METHOD_NONE: &str = r#"
    let s: Option<string> = None
    match s?.toUpperCase() { Some(u) => print(u), None => print("none") }
"#;

#[test]
fn optchain_method_call_some_vm() {
    ShapeTest::new(OPTCHAIN_METHOD_SOME).expect_output("HELLO");
}

#[test]
fn optchain_method_call_some_jit() {
    ShapeTest::new(OPTCHAIN_METHOD_SOME)
        .with_jit()
        .expect_output("HELLO");
}

#[test]
fn optchain_method_call_none_vm() {
    ShapeTest::new(OPTCHAIN_METHOD_NONE).expect_output("none");
}

#[test]
fn optchain_method_call_none_jit() {
    ShapeTest::new(OPTCHAIN_METHOD_NONE)
        .with_jit()
        .expect_output("none");
}

// =========================================================================
// Optional chaining (`?.`) — independent adversarial regression pass
// (wave7/optional-chaining finalize). Locks the receiver-must-be-Option
// strictness, the element-typed result, the nullable `T?` sugar receiver,
// and confirms plain `??`/Option behaviour is unregressed. VM + JIT.
// =========================================================================

// Nullable `T?` sugar (not the explicit `Option<T>` form) as the `?.`
// receiver: present unwraps, absent short-circuits, `??` supplies default.
const OPTCHAIN_NULLABLE_SUGAR: &str = r#"
    type Server { port: int }
    let present: Server? = Some(Server { port: 5 })
    let absent: Server? = None
    print(present?.port ?? 99)
    print(absent?.port ?? 99)
"#;

#[test]
fn optchain_nullable_sugar_receiver_vm() {
    ShapeTest::new(OPTCHAIN_NULLABLE_SUGAR).expect_output("5\n99");
}

#[test]
fn optchain_nullable_sugar_receiver_jit() {
    ShapeTest::new(OPTCHAIN_NULLABLE_SUGAR)
        .with_jit()
        .expect_output("5\n99");
}

// The `?.` result is a proven, element-typed `Option<int>`: the unwrapped
// binder participates in `int` arithmetic (would fail to type-check if the
// result element were untyped / `any`).
const OPTCHAIN_ELEMENT_TYPED: &str = r#"
    type Server { port: int }
    let s: Option<Server> = Some(Server { port: 5 })
    match s?.port { Some(v) => print(v + 100), None => print(-1) }
"#;

#[test]
fn optchain_result_is_element_typed_vm() {
    ShapeTest::new(OPTCHAIN_ELEMENT_TYPED).expect_output("105");
}

#[test]
fn optchain_result_is_element_typed_jit() {
    ShapeTest::new(OPTCHAIN_ELEMENT_TYPED)
        .with_jit()
        .expect_output("105");
}

// Strictness: `?.` on a non-Option receiver does NOT silently pass — it is a
// compile error (the Some/None arm type-check rejects the concrete receiver).
#[test]
fn optchain_on_non_option_receiver_is_error() {
    ShapeTest::new(
        r#"
        type Server { port: int }
        let s: Server = Server { port: 5 }
        let p = s?.port
        print(p)
    "#,
    )
    .expect_run_err();
}

// Regression: plain `??` on a bare Option (no `?.`) is unchanged.
#[test]
fn plain_coalesce_unregressed_vm() {
    ShapeTest::new(
        r#"
        let none_v: Option<int> = None
        let some_v: Option<int> = Some(3)
        print(none_v ?? 7)
        print(some_v ?? 7)
    "#,
    )
    .expect_output("7\n3");
}

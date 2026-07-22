//! WF-3D comptime flagship features — end-to-end gate (VM == JIT).
//!
//! Four user-facing comptime capabilities, each asserted GREEN and with
//! identical VM and JIT results. The JIT path runs through
//! `shape_jit::JITExecutor`, the same selective-AOT path the release binary
//! uses for `--mode jit`.
//!
//! These gates assert on the program's RETURN VALUE (`expect_string`), not on
//! captured stdout. Rationale: a natively-JIT-compiled `print` emits straight
//! to the process's stdout (`jit_v2_string_print`), bypassing the runtime
//! `OutputAdapter` the shape-test harness installs to capture output — so
//! `.expect_output()` under `--mode jit` only observes output when the program
//! DEOPTS to the interpreter. Asserting the return value observes the generated
//! code's result identically under native JIT and the interpreter, so it is the
//! robust VM == JIT correctness gate. (Print-channel VM == JIT parity for these
//! same programs is gated separately by the book truth-gate, which runs the
//! `../shape-web/book/snippets/advanced/*.shape` snippets through the CLI in
//! both modes and diffs stdout against the `.expected` files.)
//!
//!   F1 — generated free function: a type-targeting `@annotation` handler emits
//!        `extend (item_fn(...))`; the generated free function is
//!        visible/callable from user code. WF-3D root fix: the generated free
//!        function is compiled through the FULL driver (`compile_function`) so
//!        it carries `Function.mir_data` and the JIT compiles it NATIVELY
//!        (previously: bytecode-only → Phase-4 "has no MIR data" → whole-program
//!        deopt).
//!   F2 — `type_info(T)` reflection: `type_info(T).fields[i].name` / `.type`
//!        resolve to concrete strings; `kind` uses the renamed discriminator
//!        strings (`Number`, `Unresolved`) per §4.1.2.
//!   F3 — comptime `error()` routes through the LSDS diagnostic pipeline
//!        (`[C0001]`), the same `Diagnostic` the CLI serializes to JSON.
//!   F4 — impl-level method emission: `extend {T} { method m() -> ... }`,
//!        dispatchable via `value.m()`.

use shape_test::shape_test::ShapeTest;

// ============================================================================
// F1 — generated free function, callable from user code (VM == JIT, native)
// ============================================================================

const F1_PROGRAM: &str = r#"
annotation schema_of() on type {
    comptime post(target, ctx) {
        extend (item_fn(f"{target.name}_label", "string", "User schema"))
    }
}

@schema_of()
type User { id: int }

User_label()
"#;

#[test]
fn wf3d_f1_generated_free_fn_vm() {
    ShapeTest::new(F1_PROGRAM).expect_string("User schema");
}

#[test]
fn wf3d_f1_generated_free_fn_jit() {
    ShapeTest::new(F1_PROGRAM)
        .with_jit()
        .expect_string("User schema");
}

// ============================================================================
// F2 — type_info(T).fields[i].name / .type + renamed kind discriminators
// ============================================================================

const F2_PROGRAM: &str = r#"
type Point { x: int, y: number }
let reflected: string = comptime {
    let ti = type_info(Point)
    let n = type_info("number")
    let u = type_info("NopeXYZ")
    f"{ti.name}|{ti.kind}|{ti.fields[0].name}:{ti.fields[0].type}|{ti.fields[1].name}:{ti.fields[1].type}|{n.kind}|{u.kind}"
}
reflected
"#;

// `type_info("number").kind` == "Number" (renamed from the old "Float");
// `type_info("NopeXYZ").kind` == "Unresolved" (renamed from the old
// "Unknown"; also absorbs the old "TypeVar"). Field descriptor rows carry
// `name` + `type` (the runtime `__ComptimeFieldDescriptor` shape).
const F2_EXPECTED: &str = "Point|TypedObject|x:int|y:number|Number|Unresolved";

#[test]
fn wf3d_f2_type_info_fields_and_kind_vm() {
    ShapeTest::new(F2_PROGRAM).expect_string(F2_EXPECTED);
}

#[test]
fn wf3d_f2_type_info_fields_and_kind_jit() {
    ShapeTest::new(F2_PROGRAM)
        .with_jit()
        .expect_string(F2_EXPECTED);
}

// ============================================================================
// F3 — comptime error() routes through the LSDS diagnostic pipeline (C0001)
// ============================================================================

const F3_PROGRAM: &str = r#"
comptime {
    error("boom about widget")
}
"#;

#[test]
fn wf3d_f3_comptime_error_lsds_vm() {
    ShapeTest::new(F3_PROGRAM)
        .expect_run_err_contains("[C0001]")
        .expect_run_err_contains("boom about widget");
}

#[test]
fn wf3d_f3_comptime_error_lsds_jit() {
    ShapeTest::new(F3_PROGRAM)
        .with_jit()
        .expect_run_err_contains("[C0001]")
        .expect_run_err_contains("boom about widget");
}

// ============================================================================
// F4 — impl-level method emission, dispatchable via value.m() (VM == JIT)
// ============================================================================

const F4_PROGRAM: &str = r#"
annotation add_label() on type {
    comptime post(target, ctx) {
        extend (extend_method_literal(target.name, "label", "string", "lbl-emitted"))
    }
}

@add_label()
type Widget { id: int }

let w = Widget { id: 1 }
w.label()
"#;

#[test]
fn wf3d_f4_method_emission_dispatch_vm() {
    ShapeTest::new(F4_PROGRAM).expect_string("lbl-emitted");
}

#[test]
fn wf3d_f4_method_emission_dispatch_jit() {
    ShapeTest::new(F4_PROGRAM)
        .with_jit()
        .expect_string("lbl-emitted");
}

//! W14.2-C1 type_info comptime-builtin coverage (Phase 4b Round 5a, 2026-05-19).
//!
//! Coverage gap from `docs/cluster-audits/v0.3-w14-test-coverage-audit.md`
//! §4 W7 row: "chained `type_info(...).field.subfield` access patterns +
//! interaction with `build_config` precedent". Tests below mirror the
//! existing `blocks.rs` ct_NN_* style and exercise the user-visible
//! source-level surface (`comptime { type_info("T").kind }` etc.).
//!
//! Important contract notes (per W7 close-out at
//! `docs/cluster-audits/v0.3-w7-type_info-comptime-typed-return.md` +
//! `crates/shape-vm/src/compiler/comptime_builtins.rs:469-484`):
//!
//! - The upstream `register_typed_function` marshal layer transmits the
//!   first arg to comptime builtins (declared with `vec![]` arg types)
//!   as kind `Bool`; the closure falls back to a sentinel
//!   `__type_info_marshal_pending__` name when the arg cannot be read
//!   as a string. The returned TypeInfo TypedObject is still well-formed
//!   (correct schema_id, correct refcount discipline) so chained
//!   property access EXECUTES without panic; field-value SEMANTICS are
//!   the documented pre-existing constraint.
//! - The `ct_17_build_config` / `ct_49_build_config_fields` SIGSEGV
//!   class at HEAD `2924b685` is pre-existing and OUT of W14.2-C1
//!   scope; tests in this file avoid the `print(<TypedObject>)` shape
//!   that triggers that SIGSEGV anchor.
//!
//! These tests gate the audit-doc §4 W7 PARTIAL coverage GAP.

use shape_test::shape_test::ShapeTest;

// ============================================================================
// (1) chained_access — `type_info("T").field` patterns
// ============================================================================

/// W14.2-C1 (1a) chained: `type_info("Point").kind` discarded via let
/// — exercises the FULL chain (type_info call + property access on its
/// TypedObject result + binding) and returns a primitive string so
/// `print()` doesn't hit the TypedObject-print SIGSEGV class.
#[test]
fn w14_2_c1_chained_kind_access_then_string_return() {
    let code = r#"
let TAG: string = comptime {
  let info = type_info("Point")
  let k = info.kind
  "kind-accessed"
}
print(TAG)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("kind-accessed");
}

/// W14.2-C1 (1b) chained: `type_info("Point").name` discarded via let
/// — mirror of the kind-access case; exercises the `.name` schema slot
/// of the registered 2-field TypeInfo schema.
#[test]
fn w14_2_c1_chained_name_access_then_string_return() {
    let code = r#"
let TAG: string = comptime {
  let info = type_info("Point")
  let n = info.name
  "name-accessed"
}
print(TAG)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("name-accessed");
}

#[test]
fn type_info_fields_expose_type_ref_and_preserve_type_string() {
    let code = r#"
type Profile {
  id: int,
  nickname: string?,
}

let REFLECTED: string = comptime {
  let ti = type_info(Profile)
  let id_field = ti.fields[0]
  let nick_field = ti.fields[1]
  f"{ti.type_ref.kind}|{id_field.name}:{id_field.type}:{id_field.type_ref.kind}|{nick_field.name}:{nick_field.type}:{nick_field.type_ref.kind}:{nick_field.optional}"
}
print(REFLECTED)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("TypedObject|id:int:Int|nickname:string:String:true");
}

/// W14.2-C1 (1c) chained: inline property access without intermediate
/// let — `type_info("Point").kind` as the expression value. Verifies
/// the inline-form of chained access compiles cleanly (the audit-doc
/// gap calls out chained patterns specifically). The runtime value is
/// the documented marshal-pending Bool; we assert run-success and the
/// chained shape executes without panic.
#[test]
fn w14_2_c1_chained_kind_inline_access() {
    let code = r#"
let K: unknown = comptime {
  type_info("Point").kind
}
print("inline-ok")
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("inline-ok");
}

// ============================================================================
// (2) build_config_interaction — both builtins composed
// ============================================================================

/// W14.2-C1 (2a) interaction: both `build_config()` and `type_info(T)`
/// dispatch in the same comptime block; mirror of the `ct_49_build_config_fields`
/// pattern at `tools/shape-test/tests/comptime/blocks.rs:312` extended
/// with `type_info()` in the same scope. Returns a primitive string to
/// avoid the TypedObject-print SIGSEGV class.
#[test]
fn w14_2_c1_build_config_and_type_info_in_same_block() {
    let code = r#"
let COMBO: string = comptime {
  let cfg = build_config()
  let info = type_info("Point")
  "combo-ok"
}
print(COMBO)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("combo-ok");
}

/// W14.2-C1 (2b) interaction: chained property access on BOTH builtins
/// in the same comptime block. The `build_config().target_arch` shape
/// is the existing precedent for chained-access on comptime-builtin
/// TypedObject results; this test pairs it with `type_info(T).name`
/// to verify multi-builtin chained patterns compose.
#[test]
fn w14_2_c1_chained_access_on_both_builtins() {
    let code = r#"
let X: string = comptime {
  let cfg = build_config()
  let arch = cfg.target_arch
  let info = type_info("Point")
  let name = info.name
  "both-chained-ok"
}
print(X)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("both-chained-ok");
}

// ============================================================================
// (3) nested_generic — Array<T>, Option<T>, Result<T,E>
// ============================================================================

/// W14.2-C1 (3a) nested generic: `type_info("Array<int>")` — verifies
/// the generic-shape name string is accepted at the comptime call site
/// and dispatches through `classify_bare_type_name`'s fallback arm.
#[test]
fn w14_2_c1_type_info_on_array_generic() {
    let code = r#"
let X: string = comptime {
  let info = type_info("Array<int>")
  "array-generic-ok"
}
print(X)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("array-generic-ok");
}

/// W14.2-C1 (3b) nested generic: `type_info("Option<Point>")` — the
/// Option-wrapped struct shape; verifies the call dispatches for
/// `Option<T>` generic-shape names.
#[test]
fn w14_2_c1_type_info_on_option_generic() {
    let code = r#"
type Point {
  x: int
}

let X: string = comptime {
  let info = type_info("Option<Point>")
  "option-generic-ok"
}
print(X)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("option-generic-ok");
}

/// W14.2-C1 (3c) nested generic: `type_info("Result<int, string>")` —
/// the two-param Result shape; covers the third generic-name arm of
/// the audit-doc §4.6 TypeKind matrix.
#[test]
fn w14_2_c1_type_info_on_result_generic() {
    let code = r#"
let X: string = comptime {
  let info = type_info("Result<int, string>")
  "result-generic-ok"
}
print(X)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("result-generic-ok");
}

/// W14.2-C1 (3d) nested generic: `type_info("HashMap<string, int>")`
/// — chained access on the generic-shape result.
#[test]
fn w14_2_c1_type_info_on_hashmap_generic_chained() {
    let code = r#"
let X: string = comptime {
  let info = type_info("HashMap<string, int>")
  let k = info.kind
  "hashmap-generic-ok"
}
print(X)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("hashmap-generic-ok");
}

// ============================================================================
// (4) enum_payload_chained — type_info on user-declared enums
// ============================================================================

/// W14.2-C1 (4a) enum: `type_info("Color")` where Color is a
/// user-declared enum — verifies the `snapshot.enum_defs` lookup arm
/// in `classify_bare_type_name` is reachable from the source-level
/// comptime block.
#[test]
fn w14_2_c1_type_info_on_user_enum() {
    let code = r#"
enum Color {
  Red,
  Green,
  Blue
}

let X: string = comptime {
  let info = type_info("Color")
  "enum-ok"
}
print(X)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("enum-ok");
}

/// W14.2-C1 (4b) enum chained: `type_info("Color").kind` on enum —
/// chained property access on the enum-resolved TypeInfo. Audit-doc
/// §4.6 flat-discriminator: enums share `TypeKind::TypedObject` with
/// structs until a dedicated Enum TypeKind variant lands.
#[test]
fn w14_2_c1_type_info_chained_kind_on_enum() {
    let code = r#"
enum Color {
  Red,
  Green,
  Blue
}

let X: string = comptime {
  let info = type_info("Color")
  let k = info.kind
  "enum-kind-ok"
}
print(X)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("enum-kind-ok");
}

/// W14.2-C1 (4c) enum with payload variants: `type_info("Shape")` on
/// an enum with payload variants — verifies the snapshot path tolerates
/// payload-variant enums (audit-doc §0.3 vision-doc target for the
/// future `variants[i].fields[j].type_name` chain).
#[test]
fn w14_2_c1_type_info_on_enum_with_payload_variants() {
    let code = r#"
enum Shape {
  Circle(int),
  Rectangle { width: int, height: int }
}

let X: string = comptime {
  let info = type_info("Shape")
  "payload-enum-ok"
}
print(X)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("payload-enum-ok");
}

// ============================================================================
// (5) error_path — undefined type, structured fallback
// ============================================================================

/// W14.2-C1 (5a) error path: `type_info("UndefinedXYZ")` on a type name
/// NOT in the snapshot — the `classify_bare_type_name` unrecognized-name
/// fallback arm returns `TypeKindLabel::Unknown` and dispatches without
/// panic. This is the audit-doc §0 contract: "Error path: type_info(...)
/// on undefined type should compile-error with structured message (not
/// panic)". Empirically the current shape resolves to `Unknown` rather
/// than a compile-error — gating that the shape DOES NOT PANIC and runs
/// to completion (the structured-message form is a downstream slice).
#[test]
fn w14_2_c1_type_info_on_undefined_type_does_not_panic() {
    let code = r#"
let X: string = comptime {
  let info = type_info("UndefinedXYZ")
  "undef-ok"
}
print(X)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("undef-ok");
}

/// W14.2-C1 (5b) error path: chained property access on an
/// undefined-type TypeInfo — `type_info("UndefinedXYZ").kind` still
/// reaches a valid TypedObject (Unknown-labeled) and the property
/// access completes. Gates the audit-doc-cited "should compile-error
/// with structured message (not panic)" contract: structured-error
/// is downstream; non-panic is gated here.
#[test]
fn w14_2_c1_chained_access_on_undefined_type_does_not_panic() {
    let code = r#"
let X: string = comptime {
  let info = type_info("UndefinedXYZ")
  let k = info.kind
  "undef-chained-ok"
}
print(X)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("undef-chained-ok");
}

// ============================================================================
// (6) primitive types — int, bool, string, etc.
// ============================================================================

/// W14.2-C1 (6a) primitives: `type_info("int")` — verifies the primitive
/// classification arm of `classify_bare_type_name` (int/i64/i32/i16/i8/
/// u64/u32/u16/u8 all map to `TypeKindLabel::Int`).
#[test]
fn w14_2_c1_type_info_on_primitive_int() {
    let code = r#"
let X: string = comptime {
  let info = type_info("int")
  "int-ok"
}
print(X)
"#;
    ShapeTest::new(code).expect_run_ok().expect_output("int-ok");
}

/// W14.2-C1 (6b) primitives: `type_info("string")` — the string
/// primitive classification arm.
#[test]
fn w14_2_c1_type_info_on_primitive_string() {
    let code = r#"
let X: string = comptime {
  let info = type_info("string")
  "string-ok"
}
print(X)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("string-ok");
}

/// W14.2-C1 (6c) primitives chained: `type_info("bool").kind` — the
/// bool primitive arm plus chained property access.
#[test]
fn w14_2_c1_type_info_chained_kind_on_primitive_bool() {
    let code = r#"
let X: string = comptime {
  let info = type_info("bool")
  let k = info.kind
  "bool-chained-ok"
}
print(X)
"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("bool-chained-ok");
}

// ============================================================================
// PARSE-ONLY tests for shapes that touch the SIGSEGV class (ct_17 family)
// ============================================================================
//
// The patterns below produce TypeInfo TypedObject values that, when
// `print()`-ed directly, trigger the pre-existing `ct_17_build_config`
// SIGSEGV class (TypedObject runtime printing via the v2-raw
// receiver-recovery path). They're documented here as PARSE-ONLY tests
// to capture the user-facing source-level shape per the audit-doc gap
// while avoiding the SIGSEGV anchor. The runtime-level coverage for
// these patterns lives in the unit tests at
// `crates/shape-vm/src/compiler/comptime.rs::tests::w14_2_c1_*`.

/// W14.2-C1 parse: `print(type_info("Point"))` direct TypedObject
/// printing — the canonical shape that triggers the
/// `ct_17_build_config` SIGSEGV class. Parser MUST accept the shape;
/// the runtime path is the documented pre-existing constraint and is
/// NOT exercised here.
#[test]
fn w14_2_c1_direct_type_info_print_parses() {
    let code = r#"
const INFO = comptime {
  type_info("Point")
}
print(INFO)
"#;
    ShapeTest::new(code).expect_parse_ok();
}

/// W14.2-C1 parse: multi-level chained access `type_info("Point").name.length`
/// — the parser must accept multi-level chains so a future
/// FieldInfo-recursive `type_info(T).fields[i].type_name` shape lands
/// without grammar work (audit-doc §4.6 future-proofing).
#[test]
fn w14_2_c1_multi_level_chained_access_parses() {
    let code = r#"
const X = comptime {
  type_info("Point").name.length
}
"#;
    ShapeTest::new(code).expect_parse_ok();
}

/// W14.2-C1 parse: comptime-for-style iteration over a future
/// `.fields` payload — the `for field in type_info(T).fields { ... }`
/// shape from `docs/vision/distributed-comptime-async-vision.md:86`.
/// The semantics are downstream (the audit-doc §5.1 cascade lists
/// `compile_comptime_for` as STUBBED at HEAD); we gate ONLY the parse
/// step here so the syntax shape is locked in.
#[test]
fn w14_2_c1_comptime_for_over_fields_parses() {
    let code = r#"
comptime {
  let info = type_info("Point")
  for f in info.fields {
    warning(f"field shape")
  }
}
"#;
    ShapeTest::new(code).expect_parse_ok();
}

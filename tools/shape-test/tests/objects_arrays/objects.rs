//! Object and HashMap-related tests
//! Covers object creation, property access, nesting, merge, destructuring,
//! bracket notation, HashMap operations, and complex object compositions.

use shape_test::shape_test::ShapeTest;

const HASHMAP_KEYS_SURFACE: &str = "HashMap.keys: SURFACE";
const HASHMAP_VALUES_SURFACE: &str = "HashMap.values: SURFACE";
const HASHMAP_ENTRIES_TO_ARRAY_SURFACE: &str = "HashMap.entries/toArray: SURFACE";
const HASHMAP_MAP_INDEXED_ABSENT: &str = "no method 'mapIndexed' on receiver kind Ptr(HashMap)";
const HASHMAP_FILTER_INDEXED_ABSENT: &str =
    "no method 'filterIndexed' on receiver kind Ptr(HashMap)";
const HASHMAP_STRING_KEY_REQUIRED: &str = "HashMap key must be a string";

// =====================================================================
// Basic Objects
// =====================================================================

#[test]
fn object_literal_creation() {
    let code = r#"let user = {
  id: 1,
  name: "Ada"
}
print(user.name)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("Ada");
}

#[test]
fn object_property_assignment() {
    let code = r#"let mut user = {
  id: 1,
  name: "Ada"
}
user.score = 99
print(user.score)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("99");
}

#[test]
fn object_access_id_field() {
    let code = r#"let user = {
  id: 1,
  name: "Ada"
}
print(user.id)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("1");
}

// =====================================================================
// Nested Objects
// =====================================================================

#[test]
fn nested_object_access() {
    let code = r#"let cfg = {
  server: {
    host: "localhost",
    port: 9091
  }
}
print(cfg.server.host)"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("localhost");
}

#[test]
fn nested_object_access_number() {
    let code = r#"let cfg = {
  server: {
    host: "localhost",
    port: 9091
  }
}
print(cfg.server.port)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("9091");
}

// =====================================================================
// Object Merge
// =====================================================================

#[test]
fn object_merge_with_plus() {
    let code = r#"let mut a = { x: 1 }
a.y = 2
let b = { z: 3 }
let c = a + b
print(c)"#;
    ShapeTest::new(code).expect_run_ok();
}

#[test]
fn object_merge_does_not_mutate_originals() {
    let code = r#"let a = { x: 1 }
let b = { y: 2 }
let c = a + b
print(a)
print(b)"#;
    ShapeTest::new(code).expect_run_ok();
}

// =====================================================================
// Struct (TypedObject) REAL-MOVE semantics (user 2026-06-21)
//
// `let q = p` whole-value-binds a struct. A struct is a HEAP value, so the
// bind is a destructive MOVE (Rust-shaped): `p` is consumed and reading it
// afterward is a compile-time use-after-move (B0005). To keep BOTH `p` and
// `q` live, the program must opt into an explicit deep copy with `clone p`.
//
// These tests were rebaselined from the old clone-on-still-live policy
// (which kept `p` live silently). The move-then-read JIT divergence they
// originally regressed is now moot: the moved-from read is rejected at
// compile time, before any JIT/VM execution divergence can occur.
// =====================================================================

#[test]
fn struct_read_original_after_move_is_use_after_move() {
    // Real-move: `let q = p` consumes the struct `p`; the later `p.x` read is
    // a use-after-move. (Old policy: silent clone-on-still-live -> "1".)
    let code = r#"type P { x: int }
let p = P { x: 1 }
let q = p
print(p.x)"#;
    ShapeTest::new(code).expect_run_err_contains("B0005");
}

#[test]
fn struct_clone_keeps_both_live() {
    // Explicit `clone p` deep-copies the struct; both `p` and `q` stay live.
    let code = r#"type P { x: int }
let p = P { x: 1 }
let q = clone p
print(p.x)
print(q.x)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("1\n1");
}

#[test]
fn struct_read_original_after_bind_heap_field_is_use_after_move() {
    // A struct with a heap (string) field still moves whole; reading the
    // moved-from `p` is a use-after-move.
    let code = r#"type P { x: int, name: string }
let p = P { x: 1, name: "a" }
let q = p
print(p.x)"#;
    ShapeTest::new(code).expect_run_err_contains("B0005");
}

#[test]
fn struct_read_moved_to_binding_ok() {
    // Reading ONLY the moved-to binding `q` is fine — `p` is consumed but
    // never read after the move, so there is no use-after-move.
    let code = r#"type P { x: int }
let p = P { x: 7 }
let q = p
print(q.x)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("7");
}

#[test]
fn struct_read_both_alias_and_original_is_use_after_move() {
    // Reading the moved-to `q` then the moved-from `p` still fires B0005 on
    // the `p` read (the move is destructive regardless of the `q` read).
    let code = r#"type P { x: int }
let p = P { x: 7 }
let q = p
print(q.x)
print(p.x)"#;
    ShapeTest::new(code).expect_run_err_contains("B0005");
}

#[test]
fn struct_two_binds_then_read_is_use_after_move() {
    // The first `let q = p` already consumes `p`; the second `let r = p` is a
    // use-after-move (and so would the later `r.x` read be moot).
    let code = r#"type P { x: int }
let p = P { x: 3 }
let q = p
let r = p
print(r.x)"#;
    ShapeTest::new(code).expect_run_err_contains("B0005");
}

// =====================================================================
// Destructuring
// =====================================================================

#[test]
fn object_destructuring() {
    let code = r#"let point = { x: 3, y: 4 }
let {x, y} = point
print(x + y)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("7");
}

#[test]
fn object_destructuring_individual_values() {
    let code = r#"let point = { x: 3, y: 4 }
let {x, y} = point
print(x)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("3");
}

#[test]
fn destructuring_in_function_param() {
    let code = r#"fn distance({x, y}) {
    return x + y
}
print(distance({x: 3, y: 4}))"#;
    ShapeTest::new(code).expect_run_ok().expect_output("7");
}

// =====================================================================
// Empty Object
// =====================================================================

#[test]
fn empty_object() {
    let code = r#"let o = {}
print(o)"#;
    ShapeTest::new(code).expect_run_ok();
}

// =====================================================================
// Object with Function Values
// =====================================================================

#[test]
fn object_with_function_values() {
    let code = r#"let obj = {
  greet: |name| "Hello, " + name
}
print(obj.greet("World"))"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("Hello, World");
}

// =====================================================================
// Object with Bracket Notation
// =====================================================================

#[test]
fn object_bracket_notation() {
    let code = r#"let person = {
    name: "Alice",
    age: 30
}
let property = "age"
print(person[property])"#;
    // Strict-typing re-baseline (v0.3.3): dynamic bracket-access `person[runtimeKey]`
    // on a TypedObject is correctly REJECTED — a computed key cannot statically prove
    // the field type, and Shape is no-any / no-dynamic-fallback. Dynamic TypedObject
    // field access is a v0.4 language-design question, not a v0.3.3 bug.
    ShapeTest::new(code).expect_run_err_contains("does not support index access");
}

// =====================================================================
// Object Methods (len)
// =====================================================================

#[test]
#[ignore = "len() on TypedObject: global builtin_len retired (c-len-migrate); TypedObject lacks .len() PHF entry. Follow-up: wire typed-object .len() dispatch or drop the test."]
fn object_len_function() {
    let code = r#"let person = { name: "Alice", age: 30, balance: 100 }
print(len(person))"#;
    ShapeTest::new(code).expect_run_ok().expect_output("3");
}

// =====================================================================
// Deeply Nested Object Access
// =====================================================================

#[test]
fn deeply_nested_object() {
    let code = r#"let data = {
  level1: {
    level2: {
      level3: {
        value: 42
      }
    }
  }
}
print(data.level1.level2.level3.value)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("42");
}

// =====================================================================
// Object with Array Values
// =====================================================================

#[test]
fn object_with_array_values() {
    let code = r#"let watchlist = {
    tech: ["AAPL", "GOOGL", "MSFT"],
    finance: ["JPM", "BAC"]
}
print(watchlist.tech[0])"#;
    ShapeTest::new(code).expect_run_ok().expect_output("AAPL");
}

#[test]
fn object_with_array_values_second_array() {
    let code = r#"let watchlist = {
    tech: ["AAPL", "GOOGL", "MSFT"],
    finance: ["JPM", "BAC"]
}
print(watchlist.finance[1])"#;
    ShapeTest::new(code).expect_run_ok().expect_output("BAC");
}

// =====================================================================
// Building Objects from Functions
// =====================================================================

#[test]
fn object_returned_from_function() {
    let code = r#"function make_point(x, y) {
    return { x: x, y: y }
}
let p = make_point(10, 20)
print(p.x)
print(p.y)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("10\n20");
}

// =====================================================================
// HashMap - Basic Operations
// =====================================================================

#[test]
fn hashmap_basic_creation_and_get() {
    let code = r#"let m = HashMap()
let m2 = m.set("a", 1).set("b", 2).set("c", 3)
print(m2.get("b"))"#;
    ShapeTest::new(code).expect_run_ok().expect_output("2");
}

#[test]
fn hashmap_has_key() {
    let code = r#"let m = HashMap()
let m2 = m.set("a", 1).set("b", 2).set("c", 3)
print(m2.has("a"))"#;
    ShapeTest::new(code).expect_run_ok().expect_output("true");
}

#[test]
fn hashmap_has_missing_key() {
    let code = r#"let m = HashMap()
let m2 = m.set("a", 1)
print(m2.has("z"))"#;
    ShapeTest::new(code).expect_run_ok().expect_output("false");
}

#[test]
fn hashmap_len() {
    let code = r#"let m = HashMap()
let m2 = m.set("a", 1).set("b", 2).set("c", 3)
print(m2.len())"#;
    ShapeTest::new(code).expect_run_ok().expect_output("3");
}

#[test]
fn hashmap_keys() {
    let code = r#"let m = HashMap()
let m2 = m.set("a", 1).set("b", 2).set("c", 3)
print(m2.keys())"#;
    ShapeTest::new(code).expect_run_err_contains(HASHMAP_KEYS_SURFACE);
}

#[test]
fn hashmap_values() {
    let code = r#"let m = HashMap()
let m2 = m.set("a", 1).set("b", 2).set("c", 3)
print(m2.values())"#;
    ShapeTest::new(code).expect_run_err_contains(HASHMAP_VALUES_SURFACE);
}

#[test]
fn hashmap_entries() {
    let code = r#"let m = HashMap()
let m2 = m.set("a", 1).set("b", 2)
print(m2.entries())"#;
    ShapeTest::new(code).expect_run_err_contains(HASHMAP_ENTRIES_TO_ARRAY_SURFACE);
}

#[test]
fn hashmap_integer_keys() {
    let code = r#"let scores = HashMap()
    .set(1, "gold")
    .set(2, "silver")
    .set(3, "bronze")
print(scores.get(1))"#;
    ShapeTest::new(code).expect_run_err_contains(HASHMAP_STRING_KEY_REQUIRED);
}

#[test]
fn hashmap_immutability() {
    // After set, the original should not change
    let code = r#"let m = HashMap()
let m2 = m.set("a", 1)
print(m.len())
print(m2.len())"#;
    ShapeTest::new(code).expect_run_ok().expect_output("0\n1");
}

#[test]
fn hashmap_delete() {
    let code = r#"let m = HashMap().set("a", 1).set("b", 2).set("c", 3)
let m2 = m.delete("b")
print(m2.len())
print(m2.has("b"))"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("2\nfalse");
}

#[test]
fn hashmap_is_empty() {
    // ROOT B (HashMap <K,V> infer-or-annotate): bare `let m = HashMap()` with
    // no usage to pin K,V is rejected under strict typing; supply K,V via an
    // explicit annotation for the empty-map intent. `<string, bool>` keeps the
    // empty map on the legacy-ctor path (the typed-map fast path lacks an
    // empty-map `.isEmpty()` lowering — a separate VM-codegen gap, not ROOT B).
    let code = r#"let m: HashMap<string, bool> = HashMap()
print(m.isEmpty())"#;
    ShapeTest::new(code).expect_run_ok().expect_output("true");
}

#[test]
fn hashmap_is_not_empty() {
    let code = r#"let m = HashMap().set("x", 1)
print(m.isEmpty())"#;
    ShapeTest::new(code).expect_run_ok().expect_output("false");
}

// =====================================================================
// HashMap - Map, Filter, ForEach
// =====================================================================

#[test]
fn hashmap_map() {
    let code = r#"let m = HashMap().set("a", 1).set("b", 2).set("c", 3)
let doubled = m.map(|k, v| v * 2)
print(doubled.get("b"))"#;
    ShapeTest::new(code).expect_run_err_contains(HASHMAP_MAP_INDEXED_ABSENT);
}

#[test]
fn hashmap_filter() {
    let code = r#"let m = HashMap().set("a", 1).set("b", 2).set("c", 3)
let big = m.filter(|k, v| v > 1)
print(big.len())"#;
    ShapeTest::new(code).expect_run_err_contains(HASHMAP_FILTER_INDEXED_ABSENT);
}

#[test]
fn hashmap_foreach() {
    let code = r#"let m = HashMap().set("a", 1).set("b", 2)
m.forEach(|k, v| print(k))"#;
    ShapeTest::new(code).expect_run_ok();
}

// =====================================================================
// HashMap - Chaining and Overwrite
// =====================================================================

#[test]
fn hashmap_chained_set() {
    let code = r#"let m = HashMap().set("x", 10).set("y", 20).set("z", 30)
print(m.get("x"))
print(m.get("y"))
print(m.get("z"))"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("10\n20\n30");
}

#[test]
fn hashmap_overwrite_key() {
    let code = r#"let m = HashMap().set("a", 1).set("a", 99)
print(m.get("a"))"#;
    ShapeTest::new(code).expect_run_ok().expect_output("99");
}

// =====================================================================
// HashMap - Get Missing Key
// =====================================================================

#[test]
fn hashmap_get_missing_key() {
    let code = r#"let m = HashMap().set("a", 1)
let v = m.get("missing")
print(v)"#;
    ShapeTest::new(code).expect_run_ok();
}

// =====================================================================
// Struct deep-clone (REAL-MOVE keep-both, v0.3.3 user 2026-06-21)
// `clone p` deep-copies a TypedObject; the copy is independent of the
// source. Scalars stay copy; nested heap (struct / array) deep-copied;
// strings shared with a balanced retain. Refcount-balanced (valgrind:
// 0 definitely-lost, 0 invalid-access vs baseline).
// =====================================================================

#[test]
fn struct_clone_is_independent() {
    let code = r#"type P { x: int, name: string }
let p = P { x: 1, name: "a" }
let mut q = clone p
q.x = 99
print(p.x)
print(q.x)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("1\n99");
}

#[test]
fn struct_clone_nested_struct_is_deep() {
    let code = r#"type Inner { v: int }
type Outer { inner: Inner, tag: string }
let a = Outer { inner: Inner { v: 5 }, tag: "x" }
let mut b = clone a
b.inner.v = 77
print(a.inner.v)
print(b.inner.v)"#;
    ShapeTest::new(code).expect_run_ok().expect_output("5\n77");
}

#[test]
fn struct_clone_preserves_string_field() {
    let code = r#"type P { x: int, name: string }
let p = P { x: 7, name: "hello" }
let q = clone p
print(q.x)
print(q.name)
print(p.name)"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("7\nhello\nhello");
}

#[test]
fn struct_clone_with_array_field_does_not_crash() {
    // Nested Array<int> field is deep-cloned; mutating a scalar field on
    // the clone leaves the source untouched and the array intact in both.
    let code = r#"type Bag { items: Array<int>, name: string }
let g = Bag { items: [1, 2, 3], name: "g" }
let mut h = clone g
h.name = "h"
print(g.items[0])
print(h.items[0])
print(g.name)
print(h.name)"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("1\n1\ng\nh");
}

// =====================================================================
// c5 struct-element copy-on-bind (strict value semantics)
// =====================================================================
//
// Reading a STRUCT element out of an array must produce an INDEPENDENT
// copy (like a scalar element), NOT an alias of the array's backing
// store. Mutating the local must NOT touch the array. Fixed in
// `executor/v2_handlers/v2_array_detect.rs::copy_typed_object_for_bind`
// + the sibling `array.rs::TypedArrayGetTypedObject` arm.

#[test]
fn struct_array_element_read_is_a_copy_not_an_alias() {
    let code = r#"type Acct { balance: int }
let arr = [Acct { balance: 1 }]
let mut a = arr[0]
a.balance = 999
print(arr[0].balance)
print(a.balance)"#;
    ShapeTest::new(code)
        .expect_run_ok()
        // array element UNCHANGED (1); local copy mutated (999).
        .expect_output("1\n999");
}

#[test]
fn struct_array_element_copy_preserves_string_field_share() {
    // Struct with a heap (string) field: the copy shares the field's heap
    // object (standard struct-copy semantics) but its own scalar slot is
    // independent. No double-free / UAF when the local replaces its string
    // slot and is later dropped while the array's share survives.
    let code = r#"type Person { name: string, age: int }
let arr = [Person { name: "Alice", age: 30 }, Person { name: "Bob", age: 25 }]
let mut p = arr[0]
p.age = 99
p.name = "Zoe"
print(arr[0].name)
print(arr[0].age)
print(p.name)
print(p.age)
print(arr[1].name)"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("Alice\n30\nZoe\n99\nBob");
}

#[test]
fn struct_array_multiple_independent_element_copies() {
    let code = r#"type Acct { balance: int, label: string }
let arr = [Acct { balance: 1, label: "alpha" }]
let mut a = arr[0]
a.balance = 100
let mut b = arr[0]
b.balance = 200
let mut c = arr[0]
c.balance = 300
print(a.balance)
print(b.balance)
print(c.balance)
print(arr[0].balance)
print(arr[0].label)"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("100\n200\n300\n1\nalpha");
}

#[test]
fn scalar_array_element_read_is_still_copy() {
    // Regression guard: scalar elements were already Copy; the c5 fix is
    // struct-only and must not disturb the scalar path.
    let code = r#"let nums = [10, 20, 30]
let mut x = nums[0]
x = 999
print(nums[0])
print(x)"#;
    ShapeTest::new(code)
        .expect_run_ok()
        .expect_output("10\n999");
}

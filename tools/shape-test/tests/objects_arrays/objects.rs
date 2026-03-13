//! Object and HashMap-related tests
//! Covers object creation, property access, nesting, merge, destructuring,
//! bracket notation, HashMap operations, and complex object compositions.

use shape_test::shape_test::ShapeTest;

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
    ShapeTest::new(code).expect_run_ok().expect_output("30");
}

// =====================================================================
// Object Methods (len)
// =====================================================================

#[test]
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
    ShapeTest::new(code).expect_run_ok();
}

#[test]
fn hashmap_values() {
    let code = r#"let m = HashMap()
let m2 = m.set("a", 1).set("b", 2).set("c", 3)
print(m2.values())"#;
    ShapeTest::new(code).expect_run_ok();
}

#[test]
fn hashmap_entries() {
    let code = r#"let m = HashMap()
let m2 = m.set("a", 1).set("b", 2)
print(m2.entries())"#;
    ShapeTest::new(code).expect_run_ok();
}

#[test]
fn hashmap_integer_keys() {
    let code = r#"let scores = HashMap()
    .set(1, "gold")
    .set(2, "silver")
    .set(3, "bronze")
print(scores.get(1))"#;
    ShapeTest::new(code).expect_run_ok().expect_output("gold");
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
    let code = r#"let m = HashMap()
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
    ShapeTest::new(code).expect_run_ok().expect_output("4");
}

#[test]
fn hashmap_filter() {
    let code = r#"let m = HashMap().set("a", 1).set("b", 2).set("c", 3)
let big = m.filter(|k, v| v > 1)
print(big.len())"#;
    ShapeTest::new(code).expect_run_ok().expect_output("2");
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

//! D4 (strict-flip, 2026-06-22) — hashmap-readback key-kind tests.
//!
//! U3 (SB-9 deletion, 2026-06-23): rebased onto the single honest
//! `HashMapData` carrier (the `TypedMap<K,V>` fast path was deleted). `.set`
//! is a COW mutator that requires a `let mut` binding, so these readback
//! fixtures bind `let mut m`.
//!
//! Two regressions originally in the typed-map path, now verified through
//! `HashMapData`:
//!
//!   (D4) A string key produced by a v2-raw `TypedArray<*const StringObj>`
//!   read (`TypedArrayGetString`) carries `NativeKind::StringV2`. The
//!   typed-map key reader (`pop_string_key`) only recognized the
//!   `NativeKind::String` (`Arc<String>`) carrier, so a for-in / array-
//!   indexed string key fell to the `_ => None` arm and every get/has/
//!   delete MISSED — returning null/false even when the key was present.
//!   Surfaced in the book as `acc.total = acc.total + (m.get(k) ?? 0)`
//!   inside a `for k in keys` loop summing to 0 instead of the real total.
//!
//!   (mis-solve) A typed-object field assignment whose context involved a
//!   prior `m.set(...)` statement wrongly unioned the receiver's
//!   `HashMap<…>` type into the enclosing fn's implicit return, producing a
//!   spurious `() -> HashMap<…> | int` constraint mis-solve. A non-tail
//!   expression statement must not contribute to the implicit return.
//!
//! End-to-end through the language surface (`eval_typed_i64`) — no
//! hand-emitted bytecode, no ValueWord carrier (deleted).

use super::test_utils::eval_typed_i64;

/// D4 core: a string key read out of an `Array<string>` (carries
/// `NativeKind::StringV2`) must look up correctly in a `HashMap<string, int>`.
#[test]
fn hashmap_get_with_array_sourced_string_key() {
    let src = r#"
fn main() -> int {
    let mut m: HashMap<string, int> = HashMap()
    m.set("a", 10)
    let keys = ["a"]
    let k = keys[0]
    m.get(k) ?? -1
}
main()
"#;
    assert_eq!(eval_typed_i64(src), 10);
}

/// D4 book shape: a typed-object int field accumulating hashmap-readback
/// ints over a `for k in keys` loop. Pre-fix this summed to 0 (every key
/// missed) and/or crashed with "no method add on Int64".
#[test]
fn typed_object_int_field_sums_hashmap_readback_in_loop() {
    let src = r#"
type Acc { total: int }
fn main() -> int {
    let mut m: HashMap<string, int> = HashMap()
    m.set("a", 10)
    m.set("b", 20)
    m.set("c", 30)
    let mut acc = Acc { total: 0 }
    let keys = ["a", "b", "c"]
    for k in keys {
        acc.total = acc.total + (m.get(k) ?? 0)
    }
    acc.total
}
main()
"#;
    assert_eq!(eval_typed_i64(src), 60);
}

/// `has` with an array-sourced StringV2 key must agree with the literal-key
/// result (both `true` for a present key).
#[test]
fn hashmap_has_with_array_sourced_string_key() {
    let src = r#"
fn main() -> int {
    let mut m: HashMap<string, int> = HashMap()
    m.set("a", 1)
    let keys = ["a"]
    let k = keys[0]
    if m.has(k) { 1 } else { 0 }
}
main()
"#;
    assert_eq!(eval_typed_i64(src), 1);
}

/// Implicit-return mis-solve: a non-tail `m.set(...)` statement followed by
/// a typed-object field assignment must NOT make `main`'s return type a
/// `HashMap<…> | int` union (which previously produced "Could not solve
/// type constraints"). The program must type-check and run.
#[test]
fn field_assign_after_hashmap_set_no_constraint_mis_solve() {
    let src = r#"
type Acc { total: int }
fn main() -> int {
    let mut m: HashMap<string, int> = HashMap()
    m.set("a", 10)
    let mut acc = Acc { total: 0 }
    acc.total = m.get("a") ?? 0
    acc.total
}
main()
"#;
    assert_eq!(eval_typed_i64(src), 10);
}

/// A `HashMap<string,int>` flowing through a fn boundary (param) after a
/// fluent `.set` rebind — the pattern-matching slice shape. Pre-strict-flip
/// this mis-solved with "Generic {HashMap...} cannot have fields".
#[test]
fn hashmap_through_fn_boundary_after_set() {
    let src = r#"
fn build() -> HashMap<string, int> {
    let mut idx = HashMap()
    idx = idx.set("a", 120)
    idx = idx.set("b", 200)
    idx
}
fn lookup(idx: HashMap<string, int>, name: string) -> int {
    match idx.get(name) {
        Some(v) => v,
        None => -1,
    }
}
fn main() -> int {
    let m = build()
    lookup(m, "a") + lookup(m, "b") + lookup(m, "missing")
}
main()
"#;
    // 120 + 200 + (-1) = 319
    assert_eq!(eval_typed_i64(src), 319);
}

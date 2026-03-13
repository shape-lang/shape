//! Stress tests for enum match expressions — wildcards, nested, inline, loops, conditionals.
//!
//! Migrated from shape-vm stress_17_enums.rs — Sections 4, 12-18, 24, 27, 29, 31-32, 34, 36-37, 43, 48, 52, 55-56, 64, 66, 68, 73.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// SECTION 4: Match on Enum (All Variants Covered)
// =============================================================================

/// Verifies match returns int from enum.
#[test]
fn test_match_returns_int_from_enum() {
    ShapeTest::new(
        "enum Fruit { Apple, Banana, Cherry }\nlet f = Fruit::Banana\nmatch f { Fruit::Apple => 1, Fruit::Banana => 2, Fruit::Cherry => 3, }",
    )
    .expect_number(2.0);
}

/// Verifies match returns string from enum.
#[test]
fn test_match_returns_string_from_enum() {
    ShapeTest::new(
        r#"enum Animal { Cat, Dog, Bird }
let a = Animal::Cat
match a { Animal::Cat => "meow", Animal::Dog => "woof", Animal::Bird => "tweet", }"#,
    )
    .expect_string("meow");
}

/// Verifies match returns bool true from enum.
#[test]
fn test_match_returns_bool_from_enum() {
    ShapeTest::new(
        "enum YesNo { Yes, No }\nlet v = YesNo::Yes\nmatch v { YesNo::Yes => true, YesNo::No => false, }",
    )
    .expect_bool(true);
}

/// Verifies match returns bool false from enum.
#[test]
fn test_match_returns_bool_false_from_enum() {
    ShapeTest::new(
        "enum YesNo { Yes, No }\nlet v = YesNo::No\nmatch v { YesNo::Yes => true, YesNo::No => false, }",
    )
    .expect_bool(false);
}

/// Verifies match on all four variants.
#[test]
fn test_match_all_four_variants() {
    ShapeTest::new(
        "enum Season { Spring, Summer, Autumn, Winter }\nlet s = Season::Autumn\nmatch s { Season::Spring => 1, Season::Summer => 2, Season::Autumn => 3, Season::Winter => 4, }",
    )
    .expect_number(3.0);
}

// =============================================================================
// SECTION 12: Wildcard/Default Match
// =============================================================================

/// Verifies wildcard catches remaining variants.
#[test]
fn test_match_wildcard_catches_remaining() {
    ShapeTest::new(
        r#"enum Color { Red, Green, Blue }
let c = Color::Blue
match c { Color::Red => "red", _ => "other", }"#,
    )
    .expect_string("other");
}

/// Verifies specific arm hit before wildcard.
#[test]
fn test_match_wildcard_with_first_variant() {
    ShapeTest::new(
        r#"enum Color { Red, Green, Blue }
let c = Color::Red
match c { Color::Red => "red", _ => "other", }"#,
    )
    .expect_string("red");
}

/// Verifies wildcard-only match.
#[test]
fn test_match_wildcard_only() {
    ShapeTest::new("enum Color { Red, Green, Blue }\nlet c = Color::Green\nmatch c { _ => 42, }")
        .expect_number(42.0);
}

/// Verifies wildcard with partial coverage.
#[test]
fn test_match_wildcard_partial_coverage() {
    ShapeTest::new(
        r#"enum Dir { N, S, E, W }
let d = Dir::E
match d { Dir::N => "north", Dir::S => "south", _ => "other", }"#,
    )
    .expect_string("other");
}

// =============================================================================
// SECTION 13: Enum Match Expression (Inline)
// =============================================================================

/// Verifies inline match expression.
#[test]
fn test_match_expr_inline() {
    ShapeTest::new(
        "enum Coin { Penny, Nickel, Dime, Quarter }\nlet value = match Coin::Dime { Coin::Penny => 1, Coin::Nickel => 5, Coin::Dime => 10, Coin::Quarter => 25, }\nvalue",
    )
    .expect_number(10.0);
}

/// Verifies match expression in arithmetic.
#[test]
fn test_match_expr_in_arithmetic() {
    ShapeTest::new(
        "enum Bit { Zero, One }\nlet x = match Bit::One { Bit::Zero => 0, Bit::One => 1, }\nx + 100",
    )
    .expect_number(101.0);
}

// =============================================================================
// SECTION 14: Nested Match on Enum
// =============================================================================

/// Verifies nested match on two enums.
#[test]
fn test_nested_match_on_enum() {
    ShapeTest::new(
        "enum Outer { A, B }\nenum Inner { X, Y }\nlet o = Outer::A\nlet i = Inner::Y\nmatch o { Outer::A => match i { Inner::X => 1, Inner::Y => 2, }, Outer::B => match i { Inner::X => 3, Inner::Y => 4, }, }",
    )
    .expect_number(2.0);
}

/// Verifies nested match outer B inner X.
#[test]
fn test_nested_match_outer_b_inner_x() {
    ShapeTest::new(
        "enum Outer { A, B }\nenum Inner { X, Y }\nlet o = Outer::B\nlet i = Inner::X\nmatch o { Outer::A => match i { Inner::X => 1, Inner::Y => 2, }, Outer::B => match i { Inner::X => 3, Inner::Y => 4, }, }",
    )
    .expect_number(3.0);
}

// =============================================================================
// SECTION 15: Enum Passed Through Functions
// =============================================================================

/// Verifies enum create-pass-match chain through functions.
#[test]
fn test_enum_create_pass_match_chain() {
    ShapeTest::new(
        r#"enum Priority { Low, Medium, High }
fn make_priority() -> Priority { Priority::High }
fn describe(p: Priority) -> string { match p { Priority::Low => "low", Priority::Medium => "medium", Priority::High => "high", } }
fn test() -> string { let p = make_priority()
describe(p) }
test()"#,
    )
    .expect_string("high");
}

/// Verifies enum round trip through two functions.
#[test]
fn test_enum_round_trip_through_two_functions() {
    ShapeTest::new(
        r#"enum Mode { Fast, Slow }
fn identity(m: Mode) -> Mode { m }
fn label(m: Mode) -> string { match m { Mode::Fast => "fast", Mode::Slow => "slow", } }
fn test() -> string { let m = Mode::Slow
let m2 = identity(m)
label(m2) }
test()"#,
    )
    .expect_string("slow");
}

// =============================================================================
// SECTION 16: Enum in Conditional / If-Else
// =============================================================================

/// Verifies enum in if condition via match (on).
#[test]
fn test_enum_in_if_condition_via_match() {
    ShapeTest::new(
        "enum Toggle { On, Off }\nlet t = Toggle::On\nlet is_on = match t { Toggle::On => true, Toggle::Off => false, }\nif is_on { 1 } else { 0 }",
    )
    .expect_number(1.0);
}

/// Verifies enum in if condition via match (off).
#[test]
fn test_enum_in_if_condition_off() {
    ShapeTest::new(
        "enum Toggle { On, Off }\nlet t = Toggle::Off\nlet is_on = match t { Toggle::On => true, Toggle::Off => false, }\nif is_on { 1 } else { 0 }",
    )
    .expect_number(0.0);
}

// =============================================================================
// SECTION 17: Enum in Loop
// =============================================================================

/// Verifies enum match in a loop with accumulator.
#[test]
fn test_enum_match_in_loop() {
    ShapeTest::new(
        "enum Step { Inc, Dec, Nop }\nfn test() -> int { let steps = [Step::Inc, Step::Inc, Step::Dec, Step::Nop, Step::Inc]\nlet mut total = 0\nfor s in steps { total = total + match s { Step::Inc => 1, Step::Dec => -1, Step::Nop => 0, } }\ntotal }\ntest()",
    )
    .expect_number(2.0);
}

// =============================================================================
// SECTION 18: Enum in Array
// =============================================================================

/// Verifies matching on enum from array access.
#[test]
fn test_enum_in_array_access() {
    ShapeTest::new(
        r#"enum Color { Red, Green, Blue }
let colors = [Color::Red, Color::Green, Color::Blue]
match colors[1] { Color::Red => "red", Color::Green => "green", Color::Blue => "blue", }"#,
    )
    .expect_string("green");
}

/// Verifies length of enum array.
#[test]
fn test_enum_array_length() {
    ShapeTest::new(
        "enum Bit { Zero, One }\nlet bits = [Bit::One, Bit::Zero, Bit::One, Bit::One]\nbits.length",
    )
    .expect_number(4.0);
}

// =============================================================================
// SECTION 24: Enum and Booleans Combined
// =============================================================================

/// Verifies enum match to bool then && operator.
#[test]
fn test_enum_match_to_bool_then_and() {
    ShapeTest::new(
        "enum Perm { Read, Write, NoPerm }\nlet p1 = Perm::Read\nlet p2 = Perm::Write\nlet has_read = match p1 { Perm::Read => true, _ => false, }\nlet has_write = match p2 { Perm::Write => true, _ => false, }\nhas_read && has_write",
    )
    .expect_bool(true);
}

/// Verifies enum match to bool then || operator.
#[test]
fn test_enum_match_to_bool_or() {
    ShapeTest::new(
        "enum Perm { Read, Write, NoPerm }\nlet p = Perm::NoPerm\nlet has_read = match p { Perm::Read => true, _ => false, }\nlet has_write = match p { Perm::Write => true, _ => false, }\nhas_read || has_write",
    )
    .expect_bool(false);
}

// =============================================================================
// SECTION 27: Enum Construction and Immediate Match (No Variable)
// =============================================================================

/// Verifies immediate match without variable binding.
#[test]
fn test_enum_immediate_match_no_binding() {
    ShapeTest::new("enum Dir { Up, Down }\nmatch Dir::Down { Dir::Up => 1, Dir::Down => 2, }")
        .expect_number(2.0);
}

/// Verifies immediate match with payload.
#[test]
fn test_enum_immediate_match_with_payload() {
    ShapeTest::new(
        "enum Wrapper { Val(int), Empty }\nmatch Wrapper::Val(77) { Wrapper::Val(n) => n, Wrapper::Empty => 0, }",
    )
    .expect_number(77.0);
}

// =============================================================================
// SECTION 29: Enum Match Returning Different Types Across Arms
// =============================================================================

/// Verifies all arms return string.
#[test]
fn test_enum_match_all_arms_return_string() {
    ShapeTest::new(
        r#"enum Level { Low, Med, High }
let l = Level::Med
match l { Level::Low => "low", Level::Med => "medium", Level::High => "high", }"#,
    )
    .expect_string("medium");
}

/// Verifies all arms return number.
#[test]
fn test_enum_match_all_arms_return_number() {
    ShapeTest::new(
        "enum Level { Low, Med, High }\nlet l = Level::High\nmatch l { Level::Low => 0.1, Level::Med => 0.5, Level::High => 1.0, }",
    )
    .expect_number(1.0);
}

// =============================================================================
// SECTION 31: Enum Variant Constructed in Different Scopes
// =============================================================================

/// Verifies enum constructed in if branch.
#[test]
fn test_enum_constructed_in_if_branch() {
    ShapeTest::new(
        "enum Choice { Yes, No }\nlet cond = true\nlet c = if cond { Choice::Yes } else { Choice::No }\nmatch c { Choice::Yes => 1, Choice::No => 0, }",
    )
    .expect_number(1.0);
}

/// Verifies enum constructed in else branch.
#[test]
fn test_enum_constructed_in_else_branch() {
    ShapeTest::new(
        "enum Choice { Yes, No }\nlet cond = false\nlet c = if cond { Choice::Yes } else { Choice::No }\nmatch c { Choice::Yes => 1, Choice::No => 0, }",
    )
    .expect_number(0.0);
}

// =============================================================================
// SECTION 32: Enum With Match and Variable Shadowing
// =============================================================================

/// Verifies match arm binding shadows outer variable.
#[test]
fn test_enum_match_does_not_leak_bindings() {
    ShapeTest::new(
        "enum Wrapper { Val(int), Empty }\nlet n = 999\nlet w = Wrapper::Val(42)\nlet result = match w { Wrapper::Val(n) => n, Wrapper::Empty => 0, }\nresult",
    )
    .expect_number(42.0);
}

// =============================================================================
// SECTION 34: Enum and String Interpolation
// =============================================================================

/// Verifies enum match result used in string concatenation.
#[test]
fn test_enum_match_result_in_interpolation() {
    ShapeTest::new(
        r#"enum Color { Red, Green, Blue }
let c = Color::Red
let name = match c { Color::Red => "red", Color::Green => "green", Color::Blue => "blue", }
"Color is: " + name"#,
    )
    .expect_string("Color is: red");
}

// =============================================================================
// SECTION 36: Enum Match with Arithmetic in Arms
// =============================================================================

/// Verifies arithmetic in match arms.
#[test]
fn test_enum_match_arm_arithmetic() {
    ShapeTest::new(
        "enum Scale { Hundred, Thousand, Million }\nlet s = Scale::Thousand\nlet base = 5\nmatch s { Scale::Hundred => base * 100, Scale::Thousand => base * 1000, Scale::Million => base * 1000000, }",
    )
    .expect_number(5000.0);
}

// =============================================================================
// SECTION 37: Enum Sequential Matches on Same Value
// =============================================================================

/// Verifies multiple sequential matches on same value.
#[test]
fn test_enum_multiple_matches_same_value() {
    ShapeTest::new(
        r#"enum Color { Red, Green, Blue }
let c = Color::Green
let name = match c { Color::Red => "R", Color::Green => "G", Color::Blue => "B", }
let code = match c { Color::Red => 1, Color::Green => 2, Color::Blue => 3, }
code"#,
    )
    .expect_number(2.0);
}

// =============================================================================
// SECTION 43: Enum as TypedObject (Internal Representation)
// =============================================================================

/// Verifies enum TypedObject representation via matching 8 variants.
#[test]
fn test_enum_is_typed_object_internally() {
    ShapeTest::new(
        "enum Token { A, B, C, D, E, F, G, H }\nlet t = Token::E\nmatch t { Token::A => 1, Token::B => 2, Token::C => 3, Token::D => 4, Token::E => 5, Token::F => 6, Token::G => 7, Token::H => 8, }",
    )
    .expect_number(5.0);
}

// =============================================================================
// SECTION 48: Enum Match with Wildcard After Specific Arms
// =============================================================================

/// Verifies wildcard after specific arms catches unmatched variant.
#[test]
fn test_match_specific_then_wildcard() {
    ShapeTest::new(
        "enum HTTP { Ok, NotFound, ServerError, Redirect, Forbidden }\nlet status = HTTP::Forbidden\nmatch status { HTTP::Ok => 200, HTTP::NotFound => 404, _ => -1, }",
    )
    .expect_number(-1.0);
}

/// Verifies specific arm hit before wildcard.
#[test]
fn test_match_specific_hit_before_wildcard() {
    ShapeTest::new(
        "enum HTTP { Ok, NotFound, ServerError, Redirect, Forbidden }\nlet status = HTTP::NotFound\nmatch status { HTTP::Ok => 200, HTTP::NotFound => 404, _ => -1, }",
    )
    .expect_number(404.0);
}

// =============================================================================
// SECTION 52: Enum Match on Freshly Constructed (no let)
// =============================================================================

/// Verifies match on inline unit variant construction.
#[test]
fn test_match_on_inline_construction_unit() {
    ShapeTest::new(
        "enum Parity { Even, Odd }\nmatch Parity::Odd { Parity::Even => 0, Parity::Odd => 1, }",
    )
    .expect_number(1.0);
}

/// Verifies match on inline tuple variant construction.
#[test]
fn test_match_on_inline_construction_tuple() {
    ShapeTest::new(
        "enum Tagged { Val(int), Empty }\nmatch Tagged::Val(55) { Tagged::Val(n) => n, Tagged::Empty => -1, }",
    )
    .expect_number(55.0);
}

// =============================================================================
// SECTION 55: Multiple Match Arms Returning Same Value
// =============================================================================

/// Verifies wildcard grouping for weekend.
#[test]
fn test_enum_wildcard_for_group() {
    ShapeTest::new(
        r#"enum Weekday { Mon, Tue, Wed, Thu, Fri, Sat, Sun }
let d = Weekday::Sat
match d { Weekday::Sat => "weekend", Weekday::Sun => "weekend", _ => "weekday", }"#,
    )
    .expect_string("weekend");
}

/// Verifies wildcard grouping for weekday.
#[test]
fn test_enum_wildcard_for_group_weekday() {
    ShapeTest::new(
        r#"enum Weekday { Mon, Tue, Wed, Thu, Fri, Sat, Sun }
let d = Weekday::Wed
match d { Weekday::Sat => "weekend", Weekday::Sun => "weekend", _ => "weekday", }"#,
    )
    .expect_string("weekday");
}

// =============================================================================
// SECTION 56: Enum Match Expression as Function Argument
// =============================================================================

/// Verifies match expression used as function argument.
#[test]
fn test_enum_match_as_fn_argument() {
    ShapeTest::new(
        "enum Mode { X, Y }\nfn double(n: int) -> int { n * 2 }\nfn test() -> int { let m = Mode::Y\ndouble(match m { Mode::X => 5, Mode::Y => 10, }) }\ntest()",
    )
    .expect_number(20.0);
}

// =============================================================================
// SECTION 64: Enum Match — Exhaustiveness Concerns
// =============================================================================

/// Verifies non-exhaustive match succeeds when a matching arm exists.
#[test]
fn test_enum_non_exhaustive_match_may_fail() {
    ShapeTest::new(
        "enum RGB { Red, Green, Blue }\nlet c = RGB::Red\nmatch c { RGB::Red => 1, RGB::Green => 2, }",
    )
    .expect_number(1.0);
}

// =============================================================================
// SECTION 66: Enum and Arithmetic Together
// =============================================================================

/// Verifies enum match result used as arithmetic operand.
#[test]
fn test_enum_match_result_as_operand() {
    ShapeTest::new(
        "enum Weight { Light, Heavy }\nlet w = Weight::Heavy\nlet base = 100\nlet multiplier = match w { Weight::Light => 1, Weight::Heavy => 10, }\nbase * multiplier",
    )
    .expect_number(1000.0);
}

// =============================================================================
// SECTION 68: Enum — Match on Enum Returned from Array Index
// =============================================================================

/// Verifies matching on enum from array index.
#[test]
fn test_enum_from_array_index_match() {
    ShapeTest::new(
        r#"enum Code { Ok, Err }
let codes = [Code::Ok, Code::Err, Code::Ok]
match codes[1] { Code::Ok => "ok", Code::Err => "err", }"#,
    )
    .expect_string("err");
}

// =============================================================================
// SECTION 73: Enum — Return Enum From Match Arm (Match Produces Enum)
// =============================================================================

/// Verifies producing enum from conditional and matching it.
#[test]
fn test_match_produces_enum_value() {
    ShapeTest::new(
        r#"enum Level { Low, Med, High }
let score = 75
let level = if score >= 90 { Level::High } else if score >= 50 { Level::Med } else { Level::Low }
match level { Level::Low => "F", Level::Med => "C", Level::High => "A", }"#,
    )
    .expect_string("C");
}

// =============================================================================
// SECTION 75: Additional edge cases — match-related
// =============================================================================

/// Verifies single variant match.
#[test]
fn test_enum_single_variant_match() {
    ShapeTest::new("enum Singleton { Only }\nmatch Singleton::Only { Singleton::Only => 42, }")
        .expect_number(42.0);
}

/// Verifies first arm always hits.
#[test]
fn test_enum_match_first_arm_always_hits() {
    ShapeTest::new(
        r#"enum AB { A, B }
let x = AB::A
match x { AB::A => "first", AB::B => "second", }"#,
    )
    .expect_string("first");
}

/// Verifies last arm hits.
#[test]
fn test_enum_match_last_arm_hits() {
    ShapeTest::new(
        r#"enum AB { A, B }
let x = AB::B
match x { AB::A => "first", AB::B => "second", }"#,
    )
    .expect_string("second");
}

/// Verifies interleaved matches on different enums.
#[test]
fn test_enum_multiple_matches_different_enums_interleaved() {
    ShapeTest::new(
        "enum X { A, B }\nenum Y { C, D }\nlet x = X::B\nlet y = Y::C\nlet xv = match x { X::A => 1, X::B => 2, }\nlet yv = match y { Y::C => 10, Y::D => 20, }\nxv + yv",
    )
    .expect_number(12.0);
}

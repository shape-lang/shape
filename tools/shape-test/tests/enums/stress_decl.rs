//! Stress tests for enum declaration, construction, equality, and variable binding.
//!
//! Migrated from shape-vm stress_17_enums.rs — Sections 1-8, 11, 22, 28, 30, 33, 40, 62-63, 70.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// SECTION 1: Basic Enum Declaration
// =============================================================================

/// Verifies enum with two variants can be declared.
#[test]
fn test_enum_two_variants() {
    ShapeTest::new("enum OnOff { On, Off }\n1").expect_number(1.0);
}

/// Verifies enum with three variants can be declared.
#[test]
fn test_enum_three_variants() {
    ShapeTest::new("enum Color { Red, Green, Blue }\n1").expect_number(1.0);
}

/// Verifies enum with five variants can be declared.
#[test]
fn test_enum_five_variants() {
    ShapeTest::new("enum Weekday { Mon, Tue, Wed, Thu, Fri }\n1").expect_number(1.0);
}

/// Verifies enum with a single variant can be declared.
#[test]
fn test_enum_single_variant() {
    ShapeTest::new("enum Unit { Only }\n1").expect_number(1.0);
}

/// Verifies enum with twelve variants can be declared.
#[test]
fn test_enum_many_variants() {
    ShapeTest::new("enum Month { Jan, Feb, Mar, Apr, May, Jun, Jul, Aug, Sep, Oct, Nov, Dec }\n1")
        .expect_number(1.0);
}

/// Verifies enum with trailing comma can be declared.
#[test]
fn test_enum_with_trailing_comma() {
    ShapeTest::new("enum Dir { Up, Down, Left, Right, }\n1").expect_number(1.0);
}

// =============================================================================
// SECTION 2: Enum Variant Access (Construction)
// =============================================================================

/// Verifies constructing and matching the first enum variant.
#[test]
fn test_enum_construct_first_variant() {
    ShapeTest::new(
        "enum Color { Red, Green, Blue }\nlet c = Color::Red\nmatch c { Color::Red => 10, Color::Green => 20, Color::Blue => 30, }",
    )
    .expect_number(10.0);
}

/// Verifies constructing and matching the second enum variant.
#[test]
fn test_enum_construct_second_variant() {
    ShapeTest::new(
        "enum Color { Red, Green, Blue }\nlet c = Color::Green\nmatch c { Color::Red => 10, Color::Green => 20, Color::Blue => 30, }",
    )
    .expect_number(20.0);
}

/// Verifies constructing and matching the third enum variant.
#[test]
fn test_enum_construct_third_variant() {
    ShapeTest::new(
        "enum Color { Red, Green, Blue }\nlet c = Color::Blue\nmatch c { Color::Red => 10, Color::Green => 20, Color::Blue => 30, }",
    )
    .expect_number(30.0);
}

/// Verifies constructing and matching the last of many variants.
#[test]
fn test_enum_construct_last_of_many() {
    ShapeTest::new(
        "enum Dir { North, South, East, West }\nlet d = Dir::West\nmatch d { Dir::North => 1, Dir::South => 2, Dir::East => 3, Dir::West => 4, }",
    )
    .expect_number(4.0);
}

// =============================================================================
// SECTION 3: Enum with Explicit Values
// =============================================================================

/// Verifies enum with explicit int values parses correctly.
#[test]
fn test_enum_explicit_int_values_parses() {
    ShapeTest::new("enum Status { Pending = 0, Active = 1, Done = 2 }\n1").expect_number(1.0);
}

/// Verifies enum with explicit string values parses correctly.
#[test]
fn test_enum_explicit_string_values_parses() {
    ShapeTest::new(
        r#"enum Direction { Up = "up", Down = "down", Left = "left", Right = "right" }
1"#,
    )
    .expect_number(1.0);
}

/// Verifies match still works on enum with explicit values.
#[test]
fn test_enum_explicit_values_match_still_works() {
    ShapeTest::new(
        r#"enum Status { Pending = 0, Active = 1, Done = 2 }
let s = Status::Active
match s { Status::Pending => "pending", Status::Active => "active", Status::Done => "done", }"#,
    )
    .expect_string("active");
}

// =============================================================================
// SECTION 5: Enum Equality and Inequality
// =============================================================================

/// Verifies two same enum variants are equal.
#[test]
fn test_enum_eq_same_variant() {
    ShapeTest::new("enum Color { Red, Green, Blue }\nColor::Red == Color::Red").expect_bool(true);
}

/// Verifies two different enum variants are not equal.
#[test]
fn test_enum_eq_different_variant() {
    ShapeTest::new("enum Color { Red, Green, Blue }\nColor::Red == Color::Blue").expect_bool(false);
}

/// Verifies != returns false for same variants.
#[test]
fn test_enum_neq_same_variant() {
    ShapeTest::new("enum Color { Red, Green, Blue }\nColor::Green != Color::Green")
        .expect_bool(false);
}

/// Verifies != returns true for different variants.
#[test]
fn test_enum_neq_different_variant() {
    ShapeTest::new("enum Color { Red, Green, Blue }\nColor::Red != Color::Blue").expect_bool(true);
}

/// Verifies enum equality through variables.
#[test]
fn test_enum_eq_via_variables() {
    ShapeTest::new("enum Dir { Up, Down }\nlet a = Dir::Up\nlet b = Dir::Up\na == b")
        .expect_bool(true);
}

/// Verifies enum inequality through variables.
#[test]
fn test_enum_neq_via_variables() {
    ShapeTest::new("enum Dir { Up, Down }\nlet a = Dir::Up\nlet b = Dir::Down\na != b")
        .expect_bool(true);
}

// =============================================================================
// SECTION 6: Enum as Variable
// =============================================================================

/// Verifies enum let binding and match.
#[test]
fn test_enum_let_binding() {
    ShapeTest::new(
        "enum Light { Red, Yellow, Green }\nlet l = Light::Yellow\nmatch l { Light::Red => 1, Light::Yellow => 2, Light::Green => 3, }",
    )
    .expect_number(2.0);
}

/// Verifies enum var reassignment.
#[test]
fn test_enum_reassign_var() {
    ShapeTest::new(
        "enum Light { Red, Yellow, Green }\nlet mut l = Light::Red\nl = Light::Green\nmatch l { Light::Red => 1, Light::Yellow => 2, Light::Green => 3, }",
    )
    .expect_number(3.0);
}

/// Verifies multiple enum variable bindings.
#[test]
fn test_enum_multiple_bindings() {
    ShapeTest::new(
        "enum Bit { Zero, One }\nlet a = Bit::Zero\nlet b = Bit::One\nlet c = Bit::Zero\nmatch b { Bit::Zero => 0, Bit::One => 1, }",
    )
    .expect_number(1.0);
}

// =============================================================================
// SECTION 7: Enum as Function Parameter
// =============================================================================

/// Verifies passing enum as function parameter.
#[test]
fn test_enum_fn_param_first_variant() {
    ShapeTest::new(
        "enum Shape { Circle, Square, Triangle }\nfn sides(s: Shape) -> int { match s { Shape::Circle => 0, Shape::Square => 4, Shape::Triangle => 3, } }\nfn test() -> int { sides(Shape::Triangle) }\ntest()",
    )
    .expect_number(3.0);
}

/// Verifies enum params across all variants.
#[test]
fn test_enum_fn_param_all_variants() {
    ShapeTest::new(
        "enum Color { R, G, B }\nfn to_int(c: Color) -> int { match c { Color::R => 1, Color::G => 2, Color::B => 3, } }\nfn test() -> int { to_int(Color::R) + to_int(Color::G) + to_int(Color::B) }\ntest()",
    )
    .expect_number(6.0);
}

// =============================================================================
// SECTION 8: Enum as Return Value
// =============================================================================

/// Verifies returning enum from function.
#[test]
fn test_enum_fn_return() {
    ShapeTest::new(
        "enum Dir { Left, Right }\nfn choose() -> Dir { Dir::Left }\nfn test() -> int { let d = choose()\nmatch d { Dir::Left => 10, Dir::Right => 20, } }\ntest()",
    )
    .expect_number(10.0);
}

/// Verifies conditional enum return from function.
#[test]
fn test_enum_fn_return_conditional() {
    ShapeTest::new(
        "enum Dir { Left, Right }\nfn choose(go_left: bool) -> Dir { if go_left { Dir::Left } else { Dir::Right } }\nfn test() -> int { let d = choose(false)\nmatch d { Dir::Left => 10, Dir::Right => 20, } }\ntest()",
    )
    .expect_number(20.0);
}

// =============================================================================
// SECTION 11: Multiple Enum Definitions
// =============================================================================

/// Verifies two enums defined and used together.
#[test]
fn test_two_enums_defined_together() {
    ShapeTest::new(
        "enum Color { Red, Green, Blue }\nenum Size { Small, Medium, Large }\nlet c = Color::Green\nlet s = Size::Large\nlet color_val = match c { Color::Red => 1, Color::Green => 2, Color::Blue => 3, }\nlet size_val = match s { Size::Small => 10, Size::Medium => 20, Size::Large => 30, }\ncolor_val + size_val",
    )
    .expect_number(32.0);
}

/// Verifies three independent enums.
#[test]
fn test_three_enums_independent() {
    ShapeTest::new(
        "enum A { X, Y }\nenum B { P, Q }\nenum C { M, N }\nlet a = match A::X { A::X => 1, A::Y => 2, }\nlet b = match B::Q { B::P => 10, B::Q => 20, }\nlet c = match C::M { C::M => 100, C::N => 200, }\na + b + c",
    )
    .expect_number(121.0);
}

// =============================================================================
// SECTION 22: Enum Equality with Payload (TypedObject Equality)
// =============================================================================

/// Verifies equality of same unit variants.
#[test]
fn test_enum_eq_unit_variants_same() {
    ShapeTest::new("enum State { Open, Closed }\nlet a = State::Open\nlet b = State::Open\na == b")
        .expect_bool(true);
}

/// Verifies inequality of different unit variants.
#[test]
fn test_enum_eq_unit_variants_different() {
    ShapeTest::new(
        "enum State { Open, Closed }\nlet a = State::Open\nlet b = State::Closed\na == b",
    )
    .expect_bool(false);
}

// =============================================================================
// SECTION 28: Enum Variant Discrimination Correctness
// =============================================================================

/// Verifies variant discrimination for the first variant.
#[test]
fn test_variant_id_discrimination_first() {
    ShapeTest::new(
        "enum ABC { A, B, C }\nlet x = ABC::A\nmatch x { ABC::A => 100, ABC::B => 200, ABC::C => 300, }",
    )
    .expect_number(100.0);
}

/// Verifies variant discrimination for the middle variant.
#[test]
fn test_variant_id_discrimination_middle() {
    ShapeTest::new(
        "enum ABC { A, B, C }\nlet x = ABC::B\nmatch x { ABC::A => 100, ABC::B => 200, ABC::C => 300, }",
    )
    .expect_number(200.0);
}

/// Verifies variant discrimination for the last variant.
#[test]
fn test_variant_id_discrimination_last() {
    ShapeTest::new(
        "enum ABC { A, B, C }\nlet x = ABC::C\nmatch x { ABC::A => 100, ABC::B => 200, ABC::C => 300, }",
    )
    .expect_number(300.0);
}

// =============================================================================
// SECTION 30: Enum — Large Variant Count
// =============================================================================

/// Verifies enum with eight variants all matched.
#[test]
fn test_enum_eight_variants_all_matched() {
    ShapeTest::new(
        "enum Day { Mon, Tue, Wed, Thu, Fri, Sat, Sun, Holiday }\nlet d = Day::Sat\nmatch d { Day::Mon => 1, Day::Tue => 2, Day::Wed => 3, Day::Thu => 4, Day::Fri => 5, Day::Sat => 6, Day::Sun => 7, Day::Holiday => 8, }",
    )
    .expect_number(6.0);
}

/// Verifies enum with twelve variants, matching the last.
#[test]
fn test_enum_twelve_variants_last() {
    ShapeTest::new(
        "enum Month { Jan, Feb, Mar, Apr, May, Jun, Jul, Aug, Sep, Oct, Nov, Dec }\nlet m = Month::Dec\nmatch m { Month::Jan => 1, Month::Feb => 2, Month::Mar => 3, Month::Apr => 4, Month::May => 5, Month::Jun => 6, Month::Jul => 7, Month::Aug => 8, Month::Sep => 9, Month::Oct => 10, Month::Nov => 11, Month::Dec => 12, }",
    )
    .expect_number(12.0);
}

// =============================================================================
// SECTION 33: Pub Enum
// =============================================================================

/// Verifies pub enum declaration and match.
#[test]
fn test_pub_enum_declaration() {
    ShapeTest::new(
        r#"pub enum Visibility { Public, Private }
match Visibility::Public { Visibility::Public => "pub", Visibility::Private => "priv", }"#,
    )
    .expect_string("pub");
}

// =============================================================================
// SECTION 40: Enum Semicolon Separator
// =============================================================================

/// Verifies enum with semicolon separator between variants.
#[test]
fn test_enum_semicolon_separator() {
    ShapeTest::new("enum Sep { A; B; C }\nmatch Sep::B { Sep::A => 1, Sep::B => 2, Sep::C => 3, }")
        .expect_number(2.0);
}

// =============================================================================
// SECTION 62: Enum Equality Stress — All Combinations
// =============================================================================

/// Verifies A == A is true.
#[test]
fn test_enum_eq_three_variants_aa() {
    ShapeTest::new("enum T { A, B, C }\nT::A == T::A").expect_bool(true);
}

/// Verifies A == B is false.
#[test]
fn test_enum_eq_three_variants_ab() {
    ShapeTest::new("enum T { A, B, C }\nT::A == T::B").expect_bool(false);
}

/// Verifies A == C is false.
#[test]
fn test_enum_eq_three_variants_ac() {
    ShapeTest::new("enum T { A, B, C }\nT::A == T::C").expect_bool(false);
}

/// Verifies B == B is true.
#[test]
fn test_enum_eq_three_variants_bb() {
    ShapeTest::new("enum T { A, B, C }\nT::B == T::B").expect_bool(true);
}

/// Verifies B == C is false.
#[test]
fn test_enum_eq_three_variants_bc() {
    ShapeTest::new("enum T { A, B, C }\nT::B == T::C").expect_bool(false);
}

/// Verifies C == C is true.
#[test]
fn test_enum_eq_three_variants_cc() {
    ShapeTest::new("enum T { A, B, C }\nT::C == T::C").expect_bool(true);
}

// =============================================================================
// SECTION 63: Enum with var Reassignment across Variants
// =============================================================================

/// Verifies var reassignment across variants.
#[test]
fn test_enum_var_reassignment_across_variants() {
    ShapeTest::new(
        "enum Light { Red, Yellow, Green }\nlet mut l = Light::Red\nlet v1 = match l { Light::Red => 1, Light::Yellow => 2, Light::Green => 3, }\nl = Light::Green\nlet v2 = match l { Light::Red => 1, Light::Yellow => 2, Light::Green => 3, }\nv1 * 10 + v2",
    )
    .expect_number(13.0);
}

// =============================================================================
// SECTION 70: Enum — Many Unit Variants Equality Check
// =============================================================================

/// Verifies equality with 10 variants (same).
#[test]
fn test_enum_10_variant_eq() {
    ShapeTest::new("enum Digit { D0, D1, D2, D3, D4, D5, D6, D7, D8, D9 }\nDigit::D7 == Digit::D7")
        .expect_bool(true);
}

/// Verifies inequality with 10 variants (different).
#[test]
fn test_enum_10_variant_neq() {
    ShapeTest::new("enum Digit { D0, D1, D2, D3, D4, D5, D6, D7, D8, D9 }\nDigit::D3 == Digit::D8")
        .expect_bool(false);
}

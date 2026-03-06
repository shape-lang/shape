//! Stress tests for advanced enum features: compile-time errors, complex logic,
//! state machines, dispatch tables, conditionals, and deeply nested patterns.
//!
//! Migrated from shape-vm stress_17_enums.rs — Sections 21, 23, 26, 35, 39, 41-43, 45-48, 50, 53-54, 56, 58-61, 64, 66-68, 73, 75-partial.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// SECTION 21: Compile-Time Errors
// =============================================================================

/// Verifies unknown enum type fails.
#[test]
fn test_unknown_enum_type_fails() {
    ShapeTest::new("let x = UnknownEnum::Variant").expect_run_err();
}

/// Verifies unknown variant fails.
#[test]
fn test_unknown_variant_fails() {
    ShapeTest::new("enum Color { Red, Green, Blue }\nlet x = Color::Purple").expect_run_err();
}

/// Verifies match on unknown variant fails.
#[test]
fn test_enum_match_unknown_variant_fails() {
    ShapeTest::new(
        "enum Color { Red, Green, Blue }\nlet c = Color::Red\nmatch c { Color::Red => 1, Color::Purple => 2, }",
    )
    .expect_run_err();
}

// =============================================================================
// SECTION 23: Enum in Function with Complex Logic
// =============================================================================

/// Verifies enum function with accumulator pattern.
#[test]
fn test_enum_function_with_accumulator() {
    ShapeTest::new(
        "enum Op { Add(int), Sub(int), Mul(int) }\nfn apply(val: int, op: Op) -> int { match op { Op::Add(n) => val + n, Op::Sub(n) => val - n, Op::Mul(n) => val * n, } }\nfn test() -> int { var result = 10\nresult = apply(result, Op::Add(5))\nresult = apply(result, Op::Mul(2))\nresult = apply(result, Op::Sub(3))\nresult }\ntest()",
    )
    .expect_number(27.0);
}

/// Verifies recursive function with enum.
#[test]
fn test_enum_recursive_function_pattern() {
    ShapeTest::new(
        "enum CountDown { Go(int), Stop }\nfn step(c: CountDown) -> int { match c { CountDown::Go(n) => { if n <= 0 { 0 } else { n + step(CountDown::Go(n - 1)) } }, CountDown::Stop => 0, } }\nfn test() -> int { step(CountDown::Go(5)) }\ntest()",
    )
    .expect_number(15.0);
}

// =============================================================================
// SECTION 26: Multiple Enums in Same Function
// =============================================================================

/// Verifies two enums in one function.
#[test]
fn test_two_enums_in_one_function() {
    ShapeTest::new(
        "enum Suit { Hearts, Diamonds, Clubs, Spades }\nenum Rank { Ace, King, Queen, Jack }\nfn card_value(s: Suit, r: Rank) -> int { let suit_val = match s { Suit::Hearts => 4, Suit::Diamonds => 3, Suit::Clubs => 2, Suit::Spades => 1, }\nlet rank_val = match r { Rank::Ace => 14, Rank::King => 13, Rank::Queen => 12, Rank::Jack => 11, }\nsuit_val * 100 + rank_val }\nfn test() -> int { card_value(Suit::Spades, Rank::Ace) }\ntest()",
    )
    .expect_number(114.0);
}

// =============================================================================
// SECTION 35: Enum Variant Comparison After Roundtrip
// =============================================================================

/// Verifies equality after function roundtrip.
#[test]
fn test_enum_eq_after_fn_roundtrip() {
    ShapeTest::new(
        "enum Token { Ident, Number, Str }\nfn make() -> Token { Token::Number }\nfn test() -> bool { let t = make()\nlet expected = Token::Number\nt == expected }\ntest()",
    )
    .expect_bool(true);
}

/// Verifies inequality after function roundtrip.
#[test]
fn test_enum_neq_after_fn_roundtrip() {
    ShapeTest::new(
        "enum Token { Ident, Number, Str }\nfn make() -> Token { Token::Ident }\nfn test() -> bool { let t = make()\nlet expected = Token::Number\nt != expected }\ntest()",
    )
    .expect_bool(true);
}

// =============================================================================
// SECTION 39: Enum Constructed Inside Match Arm
// =============================================================================

/// Verifies constructing enum in match arm.
#[test]
fn test_enum_construct_in_match_arm() {
    ShapeTest::new(
        "enum State { Ready, Running, Done }\nlet current = State::Ready\nlet next = match current { State::Ready => State::Running, State::Running => State::Done, State::Done => State::Done, }\nmatch next { State::Ready => 0, State::Running => 1, State::Done => 2, }",
    )
    .expect_number(1.0);
}

/// Verifies state machine two steps.
#[test]
fn test_enum_state_machine_two_steps() {
    ShapeTest::new(
        "enum State { Ready, Running, Done }\nfn advance(s: State) -> State { match s { State::Ready => State::Running, State::Running => State::Done, State::Done => State::Done, } }\nfn test() -> int { let s0 = State::Ready\nlet s1 = advance(s0)\nlet s2 = advance(s1)\nmatch s2 { State::Ready => 0, State::Running => 1, State::Done => 2, } }\ntest()",
    )
    .expect_number(2.0);
}

// =============================================================================
// SECTION 41: Enum Used as Array Filter Discriminator
// =============================================================================

/// Verifies enum filter via match in loop.
#[test]
fn test_enum_filter_via_match_in_loop() {
    ShapeTest::new(
        "enum Kind { Good, Bad }\nfn test() -> int { let items = [Kind::Good, Kind::Bad, Kind::Good, Kind::Good, Kind::Bad]\nvar count = 0\nfor item in items { let is_good = match item { Kind::Good => true, Kind::Bad => false, }\nif is_good { count = count + 1 } }\ncount }\ntest()",
    )
    .expect_number(3.0);
}

// =============================================================================
// SECTION 42: Enum Payload Extraction in Loop
// =============================================================================

/// Verifies enum payload sum in loop.
#[test]
fn test_enum_payload_sum_in_loop() {
    ShapeTest::new(
        "enum Item { Value(int), Skip }\nfn test() -> int { let items = [Item::Value(10), Item::Skip, Item::Value(20), Item::Value(5)]\nvar total = 0\nfor item in items { total = total + match item { Item::Value(n) => n, Item::Skip => 0, } }\ntotal }\ntest()",
    )
    .expect_number(35.0);
}

// =============================================================================
// SECTION 43: Enum as TypedObject (Internal Representation)
// =============================================================================

/// Verifies 8-variant enum matching.
#[test]
fn test_enum_is_typed_object_internally() {
    ShapeTest::new(
        "enum Token { A, B, C, D, E, F, G, H }\nlet t = Token::E\nmatch t { Token::A => 1, Token::B => 2, Token::C => 3, Token::D => 4, Token::E => 5, Token::F => 6, Token::G => 7, Token::H => 8, }",
    )
    .expect_number(5.0);
}

// =============================================================================
// SECTION 45: Enum Equality Between Different Enums
// =============================================================================

/// Verifies different enum types comparison does not crash.
#[test]
fn test_enum_different_types_not_equal() {
    ShapeTest::new(
        "enum A { X }\nenum B { X }\nlet a = A::X\nlet b = B::X\na == b",
    )
    .expect_bool(false);
}

// =============================================================================
// SECTION 46: Enum With Generic-Like Patterns (Option/Result Builtin)
// =============================================================================

/// Verifies builtin Option Some.
#[test]
fn test_builtin_option_some() {
    ShapeTest::new("let x = Some(42)\nmatch x { Some(n) => n, None => 0, }").expect_number(42.0);
}

/// Verifies builtin Option None.
#[test]
fn test_builtin_option_none() {
    ShapeTest::new("let x = None\nmatch x { Some(n) => 1, None => 0, }").expect_number(0.0);
}

/// Verifies builtin Result Ok.
#[test]
fn test_builtin_result_ok() {
    ShapeTest::new("let x = Ok(10)\nmatch x { Ok(n) => n, Err(e) => 0, }").expect_number(10.0);
}

/// Verifies builtin Result Err.
#[test]
fn test_builtin_result_err() {
    ShapeTest::new(
        "let x = Err(\"oops\")\nmatch x { Ok(n) => \"ok\", Err(e) => e, }",
    )
    .expect_string("oops");
}

// =============================================================================
// SECTION 47: Enum Variant in Complex Expression
// =============================================================================

/// Verifies enum variant in complex GPA expression.
#[test]
fn test_enum_variant_in_complex_expression() {
    ShapeTest::new(
        "enum Grade { A, B, C, D, F }\nfn gpa(g: Grade) -> number { match g { Grade::A => 4.0, Grade::B => 3.0, Grade::C => 2.0, Grade::D => 1.0, Grade::F => 0.0, } }\nlet grades = [Grade::A, Grade::B, Grade::A, Grade::C]\nvar total = 0.0\nfor g in grades { total = total + gpa(g) }\ntotal",
    )
    .expect_number(13.0);
}

// =============================================================================
// SECTION 50: Stress — Enum Dispatch Table Pattern
// =============================================================================

/// Verifies enum dispatch table pattern.
#[test]
fn test_enum_dispatch_table_pattern() {
    ShapeTest::new(
        "enum BinOp { Add, Sub, Mul, Div }\nfn eval_op(op: BinOp, a: int, b: int) -> int { match op { BinOp::Add => a + b, BinOp::Sub => a - b, BinOp::Mul => a * b, BinOp::Div => a / b, } }\nfn test() -> int { let r1 = eval_op(BinOp::Add, 10, 3)\nlet r2 = eval_op(BinOp::Sub, 10, 3)\nlet r3 = eval_op(BinOp::Mul, 10, 3)\nlet r4 = eval_op(BinOp::Div, 10, 3)\nr1 + r2 + r3 + r4 }\ntest()",
    )
    .expect_number(53.0);
}

// =============================================================================
// SECTION 53: Enum Variant From Conditional
// =============================================================================

/// Verifies enum from conditional — true path.
#[test]
fn test_enum_from_conditional_true() {
    ShapeTest::new(
        "enum Gate { Open, Closed }\nlet flag = true\nlet g = if flag { Gate::Open } else { Gate::Closed }\nmatch g { Gate::Open => \"open\", Gate::Closed => \"closed\", }",
    )
    .expect_string("open");
}

/// Verifies enum from conditional — false path.
#[test]
fn test_enum_from_conditional_false() {
    ShapeTest::new(
        "enum Gate { Open, Closed }\nlet flag = false\nlet g = if flag { Gate::Open } else { Gate::Closed }\nmatch g { Gate::Open => \"open\", Gate::Closed => \"closed\", }",
    )
    .expect_string("closed");
}

// =============================================================================
// SECTION 54: Enum In Deeply Nested Functions
// =============================================================================

/// Verifies three-level function chain with enum.
#[test]
fn test_enum_three_level_function_chain() {
    ShapeTest::new(
        "enum Signal { Buy, Sell, Hold }\nfn classify(val: int) -> Signal { if val > 0 { Signal::Buy } else if val < 0 { Signal::Sell } else { Signal::Hold } }\nfn act(s: Signal) -> string { match s { Signal::Buy => \"buying\", Signal::Sell => \"selling\", Signal::Hold => \"holding\", } }\nfn pipeline(val: int) -> string { act(classify(val)) }\nfn test() -> string { pipeline(-5) }\ntest()",
    )
    .expect_string("selling");
}

// =============================================================================
// SECTION 56: Enum Match Expression as Function Argument
// =============================================================================

/// Verifies enum match as function argument.
#[test]
fn test_enum_match_as_fn_argument() {
    ShapeTest::new(
        "enum Mode { X, Y }\nfn double(n: int) -> int { n * 2 }\nfn test() -> int { let m = Mode::Y\ndouble(match m { Mode::X => 5, Mode::Y => 10, }) }\ntest()",
    )
    .expect_number(20.0);
}

// =============================================================================
// SECTION 58: Enum Used for Error-Like Patterns
// =============================================================================

/// Verifies enum error pattern — success.
#[test]
fn test_enum_error_pattern_success() {
    ShapeTest::new(
        "enum OpResult { Success(int), Failure(string) }\nfn safe_divide(a: int, b: int) -> OpResult { if b == 0 { OpResult::Failure(\"division by zero\") } else { OpResult::Success(a / b) } }\nfn test() -> int { match safe_divide(10, 2) { OpResult::Success(v) => v, OpResult::Failure(msg) => -1, } }\ntest()",
    )
    .expect_number(5.0);
}

/// Verifies enum error pattern — failure.
#[test]
fn test_enum_error_pattern_failure() {
    ShapeTest::new(
        "enum OpResult { Success(int), Failure(string) }\nfn safe_divide(a: int, b: int) -> OpResult { if b == 0 { OpResult::Failure(\"division by zero\") } else { OpResult::Success(a / b) } }\nfn test() -> string { match safe_divide(10, 0) { OpResult::Success(v) => \"ok\", OpResult::Failure(msg) => msg, } }\ntest()",
    )
    .expect_string("division by zero");
}

// =============================================================================
// SECTION 59: Enum — Variant With Same Name as Builtin
// =============================================================================

/// Verifies custom Maybe enum — Just variant.
#[test]
fn test_enum_variant_named_none_custom() {
    ShapeTest::new(
        "enum Maybe { Just(int), Nothing }\nlet m = Maybe::Just(7)\nmatch m { Maybe::Just(n) => n, Maybe::Nothing => 0, }",
    )
    .expect_number(7.0);
}

/// Verifies custom Maybe enum — Nothing variant.
#[test]
fn test_enum_variant_named_none_custom_match_none() {
    ShapeTest::new(
        "enum Maybe { Just(int), Nothing }\nlet m = Maybe::Nothing\nmatch m { Maybe::Just(n) => n, Maybe::Nothing => -1, }",
    )
    .expect_number(-1.0);
}

// =============================================================================
// SECTION 60: Enum — Chain of Matches
// =============================================================================

/// Verifies chain of phase transforms.
#[test]
fn test_enum_chain_of_transforms() {
    ShapeTest::new(
        "enum Phase { Init, Process, Complete }\nfn next_phase(p: Phase) -> Phase { match p { Phase::Init => Phase::Process, Phase::Process => Phase::Complete, Phase::Complete => Phase::Complete, } }\nfn phase_num(p: Phase) -> int { match p { Phase::Init => 0, Phase::Process => 1, Phase::Complete => 2, } }\nfn test() -> int { let p0 = Phase::Init\nlet p1 = next_phase(p0)\nlet p2 = next_phase(p1)\nlet p3 = next_phase(p2)\nphase_num(p0) + phase_num(p1) + phase_num(p2) + phase_num(p3) }\ntest()",
    )
    .expect_number(5.0);
}

// =============================================================================
// SECTION 61: Negative — Parse Failures
// =============================================================================

/// Verifies enum with no variants fails.
#[test]
fn test_enum_no_variants_fails_parse() {
    ShapeTest::new("enum Empty { }").expect_run_err();
}

/// Verifies enum missing braces fails.
#[test]
fn test_enum_missing_braces_fails() {
    ShapeTest::new("enum Broken Red, Green").expect_run_err();
}

// =============================================================================
// SECTION 64: Enum Match — Exhaustiveness Concerns (documented behavior)
// =============================================================================

// Non-exhaustive match behavior is documented but not tested here since
// behavior may vary. See stress_17 section 64 for details.

// =============================================================================
// SECTION 66: Enum and Arithmetic Together
// =============================================================================

/// Verifies match result as operand.
#[test]
fn test_enum_match_result_as_operand() {
    ShapeTest::new(
        "enum Weight { Light, Heavy }\nlet w = Weight::Heavy\nlet base = 100\nlet multiplier = match w { Weight::Light => 1, Weight::Heavy => 10, }\nbase * multiplier",
    )
    .expect_number(1000.0);
}

// =============================================================================
// SECTION 67: Enum — Idempotent Advance (State Machine)
// =============================================================================

/// Verifies idempotent state machine.
#[test]
fn test_enum_idempotent_state() {
    ShapeTest::new(
        "enum Lock { Locked, Unlocked }\nfn unlock(l: Lock) -> Lock { match l { Lock::Locked => Lock::Unlocked, Lock::Unlocked => Lock::Unlocked, } }\nfn test() -> int { let l = Lock::Locked\nlet l2 = unlock(l)\nlet l3 = unlock(l2)\nlet r2 = match l2 { Lock::Locked => 0, Lock::Unlocked => 1, }\nlet r3 = match l3 { Lock::Locked => 0, Lock::Unlocked => 1, }\nr2 + r3 }\ntest()",
    )
    .expect_number(2.0);
}

// =============================================================================
// SECTION 68: Enum — Match on Enum from Array Index
// =============================================================================

/// Verifies match on enum returned from array index.
#[test]
fn test_enum_from_array_index_match() {
    ShapeTest::new(
        "enum Code { Ok, Err }\nlet codes = [Code::Ok, Code::Err, Code::Ok]\nmatch codes[1] { Code::Ok => \"ok\", Code::Err => \"err\", }",
    )
    .expect_string("err");
}

// =============================================================================
// SECTION 73: Enum — Return Enum From Match Arm
// =============================================================================

/// Verifies match produces enum value.
#[test]
fn test_match_produces_enum_value() {
    ShapeTest::new(
        "enum Level { Low, Med, High }\nlet score = 75\nlet level = if score >= 90 { Level::High } else if score >= 50 { Level::Med } else { Level::Low }\nmatch level { Level::Low => \"F\", Level::Med => \"C\", Level::High => \"A\", }",
    )
    .expect_string("C");
}

// =============================================================================
// SECTION 75: Additional Edge Cases (non-payload)
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
        "enum AB { A, B }\nlet x = AB::A\nmatch x { AB::A => \"first\", AB::B => \"second\", }",
    )
    .expect_string("first");
}

/// Verifies last arm hits.
#[test]
fn test_enum_match_last_arm_hits() {
    ShapeTest::new(
        "enum AB { A, B }\nlet x = AB::B\nmatch x { AB::A => \"first\", AB::B => \"second\", }",
    )
    .expect_string("second");
}

/// Verifies multiple matches with different enums interleaved.
#[test]
fn test_enum_multiple_matches_different_enums_interleaved() {
    ShapeTest::new(
        "enum X { A, B }\nenum Y { C, D }\nlet x = X::B\nlet y = Y::C\nlet xv = match x { X::A => 1, X::B => 2, }\nlet yv = match y { Y::C => 10, Y::D => 20, }\nxv + yv",
    )
    .expect_number(12.0);
}

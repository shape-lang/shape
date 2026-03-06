//! Test framework types for Shape AST

use serde::{Deserialize, Serialize};

use super::expressions::Expr;
use super::statements::Statement;
use super::time::Timeframe;

/// Test definition containing test cases and fixtures
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestDef {
    /// Test suite name
    pub name: String,
    /// Optional setup code run before each test
    pub setup: Option<Vec<Statement>>,
    /// Optional teardown code run after each test
    pub teardown: Option<Vec<Statement>>,
    /// Test cases in this suite
    pub test_cases: Vec<TestCase>,
}

/// Individual test case
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCase {
    /// Test description
    pub description: String,
    /// Optional tags for categorization
    pub tags: Vec<String>,
    /// Test body containing assertions
    pub body: Vec<TestStatement>,
}

/// Statements that can appear in tests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TestStatement {
    /// Regular statement
    Statement(Statement),
    /// Assertion
    Assert(AssertStatement),
    /// Expect-style assertion
    Expect(ExpectStatement),
    /// Should-style assertion
    Should(ShouldStatement),
    /// Test fixture
    Fixture(TestFixture),
}

/// Assert statement for simple assertions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssertStatement {
    /// Condition to assert
    pub condition: Expr,
    /// Optional failure message
    pub message: Option<String>,
}

/// Expect-style assertion (BDD style)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectStatement {
    /// Expression to test
    pub actual: Expr,
    /// Matcher to apply
    pub matcher: ExpectationMatcher,
}

/// Expectation matchers for expect statements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExpectationMatcher {
    /// Expect value to be exactly equal
    ToBe(Expr),
    /// Expect value to equal (with deep equality)
    ToEqual(Expr),
    /// Expect numeric value to be close to target
    ToBeCloseTo {
        expected: Expr,
        tolerance: Option<f64>,
    },
    /// Expect value to be greater than
    ToBeGreaterThan(Expr),
    /// Expect value to be less than
    ToBeLessThan(Expr),
    /// Expect collection to contain element
    ToContain(Expr),
    /// Expect value to be truthy
    ToBeTruthy,
    /// Expect value to be falsy
    ToBeFalsy,
    /// Expect function to throw error
    ToThrow(Option<String>),
    /// Expect rows to match pattern
    ToMatchPattern {
        pattern: String,
        options: TestMatchOptions,
    },
}

/// Options for pattern matching in tests
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TestMatchOptions {
    /// Fuzzy matching tolerance
    pub fuzzy: Option<f64>,
    /// Timeframe for pattern matching
    pub timeframe: Option<Timeframe>,
    /// Symbol to test against
    pub symbol: Option<String>,
}

/// Should-style assertion (natural language)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShouldStatement {
    /// Expression being tested
    pub subject: Expr,
    /// Matcher to apply
    pub matcher: ShouldMatcher,
}

/// Matchers for should statements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ShouldMatcher {
    /// Should be equal to value
    Be(Expr),
    /// Should equal value
    Equal(Expr),
    /// Should contain element
    Contain(Expr),
    /// Should match pattern
    Match(String),
    /// Should be close to value
    BeCloseTo {
        expected: Expr,
        tolerance: Option<f64>,
    },
}

/// Test fixtures for setting up test data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TestFixture {
    /// Provide test data (generic data rows)
    WithData { data: Expr, body: Vec<Statement> },
    /// Mock a function or indicator
    WithMock {
        target: String,
        mock_value: Option<Expr>,
        body: Vec<Statement>,
    },
}

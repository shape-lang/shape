//! Test framework analysis
//!
//! This module handles analysis of test definitions and test statements.

use shape_ast::ast::{
    ExpectationMatcher, ShouldMatcher, Spanned, TestDef, TestFixture, TestStatement,
};
use shape_ast::error::Result;

use super::types;

/// Implementation of test analysis methods for SemanticAnalyzer
impl super::SemanticAnalyzer {
    /// Analyze a test definition
    pub(super) fn analyze_test(&mut self, test: &TestDef) -> Result<()> {
        // Create a new scope for the test
        self.symbol_table.push_scope();

        // Analyze setup code if present
        if let Some(setup) = &test.setup {
            for stmt in setup {
                self.analyze_statement(stmt)?;
            }
        }

        // Analyze each test case
        for test_case in &test.test_cases {
            // Create a scope for each test case
            self.symbol_table.push_scope();

            for stmt in &test_case.body {
                self.analyze_test_statement(stmt)?;
            }

            self.symbol_table.pop_scope();
        }

        // Analyze teardown code if present
        if let Some(teardown) = &test.teardown {
            for stmt in teardown {
                self.analyze_statement(stmt)?;
            }
        }

        self.symbol_table.pop_scope();
        Ok(())
    }

    /// Analyze a test statement
    pub(super) fn analyze_test_statement(&mut self, stmt: &TestStatement) -> Result<()> {
        match stmt {
            TestStatement::Statement(s) => self.analyze_statement(s),
            TestStatement::Assert(assert) => {
                // Check that the condition is boolean
                let cond_type = self.check_expr_type(&assert.condition)?;
                if cond_type != types::Type::Bool && cond_type != types::Type::Unknown {
                    return Err(self.error_at(
                        assert.condition.span(),
                        format!("Assert condition must be boolean, got {}", cond_type),
                    ));
                }
                Ok(())
            }
            TestStatement::Expect(expect) => {
                // Check the actual expression
                self.check_expr_type(&expect.actual)?;
                // Validate the matcher expression if it has one
                self.validate_expectation_matcher(&expect.matcher)?;
                Ok(())
            }
            TestStatement::Should(should) => {
                // Check the subject expression
                self.check_expr_type(&should.subject)?;
                // Validate the matcher
                self.validate_should_matcher(&should.matcher)?;
                Ok(())
            }
            TestStatement::Fixture(fixture) => self.analyze_test_fixture(fixture),
        }
    }

    /// Validate an expectation matcher
    pub(super) fn validate_expectation_matcher(
        &mut self,
        matcher: &ExpectationMatcher,
    ) -> Result<()> {
        match matcher {
            ExpectationMatcher::ToBe(expr)
            | ExpectationMatcher::ToEqual(expr)
            | ExpectationMatcher::ToBeGreaterThan(expr)
            | ExpectationMatcher::ToBeLessThan(expr)
            | ExpectationMatcher::ToContain(expr) => {
                self.check_expr_type(expr)?;
            }
            ExpectationMatcher::ToBeCloseTo { expected, .. } => {
                self.check_expr_type(expected)?;
            }
            ExpectationMatcher::ToBeTruthy
            | ExpectationMatcher::ToBeFalsy
            | ExpectationMatcher::ToThrow(_)
            | ExpectationMatcher::ToMatchPattern { .. } => {
                // These don't have expressions to check
            }
        }
        Ok(())
    }

    /// Validate a should matcher
    pub(super) fn validate_should_matcher(&mut self, matcher: &ShouldMatcher) -> Result<()> {
        match matcher {
            ShouldMatcher::Be(expr) | ShouldMatcher::Equal(expr) | ShouldMatcher::Contain(expr) => {
                self.check_expr_type(expr)?;
            }
            ShouldMatcher::BeCloseTo { expected, .. } => {
                self.check_expr_type(expected)?;
            }
            ShouldMatcher::Match(_) => {
                // Pattern name, no expression to check
            }
        }
        Ok(())
    }

    /// Analyze a test fixture
    pub(super) fn analyze_test_fixture(&mut self, fixture: &TestFixture) -> Result<()> {
        match fixture {
            TestFixture::WithData { data, body } => {
                self.check_expr_type(data)?;
                self.symbol_table.push_scope();
                for stmt in body {
                    self.analyze_statement(stmt)?;
                }
                self.symbol_table.pop_scope();
            }
            TestFixture::WithMock {
                target: _,
                mock_value,
                body,
            } => {
                if let Some(value) = mock_value {
                    self.check_expr_type(value)?;
                }
                self.symbol_table.push_scope();
                for stmt in body {
                    self.analyze_statement(stmt)?;
                }
                self.symbol_table.pop_scope();
            }
        }
        Ok(())
    }
}

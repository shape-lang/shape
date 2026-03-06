//! Default method bodies in traits.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Default method in trait definition
// =========================================================================

#[test]
fn trait_default_method_parses() {
    ShapeTest::new(
        r#"
        trait Queryable {
            filter(pred): any;
            method execute() {
                return self
            }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn trait_default_method_returns_constant() {
    // TDD: default method with a constant return value
    ShapeTest::new(
        r#"
        trait Scored {
            method default_score() {
                return 0
            }
        }
        type Player { name: string }
        impl Scored for Player {}
        let p = Player { name: "Ada" }
        p.default_score()
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn trait_default_method_can_be_overridden() {
    // TDD: impl block should be able to override default method
    ShapeTest::new(
        r#"
        trait Scored {
            method default_score() {
                return 0
            }
        }
        type Player { name: string, score: int }
        impl Scored for Player {
            method default_score() {
                return self.score
            }
        }
        let p = Player { name: "Ada", score: 100 }
        p.default_score()
    "#,
    )
    .expect_number(100.0);
}

// =========================================================================
// Mixed required + default methods
// =========================================================================

#[test]
fn trait_mixed_required_and_default_parses() {
    ShapeTest::new(
        r#"
        trait Serializable {
            serialize(self): string;
            method content_type() {
                return "application/json"
            }
        }
    "#,
    )
    .expect_parse_ok();
}

#[test]
fn trait_multiple_default_methods_parses() {
    ShapeTest::new(
        r#"
        trait Configurable {
            method timeout() { return 30 }
            method retries() { return 3 }
            method verbose() { return false }
        }
    "#,
    )
    .expect_parse_ok();
}

// =========================================================================
// Default method with logic
// =========================================================================

#[test]
fn trait_default_method_with_conditional_parses() {
    ShapeTest::new(
        r#"
        trait Validator {
            validate(self): bool;
            method is_valid() {
                if self.validate() { return "valid" } else { return "invalid" }
            }
        }
    "#,
    )
    .expect_parse_ok();
}

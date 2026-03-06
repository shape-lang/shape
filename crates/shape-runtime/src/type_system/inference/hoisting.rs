//! Optimistic Hoisting Pre-Pass
//!
//! Collects property assignments before the main type checking pass
//! to enable optimistic hoisting of object fields.
//!
//! Example:
//! ```shape
//! let a = {x: 1}
//! a.y = 2  // y is hoisted back to a's type
//! ```
//!
//! After hoisting, `a` has type `{x: number, y: int}` from the start.

use crate::visitor::{Visitor, walk_program};
use shape_ast::ast::{Expr, Program, Span};
use std::collections::HashMap;

/// Collected property assignment information
#[derive(Debug, Clone)]
pub struct PropertyAssignment {
    /// The variable being assigned to (e.g., "a" in "a.b = 2")
    pub variable: String,
    /// The property being assigned (e.g., "b" in "a.b = 2")
    pub property: String,
    /// The expression being assigned (for type inference)
    pub value_expr: Expr,
    /// Span of the assignment expression (`a.b = value`)
    pub assignment_span: Span,
}

/// Visitor that collects all property assignments in a program
pub struct PropertyAssignmentCollector {
    /// Collected property assignments
    pub assignments: Vec<PropertyAssignment>,
}

impl PropertyAssignmentCollector {
    pub fn new() -> Self {
        Self {
            assignments: Vec::new(),
        }
    }

    /// Collect all property assignments from a program
    pub fn collect(program: &Program) -> Vec<PropertyAssignment> {
        let mut collector = Self::new();
        walk_program(&mut collector, program);
        collector.assignments
    }

    /// Group assignments by variable name
    pub fn group_by_variable(
        assignments: &[PropertyAssignment],
    ) -> HashMap<String, Vec<&PropertyAssignment>> {
        let mut grouped: HashMap<String, Vec<&PropertyAssignment>> = HashMap::new();
        for assignment in assignments {
            grouped
                .entry(assignment.variable.clone())
                .or_default()
                .push(assignment);
        }
        grouped
    }
}

impl Default for PropertyAssignmentCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Visitor for PropertyAssignmentCollector {
    fn visit_expr(&mut self, expr: &Expr) -> bool {
        // Look for assignment expressions: a.b = value
        if let Expr::Assign(assign_expr, span) = expr {
            if let Expr::PropertyAccess {
                object, property, ..
            } = assign_expr.target.as_ref()
            {
                // Check if the object is a simple identifier
                if let Expr::Identifier(var_name, _) = object.as_ref() {
                    self.assignments.push(PropertyAssignment {
                        variable: var_name.clone(),
                        property: property.clone(),
                        value_expr: assign_expr.value.as_ref().clone(),
                        assignment_span: *span,
                    });
                }
            }
        }
        true // Continue visiting children
    }

    // Note: We don't override visit_stmt or visit_item because the walker
    // will automatically visit expressions within statements and items.
    // Adding explicit visit_expr calls there would cause double-counting.
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parser::parse_program;

    #[test]
    fn test_collect_property_assignments() {
        let source = r#"
            let a = {x: 1}
            a.y = 2
            a.z = "hello"
        "#;
        let program = parse_program(source).unwrap();
        let assignments = PropertyAssignmentCollector::collect(&program);

        assert_eq!(assignments.len(), 2);
        assert_eq!(assignments[0].variable, "a");
        assert_eq!(assignments[0].property, "y");
        assert_eq!(assignments[1].variable, "a");
        assert_eq!(assignments[1].property, "z");
    }

    #[test]
    fn test_collect_multiple_variables() {
        let source = r#"
            let a = {x: 1}
            let b = {y: 2}
            a.foo = 1
            b.bar = 2
            a.baz = 3
        "#;
        let program = parse_program(source).unwrap();
        let assignments = PropertyAssignmentCollector::collect(&program);

        let grouped = PropertyAssignmentCollector::group_by_variable(&assignments);

        assert_eq!(grouped.get("a").map(|v| v.len()), Some(2));
        assert_eq!(grouped.get("b").map(|v| v.len()), Some(1));
    }
}

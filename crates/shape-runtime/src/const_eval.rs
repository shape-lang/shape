//! Const evaluation for annotation metadata() handlers
//!
//! This module provides compile-time evaluation of Shape expressions.
//! Only a subset of expressions are allowed (literals, object/array construction,
//! annotation parameters, and const arithmetic).
//!
//! ## Purpose
//!
//! Const evaluation enables:
//! - **LSP to extract metadata without runtime execution** - Show code lenses, hover info
//! - **Compiler optimizations** based on static metadata (pure functions, cacheable results)
//! - **Static analysis and documentation generation**
//!
//! ## How It Works
//!
//! When the LSP encounters an `annotation ... { ... }` definition, it:
//!
//! 1. **Parses the annotation definition** (from current file or imports)
//! 2. **Finds the `metadata()` handler** in the handlers list
//! 3. **Const-evaluates the handler body** using this module
//! 4. **Extracts special properties** from the result:
//!    - `code_lens: [...]` → Creates IDE action buttons
//!    - `pure: true` → Marks for compiler optimization
//!    - Custom properties → Stored for user's own tooling
//!
//! Example:
//!
//! ```shape
//! annotation strategy() {
//!     metadata() {
//!         {
//!             is_strategy: true,              // Custom metadata
//!             code_lens: [                    // Special: IDE integration
//!                 { title: "▶ Run", command: "shape.runBacktest" }
//!             ]
//!         }
//!     }
//! }
//! ```
//!
//! When a function has `@strategy`, the LSP:
//! 1. Looks up the `@strategy` annotation definition
//! 2. Const-evaluates `metadata()` → `{ is_strategy: true, code_lens: [...] }`
//! 3. Creates a "▶ Run" button above the function
//!
//! ## Allowed Constructs
//!
//! - Literals: `42`, `"hello"`, `true`, `null`
//! - Objects: `{ key: value, ... }`
//! - Arrays: `[1, 2, 3]`
//! - Annotation parameters (captured in scope)
//! - Const arithmetic: `2 + 2`, `"a" + "b"`
//!
//! ## Not Allowed
//!
//! - Function calls (runtime dependency)
//! - Variable references (except annotation parameters)
//! - `ctx` or `fn` access (runtime state)
//! - Side effects
//! - Non-const conditionals

use shape_ast::ast::{Expr, Literal, ObjectEntry};
use shape_ast::error::{Result, ShapeError};
use shape_value::{ValueWord, ValueWordExt};
use std::collections::HashMap;
use std::sync::Arc;

/// Const evaluator for metadata() handlers
#[derive(Debug, Clone)]
pub struct ConstEvaluator {
    /// Annotation parameters available during evaluation
    /// Maps parameter name → const value
    params: HashMap<String, ValueWord>,
}

impl ConstEvaluator {
    /// Create a new const evaluator with annotation parameters
    pub fn new() -> Self {
        Self {
            params: HashMap::new(),
        }
    }

    /// Create a const evaluator with annotation parameters
    pub fn with_params(params: HashMap<String, ValueWord>) -> Self {
        Self {
            params: params.into_iter().map(|(k, v)| (k, v)).collect(),
        }
    }

    /// Add an annotation parameter to the scope
    pub fn add_param(&mut self, name: String, value: ValueWord) {
        self.params.insert(name, value);
    }

    /// Add an annotation parameter to the scope (ValueWord, avoids ValueWord conversion)
    pub fn add_param_nb(&mut self, name: String, value: ValueWord) {
        self.params.insert(name, value);
    }

    /// Evaluate an expression as a const (compile-time) value
    ///
    /// Returns an error if the expression uses non-const constructs.
    pub fn eval(&self, expr: &Expr) -> Result<ValueWord> {
        Ok(self.eval_nb(expr)?.clone())
    }

    /// Evaluate an expression as a const ValueWord value (avoids ValueWord materialization)
    pub fn eval_as_nb(&self, expr: &Expr) -> Result<ValueWord> {
        self.eval_nb(expr)
    }

    /// Evaluate an expression as a const ValueWord value
    fn eval_nb(&self, expr: &Expr) -> Result<ValueWord> {
        match expr {
            // Literals are always const
            Expr::Literal(lit, _) => match lit {
                Literal::Int(i) => Ok(ValueWord::from_f64(*i as f64)),
                Literal::UInt(u) => Ok(ValueWord::from_native_u64(*u)),
                Literal::TypedInt(v, _) => Ok(ValueWord::from_i64(*v)),
                Literal::Number(n) => Ok(ValueWord::from_f64(*n)),
                Literal::Decimal(d) => {
                    use rust_decimal::prelude::ToPrimitive;
                    Ok(ValueWord::from_f64(d.to_f64().unwrap_or(0.0)))
                }
                Literal::String(s) => Ok(ValueWord::from_string(Arc::new(s.clone()))),
                Literal::FormattedString { value, .. } => {
                    Ok(ValueWord::from_string(Arc::new(value.clone())))
                }
                Literal::ContentString { value, .. } => {
                    Ok(ValueWord::from_string(Arc::new(value.clone())))
                }
                Literal::Char(c) => Ok(ValueWord::from_char(*c)),
                Literal::Bool(b) => Ok(ValueWord::from_bool(*b)),
                Literal::None => Ok(ValueWord::none()),
                Literal::Unit => Ok(ValueWord::unit()),
                Literal::Timeframe(tf) => Ok(ValueWord::from_timeframe(*tf)),
            },

            // Object literals - recursively evaluate all values
            Expr::Object(entries, _) => {
                let mut pairs: Vec<(String, ValueWord)> = Vec::new();
                for entry in entries {
                    match entry {
                        ObjectEntry::Field {
                            key,
                            value,
                            type_annotation: _,
                        } => {
                            let val = self.eval_nb(value)?;
                            pairs.push((key.clone(), val));
                        }
                        ObjectEntry::Spread(_) => {
                            return Err(ShapeError::RuntimeError {
                                message: "Object spread (...) not allowed in const context"
                                    .to_string(),
                                location: None,
                            });
                        }
                    }
                }
                let ref_pairs: Vec<(&str, ValueWord)> =
                    pairs.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
                Ok(crate::type_schema::typed_object_from_nb_pairs(&ref_pairs))
            }

            // Array literals - recursively evaluate all elements
            Expr::Array(elements, _) => {
                let mut arr = Vec::new();
                for elem in elements {
                    arr.push(self.eval_nb(elem)?);
                }
                Ok(ValueWord::from_array(shape_value::vmarray_from_vec(arr)))
            }

            // Identifiers - only allowed if they're annotation parameters
            Expr::Identifier(name, _span) => {
                self.params
                    .get(name)
                    .cloned()
                    .ok_or_else(|| ShapeError::RuntimeError {
                        message: format!(
                            "Cannot reference variable '{}' in const context (metadata()). \
                             Only annotation parameters are allowed.",
                            name
                        ),
                        location: None,
                    })
            }

            // Binary operations - only const arithmetic/string concat
            Expr::BinaryOp {
                left,
                op,
                right,
                span: _,
            } => {
                let left_val = self.eval_nb(left)?;
                let right_val = self.eval_nb(right)?;

                use shape_ast::ast::BinaryOp;
                match op {
                    // Arithmetic
                    BinaryOp::Add => self.const_add_nb(left_val, right_val),
                    BinaryOp::Sub => {
                        self.const_arith_nb(left_val, right_val, "subtraction", |a, b| a - b)
                    }
                    BinaryOp::Mul => {
                        self.const_arith_nb(left_val, right_val, "multiplication", |a, b| a * b)
                    }
                    BinaryOp::Div => {
                        let a = left_val.as_f64().ok_or_else(|| ShapeError::RuntimeError {
                            message: "Const division only works on numbers".to_string(),
                            location: None,
                        })?;
                        let b = right_val.as_f64().ok_or_else(|| ShapeError::RuntimeError {
                            message: "Const division only works on numbers".to_string(),
                            location: None,
                        })?;
                        if b == 0.0 {
                            Err(ShapeError::RuntimeError {
                                message: "Division by zero in const context".to_string(),
                                location: None,
                            })
                        } else {
                            Ok(ValueWord::from_f64(a / b))
                        }
                    }
                    BinaryOp::Mod => {
                        self.const_arith_nb(left_val, right_val, "modulo", |a, b| a % b)
                    }

                    // Comparison
                    BinaryOp::Equal => Ok(ValueWord::from_bool(left_val.vw_equals(&right_val))),
                    BinaryOp::NotEqual => Ok(ValueWord::from_bool(!left_val.vw_equals(&right_val))),
                    BinaryOp::Less => self.const_compare_nb(left_val, right_val, |a, b| a < b),
                    BinaryOp::LessEq => self.const_compare_nb(left_val, right_val, |a, b| a <= b),
                    BinaryOp::Greater => self.const_compare_nb(left_val, right_val, |a, b| a > b),
                    BinaryOp::GreaterEq => {
                        self.const_compare_nb(left_val, right_val, |a, b| a >= b)
                    }

                    // Logical
                    BinaryOp::And => Ok(ValueWord::from_bool(
                        left_val.is_truthy() && right_val.is_truthy(),
                    )),
                    BinaryOp::Or => Ok(ValueWord::from_bool(
                        left_val.is_truthy() || right_val.is_truthy(),
                    )),

                    // Not allowed in const context
                    _ => Err(ShapeError::RuntimeError {
                        message: format!("Binary operator {:?} not allowed in const context", op),
                        location: None,
                    }),
                }
            }

            // Unary operations
            Expr::UnaryOp {
                op,
                operand,
                span: _,
            } => {
                let val = self.eval_nb(operand)?;
                use shape_ast::ast::UnaryOp;
                match op {
                    UnaryOp::Not => Ok(ValueWord::from_bool(!val.is_truthy())),
                    UnaryOp::Neg => {
                        if let Some(n) = val.as_f64() {
                            Ok(ValueWord::from_f64(-n))
                        } else {
                            Err(ShapeError::RuntimeError {
                                message: "Cannot negate non-number in const context".to_string(),
                                location: None,
                            })
                        }
                    }
                    UnaryOp::BitNot => Err(ShapeError::RuntimeError {
                        message: "Bitwise NOT not allowed in const context".to_string(),
                        location: None,
                    }),
                }
            }

            // Everything else is not allowed in const context
            Expr::FunctionCall { .. } => Err(ShapeError::RuntimeError {
                message: "Function calls are not allowed in const context (metadata())".to_string(),
                location: None,
            }),

            Expr::PropertyAccess { .. } => Err(ShapeError::RuntimeError {
                message:
                    "Property access (obj.field) is not allowed in const context (metadata()). \
                         Cannot access runtime state like ctx.* or fn.*"
                        .to_string(),
                location: None,
            }),

            _ => Err(ShapeError::RuntimeError {
                message: format!(
                    "Expression type not allowed in const context (metadata()): {:?}",
                    expr
                ),
                location: None,
            }),
        }
    }

    // Const arithmetic operations (ValueWord)

    fn const_add_nb(&self, left: ValueWord, right: ValueWord) -> Result<ValueWord> {
        if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
            return Ok(ValueWord::from_f64(a + b));
        }
        if let (Some(a), Some(b)) = (left.as_str(), right.as_str()) {
            return Ok(ValueWord::from_string(Arc::new(format!("{}{}", a, b))));
        }
        Err(ShapeError::RuntimeError {
            message: "Const addition only works on numbers or strings".to_string(),
            location: None,
        })
    }

    fn const_arith_nb(
        &self,
        left: ValueWord,
        right: ValueWord,
        op_name: &str,
        f: fn(f64, f64) -> f64,
    ) -> Result<ValueWord> {
        let a = left.as_f64().ok_or_else(|| ShapeError::RuntimeError {
            message: format!("Const {} only works on numbers", op_name),
            location: None,
        })?;
        let b = right.as_f64().ok_or_else(|| ShapeError::RuntimeError {
            message: format!("Const {} only works on numbers", op_name),
            location: None,
        })?;
        Ok(ValueWord::from_f64(f(a, b)))
    }

    fn const_compare_nb(
        &self,
        left: ValueWord,
        right: ValueWord,
        cmp: fn(f64, f64) -> bool,
    ) -> Result<ValueWord> {
        let a = left.as_f64().ok_or_else(|| ShapeError::RuntimeError {
            message: "Const comparison only works on numbers".to_string(),
            location: None,
        })?;
        let b = right.as_f64().ok_or_else(|| ShapeError::RuntimeError {
            message: "Const comparison only works on numbers".to_string(),
            location: None,
        })?;
        Ok(ValueWord::from_bool(cmp(a, b)))
    }
}

impl Default for ConstEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::Span;
    use std::sync::Arc;

    #[test]
    fn test_const_number_literal() {
        let evaluator = ConstEvaluator::new();
        let expr = Expr::Literal(Literal::Number(42.0), Span::DUMMY);
        let result = evaluator.eval(&expr).unwrap();
        assert_eq!(result, ValueWord::from_f64(42.0));
    }

    #[test]
    fn test_const_string_literal() {
        let evaluator = ConstEvaluator::new();
        let expr = Expr::Literal(Literal::String("hello".to_string()), Span::DUMMY);
        let result = evaluator.eval(&expr).unwrap();
        assert_eq!(result.as_str(), Some("hello"));
    }

    #[test]
    fn test_const_formatted_string_literal() {
        let evaluator = ConstEvaluator::new();
        let expr = Expr::Literal(
            Literal::FormattedString {
                value: "value: {x}".to_string(),
                mode: shape_ast::ast::InterpolationMode::Braces,
            },
            Span::DUMMY,
        );
        let result = evaluator.eval(&expr).unwrap();
        assert_eq!(result.as_str(), Some("value: {x}"));
    }

    #[test]
    fn test_const_boolean_literal() {
        let evaluator = ConstEvaluator::new();
        let expr = Expr::Literal(Literal::Bool(true), Span::DUMMY);
        let result = evaluator.eval(&expr).unwrap();
        assert_eq!(result, ValueWord::from_bool(true));
    }

    #[test]
    fn test_const_object_literal() {
        let evaluator = ConstEvaluator::new();
        let expr = Expr::Object(
            vec![
                ObjectEntry::Field {
                    key: "key1".to_string(),
                    value: Expr::Literal(Literal::Number(42.0), Span::DUMMY),
                    type_annotation: None,
                },
                ObjectEntry::Field {
                    key: "key2".to_string(),
                    value: Expr::Literal(Literal::String("value".to_string()), Span::DUMMY),
                    type_annotation: None,
                },
            ],
            Span::DUMMY,
        );
        let result = evaluator.eval(&expr).unwrap();

        let obj =
            crate::type_schema::typed_object_to_hashmap_nb(&result).expect("Expected TypedObject");
        assert_eq!(obj.get("key1").and_then(|v| v.as_f64()), Some(42.0));
        assert_eq!(obj.get("key2").and_then(|v| v.as_str()), Some("value"));
    }

    #[test]
    fn test_const_array_literal() {
        let evaluator = ConstEvaluator::new();
        let expr = Expr::Array(
            vec![
                Expr::Literal(Literal::Number(1.0), Span::DUMMY),
                Expr::Literal(Literal::Number(2.0), Span::DUMMY),
                Expr::Literal(Literal::Number(3.0), Span::DUMMY),
            ],
            Span::DUMMY,
        );
        let result = evaluator.eval(&expr).unwrap();

        let arr = result.as_any_array().expect("Expected array").to_generic();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_f64(), Some(1.0));
        assert_eq!(arr[1].as_f64(), Some(2.0));
        assert_eq!(arr[2].as_f64(), Some(3.0));
    }

    #[test]
    fn test_const_arithmetic_add() {
        let evaluator = ConstEvaluator::new();
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::Literal(Literal::Number(2.0), Span::DUMMY)),
            op: shape_ast::ast::BinaryOp::Add,
            right: Box::new(Expr::Literal(Literal::Number(3.0), Span::DUMMY)),
            span: Span::DUMMY,
        };
        let result = evaluator.eval(&expr).unwrap();
        assert_eq!(result, ValueWord::from_f64(5.0));
    }

    #[test]
    fn test_const_string_concat() {
        let evaluator = ConstEvaluator::new();
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::Literal(
                Literal::String("hello ".to_string()),
                Span::DUMMY,
            )),
            op: shape_ast::ast::BinaryOp::Add,
            right: Box::new(Expr::Literal(
                Literal::String("world".to_string()),
                Span::DUMMY,
            )),
            span: Span::DUMMY,
        };
        let result = evaluator.eval(&expr).unwrap();
        assert_eq!(
            result.as_str(),
            Some("hello world")
        );
    }

    #[test]
    fn test_const_annotation_param() {
        let mut evaluator = ConstEvaluator::new();
        evaluator.add_param("period".to_string(), ValueWord::from_f64(20.0));

        let expr = Expr::Identifier("period".to_string(), Span::DUMMY);
        let result = evaluator.eval(&expr).unwrap();
        assert_eq!(result, ValueWord::from_f64(20.0));
    }

    #[test]
    fn test_const_nested_object() {
        let evaluator = ConstEvaluator::new();
        let expr = Expr::Object(
            vec![
                ObjectEntry::Field {
                    key: "is_test".to_string(),
                    value: Expr::Literal(Literal::Bool(true), Span::DUMMY),
                    type_annotation: None,
                },
                ObjectEntry::Field {
                    key: "code_lens".to_string(),
                    value: Expr::Array(
                        vec![Expr::Object(
                            vec![
                                ObjectEntry::Field {
                                    key: "title".to_string(),
                                    value: Expr::Literal(
                                        Literal::String("Run".to_string()),
                                        Span::DUMMY,
                                    ),
                                    type_annotation: None,
                                },
                                ObjectEntry::Field {
                                    key: "command".to_string(),
                                    value: Expr::Literal(
                                        Literal::String("run".to_string()),
                                        Span::DUMMY,
                                    ),
                                    type_annotation: None,
                                },
                            ],
                            Span::DUMMY,
                        )],
                        Span::DUMMY,
                    ),
                    type_annotation: None,
                },
            ],
            Span::DUMMY,
        );
        let result = evaluator.eval(&expr).unwrap();

        let obj =
            crate::type_schema::typed_object_to_hashmap_nb(&result).expect("Expected TypedObject");
        assert_eq!(obj.get("is_test").and_then(|v| v.as_bool()), Some(true));
        assert!(
            obj.get("code_lens")
                .and_then(|v| v.as_any_array())
                .is_some()
        );
    }

    #[test]
    fn test_const_function_call_fails() {
        let evaluator = ConstEvaluator::new();
        let expr = Expr::FunctionCall {
            name: "foo".to_string(),
            args: vec![],
            named_args: vec![],
            span: Span::DUMMY,
        };
        let result = evaluator.eval(&expr);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not allowed in const context")
        );
    }

    #[test]
    fn test_const_undefined_variable_fails() {
        let evaluator = ConstEvaluator::new();
        let expr = Expr::Identifier("undefined_var".to_string(), Span::DUMMY);
        let result = evaluator.eval(&expr);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("annotation parameters")
        );
    }
}

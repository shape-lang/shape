//! Advanced expression compilation (list comprehension, match, try operator)

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::type_tracking::VariableTypeInfo;
use shape_ast::ast::Expr;
use shape_ast::error::Result;

use shape_runtime::type_system::Type;

use super::super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Compile a list comprehension expression
    pub(super) fn compile_expr_list_comprehension(
        &mut self,
        comp: &shape_ast::ast::ListComprehension,
    ) -> Result<()> {
        self.compile_list_comprehension(comp)
    }

    /// Compile a try operator expression (? operator for Result/Option unwrapping)
    ///
    /// The ? operator unwraps fallible values:
    /// - If Ok(value): unwraps and continues with value
    /// - If Err(error): returns early from the current function with the error
    /// - If None: returns early with an AnyError-compatible Err value
    /// - If Some(value): unwraps and continues with value
    /// - For nullable Option encoding, bare non-None values pass through as success
    ///
    /// The containing function is inferred as fallible and wrapped to Result<T>
    /// by type inference when needed.
    pub(super) fn compile_expr_try_operator(&mut self, inner: &Expr) -> Result<()> {
        // Compile the inner fallible expression.
        self.compile_expr(inner)?;

        // Emit TryUnwrap opcode which handles:
        // 1. Result propagation (Ok/Err)
        // 2. Option propagation (Some/None)
        // 3. Nullable Option runtime encoding compatibility for bare non-None values
        self.emit(Instruction::simple(OpCode::TryUnwrap));
        Ok(())
    }

    /// Compile a match expression
    pub(super) fn compile_expr_match(
        &mut self,
        match_expr: &shape_ast::ast::MatchExpr,
    ) -> Result<()> {
        // Check exhaustiveness before compiling
        self.check_match_exhaustiveness(match_expr)?;

        self.push_scope();
        self.compile_expr(&match_expr.scrutinee)?;
        let scrutinee_local = self.declare_local("__match_scrutinee")?;
        if let Some(schema_id) = self.last_expr_schema {
            self.type_tracker.set_local_type(
                scrutinee_local,
                VariableTypeInfo::known(schema_id, format!("__typed_obj_{}", schema_id)),
            );
        }
        // Propagate full type info (numeric type, storage hint) from the
        // scrutinee expression so that match bindings inherit it.
        self.propagate_initializer_type_to_slot(scrutinee_local, true, false);
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(scrutinee_local)),
        ));

        let mut end_jumps = Vec::new();

        // Capture scrutinee type info for restoring before each arm's binding.
        // This includes schema_id, numeric type, and full type_info so that
        // match binding variables inherit the scrutinee's compile-time type.
        let scrutinee_schema = self
            .type_tracker
            .get_local_type(scrutinee_local)
            .and_then(|info| info.schema_id);
        let scrutinee_numeric_type = self
            .type_tracker
            .get_local_type(scrutinee_local)
            .and_then(|info| Self::storage_hint_to_numeric_type(info.storage_hint));
        let scrutinee_type_info = self.type_tracker.get_local_type(scrutinee_local).cloned();

        for arm in &match_expr.arms {
            // Pattern check — restore scrutinee schema before checking
            self.last_expr_schema = scrutinee_schema;
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(scrutinee_local)),
            ));
            self.compile_pattern_check(&arm.pattern, arm.pattern_span)?;
            let next_arm_jump = self.emit_jump(OpCode::JumpIfFalse, 0);

            // Guard (if present) evaluated with bindings
            let mut guard_fail_jump = None;
            if let Some(guard) = &arm.guard {
                self.push_scope();
                self.last_expr_schema = scrutinee_schema;
                self.last_expr_numeric_type = scrutinee_numeric_type;
                self.last_expr_type_info = scrutinee_type_info.clone();
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(scrutinee_local)),
                ));
                self.compile_match_binding(&arm.pattern)?;
                self.compile_expr(guard)?;
                guard_fail_jump = Some(self.emit_jump(OpCode::JumpIfFalse, 0));
                self.pop_scope();
            }

            // Arm body with bindings
            self.push_scope();
            self.last_expr_schema = scrutinee_schema;
            self.last_expr_numeric_type = scrutinee_numeric_type;
            self.last_expr_type_info = scrutinee_type_info.clone();
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(scrutinee_local)),
            ));
            self.compile_match_binding(&arm.pattern)?;
            self.compile_expr(&arm.body)?;
            self.pop_scope();

            let end_jump = self.emit_jump(OpCode::Jump, 0);
            end_jumps.push(end_jump);

            // Patch failure jumps to the next arm
            self.patch_jump(next_arm_jump);
            if let Some(jump) = guard_fail_jump {
                self.patch_jump(jump);
            }
        }

        // No match - raise runtime error
        let msg = self.program.add_constant(Constant::String(
            "No match arm matched the value".to_string(),
        ));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(msg)),
        ));
        self.emit(Instruction::simple(OpCode::Throw));

        for jump in end_jumps {
            self.patch_jump(jump);
        }
        self.pop_scope();
        Ok(())
    }

    /// Compile `async let name = expr`
    ///
    /// Semantics: spawn the RHS expression as a concurrent task and bind
    /// the resulting Future to a local variable.  The future is later
    /// consumed with `await name`.
    ///
    /// Bytecode:
    ///   compile(expr)   -- push the value / closure onto the stack
    ///   SpawnTask       -- pop value, push Future(task_id)
    ///   StoreLocal(slot)-- bind the future to `name`
    ///   LoadLocal(slot) -- push it back so `async let` is an expression
    pub(super) fn compile_async_let(
        &mut self,
        async_let: &shape_ast::ast::AsyncLetExpr,
    ) -> Result<()> {
        if !self.current_function_is_async {
            return Err(shape_ast::error::ShapeError::SemanticError {
                message: "'async let' can only be used inside an async function".to_string(),
                location: None,
            });
        }

        // ── Three concurrency rules at task boundary ──
        // 1. Owned values (move/clone): always allowed
        // 2. &T (shared ref): allowed in structured child tasks
        // 3. &mut T (exclusive ref): FORBIDDEN — would create aliased mutation
        //
        // Walk the RHS expression to detect exclusive references crossing the boundary.
        self.check_task_boundary_safety(&async_let.expr, async_let.span)?;
        self.plan_flexible_binding_escape_from_expr(&async_let.expr);

        // Compile the RHS expression
        self.compile_expr(&async_let.expr)?;

        // Spawn it as an async task — replaces top-of-stack value with Future(id)
        self.emit(Instruction::simple(OpCode::SpawnTask));

        // Declare a local variable for the future and store it
        let local_idx = self.declare_local(&async_let.name)?;
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(local_idx)),
        ));
        self.immutable_locals.insert(local_idx);
        self.type_tracker
            .set_local_binding_semantics(local_idx, Self::owned_immutable_binding_semantics());

        // `async let` is an expression — push the future back onto the stack
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(local_idx)),
        ));

        Ok(())
    }

    /// Compile `async scope { body }`
    ///
    /// Semantics: open a structured concurrency boundary, execute the body,
    /// then cancel all tasks spawned inside the scope that are still pending
    /// (LIFO order).  Nested scopes are independent.
    ///
    /// Bytecode:
    ///   AsyncScopeEnter   -- push scope marker onto async scope stack
    ///   compile(body)     -- body may spawn tasks via `async let`
    ///   AsyncScopeExit    -- cancel pending tasks in LIFO order, leave result
    pub(super) fn compile_async_scope(&mut self, inner: &Expr) -> Result<()> {
        if !self.current_function_is_async {
            return Err(shape_ast::error::ShapeError::SemanticError {
                message: "'async scope' can only be used inside an async function".to_string(),
                location: None,
            });
        }

        // Enter structured concurrency scope
        self.emit(Instruction::simple(OpCode::AsyncScopeEnter));

        // Compile the body — any `async let` inside will spawn tasks tracked by this scope
        self.compile_expr(inner)?;

        // Exit scope — cancels all pending tasks spawned within
        self.emit(Instruction::simple(OpCode::AsyncScopeExit));

        Ok(())
    }

    /// Check that an expression being spawned as a concurrent task doesn't
    /// capture exclusive (`&mut`) references from the enclosing scope.
    ///
    /// Three concurrency rules:
    /// - Owned values (move/clone): always allowed across task boundary
    /// - `&T` (shared ref): allowed in structured child tasks (truly immutable)
    /// - `&mut T` (exclusive ref): FORBIDDEN (would create aliased mutation)
    fn check_task_boundary_safety(&self, expr: &Expr, span: shape_ast::ast::Span) -> Result<()> {
        // Check for explicit &mut references in the expression
        self.walk_expr_for_exclusive_refs(expr, span)
    }

    /// Walk an expression tree looking for exclusive references that would
    /// cross a task boundary. Reports the first one found.
    fn walk_expr_for_exclusive_refs(
        &self,
        expr: &Expr,
        boundary_span: shape_ast::ast::Span,
    ) -> Result<()> {
        use shape_ast::error::ShapeError;

        match expr {
            // Direct &mut reference — forbidden across task boundary
            Expr::Reference {
                is_mutable: true,
                span,
                ..
            } => {
                return Err(ShapeError::SemanticError {
                    message: "cannot share exclusive reference (&mut) across task boundary — \
                        exclusive references cannot cross into spawned tasks because they would \
                        create aliased mutation. Use an owned value (clone) or a shared reference (&) instead"
                        .to_string(),
                    location: Some(self.span_to_source_location(*span)),
                });
            }

            // Shared refs are OK — recurse into sub-expr for any nested &mut
            Expr::Reference {
                expr: inner,
                is_mutable: false,
                ..
            } => {
                self.walk_expr_for_exclusive_refs(inner, boundary_span)?;
            }

            // Identifier that resolves to an exclusive ref local
            Expr::Identifier(name, id_span) => {
                if let Some(local_idx) = self.resolve_local(name) {
                    if self.exclusive_ref_locals.contains(&local_idx) {
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "cannot share exclusive reference '{}' across task boundary — \
                                exclusive references cannot cross into spawned tasks because they \
                                would create aliased mutation. Use an owned value (clone) or a \
                                shared reference (&) instead",
                                name
                            ),
                            location: Some(self.span_to_source_location(*id_span)),
                        });
                    }
                }
            }

            // Recurse into sub-expressions
            Expr::FunctionCall { args, .. } => {
                for arg in args {
                    self.walk_expr_for_exclusive_refs(arg, boundary_span)?;
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                self.walk_expr_for_exclusive_refs(left, boundary_span)?;
                self.walk_expr_for_exclusive_refs(right, boundary_span)?;
            }
            Expr::UnaryOp { operand, .. } => {
                self.walk_expr_for_exclusive_refs(operand, boundary_span)?;
            }
            Expr::FunctionExpr { .. } => {
                // Function expressions create a new scope — captures are checked at call site
            }
            Expr::Block(block_expr, _) => {
                for item in &block_expr.items {
                    match item {
                        shape_ast::ast::BlockItem::Expression(e) => {
                            self.walk_expr_for_exclusive_refs(e, boundary_span)?;
                        }
                        shape_ast::ast::BlockItem::Statement(
                            shape_ast::ast::Statement::Expression(e, _),
                        ) => {
                            self.walk_expr_for_exclusive_refs(e, boundary_span)?;
                        }
                        _ => {}
                    }
                }
            }
            Expr::PropertyAccess { object, .. } => {
                self.walk_expr_for_exclusive_refs(object, boundary_span)?;
            }
            Expr::MethodCall { receiver, args, .. } => {
                self.walk_expr_for_exclusive_refs(receiver, boundary_span)?;
                for arg in args {
                    self.walk_expr_for_exclusive_refs(arg, boundary_span)?;
                }
            }
            Expr::IndexAccess { object, index, .. } => {
                self.walk_expr_for_exclusive_refs(object, boundary_span)?;
                self.walk_expr_for_exclusive_refs(index, boundary_span)?;
            }
            Expr::If(if_expr, _) => {
                self.walk_expr_for_exclusive_refs(&if_expr.condition, boundary_span)?;
                self.walk_expr_for_exclusive_refs(&if_expr.then_branch, boundary_span)?;
                if let Some(eb) = &if_expr.else_branch {
                    self.walk_expr_for_exclusive_refs(eb, boundary_span)?;
                }
            }
            Expr::Array(elems, _) => {
                for elem in elems {
                    self.walk_expr_for_exclusive_refs(elem, boundary_span)?;
                }
            }
            Expr::Await(inner, _) => {
                self.walk_expr_for_exclusive_refs(inner, boundary_span)?;
            }
            // Leaf expressions (literals, etc.) — no refs to check
            _ => {}
        }
        Ok(())
    }

    /// Check exhaustiveness of a match expression
    ///
    /// Uses the type inference engine to determine the scrutinee type and checks
    /// if all enum variants are covered. Returns a compile error for non-exhaustive matches.
    ///
    /// Note: This requires the type inference engine to have full program context.
    /// If type inference fails (e.g., undefined variable), we skip the check gracefully
    /// since the type inference engine needs full integration to track all program state.
    fn check_match_exhaustiveness(&mut self, match_expr: &shape_ast::ast::MatchExpr) -> Result<()> {
        use shape_runtime::type_system::exhaustiveness;

        // Try to infer scrutinee type
        // If this fails (e.g., undefined variable), fall back to parameter type annotations
        let scrutinee_type = match self.infer_expr_type(&match_expr.scrutinee) {
            Ok(t) => t,
            Err(_) => {
                // Fallback: if scrutinee is a parameter with a type annotation, use it
                if let shape_ast::ast::Expr::Identifier(name, _) = &*match_expr.scrutinee {
                    if let Some(ty) = self.lookup_param_type_annotation(name) {
                        ty
                    } else {
                        return Ok(());
                    }
                } else {
                    return Ok(());
                }
            }
        };

        // Check exhaustiveness for closed types (enums, unions).
        // Union scrutinees must use typed-pattern coverage against their concrete variants.
        let result = if matches!(
            scrutinee_type.to_annotation(),
            Some(shape_ast::ast::TypeAnnotation::Union(_))
        ) {
            exhaustiveness::check_exhaustiveness_for_type(match_expr, &scrutinee_type)
        } else if let Some(semantic_type) = scrutinee_type.to_semantic() {
            let resolved_type = self.type_inference.resolve_named_to_enum(&semantic_type);
            match resolved_type {
                shape_runtime::type_system::semantic::SemanticType::Enum { .. } => {
                    exhaustiveness::check_exhaustiveness(match_expr, &resolved_type)
                }
                _ => exhaustiveness::check_exhaustiveness_for_type(match_expr, &scrutinee_type),
            }
        } else {
            exhaustiveness::check_exhaustiveness_for_type(match_expr, &scrutinee_type)
        };

        // Non-exhaustive matches are ERRORS (not warnings)
        // Without exhaustiveness, match type cannot be determined (no null, no auto-Option<T>)
        match result {
            exhaustiveness::ExhaustivenessResult::NonExhaustive {
                enum_name,
                missing_variants,
            } => Err(shape_ast::error::ShapeError::SemanticError {
                message: format!(
                    "Non-exhaustive match on '{}': missing variants: {}",
                    enum_name,
                    missing_variants.join(", ")
                ),
                location: None,
            }),
            _ => Ok(()), // Exhaustive or not applicable
        }
    }

    /// Look up a parameter's type annotation from the current function's parameter list.
    fn lookup_param_type_annotation(&self, name: &str) -> Option<Type> {
        for param in &self.current_function_params {
            if param.pattern.as_identifier() == Some(name) {
                if let Some(ann) = &param.type_annotation {
                    return Some(Type::Concrete(ann.clone()));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::bytecode::{OpCode, Operand};
    use crate::compiler::BytecodeCompiler;
    use shape_ast::parser::parse_program;

    #[test]
    fn test_match_expression_compiles() {
        // Basic match expression should compile
        let code = r#"
            enum Color { Red, Green, Blue }

            let result = match Color::Red {
                Color::Red => 1,
                Color::Green => 2,
                Color::Blue => 3
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);

        assert!(
            result.is_ok(),
            "Match expression should compile: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_match_with_wildcard() {
        // Match with wildcard pattern should compile
        let code = r#"
            enum Color { Red, Green, Blue }

            let result = match Color::Red {
                Color::Red => 1,
                _ => 2
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);

        assert!(
            result.is_ok(),
            "Match with wildcard should compile: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_typed_union_match_without_wildcard_compiles() {
        let code = r#"
            let result = match (1 as int | string) {
                n: int => n,
                s: string => 0
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "Typed union match should compile: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_typed_union_match_missing_variant_fails_compile() {
        let code = r#"
            let result = match (1 as int | string) {
                n: int => n
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "Missing typed union arm should fail compilation"
        );
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("Non-exhaustive match"),
            "Expected non-exhaustive diagnostic, got: {}",
            msg
        );
    }

    #[test]
    fn test_match_binding_is_immutable() {
        let code = r#"
            function test() {
                let source = Some(1)
                return match source {
                    Some(x) => {
                        x = 2
                        x
                    }
                    None => 0
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(result.is_err(), "match binding reassignment should fail");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("immutable variable 'x'"),
            "unexpected error: {}",
            err_msg
        );
    }

    #[test]
    fn test_exhaustiveness_checker_integrated() {
        // Verify that check_match_exhaustiveness method exists and is called
        // This is a smoke test to ensure the integration is in place
        let code = r#"
            enum Status { Active, Inactive }

            let result = match Status::Active {
                Status::Active => 1,
                Status::Inactive => 2
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);

        // Should compile successfully
        assert!(
            result.is_ok(),
            "Exhaustive match should compile: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_type_inference_engine_initialized() {
        // Verify that BytecodeCompiler has type_inference field
        let compiler = BytecodeCompiler::new();

        // Access the type_inference field to ensure it's initialized
        // This is a compile-time check that the field exists
        let _type_inference = &compiler.type_inference;
    }

    /// Test that exhaustiveness checking infrastructure is in place
    ///
    /// Note: Full exhaustiveness checking requires program-wide type inference
    /// to track variable types. The infrastructure is in place (type_inference engine,
    /// check_match_exhaustiveness method, exhaustiveness::check_exhaustiveness call),
    /// but comprehensive testing requires completing the type inference integration.
    ///
    /// Current status:
    /// - ✅ Type inference engine added to compiler
    /// - ✅ check_match_exhaustiveness method implemented
    /// - ✅ Integration into compile_expr_match
    /// - ⏳ Full type inference pass integration (needed for variable type tracking)
    #[test]
    fn test_exhaustiveness_infrastructure_present() {
        // This test verifies the code structure is in place
        // Actual exhaustiveness checking will be tested once full type inference
        // is integrated (which requires tracking enum types and variable types
        // throughout the program compilation)

        let code = r#"
            enum SimpleEnum { A, B }
            match SimpleEnum::A {
                SimpleEnum::A => 1,
                SimpleEnum::B => 2
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);

        // Should compile (infrastructure is in place)
        assert!(
            result.is_ok(),
            "Infrastructure test should compile: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_unknown_enum_error_highlights_pattern_not_body() {
        // When an unknown enum is used in a match pattern, the error should
        // point to the pattern (Snapshot::Hash), not the arm body.
        let code =
            "match 42 {\n  Snapshot::Hash(id) => print(\"saved\"),\n  _ => print(\"other\"),\n}\n";
        let program = parse_program(code).expect("Failed to parse");
        let mut compiler = BytecodeCompiler::new();
        compiler.set_source(code);
        let result = compiler.compile(&program);
        assert!(result.is_err(), "Should fail for unknown enum");
        let err = result.unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("Unknown enum type"),
            "Error should mention unknown enum type, got: {}",
            msg
        );
        // Verify the error location points to the pattern, not the body
        if let shape_ast::error::ShapeError::SemanticError {
            location: Some(loc),
            ..
        } = &err
        {
            // Pattern "Snapshot::Hash(id)" is on line 2, starting at column 3
            // Body "print(\"saved\")" is further right on the same line
            // The error should point at the pattern (column <= ~20), not at the body (column ~25+)
            assert_eq!(
                loc.line, 2,
                "Error should be on line 2, got line {}",
                loc.line
            );
            assert!(
                loc.column <= 20,
                "Error column should point to pattern start, not body. Got column {}",
                loc.column
            );
        } else {
            panic!("Expected SemanticError with location, got: {:?}", err);
        }
    }

    // ===== Sprint 5: Async Join Compiler Tests =====

    #[test]
    fn test_join_outside_async_is_error() {
        // Using `await join` outside an async function should produce a semantic error
        let code = r#"
            function not_async() {
                await join all {
                    1,
                    2,
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "await join outside async should produce an error"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("async"),
            "Error should mention async, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_join_all_compiles_in_async() {
        // `await join all { ... }` inside an async function should compile
        // Use simple literal expressions to avoid "undefined function" errors
        let code = r#"
            async function fetch_all() {
                await join all {
                    1 + 2,
                    3 + 4,
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "await join all in async function should compile: {:?}",
            result.err()
        );

        // Verify opcode sequence contains SpawnTask, JoinInit, JoinAwait
        let bytecode = result.unwrap();
        let instructions = &bytecode.instructions;
        let opcodes: Vec<_> = instructions.iter().map(|i| i.opcode).collect();

        assert!(
            opcodes.contains(&OpCode::SpawnTask),
            "Should contain SpawnTask opcode, got: {:?}",
            opcodes
        );
        assert!(
            opcodes.contains(&OpCode::JoinInit),
            "Should contain JoinInit opcode, got: {:?}",
            opcodes
        );
        assert!(
            opcodes.contains(&OpCode::JoinAwait),
            "Should contain JoinAwait opcode, got: {:?}",
            opcodes
        );

        // Count SpawnTask opcodes — should be 2 (one per branch)
        let spawn_count = opcodes
            .iter()
            .filter(|&&op| op == OpCode::SpawnTask)
            .count();
        assert_eq!(
            spawn_count, 2,
            "Should have 2 SpawnTask opcodes (one per branch)"
        );
    }

    #[test]
    fn test_join_init_operand_encoding() {
        // Verify the packed operand encoding for JoinInit
        let code = r#"
            async function test_join() {
                await join race {
                    10,
                    20,
                    30,
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(result.is_ok(), "Should compile: {:?}", result.err());

        let bytecode = result.unwrap();
        // Find JoinInit instruction and verify operand
        let join_init = bytecode
            .instructions
            .iter()
            .find(|i| i.opcode == OpCode::JoinInit)
            .expect("Should have JoinInit instruction");

        match &join_init.operand {
            Some(Operand::Count(packed)) => {
                let kind = (packed >> 14) & 0x03;
                let arity = packed & 0x3FFF;
                assert_eq!(kind, 1, "Kind should be 1 (Race)");
                assert_eq!(arity, 3, "Arity should be 3");
            }
            other => panic!("Expected Count operand, got: {:?}", other),
        }
    }

    #[test]
    fn test_annotated_expression_compiles() {
        // @annotation expr should compile (annotation is metadata, target is compiled)
        // Use simple expression to avoid undefined function errors
        let code = r#"
            annotation timeout(duration) {}
            async function with_anno() {
                await @timeout(5s) 42
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "Annotated await should compile: {:?}",
            result.err()
        );
    }

    // ===== Sprint 7: Structured Concurrency + Async Trait Methods =====

    #[test]
    fn test_async_let_compiles_in_async_function() {
        let code = r#"
            async function fetch_data() {
                async let x = 1 + 2
                await x
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "async let in async function should compile: {:?}",
            result.err()
        );

        let bytecode = result.unwrap();
        let opcodes: Vec<_> = bytecode.instructions.iter().map(|i| i.opcode).collect();

        // Should have SpawnTask (for async let) and StoreLocal (for binding)
        assert!(
            opcodes.contains(&OpCode::SpawnTask),
            "async let should emit SpawnTask opcode"
        );
    }

    #[test]
    fn test_async_let_outside_async_is_error() {
        let code = r#"
            function sync_func() {
                async let x = 1 + 2
                x
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "async let outside async should be an error"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("async"),
            "Error should mention async, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_async_let_binding_is_immutable() {
        let code = r#"
            async function fetch_data() {
                async let x = 1 + 2
                x = 3
                await x
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(result.is_err(), "async let reassignment should fail");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("immutable variable 'x'"),
            "unexpected error: {}",
            err_msg
        );
    }

    #[test]
    fn test_async_scope_compiles_in_async_function() {
        let code = r#"
            async function process() {
                async scope {
                    42
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "async scope in async function should compile: {:?}",
            result.err()
        );

        let bytecode = result.unwrap();
        let opcodes: Vec<_> = bytecode.instructions.iter().map(|i| i.opcode).collect();

        assert!(
            opcodes.contains(&OpCode::AsyncScopeEnter),
            "async scope should emit AsyncScopeEnter opcode"
        );
        assert!(
            opcodes.contains(&OpCode::AsyncScopeExit),
            "async scope should emit AsyncScopeExit opcode"
        );
    }

    #[test]
    fn test_async_scope_outside_async_is_error() {
        let code = r#"
            function sync_func() {
                async scope {
                    42
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "async scope outside async should be an error"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("async"),
            "Error should mention async, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_async_scope_with_async_let_inside() {
        // Use a single async let inside an async scope to verify they interact correctly
        let code = r#"
            async function structured() {
                async scope {
                    async let a = 10
                    await a
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "async scope with async let should compile: {:?}",
            result.err()
        );

        let bytecode = result.unwrap();
        let opcodes: Vec<_> = bytecode.instructions.iter().map(|i| i.opcode).collect();

        // Should have AsyncScopeEnter, SpawnTask, Await, AsyncScopeExit
        assert!(
            opcodes.contains(&OpCode::AsyncScopeEnter),
            "Should contain AsyncScopeEnter"
        );
        assert!(
            opcodes.contains(&OpCode::AsyncScopeExit),
            "Should contain AsyncScopeExit"
        );
        assert!(
            opcodes.contains(&OpCode::SpawnTask),
            "Should contain SpawnTask (from async let)"
        );
        assert!(
            opcodes.contains(&OpCode::Await),
            "Should contain Await (from await a)"
        );
    }

    #[test]
    fn test_for_await_compiles_in_async_function() {
        let code = r#"
            async function consume_stream() {
                let items = [1, 2, 3]
                for await item in items {
                    item
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "for await in async function should compile: {:?}",
            result.err()
        );

        let bytecode = result.unwrap();
        let opcodes: Vec<_> = bytecode.instructions.iter().map(|i| i.opcode).collect();

        // Should have Await opcode in the loop
        assert!(
            opcodes.contains(&OpCode::Await),
            "for await should emit Await opcode"
        );
    }

    #[test]
    fn test_for_await_outside_async_is_error() {
        let code = r#"
            function sync_func() {
                let items = [1, 2, 3]
                for await item in items {
                    item
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "for await outside async should be an error"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("async"),
            "Error should mention async, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_async_scope_opcode_ordering() {
        // Verify the opcode sequence: AsyncScopeEnter → body → AsyncScopeExit
        let code = r#"
            async function test() {
                async scope {
                    99
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        let opcodes: Vec<_> = bytecode.instructions.iter().map(|i| i.opcode).collect();

        let enter_pos = opcodes
            .iter()
            .position(|&op| op == OpCode::AsyncScopeEnter)
            .expect("Should have AsyncScopeEnter");
        let exit_pos = opcodes
            .iter()
            .position(|&op| op == OpCode::AsyncScopeExit)
            .expect("Should have AsyncScopeExit");

        assert!(
            enter_pos < exit_pos,
            "AsyncScopeEnter (pos {}) should come before AsyncScopeExit (pos {})",
            enter_pos,
            exit_pos
        );
    }
}

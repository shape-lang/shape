//! Helper methods for bytecode compilation

use super::{BorrowMode, BorrowPlace};
use crate::bytecode::{BuiltinFunction, Constant, Instruction, OpCode, Operand};
use crate::executor::typed_object_ops::field_type_to_tag;
use crate::type_tracking::{
    Aliasability, BindingOwnershipClass, BindingSemantics, BindingStorageClass, EscapeStatus,
    MutationCapability, NumericType, StorageHint, TypeTracker, VariableKind, VariableTypeInfo,
};
use shape_ast::ast::{
    BlockItem, DestructurePattern, Expr, FunctionParameter, Item, Pattern,
    PatternConstructorFields, Spanned, Statement, TypeAnnotation,
};
use shape_ast::error::{Result, ShapeError, SourceLocation};
use shape_runtime::type_schema::FieldType;
use std::collections::{BTreeSet, HashMap, HashSet};

use super::{
    BuiltinNameResolution, BytecodeCompiler, DropKind, FunctionReturnReferenceSummary,
    ParamPassMode, ResolutionScope,
};

pub(super) struct TypedFieldPlace {
    pub root_name: String,
    pub is_local: bool,
    pub slot: u16,
    pub typed_operand: Operand,
    pub borrow_key: BorrowPlace,
    pub field_type_info: FieldType,
}

/// Extract the core error message from a ShapeError, stripping redundant
/// "Type error:", "Runtime error:", "Compile error:", etc. prefixes that
/// thiserror's Display impl adds.  This prevents nested comptime errors
/// from accumulating multiple prefixes like
/// "Runtime error: Comptime block evaluation failed: Runtime error: …".
pub(crate) fn strip_error_prefix(e: &ShapeError) -> String {
    let msg = e.to_string();
    // Known prefixes added by thiserror Display
    const PREFIXES: &[&str] = &[
        "Runtime error: ",
        "Type error: ",
        "Semantic error: ",
        "Parse error: ",
        "VM error: ",
        "Lexical error: ",
    ];
    let mut s = msg.as_str();
    // Strip at most 3 layers of prefix to handle deep nesting
    for _ in 0..3 {
        let mut stripped = false;
        for prefix in PREFIXES {
            if let Some(rest) = s.strip_prefix(prefix) {
                s = rest;
                stripped = true;
                break;
            }
        }
        // Also strip the comptime wrapping messages themselves
        const COMPTIME_PREFIXES: &[&str] = &[
            "Comptime block evaluation failed: ",
            "Comptime handler execution failed: ",
            "Comptime block directive processing failed: ",
        ];
        for prefix in COMPTIME_PREFIXES {
            if let Some(rest) = s.strip_prefix(prefix) {
                s = rest;
                stripped = true;
                break;
            }
        }
        if !stripped {
            break;
        }
    }
    s.to_string()
}

fn assignment_target_uses_identifier(target: &Expr, name: &str) -> bool {
    match target {
        Expr::Identifier(_, _) => false,
        Expr::PropertyAccess { object, .. } => expr_uses_identifier(object, name),
        Expr::IndexAccess {
            object,
            index,
            end_index,
            ..
        } => {
            expr_uses_identifier(object, name)
                || expr_uses_identifier(index, name)
                || end_index
                    .as_deref()
                    .is_some_and(|end| expr_uses_identifier(end, name))
        }
        other => expr_uses_identifier(other, name),
    }
}

fn block_item_uses_identifier(item: &BlockItem, name: &str) -> bool {
    match item {
        BlockItem::VariableDecl(decl) => decl
            .value
            .as_ref()
            .is_some_and(|value| expr_uses_identifier(value, name)),
        BlockItem::Assignment(assign) => expr_uses_identifier(&assign.value, name),
        BlockItem::Statement(stmt) => statement_uses_identifier(stmt, name),
        BlockItem::Expression(expr) => expr_uses_identifier(expr, name),
    }
}

fn statement_uses_identifier(stmt: &Statement, name: &str) -> bool {
    use shape_ast::ast::ForInit;

    match stmt {
        Statement::VariableDecl(decl, _) => decl
            .value
            .as_ref()
            .is_some_and(|value| expr_uses_identifier(value, name)),
        Statement::Assignment(assign, _) => expr_uses_identifier(&assign.value, name),
        Statement::Expression(expr, _) => expr_uses_identifier(expr, name),
        Statement::Return(Some(expr), _) => expr_uses_identifier(expr, name),
        Statement::If(if_stmt, _) => {
            expr_uses_identifier(&if_stmt.condition, name)
                || if_stmt
                    .then_body
                    .iter()
                    .any(|stmt| statement_uses_identifier(stmt, name))
                || if_stmt.else_body.as_ref().is_some_and(|else_body| {
                    else_body
                        .iter()
                        .any(|stmt| statement_uses_identifier(stmt, name))
                })
        }
        Statement::While(while_loop, _) => {
            expr_uses_identifier(&while_loop.condition, name)
                || while_loop
                    .body
                    .iter()
                    .any(|stmt| statement_uses_identifier(stmt, name))
        }
        Statement::For(for_loop, _) => {
            let init_uses = match &for_loop.init {
                ForInit::ForIn { iter, .. } => expr_uses_identifier(iter, name),
                ForInit::ForC {
                    init,
                    condition,
                    update,
                } => {
                    statement_uses_identifier(init, name)
                        || expr_uses_identifier(condition, name)
                        || expr_uses_identifier(update, name)
                }
            };
            init_uses
                || for_loop
                    .body
                    .iter()
                    .any(|stmt| statement_uses_identifier(stmt, name))
        }
        Statement::Extend(ext, _) => ext.methods.iter().any(|method| {
            method
                .body
                .iter()
                .any(|stmt| statement_uses_identifier(stmt, name))
        }),
        Statement::SetParamValue { expression, .. }
        | Statement::SetReturnExpr { expression, .. }
        | Statement::ReplaceBodyExpr { expression, .. }
        | Statement::ReplaceModuleExpr { expression, .. } => expr_uses_identifier(expression, name),
        Statement::ReplaceBody { body, .. } => body
            .iter()
            .any(|stmt| statement_uses_identifier(stmt, name)),
        Statement::Break(_)
        | Statement::Continue(_)
        | Statement::Return(None, _)
        | Statement::RemoveTarget(_)
        | Statement::SetParamType { .. }
        | Statement::SetReturnType { .. } => false,
    }
}

fn item_uses_identifier(item: &Item, name: &str) -> bool {
    match item {
        Item::VariableDecl(decl, _) => decl
            .value
            .as_ref()
            .is_some_and(|value| expr_uses_identifier(value, name)),
        Item::Assignment(assign, _) => expr_uses_identifier(&assign.value, name),
        Item::Expression(expr, _) => expr_uses_identifier(expr, name),
        Item::Statement(stmt, _) => statement_uses_identifier(stmt, name),
        Item::Function(func, _) => func
            .body
            .iter()
            .any(|stmt| statement_uses_identifier(stmt, name)),
        Item::Module(module, _) => module
            .items
            .iter()
            .any(|item| item_uses_identifier(item, name)),
        Item::Export(export, _) => {
            export
                .source_decl
                .as_ref()
                .and_then(|decl| decl.value.as_ref())
                .is_some_and(|value| expr_uses_identifier(value, name))
                || match &export.item {
                    shape_ast::ast::ExportItem::Function(func_def) => func_def
                        .body
                        .iter()
                        .any(|stmt| statement_uses_identifier(stmt, name)),
                    shape_ast::ast::ExportItem::Annotation(annotation_def) => annotation_def
                        .handlers
                        .iter()
                        .any(|handler| expr_uses_identifier(&handler.body, name)),
                    shape_ast::ast::ExportItem::Named(_) => false,
                    shape_ast::ast::ExportItem::TypeAlias(_) => false,
                    shape_ast::ast::ExportItem::BuiltinFunction(_)
                    | shape_ast::ast::ExportItem::BuiltinType(_)
                    | shape_ast::ast::ExportItem::ForeignFunction(_)
                    | shape_ast::ast::ExportItem::Struct(_)
                    | shape_ast::ast::ExportItem::Enum(_)
                    | shape_ast::ast::ExportItem::Trait(_)
                    | shape_ast::ast::ExportItem::Interface(_) => false,
                }
        }
        Item::Extend(ext, _) => ext.methods.iter().any(|method| {
            method
                .body
                .iter()
                .any(|stmt| statement_uses_identifier(stmt, name))
        }),
        Item::Impl(impl_block, _) => impl_block.methods.iter().any(|method| {
            method
                .body
                .iter()
                .any(|stmt| statement_uses_identifier(stmt, name))
        }),
        Item::Import(_, _)
        | Item::Interface(_, _)
        | Item::Query(_, _)
        | Item::Stream(_, _)
        | Item::Test(_, _)
        | Item::Optimize(_, _)
        | Item::TypeAlias(_, _)
        | Item::StructType(_, _)
        | Item::Enum(_, _)
        | Item::Trait(_, _)
        | Item::DataSource(_, _)
        | Item::QueryDecl(_, _)
        | Item::BuiltinTypeDecl(_, _)
        | Item::BuiltinFunctionDecl(_, _)
        | Item::ForeignFunction(_, _)
        | Item::AnnotationDef(_, _) => false,
        Item::Comptime(stmts, _) => stmts
            .iter()
            .any(|stmt| statement_uses_identifier(stmt, name)),
    }
}

fn expr_uses_identifier(expr: &Expr, name: &str) -> bool {
    macro_rules! visit_expr {
        ($expr:expr) => {
            expr_uses_identifier($expr, name)
        };
    }
    macro_rules! visit_stmt {
        ($stmt:expr) => {
            statement_uses_identifier($stmt, name)
        };
    }

    match expr {
        Expr::Identifier(ident, _) => ident == name,
        Expr::Assign(assign, _) => {
            assignment_target_uses_identifier(assign.target.as_ref(), name)
                || visit_expr!(&assign.value)
        }
        Expr::FunctionCall {
            args, named_args, ..
        } => {
            args.iter().any(|arg| visit_expr!(arg))
                || named_args.iter().any(|(_, arg)| visit_expr!(arg))
        }
        Expr::QualifiedFunctionCall {
            args, named_args, ..
        } => {
            args.iter().any(|arg| visit_expr!(arg))
                || named_args.iter().any(|(_, arg)| visit_expr!(arg))
        }
        Expr::MethodCall {
            receiver,
            args,
            named_args,
            ..
        } => {
            visit_expr!(receiver)
                || args.iter().any(|arg| visit_expr!(arg))
                || named_args.iter().any(|(_, arg)| visit_expr!(arg))
        }
        Expr::UnaryOp { operand, .. }
        | Expr::Spread(operand, _)
        | Expr::TryOperator(operand, _)
        | Expr::Await(operand, _)
        | Expr::TimeframeContext { expr: operand, .. }
        | Expr::UsingImpl { expr: operand, .. }
        | Expr::Reference { expr: operand, .. }
        | Expr::InstanceOf { expr: operand, .. } => visit_expr!(operand),
        Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
            visit_expr!(left) || visit_expr!(right)
        }
        Expr::PropertyAccess { object, .. } => visit_expr!(object),
        Expr::IndexAccess {
            object,
            index,
            end_index,
            ..
        } => {
            visit_expr!(object)
                || visit_expr!(index)
                || end_index.as_deref().is_some_and(|end| visit_expr!(end))
        }
        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            visit_expr!(condition)
                || visit_expr!(then_expr)
                || else_expr
                    .as_deref()
                    .is_some_and(|else_expr| visit_expr!(else_expr))
        }
        Expr::Array(items, _) => items.iter().any(|item| visit_expr!(item)),
        Expr::TableRows(rows, _) => rows.iter().flatten().any(|value| visit_expr!(value)),
        Expr::Object(entries, _) => entries.iter().any(|entry| match entry {
            shape_ast::ast::ObjectEntry::Field { value, .. } => visit_expr!(value),
            shape_ast::ast::ObjectEntry::Spread(spread) => visit_expr!(spread),
        }),
        Expr::ListComprehension(comp, _) => {
            visit_expr!(&comp.element)
                || comp.clauses.iter().any(|clause| {
                    visit_expr!(&clause.iterable)
                        || clause
                            .filter
                            .as_ref()
                            .is_some_and(|filter| visit_expr!(filter))
                })
        }
        Expr::Block(block, _) => block
            .items
            .iter()
            .any(|item| block_item_uses_identifier(item, name)),
        Expr::FunctionExpr { body, .. } => body.iter().any(|stmt| visit_stmt!(stmt)),
        Expr::If(if_expr, _) => {
            visit_expr!(&if_expr.condition)
                || visit_expr!(&if_expr.then_branch)
                || if_expr
                    .else_branch
                    .as_deref()
                    .is_some_and(|else_branch| visit_expr!(else_branch))
        }
        Expr::While(while_expr, _) => {
            visit_expr!(&while_expr.condition) || visit_expr!(&while_expr.body)
        }
        Expr::For(for_expr, _) => visit_expr!(&for_expr.iterable) || visit_expr!(&for_expr.body),
        Expr::Loop(loop_expr, _) => visit_expr!(&loop_expr.body),
        Expr::Let(let_expr, _) => {
            let_expr
                .value
                .as_ref()
                .is_some_and(|value| visit_expr!(value))
                || visit_expr!(&let_expr.body)
        }
        Expr::Match(match_expr, _) => {
            visit_expr!(&match_expr.scrutinee)
                || match_expr.arms.iter().any(|arm| {
                    arm.guard.as_ref().is_some_and(|guard| visit_expr!(guard))
                        || visit_expr!(&arm.body)
                })
        }
        Expr::Join(join_expr, _) => join_expr
            .branches
            .iter()
            .any(|branch| visit_expr!(&branch.expr)),
        Expr::Annotated { target, .. } => visit_expr!(target),
        Expr::AsyncLet(async_let, _) => visit_expr!(&async_let.expr),
        Expr::AsyncScope(inner, _) => visit_expr!(inner),
        Expr::Comptime(stmts, _) => stmts.iter().any(|stmt| visit_stmt!(stmt)),
        Expr::ComptimeFor(cf, _) => {
            visit_expr!(&cf.iterable) || cf.body.iter().any(|stmt| visit_stmt!(stmt))
        }
        Expr::SimulationCall { params, .. } => params.iter().any(|(_, value)| visit_expr!(value)),
        Expr::WindowExpr(window_expr, _) => {
            let fn_uses = match &window_expr.function {
                shape_ast::ast::WindowFunction::Lag { expr, default, .. }
                | shape_ast::ast::WindowFunction::Lead { expr, default, .. } => {
                    visit_expr!(expr)
                        || default.as_ref().is_some_and(|default| visit_expr!(default))
                }
                shape_ast::ast::WindowFunction::FirstValue(expr)
                | shape_ast::ast::WindowFunction::LastValue(expr)
                | shape_ast::ast::WindowFunction::NthValue(expr, _)
                | shape_ast::ast::WindowFunction::Sum(expr)
                | shape_ast::ast::WindowFunction::Avg(expr)
                | shape_ast::ast::WindowFunction::Min(expr)
                | shape_ast::ast::WindowFunction::Max(expr) => visit_expr!(expr),
                shape_ast::ast::WindowFunction::Count(expr) => {
                    expr.as_ref().is_some_and(|expr| visit_expr!(expr))
                }
                shape_ast::ast::WindowFunction::RowNumber
                | shape_ast::ast::WindowFunction::Rank
                | shape_ast::ast::WindowFunction::DenseRank
                | shape_ast::ast::WindowFunction::Ntile(_) => false,
            };
            fn_uses
                || window_expr
                    .over
                    .partition_by
                    .iter()
                    .any(|expr| visit_expr!(expr))
                || window_expr.over.order_by.as_ref().is_some_and(|order_by| {
                    order_by.columns.iter().any(|(expr, _)| visit_expr!(expr))
                })
        }
        Expr::FromQuery(fq, _) => {
            visit_expr!(&fq.source)
                || fq.clauses.iter().any(|clause| match clause {
                    shape_ast::ast::QueryClause::Where(expr) => visit_expr!(expr),
                    shape_ast::ast::QueryClause::OrderBy(items) => {
                        items.iter().any(|item| visit_expr!(&item.key))
                    }
                    shape_ast::ast::QueryClause::GroupBy { element, key, .. } => {
                        visit_expr!(element) || visit_expr!(key)
                    }
                    shape_ast::ast::QueryClause::Let { value, .. } => visit_expr!(value),
                    shape_ast::ast::QueryClause::Join {
                        source,
                        left_key,
                        right_key,
                        ..
                    } => visit_expr!(source) || visit_expr!(left_key) || visit_expr!(right_key),
                })
                || visit_expr!(&fq.select)
        }
        Expr::StructLiteral { fields, .. } => fields.iter().any(|(_, value)| visit_expr!(value)),
        Expr::EnumConstructor { payload, .. } => match payload {
            shape_ast::ast::EnumConstructorPayload::Unit => false,
            shape_ast::ast::EnumConstructorPayload::Tuple(values) => {
                values.iter().any(|value| visit_expr!(value))
            }
            shape_ast::ast::EnumConstructorPayload::Struct(fields) => {
                fields.iter().any(|(_, value)| visit_expr!(value))
            }
        },
        Expr::TypeAssertion {
            expr,
            meta_param_overrides,
            ..
        } => {
            visit_expr!(expr)
                || meta_param_overrides
                    .as_ref()
                    .is_some_and(|overrides| overrides.values().any(|value| visit_expr!(value)))
        }
        Expr::Range { start, end, .. } => {
            start.as_deref().is_some_and(|start| visit_expr!(start))
                || end.as_deref().is_some_and(|end| visit_expr!(end))
        }
        Expr::DataRelativeAccess { reference, .. } => visit_expr!(reference),
        Expr::Break(Some(expr), _) | Expr::Return(Some(expr), _) => visit_expr!(expr),
        Expr::Literal(..)
        | Expr::DataRef(..)
        | Expr::DataDateTimeRef(..)
        | Expr::TimeRef(..)
        | Expr::DateTime(..)
        | Expr::PatternRef(..)
        | Expr::Unit(..)
        | Expr::Duration(..)
        | Expr::Continue(..)
        | Expr::Break(None, _)
        | Expr::Return(None, _) => false,
    }
}

impl BytecodeCompiler {
    const MODULE_BINDING_BORROW_FLAG: BorrowPlace = 0x8000_0000;
    const FIELD_BORROW_SHIFT: u32 = 16;

    pub(super) fn borrow_key_for_local(local_idx: u16) -> BorrowPlace {
        local_idx as BorrowPlace
    }

    pub(super) fn borrow_key_for_module_binding(binding_idx: u16) -> BorrowPlace {
        Self::MODULE_BINDING_BORROW_FLAG | binding_idx as BorrowPlace
    }

    fn borrow_key_is_module_binding(place: BorrowPlace) -> bool {
        (place & Self::MODULE_BINDING_BORROW_FLAG) != 0
    }

    pub(super) fn check_read_allowed_in_current_context(
        &self,
        _place: BorrowPlace,
        _source_location: Option<SourceLocation>,
    ) -> Result<()> {
        Ok(()) // MIR analysis is the sole authority
    }

    fn encode_field_borrow(field_idx: u16) -> BorrowPlace {
        ((field_idx as BorrowPlace + 1) & 0x7FFF) << Self::FIELD_BORROW_SHIFT
    }

    pub(super) fn borrow_key_for_local_field(local_idx: u16, field_idx: u16) -> BorrowPlace {
        Self::borrow_key_for_local(local_idx) | Self::encode_field_borrow(field_idx)
    }

    pub(super) fn borrow_key_for_module_binding_field(
        binding_idx: u16,
        field_idx: u16,
    ) -> BorrowPlace {
        Self::borrow_key_for_module_binding(binding_idx) | Self::encode_field_borrow(field_idx)
    }

    pub(super) fn binding_semantics_for_var_decl(
        var_decl: &shape_ast::ast::VariableDecl,
    ) -> BindingSemantics {
        let ownership_class = match var_decl.kind {
            shape_ast::ast::VarKind::Let if var_decl.is_mut => BindingOwnershipClass::OwnedMutable,
            shape_ast::ast::VarKind::Let | shape_ast::ast::VarKind::Const => {
                BindingOwnershipClass::OwnedImmutable
            }
            shape_ast::ast::VarKind::Var => BindingOwnershipClass::Flexible,
        };
        Self::binding_semantics_for_ownership_class(ownership_class)
    }

    pub(super) const fn default_storage_class_for_ownership_class(
        ownership_class: BindingOwnershipClass,
    ) -> BindingStorageClass {
        match ownership_class {
            BindingOwnershipClass::OwnedImmutable | BindingOwnershipClass::OwnedMutable => {
                BindingStorageClass::Direct
            }
            BindingOwnershipClass::Flexible => BindingStorageClass::Deferred,
        }
    }

    pub(super) const fn binding_semantics_for_ownership_class(
        ownership_class: BindingOwnershipClass,
    ) -> BindingSemantics {
        BindingSemantics {
            ownership_class,
            storage_class: Self::default_storage_class_for_ownership_class(ownership_class),
            aliasability: Aliasability::Unique,
            mutation_capability: match ownership_class {
                BindingOwnershipClass::OwnedImmutable => MutationCapability::Immutable,
                BindingOwnershipClass::OwnedMutable => MutationCapability::LocalMutable,
                BindingOwnershipClass::Flexible => MutationCapability::SharedMutable,
            },
            escape_status: EscapeStatus::Local,
        }
    }

    pub(super) fn binding_semantics_for_param(
        param: &FunctionParameter,
        pass_mode: ParamPassMode,
    ) -> BindingSemantics {
        let ownership_class = if param.is_const || matches!(pass_mode, ParamPassMode::ByRefShared) {
            BindingOwnershipClass::OwnedImmutable
        } else {
            BindingOwnershipClass::OwnedMutable
        };
        let mut semantics = Self::binding_semantics_for_ownership_class(ownership_class);
        if pass_mode.is_reference() {
            semantics.storage_class = BindingStorageClass::Reference;
        }
        semantics
    }

    pub(super) const fn owned_immutable_binding_semantics() -> BindingSemantics {
        Self::binding_semantics_for_ownership_class(BindingOwnershipClass::OwnedImmutable)
    }

    pub(super) const fn owned_mutable_binding_semantics() -> BindingSemantics {
        Self::binding_semantics_for_ownership_class(BindingOwnershipClass::OwnedMutable)
    }

    // ─── Ownership-class-based mutability queries ───────────────────────
    //
    // These consult `BindingOwnershipClass` as the single source of truth
    // for whether a binding is mutable, falling back to the legacy HashSet
    // approach when no ownership class has been recorded yet.

    /// Check if a local slot is immutable according to its ownership class.
    /// Falls back to the `immutable_locals` HashSet if no ownership class was recorded.
    pub(super) fn is_local_immutable(&self, slot: u16) -> bool {
        if let Some(sem) = self.type_tracker.get_local_binding_semantics(slot) {
            return sem.ownership_class == BindingOwnershipClass::OwnedImmutable;
        }
        self.immutable_locals.contains(&slot)
    }

    /// Check if a local slot is const according to its ownership class.
    /// Falls back to the `const_locals` HashSet if no ownership class was recorded.
    pub(super) fn is_local_const(&self, slot: u16) -> bool {
        // `const` bindings are mapped to OwnedImmutable in binding_semantics_for_var_decl,
        // but have additional restrictions (no write-through, no reference). We check the
        // const_locals set as the canonical source since BindingOwnershipClass doesn't
        // distinguish const from let.
        self.const_locals.contains(&slot)
    }

    /// Check if a module binding is immutable according to its ownership class.
    /// Falls back to the `immutable_module_bindings` HashSet.
    pub(super) fn is_module_binding_immutable(&self, slot: u16) -> bool {
        if let Some(sem) = self.type_tracker.get_binding_semantics(slot) {
            return sem.ownership_class == BindingOwnershipClass::OwnedImmutable;
        }
        self.immutable_module_bindings.contains(&slot)
    }

    /// Check if a module binding is const according to its ownership class.
    pub(super) fn is_module_binding_const(&self, slot: u16) -> bool {
        self.const_module_bindings.contains(&slot)
    }

    // ─── MIR ownership decision queries ───────────────────────────────
    //
    // When MIR analysis is available and authoritative, the compiler can
    // consult `OwnershipDecision` to decide Move vs Clone vs Copy for
    // non-Copy type assignments.

    /// Query the MIR ownership decision for a given MIR program point.
    /// Returns `None` if no MIR analysis is available for the current function.
    ///
    /// Connected to assignment/variable-init codegen via the enriched storage
    /// plan and span→point mapping. The actual emission of distinct
    /// move/clone/copy opcodes is a follow-up refinement.
    pub(super) fn query_mir_ownership_decision(
        &self,
        point: crate::mir::types::Point,
    ) -> Option<crate::mir::analysis::OwnershipDecision> {
        let func_name = self
            .current_function
            .map(|idx| &self.program.functions[idx].name)?;
        let analysis = self.mir_borrow_analyses.get(func_name)?;
        Some(analysis.ownership_at(point))
    }

    /// Query the MIR ownership decision for a given AST span.
    /// Looks up the span→point mapping for the current function, then
    /// delegates to `query_mir_ownership_decision`.
    /// Returns `None` if no mapping or analysis is available.
    pub(super) fn query_mir_ownership_decision_at_span(
        &self,
        span: shape_ast::ast::Span,
    ) -> Option<crate::mir::analysis::OwnershipDecision> {
        let func_name = self
            .current_function
            .and_then(|idx| self.program.functions.get(idx))
            .map(|f| f.name.as_str())?;
        let span_map = self.mir_span_to_point.get(func_name)?;
        let point = span_map.get(&span)?;
        self.query_mir_ownership_decision(*point)
    }

    /// Access the storage plan for the function currently being compiled.
    /// Returns `None` if no MIR storage plan exists for the current function.
    pub(super) fn current_storage_plan(&self) -> Option<&crate::mir::StoragePlan> {
        let func_name = self
            .current_function
            .and_then(|idx| self.program.functions.get(idx))
            .map(|f| f.name.as_str())?;
        self.mir_storage_plans.get(func_name)
    }

    /// Query the MIR storage plan for a specific local slot's storage class.
    /// Returns `None` if no plan exists or the slot is not in the plan.
    pub(super) fn mir_storage_class_for_slot(&self, slot: u16) -> Option<BindingStorageClass> {
        self.current_storage_plan()
            .and_then(|plan| plan.slot_classes.get(&crate::mir::SlotId(slot)).copied())
    }

    /// MIR analysis is always authoritative now (lexical borrow checker removed).
    pub(super) fn current_function_has_mir_authority(&self) -> bool {
        true
    }

    /// MIR analysis is authoritative for both function bodies and top-level code.
    /// `analyze_non_function_items_with_mir` runs in the main pipeline before
    /// compilation, so MIR write authority applies universally.
    pub(super) fn current_binding_uses_mir_write_authority(&self, _is_local: bool) -> bool {
        true
    }

    pub(super) fn apply_binding_semantics_to_pattern_bindings(
        &mut self,
        pattern: &DestructurePattern,
        is_local: bool,
        semantics: BindingSemantics,
    ) {
        for (name, _) in pattern.get_bindings() {
            if is_local {
                if let Some(local_idx) = self.resolve_local(&name) {
                    self.type_tracker
                        .set_local_binding_semantics(local_idx, semantics);
                }
            } else {
                let scoped_name = self
                    .resolve_scoped_module_binding_name(&name)
                    .unwrap_or(name);
                if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
                    self.type_tracker
                        .set_binding_semantics(binding_idx, semantics);
                }
            }
        }
    }

    fn for_each_value_pattern_binding_name(pattern: &Pattern, visitor: &mut impl FnMut(&str)) {
        match pattern {
            Pattern::Identifier(name) | Pattern::Typed { name, .. } => visitor(name),
            Pattern::Array(patterns) => {
                for pattern in patterns {
                    Self::for_each_value_pattern_binding_name(pattern, visitor);
                }
            }
            Pattern::Object(fields) => {
                for (_, pattern) in fields {
                    Self::for_each_value_pattern_binding_name(pattern, visitor);
                }
            }
            Pattern::Constructor { fields, .. } => match fields {
                PatternConstructorFields::Unit => {}
                PatternConstructorFields::Tuple(patterns) => {
                    for pattern in patterns {
                        Self::for_each_value_pattern_binding_name(pattern, visitor);
                    }
                }
                PatternConstructorFields::Struct(fields) => {
                    for (_, pattern) in fields {
                        Self::for_each_value_pattern_binding_name(pattern, visitor);
                    }
                }
            },
            Pattern::Wildcard | Pattern::Literal(_) => {}
        }
    }

    pub(super) fn apply_binding_semantics_to_value_pattern_bindings(
        &mut self,
        pattern: &Pattern,
        semantics: BindingSemantics,
    ) {
        Self::for_each_value_pattern_binding_name(pattern, &mut |name| {
            if let Some(local_idx) = self.resolve_local(name) {
                self.type_tracker
                    .set_local_binding_semantics(local_idx, semantics);
            }
        });
    }

    pub(super) fn mark_value_pattern_bindings_immutable(&mut self, pattern: &Pattern) {
        Self::for_each_value_pattern_binding_name(pattern, &mut |name| {
            if let Some(local_idx) = self.resolve_local(name) {
                self.immutable_locals.insert(local_idx);
            }
        });
    }

    fn binding_semantics_for_slot(&self, slot: u16, is_local: bool) -> Option<BindingSemantics> {
        if is_local {
            self.type_tracker.get_local_binding_semantics(slot).copied()
        } else {
            self.type_tracker.get_binding_semantics(slot).copied()
        }
    }

    pub(super) fn binding_semantics_for_name(
        &self,
        name: &str,
    ) -> Option<(u16, bool, BindingSemantics)> {
        if let Some(local_idx) = self.resolve_local(name)
            && let Some(semantics) = self.binding_semantics_for_slot(local_idx, true)
        {
            return Some((local_idx, true, semantics));
        }

        let scoped_name = self
            .resolve_scoped_module_binding_name(name)
            .unwrap_or_else(|| name.to_string());
        self.module_bindings
            .get(&scoped_name)
            .copied()
            .and_then(|binding_idx| {
                self.binding_semantics_for_slot(binding_idx, false)
                    .map(|semantics| (binding_idx, false, semantics))
            })
    }

    fn merged_flexible_storage_class(
        current: BindingStorageClass,
        target: BindingStorageClass,
    ) -> BindingStorageClass {
        use BindingStorageClass::*;

        match target {
            SharedCow => SharedCow,
            UniqueHeap => match current {
                SharedCow | Reference => current,
                _ => UniqueHeap,
            },
            Direct => match current {
                Deferred => Direct,
                _ => current,
            },
            Deferred | Reference => current,
        }
    }

    pub(super) fn promote_flexible_binding_storage_for_slot(
        &mut self,
        slot: u16,
        is_local: bool,
        target: BindingStorageClass,
    ) {
        let Some(semantics) = self.binding_semantics_for_slot(slot, is_local) else {
            return;
        };
        if semantics.ownership_class != BindingOwnershipClass::Flexible
            || semantics.storage_class == BindingStorageClass::Reference
        {
            return;
        }

        let merged = Self::merged_flexible_storage_class(semantics.storage_class, target);
        if merged != semantics.storage_class {
            self.set_binding_storage_class(slot, is_local, merged);
        }
    }

    pub(super) fn promote_flexible_binding_storage_for_name(
        &mut self,
        name: &str,
        target: BindingStorageClass,
    ) {
        if let Some((slot, is_local, _)) = self.binding_semantics_for_name(name) {
            self.promote_flexible_binding_storage_for_slot(slot, is_local, target);
        }
    }

    /// Conservative escape planning for values that are stored beyond the
    /// immediate expression, such as closure captures, return values, or
    /// collection/object elements. This intentionally tracks only direct value
    /// flow and does not attempt full effect analysis of arbitrary calls.
    pub(super) fn plan_flexible_binding_escape_from_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Identifier(name, _) => {
                self.promote_flexible_binding_storage_for_name(
                    name,
                    BindingStorageClass::UniqueHeap,
                );
            }
            Expr::Array(elements, _) => {
                for element in elements {
                    self.plan_flexible_binding_escape_from_expr(element);
                }
            }
            Expr::ListComprehension(comp, _) => {
                self.plan_flexible_binding_escape_from_expr(&comp.element);
            }
            Expr::Object(entries, _) => {
                for entry in entries {
                    match entry {
                        shape_ast::ast::ObjectEntry::Field { value, .. } => {
                            self.plan_flexible_binding_escape_from_expr(value);
                        }
                        shape_ast::ast::ObjectEntry::Spread(expr) => {
                            self.plan_flexible_binding_escape_from_expr(expr);
                        }
                    }
                }
            }
            Expr::Block(block, _) => {
                if let Some(BlockItem::Expression(expr)) = block.items.last() {
                    self.plan_flexible_binding_escape_from_expr(expr);
                }
            }
            Expr::Spread(inner, _)
            | Expr::Annotated { target: inner, .. }
            | Expr::AsyncScope(inner, _)
            | Expr::TypeAssertion { expr: inner, .. }
            | Expr::UsingImpl { expr: inner, .. }
            | Expr::TryOperator(inner, _) => self.plan_flexible_binding_escape_from_expr(inner),
            Expr::If(if_expr, _) => {
                self.plan_flexible_binding_escape_from_expr(&if_expr.then_branch);
                if let Some(else_branch) = if_expr.else_branch.as_deref() {
                    self.plan_flexible_binding_escape_from_expr(else_branch);
                }
            }
            Expr::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                self.plan_flexible_binding_escape_from_expr(then_expr);
                if let Some(else_expr) = else_expr.as_deref() {
                    self.plan_flexible_binding_escape_from_expr(else_expr);
                }
            }
            Expr::While(while_expr, _) => {
                self.plan_flexible_binding_escape_from_expr(&while_expr.body);
            }
            Expr::For(for_expr, _) => {
                self.plan_flexible_binding_escape_from_expr(&for_expr.body);
            }
            Expr::Loop(loop_expr, _) => {
                self.plan_flexible_binding_escape_from_expr(&loop_expr.body);
            }
            Expr::Let(let_expr, _) => {
                self.plan_flexible_binding_escape_from_expr(&let_expr.body);
            }
            Expr::Assign(assign_expr, _) => {
                self.plan_flexible_binding_escape_from_expr(&assign_expr.value);
            }
            Expr::Match(match_expr, _) => {
                for arm in &match_expr.arms {
                    self.plan_flexible_binding_escape_from_expr(&arm.body);
                }
            }
            Expr::Join(join_expr, _) => {
                for branch in &join_expr.branches {
                    self.plan_flexible_binding_escape_from_expr(&branch.expr);
                }
            }
            Expr::AsyncLet(async_let, _) => {
                self.plan_flexible_binding_escape_from_expr(&async_let.expr);
            }
            Expr::EnumConstructor { payload, .. } => match payload {
                shape_ast::ast::EnumConstructorPayload::Unit => {}
                shape_ast::ast::EnumConstructorPayload::Tuple(values) => {
                    for value in values {
                        self.plan_flexible_binding_escape_from_expr(value);
                    }
                }
                shape_ast::ast::EnumConstructorPayload::Struct(fields) => {
                    for (_, value) in fields {
                        self.plan_flexible_binding_escape_from_expr(value);
                    }
                }
            },
            Expr::StructLiteral { fields, .. } => {
                for (_, value) in fields {
                    self.plan_flexible_binding_escape_from_expr(value);
                }
            }
            Expr::TableRows(rows, _) => {
                for row in rows {
                    for value in row {
                        self.plan_flexible_binding_escape_from_expr(value);
                    }
                }
            }
            Expr::FromQuery(from_query, _) => {
                self.plan_flexible_binding_escape_from_expr(&from_query.select);
            }
            _ => {}
        }
    }

    pub(super) fn finalize_flexible_binding_storage_for_slot(&mut self, slot: u16, is_local: bool) {
        let Some(semantics) = self.binding_semantics_for_slot(slot, is_local) else {
            return;
        };
        if semantics.ownership_class != BindingOwnershipClass::Flexible
            || semantics.storage_class != BindingStorageClass::Deferred
        {
            return;
        }
        self.promote_flexible_binding_storage_for_slot(slot, is_local, BindingStorageClass::Direct);
    }

    pub(super) fn plan_flexible_binding_storage_from_expr(
        &mut self,
        slot: u16,
        is_local: bool,
        expr: &Expr,
    ) {
        let Some(semantics) = self.binding_semantics_for_slot(slot, is_local) else {
            return;
        };
        if semantics.ownership_class != BindingOwnershipClass::Flexible
            || semantics.storage_class == BindingStorageClass::Reference
        {
            return;
        }

        if let Expr::Identifier(name, _) = expr
            && let Some((source_slot, source_is_local, source_semantics)) =
                self.binding_semantics_for_name(name)
            && source_semantics.ownership_class == BindingOwnershipClass::Flexible
        {
            self.promote_flexible_binding_storage_for_slot(
                source_slot,
                source_is_local,
                BindingStorageClass::SharedCow,
            );
            self.promote_flexible_binding_storage_for_slot(
                slot,
                is_local,
                BindingStorageClass::SharedCow,
            );
            return;
        }

        self.finalize_flexible_binding_storage_for_slot(slot, is_local);
    }

    pub(super) fn plan_flexible_binding_storage_for_pattern_initializer(
        &mut self,
        pattern: &DestructurePattern,
        is_local: bool,
        initializer: Option<&Expr>,
    ) {
        let bindings = pattern.get_bindings();
        if bindings.is_empty() {
            return;
        }

        if bindings.len() == 1
            && let Some(initializer) = initializer
        {
            let binding_name = &bindings[0].0;
            if is_local {
                if let Some(local_idx) = self.resolve_local(binding_name) {
                    self.plan_flexible_binding_storage_from_expr(local_idx, true, initializer);
                }
            } else {
                let scoped_name = self
                    .resolve_scoped_module_binding_name(binding_name)
                    .unwrap_or_else(|| binding_name.clone());
                if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
                    self.plan_flexible_binding_storage_from_expr(binding_idx, false, initializer);
                }
            }
            return;
        }

        for (binding_name, _) in bindings {
            if is_local {
                if let Some(local_idx) = self.resolve_local(&binding_name) {
                    self.finalize_flexible_binding_storage_for_slot(local_idx, true);
                }
            } else {
                let scoped_name = self
                    .resolve_scoped_module_binding_name(&binding_name)
                    .unwrap_or(binding_name);
                if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
                    self.finalize_flexible_binding_storage_for_slot(binding_idx, false);
                }
            }
        }
    }

    pub(super) fn set_binding_storage_class(
        &mut self,
        slot: u16,
        is_local: bool,
        storage_class: BindingStorageClass,
    ) {
        if is_local {
            self.type_tracker
                .set_local_binding_storage_class(slot, storage_class);
        } else {
            self.type_tracker
                .set_binding_storage_class(slot, storage_class);
        }
    }

    pub(super) fn set_binding_storage_class_for_name(
        &mut self,
        name: &str,
        storage_class: BindingStorageClass,
    ) {
        if let Some(local_idx) = self.resolve_local(name) {
            self.set_binding_storage_class(local_idx, true, storage_class);
            return;
        }

        let scoped_name = self
            .resolve_scoped_module_binding_name(name)
            .unwrap_or_else(|| name.to_string());
        if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
            self.set_binding_storage_class(binding_idx, false, storage_class);
        }
    }

    fn default_binding_storage_class_for_slot(
        &self,
        slot: u16,
        is_local: bool,
    ) -> BindingStorageClass {
        let ownership_class = if is_local {
            self.type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.ownership_class)
        } else {
            self.type_tracker
                .get_binding_semantics(slot)
                .map(|semantics| semantics.ownership_class)
        };
        ownership_class
            .map(Self::default_storage_class_for_ownership_class)
            .unwrap_or(BindingStorageClass::Deferred)
    }

    pub(super) fn relabel_borrow_error(
        err: ShapeError,
        borrow_key: BorrowPlace,
        label: &str,
    ) -> ShapeError {
        match err {
            ShapeError::SemanticError { message, location } => ShapeError::SemanticError {
                message: message
                    .replace(&format!("(slot {})", borrow_key), &format!("'{}'", label)),
                location,
            },
            other => other,
        }
    }

    pub(super) fn try_resolve_typed_field_place(
        &self,
        object: &Expr,
        property: &str,
    ) -> Option<TypedFieldPlace> {
        let (root_name, is_local, slot, type_info) = match object {
            Expr::Identifier(name, _) => {
                if let Some(local_idx) = self.resolve_local(name) {
                    if self.ref_locals.contains(&local_idx)
                        || self.reference_value_locals.contains(&local_idx)
                    {
                        return None;
                    }
                    (
                        name.clone(),
                        true,
                        local_idx,
                        self.type_tracker.get_local_type(local_idx)?.clone(),
                    )
                } else {
                    let scoped_name = self.resolve_scoped_module_binding_name(name)?;
                    let binding_idx = *self.module_bindings.get(&scoped_name)?;
                    if self.reference_value_module_bindings.contains(&binding_idx) {
                        return None;
                    }
                    (
                        name.clone(),
                        false,
                        binding_idx,
                        self.type_tracker.get_binding_type(binding_idx)?.clone(),
                    )
                }
            }
            // Recursive case: handle chained property access like `a.b.c`
            Expr::PropertyAccess {
                object: inner_object,
                property: inner_property,
                optional: false,
                ..
            } => {
                // Resolve the intermediate field place first
                let parent = self.try_resolve_typed_field_place(inner_object, inner_property)?;
                // The intermediate field must be a nested Object type with a known schema
                let nested_type_name = match &parent.field_type_info {
                    FieldType::Object(name) => name.clone(),
                    _ => return None,
                };
                let nested_schema = self.type_tracker.schema_registry().get(&nested_type_name)?;
                let nested_field = nested_schema.get_field(property)?;
                let nested_field_idx = nested_field.index as u16;
                // For chained borrows, the borrow key uses the root slot + leaf field
                let borrow_key = if parent.is_local {
                    Self::borrow_key_for_local_field(parent.slot, nested_field_idx)
                } else {
                    Self::borrow_key_for_module_binding_field(parent.slot, nested_field_idx)
                };
                return Some(TypedFieldPlace {
                    root_name: parent.root_name,
                    is_local: parent.is_local,
                    slot: parent.slot,
                    typed_operand: Operand::TypedField {
                        type_id: nested_schema.id as u16,
                        field_idx: nested_field_idx,
                        field_type_tag: field_type_to_tag(&nested_field.field_type),
                    },
                    borrow_key,
                    field_type_info: nested_field.field_type.clone(),
                });
            }
            _ => return None,
        };

        if !matches!(type_info.kind, VariableKind::Value) {
            return None;
        }

        let schema_id = type_info.schema_id?;
        if schema_id > u16::MAX as u32 {
            return None;
        }

        let schema = self.type_tracker.schema_registry().get_by_id(schema_id)?;
        let field = schema.get_field(property)?;
        let field_idx = field.index as u16;
        let borrow_key = if is_local {
            Self::borrow_key_for_local_field(slot, field_idx)
        } else {
            Self::borrow_key_for_module_binding_field(slot, field_idx)
        };

        Some(TypedFieldPlace {
            root_name,
            is_local,
            slot,
            typed_operand: Operand::TypedField {
                type_id: schema_id as u16,
                field_idx,
                field_type_tag: field_type_to_tag(&field.field_type),
            },
            borrow_key,
            field_type_info: field.field_type.clone(),
        })
    }

    pub(super) fn compile_reference_expr(
        &mut self,
        expr: &Expr,
        span: shape_ast::ast::Span,
        mode: BorrowMode,
    ) -> Result<u32> {
        let borrow_id = match expr {
            Expr::Identifier(name, id_span) => self.compile_reference_identifier(name, *id_span, mode),
            Expr::PropertyAccess {
                object,
                property,
                optional: false,
                ..
            } => self.compile_reference_property_access(object, property, span, mode),
            Expr::IndexAccess {
                object,
                index,
                end_index: None,
                ..
            } => self.compile_reference_index_access(object, index, span, mode),
            _ => Err(ShapeError::SemanticError {
                message:
                    "`&` can only be applied to a place expression (variable, field access, or index access)".to_string(),
                location: Some(self.span_to_source_location(span)),
            }),
        }?;
        self.last_expr_schema = None;
        self.last_expr_type_info = None;
        self.last_expr_numeric_type = None;
        Ok(borrow_id)
    }

    fn compile_reference_property_access(
        &mut self,
        object: &Expr,
        property: &str,
        span: shape_ast::ast::Span,
        mode: BorrowMode,
    ) -> Result<u32> {
        let Some(place) = self.try_resolve_typed_field_place(object, property) else {
            return Err(ShapeError::SemanticError {
                message:
                    "`&` can only be applied to a simple variable name or compile-time-resolved field access (e.g., `&x`, `&obj.a`, `&obj.nested.field`)".to_string(),
                location: Some(self.span_to_source_location(span)),
            });
        };

        if mode == BorrowMode::Exclusive {
            let is_const = if place.is_local {
                self.is_local_const(place.slot)
            } else {
                self.is_module_binding_const(place.slot)
            };
            if is_const {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Cannot pass const variable '{}.{}' by exclusive reference",
                        place.root_name, property
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
        }

        // MIR analysis is the sole authority for borrow checking.
        let borrow_id = u32::MAX;

        let root_operand = if place.is_local {
            Operand::Local(place.slot)
        } else {
            Operand::ModuleBinding(place.slot)
        };
        self.emit(Instruction::new(OpCode::MakeRef, Some(root_operand)));

        // For chained access (a.b.c), emit MakeFieldRef for each nesting level.
        let field_chain = self.collect_property_access_chain(object, property);
        for field_operand in field_chain {
            self.emit(Instruction::new(OpCode::MakeFieldRef, Some(field_operand)));
        }

        Ok(borrow_id)
    }

    /// Collect the chain of typed field operands for a property access path.
    /// For `a.b.c`, returns [operand_for_b, operand_for_c].
    /// For `a.b` (flat), returns [operand_for_b].
    fn collect_property_access_chain(&self, object: &Expr, property: &str) -> Vec<Operand> {
        let mut chain = Vec::new();
        self.collect_property_chain_inner(object, &mut chain);
        // Add the leaf field operand
        if let Some(place) = self.try_resolve_typed_field_place(object, property) {
            chain.push(place.typed_operand);
        }
        chain
    }

    fn collect_property_chain_inner(&self, expr: &Expr, chain: &mut Vec<Operand>) {
        if let Expr::PropertyAccess {
            object: inner_object,
            property: inner_property,
            optional: false,
            ..
        } = expr
        {
            // Recurse into the inner object first
            self.collect_property_chain_inner(inner_object, chain);
            // Resolve this intermediate level
            if let Some(place) = self.try_resolve_typed_field_place(inner_object, inner_property) {
                chain.push(place.typed_operand);
            }
        }
        // Base case: Identifier — no extra field ref needed (handled by MakeRef)
    }

    fn compile_reference_index_access(
        &mut self,
        object: &Expr,
        index: &Expr,
        span: shape_ast::ast::Span,
        mode: BorrowMode,
    ) -> Result<u32> {
        // Resolve the base object to a local or module binding for MakeRef.
        let (root_operand, is_const) = match object {
            Expr::Identifier(name, _id_span) => {
                if let Some(local_idx) = self.resolve_local(name) {
                    (Operand::Local(local_idx), self.is_local_const(local_idx))
                } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
                    if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
                        (
                            Operand::ModuleBinding(binding_idx),
                            self.is_module_binding_const(binding_idx),
                        )
                    } else {
                        return Err(ShapeError::SemanticError {
                            message: "`&expr[i]` requires the base to be a resolvable variable"
                                .to_string(),
                            location: Some(self.span_to_source_location(span)),
                        });
                    }
                } else {
                    return Err(ShapeError::SemanticError {
                        message: format!("Cannot resolve variable '{}' for index reference", name),
                        location: Some(self.span_to_source_location(span)),
                    });
                }
            }
            _ => {
                // For arbitrary base expressions, compile into a temp local.
                self.compile_expr(object)?;
                let temp = self.declare_temp_local("__idx_ref_base_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(temp)),
                ));
                (Operand::Local(temp), false)
            }
        };

        if mode == BorrowMode::Exclusive && is_const {
            return Err(ShapeError::SemanticError {
                message: "Cannot create an exclusive index reference into a const variable"
                    .to_string(),
                location: Some(self.span_to_source_location(span)),
            });
        }

        // MIR analysis is the sole authority for borrow checking.
        let borrow_id = u32::MAX;

        // Emit MakeRef for the base array variable.
        self.emit(Instruction::new(OpCode::MakeRef, Some(root_operand)));
        // Compile the index expression (pushes index value onto stack).
        self.compile_expr(index)?;
        // MakeIndexRef pops [base_ref, index] and pushes a projected index reference.
        self.emit(Instruction::new(OpCode::MakeIndexRef, None));
        Ok(borrow_id)
    }

    pub(super) fn mark_reference_binding(&mut self, slot: u16, is_local: bool, is_exclusive: bool) {
        if is_local {
            self.reference_value_locals.insert(slot);
            if is_exclusive {
                self.exclusive_reference_value_locals.insert(slot);
            } else {
                self.exclusive_reference_value_locals.remove(&slot);
            }
        } else {
            self.reference_value_module_bindings.insert(slot);
            if is_exclusive {
                self.exclusive_reference_value_module_bindings.insert(slot);
            } else {
                self.exclusive_reference_value_module_bindings.remove(&slot);
            }
        }
        self.set_binding_storage_class(slot, is_local, BindingStorageClass::Reference);
    }

    pub(super) fn compile_expr_for_reference_binding(
        &mut self,
        expr: &shape_ast::ast::Expr,
    ) -> Result<Option<(u32, bool)>> {
        if self.expr_should_preserve_reference_binding(expr) {
            self.compile_expr_preserving_refs(expr)?;
            Ok(self
                .last_expr_reference_mode()
                .map(|mode| (u32::MAX, mode == BorrowMode::Exclusive)))
        } else {
            self.compile_expr(expr)?;
            Ok(None)
        }
    }

    fn binding_target_requires_reference_value(&self) -> bool {
        let Some(name) = self.pending_variable_name.as_deref() else {
            return false;
        };

        if let Some(local_idx) = self.resolve_local(name) {
            if self.reference_value_locals.contains(&local_idx) {
                return true;
            }
        } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name)
            && let Some(binding_idx) = self.module_bindings.get(&scoped_name)
            && self.reference_value_module_bindings.contains(binding_idx)
        {
            return true;
        }

        self.future_reference_use_name_scopes
            .iter()
            .rev()
            .any(|scope| scope.contains(name))
    }

    fn expr_should_preserve_reference_binding(&self, expr: &shape_ast::ast::Expr) -> bool {
        match expr {
            shape_ast::ast::Expr::Reference { .. } => true,
            shape_ast::ast::Expr::FunctionCall { .. } | shape_ast::ast::Expr::MethodCall { .. } => {
                self.binding_target_requires_reference_value()
            }
            shape_ast::ast::Expr::Identifier(name, _)
            | shape_ast::ast::Expr::PatternRef(name, _) => {
                if let Some(local_idx) = self.resolve_local(name) {
                    self.reference_value_locals.contains(&local_idx)
                        || (self.binding_target_requires_reference_value()
                            && self.ref_locals.contains(&local_idx))
                } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
                    self.module_bindings
                        .get(&scoped_name)
                        .is_some_and(|binding_idx| {
                            self.reference_value_module_bindings.contains(binding_idx)
                        })
                } else {
                    false
                }
            }
            shape_ast::ast::Expr::Block(block, _) => block
                .items
                .last()
                .is_some_and(|item| self.block_item_should_preserve_reference_binding(item)),
            shape_ast::ast::Expr::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                self.expr_should_preserve_reference_binding(then_expr)
                    || else_expr
                        .as_deref()
                        .is_some_and(|expr| self.expr_should_preserve_reference_binding(expr))
            }
            shape_ast::ast::Expr::If(if_expr, _) => {
                self.expr_should_preserve_reference_binding(&if_expr.then_branch)
                    || if_expr
                        .else_branch
                        .as_deref()
                        .is_some_and(|expr| self.expr_should_preserve_reference_binding(expr))
            }
            shape_ast::ast::Expr::Match(match_expr, _) => match_expr
                .arms
                .iter()
                .any(|arm| self.expr_should_preserve_reference_binding(&arm.body)),
            shape_ast::ast::Expr::Let(let_expr, _) => {
                self.expr_should_preserve_reference_binding(&let_expr.body)
            }
            _ => false,
        }
    }

    fn block_item_should_preserve_reference_binding(&self, item: &BlockItem) -> bool {
        match item {
            BlockItem::Expression(expr) => self.expr_should_preserve_reference_binding(expr),
            _ => false,
        }
    }

    pub(super) fn local_binding_is_reference_value(&self, slot: u16) -> bool {
        self.ref_locals.contains(&slot) || self.reference_value_locals.contains(&slot)
    }

    pub(super) fn local_reference_binding_is_exclusive(&self, slot: u16) -> bool {
        self.exclusive_ref_locals.contains(&slot)
            || self.exclusive_reference_value_locals.contains(&slot)
    }

    fn track_reference_binding_slot(&mut self, _slot: u16, _is_local: bool) {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn bind_reference_value_slot(
        &mut self,
        slot: u16,
        is_local: bool,
        _name: &str,
        is_exclusive: bool,
        _borrow_id: u32,
    ) {
        self.mark_reference_binding(slot, is_local, is_exclusive);
        self.track_reference_binding_slot(slot, is_local);
        if is_local {
            self.type_tracker
                .set_local_type(slot, VariableTypeInfo::unknown());
        } else {
            self.type_tracker
                .set_binding_type(slot, VariableTypeInfo::unknown());
        }
    }

    pub(super) fn bind_untracked_reference_value_slot(
        &mut self,
        slot: u16,
        is_local: bool,
        is_exclusive: bool,
    ) {
        self.mark_reference_binding(slot, is_local, is_exclusive);
        self.track_reference_binding_slot(slot, is_local);
        if is_local {
            self.type_tracker
                .set_local_type(slot, VariableTypeInfo::unknown());
        } else {
            self.type_tracker
                .set_binding_type(slot, VariableTypeInfo::unknown());
        }
    }

    pub(super) fn release_tracked_reference_borrow(&mut self, _slot: u16, _is_local: bool) {
        // Lexical borrow tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn clear_reference_binding(&mut self, slot: u16, is_local: bool) {
        self.release_tracked_reference_borrow(slot, is_local);
        if is_local {
            self.reference_value_locals.remove(&slot);
            self.exclusive_reference_value_locals.remove(&slot);
        } else {
            self.reference_value_module_bindings.remove(&slot);
            self.exclusive_reference_value_module_bindings.remove(&slot);
        }
        let fallback_storage = self.default_binding_storage_class_for_slot(slot, is_local);
        self.set_binding_storage_class(slot, is_local, fallback_storage);
    }

    pub(super) fn update_reference_binding_from_expr(
        &mut self,
        slot: u16,
        is_local: bool,
        expr: &shape_ast::ast::Expr,
    ) {
        if let shape_ast::ast::Expr::Reference { is_mutable, .. } = expr {
            self.clear_reference_binding(slot, is_local);
            self.bind_untracked_reference_value_slot(slot, is_local, *is_mutable);
        } else {
            self.clear_reference_binding(slot, is_local);
        }
    }

    pub(super) fn finish_reference_binding_from_expr(
        &mut self,
        slot: u16,
        is_local: bool,
        name: &str,
        expr: &shape_ast::ast::Expr,
        ref_borrow: Option<(u32, bool)>,
    ) {
        if let Some((borrow_id, is_exclusive)) = ref_borrow {
            self.clear_reference_binding(slot, is_local);
            if borrow_id == u32::MAX {
                self.bind_untracked_reference_value_slot(slot, is_local, is_exclusive);
            } else {
                self.bind_reference_value_slot(slot, is_local, name, is_exclusive, borrow_id);
            }
        } else {
            self.update_reference_binding_from_expr(slot, is_local, expr);
        }
    }

    pub(super) fn callable_pass_modes_from_expr(
        &self,
        expr: &shape_ast::ast::Expr,
    ) -> Option<Vec<ParamPassMode>> {
        match expr {
            shape_ast::ast::Expr::FunctionExpr { params, body, .. } => {
                Some(self.effective_function_like_pass_modes(None, params, Some(body)))
            }
            shape_ast::ast::Expr::Identifier(name, _)
            | shape_ast::ast::Expr::PatternRef(name, _) => self.callable_pass_modes_for_name(name),
            _ => None,
        }
    }

    fn callable_return_reference_summary_from_function_expr(
        &self,
        params: &[shape_ast::ast::FunctionParameter],
        body: &[Statement],
        span: shape_ast::ast::Span,
    ) -> Option<FunctionReturnReferenceSummary> {
        let mut effective_params = params.to_vec();
        let pass_modes = self.effective_function_like_pass_modes(None, params, Some(body));
        for (param, pass_mode) in effective_params.iter_mut().zip(pass_modes) {
            param.is_reference = pass_mode.is_reference();
            param.is_mut_reference = pass_mode.is_exclusive();
        }

        let lowering = crate::mir::lowering::lower_function_detailed(
            "__callable_expr__",
            &effective_params,
            body,
            span,
        );
        if lowering.had_fallbacks {
            return None;
        }

        let callee_summaries = self.build_callee_summaries(None, &lowering.all_local_names);
        crate::mir::solver::analyze(&lowering.mir, &callee_summaries)
            .return_reference_summary
            .map(Into::into)
    }

    pub(super) fn callable_return_reference_summary_from_expr(
        &self,
        expr: &shape_ast::ast::Expr,
    ) -> Option<FunctionReturnReferenceSummary> {
        match expr {
            shape_ast::ast::Expr::FunctionExpr {
                params, body, span, ..
            } => self.callable_return_reference_summary_from_function_expr(params, body, *span),
            shape_ast::ast::Expr::Identifier(name, _)
            | shape_ast::ast::Expr::PatternRef(name, _) => {
                self.function_return_reference_summary_for_name(name)
            }
            _ => None,
        }
    }

    pub(super) fn callable_pass_modes_for_name(&self, name: &str) -> Option<Vec<ParamPassMode>> {
        if let Some(local_idx) = self.resolve_local(name) {
            self.local_callable_pass_modes.get(&local_idx).cloned()
        } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
            let binding_idx = *self.module_bindings.get(&scoped_name)?;
            self.module_binding_callable_pass_modes
                .get(&binding_idx)
                .cloned()
        } else if let Some(func_idx) = self.find_function(name) {
            let func = &self.program.functions[func_idx];
            Some(Self::pass_modes_from_ref_flags(
                &func.ref_params,
                &func.ref_mutates,
            ))
        } else {
            None
        }
    }

    pub(super) fn function_return_reference_summary_for_name(
        &self,
        name: &str,
    ) -> Option<FunctionReturnReferenceSummary> {
        if let Some(local_idx) = self.resolve_local(name) {
            self.local_callable_return_reference_summaries
                .get(&local_idx)
                .cloned()
        } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
            let binding_idx = *self.module_bindings.get(&scoped_name)?;
            self.module_binding_callable_return_reference_summaries
                .get(&binding_idx)
                .cloned()
        } else {
            self.function_return_reference_summaries.get(name).cloned()
        }
    }

    pub(super) fn update_callable_binding_from_expr(
        &mut self,
        slot: u16,
        is_local: bool,
        expr: &shape_ast::ast::Expr,
    ) {
        let pass_modes = self.callable_pass_modes_from_expr(expr);
        let return_summary = self.callable_return_reference_summary_from_expr(expr);
        if is_local {
            if let Some(pass_modes) = pass_modes {
                self.local_callable_pass_modes.insert(slot, pass_modes);
            } else {
                self.local_callable_pass_modes.remove(&slot);
            }
            if let Some(return_summary) = return_summary {
                self.local_callable_return_reference_summaries
                    .insert(slot, return_summary);
            } else {
                self.local_callable_return_reference_summaries.remove(&slot);
            }
        } else if let Some(pass_modes) = pass_modes {
            self.module_binding_callable_pass_modes
                .insert(slot, pass_modes);
            if let Some(return_summary) = return_summary {
                self.module_binding_callable_return_reference_summaries
                    .insert(slot, return_summary);
            } else {
                self.module_binding_callable_return_reference_summaries
                    .remove(&slot);
            }
        } else {
            self.module_binding_callable_pass_modes.remove(&slot);
            self.module_binding_callable_return_reference_summaries
                .remove(&slot);
        }
    }

    pub(super) fn clear_callable_binding(&mut self, slot: u16, is_local: bool) {
        if is_local {
            self.local_callable_pass_modes.remove(&slot);
            self.local_callable_return_reference_summaries.remove(&slot);
        } else {
            self.module_binding_callable_pass_modes.remove(&slot);
            self.module_binding_callable_return_reference_summaries
                .remove(&slot);
        }
    }

    pub(super) fn push_module_reference_scope(&mut self) {
        // Lexical reference scope tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn pop_module_reference_scope(&mut self) {
        // Lexical reference scope tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn collect_reference_use_names_from_expr(
        &self,
        expr: &Expr,
        preserve_result: bool,
        names: &mut HashSet<String>,
    ) {
        match expr {
            Expr::Identifier(name, _) | Expr::PatternRef(name, _) => {
                if preserve_result {
                    names.insert(name.clone());
                }
            }
            Expr::Assign(assign, _) => {
                if let Expr::Identifier(name, _) = assign.target.as_ref() {
                    names.insert(name.clone());
                } else {
                    self.collect_reference_use_names_from_expr(
                        assign.target.as_ref(),
                        false,
                        names,
                    );
                }
                self.collect_reference_use_names_from_expr(&assign.value, false, names);
            }
            Expr::FunctionCall {
                name: callee,
                args,
                named_args,
                ..
            } => {
                let pass_modes = self.callable_pass_modes_for_name(callee);
                for (idx, arg) in args.iter().enumerate() {
                    let preserve_arg = pass_modes
                        .as_ref()
                        .and_then(|modes| modes.get(idx))
                        .is_some_and(|mode| mode.is_reference());
                    self.collect_reference_use_names_from_expr(arg, preserve_arg, names);
                }
                for (_, arg) in named_args {
                    self.collect_reference_use_names_from_expr(arg, false, names);
                }
            }
            Expr::MethodCall {
                receiver,
                args,
                named_args,
                ..
            } => {
                self.collect_reference_use_names_from_expr(receiver, false, names);
                for arg in args {
                    self.collect_reference_use_names_from_expr(arg, false, names);
                }
                for (_, arg) in named_args {
                    self.collect_reference_use_names_from_expr(arg, false, names);
                }
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                self.collect_reference_use_names_from_expr(condition, false, names);
                self.collect_reference_use_names_from_expr(then_expr, preserve_result, names);
                if let Some(else_expr) = else_expr.as_deref() {
                    self.collect_reference_use_names_from_expr(else_expr, preserve_result, names);
                }
            }
            Expr::If(if_expr, _) => {
                self.collect_reference_use_names_from_expr(&if_expr.condition, false, names);
                self.collect_reference_use_names_from_expr(
                    &if_expr.then_branch,
                    preserve_result,
                    names,
                );
                if let Some(else_branch) = if_expr.else_branch.as_deref() {
                    self.collect_reference_use_names_from_expr(else_branch, preserve_result, names);
                }
            }
            Expr::Match(match_expr, _) => {
                self.collect_reference_use_names_from_expr(&match_expr.scrutinee, false, names);
                for arm in &match_expr.arms {
                    if let Some(guard) = arm.guard.as_ref() {
                        self.collect_reference_use_names_from_expr(guard, false, names);
                    }
                    self.collect_reference_use_names_from_expr(&arm.body, preserve_result, names);
                }
            }
            Expr::Block(block, _) => {
                for item in &block.items {
                    self.collect_reference_use_names_from_block_item(item, names);
                }
                if preserve_result && let Some(BlockItem::Expression(expr)) = block.items.last() {
                    self.collect_reference_use_names_from_expr(expr, true, names);
                }
            }
            Expr::Let(let_expr, _) => {
                if let Some(value) = &let_expr.value {
                    self.collect_reference_use_names_from_expr(value, false, names);
                }
                self.collect_reference_use_names_from_expr(&let_expr.body, preserve_result, names);
            }
            Expr::Array(items, _) => {
                for item in items {
                    self.collect_reference_use_names_from_expr(item, false, names);
                }
            }
            Expr::TableRows(rows, _) => {
                for row in rows {
                    for value in row {
                        self.collect_reference_use_names_from_expr(value, false, names);
                    }
                }
            }
            Expr::Object(entries, _) => {
                for entry in entries {
                    match entry {
                        shape_ast::ast::ObjectEntry::Field { value, .. } => {
                            self.collect_reference_use_names_from_expr(value, false, names);
                        }
                        shape_ast::ast::ObjectEntry::Spread(expr) => {
                            self.collect_reference_use_names_from_expr(expr, false, names);
                        }
                    }
                }
            }
            Expr::UnaryOp { operand, .. }
            | Expr::Spread(operand, _)
            | Expr::TryOperator(operand, _)
            | Expr::Await(operand, _)
            | Expr::TimeframeContext { expr: operand, .. }
            | Expr::UsingImpl { expr: operand, .. }
            | Expr::Reference { expr: operand, .. }
            | Expr::InstanceOf { expr: operand, .. } => {
                self.collect_reference_use_names_from_expr(operand, false, names);
            }
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                self.collect_reference_use_names_from_expr(left, false, names);
                self.collect_reference_use_names_from_expr(right, false, names);
            }
            Expr::PropertyAccess { object, .. } => {
                self.collect_reference_use_names_from_expr(object, false, names);
            }
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                self.collect_reference_use_names_from_expr(object, false, names);
                self.collect_reference_use_names_from_expr(index, false, names);
                if let Some(end_index) = end_index.as_deref() {
                    self.collect_reference_use_names_from_expr(end_index, false, names);
                }
            }
            _ => {}
        }
    }

    fn collect_reference_use_names_from_block_item(
        &self,
        item: &BlockItem,
        names: &mut HashSet<String>,
    ) {
        match item {
            BlockItem::VariableDecl(decl) => {
                if let Some(value) = &decl.value {
                    self.collect_reference_use_names_from_expr(value, false, names);
                }
            }
            BlockItem::Assignment(assign) => {
                if let Some(name) = assign.pattern.as_identifier() {
                    names.insert(name.to_string());
                }
                self.collect_reference_use_names_from_expr(&assign.value, false, names);
            }
            BlockItem::Statement(stmt) => {
                self.collect_reference_use_names_from_statement(stmt, names)
            }
            BlockItem::Expression(expr) => {
                self.collect_reference_use_names_from_expr(expr, false, names);
            }
        }
    }

    fn collect_reference_use_names_from_statement(
        &self,
        stmt: &Statement,
        names: &mut HashSet<String>,
    ) {
        use shape_ast::ast::ForInit;

        match stmt {
            Statement::VariableDecl(decl, _) => {
                if let Some(value) = &decl.value {
                    self.collect_reference_use_names_from_expr(value, false, names);
                }
            }
            Statement::Assignment(assign, _) => {
                if let Some(name) = assign.pattern.as_identifier() {
                    names.insert(name.to_string());
                }
                self.collect_reference_use_names_from_expr(&assign.value, false, names);
            }
            Statement::Expression(expr, _) => {
                self.collect_reference_use_names_from_expr(expr, false, names);
            }
            Statement::Return(Some(expr), _) => {
                self.collect_reference_use_names_from_expr(expr, true, names);
            }
            Statement::If(if_stmt, _) => {
                self.collect_reference_use_names_from_expr(&if_stmt.condition, false, names);
                for stmt in &if_stmt.then_body {
                    self.collect_reference_use_names_from_statement(stmt, names);
                }
                if let Some(else_body) = if_stmt.else_body.as_ref() {
                    for stmt in else_body {
                        self.collect_reference_use_names_from_statement(stmt, names);
                    }
                }
            }
            Statement::While(while_loop, _) => {
                self.collect_reference_use_names_from_expr(&while_loop.condition, false, names);
                for stmt in &while_loop.body {
                    self.collect_reference_use_names_from_statement(stmt, names);
                }
            }
            Statement::For(for_loop, _) => {
                match &for_loop.init {
                    ForInit::ForIn { iter, .. } => {
                        self.collect_reference_use_names_from_expr(iter, false, names);
                    }
                    ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        self.collect_reference_use_names_from_statement(init, names);
                        self.collect_reference_use_names_from_expr(condition, false, names);
                        self.collect_reference_use_names_from_expr(update, false, names);
                    }
                }
                for stmt in &for_loop.body {
                    self.collect_reference_use_names_from_statement(stmt, names);
                }
            }
            Statement::Extend(ext, _) => {
                for method in &ext.methods {
                    for stmt in &method.body {
                        self.collect_reference_use_names_from_statement(stmt, names);
                    }
                }
            }
            Statement::SetParamValue { expression, .. }
            | Statement::SetReturnExpr { expression, .. }
            | Statement::ReplaceBodyExpr { expression, .. }
            | Statement::ReplaceModuleExpr { expression, .. } => {
                self.collect_reference_use_names_from_expr(expression, false, names);
            }
            Statement::ReplaceBody { body, .. } => {
                for stmt in body {
                    self.collect_reference_use_names_from_statement(stmt, names);
                }
            }
            Statement::Break(_)
            | Statement::Continue(_)
            | Statement::Return(None, _)
            | Statement::RemoveTarget(_)
            | Statement::SetParamType { .. }
            | Statement::SetReturnType { .. } => {}
        }
    }

    fn collect_reference_use_names_from_item(&self, item: &Item, names: &mut HashSet<String>) {
        match item {
            Item::VariableDecl(decl, _) => {
                if let Some(value) = &decl.value {
                    self.collect_reference_use_names_from_expr(value, false, names);
                }
            }
            Item::Assignment(assign, _) => {
                if let Some(name) = assign.pattern.as_identifier() {
                    names.insert(name.to_string());
                }
                self.collect_reference_use_names_from_expr(&assign.value, false, names);
            }
            Item::Expression(expr, _) => {
                self.collect_reference_use_names_from_expr(expr, false, names);
            }
            Item::Statement(stmt, _) => {
                self.collect_reference_use_names_from_statement(stmt, names)
            }
            Item::Function(func, _) => {
                for stmt in &func.body {
                    self.collect_reference_use_names_from_statement(stmt, names);
                }
            }
            Item::Module(module, _) => {
                for item in &module.items {
                    self.collect_reference_use_names_from_item(item, names);
                }
            }
            Item::Export(export, _) => {
                if let Some(decl) = export.source_decl.as_ref()
                    && let Some(value) = decl.value.as_ref()
                {
                    self.collect_reference_use_names_from_expr(value, false, names);
                }
                if let shape_ast::ast::ExportItem::Function(func_def) = &export.item {
                    for stmt in &func_def.body {
                        self.collect_reference_use_names_from_statement(stmt, names);
                    }
                }
            }
            Item::Extend(ext, _) => {
                for method in &ext.methods {
                    for stmt in &method.body {
                        self.collect_reference_use_names_from_statement(stmt, names);
                    }
                }
            }
            Item::Impl(impl_block, _) => {
                for method in &impl_block.methods {
                    for stmt in &method.body {
                        self.collect_reference_use_names_from_statement(stmt, names);
                    }
                }
            }
            Item::Comptime(stmts, _) => {
                for stmt in stmts {
                    self.collect_reference_use_names_from_statement(stmt, names);
                }
            }
            _ => {}
        }
    }

    pub(super) fn push_future_reference_use_names(&mut self, names: HashSet<String>) {
        self.future_reference_use_name_scopes.push(names);
    }

    pub(super) fn pop_future_reference_use_names(&mut self) {
        self.future_reference_use_name_scopes.pop();
    }

    pub(super) fn future_reference_use_names_for_remaining_statements(
        &self,
        remaining: &[Statement],
    ) -> HashSet<String> {
        let mut names = HashSet::new();
        for stmt in remaining {
            self.collect_reference_use_names_from_statement(stmt, &mut names);
        }
        names
    }

    pub(super) fn future_reference_use_names_for_remaining_block_items(
        &self,
        remaining: &[BlockItem],
    ) -> HashSet<String> {
        let mut names = HashSet::new();
        for item in remaining {
            self.collect_reference_use_names_from_block_item(item, &mut names);
        }
        names
    }

    pub(super) fn future_reference_use_names_for_remaining_items(
        &self,
        remaining: &[Item],
    ) -> HashSet<String> {
        let mut names = HashSet::new();
        for item in remaining {
            self.collect_reference_use_names_from_item(item, &mut names);
        }
        names
    }

    pub(super) fn push_repeating_reference_release_barrier(&mut self) {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn pop_repeating_reference_release_barrier(&mut self) {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
    }

    fn local_reference_release_is_barrier_protected(&self, _slot: u16) -> bool {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
        false
    }

    fn module_reference_release_is_barrier_protected(&self, _slot: u16) -> bool {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
        false
    }

    pub(super) fn check_write_allowed_in_current_context(
        &self,
        _place: BorrowPlace,
        _source_location: Option<SourceLocation>,
    ) -> Result<()> {
        Ok(()) // MIR analysis is the sole authority
    }

    pub(super) fn check_named_binding_write_allowed(
        &self,
        name: &str,
        source_location: Option<SourceLocation>,
    ) -> Result<()> {
        if let Some(local_idx) = self.resolve_local(name) {
            // Immutability/const checks always run — even when MIR is authoritative.
            // MIR authority only bypasses the borrow checker (aliasing) checks below.
            if self.is_local_const(local_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!("Cannot reassign const variable '{}'", name),
                    location: source_location,
                });
            }
            if self.is_local_immutable(local_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Cannot reassign immutable variable '{}'. Use `let mut` or `var` for mutable bindings",
                        name
                    ),
                    location: source_location,
                });
            }
            return Ok(()); // MIR analysis is the sole authority for borrow checks
        }

        let scoped_name = self
            .resolve_scoped_module_binding_name(name)
            .unwrap_or_else(|| name.to_string());
        if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
            // Immutability/const checks always run.
            if self.is_module_binding_const(binding_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!("Cannot reassign const variable '{}'", name),
                    location: source_location,
                });
            }
            if self.is_module_binding_immutable(binding_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Cannot reassign immutable variable '{}'. Use `let mut` or `var` for mutable bindings",
                        name
                    ),
                    location: source_location,
                });
            }
            return Ok(()); // MIR analysis is the sole authority for borrow checks
        }

        Ok(())
    }

    pub(super) fn release_unused_local_reference_borrows_for_remaining_statements(
        &mut self,
        _remaining: &[Statement],
    ) {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn release_unused_local_reference_borrows_for_remaining_block_items(
        &mut self,
        _remaining: &[BlockItem],
    ) {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn release_unused_module_reference_borrows_for_remaining_statements(
        &mut self,
        _remaining: &[Statement],
    ) {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn release_unused_module_reference_borrows_for_remaining_block_items(
        &mut self,
        _remaining: &[BlockItem],
    ) {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
    }

    pub(super) fn release_unused_module_reference_borrows_for_remaining_items(
        &mut self,
        _remaining: &[Item],
    ) {
        // Lexical reference tracking removed — MIR borrow checker is sole authority.
    }

    fn scalar_type_name_from_numeric(numeric_type: NumericType) -> &'static str {
        match numeric_type {
            NumericType::Int | NumericType::IntWidth(_) => "int",
            NumericType::Number => "number",
            NumericType::Decimal => "decimal",
        }
    }

    fn array_type_name_from_numeric(numeric_type: NumericType) -> &'static str {
        match numeric_type {
            NumericType::Int | NumericType::IntWidth(_) => "Vec<int>",
            NumericType::Number => "Vec<number>",
            NumericType::Decimal => "Vec<decimal>",
        }
    }

    fn is_array_type_name(type_name: Option<&str>) -> bool {
        matches!(type_name, Some(name) if name.starts_with("Vec<") && name.ends_with('>'))
    }

    /// Convert a source annotation to a tracked type name when we have a
    /// canonical runtime representation for it.
    pub(super) fn tracked_type_name_from_annotation(type_ann: &TypeAnnotation) -> Option<String> {
        match type_ann {
            TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => Some(name.clone()),
            TypeAnnotation::Array(inner) => Some(format!("Vec<{}>", inner.to_type_string())),
            // Keep the canonical Vec<T> naming even if a Generic slips through.
            TypeAnnotation::Generic { name, args } if name == "Vec" && args.len() == 1 => {
                Some(format!("Vec<{}>", args[0].to_type_string()))
            }
            TypeAnnotation::Generic { name, args } if name == "Mat" && args.len() == 1 => {
                Some(format!("Mat<{}>", args[0].to_type_string()))
            }
            _ => None,
        }
    }

    /// Mark a local/module binding slot as an array with numeric element type.
    ///
    /// Used by `x = x.push(value)` in-place mutation lowering so subsequent
    /// indexed reads can recover numeric hints.
    pub(super) fn mark_slot_as_numeric_array(
        &mut self,
        slot: u16,
        is_local: bool,
        numeric_type: NumericType,
    ) {
        let info =
            VariableTypeInfo::named(Self::array_type_name_from_numeric(numeric_type).to_string());
        if is_local {
            self.type_tracker.set_local_type(slot, info);
        } else {
            self.type_tracker.set_binding_type(slot, info);
        }
    }

    /// Mark a local/module binding slot as a scalar numeric type.
    pub(super) fn mark_slot_as_numeric_scalar(
        &mut self,
        slot: u16,
        is_local: bool,
        numeric_type: NumericType,
    ) {
        let info =
            VariableTypeInfo::named(Self::scalar_type_name_from_numeric(numeric_type).to_string());
        if is_local {
            self.type_tracker.set_local_type(slot, info);
        } else {
            self.type_tracker.set_binding_type(slot, info);
        }
    }

    /// Seed numeric hints from expression usage in arithmetic contexts.
    ///
    /// - `x` in numeric arithmetic becomes scalar numeric (`int`/`number`/`decimal`).
    /// - `arr[i]` implies `arr` is `Vec<numeric>`.
    pub(super) fn seed_numeric_hint_from_expr(
        &mut self,
        expr: &shape_ast::ast::Expr,
        numeric_type: NumericType,
    ) {
        match expr {
            shape_ast::ast::Expr::Identifier(name, _) => {
                if let Some(local_idx) = self.resolve_local(name) {
                    self.mark_slot_as_numeric_scalar(local_idx, true, numeric_type);
                    return;
                }
                let scoped_name = self
                    .resolve_scoped_module_binding_name(name)
                    .unwrap_or_else(|| name.to_string());
                if let Some(binding_idx) = self.module_bindings.get(&scoped_name).copied() {
                    self.mark_slot_as_numeric_scalar(binding_idx, false, numeric_type);
                }
            }
            shape_ast::ast::Expr::IndexAccess {
                object,
                end_index: None,
                ..
            } => {
                if let shape_ast::ast::Expr::Identifier(name, _) = object.as_ref() {
                    if let Some(local_idx) = self.resolve_local(name) {
                        self.mark_slot_as_numeric_array(local_idx, true, numeric_type);
                        return;
                    }
                    let scoped_name = self
                        .resolve_scoped_module_binding_name(name)
                        .unwrap_or_else(|| name.to_string());
                    if let Some(binding_idx) = self.module_bindings.get(&scoped_name).copied() {
                        self.mark_slot_as_numeric_array(binding_idx, false, numeric_type);
                    }
                }
            }
            _ => {}
        }
    }

    fn recover_or_bail_with_null_placeholder(&mut self, err: ShapeError) -> Result<()> {
        if self.should_recover_compile_diagnostics() {
            self.errors.push(err);
            self.emit(Instruction::simple(OpCode::PushNull));
            Ok(())
        } else {
            Err(err)
        }
    }

    pub(super) fn compile_expr_as_value_or_placeholder(
        &mut self,
        expr: &shape_ast::ast::Expr,
    ) -> Result<()> {
        match self.compile_expr(expr) {
            Ok(()) => Ok(()),
            Err(err) => self.recover_or_bail_with_null_placeholder(err),
        }
    }

    /// Emit an instruction and return its index
    /// Also records the current source line and file in debug info
    pub(super) fn emit(&mut self, instruction: Instruction) -> usize {
        let idx = self.program.emit(instruction);
        // Record line number and file for this instruction
        if self.current_line > 0 {
            self.program.debug_info.line_numbers.push((
                idx,
                self.current_file_id,
                self.current_line,
            ));
        }
        idx
    }

    /// Emit a boolean constant
    pub(super) fn emit_bool(&mut self, value: bool) {
        let const_idx = self.program.add_constant(Constant::Bool(value));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(const_idx)),
        ));
    }

    /// Emit a unit constant
    pub(super) fn emit_unit(&mut self) {
        let const_idx = self.program.add_constant(Constant::Unit);
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(const_idx)),
        ));
    }

    /// Emit a jump instruction with placeholder offset.
    ///
    /// When `opcode` is `JumpIfFalse` and the immediately preceding instruction
    /// is a typed or trusted comparison (produces a known bool), upgrades to
    /// `JumpIfFalseTrusted` which skips `is_truthy()` dispatch.
    pub(super) fn emit_jump(&mut self, mut opcode: OpCode, dummy: i32) -> usize {
        if opcode == OpCode::JumpIfFalse && self.last_instruction_produces_bool() {
            opcode = OpCode::JumpIfFalseTrusted;
        }
        self.emit(Instruction::new(opcode, Some(Operand::Offset(dummy))))
    }

    /// Returns true if the last emitted instruction always produces a boolean result.
    fn last_instruction_produces_bool(&self) -> bool {
        self.program
            .instructions
            .last()
            .map(|instr| {
                matches!(
                    instr.opcode,
                    OpCode::GtInt
                        | OpCode::GtNumber
                        | OpCode::GtDecimal
                        | OpCode::LtInt
                        | OpCode::LtNumber
                        | OpCode::LtDecimal
                        | OpCode::GteInt
                        | OpCode::GteNumber
                        | OpCode::GteDecimal
                        | OpCode::LteInt
                        | OpCode::LteNumber
                        | OpCode::LteDecimal
                        | OpCode::EqInt
                        | OpCode::EqNumber
                        | OpCode::NeqInt
                        | OpCode::NeqNumber
                        | OpCode::Gt
                        | OpCode::Lt
                        | OpCode::Gte
                        | OpCode::Lte
                        | OpCode::Eq
                        | OpCode::Neq
                        | OpCode::Not
                        | OpCode::GtIntTrusted
                        | OpCode::LtIntTrusted
                        | OpCode::GteIntTrusted
                        | OpCode::LteIntTrusted
                        | OpCode::GtNumberTrusted
                        | OpCode::LtNumberTrusted
                        | OpCode::GteNumberTrusted
                        | OpCode::LteNumberTrusted
                )
            })
            .unwrap_or(false)
    }

    /// Patch a jump instruction with the correct offset
    pub(super) fn patch_jump(&mut self, jump_idx: usize) {
        let offset = self.program.current_offset() as i32 - jump_idx as i32 - 1;
        self.program.instructions[jump_idx] = Instruction::new(
            self.program.instructions[jump_idx].opcode,
            Some(Operand::Offset(offset)),
        );
    }

    /// Compile function call arguments, enabling `&` reference expressions.
    ///
    /// Each call's arguments get their own borrow region so that borrows from
    /// `&` references are released after the call returns. This matches Rust's
    /// semantics: temporary borrows from function arguments don't persist beyond
    /// the call. Sequential calls like `inc(&a); inc(&a)` are correctly allowed.
    pub(super) fn compile_call_args(
        &mut self,
        args: &[shape_ast::ast::Expr],
        expected_param_modes: Option<&[ParamPassMode]>,
    ) -> Result<Vec<(u16, u16)>> {
        self.call_arg_module_binding_ref_writebacks.push(Vec::new());

        let mut first_error: Option<ShapeError> = None;
        for (idx, arg) in args.iter().enumerate() {
            let pass_mode = expected_param_modes
                .and_then(|modes| modes.get(idx).copied())
                .unwrap_or(ParamPassMode::ByValue);

            let arg_result = match pass_mode {
                ParamPassMode::ByRefExclusive | ParamPassMode::ByRefShared => {
                    let borrow_mode = if pass_mode.is_exclusive() {
                        BorrowMode::Exclusive
                    } else {
                        BorrowMode::Shared
                    };
                    if let shape_ast::ast::Expr::Reference { expr, span, .. } = arg {
                        self.compile_reference_expr(expr, *span, borrow_mode)
                            .map(|_| ())
                    } else {
                        self.compile_implicit_reference_arg(arg, borrow_mode)
                    }
                }
                ParamPassMode::ByValue => {
                    if let shape_ast::ast::Expr::Reference { span, .. } = arg {
                        let message = if expected_param_modes.is_some() {
                            "[B0004] unexpected `&` argument: target parameter is not a reference parameter".to_string()
                        } else {
                            "[B0004] cannot pass `&` to a callable value without a declared reference contract; \
                             call a named function with known parameter modes or add an explicit callable type"
                                .to_string()
                        };
                        Err(ShapeError::SemanticError {
                            message,
                            location: Some(self.span_to_source_location(*span)),
                        })
                    } else {
                        self.plan_flexible_binding_escape_from_expr(arg);
                        self.compile_expr(arg)
                    }
                }
            };

            if let Err(err) = arg_result {
                if self.should_recover_compile_diagnostics() {
                    self.errors.push(err);
                    // Keep stack arity consistent for downstream call codegen.
                    self.emit(Instruction::simple(OpCode::PushNull));
                    continue;
                }
                first_error = Some(err);
                break;
            }
        }

        let writebacks = self
            .call_arg_module_binding_ref_writebacks
            .pop()
            .unwrap_or_default();
        if let Some(err) = first_error {
            Err(err)
        } else {
            Ok(writebacks)
        }
    }

    pub(super) fn current_arg_borrow_mode(&self) -> BorrowMode {
        // With lexical borrow checker removed, default to Exclusive for codegen.
        BorrowMode::Exclusive
    }

    pub(super) fn record_call_arg_module_binding_writeback(
        &mut self,
        local: u16,
        module_binding: u16,
    ) {
        if let Some(stack) = self.call_arg_module_binding_ref_writebacks.last_mut() {
            stack.push((local, module_binding));
        }
    }

    pub(super) fn compile_implicit_reference_arg(
        &mut self,
        arg: &shape_ast::ast::Expr,
        mode: BorrowMode,
    ) -> Result<()> {
        use shape_ast::ast::Expr;
        match arg {
            Expr::Identifier(name, span) => self
                .compile_reference_identifier(name, *span, mode)
                .map(|_| ()),
            Expr::PropertyAccess {
                object,
                property,
                optional: false,
                span,
            } => self
                .compile_reference_property_access(object, property, *span, mode)
                .map(|_| ()),
            Expr::IndexAccess {
                object,
                index,
                end_index: None,
                span,
            } => self
                .compile_reference_index_access(object, index, *span, mode)
                .map(|_| ()),
            _ => {
                self.compile_expr_preserving_refs(arg)?;
                if let Some(returned_mode) = self.last_expr_reference_mode() {
                    if mode == BorrowMode::Exclusive && returned_mode != BorrowMode::Exclusive {
                        return Err(ShapeError::SemanticError {
                            message:
                                "cannot pass a shared reference result to an exclusive parameter"
                                    .to_string(),
                            location: Some(self.span_to_source_location(arg.span())),
                        });
                    }
                    return Ok(());
                }
                if mode == BorrowMode::Exclusive {
                    return Err(ShapeError::SemanticError {
                        message:
                            "[B0004] mutable reference arguments must be simple variables or existing exclusive references"
                                .to_string(),
                        location: Some(self.span_to_source_location(arg.span())),
                    });
                }
                let temp = self.declare_temp_local("__arg_ref_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(temp)),
                ));
                // MIR analysis is the sole authority for borrow checking.
                self.emit(Instruction::new(
                    OpCode::MakeRef,
                    Some(Operand::Local(temp)),
                ));
                Ok(())
            }
        }
    }

    pub(super) fn compile_reference_identifier(
        &mut self,
        name: &str,
        span: shape_ast::ast::Span,
        mode: BorrowMode,
    ) -> Result<u32> {
        if let Some(local_idx) = self.resolve_local(name) {
            // Reject exclusive borrows of const variables
            if mode == BorrowMode::Exclusive && self.const_locals.contains(&local_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Cannot pass const variable '{}' by exclusive reference",
                        name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
            if self.ref_locals.contains(&local_idx) {
                // Forward an existing reference parameter by value (TAG_REF).
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(local_idx)),
                ));
                return Ok(u32::MAX);
            }
            if self.reference_value_locals.contains(&local_idx) {
                if mode == BorrowMode::Exclusive
                    && !self.exclusive_reference_value_locals.contains(&local_idx)
                {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "Cannot pass shared reference variable '{}' as an exclusive reference",
                            name
                        ),
                        location: Some(self.span_to_source_location(span)),
                    });
                }
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(local_idx)),
                ));
                return Ok(u32::MAX);
            }
            // MIR analysis is the sole authority for borrow checking.
            self.emit(Instruction::new(
                OpCode::MakeRef,
                Some(Operand::Local(local_idx)),
            ));
            Ok(u32::MAX)
        } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
            let Some(&binding_idx) = self.module_bindings.get(&scoped_name) else {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "[B0004] reference argument must be a local or module_binding variable, got '{}'",
                        name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            };
            // Reject exclusive borrows of const module bindings
            if mode == BorrowMode::Exclusive && self.const_module_bindings.contains(&binding_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Cannot pass const variable '{}' by exclusive reference",
                        name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
            if self.reference_value_module_bindings.contains(&binding_idx) {
                if mode == BorrowMode::Exclusive
                    && !self
                        .exclusive_reference_value_module_bindings
                        .contains(&binding_idx)
                {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "Cannot pass shared reference variable '{}' as an exclusive reference",
                            name
                        ),
                        location: Some(self.span_to_source_location(span)),
                    });
                }
                self.emit(Instruction::new(
                    OpCode::LoadModuleBinding,
                    Some(Operand::ModuleBinding(binding_idx)),
                ));
                return Ok(u32::MAX);
            }
            // MIR analysis is the sole authority for borrow checking.
            self.emit(Instruction::new(
                OpCode::MakeRef,
                Some(Operand::ModuleBinding(binding_idx)),
            ));
            Ok(u32::MAX)
        } else if let Some(func_idx) = self.find_function(name) {
            // Function name passed as reference argument: create a temporary local
            // with the function constant and make a reference to it.
            let temp = self.declare_temp_local("__fn_ref_")?;
            let const_idx = self
                .program
                .add_constant(Constant::Function(func_idx as u16));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(const_idx)),
            ));
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(temp)),
            ));
            // MIR analysis is the sole authority for borrow checking.
            self.emit(Instruction::new(
                OpCode::MakeRef,
                Some(Operand::Local(temp)),
            ));
            Ok(u32::MAX)
        } else {
            Err(ShapeError::SemanticError {
                message: format!(
                    "[B0004] reference argument must be a local or module_binding variable, got '{}'",
                    name
                ),
                location: Some(self.span_to_source_location(span)),
            })
        }
    }

    /// Push a new scope
    pub(super) fn push_scope(&mut self) {
        self.locals.push(HashMap::new());
        self.type_tracker.push_scope();
    }

    /// Pop a scope
    pub(super) fn pop_scope(&mut self) {
        self.locals.pop();
        self.type_tracker.pop_scope();
    }

    /// Declare a local variable
    pub(super) fn declare_local(&mut self, name: &str) -> Result<u16> {
        let idx = self.next_local;
        self.next_local += 1;

        if let Some(scope) = self.locals.last_mut() {
            scope.insert(name.to_string(), idx);
        }

        Ok(idx)
    }

    /// Resolve a local variable
    pub(super) fn resolve_local(&self, name: &str) -> Option<u16> {
        for scope in self.locals.iter().rev() {
            if let Some(&idx) = scope.get(name) {
                return Some(idx);
            }
        }
        None
    }

    /// Declare a temporary local variable
    pub(super) fn declare_temp_local(&mut self, prefix: &str) -> Result<u16> {
        let name = format!("{}{}", prefix, self.next_local);
        self.declare_local(&name)
    }

    /// Set type info for an existing local variable
    pub(super) fn set_local_type_info(&mut self, slot: u16, type_name: &str) {
        let info = if let Some(schema) = self.type_tracker.schema_registry().get(type_name) {
            VariableTypeInfo::known(schema.id, type_name.to_string())
        } else {
            VariableTypeInfo::named(type_name.to_string())
        };
        self.type_tracker.set_local_type(slot, info);
    }

    /// Set type info for a module_binding variable
    pub(super) fn set_module_binding_type_info(&mut self, slot: u16, type_name: &str) {
        let info = if let Some(schema) = self.type_tracker.schema_registry().get(type_name) {
            VariableTypeInfo::known(schema.id, type_name.to_string())
        } else {
            VariableTypeInfo::named(type_name.to_string())
        };
        self.type_tracker.set_binding_type(slot, info);
    }

    /// Capture local storage hints for a compiled function.
    ///
    /// Must be called before the function scope is popped so the type tracker still
    /// has local slot metadata. Also populates the function's `FrameDescriptor` so
    /// the verifier and executor can use per-slot type info for trusted opcodes.
    pub(super) fn capture_function_local_storage_hints(&mut self, func_idx: usize) {
        let Some(func) = self.program.functions.get(func_idx) else {
            return;
        };
        let hints: Vec<StorageHint> = (0..func.locals_count)
            .map(|slot| self.type_tracker.get_local_storage_hint(slot))
            .collect();

        // Populate FrameDescriptor on the function for trusted opcode verification.
        let has_any_known = hints.iter().any(|h| *h != StorageHint::Unknown);
        let instr_len = self.program.instructions.len();
        let code_end = if func.body_length > 0 {
            (func.entry_point + func.body_length).min(instr_len)
        } else {
            instr_len
        };
        let has_trusted = if func.entry_point <= code_end && code_end <= instr_len {
            self.program.instructions[func.entry_point..code_end]
                .iter()
                .any(|i| i.opcode.is_trusted())
        } else {
            false
        };
        if has_any_known || has_trusted {
            self.program.functions[func_idx].frame_descriptor = Some(
                crate::type_tracking::FrameDescriptor::from_slots(hints.clone()),
            );
        }

        if self.program.function_local_storage_hints.len() <= func_idx {
            self.program
                .function_local_storage_hints
                .resize(func_idx + 1, Vec::new());
        }
        self.program.function_local_storage_hints[func_idx] = hints;
    }

    /// Populate program-level storage hints for top-level locals and module bindings.
    pub(super) fn populate_program_storage_hints(&mut self) {
        let top_hints: Vec<StorageHint> = (0..self.next_local)
            .map(|slot| self.type_tracker.get_local_storage_hint(slot))
            .collect();
        self.program.top_level_local_storage_hints = top_hints.clone();

        // Build top-level FrameDescriptor so JIT can use per-slot type info
        let has_any_known = top_hints.iter().any(|h| *h != StorageHint::Unknown);
        let has_trusted = self
            .program
            .instructions
            .iter()
            .any(|i| i.opcode.is_trusted());
        if has_any_known || has_trusted {
            self.program.top_level_frame =
                Some(crate::type_tracking::FrameDescriptor::from_slots(top_hints));
        }

        let mut module_binding_hints = vec![StorageHint::Unknown; self.module_bindings.len()];
        for &idx in self.module_bindings.values() {
            if let Some(slot) = module_binding_hints.get_mut(idx as usize) {
                *slot = self.type_tracker.get_module_binding_storage_hint(idx);
            }
        }
        self.program.module_binding_storage_hints = module_binding_hints;

        if self.program.function_local_storage_hints.len() < self.program.functions.len() {
            self.program
                .function_local_storage_hints
                .resize(self.program.functions.len(), Vec::new());
        } else if self.program.function_local_storage_hints.len() > self.program.functions.len() {
            self.program
                .function_local_storage_hints
                .truncate(self.program.functions.len());
        }
    }

    /// Propagate the current expression's inferred type metadata to a target slot.
    ///
    /// Used by assignment sites to keep mutable locals/module_bindings typed when
    /// safe, and to clear stale hints when assigning unknown/dynamic values.
    pub(super) fn propagate_assignment_type_to_slot(
        &mut self,
        slot: u16,
        is_local: bool,
        allow_number_hint: bool,
    ) {
        if let Some(ref info) = self.last_expr_type_info {
            if info.is_indexed()
                || info.is_datatable()
                || info.schema_id.is_some()
                || Self::is_array_type_name(info.type_name.as_deref())
            {
                if is_local {
                    self.type_tracker.set_local_type(slot, info.clone());
                } else {
                    self.type_tracker.set_binding_type(slot, info.clone());
                }
                return;
            }
        }

        if let Some(schema_id) = self.last_expr_schema {
            let schema_name = self
                .type_tracker
                .schema_registry()
                .get_by_id(schema_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| format!("__anon_{}", schema_id));
            let info = VariableTypeInfo::known(schema_id, schema_name);
            if is_local {
                self.type_tracker.set_local_type(slot, info);
            } else {
                self.type_tracker.set_binding_type(slot, info);
            }
            return;
        }

        if let Some(numeric_type) = self.last_expr_numeric_type {
            let (type_name, hint) = match numeric_type {
                crate::type_tracking::NumericType::Int => ("int", StorageHint::Int64),
                crate::type_tracking::NumericType::IntWidth(w) => {
                    use shape_ast::IntWidth;
                    let hint = match w {
                        IntWidth::I8 => StorageHint::Int8,
                        IntWidth::U8 => StorageHint::UInt8,
                        IntWidth::I16 => StorageHint::Int16,
                        IntWidth::U16 => StorageHint::UInt16,
                        IntWidth::I32 => StorageHint::Int32,
                        IntWidth::U32 => StorageHint::UInt32,
                        IntWidth::U64 => StorageHint::UInt64,
                    };
                    (w.type_name(), hint)
                }
                crate::type_tracking::NumericType::Number => {
                    if !allow_number_hint {
                        if is_local {
                            self.type_tracker
                                .set_local_type(slot, VariableTypeInfo::unknown());
                        } else {
                            self.type_tracker
                                .set_binding_type(slot, VariableTypeInfo::unknown());
                        }
                        return;
                    }
                    ("number", StorageHint::Float64)
                }
                // Decimal typed opcodes are not JIT-compiled yet.
                crate::type_tracking::NumericType::Decimal => {
                    if is_local {
                        self.type_tracker
                            .set_local_type(slot, VariableTypeInfo::unknown());
                    } else {
                        self.type_tracker
                            .set_binding_type(slot, VariableTypeInfo::unknown());
                    }
                    return;
                }
            };
            let info = VariableTypeInfo::with_storage(type_name.to_string(), hint);
            if is_local {
                self.type_tracker.set_local_type(slot, info);
            } else {
                self.type_tracker.set_binding_type(slot, info);
            }
            return;
        }

        // Assignment to an unknown/dynamic expression invalidates prior hints.
        if is_local {
            self.type_tracker
                .set_local_type(slot, VariableTypeInfo::unknown());
        } else {
            self.type_tracker
                .set_binding_type(slot, VariableTypeInfo::unknown());
        }
    }

    /// Propagate current expression type metadata to an identifier target.
    ///
    /// Reference locals are skipped because assignment writes through to a pointee.
    pub(super) fn propagate_assignment_type_to_identifier(&mut self, name: &str) {
        if let Some(local_idx) = self.resolve_local(name) {
            if self.local_binding_is_reference_value(local_idx) {
                return;
            }
            self.propagate_assignment_type_to_slot(local_idx, true, true);
            return;
        }

        let scoped_name = self
            .resolve_scoped_module_binding_name(name)
            .unwrap_or_else(|| name.to_string());
        let binding_idx = self.get_or_create_module_binding(&scoped_name);
        self.propagate_assignment_type_to_slot(binding_idx, false, true);
    }

    /// Get the type tracker (for external configuration)
    pub fn type_tracker(&self) -> &TypeTracker {
        &self.type_tracker
    }

    /// Get mutable type tracker (for registering types)
    pub fn type_tracker_mut(&mut self) -> &mut TypeTracker {
        &mut self.type_tracker
    }

    /// Resolve a column name to its index using the data schema.
    /// Returns an error if no schema is provided or the column doesn't exist.
    pub(super) fn resolve_column_index(&self, field: &str) -> Result<u32> {
        self.program
            .data_schema
            .as_ref()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "No data schema provided. Cannot resolve field '{}'. \
                     Hint: Use stdlib/finance to load market data with OHLCV schema.",
                    field
                ),
                location: None,
            })?
            .get_index(field)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "Unknown column '{}' in data schema. Available columns: {:?}",
                    field,
                    self.program
                        .data_schema
                        .as_ref()
                        .map(|s| &s.column_names)
                        .unwrap_or(&vec![])
                ),
                location: None,
            })
    }

    /// Check if a field name is a known data column in the schema.
    pub(super) fn is_data_column(&self, field: &str) -> bool {
        self.program
            .data_schema
            .as_ref()
            .map(|s| s.get_index(field).is_some())
            .unwrap_or(false)
    }

    /// Collect all outer scope variables
    pub(super) fn collect_outer_scope_vars(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        for scope in &self.locals {
            for name in scope.keys() {
                names.insert(name.clone());
            }
        }
        for name in self.module_bindings.keys() {
            names.insert(name.clone());
        }
        names.into_iter().collect()
    }

    /// Get or create a module_binding variable
    pub(super) fn get_or_create_module_binding(&mut self, name: &str) -> u16 {
        if let Some(&idx) = self.module_bindings.get(name) {
            idx
        } else {
            let idx = self.next_global;
            self.next_global += 1;
            self.module_bindings.insert(name.to_string(), idx);
            idx
        }
    }

    pub(super) fn resolve_scoped_module_binding_name(&self, name: &str) -> Option<String> {
        if crate::module_resolution::is_hidden_annotation_import_module_name(name) {
            return None;
        }
        if self.module_bindings.contains_key(name) {
            return Some(name.to_string());
        }
        for module_path in self.module_scope_stack.iter().rev() {
            let candidate = format!("{}::{}", module_path, name);
            if self.module_bindings.contains_key(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    pub(super) fn resolve_scoped_function_name(&self, name: &str) -> Option<String> {
        if self.program.functions.iter().any(|f| f.name == name) {
            return Some(name.to_string());
        }
        for module_path in self.module_scope_stack.iter().rev() {
            let candidate = format!("{}::{}", module_path, name);
            if self.program.functions.iter().any(|f| f.name == candidate) {
                return Some(candidate);
            }
        }
        None
    }

    /// Find a function by name
    pub(super) fn find_function(&self, name: &str) -> Option<usize> {
        // Check function aliases first (e.g., __original__ -> shadow function).
        if let Some(actual_name) = self.function_aliases.get(name) {
            if let Some(idx) = self
                .program
                .functions
                .iter()
                .position(|f| f.name == *actual_name)
            {
                return Some(idx);
            }
        }

        // Try direct/scoped resolution
        if let Some(resolved) = self.resolve_scoped_function_name(name) {
            if let Some(idx) = self
                .program
                .functions
                .iter()
                .position(|f| f.name == resolved)
            {
                return Some(idx);
            }
        }

        // If direct lookup failed, check imported_names for alias -> original name mapping.
        // When a function is imported with an alias (e.g., `use { foo as bar } from "module"`),
        // the function is registered under its original (possibly module-qualified) name,
        // but the user refers to it by the alias.
        if let Some(imported) = self.imported_names.get(name) {
            let original = &imported.original_name;
            // Try direct match on the original name
            if let Some(idx) = self
                .program
                .functions
                .iter()
                .position(|f| f.name == *original)
            {
                return Some(idx);
            }
            // Try scoped resolution on the original name
            if let Some(resolved) = self.resolve_scoped_function_name(original) {
                if let Some(idx) = self
                    .program
                    .functions
                    .iter()
                    .position(|f| f.name == resolved)
                {
                    return Some(idx);
                }
            }
        }

        None
    }

    /// Resolve the receiver's type name for extend method dispatch.
    ///
    /// Determines the Shape type name from all available compiler state:
    /// - `last_expr_type_info.type_name` for TypedObjects (e.g., "Point", "Candle")
    /// - `last_expr_numeric_type` for numeric types → "Int", "Number", "Decimal"
    /// - Receiver expression analysis for arrays, strings, booleans
    ///
    /// Returns the base type name (e.g., "Vec" not "Vec<int>") suitable for
    /// extend method lookup as "Type.method".
    pub(super) fn resolve_receiver_extend_type(
        &self,
        receiver: &shape_ast::ast::Expr,
        receiver_type_info: &Option<crate::type_tracking::VariableTypeInfo>,
        _receiver_schema: Option<u32>,
    ) -> Option<String> {
        // 1. Numeric type from typed opcode tracking — checked first because
        //    the type tracker stores lowercase names ("int", "number") while
        //    extend blocks use capitalized TypeName ("Int", "Number", "Decimal").
        if let Some(numeric) = self.last_expr_numeric_type {
            return Some(
                match numeric {
                    crate::type_tracking::NumericType::Int
                    | crate::type_tracking::NumericType::IntWidth(_) => "Int",
                    crate::type_tracking::NumericType::Number => "Number",
                    crate::type_tracking::NumericType::Decimal => "Decimal",
                }
                .to_string(),
            );
        }

        // 2. TypedObject type name (user-defined types like Point, Candle)
        if let Some(info) = receiver_type_info {
            if let Some(type_name) = &info.type_name {
                // Strip generic params: "Vec<int>" → "Vec"
                let base = type_name.split('<').next().unwrap_or(type_name);
                return Some(base.to_string());
            }
        }

        // 3. Infer from receiver expression shape
        match receiver {
            shape_ast::ast::Expr::Literal(lit, _) => match lit {
                shape_ast::ast::Literal::String(_)
                | shape_ast::ast::Literal::FormattedString { .. }
                | shape_ast::ast::Literal::ContentString { .. } => Some("String".to_string()),
                shape_ast::ast::Literal::Bool(_) => Some("Bool".to_string()),
                _ => None,
            },
            shape_ast::ast::Expr::Array(..) => Some("Vec".to_string()),
            _ => None,
        }
    }

    /// Emit store instruction for an identifier
    pub(super) fn emit_store_identifier(&mut self, name: &str) -> Result<()> {
        // Mutable closure captures: emit StoreClosure to write to the shared upvalue
        if let Some(&upvalue_idx) = self.mutable_closure_captures.get(name) {
            self.emit(Instruction::new(
                OpCode::StoreClosure,
                Some(Operand::Local(upvalue_idx)),
            ));
            return Ok(());
        }
        if let Some(local_idx) = self.resolve_local(name) {
            if self.local_binding_is_reference_value(local_idx) {
                if !self.local_reference_binding_is_exclusive(local_idx) {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "cannot assign through shared reference variable '{}'",
                            name
                        ),
                        location: None,
                    });
                }
                self.emit(Instruction::new(
                    OpCode::DerefStore,
                    Some(Operand::Local(local_idx)),
                ));
            } else {
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(local_idx)),
                ));
                // Patch StoreLocal → StoreLocalTyped for width-typed locals
                if let Some(type_name) = self
                    .type_tracker
                    .get_local_type(local_idx)
                    .and_then(|info| info.type_name.as_deref())
                {
                    if let Some(w) = shape_ast::IntWidth::from_name(type_name) {
                        if let Some(last) = self.program.instructions.last_mut() {
                            if last.opcode == OpCode::StoreLocal {
                                last.opcode = OpCode::StoreLocalTyped;
                                last.operand = Some(Operand::TypedLocal(
                                    local_idx,
                                    crate::bytecode::NumericWidth::from_int_width(w),
                                ));
                            }
                        }
                    }
                }
            }
        } else {
            let scoped_name = self
                .resolve_scoped_module_binding_name(name)
                .unwrap_or_else(|| name.to_string());
            let binding_idx = self.get_or_create_module_binding(&scoped_name);
            self.emit(Instruction::new(
                OpCode::StoreModuleBinding,
                Some(Operand::ModuleBinding(binding_idx)),
            ));
            // Patch StoreModuleBinding → StoreModuleBindingTyped for width-typed bindings
            if let Some(type_name) = self
                .type_tracker
                .get_binding_type(binding_idx)
                .and_then(|info| info.type_name.as_deref())
            {
                if let Some(w) = shape_ast::IntWidth::from_name(type_name) {
                    if let Some(last) = self.program.instructions.last_mut() {
                        if last.opcode == OpCode::StoreModuleBinding {
                            last.opcode = OpCode::StoreModuleBindingTyped;
                            last.operand = Some(Operand::TypedModuleBinding(
                                binding_idx,
                                crate::bytecode::NumericWidth::from_int_width(w),
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn classify_builtin_function(&self, name: &str) -> Option<BuiltinNameResolution> {
        let builtin = match name {
            // Option type constructor
            "Some" => BuiltinFunction::SomeCtor,
            "Ok" => BuiltinFunction::OkCtor,
            "Err" => BuiltinFunction::ErrCtor,
            "HashMap" => BuiltinFunction::HashMapCtor,
            "Set" => BuiltinFunction::SetCtor,
            "Deque" => BuiltinFunction::DequeCtor,
            "PriorityQueue" => BuiltinFunction::PriorityQueueCtor,
            "Mutex" => BuiltinFunction::MutexCtor,
            "Atomic" => BuiltinFunction::AtomicCtor,
            "Lazy" => BuiltinFunction::LazyCtor,
            "Channel" => BuiltinFunction::ChannelCtor,
            // Json navigation helpers
            "__json_object_get" => BuiltinFunction::JsonObjectGet,
            "__json_array_at" => BuiltinFunction::JsonArrayAt,
            "__json_object_keys" => BuiltinFunction::JsonObjectKeys,
            "__json_array_len" => BuiltinFunction::JsonArrayLen,
            "__json_object_len" => BuiltinFunction::JsonObjectLen,
            "__intrinsic_vec_abs" => BuiltinFunction::IntrinsicVecAbs,
            "__intrinsic_vec_sqrt" => BuiltinFunction::IntrinsicVecSqrt,
            "__intrinsic_vec_ln" => BuiltinFunction::IntrinsicVecLn,
            "__intrinsic_vec_exp" => BuiltinFunction::IntrinsicVecExp,
            "__intrinsic_vec_add" => BuiltinFunction::IntrinsicVecAdd,
            "__intrinsic_vec_sub" => BuiltinFunction::IntrinsicVecSub,
            "__intrinsic_vec_mul" => BuiltinFunction::IntrinsicVecMul,
            "__intrinsic_vec_div" => BuiltinFunction::IntrinsicVecDiv,
            "__intrinsic_vec_max" => BuiltinFunction::IntrinsicVecMax,
            "__intrinsic_vec_min" => BuiltinFunction::IntrinsicVecMin,
            "__intrinsic_vec_select" => BuiltinFunction::IntrinsicVecSelect,
            "__intrinsic_matmul_vec" => BuiltinFunction::IntrinsicMatMulVec,
            "__intrinsic_matmul_mat" => BuiltinFunction::IntrinsicMatMulMat,

            // Existing builtins
            "abs" => BuiltinFunction::Abs,
            "min" => BuiltinFunction::Min,
            "max" => BuiltinFunction::Max,
            "sqrt" => BuiltinFunction::Sqrt,
            "ln" => BuiltinFunction::Ln,
            "pow" => BuiltinFunction::Pow,
            "exp" => BuiltinFunction::Exp,
            "log" => BuiltinFunction::Log,
            "floor" => BuiltinFunction::Floor,
            "ceil" => BuiltinFunction::Ceil,
            "round" => BuiltinFunction::Round,
            "sin" => BuiltinFunction::Sin,
            "cos" => BuiltinFunction::Cos,
            "tan" => BuiltinFunction::Tan,
            "asin" => BuiltinFunction::Asin,
            "acos" => BuiltinFunction::Acos,
            "atan" => BuiltinFunction::Atan,
            "stddev" => BuiltinFunction::StdDev,
            "__intrinsic_map" => BuiltinFunction::Map,
            "__intrinsic_filter" => BuiltinFunction::Filter,
            "__intrinsic_reduce" => BuiltinFunction::Reduce,
            "print" => BuiltinFunction::Print,
            "format" => BuiltinFunction::Format,
            "len" | "count" => BuiltinFunction::Len,
            // "throw" removed: Shape uses Result types
            "__intrinsic_snapshot" | "snapshot" => BuiltinFunction::Snapshot,
            "exit" => BuiltinFunction::Exit,
            "range" => BuiltinFunction::Range,
            "is_number" | "isNumber" => BuiltinFunction::IsNumber,
            "is_string" | "isString" => BuiltinFunction::IsString,
            "is_bool" | "isBool" => BuiltinFunction::IsBool,
            "is_array" | "isArray" => BuiltinFunction::IsArray,
            "is_object" | "isObject" => BuiltinFunction::IsObject,
            "is_data_row" | "isDataRow" => BuiltinFunction::IsDataRow,
            "to_string" | "toString" => BuiltinFunction::ToString,
            "to_number" | "toNumber" => BuiltinFunction::ToNumber,
            "to_bool" | "toBool" => BuiltinFunction::ToBool,
            "__into_int" => BuiltinFunction::IntoInt,
            "__into_number" => BuiltinFunction::IntoNumber,
            "__into_decimal" => BuiltinFunction::IntoDecimal,
            "__into_bool" => BuiltinFunction::IntoBool,
            "__into_string" => BuiltinFunction::IntoString,
            "__try_into_int" => BuiltinFunction::TryIntoInt,
            "__try_into_number" => BuiltinFunction::TryIntoNumber,
            "__try_into_decimal" => BuiltinFunction::TryIntoDecimal,
            "__try_into_bool" => BuiltinFunction::TryIntoBool,
            "__try_into_string" => BuiltinFunction::TryIntoString,
            "__native_ptr_size" => BuiltinFunction::NativePtrSize,
            "__native_ptr_new_cell" => BuiltinFunction::NativePtrNewCell,
            "__native_ptr_free_cell" => BuiltinFunction::NativePtrFreeCell,
            "__native_ptr_read_ptr" => BuiltinFunction::NativePtrReadPtr,
            "__native_ptr_write_ptr" => BuiltinFunction::NativePtrWritePtr,
            "__native_table_from_arrow_c" => BuiltinFunction::NativeTableFromArrowC,
            "__native_table_from_arrow_c_typed" => BuiltinFunction::NativeTableFromArrowCTyped,
            "__native_table_bind_type" => BuiltinFunction::NativeTableBindType,
            "fold" => BuiltinFunction::ControlFold,

            // Math intrinsics
            "__intrinsic_sum" => BuiltinFunction::IntrinsicSum,
            "__intrinsic_mean" => BuiltinFunction::IntrinsicMean,
            "__intrinsic_min" => BuiltinFunction::IntrinsicMin,
            "__intrinsic_max" => BuiltinFunction::IntrinsicMax,
            "__intrinsic_std" => BuiltinFunction::IntrinsicStd,
            "__intrinsic_variance" => BuiltinFunction::IntrinsicVariance,

            // Random intrinsics
            "__intrinsic_random" => BuiltinFunction::IntrinsicRandom,
            "__intrinsic_random_int" => BuiltinFunction::IntrinsicRandomInt,
            "__intrinsic_random_seed" => BuiltinFunction::IntrinsicRandomSeed,
            "__intrinsic_random_normal" => BuiltinFunction::IntrinsicRandomNormal,
            "__intrinsic_random_array" => BuiltinFunction::IntrinsicRandomArray,

            // Distribution intrinsics
            "__intrinsic_dist_uniform" => BuiltinFunction::IntrinsicDistUniform,
            "__intrinsic_dist_lognormal" => BuiltinFunction::IntrinsicDistLognormal,
            "__intrinsic_dist_exponential" => BuiltinFunction::IntrinsicDistExponential,
            "__intrinsic_dist_poisson" => BuiltinFunction::IntrinsicDistPoisson,
            "__intrinsic_dist_sample_n" => BuiltinFunction::IntrinsicDistSampleN,

            // Stochastic process intrinsics
            "__intrinsic_brownian_motion" => BuiltinFunction::IntrinsicBrownianMotion,
            "__intrinsic_gbm" => BuiltinFunction::IntrinsicGbm,
            "__intrinsic_ou_process" => BuiltinFunction::IntrinsicOuProcess,
            "__intrinsic_random_walk" => BuiltinFunction::IntrinsicRandomWalk,

            // Rolling intrinsics
            "__intrinsic_rolling_sum" => BuiltinFunction::IntrinsicRollingSum,
            "__intrinsic_rolling_mean" => BuiltinFunction::IntrinsicRollingMean,
            "__intrinsic_rolling_std" => BuiltinFunction::IntrinsicRollingStd,
            "__intrinsic_rolling_min" => BuiltinFunction::IntrinsicRollingMin,
            "__intrinsic_rolling_max" => BuiltinFunction::IntrinsicRollingMax,
            "__intrinsic_ema" => BuiltinFunction::IntrinsicEma,
            "__intrinsic_linear_recurrence" => BuiltinFunction::IntrinsicLinearRecurrence,

            // Series intrinsics
            "__intrinsic_shift" => BuiltinFunction::IntrinsicShift,
            "__intrinsic_diff" => BuiltinFunction::IntrinsicDiff,
            "__intrinsic_pct_change" => BuiltinFunction::IntrinsicPctChange,
            "__intrinsic_fillna" => BuiltinFunction::IntrinsicFillna,
            "__intrinsic_cumsum" => BuiltinFunction::IntrinsicCumsum,
            "__intrinsic_cumprod" => BuiltinFunction::IntrinsicCumprod,
            "__intrinsic_clip" => BuiltinFunction::IntrinsicClip,

            // Trigonometric intrinsics (map __intrinsic_ forms to existing builtins)
            "__intrinsic_sin" => BuiltinFunction::Sin,
            "__intrinsic_cos" => BuiltinFunction::Cos,
            "__intrinsic_tan" => BuiltinFunction::Tan,
            "__intrinsic_asin" => BuiltinFunction::Asin,
            "__intrinsic_acos" => BuiltinFunction::Acos,
            "__intrinsic_atan" => BuiltinFunction::Atan,
            "__intrinsic_atan2" => BuiltinFunction::IntrinsicAtan2,
            "__intrinsic_sinh" => BuiltinFunction::IntrinsicSinh,
            "__intrinsic_cosh" => BuiltinFunction::IntrinsicCosh,
            "__intrinsic_tanh" => BuiltinFunction::IntrinsicTanh,

            // Statistical intrinsics
            "__intrinsic_correlation" => BuiltinFunction::IntrinsicCorrelation,
            "__intrinsic_covariance" => BuiltinFunction::IntrinsicCovariance,
            "__intrinsic_percentile" => BuiltinFunction::IntrinsicPercentile,
            "__intrinsic_median" => BuiltinFunction::IntrinsicMedian,

            // Character code intrinsics
            "__intrinsic_char_code" => BuiltinFunction::IntrinsicCharCode,
            "__intrinsic_from_char_code" => BuiltinFunction::IntrinsicFromCharCode,

            // Series access
            "__intrinsic_series" => BuiltinFunction::IntrinsicSeries,

            // Reflection
            "reflect" => BuiltinFunction::Reflect,

            // Additional math builtins
            "sign" => BuiltinFunction::Sign,
            "gcd" => BuiltinFunction::Gcd,
            "lcm" => BuiltinFunction::Lcm,
            "hypot" => BuiltinFunction::Hypot,
            "clamp" => BuiltinFunction::Clamp,
            "isNaN" | "is_nan" => BuiltinFunction::IsNaN,
            "isFinite" | "is_finite" => BuiltinFunction::IsFinite,
            _ => return None,
        };

        let scope = match name {
            "Some" | "Ok" | "Err" => ResolutionScope::TypeAssociated,
            "print" => ResolutionScope::Prelude,
            _ if Self::is_internal_intrinsic_name(name) => ResolutionScope::InternalIntrinsic,
            _ => ResolutionScope::ModuleBinding,
        };

        Some(match scope {
            ResolutionScope::InternalIntrinsic => {
                BuiltinNameResolution::InternalOnly { builtin, scope }
            }
            _ => BuiltinNameResolution::Surface { builtin, scope },
        })
    }

    pub(super) fn is_internal_intrinsic_name(name: &str) -> bool {
        name.starts_with("__native_")
            || name.starts_with("__intrinsic_")
            || name.starts_with("__json_")
    }

    pub(super) const fn variable_scope_summary() -> &'static str {
        "Variable names resolve from local scope and module scope."
    }

    pub(super) const fn function_scope_summary() -> &'static str {
        "Function names resolve from module scope, explicit imports, type-associated scope, and the implicit prelude."
    }

    pub(super) fn undefined_variable_message(&self, name: &str) -> String {
        format!(
            "Undefined variable: {}. {}",
            name,
            Self::variable_scope_summary()
        )
    }

    pub(super) fn undefined_function_message(&self, name: &str) -> String {
        format!(
            "Undefined function: {}. {}",
            name,
            Self::function_scope_summary()
        )
    }

    pub(super) fn internal_intrinsic_error_message(
        &self,
        name: &str,
        resolution: BuiltinNameResolution,
    ) -> String {
        format!(
            "'{}' resolves to {} and is not available from ordinary user code. Internal intrinsics are reserved for std::* implementations and compiler-generated code.",
            name,
            resolution.scope().label()
        )
    }

    /// Check if a builtin function requires arg count
    pub(super) fn builtin_requires_arg_count(&self, builtin: BuiltinFunction) -> bool {
        matches!(
            builtin,
            BuiltinFunction::Abs
                | BuiltinFunction::Min
                | BuiltinFunction::Max
                | BuiltinFunction::Sqrt
                | BuiltinFunction::Ln
                | BuiltinFunction::Pow
                | BuiltinFunction::Exp
                | BuiltinFunction::Log
                | BuiltinFunction::Floor
                | BuiltinFunction::Ceil
                | BuiltinFunction::Round
                | BuiltinFunction::Sin
                | BuiltinFunction::Cos
                | BuiltinFunction::Tan
                | BuiltinFunction::Asin
                | BuiltinFunction::Acos
                | BuiltinFunction::Atan
                | BuiltinFunction::StdDev
                | BuiltinFunction::Range
                | BuiltinFunction::Slice
                | BuiltinFunction::Push
                | BuiltinFunction::Pop
                | BuiltinFunction::First
                | BuiltinFunction::Last
                | BuiltinFunction::Zip
                | BuiltinFunction::Map
                | BuiltinFunction::Filter
                | BuiltinFunction::Reduce
                | BuiltinFunction::ForEach
                | BuiltinFunction::Find
                | BuiltinFunction::FindIndex
                | BuiltinFunction::Some
                | BuiltinFunction::Every
                | BuiltinFunction::SomeCtor
                | BuiltinFunction::OkCtor
                | BuiltinFunction::ErrCtor
                | BuiltinFunction::HashMapCtor
                | BuiltinFunction::SetCtor
                | BuiltinFunction::DequeCtor
                | BuiltinFunction::PriorityQueueCtor
                | BuiltinFunction::MutexCtor
                | BuiltinFunction::AtomicCtor
                | BuiltinFunction::LazyCtor
                | BuiltinFunction::ChannelCtor
                | BuiltinFunction::Print
                | BuiltinFunction::Format
                | BuiltinFunction::Len
                // BuiltinFunction::Throw removed
                | BuiltinFunction::Snapshot
                | BuiltinFunction::ObjectRest
                | BuiltinFunction::IsNumber
                | BuiltinFunction::IsString
                | BuiltinFunction::IsBool
                | BuiltinFunction::IsArray
                | BuiltinFunction::IsObject
                | BuiltinFunction::IsDataRow
                | BuiltinFunction::ToString
                | BuiltinFunction::ToNumber
                | BuiltinFunction::ToBool
                | BuiltinFunction::IntoInt
                | BuiltinFunction::IntoNumber
                | BuiltinFunction::IntoDecimal
                | BuiltinFunction::IntoBool
                | BuiltinFunction::IntoString
                | BuiltinFunction::TryIntoInt
                | BuiltinFunction::TryIntoNumber
                | BuiltinFunction::TryIntoDecimal
                | BuiltinFunction::TryIntoBool
                | BuiltinFunction::TryIntoString
                | BuiltinFunction::NativePtrSize
                | BuiltinFunction::NativePtrNewCell
                | BuiltinFunction::NativePtrFreeCell
                | BuiltinFunction::NativePtrReadPtr
                | BuiltinFunction::NativePtrWritePtr
                | BuiltinFunction::NativeTableFromArrowC
                | BuiltinFunction::NativeTableFromArrowCTyped
                | BuiltinFunction::NativeTableBindType
                | BuiltinFunction::ControlFold
                | BuiltinFunction::IntrinsicSum
                | BuiltinFunction::IntrinsicMean
                | BuiltinFunction::IntrinsicMin
                | BuiltinFunction::IntrinsicMax
                | BuiltinFunction::IntrinsicStd
                | BuiltinFunction::IntrinsicVariance
                | BuiltinFunction::IntrinsicRandom
                | BuiltinFunction::IntrinsicRandomInt
                | BuiltinFunction::IntrinsicRandomSeed
                | BuiltinFunction::IntrinsicRandomNormal
                | BuiltinFunction::IntrinsicRandomArray
                | BuiltinFunction::IntrinsicDistUniform
                | BuiltinFunction::IntrinsicDistLognormal
                | BuiltinFunction::IntrinsicDistExponential
                | BuiltinFunction::IntrinsicDistPoisson
                | BuiltinFunction::IntrinsicDistSampleN
                | BuiltinFunction::IntrinsicBrownianMotion
                | BuiltinFunction::IntrinsicGbm
                | BuiltinFunction::IntrinsicOuProcess
                | BuiltinFunction::IntrinsicRandomWalk
                | BuiltinFunction::IntrinsicRollingSum
                | BuiltinFunction::IntrinsicRollingMean
                | BuiltinFunction::IntrinsicRollingStd
                | BuiltinFunction::IntrinsicRollingMin
                | BuiltinFunction::IntrinsicRollingMax
                | BuiltinFunction::IntrinsicEma
                | BuiltinFunction::IntrinsicLinearRecurrence
                | BuiltinFunction::IntrinsicShift
                | BuiltinFunction::IntrinsicDiff
                | BuiltinFunction::IntrinsicPctChange
                | BuiltinFunction::IntrinsicFillna
                | BuiltinFunction::IntrinsicCumsum
                | BuiltinFunction::IntrinsicCumprod
                | BuiltinFunction::IntrinsicClip
                | BuiltinFunction::IntrinsicCorrelation
                | BuiltinFunction::IntrinsicCovariance
                | BuiltinFunction::IntrinsicPercentile
                | BuiltinFunction::IntrinsicMedian
                | BuiltinFunction::IntrinsicAtan2
                | BuiltinFunction::IntrinsicSinh
                | BuiltinFunction::IntrinsicCosh
                | BuiltinFunction::IntrinsicTanh
                | BuiltinFunction::IntrinsicCharCode
                | BuiltinFunction::IntrinsicFromCharCode
                | BuiltinFunction::IntrinsicSeries
                | BuiltinFunction::IntrinsicVecAbs
                | BuiltinFunction::IntrinsicVecSqrt
                | BuiltinFunction::IntrinsicVecLn
                | BuiltinFunction::IntrinsicVecExp
                | BuiltinFunction::IntrinsicVecAdd
                | BuiltinFunction::IntrinsicVecSub
                | BuiltinFunction::IntrinsicVecMul
                | BuiltinFunction::IntrinsicVecDiv
                | BuiltinFunction::IntrinsicVecMax
                | BuiltinFunction::IntrinsicVecMin
                | BuiltinFunction::IntrinsicVecSelect
                | BuiltinFunction::IntrinsicMatMulVec
                | BuiltinFunction::IntrinsicMatMulMat
                | BuiltinFunction::Sign
                | BuiltinFunction::Gcd
                | BuiltinFunction::Lcm
                | BuiltinFunction::Hypot
                | BuiltinFunction::Clamp
                | BuiltinFunction::IsNaN
                | BuiltinFunction::IsFinite
        )
    }

    /// Check if any compiled function exists whose name indicates a user-defined
    /// override of the given method name (via extend blocks or impl blocks).
    ///
    /// Looks for function names like `Type.method` or `Type::method`.
    pub(super) fn has_any_user_defined_method(&self, method: &str) -> bool {
        let dot_suffix = format!(".{}", method);
        let colon_suffix = format!("::{}", method);
        self.program
            .functions
            .iter()
            .any(|f| f.name.ends_with(&dot_suffix) || f.name.ends_with(&colon_suffix))
    }

    /// Check if a method name is a known built-in method on any VM type.
    /// Used by UFCS to determine if `receiver.method(args)` should be dispatched
    /// as a built-in method call or rewritten to `method(receiver, args)`.
    pub(super) fn is_known_builtin_method(method: &str) -> bool {
        // Array methods (from ARRAY_METHODS PHF map)
        matches!(method,
            "map" | "filter" | "reduce" | "forEach" | "find" | "findIndex"
            | "some" | "every" | "sort" | "groupBy" | "flatMap"
            | "len" | "length" | "first" | "last" | "reverse" | "slice"
            | "concat" | "take" | "drop" | "skip"
            | "indexOf" | "includes"
            | "join" | "flatten" | "unique" | "distinct" | "distinctBy"
            | "sum" | "avg" | "min" | "max" | "count"
            | "where" | "select" | "orderBy" | "thenBy" | "takeWhile"
            | "skipWhile" | "single" | "any" | "all"
            | "innerJoin" | "leftJoin" | "crossJoin"
            | "union" | "intersect" | "except"
        )
        // DataTable methods (from DATATABLE_METHODS PHF map)
        || matches!(method,
            "columns" | "column" | "head" | "tail" | "mean" | "std"
            | "describe" | "aggregate" | "group_by" | "index_by" | "indexBy"
            | "simulate" | "toMat" | "to_mat"
        )
        // Column methods (from COLUMN_METHODS PHF map)
        || matches!(method, "toArray")
        // IndexedTable methods (from INDEXED_TABLE_METHODS PHF map)
        || matches!(method, "resample" | "between")
        // Number methods handled inline in op_call_method
        || matches!(method,
            "toFixed" | "toInt" | "toNumber" | "to_number" | "floor" | "ceil" | "round"
            | "abs" | "sign" | "clamp"
        )
        // String methods handled inline
        || matches!(method,
            "toUpperCase" | "toLowerCase" | "trim" | "contains" | "startsWith"
            | "endsWith" | "split" | "replace" | "substring" | "charAt"
            | "padStart" | "padEnd" | "repeat" | "toString"
        )
        // Object methods handled by handle_object_method
        || matches!(method, "keys" | "values" | "has" | "get" | "set" | "len")
        // DateTime methods (from DATETIME_METHODS PHF map)
        || matches!(method, "format")
        // Universal intrinsic methods
        || matches!(method, "type")
    }

    /// Try to track a `Table<T>` type annotation as a DataTable variable.
    ///
    /// If the annotation is `Generic { name: "Table", args: [Reference(T)] }`,
    /// looks up T's schema and marks the variable as `is_datatable`.
    pub(super) fn try_track_datatable_type(
        &mut self,
        type_ann: &shape_ast::ast::TypeAnnotation,
        slot: u16,
        is_local: bool,
    ) -> shape_ast::error::Result<()> {
        use shape_ast::ast::TypeAnnotation;
        if let TypeAnnotation::Generic { name, args } = type_ann {
            if name == "Table" && args.len() == 1 {
                let inner_name = match &args[0] {
                    TypeAnnotation::Reference(t) => Some(t.as_str()),
                    TypeAnnotation::Basic(t) => Some(t.as_str()),
                    _ => None,
                };
                if let Some(type_name) = inner_name {
                    let schema_id = self
                        .type_tracker
                        .schema_registry()
                        .get(type_name)
                        .map(|s| s.id);
                    if let Some(sid) = schema_id {
                        let info = crate::type_tracking::VariableTypeInfo::datatable(
                            sid,
                            type_name.to_string(),
                        );
                        if is_local {
                            self.type_tracker.set_local_type(slot, info);
                        } else {
                            self.type_tracker.set_binding_type(slot, info);
                        }
                    } else {
                        return Err(shape_ast::error::ShapeError::SemanticError {
                            message: format!(
                                "Unknown type '{}' in Table<{}> annotation",
                                type_name, type_name
                            ),
                            location: None,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Check if a variable is a RowView (typed row from Arrow DataTable).
    pub(super) fn is_row_view_variable(&self, name: &str) -> bool {
        if let Some(local_idx) = self.resolve_local(name) {
            if let Some(info) = self.type_tracker.get_local_type(local_idx) {
                return info.is_row_view();
            }
        }
        if let Some(&binding_idx) = self.module_bindings.get(name) {
            if let Some(info) = self.type_tracker.get_binding_type(binding_idx) {
                return info.is_row_view();
            }
        }
        false
    }

    /// Get the available field names for a RowView variable's schema.
    pub(super) fn get_row_view_field_names(&self, name: &str) -> Option<Vec<String>> {
        let type_name = if let Some(local_idx) = self.resolve_local(name) {
            self.type_tracker
                .get_local_type(local_idx)
                .and_then(|info| {
                    if info.is_row_view() {
                        info.type_name.clone()
                    } else {
                        None
                    }
                })
        } else if let Some(&binding_idx) = self.module_bindings.get(name) {
            self.type_tracker
                .get_binding_type(binding_idx)
                .and_then(|info| {
                    if info.is_row_view() {
                        info.type_name.clone()
                    } else {
                        None
                    }
                })
        } else {
            None
        };

        if let Some(tn) = type_name {
            if let Some(schema) = self.type_tracker.schema_registry().get(&tn) {
                return Some(schema.field_names().map(|n| n.to_string()).collect());
            }
        }
        None
    }

    /// Try to resolve a property access on a RowView variable to a column ID.
    ///
    /// Returns `Some(col_id)` if the variable is a tracked RowView and the field
    /// exists in its schema. Returns `None` if the variable isn't a RowView or
    /// the field is unknown (caller should emit a compile-time error).
    pub(super) fn try_resolve_row_view_column(
        &self,
        var_name: &str,
        field_name: &str,
    ) -> Option<u32> {
        // Check locals first, then module_bindings
        if let Some(local_idx) = self.resolve_local(var_name) {
            return self
                .type_tracker
                .get_row_view_column_id(local_idx, true, field_name);
        }
        if let Some(&binding_idx) = self.module_bindings.get(var_name) {
            return self
                .type_tracker
                .get_row_view_column_id(binding_idx, false, field_name);
        }
        None
    }

    /// Determine the appropriate LoadCol opcode for a RowView field.
    ///
    /// Looks up the field's FieldType and maps it to the corresponding opcode.
    /// Falls back to LoadColF64 if the type can't be determined.
    pub(super) fn row_view_field_opcode(&self, var_name: &str, field_name: &str) -> OpCode {
        use shape_runtime::type_schema::FieldType;

        let type_name = if let Some(local_idx) = self.resolve_local(var_name) {
            self.type_tracker
                .get_local_type(local_idx)
                .and_then(|info| info.type_name.clone())
        } else if let Some(&binding_idx) = self.module_bindings.get(var_name) {
            self.type_tracker
                .get_binding_type(binding_idx)
                .and_then(|info| info.type_name.clone())
        } else {
            None
        };

        if let Some(type_name) = type_name {
            if let Some(schema) = self.type_tracker.schema_registry().get(&type_name) {
                if let Some(field) = schema.get_field(field_name) {
                    return match field.field_type {
                        FieldType::F64 => OpCode::LoadColF64,
                        FieldType::I64 | FieldType::Timestamp => OpCode::LoadColI64,
                        FieldType::Bool => OpCode::LoadColBool,
                        FieldType::String => OpCode::LoadColStr,
                        _ => OpCode::LoadColF64, // default
                    };
                }
            }
        }
        OpCode::LoadColF64 // default
    }

    /// Resolve the NumericType for a RowView field (used for typed opcode emission).
    pub(super) fn resolve_row_view_field_numeric_type(
        &self,
        var_name: &str,
        field_name: &str,
    ) -> Option<crate::type_tracking::NumericType> {
        use crate::type_tracking::NumericType;
        use shape_runtime::type_schema::FieldType;

        let type_name = if let Some(local_idx) = self.resolve_local(var_name) {
            self.type_tracker
                .get_local_type(local_idx)
                .and_then(|info| info.type_name.clone())
        } else if let Some(&binding_idx) = self.module_bindings.get(var_name) {
            self.type_tracker
                .get_binding_type(binding_idx)
                .and_then(|info| info.type_name.clone())
        } else {
            None
        };

        if let Some(type_name) = type_name {
            if let Some(schema) = self.type_tracker.schema_registry().get(&type_name) {
                if let Some(field) = schema.get_field(field_name) {
                    return match field.field_type {
                        FieldType::F64 => Some(NumericType::Number),
                        FieldType::I64 | FieldType::Timestamp => Some(NumericType::Int),
                        FieldType::Decimal => Some(NumericType::Decimal),
                        _ => None,
                    };
                }
            }
        }
        None
    }

    /// Convert a TypeAnnotation to a FieldType for TypeSchema registration
    pub(super) fn type_annotation_to_field_type(
        ann: &shape_ast::ast::TypeAnnotation,
    ) -> shape_runtime::type_schema::FieldType {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_schema::FieldType;
        match ann {
            TypeAnnotation::Basic(s) => match s.as_str() {
                "number" | "float" | "f64" | "f32" => FieldType::F64,
                "i8" => FieldType::I8,
                "u8" => FieldType::U8,
                "i16" => FieldType::I16,
                "u16" => FieldType::U16,
                "i32" => FieldType::I32,
                "u32" => FieldType::U32,
                "u64" => FieldType::U64,
                "int" | "i64" | "integer" | "isize" | "usize" | "byte" | "char" => FieldType::I64,
                "string" | "str" => FieldType::String,
                "decimal" => FieldType::Decimal,
                "bool" | "boolean" => FieldType::Bool,
                "timestamp" => FieldType::Timestamp,
                // Non-primitive type names (e.g. "Server", "Inner") are nested
                // object references.  The parser emits Basic for `ident` matches
                // inside `basic_type`, so treat unknown names as Object references
                // to enable typed field access on nested structs.
                other => FieldType::Object(other.to_string()),
            },
            TypeAnnotation::Reference(s) => FieldType::Object(s.clone()),
            TypeAnnotation::Array(inner) => {
                FieldType::Array(Box::new(Self::type_annotation_to_field_type(inner)))
            }
            TypeAnnotation::Generic { name, .. } => match name.as_str() {
                // Generic containers that need NaN boxing
                "HashMap" | "Map" | "Result" | "Option" | "Set" => FieldType::Any,
                // User-defined generic structs — preserve the type name
                other => FieldType::Object(other.to_string()),
            },
            _ => FieldType::Any,
        }
    }

    /// Evaluate an annotation argument expression to a string representation.
    /// Only handles compile-time evaluable expressions (literals).
    pub(super) fn eval_annotation_arg(expr: &shape_ast::ast::Expr) -> Option<String> {
        use shape_ast::ast::{Expr, Literal};
        match expr {
            Expr::Literal(Literal::String(s), _) => Some(s.clone()),
            Expr::Literal(Literal::Number(n), _) => Some(n.to_string()),
            Expr::Literal(Literal::Int(i), _) => Some(i.to_string()),
            Expr::Literal(Literal::Bool(b), _) => Some(b.to_string()),
            _ => None,
        }
    }

    /// Get the schema ID for a `Table<T>` type annotation, if applicable.
    ///
    /// Returns `Some(schema_id)` if the annotation is `Table<T>` and `T` is a registered
    /// TypeSchema. Returns `None` otherwise.
    pub(super) fn get_table_schema_id(
        &self,
        type_ann: &shape_ast::ast::TypeAnnotation,
    ) -> Option<u16> {
        use shape_ast::ast::TypeAnnotation;
        if let TypeAnnotation::Generic { name, args } = type_ann {
            if name == "Table" && args.len() == 1 {
                let inner_name = match &args[0] {
                    TypeAnnotation::Reference(t) | TypeAnnotation::Basic(t) => Some(t.as_str()),
                    _ => None,
                };
                if let Some(type_name) = inner_name {
                    return self
                        .type_tracker
                        .schema_registry()
                        .get(type_name)
                        .map(|s| s.id as u16);
                }
            }
        }
        None
    }

    // ===== Drop scope management =====

    /// Push a new drop scope. Must be paired with pop_drop_scope().
    pub(super) fn push_drop_scope(&mut self) {
        self.drop_locals.push(Vec::new());
    }

    /// Pop the current drop scope, emitting DropCall instructions for all
    /// tracked locals in reverse order.
    pub(super) fn pop_drop_scope(&mut self) -> Result<()> {
        // Emit DropCall for each tracked local in reverse order
        if let Some(locals) = self.drop_locals.pop() {
            for (local_idx, is_async) in locals.into_iter().rev() {
                self.emit_drop_call_for_local(local_idx, is_async);
            }
        }
        Ok(())
    }

    /// Emit a single LoadLocal + DropCall pair for a local variable.
    /// The type name is resolved from the type tracker and encoded as a
    /// Property operand so the executor can look up `TypeName::drop`.
    fn emit_drop_call_for_local(&mut self, local_idx: u16, is_async: bool) {
        let type_name_opt = self
            .type_tracker
            .get_local_type(local_idx)
            .and_then(|info| info.type_name.clone());
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(local_idx)),
        ));
        let opcode = if is_async {
            OpCode::DropCallAsync
        } else {
            OpCode::DropCall
        };
        if let Some(type_name) = type_name_opt {
            let str_idx = self.program.add_string(type_name);
            self.emit(Instruction::new(opcode, Some(Operand::Property(str_idx))));
        } else {
            self.emit(Instruction::simple(opcode));
        }
    }

    /// Emit a single LoadModuleBinding + DropCall pair for a module binding.
    /// Similar to `emit_drop_call_for_local` but loads from module bindings.
    pub(super) fn emit_drop_call_for_module_binding(&mut self, binding_idx: u16, is_async: bool) {
        let type_name_opt = self
            .type_tracker
            .get_binding_type(binding_idx)
            .and_then(|info| info.type_name.clone());
        self.emit(Instruction::new(
            OpCode::LoadModuleBinding,
            Some(Operand::ModuleBinding(binding_idx)),
        ));
        let opcode = if is_async {
            OpCode::DropCallAsync
        } else {
            OpCode::DropCall
        };
        if let Some(type_name) = type_name_opt {
            let str_idx = self.program.add_string(type_name);
            self.emit(Instruction::new(opcode, Some(Operand::Property(str_idx))));
        } else {
            self.emit(Instruction::simple(opcode));
        }
    }

    /// Track a local variable as needing Drop at scope exit.
    pub(super) fn track_drop_local(&mut self, local_idx: u16, is_async: bool) {
        if let Some(scope) = self.drop_locals.last_mut() {
            scope.push((local_idx, is_async));
        }
    }

    /// Resolve the DropKind for a local variable's type.
    /// Returns None if the type is unknown or has no Drop impl.
    pub(super) fn local_drop_kind(&self, local_idx: u16) -> Option<DropKind> {
        let type_name = self
            .type_tracker
            .get_local_type(local_idx)
            .and_then(|info| info.type_name.as_ref())?;
        self.drop_type_info.get(type_name).copied()
    }

    /// Resolve DropKind from a type annotation.
    pub(super) fn annotation_drop_kind(&self, type_ann: &TypeAnnotation) -> Option<DropKind> {
        let type_name = Self::tracked_type_name_from_annotation(type_ann)?;
        self.drop_type_info.get(&type_name).copied()
    }

    /// Emit drops for all scopes being exited (used by return/break/continue).
    /// `scopes_to_exit` is the number of drop scopes to emit drops for.
    pub(super) fn emit_drops_for_early_exit(&mut self, scopes_to_exit: usize) -> Result<()> {
        let total = self.drop_locals.len();
        if scopes_to_exit > total {
            return Ok(());
        }
        // Collect locals from scopes being exited (innermost first)
        let mut scopes: Vec<Vec<(u16, bool)>> = Vec::new();
        for i in (total - scopes_to_exit..total).rev() {
            let locals = self.drop_locals.get(i).cloned().unwrap_or_default();
            scopes.push(locals);
        }
        // Now emit DropCall instructions
        for locals in scopes {
            for (local_idx, is_async) in locals.into_iter().rev() {
                self.emit_drop_call_for_local(local_idx, is_async);
            }
        }
        Ok(())
    }

    /// Track a module binding as needing Drop at program exit.
    pub(super) fn track_drop_module_binding(&mut self, binding_idx: u16, is_async: bool) {
        self.drop_module_bindings.push((binding_idx, is_async));
    }
}

#[cfg(test)]
mod tests {
    use super::super::BytecodeCompiler;
    use crate::compiler::ParamPassMode;
    use crate::type_tracking::BindingStorageClass;
    use shape_ast::ast::{Expr, Span, TypeAnnotation};
    use shape_runtime::type_schema::FieldType;

    #[test]
    fn test_type_annotation_to_field_type_array_recursive() {
        let ann = TypeAnnotation::Array(Box::new(TypeAnnotation::Basic("int".to_string())));
        let ft = BytecodeCompiler::type_annotation_to_field_type(&ann);
        assert_eq!(ft, FieldType::Array(Box::new(FieldType::I64)));
    }

    #[test]
    fn test_type_annotation_to_field_type_optional() {
        let ann = TypeAnnotation::Generic {
            name: "Option".to_string(),
            args: vec![TypeAnnotation::Basic("int".to_string())],
        };
        let ft = BytecodeCompiler::type_annotation_to_field_type(&ann);
        assert_eq!(ft, FieldType::Any);
    }

    #[test]
    fn test_type_annotation_to_field_type_generic_hashmap() {
        let ann = TypeAnnotation::Generic {
            name: "HashMap".to_string(),
            args: vec![
                TypeAnnotation::Basic("string".to_string()),
                TypeAnnotation::Basic("int".to_string()),
            ],
        };
        let ft = BytecodeCompiler::type_annotation_to_field_type(&ann);
        assert_eq!(ft, FieldType::Any);
    }

    #[test]
    fn test_type_annotation_to_field_type_generic_user_struct() {
        let ann = TypeAnnotation::Generic {
            name: "MyContainer".to_string(),
            args: vec![TypeAnnotation::Basic("string".to_string())],
        };
        let ft = BytecodeCompiler::type_annotation_to_field_type(&ann);
        assert_eq!(ft, FieldType::Object("MyContainer".to_string()));
    }

    #[test]
    fn test_flexible_storage_promotion_is_monotonic() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        compiler.promote_flexible_binding_storage_for_slot(
            slot,
            true,
            BindingStorageClass::UniqueHeap,
        );
        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );

        compiler.promote_flexible_binding_storage_for_slot(slot, true, BindingStorageClass::Direct);
        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );

        compiler.promote_flexible_binding_storage_for_slot(
            slot,
            true,
            BindingStorageClass::SharedCow,
        );
        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::SharedCow)
        );
    }

    #[test]
    fn test_escape_planner_marks_array_element_identifier_as_unique_heap() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        let expr = Expr::Array(
            vec![Expr::Identifier("value".to_string(), Span::DUMMY)],
            Span::DUMMY,
        );
        compiler.plan_flexible_binding_escape_from_expr(&expr);

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );
    }

    #[test]
    fn test_escape_planner_marks_if_branch_identifier_as_unique_heap() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        let expr = Expr::If(
            Box::new(shape_ast::ast::IfExpr {
                condition: Box::new(Expr::Literal(
                    shape_ast::ast::Literal::Bool(true),
                    Span::DUMMY,
                )),
                then_branch: Box::new(Expr::Identifier("value".to_string(), Span::DUMMY)),
                else_branch: None,
            }),
            Span::DUMMY,
        );
        compiler.plan_flexible_binding_escape_from_expr(&expr);

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );
    }

    #[test]
    fn test_escape_planner_marks_async_let_rhs_identifier_as_unique_heap() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        let expr = Expr::AsyncLet(
            Box::new(shape_ast::ast::AsyncLetExpr {
                name: "task".to_string(),
                expr: Box::new(Expr::Identifier("value".to_string(), Span::DUMMY)),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        );
        compiler.plan_flexible_binding_escape_from_expr(&expr);

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );
    }

    #[test]
    fn test_call_args_mark_by_value_identifier_as_unique_heap() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        compiler
            .compile_call_args(&[Expr::Identifier("value".to_string(), Span::DUMMY)], None)
            .expect("call args should compile");

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );
    }

    #[test]
    fn test_call_args_leave_by_ref_identifier_storage_unchanged() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        compiler
            .compile_call_args(
                &[Expr::Identifier("value".to_string(), Span::DUMMY)],
                Some(&[ParamPassMode::ByRefShared]),
            )
            .expect("reference call args should compile");

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::Deferred)
        );
    }
}

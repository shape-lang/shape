//! Helper methods for bytecode compilation

use crate::borrow_checker::{BorrowId, BorrowMode, BorrowPlace};
use crate::bytecode::{BuiltinFunction, Constant, Instruction, OpCode, Operand};
use crate::executor::typed_object_ops::field_type_to_tag;
use crate::type_tracking::{
    BindingOwnershipClass, BindingSemantics, BindingStorageClass, NumericType, StorageHint,
    TypeTracker, VariableKind, VariableTypeInfo,
};
use shape_ast::ast::{
    BlockItem, DestructurePattern, Expr, FunctionParameter, Item, Pattern,
    PatternConstructorFields, Spanned, Statement, TypeAnnotation,
};
use shape_ast::error::{Result, ShapeError, SourceLocation};
use shape_runtime::type_schema::FieldType;
use std::collections::{BTreeSet, HashMap, HashSet};

use super::{BytecodeCompiler, DropKind, ParamPassMode, TrackedReferenceBorrow};

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
                    shape_ast::ast::ExportItem::Named(_) => false,
                    shape_ast::ast::ExportItem::TypeAlias(_) => false,
                    shape_ast::ast::ExportItem::ForeignFunction(_)
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
            Expr::Spread(inner, _)
            | Expr::TypeAssertion { expr: inner, .. }
            | Expr::UsingImpl { expr: inner, .. }
            | Expr::TryOperator(inner, _) => self.plan_flexible_binding_escape_from_expr(inner),
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
    ) -> Result<BorrowId> {
        let borrow_id = match expr {
            Expr::Identifier(name, id_span) => self.compile_reference_identifier(name, *id_span, mode),
            Expr::PropertyAccess {
                object,
                property,
                optional: false,
                ..
            } => self.compile_reference_property_access(object, property, span, mode),
            _ => Err(ShapeError::SemanticError {
                message:
                    "`&` can only be applied to a simple variable name or compile-time-resolved field access (e.g., `&x`, `&obj.a`)".to_string(),
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
    ) -> Result<BorrowId> {
        let Some(place) = self.try_resolve_typed_field_place(object, property) else {
            return Err(ShapeError::SemanticError {
                message:
                    "`&` can only be applied to a simple variable name or compile-time-resolved field access (e.g., `&x`, `&obj.a`)".to_string(),
                location: Some(self.span_to_source_location(span)),
            });
        };

        if mode == BorrowMode::Exclusive {
            if place.is_local {
                if self.const_locals.contains(&place.slot) {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "Cannot pass const variable '{}.{}' by exclusive reference",
                            place.root_name, property
                        ),
                        location: Some(self.span_to_source_location(span)),
                    });
                }
            } else if self.const_module_bindings.contains(&place.slot) {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Cannot pass const variable '{}.{}' by exclusive reference",
                        place.root_name, property
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
        }

        let label = format!("{}.{}", place.root_name, property);
        let source_loc = self.span_to_source_location(span);
        let borrow_id = self
            .borrow_checker
            .create_borrow(place.borrow_key, place.slot, mode, span, Some(source_loc))
            .map_err(|err| Self::relabel_borrow_error(err, place.borrow_key, &label))?;

        let root_operand = if place.is_local {
            Operand::Local(place.slot)
        } else {
            Operand::ModuleBinding(place.slot)
        };
        self.emit(Instruction::new(OpCode::MakeRef, Some(root_operand)));
        self.emit(Instruction::new(
            OpCode::MakeFieldRef,
            Some(place.typed_operand),
        ));
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
    ) -> Result<Option<(BorrowId, bool)>> {
        if let shape_ast::ast::Expr::Reference {
            expr: inner,
            is_mutable,
            span,
        } = expr
        {
            let mode = if *is_mutable {
                BorrowMode::Exclusive
            } else {
                BorrowMode::Shared
            };
            let borrow_id = self.compile_reference_expr(inner, *span, mode)?;
            if borrow_id == BorrowId::MAX {
                Ok(None)
            } else {
                Ok(Some((borrow_id, *is_mutable)))
            }
        } else {
            self.compile_expr(expr)?;
            Ok(None)
        }
    }

    fn track_reference_binding_slot(&mut self, slot: u16, is_local: bool) {
        if is_local {
            if let Some(scope) = self.scoped_reference_value_locals.last_mut() {
                scope.insert(slot);
            }
        } else if let Some(scope) = self.scoped_reference_value_module_bindings.last_mut() {
            scope.insert(slot);
        }
    }

    pub(super) fn bind_reference_value_slot(
        &mut self,
        slot: u16,
        is_local: bool,
        name: &str,
        is_exclusive: bool,
        borrow_id: BorrowId,
    ) {
        self.borrow_checker.rebind_borrow_ref_slot(borrow_id, slot);
        self.mark_reference_binding(slot, is_local, is_exclusive);
        self.track_reference_binding_slot(slot, is_local);
        if is_local {
            self.type_tracker
                .set_local_type(slot, VariableTypeInfo::unknown());
        } else {
            self.type_tracker
                .set_binding_type(slot, VariableTypeInfo::unknown());
        }
        let tracked = TrackedReferenceBorrow {
            borrow_id,
            name: name.to_string(),
        };
        if is_local {
            self.tracked_reference_borrow_locals.insert(slot, tracked);
        } else {
            self.tracked_reference_borrow_module_bindings
                .insert(slot, tracked);
        }
    }

    pub(super) fn release_tracked_reference_borrow(&mut self, slot: u16, is_local: bool) {
        let tracked = if is_local {
            self.tracked_reference_borrow_locals.remove(&slot)
        } else {
            self.tracked_reference_borrow_module_bindings.remove(&slot)
        };
        if let Some(tracked) = tracked {
            self.borrow_checker.release_borrow_by_id(tracked.borrow_id);
        }
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
            self.mark_reference_binding(slot, is_local, *is_mutable);
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
        ref_borrow: Option<(BorrowId, bool)>,
    ) {
        if let Some((borrow_id, is_exclusive)) = ref_borrow {
            self.bind_reference_value_slot(slot, is_local, name, is_exclusive, borrow_id);
        } else {
            self.update_reference_binding_from_expr(slot, is_local, expr);
        }
    }

    pub(super) fn push_module_reference_scope(&mut self) {
        self.scoped_reference_value_module_bindings
            .push(Default::default());
    }

    pub(super) fn pop_module_reference_scope(&mut self) {
        self.scoped_reference_value_module_bindings.pop();
    }

    pub(super) fn push_future_reference_use_names(&mut self, names: HashSet<String>) {
        self.future_reference_use_names.push(names);
    }

    pub(super) fn pop_future_reference_use_names(&mut self) {
        self.future_reference_use_names.pop();
        if self.future_reference_use_names.is_empty() {
            self.future_reference_use_names.push(HashSet::new());
        }
    }

    pub(super) fn future_reference_use_names_for_remaining_statements(
        &self,
        remaining: &[Statement],
    ) -> HashSet<String> {
        let mut names = self
            .future_reference_use_names
            .last()
            .cloned()
            .unwrap_or_default();
        for tracked in self.tracked_reference_borrow_locals.values() {
            if remaining
                .iter()
                .any(|stmt| statement_uses_identifier(stmt, &tracked.name))
            {
                names.insert(tracked.name.clone());
            }
        }
        for tracked in self.tracked_reference_borrow_module_bindings.values() {
            if remaining
                .iter()
                .any(|stmt| statement_uses_identifier(stmt, &tracked.name))
            {
                names.insert(tracked.name.clone());
            }
        }
        names
    }

    pub(super) fn future_reference_use_names_for_remaining_block_items(
        &self,
        remaining: &[BlockItem],
    ) -> HashSet<String> {
        let mut names = self
            .future_reference_use_names
            .last()
            .cloned()
            .unwrap_or_default();
        for tracked in self.tracked_reference_borrow_locals.values() {
            if remaining
                .iter()
                .any(|item| block_item_uses_identifier(item, &tracked.name))
            {
                names.insert(tracked.name.clone());
            }
        }
        for tracked in self.tracked_reference_borrow_module_bindings.values() {
            if remaining
                .iter()
                .any(|item| block_item_uses_identifier(item, &tracked.name))
            {
                names.insert(tracked.name.clone());
            }
        }
        names
    }

    pub(super) fn future_reference_use_names_for_remaining_items(
        &self,
        remaining: &[Item],
    ) -> HashSet<String> {
        let mut names = self
            .future_reference_use_names
            .last()
            .cloned()
            .unwrap_or_default();
        for tracked in self.tracked_reference_borrow_locals.values() {
            if remaining
                .iter()
                .any(|item| item_uses_identifier(item, &tracked.name))
            {
                names.insert(tracked.name.clone());
            }
        }
        for tracked in self.tracked_reference_borrow_module_bindings.values() {
            if remaining
                .iter()
                .any(|item| item_uses_identifier(item, &tracked.name))
            {
                names.insert(tracked.name.clone());
            }
        }
        names
    }

    pub(super) fn push_repeating_reference_release_barrier(&mut self) {
        self.repeating_body_reference_local_barriers.push(
            self.tracked_reference_borrow_locals
                .keys()
                .copied()
                .collect(),
        );
        self.repeating_body_reference_module_binding_barriers.push(
            self.tracked_reference_borrow_module_bindings
                .keys()
                .copied()
                .collect(),
        );
        let protected_places = self
            .tracked_reference_borrow_locals
            .keys()
            .copied()
            .chain(
                self.tracked_reference_borrow_module_bindings
                    .keys()
                    .copied(),
            )
            .filter_map(|slot| self.borrow_checker.borrow_place_for_ref_slot(slot))
            .collect();
        self.repeating_body_protected_places.push(protected_places);
    }

    pub(super) fn pop_repeating_reference_release_barrier(&mut self) {
        self.repeating_body_reference_local_barriers.pop();
        self.repeating_body_reference_module_binding_barriers.pop();
        self.repeating_body_protected_places.pop();
    }

    fn local_reference_release_is_barrier_protected(&self, slot: u16) -> bool {
        self.repeating_body_reference_local_barriers
            .iter()
            .any(|slots| slots.contains(&slot))
    }

    fn module_reference_release_is_barrier_protected(&self, slot: u16) -> bool {
        self.repeating_body_reference_module_binding_barriers
            .iter()
            .any(|slots| slots.contains(&slot))
    }

    pub(super) fn check_write_allowed_in_current_context(
        &self,
        place: BorrowPlace,
        source_location: Option<SourceLocation>,
    ) -> Result<()> {
        self.borrow_checker
            .check_write_allowed(place, source_location.clone())?;
        if self
            .repeating_body_protected_places
            .iter()
            .flatten()
            .any(|protected| {
                crate::borrow_checker::BorrowChecker::places_conflict(place, *protected)
            })
        {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "[B0002] cannot write to this value while it is borrowed in an active loop iteration (slot {})",
                    place
                ),
                location: source_location,
            });
        }
        Ok(())
    }

    pub(super) fn check_named_binding_write_allowed(
        &self,
        name: &str,
        source_location: Option<SourceLocation>,
    ) -> Result<()> {
        if let Some(local_idx) = self.resolve_local(name) {
            if self.const_locals.contains(&local_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!("Cannot reassign const variable '{}'", name),
                    location: source_location,
                });
            }
            if self.immutable_locals.contains(&local_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Cannot reassign immutable variable '{}'. Use `let mut` or `var` for mutable bindings",
                        name
                    ),
                    location: source_location,
                });
            }
            return self
                .check_write_allowed_in_current_context(
                    Self::borrow_key_for_local(local_idx),
                    source_location,
                )
                .map_err(|e| {
                    Self::relabel_borrow_error(e, Self::borrow_key_for_local(local_idx), name)
                });
        }

        let scoped_name = self
            .resolve_scoped_module_binding_name(name)
            .unwrap_or_else(|| name.to_string());
        if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
            if self.const_module_bindings.contains(&binding_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!("Cannot reassign const variable '{}'", name),
                    location: source_location,
                });
            }
            if self.immutable_module_bindings.contains(&binding_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Cannot reassign immutable variable '{}'. Use `let mut` or `var` for mutable bindings",
                        name
                    ),
                    location: source_location,
                });
            }
            return self
                .check_write_allowed_in_current_context(
                    Self::borrow_key_for_module_binding(binding_idx),
                    source_location,
                )
                .map_err(|e| {
                    Self::relabel_borrow_error(
                        e,
                        Self::borrow_key_for_module_binding(binding_idx),
                        name,
                    )
                });
        }

        Ok(())
    }

    pub(super) fn release_unused_local_reference_borrows_for_remaining_statements(
        &mut self,
        remaining: &[Statement],
    ) {
        let future_names = self
            .future_reference_use_names
            .last()
            .cloned()
            .unwrap_or_default();
        let dead_slots: Vec<u16> = self
            .tracked_reference_borrow_locals
            .iter()
            .filter_map(|(slot, tracked)| {
                if self.local_reference_release_is_barrier_protected(*slot)
                    || future_names.contains(&tracked.name)
                    || remaining
                        .iter()
                        .any(|stmt| statement_uses_identifier(stmt, &tracked.name))
                {
                    None
                } else {
                    Some(*slot)
                }
            })
            .collect();
        for slot in dead_slots {
            self.release_tracked_reference_borrow(slot, true);
        }
    }

    pub(super) fn release_unused_local_reference_borrows_for_remaining_block_items(
        &mut self,
        remaining: &[BlockItem],
    ) {
        let future_names = self
            .future_reference_use_names
            .last()
            .cloned()
            .unwrap_or_default();
        let dead_slots: Vec<u16> = self
            .tracked_reference_borrow_locals
            .iter()
            .filter_map(|(slot, tracked)| {
                if self.local_reference_release_is_barrier_protected(*slot)
                    || future_names.contains(&tracked.name)
                    || remaining
                        .iter()
                        .any(|item| block_item_uses_identifier(item, &tracked.name))
                {
                    None
                } else {
                    Some(*slot)
                }
            })
            .collect();
        for slot in dead_slots {
            self.release_tracked_reference_borrow(slot, true);
        }
    }

    pub(super) fn release_unused_module_reference_borrows_for_remaining_statements(
        &mut self,
        remaining: &[Statement],
    ) {
        let future_names = self
            .future_reference_use_names
            .last()
            .cloned()
            .unwrap_or_default();
        let dead_slots: Vec<u16> = self
            .tracked_reference_borrow_module_bindings
            .iter()
            .filter_map(|(slot, tracked)| {
                if self.module_reference_release_is_barrier_protected(*slot)
                    || future_names.contains(&tracked.name)
                    || remaining
                        .iter()
                        .any(|stmt| statement_uses_identifier(stmt, &tracked.name))
                {
                    None
                } else {
                    Some(*slot)
                }
            })
            .collect();
        for slot in dead_slots {
            self.release_tracked_reference_borrow(slot, false);
        }
    }

    pub(super) fn release_unused_module_reference_borrows_for_remaining_block_items(
        &mut self,
        remaining: &[BlockItem],
    ) {
        let future_names = self
            .future_reference_use_names
            .last()
            .cloned()
            .unwrap_or_default();
        let dead_slots: Vec<u16> = self
            .tracked_reference_borrow_module_bindings
            .iter()
            .filter_map(|(slot, tracked)| {
                if self.module_reference_release_is_barrier_protected(*slot)
                    || future_names.contains(&tracked.name)
                    || remaining
                        .iter()
                        .any(|item| block_item_uses_identifier(item, &tracked.name))
                {
                    None
                } else {
                    Some(*slot)
                }
            })
            .collect();
        for slot in dead_slots {
            self.release_tracked_reference_borrow(slot, false);
        }
    }

    pub(super) fn release_unused_module_reference_borrows_for_remaining_items(
        &mut self,
        remaining: &[Item],
    ) {
        let future_names = self
            .future_reference_use_names
            .last()
            .cloned()
            .unwrap_or_default();
        let dead_slots: Vec<u16> = self
            .tracked_reference_borrow_module_bindings
            .iter()
            .filter_map(|(slot, tracked)| {
                if self.module_reference_release_is_barrier_protected(*slot)
                    || future_names.contains(&tracked.name)
                    || remaining
                        .iter()
                        .any(|item| item_uses_identifier(item, &tracked.name))
                {
                    None
                } else {
                    Some(*slot)
                }
            })
            .collect();
        for slot in dead_slots {
            self.release_tracked_reference_borrow(slot, false);
        }
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
        let saved = self.in_call_args;
        let saved_mode = self.current_call_arg_borrow_mode;
        self.in_call_args = true;
        self.borrow_checker.enter_region();
        self.call_arg_module_binding_ref_writebacks.push(Vec::new());

        let mut first_error: Option<ShapeError> = None;
        for (idx, arg) in args.iter().enumerate() {
            let pass_mode = expected_param_modes
                .and_then(|modes| modes.get(idx).copied())
                .unwrap_or(ParamPassMode::ByValue);
            self.current_call_arg_borrow_mode = match pass_mode {
                ParamPassMode::ByRefExclusive => Some(BorrowMode::Exclusive),
                ParamPassMode::ByRefShared => Some(BorrowMode::Shared),
                ParamPassMode::ByValue => None,
            };

            let arg_result = match pass_mode {
                ParamPassMode::ByRefExclusive | ParamPassMode::ByRefShared => {
                    let borrow_mode = if pass_mode.is_exclusive() {
                        BorrowMode::Exclusive
                    } else {
                        BorrowMode::Shared
                    };
                    if matches!(arg, shape_ast::ast::Expr::Reference { .. }) {
                        self.compile_expr(arg)
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

        self.current_call_arg_borrow_mode = saved_mode;
        self.borrow_checker.exit_region();
        self.in_call_args = saved;
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
        self.current_call_arg_borrow_mode
            .unwrap_or(BorrowMode::Exclusive)
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

    fn compile_implicit_reference_arg(
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
            _ if mode == BorrowMode::Exclusive => Err(ShapeError::SemanticError {
                message: "[B0004] mutable reference arguments must be simple variables".to_string(),
                location: Some(self.span_to_source_location(arg.span())),
            }),
            _ => {
                self.compile_expr(arg)?;
                let temp = self.declare_temp_local("__arg_ref_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(temp)),
                ));
                let source_loc = self.span_to_source_location(arg.span());
                self.borrow_checker.create_borrow(
                    Self::borrow_key_for_local(temp),
                    temp,
                    mode,
                    arg.span(),
                    Some(source_loc),
                )?;
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
    ) -> Result<BorrowId> {
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
            let source_loc = self.span_to_source_location(span);
            let borrow_id = self
                .borrow_checker
                .create_borrow(
                    Self::borrow_key_for_local(local_idx),
                    local_idx,
                    mode,
                    span,
                    Some(source_loc),
                )
                .map_err(|e| match e {
                    ShapeError::SemanticError { message, location } => {
                        let user_msg = message
                            .replace(&format!("(slot {})", local_idx), &format!("'{}'", name));
                        ShapeError::SemanticError {
                            message: user_msg,
                            location,
                        }
                    }
                    other => other,
                })?;
            self.emit(Instruction::new(
                OpCode::MakeRef,
                Some(Operand::Local(local_idx)),
            ));
            Ok(borrow_id)
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
            let source_loc = self.span_to_source_location(span);
            let borrow_id = self
                .borrow_checker
                .create_borrow(
                    Self::borrow_key_for_module_binding(binding_idx),
                    binding_idx,
                    mode,
                    span,
                    Some(source_loc),
                )
                .map_err(|e| match e {
                    ShapeError::SemanticError { message, location } => {
                        let user_msg = message.replace(
                            &format!(
                                "(slot {})",
                                Self::borrow_key_for_module_binding(binding_idx)
                            ),
                            &format!("'{}'", name),
                        );
                        ShapeError::SemanticError {
                            message: user_msg,
                            location,
                        }
                    }
                    other => other,
                })?;
            self.emit(Instruction::new(
                OpCode::MakeRef,
                Some(Operand::ModuleBinding(binding_idx)),
            ));
            Ok(borrow_id)
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
            let source_loc = self.span_to_source_location(span);
            let borrow_id = self.borrow_checker.create_borrow(
                Self::borrow_key_for_local(temp),
                temp,
                mode,
                span,
                Some(source_loc),
            )?;
            self.emit(Instruction::new(
                OpCode::MakeRef,
                Some(Operand::Local(temp)),
            ));
            Ok(borrow_id)
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
        self.scoped_reference_value_locals.push(Default::default());
        self.type_tracker.push_scope();
        self.borrow_checker.enter_region();
    }

    /// Pop a scope
    pub(super) fn pop_scope(&mut self) {
        let scope_slots = self.scoped_reference_value_locals.pop().unwrap_or_default();
        for slot in scope_slots {
            self.clear_reference_binding(slot, true);
        }
        self.borrow_checker.exit_region();
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
            if self.ref_locals.contains(&local_idx) {
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
            if self.ref_locals.contains(&local_idx) {
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

    /// Get built-in function by name
    pub(super) fn get_builtin_function(&self, name: &str) -> Option<BuiltinFunction> {
        // Internal builtins are only accessible from stdlib functions.
        // User code must use the safe wrappers (e.g. std::core::math).
        // Note: __into_* and __try_into_* are NOT gated — the compiler generates
        // calls to them for type assertions (x as int, x try as int).
        if !self.allow_internal_builtins
            && (name.starts_with("__native_")
                || name.starts_with("__intrinsic_")
                || name.starts_with("__json_"))
        {
            return None;
        }
        match name {
            // Option type constructor
            "Some" => Some(BuiltinFunction::SomeCtor),
            "Ok" => Some(BuiltinFunction::OkCtor),
            "Err" => Some(BuiltinFunction::ErrCtor),
            "HashMap" => Some(BuiltinFunction::HashMapCtor),
            "Set" => Some(BuiltinFunction::SetCtor),
            "Deque" => Some(BuiltinFunction::DequeCtor),
            "PriorityQueue" => Some(BuiltinFunction::PriorityQueueCtor),
            "Mutex" => Some(BuiltinFunction::MutexCtor),
            "Atomic" => Some(BuiltinFunction::AtomicCtor),
            "Lazy" => Some(BuiltinFunction::LazyCtor),
            "Channel" => Some(BuiltinFunction::ChannelCtor),
            // Json navigation helpers
            "__json_object_get" => Some(BuiltinFunction::JsonObjectGet),
            "__json_array_at" => Some(BuiltinFunction::JsonArrayAt),
            "__json_object_keys" => Some(BuiltinFunction::JsonObjectKeys),
            "__json_array_len" => Some(BuiltinFunction::JsonArrayLen),
            "__json_object_len" => Some(BuiltinFunction::JsonObjectLen),
            "__intrinsic_vec_abs" => Some(BuiltinFunction::IntrinsicVecAbs),
            "__intrinsic_vec_sqrt" => Some(BuiltinFunction::IntrinsicVecSqrt),
            "__intrinsic_vec_ln" => Some(BuiltinFunction::IntrinsicVecLn),
            "__intrinsic_vec_exp" => Some(BuiltinFunction::IntrinsicVecExp),
            "__intrinsic_vec_add" => Some(BuiltinFunction::IntrinsicVecAdd),
            "__intrinsic_vec_sub" => Some(BuiltinFunction::IntrinsicVecSub),
            "__intrinsic_vec_mul" => Some(BuiltinFunction::IntrinsicVecMul),
            "__intrinsic_vec_div" => Some(BuiltinFunction::IntrinsicVecDiv),
            "__intrinsic_vec_max" => Some(BuiltinFunction::IntrinsicVecMax),
            "__intrinsic_vec_min" => Some(BuiltinFunction::IntrinsicVecMin),
            "__intrinsic_vec_select" => Some(BuiltinFunction::IntrinsicVecSelect),
            "__intrinsic_matmul_vec" => Some(BuiltinFunction::IntrinsicMatMulVec),
            "__intrinsic_matmul_mat" => Some(BuiltinFunction::IntrinsicMatMulMat),

            // Existing builtins
            "abs" => Some(BuiltinFunction::Abs),
            "min" => Some(BuiltinFunction::Min),
            "max" => Some(BuiltinFunction::Max),
            "sqrt" => Some(BuiltinFunction::Sqrt),
            "ln" => Some(BuiltinFunction::Ln),
            "pow" => Some(BuiltinFunction::Pow),
            "exp" => Some(BuiltinFunction::Exp),
            "log" => Some(BuiltinFunction::Log),
            "floor" => Some(BuiltinFunction::Floor),
            "ceil" => Some(BuiltinFunction::Ceil),
            "round" => Some(BuiltinFunction::Round),
            "sin" => Some(BuiltinFunction::Sin),
            "cos" => Some(BuiltinFunction::Cos),
            "tan" => Some(BuiltinFunction::Tan),
            "asin" => Some(BuiltinFunction::Asin),
            "acos" => Some(BuiltinFunction::Acos),
            "atan" => Some(BuiltinFunction::Atan),
            "stddev" => Some(BuiltinFunction::StdDev),
            "__intrinsic_map" => Some(BuiltinFunction::Map),
            "__intrinsic_filter" => Some(BuiltinFunction::Filter),
            "__intrinsic_reduce" => Some(BuiltinFunction::Reduce),
            "print" => Some(BuiltinFunction::Print),
            "format" => Some(BuiltinFunction::Format),
            "len" | "count" => Some(BuiltinFunction::Len),
            // "throw" removed: Shape uses Result types
            "__intrinsic_snapshot" | "snapshot" => Some(BuiltinFunction::Snapshot),
            "exit" => Some(BuiltinFunction::Exit),
            "range" => Some(BuiltinFunction::Range),
            "is_number" | "isNumber" => Some(BuiltinFunction::IsNumber),
            "is_string" | "isString" => Some(BuiltinFunction::IsString),
            "is_bool" | "isBool" => Some(BuiltinFunction::IsBool),
            "is_array" | "isArray" => Some(BuiltinFunction::IsArray),
            "is_object" | "isObject" => Some(BuiltinFunction::IsObject),
            "is_data_row" | "isDataRow" => Some(BuiltinFunction::IsDataRow),
            "to_string" | "toString" => Some(BuiltinFunction::ToString),
            "to_number" | "toNumber" => Some(BuiltinFunction::ToNumber),
            "to_bool" | "toBool" => Some(BuiltinFunction::ToBool),
            "__into_int" => Some(BuiltinFunction::IntoInt),
            "__into_number" => Some(BuiltinFunction::IntoNumber),
            "__into_decimal" => Some(BuiltinFunction::IntoDecimal),
            "__into_bool" => Some(BuiltinFunction::IntoBool),
            "__into_string" => Some(BuiltinFunction::IntoString),
            "__try_into_int" => Some(BuiltinFunction::TryIntoInt),
            "__try_into_number" => Some(BuiltinFunction::TryIntoNumber),
            "__try_into_decimal" => Some(BuiltinFunction::TryIntoDecimal),
            "__try_into_bool" => Some(BuiltinFunction::TryIntoBool),
            "__try_into_string" => Some(BuiltinFunction::TryIntoString),
            "__native_ptr_size" => Some(BuiltinFunction::NativePtrSize),
            "__native_ptr_new_cell" => Some(BuiltinFunction::NativePtrNewCell),
            "__native_ptr_free_cell" => Some(BuiltinFunction::NativePtrFreeCell),
            "__native_ptr_read_ptr" => Some(BuiltinFunction::NativePtrReadPtr),
            "__native_ptr_write_ptr" => Some(BuiltinFunction::NativePtrWritePtr),
            "__native_table_from_arrow_c" => Some(BuiltinFunction::NativeTableFromArrowC),
            "__native_table_from_arrow_c_typed" => {
                Some(BuiltinFunction::NativeTableFromArrowCTyped)
            }
            "__native_table_bind_type" => Some(BuiltinFunction::NativeTableBindType),
            "fold" => Some(BuiltinFunction::ControlFold),

            // Math intrinsics
            "__intrinsic_sum" => Some(BuiltinFunction::IntrinsicSum),
            "__intrinsic_mean" => Some(BuiltinFunction::IntrinsicMean),
            "__intrinsic_min" => Some(BuiltinFunction::IntrinsicMin),
            "__intrinsic_max" => Some(BuiltinFunction::IntrinsicMax),
            "__intrinsic_std" => Some(BuiltinFunction::IntrinsicStd),
            "__intrinsic_variance" => Some(BuiltinFunction::IntrinsicVariance),

            // Random intrinsics
            "__intrinsic_random" => Some(BuiltinFunction::IntrinsicRandom),
            "__intrinsic_random_int" => Some(BuiltinFunction::IntrinsicRandomInt),
            "__intrinsic_random_seed" => Some(BuiltinFunction::IntrinsicRandomSeed),
            "__intrinsic_random_normal" => Some(BuiltinFunction::IntrinsicRandomNormal),
            "__intrinsic_random_array" => Some(BuiltinFunction::IntrinsicRandomArray),

            // Distribution intrinsics
            "__intrinsic_dist_uniform" => Some(BuiltinFunction::IntrinsicDistUniform),
            "__intrinsic_dist_lognormal" => Some(BuiltinFunction::IntrinsicDistLognormal),
            "__intrinsic_dist_exponential" => Some(BuiltinFunction::IntrinsicDistExponential),
            "__intrinsic_dist_poisson" => Some(BuiltinFunction::IntrinsicDistPoisson),
            "__intrinsic_dist_sample_n" => Some(BuiltinFunction::IntrinsicDistSampleN),

            // Stochastic process intrinsics
            "__intrinsic_brownian_motion" => Some(BuiltinFunction::IntrinsicBrownianMotion),
            "__intrinsic_gbm" => Some(BuiltinFunction::IntrinsicGbm),
            "__intrinsic_ou_process" => Some(BuiltinFunction::IntrinsicOuProcess),
            "__intrinsic_random_walk" => Some(BuiltinFunction::IntrinsicRandomWalk),

            // Rolling intrinsics
            "__intrinsic_rolling_sum" => Some(BuiltinFunction::IntrinsicRollingSum),
            "__intrinsic_rolling_mean" => Some(BuiltinFunction::IntrinsicRollingMean),
            "__intrinsic_rolling_std" => Some(BuiltinFunction::IntrinsicRollingStd),
            "__intrinsic_rolling_min" => Some(BuiltinFunction::IntrinsicRollingMin),
            "__intrinsic_rolling_max" => Some(BuiltinFunction::IntrinsicRollingMax),
            "__intrinsic_ema" => Some(BuiltinFunction::IntrinsicEma),
            "__intrinsic_linear_recurrence" => Some(BuiltinFunction::IntrinsicLinearRecurrence),

            // Series intrinsics
            "__intrinsic_shift" => Some(BuiltinFunction::IntrinsicShift),
            "__intrinsic_diff" => Some(BuiltinFunction::IntrinsicDiff),
            "__intrinsic_pct_change" => Some(BuiltinFunction::IntrinsicPctChange),
            "__intrinsic_fillna" => Some(BuiltinFunction::IntrinsicFillna),
            "__intrinsic_cumsum" => Some(BuiltinFunction::IntrinsicCumsum),
            "__intrinsic_cumprod" => Some(BuiltinFunction::IntrinsicCumprod),
            "__intrinsic_clip" => Some(BuiltinFunction::IntrinsicClip),

            // Trigonometric intrinsics (map __intrinsic_ forms to existing builtins)
            "__intrinsic_sin" => Some(BuiltinFunction::Sin),
            "__intrinsic_cos" => Some(BuiltinFunction::Cos),
            "__intrinsic_tan" => Some(BuiltinFunction::Tan),
            "__intrinsic_asin" => Some(BuiltinFunction::Asin),
            "__intrinsic_acos" => Some(BuiltinFunction::Acos),
            "__intrinsic_atan" => Some(BuiltinFunction::Atan),
            "__intrinsic_atan2" => Some(BuiltinFunction::IntrinsicAtan2),
            "__intrinsic_sinh" => Some(BuiltinFunction::IntrinsicSinh),
            "__intrinsic_cosh" => Some(BuiltinFunction::IntrinsicCosh),
            "__intrinsic_tanh" => Some(BuiltinFunction::IntrinsicTanh),

            // Statistical intrinsics
            "__intrinsic_correlation" => Some(BuiltinFunction::IntrinsicCorrelation),
            "__intrinsic_covariance" => Some(BuiltinFunction::IntrinsicCovariance),
            "__intrinsic_percentile" => Some(BuiltinFunction::IntrinsicPercentile),
            "__intrinsic_median" => Some(BuiltinFunction::IntrinsicMedian),

            // Character code intrinsics
            "__intrinsic_char_code" => Some(BuiltinFunction::IntrinsicCharCode),
            "__intrinsic_from_char_code" => Some(BuiltinFunction::IntrinsicFromCharCode),

            // Series access
            "__intrinsic_series" => Some(BuiltinFunction::IntrinsicSeries),

            // Reflection
            "reflect" => Some(BuiltinFunction::Reflect),

            // Additional math builtins
            "sign" => Some(BuiltinFunction::Sign),
            "gcd" => Some(BuiltinFunction::Gcd),
            "lcm" => Some(BuiltinFunction::Lcm),
            "hypot" => Some(BuiltinFunction::Hypot),
            "clamp" => Some(BuiltinFunction::Clamp),
            "isNaN" | "is_nan" => Some(BuiltinFunction::IsNaN),
            "isFinite" | "is_finite" => Some(BuiltinFunction::IsFinite),

            _ => None,
        }
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
}

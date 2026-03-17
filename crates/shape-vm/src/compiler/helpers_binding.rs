//! Binding semantics and storage class management

use crate::type_tracking::{
    Aliasability, BindingOwnershipClass, BindingSemantics, BindingStorageClass, EscapeStatus,
    MutationCapability,
};
use shape_ast::ast::{
    BlockItem, DestructurePattern, Expr, FunctionParameter, Pattern, PatternConstructorFields,
};

use super::{BytecodeCompiler, ParamPassMode};

impl BytecodeCompiler {
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

    pub(super) fn default_binding_storage_class_for_slot(
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
}

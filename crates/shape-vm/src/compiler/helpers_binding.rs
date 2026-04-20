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
            return_ownership_hint: None,
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
    ///
    /// The compiler's local indexing is 0-based (first bytecode local =
    /// index 0), but MIR reserves `SlotId(0)` for the return slot and starts
    /// user locals at `SlotId(1)`. Add 1 when indexing into the plan so
    /// callers can pass bytecode-local indices directly.
    pub(super) fn mir_storage_class_for_slot(&self, slot: u16) -> Option<BindingStorageClass> {
        let plan = self.current_storage_plan()?;
        // Prefer the +1-offset lookup (current MIR convention). Fall back to
        // the direct lookup so the helper stays robust if a future MIR ABI
        // change stops reserving SlotId(0).
        plan.slot_classes
            .get(&crate::mir::SlotId(slot.saturating_add(1)))
            .copied()
            .or_else(|| plan.slot_classes.get(&crate::mir::SlotId(slot)).copied())
    }

    /// MIR analysis is authoritative for both function bodies and top-level code.
    /// `analyze_non_function_items_with_mir` runs in the main pipeline before
    /// compilation, so MIR write authority applies universally.
    pub(super) fn current_binding_uses_mir_write_authority(&self, _is_local: bool) -> bool {
        true
    }

    /// Get the MIR context name for the code currently being compiled.
    /// For function bodies this is the function name; for top-level code
    /// it comes from the `non_function_mir_context_stack`.
    pub(super) fn current_mir_context_name(&self) -> Option<&str> {
        // Try function context first (most common)
        if let Some(name) = self
            .current_function
            .and_then(|idx| self.program.functions.get(idx))
            .map(|f| f.name.as_str())
        {
            return Some(name);
        }
        // Fall back to non-function MIR context (top-level code)
        self.non_function_mir_context_stack.last().map(|s| s.as_str())
    }

    /// Query the MIR borrow analysis for the ownership decision at a given span.
    /// Returns `None` if MIR analysis isn't available for the current context,
    /// or if the span doesn't map to a known MIR program point.
    pub(super) fn query_ownership_decision(
        &self,
        span: &shape_ast::ast::Span,
    ) -> Option<crate::mir::analysis::OwnershipDecision> {
        let ctx = self.current_mir_context_name()?;
        let analysis = self.mir_borrow_analyses.get(ctx)?;
        let span_map = self.mir_span_to_point.get(ctx)?;
        let point = span_map.get(span)?;
        Some(analysis.ownership_at(*point))
    }

    /// Emit a local-variable load with ownership awareness.
    ///
    /// When MIR analysis is available and proves Move semantics, emits
    /// `LoadLocalMove`. When it proves Clone, emits `LoadLocalClone`.
    /// Otherwise falls back to `LoadLocal` (Copy types or no MIR info),
    /// preserving the existing behavior.
    ///
    /// This is intentionally conservative: the `query_ownership_decision`
    /// lookup can return `None` for many spans (e.g. compiler-generated
    /// code, pre-MIR contexts), and the fallback is always safe.
    ///
    /// Phase V1.1C: when the process-level `SHAPE_V2_OWNERSHIP_MOVES` flag
    /// is on AND the slot's MIR storage class is `UniqueHeap` (owned
    /// heap-ref), emit the leaner V1.1A/B `CloneLocal` opcode instead of
    /// `LoadLocal`/`LoadLocalMove`/`LoadLocalClone`. `CloneLocal` routes
    /// through `raw_helpers::clone_raw_bits`, which handles inline scalars,
    /// shared Arc refs, and owned Box refs in a single dispatch — avoiding
    /// the `LoadLocalMove`/`LoadLocalClone` SharedCell fallback (not
    /// reachable for a `UniqueHeap` slot by construction). We emit
    /// `CloneLocal` rather than `MoveLocal` as the conservative default;
    /// last-use information is not yet threaded through this emission
    /// path, so treating every read as a clone is the safe baseline.
    /// TODO(V1.1D prerequisite): once MIR `OwnershipDecision::Move` is
    /// re-consulted here the compiler should emit `MoveLocal` on the last
    /// use and `DropLocal` tracking must skip moved-out slots.
    ///
    /// For inline-scalar (`Direct`) bindings, heap-shared (`SharedCow`)
    /// bindings, and references we fall through to the existing emission
    /// path unchanged — the new opcodes target owned heap allocations
    /// only.
    ///
    /// Flag off: emission is byte-identical to pre-V1.1C.
    pub(super) fn emit_load_local_owned(
        &mut self,
        slot: u16,
        span: &shape_ast::ast::Span,
    ) {
        use crate::bytecode::{Instruction, OpCode, Operand};
        use crate::mir::analysis::OwnershipDecision;

        // Phase V1.1C gate. Flag off ⇒ byte-identical pre-V1.1C emission.
        //
        // Boxed-slot bailout: when the slot appears in `self.boxed_locals`,
        // a prior legacy cell-wrapping emission has replaced the slot's
        // inline value with a `SharedCell`-wrapped heap pointer (see
        // `expressions/closures.rs`). The `LoadLocal` legacy path auto-
        // unwraps that cell via `raw_helpers::extract_shared_cell`. The
        // V1.1C `CloneLocal` path reads the raw u64 bits and delegates to
        // `raw_helpers::clone_raw_bits`, which bumps the Arc on the cell
        // itself — it does not unwrap. Emitting `CloneLocal` on a boxed
        // slot therefore leaves a `shared_cell` ValueWord on the stack,
        // which later arithmetic / method dispatch rejects ("cannot apply
        // '+' to int and shared_cell"). Fall through to the existing
        // emission so the legacy unwrap path handles it.
        if super::helpers::ownership_moves_enabled()
            && self.slot_is_heap_backed_owned(slot)
            && !self.slot_is_boxed(slot)
        {
            self.emit(Instruction::new(
                OpCode::CloneLocal,
                Some(Operand::Local(slot)),
            ));
            return;
        }

        let decision = self.query_ownership_decision(span);

        let opcode = match decision {
            Some(OwnershipDecision::Move) => OpCode::LoadLocalMove,
            Some(OwnershipDecision::Clone) => OpCode::LoadLocalClone,
            _ => {
                // Copy types or no MIR info: use existing LoadLocal
                // (backward compatible, zero behavioral change)
                OpCode::LoadLocal
            }
        };

        self.emit(Instruction::new(opcode, Some(Operand::Local(slot))));
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

    /// Phase 5.C: Look up the current function's inferred return ownership
    /// mode, if any. Returns `None` when there is no MIR context (top-level
    /// code), when no summary was stored, or when the mode is `Unknown`.
    pub(super) fn current_function_return_ownership_mode(
        &self,
    ) -> Option<crate::mir::ReturnOwnershipMode> {
        let ctx = self.current_mir_context_name()?;
        let summary = self.function_borrow_summaries.get(ctx)?;
        match summary.return_ownership_mode {
            crate::mir::ReturnOwnershipMode::Unknown => None,
            mode => Some(mode),
        }
    }

    /// Phase 5.C: Emit the function return sequence. If the enclosing
    /// function's inferred return mode is `NewlyOwned`, emits a `ReturnOwned`
    /// immediately before the `ReturnValue` so the caller receives a
    /// Box-backed (unique) value and can skip its own `PromoteToOwned`.
    ///
    /// For any other mode (including `Unknown`, `BorrowedFromParam`,
    /// `Shared`, `Static`), emits the plain `ReturnValue` — today's behavior.
    pub(super) fn emit_return_value_with_ownership(&mut self) {
        use crate::bytecode::{Instruction, OpCode};
        if matches!(
            self.current_function_return_ownership_mode(),
            Some(crate::mir::ReturnOwnershipMode::NewlyOwned)
        ) {
            self.emit(Instruction::simple(OpCode::ReturnOwned));
        }
        self.emit(Instruction::simple(OpCode::ReturnValue));
    }

    /// Phase 5.B: If the initializer is a simple (non-qualified) call to a
    /// function whose return-ownership mode has been inferred, return that
    /// mode. Callers use it to populate `BindingSemantics::return_ownership_hint`.
    ///
    /// Intentionally conservative: returns `None` for method calls, qualified
    /// calls, indirect/callable-value calls, and calls into module-bound or
    /// closure-captured names (which may shadow or override the global
    /// summary). That matches the call-resolution precedence in
    /// `build_callee_summaries`.
    pub(super) fn return_ownership_hint_for_initializer(
        &self,
        expr: &Expr,
    ) -> Option<crate::mir::ReturnOwnershipMode> {
        let Expr::FunctionCall { name, .. } = expr else {
            return None;
        };
        // Shadowing rules: if the name resolves to a local, a captured name,
        // or a module binding, the call target may not be the module-level
        // function whose summary we stored. In any of those cases, skip.
        if self.resolve_local(name).is_some() {
            return None;
        }
        if self.mutable_closure_captures.contains_key(name.as_str()) {
            return None;
        }
        if self.resolve_scoped_module_binding_name(name).is_some() {
            return None;
        }
        let summary = self.function_borrow_summaries.get(name)?;
        match summary.return_ownership_mode {
            crate::mir::ReturnOwnershipMode::Unknown => None,
            mode => Some(mode),
        }
    }

    /// Apply a return-ownership hint to every pattern binding's semantics
    /// *without* overwriting the other fields. Must be called after
    /// `apply_binding_semantics_to_pattern_bindings`.
    pub(super) fn apply_return_ownership_hint_to_pattern_bindings(
        &mut self,
        pattern: &DestructurePattern,
        is_local: bool,
        hint: crate::mir::ReturnOwnershipMode,
    ) {
        for (name, _) in pattern.get_bindings() {
            if is_local {
                if let Some(local_idx) = self.resolve_local(&name) {
                    if let Some(mut sem) = self
                        .type_tracker
                        .get_local_binding_semantics(local_idx)
                        .copied()
                    {
                        sem.return_ownership_hint = Some(hint);
                        self.type_tracker
                            .set_local_binding_semantics(local_idx, sem);
                    }
                }
            } else {
                let scoped_name = self
                    .resolve_scoped_module_binding_name(&name)
                    .unwrap_or(name);
                if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
                    if let Some(mut sem) =
                        self.type_tracker.get_binding_semantics(binding_idx).copied()
                    {
                        sem.return_ownership_hint = Some(hint);
                        self.type_tracker
                            .set_binding_semantics(binding_idx, sem);
                    }
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
            // Phase D: `LocalMutablePtr` upgrades `Deferred` / `Direct` but
            // never overrides an already-shared binding (SharedCow / Reference
            // / UniqueHeap). `LocalMutablePtr` is strictly for non-escaping
            // bindings whose outer slot stays on the stack.
            LocalMutablePtr => match current {
                Deferred | Direct => LocalMutablePtr,
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

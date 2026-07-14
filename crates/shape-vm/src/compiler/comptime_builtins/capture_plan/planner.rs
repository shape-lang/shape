use super::*;
use crate::compiler::BytecodeCompiler;

impl BytecodeCompiler {
    /// Gather [`CaptureBindingFacts`] for one captured name, in the enclosing
    /// function's scope.
    fn capture_binding_facts(&self, name: &str, mutated: bool) -> CaptureBindingFacts {
        // ONE slot resolver, shared with the declared-clause set diff — so a
        // declared entry and the discovered capture it describes can never
        // resolve to different targets.
        let target = self.resolve_capture_target(name);

        let storage = match target {
            Some(CaptureTarget::Local(idx)) => self.mir_storage_class_for_slot(idx),
            _ => None,
        };

        CaptureBindingFacts {
            name: name.to_string(),
            target,
            ownership: self
                .binding_semantics_for_name(name)
                .map(|(_, _, sem)| sem.ownership_class),
            storage,
            mutated,
            boxed: self.boxed_locals.contains(name),
            witness_shared_local: self.shared_locals.contains(name),
            witness_shared_module_binding: self.shared_module_binding_contains(name),
            witness_owned_mutable_local: self.owned_mutable_locals.contains(name),
        }
    }

    /// THE producer. One call, one plan per capture, in `captured_vars` order.
    ///
    /// Called once per closure literal, before the closure body is compiled.
    /// Returns the facts alongside the plan so the escape veto (B0003) and the
    /// storage-promotion bookkeeping can read the same snapshot the selector
    /// saw, rather than re-deriving it from a compiler whose type tracker a
    /// nested `compile_function` may have moved on.
    ///
    /// ADR-009 C1 (slice 3) — ONE SELECTOR, TWO SOURCES OF TRUTH FOR *KIND*,
    /// never both at once:
    ///
    ///   * `declared == None` → [`infer_plan`] per capture (ordinary source).
    ///   * `declared == Some(clause)` → the clause is validated against
    ///     discovery (a SET DIFF OVER `CaptureTarget`, never over names — see
    ///     [`Self::validate_declared_clause`]) and then [`lower_declared`]
    ///     produces every plan. Inference does not get a vote on the kind; if
    ///     it did, `capture(x)` and no-clause would emit identical bytecode,
    ///     which is exactly the defect that got this ticket rejected once.
    pub(crate) fn plan_captures(
        &self,
        captured_vars: &[String],
        mutated_captures: &std::collections::HashSet<String>,
        declared: Option<&CaptureClause>,
        origin: Option<&GeneratedNodeOrigin>,
        closure_span: Span,
    ) -> Result<Vec<PlannedCapture>> {
        let facts: Vec<CaptureBindingFacts> = captured_vars
            .iter()
            .map(|name| self.capture_binding_facts(name, mutated_captures.contains(name)))
            .collect();

        let Some(clause) = declared else {
            return Ok(facts
                .into_iter()
                .map(|facts| {
                    let plan = infer_plan(&facts);
                    PlannedCapture {
                        facts,
                        plan,
                        declared: None,
                    }
                })
                .collect());
        };

        // Validate first — a clause that does not describe the discovered
        // capture set is an error BEFORE any lowering runs.
        let entry_for_target = self.validate_declared_clause(clause, &facts, origin, closure_span)?;

        facts
            .into_iter()
            .map(|facts| {
                // `validate_declared_clause` proved every discovered capture
                // resolves and has a matching entry; both unwraps are its
                // post-conditions.
                let target = facts.target.expect("validated: every capture resolves");
                let mode = *entry_for_target
                    .get(&target)
                    .expect("validated: every capture is declared");
                let plan = lower_declared(mode, &facts).map_err(|message| {
                    ShapeError::SemanticError {
                        message,
                        location: Some(self.span_to_source_location(closure_span)),
                    }
                })?;
                debug_assert_ne!(
                    plan.access(),
                    CaptureAccess::MutableCell,
                    "the inference residual is unreachable on the declared path"
                );
                Ok(PlannedCapture {
                    facts,
                    plan,
                    declared: Some(mode),
                })
            })
            .collect()
    }

    /// Resolve every clause entry to a compiler-issued [`CaptureTarget`] and
    /// diff the declared set against the discovered set.
    ///
    /// **The diff is over TARGETS, never over names.** This is the single
    /// easiest place for the design to rot: `EnvironmentAnalyzer` hands back
    /// `Vec<String>`, and a name-keyed comparison would silently mis-pair a
    /// shadowed binding. Both sets are mapped through the same slot resolver
    /// before they are compared.
    fn validate_declared_clause(
        &self,
        clause: &CaptureClause,
        facts: &[CaptureBindingFacts],
        origin: Option<&GeneratedNodeOrigin>,
        closure_span: Span,
    ) -> Result<std::collections::HashMap<CaptureTarget, CaptureMode>> {
        let reject = |message: String| ShapeError::SemanticError {
            message,
            location: Some(self.span_to_source_location(closure_span)),
        };

        // (i) every DECLARED entry resolves to a slot — [C0905].
        // (ii) no two entries name the same slot — [C0907].
        let mut entry_for_target: std::collections::HashMap<CaptureTarget, CaptureMode> =
            std::collections::HashMap::new();
        let mut declared_names: std::collections::HashMap<CaptureTarget, String> =
            std::collections::HashMap::new();
        for entry in &clause.entries {
            let Some(target) = self.resolve_capture_target(&entry.name) else {
                return Err(reject(format!(
                    "[C0905] declared capture '{} {}' does not resolve to a binding in the \
                     enclosing scope",
                    entry.mode.spelling(),
                    entry.name,
                )));
            };
            if let Some(previous) = declared_names.insert(target, entry.name.clone()) {
                return Err(reject(format!(
                    "[C0907] duplicate capture declaration for '{}'{}; each captured binding may \
                     be declared exactly once",
                    entry.name,
                    if previous == entry.name {
                        String::new()
                    } else {
                        format!(" (already declared as '{previous}')")
                    },
                )));
            }
            entry_for_target.insert(target, entry.mode);
        }

        // (iii) every DISCOVERED capture resolves to a slot — [C0905]. Slice 1
        //       left `CaptureBindingFacts.target` an `Option` because the
        //       pre-fusion selector had live `None` arms; the declared path
        //       rejects `None` by name rather than inheriting an `Immutable`
        //       fallback.
        let mut discovered: HashSet<CaptureTarget> = HashSet::new();
        for f in facts {
            let Some(target) = f.target else {
                return Err(reject(format!(
                    "[C0905] captured binding '{}' does not resolve to a frame local or a module \
                     binding; it cannot be declared",
                    f.name,
                )));
            };
            discovered.insert(target);
        }

        // (iv) THE SET DIFF, both directions.
        //
        // declared ∖ discovered — a declaration for something the body never
        // reads. Not a warning: a stale declaration is how a generated closure
        // silently keeps a capture alive after the body that used it changed.
        let mut unused: Vec<&str> = clause
            .entries
            .iter()
            .filter(|entry| {
                self.resolve_capture_target(&entry.name)
                    .is_none_or(|target| !discovered.contains(&target))
            })
            .map(|entry| entry.name.as_str())
            .collect();
        unused.sort_unstable();
        unused.dedup();
        if !unused.is_empty() {
            let names = unused
                .iter()
                .map(|n| format!("'{n}'"))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(reject(format!(
                "[C0901] declared capture {names} is never used by the closure body; remove the \
                 declaration"
            )));
        }

        // discovered ∖ declared — the Wave-46 used-but-undeclared error. The
        // message is the EXISTING implicit-capture message, verbatim: an
        // undeclared capture inside generated code IS an implicit capture,
        // whether the closure carried a partial clause or no clause at all.
        let mut undeclared: Vec<&str> = facts
            .iter()
            .filter(|f| {
                f.target
                    .is_none_or(|target| !entry_for_target.contains_key(&target))
            })
            .map(|f| f.name.as_str())
            .collect();
        undeclared.sort_unstable();
        if !undeclared.is_empty() {
            return Err(reject(implicit_capture_message(&undeclared, origin)));
        }

        Ok(entry_for_target)
    }

    /// The slot resolver both halves of the set diff run through.
    pub(crate) fn resolve_capture_target(&self, name: &str) -> Option<CaptureTarget> {
        if let Some(local_idx) = self.resolve_local(name) {
            return Some(CaptureTarget::Local(local_idx));
        }
        if let Some(scoped) = self.resolve_scoped_module_binding_name(name)
            && let Some(&idx) = self.module_bindings.get(&scoped)
        {
            return Some(CaptureTarget::ModuleBinding(idx));
        }
        self.module_bindings
            .get(name)
            .copied()
            .map(CaptureTarget::ModuleBinding)
    }

    /// THE `ClosureTypeId` producer — used by BOTH emission
    /// (`compile_expr_closure`) and the monomorphization pre-pass
    /// (`mint_closure_type_id_peek`).
    ///
    /// The id is the closure's layout identity. When every capture is a
    /// snapshot param the types-only intern is canonical; as soon as any
    /// capture is cell-backed the kinds must enter the key, or two closures
    /// with identical capture TYPES but different capture KINDS collide.
    ///
    /// Both call sites route through here so the id the mono cache is keyed on
    /// is the id the emitted closure carries. Before slice 3 the peek was
    /// unconditionally types-only, so it diverged from emission for every
    /// cell-backed capture; the DECLARED path makes that divergence load-bearing
    /// (`move hits` over a read-only `let mut` is precisely the case where
    /// inference says all-Immutable and the declaration says `OwnedMutable`).
    pub(crate) fn intern_closure_type_id_for_pack(
        &mut self,
        pack: &CapturePack,
    ) -> shape_value::v2::concrete_type::ClosureTypeId {
        // The pack's `capture_type`s are THE types the emitted `ClosureLayout`
        // is built from (`compiler_impl_reference_model.rs`), so the interned
        // id and the layout can never be keyed on different types.
        let capture_types: Vec<ConcreteType> = pack
            .descriptors
            .iter()
            .map(|d| d.capture_type.clone())
            .collect();
        if pack.any_cell_backed() {
            self.closure_registry
                .intern_with_kinds(capture_types, pack.kinds())
        } else {
            self.closure_registry.intern(capture_types)
        }
    }

    /// Build the closure's [`CapturePack`] from the plan. Stamps each
    /// capture's resolved `ConcreteType` — the same type the layout is built
    /// from — so the pack is a faithful model of the emitted artifact.
    pub(crate) fn build_capture_pack(
        &mut self,
        func_idx: u16,
        plan: &[PlannedCapture],
        origin: Option<&GeneratedNodeOrigin>,
    ) -> CapturePack {
        let descriptors = plan
            .iter()
            .enumerate()
            .map(|(i, planned)| {
                let capture_type = self.resolve_capture_concrete_type(&planned.facts.name);
                CaptureDescriptor {
                    index: i as u16,
                    target: planned.facts.target,
                    capture_type,
                    declared: planned.declared,
                    lowered: planned.plan.kind(),
                    access: planned.plan.access(),
                    ownership: planned.facts.ownership,
                    storage: planned.facts.storage,
                    name: planned.facts.name.clone(),
                }
            })
            .collect();
        CapturePack {
            closure: func_idx,
            origin: origin.cloned(),
            descriptors,
        }
    }
}

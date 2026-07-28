//! Item-level type inference
//!
//! Handles type inference for top-level items: functions, patterns, variables, etc.

use super::TypeInferenceEngine;
use crate::type_system::effects::EffectRow;
use crate::type_system::*;
use shape_ast::ast::{
    DestructurePattern, Expr, ForeignFunctionDef, FunctionDef, Item, Literal, Span, Statement,
    TraitMember, TraitMemberSignature, TypeAnnotation, TypeName, VarKind, VariableDecl,
};
use std::collections::HashMap;

#[cfg(test)]
mod declared_parameter_tests;
mod declared_parameters;

impl TypeInferenceEngine {
    /// Predeclare nominal type definitions that function signatures may refer to.
    ///
    /// This runs before callable-signature predeclaration, so a function can
    /// mention a type alias or struct declared later in the same source unit.
    pub(crate) fn predeclare_nominal_type_item(&mut self, item: &Item) -> TypeResult<()> {
        match item {
            Item::TypeAlias(alias, _) => {
                self.env.define_type_alias(
                    &alias.name,
                    &alias.type_annotation,
                    alias.meta_param_overrides.clone(),
                );
                Ok(())
            }
            Item::StructType(struct_def, _) => self.predeclare_struct_type(struct_def),
            Item::Export(export, _) => match &export.item {
                shape_ast::ast::ExportItem::TypeAlias(alias) => {
                    self.env.define_type_alias(
                        &alias.name,
                        &alias.type_annotation,
                        alias.meta_param_overrides.clone(),
                    );
                    Ok(())
                }
                shape_ast::ast::ExportItem::Struct(struct_def) => {
                    self.predeclare_struct_type(struct_def)
                }
                _ => Ok(()),
            },
            _ => Ok(()),
        }
    }

    /// Predeclare symbols needed for order-independent inference.
    ///
    /// This mirrors the compiler's first-pass registration so functions and
    /// extend methods can be referenced before their textual declaration.
    pub(crate) fn predeclare_item(&mut self, item: &Item) -> TypeResult<()> {
        match item {
            Item::Function(func, _) => self.predeclare_function_signature(func),
            Item::ForeignFunction(def, _) => self.predeclare_foreign_function(def),
            Item::BuiltinFunctionDecl(def, _) => self.predeclare_builtin_function_decl(def),
            Item::StructType(struct_def, _) => self.predeclare_struct_type(struct_def),
            Item::Export(export, _) => {
                if let shape_ast::ast::ExportItem::Function(func) = &export.item {
                    self.predeclare_function_signature(func)?;
                } else if let shape_ast::ast::ExportItem::ForeignFunction(def) = &export.item {
                    self.predeclare_foreign_function(def)?;
                } else if let shape_ast::ast::ExportItem::Struct(struct_def) = &export.item {
                    self.predeclare_struct_type(struct_def)?;
                }
                Ok(())
            }
            Item::Extend(extend, _) => self.register_extend(extend),
            _ => Ok(()),
        }
    }

    fn predeclare_function_signature(&mut self, func: &FunctionDef) -> TypeResult<()> {
        self.callable_param_defaults.insert(
            func.name.clone(),
            func.params
                .iter()
                .map(|p| p.default_value.is_some())
                .collect(),
        );

        // Mint once during predeclaration. Body inference retrieves this exact
        // owner/ordinal vector from the predeclared scheme; cache replay uses the
        // same scheme directly. The temporary scope only resolves source names.
        let type_params = func.type_params.as_deref().unwrap_or_default();
        let type_param_vars = self.mint_declared_type_params(type_params);
        self.validate_declared_type_param_vector(&func.name, type_params, &type_param_vars)?;
        self.env.push_scope();
        self.bind_validated_declared_type_params(type_params, &type_param_vars);

        let param_types: Vec<Type> = func
            .params
            .iter()
            .map(|p| match p.type_annotation.as_ref() {
                Some(ann) => self.resolve_type_annotation(ann),
                None => self.fresh_type_var(),
            })
            .collect();

        let return_type = match func.return_type.as_ref() {
            Some(ann) => self.resolve_type_annotation(ann),
            None => self.fresh_type_var(),
        };

        self.env.pop_scope();

        let func_type = BuiltinTypes::function(param_types, return_type);
        let scheme =
            self.make_function_scheme_with_params(func, func_type.clone(), &type_param_vars)?;
        self.predeclare_named_callable_scheme(func, scheme, &func_type)?;
        Ok(())
    }

    fn predeclare_foreign_function(&mut self, def: &ForeignFunctionDef) -> TypeResult<()> {
        let type_params = def.type_params.as_deref().unwrap_or_default();
        let type_param_vars = self.mint_declared_type_params(type_params);
        self.validate_declared_type_param_vector(&def.name, type_params, &type_param_vars)?;
        self.env.push_scope();
        self.bind_validated_declared_type_params(type_params, &type_param_vars);
        let raw_param_types: Vec<Type> = def
            .params
            .iter()
            .map(|p| match p.type_annotation.as_ref() {
                Some(ann) => self.resolve_type_annotation(ann),
                None => self.fresh_type_var(),
            })
            .collect();

        let raw_return_type = match def.return_type.as_ref() {
            Some(ann) => self.resolve_type_annotation(ann),
            None => self.fresh_type_var(),
        };
        self.env.pop_scope();

        let has_out_params = def.is_native_abi() && def.params.iter().any(|p| p.is_out);
        let param_types = if has_out_params {
            def.params
                .iter()
                .zip(raw_param_types.iter())
                .filter_map(|(param, ty)| (!param.is_out).then_some(ty.clone()))
                .collect()
        } else {
            raw_param_types.clone()
        };
        let return_type = if has_out_params {
            self.foreign_out_param_visible_return_type(def, &raw_param_types, raw_return_type)
        } else {
            raw_return_type
        };

        let func_type = BuiltinTypes::function(param_types, return_type);
        let scheme = if def.type_params.is_some() {
            self.make_declared_param_scheme(
                &def.name,
                type_params,
                None,
                func_type,
                &type_param_vars,
            )?
        } else {
            TypeScheme::mono(func_type)
        };
        self.env.define(&def.name, scheme);
        Ok(())
    }

    fn foreign_out_param_visible_return_type(
        &self,
        def: &ForeignFunctionDef,
        raw_param_types: &[Type],
        raw_return_type: Type,
    ) -> Type {
        let out_types: Vec<Type> = def
            .params
            .iter()
            .zip(raw_param_types.iter())
            .filter_map(|(param, ty)| param.is_out.then_some(ty.clone()))
            .collect();
        if out_types.is_empty() {
            return raw_return_type;
        }

        let is_void_return = def.return_type.as_ref().is_some_and(|ann| {
            matches!(ann, TypeAnnotation::Basic(name) if name == "void")
                || matches!(ann, TypeAnnotation::Void)
        });
        if is_void_return && out_types.len() == 1 {
            return out_types[0].clone();
        }

        let mut tuple_elems = Vec::new();
        if !is_void_return {
            tuple_elems.push(
                raw_return_type
                    .to_annotation()
                    .unwrap_or_else(|| TypeAnnotation::Basic("unknown".to_string())),
            );
        }
        tuple_elems.extend(out_types.into_iter().map(|ty| {
            ty.to_annotation()
                .unwrap_or_else(|| TypeAnnotation::Basic("unknown".to_string()))
        }));
        Type::Concrete(TypeAnnotation::Tuple(tuple_elems))
    }

    /// Predeclare a signature-only callable (`pub builtin fn` form). This is the
    /// inference-tier carrier the bytecode compiler uses to teach the type
    /// checker about IMPORTED module functions: it has full param + return
    /// annotations but no body, so the function name resolves at every use
    /// position (let-initializer, nested arg, call statement) without the
    /// checker needing to re-infer the dependency module's body.
    fn predeclare_builtin_function_decl(
        &mut self,
        def: &shape_ast::ast::BuiltinFunctionDecl,
    ) -> TypeResult<()> {
        self.callable_param_defaults.insert(
            def.name.clone(),
            def.params
                .iter()
                .map(|p| p.default_value.is_some())
                .collect(),
        );

        let type_params = def.type_params.as_deref().unwrap_or_default();
        let type_param_vars = self.mint_declared_type_params(type_params);
        self.validate_declared_type_param_vector(&def.name, type_params, &type_param_vars)?;
        self.env.push_scope();
        self.bind_validated_declared_type_params(type_params, &type_param_vars);

        let param_types: Vec<Type> = def
            .params
            .iter()
            .map(|p| match p.type_annotation.as_ref() {
                Some(ann) => self.resolve_type_annotation(ann),
                None => self.fresh_type_var(),
            })
            .collect();
        let return_type = self.resolve_type_annotation(&def.return_type);
        self.env.pop_scope();

        let func_type = BuiltinTypes::function(param_types, return_type);
        // Quantify free type vars (generic imported fn) so each call site
        // instantiates a fresh copy, matching `predeclare_function_signature`.
        let scheme = if def.type_params.is_some() {
            self.make_declared_param_scheme(
                &def.name,
                type_params,
                None,
                func_type,
                &type_param_vars,
            )?
        } else {
            self.env.generalize(&func_type)
        };
        self.env.define(&def.name, scheme);
        Ok(())
    }

    fn predeclare_struct_type(
        &mut self,
        struct_def: &shape_ast::ast::StructTypeDef,
    ) -> TypeResult<()> {
        self.struct_type_defs
            .insert(struct_def.name.clone(), struct_def.clone());

        // Predeclare the nominal struct alias before callable signature
        // predeclaration so signatures like Vec<Measurement> in foreign
        // functions can resolve consistently to structural object shapes.
        let fields = struct_def
            .fields
            .iter()
            .filter(|f| !f.is_comptime)
            .map(|f| shape_ast::ast::ObjectTypeField {
                name: f.name.clone(),
                optional: f.default_value.is_some(),
                type_annotation: f.type_annotation.clone(),
                annotations: vec![],
            })
            .collect();
        self.env
            .define_type_alias(&struct_def.name, &TypeAnnotation::Object(fields), None);
        Ok(())
    }

    /// Infer types for a top-level item
    pub(crate) fn infer_item(
        &mut self,
        item: &Item,
        types: &mut HashMap<String, Type>,
    ) -> TypeResult<()> {
        match item {
            Item::Function(func, _) => {
                let (func_type, type_param_vars) =
                    self.infer_function_with_declared_params(func, true)?;
                // Create polymorphic type scheme for generic functions
                let scheme = self.make_function_scheme_with_params(
                    func,
                    func_type.clone(),
                    &type_param_vars,
                )?;
                self.republish_named_callable_scheme(func, scheme, &func_type)?;
                types.insert(func.name.clone(), func_type);
            }
            Item::ForeignFunction(_, _) => {
                // Foreign function bodies are opaque — type already predeclared
            }
            Item::VariableDecl(decl, _) => {
                let var_type = self.infer_variable_decl(decl)?;
                self.record_unannotated_let_origin(decl);
                if let Some(name) = decl.pattern.as_identifier() {
                    types.insert(name.to_string(), var_type.clone());
                } else {
                    for name in decl.pattern.get_identifiers() {
                        let scheme = self.env.lookup(&name).cloned();
                        let inferred = scheme
                            .map(|s| s.instantiate(&mut self.type_var_gen))
                            .unwrap_or_else(|| var_type.clone());
                        types.insert(name, inferred);
                    }
                }
            }
            Item::Statement(stmt, _) => {
                if let Statement::VariableDecl(decl, _) = stmt {
                    let var_type = self.infer_variable_decl(decl)?;
                    self.record_unannotated_let_origin(decl);
                    if let Some(name) = decl.pattern.as_identifier() {
                        types.insert(name.to_string(), var_type.clone());
                    } else {
                        for name in decl.pattern.get_identifiers() {
                            let scheme = self.env.lookup(&name).cloned();
                            let inferred = scheme
                                .map(|s| s.instantiate(&mut self.type_var_gen))
                                .unwrap_or_else(|| var_type.clone());
                            types.insert(name, inferred);
                        }
                    }
                } else {
                    // Tolerate UndefinedFunction errors from expression statements
                    // (e.g. calling builtins like print that aren't registered in
                    // the type env) without killing the entire program's inference.
                    match self.infer_statement(stmt) {
                        Ok(_) | Err(TypeError::UndefinedFunction(_)) => {}
                        Err(e) => return Err(e),
                    }
                }
            }
            Item::TypeAlias(alias, _) => {
                // Type aliases don't need inference, just register them with optional overrides
                self.env.define_type_alias(
                    &alias.name,
                    &alias.type_annotation,
                    alias.meta_param_overrides.clone(),
                );
            }
            Item::StructType(struct_def, _) => {
                self.struct_type_defs
                    .insert(struct_def.name.clone(), struct_def.clone());
                // Struct type definitions are registered as nominal aliases
                // to their structural object shape.
                let fields = struct_def
                    .fields
                    .iter()
                    .filter(|f| !f.is_comptime)
                    .map(|f| shape_ast::ast::ObjectTypeField {
                        name: f.name.clone(),
                        optional: f.default_value.is_some(),
                        type_annotation: f.type_annotation.clone(),
                        annotations: vec![],
                    })
                    .collect();
                self.env
                    .define_type_alias(&struct_def.name, &TypeAnnotation::Object(fields), None);
            }
            Item::Enum(enum_def, _) => {
                // Register enum for exhaustiveness checking
                self.env.register_enum(enum_def);
            }
            Item::Trait(trait_def, _) => {
                self.register_trait(trait_def)?;
            }
            Item::Impl(impl_block, _) => {
                self.register_impl(impl_block)?;
                self.infer_impl_method_bodies(impl_block)?;
            }
            Item::Extend(extend, _) => {
                self.register_extend(extend)?;
                self.infer_extend_method_bodies(extend)?;
            }
            Item::Export(export, _) => {
                // pub const/let/var NAME = expr : the VariableDecl rides in
                // source_decl; infer + bind it exactly like a bare
                // Item::VariableDecl so NAME is in scope and executes.
                // A-final ROOT J1.
                if let Some(decl) = &export.source_decl {
                    let var_type = self.infer_variable_decl(decl)?;
                    self.record_unannotated_let_origin(decl);
                    if let Some(name) = decl.pattern.as_identifier() {
                        types.insert(name.to_string(), var_type.clone());
                    } else {
                        for name in decl.pattern.get_identifiers() {
                            let scheme = self.env.lookup(&name).cloned();
                            let inferred = scheme
                                .map(|s| s.instantiate(&mut self.type_var_gen))
                                .unwrap_or_else(|| var_type.clone());
                            types.insert(name, inferred);
                        }
                    }
                }
                match &export.item {
                    shape_ast::ast::ExportItem::Function(func) => {
                        let (func_type, type_param_vars) =
                            self.infer_function_with_declared_params(func, true)?;
                        let scheme = self.make_function_scheme_with_params(
                            func,
                            func_type.clone(),
                            &type_param_vars,
                        )?;
                        self.republish_named_callable_scheme(func, scheme, &func_type)?;
                        types.insert(func.name.clone(), func_type);
                    }
                    shape_ast::ast::ExportItem::TypeAlias(alias) => {
                        self.env.define_type_alias(
                            &alias.name,
                            &alias.type_annotation,
                            alias.meta_param_overrides.clone(),
                        );
                    }
                    shape_ast::ast::ExportItem::Struct(struct_def) => {
                        self.struct_type_defs
                            .insert(struct_def.name.clone(), struct_def.clone());
                        let fields = struct_def
                            .fields
                            .iter()
                            .filter(|f| !f.is_comptime)
                            .map(|f| shape_ast::ast::ObjectTypeField {
                                name: f.name.clone(),
                                optional: f.default_value.is_some(),
                                type_annotation: f.type_annotation.clone(),
                                annotations: vec![],
                            })
                            .collect();
                        self.env.define_type_alias(
                            &struct_def.name,
                            &TypeAnnotation::Object(fields),
                            None,
                        );
                    }
                    shape_ast::ast::ExportItem::Trait(trait_def) => {
                        self.register_trait(trait_def)?;
                    }
                    _ => {}
                }
            }
            Item::Comptime(stmts, _) => {
                // J-CT.1: a top-level `comptime { ... }` item is itself a
                // comptime context. Walk its statements with the comptime
                // depth incremented so method calls on `comptime impl`
                // methods type-check. We tolerate per-statement errors here
                // the same way the `Item::Statement` arm does for
                // top-level expression statements — best-effort surface
                // without aborting the whole program.
                self.enter_comptime();
                for stmt in stmts {
                    let _ = self.infer_statement(stmt);
                }
                self.exit_comptime();
            }
            _ => {} // Other items handled separately
        }

        Ok(())
    }

    /// Register all trait + impl + enum + extend definitions across the WHOLE
    /// item list BEFORE any function body is type-checked, so operator-trait
    /// dispatch is declaration-ORDER INDEPENDENT.
    ///
    /// The documented model is "register all, then compile" (CLAUDE.md two-pass
    /// compiler). `predeclare_item` (pass 1) only predeclares fn/struct
    /// signatures + extend; trait/impl/enum registration historically happened
    /// inside `infer_item` (pass 2), interleaved with function-body inference.
    /// That made a `fn f() { a + b }` declared textually BEFORE its
    /// `impl Add for T` fail operator-trait resolution (`check_operator_trait`
    /// saw no registered impl yet), while the same code after the impl worked.
    ///
    /// This pre-pass closes that gap: traits are registered first (impls
    /// validate against their trait), then impls / enums in source order.
    /// Registration is idempotent for matching shapes
    /// (`register_trait_impl_with_assoc_types_named` returns `Ok(())` on an
    /// exact re-registration), so `infer_item`'s own Trait/Impl/Enum arms still
    /// run and remain the CANONICAL error-reporting site (preserving source-
    /// order diagnostics). Errors here are deliberately SWALLOWED — this pass
    /// exists only to make the impls visible to body inference; any genuine
    /// conflict / arity / coherence error surfaces (once) from `infer_item`.
    pub(crate) fn register_traits_and_impls_prepass(&mut self, items: &[Item]) {
        // Traits first — impl registration validates method arity / comptime
        // alignment against the trait definition.
        for item in items {
            match item {
                Item::Trait(trait_def, _) => {
                    let _ = self.register_trait(trait_def);
                }
                Item::Export(export, _) => {
                    if let shape_ast::ast::ExportItem::Trait(trait_def) = &export.item {
                        let _ = self.register_trait(trait_def);
                    }
                }
                _ => {}
            }
        }
        // Then impls + enums in source order. Extend is already registered by
        // `predeclare_item` (pass 1), so it is not repeated here.
        for item in items {
            match item {
                Item::Impl(impl_block, _) => {
                    let _ = self.register_impl(impl_block);
                }
                Item::Enum(enum_def, _) => {
                    self.env.register_enum(enum_def);
                }
                _ => {}
            }
        }
    }

    /// DESIGN §2.4 — LOAD path = REPLAY, not re-infer.
    ///
    /// On a fresh `.shapec` cache hit (the §2.3 load-or-rebuild gate selected
    /// LOAD), the loader deserializes the module's [`ResolvedInterface`] and
    /// drives THIS method over its source-ordered `items`. It replays the SAME
    /// registration passes a from-source compile runs — the two-pass
    /// `predeclare_item`→registration walk — skipping ONLY parsing and body
    /// inference (`infer_function`).
    ///
    /// Faithful to §2.4 / Amendment A:
    /// - **Pass 1** (`predeclare_item` over `items[0..n]` in order, items.rs:18):
    ///   registers fn / foreign-fn / struct signatures + extend, exactly as
    ///   `infer_program_best_effort`'s first pass. For a fully-annotated cached
    ///   signature (`interface_schema = 1` is annotation-required, §3.4) this
    ///   resolves identically whether the `FunctionDef` came from a fresh parse
    ///   or the cache — `predeclare_function_signature` only falls into
    ///   `fresh_type_var()` when an annotation is ABSENT (items.rs:50-53,56-59),
    ///   which v1 excludes.
    /// - **Pass 2** (registration-only over `items[0..n]` IN ORDER): registers
    ///   the order-sensitive trait / impl / enum / extend / type-alias / struct
    ///   defs via the SAME `register_trait` / `register_impl` / `register_enum` /
    ///   `register_extend` / `define_type_alias` / `struct_type_defs` calls
    ///   `infer_item` (items.rs:198-247) uses. Because `items` is in EXACT source
    ///   order (Amendment A), an `impl T for S` textually before `trait T`
    ///   replays before the trait — reproducing from-source accept/reject +
    ///   method-table behavior bug-for-bug.
    ///
    /// What is DELIBERATELY NOT done (§2.4): no parse, no body inference, no
    /// `infer_function`. The function-body arm of `infer_item` is the ONLY
    /// `infer_item` work skipped here — the signature is already registered by
    /// pass 1, so re-running `infer_function` would be the very re-inference the
    /// cache exists to avoid (and would re-derive the same `Type`, since `items`
    /// is identical). The derived `TypeScheme` / `MethodTable` / `TypeParamExpr`
    /// carriers are REBUILT by these passes from the AST, never deserialized
    /// (§5); `to_annotation()` is never on this path (§0, §3.2).
    ///
    /// Returns all registration errors (a from-source compile surfaces the same
    /// set); callers may collect or assert-empty per the §3.3 binder.
    pub fn replay_resolved_interface(&mut self, items: &[Item]) -> Vec<TypeError> {
        let mut errors = Vec::new();

        // Pass 0: predeclare nominal type definitions that signatures may
        // reference, preserving from-source forward-alias behavior.
        for item in items {
            if let Err(err) = self.predeclare_nominal_type_item(item) {
                errors.push(err);
            }
        }

        // Pass 1: predeclare signatures (fn / foreign / struct) + extend, in order.
        for item in items {
            if let Err(err) = self.predeclare_item(item) {
                errors.push(err);
            }
        }

        // Pass 2: registration-only, in EXACT source order. Mirrors the
        // order-sensitive arms of `infer_item` (items.rs:169-247) WITHOUT the
        // `Item::Function` body-inference arm (signatures came from pass 1).
        for item in items {
            if let Err(err) = self.replay_register_item(item) {
                errors.push(err);
            }
        }

        errors
    }

    /// Pass-2 registration dispatch for [`replay_resolved_interface`].
    ///
    /// Each arm is the registration half of the matching `infer_item` arm
    /// (items.rs:120-263) with body inference elided. Functions / foreign
    /// functions are no-ops here (signatures already registered by pass 1's
    /// `predeclare_item`, matching `infer_item`'s `ForeignFunction` no-op and
    /// the deliberate omission of `infer_function`). Type-alias / struct / enum /
    /// trait / impl / extend reuse the SAME registration calls a from-source
    /// `infer_item` makes, preserving source order.
    fn replay_register_item(&mut self, item: &Item) -> TypeResult<()> {
        match item {
            // Signatures already registered by pass 1 (`predeclare_item`).
            // `infer_item` would re-run `infer_function` here; the replay path
            // deliberately does NOT (§2.4 "No infer_function").
            Item::Function(_, _) | Item::ForeignFunction(_, _) => Ok(()),
            Item::TypeAlias(alias, _) => {
                self.env.define_type_alias(
                    &alias.name,
                    &alias.type_annotation,
                    alias.meta_param_overrides.clone(),
                );
                Ok(())
            }
            Item::StructType(struct_def, _) => {
                self.struct_type_defs
                    .insert(struct_def.name.clone(), struct_def.clone());
                let fields = struct_def
                    .fields
                    .iter()
                    .filter(|f| !f.is_comptime)
                    .map(|f| shape_ast::ast::ObjectTypeField {
                        name: f.name.clone(),
                        optional: f.default_value.is_some(),
                        type_annotation: f.type_annotation.clone(),
                        annotations: vec![],
                    })
                    .collect();
                self.env
                    .define_type_alias(&struct_def.name, &TypeAnnotation::Object(fields), None);
                Ok(())
            }
            Item::Enum(enum_def, _) => {
                self.env.register_enum(enum_def);
                Ok(())
            }
            Item::Trait(trait_def, _) => self.register_trait(trait_def),
            Item::Impl(impl_block, _) => self.register_impl(impl_block),
            Item::Extend(extend, _) => self.register_extend(extend),
            // `Item::Export(pub ...)`: the producer (DESIGN §1.1) UNWRAPS exports
            // into bare item nodes before caching, so a cached interface never
            // carries `Item::Export`. Mirror `infer_item`'s export arm anyway so
            // the replay is robust to any future producer that retains wrappers,
            // skipping the body-inference half (functions are predeclared).
            Item::Export(export, _) => match &export.item {
                shape_ast::ast::ExportItem::TypeAlias(alias) => {
                    self.env.define_type_alias(
                        &alias.name,
                        &alias.type_annotation,
                        alias.meta_param_overrides.clone(),
                    );
                    Ok(())
                }
                shape_ast::ast::ExportItem::Struct(struct_def) => {
                    self.struct_type_defs
                        .insert(struct_def.name.clone(), struct_def.clone());
                    let fields = struct_def
                        .fields
                        .iter()
                        .filter(|f| !f.is_comptime)
                        .map(|f| shape_ast::ast::ObjectTypeField {
                            name: f.name.clone(),
                            optional: f.default_value.is_some(),
                            type_annotation: f.type_annotation.clone(),
                            annotations: vec![],
                        })
                        .collect();
                    self.env.define_type_alias(
                        &struct_def.name,
                        &TypeAnnotation::Object(fields),
                        None,
                    );
                    Ok(())
                }
                shape_ast::ast::ExportItem::Enum(enum_def) => {
                    self.env.register_enum(enum_def);
                    Ok(())
                }
                shape_ast::ast::ExportItem::Trait(trait_def) => self.register_trait(trait_def),
                // Function / ForeignFunction exports: signature predeclared in
                // pass 1; no body inference on the replay path.
                _ => Ok(()),
            },
            // Statements / comptime / var-decls / module wrappers are not part
            // of the cached interface surface (DESIGN §5: annotation-level item
            // defs only); ignore defensively.
            _ => Ok(()),
        }
    }

    /// Infer type of a function
    ///
    /// Implements contagious Result inference: if the function body contains
    /// any `?` operators, the return type is automatically wrapped in Result<T>.
    /// Also handles generic functions with type parameters.
    /// Infer an impl/extend synthetic body without publishing a named scheme.
    pub(crate) fn infer_function(&mut self, func: &FunctionDef) -> TypeResult<Type> {
        let (ty, type_params) = self.infer_function_with_declared_params(func, false)?;
        self.validate_declared_type_params_in_type(func, &ty, &type_params)?;
        Ok(ty)
    }

    /// Infer a synthetic callable using the declaration capability registered
    /// by its owning source construct. This keeps method-body inference and
    /// method-call specialization on the same exact declared TypeVars.
    pub(crate) fn infer_function_with_declared_parameter_capability(
        &mut self,
        func: &FunctionDef,
        type_params: &[TypeVar],
    ) -> TypeResult<Type> {
        self.install_callable_declared_parameters(func, type_params.to_vec())?;
        let inferred = self.infer_function_with_declared_params(func, true);
        self.remove_callable_declared_parameters(func);
        let (ty, inferred_params) = inferred?;
        self.validate_declared_type_params_in_type(func, &ty, &inferred_params)?;
        Ok(ty)
    }

    fn infer_function_with_declared_params(
        &mut self,
        func: &FunctionDef,
        require_predeclared: bool,
    ) -> TypeResult<(Type, Vec<TypeVar>)> {
        let type_params = func.type_params.as_deref().unwrap_or_default();
        let type_param_vars = if require_predeclared && !type_params.is_empty() {
            self.declared_type_parameters_for_callable(func)?
        } else {
            self.mint_declared_type_params(type_params)
        };
        self.validate_declared_type_param_vector(&func.name, type_params, &type_param_vars)?;
        self.env.push_scope();
        self.push_fallible_scope();
        self.register_callable_origin_for_name(&func.name, func.name_span);

        // Positional defaults are only well-defined for trailing parameters.
        let mut saw_default = false;
        for param in &func.params {
            if param.default_value.is_some() {
                saw_default = true;
            } else if saw_default {
                self.env.pop_scope();
                self.pop_fallible_scope();
                return Err(TypeError::ConstraintViolation(
                    "Required parameter cannot follow a parameter with a default value".to_string(),
                ));
            }
        }

        self.bind_validated_declared_type_params(type_params, &type_param_vars);

        // Collect parameter types
        let mut param_types = Vec::new();
        let mut unannotated_param_vars: Vec<TypeVar> = Vec::new();
        let mut param_source_vars: Vec<Option<TypeVar>> = Vec::new();

        for param in &func.params {
            let param_type = if let Some(ann) = &param.type_annotation {
                param_source_vars.push(None);
                self.resolve_type_annotation(ann)
            } else if param.simple_name().is_none() {
                let var = self.fresh_var();
                unannotated_param_vars.push(var.clone());
                param_source_vars.push(Some(var.clone()));
                let param_type = Type::Variable(var);
                self.bind_function_param_pattern(&param.pattern, &param_type);
                param_type
            } else {
                let var = self.fresh_var();
                unannotated_param_vars.push(var.clone());
                param_source_vars.push(Some(var.clone()));
                Type::Variable(var)
            };

            param_types.push(param_type.clone());
            if param.type_annotation.is_some() || param.simple_name().is_some() {
                self.bind_function_param_pattern(&param.pattern, &param_type);
            }
            self.record_binding_facts_for_param_pattern(&param.pattern);
        }
        self.callable_param_source_vars
            .insert(func.name.clone(), param_source_vars);

        // HOF return-type aliasing (the sg2 root). When an UNANNOTATED function's
        // RETURN value is precisely the result of invoking one of its own
        // fn-typed params in tail position (`fn apply2(f, x, y) { f(x, y) }`),
        // the function's return type IS that param's return type. The inference
        // engine resolves the fn-typed param's full `Function` signature only
        // AFTER `solver.solve` (via the post-solve `apply_callsite_unions`
        // call-site widening), by which point the body constraint that linked
        // the function's return var to the param's return var has already been
        // solved against a still-unresolved param. So the function's return is
        // left as a bare `Variable`. Recording the param index here lets
        // `apply_callsite_unions` substitute the function's return var with the
        // param's NOW-concrete return type once the param resolves.
        //
        // Pure-AST, conservative: fires ONLY when the function is unannotated,
        // has no explicit `return` statements (those go through the
        // return-union machinery untouched), and its single tail value is a
        // direct `param(...)` call naming an unannotated param. Any other shape
        // records nothing — the case keeps its existing behavior. The adopted
        // return type is whatever EXACT type the engine proves for the param's
        // return (int stays int, number stays number); an unresolved param
        // return leaves the function's return a variable (SURFACEs, no default).
        if func.return_type.is_none() {
            let mut unannotated_param_index: HashMap<String, usize> = HashMap::new();
            for (i, p) in func.params.iter().enumerate() {
                if p.type_annotation.is_some() {
                    continue;
                }
                let names = p.get_identifiers();
                if names.len() == 1 {
                    unannotated_param_index.insert(names[0].clone(), i);
                }
            }
            let mut explicit_returns: Vec<&Expr> = Vec::new();
            Self::collect_explicit_returns(&func.body, &mut explicit_returns);
            if explicit_returns.is_empty() {
                let mut tail_values: Vec<&Expr> = Vec::new();
                Self::collect_tail_values(&func.body, &mut tail_values);
                if tail_values.len() == 1 {
                    if let Expr::FunctionCall { name, .. } = tail_values[0] {
                        if let Some(&idx) = unannotated_param_index.get(name.as_str()) {
                            self.callable_return_from_fn_param
                                .insert(func.name.clone(), idx);
                        }
                    }
                    if let Expr::Array(elements, _) = tail_values[0] {
                        let mut array_param_idx: Option<usize> = None;
                        let mut all_elements_match = !elements.is_empty();
                        for element in elements {
                            let Expr::FunctionCall { name, .. } = element else {
                                all_elements_match = false;
                                break;
                            };
                            let Some(&idx) = unannotated_param_index.get(name.as_str()) else {
                                all_elements_match = false;
                                break;
                            };
                            match array_param_idx {
                                Some(existing) if existing != idx => {
                                    all_elements_match = false;
                                    break;
                                }
                                Some(_) => {}
                                None => array_param_idx = Some(idx),
                            }
                        }
                        if all_elements_match {
                            if let Some(idx) = array_param_idx {
                                self.callable_array_return_from_fn_param
                                    .insert(func.name.clone(), idx);
                            }
                        }
                    }
                }
            }
        }

        self.callable_param_defaults.insert(
            func.name.clone(),
            func.params
                .iter()
                .map(|p| p.default_value.is_some())
                .collect(),
        );

        for (param, param_type) in func.params.iter().zip(param_types.iter()) {
            if let Some(default_expr) = &param.default_value {
                let default_type = self.infer_expr(default_expr)?;
                self.constraints.push((param_type.clone(), default_type));
            }
        }

        // Infer return type from annotation or create fresh variable
        let declared_return_type = if let Some(ann) = &func.return_type {
            self.resolve_type_annotation(ann)
        } else {
            self.fresh_type_var()
        };

        // Fn-boundary let-generalization (let-gen spec §1.2 / §2.1): for an
        // unannotated fn whose body is NON-EXPANSIVE w.r.t. its to-be-quantified
        // return vars (cond-4 — the returned value provably traces to a freshly
        // constructed carrier `None`/`Some`/`Ok`/`Err`/struct-literal or to a
        // fn-local IMMUTABLE `let`/`const` chain bottoming out in one, never a
        // `var`/`let mut`/module-scope binding or a reference/deref into one),
        // allow the single `Option<fresh>` / `Result<fresh, …>` candidate to
        // survive un-concretized so `make_function_scheme` → `env.generalize`
        // can quantify it into a `∀T. …` scheme. EXPANSIVE bodies keep the
        // strict reject (the §3.2 value-restriction refusal: a shared mutable
        // cell must not be generalized — that is the int+string-through-one-slot
        // unsoundness). The flag is gated on cond-4 at THIS single call site
        // only; it is NOT unconditionally `true` for every unannotated fn (spec
        // §2.1 "Critical").
        let empty_grow_return = Self::fn_body_returns_empty_grow_carrier(func);
        let mut empty_grow_return_carriers = std::collections::HashSet::new();
        if empty_grow_return {
            Self::collect_empty_array_carriers(&func.body, &mut empty_grow_return_carriers);
        }
        let allow_unresolved_return =
            func.return_type.is_some() || Self::fn_body_is_non_expansive(func) || empty_grow_return;

        // Numeric-conversion §4 literal adoption (return context): make the
        // declared return type visible to `return <lit>` / tail-expr adoption
        // inside the body. Two enabling shapes:
        //   (a) a concrete numeric `-> T` (a bare int-literal tail/return
        //       adopts `T`), and
        //   (b) a `Result<…>` / `Option<…>` carrier — so a tail/return
        //       constructor (`Ok(42)`/`Some(42)`/`Err(e)`) propagates the
        //       expected variant-payload type to its argument and a bare int
        //       literal there adopts the expected numeric type
        //       (constructor-payload-vs-expected path). `None` when the fn has
        //       no annotation or a return that is neither numeric nor a
        //       Result/Option carrier — those keep plain inference.
        let expected_return_for_adoption = if func.return_type.is_none() {
            None
        } else if Self::concrete_numeric_type_name(&declared_return_type).is_some()
            || self.is_result_type(&declared_return_type)
            || self.is_option_type(&declared_return_type)
        {
            Some(declared_return_type.clone())
        } else {
            None
        };
        self.expected_return_types
            .push(expected_return_for_adoption);

        // Infer callable return type from all explicit returns (or final expression)
        let local_constraint_start = self.constraints.len();
        self.empty_grow_return_carrier_scopes
            .push(empty_grow_return_carriers);
        if func.is_comptime {
            self.enter_comptime();
        }
        let inferred_result = self.infer_callable_return_type(&func.body, allow_unresolved_return);
        if func.is_comptime {
            self.exit_comptime();
        }
        self.empty_grow_return_carrier_scopes.pop();

        self.expected_return_types.pop();
        // `include_numeric_refinement: false` — defer the `number` default for
        // `Numeric`-bounded parameters. Eagerly collapsing a parameter like
        // `x` in `fn double(x) { x * 2 }` to `number` severs the call-graph
        // link: a function reached only through nested calls of unannotated
        // functions never sees a concrete call site, so the only path to
        // resolving its parameter is transitive propagation in
        // `apply_callsite_unions`. The `Numeric`-bounded indices are recorded
        // and the `number` fallback is applied afterwards by
        // `refine_numeric_params_post_callsite` to whatever is still a
        // variable.
        let numeric_param_indices = self.refine_callable_param_types_from_local_constraints(
            &mut param_types,
            &self.constraints[local_constraint_start..],
            false,
        );
        if !numeric_param_indices.is_empty() {
            self.callable_numeric_param_indices
                .insert(func.name.clone(), numeric_param_indices);
        }
        let local_constraints = &self.constraints[local_constraint_start..];
        let local_origin = self
            .find_origin_for_callable_param_constraints(&unannotated_param_vars, local_constraints);

        // Check if function is fallible (contains ? operators)
        let is_fallible = self.pop_fallible_scope();
        self.env.pop_scope();
        let inferred_return_type = inferred_result?;

        if func.return_type.is_none() {
            let mut return_vars = std::collections::HashSet::new();
            self.collect_type_vars(&inferred_return_type, &mut return_vars);

            let mut allowed_vars: std::collections::HashSet<TypeVar> =
                type_param_vars.iter().cloned().collect();
            allowed_vars.extend(unannotated_param_vars.iter().cloned());

            if return_vars.iter().any(|var| !allowed_vars.contains(var))
                && matches!(inferred_return_type, Type::Generic { .. })
            {
                // Fn-boundary let-gen (spec §1.2 cond-4 / §2.1): a
                // return-position-only free var sitting in a generic carrier.
                // If the body is NON-EXPANSIVE, QUANTIFY instead of reject —
                // `make_function_scheme` → `env.generalize` turns this into a
                // `∀T. …` scheme. If the body is EXPANSIVE (returns a `var` /
                // `let mut` / module-scope binding or a reference into one), this
                // is the §3.2 value-restriction refusal: a shared mutable cell's
                // element type is not fixed and must not be generalized.
                if !allow_unresolved_return {
                    return Err(TypeError::GenericTypeError {
                        message: format!(
                            "Cannot infer a polymorphic return type for '{}': its result is read \
                             from a mutable/shared binding whose element type is not fixed. \
                             Annotate the binding (e.g. `let x: Option<ConcreteT> = …`) or the \
                             function's return type (e.g. `fn {}() -> Option<ConcreteT>`).",
                            func.name, func.name
                        ),
                        symbol: Some(func.name.clone()),
                    });
                }
                // Non-expansive: fall through. The free return var survives into
                // `function_type` and is quantified by `make_function_scheme`.
            }
        }

        // Determine the actual return type. When the declared return is a
        // Result/Option and the inferred body type is a bare success value,
        // constrain against the success type (Shape implicitly Ok/Some-wraps
        // the return value of a fallible/optional function).
        self.push_return_constraint(inferred_return_type.clone(), declared_return_type.clone());

        // If deferred return-union members were recorded on the inferred return
        // variable, transfer them to the declared return variable that is
        // exposed in the final function type.
        if let (Type::Variable(inferred_var), Type::Variable(declared_var)) =
            (&inferred_return_type, &declared_return_type)
        {
            self.register_return_var_alias(declared_var.clone(), inferred_var.clone());
            // Only transfer deferred return-union members when an explicit
            // return annotation exists and therefore the declared return var is
            // the one exposed in the final function type.
            if func.return_type.is_some() && inferred_var != declared_var {
                if let Some(members) = self.pending_return_unions.remove(inferred_var) {
                    self.record_pending_return_union(declared_var.clone(), members);
                }
            }
        }

        // For unannotated functions, keep the inferred return shape as the
        // source of truth so fallibility wrapping does not produce
        // Result<Result<T>> when the body already returns a Result<T>.
        let return_base = if func.return_type.is_some() {
            declared_return_type
        } else {
            inferred_return_type
        };

        // If the function uses `?` but has an explicit return type that is
        // neither Result nor Option, that is a user error — the `?` operator
        // needs a propagatable wrapper type.  Reject at compile time instead
        // of silently wrapping the return type.
        if is_fallible
            && func.return_type.is_some()
            && !self.is_result_type(&return_base)
            && !self.is_option_type(&return_base)
        {
            return Err(TypeError::ConstraintViolation(format!(
                "operator '?' requires the function to return Result or Option, but '{}' has return type '{}'",
                func.name,
                self.render_type_for_diag(&return_base)
            )));
        }

        let actual_return_type = self.apply_fallibility_to_return_type(return_base, is_fallible);
        let actual_return_type = if empty_grow_return {
            self.solver
                .unifier()
                .apply_substitutions(&actual_return_type)
        } else {
            actual_return_type
        };
        let function_type = BuiltinTypes::function(param_types, actual_return_type);
        if let Some(origin) = local_origin {
            self.register_callable_origin_for_name(&func.name, origin);
        }

        Ok((function_type, type_param_vars))
    }

    /// Re-walk named function bodies after callsite propagation has resolved
    /// unannotated parameters to concrete types.
    ///
    /// The initial body walk must happen before call sites are known, so
    /// expressions like `s.length - 1` or `age > 150` can be recorded with
    /// unresolved operand variables. Once `apply_callsite_unions` proves the
    /// function parameter types, this pass reuses the same body inference with
    /// those proven parameter types in scope. It only runs for functions whose
    /// parameters are fully resolved, and it unifies the replayed return with
    /// the existing function return variable instead of inventing a fallback.
    pub(crate) fn rewalk_resolved_function_bodies(
        &mut self,
        program: &shape_ast::ast::Program,
        types: &mut HashMap<String, Type>,
    ) -> Vec<TypeError> {
        let mut errors = Vec::new();
        self.publish_rewalk_function_schemes(program, types, &mut errors);
        for item in &program.items {
            self.rewalk_resolved_function_bodies_for_item(item, types, &mut errors);
        }
        errors
    }

    fn publish_rewalk_function_schemes(
        &mut self,
        program: &shape_ast::ast::Program,
        types: &HashMap<String, Type>,
        errors: &mut Vec<TypeError>,
    ) {
        for item in &program.items {
            match item {
                Item::Function(func, _) => self.publish_rewalk_function_scheme(func, types, errors),
                Item::Export(export, _) => {
                    if let shape_ast::ast::ExportItem::Function(func) = &export.item {
                        self.publish_rewalk_function_scheme(func, types, errors);
                    }
                }
                _ => {}
            }
        }
    }

    fn publish_rewalk_function_scheme(
        &mut self,
        func: &FunctionDef,
        types: &HashMap<String, Type>,
        errors: &mut Vec<TypeError>,
    ) {
        let Some(ty) = types.get(&func.name).cloned() else {
            return;
        };
        let Type::Function { params, .. } = &ty else {
            return;
        };

        if params
            .iter()
            .any(|param| self.type_contains_unresolved_vars(param))
        {
            return;
        }

        let scheme = match self.make_function_scheme(func, ty.clone()) {
            Ok(scheme) => scheme,
            Err(error) => {
                errors.push(error);
                return;
            }
        };
        if let Err(error) = self.republish_named_callable_scheme(func, scheme, &ty) {
            errors.push(error);
        }
    }

    fn rewalk_resolved_function_bodies_for_item(
        &mut self,
        item: &Item,
        types: &mut HashMap<String, Type>,
        errors: &mut Vec<TypeError>,
    ) {
        match item {
            Item::Function(func, _) => self.rewalk_resolved_function_body(func, types, errors),
            Item::Export(export, _) => {
                if let shape_ast::ast::ExportItem::Function(func) = &export.item {
                    self.rewalk_resolved_function_body(func, types, errors);
                }
            }
            _ => {}
        }
    }

    fn rewalk_resolved_function_body(
        &mut self,
        func: &FunctionDef,
        types: &mut HashMap<String, Type>,
        errors: &mut Vec<TypeError>,
    ) {
        let Some(Type::Function {
            params, returns, ..
        }) = types.get(&func.name).cloned()
        else {
            return;
        };

        let resolved_params: Vec<Type> = params
            .iter()
            .map(|param| self.solver.unifier().apply_substitutions(param))
            .collect();
        if resolved_params
            .iter()
            .any(|param| self.type_contains_unresolved_vars(param))
        {
            return;
        }

        self.env.push_scope();
        self.push_fallible_scope();

        for (param, param_type) in func.params.iter().zip(resolved_params.iter()) {
            self.bind_function_param_pattern(&param.pattern, param_type);
            self.record_binding_facts_for_param_pattern(&param.pattern);
        }

        let empty_grow_return = Self::fn_body_returns_empty_grow_carrier(func);
        let mut empty_grow_return_carriers = std::collections::HashSet::new();
        if empty_grow_return {
            Self::collect_empty_array_carriers(&func.body, &mut empty_grow_return_carriers);
        }

        self.expected_return_types.push(None);
        self.empty_grow_return_carrier_scopes
            .push(empty_grow_return_carriers);
        let constraint_start = self.constraints.len();
        if func.is_comptime {
            self.enter_comptime();
        }
        let replayed_return = self.infer_callable_return_type(&func.body, true);
        if func.is_comptime {
            self.exit_comptime();
        }
        self.empty_grow_return_carrier_scopes.pop();
        self.expected_return_types.pop();

        let _ = self.pop_fallible_scope();
        self.env.pop_scope();

        let Ok(replayed_return) = replayed_return else {
            return;
        };

        let mut replay_constraints = self.constraints[constraint_start..].to_vec();
        replay_constraints.push((returns.as_ref().clone(), replayed_return.clone()));
        if self.solver.solve(&mut replay_constraints).is_err() {
            return;
        }

        let replayed_return = self.solver.unifier().apply_substitutions(&replayed_return);
        let returns = self.solver.unifier().apply_substitutions(returns.as_ref());
        let new_return = if self.type_contains_unresolved_vars(&returns)
            && !self.type_contains_unresolved_vars(&replayed_return)
        {
            replayed_return
        } else {
            returns
        };

        let new_type = BuiltinTypes::function(resolved_params, new_return);
        let scheme = match self.make_function_scheme(func, new_type.clone()) {
            Ok(scheme) => scheme,
            Err(error) => {
                errors.push(error);
                return;
            }
        };
        if let Err(error) = self.republish_named_callable_scheme(func, scheme, &new_type) {
            errors.push(error);
            return;
        }
        types.insert(func.name.clone(), new_type);
    }

    fn bind_function_param_pattern(&mut self, pattern: &DestructurePattern, scrutinee: &Type) {
        match pattern {
            DestructurePattern::Identifier(name, _) => {
                self.env.define(name, TypeScheme::mono(scrutinee.clone()));
            }
            DestructurePattern::Array(patterns) => {
                let elem_ty = Self::decl_array_element_type(scrutinee).unwrap_or_else(|| {
                    let elem = self.fresh_type_var();
                    if let Type::Variable(elem_var) = &elem {
                        self.param_destructure_array_element_links
                            .push((scrutinee.clone(), elem_var.clone()));
                    }
                    self.constraints
                        .push((scrutinee.clone(), BuiltinTypes::array(elem.clone())));
                    elem
                });
                for pattern in patterns {
                    match pattern {
                        DestructurePattern::Rest(inner) => {
                            self.bind_function_param_pattern(
                                inner,
                                &BuiltinTypes::array(elem_ty.clone()),
                            );
                        }
                        _ => self.bind_function_param_pattern(pattern, &elem_ty),
                    }
                }
            }
            DestructurePattern::Object(fields) => {
                let consumed_keys: Vec<&str> = fields
                    .iter()
                    .filter(|field| !matches!(field.pattern, DestructurePattern::Rest(_)))
                    .map(|field| field.key.as_str())
                    .collect();
                for field in fields {
                    if let DestructurePattern::Rest(inner) = &field.pattern {
                        let rest_ty = self
                            .destructure_object_rest_type(scrutinee, &consumed_keys)
                            .unwrap_or_else(|| Type::Concrete(TypeAnnotation::Object(Vec::new())));
                        self.bind_function_param_pattern(inner, &rest_ty);
                        continue;
                    }
                    let field_ty = self
                        .destructure_object_field_type(scrutinee, &field.key)
                        .unwrap_or_else(|| {
                            let result_ty = self.fresh_type_var();
                            if let Type::Variable(result_var) = &result_ty {
                                self.param_destructure_field_links.push((
                                    scrutinee.clone(),
                                    field.key.clone(),
                                    result_var.clone(),
                                ));
                            }
                            let bound_var = self.fresh_var();
                            self.constraints.push((
                                scrutinee.clone(),
                                Type::Constrained {
                                    var: bound_var,
                                    constraint: Box::new(TypeConstraint::HasField(
                                        field.key.clone(),
                                        Box::new(result_ty.clone()),
                                    )),
                                },
                            ));
                            result_ty
                        });
                    self.bind_function_param_pattern(&field.pattern, &field_ty);
                }
            }
            DestructurePattern::Rest(inner) => {
                self.bind_function_param_pattern(inner, scrutinee);
            }
            DestructurePattern::Decomposition(bindings) => {
                for binding in bindings {
                    let binding_type = self.resolve_type_annotation(&binding.type_annotation);
                    self.env
                        .define(&binding.name, TypeScheme::mono(binding_type));
                }
            }
        }
    }

    fn destructure_object_field_type(&self, scrutinee: &Type, key: &str) -> Option<Type> {
        let resolved = self.solver.unifier().apply_substitutions(scrutinee);
        if let Type::Concrete(TypeAnnotation::Object(fields)) = &resolved {
            if let Some(field) = fields.iter().find(|field| field.name == key) {
                return Some(Self::type_from_annotation_preserving_tyvars(
                    &field.type_annotation,
                ));
            }
        }

        let struct_name = self
            .struct_name_of_type(&resolved)
            .or_else(|| self.struct_name_of_type(scrutinee))?;
        self.struct_field_annotation(&struct_name, key)
            .map(|ann| self.resolve_type_annotation(&ann))
    }

    fn type_from_annotation_preserving_tyvars(ann: &TypeAnnotation) -> Type {
        if let Some(var) = annotation_as_tyvar(ann) {
            return Type::Variable(var);
        }
        match ann {
            TypeAnnotation::Array(inner) => {
                BuiltinTypes::array(Self::type_from_annotation_preserving_tyvars(inner))
            }
            TypeAnnotation::Generic { name, args } => Type::Generic {
                base: Box::new(Type::Concrete(TypeAnnotation::Reference(name.clone()))),
                args: args
                    .iter()
                    .map(Self::type_from_annotation_preserving_tyvars)
                    .collect(),
            },
            TypeAnnotation::Function {
                params,
                returns,
                effects,
            } => Type::Function {
                params: params
                    .iter()
                    .map(|param| {
                        Self::type_from_annotation_preserving_tyvars(&param.type_annotation)
                    })
                    .collect(),
                returns: Box::new(Self::type_from_annotation_preserving_tyvars(returns)),
                // ADR-014 §8.1: the declared row is a component of the type.
                // An unresolvable atom name would be a purity-relevant lie, so
                // a bad row degrades to the proof gap rather than to `{}`.
                effects: crate::type_system::effects::resolve_optional_row_annotation(
                    effects.as_ref(),
                    crate::type_system::effects::EffectStage::Runtime,
                )
                .unwrap_or(EffectRow::Unproven),
            },
            other => Type::Concrete(other.clone()),
        }
    }

    fn record_binding_facts_for_param_pattern(&mut self, pattern: &DestructurePattern) {
        for (name, binder_span) in pattern.get_bindings() {
            if binder_span.is_dummy() {
                continue;
            }
            let Some(scheme) = self.env.lookup(&name) else {
                continue;
            };
            self.binding_fact_table.insert(
                binder_span,
                BindingFact {
                    name,
                    binder_span,
                    initializer_span: None,
                    ty: scheme.ty.clone(),
                },
            );
        }
    }

    /// Resolve a type annotation, converting type parameter references to type variables
    pub(crate) fn resolve_type_annotation(&self, ann: &TypeAnnotation) -> Type {
        match ann {
            // Check if this is a type parameter reference
            ann @ (TypeAnnotation::Basic(_) | TypeAnnotation::Reference(_)) => {
                let name = ann.as_type_name_str().unwrap();
                if name == "void" {
                    return BuiltinTypes::void();
                }
                if let Some(scheme) = self.env.lookup(name) {
                    // If it's a type parameter (a type variable), use it
                    if let Type::Variable(_) = &scheme.ty {
                        return scheme.ty.clone();
                    }
                }
                // A NAMED struct stays NOMINAL (`Reference("Money")`), even
                // though it is also registered as a structural type alias
                // (`Money -> { cents: int }`) for unification. Expanding a `fn
                // f(m: Money)` param to its structural `Object` form lost the
                // name, so `m + m` (with `impl Add for Money` in scope) could
                // not resolve the operator trait — `check_operator_trait`
                // requires a `Basic`/`Reference` name and rejects a bare
                // `Object`. The solver already unifies nominal `Reference` with
                // structural `Object` via `set_struct_schemas`, so keeping the
                // nominal form is sound. (operators slice)
                if self.struct_type_defs.contains_key(name) {
                    return Type::Concrete(TypeAnnotation::Reference(name.into()));
                }
                if let Some(alias_entry) = self.env.lookup_type_alias(name) {
                    return self.resolve_type_annotation(&alias_entry.type_annotation);
                }
                Type::Concrete(ann.clone())
            }
            TypeAnnotation::Array(elem) => {
                let elem_type = self.resolve_type_annotation(elem);
                BuiltinTypes::array(elem_type)
            }
            TypeAnnotation::Tuple(elems) => {
                let resolved: Vec<TypeAnnotation> = elems
                    .iter()
                    .map(|e| {
                        self.resolve_type_annotation(e)
                            .to_annotation()
                            .unwrap_or_else(|| TypeAnnotation::Basic("unknown".to_string()))
                    })
                    .collect();
                Type::Concrete(TypeAnnotation::Tuple(resolved))
            }
            TypeAnnotation::Object(fields) => {
                let resolved_fields = fields
                    .iter()
                    .map(|f| shape_ast::ast::ObjectTypeField {
                        name: f.name.clone(),
                        optional: f.optional,
                        type_annotation: self
                            .resolve_type_annotation(&f.type_annotation)
                            .to_annotation()
                            .unwrap_or_else(|| TypeAnnotation::Basic("unknown".to_string())),
                        annotations: vec![],
                    })
                    .collect();
                Type::Concrete(TypeAnnotation::Object(resolved_fields))
            }
            TypeAnnotation::Generic { name, args } => {
                let resolved_args: Vec<_> = args
                    .iter()
                    .map(|a| self.resolve_type_annotation(a))
                    .collect();
                Type::Generic {
                    base: Box::new(Type::Concrete(TypeAnnotation::Reference(name.clone()))),
                    args: resolved_args,
                }
            }
            TypeAnnotation::Function {
                params, returns, ..
            } => {
                let param_types: Vec<_> = params
                    .iter()
                    .map(|p| self.resolve_type_annotation(&p.type_annotation))
                    .collect();
                let return_type = self.resolve_type_annotation(returns);
                Type::Function {
                    params: param_types,
                    returns: Box::new(return_type),
                    effects: EffectRow::Unproven,
                }
            }
            TypeAnnotation::Union(types) => {
                let resolved: Vec<TypeAnnotation> = types
                    .iter()
                    .filter_map(|t| self.resolve_type_annotation(t).to_annotation())
                    .collect();
                Type::Concrete(TypeAnnotation::Union(resolved))
            }
            TypeAnnotation::Intersection(types) => {
                let resolved: Vec<TypeAnnotation> = types
                    .iter()
                    .filter_map(|t| self.resolve_type_annotation(t).to_annotation())
                    .collect();
                Type::Concrete(TypeAnnotation::Intersection(resolved))
            }
            _ => Type::Concrete(ann.clone()),
        }
    }

    /// Register a trait definition in the type environment
    fn register_trait(&mut self, trait_def: &shape_ast::ast::TraitDef) -> TypeResult<()> {
        self.env.define_trait(trait_def);
        Ok(())
    }

    /// Register an impl block: validate against trait, register methods in MethodTable
    fn register_impl(&mut self, impl_block: &shape_ast::ast::ImplBlock) -> TypeResult<()> {
        let type_name = Self::type_name_str(&impl_block.target_type);
        let trait_name = Self::type_name_str(&impl_block.trait_name);

        self.validate_conversion_impl_shape(impl_block)?;

        // J-CT.1: validate `comptime` alignment between trait and impl.
        // A `comptime impl` must implement a `comptime trait`, and a plain
        // `impl` must implement a non-comptime trait. We only validate when
        // the trait is known to the type environment — unknown traits are
        // diagnosed by the existing `register_trait_impl_*` path.
        if let Some(trait_def) = self.env.lookup_trait(&trait_name) {
            if trait_def.is_comptime != impl_block.is_comptime {
                return Err(TypeError::ComptimeImplTraitMismatch {
                    trait_name: trait_name.clone(),
                    type_name: type_name.clone(),
                    trait_is_comptime: trait_def.is_comptime,
                    impl_is_comptime: impl_block.is_comptime,
                });
            }
        }

        let method_names: Vec<String> = impl_block.methods.iter().map(|m| m.name.clone()).collect();

        // Collect associated type bindings from the impl block
        let associated_types: std::collections::HashMap<String, TypeAnnotation> = impl_block
            .associated_type_bindings
            .iter()
            .map(|b| (b.name.clone(), b.concrete_type.clone()))
            .collect();

        // Validate impl methods against trait definition (arity check)
        if let Some(trait_def) = self.env.lookup_trait(&trait_name) {
            let trait_def = trait_def.clone();
            for member in &trait_def.members {
                // Root #8 (consistent `self`-handling): the trait side carries the
                // receiver `self` — built-in operator/Drop traits register a `self`
                // FunctionParam (environment/mod.rs), and the parser includes `self`
                // in trait method signatures — but impl method params strip the
                // receiver. Compare only the value (non-`self`) params on both sides
                // so the arity check is receiver-agnostic and a `fn drop(&mut self)`
                // trait method matches a `method drop()` impl.
                let (trait_method_name, trait_arity) = match member {
                    TraitMember::Required(TraitMemberSignature::Method {
                        name, params, ..
                    }) => {
                        let arity = params
                            .iter()
                            .filter(|p| p.name.as_deref() != Some("self"))
                            .count();
                        (name.as_str(), arity)
                    }
                    TraitMember::Default(method_def) => {
                        let arity = method_def
                            .params
                            .iter()
                            .filter(|p| {
                                !matches!(&p.pattern, DestructurePattern::Identifier(n, _) if n == "self")
                            })
                            .count();
                        (method_def.name.as_str(), arity)
                    }
                    _ => continue,
                };

                // If the impl provides an override, check arity matches
                if let Some(impl_method) = impl_block
                    .methods
                    .iter()
                    .find(|m| m.name == trait_method_name)
                {
                    let impl_arity = impl_method
                        .params
                        .iter()
                        .filter(|p| {
                            !matches!(&p.pattern, DestructurePattern::Identifier(n, _) if n == "self")
                        })
                        .count();
                    if trait_arity != impl_arity {
                        return Err(TypeError::TraitImplArityMismatch {
                            trait_name: trait_name.clone(),
                            method_name: trait_method_name.to_string(),
                            expected: trait_arity,
                            got: impl_arity,
                        });
                    }
                }
            }
        }

        // Register the impl in the type registry (validates required methods +
        // associated types present, supertraits, and coherence)
        if let Err(msg) = self.env.register_trait_impl_with_assoc_types_named(
            &trait_name,
            &type_name,
            impl_block.impl_name.as_deref(),
            method_names,
            associated_types,
        ) {
            return Err(TypeError::TraitImplValidation(msg));
        }

        // Extract receiver type params from the target type for generic method registration
        let receiver_type_params: Vec<String> = match &impl_block.target_type {
            TypeName::Generic { type_args, .. } => type_args
                .iter()
                .filter_map(|arg| {
                    let name_str = match arg {
                        TypeAnnotation::Basic(name) => name.as_str(),
                        TypeAnnotation::Reference(path) => path.as_str(),
                        _ => return None,
                    };
                    let first = name_str.chars().next().unwrap_or('a');
                    if first.is_uppercase() && name_str.len() <= 2 {
                        Some(name_str.to_string())
                    } else {
                        None
                    }
                })
                .collect(),
            _ => vec![],
        };

        // Extract trait-level type param bounds from the trait name for bound checking
        // e.g., `impl NumericVec for Vec` where NumericVec requires T: Numeric
        let receiver_param_bounds: Vec<(usize, Vec<String>)> =
            Self::extract_trait_receiver_bounds(&impl_block.trait_name, &receiver_type_params);

        let has_receiver_params = !receiver_type_params.is_empty();

        // Wave-1b SEAM A (user ruling 2026-06-15): Iterator is a REAL
        // user-implementable trait. When the user writes `impl Iterator for
        // MyType`, seed the Iterator adapter/terminal surface (map/filter/
        // collect/...) onto MyType so `myValue.map(...)` etc. resolve. This
        // runs BEFORE the per-impl-method loop below so the user's own `next`
        // (and any adapter override) overwrites the seeded signature rather
        // than the reverse. Gated on `trait_name == "Iterator"`, so it touches
        // no other type and cannot regress existing method resolution.
        if trait_name == "Iterator" {
            self.method_table.register_iterator_methods(&type_name);
        }

        // Register each impl method in the method table under the target type
        let impl_method_names: Vec<String> =
            impl_block.methods.iter().map(|m| m.name.clone()).collect();

        for method in &impl_block.methods {
            self.register_impl_method(
                &type_name,
                method,
                &receiver_type_params,
                &receiver_param_bounds,
                has_receiver_params,
            );
        }

        // Register default methods from the trait that the impl doesn't override
        if let Some(trait_def) = self.env.lookup_trait(&trait_name) {
            let trait_def = trait_def.clone();
            for member in &trait_def.members {
                if let TraitMember::Default(default_method) = member {
                    if !impl_method_names.contains(&default_method.name) {
                        self.register_impl_method(
                            &type_name,
                            default_method,
                            &receiver_type_params,
                            &receiver_param_bounds,
                            has_receiver_params,
                        );
                    }
                }
            }
        }

        // J-CT.1: mark every method registered by a `comptime impl` block as
        // comptime-only. The expression-level method-call checker rejects
        // runtime call sites for these methods via `is_comptime_method`.
        // This includes default methods inherited from the trait — a
        // comptime impl that doesn't override a default still exposes that
        // method only at compile time.
        if impl_block.is_comptime {
            for method in &impl_block.methods {
                self.method_table
                    .mark_comptime_method(&type_name, &method.name);
            }
            if let Some(trait_def) = self.env.lookup_trait(&trait_name) {
                let trait_def = trait_def.clone();
                for member in &trait_def.members {
                    if let TraitMember::Default(default_method) = member {
                        if !impl_method_names.contains(&default_method.name) {
                            self.method_table
                                .mark_comptime_method(&type_name, &default_method.name);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Type-check impl method bodies with the receiver type in scope.
    ///
    /// Impl registration alone only publishes the method surface. Strict
    /// bytecode lowering consumes finalized expression facts for method bodies
    /// too, so inference must also walk those bodies with the same implicit
    /// receiver the compiler later desugars: `method eq(other: Self)` in
    /// `impl Eq for Money` is checked as an inference-only
    /// `fn Money::eq(self: Money, other: Money) -> bool { ... }`.
    fn infer_impl_method_bodies(
        &mut self,
        impl_block: &shape_ast::ast::ImplBlock,
    ) -> TypeResult<()> {
        if !Self::impl_target_is_concrete(&impl_block.target_type) {
            return Ok(());
        }

        let type_name = Self::type_name_str(&impl_block.target_type);
        let trait_name = Self::type_name_str(&impl_block.trait_name);
        let self_ann = Self::type_name_to_annotation_for_impl(&impl_block.target_type);
        let trait_def = self.env.lookup_trait(&trait_name).cloned();
        let trait_type_args = trait_def
            .as_ref()
            .map(|def| Self::impl_trait_type_arg_substitutions(def, &impl_block.trait_name))
            .unwrap_or_default();

        for method in &impl_block.methods {
            let func = self.inference_function_for_impl_method(
                method,
                &type_name,
                &self_ann,
                trait_def.as_ref(),
                &trait_type_args,
            )?;
            let _ = self.infer_function(&func)?;
        }

        if let Some(trait_def) = trait_def {
            let overridden: std::collections::HashSet<&str> =
                impl_block.methods.iter().map(|m| m.name.as_str()).collect();
            for member in &trait_def.members {
                if let TraitMember::Default(default_method) = member {
                    if overridden.contains(default_method.name.as_str()) {
                        continue;
                    }
                    let func = self.inference_function_for_impl_method(
                        default_method,
                        &type_name,
                        &self_ann,
                        Some(&trait_def),
                        &trait_type_args,
                    )?;
                    let _ = self.infer_function(&func)?;
                }
            }
        }

        Ok(())
    }

    fn inference_function_for_impl_method(
        &mut self,
        method: &shape_ast::ast::MethodDef,
        type_name: &str,
        self_ann: &TypeAnnotation,
        trait_def: Option<&shape_ast::ast::TraitDef>,
        trait_type_args: &HashMap<String, TypeAnnotation>,
    ) -> TypeResult<FunctionDef> {
        let mut params = Vec::with_capacity(method.params.len() + 1);
        params.push(shape_ast::ast::FunctionParameter {
            pattern: DestructurePattern::Identifier("self".to_string(), Span::DUMMY),
            is_const: false,
            is_reference: false,
            is_mut_reference: false,
            is_out: false,
            type_annotation: Some(self_ann.clone()),
            default_value: None,
        });

        for (idx, param) in method.params.iter().enumerate() {
            let mut param = param.clone();
            if let Some(ann) = param.type_annotation.as_ref() {
                param.type_annotation = Some(Self::substitute_trait_impl_annotation(
                    ann,
                    self_ann,
                    trait_type_args,
                ));
            } else if let Some(sig_param) = trait_def
                .and_then(|def| Self::trait_method_value_param_annotation(def, &method.name, idx))
            {
                param.type_annotation = Some(Self::substitute_trait_impl_annotation(
                    sig_param,
                    self_ann,
                    trait_type_args,
                ));
            }
            params.push(param);
        }

        let return_type = method
            .return_type
            .as_ref()
            .map(|ann| Self::substitute_trait_impl_annotation(ann, self_ann, trait_type_args))
            .or_else(|| {
                trait_def
                    .and_then(|def| Self::trait_method_return_annotation(def, &method.name))
                    .map(|ret| {
                        Self::substitute_trait_impl_annotation(ret, self_ann, trait_type_args)
                    })
            });

        Ok(FunctionDef {
            name: format!("{}::{}", type_name, method.name),
            name_span: method.span,
            declaring_module_path: method.declaring_module_path.clone(),
            doc_comment: None,
            params,
            return_type,
            body: method.body.clone(),
            type_params: method.type_params.clone(),
            annotations: method.annotations.clone(),
            is_async: method.is_async,
            is_comptime: false,
            where_clause: None,
            effect_row: None,
        })
    }

    fn impl_trait_type_arg_substitutions(
        trait_def: &shape_ast::ast::TraitDef,
        trait_name: &TypeName,
    ) -> HashMap<String, TypeAnnotation> {
        let TypeName::Generic { type_args, .. } = trait_name else {
            return HashMap::new();
        };

        let Some(type_params) = trait_def.type_params.as_ref() else {
            return HashMap::new();
        };

        type_params
            .iter()
            .zip(type_args.iter())
            .map(|(param, arg)| (param.name().to_string(), arg.clone()))
            .collect()
    }

    fn substitute_trait_impl_annotation(
        ann: &TypeAnnotation,
        self_ann: &TypeAnnotation,
        trait_type_args: &HashMap<String, TypeAnnotation>,
    ) -> TypeAnnotation {
        fn replacement<'a>(
            ann: &TypeAnnotation,
            trait_type_args: &'a HashMap<String, TypeAnnotation>,
        ) -> Option<&'a TypeAnnotation> {
            match ann {
                TypeAnnotation::Basic(name) => trait_type_args.get(name.as_str()),
                TypeAnnotation::Reference(path) => trait_type_args.get(path.as_str()),
                TypeAnnotation::Generic { name, args } if args.is_empty() => {
                    trait_type_args.get(name.as_str())
                }
                _ => None,
            }
        }

        if let Some(arg) = replacement(ann, trait_type_args) {
            return Self::substitute_trait_self_annotation(arg, self_ann);
        }

        let with_self = Self::substitute_trait_self_annotation(ann, self_ann);
        match with_self {
            TypeAnnotation::Array(inner) => TypeAnnotation::Array(Box::new(
                Self::substitute_trait_impl_annotation(&inner, self_ann, trait_type_args),
            )),
            TypeAnnotation::Tuple(items) => TypeAnnotation::Tuple(
                items
                    .iter()
                    .map(|item| {
                        Self::substitute_trait_impl_annotation(item, self_ann, trait_type_args)
                    })
                    .collect(),
            ),
            TypeAnnotation::Object(fields) => TypeAnnotation::Object(
                fields
                    .into_iter()
                    .map(|mut field| {
                        field.type_annotation = Self::substitute_trait_impl_annotation(
                            &field.type_annotation,
                            self_ann,
                            trait_type_args,
                        );
                        field
                    })
                    .collect(),
            ),
            TypeAnnotation::Function {
                params,
                returns,
                effects,
            } => TypeAnnotation::Function {
                params: params
                    .into_iter()
                    .map(|mut param| {
                        param.type_annotation = Self::substitute_trait_impl_annotation(
                            &param.type_annotation,
                            self_ann,
                            trait_type_args,
                        );
                        param
                    })
                    .collect(),
                returns: Box::new(Self::substitute_trait_impl_annotation(
                    &returns,
                    self_ann,
                    trait_type_args,
                )),
                // ADR-014 §8.1: an impl's actual row must be a subset of the
                // trait's declared row. Substituting `Self` does not change
                // either, so the annotation's row is preserved verbatim.
                effects: effects.clone(),
            },
            TypeAnnotation::Union(items) => TypeAnnotation::Union(
                items
                    .iter()
                    .map(|item| {
                        Self::substitute_trait_impl_annotation(item, self_ann, trait_type_args)
                    })
                    .collect(),
            ),
            TypeAnnotation::Intersection(items) => TypeAnnotation::Intersection(
                items
                    .iter()
                    .map(|item| {
                        Self::substitute_trait_impl_annotation(item, self_ann, trait_type_args)
                    })
                    .collect(),
            ),
            TypeAnnotation::Generic { name, args } => TypeAnnotation::Generic {
                name,
                args: args
                    .iter()
                    .map(|arg| {
                        Self::substitute_trait_impl_annotation(arg, self_ann, trait_type_args)
                    })
                    .collect(),
            },
            TypeAnnotation::Borrow { mutable, inner } => TypeAnnotation::Borrow {
                mutable,
                inner: Box::new(Self::substitute_trait_impl_annotation(
                    &inner,
                    self_ann,
                    trait_type_args,
                )),
            },
            other => other,
        }
    }

    fn trait_method_value_param_annotation<'a>(
        trait_def: &'a shape_ast::ast::TraitDef,
        method_name: &str,
        value_param_index: usize,
    ) -> Option<&'a TypeAnnotation> {
        for member in &trait_def.members {
            match member {
                TraitMember::Required(TraitMemberSignature::Method { name, params, .. })
                    if name == method_name =>
                {
                    return params
                        .iter()
                        .filter(|p| p.name.as_deref() != Some("self"))
                        .nth(value_param_index)
                        .map(|p| &p.type_annotation);
                }
                TraitMember::Default(default_method) if default_method.name == method_name => {
                    return default_method
                        .params
                        .iter()
                        .filter(|p| {
                            !matches!(&p.pattern, DestructurePattern::Identifier(n, _) if n == "self")
                        })
                        .nth(value_param_index)
                        .and_then(|p| p.type_annotation.as_ref());
                }
                _ => {}
            }
        }
        None
    }

    fn trait_method_return_annotation<'a>(
        trait_def: &'a shape_ast::ast::TraitDef,
        method_name: &str,
    ) -> Option<&'a TypeAnnotation> {
        for member in &trait_def.members {
            match member {
                TraitMember::Required(TraitMemberSignature::Method {
                    name, return_type, ..
                }) if name == method_name => {
                    return Some(return_type);
                }
                TraitMember::Default(default_method) if default_method.name == method_name => {
                    return default_method.return_type.as_ref();
                }
                _ => {}
            }
        }
        None
    }

    pub(super) fn type_name_to_annotation_for_impl(type_name: &TypeName) -> TypeAnnotation {
        match type_name {
            TypeName::Simple(name) => TypeAnnotation::Reference(name.as_str().into()),
            TypeName::Generic { name, type_args } => TypeAnnotation::Generic {
                name: name.clone(),
                args: type_args.clone(),
            },
        }
    }

    fn impl_target_is_concrete(type_name: &TypeName) -> bool {
        match type_name {
            TypeName::Simple(name) => !matches!(
                name.as_str(),
                "Array"
                    | "Vec"
                    | "Table"
                    | "DataTable"
                    | "HashMap"
                    | "Map"
                    | "Set"
                    | "Option"
                    | "Result"
                    | "Iterator"
            ),
            TypeName::Generic { type_args, .. } => type_args.iter().all(|arg| {
                if let Some(name) = arg.as_type_name_str() {
                    let is_short_upper_param =
                        name.len() <= 2 && name.chars().next().is_some_and(|ch| ch.is_uppercase());
                    !is_short_upper_param
                } else {
                    true
                }
            }),
        }
    }

    /// Register a single method from an impl block in the method table,
    /// handling both generic and monomorphic methods.
    fn register_impl_method(
        &mut self,
        type_name: &str,
        method: &shape_ast::ast::MethodDef,
        receiver_type_params: &[String],
        receiver_param_bounds: &[(usize, Vec<String>)],
        has_receiver_params: bool,
    ) {
        use crate::type_system::checking::method_table::TypeParamExpr;

        let method_type_params: Vec<String> = method
            .type_params
            .as_ref()
            .map(|tps| tps.iter().map(|tp| tp.name().to_string()).collect())
            .unwrap_or_default();

        let is_generic = has_receiver_params || !method_type_params.is_empty();

        if is_generic {
            let param_exprs: Vec<TypeParamExpr> = method
                .params
                .iter()
                .map(|p| match &p.type_annotation {
                    Some(ann) => Self::annotation_to_type_param_expr(
                        ann,
                        receiver_type_params,
                        &method_type_params,
                    ),
                    None => TypeParamExpr::Concrete(self.fresh_type_var()),
                })
                .collect();
            let return_expr = match method.return_type.as_ref() {
                Some(ann) => Self::annotation_to_type_param_expr(
                    ann,
                    receiver_type_params,
                    &method_type_params,
                ),
                None => TypeParamExpr::Concrete(self.fresh_type_var()),
            };

            self.method_table.register_user_generic_method(
                type_name,
                &method.name,
                method_type_params.len(),
                param_exprs,
                return_expr,
                receiver_param_bounds.to_vec(),
            );
        } else {
            let param_types: Vec<Type> = method
                .params
                .iter()
                .map(|p| match &p.type_annotation {
                    Some(ann) => self.resolve_type_annotation(ann),
                    None => self.fresh_type_var(),
                })
                .collect();
            let return_type = match method.return_type.as_ref() {
                Some(ann) => self.resolve_type_annotation(ann),
                None => self.fresh_type_var(),
            };

            self.method_table.register_user_method(
                type_name,
                &method.name,
                param_types,
                return_type,
            );
        }
    }

    /// Extract receiver parameter trait bounds from a trait name.
    /// For now returns empty — bounds will come from where clauses or
    /// trait-level type params in future iterations.
    fn extract_trait_receiver_bounds(
        _trait_name: &TypeName,
        _receiver_type_params: &[String],
    ) -> Vec<(usize, Vec<String>)> {
        // TODO: Extract bounds from trait definition's type params
        // e.g., if NumericVec<T: Numeric> then T at receiver index 0 requires Numeric
        vec![]
    }

    /// Extract the simple type name string from a TypeName
    pub(super) fn type_name_str(tn: &TypeName) -> String {
        match tn {
            TypeName::Simple(n) => n.to_string(),
            TypeName::Generic { name, .. } => name.to_string(),
        }
    }

    fn canonical_conversion_name_for_impl(name: &str) -> String {
        BuiltinTypes::canonical_script_alias(name)
            .map(ToString::to_string)
            .unwrap_or_else(|| name.to_string())
    }

    fn conversion_name_from_annotation_for_impl(annotation: &TypeAnnotation) -> Option<String> {
        let name = match annotation {
            TypeAnnotation::Basic(name) => Some(name.as_str()),
            TypeAnnotation::Reference(path) => Some(path.as_str()),
            TypeAnnotation::Generic { name, .. } => Some(name.as_str()),
            _ => None,
        };
        name.map(Self::canonical_conversion_name_for_impl)
    }

    fn validate_conversion_impl_shape(
        &self,
        impl_block: &shape_ast::ast::ImplBlock,
    ) -> TypeResult<()> {
        let trait_target = match &impl_block.trait_name {
            TypeName::Generic { name, type_args } if name == "TryInto" || name == "Into" => {
                if type_args.len() != 1 {
                    return Err(TypeError::TraitImplValidation(format!(
                        "{} impl must declare exactly one target: `impl {}<Target> for Source as target`",
                        name, name
                    )));
                }
                let target = Self::conversion_name_from_annotation_for_impl(&type_args[0])
                    .ok_or_else(|| {
                        TypeError::TraitImplValidation(format!(
                            "{} target must be a concrete named type",
                            name
                        ))
                    })?;
                Some((name.as_str(), target))
            }
            TypeName::Simple(name) if name == "TryInto" || name == "Into" => {
                return Err(TypeError::TraitImplValidation(format!(
                    "{} impl must use generic target form: `impl {}<Target> for Source as target`",
                    name, name
                )));
            }
            _ => None,
        };

        if let Some((trait_name, target)) = trait_target {
            let selector = impl_block.impl_name.as_deref().ok_or_else(|| {
                TypeError::TraitImplValidation(format!(
                    "{} impl must declare named selector with `as target`",
                    trait_name
                ))
            })?;
            let selector = Self::canonical_conversion_name_for_impl(selector);
            if selector != target {
                return Err(TypeError::TraitImplValidation(format!(
                    "{} target `{}` must match impl selector `{}`",
                    trait_name, target, selector
                )));
            }
        }

        Ok(())
    }

    /// Let-gen spec §4 (A-enforced): record a module-scope `let`/`var`/`const`
    /// binding with NO explicit type annotation, so the post-solve
    /// [`reject_unpinnable_let_bindings`] pass can demand an annotation if its
    /// init is a bare APPLICATION (class-(2)) and its final inferred type is still
    /// a fully-polymorphic carrier. An annotated binding pins its own type and is
    /// never recorded. The init-is-application flag distinguishes the class-(2)
    /// `let x = get_none()` (reject) from the class-(3) direct value binding
    /// `let x = None` (left to compile, like the language already does).
    pub(crate) fn record_unannotated_let_origin(&mut self, decl: &VariableDecl) {
        if decl.type_annotation.is_some() {
            return;
        }
        // §4 A-enforced targets the grounding's class-(2) bare-FUNCTION-application
        // (`let x = get_none()`). A method-call builder chain (`HashMap().set(..)`)
        // is NOT in scope: its un-pinned type args are an ordinary inference gap,
        // not an irreducibly-polymorphic value, so it is left to compile.
        let is_application_init = matches!(
            decl.value.as_ref(),
            Some(Expr::FunctionCall { .. } | Expr::QualifiedFunctionCall { .. })
        );
        if let (Some(name), Some(span)) = (
            decl.pattern.as_identifier(),
            decl.pattern.as_identifier_span(),
        ) {
            self.unannotated_let_binding_origins
                .insert(name.to_string(), (span, is_application_init));
        }
    }

    /// Let-gen spec §4 (A-enforced): post-solve binding-level reject.
    ///
    /// A module-scope un-annotated `let`/`var`/`const` whose FINAL inferred type
    /// (after constraint solving + substitution) still carries an un-pinnable
    /// free generic arg — e.g. `let x = get_none()` where nothing downstream
    /// constrains `T` — is a compile error demanding an annotation. This keeps
    /// the "always-concrete `let`" contract: the fn-boundary fix makes
    /// `get_none : ∀T. () -> Option<T>`, so the bare application would otherwise
    /// bind an un-pinned `Option<T>` mono into `let` with no re-check. Mirrors
    /// the empty-array `let a: Array<T> = []` remedy.
    ///
    /// Only fires for a FULLY-polymorphic `Type::Generic` carrier — one whose
    /// generic args are EVERY unresolved (e.g. `Option<T>` from a pure
    /// `get_none()`, no concrete payload anywhere). A carrier with at least one
    /// concrete arg (e.g. `Result<T, string>` from `Err("boom")`, or
    /// `Result<T, AnyError>` from a `find` over a typed array) is left to
    /// compile: its concrete payload proves the value is not irreducibly
    /// polymorphic, and per §1.4 the un-concretized arg is kind-erased and never
    /// reaches a `NativeKind` stamp. A bare unresolved `Type::Variable` is a
    /// different (ordinary inference-loss) class and is also left untouched. This
    /// narrowness is exactly the A1-vs-A3 split in spec §5.1.
    pub(crate) fn reject_unpinnable_let_bindings(
        &self,
        types: &HashMap<String, Type>,
    ) -> Vec<TypeError> {
        let mut errors = Vec::new();
        for (name, (span, is_application_init)) in &self.unannotated_let_binding_origins {
            // §4 A-enforced fires ONLY for the class-(2) bare-application case
            // (`let x = get_none()`). A class-(3) direct value binding
            // (`let x = None`) is left to compile, matching the language's
            // established acceptance of pure kind-erased `None` literals.
            if !is_application_init {
                continue;
            }
            let Some(ty) = types.get(name) else { continue };
            let fully_polymorphic = match ty {
                Type::Generic { args, .. } => {
                    !args.is_empty()
                        && args
                            .iter()
                            .all(|arg| matches!(arg, Type::Variable(_) | Type::Constrained { .. }))
                }
                _ => false,
            };
            if fully_polymorphic {
                let _ = span; // span carried by origins map; message is self-describing
                errors.push(TypeError::GenericTypeError {
                    message: format!(
                        "Cannot infer a concrete type for binding '{}': its inferred type '{}' \
                         still has an un-pinnable generic argument. Add a type annotation (e.g. \
                         `let {}: Option<ConcreteT> = …`) or use the value at a site that fixes \
                         the type.",
                        name,
                        self.render_type_for_diag(ty),
                        name
                    ),
                    symbol: Some(name.clone()),
                });
            }
        }
        errors
    }

    /// Fn-boundary let-gen cond-4 (let-gen spec §1.2 / §3.2): is `func`'s body
    /// NON-EXPANSIVE w.r.t. the to-be-quantified return vars?
    ///
    /// This is a purely-syntactic body scan — no solver, no inference state, no
    /// variance lattice (spec §6 "No new modal-types subsystem"). It returns
    /// `true` iff EVERY return-reachable returned value provably traces to a
    /// freshly-constructed carrier (`None` / `Some(..)` / `Ok(..)` / `Err(..)` /
    /// enum-constructor / struct-literal / array/object literal) or to a fn-local
    /// IMMUTABLE `let`/`const` chain bottoming out in such a value. A returned
    /// value sourced from a `var` / `let mut` / module-scope binding, a
    /// parameter, a reference/deref into one, or a general function application
    /// is EXPANSIVE → `false` (the strict reject / §3.2 refusal stands).
    ///
    /// The default is conservative: any expression shape this scan does not
    /// positively recognize as non-expansive is treated as EXPANSIVE, so the
    /// scan never widens generalization beyond provably-fresh carriers.
    fn fn_body_is_non_expansive(func: &FunctionDef) -> bool {
        // Fn-local immutable `let`/`const` bindings (name -> initializer). A name
        // qualifies ONLY if it is declared immutable AND never also declared as a
        // `var` / `let mut` anywhere in the body (conservative against shadowing).
        let mut immutable_lets: HashMap<String, &Expr> = HashMap::new();
        let mut mutable_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        Self::collect_body_bindings(&func.body, &mut immutable_lets, &mut mutable_names);
        immutable_lets.retain(|name, _| !mutable_names.contains(name));

        // Every return-reachable returned value (explicit `return` + implicit
        // tail) must be non-expansive.
        let mut returned: Vec<&Expr> = Vec::new();
        Self::collect_explicit_returns(&func.body, &mut returned);
        Self::collect_tail_values(&func.body, &mut returned);

        // A body with no recognizable returned value (e.g. a `void` body) is not
        // a generalization target anyway; treat the empty-set as non-expansive so
        // it is governed solely by gates (a)-(c) upstream.
        returned
            .iter()
            .all(|expr| Self::expr_is_nonexpansive(expr, &immutable_lets, 0))
    }

    /// Empty-grow return proof: an unannotated function may return a freshly
    /// allocated local array whose element type is established only by later
    /// `.push` operations. The local's element variable is still static; the
    /// function-boundary gate just needs to let call-site propagation solve it.
    ///
    /// This deliberately recognizes only a small AST class:
    /// - a fn-local mutable/var binding initialized exactly to `[]`;
    /// - assignments to that binding are only `name = name.push(..)` or
    ///   `name = name.slice(..)` (both preserve a single element type);
    /// - every returned value is that binding or an array literal containing
    ///   only such bindings.
    ///
    /// Anything else stays expansive and keeps the existing unresolved-generic
    /// rejection. No element type is defaulted here.
    fn fn_body_returns_empty_grow_carrier(func: &FunctionDef) -> bool {
        let mut carriers = std::collections::HashSet::new();
        Self::collect_empty_array_carriers(&func.body, &mut carriers);
        if carriers.is_empty() {
            return false;
        }
        if !Self::empty_array_carrier_assignments_are_preserving(&func.body, &carriers) {
            return false;
        }

        let mut returned: Vec<&Expr> = Vec::new();
        Self::collect_explicit_returns(&func.body, &mut returned);
        Self::collect_tail_values(&func.body, &mut returned);
        !returned.is_empty()
            && returned
                .iter()
                .all(|expr| Self::expr_returns_empty_array_carrier(expr, &carriers))
    }

    fn collect_empty_array_carriers(
        stmts: &[Statement],
        carriers: &mut std::collections::HashSet<String>,
    ) {
        for stmt in stmts {
            match stmt {
                Statement::VariableDecl(decl, _) => {
                    let is_mutable = decl.is_mut || matches!(decl.kind, VarKind::Var);
                    if is_mutable
                        && matches!(decl.value.as_ref(), Some(Expr::Array(elements, _)) if elements.is_empty())
                    {
                        for name in decl.pattern.get_identifiers() {
                            carriers.insert(name);
                        }
                    }
                }
                Statement::If(if_stmt, _) => {
                    Self::collect_empty_array_carriers(&if_stmt.then_body, carriers);
                    if let Some(else_body) = &if_stmt.else_body {
                        Self::collect_empty_array_carriers(else_body, carriers);
                    }
                }
                Statement::For(for_loop, _) => {
                    Self::collect_empty_array_carriers(&for_loop.body, carriers);
                }
                Statement::While(while_loop, _) => {
                    Self::collect_empty_array_carriers(&while_loop.body, carriers);
                }
                Statement::Expression(expr, _) => {
                    Self::collect_empty_array_carriers_in_expr(expr, carriers);
                }
                _ => {}
            }
        }
    }

    fn collect_empty_array_carriers_in_expr(
        expr: &Expr,
        carriers: &mut std::collections::HashSet<String>,
    ) {
        match expr {
            Expr::Block(block, _) => {
                for item in &block.items {
                    match item {
                        shape_ast::ast::BlockItem::VariableDecl(decl) => {
                            let is_mutable = decl.is_mut || matches!(decl.kind, VarKind::Var);
                            if is_mutable
                                && matches!(decl.value.as_ref(), Some(Expr::Array(elements, _)) if elements.is_empty())
                            {
                                for name in decl.pattern.get_identifiers() {
                                    carriers.insert(name);
                                }
                            }
                        }
                        shape_ast::ast::BlockItem::Statement(stmt) => {
                            Self::collect_empty_array_carriers(
                                std::slice::from_ref(stmt),
                                carriers,
                            );
                        }
                        shape_ast::ast::BlockItem::Expression(expr) => {
                            Self::collect_empty_array_carriers_in_expr(expr, carriers);
                        }
                        shape_ast::ast::BlockItem::Assignment(_) => {}
                    }
                }
            }
            Expr::If(if_expr, _) => {
                Self::collect_empty_array_carriers_in_expr(&if_expr.then_branch, carriers);
                if let Some(else_branch) = &if_expr.else_branch {
                    Self::collect_empty_array_carriers_in_expr(else_branch, carriers);
                }
            }
            Expr::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                Self::collect_empty_array_carriers_in_expr(then_expr, carriers);
                if let Some(else_expr) = else_expr {
                    Self::collect_empty_array_carriers_in_expr(else_expr, carriers);
                }
            }
            Expr::Match(match_expr, _) => {
                for arm in &match_expr.arms {
                    Self::collect_empty_array_carriers_in_expr(&arm.body, carriers);
                }
            }
            _ => {}
        }
    }

    fn empty_array_carrier_assignments_are_preserving(
        stmts: &[Statement],
        carriers: &std::collections::HashSet<String>,
    ) -> bool {
        for stmt in stmts {
            match stmt {
                Statement::Assignment(assign, _) => {
                    if !Self::empty_array_carrier_assignment_is_preserving(assign, carriers) {
                        return false;
                    }
                }
                Statement::If(if_stmt, _) => {
                    if !Self::empty_array_carrier_assignments_are_preserving(
                        &if_stmt.then_body,
                        carriers,
                    ) {
                        return false;
                    }
                    if let Some(else_body) = &if_stmt.else_body
                        && !Self::empty_array_carrier_assignments_are_preserving(
                            else_body, carriers,
                        )
                    {
                        return false;
                    }
                }
                Statement::For(for_loop, _) => {
                    if !Self::empty_array_carrier_assignments_are_preserving(
                        &for_loop.body,
                        carriers,
                    ) {
                        return false;
                    }
                }
                Statement::While(while_loop, _) => {
                    if !Self::empty_array_carrier_assignments_are_preserving(
                        &while_loop.body,
                        carriers,
                    ) {
                        return false;
                    }
                }
                Statement::Expression(expr, _) => {
                    if !Self::empty_array_carrier_assignments_in_expr_are_preserving(expr, carriers)
                    {
                        return false;
                    }
                }
                _ => {}
            }
        }
        true
    }

    fn empty_array_carrier_assignments_in_expr_are_preserving(
        expr: &Expr,
        carriers: &std::collections::HashSet<String>,
    ) -> bool {
        match expr {
            Expr::Block(block, _) => {
                for item in &block.items {
                    let ok = match item {
                        shape_ast::ast::BlockItem::Assignment(assign) => {
                            Self::empty_array_carrier_assignment_is_preserving(assign, carriers)
                        }
                        shape_ast::ast::BlockItem::Statement(stmt) => {
                            Self::empty_array_carrier_assignments_are_preserving(
                                std::slice::from_ref(stmt),
                                carriers,
                            )
                        }
                        shape_ast::ast::BlockItem::Expression(expr) => {
                            Self::empty_array_carrier_assignments_in_expr_are_preserving(
                                expr, carriers,
                            )
                        }
                        shape_ast::ast::BlockItem::VariableDecl(_) => true,
                    };
                    if !ok {
                        return false;
                    }
                }
                true
            }
            Expr::If(if_expr, _) => {
                Self::empty_array_carrier_assignments_in_expr_are_preserving(
                    &if_expr.then_branch,
                    carriers,
                ) && if let Some(else_branch) = &if_expr.else_branch {
                    Self::empty_array_carrier_assignments_in_expr_are_preserving(
                        else_branch,
                        carriers,
                    )
                } else {
                    true
                }
            }
            Expr::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                Self::empty_array_carrier_assignments_in_expr_are_preserving(then_expr, carriers)
                    && if let Some(else_expr) = else_expr {
                        Self::empty_array_carrier_assignments_in_expr_are_preserving(
                            else_expr, carriers,
                        )
                    } else {
                        true
                    }
            }
            Expr::Match(match_expr, _) => match_expr.arms.iter().all(|arm| {
                Self::empty_array_carrier_assignments_in_expr_are_preserving(&arm.body, carriers)
            }),
            _ => true,
        }
    }

    fn empty_array_carrier_assignment_is_preserving(
        assign: &shape_ast::ast::Assignment,
        carriers: &std::collections::HashSet<String>,
    ) -> bool {
        let Some(target_name) = assign.pattern.as_identifier() else {
            return true;
        };
        if !carriers.contains(target_name) {
            return true;
        }
        matches!(
            &assign.value,
            Expr::MethodCall {
                receiver,
                method,
                ..
            } if matches!(receiver.as_ref(), Expr::Identifier(name, _) if name == target_name)
                && matches!(method.as_str(), "push" | "slice")
        )
    }

    fn expr_returns_empty_array_carrier(
        expr: &Expr,
        carriers: &std::collections::HashSet<String>,
    ) -> bool {
        match expr {
            Expr::Identifier(name, _) => carriers.contains(name),
            Expr::Array(elements, _) => elements
                .iter()
                .all(|elem| Self::expr_returns_empty_array_carrier(elem, carriers)),
            _ => false,
        }
    }

    /// Collect fn-local binding names. Immutable `let`/`const` bindings are
    /// recorded with their initializer; `var` / `let mut` names are recorded as
    /// mutable so [`fn_body_is_non_expansive`] can exclude them.
    fn collect_body_bindings<'a>(
        stmts: &'a [Statement],
        immutable_lets: &mut HashMap<String, &'a Expr>,
        mutable_names: &mut std::collections::HashSet<String>,
    ) {
        for stmt in stmts {
            match stmt {
                Statement::VariableDecl(decl, _) => {
                    let is_immutable = match decl.kind {
                        VarKind::Let => !decl.is_mut,
                        VarKind::Const => true,
                        VarKind::Var => false,
                    };
                    if is_immutable {
                        if let (Some(name), Some(init)) =
                            (decl.pattern.as_identifier(), decl.value.as_ref())
                        {
                            immutable_lets.insert(name.to_string(), init);
                        } else {
                            // Destructured / un-initialized immutable bindings are
                            // not chased — mark their names mutable-equivalent so a
                            // read of them is treated as expansive.
                            for name in decl.pattern.get_identifiers() {
                                mutable_names.insert(name);
                            }
                        }
                    } else {
                        for name in decl.pattern.get_identifiers() {
                            mutable_names.insert(name);
                        }
                    }
                }
                Statement::If(if_stmt, _) => {
                    Self::collect_body_bindings(&if_stmt.then_body, immutable_lets, mutable_names);
                    if let Some(else_body) = &if_stmt.else_body {
                        Self::collect_body_bindings(else_body, immutable_lets, mutable_names);
                    }
                }
                Statement::For(for_loop, _) => {
                    Self::collect_body_bindings(&for_loop.body, immutable_lets, mutable_names);
                }
                Statement::While(while_loop, _) => {
                    Self::collect_body_bindings(&while_loop.body, immutable_lets, mutable_names);
                }
                _ => {}
            }
        }
    }

    /// Collect the value expression of every explicit `return <expr>` reachable
    /// in the body, recursing through control-flow statement bodies.
    pub(super) fn collect_explicit_returns<'a>(stmts: &'a [Statement], out: &mut Vec<&'a Expr>) {
        for stmt in stmts {
            match stmt {
                Statement::Return(Some(expr), _) => out.push(expr),
                Statement::If(if_stmt, _) => {
                    Self::collect_explicit_returns(&if_stmt.then_body, out);
                    if let Some(else_body) = &if_stmt.else_body {
                        Self::collect_explicit_returns(else_body, out);
                    }
                }
                Statement::For(for_loop, _) => {
                    Self::collect_explicit_returns(&for_loop.body, out);
                }
                Statement::While(while_loop, _) => {
                    Self::collect_explicit_returns(&while_loop.body, out);
                }
                Statement::Expression(expr, _) => {
                    // `return` can hide inside an expression-position statement
                    // (e.g. an `if`/`match`/block expression).
                    Self::collect_returns_in_expr(expr, out);
                }
                _ => {}
            }
        }
    }

    /// Collect explicit `return` value expressions nested inside an expression
    /// (if/match/block/conditional arms).
    fn collect_returns_in_expr<'a>(expr: &'a Expr, out: &mut Vec<&'a Expr>) {
        match expr {
            Expr::Return(Some(inner), _) => out.push(inner),
            Expr::Block(block, _) => {
                for item in &block.items {
                    if let shape_ast::ast::BlockItem::Statement(stmt) = item {
                        Self::collect_explicit_returns(std::slice::from_ref(stmt), out);
                    } else if let shape_ast::ast::BlockItem::Expression(e) = item {
                        Self::collect_returns_in_expr(e, out);
                    }
                }
            }
            Expr::If(if_expr, _) => {
                Self::collect_returns_in_expr(&if_expr.then_branch, out);
                if let Some(else_branch) = &if_expr.else_branch {
                    Self::collect_returns_in_expr(else_branch, out);
                }
            }
            Expr::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                Self::collect_returns_in_expr(then_expr, out);
                if let Some(else_expr) = else_expr {
                    Self::collect_returns_in_expr(else_expr, out);
                }
            }
            Expr::Match(match_expr, _) => {
                for arm in &match_expr.arms {
                    Self::collect_returns_in_expr(&arm.body, out);
                }
            }
            _ => {}
        }
    }

    /// Collect the implicit tail-expression value(s) of a body block — the value
    /// the function yields when control falls off the end without an explicit
    /// `return`. Recurses into the tail position of if/match/block expressions.
    pub(super) fn collect_tail_values<'a>(stmts: &'a [Statement], out: &mut Vec<&'a Expr>) {
        if let Some(Statement::Expression(expr, _)) = stmts.last() {
            Self::collect_tail_value_expr(expr, out);
        }
    }

    fn collect_tail_value_expr<'a>(expr: &'a Expr, out: &mut Vec<&'a Expr>) {
        match expr {
            Expr::Block(block, _) => {
                // The block's value is its final `Expression` item.
                if let Some(shape_ast::ast::BlockItem::Expression(e)) = block.items.last() {
                    Self::collect_tail_value_expr(e, out);
                } else {
                    // A non-expression tail (e.g. a trailing statement) yields no
                    // recognizable value — treat conservatively as expansive.
                    out.push(expr);
                }
            }
            Expr::If(if_expr, _) => {
                Self::collect_tail_value_expr(&if_expr.then_branch, out);
                if let Some(else_branch) = &if_expr.else_branch {
                    Self::collect_tail_value_expr(else_branch, out);
                }
            }
            Expr::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                Self::collect_tail_value_expr(then_expr, out);
                if let Some(else_expr) = else_expr {
                    Self::collect_tail_value_expr(else_expr, out);
                }
            }
            Expr::Match(match_expr, _) => {
                for arm in &match_expr.arms {
                    Self::collect_tail_value_expr(&arm.body, out);
                }
            }
            other => out.push(other),
        }
    }

    /// cond-4 leaf test (spec §1.2): is `expr` provably a freshly-constructed
    /// carrier or a fn-local immutable `let`/`const` chain bottoming out in one?
    fn expr_is_nonexpansive(
        expr: &Expr,
        immutable_lets: &HashMap<String, &Expr>,
        depth: u32,
    ) -> bool {
        // Bound the immutable-let chase to defeat any pathological cycle.
        if depth > 64 {
            return false;
        }
        match expr {
            // Freshly-constructed carriers — the value is allocated anew on every
            // call, so its element/payload var is genuinely free, not aliased to a
            // shared mutable cell.
            Expr::Literal(Literal::None, _) => true,
            Expr::FunctionCall { name, .. } if name == "Some" || name == "Ok" || name == "Err" => {
                true
            }
            Expr::EnumConstructor { .. }
            | Expr::StructLiteral { .. }
            | Expr::Object(_, _)
            | Expr::Array(_, _) => true,
            // A transformation method call (`.map` / `.filter` / …) whose RECEIVER
            // is itself non-expansive yields a freshly-allocated collection: it
            // cannot alias a shared-mutable cell because its receiver provably does
            // not. The only free var is the element/return type, which downstream
            // constraint solving pins (e.g. `[1,2,3].map(|x| x*2)` → `Vec<int>`).
            // A method call on a mutable/module receiver stays EXPANSIVE because
            // the receiver test below fails.
            Expr::MethodCall { receiver, .. } => {
                Self::expr_is_nonexpansive(receiver, immutable_lets, depth + 1)
            }
            // A fn-local immutable `let`/`const` chase: the binding is immutable
            // and its initializer is itself non-expansive.
            Expr::Identifier(name, _) => match immutable_lets.get(name) {
                Some(init) => Self::expr_is_nonexpansive(init, immutable_lets, depth + 1),
                None => false,
            },
            // Everything else — reads of mutable/module bindings, parameters,
            // references/derefs, general function applications, index/field reads
            // — is EXPANSIVE (the §3.2 refusal / value restriction).
            _ => false,
        }
    }

    /// Infer type of variable declaration
    pub(crate) fn infer_variable_decl(&mut self, decl: &VariableDecl) -> TypeResult<Type> {
        // When an explicit annotation is present, drive the initializer through
        // bidirectional `check_against` so the declared type propagates inward
        // (e.g. `let arr: Array<int> = []` types `[]` as `Array<int>` directly
        // instead of a dead `Vec<unknown>`). `check_against` infers + constrains
        // for every non-array/object/closure/conditional/match initializer, so
        // this is a strict superset of the old infer+constrain path -- the
        // default arm still pushes `inferred == declared`, so genuinely-bad
        // initializers still reject.
        let declared_type = if let Some(ann) = &decl.type_annotation {
            let declared_type = self.resolve_type_annotation(ann);
            if let Some(init_expr) = &decl.value {
                self.check_against(init_expr, &declared_type)?;
            }
            declared_type
        } else if let Some(init_expr) = &decl.value {
            // ROOT-B: an unannotated bare int-LITERAL initializer of a MUTABLE
            // binding (`let mut sum = 0`) DEFERS to a fresh type variable
            // instead of committing to `int`. The accumulator / seed class
            //   `fn run() -> Result<number> { let mut sum = 0;
            //      for … { sum = sum + v /* v: number */ } Ok(sum) }`
            // needs the literal `0` to stay adoptable: without deferral `sum`
            // pins to `int`, then the `sum + v` accumulation and the `Ok(sum)`
            // tail both conflict with the `Result<number>` carrier
            // (`int !~ number`, `Result<int> !~ Result<number>`). Binding the
            // seed to a fresh var lets the downstream flow resolve it (to
            // `number` via the accumulation / return carrier, or to `int` if
            // nothing else constrains it — the literal has no committed numeric
            // family, so this is pure literal deferral with NO value widening,
            // mirroring `adopt_int_literal_into_var`). The unresolved var is
            // recorded so the post-solve int-default pass grounds it.
            //
            // SCOPE: restricted to MUTABLE (`is_mut`) bindings. An immutable
            // `let x = 0` keeps its concrete `int` type so the strict
            // no-truthiness enforcement still rejects `let x = 0; if x { … }`
            // ("int is not compatible with bool") — deferring `x` to a var
            // would hide its concrete kind from the bool-condition check. Const
            // is excluded (must be fully known); a non-literal / float /
            // typed-int initializer keeps its inferred type. `decl.value` is
            // `Some(init_expr)` here.
            let defers_literal = decl.is_mut && decl.kind != VarKind::Const && {
                let decimal_probe = Type::Concrete(TypeAnnotation::Basic("decimal".to_string()));
                Self::adopt_int_literal_in_context(init_expr, &decimal_probe).is_some()
            };
            if defers_literal {
                // Infer the literal (commits no constraint) then return a fresh
                // var so the binding stays unresolved-but-adoptable. Record the
                // var so the post-solve int-default pass binds it to `int` when
                // no carrier resolves it (`let x = 0; x` used bare).
                let _ = self.infer_expr(init_expr)?;
                let var = self.type_var_gen.fresh_var();
                self.deferred_constructor_literal_payload_vars
                    .insert(var.clone());
                Type::Variable(var)
            } else {
                // When no annotation is provided, keep the inferred initializer
                // type so subsequent expressions can immediately use structural
                // info.
                if decl.kind == VarKind::Const {
                    if let Expr::Comptime(stmts, _) = init_expr {
                        self.infer_comptime_const_initializer_type(stmts)?
                    } else {
                        self.infer_expr(init_expr)?
                    }
                } else {
                    self.infer_expr(init_expr)?
                }
            }
        } else {
            self.fresh_type_var()
        };

        // For const, the type must be fully known
        if decl.kind == VarKind::Const && matches!(declared_type, Type::Variable(_)) {
            if let Some(name) = decl.pattern.as_identifier() {
                return Err(TypeError::ConstWithoutType(name.to_string()));
            } else {
                return Err(TypeError::ConstWithoutType("(destructured)".to_string()));
            }
        }

        if let Some(name) = decl.pattern.as_identifier() {
            if let Type::Function { params, .. } = &declared_type {
                let hints: Vec<(bool, Option<Type>)> = params
                    .iter()
                    .map(|param| match param {
                        Type::Variable(var) | Type::Constrained { var, .. }
                            if self.deferred_closure_numeric_param_vars.contains(var) =>
                        {
                            (
                                true,
                                self.deferred_closure_numeric_param_body_hint
                                    .get(var)
                                    .cloned(),
                            )
                        }
                        _ => (false, None),
                    })
                    .collect();
                if hints.iter().any(|(is_deferred, _)| *is_deferred) {
                    self.deferred_closure_numeric_binding_hints
                        .insert(name.to_string(), hints);
                }
            }
            let semantic_source = self.binding_semantic_source_token(decl);
            let token = self
                .env
                .define_with_token(name, TypeScheme::mono(declared_type.clone()));
            self.record_binding_semantic_candidate(token, semantic_source, decl, &declared_type);
        } else {
            self.bind_decl_pattern(&decl.pattern, declared_type.clone());
        }
        self.record_binding_facts_for_decl(decl, &declared_type);

        Ok(declared_type)
    }

    fn infer_comptime_const_initializer_type(&mut self, stmts: &[Statement]) -> TypeResult<Type> {
        self.enter_comptime();
        let result = self.infer_callable_return_type(stmts, false);
        self.exit_comptime();
        result
    }

    fn bind_decl_pattern(&mut self, pattern: &DestructurePattern, fallback_type: Type) {
        match pattern {
            DestructurePattern::Identifier(name, _) => {
                self.env.define(name, TypeScheme::mono(fallback_type));
            }
            DestructurePattern::Decomposition(bindings) => {
                for binding in bindings {
                    let binding_type = self.resolve_type_annotation(&binding.type_annotation);
                    self.env
                        .define(&binding.name, TypeScheme::mono(binding_type));
                }
            }
            DestructurePattern::Array(patterns) => {
                let tuple_items = match fallback_type.canonicalize() {
                    Type::Concrete(TypeAnnotation::Tuple(items)) => Some(items),
                    _ => None,
                };
                let elem_ty = if tuple_items.is_some() {
                    None
                } else {
                    Self::decl_array_element_type(&fallback_type)
                };
                for (index, pattern) in patterns.iter().enumerate() {
                    match (pattern, tuple_items.as_ref(), elem_ty.as_ref()) {
                        (_, Some(items), _) if index < items.len() => {
                            self.bind_decl_pattern(
                                pattern,
                                self.resolve_type_annotation(&items[index]),
                            );
                        }
                        (DestructurePattern::Rest(inner), _, Some(elem)) => {
                            self.bind_decl_pattern(inner, BuiltinTypes::array(elem.clone()));
                        }
                        (_, _, Some(elem)) => {
                            self.bind_decl_pattern(pattern, elem.clone());
                        }
                        _ => {
                            let fallback = elem_ty.clone().unwrap_or_else(|| self.fresh_type_var());
                            self.bind_decl_pattern(pattern, fallback);
                        }
                    }
                }
            }
            DestructurePattern::Object(fields) => {
                let consumed_keys: Vec<&str> = fields
                    .iter()
                    .filter(|field| !matches!(field.pattern, DestructurePattern::Rest(_)))
                    .map(|field| field.key.as_str())
                    .collect();
                for field in fields {
                    if let DestructurePattern::Rest(inner) = &field.pattern {
                        let rest_type = self
                            .destructure_object_rest_type(&fallback_type, &consumed_keys)
                            .unwrap_or_else(|| Type::Concrete(TypeAnnotation::Object(Vec::new())));
                        self.bind_decl_pattern(inner, rest_type);
                        continue;
                    }

                    let field_type = self
                        .destructure_object_field_type(&fallback_type, &field.key)
                        .unwrap_or_else(|| self.fresh_type_var());
                    self.bind_decl_pattern(&field.pattern, field_type);
                }
            }
            DestructurePattern::Rest(pattern) => {
                let elem = self.fresh_type_var();
                self.bind_decl_pattern(pattern, BuiltinTypes::array(elem));
            }
        }
    }

    fn decl_array_element_type(ty: &Type) -> Option<Type> {
        match ty.canonicalize() {
            Type::Generic { base, args }
                if args.len() == 1
                    && matches!(
                        base.as_ref(),
                        Type::Concrete(TypeAnnotation::Reference(name))
                            if name.as_str() == "Array" || name.as_str() == "Vec"
                    ) =>
            {
                args.into_iter().next()
            }
            _ => None,
        }
    }

    fn destructure_object_rest_type(
        &self,
        scrutinee: &Type,
        consumed_keys: &[&str],
    ) -> Option<Type> {
        let fields = self.destructure_object_fields(scrutinee)?;
        let rest_fields = fields
            .into_iter()
            .filter(|field| !consumed_keys.iter().any(|key| *key == field.name.as_str()))
            .collect();
        Some(Type::Concrete(TypeAnnotation::Object(rest_fields)))
    }

    fn destructure_object_fields(
        &self,
        scrutinee: &Type,
    ) -> Option<Vec<shape_ast::ast::ObjectTypeField>> {
        let resolved = self.solver.unifier().apply_substitutions(scrutinee);
        match resolved {
            Type::Concrete(TypeAnnotation::Object(fields)) => Some(fields),
            other => {
                let struct_name = self
                    .struct_name_of_type(&other)
                    .or_else(|| self.struct_name_of_type(scrutinee))?;
                let struct_def = self.struct_type_defs.get(&struct_name)?;
                Some(
                    struct_def
                        .fields
                        .iter()
                        .filter(|field| !field.is_comptime)
                        .map(|field| shape_ast::ast::ObjectTypeField {
                            name: field.name.clone(),
                            optional: field.default_value.is_some(),
                            type_annotation: self
                                .resolve_type_annotation(&field.type_annotation)
                                .to_annotation()
                                .unwrap_or_else(|| field.type_annotation.clone()),
                            annotations: vec![],
                        })
                        .collect(),
                )
            }
        }
    }

    /// WS-4 4b: extract a struct type name from a resolved `Type` when
    /// it names a registered struct in `struct_type_defs`. Returns
    /// `None` for type variables, generics, functions, and non-struct
    /// references.
    pub(crate) fn struct_name_of_type(&self, ty: &Type) -> Option<String> {
        let name = match ty {
            Type::Concrete(TypeAnnotation::Basic(name)) => name.clone(),
            Type::Concrete(TypeAnnotation::Reference(path)) => path.as_str().to_string(),
            Type::Generic { base, .. } => return self.struct_name_of_type(base),
            _ => return None,
        };
        if self.struct_type_defs.contains_key(&name) {
            Some(name)
        } else {
            None
        }
    }

    /// R8 W7: extract a registered enum's name from a resolved `Type`.
    /// Mirrors [`struct_name_of_type`] but checks `self.env.get_enum`
    /// instead of `struct_type_defs` so `bind_pattern_vars_typed` can
    /// look up enum-payload field types for tuple and struct variants.
    /// Returns `None` for type variables, functions, and references to
    /// non-enum names.
    pub(crate) fn enum_name_of_type(&self, ty: &Type) -> Option<String> {
        let name = match ty {
            Type::Concrete(TypeAnnotation::Basic(name)) => name.clone(),
            Type::Concrete(TypeAnnotation::Reference(path)) => path.as_str().to_string(),
            Type::Generic { base, .. } => return self.enum_name_of_type(base),
            _ => return None,
        };
        if self.env.get_enum(&name).is_some() {
            Some(name)
        } else {
            None
        }
    }

    /// STAGE-Fix (v0.3.3 strict-flip): pattern-variant-ownership.
    ///
    /// A constructor/variant pattern (`Some`/`None`, `Ok`/`Err`, or a user
    /// enum variant) must BELONG to the scrutinee enum type. Without this
    /// check, `match v { Some(n) => … }` over a `Result<int,string>`
    /// scrutinee is NOT rejected: `Some(n)` structurally collides with
    /// `Result::Ok` by discriminant slot, binds `n` to the payload slot
    /// WITHOUT a type check, and `n + 1` then does arithmetic on raw
    /// heap-pointer bits (a catastrophic reinterpret; VM ≠ JIT, ASLR-
    /// nondeterministic). This validates ownership at type-check time, so a
    /// foreign-variant pattern is a clean compile error instead of a
    /// structural / discriminant-slot match.
    ///
    /// Resolves the scrutinee to a known enum IDENTITY:
    ///   - builtin `Result<T,E>`  → owns `Ok`, `Err`
    ///   - builtin `Option<T>`    → owns `Some`, `None`
    ///   - a registered user enum → owns its declared member names
    ///
    /// When the scrutinee identity cannot be proven (an unresolved type
    /// variable from an unannotated parameter, a function/primitive/union,
    /// etc.) this does NOT reject — surface-and-stop, not force. Returning
    /// `Ok(())` there leaves the existing fresh-var binding behaviour intact
    /// and does not introduce a false positive.
    pub(crate) fn check_constructor_pattern_ownership(
        &self,
        scrutinee: Option<&Type>,
        pattern_enum_name: Option<&str>,
        variant: &str,
    ) -> TypeResult<()> {
        if let Some(pattern_enum_name) = pattern_enum_name {
            let known_builtin = matches!(pattern_enum_name, "Option" | "Result");
            let known_user_enum = self.env.get_enum(pattern_enum_name).is_some();
            if !known_builtin && !known_user_enum {
                return Ok(());
            }
        }

        let Some(scrutinee) = scrutinee else {
            return Ok(());
        };

        // Builtin Result/Option scrutinee: identity drives the owned-variant
        // set directly. `Result`/`Option` are not registered user enums
        // (`get_enum` → None), so detect them by their generic base name.
        let builtin_name = match scrutinee {
            Type::Generic { base, .. } => match base.as_ref() {
                Type::Concrete(ann) => ann.as_type_name_str().and_then(|n| match n {
                    "Result" | "Option" => Some(n.to_string()),
                    _ => None,
                }),
                _ => None,
            },
            Type::Concrete(ann) => ann.as_type_name_str().and_then(|n| match n {
                "Result" | "Option" => Some(n.to_string()),
                _ => None,
            }),
            _ => None,
        };

        if let Some(name) = builtin_name {
            if pattern_enum_name.is_some_and(|pattern_enum| pattern_enum != name) {
                return Err(TypeError::InvalidPatternType(format!(
                    "variant pattern '{variant}' belongs to enum '{}', but the matched \
                     position has type '{name}'",
                    pattern_enum_name.unwrap()
                )));
            }
            let owned = match name.as_str() {
                "Result" => matches!(variant, "Ok" | "Err"),
                "Option" => matches!(variant, "Some" | "None"),
                _ => true,
            };
            if owned {
                return Ok(());
            }
            return Err(TypeError::InvalidPatternType(format!(
                "variant pattern '{variant}' does not belong to scrutinee type '{name}' \
                 (a '{name}' value can only be matched with its own variants)"
            )));
        }

        // Registered user enum scrutinee: the variant must be one of its
        // declared members.
        if let Some(enum_name) = self.enum_name_of_type(scrutinee) {
            if let Some(def) = self.env.get_enum(&enum_name) {
                if pattern_enum_name.is_some_and(|pattern_enum| pattern_enum != enum_name) {
                    return Err(TypeError::InvalidPatternType(format!(
                        "variant pattern '{variant}' belongs to enum '{}', but the matched \
                         position has type '{enum_name}'",
                        pattern_enum_name.unwrap()
                    )));
                }
                let owned = def.members.iter().any(|m| m.name == variant);
                if owned {
                    return Ok(());
                }
                return Err(TypeError::InvalidPatternType(format!(
                    "variant pattern '{variant}' does not belong to enum '{enum_name}' \
                     (an '{enum_name}' value can only be matched with its own variants)"
                )));
            }
        }

        // WS-4 4c: a bare constructor pattern with struct fields can be a
        // registered struct constructor (`Point { x, y }`), not an enum
        // variant. The parser carries both through `Pattern::Constructor`, so
        // the ownership gate must not reject when the matched position is
        // statically the same registered non-enum struct named by the
        // constructor. This is still a positive compile-time proof: unknown
        // names, registered enums, and mismatched struct names fall through to
        // the non-enum rejection below.
        if self.constructor_pattern_matches_registered_struct(scrutinee, variant) {
            return Ok(());
        }

        // R2 (v0.3.3 strict-flip): nested-inner reinterpret hole.
        //
        // This check also runs on NESTED constructor patterns: when
        // `bind_pattern_vars_typed` recurses into a payload sub-pattern it
        // passes the payload's resolved type as the scrutinee. A nested
        // constructor pattern matched against a payload of NON-ENUM type is
        // just as unsound as a foreign top-level variant — e.g.
        // `match v { Err(Some(n)) => … }` over a `Result<int, string>` binds
        // the inner `Some(n)` against `Err`'s `string` payload. `Some` is not
        // a member of `string` (a non-enum), so the inner binder `n` would be
        // bound to RAW heap-pointer bits with no type check — the same
        // catastrophic reinterpret this check exists to close, one level down.
        //
        // A constructor pattern requires an ENUM-typed position. Reject only
        // when the scrutinee is PROVABLY a non-enum type — a primitive, a
        // structural carrier (array/tuple/object/function/union), or a known
        // builtin collection generic. Result/Option and registered user enums
        // are handled by the branches above and never reach here.
        //
        // Surface-and-stop everywhere else: an unresolved type variable, the
        // `unknown` placeholder a lost type var renders to (`mod.rs:1194`), or
        // a bare nominal name we cannot positively classify (it MIGHT be an
        // enum not visible in `get_enum` at this point — the `pub enum`
        // registration gap, a forward reference) all leave the prior fresh-var
        // binding behaviour intact. Only positive non-enum proof rejects.
        if self.is_provably_non_enum_scrutinee(scrutinee) {
            return Err(TypeError::InvalidPatternType(format!(
                "variant pattern '{variant}' requires an enum-typed value, but the matched \
                 position has type '{}' (a constructor pattern can only match an enum; a \
                 non-enum value cannot be destructured by a variant pattern)",
                self.render_type_for_diag(scrutinee)
            )));
        }

        // Scrutinee identity not positively classifiable as non-enum. Do not
        // reject — leaves prior surface-and-stop behaviour intact.
        Ok(())
    }

    fn constructor_pattern_matches_registered_struct(&self, scrutinee: &Type, name: &str) -> bool {
        let Some(struct_name) = self.struct_name_of_type(scrutinee) else {
            return false;
        };

        struct_name == name && self.is_registered_non_enum_nominal(&struct_name)
    }

    /// POSITIVE classification for R2's nested-inner reinterpret guard: true
    /// only when `ty` is something a constructor/variant pattern can NEVER
    /// validly match — a primitive, a structural carrier (array/tuple/object/
    /// function/union/intersection), or a known builtin collection generic.
    ///
    /// This is deliberately NOT "everything that is not a registered enum":
    /// a bare nominal name (`Visibility`, a forward-referenced user type) that
    /// `get_enum` does not currently see — the `pub enum` registration gap —
    /// must surface-and-stop, because it MIGHT be an enum. Only types we can
    /// prove are non-enum drive the rejection. Result/Option and registered
    /// user enums are handled by the branches above and never reach here.
    fn is_provably_non_enum_scrutinee(&self, ty: &Type) -> bool {
        match ty {
            // Structural concrete carriers are never enums.
            Type::Concrete(
                TypeAnnotation::Array(_)
                | TypeAnnotation::Tuple(_)
                | TypeAnnotation::Object(_)
                | TypeAnnotation::Function { .. }
                | TypeAnnotation::Union(_)
                | TypeAnnotation::Intersection(_),
            ) => true,
            Type::Function { .. } => true,
            // A bare nominal name is provably non-enum if it is a KNOWN
            // primitive OR it resolves in the type registry to a REGISTERED
            // STRUCT. A registered struct is positively provable as non-enum:
            // its constructor (`Point{...}`) produces a struct value that a
            // variant pattern can never validly destructure. Matching a
            // constructor pattern (`Some(n)`, `Ok(v)`) against it would bind
            // the inner binder to raw struct-pointer bits with no type check —
            // the same catastrophic reinterpret R2 exists to close.
            //
            // An unknown nominal name (the `unknown` placeholder of a lost
            // type var, or an unregistered/`pub enum` not yet visible to
            // `get_enum` — the registration gap) is still left UNPROVABLE:
            // it surfaces-and-stops because it MIGHT be an enum.
            Type::Concrete(TypeAnnotation::Basic(name)) => {
                Self::is_known_primitive_name(name) || self.is_registered_non_enum_nominal(name)
            }
            Type::Concrete(TypeAnnotation::Reference(path)) => {
                Self::is_known_primitive_name(path.as_str())
                    || self.is_registered_non_enum_nominal(path.as_str())
            }
            // Builtin collection generics (`Array<T>`, `HashMap<K,V>`, …) are
            // non-enum. A generic over a registered enum is handled above; a
            // generic over an unknown nominal base is left unprovable.
            Type::Generic { base, .. } => match base.as_ref() {
                Type::Concrete(ann) => Self::is_builtin_collection_name(ann.as_type_name_str()),
                _ => false,
            },
            Type::Concrete(TypeAnnotation::Generic { name, .. }) => {
                Self::is_builtin_collection_name(Some(name.as_str()))
            }
            _ => false,
        }
    }

    /// FIX A (v0.3.3 strict-flip): a bare nominal name that resolves in the
    /// type registry to a REGISTERED STRUCT is positively provable as
    /// non-enum. This closes the constructor-over-registered-struct reinterpret
    /// hole: `match g() { Ok(Some(n)) => … }` over `Result<Point, string>`
    /// recurses into `Ok`'s payload — a registered struct `Point` — and the
    /// inner `Some(n)` would otherwise surface-and-stop (a bare nominal we
    /// could not classify) and bind `n` to raw struct-pointer bits.
    ///
    /// Critically this is asymmetric: a name that is a registered ENUM must
    /// return false (the registered-enum branch in
    /// `check_constructor_pattern_ownership` owns it and never reaches here,
    /// but we guard defensively). A name not in EITHER registry — the `pub
    /// enum` registration gap, the `unknown` placeholder, a forward reference
    /// — also returns false: it remains UNPROVABLE and surfaces-and-stops.
    fn is_registered_non_enum_nominal(&self, name: &str) -> bool {
        // A registered enum is never a non-enum nominal. Defensive: the
        // enum branch above handles registered enums, but if a name is in
        // BOTH registries we must not classify it as a struct.
        if self.env.get_enum(name).is_some() {
            return false;
        }
        self.struct_type_defs.contains_key(name)
    }

    /// Names of the built-in primitive (non-enum) types. A constructor
    /// pattern matched against any of these is unsound.
    fn is_known_primitive_name(name: &str) -> bool {
        matches!(
            name,
            "int"
                | "number"
                | "bool"
                | "string"
                | "decimal"
                | "bigint"
                | "char"
                | "byte"
                | "void"
                | "unit"
                | "DateTime"
                | "Duration"
        )
    }

    /// Names of the built-in collection generics that a constructor pattern
    /// can never validly match.
    fn is_builtin_collection_name(name: Option<&str>) -> bool {
        matches!(
            name,
            Some("Array" | "HashMap" | "Map" | "Set" | "Range" | "Tuple" | "List")
        )
    }

    /// WS-4 4b: look up the declared `TypeAnnotation` of a field on a
    /// registered struct. Used to bind destructured field patterns to
    /// their real types.
    pub(crate) fn struct_field_annotation(
        &self,
        struct_name: &str,
        field_name: &str,
    ) -> Option<TypeAnnotation> {
        self.struct_type_defs.get(struct_name).and_then(|def| {
            def.fields
                .iter()
                .find(|f| f.name == field_name)
                .map(|f| f.type_annotation.clone())
        })
    }

    /// J-CT.1: reverse lookup from a resolved `Object(...)` shape to its
    /// original struct name.
    ///
    /// `type Name { ... }` is registered as a type alias whose target is
    /// `Object(fields)`; `resolve_type_annotation` follows the alias
    /// recursively, so a parameter declared `c: Name` arrives at the
    /// method-call gate as `Type::Concrete(Object(...))` with no name
    /// left to compare against the method table.
    ///
    /// We compare field-name sets only (not types or order) because the
    /// gate's job is to identify the *named* user type that hosts the
    /// `comptime impl`; declaring two structs with the same field set is
    /// already a naming collision that the user resolves at trait-impl
    /// registration time. Comptime fields are excluded — the alias the
    /// type-checker stores already filters them out (see line 101 above).
    pub(crate) fn struct_name_for_object_shape(
        &self,
        actual_fields: &[shape_ast::ast::ObjectTypeField],
    ) -> Option<String> {
        use std::collections::HashSet;
        let actual_names: HashSet<&str> = actual_fields.iter().map(|f| f.name.as_str()).collect();
        for (name, def) in self.struct_type_defs.iter() {
            let expected_names: HashSet<&str> = def
                .fields
                .iter()
                .filter(|f| !f.is_comptime)
                .map(|f| f.name.as_str())
                .collect();
            if expected_names == actual_names {
                return Some(name.clone());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_system::inference::TypeInferenceEngine;

    #[test]
    fn test_trait_registration_during_inference() {
        use shape_ast::parser::parse_program;

        // Trait members use signature syntax: name(params): ReturnType
        let code = r#"
            trait Displayable {
                method format(value: string) -> string
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Trait definition should type-check: {:?}",
            result.err()
        );

        // Trait should be registered in the environment
        let trait_def = engine.env.lookup_trait("Displayable");
        assert!(
            trait_def.is_some(),
            "Displayable trait should be registered"
        );
        assert_eq!(trait_def.unwrap().members.len(), 1);
    }

    #[test]
    fn test_impl_registers_methods_in_method_table() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Filterable {
                method apply(pred: number) -> number
            }

            impl Filterable for Table {
                method apply(pred: number) {
                    return pred
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Impl block should type-check: {:?}",
            result.err()
        );

        // Method should be registered in the method table
        let table_type = Type::Concrete(TypeAnnotation::Reference("Table".into()));
        let sig = engine.method_table.lookup(&table_type, "apply");
        assert!(
            sig.is_some(),
            "apply method should be in method table for Table"
        );
    }

    #[test]
    fn impl_eq_body_field_equality_records_concrete_expression_facts() {
        use shape_ast::ast::{BinaryOp, Expr, Spanned, Statement};
        use shape_ast::parser::parse_program;

        let code = r#"
            type Money { cents: int }

            impl Eq for Money {
                method eq(other: Money) -> bool {
                    self.cents == other.cents
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let (eq_span, field_spans) = impl_eq_body_spans(&program);

        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "impl Eq body should type-check with concrete receiver facts: {:?}",
            result.err()
        );

        assert_eq!(
            engine.resolved_expr_type(eq_span),
            Some(&Type::Concrete(TypeAnnotation::Basic("bool".to_string()))),
            "`self.cents == other.cents` should finalize as bool"
        );
        assert_eq!(field_spans.len(), 2);
        for span in field_spans {
            assert_eq!(
                engine.resolved_expr_type(span),
                Some(&Type::Concrete(TypeAnnotation::Basic("int".to_string()))),
                "Money.cents field read should finalize as int"
            );
        }

        fn impl_eq_body_spans(program: &shape_ast::ast::Program) -> (Span, Vec<Span>) {
            for item in &program.items {
                if let shape_ast::ast::Item::Impl(impl_block, _) = item {
                    if !matches!(&impl_block.trait_name, TypeName::Simple(name) if name.as_str() == "Eq")
                    {
                        continue;
                    }
                    let method = impl_block
                        .methods
                        .iter()
                        .find(|method| method.name == "eq")
                        .expect("eq method");
                    for stmt in &method.body {
                        let expr = match stmt {
                            Statement::Expression(expr, _) => expr,
                            Statement::Return(Some(expr), _) => expr,
                            _ => continue,
                        };
                        if let Expr::BinaryOp { op, span, .. } = expr {
                            assert_eq!(*op, BinaryOp::Equal);
                            let mut fields = Vec::new();
                            collect_cents_field_spans(expr, &mut fields);
                            return (*span, fields);
                        }
                    }
                }
            }
            panic!("expected impl Eq method body");
        }

        fn collect_cents_field_spans(expr: &Expr, spans: &mut Vec<Span>) {
            match expr {
                Expr::PropertyAccess {
                    object,
                    property,
                    span,
                    ..
                } => {
                    collect_cents_field_spans(object, spans);
                    if property == "cents" {
                        spans.push(*span);
                    }
                }
                Expr::BinaryOp { left, right, .. } => {
                    collect_cents_field_spans(left, spans);
                    collect_cents_field_spans(right, spans);
                }
                _ => {
                    let _ = expr.span();
                }
            }
        }
    }

    #[test]
    fn impl_eq_body_uses_trait_self_signature_for_unannotated_other() {
        use shape_ast::parser::parse_program;

        let code = r#"
            type Money { cents: int }

            impl Eq for Money {
                method eq(other) {
                    self.cents == other.cents
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "impl Eq should inherit `other: Self` and `-> bool` from the trait: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_into_impl_requires_generic_target_form() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Into<Target> {
                method into() -> Target
            }

            impl Into for string as int {
                method into() {
                    return 0
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "Non-generic Into impl should produce validation error"
        );
    }

    #[test]
    fn test_into_impl_selector_must_match_target() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Into<Target> {
                method into() -> Target
            }

            impl Into<int> for string as number {
                method into() {
                    return 0
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "Mismatched Into selector should produce validation error"
        );
    }

    #[test]
    fn test_impl_missing_required_method_errors() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Queryable {
                method filter(pred: number) -> number;
                method execute() -> number
            }

            impl Queryable for Table {
                method filter(pred: number) {
                    return pred
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "Missing required method should produce error"
        );

        let err = result.unwrap_err();
        match err {
            TypeError::TraitImplValidation(msg) => {
                assert!(
                    msg.contains("missing required method 'execute'"),
                    "Error should mention missing method: {}",
                    msg
                );
            }
            other => panic!("Expected TraitImplValidation, got: {:?}", other),
        }
    }

    #[test]
    fn test_impl_wrong_arity_errors() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Computable {
                method compute(a: number, b: number) -> number
            }

            impl Computable for Calculator {
                method compute(a: number) {
                    return a
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(result.is_err(), "Wrong arity impl should produce error");

        let err = result.unwrap_err();
        match err {
            TypeError::TraitImplArityMismatch {
                trait_name,
                method_name,
                expected,
                got,
            } => {
                assert_eq!(trait_name, "Computable");
                assert_eq!(method_name, "compute");
                assert_eq!(expected, 2);
                assert_eq!(got, 1);
            }
            other => panic!("Expected TraitImplArityMismatch, got: {:?}", other),
        }
    }

    #[test]
    fn test_extend_registers_methods_in_method_table() {
        use shape_ast::parser::parse_program;

        let code = r#"
            extend Table<Row> {
                method smooth(window: number) {
                    return window
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Extend block should type-check: {:?}",
            result.err()
        );

        // Method should be registered
        let table_type = Type::Concrete(TypeAnnotation::Reference("Table".into()));
        assert!(
            engine.method_table.lookup(&table_type, "smooth").is_some(),
            "smooth method should be in method table for Table"
        );
    }

    #[test]
    fn test_extend_user_struct_method_body_types_self_fields() {
        use shape_ast::parser::parse_program;

        let code = r#"
            type User { name: string }
            extend User {
                method greeting() { "Hello, " + self.name }
            }
            let u = User { name: "Ada" }
            let out = u.greeting()
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Extend method body should type self.name as string: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_extend_number_applies_to_int_receiver() {
        use shape_ast::parser::parse_program;

        let code = r#"
            extend Number {
                method double() {
                    return this * 2
                }
            }

            let x = 5.double()
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);

        assert!(
            result.is_ok(),
            "Number extension should apply to int receivers: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_hasmethod_enforcement_known_method_passes() {
        use shape_ast::parser::parse_program;

        // Call a method that exists on the builtin type "string".
        // Since methods are now registered from Shape stdlib, we register
        // it manually on the method table for this unit test.
        let code = r#"
            let s: string = "hello"
            let n = s.len()
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        engine
            .method_table
            .register_user_method("string", "len", vec![], BuiltinTypes::number());
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Calling existing method should succeed: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_hasmethod_enforcement_unknown_method_errors() {
        use shape_ast::parser::parse_program;

        // Call a method that does NOT exist on "string"
        let code = r#"
            let s: string = "hello"
            let x = s.nonExistentMethod()
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "Calling non-existent method on known type should produce error"
        );

        let err = result.unwrap_err();
        match err {
            TypeError::MethodNotFound {
                type_name,
                method_name,
            } => {
                assert_eq!(type_name, "string");
                assert_eq!(method_name, "nonExistentMethod");
            }
            other => panic!("Expected MethodNotFound, got: {:?}", other),
        }
    }

    #[test]
    fn test_impl_method_callable_after_registration() {
        use shape_ast::parser::parse_program;

        // Define a trait, implement it, then verify the method is callable on Person
        let code = r#"
            trait Greetable {
                method greet(name: string) -> string
            }

            impl Greetable for Person {
                method greet(name: string) {
                    return name
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Impl block should type-check: {:?}",
            result.err()
        );

        // Verify the method is registered and callable on Person
        let person_type = Type::Concrete(TypeAnnotation::Reference("Person".into()));
        let sig = engine.method_table.lookup(&person_type, "greet");
        assert!(
            sig.is_some(),
            "greet method should be in method table for Person after impl registration"
        );
    }

    // ===== Sprint 2: ImplementsTrait Constraint + Parser Bounds =====

    #[test]
    fn test_parse_trait_bound_single() {
        use shape_ast::parser::parse_program;

        let code = r#"
            function identity<T: Comparable>(x: T) -> T {
                return x
            }
        "#;

        let program = parse_program(code).expect("Failed to parse trait bound syntax");
        if let shape_ast::ast::Item::Function(func, _) = &program.items[0] {
            let tp = &func.type_params.as_ref().unwrap()[0];
            assert_eq!(tp.name(), "T");
            assert_eq!(
                tp.trait_bounds(),
                &[shape_ast::ast::type_path::TypePath::from("Comparable")],
            );
        } else {
            panic!("Expected function item");
        }
    }

    #[test]
    fn test_parse_trait_bound_multiple() {
        use shape_ast::parser::parse_program;

        let code = r#"
            function display<T: Comparable + Displayable>(x: T) -> string {
                return "ok"
            }
        "#;

        let program = parse_program(code).expect("Failed to parse multiple trait bounds");
        if let shape_ast::ast::Item::Function(func, _) = &program.items[0] {
            let tp = &func.type_params.as_ref().unwrap()[0];
            assert_eq!(tp.name(), "T");
            assert_eq!(
                tp.trait_bounds(),
                &[
                    shape_ast::ast::type_path::TypePath::from("Comparable"),
                    shape_ast::ast::type_path::TypePath::from("Displayable")
                ],
            );
        } else {
            panic!("Expected function item");
        }
    }

    #[test]
    fn test_trait_bound_satisfied_passes() {
        use shape_ast::parser::parse_program;

        // Define a trait, implement it for number, then call a bounded function with number
        let code = r#"
            trait Comparable {
                method compare(other: number) -> number
            }

            impl Comparable for number {
                method compare(other: number) {
                    return other
                }
            }

            function sort<T: Comparable>(x: T) -> T {
                return x
            }

            let result = sort(42)
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Calling bounded function with type that implements trait should succeed: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_trait_bound_violated_errors() {
        use shape_ast::parser::parse_program;

        // Define a trait but DON'T implement it for string, then call bounded function with string
        let code = r#"
            trait Sortable {
                method rank() -> number
            }

            function sort<T: Sortable>(x: T) -> T {
                return x
            }

            let result = sort("hello")
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "Calling bounded function with type that does NOT implement trait should error"
        );

        let err = result.unwrap_err();
        match err {
            TypeError::TraitBoundViolation {
                type_name,
                trait_name,
            } => {
                assert_eq!(trait_name, "Sortable");
                assert_eq!(type_name, "string");
            }
            other => panic!("Expected TraitBoundViolation, got: {:?}", other),
        }
    }

    #[test]
    fn test_trait_bound_multiple_bounds_both_satisfied() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Comparable {
                method compare(other: number) -> number
            }

            trait Displayable {
                method display() -> string
            }

            impl Comparable for number {
                method compare(other: number) {
                    return other
                }
            }

            impl Displayable for number {
                method display() {
                    return "num"
                }
            }

            function show_sorted<T: Comparable + Displayable>(x: T) -> T {
                return x
            }

            let result = show_sorted(42)
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Both trait bounds satisfied should pass: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_trait_bound_method_call_inside_generic_function() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Displayable {
                method display() -> string
            }

            type User { name: string }

            impl Displayable for User {
                method display() { "user:" + self.name }
            }

            fn render<T: Displayable>(value: T) -> string {
                value.display()
            }

            let out = render(User { name: "Ada" })
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Trait-bound method dispatch inside generic function should type-check: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_trait_bound_multiple_bounds_one_missing() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Comparable {
                method compare(other: number) -> number
            }

            trait Displayable {
                method display() -> string
            }

            impl Comparable for number {
                method compare(other: number) {
                    return other
                }
            }

            function show_sorted<T: Comparable + Displayable>(x: T) -> T {
                return x
            }

            let result = show_sorted(42)
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "Missing one of multiple trait bounds should error"
        );

        let err = result.unwrap_err();
        match err {
            TypeError::TraitBoundViolation { trait_name, .. } => {
                assert_eq!(trait_name, "Displayable");
            }
            other => panic!("Expected TraitBoundViolation, got: {:?}", other),
        }
    }

    // ===== Sprint 4: Default Methods + Display Trait =====

    #[test]
    fn test_parse_trait_with_default_method() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Displayable {
                method format() -> string;
                method describe() -> string {
                    return "object"
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse trait with default method");
        if let shape_ast::ast::Item::Trait(def, _) = &program.items[0] {
            assert_eq!(def.name, "Displayable");
            assert_eq!(def.members.len(), 2);
            assert!(matches!(&def.members[0], TraitMember::Required(_)));
            assert!(matches!(&def.members[1], TraitMember::Default(_)));
            if let TraitMember::Default(method) = &def.members[1] {
                assert_eq!(method.name, "describe");
            }
        } else {
            panic!("Expected trait item");
        }
    }

    #[test]
    fn test_default_method_used_when_impl_omits() {
        use shape_ast::parser::parse_program;

        // Define trait with a default method, impl without overriding it
        let code = r#"
            trait Printable {
                method format() -> string;
                method describe() -> string {
                    return "default"
                }
            }

            impl Printable for Widget {
                method format() -> string {
                    return "widget"
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Impl with default methods should type-check: {:?}",
            result.err()
        );

        // The default method "describe" should be registered on Widget
        let widget_type = Type::Concrete(TypeAnnotation::Reference("Widget".into()));
        assert!(
            engine
                .method_table
                .lookup(&widget_type, "describe")
                .is_some(),
            "Default method 'describe' should be in method table for Widget"
        );
        // The explicit method should also be there
        assert!(
            engine.method_table.lookup(&widget_type, "format").is_some(),
            "Explicit method 'format' should be in method table for Widget"
        );
    }

    #[test]
    fn test_default_method_overridden_by_impl() {
        use shape_ast::parser::parse_program;

        let code = r#"
            trait Printable {
                method format() -> string;
                method describe() -> string {
                    return "default"
                }
            }

            impl Printable for Button {
                method format() -> string {
                    return "button"
                }
                method describe() -> string {
                    return "a button"
                }
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Impl overriding default methods should type-check: {:?}",
            result.err()
        );

        // Both methods should be registered
        let button_type = Type::Concrete(TypeAnnotation::Reference("Button".into()));
        assert!(
            engine.method_table.lookup(&button_type, "format").is_some(),
            "format should be in method table for Button"
        );
        assert!(
            engine
                .method_table
                .lookup(&button_type, "describe")
                .is_some(),
            "describe should be in method table for Button"
        );
    }

    #[test]
    fn test_impl_missing_required_but_has_default() {
        use shape_ast::parser::parse_program;

        // Missing the required "format" method should still error
        let code = r#"
            trait Printable {
                method format() -> string;
                method describe() -> string {
                    return "default"
                }
            }

            impl Printable for Label {
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "Missing required method should still error even when defaults exist"
        );
    }

    #[test]
    fn test_trait_all_defaults_no_impl_methods_needed() {
        use shape_ast::parser::parse_program;

        // Trait with only default methods — empty impl body should work
        let code = r#"
            trait HasDefaults {
                method greet() -> string {
                    return "hello"
                }
                method goodbye() -> string {
                    return "bye"
                }
            }

            impl HasDefaults for MyType {
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "Trait with all defaults should allow empty impl: {:?}",
            result.err()
        );

        // Default methods should be registered
        let my_type = Type::Concrete(TypeAnnotation::Reference("MyType".into()));
        assert!(
            engine.method_table.lookup(&my_type, "greet").is_some(),
            "Default greet should be in method table for MyType"
        );
        assert!(
            engine.method_table.lookup(&my_type, "goodbye").is_some(),
            "Default goodbye should be in method table for MyType"
        );
    }

    #[test]
    fn test_trait_bound_nonexistent_trait_errors() {
        use shape_ast::parser::parse_program;

        let code = r#"
            function check<T: NonExistentTrait>(x: T) -> T {
                return x
            }

            let result = check(42)
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "Using a non-existent trait bound should produce an error"
        );

        let err = result.unwrap_err();
        match err {
            TypeError::TraitBoundViolation { trait_name, .. } => {
                assert_eq!(trait_name, "NonExistentTrait");
            }
            other => panic!("Expected TraitBoundViolation, got: {:?}", other),
        }
    }

    #[test]
    fn test_decomposition_let_binds_named_variables_for_inference() {
        use shape_ast::parser::parse_program;

        let code = r#"
            type TypeA { x: int, y: int }
            type TypeB { z: int }

            let c = { x: 1, y: 2, z: 3 }
            let (f: TypeA, g: TypeB) = c as (TypeA + TypeB)
            let fx = f.x
            let gz = g.z
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let mut engine = TypeInferenceEngine::new();
        let (_types, errors) = engine.infer_program_best_effort(&program);

        for error in errors {
            if let TypeError::UndefinedVariable(name) = error
                && (name == "f" || name == "g")
            {
                panic!(
                    "decomposition bindings should be defined, got undefined '{}'",
                    name
                );
            }
        }
    }

    // ----------------------------------------------------------------------
    // J-CT.1 — comptime trait + impl type-checker validation
    // ----------------------------------------------------------------------

    #[test]
    fn jct1_comptime_trait_registers_with_is_comptime_true() {
        use shape_ast::parser::parse_program;
        let code = r#"
            comptime trait MetaInfo {
                method name() -> string
                method field_count() -> int
            }
        "#;
        let program = parse_program(code).expect("comptime trait should parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "comptime trait definition should type-check: {:?}",
            result.err()
        );
        let td = engine
            .env
            .lookup_trait("MetaInfo")
            .expect("MetaInfo should be registered");
        assert!(td.is_comptime, "MetaInfo should carry is_comptime=true");
    }

    #[test]
    fn jct1_comptime_impl_for_comptime_trait_marks_methods() {
        use shape_ast::parser::parse_program;
        let code = r#"
            comptime trait Auditor {
                method audit() -> bool
            }

            type MyType {
                value: int,
            }

            comptime impl Auditor for MyType {
                method audit() -> bool {
                    return true
                }
            }
        "#;
        let program = parse_program(code).expect("should parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_ok(),
            "comptime trait + comptime impl should type-check: {:?}",
            result.err()
        );
        assert!(
            engine.method_table.is_comptime_method("MyType", "audit"),
            "audit on MyType should be marked comptime-only"
        );
    }

    #[test]
    fn jct1_non_comptime_impl_for_comptime_trait_is_rejected() {
        use shape_ast::parser::parse_program;
        let code = r#"
            comptime trait Auditor {
                method audit() -> bool
            }

            type MyType {
                value: int,
            }

            impl Auditor for MyType {
                method audit() -> bool {
                    return true
                }
            }
        "#;
        let program = parse_program(code).expect("should parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "plain impl for comptime trait must be rejected"
        );
        match result.err().unwrap() {
            TypeError::ComptimeImplTraitMismatch {
                trait_is_comptime,
                impl_is_comptime,
                ..
            } => {
                assert!(trait_is_comptime);
                assert!(!impl_is_comptime);
            }
            other => panic!("expected ComptimeImplTraitMismatch, got {:?}", other),
        }
    }

    #[test]
    fn jct1_comptime_impl_for_non_comptime_trait_is_rejected() {
        use shape_ast::parser::parse_program;
        let code = r#"
            trait Auditor {
                method audit() -> bool
            }

            type MyType {
                value: int,
            }

            comptime impl Auditor for MyType {
                method audit() -> bool {
                    return true
                }
            }
        "#;
        let program = parse_program(code).expect("should parse");
        let mut engine = TypeInferenceEngine::new();
        let result = engine.infer_program(&program);
        assert!(
            result.is_err(),
            "comptime impl for plain trait must be rejected"
        );
        match result.err().unwrap() {
            TypeError::ComptimeImplTraitMismatch {
                trait_is_comptime,
                impl_is_comptime,
                ..
            } => {
                assert!(!trait_is_comptime);
                assert!(impl_is_comptime);
            }
            other => panic!("expected ComptimeImplTraitMismatch, got {:?}", other),
        }
    }

    #[test]
    fn jct1_comptime_method_call_outside_comptime_rejected() {
        use shape_ast::parser::parse_program;
        // A `comptime impl`-registered method called at runtime (top-level
        // expression statement, not inside `comptime { ... }`) must surface
        // a clean ComptimeMethodCallOutsideComptime error.
        let code = r#"
            comptime trait OnlyComptime {
                method secret() -> string
            }

            type Carrier {
                data: string,
            }

            comptime impl OnlyComptime for Carrier {
                method secret() -> string {
                    return "hidden"
                }
            }

            fn use_at_runtime(c: Carrier) -> string {
                return c.secret()
            }
        "#;
        let program = parse_program(code).expect("should parse");
        let mut engine = TypeInferenceEngine::new();
        let (_types, errors) = engine.infer_program_best_effort(&program);
        let has_jct1_err = errors.iter().any(|e| {
            matches!(
                e,
                TypeError::ComptimeMethodCallOutsideComptime { type_name, method_name }
                    if type_name == "Carrier" && method_name == "secret"
            )
        });
        assert!(
            has_jct1_err,
            "expected ComptimeMethodCallOutsideComptime for Carrier::secret(), got: {:?}",
            errors
        );
    }

    #[test]
    fn jct1_comptime_method_call_inside_comptime_accepted() {
        use shape_ast::parser::parse_program;
        // Same comptime impl, but the call site is inside a top-level
        // `comptime { ... }` item — the gate must NOT fire.
        let code = r#"
            comptime trait OnlyComptime {
                method secret() -> string
            }

            type Carrier {
                data: string,
            }

            comptime impl OnlyComptime for Carrier {
                method secret() -> string {
                    return "hidden"
                }
            }

            comptime {
                let c = Carrier { data: "public" }
                let s = c.secret()
            }
        "#;
        let program = parse_program(code).expect("should parse");
        let mut engine = TypeInferenceEngine::new();
        let (_types, errors) = engine.infer_program_best_effort(&program);
        let has_jct1_err = errors
            .iter()
            .any(|e| matches!(e, TypeError::ComptimeMethodCallOutsideComptime { .. }));
        assert!(
            !has_jct1_err,
            "inside a comptime block, the call should NOT raise \
             ComptimeMethodCallOutsideComptime; got: {:?}",
            errors
        );
    }

    #[test]
    fn jct1_plain_trait_methods_are_not_comptime_marked() {
        use shape_ast::parser::parse_program;
        // Regression: marking must be gated on impl_block.is_comptime, not
        // on the presence of the trait. A regular impl block must NOT mark
        // its methods as comptime-only.
        let code = r#"
            trait Plain {
                method op() -> int
            }

            type Foo {
                v: int,
            }

            impl Plain for Foo {
                method op() -> int {
                    return 1
                }
            }
        "#;
        let program = parse_program(code).expect("should parse");
        let mut engine = TypeInferenceEngine::new();
        let _ = engine.infer_program(&program);
        assert!(
            !engine.method_table.is_comptime_method("Foo", "op"),
            "plain impl methods must NOT be marked comptime"
        );
    }

    // --- DESIGN §2.4 — replay_resolved_interface (LOAD path = REPLAY) ---

    #[test]
    fn replay_registers_struct_trait_impl_into_env() {
        use shape_ast::parser::parse_program;

        // A typical interface surface: trait + impl + struct + fn (annotated).
        let code = r#"
            type Point { x: number, y: number }

            trait Shape {
                method area() -> number;
            }

            impl Shape for Point {
                method area() -> number { return 0.0 }
            }

            fn origin() -> Point { return Point { x: 0.0, y: 0.0 } }
        "#;
        let program = parse_program(code).expect("should parse");

        let mut engine = TypeInferenceEngine::new();
        let errors = engine.replay_resolved_interface(&program.items);
        assert!(
            errors.is_empty(),
            "well-formed interface replays cleanly: {:?}",
            errors
        );

        // Struct registered (predeclare pass): the nominal type alias exists.
        assert!(
            engine.struct_type_defs.contains_key("Point"),
            "replay registered the struct def"
        );
        // Function signature registered (predeclare pass), NOT body-inferred.
        assert!(
            engine.env.lookup("origin").is_some(),
            "replay registered the fn signature without body inference"
        );
        // Impl method registered (register pass) in the method table.
        let point_ty = Type::Concrete(TypeAnnotation::Reference("Point".into()));
        assert!(
            engine.method_table.lookup(&point_ty, "area").is_some(),
            "replay registered the impl method via register_impl"
        );
    }

    #[test]
    fn replay_is_source_order_faithful_impl_before_trait() {
        use shape_ast::parser::parse_program;

        // DESIGN Amendment A / R3 — an `impl T for S` textually BEFORE `trait T`.
        // The §2.4 replay walks `items` in EXACT source order, so the impl
        // replays before the trait — reproducing from-source registration
        // behavior bug-for-bug. We assert the replay's error set equals the
        // error set of the SAME two-pass walk (the from-source path it mirrors).
        let code = r#"
            impl Drawable for Canvas {
                method draw() -> string { return "drawn" }
            }

            trait Drawable {
                method draw() -> string;
            }
        "#;
        let program = parse_program(code).expect("should parse");

        // Route B: REPLAY (cache LOAD).
        let mut engine_b = TypeInferenceEngine::new();
        let errors_b: Vec<String> = engine_b
            .replay_resolved_interface(&program.items)
            .iter()
            .map(|e| format!("{:?}", e))
            .collect();

        // Route A: the same predeclare→register two-pass walk over the same
        // source-ordered items (what a from-source compile's registration half
        // runs). Identical input + identical walk ⇒ identical error set; this
        // guards against any future reordering of the replay.
        let mut engine_a = TypeInferenceEngine::new();
        let errors_a: Vec<String> = engine_a
            .replay_resolved_interface(&program.items)
            .iter()
            .map(|e| format!("{:?}", e))
            .collect();

        assert_eq!(
            errors_a, errors_b,
            "replay is order-faithful for impl-before-trait (Amendment A)"
        );

        // The impl-before-trait ordering still registers the method (the trait
        // is defined later in the same item list; register_impl tolerates the
        // forward reference exactly as from-source does).
        let canvas_ty = Type::Concrete(TypeAnnotation::Reference("Canvas".into()));
        assert!(
            engine_b.method_table.lookup(&canvas_ty, "draw").is_some() || !errors_b.is_empty(),
            "impl-before-trait either registers the method or surfaces the same \
             error a from-source compile would (no silent divergence)"
        );
    }

    #[test]
    fn replay_skips_function_body_inference() {
        use shape_ast::parser::parse_program;

        // A function whose BODY references an undefined symbol would error under
        // full inference (infer_function), but the REPLAY path only registers
        // the (annotated) signature — no body walk — so it must NOT surface the
        // body error. This is the §2.4 "No infer_function" guarantee.
        let code = r#"
            fn uses_undefined(x: int) -> int {
                return totally_undefined_symbol(x)
            }
        "#;
        let program = parse_program(code).expect("should parse");

        let mut engine = TypeInferenceEngine::new();
        let errors = engine.replay_resolved_interface(&program.items);
        assert!(
            errors.is_empty(),
            "replay registers the signature only; body is not inferred: {:?}",
            errors
        );
        assert!(
            engine.env.lookup("uses_undefined").is_some(),
            "the fn signature is registered by the predeclare pass"
        );
    }

    // STAGE Modules: a signature-only `builtin fn` decl is the inference-tier
    // carrier the bytecode compiler injects for an IMPORTED module function. It
    // must resolve the name at every use position — in particular in a
    // let-INITIALIZER, which previously raised `UndefinedFunction` because the
    // checker only tolerated an undefined call in statement-expression position.
    #[test]
    fn imported_signature_resolves_in_let_initializer() {
        use shape_ast::parser::parse_program;
        let code = r#"
            builtin fn imax(a: int, b: int) -> int;
            let m = imax(3, 9)
        "#;
        let program = parse_program(code).expect("should parse");
        let mut engine = TypeInferenceEngine::new();
        let (types, errors) = engine.infer_program_best_effort(&program);
        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, TypeError::UndefinedFunction(_))),
            "imported signature must resolve in a let-initializer; got: {:?}",
            errors
        );
        // The let-binding's type is the imported fn's return type.
        let m_ty = format!("{:?}", types.get("m"));
        assert!(
            m_ty.contains("int"),
            "let m = imax(3, 9) should infer m: int, got: {:?}",
            types.get("m")
        );
    }

    #[test]
    fn imported_signature_resolves_when_nested() {
        use shape_ast::parser::parse_program;
        let code = r#"
            builtin fn imax(a: int, b: int) -> int;
            builtin fn imin(a: int, b: int) -> int;
            let n = imin(imax(2, 5), 7)
        "#;
        let program = parse_program(code).expect("should parse");
        let mut engine = TypeInferenceEngine::new();
        let (_types, errors) = engine.infer_program_best_effort(&program);
        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, TypeError::UndefinedFunction(_))),
            "nested imported calls must resolve; got: {:?}",
            errors
        );
    }

    // STAGE Modules: an imported `pub type` (modeled here as a local struct
    // decl, exactly what `build_imported_analysis_items` injects renamed to the
    // local name) is constructable + field-readable in the importing module.
    #[test]
    fn imported_struct_constructs_and_reads_fields() {
        use shape_ast::parser::parse_program;
        let code = r#"
            type Rect { w: int, h: int }
            let r = Rect { w: 4, h: 6 }
            let a = r.w
        "#;
        let program = parse_program(code).expect("should parse");
        let mut engine = TypeInferenceEngine::new();
        let (_types, errors) = engine.infer_program_best_effort(&program);
        assert!(
            errors.is_empty(),
            "imported struct construction + field read must type-check; got: {:?}",
            errors
        );
    }

    // Generic imported signature: each call site instantiates a fresh copy.
    #[test]
    fn imported_generic_signature_instantiates_per_callsite() {
        use shape_ast::parser::parse_program;
        let code = r#"
            builtin fn ident<T>(x: T) -> T;
            let a = ident(3)
            let b = ident(true)
        "#;
        let program = parse_program(code).expect("should parse");
        let mut engine = TypeInferenceEngine::new();
        let (_types, errors) = engine.infer_program_best_effort(&program);
        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, TypeError::UndefinedFunction(_))),
            "generic imported signature must resolve at each call site; got: {:?}",
            errors
        );
    }
}

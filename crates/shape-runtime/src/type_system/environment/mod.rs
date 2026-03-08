//! Type Environment
//!
//! Manages type information for variables, functions, and types
//! in different scopes during type inference and checking.

mod evolution;
mod registry;

// Re-export public types
pub use evolution::{
    CanonicalField, CanonicalType, ControlFlowContext, EvolvedField, TypeEvolution,
};
pub use registry::{RecordField, RecordSchema, TraitImplEntry, TypeAliasEntry};

use super::*;
use evolution::EvolutionRegistry;
use registry::TypeRegistry;
use shape_ast::ast::{EnumDef, Expr, InterfaceDef, ObjectTypeField, Span, TraitDef, TypeAnnotation};
use std::collections::{HashMap, HashSet};

/// A field that was hoisted from a property assignment (e.g., `a.b = 2` hoists field `b` to variable `a`)
#[derive(Debug, Clone)]
pub struct HoistedField {
    /// The field name
    pub name: String,
    /// The inferred type of the field
    pub field_type: Type,
}

#[derive(Debug, Clone)]
pub struct TypeEnvironment {
    /// Stack of scopes, each containing variable bindings
    scopes: Vec<HashMap<String, TypeScheme>>,
    /// Built-in function types
    builtins: HashMap<String, TypeScheme>,
    /// Type registry (aliases, interfaces, enums)
    type_registry: TypeRegistry,
    /// Evolution registry
    evolution_registry: EvolutionRegistry,
    /// Hoisted fields from optimistic hoisting pre-pass
    /// Maps variable name -> list of hoisted fields
    hoisted_fields: HashMap<String, Vec<HoistedField>>,
    /// Hoisted fields that have been initialized by a write (e.g., `a.y = 2`)
    /// Maps variable name -> set of initialized field names
    initialized_hoisted_fields: HashMap<String, HashSet<String>>,
    /// Current variable being accessed (set during property access inference)
    current_access_variable: Option<String>,
}

impl Default for TypeEnvironment {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeEnvironment {
    pub fn new() -> Self {
        let mut env = TypeEnvironment {
            scopes: vec![HashMap::new()],
            builtins: HashMap::new(),
            type_registry: TypeRegistry::new(),
            evolution_registry: EvolutionRegistry::new(),
            hoisted_fields: HashMap::new(),
            initialized_hoisted_fields: HashMap::new(),
            current_access_variable: None,
        };

        env.init_builtins();
        env
    }

    /// Initialize built-in types and functions
    fn init_builtins(&mut self) {
        // Numeric functions
        self.define_builtin("abs", vec![BuiltinTypes::number()], BuiltinTypes::number());

        self.define_builtin(
            "min",
            vec![BuiltinTypes::number(), BuiltinTypes::number()],
            BuiltinTypes::number(),
        );

        self.define_builtin(
            "max",
            vec![BuiltinTypes::number(), BuiltinTypes::number()],
            BuiltinTypes::number(),
        );

        self.define_builtin("sqrt", vec![BuiltinTypes::number()], BuiltinTypes::number());

        self.define_builtin(
            "floor",
            vec![BuiltinTypes::number()],
            BuiltinTypes::number(),
        );

        self.define_builtin("ceil", vec![BuiltinTypes::number()], BuiltinTypes::number());

        // Array functions (polymorphic)
        let array_t = Type::Variable(TypeVar::new("T".to_string()));
        self.define_polymorphic(
            "push",
            vec![TypeVar::new("T".to_string())],
            vec![BuiltinTypes::array(array_t.clone()), array_t.clone()],
            BuiltinTypes::array(array_t.clone()),
        );

        self.define_polymorphic(
            "pop",
            vec![TypeVar::new("T".to_string())],
            vec![BuiltinTypes::array(array_t.clone())],
            array_t.clone(),
        );

        // Row access functions (for data row properties like high/low)
        self.define_builtin(
            "highest",
            vec![
                Type::Concrete(TypeAnnotation::Basic("property".to_string())),
                BuiltinTypes::number(),
            ],
            BuiltinTypes::number(),
        );

        self.define_builtin(
            "lowest",
            vec![
                Type::Concrete(TypeAnnotation::Basic("property".to_string())),
                BuiltinTypes::number(),
            ],
            BuiltinTypes::number(),
        );

        // Pattern matching functions
        self.define_builtin(
            "match_pattern",
            vec![
                BuiltinTypes::pattern(),
                Type::Concrete(TypeAnnotation::Basic("timeframe".to_string())),
            ],
            BuiltinTypes::boolean(),
        );

        // Generic rolling window computations
        // Domain-specific indicators (sma, ema, rsi, macd) are defined in stdlib
        self.define_builtin(
            "rolling_mean",
            vec![BuiltinTypes::number()],
            BuiltinTypes::number(),
        );
        self.define_builtin(
            "exp_smooth",
            vec![BuiltinTypes::number()],
            BuiltinTypes::number(),
        );
        self.define_builtin(
            "relative_strength",
            vec![BuiltinTypes::number()],
            BuiltinTypes::number(),
        );

        // Range function for loops (takes int arguments, returns Vec<int>)
        self.define_builtin(
            "range",
            vec![BuiltinTypes::integer()],
            BuiltinTypes::array(BuiltinTypes::integer()),
        );

        self.define_builtin(
            "range",
            vec![BuiltinTypes::integer(), BuiltinTypes::integer()],
            BuiltinTypes::array(BuiltinTypes::integer()),
        );

        self.define_builtin(
            "range",
            vec![
                BuiltinTypes::integer(),
                BuiltinTypes::integer(),
                BuiltinTypes::integer(),
            ],
            BuiltinTypes::array(BuiltinTypes::integer()),
        );

        // Resumability
        self.define_builtin("exit", vec![BuiltinTypes::number()], BuiltinTypes::void());

        // Register the Content trait — built-in trait for rendering values as ContentNode
        self.register_content_trait();

        // Register the Drop trait — scope-based resource cleanup
        self.register_drop_trait();

        // Register the Into trait shape.
        self.register_into_trait();

        // Register the TryInto trait shape.
        self.register_try_into_trait();

        // Register the Iterable trait — lazy iteration protocol.
        self.register_iterable_trait();

        // Register operator traits — trait-based operator overloading.
        self.register_operator_traits();
    }

    /// Register the Content trait and built-in implementations for primitive types.
    ///
    /// The Content trait has a single required method `render()` that returns a ContentNode.
    /// Built-in types (string, number, int, decimal, bool, array, hashmap, Table, DataTable)
    /// get automatic Content implementations so they can be used in c-strings and content dispatch.
    fn register_content_trait(&mut self) {
        use shape_ast::ast::{FunctionParam, InterfaceMember, TraitMember};

        let content_trait = TraitDef {
            name: "Content".to_string(),
            doc_comment: None,
            type_params: None,
            members: vec![TraitMember::Required(InterfaceMember::Method {
                name: "render".to_string(),
                optional: false,
                params: vec![FunctionParam {
                    name: Some("self".to_string()),
                    type_annotation: TypeAnnotation::Basic("Self".to_string()),
                    optional: false,
                }],
                return_type: TypeAnnotation::Basic("ContentNode".to_string()),
                is_async: false,
                span: Span::DUMMY,
                doc_comment: None,
            })],
            annotations: vec![],
        };
        self.define_trait(&content_trait);

        // Register built-in Content impls for primitive types.
        // These are used by content_dispatch::render_as_content() at runtime.
        let builtin_types = [
            "string",
            "number",
            "int",
            "decimal",
            "bool",
            "array",
            "hashmap",
            "Table",
            "DataTable",
        ];
        for type_name in &builtin_types {
            let _ = self.register_trait_impl("Content", type_name, vec!["render".to_string()]);
        }

        // Register ContentFor<Adapter> trait — adapter-specific rendering.
        // ContentFor takes a type parameter (the adapter type) and provides
        // render(self, caps: RendererCapabilities) -> ContentNode.
        use shape_ast::ast::TypeParam;
        let content_for_trait = TraitDef {
            name: "ContentFor".to_string(),
            doc_comment: None,
            type_params: Some(vec![TypeParam {
                name: "Adapter".to_string(),
                span: Span::DUMMY,
                doc_comment: None,
                default_type: None,
                trait_bounds: vec![],
            }]),
            members: vec![TraitMember::Required(InterfaceMember::Method {
                name: "render".to_string(),
                optional: false,
                params: vec![
                    FunctionParam {
                        name: Some("self".to_string()),
                        type_annotation: TypeAnnotation::Basic("Self".to_string()),
                        optional: false,
                    },
                    FunctionParam {
                        name: Some("caps".to_string()),
                        type_annotation: TypeAnnotation::Basic("RendererCapabilities".to_string()),
                        optional: false,
                    },
                ],
                return_type: TypeAnnotation::Basic("ContentNode".to_string()),
                is_async: false,
                span: Span::DUMMY,
                doc_comment: None,
            })],
            annotations: vec![],
        };
        self.define_trait(&content_for_trait);
    }

    /// Register the Drop trait — automatic scope-based resource cleanup.
    /// Types implementing Drop have their `drop(self)` method called automatically
    /// when a binding goes out of scope.
    fn register_drop_trait(&mut self) {
        use shape_ast::ast::{FunctionParam, InterfaceMember, TraitMember};

        let drop_trait = TraitDef {
            name: "Drop".to_string(),
            doc_comment: None,
            type_params: None,
            members: vec![TraitMember::Required(InterfaceMember::Method {
                name: "drop".to_string(),
                optional: false,
                params: vec![FunctionParam {
                    name: Some("self".to_string()),
                    type_annotation: TypeAnnotation::Basic("Self".to_string()),
                    optional: false,
                }],
                return_type: TypeAnnotation::Basic("void".to_string()),
                is_async: false,
                span: Span::DUMMY,
                doc_comment: None,
            })],
            annotations: vec![],
        };
        self.define_trait(&drop_trait);

        // Register built-in Drop impls for I/O resource types.
        // At runtime, op_drop_call has a fast path that calls handle.close() directly.
        let io_types = ["IoHandle", "io_handle"];
        for type_name in &io_types {
            let _ = self.register_trait_impl("Drop", type_name, vec!["drop".to_string()]);
        }
    }

    /// Register the Into trait shape.
    ///
    /// Concrete conversions are provided by trait implementations (for example
    /// in `std::core::into`) and may be named selectors.
    fn register_into_trait(&mut self) {
        use shape_ast::ast::{InterfaceMember, TraitMember, TypeParam};

        let into_trait = TraitDef {
            name: "Into".to_string(),
            doc_comment: None,
            type_params: Some(vec![TypeParam {
                name: "Target".to_string(),
                span: Span::DUMMY,
                doc_comment: None,
                default_type: None,
                trait_bounds: vec![],
            }]),
            members: vec![TraitMember::Required(InterfaceMember::Method {
                name: "into".to_string(),
                optional: false,
                params: vec![],
                return_type: TypeAnnotation::Reference("Target".to_string()),
                is_async: false,
                span: Span::DUMMY,
                doc_comment: None,
            })],
            annotations: vec![],
        };
        self.define_trait(&into_trait);
    }

    /// Register the TryInto trait shape.
    ///
    /// Concrete conversions are provided by trait implementations (for example
    /// in `std::core::try_into`) and may be named selectors.
    fn register_try_into_trait(&mut self) {
        use shape_ast::ast::{InterfaceMember, TraitMember, TypeParam};

        let try_into_trait = TraitDef {
            name: "TryInto".to_string(),
            doc_comment: None,
            type_params: Some(vec![TypeParam {
                name: "Target".to_string(),
                span: Span::DUMMY,
                doc_comment: None,
                default_type: None,
                trait_bounds: vec![],
            }]),
            members: vec![TraitMember::Required(InterfaceMember::Method {
                name: "tryInto".to_string(),
                optional: false,
                params: vec![],
                return_type: TypeAnnotation::Generic {
                    name: "Result".to_string(),
                    args: vec![
                        TypeAnnotation::Reference("Target".to_string()),
                        TypeAnnotation::Reference("AnyError".to_string()),
                    ],
                },
                is_async: false,
                span: Span::DUMMY,
                doc_comment: None,
            })],
            annotations: vec![],
        };
        self.define_trait(&try_into_trait);
    }

    /// Register the Iterable<T> trait and built-in implementations.
    ///
    /// Types implementing Iterable have an `iter()` method that returns an `Iterator<T>`.
    /// Built-in impls: Array, String, Range, HashMap, DataTable.
    fn register_iterable_trait(&mut self) {
        use shape_ast::ast::{FunctionParam, InterfaceMember, TraitMember, TypeParam};

        let iterable_trait = TraitDef {
            name: "Iterable".to_string(),
            doc_comment: None,
            type_params: Some(vec![TypeParam {
                name: "T".to_string(),
                span: Span::DUMMY,
                doc_comment: None,
                default_type: None,
                trait_bounds: vec![],
            }]),
            members: vec![TraitMember::Required(InterfaceMember::Method {
                name: "iter".to_string(),
                optional: false,
                params: vec![FunctionParam {
                    name: Some("self".to_string()),
                    type_annotation: TypeAnnotation::Basic("Self".to_string()),
                    optional: false,
                }],
                return_type: TypeAnnotation::Generic {
                    name: "Iterator".to_string(),
                    args: vec![TypeAnnotation::Reference("T".to_string())],
                },
                is_async: false,
                span: Span::DUMMY,
                doc_comment: None,
            })],
            annotations: vec![],
        };
        self.define_trait(&iterable_trait);

        // Register built-in Iterable impls for collection types.
        let iterable_types = [
            "Array",
            "array",
            "String",
            "string",
            "Range",
            "range",
            "HashMap",
            "hashmap",
            "DataTable",
            "datatable",
        ];
        for type_name in &iterable_types {
            let _ = self.register_trait_impl("Iterable", type_name, vec!["iter".to_string()]);
        }
    }

    /// Register operator traits for trait-based operator overloading.
    ///
    /// Binary: Add(add), Sub(sub), Mul(mul), Div(div) — `fn method(self, other) -> Self`
    /// Unary:  Neg(neg) — `fn neg(self) -> Self`
    /// Comparison: Eq(eq) — `fn eq(self, other) -> bool`, Ord(cmp) — `fn cmp(self, other) -> int`
    fn register_operator_traits(&mut self) {
        use shape_ast::ast::{FunctionParam, InterfaceMember, TraitMember};

        let self_param = FunctionParam {
            name: Some("self".to_string()),
            type_annotation: TypeAnnotation::Basic("Self".to_string()),
            optional: false,
        };
        let other_param = FunctionParam {
            name: Some("other".to_string()),
            type_annotation: TypeAnnotation::Basic("Self".to_string()),
            optional: false,
        };

        // Binary operator traits: Add, Sub, Mul, Div
        for (trait_name, method_name) in &[
            ("Add", "add"),
            ("Sub", "sub"),
            ("Mul", "mul"),
            ("Div", "div"),
        ] {
            let trait_def = TraitDef {
                name: trait_name.to_string(),
                doc_comment: None,
                type_params: None,
                members: vec![TraitMember::Required(InterfaceMember::Method {
                    name: method_name.to_string(),
                    optional: false,
                    params: vec![self_param.clone(), other_param.clone()],
                    return_type: TypeAnnotation::Basic("Self".to_string()),
                    is_async: false,
                    span: Span::DUMMY,
                    doc_comment: None,
                })],
                annotations: vec![],
            };
            self.define_trait(&trait_def);
        }

        // Unary operator trait: Neg
        let neg_trait = TraitDef {
            name: "Neg".to_string(),
            doc_comment: None,
            type_params: None,
            members: vec![TraitMember::Required(InterfaceMember::Method {
                name: "neg".to_string(),
                optional: false,
                params: vec![self_param.clone()],
                return_type: TypeAnnotation::Basic("Self".to_string()),
                is_async: false,
                span: Span::DUMMY,
                doc_comment: None,
            })],
            annotations: vec![],
        };
        self.define_trait(&neg_trait);

        // Eq trait: eq(self, other) -> bool
        let eq_trait = TraitDef {
            name: "Eq".to_string(),
            doc_comment: None,
            type_params: None,
            members: vec![TraitMember::Required(InterfaceMember::Method {
                name: "eq".to_string(),
                optional: false,
                params: vec![self_param.clone(), other_param.clone()],
                return_type: TypeAnnotation::Basic("bool".to_string()),
                is_async: false,
                span: Span::DUMMY,
                doc_comment: None,
            })],
            annotations: vec![],
        };
        self.define_trait(&eq_trait);

        // Ord trait: cmp(self, other) -> int
        let ord_trait = TraitDef {
            name: "Ord".to_string(),
            doc_comment: None,
            type_params: None,
            members: vec![TraitMember::Required(InterfaceMember::Method {
                name: "cmp".to_string(),
                optional: false,
                params: vec![self_param, other_param],
                return_type: TypeAnnotation::Basic("int".to_string()),
                is_async: false,
                span: Span::DUMMY,
                doc_comment: None,
            })],
            annotations: vec![],
        };
        self.define_trait(&ord_trait);
    }

    /// Define a built-in function with monomorphic type
    fn define_builtin(&mut self, name: &str, params: Vec<Type>, returns: Type) {
        let func_type = BuiltinTypes::function(params, returns);
        self.builtins
            .insert(name.to_string(), TypeScheme::mono(func_type));
    }

    /// Define a built-in function with polymorphic type
    fn define_polymorphic(
        &mut self,
        name: &str,
        type_vars: Vec<TypeVar>,
        params: Vec<Type>,
        returns: Type,
    ) {
        let func_type = BuiltinTypes::function(params, returns);
        self.builtins.insert(
            name.to_string(),
            TypeScheme {
                quantified: type_vars,
                ty: func_type,
                trait_bounds: std::collections::HashMap::new(),
                default_types: std::collections::HashMap::new(),
            },
        );
    }

    fn define_builtin_any(&mut self, name: &str, arity: usize) {
        self.define_builtin(name, vec![BuiltinTypes::any(); arity], BuiltinTypes::any());
    }

    /// Add built-in financial analysis functions
    pub fn define_builtin_functions(&mut self) {
        // Core utility functions available without imports
        self.define_builtin("print", vec![BuiltinTypes::any()], BuiltinTypes::void());
        self.define_builtin("len", vec![BuiltinTypes::any()], BuiltinTypes::integer());
        self.define_builtin(
            "fold",
            vec![
                BuiltinTypes::any(),
                BuiltinTypes::any(),
                BuiltinTypes::any(),
            ],
            BuiltinTypes::any(),
        );

        // HashMap constructor: HashMap() -> HashMap<any, any>
        self.define_builtin(
            "HashMap",
            vec![],
            Type::Concrete(TypeAnnotation::Reference("HashMap".to_string())),
        );

        // Option/Result constructors are polymorphic and must never force `any`.
        let option_t = TypeVar::new("T".to_string());
        let option_inner = Type::Variable(option_t.clone());
        let option_result = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Option".to_string(),
            ))),
            args: vec![option_inner.clone()],
        };
        self.define_polymorphic("Some", vec![option_t], vec![option_inner], option_result);

        let ok_t = TypeVar::new("T".to_string());
        let ok_e = TypeVar::new("E".to_string());
        let ok_inner = Type::Variable(ok_t.clone());
        let ok_result = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Result".to_string(),
            ))),
            args: vec![ok_inner.clone(), Type::Variable(ok_e.clone())],
        };
        let mut ok_defaults = std::collections::HashMap::new();
        ok_defaults.insert(
            ok_e.0.clone(),
            Type::Concrete(TypeAnnotation::Reference("AnyError".to_string())),
        );
        self.builtins.insert(
            "Ok".to_string(),
            TypeScheme::poly_bounded_with_defaults(
                vec![ok_t, ok_e],
                BuiltinTypes::function(vec![ok_inner], ok_result),
                std::collections::HashMap::new(),
                ok_defaults,
            ),
        );

        let err_ok_t = TypeVar::new("T".to_string());
        let err_payload_t = TypeVar::new("E".to_string());
        let err_result = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Result".to_string(),
            ))),
            args: vec![
                Type::Variable(err_ok_t.clone()),
                Type::Variable(err_payload_t.clone()),
            ],
        };
        self.builtins.insert(
            "Err".to_string(),
            TypeScheme::poly_bounded_with_defaults(
                vec![err_ok_t, err_payload_t.clone()],
                BuiltinTypes::function(vec![Type::Variable(err_payload_t)], err_result),
                std::collections::HashMap::new(),
                std::collections::HashMap::new(),
            ),
        );

        // Internal conversion helpers used by std::core::try_into implementations.
        let any_error = Type::Concrete(TypeAnnotation::Reference("AnyError".to_string()));
        let result_of = |ok: Type| Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Result".to_string(),
            ))),
            args: vec![ok, any_error.clone()],
        };
        let define_try_into_input_poly = |this: &mut Self, name: &str, output: Type| {
            let input = TypeVar::new("Input".to_string());
            this.define_polymorphic(
                name,
                vec![input.clone()],
                vec![Type::Variable(input)],
                result_of(output),
            );
        };
        let define_into_input_poly = |this: &mut Self, name: &str, output: Type| {
            let input = TypeVar::new("Input".to_string());
            this.define_polymorphic(
                name,
                vec![input.clone()],
                vec![Type::Variable(input)],
                output,
            );
        };
        define_into_input_poly(self, "__into_int", BuiltinTypes::integer());
        define_into_input_poly(self, "__into_number", BuiltinTypes::number());
        define_into_input_poly(
            self,
            "__into_decimal",
            Type::Concrete(TypeAnnotation::Basic("decimal".to_string())),
        );
        define_into_input_poly(self, "__into_bool", BuiltinTypes::boolean());
        define_into_input_poly(self, "__into_string", BuiltinTypes::string());
        define_try_into_input_poly(self, "__try_into_int", BuiltinTypes::integer());
        define_try_into_input_poly(self, "__try_into_number", BuiltinTypes::number());
        define_try_into_input_poly(
            self,
            "__try_into_decimal",
            Type::Concrete(TypeAnnotation::Basic("decimal".to_string())),
        );
        define_try_into_input_poly(self, "__try_into_bool", BuiltinTypes::boolean());
        define_try_into_input_poly(self, "__try_into_string", BuiltinTypes::string());

        // Position management
        self.define_builtin(
            "open_position",
            vec![
                Type::Concrete(TypeAnnotation::Basic("string".to_string())), // symbol
                BuiltinTypes::number(),                                      // size
                Type::Concrete(TypeAnnotation::Basic("string".to_string())), // side
            ],
            Type::Concrete(TypeAnnotation::Basic("position".to_string())),
        );

        self.define_builtin(
            "close_position",
            vec![Type::Concrete(TypeAnnotation::Basic(
                "position".to_string(),
            ))],
            BuiltinTypes::void(),
        );

        // Risk management
        self.define_builtin(
            "calculate_position_size",
            vec![
                BuiltinTypes::number(), // account_balance
                BuiltinTypes::number(), // risk_percent
                BuiltinTypes::number(), // stop_loss
            ],
            BuiltinTypes::number(),
        );

        // Market data access
        self.define_builtin(
            "get_rows",
            vec![
                Type::Concrete(TypeAnnotation::Basic("string".to_string())), // symbol
                Type::Concrete(TypeAnnotation::Basic("timeframe".to_string())),
                BuiltinTypes::number(), // count
            ],
            BuiltinTypes::array(BuiltinTypes::row()),
        );

        // Intrinsics mirrored from VM helper dispatch (typed loosely in the
        // shared analyzer to avoid undefined-function false positives).
        for name in ["map", "__intrinsic_map", "filter", "__intrinsic_filter"] {
            self.define_builtin_any(name, 2);
        }
        for name in ["reduce", "__intrinsic_reduce"] {
            self.define_builtin_any(name, 3);
        }

        for name in [
            "__intrinsic_sum",
            "__intrinsic_mean",
            "__intrinsic_min",
            "__intrinsic_max",
            "__intrinsic_std",
            "__intrinsic_variance",
            "__intrinsic_median",
            "__intrinsic_random_seed",
            "__intrinsic_random_array",
            "__intrinsic_dist_exponential",
            "__intrinsic_dist_poisson",
            "__intrinsic_shift",
            "__intrinsic_diff",
            "__intrinsic_pct_change",
            "__intrinsic_cumsum",
            "__intrinsic_cumprod",
            "__intrinsic_vec_abs",
            "__intrinsic_vec_sqrt",
            "__intrinsic_vec_ln",
            "__intrinsic_vec_exp",
            "__intrinsic_snapshot",
            "__intrinsic_rolling_sum",
            "__intrinsic_rolling_mean",
            "__intrinsic_rolling_std",
            "__intrinsic_rolling_min",
            "__intrinsic_rolling_max",
            "__intrinsic_ema",
        ] {
            self.define_builtin_any(name, 1);
        }

        for name in [
            "__intrinsic_correlation",
            "__intrinsic_covariance",
            "__intrinsic_percentile",
            "__intrinsic_random_int",
            "__intrinsic_random_normal",
            "__intrinsic_dist_uniform",
            "__intrinsic_dist_lognormal",
            "__intrinsic_random_walk",
            "__intrinsic_fillna",
            "__intrinsic_vec_add",
            "__intrinsic_vec_sub",
            "__intrinsic_vec_mul",
            "__intrinsic_vec_div",
            "__intrinsic_vec_max",
            "__intrinsic_vec_min",
        ] {
            self.define_builtin_any(name, 2);
        }

        for name in [
            "__intrinsic_dist_sample_n",
            "__intrinsic_vec_select",
            "__intrinsic_clip",
            "__intrinsic_linear_recurrence",
            "__intrinsic_brownian_motion",
        ] {
            self.define_builtin_any(name, 3);
        }

        self.define_builtin_any("__intrinsic_gbm", 5);
        self.define_builtin_any("__intrinsic_ou_process", 6);
        self.define_builtin_any("__intrinsic_random", 0);
    }

    /// Define a variable in the current scope
    pub fn define(&mut self, name: &str, scheme: TypeScheme) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), scheme);
        }
    }

    /// Look up a variable type
    pub fn lookup(&self, name: &str) -> Option<&TypeScheme> {
        // Search from innermost to outermost scope
        for scope in self.scopes.iter().rev() {
            if let Some(scheme) = scope.get(name) {
                return Some(scheme);
            }
        }

        // Check built-ins
        self.builtins.get(name)
    }

    /// Push a new scope
    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    /// Pop the current scope
    pub fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    /// Import symbols from a SymbolTable into this TypeEnvironment
    ///
    /// This syncs the semantic analyzer's symbol table with the type inference
    /// engine's environment, allowing the inference engine to see all defined
    /// variables, functions, and type aliases.
    pub fn import_from_symbol_table(
        &mut self,
        symbol_table: &crate::semantic::symbol_table::SymbolTable,
    ) {
        use crate::semantic::symbol_table::Symbol;

        // Import all symbols (variables, functions, and modules)
        for (name, symbol) in symbol_table.iter_all_symbols() {
            match symbol {
                Symbol::Variable { ty, .. } => {
                    let inference_type = ty.to_inference_type();
                    self.define(name, TypeScheme::mono(inference_type));
                }
                Symbol::Function {
                    params, returns, ..
                } => {
                    let param_types: Vec<Type> =
                        params.iter().map(|p| p.to_inference_type()).collect();
                    let return_type = returns.to_inference_type();
                    let func_type = BuiltinTypes::function(param_types, return_type);
                    self.define(name, TypeScheme::mono(func_type));
                }
                Symbol::Module {
                    type_annotation, ..
                } => {
                    // Module is an Object type with function-typed fields.
                    // The inference engine's infer_property_access() will resolve
                    // field access (e.g., duckdb.connect) to the field's Function type.
                    let module_type = Type::Concrete(type_annotation.clone());
                    self.define(name, TypeScheme::mono(module_type));
                }
            }
        }

        // Import type aliases
        for (name, alias_entry) in symbol_table.iter_type_aliases() {
            self.type_registry.define_type_alias(
                name,
                &alias_entry.type_annotation,
                alias_entry.meta_param_overrides.clone(),
            );
        }
    }

    /// Import a single variable from a SymbolTable
    pub fn import_variable(&mut self, name: &str, legacy_type: &crate::semantic::types::Type) {
        let inference_type = legacy_type.to_inference_type();
        self.define(name, TypeScheme::mono(inference_type));
    }

    /// Import a single function from a SymbolTable
    pub fn import_function(
        &mut self,
        name: &str,
        params: &[crate::semantic::types::Type],
        returns: &crate::semantic::types::Type,
    ) {
        let param_types: Vec<Type> = params.iter().map(|p| p.to_inference_type()).collect();
        let return_type = returns.to_inference_type();
        let func_type = BuiltinTypes::function(param_types, return_type);
        self.define(name, TypeScheme::mono(func_type));
    }

    /// Define a type alias with optional meta parameter overrides
    pub fn define_type_alias(
        &mut self,
        name: &str,
        ty: &TypeAnnotation,
        meta_param_overrides: Option<HashMap<String, Expr>>,
    ) {
        self.type_registry
            .define_type_alias(name, ty, meta_param_overrides);
    }

    /// Look up a type alias
    pub fn lookup_type_alias(&self, name: &str) -> Option<&TypeAliasEntry> {
        self.type_registry.lookup_type_alias(name)
    }

    /// Get the meta parameter overrides for a type alias
    pub fn get_type_alias_meta_overrides(&self, name: &str) -> Option<&HashMap<String, Expr>> {
        self.type_registry.get_type_alias_meta_overrides(name)
    }

    /// Define an interface
    pub fn define_interface(&mut self, interface: &InterfaceDef) {
        self.type_registry.define_interface(interface);
    }

    /// Look up an interface
    pub fn lookup_interface(&self, name: &str) -> Option<&InterfaceDef> {
        self.type_registry.lookup_interface(name)
    }

    // =========================================================================
    // Trait Registry Methods
    // =========================================================================

    /// Define a trait
    pub fn define_trait(&mut self, trait_def: &TraitDef) {
        self.type_registry.define_trait(trait_def);
    }

    /// Look up a trait
    pub fn lookup_trait(&self, name: &str) -> Option<&TraitDef> {
        self.type_registry.lookup_trait(name)
    }

    /// Register a trait implementation with validation
    pub fn register_trait_impl(
        &mut self,
        trait_name: &str,
        target_type: &str,
        method_names: Vec<String>,
    ) -> Result<(), String> {
        self.type_registry
            .register_trait_impl(trait_name, target_type, method_names)
    }

    /// Register a named trait implementation
    pub fn register_trait_impl_named(
        &mut self,
        trait_name: &str,
        target_type: &str,
        impl_name: &str,
        method_names: Vec<String>,
    ) -> Result<(), String> {
        self.type_registry.register_trait_impl_named(
            trait_name,
            target_type,
            impl_name,
            method_names,
        )
    }

    /// Register a trait implementation with associated type bindings
    pub fn register_trait_impl_with_assoc_types(
        &mut self,
        trait_name: &str,
        target_type: &str,
        method_names: Vec<String>,
        associated_types: std::collections::HashMap<String, TypeAnnotation>,
    ) -> Result<(), String> {
        self.type_registry.register_trait_impl_with_assoc_types(
            trait_name,
            target_type,
            method_names,
            associated_types,
        )
    }

    /// Register a named trait implementation with associated type bindings
    pub fn register_trait_impl_with_assoc_types_named(
        &mut self,
        trait_name: &str,
        target_type: &str,
        impl_name: Option<&str>,
        method_names: Vec<String>,
        associated_types: std::collections::HashMap<String, TypeAnnotation>,
    ) -> Result<(), String> {
        self.type_registry
            .register_trait_impl_with_assoc_types_named(
                trait_name,
                target_type,
                impl_name,
                method_names,
                associated_types,
            )
    }

    /// Check if a type implements a trait
    pub fn type_implements_trait(&self, type_name: &str, trait_name: &str) -> bool {
        self.type_registry
            .type_implements_trait(type_name, trait_name)
    }

    /// Look up a trait implementation
    pub fn lookup_trait_impl(&self, trait_name: &str, type_name: &str) -> Option<&TraitImplEntry> {
        self.type_registry.lookup_trait_impl(trait_name, type_name)
    }

    /// Look up a named trait implementation
    pub fn lookup_trait_impl_named(
        &self,
        trait_name: &str,
        type_name: &str,
        impl_name: &str,
    ) -> Option<&TraitImplEntry> {
        self.type_registry
            .lookup_trait_impl_named(trait_name, type_name, impl_name)
    }

    /// Resolve an associated type from a trait implementation
    pub fn resolve_associated_type(
        &self,
        trait_name: &str,
        type_name: &str,
        assoc_type_name: &str,
    ) -> Option<&TypeAnnotation> {
        self.type_registry
            .resolve_associated_type(trait_name, type_name, assoc_type_name)
    }

    /// Resolve an associated type from a named trait implementation
    pub fn resolve_associated_type_named(
        &self,
        trait_name: &str,
        type_name: &str,
        impl_name: &str,
        assoc_type_name: &str,
    ) -> Option<&TypeAnnotation> {
        self.type_registry.resolve_associated_type_named(
            trait_name,
            type_name,
            impl_name,
            assoc_type_name,
        )
    }

    /// Get all trait implementation keys ("TraitName::TypeName") as a set
    pub fn trait_impl_keys(&self) -> std::collections::HashSet<String> {
        self.type_registry.trait_impl_keys()
    }

    /// Register a blanket implementation: `impl<T: Bound> Trait for T`
    pub fn register_blanket_impl(
        &mut self,
        trait_name: &str,
        required_bounds: Vec<String>,
        method_names: Vec<String>,
    ) {
        self.type_registry
            .register_blanket_impl(trait_name, required_bounds, method_names)
    }

    // =========================================================================
    // Enum Registry Methods (for exhaustiveness checking)
    // =========================================================================

    /// Register an enum definition for exhaustiveness checking
    pub fn register_enum(&mut self, enum_def: &EnumDef) {
        self.type_registry.register_enum(enum_def);
    }

    /// Look up an enum definition by name
    pub fn get_enum(&self, name: &str) -> Option<&EnumDef> {
        self.type_registry.get_enum(name)
    }

    // =========================================================================
    // Record Schema Methods
    // =========================================================================

    /// Register a record schema
    pub fn register_record_schema(&mut self, name: &str, schema: RecordSchema) {
        self.type_registry.register_record_schema(name, schema);
    }

    /// Look up a record schema
    pub fn lookup_record_schema(&self, name: &str) -> Option<&RecordSchema> {
        self.type_registry.lookup_record_schema(name)
    }

    /// Get field type from a record schema
    pub fn get_record_field_type(
        &self,
        schema_name: &str,
        field_name: &str,
    ) -> Option<&TypeAnnotation> {
        self.type_registry
            .get_record_field_type(schema_name, field_name)
    }

    /// Check if a record schema has a field
    pub fn record_has_field(&self, schema_name: &str, field_name: &str) -> bool {
        self.type_registry.record_has_field(schema_name, field_name)
    }

    // =========================================================================
    /// Generalize a type by quantifying free type variables
    pub fn generalize(&self, ty: &Type) -> TypeScheme {
        let free_vars = self.free_type_vars(ty);
        let env_vars = self.environment_type_vars();

        // Quantify variables that are free in type but not in environment
        let quantified: Vec<_> = free_vars.difference(&env_vars).cloned().collect();

        TypeScheme {
            quantified,
            ty: ty.clone(),
            trait_bounds: std::collections::HashMap::new(),
            default_types: std::collections::HashMap::new(),
        }
    }

    /// Get free type variables in a type
    fn free_type_vars(&self, ty: &Type) -> std::collections::HashSet<TypeVar> {
        use std::collections::HashSet;

        match ty {
            Type::Variable(var) => {
                let mut set = HashSet::new();
                set.insert(var.clone());
                set
            }
            Type::Generic { base, args } => {
                let mut vars = self.free_type_vars(base);
                for arg in args {
                    vars.extend(self.free_type_vars(arg));
                }
                vars
            }
            Type::Constrained { var, .. } => {
                let mut set = HashSet::new();
                set.insert(var.clone());
                set
            }
            Type::Function { params, returns } => {
                let mut vars = HashSet::new();
                for p in params {
                    vars.extend(self.free_type_vars(p));
                }
                vars.extend(self.free_type_vars(returns));
                vars
            }
            Type::Concrete(_) => HashSet::new(),
        }
    }

    /// Get type variables in the environment
    fn environment_type_vars(&self) -> std::collections::HashSet<TypeVar> {
        use std::collections::HashSet;

        let mut vars = HashSet::new();

        for scope in &self.scopes {
            for scheme in scope.values() {
                // Don't include quantified variables
                let ty_vars = self.free_type_vars(&scheme.ty);
                for var in ty_vars {
                    if !scheme.quantified.contains(&var) {
                        vars.insert(var);
                    }
                }
            }
        }

        vars
    }

    // =========================================================================
    // Optimistic Hoisting Methods (Pre-pass field collection)
    // =========================================================================

    /// Register a hoisted field for a variable (called during pre-pass)
    pub fn register_hoisted_field(&mut self, var_name: &str, field_name: &str, field_type: Type) {
        let fields = self.hoisted_fields.entry(var_name.to_string()).or_default();

        if let Some(existing) = fields.iter_mut().find(|f| f.name == field_name) {
            existing.field_type = field_type;
        } else {
            fields.push(HoistedField {
                name: field_name.to_string(),
                field_type,
            });
        }

        self.initialized_hoisted_fields
            .entry(var_name.to_string())
            .or_default();
    }

    /// Get all hoisted fields for a variable
    pub fn get_hoisted_fields(&self, var_name: &str) -> Option<&Vec<HoistedField>> {
        self.hoisted_fields.get(var_name)
    }

    /// Set the current variable being accessed (for property access inference)
    pub fn set_current_access_variable(&mut self, var_name: Option<String>) {
        self.current_access_variable = var_name;
    }

    /// Get the current variable being accessed
    pub fn get_current_access_variable(&self) -> Option<&String> {
        self.current_access_variable.as_ref()
    }

    /// Mark a hoisted field as initialized after a write (`a.y = ...`).
    pub fn mark_hoisted_field_initialized(&mut self, var_name: &str, field_name: &str) {
        self.initialized_hoisted_fields
            .entry(var_name.to_string())
            .or_default()
            .insert(field_name.to_string());
    }

    /// Check whether a hoisted field has been initialized by a write.
    pub fn is_hoisted_field_initialized(&self, var_name: &str, field_name: &str) -> bool {
        self.initialized_hoisted_fields
            .get(var_name)
            .is_some_and(|fields| fields.contains(field_name))
    }

    /// Get a hoisted field type for the current access variable in read context.
    /// Field is only visible after it has been initialized by assignment.
    pub fn get_hoisted_field(&self, field_name: &str) -> Option<Type> {
        let var_name = self.current_access_variable.as_ref()?;
        if !self.is_hoisted_field_initialized(var_name, field_name) {
            return None;
        }
        let fields = self.hoisted_fields.get(var_name)?;
        fields
            .iter()
            .find(|f| f.name == field_name)
            .map(|f| f.field_type.clone())
    }

    /// Get a hoisted field type for the current access variable in assignment context.
    /// Assignment targets may reference hoisted fields before first write.
    pub fn get_hoisted_field_for_assignment(&self, field_name: &str) -> Option<Type> {
        let var_name = self.current_access_variable.as_ref()?;
        let fields = self.hoisted_fields.get(var_name)?;
        fields
            .iter()
            .find(|f| f.name == field_name)
            .map(|f| f.field_type.clone())
    }

    /// Clear all hoisted fields (for resetting between analyses)
    pub fn clear_hoisted_fields(&mut self) {
        self.hoisted_fields.clear();
        self.initialized_hoisted_fields.clear();
    }

    /// Evolve an in-scope object variable by adding/updating a field.
    ///
    /// This keeps runtime-inferred object types in sync with successful property
    /// assignments so later expressions (e.g., `a + b`) observe the evolved shape.
    pub fn upsert_object_field(&mut self, var_name: &str, field_name: &str, field_type: Type) {
        let field_annotation = match field_type.to_annotation() {
            Some(ann) => ann,
            None => return,
        };

        for scope in self.scopes.iter_mut().rev() {
            let Some(scheme) = scope.get_mut(var_name) else {
                continue;
            };

            if let Type::Concrete(TypeAnnotation::Object(fields)) = &mut scheme.ty {
                if let Some(existing) = fields.iter_mut().find(|f| f.name == field_name) {
                    existing.type_annotation = field_annotation;
                    existing.optional = false;
                } else {
                    fields.push(ObjectTypeField {
                        name: field_name.to_string(),
                        optional: false,
                        type_annotation: field_annotation,
                        annotations: vec![],
                    });
                }
            }
            break;
        }
    }

    // =========================================================================
    // Type Evolution Methods (Monotonic Type Growth)
    // =========================================================================

    /// Begin tracking type evolution for a variable
    pub fn begin_evolution(&mut self, var_name: &str, initial_type: SemanticType) {
        self.evolution_registry
            .begin_evolution(var_name, initial_type);
    }

    /// Record a field assignment for type evolution tracking
    pub fn record_field_assignment(
        &mut self,
        var_name: &str,
        field_name: &str,
        field_type: SemanticType,
    ) -> TypeResult<()> {
        self.evolution_registry
            .record_field_assignment(var_name, field_name, field_type);
        Ok(())
    }

    /// Get the current evolved type for a variable
    pub fn get_evolved_type(&self, var_name: &str) -> Option<SemanticType> {
        self.evolution_registry.get_evolved_type(var_name)
    }

    /// Get the type evolution for a variable
    pub fn get_evolution(&self, var_name: &str) -> Option<&TypeEvolution> {
        self.evolution_registry.get_evolution(var_name)
    }

    /// Enter a conditional block (if/else)
    pub fn enter_conditional(&mut self) {
        self.evolution_registry.enter_conditional();
    }

    /// Exit a conditional block
    pub fn exit_conditional(&mut self) {
        self.evolution_registry.exit_conditional();
    }

    /// Enter a loop block (for/while)
    pub fn enter_loop(&mut self) {
        self.evolution_registry.enter_loop();
    }

    /// Exit a loop block
    pub fn exit_loop(&mut self) {
        self.evolution_registry.exit_loop();
    }

    /// Check if we're inside a conditional or loop context
    pub fn in_conditional_context(&self) -> bool {
        self.evolution_registry.in_conditional_context()
    }

    /// Get all type evolutions
    pub fn all_evolutions(&self) -> &HashMap<String, TypeEvolution> {
        self.evolution_registry.all_evolutions()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_evolution_basic() {
        let initial = SemanticType::Struct {
            name: "Object".to_string(),
            fields: vec![("x".to_string(), SemanticType::Number)],
        };
        let mut ev = TypeEvolution::new("a".to_string(), initial);

        // Add a field
        ev.add_field("y".to_string(), SemanticType::Number, false, 0);

        let current = ev.current_type();
        if let SemanticType::Struct { fields, .. } = current {
            assert_eq!(fields.len(), 2);
            assert!(fields.iter().any(|(n, _)| n == "x"));
            assert!(fields.iter().any(|(n, _)| n == "y"));
        } else {
            panic!("Expected struct type");
        }
    }

    #[test]
    fn test_evolution_conditional_makes_optional() {
        let initial = SemanticType::Struct {
            name: "Object".to_string(),
            fields: vec![("x".to_string(), SemanticType::Number)],
        };
        let mut ev = TypeEvolution::new("a".to_string(), initial);

        // Add a field in conditional context (optional)
        ev.add_field("y".to_string(), SemanticType::Number, true, 1);

        let current = ev.current_type();
        if let SemanticType::Struct { fields, .. } = current {
            assert_eq!(fields.len(), 2);
            let y_field = fields.iter().find(|(n, _)| n == "y").unwrap();
            // Should be wrapped in Option
            assert!(matches!(y_field.1, SemanticType::Option(_)));
        } else {
            panic!("Expected struct type");
        }
    }

    #[test]
    fn test_environment_evolution_tracking() {
        let mut env = TypeEnvironment::new();

        let initial = SemanticType::Struct {
            name: "Object".to_string(),
            fields: vec![("x".to_string(), SemanticType::Number)],
        };

        env.begin_evolution("a", initial);

        // Record field assignment outside conditional
        env.record_field_assignment("a", "y", SemanticType::Number)
            .unwrap();

        let evolved = env.get_evolved_type("a").unwrap();
        if let SemanticType::Struct { fields, .. } = evolved {
            assert_eq!(fields.len(), 2);
        } else {
            panic!("Expected struct type");
        }
    }

    #[test]
    fn test_environment_conditional_context() {
        let mut env = TypeEnvironment::new();

        let initial = SemanticType::Struct {
            name: "Object".to_string(),
            fields: vec![("x".to_string(), SemanticType::Number)],
        };

        env.begin_evolution("a", initial);

        // Enter conditional
        env.enter_conditional();
        assert!(env.in_conditional_context());

        // Record field assignment in conditional
        env.record_field_assignment("a", "y", SemanticType::Number)
            .unwrap();

        env.exit_conditional();
        assert!(!env.in_conditional_context());

        // Check that field was marked as optional
        let ev = env.get_evolution("a").unwrap();
        let y_field = ev.evolved_fields.iter().find(|f| f.name == "y").unwrap();
        assert!(y_field.optional);
    }

    #[test]
    fn test_canonical_type_creation() {
        let initial = SemanticType::Struct {
            name: "Object".to_string(),
            fields: vec![("x".to_string(), SemanticType::Number)],
        };
        let mut ev = TypeEvolution::new("a".to_string(), initial);
        ev.add_field("y".to_string(), SemanticType::Number, false, 0);
        ev.add_field("z".to_string(), SemanticType::String, true, 1); // Optional

        let canonical = ev.to_canonical();

        assert_eq!(canonical.name, "a");
        assert_eq!(canonical.fields.len(), 3);

        // Check fields
        let x = canonical.get_field("x").unwrap();
        assert!(!x.optional);
        assert_eq!(x.offset, 0);

        let y = canonical.get_field("y").unwrap();
        assert!(!y.optional);
        assert_eq!(y.offset, 8);

        let z = canonical.get_field("z").unwrap();
        assert!(z.optional);
        assert_eq!(z.offset, 16);

        // Check total size (3 fields * 8 bytes = 24)
        assert_eq!(canonical.data_size, 24);
    }

    #[test]
    fn test_canonical_field_helpers() {
        let canonical = CanonicalType::new(
            "Test".to_string(),
            vec![
                CanonicalField::new("a".to_string(), SemanticType::Number, false),
                CanonicalField::new("b".to_string(), SemanticType::String, true),
            ],
        );

        assert_eq!(canonical.field_offset("a"), Some(0));
        assert_eq!(canonical.field_offset("b"), Some(8));
        assert_eq!(canonical.field_offset("c"), None);

        assert!(!canonical.is_field_optional("a"));
        assert!(canonical.is_field_optional("b"));
    }

    #[test]
    fn test_import_from_symbol_table() {
        use crate::semantic::symbol_table::SymbolTable;
        use crate::semantic::types::Type as LegacyType;
        use shape_ast::ast::VarKind;

        let mut symbol_table = SymbolTable::new();

        // Define a variable
        symbol_table
            .define_variable("x", LegacyType::Number, VarKind::Let, true)
            .unwrap();

        // Define a function
        symbol_table
            .define_function(
                "add",
                vec![LegacyType::Number, LegacyType::Number],
                LegacyType::Number,
            )
            .unwrap();

        // Import into TypeEnvironment
        let mut env = TypeEnvironment::new();
        env.import_from_symbol_table(&symbol_table);

        // Check variable was imported
        let x_scheme = env.lookup("x").expect("x should be defined");
        let x_type = x_scheme.instantiate();
        assert!(matches!(x_type, Type::Concrete(_)));

        // Check function was imported
        let add_scheme = env.lookup("add").expect("add should be defined");
        let add_type = add_scheme.instantiate();
        assert!(matches!(add_type, Type::Function { .. }));
    }

    #[test]
    fn test_import_variable_directly() {
        use crate::semantic::types::Type as LegacyType;

        let mut env = TypeEnvironment::new();
        env.import_variable("count", &LegacyType::Number);

        let scheme = env.lookup("count").expect("count should be defined");
        let ty = scheme.instantiate();

        // Should be a number type
        if let Type::Concrete(ann) = ty {
            assert!(matches!(ann, shape_ast::ast::TypeAnnotation::Basic(ref s) if s == "number"));
        } else {
            panic!("Expected Concrete type");
        }
    }

    #[test]
    fn test_import_function_directly() {
        use crate::semantic::types::Type as LegacyType;

        let mut env = TypeEnvironment::new();
        env.import_function(
            "multiply",
            &[LegacyType::Number, LegacyType::Number],
            &LegacyType::Number,
        );

        let scheme = env.lookup("multiply").expect("multiply should be defined");
        let ty = scheme.instantiate();

        // Should be a function type (now Type::Function, not Concrete)
        if let Type::Function { params, returns } = ty {
            assert_eq!(params.len(), 2);
            assert_eq!(*returns, BuiltinTypes::number());
        } else {
            panic!("Expected Type::Function, got {:?}", ty);
        }
    }

    #[test]
    fn test_environment_starts_without_hardcoded_trait_contracts() {
        let env = TypeEnvironment::new();
        assert!(env.lookup_trait("Serializable").is_none());
        assert!(env.lookup_trait("Distributable").is_none());
        assert!(!env.type_implements_trait("number", "Serializable"));
    }

    #[test]
    fn test_trait_define_and_lookup() {
        use shape_ast::ast::{InterfaceMember, TraitMember};

        let mut env = TypeEnvironment::new();

        let trait_def = TraitDef {
            name: "Queryable".to_string(),
            doc_comment: None,
            type_params: None,
            members: vec![
                TraitMember::Required(InterfaceMember::Method {
                    name: "filter".to_string(),
                    optional: false,
                    params: vec![],
                    return_type: shape_ast::ast::TypeAnnotation::Basic("number".to_string()),
                    is_async: false,
                    span: Span::DUMMY,
                    doc_comment: None,
                }),
                TraitMember::Required(InterfaceMember::Method {
                    name: "execute".to_string(),
                    optional: false,
                    params: vec![],
                    return_type: shape_ast::ast::TypeAnnotation::Basic("number".to_string()),
                    is_async: false,
                    span: Span::DUMMY,
                    doc_comment: None,
                }),
            ],
            annotations: vec![],
        };

        env.define_trait(&trait_def);

        let looked_up = env.lookup_trait("Queryable");
        assert!(looked_up.is_some());
        assert_eq!(looked_up.unwrap().name, "Queryable");
        assert_eq!(looked_up.unwrap().members.len(), 2);

        assert!(env.lookup_trait("NonExistent").is_none());
    }

    #[test]
    fn test_trait_impl_registration() {
        use shape_ast::ast::{InterfaceMember, TraitMember};

        let mut env = TypeEnvironment::new();

        // Define a trait first
        let trait_def = TraitDef {
            name: "Queryable".to_string(),
            doc_comment: None,
            type_params: None,
            members: vec![
                TraitMember::Required(InterfaceMember::Method {
                    name: "filter".to_string(),
                    optional: false,
                    params: vec![],
                    return_type: shape_ast::ast::TypeAnnotation::Basic("number".to_string()),
                    is_async: false,
                    span: Span::DUMMY,
                    doc_comment: None,
                }),
                TraitMember::Required(InterfaceMember::Method {
                    name: "execute".to_string(),
                    optional: false,
                    params: vec![],
                    return_type: shape_ast::ast::TypeAnnotation::Basic("number".to_string()),
                    is_async: false,
                    span: Span::DUMMY,
                    doc_comment: None,
                }),
            ],
            annotations: vec![],
        };
        env.define_trait(&trait_def);

        // Register a valid impl
        let result = env.register_trait_impl(
            "Queryable",
            "Table",
            vec!["filter".to_string(), "execute".to_string()],
        );
        assert!(result.is_ok());
        assert!(env.type_implements_trait("Table", "Queryable"));
        assert!(!env.type_implements_trait("Vec", "Queryable"));
    }

    #[test]
    fn test_trait_impl_missing_method() {
        use shape_ast::ast::{InterfaceMember, TraitMember};

        let mut env = TypeEnvironment::new();

        let trait_def = TraitDef {
            name: "Queryable".to_string(),
            doc_comment: None,
            type_params: None,
            members: vec![
                TraitMember::Required(InterfaceMember::Method {
                    name: "filter".to_string(),
                    optional: false,
                    params: vec![],
                    return_type: shape_ast::ast::TypeAnnotation::Basic("number".to_string()),
                    is_async: false,
                    span: Span::DUMMY,
                    doc_comment: None,
                }),
                TraitMember::Required(InterfaceMember::Method {
                    name: "execute".to_string(),
                    optional: false,
                    params: vec![],
                    return_type: shape_ast::ast::TypeAnnotation::Basic("number".to_string()),
                    is_async: false,
                    span: Span::DUMMY,
                    doc_comment: None,
                }),
            ],
            annotations: vec![],
        };
        env.define_trait(&trait_def);

        // Register an impl missing 'execute'
        let result = env.register_trait_impl("Queryable", "Table", vec!["filter".to_string()]);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("missing required method 'execute'")
        );
    }

    #[test]
    fn test_hoisted_field_read_requires_initialization() {
        let mut env = TypeEnvironment::new();
        env.register_hoisted_field("a", "y", BuiltinTypes::number());
        env.set_current_access_variable(Some("a".to_string()));

        // Assignment target may resolve before first write.
        assert!(env.get_hoisted_field_for_assignment("y").is_some());
        // Read access is blocked until assignment happens.
        assert!(env.get_hoisted_field("y").is_none());

        env.mark_hoisted_field_initialized("a", "y");
        assert!(env.get_hoisted_field("y").is_some());
    }

    #[test]
    fn test_upsert_object_field_updates_variable_shape() {
        let mut env = TypeEnvironment::new();
        env.define(
            "a",
            TypeScheme::mono(Type::Concrete(TypeAnnotation::Object(vec![
                ObjectTypeField {
                    name: "x".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("int".to_string()),
                    annotations: vec![],
                },
            ]))),
        );

        env.upsert_object_field(
            "a",
            "y",
            Type::Concrete(TypeAnnotation::Basic("number".to_string())),
        );

        let ty = env.lookup("a").map(|s| s.ty.clone()).unwrap();
        match ty {
            Type::Concrete(TypeAnnotation::Object(fields)) => {
                assert!(fields.iter().any(|f| f.name == "x"));
                assert!(fields.iter().any(|f| f.name == "y"));
            }
            other => panic!("expected object type, got {:?}", other),
        }
    }

    #[test]
    fn test_drop_trait_registered_as_builtin() {
        let env = TypeEnvironment::new();

        // Drop trait should be registered by init_builtins
        let drop_trait = env.lookup_trait("Drop");
        assert!(drop_trait.is_some(), "Drop trait should be registered");

        let trait_def = drop_trait.unwrap();
        assert_eq!(trait_def.name, "Drop");
        assert_eq!(
            trait_def.members.len(),
            1,
            "Drop trait should have one member: drop(self)"
        );

        // Verify the required method is named "drop"
        match &trait_def.members[0] {
            shape_ast::ast::TraitMember::Required(member) => match member {
                shape_ast::ast::InterfaceMember::Method { name, .. } => {
                    assert_eq!(name, "drop");
                }
                other => panic!("expected Method member, got {:?}", other),
            },
            other => panic!("expected Required member, got {:?}", other),
        }
    }

    #[test]
    fn test_iterable_trait_registered_as_builtin() {
        let env = TypeEnvironment::new();

        let iterable_trait = env.lookup_trait("Iterable");
        assert!(
            iterable_trait.is_some(),
            "Iterable trait should be registered"
        );

        let trait_def = iterable_trait.unwrap();
        assert_eq!(trait_def.name, "Iterable");
        assert!(
            trait_def.type_params.is_some(),
            "Iterable should have type params"
        );
        assert_eq!(trait_def.type_params.as_ref().unwrap().len(), 1);
        assert_eq!(trait_def.type_params.as_ref().unwrap()[0].name, "T");
        assert_eq!(
            trait_def.members.len(),
            1,
            "Iterable trait should have one member: iter(self)"
        );

        match &trait_def.members[0] {
            shape_ast::ast::TraitMember::Required(member) => match member {
                shape_ast::ast::InterfaceMember::Method { name, .. } => {
                    assert_eq!(name, "iter");
                }
                other => panic!("expected Method member, got {:?}", other),
            },
            other => panic!("expected Required member, got {:?}", other),
        }
    }

    #[test]
    fn test_iterable_builtin_impls() {
        let env = TypeEnvironment::new();

        assert!(env.type_implements_trait("Array", "Iterable"));
        assert!(env.type_implements_trait("String", "Iterable"));
        assert!(env.type_implements_trait("Range", "Iterable"));
        assert!(env.type_implements_trait("HashMap", "Iterable"));
        assert!(env.type_implements_trait("DataTable", "Iterable"));

        // Lowercase variants
        assert!(env.type_implements_trait("array", "Iterable"));
        assert!(env.type_implements_trait("string", "Iterable"));
        assert!(env.type_implements_trait("range", "Iterable"));
        assert!(env.type_implements_trait("hashmap", "Iterable"));
        assert!(env.type_implements_trait("datatable", "Iterable"));
    }

    #[test]
    fn test_non_iterable_types() {
        let env = TypeEnvironment::new();

        assert!(!env.type_implements_trait("int", "Iterable"));
        assert!(!env.type_implements_trait("number", "Iterable"));
        assert!(!env.type_implements_trait("bool", "Iterable"));
    }
}

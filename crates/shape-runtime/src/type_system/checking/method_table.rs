//! Method Table for Static Method Resolution
//!
//! Provides compile-time method type checking by maintaining a unified
//! registry of methods available on each type. The table has two tiers:
//!
//! ## Concrete method signatures (`methods`)
//!
//! Simple `(receiver_type, method_name) -> Vec<MethodSignature>` map.
//! Used for monomorphic methods (e.g. `String.len() -> number`) and as
//! a fallback for generic types when no `GenericMethodSignature` exists.
//! Multiple overloads for the same name are stored as separate entries
//! in the `Vec`.
//!
//! ## Generic method signatures (`generic_methods`)
//!
//! `(receiver_type, method_name) -> GenericMethodSignature` map for
//! methods on parameterised types (`Vec<T>`, `HashMap<K,V>`, `Option<T>`,
//! `Result<T,E>`). Signatures use `TypeParamExpr` to express return and
//! parameter types in terms of:
//!
//! - `ReceiverParam(i)` -- the i-th type parameter of the receiver
//!   (e.g. `T` for `Vec<T>`, `K`/`V` for `HashMap<K,V>`)
//! - `MethodParam(i)` -- a type parameter introduced by the method itself
//!   (e.g. `U` in `.map<U>(fn(T) -> U) -> Vec<U>`)
//! - `SelfType` -- the full receiver type (used for `filter`, `sort`, etc.)
//! - `Concrete(Type)` -- a fixed type (`bool`, `void`, `number`, ...)
//! - `Function { params, returns }` -- a callback shape
//! - `GenericContainer { name, args }` -- a parameterised return container
//!
//! At a call site the inference engine calls `extract_receiver_info` to
//! obtain the receiver's type name and actual type arguments, allocates
//! fresh type variables for each `MethodParam`, then resolves the
//! `TypeParamExpr` tree into concrete `Type` values.
//!
//! ## User-defined methods
//!
//! `impl` blocks and `extend` blocks register methods at inference time
//! via `register_user_method`. These are stored in the concrete `methods`
//! map alongside builtins. A universal receiver key (`__Any__`) is used
//! for methods available on every value (e.g. `toString`, `toJSON`).

use crate::type_system::{BuiltinTypes, Type};
use shape_value::ValueWordExt;
use shape_ast::ast::TypeAnnotation;
use std::collections::HashMap;

const UNIVERSAL_RECEIVER: &str = "__Any__";

/// Type expression that can reference generic type parameters from the receiver
/// or from the method itself. Used to express generic method signatures like
/// `Vec<T>.map(fn(T) -> U) -> Vec<U>`.
#[derive(Debug, Clone)]
pub enum TypeParamExpr {
    /// A concrete, fully-resolved type (e.g., number, string, bool, void)
    Concrete(Type),
    /// References a type parameter from the receiver type.
    /// For Vec<T>, index 0 = T. For HashMap<K,V>, index 0 = K, index 1 = V.
    ReceiverParam(usize),
    /// References a type parameter introduced by the method itself.
    /// For .map<U>(fn(T)->U) -> Vec<U>, index 0 = U.
    MethodParam(usize),
    /// A function type with generic parameter/return expressions
    Function {
        params: Vec<TypeParamExpr>,
        returns: Box<TypeParamExpr>,
    },
    /// A generic container with type argument expressions
    /// e.g., Vec<ReceiverParam(0)> or Option<MethodParam(0)>
    GenericContainer {
        name: String,
        args: Vec<TypeParamExpr>,
    },
    /// Returns the same type as the receiver (used for filter, sort, etc.)
    SelfType,
}

/// A method signature with generic type parameter support.
/// Used for builtin methods on generic types (Vec<T>, Table<T>, HashMap<K,V>, etc.)
#[derive(Debug, Clone)]
pub struct GenericMethodSignature {
    pub name: String,
    /// Type parameters introduced by this method (e.g., U in .map<U>)
    pub method_type_params: usize,
    /// Parameter types using TypeParamExpr
    pub param_types: Vec<TypeParamExpr>,
    /// Return type using TypeParamExpr
    pub return_type: TypeParamExpr,
    pub is_fallible: bool,
    /// Trait bounds on receiver type parameters.
    /// Each entry is (receiver_param_index, vec_of_trait_names).
    /// e.g., `Vec<T: Numeric>.sum()` → `[(0, ["Numeric"])]`
    #[allow(dead_code)]
    pub receiver_param_bounds: Vec<(usize, Vec<String>)>,
}

/// A method signature
#[derive(Debug, Clone)]
pub struct MethodSignature {
    /// Name of the method
    pub name: String,
    /// Parameter types (not including receiver)
    pub param_types: Vec<Type>,
    /// Return type
    pub return_type: Type,
    /// Whether the method is fallible (can return Result/error)
    pub is_fallible: bool,
}

/// The receiver type for a method
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReceiverType {
    /// Concrete type like `Vec<T>`, `String`, `Number`
    Concrete(String),
    /// Generic type like `Array` (works with any element type)
    Generic(String),
}

/// Method table for compile-time method resolution
#[derive(Clone)]
pub struct MethodTable {
    /// Methods indexed by (receiver type name, method name)
    methods: HashMap<(String, String), Vec<MethodSignature>>,
    /// Generic method signatures for types with type parameters
    generic_methods: HashMap<(String, String), GenericMethodSignature>,
}

impl MethodTable {
    pub fn new() -> Self {
        let mut table = MethodTable {
            methods: HashMap::new(),
            generic_methods: HashMap::new(),
        };
        table.register_builtin_methods();
        table
    }

    /// Register builtin methods for standard types.
    ///
    /// Only universal methods (__Any__) are registered here. All type-specific
    /// methods are defined in Shape stdlib files (stdlib-src/core/*.shape) and
    /// registered via extend/impl blocks during compilation.
    fn register_builtin_methods(&mut self) {
        // Universal methods available on every value.
        self.register_method(
            UNIVERSAL_RECEIVER,
            "type",
            vec![],
            Type::Concrete(TypeAnnotation::Reference("Type".into())),
            false,
        );
        self.register_method(
            UNIVERSAL_RECEIVER,
            "to_string",
            vec![],
            BuiltinTypes::string(),
            false,
        );
        // Alias for compatibility with existing code paths.
        self.register_method(
            UNIVERSAL_RECEIVER,
            "toString",
            vec![],
            BuiltinTypes::string(),
            false,
        );
    }

    /// Register generic builtin methods for types with type parameters.
    ///
    /// Register a generic method for a type (from extend/impl blocks in Shape stdlib).
    /// Supports receiver parameter trait bounds for compile-time checking.
    pub fn register_user_generic_method(
        &mut self,
        type_name: &str,
        method_name: &str,
        method_type_params: usize,
        param_types: Vec<TypeParamExpr>,
        return_type: TypeParamExpr,
        receiver_param_bounds: Vec<(usize, Vec<String>)>,
    ) {
        let key = (type_name.to_string(), method_name.to_string());
        self.generic_methods.insert(
            key,
            GenericMethodSignature {
                name: method_name.to_string(),
                method_type_params,
                param_types,
                return_type,
                is_fallible: false,
                receiver_param_bounds,
            },
        );
    }

    /// Register a method for a type (used internally for builtins)
    fn register_method(
        &mut self,
        type_name: &str,
        method_name: &str,
        param_types: Vec<Type>,
        return_type: Type,
        is_fallible: bool,
    ) {
        let key = (type_name.to_string(), method_name.to_string());
        let sig = MethodSignature {
            name: method_name.to_string(),
            param_types,
            return_type,
            is_fallible,
        };
        self.methods.entry(key).or_default().push(sig);
    }

    /// Register a user-defined method for a type (from extend/impl blocks)
    pub fn register_user_method(
        &mut self,
        type_name: &str,
        method_name: &str,
        param_types: Vec<Type>,
        return_type: Type,
    ) {
        self.register_method(type_name, method_name, param_types, return_type, false);
    }

    /// Get all methods registered for a type name
    pub fn methods_for_type(&self, type_name: &str) -> Vec<&MethodSignature> {
        self.methods
            .iter()
            .filter(|((receiver, _), _)| receiver == type_name || receiver == UNIVERSAL_RECEIVER)
            .flat_map(|(_, sigs)| sigs.iter())
            .collect()
    }

    /// Look up a method on a type
    pub fn lookup(&self, receiver_type: &Type, method_name: &str) -> Option<&MethodSignature> {
        // Try to extract the type name from the receiver
        let type_name = match receiver_type {
            Type::Concrete(TypeAnnotation::Basic(name)) => name.clone(),
            Type::Concrete(TypeAnnotation::Reference(name)) => name.to_string(),
            Type::Concrete(TypeAnnotation::Array(_)) => "Vec".to_string(),
            Type::Generic { base, .. } => {
                if let Type::Concrete(TypeAnnotation::Reference(name)) = base.as_ref() {
                    name.to_string()
                } else {
                    return None;
                }
            }
            _ => return None,
        };

        let key = (type_name, method_name.to_string());
        if let Some(sig) = self.methods.get(&key).and_then(|sigs| sigs.first()) {
            return Some(sig);
        }

        let universal_key = (UNIVERSAL_RECEIVER.to_string(), method_name.to_string());
        self.methods
            .get(&universal_key)
            .and_then(|sigs| sigs.first())
    }

    /// Resolve a TypeParamExpr into a concrete Type given the receiver type params
    /// and fresh variables for method type params.
    pub fn resolve_type_param_expr(
        expr: &TypeParamExpr,
        receiver_type: &Type,
        receiver_params: &[Type],
        method_vars: &[Type],
    ) -> Type {
        match expr {
            TypeParamExpr::Concrete(t) => t.clone(),
            TypeParamExpr::ReceiverParam(idx) => receiver_params
                .get(*idx)
                .cloned()
                .unwrap_or_else(|| Type::fresh_var()),
            TypeParamExpr::MethodParam(idx) => method_vars
                .get(*idx)
                .cloned()
                .unwrap_or_else(|| Type::fresh_var()),
            TypeParamExpr::SelfType => receiver_type.clone(),
            TypeParamExpr::Function { params, returns } => Type::Function {
                params: params
                    .iter()
                    .map(|p| {
                        Self::resolve_type_param_expr(
                            p,
                            receiver_type,
                            receiver_params,
                            method_vars,
                        )
                    })
                    .collect(),
                returns: Box::new(Self::resolve_type_param_expr(
                    returns,
                    receiver_type,
                    receiver_params,
                    method_vars,
                )),
            },
            TypeParamExpr::GenericContainer { name, args } => {
                let resolved_args: Vec<Type> = args
                    .iter()
                    .map(|a| {
                        Self::resolve_type_param_expr(
                            a,
                            receiver_type,
                            receiver_params,
                            method_vars,
                        )
                    })
                    .collect();
                Type::Generic {
                    base: Box::new(Type::Concrete(TypeAnnotation::Reference(name.as_str().into()))),
                    args: resolved_args,
                }
            }
        }
    }

    /// Extract type name and receiver type parameters from a receiver type.
    pub fn extract_receiver_info(receiver_type: &Type) -> (Option<String>, Vec<Type>) {
        match receiver_type {
            Type::Generic { base, args } => {
                if let Type::Concrete(TypeAnnotation::Reference(name)) = base.as_ref() {
                    let mut params = args.clone();
                    if name == "Result" && params.len() == 1 {
                        params.push(Type::Concrete(TypeAnnotation::Reference(
                            "AnyError".into(),
                        )));
                    }
                    (Some(name.to_string()), params)
                } else {
                    (None, vec![])
                }
            }
            Type::Concrete(TypeAnnotation::Array(elem)) => {
                (Some("Vec".to_string()), vec![Type::Concrete(*elem.clone())])
            }
            Type::Concrete(TypeAnnotation::Basic(name)) => (Some(name.clone()), vec![]),
            Type::Concrete(TypeAnnotation::Reference(name)) => (Some(name.to_string()), vec![]),
            Type::Concrete(TypeAnnotation::Generic { name, args }) => {
                let mut params: Vec<Type> =
                    args.iter().map(|a| Type::Concrete(a.clone())).collect();
                if name == "Result" && params.len() == 1 {
                    params.push(Type::Concrete(TypeAnnotation::Reference(
                        "AnyError".into(),
                    )));
                }
                (Some(name.to_string()), params)
            }
            _ => (None, vec![]),
        }
    }

    /// Get return type for a method call, performing basic type checking.
    /// Tries generic method signatures first, then falls back to monomorphic lookup.
    pub fn resolve_method_call(
        &self,
        receiver_type: &Type,
        method_name: &str,
        _arg_types: &[Type],
    ) -> Option<Type> {
        // Extract type name and receiver params
        let (type_name, receiver_params) = Self::extract_receiver_info(receiver_type);
        let type_name = type_name?;

        // Try generic method first
        let key = (type_name, method_name.to_string());
        if let Some(gsig) = self.generic_methods.get(&key) {
            let method_vars: Vec<Type> = (0..gsig.method_type_params)
                .map(|_| Type::fresh_var())
                .collect();
            return Some(Self::resolve_type_param_expr(
                &gsig.return_type,
                receiver_type,
                &receiver_params,
                &method_vars,
            ));
        }

        // Fall back to non-generic lookup
        let sig = self.lookup(receiver_type, method_name)?;
        Some(sig.return_type.clone())
    }

    /// Look up the generic signature for a method on a type.
    /// Used by the compiler to determine if a method takes closures with receiver params.
    pub fn lookup_generic_signature(
        &self,
        type_name: &str,
        method_name: &str,
    ) -> Option<&GenericMethodSignature> {
        let key = (type_name.to_string(), method_name.to_string());
        self.generic_methods.get(&key)
    }

    /// Check if a method's return type preserves the receiver type (SelfType).
    pub fn is_self_returning(&self, type_name: &str, method_name: &str) -> bool {
        self.lookup_generic_signature(type_name, method_name)
            .map_or(false, |sig| {
                matches!(sig.return_type, TypeParamExpr::SelfType)
            })
    }

    /// Check if a method's first parameter is a function that takes ReceiverParam(0).
    pub fn takes_closure_with_receiver_param(&self, type_name: &str, method_name: &str) -> bool {
        self.lookup_generic_signature(type_name, method_name)
            .map_or(false, |sig| {
                matches!(sig.param_types.first(), Some(TypeParamExpr::Function { params, .. })
                    if params.iter().any(|p| matches!(p, TypeParamExpr::ReceiverParam(0))))
            })
    }
}

impl Default for MethodTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lookup_user_registered_method() {
        let mut table = MethodTable::new();
        // Methods are now registered from Shape stdlib, not at MethodTable::new()
        table.register_user_method("string", "len", vec![], BuiltinTypes::number());

        let string_type = BuiltinTypes::string();
        let sig = table.lookup(&string_type, "len");
        assert!(sig.is_some());

        let sig = table.lookup(&string_type, "nonexistent");
        assert!(sig.is_none());
    }

    #[test]
    fn test_lookup_user_registered_array_method() {
        let mut table = MethodTable::new();
        table.register_user_method("Vec", "len", vec![], BuiltinTypes::number());

        let array_type = BuiltinTypes::array(BuiltinTypes::number());
        let sig = table.lookup(&array_type, "len");
        assert!(sig.is_some());
    }

    #[test]
    fn test_methods_for_type_unknown() {
        let table = MethodTable::new();
        let methods = table.methods_for_type("Nonexistent");
        let names: Vec<&str> = methods.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"type"));
        assert!(names.contains(&"to_string"));
    }

    #[test]
    fn test_lookup_universal_methods() {
        let table = MethodTable::new();
        let user_type = Type::Concrete(TypeAnnotation::Reference("User".into()));
        let sig = table.lookup(&user_type, "type");
        assert!(sig.is_some(), "type() should resolve on any receiver");
        assert!(matches!(
            sig.unwrap().return_type,
            Type::Concrete(TypeAnnotation::Reference(ref n)) if n == "Type"
        ));
        let sig = table.lookup(&user_type, "to_string");
        assert!(sig.is_some(), "to_string() should resolve on any receiver");
    }

    #[test]
    fn test_resolve_array_first_with_user_generic() {
        let mut table = MethodTable::new();
        // Register first() -> T as a generic method (as Shape stdlib would)
        table.register_user_generic_method(
            "Vec", "first", 0, vec![], TypeParamExpr::ReceiverParam(0), vec![],
        );

        let array_type = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Vec".into()))),
            args: vec![BuiltinTypes::number()],
        };

        let result = table.resolve_method_call(&array_type, "first", &[]);
        assert!(result.is_some());
        // Should return the element type (number)
        assert!(
            matches!(result.unwrap(), Type::Concrete(TypeAnnotation::Basic(ref n)) if n == "number")
        );
    }

    #[test]
    fn test_register_user_method() {
        let mut table = MethodTable::new();

        // Register a custom method on a user type
        table.register_user_method(
            "Table",
            "query",
            vec![BuiltinTypes::string()],
            BuiltinTypes::any(),
        );

        let table_type = Type::Concrete(TypeAnnotation::Reference("Table".into()));
        let sig = table.lookup(&table_type, "query");
        assert!(
            sig.is_some(),
            "user method 'query' should be found on Table"
        );
        assert_eq!(sig.unwrap().param_types.len(), 1);
    }

    #[test]
    fn test_user_method_not_found_on_other_type() {
        let mut table = MethodTable::new();
        table.register_user_method("Table", "query", vec![], BuiltinTypes::any());

        let array_type = BuiltinTypes::array(BuiltinTypes::number());
        let sig = table.lookup(&array_type, "query");
        assert!(
            sig.is_none(),
            "user method 'query' should not exist on Array"
        );
    }

    #[test]
    fn test_extend_methods_visible() {
        let mut table = MethodTable::new();

        // Simulate extend Table<Row> { fn smooth(self, window: number) -> Table<Row> { ... } }
        table.register_user_method(
            "Table",
            "smooth",
            vec![BuiltinTypes::number()],
            Type::Concrete(TypeAnnotation::Reference("Table".into())),
        );

        let table_type = Type::Concrete(TypeAnnotation::Reference("Table".into()));
        assert!(table.lookup(&table_type, "smooth").is_some());

        let methods = table.methods_for_type("Table");
        let names: Vec<&str> = methods.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"smooth"));
    }

    #[test]
    fn test_resolve_generic_filter_with_user_registration() {
        let mut table = MethodTable::new();
        table.register_user_generic_method(
            "Vec", "filter", 0,
            vec![TypeParamExpr::Function {
                params: vec![TypeParamExpr::ReceiverParam(0)],
                returns: Box::new(TypeParamExpr::Concrete(BuiltinTypes::boolean())),
            }],
            TypeParamExpr::SelfType, vec![],
        );

        let array_type = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Vec".into()))),
            args: vec![BuiltinTypes::number()],
        };
        let result = table.resolve_method_call(&array_type, "filter", &[]);
        assert!(result.is_some());
        let rt = result.unwrap();
        assert!(matches!(rt, Type::Generic { .. }), "filter should return Vec<number>, got {:?}", rt);
    }

    #[test]
    fn test_resolve_generic_map_with_user_registration() {
        let mut table = MethodTable::new();
        table.register_user_generic_method(
            "Vec", "map", 1,
            vec![TypeParamExpr::Function {
                params: vec![TypeParamExpr::ReceiverParam(0)],
                returns: Box::new(TypeParamExpr::MethodParam(0)),
            }],
            TypeParamExpr::GenericContainer {
                name: "Vec".to_string(),
                args: vec![TypeParamExpr::MethodParam(0)],
            },
            vec![],
        );

        let array_type = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Vec".into()))),
            args: vec![BuiltinTypes::string()],
        };
        let result = table.resolve_method_call(&array_type, "map", &[]);
        assert!(result.is_some());
        let rt = result.unwrap();
        assert!(matches!(rt, Type::Generic { .. }), "map should return Vec<U>, got {:?}", rt);
    }

    #[test]
    fn test_resolve_generic_option_unwrap_with_user_registration() {
        let mut table = MethodTable::new();
        table.register_user_generic_method(
            "Option", "unwrap", 0, vec![],
            TypeParamExpr::ReceiverParam(0), vec![],
        );

        let option_type = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Option".into()))),
            args: vec![BuiltinTypes::number()],
        };
        let result = table.resolve_method_call(&option_type, "unwrap", &[]);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), Type::Concrete(TypeAnnotation::Basic(ref n)) if n == "number"));
    }

    #[test]
    fn test_resolve_generic_hashmap_get_with_user_registration() {
        let mut table = MethodTable::new();
        table.register_user_generic_method(
            "HashMap", "get", 0,
            vec![TypeParamExpr::ReceiverParam(0)],
            TypeParamExpr::GenericContainer {
                name: "Option".to_string(),
                args: vec![TypeParamExpr::ReceiverParam(1)],
            },
            vec![],
        );

        let map_type = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("HashMap".into()))),
            args: vec![BuiltinTypes::string(), BuiltinTypes::number()],
        };
        let result = table.resolve_method_call(&map_type, "get", &[]);
        assert!(result.is_some());
        let rt = result.unwrap();
        assert!(
            matches!(&rt, Type::Generic { base, args }
                if matches!(base.as_ref(), Type::Concrete(TypeAnnotation::Reference(n)) if n == "Option")
                && args.len() == 1),
            "get should return Option<number>, got {:?}", rt
        );
    }

    #[test]
    fn test_is_self_returning_with_user_registration() {
        let mut table = MethodTable::new();
        table.register_user_generic_method(
            "Vec", "filter", 0, vec![], TypeParamExpr::SelfType, vec![],
        );
        table.register_user_generic_method(
            "Vec", "map", 1, vec![],
            TypeParamExpr::GenericContainer { name: "Vec".to_string(), args: vec![TypeParamExpr::MethodParam(0)] },
            vec![],
        );

        assert!(table.is_self_returning("Vec", "filter"));
        assert!(!table.is_self_returning("Vec", "map"));
    }

    #[test]
    fn test_takes_closure_with_receiver_param_with_user_registration() {
        let mut table = MethodTable::new();
        table.register_user_generic_method(
            "Vec", "filter", 0,
            vec![TypeParamExpr::Function {
                params: vec![TypeParamExpr::ReceiverParam(0)],
                returns: Box::new(TypeParamExpr::Concrete(BuiltinTypes::boolean())),
            }],
            TypeParamExpr::SelfType, vec![],
        );

        assert!(table.takes_closure_with_receiver_param("Vec", "filter"));
        assert!(!table.takes_closure_with_receiver_param("Vec", "len"));
    }
}

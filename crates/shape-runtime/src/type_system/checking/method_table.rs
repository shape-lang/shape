//! Method Table for Static Method Resolution
//!
//! Provides compile-time method type checking by maintaining a registry
//! of methods available on each type.

use crate::type_system::{BuiltinTypes, Type, TypeVar};
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
        table.register_generic_builtin_methods();
        table
    }

    /// Register builtin methods for standard types
    fn register_builtin_methods(&mut self) {
        // Universal methods available on every value.
        self.register_method(
            UNIVERSAL_RECEIVER,
            "type",
            vec![],
            Type::Concrete(TypeAnnotation::Reference("Type".to_string())),
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

        // Array methods
        self.register_method("Vec", "len", vec![], BuiltinTypes::number(), false);
        self.register_method("Vec", "isEmpty", vec![], BuiltinTypes::boolean(), false);
        self.register_method(
            "Vec",
            "first",
            vec![],
            Type::Variable(TypeVar::fresh()),
            false,
        );
        self.register_method(
            "Vec",
            "last",
            vec![],
            Type::Variable(TypeVar::fresh()),
            false,
        );
        self.register_method(
            "Vec",
            "push",
            vec![Type::Variable(TypeVar::fresh())],
            BuiltinTypes::void(),
            false,
        );
        self.register_method(
            "Vec",
            "pop",
            vec![],
            Type::Variable(TypeVar::fresh()),
            false,
        );
        self.register_method(
            "Vec",
            "reverse",
            vec![],
            Type::Variable(TypeVar::fresh()),
            false,
        );

        // Array higher-order methods (with callback) — kept for fallback; generic_methods takes priority
        self.register_method(
            "Vec",
            "map",
            vec![BuiltinTypes::any()],
            BuiltinTypes::any(),
            false,
        );
        self.register_method(
            "Vec",
            "filter",
            vec![BuiltinTypes::any()],
            BuiltinTypes::any(),
            false,
        );
        self.register_method(
            "Vec",
            "reduce",
            vec![BuiltinTypes::any(), BuiltinTypes::any()],
            BuiltinTypes::any(),
            false,
        );
        self.register_method(
            "Vec",
            "find",
            vec![BuiltinTypes::any()],
            BuiltinTypes::any(),
            false,
        );
        self.register_method(
            "Vec",
            "forEach",
            vec![BuiltinTypes::any()],
            BuiltinTypes::void(),
            false,
        );
        self.register_method(
            "Vec",
            "some",
            vec![BuiltinTypes::any()],
            BuiltinTypes::boolean(),
            false,
        );
        self.register_method(
            "Vec",
            "every",
            vec![BuiltinTypes::any()],
            BuiltinTypes::boolean(),
            false,
        );
        self.register_method(
            "Vec",
            "join",
            vec![BuiltinTypes::string()],
            BuiltinTypes::string(),
            false,
        );
        self.register_method(
            "Vec",
            "slice",
            vec![BuiltinTypes::number(), BuiltinTypes::number()],
            Type::Variable(TypeVar::fresh()),
            false,
        );
        self.register_method(
            "Vec",
            "take",
            vec![BuiltinTypes::number()],
            Type::Variable(TypeVar::fresh()),
            false,
        );
        self.register_method(
            "Vec",
            "drop",
            vec![BuiltinTypes::number()],
            Type::Variable(TypeVar::fresh()),
            false,
        );
        self.register_method(
            "Vec",
            "flatten",
            vec![],
            Type::Variable(TypeVar::fresh()),
            false,
        );
        self.register_method(
            "Vec",
            "unique",
            vec![],
            Type::Variable(TypeVar::fresh()),
            false,
        );
        self.register_method(
            "Vec",
            "concat",
            vec![Type::Variable(TypeVar::fresh())],
            Type::Variable(TypeVar::fresh()),
            false,
        );
        self.register_method(
            "Vec",
            "indexOf",
            vec![Type::Variable(TypeVar::fresh())],
            BuiltinTypes::number(),
            false,
        );
        self.register_method(
            "Vec",
            "sort",
            vec![BuiltinTypes::any()],
            Type::Variable(TypeVar::fresh()),
            false,
        );

        // Table methods used by query/dataflow chains.
        // These are typed loosely here; execution-level validation remains in VM/runtime.
        self.register_method(
            "Table",
            "filter",
            vec![BuiltinTypes::any()],
            BuiltinTypes::any(),
            false,
        );
        self.register_method(
            "Table",
            "map",
            vec![BuiltinTypes::any()],
            BuiltinTypes::any(),
            false,
        );
        self.register_method(
            "Table",
            "reduce",
            vec![BuiltinTypes::any(), BuiltinTypes::any()],
            BuiltinTypes::any(),
            false,
        );
        self.register_method(
            "Table",
            "groupBy",
            vec![BuiltinTypes::any()],
            BuiltinTypes::any(),
            false,
        );
        self.register_method(
            "Table",
            "indexBy",
            vec![BuiltinTypes::any()],
            BuiltinTypes::any(),
            false,
        );
        self.register_method(
            "Table",
            "select",
            vec![BuiltinTypes::any()],
            BuiltinTypes::any(),
            false,
        );
        self.register_method(
            "Table",
            "orderBy",
            vec![BuiltinTypes::any()],
            BuiltinTypes::any(),
            false,
        );
        self.register_method(
            "Table",
            "simulate",
            vec![BuiltinTypes::any()],
            BuiltinTypes::any(),
            false,
        );
        self.register_method(
            "Table",
            "aggregate",
            vec![BuiltinTypes::any()],
            BuiltinTypes::any(),
            false,
        );
        self.register_method(
            "Table",
            "forEach",
            vec![BuiltinTypes::any()],
            BuiltinTypes::void(),
            false,
        );
        self.register_method("Table", "describe", vec![], BuiltinTypes::any(), false);
        self.register_method("Table", "count", vec![], BuiltinTypes::number(), false);

        // String methods
        self.register_method("string", "len", vec![], BuiltinTypes::number(), false);
        self.register_method("string", "isEmpty", vec![], BuiltinTypes::boolean(), false);
        self.register_method(
            "string",
            "toLowerCase",
            vec![],
            BuiltinTypes::string(),
            false,
        );
        self.register_method(
            "string",
            "toUpperCase",
            vec![],
            BuiltinTypes::string(),
            false,
        );
        self.register_method("string", "trim", vec![], BuiltinTypes::string(), false);
        self.register_method(
            "string",
            "split",
            vec![BuiltinTypes::string()],
            BuiltinTypes::array(BuiltinTypes::string()),
            false,
        );
        self.register_method(
            "string",
            "contains",
            vec![BuiltinTypes::string()],
            BuiltinTypes::boolean(),
            false,
        );
        self.register_method(
            "string",
            "startsWith",
            vec![BuiltinTypes::string()],
            BuiltinTypes::boolean(),
            false,
        );
        self.register_method(
            "string",
            "endsWith",
            vec![BuiltinTypes::string()],
            BuiltinTypes::boolean(),
            false,
        );
        self.register_method(
            "string",
            "replace",
            vec![BuiltinTypes::string(), BuiltinTypes::string()],
            BuiltinTypes::string(),
            false,
        );
        self.register_method("string", "trimStart", vec![], BuiltinTypes::string(), false);
        self.register_method("string", "trimEnd", vec![], BuiltinTypes::string(), false);
        self.register_method("string", "toNumber", vec![], BuiltinTypes::number(), true);
        self.register_method("string", "toBool", vec![], BuiltinTypes::boolean(), true);
        self.register_method(
            "string",
            "chars",
            vec![],
            BuiltinTypes::array(BuiltinTypes::string()),
            false,
        );
        self.register_method(
            "string",
            "padStart",
            vec![BuiltinTypes::number()],
            BuiltinTypes::string(),
            false,
        );
        self.register_method(
            "string",
            "padEnd",
            vec![BuiltinTypes::number()],
            BuiltinTypes::string(),
            false,
        );
        self.register_method(
            "string",
            "repeat",
            vec![BuiltinTypes::number()],
            BuiltinTypes::string(),
            false,
        );
        self.register_method(
            "string",
            "charAt",
            vec![BuiltinTypes::number()],
            BuiltinTypes::string(),
            false,
        );
        self.register_method("string", "reverse", vec![], BuiltinTypes::string(), false);
        self.register_method(
            "string",
            "indexOf",
            vec![BuiltinTypes::string()],
            BuiltinTypes::number(),
            false,
        );
        self.register_method("string", "isDigit", vec![], BuiltinTypes::boolean(), false);
        self.register_method("string", "isAlpha", vec![], BuiltinTypes::boolean(), false);
        self.register_method(
            "string",
            "codePointAt",
            vec![BuiltinTypes::number()],
            BuiltinTypes::number(),
            false,
        );
        self.register_method(
            "string",
            "substring",
            vec![BuiltinTypes::number()],
            BuiltinTypes::string(),
            false,
        );
        self.register_method(
            "string",
            "normalize",
            vec![BuiltinTypes::string()],
            BuiltinTypes::string(),
            false,
        );
        self.register_method(
            "string",
            "graphemes",
            vec![],
            BuiltinTypes::array(BuiltinTypes::string()),
            false,
        );
        self.register_method(
            "string",
            "graphemeLen",
            vec![],
            BuiltinTypes::integer(),
            false,
        );
        self.register_method("string", "isAscii", vec![], BuiltinTypes::boolean(), false);

        // Number methods
        self.register_method("number", "abs", vec![], BuiltinTypes::number(), false);
        self.register_method("number", "floor", vec![], BuiltinTypes::number(), false);
        self.register_method("number", "ceil", vec![], BuiltinTypes::number(), false);
        self.register_method("number", "round", vec![], BuiltinTypes::number(), false);
        self.register_method("number", "toString", vec![], BuiltinTypes::string(), false);
        self.register_method(
            "number",
            "toFixed",
            vec![BuiltinTypes::number()],
            BuiltinTypes::string(),
            false,
        );

        self.register_method("number", "sign", vec![], BuiltinTypes::number(), false);
        self.register_method(
            "number",
            "clamp",
            vec![BuiltinTypes::number(), BuiltinTypes::number()],
            BuiltinTypes::number(),
            false,
        );

        // Integer methods
        self.register_method(
            "int",
            "abs",
            vec![],
            Type::Concrete(TypeAnnotation::Basic("int".to_string())),
            false,
        );
        self.register_method("int", "toString", vec![], BuiltinTypes::string(), false);
        self.register_method(
            "int",
            "sign",
            vec![],
            Type::Concrete(TypeAnnotation::Basic("int".to_string())),
            false,
        );
        self.register_method(
            "int",
            "clamp",
            vec![
                Type::Concrete(TypeAnnotation::Basic("int".to_string())),
                Type::Concrete(TypeAnnotation::Basic("int".to_string())),
            ],
            Type::Concrete(TypeAnnotation::Basic("int".to_string())),
            false,
        );

        // Option methods
        self.register_method(
            "Option",
            "unwrap",
            vec![],
            Type::Variable(TypeVar::fresh()),
            true,
        );
        self.register_method(
            "Option",
            "unwrapOr",
            vec![Type::Variable(TypeVar::fresh())],
            Type::Variable(TypeVar::fresh()),
            false,
        );
        self.register_method("Option", "isSome", vec![], BuiltinTypes::boolean(), false);
        self.register_method("Option", "isNone", vec![], BuiltinTypes::boolean(), false);
        self.register_method(
            "Option",
            "map",
            vec![BuiltinTypes::any()],
            Type::Variable(TypeVar::fresh()),
            false,
        );

        // Result methods
        self.register_method(
            "Result",
            "unwrap",
            vec![],
            Type::Variable(TypeVar::fresh()),
            true,
        );
        self.register_method(
            "Result",
            "unwrapOr",
            vec![Type::Variable(TypeVar::fresh())],
            Type::Variable(TypeVar::fresh()),
            false,
        );
        self.register_method("Result", "isOk", vec![], BuiltinTypes::boolean(), false);
        self.register_method("Result", "isErr", vec![], BuiltinTypes::boolean(), false);
        self.register_method(
            "Result",
            "map",
            vec![BuiltinTypes::any()],
            Type::Variable(TypeVar::fresh()),
            false,
        );
        self.register_method(
            "Result",
            "mapErr",
            vec![BuiltinTypes::any()],
            Type::Variable(TypeVar::fresh()),
            false,
        );

        // Column methods (for vectorized column operations)
        self.register_method("Column", "len", vec![], BuiltinTypes::number(), false);
        self.register_method(
            "Column",
            "first",
            vec![],
            Type::Variable(TypeVar::fresh()),
            false,
        );
        self.register_method(
            "Column",
            "last",
            vec![],
            Type::Variable(TypeVar::fresh()),
            false,
        );
        self.register_method("Column", "sum", vec![], BuiltinTypes::number(), false);
        self.register_method("Column", "mean", vec![], BuiltinTypes::number(), false);
        self.register_method("Column", "min", vec![], BuiltinTypes::number(), false);
        self.register_method("Column", "max", vec![], BuiltinTypes::number(), false);
        self.register_method("Column", "std", vec![], BuiltinTypes::number(), false);
        self.register_method(
            "Column",
            "abs",
            vec![],
            BuiltinTypes::array(BuiltinTypes::number()),
            false,
        );
        self.register_method(
            "Column",
            "toArray",
            vec![],
            BuiltinTypes::array(BuiltinTypes::any()),
            false,
        );
    }

    /// Register generic builtin methods for types with type parameters
    fn register_generic_builtin_methods(&mut self) {
        use TypeParamExpr::*;

        // Vec<T> methods
        // filter(fn(T) -> bool) -> Vec<T>
        self.register_generic_method(
            "Vec",
            "filter",
            0,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(Concrete(BuiltinTypes::boolean())),
            }],
            SelfType,
            false,
        );
        // map<U>(fn(T) -> U) -> Vec<U>
        self.register_generic_method(
            "Vec",
            "map",
            1,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(MethodParam(0)),
            }],
            GenericContainer {
                name: "Vec".to_string(),
                args: vec![MethodParam(0)],
            },
            false,
        );
        // reduce<U>(fn(U, T) -> U, U) -> U
        self.register_generic_method(
            "Vec",
            "reduce",
            1,
            vec![
                Function {
                    params: vec![MethodParam(0), ReceiverParam(0)],
                    returns: Box::new(MethodParam(0)),
                },
                MethodParam(0),
            ],
            MethodParam(0),
            false,
        );
        // find(fn(T) -> bool) -> T
        self.register_generic_method(
            "Vec",
            "find",
            0,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(Concrete(BuiltinTypes::boolean())),
            }],
            ReceiverParam(0),
            false,
        );
        // forEach(fn(T) -> void) -> void
        self.register_generic_method(
            "Vec",
            "forEach",
            0,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(Concrete(BuiltinTypes::void())),
            }],
            Concrete(BuiltinTypes::void()),
            false,
        );
        // some(fn(T) -> bool) -> bool
        self.register_generic_method(
            "Vec",
            "some",
            0,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(Concrete(BuiltinTypes::boolean())),
            }],
            Concrete(BuiltinTypes::boolean()),
            false,
        );
        // every(fn(T) -> bool) -> bool
        self.register_generic_method(
            "Vec",
            "every",
            0,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(Concrete(BuiltinTypes::boolean())),
            }],
            Concrete(BuiltinTypes::boolean()),
            false,
        );
        // sort(fn(T,T) -> number) -> Vec<T>
        self.register_generic_method(
            "Vec",
            "sort",
            0,
            vec![Function {
                params: vec![ReceiverParam(0), ReceiverParam(0)],
                returns: Box::new(Concrete(BuiltinTypes::number())),
            }],
            SelfType,
            false,
        );
        // flatMap<U>(fn(T) -> Vec<U>) -> Vec<U>
        self.register_generic_method(
            "Vec",
            "flatMap",
            1,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(GenericContainer {
                    name: "Vec".to_string(),
                    args: vec![MethodParam(0)],
                }),
            }],
            GenericContainer {
                name: "Vec".to_string(),
                args: vec![MethodParam(0)],
            },
            false,
        );
        // groupBy<K>(fn(T) -> K) -> Vec<{key: K, group: Vec<T>}>
        self.register_generic_method(
            "Vec",
            "groupBy",
            1,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(MethodParam(0)),
            }],
            Concrete(BuiltinTypes::any()),
            false,
        ); // groupBy result shape is complex, keep any
        // findIndex(fn(T) -> bool) -> number
        self.register_generic_method(
            "Vec",
            "findIndex",
            0,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(Concrete(BuiltinTypes::boolean())),
            }],
            Concrete(BuiltinTypes::number()),
            false,
        );
        // sortBy(fn(T) -> any) -> Vec<T>
        self.register_generic_method(
            "Vec",
            "sortBy",
            0,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(Concrete(BuiltinTypes::any())),
            }],
            SelfType,
            false,
        );
        // includes(T) -> bool
        self.register_generic_method(
            "Vec",
            "includes",
            0,
            vec![ReceiverParam(0)],
            Concrete(BuiltinTypes::boolean()),
            false,
        );
        // first() -> T
        self.register_generic_method("Vec", "first", 0, vec![], ReceiverParam(0), false);
        // last() -> T
        self.register_generic_method("Vec", "last", 0, vec![], ReceiverParam(0), false);

        // Table<T> methods
        // filter(fn(T) -> bool) -> Table<T>
        self.register_generic_method(
            "Table",
            "filter",
            0,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(Concrete(BuiltinTypes::boolean())),
            }],
            SelfType,
            false,
        );
        // map<U>(fn(T) -> U) -> Table<U>
        self.register_generic_method(
            "Table",
            "map",
            1,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(MethodParam(0)),
            }],
            GenericContainer {
                name: "Table".to_string(),
                args: vec![MethodParam(0)],
            },
            false,
        );
        // reduce<U>(fn(U, T) -> U, U) -> U
        self.register_generic_method(
            "Table",
            "reduce",
            1,
            vec![
                Function {
                    params: vec![MethodParam(0), ReceiverParam(0)],
                    returns: Box::new(MethodParam(0)),
                },
                MethodParam(0),
            ],
            MethodParam(0),
            false,
        );
        // groupBy(fn(T) -> any) -> Vec<{key: any, group: Table<T>}>
        self.register_generic_method(
            "Table",
            "groupBy",
            0,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(Concrete(BuiltinTypes::any())),
            }],
            Concrete(BuiltinTypes::any()),
            false,
        );
        // indexBy(fn(T) -> any) -> Table<T> (indexed)
        self.register_generic_method(
            "Table",
            "indexBy",
            0,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(Concrete(BuiltinTypes::any())),
            }],
            SelfType,
            false,
        );
        // select<U>(fn(T) -> U) -> Table<U>
        self.register_generic_method(
            "Table",
            "select",
            1,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(MethodParam(0)),
            }],
            GenericContainer {
                name: "Table".to_string(),
                args: vec![MethodParam(0)],
            },
            false,
        );
        // orderBy(fn(T) -> any, string) -> Table<T>
        self.register_generic_method(
            "Table",
            "orderBy",
            0,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(Concrete(BuiltinTypes::any())),
            }],
            SelfType,
            false,
        );
        // simulate(fn(T) -> any) -> any
        self.register_generic_method(
            "Table",
            "simulate",
            0,
            vec![Concrete(BuiltinTypes::any())],
            Concrete(BuiltinTypes::any()),
            false,
        );
        // aggregate(any) -> any (dynamic shape)
        self.register_generic_method(
            "Table",
            "aggregate",
            0,
            vec![Concrete(BuiltinTypes::any())],
            Concrete(BuiltinTypes::any()),
            false,
        );
        // forEach(fn(T) -> void) -> void
        self.register_generic_method(
            "Table",
            "forEach",
            0,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(Concrete(BuiltinTypes::void())),
            }],
            Concrete(BuiltinTypes::void()),
            false,
        );
        // head(number) -> Table<T>
        self.register_generic_method(
            "Table",
            "head",
            0,
            vec![Concrete(BuiltinTypes::number())],
            SelfType,
            false,
        );
        // tail(number) -> Table<T>
        self.register_generic_method(
            "Table",
            "tail",
            0,
            vec![Concrete(BuiltinTypes::number())],
            SelfType,
            false,
        );
        // limit(number) -> Table<T>
        self.register_generic_method(
            "Table",
            "limit",
            0,
            vec![Concrete(BuiltinTypes::number())],
            SelfType,
            false,
        );
        // toMat() -> Mat<number>
        self.register_generic_method(
            "Table",
            "toMat",
            0,
            vec![],
            GenericContainer {
                name: "Mat".to_string(),
                args: vec![Concrete(BuiltinTypes::number())],
            },
            false,
        );

        // Option<T> methods
        // unwrap() -> T
        self.register_generic_method("Option", "unwrap", 0, vec![], ReceiverParam(0), true);
        // unwrapOr(T) -> T
        self.register_generic_method(
            "Option",
            "unwrapOr",
            0,
            vec![ReceiverParam(0)],
            ReceiverParam(0),
            false,
        );
        // map<U>(fn(T) -> U) -> Option<U>
        self.register_generic_method(
            "Option",
            "map",
            1,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(MethodParam(0)),
            }],
            GenericContainer {
                name: "Option".to_string(),
                args: vec![MethodParam(0)],
            },
            false,
        );

        // Result<T, E> methods (Result<T> defaults E to AnyError)
        // unwrap() -> T
        self.register_generic_method("Result", "unwrap", 0, vec![], ReceiverParam(0), true);
        // unwrapOr(T) -> T
        self.register_generic_method(
            "Result",
            "unwrapOr",
            0,
            vec![ReceiverParam(0)],
            ReceiverParam(0),
            false,
        );
        // map<U>(fn(T) -> U) -> Result<U, E>
        self.register_generic_method(
            "Result",
            "map",
            1,
            vec![Function {
                params: vec![ReceiverParam(0)],
                returns: Box::new(MethodParam(0)),
            }],
            GenericContainer {
                name: "Result".to_string(),
                args: vec![MethodParam(0), ReceiverParam(1)],
            },
            false,
        );
        // mapErr<U>(fn(E) -> U) -> Result<T, U>
        self.register_generic_method(
            "Result",
            "mapErr",
            1,
            vec![Function {
                params: vec![ReceiverParam(1)],
                returns: Box::new(MethodParam(0)),
            }],
            GenericContainer {
                name: "Result".to_string(),
                args: vec![ReceiverParam(0), MethodParam(0)],
            },
            false,
        );

        // HashMap<K,V> methods
        // get(K) -> Option<V>
        self.register_generic_method(
            "HashMap",
            "get",
            0,
            vec![ReceiverParam(0)],
            GenericContainer {
                name: "Option".to_string(),
                args: vec![ReceiverParam(1)],
            },
            false,
        );
        // set(K, V) -> HashMap<K,V>
        self.register_generic_method(
            "HashMap",
            "set",
            0,
            vec![ReceiverParam(0), ReceiverParam(1)],
            SelfType,
            false,
        );
        // has(K) -> bool
        self.register_generic_method(
            "HashMap",
            "has",
            0,
            vec![ReceiverParam(0)],
            Concrete(BuiltinTypes::boolean()),
            false,
        );
        // delete(K) -> HashMap<K,V>
        self.register_generic_method(
            "HashMap",
            "delete",
            0,
            vec![ReceiverParam(0)],
            SelfType,
            false,
        );
        // keys() -> Vec<K>
        self.register_generic_method(
            "HashMap",
            "keys",
            0,
            vec![],
            GenericContainer {
                name: "Vec".to_string(),
                args: vec![ReceiverParam(0)],
            },
            false,
        );
        // values() -> Vec<V>
        self.register_generic_method(
            "HashMap",
            "values",
            0,
            vec![],
            GenericContainer {
                name: "Vec".to_string(),
                args: vec![ReceiverParam(1)],
            },
            false,
        );
        // entries() -> Vec<[K,V]>
        self.register_generic_method(
            "HashMap",
            "entries",
            0,
            vec![],
            Concrete(BuiltinTypes::any()),
            false,
        ); // tuple type not expressible
        // len() -> number
        self.register_generic_method(
            "HashMap",
            "len",
            0,
            vec![],
            Concrete(BuiltinTypes::number()),
            false,
        );
        // isEmpty() -> bool
        self.register_generic_method(
            "HashMap",
            "isEmpty",
            0,
            vec![],
            Concrete(BuiltinTypes::boolean()),
            false,
        );
        // map<U>(fn(K,V) -> U) -> HashMap<K,U>
        self.register_generic_method(
            "HashMap",
            "map",
            1,
            vec![Function {
                params: vec![ReceiverParam(0), ReceiverParam(1)],
                returns: Box::new(MethodParam(0)),
            }],
            GenericContainer {
                name: "HashMap".to_string(),
                args: vec![ReceiverParam(0), MethodParam(0)],
            },
            false,
        );
        // filter(fn(K,V) -> bool) -> HashMap<K,V>
        self.register_generic_method(
            "HashMap",
            "filter",
            0,
            vec![Function {
                params: vec![ReceiverParam(0), ReceiverParam(1)],
                returns: Box::new(Concrete(BuiltinTypes::boolean())),
            }],
            SelfType,
            false,
        );
        // forEach(fn(K,V) -> void) -> void
        self.register_generic_method(
            "HashMap",
            "forEach",
            0,
            vec![Function {
                params: vec![ReceiverParam(0), ReceiverParam(1)],
                returns: Box::new(Concrete(BuiltinTypes::void())),
            }],
            Concrete(BuiltinTypes::void()),
            false,
        );
    }

    /// Register a generic method for a type
    fn register_generic_method(
        &mut self,
        type_name: &str,
        method_name: &str,
        method_type_params: usize,
        param_types: Vec<TypeParamExpr>,
        return_type: TypeParamExpr,
        is_fallible: bool,
    ) {
        let key = (type_name.to_string(), method_name.to_string());
        self.generic_methods.insert(
            key,
            GenericMethodSignature {
                name: method_name.to_string(),
                method_type_params,
                param_types,
                return_type,
                is_fallible,
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
            Type::Concrete(TypeAnnotation::Reference(name)) => name.clone(),
            Type::Concrete(TypeAnnotation::Array(_)) => "Vec".to_string(),
            Type::Generic { base, .. } => {
                if let Type::Concrete(TypeAnnotation::Reference(name)) = base.as_ref() {
                    name.clone()
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
                .unwrap_or_else(|| Type::Variable(TypeVar::fresh())),
            TypeParamExpr::MethodParam(idx) => method_vars
                .get(*idx)
                .cloned()
                .unwrap_or_else(|| Type::Variable(TypeVar::fresh())),
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
                    base: Box::new(Type::Concrete(TypeAnnotation::Reference(name.clone()))),
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
                            "AnyError".to_string(),
                        )));
                    }
                    (Some(name.clone()), params)
                } else {
                    (None, vec![])
                }
            }
            Type::Concrete(TypeAnnotation::Array(elem)) => {
                (Some("Vec".to_string()), vec![Type::Concrete(*elem.clone())])
            }
            Type::Concrete(TypeAnnotation::Basic(name)) => (Some(name.clone()), vec![]),
            Type::Concrete(TypeAnnotation::Reference(name)) => (Some(name.clone()), vec![]),
            Type::Concrete(TypeAnnotation::Generic { name, args }) => {
                let mut params: Vec<Type> =
                    args.iter().map(|a| Type::Concrete(a.clone())).collect();
                if name == "Result" && params.len() == 1 {
                    params.push(Type::Concrete(TypeAnnotation::Reference(
                        "AnyError".to_string(),
                    )));
                }
                (Some(name.clone()), params)
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
                .map(|_| Type::Variable(TypeVar::fresh()))
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
    fn test_lookup_string_method() {
        let table = MethodTable::new();
        let string_type = BuiltinTypes::string();

        let sig = table.lookup(&string_type, "len");
        assert!(sig.is_some());

        let sig = table.lookup(&string_type, "nonexistent");
        assert!(sig.is_none());
    }

    #[test]
    fn test_lookup_array_method() {
        let table = MethodTable::new();
        let array_type = BuiltinTypes::array(BuiltinTypes::number());

        let sig = table.lookup(&array_type, "len");
        assert!(sig.is_some());

        let sig = table.lookup(&array_type, "map");
        assert!(sig.is_some());
    }

    #[test]
    fn test_methods_for_type_array() {
        let table = MethodTable::new();
        let methods = table.methods_for_type("Vec");
        let names: Vec<&str> = methods.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"len"));
        assert!(names.contains(&"map"));
        assert!(names.contains(&"filter"));
        assert!(names.contains(&"reduce"));
        assert!(names.contains(&"forEach"));
        assert!(names.contains(&"some"));
        assert!(names.contains(&"every"));
        assert!(
            methods.len() >= 13,
            "Array should have at least 13 methods, got {}",
            methods.len()
        );
    }

    #[test]
    fn test_methods_for_type_string() {
        let table = MethodTable::new();
        let methods = table.methods_for_type("string");
        let names: Vec<&str> = methods.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"toLowerCase"));
        assert!(names.contains(&"split"));
        assert!(names.contains(&"contains"));
        assert!(names.contains(&"trim"));
        assert!(
            methods.len() >= 10,
            "string should have at least 10 methods, got {}",
            methods.len()
        );
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
        let user_type = Type::Concrete(TypeAnnotation::Reference("User".to_string()));
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
    fn test_resolve_array_first() {
        let table = MethodTable::new();
        let array_type = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Vec".to_string()))),
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

        let table_type = Type::Concrete(TypeAnnotation::Reference("Table".to_string()));
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
            Type::Concrete(TypeAnnotation::Reference("Table".to_string())),
        );

        let table_type = Type::Concrete(TypeAnnotation::Reference("Table".to_string()));
        assert!(table.lookup(&table_type, "smooth").is_some());

        let methods = table.methods_for_type("Table");
        let names: Vec<&str> = methods.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"smooth"));
    }

    #[test]
    fn test_resolve_generic_array_filter() {
        let table = MethodTable::new();
        let array_type = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Vec".to_string()))),
            args: vec![BuiltinTypes::number()],
        };
        let result = table.resolve_method_call(&array_type, "filter", &[]);
        assert!(result.is_some());
        // filter returns SelfType, so should be same as receiver
        let rt = result.unwrap();
        assert!(
            matches!(rt, Type::Generic { .. }),
            "filter should return Vec<number>, got {:?}",
            rt
        );
    }

    #[test]
    fn test_resolve_generic_array_map() {
        let table = MethodTable::new();
        let array_type = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Vec".to_string()))),
            args: vec![BuiltinTypes::string()],
        };
        let result = table.resolve_method_call(&array_type, "map", &[]);
        assert!(result.is_some());
        // map returns Vec<U> where U is a fresh type variable
        let rt = result.unwrap();
        assert!(
            matches!(rt, Type::Generic { .. }),
            "map should return Vec<U>, got {:?}",
            rt
        );
    }

    #[test]
    fn test_resolve_generic_option_unwrap() {
        let table = MethodTable::new();
        let option_type = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Option".to_string(),
            ))),
            args: vec![BuiltinTypes::number()],
        };
        let result = table.resolve_method_call(&option_type, "unwrap", &[]);
        assert!(result.is_some());
        // unwrap returns ReceiverParam(0) = number
        assert!(
            matches!(result.unwrap(), Type::Concrete(TypeAnnotation::Basic(ref n)) if n == "number")
        );
    }

    #[test]
    fn test_resolve_generic_hashmap_get() {
        let table = MethodTable::new();
        let map_type = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "HashMap".to_string(),
            ))),
            args: vec![BuiltinTypes::string(), BuiltinTypes::number()],
        };
        let result = table.resolve_method_call(&map_type, "get", &[]);
        assert!(result.is_some());
        // get returns Option<V> = Option<number>
        let rt = result.unwrap();
        assert!(
            matches!(&rt, Type::Generic { base, args }
                if matches!(base.as_ref(), Type::Concrete(TypeAnnotation::Reference(n)) if n == "Option")
                && args.len() == 1
            ),
            "get should return Option<number>, got {:?}",
            rt
        );
    }

    #[test]
    fn test_resolve_generic_hashmap_keys() {
        let table = MethodTable::new();
        let map_type = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "HashMap".to_string(),
            ))),
            args: vec![BuiltinTypes::string(), BuiltinTypes::number()],
        };
        let result = table.resolve_method_call(&map_type, "keys", &[]);
        assert!(result.is_some());
        // keys returns Vec<K> = Vec<string>
        let rt = result.unwrap();
        assert!(
            matches!(&rt, Type::Generic { base, args }
                if matches!(base.as_ref(), Type::Concrete(TypeAnnotation::Reference(n)) if n == "Vec")
                && args.len() == 1
            ),
            "keys should return Vec<string>, got {:?}",
            rt
        );
    }

    #[test]
    fn test_resolve_generic_table_filter_selftype() {
        let table = MethodTable::new();
        let table_type = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                "Table".to_string(),
            ))),
            args: vec![Type::Concrete(TypeAnnotation::Reference(
                "Candle".to_string(),
            ))],
        };
        let result = table.resolve_method_call(&table_type, "filter", &[]);
        assert!(result.is_some());
        // filter returns SelfType = Table<Candle>
        let rt = result.unwrap();
        assert!(
            matches!(rt, Type::Generic { .. }),
            "filter should return Table<Candle>, got {:?}",
            rt
        );
    }

    #[test]
    fn test_is_self_returning() {
        let table = MethodTable::new();
        assert!(table.is_self_returning("Vec", "filter"));
        assert!(table.is_self_returning("Vec", "sort"));
        assert!(table.is_self_returning("Table", "filter"));
        assert!(table.is_self_returning("Table", "orderBy"));
        assert!(table.is_self_returning("Table", "head"));
        assert!(!table.is_self_returning("Vec", "map"));
        assert!(!table.is_self_returning("Vec", "find"));
        assert!(!table.is_self_returning("Table", "count"));
    }

    #[test]
    fn test_takes_closure_with_receiver_param() {
        let table = MethodTable::new();
        assert!(table.takes_closure_with_receiver_param("Vec", "filter"));
        assert!(table.takes_closure_with_receiver_param("Vec", "map"));
        assert!(table.takes_closure_with_receiver_param("Table", "filter"));
        assert!(table.takes_closure_with_receiver_param("Table", "forEach"));
        assert!(!table.takes_closure_with_receiver_param("Vec", "len"));
        assert!(!table.takes_closure_with_receiver_param("Table", "count"));
    }
}

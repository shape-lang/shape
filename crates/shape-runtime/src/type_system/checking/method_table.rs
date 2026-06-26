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
    /// Resolves the inner expression to a container type, then projects out
    /// that container's element type (one level of un-nesting). Used by
    /// `Iterator<Array<T>>.flatten() -> Iterator<T>`, where the flattened
    /// output element is the INNER element type of the nested-array receiver
    /// (`ElementOf(ReceiverParam(0))` projects `Vec<int>` → `int`).
    ///
    /// The existing `ReceiverParam` / `MethodParam` combinators can only name a
    /// type-param directly; they have no "element-of-element" form. Rather than
    /// leave flatten's output element as an unconstrained `MethodParam(0)` var
    /// (which leaves `a + x` over the flattened element typed `unknown` and the
    /// strict checker rejects it), `ElementOf` resolves the receiver-param
    /// container and extracts its single element type. When the inner
    /// expression does not resolve to a recognized one-arg container, this
    /// yields a placeholder TypeVar so the un-inferable case SURFACEs (the
    /// `Add` strict error) rather than being silently mistyped.
    ElementOf(Box<TypeParamExpr>),
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
    /// J-CT.1: methods registered by a `comptime impl` block, indexed by
    /// (receiver type name, method name). The type-checker rejects calls
    /// to these methods outside a `comptime { ... }` context.
    comptime_methods: std::collections::HashSet<(String, String)>,
}

impl MethodTable {
    pub fn new() -> Self {
        let mut table = MethodTable {
            methods: HashMap::new(),
            generic_methods: HashMap::new(),
            comptime_methods: std::collections::HashSet::new(),
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

        // Builtin collection methods (Vec / string / HashMap).
        //
        // STRICT-FLIP (v0.3.3, collection-dispatch root #1): the strict
        // type-checker runs over the user program only — it never sees the
        // stdlib `extend Vec<T>` / `extend string` / `extend HashMap<K,V>`
        // blocks that normally register these signatures at inference time
        // (compiler_impl_reference_model.rs builds `analysis_program` from the
        // user file, not the prelude). Without seeding, every valid collection
        // method call (`[1,2,3].map(...)`, `"hi".split(...)`, `m.get(k)`) had
        // `resolve_method_call` return `None`, fell through to the
        // HasField / HasMethod fallback, and surfaced a spurious
        // "cannot have fields" / "Method 'X' not found on type 'Vec'/'string'"
        // — the #1 strict false-positive class.
        //
        // The runtime dispatches these correctly through shape-vm's PHF method
        // registry; the checker's MethodTable was simply incomplete. The seed
        // below mirrors the canonical stdlib `.shape` definitions
        // (stdlib-src/core/{vec,string_methods,hashmap_methods}.shape) so the
        // checker resolves them. A method that is genuinely NOT a stdlib
        // collection method (e.g. `[1].frobnicate()`) is still absent here and
        // still errors — this is a correct resolution, not a blanket suppress.
        self.register_builtin_collection_methods();
        self.register_datetime_methods();
    }

    /// Seed the canonical `DateTime` instance-method signatures for the
    /// strict type-checker.
    ///
    /// STRICT-FLIP (v0.3.3, book-gate DateTime fix): the strict checker
    /// runs over the user program only and never sees the stdlib `extend`
    /// blocks, so a `DateTime.now()` value's instance methods
    /// (`dt.format(...)`, `.year()`, `.add_days(n)`, …) had `lookup`
    /// return `None`. The downstream effect is worse than a spurious
    /// rejection: with no return-type signature the call's result kind was
    /// left unstamped and the runtime read a heap `Arc<TemporalData>`
    /// pointer back as a raw `i64` (`d.year()` returning garbage like
    /// `-1407374883553280`). This mirrors the
    /// `register_builtin_collection_methods` seed (Vec/String/HashMap) and
    /// the canonical `DATETIME_METHODS` PHF map in shape-vm
    /// (`executor/objects/method_registry.rs`). Receiver type name is
    /// `"DateTime"` (a `DateTime.now()` value infers to
    /// `Type::Concrete(TypeAnnotation::Reference("DateTime"))`).
    fn register_datetime_methods(&mut self) {
        let dt = "DateTime";
        let datetime_ty = || Type::Concrete(TypeAnnotation::Reference("DateTime".into()));
        let int = BuiltinTypes::integer;
        let string = BuiltinTypes::string;
        let boolean = BuiltinTypes::boolean;

        // Component access — `-> int`, no args.
        for m in [
            "year",
            "month",
            "day",
            "hour",
            "minute",
            "second",
            "millisecond",
            "microsecond",
            "day_of_week",
            "day_of_year",
            "week_of_year",
            "unix_timestamp",
            "to_unix_millis",
        ] {
            self.register_method(dt, m, vec![], int(), false);
        }

        // Day-info predicates — `-> bool`, no args.
        for m in ["is_weekday", "is_weekend"] {
            self.register_method(dt, m, vec![], boolean(), false);
        }

        // Formatting / timezone-name / offset — `-> string`.
        self.register_method(dt, "format", vec![string()], string(), false);
        for m in ["iso8601", "rfc2822", "timezone", "offset"] {
            self.register_method(dt, m, vec![], string(), false);
        }

        // Timezone conversions — `-> DateTime`.
        self.register_method(dt, "to_utc", vec![], datetime_ty(), false);
        self.register_method(dt, "to_local", vec![], datetime_ty(), false);
        self.register_method(dt, "to_timezone", vec![string()], datetime_ty(), false);

        // Arithmetic — `(int) -> DateTime`.
        for m in [
            "add_days",
            "add_hours",
            "add_minutes",
            "add_seconds",
            "add_months",
        ] {
            self.register_method(dt, m, vec![int()], datetime_ty(), false);
        }

        // Operator-trait arithmetic — `(DateTime-or-TimeSpan) -> DateTime`.
        // The VM `v2_add`/`v2_sub` dispatch on the rhs `TemporalData` arm;
        // the common surface is a `DateTime` rhs, so seed that.
        self.register_method(dt, "add", vec![datetime_ty()], datetime_ty(), false);
        self.register_method(dt, "sub", vec![datetime_ty()], datetime_ty(), false);

        // Comparison — `(DateTime) -> bool`.
        for m in ["is_before", "is_after", "is_same_day"] {
            self.register_method(dt, m, vec![datetime_ty()], boolean(), false);
        }
    }

    /// Seed the canonical builtin collection-method signatures for the strict
    /// type-checker. Mirrors stdlib `extend Vec<T>` / `extend string` /
    /// `extend HashMap<K,V>`. See `register_builtin_methods` for why this is
    /// needed (the strict analysis path doesn't load the stdlib prelude).
    fn register_builtin_collection_methods(&mut self) {
        use TypeParamExpr as E;

        let func = |params: Vec<E>, returns: E| E::Function {
            params,
            returns: Box::new(returns),
        };
        let vec_of = |arg: E| E::GenericContainer {
            name: "Vec".to_string(),
            args: vec![arg],
        };
        let opt_of = |arg: E| E::GenericContainer {
            name: "Option".to_string(),
            args: vec![arg],
        };
        let hashmap_of = |k: E, v: E| E::GenericContainer {
            name: "HashMap".to_string(),
            args: vec![k, v],
        };
        let iter_of = |arg: E| E::GenericContainer {
            name: "Iterator".to_string(),
            args: vec![arg],
        };
        let int = || E::Concrete(BuiltinTypes::integer());
        let num = || E::Concrete(BuiltinTypes::number());
        let boolean = || E::Concrete(BuiltinTypes::boolean());
        let string = || E::Concrete(BuiltinTypes::string());
        let void = || E::Concrete(Type::Concrete(TypeAnnotation::Basic("void".into())));

        // ---- Vec<T> (receiver param 0 = T) -----------------------------
        // (name, method_type_params, param_types, return_type)
        let vec_methods: Vec<(&str, usize, Vec<E>, E)> = vec![
            ("first", 0, vec![], E::ReceiverParam(0)),
            ("last", 0, vec![], E::ReceiverParam(0)),
            // Wave-1b SEAM A: `iter()` produces a lazy `Iterator<T>`. The
            // Iterator receiver itself carries the adapter/terminal sigs
            // (`register_iterator_methods` below). Mirrors the W13 lazy-iterator
            // factory (`Array.iter`, method_registry.rs:348). Runtime body is
            // SEAM B.
            ("iter", 0, vec![], iter_of(E::ReceiverParam(0))),
            ("push", 0, vec![E::ReceiverParam(0)], E::SelfType),
            ("pop", 0, vec![], E::ReceiverParam(0)),
            ("reverse", 0, vec![], E::SelfType),
            ("clone", 0, vec![], E::SelfType),
            ("toArray", 0, vec![], E::SelfType),
            (
                "filter",
                0,
                vec![func(vec![E::ReceiverParam(0)], boolean())],
                E::SelfType,
            ),
            (
                "map",
                1,
                vec![func(vec![E::ReceiverParam(0)], E::MethodParam(0))],
                vec_of(E::MethodParam(0)),
            ),
            (
                "reduce",
                1,
                vec![
                    func(
                        vec![E::MethodParam(0), E::ReceiverParam(0)],
                        E::MethodParam(0),
                    ),
                    E::MethodParam(0),
                ],
                E::MethodParam(0),
            ),
            (
                "find",
                0,
                vec![func(vec![E::ReceiverParam(0)], boolean())],
                E::ReceiverParam(0),
            ),
            (
                "findIndex",
                0,
                vec![func(vec![E::ReceiverParam(0)], boolean())],
                int(),
            ),
            (
                "forEach",
                0,
                vec![func(vec![E::ReceiverParam(0)], void())],
                void(),
            ),
            (
                "some",
                0,
                vec![func(vec![E::ReceiverParam(0)], boolean())],
                boolean(),
            ),
            (
                "every",
                0,
                vec![func(vec![E::ReceiverParam(0)], boolean())],
                boolean(),
            ),
            ("join", 0, vec![string()], string()),
            ("slice", 0, vec![int(), int()], E::SelfType),
            ("take", 0, vec![int()], E::SelfType),
            ("drop", 0, vec![int()], E::SelfType),
            // `flatten()` on `Array<Array<T>>` removes one level → `Array<T>`,
            // which is exactly the receiver's element type `ReceiverParam(0)`
            // (the receiver type-param of `Array<Array<T>>` is `Array<T>`).
            // `SelfType` was wrong: it modeled the result as the *nested*
            // `Array<Array<T>>`, so the strict checker rejected the assignment
            // `let flat: Array<T> = nested.flatten()` (Vec<Vec<int>> not
            // compatible with Vec<int>). The VM-side `handle_flatten_v2`
            // concatenates one level — element-of-element is the correct type.
            ("flatten", 0, vec![], E::ReceiverParam(0)),
            ("unique", 0, vec![], E::SelfType),
            ("concat", 0, vec![E::SelfType], E::SelfType),
            ("indexOf", 0, vec![E::ReceiverParam(0)], int()),
            ("includes", 0, vec![E::ReceiverParam(0)], boolean()),
            (
                "flatMap",
                1,
                vec![func(vec![E::ReceiverParam(0)], vec_of(E::MethodParam(0)))],
                vec_of(E::MethodParam(0)),
            ),
            (
                "sortBy",
                0,
                vec![func(vec![E::ReceiverParam(0)], num())],
                E::SelfType,
            ),
            // `.sort()` / `.sort(cmp)` — PHF-only (handle_sort_v2; no stdlib
            // .shape def per vec.shape:166), so the mirror-stdlib seed missed
            // it. Comparator `(T,T)->number`; resolver ignores arity so bare
            // `.sort()` resolves too. Returns a sorted copy (non-mutating).
            (
                "sort",
                0,
                vec![func(vec![E::ReceiverParam(0), E::ReceiverParam(0)], num())],
                E::SelfType,
            ),
            // `groupBy<K>(key_fn: (T)=>K) -> Vec<T>` (vec.shape:250) — present
            // in stdlib but missed by the seed.
            (
                "groupBy",
                1,
                vec![func(vec![E::ReceiverParam(0)], E::MethodParam(0))],
                E::SelfType,
            ),
            // LINQ / query-DSL + remaining builtin Vec methods — PHF-registry
            // (array_transform.rs) / Table-Queryable defined, absent from the
            // stdlib `extend Vec` the seed mirrored. Closure-taking entries
            // carry sigs so closure params still infer (`a.where(|x| ...)`);
            // the rest resolve by name (the resolver ignores arity).
            ("length", 0, vec![], int()),
            // `.len()` alias for `.length` — mirrors HashMap `len` (:466) and
            // string `len` (:375); PHF-registered but missed by the seed.
            ("len", 0, vec![], int()),
            ("distinct", 0, vec![], E::SelfType),
            ("skip", 0, vec![int()], E::SelfType),
            ("union", 0, vec![E::SelfType], E::SelfType),
            ("except", 0, vec![E::SelfType], E::SelfType),
            ("intersect", 0, vec![E::SelfType], E::SelfType),
            ("zip", 0, vec![E::SelfType], E::SelfType),
            (
                "where",
                0,
                vec![func(vec![E::ReceiverParam(0)], boolean())],
                E::SelfType,
            ),
            (
                "select",
                1,
                vec![func(vec![E::ReceiverParam(0)], E::MethodParam(0))],
                vec_of(E::MethodParam(0)),
            ),
            (
                "orderBy",
                0,
                vec![func(vec![E::ReceiverParam(0)], num())],
                E::SelfType,
            ),
            (
                "skipWhile",
                0,
                vec![func(vec![E::ReceiverParam(0)], boolean())],
                E::SelfType,
            ),
            (
                "takeWhile",
                0,
                vec![func(vec![E::ReceiverParam(0)], boolean())],
                E::SelfType,
            ),
            (
                "any",
                0,
                vec![func(vec![E::ReceiverParam(0)], boolean())],
                boolean(),
            ),
            (
                "all",
                0,
                vec![func(vec![E::ReceiverParam(0)], boolean())],
                boolean(),
            ),
            (
                "distinctBy",
                0,
                vec![func(vec![E::ReceiverParam(0)], num())],
                E::SelfType,
            ),
            (
                "count",
                0,
                vec![func(vec![E::ReceiverParam(0)], boolean())],
                int(),
            ),
            (
                "single",
                0,
                vec![func(vec![E::ReceiverParam(0)], boolean())],
                E::ReceiverParam(0),
            ),
        ];
        for (name, mtp, params, ret) in vec_methods {
            self.register_user_generic_method("Vec", name, mtp, params, ret, vec![]);
        }
        // Numeric-vector aggregates (`impl NumericVec for Vec`). Receiver-param
        // generic so they register in the generic table alongside the rest.
        // D1 (S4): the receiver-element-dependent aggregates return the
        // receiver's ELEMENT type, not an unconditional `number`. `Array<int>
        // .sum()/.min()/.max()` must be `int` (the typed-array method registry
        // returns `KindedSlot::from_<elem>` per receiver-element kind); only
        // `Array<number>` returns `number`.
        //
        // U4-2 FIX: the element type IS `ReceiverParam(0)` directly. For a
        // `Vec`/`Array` receiver, `extract_receiver_info` returns the receiver
        // PARAMS = `[T]` (the element), so `ReceiverParam(0)` already resolves
        // to `int`/`number` — exactly like `first`/`last`/`pop` above (line 330,
        // "receiver param 0 = T"). The previous `ElementOf(ReceiverParam(0))`
        // DOUBLE-projected: `ReceiverParam(0)` gave the element `int`, then
        // `ElementOf(int)` tried to project the element OF `int` → no element →
        // an `_oob` placeholder var that stayed free post-solve and was DROPPED
        // by `finalize_expr_type_table`, so `a.sum()` never reached the span
        // table (the g1 regression surfaced when U4-2 deleted the closure
        // mini-inferencer's hand-rolled `.sum()→int` arm). `ElementOf` is for a
        // receiver-param that is itself a CONTAINER (`flatten`'s
        // `Iterator<Vec<int>>`), not for a flat `Vec<int>` element accessor.
        // `avg`/`mean`/`std`/`variance`/`norm` are genuine `number`-producing
        // reductions regardless of element type (division / sqrt), so they stay
        // `num()`.
        // The element type T of a `Vec<T>`/`Array<T>` receiver IS receiver-param
        // 0 (see line 330 "receiver param 0 = T"); `sum`/`min`/`max` return it.
        let elem_t = || E::ReceiverParam(0);
        let vec_numeric: Vec<(&str, Vec<E>, E)> = vec![
            ("sum", vec![], elem_t()),
            ("avg", vec![], num()),
            ("mean", vec![], num()),
            ("min", vec![], elem_t()),
            ("max", vec![], elem_t()),
            ("std", vec![], num()),
            ("variance", vec![], num()),
            ("dot", vec![vec_of(num())], num()),
            ("norm", vec![], num()),
            ("normalize", vec![], vec_of(num())),
            ("cumsum", vec![], vec_of(num())),
            ("diff", vec![], vec_of(num())),
            ("abs", vec![], vec_of(num())),
        ];
        for (name, params, ret) in vec_numeric {
            self.register_user_generic_method("Vec", name, 0, params, ret, vec![]);
        }

        // ---- string (monomorphic) --------------------------------------
        let str_methods: Vec<(&str, Vec<Type>, Type)> = vec![
            ("len", vec![], BuiltinTypes::integer()),
            // PHF registry has len+length (method_registry.rs:901-902); both
            // -> v2_string_len. Checker seed dropped one of the pair. A-final ROOT D.
            ("length", vec![], BuiltinTypes::integer()),
            // `s.clone()` — strings are immutable `Arc<String>`, so clone is an
            // independent-handle copy (observationally a deep copy). Backs the
            // `clone` keyword desugar (`let s2 = clone s` → `s.clone()`).
            ("clone", vec![], BuiltinTypes::string()),
            ("isEmpty", vec![], BuiltinTypes::boolean()),
            ("toLowerCase", vec![], BuiltinTypes::string()),
            ("toUpperCase", vec![], BuiltinTypes::string()),
            // snake_case aliases documented in book strings.mdx §Methods.
            // Resolve to the same handlers as their camelCase equivalents
            // (PHF registry: method_registry.rs:910-920).
            ("to_lower_case", vec![], BuiltinTypes::string()),
            ("to_upper_case", vec![], BuiltinTypes::string()),
            ("trim", vec![], BuiltinTypes::string()),
            (
                "split",
                vec![BuiltinTypes::string()],
                Type::Concrete(TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                    "string".into(),
                )))),
            ),
            (
                "contains",
                vec![BuiltinTypes::string()],
                BuiltinTypes::boolean(),
            ),
            (
                "startsWith",
                vec![BuiltinTypes::string()],
                BuiltinTypes::boolean(),
            ),
            (
                "endsWith",
                vec![BuiltinTypes::string()],
                BuiltinTypes::boolean(),
            ),
            (
                "replace",
                vec![BuiltinTypes::string(), BuiltinTypes::string()],
                BuiltinTypes::string(),
            ),
            ("trimStart", vec![], BuiltinTypes::string()),
            ("trimEnd", vec![], BuiltinTypes::string()),
            // snake_case aliases (book strings.mdx §Methods).
            ("trim_start", vec![], BuiltinTypes::string()),
            ("trim_end", vec![], BuiltinTypes::string()),
            ("toNumber", vec![], BuiltinTypes::number()),
            ("toBool", vec![], BuiltinTypes::boolean()),
            (
                "chars",
                vec![],
                Type::Concrete(TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                    "string".into(),
                )))),
            ),
            (
                "padStart",
                vec![BuiltinTypes::integer(), BuiltinTypes::string()],
                BuiltinTypes::string(),
            ),
            (
                "padEnd",
                vec![BuiltinTypes::integer(), BuiltinTypes::string()],
                BuiltinTypes::string(),
            ),
            (
                "repeat",
                vec![BuiltinTypes::integer()],
                BuiltinTypes::string(),
            ),
            (
                "charAt",
                vec![BuiltinTypes::integer()],
                BuiltinTypes::string(),
            ),
            ("reverse", vec![], BuiltinTypes::string()),
            (
                "indexOf",
                vec![BuiltinTypes::string()],
                BuiltinTypes::integer(),
            ),
            ("isDigit", vec![], BuiltinTypes::boolean()),
            ("isAlpha", vec![], BuiltinTypes::boolean()),
            (
                "codePointAt",
                vec![BuiltinTypes::integer()],
                BuiltinTypes::integer(),
            ),
            (
                "substring",
                vec![BuiltinTypes::integer(), BuiltinTypes::integer()],
                BuiltinTypes::string(),
            ),
            (
                "normalize",
                vec![BuiltinTypes::string()],
                BuiltinTypes::string(),
            ),
            (
                "graphemes",
                vec![],
                Type::Concrete(TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                    "string".into(),
                )))),
            ),
            ("graphemeLen", vec![], BuiltinTypes::integer()),
            ("isAscii", vec![], BuiltinTypes::boolean()),
            (
                "slice",
                vec![BuiltinTypes::integer(), BuiltinTypes::integer()],
                BuiltinTypes::string(),
            ),
            ("toString", vec![], BuiltinTypes::string()),
            ("join", vec![BuiltinTypes::string()], BuiltinTypes::string()),
            // Wave-1b SEAM A: `"abc".iter()` -> Iterator<string> (per-char).
            // Mirrors W13 `String.iter`. Runtime body is SEAM B.
            (
                "iter",
                vec![],
                Type::Generic {
                    base: Box::new(Type::Concrete(TypeAnnotation::Reference("Iterator".into()))),
                    args: vec![BuiltinTypes::string()],
                },
            ),
        ];
        for (name, params, ret) in str_methods {
            self.register_method("string", name, params, ret, false);
        }

        // ---- HashMap<K,V> (receiver param 0 = K, 1 = V) ----------------
        let map_methods: Vec<(&str, usize, Vec<E>, E)> = vec![
            (
                "get",
                0,
                vec![E::ReceiverParam(0)],
                opt_of(E::ReceiverParam(1)),
            ),
            (
                "set",
                0,
                vec![E::ReceiverParam(0), E::ReceiverParam(1)],
                hashmap_of(E::ReceiverParam(0), E::ReceiverParam(1)),
            ),
            ("has", 0, vec![E::ReceiverParam(0)], boolean()),
            ("includes", 0, vec![E::ReceiverParam(0)], boolean()),
            (
                "delete",
                0,
                vec![E::ReceiverParam(0)],
                hashmap_of(E::ReceiverParam(0), E::ReceiverParam(1)),
            ),
            // U3 (SB-9 deletion): `remove(key)` returns the removed value
            // `Option<V>` (tuple-return mutator, runtime handler `v2_remove`).
            // Previously absent from the inference table — exposed once all
            // HashMaps route through the single HashMapData carrier.
            (
                "remove",
                0,
                vec![E::ReceiverParam(0)],
                opt_of(E::ReceiverParam(1)),
            ),
            ("keys", 0, vec![], vec_of(E::ReceiverParam(0))),
            ("values", 0, vec![], vec_of(E::ReceiverParam(1))),
            ("entries", 0, vec![], vec_of(vec_of(E::ReceiverParam(0)))),
            // Wave-1b SEAM A: `m.iter()` -> Iterator<[K]> (entry pairs, mirrors
            // `entries`). W13 `HashMap.iter` factory. Runtime body is SEAM B.
            ("iter", 0, vec![], iter_of(vec_of(E::ReceiverParam(0)))),
            ("len", 0, vec![], int()),
            ("isEmpty", 0, vec![], boolean()),
            (
                "map",
                1,
                vec![func(
                    vec![E::ReceiverParam(0), E::ReceiverParam(1)],
                    E::MethodParam(0),
                )],
                hashmap_of(E::ReceiverParam(0), E::MethodParam(0)),
            ),
            (
                "filter",
                0,
                vec![func(
                    vec![E::ReceiverParam(0), E::ReceiverParam(1)],
                    boolean(),
                )],
                E::SelfType,
            ),
            (
                "forEach",
                0,
                vec![func(vec![E::ReceiverParam(0), E::ReceiverParam(1)], void())],
                void(),
            ),
            // PHF-defined HashMap methods absent from the seed's stdlib mirror.
            (
                "getOrDefault",
                0,
                vec![E::ReceiverParam(0), E::ReceiverParam(1)],
                E::ReceiverParam(1),
            ),
            ("merge", 0, vec![E::SelfType], E::SelfType),
            (
                "reduce",
                1,
                vec![
                    func(
                        vec![E::MethodParam(0), E::ReceiverParam(0), E::ReceiverParam(1)],
                        E::MethodParam(0),
                    ),
                    E::MethodParam(0),
                ],
                E::MethodParam(0),
            ),
            ("toArray", 0, vec![], vec_of(E::ReceiverParam(1))),
            (
                "groupBy",
                1,
                vec![func(
                    vec![E::ReceiverParam(0), E::ReceiverParam(1)],
                    E::MethodParam(0),
                )],
                E::SelfType,
            ),
        ];
        for (name, mtp, params, ret) in map_methods {
            self.register_user_generic_method("HashMap", name, mtp, params, ret, vec![]);
        }

        // ---- Set<T> (receiver param 0 = T) -----------------------------
        // STRICT-FLIP (v0.3.3, SMOKE-s4): mirror the PHF `SET_METHODS`
        // (shape-vm method_registry.rs:492) so `.add` / `.len` / etc. resolve
        // in the strict checker. `.size` is intentionally absent per the Set
        // naming policy: use `.len` / `.length`. The ctor is registered in
        // `environment/mod.rs::define_builtin_functions`. `add` / `delete` are
        // `&mut self` mutators at runtime; for the checker they return Self
        // (the s4 fixture discards the result either way).
        let set_methods: Vec<(&str, usize, Vec<E>, E)> = vec![
            ("add", 0, vec![E::ReceiverParam(0)], E::SelfType),
            ("delete", 0, vec![E::ReceiverParam(0)], E::SelfType),
            ("has", 0, vec![E::ReceiverParam(0)], boolean()),
            ("includes", 0, vec![E::ReceiverParam(0)], boolean()),
            ("len", 0, vec![], int()),
            ("length", 0, vec![], int()),
            ("isEmpty", 0, vec![], boolean()),
            ("toArray", 0, vec![], vec_of(E::ReceiverParam(0))),
            ("union", 0, vec![E::SelfType], E::SelfType),
            ("intersection", 0, vec![E::SelfType], E::SelfType),
            ("difference", 0, vec![E::SelfType], E::SelfType),
            (
                "forEach",
                0,
                vec![func(vec![E::ReceiverParam(0)], void())],
                void(),
            ),
            (
                "map",
                1,
                vec![func(vec![E::ReceiverParam(0)], E::MethodParam(0))],
                E::GenericContainer {
                    name: "Set".to_string(),
                    args: vec![E::MethodParam(0)],
                },
            ),
            (
                "filter",
                0,
                vec![func(vec![E::ReceiverParam(0)], boolean())],
                E::SelfType,
            ),
        ];
        for (name, mtp, params, ret) in set_methods {
            self.register_user_generic_method("Set", name, mtp, params, ret, vec![]);
        }

        // ---- Deque<T> (receiver param 0 = T) ---------------------------
        // Mirrors PHF `DEQUE_METHODS` (method_registry.rs:517). Mutators
        // return Self for the checker; `popBack`/`popFront`/`peek*`/`get`
        // return the element type T.
        let deque_methods: Vec<(&str, usize, Vec<E>, E)> = vec![
            ("pushBack", 0, vec![E::ReceiverParam(0)], E::SelfType),
            ("pushFront", 0, vec![E::ReceiverParam(0)], E::SelfType),
            ("popBack", 0, vec![], E::ReceiverParam(0)),
            ("popFront", 0, vec![], E::ReceiverParam(0)),
            ("peekBack", 0, vec![], E::ReceiverParam(0)),
            ("peekFront", 0, vec![], E::ReceiverParam(0)),
            ("get", 0, vec![int()], E::ReceiverParam(0)),
            ("size", 0, vec![], int()),
            ("len", 0, vec![], int()),
            ("length", 0, vec![], int()),
            ("isEmpty", 0, vec![], boolean()),
            ("toArray", 0, vec![], vec_of(E::ReceiverParam(0))),
        ];
        for (name, mtp, params, ret) in deque_methods {
            self.register_user_generic_method("Deque", name, mtp, params, ret, vec![]);
        }

        // ---- PriorityQueue<T> (receiver param 0 = T) -------------------
        // Mirrors PHF `PRIORITY_QUEUE_METHODS` (method_registry.rs:539).
        let pq_methods: Vec<(&str, usize, Vec<E>, E)> = vec![
            ("push", 0, vec![E::ReceiverParam(0)], E::SelfType),
            ("pop", 0, vec![], E::ReceiverParam(0)),
            ("peek", 0, vec![], E::ReceiverParam(0)),
            ("size", 0, vec![], int()),
            ("len", 0, vec![], int()),
            ("length", 0, vec![], int()),
            ("isEmpty", 0, vec![], boolean()),
            ("toArray", 0, vec![], vec_of(E::ReceiverParam(0))),
            ("toSortedArray", 0, vec![], vec_of(E::ReceiverParam(0))),
        ];
        for (name, mtp, params, ret) in pq_methods {
            self.register_user_generic_method("PriorityQueue", name, mtp, params, ret, vec![]);
        }

        // ---- Mutex<T> / Atomic ----------------------------------------
        //
        // These are interior-mutability carriers, not COW collection
        // writeback participants. Seed their exact PHF method signatures so
        // strict checking accepts `let m = Mutex(0); m.set(1)` without
        // treating `set`/`store` as a binding reassignment.
        let mutex_methods: Vec<(&str, usize, Vec<E>, E)> = vec![
            ("lock", 0, vec![], E::SelfType),
            ("try_lock", 0, vec![], boolean()),
            ("set", 0, vec![E::ReceiverParam(0)], E::SelfType),
            ("get", 0, vec![], E::ReceiverParam(0)),
        ];
        for (name, mtp, params, ret) in mutex_methods {
            self.register_user_generic_method("Mutex", name, mtp, params, ret, vec![]);
        }

        let atomic_ty = || Type::Concrete(TypeAnnotation::Reference("Atomic".into()));
        self.register_method("Atomic", "load", vec![], BuiltinTypes::integer(), false);
        self.register_method(
            "Atomic",
            "store",
            vec![BuiltinTypes::integer()],
            atomic_ty(),
            false,
        );
        self.register_method(
            "Atomic",
            "fetch_add",
            vec![BuiltinTypes::integer()],
            BuiltinTypes::integer(),
            false,
        );
        self.register_method(
            "Atomic",
            "fetch_sub",
            vec![BuiltinTypes::integer()],
            BuiltinTypes::integer(),
            false,
        );
        self.register_method(
            "Atomic",
            "compare_exchange",
            vec![BuiltinTypes::integer(), BuiltinTypes::integer()],
            BuiltinTypes::integer(),
            false,
        );

        // ---- Range<T> (receiver param 0 = T) ---------------------------
        // Wave-1b SEAM A: `(0..10).iter()` -> Iterator<int>. A range is
        // `Range<int>` (expressions.rs:1423); `extract_receiver_info` keys it
        // under "Range". Only `iter` is seeded here — other Range PHF methods
        // (RANGE_METHODS, method_registry.rs:1048) are out of this seam's
        // scope. Runtime body is SEAM B.
        self.register_user_generic_method(
            "Range",
            "iter",
            0,
            vec![],
            iter_of(E::ReceiverParam(0)),
            vec![],
        );

        // ---- Iterator<T> (receiver param 0 = T) ------------------------
        // Wave-1b SEAM A (user ruling 2026-06-15): Iterator is a REAL
        // user-implementable trait. Seed its adapter/terminal signatures onto
        // the canonical `Iterator` receiver so a chained pipeline
        // (`xs.iter().map(f).filter(g).collect()`) type-resolves. Mirrors the
        // W13 `ITERATOR_METHODS` PHF (method_registry.rs:638). A user
        // `impl Iterator for MyType` inherits the same set via
        // `register_iterator_methods("MyType")` (items.rs::register_impl).
        // Runtime bodies are SEAM B.
        self.register_iterator_methods("Iterator");

        // FOLLOW-UP (remaining concurrency-method seeds): `Lazy` / `Channel`
        // ctors ARE registered in `environment/mod.rs`, but their full PHF
        // method sets (`LAZY_METHODS` / `CHANNEL_METHODS`, method_registry.rs)
        // remain unseeded here. Their signatures cross scheduler / closure /
        // Option-carrier boundaries (`lazy.get()`, `channel.recv()`), so they
        // stay out of this focused Mutex/Atomic metadata fix.
    }

    /// Wave-1b SEAM A (user ruling 2026-06-15): seed the Iterator-trait
    /// adapter + terminal method signatures onto `receiver` (receiver param 0 =
    /// the element type T). Called once for the canonical `Iterator` receiver
    /// (from `register_builtin_collection_methods`) and again, per user type,
    /// from `register_impl` when a user writes `impl Iterator for MyType`. This
    /// is purely additive: it only registers Iterator-trait methods, and only
    /// onto the named receiver — it cannot change resolution of any existing
    /// builtin/trait method on any other type.
    ///
    /// Mirrors the W13 `ITERATOR_METHODS` PHF (method_registry.rs:638):
    /// lazy adapters return a new `Iterator<...>`; eager terminals consume.
    /// Runtime bodies are SEAM B.
    pub fn register_iterator_methods(&mut self, receiver: &str) {
        use TypeParamExpr as E;

        let func = |params: Vec<E>, returns: E| E::Function {
            params,
            returns: Box::new(returns),
        };
        let iter_of = |arg: E| E::GenericContainer {
            name: "Iterator".to_string(),
            args: vec![arg],
        };
        let vec_of = |arg: E| E::GenericContainer {
            name: "Vec".to_string(),
            args: vec![arg],
        };
        let opt_of = |arg: E| E::GenericContainer {
            name: "Option".to_string(),
            args: vec![arg],
        };
        let int = || E::Concrete(BuiltinTypes::integer());
        let boolean = || E::Concrete(BuiltinTypes::boolean());
        let void = || E::Concrete(Type::Concrete(TypeAnnotation::Basic("void".into())));

        // T = element type = receiver param 0.
        let t = || E::ReceiverParam(0);

        // (name, method_type_params, param_types, return_type)
        let iterator_methods: Vec<(&str, usize, Vec<E>, E)> = vec![
            // --- the trait's defining method ---
            // `next(self) -> Option<T>` — the single REQUIRED member of the
            // Iterator trait. Seeded so it resolves on the canonical Iterator
            // receiver; a user impl overrides it with the user's own body.
            ("next", 0, vec![], opt_of(t())),
            // --- lazy adapters (return a new Iterator) ---
            (
                "map",
                1,
                vec![func(vec![t()], E::MethodParam(0))],
                iter_of(E::MethodParam(0)),
            ),
            ("filter", 0, vec![func(vec![t()], boolean())], iter_of(t())),
            ("take", 0, vec![int()], iter_of(t())),
            ("skip", 0, vec![int()], iter_of(t())),
            (
                "flatMap",
                1,
                vec![func(vec![t()], iter_of(E::MethodParam(0)))],
                iter_of(E::MethodParam(0)),
            ),
            // `flatten()` on `Iterator<Array<U>>` removes one level →
            // `Iterator<U>`. The receiver element type `T = ReceiverParam(0)`
            // is itself an array (`Vec<U>`); the flattened output element is
            // its INNER element type `U`, resolved via `ElementOf(t())` (project
            // the one element type out of the nested-array receiver param).
            // Previously this used an unconstrained `MethodParam(0)` var, which
            // left the flattened element typed `unknown` so a downstream
            // binop/reduce (`a + x` over flattened ints) was rejected by the
            // strict checker. With `ElementOf`, a flattened `int` element stays
            // `int` (not number/unknown); a non-array receiver param yields a
            // placeholder var → SURFACE.
            ("flatten", 0, vec![], iter_of(E::ElementOf(Box::new(t())))),
            // `enumerate()` -> Iterator<[int, T]> ([index, value] pairs).
            ("enumerate", 0, vec![], iter_of(vec_of(t()))),
            ("chain", 0, vec![iter_of(t())], iter_of(t())),
            // --- eager terminals (consume the iterator) ---
            ("collect", 0, vec![], vec_of(t())),
            ("toArray", 0, vec![], vec_of(t())),
            ("forEach", 0, vec![func(vec![t()], void())], void()),
            (
                "reduce",
                1,
                vec![
                    func(vec![E::MethodParam(0), t()], E::MethodParam(0)),
                    E::MethodParam(0),
                ],
                E::MethodParam(0),
            ),
            ("count", 0, vec![], int()),
            ("any", 0, vec![func(vec![t()], boolean())], boolean()),
            ("all", 0, vec![func(vec![t()], boolean())], boolean()),
            ("find", 0, vec![func(vec![t()], boolean())], opt_of(t())),
        ];
        for (name, mtp, params, ret) in iterator_methods {
            self.register_user_generic_method(receiver, name, mtp, params, ret, vec![]);
        }
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

    /// J-CT.1: mark a method as comptime-only.
    ///
    /// Called by `register_impl` when the source `impl_block.is_comptime`
    /// is true (i.e. `comptime impl Trait for Type { ... }`). After this,
    /// `is_comptime_method(type_name, method_name)` returns true and the
    /// expression-level type checker rejects runtime call sites.
    pub fn mark_comptime_method(&mut self, type_name: &str, method_name: &str) {
        self.comptime_methods
            .insert((type_name.to_string(), method_name.to_string()));
    }

    /// J-CT.1: query whether a method was registered by a `comptime impl`.
    ///
    /// Used by the method-call type-checker to reject runtime call sites
    /// for compile-time-only methods. Returns `false` for any (type, method)
    /// pair never marked via `mark_comptime_method`.
    pub fn is_comptime_method(&self, type_name: &str, method_name: &str) -> bool {
        self.comptime_methods
            .contains(&(type_name.to_string(), method_name.to_string()))
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
                    // U1: the canonical array carrier is `Generic{Array, ..}`;
                    // array methods are registered under the "Vec" key. Normalize
                    // the canonical "Array" base name to "Vec" so the canonical
                    // carrier resolves the same registered methods as the legacy
                    // `Concrete(Array(_))` spelling (line above) and the empty
                    // `Generic{Array}` literal. Mirrors `extract_receiver_info`.
                    if name.to_string() == "Array" {
                        "Vec".to_string()
                    } else {
                        name.to_string()
                    }
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
        // Out-of-bounds accesses here indicate malformed TypeParamExpr inputs
        // (more indices than params supplied). We return a stable placeholder
        // TypeVar so failures are deterministic rather than depending on a
        // process-global fresh counter.
        let placeholder = || Type::Variable(TypeVar::new("_oob".to_string()));
        match expr {
            TypeParamExpr::Concrete(t) => t.clone(),
            TypeParamExpr::ReceiverParam(idx) => receiver_params
                .get(*idx)
                .cloned()
                .unwrap_or_else(placeholder),
            TypeParamExpr::MethodParam(idx) => {
                method_vars.get(*idx).cloned().unwrap_or_else(placeholder)
            }
            TypeParamExpr::SelfType => receiver_type.clone(),
            TypeParamExpr::ElementOf(inner) => {
                // Resolve the inner expr to a container type, then project out
                // its single element type (one level of un-nesting). For
                // `Iterator<Vec<int>>.flatten()`, inner = ReceiverParam(0)
                // resolves to `Vec<int>`; `extract_receiver_info` returns the
                // element params `[int]`. A non-one-arg-container resolution
                // (e.g. inner is an unresolved var or a scalar) returns the
                // OOB placeholder var so the un-inferable case SURFACEs rather
                // than being silently mistyped.
                let container = Self::resolve_type_param_expr(
                    inner,
                    receiver_type,
                    receiver_params,
                    method_vars,
                );
                let (_name, elem_params) = Self::extract_receiver_info(&container);
                elem_params.into_iter().next().unwrap_or_else(placeholder)
            }
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
                    base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                        name.as_str().into(),
                    ))),
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
                        params.push(Type::Concrete(TypeAnnotation::Reference("AnyError".into())));
                    }
                    // The var-preserving empty-array form (`expressions.rs`
                    // `Expr::Array` empty arm) is `Type::Generic { base:
                    // Reference("Array"), args: [Variable] }`, but every Vec
                    // builtin-collection method is registered under the canonical
                    // "Vec" key (and `Type::Concrete(Array(_))` normalizes to
                    // "Vec" below). Normalize the Generic-Array spelling to the
                    // same key so `lookup_generic_signature("Vec", "push")`
                    // resolves on an unresolved-element empty array. The element
                    // var then binds to the pushed arg via the bare-variable
                    // value-position binding in `expressions.rs::MethodCall`
                    // (R1 empty-array-push let-gen, 2026-06-14).
                    let canonical = if name == "Array" {
                        "Vec".to_string()
                    } else {
                        name.to_string()
                    };
                    (Some(canonical), params)
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
                    params.push(Type::Concrete(TypeAnnotation::Reference("AnyError".into())));
                }
                let canonical = if name == "Array" {
                    "Vec".to_string()
                } else {
                    name.to_string()
                };
                (Some(canonical), params)
            }
            _ => (None, vec![]),
        }
    }

    /// Get return type for a method call, performing basic type checking.
    /// Tries generic method signatures first, then falls back to monomorphic lookup.
    ///
    /// Fresh method type vars are sourced from the caller-provided per-engine
    /// `TypeVarGen`, so IDs are scoped to a single inference run and don't
    /// collide across independent calls.
    pub fn resolve_method_call(
        &self,
        receiver_type: &Type,
        method_name: &str,
        _arg_types: &[Type],
        var_gen: &mut crate::type_system::TypeVarGen,
    ) -> Option<Type> {
        let (type_name, receiver_params) = Self::extract_receiver_info(receiver_type);

        // D-β string-join receiver-kind fix (v0.3 KC #6(d), 2026-05-22):
        // when the receiver is a Type::Variable / Type::Constrained
        // (e.g. `self[i]` on a `Vec<string>` self, which falls through
        // to `push_indexable_constraint` and returns a fresh type-var),
        // `extract_receiver_info` returns `None` and the original early-
        // return at `type_name?` skipped the universal method registry
        // (`UNIVERSAL_RECEIVER`-keyed `toString` / `to_string` / `type`).
        // The cascade: `result + self[i].toString()` inside `Vec.join`'s
        // monomorphized body inferred `unknown` for the RHS, surfaced
        // "Cannot infer types for binary operation `Add`" inside
        // `ensure_monomorphic_function`, leaked Vec.join's
        // `current_blob_builder` past the `?`-early-exit in
        // `compile_function_body` (between take and restore), and
        // `build_content_addressed_program` finalized that leaked builder
        // as the synthetic `__main__` (arity=0). The `__main__` blob
        // disappeared from the linker output, the linker entry pointed
        // to Vec.join's body, execution started inside Vec.join with
        // self/separator slots at the §2.7.7 Bool sentinel —
        // "no method 'len' on receiver kind Bool".
        //
        // Per ADR-006 §2.7.5 the universal methods' return types are
        // statically known at registration time (`register_builtin_methods`
        // above stamps `toString`/`to_string` -> `string`, `type` ->
        // `Type`). When the receiver type didn't extract a name, fall
        // through to the universal registry so the receiver-agnostic
        // method's return type still propagates.
        if type_name.is_none() {
            let universal_key = (UNIVERSAL_RECEIVER.to_string(), method_name.to_string());
            if let Some(sig) = self
                .methods
                .get(&universal_key)
                .and_then(|sigs| sigs.first())
            {
                return Some(sig.return_type.clone());
            }
            return None;
        }
        let type_name = type_name?;

        let key = (type_name, method_name.to_string());
        if let Some(gsig) = self.generic_methods.get(&key) {
            let method_vars: Vec<Type> = (0..gsig.method_type_params)
                .map(|_| var_gen.fresh_type())
                .collect();
            let resolved = Self::resolve_type_param_expr(
                &gsig.return_type,
                receiver_type,
                &receiver_params,
                &method_vars,
            );
            // D1 (S4): the `_oob` placeholder minted by `resolve_type_param_expr`
            // for an un-resolvable projection (e.g. `ElementOf` over a receiver
            // whose element type is not yet pinned) is a FIXED-NAME var. If two
            // call sites both fall into it (`[1,2,3].sum()` and
            // `[1.0,2.0].sum()` in the same program), the shared name unifies
            // their results and produces a spurious `int`-vs-`number` clash.
            // Freshen any `_oob` occurrence per call so each unresolved
            // projection gets its own independent var (which the surrounding
            // inference then solves or surfaces in isolation).
            return Some(Self::freshen_oob_placeholder(resolved, var_gen));
        }

        let sig = self.lookup(receiver_type, method_name)?;
        Some(sig.return_type.clone())
    }

    /// Replace every `_oob` placeholder var (the fixed-name fallback minted by
    /// `resolve_type_param_expr` for an un-resolvable projection) with a fresh
    /// per-call type variable. Keeps two distinct call sites that both hit the
    /// placeholder from accidentally unifying their result types (D1, S4).
    fn freshen_oob_placeholder(ty: Type, var_gen: &mut crate::type_system::TypeVarGen) -> Type {
        match ty {
            Type::Variable(ref v) if v.0 == "_oob" => var_gen.fresh_type(),
            Type::Variable(_) | Type::Concrete(_) => ty,
            Type::Generic { base, args } => Type::Generic {
                base: Box::new(Self::freshen_oob_placeholder(*base, var_gen)),
                args: args
                    .into_iter()
                    .map(|a| Self::freshen_oob_placeholder(a, var_gen))
                    .collect(),
            },
            Type::Function { params, returns } => Type::Function {
                params: params
                    .into_iter()
                    .map(|p| Self::freshen_oob_placeholder(p, var_gen))
                    .collect(),
                returns: Box::new(Self::freshen_oob_placeholder(*returns, var_gen)),
            },
            Type::Constrained { .. } => ty,
        }
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
    fn set_size_name_is_not_registered_in_strict_table() {
        let table = MethodTable::new();
        let set_type = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Set".into()))),
            args: vec![BuiltinTypes::string()],
        };
        let mut tvgen = crate::type_system::TypeVarGen::new();
        let string_type = BuiltinTypes::string();

        assert!(table
            .resolve_method_call(&set_type, "len", &[], &mut tvgen)
            .is_some());
        assert!(table
            .resolve_method_call(&set_type, "length", &[], &mut tvgen)
            .is_some());
        assert!(table
            .resolve_method_call(&set_type, "includes", &[string_type], &mut tvgen)
            .is_some());
        assert!(table
            .resolve_method_call(&set_type, "size", &[], &mut tvgen)
            .is_none());
    }

    #[test]
    fn datetime_instance_methods_seeded_in_strict_table() {
        // v0.3.3 book-gate fix: DateTime instance methods must resolve in
        // the strict checker (the same Vec/String seed pattern). A
        // `DateTime.now()` value infers to Reference("DateTime").
        let table = MethodTable::new();
        let dt = Type::Concrete(TypeAnnotation::Reference("DateTime".into()));

        // Component access -> int.
        let year = table.lookup(&dt, "year").expect("year() must resolve");
        assert!(matches!(
            year.return_type,
            Type::Concrete(TypeAnnotation::Basic(ref n)) if n == "int"
        ));

        // Formatting -> string, takes one string arg.
        let format = table.lookup(&dt, "format").expect("format() must resolve");
        assert_eq!(format.param_types.len(), 1);
        assert!(matches!(
            format.return_type,
            Type::Concrete(TypeAnnotation::Basic(ref n)) if n == "string"
        ));

        // Arithmetic -> DateTime.
        let plus = table
            .lookup(&dt, "add_days")
            .expect("add_days() must resolve");
        assert!(matches!(
            plus.return_type,
            Type::Concrete(TypeAnnotation::Reference(ref n)) if n.as_ref() == "DateTime"
        ));

        // Predicate -> bool.
        let we = table
            .lookup(&dt, "is_weekend")
            .expect("is_weekend() must resolve");
        assert!(matches!(
            we.return_type,
            Type::Concrete(TypeAnnotation::Basic(ref n)) if n == "bool"
        ));

        // A genuinely-absent method still does NOT resolve (not a blanket
        // suppress).
        assert!(table.lookup(&dt, "frobnicate").is_none());
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
            "Vec",
            "first",
            0,
            vec![],
            TypeParamExpr::ReceiverParam(0),
            vec![],
        );

        let array_type = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Vec".into()))),
            args: vec![BuiltinTypes::number()],
        };

        let mut tvgen = crate::type_system::TypeVarGen::new();
        let result = table.resolve_method_call(&array_type, "first", &[], &mut tvgen);
        assert!(result.is_some());
        // Should return the element type (number)
        assert!(
            matches!(result.unwrap(), Type::Concrete(TypeAnnotation::Basic(ref n)) if n == "number")
        );
    }

    #[test]
    fn test_iterator_flatten_resolves_inner_element_type() {
        // Wave 1b FlattenReduce (2026-06-16): `Iterator<Array<int>>.flatten()`
        // un-nests one level → `Iterator<int>`. The registered signature uses
        // `iter_of(ElementOf(ReceiverParam(0)))`; resolving it against an
        // `Iterator<Vec<int>>` receiver must yield `Iterator<int>` (NOT a free
        // `MethodParam` var, NOT `Iterator<Vec<int>>`). int stays int.
        let table = MethodTable::new();
        let int = || Type::Concrete(TypeAnnotation::Basic("int".into()));
        let vec_int = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Vec".into()))),
            args: vec![int()],
        };
        let iter_vec_int = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Iterator".into()))),
            args: vec![vec_int],
        };

        let mut tvgen = crate::type_system::TypeVarGen::new();
        let result = table
            .resolve_method_call(&iter_vec_int, "flatten", &[], &mut tvgen)
            .expect("flatten resolves on Iterator<Vec<int>>");

        // Expect Iterator<int> — the inner element type was projected out.
        match result {
            Type::Generic { base, args } => {
                assert!(
                    matches!(base.as_ref(), Type::Concrete(TypeAnnotation::Reference(n)) if n.as_str() == "Iterator"),
                    "flatten result base must be Iterator, got {base:?}"
                );
                assert_eq!(args.len(), 1, "Iterator carries one element arg");
                assert!(
                    matches!(&args[0], Type::Concrete(TypeAnnotation::Basic(n)) if n == "int"),
                    "flatten element must be int (un-nested), got {:?}",
                    args[0]
                );
            }
            other => panic!("flatten must resolve to Iterator<int>, got {other:?}"),
        }
    }

    #[test]
    fn test_element_of_non_container_surfaces_placeholder() {
        // `ElementOf` over a non-container receiver param (here a scalar `int`)
        // must NOT fabricate a type — it yields the OOB placeholder var so the
        // un-inferable case SURFACEs downstream rather than being mistyped.
        let int = Type::Concrete(TypeAnnotation::Basic("int".into()));
        let resolved = MethodTable::resolve_type_param_expr(
            &TypeParamExpr::ElementOf(Box::new(TypeParamExpr::ReceiverParam(0))),
            &int,
            std::slice::from_ref(&int),
            &[],
        );
        assert!(
            matches!(resolved, Type::Variable(_)),
            "ElementOf of a scalar must surface a placeholder var, got {resolved:?}"
        );
    }

    #[test]
    fn test_register_user_method() {
        let mut table = MethodTable::new();
        let mut tvgen = crate::type_system::TypeVarGen::new();

        // Register a custom method on a user type
        table.register_user_method(
            "Table",
            "query",
            vec![BuiltinTypes::string()],
            tvgen.fresh_type(),
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
        let mut tvgen = crate::type_system::TypeVarGen::new();
        table.register_user_method("Table", "query", vec![], tvgen.fresh_type());

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
            "Vec",
            "filter",
            0,
            vec![TypeParamExpr::Function {
                params: vec![TypeParamExpr::ReceiverParam(0)],
                returns: Box::new(TypeParamExpr::Concrete(BuiltinTypes::boolean())),
            }],
            TypeParamExpr::SelfType,
            vec![],
        );

        let array_type = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Vec".into()))),
            args: vec![BuiltinTypes::number()],
        };
        let mut tvgen = crate::type_system::TypeVarGen::new();
        let result = table.resolve_method_call(&array_type, "filter", &[], &mut tvgen);
        assert!(result.is_some());
        let rt = result.unwrap();
        assert!(
            matches!(rt, Type::Generic { .. }),
            "filter should return Vec<number>, got {:?}",
            rt
        );
    }

    #[test]
    fn test_resolve_generic_map_with_user_registration() {
        let mut table = MethodTable::new();
        table.register_user_generic_method(
            "Vec",
            "map",
            1,
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
        let mut tvgen = crate::type_system::TypeVarGen::new();
        let result = table.resolve_method_call(&array_type, "map", &[], &mut tvgen);
        assert!(result.is_some());
        let rt = result.unwrap();
        assert!(
            matches!(rt, Type::Generic { .. }),
            "map should return Vec<U>, got {:?}",
            rt
        );
    }

    #[test]
    fn test_resolve_generic_option_unwrap_with_user_registration() {
        let mut table = MethodTable::new();
        table.register_user_generic_method(
            "Option",
            "unwrap",
            0,
            vec![],
            TypeParamExpr::ReceiverParam(0),
            vec![],
        );

        let option_type = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Option".into()))),
            args: vec![BuiltinTypes::number()],
        };
        let mut tvgen = crate::type_system::TypeVarGen::new();
        let result = table.resolve_method_call(&option_type, "unwrap", &[], &mut tvgen);
        assert!(result.is_some());
        assert!(
            matches!(result.unwrap(), Type::Concrete(TypeAnnotation::Basic(ref n)) if n == "number")
        );
    }

    #[test]
    fn test_resolve_generic_hashmap_get_with_user_registration() {
        let mut table = MethodTable::new();
        table.register_user_generic_method(
            "HashMap",
            "get",
            0,
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
        let mut tvgen = crate::type_system::TypeVarGen::new();
        let result = table.resolve_method_call(&map_type, "get", &[], &mut tvgen);
        assert!(result.is_some());
        let rt = result.unwrap();
        assert!(
            matches!(&rt, Type::Generic { base, args }
                if matches!(base.as_ref(), Type::Concrete(TypeAnnotation::Reference(n)) if n == "Option")
                && args.len() == 1),
            "get should return Option<number>, got {:?}",
            rt
        );
    }

    #[test]
    fn test_is_self_returning_with_user_registration() {
        let mut table = MethodTable::new();
        table.register_user_generic_method(
            "Vec",
            "filter",
            0,
            vec![],
            TypeParamExpr::SelfType,
            vec![],
        );
        table.register_user_generic_method(
            "Vec",
            "map",
            1,
            vec![],
            TypeParamExpr::GenericContainer {
                name: "Vec".to_string(),
                args: vec![TypeParamExpr::MethodParam(0)],
            },
            vec![],
        );

        assert!(table.is_self_returning("Vec", "filter"));
        assert!(!table.is_self_returning("Vec", "map"));
    }

    #[test]
    fn test_takes_closure_with_receiver_param_with_user_registration() {
        let mut table = MethodTable::new();
        table.register_user_generic_method(
            "Vec",
            "filter",
            0,
            vec![TypeParamExpr::Function {
                params: vec![TypeParamExpr::ReceiverParam(0)],
                returns: Box::new(TypeParamExpr::Concrete(BuiltinTypes::boolean())),
            }],
            TypeParamExpr::SelfType,
            vec![],
        );

        assert!(table.takes_closure_with_receiver_param("Vec", "filter"));
        assert!(!table.takes_closure_with_receiver_param("Vec", "len"));
    }

    // ----------------------------------------------------------------------
    // J-CT.1 — comptime-method marking
    // ----------------------------------------------------------------------

    #[test]
    fn jct1_mark_comptime_method_records_pair() {
        let mut table = MethodTable::new();
        table.mark_comptime_method("Calculator", "eval");
        assert!(
            table.is_comptime_method("Calculator", "eval"),
            "marked method should report comptime-only"
        );
    }

    #[test]
    fn jct1_unmarked_methods_are_not_comptime() {
        let mut table = MethodTable::new();
        // Register a regular user method — must not be classified as comptime.
        table.register_user_method("Calculator", "value", vec![], BuiltinTypes::number());
        assert!(
            !table.is_comptime_method("Calculator", "value"),
            "register_user_method must NOT mark methods as comptime"
        );
    }

    #[test]
    fn jct1_comptime_marker_is_per_type() {
        let mut table = MethodTable::new();
        table.mark_comptime_method("Calculator", "eval");
        assert!(table.is_comptime_method("Calculator", "eval"));
        assert!(
            !table.is_comptime_method("OtherType", "eval"),
            "comptime marker must not leak across types"
        );
        assert!(
            !table.is_comptime_method("Calculator", "other_method"),
            "comptime marker must not leak across method names"
        );
    }
}

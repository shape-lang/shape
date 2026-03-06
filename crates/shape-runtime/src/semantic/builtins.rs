//! Built-in function and variable registration for semantic analysis
//!
//! This module contains all the built-in functions, variables, and intrinsics
//! that are available in Shape programs.

use super::symbol_table::SymbolTable;
use super::types::Type;

/// Register all built-in functions and variables in the symbol table
/// Only generic, domain-agnostic functions should be registered here.
/// Domain-specific functions (finance indicators, IoT patterns, etc.) are
/// registered via stdlib module loading.
pub fn register_builtins(symbol_table: &mut SymbolTable) {
    // Math functions
    symbol_table
        .define_function("abs", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("max", vec![Type::Number, Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("min", vec![Type::Number, Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("avg", vec![Type::Unknown, Type::Number], Type::Number)
        .expect("static builtin registration");

    // Array functions
    symbol_table
        .define_function("highest", vec![Type::Unknown, Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("lowest", vec![Type::Unknown, Type::Number], Type::Number)
        .expect("static builtin registration");

    symbol_table
        .define_function(
            "map",
            vec![
                Type::Column(Box::new(Type::Unknown)),
                Type::Function {
                    params: vec![],
                    returns: Box::new(Type::Unknown),
                },
            ],
            Type::Column(Box::new(Type::Unknown)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "filter",
            vec![
                Type::Column(Box::new(Type::Unknown)),
                Type::Function {
                    params: vec![],
                    returns: Box::new(Type::Bool),
                },
            ],
            Type::Column(Box::new(Type::Unknown)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "count",
            vec![Type::Array(Box::new(Type::Unknown))],
            Type::Number,
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "sum",
            vec![Type::Column(Box::new(Type::Number))],
            Type::Number,
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "mean",
            vec![Type::Column(Box::new(Type::Number))],
            Type::Number,
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "shift",
            vec![Type::Column(Box::new(Type::Unknown)), Type::Number],
            Type::Column(Box::new(Type::Unknown)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "resample",
            vec![
                Type::Column(Box::new(Type::Unknown)),
                Type::String,
                Type::String,
            ],
            Type::Column(Box::new(Type::Unknown)),
        )
        .expect("static builtin registration");

    // throw removed: Shape uses Result types, not exceptions

    // Output - print() is handled as an intrinsic at evaluation time
    // but still registered here for semantic analysis
    symbol_table
        .define_function_with_defaults(
            "print",
            vec![
                Type::Unknown,
                Type::Unknown,
                Type::Unknown,
                Type::Unknown,
                Type::Unknown,
                Type::Unknown,
                Type::Unknown,
                Type::Unknown,
            ],
            Type::Unknown,
            vec![false, true, true, true, true, true, true, true],
        )
        .expect("static builtin registration");
    // Resumability
    symbol_table
        .define_function("snapshot", vec![], Type::Unknown)
        .expect("static builtin registration");
    symbol_table
        .define_function("exit", vec![Type::Number], Type::Unit)
        .expect("static builtin registration");

    // Result type constructors
    symbol_table
        .define_function("Some", vec![Type::Unknown], Type::Unknown)
        .expect("static builtin registration");
    symbol_table
        .define_function("Ok", vec![Type::Unknown], Type::Unknown)
        .expect("static builtin registration");
    symbol_table
        .define_function("Err", vec![Type::Unknown], Type::Unknown)
        .expect("static builtin registration");

    // Formatting functions
    symbol_table
        .define_function("format_percent", vec![Type::Unknown], Type::String)
        .expect("static builtin registration");
    symbol_table
        .define_function_with_defaults(
            "format_number",
            vec![Type::Unknown, Type::Unknown],
            Type::String,
            vec![false, true],
        )
        .expect("static builtin registration");

    // Additional math functions
    symbol_table
        .define_function("sqrt", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("pow", vec![Type::Number, Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("log", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("ln", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("exp", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("floor", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("ceil", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function_with_defaults(
            "round",
            vec![Type::Unknown, Type::Unknown],
            Type::Number,
            vec![false, true],
        )
        .expect("static builtin registration");

    // Intrinsic functions (used by stdlib indicators)
    // These are low-level SIMD-accelerated functions called from Shape stdlib

    // Column/table intrinsics
    symbol_table
        .define_function(
            "__intrinsic_diff",
            vec![Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_shift",
            vec![Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_pct_change",
            vec![Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_fillna",
            vec![Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_cumsum",
            vec![Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_cumprod",
            vec![Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_clip",
            vec![Type::Unknown, Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");

    // Rolling intrinsics
    symbol_table
        .define_function(
            "__intrinsic_rolling_sum",
            vec![Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_rolling_mean",
            vec![Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_rolling_std",
            vec![Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_rolling_min",
            vec![Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_rolling_max",
            vec![Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_ema",
            vec![Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_linear_recurrence",
            vec![Type::Unknown, Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");

    // Math intrinsics
    symbol_table
        .define_function("__intrinsic_sum", vec![Type::Unknown], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("__intrinsic_mean", vec![Type::Unknown], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("__intrinsic_min", vec![Type::Unknown], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("__intrinsic_max", vec![Type::Unknown], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("__intrinsic_std", vec![Type::Unknown], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("__intrinsic_variance", vec![Type::Unknown], Type::Number)
        .expect("static builtin registration");

    // Trig intrinsics
    symbol_table
        .define_function("__intrinsic_sin", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("__intrinsic_cos", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("__intrinsic_tan", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("__intrinsic_asin", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("__intrinsic_acos", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("__intrinsic_atan", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_atan2",
            vec![Type::Number, Type::Number],
            Type::Number,
        )
        .expect("static builtin registration");
    symbol_table
        .define_function("__intrinsic_sinh", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("__intrinsic_cosh", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("__intrinsic_tanh", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    // Character code intrinsics
    symbol_table
        .define_function("__intrinsic_char_code", vec![Type::String], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_from_char_code",
            vec![Type::Number],
            Type::String,
        )
        .expect("static builtin registration");

    // Trig builtins (top-level)
    symbol_table
        .define_function("sin", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("cos", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("tan", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("asin", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("acos", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("atan", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("atan2", vec![Type::Number, Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("sinh", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("cosh", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("tanh", vec![Type::Number], Type::Number)
        .expect("static builtin registration");

    // Random intrinsics
    symbol_table
        .define_function("__intrinsic_random", vec![], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_random_int",
            vec![Type::Number, Type::Number],
            Type::Number,
        )
        .expect("static builtin registration");
    symbol_table
        .define_function("__intrinsic_random_seed", vec![Type::Number], Type::Unit)
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_random_normal",
            vec![Type::Number, Type::Number],
            Type::Number,
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_random_array",
            vec![Type::Number],
            Type::Array(Box::new(Type::Number)),
        )
        .expect("static builtin registration");

    // Distribution intrinsics
    symbol_table
        .define_function(
            "__intrinsic_dist_uniform",
            vec![Type::Number, Type::Number],
            Type::Number,
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_dist_lognormal",
            vec![Type::Number, Type::Number],
            Type::Number,
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_dist_exponential",
            vec![Type::Number],
            Type::Number,
        )
        .expect("static builtin registration");
    symbol_table
        .define_function("__intrinsic_dist_poisson", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_dist_sample_n",
            vec![
                Type::String,
                Type::Array(Box::new(Type::Unknown)),
                Type::Number,
            ],
            Type::Array(Box::new(Type::Number)),
        )
        .expect("static builtin registration");

    // Stochastic process intrinsics
    symbol_table
        .define_function(
            "__intrinsic_brownian_motion",
            vec![Type::Number, Type::Number, Type::Number],
            Type::Array(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_gbm",
            vec![
                Type::Number,
                Type::Number,
                Type::Number,
                Type::Number,
                Type::Number,
            ],
            Type::Array(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_ou_process",
            vec![
                Type::Number,
                Type::Number,
                Type::Number,
                Type::Number,
                Type::Number,
                Type::Number,
            ],
            Type::Array(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_random_walk",
            vec![Type::Number, Type::Number],
            Type::Array(Box::new(Type::Number)),
        )
        .expect("static builtin registration");

    // Array intrinsics
    symbol_table
        .define_function(
            "__intrinsic_map",
            vec![Type::Unknown, Type::Unknown],
            Type::Unknown,
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_filter",
            vec![Type::Unknown, Type::Unknown],
            Type::Unknown,
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_reduce",
            vec![Type::Unknown, Type::Unknown, Type::Unknown],
            Type::Unknown,
        )
        .expect("static builtin registration");

    // Utility functions
    symbol_table
        .define_function("len", vec![Type::Unknown], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function_with_defaults(
            "range",
            vec![Type::Unknown, Type::Unknown, Type::Unknown],
            Type::Array(Box::new(Type::Number)),
            vec![false, true, true],
        )
        .expect("static builtin registration");
    symbol_table
        .define_function("format", vec![Type::Unknown, Type::Unknown], Type::String)
        .expect("static builtin registration");

    // Statistical functions
    symbol_table
        .define_function("stddev", vec![Type::Unknown], Type::Number)
        .expect("static builtin registration");

    // Additional math builtins
    symbol_table
        .define_function("sign", vec![Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("gcd", vec![Type::Number, Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("lcm", vec![Type::Number, Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function("hypot", vec![Type::Number, Type::Number], Type::Number)
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "clamp",
            vec![Type::Number, Type::Number, Type::Number],
            Type::Number,
        )
        .expect("static builtin registration");
    symbol_table
        .define_function("isNaN", vec![Type::Unknown], Type::Bool)
        .expect("static builtin registration");
    symbol_table
        .define_function("is_nan", vec![Type::Unknown], Type::Bool)
        .expect("static builtin registration");
    symbol_table
        .define_function("isFinite", vec![Type::Unknown], Type::Bool)
        .expect("static builtin registration");
    symbol_table
        .define_function("is_finite", vec![Type::Unknown], Type::Bool)
        .expect("static builtin registration");

    // Vector intrinsics
    symbol_table
        .define_function(
            "__intrinsic_vec_abs",
            vec![Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_vec_sqrt",
            vec![Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_vec_ln",
            vec![Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_vec_exp",
            vec![Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_vec_add",
            vec![Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_vec_sub",
            vec![Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_vec_mul",
            vec![Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_vec_div",
            vec![Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_vec_max",
            vec![Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_vec_min",
            vec![Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
    symbol_table
        .define_function(
            "__intrinsic_vec_select",
            vec![Type::Unknown, Type::Unknown, Type::Unknown],
            Type::Column(Box::new(Type::Number)),
        )
        .expect("static builtin registration");
}

//! End-to-end regression tests for the `std::finance` stdlib package.
//!
//! WF-6 (audit #15): importing/using a `std::finance` function used to blow
//! the compiler's stack at COMPILE time (SIGSEGV / "stack overflow, aborting",
//! exit 134) before any execution. Root cause was an unbounded
//! `apply_substitutions <-> apply_to_annotation` recursion in the type
//! unifier: a cyclic substitution `X |-> Concrete(Object({f: <tyvar X>}))` was
//! stored because the occurs-check treated every `Concrete(_)` annotation as
//! variable-free. See `crates/shape-runtime/src/type_system/unification/
//! unifier.rs` (`occurs_check`, `bind`) and `constraints.rs::occurs_in`.
//!
//! This target exists so the overflow can never silently return: a std::finance
//! program must compile and run to completion.

mod regression;
mod sweep;

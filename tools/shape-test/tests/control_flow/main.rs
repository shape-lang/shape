//! Integration tests for Shape language control flow features.
//!
//! Covers:
//! - If expressions (as values and as statements)
//! - Match expressions (literals, guards, enums, Option/Result)
//! - Loops (for, while, loop, break, continue, ranges)
//! - Functions and return (explicit, implicit, early return, recursion)
//! - Block expressions and return values
//! - Combined / edge-case scenarios

mod blocks;
mod combined;
mod functions;
mod if_else;
mod loops;
mod loops_nested;
mod match_expr;
mod stress_break_continue;
mod stress_for_in;
mod stress_if_basic;
mod stress_if_expressions;
mod stress_if_nested;
mod stress_loop_accumulate;
mod stress_match_basic;
mod stress_nested_loops;
mod stress_while;

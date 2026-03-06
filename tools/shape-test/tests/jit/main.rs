//! Integration tests for the JIT compiler.
//!
//! Shape has a tiered JIT (tier 1 baseline, tier 2 optimized) that
//! compiles hot functions to native code. These tests verify correctness
//! and tiering behavior. Many are TDD since JIT is not directly
//! accessible through the ShapeTest builder.

mod correctness;
mod tiering;

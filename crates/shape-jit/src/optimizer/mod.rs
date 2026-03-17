//! Typed optimization planning for JIT compilation.
//!
//! This module builds a function-level plan from bytecode using static,
//! pattern-based analysis only. It does not rely on benchmark names.

mod bounds;
mod call_path;
mod correctness;
mod cross_function;
pub mod escape_analysis;
mod hof_inline;
pub mod licm;
mod loop_lowering;
mod numeric_arrays;
mod table_queryable;
mod typed_mir;
pub(crate) mod vectorization;

use std::collections::{HashMap, HashSet};

use shape_vm::bytecode::BytecodeProgram;

use crate::translator::loop_analysis::LoopInfo;

pub use bounds::{AffineGuardArraySource, AffineSquareGuard, LinearBoundGuard};
pub use call_path::CallPathPlan;
pub use cross_function::Tier2CacheKey;
pub use escape_analysis::EscapeAnalysisPlan;
pub use hof_inline::{HofInlinePlan, HofInlineSite};
pub use licm::LicmPlan;
pub use loop_lowering::LoopLoweringPlan;
pub use numeric_arrays::NumericArrayPlan;
pub use table_queryable::TableQueryablePlan;
pub use typed_mir::TypedMirFunction;
pub use vectorization::SIMDPlan;

/// Function-level optimization plan consumed by bytecode->IR lowering.
#[derive(Debug, Clone, Default)]
pub struct FunctionOptimizationPlan {
    #[allow(dead_code)]
    /// Phase 1: typed MIR representation of the function bytecode.
    pub typed_mir: TypedMirFunction,
    /// Phase 2/4: loop lowering and nested-loop specialization decisions.
    pub loops: HashMap<usize, LoopLoweringPlan>,
    /// Phase 3: instruction indices where array get/set bounds are statically proven.
    pub trusted_array_get_indices: HashSet<usize>,
    pub trusted_array_set_indices: HashSet<usize>,
    /// Phase 3: instruction indices where index expressions are proven
    /// non-negative in loop context, so negative-index normalization can be skipped.
    pub non_negative_array_get_indices: HashSet<usize>,
    pub non_negative_array_set_indices: HashSet<usize>,
    /// Phase 3: loop-entry guards requiring induction variables to be non-negative.
    pub non_negative_iv_guards_by_loop: HashMap<usize, Vec<u16>>,
    /// Phase 3: loop-entry guards requiring invariant step locals to be non-negative.
    pub non_negative_step_guards_by_loop: HashMap<usize, Vec<u16>>,
    /// Phase 3b: loop-entry guards for `arr[iv]` style indexed access.
    pub linear_bound_guards_by_loop: HashMap<usize, Vec<LinearBoundGuard>>,
    /// Phase 3b: loop-entry guards used for affine `n*n` index kernels.
    pub affine_square_guards_by_loop: HashMap<usize, Vec<AffineSquareGuard>>,
    /// Phase 5: vectorization candidates (strip-mining width keyed by loop header).
    pub vector_width_by_loop: HashMap<usize, u8>,
    /// Phase 5b: SIMD F64X2 lowering plans for eligible typed-data array loops.
    pub simd_plans: HashMap<usize, SIMDPlan>,
    /// Phase 4: typed numeric array access/write opportunities.
    pub numeric_arrays: NumericArrayPlan,
    /// Phase 6: call-path optimization decisions.
    pub call_path: CallPathPlan,
    #[allow(dead_code)]
    /// Phase 7: typed table/queryable lowering opportunities.
    pub table_queryable: TableQueryablePlan,
    /// Phase 8: HOF method inlining opportunities (map/filter/reduce/find/some/every/forEach/findIndex).
    pub hof_inline: HofInlinePlan,
    /// Call LICM: hoistable pure function/method calls per loop.
    pub licm: LicmPlan,
    /// Escape analysis: arrays eligible for scalar replacement (heap elision).
    pub escape_analysis: EscapeAnalysisPlan,
}

/// Build a plan for one function/sub-program.
pub fn build_function_plan(
    program: &BytecodeProgram,
    loop_info: &HashMap<usize, LoopInfo>,
) -> FunctionOptimizationPlan {
    let typed_mir = typed_mir::build_typed_mir(program);
    let loops = loop_lowering::plan_loops(program, loop_info, &typed_mir);
    let bounds = bounds::analyze_bounds(program, loop_info, &loops);
    let numeric_arrays = numeric_arrays::analyze_numeric_arrays(
        program,
        &bounds.trusted_get_indices,
        &bounds.non_negative_get_indices,
        &bounds.trusted_set_indices,
        &bounds.non_negative_set_indices,
    );
    let vector_width_by_loop =
        vectorization::analyze_vectorization(program, loop_info, &loops, &typed_mir);
    let simd_plans = vectorization::analyze_simd(program, loop_info, &loops);
    let call_path = call_path::analyze_call_path(program, &loops);
    let table_queryable = table_queryable::analyze_table_queryable(program);
    let hof_inline = hof_inline::analyze_hof_inline(program);
    let licm = licm::analyze_licm(program, loop_info);
    let escape_analysis = escape_analysis::analyze_escape(program);

    let plan = FunctionOptimizationPlan {
        typed_mir,
        loops,
        trusted_array_get_indices: bounds.trusted_get_indices,
        trusted_array_set_indices: bounds.trusted_set_indices,
        non_negative_array_get_indices: bounds.non_negative_get_indices,
        non_negative_array_set_indices: bounds.non_negative_set_indices,
        non_negative_iv_guards_by_loop: bounds.non_negative_iv_guards_by_loop,
        non_negative_step_guards_by_loop: bounds.non_negative_step_guards_by_loop,
        linear_bound_guards_by_loop: bounds.linear_bound_guards_by_loop,
        affine_square_guards_by_loop: bounds.affine_square_guards_by_loop,
        vector_width_by_loop,
        simd_plans,
        numeric_arrays,
        call_path,
        table_queryable,
        hof_inline,
        licm,
        escape_analysis,
    };

    // Keep invariants explicit even in release builds; this catches accidental
    // unsound plans before codegen starts.
    correctness::validate_plan(program, &plan);
    plan
}

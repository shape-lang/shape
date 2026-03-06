//! Loop unboxing analysis and safety checks.

use std::collections::HashSet;

use shape_vm::bytecode::Operand;

use crate::translator::types::BytecodeToIR;

const READ_SAFETY_LOOKAHEAD: usize = 64;

/// Classification of what precedes writes to a variable in a loop body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteClass {
    /// All writes preceded by typed integer ops (AddInt, SubInt, etc.)
    Integer,
    /// All writes preceded by generic/float ops (Add, Sub, Mul, etc.)
    Float,
    /// All writes preceded by LoadLocal/LoadModuleBinding (copy pattern)
    Copy,
    /// Mix of int and float ops — cannot unbox
    Mixed,
    /// No writes found or invalid pattern
    None,
}

/// Initialization type of a local variable (from constant pool)
#[derive(Debug, Clone, Copy)]
enum InitType {
    Int,
    Float,
}

/// Numeric signal inferred from a generic arithmetic expression.
enum GenericExprSignal {
    Int,
    Float,
    Unknown,
}

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Identify locals and module bindings eligible for unboxing within a loop.
    ///
    /// Returns (int_locals, float_locals, int_module_bindings):
    /// - int_locals: locals that can hold raw i64
    /// - float_locals: locals that can hold raw f64 (float arithmetic only)
    /// - int_module_bindings: module bindings that can hold raw i64
    ///
    /// Includes induction variables, invariant bound variables, and accumulators.
    /// Uses fixed-point iteration so that mutually-dependent swap patterns
    /// (e.g., `t = a + b; a = b; b = t`) are recognized as a group.
    pub(super) fn identify_loop_unbox_vars(
        &self,
        info: &super::super::loop_analysis::LoopInfo,
        allow_float_with_int_state: bool,
    ) -> (HashSet<u16>, HashSet<u16>, HashSet<u16>) {
        use shape_vm::bytecode::OpCode;

        // Safety: loops with function calls can't have unboxed vars because
        // call arguments pass raw i64 where NaN-boxed is expected.
        let unbox_log = std::env::var_os("SHAPE_JIT_UNBOX_LOG").is_some();
        for i in (info.header_idx + 1)..info.end_idx {
            let op = self.program.instructions[i].opcode;
            if matches!(
                op,
                OpCode::Call
                    | OpCode::CallValue
                    | OpCode::CallMethod
                    | OpCode::DynMethodCall
                    | OpCode::IterNext
                    | OpCode::IterDone
            ) {
                if unbox_log {
                    eprintln!(
                        "[shape-jit-unbox-debug] loop_header={} BAIL: {:?} at instr {}",
                        info.header_idx, op, i
                    );
                }
                return (HashSet::new(), HashSet::new(), HashSet::new());
            }
        }

        let is_int_arith = |op: OpCode| {
            matches!(
                op,
                OpCode::AddInt
                    | OpCode::SubInt
                    | OpCode::MulInt
                    | OpCode::DivInt
                    | OpCode::ModInt
                    | OpCode::AddIntTrusted
                    | OpCode::SubIntTrusted
                    | OpCode::MulIntTrusted
                    | OpCode::DivIntTrusted
                    | OpCode::Add
                    | OpCode::Sub
                    | OpCode::Mul
                    | OpCode::Div
                    | OpCode::Mod
                    | OpCode::LtInt
                    | OpCode::GtInt
                    | OpCode::LteInt
                    | OpCode::GteInt
                    | OpCode::Lt
                    | OpCode::Gt
                    | OpCode::Lte
                    | OpCode::Gte
                    | OpCode::EqInt
                    | OpCode::NeqInt
                    | OpCode::GtIntTrusted
                    | OpCode::LtIntTrusted
                    | OpCode::GteIntTrusted
                    | OpCode::LteIntTrusted
                    | OpCode::Eq
                    | OpCode::Neq
            )
        };

        let is_float_arith = |op: OpCode| {
            matches!(
                op,
                OpCode::Add
                    | OpCode::Sub
                    | OpCode::Mul
                    | OpCode::Div
                    | OpCode::Mod
                    | OpCode::AddNumber
                    | OpCode::SubNumber
                    | OpCode::MulNumber
                    | OpCode::DivNumber
                    | OpCode::ModNumber
                    | OpCode::AddNumberTrusted
                    | OpCode::SubNumberTrusted
                    | OpCode::MulNumberTrusted
                    | OpCode::DivNumberTrusted
                    | OpCode::Lt
                    | OpCode::Gt
                    | OpCode::Lte
                    | OpCode::Gte
                    | OpCode::LtNumber
                    | OpCode::GtNumber
                    | OpCode::LteNumber
                    | OpCode::GteNumber
                    | OpCode::EqNumber
                    | OpCode::NeqNumber
                    | OpCode::GtNumberTrusted
                    | OpCode::LtNumberTrusted
                    | OpCode::GteNumberTrusted
                    | OpCode::LteNumberTrusted
                    | OpCode::Eq
                    | OpCode::Neq
            )
        };

        let mut int_locals = HashSet::new();
        let mut float_locals = HashSet::new();
        let mut int_mbs = HashSet::new();

        // Scan for reference-related locals — these must NOT be unboxed
        // because they hold raw pointer values, not NaN-boxed numbers.
        let mut ref_locals: HashSet<u16> = HashSet::new();
        for i in (info.header_idx + 1)..info.end_idx {
            let instr = &self.program.instructions[i];
            match instr.opcode {
                OpCode::MakeRef | OpCode::DerefLoad | OpCode::DerefStore | OpCode::SetIndexRef => {
                    if let Some(Operand::Local(idx)) = &instr.operand {
                        ref_locals.insert(*idx);
                    }
                }
                _ => {}
            }
        }

        // Phase 1: Seed with induction variables (guaranteed integer by loop_analysis)
        for iv in &info.induction_vars {
            if iv.is_module_binding {
                int_mbs.insert(iv.local_slot);
                if let Some(bound) = iv.bound_slot {
                    if info.invariant_module_bindings.contains(&bound) {
                        int_mbs.insert(bound);
                    }
                }
            } else {
                int_locals.insert(iv.local_slot);
                if let Some(bound) = iv.bound_slot {
                    if info.invariant_locals.contains(&bound) {
                        int_locals.insert(bound);
                    }
                }
            }
        }

        // Phase 1b: include read-only invariant integer locals that feed arithmetic
        // in this loop (e.g. outer-loop IVs used in inner-loop index math).
        let no_local_candidates = HashSet::new();
        let unbox_log = std::env::var_os("SHAPE_JIT_UNBOX_LOG").is_some();
        for &local_idx in &info.body_locals_read {
            if info.body_locals_written.contains(&local_idx) {
                continue;
            }
            if int_locals.contains(&local_idx) || ref_locals.contains(&local_idx) {
                continue;
            }
            let init_type = self.local_init_type(info, local_idx);
            if !matches!(init_type, Some(InitType::Int)) {
                if unbox_log {
                    let init_name = match init_type {
                        Some(InitType::Int) => "int",
                        Some(InitType::Float) => "float",
                        None => "unknown",
                    };
                    eprintln!(
                        "[shape-jit-unbox-debug] loop_header={} invariant_local={} init={} reads_safe=false",
                        info.header_idx, local_idx, init_name
                    );
                }
                continue;
            }
            let reads_safe = self.all_reads_safe_local(
                info,
                local_idx,
                &int_locals,
                &no_local_candidates,
                &is_int_arith,
            );
            if unbox_log {
                eprintln!(
                    "[shape-jit-unbox-debug] loop_header={} invariant_local={} init=int reads_safe={}",
                    info.header_idx, local_idx, reads_safe
                );
            }
            if reads_safe {
                int_locals.insert(local_idx);
            }
        }

        // Phase 1c: include read-only invariant float locals (e.g. `cr`, `ci`
        // computed in outer loop, consumed by inner loop's float arithmetic).
        for &local_idx in &info.body_locals_read {
            if info.body_locals_written.contains(&local_idx) {
                continue;
            }
            if int_locals.contains(&local_idx)
                || float_locals.contains(&local_idx)
                || ref_locals.contains(&local_idx)
            {
                continue;
            }
            let init_type = self.local_init_type(info, local_idx);
            if !matches!(init_type, Some(InitType::Float)) {
                continue;
            }
            let reads_safe = self.all_reads_safe_local(
                info,
                local_idx,
                &float_locals,
                &no_local_candidates,
                &is_float_arith,
            );
            if unbox_log {
                eprintln!(
                    "[shape-jit-unbox-debug] loop_header={} invariant_float_local={} reads_safe={}",
                    info.header_idx, local_idx, reads_safe
                );
            }
            if reads_safe {
                float_locals.insert(local_idx);
            }
        }

        // Phase 2: Identify candidate accumulators, classified by write type
        let mut int_local_candidates = HashSet::new();
        let mut float_local_candidates = HashSet::new();
        for &local_idx in &info.body_locals_written {
            if int_locals.contains(&local_idx) || !info.body_locals_read.contains(&local_idx) {
                continue;
            }
            let init_type = self.local_init_type(info, local_idx);
            let is_loop_temp =
                !info.body_can_allocate && self.local_first_access_is_store(info, local_idx);
            // Classify: check what kind of arithmetic precedes writes
            let write_class = self.classify_write_type(info, local_idx, OpCode::StoreLocal);
            if unbox_log {
                eprintln!(
                    "[shape-jit-unbox-debug] loop_header={} local={} write_class={:?} init={:?} first_access_store={}",
                    info.header_idx, local_idx, write_class, init_type, is_loop_temp
                );
            }
            match write_class {
                WriteClass::Integer => {
                    // Allow integer locals initialized inside the loop when
                    // their first loop access is a store (temporary pattern).
                    //
                    // Also allow unknown preheader init: fixed-point read-safety
                    // still rejects locals that flow into non-integer contexts.
                    if matches!(init_type, Some(InitType::Int))
                        || is_loop_temp
                        || init_type.is_none()
                    {
                        int_local_candidates.insert(local_idx);
                    }
                }
                WriteClass::Float => {
                    // Same rule for float temporaries.
                    // Also allow unknown preheader init (like integer candidates):
                    // fixed-point read-safety still rejects locals that flow into
                    // non-float contexts, and the prelude initializes to 0.0.
                    if matches!(init_type, Some(InitType::Float))
                        || is_loop_temp
                        || init_type.is_none()
                    {
                        float_local_candidates.insert(local_idx);
                    }
                }
                WriteClass::Copy => {
                    // Copy is ambiguous (generic ops or pure loads).
                    // Use initialization constant to disambiguate:
                    // - Int constant → integer candidate only
                    // - Number constant → float candidate only
                    // - Unknown/no init → do not unbox (avoids unsound truncation)
                    match init_type {
                        Some(InitType::Float) => {
                            float_local_candidates.insert(local_idx);
                        }
                        Some(InitType::Int) => {
                            int_local_candidates.insert(local_idx);
                        }
                        None => {}
                    }
                }
                WriteClass::Mixed | WriteClass::None => {}
            }
        }

        let mut int_mb_candidates = HashSet::new();
        for &mb_idx in &info.body_module_bindings_written {
            if int_mbs.contains(&mb_idx) || !info.body_module_bindings_read.contains(&mb_idx) {
                continue;
            }
            if self.all_writes_int_or_copy(info, mb_idx, OpCode::StoreModuleBinding, &is_int_arith)
            {
                int_mb_candidates.insert(mb_idx);
            }
        }

        // Remove reference-related locals from all candidate sets
        for &ref_local in &ref_locals {
            int_locals.remove(&ref_local);
            int_local_candidates.remove(&ref_local);
            float_local_candidates.remove(&ref_local);
        }

        // Phase 3: Fixed-point iteration for integer candidates
        loop {
            let mut changed = false;
            for candidate in int_local_candidates.clone() {
                if int_locals.contains(&candidate) {
                    continue;
                }
                if self.all_reads_safe_local(
                    info,
                    candidate,
                    &int_locals,
                    &int_local_candidates,
                    &is_int_arith,
                ) {
                    int_locals.insert(candidate);
                    changed = true;
                }
            }
            for candidate in int_mb_candidates.clone() {
                if int_mbs.contains(&candidate) {
                    continue;
                }
                if self.all_reads_safe_mb(
                    info,
                    candidate,
                    &int_mbs,
                    &int_mb_candidates,
                    &is_int_arith,
                ) {
                    int_mbs.insert(candidate);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        // Phase 4: Fixed-point iteration for float candidates.
        //
        // Mixed int+float loop state is now supported by arithmetic lowering:
        // generic numeric ops use f64 semantics for mixed operands. Keep the
        // sets disjoint (a local cannot be both int and float unboxed).
        let _ = allow_float_with_int_state;
        {
            for &idx in &int_locals {
                float_local_candidates.remove(&idx);
            }
            loop {
                let mut changed = false;
                for candidate in float_local_candidates.clone() {
                    if float_locals.contains(&candidate) {
                        continue;
                    }
                    let reads_safe = self.all_reads_safe_local(
                        info,
                        candidate,
                        &float_locals,
                        &float_local_candidates,
                        &is_float_arith,
                    );
                    if reads_safe {
                        float_locals.insert(candidate);
                        changed = true;
                    }
                }
                if !changed {
                    break;
                }
            }
        }

        // Keep single float accumulators in mixed loops. In numeric kernels this
        // avoids repeated box/unbox churn on loop-carried state.

        (int_locals, float_locals, int_mbs)
    }

    /// Classify the numeric kind of writes to a target variable.
    fn classify_write_type(
        &self,
        info: &super::super::loop_analysis::LoopInfo,
        target_idx: u16,
        store_opcode: shape_vm::bytecode::OpCode,
    ) -> WriteClass {
        use shape_vm::bytecode::OpCode;

        let is_typed_int_op = |op: OpCode| {
            matches!(
                op,
                OpCode::AddInt
                    | OpCode::SubInt
                    | OpCode::MulInt
                    | OpCode::DivInt
                    | OpCode::ModInt
                    | OpCode::AddIntTrusted
                    | OpCode::SubIntTrusted
                    | OpCode::MulIntTrusted
                    | OpCode::DivIntTrusted
            )
        };
        let is_generic_op = |op: OpCode| {
            matches!(
                op,
                OpCode::Add | OpCode::Sub | OpCode::Mul | OpCode::Div | OpCode::Mod
            )
        };
        let is_typed_float_op = |op: OpCode| {
            matches!(
                op,
                OpCode::AddNumber
                    | OpCode::SubNumber
                    | OpCode::MulNumber
                    | OpCode::DivNumber
                    | OpCode::ModNumber
                    | OpCode::AddNumberTrusted
                    | OpCode::SubNumberTrusted
                    | OpCode::MulNumberTrusted
                    | OpCode::DivNumberTrusted
            )
        };
        let is_comparison = |op: OpCode| {
            matches!(
                op,
                OpCode::Lt
                    | OpCode::Gt
                    | OpCode::Lte
                    | OpCode::Gte
                    | OpCode::LtInt
                    | OpCode::GtInt
                    | OpCode::LteInt
                    | OpCode::GteInt
                    | OpCode::EqInt
                    | OpCode::NeqInt
                    | OpCode::EqNumber
                    | OpCode::NeqNumber
                    | OpCode::Eq
                    | OpCode::Neq
                    | OpCode::GtIntTrusted
                    | OpCode::LtIntTrusted
                    | OpCode::GteIntTrusted
                    | OpCode::LteIntTrusted
                    | OpCode::GtNumberTrusted
                    | OpCode::LtNumberTrusted
                    | OpCode::GteNumberTrusted
                    | OpCode::LteNumberTrusted
            )
        };

        let mut has_writes = false;
        let mut has_int = false; // typed int ops or generic ops with int-only evidence
        let mut has_typed_float = false; // typed float ops or generic ops with float evidence
        let mut has_generic = false; // generic ops without enough evidence
        let mut all_copies = true;

        for i in (info.header_idx + 1)..info.end_idx {
            let instr = &self.program.instructions[i];
            if instr.opcode != store_opcode {
                continue;
            }
            let matches = match (&instr.operand, store_opcode) {
                (Some(Operand::Local(idx)), OpCode::StoreLocal) => *idx == target_idx,
                (Some(Operand::ModuleBinding(idx)), OpCode::StoreModuleBinding) => {
                    *idx == target_idx
                }
                _ => false,
            };
            if !matches {
                continue;
            }
            has_writes = true;
            if i <= info.header_idx + 1 {
                return WriteClass::None; // Can't look back
            }
            let prev = &self.program.instructions[i - 1];
            if is_typed_int_op(prev.opcode) {
                has_int = true;
                all_copies = false;
            } else if is_generic_op(prev.opcode) {
                // Generic ops are ambiguous; infer from nearby expression inputs.
                match self.generic_expr_signal(info, i) {
                    GenericExprSignal::Float => has_typed_float = true,
                    GenericExprSignal::Int => has_int = true,
                    GenericExprSignal::Unknown => has_generic = true,
                }
                all_copies = false;
            } else if is_typed_float_op(prev.opcode) {
                has_typed_float = true;
                all_copies = false;
            } else if is_comparison(prev.opcode) {
                all_copies = false;
            } else if matches!(prev.opcode, OpCode::LoadLocal | OpCode::LoadModuleBinding) {
            } else {
                return WriteClass::None;
            }
        }

        if !has_writes {
            return WriteClass::None;
        }
        if all_copies
            || has_generic
            || (has_int && has_typed_float)
            || (!has_int && !has_typed_float)
        {
            return WriteClass::Copy;
        }
        if has_int {
            return WriteClass::Integer;
        }
        if has_typed_float {
            return WriteClass::Float;
        }
        WriteClass::Mixed
    }

    /// Infer numeric signal for generic arithmetic ending at `store_idx`.
    fn generic_expr_signal(
        &self,
        info: &super::super::loop_analysis::LoopInfo,
        store_idx: usize,
    ) -> GenericExprSignal {
        use shape_vm::bytecode::{Constant, OpCode, Operand};

        let start = (store_idx.saturating_sub(48)).max(info.header_idx + 1);
        let mut saw_int = false;
        let mut saw_float = false;

        for j in (start..store_idx).rev() {
            let instr = &self.program.instructions[j];
            match instr.opcode {
                OpCode::PushConst => {
                    if let Some(Operand::Const(const_idx)) = &instr.operand {
                        match self.program.constants.get(*const_idx as usize) {
                            Some(Constant::Int(_)) | Some(Constant::UInt(_)) => saw_int = true,
                            Some(Constant::Number(_)) => saw_float = true,
                            _ => {}
                        }
                    }
                }
                OpCode::LoadLocal => {
                    if let Some(Operand::Local(local_idx)) = &instr.operand {
                        match self.local_init_type(info, *local_idx) {
                            Some(InitType::Int) => saw_int = true,
                            Some(InitType::Float) => saw_float = true,
                            None => {}
                        }
                    }
                }
                OpCode::Add
                | OpCode::Sub
                | OpCode::Mul
                | OpCode::Div
                | OpCode::Mod
                | OpCode::AddInt
                | OpCode::SubInt
                | OpCode::MulInt
                | OpCode::DivInt
                | OpCode::ModInt
                | OpCode::AddIntTrusted
                | OpCode::SubIntTrusted
                | OpCode::MulIntTrusted
                | OpCode::DivIntTrusted
                | OpCode::AddNumber
                | OpCode::SubNumber
                | OpCode::MulNumber
                | OpCode::DivNumber
                | OpCode::ModNumber
                | OpCode::AddNumberTrusted
                | OpCode::SubNumberTrusted
                | OpCode::MulNumberTrusted
                | OpCode::DivNumberTrusted
                | OpCode::LoadModuleBinding
                | OpCode::Dup
                | OpCode::Swap => {}
                _ => break,
            }
        }

        if saw_float {
            GenericExprSignal::Float
        } else if saw_int {
            GenericExprSignal::Int
        } else {
            GenericExprSignal::Unknown
        }
    }

    /// Infer numeric signal from nearby constants for a generic expression.
    ///
    /// Unlike `generic_expr_signal`, this does not consult local initialization
    /// and is safe to use from initialization inference paths.
    fn generic_const_signal(&self, floor_idx: usize, store_idx: usize) -> GenericExprSignal {
        use shape_vm::bytecode::{Constant, OpCode, Operand};

        let start = store_idx.saturating_sub(48).max(floor_idx);
        let mut saw_int = false;
        let mut saw_float = false;
        for j in (start..store_idx).rev() {
            let instr = &self.program.instructions[j];
            match instr.opcode {
                OpCode::PushConst => {
                    if let Some(Operand::Const(const_idx)) = &instr.operand {
                        match self.program.constants.get(*const_idx as usize) {
                            Some(Constant::Int(_)) | Some(Constant::UInt(_)) => saw_int = true,
                            Some(Constant::Number(_)) => saw_float = true,
                            _ => {}
                        }
                    }
                }
                OpCode::Add
                | OpCode::Sub
                | OpCode::Mul
                | OpCode::Div
                | OpCode::Mod
                | OpCode::AddInt
                | OpCode::SubInt
                | OpCode::MulInt
                | OpCode::DivInt
                | OpCode::ModInt
                | OpCode::AddIntTrusted
                | OpCode::SubIntTrusted
                | OpCode::MulIntTrusted
                | OpCode::DivIntTrusted
                | OpCode::AddNumber
                | OpCode::SubNumber
                | OpCode::MulNumber
                | OpCode::DivNumber
                | OpCode::ModNumber
                | OpCode::AddNumberTrusted
                | OpCode::SubNumberTrusted
                | OpCode::MulNumberTrusted
                | OpCode::DivNumberTrusted
                | OpCode::LoadLocal
                | OpCode::LoadModuleBinding
                | OpCode::IntToNumber
                | OpCode::NumberToInt
                | OpCode::Dup
                | OpCode::Swap => {}
                _ => break,
            }
        }
        if saw_float {
            GenericExprSignal::Float
        } else if saw_int {
            GenericExprSignal::Int
        } else {
            GenericExprSignal::Unknown
        }
    }

    /// Determine initialization type by scanning stores before loop entry.
    fn local_init_type(
        &self,
        info: &super::super::loop_analysis::LoopInfo,
        local_idx: u16,
    ) -> Option<InitType> {
        use shape_vm::bytecode::{Constant, OpCode, Operand};

        // Numeric parameter hints come from bytecode-side usage analysis.
        // We treat hinted params as integer-initialized for loop unboxing;
        // read-safety checks still gate actual promotion.
        if self.numeric_param_hints.contains(&local_idx) {
            return Some(InitType::Int);
        }

        // Scan backwards so we use the nearest pre-header initialization.
        // Forward scans can pick stale writes from earlier scopes.
        for i in (0..info.header_idx).rev() {
            let instr = &self.program.instructions[i];
            let is_store = match (instr.opcode, &instr.operand) {
                (OpCode::StoreLocal, Some(Operand::Local(idx))) if *idx == local_idx => true,
                (OpCode::StoreLocalTyped, Some(Operand::TypedLocal(idx, _)))
                    if *idx == local_idx =>
                {
                    true
                }
                _ => false,
            };
            if !is_store {
                continue;
            }
            if i > 0 {
                let prev = &self.program.instructions[i - 1];
                match prev.opcode {
                    OpCode::PushConst => {
                        if let Some(Operand::Const(const_idx)) = &prev.operand {
                            if let Some(constant) = self.program.constants.get(*const_idx as usize)
                            {
                                return match constant {
                                    Constant::Int(_) | Constant::UInt(_) => Some(InitType::Int),
                                    Constant::Number(_) => Some(InitType::Float),
                                    _ => None,
                                };
                            }
                        }
                    }
                    OpCode::AddInt
                    | OpCode::SubInt
                    | OpCode::MulInt
                    | OpCode::DivInt
                    | OpCode::ModInt
                    | OpCode::AddIntTrusted
                    | OpCode::SubIntTrusted
                    | OpCode::MulIntTrusted
                    | OpCode::DivIntTrusted => return Some(InitType::Int),
                    OpCode::AddNumber
                    | OpCode::SubNumber
                    | OpCode::MulNumber
                    | OpCode::DivNumber
                    | OpCode::ModNumber
                    | OpCode::AddNumberTrusted
                    | OpCode::SubNumberTrusted
                    | OpCode::MulNumberTrusted
                    | OpCode::DivNumberTrusted => return Some(InitType::Float),
                    OpCode::Add | OpCode::Sub | OpCode::Mul | OpCode::Div | OpCode::Mod => {
                        return match self.generic_const_signal(0, i) {
                            GenericExprSignal::Float => Some(InitType::Float),
                            GenericExprSignal::Int => Some(InitType::Int),
                            GenericExprSignal::Unknown => None,
                        };
                    }
                    _ => {}
                }
            }
        }
        None
    }

    /// Returns true if the first loop access to `local_idx` is a store.
    fn local_first_access_is_store(
        &self,
        info: &super::super::loop_analysis::LoopInfo,
        local_idx: u16,
    ) -> bool {
        use shape_vm::bytecode::{OpCode, Operand};

        for i in (info.header_idx + 1)..info.end_idx {
            let instr = &self.program.instructions[i];
            match instr.opcode {
                OpCode::LoadLocal => {
                    if matches!(&instr.operand, Some(Operand::Local(idx)) if *idx == local_idx) {
                        return false;
                    }
                }
                OpCode::StoreLocal => {
                    if matches!(&instr.operand, Some(Operand::Local(idx)) if *idx == local_idx) {
                        return true;
                    }
                }
                OpCode::StoreLocalTyped => {
                    if matches!(&instr.operand, Some(Operand::TypedLocal(idx, _)) if *idx == local_idx)
                    {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// Return true when all writes are arithmetic or copy-style.
    fn all_writes_int_or_copy<F>(
        &self,
        info: &super::super::loop_analysis::LoopInfo,
        target_idx: u16,
        store_opcode: shape_vm::bytecode::OpCode,
        is_arith: &F,
    ) -> bool
    where
        F: Fn(shape_vm::bytecode::OpCode) -> bool,
    {
        use shape_vm::bytecode::OpCode;

        let mut has_writes = false;
        for i in (info.header_idx + 1)..info.end_idx {
            let instr = &self.program.instructions[i];
            if instr.opcode != store_opcode {
                continue;
            }
            let matches = match (&instr.operand, store_opcode) {
                (Some(Operand::Local(idx)), OpCode::StoreLocal) => *idx == target_idx,
                (Some(Operand::ModuleBinding(idx)), OpCode::StoreModuleBinding) => {
                    *idx == target_idx
                }
                _ => false,
            };
            if !matches {
                continue;
            }
            has_writes = true;
            if i > info.header_idx + 1 {
                let prev = &self.program.instructions[i - 1];
                if !is_arith(prev.opcode)
                    && !matches!(prev.opcode, OpCode::LoadLocal | OpCode::LoadModuleBinding)
                {
                    return false;
                }
            } else {
                return false;
            }
        }
        has_writes
    }

    /// Return true when all reads flow into safe numeric/copy uses.
    fn all_reads_safe_local<F>(
        &self,
        info: &super::super::loop_analysis::LoopInfo,
        target: u16,
        confirmed: &HashSet<u16>,
        candidates: &HashSet<u16>,
        is_arith: &F,
    ) -> bool
    where
        F: Fn(shape_vm::bytecode::OpCode) -> bool,
    {
        use shape_vm::bytecode::OpCode;
        let unbox_log = std::env::var_os("SHAPE_JIT_UNBOX_LOG").is_some();

        for i in (info.header_idx + 1)..info.end_idx {
            let instr = &self.program.instructions[i];
            if instr.opcode != OpCode::LoadLocal {
                continue;
            }
            if !matches!(&instr.operand, Some(Operand::Local(idx)) if *idx == target) {
                continue;
            }
            let mut found_safe = false;
            for j in (i + 1)..info.end_idx.min(i + READ_SAFETY_LOOKAHEAD) {
                let next = &self.program.instructions[j];
                if is_arith(next.opcode) {
                    found_safe = true;
                    break;
                }
                // Index/collection ops that consume a numeric index are safe.
                // Allow a small window (not just immediate-next) because the
                // bytecode often pushes the value and a temp store between the
                // index load and the actual set/get instruction.
                if matches!(
                    next.opcode,
                    OpCode::GetProp
                        | OpCode::SetIndexRef
                        | OpCode::SetLocalIndex
                        | OpCode::SetModuleBindingIndex
                ) {
                    found_safe = true;
                    break;
                }
                if next.opcode == OpCode::StoreLocal {
                    if let Some(Operand::Local(store_target)) = &next.operand {
                        if confirmed.contains(store_target) || candidates.contains(store_target) {
                            found_safe = true;
                            break;
                        }
                        // StoreLocal to a non-target, non-candidate local is
                        // pass-through: it pops a different value from the stack
                        // (pushed after our tracked local), not our tracked value.
                        if *store_target != target {
                            continue;
                        }
                    }
                    break;
                }
                if next.opcode == OpCode::StoreLocalTyped {
                    if let Some(Operand::TypedLocal(store_target, _)) = &next.operand {
                        if confirmed.contains(store_target) || candidates.contains(store_target) {
                            found_safe = true;
                            break;
                        }
                        if *store_target != target {
                            continue;
                        }
                    }
                    break;
                }
                if matches!(next.opcode, OpCode::DropCall | OpCode::DropCallAsync) {
                    found_safe = true;
                    break;
                }
                // IntToNumber is a safe terminal for integer locals: the JIT's
                // compile_int_to_number handles raw i64 via fcvt_from_sint.
                // NumberToInt is safe for float locals similarly.
                if next.opcode == OpCode::IntToNumber || next.opcode == OpCode::NumberToInt {
                    found_safe = true;
                    break;
                }
                if matches!(
                    next.opcode,
                    OpCode::LoadLocal
                        | OpCode::LoadModuleBinding
                        | OpCode::PushConst
                        | OpCode::AddInt
                        | OpCode::SubInt
                        | OpCode::MulInt
                        | OpCode::DivInt
                        | OpCode::ModInt
                        | OpCode::AddIntTrusted
                        | OpCode::SubIntTrusted
                        | OpCode::MulIntTrusted
                        | OpCode::DivIntTrusted
                        | OpCode::AddNumber
                        | OpCode::SubNumber
                        | OpCode::MulNumber
                        | OpCode::DivNumber
                        | OpCode::ModNumber
                        | OpCode::AddNumberTrusted
                        | OpCode::SubNumberTrusted
                        | OpCode::MulNumberTrusted
                        | OpCode::DivNumberTrusted
                        | OpCode::LtIntTrusted
                        | OpCode::GtIntTrusted
                        | OpCode::LteIntTrusted
                        | OpCode::GteIntTrusted
                        | OpCode::LtNumber
                        | OpCode::GtNumber
                        | OpCode::LteNumber
                        | OpCode::GteNumber
                        | OpCode::LtNumberTrusted
                        | OpCode::GtNumberTrusted
                        | OpCode::LteNumberTrusted
                        | OpCode::GteNumberTrusted
                        | OpCode::GetProp
                        | OpCode::DerefLoad
                        | OpCode::Length
                        | OpCode::Dup
                        | OpCode::Swap
                ) {
                    continue;
                }
                break;
            }
            if !found_safe {
                if unbox_log {
                    let mut window = Vec::new();
                    for j in (i + 1)..info.end_idx.min(i + 6) {
                        window.push(format!("{j}:{:?}", self.program.instructions[j]));
                    }
                    eprintln!(
                        "[shape-jit-unbox-debug] loop_header={} local={} read_instr={} unsafe_read_at={} follow=[{}]",
                        info.header_idx,
                        target,
                        format!("{:?}", self.program.instructions[i]),
                        i,
                        window.join(",")
                    );
                }
                return false;
            }
        }
        true
    }

    /// Return true when all module-binding reads flow into safe uses.
    fn all_reads_safe_mb<F>(
        &self,
        info: &super::super::loop_analysis::LoopInfo,
        target: u16,
        confirmed: &HashSet<u16>,
        candidates: &HashSet<u16>,
        is_arith: &F,
    ) -> bool
    where
        F: Fn(shape_vm::bytecode::OpCode) -> bool,
    {
        use shape_vm::bytecode::OpCode;

        for i in (info.header_idx + 1)..info.end_idx {
            let instr = &self.program.instructions[i];
            if instr.opcode != OpCode::LoadModuleBinding {
                continue;
            }
            if !matches!(&instr.operand, Some(Operand::ModuleBinding(idx)) if *idx == target) {
                continue;
            }
            let mut found_safe = false;
            for j in (i + 1)..info.end_idx.min(i + READ_SAFETY_LOOKAHEAD) {
                let next = &self.program.instructions[j];
                if is_arith(next.opcode) {
                    found_safe = true;
                    break;
                }
                if matches!(
                    next.opcode,
                    OpCode::GetProp
                        | OpCode::SetIndexRef
                        | OpCode::SetLocalIndex
                        | OpCode::SetModuleBindingIndex
                ) {
                    found_safe = true;
                    break;
                }
                if next.opcode == OpCode::StoreLocal {
                    // Pass-through: StoreLocal in MB context stores a different
                    // stack value, not our tracked module binding.
                    continue;
                }
                if next.opcode == OpCode::StoreLocalTyped {
                    continue;
                }
                if next.opcode == OpCode::StoreModuleBinding {
                    if let Some(Operand::ModuleBinding(store_target)) = &next.operand {
                        if confirmed.contains(store_target) || candidates.contains(store_target) {
                            found_safe = true;
                            break;
                        }
                        if *store_target != target {
                            continue;
                        }
                    }
                    break;
                }
                // IntToNumber/NumberToInt are safe terminals (see all_reads_safe_local).
                if next.opcode == OpCode::IntToNumber || next.opcode == OpCode::NumberToInt {
                    found_safe = true;
                    break;
                }
                if matches!(
                    next.opcode,
                    OpCode::LoadLocal
                        | OpCode::LoadModuleBinding
                        | OpCode::PushConst
                        | OpCode::AddInt
                        | OpCode::SubInt
                        | OpCode::MulInt
                        | OpCode::DivInt
                        | OpCode::ModInt
                        | OpCode::AddIntTrusted
                        | OpCode::SubIntTrusted
                        | OpCode::MulIntTrusted
                        | OpCode::DivIntTrusted
                        | OpCode::AddNumber
                        | OpCode::SubNumber
                        | OpCode::MulNumber
                        | OpCode::DivNumber
                        | OpCode::ModNumber
                        | OpCode::AddNumberTrusted
                        | OpCode::SubNumberTrusted
                        | OpCode::MulNumberTrusted
                        | OpCode::DivNumberTrusted
                        | OpCode::LtIntTrusted
                        | OpCode::GtIntTrusted
                        | OpCode::LteIntTrusted
                        | OpCode::GteIntTrusted
                        | OpCode::LtNumber
                        | OpCode::GtNumber
                        | OpCode::LteNumber
                        | OpCode::GteNumber
                        | OpCode::LtNumberTrusted
                        | OpCode::GtNumberTrusted
                        | OpCode::LteNumberTrusted
                        | OpCode::GteNumberTrusted
                        | OpCode::GetProp
                        | OpCode::DerefLoad
                        | OpCode::Length
                        | OpCode::Dup
                        | OpCode::Swap
                ) {
                    continue;
                }
                break;
            }
            if !found_safe {
                return false;
            }
        }
        true
    }
}

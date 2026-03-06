//! On-Stack Replacement (OSR) and deoptimization support for the VM executor.
//!
//! OSR allows the VM to transfer execution from the bytecode interpreter into
//! JIT-compiled native code mid-function, specifically at hot loop headers.
//! Deoptimization is the reverse: when JIT-compiled code encounters an
//! unexpected condition (type guard failure, etc.), it returns a sentinel value
//! and the VM reconstructs the interpreter frame from JIT state.
//!
//! # Flow
//!
//! 1. The dispatch loop increments a per-loop back-edge counter in the
//!    `TierManager` each time a `LoopStart` (with operand) or backward `Jump`
//!    is executed.
//! 2. When the counter exceeds the OSR threshold, a `CompilationRequest` with
//!    `osr: true` is sent to the background JIT thread.
//! 3. When the JIT completes, `poll_completions()` installs the native code
//!    pointer in the `TierManager::osr_table`.
//! 4. On subsequent loop iterations, `try_osr_entry()` checks for available
//!    OSR code, snapshots live locals into a JIT context buffer, and calls the
//!    native function.
//! 5. If the JIT returns `u64::MAX` (deopt sentinel), `restore_frame_from_osr()`
//!    or `deopt_with_info()` reconstructs interpreter state from the JIT context.

#[cfg(feature = "jit")]
use crate::bytecode::{DeoptInfo, OsrEntryPoint};
#[cfg(feature = "jit")]
use crate::executor::control_flow::jit_abi;
#[cfg(feature = "jit")]
use crate::type_tracking::SlotKind;
#[cfg(feature = "jit")]
use shape_value::VMError;

/// JIT context buffer size in u64 words.
///
/// Must match the `JITContext` layout from `shape-jit/src/context.rs`:
///   - locals:    byte 64  (u64 index 8)
///   - stack:     byte 576 (u64 index 72)
///   - stack_ptr: byte 1600 (u64 index 200)
///   - total:     ~1728 bytes = 216 u64s
#[cfg(feature = "jit")]
const CTX_U64_SIZE: usize = 216;

/// Offset (in u64 words) where locals begin in the JIT context buffer.
#[cfg(feature = "jit")]
const LOCALS_U64_OFFSET: usize = 8;

#[cfg(feature = "jit")]
impl VirtualMachine {
    /// Attempt OSR entry for a hot loop.
    ///
    /// Called when a loop back-edge is reached and the `TierManager` indicates
    /// OSR code may be available. Snapshots live locals from the interpreter
    /// frame into a JIT context buffer, invokes the OSR-compiled loop body,
    /// and handles the result:
    ///
    /// - **Success** (`result != u64::MAX`): reads modified locals back from
    ///   the JIT context, sets IP to the loop exit, returns `Ok(true)`.
    /// - **Deopt** (`result == u64::MAX`): restores interpreter locals from
    ///   the JIT context (which may have been partially modified), returns
    ///   `Ok(false)` so the interpreter resumes at the current position.
    /// - **No OSR code available**: returns `Ok(false)` immediately.
    pub(crate) fn try_osr_entry(&mut self, func_id: u16, loop_ip: usize) -> Result<bool, VMError> {
        // Look up OSR native code pointer
        let osr_fn = match self
            .tier_manager
            .as_ref()
            .and_then(|mgr| mgr.get_osr_code(func_id, loop_ip))
        {
            Some(ptr) => ptr,
            None => return Ok(false),
        };

        // Look up the OSR entry point metadata from the function
        let osr_entry = {
            let func = self
                .program
                .functions
                .get(func_id as usize)
                .ok_or(VMError::InvalidCall)?;
            func.osr_entry_points
                .iter()
                .find(|e| e.bytecode_ip == loop_ip)
                .cloned()
        };
        let osr_entry = match osr_entry {
            Some(e) => e,
            None => return Ok(false),
        };

        // Get the current frame's base pointer
        let base = self
            .call_stack
            .last()
            .ok_or(VMError::StackUnderflow)?
            .base_pointer;

        // Bail out if any live local exceeds JIT locals capacity.
        // The JIT compiler rejects these loops at compile time, but this
        // guard catches any stale OSR entry metadata that slips through.
        for &local_idx in &osr_entry.live_locals {
            if local_idx as usize >= 64 {
                return Ok(false);
            }
        }

        // Build a JIT context buffer and snapshot live locals into it.
        // Uses local_idx (not sequential i) as the buffer index so the JIT
        // code can access locals by their original variable index.
        let mut ctx_buf = [0u64; CTX_U64_SIZE];
        for (i, &local_idx) in osr_entry.live_locals.iter().enumerate() {
            let kind = osr_entry
                .local_kinds
                .get(i)
                .copied()
                .unwrap_or(SlotKind::Unknown);
            let slot_idx = base + local_idx as usize;
            if slot_idx < self.stack.len() {
                ctx_buf[LOCALS_U64_OFFSET + local_idx as usize] =
                    jit_abi::marshal_arg_to_jit(&self.stack[slot_idx], kind);
            }
        }

        // Invoke the OSR-compiled loop body
        let jit_fn: unsafe extern "C" fn(*mut u8, *const u8) -> u64 =
            unsafe { std::mem::transmute(osr_fn) };
        let result_bits = unsafe { jit_fn(ctx_buf.as_mut_ptr() as *mut u8, std::ptr::null()) };

        if result_bits == u64::MAX {
            // Deopt: restore locals from JIT context back to interpreter frame.
            // The JIT may have partially modified locals before bailing out.
            self.restore_frame_from_osr(&osr_entry, &ctx_buf, base);
            // Return false so the interpreter resumes at the current IP
            // (re-executes the loop iteration in interpreted mode).
            return Ok(false);
        }

        // Success: read modified locals back from JIT context.
        // All locals should be < 64 (validated at entry above).
        for (i, &local_idx) in osr_entry.live_locals.iter().enumerate() {
            debug_assert!(
                (local_idx as usize) < 64,
                "OSR local {} exceeds capacity after successful JIT execution",
                local_idx
            );
            let kind = osr_entry
                .local_kinds
                .get(i)
                .copied()
                .unwrap_or(SlotKind::Unknown);
            let slot_idx = base + local_idx as usize;
            if slot_idx < self.stack.len() {
                self.stack[slot_idx] = jit_abi::unmarshal_jit_result(
                    ctx_buf[LOCALS_U64_OFFSET + local_idx as usize],
                    kind,
                );
            }
        }

        // Jump to the loop exit — the JIT has completed the loop body
        self.ip = osr_entry.exit_ip;
        Ok(true)
    }

    /// Restore interpreter frame locals from a JIT context buffer after an
    /// OSR deoptimization.
    ///
    /// The JIT may have modified some locals before encountering a deopt point.
    /// This method writes the (potentially modified) values back so the
    /// interpreter sees a consistent state.
    fn restore_frame_from_osr(
        &mut self,
        osr_entry: &OsrEntryPoint,
        ctx_buf: &[u64; CTX_U64_SIZE],
        base: usize,
    ) {
        for (i, &local_idx) in osr_entry.live_locals.iter().enumerate() {
            debug_assert!(
                (local_idx as usize) < 64,
                "OSR deopt: local {} exceeds JIT locals capacity",
                local_idx
            );
            if local_idx as usize >= 64 {
                continue; // Defensive: should never happen (JIT rejects at compile time)
            }
            let kind = osr_entry
                .local_kinds
                .get(i)
                .copied()
                .unwrap_or(SlotKind::Unknown);
            let slot_idx = base + local_idx as usize;
            if slot_idx < self.stack.len() {
                self.stack[slot_idx] = jit_abi::unmarshal_jit_result(
                    ctx_buf[LOCALS_U64_OFFSET + local_idx as usize],
                    kind,
                );
            }
        }
    }

    /// Reconstruct interpreter frame from JIT state using a `DeoptInfo`.
    ///
    /// Unlike `restore_frame_from_osr` (which uses `OsrEntryPoint`),
    /// this provides a general mapping from JIT local indices to bytecode
    /// local indices — supporting deopt at arbitrary points.
    ///
    /// **Prerequisite:** A callee `CallFrame` must already exist on
    /// `call_stack` (this method reads `call_stack.last()`). The Tier 2
    /// whole-function deopt handler in `control_flow/mod.rs` pushes a
    /// synthetic callee frame before calling this method when the
    /// `DeoptInfo` has non-empty `local_mapping` (precise deopt).
    ///
    /// After calling this, the VM's `ip` is set to `deopt_info.resume_ip`
    /// and `sp` is adjusted per `deopt_info.stack_depth`.
    pub(crate) fn deopt_with_info(
        &mut self,
        deopt_info: &DeoptInfo,
        ctx_buf: &[u64; CTX_U64_SIZE],
    ) -> Result<(), VMError> {
        let frame = self.call_stack.last().ok_or(VMError::StackUnderflow)?;
        let base = frame.base_pointer;
        let locals_count = frame.locals_count;

        debug_assert!(
            deopt_info.local_kinds.len() == deopt_info.local_mapping.len(),
            "DeoptInfo: local_kinds len {} != local_mapping len {}",
            deopt_info.local_kinds.len(),
            deopt_info.local_mapping.len()
        );

        // Restore locals using the JIT-to-bytecode index mapping
        for (i, &(jit_idx, bc_idx)) in deopt_info.local_mapping.iter().enumerate() {
            let kind = deopt_info
                .local_kinds
                .get(i)
                .copied()
                .unwrap_or(SlotKind::Unknown);
            let src_idx = LOCALS_U64_OFFSET + jit_idx as usize;
            let dst_idx = base + bc_idx as usize;
            if src_idx < CTX_U64_SIZE && dst_idx < self.stack.len() {
                self.stack[dst_idx] = jit_abi::unmarshal_jit_result(ctx_buf[src_idx], kind);
            }
        }

        // Set interpreter IP to the deopt resume point
        self.ip = deopt_info.resume_ip;

        // Restore stack depth: locals region + operand stack depth from deopt info
        self.sp = base + locals_count + deopt_info.stack_depth as usize;

        Ok(())
    }

    /// Reconstruct interpreter state for multi-frame inline deopt.
    ///
    /// When a guard fails inside inlined code, the JIT has a single physical
    /// function but the interpreter needs a full call stack. This method:
    /// 1. Pushes synthetic CallFrames for all caller frames (outermost first)
    /// 2. Restores locals for each frame from the ctx_buf
    /// 3. Pushes the innermost (inlined callee) frame
    /// 4. Restores the innermost frame's locals from the main local_mapping
    /// 5. Sets ip = deopt_info.resume_ip (interpreter resumes in innermost)
    ///
    /// The interpreter will naturally return up through the reconstructed stack.
    pub(crate) fn deopt_with_inline_frames(
        &mut self,
        deopt_info: &DeoptInfo,
        ctx_buf: &[u64; CTX_U64_SIZE],
        outer_func_id: u16,
        outer_bp: usize,
    ) -> Result<(), VMError> {
        // inline_frames is outermost-first:
        //   [0] = outermost physical function (its locals at the call site)
        //   [last] = immediate caller of the innermost inlined callee
        //
        // We iterate forward, pushing CallFrames from outermost to innermost,
        // matching the interpreter's call stack ordering.
        let frames = &deopt_info.inline_frames;

        debug_assert!(
            frames.first().map(|f| f.function_id) == Some(outer_func_id),
            "inline_frames[0].function_id ({:?}) should match outer_func_id ({})",
            frames.first().map(|f| f.function_id),
            outer_func_id
        );

        let mut current_bp = outer_bp;

        for (i, iframe) in frames.iter().enumerate() {
            let func = self
                .program
                .functions
                .get(iframe.function_id as usize)
                .ok_or(VMError::InvalidCall)?;
            let locals_count = func.locals_count as usize;
            let needed = current_bp + locals_count + iframe.stack_depth as usize;
            if needed > self.stack.len() {
                self.stack.resize_with(needed * 2 + 1, ValueWord::none);
            }
            // Zero-init local slots
            for j in 0..locals_count {
                self.stack[current_bp + j] = ValueWord::none();
            }

            // Restore locals from ctx_buf using the frame's mapping
            for (j, &(ctx_pos, bc_idx)) in iframe.local_mapping.iter().enumerate() {
                let kind = iframe
                    .local_kinds
                    .get(j)
                    .copied()
                    .unwrap_or(SlotKind::NanBoxed);
                let src_idx = LOCALS_U64_OFFSET + ctx_pos as usize;
                let dst_idx = current_bp + bc_idx as usize;
                if src_idx < CTX_U64_SIZE && dst_idx < self.stack.len() {
                    self.stack[dst_idx] = jit_abi::unmarshal_jit_result(ctx_buf[src_idx], kind);
                }
            }

            // return_ip: where the interpreter resumes when this frame returns.
            //
            // The dispatch loop stores return_ip = self.ip AFTER bumping it past
            // the Call instruction (ip += 1 happens before dispatch). So return_ip
            // = call_instruction_ip + 1.
            //
            // For the outermost frame (i=0): self.ip is already bumped to the
            // instruction after the CallValue that entered the JIT function.
            //
            // For intermediate frames: the previous frame's resume_ip is the Call
            // instruction IP in that frame. We add 1 to get the instruction after.
            let return_ip = if i == 0 {
                self.ip
            } else {
                frames[i - 1].resume_ip + 1
            };

            let blob_hash = self.blob_hash_for_function(iframe.function_id);
            self.call_stack.push(super::super::CallFrame {
                return_ip,
                base_pointer: current_bp,
                locals_count,
                function_id: Some(iframe.function_id),
                upvalues: None,
                blob_hash,
            });

            current_bp += locals_count + iframe.stack_depth as usize;
        }

        // Push the innermost (inlined callee) frame.
        let innermost_id = deopt_info
            .innermost_function_id
            .ok_or(VMError::InvalidCall)?;
        let innermost_func = self
            .program
            .functions
            .get(innermost_id as usize)
            .ok_or(VMError::InvalidCall)?;
        let innermost_locals = innermost_func.locals_count as usize;
        let needed = current_bp + innermost_locals + deopt_info.stack_depth as usize;
        if needed > self.stack.len() {
            self.stack.resize_with(needed * 2 + 1, ValueWord::none);
        }
        for i in 0..innermost_locals {
            self.stack[current_bp + i] = ValueWord::none();
        }

        // return_ip for innermost: instruction after the last inline frame's call
        let innermost_return_ip = frames
            .last()
            .map(|f| f.resume_ip + 1)
            .unwrap_or(self.ip);

        let blob_hash = self.blob_hash_for_function(innermost_id);
        self.call_stack.push(super::super::CallFrame {
            return_ip: innermost_return_ip,
            base_pointer: current_bp,
            locals_count: innermost_locals,
            function_id: Some(innermost_id),
            upvalues: None,
            blob_hash,
        });

        // Restore innermost frame's locals + operand stack via deopt_with_info.
        // deopt_with_info reads call_stack.last() for base_pointer, which is the
        // innermost frame we just pushed.
        self.deopt_with_info(deopt_info, ctx_buf)?;

        Ok(())
    }

    /// Handle deoptimization from a Tier 2 whole-function JIT execution.
    ///
    /// **Note:** The Tier 2 deopt sentinel handler in `control_flow/mod.rs`
    /// now handles precise mid-function deopt directly (pushing a synthetic
    /// callee frame and calling `deopt_with_info()`). This method is
    /// retained as a utility for code paths that already have a callee
    /// frame on the call stack (e.g., OSR deopt scenarios).
    ///
    /// # Returns
    /// `Ok(true)` if deopt was handled, `Ok(false)` if no deopt info found.
    pub(crate) fn handle_tier2_deopt(
        &mut self,
        func_id: u16,
        ctx_buf: &[u64; CTX_U64_SIZE],
    ) -> Result<bool, VMError> {
        // The JIT stores the deopt_id at ctx_buf[0]
        let deopt_id = ctx_buf[0] as usize;
        if deopt_id == u32::MAX as usize {
            // Sentinel means "generic deopt with no precise metadata".
            return Ok(false);
        }

        // Look up DeoptInfo from the TierManager's deopt_points table
        let deopt_info = self
            .tier_manager
            .as_ref()
            .and_then(|mgr| mgr.get_deopt_info(func_id, deopt_id))
            .cloned();

        match deopt_info {
            Some(info) => {
                self.deopt_with_info(&info, ctx_buf)?;

                // Invalidate the Tier 2 code so we don't keep hitting the
                // same guard failure. The function will run interpreted until
                // new feedback triggers recompilation.
                if let Some(ref mut mgr) = self.tier_manager {
                    mgr.invalidate_function(func_id);
                }

                Ok(true)
            }
            None => {
                // No DeoptInfo found — this should only happen if there's a
                // mismatch between JIT code and the deopt_points table (e.g.,
                // after an invalidation race). Fall back to interpreter at
                // current IP without restoring locals.
                Ok(false)
            }
        }
    }

    /// Helper: get the current function ID from the call stack.
    ///
    /// Returns `None` when executing top-level code (no call frame).
    #[inline]
    pub(crate) fn current_function_id(&self) -> Option<u16> {
        self.call_stack.last().and_then(|f| f.function_id)
    }

    /// Called from the dispatch loop when a loop back-edge is detected.
    ///
    /// Increments the per-loop counter in the TierManager and, if the threshold
    /// is crossed, sends an OSR compilation request. Returns whether an OSR
    /// request was triggered (for metrics/logging, not used by the caller to
    /// change control flow).
    pub(crate) fn check_osr_back_edge(&mut self, func_id: u16, loop_ip: usize) -> bool {
        // Record the iteration and check if we should request compilation
        let should_compile = if let Some(ref mut tier_mgr) = self.tier_manager {
            tier_mgr.record_loop_iteration(func_id, loop_ip)
        } else {
            false
        };

        if should_compile {
            // Send OSR compilation request via the TierManager's channel
            if let Some(ref tier_mgr) = self.tier_manager {
                if let Some(tx) = tier_mgr.compilation_sender() {
                    let _ = tx.send(crate::tier::CompilationRequest {
                        function_id: func_id,
                        target_tier: crate::tier::Tier::BaselineJit,
                        blob_hash: None,
                        osr: true,
                        loop_header_ip: Some(loop_ip),
                        feedback: None, // OSR uses Tier 1 (no feedback needed)
                        callee_feedback: std::collections::HashMap::new(),
                    });
                }
            }
        }

        should_compile
    }
}

#[cfg(test)]
mod tests {
    use crate::tier::TierManager;

    #[test]
    fn test_loop_counter_threshold() {
        let mut mgr = TierManager::new(5, true);
        mgr.set_osr_threshold(100);

        for _ in 0..99 {
            assert!(!mgr.record_loop_iteration(0, 42));
        }
        assert!(mgr.record_loop_iteration(0, 42)); // 100th
        assert!(!mgr.record_loop_iteration(0, 42)); // 101st — already triggered
    }

    #[test]
    fn test_osr_table_registration() {
        let mut mgr = TierManager::new(5, true);

        assert!(mgr.get_osr_code(0, 42).is_none());
        let ptr = 0xDEAD as *const u8;
        mgr.register_osr_code(0, 42, ptr);
        assert_eq!(mgr.get_osr_code(0, 42), Some(ptr));
    }

    #[test]
    fn test_invalidate_function_clears_native_code() {
        use std::sync::mpsc;

        let mut mgr = TierManager::new(5, true);
        let (req_tx, _req_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();
        mgr.set_channels(req_tx, res_rx);

        // Drive to threshold and simulate compilation
        for _ in 0..100 {
            mgr.record_call(0, None);
        }
        res_tx
            .send(crate::tier::CompilationResult {
                function_id: 0,
                compiled_tier: crate::tier::Tier::BaselineJit,
                native_code: Some(0xBEEF as *const u8),
                error: None,
                osr_entry: None,
                deopt_points: Vec::new(),
                loop_header_ip: None,
                shape_guards: Vec::new(),
            })
            .unwrap();
        mgr.poll_completions();
        assert!(mgr.get_native_code(0).is_some());

        mgr.invalidate_function(0);

        assert!(mgr.get_native_code(0).is_none());
        assert_eq!(mgr.get_tier(0), crate::tier::Tier::Interpreted);
    }

    #[test]
    fn test_invalidate_osr_clears_loop_entries() {
        let mut mgr = TierManager::new(5, true);
        mgr.set_osr_threshold(10);

        // Register OSR code for two loops
        mgr.register_osr_code(0, 42, 0x1000 as *const u8);
        mgr.register_osr_code(0, 100, 0x2000 as *const u8);
        mgr.register_osr_code(1, 42, 0x3000 as *const u8);

        mgr.invalidate_osr(0);

        assert!(mgr.get_osr_code(0, 42).is_none());
        assert!(mgr.get_osr_code(0, 100).is_none());
        // Function 1 unaffected
        assert!(mgr.get_osr_code(1, 42).is_some());
    }

    #[test]
    fn test_invalidate_all() {
        use std::sync::mpsc;

        let mut mgr = TierManager::new(5, true);
        let (req_tx, _req_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();
        mgr.set_channels(req_tx, res_rx);

        // Simulate whole-function JIT completion
        for _ in 0..100 {
            mgr.record_call(0, None);
        }
        res_tx
            .send(crate::tier::CompilationResult {
                function_id: 0,
                compiled_tier: crate::tier::Tier::BaselineJit,
                native_code: Some(0x1000 as *const u8),
                error: None,
                osr_entry: None,
                deopt_points: Vec::new(),
                loop_header_ip: None,
                shape_guards: Vec::new(),
            })
            .unwrap();
        mgr.poll_completions();

        // Also set up OSR entries
        mgr.register_osr_code(0, 42, 0x2000 as *const u8);
        for _ in 0..20 {
            mgr.record_loop_iteration(0, 42);
        }

        mgr.invalidate_all(0);

        assert!(mgr.get_native_code(0).is_none());
        assert_eq!(mgr.get_tier(0), crate::tier::Tier::Interpreted);
        assert!(mgr.get_osr_code(0, 42).is_none());
        assert_eq!(mgr.get_loop_count(0, 42), 0);
    }

    #[test]
    fn test_deopt_info_inline_frames_default_empty() {
        use crate::bytecode::DeoptInfo;
        use crate::type_tracking::SlotKind;

        let info = DeoptInfo {
            resume_ip: 10,
            local_mapping: vec![(0, 0)],
            local_kinds: vec![SlotKind::NanBoxed],
            stack_depth: 0,
            innermost_function_id: None,
            inline_frames: Vec::new(),
        };
        assert!(info.inline_frames.is_empty());
    }

    #[test]
    fn test_deopt_info_with_inline_frames() {
        use crate::bytecode::{DeoptInfo, InlineFrameInfo};
        use crate::type_tracking::SlotKind;

        let info = DeoptInfo {
            resume_ip: 10,
            local_mapping: vec![(0, 0), (1, 1)],
            local_kinds: vec![SlotKind::NanBoxed, SlotKind::Int64],
            stack_depth: 1,
            innermost_function_id: Some(3),
            inline_frames: vec![
                InlineFrameInfo {
                    function_id: 2,
                    resume_ip: 30,
                    local_mapping: vec![(200, 0)],
                    local_kinds: vec![SlotKind::Float64],
                    stack_depth: 0,
                },
            ],
        };

        assert_eq!(info.inline_frames.len(), 1);
        assert_eq!(info.inline_frames[0].function_id, 2);
        assert_eq!(info.inline_frames[0].resume_ip, 30);
        assert_eq!(info.inline_frames[0].local_kinds[0], SlotKind::Float64);
        assert_eq!(info.innermost_function_id, Some(3));
    }

    #[test]
    fn test_deopt_with_info_debug_assert_length_parity() {
        use crate::bytecode::DeoptInfo;
        use crate::type_tracking::SlotKind;

        // Verify that local_mapping and local_kinds lengths match
        let info = DeoptInfo {
            resume_ip: 5,
            local_mapping: vec![(0, 0), (1, 1), (2, 2)],
            local_kinds: vec![SlotKind::NanBoxed, SlotKind::Int64, SlotKind::Float64],
            stack_depth: 0,
            innermost_function_id: None,
            inline_frames: Vec::new(),
        };
        assert_eq!(info.local_mapping.len(), info.local_kinds.len());
    }

    /// Behavioral test for multi-frame inline deopt reconstruction.
    ///
    /// Since the JIT feature can't be enabled in shape-vm tests, this test
    /// validates the deopt data structures and frame ordering invariants
    /// that deopt_with_inline_frames relies on:
    ///
    /// - inline_frames is outermost-first: [0]=outermost physical function
    /// - return_ip for outermost frame = interpreter ip (after CallValue to JIT fn)
    /// - return_ip for intermediate frames = previous frame's resume_ip + 1
    /// - return_ip for innermost frame = last inline frame's resume_ip + 1
    #[test]
    fn test_deopt_inline_frame_return_ip_calculation() {
        use crate::bytecode::{DeoptInfo, InlineFrameInfo};
        use crate::type_tracking::SlotKind;

        // Simulates A (func 0) → B (func 1) → C (func 2, innermost)
        let deopt_info = DeoptInfo {
            resume_ip: 45,
            local_mapping: vec![(20, 0), (21, 1)],
            local_kinds: vec![SlotKind::NanBoxed, SlotKind::NanBoxed],
            stack_depth: 0,
            innermost_function_id: Some(2),
            inline_frames: vec![
                InlineFrameInfo {
                    function_id: 0,
                    resume_ip: 5, // A's CallValue(B) at IP=5
                    local_mapping: vec![(0, 0), (1, 1)],
                    local_kinds: vec![SlotKind::NanBoxed, SlotKind::NanBoxed],
                    stack_depth: 0,
                },
                InlineFrameInfo {
                    function_id: 1,
                    resume_ip: 25, // B's CallValue(C) at IP=25
                    local_mapping: vec![(10, 0)],
                    local_kinds: vec![SlotKind::NanBoxed],
                    stack_depth: 0,
                },
            ],
        };

        let interpreter_ip = 11; // after CallValue(A) at IP=10

        // Compute return_ips using the same algorithm as deopt_with_inline_frames:
        let frames = &deopt_info.inline_frames;
        let mut return_ips = Vec::new();
        for (i, _iframe) in frames.iter().enumerate() {
            let rip = if i == 0 {
                interpreter_ip // outermost: interpreter's call site + 1
            } else {
                frames[i - 1].resume_ip + 1 // intermediate: prev call + 1
            };
            return_ips.push(rip);
        }
        // Innermost frame's return_ip
        let innermost_rip = frames.last().map(|f| f.resume_ip + 1).unwrap_or(interpreter_ip);

        // Verify return_ip for A (outermost): interpreter ip = 11
        assert_eq!(return_ips[0], 11, "A return_ip");
        // Verify return_ip for B: A's CallValue(B) IP + 1 = 5 + 1 = 6
        assert_eq!(return_ips[1], 6, "B return_ip");
        // Verify return_ip for C (innermost): B's CallValue(C) IP + 1 = 25 + 1 = 26
        assert_eq!(innermost_rip, 26, "C return_ip");

        // Verify frame ordering: [0] is outermost (function 0 = A)
        assert_eq!(frames[0].function_id, 0, "outermost is A");
        assert_eq!(frames[1].function_id, 1, "intermediate is B");
        assert_eq!(deopt_info.innermost_function_id, Some(2), "innermost is C");
    }

    /// Verify that return_ip semantics match the VM dispatch loop.
    ///
    /// The dispatch loop does: fetch at ip → ip += 1 → dispatch.
    /// So return_ip must point to the instruction AFTER the call, not the
    /// call itself. resume_ip stores the call instruction IP, so return_ip
    /// = resume_ip + 1.
    #[test]
    fn test_return_ip_dispatch_loop_semantics() {
        use crate::bytecode::InlineFrameInfo;
        use crate::type_tracking::SlotKind;

        let iframe = InlineFrameInfo {
            function_id: 0,
            resume_ip: 42, // CallValue at IP=42
            local_mapping: vec![],
            local_kinds: vec![],
            stack_depth: 0,
        };

        // The normal VM call convention stores return_ip = self.ip where
        // self.ip has been bumped past the Call instruction. So return_ip
        // = call_ip + 1. Our deopt reconstruction must use resume_ip + 1
        // for consistency.
        let return_ip = iframe.resume_ip + 1;
        assert_eq!(return_ip, 43, "return_ip should be instruction AFTER the call");
    }
}

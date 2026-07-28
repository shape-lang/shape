//! WF-3F regression: SIGINT (interrupt) mid-loop → snapshot → `--resume`
//! must reproduce the CORRECT full-program result, never a silently
//! truncated one.
//!
//! Root cause (fixed at `executor/snapshot.rs` `from_snapshot`): the
//! `load_program` → `reset()` pre-reservation advanced `sp` to
//! `top_level_locals_count`, and the Pass-2 stack rebuild then re-pushed the
//! full saved absolute stack image ON TOP of that reservation — shifting every
//! saved slot up by `top_level_locals_count`. Top-level code runs frameless
//! (bp = 0), so `LoadLocal(idx)` read the empty reserved slot instead of the
//! saved value: the resumed loop counter read 0/Bool, the loop-continue check
//! exited on the first resumed iteration, and the loop tail was silently
//! skipped (save OK, resume WRONG, exit 0). The fix resets `vm.sp = 0` before
//! the rebuild so the saved image restores verbatim at offsets 0..len.
//!
//! This test is DETERMINISTIC (not timing-dependent): the interrupt flag is
//! pre-armed, so the dispatch loop's 1024-instruction interrupt check trips
//! mid-loop on the very first boundary, well before a 200_000-iteration loop
//! can complete. Resume must then finish the loop tail and return the full sum.

use crate::*;
use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
use shape_runtime::snapshot::{SnapshotStore, VmSnapshot};
use std::sync::Arc;
use std::sync::atomic::AtomicU8;

/// Extract an i64 from a program-completion wire value regardless of which
/// integer carrier the host-boundary projection chose.
fn wire_int(v: &shape_wire::WireValue) -> Option<i64> {
    use shape_wire::WireValue as W;
    match v {
        W::Integer(i) | W::I64(i) | W::Isize(i) => Some(*i),
        W::Number(n) => Some(*n as i64),
        _ => None,
    }
}

#[test]
fn interrupt_mid_loop_resume_yields_full_loop_sum() {
    // sum(0..200_000) = 200_000 * 199_999 / 2 = 19_999_900_000
    const N: i64 = 200_000;
    const EXPECTED: i64 = (N * (N - 1)) / 2;

    let source = "\
let mut acc = 0
for i in 0..200000 {
    acc = acc + i
}
acc
";

    // --- Engine + snapshot store setup (mirrors `shape run`) ---
    let tmp = tempfile::tempdir().expect("tempdir");
    let store = SnapshotStore::new(tmp.path()).expect("snapshot store");

    let mut engine = ShapeEngine::new().expect("engine");
    engine.load_stdlib().expect("stdlib");
    engine.init_repl();
    engine.set_script_path("interrupt_resume_regression.shape");
    engine.enable_snapshot_store(store.clone());

    let program = shape_ast::parser::parse_program(source).expect("parse");
    let default_data = shape_runtime::data::DataFrame::default();
    engine
        .get_runtime_mut()
        .load_program(&program, &default_data)
        .expect("load_program");

    // --- Run 1: pre-arm the interrupt so the 1024-instruction dispatch check
    //     fires mid-loop (deterministic — the loop needs far more than 1024
    //     instructions to reach 200_000 iterations). ---
    let interrupt = Arc::new(AtomicU8::new(1));
    let mut executor = BytecodeExecutor::new();
    executor.set_interrupt(interrupt);

    let run = executor.execute_program(&mut engine, &program);
    let snapshot_hex = match run {
        Err(shape_runtime::error::ShapeError::Interrupted {
            snapshot_hash: Some(hex),
        }) => hex,
        Err(shape_runtime::error::ShapeError::Interrupted {
            snapshot_hash: None,
        }) => panic!("interrupt did not persist a snapshot (no-save barrier)"),
        Ok(result) => panic!(
            "loop completed before the interrupt fired (wire={:?}); the interrupt \
             arming/loop-size invariant is broken",
            result.wire_value
        ),
        Err(e) => panic!("unexpected run error: {e:?}"),
    };

    // --- Resolve the saved snapshot and its VM + bytecode blobs (mirrors the
    //     `shape --resume <hash>` full-resume branch). ---
    let hash = store.resolve_hash(&snapshot_hex).expect("resolve hash");
    let (semantic, context, vm_hash, bytecode_hash) =
        engine.load_snapshot(&hash).expect("load_snapshot");
    engine
        .apply_snapshot(semantic, context)
        .expect("apply_snapshot");

    let vm_hash = vm_hash.expect("snapshot carries VM state");
    let bytecode_hash = bytecode_hash.expect("snapshot carries bytecode");
    let vm_snapshot: VmSnapshot = store.get_struct(&vm_hash).expect("get VmSnapshot");
    let bytecode: crate::BytecodeProgram = store
        .get_struct(&bytecode_hash)
        .expect("get BytecodeProgram");

    // Interrupt-origin snapshots carry the WF-3F origin stamp.
    assert!(
        vm_snapshot.interrupt_saved,
        "an interrupt-saved snapshot must stamp interrupt_saved=true"
    );

    // --- Resume with a FRESH (un-armed) interrupt flag so the resumed run
    //     runs to completion. ---
    let resume_interrupt = Arc::new(AtomicU8::new(0));
    let mut resume_executor = BytecodeExecutor::new();
    resume_executor.set_interrupt(resume_interrupt);

    let resumed = resume_executor
        .resume_snapshot(&mut engine, vm_snapshot, bytecode)
        .expect("resume");

    let got = wire_int(&resumed.wire_value).unwrap_or_else(|| {
        panic!(
            "resumed completion is not an integer: {:?}",
            resumed.wire_value
        )
    });

    assert_eq!(
        got, EXPECTED,
        "interrupt-resume must yield the FULL loop sum ({EXPECTED}); got {got} — \
         the loop tail was silently skipped (the WF-3F stack-base-shift regression)"
    );
}

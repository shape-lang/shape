//! W13.2 smoke self-test: run the canonical s1 seed through the harness and
//! assert it classifies as `Convergent` with VM == JIT == `4950`.
//!
//! Skipped when `target/release/shape` is not available at the workspace
//! root (e.g. during `cargo test` before the release binary has been
//! built). The W13.2 close gate runs this test after the release binary
//! is present.

use std::path::PathBuf;

use shape_fuzz::divergence::Divergence;
use shape_fuzz::{CompareConfig, classify_divergence, compare_outputs};

/// Locate the workspace's `target/release/shape` binary. Walks up from
/// `CARGO_MANIFEST_DIR` (`.../tools/shape-fuzz`) to the workspace root.
fn release_binary() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent()?.parent()?;
    let bin = workspace_root.join("target/release/shape");
    if bin.is_file() { Some(bin) } else { None }
}

#[test]
fn s1_self_test_classifies_convergent() {
    let Some(bin) = release_binary() else {
        eprintln!(
            "skipping smoke self-test: target/release/shape not present \
             (build with `cargo build --release --bin shape` first)"
        );
        return;
    };

    let seed = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("smoke-self-test")
        .join("s1.shape");
    assert!(
        seed.is_file(),
        "smoke self-test seed missing: {}",
        seed.display()
    );

    let cfg = CompareConfig {
        shape_binary: bin,
        timeout: std::time::Duration::from_secs(30),
    };
    let cmp = compare_outputs(&seed, &cfg).expect("compare_outputs should succeed");

    let div = classify_divergence(&cmp.vm, &cmp.jit);
    assert_eq!(
        div,
        Divergence::Convergent,
        "s1 self-test must classify as Convergent; got {} (vm={:?} jit={:?})",
        div,
        cmp.vm,
        cmp.jit,
    );
    assert_eq!(cmp.vm.stdout_tail, "4950", "s1 VM stdout_tail must be 4950");
    assert_eq!(
        cmp.jit.stdout_tail, "4950",
        "s1 JIT stdout_tail must be 4950"
    );
    assert_eq!(cmp.vm.exit_code, Some(0));
    assert_eq!(cmp.jit.exit_code, Some(0));
}

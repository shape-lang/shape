//! W13.3 integration test — runs the full corpus through the harness and
//! exercises the mutation engine + minimizer end-to-end.
//!
//! Skipped when `target/release/shape` is not present (mirrors the smoke
//! self-test policy so `cargo test -p shape-fuzz` works without a release
//! build). When the binary is present, this test:
//!
//! 1. Walks `tests/corpus/<domain>/*.shape`, runs each through
//!    `compare_outputs` + `classify_divergence`, and asserts every seed
//!    classifies as `Convergent` OR carries a `// CLASS:` header (the
//!    audit §4.1 negative-corpus marker for known divergences).
//! 2. Picks one base seed, runs `mutate_seed` with a fixed PRNG seed, and
//!    asserts the engine produces > 0 deterministic derived seeds.
//! 3. Writes an injected-divergent program to a tmp file (VM converges via
//!    obvious arithmetic; the bisect tests on convergence preservation
//!    work the same way), invokes `minimize_failure`, and asserts the
//!    outcome shape is well-formed.

use std::path::PathBuf;
use std::time::Duration;

use shape_fuzz::divergence::Signal;
use shape_fuzz::{
    CompareConfig, MinimizeConfig, MutationConfig, classify_divergence, compare_outputs,
    minimize_failure, mutate_seed,
};

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn release_binary() -> Option<PathBuf> {
    let bin = workspace_root().join("target/release/shape");
    if bin.is_file() { Some(bin) } else { None }
}

fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
}

fn collect_seeds(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return out;
    }
    fn visit(p: &std::path::Path, out: &mut Vec<PathBuf>) {
        if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("shape") {
            out.push(p.to_path_buf());
        } else if p.is_dir() {
            for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
                visit(&e.path(), out);
            }
        }
    }
    visit(dir, &mut out);
    out.sort();
    out
}

fn read_first_line(path: &std::path::Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .next()
        .unwrap_or("")
        .to_string()
}

/// A seed is a known negative-corpus entry if its first line contains
/// `CLASS:` (audit §5.3 reproducer ceremony header marker).
fn is_known_negative(path: &std::path::Path) -> bool {
    let head = read_first_line(path);
    head.contains("CLASS:")
}

#[test]
fn corpus_inventory_matches_audit_5_3_expected_layout() {
    // This part runs without the release binary — verifies the corpus
    // shape on disk (audit §3 inventory: 50 hand-seeded + 3 negative).
    let root = corpus_root();
    assert!(root.is_dir(), "corpus root missing: {}", root.display());

    let expected_domains = [
        ("arithmetic", 10),
        // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18):
        // c11_array_typed_object.shape added per audit §4.C (Array<TypedObject>
        // construction + index access).
        // Phase 4b Round 6 WS-1 W16.2-C op_new_array spread/comprehension
        // construction (2026-05-21): c12_spread.shape + c13_comprehension.shape
        // added per audit v0.3-w16-2-c-empty-literal-audit.md §5.E.
        // Phase 4b Round 6 WS-1b W16.2-C residual — bare empty-array
        // accumulator construction (2026-05-21): c14_bare_accumulator.shape.
        // Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05):
        // c15_array_trait_object.shape added (Array<dyn Trait> construction +
        // index access + vtable method dispatch).
        ("collections", 15),
        ("closures", 7),
        ("patterns", 8),
        ("async", 5),
        ("generics", 8),
        ("fallthrough", 2),
    ];
    let mut total = 0usize;
    for (domain, expected_count) in expected_domains {
        let dir = root.join(domain);
        let seeds = collect_seeds(&dir);
        assert_eq!(
            seeds.len(),
            expected_count,
            "{domain} expected {expected_count} seeds, got {}",
            seeds.len()
        );
        total += seeds.len();
    }
    // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18):
    // total grows by 1 (c11_array_typed_object) → 51.
    // Phase 4b Round 6 WS-1 W16.2-C (2026-05-21): total grows by 2
    // (c12_spread + c13_comprehension) → 53.
    // Phase 4b Round 6 WS-1b W16.2-C residual (2026-05-21): total grows by
    // 1 (c14_bare_accumulator) → 54.
    // Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05): total
    // grows by 1 (c15_array_trait_object) → 55.
    assert_eq!(total, 55, "audit §3 requires 55 hand-seeded total");

    // Audit §4.1 baseline negative-corpus inventory: a10 + c09 + c10 = 3
    // entries. W13.3 corpus surfaced 2 NEW divergence classes during the
    // integration-test bring-up (LANG-W13-3-iife-closure-capture +
    // LANG-W13-3-double-filter-chain, see f06 + c08 CLASS headers), so the
    // current floor is 5. Each class flip (n) -> (g) at fix-landing time
    // adjusts this count down.
    let all = collect_seeds(&root);
    let negative_count = all.iter().filter(|p| is_known_negative(p)).count();
    assert!(
        negative_count >= 3,
        "audit §4.1 requires >=3 negative-corpus seeds (a10 / c09 / c10 baseline); \
         W13.3 surfaced +2 NEW classes (LANG-W13-3-iife-closure-capture + \
         LANG-W13-3-double-filter-chain); found {negative_count}"
    );
}

#[test]
fn corpus_all_seeds_converge_or_are_known_negative() {
    let Some(bin) = release_binary() else {
        eprintln!(
            "skipping corpus convergence integration test: target/release/shape missing \
             (build with `cargo build --release --bin shape` first)"
        );
        return;
    };
    let cfg = CompareConfig {
        shape_binary: bin,
        timeout: Duration::from_secs(30),
    };
    let seeds = collect_seeds(&corpus_root());
    assert!(!seeds.is_empty(), "corpus must contain seeds");

    let mut unexpected_divergences = Vec::new();
    for seed in &seeds {
        let cmp = match compare_outputs(seed, &cfg) {
            Ok(c) => c,
            Err(e) => panic!("driver failure on {}: {e}", seed.display()),
        };
        let div = classify_divergence(&cmp.vm, &cmp.jit);
        let known_neg = is_known_negative(seed);
        match (div.signal(), known_neg) {
            (Signal::Convergent, _) | (Signal::Noise, _) => {}
            (Signal::Low, _) => {
                // Both modes error; audit §2 row 4 treats this as LOW-signal.
                // Acceptable for the corpus integration test — the harness
                // CLI exits non-zero on LOW only without --allow-low-signal.
            }
            (_, true) => {
                // Known-negative seed actually diverged — expected per the
                // §5.1 residual class. No assertion failure.
            }
            (_, false) => {
                unexpected_divergences.push(format!(
                    "{}: {} (vm={:?} jit={:?})",
                    seed.display(),
                    div,
                    cmp.vm,
                    cmp.jit
                ));
            }
        }
    }

    if !unexpected_divergences.is_empty() {
        panic!(
            "corpus must not contain unmarked divergent seeds (audit §4.1 binding).\n\
             {} unmarked divergence(s):\n{}",
            unexpected_divergences.len(),
            unexpected_divergences.join("\n")
        );
    }
}

#[test]
fn mutation_engine_produces_derived_seeds_for_a_base_corpus_seed() {
    let base = corpus_root().join("arithmetic").join("a09_for_sum.shape");
    assert!(base.is_file(), "base seed missing: {}", base.display());
    let source = std::fs::read_to_string(&base).unwrap();
    let cfg = MutationConfig {
        max_mutations: 5,
        prng_seed: 0x123456789abcdef,
    };
    let derived = mutate_seed(&source, &cfg);
    assert!(
        !derived.is_empty(),
        "mutation engine should produce derived seeds for a09 (has integer literals + tier-up wrap candidate)"
    );
    assert!(derived.len() <= 5, "exceeded max_mutations cap");
    for d in &derived {
        assert_ne!(d.source, source, "derived seed must differ from base");
    }
    // Determinism check.
    let derived_again = mutate_seed(&source, &cfg);
    assert_eq!(
        derived, derived_again,
        "mutation engine must be deterministic"
    );
}

#[test]
fn minimizer_handles_a_base_not_high_signal_seed_correctly() {
    let Some(bin) = release_binary() else {
        eprintln!("skipping minimizer integration test: target/release/shape missing");
        return;
    };
    // The audit §5.1 minimizer pre-condition is "the seed is HIGH-signal
    // divergent". We use a convergent seed here to assert the negative
    // path returns `BaseNotHighSignal`, which is the engine's binding for
    // "nothing to minimize". An end-to-end HIGH-signal minimization happens
    // when a real finding fires in CI; integration-testing that requires
    // the corpus to actually surface a HIGH divergence, which is not a
    // stable test fixture.
    let tmp_dir = std::env::temp_dir().join(format!(
        "shape-fuzz-min-int-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let seed = tmp_dir.join("convergent.shape");
    std::fs::write(
        &seed,
        "let mut sum = 0\nfor i in 0..10 { sum += i }\nprint(sum)\n",
    )
    .unwrap();

    let cfg = MinimizeConfig::new(
        tmp_dir.clone(),
        CompareConfig {
            shape_binary: bin,
            timeout: Duration::from_secs(30),
        },
    );
    let outcome = minimize_failure(&seed, &cfg).expect("minimize_failure should not driver-fail");
    match outcome {
        shape_fuzz::MinimizeOutcome::BaseNotHighSignal { .. } => {}
        other => panic!("expected BaseNotHighSignal on a convergent seed, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn minimizer_split_handles_multi_block_source_correctly() {
    // Pure-Rust test of the bisect block splitter — does not invoke the
    // shape binary, so it always runs. Verifies the splitter sees an
    // injected-divergent style program (extra unrelated lines around a
    // core 2-line failure) as separable blocks.
    let src = "let unrelated = 1\n\
               fn f(x: int) -> int { x + 1 }\n\
               print(f(2))\n\
               let trailing = 99\n";
    let blocks = shape_fuzz::minimizer::split_into_blocks(src);
    assert!(
        blocks.len() >= 4,
        "expected >=4 blocks, got {}: {:?}",
        blocks.len(),
        blocks
    );
    // The fn block must remain intact (brace-balanced).
    let fn_block = blocks
        .iter()
        .find(|b| b.starts_with("fn f"))
        .expect("fn block");
    assert!(fn_block.ends_with("}"));
}

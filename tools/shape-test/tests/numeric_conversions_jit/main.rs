//! # Numeric-conversion conformance suite — JIT-mode execution variant (WF-0B)
//!
//! This target runs the EXACT SAME 117 conformance cases as the sibling
//! `numeric_conversions` target (the five category files are `#[path]`-included,
//! byte-identical — see that target's `main.rs` for THE RULE and the category
//! taxonomy), but drives every program through `shape_jit::JITExecutor`
//! (the release binary's `--mode jit` path: AOT selective compilation with
//! interpreter fall-through) instead of the bytecode interpreter.
//!
//! ## Why this target exists
//!
//! audit-2026-07-04 (`docs/cluster-audits/audit-2026-07-04-claimed-vs-real.md`
//! §4.4) found the conformance suite was structurally VM-only: `ShapeTest`
//! executed in-process through `BytecodeExecutor` exclusively, so the JIT tier
//! never ran these programs and the D3 i64-overflow VM/JIT split-brain
//! (VM: structured overflow error; JIT: silent i64 wrap) was invisible to CI.
//! This target closes that blind spot.
//!
//! ## Known-divergence discipline (known-divergent-pending-WF-1A)
//!
//! Cases where `--mode jit` currently DIVERGES from THE RULE are NOT skipped
//! and NOT left red: they are pinned in [`suite::KNOWN_JIT_DIVERGENT`], keyed
//! by the exact Shape source of the case. For a pinned case the wrapper
//! asserts the divergence is STILL PRESENT (the spec-level assertion must
//! fail). The moment WF-1A fixes the JIT class, the pinned case starts
//! passing and this suite fails LOUDLY with an instruction to remove the
//! entry — flipping the case to an unconditional both-modes assertion.
//!
//! The conformance test bodies themselves are never relaxed (they remain the
//! permanent encoding of the 2026-06-01 D1/D2/D3 ruling); only this target's
//! divergence ledger tracks the JIT gap.

/// Executor selection shim + JIT known-divergence ledger.
///
/// The category files resolve `crate::suite::ShapeTest`; here that is a thin
/// wrapper around `shape_test::shape_test::ShapeTest` that (a) switches
/// execution to `shape_jit::JITExecutor` via `.with_jit()` and (b) inverts
/// the assertion for cases pinned in [`KNOWN_JIT_DIVERGENT`].
pub(crate) mod suite {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    /// One hand-verified VM-vs-JIT divergence, pending the WF-1A JIT fix.
    pub struct KnownDivergence {
        /// Test id (`<category_module>::<test_fn>`), for humans and grep.
        pub id: &'static str,
        /// Exact Shape source of the case — the lookup key. The conformance
        /// sources are permanent, so this key is stable.
        pub source: &'static str,
        /// Divergence class (audit-2026-07-04 §4.4 taxonomy / WF-0B corpus).
        pub class: &'static str,
        /// Hand-verified observed JIT behavior at the time of pinning.
        pub observed: &'static str,
    }

    /// JIT-mode divergences from THE RULE, pending WF-1A.
    ///
    /// INVARIANT (asserted by the wrapper): every entry here still diverges.
    /// When WF-1A lands the fix for an entry's class, the corresponding test
    /// panics with a removal instruction. NEVER add an entry without hand-
    /// verifying the divergence against the current binary; NEVER remove one
    /// without the fix actually landing.
    ///
    /// The 23 original entries were hand-verified 2026-07-05 (shape 0.3.2 @
    /// 1fb805b3): each class representative reproduced through BOTH the
    /// in-process `JITExecutor` path and the release binary
    /// (`target/release/shape run --mode jit` vs `--mode vm` — VM correct in
    /// every case, all 117 green under the sibling VM target). Class 5
    /// (`jit-i64-overflow-silent-wrap`, 1 entry) was RETIRED by the WF-1A
    /// i64-checked fix (2026-07-05), leaving 22 entries across four classes:
    ///
    /// 1. `jit-i8-u8-result-misread-as-bool` (17): any program whose final
    ///    value ORIGINATES from an i8/u8-typed binding (even after widening
    ///    into i16/i32/int/u16/u32/u64/number) comes back as `Bool(true)`
    ///    under JIT — a 1-byte NativeKind (Int8) misread as Bool at the JIT
    ///    result boundary.
    /// 2. `jit-u32-sign-extension-wrap` (3): u32 value above i32::MAX
    ///    (4000000000) comes back sign-extended from 32 bits: -294967296.
    /// 3. `jit-return-raw-int-bits-as-f64` (1): `-> number` fn returning an
    ///    adopted int literal yields f64::from_bits(1) = 5e-324 — raw i64 bits
    ///    reinterpreted as f64 (WF-0B baseline class hof-return-kind-raw-bits).
    /// 4. `jit-number-eq-int-literal-false` (1): `5.0 == 5` (literal adoption
    ///    in comparison) evaluates false under JIT, true under VM.
    ///
    /// RETIRED — 5. `jit-i64-overflow-silent-wrap` (was 1 entry): i64::MAX + 1
    ///    silently wrapped to i64::MIN under JIT instead of the D3 structured
    ///    overflow error (the audit-2026-07-04 §4.4 split-brain this target
    ///    exists to watch). WF-1A now emits a guarded signed-overflow branch
    ///    in the JIT (`compile_int64_checked_arith`); both modes raise the
    ///    same overflow error, so the case is asserted unconditionally.
    pub const KNOWN_JIT_DIVERGENT: &[KnownDivergence] = &[
        // -- class 1: jit-i8-u8-result-misread-as-bool ---------------------
        KnownDivergence {
            id: "category_a_lossless_widening::a_i8_to_i16_positive",
            source: "let a: i8 = 100\nlet b: i16 = a\nb",
            class: "jit-i8-u8-result-misread-as-bool",
            observed: "Bool(true) instead of Integer(100)",
        },
        KnownDivergence {
            id: "category_a_lossless_widening::a_i8_to_i16_negative",
            source: "let a: i8 = -100\nlet b: i16 = a\nb",
            class: "jit-i8-u8-result-misread-as-bool",
            observed: "Bool(true) instead of Integer(-100)",
        },
        KnownDivergence {
            id: "category_a_lossless_widening::a_i8_to_i32",
            source: "let a: i8 = -128\nlet b: i32 = a\nb",
            class: "jit-i8-u8-result-misread-as-bool",
            observed: "Bool(true) instead of Integer(-128)",
        },
        KnownDivergence {
            id: "category_a_lossless_widening::a_i8_to_int",
            source: "let a: i8 = 127\nlet b: int = a\nb",
            class: "jit-i8-u8-result-misread-as-bool",
            observed: "Bool(true) instead of Integer(127)",
        },
        KnownDivergence {
            id: "category_a_lossless_widening::a_i8_to_number",
            source: "let a: i8 = -100\nlet b: number = a\nb",
            class: "jit-i8-u8-result-misread-as-bool",
            observed: "Bool(true) instead of Number(-100.0)",
        },
        KnownDivergence {
            id: "category_a_lossless_widening::a_u8_identity",
            source: "let a: u8 = 200\nlet b: u8 = a\nb",
            class: "jit-i8-u8-result-misread-as-bool",
            observed: "Bool(true) instead of Integer(200)",
        },
        KnownDivergence {
            id: "category_a_lossless_widening::a_u8_to_u16",
            source: "let a: u8 = 200\nlet b: u16 = a\nb",
            class: "jit-i8-u8-result-misread-as-bool",
            observed: "Bool(true) instead of Integer(200)",
        },
        KnownDivergence {
            id: "category_a_lossless_widening::a_u8_to_u16_max",
            source: "let a: u8 = 255\nlet b: u16 = a\nb",
            class: "jit-i8-u8-result-misread-as-bool",
            observed: "Bool(true) instead of Integer(255)",
        },
        KnownDivergence {
            id: "category_a_lossless_widening::a_u8_to_u32",
            source: "let a: u8 = 255\nlet b: u32 = a\nb",
            class: "jit-i8-u8-result-misread-as-bool",
            observed: "Bool(true) instead of Integer(255)",
        },
        KnownDivergence {
            id: "category_a_lossless_widening::a_u8_to_u64",
            source: "let a: u8 = 255\nlet b: u64 = a\nb",
            class: "jit-i8-u8-result-misread-as-bool",
            observed: "Bool(true) instead of Integer(255)",
        },
        KnownDivergence {
            id: "category_a_lossless_widening::a_u8_to_i16",
            source: "let a: u8 = 255\nlet b: i16 = a\nb",
            class: "jit-i8-u8-result-misread-as-bool",
            observed: "Bool(true) instead of Integer(255)",
        },
        KnownDivergence {
            id: "category_a_lossless_widening::a_u8_to_i32",
            source: "let a: u8 = 200\nlet b: i32 = a\nb",
            class: "jit-i8-u8-result-misread-as-bool",
            observed: "Bool(true) instead of Integer(200)",
        },
        KnownDivergence {
            id: "category_a_lossless_widening::a_u8_to_int",
            source: "let a: u8 = 200\nlet b: int = a\nb",
            class: "jit-i8-u8-result-misread-as-bool",
            observed: "Bool(true) instead of Integer(200)",
        },
        KnownDivergence {
            id: "category_a_lossless_widening::a_u8_to_number",
            source: "let a: u8 = 255\nlet b: number = a\nb",
            class: "jit-i8-u8-result-misread-as-bool",
            observed: "Bool(true) instead of Number(255.0)",
        },
        KnownDivergence {
            id: "category_d_literal_adoption::d_in_range_literal_u8",
            source: "let x: u8 = 200\nx",
            class: "jit-i8-u8-result-misread-as-bool",
            observed: "Bool(true) instead of Integer(200)",
        },
        KnownDivergence {
            id: "category_d_literal_adoption::d_in_range_literal_u8_max",
            source: "let x: u8 = 255\nx",
            class: "jit-i8-u8-result-misread-as-bool",
            observed: "Bool(true) instead of Integer(255)",
        },
        KnownDivergence {
            id: "category_d_literal_adoption::d_in_range_literal_i8_min",
            source: "let x: i8 = -128\nx",
            class: "jit-i8-u8-result-misread-as-bool",
            observed: "Bool(true) instead of Integer(-128)",
        },
        // -- class 2: jit-u32-sign-extension-wrap --------------------------
        KnownDivergence {
            id: "category_a_lossless_widening::a_u32_to_int",
            source: "let a: u32 = 4000000000\nlet b: int = a\nb",
            class: "jit-u32-sign-extension-wrap",
            observed: "Integer(-294967296) instead of Integer(4000000000)",
        },
        KnownDivergence {
            id: "category_a_lossless_widening::a_u32_to_u64",
            source: "let a: u32 = 4000000000\nlet b: u64 = a\nb",
            class: "jit-u32-sign-extension-wrap",
            observed: "Integer(-294967296) instead of Integer(4000000000)",
        },
        KnownDivergence {
            id: "category_a_lossless_widening::a_u32_to_number",
            source: "let a: u32 = 4000000000\nlet b: number = a\nb",
            class: "jit-u32-sign-extension-wrap",
            observed: "Integer(-294967296) instead of Number(4000000000.0)",
        },
        // -- class 3: jit-return-raw-int-bits-as-f64 -----------------------
        KnownDivergence {
            id: "category_d_literal_adoption::d_match_arm_int_literal_in_number_fn",
            source: "fn f(x: number) -> number {\n  match x {\n    0.0 => 1\n    _ => 2\n  }\n}\nf(0.0)",
            class: "jit-return-raw-int-bits-as-f64",
            observed: "Number(5e-324) = f64::from_bits(1) instead of Number(1.0)",
        },
        // -- class 4: jit-number-eq-int-literal-false ----------------------
        KnownDivergence {
            id: "category_d_literal_adoption::d_number_var_eq_int_literal",
            source: "let val: number = 5.0\nval == 5",
            class: "jit-number-eq-int-literal-false",
            observed: "Bool(false) instead of Bool(true)",
        },
        // -- class 5: jit-i64-overflow-silent-wrap (audit §4.4 / D3) -------
        // RETIRED by WF-1A i64-checked (2026-07-05): the JIT now emits a
        // guarded signed-overflow branch on `int` (i64) add/sub/mul
        // (`compile_int64_checked_arith` → `JIT_SIGNAL_INT_OVERFLOW`), so
        // `let a: int = i64::MAX\nlet b: int = 1\na + b` raises the SAME
        // structured integer-overflow runtime error under `--mode jit` as
        // under `--mode vm`. The conformance case
        // `category_e_silent_lossy_forbidden::e_int_overflow_is_runtime_error`
        // is now asserted unconditionally in BOTH modes (no ledger entry).
    ];

    /// JIT-mode `ShapeTest` wrapper. Same builder surface as the real one
    /// (only the methods the numeric_conversions categories use), executing
    /// through the JIT and enforcing the known-divergence ledger.
    pub struct ShapeTest {
        source: String,
        inner: shape_test::shape_test::ShapeTest,
    }

    impl ShapeTest {
        pub fn new(text: &str) -> Self {
            Self {
                source: text.to_string(),
                inner: shape_test::shape_test::ShapeTest::new(text).with_jit(),
            }
        }

        /// Run one assertion, inverted when the case is a pinned divergence:
        /// - unlisted case → assert normally (JIT must match THE RULE);
        /// - listed + assertion FAILS → divergence still present → test green
        ///   (with an audit note on stderr);
        /// - listed + assertion PASSES → WF-1A fixed the class → panic with a
        ///   remove-the-entry instruction so the flip is loud, not silent.
        fn guarded(
            self,
            run: impl FnOnce(
                shape_test::shape_test::ShapeTest,
            ) -> shape_test::shape_test::ShapeTest,
        ) -> Self {
            let Self { source, inner } = self;
            let pinned = KNOWN_JIT_DIVERGENT.iter().find(|d| d.source == source);
            match pinned {
                None => {
                    let inner = run(inner);
                    Self { source, inner }
                }
                Some(entry) => {
                    let outcome = catch_unwind(AssertUnwindSafe(move || run(inner)));
                    match outcome {
                        Err(_) => {
                            eprintln!(
                                "known-divergent-pending-WF-1A: '{}' still diverges under \
                                 --mode jit [{}]: {}",
                                entry.id, entry.class, entry.observed
                            );
                            // Fresh (unexecuted) builder so chaining stays valid.
                            Self::new(&source)
                        }
                        Ok(_) => panic!(
                            "KNOWN_JIT_DIVERGENT entry '{}' now PASSES under --mode jit.\n\
                             The WF-1A fix for class '{}' has landed — remove this entry \
                             from\ntools/shape-test/tests/numeric_conversions_jit/main.rs \
                             (mod suite, KNOWN_JIT_DIVERGENT)\nso the case is asserted \
                             unconditionally in both execution modes.",
                            entry.id, entry.class
                        ),
                    }
                }
            }
        }

        pub fn expect_number(self, expected: f64) -> Self {
            self.guarded(move |t| t.expect_number(expected))
        }

        pub fn expect_bool(self, expected: bool) -> Self {
            self.guarded(move |t| t.expect_bool(expected))
        }

        pub fn expect_run_err(self) -> Self {
            self.guarded(|t| t.expect_run_err())
        }
    }

    /// Ledger hygiene: every pinned source must correspond to exactly one
    /// conformance case key (no duplicate keys inside the ledger either).
    #[test]
    fn known_divergent_ledger_has_unique_keys() {
        for (i, a) in KNOWN_JIT_DIVERGENT.iter().enumerate() {
            for b in &KNOWN_JIT_DIVERGENT[i + 1..] {
                assert_ne!(
                    a.source, b.source,
                    "duplicate KNOWN_JIT_DIVERGENT source key: {} / {}",
                    a.id, b.id
                );
            }
        }
    }
}

#[path = "../numeric_conversions/category_a_lossless_widening.rs"]
mod category_a_lossless_widening;
#[path = "../numeric_conversions/category_b_lossy_implicit_rejected.rs"]
mod category_b_lossy_implicit_rejected;
#[path = "../numeric_conversions/category_c_explicit_casts.rs"]
mod category_c_explicit_casts;
#[path = "../numeric_conversions/category_d_literal_adoption.rs"]
mod category_d_literal_adoption;
#[path = "../numeric_conversions/category_e_silent_lossy_forbidden.rs"]
mod category_e_silent_lossy_forbidden;

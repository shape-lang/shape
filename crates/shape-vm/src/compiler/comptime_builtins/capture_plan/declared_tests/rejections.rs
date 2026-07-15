use super::*;

    // ───────────────────────────────────────────────────────────────────
    // (3) THE REJECTION MATRIX — `lower_declared` is TOTAL-OR-REJECT.
    //     Every `(mode × facts)` pair below either lowers or names a code.
    // ───────────────────────────────────────────────────────────────────

    fn facts(
        target: Option<CaptureTarget>,
        ownership: Option<BindingOwnershipClass>,
        witness_shared_local: bool,
    ) -> CaptureBindingFacts {
        CaptureBindingFacts {
            name: "x".to_string(),
            target,
            binding_span: None,
            binding_lineage: None,
            binding_file_id: 0,
            semantic_type: CaptureSemanticEvidence::unavailable(
                CaptureSemanticIssueKind::MissingInferenceFact,
                "rejection test has no binding inference subject",
            ),
            ownership,
            storage: None,
            mutated: false,
            boxed: false,
            witness_shared_local,
            witness_shared_module_binding: false,
            witness_owned_mutable_local: false,
            inherited_capture_parameter: false,
            inherited_shared_cell: false,
        }
    }

    fn reject(mode: CaptureMode, f: &CaptureBindingFacts) -> String {
        lower_declared(mode, f).expect_err("must reject")
    }

    #[test]
    fn c0902_borrow_modes_are_a_total_rejection() {
        for mode in [CaptureMode::SharedBorrow, CaptureMode::ExclusiveBorrow] {
            // Every fact shape — a borrow never lowers, whatever it points at.
            for target in [
                None,
                Some(CaptureTarget::Local(1)),
                Some(CaptureTarget::ModuleBinding(1)),
            ] {
                for ownership in [
                    None,
                    Some(BindingOwnershipClass::OwnedImmutable),
                    Some(BindingOwnershipClass::OwnedMutable),
                    Some(BindingOwnershipClass::Flexible),
                ] {
                    let message = reject(mode, &facts(target, ownership, false));
                    assert!(
                        message.starts_with("[C0902] ReferenceEscapeIntoClosure:"),
                        "got {message}"
                    );
                }
            }
        }
    }

    #[test]
    fn c0905_unresolvable_target_is_rejected_not_defaulted() {
        for mode in [CaptureMode::Move, CaptureMode::Share] {
            let message = reject(mode, &facts(None, Some(BindingOwnershipClass::OwnedImmutable), false));
            assert!(message.starts_with("[C0905]"), "got {message}");
        }
    }

    /// [C0905] — the `move` × unknown-ownership arm. NO `Immutable` fallback,
    /// no `MutableCell`: an ownership class the compiler cannot name is an
    /// error, because guessing it is how a declaration gets silently
    /// downgraded.
    #[test]
    fn c0905_unknown_ownership_class_is_rejected_not_defaulted() {
        let message = reject(
            CaptureMode::Move,
            &facts(Some(CaptureTarget::Local(1)), None, false),
        );
        assert!(message.starts_with("[C0905]"), "got {message}");
    }

    /// RULING 1 — `move` never lies. A module-level binding admits no move.
    /// Inference lowers exactly this shape to `Shared`; the declared path
    /// refuses rather than emit a kind whose name is not the declared word.
    #[test]
    fn c0906_move_on_module_binding_is_rejected() {
        for ownership in [
            Some(BindingOwnershipClass::OwnedImmutable),
            Some(BindingOwnershipClass::OwnedMutable),
            Some(BindingOwnershipClass::Flexible),
        ] {
            let message = reject(
                CaptureMode::Move,
                &facts(Some(CaptureTarget::ModuleBinding(2)), ownership, false),
            );
            assert_eq!(
                message,
                "[C0906] module-level binding 'x' cannot be moved into a closure; module \
                 bindings live for the program and admit no move"
            );
        }
    }

    /// [C0904] — a declaration may not UN-SHARE. A local `var`, or a local a
    /// sibling closure already promoted to a `SharedCell`, is shared ownership;
    /// `move` would hand this closure a private snapshot while the sibling keeps
    /// writing the cell.
    #[test]
    fn c0904_move_cannot_unshare_a_var_or_a_sibling_shared_local() {
        let var_local = reject(
            CaptureMode::Move,
            &facts(
                Some(CaptureTarget::Local(1)),
                Some(BindingOwnershipClass::Flexible),
                false,
            ),
        );
        assert!(var_local.starts_with("[C0904]"), "got {var_local}");

        let sibling_shared = reject(
            CaptureMode::Move,
            &facts(
                Some(CaptureTarget::Local(1)),
                Some(BindingOwnershipClass::OwnedImmutable),
                true, // a sibling closure already promoted it
            ),
        );
        assert!(sibling_shared.starts_with("[C0904]"), "got {sibling_shared}");
    }

    /// [C0908] (ruling 2) — `share` over a plain local: there is nothing shared
    /// to take a share OF.
    #[test]
    fn c0908_share_on_a_plain_local_is_rejected() {
        for ownership in [
            BindingOwnershipClass::OwnedImmutable,
            BindingOwnershipClass::OwnedMutable,
        ] {
            let message = reject(
                CaptureMode::Share,
                &facts(Some(CaptureTarget::Local(1)), Some(ownership), false),
            );
            assert!(message.starts_with("[C0908]"), "got {message}");
        }
    }

    /// THE ACCEPT TABLE, total. Every pair that lowers, and what it lowers to —
    /// declared word == emitted kind, with no exceptions (rulings 1 + 2).
    #[test]
    fn lower_declared_accept_table_is_the_ruling() {
        // move × local `let` → Immutable + a leading param.
        let p = lower_declared(
            CaptureMode::Move,
            &facts(
                Some(CaptureTarget::Local(1)),
                Some(BindingOwnershipClass::OwnedImmutable),
                false,
            ),
        )
        .unwrap();
        assert_eq!(p.kind(), CaptureKind::Immutable);
        assert_eq!(p.access(), CaptureAccess::Param);

        // move × local `let mut` → OwnedMutable, REGARDLESS of `mutated`.
        for mutated in [false, true] {
            let mut f = facts(
                Some(CaptureTarget::Local(1)),
                Some(BindingOwnershipClass::OwnedMutable),
                false,
            );
            f.mutated = mutated;
            let p = lower_declared(CaptureMode::Move, &f).unwrap();
            assert_eq!(
                p.kind(),
                CaptureKind::OwnedMutable,
                "the DECLARATION decides, not the body: `mutated` is not an input"
            );
            assert_eq!(p.access(), CaptureAccess::OwnedMutableCell);
        }

        // share × local `var` → Shared.
        let p = lower_declared(
            CaptureMode::Share,
            &facts(
                Some(CaptureTarget::Local(1)),
                Some(BindingOwnershipClass::Flexible),
                false,
            ),
        )
        .unwrap();
        assert_eq!(p.kind(), CaptureKind::Shared);
        assert_eq!(p.access(), CaptureAccess::SharedCell);

        // share × module binding → Shared.
        let p = lower_declared(
            CaptureMode::Share,
            &facts(
                Some(CaptureTarget::ModuleBinding(3)),
                Some(BindingOwnershipClass::OwnedMutable),
                false,
            ),
        )
        .unwrap();
        assert_eq!(p.kind(), CaptureKind::Shared);

        // share × sibling-shared local → Shared.
        let p = lower_declared(
            CaptureMode::Share,
            &facts(
                Some(CaptureTarget::Local(1)),
                Some(BindingOwnershipClass::OwnedImmutable),
                true,
            ),
        )
        .unwrap();
        assert_eq!(p.kind(), CaptureKind::Shared);
    }

    /// [B0003] — NEITHER WIDENED NOR NARROWED by a declaration.
    ///
    /// The reference-escape rule is not the declared path's to change, so this
    /// pins the declared path against the INFERRED path at the same position,
    /// in both directions:
    ///
    ///   (a) TOP-LEVEL, inferred: `[B0003]` fires. (The clause cannot reach
    ///       here — it is generated-code-only, and generated code always
    ///       compiles inside a function — so the arm is held by the inferred
    ///       path, which is the only one that can.)
    ///   (b) INSIDE A FUNCTION, inferred: it does NOT fire. The front-end arm
    ///       is guarded on `current_function.is_none()`, and the MIR solver's
    ///       `ReferenceEscapeIntoClosure` fact does not catch this shape.
    ///       That is a PRE-EXISTING hole in B0003's coverage — verified on the
    ///       INFERRED path, on the parent commit's behaviour, with no clause
    ///       anywhere in the program.
    ///   (c) INSIDE A GENERATED BODY, DECLARED `move`: identical to (b).
    ///
    /// (c) == (b) is the whole assertion. A declared `move` does not rescue a
    /// reference the compiler would otherwise reject, and it does not reject a
    /// reference the compiler would otherwise admit. If a future change makes
    /// the escape check total inside function bodies, (b) and (c) must move
    /// TOGETHER — and this test will say so, loudly, rather than letting the
    /// declared path drift ahead of or behind inference.
    #[test]
    fn b0003_is_neither_widened_nor_narrowed_by_a_declaration() {
        fn outcome(src: &str) -> std::result::Result<(), String> {
            let program = shape_ast::parse_program(src).expect("fixture parses");
            let mut compiler = BytecodeCompiler::new();
            compiler
                .compile_in_place(&program)
                .map_err(|e| e.to_string())
        }

        // (a) top level, inferred — the arm fires, VERBATIM.
        let top_level = outcome(
            r#"
let value = 7
let r = &value
let worker = |y: int| y + r
worker(2)
"#,
        )
        .expect_err("a reference cannot escape into a top-level closure");
        assert!(
            top_level.contains(
                "[B0003] reference 'r' cannot escape into a closure; capture a value instead"
            ),
            "the B0003 message must be byte-identical: {top_level}"
        );

        // (b) inside a function, inferred — the PRE-EXISTING hole.
        let inferred_in_fn = outcome(
            r#"
fn read(x: int) -> int {
  let value = 7
  let r = &value
  let worker = |y: int| y + r
  worker(x)
}
read(2)
"#,
        );

        // (c) inside a GENERATED body, DECLARED `move` — must match (b) exactly.
        let declared_in_generated = outcome(
            r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read(x: int) -> int { let value = 7
      let r = &value
      let worker = |y: int; move r| y + r
      worker(x) } }")
  }
}
@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.read(2)
"#,
        );

        assert_eq!(
            inferred_in_fn.is_ok(),
            declared_in_generated.is_ok(),
            "a declared `move` must treat a reference capture EXACTLY as inference does \
             at the same position — inferred: {inferred_in_fn:?}, declared: \
             {declared_in_generated:?}"
        );
    }

    /// R6 / X1 — the inference residual is UNREACHABLE on the declared path.
    /// Every accepting arm of `lower_declared` is enumerated above; none of them
    /// produces `MutableCell`. This test proves it exhaustively over the fact
    /// cross-product rather than by reading the code.
    #[test]
    fn declared_path_never_produces_the_mutable_cell_residual() {
        let mut accepted = 0usize;
        for mode in [
            CaptureMode::Move,
            CaptureMode::Share,
            CaptureMode::SharedBorrow,
            CaptureMode::ExclusiveBorrow,
        ] {
            for target in [
                None,
                Some(CaptureTarget::Local(1)),
                Some(CaptureTarget::ModuleBinding(1)),
            ] {
                for ownership in [
                    None,
                    Some(BindingOwnershipClass::OwnedImmutable),
                    Some(BindingOwnershipClass::OwnedMutable),
                    Some(BindingOwnershipClass::Flexible),
                ] {
                    for mutated in [false, true] {
                        for boxed in [false, true] {
                            for wsl in [false, true] {
                                for wsm in [false, true] {
                                    let mut f = facts(target, ownership, wsl);
                                    f.mutated = mutated;
                                    f.boxed = boxed;
                                    f.witness_shared_module_binding = wsm;
                                    match lower_declared(mode, &f) {
                                        Ok(plan) => {
                                            accepted += 1;
                                            assert_ne!(
                                                plan.access(),
                                                CaptureAccess::MutableCell,
                                                "the declared path produced the inference \
                                                 residual for {f:?} / {mode:?}"
                                            );
                                            // And the ruling: word == kind.
                                            match mode {
                                                CaptureMode::Move => assert!(matches!(
                                                    plan.kind(),
                                                    CaptureKind::Immutable
                                                        | CaptureKind::OwnedMutable
                                                )),
                                                CaptureMode::Share => assert_eq!(
                                                    plan.kind(),
                                                    CaptureKind::Shared
                                                ),
                                                _ => panic!("a borrow must never lower"),
                                            }
                                        }
                                        Err(message) => assert!(
                                            message.starts_with("[C09"),
                                            "every rejection carries a code: {message}"
                                        ),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(accepted > 0, "the accept arms must be reachable");
    }

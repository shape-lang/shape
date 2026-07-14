    use super::tests::compile;
    use super::*;
    use crate::compiler::BytecodeCompiler;
    use shape_ast::ast::CaptureMode;

    // ───────────────────────────────────────────────────────────────────
    // (1) THE DISTINGUISHING ACCEPT PAIR — the test the rejected C1 could
    //     not write.
    //
    // `let mut hits`, READ-ONLY in the closure body. Inference gives
    // `Immutable` + a leading param (the `!mutable_flags[i]` short-circuit).
    // The DECLARATION says `move`, and `move` × `let mut` is `OwnedMutable`.
    //
    // SELF-TEST (the reason C1's accept test was worthless): DELETE the
    // clause from `flagship_declared` and the assertions below FAIL — the
    // capture reverts to `Immutable`/`Param`. The declaration is doing the
    // work, not inference. `flagship_inferred_source` is that deletion,
    // pinned as the negative control.
    // ───────────────────────────────────────────────────────────────────

    /// A generated `extend Job { method scale }` — THE FLAGSHIP surface — whose
    /// closure declares `move hits` over a `let mut` it only READS.
    const FLAGSHIP_DECLARED: &str = r#"
annotation add_scaler() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method scale(f: int) -> int \{ let mut hits = 3
      let worker = |x: int; move hits| x * hits
      worker(f) \} \}")
  }
}

@add_scaler()
type Job { id: int }

let job = Job { id: 1 }
job.scale(2)
"#;

    /// THE NEGATIVE CONTROL: byte-for-byte the same closure body and the same
    /// `let mut hits`, with NO clause, in ORDINARY SOURCE (the clause is a
    /// generated-code-only surface, so the control cannot also be generated).
    /// Inference picks `Immutable`.
    const FLAGSHIP_INFERRED_SOURCE: &str = r#"
fn scale(f: int) -> int {
  let mut hits = 3
  let worker = |x: int| x * hits
  worker(f)
}
scale(2)
"#;

    /// The closure function's own instruction window. Reading the OPCODES is
    /// the half of the accept proof a layout-mask assertion cannot give you:
    /// the rejected C1 branch set the mask correctly and still emitted a
    /// leading-param load in the body.
    fn closure_body_opcodes(c: &BytecodeCompiler, func_idx: u16) -> Vec<crate::bytecode::OpCode> {
        let function = &c.program.functions[func_idx as usize];
        let start = function.entry_point;
        let end = (start + function.body_length).min(c.program.instructions.len());
        c.program.instructions[start..end]
            .iter()
            .map(|instruction| instruction.opcode)
            .collect()
    }

    fn sole_pack(c: &BytecodeCompiler) -> &CapturePack {
        let packs: Vec<&CapturePack> = c
            .closure_capture_packs
            .iter()
            .filter(|p| p.len() == 1 && p.descriptors[0].name == "hits")
            .collect();
        assert_eq!(
            packs.len(),
            1,
            "fixture must produce exactly one `hits`-capturing closure"
        );
        packs[0]
    }

    /// Read the EMITTED artifact — `program.closure_function_layouts[fid]` —
    /// never the model's own table (R2).
    fn emitted(
        c: &BytecodeCompiler,
        pack: &CapturePack,
    ) -> std::sync::Arc<shape_value::v2::closure_layout::ClosureLayout> {
        c.program.closure_function_layouts[pack.closure as usize]
            .as_ref()
            .expect("closure has an emitted layout")
            .clone()
    }

    #[test]
    fn flagship_declared_move_over_read_only_let_mut_emits_owned_mutable() {
        let c = compile(FLAGSHIP_DECLARED);
        let pack = sole_pack(&c);
        let layout = emitted(&c, pack);

        // THE assertion: the EMITTED layout, not the plan.
        assert_eq!(
            layout.capture_storage_kind(0),
            CaptureKind::OwnedMutable,
            "the declared `move` over a `let mut` must reach the emitted ClosureLayout"
        );
        assert_eq!(
            layout.owned_mutable_capture_mask & 1,
            1,
            "owned_mutable_capture_mask bit must be set"
        );
        assert_eq!(layout.shared_capture_mask & 1, 0);
        // `hits: int` — the heap mask follows the TYPE, not the mode.
        assert_eq!(layout.heap_capture_mask & 1, 0);

        // The BODY must reach the capture through the owned-mutable cell, not
        // as a leading immutable param. This is the half C1 got wrong: it
        // flipped the layout mask while emission still read a param.
        let closure_fn = &c.program.functions[pack.closure as usize];
        assert_eq!(
            closure_fn.mutable_captures,
            vec![true],
            "`Function.mutable_captures` must say the body reads a cell"
        );
        assert_eq!(pack.descriptors[0].access, CaptureAccess::OwnedMutableCell);
        assert_eq!(pack.descriptors[0].declared, Some(CaptureMode::Move));

        // The owned-mutable capture OPCODES, not a `LoadLocal` of a leading
        // param slot. `capture_storage_kind` alone cannot prove this — the
        // rejected C1 branch passed a mask assertion and still emitted a param
        // read.
        let body = closure_body_opcodes(&c, pack.closure);
        assert!(
            body.iter().any(|op| format!("{op:?}").contains("OwnedMutable")),
            "closure body must emit Load/StoreOwnedMutableCapture; got {body:?}"
        );
    }

    /// SELF-TEST, executed: the same program WITHOUT the declaration (in
    /// ordinary source, where inference is legal) emits `Immutable` + a leading
    /// param. If this test and the one above ever agree, the declaration is
    /// being discarded — which is exactly rejection finding (1).
    #[test]
    fn negative_control_no_clause_infers_immutable_param() {
        let c = compile(FLAGSHIP_INFERRED_SOURCE);
        let pack = sole_pack(&c);
        let layout = emitted(&c, pack);

        assert_eq!(
            layout.capture_storage_kind(0),
            CaptureKind::Immutable,
            "inference over a READ-ONLY `let mut` picks Immutable — this is the \
             behaviour the declaration must be able to override"
        );
        assert_eq!(layout.owned_mutable_capture_mask & 1, 0);
        assert_eq!(pack.descriptors[0].access, CaptureAccess::Param);
        assert_eq!(pack.descriptors[0].declared, None);
        assert_eq!(
            c.program.functions[pack.closure as usize].mutable_captures,
            vec![false]
        );

        let body = closure_body_opcodes(&c, pack.closure);
        assert!(
            !body.iter().any(|op| format!("{op:?}").contains("OwnedMutable")),
            "the inferred closure must NOT emit owned-mutable capture opcodes; got {body:?}"
        );
    }

    /// The two halves of the pair, side by side, in ONE assertion — so a future
    /// edit cannot make them agree without deleting this line.
    #[test]
    fn declaration_changes_the_emitted_bytecode() {
        let declared = compile(FLAGSHIP_DECLARED);
        let inferred = compile(FLAGSHIP_INFERRED_SOURCE);
        let dk = emitted(&declared, sole_pack(&declared)).capture_storage_kind(0);
        let ik = emitted(&inferred, sole_pack(&inferred)).capture_storage_kind(0);
        assert_ne!(
            dk, ik,
            "a declared capture mode MUST produce different bytecode from inference; \
             identical output is the defect that got C1 rejected"
        );
        assert_eq!(dk, CaptureKind::OwnedMutable);
        assert_eq!(ik, CaptureKind::Immutable);
    }

    /// `share` over a `var` (ruling 2) — the declared word IS the kind. The
    /// `shared_capture_mask` bit is set on the EMITTED layout.
    ///
    /// The binding is a LOCAL `var`, not a module-level one, for a reason that
    /// is worth writing down: a generated `extend` method cannot reference a
    /// module-level binding AT ALL on main — `extend Job { method read(x: int)
    /// { x + total } }` fails with "Undefined variable: 'total'" with no closure
    /// anywhere in sight. That is a pre-existing scoping limitation of generated
    /// extend bodies, not something the declared-capture path introduces, and it
    /// is out of this ticket's territory. The module-binding arms of the ruling
    /// (`share` → Shared, `move` → [C0906]) are pinned at the `lower_declared`
    /// level instead, where the facts can be stated directly.
    #[test]
    fn declared_share_over_local_var_emits_shared() {
        let c = compile(
            r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method read(x: int) -> int \{ var total = 5
      let worker = |y: int; share total| y + total
      worker(x) \} \}")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
job.read(2)
"#,
        );
        let pack = c
            .closure_capture_packs
            .iter()
            .find(|p| p.descriptors.iter().any(|d| d.name == "total"))
            .expect("the `total`-capturing closure");
        let layout = emitted(&c, pack);
        assert_eq!(layout.capture_storage_kind(0), CaptureKind::Shared);
        assert_eq!(layout.shared_capture_mask & 1, 1);
        assert_eq!(layout.owned_mutable_capture_mask & 1, 0);
        assert_eq!(pack.descriptors[0].declared, Some(CaptureMode::Share));
        assert_eq!(pack.descriptors[0].access, CaptureAccess::SharedCell);
        assert!(matches!(
            pack.descriptors[0].target,
            Some(CaptureTarget::Local(_))
        ));

        // The body must reach it through the SHARED cell opcodes.
        let body = closure_body_opcodes(&c, pack.closure);
        assert!(
            body.iter()
                .any(|op| format!("{op:?}").contains("SharedCapture")),
            "closure body must emit Load/StoreSharedCapture; got {body:?}"
        );
    }

    /// #53 / slice 4: a Shared capture that is recaptured by a nested
    /// generated closure stays the SAME SharedCell. The outer closure's
    /// synthetic parameter is mechanically a by-value local, so checking only
    /// its ordinary binding semantics would reclassify it as OwnedMutable.
    /// The inner descriptor must instead carry the structural inherited-cell
    /// evidence from the outer pack, emit Shared again, and construct the
    /// nested closure without allocating a second cell.
    #[test]
    fn nested_declared_share_preserves_the_outer_cell_descriptor() {
        let c = compile(
            r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read() -> int { var total = 40
      let outer = |; share total| { let inner = |; share total| {
        total = total + 2
        total }
        inner()
        total }
      outer() } }")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
job.read()
"#,
        );
        let mut packs = c
            .closure_capture_packs
            .iter()
            .filter(|pack| pack.descriptors.len() == 1 && pack.descriptors[0].name == "total");
        let outer = packs.next().expect("outer total-capturing closure");
        let inner = packs.next().expect("inner total-capturing closure");
        assert!(packs.next().is_none(), "exactly two total capture packs");

        assert!(!outer.descriptors[0].inherited_shared_cell);
        assert!(
            inner.descriptors[0].inherited_shared_cell,
            "the inner plan must retain structural SharedCell evidence from the outer pack"
        );
        assert!(
            outer.descriptors[0].binding_span.is_some(),
            "the declaring binding must have authored span evidence"
        );
        assert_eq!(
            inner.descriptors[0].binding_span, outer.descriptors[0].binding_span,
            "the synthetic capture parameter must preserve the outer binding span by ordinal"
        );
        for pack in [outer, inner] {
            assert_eq!(pack.descriptors[0].declared, Some(CaptureMode::Share));
            assert_eq!(pack.descriptors[0].access, CaptureAccess::SharedCell);
            assert_eq!(
                emitted(&c, pack).capture_storage_kind(0),
                CaptureKind::Shared
            );
        }

        let outer_body = closure_body_opcodes(&c, outer.closure);
        assert!(
            !outer_body.contains(&crate::bytecode::OpCode::AllocSharedLocal),
            "recapturing an inherited Shared parameter must not allocate a second cell: \
             {outer_body:?}"
        );
        let inner_body = closure_body_opcodes(&c, inner.closure);
        assert!(
            inner_body
                .iter()
                .any(|opcode| format!("{opcode:?}").contains("SharedCapture")),
            "the nested body must access the inherited cell through Shared capture opcodes: \
             {inner_body:?}"
        );
    }

    #[path = "declared_tests/peek.rs"]
    mod peek;

    /// `move` over a `let` — `Immutable`, and when the value is a heap type the
    /// `heap_capture_mask` bit follows the TYPE, not the mode.
    #[test]
    fn declared_move_over_let_string_emits_immutable_with_heap_mask() {
        let c = compile(
            r#"
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read() -> string { let tag = \"hi\"
      let worker = |; move tag| tag
      worker() } }")
  }
}

@add_reader()
type Job { id: int }

let job = Job { id: 1 }
job.read()
"#,
        );
        let pack = c
            .closure_capture_packs
            .iter()
            .find(|p| p.descriptors.iter().any(|d| d.name == "tag"))
            .expect("the `tag`-capturing closure");
        let layout = emitted(&c, pack);
        assert_eq!(layout.capture_storage_kind(0), CaptureKind::Immutable);
        assert_eq!(layout.owned_mutable_capture_mask & 1, 0);
        assert_eq!(layout.shared_capture_mask & 1, 0);
        assert_eq!(
            layout.heap_capture_mask & 1,
            1,
            "a `string` capture is heap-refcounted regardless of the declared mode"
        );
        assert_eq!(pack.descriptors[0].declared, Some(CaptureMode::Move));
    }

    /// R3: the pack of a MONOMORPHIZED generated body is keyed by its own
    /// `func_idx` and carries the declared mode into the specialization. A
    /// span-keyed table collides here (generated AST parses from offset 0).
    #[test]
    fn declared_mode_survives_monomorphization() {
        let c = compile(
            r#"
annotation add_scaler() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method scale<T>(f: T) -> int \{ let mut hits = 3
      let worker = |x: T; move hits| hits
      worker(f) \} \}")
  }
}

@add_scaler()
type Job { id: int }

let job = Job { id: 1 }
job.scale(2)
"#,
        );
        let packs: Vec<&CapturePack> = c
            .closure_capture_packs
            .iter()
            .filter(|p| p.descriptors.iter().any(|d| d.name == "hits"))
            .collect();
        assert!(
            !packs.is_empty(),
            "the specialization must produce a `hits` pack"
        );
        for pack in packs {
            assert_eq!(pack.descriptors[0].declared, Some(CaptureMode::Move));
            assert_eq!(
                emitted(&c, pack).capture_storage_kind(0),
                CaptureKind::OwnedMutable,
                "the declaration must drive the SPECIALIZATION's layout too"
            );
        }
    }

    #[test]
    fn declared_move_requires_its_exact_storage_derived_artifact_kind() {
        let c = compile(FLAGSHIP_DECLARED);
        let pack = sole_pack(&c);
        let mut wrong = (*emitted(&c, pack)).clone();
        wrong.capture_kinds[0] = CaptureKind::Immutable;
        let function = &c.program.functions[usize::from(pack.closure)];
        let end = function.entry_point + function.body_length;
        let error = pack
            .validate_emitted_artifact(
                &wrong,
                function,
                &c.program.instructions[function.entry_point..end],
            )
            .expect_err("move over let mut is exactly OwnedMutable, not either move-like kind");
        assert!(error.contains("does not exactly match"), "{error}");
    }

    #[path = "declared_tests/rejections.rs"]
    mod rejections;

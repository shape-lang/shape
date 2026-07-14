use super::*;

    // ───────────────────────────────────────────────────────────────────
    // (c) SENTINEL — ONE PRODUCER. The single most load-bearing artifact of
    //     this ticket: it turns R2 from a code-review norm into a build
    //     failure. Mirrored in scripts/check-no-dynamic.sh.
    // ───────────────────────────────────────────────────────────────────

    fn walk_rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_rs_files(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn capture_kind_is_constructed_in_exactly_one_compiler_file() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/compiler");
        let mut files = Vec::new();
        walk_rs_files(&root, &mut files);
        assert!(!files.is_empty(), "compiler source tree must be walkable");

        let needles = [
            "CaptureKind::Immutable",
            "CaptureKind::OwnedMutable",
            "CaptureKind::Shared",
        ];
        let mut offenders: Vec<String> = Vec::new();
        for path in files {
            if path.file_name().and_then(|f| f.to_str()) == Some("capture_plan.rs") {
                continue;
            }
            let display = path.to_string_lossy();
            if display.contains("/capture_plan/") && display.contains("tests") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if needles.iter().any(|n| text.contains(n)) {
                offenders.push(path.display().to_string());
            }
        }
        assert!(
            offenders.is_empty(),
            "ADR-009 C1 K1 gate: `CaptureKind::<Variant>` may be named in exactly ONE \
             bytecode-compiler file (comptime_builtins/capture_plan.rs). A second producer \
             is how the declared capture mode gets discarded while inference stays \
             authoritative — the defect that got C1 rejected. Offenders: {offenders:?}"
        );
    }

fn stamped_program(origin: &shape_ast::ast::GeneratedNodeOrigin) -> shape_ast::ast::Program {
    let mut program = shape_ast::parse_program(
        "fn run() -> int { let base = 1; let f = || base; f() } run()",
    )
    .expect("fixture parses");
    let body = program
        .items
        .iter_mut()
        .find_map(|item| match item {
            shape_ast::ast::Item::Function(function, _) if function.name == "run" => {
                Some(&mut function.body)
            }
            _ => None,
        })
        .expect("fixture has run");
    shape_ast::transform::stamp_generated_closures(body, origin);
    program
}

#[test]
fn foreign_issuer_cannot_fabricate_generated_code_authority() {
    let foreign = shape_ast::ast::GeneratedNodeIssuer::new();
    let origin = foreign.issue(
        (7, 9),
        vec!["foreign".to_string()],
        0,
        shape_ast::ast::Span::DUMMY,
        "run".to_string(),
    );
    let program = stamped_program(&origin);
    let error = BytecodeCompiler::new()
        .compile_in_place(&program)
        .expect_err("foreign provenance must not authorize generated capture syntax");
    assert!(format!("{error:?}").contains("[C0909]"), "{error:?}");
}

#[test]
fn serde_round_trip_erases_current_compiler_authority() {
    let mut compiler = BytecodeCompiler::new();
    let origin = compiler.generated_node_issuer.issue(
        (7, 9),
        vec!["roundtrip".to_string()],
        0,
        shape_ast::ast::Span::DUMMY,
        "run".to_string(),
    );
    let program = stamped_program(&origin);
    let json = serde_json::to_string(&program).expect("program serializes");
    let decoded: shape_ast::ast::Program =
        serde_json::from_str(&json).expect("diagnostic provenance data round-trips");
    let error = compiler
        .compile_in_place(&decoded)
        .expect_err("round-tripped provenance must be non-authoritative");
    assert!(format!("{error:?}").contains("[C0909]"), "{error:?}");
}

#[test]
fn artifact_validation_rejects_wrong_length_and_missing_cell_opcodes() {
    let compiler = compile(
        r#"
fn run() -> int {
  let mut value = 1
  let bump = || { value = value + 1
    value }
  bump()
}
run()
"#,
    );
    let pack = compiler
        .closure_capture_packs
        .iter()
        .find(|pack| pack.descriptors.iter().any(|d| d.name == "value"))
        .expect("fixture has a value capture");
    let layout = compiler.program.closure_function_layouts[usize::from(pack.closure)]
        .as_ref()
        .expect("layout");
    let function = &compiler.program.functions[usize::from(pack.closure)];
    let end = function.entry_point + function.body_length;
    let window = &compiler.program.instructions[function.entry_point..end];

    let mut short = (**layout).clone();
    short.capture_kinds.clear();
    let length_error = pack
        .validate_emitted_artifact(&short, function, window)
        .expect_err("wrong-length layout must reject");
    assert!(length_error.contains("capture pack has"), "{length_error}");

    let mut wrong_index = pack.clone();
    wrong_index.descriptors[0].index = 1;
    let index_error = wrong_index
        .validate_emitted_artifact(layout, function, window)
        .expect_err("descriptor indices must be canonical and contiguous");
    assert!(index_error.contains("non-canonical index"), "{index_error}");

    let mut wrong_masks = (**layout).clone();
    wrong_masks.owned_mutable_capture_mask = 0;
    let mask_error = pack
        .validate_emitted_artifact(&wrong_masks, function, window)
        .expect_err("the emitted storage mask is part of the exact artifact");
    assert!(
        mask_error.contains("masks or aggregate geometry"),
        "{mask_error}"
    );

    let without_capture_ops = window
        .iter()
        .copied()
        .filter(|instruction| artifact::cell_capture_family(instruction.opcode).is_none())
        .collect::<Vec<_>>();
    let opcode_error = pack
        .validate_emitted_artifact(layout, function, &without_capture_ops)
        .expect_err("cell-backed descriptor requires its exact opcode family");
    assert!(opcode_error.contains("requires opcode family"), "{opcode_error}");
}

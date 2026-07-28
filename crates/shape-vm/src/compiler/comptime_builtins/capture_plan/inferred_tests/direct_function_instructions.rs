use super::*;

#[test]
fn nested_capture_opcodes_are_validated_only_in_their_direct_function_window() {
    let c = compile(
        r#"
fn run() -> int {
  let base = 40
  let outer = || {
    let mut total = 1
    let inner = || { total = total + 1
      total }
    base + inner()
  }
  outer()
}
run()
"#,
    );
    let outer = c
        .closure_capture_packs
        .iter()
        .find(|pack| {
            pack.descriptors
                .iter()
                .any(|descriptor| descriptor.name == "base")
        })
        .expect("outer base capture pack");
    let inner = c
        .closure_capture_packs
        .iter()
        .find(|pack| {
            pack.descriptors
                .iter()
                .any(|descriptor| descriptor.name == "total")
        })
        .expect("inner total capture pack");

    assert_eq!(outer.descriptors[0].index, 0);
    assert_eq!(inner.descriptors[0].index, 0);
    assert_eq!(outer.descriptors[0].access, CaptureAccess::Param);
    assert_eq!(inner.descriptors[0].access, CaptureAccess::OwnedMutableCell);

    let outer_function = &c.program.functions[usize::from(outer.closure)];
    let outer_end = outer_function.entry_point + outer_function.body_length;
    let raw_outer = &c.program.instructions[outer_function.entry_point..outer_end];
    assert!(
        raw_outer.iter().any(|instruction| {
            instruction.operand == Some(crate::bytecode::Operand::Local(0))
                && artifact::cell_capture_family(instruction.opcode)
                    == Some(artifact::CellCaptureFamily::OwnedMutable)
        }),
        "negative control: the physical outer span must contain the nested Local(0) cell opcode"
    );

    let direct_outer = c
        .program
        .direct_function_instructions(usize::from(outer.closure))
        .expect("outer direct window");
    assert!(
        direct_outer
            .into_iter()
            .all(|instruction| artifact::cell_capture_family(instruction.opcode).is_none()),
        "the outer Param capture must not observe the nested function's cell family"
    );

    let direct_inner = c
        .program
        .direct_function_instructions(usize::from(inner.closure))
        .expect("inner direct window");
    assert!(direct_inner.into_iter().any(|instruction| {
        instruction.operand == Some(crate::bytecode::Operand::Local(0))
            && artifact::cell_capture_family(instruction.opcode)
                == Some(artifact::CellCaptureFamily::OwnedMutable)
    }));

    let outer_layout = c.program.closure_function_layouts[usize::from(outer.closure)]
        .as_deref()
        .expect("published outer layout");
    outer
        .validate_emitted_artifact(
            outer_layout,
            outer_function,
            c.program
                .direct_function_instructions(usize::from(outer.closure))
                .expect("outer direct window remains valid"),
        )
        .expect("outer artifact validates without nested opcodes");

    let inner_function = &c.program.functions[usize::from(inner.closure)];
    let inner_layout = c.program.closure_function_layouts[usize::from(inner.closure)]
        .as_deref()
        .expect("published inner layout");
    inner
        .validate_emitted_artifact(
            inner_layout,
            inner_function,
            c.program
                .direct_function_instructions(usize::from(inner.closure))
                .expect("inner direct window remains valid"),
        )
        .expect("inner artifact retains its exact cell opcode family");
}

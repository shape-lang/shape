use super::*;

fn lower_parsed_function(source: &str) -> MirLoweringResult {
    let program = shape_ast::parser::parse_program(source).expect("parse f-string function");
    let function = match &program.items[0] {
        ast::Item::Function(function, _) => function,
        other => panic!("expected function, got {other:?}"),
    };
    lower_function_detailed(
        &function.name,
        &function.params,
        &function.body,
        function.name_span,
    )
}

fn assigned_rvalues(lowering: &MirLoweringResult) -> Vec<&Rvalue> {
    lowering
        .mir
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match &statement.kind {
            StatementKind::Assign(_, rvalue) => Some(rvalue),
            _ => None,
        })
        .collect()
}

#[test]
fn pure_expression_part_is_formatted_before_becoming_the_result() {
    let lowering = lower_parsed_function(r#"fn tag(value: int) -> string { f"{value}" }"#);
    let rvalues = assigned_rvalues(&lowering);
    assert_eq!(
        rvalues
            .iter()
            .filter(|rvalue| matches!(rvalue, Rvalue::FormatValue { .. }))
            .count(),
        1
    );
    assert!(rvalues.iter().any(|rvalue| matches!(
        rvalue,
        Rvalue::FormatValue {
            spec: MirFormatSpec::Default,
            ..
        }
    )));
}

#[test]
fn adjacent_expression_parts_are_each_formatted_before_concat() {
    let lowering = lower_parsed_function(r#"fn pair(a: int, b: bool) -> string { f"{a}{b}" }"#);
    let rvalues = assigned_rvalues(&lowering);
    assert_eq!(
        rvalues
            .iter()
            .filter(|rvalue| matches!(rvalue, Rvalue::FormatValue { .. }))
            .count(),
        2
    );
    assert!(
        rvalues
            .iter()
            .any(|rvalue| matches!(rvalue, Rvalue::BinaryOp(BinOp::Add, _, _)))
    );
}

#[test]
fn format_classes_survive_mir_lowering() {
    let fixed =
        lower_parsed_function(r#"fn render(value: number) -> string { f"{value:fixed(3)}" }"#);
    assert!(assigned_rvalues(&fixed).iter().any(|rvalue| matches!(
        rvalue,
        Rvalue::FormatValue {
            spec: MirFormatSpec::Fixed { precision: 3 },
            ..
        }
    )));

    let table = lower_parsed_function(r#"fn render(value: int) -> string { f"{value:table()}" }"#);
    assert!(assigned_rvalues(&table).iter().any(|rvalue| matches!(
        rvalue,
        Rvalue::FormatValue {
            spec: MirFormatSpec::Table,
            ..
        }
    )));

    let styled = lower_parsed_function(r#"fn render(value: int) { f"{value:bold}" }"#);
    assert!(assigned_rvalues(&styled).iter().any(|rvalue| matches!(
        rvalue,
        Rvalue::FormatValue {
            spec: MirFormatSpec::ContentStyle,
            ..
        }
    )));
}

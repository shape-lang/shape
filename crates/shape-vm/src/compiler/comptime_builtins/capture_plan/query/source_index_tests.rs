use shape_ast::ast::{Expr, Item, Program, Span};
use shape_runtime::visitor::{Visitor, walk_program};

use super::AuthoredCaptureIndex;

const DISTINCT_TYPED_CARRIERS: &str = r#"
comptime {
  extend RootTarget {
    method root_item_probe() -> int { 1 }
  }
}

mod nested {
  comptime {
    extend NestedTarget {
      method nested_item_probe() -> int { 2 }
    }
  }
}

let expression_value = comptime {
  extend ExpressionTarget {
    method expression_probe() -> int { 3 }
  }
  0
}
"#;

const NESTED_CARRIER_BOUNDARIES: &str = r#"
annotation nested_generators() {
  targets: [type]
  comptime post(target, ctx) {
    extend AnnotationOuter {
      method annotation_outer_probe() -> int { 1 }
    }
    let nested = comptime {
      extend AnnotationInner {
        method annotation_inner_probe() -> int { 2 }
      }
      0
    }
  }
}

comptime {
  extend ComptimeOuter {
    method comptime_outer_probe() -> int { 3 }
  }
  let nested = comptime {
    extend ComptimeInner {
      method comptime_inner_probe() -> int { 4 }
    }
    0
  }
}
"#;

#[test]
fn item_and_expression_comptime_carriers_keep_their_parser_spans() {
    let program = shape_ast::parse_program(DISTINCT_TYPED_CARRIERS).expect("fixture parses");
    let index = AuthoredCaptureIndex::build(&program);

    let root_span = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Comptime(_, span) => Some(*span),
            _ => None,
        })
        .expect("root comptime item");
    let nested_span = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Module(module, _) => module.items.iter().find_map(|item| match item {
                Item::Comptime(_, span) => Some(*span),
                _ => None,
            }),
            _ => None,
        })
        .expect("nested comptime item");
    let expression_spans = expression_comptime_spans(&program);
    let [expression_span] = expression_spans.as_slice() else {
        panic!("fixture has one expression comptime carrier")
    };

    assert_eq!(index.methods.len(), 3);
    assert_eq!(only_method_span(&index, "root_item_probe"), root_span);
    assert_eq!(only_method_span(&index, "nested_item_probe"), nested_span);
    assert_eq!(
        only_method_span(&index, "expression_probe"),
        *expression_span,
    );
    assert_ne!(root_span, nested_span);
    assert_ne!(root_span, *expression_span);
    assert_ne!(nested_span, *expression_span);
}

#[test]
fn nested_comptime_is_indexed_once_under_its_own_carrier() {
    let program = shape_ast::parse_program(NESTED_CARRIER_BOUNDARIES).expect("fixture parses");
    let index = AuthoredCaptureIndex::build(&program);

    let annotation_span = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::AnnotationDef(definition, _) => {
                definition.handlers.first().map(|handler| handler.span)
            }
            _ => None,
        })
        .expect("annotation handler carrier");
    let item_span = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Comptime(_, span) => Some(*span),
            _ => None,
        })
        .expect("item comptime carrier");
    let expression_spans = expression_comptime_spans(&program);
    let annotation_inner_span = span_containing(
        NESTED_CARRIER_BOUNDARIES,
        &expression_spans,
        "annotation_inner_probe",
    );
    let comptime_inner_span = span_containing(
        NESTED_CARRIER_BOUNDARIES,
        &expression_spans,
        "comptime_inner_probe",
    );

    assert_eq!(index.methods.len(), 4);
    assert_eq!(
        only_method_span(&index, "annotation_outer_probe"),
        annotation_span,
    );
    assert_eq!(
        only_method_span(&index, "annotation_inner_probe"),
        annotation_inner_span,
    );
    assert_eq!(only_method_span(&index, "comptime_outer_probe"), item_span,);
    assert_eq!(
        only_method_span(&index, "comptime_inner_probe"),
        comptime_inner_span,
    );
    assert_ne!(annotation_inner_span, annotation_span);
    assert_ne!(comptime_inner_span, item_span);
}

fn only_method_span(index: &AuthoredCaptureIndex, name: &str) -> Span {
    let matching: Vec<_> = index
        .methods
        .iter()
        .filter(|method| method.name == name)
        .collect();
    let [method] = matching.as_slice() else {
        panic!("expected exactly one indexed method named {name}")
    };
    method.generator_span
}

fn expression_comptime_spans(program: &Program) -> Vec<Span> {
    #[derive(Default)]
    struct Collector {
        spans: Vec<Span>,
    }

    impl Visitor for Collector {
        fn visit_expr_comptime(&mut self, expression: &Expr, span: Span) -> bool {
            if matches!(expression, Expr::Comptime(..)) {
                self.spans.push(span);
            }
            true
        }
    }

    let mut collector = Collector::default();
    walk_program(&mut collector, program);
    collector.spans
}

fn span_containing(source: &str, spans: &[Span], needle: &str) -> Span {
    let matching: Vec<_> = spans
        .iter()
        .copied()
        .filter(|span| source[span.start..span.end].contains(needle))
        .collect();
    let [span] = matching.as_slice() else {
        panic!("expected one comptime expression containing {needle}")
    };
    *span
}

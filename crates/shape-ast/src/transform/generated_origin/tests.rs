use super::*;
use crate::ast::{GeneratedExpansionFingerprint, GeneratedNodeIssuer, GeneratedNodePath, Span};
use crate::parse_program;

fn origin() -> GeneratedNodeOrigin {
    GeneratedNodeIssuer::new().issue(
        GeneratedExpansionFingerprint::from_components(
            0x1122_3344_5566_7788,
            0x0102_0304_0506_0708,
        ),
        GeneratedNodePath::decl_root("extend:Job").child("method:read"),
        7,
        Span { start: 10, end: 40 },
        "Job.read".to_string(),
    )
}

fn body_of(source: &str) -> Vec<Statement> {
    let program = parse_program(source).expect("fixture must parse");
    for item in &program.items {
        if let crate::ast::Item::Function(func, _) = item
            && func.name == "generated"
        {
            return func.body.clone();
        }
    }
    panic!("fixture must declare `fn generated`");
}

/// TOTALITY PROOF (see module docs). The oracle is serde's derived traversal,
/// so an unvisited closure remains visible as `"generated_origin":null`.
#[test]
fn stamp_is_total_over_every_syntactic_closure_position() {
    let source = r#"
fn generated(n: int) -> int {
  let direct = || 1
  let nested = || { let inner = || 2; inner() }
  let in_call = apply(|| 3)
  let in_method = xs.map(|x| x + 1)
  let in_array = [|| 4, || 5]
  let in_object = { a: || 6 }
  let in_struct = Holder { f: || 7 }
  let in_binary = pick(|| 8) + pick(|| 9)
  let in_unary = -pick(|| 10)
  let in_index = table[pick(|| 11)]
  let in_conditional = if n > 0 { pick(|| 12) } else { pick(|| 13) }
  let in_named_arg = call(cb: || 14)
  let in_spread = [...pick(|| 15)]
  let in_range = pick(|| 16)..pick(|| 17)
  let in_try = pick(|| 18)?
  let in_await = await pick(|| 19)
  let in_ref = &pick(|| 20)
  let in_match = match n { 0 => pick(|| 21), _ => pick(|| 22) }
  let in_block = { let b = || 23; b() }
  let in_property = pick(|| 24).field
  let in_comprehension = [pick(|| 25) for y in ys if guard(|| 26)]
  let in_cast = pick(|| 27) as int
  for item in pick(|| 29) {
    let in_for_body = || 30
    in_for_body()
  }
  while guard(|| 31) {
    let in_while_body = || 32
    in_while_body()
  }
  if guard(|| 33) {
    let in_if_body = || 34
    in_if_body()
  } else {
    let in_else_body = || 35
    in_else_body()
  }
  let in_pipe = xs |> map(|x| x)
  n
}
"#;
    let mut body = body_of(source);
    stamp_generated_closures(&mut body, &origin());

    let json = serde_json::to_string(&body).expect("body must serialize");
    assert!(json.contains("\"generated_origin\""));
    let unstamped = json.matches("\"generated_origin\":null").count();
    assert_eq!(
        unstamped, 0,
        "the walk missed {unstamped} closure node(s) in the generated capture gate"
    );
}

#[test]
fn stamp_data_survives_serde_round_trip() {
    let mut body = body_of("fn generated() -> int { let w = || 1; w() }");
    stamp_generated_closures(&mut body, &origin());
    let json = serde_json::to_string(&body).unwrap();
    let round_tripped: Vec<Statement> = serde_json::from_str(&json).unwrap();
    assert_eq!(round_tripped, body);
    assert_eq!(first_closure_origin(&round_tripped), Some(closure_path(0)));
}

#[test]
fn stamp_survives_the_emit_extend_payload_type() {
    let program =
        parse_program("extend Job { method read() -> int { let v = 41; let w = || v + 1; w() } }")
            .expect("fixture must parse");
    let mut extend = program
        .items
        .into_iter()
        .find_map(|item| match item {
            crate::ast::Item::Extend(extend, _) => Some(extend),
            _ => None,
        })
        .expect("fixture must declare an extend block");
    for method in &mut extend.methods {
        stamp_generated_closures(&mut method.body, &origin());
    }

    let payload = serde_json::to_string(&extend).expect("payload must serialize");
    let round_tripped: crate::ast::types::ExtendStatement =
        serde_json::from_str(&payload).expect("payload must parse back");
    assert_eq!(
        first_closure_origin(&round_tripped.methods[0].body),
        Some(closure_path(0))
    );
}

#[test]
fn absent_field_deserializes_as_ordinary_source() {
    let body = body_of("fn generated() -> int { let w = || 1; w() }");
    let json = serde_json::to_string(&body).unwrap();
    let stripped = json.replace(",\"generated_origin\":null", "");
    assert!(!stripped.contains("generated_origin"));
    let round_tripped: Vec<Statement> = serde_json::from_str(&stripped).unwrap();
    assert_eq!(first_closure_origin(&round_tripped), None);
}

#[test]
fn nested_closures_extend_the_parent_path_and_siblings_are_indexed() {
    let mut body = body_of(
        "fn generated() -> int { let a = || 1; let b = || { let c = || 2; c() }; a() + b() }",
    );
    stamp_generated_closures(&mut body, &origin());
    let mut paths = Vec::new();
    collect_paths(&body, &mut paths);
    assert_eq!(
        paths,
        vec![
            closure_path(0),
            closure_path(1),
            ["extend:Job", "method:read", "closure:1", "closure:0",]
                .map(str::to_string)
                .to_vec(),
        ]
    );
}

#[test]
fn stamping_is_idempotent() {
    let mut once = body_of(
        "fn generated() -> int { let a = || 1; let b = || { let c = || 2; c() }; a() + b() }",
    );
    stamp_generated_closures(&mut once, &origin());
    let mut twice = once.clone();
    stamp_generated_closures(&mut twice, &origin());
    assert_eq!(once, twice);
}

fn closure_path(index: u32) -> Vec<String> {
    vec![
        "extend:Job".to_string(),
        "method:read".to_string(),
        format!("closure:{index}"),
    ]
}

fn first_closure_origin(body: &[Statement]) -> Option<Vec<String>> {
    let mut paths = Vec::new();
    collect_paths(body, &mut paths);
    paths.into_iter().next()
}

fn collect_paths(body: &[Statement], out: &mut Vec<Vec<String>>) {
    let value = serde_json::to_value(body).unwrap();
    fn visit(value: &serde_json::Value, out: &mut Vec<Vec<String>>) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(path) = value.pointer("/FunctionExpr/generated_origin/node_path") {
                    out.push(
                        path.as_array()
                            .unwrap()
                            .iter()
                            .map(|segment| segment.as_str().unwrap().to_string())
                            .collect(),
                    );
                }
                for nested in map.values() {
                    visit(nested, out);
                }
            }
            serde_json::Value::Array(items) => {
                for nested in items {
                    visit(nested, out);
                }
            }
            _ => {}
        }
    }
    visit(&value, out);
}

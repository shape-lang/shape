use crate::ast::{
    CaptureMode, GeneratedExpansionFingerprint, GeneratedNodeIssuer, GeneratedNodePath, Item,
    Statement,
};
use crate::transform::{generated_closure_source_paths, stamp_generated_closures};

const SOURCE: &str = r#"
fn generated() {
  let first = |x: int; move seed| x + seed
  let outer = |; share total| {
    let inner = |; share total| total
    inner()
  }
  first(outer())
}
"#;

#[test]
fn source_path_enumerator_matches_the_canonical_stamper() {
    let mut body = function_body(SOURCE);
    let root = vec!["extend:Job".to_string(), "method:read".to_string()];
    let projected = generated_closure_source_paths(&body, &root);
    let indexed: Vec<_> = projected
        .iter()
        .map(|source| source.node_path.clone())
        .collect();
    let declared_captures: Vec<_> = projected
        .iter()
        .map(|source| {
            source.captures.as_ref().map(|clause| {
                clause
                    .entries
                    .iter()
                    .map(|entry| (entry.mode, entry.name.as_str()))
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    assert_eq!(
        declared_captures,
        [
            Some(vec![(CaptureMode::Move, "seed")]),
            Some(vec![(CaptureMode::Share, "total")]),
            Some(vec![(CaptureMode::Share, "total")]),
        ],
    );

    let issuer = GeneratedNodeIssuer::new();
    let origin = issuer.issue(
        GeneratedExpansionFingerprint::from_components(11, 29),
        GeneratedNodePath::try_from_rendered_segments(root)
            .expect("fixture root is a valid structural path"),
        0,
        crate::ast::Span::new(1, 2),
        "Job.read".to_string(),
    );
    stamp_generated_closures(&mut body, &origin);
    let stamped = stamped_paths(&body);

    assert_eq!(indexed, stamped);
    assert_eq!(
        indexed,
        [
            "extend:Job/method:read/closure:0",
            "extend:Job/method:read/closure:1",
            "extend:Job/method:read/closure:1/closure:0",
        ]
        .map(|path| path.split('/').map(str::to_string).collect::<Vec<_>>()),
    );
}

fn function_body(source: &str) -> Vec<Statement> {
    crate::parse_program(source)
        .expect("fixture parses")
        .items
        .into_iter()
        .find_map(|item| match item {
            Item::Function(function, _) if function.name == "generated" => Some(function.body),
            _ => None,
        })
        .expect("fixture function")
}

fn stamped_paths(body: &[Statement]) -> Vec<Vec<String>> {
    let json = serde_json::to_value(body).expect("body serializes");
    let mut paths = Vec::new();
    collect_stamped_paths(&json, &mut paths);
    paths
}

fn collect_stamped_paths(value: &serde_json::Value, paths: &mut Vec<Vec<String>>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::Object(origin)) = map.get("generated_origin")
                && let Some(serde_json::Value::Array(path)) = origin.get("node_path")
            {
                paths.push(
                    path.iter()
                        .map(|segment| segment.as_str().expect("path segment").to_string())
                        .collect(),
                );
            }
            for nested in map.values() {
                collect_stamped_paths(nested, paths);
            }
        }
        serde_json::Value::Array(values) => {
            for nested in values {
                collect_stamped_paths(nested, paths);
            }
        }
        _ => {}
    }
}

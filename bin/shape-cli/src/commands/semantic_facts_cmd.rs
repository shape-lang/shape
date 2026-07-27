//! `shape semantic-facts` — the compiler's consumer of the shared semantic
//! seam (ADR-011 §6, ADR-013 §1).
//!
//! The command reads facts from `shape-semantic-db` and prints them. It calls
//! `shape_semantic_db::callable_facts_for_source`, which is the same function
//! the language server calls, so the identity printed here and the identity
//! shown in an editor hover are the same value, not two values that happen to
//! agree.
//!
//! Scope is R16's stop line: this reports the resolved definition identity, the
//! normalized base contract, deterministic diagnostics and source provenance
//! for callables and call sites. It does not compile, annotate, or expand
//! anything.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use shape_semantic_db::{CallSiteFacts, CallableFacts, SemanticSession, unit_path_for_file};

pub async fn run_semantic_facts(
    script: PathBuf,
    callable: Option<String>,
    json: bool,
) -> Result<()> {
    let text = std::fs::read_to_string(&script)
        .with_context(|| format!("failed to read {}", script.display()))?;
    let unit_path = unit_path_for_file(Some(&script));

    let mut session = SemanticSession::new();
    session.insert_unit(&unit_path, &text);

    let names: Vec<(String, u32)> = match &callable {
        Some(name) => session
            .declared_callables(&unit_path)
            .into_iter()
            .filter(|(declared, _)| declared == name)
            .collect(),
        None => session.declared_callables(&unit_path),
    };

    if let Some(name) = &callable {
        if names.is_empty() {
            anyhow::bail!("`{name}` is not declared in {}", script.display());
        }
    }

    let facts: Vec<CallableFacts> = names
        .iter()
        .filter_map(|(name, ordinal)| session.declared_callable_facts(&unit_path, name, *ordinal))
        .collect();
    let call_sites: Vec<CallSiteFacts> = (0..session.call_site_count(&unit_path) as u32)
        .filter_map(|occurrence| session.call_site_facts(&unit_path, occurrence))
        .filter(|site| match &callable {
            Some(name) => &site.written_name == name,
            None => true,
        })
        .collect();

    if json {
        println!("{}", render_json(&unit_path, &script, &facts, &call_sites));
    } else {
        print!("{}", render_text(&unit_path, &facts, &call_sites));
    }
    Ok(())
}

fn render_text(unit_path: &str, facts: &[CallableFacts], call_sites: &[CallSiteFacts]) -> String {
    let mut out = format!("unit {unit_path}\n");
    for fact in facts {
        out.push_str(&format!("\n  {}\n", fact.contract().render(fact.name())));
        out.push_str(&format!("    definition : {}\n", fact.identity()));
        out.push_str(&format!("    facts      : {}\n", fact.content_identity()));
        out.push_str(&format!(
            "    provenance : bytes {}..{}\n",
            fact.provenance.declaration_span.start, fact.provenance.declaration_span.end
        ));
        for diagnostic in &fact.diagnostics {
            out.push_str(&format!(
                "    {} {:?}: {}\n",
                diagnostic.code,
                diagnostic.severity,
                diagnostic.message()
            ));
        }
    }
    for site in call_sites {
        out.push_str(&format!(
            "\n  call `{}` #{}\n",
            site.written_name, site.occurrence
        ));
        match &site.callee {
            Some(callee) => {
                out.push_str(&format!(
                    "    callee     : {} ({}::{})\n",
                    callee.identity, callee.declaring_unit, callee.name
                ));
            }
            None => out.push_str("    callee     : <unresolved>\n"),
        }
        for diagnostic in &site.diagnostics {
            out.push_str(&format!(
                "    {} {:?}: {}\n",
                diagnostic.code,
                diagnostic.severity,
                diagnostic.message()
            ));
        }
    }
    out
}

fn render_json(
    unit_path: &str,
    script: &Path,
    facts: &[CallableFacts],
    call_sites: &[CallSiteFacts],
) -> String {
    let callables: Vec<serde_json::Value> = facts
        .iter()
        .map(|fact| {
            serde_json::json!({
                "name": fact.name(),
                "definition_identity": fact.identity().to_hex(),
                "facts_content_identity": fact.content_identity().to_hex(),
                "contract": fact.contract().render(fact.name()),
                "result": fact.contract().result.render(),
                "params": fact.contract().params.iter().map(|param| serde_json::json!({
                    "name": param.name,
                    "type": param.ty.render(),
                })).collect::<Vec<_>>(),
                "provenance": {
                    "unit": fact.provenance.unit_path,
                    "declaration_span": [
                        fact.provenance.declaration_span.start,
                        fact.provenance.declaration_span.end,
                    ],
                },
                "diagnostics": fact.diagnostics.iter().map(diagnostic_json).collect::<Vec<_>>(),
            })
        })
        .collect();
    let sites: Vec<serde_json::Value> = call_sites
        .iter()
        .map(|site| {
            serde_json::json!({
                "written_name": site.written_name,
                "occurrence": site.occurrence,
                "callee_identity": site.callee_identity().map(|identity| identity.to_hex()),
                "callee_contract_identity": site
                    .callee_contract_identity
                    .map(|identity| identity.to_hex()),
                "diagnostics": site.diagnostics.iter().map(diagnostic_json).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "file": script.display().to_string(),
        "unit": unit_path,
        "callables": callables,
        "call_sites": sites,
    }))
    .expect("semantic facts serialize")
}

fn diagnostic_json(diagnostic: &shape_semantic_db::SemanticDiagnostic) -> serde_json::Value {
    serde_json::json!({
        "code": diagnostic.code,
        "severity": format!("{:?}", diagnostic.severity),
        "message": diagnostic.message(),
        "args": diagnostic.args().iter().cloned().collect::<std::collections::BTreeMap<_, _>>(),
        "span": diagnostic.span.map(|span| [span.start, span.end]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "fn add(a: int, b: int) -> int {\n    a + b\n}\n\nlet total = add(1, 2)\n";

    fn session_for(source: &str) -> (SemanticSession, String) {
        let unit_path = unit_path_for_file(Some(Path::new("/tmp/app/main.shape")));
        let mut session = SemanticSession::new();
        session.insert_unit(&unit_path, source);
        (session, unit_path)
    }

    #[test]
    fn the_compiler_side_projection_prints_the_published_identity() {
        let (session, unit_path) = session_for(SOURCE);
        let facts = session.callable_facts_of(&unit_path, "add").unwrap();
        let call = session.call_site_facts(&unit_path, 0).unwrap();
        let rendered = render_text(&unit_path, &[facts.clone()], &[call]);

        assert!(rendered.contains(&facts.identity().to_hex()), "{rendered}");
        assert!(
            rendered.contains(&facts.content_identity().to_hex()),
            "{rendered}"
        );
        assert!(
            rendered.contains("fn add(a: int, b: int) -> int"),
            "{rendered}"
        );
    }

    #[test]
    fn the_compiler_and_the_language_server_publish_one_identity() {
        // Both consumers reach the facts through the same seam entry point with
        // the same unit-path rule. This asserts the value they share, so a
        // future divergence — a second resolver, a different unit-path rule —
        // fails here.
        let script = Path::new("/tmp/app/main.shape");
        let compiler_facts = shape_semantic_db::callable_facts_for_source(
            &unit_path_for_file(Some(script)),
            SOURCE,
            "add",
        )
        .unwrap();
        let (session, unit_path) = session_for(SOURCE);
        let tooling_facts = session.callable_facts_of(&unit_path, "add").unwrap();
        assert_eq!(
            compiler_facts.content_identity(),
            tooling_facts.content_identity()
        );
    }

    #[test]
    fn json_projection_carries_the_same_identities() {
        let (session, unit_path) = session_for(SOURCE);
        let facts = session.callable_facts_of(&unit_path, "add").unwrap();
        let call = session.call_site_facts(&unit_path, 0).unwrap();
        let json = render_json(
            &unit_path,
            Path::new("/tmp/app/main.shape"),
            &[facts.clone()],
            &[call],
        );
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed["callables"][0]["definition_identity"],
            serde_json::Value::String(facts.identity().to_hex())
        );
        assert_eq!(
            parsed["call_sites"][0]["callee_identity"],
            serde_json::Value::String(facts.identity().to_hex())
        );
    }

    #[test]
    fn a_call_that_disagrees_with_the_published_contract_is_reported() {
        let (session, unit_path) =
            session_for("fn add(a: string, b: int) -> int {\n    b\n}\n\nlet t = add(1, 2)\n");
        let call = session.call_site_facts(&unit_path, 0).unwrap();
        let rendered = render_text(&unit_path, &[], &[call]);
        assert!(rendered.contains("SEMDB0011"), "{rendered}");
        assert!(
            rendered.contains("expects `string`, found `int`"),
            "{rendered}"
        );
    }
}

//! Native `regex` module for regular expression operations.
//!
//! Exports: regex.match_all, regex.replace, regex.replace_all,
//!          regex.is_match, regex.split
//!
//! Phase 2c migration: ported to the typed marshal layer.
//!
//! `regex.match` and `regex.find` (Option<Object> return) are deferred —
//! `TypedReturn::Some` takes a `ConcreteReturn` payload, and
//! `ConcreteReturn` is intentionally a leaf-only set (no recursive
//! `TypedObject` variant per the Concrete/Wrapper split). The strict-
//! typed answer needs either a flat `TypedReturn::SomeObjectPairs`
//! variant or a monomorphized TypedObject schema_id projection. Both
//! are marshal extensions tracked alongside the parser-cluster work.

use crate::marshal::{register_typed_fn_2, register_typed_fn_3};
use crate::module_exports::ModuleExports;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use std::sync::Arc;

/// Build a match-result row as a typed `(name, ConcreteReturn)` pair list.
/// Fields: text (string), start (int), end (int), groups (Array<string>).
fn match_to_pairs(m: &regex::Match, captures: &regex::Captures) -> Vec<(String, ConcreteReturn)> {
    let groups: Vec<String> = captures
        .iter()
        .skip(1)
        .map(|opt| match opt {
            Some(g) => g.as_str().to_string(),
            None => String::new(),
        })
        .collect();
    vec![
        ("text".to_string(), ConcreteReturn::String(m.as_str().to_string())),
        ("start".to_string(), ConcreteReturn::I64(m.start() as i64)),
        ("end".to_string(), ConcreteReturn::I64(m.end() as i64)),
        ("groups".to_string(), ConcreteReturn::ArrayString(groups)),
    ]
}

/// Create the `regex` module with regular expression functions.
pub fn create_regex_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::regex");
    module.description = "Regular expression matching and replacement".to_string();

    // regex.is_match(text: string, pattern: string) -> bool
    register_typed_fn_2::<_, Arc<String>, Arc<String>>(
        &mut module,
        "is_match",
        "Test whether the pattern matches anywhere in the text",
        [("text", "string"), ("pattern", "string")],
        ConcreteType::Bool,
        |text, pattern, _ctx| {
            let re = regex::Regex::new(pattern.as_str())
                .map_err(|e| format!("regex.is_match() invalid pattern: {}", e))?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Bool(re.is_match(text.as_str()))))
        },
    );

    // regex.match_all(text: string, pattern: string) -> Array<object>
    register_typed_fn_2::<_, Arc<String>, Arc<String>>(
        &mut module,
        "match_all",
        "Find all non-overlapping matches of the pattern",
        [("text", "string"), ("pattern", "string")],
        ConcreteType::ArrayObject("Array<object>".to_string()),
        |text, pattern, _ctx| {
            let re = regex::Regex::new(pattern.as_str())
                .map_err(|e| format!("regex.match_all() invalid pattern: {}", e))?;
            let matches: Vec<Vec<(String, ConcreteReturn)>> = re
                .captures_iter(text.as_str())
                .map(|caps| {
                    let m = caps.get(0).unwrap();
                    match_to_pairs(&m, &caps)
                })
                .collect();
            Ok(TypedReturn::ArrayObjectPairs(matches))
        },
    );

    // regex.replace(text: string, pattern: string, replacement: string) -> string
    register_typed_fn_3::<_, Arc<String>, Arc<String>, Arc<String>>(
        &mut module,
        "replace",
        "Replace the first match of the pattern with the replacement",
        [("text", "string"), ("pattern", "string"), ("replacement", "string")],
        ConcreteType::String,
        |text, pattern, replacement, _ctx| {
            let re = regex::Regex::new(pattern.as_str())
                .map_err(|e| format!("regex.replace() invalid pattern: {}", e))?;
            let result = re.replace(text.as_str(), replacement.as_str());
            Ok(TypedReturn::Concrete(ConcreteReturn::String(result.into_owned())))
        },
    );

    // regex.replace_all(text: string, pattern: string, replacement: string) -> string
    register_typed_fn_3::<_, Arc<String>, Arc<String>, Arc<String>>(
        &mut module,
        "replace_all",
        "Replace all matches of the pattern with the replacement",
        [("text", "string"), ("pattern", "string"), ("replacement", "string")],
        ConcreteType::String,
        |text, pattern, replacement, _ctx| {
            let re = regex::Regex::new(pattern.as_str())
                .map_err(|e| format!("regex.replace_all() invalid pattern: {}", e))?;
            let result = re.replace_all(text.as_str(), replacement.as_str());
            Ok(TypedReturn::Concrete(ConcreteReturn::String(result.into_owned())))
        },
    );

    // regex.split(text: string, pattern: string) -> Array<string>
    register_typed_fn_2::<_, Arc<String>, Arc<String>>(
        &mut module,
        "split",
        "Split the text at each match of the pattern",
        [("text", "string"), ("pattern", "string")],
        ConcreteType::ArrayString,
        |text, pattern, _ctx| {
            let re = regex::Regex::new(pattern.as_str())
                .map_err(|e| format!("regex.split() invalid pattern: {}", e))?;
            let parts: Vec<String> = re.split(text.as_str()).map(|s| s.to_string()).collect();
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayString(parts)))
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regex_module_creation() {
        let module = create_regex_module();
        assert_eq!(module.name, "std::core::regex");
        assert!(module.has_export("is_match"));
        assert!(module.has_export("match_all"));
        assert!(module.has_export("replace"));
        assert!(module.has_export("replace_all"));
        assert!(module.has_export("split"));
    }

    #[test]
    fn test_regex_schemas() {
        let module = create_regex_module();
        let split_schema = module.get_schema("split").unwrap();
        assert_eq!(split_schema.return_type.as_deref(), Some("Array<string>"));
    }

    // Behavioural tests removed — they used `module.invoke_export(&[ValueWord::...])`
    // which is the deleted dynamic-dispatch entry point. End-to-end coverage
    // belongs in `shape-test`'s integration suite.
}

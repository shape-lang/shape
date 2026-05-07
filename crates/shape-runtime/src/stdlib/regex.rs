//! Native `regex` module for regular expression operations.
//!
//! Exports: regex.match, regex.match_all, regex.find, regex.replace,
//!          regex.replace_all, regex.is_match, regex.split
//!
//! Phase 2c migration: ported to the typed marshal layer.
//! Phase 2d Cluster #4 (2026-05-07): regex.match and regex.find activated
//! using the new `TypedReturn::SomeObjectPairs` variant.

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

    // regex.match(text: string, pattern: string) -> Option<{text, start, end, groups}>
    register_typed_fn_2::<_, Arc<String>, Arc<String>>(
        &mut module,
        "match",
        "Find the first match of the pattern, returning Some({text, start, end, groups}) or None",
        [("text", "string"), ("pattern", "string")],
        ConcreteType::Option(Box::new(ConcreteType::Object)),
        |text, pattern, _ctx| {
            let re = regex::Regex::new(pattern.as_str())
                .map_err(|e| format!("regex.match() invalid pattern: {}", e))?;
            match re.captures(text.as_str()) {
                Some(caps) => {
                    let m = caps.get(0).unwrap();
                    Ok(TypedReturn::SomeObjectPairs(match_to_pairs(&m, &caps)))
                }
                None => Ok(TypedReturn::None),
            }
        },
    );

    // regex.find(text: string, pattern: string) -> Option<{text, start, end, groups}>
    //
    // Same shape as regex.match — kept as a separate name for the
    // historical "find first match" idiom in Shape user code.
    register_typed_fn_2::<_, Arc<String>, Arc<String>>(
        &mut module,
        "find",
        "Find the first match of the pattern (alias for regex.match)",
        [("text", "string"), ("pattern", "string")],
        ConcreteType::Option(Box::new(ConcreteType::Object)),
        |text, pattern, _ctx| {
            let re = regex::Regex::new(pattern.as_str())
                .map_err(|e| format!("regex.find() invalid pattern: {}", e))?;
            match re.captures(text.as_str()) {
                Some(caps) => {
                    let m = caps.get(0).unwrap();
                    Ok(TypedReturn::SomeObjectPairs(match_to_pairs(&m, &caps)))
                }
                None => Ok(TypedReturn::None),
            }
        },
    );

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
        assert!(module.has_export("match"));
        assert!(module.has_export("find"));
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

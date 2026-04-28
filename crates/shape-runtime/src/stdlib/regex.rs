//! Native `regex` module for regular expression operations.
//!
//! Exports: regex.match, regex.match_all, regex.replace, regex.replace_all,
//!          regex.is_match, regex.split

use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteType, TypedReturn, register_typed_function};
use shape_value::{ArgVec, ValueWord, ValueWordExt};
use std::sync::Arc;

/// Build a match result object as a ValueWord HashMap.
/// Fields: text (string), start (number), end (number), groups (array of strings).
fn match_to_nanboxed(m: &regex::Match, captures: &regex::Captures) -> ValueWord {
    let mut keys = Vec::with_capacity(4);
    let mut values = Vec::with_capacity(4);

    keys.push(ValueWord::from_string(Arc::new("text".to_string())));
    values.push(ValueWord::from_string(Arc::new(m.as_str().to_string())));

    keys.push(ValueWord::from_string(Arc::new("start".to_string())));
    values.push(ValueWord::from_f64(m.start() as f64));

    keys.push(ValueWord::from_string(Arc::new("end".to_string())));
    values.push(ValueWord::from_f64(m.end() as f64));

    let groups: ArgVec = ArgVec::from_vec(captures
        .iter()
        .skip(1)
        .map(|opt| match opt {
            Some(g) => ValueWord::from_string(Arc::new(g.as_str().to_string())),
            None => ValueWord::none(),
        })
        .collect());
    keys.push(ValueWord::from_string(Arc::new("groups".to_string())));
    values.push(ValueWord::from_array(shape_value::vmarray_from_vec(groups.into_inner())));

    ValueWord::from_hashmap_pairs(keys, values)
}

/// Create the `regex` module with regular expression functions.
pub fn create_regex_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::regex");
    module.description = "Regular expression matching and replacement".to_string();

    let text_pattern_params = || {
        vec![
            ModuleParam {
                name: "text".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Text to search".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "pattern".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Regular expression pattern".to_string(),
                ..Default::default()
            },
        ]
    };

    // regex.is_match(text: string, pattern: string) -> bool
    register_typed_function(
        &mut module,
        "is_match",
        "Test whether the pattern matches anywhere in the text",
        text_pattern_params(),
        ConcreteType::Bool,
        |args, _ctx| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "regex.is_match() requires a text string argument".to_string())?;

            let pattern = args
                .get(1)
                .and_then(|a| a.as_str())
                .ok_or_else(|| "regex.is_match() requires a pattern string argument".to_string())?;

            let re = regex::Regex::new(pattern)
                .map_err(|e| format!("regex.is_match() invalid pattern: {}", e))?;

            Ok(TypedReturn::Bool(re.is_match(text)))
        },
    );

    // regex.match(text: string, pattern: string) -> Option<object>
    register_typed_function(
        &mut module,
        "match",
        "Find the first match of the pattern, returning a match object or none",
        text_pattern_params(),
        ConcreteType::Option(Box::new(ConcreteType::Object)),
        |args, _ctx| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "regex.match() requires a text string argument".to_string())?;

            let pattern = args
                .get(1)
                .and_then(|a| a.as_str())
                .ok_or_else(|| "regex.match() requires a pattern string argument".to_string())?;

            let re = regex::Regex::new(pattern)
                .map_err(|e| format!("regex.match() invalid pattern: {}", e))?;

            match re.captures(text) {
                Some(caps) => {
                    let m = caps.get(0).unwrap();
                    let obj = match_to_nanboxed(&m, &caps);
                    Ok(TypedReturn::Some(Box::new(TypedReturn::ValueWord(obj))))
                }
                None => Ok(TypedReturn::None),
            }
        },
    );

    // regex.find(text, pattern) — alias for `match` (since `match` is a
    // keyword in Shape). The original installs without a schema; we mirror
    // that via register_typed_function and accept the auto-installed
    // schema as a small surface improvement.
    register_typed_function(
        &mut module,
        "find",
        "Alias for `match` (since `match` is a keyword in Shape)",
        text_pattern_params(),
        ConcreteType::Option(Box::new(ConcreteType::Object)),
        |args, _ctx| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "regex.find() requires a text string argument".to_string())?;
            let pattern = args
                .get(1)
                .and_then(|a| a.as_str())
                .ok_or_else(|| "regex.find() requires a pattern string argument".to_string())?;
            let re = regex::Regex::new(pattern)
                .map_err(|e| format!("regex.find() invalid pattern: {}", e))?;
            match re.captures(text) {
                Some(caps) => {
                    let m = caps.get(0).unwrap();
                    let obj = match_to_nanboxed(&m, &caps);
                    Ok(TypedReturn::Some(Box::new(TypedReturn::ValueWord(obj))))
                }
                None => Ok(TypedReturn::None),
            }
        },
    );

    // regex.match_all(text: string, pattern: string) -> Array<object>
    register_typed_function(
        &mut module,
        "match_all",
        "Find all non-overlapping matches of the pattern",
        text_pattern_params(),
        ConcreteType::ArrayObject("Array<object>".to_string()),
        |args, _ctx| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "regex.match_all() requires a text string argument".to_string())?;

            let pattern = args.get(1).and_then(|a| a.as_str()).ok_or_else(|| {
                "regex.match_all() requires a pattern string argument".to_string()
            })?;

            let re = regex::Regex::new(pattern)
                .map_err(|e| format!("regex.match_all() invalid pattern: {}", e))?;

            let matches: Vec<ValueWord> = re
                .captures_iter(text)
                .map(|caps| {
                    let m = caps.get(0).unwrap();
                    match_to_nanboxed(&m, &caps)
                })
                .collect();

            Ok(TypedReturn::ArrayValueWord(matches))
        },
    );

    let replace_params = || {
        vec![
            ModuleParam {
                name: "text".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Text to search".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "pattern".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Regular expression pattern".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "replacement".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Replacement string (supports $1, $2 for capture groups)".to_string(),
                ..Default::default()
            },
        ]
    };

    // regex.replace(text: string, pattern: string, replacement: string) -> string
    register_typed_function(
        &mut module,
        "replace",
        "Replace the first match of the pattern with the replacement",
        replace_params(),
        ConcreteType::String,
        |args, _ctx| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "regex.replace() requires a text string argument".to_string())?;

            let pattern = args
                .get(1)
                .and_then(|a| a.as_str())
                .ok_or_else(|| "regex.replace() requires a pattern string argument".to_string())?;

            let replacement = args.get(2).and_then(|a| a.as_str()).ok_or_else(|| {
                "regex.replace() requires a replacement string argument".to_string()
            })?;

            let re = regex::Regex::new(pattern)
                .map_err(|e| format!("regex.replace() invalid pattern: {}", e))?;

            let result = re.replace(text, replacement);
            Ok(TypedReturn::String(result.into_owned()))
        },
    );

    // regex.replace_all(text: string, pattern: string, replacement: string) -> string
    register_typed_function(
        &mut module,
        "replace_all",
        "Replace all matches of the pattern with the replacement",
        replace_params(),
        ConcreteType::String,
        |args, _ctx| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "regex.replace_all() requires a text string argument".to_string())?;

            let pattern = args.get(1).and_then(|a| a.as_str()).ok_or_else(|| {
                "regex.replace_all() requires a pattern string argument".to_string()
            })?;

            let replacement = args.get(2).and_then(|a| a.as_str()).ok_or_else(|| {
                "regex.replace_all() requires a replacement string argument".to_string()
            })?;

            let re = regex::Regex::new(pattern)
                .map_err(|e| format!("regex.replace_all() invalid pattern: {}", e))?;

            let result = re.replace_all(text, replacement);
            Ok(TypedReturn::String(result.into_owned()))
        },
    );

    // regex.split(text: string, pattern: string) -> Array<string>
    register_typed_function(
        &mut module,
        "split",
        "Split the text at each match of the pattern",
        vec![
            ModuleParam {
                name: "text".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Text to split".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "pattern".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Regular expression pattern to split on".to_string(),
                ..Default::default()
            },
        ],
        ConcreteType::ArrayString,
        |args, _ctx| {
            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "regex.split() requires a text string argument".to_string())?;

            let pattern = args
                .get(1)
                .and_then(|a| a.as_str())
                .ok_or_else(|| "regex.split() requires a pattern string argument".to_string())?;

            let re = regex::Regex::new(pattern)
                .map_err(|e| format!("regex.split() invalid pattern: {}", e))?;

            let parts: Vec<String> = re.split(text).map(|s| s.to_string()).collect();
            Ok(TypedReturn::ArrayString(parts))
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(val: &str) -> ValueWord {
        ValueWord::from_string(Arc::new(val.to_string()))
    }

    fn test_ctx() -> crate::module_exports::ModuleContext<'static> {
        let registry = Box::leak(Box::new(crate::type_schema::TypeSchemaRegistry::new()));
        crate::module_exports::ModuleContext {
            schemas: registry,
            invoke_callable: None,
            raw_invoker: None,
            function_hashes: None,
            vm_state: None,
            granted_permissions: None,
            scope_constraints: None,
            set_pending_resume: None,
            set_pending_frame_resume: None,
        }
    }

    #[test]
    fn test_regex_module_creation() {
        let module = create_regex_module();
        assert_eq!(module.name, "std::core::regex");
        assert!(module.has_export("is_match"));
        assert!(module.has_export("match"));
        assert!(module.has_export("match_all"));
        assert!(module.has_export("replace"));
        assert!(module.has_export("replace_all"));
        assert!(module.has_export("split"));
    }

    #[test]
    fn test_is_match_true() {
        let module = create_regex_module();
        let ctx = test_ctx();
        let f = module.get_export("is_match").unwrap();
        let result = f(&[s("hello world"), s(r"\bworld\b")], &ctx).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_is_match_false() {
        let module = create_regex_module();
        let ctx = test_ctx();
        let f = module.get_export("is_match").unwrap();
        let result = f(&[s("hello world"), s(r"^\d+$")], &ctx).unwrap();
        assert_eq!(result.as_bool(), Some(false));
    }

    #[test]
    fn test_is_match_invalid_pattern() {
        let module = create_regex_module();
        let ctx = test_ctx();
        let f = module.get_export("is_match").unwrap();
        assert!(f(&[s("text"), s("[invalid")], &ctx).is_err());
    }

    #[test]
    fn test_match_found() {
        let module = create_regex_module();
        let ctx = test_ctx();
        let f = module.get_export("match").unwrap();
        let result = f(&[s("abc 123 def"), s(r"(\d+)")], &ctx).unwrap();
        // Should be Some(match_object)
        let inner = result.as_some_inner().expect("should be Some");
        let (keys, values, _) = inner.as_hashmap().expect("should be hashmap");
        // Find "text" field
        let text_idx = keys
            .iter()
            .position(|k| k.as_str() == Some("text"))
            .unwrap();
        assert_eq!(values[text_idx].as_str(), Some("123"));
    }

    #[test]
    fn test_match_not_found() {
        let module = create_regex_module();
        let ctx = test_ctx();
        let f = module.get_export("match").unwrap();
        let result = f(&[s("abc def"), s(r"\d+")], &ctx).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_match_all() {
        let module = create_regex_module();
        let ctx = test_ctx();
        let f = module.get_export("match_all").unwrap();
        let result = f(&[s("a1 b2 c3"), s(r"\d")], &ctx).unwrap();
        let arr = result.as_any_array().expect("should be array").to_generic();
        assert_eq!(arr.len(), 3);
    }

    #[test]
    fn test_match_all_no_matches() {
        let module = create_regex_module();
        let ctx = test_ctx();
        let f = module.get_export("match_all").unwrap();
        let result = f(&[s("abc"), s(r"\d+")], &ctx).unwrap();
        let arr = result.as_any_array().expect("should be array").to_generic();
        assert_eq!(arr.len(), 0);
    }

    #[test]
    fn test_replace_first() {
        let module = create_regex_module();
        let ctx = test_ctx();
        let f = module.get_export("replace").unwrap();
        let result = f(&[s("foo bar foo"), s("foo"), s("baz")], &ctx).unwrap();
        assert_eq!(result.as_str(), Some("baz bar foo"));
    }

    #[test]
    fn test_replace_all() {
        let module = create_regex_module();
        let ctx = test_ctx();
        let f = module.get_export("replace_all").unwrap();
        let result = f(&[s("foo bar foo"), s("foo"), s("baz")], &ctx).unwrap();
        assert_eq!(result.as_str(), Some("baz bar baz"));
    }

    #[test]
    fn test_replace_with_capture_group() {
        let module = create_regex_module();
        let ctx = test_ctx();
        let f = module.get_export("replace_all").unwrap();
        let result = f(
            &[
                s("2024-01-15"),
                s(r"(\d{4})-(\d{2})-(\d{2})"),
                s("$3/$2/$1"),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_str(), Some("15/01/2024"));
    }

    #[test]
    fn test_split() {
        let module = create_regex_module();
        let ctx = test_ctx();
        let f = module.get_export("split").unwrap();
        let result = f(&[s("one,two,,three"), s(",")], &ctx).unwrap();
        let arr = result.as_any_array().expect("should be array").to_generic();
        assert_eq!(arr.len(), 4);
        assert_eq!(arr[0].as_str(), Some("one"));
        assert_eq!(arr[1].as_str(), Some("two"));
        assert_eq!(arr[2].as_str(), Some(""));
        assert_eq!(arr[3].as_str(), Some("three"));
    }

    #[test]
    fn test_split_by_whitespace() {
        let module = create_regex_module();
        let ctx = test_ctx();
        let f = module.get_export("split").unwrap();
        let result = f(&[s("hello   world  test"), s(r"\s+")], &ctx).unwrap();
        let arr = result.as_any_array().expect("should be array").to_generic();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_str(), Some("hello"));
        assert_eq!(arr[1].as_str(), Some("world"));
        assert_eq!(arr[2].as_str(), Some("test"));
    }

    #[test]
    fn test_regex_schemas() {
        let module = create_regex_module();

        let match_schema = module.get_schema("match").unwrap();
        assert_eq!(match_schema.params.len(), 2);
        assert_eq!(match_schema.return_type.as_deref(), Some("Option<object>"));

        let replace_schema = module.get_schema("replace").unwrap();
        assert_eq!(replace_schema.params.len(), 3);

        let split_schema = module.get_schema("split").unwrap();
        assert_eq!(split_schema.return_type.as_deref(), Some("Array<string>"));
    }
}

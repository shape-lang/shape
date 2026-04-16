//! Native `regex` module for regular expression operations.
//!
//! Exports: regex.match, regex.match_all, regex.replace, regex.replace_all,
//!          regex.is_match, regex.split

use crate::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use shape_value::{ValueWord, ValueWordExt};
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

    let groups: Vec<ValueWord> = captures
        .iter()
        .skip(1)
        .map(|opt| match opt {
            Some(g) => ValueWord::from_string(Arc::new(g.as_str().to_string())),
            None => ValueWord::none(),
        })
        .collect();
    keys.push(ValueWord::from_string(Arc::new("groups".to_string())));
    values.push(ValueWord::from_array(Arc::new(groups)));

    ValueWord::from_hashmap_pairs(keys, values)
}

/// Create the `regex` module with regular expression functions.
pub fn create_regex_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::regex");
    module.description = "Regular expression matching and replacement".to_string();

    // regex.is_match(text: string, pattern: string) -> bool
    module.add_function_with_schema(
        "is_match",
        |args: &[ValueWord], _ctx: &ModuleContext| {
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

            Ok(ValueWord::from_bool(re.is_match(text)))
        },
        ModuleFunction {
            description: "Test whether the pattern matches anywhere in the text".to_string(),
            params: vec![
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
            ],
            return_type: Some("bool".to_string()),
        },
    );

    // regex.match(text: string, pattern: string) -> Option<object>
    module.add_function_with_schema(
        "match",
        |args: &[ValueWord], _ctx: &ModuleContext| {
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
                    Ok(ValueWord::from_some(match_to_nanboxed(&m, &caps)))
                }
                None => Ok(ValueWord::none()),
            }
        },
        ModuleFunction {
            description: "Find the first match of the pattern, returning a match object or none"
                .to_string(),
            params: vec![
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
            ],
            return_type: Some("Option<object>".to_string()),
        },
    );

    // regex.find(text, pattern) — alias for `match` (since `match` is a keyword in Shape)
    module.add_function(
        "find",
        |args: &[ValueWord], _ctx: &ModuleContext| {
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
                    Ok(ValueWord::from_some(match_to_nanboxed(&m, &caps)))
                }
                None => Ok(ValueWord::none()),
            }
        },
    );

    // regex.match_all(text: string, pattern: string) -> Array<object>
    module.add_function_with_schema(
        "match_all",
        |args: &[ValueWord], _ctx: &ModuleContext| {
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

            Ok(ValueWord::from_array(Arc::new(matches)))
        },
        ModuleFunction {
            description: "Find all non-overlapping matches of the pattern".to_string(),
            params: vec![
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
            ],
            return_type: Some("Array<object>".to_string()),
        },
    );

    // regex.replace(text: string, pattern: string, replacement: string) -> string
    module.add_function_with_schema(
        "replace",
        |args: &[ValueWord], _ctx: &ModuleContext| {
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
            Ok(ValueWord::from_string(Arc::new(result.into_owned())))
        },
        ModuleFunction {
            description: "Replace the first match of the pattern with the replacement".to_string(),
            params: vec![
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
                    description: "Replacement string (supports $1, $2 for capture groups)"
                        .to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("string".to_string()),
        },
    );

    // regex.replace_all(text: string, pattern: string, replacement: string) -> string
    module.add_function_with_schema(
        "replace_all",
        |args: &[ValueWord], _ctx: &ModuleContext| {
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
            Ok(ValueWord::from_string(Arc::new(result.into_owned())))
        },
        ModuleFunction {
            description: "Replace all matches of the pattern with the replacement".to_string(),
            params: vec![
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
                    description: "Replacement string (supports $1, $2 for capture groups)"
                        .to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("string".to_string()),
        },
    );

    // regex.split(text: string, pattern: string) -> Array<string>
    module.add_function_with_schema(
        "split",
        |args: &[ValueWord], _ctx: &ModuleContext| {
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

            let parts: Vec<ValueWord> = re
                .split(text)
                .map(|s| ValueWord::from_string(Arc::new(s.to_string())))
                .collect();

            Ok(ValueWord::from_array(Arc::new(parts)))
        },
        ModuleFunction {
            description: "Split the text at each match of the pattern".to_string(),
            params: vec![
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
            return_type: Some("Array<string>".to_string()),
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

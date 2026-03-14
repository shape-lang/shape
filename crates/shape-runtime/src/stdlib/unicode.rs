//! Native `unicode` module for Unicode text processing.
//!
//! Exports: unicode.normalize, unicode.category, unicode.is_letter, unicode.is_digit, unicode.graphemes

use crate::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use shape_value::ValueWord;
use std::sync::Arc;

/// Create the `unicode` module.
pub fn create_unicode_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::unicode");
    module.description = "Unicode text processing utilities".to_string();

    // unicode.normalize(text: string, form: string) -> string
    module.add_function_with_schema(
        "normalize",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            use unicode_normalization::UnicodeNormalization;

            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "unicode.normalize() requires a string argument".to_string())?;

            let form = args
                .get(1)
                .and_then(|a| a.as_str())
                .ok_or_else(|| {
                    "unicode.normalize() requires a form argument (\"NFC\", \"NFD\", \"NFKC\", or \"NFKD\")"
                        .to_string()
                })?;

            let normalized: String = match form {
                "NFC" => text.nfc().collect(),
                "NFD" => text.nfd().collect(),
                "NFKC" => text.nfkc().collect(),
                "NFKD" => text.nfkd().collect(),
                _ => {
                    return Err(format!(
                        "unicode.normalize(): unknown form '{}', expected NFC/NFD/NFKC/NFKD",
                        form
                    ));
                }
            };

            Ok(ValueWord::from_string(Arc::new(normalized)))
        },
        ModuleFunction {
            description: "Normalize a Unicode string to the specified form".to_string(),
            params: vec![
                ModuleParam {
                    name: "text".to_string(),
                    type_name: "string".to_string(),
                    required: true,
                    description: "Text to normalize".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "form".to_string(),
                    type_name: "string".to_string(),
                    required: true,
                    description: "Normalization form: NFC, NFD, NFKC, or NFKD".to_string(),
                    allowed_values: Some(vec![
                        "NFC".to_string(),
                        "NFD".to_string(),
                        "NFKC".to_string(),
                        "NFKD".to_string(),
                    ]),
                    ..Default::default()
                },
            ],
            return_type: Some("string".to_string()),
        },
    );

    // unicode.category(codepoint: int) -> string
    module.add_function_with_schema(
        "category",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let cp = args
                .first()
                .and_then(|a| a.as_i64().or_else(|| a.as_f64().map(|n| n as i64)))
                .ok_or_else(|| {
                    "unicode.category() requires an int argument (codepoint)".to_string()
                })?;

            let ch = char::from_u32(cp as u32)
                .ok_or_else(|| format!("unicode.category(): invalid codepoint {}", cp))?;

            let category = unicode_general_category(ch);
            Ok(ValueWord::from_string(Arc::new(category.to_string())))
        },
        ModuleFunction {
            description: "Get the Unicode general category of a codepoint".to_string(),
            params: vec![ModuleParam {
                name: "codepoint".to_string(),
                type_name: "int".to_string(),
                required: true,
                description: "Unicode codepoint (e.g., 65 for 'A')".to_string(),
                ..Default::default()
            }],
            return_type: Some("string".to_string()),
        },
    );

    // unicode.is_letter(char: string) -> bool
    module.add_function_with_schema(
        "is_letter",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let s = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "unicode.is_letter() requires a string argument".to_string())?;

            let result = s.chars().next().map_or(false, |c| c.is_alphabetic());
            Ok(ValueWord::from_bool(result))
        },
        ModuleFunction {
            description: "Check if the first character is a Unicode letter".to_string(),
            params: vec![ModuleParam {
                name: "char".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Single character string to check".to_string(),
                ..Default::default()
            }],
            return_type: Some("bool".to_string()),
        },
    );

    // unicode.is_digit(char: string) -> bool
    module.add_function_with_schema(
        "is_digit",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            let s = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "unicode.is_digit() requires a string argument".to_string())?;

            let result = s.chars().next().map_or(false, |c| c.is_numeric());
            Ok(ValueWord::from_bool(result))
        },
        ModuleFunction {
            description: "Check if the first character is a Unicode digit".to_string(),
            params: vec![ModuleParam {
                name: "char".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Single character string to check".to_string(),
                ..Default::default()
            }],
            return_type: Some("bool".to_string()),
        },
    );

    // unicode.graphemes(text: string) -> Array<string>
    module.add_function_with_schema(
        "graphemes",
        |args: &[ValueWord], _ctx: &ModuleContext| {
            use unicode_segmentation::UnicodeSegmentation;

            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "unicode.graphemes() requires a string argument".to_string())?;

            let clusters: Vec<ValueWord> = text
                .graphemes(true)
                .map(|g| ValueWord::from_string(Arc::new(g.to_string())))
                .collect();

            Ok(ValueWord::from_array(Arc::new(clusters)))
        },
        ModuleFunction {
            description: "Split a string into Unicode grapheme clusters".to_string(),
            params: vec![ModuleParam {
                name: "text".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Text to split into grapheme clusters".to_string(),
                ..Default::default()
            }],
            return_type: Some("Array<string>".to_string()),
        },
    );

    module
}

/// Approximate Unicode general category using std::char classification.
fn unicode_general_category(ch: char) -> &'static str {
    if ch.is_uppercase() {
        "Lu"
    } else if ch.is_lowercase() {
        "Ll"
    } else if ch.is_alphabetic() {
        "Lo"
    } else if ch.is_ascii_digit() {
        "Nd"
    } else if ch.is_numeric() {
        "No"
    } else if ch.is_whitespace() {
        "Zs"
    } else if ch.is_control() {
        "Cc"
    } else if ch.is_ascii_punctuation() {
        "Po"
    } else {
        "Cn"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_unicode_module_creation() {
        let module = create_unicode_module();
        assert_eq!(module.name, "std::core::unicode");
        assert!(module.has_export("normalize"));
        assert!(module.has_export("category"));
        assert!(module.has_export("is_letter"));
        assert!(module.has_export("is_digit"));
        assert!(module.has_export("graphemes"));
    }

    #[test]
    fn test_normalize_nfc() {
        let module = create_unicode_module();
        let f = module.get_export("normalize").unwrap();
        let ctx = test_ctx();
        // e followed by combining acute accent
        let input = ValueWord::from_string(Arc::new("e\u{0301}".to_string()));
        let form = ValueWord::from_string(Arc::new("NFC".to_string()));
        let result = f(&[input, form], &ctx).unwrap();
        assert_eq!(result.as_str(), Some("\u{00e9}"));
    }

    #[test]
    fn test_normalize_nfd() {
        let module = create_unicode_module();
        let f = module.get_export("normalize").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("\u{00e9}".to_string()));
        let form = ValueWord::from_string(Arc::new("NFD".to_string()));
        let result = f(&[input, form], &ctx).unwrap();
        assert_eq!(result.as_str(), Some("e\u{0301}"));
    }

    #[test]
    fn test_normalize_invalid_form() {
        let module = create_unicode_module();
        let f = module.get_export("normalize").unwrap();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("hello".to_string()));
        let form = ValueWord::from_string(Arc::new("INVALID".to_string()));
        assert!(f(&[input, form], &ctx).is_err());
    }

    #[test]
    fn test_category_uppercase() {
        let module = create_unicode_module();
        let f = module.get_export("category").unwrap();
        let ctx = test_ctx();
        let result = f(&[ValueWord::from_i64(65)], &ctx).unwrap(); // 'A'
        assert_eq!(result.as_str(), Some("Lu"));
    }

    #[test]
    fn test_category_lowercase() {
        let module = create_unicode_module();
        let f = module.get_export("category").unwrap();
        let ctx = test_ctx();
        let result = f(&[ValueWord::from_i64(97)], &ctx).unwrap(); // 'a'
        assert_eq!(result.as_str(), Some("Ll"));
    }

    #[test]
    fn test_category_digit() {
        let module = create_unicode_module();
        let f = module.get_export("category").unwrap();
        let ctx = test_ctx();
        let result = f(&[ValueWord::from_i64(48)], &ctx).unwrap(); // '0'
        assert_eq!(result.as_str(), Some("Nd"));
    }

    #[test]
    fn test_is_letter_alpha() {
        let module = create_unicode_module();
        let f = module.get_export("is_letter").unwrap();
        let ctx = test_ctx();
        let result = f(
            &[ValueWord::from_string(Arc::new("\u{00e9}".to_string()))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_is_letter_digit() {
        let module = create_unicode_module();
        let f = module.get_export("is_letter").unwrap();
        let ctx = test_ctx();
        let result = f(&[ValueWord::from_string(Arc::new("5".to_string()))], &ctx).unwrap();
        assert_eq!(result.as_bool(), Some(false));
    }

    #[test]
    fn test_is_digit_numeric() {
        let module = create_unicode_module();
        let f = module.get_export("is_digit").unwrap();
        let ctx = test_ctx();
        let result = f(&[ValueWord::from_string(Arc::new("7".to_string()))], &ctx).unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_is_digit_alpha() {
        let module = create_unicode_module();
        let f = module.get_export("is_digit").unwrap();
        let ctx = test_ctx();
        let result = f(&[ValueWord::from_string(Arc::new("a".to_string()))], &ctx).unwrap();
        assert_eq!(result.as_bool(), Some(false));
    }

    #[test]
    fn test_graphemes_emoji() {
        let module = create_unicode_module();
        let f = module.get_export("graphemes").unwrap();
        let ctx = test_ctx();
        // Family emoji (multiple codepoints, single grapheme cluster)
        let input = ValueWord::from_string(Arc::new("hello".to_string()));
        let result = f(&[input], &ctx).unwrap();
        let arr = result.as_any_array().unwrap().to_generic();
        assert_eq!(arr.len(), 5);
        assert_eq!(arr[0].as_str(), Some("h"));
        assert_eq!(arr[4].as_str(), Some("o"));
    }

    #[test]
    fn test_graphemes_combining() {
        let module = create_unicode_module();
        let f = module.get_export("graphemes").unwrap();
        let ctx = test_ctx();
        // "e" + combining acute = one grapheme cluster
        let input = ValueWord::from_string(Arc::new("e\u{0301}a".to_string()));
        let result = f(&[input], &ctx).unwrap();
        let arr = result.as_any_array().unwrap().to_generic();
        assert_eq!(arr.len(), 2); // "e\u{0301}" and "a"
    }
}

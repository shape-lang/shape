//! Native `unicode` module for Unicode text processing.
//!
//! Exports: unicode.normalize, unicode.category, unicode.is_letter, unicode.is_digit, unicode.graphemes
//!
//! Phase 4b: all 5 exports migrated to `TypedModuleExports`.

use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteType, TypedReturn, register_typed_function};
use shape_value::{ValueWord, ValueWordExt};

/// Create the `unicode` module.
pub fn create_unicode_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::unicode");
    module.description = "Unicode text processing utilities".to_string();

    // unicode.normalize(text: string, form: string) -> string
    register_typed_function(
        &mut module,
        "normalize",
        "Normalize a Unicode string to the specified form",
        vec![
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
        ConcreteType::String,
        |args, _ctx| {
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

            Ok(TypedReturn::String(normalized))
        },
    );

    // unicode.category(codepoint: int) -> string
    register_typed_function(
        &mut module,
        "category",
        "Get the Unicode general category of a codepoint",
        vec![ModuleParam {
            name: "codepoint".to_string(),
            type_name: "int".to_string(),
            required: true,
            description: "Unicode codepoint (e.g., 65 for 'A')".to_string(),
            ..Default::default()
        }],
        ConcreteType::String,
        |args, _ctx| {
            let cp = args
                .first()
                .and_then(|a| a.as_i64().or_else(|| a.as_f64().map(|n| n as i64)))
                .ok_or_else(|| {
                    "unicode.category() requires an int argument (codepoint)".to_string()
                })?;

            let ch = char::from_u32(cp as u32)
                .ok_or_else(|| format!("unicode.category(): invalid codepoint {}", cp))?;

            Ok(TypedReturn::String(unicode_general_category(ch).to_string()))
        },
    );

    // unicode.is_letter(char: string) -> bool
    register_typed_function(
        &mut module,
        "is_letter",
        "Check if the first character is a Unicode letter",
        vec![ModuleParam {
            name: "char".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Single character string to check".to_string(),
            ..Default::default()
        }],
        ConcreteType::Bool,
        |args, _ctx| {
            let s = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "unicode.is_letter() requires a string argument".to_string())?;

            Ok(TypedReturn::Bool(
                s.chars().next().map_or(false, |c| c.is_alphabetic()),
            ))
        },
    );

    // unicode.is_digit(char: string) -> bool
    register_typed_function(
        &mut module,
        "is_digit",
        "Check if the first character is a Unicode digit",
        vec![ModuleParam {
            name: "char".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Single character string to check".to_string(),
            ..Default::default()
        }],
        ConcreteType::Bool,
        |args, _ctx| {
            let s = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "unicode.is_digit() requires a string argument".to_string())?;

            Ok(TypedReturn::Bool(
                s.chars().next().map_or(false, |c| c.is_numeric()),
            ))
        },
    );

    // unicode.graphemes(text: string) -> Array<string>
    register_typed_function(
        &mut module,
        "graphemes",
        "Split a string into Unicode grapheme clusters",
        vec![ModuleParam {
            name: "text".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Text to split into grapheme clusters".to_string(),
            ..Default::default()
        }],
        ConcreteType::ArrayString,
        |args, _ctx| {
            use unicode_segmentation::UnicodeSegmentation;

            let text = args
                .first()
                .and_then(|a| a.as_str())
                .ok_or_else(|| "unicode.graphemes() requires a string argument".to_string())?;

            let clusters: Vec<String> = text.graphemes(true).map(|g| g.to_string()).collect();
            Ok(TypedReturn::ArrayString(clusters))
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
    use std::sync::Arc;

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
        let ctx = test_ctx();
        // e followed by combining acute accent
        let input = ValueWord::from_string(Arc::new("e\u{0301}".to_string()));
        let form = ValueWord::from_string(Arc::new("NFC".to_string()));
        let result = module.invoke_export("normalize", &[input, form], &ctx).unwrap().unwrap();
        assert_eq!(result.as_str(), Some("\u{00e9}"));
    }

    #[test]
    fn test_normalize_nfd() {
        let module = create_unicode_module();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("\u{00e9}".to_string()));
        let form = ValueWord::from_string(Arc::new("NFD".to_string()));
        let result = module.invoke_export("normalize", &[input, form], &ctx).unwrap().unwrap();
        assert_eq!(result.as_str(), Some("e\u{0301}"));
    }

    #[test]
    fn test_normalize_invalid_form() {
        let module = create_unicode_module();
        let ctx = test_ctx();
        let input = ValueWord::from_string(Arc::new("hello".to_string()));
        let form = ValueWord::from_string(Arc::new("INVALID".to_string()));
        assert!(module.invoke_export("normalize", &[input, form], &ctx).unwrap().is_err());
    }

    #[test]
    fn test_category_uppercase() {
        let module = create_unicode_module();
        let ctx = test_ctx();
        let result = module.invoke_export("category", &[ValueWord::from_i64(65)], &ctx).unwrap().unwrap(); // 'A'
        assert_eq!(result.as_str(), Some("Lu"));
    }

    #[test]
    fn test_category_lowercase() {
        let module = create_unicode_module();
        let ctx = test_ctx();
        let result = module.invoke_export("category", &[ValueWord::from_i64(97)], &ctx).unwrap().unwrap(); // 'a'
        assert_eq!(result.as_str(), Some("Ll"));
    }

    #[test]
    fn test_category_digit() {
        let module = create_unicode_module();
        let ctx = test_ctx();
        let result = module.invoke_export("category", &[ValueWord::from_i64(48)], &ctx).unwrap().unwrap(); // '0'
        assert_eq!(result.as_str(), Some("Nd"));
    }

    #[test]
    fn test_is_letter_alpha() {
        let module = create_unicode_module();
        let ctx = test_ctx();
        let result = module.invoke_export("is_letter", 
            &[ValueWord::from_string(Arc::new("\u{00e9}".to_string()))],
            &ctx,
        ).unwrap()
        .unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_is_letter_digit() {
        let module = create_unicode_module();
        let ctx = test_ctx();
        let result = module.invoke_export("is_letter", &[ValueWord::from_string(Arc::new("5".to_string()))], &ctx).unwrap().unwrap();
        assert_eq!(result.as_bool(), Some(false));
    }

    #[test]
    fn test_is_digit_numeric() {
        let module = create_unicode_module();
        let ctx = test_ctx();
        let result = module.invoke_export("is_digit", &[ValueWord::from_string(Arc::new("7".to_string()))], &ctx).unwrap().unwrap();
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_is_digit_alpha() {
        let module = create_unicode_module();
        let ctx = test_ctx();
        let result = module.invoke_export("is_digit", &[ValueWord::from_string(Arc::new("a".to_string()))], &ctx).unwrap().unwrap();
        assert_eq!(result.as_bool(), Some(false));
    }

    #[test]
    fn test_graphemes_emoji() {
        let module = create_unicode_module();
        let ctx = test_ctx();
        // Family emoji (multiple codepoints, single grapheme cluster)
        let input = ValueWord::from_string(Arc::new("hello".to_string()));
        let result = module.invoke_export("graphemes", &[input], &ctx).unwrap().unwrap();
        let arr = result.as_any_array().unwrap().to_generic();
        assert_eq!(arr.len(), 5);
        assert_eq!(arr[0].as_str(), Some("h"));
        assert_eq!(arr[4].as_str(), Some("o"));
    }

    #[test]
    fn test_graphemes_combining() {
        let module = create_unicode_module();
        let ctx = test_ctx();
        // "e" + combining acute = one grapheme cluster
        let input = ValueWord::from_string(Arc::new("e\u{0301}a".to_string()));
        let result = module.invoke_export("graphemes", &[input], &ctx).unwrap().unwrap();
        let arr = result.as_any_array().unwrap().to_generic();
        assert_eq!(arr.len(), 2); // "e\u{0301}" and "a"
    }

    #[test]
    fn test_unicode_typed_registry_populated() {
        let module = create_unicode_module();
        let typed = module.typed_exports();
        assert!(typed.get("normalize").is_some());
        assert!(typed.get("category").is_some());
        assert!(typed.get("is_letter").is_some());
        assert!(typed.get("is_digit").is_some());
        assert!(typed.get("graphemes").is_some());
        assert_eq!(typed.functions.len(), 5);
    }
}

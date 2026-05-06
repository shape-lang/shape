//! Native `unicode` module for Unicode text processing.
//!
//! Exports: unicode.normalize, unicode.category, unicode.is_letter, unicode.is_digit, unicode.graphemes
//!
//! Phase 2c: migrated to the typed marshal layer
//! (`crate::marshal::register_typed_fn_N`). Native function bodies take
//! typed Rust args via [`crate::marshal::FromSlot`]; their Rust signatures
//! *are* the typed signatures. The Rust trait system rejects registration
//! whose body's parameter types don't match.

use crate::marshal::{register_typed_fn_1, register_typed_fn_2};
use crate::module_exports::ModuleExports;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use std::sync::Arc;

/// Create the `unicode` module.
pub fn create_unicode_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::unicode");
    module.description = "Unicode text processing utilities".to_string();

    // unicode.normalize(text: string, form: string) -> string
    register_typed_fn_2::<_, Arc<String>, Arc<String>>(
        &mut module,
        "normalize",
        "Normalize a Unicode string to the specified form",
        [("text", "string"), ("form", "string")],
        ConcreteType::String,
        |text, form, _ctx| {
            use unicode_normalization::UnicodeNormalization;

            let normalized: String = match form.as_str() {
                "NFC" => text.nfc().collect(),
                "NFD" => text.nfd().collect(),
                "NFKC" => text.nfkc().collect(),
                "NFKD" => text.nfkd().collect(),
                _ => {
                    return Err(format!(
                        "unicode.normalize(): unknown form '{}', expected NFC/NFD/NFKC/NFKD",
                        form.as_str()
                    ));
                }
            };

            Ok(TypedReturn::Concrete(ConcreteReturn::String(normalized)))
        },
    );

    // unicode.category(codepoint: int) -> string
    register_typed_fn_1::<_, i64>(
        &mut module,
        "category",
        "Get the Unicode general category of a codepoint",
        "codepoint",
        "int",
        ConcreteType::String,
        |cp, _ctx| {
            let ch = char::from_u32(cp as u32)
                .ok_or_else(|| format!("unicode.category(): invalid codepoint {}", cp))?;

            Ok(TypedReturn::Concrete(ConcreteReturn::String(
                unicode_general_category(ch).to_string(),
            )))
        },
    );

    // unicode.is_letter(char: string) -> bool
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "is_letter",
        "Check if the first character is a Unicode letter",
        "char",
        "string",
        ConcreteType::Bool,
        |s, _ctx| {
            Ok(TypedReturn::Concrete(ConcreteReturn::Bool(
                s.chars().next().map_or(false, |c| c.is_alphabetic()),
            )))
        },
    );

    // unicode.is_digit(char: string) -> bool
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "is_digit",
        "Check if the first character is a Unicode digit",
        "char",
        "string",
        ConcreteType::Bool,
        |s, _ctx| {
            Ok(TypedReturn::Concrete(ConcreteReturn::Bool(
                s.chars().next().map_or(false, |c| c.is_numeric()),
            )))
        },
    );

    // unicode.graphemes(text: string) -> Array<string>
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "graphemes",
        "Split a string into Unicode grapheme clusters",
        "text",
        "string",
        ConcreteType::ArrayString,
        |text, _ctx| {
            use unicode_segmentation::UnicodeSegmentation;

            let clusters: Vec<String> = text.graphemes(true).map(|g| g.to_string()).collect();
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayString(clusters)))
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

    // Behavioural invocation tests removed — they used `module.invoke_export`
    // with `ValueWord` arrays, which is the deleted dynamic-dispatch entry
    // point. Behaviour is now covered through typed-slot dispatch via the
    // marshal layer. End-to-end tests live in `shape-test`'s integration
    // suite once the strict-typed cascade reaches shape-vm.
}

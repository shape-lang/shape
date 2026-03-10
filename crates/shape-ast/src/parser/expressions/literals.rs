//! Literal expression parsing
//!
//! This module handles parsing of literal values:
//! - Numbers, strings, booleans, None (Option)
//! - Colors and timeframes
//! - Array literals
//! - Object literals

use crate::ast::{Expr, Literal, Timeframe};
use crate::error::{Result, ShapeError};
use crate::int_width::IntWidth;
use crate::parser::string_literals::parse_string_literal_with_kind;
use crate::parser::{Rule, pair_location};
use pest::iterators::Pair;

use super::super::pair_span;

/// Parse a literal value
pub fn parse_literal(pair: Pair<Rule>) -> Result<Expr> {
    let pair_loc = pair_location(&pair);
    let span = pair_span(&pair);
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| ShapeError::ParseError {
            message: "expected literal value".to_string(),
            location: Some(pair_loc.clone()),
        })?;

    let literal = match inner.as_rule() {
        Rule::decimal => {
            let dec_str = inner.as_str();
            // Remove the 'D' suffix and parse as Decimal
            let num_part = dec_str.trim_end_matches('D');
            use rust_decimal::Decimal;
            Literal::Decimal(
                num_part
                    .parse::<Decimal>()
                    .map_err(|e| ShapeError::ParseError {
                        message: format!("Invalid decimal: {}", e),
                        location: None,
                    })?,
            )
        }
        Rule::percent_literal => {
            let pct_str = inner.as_str().trim_end_matches('%');
            let value: f64 = pct_str.parse().map_err(|e| ShapeError::ParseError {
                message: format!("Invalid percent literal: {}", e),
                location: None,
            })?;
            Literal::Number(value / 100.0)
        }
        Rule::number => {
            let num_str = inner.as_str();
            // Check for hex/binary/octal prefixes
            let stripped = num_str.trim_start_matches('-');
            let is_negative = num_str.starts_with('-');
            if stripped.starts_with("0x") || stripped.starts_with("0X") {
                parse_prefixed_int(num_str, stripped, 16, 2, is_negative, &pair_loc)?
            } else if stripped.starts_with("0b") || stripped.starts_with("0B") {
                parse_prefixed_int(num_str, stripped, 2, 2, is_negative, &pair_loc)?
            } else if stripped.starts_with("0o") || stripped.starts_with("0O") {
                parse_prefixed_int(num_str, stripped, 8, 2, is_negative, &pair_loc)?
            } else if let Some(lit) = try_parse_suffixed_int(num_str, &pair_loc)? {
                // Check for integer width suffix
                lit
            } else if num_str.contains('.') || num_str.contains('e') || num_str.contains('E') {
                // Fraction or exponent → f64
                Literal::Number(num_str.parse().map_err(|e| ShapeError::ParseError {
                    message: format!("Invalid number: {}", e),
                    location: None,
                })?)
            } else {
                // Plain integer (no suffix, no decimal)
                Literal::Int(num_str.parse().map_err(|e| ShapeError::ParseError {
                    message: format!("Invalid integer: {}", e),
                    location: None,
                })?)
            }
        }
        Rule::string => {
            let parsed = parse_string_literal_with_kind(inner.as_str())?;
            if let Some(mode) = parsed.interpolation_mode {
                if parsed.is_content {
                    Literal::ContentString {
                        value: parsed.value,
                        mode,
                    }
                } else {
                    Literal::FormattedString {
                        value: parsed.value,
                        mode,
                    }
                }
            } else {
                Literal::String(parsed.value)
            }
        }
        Rule::boolean => Literal::Bool(inner.as_str() == "true"),
        Rule::none_literal => Literal::None,

        Rule::char_literal => {
            let raw = inner.as_str();
            // Strip surrounding quotes: 'x' -> x
            let inner_str = &raw[1..raw.len() - 1];
            let c = parse_char_literal_inner(inner_str).map_err(|msg| ShapeError::ParseError {
                message: msg,
                location: Some(pair_loc.clone()),
            })?;
            Literal::Char(c)
        }
        Rule::timeframe => {
            let tf = Timeframe::parse(inner.as_str()).ok_or_else(|| ShapeError::ParseError {
                message: format!("Invalid timeframe: {}", inner.as_str()),
                location: None,
            })?;
            Literal::Timeframe(tf)
        }
        _ => {
            return Err(ShapeError::ParseError {
                message: format!("Unexpected literal: {:?}", inner.as_rule()),
                location: None,
            });
        }
    };

    Ok(Expr::Literal(literal, span))
}

/// Parse the inner content of a char literal (after stripping quotes).
fn parse_char_literal_inner(s: &str) -> std::result::Result<char, String> {
    if s.is_empty() {
        return Err("Empty char literal".to_string());
    }
    if s.starts_with('\\') {
        if s.starts_with("\\u{") && s.ends_with('}') {
            // Unicode escape: \u{XXXX}
            let hex = &s[3..s.len() - 1];
            let code = u32::from_str_radix(hex, 16)
                .map_err(|_| format!("Invalid unicode escape: {}", s))?;
            char::from_u32(code)
                .ok_or_else(|| format!("Invalid unicode code point: U+{:04X}", code))
        } else if s.len() == 2 {
            // Simple escape: \n, \t, \r, \\, \', \0
            match s.as_bytes()[1] {
                b'n' => Ok('\n'),
                b't' => Ok('\t'),
                b'r' => Ok('\r'),
                b'\\' => Ok('\\'),
                b'\'' => Ok('\''),
                b'0' => Ok('\0'),
                other => Err(format!("Unknown escape sequence: \\{}", other as char)),
            }
        } else {
            Err(format!("Invalid escape sequence: {}", s))
        }
    } else {
        let mut chars = s.chars();
        let c = chars
            .next()
            .ok_or_else(|| "Empty char literal".to_string())?;
        if chars.next().is_some() {
            return Err(format!(
                "Char literal must be a single character, got: {}",
                s
            ));
        }
        Ok(c)
    }
}

/// Parse an array literal
pub fn parse_array_literal(pair: Pair<Rule>) -> Result<Expr> {
    let mut elements = Vec::new();
    let span = pair_span(&pair);

    // Check if we have any inner pairs (empty array case)
    let inner_pairs: Vec<_> = pair.into_inner().collect();

    // Parse each element in the array
    for inner_pair in inner_pairs {
        match inner_pair.as_rule() {
            Rule::array_elements => {
                // Parse the array_elements node
                for element_pair in inner_pair.into_inner() {
                    match element_pair.as_rule() {
                        Rule::array_element => {
                            // Parse each array_element
                            let elem_loc = pair_location(&element_pair);
                            let mut elem_inner = element_pair.into_inner();
                            let elem = elem_inner.next().ok_or_else(|| ShapeError::ParseError {
                                message: "expected array element content".to_string(),
                                location: Some(elem_loc.clone()),
                            })?;
                            match elem.as_rule() {
                                Rule::spread_element => {
                                    // Parse the expression inside the spread
                                    let elem_span = pair_span(&elem);
                                    let spread_inner =
                                        elem.into_inner().next().ok_or_else(|| {
                                            ShapeError::ParseError {
                                                message:
                                                    "expected expression after '...' in spread"
                                                        .to_string(),
                                                location: Some(elem_loc),
                                            }
                                        })?;
                                    let spread_expr = super::parse_expression(spread_inner)?;
                                    elements.push(Expr::Spread(Box::new(spread_expr), elem_span));
                                }
                                _ => {
                                    elements.push(super::parse_expression(elem)?);
                                }
                            }
                        }
                        _ => {
                            return Err(ShapeError::ParseError {
                                message: format!(
                                    "Unexpected rule in array_elements: {:?}",
                                    element_pair.as_rule()
                                ),
                                location: None,
                            });
                        }
                    }
                }
            }
            Rule::list_comprehension => {
                // List comprehensions are parsed as the array_literal alternative in grammar
                // This branch handles when they appear inside an array literal context
                return super::comprehensions::parse_list_comprehension(inner_pair);
            }
            _ => {
                // This shouldn't happen for array literals
                return Err(ShapeError::ParseError {
                    message: format!(
                        "Unexpected rule in array literal: {:?}",
                        inner_pair.as_rule()
                    ),
                    location: None,
                });
            }
        }
    }

    Ok(Expr::Array(elements, span))
}

/// Parse an object literal
pub fn parse_object_literal(pair: Pair<Rule>) -> Result<Expr> {
    use crate::ast::ObjectEntry;
    use crate::parser::types::parse_type_annotation;

    let mut entries = Vec::new();
    let span = pair_span(&pair);

    // Parse object fields if present
    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::object_fields => {
                for field_item_pair in inner_pair.into_inner() {
                    match field_item_pair.as_rule() {
                        Rule::object_field_item => {
                            let field_item_loc = pair_location(&field_item_pair);
                            let field_item_inner =
                                field_item_pair.into_inner().next().ok_or_else(|| {
                                    ShapeError::ParseError {
                                        message: "expected object field content".to_string(),
                                        location: Some(field_item_loc.clone()),
                                    }
                                })?;
                            match field_item_inner.as_rule() {
                                Rule::object_field => {
                                    let field_loc = pair_location(&field_item_inner);
                                    let mut field_inner = field_item_inner.into_inner();
                                    let field_kind = field_inner.next().ok_or_else(|| {
                                        ShapeError::ParseError {
                                            message: "expected object field content".to_string(),
                                            location: Some(field_loc.clone()),
                                        }
                                    })?;

                                    match field_kind.as_rule() {
                                        Rule::object_typed_field => {
                                            let mut typed_inner = field_kind.into_inner();
                                            let key_pair = typed_inner.next().ok_or_else(|| {
                                                ShapeError::ParseError {
                                                    message: "expected object field key"
                                                        .to_string(),
                                                    location: Some(field_loc.clone()),
                                                }
                                            })?;
                                            let key_pair =
                                                if key_pair.as_rule() == Rule::object_field_name {
                                                    key_pair.into_inner().next().ok_or_else(
                                                        || ShapeError::ParseError {
                                                            message: "expected object field key"
                                                                .to_string(),
                                                            location: Some(field_loc.clone()),
                                                        },
                                                    )?
                                                } else {
                                                    key_pair
                                                };
                                            let key = match key_pair.as_rule() {
                                                Rule::ident | Rule::keyword => {
                                                    key_pair.as_str().to_string()
                                                }
                                                _ => {
                                                    return Err(ShapeError::ParseError {
                                                        message: format!(
                                                            "unexpected object key type: {:?}",
                                                            key_pair.as_rule()
                                                        ),
                                                        location: Some(pair_location(&key_pair)),
                                                    });
                                                }
                                            };

                                            let type_pair = typed_inner.next().ok_or_else(|| ShapeError::ParseError {
                                                message: format!("expected type annotation for object field '{}'", key),
                                                location: Some(field_loc.clone()),
                                            })?;
                                            let type_annotation = parse_type_annotation(type_pair)?;

                                            let value_pair =
                                                typed_inner.next().ok_or_else(|| {
                                                    ShapeError::ParseError {
                                                        message: format!(
                                                            "expected value for object field '{}'",
                                                            key
                                                        ),
                                                        location: Some(field_loc),
                                                    }
                                                })?;
                                            let value = super::parse_expression(value_pair)?;

                                            entries.push(ObjectEntry::Field {
                                                key,
                                                value,
                                                type_annotation: Some(type_annotation),
                                            });
                                        }
                                        Rule::object_value_field => {
                                            let mut value_inner = field_kind.into_inner();
                                            let key_pair = value_inner.next().ok_or_else(|| {
                                                ShapeError::ParseError {
                                                    message: "expected object field key"
                                                        .to_string(),
                                                    location: Some(field_loc.clone()),
                                                }
                                            })?;
                                            let key_pair =
                                                if key_pair.as_rule() == Rule::object_field_name {
                                                    key_pair.into_inner().next().ok_or_else(
                                                        || ShapeError::ParseError {
                                                            message: "expected object field key"
                                                                .to_string(),
                                                            location: Some(field_loc.clone()),
                                                        },
                                                    )?
                                                } else {
                                                    key_pair
                                                };
                                            let key = match key_pair.as_rule() {
                                                Rule::ident | Rule::keyword => {
                                                    key_pair.as_str().to_string()
                                                }
                                                _ => {
                                                    return Err(ShapeError::ParseError {
                                                        message: format!(
                                                            "unexpected object key type: {:?}",
                                                            key_pair.as_rule()
                                                        ),
                                                        location: Some(pair_location(&key_pair)),
                                                    });
                                                }
                                            };

                                            let value_pair =
                                                value_inner.next().ok_or_else(|| {
                                                    ShapeError::ParseError {
                                                        message: format!(
                                                            "expected value for object field '{}'",
                                                            key
                                                        ),
                                                        location: Some(field_loc),
                                                    }
                                                })?;
                                            let value = super::parse_expression(value_pair)?;

                                            entries.push(ObjectEntry::Field {
                                                key,
                                                value,
                                                type_annotation: None,
                                            });
                                        }
                                        other => {
                                            return Err(ShapeError::ParseError {
                                                message: format!(
                                                    "unexpected object field kind: {:?}",
                                                    other
                                                ),
                                                location: Some(pair_location(&field_kind)),
                                            });
                                        }
                                    }
                                }
                                Rule::object_spread => {
                                    let spread_expr_pair = field_item_inner
                                        .into_inner()
                                        .next()
                                        .ok_or_else(|| ShapeError::ParseError {
                                            message: "expected expression after spread operator"
                                                .to_string(),
                                            location: Some(field_item_loc),
                                        })?;
                                    let spread_expr = super::parse_expression(spread_expr_pair)?;
                                    entries.push(ObjectEntry::Spread(spread_expr));
                                }
                                _ => {
                                    return Err(ShapeError::ParseError {
                                        message: format!(
                                            "Unexpected rule in object_field_item: {:?}",
                                            field_item_inner.as_rule()
                                        ),
                                        location: None,
                                    });
                                }
                            }
                        }
                        _ => {
                            return Err(ShapeError::ParseError {
                                message: format!(
                                    "Unexpected rule in object_fields: {:?}",
                                    field_item_pair.as_rule()
                                ),
                                location: None,
                            });
                        }
                    }
                }
            }
            _ => {} // Empty object case
        }
    }

    Ok(Expr::Object(entries, span))
}

/// Parse a prefixed integer literal (hex 0x, binary 0b, octal 0o).
/// `stripped` is the number string with leading '-' removed, `prefix_len` is the length of the base prefix (2 for "0x"/"0b"/"0o").
fn parse_prefixed_int(
    full_str: &str,
    stripped: &str,
    radix: u32,
    prefix_len: usize,
    is_negative: bool,
    loc: &crate::error::SourceLocation,
) -> Result<Literal> {
    let after_prefix = &stripped[prefix_len..];
    // Check for width suffix
    let (digits, width) = try_strip_width_suffix(after_prefix);
    let value = i64::from_str_radix(digits, radix).map_err(|e| ShapeError::ParseError {
        message: format!("Invalid base-{} integer '{}': {}", radix, full_str, e),
        location: Some(loc.clone()),
    })?;
    let value = if is_negative { -value } else { value };
    if let Some(w) = width {
        if !w.in_range_i64(value) {
            return Err(ShapeError::ParseError {
                message: format!(
                    "Value {} out of range for {}: [{}, {}]",
                    value,
                    w.type_name(),
                    w.min_value(),
                    w.max_value(),
                ),
                location: Some(loc.clone()),
            });
        }
        Ok(Literal::TypedInt(value, w))
    } else {
        Ok(Literal::Int(value))
    }
}

/// Try to strip width suffix from digit string, returning (digits, optional width).
fn try_strip_width_suffix(s: &str) -> (&str, Option<IntWidth>) {
    const SUFFIXES: &[(&str, IntWidth)] = &[
        ("i32", IntWidth::I32),
        ("i16", IntWidth::I16),
        ("i8", IntWidth::I8),
        ("u64", IntWidth::U64),
        ("u32", IntWidth::U32),
        ("u16", IntWidth::U16),
        ("u8", IntWidth::U8),
    ];
    for &(suffix, width) in SUFFIXES {
        if let Some(digits) = s.strip_suffix(suffix) {
            return (digits, Some(width));
        }
    }
    (s, None)
}

/// Try to parse a suffixed integer literal (e.g., "42i8", "255u8", "18446744073709551615u64").
/// Returns None if no suffix is found. Returns Err for invalid range.
fn try_parse_suffixed_int(
    num_str: &str,
    loc: &crate::error::SourceLocation,
) -> Result<Option<Literal>> {
    // Check all suffixes (longer first to avoid prefix issues)
    const SUFFIXES: &[(&str, IntWidth)] = &[
        ("i32", IntWidth::I32),
        ("i16", IntWidth::I16),
        ("i8", IntWidth::I8),
        ("u64", IntWidth::U64),
        ("u32", IntWidth::U32),
        ("u16", IntWidth::U16),
        ("u8", IntWidth::U8),
    ];

    for &(suffix, width) in SUFFIXES {
        if let Some(digits) = num_str.strip_suffix(suffix) {
            if digits.is_empty() {
                return Err(ShapeError::ParseError {
                    message: format!("Missing digits before '{}'", suffix),
                    location: Some(loc.clone()),
                });
            }

            if width == IntWidth::U64 {
                // u64: parse as u64 directly (handles values > i64::MAX)
                let value: u64 = digits.parse().map_err(|e| ShapeError::ParseError {
                    message: format!("Invalid u64 literal '{}': {}", num_str, e),
                    location: Some(loc.clone()),
                })?;
                if value > i64::MAX as u64 {
                    return Ok(Some(Literal::UInt(value)));
                }
                return Ok(Some(Literal::TypedInt(value as i64, width)));
            }

            // Signed/unsigned sub-64: parse as i64, then range-check
            let value: i64 = digits.parse().map_err(|e| ShapeError::ParseError {
                message: format!("Invalid {} literal '{}': {}", suffix, num_str, e),
                location: Some(loc.clone()),
            })?;

            if !width.in_range_i64(value) {
                return Err(ShapeError::ParseError {
                    message: format!(
                        "Value {} out of range for {}: [{}, {}]",
                        value,
                        width.type_name(),
                        width.min_value(),
                        width.max_value(),
                    ),
                    location: Some(loc.clone()),
                });
            }

            return Ok(Some(Literal::TypedInt(value, width)));
        }
    }

    Ok(None)
}

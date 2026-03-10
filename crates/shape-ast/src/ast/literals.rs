//! Literal types for Shape AST

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::data::Timeframe;
use crate::int_width::IntWidth;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterpolationMode {
    Braces,
    Dollar,
    Hash,
}

impl InterpolationMode {
    pub fn prefix(self) -> &'static str {
        match self {
            InterpolationMode::Braces => "f",
            InterpolationMode::Dollar => "f$",
            InterpolationMode::Hash => "f#",
        }
    }

    pub fn sigil(self) -> Option<char> {
        match self {
            InterpolationMode::Braces => None,
            InterpolationMode::Dollar => Some('$'),
            InterpolationMode::Hash => Some('#'),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Literal {
    Int(i64),
    /// Unsigned integer literal > i64::MAX (e.g., 18446744073709551615u64)
    UInt(u64),
    /// Explicitly width-typed integer literal (e.g., 42i8, 100u16)
    TypedInt(i64, IntWidth),
    Number(f64),
    /// Decimal type for exact arithmetic (finance, currency)
    Decimal(Decimal),
    String(String),
    /// Unicode scalar value char literal (`'a'`, `'\n'`, `'\u{1F600}'`)
    Char(char),
    /// Formatted string literal (`f"..."`, `f$"..."`, `f#"..."` + triple variants)
    FormattedString {
        value: String,
        mode: InterpolationMode,
    },
    /// Content string literal (`c"..."`, `c$"..."`, `c#"..."` + triple variants)
    ContentString {
        value: String,
        mode: InterpolationMode,
    },
    Bool(bool),
    /// Option::None literal (replaces null)
    None,
    /// Unit literal `()`
    Unit,
    Timeframe(Timeframe),
}

impl std::fmt::Display for Literal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Literal::Int(i) => write!(f, "{}", i),
            Literal::UInt(u) => write!(f, "{}u64", u),
            Literal::TypedInt(v, w) => write!(f, "{}{}", v, w),
            Literal::Number(n) => {
                if n.fract() == 0.0 {
                    write!(f, "{}", *n as i64)
                } else {
                    write!(f, "{}", n)
                }
            }
            Literal::Decimal(d) => write!(f, "{}D", d),
            Literal::String(s) => write!(f, "\"{}\"", s),
            Literal::Char(c) => write!(f, "'{}'", c.escape_default()),
            Literal::FormattedString { value, mode } => write!(f, "{}\"{}\"", mode.prefix(), value),
            Literal::ContentString { value, mode } => {
                let prefix = match mode {
                    InterpolationMode::Braces => "c",
                    InterpolationMode::Dollar => "c$",
                    InterpolationMode::Hash => "c#",
                };
                write!(f, "{}\"{}\"", prefix, value)
            }
            Literal::Bool(b) => write!(f, "{}", b),
            Literal::None => write!(f, "None"),
            Literal::Unit => write!(f, "()"),
            Literal::Timeframe(tf) => write!(f, "{}", tf),
        }
    }
}

impl Literal {
    /// Convert literal to a JSON value
    pub fn to_json_value(&self) -> serde_json::Value {
        match self {
            Literal::Int(i) => serde_json::json!(*i),
            Literal::UInt(u) => serde_json::json!(*u),
            Literal::TypedInt(v, _) => serde_json::json!(*v),
            Literal::Number(n) => serde_json::json!(*n),
            Literal::Decimal(d) => serde_json::json!(d.to_string()),
            Literal::String(s) => serde_json::json!(s),
            Literal::Char(c) => serde_json::json!(c.to_string()),
            Literal::FormattedString { value, .. } => serde_json::json!(value),
            Literal::ContentString { value, .. } => serde_json::json!(value),
            Literal::Bool(b) => serde_json::json!(*b),
            Literal::None => serde_json::Value::Null,
            Literal::Unit => serde_json::Value::Null,
            Literal::Timeframe(t) => serde_json::json!(t.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Duration {
    pub value: f64,
    pub unit: DurationUnit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DurationUnit {
    Seconds,
    Minutes,
    Hours,
    Days,
    Weeks,
    Months,
    Years,
    Samples,
}

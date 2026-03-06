//! Time-related types for Shape AST

use serde::{Deserialize, Serialize};

use super::literals::Duration;

// Timeframe is now defined in our internal data module
pub use crate::data::{Timeframe, TimeframeUnit};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TimeReference {
    /// Absolute time: @"2024-01-15 09:30"
    Absolute(String), // Will be parsed to DateTime in semantic phase
    /// Named time: @today, @yesterday
    Named(NamedTime),
    /// Relative time: @"1 week ago"
    Relative(RelativeTime),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DateTimeExpr {
    /// Absolute datetime: @"2024-01-15 09:30"
    Absolute(String),
    /// Literal datetime string
    Literal(String),
    /// Named time: @today, @now
    Named(NamedTime),
    /// Relative time with duration
    Relative {
        base: Box<DateTimeExpr>,
        offset: Duration,
    },
    /// Arithmetic operations on datetime
    Arithmetic {
        base: Box<DateTimeExpr>,
        operator: String,
        duration: Duration,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NamedTime {
    Today,
    Yesterday,
    Now,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelativeTime {
    pub amount: i32,
    pub unit: TimeUnit,
    pub direction: TimeDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TimeUnit {
    Minutes,
    Hours,
    Days,
    Weeks,
    Months,
    Samples,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TimeDirection {
    Ago,
    Future,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TimeWindow {
    /// last(100 rows)
    Last { amount: i32, unit: TimeUnit },
    /// between(@"2024-01-01", @"2024-01-31")
    Between {
        start: TimeReference,
        end: TimeReference,
    },
    /// window(5) or window(-10, -1)
    Window { start: i32, end: Option<i32> },
    /// window(@"09:30", @"16:00") - session window
    Session { start: String, end: String },
}

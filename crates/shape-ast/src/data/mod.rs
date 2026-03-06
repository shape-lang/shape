//! Timeframe definitions and utilities
//!
//! Represents time intervals for data aggregation (e.g., 5m, 1h, 1d).

use serde::{Deserialize, Serialize};
use std::fmt;

/// Represents a time interval for data aggregation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Timeframe {
    pub value: u32,
    pub unit: TimeframeUnit,
}

impl PartialOrd for Timeframe {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Timeframe {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.to_seconds().cmp(&other.to_seconds())
    }
}

/// Time unit for timeframes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TimeframeUnit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Year,
}

impl Timeframe {
    /// Create a new timeframe
    pub fn new(value: u32, unit: TimeframeUnit) -> Self {
        Self { value, unit }
    }

    /// Parse a timeframe string like "5m", "1h", "4h", "1d", etc.
    pub fn parse(s: &str) -> Option<Self> {
        if s.is_empty() {
            return None;
        }

        // Find where the number ends and unit begins
        let digit_end = s.find(|c: char| !c.is_numeric())?;
        if digit_end == 0 {
            return None;
        }

        let (num_str, unit_str) = s.split_at(digit_end);
        let value: u32 = num_str.parse().ok()?;

        let unit = match unit_str.to_lowercase().as_str() {
            "s" | "sec" | "second" | "seconds" => TimeframeUnit::Second,
            "m" | "min" | "minute" | "minutes" => TimeframeUnit::Minute,
            "h" | "hr" | "hour" | "hours" => TimeframeUnit::Hour,
            "d" | "day" | "days" => TimeframeUnit::Day,
            "w" | "week" | "weeks" => TimeframeUnit::Week,
            "mo" | "mon" | "month" | "months" => TimeframeUnit::Month,
            _ => return None,
        };

        Some(Self { value, unit })
    }

    /// Convert to total seconds for comparison
    pub fn to_seconds(&self) -> u64 {
        let multiplier = match self.unit {
            TimeframeUnit::Second => 1,
            TimeframeUnit::Minute => 60,
            TimeframeUnit::Hour => 3600,
            TimeframeUnit::Day => 86400,
            TimeframeUnit::Week => 604800,
            TimeframeUnit::Month => 2592000, // Approximate: 30 days
            TimeframeUnit::Year => 31536000, // Approximate: 365 days
        };
        self.value as u64 * multiplier
    }

    /// Convert to total milliseconds
    pub fn to_millis(&self) -> u64 {
        self.to_seconds() * 1000
    }

    /// Check if this timeframe can be aggregated from another
    pub fn can_aggregate_from(&self, base: &Timeframe) -> bool {
        let self_seconds = self.to_seconds();
        let base_seconds = base.to_seconds();

        // Must be larger than base and evenly divisible
        self_seconds > base_seconds && self_seconds.is_multiple_of(base_seconds)
    }

    /// Calculate how many base rows are needed for this timeframe
    pub fn aggregation_factor(&self, base: &Timeframe) -> Option<usize> {
        if !self.can_aggregate_from(base) {
            return None;
        }
        Some((self.to_seconds() / base.to_seconds()) as usize)
    }

    /// Get alignment timestamp for a given timestamp
    /// For example, 9:32:45 with 5m timeframe aligns to 9:30:00
    pub fn align_timestamp(&self, timestamp: i64) -> i64 {
        let interval = self.to_seconds() as i64;
        (timestamp / interval) * interval
    }

    /// Get next aligned timestamp
    pub fn next_aligned_timestamp(&self, timestamp: i64) -> i64 {
        self.align_timestamp(timestamp) + self.to_seconds() as i64
    }

    // Common timeframes as convenience constructors
    pub fn m1() -> Self {
        Self::new(1, TimeframeUnit::Minute)
    }
    pub fn m5() -> Self {
        Self::new(5, TimeframeUnit::Minute)
    }
    pub fn m15() -> Self {
        Self::new(15, TimeframeUnit::Minute)
    }
    pub fn m30() -> Self {
        Self::new(30, TimeframeUnit::Minute)
    }
    pub fn h1() -> Self {
        Self::new(1, TimeframeUnit::Hour)
    }
    pub fn h4() -> Self {
        Self::new(4, TimeframeUnit::Hour)
    }
    pub fn d1() -> Self {
        Self::new(1, TimeframeUnit::Day)
    }
    pub fn w1() -> Self {
        Self::new(1, TimeframeUnit::Week)
    }

    /// Check if this is an intraday timeframe
    pub fn is_intraday(&self) -> bool {
        matches!(
            self.unit,
            TimeframeUnit::Second | TimeframeUnit::Minute | TimeframeUnit::Hour
        )
    }
}

impl fmt::Display for Timeframe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let unit_str = match self.unit {
            TimeframeUnit::Second => "s",
            TimeframeUnit::Minute => "m",
            TimeframeUnit::Hour => "h",
            TimeframeUnit::Day => "d",
            TimeframeUnit::Week => "w",
            TimeframeUnit::Month => "M",
            TimeframeUnit::Year => "y",
        };
        write!(f, "{}{}", self.value, unit_str)
    }
}

impl std::str::FromStr for Timeframe {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| format!("Invalid timeframe: {}", s))
    }
}

impl Default for Timeframe {
    fn default() -> Self {
        Self::d1()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeframe_parsing() {
        assert_eq!(
            Timeframe::parse("5m"),
            Some(Timeframe::new(5, TimeframeUnit::Minute))
        );
        assert_eq!(
            Timeframe::parse("1h"),
            Some(Timeframe::new(1, TimeframeUnit::Hour))
        );
        assert_eq!(
            Timeframe::parse("4h"),
            Some(Timeframe::new(4, TimeframeUnit::Hour))
        );
        assert_eq!(
            Timeframe::parse("1d"),
            Some(Timeframe::new(1, TimeframeUnit::Day))
        );
    }

    #[test]
    fn test_aggregation() {
        let m1 = Timeframe::m1();
        let m5 = Timeframe::m5();
        let h1 = Timeframe::h1();

        assert!(m5.can_aggregate_from(&m1));
        assert!(h1.can_aggregate_from(&m1));
        assert!(h1.can_aggregate_from(&m5));

        assert_eq!(m5.aggregation_factor(&m1), Some(5));
        assert_eq!(h1.aggregation_factor(&m1), Some(60));
        assert_eq!(h1.aggregation_factor(&m5), Some(12));
    }

    #[test]
    fn test_timestamp_alignment() {
        let m5 = Timeframe::m5();
        let ts = 1704111165; // Some timestamp
        let aligned = m5.align_timestamp(ts);
        assert_eq!(aligned % 300, 0); // Should be divisible by 300 seconds
    }
}

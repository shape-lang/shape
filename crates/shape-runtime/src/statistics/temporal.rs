//! Time-based statistical analysis

use crate::query_result::{PatternMatch, QueryResult};
use chrono::{Datelike, Timelike};
use shape_ast::error::Result;
use std::collections::HashMap;

use super::types::{
    SeasonalityAnalysis, TemporalStatistics, TimePeriodStats, TrendAnalysis, TrendDirection,
};

/// Calculate temporal statistics
pub(super) fn calculate_temporal_statistics(result: &QueryResult) -> Result<TemporalStatistics> {
    let matches = result.matches.as_deref().unwrap_or(&[]);

    // Calculate performance by time period
    let best_hours = calculate_hourly_performance(matches)?;
    let best_days = calculate_daily_performance(matches)?;
    let best_months = calculate_monthly_performance(matches)?;

    // Analyze seasonality
    let seasonality = analyze_seasonality(matches)?;

    // Analyze trends
    let trends = analyze_trends(matches)?;

    Ok(TemporalStatistics {
        best_hours,
        best_days,
        best_months,
        seasonality,
        trends,
    })
}

/// Calculate hourly performance
fn calculate_hourly_performance(matches: &[PatternMatch]) -> Result<Vec<TimePeriodStats>> {
    let mut hourly_stats: HashMap<u32, (usize, usize, f64)> = HashMap::new();

    for pattern_match in matches {
        let hour = pattern_match.timestamp.hour();
        let entry = hourly_stats.entry(hour).or_insert((0, 0, 0.0));

        entry.0 += 1; // occurrence count
        entry.2 += pattern_match.confidence;

        if pattern_match.confidence > 0.5 {
            entry.1 += 1; // success count
        }
    }

    let mut stats: Vec<TimePeriodStats> = hourly_stats
        .into_iter()
        .map(|(hour, (count, successes, total_value))| {
            let success_rate = if count > 0 {
                successes as f64 / count as f64
            } else {
                0.0
            };

            let avg_value = if count > 0 {
                total_value / count as f64
            } else {
                0.0
            };

            TimePeriodStats {
                period: format!("{:02}:00", hour),
                success_rate,
                avg_value,
                occurrence_count: count,
            }
        })
        .collect();

    stats.sort_by(|a, b| b.success_rate.partial_cmp(&a.success_rate).unwrap());
    stats.truncate(5); // Top 5 hours

    Ok(stats)
}

/// Calculate daily performance
fn calculate_daily_performance(matches: &[PatternMatch]) -> Result<Vec<TimePeriodStats>> {
    let mut daily_stats: HashMap<String, (usize, usize, f64)> = HashMap::new();

    for pattern_match in matches {
        let weekday = pattern_match.timestamp.weekday().to_string();
        let entry = daily_stats.entry(weekday).or_insert((0, 0, 0.0));

        entry.0 += 1;
        entry.2 += pattern_match.confidence;

        if pattern_match.confidence > 0.5 {
            entry.1 += 1;
        }
    }

    let mut stats: Vec<TimePeriodStats> = daily_stats
        .into_iter()
        .map(|(day, (count, successes, total_value))| {
            let success_rate = if count > 0 {
                successes as f64 / count as f64
            } else {
                0.0
            };

            let avg_value = if count > 0 {
                total_value / count as f64
            } else {
                0.0
            };

            TimePeriodStats {
                period: day,
                success_rate,
                avg_value,
                occurrence_count: count,
            }
        })
        .collect();

    stats.sort_by(|a, b| b.success_rate.partial_cmp(&a.success_rate).unwrap());

    Ok(stats)
}

/// Calculate monthly performance
fn calculate_monthly_performance(matches: &[PatternMatch]) -> Result<Vec<TimePeriodStats>> {
    let mut monthly_stats: HashMap<u32, (usize, usize, f64)> = HashMap::new();

    for pattern_match in matches {
        let month = pattern_match.timestamp.month();
        let entry = monthly_stats.entry(month).or_insert((0, 0, 0.0));

        entry.0 += 1;
        entry.2 += pattern_match.confidence;

        if pattern_match.confidence > 0.5 {
            entry.1 += 1;
        }
    }

    let mut stats: Vec<TimePeriodStats> = monthly_stats
        .into_iter()
        .map(|(month, (count, successes, total_value))| {
            let success_rate = if count > 0 {
                successes as f64 / count as f64
            } else {
                0.0
            };

            let avg_value = if count > 0 {
                total_value / count as f64
            } else {
                0.0
            };

            TimePeriodStats {
                period: month_name(month).to_string(),
                success_rate,
                avg_value,
                occurrence_count: count,
            }
        })
        .collect();

    stats.sort_by(|a, b| b.success_rate.partial_cmp(&a.success_rate).unwrap());

    Ok(stats)
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "Unknown",
    }
}

/// Analyze seasonality
fn analyze_seasonality(_matches: &[PatternMatch]) -> Result<SeasonalityAnalysis> {
    // Basic implementation placeholder
    Ok(SeasonalityAnalysis {
        daily_pattern: false,
        weekly_pattern: false,
        monthly_pattern: false,
        quarterly_pattern: false,
        strength: 0.0,
    })
}

/// Analyze trends
fn analyze_trends(_matches: &[PatternMatch]) -> Result<TrendAnalysis> {
    // Basic implementation placeholder
    Ok(TrendAnalysis {
        pattern_frequency_trend: 0.0,
        success_rate_trend: 0.0,
        value_trend: 0.0,
        trend_direction: TrendDirection::Stable,
    })
}

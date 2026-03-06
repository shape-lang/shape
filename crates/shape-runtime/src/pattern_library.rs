//! Pattern library stub for semantic analysis
//!
//! Patterns are now loaded from Shape files, not hardcoded.
//! This is a minimal stub to satisfy the semantic analyzer.

use std::collections::HashSet;

pub struct PatternLibrary {
    pattern_names: HashSet<String>,
}

impl Default for PatternLibrary {
    fn default() -> Self {
        Self::new()
    }
}

impl PatternLibrary {
    pub fn new() -> Self {
        // Initialize with standard pattern names from stdlib.patterns
        let mut pattern_names = HashSet::new();

        // Single row patterns
        pattern_names.insert("hammer".to_string());
        pattern_names.insert("doji".to_string());
        pattern_names.insert("shooting_star".to_string());
        pattern_names.insert("dragonfly_doji".to_string());
        pattern_names.insert("gravestone_doji".to_string());
        pattern_names.insert("long_legged_doji".to_string());
        pattern_names.insert("marubozu".to_string());
        pattern_names.insert("bullish_marubozu".to_string());
        pattern_names.insert("bearish_marubozu".to_string());
        pattern_names.insert("spinning_top".to_string());

        // Two row patterns
        pattern_names.insert("bullish_engulfing".to_string());
        pattern_names.insert("bearish_engulfing".to_string());
        pattern_names.insert("tweezer_top".to_string());
        pattern_names.insert("tweezer_bottom".to_string());
        pattern_names.insert("piercing_line".to_string());
        pattern_names.insert("dark_cloud_cover".to_string());
        pattern_names.insert("bullish_harami".to_string());
        pattern_names.insert("bearish_harami".to_string());
        pattern_names.insert("harami".to_string());
        pattern_names.insert("bullish_belt_hold".to_string());
        pattern_names.insert("bearish_belt_hold".to_string());
        pattern_names.insert("in_neck_line".to_string());
        pattern_names.insert("on_neck_line".to_string());
        pattern_names.insert("thrusting_pattern".to_string());

        // Three row patterns
        pattern_names.insert("morning_star".to_string());
        pattern_names.insert("evening_star".to_string());
        pattern_names.insert("three_white_soldiers".to_string());
        pattern_names.insert("three_black_crows".to_string());
        pattern_names.insert("three_inside_up".to_string());
        pattern_names.insert("three_inside_down".to_string());
        pattern_names.insert("three_outside_up".to_string());
        pattern_names.insert("three_outside_down".to_string());
        pattern_names.insert("abandoned_baby_bullish".to_string());
        pattern_names.insert("abandoned_baby_bearish".to_string());
        pattern_names.insert("bullish_tri_star".to_string());
        pattern_names.insert("bearish_tri_star".to_string());

        Self { pattern_names }
    }

    pub fn has_pattern(&self, name: &str) -> bool {
        self.pattern_names.contains(name)
    }

    pub fn pattern_names(&self) -> Vec<String> {
        self.pattern_names.iter().cloned().collect()
    }
}
